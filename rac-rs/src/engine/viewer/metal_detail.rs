//! [PC] The Metal reflection detail (lane mres, user 2026-11-08: a speed cut for the metal floor reflections; "the new low becomes
//! the new high, and we add lower levels"): the size of the planar floor reflection (metalplanar.rs) as a divisor of the render size
//! per axis. `Full` = 1/2 (the reflection source's size, metalenv.rs `ssr_size`: the v0.5 picture), the engine default, so a game
//! that never calls `init` / the setter (R&C2) keeps it; the R&C1 Graphics -> Effects row (rac1 modern_menu/rows_effects.rs, set and
//! published by rac1 retail/pcext/ui_bridge_more.rs, its default in rac1 retail/pcmetal.rs `DETAIL_DEFAULT`) offers High = 1/4,
//! Medium = 1/6, Low = 1/8. A smaller target has bigger texels, so the shader's mip pick goes down by log2(divisor / 2) levels
//! (u.metal[13].w, metalfx.wgsl `metal_planar`; fractional at Medium, the sampler blends mips linearly): each blur level covers the
//! same screen size as at Full, the 4-tap offset (a texel of the picked level) too; only the sharpest levels are gone. The mip count
//! drops by the whole levels of that bias (`planar_size`), so the blurriest level reached stays Full's. The screen-space walk and
//! the sky faces are the same at every level. Kept in the options file only (`metal_detail=high|medium|low`): no RAM byte, so the
//! game and its RAM hash never see it. Read each frame by `MetalEnv::alloc`, so a change remakes the planar targets on the next
//! frame.

use crate::pcopts::PcOptions;
use std::sync::atomic::{AtomicU8, Ordering};

/// Options file key of the row; values `high` / `medium` / `low` (`Full` is never written: the engine default only).
pub const KEY: &str = "metal_detail";

/// The planar target's size: a divisor of the render size per axis (`divisor`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Detail {
	Low = 0,
	Medium = 1,
	High = 2,
	Full = 3,
}

/// The engine default: Full (the v0.5 half-size picture). ONE place; a pack passes its own default to `init`.
pub const ENGINE_DEFAULT: Detail = Detail::Full;

static LEVEL: AtomicU8 = AtomicU8::new(ENGINE_DEFAULT as u8);

impl Detail {
	const ALL: [Detail; 4] = [Detail::Low, Detail::Medium, Detail::High, Detail::Full];

	/// The render size over the planar target's size, per axis.
	pub const fn divisor(self) -> u32 {
		match self {
			Detail::Full => 2,
			Detail::High => 4,
			Detail::Medium => 6,
			Detail::Low => 8,
		}
	}

	/// Mip levels the shader's pick goes down (u.metal[13].w): log2(divisor / 2), the target's texel size in reflection source texels.
	/// Exact constants (Full 0.0 and High 1.0 are the bytes the earlier builds wrote).
	pub(crate) const fn lod_bias(self) -> f32 {
		match self {
			Detail::Full => 0.0,
			Detail::High => 1.0,
			Detail::Medium => 1.584_962_5,
			Detail::Low => 2.0,
		}
	}

	/// The row's UI value (0 Low, 1 Medium, 2 High; 3 Full, offered by no R&C1 row).
	pub const fn ui(self) -> i32 {
		self as i32
	}

	/// The level of a UI value (clamped).
	pub fn from_ui(v: i32) -> Detail {
		Detail::ALL[v.clamp(0, 3) as usize]
	}

	fn name(self) -> &'static str {
		match self {
			Detail::Low => "low",
			Detail::Medium => "medium",
			Detail::High => "high",
			Detail::Full => "full",
		}
	}
}

/// The value of the file key: `high` / `medium` / `low` (case and spaces ignored); a missing or unknown value is `default`. The
/// lane's first test builds wrote `high` (then the half size) and `low` (then the quarter size): they read as High and Low, the
/// best and the fastest choice as before.
fn level_of(v: Option<&str>, default: Detail) -> Detail {
	match v.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
		Some("high") => Detail::High,
		Some("medium") => Detail::Medium,
		Some("low") => Detail::Low,
		_ => default,
	}
}

/// Start-up: the row from the options file; `default` = the pack's (when the file has no valid value).
pub fn init(opts: &PcOptions, default: Detail) {
	LEVEL.store(level_of(opts.extra_get(KEY), default) as u8, Ordering::Relaxed);
	if get() != default {
		eprintln!("metal: reflection detail {} (options file)", get().name());
	}
}

/// The level in force.
pub fn get() -> Detail {
	Detail::from_ui(LEVEL.load(Ordering::Relaxed) as i32)
}

/// The menu set the row: in force from the next frame and remembered; true = the options file changed.
pub fn set_from_ui(opts: &mut PcOptions, d: Detail) -> bool {
	if Detail::from_ui(LEVEL.swap(d as u8, Ordering::Relaxed) as i32) == d {
		return false;
	}
	opts.extra_set(KEY, (d != Detail::Full).then(|| d.name().to_string()));
	eprintln!("metal: PC options: reflection detail {}", d.name());
	true
}

/// The smallest planar target side.
const MIN_SIDE: u32 = 16;

/// The planar targets' size and mip count for the reflection source size `src` (metalenv.rs `ssr_size`: half the render size, a
/// multiple of 16, at least 32) and `mips` levels at Full: Full = both unchanged; else each side 2 * src / divisor rounded to the
/// nearest multiple of 8 (exact at High: src / 2), at least `MIN_SIDE`, and `mips` minus the whole levels of `lod_bias` (at least 1),
/// so the blurriest level stays Full's and every level halves exactly (a multiple of 8 halves twice).
pub(crate) fn planar_size(src: (u32, u32), mips: u32, d: Detail) -> ((u32, u32), u32) {
	if d == Detail::Full {
		return (src, mips);
	}
	let n = 8 * d.divisor();
	let side = |s: u32| ((2 * s + n / 2) / n * 8).max(MIN_SIDE);
	((side(src.0), side(src.1)), mips.saturating_sub(d.lod_bias() as u32).max(1))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn file_values() {
		for d in [Detail::High, Detail::Low, Detail::Full] {
			assert_eq!(level_of(None, d), d);
			assert_eq!(level_of(Some("maybe"), d), d, "an unknown value is the default");
			assert_eq!(level_of(Some("full"), d), d, "Full is never a file value");
		}
		assert_eq!(level_of(Some(" HIGH "), Detail::Low), Detail::High);
		assert_eq!(level_of(Some("Medium"), Detail::High), Detail::Medium);
		assert_eq!(level_of(Some("low"), Detail::High), Detail::Low);
		assert_eq!(ENGINE_DEFAULT, Detail::Full, "a game that never calls init keeps the v0.5 picture");
		for d in Detail::ALL {
			assert_eq!(Detail::from_ui(d.ui()), d);
		}
	}

	/// The bias is log2(divisor / 2) (the target's texel in reflection source texels).
	#[test]
	fn the_bias_is_the_texel_ratio() {
		for d in Detail::ALL {
			assert!((d.lod_bias() - (d.divisor() as f32 / 2.0).log2()).abs() < 1e-6, "{d:?}");
		}
	}

	/// Full is today's targets exactly; High halves the source (the lane's first Low, byte for byte), Medium a third, Low a quarter:
	/// multiples of 8, at least 16 x 16, every mip halving exactly, the blurriest level Full's.
	#[test]
	fn the_size_rule() {
		// 2560 x 1440 -> source 1280 x 720; 1920 x 1080 -> 960 x 544; Steam Deck 1280 x 800 -> 640 x 400; the 32 x 32 floor.
		let cases = [
			((1280, 720), [(640, 360), (424, 240), (320, 184)]),
			((960, 544), [(480, 272), (320, 184), (240, 136)]),
			((640, 400), [(320, 200), (216, 136), (160, 104)]),
			((32, 32), [(16, 16), (16, 16), (16, 16)]),
		];
		for (src, want) in cases {
			assert_eq!(planar_size(src, 4, Detail::Full), (src, 4));
			for (d, size) in [Detail::High, Detail::Medium, Detail::Low].into_iter().zip(want) {
				let ((w, h), m) = planar_size(src, 4, d);
				assert_eq!((w, h), size, "{src:?} {d:?}");
				assert_eq!(m - 1 + d.lod_bias() as u32, 3, "{d:?}: the top level is Full's top in whole levels");
				assert!(w % 8 == 0 && h % 8 == 0 && w >= MIN_SIDE && h >= MIN_SIDE);
				assert!(w % (1 << (m - 1)) == 0 && h % (1 << (m - 1)) == 0, "{src:?} {d:?}: every mip halves exactly");
			}
		}
		assert_eq!(planar_size((32, 32), 1, Detail::Low).1, 1, "never zero mips");
	}

	/// A CPU copy of metalfx.wgsl `metal_planar`'s two lod lines: (l0, lod) for the roughness lod `r`, the contact blur `extra`, the
	/// top level `top` and the bias `w` (u.metal[13].w).
	fn shader_lod(r: f32, extra: f32, top: f32, w: f32) -> f32 {
		let fb = w.floor();
		let l0 = r.clamp(0.0, top + fb);
		(l0 + extra - w).clamp(0.0, top - (w - fb))
	}

	/// The mip rule: the level picked at a lower detail covers the same screen footprint (texel size in reflection source texels,
	/// 2^lod * src / dim) as Full's, wherever Full's lod is at least the bias (below it the lower detail reads its sharpest level).
	#[test]
	fn a_lower_detail_picks_the_same_footprint() {
		for src in [(1280u32, 720u32), (960, 544), (640, 400)] {
			let full_top = 3.0;
			for d in [Detail::High, Detail::Medium, Detail::Low] {
				let ((w, _), m) = planar_size(src, 4, d);
				let (top, b) = ((m - 1) as f32, d.lod_bias());
				let mut k = 0.0f32;
				while k <= 5.0 {
					for extra in [0.0, 0.7, 2.4] {
						let lf = shader_lod(k, extra, full_top, 0.0);
						let ld = shader_lod(k, extra, top, b);
						let foot_full = lf.exp2();
						let foot = ld.exp2() * src.0 as f32 / w as f32;
						if lf >= b {
							assert!((foot / foot_full - 1.0).abs() < 0.02, "{src:?} {d:?} r {k} extra {extra}: {foot} vs {foot_full}");
						} else {
							assert_eq!(ld, 0.0, "{d:?}: under the bias the sharpest level");
						}
					}
					k += 0.125;
				}
			}
		}
		// High (bias 1.0) is the lane's first Low exactly: clamp(l0, 0, top + 1) and clamp(.. - 1, 0, top).
		for r in [0.0f32, 0.5, 1.7, 2.9, 3.5, 9.0] {
			let old = (r.clamp(0.0, 2.0 + 1.0) + 0.3 - 1.0).clamp(0.0, 2.0);
			assert_eq!(shader_lod(r, 0.3, 2.0, 1.0).to_bits(), old.to_bits());
		}
	}

	#[test]
	fn set_saves_once_and_round_trips() {
		let _guard = crate::pcopts::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		let mut opts = PcOptions::new();
		init(&opts, Detail::High);
		assert_eq!(get(), Detail::High);
		assert!(!set_from_ui(&mut opts, Detail::High), "already the default: nothing written");
		assert_eq!(opts.extra_get(KEY), None);
		assert!(set_from_ui(&mut opts, Detail::Medium));
		assert_eq!(get(), Detail::Medium);
		assert!(!set_from_ui(&mut opts, Detail::Medium));
		let saved = opts.to_text();
		assert!(saved.contains("metal_detail=medium\n"), "{saved}");
		init(&PcOptions::new(), Detail::High);
		assert_eq!(get(), Detail::High);
		init(&PcOptions::parse(&saved), Detail::High);
		assert_eq!(get(), Detail::Medium);
		assert!(set_from_ui(&mut opts, Detail::Low));
		assert_eq!(PcOptions::parse(&opts.to_text()).extra_get(KEY), Some("low"));
		init(&PcOptions::new(), ENGINE_DEFAULT);
		assert_eq!(get(), Detail::Full);
	}
}
