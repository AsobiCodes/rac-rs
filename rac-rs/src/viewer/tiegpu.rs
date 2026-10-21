//! [PC] perf (speed lane D5b, step A): the tie vertex rewrite of `Gpu::cull_ties` (geomorph position + VU1 prog7 vertex colour,
//! `ties::morphed` / `ties::TieColours`) on the GPU. The CPU pick stays authoritative: LOD, Q, the draw lists, the sun shadow
//! lists and the stale / full bookkeeping are all unchanged; only the work of writing the 64-byte vertices of a picked range
//! (rayon rewrite + `queue.write_buffer`) moves into one compute dispatch per pass. The maths must be bit-exact, so the kernels
//! are u32 integer soft-float (work/softf/main.rs proved them on 20M random cases): WGSL may fuse a*b+c and divides in 2.5 ULP,
//! so no GPU float arithmetic is used at all. A job outside the proven domain (Q not 0 or in [2^-20, 1], a vertex or colour value
//! outside the checked ranges) stays on the CPU rewrite, and a range last written by the GPU is then rewritten whole on the CPU
//! (`route`), because the CPU mirror `Scene::tie_verts` is stale for GPU-written ranges.
//! Off with RAC_GPU_CULL=0 (also when the adapter lacks compute shaders or the buffers exceed the storage limits).
//! RAC_GPU_CULL_CHECK=1: the CPU rewrite runs on the mirror for every job as before, the GPU result of every dispatched range is
//! read back and compared byte for byte (the whole buffer every 240 passes); `tiegpu_check:` lines on stderr, mismatches must be 0.
//! RAC_GPU_CULL_STATS=1 prints the per-pass counters (instances, draws, jobs, vertices).
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// (tie instance, LOD, Q, positions moved, whole range, routed to the GPU)
pub(super) type Job = (usize, usize, f32, bool, bool, bool);

const Q_MIN: f32 = 9.5367431640625e-7; // 2^-20
const BLOCK: usize = 64;

static COMPUTE_OK: AtomicBool = AtomicBool::new(false);

pub(super) fn note_adapter(adapter: &wgpu::Adapter) {
	COMPUTE_OK.store(adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS), Ordering::Relaxed);
}

fn env_flag(name: &str, default: bool) -> bool {
	match std::env::var(name) {
		Ok(v) => v == "1",
		Err(_) => default,
	}
}

/// The path is wanted on this adapter (RAC_GPU_CULL=0 turns it off).
pub(super) fn wanted() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| !std::env::var("RAC_GPU_CULL").is_ok_and(|v| v == "0")) && COMPUTE_OK.load(Ordering::Relaxed)
}

pub(super) fn check_on() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| env_flag("RAC_GPU_CULL_CHECK", false))
}

fn stats_on() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| env_flag("RAC_GPU_CULL_STATS", false) || check_on())
}

#[derive(Default)]
struct Stats {
	passes: AtomicU64,
	inst: AtomicU64,
	draws: AtomicU64,
	jobs: AtomicU64,
	gpu_jobs: AtomicU64,
	all_jobs: AtomicU64,
	moved_jobs: AtomicU64,
	range_v: AtomicU64,
	work_v: AtomicU64,
	gpu_range_v: AtomicU64,
	gpu_work_v: AtomicU64,
	tasks: AtomicU64,
	checked_v: AtomicU64,
	bad_v: AtomicU64,
	bad_passes: AtomicU64,
	full_checks: AtomicU64,
}
static STATS: std::sync::OnceLock<Stats> = std::sync::OnceLock::new();
fn stats() -> &'static Stats {
	STATS.get_or_init(Stats::default)
}

const WGSL: &str = r#"
@group(0) @binding(0) var<storage, read_write> vbuf: array<u32>;
@group(0) @binding(1) var<storage, read> stat: array<u32>;
@group(0) @binding(2) var<storage, read> caps: array<u32>;
@group(0) @binding(3) var<storage, read> tasks: array<u32>;

fn mul24(a: u32, b: u32) -> vec2<u32> {
	let a1 = a & 0xfffu; let a2 = a >> 12u; let b1 = b & 0xfffu; let b2 = b >> 12u;
	let t0 = a1 * b1;
	let t1 = a1 * b2 + a2 * b1;
	let t2 = a2 * b2;
	let s0 = t0 + ((t1 & 0xfffu) << 12u);
	let s1 = t2 + (t1 >> 12u) + (s0 >> 24u);
	return vec2<u32>(s1, s0 & 0xffffffu);
}
fn vmul_trunc(a: u32, b: u32) -> u32 {
	if (a < 0x00800000u || b < 0x00800000u) { return 0u; }
	let ea = i32(a >> 23u); let eb = i32(b >> 23u);
	let p = mul24((a & 0x7fffffu) | 0x800000u, (b & 0x7fffffu) | 0x800000u);
	var m: u32; var e: i32;
	if (p.x >= 0x800000u) { m = p.x; e = ea + eb - 126; } else { m = (p.x << 1u) | (p.y >> 23u); e = ea + eb - 127; }
	return (u32(e) << 23u) | (m & 0x7fffffu);
}
fn vadd_trunc(x: u32, y: u32) -> u32 {
	var a = x; var b = y;
	if (a < b) { a = y; b = x; }
	if (b < 0x00800000u) { if (a < 0x00800000u) { return 0u; } return a; }
	let ea = a >> 23u; let eb = b >> 23u;
	let d = ea - eb;
	let ma = (a & 0x7fffffu) | 0x800000u; let mb = (b & 0x7fffffu) | 0x800000u;
	let s = ma + (mb >> d);
	var m = s; var e = ea;
	if (s >= 0x1000000u) { m = s >> 1u; e = ea + 1u; }
	return (e << 23u) | (m & 0x7fffffu);
}
fn ieee_mul(a: u32, b: u32) -> u32 {
	let sign = (a ^ b) & 0x80000000u;
	if ((a & 0x7fffffffu) == 0u || (b & 0x7fffffffu) == 0u) { return sign; }
	let ea = i32((a >> 23u) & 255u); let eb = i32((b >> 23u) & 255u);
	let p = mul24((a & 0x7fffffu) | 0x800000u, (b & 0x7fffffu) | 0x800000u);
	var m: u32; var e: i32; var rem: u32; var half: u32;
	if (p.x >= 0x800000u) { m = p.x; e = ea + eb - 126; rem = p.y; half = 0x800000u; }
	else { m = (p.x << 1u) | (p.y >> 23u); e = ea + eb - 127; rem = p.y & 0x7fffffu; half = 0x400000u; }
	if (rem > half || (rem == half && (m & 1u) == 1u)) {
		m += 1u;
		if (m == 0x1000000u) { m = 0x800000u; e += 1; }
	}
	return sign | (u32(e) << 23u) | (m & 0x7fffffu);
}
fn ieee_add(x: u32, y: u32) -> u32 {
	let az = (x & 0x7fffffffu) == 0u; let bz = (y & 0x7fffffffu) == 0u;
	if (az && bz) { return x & y & 0x80000000u; }
	if (az) { return y; }
	if (bz) { return x; }
	var a = x; var b = y;
	if ((x & 0x7fffffffu) < (y & 0x7fffffffu)) { a = y; b = x; }
	let sa = a & 0x80000000u; let sb = b & 0x80000000u;
	let ea = i32((a >> 23u) & 255u); let eb = i32((b >> 23u) & 255u);
	let ma = ((a & 0x7fffffu) | 0x800000u) << 3u;
	var mb = ((b & 0x7fffffu) | 0x800000u) << 3u;
	let d = u32(ea - eb);
	if (d >= 27u) { mb = 1u; } else { mb = (mb >> d) | select(0u, 1u, (mb & ((1u << d) - 1u)) != 0u); }
	var e = ea;
	var s: u32;
	if (sa == sb) {
		s = ma + mb;
		if (s >= 0x08000000u) { s = (s >> 1u) | (s & 1u); e += 1; }
	} else {
		s = ma - mb;
		if (s == 0u) { return 0u; }
		loop {
			if (s >= 0x04000000u) { break; }
			s = s << 1u; e -= 1;
		}
	}
	var m = s >> 3u; let rem = s & 7u;
	if (rem > 4u || (rem == 4u && (m & 1u) == 1u)) {
		m += 1u;
		if (m == 0x1000000u) { m = m >> 1u; e += 1; }
	}
	return sa | (u32(e) << 23u) | (m & 0x7fffffu);
}
fn lane8(c: u32, k: u32) -> u32 { return (c >> (8u * k)) & 255u; }
fn colour_lane(fx: u32, fy: u32, fz: u32, w: u32, zz: u32) -> u32 {
	let hy = vmul_trunc(0x47800000u + fy, 0x3f000000u);
	let hz = vmul_trunc(0x47800000u + fz, 0x3f000000u);
	let m = vadd_trunc(hy, hz);
	let hi = vmul_trunc(m, w);
	let low = vmul_trunc(0x47800000u + fx, zz);
	return vadd_trunc(hi, low) & 0xffu;
}
// n / 128 as f32 bits (n = 0..255, exact).
fn byte_div128(n: u32) -> u32 {
	if (n == 0u) { return 0u; }
	let msb = 31u - countLeadingZeros(n);
	return ((msb + 120u) << 23u) | ((n << (23u - msb)) & 0x7fffffu);
}

@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
	let t = wid.y * 65535u + wid.x;
	if (t >= tasks[0]) { return; }
	let o = 8u + t * 8u;
	if (li >= tasks[o + 1u]) { return; }
	let v = tasks[o] + li;
	let cb = tasks[o + 2u] * 64u;
	let q = tasks[o + 3u]; let w = tasks[o + 4u]; let zz = tasks[o + 5u];
	let flags = tasks[o + 6u];
	let sw = stat[v * 8u + 6u];
	if ((flags & 2u) == 0u && ((sw >> 25u) & 1u) == 0u) { return; }
	let vo = v * 16u;
	if ((flags & 1u) != 0u) {
		for (var c = 0u; c < 3u; c++) {
			vbuf[vo + c] = ieee_add(stat[v * 8u + c], ieee_mul(q, stat[v * 8u + 3u + c]));
		}
	}
	let cx = caps[cb + (sw & 255u)];
	var rgb = vec3<u32>(0u);
	if (((sw >> 24u) & 1u) == 0u) {
		rgb = vec3<u32>(lane8(cx, 0u), lane8(cx, 1u), lane8(cx, 2u));
	} else {
		let cy = caps[cb + ((sw >> 8u) & 255u)];
		let cz = caps[cb + ((sw >> 16u) & 255u)];
		for (var k = 0u; k < 3u; k++) {
			rgb[k] = colour_lane(lane8(cx, k), lane8(cy, k), lane8(cz, k), w, zz);
		}
	}
	vbuf[vo + 8u] = byte_div128(rgb.x);
	vbuf[vo + 9u] = byte_div128(rgb.y);
	vbuf[vo + 10u] = byte_div128(rgb.z);
	vbuf[vo + 11u] = 0xc0000000u;
}
"#;

/// A base position component: zero or 2^-60 .. 2^60. A morph delta: zero or 2^-100 .. 2^100 (with Q >= 2^-20 the product stays
/// normal, and a sum that cancels cannot reach the denormal range because the base is far above the product then).
fn dom(x: f32) -> bool {
	x == 0.0 || (x.abs() >= 8.673617e-19 && x.abs() <= 1.1529215e18)
}
fn dom_morph(x: f32) -> bool {
	x == 0.0 || (x.abs() >= 7.888609e-31 && x.abs() <= 1.2676506e30)
}

/// One instance's 64 slot colours as bytes r | g << 8 | b << 16 (their floors), None when any is outside 0..=255 or the slot count is not 64.
fn pack_caps(cols: &[[f32; 4]]) -> Option<[u32; 64]> {
	if cols.len() != 64 {
		return None;
	}
	let mut out = [0u32; 64];
	for (o, c) in out.iter_mut().zip(cols) {
		let f = [c[0].floor(), c[1].floor(), c[2].floor()];
		if !f.iter().all(|&x| (0.0..=255.0).contains(&x)) {
			return None;
		}
		*o = f[0] as u32 | (f[1] as u32) << 8 | (f[2] as u32) << 16;
	}
	Some(out)
}

pub(super) struct TieGpu {
	pipeline: wgpu::ComputePipeline,
	bgl: wgpu::BindGroupLayout,
	bind: wgpu::BindGroup,
	vbuf: wgpu::Buffer,
	stat: wgpu::Buffer,
	caps: wgpu::Buffer,
	tasks: wgpu::Buffer,
	task_cap: usize,
	stage: Option<(wgpu::Buffer, usize)>,
	/// Per (instance, LOD): the vertex data of the range is inside the proven domain.
	range_ok: Vec<[bool; 3]>,
	/// Per instance: the 64 slot colours are inside 0..=255 (re-checked at every upload).
	caps_ok: Vec<bool>,
	/// Instances whose slot colours changed (`Gpu::relight`) and are not uploaded yet.
	pub caps_dirty: Vec<usize>,
	/// Per (instance, LOD): the range was last written by the GPU, so the CPU mirror is stale for it.
	dirty: Vec<[bool; 3]>,
	pub any_dirty: bool,
	/// Per 64-vertex block: any Q-dependent vertex; per (instance, LOD): the number of Q-dependent vertices.
	qd_block: Vec<bool>,
	qn: Vec<[u32; 3]>,
	tasks_cpu: Vec<u32>,
	passes: u64,
	rng: u64,
	/// Task count uploaded but not yet recorded (`encode` puts the compute pass at the start of the frame encoder).
	pending: std::sync::atomic::AtomicU32,
}

impl TieGpu {
	#[allow(clippy::too_many_arguments)]
	pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, vbuf: &wgpu::Buffer, base: &[[f32; 3]], morph: &[[f32; 3]], slots: &[ties::TieSlot], qdep: &[bool], vranges: &[[std::ops::Range<usize>; 3]], cols: &[Vec<[f32; 4]>]) -> Option<TieGpu> {
		if !wanted() {
			return None;
		}
		let n = base.len();
		let lim = device.limits();
		if n == 0 || vranges.is_empty() || lim.max_storage_buffers_per_shader_stage < 4 || (n * 64) as u64 > lim.max_storage_buffer_binding_size as u64 || (n * 32) as u64 > lim.max_storage_buffer_binding_size as u64 || (vranges.len() * 256) as u64 > lim.max_storage_buffer_binding_size as u64 {
			eprintln!("tiegpu: off (vertices {n}, storage limits)");
			return None;
		}
		let mut stat = vec![0u32; n * 8];
		let mut range_ok = vec![[true; 3]; vranges.len()];
		for (i, r3) in vranges.iter().enumerate() {
			for (lod, r) in r3.iter().enumerate() {
				for k in r.clone() {
					let ([x, y, z], fat) = slots[k];
					let ok = [x, y, z].iter().all(|&s| s < 64) && base[k].iter().all(|&c| dom(c)) && morph[k].iter().all(|&c| dom_morph(c));
					if !ok && range_ok[i][lod] && stats_on() {
						eprintln!("tiegpu: range ({i},{lod}) out of domain at vertex {k}: base {:?} morph {:?} slot {:?}", base[k], morph[k], slots[k]);
					}
					range_ok[i][lod] &= ok;
					let s = &mut stat[k * 8..k * 8 + 8];
					for c in 0..3 {
						s[c] = base[k][c].to_bits();
						s[3 + c] = morph[k][c].to_bits();
					}
					s[6] = (x & 255) as u32 | ((y & 255) as u32) << 8 | ((z & 255) as u32) << 16 | (fat as u32) << 24 | (qdep[k] as u32) << 25;
				}
			}
		}
		let mut qd_block = vec![false; n.div_ceil(BLOCK)];
		for (k, &q) in qdep.iter().enumerate() {
			qd_block[k / BLOCK] |= q;
		}
		let qn: Vec<[u32; 3]> = vranges.iter().map(|r3| std::array::from_fn(|l| qdep[r3[l].clone()].iter().filter(|&&q| q).count() as u32)).collect();
		let mk = |label: &str, bytes: usize| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (bytes as u64).max(16).next_multiple_of(4), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let stat_buf = mk("tie rewrite static", n * 32);
		queue.write_buffer(&stat_buf, 0, bytemuck::cast_slice(&stat));
		let caps_buf = mk("tie rewrite caps", cols.len() * 256);
		let mut caps_ok = vec![false; cols.len()];
		let mut caps_cpu = vec![0u32; cols.len() * 64];
		for (i, c) in cols.iter().enumerate() {
			if let Some(p) = pack_caps(c) {
				caps_ok[i] = true;
				caps_cpu[i * 64..i * 64 + 64].copy_from_slice(&p);
			}
		}
		queue.write_buffer(&caps_buf, 0, bytemuck::cast_slice(&caps_cpu));
		let task_cap = 4096;
		let tasks = mk("tie rewrite tasks", (task_cap * 8 + 8) * 4);
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("tie rewrite"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("tie rewrite", WGSL.into())) });
		let entry = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None }, count: None };
		let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("tie rewrite"), entries: &[entry(0, false), entry(1, true), entry(2, true), entry(3, true)] });
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("tie rewrite"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
		let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("tie rewrite"), layout: Some(&pl), module: &module, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
		let bind = Self::make_bind(device, &bgl, vbuf, &stat_buf, &caps_buf, &tasks);
		let bad_ranges = range_ok.iter().flatten().filter(|&&o| !o).count();
		let bad_caps = caps_ok.iter().filter(|&&o| !o).count();
		eprintln!("tiegpu: on ({n} vertices, {} instances, {bad_ranges} ranges and {bad_caps} instances stay on the CPU path{})", vranges.len(), if check_on() { ", CHECK" } else { "" });
		Some(TieGpu { pipeline, bgl, bind, vbuf: vbuf.clone(), stat: stat_buf, caps: caps_buf, tasks, task_cap, stage: None, range_ok, caps_ok, caps_dirty: Vec::new(), dirty: vec![[false; 3]; vranges.len()], any_dirty: false, qd_block, qn, tasks_cpu: Vec::new(), passes: 0, rng: 0x9e3779b97f4a7c15, pending: std::sync::atomic::AtomicU32::new(0) })
	}

	fn make_bind(device: &wgpu::Device, bgl: &wgpu::BindGroupLayout, vbuf: &wgpu::Buffer, stat: &wgpu::Buffer, caps: &wgpu::Buffer, tasks: &wgpu::Buffer) -> wgpu::BindGroup {
		fn e(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
			wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }
		}
		device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("tie rewrite"), layout: bgl, entries: &[e(0, vbuf), e(1, stat), e(2, caps), e(3, tasks)] })
	}

	/// Uploads the slot colours of the instances `relight` changed since the last pass.
	pub fn flush_caps(&mut self, queue: &wgpu::Queue, cols: &[Vec<[f32; 4]>]) {
		for i in std::mem::take(&mut self.caps_dirty) {
			match pack_caps(&cols[i]) {
				Some(p) => {
					self.caps_ok[i] = true;
					queue.write_buffer(&self.caps, (i * 256) as u64, bytemuck::cast_slice(&p));
				}
				None => self.caps_ok[i] = false,
			}
		}
	}

	/// Decides whether a job goes to the GPU. A job that stays on the CPU for a range the GPU wrote last turns into a whole-range
	/// rewrite (positions too when the LOD morphs), since the mirror is stale there.
	pub fn route(&mut self, (i, lod, q, moved, all, _): Job, fat: bool, check: bool) -> Job {
		let eligible = self.range_ok[i][lod] && self.caps_ok[i] && (q == 0.0 || (Q_MIN..=1.0).contains(&q));
		if eligible {
			if !check {
				self.dirty[i][lod] = true;
				self.any_dirty = true;
			}
			(i, lod, q, moved, all, true)
		} else if self.dirty[i][lod] {
			self.dirty[i][lod] = false;
			(i, lod, q, fat, true, false)
		} else {
			(i, lod, q, moved, all, false)
		}
	}

	/// Builds the task list of the GPU jobs, uploads it and runs the dispatch; in check mode also compares the written ranges with
	/// the CPU mirror (which the caller has rewritten for every job).
	pub fn dispatch(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, jobs: &[Job], vranges: &[[std::ops::Range<usize>; 3]], mirror: &[Vertex], check: bool) -> u64 {
		self.flush_pending(device, queue);
		self.tasks_cpu.clear();
		self.tasks_cpu.extend([0u32; 8]);
		let mut nt = 0u32;
		let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
		for &(i, lod, q, moved, all, g) in jobs {
			if !g {
				continue;
			}
			let r = vranges[i][lod].clone();
			if r.is_empty() {
				continue;
			}
			let w = crate::physics::vmul(256.0, q);
			let zz = crate::physics::vsub1(256.0, w);
			let flags = moved as u32 | (all as u32) << 1;
			for b in r.start / BLOCK..=(r.end - 1) / BLOCK {
				if !all && !self.qd_block[b] {
					continue;
				}
				let (lo, hi) = (r.start.max(b * BLOCK), r.end.min(b * BLOCK + BLOCK));
				self.tasks_cpu.extend([lo as u32, (hi - lo) as u32, i as u32, q.to_bits(), w.to_bits(), zz.to_bits(), flags, 0]);
				nt += 1;
			}
			ranges.push(r);
		}
		if nt == 0 {
			return 0;
		}
		self.tasks_cpu[0] = nt;
		if nt as usize > self.task_cap {
			// Grow; the bind group is rebuilt around the new task buffer.
			self.task_cap = (nt as usize).next_power_of_two();
			self.tasks = device.create_buffer(&wgpu::BufferDescriptor { label: Some("tie rewrite tasks"), size: ((self.task_cap * 8 + 8) * 4) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
			self.rebind(device);
		}
		queue.write_buffer(&self.tasks, 0, bytemuck::cast_slice(&self.tasks_cpu));
		self.pending.store(nt, Ordering::Relaxed);
		self.passes += 1;
		if !check {
			return nt as u64;
		}
		let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tie rewrite") });
		self.encode(&mut enc);
		let full = check && self.passes % 240 == 1;
		let mut copies: Vec<(std::ops::Range<usize>, u64)> = Vec::new();
		if check {
			let vsz = std::mem::size_of::<Vertex>();
			let list: Vec<std::ops::Range<usize>> = if full { vec![0..mirror.len()] } else { ranges };
			let total: usize = list.iter().map(|r| r.len() * vsz).sum();
			if self.stage.as_ref().is_none_or(|s| s.1 < total) {
				let cap = total.next_power_of_two().max(1 << 16);
				self.stage = Some((device.create_buffer(&wgpu::BufferDescriptor { label: Some("tie rewrite check"), size: cap as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false }), cap));
			}
			let stage = &self.stage.as_ref().unwrap().0;
			let mut at = 0u64;
			for r in list {
				let bytes = (r.len() * vsz) as u64;
				enc.copy_buffer_to_buffer(&self.vbuf, (r.start * vsz) as u64, stage, at, bytes);
				copies.push((r, at));
				at += bytes;
			}
		}
		queue.submit([enc.finish()]);
		if check {
			let st = stats();
			let stage = &self.stage.as_ref().unwrap().0;
			stage.slice(..).map_async(wgpu::MapMode::Read, |_| {});
			device.poll(wgpu::PollType::wait_indefinitely()).ok();
			let data = stage.slice(..).get_mapped_range().expect("check map");
			let vsz = std::mem::size_of::<Vertex>();
			let mut bad_v = 0u64;
			let mut checked = 0u64;
			for (r, at) in &copies {
				let got: &[Vertex] = bytemuck::cast_slice(&data[*at as usize..*at as usize + r.len() * vsz]);
				for (k, (g, m)) in got.iter().zip(&mirror[r.clone()]).enumerate() {
					checked += 1;
					if bytemuck::bytes_of(g) != bytemuck::bytes_of(m) {
						bad_v += 1;
						if st.bad_v.load(Ordering::Relaxed) + bad_v <= 12 {
							eprintln!("tiegpu_check: vertex {} differs: gpu pos {:?} col {:?} / cpu pos {:?} col {:?}", r.start + k, g.pos.map(f32::to_bits), g.colour.map(f32::to_bits), m.pos.map(f32::to_bits), m.colour.map(f32::to_bits));
						}
					}
				}
			}
			drop(data);
			stage.unmap();
			st.checked_v.fetch_add(checked, Ordering::Relaxed);
			st.bad_v.fetch_add(bad_v, Ordering::Relaxed);
			if bad_v > 0 {
				st.bad_passes.fetch_add(1, Ordering::Relaxed);
			}
			if full {
				st.full_checks.fetch_add(1, Ordering::Relaxed);
			}
		}
		nt as u64
	}

	/// The last pass was never encoded into a frame (the frame was dropped): run it now, before anything else writes the tie vertices
	/// or its task buffer is reused (`cull_ties` calls this first).
	pub fn flush_pending(&self, device: &wgpu::Device, queue: &wgpu::Queue) {
		if self.pending.load(Ordering::Relaxed) > 0 {
			let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tie rewrite") });
			self.encode(&mut enc);
			queue.submit([enc.finish()]);
		}
	}

	/// Records the compute pass of the tasks uploaded by the last `dispatch` (first thing in the frame's encoder, before any pass that reads the tie vertices).
	pub fn encode(&self, enc: &mut wgpu::CommandEncoder) {
		let nt = self.pending.swap(0, Ordering::Relaxed);
		if nt == 0 {
			return;
		}
		let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("tie rewrite"), timestamp_writes: None });
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, &self.bind, &[]);
		pass.dispatch_workgroups(nt.min(65535), nt.div_ceil(65535), 1);
	}

	fn rebind(&mut self, device: &wgpu::Device) {
		self.bind = Self::make_bind(device, &self.bgl, &self.vbuf, &self.stat, &self.caps, &self.tasks);
	}

	/// Counters for one pass.
	pub fn note_pass(&self, instances: usize, draws: usize, jobs: &[Job], vranges: &[[std::ops::Range<usize>; 3]], tasks: u64) {
		if !stats_on() {
			return;
		}
		let st = stats();
		let (mut a, mut m, mut gj, mut rv, mut wv, mut grv, mut gwv) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
		for &(i, lod, _, moved, all, g) in jobs {
			let len = vranges[i][lod].len() as u64;
			let work = if all { len } else { self.qn[i][lod] as u64 };
			a += all as u64;
			m += moved as u64;
			rv += len;
			wv += work;
			if g {
				gj += 1;
				grv += len;
				gwv += work;
			}
		}
		st.inst.fetch_add(instances as u64, Ordering::Relaxed);
		st.draws.fetch_add(draws as u64, Ordering::Relaxed);
		st.jobs.fetch_add(jobs.len() as u64, Ordering::Relaxed);
		st.gpu_jobs.fetch_add(gj, Ordering::Relaxed);
		st.all_jobs.fetch_add(a, Ordering::Relaxed);
		st.moved_jobs.fetch_add(m, Ordering::Relaxed);
		st.range_v.fetch_add(rv, Ordering::Relaxed);
		st.work_v.fetch_add(wv, Ordering::Relaxed);
		st.gpu_range_v.fetch_add(grv, Ordering::Relaxed);
		st.gpu_work_v.fetch_add(gwv, Ordering::Relaxed);
		st.tasks.fetch_add(tasks, Ordering::Relaxed);
		let n = st.passes.fetch_add(1, Ordering::Relaxed) + 1;
		if n % 300 == 0 {
			let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
			eprintln!(
				"tiegpu: {} passes, per pass: instances {}, draws {}, jobs {:.1} (gpu {:.1}, whole-range {:.1}, moved {:.1}), vertices in ranges {:.0} (gpu {:.0}), worked {:.0} (gpu {:.0}), tasks {:.1}",
				n,
				g(&st.inst) / n,
				g(&st.draws) / n,
				g(&st.jobs) as f64 / n as f64,
				g(&st.gpu_jobs) as f64 / n as f64,
				g(&st.all_jobs) as f64 / n as f64,
				g(&st.moved_jobs) as f64 / n as f64,
				g(&st.range_v) as f64 / n as f64,
				g(&st.gpu_range_v) as f64 / n as f64,
				g(&st.work_v) as f64 / n as f64,
				g(&st.gpu_work_v) as f64 / n as f64,
				g(&st.tasks) as f64 / n as f64
			);
			if check_on() {
				eprintln!("tiegpu_check: {} vertices compared, {} MISMATCHES in {} passes, {} full-buffer compares", g(&st.checked_v), g(&st.bad_v), g(&st.bad_passes), g(&st.full_checks));
			}
		}
	}

	/// Check mode only: extra whole-range jobs with random Q (0, 1, tiny, near 1, anything) on fat ranges that are not drawn this
	/// frame, so the position and fat-colour kernels are compared on Q values the replay never produces. The caller marks them
	/// stale (Q = NaN, not full) so their next real pick rewrites them.
	pub fn stress_jobs(&mut self, tie_lod: &[u8], fat: &[[bool; 3]], n: usize) -> Vec<Job> {
		let mut next = || {
			self.rng ^= self.rng << 13;
			self.rng ^= self.rng >> 7;
			self.rng ^= self.rng << 17;
			self.rng
		};
		let mut out: Vec<Job> = Vec::new();
		for _ in 0..n * 8 {
			if out.len() >= n {
				break;
			}
			let (i, lod) = ((next() % fat.len() as u64) as usize, (next() % 3) as usize);
			if !fat[i][lod] || tie_lod[i] as usize == lod || !self.range_ok[i][lod] || !self.caps_ok[i] || out.iter().any(|j| j.0 == i && j.1 == lod) {
				continue;
			}
			let r = next();
			let q = match r % 6 {
				0 => 0.0,
				1 => 1.0,
				2 => Q_MIN * (1.0 + (r >> 8 & 0xffff) as f32 / 65536.0),
				3 => 1.0 - (r >> 8 & 0xffff) as f32 / 65536.0 * 1e-3,
				_ => (next() >> 40) as f32 / (1u64 << 24) as f32,
			};
			if q != 0.0 && !(Q_MIN..=1.0).contains(&q) {
				continue;
			}
			out.push((i, lod, q, true, true, true));
		}
		out
	}

	/// `tag_ice` on a scene whose tie buffer the GPU wrote: only the ice mark (light w) of the changed vertices goes up; the rest of
	/// the mirror is stale.
	pub fn dirty_mirror(&self) -> bool {
		self.any_dirty
	}
}
