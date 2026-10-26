//! Camera Options rows of `pcext.rs`: Lock-on and Camera speed (moved out of pcext.rs 2026-10-23, lane cam: a pure move;
//! `use super::*` reaches the same private items). Camera speed scales the controller's stick turn only; the Mouse speed row right
//! below it is `mousespeed.rs` (`camopts_field` / `camopt_text` / `camopts_init` call into it).
use super::*;

// ---- Camera Options: Lock-on and Camera speed rows ---------------------------------------------------------------------------
//
// Two rows of the Camera Options page (see `patch_rows_at`; the page then has 8 rows, Mouse speed is row 3, mousespeed.rs):
//
// - Camera speed (row 2, replaces retail's Slow / Medium / Fast row): a value row in percent, 10..200 in steps of 10, shown as
//   `< 50 >` in retail's font (no percent sign: the font's code 0x25 is the Cross button glyph). Retail has no slider item the page could reuse (the only bars are the Sound page's own item,
//   per-level code), so it is a multi-choice row whose two choices are the same dynamic text (`ID_PCT`): retail's update toggles
//   the row's byte `CAMSEL` on Cross, which the host reads as "+10" (200 wraps to 10); Left / Right (pad bits of the field, also
//   the mouse wheel) are read by the host while this row is the page's cursor row: -10 / +10 with the menu's auto-repeat.
//   50 % is retail's Fast exactly. The percent is the player's own key `camspeed` (pcopts.rs); without the key nothing is
//   written: retail's row decides (Slow 30 / Medium 40 / Fast 50 is only what the row shows). With the key, all three entries of
//   retail's follow-camera rate table (1.0 / 1.3 / 1.6 degrees a tick, found by value in the overlay's data, per level) are
//   written to Fast * percent / 50 every field and retail's own byte 0x15ede4 gets the nearest preset (so the save stays valid);
//   the Modern camera (modcam.rs) scales its stick turn by percent / 50 (the mouse has its own Mouse speed row, mousespeed.rs).
// - Lock-on (row 7): Auto / On / Off for the strafe aim's lock-on (aim.rs `blaster_search_any`: with it off the green crosshair
//   stays centred and the shot flies to the aim point). Auto = on with a pad, off with keyboard and mouse (`prompts::keyboard_now`).
//
// Reserved RAM: the row bytes `LOCKON` / `CAMSEL`, the three text slots from `CO_POOL` (BASE + 0x1000..0x1800 is this section's,
// prompts.rs' canary skips it) and the texts' ids 0x7f80..0x7f82.

/// Lock-on row: 0 Auto, 1 On, 2 Off (the retail update cycles it). Zero RAM is Auto.
pub const LOCKON: u32 = BASE + 0x38;
/// The camera speed row's byte: retail toggles it 0 <-> 1 on every Cross; the host reads each change as one +10 step.
pub(super) const CAMSEL: u32 = BASE + 0x39;
pub const ID_LOCKON: u32 = 0x7f80;
pub const ID_AUTO: u32 = 0x7f81;
/// The camera speed row's value text, "< 50 >" (built from the current percent on every lookup).
pub const ID_PCT: u32 = 0x7f82;
/// The Camera row's two feels (Static = the plain Modern camera, Smooth = the dynamic one; the third value is `ID_CLASSIC`).
pub const ID_STATIC: u32 = 0x7f83;
pub const ID_SMOOTH: u32 = 0x7f84;
/// This section's string pool: one 0x40-byte slot per id (`ID_LOCKON`..=`ID_SMOOTH`); the range is skipped by prompts.rs' canary.
pub(super) const CO_POOL: u32 = BASE + 0x1000;
pub const CO_POOL_END: u32 = BASE + 0x1800;
/// Retail's camera speed byte (save struct): 0 Slow, 1 Medium, 2 Fast.
pub(super) const RETAIL_SPEED: u32 = 0x15ede4;
/// The page-menu mode word (mouse_menu::MODE while a pause page is open) and the row index of the camera speed row.
pub(super) const MENU_MODE: u32 = 0x15f5a4;
pub(super) const SPEED_ROW: u32 = 2;
/// The Camera Options page id (the Graphics page is a copy with the same id: told apart by the page address).
pub(super) const CAMERA_PAGE: u32 = 0x1d;
/// Retail's follow-camera turn rates (radians a tick: 1.0, 1.3, 1.6 degrees) indexed by `RETAIL_SPEED`; the Fast bits are 50 %.
pub(super) const RATE_RETAIL: [u32; 3] = [0x3c8e_fa35, 0x3cb9_dede, 0x3ce4_c388];
/// Where the overlay's data holds that table on every level of both discs (PAL 0x161d70..0x1626e8).
pub(super) const RATE_SCAN: std::ops::Range<u32> = 0x15_ef00..0x17_0000;

/// Lock-on texts per language (En, Fr, De, Es, It) and the Auto value.
#[rustfmt::skip]
pub(super) static CO_TEXTS: [[&[u8]; 5]; 4] = [
	[b"Lock-on", b"Verrouillage", b"Zielerfassung", b"Fijar objetivo", b"Aggancio"],
	[b"Auto", b"Auto", b"Auto", b"Auto", b"Auto"],
	// The Camera row's feels (Es: a-acute 0x84). [I] wording, native review wanted.
	[b"Static", b"Statique", b"Statisch", b"Est\x84tica", b"Statica"],
	[b"Smooth", b"Fluide", b"Weich", b"Suave", b"Fluida"],
];

/// The Lock-on row as last applied (0 Auto, 1 On, 2 Off).
pub(super) static LOCKON_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// The player's camera speed in percent (`camspeed` key); 0 = never set, retail's row decides and nothing is written.
pub(super) static CAMSPEED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// The percent the row shows now (the key, else retail's byte as 30 / 40 / 50).
pub(super) static CAMSHOW: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(50);
/// `CAMSEL` as last seen (0xff: not known yet, the next look is the baseline).
pub(super) static CAMSEL_SEEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0xff);
/// Where the rate table was found (0: not yet) and the bits the host last wrote into it (0: nothing).
pub(super) static RATE_AT: AtomicU32 = AtomicU32::new(0);
pub(super) static RATE_SET: AtomicU32 = AtomicU32::new(0);
/// Fields Left / Right have been held (auto-repeat).
pub(super) static HELD: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];

/// The Lock-on decision of the strafe aim (aim.rs): On, Off, or Auto = on while the last real input came from a pad.
pub fn lockon_on() -> bool {
	!crate::omni::clash::off(crate::omni::clash::LOCKON) && lockon_decide(LOCKON_MODE.load(Ordering::Relaxed), super::super::prompts::keyboard_now())
}

pub(super) fn lockon_decide(mode: u8, keyboard: bool) -> bool {
	match mode {
		1 => true,
		2 => false,
		_ => !keyboard,
	}
}

/// The Modern camera's stick turn scale (modcam.rs `step`): the camera speed row's percent / 50, exactly 1.0 without the key.
pub fn camspeed_scale() -> f32 {
	camspeed_factor(CAMSPEED.load(Ordering::Relaxed))
}

pub(super) fn camspeed_factor(pct: u8) -> f32 {
	if pct == 0 {
		1.0
	} else {
		pct as f32 / 50.0
	}
}

/// The turn rate (radians a tick) for `pct` percent: Fast * pct / 50, so 50 is retail's Fast bits exactly.
pub fn camspeed_rate_bits(pct: u8) -> u32 {
	(f32::from_bits(RATE_RETAIL[2]) * (pct as f32 / 50.0)).to_bits()
}

/// What retail's byte shows as a percent (Slow, Medium, Fast).
pub(super) fn preset_pct(byte: u8) -> u8 {
	[30, 40, 50][byte.min(2) as usize]
}

/// Retail's byte for the nearest preset of `pct` (the save keeps a valid Slow / Medium / Fast).
pub(super) fn preset_byte(pct: u8) -> u8 {
	match pct {
		..=35 => 0,
		36..=45 => 1,
		_ => 2,
	}
}

/// A button of the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
	Left,
	Right,
	Cross,
}

/// The row's value after one `step` from `cur`: Left -10 and Right +10 stop at the ends, Cross +10 wraps from 200 to 10.
pub(super) fn camspeed_step(cur: u8, step: Step) -> u8 {
	use pcopts::{CAMSPEED_MAX as MAX, CAMSPEED_MIN as MIN, CAMSPEED_STEP as STEP};
	let cur = pcopts::snap_camspeed(cur);
	match step {
		Step::Left => cur.saturating_sub(STEP).max(MIN),
		Step::Right => (cur + STEP).min(MAX),
		Step::Cross => {
			if cur >= MAX {
				MIN
			} else {
				cur + STEP
			}
		}
	}
}

/// Whether a held direction steps in this field: on the press, then after 12 fields every 4 (the menu's own auto-repeat feel).
pub(super) fn repeat_due(held: u32) -> bool {
	held == 1 || (held >= 13 && (held - 13) % 4 == 0)
}

/// The row's text: "< 50 >" in retail's font (`<` and `>` are glyphs of all three fonts; a `%` is not: its code 0x25 draws the
/// Cross button, so the unit is left out).
pub(super) fn pct_text(pct: u8) -> Vec<u8> {
	format!("< {pct} >").into_bytes()
}

/// The string of new text `id` in language `lang` (0 En, 1 Fr, 2 De, 3 Es, 4 It); the percent row uses `pct`.
pub(super) fn camopt_string(id: u32, lang: usize, pct: u8) -> Option<Vec<u8>> {
	match id {
		ID_LOCKON => Some(CO_TEXTS[0][lang.min(4)].to_vec()),
		ID_AUTO => Some(CO_TEXTS[1][lang.min(4)].to_vec()),
		ID_PCT => Some(pct_text(pct)),
		ID_STATIC => Some(CO_TEXTS[2][lang.min(4)].to_vec()),
		ID_SMOOTH => Some(CO_TEXTS[3][lang.min(4)].to_vec()),
		ID_MOUSE | ID_MPCT => mousespeed::string(id, lang, pct),
		_ => None,
	}
}

/// Text hook, ids `ID_LOCKON..=ID_MPCT`: the string goes to its pool slot and the lookup returns it.
pub(super) fn camopt_text(c: &mut Cpu, id: u32) -> bool {
	let lang = Lang::from_word(c.mem.rd32(LANG_WORD)).map_or(0, |l| l as usize);
	let pct = if id == ID_MPCT { mousespeed::shown() } else { CAMSHOW.load(Ordering::Relaxed) };
	let Some(s) = camopt_string(id, lang, pct) else { return super::super::prompts::lookup(c) };
	let at = CO_POOL + 0x40 * (id - ID_LOCKON);
	for (i, b) in s.iter().enumerate() {
		c.mem.wr8(at + i as u32, *b);
	}
	c.mem.wr8(at + s.len() as u32, 0);
	c.s(2, at as u64);
	true
}

/// `init`: the options file's rows are known before the first field.
pub(super) fn camopts_init(opts: &PcOptions) {
	LOCKON_MODE.store(opts.lockon_mode(), Ordering::Relaxed);
	CAMSPEED.store(opts.camspeed.map_or(0, pcopts::snap_camspeed), Ordering::Relaxed);
	CAMSEL_SEEN.store(0xff, Ordering::Relaxed);
	RATE_AT.store(0, Ordering::Relaxed);
	RATE_SET.store(0, Ordering::Relaxed);
	mousespeed::load(opts);
	eprintln!(
		"retail: PC options: lock-on {}, camera speed {}, mouse speed {}",
		pcopts::LOCKON[opts.lockon_mode() as usize],
		opts.camspeed.map_or("(retail row)".to_string(), |p| format!("{}%", pcopts::snap_camspeed(p))),
		opts.mousespeed.map_or("(default)".to_string(), |p| format!("{}%", pcopts::snap_camspeed(p)))
	);
}

/// Fresh RAM (`setup`): the row bytes take the file's values; the rate table is looked for again.
pub(super) fn camopts_setup(mem: &mut Mem, s: &St) {
	let l = s.opts.lockon_mode();
	mem.wr8(LOCKON, l);
	mem.wr8(CAMSEL, 0);
	mem.wr8(MOUSESEL, 0);
	LOCKON_MODE.store(l, Ordering::Relaxed);
	CAMSPEED.store(s.opts.camspeed.map_or(0, pcopts::snap_camspeed), Ordering::Relaxed);
	mousespeed::load(&s.opts);
	CAMSEL_SEEN.store(0xff, Ordering::Relaxed);
	RATE_AT.store(0, Ordering::Relaxed);
	RATE_SET.store(0, Ordering::Relaxed);
	for h in &HELD {
		h.store(0, Ordering::Relaxed);
	}
}

/// Whether row `row` (Camera speed `SPEED_ROW`, Mouse speed `MOUSE_ROW`) is the cursor row of the open Camera Options page (not the
/// Graphics copy) of `level`, idle.
pub(super) fn on_row(mem: &mut Mem, level: u32, row: u32) -> bool {
	if mem.rd32(crate::retail::disc::va(MENU_MODE)) != super::super::mouse_menu::MODE {
		return false;
	}
	let Some(&t) = (*crate::retail::disc::pick(&TABLES, &TABLES_USA)).get(level as usize) else { return false };
	let (cp, item) = (t - 0xe0, t + 0x60);
	let ram = &mem.ram;
	let rd = |a: u32| -> u32 {
		let i = (a & 0x01ff_ffff) as usize;
		ram.get(i..i + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
	};
	if rd(item + 0x34) != TAB {
		return false;
	}
	match super::super::mouse_menu::read(&rd, level, super::super::mouse_menu::MODE) {
		Some(snap) => snap.page == cp && snap.id == CAMERA_PAGE && snap.state == CAMERA_PAGE && snap.cur == item && rd(item + 0x38) == row,
		None => false,
	}
}

/// Finds retail's rate table (the three turn rates in a row) in the overlay's data.
pub(super) fn find_rate_table(mem: &mut Mem) -> Option<u32> {
	RATE_SCAN.step_by(4).find(|&a| RATE_RETAIL.iter().enumerate().all(|(k, &w)| mem.rd32(a + 4 * k as u32) == w))
}

/// Writes `bits` into all three entries of the rate table (found on first use, looked for again when a level load put other data
/// there). Only entries that differ are written.
pub(super) fn set_rate_table(mem: &mut Mem, bits: u32) -> bool {
	let applied = RATE_SET.load(Ordering::Relaxed);
	let mut at = RATE_AT.load(Ordering::Relaxed);
	let words = |mem: &mut Mem, a: u32| [mem.rd32(a), mem.rd32(a + 4), mem.rd32(a + 8)];
	if at == 0 || !(words(mem, at) == RATE_RETAIL || (applied != 0 && words(mem, at) == [applied; 3])) {
		let Some(found) = find_rate_table(mem) else {
			RATE_AT.store(0, Ordering::Relaxed);
			return false;
		};
		at = found;
		RATE_AT.store(at, Ordering::Relaxed);
	}
	for k in 0..3 {
		if mem.rd32(at + 4 * k) != bits {
			mem.wr32(at + 4 * k, bits);
		}
	}
	RATE_SET.store(bits, Ordering::Relaxed);
	true
}

/// The button of a speed row this field: Cross (retail toggled the row's byte), else the pressed direction.
pub(super) fn step_of(cross: bool, dir: [bool; 2]) -> Step {
	if cross {
		Step::Cross
	} else if dir[0] {
		Step::Left
	} else {
		Step::Right
	}
}

/// The player set the camera speed to `pct`: remembered, retail's byte follows with the nearest preset.
pub(super) fn set_camspeed(mem: &mut Mem, s: &mut St, pct: u8) {
	CAMSPEED.store(pct, Ordering::Relaxed);
	CAMSHOW.store(pct, Ordering::Relaxed);
	mem.wr8(RETAIL_SPEED, preset_byte(pct));
	s.opts.camspeed = Some(pct);
	s.save();
	eprintln!("retail: PC options: camera speed {pct}%");
}

/// Worker, each field of a level, after `sync`: the Lock-on byte, the camera speed row's steps and the rate table.
pub(super) fn camopts_field(mem: &mut Mem, level: u32, buttons: u16, s: &mut St) {
	// Lock-on: the retail update cycles the byte; a change is applied and remembered.
	let l = mem.rd8(LOCKON).min(2);
	if LOCKON_MODE.swap(l, Ordering::Relaxed) != l {
		eprintln!("retail: PC options: lock-on {}", pcopts::LOCKON[l as usize]);
		s.opts.lockon = Some(l);
		s.save();
	}
	// Camera speed: the percent the row shows, then the steps (Cross: retail toggled CAMSEL; Left / Right: the pad bits).
	let key = CAMSPEED.load(Ordering::Relaxed);
	let mut shown = if key != 0 { key } else { preset_pct(mem.rd8(RETAIL_SPEED)) };
	let sel = mem.rd8(CAMSEL);
	let seen = CAMSEL_SEEN.swap(sel, Ordering::Relaxed);
	let cross = seen != 0xff && seen != sel;
	let mut dir = [false; 2];
	for (i, bit) in [super::super::mouse_menu::LEFT, super::super::mouse_menu::RIGHT].into_iter().enumerate() {
		let held = if buttons & bit != 0 { HELD[i].load(Ordering::Relaxed).saturating_add(1) } else { 0 };
		HELD[i].store(held, Ordering::Relaxed);
		dir[i] = repeat_due(held);
	}
	if (cross || dir[0] != dir[1]) && on_row(mem, level, SPEED_ROW) {
		let next = camspeed_step(shown, step_of(cross, dir));
		if next != shown {
			set_camspeed(mem, s, next);
			shown = next;
		}
	}
	CAMSHOW.store(shown, Ordering::Relaxed);
	mousespeed::field(mem, level, dir, s);
	let key = CAMSPEED.load(Ordering::Relaxed);
	if key != 0 {
		set_rate_table(mem, camspeed_rate_bits(key));
	}
}
