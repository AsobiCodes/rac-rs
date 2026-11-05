//! Snapshots, holds and the debug dots of the pose overrides (ABI 1.4 `pose_snapshot`, the `hold` op, `pose_debug`): the "last rendered hero
//! pose" slot between the render thread and the worker (game) thread.
//!
//! A hold freezes a joint at the rotation it had when the op was set. The rotation is the game's own pose of that moment (the animation,
//! retail's joint records and the modern-controls leg twist, never a mod's overrides), so the renderer captures it: `pose_snapshot` (and a
//! `hold` with no snapshot, which is a snapshot of its own joint) only REGISTERS a request and returns its id; the render thread's next frame
//! (viewer/handler/pose_service.rs) evaluates the base local pose once and `fulfil`s every waiting request. The store (pose.rs) turns a hold into
//! a rotation SET record once its snapshot holds the joint, so a hold takes effect one frame (+ one tick) after it is set; nothing touches game
//! RAM, nothing here depends on a lock order but STORE then SLOT (the render thread takes only SLOT).
//!
//! The same service publishes the world position of every joint of the DRAWN pose (`publish_joints`) while `pose_debug` is on; the worker
//! projects them with the game's camera each tick (`debug_cmds`, `omni::tick`) into a draw list of dots and names that the renderer draws as the
//! last 2D layer (draw.rs `submit_debug`).
//!
//! `pose_read` of a joint the game does not keep a matrix for (every joint but the nine attach points in game RAM, pose_read.rs) is served from
//! the same slot: while a mod reads, the render thread publishes the world frame of every joint of the GAME pose (`publish_world`: the base
//! animation, no mod override) once per frame. The worker sees the LAST rendered frame, i.e. one frame behind the tick (and, as the render
//! thread is not locked to the game, it can be a frame or two older); pose_read.rs copies the slot once per tick so one tick sees one frame.
//!
//! Cost with no mod using any of it: three atomic loads per frame (`pending`, `debug_on`, `reads_on`).

use super::cabi::OmniDrawCmd;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Snapshots kept (the loader forgets the oldest of 256 as well).
pub const MAX_SNAPSHOTS: usize = 256;
/// Error codes (include/omniloader.h): -1 no level, -2 not posable, -3 bad joint, -4 out of range.
pub const ERR_BAD_JOINT: i32 = -3;
pub const ERR_RANGE: i32 = -4;

/// The rotations of joints `first..last` once the renderer captured them (`None`: still waiting).
struct Snap {
	first: u32,
	last: u32,
	q: Option<Vec<[f32; 4]>>,
}

#[derive(Default)]
struct Slot {
	next: u32,
	snaps: BTreeMap<u32, Snap>,
	/// World positions of the drawn pose's joints (render thread writes, worker reads), empty when there is none.
	joints: Vec<[f32; 3]>,
	/// World frames of the GAME pose's joints (three basis rows then the position, game world), and the hero position of the same frame.
	world: Vec<[f32; 12]>,
	hero_pos: [f32; 3],
	/// Snapshot ids a live `hold` op uses: the 256 limit never evicts these (the store sets them, pose.rs `Store::step`).
	pinned: BTreeSet<u32>,
}

static SLOT: Mutex<Slot> = Mutex::new(Slot { next: 0, snaps: BTreeMap::new(), joints: Vec::new(), world: Vec::new(), hero_pos: [0.0; 3], pinned: BTreeSet::new() });
/// A snapshot waits for the renderer.
static PENDING: AtomicBool = AtomicBool::new(false);
static DEBUG: AtomicBool = AtomicBool::new(false);
/// A mod reads joints the game keeps no matrix for: the renderer publishes the game pose's world frames.
static READS: AtomicBool = AtomicBool::new(false);

fn lock() -> std::sync::MutexGuard<'static, Slot> {
	SLOT.lock().unwrap_or_else(|e| e.into_inner())
}

/// Registers a snapshot of joints `first..last` (the caller checked the range); its id (> 0). The renderer captures it on its next frame.
pub fn request(first: u32, last: u32) -> u32 {
	let mut s = lock();
	s.next = s.next.wrapping_add(1).max(1);
	let id = s.next;
	s.snaps.insert(id, Snap { first, last, q: None });
	// The oldest snapshot no live hold uses goes (a hold keeps its capture however many newer snapshots come).
	while s.snaps.len() > MAX_SNAPSHOTS {
		let Some(oldest) = s.snaps.keys().copied().find(|k| *k != id && !s.pinned.contains(k)) else { break };
		s.snaps.remove(&oldest);
	}
	PENDING.store(true, Ordering::Release);
	id
}

/// A `hold` op uses snapshot `id` from now on: it is not evicted by newer snapshots (until the next `set_pinned`).
pub fn pin(id: u32) {
	lock().pinned.insert(id);
}

/// The snapshot ids the live hold ops use now (the store, once per tick): everything else may be evicted again.
pub fn set_pinned(ids: impl IntoIterator<Item = u32>) {
	lock().pinned = ids.into_iter().collect();
}

/// Checks that snapshot `id` exists and covers `joint` (a `hold` naming it): Err(ERR_RANGE) unknown, Err(ERR_BAD_JOINT) outside.
pub fn covers(id: u32, joint: u32) -> Result<(), i32> {
	match lock().snaps.get(&id) {
		None => Err(ERR_RANGE),
		Some(s) if joint < s.first || joint >= s.last => Err(ERR_BAD_JOINT),
		Some(_) => Ok(()),
	}
}

/// The captured rotation of `joint` in snapshot `id`; `None` while it waits for the renderer, or when it was forgotten.
pub fn rotation(id: u32, joint: u32) -> Option<[f32; 4]> {
	let s = lock();
	let snap = s.snaps.get(&id)?;
	snap.q.as_ref()?.get(joint.checked_sub(snap.first)? as usize).copied()
}

/// Whether the renderer has snapshots to capture (one atomic load).
pub fn pending() -> bool {
	PENDING.load(Ordering::Acquire)
}

/// The render thread: `local` = the base local rotation of every joint. Every waiting snapshot takes its joints; how many were fulfilled.
pub fn service(local: &[[f32; 4]]) -> usize {
	let mut s = lock();
	let mut done = 0;
	for snap in s.snaps.values_mut().filter(|n| n.q.is_none()) {
		if let Some(q) = local.get(snap.first as usize..snap.last as usize) {
			snap.q = Some(q.to_vec());
			done += 1;
		}
	}
	let waiting = s.snaps.values().any(|n| n.q.is_none());
	PENDING.store(waiting, Ordering::Release);
	done
}

/// Forgets every snapshot and the debug state (a load, a new game, a level change, the mods replaced).
pub fn clear() {
	forget_snapshots();
	let mut s = lock();
	s.joints.clear();
	s.world.clear();
	drop(s);
	READS.store(false, Ordering::Release);
	// The dots go at once (not frozen for the 1 s the draw list stays fresh); the mod calls `pose_debug(true)` again.
	if DEBUG.swap(false, Ordering::AcqRel) {
		super::draw::submit_debug(&[], &[]);
	}
}

/// Forgets every snapshot (a scene tick: the holds are cleared with them) and keeps the debug dots and reads as they are.
pub fn forget_snapshots() {
	let mut s = lock();
	s.snaps.clear();
	s.pinned.clear();
	PENDING.store(false, Ordering::Release);
}

pub fn set_debug(on: bool) {
	DEBUG.store(on, Ordering::Release);
	if !on {
		lock().joints.clear();
	}
}

/// Whether `pose_debug` is on (one atomic load).
pub fn debug_on() -> bool {
	DEBUG.load(Ordering::Acquire)
}

/// The render thread: the world positions of the drawn pose's joints (empty = no hero to show).
pub fn publish_joints(joints: Vec<[f32; 3]>) {
	lock().joints = joints;
}

/// The positions the renderer published last.
pub fn joints() -> Vec<[f32; 3]> {
	lock().joints.clone()
}

/// The worker: a mod reads joints that need the rendered pose (the renderer starts publishing on its next frame).
pub fn want_reads() {
	READS.store(true, Ordering::Release);
}

/// Whether the renderer should publish the game pose's world frames (one atomic load).
pub fn reads_on() -> bool {
	READS.load(Ordering::Acquire)
}

/// The render thread: the world frame (three basis rows then the position, game world) of every joint of the GAME pose, and the hero position
/// of the same frame (empty = no hero to show).
pub fn publish_world(world: Vec<[f32; 12]>, hero_pos: [f32; 3]) {
	let mut s = lock();
	s.world = world;
	s.hero_pos = hero_pos;
}

/// The worker: a copy of the last published game-pose frames and the hero position they were made at (empty before the first rendered frame).
pub fn rendered() -> (Vec<[f32; 12]>, [f32; 3]) {
	let s = lock();
	(s.world.clone(), s.hero_pos)
}

/// The draw list of the debug dots: for every joint with a screen place (`Some` = the point in the game's 512 x 448 space) a 4 x 4 dot and
/// its name (or number). Names are `(name, joint)` pairs of the binding. Returns the commands and the text pool.
pub fn debug_cmds(points: &[Option<[f32; 2]>], names: &[(String, u32)]) -> (Vec<OmniDrawCmd>, Vec<u8>) {
	let mut cmds = Vec::new();
	let mut pool = Vec::new();
	for (j, p) in points.iter().enumerate() {
		let Some([x, y]) = *p else { continue };
		cmds.push(OmniDrawCmd { kind: super::draw_quads::KIND_RECT, x: x - 2.0, y: y - 2.0, w: 4.0, h: 4.0, color: 0xffe0_20ff, ..OmniDrawCmd::default() });
		let label = names.iter().find(|n| n.1 as usize == j).map_or_else(|| j.to_string(), |n| n.0.clone());
		let (off, len) = (pool.len() as u32, label.len() as u32);
		pool.extend_from_slice(label.as_bytes());
		cmds.push(OmniDrawCmd { kind: super::draw_quads::KIND_TEXT, x: x + 3.0, y: y - 4.0, w: 0.4, color: 0xffff_ffff, color2: 0x0000_00ff, text_off: off, text_len: len, ..OmniDrawCmd::default() });
	}
	(cmds, pool)
}

/// The worker, once per game tick: while `pose_debug` is on, the published joints through `project` (the game's camera) into the debug draw list.
pub fn debug_tick(project: impl Fn([f32; 3]) -> Option<[f32; 2]>, names: &[(String, u32)]) {
	if !debug_on() {
		return;
	}
	let points: Vec<Option<[f32; 2]>> = joints().into_iter().map(project).collect();
	let (cmds, pool) = debug_cmds(&points, names);
	super::draw::submit_debug(&cmds, &pool);
}

/// `pose_debug` turned off: the dots go.
pub fn debug_off() {
	set_debug(false);
	super::draw::submit_debug(&[], &[]);
}

#[cfg(test)]
#[path = "pose_capture_tests.rs"]
mod tests;
