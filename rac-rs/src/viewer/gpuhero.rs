//! [PC] perf (lane R2): the hero marker built on the GPU. `Gpu::set_marker` builds the marker's world-space vertices on the CPU
//! every display frame (blend of the two step poses, turn, light / tint / ghost colour, upload). With frame smoothing that is
//! 2-3 times per game step. Here a compute pass writes `marker.vbuf` instead: the model and the step's two poses (before / now)
//! are uploaded once per change, a display frame only writes the 368-byte parameter block (placement, light, colours, blend factor).
//! `build_cpu` in gpu.rs stays the reference (and the fallback when the hero crosses the clip guard band, needing CPU vertices);
//! RAC_HERO_GPU=0 keeps the CPU path everywhere, RAC_HERO_GPU_CHECK=1 compares the GPU vertices with the CPU ones every frame.

use super::*;

/// RAC_HERO_GPU: unset = `Auto` (GPU build, verified by a self-check on first use), `0` = `Off` (CPU build always), `1` = `On` (GPU
/// build, no self-check: for measuring). RAC_HERO_GPU_CHECK=1 = `Check` (GPU build, compared with the CPU build every frame).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
	Off,
	Auto,
	On,
	Check,
}

pub(super) fn mode() -> Mode {
	static S: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
	*S.get_or_init(|| {
		if std::env::var("RAC_HERO_GPU_CHECK").is_ok_and(|v| v == "1") {
			Mode::Check
		} else {
			match std::env::var("RAC_HERO_GPU").ok().as_deref() {
				Some("0") => Mode::Off,
				Some("1") => Mode::On,
				_ => Mode::Auto,
			}
		}
	})
}

/// Frames the self-check compares per kind of frame (see `kind`).
const SELF_FRAMES: u32 = 60;
const KINDS: usize = 8;
const KIND_NAMES: [&str; 3] = ["blend", "tilt", "extras"];

/// The adapter can run the build (set by `device_created`); false until then.
static AVAILABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The self-check found a difference: the CPU path for the rest of the run (until the next device).
static FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Self-check frames still to compare, per kind of frame.
static LEFT: [std::sync::atomic::AtomicU32; KINDS] = [const { std::sync::atomic::AtomicU32::new(SELF_FRAMES) }; KINDS];
static SELF_FRAMES_DONE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SELF_VERTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The GPU build is in use (switch on, adapter able, self-check not failed). The CPU path is used otherwise.
pub(super) fn gpu_hero_on() -> bool {
	use std::sync::atomic::Ordering::Relaxed;
	mode() != Mode::Off && AVAILABLE.load(Relaxed) && !FAILED.load(Relaxed)
}

pub(super) fn gpu_hero_check() -> bool {
	mode() == Mode::Check
}

/// A new device (start, or re-creation): decides from the adapter whether the build can run, and starts the self-check over.
pub(super) fn device_created(adapter: &wgpu::Adapter, device: &wgpu::Device) {
	use std::sync::atomic::Ordering::Relaxed;
	FAILED.store(false, Relaxed);
	for l in &LEFT {
		l.store(SELF_FRAMES, Relaxed);
	}
	SELF_FRAMES_DONE.store(0, Relaxed);
	SELF_VERTS.store(0, Relaxed);
	if mode() == Mode::Off {
		AVAILABLE.store(false, Relaxed);
		return;
	}
	let lim = device.limits();
	let why = if !adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS) {
		Some("no compute shaders")
	} else if lim.max_storage_buffers_per_shader_stage < 3 {
		Some("fewer than 3 storage buffers per stage")
	} else if lim.max_compute_invocations_per_workgroup < 64 || lim.max_compute_workgroup_size_x < 64 {
		Some("compute workgroup of 64 not supported")
	} else {
		None
	};
	AVAILABLE.store(why.is_none(), Relaxed);
	if let Some(w) = why {
		eprintln!("hero_gpu: not available on this adapter ({w}) -> CPU path");
	}
}

/// The kind of a hero frame for the self-check: bit 0 = blends two poses, bit 1 = tilted (euler rows), bit 2 = afterimage / bomb /
/// nozzle shift / Clank light. Each kind is compared for `SELF_FRAMES` frames before the GPU build is trusted for it.
pub(super) fn kind(blend: bool, tilt: bool, extras: bool) -> usize {
	blend as usize | (tilt as usize) << 1 | (extras as usize) << 2
}

/// Whether a frame of this kind is still being verified (Auto mode, build not failed).
pub(super) fn self_check_wanted(kind: usize) -> bool {
	use std::sync::atomic::Ordering::Relaxed;
	mode() == Mode::Auto && !FAILED.load(Relaxed) && LEFT[kind].load(Relaxed) > 0
}

fn kind_name(kind: usize) -> String {
	let n: Vec<&str> = (0..3).filter(|b| kind >> b & 1 == 1).map(|b| KIND_NAMES[b]).collect();
	if n.is_empty() { "plain".into() } else { n.join("+") }
}

/// Self-check result of one frame (`gpu` = GPU vertices, `cpu` = the CPU build of the same frame): a difference switches to the CPU
/// path for good; N frames of a kind without one end that kind's check (stderr line).
pub(super) fn self_check_result(kind: usize, gpu: &[u8], cpu: &[u8]) {
	use std::sync::atomic::Ordering::Relaxed;
	let (a, b): (&[f32], &[f32]) = (bytemuck::cast_slice(gpu), bytemuck::cast_slice(cpu));
	let total = (a.len() / 16) as u64;
	let mut bad = 0u64;
	let mut max = 0.0f32;
	// Test only: RAC_HERO_GPU_FAULT=1 pretends the 20th self-check frame differed, to exercise the fall-back to the CPU path.
	static FAULT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	if *FAULT.get_or_init(|| std::env::var("RAC_HERO_GPU_FAULT").is_ok_and(|v| v == "1")) && SELF_FRAMES_DONE.load(Relaxed) == 20 {
		bad = 1;
		max = 1.0;
	} else if a.len() != b.len() {
		bad = total.max(1);
		max = f32::INFINITY;
	} else {
		for (x, y) in a.chunks_exact(16).zip(b.chunks_exact(16)) {
			if x.iter().zip(y).any(|(p, q)| p.to_bits() != q.to_bits()) {
				bad += 1;
				for (p, q) in x.iter().zip(y) {
					let d = (p - q).abs();
					if d.is_nan() || d > max {
						max = if d.is_nan() { f32::INFINITY } else { d };
					}
				}
			}
		}
	}
	if bad > 0 {
		if !FAILED.swap(true, Relaxed) {
			eprintln!("hero_gpu: self-check FAILED ({} frame {}, {} of {} vertices differ, max abs diff {:e}) -> CPU path", kind_name(kind), SELF_FRAMES_DONE.load(Relaxed) + 1, bad, total, max);
		}
		return;
	}
	SELF_FRAMES_DONE.fetch_add(1, Relaxed);
	SELF_VERTS.fetch_add(total, Relaxed);
	if LEFT[kind].fetch_update(Relaxed, Relaxed, |v| Some(v.saturating_sub(1))).unwrap_or(0) == 1 {
		eprintln!("hero_gpu: self-check passed ({} frames, {} vertices) [{}]", SELF_FRAMES_DONE.load(Relaxed), SELF_VERTS.load(Relaxed), kind_name(kind));
	}
}

/// std140-shaped parameter block (all vec4 rows); mirrors `P` in `SHADER`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub(super) struct HeroParams {
	pub feet: [f32; 4],
	/// cos yaw, sin yaw, blend factor, dynamic-light id
	pub cs: [f32; 4],
	pub r0: [f32; 4],
	pub r1: [f32; 4],
	pub r2: [f32; 4],
	pub light: [f32; 4],
	pub tint: [f32; 4],
	pub clank: [f32; 4],
	/// bomb colour scale rgb, V shift
	pub bomb: [f32; 4],
	/// ghost alpha encodings (x)
	pub gh: [[f32; 4]; 5],
	/// ghost range a, b, present
	pub gr: [[u32; 4]; 5],
	/// vertex count, have-before, tilt basis, tint present
	pub u0: [u32; 4],
	/// tint start, hero length, clank a, clank b
	pub u1: [u32; 4],
	/// clank present, bomb present, shift present
	pub u2: [u32; 4],
	/// bomb a, bomb b, shift a, shift b
	pub u3: [u32; 4],
}

const SHADER: &str = r#"
struct P {
	feet: vec4<f32>, cs: vec4<f32>, r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, light: vec4<f32>, tint: vec4<f32>,
	clank: vec4<f32>, bomb: vec4<f32>, gh: array<vec4<f32>, 5>, gr: array<vec4<u32>, 5>,
	u0: vec4<u32>, u1: vec4<u32>, u2: vec4<u32>, u3: vec4<u32>,
};
@group(0) @binding(0) var<storage, read> model: array<f32>;
@group(0) @binding(1) var<storage, read> poses: array<f32>;
@group(0) @binding(2) var<uniform> prm: P;
@group(0) @binding(3) var<storage, read_write> outv: array<f32>;

// Every product goes through m(): its bits are OR-ed with u2.w, which is 0 at run time but unknown to the driver. The
// product is rounded on its own, so no a*b+c is contracted into an FMA (the CPU code rounds every product and every sum).
fn m(a: f32, b: f32) -> f32 { return bitcast<f32>(bitcast<u32>(a * b) | prm.u2.w); }

// IEEE-exact divide and square root (Vulkan only promises a few ulp for them, and the GPU's division is an approximation too): the
// driver's estimate, one correction, then the exact fma residual picks the nearest of the three neighbouring floats (ties to even).
fn step(q: f32, up: bool) -> f32 {
	let b = bitcast<u32>(q);
	let away = (b >> 31u) == 0u;
	if (away == up) { return bitcast<f32>(b + 1u); }
	return bitcast<f32>(b - 1u);
}
fn pick(c0: f32, e0: f32, c1: f32, e1: f32) -> f32 {
	if (e1 < e0 || (e1 == e0 && (bitcast<u32>(c1) & 1u) == 0u)) { return c1; }
	return c0;
}
fn dv(a: f32, b: f32) -> f32 {
	let q0 = a / b;
	let q = q0 + fma(-q0, b, a) / b;
	if (q == 0.0) { return q; }
	var best = q;
	var be = abs(fma(-q, b, a));
	let u = step(q, true);
	let d = step(q, false);
	let eu = abs(fma(-u, b, a));
	let ed = abs(fma(-d, b, a));
	let bu = pick(best, be, u, eu);
	let eb = select(be, eu, bu == u && bu != best);
	best = bu;
	be = eb;
	return pick(best, be, d, ed);
}
fn sq(a: f32) -> f32 {
	let s0 = sqrt(a);
	if (s0 == 0.0) { return s0; }
	let s = s0 + fma(-s0, s0, a) / (2.0 * s0);
	var best = s;
	var be = abs(fma(-s, s, a));
	let u = step(s, true);
	let d = step(s, false);
	let eu = abs(fma(-u, u, a));
	let ed = abs(fma(-d, d, a));
	let bu = pick(best, be, u, eu);
	let eb = select(be, eu, bu == u && bu != best);
	best = bu;
	be = eb;
	return pick(best, be, d, ed);
}

fn turn(v: vec3<f32>) -> vec3<f32> {
	if (prm.u0.z != 0u) {
		return vec3<f32>(
			m(v.x, prm.r0.x) + m(v.y, prm.r1.x) + m(v.z, prm.r2.x),
			m(v.x, prm.r0.y) + m(v.y, prm.r1.y) + m(v.z, prm.r2.y),
			m(v.x, prm.r0.z) + m(v.y, prm.r1.z) + m(v.z, prm.r2.z));
	}
	return vec3<f32>(m(v.x, prm.cs.x) - m(v.y, prm.cs.y), m(v.x, prm.cs.y) + m(v.y, prm.cs.x), v.z);
}

fn rd3(b: u32) -> vec3<f32> { return vec3<f32>(poses[b], poses[b + 1u], poses[b + 2u]); }

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) g: vec3<u32>) {
	let k = g.x;
	let n = prm.u0.x;
	if (k >= n) { return; }
	let fl = prm.r0.w != 0.0;
	let cb = (select(n, 0u, fl) + k) * 6u;
	var pos = rd3(cb);
	var nrm = rd3(cb + 3u);
	if (prm.u0.y != 0u) {
		let ab = (select(0u, n, fl) + k) * 6u;
		let pa = rd3(ab);
		let na = rd3(ab + 3u);
		let d = pos - pa;
		let hidden = all(pa == vec3<f32>(0.0)) || all(pos == vec3<f32>(0.0));
		let keep = hidden || (m(d.x, d.x) + m(d.y, d.y) + m(d.z, d.z)) > 16.0;
		if (!keep) {
			let t = prm.cs.z;
			let nb = nrm;
			let dp = pos - pa;
			pos = vec3<f32>(pa.x + m(dp.x, t), pa.y + m(dp.y, t), pa.z + m(dp.z, t));
			let dn = nb - na;
			let nl = vec3<f32>(na.x + m(dn.x, t), na.y + m(dn.y, t), na.z + m(dn.z, t));
			let l = sq(m(nl.x, nl.x) + m(nl.y, nl.y) + m(nl.z, nl.z));
			if (l > 1e-6) { nrm = vec3<f32>(dv(nl.x, l), dv(nl.y, l), dv(nl.z, l)); } else { nrm = nb; }
		}
	}
	let mb = k * 16u;
	var col = vec4<f32>(model[mb + 8u], model[mb + 9u], model[mb + 10u], model[mb + 11u]);
	let hero_tint = prm.u0.w != 0u && k >= prm.u1.x && k < prm.u1.y;
	var done = false;
	if (hero_tint) {
		col = vec4<f32>(prm.tint.xyz, -2.0);
		done = true;
	}
	if (!done && prm.u2.x != 0u && k >= prm.u1.z && k < prm.u1.w) {
		col = vec4<f32>(prm.clank.xyz, -2.0);
		done = true;
	}
	if (!done) {
		var w = prm.cs.w;
		for (var gi = 0u; gi < 5u; gi = gi + 1u) {
			let r = prm.gr[gi];
			if (r.z != 0u && k >= r.x && k < r.y) {
				w = prm.gh[gi].x;
				break;
			}
		}
		col.w = w;
	}
	if (prm.u2.y != 0u && k >= prm.u3.x && k < prm.u3.y) {
		col = vec4<f32>(m(col.x, prm.bomb.x), m(col.y, prm.bomb.y), m(col.z, prm.bomb.z), col.w);
	}
	var uv = vec2<f32>(model[mb + 3u], model[mb + 4u]);
	if (prm.u2.z != 0u && k >= prm.u3.z && k < prm.u3.w) { uv.y = uv.y + prm.bomb.w; }
	let p = turn(pos);
	let nn = turn(nrm);
	outv[mb] = p.x + prm.feet.x;
	outv[mb + 1u] = p.y + prm.feet.y;
	outv[mb + 2u] = p.z + prm.feet.z;
	outv[mb + 3u] = uv.x;
	outv[mb + 4u] = uv.y;
	outv[mb + 5u] = nn.x;
	outv[mb + 6u] = nn.y;
	outv[mb + 7u] = nn.z;
	outv[mb + 8u] = col.x;
	outv[mb + 9u] = col.y;
	outv[mb + 10u] = col.z;
	outv[mb + 11u] = col.w;
	outv[mb + 12u] = prm.light.x;
	outv[mb + 13u] = prm.light.y;
	outv[mb + 14u] = prm.light.z;
	outv[mb + 15u] = prm.light.w;
}
"#;

pub(super) struct HeroGpu {
	pipeline: wgpu::ComputePipeline,
	bind: wgpu::BindGroup,
	params: wgpu::Buffer,
	poses: wgpu::Buffer,
	n: usize,
	/// What `poses` holds: before (first n entries) and now (next n), as the CPU copies they were made from.
	kept_before: Posed,
	kept_now: Posed,
	/// Generation numbers of those poses (0 = unknown): equal non-zero generations mean equal bytes, so no byte compare.
	gen_before: u64,
	gen_now: u64,
	/// Bounding radius about the feet of every pose position (both poses), for the clip guard-band pre-test.
	pub radius: f32,
	/// The parameter block for the next `record`.
	last: Option<HeroParams>,
	/// A build is wanted for the next frame encoder (`record`).
	armed: std::sync::atomic::AtomicBool,
	have_poses: bool,
	radius_blend: bool,
	/// [PC] (lane A6) The halves of `poses` swap roles (`flip`): a new tick's "before" is the "now" half already in the buffer
	/// (the shader reads the halves by `parity`), so only the new "now" is written. The bounding radius of each kept pose.
	flip: bool,
	rad_now: f32,
	rad_before: f32,
}

type PoseEntry6 = [f32; 6];

fn entries(p: &Posed) -> Vec<PoseEntry6> {
	p.0.iter().zip(&p.1).map(|(a, b)| [a[0], a[1], a[2], b[0], b[1], b[2]]).collect()
}

impl HeroGpu {
	/// `vbuf` is the marker batch's vertex buffer, which must have STORAGE usage.
	pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue, model: &[Vertex], vbuf: &wgpu::Buffer) -> Option<HeroGpu> {
		let n = model.len();
		let lim = device.limits();
		if (2 * n * 24) as u64 > lim.max_storage_buffer_binding_size as u64 || (n * std::mem::size_of::<Vertex>()) as u64 > lim.max_storage_buffer_binding_size as u64 || n.div_ceil(64) as u32 > lim.max_compute_workgroups_per_dimension {
			eprintln!("hero_gpu: model too big for this adapter's storage buffer limits -> CPU path");
			return None;
		}
		// Any validation error while building the pipeline (a driver that rejects the shader) means the CPU path, not a panic.
		let scope_v = device.push_error_scope(wgpu::ErrorFilter::Validation);
		let scope_i = device.push_error_scope(wgpu::ErrorFilter::Internal);
		let make = |label, size: usize, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (size as u64).max(16), usage, mapped_at_creation: false });
		let model_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("hero model"), size: ((n * std::mem::size_of::<Vertex>()) as u64).max(16), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		queue.write_buffer(&model_buf, 0, bytemuck::cast_slice(model));
		let poses = make("hero poses", 2 * n * 24, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
		let params = make("hero params", std::mem::size_of::<HeroParams>(), wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
		let st = |binding, read_only| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::COMPUTE,
			ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
			count: None,
		};
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("hero build"),
			entries: &[
				st(0, true),
				st(1, true),
				wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
				st(3, false),
			],
		});
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("hero build"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("hero build", SHADER.into())) });
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("hero build"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("hero build"), layout: Some(&pl), module: &module, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
		let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("hero build"),
			layout: &layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: model_buf.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 1, resource: poses.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 3, resource: vbuf.as_entire_binding() },
			],
		});
		let err = pollster::block_on(scope_i.pop()).or_else(|| pollster::block_on(scope_v.pop()));
		if let Some(e) = err {
			eprintln!("hero_gpu: shader / pipeline rejected by the driver ({e}) -> CPU path");
			return None;
		}
		Some(HeroGpu { pipeline, bind, params, poses, n, kept_before: Default::default(), kept_now: Default::default(), gen_before: 0, gen_now: 0, radius: 0.0, last: None, armed: false.into(), have_poses: false, radius_blend: false, flip: false, rad_now: 0.0, rad_before: 0.0 })
	}

	pub(super) fn n(&self) -> usize {
		self.n
	}

	/// Uploads the pose(s) that differ from what the buffer holds. `before` None = no blend source (the shader gets have-before 0).
	/// A pose comes with its generation number (0 = unknown): the producer (handler.rs hero cache) gives a new number to every pose it
	/// computes and the same number to every unmodified copy of it, so equal non-zero numbers prove equal bytes and the 1.25 MB
	/// compare is skipped. Check mode still compares and counts the cases where the number said "equal" but the bytes were not.
	/// Returns whether the shader may blend.
	pub(super) fn set_poses(&mut self, queue: &wgpu::Queue, before: Option<(&Posed, u64)>, now: (&Posed, u64)) -> bool {
		use std::sync::atomic::Ordering::Relaxed;
		let n = self.n;
		let same = |a: &Posed, b: &Posed| bytemuck::cast_slice::<_, u8>(&a.0) == bytemuck::cast_slice::<_, u8>(&b.0) && bytemuck::cast_slice::<_, u8>(&a.1) == bytemuck::cast_slice::<_, u8>(&b.1);
		let check = gpu_hero_check();
		// true = the buffer already holds this pose.
		let held = |kept: &Posed, kept_gen: u64, p: &Posed, g: u64, have: bool| -> bool {
			if !have {
				return false;
			}
			if g != 0 && g == kept_gen {
				if check {
					let eq = same(kept, p);
					GEN_CALLS.fetch_add(1, Relaxed);
					if !eq {
						GEN_MISMATCH.fetch_add(1, Relaxed);
					}
					return eq;
				}
				return true;
			}
			same(kept, p)
		};
		let mut changed = false;
		let blend = before.filter(|b| b.0.0.len() == n && b.0.1.len() == n);
		let far = |p: &Posed| p.0.iter().map(|e| (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt()).fold(0.0f32, f32::max);
		// A new tick: the "before" pose is the "now" the buffer held (same generation), so its entries and radius are kept
		// ones: copied to the first half, not rebuilt (the bytes written are the same as the rebuild's).
		let shift = !check && self.have_poses && self.gen_now != 0 && now.1 != self.gen_now && blend.is_some_and(|(_, g)| g == self.gen_now);
		if shift {
			self.flip = !self.flip;
			self.kept_before = std::mem::take(&mut self.kept_now);
			self.gen_before = self.gen_now;
			self.rad_before = self.rad_now;
			changed = true;
		}
		// Byte offsets of the two halves.
		let (now_off, before_off) = if self.flip { (0, (n * 24) as u64) } else { ((n * 24) as u64, 0) };
		if shift || !held(&self.kept_now, self.gen_now, now.0, now.1, self.have_poses) {
			queue.write_buffer(&self.poses, now_off, bytemuck::cast_slice(&entries(now.0)));
			self.kept_now = now.0.clone();
			self.rad_now = far(&self.kept_now);
			changed = true;
		}
		self.gen_now = now.1;
		if let (Some((b, g)), false) = (blend, shift) {
			if !held(&self.kept_before, self.gen_before, b, g, self.have_poses) {
				queue.write_buffer(&self.poses, before_off, bytemuck::cast_slice(&entries(b)));
				self.kept_before = b.clone();
				self.rad_before = far(&self.kept_before);
				changed = true;
			}
			self.gen_before = g;
		}
		self.have_poses = true;
		if changed || self.radius_blend != blend.is_some() {
			self.radius_blend = blend.is_some();
			self.radius = self.rad_now.max(if blend.is_some() { self.rad_before } else { 0.0 });
		}
		blend.is_some()
	}

	/// 1.0 when the "before" pose is in the second half of the pose buffer (the shader's `fl`), else 0.0.
	pub(super) fn parity(&self) -> f32 {
		self.flip as u32 as f32
	}

	pub(super) fn queue_build(&mut self, queue: &wgpu::Queue, p: HeroParams) {
		if self.last != Some(p) {
			queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
		}
		self.last = Some(p);
		self.armed.store(true, std::sync::atomic::Ordering::Relaxed);
	}

	/// The CPU path wrote the vertex buffer this frame: no build.
	pub(super) fn cancel(&mut self) {
		self.armed.store(false, std::sync::atomic::Ordering::Relaxed);
	}

	/// Check mode: runs the pending build now and returns the vertex buffer's bytes.
	pub(super) fn build_and_read(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, vbuf: &wgpu::Buffer) -> Vec<u8> {
		let size = (self.n * std::mem::size_of::<Vertex>()) as u64;
		let stage = device.create_buffer(&wgpu::BufferDescriptor { label: Some("hero check"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let mut enc = device.create_command_encoder(&Default::default());
		self.record(&mut enc);
		enc.copy_buffer_to_buffer(vbuf, 0, &stage, 0, size);
		queue.submit(Some(enc.finish()));
		stage.slice(..).map_async(wgpu::MapMode::Read, |_| {});
		let _w = crate::runlog::waiting("the GPU's hero build for the self-check (gpuhero)");
		device.poll(wgpu::PollType::wait_indefinitely()).ok();
		let out = stage.slice(..).get_mapped_range().expect("mapped").to_vec();
		stage.unmap();
		out
	}

	/// Encodes this frame's build (once; the next `queue_build` re-arms it).
	pub(super) fn record(&self, enc: &mut wgpu::CommandEncoder) {
		if !self.armed.swap(false, std::sync::atomic::Ordering::Relaxed) {
			return;
		}
		let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("hero build"), timestamp_writes: None });
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, &self.bind, &[]);
		pass.dispatch_workgroups(self.n.div_ceil(64) as u32, 1, 1);
	}
}

static GEN_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static GEN_MISMATCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BAD_FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BAD_VERTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static VERTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static FIELD: [std::sync::atomic::AtomicU64; 16] = [const { std::sync::atomic::AtomicU64::new(0) }; 16];
static DBG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static DBG2: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static MAXDIFF: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Check mode: counts the vertices whose float bits differ between the GPU build (`gpu`) and the CPU build (`cpu`), and the largest
/// absolute difference (stderr `hero_gpu_check:` lines).
pub(super) fn note_check(gpu: &[u8], cpu: &[u8]) {
	use std::sync::atomic::Ordering::Relaxed;
	let (a, b): (&[f32], &[f32]) = (bytemuck::cast_slice(gpu), bytemuck::cast_slice(cpu));
	let mut bad = 0u64;
	let mut max = 0.0f32;
	for (x, y) in a.chunks_exact(16).zip(b.chunks_exact(16)) {
		if x.iter().zip(y).any(|(p, q)| p.to_bits() != q.to_bits()) {
			bad += 1;
			let pd = x[..3].iter().zip(&y[..3]).any(|(p, q)| p.to_bits() != q.to_bits());
			let nd = x[5..8].iter().zip(&y[5..8]).any(|(p, q)| p.to_bits() != q.to_bits());
			if (pd && DBG.fetch_add(1, Relaxed) < 10) || (nd && !pd && DBG2.fetch_add(1, Relaxed) < 10) {
				eprintln!("hero_gpu_check {} diff gpu {:x?} cpu {:x?}", if pd { "pos" } else { "nrm" }, x[..8].iter().map(|v| v.to_bits()).collect::<Vec<_>>(), y[..8].iter().map(|v| v.to_bits()).collect::<Vec<_>>());
			}

			for (i, (p, q)) in x.iter().zip(y).enumerate() {
				if p.to_bits() != q.to_bits() {
					FIELD[i].fetch_add(1, Relaxed);
				}
			}
			for (p, q) in x.iter().zip(y) {
				let d = (p - q).abs();
				if d.is_nan() || d > max {
					max = if d.is_nan() { f32::INFINITY } else { d };
				}
			}
		}
	}
	let n = CALLS.fetch_add(1, Relaxed) + 1;
	VERTS.fetch_add((a.len() / 16) as u64, Relaxed);
	BAD_VERTS.fetch_add(bad, Relaxed);
	if bad > 0 {
		BAD_FRAMES.fetch_add(1, Relaxed);
	}
	let prev = f32::from_bits(MAXDIFF.load(Relaxed));
	if max > prev {
		MAXDIFF.store(max.to_bits(), Relaxed);
	}
	if n % 500 == 0 {
		eprintln!("hero_gpu_check: {} frames, {} differ, {} of {} vertices differ, max abs diff {:e}", n, BAD_FRAMES.load(Relaxed), BAD_VERTS.load(Relaxed), VERTS.load(Relaxed), f32::from_bits(MAXDIFF.load(Relaxed)));
		eprintln!("hero_gpu_check pose generation: {} 'equal' verdicts, {} of them with different bytes", GEN_CALLS.load(Relaxed), GEN_MISMATCH.load(Relaxed));
		eprintln!("hero_gpu_check fields: {:?}", FIELD.iter().map(|c| c.load(Relaxed)).collect::<Vec<_>>());
	}
}
