//! Ground shadow quads of the ship pad stream and the flying pad ship.

use super::*;

/// A scene-local point through the ship pad frame (`place_at` of the viewer tick): yaw about z, then the pad position.
pub(super) fn pad_place(p: [f32; 3], yaw: f32, v: [f32; 3]) -> [f32; 3] {
	let (s, c) = yaw.sin_cos();
	[p[0] + v[0] * c - v[1] * s, p[1] + v[0] * s + v[1] * c, p[2] + v[2]]
}

/// NOV-14.5d: the ground shadow quads of the ship-stream actors (class 0x213..=0x215) of scene `id` at `tick`, in the
/// pad frame `(pos, yaw)`; `ship` is the pad ship's type (retail `sRam0013e056`), `flight_tick` the substate-3 tick.
#[allow(clippy::too_many_arguments)]
/// SCN-4.3e: the flying pad ship's ground shadow (`cam` = eye, forward, right, up, projection scalar; `c` the flight's
/// tick counter, retail `sRam0013e054` = ticks done minus one).
#[allow(clippy::too_many_arguments)]
pub(super) fn flight_shadow_quads(world: &World, pos: [f32; 3], rot: [f32; 3], t: usize, sphere: [f32; 4], scale: f32, c: i32, cam: ([f32; 3], [f32; 3], [f32; 3], [f32; 3], f32)) -> Option<particles::WorldQuad> {
	let m = particles::euler_m(rot);
	let (cx, cy, cz) = (sphere[0] * scale, sphere[1] * scale, sphere[2] * scale);
	let centre = [0, 1, 2].map(|i| pos[i] + (m[0][i] * cx + m[1][i] * cy + m[2][i] * cz) / 1024.0);
	let a = crate::shadows::ship_alpha(centre, sphere[3] * scale, cam.0, cam.1, cam.2, cam.3, cam.4, true)?;
	let a = crate::shadows::ship_fade(a, Some(c))?;
	crate::shadows::ship_shadow_m(world, pos, m, t, a)
}

pub(super) fn ship_shadow_quads(id: u32, sc: &scene::Scene, tick: f32, pad: ([f32; 3], f32), ship: usize, actor_slots: &[(u32, usize, u32)], prop_anims: &HashMap<u32, (mobies::MobyClass, anim::Animated)>, world: &World, flight_tick: Option<i32>) -> Vec<particles::WorldQuad> {
	let (pad_pos, yaw) = pad;
	let Some(mut k) = sc.camera_at(tick) else { return Vec::new() };
	k.rot[2] += yaw;
	let eye = pad_place(pad_pos, yaw, k.pos);
	let (fwd, up) = (k.forward(), k.up());
	let right = [fwd[1] * up[2] - fwd[2] * up[1], fwd[2] * up[0] - fwd[0] * up[2], fwd[0] * up[1] - fwd[1] * up[0]];
	let m = particles::euler_m([0.0, 0.0, yaw]);
	let mut out = Vec::new();
	for &(sid, index, class_id) in actor_slots {
		if sid != id || class_id.wrapping_sub(0x213) >= 3 {
			continue;
		}
		let (Some(actor), Some((class, _))) = (sc.actors.iter().find(|a| a.index == index), prop_anims.get(&class_id)) else { continue };
		let (Some((ka, _, _)), Some(p0)) = (actor.pair(tick), actor.pos_at(tick)) else { continue };
		let pos = pad_place(pad_pos, yaw, p0);
		// moby[0..3] (FUN_00265770): pos * 1024 + R * (centre * s), radius r * s, in 1024 units.
		let (s, c) = (class.scale, ka.sphere);
		let centre = [0, 1, 2].map(|i| pos[i] + (m[0][i] * (c[0] * s) + m[1][i] * (c[1] * s) + m[2][i] * (c[2] * s)) / 1024.0);
		let Some(a) = crate::shadows::ship_alpha(centre, c[3] * s, eye, fwd, right, up, k.tan_half_fov, true) else { continue };
		let Some(a) = crate::shadows::ship_fade(a, flight_tick) else { continue };
		out.extend(crate::shadows::ship_shadow(world, pos, yaw, ship, a));
	}
	out
}
