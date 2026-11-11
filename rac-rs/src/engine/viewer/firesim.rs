//! [PC] Fire, simulated (the Fire option on; dev knob `fire_sim` 0 draws the old volumes for an A/B): every fire the fire code takes
//! over (the level fires' tongues, the effect flames, burning enemies, the explosions and fireballs, the Pyrocitor's stream, the ground
//! fire after explosions) is made of thousands of small particles simulated on the GPU, not one volume. The volumes the frame built
//! (firefx.rs `vols` / `svols`, firefx_vol.rs `Vol`) are the EMITTERS: an upright flame emits from its foot, a slab of the jet along
//! its axis with the stream's flow, a fireball from its middle outwards; so the fire keeps the retail fire's places, sizes, reach and
//! colour (the weapon lights and the grass keep reading the same sources). The retail game is never touched; Classic and Fire Off
//! dispatch and draw nothing.
//! - The step (firesim_step.wgsl, one compute pass per display frame with the frame's game time step: frame smoothing on, 30..240 fps;
//!   a paused game stands): new particles take the ring's next slots (`plan`: no atomics, no allocation), the others rise by their
//!   heat, drag, ride a 3D curl-noise wind, cool, turn into thin smoke (a share of them) or burn out, and stop on the scene's surfaces
//!   (the level pass's depth: a stream hitting the floor splashes along it).
//! - The draw (firesim.wgsl, inside the particle pass): the smoke particles as soft light-grey billboards; the flames through the heat
//!   buffer (firesim_heat.rs: soft heat blobs into a small target, then one composite with a rising world-sized warp and a blackbody
//!   ramp; flames only add light).
//! - Budget: a fixed pool by the Particles quality (Low 4k, Medium 8k, High 16k), the density by the Fire level; far fires get fewer,
//!   bigger particles (`want_of`), over the pool's 80 % all rates drop together and the particles grow (`plan`).
//! Dev knobs (`RAC_TUNE`, ENV.md): fire_sim fire_sim_dens fire_sim_rise fire_sim_turb fire_sim_drag fire_sim_bright fire_sim_smoke
//! fire_sim_glow fire_sim_tint fire_sim_streak fire_sim_smoke_a fire_sim_cover fire_sim_size fire_sim_collide (and fire_jet_flow);
//! `RAC_FIRE_SIM_WGSL=<file>` / `RAC_FIRE_SIM_STEP_WGSL=<file>` load the shaders from disk. GPU time: RAC_GPU_PASSES=1, row "fire sim".

use super::firefx::{tune, FireEnv};
use super::firefx_vol::{Vol, KIND_JET};
use super::*;

pub(super) const STEP_WGSL: &str = include_str!("firesim_step.wgsl");
/// The draw module: the particles (firesim.wgsl) and the flames' heat buffer passes (firesim_heat.wgsl) share one source.
pub(super) const DRAW_WGSL: &str = concat!(include_str!("firesim.wgsl"), include_str!("firesim_heat.wgsl"));

/// The pool at Particles High (the buffer is made this big once; Low and Medium use its first part).
pub(super) const POOL_MAX: u32 = 16384;
/// One particle in the pool (firesim_step.wgsl `P`: five vec4).
const PART_BYTES: u64 = 80;
/// Emitters a frame (the volumes are capped at firefx_vol::MAX_VOLS before this).
pub(super) const MAX_EMIT: usize = 512;
// Every volume gets a row before any flame splits (firesim_tongues::split_all).
const _: () = assert!(super::firefx_vol::MAX_VOLS <= MAX_EMIT);
/// Particles per unit of `want_of`'s counts (dev knob fire_sim_dens): many small, crisp flames (each `want_of` size gain
/// 1/sqrt(density) smaller) instead of a few big soft ones that blur into one smear (user 2026-11-03, "real fire simulation").
const DENS: f32 = 8.0;
/// A smoke particle lives SMOKE_X x its flame part (the shaders' `SMOKE_X`).
pub(super) const SMOKE_X: f32 = 2.4;
/// The share of each kind's particles that turn into smoke (flame, jet, burst; firesim_step.wgsl `smoke_share`).
const SHARE: [f32; 3] = [0.28, 0.12, 0.55];
/// Up to this far (m) a fire gets its full count; farther, fewer and bigger particles (to 0.15 of the count).
const LOD_D: f32 = 10.0;
/// The longest step (s): a hitch does not throw the particles.
const MAX_DT: f32 = 0.05;
/// After this long with nothing spawned (s) every particle is dead: no dispatch, no draw.
const QUIET: f64 = 3.0;
/// Voxels per side of the wind / outline noise.
const NOISE: u32 = 32;

/// The size rule for the level's fire tongues: their flames reach at most this many times the retail quad's half bottom edge across.
pub(super) const LEVEL_X: f32 = 1.2;
/// The share of a level tongue's licks that were embers (firesim_step.wgsl EMBER_P): a flipbook tongue's ember emitter spawns that many.
pub(super) const EMBER_P: f32 = 0.025;

pub(super) const EMIT_FLAME: u32 = 0;
pub(super) const EMIT_JET: u32 = 1;
pub(super) const EMIT_BURST: u32 = 2;

/// One emitter, as the step shader reads it (firesim_step.wgsl `E`, the layout written there).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct SimEmit {
	pub a: [f32; 4],
	pub b: [f32; 4],
	pub c: [f32; 4],
	pub d: [f32; 4],
	pub e: [f32; 4],
	pub n: [u32; 4],
	/// The same emitter's a.xyz last display frame (w 1: matched, `link_prev`) and its b.xyz (a jet slab's far end): the step spawns
	/// each particle where the emitter stood at its moment of birth inside the frame.
	pub pa: [f32; 4],
	pub pb: [f32; 4],
}

/// The shaders' uniform block (`SimU`): camera (eye with the step, forward with tan of half the fov, right with the aspect, up with the
/// clock), the ring (head, spawn count, pool, frame), (near, far, emitters, collide), the step's knobs, the draw's knobs.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct SimU {
	pub eye: [f32; 4],
	pub fwd: [f32; 4],
	pub right: [f32; 4],
	pub up: [f32; 4],
	pub ring: [u32; 4],
	pub k: [f32; 4],
	pub m: [f32; 4],
	pub look: [f32; 4],
	pub look2: [f32; 4],
}

/// An emitter as the frame wants it: its row, how many particles a second it spawns and how many it keeps alive (smoke included).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Want {
	pub e: SimEmit,
	pub rate: f32,
	pub live: f32,
	/// What the pool pressure (`plan`) counts for this emitter: `live`, except a flipbook tongue's embers count as the licks it replaced
	/// (so weapon fire is spawned exactly as before the flipbooks).
	pub weigh: f32,
}

/// Whether the fire is simulated (else the volumes draw, firefx_vol.rs).
pub(super) fn on() -> bool {
	tune("fire_sim", 1.0) > 0.5
}

/// The pool by the Particles quality (`Graphics::particles_q`): Low 4k, Medium 8k, High 16k particles.
pub(super) fn pool_of(particles_q: u8) -> u32 {
	match particles_q {
		0 | 1 => 4096,
		2 => 8192,
		_ => POOL_MAX,
	}
}

/// The particle density by the Fire level (Low, Medium, High), times DENS.
pub(super) fn density(level: u8) -> f32 {
	(match level {
		0 | 1 => 0.45,
		2 => 0.7,
		_ => 1.0,
	}) * tune("fire_sim_dens", DENS)
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
	super::firefx::dist(a, b)
}

/// The emitter of volume `v` (None: nothing visible): its row, rate and live count at density `q`, fewer and bigger particles far from
/// `eye`. Counts: a flame ~110 x (h r)^0.8 particles, a jet slab ~55 per metre (more near the nozzle, where nothing flows in from
/// behind), a fireball ~140 r^1.5 while new.
pub(super) fn want_of(v: &Vol, eye: [f32; 3], q: f32) -> Option<Want> {
	let tint = [v.c[0], v.c[1], v.c[2]];
	let (e, live, life, centre, k) = if v.b[3] > 1.5 {
		let (p0, p1) = ([v.a[0], v.a[1], v.a[2]], [v.d[0], v.d[1], v.d[2]]);
		let (al0, al1, s0) = (v.c[3], v.b[2], v.e[3]);
		let alpha = al0.max(al1);
		if alpha <= 0.01 {
			return None;
		}
		let rm = 0.5 * (v.a[3] + v.d[3]);
		let live = 55.0 * (rm / 0.35).clamp(0.5, 2.5) * dist(p0, p1) * (1.0 + 1.5 * (-s0.max(0.0) / 1.2).exp()) * alpha.sqrt();
		let e = SimEmit { a: v.a, b: [p1[0], p1[1], p1[2], v.d[3]], c: [tint[0], tint[1], tint[2], al0], d: [s0, v.f[3], v.b[0], al1], e: [1.0, (v.b[3] - KIND_JET).max(0.0), 1.0, 0.0], n: [0, EMIT_JET, 0, 0], ..Default::default() };
		(e, live, 0.26, std::array::from_fn(|i| 0.5 * (p0[i] + p1[i])), EMIT_JET)
	} else if v.b[3] > 0.5 {
		let (r, age) = (v.b[0].max(0.05), v.b[2].clamp(0.0, 1.0));
		if age <= 0.01 {
			return None;
		}
		let live = (140.0 * r.powf(1.5)).clamp(10.0, 600.0) * (0.3 + 0.7 * age);
		let e = SimEmit { a: [v.a[0], v.a[1], v.a[2], r], b: [age, v.d[0].max(0.0), 0.0, 0.0], c: [tint[0], tint[1], tint[2], 1.0], e: [1.0, 0.0, 1.0, 0.0], n: [0, EMIT_BURST, 0, 0], ..Default::default() };
		(e, live, 0.3 + 0.12 * r, [v.a[0], v.a[1], v.a[2]], EMIT_BURST)
	} else {
		let (h, r, fade) = (v.a[3].max(0.05), v.b[0].max(0.02), v.b[2]);
		if fade <= 0.01 {
			return None;
		}
		let live = (110.0 * (h * r).powf(0.8)).clamp(10.0, 480.0) * fade.sqrt();
		let mut e = SimEmit { a: [v.a[0], v.a[1], v.a[2], h], b: [r, 0.0, 0.0, 0.0], c: [tint[0], tint[1], tint[2], fade], e: [1.0, 0.0, 1.0, 0.0], n: [0, EMIT_FLAME, 0, 0], ..Default::default() };
		if v.d[2] > 1.5 {
			// A level tongue drawn as flipbooks (firebook.rs, firefx_vol.rs `ember_tongue`): n.z 2, only its embers (EMBER_P of the
			// licks it had, firesim_step.wgsl `ember_of`), b.z its height.
			(e.n[2], e.b[2]) = (2, h);
			let live = live * EMBER_P;
			let life = 1.6;
			let lod = (LOD_D / dist([v.a[0], v.a[1], v.a[2] + 0.45 * h], eye).max(1e-3)).clamp(0.15, 1.0);
			let n = live * q * lod;
			e.e[0] = (1.0 / (q * lod).max(0.05)).sqrt().min(3.0);
			let old = live / EMBER_P * q * lod * (1.0 + SHARE[EMIT_FLAME as usize] * tune("fire_sim_smoke", 1.0) * (SMOKE_X - 1.0));
			return Some(Want { e, rate: n / life, live: n, weigh: old });
		} else if v.d[2] > 0.5 {
			// One of the level's fire tongues (firefx_vol.rs `level_tongue`): n.z 1, b.y its reach across (the retail quad's half bottom
			// edge x 1.2; the tongue's r is that half edge x fire_w), b.z its top over the foot (the quad's side).
			(e.n[2], e.b[1], e.b[2]) = (1, LEVEL_X * r / tune("fire_w", 1.25).max(0.1), h);
		}
		(e, live, 0.95 * (0.35 + 0.2 * h), [v.a[0], v.a[1], v.a[2] + 0.45 * h], EMIT_FLAME)
	};
	let lod = (LOD_D / dist(centre, eye).max(1e-3)).clamp(0.15, 1.0);
	let n = live * q * lod;
	let mut e = e;
	e.e[0] = (1.0 / (q * lod).max(0.05)).sqrt().min(3.0);
	let smoke = SHARE[k as usize] * tune("fire_sim_smoke", 1.0);
	let live = n * (1.0 + smoke * (SMOKE_X - 1.0));
	Some(Want { e, rate: n / life, live, weigh: live })
}

/// A number in 0..1 from (emitter, frame): the rounding of a fraction of a particle per frame.
fn frac(i: u32, frame: u32) -> f32 {
	let mut x = i.wrapping_mul(0x9E37_79B9) ^ frame.wrapping_mul(0x85EB_CA6B);
	x ^= x >> 16;
	x = x.wrapping_mul(0x7FEB_352D);
	x ^= x >> 15;
	(x >> 8) as f32 / 16_777_216.0
}

/// The frame's emitter rows (`rows`: each with its running end in n.x, the emitters that spawn nothing left out) and the particles to
/// spawn in all, from `wants`, step `dt`, `pool`. Over 80 % of the pool alive the rates drop together and the particles grow to keep
/// the cover (the ring would otherwise take living particles); one frame never spawns more than half the pool.
pub(super) fn plan(wants: &[Want], dt: f32, pool: u32, frame: u32, rows: &mut Vec<SimEmit>) -> u32 {
	rows.clear();
	let room = 0.8 * pool as f32;
	let total: f32 = wants.iter().map(|w| w.weigh).sum();
	let s = if total > room { room / total } else { 1.0 };
	let grow = (1.0 / s).sqrt().min(2.5);
	let cap = pool / 2;
	let mut end = 0u32;
	for (i, w) in wants.iter().take(MAX_EMIT).enumerate() {
		let x = w.rate * s * dt + frac(i as u32, frame);
		let n = (x.max(0.0).floor() as u32).min(cap - end);
		if n == 0 {
			continue;
		}
		end += n;
		let mut e = w.e;
		e.e[0] = (e.e[0] * grow).min(4.0);
		e.n[0] = end;
		rows.push(e);
	}
	end
}

/// The largest turn (cos of the angle) of a jet slab's axis between two display frames that the sub-frame emission still sweeps.
const TURN_MAX_COS: f32 = 0.866;

/// Whether jet slabs `r` (now) and `p` (last frame) point within 30 degrees of each other (a slab too short to tell: yes).
pub(super) fn same_way(r: &SimEmit, p: &SimEmit) -> bool {
	let a: [f32; 3] = std::array::from_fn(|k| r.b[k] - r.a[k]);
	let b: [f32; 3] = std::array::from_fn(|k| p.b[k] - p.a[k]);
	let (la, lb) = (a.iter().map(|x| x * x).sum::<f32>().sqrt(), b.iter().map(|x| x * x).sum::<f32>().sqrt());
	if la < 1e-3 || lb < 1e-3 {
		return true;
	}
	(0..3).map(|k| a[k] * b[k]).sum::<f32>() >= TURN_MAX_COS * la * lb
}

/// Each row's place last display frame (`pa`, `pb`): the emitter of the same kind in `prev` nearest to it within reach (a jet slab: the
/// one nearest in place and along the stream, so the nozzle's slab follows the nozzle); none: spawned at this frame's place.
pub(super) fn link_prev(rows: &mut [SimEmit], prev: &[SimEmit]) {
	for r in rows.iter_mut() {
		let (kind, a) = (r.n[1], [r.a[0], r.a[1], r.a[2]]);
		let reach = if kind == EMIT_JET { 1.5 } else { 0.6 };
		let mut best: Option<(f32, &SimEmit)> = None;
		for p in prev.iter().filter(|p| p.n[1] == kind) {
			let d = dist(a, [p.a[0], p.a[1], p.a[2]]);
			// A slab whose stream turned more than TURN_MAX since last frame (an about-turn while firing: the keyboard's A/D or S turn
			// the hero) starts afresh: spawning along the straight line between the two would draw a flat sheet of flame across the turn.
			if d > reach || (kind == EMIT_JET && !same_way(r, p)) {
				continue;
			}
			let score = d + if kind == EMIT_JET { (r.d[0] - p.d[0]).abs() } else { 0.0 };
			if best.is_none_or(|b| score < b.0) {
				best = Some((score, p));
			}
		}
		(r.pa, r.pb) = match best {
			Some((_, p)) => ([p.a[0], p.a[1], p.a[2], 1.0], [p.b[0], p.b[1], p.b[2], 0.0]),
			None => ([0.0; 4], [0.0; 4]),
		};
	}
}

/// The step (s) from the last display frame's draw clock to `shown`: 0 on a repeated frame (the split view's second pass, a pause),
/// at most MAX_DT; None when the clock went back or jumped (a world swap, the fire off for a while): the pool starts over.
pub(super) fn step_of(last: Option<f64>, shown: f64) -> Option<f32> {
	let d = shown - last?;
	(0.0..=0.5).contains(&d).then(|| (d as f32).min(MAX_DT))
}

/// What the step needs from the frame besides the fire's own lists.
pub(super) struct StepIn<'a> {
	/// The level pass's depth (multisampled) and its key (width, height, the post targets' generation): the step's bind group is
	/// remade when it changes.
	pub depth: &'a wgpu::TextureView,
	pub key: (u32, u32, u64),
	/// The near and far plane the depth was drawn with, the target's aspect.
	pub near_far: [f32; 2],
	pub aspect: f32,
	/// Graphics::particles_q and the Fire level.
	pub particles_q: u8,
	pub level: u8,
}

/// The pool, the emitters and the pipelines; the ring and clock state.
#[derive(Default)]
pub(super) struct FireSim {
	res: Option<SimRes>,
	/// The frame's emitters before the split (`firesim_tongues::split_all`) and after it.
	src: Vec<Want>,
	wants: Vec<Want>,
	rows: Vec<SimEmit>,
	/// Last display frame's emitters (`link_prev`).
	prev: Vec<SimEmit>,
	head: u32,
	frame: u32,
	clock: Option<f64>,
	last_spawn: f64,
	reset: bool,
	pool: u32,
	/// This frame stepped (or holds live particles): `draw` draws.
	ran: bool,
}

struct SimRes {
	step: wgpu::ComputePipeline,
	draw: wgpu::RenderPipeline,
	ubuf: wgpu::Buffer,
	parts: wgpu::Buffer,
	emits: wgpu::Buffer,
	draw_bind: wgpu::BindGroup,
	step_layout: wgpu::BindGroupLayout,
	noise: wgpu::TextureView,
	samp: wgpu::Sampler,
	step_bind: Option<((u32, u32, u64), wgpu::BindGroup)>,
	/// The flames' heat buffer and its composite (firesim_heat.rs).
	heat: super::firesim_heat::HeatRes,
}

/// The wind (RGB: three smooth potentials, the curl's) and the outline noise (A: finer fbm), tiling n^3 RGBA8.
fn noise_bytes(n: u32) -> Vec<u8> {
	use super::volfog::noise::fbm;
	let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
	let mut out = Vec::with_capacity((n * n * n * 4) as usize);
	for z in 0..n {
		for y in 0..n {
			for x in 0..n {
				let p = [x as f32 / n as f32, y as f32 / n as f32, z as f32 / n as f32];
				out.extend([to_u8(fbm(p, 2, 2, 41)), to_u8(fbm(p, 2, 2, 43)), to_u8(fbm(p, 2, 2, 47)), to_u8(fbm(p, 4, 3, 53))]);
			}
		}
	}
	out
}

fn shader(device: &wgpu::Device, label: &str, env: &str, src: &'static str) -> wgpu::ShaderModule {
	let src = match std::env::var_os(env).and_then(|p| std::fs::read_to_string(p).ok()) {
		Some(s) => std::borrow::Cow::Owned(s),
		None => std::borrow::Cow::Borrowed(src),
	};
	device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl(label, src)) })
}

fn layout_entry(binding: u32, visibility: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
	wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None }
}

fn storage(read_only: bool) -> wgpu::BindingType {
	wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None }
}

const UNIFORM: wgpu::BindingType = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
const NOISE_TEX: wgpu::BindingType = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false };
const SAMPLER: wgpu::BindingType = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);

impl SimRes {
	fn new(env: &FireEnv<'_>) -> SimRes {
		let device = env.device;
		let (step_mod, draw_mod) = (shader(device, "fire_sim_step", "RAC_FIRE_SIM_STEP_WGSL", STEP_WGSL), shader(device, "fire_sim", "RAC_FIRE_SIM_WGSL", DRAW_WGSL));
		let ext = wgpu::Extent3d { width: NOISE, height: NOISE, depth_or_array_layers: NOISE };
		let tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("fire sim noise"), size: ext, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D3, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
		env.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, &noise_bytes(NOISE), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(NOISE * 4), rows_per_image: Some(NOISE) }, ext);
		let noise = tex.create_view(&Default::default());
		let rep = wgpu::AddressMode::Repeat;
		let samp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("fire sim noise"), address_mode_u: rep, address_mode_v: rep, address_mode_w: rep, mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
		let buf = |label: &str, size: u64, usage: wgpu::BufferUsages| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
		let ubuf = buf("fire sim", std::mem::size_of::<SimU>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
		let parts = buf("fire sim particles", POOL_MAX as u64 * PART_BYTES, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
		let emits = buf("fire sim emitters", (MAX_EMIT * std::mem::size_of::<SimEmit>()) as u64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
		let (cs, vs, fs, vf) = (wgpu::ShaderStages::COMPUTE, wgpu::ShaderStages::VERTEX, wgpu::ShaderStages::FRAGMENT, wgpu::ShaderStages::VERTEX_FRAGMENT);
		let depth_ms = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: crate::viewer::scene_msaa::msaa() > 1 };
		let step_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("fire sim step"), entries: &[layout_entry(0, cs, UNIFORM), layout_entry(1, cs, storage(false)), layout_entry(2, cs, storage(true)), layout_entry(3, cs, NOISE_TEX), layout_entry(4, cs, SAMPLER), layout_entry(5, cs, depth_ms)] });
		let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("fire sim draw"), entries: &[layout_entry(0, vf, UNIFORM), layout_entry(1, vs, storage(true)), layout_entry(2, fs, NOISE_TEX), layout_entry(3, fs, SAMPLER)] });
		let draw_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("fire sim draw"),
			layout: &draw_layout,
			entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: parts.as_entire_binding() }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&noise) }, wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&samp) }],
		});
		let spl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fire sim step"), bind_group_layouts: &[Some(&step_layout)], immediate_size: 0 });
		let step = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("fire sim step"), layout: Some(&spl), module: &step_mod, entry_point: Some("cs_step"), compilation_options: Default::default(), cache: None });
		let l0 = env.pipeline.get_bind_group_layout(0);
		let dpl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fire sim draw"), bind_group_layouts: &[Some(&l0), Some(&draw_layout), Some(env.partfx.soft_layout())], immediate_size: 0 });
		// Premultiplied (a flame adds its light and covers a little, smoke covers), the glow target adds; tested against the level's
		// depth, never written.
		let pre = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
		let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
		let draw = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("fire sim draw"),
			layout: Some(&dpl),
			vertex: wgpu::VertexState { module: &draw_mod, entry_point: Some("vs_part"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState {
				module: &draw_mod,
				entry_point: Some("fs_part"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState { format: env.format, blend: Some(wgpu::BlendState { color: pre, alpha: pre }), write_mask: wgpu::ColorWrites::ALL }), Some(wgpu::ColorTargetState { format: partfx::GLOW_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::ALL })],
			}),
			primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
			depth_stencil: Some(wgpu::DepthStencilState { format: env.depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
			multisample: wgpu::MultisampleState { count: msaa(), ..Default::default() },
			multiview_mask: None,
			cache: None,
		});
		let heat = super::firesim_heat::HeatRes::new(env, &draw_mod, &draw_layout);
		SimRes { step, draw, ubuf, parts, emits, draw_bind, step_layout, noise, samp, step_bind: None, heat }
	}

	/// The step's bind group for depth target `key` (remade with the target).
	fn step_bind(&mut self, device: &wgpu::Device, depth: &wgpu::TextureView, key: (u32, u32, u64)) {
		if self.step_bind.as_ref().is_none_or(|b| b.0 != key) {
			let d = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
			let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("fire sim step"),
				layout: &self.step_layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 1, resource: self.parts.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 2, resource: self.emits.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.noise) },
					wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.samp) },
					wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&d) },
				],
			});
			self.step_bind = Some((key, bind));
		}
	}
}

/// The uniform block of a frame (camera from `cam`, `aspect`; the ring; the knobs).
pub(super) fn uniform(cam: &Camera, at: &StepIn<'_>, dt: f32, clock: f32, ring: [u32; 4], emitters: usize) -> SimU {
	let f = [cam.pitch.cos() * cam.yaw.cos(), cam.pitch.cos() * cam.yaw.sin(), cam.pitch.sin()];
	let right = normalise(cross(f, cam.up));
	let up = cross(right, f);
	let p = cam.pos;
	SimU {
		eye: [p[0], p[1], p[2], dt],
		fwd: [f[0], f[1], f[2], cam.tan_half],
		right: [right[0], right[1], right[2], at.aspect],
		up: [up[0], up[1], up[2], clock],
		ring,
		k: [at.near_far[0], at.near_far[1], emitters as f32, tune("fire_sim_collide", 1.0)],
		m: [tune("fire_sim_rise", 2.2), tune("fire_sim_turb", 2.8), tune("fire_jet_flow", 5.0), tune("fire_sim_drag", 1.0)],
		look: [tune("fire_sim_bright", 1.6), tune("fire_sim_smoke", 1.0), tune("fire_sim_glow", 1.0), tune("fire_sim_tint", 0.0)],
		look2: [tune("fire_sim_streak", 0.02), tune("fire_sim_smoke_a", 0.13), tune("fire_heat_min", 0.01), tune("fire_sim_size", 1.0)],
	}
}

impl FireSim {
	/// The pool starts over at the next step (a swapped world, the warp).
	pub(super) fn forget(&mut self) {
		self.reset = true;
	}

	/// A frame the fire does not draw: nothing is drawn; the next step that runs starts the pool over if the clock jumped meanwhile.
	pub(super) fn idle(&mut self) {
		self.ran = false;
	}

	/// Before the particle pass (the level's depth is complete): this frame's emitters from `vols`, then one step of the pool with the
	/// frame's game time step (`shown`: the fire's draw clock, firefx.rs `shown`).
	pub(super) fn step<'v>(&mut self, env: &FireEnv<'_>, enc: &mut wgpu::CommandEncoder, cam: &Camera, vols: impl Iterator<Item = &'v Vol>, shown: f64, at: &StepIn<'_>) {
		self.ran = false;
		if self.res.is_none() {
			self.res = crate::viewer::pipe_guard::guard(env.device, crate::viewer::pipe_guard::Effect::Fire, || SimRes::new(env));
		}
		let pool = pool_of(at.particles_q);
		let dt = match step_of(self.clock, shown) {
			Some(dt) => dt,
			None => {
				self.reset = true;
				0.0
			}
		};
		self.clock = Some(shown);
		if pool != self.pool {
			(self.pool, self.reset) = (pool, true);
		}
		let Some(res) = self.res.as_mut() else { return };
		self.wants.clear();
		let q = density(at.level);
		self.src.clear();
		self.src.extend(vols.filter_map(|v| want_of(v, cam.pos, q)));
		let full = super::firesim_tongues::split_all(&self.src, MAX_EMIT, &mut self.wants);
		if self.frame % 60 == 0 && tune("fire_vol_log", 0.0) > 0.5 {
			eprintln!("fire_sim: {} emitters, {} rows after the split ({} unbudgeted), cap {MAX_EMIT}", self.src.len(), self.wants.len(), full);
		}
		let n = if dt > 0.0 { plan(&self.wants, dt, pool, self.frame, &mut self.rows) } else { 0 };
		if self.reset {
			self.prev.clear();
			enc.clear_buffer(&res.parts, 0, None);
			(self.head, self.reset, self.last_spawn) = (0, false, shown - QUIET);
		}
		if n > 0 {
			self.last_spawn = shown;
		}
		if shown - self.last_spawn >= QUIET {
			return; // every particle is dead
		}
		self.ran = true;
		let rows = if n > 0 { self.rows.len() } else { 0 };
		let clock = super::firefx_clock::Clock::game(shown).t;
		env.queue.write_buffer(&res.ubuf, 0, bytemuck::bytes_of(&uniform(cam, at, dt, clock, [self.head, n, pool, self.frame], rows)));
		if rows > 0 {
			link_prev(&mut self.rows, &self.prev);
			env.queue.write_buffer(&res.emits, 0, bytemuck::cast_slice(&self.rows));
		}
		if dt > 0.0 {
			self.prev.clear();
			self.prev.extend(self.wants.iter().take(MAX_EMIT).map(|w| w.e));
		}
		if dt > 0.0 {
			res.step_bind(env.device, at.depth, at.key);
			let Some((_, bind)) = res.step_bind.as_ref() else { return };
			let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("fire sim"), timestamp_writes: crate::viewer::passtime::cw("fire sim") });
			cp.set_pipeline(&res.step);
			cp.set_bind_group(0, bind, &[]);
			cp.dispatch_workgroups(pool.div_ceil(64), 1, 1);
		}
		res.heat.pass(env, enc, &res.draw_bind, cam, at, clock, pool);
		self.head = (self.head + n) % pool;
		self.frame = self.frame.wrapping_add(1);
	}

	/// Inside the particle pass (groups 0 and 2 bound by the caller): the pool's smoke, then its flames (the heat buffer's composite).
	pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
		let (true, Some(res)) = (self.ran, self.res.as_ref()) else { return };
		pass.set_pipeline(&res.draw);
		pass.set_bind_group(1, &res.draw_bind, &[]);
		pass.draw(0..6, 0..self.pool);
		res.heat.composite(pass, &res.draw_bind);
	}
}

#[cfg(test)]
#[path = "firesim_tests.rs"]
mod tests;
