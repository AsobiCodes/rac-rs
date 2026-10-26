//! Vegetation (viewer/vegfx.rs): what may and may not grow plants, apart from the terrain's own grass test.
//!
//! - Grass blocks (`tie_blocks`): on the swamp levels a tie is a raised grass bed only when it stands at least `BLOCK_MIN_H` tall. Retail gives a
//!   block's top no floor of its own, so a block skips the floor test; a thin slab is no bed. Umbris' pump room holds seven
//!   62 x 62 m slabs 1.2 m thick (ties 1116..1122, one per water level) textured all over with the pale-green water texture;
//!   as "blocks" they grew bright grass on the water and on the dark metal floor beside it (the YouTuber's picture, v0.3.1).
//! - Dark swamp ground (`Cover::lit_green`): the baked shade of a swamp level is so dark that the ground's green no longer
//!   passes the grass test as drawn; the shade is lifted to `SWAMP_LUM` for the green test only, and the swamp plants are
//!   sparser (`SWAMP_DENSITY`) and carry no flower heads.

use super::*;

/// The least height (m) of a grass block: Eudora's lowest raised bed is 3.4 m, Umbris' water slabs are 1.2-1.3 m.
const BLOCK_MIN_H: f32 = 2.0;
/// The dark swamp levels (Umbris).
const SWAMP_LEVELS: [usize; 1] = [7];
/// The luminance the shade is lifted to for the green test, and the most it is lifted by.
const SWAMP_LUM: f32 = 0.8;
const SWAMP_LIFT_MAX: f32 = 4.0;
/// The blade density of a swamp level against the Vegetation row's.
const SWAMP_DENSITY: f32 = 0.8;
/// The smallest ground texture (side, texels) a swamp's plants grow on (the 32 x 32 ones are effects).
const GROUND_MIN: usize = 64;
/// A grass-textured tie top (a water slab) from this far below (m) to this far above a ground point puts it under or on the water.
const SLAB_MIN: f32 = -2.5;
const SLAB_REACH: f32 = 40.0;
/// How far (m) the water of a slab reaches past its footprint.
const SLAB_MARGIN: f32 = 40.0;
/// The mark of a thin grass tie (a water slab) among the ids of `Cover::blocks`.
const SLAB_FLAG: u32 = 1 << 31;
/// The patch noise (`patch_noise`, 0..1) the tufts start growing at, and the rise over which they thicken to full.
const TUFT_FROM: f32 = 0.38;
const TUFT_RISE: f32 = 0.17;
/// The tuft blade colour (unit luminance 0.446) at the luminance of the ground under it, and the most it may brighten.
const TUFT_COL: [f32; 3] = [0.30, 0.46, 0.14];
const TUFT_BOOST: f32 = 0.6;
/// The least baked shade luminance of the ground a tuft grows on (the ground drowned in the dark of the pump room is not lit).
const TUFT_LIT: f32 = 0.3;

pub(super) fn is_swamp(level: usize) -> bool {
	SWAMP_LEVELS.contains(&level)
}

/// The share of the opaque texels of `t` that are `leafy` (moss and the like).
pub(super) fn moss_share(t: &Texture) -> f32 {
	let (mut n, mut m) = (0usize, 0usize);
	for p in t.rgba.iter().filter(|p| p[3] >= 128) {
		n += 1;
		m += leafy([p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32]) as usize;
	}
	m as f32 / n.max(1) as f32
}

/// Whether the ground texture `t` of level `level` is tried for swamp plants without being a grass texture: any ground texture of a swamp
/// level. The baked shade of the dark swamp tints the textures (a blue-grey one is drawn olive), so the texture alone says nothing;
/// the per-texel test of the ground as drawn (`Cover::grows`) decides.
pub(super) fn mossy(level: usize, t: &Texture) -> bool {
	is_swamp(level) && t.w >= GROUND_MIN && t.h >= GROUND_MIN
}

/// Earthy ground (bytes): olive, khaki and sandy browns, where green is not far under red and clearly over blue. Not the bluish
/// metal and rock, not the orange of lava and glow, not the red-brown of dirt.
fn soil(c: [f32; 4]) -> bool {
	let (r, g, b) = (c[0], c[1], c[2]);
	g >= 12.0 && g >= r * 0.80 && g >= b * 1.15 && g - b >= 8.0
}

/// How much of full density a grass tuft has where the patch noise `nz` (0..1) is: none below `TUFT_FROM`, all `TUFT_RISE` above it.
fn tuft_cover(nz: f32) -> f32 {
	((nz - TUFT_FROM) / TUFT_RISE).clamp(0.0, 1.0)
}

/// Whether a tie of this height is tall enough to be a raised grass block. Only the swamp levels ask for a height: elsewhere every
/// tie with a flat top counts, as in retail (Novalis' 1.2-2 m hedges and rocks carry their own plants).
fn tall_enough(level: usize, zlo: f32, zhi: f32) -> bool {
	!is_swamp(level) || zhi - zlo >= BLOCK_MIN_H
}

/// The factor the shade is lifted by for the green test: 1 for a swamp's light ground and above, up to `SWAMP_LIFT_MAX`.
fn lift(shade_lum: f32) -> f32 {
	(SWAMP_LUM / shade_lum.max(1e-3)).clamp(1.0, SWAMP_LIFT_MAX)
}

impl Cover<'_> {
	/// `seen` with the shade lifted (swamp levels).
	fn lifted(seen: [f32; 4], shade: [f32; 3]) -> [f32; 4] {
		let k = lift(lum(shade));
		[(seen[0] * k).min(255.0), (seen[1] * k).min(255.0), (seen[2] * k).min(255.0), seen[3]]
	}

	/// Whether the ground drawn as `seen` (texel x baked `shade`) is grass: `green`; on a swamp level the shade is lifted first
	/// and the test is the dark-tolerant `leafy` or earthy `soil` (the dirt margin and the cover tests use this one).
	pub(super) fn lit_green(&self, seen: [f32; 4], shade: [f32; 3]) -> bool {
		if !self.swamp {
			return green(seen);
		}
		let c = Self::lifted(seen, shade);
		leafy(c) || soil(c)
	}

	/// Whether a blade clump may grow at `p` (`r` its random 0..1): `lit_green`; on a swamp level the mossy ground always, the earthy
	/// ground only in the noise patches (a few metres across, thicker towards their middle) so the tufts come with bare gaps.
	pub(super) fn grows(&self, seen: [f32; 4], shade: [f32; 3], p: [f32; 3], r: f32) -> bool {
		if !self.swamp {
			return green(seen);
		}
		let c = Self::lifted(seen, shade);
		lum(shade) >= TUFT_LIT && (leafy(c) || (soil(c) && r < tuft_cover(patch_noise(p[0] + 313.0, p[1] - 71.0))))
	}

	/// The blade colour (`col`: ground texel x shade, 0..1) of a clump: unchanged, except on a swamp level over earthy ground, where
	/// the blades are dark olive green at the ground's brightness.
	pub(super) fn tint(&self, seen: [f32; 4], shade: [f32; 3], col: [f32; 3]) -> [f32; 3] {
		if !self.swamp || leafy(Self::lifted(seen, shade)) {
			return col;
		}
		let k = (lum(col) * TUFT_BOOST / lum(TUFT_COL)).min(1.0 / TUFT_COL[1]);
		TUFT_COL.map(|x| x * k)
	}

	/// Whether the point lies under or at the water of a water slab (the thin grass-textured ties of Umbris' pump room, marked with
	/// `SLAB_FLAG`): within `SLAB_MARGIN` m of the slab's footprint (the drawn water reaches past the slab) and from `SLAB_MIN` below to
	/// `SLAB_REACH` m under its top. Swamp levels only; the slabs have no collision, so the floor test cannot see the water.
	pub(super) fn under_slab(&self, p: [f32; 3]) -> bool {
		self.swamp
			&& self.blocks.iter().filter(|&&ii| ii & SLAB_FLAG != 0).any(|&ii| {
				let m = &self.ties[(ii & !SLAB_FLAG) as usize].lods[0];
				let (lo, hi) = m.positions.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), q| (std::array::from_fn(|k| lo[k].min(q[k])), std::array::from_fn(|k| hi[k].max(q[k]))));
				let dz = hi[2] - p[2];
				(0..2).all(|k| p[k] > lo[k] - SLAB_MARGIN && p[k] < hi[k] + SLAB_MARGIN) && dz > SLAB_MIN && dz < SLAB_REACH
			})
	}

	/// The blade density factor of the level (`SWAMP_DENSITY` on a swamp, else 1).
	pub(super) fn density(&self) -> f32 {
		if self.swamp { SWAMP_DENSITY } else { 1.0 }
	}
}

/// The grass blocks among the ties (`Cover::blocks`): ties (on the swamp levels: at least `BLOCK_MIN_H` tall) with at least `BLOCK_TOP` m2 of flat
/// grass faces (a bush's rounded leaves have next to none). On the swamp levels the thin ones (the water slabs) are listed too,
/// their id marked with `SLAB_FLAG` (`Cover::under_slab`).
pub(super) fn tie_blocks(level: usize, tie_set: &[ties::LitTie], ttex: &HashMap<u32, &Texture>) -> std::collections::HashSet<u32> {
	let mut out = std::collections::HashSet::new();
	for ii in 0..tie_set.len() as u32 {
		let m = &tie_set[ii as usize].lods[0];
		let (zlo, zhi) = m.positions.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p[2]), hi.max(p[2])));
		let flat: f32 = m
			.faces
			.iter()
			.filter(|(key, _)| ttex.contains_key(&(key & crate::mobies::KEY_MASK)))
			.map(|&(_, f)| {
				let [a, b, c] = f.map(|i| m.positions[i as usize]);
				let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
				let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
				let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
				let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
				if len > 1e-6 && n[2] / len >= BLOCK_FLAT { len * 0.5 } else { 0.0 }
			})
			.sum();
		if flat >= BLOCK_TOP {
			out.insert(if tall_enough(level, zlo, zhi) { ii } else { ii | SLAB_FLAG });
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn slabs_are_no_blocks() {
		// Umbris' water slabs (1.2 and 1.3 m) are out, Eudora's lowest bed (3.4 m) and a tall block are in.
		assert!(!tall_enough(7, 72.0, 73.2));
		assert!(!tall_enough(7, 131.0, 132.3));
		assert!(tall_enough(7, 54.8, 58.2));
		// Off the swamp levels a thin tie still counts (Novalis' 1.2 m hedges).
		assert!(tall_enough(1, 82.8, 84.0));
		assert!(tall_enough(7, 30.1, 45.5));
	}

	#[test]
	fn swamp_shade_lift() {
		// A bright ground is not lifted; a dark one is, up to the cap.
		assert_eq!(lift(1.0), 1.0);
		assert!((lift(0.4) - 2.0).abs() < 1e-5);
		assert_eq!(lift(0.05), SWAMP_LIFT_MAX);
		assert_eq!(lift(0.0), SWAMP_LIFT_MAX);
	}

	#[test]
	fn soil_is_olive_not_metal() {
		assert!(soil([93.0, 81.0, 58.0, 255.0]));
		assert!(soil([106.0, 89.0, 72.0, 255.0]));
		assert!(!soil([56.0, 63.0, 81.0, 255.0])); // bluish metal
		assert!(!soil([66.0, 65.0, 61.0, 255.0])); // grey rock
		assert!(!soil([122.0, 75.0, 0.0, 255.0])); // orange glow
		assert!(!soil([82.0, 58.0, 37.0, 255.0])); // red-brown dirt
	}

	#[test]
	fn tufts_come_in_patches() {
		assert_eq!(tuft_cover(0.1), 0.0);
		assert_eq!(tuft_cover(TUFT_FROM), 0.0);
		assert_eq!(tuft_cover(1.0), 1.0);
		assert!((tuft_cover(TUFT_FROM + TUFT_RISE * 0.5) - 0.5).abs() < 1e-5);
	}

	#[test]
	fn only_umbris_is_swamp() {
		assert!(is_swamp(7));
		assert!(!is_swamp(1) && !is_swamp(4) && !is_swamp(11));
	}
}
