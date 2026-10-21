//! Native movies for retail mode (no IPU/MPEG emulation in the oracle).
//!
//! The retail movie player is FUN_xxxxxxxx(lba, bytes, buf_a, buf_b, lang) with one copy per level overlay (the
//! boot image's copy, FUN_00239ec8, is overwritten by the level overlay at run time: in Eudora RAM 0x239ec8 holds
//! other code). Eudora: 0x2f13e8..0x2f1490 (twin of Novalis FUN_00319b90's callee), Rilgar: 0x32b770..0x32b818. The
//! in-game caller passes the lba/bytes pair from the table 0x1394b8 (NTSC) / 0x139518 (PAL, 0x15ed80 = 1) and lang
//! from 0x15ed88 (boot data, valid in every level). It is synchronous: it sets up a sceMpeg-style
//! IPU decoder (FUN_0023c5d8/0x23c6e0, thread FUN_0023c938, INTC 2 and DMAC 2 handlers) and returns when the stream
//! ended or Start was pressed. The oracle has no IPU, so the interpreter spins in it (pc inside the pad parser the
//! player polls, "SPIN ... pc 0025939c" in the eudora1 window 0041 verify run, game mode 1 = 0x15f5a4).
//!
//! This hook replaces that one function. The game thread asks rac-rs to play the PSS and waits until it is done
//! (or skipped); nothing else is faked: the caller (FUN_00231118) restores the screen and sound itself after the
//! player returns, exactly as after a retail movie. Returns v0 = 0 (the retail return value).
//!
//! Wiring (retail.rs / handler.rs, applied by the owner of those files):
//!   * `fmv_bridge::register()` next to `snd::register()` in `Retail::load` (before `enable_rec`).
//!   * `fmv_bridge::set_serviced(true)` once the window can play movies (viewer started with an ISO); without it
//!     the hook skips every movie at once (tests, probes, headless runs) so nothing ever waits for nobody.
//!   * `Retail::tick` must not block in `rx.recv()` while a request is pending: poll `fmv_bridge::take_request()`
//!     between `rx.recv_timeout(2 ms)` attempts; on a request, hand it to the viewer and return early.
//!   * the viewer plays it like the attract movie (`fmv::player::Player::start`), and calls `fmv_bridge::finish()`
//!     when `player.finished` or Start / Esc is pressed.
//!   * `Retail::drop` calls `fmv_bridge::abort()` before joining the worker (a worker waiting in the hook would
//!     otherwise never exit).
use ee_oracle::cpu::Cpu;
use std::sync::{Condvar, Mutex};

/// One copy of the retail movie player: the same 42-instruction function lives once per level overlay, once in the
/// boot ELF (title / logo movie) and once in the frontbin overlay (attract movies, New Game), at different addresses
/// and with different jal targets / global offsets, so every copy has its own hash.
pub struct Player {
	/// Level number 0..18, or `FRONT` / `BOOT`.
	pub level: u32,
	pub name: &'static str,
	pub entry: u32,
	pub end: u32,
	/// FNV hash of the code words as `ee_oracle::rec::hash` computes it (equals the ELF's PT_LOAD words).
	pub hash: u64,
}

/// `Player::level` of the frontbin overlay copy (title screen, attract movies, New Game intro movies).
pub const FRONT: u32 = 100;
/// `Player::level` of the boot ELF copy (the logo movie before the front end overlay is loaded).
pub const BOOT: u32 = 101;

/// Entry / hash of every copy, computed from the ELF code words (PT_LOAD) of `extracted/levels/<n>_<name>/overlay.elf`,
/// `extracted/globals/frontbin/frontbin.elf` and `extracted/boot_elf.elf` by matching the Eudora player's 42 words with
/// the jal targets and the lw/sw offsets masked (each file has exactly one such function). Eudora's hash was also read
/// from RAM of states s07 and fg1/4_0159; Rilgar's is the one that was computed from its overlay.elf. The test
/// `players_match_elfs` re-derives all of them when the extracted files are present.
pub const PLAYERS: [Player; 21] = [
	Player { level: 0, name: "veldin_tutorial", entry: 0x2ed6d0, end: 0x2ed778, hash: 0x5ba1_5322_13d4_d30c },
	Player { level: 1, name: "novalis", entry: 0x319b90, end: 0x319c38, hash: 0xc7ad_5d0c_cc71_a6dc },
	Player { level: 2, name: "aridia", entry: 0x2fc2b8, end: 0x2fc360, hash: 0x94d8_295b_3a28_6e46 },
	Player { level: 3, name: "kerwan", entry: 0x2ed3c8, end: 0x2ed470, hash: 0x0ded_5c58_aad6_94ca },
	Player { level: 4, name: "eudora", entry: 0x2f13e8, end: 0x2f1490, hash: 0x965c_f8c5_a4b0_3b52 },
	Player { level: 5, name: "rilgar", entry: 0x32b770, end: 0x32b818, hash: 0xdc30_bf36_eff2_0e94 },
	Player { level: 6, name: "nebula_g34", entry: 0x316318, end: 0x3163c0, hash: 0xdfa6_8f5f_a0e2_da9e },
	Player { level: 7, name: "umbris", entry: 0x329300, end: 0x3293a8, hash: 0xdf6f_8f94_fa91_bf78 },
	Player { level: 8, name: "batalia", entry: 0x316c98, end: 0x316d40, hash: 0x9bd3_909b_99dd_aa3e },
	Player { level: 9, name: "gaspar", entry: 0x315710, end: 0x3157b8, hash: 0x64eb_67f2_8fd5_85fc },
	Player { level: 10, name: "orxon", entry: 0x2f68e0, end: 0x2f6988, hash: 0xf3a6_ec18_e419_72d0 },
	Player { level: 11, name: "pokitaru", entry: 0x3288d0, end: 0x328978, hash: 0xaaab_f7cb_c872_13cc },
	Player { level: 12, name: "hoven", entry: 0x316698, end: 0x316740, hash: 0x8fce_7544_7a18_9bbe },
	Player { level: 13, name: "oltanis_orbit", entry: 0x318028, end: 0x3180d0, hash: 0xe050_f1fb_8964_4dc2 },
	Player { level: 14, name: "oltanis", entry: 0x316c00, end: 0x316ca8, hash: 0xf086_9706_2d70_c978 },
	Player { level: 15, name: "quartu", entry: 0x2f8d70, end: 0x2f8e18, hash: 0x693e_1b8b_9baa_1a54 },
	Player { level: 16, name: "kalebo_iii", entry: 0x2f57e8, end: 0x2f5890, hash: 0x7e7c_b450_3739_da12 },
	Player { level: 17, name: "veldin_orbit", entry: 0x300848, end: 0x3008f0, hash: 0x94de_2136_8b35_4d2a },
	Player { level: 18, name: "veldin_finale", entry: 0x306a48, end: 0x306af0, hash: 0x76a2_0dd7_51c4_0faa },
	Player { level: FRONT, name: "frontbin", entry: 0x239880, end: 0x239928, hash: 0x167e_6217_a13a_cbd8 },
	Player { level: BOOT, name: "boot", entry: 0x239ec8, end: 0x239f70, hash: 0x3c23_96fd_f89d_3d8a },
];

/// The players of the disc that runs (`PLAYERS` PAL, retail/disc.rs `PLAYERS_USA`).
pub fn players() -> &'static [Player] {
	if super::disc::usa() { &super::disc::PLAYERS_USA } else { &PLAYERS }
}

/// Which presses end a movie early. Retail's player (boot FUN_00239f70, lines 77746-77760) reads these words each field:
/// `0x15eed8` (-1 the boot logos, 2 the attract movie, 0 the story movies), `0x15eea0` / `0x15ee20` (zero in a normal
/// run) and the level word `0x15ed84`; `0x13cae4` is the buttons newly pressed this field, `0x13cae0` the held ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
	/// The boot logo movies: nothing skips them.
	Never,
	/// The attract movie: any newly pressed button.
	Any,
	/// The New Game intro movies (level word 0): Start newly pressed.
	Start,
	/// A story movie past Veldin in a normal run: Start newly pressed while L1 + L2 + R1 + R2 are held.
	Combo,
}

impl Skip {
	/// The rule from the words the player reads (`0x15eed8`, `0x15eea0`, `0x15ee20`, `0x15ed84`).
	pub fn from_words(a: i32, e0: u32, e1: u32, level: i32) -> Skip {
		if a == -1 {
			Skip::Never
		} else if a == 2 {
			Skip::Any
		} else if a == 0 && e0 == 0 && e1 == 0 && level > 0 {
			Skip::Combo
		} else {
			Skip::Start
		}
	}

	/// The pad bits that satisfy this rule's test: what Esc / Space stand for while a movie plays (the Start press, plus the
	/// four shoulder buttons where the rule wants them held). The attract movie takes any press already, the logos none.
	pub fn feed(self) -> u16 {
		match self {
			Skip::Never | Skip::Any => 0,
			Skip::Start => 0x800,
			Skip::Combo => 0x80f,
		}
	}

	/// Whether the pad `now` ends the movie when the previous poll read `prev`: only a button that was not down at the
	/// previous poll counts (a key held from before the movie, or held through the last one, skips nothing).
	pub fn pressed(self, prev: u16, now: u16) -> bool {
		let new = now & !prev;
		match self {
			Skip::Never => false,
			Skip::Any => new != 0,
			Skip::Start => new & 0x800 != 0,
			Skip::Combo => new & 0x800 != 0 && now & 0xf == 0xf,
		}
	}
}

/// One movie the game asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
	/// First disc sector (2048-byte) of the PSS.
	pub lba: u32,
	/// Stream length in bytes as the game's table gives it.
	pub bytes: u32,
	/// The game's language value (0x15ed88: 0 English, 2 French, 3 German, 4 Spanish, 5 Italian = the PSS audio track).
	pub lang: u32,
	/// What ends it early (read from the game's words when it was requested).
	pub skip: Skip,
}

#[derive(Default)]
struct State {
	serviced: bool,
	abort: bool,
	/// Asked for, not yet taken by the window.
	req: Option<Request>,
	/// The game thread is inside the hook until `finish`.
	playing: bool,
	/// Movies played / skipped without a window (tests).
	played: u32,
	skipped: u32,
}

static ST: Mutex<State> = Mutex::new(State { serviced: false, abort: false, req: None, playing: false, played: 0, skipped: 0 });
static CV: Condvar = Condvar::new();

fn st() -> std::sync::MutexGuard<'static, State> {
	ST.lock().unwrap_or_else(|e| e.into_inner())
}

/// The hooks (registered with the oracle's table; `ee_game::register` first, like `snd::register`).
static HOOKS: [ee_oracle::rec::Hand; PLAYERS.len()] = {
	const NONE: ee_oracle::rec::Hand = ee_oracle::rec::Hand { entry: 0, end: 0, hash: 0, est: 0, f: movie_call };
	let mut v = [NONE; PLAYERS.len()];
	let mut i = 0;
	while i < PLAYERS.len() {
		v[i] = ee_oracle::rec::Hand { entry: PLAYERS[i].entry, end: PLAYERS[i].end, hash: PLAYERS[i].hash, est: 0, f: movie_call };
		i += 1;
	}
	v
};

/// Logs, for a level whose gameplay just started, whether the movie player in RAM has the hash the hook gates on (a
/// mismatch means that level's movies would run the retail player, which spins without IPU emulation).
pub fn check_level(m: &ee_oracle::Machine, level: u32) -> bool {
	let ram = |p: &Player| ee_oracle::rec::hash(&m.cpu.mem, p.entry, p.end);
	let mine = players().iter().find(|p| p.level == level);
	let ok = mine.is_some_and(|p| ram(p) == p.hash);
	match mine {
		Some(p) => eprintln!("fmv_bridge: level {level} movie player hook {} ({} {:06x} ram {:016x} want {:016x})", if ok { "MATCH" } else { "NO MATCH" }, p.name, p.entry, ram(p), p.hash),
		None => eprintln!("fmv_bridge: level {level} has no movie player entry (NO MATCH)"),
	}
	ok
}

pub fn register() {
	if super::disc::usa() {
		ee_oracle::rec::register_hand(&super::disc::HOOKS_USA);
	} else {
		ee_oracle::rec::register_hand(&HOOKS);
	}
}

/// A new game start (`Retail::load`): clears an `abort` left by the previous Retail's drop (and any stale request).
pub fn reset() {
	let mut s = st();
	s.abort = false;
	s.playing = false;
	s.req = None;
}

/// The window can play movies (and will call `finish`). RAC_RETAIL_FMV=0 turns it off (every movie is skipped).
pub fn set_serviced(on: bool) {
	let off = std::env::var("RAC_RETAIL_FMV").is_ok_and(|v| v == "0");
	st().serviced = on && !off;
}

/// A movie the game is waiting for, once (main thread).
pub fn take_request() -> Option<Request> {
	st().req.take()
}

/// True while the game thread waits in the hook (asked for or being played). The main thread must keep presenting
/// movie frames (not wait for a field) while this holds.
pub fn waiting() -> bool {
	let s = st();
	s.playing
}

/// [PC] Run-ahead (retail.rs `Retail::ahead`): a field the worker was handed before a movie began waits here until the
/// movie ended (or the window went away), as `Retail::tick` holds back every later field while one plays; in lockstep no
/// field is handed out meanwhile and this returns at once.
pub fn hold_field() {
	wait_finish();
}

/// The movie ended or was skipped: the game thread returns from the player.
pub fn finish() {
	let mut s = st();
	s.playing = false;
	s.req = None;
	s.played += 1;
	CV.notify_all();
}

/// The window is going away: release the game thread and skip every later movie.
pub fn abort() {
	let mut s = st();
	s.abort = true;
	s.playing = false;
	s.req = None;
	CV.notify_all();
}

/// (played, skipped without a window), for tests.
pub fn counts() -> (u32, u32) {
	let s = st();
	(s.played, s.skipped)
}


/// Queues the request (true) or counts a skip (false: no window, or aborted).
fn queue(r: Request) -> bool {
	let mut s = st();
	if !s.serviced || s.abort {
		s.skipped += 1;
		return false;
	}
	s.req = Some(r);
	s.playing = true;
	true
}

fn wait_finish() {
	let mut s = st();
	while s.playing && !s.abort {
		s = CV.wait(s).unwrap_or_else(|e| e.into_inner());
	}
}

/// The hook: a0 = lba, a1 = bytes, t0 = lang (retail's calling convention); v0 = 0 on return.
///
/// While a window plays the movie, the game thread first completes the current field through the oracle's vblank
/// callback (a real PS2 keeps taking vblanks while its movie player runs): the host's `Machine::frame` returns
/// instead of hitting its 20 s spin watchdog, and the callback parks the game thread until the host's next
/// `frame`, which `Retail::tick` holds back until `finish` (or the drop's abort). Without a window (replays, tests,
/// acceptance) the movie is skipped at once and no field is added, so recorded runs stay exact.
pub fn movie_call(c: &mut Cpu) -> bool {
	let skip = Skip::from_words(c.mem.rd32(0x15eed8) as i32, c.mem.rd32(0x15eea0), c.mem.rd32(0x15ee20), c.mem.rd32(0x15ed84) as i32);
	let r = Request { lba: c.g(4) as u32, bytes: c.g(5) as u32, lang: c.g(8) as u32, skip };
	if queue(r) {
		if let Some(mut f) = c.vblank_cb.take() {
			f(c);
			if c.vblank_cb.is_none() {
				c.vblank_cb = Some(f);
			}
			// A rebuilt recompiled table (`rec_unwind`) stays pending: the next vblank wait unwinds at a safe point.
		}
		wait_finish();
	}
	c.s(2, 0);
	true
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The movie skip rule of retail's player: the attract movie takes any new press, the New Game intro movies only a new
	/// Start press (Cross / Space does nothing), a story movie past Veldin the L1 + L2 + R1 + R2 + Start combo, the boot
	/// logos nothing; a button already down at the previous poll never counts.
	#[test]
	fn skip_rule_follows_retail() {
		const START: u16 = 0x800;
		const CROSS: u16 = 0x40;
		assert_eq!(Skip::from_words(-1, 0, 0, 0), Skip::Never);
		assert_eq!(Skip::from_words(2, 0, 0, 0), Skip::Any);
		assert_eq!(Skip::from_words(0, 0, 0, 0), Skip::Start, "level 0: the New Game intro");
		assert_eq!(Skip::from_words(0, 0, 0, 1), Skip::Combo, "past Veldin in a normal run");
		assert_eq!(Skip::from_words(0, 1, 0, 1), Skip::Start, "0x15eea0 set");
		assert_eq!(Skip::from_words(0, 0, 1, 1), Skip::Start, "0x15ee20 set");
		// New Game intro: Start newly pressed skips, a held Start or any other button does not.
		assert!(Skip::Start.pressed(0, START));
		assert!(!Skip::Start.pressed(START, START), "held from before");
		assert!(!Skip::Start.pressed(0, CROSS), "Cross / Space is no skip");
		assert!(Skip::Start.pressed(CROSS, CROSS | START));
		// Attract: any new button.
		assert!(Skip::Any.pressed(0, CROSS));
		assert!(!Skip::Any.pressed(CROSS, CROSS));
		// Logos: never.
		assert!(!Skip::Never.pressed(0, START | CROSS));
		// Combo: Start newly pressed with all four shoulder buttons held.
		assert!(!Skip::Combo.pressed(0, START));
		assert!(Skip::Combo.pressed(0xf, 0xf | START));
		assert!(!Skip::Combo.pressed(0xf | START, 0xf | START));
	}

	/// Reads the PT_LOAD segments of an ELF as (vaddr, bytes).
	fn load(path: &std::path::Path) -> Option<Vec<(u32, Vec<u8>)>> {
		let d = std::fs::read(path).ok()?;
		let u32_at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
		let u16_at = |o: usize| u16::from_le_bytes(d[o..o + 2].try_into().unwrap()) as usize;
		let (phoff, phsz, phn) = (u32_at(28) as usize, u16_at(42), u16_at(44));
		let mut v = Vec::new();
		for i in 0..phn {
			let o = phoff + i * phsz;
			if u32_at(o) == 1 {
				let (off, va, fs) = (u32_at(o + 4) as usize, u32_at(o + 8), u32_at(o + 16) as usize);
				v.push((va, d[off..off + fs].to_vec()));
			}
		}
		Some(v)
	}

	/// Every PLAYERS hash equals the FNV of the code words in the ELF it belongs to, and the words look like the player
	/// (frame setup first, `jr ra` / `addiu sp,sp,0x40` last, `lui 0xd / ori 0x9048` inside). Skipped when the
	/// extracted game files are not on this machine.
	#[test]
	fn players_match_elfs() {
		let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../extracted"));
		let dirs = ["0_veldin_tutorial", "1_novalis", "2_aridia", "3_kerwan", "4_eudora", "5_rilgar", "6_nebula_g34", "7_umbris", "8_batalia", "9_gaspar", "10_orxon", "11_pokitaru", "12_hoven", "13_oltanis_orbit", "14_oltanis", "15_quartu", "16_kalebo_iii", "17_veldin_orbit", "18_veldin_finale"];
		let mut checked = 0;
		for p in PLAYERS.iter() {
			let path = match p.level {
				FRONT => root.join("globals/frontbin/frontbin.elf"),
				BOOT => root.join("boot_elf.elf"),
				l => root.join("levels").join(dirs[l as usize]).join("overlay.elf"),
			};
			let Some(segs) = load(&path) else { continue };
			let w = |a: u32| -> u32 {
				let (va, b) = segs.iter().find(|(va, b)| a >= *va && (a - va) as usize + 4 <= b.len()).expect("player inside a PT_LOAD");
				let o = (a - va) as usize;
				u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
			};
			let mut h = 0xcbf2_9ce4_8422_2325u64;
			for a in (p.entry..p.end).step_by(4) {
				h = (h ^ w(a) as u64).wrapping_mul(0x100_0000_01b3);
			}
			assert_eq!(h, p.hash, "{} hash", p.name);
			assert_eq!(p.end - p.entry, 0xa8, "{} size", p.name);
			assert_eq!(w(p.entry), 0x27bd_ffc0, "{} prologue", p.name);
			assert_eq!(w(p.end - 8), 0x03e0_0008, "{} jr ra", p.name);
			assert_eq!(w(p.end - 4), 0x27bd_0040, "{} sp restore", p.name);
			assert!((p.entry..p.end).step_by(4).any(|a| w(a) == 0x3442_9048), "{} ori 0x9048", p.name);
			checked += 1;
		}
		eprintln!("players_match_elfs: {checked} of {} checked", PLAYERS.len());
	}

	/// Dispatch proof for one savestate (RAC_TEST_STATE=dir, e.g. oracle/states/fg1/5_0246): the level's player has the
	/// hook's hash in RAM, the recompiled table dispatches the hook at its entry, and calling the retail entry returns
	/// at once with v0 = 0 and counts one skipped movie (without the hook the interpreter would spin in the player).
	/// Run: cargo test --release hook_dispatch_in_state -- --ignored --nocapture
	#[test]
	#[ignore]
	fn hook_dispatch_in_state() {
		let Ok(dir) = std::env::var("RAC_TEST_STATE") else { return };
		let mut m = ee_oracle::Machine::load(&dir);
		ee_game::register();
		reset();
		set_serviced(false);
		register();
		let n = m.enable_rec();
		let level = m.level();
		let p = PLAYERS.iter().find(|p| p.level == level).expect("a player entry for the state's level");
		assert!(check_level(&m, level), "RAM hash of the player equals PLAYERS");
		let has = m.cpu.rec.as_ref().is_some_and(|t| t.has_hand(p.entry));
		eprintln!("hook_dispatch_in_state: level {level} {} {:06x} recompiled fns {n} table dispatches hook {has}", p.name, p.entry);
		assert!(has, "the recompiled table dispatches the hook (a level without crates does not)");
		m.cpu.s(4, 386668);
		m.cpu.s(5, 38518788);
		m.cpu.s(8, 3);
		m.cpu.s(2, 0x1234);
		let before = counts().1;
		m.cpu.call(p.entry);
		assert_eq!(m.cpu.g(2), 0, "v0");
		assert_eq!(counts().1, before + 1, "one movie skipped through the hook");
		eprintln!("hook_dispatch_in_state: call returned, v0 = 0, skipped movies {}", counts().1);
		// With a window: the game thread asks for the movie (lba / bytes / lang as the registers gave them) and stays
		// in the hook until the window reports the end.
		set_serviced(true);
		m.cpu.s(4, 735525);
		m.cpu.s(5, 22315012);
		m.cpu.s(8, 2);
		let (played, cpu) = (counts().0, &mut m.cpu);
		let got = std::thread::scope(|sc| {
			let t = sc.spawn(|| cpu.call(p.entry));
			let q = loop {
				if let Some(q) = take_request() {
					break q;
				}
				assert!(!t.is_finished(), "the hook returned before the window took the request");
				std::thread::sleep(std::time::Duration::from_millis(1));
			};
			assert!(waiting());
			finish();
			t.join().unwrap();
			q
		});
		assert_eq!((got.lba, got.bytes, got.lang), (735525, 22315012, 2));
		assert_eq!((m.cpu.g(2), counts().0), (0, played + 1));
		eprintln!("hook_dispatch_in_state: windowed call asked {got:?}, returned after finish, v0 = 0");
	}
}
