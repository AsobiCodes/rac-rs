//! [PC] Enhanced ice (user request 2026-10-16): Hoven's ice floes, ice blocks and icicles get a cold gloss that follows the camera,
//! a fresnel rim, light through their thin edges and small sparkles (`ice_look` in the world shader, viewer.rs). No retail
//! colour changes: the look only adds light on top of the retail texel x baked colour.
//!
//! The ice is picked by texture: at load every tie and shrub vertex remembers the texture key of a face that uses it
//! (`vertex_keys`); once the level is known, `retag` marks the vertices on the level's ice textures (`ice_keys`) by moving their
//! class tag (vertex light w) from -3 / -2 to -3.25 / -2.25, inside every range the shaders test. The look runs only while the
//! gate (`ICE_GATE`) is Enhanced (`flag`, the uniform eye w).

use crate::mesh::Mesh;

/// Which Graphics row turns the ice on (`ICE_GATE`; Water = only while Water is Enhanced too).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Gate {
	Lighting,
	Water,
}

pub(super) const ICE_GATE: Gate = Gate::Lighting;

/// The look (`ice_look` in viewer.rs): 1 = a cold gloss that follows the camera and a fresnel rim, 2 = also light through
/// the thin edges and sparkles, 3 = all of it stronger.
pub(super) const ICE_LOOK: f32 = 2.0;

/// How far the ice mark moves the class tag (tie -3, shrub -2).
pub(super) const MARK: f32 = 0.25;

/// The bare texture key of a face key, as `Gpu::batch_instances` binds it (tie / shrub material bits dropped).
fn bare(key: u32) -> u32 {
	if crate::shrubs::key_k(key).is_some() { key & crate::mobies::KEY_MASK & !crate::shrubs::MATERIAL_BITS } else { key & crate::mobies::KEY_MASK }
}

/// Per vertex of a batch (`n` vertices, each mesh starting at its offset), the bare texture key of a face using it (u32::MAX: none).
pub(super) fn vertex_keys<'a>(meshes: impl Iterator<Item = (&'a Mesh, usize)>, n: usize) -> Vec<u32> {
	let mut keys = vec![u32::MAX; n];
	for (m, at) in meshes {
		for &(key, f) in &m.faces {
			for v in f {
				if let Some(k) = keys.get_mut(at + v as usize) {
					*k = bare(key);
				}
			}
		}
	}
	keys
}

/// The level's ice textures: (tie keys, shrub keys). Hoven (level 12) only.
pub(super) fn ice_keys(level: usize) -> (&'static [u32], &'static [u32]) {
	match level {
		12 => (HOVEN_TIES, HOVEN_SHRUBS),
		_ => (&[], &[]),
	}
}

/// Hoven's ice [C: debug runs drawing one key at a time in magenta on play/compare/Hoven.racpad fields 200 / 500 / 1950, 2026-10-16]: tie
/// texture 0x2f the ice floes and crystals standing in the abyss and by the village water, 0x31 the small ice pieces at the
/// village edge; shrub texture 8 the icicles under the roofs. Not ice: tie 0x28 (the platforms' light pillars), shrub 0x11
/// (snow-capped rocks).
const HOVEN_TIES: &[u32] = &[0x2002f, 0x20031];
const HOVEN_SHRUBS: &[u32] = &[0x30008];

/// Marks the vertices of class `base` (-3 tie, -2 shrub) whose key is in `ice` and clears the rest; true when any changed.
pub(super) fn retag(verts: &mut [super::Vertex], keys: &[u32], ice: &[u32], base: f32) -> bool {
	let mut changed = false;
	for (v, k) in verts.iter_mut().zip(keys) {
		let w = v.light[3];
		if (w - base).abs() > 0.4 {
			continue;
		}
		let want = if ice.contains(k) { base - MARK } else { base };
		if w != want {
			v.light[3] = want;
			changed = true;
		}
	}
	changed
}

/// The uniform eye w: 1 off, 1 + `ICE_LOOK` the ice look.
pub(super) fn flag(level: usize) -> f32 {
	let (t, s) = ice_keys(level);
	if t.is_empty() && s.is_empty() {
		return 1.0;
	}
	let g = crate::retail::pcext::graphics();
	let on = match ICE_GATE {
		Gate::Lighting => g.enhanced(),
		Gate::Water => g.enhanced() && g.water,
	};
	if on { 1.0 + ICE_LOOK } else { 1.0 }
}
