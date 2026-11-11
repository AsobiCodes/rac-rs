//! [PC] The DLSS preset choice (lane disp worker 8, user 2026-11-08 after GitHub #84; no retail counterpart): which DLSS Super
//! Resolution model NGX runs. No game menu row since 2026-11-08 (user: "too many options"; was R&C1 Graphics -> Resolution): the
//! launcher's row writes the key. Options file only (`dlss_preset=m|l|k|j`, written only when not Auto; absent or unknown = Auto): no RAM byte.
//! - Auto = K in every mode, L for Ultra performance (`auto_preset`; user 2026-11-08 "we do the K preset the default one" after
//!   measuring test build 14 at 2560x1440 DLSS Quality on an RTX 5070 Ti: the DLSS pass 0.82..0.89 ms with K vs 1.35..1.50 ms with
//!   M, Veldin 176 -> 197 fps; was M, L for Ultra performance, `abi::preset_dlss45`).
//! - The others ask NGX for that preset in every mode (rac_ngx.dll puts `RacNgxCreate.preset` on all 6 per-mode hints).
//! - What NVIDIA says the presets are for [C] (DLSS_Programming_Guide_Release.pdf 310.9.16 section 3.12 + nvsdk_ngx_defs.h):
//!   M = "similar image quality improvements as Preset L but closer in speed to Presets J, K" (default for Performance);
//!   L = "a sharper, more stable image with less ghosting than Preset J, K but more expensive" (default for Ultra performance);
//!   K = "less expensive performance wise compared to Preset L" (default for DLAA / Quality / Balanced; 1st-gen transformer);
//!   J = "similar to preset K ... slightly less ghosting at the cost of extra flickering; K is generally recommended over J".
//!   L and M are "peak performant on RTX 40 series GPUs and above". Other letters are removed / deprecated / reserved: not offered.
//! - A change applies at the next frame: the preset is part of dlss_frame.rs `Key`, so slot.rs makes the GPU idle and the feature
//!   is made again with the new hint (one short hitch). `RAC_DLSS_PRESET` (tests) wins over the row. The render thread reads
//!   `in_force` ONCE per frame (upscale.rs `upscale_begin` -> `Prep::dlss_row`; the worker thread writes the row any time), so the
//!   idle check and the new key always agree (review REVIEW3 F1).

use rac_ngx_abi as abi;
use std::sync::atomic::{AtomicU8, Ordering};

/// Options file key of the DLSS preset row.
pub const KEY: &str = "dlss_preset";
/// The row's values in the file, in row order: Auto (never written), then the presets.
pub const NAMES: [&str; 5] = ["auto", "m", "l", "k", "j"];
/// The NGX preset of each row value (0 = Auto: per mode, `preset_of`).
const PRESETS: [u32; 5] = [0, abi::PRESET_M, abi::PRESET_L, abi::PRESET_K, abi::PRESET_J];

static CHOICE: AtomicU8 = AtomicU8::new(0);

/// The row value of a file / env value (case and spaces ignored; a missing or unknown value = Auto).
pub fn index(v: Option<&str>) -> u8 {
	v.and_then(|v| NAMES.iter().position(|n| n.eq_ignore_ascii_case(v.trim()))).map_or(0, |i| i as u8)
}

/// `RAC_DLSS_PRESET` (tests): a row value that wins over the menu; None = unset (the row).
fn env() -> Option<u8> {
	static ENV: std::sync::OnceLock<Option<u8>> = std::sync::OnceLock::new();
	*ENV.get_or_init(|| std::env::var("RAC_DLSS_PRESET").ok().map(|v| index(Some(&v))))
}

/// Start-up (upscale choice.rs `init`): the row from the options file.
pub fn init(opts: &crate::pcopts::PcOptions) {
	CHOICE.store(index(opts.extra_get(KEY)), Ordering::Relaxed);
	if let Some(e) = env() {
		eprintln!("upscale: RAC_DLSS_PRESET={} wins over the DLSS preset row", NAMES[e as usize]);
	}
}

/// The row's value: 0 Auto, 1 M, 2 L, 3 K, 4 J.
pub fn get() -> u8 {
	CHOICE.load(Ordering::Relaxed)
}

/// The value in force: `RAC_DLSS_PRESET`, else the row.
pub fn in_force() -> u8 {
	env().unwrap_or_else(get)
}

/// The menu set the row; true = the options file changed. Auto removes the key (the file stays as before the row existed).
pub fn set(opts: &mut crate::pcopts::PcOptions, v: i32) -> bool {
	let v = v.clamp(0, NAMES.len() as i32 - 1) as u8;
	if CHOICE.swap(v, Ordering::Relaxed) == v {
		return false;
	}
	opts.extra_set(KEY, (v != 0).then(|| NAMES[v as usize].to_string()));
	eprintln!("upscale: PC options: {KEY} {}", NAMES[v as usize]);
	true
}

/// The NGX preset for row value `v` and NGX quality mode `quality`: Auto = `auto_preset`.
pub fn preset_of(v: u8, quality: u32) -> u32 {
	match PRESETS.get(v as usize).copied().unwrap_or(0) {
		0 => auto_preset(quality),
		p => p,
	}
}

/// Auto: K (1st-gen transformer, the cheaper model) in every mode, L for Ultra performance (NVIDIA's model for that mode).
pub const fn auto_preset(quality: u32) -> u32 {
	if quality == abi::QUALITY_ULTRA_PERFORMANCE {
		abi::PRESET_L
	} else {
		abi::PRESET_K
	}
}

/// The log name of row value `v` ("auto", "L").
pub fn log_name(v: u8) -> String {
	match v {
		0 => String::from("auto"),
		v => NAMES.get(v as usize).map_or(String::from("?"), |n| n.to_ascii_uppercase()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::pcopts::PcOptions;

	/// Auto is K in every mode, L for Ultra performance (user 2026-11-08); the others ask their own letter everywhere.
	#[test]
	fn auto_is_k_and_a_letter_is_asked_in_every_mode() {
		let modes = [abi::QUALITY_DLAA, abi::QUALITY_MAX_QUALITY, abi::QUALITY_BALANCED, abi::QUALITY_MAX_PERF, abi::QUALITY_ULTRA_PERFORMANCE];
		for q in modes {
			assert_eq!(preset_of(0, q), auto_preset(q));
			for (v, letter) in [(1, 'M'), (2, 'L'), (3, 'K'), (4, 'J')] {
				assert_eq!(abi::preset_letter(preset_of(v, q)), letter);
			}
		}
		for q in [abi::QUALITY_DLAA, abi::QUALITY_MAX_QUALITY, abi::QUALITY_BALANCED, abi::QUALITY_MAX_PERF] {
			assert_eq!(abi::preset_letter(preset_of(0, q)), 'K');
		}
		assert_eq!(abi::preset_letter(preset_of(0, abi::QUALITY_ULTRA_PERFORMANCE)), 'L');
		assert_eq!(preset_of(9, abi::QUALITY_DLAA), abi::PRESET_K, "out of range = Auto");
		assert_eq!((log_name(0), log_name(2)), (String::from("auto"), String::from("L")));
	}

	#[test]
	fn file_values_parse_and_unknown_ones_are_auto() {
		assert_eq!(index(Some("l")), 2);
		assert_eq!(index(Some(" M ")), 1);
		assert_eq!(index(Some("K")), 3);
		assert_eq!(index(Some("j")), 4);
		assert_eq!(index(Some("auto")), 0);
		assert_eq!(index(Some("e")), 0, "a removed / deprecated preset = Auto");
		assert_eq!(index(None), 0);
	}

	/// Written only when not Auto; the same value writes nothing; Auto removes the key. (The process-wide cell is put back.)
	#[test]
	fn a_change_writes_the_key_only_when_not_auto() {
		let before = get();
		CHOICE.store(0, Ordering::Relaxed);
		let mut o = PcOptions::new();
		assert!(!set(&mut o, 0), "Auto again: no save");
		assert_eq!(o.extra_get(KEY), None);
		assert!(set(&mut o, 2));
		assert_eq!(o.extra_get(KEY), Some("l"));
		assert_eq!(index(o.extra_get(KEY)), 2, "read back");
		assert!(!set(&mut o, 2));
		assert!(set(&mut o, 9), "clamped to J");
		assert_eq!(o.extra_get(KEY), Some("j"));
		assert!(set(&mut o, 0));
		assert_eq!(o.extra_get(KEY), None, "Auto is never written");
		CHOICE.store(before, Ordering::Relaxed);
	}
}
