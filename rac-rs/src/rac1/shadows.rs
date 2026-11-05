//! Ground blob shadows: the shared moby blob queue (FUN_00259bf8 -> FUN_001f95e0), drawn as one
//! textured quad per caster on the ground under it (decomp/notes/shadows.md).

use crate::particles::WorldQuad;
use crate::physics::{World, V3};

/// The queue takes at most 32 blobs a frame (DAT_0016e080).
pub const CAPACITY: usize = 32;
/// fx texture 0: the 32x32 dark blob.
pub const TEXTURE: usize = 0;
/// Vertex colour 0x40808080: RGB 0x80 = 1.0 (modulate), alpha 0x40 of 0x80 = half.
const COLOUR: [f32; 4] = [1.0, 1.0, 1.0, 0.5];
/// The blob sits this far above the hit (FUN_00259bf8).


/// One caster's blob: a ray down from z + 0.5 to z = 0.01; nothing when it finds no ground. The
/// square's half-size is `radius * max(0.25, (8 - height) * 0.125)` (size fade only, alpha is fixed).
pub fn blob(world: &World, pos: V3, radius: f32) -> Option<WorldQuad> {
	let from = [pos[0], pos[1], pos[2] + 0.5];
	let (z, n) = world.ground(from, (from[2] - 0.01).max(0.0))?;
	Some(blob_quad(pos, z, n, radius))
}

/// FUN_0021b160 (NOV-14.3b): the blob square from the ground z `gz` and normal `n` (normalised here). The centre is
/// lifted 0.025 in z only; each corner (cx, cy) is projected onto the ground plane along z.
pub fn blob_quad(pos: V3, gz: f32, n: V3, radius: f32) -> WorldQuad {
	let s = radius * ((8.0 - (pos[2] - gz).abs()) * 0.125).max(0.25);
	blob_square([pos[0], pos[1], gz + 0.025], s, n)
}

/// The draw side of a queued blob (Eudora shadow_queue_draw FUN_001ebec8): centre `p` (z already ground + 0.025), half-size
/// `s` (radius times the height fade) and ground normal `n` (set_length 1 here), one record of the retail queue as is.
pub fn blob_square(p: V3, s: f32, n: V3) -> WorldQuad {
	let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
	let n = [n[0] / l, n[1] / l, n[2] / l];
	let at = |cx: f32, cy: f32| {
		let d = cx * n[0] + cy * n[1];
		[p[0] + (cx - n[0] * d) * s, p[1] + (cy - n[1] * d) * s, p[2] - n[2] * d * s]
	};
	WorldQuad {
		pos: [at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)],
		uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
		colour: [COLOUR; 4],
		tex: TEXTURE,
		additive: false,
	}
}

/// Ship ground shadow corners (x, y) per ship type 0..=2 (overlay table 0x1bdcd0, stride 0x40; NOV-14.5d).
const SHIP_CORNERS: [[[f32; 2]; 4]; 3] = [
	[[4.0, 1.6], [4.0, -1.6], [-4.0, 1.6], [-4.0, -1.6]],
	[[4.0, 2.0], [4.0, -2.0], [-4.0, 2.0], [-4.0, -2.0]],
	[[3.5, 1.5], [3.5, -1.5], [-3.5, 2.0], [-3.5, -2.0]],
];

pub use crate::vumath::fast_atan2;

/// The ship shadow's alpha (`FUN_00221e18(32.0, sphere)`, NOV-14.5d): `None` when the sphere (centre world units,
/// `radius` = r * s in 1024 units) is outside the view volume, else `clamp((32 * 1024 - (Vz - R)) >> 7, 0, 128)` with
/// `Vz` the 1024-unit depth along `fwd`. `right`/`up` are the other two view axes (only `|Vx|`, `|Vy|` are used),
/// `tan_half` the camera's horizontal projection scalar, `pal` the PAL aspect (0.775, else 0.756).
#[allow(clippy::too_many_arguments)]
pub fn ship_alpha(centre: V3, radius: f32, eye: V3, fwd: V3, right: V3, up: V3, tan_half: f32, pal: bool) -> Option<i32> {
	let f = tan_half;
	let fk = f * if pal { 0.775 } else { 0.756 };
	let sec = |x: f32| 1.0 / crate::camera::fast_cos(fast_atan2(1.0, x));
	let (cef8, cefc) = (sec(f), sec(fk));
	let d = [0, 1, 2].map(|i| centre[i] * 1024.0 - eye[i] * 1024.0);
	let dot = |a: V3| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
	let (vz, vx, vy) = (dot(fwd), dot(right), dot(up));
	let r = radius;
	let y3 = 32.0 * 1024.0 - (vz - r);
	if y3.is_sign_negative() || !(-(r + vz)).is_sign_negative() || (f * vz - vx.abs() + cef8 * r).is_sign_negative() || (fk * vz - vy.abs() + cefc * r).is_sign_negative() {
		return None;
	}
	Some(((y3 as u32) >> 7).min(128) as i32)
}

/// The substate-3 (flight away) fade of the ship shadow alpha: `a += (t - 125) * -4` once `t > 125`, nothing drawn
/// when it falls below 1. `flight_tick` is `None` in the streams the port plays (substates 0 and 8).
pub fn ship_fade(alpha: i32, flight_tick: Option<i32>) -> Option<i32> {
	let a = match flight_tick {
		Some(t) if t > crate::gameplay::pal_ticks(0x96) => alpha + (t - crate::gameplay::pal_ticks(0x96)) * -4,
		_ => alpha,
	};
	(a >= 1 || flight_tick.is_none_or(|t| t <= crate::gameplay::pal_ticks(0x96))).then_some(a)
}

/// The flat ground shadow of the pad ship's stream actors (FUN_002a1a60, mode 6; NOV-14.5d): four corners of ship type
/// `ship` turned by `yaw` about `pos`, at the ground z under the ship plus 0.1 (0.0 + 0.1 on a miss).
pub fn ship_shadow(world: &World, pos: V3, yaw: f32, ship: usize, alpha: i32) -> Option<WorldQuad> {
	ship_shadow_m(world, pos, crate::particles::euler_m([0.0, 0.0, yaw]), ship, alpha)
}

/// `ship_shadow` with the ship's whole basis `m` (`euler_m` rows) instead of a yaw: the departure flight's shadow
/// (SCN-4.3e) turns the corners by the flying ship's matrix.
pub fn ship_shadow_m(world: &World, pos: V3, m: [[f32; 3]; 3], ship: usize, alpha: i32) -> Option<WorldQuad> {
	ship_shadow_z(pos, m, ship, alpha, ship_ground_z(world, pos))
}

/// The mode-6 shadow height: the ground under `pos` (ray from pos.z + 0.5 down to z 0.01, FUN_0026e1b0) plus 0.1.
pub fn ship_ground_z(world: &World, pos: V3) -> f32 {
	world.ray_masked([pos[0], pos[1], pos[2] + 0.5], [pos[0], pos[1], 0.01], 2).map_or(0.0, |h| h.p[2]) + 0.1
}

/// `ship_shadow_m` at height `z` (outside mode 6 retail takes pos.z + 0.1, Eudora FUN_0027ee90).
pub fn ship_shadow_z(pos: V3, m: [[f32; 3]; 3], ship: usize, alpha: i32, z: f32) -> Option<WorldQuad> {
	let corners = SHIP_CORNERS.get(ship)?;
	let v = |c: [f32; 2]| [c[0] * m[0][0] + c[1] * m[1][0] + pos[0], c[0] * m[0][1] + c[1] * m[1][1] + pos[1], z];
	Some(WorldQuad {
		pos: [v(corners[0]), v(corners[1]), v(corners[3]), v(corners[2])],
		uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
		colour: [[1.0, 1.0, 1.0, (alpha >> 1) as f32 / 128.0]; 4],
		tex: TEXTURE,
		additive: false,
	})
}

/// Blobs for the casters nearest the camera, up to the queue's 32 [I: the game keeps the first 32 queued].
pub fn blobs(world: &World, casters: &[(V3, f32)], camera: V3) -> Vec<WorldQuad> {
	let mut near: Vec<&(V3, f32)> = casters.iter().collect();
	let d = |p: &V3| (0..3).map(|i| (p[i] - camera[i]).powi(2)).sum::<f32>();
	near.sort_by(|a, b| d(&a.0).total_cmp(&d(&b.0)));
	near.into_iter().take(CAPACITY).filter_map(|&(p, r)| blob(world, p, r)).collect()
}

pub use crate::shadowgeom::{RING_TURN, volume_fade, volume_light, VolumeSphere, VolumeCaster, ShadowVolume, volume_basis, volumes, RecPrim, RecCaster, capsule_ring};

#[cfg(test)]
mod tests {
	use crate::{frontend, iso, level::Level, toc};
	use std::path::Path;

	/// BRIDGE-64a test: the collector fade (shark dist 0x16 near and far), the alpha-counter steps and the ring step.
	#[test]
	fn volume_fade_counter_and_step() {
		use super::volume_fade;
		assert_eq!(((0x16 << 10) - 10000) >> 1, 6264);
		assert_eq!(volume_fade(0x16, 10000), 0x1000);
		assert_eq!(volume_fade(0x16, 20000), 1264);
		assert_eq!(1264.0f32 / 4096.0, 0.30859375);
		assert_eq!([127 * 130 >> 7, 128 * 127 >> 7, 128 * 130 >> 7, 130 * 127 >> 7], [128, 127, 130, 128]);
		assert_eq!(super::RING_TURN / 8.0, 0.785);
	}

	/// BRIDGE-64a: light slot 0 for a hero light vector along +x (a = 0) and +y (a = pi/2).
	#[test]
	fn volume_light_slot_zero() {
		let l = super::volume_light([1.0, 0.0, 0.0]);
		assert!((l[0] - 0.14).abs() < 1e-5 && l[1].abs() < 1e-5 && l[2] == -0.99, "{l:?}");
		let l = super::volume_light([0.0, 2.0, 0.0]);
		assert!(l[0].abs() < 1e-4 && (l[1] - 0.14).abs() < 1e-5, "{l:?}");
	}

	/// The z-pass count of a ray from `eye` to `p`: +1 per `inc` triangle crossed before `p`, -1 per `dec` one.
	fn ray_count(vols: &[super::ShadowVolume], eye: crate::physics::V3, p: crate::physics::V3) -> i32 {
		use crate::physics::{cross, dot, sub};
		let hit = |t: &[crate::physics::V3; 3]| {
			let d = sub(p, eye);
			let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
			let h = cross(d, e2);
			let a = dot(e1, h);
			if a.abs() < 1e-9 {
				return false;
			}
			let s = sub(eye, t[0]);
			let u = dot(s, h) / a;
			let q = cross(s, e1);
			let v = dot(d, q) / a;
			let t = dot(e2, q) / a;
			(0.0..=1.0).contains(&u) && v >= 0.0 && u + v <= 1.0 && t > 0.0 && t < 1.0
		};
		vols.iter().map(|v| v.inc.iter().filter(|t| hit(t)).count() as i32 - v.dec.iter().filter(|t| hit(t)).count() as i32).sum()
	}

	/// BRIDGE-64a test: one sphere (r 0.5, 8 sides) at z 4 over ground at z 3 (band [2.8, 3.2]): the ground under it along
	/// the light is darkened (count 1), the ground beside it and the caster itself are not (count 0), from above and from
	/// the side; a straddling caster with every vertex outside draws nothing, band 0 means no volume.
	#[test]
	fn sphere_volume_darkens_the_ground_disc() {
		use super::{volumes, VolumeCaster, VolumeSphere};
		let mut m = [0.0; 16];
		(m[0], m[5], m[10], m[15]) = (1.0, 1.0, 1.0, 1.0);
		(m[12], m[13], m[14]) = (0.0, 0.0, 4.0);
		let caster = VolumeCaster { spheres: vec![VolumeSphere { c: [0.0; 3], r: 0.5, n: 8 }], m, k: 1.0, band: [2.8, 3.2], straddle: false };
		let light = super::volume_light([1.0, 0.0, 0.0]);
		let dx = 0.14 / 0.99;
		for eye in [[10.0, 0.0, 10.0], [0.0, -12.0, 6.0], [1.0, 1.0, 30.0]] {
			let vols = volumes(std::slice::from_ref(&caster), light, eye, &|_| false);
			assert_eq!(vols.len(), 1);
			assert_eq!(ray_count(&vols, eye, [dx, 0.0, 3.0]), 1, "eye {eye:?}");
			assert_eq!(ray_count(&vols, eye, [dx + 0.3, 0.1, 3.0]), 1, "eye {eye:?}");
			assert_eq!(ray_count(&vols, eye, [dx + 0.8, 0.0, 3.0]), 0, "eye {eye:?}");
			assert_eq!(ray_count(&vols, eye, [-2.0, 1.0, 3.0]), 0, "eye {eye:?}");
			assert_eq!(ray_count(&vols, eye, [0.0, 0.0, 4.0]), 0, "eye {eye:?}");
		}
		let all_out = VolumeCaster { straddle: true, ..caster.clone() };
		let vols = volumes(&[all_out], light, [10.0, 0.0, 10.0], &|_| true);
		assert!(vols[0].inc.is_empty() && vols[0].dec.is_empty());
		let none = VolumeCaster { band: [0.0; 2], ..caster };
		assert!(volumes(&[none], light, [10.0, 0.0, 10.0], &|_| false).is_empty());
	}

	/// NOV-14.3b: the retail blob projection (n = (0, 0.6, 0.8), P = (10, 20, 5) over ground z 4).
	#[test]
	fn blob_projection_corners() {
		let q = super::blob_quad([10.0, 20.0, 5.0], 4.0, [0.0, 0.6, 0.8], 0.3);
		let want = [[9.7375, 19.832, 4.151], [10.2625, 19.832, 4.151], [10.2625, 20.168, 3.899], [9.7375, 20.168, 3.899]];
		for (g, w) in q.pos.iter().zip(want) {
			assert!((0..3).all(|i| (g[i] - w[i]).abs() < 1e-5), "{g:?} {w:?}");
		}
		assert_eq!(q.uv, [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]);
	}

	/// NOV-14.5d test 1: alpha from the sphere depth (R = 6144 = 6 world units, camera along +y).
	#[test]
	fn ship_alpha_by_depth() {
		let a = |depth: f32| super::ship_alpha([0.0, depth, 0.0], 6144.0, [0.0; 3], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], 0.63, true);
		assert_eq!((a(20.0), a(30.0), a(38.0), a(40.0)), (Some(128), Some(64), Some(0), None));
	}

	/// NOV-14.5d test 2: the substate-3 fade.
	#[test]
	fn ship_fade_flight() {
		assert_eq!(super::ship_fade(64, Some(135)), Some(24));
		assert_eq!(super::ship_fade(64, Some(141)), None);
		assert_eq!(super::ship_fade(64, None), Some(64));
		assert_eq!(super::ship_fade(0, None), Some(0));
		assert_eq!(super::ship_fade(64, Some(125)), Some(64));
	}

	/// NOV-14.5d tests 3-5: corners through the yaw, the ground z, the type rows.
	#[test]
	fn ship_shadow_corners() {
		use crate::physics::World;
		let yaw = 1.5707964f32;
		let floor = World::new([[[-50.0, -50.0, 4.0], [50.0, -50.0, 4.0], [50.0, 50.0, 4.0]], [[-50.0, -50.0, 4.0], [50.0, 50.0, 4.0], [-50.0, 50.0, 4.0]]], 4.0);
		let q = super::ship_shadow(&floor, [10.0, 20.0, 5.0], yaw, 0, 128).unwrap();
		let want = [[8.4, 24.0, 4.1], [11.6, 24.0, 4.1], [11.6, 16.0, 4.1], [8.4, 16.0, 4.1]];
		for k in 0..4 {
			for i in 0..3 {
				assert!((q.pos[k][i] - want[k][i]).abs() < 1e-5, "{k} {i}: {:?}", q.pos);
			}
		}
		assert_eq!(q.colour[0][3], 64.0 / 128.0);
		assert_eq!((q.tex, q.additive, q.uv), (super::TEXTURE, false, [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]));
		// No ground: z = 0.0 + 0.1.
		let none = World::new([], 4.0);
		assert_eq!(super::ship_shadow(&none, [10.0, 20.0, 5.0], 0.0, 1, 0).unwrap().pos[0], [14.0, 22.0, 0.1]);
		assert_eq!(super::ship_shadow(&none, [0.0; 3], 0.0, 2, 0).unwrap().pos[0][..2], [3.5, 1.5]);
		assert!(super::ship_shadow(&none, [0.0; 3], 0.0, 3, 0).is_none());
		// SCN-4.3e test 3: the matrix form is the yaw form bit for bit.
		let a = super::ship_shadow(&floor, [10.0, 20.0, 5.0], yaw, 0, 128).unwrap();
		let b = super::ship_shadow_m(&floor, [10.0, 20.0, 5.0], crate::particles::euler_m([0.0, 0.0, yaw]), 0, 128).unwrap();
		assert_eq!((a.pos, a.uv, a.colour), (b.pos, b.uv, b.colour));
	}

	/// SCN-4.3e test 2: the flight fade of the shadow alpha.
	#[test]
	fn flight_fade_values() {
		assert_eq!(super::ship_fade(64, Some(126)), Some(60));
		assert_eq!(super::ship_fade(4, Some(126)), None);
		assert_eq!(super::ship_fade(64, Some(125)), Some(64));
	}

	/// NOV-14.5d test 6 (numpy float32 values of the spec).
	#[test]
	fn fast_atan2_values() {
		let tol = 1e-6;
		for (a, b, want) in [(1.0f32, 0.63f32, 0.5621867f32), (1.0, 0.48825, 0.45420355), (1.0, 2.52, 1.1930293), (0.5, -0.5, -0.7853982), (-1.0, 0.0, 3.141593), (0.0, 0.0, 0.0)] {
			let got = super::fast_atan2(a, b);
			assert!((got - want).abs() < tol, "({a}, {b}): {got} vs {want}");
		}
		let sec = |x: f32| 1.0 / crate::camera::fast_cos(super::fast_atan2(1.0, x));
		assert!((sec(0.63) - 1.1819053).abs() < 1e-6 && (sec(0.48825) - 1.1128289).abs() < 1e-6);
	}

	const ISO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");

	/// BRIDGE-64b tests: the capsule ring counts (equal radii, flat ends, containment) and the first end-1 vertex.
	#[test]
	fn capsule_ring_counts() {
		use super::{capsule_ring, volume_basis};
		let (d, e, p) = volume_basis([0.0, 0.0, -1.0]);
		let (dt, et, pt) = volume_basis([0.14, 0.0, -0.99]);
		let (p0, p1) = ([0.0, 0.0, 0.0], [4.0, 0.0, 0.0]);
		let r = capsule_ring(p0, 1.0, p1, 1.0, 4, 4, d, p, e);
		assert_eq!(r.len(), 9);
		assert_eq!(r[8], r[0]);
		assert!((r[0].1[2] - r[0].0[2] + 1000.0).abs() < 1e-3);
		// End 1 starts at angle (1.57 + (1.57 + 3.14)) - 6.28 = 0: p1 + u, u = unit((p1 - p0) x D).
		let u = crate::physics::normalize(crate::physics::cross(crate::physics::sub(p1, p0), d));
		let w = crate::physics::sub(r[4].0, p1);
		assert!((w[0] - u[0]).abs() < 1e-3 && (w[1] - u[1]).abs() < 1e-3, "{w:?} {u:?}");
		assert_eq!(capsule_ring(p0, 1.0, p1, 1.0, 0, 0, d, p, e).len(), 5);
		// Containment (L 1 + r1 1 - r0 3 < 0): the p0 ring alone, every top 3 from p0.
		let c = capsule_ring(p0, 3.0, [1.0, 0.0, 0.0], 1.0, 8, 4, dt, pt, et);
		assert_eq!(c.len(), 9);
		assert!(c.iter().all(|q| ((q.0[0] * q.0[0] + q.0[1] * q.0[1] + q.0[2] * q.0[2]).sqrt() - 3.0).abs() < 1e-3));
		assert!(capsule_ring(p0, 3.0, [1.0, 0.0, 0.0], 1.0, 0, 4, d, p, e).is_empty());
	}
}
