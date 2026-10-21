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

/// A mirrored prop's blended rotation with model Y reflected again (`place_mirrored`: column 1 negated).
fn mirror_column(mut m: [f32; 16], mirror: bool) -> [f32; 16] {
	if mirror {
		for v in &mut m[4..8] {
			*v = -*v;
		}
	}
	m
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
	pub mirror: bool,
	pub scale: f32,
	pub shown: bool,
	pub class: u32,
	pub key: PoseKey,
}

impl PropXf {
	pub(super) fn of(p: &Prop) -> PropXf {
		PropXf { pos: p.pos, rot: p.rot, mirror: p.mirror, scale: p.scale, shown: p.shown, class: p.class_id, key: pose_key(p) }
	}
}

/// Whether a prop may be drawn blended from `prev` to its current state.
pub(super) fn prop_blends(prev: &PropXf, cur: &PropXf) -> bool {
	prev.shown && cur.shown && prev.class == cur.class && prev.mirror == cur.mirror && near(prev.pos, cur.pos)
}

/// Moby classes drawn on the hero: Clank 601, the back packs 607 / 608 / 609, the head items 433 / 1289 / 1290 (hero item table rows
/// 1-7, collision_and_gameplay.md) and the class 1204 bulb mesh; only while within `ATTACH_REACH` of him (strafe.rs `turn_parts`).
const ATTACHED: [u32; 8] = [601, 607, 608, 609, 433, 1289, 1290, ATTACH_BULB];
const ATTACH_BULB: u32 = 1204;
const ATTACH_REACH: f32 = 1.5;

/// The hero's model frame as a column-major placement: feet `pos`, columns = the hero rows of BRIDGE-75 (yaw only when untilted).
fn hero_frame(pos: V3, yaw: f32, tilt: [f32; 2]) -> [f32; 16] {
	let r = if tilt == [0.0; 2] {
		let (s, c) = yaw.sin_cos();
		[[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]
	} else {
		particles::euler_m([tilt[0], tilt[1], yaw])
	};
	[r[0][0], r[0][1], r[0][2], 0.0, r[1][0], r[1][1], r[1][2], 0.0, r[2][0], r[2][1], r[2][2], 0.0, pos[0], pos[1], pos[2], 1.0]
}

/// `inverse(h) * m` of two rigid column-major placements: `m` in the frame of `h`.
fn rel_place(h: &[f32; 16], m: &[f32; 16]) -> [f32; 16] {
	let mut o = [0.0; 16];
	o[15] = 1.0;
	for j in 0..3 {
		for k in 0..3 {
			o[j * 4 + k] = h[k * 4] * m[j * 4] + h[k * 4 + 1] * m[j * 4 + 1] + h[k * 4 + 2] * m[j * 4 + 2];
		}
	}
	let d = [m[12] - h[12], m[13] - h[13], m[14] - h[14]];
	for k in 0..3 {
		o[12 + k] = h[k * 4] * d[0] + h[k * 4 + 1] * d[1] + h[k * 4 + 2] * d[2];
	}
	o
}

/// `a * b` of two rigid column-major placements.
fn compose(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
	let mut o = [0.0; 16];
	o[15] = 1.0;
	for j in 0..3 {
		for i in 0..3 {
			o[j * 4 + i] = a[i] * b[j * 4] + a[4 + i] * b[j * 4 + 1] + a[8 + i] * b[j * 4 + 2];
		}
	}
	for i in 0..3 {
		o[12 + i] = a[12 + i] + a[i] * b[12] + a[4 + i] * b[13] + a[8 + i] * b[14];
	}
	o
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
	/// Hero-attached props (`attach`): the hero's placement as the game ticked it (before / after the newest tick), what `hero`
	/// drew this frame (feet, yaw, tilt; None = not blended), and the placements `attach` made, per prop slot.
	hero_was: Option<[f32; 16]>,
	hero_now: Option<[f32; 16]>,
	drawn: Option<(V3, f32, [f32; 2])>,
	attached: Vec<Option<([f32; 16], f32)>>,
	/// Skinned poses: the last computed (state after the newest tick seen) and the one before it.
	hero_last: Option<Posed>,
	hero_before: Option<Posed>,
	/// [PC] perf (lane R2): with the GPU hero (gpuhero.rs) `hero` leaves the pose unblended and this holds the blend factor for
	/// `Gpu::set_marker` (the blend source is `hero_before`).
	hero_blend: Option<f32>,
	/// Set during a tick whose hero pose jumps on purpose (Modern controls, retail/strafe.rs: the run clip flipped between forward
	/// and reversed, or the leg twist jumped): the next `hero` call draws the new pose unblended. A vertex blend of a forward and a
	/// reversed run frame (legs swapped, lower body turned) pulled the legs through each other for a display frame.
	pub hero_cut: bool,
	/// Pose generation numbers (herocache `next_gen`) of `hero_last` / `hero_before`, and what `hero` handed out: (pose as returned,
	/// blend source). 0 = unknown, e.g. a pose the CPU blended.
	gen_last: u64,
	gen_before: u64,
	hero_gens: (u64, u64),
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
		// The field time of the disc (USA 59.94 Hz: PAL_DT left alpha at 0.83 and jumped back every field).
		self.alpha = (accum / crate::retail::disc::field_dt()).clamp(0.0, 1.0);
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
	pub(super) fn hero(&mut self, game: &Game, at: Option<(V3, f32)>, skinned: Option<Posed>, pgen: u64) -> (Option<(V3, f32)>, Option<Posed>, [f32; 2]) {
		if self.ticks > 0 {
			let cut = std::mem::take(&mut self.hero_cut);
			let keep = self.ticks == 1 && !cut;
			self.hero_before = if keep { self.hero_last.take() } else { None };
			self.gen_before = if keep { self.gen_last } else { 0 };
			self.hero_last = skinned.clone();
			self.gen_last = pgen;
		}
		let tilt_now = game.player.body_tilt;
		self.hero_now = Some(hero_frame(game.player.pos, game.player.yaw, tilt_now));
		self.hero_was = self.hero_prev.map(|(p, y)| hero_frame(p, y, self.hero_tilt_prev));
		self.drawn = None;
		self.hero_blend = None;
		self.hero_gens = (pgen, self.gen_before);
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
				let drawn = (lerp3(prev.0, a.0, t), lerp_angle(prev.1, a.1, t));
				self.drawn = Some((drawn.0, drawn.1, tilt));
				Some(drawn)
			}
			(a, _) => a,
		};
		let skinned = match (&self.hero_before, skinned) {
			(Some(_), Some(s)) if super::gpuhero::gpu_hero_on() => {
				self.hero_blend = Some(t);
				Some(s)
			}
			(Some(b), Some(s)) => match blend_posed(b, &s, t) {
				// Blended on the CPU: a new pose every frame, no generation.
				Some(p) => {
					self.hero_gens.0 = 0;
					Some(p)
				}
				None => Some(s),
			},
			(_, s) => s,
		};
		(at, skinned, tilt)
	}

	/// Generation numbers of the pose `hero` returned and of `hero_before` (0 = unknown).
	pub(super) fn hero_gens(&self) -> (u64, u64) {
		self.hero_gens
	}

	pub(super) fn hero_before(&self) -> Option<&Posed> {
		self.hero_before.as_ref()
	}

	/// The GPU hero's blend: the pose before the newest tick and the factor (None = draw the pose as given).
	pub(super) fn hero_blend(&self) -> Option<(&Posed, f32)> {
		self.hero_before.as_ref().zip(self.hero_blend)
	}

	/// Hero-attached props (Clank, the back pack, the head items, the bulb mesh) of this display frame, drawn from their placement
	/// RELATIVE to the hero: per tick `rel = inverse(hero as the game ticked him) * prop`, and the prop is drawn at the drawn hero
	/// (blended, Modern-strafe turned) times `rel`, blended between the two ticks only while the hero's own pose blends (so Clank sits at
	/// the joint of the pose that is drawn). They stay glued to Ratchet whatever the smoothing, the speed or the turn: the prop path
	/// (blend in world space from `props_prev`) and the hero path (own pose blend, dropped when a frame runs several ticks) disagreed.
	/// Called with the final drawn `at` / `tilt` before the glows and `props`; fills `attached`. Not blended (Smoothing off, a
	/// placement other than the plain gameplay one): nothing, so the props are drawn as the game ticked them.
	pub(super) fn attach(&mut self, props: &[Prop], at: Option<(V3, f32)>, tilt: [f32; 2]) {
		self.attached.clear();
		if !self.active {
			return;
		}
		let (Some(drawn), Some(at), Some(now)) = (self.drawn, at, self.hero_now) else { return };
		if drawn != (at.0, at.1, tilt) {
			return;
		}
		let was = self.hero_was.unwrap_or(now);
		let blend = self.hero_before.is_some();
		let t = self.alpha;
		let d = hero_frame(at.0, at.1, tilt);
		let near_hero = |h: &[f32; 16], p: V3| (p[0] - h[12]).hypot(p[1] - h[13]) < ATTACH_REACH && (p[2] - h[14]).abs() < 3.0;
		self.attached.resize(props.len(), None);
		for (pi, p) in props.iter().enumerate() {
			if !p.shown || !ATTACHED.contains(&p.class_id) || !near_hero(&now, p.pos) {
				continue;
			}
			let rel_now = rel_place(&now, &place(p.pos, p.rot));
			let prev = self.props_prev.get(pi).filter(|pv| blend && prop_blends(pv, &PropXf::of(p)) && near_hero(&was, pv.pos));
			let (rel, scale) = match prev {
				Some(pv) => (blend_place(&rel_place(&was, &place(pv.pos, pv.rot)), &rel_now, t), pv.scale + (p.scale - pv.scale) * t),
				None => (rel_now, p.scale),
			};
			self.attached[pi] = Some((mirror_column(compose(&d, &rel), p.mirror), scale));
		}
	}

	/// The placement `attach` made for prop `pi` this frame, if it is hero-attached and the hero is blended.
	pub(super) fn attached_xf(&self, pi: usize) -> Option<([f32; 16], f32)> {
		self.attached.get(pi).copied().flatten()
	}

	/// Clank's bulb glow as this frame draws it: `raw` (the glow as the game ticked it) kept at its offset from the bulb mesh
	/// (class 1204) and carried along with the mesh's drawn placement (`attach`). None when the mesh is not drawn by `attach`.
	pub(super) fn bulb_glow(&self, props: &[Prop], raw: V3) -> Option<V3> {
		let (pi, p) = props.iter().enumerate().find(|&(pi, p)| p.class_id == ATTACH_BULB && p.shown && !p.mirror && self.attached_xf(pi).is_some())?;
		let (m, _) = self.attached_xf(pi)?;
		let r = place(p.pos, p.rot);
		let d = [raw[0] - p.pos[0], raw[1] - p.pos[1], raw[2] - p.pos[2]];
		let off: V3 = std::array::from_fn(|k| r[k * 4] * d[0] + r[k * 4 + 1] * d[1] + r[k * 4 + 2] * d[2]);
		Some(std::array::from_fn(|i| m[12 + i] + m[i] * off[0] + m[4 + i] * off[1] + m[8 + i] * off[2]))
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
					xf[pi] = Some((mirror_column(blend_place(&place(prev.pos, prev.rot), &place(cur.pos, cur.rot), t), cur.mirror), prev.scale + (cur.scale - prev.scale) * t));
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
		for (pi, a) in self.attached.iter().enumerate() {
			if a.is_some() && pi < xf.len() {
				xf[pi] = *a;
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
	fn hero_attached_placement_follows_the_hero() {
		// A prop beside a turned and tilted hero: its placement in the hero's frame composed with the same frame gives it back,
		// and with the hero turned about the feet it keeps its distance from the feet and its hero-relative placement.
		let h = hero_frame([1.0, 2.0, 0.5], 0.7, [0.1, -0.2]);
		let m = place([1.1, 2.2, 1.3], [0.0, 0.1, 0.9]);
		let rel = rel_place(&h, &m);
		assert!(close(&compose(&h, &rel), &m));
		let turned = hero_frame([1.0, 2.0, 0.5], 0.7 + 0.5, [0.1, -0.2]);
		let m2 = compose(&turned, &rel);
		let dist = |a: &[f32; 16]| ((a[12] - 1.0).powi(2) + (a[13] - 2.0).powi(2) + (a[14] - 0.5).powi(2)).sqrt();
		assert!((dist(&m2) - dist(&m)).abs() < 1e-5);
		assert!(close(&rel_place(&turned, &m2), &rel));
	}

	#[test]
	fn prop_snap_rules() {
		let key: PoseKey = (1, 0, 0, 0, 0, 0);
		let p = PropXf { pos: [0.0; 3], rot: [0.0; 3], mirror: false, scale: 1.0, shown: true, class: 7, key };
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
