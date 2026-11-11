//! Fast travel (v0.5.1, lane fload; modern menu Game -> Gameplay "Fast travel", classic Graphics -> Gameplay row, options file
//! key `fasttravel` = on / off). Default On for players; Off = retail: nothing runs, nothing is written. The same row also
//! shortens the warp after loading a save: an in-game Load runs the level warp below (`pre_field`), the title's Load Game its
//! front-end twin (fast_load.rs, `act` with the front end's words).
//!
//! The ship trip to another planet [C] decomp/notes/level_transition.md section 6 ("Travel to another planet, full order", "Level
//! loader", "Warp end"): the takeoff (mode 6 substate 3) plays on the departed planet; then FUN_002a6228 plays the first-visit
//! title cards and story movies, sets the warp's hold on the level loader (`HOLD`), the target level and the loader state 0, and
//! starts the arrival warp (substate 4, the flight among the stars that hid the disc load on the PS2): per field one warp tick
//! (FUN_002a2ce0) and one loader step (FUN_00257938) until the warp sets `DONE`. The loader reads three files, then waits in state
//! 3 while `HOLD` is set; the warp runs random segments while the loader reads, then one closing segment, and ends with `HOLD`
//! cleared. Then the new level loads and its landing plays (substate 8). An in-game Load (pause Options -> Load) ends the level
//! loop with the loaded save's level as the target: the same FUN_002a6228 branch and warp, without the takeoff.
//!
//! On: while the warp holds the loader the fade word stays full black (1.25: the tick takes 0.25 off before it draws), and once
//! the loader waits in state 3 (the reads are done) the segment count is set to 2 and the segment end to 0, so retail's next
//! tick ends the warp itself (frees its actors, clears `HOLD`, sets `DONE`): the loader goes on at once (sound stop, sound bank),
//! the level loads, the landing follows. The takeoff, the cards and the movies, the save, the music and the landing stay
//! retail's. After the warp the fade word is put back to 0.0, the value retail's warp leaves.
//! The warp sound (retail starts it on the warp's first tick, the loader stops it when the hold goes): the fields this file holds
//! black start no voice (`snd::hush_at_least`, host side: the driver and RAM run as retail, only the mixer never keys the voice on).
//!
//! Replays and test runs: a deterministic run (console_clock.rs `DETERMINISTIC_RUNS`: replays, recordings, scripted pads, held
//! fields, the test clock) or a replay feed gets Off unless its options file names the key, so the gate, fullcheck and picture
//! hashes stay retail; the row in the menus writes the key, so a player's choice always counts.
//!
//! RAM: boot words (both discs alike) and gp words of the overlay data through `va`; the warp's tick and segment end are words of
//! the DEPARTED level's overlay (still in memory during the whole warp: warp.rs), Novalis addresses + `LEVEL_SHIFT`, mapped with
//! `va_level` of that level. The overlay in memory is followed by its code (warp.rs `track_overlay`), not by the level byte
//! alone: after an in-game Load the byte names the loaded level while the old overlay still runs. Nothing is written unless
//! every word reads as a running warp: End also needs the tick one further than in the field before (one warp tick per field).
use super::pcopts::PcOptions;
use crate::retail::disc::{va, va_level};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Options file key; values `on` / `off`.
const KEY: &str = "fasttravel";
/// The setting (default On).
static ON: AtomicBool = AtomicBool::new(true);
/// The options file names the key (or the menu set the row): the setting counts in a deterministic run too.
static EXPLICIT: AtomicBool = AtomicBool::new(false);
/// Worker: the level whose overlay is in memory (u32::MAX: not known yet) and the warp.rs `code_sig` it was taken at.
static OV: AtomicU32 = AtomicU32::new(u32::MAX);
static OV_SIG: AtomicU64 = AtomicU64::new(0);
/// Worker: the level warp's state (trips, in-game Load).
static LEVEL_WARP: Ctx = Ctx::new("fast travel");

/// Game mode (6 = ship travel) and its substate (4 = arrival warp).
const MODE: u32 = 0x15f5a4;
const SUB: u32 = 0x13e050;
const TRAVEL: u32 = 6;
const WARP: u32 = 4;
/// The warp's hold on the level loader (u16) and the loader's state (u16; 3 = reads done, waiting for the hold).
const HOLD: u32 = 0x15ee4a;
const LOADER: u32 = 0x15ee48;
const READS_DONE: u16 = 3;
/// The black fade (f32), the warp's segment count, the warp-over flag, the sound bank handle (-1 while the warp's bank loads).
const FADE: u32 = 0x15f3fc;
const BANK: u32 = 0x15ed5c;
const SEG: u32 = 0x13e08c;
const DONE: u32 = 0x15f5b8;
/// Novalis addresses of the warp tick and the segment end (other levels: + LEVEL_SHIFT of the departed level).
const TICK: u32 = 0x16cc94;
const END: u32 = 0x16cca0;
/// Fade held during the warp: the tick takes 0.25 off and draws a full black quad at 1.0.
const BLACK: f32 = 1.25;
/// Segment count that makes the warp end at its next segment end (retail: > 1 after the closing segment).
const LAST_SEG: u32 = 2;
/// `Ctx::last_tick` / `trace_last` with nothing seen.
const NONE: u32 = u32::MAX;

/// The key's value: (setting, named). Absent or unknown = On, not named.
fn parse(v: Option<&str>) -> (bool, bool) {
	match v.map(str::trim) {
		Some(s) if s.eq_ignore_ascii_case("off") => (false, true),
		Some(s) if s.eq_ignore_ascii_case("on") => (true, true),
		_ => (true, false),
	}
}

/// Pure: does fast travel act in this run? The setting, and in a deterministic run only when the options file names it.
fn wanted(on: bool, explicit: bool, scripted: bool) -> bool {
	on && (explicit || !scripted)
}

pub fn init(opts: &PcOptions) {
	let (on, named) = parse(opts.extra_get(KEY));
	ON.store(on, Ordering::Relaxed);
	EXPLICIT.store(named, Ordering::Relaxed);
}

/// The row is On (the setting; a deterministic run without the key still keeps retail's flight, `active`).
pub fn on() -> bool {
	ON.load(Ordering::Relaxed)
}

/// The classic row's value (pcmore.rs `FT_B`): whether fast travel acts in this run, so a deterministic run without the key shows
/// Off, not the setting (tb5 review N1). A replay feed (RAC_RETAIL_PLAY) is one of console_clock.rs `DETERMINISTIC_RUNS`, so this
/// is `active` of every field. A press on the row writes the key, after which the setting counts in that run too.
pub fn shown() -> bool {
	active(false)
}

/// The modern screen or the classic row set the row: applied from the next trip and remembered; true = the options file changed.
pub fn set_from_ui(opts: &mut PcOptions, on: bool) -> bool {
	EXPLICIT.store(true, Ordering::Relaxed);
	let changed = write_key(opts, ON.swap(on, Ordering::Relaxed), on);
	if changed {
		eprintln!("retail: PC options: Fast travel {}", if on { "On" } else { "Off" });
	}
	changed
}

/// Pure: the row went from `was` to `on`: writes the key when the value changed or the file does not name it yet (so the
/// player's choice counts in every later run); true = the options file changed.
fn write_key(opts: &mut PcOptions, was: bool, on: bool) -> bool {
	if was == on && opts.extra_get(KEY).is_some() {
		return false;
	}
	opts.extra_set(KEY, Some(if on { "on" } else { "off" }.to_string()));
	true
}

/// Pure: the warp counters read as a running warp (a segment is a few hundred ticks; anything else is not a warp word and is
/// never written).
fn plausible(tick: u32, end: u32) -> bool {
	tick < 0x4000 && end < 0x4000
}

/// Pure: End may write the segment words: tick and end read as warp words, the tick is exactly one further than in the last
/// held field (the warp ticks once per field; a word at a wrong address is not a counter in step with it) and the segment count
/// is retail's. Otherwise the field stays black and the next one tries again (a new segment restarts the tick).
fn may_end(now: Option<(u32, u32)>, last: u32, seg: u32) -> bool {
	matches!(now, Some((t, e)) if plausible(t, e) && last != NONE && t == last.wrapping_add(1) && seg <= LAST_SEG)
}

/// Fast travel acts in this run (`replay`: the worker replays a recording).
pub(super) fn active(replay: bool) -> bool {
	wanted(on(), EXPLICIT.load(Ordering::Relaxed), replay || super::console_clock::deterministic())
}

/// What the decision reads at the start of a field.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Seen {
	mode: u32,
	sub: u32,
	hold: bool,
	loader: u16,
}

/// What `act` does in a field.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Act {
	Nothing,
	/// The warp runs and the loader still reads: keep it black.
	Black,
	/// The reads are done: black, and end the warp at its next tick.
	End,
	/// The warp this file held is over: the fade back to retail's end value.
	Release,
}

/// Pure: the field's action.
fn decide(active: bool, held: bool, s: &Seen) -> Act {
	let warp = s.mode == TRAVEL && s.sub == WARP && s.hold;
	match (active && warp, held) {
		(true, _) if s.loader == READS_DONE => Act::End,
		(true, _) => Act::Black,
		(false, true) => Act::Release,
		(false, false) => Act::Nothing,
	}
}

/// Pure: the field's batch starts no voice (the black warp: its sound is the only one that starts there; the release field and
/// everything after it, the landing's sounds, play).
fn silent(what: Act) -> bool {
	matches!(what, Act::Black | Act::End)
}

/// Pure: the fade word is put back on release only in the warp's own field (mode word travel, substate 4: the trip and the title
/// Load release there, the loader steps 4..7 follow in substate 4) and while its words are still the warp's (`own`). tb5 review
/// S2: the new level's landing runs in mode 6 too (substate 8) with its own fade word at the same address: never written.
fn release_writes(mode: u32, sub: u32, own: bool) -> bool {
	mode == TRAVEL && sub == WARP && own
}

/// Pure: the warp's words are still its own on release: the front end (`code` false) is checked by its caller (fast_load.rs runs
/// only while the frontbin is in memory); a level warp's words are its overlay's, so the code signature (warp.rs `code_sig`) now
/// must be the one taken while it was held (`held_sig`, 0: none), else another overlay was read in.
fn own(code: bool, now: u64, held_sig: u64) -> bool {
	!code || (held_sig != 0 && now == held_sig)
}

/// Pure: the arrival substates (the departed overlay stays in memory until the new level loads).
fn arriving(mode: u32, sub: u32) -> bool {
	mode == TRAVEL && (4..=6).contains(&sub)
}

/// The departed overlay's warp words (tick, end), None while it is not known.
fn warp_words(ov: u32) -> Option<(u32, u32)> {
	let d = *super::camera::LEVEL_SHIFT.get(ov as usize)?;
	Some((va_level(ov as usize, TICK.wrapping_add_signed(d)), va_level(ov as usize, END.wrapping_add_signed(d))))
}

/// The words of one warp on the disc that runs (mapped by the caller): the mode word of the code that runs it (6 = travel), the
/// black fade, the warp-over flag, and the warp tick + segment end (None while not known: then the warp is only held black).
/// `code`: the words are a level overlay's (Release checks that overlay is still in memory, `own`).
#[derive(Clone, Copy, Debug)]
pub(super) struct Warp {
	pub(super) mode: u32,
	pub(super) fade: u32,
	pub(super) done: u32,
	pub(super) words: Option<(u32, u32)>,
	pub(super) code: bool,
}

/// One warp context's state (worker thread): the level warp here, the title's Load Game warp in fast_load.rs.
pub(super) struct Ctx {
	/// Name in the log lines.
	name: &'static str,
	/// The warp is being held black by this file (the fade goes back when it ends).
	held: AtomicBool,
	/// The warp tick of the last held field (`NONE`: none), for `may_end`.
	last_tick: AtomicU32,
	/// End wrote the segment words in the warp held now (log only).
	ended: AtomicBool,
	/// RAC_SCENE_LOG: the last state `trace` printed.
	trace_last: AtomicU32,
	/// warp.rs `code_sig` in the last held field (a level warp, `Warp::code`; 0: none).
	sig: AtomicU64,
}

impl Ctx {
	pub(super) const fn new(name: &'static str) -> Ctx {
		Ctx { name, held: AtomicBool::new(false), last_tick: AtomicU32::new(NONE), ended: AtomicBool::new(false), trace_last: AtomicU32::new(NONE), sig: AtomicU64::new(0) }
	}

	/// Forget the held warp (Release, or its code left memory before the release field: fast_load.rs `level_field`). Returns
	/// whether one was held.
	pub(super) fn clear(&self) -> bool {
		self.last_tick.store(NONE, Ordering::Relaxed);
		self.sig.store(0, Ordering::Relaxed);
		self.ended.store(false, Ordering::Relaxed);
		self.held.swap(false, Ordering::Relaxed)
	}

	#[cfg(test)]
	pub(super) fn hold_for_test(&self) {
		self.held.store(true, Ordering::Relaxed);
	}
}

fn scene_log() -> bool {
	static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*LOG.get_or_init(|| std::env::var_os("RAC_SCENE_LOG").is_some())
}

/// RAC_SCENE_LOG: the trip's steps as they change (substate, warp hold, loader state while the mode word says travel; the mode
/// when it leaves travel), Off and On alike; reads only. `ov`: the overlay in memory (the level warp), u32::MAX: not shown.
pub(super) fn trace(m: &ee_oracle::Machine, field: u32, w: &Warp, c: &Ctx, ov: u32) {
	if !scene_log() {
		return;
	}
	let (mode, sub) = (m.rd32(w.mode), m.rd32(va(SUB)));
	let (hold, loader) = (m.rd16(va(HOLD)), m.rd16(va(LOADER)));
	let key = if mode == TRAVEL { sub << 24 | (hold.min(1) as u32) << 16 | loader as u32 } else { NONE - 1 };
	let prev = c.trace_last.swap(key, Ordering::Relaxed);
	if prev == key {
		return;
	}
	if mode == TRAVEL {
		eprintln!("retail: {} trace f{field} sub {sub} hold {hold} loader {loader} level {} ov {}", c.name, m.level(), ov as i32);
	} else if prev != NONE {
		eprintln!("retail: {} trace f{field} mode {mode} (travel over) level {}", c.name, m.level());
	}
}

/// One field of a warp context (`pre_field` here, fast_load.rs `front_field`): holds the warp black while the loader reads,
/// ends it once the reads are done, and puts the fade back after it. `active`: the row acts in this run. Off and idle: no read.
///
/// The warp sound: retail starts it on the warp's first tick (tick counter 1, `func_0x0012e308`) and the loader stops every sound
/// when the hold is released (RPC 0x18), a few fields later with the warp ended early: a blip. The fields this file holds black
/// start no voice: only the warp's own sound starts there.
pub(super) fn act(m: &mut ee_oracle::Machine, field: u32, active: bool, w: &Warp, c: &Ctx) {
	let held = c.held.load(Ordering::Relaxed);
	if !active && !held {
		return;
	}
	let s = Seen { mode: m.rd32(w.mode), sub: m.rd32(va(SUB)), hold: m.rd16(va(HOLD)) != 0, loader: m.rd16(va(LOADER)) };
	let what = decide(active, held, &s);
	if what == Act::Nothing {
		return;
	}
	let now = w.words.map(|(t, e)| (m.rd32(t), m.rd32(e)));
	let (seg, last) = (m.rd32(va(SEG)), c.last_tick.load(Ordering::Relaxed));
	if scene_log() {
		let (t, e) = now.map_or((-1, -1), |(t, e)| (t as i32, e as i32));
		eprintln!(
			"retail: {} f{field} {what:?} sub {} tick {t} (last {}) end {e} seg {seg} loader {} hold {} done {} fade {} bank {}",
			c.name,
			s.sub,
			last as i32,
			s.loader,
			s.hold as u8,
			m.rd32(w.done),
			f32::from_bits(m.rd32(w.fade)),
			m.rd32(va(BANK)) as i32
		);
	}
	if silent(what) {
		super::snd::hush_at_least(1);
	}
	match what {
		Act::Nothing => {}
		Act::Black | Act::End => {
			m.cpu.mem.wr32(w.fade, BLACK.to_bits());
			c.held.store(true, Ordering::Relaxed);
			if w.code {
				c.sig.store(super::warp::code_sig(m), Ordering::Relaxed);
			}
			c.last_tick.store(now.map_or(NONE, |(t, _)| t), Ordering::Relaxed);
			if what == Act::End {
				match now.zip(w.words) {
					Some((_, (_, end))) if may_end(now, last, seg) => {
						let mem = &mut m.cpu.mem;
						mem.wr32(va(SEG), LAST_SEG);
						mem.wr32(end, 0);
						c.ended.store(true, Ordering::Relaxed);
					}
					_ if scene_log() => eprintln!("retail: {}: f{field} warp words not in step: not ended this field", c.name),
					_ => {}
				}
			}
		}
		Act::Release => {
			let mine = own(w.code, if w.code { super::warp::code_sig(m) } else { 0 }, c.sig.load(Ordering::Relaxed));
			let write = release_writes(s.mode, s.sub, mine);
			if write {
				m.cpu.mem.wr32(w.fade, 0f32.to_bits());
			}
			let how = if c.ended.load(Ordering::Relaxed) { "warp skipped" } else { "warp held black for its own length (words not as expected)" };
			c.clear();
			let fade = if write { "" } else { "; fade left: not the warp's field or code any more" };
			eprintln!("retail: {}: {how} (field {field}, level {}{fade})", c.name, m.level());
		}
	}
}

/// The level whose overlay is in memory, for this field (warp.rs `track_overlay`; kept during the arrival).
fn level_overlay(m: &ee_oracle::Machine, arriving: bool) -> u32 {
	let ov = OV.load(Ordering::Relaxed);
	if arriving {
		return ov;
	}
	let known = (ov != u32::MAX).then(|| (ov, OV_SIG.load(Ordering::Relaxed)));
	let (ov, sig) = super::warp::track_overlay(known, m.level(), super::warp::code_sig(m));
	OV.store(ov, Ordering::Relaxed);
	OV_SIG.store(sig, Ordering::Relaxed);
	ov
}

/// Worker, before a level field (`worker/field_run.rs`; `replay`: a recording feeds the pad). Off: reads only (the overlay and
/// the RAC_SCENE_LOG trace).
pub fn pre_field(m: &mut ee_oracle::Machine, field: u32, replay: bool) {
	super::fast_load::level_field();
	let (mode, sub) = (m.rd32(va(MODE)), m.rd32(va(SUB)));
	let ov = level_overlay(m, arriving(mode, sub));
	let w = Warp { mode: va(MODE), fade: va(FADE), done: va(DONE), words: warp_words(ov), code: true };
	trace(m, field, &w, &LEVEL_WARP, ov);
	act(m, field, active(replay), &w, &LEVEL_WARP);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn key_default_on_explicit_off() {
		assert_eq!(parse(None), (true, false), "no key: On, not named");
		assert_eq!(parse(Some("off")), (false, true));
		assert_eq!(parse(Some("OFF ")), (false, true));
		assert_eq!(parse(Some("on")), (true, true));
		assert_eq!(parse(Some("maybe")), (true, false), "an unknown value counts as no key");
	}

	/// A replay / test run without the key keeps retail's flight (gate, fullcheck, pp hashes); a named key counts everywhere.
	#[test]
	fn replay_rule() {
		assert!(wanted(true, false, false), "a player without the key: On");
		assert!(!wanted(true, false, true), "a replay or test run without the key: Off");
		assert!(wanted(true, true, true), "a replay with fasttravel=on: On");
		assert!(!wanted(false, true, false) && !wanted(false, true, true), "fasttravel=off: Off everywhere");
		// tb5 review N1: the classic row shows what acts (Off above in a replay / test run without the key), not the setting.
		let _g = crate::retail::pcopts::globals_guard();
		assert_eq!(shown(), wanted(on(), EXPLICIT.load(Ordering::Relaxed), super::super::console_clock::deterministic()));
	}

	/// The row writes the key (statics left alone: other tests init them in parallel).
	#[test]
	fn the_row_writes_the_key() {
		let mut o = PcOptions::new();
		assert!(write_key(&mut o, true, false));
		assert_eq!(o.extra_get(KEY), Some("off"));
		assert_eq!(parse(o.extra_get(KEY)), (false, true));
		assert!(!write_key(&mut o, false, false), "no change: no save");
		assert!(write_key(&mut o, false, true));
		assert_eq!(o.extra_get(KEY), Some("on"));
		// The default chosen in the menu is still written, so a later replay or test run with that options file gets it.
		let mut fresh = PcOptions::new();
		assert!(write_key(&mut fresh, true, true) && fresh.extra_get(KEY) == Some("on"));
		assert!(plausible(0, 300) && plausible(17, 0) && !plausible(0x1234_5678, 300) && !plausible(3, 0xffff_ffff));
	}

	#[test]
	fn decide_holds_black_then_ends_then_releases() {
		let warp = |loader| Seen { mode: TRAVEL, sub: WARP, hold: true, loader };
		assert_eq!(decide(true, false, &warp(0)), Act::Black, "reads running: black only");
		assert_eq!(decide(true, true, &warp(2)), Act::Black);
		assert_eq!(decide(true, true, &warp(READS_DONE)), Act::End, "reads done: end the warp");
		let after = Seen { mode: TRAVEL, sub: WARP, hold: false, loader: 4 };
		assert_eq!(decide(true, true, &after), Act::Release, "hold cleared by retail's tick: fade back");
		assert_eq!(decide(true, false, &after), Act::Nothing);
		assert_eq!(decide(false, false, &warp(READS_DONE)), Act::Nothing, "Off: nothing written");
		assert_eq!(decide(true, false, &Seen { mode: TRAVEL, sub: 3, hold: true, loader: 0 }), Act::Nothing, "the takeoff stays");
		assert_eq!(decide(true, false, &Seen { mode: 0, sub: WARP, hold: true, loader: 3 }), Act::Nothing, "gameplay: nothing");
		assert_eq!(decide(true, true, &Seen { mode: 0, sub: WARP, hold: true, loader: 3 }), Act::Release, "mode left travel: release");
		assert!(release_writes(TRAVEL, WARP, true) && !release_writes(0, WARP, true), "the fade word is the warp's only in travel mode");
		assert!(arriving(TRAVEL, 4) && arriving(TRAVEL, 6) && !arriving(TRAVEL, 3) && !arriving(0, 4));
		assert!(silent(Act::Black) && silent(Act::End) && !silent(Act::Release) && !silent(Act::Nothing), "only the black warp is hushed");
	}

	/// tb5 review S2: Release puts the fade back only in the warp's own field (substate 4, the trip runs: f417 / f489, the title Load
	/// f734) with the code it held still in memory; the new level's landing (substate 8, its own overlay at the same fade address)
	/// and a field where another overlay was read in are left alone.
	#[test]
	fn release_writes_only_the_warps_own_fade() {
		let (departed, landed) = (0x1111u64, 0x2223u64);
		assert!(release_writes(TRAVEL, WARP, own(true, departed, departed)), "the trip's release field");
		assert!(!release_writes(TRAVEL, 8, own(true, landed, departed)), "the landing: not the warp's field");
		assert!(!release_writes(TRAVEL, 8, own(true, departed, departed)), "the landing, even with the same code");
		assert!(!release_writes(TRAVEL, WARP, own(true, landed, departed)), "the target's overlay already read in");
		assert!(!release_writes(TRAVEL, WARP, own(true, departed, 0)), "no code taken while held: no write");
		assert!(release_writes(TRAVEL, WARP, own(false, 0, 0)), "the front end: its field runs only with the frontbin");
		assert!(!release_writes(0, WARP, own(false, 0, 0)) && !release_writes(TRAVEL, 5, own(false, 0, 0)));
		let c = Ctx::new("test");
		c.held.store(true, Ordering::Relaxed);
		c.last_tick.store(4, Ordering::Relaxed);
		c.sig.store(departed, Ordering::Relaxed);
		c.ended.store(true, Ordering::Relaxed);
		assert!(c.clear(), "was held");
		assert!(!c.held.load(Ordering::Relaxed) && c.last_tick.load(Ordering::Relaxed) == NONE && c.sig.load(Ordering::Relaxed) == 0 && !c.ended.load(Ordering::Relaxed));
		assert!(!c.clear(), "nothing held");
	}

	/// End writes only when the tick is one further than in the last held field (the trip run: f415 tick 3, f416 tick 4).
	#[test]
	fn end_needs_the_tick_in_step() {
		assert!(may_end(Some((4, 218)), 3, 0), "in step: end");
		assert!(may_end(Some((4, 218)), 3, LAST_SEG));
		assert!(!may_end(Some((4, 218)), NONE, 0), "first held field: wait one");
		assert!(!may_end(Some((1, 300)), 218, 1), "a new segment restarted the tick: wait one");
		assert!(!may_end(Some((5, 218)), 3, 0), "two ticks in one field: not the warp's counter");
		assert!(!may_end(Some((4, 218)), 3, 7), "segment count not retail's");
		assert!(!may_end(Some((0x10000, 5)), 0xffff, 0), "not a warp word");
		assert!(!may_end(None, 3, 0), "words not known: black only");
	}

	#[test]
	fn warp_words_follow_the_departed_overlay() {
		assert_eq!(warp_words(1), Some((TICK, END)), "Novalis");
		assert_eq!(warp_words(4), Some((TICK - 0x300, END - 0x300)), "Eudora (-0x300, warp.rs log)");
		assert_eq!(warp_words(u32::MAX), None, "unknown overlay: no write");
	}

	/// In-game Load on Kerwan (3) of a save on Eudora (4): the level byte says 4 from the card read on, but Kerwan's code runs
	/// until the arrival, so the warp words are Kerwan's; the End write can never land in Eudora's offsets of Kerwan's overlay.
	#[test]
	fn a_load_to_another_planet_writes_the_departed_overlay() {
		let (kerwan, eudora) = (0x1111u64, 0x2222u64);
		let ov = super::super::warp::track_overlay(None, 3, kerwan);
		let ov = super::super::warp::track_overlay(Some(ov), 4, kerwan);
		assert_eq!(ov.0, 3, "pause menu after the card read: still Kerwan's overlay");
		assert_eq!(warp_words(ov.0), warp_words(3));
		assert_ne!(warp_words(3), warp_words(4), "Kerwan's and Eudora's warp words differ (the old tracker used Eudora's)");
		assert_eq!(super::super::warp::track_overlay(Some(ov), 4, eudora).0, 4, "the landing runs Eudora's code");
	}
}
