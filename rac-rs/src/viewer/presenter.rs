//! [PC] Presents on a worker thread (no retail counterpart; render path only, the picture is unchanged).
//!
//! `Queue::present` can block the calling thread until the GPU has finished the frame (seen on a laptop with NVIDIA Optimus,
//! where the driver copies the finished frame to the other adapter inside vkQueuePresentKHR): the CPU then prepares frame N+1
//! only after the GPU drew frame N, so frame time is CPU + GPU instead of max(CPU, GPU). The frame is submitted on the render
//! thread as before; only the `present` call moves here. Ordering is safe: the submit of frame N is already queued, and wgpu
//! serialises submit and present on its own queue lock, so frame N+1's `write_buffer`s and submit still run after frame N's.
//! At most one present is in flight: handing over the next frame first waits for the previous present (`flush`), and the
//! surface is flushed before it is reconfigured (resize) so wgpu never sees an acquired texture still outstanding.
//! Input latency: the game tick for frame N+1 no longer waits for the GPU to finish frame N, so a new frame shows input
//! sampled up to one frame later than the blocked version only when GPU-bound; CPU-bound it is the same or earlier.
//! The thread is on by default only on NVIDIA, where it was measured. Other drivers present inline (the old behaviour): a Windows
//! driver may show the frame through DXGI and need the window's thread inside vkQueuePresentKHR, while that thread waits for the
//! present (`flush`, the submit gate, or wgpu's queue lock that present holds, e.g. in the front end's level load right after the
//! first frame), and neither returns: GitHub #12, v0.3.0 white "Not Responding" window on an RX 9070 XT where v0.2.2 (inline) works.
//! RAC_PRESENT_THREAD=0 presents inline, =1 uses the thread on every GPU; RAC_PRESENT_LOG=1 prints the time spent inside present.
//!
//! [PC] Deferred submit (lane A6): the render thread no longer waits for the frame's `enc-finish` jobs (`Cuts::take`, ~1.3 ms per
//! frame) nor runs the `submit` (~0.3 ms): `Presenter::present_frame` hands the unfinished parts to the present thread, which
//! collects them, submits and presents, while the render thread starts the next frame's game ticks and skinning (CPU only).
//! Ordering stays exact because every use of the queue from the render thread goes through `GatedQueue` (the `Gpu::queue` field):
//! its `Deref` waits until the pending submit has returned, so a `write_buffer` / `submit` of frame N+1 can never land before
//! the command buffers of frame N (wgpu applies `write_buffer`s at the next submit). RAC_SUBMIT_DEFER=0 submits inline (old).
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

pub(super) struct Presenter {
	queue: wgpu::Queue,
	worker: Option<(mpsc::Sender<Job>, JoinHandle<()>)>,
	/// true while a frame is handed over and its present has not returned yet.
	busy: Arc<(Mutex<bool>, Condvar)>,
	gate: Arc<Gate>,
	defer: bool,
}

/// What the present thread does for one frame: the unfinished command-buffer parts (in order) and the last buffer, to submit
/// first (None: already submitted), then the swapchain image to present.
struct Job {
	parts: Vec<mpsc::Receiver<wgpu::CommandBuffer>>,
	last: Option<wgpu::CommandBuffer>,
	/// true: submit the parts (and `last`); false: present only (`Presenter::present`).
	submit: bool,
	frame: wgpu::SurfaceTexture,
	/// [PC] ReShade add-on: where the frame's submit is split and the hook that runs there (`Cuts::split`).
	mid: Option<(usize, Hook)>,
	/// [PC] Overlay writes (gpu/ovwrite.rs) issued up to this frame's hand-over: `submit_parts` waits for exactly these.
	ov: u64,
}

/// [PC] ReShade add-on (gpu/reshade.rs): runs on the submitting thread between the two halves of a frame's submit.
pub(super) type Hook = Box<dyn FnOnce() + Send>;

/// `queue.present` on the calling thread, with its start and length in the RAC_FRAME_CSV file.
fn present_timed(q: &wgpu::Queue, frame: wgpu::SurfaceTexture) {
	let t = Instant::now();
	q.present(frame);
	super::framecsv::present(t, t.elapsed().as_secs_f32() * 1000.0);
}

/// Submits `cmds`; with `mid` = (n, hook): the first n, then the hook, then the rest (the hook's work lands between them on the queue).
fn submit_parts(q: &wgpu::Queue, mut cmds: Vec<wgpu::CommandBuffer>, mid: Option<(usize, Hook)>, ov: u64) {
	// The frame's overlay texture write (a helper thread's, see gpu/ovwrite.rs) must be staged before this submit executes it.
	super::gpu::ovwrite::join_to(ov);
	let Some((n, hook)) = mid else {
		q.submit(cmds);
		return;
	};
	let rest = cmds.split_off(n.min(cmds.len()));
	q.submit(cmds);
	run_hook(hook);
	q.submit(rest);
}

/// Runs the ReShade hook under the GS capture's submit lock. The add-on's `flush_immediate` is a raw vkQueueSubmit on the window's queue
/// with no lock of its own, and ReShade's hooked vkQueueSubmit (the capture's submit, when it shares the window's device) flushes the same
/// immediate list: held across the hook, neither the capture's submit nor `configure_surface` can run inside it. The lock is the outermost
/// one on every path (the hook takes no other lock of ours; ReShade's own callbacks `on_init` / `on_destroy` take none).
fn run_hook(hook: Hook) {
	let _no_capture_submit = crate::retail::gpu_capture_submit_lock();
	hook();
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::{AtomicBool, Ordering};
	use std::sync::Arc;

	/// While the hook runs, a second holder of the capture's submit lock (the capture thread's `submit`) waits for it to return.
	#[test]
	fn the_reshade_hook_runs_under_the_capture_submit_lock() {
		let other_in = Arc::new(AtomicBool::new(false));
		let (flag, seen) = (other_in.clone(), Arc::new(AtomicBool::new(true)));
		let seen2 = seen.clone();
		let (started_tx, started_rx) = std::sync::mpsc::channel();
		let hook: super::Hook = Box::new(move || {
			let f = flag.clone();
			let h = std::thread::spawn(move || {
				started_tx.send(()).unwrap();
				let _g = crate::retail::gpu_capture_submit_lock();
				f.store(true, Ordering::SeqCst);
			});
			started_rx.recv().unwrap();
			std::thread::sleep(std::time::Duration::from_millis(100));
			seen2.store(flag.load(Ordering::SeqCst), Ordering::SeqCst); // the other holder must not have got in yet
			drop(h);
		});
		super::run_hook(hook);
		assert!(!seen.load(Ordering::SeqCst), "the capture's submit entered while the ReShade hook ran");
		for _ in 0..200 {
			if other_in.load(Ordering::SeqCst) {
				return;
			}
			std::thread::sleep(std::time::Duration::from_millis(10));
		}
		panic!("the lock was not released after the hook");
	}

	fn gate() -> Arc<super::Gate> {
		use std::sync::atomic::AtomicU64;
		use std::sync::{Condvar, Mutex};
		Arc::new(super::Gate { pending: AtomicBool::new(false), issued: AtomicU64::new(0), done: AtomicU64::new(0), lock: Mutex::new(()), cv: Condvar::new() })
	}

	/// Runs `m.wait()` on a thread and gives its "returned" flag after `ms` milliseconds (only for checks that it has NOT returned:
	/// a slow machine can make those pass wrongly, never fail wrongly).
	fn waits_for(m: super::GateMark, ms: u64) -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
		let returned = Arc::new(AtomicBool::new(false));
		let r = returned.clone();
		let h = std::thread::spawn(move || {
			m.wait();
			r.store(true, Ordering::SeqCst);
		});
		std::thread::sleep(std::time::Duration::from_millis(ms));
		(returned, h)
	}

	/// A mark taken before a hand-over never waits for it (a helper must not wait on the frame being built); the counters are equal
	/// exactly when nothing is pending. No timing: `wait` is called on the test thread (a wrong wait would hang the test).
	#[test]
	fn a_mark_does_not_include_a_later_hand_over() {
		let g = gate();
		let m0 = g.mark(); // frame N-1's hand-over is not made yet: n = 0
		assert_eq!(m0.n, 0);
		g.set();
		assert_eq!((g.issued.load(Ordering::SeqCst), g.done.load(Ordering::SeqCst)), (1, 0));
		m0.wait(); // the hand-over above is still pending, and this returns
		g.clear();
		assert_eq!((g.issued.load(Ordering::SeqCst), g.done.load(Ordering::SeqCst)), (1, 1));
		assert!(!g.pending.load(Ordering::SeqCst));
	}

	/// A mark taken after a hand-over waits until that submit has returned (`clear`), and not longer.
	#[test]
	fn a_mark_waits_until_its_hand_over_is_submitted() {
		let g = gate();
		g.set();
		let m1 = g.mark();
		assert_eq!(m1.n, 1);
		let (returned, h) = waits_for(m1, 100);
		assert!(!returned.load(Ordering::SeqCst), "the mark returned before the submit");
		g.clear();
		h.join().unwrap();
		assert!(returned.load(Ordering::SeqCst));
		// The next hand-over is not in the old mark's way, a new mark of it is.
		g.set();
		let m2 = g.mark();
		assert_eq!(m2.n, 2);
		let (returned2, h2) = waits_for(m2, 100);
		assert!(!returned2.load(Ordering::SeqCst));
		g.clear();
		h2.join().unwrap();
	}

	/// When the present thread ends (also by a panic) `Release` counts every hand-over as done: no mark waits forever.
	#[test]
	fn release_frees_the_marks() {
		let g = gate();
		g.set();
		g.set(); // cannot happen with the real flush, but the counter logic must still end
		let m = g.mark();
		let (returned, h) = waits_for(m, 100);
		assert!(!returned.load(Ordering::SeqCst));
		drop(super::Release(Arc::new((std::sync::Mutex::new(true), std::sync::Condvar::new())), g.clone()));
		h.join().unwrap();
		assert!(returned.load(Ordering::SeqCst) && !g.pending.load(Ordering::SeqCst));
	}
}

/// True while a frame's command buffers are handed over but not yet submitted.
struct Gate {
	pending: AtomicBool,
	/// Hand-overs made (`set`) and hand-overs whose submit has returned (`clear`): `done == issued` exactly when `pending` is false.
	issued: AtomicU64,
	done: AtomicU64,
	lock: Mutex<()>,
	cv: Condvar,
}

impl Gate {
	fn wait(&self) {
		if !self.pending.load(Ordering::Acquire) {
			return;
		}
		let _t = super::profile::span("sgate");
		let _w = crate::runlog::waiting("the present thread's submit of the last frame (presenter gate)");
		let mut g = self.lock.lock().unwrap();
		while self.pending.load(Ordering::Acquire) {
			g = self.cv.wait(g).unwrap();
		}
	}

	fn set(&self) {
		self.issued.fetch_add(1, Ordering::AcqRel);
		self.pending.store(true, Ordering::Release);
	}

	fn clear(&self) {
		let _g = self.lock.lock().unwrap();
		self.done.store(self.issued.load(Ordering::Acquire), Ordering::Release);
		self.pending.store(false, Ordering::Release);
		self.cv.notify_all();
	}

	/// The gate now: every hand-over made so far (`GateMark`).
	fn mark(self: &Arc<Self>) -> GateMark {
		GateMark { gate: self.clone(), n: self.issued.load(Ordering::Acquire) }
	}

	/// Waits until the hand-over numbered `n` (`GateMark`) has been submitted (or the present thread is gone).
	fn wait_done(&self, n: u64) {
		if self.done.load(Ordering::Acquire) >= n {
			return;
		}
		let mut g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
		while self.done.load(Ordering::Acquire) < n {
			g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
		}
	}
}

/// Clears the `busy` flag and the gate when the present thread ends, also by a panic (a submit or present error): the render
/// thread would otherwise wait for them forever in `flush` / `Gate::wait`. Its next hand-over then finds the channel closed and
/// submits and presents inline.
struct Release(Arc<(Mutex<bool>, Condvar)>, Arc<Gate>);

impl Drop for Release {
	fn drop(&mut self) {
		*self.0 .0.lock().unwrap_or_else(|e| e.into_inner()) = false;
		self.0 .1.notify_all();
		let _g = self.1.lock.lock().unwrap_or_else(|e| e.into_inner());
		self.1.done.store(self.1.issued.load(Ordering::Acquire), Ordering::Release);
		self.1.pending.store(false, Ordering::Release);
		self.1.cv.notify_all();
	}
}

/// [PC] The viewer's queue: derefs to the `wgpu::Queue` after waiting for a deferred submit of the previous frame to return.
pub(super) struct GatedQueue {
	q: wgpu::Queue,
	gate: Arc<Gate>,
}

impl Deref for GatedQueue {
	type Target = wgpu::Queue;
	fn deref(&self) -> &wgpu::Queue {
		self.gate.wait();
		&self.q
	}
}

/// [PC] The gate's state at one moment: every deferred submit handed over before it. `wait` returns once those have all returned
/// from `Queue::submit`, without waiting for a LATER hand-over (a helper thread must not wait on the frame that is being built).
pub(super) struct GateMark {
	gate: Arc<Gate>,
	n: u64,
}

impl GateMark {
	pub(super) fn wait(&self) {
		self.gate.wait_done(self.n);
	}
}

impl GatedQueue {
	/// The gate now (see `GateMark`), for a helper that uses `raw` instead of this queue.
	pub(super) fn mark(&self) -> GateMark {
		self.gate.mark()
	}

	/// The queue without the gate wait: only after `GateMark::wait` of a mark taken on the render thread.
	pub(super) fn raw(&self) -> wgpu::Queue {
		self.q.clone()
	}
}

impl Presenter {
	/// `vendor`: the adapter's PCI vendor id (the thread's default, see the module doc).
	pub(super) fn new(queue: wgpu::Queue, vendor: u32) -> Presenter {
		let gate = Arc::new(Gate { pending: AtomicBool::new(false), issued: AtomicU64::new(0), done: AtomicU64::new(0), lock: Mutex::new(()), cv: Condvar::new() });
		let defer = std::env::var("RAC_SUBMIT_DEFER").map_or(true, |v| v != "0");
		let busy = Arc::new((Mutex::new(false), Condvar::new()));
		let on = std::env::var("RAC_PRESENT_THREAD").map_or(vendor == 0x10de, |v| v != "0");
		eprintln!("present: {}", if on { "on the present thread" } else { "inline (render thread)" });
		let log = std::env::var("RAC_PRESENT_LOG").is_ok_and(|v| v == "1");
		let worker = on.then(|| {
			let (tx, rx) = mpsc::channel::<Job>();
			let (q, b, g) = (queue.clone(), busy.clone(), gate.clone());
			let h = std::thread::Builder::new()
				.name("present".into())
				.spawn(move || {
					let _release = Release(b.clone(), g.clone());
					let (mut n, mut ns) = (0u64, 0u64);
					let mut sub = (0u64, 0u64);
					while let Ok(job) = rx.recv() {
						let Job { parts, last, submit, frame, mid, ov } = job;
						if submit {
							let t0 = Instant::now();
							let mut cmds: Vec<wgpu::CommandBuffer> = parts.into_iter().filter_map(|rx| rx.recv().ok()).collect();
							cmds.extend(last);
							let t1 = t0.elapsed().as_nanos() as u64;
							submit_parts(&q, cmds, mid, ov);
							sub = (sub.0 + t1, sub.1 + t0.elapsed().as_nanos() as u64 - t1);
							g.clear();
						}
						let t = Instant::now();
						q.present(frame);
						let d = t.elapsed().as_nanos() as u64;
						super::framecsv::present(t, d as f32 / 1e6);
						(n, ns) = (n + 1, ns + d);
						if log && n == 300 {
							eprintln!("present-thread: {:.2} ms inside present, {:.2} ms waiting for the parts, {:.2} ms in submit (avg of {n})", ns as f64 / n as f64 / 1e6, sub.0 as f64 / n as f64 / 1e6, sub.1 as f64 / n as f64 / 1e6);
							(n, ns, sub) = (0, 0, (0, 0));
						}
						*b.0.lock().unwrap() = false;
						b.1.notify_all();
					}
				})
				.expect("present thread");
			(tx, h)
		});
		Presenter { queue, worker, busy, gate, defer }
	}

	/// The viewer's queue, gated on the deferred submit (see the module doc).
	pub(super) fn gated(&self) -> GatedQueue {
		GatedQueue { q: self.queue.clone(), gate: self.gate.clone() }
	}

	/// Waits until no present is in flight.
	pub(super) fn flush(&self) {
		let mut g = self.busy.0.lock().unwrap();
		let _w = (*g).then(|| crate::runlog::waiting("the present thread's last present (presenter flush)"));
		while *g {
			g = self.busy.1.wait(g).unwrap();
		}
	}

	/// Presents `frame` (inline, or on the worker once the previous present has returned).
	pub(super) fn present(&self, frame: wgpu::SurfaceTexture) {
		let Some((tx, _)) = &self.worker else {
			present_timed(&self.queue, frame);
			return;
		};
		{
			let _t = super::profile::span("pflush");
			self.flush();
		}
		*self.busy.0.lock().unwrap() = true;
		if let Err(mpsc::SendError(job)) = tx.send(Job { parts: Vec::new(), last: None, submit: false, frame, mid: None, ov: 0 }) {
			*self.busy.0.lock().unwrap() = false;
			present_timed(&self.queue, job.frame);
		}
	}

	/// Submits the frame's command buffers (`cuts` parts in order, then `last`) and presents `frame`. With the worker and
	/// RAC_SUBMIT_DEFER on this returns at once: the present thread collects the parts, submits and presents; any use of the
	/// viewer's `GatedQueue` meanwhile waits for the submit. Otherwise submit here and present as `present`.
	pub(super) fn present_frame(&self, cuts: Cuts, last: wgpu::CommandBuffer, frame: wgpu::SurfaceTexture) {
		let Some((tx, _)) = self.worker.as_ref().filter(|_| self.defer && cuts.mode == 2) else {
			cuts.submit(&self.queue, last);
			self.present(frame);
			return;
		};
		{
			let _t = super::profile::span("pflush");
			self.flush();
		}
		*self.busy.0.lock().unwrap() = true;
		self.gate.set();
		let job = Job { parts: cuts.parts, last: Some(last), submit: true, frame, mid: cuts.mid, ov: super::gpu::ovwrite::issued() };
		if let Err(mpsc::SendError(Job { parts, last, frame, mid, ov, .. })) = tx.send(job) {
			let mut cmds: Vec<wgpu::CommandBuffer> = parts.into_iter().filter_map(|rx| rx.recv().ok()).collect();
			cmds.extend(last);
			submit_parts(&self.queue, cmds, mid, ov);
			self.gate.clear();
			*self.busy.0.lock().unwrap() = false;
			present_timed(&self.queue, frame);
		}
	}
}

impl Presenter {
	/// True when `present_frame_enc` can hand the final encoder to the present thread (worker on, deferred submit, parallel finish).
	pub(super) fn defers_finish(&self, cuts: &Cuts) -> bool {
		self.worker.is_some() && self.defer && cuts.mode == 2
	}

	/// Like `present_frame`, but the frame's final encoder is finished on an enc-finish thread too (it was the one `finish` left on
	/// the render thread, ~1.1 ms a frame): same command buffers in the same order, so the GPU work and the picture are unchanged.
	/// Only when `defers_finish`; the caller finishes `enc` itself otherwise.
	pub(super) fn present_frame_enc(&self, mut cuts: Cuts, enc: wgpu::CommandEncoder, frame: wgpu::SurfaceTexture) {
		let Some((tx, _)) = self.worker.as_ref() else { unreachable!("checked by defers_finish") };
		let (ctx, crx) = mpsc::channel();
		finish_pool().spawn(move || {
			let _ = ctx.send(enc.finish());
		});
		cuts.parts.push(crx);
		{
			let _t = super::profile::span("pflush");
			self.flush();
		}
		*self.busy.0.lock().unwrap() = true;
		self.gate.set();
		let job = Job { parts: cuts.parts, last: None, submit: true, frame, mid: cuts.mid, ov: super::gpu::ovwrite::issued() };
		if let Err(mpsc::SendError(Job { parts, last, frame, mid, ov, .. })) = tx.send(job) {
			let cmds: Vec<wgpu::CommandBuffer> = parts.into_iter().filter_map(|rx| rx.recv().ok()).chain(last).collect();
			submit_parts(&self.queue, cmds, mid, ov);
			self.gate.clear();
			*self.busy.0.lock().unwrap() = false;
			present_timed(&self.queue, frame);
		}
	}
}

impl Drop for Presenter {
	fn drop(&mut self) {
		if let Some((tx, h)) = self.worker.take() {
			drop(tx);
			let _ = h.join();
		}
	}
}

/// [PC] Splits a frame's command encoding into several encoders at pass boundaries so that `CommandEncoder::finish` (wgpu
/// records and validates the whole frame there: ~4 ms of CPU at max settings) can run on other threads while the render
/// thread keeps encoding. The command buffers are submitted together, in order, in the one `submit` the frame had before, so
/// the GPU work, the `write_buffer` order and the picture are unchanged.
/// RAC_ENC_SPLIT=0 one encoder (old), 1 split with serial finish (timing: spans `f_*`), 2 split with parallel finish (default).
pub(super) struct Cuts {
	mode: u8,
	parts: Vec<mpsc::Receiver<wgpu::CommandBuffer>>,
	/// [PC] ReShade add-on: (parts before the split, hook), set by `split`.
	mid: Option<(usize, Hook)>,
}

impl Cuts {
	pub(super) fn new() -> Cuts {
		static MODE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
		// Fewer than 3 hardware threads: nothing to run the finishes on, keep the single encoder.
		let few = std::thread::available_parallelism().map_or(false, |n| n.get() < 3);
		let mode = *MODE.get_or_init(|| std::env::var("RAC_ENC_SPLIT").ok().and_then(|v| v.parse().ok()).unwrap_or(if few { 0 } else { 2 }));
		Cuts { mode, parts: Vec::new(), mid: None }
	}

	/// [PC] ReShade add-on: ends the current encoder like `cut` (also in mode 0, finishing it here when not in mode 2) and
	/// makes the submit of the frame two: everything up to here, then `hook`, then the rest.
	pub(super) fn split(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, hook: Hook) {
		let old = std::mem::replace(enc, device.create_command_encoder(&Default::default()));
		let (tx, rx) = mpsc::channel();
		if self.mode == 2 {
			finish_pool().spawn(move || {
				let _ = tx.send(old.finish());
			});
		} else {
			let _ = tx.send(old.finish());
		}
		self.parts.push(rx);
		self.mid = Some((self.parts.len(), hook));
	}

	/// Submits the frame's command buffers (`take`), in two halves with the hook between them when `split` was used.
	pub(super) fn submit(mut self, q: &wgpu::Queue, last: wgpu::CommandBuffer) {
		let mid = self.mid.take();
		submit_parts(q, self.take(last), mid, super::gpu::ovwrite::issued());
	}

	/// Ends the current encoder (no open pass allowed) and continues with a fresh one in `enc`.
	pub(super) fn cut(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, name: &'static str) {
		if self.mode == 0 {
			return;
		}
		let old = std::mem::replace(enc, device.create_command_encoder(&Default::default()));
		let (tx, rx) = mpsc::channel();
		if self.mode == 1 {
			let _t = super::profile::span(name);
			let _ = tx.send(old.finish());
		} else {
			finish_pool().spawn(move || {
				let _ = tx.send(old.finish());
			});
		}
		self.parts.push(rx);
	}

	/// All command buffers of the frame in order, `last` being the final encoder's.
	pub(super) fn take(mut self, last: wgpu::CommandBuffer) -> Vec<wgpu::CommandBuffer> {
		let _t = super::profile::span("fjoin");
		let _w = crate::runlog::waiting("the frame's parallel command encoders");
		let mut v: Vec<wgpu::CommandBuffer> = self.parts.drain(..).filter_map(|rx| rx.recv().ok()).collect();
		v.push(last);
		v
	}
}

/// Two threads of their own (not rayon's global pool: the game thread's parallel loops must not delay a frame's finish, nor the other way round).
fn finish_pool() -> &'static rayon::ThreadPool {
	static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
	POOL.get_or_init(|| rayon::ThreadPoolBuilder::new().num_threads(2).thread_name(|i| format!("enc-finish-{i}")).build().expect("enc-finish pool"))
}
