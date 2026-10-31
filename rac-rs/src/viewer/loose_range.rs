//! The loose wrenches inside the hero marker (the thrown one and its two afterimages) and the light they take.
//!
//! They are extra vertex ranges of the hero marker (handler/frame_hero.rs), but retail lights them as the class-71 mobys they are:
//! each from its own +0x3c ambient bytes and +0x38 light word plus the third light at its own position (`Gpu::moby_light` and `dyn_id`,
//! exactly as the props path does, props_sync.rs / props_set.rs), not from the light at Ratchet's feet. Next to Blarg's purple lights
//! a thrown wrench stays grey. `Gpu::set_marker` turns each range into a `marker_ctx::Own`.

use crate::retail::LooseWrench;

/// Loose ranges of one frame: the thrown wrench first, then its afterimages (the retail draw has no more than three).
pub(in crate::viewer) const MAX: usize = 3;

/// One loose wrench's vertices `[a, b)` in the marker, where it stands in the world and its own light.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(in crate::viewer) struct LooseRange {
	pub a: usize,
	pub b: usize,
	pub pos: [f32; 3],
	pub light: Option<([u8; 3], u32)>,
}

impl LooseRange {
	/// The range `[a, b)` of retail's loose wrench `l`.
	pub fn of(l: &LooseWrench, a: usize, b: usize) -> LooseRange {
		LooseRange { a, b, pos: l.pos, light: l.light }
	}

	/// The first `MAX` non-empty ranges, in order (what `Gpu::marker_loose` carries); none = all `None`.
	pub fn first(ranges: &[LooseRange]) -> [Option<LooseRange>; MAX] {
		let mut out = [None; MAX];
		for (o, r) in out.iter_mut().zip(ranges.iter().filter(|r| r.a < r.b)) {
			*o = Some(*r);
		}
		out
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn r(a: usize, b: usize, x: f32) -> LooseRange {
		LooseRange { a, b, pos: [x, 0.0, 0.0], light: Some(([64; 3], 0)) }
	}

	#[test]
	fn first_keeps_the_order_and_skips_empty_ranges() {
		assert_eq!(LooseRange::first(&[]), [None; MAX]);
		assert_eq!(LooseRange::first(&[r(5, 5, 0.0)]), [None; MAX]);
		let got = LooseRange::first(&[r(10, 20, 1.0), r(7, 7, 2.0), r(30, 40, 3.0), r(50, 60, 4.0)]);
		assert_eq!(got, [Some(r(10, 20, 1.0)), Some(r(30, 40, 3.0)), Some(r(50, 60, 4.0))]);
		// More than MAX: the first three non-empty ones stay.
		let more = LooseRange::first(&[r(1, 2, 1.0), r(3, 4, 2.0), r(5, 6, 3.0), r(7, 8, 4.0)]);
		assert_eq!(more[2], Some(r(5, 6, 3.0)));
	}
}
