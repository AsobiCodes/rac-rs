//! [PC] The player's upscaler choice (lane ndlss, stage 5; no retail counterpart): the Upscaler and Upscale quality rows of the
//! game's modern menu (R&C1: Graphics -> Resolution, rac1 modern_menu/rows_upscale.rs; set and published by rac1
//! retail/pcext/ui_bridge_more.rs). Kept in the options file only (`upscaler=off|fsr|dlss`, `upscale_q=native|quality|balanced|
//! performance|ultra`): no RAM byte, so the game and its RAM hash never see it. `slot.rs` `setting` reads it while the dev env
//! RAC_UPSCALE is unset; a change re-plans the render size at the next frame (gpu/rscale.rs `rscale_sync`), no restart. A game that
//! never calls `init` or a setter keeps the default: Off. An upscaler the engine found it cannot run on this PC (`set_runs`: no
//! DLLs, the GPU or the runtime refused) is not offered by the menu (ui/live.rs `offer`) and a choice of it is Off (`kind`); the
//! file keeps the choice, so it comes back with the DLLs. FSR 1 was removed in lane nfsr1: `upscaler=fsr1` loads as FSR.

use super::slot::{Kind, Setting, DEFAULT_RATIO, QUALITY, SHARP};
use crate::pcopts::PcOptions;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};

/// Options file key of the Upscaler row.
pub const KEY: &str = "upscaler";
/// Options file key of the Upscale quality row.
pub const KEY_Q: &str = "upscale_q";
/// The Upscaler row's values in the file, in row order: Off, FSR (the newest FSR the PC's FidelityFX runtime offers: 3.1 or 4.x,
/// ffx_frame.rs), `dlss` (lane dlss2: NVIDIA DLSS 4.5 through NGX, dlss.rs). Where one cannot run it is not offered (`set_runs`).
pub const KINDS: [&str; 3] = ["off", "fsr", "dlss"];
/// The row value of `fsr` (its text is the runtime's version, ui/live.rs).
pub const FFX_VALUE: i32 = 1;
/// The row value of `dlss` (its text: "DLSS 4.5" once NGX reported the model, ui/live.rs).
pub const DLSS_VALUE: i32 = 2;
/// Old file values and the row value they load as: `fsr1` (FSR 1, removed in lane nfsr1) = FSR (FidelityFX), which is Off where
/// FSR cannot run (`kind`). The file keeps `fsr1` until the row is changed.
const ALIASES: [(&str, u8); 1] = [("fsr1", FFX_VALUE as u8)];
/// The Upscale quality row's values in the file, in row order (names of `slot::QUALITY`).
pub const QUALITIES: [&str; 5] = ["native", "quality", "balanced", "performance", "ultra"];
/// The Upscale quality row's default: Quality (67 %).
pub const DEFAULT_Q: u8 = 1;
/// The quality row's 100 % value while DLSS is chosen: NVIDIA's DLAA (ngx_quality 1.0); other upscalers keep "Native AA".
/// A product name, the same in all languages.
pub const DLAA_TEXT: &str = "DLAA (100%)";

static KIND: AtomicU8 = AtomicU8::new(0);
static Q: AtomicU8 = AtomicU8::new(DEFAULT_Q);
/// Bit v set: row value v cannot run on this PC (`set_runs`; nothing known yet = it runs; known from the window's device on:
/// upscale.rs `Gpu::upscale_probe`).
static NOT_RUNNING: AtomicU8 = AtomicU8::new(0);

/// The row index of a file value (case and spaces ignored); a missing or unknown value = `def`.
fn index(v: Option<&str>, names: &[&str], def: u8) -> u8 {
	v.and_then(|v| names.iter().position(|n| n.eq_ignore_ascii_case(v.trim()))).map_or(def, |i| i as u8)
}

/// The Upscaler row value of a file value: `KINDS`, then the old names (`ALIASES`); unknown = Off.
fn kind_index(v: Option<&str>) -> u8 {
	let old = v.and_then(|v| ALIASES.iter().find(|a| a.0.eq_ignore_ascii_case(v.trim())));
	old.map_or_else(|| index(v, &KINDS, 0), |a| a.1)
}

/// Start-up: the choice from the options file.
pub fn init(opts: &PcOptions) {
	KIND.store(kind_index(opts.extra_get(KEY)), Ordering::Relaxed);
	Q.store(index(opts.extra_get(KEY_Q), &QUALITIES, DEFAULT_Q), Ordering::Relaxed);
	crate::ui::live::watch(KEY); // [PC] the menu showing the row = the moment to name the FidelityFX version (upscale.rs)
	label_quality(kind());
	if kind() != 0 {
		eprintln!("upscale: Upscaler {}, quality {} (options file)", KINDS[kind() as usize], QUALITIES[quality() as usize]);
	}
}

/// The Upscaler row's value in force: 0 Off, 1 FSR (FidelityFX), 2 DLSS; the chosen one, or Off while it cannot run here.
pub fn kind() -> u8 {
	effective(KIND.load(Ordering::Relaxed), NOT_RUNNING.load(Ordering::Relaxed))
}

/// `kind` for a chosen row value and the not-running bits.
fn effective(kind: u8, not_running: u8) -> u8 {
	if kind < 8 && not_running & (1 << kind) != 0 { 0 } else { kind }
}

/// The row value of a slot upscaler.
fn value_of(kind: Kind) -> i32 {
	match kind {
		Kind::Fsr => FFX_VALUE,
		Kind::Dlss => DLSS_VALUE,
	}
}

/// Whether `kind` runs on this PC as far as the engine knows (slot.rs `active_of`: no = no upscaler, the plain picture).
pub(in crate::viewer) fn runs(kind: Kind) -> bool {
	NOT_RUNNING.load(Ordering::Relaxed) & (1 << value_of(kind)) == 0
}

/// The engine found out whether row value `v` (`FFX_VALUE` / `DLSS_VALUE`) runs on this PC: the menu offers it or not (ui/live.rs
/// `offer`), a choice of it is Off while not (`kind`; slot.rs `active`: the next `rscale_sync` plans the window size). Cheap when
/// nothing changes (called per frame while the menu shows the row); one log line per change.
pub(in crate::viewer) fn set_runs(v: i32, runs: bool) {
	let bit = 1u8 << v.clamp(0, 7);
	let before = if runs { NOT_RUNNING.fetch_and(!bit, Ordering::Relaxed) } else { NOT_RUNNING.fetch_or(bit, Ordering::Relaxed) };
	if (before & bit == 0) == runs {
		return;
	}
	crate::ui::live::offer(KEY, v, runs);
	label_quality(kind());
	let name = KINDS.get(v.max(0) as usize).copied().unwrap_or("?");
	eprintln!("upscale: {name} {}", if runs { "runs on this PC again: offered" } else { "cannot run on this PC: not offered, Off while chosen" });
}

/// The quality row's 100 % value says "DLAA (100%)" while the Upscaler is DLSS, its static name otherwise (ui/live.rs).
/// Bumped by every `label_quality`: dlss_frame.rs `label` repeats its "no DLSS" work only after one (review R21 m5).
static LABEL_GEN: AtomicU32 = AtomicU32::new(0);

pub fn label_gen() -> u32 {
	LABEL_GEN.load(Ordering::Relaxed)
}

fn label_quality(kind: u8) {
	LABEL_GEN.fetch_add(1, Ordering::Relaxed);
	if kind as i32 == DLSS_VALUE {
		crate::ui::live::set(KEY_Q, 0, DLAA_TEXT.to_string());
	} else {
		crate::ui::live::clear(KEY_Q, 0);
	}
}

/// The Upscale quality row's value: 0 Native AA (DLAA with DLSS), 1 Quality, 2 Balanced, 3 Performance, 4 Ultra performance.
pub fn quality() -> u8 {
	Q.load(Ordering::Relaxed)
}

/// Row value `v` (clamped to `names`) into `cell` and the options file key `key`; true = the options file changed.
fn put(cell: &AtomicU8, opts: &mut PcOptions, key: &str, names: &[&str], v: i32) -> bool {
	let v = v.clamp(0, names.len() as i32 - 1) as u8;
	if cell.swap(v, Ordering::Relaxed) == v {
		return false;
	}
	opts.extra_set(key, Some(names[v as usize].to_string()));
	eprintln!("upscale: PC options: {key} {}", names[v as usize]);
	true
}

/// The menu set the Upscaler row (0 Off, 1 FSR, 2 DLSS): in force from the next frame and remembered; true = the options file
/// changed.
pub fn set_kind(opts: &mut PcOptions, v: i32) -> bool {
	let changed = put(&KIND, opts, KEY, &KINDS, v);
	label_quality(kind());
	changed
}

/// The menu set the Upscale quality row (0 Native AA .. 4 Ultra performance); true = the options file changed.
pub fn set_quality(opts: &mut PcOptions, v: i32) -> bool {
	put(&Q, opts, KEY_Q, &QUALITIES, v)
}

/// The slot setting of a choice: None for Off; FSR (FidelityFX) or DLSS at the quality's window / render ratio with the usual
/// sharpening.
pub(in crate::viewer) fn setting_of(kind: u8, q: u8) -> Option<Setting> {
	let kind = match kind {
		0 => return None,
		1 => Kind::Fsr,
		_ => Kind::Dlss,
	};
	let name = QUALITIES.get(q as usize).copied().unwrap_or(QUALITIES[DEFAULT_Q as usize]);
	let ratio = QUALITY.iter().find(|(n, _)| *n == name).map_or(DEFAULT_RATIO, |&(_, r)| r);
	Some(Setting { kind, ratio, sharp: SHARP })
}

/// The player's choice in force as a slot setting (`slot::setting` when RAC_UPSCALE is unset; Off while it cannot run here).
pub(in crate::viewer) fn setting() -> Option<Setting> {
	setting_of(kind(), quality())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn file_values_map_to_rows_and_unknown_ones_to_the_default() {
		assert_eq!(kind_index(Some("off")), 0);
		assert_eq!(kind_index(Some("fsr")), 1);
		assert_eq!(kind_index(Some(" FSR ")), 1);
		assert_eq!(KINDS[FFX_VALUE as usize], "fsr");
		assert_eq!(KINDS[DLSS_VALUE as usize], "dlss");
		assert_eq!(kind_index(Some("dlss")), 2);
		assert_eq!(kind_index(Some("xess")), 0, "a value this build does not know = Off");
		assert_eq!(kind_index(None), 0);
		assert!(!KINDS.contains(&"fsr1"), "FSR 1 is gone (lane nfsr1)");
		assert_eq!(index(Some("performance"), &QUALITIES, DEFAULT_Q), 3);
		assert_eq!(index(Some("ultra"), &QUALITIES, DEFAULT_Q), 4);
		assert_eq!(index(Some("extreme"), &QUALITIES, DEFAULT_Q), DEFAULT_Q);
		for q in QUALITIES {
			assert!(QUALITY.iter().any(|(n, _)| *n == q), "{q} is a slot quality name");
		}
	}

	/// Migration (lane nfsr1): a file that chose FSR 1 loads as FSR; where FSR cannot run that is Off, where DLSS cannot run a
	/// DLSS choice is Off; Off stays Off whatever runs. (Pure: the process-wide not-running bits are left alone.)
	#[test]
	fn an_fsr1_file_loads_as_fsr_and_an_upscaler_that_cannot_run_is_off() {
		let fsr1 = kind_index(Some("fsr1"));
		assert_eq!(fsr1, FFX_VALUE as u8);
		assert_eq!(kind_index(Some(" FSR1 ")), FFX_VALUE as u8);
		let (no_fsr, no_dlss) = (1u8 << FFX_VALUE, 1u8 << DLSS_VALUE);
		assert_eq!(effective(fsr1, 0), FFX_VALUE as u8, "FidelityFX runs: FSR");
		assert_eq!(effective(fsr1, no_fsr), 0, "no FidelityFX: Off");
		assert_eq!(effective(fsr1, no_dlss), FFX_VALUE as u8);
		assert_eq!(effective(DLSS_VALUE as u8, no_dlss), 0);
		assert_eq!(effective(DLSS_VALUE as u8, no_fsr), DLSS_VALUE as u8);
		assert_eq!(effective(0, 0xff), 0);
		assert_eq!(setting_of(effective(fsr1, no_fsr), 1), None, "Off: no slot at all, the plain picture");
		let mut o = PcOptions::new();
		o.extra_set(KEY, Some("fsr1".to_string()));
		assert_eq!(kind_index(o.extra_get(KEY)), FFX_VALUE as u8, "read back from an options file");
	}

	/// The rows' percentages (100 / 67 / 59 / 50) are the slot's ratios; Off is no setting at all (the old picture).
	#[test]
	fn the_choice_gives_fsr_at_the_quality_ratio_and_off_gives_none() {
		assert_eq!(setting_of(0, 3), None);
		let pct: Vec<u32> = (0..5).map(|q| (100.0 / setting_of(1, q).unwrap().ratio).round() as u32).collect();
		assert_eq!(pct, vec![100, 67, 59, 50, 33]);
		assert_eq!(setting_of(DLSS_VALUE as u8, 0).unwrap().kind, Kind::Dlss);
		let s = setting_of(FFX_VALUE as u8, 3).unwrap();
		assert_eq!((s.kind, s.sharp, s.ratio), (Kind::Fsr, SHARP, 2.0));
		for k in [Kind::Fsr, Kind::Dlss] {
			assert_eq!(setting_of(value_of(k) as u8, 1).map(|s| s.kind), Some(k), "row value and slot kind agree");
		}
		assert_eq!(setting_of(1, 9).unwrap().ratio, DEFAULT_RATIO, "an out-of-range quality = Quality");
	}

	/// The setter writes the file key once per change (local cells: the process-wide choice is left alone).
	#[test]
	fn a_change_writes_the_key_and_the_same_value_writes_nothing() {
		let cell = AtomicU8::new(0);
		let mut o = PcOptions::new();
		assert!(put(&cell, &mut o, KEY, &KINDS, 1));
		assert_eq!(o.extra_get(KEY), Some("fsr"));
		assert!(!put(&cell, &mut o, KEY, &KINDS, 1), "same value: no save");
		assert!(put(&cell, &mut o, KEY, &KINDS, 7), "7 is clamped to DLSS");
		assert_eq!(o.extra_get(KEY), Some("dlss"));
		assert!(put(&cell, &mut o, KEY, &KINDS, 0));
		assert_eq!(o.extra_get(KEY), Some("off"), "Off is written too (a launcher reads the key)");
		let q = AtomicU8::new(DEFAULT_Q);
		assert!(put(&q, &mut o, KEY_Q, &QUALITIES, 3));
		assert_eq!((o.extra_get(KEY_Q), q.load(Ordering::Relaxed)), (Some("performance"), 3));
	}

	/// With DLSS the quality row's first value reads DLAA; any other upscaler gives the row its static name back.
	#[test]
	fn dlss_names_the_100_percent_value_dlaa() {
		use crate::ui::live;
		label_quality(DLSS_VALUE as u8);
		assert_eq!(live::get(KEY_Q, 0).as_deref(), Some(DLAA_TEXT));
		assert_eq!(live::get(KEY_Q, 1), None, "only the 100 % value");
		label_quality(FFX_VALUE as u8);
		assert_eq!(live::get(KEY_Q, 0), None);
		label_quality(DLSS_VALUE as u8);
		label_quality(0);
		assert_eq!(live::get(KEY_Q, 0), None, "Off clears it too");
	}
}
