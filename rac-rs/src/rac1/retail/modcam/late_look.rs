//! [PC] Late mouse look (GitHub #63 "input lag mouse", lane nfeat1, 2026-11-02): the shown camera turns by the mouse counts the
//! shown picture does not hold yet, so a mouse turn reaches the screen in the frame it was made instead of two fields later.
//!
//! Why the delay was there (measured in lane nfeat1, HANDOFF.md): the mouse counts of a field go to the worker at a tick
//! (`Retail::tick`), the worker runs that field while the frame draws, the next tick takes it, and frame smoothing then blends the
//! previous field's camera into it over one more field. A count moved at the start of a field interval needed about 2 fields
//! (40 ms PAL) plus the time to the next tick before it was fully on screen; with every display frame captured (3 per field) the
//! first moving frame came 4 frames after the field's input was handed out (first and third person, Vulkan and DirectX 12).
//!
//! What this file does (display only: the game, its RAM and its timing are untouched, so a run without mouse counts draws exactly
//! what it drew before): the main thread keeps the counts of the fields handed out and not shown yet (`sent` / `taken` from
//! `Retail::tick`) plus the counts not handed out yet (`ui_look`); `Retail::camera` turns the blended camera by those counts
//! with the gains of the worker's newest field (`post_field`: the Modern camera's follow hand or the first-person mouse drove it;
//! its gains, Reversed options and the pitch room left before the camera's own pitch stop). Third person orbits the eye round
//! the boom's pivot (the Modern camera's eye = pivot - forward x boom length), first person turns in place. When the fields land,
//! the blend has the same turn and the offset has shrunk by it, so the picture does not jump. RAC_LATE_LOOK=0: off.

use super::*;
use std::collections::VecDeque;
use std::sync::Mutex;

/// The most yaw the late turn adds (rad): a fast flick shows the rest as before (blended in over the next fields).
const YAW_MAX: f32 = 0.6;
/// The shown pitch stays inside +-this (rad; the free look stops at 84 deg).
const PITCH_LIMIT: f32 = 1.5;

/// What the worker's newest field says about the mouse and the camera (`post_field`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Drive {
	/// The mouse turned the camera in that field (Modern camera follow hand, or first person).
	pub on: bool,
	/// Boom length (m) from the eye to the pivot the eye orbits; 0 = turn in place (first person).
	pub len: f32,
	/// Shown yaw (+ = left) and shown pitch (+ = up) in rad per mouse count, Reversed options and speed rows included.
	pub k: [f32; 2],
	/// Pitch room (rad, >= 0) up and down before the camera's own pitch stop.
	pub room: [f32; 2],
}

/// The counts of the fields handed to the worker and not taken yet (oldest first) and the counts of the newest field taken.
#[derive(Debug, Default)]
pub(crate) struct Queue {
	pub sent: VecDeque<[i16; 2]>,
	pub cur: [i16; 2],
}

impl Queue {
	/// Keeps one entry per field in flight (`inflight` = the tick's count; heals a queue that lost step: zeros in front).
	fn fit(&mut self, inflight: u32) {
		let n = inflight as usize;
		while self.sent.len() > n {
			self.sent.pop_front();
		}
		while self.sent.len() < n {
			self.sent.push_front([0, 0]);
		}
	}

	/// A field with `look` handed out while `inflight` fields were out before it.
	pub fn sent(&mut self, look: [i16; 2], inflight: u32) {
		self.fit(inflight);
		self.sent.push_back(look);
	}

	/// The oldest field in flight was taken (`inflight` counts it).
	pub fn taken(&mut self, inflight: u32) {
		self.fit(inflight);
		self.cur = self.sent.pop_front().unwrap_or([0, 0]);
	}

	/// Counts not in the camera blended at `alpha` (0 = previous field, 1 = newest taken): the rest of the newest field's, every
	/// field in flight, and `ui` (not handed out yet).
	pub fn pending(&self, alpha: f32, ui: [f32; 2]) -> [f32; 2] {
		let rest = 1.0 - alpha.clamp(0.0, 1.0);
		let mut p = [self.cur[0] as f32 * rest + ui[0], self.cur[1] as f32 * rest + ui[1]];
		for s in &self.sent {
			p[0] += s[0] as f32;
			p[1] += s[1] as f32;
		}
		p
	}
}

static DRIVE: Mutex<Drive> = Mutex::new(Drive { on: false, len: 0.0, k: [0.0; 2], room: [0.0; 2] });
static QUEUE: Mutex<Queue> = Mutex::new(Queue { sent: VecDeque::new(), cur: [0, 0] });

/// RAC_LATE_LOOK=0 turns the late mouse look off (read once).
pub fn switch() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_LATE_LOOK").map_or(true, |v| v.trim() != "0"))
}

/// Main thread (`Retail::tick`): a field handed to the worker with these mouse counts.
pub fn sent(look: [i16; 2], inflight: u32) {
	if let Ok(mut q) = QUEUE.lock() {
		q.sent(look, inflight);
	}
}

/// Main thread (`Retail::tick`): the worker's oldest field in flight was taken.
pub fn taken(inflight: u32) {
	if let Ok(mut q) = QUEUE.lock() {
		q.taken(inflight);
	}
}

/// Worker, after each field (`field_gs.rs`, after `fp_look::post_field`): what the mouse did to the camera in that field.
pub fn post_field(m: &ee_oracle::Machine) {
	let d = if switch() { drive_now(m) } else { Drive::default() };
	if let Ok(mut g) = DRIVE.lock() {
		*g = d;
	}
}

fn drive_now(m: &ee_oracle::Machine) -> Drive {
	let (lr, ud) = (if m.rd8(OPT_LR) == 0 { -1.0 } else { 1.0 }, if m.rd8(OPT_UD) == 0 { -1.0 } else { 1.0 });
	let k = MOUSE * sens() * crate::retail::pcext::mousespeed_scale();
	if let Some((up, down, gated)) = fp_look::pitch_room(m) {
		return Drive { on: true, len: 0.0, k: [-k * lr, if gated { 0.0 } else { -k * ud }], room: [up, down] };
	}
	let Ok(st) = ST.lock() else { return Drive::default() };
	if !(enabled() && st.live && st.ran == st.field && classic::drives(&st) && st.len.is_finite() && st.len > 0.0) {
		return Drive::default();
	}
	// The follow pitch is + = looking down (`rows`): looking up runs toward PITCH.0.
	Drive { on: true, len: st.len, k: [-k * lr, -k * ud], room: [(st.pitch - PITCH.0).max(0.0), (PITCH.1 - st.pitch).max(0.0)] }
}

/// The mouse counts the window gave since the last field was handed out (none under a replay: its counts are in the file).
fn ui_look(live: bool) -> [f32; 2] {
	if !live {
		return [0.0; 2];
	}
	UI.lock().map_or([0.0; 2], |u| u.look)
}

/// The late turn (shown yaw, shown pitch, boom length) for a camera blended at `alpha`; None = nothing to add.
pub(crate) fn offset(alpha: f32, live: bool) -> Option<(f32, f32, f32)> {
	if !switch() {
		return None;
	}
	let d = *DRIVE.lock().ok()?;
	let p = QUEUE.lock().ok()?.pending(alpha, ui_look(live));
	turn_of(&d, p)
}

/// Pure: the turn of `pending` counts under `d` (yaw capped at YAW_MAX, pitch inside the room).
pub(crate) fn turn_of(d: &Drive, pending: [f32; 2]) -> Option<(f32, f32, f32)> {
	if !d.on || pending == [0.0, 0.0] {
		return None;
	}
	let yaw = (pending[0] * d.k[0]).clamp(-YAW_MAX, YAW_MAX);
	let pitch = (pending[1] * d.k[1]).clamp(-d.room[1], d.room[0]);
	(yaw != 0.0 || pitch != 0.0).then_some((yaw, pitch, d.len))
}

/// Pure: camera `c` (eye, yaw, pitch + = up, tan) turned by `dyaw` / `dpitch`; with `len` > 0 the eye orbits the point `len` ahead
/// of it (the pivot stays where it is on screen), else it turns in place.
pub(crate) fn turn_cam(c: crate::retail::Cam, dyaw: f32, dpitch: f32, len: f32) -> crate::retail::Cam {
	let fwd = |y: f32, p: f32| {
		let ((sy, cy), (sp, cp)) = (y.fsin_cos(), p.fsin_cos());
		[cy * cp, sy * cp, sp]
	};
	let (lo, hi) = ((-PITCH_LIMIT).min(c.2), PITCH_LIMIT.max(c.2));
	let (y1, p1) = (c.1 + dyaw, (c.2 + dpitch).clamp(lo, hi));
	if len <= 0.0 {
		return (c.0, y1, p1, c.3);
	}
	let (f0, f1) = (fwd(c.1, c.2), fwd(y1, p1));
	let piv = [c.0[0] + f0[0] * len, c.0[1] + f0[1] * len, c.0[2] + f0[2] * len];
	([piv[0] - f1[0] * len, piv[1] - f1[1] * len, piv[2] - f1[2] * len], y1, p1, c.3)
}

#[cfg(test)]
#[path = "late_look_tests.rs"]
mod tests;
