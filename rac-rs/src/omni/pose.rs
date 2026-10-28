//! Pose overrides of the hero (ABI 1.4 `pose_set` ...; the loader's `omni.pose`): the HOST STORE. A mod re-poses joints
//! (mirror one arm onto the other, turn the head, ...) for the PICTURE only: nothing here writes game RAM.
//!
//! One ledger of ops per (moby, joint), kept on the worker thread's side in a `Mutex` static:
//! - `set` (pose_set) adds/replaces/removes ops with a weight that ramps `blend_ticks` per call (the loader's `blend`, in game ticks).
//!   Calling again for the same joint REPLACES the op; its weight and rotation continue from where they are, so a head turn renewed every
//!   tick is smooth. A `remove` flag (or `duration_ticks` running out) fades the op out over `blend_ticks`, then deletes it.
//! - `tick` (once per game field, from `omni::tick`) advances the ramps and applies the AUTO-CLEAR rules from the RAM words: level change,
//!   a cutscene / movie / vendor scene (ops dropped when it begins), the hero's death states (fade out), the hero cell no longer holding the
//!   posable hero (ops of other mobys dropped).
//! - `snapshot_for(hero)` turns the ops into `JointFx` records for `HeroParts.pose_ops` (retail/snapshot.rs `snap`); the renderer applies
//!   them in `Animated::local_fx_parts` (anim_pose.rs). Mirror ops become `JointFx::mirror_from`, `set_rotation` a `JointFx::set_rot`, and
//!   `offset_rotation` an ordinary mode-0 record (rotation only; its weight scales the angle). A `hold` is a rotation SET at the rotation a
//!   snapshot captured (pose_capture.rs): no record until the renderer has captured it; renewing a hold with no snapshot keeps its capture.
//!
//! Only the hero is posable: class 0 with 111 joints (`hero_joints`). The ABI callbacks are in pose_host.rs, the `pose_read` maths in pose_read.rs.

use super::cabi::OmniPoseOp;
use super::{pose_capture, pose_read};
use crate::anim::JointFx;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// The ops a moby may hold (the same cap as retail's record list in `joint_records`).
pub const MAX_OPS: usize = 64;
/// Error codes (include/omniloader.h): the others are -1 no level, -2 not posable.
pub const ERR_BAD_JOINT: i32 = -3;
pub const ERR_RANGE: i32 = -4;
pub const ERR_CAP: i32 = -5;
/// `OmniPoseOp.op` / `.flags`
pub const OP_MIRROR_FROM: u32 = 1;
pub const OP_SET_ROTATION: u32 = 2;
pub const OP_OFFSET_ROTATION: u32 = 3;
pub const OP_HOLD: u32 = 4;
pub const FLAG_REMOVE: u32 = 1;
/// The fade (ticks) of a "clear everything" that names none (`n = 0`, a mod stopped, the hero died).
pub const FADE_TICKS: u32 = 6;
/// Angles beyond this many radians-equivalent magnitude in a quaternion component are refused (a unit quaternion is within 1).
const Q_LIMIT: f32 = 1.5;
const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
	/// The joint takes the mirror image of this joint's final pose.
	Mirror(u16),
	/// The joint's rotation moves towards `q` (absolute, parent frame).
	Set,
	/// `q` is turned on top of the animation's rotation (parent frame).
	Offset,
	/// The joint keeps the rotation snapshot `id` captured (pose_capture.rs); id 0 = "a snapshot of this joint, to be requested when the batch
	/// is applied".
	Hold(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Op {
	pub kind: Kind,
	/// The rotation now and where it is heading (unit quaternions x y z w; unused by mirrors).
	pub q: [f32; 4],
	pub q_to: [f32; 4],
	/// The weight now and its target; `step` is how far the weight (and a fraction for `q`) moves per tick.
	pub w: f32,
	pub w_to: f32,
	pub step: f32,
	/// The fade-out length (ticks) when the op ends by itself or is removed.
	pub blend: u32,
	/// Ticks left before it fades out by itself.
	pub left: Option<u32>,
	/// Fading out: removed when `w` reaches 0.
	pub fading: bool,
}

#[derive(Default)]
pub struct Store {
	ops: BTreeMap<(u32, u32), Op>,
	/// The level the ops were set in (clear on change); `None` before the first tick.
	level: Option<i32>,
	hero: u32,
}

static STORE: Mutex<Store> = Mutex::new(Store { ops: BTreeMap::new(), level: None, hero: 0 });

fn lock() -> std::sync::MutexGuard<'static, Store> {
	STORE.lock().unwrap_or_else(|e| e.into_inner())
}

pub(super) fn norm(q: [f32; 4]) -> Option<[f32; 4]> {
	if !q.iter().all(|c| c.is_finite() && c.abs() <= Q_LIMIT) {
		return None;
	}
	let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
	(n > 0.25).then(|| q.map(|c| c / n))
}

/// `a` moved the fraction `t` of the way to `b` (shortest way, normalised).
pub fn nlerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
	let d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
	let s = if d < 0.0 { -1.0 } else { 1.0 };
	let v = [0, 1, 2, 3].map(|i| a[i] * (1.0 - t) + b[i] * s * t);
	let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2] + v[3] * v[3]).sqrt();
	if n > 0.0 {
		v.map(|c| c / n)
	} else {
		a
	}
}

impl Op {
	fn new(kind: Kind, q: [f32; 4], w: f32, blend: u32, duration: u32) -> Op {
		let step = if blend == 0 { 1.0 } else { 1.0 / blend as f32 };
		Op { kind, q, q_to: q, w: if blend == 0 { w } else { 0.0 }, w_to: w, step, blend, left: (duration > 0).then_some(duration), fading: false }
	}

	/// Starts the fade-out over `ticks` (0 = gone at once on the next `tick`).
	fn fade(&mut self, ticks: u32) {
		if self.fading {
			return;
		}
		self.fading = true;
		self.w_to = 0.0;
		self.step = if ticks == 0 { 1.0 } else { (self.w.max(0.01) / ticks as f32).max(1e-4) };
		self.left = None;
	}

	fn advance(&mut self) {
		let dw = self.w_to - self.w;
		self.w = if dw.abs() <= self.step { self.w_to } else { self.w + self.step.copysign(dw) };
		if self.q != self.q_to {
			self.q = nlerp(self.q, self.q_to, (self.step * 2.0).min(1.0));
		}
		if let Some(n) = self.left {
			if n <= 1 {
				let b = self.blend;
				self.fade(b);
			} else {
				self.left = Some(n - 1);
			}
		}
	}
}

impl Store {
	/// Applies one `pose_set` batch (ops already known to belong to the hero). Err: a refusal code, the batch is then not applied at all.
	pub fn set(&mut self, moby: u32, joints: u32, ops: &[OmniPoseOp]) -> Result<usize, i32> {
		let mut staged: Vec<(u32, Option<Op>, u32)> = Vec::with_capacity(ops.len());
		for o in ops {
			if o.joint >= joints {
				return Err(ERR_BAD_JOINT);
			}
			if o.flags & FLAG_REMOVE != 0 {
				staged.push((o.joint, None, o.blend_ticks));
				continue;
			}
			if !o.weight.is_finite() || !(0.0..=1.0).contains(&o.weight) {
				return Err(ERR_RANGE);
			}
			let (kind, q) = match o.op {
				OP_MIRROR_FROM if o.source < joints => (Kind::Mirror(o.source as u16), IDENTITY),
				OP_MIRROR_FROM => return Err(ERR_BAD_JOINT),
				OP_SET_ROTATION | OP_OFFSET_ROTATION => {
					let q = norm(o.q).ok_or(ERR_RANGE)?;
					(if o.op == OP_SET_ROTATION { Kind::Set } else { Kind::Offset }, q)
				}
				OP_HOLD => {
					if o.source != 0 {
						pose_capture::covers(o.source, o.joint)?;
						// Pinned now, not when the op is applied: a `hold(0)` earlier in this batch takes a new snapshot, which evicts the
						// oldest unpinned one, possibly this one.
						pose_capture::pin(o.source);
					}
					(Kind::Hold(o.source), IDENTITY)
				}
				_ => return Err(ERR_RANGE),
			};
			staged.push((o.joint, Some(Op::new(kind, q, o.weight, o.blend_ticks, o.duration_ticks)), 0));
		}
		let new: std::collections::BTreeSet<u32> = staged.iter().filter(|s| s.1.is_some()).map(|s| s.0).collect();
		let held = self.ops.range((moby, 0)..=(moby, u32::MAX)).filter(|(k, _)| !new.contains(&k.1)).count();
		if held + new.len() > MAX_OPS {
			return Err(ERR_CAP);
		}
		for (joint, op, fade) in &staged {
			match op {
				Some(op) => {
					let mut op = *op;
					if op.kind == Kind::Hold(0) {
						// A hold with no snapshot freezes the pose of now; renewed over a hold it keeps that capture.
						op.kind = match self.ops.get(&(moby, *joint)).map(|o| o.kind) {
							// ... unless that capture is gone (then the hold takes a new one instead of staying dead).
							Some(Kind::Hold(id)) if id != 0 && pose_capture::covers(id, *joint).is_ok() => Kind::Hold(id),
							_ => Kind::Hold(pose_capture::request(*joint, *joint + 1)),
						};
					}
					if let Kind::Hold(id) = op.kind {
						pose_capture::pin(id);
					}
					if let Some(old) = self.ops.get(&(moby, *joint)) {
						// Replacing keeps the weight (and the rotation of a rotation op) where it is: no pop, a renewed op is smooth.
						if op.blend > 0 {
							op.w = old.w;
						}
						if matches!(op.kind, Kind::Set | Kind::Offset) && old.kind == op.kind && op.blend > 0 {
							op.q = old.q;
						}
					}
					self.ops.insert((moby, *joint), op);
				}
				None => {
					if let Some(old) = self.ops.get_mut(&(moby, *joint)) {
						old.fade(*fade);
					}
				}
			}
		}
		Ok(staged.len())
	}

	/// `pose_set(n = 0)`: every op of `moby` fades out.
	pub fn fade_moby(&mut self, moby: u32, ticks: u32) {
		for (_, op) in self.ops.range_mut((moby, 0)..=(moby, u32::MAX)) {
			op.fade(ticks);
		}
	}

	/// One game tick: ramps, durations, fade-outs finishing.
	pub fn step(&mut self) {
		for op in self.ops.values_mut() {
			op.advance();
		}
		self.ops.retain(|_, op| !(op.fading && op.w <= 0.0));
		// The captures live holds use stay however many newer snapshots the mod takes.
		pose_capture::set_pinned(self.ops.values().filter_map(|op| if let Kind::Hold(id) = op.kind { Some(id).filter(|&i| i != 0) } else { None }));
	}

	pub fn len(&self) -> usize {
		self.ops.len()
	}

	pub fn is_empty(&self) -> bool {
		self.ops.is_empty()
	}

	/// The records for the renderer: weight 0 ops give nothing; order is by joint (stable, so the cache key is).
	pub fn records(&self, moby: u32) -> Vec<JointFx> {
		let mut out = Vec::new();
		for (&(_, joint), op) in self.ops.range((moby, 0)..=(moby, u32::MAX)) {
			let (j, w) = (joint as usize, op.w.clamp(0.0, 1.0));
			if w <= 0.0 {
				continue;
			}
			out.push(match op.kind {
				Kind::Mirror(src) => JointFx::mirror_from(j, src as usize, w),
				Kind::Set => JointFx::set_rot(j, op.q, w),
				Kind::Offset => JointFx::rot(j, nlerp(IDENTITY, op.q, w)),
				Kind::Hold(id) => match pose_capture::rotation(id, joint) {
					Some(q) => JointFx::set_rot(j, q, w),
					None => continue,
				},
			});
		}
		out
	}

	/// Auto-clear: drop everything at once.
	pub fn clear_all(&mut self) {
		self.ops.clear();
	}

	/// Auto-clear: the ops of every moby but `keep` go.
	fn retain_moby(&mut self, keep: u32) {
		self.ops.retain(|k, _| k.0 == keep);
	}

	pub fn counts(&self) -> (usize, Option<i32>) {
		(self.ops.len(), self.level)
	}
}

// ---------------------------------------------------------------------------------------------------------------------------------
// The RAM words the rules read.

const HERO_CELL: usize = 0x14_13d0;
const HERO_STATE: usize = 0x14_13d4;
/// Masks a game address to the 32 MB RAM (the pose files share this and `u32_at`).
pub(super) const MASK: u32 = 0x01ff_ffff;
/// Hero death states (death_respawn.md section 2; the same list as modcam.rs GATE_STATES' death part).
const DEATH_STATES: [u32; 10] = [0x3d, 0x47, 0x77, 0x7a, 0x7b, 0x7c, 0x7d, 0x7f, 0x80, 0x82];
/// Game modes (events.rs): movie, in-engine cutscene, vendor.
const SCENE_MODES: [u32; 3] = [1, 2, 5];
/// The hero's class id and joint count: the only body that can be posed.
const HERO_CLASS: u16 = 0;

pub(super) fn u32_at(ram: &[u8], a: usize) -> u32 {
	ram.get(a..a + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// The moby (masked address) if it is the hero cell's moby and a class-0 body: and its joint count (the class record's byte at +8).
pub fn hero_joints(ram: &[u8], moby: u32) -> Option<u32> {
	let hero = u32_at(ram, HERO_CELL) & MASK;
	if hero == 0 || moby & MASK != hero {
		return None;
	}
	let (class, hdr) = (ram.get(hero as usize + 0xa6..hero as usize + 0xa8)?, u32_at(ram, hero as usize + 0x24) & MASK);
	if u16::from_le_bytes([class[0], class[1]]) != HERO_CLASS || hdr == 0 {
		return None;
	}
	ram.get(hdr as usize + 8).map(|&n| u32::from(n)).filter(|&n| n > 0)
}

/// The hero cell's moby (masked address), 0 when the cell is empty.
pub(super) fn hero_addr(ram: &[u8]) -> u32 {
	u32_at(ram, HERO_CELL) & MASK
}

/// Whether the game is in a movie, cutscene or vendor scene (`mode` of events.rs).
pub(super) fn scene_mode(mode: u32) -> bool {
	SCENE_MODES.contains(&mode)
}

/// Whether the hero is in one of its death states.
pub fn hero_dead(ram: &[u8]) -> bool {
	DEATH_STATES.contains(&u32_at(ram, HERO_STATE))
}

/// Once per game field (`omni::tick`, after the mods ran this field is NOT yet over: the snapshot of the field comes later): the ramps move
/// one tick and the auto-clear rules run.
pub fn tick(ram: &[u8], level: i32, exit: bool, mode: u32) {
	let mut s = lock();
	let changed = s.level != Some(level) || exit;
	s.level = Some(level);
	pose_read::new_tick();
	if changed || SCENE_MODES.contains(&mode) {
		s.clear_all();
		// A level change drops the debug state too; a cutscene or vendor tick only the snapshots the cleared holds used.
		if changed {
			pose_capture::clear();
		} else {
			pose_capture::forget_snapshots();
		}
	}
	let hero = u32_at(ram, HERO_CELL) & MASK;
	s.hero = hero;
	s.retain_moby(hero);
	if hero_dead(ram) {
		s.fade_moby(hero, FADE_TICKS);
	}
	s.step();
}

/// A save was loaded, a new game starts or the mods were replaced: everything goes at once.
pub fn clear_all() {
	lock().clear_all();
	pose_capture::clear();
	pose_read::new_tick();
}

/// The records for `HeroParts.pose_ops` (retail/snapshot.rs; `posable` = class 0 with 111 joints, checked there): empty unless there are ops on `hero_addr`, and the
/// scene flag is off (a scene draws the hero from scene frames).
pub fn snapshot_for(hero_addr: u32, posable: bool, scene: bool) -> Vec<JointFx> {
	let s = lock();
	if s.is_empty() || scene || !posable {
		return Vec::new();
	}
	s.records(hero_addr & MASK)
}

/// `pose_set` on the store (pose_host.rs): the hero's joints are checked by the caller.
pub(super) fn with_store<R>(f: impl FnOnce(&mut Store) -> R) -> R {
	f(&mut lock())
}

/// The store, the capture slot and the read cache are process statics: the tests that use them take turns.
#[cfg(test)]
pub(super) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
#[path = "pose_tests.rs"]
mod tests;
