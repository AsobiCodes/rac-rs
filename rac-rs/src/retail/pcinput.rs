//! Three input options (v0.4.2, lane menu; GitHub #27, #13, #30). All default to retail / today's behaviour; each is a row of the
//! game's own Options menu and a key of `pc_options.txt` (`pcopts.rs` `extra_get` / `extra_set`, a default value removes its key):
//!
//! - Strafe aiming: Raised / Shoulder / Off (Camera Options page, the Strafe aiming row; `pcext.rs` owns the row and its byte `STRAFE`:
//!   0 Raised, 1 Shoulder, 2 Off). This file words the two new values (`ID_RAISED`, `ID_SHOULDER`) and maps the row to the file's
//!   keys: `strafe=on|off` (the launcher's key, unchanged: Off = `off`, Raised and Shoulder = `on`) plus `aimcam=shoulder` for
//!   Shoulder (Raised has no key; an `aimcam=level` of the v0.4.2 test builds reads as Raised). Raised is today's strafe aim of the
//!   Modern camera (`modcam.rs`: the orbit pivot lifts `AIM_LIFT`, Ratchet sits low in the frame); Shoulder is the over-the-shoulder
//!   aim camera of `modcam/shoulder.rs`.
//! - Quick Select mouse (Graphics -> System; key `wheelmouse=on`): the mouse direction picks the weapon wheel's sector while the
//!   wheel is open (what `RAC_WHEEL_MOUSE=1` switches on in `wheel_mouse.rs`; that file asks `wheel_mouse()`).
//! - Stick dead zone: High / Medium / Low (Graphics -> System; key `stickdz=small|minimal`). High is retail's rule untouched. The
//!   dead zone a player feels is the game's own (each axis ignores 47 of 127 counts, `prompts::stick`), which sits above the host's
//!   noise gate, so a smaller gate alone changes nothing; Medium and Low therefore also lift a push that is past the host's smaller
//!   gate to just above the game's (`left_stick`): Ratchet moves at once, slowly, and the push then grows into the retail curve
//!   (identical from 55 % of the throw on). The left stick only: the right stick (camera) keeps its gate.
//!
//! The other two rows are bytes of the Graphics page's block (`pcctl`). Every byte is 0 for the defaults, so a default RAM stays what it
//! was. The labels and values are the text ids 0x7fba..0x7fbd in `pcdisp`'s pool (`pcdisp::text` hands them here; 0x7fb9 is free).
//! Stick dead zone reuses retail's High / Medium / Low words as its values.
use super::pcctl::CBASE;
use super::pcopts::PcOptions;
use ee_oracle::mem::Mem;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// The Strafe aiming row's bytes (`pcext::STRAFE`).
pub const STRAFE_RAISED: u8 = 0;
pub const STRAFE_SHOULDER: u8 = 1;
pub const STRAFE_OFF: u8 = 2;

/// The Quick Select mouse row's byte (0 Off, 1 On) and the Stick dead zone row's (0 High = retail, 1 Medium, 2 Low); pcctl's block.
pub(super) const WHEEL_B: u32 = CBASE + 0x1b;
pub(super) const STICK_B: u32 = CBASE + 0x1c;
/// Text ids (the Strafe aiming values, then the labels); pcdisp's pool has room for them (ids 0x7fb9..0x7fbf).
pub(super) const ID_RAISED: u32 = 0x7fba;
pub(super) const ID_SHOULDER: u32 = 0x7fbb;
pub(super) const ID_WHEEL: u32 = 0x7fbc;
pub(super) const ID_STICK: u32 = 0x7fbd;
/// The ids this file words.
pub(super) const IDS: [u32; 4] = [ID_RAISED, ID_SHOULDER, ID_WHEEL, ID_STICK];

static WHEEL: AtomicBool = AtomicBool::new(false);
static STICK: AtomicU8 = AtomicU8::new(0);

/// The dead-zone value names the file uses, in row order.
const STICK_NAMES: [&str; 3] = ["retail", "small", "minimal"];

/// `pcext::init`: the file's choices are the rows' start values.
pub fn init(opts: &PcOptions) {
	super::modcam::shoulder::set(strafe_row(opts) == STRAFE_SHOULDER);
	WHEEL.store(opts.extra_get("wheelmouse").is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "on" | "1" | "true")), Ordering::Relaxed);
	let s = opts.extra_get("stickdz").map_or(0, |v| STICK_NAMES.iter().position(|n| v.trim().eq_ignore_ascii_case(n)).unwrap_or(0));
	STICK.store(s as u8, Ordering::Relaxed);
}

/// The Strafe aiming row's start byte from the file: Off when `strafe=off`, else Shoulder with `aimcam=shoulder`, else Raised (any
/// other `aimcam`, such as the test builds' `level`, is Raised).
pub fn strafe_row(opts: &PcOptions) -> u8 {
	if !opts.strafe_on() {
		STRAFE_OFF
	} else if opts.extra_get("aimcam").is_some_and(|v| v.trim().eq_ignore_ascii_case("shoulder")) {
		STRAFE_SHOULDER
	} else {
		STRAFE_RAISED
	}
}

/// The row changed to `row`: the file's keys follow. Off keeps an `aimcam` the file has (the launcher can switch strafe back on and
/// get Shoulder again); Raised removes it.
pub fn set_strafe_row(opts: &mut PcOptions, row: u8) {
	opts.strafe = Some(row != STRAFE_OFF);
	match row {
		STRAFE_SHOULDER => opts.extra_set("aimcam", Some("shoulder".to_string())),
		STRAFE_RAISED => opts.extra_set("aimcam", None),
		_ => {}
	}
	super::modcam::shoulder::set(row == STRAFE_SHOULDER);
}

/// The Quick Select mouse row's start byte and the Stick dead zone row's (`pcctl::build_gfx` on fresh RAM).
pub(super) fn wheel_byte() -> u8 {
	WHEEL.load(Ordering::Relaxed) as u8
}
pub(super) fn stick_byte() -> u8 {
	STICK.load(Ordering::Relaxed)
}

/// Whether the Quick Select mouse row is On. `wheel_mouse.rs` `enabled()` is true when this is (or when `RAC_WHEEL_MOUSE=1`).
pub fn wheel_mouse() -> bool {
	WHEEL.load(Ordering::Relaxed)
}

// ---- the left stick -----------------------------------------------------------------------------------------------------

/// PCSX2's analog sensitivity (`prompts::AXIS_SCALE`) times the byte range: a push of fraction m gives 1.33 * 127 * m counts.
const COUNTS_PER_UNIT: f32 = 1.33 * 127.0;
/// The first count past the game's own per-axis dead zone (47 of 127; the game reads nothing at or below it): 50.
const LO: f32 = 50.0 / COUNTS_PER_UNIT;
/// From this much throw (largest axis) on, the push is retail's curve unchanged.
const FULL_FROM: f32 = 0.55;
/// The host's noise gate per mode (fraction of the full throw): Medium and Low.
const GATE: [f32; 3] = [0.0, 0.12, 0.06];

/// `prompts::map_xinput`: the left stick of an XInput report `(x, y)` and the radial gate to give `prompts::stick` (retail:
/// `retail_gate`, the vector untouched). Medium / Low: the gate is the mode's smaller one and a push beyond it is lifted so the
/// game's own dead zone does not swallow it (`lift`); an inside push is centred (gate 0 with a zero vector).
pub fn left_stick(x: i16, y: i16, retail_gate: f32) -> ((i16, i16), f32) {
	left_stick_mode(x, y, retail_gate, STICK.load(Ordering::Relaxed))
}

fn left_stick_mode(x: i16, y: i16, retail_gate: f32, mode: u8) -> ((i16, i16), f32) {
	let g = GATE[mode.min(2) as usize];
	if g == 0.0 {
		return ((x, y), retail_gate);
	}
	let (fx, fy) = (x as f32 / 32767.0, y as f32 / 32767.0);
	if (fx * fx + fy * fy).sqrt() <= g {
		return ((0, 0), 0.0);
	}
	let m = fx.abs().max(fy.abs());
	let k = lift(m, g) / m;
	let s = |v: f32| (v * k * 32767.0).round().clamp(-32767.0, 32767.0) as i16;
	((s(fx), s(fy)), 0.0)
}

/// The largest axis (fraction of the throw) `m` of a push past the gate `g`, as the game should see it: from `LO` (just past its own
/// dead zone) at the gate, rising linearly to retail's value at `FULL_FROM`, unchanged beyond.
fn lift(m: f32, g: f32) -> f32 {
	if m >= FULL_FROM {
		m
	} else {
		LO + (FULL_FROM - LO) * ((m - g).max(0.0) / (FULL_FROM - g))
	}
}

// ---- rows ---------------------------------------------------------------------------------------------------------------

/// Worker, every field after the page patch (`pcext::sync`): a changed row goes into the options file (true = the caller saves).
/// Quick Select mouse and Stick dead zone are read once the Graphics page's block exists (the Strafe aiming row is `pcext::apply`'s).
pub fn sync(mem: &mut Mem, opts: &mut PcOptions) -> bool {
	let mut changed = false;
	if !super::pcctl::gfx_built(mem) {
		return changed;
	}
	let wheel = mem.rd8(WHEEL_B) == 1;
	if WHEEL.swap(wheel, Ordering::Relaxed) != wheel {
		eprintln!("retail: PC options: Quick Select mouse {}", if wheel { "On" } else { "Off" });
		opts.extra_set("wheelmouse", wheel.then(|| "on".to_string()));
		changed = true;
	}
	// A three-valued row: the pause page's item cycles the byte, the title's value item only flips it, so any change is one press and
	// the row steps on from its last value on both pages (like the Vegetation row, `pcext::sync`).
	let (b, last) = (mem.rd8(STICK_B), STICK.load(Ordering::Relaxed));
	if b != last {
		let next = (last + 1) % 3;
		mem.wr8(STICK_B, next);
		STICK.store(next, Ordering::Relaxed);
		eprintln!("retail: PC options: stick dead zone {}", STICK_NAMES[next as usize]);
		opts.extra_set("stickdz", (next != 0).then(|| STICK_NAMES[next as usize].to_string()));
		changed = true;
	}
	changed
}

// ---- texts --------------------------------------------------------------------------------------------------------------

type T5 = [&'static [u8]; 5];

#[rustfmt::skip]
static RAISED: T5 = [b"Raised", b"Relev\x8ce", b"Angehoben", b"Elevada", b"Alzata"];
#[rustfmt::skip]
static SHOULDER: T5 = [b"Shoulder", b"Epaule", b"Schulter", b"Hombro", b"Spalla"];
#[rustfmt::skip]
static WHEEL_LABEL: T5 = [b"Quick Select mouse", b"Roue par souris", b"Schnellwahl Maus", b"Rueda con rat\x9cn", b"Ruota con mouse"];
#[rustfmt::skip]
static STICK_LABEL: T5 = [b"Stick dead zone", b"Zone morte stick", b"Stick-Totzone", b"Zona muerta stick", b"Zona morta stick"];

/// The text of `id` (`IDS`) in language `l` (0 En .. 4 It); empty for any other id.
pub fn text(id: u32, l: usize) -> Vec<u8> {
	let l = l.min(4);
	let t = match id {
		ID_RAISED => &RAISED,
		ID_SHOULDER => &SHOULDER,
		ID_WHEEL => &WHEEL_LABEL,
		ID_STICK => &STICK_LABEL,
		_ => return Vec::new(),
	};
	t[l].to_vec()
}

#[cfg(test)]
mod tests {
	use super::*;
	use super::super::modcam::shoulder;
	use super::super::pcext::BASE;

	/// Retail's rule untouched, and the other two start from the file.
	#[test]
	fn defaults_are_retail() {
		let _g = crate::retail::pcopts::globals_guard();
		init(&PcOptions::parse(""));
		assert_eq!((strafe_row(&PcOptions::parse("")), wheel_mouse(), stick_byte(), shoulder::on()), (STRAFE_RAISED, false, 0, false));
		assert_eq!(left_stick(1234, -20000, 7849.0), ((1234, -20000), 7849.0), "retail: the vector and the gate untouched");
		init(&PcOptions::parse("aimcam=shoulder\nwheelmouse=on\nstickdz=minimal\n"));
		assert_eq!((wheel_mouse(), stick_byte(), shoulder::on()), (true, 2, true));
		init(&PcOptions::parse(""));
	}

	/// A lifted push clears the game's own dead zone (47 counts per axis), is continuous at FULL_FROM and never lowers a push.
	#[test]
	fn a_small_push_moves_ratchet_and_a_big_one_is_retail() {
		for mode in [1u8, 2] {
			let g = GATE[mode as usize];
			let counts = |m: f32| -> f32 {
				let ((x, _), _) = left_stick_mode((m * 32767.0) as i16, 0, 7849.0, mode);
				x as f32 / 32767.0 * COUNTS_PER_UNIT
			};
			assert_eq!(left_stick_mode((g * 32767.0 * 0.9) as i16, 0, 7849.0, mode), ((0, 0), 0.0), "inside the gate: centred");
			assert!(counts(g + 0.01) > 47.0, "mode {mode}: just past the gate the game sees a push ({})", counts(g + 0.01));
			assert!(counts(0.30) > 0.30 * COUNTS_PER_UNIT, "mode {mode}: never weaker than retail's curve");
			let mut prev = 0.0;
			for i in 1..=100 {
				let c = counts(i as f32 / 100.0 * 0.999).max(0.0);
				assert!(c + 0.6 >= prev, "mode {mode}: monotone at {i}");
				prev = c;
			}
			assert_eq!(left_stick_mode(20000, 0, 7849.0, mode), ((20000, 0), 0.0), "from 55 % on the retail curve");
		}
	}

	/// The game reads each axis alone: a diagonal push is lifted by its larger axis, so that axis passes the game's dead zone.
	#[test]
	fn a_diagonal_is_lifted_by_its_larger_axis() {
		let ((x, y), _) = left_stick_mode(5000, 4800, 7849.0, 2);
		assert!(x as f32 / 32767.0 * COUNTS_PER_UNIT > 47.0 && y > 0 && y < x, "{x} {y}");
	}

	/// The three rows' bytes, the file keys and the log: a change is saved once; a three-valued press steps on from the last value.
	#[test]
	fn rows_save_to_the_options_file() {
		let _g = crate::retail::pcopts::globals_guard();
		init(&PcOptions::parse(""));
		let mut mem = Mem::new();
		let mut o = PcOptions::default();
		// The Graphics block exists: the other two rows.
		let rows: Vec<[u32; 6]> = (0..15u32).map(|i| [0x100 + i, 0x200 + i, 0x300, 0x301, 0, 0]).collect();
		super::super::pcctl::build_gfx(&mut mem, BASE + 0xaa0, &rows, true, false);
		assert_eq!((mem.rd8(WHEEL_B), mem.rd8(STICK_B)), (0, 0), "default RAM stays zero");
		mem.wr8(WHEEL_B, 1);
		assert!(sync(&mut mem, &mut o) && o.extra_get("wheelmouse") == Some("on") && wheel_mouse());
		mem.wr8(STICK_B, 1);
		assert!(sync(&mut mem, &mut o) && o.extra_get("stickdz") == Some("small"));
		mem.wr8(STICK_B, 0); // the title's item flipped 1 -> 0: that is the press after Medium, so Low
		assert!(sync(&mut mem, &mut o) && o.extra_get("stickdz") == Some("minimal") && mem.rd8(STICK_B) == 2);
		mem.wr8(STICK_B, 0); // Low -> High
		assert!(sync(&mut mem, &mut o) && o.extra_get("stickdz").is_none() && mem.rd8(STICK_B) == 0);
		mem.wr8(WHEEL_B, 0);
		assert!(sync(&mut mem, &mut o) && o.extra_get("wheelmouse").is_none());
		init(&PcOptions::parse(""));
	}

	#[test]
	fn the_ids_are_worded_in_every_language() {
		for id in IDS {
			for l in 0..5 {
				let s = text(id, l);
				assert!(!s.is_empty() && s.len() < 0x40, "id {id:#x} lang {l}");
				assert!(s.len() <= 22, "id {id:#x} lang {l} fits a cell");
			}
		}
		assert_eq!((text(ID_RAISED, 0), text(ID_SHOULDER, 0)), (b"Raised".to_vec(), b"Shoulder".to_vec()));
		assert!(text(0x7fbe, 0).is_empty());
		assert!(IDS.iter().all(|&i| (super::super::pcdisp::ID0..super::super::pcdisp::ID_END).contains(&i)));
	}

	/// The Strafe aiming row and the file: the launcher's `strafe=on|off` stays, Shoulder adds `aimcam=shoulder`, Raised removes it,
	/// Off keeps it (a launcher switch back to on gives Shoulder again) and an old `aimcam=level` is Raised.
	#[test]
	fn strafe_row_round_trips_with_the_launcher_keys() {
		let _g = crate::retail::pcopts::globals_guard();
		let row = |t: &str| strafe_row(&PcOptions::parse(t));
		assert_eq!([row(""), row("strafe=on"), row("strafe=off"), row("aimcam=level"), row("strafe=on\naimcam=shoulder"), row("strafe=off\naimcam=shoulder")], [0, 0, 2, 0, 1, 2]);
		let mut o = PcOptions::parse("version=1\nzzz=1\n");
		set_strafe_row(&mut o, STRAFE_SHOULDER);
		assert_eq!(o.to_text(), "version=1\nstrafe=on\nzzz=1\naimcam=shoulder\n");
		assert!(shoulder::on());
		set_strafe_row(&mut o, STRAFE_OFF);
		assert!(o.to_text().contains("strafe=off") && o.to_text().contains("aimcam=shoulder") && !shoulder::on());
		assert_eq!(strafe_row(&PcOptions::parse(&o.to_text())), STRAFE_OFF);
		set_strafe_row(&mut o, STRAFE_RAISED);
		assert_eq!(o.to_text(), "version=1\nstrafe=on\nzzz=1\n");
		init(&PcOptions::parse(""));
	}
}
