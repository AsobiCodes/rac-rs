//! [PC] Enhanced: shrub textures whose alpha is a soft smear, not a cut-out (2026-10-24, Hoven's snow footprints). Split view: `crisp_leaf` runs only under `post_on` = `splitview::fx`.
//!
//! `crisp_leaf` (viewer.rs) cuts a magnified leaf's alpha ramp at half alpha, which suits a cut-out leaf (alpha almost only 0 or 1
//! with a thin rim). A soft texture (snow footprints, smoke, mist, a speckled smudge: a large share of its visible texels at
//! partial alpha) lost its softness that way: Hoven's footprint smudge turned into hard black blotches that shimmered with every
//! camera move. Such a texture's vertices carry the class tag mark `SOFT_MARK` (like the ice mark, `icefx::retag`) and the shader
//! leaves their alpha as the GS samples it. Classic ignores the mark (it never calls `crisp_leaf`).

use crate::level::Texture;
use std::collections::HashMap;

/// How far the soft mark moves a shrub's class tag (-2 to -2.125: inside the fog class range, outside the ice window).
pub(in crate::viewer) const SOFT_MARK: f32 = 0.125;

/// Share of partial-alpha texels (0.2 < a < 0.8, alpha 1 = 128) among the visible ones (a > 0.05) from which a texture is soft.
/// Measured over the whole game's shrub textures: cut-out leaves 0.00..0.34 (most < 0.2), the Hoven footprint smudge 0.54,
/// smooth sprites (mist, smoke) 0.46..0.84.
const SOFT_SHARE: f32 = 0.45;

/// Whether the texture's alpha is a soft smear (`SOFT_SHARE`).
pub(in crate::viewer) fn is_soft(t: &Texture) -> bool {
	let (mut vis, mut mid) = (0usize, 0usize);
	for p in &t.rgba {
		let a = p[3] as f32 / 128.0;
		vis += (a > 0.05) as usize;
		mid += (a > 0.2 && a < 0.8) as usize;
	}
	vis > 0 && mid as f32 >= SOFT_SHARE * vis as f32
}

/// The soft ones among the bare texture keys of a shrub batch (`vertex_keys`: u32::MAX = none), sorted and unique.
pub(in crate::viewer) fn soft_keys(vkeys: &[u32], textures: &HashMap<u32, Texture>) -> Vec<u32> {
	let mut keys: Vec<u32> = vkeys.iter().copied().filter(|&k| k != u32::MAX).collect();
	keys.sort_unstable();
	keys.dedup();
	keys.retain(|k| textures.get(k).is_some_and(is_soft));
	keys
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tex(alpha: &[u8]) -> Texture {
		Texture { w: alpha.len(), h: 1, rgba: alpha.iter().map(|&a| [10, 20, 30, a]).collect() }
	}

	/// A cut-out leaf (solid with a thin rim) is not soft; a smudge whose texels sit at partial alpha is; an empty one is not.
	#[test]
	fn cutout_is_not_soft_and_smear_is() {
		let mut leaf = vec![128u8; 90];
		leaf.extend([40u8; 4]);
		leaf.extend([0u8; 20]);
		assert!(!is_soft(&tex(&leaf)));
		let mut smear = vec![60u8; 60];
		smear.extend([128u8; 30]);
		assert!(is_soft(&tex(&smear)));
		assert!(!is_soft(&tex(&[0, 0, 0])));
	}

	#[test]
	fn keys_pick_only_the_soft_textures() {
		let mut m = HashMap::new();
		m.insert(1u32, tex(&[60; 20]));
		m.insert(2u32, tex(&[128; 20]));
		assert_eq!(soft_keys(&[2, 1, u32::MAX, 1, 3], &m), vec![1]);
	}
}
