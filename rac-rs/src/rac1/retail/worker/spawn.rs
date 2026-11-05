//! `load`, part 4: the worker thread (`spawn_worker`, what it takes along: `ThreadIn`) and its field loop (`worker_main`).

use super::*;

pub(super) struct ThreadIn {
	pub(super) level: u32,
	pub(super) m: ee_oracle::Machine,
	pub(super) hold_card: bool,
	pub(super) gtx: Option<std::sync::mpsc::SyncSender<crate::gsfull::pipe::FieldIn>>,
	pub(super) jobs_rx: Option<std::sync::mpsc::Receiver<ee_oracle::gs::Job>>,
	pub(super) prx: std::sync::mpsc::Receiver<FieldIn>,
	pub(super) booting: bool,
	pub(super) play: Option<playfeed::Feed>,
	pub(super) vush: efx_vu::Shadow,
	pub(super) cold_front: bool,
	pub(super) cold: Option<&'static String>,
	pub(super) dir: &'static String,
	pub(super) cap: Option<Capture>,
	pub(super) gsh: Option<Arc<crate::gsfull::pipe::GsFull>>,
	pub(super) pipe: Option<snap_pipe::Pipe>,
	pub(super) picker: Option<efx_vu::Picker>,
	pub(super) par: Option<snap_pipe::Par>,
	pub(super) layers_used: (u32, Vec<crate::anim::ListLayer>),
	pub(super) swap_lag: Option<SwapWords>,
	pub(super) ptx: std::sync::mpsc::Sender<Snap>,
}

pub(super) fn spawn_worker(inp: ThreadIn) -> Option<std::thread::JoinHandle<()>> {
	let worker = std::thread::Builder::new()
		.name("retail".into())
		// Recompiled retail code nests a Rust frame per retail call.
		.stack_size(256 << 20)
		.spawn(move || {
			worker_main(inp);
		})
		.ok()?;
	Some(worker)
}

fn worker_main(inp: ThreadIn) {
	let mut w = Worker::start(inp);
	while let Ok(fin) = w.prx.recv() {
		let t = w.field_begin(fin);
		let (r, frame_ns) = w.field_ee();
		let in_front = w.field_front(frame_ns);
		let turbo = w.field_load_loop(in_front);
		let own = w.field_post(frame_ns);
		w.field_capture_end(turbo);
		let mut s = w.field_snap(own);
		w.field_levels(&mut s, turbo);
		w.field_overlay(&mut s);
		if w.field_end(s, t, r, frame_ns).is_break() {
			break;
		}
	}
	w.finish();
}
