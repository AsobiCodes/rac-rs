//! The Graphics page's Fur and Fur style rows (Ratchet's fur shader, lane fur / furopt, 2026-10-31). Fur: Off / High / Medium / Low like
//! the Fog row (menu byte: 0 Off, 1 High, 2 Medium, 3 Low; `swap_level` turns it into the level 0 Off .. 3 High that the file's `fur`
//! key and the renderer use). Fur style: Soft / Fluffy (byte 0 / 1; the file's `furstyle` key). Both only act with Lighting Enhanced (the
//! shader checks). They sit in the "More effects" section of both Graphics pages (pause and title; `pcctl.rs` `GSECS`), after the
//! Effects section: that one is full at six rows.
//!
//! `pack_gfx` has no free bit, so the two values live here: `apply_graphics` and `init` store them, `menu_graphics()` reads them into
//! `Graphics::fur_q` / `fur_long`. A Graphics preset sets Fur (Low: off, Medium: low, High: high) but not the style (presets.rs).
//!
//! A `pc_options.txt` without a `fur` key (a file from before the row) must not turn the Preset row to Custom: `rows` gives such a
//! file the Fur level of the preset its other rows match (else Off). A new player (no file) matches High, so Fur starts at High.
//! The same rule holds for the keys `fire` and `smoke` (fireopt.rs, smokeopt.rs), and `rows` below is the ONE place that decides it for
//! all three.
use super::*;
use std::sync::atomic::AtomicU8;

/// Graphics -> Fur (the row byte, see above) and Graphics -> Fur style. BASE + 24 is the title stepper (`titlegfx::STEP`).
pub const FUR: u32 = BASE + 23;
pub const FURSTYLE: u32 = BASE + 25;

/// Texts: the two row labels and the style's two values (0x7f24 is the Culling notice, `cullwarn::ID`).
pub const ID_FUR: u32 = 0x7f25;
pub const ID_FURSTYLE: u32 = 0x7f26;
pub const ID_SOFT: u32 = 0x7f27;
pub const ID_FLUFFY: u32 = 0x7f28;

/// The level the Fur row asks for (0 Off .. 3 High) and the style (0 Soft, 1 Fluffy), as last applied.
static FUR_Q: AtomicU8 = AtomicU8::new(3);
static FUR_STYLE: AtomicU8 = AtomicU8::new(0);

/// `apply_graphics` / `init`: the level of the Fur row (0 Off .. 3 High) and the style byte (0 Soft, 1 Fluffy).
pub(super) fn set(level: u8, style: u8) {
	FUR_Q.store(level.min(3), Ordering::Relaxed);
	FUR_STYLE.store(style.min(1), Ordering::Relaxed);
}

/// The Fur level (0 Off .. 3 High) for `Graphics::fur_q`.
pub(super) fn level() -> u8 {
	FUR_Q.load(Ordering::Relaxed)
}

/// The style for `Graphics::fur_long` (true = Fluffy).
pub(super) fn fluffy() -> bool {
	FUR_STYLE.load(Ordering::Relaxed) == 1
}

/// The file's Graphics rows (`PcOptions::graphics_rows`, levels) with THE MISSING-KEY RULE for Fur, Fire and Smoke: a `pc_options.txt`
/// that has no `fur` / `fire` / `smoke` key (a file from before the row) reads the missing key as the level of the Graphics preset the
/// file's other rows match (High -> High, Medium -> Low, Low -> Off), Off when they match none. Players who never changed anything get
/// the new looks at once, and the Preset row does not turn Custom just because the file is old. (The other choice, Off for any existing
/// file, was a const switch that was never set; R2-7 removed its dead branch 2026-11-02.)
pub(super) fn rows(opts: &PcOptions) -> [u8; 19] {
	let mut g = opts.graphics_rows();
	let [fur, fire, smoke] = presets::effects_of_matching(swap_rows(g), opts.hud_sharp(), opts.texfilter.unwrap_or(true));
	for (at, key, preset) in [(15, opts.fur, fur), (17, opts.fire, fire), (18, opts.smoke, smoke)] {
		if key.is_none() {
			g[at] = preset;
		}
	}
	g
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The Fur rows are the 16th and 17th of `ROWS`: Off, High, Medium, Low and Soft, Fluffy; their bytes are outside the title page, the
	/// packed word and the neighbours' (BASE + 24 is the title stepper).
	#[test]
	fn the_fur_rows_are_the_last_two() {
		assert_eq!((GROWS[15], GROWS[16]), (FUR, FURSTYLE));
		assert_eq!(ROWS[15], [ID_FUR, FUR, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW]);
		assert_eq!(ROWS[16], [ID_FURSTYLE, FURSTYLE, ID_SOFT, ID_FLUFFY, 0, 0]);
		assert_eq!((row_values(15), row_values(16), ROWS.len(), FUR, FURSTYLE), (4, 2, 19, cloudopt::CLOUDS + 1, FUR + 2));
		assert_eq!(FUR + 1, titlegfx::STEP);
		assert_eq!((DEFAULT_ROWS[15], DEFAULT_ROWS[16]), (1, 0), "a new player: High, Soft");
		assert_eq!([0u8, 1, 2, 3].map(|b| fogopt::name(swap_level(b))), ["classic", "high", "medium", "low"]);
		// The word does not hold the Fur rows: the other rows are what they were.
		let mut other = DEFAULT_ROWS;
		(other[15], other[16]) = (3, 1);
		assert_eq!(pack_gfx(DEFAULT_ROWS), pack_gfx(other));
		assert_eq!(unpack_gfx(pack_gfx(DEFAULT_ROWS))[15..], [0, 0, 0, 0]);
	}

	#[test]
	fn the_level_and_the_style_are_clamped() {
		let _turn = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		set(7, 9);
		assert_eq!((level(), fluffy()), (3, true));
		set(0, 0);
		assert_eq!((level(), fluffy()), (0, false));
		set(3, 0);
	}

	/// The five languages have all four texts, short enough for the cell (the longest existing label is "Distancia de dibujado").
	#[test]
	fn the_texts_exist_in_five_languages() {
		for id in [ID_FUR, ID_FURSTYLE, ID_SOFT, ID_FLUFFY] {
			let k = NEW_IDS.iter().position(|&n| n == id).expect("listed in NEW_IDS");
			assert!(TEXTS[k].iter().all(|t| !t.is_empty() && t.len() <= 22), "text {id:#x}");
		}
		let at = |id: u32| NEW_IDS.iter().position(|&n| n == id).unwrap();
		assert_eq!(TEXTS[at(ID_FUR)][0], b"Fur");
		assert_eq!(TEXTS[at(ID_FURSTYLE)][2], b"Fellstil");
		assert_eq!(TEXTS[at(ID_FLUFFY)][1], b"Duveteux");
	}

	/// Every file gives the same Fur level as before it had the key if its other rows match a preset; a file with the key keeps it.
	#[test]
	fn a_file_without_the_key_follows_its_preset() {
		let tab = |preset: &str| include_str!("../../../../assets/gfx_presets.txt").lines().filter_map(|l| l.strip_prefix(&format!("{preset}."))).map(|l| format!("{l}\n")).collect::<String>();
		let of = |t: &str| {
			let g = rows(&PcOptions::parse(t));
			[g[15], g[17], g[18]]
		};
		// No file: the other rows are the High preset's.
		assert_eq!(of(""), [3; 3]);
		for (preset, level) in [("low", 0u8), ("medium", 1), ("high", 3)] {
			let text: String = tab(preset).lines().filter(|l| !l.starts_with("fur=") && !l.starts_with("fire=") && !l.starts_with("smoke=")).map(|l| format!("{l}\n")).collect();
			assert!(!text.is_empty(), "{preset}: the table has its rows");
			assert_eq!(of(&text), [level; 3], "{preset} rows without the keys");
			// The keys win, whatever the other rows are; a missing one still follows the preset.
			assert_eq!(of(&format!("{text}fur=low\n")), [1, level, level]);
			assert_eq!(of(&format!("{text}fire=off\nsmoke=high\n")), [level, 0, 3]);
			assert_eq!(of(&format!("{text}fur=off\nfire=low\nsmoke=medium\n")), [0, 1, 2]);
		}
		// Rows that match no preset: Off.
		assert_eq!(of("water=low\nshadows=high\nsunrays=medium\n"), [0; 3]);
	}
}
