//! [PC] perf round 3 (no retail counterpart): the props batch posed in the vertex shader (`vs_props`). Its vertex buffer
//! keeps the static model; per frame only one record per slot (`PropInst`: placement, scale, light word, alpha code,
//! tints, V scroll) and the slots' skinned poses (one copy per distinct pose, shared by every instance of a pose key) are
//! uploaded. `emulate` is the CPU twin of the shader and of `SlotJob::fill` / the hidden, deferred and actor slot writes
//! of `set_props`: the clipper (NOV-5.9c), the purchase prompt batch and the bench's determinism mirror use it.

use super::*;
use super::gpu::HIDDEN_POS;
use super::gpu::pose_reuse::{entry, pose_runs, PoseJob};

pub(in crate::viewer) mod check;

/// Slot record kinds (`PropInst::k[0]` low byte) and the prompt flag (BRIDGE-36 c2: the slot's vertices sit at
/// `HIDDEN_POS` after the pose, the prompt batch drawing them).
pub(super) const KIND_HIDDEN: u32 = 0;
pub(super) const KIND_PROP: u32 = 1;
pub(super) const KIND_ACTOR: u32 = 2;
pub(super) const KIND_DEFERRED: u32 = 3;
pub(super) const FLAG_PROMPT: u32 = 0x100;
/// [PC] A prop record only the sun shadow pass poses (`vs_sh_props`): a moby retail culled for the game camera alone
/// (`Prop::cast`). Every other reader (`vs_props`, `emulate`, `slot_sphere`) takes it as hidden.
pub(super) const KIND_CAST: u32 = 4;

/// One props slot's record (std430 layout of the shader's `PropInst`).
/// `m`: placement (column-major; an actor's world offset in `m[12..15]`); `light`: the vertex light word; `tint`: the
/// colour from vertex `k[1]` on (rgb, colour w); `ptint`: the prop tint (rgb, w 1 = present); `a`: (scale, colour w,
/// V scroll, 0); `k`: (kind | flags, tint start or u32::MAX, first pose entry, posed vertices).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub(super) struct PropInst {
	pub m: [f32; 16],
	pub light: [f32; 4],
	pub tint: [f32; 4],
	pub ptint: [f32; 4],
	pub a: [f32; 4],
	pub k: [u32; 4],
}

impl PropInst {
	pub(super) fn hidden() -> PropInst {
		PropInst { m: [0.0; 16], light: [0.0; 4], tint: [0.0; 4], ptint: [0.0; 4], a: [0.0; 4], k: [KIND_HIDDEN, u32::MAX, 0, 0] }
	}
}

/// A pose entry: model-space position and normal.
pub(super) type PoseEntry = [f32; 6];

/// The vertex `j` of a slot from its model vertex `m0`, exactly as the CPU path writes it (same operations in the same
/// order as `SlotJob::fill` (test oracle, since removed CPU path), the deferred-list overflow and the scene-actor write).
pub(super) fn emulate(s: &PropInst, pose: &[PoseEntry], m0: &Vertex, j: usize) -> Vertex {
	let posed = (j < s.k[3] as usize).then(|| &pose[s.k[2] as usize + j]);
	let mut v = match s.k[0] & 0xff {
		KIND_PROP => {
			let mut v = *m0;
			if let Some(q) = posed {
				v.pos = [q[0], q[1], q[2]];
				v.normal = [q[3], q[4], q[5]];
			}
			v.uv[0] += s.a[3];
			v.uv[1] += s.a[2];
			v.colour[3] = s.a[1];
			v.light = s.light;
			let (m, scale) = (&s.m, s.a[0]);
			let (a, n) = (v.pos.map(|c| c * scale), v.normal);
			for i in 0..3 {
				v.pos[i] = m[i] * a[0] + m[4 + i] * a[1] + m[8 + i] * a[2] + m[12 + i];
				v.normal[i] = m[i] * n[0] + m[4 + i] * n[1] + m[8 + i] * n[2];
			}
			if s.ptint[3] != 0.0 {
				for i in 0..3 {
					v.colour[i] *= s.ptint[i];
				}
			}
			if j >= s.k[1] as usize {
				v.colour = s.tint;
			}
			v
		}
		KIND_ACTOR => {
			let mut v = *m0;
			if let Some(q) = posed {
				v.pos = [q[0] + s.m[12], q[1] + s.m[13], q[2] + s.m[14]];
				v.normal = [q[3], q[4], q[5]];
				v.colour[3] = s.a[1];
				v.light = s.light;
				if j >= s.k[1] as usize {
					v.colour = s.tint;
				}
			}
			v
		}
		KIND_DEFERRED => {
			let mut v = *m0;
			if let Some(q) = posed {
				v.normal = [q[3], q[4], q[5]];
			}
			v.pos = HIDDEN_POS;
			v
		}
		_ => Vertex { pos: HIDDEN_POS, ..*m0 },
	};
	if s.k[0] & FLAG_PROMPT != 0 {
		v.pos = HIDDEN_POS;
	}
	v
}

/// `emulate(..).pos` without the rest of the vertex (the clipper's band scan): the same operations on the position.
pub(super) fn emulate_pos(s: &PropInst, pose: &[PoseEntry], m0: [f32; 3], j: usize) -> [f32; 3] {
	if s.k[0] & FLAG_PROMPT != 0 {
		return HIDDEN_POS;
	}
	let posed = (j < s.k[3] as usize).then(|| &pose[s.k[2] as usize + j]);
	match s.k[0] & 0xff {
		KIND_PROP => {
			let p = posed.map_or(m0, |q| [q[0], q[1], q[2]]);
			let (m, scale) = (&s.m, s.a[0]);
			let a = p.map(|c| c * scale);
			std::array::from_fn(|i| m[i] * a[0] + m[4 + i] * a[1] + m[8 + i] * a[2] + m[12 + i])
		}
		KIND_ACTOR => posed.map_or(m0, |q| [q[0] + s.m[12], q[1] + s.m[13], q[2] + s.m[14]]),
		_ => HIDDEN_POS,
	}
}

/// A world sphere holding every vertex position of a slot (`rad`: the largest model-space distance from the origin of its
/// posed and model positions, `build_records`), or None (hidden, prompt, or an actor with unposed vertices).
pub(super) fn slot_sphere(s: &PropInst, rad: f32) -> Option<([f32; 3], f32)> {
	if s.k[0] & FLAG_PROMPT != 0 || !rad.is_finite() {
		return None;
	}
	let m = &s.m;
	match s.k[0] & 0xff {
		KIND_PROP => {
			// |M a| <= sqrt(lambda_max(M^T M)) |a|, bounded by Gershgorin on the column Gram matrix.
			let c = [0, 4, 8].map(|o| [m[o], m[o + 1], m[o + 2]]);
			let dot = |a: [f32; 3], b: [f32; 3]| (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).abs();
			let g = (0..3).map(|i| (0..3).map(|j| dot(c[i], c[j])).sum::<f32>()).fold(0.0f32, f32::max);
			Some(([m[12], m[13], m[14]], rad * s.a[0].abs() * g.sqrt()))
		}
		KIND_ACTOR => Some(([m[12], m[13], m[14]], rad)),
		_ => None,
	}
}

/// Whether the clipper's per-vertex scan of a slot inside the world sphere `(c, r)` must come out empty: every vertex inside
/// the guard band, or every vertex past one viewport plane (`clip_props`; the margin covers the rounding of the posing and
/// of `ClipView::view`).
pub(super) fn sphere_skips(view: &super::clip::ClipView, c: [f32; 3], r: f32) -> bool {
	let (band, port) = (view.band(), view.viewport());
	let p = view.view(c);
	let r = r * 1.001 + 0.01;
	let (sx, sy) = ((1.0 + band.kx * band.kx).sqrt(), (1.0 + band.ky * band.ky).sqrt());
	let inside = p[2] - r >= band.near && p[2] + r <= band.far && p[0].abs() + r * sx <= band.kx * p[2] && p[1].abs() + r * sy <= band.ky * p[2];
	let (px, py) = ((1.0 + port.kx * port.kx).sqrt(), (1.0 + port.ky * port.ky).sqrt());
	let (ax, ay) = (port.kx * p[2].abs(), port.ky * p[2].abs());
	let out = p[2] + r < port.near || p[2] - r > port.far || p[1] + ay + r * py < 0.0 || -p[1] + ay + r * py < 0.0 || p[0] + ax + r * px < 0.0 || -p[0] + ax + r * px < 0.0;
	inside || out
}

/// The device side: the props pipelines (opaque, both cull modes, the two moby ABE passes), the record and pose storage
/// buffers (grown to the next power of two) and their bind group.
pub(super) struct PropsGpu {
	pub pipeline: wgpu::RenderPipeline,
	pub pipeline_cull: [wgpu::RenderPipeline; 2],
	pub moby_pipelines: [wgpu::RenderPipeline; 2],
	/// The moby passes with a packet cull mode (index 2 * (cull - 1) + pass).
	pub moby_cull: [wgpu::RenderPipeline; 4],
	/// [PC] #29 (gpu/moby_add.rs): the two additive moby passes (index = pass, no face culling).
	pub add: [wgpu::RenderPipeline; 2],
	pub(super) layout: wgpu::BindGroupLayout,
	insts: wgpu::Buffer,
	pose: wgpu::Buffer,
	/// Capacities in records and pose entries.
	cap: (usize, usize),
	pub bind: wgpu::BindGroup,
	/// [PC] perf: which pose entries the last build left as they were (gpu/pose_reuse.rs).
	pub(super) reuse: super::gpu::pose_reuse::PoseReuse,
}

const INST_MIN: usize = 1024;
/// [PC] perf: changed pose ranges closer than this many entries go up in one write (gpu/pose_reuse.rs `spans`).
const POSE_GAP: usize = 64;
/// Instance records (144 bytes each) written together when this few unchanged ones lie between two changed ones.
const INST_GAP: usize = 8;

/// RAC_POSE_FULL=1: every pose written and uploaded each frame, as before the pose tokens (A/B and fault isolation).
pub(super) fn full_pose_upload() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_POSE_FULL").is_ok_and(|v| v == "1"))
}
const POSE_MIN: usize = 16384;

impl PropsGpu {
	pub(super) fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
		let entry = |binding| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::VERTEX,
			ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
			count: None,
		};
		device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("props skin"), entries: &[entry(0), entry(1)] })
	}

	pub(super) fn new(device: &wgpu::Device, layout: wgpu::BindGroupLayout, pipeline: wgpu::RenderPipeline, pipeline_cull: [wgpu::RenderPipeline; 2], moby_pipelines: [wgpu::RenderPipeline; 2], moby_cull: [wgpu::RenderPipeline; 4], add: [wgpu::RenderPipeline; 2]) -> PropsGpu {
		let cap = (INST_MIN, POSE_MIN);
		let (insts, pose, bind) = Self::buffers(device, &layout, cap);
		PropsGpu { pipeline, pipeline_cull, moby_pipelines, moby_cull, add, layout, insts, pose, cap, bind, reuse: Default::default() }
	}

	fn buffers(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, cap: (usize, usize)) -> (wgpu::Buffer, wgpu::Buffer, wgpu::BindGroup) {
		let make = |label, size: usize| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | if check::on() { wgpu::BufferUsages::COPY_SRC } else { wgpu::BufferUsages::empty() }, mapped_at_creation: false });
		let insts = make("props records", cap.0 * std::mem::size_of::<PropInst>());
		let pose = make("props poses", cap.1 * std::mem::size_of::<PoseEntry>());
		let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("props skin"),
			layout,
			entries: &[wgpu::BindGroupEntry { binding: 0, resource: insts.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: pose.as_entire_binding() }],
		});
		(insts, pose, bind)
	}

	/// Grows the buffers to hold `insts` records and `poses` pose entries when they do not (their next power of two); true then:
	/// the new buffers are empty, so the caller writes everything.
	fn grow(&mut self, device: &wgpu::Device, insts: usize, poses: usize) -> bool {
		let grown = insts > self.cap.0 || poses > self.cap.1;
		if grown {
			self.cap = (insts.next_power_of_two().max(self.cap.0), poses.next_power_of_two().max(self.cap.1));
			(self.insts, self.pose, self.bind) = Self::buffers(device, &self.layout, self.cap);
		}
		grown
	}

	/// The record half of an upload: all of `insts` (`whole`) or the slots `reuse.inst_dirty` lists.
	fn upload_insts(&self, queue: &GatedQueue, insts: &[PropInst], whole: bool) {
		let _s = super::profile::span("pw_inst");
		if whole {
			if !insts.is_empty() {
				queue.write_buffer(&self.insts, 0, bytemuck::cast_slice(insts));
			}
		} else {
			let size = std::mem::size_of::<PropInst>();
			for r in super::gpu::pose_reuse::spans(&self.reuse.inst_dirty, INST_GAP) {
				queue.write_buffer(&self.insts, (r.start * size) as u64, bytemuck::cast_slice(&insts[r]));
			}
		}
	}

	/// Writes this frame's records and poses (growing the buffers first when needed). [PC] perf: of the poses only the ranges
	/// the build wrote (`reuse.dirty`, gpu/pose_reuse.rs); all of them into a new buffer.
	pub(super) fn upload(&mut self, device: &wgpu::Device, queue: &GatedQueue, insts: &[PropInst], pose: &[PoseEntry]) {
		let grown = self.grow(device, insts.len(), pose.len());
		self.upload_insts(queue, insts, grown || full_pose_upload());
		let _s = super::profile::span("pw_pose");
		if grown || full_pose_upload() {
			if !pose.is_empty() {
				queue.write_buffer(&self.pose, 0, bytemuck::cast_slice(pose));
			}
		} else {
			for r in super::gpu::pose_reuse::spans(&self.reuse.dirty, POSE_GAP) {
				queue.write_buffer(&self.pose, (r.start * std::mem::size_of::<PoseEntry>()) as u64, bytemuck::cast_slice(&pose[r]));
			}
		}
		drop(_s);
		if check::on() {
			self.check(device, queue, insts, None, pose, pose.len());
		}
	}

	/// [PC] nprops: `upload` for the direct mode (gpu/pose_reuse.rs): the same buffers, writes and bytes, but the pose entries are
	/// converted from their sources (`jobs`: every distinct pose in entry order, `total` entries) straight into the writes' staging
	/// memory, with no CPU copy in between. The runs are `upload`'s `spans` (`pose_runs`); the unwritten poses a run spans
	/// (< POSE_GAP entries) are converted from their sources too: their token says they are bit-equal to what the GPU holds.
	pub(super) fn upload_direct(&mut self, device: &wgpu::Device, queue: &GatedQueue, insts: &[PropInst], jobs: &[PoseJob], total: usize) {
		let grown = self.grow(device, insts.len(), total);
		let whole = grown || full_pose_upload();
		self.upload_insts(queue, insts, whole);
		let _s = super::profile::span("pw_pose");
		if whole {
			if total > 0 {
				self.write_jobs(queue, jobs);
			}
		} else {
			for r in pose_runs(jobs, POSE_GAP) {
				self.write_jobs(queue, &jobs[r]);
			}
		}
		drop(_s);
		if check::on() {
			self.check(device, queue, insts, Some(jobs), &[], total);
		}
	}

	/// One write of the consecutive `jobs` (each n > 0, gapless) into the pose buffer, converted into the staging memory.
	fn write_jobs(&self, queue: &GatedQueue, jobs: &[PoseJob]) {
		let (first, last) = (jobs[0].base, jobs[jobs.len() - 1].base + jobs[jobs.len() - 1].n);
		let size = std::mem::size_of::<PoseEntry>();
		let Some(mut view) = std::num::NonZeroU64::new(((last - first) * size) as u64).and_then(|len| queue.write_buffer_with(&self.pose, (first * size) as u64, len)) else {
			// No staging view (the write is refused or the allocation failed): the plain write reports it as before.
			let all: Vec<PoseEntry> = jobs.iter().flat_map(|j| j.ps[..j.n].iter().zip(&j.ns[..j.n]).map(|(p, q)| entry(p, q))).collect();
			queue.write_buffer(&self.pose, (first * size) as u64, bytemuck::cast_slice(&all));
			return;
		};
		fill_jobs(view.slice(..), jobs);
	}
}

/// The consecutive `jobs`' entries converted into `rest` back to back (`rest` holds exactly their bytes).
fn fill_jobs(mut rest: wgpu::WriteOnly<'_, [u8]>, jobs: &[PoseJob]) {
	for j in jobs {
		let (dst, tail) = rest.split_at(j.n * std::mem::size_of::<PoseEntry>());
		rest = tail;
		dst.into_chunks::<24>().0.write_iter(j.ps[..j.n].iter().zip(&j.ns[..j.n]).map(|(p, q)| bytemuck::cast::<PoseEntry, [u8; 24]>(entry(p, q))));
	}
}

/// The static per-vertex (slot, index in slot) attribute of the props batch.
pub(super) fn slot_attr(ranges: &[std::ops::Range<usize>]) -> Vec<[u32; 2]> {
	let mut out = Vec::with_capacity(ranges.last().map_or(0, |r| r.end));
	for (s, r) in ranges.iter().enumerate() {
		out.extend((0..r.len() as u32).map(|j| [s as u32, j]));
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use super::super::gpu::SlotJob;

	fn model() -> Vertex {
		Vertex { pos: [1.0, 2.0, 3.0], uv: [0.25, 0.5], normal: [0.0, 0.0, 1.0], colour: [0.5, 0.75, 1.0, 0.0], light: [0.0; 4] }
	}

	#[test]
	fn emulate_matches_slot_job_fill() {
		let m = [0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 10.0, 20.0, 30.0, 1.0];
		let ps = [[0.5, 0.25, 2.0], [3.0, -1.0, 0.125]];
		let ns = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
		let job = SlotJob { ps: &ps, ns: &ns, m, scale: 1.5, light: [0.25, 0.5, 0.75, 17.25], alpha_code: 3.0625, prop_tint: Some([0.5, 0.25, 2.0]), tint: Some(([0.1, 0.2, 0.3], 2)), alpha: 0x60, dv: 0.125, du: 0.0625 };
		let models = [model(); 3];
		let mut cpu = [model(); 3];
		job.fill(&mut cpu, &models, 0);
		let inst = PropInst { m, light: job.light, tint: [0.1, 0.2, 0.3, moby_baked_encoding(0x60)], ptint: [0.5, 0.25, 2.0, 1.0], a: [1.5, 3.0625, 0.125, 0.0625], k: [KIND_PROP, 2, 0, 2] };
		let pose: Vec<PoseEntry> = ps.iter().zip(&ns).map(|(p, n)| [p[0], p[1], p[2], n[0], n[1], n[2]]).collect();
		for j in 0..3 {
			assert_eq!(bytemuck::bytes_of(&emulate(&inst, &pose, &models[j], j)), bytemuck::bytes_of(&cpu[j]), "vertex {j}");
		}
	}

	#[test]
	fn emulate_hidden_kinds() {
		let pose = [[9.0, 9.0, 9.0, 0.0, 1.0, 0.0]];
		let h = emulate(&PropInst::hidden(), &pose, &model(), 0);
		assert_eq!(h.pos, HIDDEN_POS);
		assert_eq!(h.normal, [0.0, 0.0, 1.0]);
		let d = emulate(&PropInst { k: [KIND_DEFERRED, u32::MAX, 0, 1], ..PropInst::hidden() }, &pose, &model(), 0);
		assert_eq!((d.pos, d.normal, d.uv), (HIDDEN_POS, [0.0, 1.0, 0.0], [0.25, 0.5]));
		let a = emulate(&PropInst { m: { let mut m = [0.0; 16]; m[12] = 1.0; m }, a: [0.0, 4.0, 0.0, 0.0], tint: [0.5, 0.5, 0.5, -2.0], k: [KIND_ACTOR | FLAG_PROMPT, 0, 0, 1], ..PropInst::hidden() }, &pose, &model(), 0);
		assert_eq!((a.pos, a.normal, a.colour), (HIDDEN_POS, [0.0, 1.0, 0.0], [0.5, 0.5, 0.5, -2.0]));
	}

	#[test]
	fn fill_jobs_writes_the_entries_back_to_back() {
		let pos: Vec<[f32; 3]> = (0..9).map(|i| [i as f32, 0.5, -2.0]).collect();
		let nrm: Vec<[f32; 3]> = (0..9).map(|i| [0.25, i as f32 + 0.75, 1.0]).collect();
		let jobs = [
			PoseJob { base: 4, n: 3, ps: &pos[..3], ns: &nrm[..3], write: true },
			PoseJob { base: 7, n: 1, ps: &pos[3..5], ns: &nrm[3..5], write: false },
			PoseJob { base: 8, n: 5, ps: &pos[4..], ns: &nrm[4..], write: true },
		];
		let want: Vec<PoseEntry> = jobs.iter().flat_map(|j| (0..j.n).map(|k| entry(&j.ps[k], &j.ns[k]))).collect();
		let mut got = vec![0xAAu8; want.len() * std::mem::size_of::<PoseEntry>()];
		fill_jobs(wgpu::WriteOnly::from_mut(&mut got[..]), &jobs);
		assert_eq!(got, bytemuck::cast_slice::<PoseEntry, u8>(&want));
	}

	#[test]
	fn slot_attr_numbers_each_slot() {
		assert_eq!(slot_attr(&[0..2, 2..2, 2..3]), vec![[0, 0], [0, 1], [2, 0]]);
	}
}
