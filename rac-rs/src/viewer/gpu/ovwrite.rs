//! [PC] The retail overlay texture's upload (~1.1 ms of CPU per game tick: staging memcpy + copy command) on a helper thread (lane tick).
//!
//! `Gpu::set_retail_overlay` used to stage the new HUD picture (`Queue::write_texture`) on the render thread. The copy only has to be staged before the
//! frame's FINAL submit executes it (wgpu runs a pending write at the next `Queue::submit`), so the render thread now hands the
//! picture to a persistent worker and goes on with the frame; `join_to` (the first thing `presenter::submit_parts` does) waits for
//! it. Picture and order on the GPU are unchanged. The argument why no frame can show the wrong HUD:
//! - The write for frame N must not be staged before frame N-1's final submit has executed: that submit would then carry
//!   frame N's HUD. The old code got this from the `GatedQueue` deref (waits until the deferred submit of N-1 returned). The worker
//!   waits on a `GateMark` taken on the render thread when it hands the job over (frame N-1's hand-over is already counted
//!   there, frame N's own is not yet, so the wait cannot depend on the frame being built: no deadlock with the join below).
//! - The write for frame N must be staged before frame N's final submit: `submit_parts` calls `join_to(seq of frame N's job)`
//!   first, on the present thread (hand-over) or the render thread (inline submit). Jobs are numbered, the worker is in order, and
//!   the present thread joins only the jobs issued up to ITS frame's hand-over (`Job::ov`): a job of frame N+1 (issued while the
//!   present thread still submits frame N) waits for N's submit and is not waited for by it.
//! - A mid-frame submit of frame N (props layout, tie GPU, ...) may execute the staged write earlier: nothing in it samples the
//!   overlay (only the final HUD pass of the frame does), and the content staged is the same.
//! - Smoothed frames between game ticks take no overlay (`take_overlay` is None), so no job: the texture keeps its content.
//! - Why not just `write_texture` on the worker (first version): wgpu 30 holds the queue's pending-writes lock for the whole 1.1 MB
//!   staging memcpy, so the render thread's next `write_buffer` (culling, props) waited for it: the cost only moved. The worker now
//!   fills its own mapped staging buffer (no lock held), records `copy_buffer_to_texture` and submits that one small command
//!   buffer. It runs after frame N-1's final submit (gate mark) and before frame N's (join), so the GPU order is N-1's draws, the
//!   copy, N's draws: the same as the pending-write order was. The staging buffer is ONE persistent device buffer per overlay size
//!   (slot 4 of `Gpu::retail_ov`, recreated with the texture), filled by `Queue::write_buffer_with` (an unzeroed staging allocation, the
//!   pending-writes lock held only to record the copy) before the copy command: nothing is created per tick but that staging
//!   allocation. Its write is ordered like the texture copy (after frame N-1's final submit by the mark, before the copy by the same
//!   submit sequence), and the previous job's copy has been submitted before the worker takes the next job. The submit takes the capture's submit lock like any submit from a
//!   thread other than the window's (`Surface::configure` fails when another thread submits meanwhile, setup.rs).
//! - Deferred queue writes (qdefer.rs): the sync path `Gpu::set_retail_overlay` LISTS its texture write for the present thread, which applies it
//!   before the frame's submit, i.e. after this helper's copy (the helper submits as soon as its gate mark allows). Before the deferral the sync
//!   write was staged first and the helper's submit flushed it ahead of the copy, so the newer (helper) picture won. `write_overlay` therefore
//!   drops the still-listed texture writes to the overlay texture before it hands a job over (`GatedQueue::cancel_texture_writes`: no gate
//!   wait, and no scan unless a texture write is listed). A job replaces the whole texture, so nothing is lost. Invariant: an overlay write
//!   that is NEWER than a job goes through `overlay_texture` first (`join_to`: the job's copy is submitted before the newer write is even
//!   listed); one that is OLDER is cancelled in `write_overlay`. The two callers are exclusive per redraw today (handler/frame_render.rs,
//!   front_mirror.rs), the rule keeps a mode switch inside one redraw safe.
//! A panic in a job (e.g. a wgpu validation error, which the default handler turns into a panic) is caught: the job still counts
//! as done (no wait ever hangs), `DEAD` is set and every later overlay is written on the render thread as with RAC_OV_ASYNC=0.
//! RAC_OV_ASYNC=0 writes on the render thread as before.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Condvar, Mutex, OnceLock};

use crate::viewer::presenter::GateMark;

struct Msg {
	seq: u64,
	mark: GateMark,
	device: wgpu::Device,
	queue: wgpu::Queue,
	tex: wgpu::Texture,
	buf: wgpu::Buffer,
	w: u32,
	rows: u32,
	px: Vec<[u8; 4]>,
}

/// Set once a job panicked: the helper is not used any more (see the module doc).
static DEAD: AtomicBool = AtomicBool::new(false);

/// Jobs numbered so far (render thread only writes) and jobs whose write has been staged (worker writes).
static ISSUED: AtomicU64 = AtomicU64::new(0);
static DONE: AtomicU64 = AtomicU64::new(0);
static DONE_LOCK: Mutex<()> = Mutex::new(());
static DONE_CV: Condvar = Condvar::new();

/// Number of the newest job handed to the worker (0: none yet).
pub(in crate::viewer) fn issued() -> u64 {
	ISSUED.load(Ordering::Acquire)
}

/// Waits until jobs `1..=seq` have staged their writes.
pub(in crate::viewer) fn join_to(seq: u64) {
	if DONE.load(Ordering::Acquire) >= seq {
		return;
	}
	let _t = crate::viewer::profile::span("ov_join");
	let mut g = DONE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	while DONE.load(Ordering::Acquire) < seq {
		g = DONE_CV.wait(g).unwrap_or_else(|e| e.into_inner());
	}
}

/// Marks job `seq` done also when the write panics, so `join_to` never waits forever.
struct Finish(u64);

impl Drop for Finish {
	fn drop(&mut self) {
		let _g = DONE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		// Never backwards (the worker runs the jobs in order; the tests run several `Finish`es in parallel).
		DONE.fetch_max(self.0, Ordering::AcqRel);
		DONE_CV.notify_all();
	}
}

fn run(rx: mpsc::Receiver<Msg>) {
	while let Ok(Msg { seq, mark, device, queue, tex, buf, w, rows, px }) = rx.recv() {
		finish_job(seq, || {
			mark.wait();
			stage_and_submit(&device, &queue, &tex, &buf, w, rows, &px);
		});
	}
}

/// Runs job `seq`; the job counts as done afterwards in every case. A panic in it is caught and sets `DEAD` BEFORE the job is
/// counted (the `Finish` drops last), so the render thread that wakes in `join_to` already sees it.
fn finish_job(seq: u64, job: impl FnOnce()) {
	let _done = Finish(seq);
	if guarded(job) {
		DEAD.store(true, Ordering::Release);
		eprintln!("ov-write: the overlay copy failed; the overlay is written on the render thread from now on");
	}
}

/// Runs `job`; true when it panicked. The panic hook (main.rs) logs one line for it and writes no crash.log (texpack/guard.rs).
fn guarded(job: impl FnOnce()) -> bool {
	crate::texpack::guard::guarded_with("ov-write: the overlay copy panicked (caught, the overlay is written on the render thread from now on)", job).is_err()
}

/// Whether a picture of `len` pixels goes to the helper (a shorter one is left to `write_texture`, which reports the error).
fn usable(len: usize, w: u32, rows: u32) -> bool {
	len >= (w * rows) as usize && !DEAD.load(Ordering::Acquire)
}

/// Fills the persistent staging buffer `buf` (the queue's pending write, no pending-writes lock during the memcpy) and submits
/// the one copy command into the overlay texture: see the module doc.
fn stage_and_submit(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture, buf: &wgpu::Buffer, w: u32, rows: u32, px: &[[u8; 4]]) {
	let (row, src) = ((w * 4) as usize, bytemuck::cast_slice::<_, u8>(px));
	let stride = stride_of(w);
	{
		let size = wgpu::BufferSize::new((stride * rows as usize) as u64).expect("overlay is not empty");
		let mut view = queue.write_buffer_with(buf, 0, size).expect("overlay staging write");
		for r in 0..rows as usize {
			view.slice(r * stride..r * stride + row).copy_from_slice(&src[r * row..(r + 1) * row]);
		}
	}
	let mut enc = device.create_command_encoder(&Default::default());
	let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride as u32), rows_per_image: Some(rows) };
	enc.copy_buffer_to_texture(wgpu::TexelCopyBufferInfo { buffer: buf, layout }, tex.as_image_copy(), wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 });
	let cmd = enc.finish();
	let _no_configure = crate::retail::gpu_capture_submit_lock();
	queue.submit([cmd]);
}

/// Bytes per row of the staging buffer (copy rows start on a 256-byte boundary).
fn stride_of(w: u32) -> usize {
	((w * 4) as usize).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
}

fn write(queue: &crate::viewer::presenter::GatedQueue, tex: &wgpu::Texture, w: u32, rows: u32, px: &[[u8; 4]]) {
	queue.write_texture(
		tex.as_image_copy(),
		bytemuck::cast_slice(px),
		wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(rows) },
		wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 },
	);
}

fn worker() -> Option<&'static mpsc::Sender<Msg>> {
	static TX: OnceLock<Option<mpsc::Sender<Msg>>> = OnceLock::new();
	TX.get_or_init(|| {
		if std::env::var("RAC_OV_ASYNC").is_ok_and(|v| v == "0") {
			return None;
		}
		let (tx, rx) = mpsc::channel();
		std::thread::Builder::new().name("ov-write".into()).spawn(move || run(rx)).ok()?;
		Some(tx)
	})
	.as_ref()
}

impl super::Gpu {
	/// Stages `px` into `tex` (the overlay texture) on the worker; falls back to writing here when there is no worker (or it died).
	pub(super) fn write_overlay(&mut self, tex: &wgpu::Texture, w: u32, rows: u32, px: Vec<[u8; 4]>) {
		if let Some(tx) = worker().filter(|_| usable(px.len(), w, rows)) {
			self.queue.cancel_texture_writes(tex); // the older sync picture, if one is listed in this frame (module doc)
			let seq = ISSUED.load(Ordering::Acquire) + 1;
			let device = self.device.clone();
			let size = (stride_of(w) * rows as usize) as u64;
			let slot = &mut self.retail_ov.as_mut().expect("the overlay texture exists").4;
			let buf = slot.get_or_insert_with(|| {
				device.create_buffer(&wgpu::BufferDescriptor { label: Some("overlay staging"), size, usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
			});
			let buf = buf.clone();
			let msg = Msg { seq, mark: self.queue.mark(), device, queue: self.queue.raw(), tex: tex.clone(), buf, w, rows, px };
			ISSUED.store(seq, Ordering::Release);
			match tx.send(msg) {
				Ok(()) => return,
				Err(mpsc::SendError(m)) => {
					// The worker is gone: count the job as done and write here.
					drop(Finish(m.seq));
					write(&self.queue, tex, w, rows, &m.px);
					return;
				}
			}
		}
		let _sw = crate::viewer::profile::span("ov_write");
		write(&self.queue, tex, w, rows, &px);
	}
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::Ordering;

	/// A panicking job is reported (the worker then marks `DEAD` and the job done), a normal one is not; the panic hook is told it
	/// is an expected one while the job runs (no crash.log).
	#[test]
	fn a_panicking_job_is_caught_and_reported() {
		let hook = std::panic::take_hook();
		std::panic::set_hook(Box::new(|_| {}));
		let (bad, good) = (super::guarded(|| panic!("wgpu error")), super::guarded(|| {}));
		std::panic::set_hook(hook);
		assert!(bad && !good);
		let mut seen = (false, "");
		let _ = super::guarded(|| seen = (crate::texpack::guard::panic_expected(), crate::texpack::guard::expected_note()));
		assert!(seen.0 && seen.1.starts_with("ov-write:"));
		assert!(!crate::texpack::guard::panic_expected());
	}

	/// A job that panics sets `DEAD` before it counts as done (a thread woken in `join_to` sees it), and after that no picture goes
	/// to the helper (`write_overlay` writes inline).
	#[test]
	fn dead_is_set_before_the_job_counts_as_done() {
		let seq = super::issued() + 2_000_000;
		let hook = std::panic::take_hook();
		std::panic::set_hook(Box::new(|_| {}));
		let waiter = std::thread::spawn(move || {
			super::join_to(seq);
			super::DEAD.load(Ordering::Acquire)
		});
		super::finish_job(seq, || panic!("in a job"));
		std::panic::set_hook(hook);
		assert!(waiter.join().unwrap(), "join_to returned before DEAD was set");
		assert!(!super::usable(10, 2, 5));
		super::DEAD.store(false, Ordering::Release);
		assert!(super::usable(10, 2, 5) && !super::usable(9, 2, 5));
	}

	/// `Finish` marks its job done on drop, also while unwinding, and `join_to` then returns.
	#[test]
	fn finish_marks_the_job_done_when_unwinding() {
		let seq = super::issued() + 1_000_000;
		let r = std::panic::catch_unwind(|| {
			let _f = super::Finish(seq);
			panic!("in a job");
		});
		assert!(r.is_err());
		super::join_to(seq);
	}

	#[test]
	fn staging_rows_start_on_256_bytes() {
		assert_eq!(super::stride_of(640), 2560);
		assert_eq!(super::stride_of(1), 256);
		assert_eq!(super::stride_of(65), 512);
	}
}
