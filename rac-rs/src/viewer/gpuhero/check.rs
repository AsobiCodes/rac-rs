//! [PC] perf (v0.4.2, 1% lows): the hero GPU build's self-check (Auto mode) without a wait for the GPU.
//!
//! The first `SELF_FRAMES` frames of each kind are built on the GPU AND on the CPU and compared (gpuhero.rs). The compare used to
//! block the render thread (`Device::poll(wait_indefinitely)`, a full GPU sync): 15-60 ms on the laptop for each of ~280 frames in
//! the first minutes of play, which were nearly all of the 1% lows. Then the check frame submitted its own command buffer and
//! polled the device without blocking; that still stalled on the laptop (lane nhero, 2026-10-29): wgpu's `Queue::submit` and
//! `Device::poll` both take `command_indices`, which `Queue::present` holds across the whole vkQueuePresentKHR (NVIDIA Optimus:
//! ms), so every check frame waited for the present thread (set_marker 8-14 ms against 0.6 ms on a fast GPU).
//! Now a check frame touches no lock the present holds: `start_check` only asks for a check; the frame's own encoder (`record`,
//! prepare.rs) then runs the build into a separate buffer, copies it into a staging buffer and registers the map with
//! `CommandEncoder::map_buffer_on_submit`, so the frame's single submit (present thread) starts the map and, later, delivers the
//! callback (`Queue::submit` fires the finished maps); `poll_check` never calls the device. The CPU vertices (which this frame
//! draws, as before) are copied into a reused buffer; once the map is ready `poll_check` compares (`self_check_result`: a difference
//! switches the GPU build off from then on, a frame or two later than before). Two checks are in flight at most (two staging
//! buffers), so the frames of a kind are still verified at about one per display frame. RAC_HERO_CHECK_OLD=1 = the previous scheme
//! (own submit + `Device::poll`, one check in flight); RAC_HERO_GPU_CHECK=1 and RAC_MARKER_CHECK=1 (dev) still wait (`build_and_read`).

use super::*;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

const WAITING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED_MAP: u8 = 2;

/// A check whose map is still pending after this many frames is given up (its encoder was never submitted): a new staging buffer.
const GIVE_UP: u32 = 900;
/// Checks in flight at most (staging buffers).
pub(super) const SLOTS: usize = 2;

/// What `AsyncCheck::step` found.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Step {
	/// No check in flight.
	Idle,
	/// The GPU has not finished (or the map is not delivered yet).
	Waiting,
	/// Compare the staging bytes (`mapped`: the map succeeded) of a frame of `kind` with the CPU bytes.
	Compare { kind: usize, mapped: bool },
	/// The frame never delivered its CPU bytes: drop the check (unmap when `mapped`).
	Drop { mapped: bool },
}

/// One check in flight: the frame kind, the CPU build's bytes of that frame (a reused buffer) and the state the map callback sets.
#[derive(Default)]
pub(super) struct AsyncCheck {
	kind: usize,
	busy: bool,
	cpu: Vec<u8>,
	cpu_set: bool,
	state: Arc<AtomicU8>,
	/// `poll_check` calls (display frames) that found the map still pending.
	waited: u32,
}

impl AsyncCheck {
	pub(super) fn busy(&self) -> bool {
		self.busy
	}

	/// A check for a frame of `kind` starts; the returned cell is set by the map callback.
	pub(super) fn begin(&mut self, kind: usize) -> Arc<AtomicU8> {
		self.kind = kind;
		self.busy = true;
		self.cpu_set = false;
		self.waited = 0;
		self.state = Arc::new(AtomicU8::new(WAITING));
		self.state.clone()
	}

	/// The map callback's verdict.
	pub(super) fn deliver(state: &AtomicU8, ok: bool) {
		state.store(if ok { MAPPED } else { FAILED_MAP }, Relaxed);
	}

	/// The CPU build's bytes of the check frame (kept in a reused buffer).
	pub(super) fn set_cpu(&mut self, bytes: &[u8]) {
		if self.busy && !self.cpu_set {
			self.cpu.clear();
			self.cpu.extend_from_slice(bytes);
			self.cpu_set = true;
		}
	}

	pub(super) fn cpu(&self) -> &[u8] {
		&self.cpu
	}

	pub(super) fn step(&self) -> Step {
		if !self.busy {
			return Step::Idle;
		}
		match (self.state.load(Relaxed), self.cpu_set) {
			(WAITING, _) => Step::Waiting,
			(s, false) => Step::Drop { mapped: s == MAPPED },
			(s, true) => Step::Compare { kind: self.kind, mapped: s == MAPPED },
		}
	}

	/// One more frame found the map pending; true once it has waited `GIVE_UP` frames.
	pub(super) fn wait_tick(&mut self) -> bool {
		self.waited += 1;
		self.waited >= GIVE_UP
	}

	/// The check is over (compared or dropped): the next frame may start one.
	pub(super) fn finish(&mut self) {
		self.busy = false;
		self.cpu_set = false;
	}
}

/// The map callback's cell. Dropped uncalled (the frame's encoder was never submitted: `frame_buffers` / `draw_passes` failed) it
/// reports a failed map, so the slot is free on the next frame instead of after `GIVE_UP` frames; after a delivery it changes nothing.
struct Deliver(Arc<AtomicU8>);

impl Drop for Deliver {
	fn drop(&mut self) {
		let _ = self.0.compare_exchange(WAITING, FAILED_MAP, Relaxed, Relaxed);
	}
}

/// One check slot: the check state, its staging buffer, and the request `record_checks` takes (new scheme).
#[derive(Default)]
pub(super) struct Slot {
	chk: AsyncCheck,
	stage: Option<wgpu::Buffer>,
	/// `start_check` asked; the frame encoder has not recorded the build + copy yet (the map callback's cell).
	req: Mutex<Option<Arc<AtomicU8>>>,
}

/// RAC_HERO_CHECK_OLD=1: the previous scheme (one check, own submit, `Device::poll`), for A/B runs.
fn old_scheme() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_HERO_CHECK_OLD").is_ok_and(|v| v == "1"))
}

impl HeroGpu {
	fn slots(&self) -> usize {
		if old_scheme() { 1 } else { SLOTS }
	}

	/// No slot is free: no new check frame starts.
	pub(in crate::viewer) fn check_busy(&self) -> bool {
		self.checks[..self.slots()].iter().all(|s| s.chk.busy())
	}

	/// A request not yet recorded by the frame encoder (the frame never reached `record`, or `set_marker` ran twice) is dropped: its
	/// CPU bytes would not match the parameters the build is recorded with.
	pub(in crate::viewer) fn drop_unrecorded(&mut self) {
		for s in &mut self.checks {
			let dropped = s.req.get_mut().unwrap_or_else(|e| e.into_inner()).take().is_some();
			if dropped {
				s.chk.finish();
			}
		}
	}

	/// The check frame. New scheme: only asks (the frame encoder's `record_checks` runs the build into `chk`, copies it into the
	/// staging buffer and registers the map). Old scheme: runs the pending build in its own submit, copies the vertex buffer and
	/// maps. Neither waits. The caller then builds the CPU vertices (which this frame draws) and hands them to `set_check_cpu`.
	pub(in crate::viewer) fn start_check(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, vbuf: &wgpu::Buffer, kind: usize) {
		let Some(i) = self.checks[..self.slots()].iter().position(|s| !s.chk.busy()) else { return };
		let size = (self.n * std::mem::size_of::<Vertex>()) as u64;
		if self.checks[i].stage.as_ref().is_none_or(|s| s.size() != size) {
			self.checks[i].stage = Some(device.create_buffer(&wgpu::BufferDescriptor { label: Some("hero check"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
		}
		self.last_check = i;
		if old_scheme() {
			let mut enc = device.create_command_encoder(&Default::default());
			self.record(&mut enc);
			let slot = &mut self.checks[i];
			let Some(stage) = slot.stage.as_ref() else { return };
			enc.copy_buffer_to_buffer(vbuf, 0, stage, 0, size);
			queue.submit(Some(enc.finish()));
			let state = slot.chk.begin(kind);
			stage.slice(..).map_async(wgpu::MapMode::Read, move |r| AsyncCheck::deliver(&state, r.is_ok()));
			return;
		}
		if self.chk.as_ref().is_none_or(|(b, _)| b.size() != size.max(16)) {
			self.chk = Some(self.make_chk_target(device, size.max(16)));
		}
		let slot = &mut self.checks[i];
		let state = slot.chk.begin(kind);
		*slot.req.get_mut().unwrap_or_else(|e| e.into_inner()) = Some(state);
	}

	/// The build's output buffer for the new scheme's check and the bind group that writes it (same pipeline, `vbuf` replaced).
	fn make_chk_target(&self, device: &wgpu::Device, size: u64) -> (wgpu::Buffer, wgpu::BindGroup) {
		let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("hero check out"), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
		let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("hero check"),
			layout: &self.layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: self.model_buf.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 1, resource: self.poses.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 2, resource: self.params.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 3, resource: buf.as_entire_binding() },
			],
		});
		(buf, bind)
	}

	/// The frame encoder (new scheme): the build into `chk`, the copy into the staging buffer, the map on submit.
	pub(super) fn record_checks(&self, enc: &mut wgpu::CommandEncoder) {
		for s in &self.checks {
			let Some(state) = s.req.lock().unwrap_or_else(|e| e.into_inner()).take() else { continue };
			let (Some((chk, bind)), Some(stage)) = (self.chk.as_ref(), s.stage.as_ref()) else {
				AsyncCheck::deliver(&state, false);
				continue;
			};
			{
				let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("hero check build"), timestamp_writes: None });
				pass.set_pipeline(&self.pipeline);
				pass.set_bind_group(0, bind, &[]);
				pass.dispatch_workgroups(self.n.div_ceil(64) as u32, 1, 1);
			}
			enc.copy_buffer_to_buffer(chk, 0, stage, 0, stage.size());
			let cell = Deliver(state);
			enc.map_buffer_on_submit(stage, wgpu::MapMode::Read, .., move |r| AsyncCheck::deliver(&cell.0, r.is_ok()));
		}
	}

	/// The CPU vertices of the check frame (see `start_check`).
	pub(in crate::viewer) fn set_check_cpu(&mut self, bytes: &[u8]) {
		self.checks[self.last_check].chk.set_cpu(bytes);
	}

	/// Every `set_marker`: collects the checks whose map is ready (old scheme: after a non-blocking device poll; new scheme: the
	/// frame's submit delivers the callbacks, no device call here). Each ready check is compared and its staging buffer unmapped.
	pub(in crate::viewer) fn poll_check(&mut self, device: &wgpu::Device) {
		let n = self.slots();
		if self.checks[..n].iter().all(|s| !s.chk.busy()) {
			return;
		}
		if old_scheme() {
			device.poll(wgpu::PollType::Poll).ok();
		}
		for s in &mut self.checks[..n] {
			let (kind, mapped) = match s.chk.step() {
				Step::Idle => continue,
				Step::Waiting => {
					if s.chk.wait_tick() {
						// Never delivered: a fresh staging buffer for the next check (the old one may still be pending).
						s.stage = None;
						*s.req.get_mut().unwrap_or_else(|e| e.into_inner()) = None;
						s.chk.finish();
					}
					continue;
				}
				Step::Drop { mapped } => (None, mapped),
				Step::Compare { kind, mapped } => (Some(kind), mapped),
			};
			if let (true, Some(stage)) = (mapped, s.stage.as_ref()) {
				if let (Some(kind), Ok(view)) = (kind, stage.slice(..).get_mapped_range()) {
					super::self_check_result(kind, &view, s.chk.cpu());
				}
				stage.unmap();
			}
			s.chk.finish();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn one_check_in_flight_and_the_cpu_bytes_are_kept() {
		let mut c = AsyncCheck::default();
		assert_eq!(c.step(), Step::Idle);
		assert!(!c.busy());
		let state = c.begin(5);
		assert!(c.busy());
		c.set_cpu(&[1, 2, 3]);
		// The GPU has not delivered yet.
		assert_eq!(c.step(), Step::Waiting);
		AsyncCheck::deliver(&state, true);
		assert_eq!(c.step(), Step::Compare { kind: 5, mapped: true });
		assert_eq!(c.cpu(), &[1, 2, 3]);
		// A second set_cpu of the same check does not overwrite the first.
		c.set_cpu(&[9]);
		assert_eq!(c.cpu(), &[1, 2, 3]);
		c.finish();
		assert_eq!(c.step(), Step::Idle);
		// The next check starts clean; the buffer is reused.
		let state = c.begin(2);
		assert_eq!(c.step(), Step::Waiting, "a new check does not see the old verdict");
		c.set_cpu(&[7, 7]);
		AsyncCheck::deliver(&state, false);
		assert_eq!(c.step(), Step::Compare { kind: 2, mapped: false });
		assert_eq!(c.cpu(), &[7, 7]);
	}

	#[test]
	fn a_check_without_cpu_bytes_is_dropped_and_a_stale_callback_cannot_touch_the_next_check() {
		let mut c = AsyncCheck::default();
		let old = c.begin(1);
		AsyncCheck::deliver(&old, true);
		assert_eq!(c.step(), Step::Drop { mapped: true });
		c.finish();
		let _new = c.begin(3);
		// A late callback of the first check writes its own cell, not the new check's.
		AsyncCheck::deliver(&old, true);
		c.set_cpu(&[1]);
		assert_eq!(c.step(), Step::Waiting);
	}

	#[test]
	fn a_check_whose_map_never_arrives_is_given_up_after_the_wait_limit() {
		let mut c = AsyncCheck::default();
		let _state = c.begin(0);
		for _ in 0..GIVE_UP - 1 {
			assert!(!c.wait_tick());
		}
		assert!(c.wait_tick(), "the limit is reached");
		// A new check starts its count again.
		let _next = c.begin(0);
		assert!(!c.wait_tick());
	}

	#[test]
	fn a_map_callback_dropped_uncalled_frees_the_slot_and_a_delivered_one_keeps_its_verdict() {
		let mut c = AsyncCheck::default();
		drop(Deliver(c.begin(4)));
		c.set_cpu(&[1]);
		assert_eq!(c.step(), Step::Compare { kind: 4, mapped: false }, "no compare, no unmap: the slot is free");
		c.finish();
		let state = c.begin(4);
		let cell = Deliver(state);
		AsyncCheck::deliver(&cell.0, true);
		drop(cell);
		c.set_cpu(&[1]);
		assert_eq!(c.step(), Step::Compare { kind: 4, mapped: true });
	}

	#[test]
	fn an_unrecorded_request_cell_is_taken_exactly_once() {
		let s = Slot::default();
		let cell = Arc::new(AtomicU8::new(WAITING));
		*s.req.lock().unwrap() = Some(cell.clone());
		let first = s.req.lock().unwrap().take();
		assert!(first.is_some() && Arc::ptr_eq(&first.unwrap(), &cell));
		assert!(s.req.lock().unwrap().take().is_none(), "record_checks encodes a request once");
	}
}
