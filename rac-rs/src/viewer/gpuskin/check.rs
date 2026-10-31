//! [PC] nprops (RAC_PROPS_SET_CHECK=1, a debug aid): after each props upload, reads the GPU's record and pose buffers back and counts the
//! entries that differ from what they must hold. Three references: the instance mirror (`insts`, both modes); the old pose mirror
//! path, run in parallel on the same inputs (`Shadow`, direct mode; the mirror itself with RAC_PROPS_SET_OLD=1); and, in direct mode,
//! every pose converted afresh from its source (so a reused pose whose token lied shows up too). Prints `PROPS_CHECK ...` every
//! 300 checked uploads and the first differing entries; all counts must stay 0. Costs a GPU round trip per upload.

use super::*;
use super::super::gpu::pose_reuse::{entry, PoseJob, PoseReuse};

pub(in crate::viewer) fn on() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_PROPS_SET_CHECK").is_ok_and(|v| v == "1"))
}

/// The old path's state, fed with the same poses as the direct mode: its mirror is what the old upload would have sent.
#[derive(Default)]
pub(in crate::viewer) struct Shadow {
	reuse: PoseReuse,
	pose: Vec<PoseEntry>,
}

impl Shadow {
	pub(in crate::viewer) fn begin(&mut self) {
		self.reuse.begin();
	}

	pub(in crate::viewer) fn place(&mut self, token: u64, ps: &[[f32; 3]], ns: &[[f32; 3]], n: usize) {
		self.reuse.place(&mut self.pose, token, ps, ns, n, || 0.0);
	}

	pub(in crate::viewer) fn finish(&mut self) {
		self.reuse.finish(&mut self.pose);
	}

	/// The mirror was emptied (a layout rebuild).
	pub(in crate::viewer) fn forget(&mut self) {
		self.pose.clear();
	}
}

#[derive(Default)]
struct Tally {
	uploads: u64,
	entries: u64,
	vs_old: u64,
	vs_src: u64,
	records: u64,
	rec_bad: u64,
	len_bad: u64,
}

static TALLY: std::sync::Mutex<Option<Tally>> = std::sync::Mutex::new(None);

fn read(device: &wgpu::Device, queue: &GatedQueue, buf: &wgpu::Buffer, size: u64) -> Vec<u8> {
	let stage = device.create_buffer(&wgpu::BufferDescriptor { label: Some("props check"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
	let mut enc = device.create_command_encoder(&Default::default());
	enc.copy_buffer_to_buffer(buf, 0, &stage, 0, size);
	{
		let _no_configure = crate::retail::gpu_capture_submit_lock();
		queue.sync().submit(Some(enc.finish()));
	}
	stage.slice(..).map_async(wgpu::MapMode::Read, |_| {});
	device.poll(wgpu::PollType::wait_indefinitely()).ok();
	let out = stage.slice(..).get_mapped_range().expect("mapped").to_vec();
	stage.unmap();
	out
}

/// Entries of `got` (bytes) that differ from `want`; the first few are printed under `what`.
fn diff(what: &str, got: &[u8], want: &[PoseEntry]) -> u64 {
	let mut bad = want.len().abs_diff(got.len() / 24) as u64; // a short read-back must not pass
	for (i, (g, w)) in got.chunks_exact(24).zip(want).enumerate() {
		if g != bytemuck::bytes_of(w) {
			if bad < 3 {
				eprintln!("PROPS_CHECK {what}: entry {i} gpu {:?} want {w:?}", bytemuck::pod_collect_to_vec::<u8, f32>(g));
			}
			bad += 1;
		}
	}
	bad
}

impl PropsGpu {
	/// Compares the buffers with the references after an upload: `jobs` Some = direct mode (the shadow and the sources are the
	/// references), None = old mode (`mirror` is). `total`: the entries placed.
	pub(super) fn check(&self, device: &wgpu::Device, queue: &GatedQueue, insts: &[PropInst], jobs: Option<&[PoseJob]>, mirror: &[PoseEntry], total: usize) {
		let mut t = TALLY.lock().unwrap_or_else(|e| e.into_inner());
		let t = t.get_or_insert_with(Tally::default);
		t.uploads += 1;
		let got = if total > 0 { read(device, queue, &self.pose, (total * 24) as u64) } else { Vec::new() };
		let old: &[PoseEntry] = match (jobs, self.reuse.shadow.as_ref()) {
			(Some(_), Some(sh)) => &sh.pose,
			_ => mirror,
		};
		if old.len() != total {
			t.len_bad += 1;
			eprintln!("PROPS_CHECK old mirror holds {} entries, {total} placed", old.len());
		}
		t.entries += total as u64;
		t.vs_old += diff("vs old mirror", &got, old);
		if let Some(jobs) = jobs {
			let fresh: Vec<PoseEntry> = jobs.iter().flat_map(|j| j.ps[..j.n].iter().zip(&j.ns[..j.n]).map(|(p, q)| entry(p, q))).collect();
			if fresh.len() != total {
				t.len_bad += 1;
				eprintln!("PROPS_CHECK jobs hold {} entries, {total} placed", fresh.len());
			}
			t.vs_src += diff("vs sources", &got, &fresh);
		}
		if !insts.is_empty() {
			let size = std::mem::size_of::<PropInst>();
			let got = read(device, queue, &self.insts, (insts.len() * size) as u64);
			t.records += insts.len() as u64;
			for (i, (g, w)) in got.chunks_exact(size).zip(insts).enumerate() {
				if g != bytemuck::bytes_of(w) {
					if t.rec_bad < 3 {
						eprintln!("PROPS_CHECK record {i} differs");
					}
					t.rec_bad += 1;
				}
			}
		}
		if t.uploads % 300 == 1 {
			eprintln!("PROPS_CHECK uploads {} entries {} mismatch_vs_old_mirror {} mismatch_vs_sources {} records {} record_mismatch {} length_mismatch {}", t.uploads, t.entries, t.vs_old, t.vs_src, t.records, t.rec_bad, t.len_bad);
		}
	}
}
