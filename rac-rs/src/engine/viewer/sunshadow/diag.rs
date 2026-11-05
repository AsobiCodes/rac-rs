//! [PC] Sun shadow pass (lane nsun): diagnosis of the casters. RAC_SUN_LOG=1 logs every `EVERY`th frame (counted by `begin`, once
//! per `SunShadow::frame`): the render camera and the light, every cascade, the moby slots that cast into each cascade (props.rs;
//! RAC_SUN_SKIP leaves moby classes out) and, with RAC_SUN_LOG_AT="x y z", the terrain pieces, ties and shrubs whose sphere comes
//! near the ray from that world point towards the sun, with what decides whether they cast (cascade sphere test, tight box, LOD,
//! occlusion).

use super::{dot3, forward, Camera, Cascade, Scene, SunFrame};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

static FRAME: AtomicU32 = AtomicU32::new(0);
static ON: AtomicBool = AtomicBool::new(false);
/// How far up the ray a caster is still listed, in metres.
const UP_TO: f32 = 120.0;
/// Every how many frames RAC_SUN_LOG logs.
const EVERY: u32 = 120;

/// Counts a frame of the shadow pass; every `EVERY`th one (with RAC_SUN_LOG) is logged: the camera, the light and the cascades.
pub(super) fn begin(cam: &Camera, aspect: f32, f: &SunFrame) {
	if !super::super::gfxq::sun_log() {
		return;
	}
	let n = FRAME.fetch_add(1, Relaxed);
	let on = n % EVERY == 0;
	ON.store(on, Relaxed);
	if on {
		eprintln!(
			"sun_log: frame {n} cam pos {:?} yaw {} pitch {} tan_half {} aspect {} up {:?} fwd {:?} sun dir {:?} strength {}",
			cam.pos,
			cam.yaw,
			cam.pitch,
			cam.tan_half,
			aspect,
			cam.up,
			forward(cam),
			f.dir,
			f.strength
		);
		for (k, c) in f.cascades.iter().enumerate().take(f.q.cascades) {
			eprintln!("sun_log: cascade {k} texel {} half {} centre {:?} depth {:?}", c.texel, c.r, c.c, c.z);
		}
	}
}

/// Whether this frame is logged.
pub(super) fn on() -> bool {
	ON.load(Relaxed)
}

/// (distance of `p` from the ray from `at` along the unit `up`, ray parameter); a point below `at` measures to `at`.
fn ray_dist(at: [f32; 3], up: [f32; 3], p: [f32; 3]) -> (f32, f32) {
	let v = [p[0] - at[0], p[1] - at[1], p[2] - at[2]];
	let t = dot3(v, up).max(0.0);
	let q = [v[0] - up[0] * t, v[1] - up[1] * t, v[2] - up[2] * t];
	(dot3(q, q).sqrt(), t)
}

/// Whether a sphere comes within `beside` metres of the ray from `at` along `up`: (ray parameter, distance beside) when it does.
fn near(at: [f32; 3], up: [f32; 3], beside: f32, centre: [f32; 3], radius: f32) -> Option<(f32, f32)> {
	let (d, t) = ray_dist(at, up, centre);
	(d <= radius + beside && t <= UP_TO).then_some((t, d))
}

/// The texture slots of `draws` with their smallest texel alpha (`Scene::sun_alpha`).
fn slots(scene: &Scene, draws: &[super::Draw]) -> Vec<(usize, u8, u32)> {
	draws.iter().map(|d| (d.texture, scene.sun_alpha.get(d.texture).copied().unwrap_or(0), d.count)).collect()
}

/// RAC_SUN_LOG_AT: logs the terrain pieces, ties and shrubs of cascade `k` near the ray from the point towards the sun.
pub(super) fn near_ray(scene: &Scene, f: &SunFrame, k: usize, c: &Cascade) {
	let Some((at, m)) = super::super::gfxq::sun_log_at().filter(|_| on()) else { return };
	let up = [-f.dir[0], -f.dir[1], -f.dir[2]];
	for (i, t) in scene.tfrag_set.iter().enumerate() {
		let Some((u, d)) = near(at, up, m, t.cull.centre, t.cull.radius) else { continue };
		let lod = scene.tfrag_lod.get(i).copied().unwrap_or(u8::MAX);
		eprintln!("sun_log: k{k} tfrag {i} centre {:?} r {} up {u:.2} beside {d:.2} lod {lod} ever {} passes {} touches {}", t.cull.centre, t.cull.radius, scene.occ.ever_visible(t.cull.occ), scene.occ.passes(t.cull.occ), c.touches(t.cull.centre, t.cull.radius));
	}
	for (i, t) in scene.tie_cull.iter().enumerate() {
		let Some((u, d)) = near(at, up, m, t.centre, t.radius) else { continue };
		let lod = scene.tie_lod.get(i).copied().unwrap_or(u8::MAX);
		let cast_lod = if lod < 3 { lod as usize } else { 0 };
		let bb = scene.sun_bb.ties.get(i).and_then(|b| b[cast_lod]);
		let r = &scene.tie_ranges[i][cast_lod];
		let ds = &scene.tie_draws[r.start as usize..r.end as usize];
		eprintln!(
			"sun_log: k{k} tie {i} centre {:?} r {} up {u:.2} beside {d:.2} lod {lod} ever {} passes {} touches {} box {:?} reaches {:?} fat {:?} q {:?} draws {:?}",
			t.centre,
			t.radius,
			scene.occ.ever_visible(t.occ),
			scene.occ.passes(t.occ),
			c.touches(t.centre, t.radius),
			bb,
			bb.map(|b| c.reaches(&b)),
			scene.tie_fat.get(i),
			scene.tie_q.get(i),
			slots(scene, ds)
		);
	}
	for (i, s) in scene.shrub_cull.iter().enumerate() {
		let Some((u, d)) = near(at, up, m, s.centre, s.radius) else { continue };
		let bb = scene.sun_bb.shrubs.get(i).copied().flatten();
		let ds = &scene.shrubs.draws[scene.shrub_ranges[i].0.clone()];
		eprintln!("sun_log: k{k} shrub {i} centre {:?} r {} up {u:.2} beside {d:.2} touches {} box {:?} reaches {:?} draws {:?}", s.centre, s.radius, c.touches(s.centre, s.radius), bb, bb.map(|b| c.reaches(&b)), slots(scene, ds));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_caster_is_near_the_ray_when_its_sphere_comes_within_reach() {
		let up = [0.0, 0.0, 1.0];
		assert_eq!(ray_dist([0.0; 3], up, [3.0, 4.0, 10.0]), (5.0, 10.0));
		assert_eq!(ray_dist([0.0; 3], up, [0.0, 0.0, -2.0]), (2.0, 0.0), "below the point: the distance to the point");
		assert_eq!(near([0.0; 3], up, 0.75, [1.0, 0.0, 5.0], 0.5), Some((5.0, 1.0)));
		assert_eq!(near([0.0; 3], up, 0.75, [2.0, 0.0, 5.0], 0.5), None);
		assert_eq!(near([0.0; 3], up, 2.0, [2.0, 0.0, 5.0], 0.5), Some((5.0, 2.0)));
		assert_eq!(near([0.0; 3], up, 0.75, [0.0, 0.0, UP_TO + 1.0], 0.5), None);
	}
}
