//! [PC] Developer tools for retail mode (2026-10-18). Inert unless the environment variable RAC_DEV_TOOLS=1 is set or the player
//! switched Debug mode on (Pause -> Options -> Controls -> Debug, pc_options `debug=on`, `set_menu`; default Off). Off, every entry point
//! returns at once: no key or mouse event is taken, the field input is untouched, nothing is drawn, so the RAM gates stay
//! byte-identical.
//!
//! Planet teleport (F1): a list of the 19 levels drawn over the picture; Enter asks retail to travel there through its own
//! level request, the helper the in-game Load Game page and the level ends call (Novalis FUN_002a22d0, Veldin 0x28e858, the
//! same three stores on all 19 levels: target 0x15f5a0, no-present 0x15f5b8 = 1, level-loop exit 0x15f550 = 1;
//! decomp/notes/level_transition.md, ingame_save_quit.md). The request goes out as the next field's `FieldIn::travel` (the
//! poke RAC_RETAIL_TRAVEL makes, recorded like it) and only in gameplay (game mode 0, no trip under way). Retail then runs
//! its own exit: the warp, the loader, any story movie its rules pick, and the arrival unlocks the planet as a ship trip does.
//! Picking the planet you are on restarts it (the Load Game page does the same for a save of the current planet).
//!
//! Free camera (F2, or L3 + R3 on a gamepad): the drawn view leaves retail's camera and flies freely; the game's camera RAM
//! is never written (the viewer only replaces the camera it draws with). While it is on the game gets a neutral pad, so
//! Ratchet stands still, and P (Start) freezes the game, `.` (Select) steps one field while frozen. F2 again (or L3 + R3)
//! goes back to retail's camera; the game state is the one it would have had with the same neutral input.
//!
//! Readout: the window title gets the level, Ratchet's position and, with the free camera, its position too; the same line
//! is drawn at the top left while a tool is up.
//!
//! Keys: README in deliver/REPORT.md of the scratch_devtools lane, and `HELP_*` below (drawn on screen).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use winit::keyboard::KeyCode as K;

/// [pos, yaw, pitch, tan_half] as `retail::Cam`.
pub type Cam = ([f32; 3], f32, f32, f32);

/// Debug mode of the Controls page (retail/pcctl.rs, pcopts `debug=on`): the player's switch, live. `debugkeys::enabled` follows it too.
static MENU: AtomicBool = AtomicBool::new(false);

/// The two switches as one: the environment variable forces the tools on, the menu switches them on and off.
pub fn gate(env: bool, menu: bool) -> bool {
	env || menu
}

/// The switch: RAC_DEV_TOOLS=1 or Debug mode On in the menu.
pub fn enabled() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	gate(*ON.get_or_init(|| std::env::var("RAC_DEV_TOOLS").is_ok_and(|v| v == "1")), MENU.load(Ordering::Relaxed))
}

/// Debug mode as the menu has it (the environment switches are not in it).
pub fn menu() -> bool {
	MENU.load(Ordering::Relaxed)
}

/// Worker: Debug mode On / Off (the options file at start, the Controls page's row live). True = it changed. `announce`: the player
/// just switched it On, so the on-screen notice (keys and the warp warning) goes up. Off puts every tool away (list, free camera,
/// freeze) and clears the cheats of `debugkeys`, so a later On starts from nothing.
pub fn set_menu(on: bool, announce: bool) -> bool {
	if MENU.swap(on, Ordering::Relaxed) == on {
		return false;
	}
	let mut u = ui();
	if on {
		u.notice = announce.then(Instant::now);
		eprintln!("retail: devtools: Debug mode On (F1 planet list / warp, F2 free camera, F3 draw distance A/B, F5 immortality, F6 infinite ammo, F8 +10000 bolts)");
	} else {
		u.pick = None;
		u.warp = None;
		u.want = false;
		u.fc = None;
		u.reset = false;
		u.frozen = false;
		u.steps = 0;
		u.pad_mask = 0;
		u.notice = None;
		u.note = None;
		u.line.clear();
		FREE.store(false, Ordering::Relaxed);
		eprintln!("retail: devtools: Debug mode Off");
	}
	drop(u);
	if !on {
		super::debugkeys::menu_off();
	}
	true
}

const LEVELS: usize = crate::toc::LEVEL_COUNT;
/// How long an event stays in the title / on screen.
const NOTE_FOR: Duration = Duration::from_secs(4);
/// How long the notice of a fresh Debug mode On stays up.
const NOTICE_FOR: Duration = Duration::from_secs(15);
/// Free camera speeds: units (metres) per second at speed step 0, the fast / slow factors, the wheel step.
const BASE_SPEED: f32 = 12.0;
const FAST: f32 = 4.0;
const SLOW: f32 = 0.25;
const WHEEL_STEP: f32 = 1.25;
/// Look rates: radians per mouse count, per second for the arrow keys and a full right stick.
const MOUSE_RATE: f32 = 0.0025;
const KEY_TURN: f32 = 1.8;
const STICK_TURN: f32 = 2.6;
const STICK_DEAD: i32 = 0x18;
const PITCH_MAX: f32 = 1.55;

/// PS2 pad bits (pressed = 1) as prompts.rs builds them.
const PAD_L2: u16 = 0x1;
const PAD_R2: u16 = 0x2;
const PAD_L1: u16 = 0x4;
const PAD_R1: u16 = 0x8;
const PAD_TRI: u16 = 0x10;
const PAD_SELECT: u16 = 0x100;
const PAD_L3: u16 = 0x200;
const PAD_R3: u16 = 0x400;
const PAD_START: u16 = 0x800;

/// The notice drawn when the player switches Debug mode On: the keys, then the warning (the overlay font has capitals only).
pub const NOTICE: [&str; 9] = [
	"DEBUG MODE ON",
	"F1  PLANET LIST / WARP (ENTER GOES, ESC CLOSES)",
	"F2  FREE CAMERA (WASD, MOUSE, P FREEZE, R RESET)",
	"F3  DRAW DISTANCE: LOW / YOUR SETTING",
	"F5  IMMORTALITY ON / OFF",
	"F6  INFINITE AMMO ON / OFF",
	"F8  +10000 BOLTS",
	"WARNING: WARPING CAN SKIP STORY PROGRESS.",
	"SAVE YOUR GAME FIRST. OFF: OPTIONS > CONTROLS > DEBUG.",
];

pub const HELP_PICK: &str = "UP/DOWN PGUP/PGDN PICK - ENTER WARP - ESC CLOSE";
pub const HELP_FREE: &str = "WASD MOVE - SPACE/C UP/DOWN - MOUSE/ARROWS LOOK - SHIFT FAST - WHEEL SPEED - P FREEZE - . STEP - R RESET - F2 EXIT";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FreeCam {
	pub pos: [f32; 3],
	pub yaw: f32,
	pub pitch: f32,
	pub tan_half: f32,
}

struct Ui {
	/// The planet list is open, with this row under the cursor.
	pick: Option<usize>,
	/// A warp asked for and not yet handed to a field.
	warp: Option<u32>,
	note: Option<(String, Instant)>,
	/// Free camera: on (`want`) and its pose (None until the first frame takes the drawn camera).
	want: bool,
	fc: Option<FreeCam>,
	reset: bool,
	frozen: bool,
	steps: u32,
	/// Speed step (wheel notches; speed = BASE_SPEED * WHEEL_STEP ^ step).
	speed: i32,
	mouse: (f32, f32),
	/// The gamepad buttons of the last frame (edges) and the ones kept from the game until released (the L3 + R3 that left).
	pad_prev: u16,
	pad_mask: u16,
	/// The readout line of the last frame.
	line: String,
	/// Debug mode was just switched On by the player: when (the notice is up for `NOTICE_FOR`).
	notice: Option<Instant>,
}

static UI: Mutex<Ui> = Mutex::new(Ui { pick: None, warp: None, note: None, want: false, fc: None, reset: false, frozen: false, steps: 0, speed: 0, mouse: (0.0, 0.0), pad_prev: 0, pad_mask: 0, line: String::new(), notice: None });
/// The free camera is on (read by the field input every field without the lock).
static FREE: AtomicBool = AtomicBool::new(false);

fn ui() -> std::sync::MutexGuard<'static, Ui> {
	UI.lock().unwrap_or_else(|e| e.into_inner())
}

fn note(u: &mut Ui, s: String) {
	eprintln!("retail: devtools: {s}");
	u.note = Some((s, Instant::now()));
}

fn level_name(l: usize) -> &'static str {
	crate::toc::LEVEL_NAMES.get(l).copied().unwrap_or("?")
}

/// Window thread: a key event. True = the tools took it (the caller drops it: it reaches neither the held keys nor the game).
/// Releases always pass, so a key held before the list opened is let go normally.
pub fn ui_key(k: K, pressed: bool, repeat: bool) -> bool {
	if !enabled() || !pressed {
		return false;
	}
	let mut u = ui();
	if let Some(row) = u.pick {
		let n = LEVELS;
		match k {
			K::ArrowUp | K::KeyW => u.pick = Some((row + n - 1) % n),
			K::ArrowDown | K::KeyS => u.pick = Some((row + 1) % n),
			K::PageUp => u.pick = Some(row.saturating_sub(5)),
			K::PageDown => u.pick = Some((row + 5).min(n - 1)),
			K::Home => u.pick = Some(0),
			K::End => u.pick = Some(n - 1),
			K::Enter | K::NumpadEnter | K::Space if !repeat => {
				u.pick = None;
				u.warp = Some(row as u32);
				note(&mut u, format!("WARP TO {row:02} {} ASKED", level_name(row).to_uppercase()));
			}
			K::Escape | K::Backspace | K::F1 if !repeat => u.pick = None,
			_ => {}
		}
		// Every press goes to the list while it is open.
		return true;
	}
	if repeat {
		return matches!(k, K::F1 | K::F2) || (u.want && free_key(k));
	}
	match k {
		K::F1 => {
			u.pick = Some((CURRENT.load(Ordering::Relaxed) as usize).min(LEVELS - 1));
			true
		}
		K::F2 => {
			toggle_free(&mut u);
			true
		}
		K::KeyP if u.want => {
			u.frozen = !u.frozen;
			let s = if u.frozen { "GAME FROZEN" } else { "GAME RUNNING" };
			note(&mut u, s.into());
			true
		}
		K::Period | K::NumpadDecimal if u.want => {
			if u.frozen {
				u.steps += 1;
			}
			true
		}
		K::KeyR if u.want => {
			u.reset = true;
			true
		}
		// The movement keys stay in the held set (the camera reads them there); the game gets a neutral pad anyway.
		_ => false,
	}
}

/// The keys the free camera reads from the held set (their auto-repeats are dropped).
fn free_key(k: K) -> bool {
	matches!(k, K::KeyW | K::KeyA | K::KeyS | K::KeyD | K::Space | K::KeyC | K::ArrowUp | K::ArrowDown | K::ArrowLeft | K::ArrowRight | K::ShiftLeft | K::ShiftRight)
}

fn toggle_free(u: &mut Ui) {
	u.want = !u.want;
	if u.want {
		u.fc = None;
		u.mouse = (0.0, 0.0);
		// The modern camera lets go of a held aim button (its release now goes nowhere).
		super::modcam::ui_release();
		note(u, "FREE CAMERA ON".into());
	} else {
		u.fc = None;
		u.frozen = false;
		u.steps = 0;
		note(u, "FREE CAMERA OFF".into());
	}
	FREE.store(u.want, Ordering::Relaxed);
}

/// Window thread: raw mouse motion. True = the free camera took it (the modern camera must not turn).
pub fn ui_mouse(dx: f32, dy: f32) -> bool {
	if !free_on() {
		return false;
	}
	let mut u = ui();
	u.mouse.0 += dx;
	u.mouse.1 += dy;
	true
}

/// Window thread: wheel notches (positive = away). True = the free camera took them (speed step).
pub fn ui_wheel(lines: f32) -> bool {
	if !free_on() {
		return false;
	}
	let mut u = ui();
	u.speed = (u.speed + lines.signum() as i32 * (lines.abs() > 0.0) as i32).clamp(-12, 16);
	true
}

/// The free camera is on: the game gets a neutral pad and the cursor is captured.
pub fn free_on() -> bool {
	enabled() && FREE.load(Ordering::Relaxed)
}

/// Gamepad buttons the game must not see: all while the free camera is on, else the toggle buttons until released.
pub fn pad_mask() -> u16 {
	if !enabled() {
		return 0;
	}
	if free_on() { 0xffff } else { ui().pad_mask }
}

/// The level retail is on (the picker opens there); set from the field input.
static CURRENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Window thread, building a field's input: the warp asked for, once, when retail can take it now (`gameplay`: game mode 0,
/// not the front end, no trip under way). Asked anywhere else it is dropped with a note.
pub fn take_warp(gameplay: bool, level: u32) -> Option<u32> {
	if !enabled() {
		return None;
	}
	CURRENT.store(level, Ordering::Relaxed);
	let mut u = ui();
	let to = u.warp.take()?;
	if gameplay {
		note(&mut u, format!("WARP {:02} {} -> {to:02} {}", level, level_name(level as usize).to_uppercase(), level_name(to as usize).to_uppercase()));
		Some(to)
	} else {
		note(&mut u, "WARP DROPPED: ONLY IN GAMEPLAY".into());
		None
	}
}

/// Viewer, before it steps the game: Some(accumulator) while the free camera holds the game frozen (just under one field, so
/// no field runs and the newest one is drawn; one field more per `.` press). None = the game runs as usual.
pub fn field_hold(field_dt: f32) -> Option<f32> {
	if !free_on() {
		return None;
	}
	let mut u = ui();
	if !u.frozen {
		return None;
	}
	Some(if u.steps > 0 {
		u.steps -= 1;
		field_dt * 1.999
	} else {
		field_dt * 0.999
	})
}

/// Viewer, once per frame after the drawn camera is final: polls the gamepad (L3 + R3 toggles the free camera), steps the
/// free camera and returns its view, or None while it is off. `drawn` is the camera the frame would draw (the free camera
/// starts there and R goes back there).
pub fn frame(dt: f32, held: impl Fn(K) -> bool, drawn: Cam) -> Option<Cam> {
	if !enabled() {
		return None;
	}
	let pad = super::prompts::pad_peek();
	let mut u = ui();
	let (pb, ps) = pad.map_or((0u16, [0x80u8; 4]), |p| (p.buttons, p.sticks));
	let edge = pb & !u.pad_prev;
	u.pad_prev = pb;
	u.pad_mask &= pb;
	if pb & (PAD_L3 | PAD_R3) == PAD_L3 | PAD_R3 && edge & (PAD_L3 | PAD_R3) != 0 {
		toggle_free(&mut u);
		if !u.want {
			// The game sees neither stick click until both are released.
			u.pad_mask = pb;
		}
	}
	if !u.want {
		return None;
	}
	if edge & PAD_START != 0 {
		u.frozen = !u.frozen;
		let s = if u.frozen { "GAME FROZEN" } else { "GAME RUNNING" };
		note(&mut u, s.into());
	}
	if edge & PAD_SELECT != 0 && u.frozen {
		u.steps += 1;
	}
	if edge & PAD_TRI != 0 {
		u.reset = true;
	}
	if u.fc.is_none() || std::mem::take(&mut u.reset) {
		u.fc = Some(FreeCam { pos: drawn.0, yaw: drawn.1, pitch: drawn.2, tan_half: drawn.3 });
	}
	let mouse = std::mem::take(&mut u.mouse);
	let speed = u.speed;
	let mut fc = u.fc?;
	step(&mut fc, dt, &held, mouse, pb, ps, speed);
	u.fc = Some(fc);
	Some((fc.pos, fc.yaw, fc.pitch, fc.tan_half))
}

fn stick(v: u8) -> f32 {
	let d = v as i32 - 0x80;
	if d.abs() <= STICK_DEAD { 0.0 } else { (d - d.signum() * STICK_DEAD) as f32 / (127 - STICK_DEAD) as f32 }
}

/// One frame of free flight: keys / mouse / pad -> pose. Movement is along the view (forward follows the pitch), up / down
/// along world z.
fn step(fc: &mut FreeCam, dt: f32, held: &impl Fn(K) -> bool, mouse: (f32, f32), pb: u16, ps: [u8; 4], speed: i32) {
	let key = |k| held(k) as i32 as f32;
	let dt = dt.clamp(0.0, 0.1);
	// Look: mouse, arrows, right stick ([rx, ry, lx, ly], y grows downward).
	fc.yaw -= mouse.0 * MOUSE_RATE + (key(K::ArrowRight) - key(K::ArrowLeft)) * KEY_TURN * dt + stick(ps[0]) * STICK_TURN * dt;
	fc.pitch -= mouse.1 * MOUSE_RATE + (key(K::ArrowDown) - key(K::ArrowUp)) * KEY_TURN * dt + stick(ps[1]) * STICK_TURN * dt;
	fc.pitch = fc.pitch.clamp(-PITCH_MAX, PITCH_MAX);
	fc.yaw = fc.yaw.rem_euclid(std::f32::consts::TAU);
	// Move: WASD + left stick, Space / C + R1 / L1.
	let fwd_in = (key(K::KeyW) - key(K::KeyS) - stick(ps[3])).clamp(-1.0, 1.0);
	let side_in = (key(K::KeyD) - key(K::KeyA) + stick(ps[2])).clamp(-1.0, 1.0);
	let up_in = (key(K::Space) - key(K::KeyC) + (pb & PAD_R1 != 0) as i32 as f32 - (pb & PAD_L1 != 0) as i32 as f32).clamp(-1.0, 1.0);
	let mut v = BASE_SPEED * WHEEL_STEP.powi(speed);
	if held(K::ShiftLeft) || held(K::ShiftRight) || pb & PAD_R2 != 0 {
		v *= FAST;
	}
	if pb & PAD_L2 != 0 {
		v *= SLOW;
	}
	let (sy, cy) = fc.yaw.sin_cos();
	let (sp, cp) = fc.pitch.sin_cos();
	let f = [cy * cp, sy * cp, sp];
	let r = [sy, -cy, 0.0];
	for i in 0..3 {
		fc.pos[i] += (f[i] * fwd_in + r[i] * side_in) * v * dt;
	}
	fc.pos[2] += up_in * v * dt;
}

/// The readout: level, Ratchet's position, the free camera's position and state, the last event.
pub fn readout(level: usize, hero: [f32; 3]) -> String {
	if !enabled() {
		return String::new();
	}
	let mut u = ui();
	let mut s = format!("L{level:02} {} - HERO {:.1} {:.1} {:.1}", level_name(level).to_uppercase(), hero[0], hero[1], hero[2]);
	if let (true, Some(fc)) = (u.want, u.fc) {
		s += &format!(" - FREECAM {:.1} {:.1} {:.1} YAW {:.0} PITCH {:.0} SPEED {:.1}{}", fc.pos[0], fc.pos[1], fc.pos[2], fc.yaw.to_degrees(), fc.pitch.to_degrees(), BASE_SPEED * WHEEL_STEP.powi(u.speed), if u.frozen { " FROZEN" } else { "" });
	}
	if let Some((t, at)) = &u.note {
		if at.elapsed() < NOTE_FOR {
			s += " - ";
			s += t;
		}
	}
	u.line = s.clone();
	s
}

/// What the window title gets appended (with the tools off: nothing).
pub fn title_suffix(level: usize, hero: [f32; 3]) -> String {
	if !enabled() {
		return String::new();
	}
	let s = readout(level, hero);
	let pick = ui().pick;
	match pick {
		Some(row) => format!(" - DEV {s} - WARP TO > {row:02} {} <", level_name(row)),
		None => format!(" - DEV {s}"),
	}
}

/// The Debug mode notice: a dark panel at the top left with the keys, the warning in red (window pixels).
fn draw_notice(out: &mut Vec<crate::overlay::Quad>, k: f32, y0: f32) {
	let size = 13.0 * k;
	let lh = size * 1.45;
	let w = NOTICE.iter().map(|l| crate::overlay::text_width(l, size)).fold(0.0, f32::max) + 24.0 * k;
	crate::overlay::rect(out, [6.0 * k, y0, w, NOTICE.len() as f32 * lh + 12.0 * k], [0.0, 0.0, 0.0, 0.75]);
	for (i, l) in NOTICE.iter().enumerate() {
		let colour = if i == 0 { [1.0, 0.8, 0.3, 1.0] } else if i >= NOTICE.len() - 2 { [1.0, 0.55, 0.45, 1.0] } else { [0.92, 0.92, 0.92, 1.0] };
		crate::overlay::text(out, l, 12.0 * k, y0 + 6.0 * k + i as f32 * lh, size, colour);
	}
}

/// The on-screen part (window pixels): the readout line while a tool is up, and the planet list while it is open.
pub fn draw(out: &mut Vec<crate::overlay::Quad>, w: f32, h: f32, level: usize, hero: [f32; 3]) {
	if !enabled() {
		return;
	}
	readout(level, hero);
	let u = ui();
	let k = (h / 720.0).max(0.6);
	let size = 16.0 * k;
	if u.notice.is_some_and(|at| at.elapsed() < NOTICE_FOR) {
		draw_notice(out, k, 8.0 * k + size + 12.0 * k);
	}
	let shown = u.want || u.pick.is_some() || u.note.as_ref().is_some_and(|(_, at)| at.elapsed() < NOTE_FOR);
	if shown && !u.line.is_empty() {
		let line = u.line.as_str();
		crate::overlay::rect(out, [6.0 * k, 6.0 * k, crate::overlay::text_width(line, size) + 12.0 * k, size + 8.0 * k], [0.0, 0.0, 0.0, 0.55]);
		crate::overlay::text(out, line, 12.0 * k, 10.0 * k, size, [1.0, 1.0, 0.6, 1.0]);
	}
	if u.want && u.pick.is_none() {
		let hs = size * 0.75;
		crate::overlay::text(out, HELP_FREE, 12.0 * k, h - 12.0 * k - hs, hs, [0.85, 0.85, 0.85, 1.0]);
	}
	let Some(row) = u.pick else { return };
	let line_h = size * 1.35;
	let rows = LEVELS as f32 + 2.5;
	let pw = crate::overlay::text_width(HELP_PICK, size * 0.8).max(crate::overlay::text_width("00  DREK'S FLEET (NEBULA G34)  ", size)) + 32.0 * k;
	let ph = rows * line_h + 24.0 * k;
	let (x0, y0) = ((w - pw) / 2.0, ((h - ph) / 2.0).max(4.0));
	crate::overlay::rect(out, [x0, y0, pw, ph], [0.0, 0.0, 0.0, 0.75]);
	crate::overlay::text(out, "WARP TO PLANET (RETAIL LEVEL LOAD)", x0 + 16.0 * k, y0 + 12.0 * k, size, [1.0, 0.8, 0.3, 1.0]);
	let cur = CURRENT.load(Ordering::Relaxed) as usize;
	for l in 0..LEVELS {
		let y = y0 + 12.0 * k + (l as f32 + 1.4) * line_h;
		if l == row {
			crate::overlay::rect(out, [x0 + 8.0 * k, y - 3.0 * k, pw - 16.0 * k, line_h], [0.25, 0.45, 0.9, 0.8]);
		}
		let t = format!("{l:02}  {}{}", level_name(l), if l == cur { "  (HERE)" } else { "" });
		crate::overlay::text(out, &t, x0 + 16.0 * k, y, size, if l == row { [1.0, 1.0, 1.0, 1.0] } else { [0.8, 0.8, 0.8, 1.0] });
	}
	crate::overlay::text(out, HELP_PICK, x0 + 16.0 * k, y0 + 12.0 * k + (LEVELS as f32 + 1.6) * line_h, size * 0.8, [0.7, 0.7, 0.7, 1.0]);
}

/// Test feed (no keyboard): RAC_DEVTOOLS_FEED=`FIELD:ACTION;...` drives the tools from the field input at that field number:
/// `wN` warp to level N, `lN` the planet list open at row N, `f` free camera on/off, `p` freeze, `s` one step, `mDX,DY` mouse counts, `oF,R,U` moves the free camera that
/// far forward / right / up, `vS` speed steps (keys for the list come from RAC_RETAIL_KEYTAP), `cX,Y,Z,YAW,PITCH` puts the free camera at a pose (degrees).
pub fn feed_field(field: u32) {
	if !enabled() {
		return;
	}
	static F: OnceLock<Vec<(u32, String)>> = OnceLock::new();
	let feed = F.get_or_init(|| {
		std::env::var("RAC_DEVTOOLS_FEED").unwrap_or_default().split(';').filter_map(|p| {
			let (f, a) = p.split_once(':')?;
			Some((f.trim().parse().ok()?, a.trim().to_string()))
		}).collect()
	});
	for (_, a) in feed.iter().filter(|e| e.0 == field) {
		let mut u = ui();
		let (c, rest) = a.split_at(a.char_indices().nth(1).map_or(a.len(), |(i, _)| i));
		let nums: Vec<f32> = rest.split(',').filter_map(|x| x.trim().parse().ok()).collect();
		match c {
			"w" => {
				if let Some(&n) = nums.first() {
					u.warp = Some(n as u32);
					note(&mut u, format!("FEED WARP {n}"));
				}
			}
			"l" => u.pick = Some((nums.first().copied().unwrap_or(0.0) as usize).min(LEVELS - 1)),
			"f" => toggle_free(&mut u),
			"p" => u.frozen = !u.frozen,
			"s" => u.steps += 1,
			"m" if nums.len() == 2 => {
				u.mouse.0 += nums[0];
				u.mouse.1 += nums[1];
			}
			"o" if nums.len() == 3 => {
				if let Some(fc) = u.fc.as_mut() {
					let (sy, cy) = fc.yaw.sin_cos();
					let (sp, cp) = fc.pitch.sin_cos();
					let (f, r) = ([cy * cp, sy * cp, sp], [sy, -cy, 0.0]);
					for i in 0..3 {
						fc.pos[i] += f[i] * nums[0] + r[i] * nums[1];
					}
					fc.pos[2] += nums[2];
				}
			}
			"v" if !nums.is_empty() => u.speed = nums[0] as i32,
			"c" if nums.len() == 5 && u.want => {
				let t = u.fc.map_or(0.7, |f| f.tan_half);
				u.fc = Some(FreeCam { pos: [nums[0], nums[1], nums[2]], yaw: nums[3].to_radians(), pitch: nums[4].to_radians(), tan_half: t });
			}
			_ => {}
		}
		eprintln!("retail: devtools: field {field}: feed {a}");
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn step_moves_along_the_view_and_turns_with_the_mouse() {
		let none = |_: K| false;
		let mut fc = FreeCam { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: 0.7 };
		let w = |k: K| k == K::KeyW;
		step(&mut fc, 0.1, &w, (0.0, 0.0), 0, [0x80; 4], 0);
		assert!((fc.pos[0] - BASE_SPEED * 0.1).abs() < 1e-4 && fc.pos[1].abs() < 1e-5 && fc.pos[2].abs() < 1e-5, "{:?}", fc.pos);
		// D strafes right: yaw 0 looks +x, right is -y.
		let d = |k: K| k == K::KeyD;
		step(&mut fc, 0.1, &d, (0.0, 0.0), 0, [0x80; 4], 0);
		assert!(fc.pos[1] < -1.0);
		// Mouse right turns right (yaw down), mouse down looks down; the pitch stops short of straight down.
		step(&mut fc, 0.0, &none, (100.0, 10_000.0), 0, [0x80; 4], 0);
		assert!(fc.yaw > std::f32::consts::PI && (fc.pitch + PITCH_MAX).abs() < 1e-6);
		// R1 rises along world z whatever the pitch; R2 is 4x.
		let z0 = fc.pos[2];
		step(&mut fc, 0.1, &none, (0.0, 0.0), PAD_R1 | PAD_R2, [0x80; 4], 0);
		assert!((fc.pos[2] - z0 - BASE_SPEED * FAST * 0.1).abs() < 1e-3);
	}

	#[test]
	fn stick_has_a_dead_zone_and_reaches_one() {
		assert_eq!(stick(0x80), 0.0);
		assert_eq!(stick(0x80 + STICK_DEAD as u8), 0.0);
		assert!((stick(0xff) - 1.0).abs() < 1e-6);
		assert!((stick(0x00) + 1.0).abs() < 0.01);
	}
}
