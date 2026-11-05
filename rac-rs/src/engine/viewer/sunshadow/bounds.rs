//! [PC] Sun shadow pass (lane v042_shadow): tight world boxes of the tie and shrub casters, for the per-cascade cull.
//!
//! The instance spheres of `TieCull` / `ShrubCull` are the class's sphere moved to the placement: round, so they reach far
//! past a long flat object and let it into cascades it never touches. Here every tie (per LOD) and shrub gets the axis box
//! of its own vertices once, at scene build; a tie's box takes both ends of every geomorph (`ties::morphed`: base + q * delta
//! with q in 0..=1, see `tie_pick` / `fardraw::tie`), so it holds the vertices at any Q. `Cascade::reaches` is the separating
//! axis test along the light's three axes (the cascade is exactly its light-space box), used AFTER the sphere test: a caster
//! the box rejects lies wholly outside the map's x / y range or its depth range, where it would have been clipped.

use super::Cascade;
use std::ops::Range;

/// An axis box in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::viewer) struct Aabb {
	pub lo: [f32; 3],
	pub hi: [f32; 3],
}

/// The boxes of a scene's casters: per tie instance its three LODs, per shrub instance its mesh. None = no proven box (the
/// sphere test alone decides).
#[derive(Default)]
pub(in crate::viewer) struct Boxes {
	pub ties: Vec<[Option<Aabb>; 3]>,
	pub shrubs: Vec<Option<Aabb>>,
}

impl Aabb {
	/// The box of finite points; None when there are none or one is not finite.
	fn of(points: impl Iterator<Item = [f32; 3]>) -> Option<Aabb> {
		let (mut lo, mut hi, mut n) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3], 0usize);
		for p in points {
			for k in 0..3 {
				if !p[k].is_finite() {
					return None;
				}
				lo[k] = lo[k].min(p[k]);
				hi[k] = hi[k].max(p[k]);
			}
			n += 1;
		}
		(n > 0).then_some(Aabb { lo, hi })
	}
}

/// The box of every tie LOD: `base` the unmorphed vertices, `morph` each vertex's delta (parallel), `vranges[i][lod]` the
/// vertex range of instance `i` at that LOD.
pub(in crate::viewer) fn tie_boxes(base: &[[f32; 3]], morph: &[[f32; 3]], vranges: &[[Range<usize>; 3]]) -> Vec<[Option<Aabb>; 3]> {
	if base.len() != morph.len() {
		return vec![[None; 3]; vranges.len()];
	}
	vranges
		.iter()
		.map(|r| {
			std::array::from_fn(|lod| {
				let (b, m) = (base.get(r[lod].clone())?, morph.get(r[lod].clone())?);
				Aabb::of(b.iter().zip(m).flat_map(|(b, m)| [*b, std::array::from_fn(|k| b[k] + m[k])]))
			})
		})
		.collect()
}

/// The box of every shrub mesh (static world positions, `ranges[i].1` the vertex range of instance `i`); None for an instance whose
/// range runs past the vertex list (no proven box: the sphere test alone decides, as `tie_boxes` does).
pub(in crate::viewer) fn shrub_boxes(pos: impl Fn(usize) -> Option<[f32; 3]>, ranges: &[(Range<usize>, Range<usize>)]) -> Vec<Option<Aabb>> {
	ranges.iter().map(|r| r.1.clone().map(&pos).collect::<Option<Vec<_>>>().and_then(|v| Aabb::of(v.into_iter()))).collect()
}

impl Cascade {
	/// Whether a box can cast into the cascade: the interval test along the light's right, up and travel axes (the box's
	/// extent along an axis is the sum of its half sizes weighted by the axis), with the slack of the props test.
	pub(super) fn reaches(&self, b: &Aabb) -> bool {
		let c = [0, 1, 2].map(|k| 0.5 * (b.lo[k] + b.hi[k]));
		let h = [0, 1, 2].map(|k| 0.5 * (b.hi[k] - b.lo[k]));
		let ext = |a: [f32; 3]| (a[0].abs() * h[0] + a[1].abs() * h[1] + a[2].abs() * h[2]) * 1.001 + 0.01;
		let dot = |a: [f32; 3]| a[0] * c[0] + a[1] * c[1] + a[2] * c[2];
		let [right, up, d] = self.basis;
		let (er, eu, ed) = (ext(right), ext(up), ext(d));
		(dot(right) - self.c[0]).abs() <= self.r + er && (dot(up) - self.c[1]).abs() <= self.r + eu && {
			let z = dot(d);
			z >= self.z[0] - ed && z <= self.z[1] + ed
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn cascade() -> Cascade {
		Cascade::fit([0.0; 3], [0.0, 1.0, 0.0], 0.6, 16.0 / 9.0, 0.03, 12.0, [0.3, -0.5, -0.8], 2048)
	}

	#[test]
	fn the_box_holds_both_ends_of_the_morph() {
		let b = tie_boxes(&[[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]], &[[0.0; 3], [2.0, -3.0, 0.5]], &[[0..2, 0..0, 1..2]]);
		assert_eq!(b[0][0], Some(Aabb { lo: [0.0, -2.0, 0.0], hi: [3.0, 1.0, 1.5] }));
		assert_eq!(b[0][1], None, "an empty LOD has no box");
		assert_eq!(b[0][2], Some(Aabb { lo: [1.0, -2.0, 1.0], hi: [3.0, 1.0, 1.5] }));
		assert_eq!(tie_boxes(&[[f32::NAN; 3]], &[[0.0; 3]], &[[0..1, 0..1, 0..1]])[0], [None; 3]);
	}

	#[test]
	fn a_shrub_range_past_the_vertex_list_has_no_box() {
		let verts = [[0.0f32, 0.0, 0.0], [1.0, 2.0, 3.0]];
		let pos = |i: usize| verts.get(i).copied();
		let b = shrub_boxes(pos, &[(0..0, 0..2), (0..0, 1..3), (0..0, 5..6), (0..0, 0..0)]);
		assert_eq!(b[0], Some(Aabb { lo: [0.0; 3], hi: [1.0, 2.0, 3.0] }));
		assert_eq!(b[1], None, "a range that runs past the list: no prefix box");
		assert_eq!(b[2], None);
		assert_eq!(b[3], None, "an empty range has no box");
	}

	#[test]
	fn a_box_inside_the_sphere_test_still_reaches_when_it_overlaps_the_map() {
		let c = cascade();
		let near = Aabb { lo: [-1.0, 4.0, -1.0], hi: [1.0, 6.0, 1.0] };
		assert!(c.reaches(&near));
		// Far away along the light's right axis: no overlap.
		let [right, _, _] = c.basis;
		let off = 3.0 * c.r;
		let far = Aabb { lo: [right[0] * off - 1.0, right[1] * off - 1.0, right[2] * off - 1.0], hi: [right[0] * off + 1.0, right[1] * off + 1.0, right[2] * off + 1.0] };
		assert!(!c.reaches(&far));
	}

	#[test]
	fn the_box_test_never_rejects_what_the_sphere_of_its_corners_touches() {
		// A box rejected by `reaches` has all its corners (and so the sphere around them) outside the map.
		let c = cascade();
		let mut rng = 12345u32;
		let mut next = || {
			rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
			(rng >> 8) as f32 / (1 << 24) as f32
		};
		for _ in 0..4000 {
			let lo = [next() * 80.0 - 40.0, next() * 80.0 - 40.0, next() * 80.0 - 40.0];
			let hi = [lo[0] + next() * 6.0, lo[1] + next() * 6.0, lo[2] + next() * 6.0];
			let b = Aabb { lo, hi };
			if c.reaches(&b) {
				continue;
			}
			for k in 0..8 {
				let p = [if k & 1 == 0 { lo[0] } else { hi[0] }, if k & 2 == 0 { lo[1] } else { hi[1] }, if k & 4 == 0 { lo[2] } else { hi[2] }];
				assert!(!c.touches(p, 0.0), "corner {p:?} of a rejected box is inside the cascade");
			}
		}
	}
}
