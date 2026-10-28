//! The ABI 1.4 callbacks of the `OmniHost` (`pose_set`, `pose_info`, `pose_read`, `pose_snapshot`, `pose_debug`) on the host store in pose.rs and
//! the renderer slot in pose_capture.rs. They run on the worker thread inside a mod's callback, with the machine in scope (host.rs `cpu`).
//! `pose_snapshot` registers a capture the renderer fulfils on its next frame (the id is > 0 at once); `pose_debug` turns the joint dots on or off.
//!
//! The host sees the loader as ONE client: the loader keeps which mod owns which joint (last call wins); `pose_set(n = 0)` fades every op of the
//! moby. Only the hero is posable (class 0, 111 joints, the hero cell's moby); any other moby answers -2 / 0 joints.

use super::binding::{Binding, SkeletonDef};
use super::cabi::{Host, OmniPoseOp};
use super::host::{cpu, state, HostState};
use super::pose::{u32_at, MASK};
use super::{events, pose, pose_capture};
use std::ffi::c_void;

/// `parent` and `partner` of every joint (0xffff = none), from the class record's bone table and the binding's pairs.
pub fn tables(ram: &[u8], moby: u32, joints: u32, sk: &SkeletonDef) -> (Vec<u16>, Vec<u16>) {
	let class = u32_at(ram, moby as usize + 0x24) & MASK;
	let bones = u32_at(ram, class as usize + 0x18) & MASK;
	let h16 = |a: usize| ram.get(a..a + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
	let parent = (0..joints as usize)
		.map(|j| {
			let b = bones as usize + 16 * j;
			let (p, marker) = (h16(b + 12) / 64, h16(b + 14));
			if bones == 0 || marker == 0 || p as usize >= j {
				0xffff
			} else {
				p
			}
		})
		.collect();
	let mut partner = vec![0xffffu16; joints as usize];
	for &(a, b) in &sk.pairs {
		partner[a as usize] = b as u16;
		partner[b as usize] = a as u16;
	}
	for &c in &sk.centre {
		partner[c as usize] = c as u16;
	}
	(parent, partner)
}

unsafe extern "C" fn pose_set_cb(_user: *mut c_void, moby: u32, ops: *const OmniPoseOp, n: u32) -> i32 {
	let Some(c) = cpu() else { return -1 };
	if events::live_level(c).is_none() {
		return -1;
	}
	let Some(joints) = pose::hero_joints(&c.mem.ram, moby) else { return -2 };
	if n == 0 || ops.is_null() {
		pose::with_store(|s| s.fade_moby(moby & MASK, pose::FADE_TICKS));
		return 0;
	}
	if n as usize > pose::MAX_OPS {
		return pose::ERR_CAP;
	}
	// SAFETY: the loader passes `n` ops.
	let ops = unsafe { std::slice::from_raw_parts(ops, n as usize) };
	match pose::with_store(|s| s.set(moby & MASK, joints, ops)) {
		Ok(k) => k as i32,
		Err(code) => code,
	}
}

unsafe extern "C" fn pose_info_cb(user: *mut c_void, moby: u32, parent: *mut u16, partner: *mut u16, cap: u32) -> i32 {
	let (Some(c), Some(sk)) = (cpu(), state(user).binding.skeleton.as_ref()) else { return 0 };
	let Some(joints) = pose::hero_joints(&c.mem.ram, moby).filter(|&j| j == sk.joint_count) else { return 0 };
	if parent.is_null() || partner.is_null() {
		return joints as i32;
	}
	let (par, pair) = tables(&c.mem.ram, moby & MASK, joints, sk);
	let n = (cap as usize).min(par.len());
	// SAFETY: the loader passes arrays with room for `cap` entries.
	unsafe {
		std::ptr::copy_nonoverlapping(par.as_ptr(), parent, n);
		std::ptr::copy_nonoverlapping(pair.as_ptr(), partner, n);
	}
	joints as i32
}

unsafe extern "C" fn pose_read_cb(user: *mut c_void, moby: u32, joint: u32, flags: u32, out12: *mut f32) -> i32 {
	let (Some(c), Some(sk)) = (cpu(), state(user).binding.skeleton.as_ref()) else { return -1 };
	if out12.is_null() {
		return -1;
	}
	match super::pose_read::read(c, sk, moby, joint, flags) {
		Ok(m) => {
			// SAFETY: the loader passes room for twelve floats.
			unsafe { std::ptr::copy_nonoverlapping(m.as_ptr(), out12, 12) };
			0
		}
		Err(code) => code,
	}
}

unsafe extern "C" fn pose_snapshot_cb(_user: *mut c_void, moby: u32, first: u32, last_excl: u32) -> i32 {
	let Some(c) = cpu() else { return -1 };
	if events::live_level(c).is_none() {
		return -1;
	}
	let Some(joints) = pose::hero_joints(&c.mem.ram, moby) else { return -2 };
	if first >= last_excl || last_excl > joints {
		return pose_capture::ERR_BAD_JOINT;
	}
	pose_capture::request(first, last_excl) as i32
}

unsafe extern "C" fn pose_debug_cb(_user: *mut c_void, moby: u32, on: i32) -> i32 {
	let Some(c) = cpu() else { return -1 };
	if events::live_level(c).is_none() {
		return -1;
	}
	if pose::hero_joints(&c.mem.ram, moby).is_none() {
		return -2;
	}
	if on != 0 {
		pose_capture::set_debug(true);
	} else {
		pose_capture::debug_off();
	}
	0
}

/// Once per game tick (`omni::tick`): the dots of `pose_debug`, through the game's camera of this tick.
pub fn debug_tick(c: &ee_oracle::cpu::Cpu, st: &HostState) {
	if !pose_capture::debug_on() {
		return;
	}
	let names: &[(String, u32)] = st.binding.skeleton.as_ref().map_or(&[], |s| &s.names);
	pose_capture::debug_tick(|p| super::draw_w2s::screen_of(c, p).filter(|r| r.1).map(|r| r.0), names);
}

/// Fills the entries when the binding has the hero skeleton (NULL = not supported).
pub fn fill(h: &mut Host, binding: &Binding) {
	if binding.skeleton.is_some() {
		h.pose_set = Some(pose_set_cb);
		h.pose_info = Some(pose_info_cb);
		h.pose_read = Some(pose_read_cb);
		h.pose_snapshot = Some(pose_snapshot_cb);
		h.pose_debug = Some(pose_debug_cb);
		h.pose_follow = Some(super::pose_follow::pose_follow_cb);
	}
}
