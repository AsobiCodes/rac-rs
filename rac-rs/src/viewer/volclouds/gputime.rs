//! GPU time of the two volumetric cloud passes before the level pass (march, accumulate; the apply is one draw inside the level pass) with wgpu timestamp queries, only under RAC_FOG_LOG=1
//! (the device asks for TIMESTAMP_QUERY then, `features`). One reading is in flight at a time: the passes of a frame write their
//! begin / end stamps, the frame's encoder resolves them into a readback buffer, and the next frames map it and print one line
//! `volclouds gpu: march A ms, accumulate B ms, total C ms`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(PartialEq)]
enum State {
	Idle,
	/// This frame's passes carry stamps and the resolve is encoded; the buffer is mapped after the submit (next `run`).
	Copied,
	Mapping,
}

pub(super) struct GpuTimer {
	qs: wgpu::QuerySet,
	resolve: wgpu::Buffer,
	read: wgpu::Buffer,
	period: f32,
	state: State,
	ready: Arc<AtomicBool>,
}

impl GpuTimer {
	/// A timer, or None when the device has no timestamps or RAC_FOG_LOG is off.
	pub(super) fn new(device: &wgpu::Device, queue: &crate::viewer::presenter::GatedQueue) -> Option<GpuTimer> {
		if !super::super::volfog::frame::fog_log() || !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
			return None;
		}
		let qs = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("volclouds time"), ty: wgpu::QueryType::Timestamp, count: 4 });
		let mk = |label, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: 32, usage, mapped_at_creation: false });
		let resolve = mk("volclouds time resolve", wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC);
		let read = mk("volclouds time read", wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
		Some(GpuTimer { qs, resolve, read, period: queue.get_timestamp_period(), state: State::Idle, ready: Arc::new(AtomicBool::new(false)) })
	}

	/// Called first in every `run`: maps a finished reading, prints a mapped one. The copy into `read` was encoded in an earlier frame that
	/// may still be handed over and not yet submitted (the split-view Classic pass acquires no frame, so nothing flushed it): `sync` waits
	/// for that submit first, or the map would meet a buffer a queued command still uses. Debug path only (RAC_FOG_LOG), once per reading.
	pub(super) fn poll(&mut self, device: &wgpu::Device, queue: &crate::viewer::presenter::GatedQueue) {
		match self.state {
			State::Copied => {
				queue.sync();
				let ready = self.ready.clone();
				ready.store(false, Ordering::Relaxed);
				self.read.slice(..).map_async(wgpu::MapMode::Read, move |r| ready.store(r.is_ok(), Ordering::Relaxed));
				self.state = State::Mapping;
			}
			State::Mapping => {
				device.poll(wgpu::PollType::Poll).ok();
				if self.ready.load(Ordering::Relaxed) {
					{
						let Ok(data) = self.read.slice(..).get_mapped_range() else { return };
						let t: &[u64] = bytemuck::cast_slice(&data);
						let ms = |a: usize| t[a + 1].saturating_sub(t[a]) as f64 * self.period as f64 * 1e-6;
						eprintln!("volclouds gpu: march {:.3} ms, accumulate {:.3} ms, total {:.3} ms", ms(0), ms(2), ms(0) + ms(2));
					}
					self.read.unmap();
					self.state = State::Idle;
				}
			}
			State::Idle => {}
		}
	}

	/// The timestamp writes of pass `i` (0 march, 1 accumulate) while a reading is being taken this frame.
	pub(super) fn writes(&self, i: u32) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
		(self.state == State::Idle).then(|| wgpu::RenderPassTimestampWrites { query_set: &self.qs, beginning_of_pass_write_index: Some(2 * i), end_of_pass_write_index: Some(2 * i + 1) })
	}

	/// After the two passes: resolves this frame's stamps into the readback buffer (when they were taken).
	pub(super) fn finish(&mut self, enc: &mut wgpu::CommandEncoder) {
		if self.state == State::Idle {
			enc.resolve_query_set(&self.qs, 0..4, &self.resolve, 0);
			enc.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, 32);
			self.state = State::Copied;
		}
	}
}
