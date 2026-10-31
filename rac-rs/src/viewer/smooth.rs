//! [PC] Render interpolation for displays faster than the 50 Hz game (port extra, user request 2026-10-06).
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
pub(super) fn mirror_column(mut m: [f32; 16], mirror: bool) -> [f32; 16] {
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
		let (p, n) = blend_pn(pa, a.1[k], pb, b.1[k], t);
		ps.push(p);
		ns.push(n);
	}
	Some((ps, ns))
}

/// One vertex of `blend_posed` (position and normal from the two poses' entries): the hero marker's selective clipper
/// (`gpu/marker_clip.rs`) builds single vertices with it, bit for bit what the whole-pose blend gives.
pub(super) fn blend_pn(pa: [f32; 3], na: [f32; 3], pb: [f32; 3], nb: [f32; 3], t: f32) -> ([f32; 3], [f32; 3]) {
	let keep = pa == [0.0; 3] || pb == [0.0; 3] || !near(pa, pb);
	if keep {
		return (pb, nb);
	}
	let n = lerp3(na, nb, t);
	let l = dist2([0.0; 3], n).sqrt();
	(lerp3(pa, pb, t), if l > 1e-6 { n.map(|c| c / l) } else { nb })
}

/// A prop's placement before the newest tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PropXf {
	pub pos: V3,
	pub rot: V3,
	pub mirror: bool,
	pub scale: f32,
	/// Drawn, or a retail moby culled for the game camera alone (`Prop::cast`: the sun shadow pass and Culling Off pose it, so it
	/// blends too; before, such a crane stepped once per field in the shadow and with Culling Off).
	pub shown: bool,
	pub class: u32,
	pub key: PoseKey,
}

impl PropXf {
	pub(super) fn of(p: &Prop) -> PropXf {
		PropXf { pos: p.pos, rot: p.rot, mirror: p.mirror, scale: p.scale, shown: p.shown || p.cast, class: p.class_id, key: pose_key(p) }
	}
}

/// Whether a prop may be drawn blended from `prev` to its current state.
pub(super) fn prop_blends(prev: &PropXf, cur: &PropXf) -> bool {
	prev.shown && cur.shown && prev.class == cur.class && prev.mirror == cur.mirror && near(prev.pos, cur.pos)
}

/// Moby classes drawn on the hero: Clank 601, the back packs 607 / 608 / 609, the head items 433 / 1289 / 1290 (hero item table rows
/// 1-7, collision_and_gameplay.md), the class 1204 bulb mesh, the Pyrocitor's pilot flame 179 (on the held gun) and the Magneboots 173
/// (on the feet); only while within `ATTACH_REACH` of him (strafe.rs `turn_parts`).
const ATTACHED: [u32; 10] = [601, 607, 608, 609, 433, 1289, 1290, ATTACH_BULB, PYRO_PILOT, MAGNEBOOTS];
/// The Magneboots, one moby per foot (carried with the drawn legs, `PoseFix::place_leg`).
const MAGNEBOOTS: u32 = 173;
/// The Pyrocitor's pilot flame moby (on the held gun's nozzle).
const PYRO_PILOT: u32 = 179;
const ATTACH_BULB: u32 = 1204;
const ATTACH_REACH: f32 = 1.5;
/// How close to a hero part's origin a hero glow counts as sitting on it (`hero_glow`; Ratchet is about 1.2 tall, the glove's bomb
/// sits 0.13 from the glove: runs/strafe.log 2026-10-24).
const GLOW_ON_PART: f32 = 0.15;

/// The hero's model frame as a column-major placement: feet `pos`, columns = the hero rows of BRIDGE-75 (yaw only when untilted).
pub(super) fn hero_frame(pos: V3, yaw: f32, tilt: [f32; 2]) -> [f32; 16] {
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
	/// [PC] Modern controls: the hero parts' move from retail's pose onto the drawn one (retail/strafe.rs `PoseFix`) of the newest
	/// tick and the one before, and the (drawn, ticked) hero frames `attach` used this frame (for `hero_point`, also without a fix).
	fix_now: Option<crate::retail::strafe::PoseFix>,
	fix_was: Option<crate::retail::strafe::PoseFix>,
	fix_frames: Option<([f32; 16], [f32; 16])>,
	/// [PC] Modern controls: the held Pyrocitor's move from retail's held basis onto the drawn hand (handler/hero_rows.rs
	/// `pyro_basis`, hero model space, 3x4 row major) of the newest tick and the one before; its pilot moby rides with it.
	gun_now: Option<[f32; 12]>,
	gun_was: Option<[f32; 12]>,
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
	/// The pose of the last `hero` call had the hero collapsed with only a loose wrench kept (`hero_pose::collapse_outside`).
	hero_collapsed: bool,
	/// Pose generation numbers (herocache `next_gen`) of `hero_last` / `hero_before`, and what `hero` handed out: (pose as returned,
	/// blend source). 0 = unknown, e.g. a pose the CPU blended.
	gen_last: u64,
	gen_before: u64,
	hero_gens: (u64, u64),
	posed_last: HashMap<PoseKey, Posed>,
	posed_before: HashMap<PoseKey, Posed>,
	/// [PC] perf: the joint-record props' skinnings of the last frame (handler/fx_skin.rs).
	pub(super) fx_skins: super::handler::fx_skin::FxSkins,
	fx_last: HashMap<usize, std::sync::Arc<Posed>>,
	fx_before: HashMap<usize, std::sync::Arc<Posed>>,
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
		self.alpha = crate::viewer::test_clock::alpha((accum / crate::retail::disc::field_dt()).clamp(0.0, 1.0));
		self.active = on;
	}

	/// Pose history step; `ticks == 1` keeps the previous pose, more than one tick per frame drops it (snap).
	fn roll<K: std::hash::Hash + Eq, V>(ticks: u32, last: &mut HashMap<K, V>, before: &mut HashMap<K, V>, now: HashMap<K, V>) {
		if ticks > 0 {
			*before = if ticks == 1 { std::mem::take(last) } else { HashMap::new() };
		}
		*last = now;
	}

	/// Called before `hero` with whether this frame's pose is the collapsed one: a switch between the collapsed and the full pose is drawn
	/// unblended (a vertex blend of the two would smear the hero's vertices out of the origin for a display frame).
	pub(super) fn note_hero_collapsed(&mut self, collapsed: bool) {
		if self.hero_collapsed != collapsed {
			self.hero_collapsed = collapsed;
			self.hero_cut = true;
		}
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
	/// Called with the final drawn `at` / `tilt` before the glows and `props`; fills `attached`. `parts` = more props on the hero
	/// this tick (Modern strafe, retail/strafe.rs `turn_parts`: the bomb in the Bomb Glove, the Glove of Doom capsule). With a Modern-strafe pose fix
	/// (`set_pose_fix`) `rel` is also carried from retail's pose onto the drawn one, then also unblended (Smoothing off). Otherwise
	/// not blended (Smoothing off, a placement other than the plain gameplay one): nothing, the props are drawn as the game ticked them.
	pub(super) fn attach(&mut self, props: &[Prop], parts: &[usize], at: Option<(V3, f32)>, tilt: [f32; 2]) {
		self.attached.clear();
		self.fix_frames = None;
		let (Some(at), Some(now)) = (at, self.hero_now) else { return };
		let d = hero_frame(at.0, at.1, tilt);
		let blended = self.active && self.drawn == Some((at.0, at.1, tilt));
		if !blended && ((self.fix_now.is_none() && self.gun_now.is_none()) || d != now) {
			return;
		}
		let was = self.hero_was.unwrap_or(now);
		let blend = blended && self.hero_before.is_some();
		let t = self.alpha;
		let fixed = |f: &Option<crate::retail::strafe::PoseFix>, class: u32, rel: [f32; 16]| f.as_ref().map_or(rel, |f| if class == MAGNEBOOTS { f.place_leg(rel) } else { f.place(rel) });
		let gun = |g: &Option<[f32; 12]>, class: u32, rel: [f32; 16]| g.filter(|_| class == PYRO_PILOT).map_or(rel, |m| crate::retail::strafe::place34(&m, rel));
		self.fix_frames = Some((d, now));
		let near_hero = |h: &[f32; 16], p: V3| (p[0] - h[12]).hypot(p[1] - h[13]) < ATTACH_REACH && (p[2] - h[14]).abs() < 3.0;
		self.attached.resize(props.len(), None);
		for (pi, p) in props.iter().enumerate() {
			if !p.shown || !(ATTACHED.contains(&p.class_id) || parts.contains(&pi)) || !near_hero(&now, p.pos) {
				continue;
			}
			let rel_now = gun(&self.gun_now, p.class_id, fixed(&self.fix_now, p.class_id, rel_place(&now, &place(p.pos, p.rot))));
			let prev = self.props_prev.get(pi).filter(|pv| blend && prop_blends(pv, &PropXf::of(p)) && near_hero(&was, pv.pos));
			let (rel, scale) = match prev {
				Some(pv) => (blend_place(&gun(&self.gun_was, p.class_id, fixed(&self.fix_was, p.class_id, rel_place(&was, &place(pv.pos, pv.rot)))), &rel_now, t), pv.scale + (p.scale - pv.scale) * t),
				None => (rel_now, p.scale),
			};
			self.attached[pi] = Some((mirror_column(compose(&d, &rel), p.mirror), scale));
		}
	}

	/// The pose fix of the newest tick (frame_hero.rs, once per computed hero pose; None while the strafe draw is off).
	pub(super) fn set_pose_fix(&mut self, fix: Option<crate::retail::strafe::PoseFix>) {
		self.fix_was = std::mem::replace(&mut self.fix_now, fix);
		self.gun_was = self.gun_now.take();
	}

	/// The newest tick's held-Pyrocitor move (`gun_now`), after `set_pose_fix` of the same pose.
	pub(super) fn put_gun_fix(&mut self, fix: Option<[f32; 12]>) {
		self.gun_now = fix;
	}

	/// A point retail placed on the hero this tick (a hero glow, as the game ticked it), as this frame draws it: kept where it sits
	/// on the ticked hero and carried with his drawn placement (and the pose fix, if any) like the props in `attach`. None when
	/// `attach` draws the hero neither blended nor fixed.
	fn hero_point(&self, p: V3, gun: bool) -> Option<V3> {
		let (d, now) = self.fix_frames?;
		let w = [p[0] - now[12], p[1] - now[13], p[2] - now[14]];
		let l = std::array::from_fn(|k| now[k * 4] * w[0] + now[k * 4 + 1] * w[1] + now[k * 4 + 2] * w[2]);
		let m = self.fix_now.as_ref().map_or(l, |f| f.point(l));
		let m = self.gun_now.filter(|_| gun).map_or(m, |g| crate::retail::strafe::mul34(&g, m, 1.0));
		Some(std::array::from_fn(|i| d[12 + i] + d[i] * m[0] + d[4 + i] * m[1] + d[8 + i] * m[2]))
	}

	/// The placement `attach` made for prop `pi` this frame, if it is hero-attached and the hero is blended.
	pub(super) fn attached_xf(&self, pi: usize) -> Option<([f32; 16], f32)> {
		self.attached.get(pi).copied().flatten()
	}

	/// A point retail placed on hero part `pi` this tick (`raw`, as the game ticked it), as this frame draws it: kept at its offset
	/// from the part and carried with the part's drawn placement (`attach`). None when `attach` does not draw that part.
	fn carry(&self, props: &[Prop], pi: usize, raw: V3) -> Option<V3> {
		let p = props.get(pi).filter(|p| p.shown && !p.mirror)?;
		let (m, _) = self.attached_xf(pi)?;
		let r = place(p.pos, p.rot);
		let d = [raw[0] - p.pos[0], raw[1] - p.pos[1], raw[2] - p.pos[2]];
		let off: V3 = std::array::from_fn(|k| r[k * 4] * d[0] + r[k * 4 + 1] * d[1] + r[k * 4 + 2] * d[2]);
		Some(std::array::from_fn(|i| m[12 + i] + m[i] * off[0] + m[4 + i] * off[1] + m[8 + i] * off[2]))
	}

	/// A hero glow as this frame draws it (`raw` = as the game ticked it, `blended` = as the field blend gives it): carried with the
	/// hero part it sits on (`carry`: Clank's bulb glow, `bulb`, on the bulb mesh; any other on the nearest part `attach` draws within
	/// `GLOW_ON_PART`), else from where the game ticked it with the drawn hero (`hero_point`); as blended only when the hero is not
	/// drawn blended. Before, the field blend of the glow and the hero's own blend added up (or, for a glow whose colour changes
	/// every field such as the Pyrocitor's pilot light, the glow was not blended at all): the pilot light and the thruster pack's
	/// flames trailed or led the drawn Ratchet while he turned or strafed. A glow of the held Pyrocitor (`gun`) rides the drawn gun
	/// (`gun_now`) from where the game ticked it: its pilot moby is hidden while it does not fire.
	pub(super) fn hero_glow(&self, props: &[Prop], raw: V3, blended: V3, bulb: bool, gun: bool) -> V3 {
		let dist = |pi: &usize| (0..3).map(|k| (props[*pi].pos[k] - raw[k]).powi(2)).sum::<f32>();
		let mut on = (0..props.len()).filter(|&pi| !(gun && self.gun_now.is_some()) && self.attached_xf(pi).is_some() && if bulb { props[pi].class_id == ATTACH_BULB } else { dist(&pi) < GLOW_ON_PART * GLOW_ON_PART });
		let part = if bulb { on.next() } else { on.min_by(|a, b| dist(a).total_cmp(&dist(b))) };
		part.and_then(|pi| self.carry(props, pi, raw)).or_else(|| self.hero_point(raw, gun)).unwrap_or(blended)
	}

	/// The newest tick's strafe pose fix (None while the strafe draw leaves retail's pose as it is).
	pub(super) fn pose_fix_now(&self) -> Option<&crate::retail::strafe::PoseFix> {
		self.fix_now.as_ref()
	}

	/// The effect pool's sprites (`fx`, field-blended) into `out`; a sprite a hero part owns (`RetailFx::sprite_owners`, the Glove
	/// of Doom capsule's glow) is carried with that part's drawn placement (`carry`, from the same sprite of `tick`, the unblended
	/// field; `prop` = the prop index of a moby address). Before, the capsule's glow sat off the drawn glove while strafing.
	pub(super) fn pool_sprites(&self, props: &[Prop], fx: &crate::retail::RetailFx, tick: Option<&crate::retail::RetailFx>, prop: impl Fn(u32) -> Option<usize>, out: &mut Vec<particles::Sprite>) {
		out.extend(fx.sprites.iter().enumerate().map(|(i, s)| {
			let owned = fx.sprite_owners.get(i).filter(|o| fx.hero_parts.contains(o)).and_then(|&o| prop(o));
			let raw = tick.and_then(|t| t.sprites.get(i)).map_or(s.pos, |t| t.pos);
			owned.and_then(|pi| self.carry(props, pi, raw)).map_or(*s, |pos| particles::Sprite { pos, ..*s })
		}));
	}

	/// Props: keeps the pose history, adds the blended poses to `fx_posed` (they take precedence in `set_props`) and
	/// fills `PROP_XF` with the blended placements. `posed` is moved in and stays readable through `posed()`. `blend[pi]` false = the
	/// prop's pose is kept in the history but not blended for this frame (a camera-culled moby the sun shadow pass cannot take yet,
	/// handler/sun_gate.rs); the poses of the props that do blend are blended on the worker pool.
	pub(super) fn props(&mut self, props: &[Prop], posed: HashMap<PoseKey, Posed>, fx_posed: &mut HashMap<usize, std::sync::Arc<Posed>>, blend: &[bool]) {
		let ticks = self.ticks;
		Self::roll(ticks, &mut self.posed_last, &mut self.posed_before, posed);
		Self::roll(ticks, &mut self.fx_last, &mut self.fx_before, fx_posed.clone());
		let mut xf = vec![None; props.len()];
		if self.active {
			let t = self.alpha;
			let mut jobs: Vec<(usize, &Posed, &Posed)> = Vec::new();
			for (pi, p) in props.iter().enumerate() {
				let Some(prev) = self.props_prev.get(pi) else { continue };
				let cur = PropXf::of(p);
				if !prop_blends(prev, &cur) {
					continue;
				}
				if prev.pos != cur.pos || prev.rot != cur.rot || prev.scale != cur.scale {
					xf[pi] = Some((mirror_column(blend_place(&place(prev.pos, prev.rot), &place(cur.pos, cur.rot), t), cur.mirror), prev.scale + (cur.scale - prev.scale) * t));
				}
				let now = self.fx_last.get(&pi).map(|a| &**a).or_else(|| self.posed_last.get(&cur.key));
				let was = self.fx_before.get(&pi).map(|a| &**a).or_else(|| self.posed_before.get(&prev.key));
				let same = prev.key == cur.key && !self.fx_last.contains_key(&pi) && !self.fx_before.contains_key(&pi);
				if let (Some(a), Some(b), false) = (was, now, same) {
					if blend.get(pi).copied().unwrap_or(true) {
						jobs.push((pi, a, b));
					}
				}
			}
			let blended: Vec<Option<Posed>> = {
				use rayon::prelude::*;
				if jobs.len() < 3 { jobs.iter().map(|&(_, a, b)| blend_posed(a, b, t)).collect() } else { jobs.par_iter().map(|&(_, a, b)| blend_posed(a, b, t)).collect() }
			};
			for ((pi, _, _), bl) in jobs.iter().zip(blended) {
				if let Some(bl) = bl {
					self.fx_skins.born.fx.remove(pi); // [PC] perf: a pose blended now has no token (handler/pose_born.rs)
					fx_posed.insert(*pi, std::sync::Arc::new(bl));
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
