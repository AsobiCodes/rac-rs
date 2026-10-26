//! GPU time of the three volumetric fog passes (march, accumulate, apply) with wgpu timestamp queries, only under RAC_FOG_LOG=1
//! (the device asks for TIMESTAMP_QUERY then, `features`). One reading is in flight at a time: the passes of a frame write their
//! begin / end stamps, the frame's encoder resolves them into a readback buffer, and the next frames map it and print one line
//! `volfog gpu: march A ms, accumulate B ms, apply C ms, total D ms`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The device feature the timing needs (empty unless RAC_FOG_LOG is set and the adapter has it).
pub(in crate::viewer) fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
	if super::frame::fog_log() { wgpu::Features::TIMESTAMP_QUERY & adapter.features() } else { wgpu::Features::empty() }
}

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
	pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<GpuTimer> {
		if !super::frame::fog_log() || !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
			return None;
		}
		let qs = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("volfog time"), ty: wgpu::QueryType::Timestamp, count: 6 });
		let mk = |label, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: 48, usage, mapped_at_creation: false });
		let resolve = mk("volfog time resolve", wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC);
		let read = mk("volfog time read", wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
		Some(GpuTimer { qs, resolve, read, period: queue.get_timestamp_period(), state: State::Idle, ready: Arc::new(AtomicBool::new(false)) })
	}

	/// Called first in every `run`: maps a finished reading, prints a mapped one.
	pub(super) fn poll(&mut self, device: &wgpu::Device) {
		match self.state {
			State::Copied => {
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
						eprintln!("volfog gpu: march {:.3} ms, accumulate {:.3} ms, apply {:.3} ms, total {:.3} ms", ms(0), ms(2), ms(4), ms(0) + ms(2) + ms(4));
					}
					self.read.unmap();
					self.state = State::Idle;
				}
			}
			State::Idle => {}
		}
	}

	/// The timestamp writes of pass `i` (0 march, 1 accumulate, 2 apply) while a reading is being taken this frame.
	pub(super) fn writes(&self, i: u32) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
		(self.state == State::Idle).then(|| wgpu::RenderPassTimestampWrites { query_set: &self.qs, beginning_of_pass_write_index: Some(2 * i), end_of_pass_write_index: Some(2 * i + 1) })
	}

	/// After the three passes: resolves this frame's stamps into the readback buffer (when they were taken).
	pub(super) fn finish(&mut self, enc: &mut wgpu::CommandEncoder) {
		if self.state == State::Idle {
			enc.resolve_query_set(&self.qs, 0..6, &self.resolve, 0);
			enc.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, 48);
			self.state = State::Copied;
		}
	}
}
