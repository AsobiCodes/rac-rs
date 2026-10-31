//! [PC] Deferred queue writes (lane lows, part 4): the render thread's `write_buffer` / `write_texture` / `write_buffer_with` calls
//! no longer wait for the present thread's submit of the previous frame.
//!
//! Why the gate existed (presenter.rs): wgpu applies a queue write at the NEXT `Queue::submit`, so a write of frame N+1 made before
//! frame N's submit would show in frame N. Every use of the queue therefore waited until the present thread had submitted frame N
//! (`Gate::wait`), which put the present thread's latency (waiting for the enc-finish jobs, a `finish()` of 4 ms in 2 % of the frames)
//! straight into the render thread's frame (span `sgate`).
//!
//! The order is the only thing that matters, so the writes are kept in a list instead: `Pending` collects (target, offset, bytes) in
//! call order, the hand-over of frame N (`Presenter::present_frame*`) takes the list and gives it to the present thread, and the present
//! thread applies it, in the same order and with the same wgpu calls, right before it submits frame N's command buffers. A write
//! made after the hand-over goes into the next list, so it lands after frame N's submit exactly as the gate guaranteed. The
//! render thread meanwhile never touches the queue between two hand-overs.
//!
//! Anything that needs the queue itself (a `submit`, a read-back, a write it must have seen) calls `GatedQueue::sync`: wait for the
//! gate as before, then apply whatever is pending in order, then use the queue. A write is therefore never behind an operation
//! that was issued after it. The bytes are copied at the call, as wgpu copies them.
//!
//! The list is generic over the target types so the ordering can be unit-tested without a GPU (`Sink`).
//!
//! Three limits keep this exact and small (v0.4.3 review): a write made inside a build error scope is NOT listed (`pipe_guard::in_scope`,
//! `GatedQueue::deferring`: it reaches wgpu inside the scope); a listed texture write that a later whole-texture write from outside
//! the list replaces is dropped (`cancel_textures`, used by the overlay helper); and a byte arena that grew for one big frame is trimmed
//! when its frames are small again (`trim`), so the peak is not kept three to six times (current list, in flight, spares).

use std::ops::RangeBounds;
use std::sync::Mutex;

/// Where a texture write goes inside its texture and how the bytes are laid out (the plain-data part of `write_texture`'s arguments).
#[derive(Clone, Copy)]
pub(super) struct TexMeta {
	pub mip_level: u32,
	pub origin: wgpu::Origin3d,
	pub aspect: wgpu::TextureAspect,
	pub layout: wgpu::TexelCopyBufferLayout,
	pub size: wgpu::Extent3d,
}

enum Op<B, T> {
	Buf { buf: B, offset: u64, at: usize, len: usize },
	Tex { tex: T, meta: TexMeta, at: usize, len: usize },
}

/// What the writes are applied to: the wgpu queue (or a recorder in the tests).
pub(super) trait Sink<B, T> {
	fn buffer(&self, buf: &B, offset: u64, data: &[u8]);
	fn texture(&self, tex: &T, meta: &TexMeta, data: &[u8]);
}

impl Sink<wgpu::Buffer, wgpu::Texture> for wgpu::Queue {
	fn buffer(&self, buf: &wgpu::Buffer, offset: u64, data: &[u8]) {
		self.write_buffer(buf, offset, data);
	}

	fn texture(&self, tex: &wgpu::Texture, m: &TexMeta, data: &[u8]) {
		self.write_texture(wgpu::TexelCopyTextureInfo { texture: tex, mip_level: m.mip_level, origin: m.origin, aspect: m.aspect }, data, m.layout, m.size);
	}
}

/// Arenas with more capacity than this are trimmed once their frames are small again (see `trim`).
const TRIM_ABOVE: usize = 8 << 20;

/// An arena that grew for one big frame (the props index rewrite at a layout change, a first-use texture, the 1.1 MB overlay fallback)
/// keeps that capacity for as long as its list is recycled: with the current list, the one in flight and the spares that is three to
/// six copies of the peak. Gives the memory back when the capacity is above 8 MB and more than 4 times what the list carried last
/// (`used`). The new capacity is twice `used` (at least 1 MB), so the test fails afterwards and a steady stream of frames never
/// reallocates.
fn trim(v: &mut Vec<u8>, used: usize) {
	if v.capacity() > TRIM_ABOVE && v.capacity() > used.saturating_mul(4) {
		v.shrink_to(used.saturating_mul(2).max(1 << 20));
	}
}

/// The writes of one frame, in call order, with their bytes in one arena.
pub(super) struct WriteList<B, T> {
	data: Vec<u8>,
	ops: Vec<Op<B, T>>,
	/// Bytes of the last frame this list carried (set by `apply`), for `trim`.
	last_fill: usize,
	/// How many of `ops` are texture writes (`cancel_textures` scans nothing when there is none).
	tex_ops: usize,
}

impl<B, T> Default for WriteList<B, T> {
	fn default() -> Self {
		WriteList { data: Vec::new(), ops: Vec::new(), last_fill: 0, tex_ops: 0 }
	}
}

impl<B, T> WriteList<B, T> {
	pub(super) fn is_empty(&self) -> bool {
		self.ops.is_empty()
	}

	#[cfg(test)]
	pub(super) fn len(&self) -> usize {
		self.ops.len()
	}

	fn push_buffer(&mut self, buf: B, offset: u64, bytes: &[u8]) {
		let at = self.data.len();
		self.data.extend_from_slice(bytes);
		self.ops.push(Op::Buf { buf, offset, at, len: bytes.len() });
	}

	fn push_texture(&mut self, tex: T, meta: TexMeta, bytes: &[u8]) {
		let at = self.data.len();
		self.data.extend_from_slice(bytes);
		self.ops.push(Op::Tex { tex, meta, at, len: bytes.len() });
		self.tex_ops += 1;
	}

	/// Removes the texture writes `hit` selects and returns how many. Only for a texture that a later write replaces as a whole.
	fn cancel_textures(&mut self, mut hit: impl FnMut(&T) -> bool) -> usize {
		if self.tex_ops == 0 {
			return 0;
		}
		let before = self.ops.len();
		self.ops.retain(|op| !matches!(op, Op::Tex { tex, .. } if hit(tex)));
		let gone = before - self.ops.len();
		self.tex_ops -= gone;
		if self.ops.is_empty() {
			self.data.clear(); // nothing refers to the arena any more
		}
		gone
	}

	/// Gives a peak arena back (see `trim`); the list is empty.
	fn trim(&mut self) {
		trim(&mut self.data, self.last_fill);
	}

	/// Applies every write in the order it was made and empties the list (the arena keeps its capacity).
	pub(super) fn apply(&mut self, sink: &impl Sink<B, T>) {
		for op in self.ops.drain(..) {
			match op {
				Op::Buf { buf, offset, at, len } => sink.buffer(&buf, offset, &self.data[at..at + len]),
				Op::Tex { tex, meta, at, len } => sink.texture(&tex, &meta, &self.data[at..at + len]),
			}
		}
		self.last_fill = self.data.len();
		self.data.clear();
		self.tex_ops = 0;
	}
}

/// The writes made since the last hand-over (any thread), plus spare lists and byte buffers so a frame allocates nothing.
pub(super) struct Pending<B, T> {
	cur: Mutex<WriteList<B, T>>,
	spare: Mutex<Vec<WriteList<B, T>>>,
	scratch: Mutex<Vec<Vec<u8>>>,
}

impl<B, T> Default for Pending<B, T> {
	fn default() -> Self {
		Pending { cur: Mutex::new(WriteList::default()), spare: Mutex::new(Vec::new()), scratch: Mutex::new(Vec::new()) }
	}
}

fn lock<X>(m: &Mutex<X>) -> std::sync::MutexGuard<'_, X> {
	m.lock().unwrap_or_else(|e| e.into_inner())
}

impl<B: Clone, T: Clone> Pending<B, T> {
	/// Appends a buffer write (an empty one is dropped: wgpu ignores it too).
	pub(super) fn write_buffer(&self, buf: &B, offset: u64, bytes: &[u8]) {
		if !bytes.is_empty() {
			lock(&self.cur).push_buffer(buf.clone(), offset, bytes);
		}
	}

	pub(super) fn write_texture(&self, tex: &T, meta: TexMeta, bytes: &[u8]) {
		lock(&self.cur).push_texture(tex.clone(), meta, bytes);
	}

	/// Drops the pending texture writes `hit` selects (not yet taken by a hand-over); returns how many.
	pub(super) fn cancel_textures(&self, hit: impl FnMut(&T) -> bool) -> usize {
		lock(&self.cur).cancel_textures(hit)
	}

	/// Everything pending, in order (None when nothing is); later writes go into a new list.
	pub(super) fn take(&self) -> Option<WriteList<B, T>> {
		let mut cur = lock(&self.cur);
		if cur.is_empty() {
			return None;
		}
		let fresh = lock(&self.spare).pop().unwrap_or_default();
		Some(std::mem::replace(&mut *cur, fresh))
	}

	/// Hands an applied (empty) list back for reuse.
	pub(super) fn recycle(&self, mut list: WriteList<B, T>) {
		if !list.is_empty() {
			return;
		}
		list.trim();
		let mut spare = lock(&self.spare);
		if spare.len() < 4 {
			spare.push(list);
		}
	}

	/// Applies everything pending now, in order (the caller has made sure no hand-over is still waiting to submit: `GatedQueue::sync`).
	pub(super) fn flush(&self, sink: &impl Sink<B, T>) {
		if let Some(mut list) = self.take() {
			list.apply(sink);
			self.recycle(list);
		}
	}

	/// A zeroed byte buffer of `n` bytes (reused from an earlier view).
	fn scratch(&self, n: usize) -> Vec<u8> {
		let mut v = lock(&self.scratch).pop().unwrap_or_default();
		v.clear();
		v.resize(n, 0);
		v
	}

	fn give_back(&self, mut v: Vec<u8>) {
		let used = v.len();
		trim(&mut v, used);
		let mut s = lock(&self.scratch);
		if s.len() < 4 {
			s.push(v);
		}
	}
}

/// A deferred `Queue::write_buffer_with`: the caller fills `slice(..)`, the write is made (ordered) when the view is dropped.
pub(super) struct DeferredView<'a, B: Clone, T: Clone> {
	pending: &'a Pending<B, T>,
	buf: B,
	offset: u64,
	data: Vec<u8>,
}

impl<'a, B: Clone, T: Clone> DeferredView<'a, B, T> {
	pub(super) fn new(pending: &'a Pending<B, T>, buf: &B, offset: u64, size: usize) -> Self {
		DeferredView { pending, buf: buf.clone(), offset, data: pending.scratch(size) }
	}

	pub(super) fn slice<S: RangeBounds<usize>>(&mut self, bounds: S) -> wgpu::WriteOnly<'_, [u8]> {
		wgpu::WriteOnly::from_mut(&mut self.data[..]).into_slice(bounds)
	}
}

impl<B: Clone, T: Clone> Drop for DeferredView<'_, B, T> {
	fn drop(&mut self) {
		self.pending.write_buffer(&self.buf, self.offset, &self.data);
		self.pending.give_back(std::mem::take(&mut self.data));
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::cell::RefCell;

	/// Records what reaches the "queue" and in which order, as (target, offset or mip, bytes).
	#[derive(Default)]
	struct Rec(RefCell<Vec<(u32, u64, Vec<u8>)>>);

	impl Sink<u32, u32> for Rec {
		fn buffer(&self, buf: &u32, offset: u64, data: &[u8]) {
			self.0.borrow_mut().push((*buf, offset, data.to_vec()));
		}

		fn texture(&self, tex: &u32, m: &TexMeta, data: &[u8]) {
			self.0.borrow_mut().push((1000 + *tex, m.mip_level as u64, data.to_vec()));
		}
	}

	fn meta(mip: u32) -> TexMeta {
		TexMeta {
			mip_level: mip,
			origin: wgpu::Origin3d::ZERO,
			aspect: wgpu::TextureAspect::All,
			layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
			size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
		}
	}

	/// Writes reach the sink in the order they were made, buffers and textures mixed, and each with its own bytes.
	#[test]
	fn writes_are_applied_in_call_order() {
		let p: Pending<u32, u32> = Pending::default();
		p.write_buffer(&1, 0, &[1, 2, 3, 4]);
		p.write_texture(&7, meta(2), &[9, 9, 9, 9]);
		p.write_buffer(&1, 0, &[5, 6, 7, 8]); // the same target again: the later bytes must win
		p.write_buffer(&2, 16, &[0xaa; 8]);
		let rec = Rec::default();
		p.flush(&rec);
		let got = rec.0.into_inner();
		assert_eq!(got, vec![(1, 0, vec![1, 2, 3, 4]), (1007, 2, vec![9; 4]), (1, 0, vec![5, 6, 7, 8]), (2, 16, vec![0xaa; 8])]);
	}

	/// The hand-over takes the list: a write made after `take` is NOT in it (it belongs to the next frame), and the two lists applied
	/// one after the other give the order of the calls.
	#[test]
	fn a_write_after_the_take_belongs_to_the_next_list() {
		let p: Pending<u32, u32> = Pending::default();
		p.write_buffer(&1, 0, &[1; 4]);
		let mut frame_n = p.take().expect("frame N's writes");
		p.write_buffer(&1, 0, &[2; 4]); // frame N+1, made while frame N is still being submitted
		assert_eq!(frame_n.len(), 1);
		let rec = Rec::default();
		frame_n.apply(&rec);
		assert_eq!(rec.0.borrow().len(), 1, "frame N+1's write must not be applied with frame N's list");
		p.recycle(frame_n);
		p.flush(&rec);
		assert_eq!(rec.0.into_inner(), vec![(1, 0, vec![1; 4]), (1, 0, vec![2; 4])]);
	}

	/// `sync` before a render-thread submit = `flush`: everything made before it is applied, then nothing is left (nothing applied twice).
	#[test]
	fn a_flush_applies_everything_once() {
		let p: Pending<u32, u32> = Pending::default();
		for k in 0..5u8 {
			p.write_buffer(&3, k as u64 * 4, &[k; 4]);
		}
		let rec = Rec::default();
		p.flush(&rec);
		p.flush(&rec);
		assert!(p.take().is_none());
		let got = rec.0.into_inner();
		assert_eq!(got.len(), 5);
		assert!(got.iter().enumerate().all(|(k, w)| w.1 == k as u64 * 4 && w.2 == vec![k as u8; 4]));
	}

	/// Nothing pending: `take` is None (the hand-over of a frame without writes costs nothing) and empty writes are not kept.
	#[test]
	fn an_empty_list_takes_nothing() {
		let p: Pending<u32, u32> = Pending::default();
		assert!(p.take().is_none());
		p.write_buffer(&1, 0, &[]);
		assert!(p.take().is_none());
	}

	/// A deferred `write_buffer_with` is ordered where its view is dropped, carries what the caller wrote, and a recycled arena is empty.
	#[test]
	fn a_view_is_ordered_at_its_drop() {
		let p: Pending<u32, u32> = Pending::default();
		p.write_buffer(&1, 0, &[1; 4]);
		{
			let mut v = DeferredView::new(&p, &2, 8, 4);
			v.slice(..).copy_from_slice(&[7, 8, 9, 10]);
			p.write_buffer(&3, 0, &[3; 4]); // made while the view is open: before the view's write
		}
		p.write_buffer(&1, 4, &[4; 4]);
		let mut l = p.take().unwrap();
		let rec = Rec::default();
		l.apply(&rec);
		assert_eq!(rec.0.into_inner(), vec![(1, 0, vec![1; 4]), (3, 0, vec![3; 4]), (2, 8, vec![7, 8, 9, 10]), (1, 4, vec![4; 4])]);
		assert!(l.is_empty() && l.data.is_empty());
		p.recycle(l);
		let again = DeferredView::new(&p, &2, 0, 4);
		assert!(again.data.iter().all(|&b| b == 0), "a reused view buffer starts zeroed");
	}

	/// Records nothing (for the size tests).
	struct Null;

	impl Sink<u32, u32> for Null {
		fn buffer(&self, _: &u32, _: u64, _: &[u8]) {}

		fn texture(&self, _: &u32, _: &TexMeta, _: &[u8]) {}
	}

	/// An arena that carried a 9 MB frame keeps its capacity for that frame, is trimmed when it is recycled after a small one, and then
	/// stays put (no reallocation per frame). The view scratch buffers follow the same rule.
	#[test]
	fn a_peak_arena_is_trimmed_once_its_frames_are_small_again() {
		let p: Pending<u32, u32> = Pending::default();
		let mut peak: WriteList<u32, u32> = WriteList::default();
		peak.push_buffer(1, 0, &vec![7u8; 9 << 20]);
		peak.apply(&Null);
		p.recycle(peak);
		let mut l = lock(&p.spare).pop().unwrap();
		assert!(l.data.capacity() >= 9 << 20, "the list that carried the peak keeps what it used");
		l.push_buffer(1, 0, &[1; 64]);
		l.apply(&Null);
		p.recycle(l);
		let mut l = lock(&p.spare).pop().unwrap();
		let small = l.data.capacity();
		assert!((1 << 20..4 << 20).contains(&small), "trimmed to about twice the small frame, at least 1 MB: {small}");
		for k in 0..3u8 {
			l.push_buffer(1, 0, &[k; 64]);
			l.apply(&Null);
			l.trim();
			assert_eq!(l.data.capacity(), small, "no reallocation per frame");
		}
		let mut v = Vec::with_capacity(9 << 20);
		v.resize(100, 0u8);
		p.give_back(v);
		assert!(lock(&p.scratch).pop().unwrap().capacity() < 4 << 20, "a peak scratch buffer is trimmed too");
	}

	/// A whole-texture write made outside the list (the overlay helper) drops the earlier listed writes to that texture and nothing else.
	#[test]
	fn a_listed_texture_write_can_be_cancelled_by_its_target() {
		let p: Pending<u32, u32> = Pending::default();
		assert_eq!(p.cancel_textures(|_| true), 0, "nothing listed");
		p.write_buffer(&1, 0, &[1; 4]);
		assert_eq!(p.cancel_textures(|_| true), 0, "buffer writes are never cancelled");
		p.write_texture(&7, meta(0), &[9; 4]);
		p.write_texture(&8, meta(1), &[8; 4]);
		p.write_texture(&7, meta(2), &[7; 4]);
		p.write_buffer(&2, 0, &[2; 4]);
		assert_eq!(p.cancel_textures(|t| *t == 7), 2);
		let rec = Rec::default();
		p.flush(&rec);
		assert_eq!(rec.0.into_inner(), vec![(1, 0, vec![1; 4]), (1008, 1, vec![8; 4]), (2, 0, vec![2; 4])]);
		p.write_texture(&7, meta(0), &[5; 4]);
		assert_eq!(p.cancel_textures(|t| *t == 7), 1);
		assert!(lock(&p.cur).data.is_empty(), "an emptied list gives its bytes up");
		assert!(p.take().is_none());
	}

	/// Writes from several threads are all kept (none lost), and a thread's own writes stay in its order.
	#[test]
	fn writes_from_several_threads_are_all_kept_in_thread_order() {
		let p: Pending<u32, u32> = Pending::default();
		std::thread::scope(|s| {
			for t in 0..4u32 {
				let p = &p;
				s.spawn(move || {
					for k in 0..200u64 {
						p.write_buffer(&t, k * 4, &(k as u32).to_le_bytes());
					}
				});
			}
		});
		let rec = Rec::default();
		p.flush(&rec);
		let got = rec.0.into_inner();
		assert_eq!(got.len(), 800);
		for t in 0..4u32 {
			let mine: Vec<u64> = got.iter().filter(|w| w.0 == t).map(|w| w.1).collect();
			assert_eq!(mine, (0..200u64).map(|k| k * 4).collect::<Vec<_>>());
		}
	}
}
