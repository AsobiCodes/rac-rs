use super::*;

impl Game {












	/// The held bomb's grey pulse (OV:127434-127445), as the moby colour scale (0x80 = 1).
	pub fn bomb_held_tint(&self) -> Option<[f32; 3]> {
		let c = self.bomb_hold;
		if !self.bomb_in_hand() {
			return None;
		}
		let a = (c as f32 * 0.068 + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
		let grey = (96.0 * crate::camera::fast_sin(a)) as i32 + 0x9f;
		Some([grey as f32 / 128.0; 3])
	}






}





/// REN-3: the marker spin `(frames % 150) * 2 pi / (180 * 0.8333333)` (gp 0x15ed68 is the PAL 0.8333333, so the
/// divisor is 150), wrapped by FUN_00221bb8 (`pi <= x` takes 2 pi off, so 75 ticks give -pi).
pub(super) fn reticle_spin(frames: u32) -> f32 {
	let pi = std::f32::consts::PI;
	let mut x = ((frames % 150) as f32) * 6.2831855 / 150.0;
	while pi <= x {
		x = (x - pi) - pi;
	}
	while x < -pi {
		x = x + pi + pi;
	}
	x
}

/// FUN_002ac598: the two additive quads of a landing marker (fx textures 17 and 18, the second spinning the other way),
/// lying on the surface at `eye + 0.95 (pos - eye)` and turning once per 150 ticks (REN-3, decomp/notes/bomb_glove.md).
pub fn reticle_quads(reticles: &[Reticle], eye: V3, frames: u32) -> Vec<crate::particles::WorldQuad> {
	reticle_quads_with(reticles, frames, |p| add(eye, scale(sub(p, eye), 0.95)))
}

/// `reticle_quads` of markers whose `pos` is already retail's pvar +0x10, which the predictor stored pulled toward the eye
/// (Eudora FUN_0029f6e8 tail: +0x10 = eye 0x166ec0 + 0.95 (P - eye)); the draws (bomb FUN_0029eec8, Glove of Doom
/// FUN_002bc450, Mine Glove FUN_0029b780) read +0x10 as the translation as it is. [C]
pub fn reticle_quads_placed(reticles: &[Reticle], frames: u32) -> Vec<crate::particles::WorldQuad> {
	reticle_quads_with(reticles, frames, |p| p)
}

fn reticle_quads_with(reticles: &[Reticle], frames: u32, place: impl Fn(V3) -> V3) -> Vec<crate::particles::WorldQuad> {
	use crate::particles::euler_m;
	let wrap = |a: f32| if a > std::f32::consts::PI { a - std::f32::consts::TAU } else if a < -std::f32::consts::PI { a + std::f32::consts::TAU } else { a };
	let turn = |m: [[f32; 3]; 3], v: V3| add(add(scale(m[0], v[0]), scale(m[1], v[1])), scale(m[2], v[2]));
	let q = reticle_spin(frames);
	let mut out = Vec::new();
	for r in reticles {
		let at = place(r.pos);
		let n = r.normal;
		let h = (n[0] * n[0] + n[2] * n[2]).sqrt();
		let orient = euler_m([wrap(-n[1].atan2(h) + 1.5707964), n[0].atan2(n[2]), 0.0]);
		for i in 0..2usize {
			let spin = euler_m([0.0, if i == 1 { -q } else { q }, 0.0]);
			let corner = |v: V3| add(at, turn(orient, turn(spin, v)));
			let p = [[-1.0, 0.0, 1.0], [1.0, 0.0, 1.0], [-1.0, 0.0, -1.0], [1.0, 0.0, -1.0]].map(corner);
			// Strip order v0 v1 v2 v3 -> fan order (v0, v1, v3, v2).
			out.push(crate::particles::WorldQuad { pos: [p[0], p[1], p[3], p[2]], uv: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]], colour: [[1.0; 4]; 4], tex: 0x11 + i, additive: true });
		}
	}
	out
}


