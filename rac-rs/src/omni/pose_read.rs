//! `pose_read` (ABI 1.4): the world frame of a hero joint, for `pose.joint_position`, `pose.joint_rotation` and `pose.mirrored`. PURE reads: no
//! retail code is called and nothing is written (the functions take `&Cpu` / `&[u8]`), so a mod reading joints cannot change the game.
//!
//! Two sources, by joint:
//! - The nine hero attach matrices that retail's own `HeroComputeAttachMatrices` writes every tick at `0x13fe10` (stride `0x40`, four rows, the
//!   translation row at `+0x30`). They are exactly what retail's logic uses (the held weapon follows matrix 0), current tick, deterministic. The
//!   list of attach points that fill them is `ATTACH_LIST`; the joints they follow (hero class 0, 111 joints) are 56 (right hand / weapon grip),
//!   55, 99, 93, 7, 5, 54, 3, 5. The data layout is shared by the PAL and USA discs (layout.rs: both use the "ntsc" tables), so the address is
//!   the same.
//! - Every other joint (left hand 76, chest 6, fingers, ...) comes from the LAST RENDERED pose slot (pose_capture.rs): the world frame of the
//!   game pose as the renderer evaluated it, ONE FRAME BEHIND the tick (the render thread is not locked to the game, so it can be a little
//!   older). That value depends on render timing: a mod must use it for looks only, never to decide game state. Before the first rendered frame
//!   (or when no hero is drawn) such a read fails with -1. One copy of the slot is kept per tick so every read of a tick sees the same frame.
//!
//! `flags` bit 0 (`pose.mirrored`): the joint's frame reflected across the chest plane, where the same part would be on the other side of the body
//! (the second gun of Dual Blasters). The plane's normal is the hero's left axis (row 1 of the hero moby's world matrix in RAM, current tick); its
//! offset is the chest joint's lateral distance from the hero origin, taken from the rendered slot (one frame behind, 0 before the first frame): the
//! plane passes through `hero position + normal * offset`. The chest's own tilt is not followed, so the reflected point is off by the torso lean
//! (a few cm to about 12 cm at arm length) against the exact reflection across the chest joint's own plane.

use super::binding::SkeletonDef;
use super::{events, pose, pose_capture};
use ee_oracle::cpu::Cpu;
use std::sync::Mutex;

const MASK: u32 = 0x01ff_ffff;
/// The hero attach matrices (`HeroComputeAttachMatrices` -> `MobyEvalAttachLists`, out buffer): nine 4x4 float matrices, stride `0x40`.
pub const ATTACH_MATRICES: usize = 0x13_fe10;
const MATRIX_STRIDE: usize = 0x40;
/// The attach points (indices into the class attach table `class+0x1c`) whose matrices are stored, in order.
pub const ATTACH_LIST: [u32; 9] = [0, 1, 2, 3, 4, 5, 6, 0x1d, 0x1e];
/// Hero moby: position `+0x10`, world rows `+0xc0` / `+0xd0` / `+0xe0` (row 1 = the left axis).
const POS: usize = 0x10;
const ROW1: usize = 0xd0;
pub const FLAG_MIRRORED: u32 = 1;

fn u32_at(ram: &[u8], a: usize) -> Option<u32> {
	ram.get(a..a + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn f32_at(ram: &[u8], a: usize) -> Option<f32> {
	u32_at(ram, a).map(f32::from_bits)
}

fn vec3_at(ram: &[u8], a: usize) -> Option<[f32; 3]> {
	Some([f32_at(ram, a)?, f32_at(ram, a + 4)?, f32_at(ram, a + 8)?])
}

/// The joint that attach point `point` of the class record `class` follows (the last byte of its chain).
fn chain_end(ram: &[u8], class: u32, point: u32) -> Option<u32> {
	let table = u32_at(ram, class as usize + 0x1c)? & MASK;
	if table == 0 {
		return None;
	}
	let c = (u32_at(ram, (table + 4 + 4 * point) as usize)? & MASK) as usize;
	let n = ram.get(c..c + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]) as usize);
	if c == 0 || n == 0 {
		return None;
	}
	ram.get(c + 4 + n - 1).map(|&j| u32::from(j))
}

/// The index (0..9) of the stored attach matrix that follows `joint`, if there is one.
pub fn matrix_index(ram: &[u8], class: u32, joint: u32) -> Option<usize> {
	ATTACH_LIST.iter().position(|&p| chain_end(ram, class, p) == Some(joint))
}

/// The 12 floats of an attach matrix as retail stored it: three rows then the position. None when a value is not finite.
fn stored(ram: &[u8], k: usize) -> Option<[f32; 12]> {
	let at = ATTACH_MATRICES + k * MATRIX_STRIDE;
	let mut out = [0.0f32; 12];
	for (i, v) in out.iter_mut().enumerate() {
		*v = f32_at(ram, at + 16 * (i / 3) + 4 * (i % 3))?;
	}
	out.iter().all(|v| v.is_finite()).then_some(out)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `m` (three basis rows then the position, 12 floats) reflected across the plane through `centre` with the unit normal `n`. A reflected frame is
/// left-handed.
pub fn reflect(m: &[f32; 12], n: [f32; 3], centre: [f32; 3]) -> [f32; 12] {
	let mut out = [0.0f32; 12];
	for r in 0..4 {
		let v = [m[r * 3], m[r * 3 + 1], m[r * 3 + 2]];
		// rows are directions; the position (row 3) is reflected about the plane through `centre`
		let d = if r == 3 { dot([v[0] - centre[0], v[1] - centre[1], v[2] - centre[2]], n) } else { dot(v, n) };
		for i in 0..3 {
			out[r * 3 + i] = v[i] - 2.0 * d * n[i];
		}
	}
	out
}

/// The hero's unit left axis (row 1 of its world matrix) and its position, from RAM. None: zero or non-finite.
fn hero_axes(ram: &[u8], hero: u32) -> Option<([f32; 3], [f32; 3])> {
	let (v, pos) = (vec3_at(ram, hero as usize + ROW1)?, vec3_at(ram, hero as usize + POS)?);
	let len = dot(v, v).sqrt();
	(len > 1e-6 && len.is_finite() && pos.iter().all(|c| c.is_finite())).then(|| ([v[0] / len, v[1] / len, v[2] / len], pos))
}

/// A copy of the rendered slot made once per tick (keyed by the instruction count), so one tick sees one frame. `new_tick` empties it.
struct Cache {
	stamp: u64,
	world: Vec<[f32; 12]>,
	hero_pos: [f32; 3],
}

static CACHE: Mutex<Cache> = Mutex::new(Cache { stamp: u64::MAX, world: Vec::new(), hero_pos: [0.0; 3] });

fn cache() -> std::sync::MutexGuard<'static, Cache> {
	CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// A new game tick (or a load): the slot copy is stale.
pub fn new_tick() {
	let mut c = cache();
	c.world.clear();
	c.stamp = u64::MAX;
}

/// The rendered slot of this tick: asks the renderer to keep publishing and copies the slot on the first call of a tick.
fn slot(stamp: u64) -> std::sync::MutexGuard<'static, Cache> {
	let mut c = cache();
	if c.stamp != stamp {
		pose_capture::want_reads();
		(c.world, c.hero_pos) = pose_capture::rendered();
		c.stamp = stamp;
	}
	c
}

/// The lateral offset of the chest plane from the hero origin along `n` (0 before the first rendered frame).
fn chest_offset(c: &Cache, chest: u32, n: [f32; 3]) -> f32 {
	let Some(w) = c.world.get(chest as usize) else { return 0.0 };
	let off = dot([w[9] - c.hero_pos[0], w[10] - c.hero_pos[1], w[11] - c.hero_pos[2]], n);
	if off.is_finite() { off } else { 0.0 }
}

/// The answer for `pose_read` from RAM and the rendered slot: Ok(12 floats), Err(-1 no matrix yet, -2 not posable, -3 no such joint). `stamp` = the tick
/// (the instruction count).
pub fn read_ram(ram: &[u8], stamp: u64, sk: &SkeletonDef, moby: u32, joint: u32, flags: u32) -> Result<[f32; 12], i32> {
	let moby = moby & MASK;
	let joints = pose::hero_joints(ram, moby).ok_or(-2)?;
	if joint >= joints {
		return Err(-3);
	}
	let class = u32_at(ram, moby as usize + 0x24).ok_or(-3)? & MASK;
	let m = match matrix_index(ram, class, joint) {
		Some(k) => stored(ram, k).ok_or(-1)?,
		None => *slot(stamp).world.get(joint as usize).filter(|w| w.iter().all(|v| v.is_finite())).ok_or(-1)?,
	};
	if flags & FLAG_MIRRORED == 0 {
		return Ok(m);
	}
	let (n, pos) = hero_axes(ram, moby).ok_or(-1)?;
	let off = chest_offset(&slot(stamp), sk.chest, n);
	Ok(reflect(&m, n, [pos[0] + n[0] * off, pos[1] + n[1] * off, pos[2] + n[2] * off]))
}

/// The answer for `pose_read` (see `read_ram`); Err(-1) also when no level is live. Takes the machine by shared reference: a read changes nothing.
pub fn read(c: &Cpu, sk: &SkeletonDef, moby: u32, joint: u32, flags: u32) -> Result<[f32; 12], i32> {
	events::live_level(c).ok_or(-1)?;
	read_ram(&c.mem.ram, c.icount, sk, moby, joint, flags)
}

#[cfg(test)]
#[path = "pose_read_tests.rs"]
mod tests;
