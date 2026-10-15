//! PC-FPS-1 [PC] (port extra, user request 2026-10-06): render interpolation for displays faster than the 50 Hz game.
//!
//! The simulation keeps its fixed 0.02 s steps; this module only changes what is drawn. Each display frame draws the
//! hero, the props, their skinned poses and the pool sprites between the state before the newest tick and the state
//! after it, at `alpha = accum / 0.02` (one tick of display latency, the usual fixed-step scheme). Nothing here is read
//! back by the game, the collision, the sound or the RNG. Off (Options > Camera "Smooth motion") or during a `--shot`
//! capture every helper returns the current tick's state unchanged, which is the exact 50 Hz stepping.
//!
//! Snap rules (draw the newest state, no blend): a move longer than `SNAP_DIST` in one tick (teleport, respawn, a
//! scene cut placing an actor), a prop that was hidden or of another class before the tick (spawn / despawn), a pose
//! whose vertex count changed, a hidden vertex range (the `hide` sentinel at the origin) on either side, more than one
//! tick in a display frame (poses only), the pause menu, and a level load (`Smooth::default`).
use super::*;

type V3 = [f32; 3];

/// One tick's move longer than this is a teleport / respawn / cut: drawn at the new state (200 u/s, past every
/// gameplay speed).
pub(super) const SNAP_DIST: f32 = 4.0;

thread_local! {
	/// Per prop slot for the next `Gpu::set_props`: the interpolated placement matrix and scale (None = the prop's own).
	pub(super) static PROP_XF: std::cell::RefCell<Vec<Option<([f32; 16], f32)>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The interpolated placement of prop `pi` for this frame, if any.
pub(super) fn prop_xf(pi: usize) -> Option<([f32; 16], f32)> {
	PROP_XF.with(|t| t.borrow().get(pi).copied().flatten())
}

pub(super) fn lerp3(a: V3, b: V3, t: f32) -> V3 {
	[a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn dist2(a: V3, b: V3) -> f32 {
	(0..3).map(|i| (b[i] - a[i]) * (b[i] - a[i])).sum()
}

/// True when a one-tick move from `a` to `b` may be drawn blended (not a teleport).
pub(super) fn near(a: V3, b: V3) -> bool {
	dist2(a, b) <= SNAP_DIST * SNAP_DIST
}

/// Angle blend along the shorter way round.
pub(super) fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
	use std::f32::consts::{PI, TAU};
	let d = (b - a).rem_euclid(TAU);
	let d = if d > PI { d - TAU } else { d };
	a + d * t
}

/// Unit quaternion (x, y, z, w) of the rotation in a column-major 4x4 (columns 0..2 = the rotated basis).
fn quat_of(m: &[f32; 16]) -> [f32; 4] {
	let (m00, m11, m22) = (m[0], m[5], m[10]);
	// r(row, col) = m[col * 4 + row]
	let r = |row: usize, col: usize| m[col * 4 + row];
	let tr = m00 + m11 + m22;
	let q = if tr > 0.0 {
		let s = (tr + 1.0).sqrt() * 2.0;
		[(r(2, 1) - r(1, 2)) / s, (r(0, 2) - r(2, 0)) / s, (r(1, 0) - r(0, 1)) / s, 0.25 * s]
	} else if m00 > m11 && m00 > m22 {
		let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
		[0.25 * s, (r(0, 1) + r(1, 0)) / s, (r(0, 2) + r(2, 0)) / s, (r(2, 1) - r(1, 2)) / s]
	} else if m11 > m22 {
		let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
		[(r(0, 1) + r(1, 0)) / s, 0.25 * s, (r(1, 2) + r(2, 1)) / s, (r(0, 2) - r(2, 0)) / s]
	} else {
		let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
		[(r(0, 2) + r(2, 0)) / s, (r(1, 2) + r(2, 1)) / s, 0.25 * s, (r(1, 0) - r(0, 1)) / s]
	};
	let n = q.iter().map(|c| c * c).sum::<f32>().sqrt().max(1e-12);
	q.map(|c| c / n)
}

/// Spherical blend of two unit quaternions along the shorter arc.
pub(super) fn slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
	let mut d: f32 = (0..4).map(|i| a[i] * b[i]).sum();
	let b = if d < 0.0 {
		d = -d;
		b.map(|c| -c)
	} else {
		b
	};
	let (wa, wb) = if d > 0.9995 {
		(1.0 - t, t)
	} else {
		let th = d.min(1.0).acos();
		let s = th.sin();
		(((1.0 - t) * th).sin() / s, (t * th).sin() / s)
	};
	let q: [f32; 4] = std::array::from_fn(|i| a[i] * wa + b[i] * wb);
	let n = q.iter().map(|c| c * c).sum::<f32>().sqrt().max(1e-12);
	q.map(|c| c / n)
}

/// Placement matrix between two rigid placements: slerped rotation, lerped translation.
pub(super) fn blend_place(a: &[f32; 16], b: &[f32; 16], t: f32) -> [f32; 16] {
	let [x, y, z, w] = slerp(quat_of(a), quat_of(b), t);
	let p = lerp3([a[12], a[13], a[14]], [b[12], b[13], b[14]], t);
	[
		1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w), 0.0,
		2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w), 0.0,
		2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y), 0.0,
		p[0], p[1], p[2], 1.0,
	]
}

/// Vertex blend of two skinned poses of one model (linear in the joint matrices, as skinning is). None when the
/// vertex counts differ. A vertex hidden (at the origin) on either side, or one that jumped, takes the new pose.
pub(super) fn blend_posed(a: &Posed, b: &Posed, t: f32) -> Option<Posed> {
	if a.0.len() != b.0.len() || a.1.len() != b.1.len() {
		return None;
	}
	let mut ps = Vec::with_capacity(b.0.len());
	let mut ns = Vec::with_capacity(b.1.len());
	for (k, (&pa, &pb)) in a.0.iter().zip(&b.0).enumerate() {
		let keep = pa == [0.0; 3] || pb == [0.0; 3] || !near(pa, pb);
		ps.push(if keep { pb } else { lerp3(pa, pb, t) });
		let (na, nb) = (a.1[k], b.1[k]);
		ns.push(if keep {
			nb
		} else {
			let n = lerp3(na, nb, t);
			let l = dist2([0.0; 3], n).sqrt();
			if l > 1e-6 { n.map(|c| c / l) } else { nb }
		});
	}
	Some((ps, ns))
}

/// A prop's placement before the newest tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PropXf {
	pub pos: V3,
	pub rot: V3,
	pub scale: f32,
	pub shown: bool,
	pub class: u32,
	pub key: PoseKey,
}

impl PropXf {
	pub(super) fn of(p: &Prop) -> PropXf {
		PropXf { pos: p.pos, rot: p.rot, scale: p.scale, shown: p.shown, class: p.class_id, key: pose_key(p) }
	}
}

/// Whether a prop may be drawn blended from `prev` to its current state.
pub(super) fn prop_blends(prev: &PropXf, cur: &PropXf) -> bool {
	prev.shown && cur.shown && prev.class == cur.class && near(prev.pos, cur.pos)
}

#[derive(Default)]
pub(super) struct Smooth {
	/// This display frame: ticks run, blend factor, and whether blending is on (setting on, not paused, no capture).
	pub ticks: u32,
	pub alpha: f32,
	pub active: bool,
	/// The state before the newest tick (written at the top of every tick).
	props_prev: Vec<PropXf>,
	hero_prev: Option<(V3, f32)>,
	/// BRIDGE-75 [PC]: the hero body tilt (roll, pitch) before the newest tick.
	hero_tilt_prev: [f32; 2],
	/// Skinned poses: the last computed (state after the newest tick seen) and the one before it.
	hero_last: Option<Posed>,
	hero_before: Option<Posed>,
	posed_last: HashMap<PoseKey, Posed>,
	posed_before: HashMap<PoseKey, Posed>,
	fx_last: HashMap<usize, Posed>,
	fx_before: HashMap<usize, Posed>,
	/// Pool sprites by slot key: position and texture after the newest tick and before it.
	sprites_last: HashMap<u32, (V3, u32)>,
	sprites_before: HashMap<u32, (V3, u32)>,
	/// The camera of the scene's true tick (pos, yaw, pitch, tan_half), put back once the world draw lists are built
	/// (handler.rs) so the drawn (offset) scene camera never reaches the sound or the next game ticks.
	pub tick_cam: Option<(V3, f32, f32, f32)>,
}

impl Smooth {
	/// Top of each game tick: remember the state the tick starts from.
	pub(super) fn begin_tick(&mut self, game: &Game) {
		self.ticks += 1;
		self.props_prev.clear();
		self.props_prev.extend(game.props.iter().map(PropXf::of));
		self.hero_prev = Some((game.player.pos, game.player.yaw));
		self.hero_tilt_prev = game.player.body_tilt;
	}

	/// After the game ticks of a display frame: the blend factor and whether to blend.
	pub(super) fn frame(&mut self, on: bool, accum: f32) {
		self.alpha = (accum / crate::gameplay::PAL_DT).clamp(0.0, 1.0);
		self.active = on;
	}

	/// Pose history step; `ticks == 1` keeps the previous pose, more than one tick per frame drops it (snap).
	fn roll<K: std::hash::Hash + Eq>(ticks: u32, last: &mut HashMap<K, Posed>, before: &mut HashMap<K, Posed>, now: HashMap<K, Posed>) {
		if ticks > 0 {
			*before = if ticks == 1 { std::mem::take(last) } else { HashMap::new() };
		}
		*last = now;
	}

	/// The hero's drawn placement, pose and body tilt (`at` = feet and yaw, None = not drawn; tilt = BRIDGE-75 roll, pitch).
	pub(super) fn hero(&mut self, game: &Game, at: Option<(V3, f32)>, skinned: Option<Posed>) -> (Option<(V3, f32)>, Option<Posed>, [f32; 2]) {
		if self.ticks > 0 {
			self.hero_before = if self.ticks == 1 { self.hero_last.take() } else { None };
			self.hero_last = skinned.clone();
		}
		let tilt_now = game.player.body_tilt;
		if !self.active {
			return (at, skinned, tilt_now);
		}
		let t = self.alpha;
		let cur = (game.player.pos, game.player.yaw);
		let mut tilt = tilt_now;
		let at = match (at, self.hero_prev) {
			// Only the plain gameplay placement blends; any other placement is drawn as given.
			(Some(a), Some(prev)) if a == cur && near(prev.0, a.0) => {
				let p = self.hero_tilt_prev;
				tilt = [lerp_angle(p[0], tilt_now[0], t), lerp_angle(p[1], tilt_now[1], t)];
				Some((lerp3(prev.0, a.0, t), lerp_angle(prev.1, a.1, t)))
			}
			(a, _) => a,
		};
		let skinned = match (&self.hero_before, skinned) {
			(Some(b), Some(s)) => blend_posed(b, &s, t).or(Some(s)),
			(_, s) => s,
		};
		(at, skinned, tilt)
	}

	/// Props: keeps the pose history, adds the blended poses to `fx_posed` (they take precedence in `set_props`) and
	/// fills `PROP_XF` with the blended placements. `posed` is moved in and stays readable through `posed()`.
	pub(super) fn props(&mut self, props: &[Prop], posed: HashMap<PoseKey, Posed>, fx_posed: &mut HashMap<usize, Posed>) {
		let ticks = self.ticks;
		Self::roll(ticks, &mut self.posed_last, &mut self.posed_before, posed);
		Self::roll(ticks, &mut self.fx_last, &mut self.fx_before, fx_posed.clone());
		let mut xf = vec![None; props.len()];
		if self.active {
			let t = self.alpha;
			for (pi, p) in props.iter().enumerate() {
				let Some(prev) = self.props_prev.get(pi) else { continue };
				let cur = PropXf::of(p);
				if !prop_blends(prev, &cur) {
					continue;
				}
				if prev.pos != cur.pos || prev.rot != cur.rot || prev.scale != cur.scale {
					xf[pi] = Some((blend_place(&place(prev.pos, prev.rot), &place(cur.pos, cur.rot), t), prev.scale + (cur.scale - prev.scale) * t));
				}
				let now = self.fx_last.get(&pi).or_else(|| self.posed_last.get(&cur.key));
				let was = self.fx_before.get(&pi).or_else(|| self.posed_before.get(&prev.key));
				let same = prev.key == cur.key && !self.fx_last.contains_key(&pi) && !self.fx_before.contains_key(&pi);
				if let (Some(a), Some(b), false) = (was, now, same) {
					if let Some(bl) = blend_posed(a, b, t) {
						fx_posed.insert(pi, bl);
					}
				}
			}
		}
		PROP_XF.with(|c| *c.borrow_mut() = xf);
	}

	/// The props' poses of this frame (moved in by `props`).
	/// The poses of the last `props` call, taken out (the next `props` call stores the new ones).
	pub(super) fn take_posed(&mut self) -> HashMap<PoseKey, Posed> {
		std::mem::take(&mut self.posed_last)
	}

	pub(super) fn posed(&self) -> &HashMap<PoseKey, Posed> {
		&self.posed_last
	}

	/// Pool sprites (`keys[k]` for `sprites[k]`, `Particles::sprites_keyed`): the position blends from the same slot's
	/// sprite of the tick before when its texture matches and it did not jump.
	pub(super) fn sprites(&mut self, sprites: &mut [particles::Sprite], keys: &[u32]) {
		if self.ticks > 0 {
			self.sprites_before = if self.ticks == 1 { std::mem::take(&mut self.sprites_last) } else { HashMap::new() };
			self.sprites_last = keys.iter().zip(sprites.iter()).map(|(&k, s)| (k, (s.pos, s.tex))).collect();
		}
		if !self.active {
			return;
		}
		for (s, k) in sprites.iter_mut().zip(keys) {
			if let Some(&(p, tex)) = self.sprites_before.get(k) {
				if tex == s.tex && near(p, s.pos) {
					s.pos = lerp3(p, s.pos, self.alpha);
				}
			}
		}
	}

	/// Scene playback tick to draw at: one tick behind the newest by `1 - alpha`, held on the tick before a camera
	/// cut so the camera and the actors never blend across it.
	pub(super) fn scene_tick(&self, sc: &scene::Scene, tick: f32) -> f32 {
		if !self.active {
			return tick;
		}
		scene_draw_tick(tick, self.alpha, |i| sc.camera.get(i).is_some_and(|k| k.cut))
	}
}

/// `tick - (1 - alpha)`, never before 0; a fraction that would blend into a cut key is dropped.
pub(super) fn scene_draw_tick(tick: f32, alpha: f32, cut: impl Fn(usize) -> bool) -> f32 {
	let mut t = (tick - (1.0 - alpha)).max(0.0);
	if (t - t.round()).abs() < 1e-3 {
		t = t.round();
	}
	if t.fract() > 0.0 && cut(t as usize + 1) {
		t = t.floor();
	}
	t
}

#[cfg(test)]
mod tests {
	use super::*;

	fn close(a: &[f32], b: &[f32]) -> bool {
		a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
	}

	#[test]
	fn angles_take_the_short_way() {
		assert!((lerp_angle(0.1, 0.3, 0.5) - 0.2).abs() < 1e-6);
		// 3.0 -> -3.0 crosses pi: halfway is pi, not 0.
		let h = lerp_angle(3.0, -3.0, 0.5);
		assert!((h.sin() - std::f32::consts::PI.sin()).abs() < 1e-5 && h.cos() < -0.99, "{h}");
		assert_eq!(lerp_angle(1.0, 2.0, 0.0), 1.0);
		assert!((lerp_angle(1.0, 2.0, 1.0) - 2.0).abs() < 1e-6);
	}

	#[test]
	fn placement_blend_slerps_and_lerps() {
		let a = place([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
		let b = place([2.0, 4.0, 0.0], [0.0, 0.0, 1.0]);
		let m = blend_place(&a, &b, 0.5);
		assert!(close(&m, &place([1.0, 2.0, 0.0], [0.0, 0.0, 0.5])), "{m:?}");
		// The ends are the placements themselves, including a general (x, y, z) rotation.
		let c = place([1.0, -2.0, 3.0], [0.3, -0.7, 2.9]);
		assert!(close(&blend_place(&a, &c, 1.0), &c));
		assert!(close(&blend_place(&c, &a, 0.0), &c));
		// Yaw 3.0 -> -3.0 goes through pi (shorter arc).
		let m = blend_place(&place([0.0; 3], [0.0, 0.0, 3.0]), &place([0.0; 3], [0.0, 0.0, -3.0]), 0.5);
		assert!(close(&m, &place([0.0; 3], [0.0, 0.0, std::f32::consts::PI])), "{m:?}");
	}

	#[test]
	fn pose_blend_and_its_snaps() {
		let a: Posed = (vec![[1.0, 0.0, 0.0], [0.0; 3], [0.0, 1.0, 0.0]], vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]]);
		let b: Posed = (vec![[2.0, 0.0, 0.0], [5.0, 0.0, 0.0], [0.0, 9.0, 0.0]], vec![[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]]);
		let (ps, ns) = blend_posed(&a, &b, 0.5).unwrap();
		assert_eq!(ps[0], [1.5, 0.0, 0.0]);
		let r = std::f32::consts::FRAC_1_SQRT_2;
		assert!(close(&ns[0], &[r, r, 0.0]));
		// Hidden before (origin sentinel): the new pose; a jump past SNAP_DIST: the new pose.
		assert_eq!(ps[1], [5.0, 0.0, 0.0]);
		assert_eq!(ps[2], [0.0, 9.0, 0.0]);
		// Different vertex counts: no blend.
		assert!(blend_posed(&a, &(vec![[0.0; 3]], vec![[0.0; 3]]), 0.5).is_none());
	}

	#[test]
	fn prop_snap_rules() {
		let key: PoseKey = (1, 0, 0, 0, 0, 0);
		let p = PropXf { pos: [0.0; 3], rot: [0.0; 3], scale: 1.0, shown: true, class: 7, key };
		assert!(prop_blends(&p, &PropXf { pos: [0.5, 0.0, 0.0], ..p }));
		assert!(!prop_blends(&p, &PropXf { pos: [SNAP_DIST + 0.1, 0.0, 0.0], ..p }), "teleport");
		assert!(!prop_blends(&PropXf { shown: false, ..p }, &p), "spawned / shown this tick");
		assert!(!prop_blends(&p, &PropXf { shown: false, ..p }), "despawned");
		assert!(!prop_blends(&p, &PropXf { class: 8, ..p }), "slot reused by another class");
	}

	#[test]
	fn scene_tick_lags_and_holds_before_cuts() {
		let no_cut = |_: usize| false;
		assert_eq!(scene_draw_tick(10.0, 1.0, no_cut), 10.0);
		assert!((scene_draw_tick(10.0, 0.25, no_cut) - 9.25).abs() < 1e-5);
		assert_eq!(scene_draw_tick(0.0, 0.5, no_cut), 0.0);
		// Key 10 is a cut: between ticks 9 and 10 the draw holds key 9; at 10 it shows the cut.
		let cut = |i: usize| i == 10;
		assert_eq!(scene_draw_tick(10.0, 0.25, cut), 9.0);
		assert_eq!(scene_draw_tick(10.0, 1.0, cut), 10.0);
		// Float noise around a whole tick is rounded, not blended into the next key.
		assert_eq!(scene_draw_tick(9.9999995, 1.0, cut), 10.0);
	}

	#[test]
	fn off_draws_the_newest_tick() {
		let mut s = Smooth::default();
		s.frame(false, 0.01);
		let mut sp = vec![particles::Sprite { pos: [1.0, 0.0, 0.0], size: 1.0, rot: 0.0, colour: [1.0; 4], tex: 3, additive: true, pool: None, flat: false }];
		s.ticks = 1;
		s.sprites(&mut sp, &[5]);
		s.ticks = 1;
		sp[0].pos = [2.0, 0.0, 0.0];
		s.sprites(&mut sp, &[5]);
		assert_eq!(sp[0].pos, [2.0, 0.0, 0.0]);
		// On: halfway between the two ticks' positions of slot key 5.
		s.frame(true, 0.01);
		s.ticks = 1;
		sp[0].pos = [3.0, 0.0, 0.0];
		s.sprites(&mut sp, &[5]);
		assert!((sp[0].pos[0] - 2.5).abs() < 1e-6, "{:?}", sp[0].pos);
		// A different texture in the same slot (a new effect) is not blended.
		s.ticks = 1;
		sp[0] = particles::Sprite { pos: [3.5, 0.0, 0.0], tex: 4, ..sp[0] };
		s.sprites(&mut sp, &[5]);
		assert_eq!(sp[0].pos, [3.5, 0.0, 0.0]);
	}
}
