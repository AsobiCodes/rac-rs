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
use std::sync::atomic::{AtomicBool, Ordering};
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
	frame: wgpu::SurfaceTexture,
}

/// True while a frame's command buffers are handed over but not yet submitted.
struct Gate {
	pending: AtomicBool,
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
		self.pending.store(true, Ordering::Release);
	}

	fn clear(&self) {
		let _g = self.lock.lock().unwrap();
		self.pending.store(false, Ordering::Release);
		self.cv.notify_all();
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

impl Presenter {
	/// `vendor`: the adapter's PCI vendor id (the thread's default, see the module doc).
	pub(super) fn new(queue: wgpu::Queue, vendor: u32) -> Presenter {
		let gate = Arc::new(Gate { pending: AtomicBool::new(false), lock: Mutex::new(()), cv: Condvar::new() });
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
						let Job { parts, last, frame } = job;
						if let Some(last) = last {
							let t0 = Instant::now();
							let mut cmds: Vec<wgpu::CommandBuffer> = parts.into_iter().filter_map(|rx| rx.recv().ok()).collect();
							cmds.push(last);
							let t1 = t0.elapsed().as_nanos() as u64;
							q.submit(cmds);
							sub = (sub.0 + t1, sub.1 + t0.elapsed().as_nanos() as u64 - t1);
							g.clear();
						}
						let t = Instant::now();
						q.present(frame);
						let d = t.elapsed().as_nanos() as u64;
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
			self.queue.present(frame);
			return;
		};
		{
			let _t = super::profile::span("pflush");
			self.flush();
		}
		*self.busy.0.lock().unwrap() = true;
		if let Err(mpsc::SendError(job)) = tx.send(Job { parts: Vec::new(), last: None, frame }) {
			*self.busy.0.lock().unwrap() = false;
			self.queue.present(job.frame);
		}
	}

	/// Submits the frame's command buffers (`cuts` parts in order, then `last`) and presents `frame`. With the worker and
	/// RAC_SUBMIT_DEFER on this returns at once: the present thread collects the parts, submits and presents; any use of the
	/// viewer's `GatedQueue` meanwhile waits for the submit. Otherwise submit here and present as `present`.
	pub(super) fn present_frame(&self, cuts: Cuts, last: wgpu::CommandBuffer, frame: wgpu::SurfaceTexture) {
		let Some((tx, _)) = self.worker.as_ref().filter(|_| self.defer && cuts.mode == 2) else {
			self.queue.submit(cuts.take(last));
			self.present(frame);
			return;
		};
		{
			let _t = super::profile::span("pflush");
			self.flush();
		}
		*self.busy.0.lock().unwrap() = true;
		self.gate.set();
		let job = Job { parts: cuts.parts, last: Some(last), frame };
		if let Err(mpsc::SendError(Job { parts, last, frame })) = tx.send(job) {
			let mut cmds: Vec<wgpu::CommandBuffer> = parts.into_iter().filter_map(|rx| rx.recv().ok()).collect();
			cmds.extend(last);
			self.queue.submit(cmds);
			self.gate.clear();
			*self.busy.0.lock().unwrap() = false;
			self.queue.present(frame);
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
}

impl Cuts {
	pub(super) fn new() -> Cuts {
		static MODE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
		// Fewer than 3 hardware threads: nothing to run the finishes on, keep the single encoder.
		let few = std::thread::available_parallelism().map_or(false, |n| n.get() < 3);
		let mode = *MODE.get_or_init(|| std::env::var("RAC_ENC_SPLIT").ok().and_then(|v| v.parse().ok()).unwrap_or(if few { 0 } else { 2 }));
		Cuts { mode, parts: Vec::new() }
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
