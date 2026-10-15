//! Boot to the game's own title screen, and the front end (frontbin) in retail mode.
//!
//! There is no savestate at the title, so `--title` (or RAC_BOOT_TITLE=1) starts from the usual fresh-game state and lets the
//! GAME walk back to its title through its own Quit path, steered by spaced pad pulses (nothing is written to game RAM):
//! pause (Start) -> Options -> Quit Game -> Yes (Circle, decomp/notes/ingame_save_quit.md section 5). The level overlay then
//! unloads and the front end (frontbin) loads from the disc and runs its title loop (frontend_flow.md sections 1-2). Until
//! the title runs the display is plain black and the player's keys are ignored; after `TIMEOUT` the cover is lifted, the fresh
//! game is left running and the reason goes to stderr.
//!
//! The front end itself needs three things from retail.rs, all independent of the flag (a Quit Game from the pause menu
//! lands there too):
//! * `front_end` tells it apart from a level (the movie player's code hash at its frontbin address);
//! * the worker sends a minimal `Snap` there (`front`, no level mirror) and reads the menu from the front end's page pointer;
//! * the picture is the GS's own (the GS-full pipe, started when the quit is requested: `want_late_gs` / `late`).
//!
//! The memory card is held while the boot runs: the fresh game has an active save slot with autosave on and may write to it
//! (a cutscene skip is an autosave trigger). `Saver::hold` keeps card.bin untouched meanwhile and the card in the machine goes
//! back to what it was at start once the front end runs, so the title's Load Game lists the player's real saves.
use super::mouse_menu::{self, MenuSnap, Target};
use ee_oracle::Machine;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static ON: AtomicBool = AtomicBool::new(false);
/// The boot runs: the memory card is held (see the module doc).
static HOLD: AtomicBool = AtomicBool::new(false);

pub fn holding() -> bool {
	HOLD.load(Ordering::Relaxed)
}

/// `--title`: boot to the title screen.
pub fn enable() {
	ON.store(true, Ordering::Relaxed);
}

pub fn enabled() -> bool {
	ON.load(Ordering::Relaxed) || std::env::var("RAC_BOOT_TITLE").is_ok_and(|v| v != "0" && !v.is_empty())
}

/// The front end's mode word (boot data in frontbin: 0 title idle, 3 page menu, 4 text screen; the level overlays keep theirs at 0x15f5a4).
pub const FRONT_MODE: u32 = 0x15f5e4;
/// Level-request words of the level overlays (Eudora/Veldin 0x27f6d0, ingame_save_quit.md section 5): target level (-1 = front end), request flags.
const REQ_LEVEL: u32 = 0x15f5a0;
const REQ_FLAG: u32 = 0x15f5b8;

/// The frontbin overlay is in RAM: its movie player (fmv_bridge `FRONT`) has the hash of the ELF's code. A level overlay or the
/// boot image has other code there.
pub fn front_end(m: &Machine) -> bool {
	use super::fmv_bridge::{FRONT, PLAYERS};
	PLAYERS.iter().find(|p| p.level == FRONT).is_some_and(|p| ee_oracle::rec::hash(&m.cpu.mem, p.entry, p.end) == p.hash)
}

/// The front end runs interpreted (no recompiled code), and the oracle's hand table is only consulted from recompiled
/// function entries, so the fmv_bridge hook never fires there on its own: the retail movie player would spin on the
/// unemulated IPU. An oracle host stub (`Cpu::add_host_stub`, called from the interpreter's step) on the frontbin player
/// runs the same hook (`fmv_bridge::movie_call`) as the level movies: the attract movie after ~25 s idle on the title and
/// the New Game intro play in the window, or are skipped at once without one (v0 = 0 either way, like a skipped movie).
/// The stub applies only while RAM holds frontbin's player code (its first four words).
pub fn hook_front_movies(m: &mut Machine) {
	use super::fmv_bridge::{movie_call, FRONT, PLAYERS};
	let Some(p) = PLAYERS.iter().find(|p| p.level == FRONT) else { return };
	let sig = [0, 1, 2, 3].map(|k| m.rd32(p.entry + 4 * k));
	m.cpu.add_host_stub(p.entry, "front end movie player (fmv_bridge)", sig, movie_call);
}

/// Cold boot only: a level overlay's movie player (any of the 19 levels) is in RAM, i.e. the boot ELF's own front end has handed
/// over to a level. (The boot ELF carries the first title itself; frontbin only loads after a Quit Game.)
pub fn level_loaded(m: &Machine) -> bool {
	use super::fmv_bridge::PLAYERS;
	PLAYERS.iter().any(|p| p.level < 19 && ee_oracle::rec::hash(&m.cpu.mem, p.entry, p.end) == p.hash)
}

/// Cold boot (`--retail-cold`): the BOOT player (the logo movies, boot ELF 0x239ec8) is replaced like the front end's one. The cold
/// machine's hook rule `ret 239ec8 1` (cold_hooks.txt: movies skipped for the oracle-only drivers) is removed so the movie reaches
/// the native bridge: the registered hand takes compiled callers, this host stub the interpreted ones.
pub fn hook_boot_movies(m: &mut Machine) {
	use super::fmv_bridge::{movie_call, BOOT, PLAYERS};
	let Some(p) = PLAYERS.iter().find(|p| p.level == BOOT) else { return };
	if let Some(k) = m.cold_kernel_mut() {
		k.rules_ret.remove(&p.entry);
	}
	let sig = [0, 1, 2, 3].map(|k| m.rd32(p.entry + 4 * k));
	m.cpu.add_host_stub(p.entry, "boot movie player (fmv_bridge)", sig, movie_call);
}

/// The game asked for the front end (Quit Game: target level -1 and the load request flag): the picture of the front end
/// has to be recorded from before its first draw, so the worker starts the GS-full pipe now.
pub fn want_late_gs(m: &Machine) -> bool {
	m.rd32(REQ_LEVEL) == u32::MAX && m.rd32(REQ_FLAG) != 0
}

/// RAC_FRONT_GS=1: the front end's picture is the GS's own (debug fallback, -GsFull references); else the mirror draws it.
pub fn gs_picture() -> bool {
	std::env::var("RAC_FRONT_GS").is_ok_and(|v| v != "0" && !v.is_empty())
}

/// The GS-full pipe of the front end, for the main thread to take frames from (None outside the front end).
static LATE: Mutex<Option<Arc<crate::gsfull::pipe::GsFull>>> = Mutex::new(None);

pub fn set_late(s: Option<Arc<crate::gsfull::pipe::GsFull>>) {
	if let Ok(mut l) = LATE.lock() {
		*l = s;
	}
}

pub fn late() -> Option<Arc<crate::gsfull::pipe::GsFull>> {
	LATE.lock().ok().and_then(|l| l.clone())
}

/// The front end's menu out of RAM, when the page-menu mode runs there.
pub fn front_menu(m: &Machine, mode: u32) -> Option<MenuSnap> {
	mouse_menu::read_front(&|a| m.rd32(a), mode)
}

const START: u16 = 0x800;
const CROSS: u16 = mouse_menu::CROSS;
const CIRCLE: u16 = mouse_menu::CIRCLE;
const TRIANGLE: u16 = mouse_menu::TRIANGLE;
/// Pulse design of the mouse menu: held 2 fields, released 3.
const PRESS: u8 = 2;
const RELEASE: u8 = 3;
/// The in-game Options page's Quit Game row, the pause ring's Options button, and the Quit Game page itself (Veldin overlay).
const T_QUIT: u16 = 0x4f26;
const T_OPTIONS: u16 = 0x4ee7;
const PAGE_QUIT: u32 = 0x1b5be0;
/// Wall-clock budget to reach the title, and what a confirmed Yes adds (the frontbin load).
const TIMEOUT: Duration = Duration::from_secs(15);
const AFTER_YES: Duration = Duration::from_secs(25);
/// Fields of the running title loop shown black before the cover lifts (the first draws fade the scene in).
const SETTLE: u32 = 25;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
	/// Open the pause menu and walk it to the Quit Game page.
	Steer,
	/// Yes pressed: wait for the front end.
	Quitting,
	/// The front end runs: wait `SETTLE` fields of its title loop.
	Settle(u32),
	Done,
	Failed,
}

/// The boot automaton (main thread; one `step` per field).
pub struct Boot {
	stage: Stage,
	deadline: Instant,
	fields: u32,
	last_start: u32,
	pulse: u16,
	left: u8,
	pilot: Pilot,
	/// Fields since the front end began (log).
	seen_front: bool,
	trace: bool,
}

impl Boot {
	pub fn new() -> Boot {
		eprintln!("title: booting to the title screen (pause > Options > Quit Game > Yes), display black, keys ignored");
		HOLD.store(true, Ordering::Relaxed);
		Boot { stage: Stage::Steer, deadline: Instant::now() + TIMEOUT, fields: 0, last_start: 0, pulse: 0, left: 0, pilot: Pilot::from_env(), seen_front: false, trace: std::env::var_os("RAC_TITLE_TRACE").is_some() }
	}

	/// A cold boot (the real game from power-on) with RAC_TITLE_PILOT set: no boot automaton, only the scripted player (tests).
	pub fn pilot_only() -> Boot {
		Boot { stage: Stage::Done, deadline: Instant::now(), fields: 0, last_start: 0, pulse: 0, left: 0, pilot: Pilot::from_env(), seen_front: false, trace: false }
	}

	/// The player's keys are ignored and the display is black.
	pub fn active(&self) -> bool {
		!matches!(self.stage, Stage::Done | Stage::Failed)
	}

	/// The pad bits to send this field. `menu` is the newest menu snapshot, `mode` the game mode word of where the game is (the
	/// front end's when `front`).
	pub fn step(&mut self, menu: Option<&MenuSnap>, mode: u32, front: bool) -> u16 {
		self.fields += 1;
		if self.trace && self.fields % 50 == 0 {
			eprintln!("title: field {} stage {:?} front {front} mode {mode} menu {:?}", self.fields, self.stage, menu.map(|m| (m.page, m.id, m.state, m.cur, m.items.len())));
		}
		if self.left > 0 {
			return self.advance();
		}
		match self.stage {
			Stage::Steer => {
				if front {
					return self.to_settle();
				}
				if Instant::now() > self.deadline {
					return self.fail(format!("the pause menu did not reach Quit Game within {} s (mode {mode}, menu {:?})", TIMEOUT.as_secs(), menu.map(|m| (m.page, m.id, m.state))));
				}
				let Some(m) = menu.filter(|m| !m.busy()) else {
					// No settled menu: a menu that is flying in or out is left alone; with none, Start opens it (or skips a cutscene first).
					if menu.is_none() && mode != 3 && self.fields - self.last_start >= 45 {
						self.last_start = self.fields;
						return self.fire(START);
					}
					return 0;
				};
				if m.page == PAGE_QUIT {
					self.stage = Stage::Quitting;
					self.deadline = Instant::now() + AFTER_YES;
					eprintln!("title: Quit Game > Yes at boot field {}", self.fields);
					return self.fire(CIRCLE);
				}
				// Quit Game row anywhere on this page, else the Options button, else back out of whatever page this is.
				let goal = find_text(m, T_QUIT).or_else(|| find_text(m, T_OPTIONS));
				match goal {
					Some(t) if m.selected(t) => self.fire(CROSS),
					Some(t) => match m.toward(t) {
						Some(b) => self.fire(b),
						None => self.fire(TRIANGLE),
					},
					None => self.fire(TRIANGLE),
				}
			}
			Stage::Quitting => {
				if front {
					return self.to_settle();
				}
				if Instant::now() > self.deadline {
					return self.fail("the front end did not start after Quit Game > Yes".into());
				}
				0
			}
			Stage::Settle(n) => {
				if !front {
					return self.fail("the front end went away again".into());
				}
				if mode == 0 {
					self.stage = if n + 1 >= SETTLE { Stage::Done } else { Stage::Settle(n + 1) };
					if self.stage == Stage::Done {
						HOLD.store(false, Ordering::Relaxed);
						eprintln!("title: the title loop runs (boot field {}), display uncovered", self.fields);
					}
				}
				0
			}
			Stage::Done | Stage::Failed => 0,
		}
	}

	fn to_settle(&mut self) -> u16 {
		self.stage = Stage::Settle(0);
		self.seen_front = true;
		eprintln!("title: front end loaded at boot field {}", self.fields);
		0
	}

	fn fail(&mut self, why: String) -> u16 {
		eprintln!("title: boot to the title FAILED: {why}; the fresh game keeps running");
		self.stage = Stage::Failed;
		HOLD.store(false, Ordering::Relaxed);
		0
	}

	fn fire(&mut self, b: u16) -> u16 {
		self.pulse = b;
		self.left = PRESS + RELEASE;
		self.advance()
	}

	fn advance(&mut self) -> u16 {
		let out = if self.left > RELEASE { self.pulse } else { 0 };
		self.left -= 1;
		out
	}

	/// After the boot: the optional test pilot (RAC_TITLE_PILOT) keeps pressing buttons in the front end.
	pub fn pilot(&mut self, menu: Option<&MenuSnap>, mode: u32, front: bool, mm: &mut mouse_menu::State) -> u16 {
		if self.active() || !front {
			return 0;
		}
		if self.left > 0 {
			return self.advance();
		}
		let b = self.pilot.step(menu, mode, mm);
		match b {
			0 => 0,
			b => self.fire(b),
		}
	}
}

/// The Target of the first item (or row) whose label list holds text id `t`.
fn find_text(m: &MenuSnap, t: u16) -> Option<Target> {
	m.items.iter().find_map(|it| it.texts.iter().position(|&x| x == t).map(|k| Target::Sub { item: it.addr, sub: k as u32 }))
}

/// RAC_TITLE_PILOT="start,item:4f28,cross,down:4,cross,wait:200": a scripted player for the headless proofs. `start` presses Start
/// until the page menu opens, `item:HEX` walks the selection to the label with that text id and presses Cross, `cross` / `circle`
/// / `triangle` one press, `down:N` / `up:N` N presses, `wait:N` N fields. Each op waits for a settled menu (or the title).
struct Pilot {
	ops: Vec<String>,
	i: usize,
	wait: u32,
	n: u32,
	last: u32,
	fields: u32,
}

impl Pilot {
	fn from_env() -> Pilot {
		let ops = std::env::var("RAC_TITLE_PILOT").map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
		Pilot { ops, i: 0, wait: 0, n: 0, last: 0, fields: 0 }
	}

	fn step(&mut self, menu: Option<&MenuSnap>, mode: u32, mm: &mut mouse_menu::State) -> u16 {
		self.fields += 1;
		if self.wait > 0 {
			self.wait -= 1;
			return 0;
		}
		let Some(op) = self.ops.get(self.i).cloned() else { return 0 };
		let (name, arg) = op.split_once(':').unwrap_or((&op, ""));
		let num = |d: u32| arg.parse::<u32>().unwrap_or(d);
		let settled = menu.filter(|m| !m.busy());
		let next = |p: &mut Pilot, wait: u32| {
			eprintln!("title: pilot {op} done (field {})", p.fields);
			p.i += 1;
			p.n = 0;
			p.wait = wait;
		};
		match name {
			"start" => {
				if menu.is_some() && mode == 3 {
					next(self, 60);
					0
				} else if self.fields - self.last >= 45 {
					self.last = self.fields;
					START
				} else {
					0
				}
			}
			// `title:N`: N fields of the title loop (front mode 0) with no movie parked, e.g. after the logos.
			"title" => {
				if mode == 0 && !super::fmv_bridge::waiting() {
					self.n += 1;
					if self.n >= num(60) {
						next(self, 0);
					}
				}
				0
			}
			"item" => {
				let Some(m) = settled else { return 0 };
				let Some(t) = u16::from_str_radix(arg, 16).ok().and_then(|t| find_text(m, t)) else {
					if self.fields - self.last >= 120 {
						self.last = self.fields;
						eprintln!("title: pilot {op}: no such label on page {:#x} (id {:#x}), texts {:?}", m.page, m.id, m.items.iter().map(|i| i.texts.clone()).collect::<Vec<_>>());
					}
					return 0;
				};
				if m.selected(t) {
					next(self, 90);
					CROSS
				} else {
					m.toward(t).unwrap_or(0)
				}
			}
			"cross" | "circle" | "triangle" | "start1" => {
				next(self, num(90));
				match name {
					"cross" => CROSS,
					"circle" => CIRCLE,
					"start1" => START,
					_ => TRIANGLE,
				}
			}
			"down" | "up" => {
				if self.n >= num(1) {
					next(self, 40);
					return 0;
				}
				self.n += 1;
				self.wait = 6;
				if name == "down" { mouse_menu::DOWN } else { mouse_menu::UP }
			}
			"dump" => {
				match menu {
					Some(m) => eprintln!("title: pilot dump: mode {mode} page {:#x} id {:#x} busy {} items {:?}", m.page, m.id, m.busy(), m.items.iter().map(|i| (i.addr, i.texts.clone())).collect::<Vec<_>>()),
					None => eprintln!("title: pilot dump: mode {mode} no menu"),
				}
				next(self, 30);
				0
			}
			// The mouse (no real mouse moves): `hover:HEX` puts the cursor on the row of that text id, `sel:HEX` logs whether the
			// game's selection is on it, `click` / `mback` are a left / right click (mouse_menu::State, the same path the window's
			// events take).
			"hover" => {
				let Some(m) = settled else { return 0 };
				let pos = u16::from_str_radix(arg, 16).ok().and_then(|t| m.items.iter().find_map(|it| it.texts.iter().position(|&x| x == t).map(|k| (it, k))));
				match pos.and_then(|(it, k)| match &it.kind {
					mouse_menu::Kind::Label { rows, .. } => rows.get(k).map(|r| (it.rect[0] as f32 + it.rect[2] as f32 * 0.5, it.rect[1] as f32 + r.0)),
					_ => None,
				}) {
					Some((x, y)) => {
						eprintln!("title: pilot hover {arg} at overlay ({x:.1}, {y:.1}) on page {:#x} (id {:#x})", m.page, m.id);
						mm.cursor(x, y);
						next(self, num(40).max(1));
					}
					None => {
						eprintln!("title: pilot hover {arg}: no Label row on page {:#x}; items {:?}", m.page, m.items.iter().map(|i| (i.addr, i.rect, i.texts.clone())).collect::<Vec<_>>());
						next(self, 1);
					}
				}
				0
			}
			"sel" => {
				if let Some(m) = settled {
					let on = u16::from_str_radix(arg, 16).ok().and_then(|t| find_text(m, t)).is_some_and(|t| m.selected(t));
					eprintln!("title: pilot sel {arg}: selected {on} (page id {:#x}, cur {:#x})", m.id, m.cur);
					next(self, 1);
				}
				0
			}
			"click" | "mback" => {
				if name == "click" { mm.click() } else { mm.back() }
				next(self, num(90));
				0
			}
			"wait" => {
				next(self, num(60));
				0
			}
			_ => {
				eprintln!("title: pilot: unknown op {op}");
				self.i += 1;
				0
			}
		}
	}
}
