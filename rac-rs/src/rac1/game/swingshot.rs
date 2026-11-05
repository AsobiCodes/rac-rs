//! ARI-47c / BRIDGE-76: the held Swingshot (item 12, held class 208), its hook (class 209) and the rope ribbon, plus the
//! ARI-47e reticles (decomp/notes/aridia_gadgets.md "ARI-47c port spec", "ARI-47e port spec"; port_bridges.md
//! BRIDGE-76). Held updater 0x2c5498, hook creation 0x2c62c8, hook updater 0x2c63b0, rope draw callback 0x2c5108.
//! The grapple launch and states 2-7 (hero 0x24-0x26) are the ARI-46 steps of game/grapple.rs, called from here.

use super::*;
use crate::anim::PropAnim;
use crate::particles::WorldQuad;
use crate::physics::cross;

pub(super) const HELD: u32 = super::held::SWINGSHOT_HELD;
pub(super) const HOOK: u32 = super::held::SWINGSHOT_HOOK;

/// Rope ribbon: segment length, half width, side reference drop, fx texture.
const SEG: f32 = 0.2;
const HALF_W: f32 = 0.05;
const SIDE_DROP: f32 = 0.1;
const ROPE_TEX: usize = 0xf;

/// The held Swingshot's pvars: updater state, ripple phase (+0x18), wavelength (+0x1c), amplitude (+0x20), the hook
/// moby (+0x14), `fresh` (hook created this tick: its own update starts next tick) and `rope` (the draw callback was
/// registered this tick).
#[derive(Default)]
pub(super) struct Held208 {
	pub phase: f32,
	pub wave: f32,
	pub amp: f32,
	pub hook: Option<SwingHook>,
	pub rope: bool,
}

/// The hook moby (class 209): position, rows copied from the held moby (None until the viewer posed it), speed per
/// tick (pvar +4), its own state and clock.
pub(super) struct SwingHook {
	pub pos: V3,
	pub rows: Option<[V3; 3]>,
	pub anim: PropAnim,
}

/// BRIDGE-76: what the viewer needs to draw the hook model.
pub struct SwingRopeDraw<'a> {
	pub hook: V3,
	pub hook_rows: Option<[V3; 3]>,
	pub hook_anim: &'a PropAnim,
	pub rope: bool,
	pub phase: f32,
	pub wave: f32,
	pub amp: f32,
}

/// FUN_00210d10: fold an angle into [-pi, pi) by repeated pi steps.
fn wrap(mut a: f32) -> f32 {
	const P: f32 = 3.1415927;
	while P <= a {
		a = a - P - P;
	}
	while a < -P {
		a = a + P + P;
	}
	a
}




impl Game {


	/// BRIDGE-76: the hook model's draw data while the Swingshot rig holds a hook.
	pub fn swing_rope(&self) -> Option<SwingRopeDraw<'_>> {
		let h = self.held.as_ref().filter(|h| h.class == HELD)?;
		let s = &h.swing;
		let k = s.hook.as_ref()?;
		Some(SwingRopeDraw { hook: k.pos, hook_rows: k.rows, hook_anim: &k.anim, rope: s.rope, phase: s.phase, wave: s.wave, amp: s.amp })
	}

	/// BRIDGE-76 (FUN_002c5108): the rope ribbon from the hook to the held joint 0, camera-facing, rippled.
	pub fn swing_rope_quads(&self, cam: V3) -> Vec<WorldQuad> {
		let mut out = Vec::new();
		let Some(d) = self.swing_rope().filter(|d| d.rope) else { return out };
		let jt = self.held_muzzle_point();
		let k = d.hook;
		let mut v = sub(jt, k);
		let l = length(v);
		if l <= 0.0 {
			return out;
		}
		let side = cross(v, [v[0], v[1], v[2] - SIDE_DROP]);
		let c5 = crate::camera::fast_cos(wrap(d.phase));
		let mut p = k;
		let (mut a, mut b) = ([k[0], k[1], k[2] - HALF_W], [k[0], k[1], k[2] + HALF_W]);
		let mut t = 0.0f32;
		loop {
			let seg = SEG.min(l - t);
			t += seg;
			v = aridia::set_len(v, seg);
			p = add(p, v);
			let e = aridia::set_len(cross(sub(cam, p), v), HALF_W);
			let pi = std::f32::consts::PI;
			let amt = crate::camera::fast_sin(t * pi / l) * crate::camera::fast_sin(wrap(t * pi / d.wave)) * c5 * d.amp;
			let off = aridia::set_len(side, amt);
			let a2 = add(add(p, e), off);
			let b2 = add(sub(p, e), off);
			// Strip (prev a, a2, prev b, b2) as the fan [v0, v1, v3, v2].
			out.push(WorldQuad { pos: [a, a2, b2, b], uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], colour: [[1.0; 4]; 4], tex: ROPE_TEX, additive: false });
			(a, b) = (a2, b2);
			if l <= t {
				break;
			}
		}
		out
	}
}


