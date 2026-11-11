//! [PC] Per-frame vertex / index buffers without a `create_buffer` each (lane rcpu).
//!
//! The level pass built 10-30 small buffers a frame (water quads, particles, pool lines, shrub billboards, strips, HUD), each a
//! fresh wgpu buffer plus a quad index list built on the CPU. Here they come from a pool: every buffer handed out in frame N
//! goes back to the free list at the start of frame N+1 (`frame_begin`, called by `Gpu::draw`) and is handed out again,
//! by usage and size class (the next power of two, at least 256 bytes). Exact by order: frame N+1's write to a reused buffer
//! is listed after frame N's hand-over (qdefer.rs), so wgpu applies it after frame N's submit, which read the old bytes.
//! Every draw reads only its own range (draw counts, never the buffer size), so the larger buffers draw the same picture.
//! A buffer not reused in a frame is dropped (the pool is just the last frame's set). Quad lists share one index buffer
//! (`quad_indices`: q*4 + [0, 1, 2, 0, 2, 3], grown by doubling, written once).
//! Bypassed (plain buffers, as before) while the split view is on (its Classic half draws a second frame between two
//! hand-overs) and with RAC_FRAMEBUF_OLD=1 (the old path, for A/B).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::GatedQueue;

type Key = (u32, u64);

#[derive(Default)]
struct Pool {
	device: Option<wgpu::Device>,
	free: HashMap<Key, Vec<wgpu::Buffer>>,
	used: Vec<(Key, wgpu::Buffer)>,
	quads: Option<(wgpu::Buffer, u32)>,
	split: bool,
}

static POOL: Mutex<Option<Pool>> = Mutex::new(None);

fn old() -> bool {
	static OLD: OnceLock<bool> = OnceLock::new();
	*OLD.get_or_init(|| std::env::var("RAC_FRAMEBUF_OLD").is_ok_and(|v| v == "1"))
}

fn class(len: u64) -> u64 {
	len.max(256).next_power_of_two()
}

fn plain(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
	device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(4).next_multiple_of(4), usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
}

/// Start of a frame (`Gpu::draw`): last frame's buffers become the free list (older unused ones are dropped).
pub(crate) fn frame_begin(device: &wgpu::Device) {
	if old() {
		return;
	}
	let split = crate::splitview::active();
	let mut g = POOL.lock().unwrap_or_else(|e| e.into_inner());
	let p = g.get_or_insert_with(Pool::default);
	if p.device.as_ref() != Some(device) || split {
		*p = Pool { device: (!split).then(|| device.clone()), split, ..Pool::default() };
		return;
	}
	p.split = false;
	p.free.clear();
	for (k, b) in p.used.drain(..) {
		p.free.entry(k).or_default().push(b);
	}
}

/// A buffer for this frame only, at least `len` bytes, with `usage | COPY_DST`; its bytes are undefined until written.
pub(crate) fn frame_buffer(device: &wgpu::Device, label: &str, len: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
	if old() {
		return plain(device, label, len, usage);
	}
	let mut g = POOL.lock().unwrap_or_else(|e| e.into_inner());
	let Some(p) = g.as_mut().filter(|p| !p.split && p.device.as_ref() == Some(device)) else {
		return plain(device, label, len, usage);
	};
	let key = (usage.bits(), class(len));
	let b = p.free.get_mut(&key).and_then(Vec::pop).unwrap_or_else(|| plain(device, label, key.1, usage));
	p.used.push((key, b.clone()));
	b
}

/// `frame_buffer` with `data` written at 0 (through the deferred queue, as the old `buffer` + `write_buffer` pairs).
pub(crate) fn upload(device: &wgpu::Device, queue: &GatedQueue, label: &str, data: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
	let b = frame_buffer(device, label, data.len() as u64, usage);
	queue.write_buffer(&b, 0, data);
	b
}

/// The index list of `n` quads (q*4 + [0, 1, 2, 0, 2, 3]), 6n indices. Pooled: one shared buffer, grown by doubling.
pub(crate) fn quad_indices(device: &wgpu::Device, queue: &GatedQueue, label: &str, n: u32) -> wgpu::Buffer {
	let make = |n: u32| -> (wgpu::Buffer, Vec<u32>) {
		let idx: Vec<u32> = (0..n).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
		(plain(device, label, idx.len() as u64 * 4, wgpu::BufferUsages::INDEX), idx)
	};
	let pooled = !old() && POOL.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|p| !p.split && p.device.as_ref() == Some(device));
	if !pooled {
		let (b, idx) = make(n);
		queue.write_buffer(&b, 0, bytemuck::cast_slice(&idx));
		return b;
	}
	let mut g = POOL.lock().unwrap_or_else(|e| e.into_inner());
	let p = g.as_mut().expect("checked above");
	match &p.quads {
		Some((b, cap)) if *cap >= n => b.clone(),
		_ => {
			let cap = n.max(256).next_power_of_two();
			let (b, idx) = make(cap);
			queue.write_buffer(&b, 0, bytemuck::cast_slice(&idx));
			p.quads = Some((b.clone(), cap));
			b
		}
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn size_class() {
		assert_eq!(super::class(0), 256);
		assert_eq!(super::class(257), 512);
		assert_eq!(super::class(4096), 4096);
	}
}
