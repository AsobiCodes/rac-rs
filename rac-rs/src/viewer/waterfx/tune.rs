//! [PC] (speed, lossy, dev) `RAC_TUNE` switches for the size of the water's helper targets. The defaults (2, 2) are today's picture
//! exactly; a larger divisor draws that target smaller (the water bed or the planar mirror): cheaper, softer. Read once.

use std::sync::OnceLock;

fn div(name: &str, key: &'static OnceLock<u32>) -> u32 {
	*key.get_or_init(|| (crate::devtune::f32(name, 2.0).round() as u32).clamp(2, 8))
}

/// `RAC_TUNE=bed_div=4`: the water bed's size divisor (default 2, half of the window); the shore map keeps its sample spacing
/// (`shore_stride`).
pub(super) fn bed_div() -> u32 {
	static V: OnceLock<u32> = OnceLock::new();
	div("bed_div", &V)
}

/// `RAC_TUNE=mirror_div=4`: the planar mirrors' size divisor (default 2).
pub(super) fn mirror_div() -> u32 {
	static V: OnceLock<u32> = OnceLock::new();
	div("mirror_div", &V)
}

/// The shore map's pixel stride over the bed's depth: `SHORE_STRIDE` at the default half size, 1 on a quarter-size bed (the same
/// spacing in window pixels).
pub(super) fn shore_stride() -> u32 {
	(super::shore_map::SHORE_STRIDE * 2 / bed_div()).max(1)
}
