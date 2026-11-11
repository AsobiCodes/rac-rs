//! [PC] Fire flipbooks (lane nfire, firebook B, user 2026-11-03: the level's building fires as "flipbooks"): with the fire simulation
//! on, every tongue of the level's fire quads (firefx_ground.rs `Tongue`, one per retail FireKind::Ground flame quad) is drawn as a
//! row of upright camera-facing sprites standing on the quad's bottom edge that play a looping 256-frame flame animation
//! (firebook/firebook.r8g8.z, deflate of the 16 x 16 frames of 128 x 256, R temperature, G opacity, one solver step apart, 64 fps) instead of the simulation's licks; the
//! simulation keeps only their embers (firefx_vol.rs `ember_tongue`). Per quad: `count` sprites along the edge (overlapping by
//! `OVERLAP` of the flame body), each with its own frame offset, mirror and height (+-`H_VAR`), as tall as `H_X` x the quad's side;
//! by a hash about `GAP` of them are left out or drawn small (`gap_of`): separate tongues with dark gaps, not one wall of flame.
//! Fire Medium / Low draw fewer, wider sprites (`max_per`). Drawn in the particle pass after the simulated fire, far to near,
//! premultiplied, emissive (firebook.wgsl). Knobs (RAC_TUNE): fb_h, fb_w, fb_ovl, fb_hvar, fb_gap, fb_bright, fb_alpha, fb_glow, fb_soft, fb_fps.

use super::firefx::{tune, FireEnv};
use super::*;
use rayon::prelude::*;

/// The atlas, deflated (tools/firebook/gen.py writes both files): 16 x 16 frames of 128 x 256 texels, two bytes each (R temperature,
/// G opacity), 256 frames one solver step (1/120 s of the fire's own time) apart = a 4 s loop; frame 4 k is the old 64-frame atlas's frame k.
const ATLAS_Z: &[u8] = include_bytes!("firebook/firebook.r8g8.z");
/// The atlas's size in texels and in frames (firebook.wgsl `COLS`, `FRAMES`).
const AW: u32 = 2048;
const AH: u32 = 4096;
pub(super) const FRAMES: u32 = 256;

/// The unpacked atlas (made on first use; the deflate stream is the exe's, a bad one is a bug caught by the tests).
pub(super) fn atlas() -> &'static [u8] {
	static RAW: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
	RAW.get_or_init(inflate_atlas)
}
/// The atlas unpacked into a fresh buffer (the game path owns and frees it; `atlas()` keeps one copy for the tests).
fn inflate_atlas() -> Vec<u8> {
	crate::texpack::inflate::zlib(ATLAS_Z, (AW * AH * 2) as usize).ok().filter(|v| v.len() == (AW * AH * 2) as usize).unwrap_or_else(|| vec![0; (AW * AH * 2) as usize])
}
/// The mip chain being made in the background (`prewarm`), taken by the first draw (`take_mips`).
static PREWARM: std::sync::Mutex<Option<std::thread::JoinHandle<Vec<Vec<u8>>>>> = std::sync::Mutex::new(None);
/// Set once the background job was started (or the chain taken), so a later frame never starts a second one.
static PREWARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Starts the atlas inflate and the mip build on a worker thread, once (`firefx.rs` `gather`, the first frame with flipbook fire on,
/// before any fire is in view), so the first draw does not hitch on 16 MB of inflate and mips.
pub(super) fn prewarm() {
	if PREWARMED.swap(true, std::sync::atomic::Ordering::AcqRel) {
		return;
	}
	if let Ok(h) = std::thread::Builder::new().name("firebook-mips".into()).spawn(mips) {
		if let Ok(mut g) = PREWARM.lock() {
			*g = Some(h);
		}
	}
}

/// The mip chain for the upload: the background job's (blocks only while it is unfinished) or made here. The caller drops it after
/// the upload, so no CPU copy of the atlas stays.
fn take_mips() -> Vec<Vec<u8>> {
	PREWARMED.store(true, std::sync::atomic::Ordering::Release);
	let job = PREWARM.lock().ok().and_then(|mut g| g.take());
	job.and_then(|h| h.join().ok()).unwrap_or_else(mips)
}
/// Mip levels made on the CPU (the last one = cells of 8 x 16: a far fire does not sparkle, no level mixes two cells).
const MIPS: u32 = 5;
/// The frame's width over its height.
pub(super) const ASPECT: f32 = 0.5;
/// The share of a frame's width the flame's body fills.
pub(super) const BODY: f32 = 0.6;
/// Neighbouring sprites of one quad overlap by this share of the flame body (0.7 with `W_X` 0.8 and `H_X` 2.0: nfire28, the mass of the
/// nfire26 fire the user liked once the mirror's triangles no longer make tongues, firefx_ground.rs `is_tri`; denser blew the close
/// tower view out to white: pp/sheet_mid.png).
pub(super) const OVERLAP: f32 = 0.7;
/// Each sprite's height varies by up to this share either way.
pub(super) const H_VAR: f32 = 0.2;
/// About this share of the sprites is a gap: half of them left out, half drawn at `SMALL` of their height.
pub(super) const GAP: f32 = 0.08;
pub(super) const SMALL: f32 = 0.5;
/// The sprite's height over the retail quad's side (calibrated on the board, b\fb_board.ps1: the flames hug the burning dome).
/// The playback rate in frames a second (the flame's licking is slow and heavy; 256 frames at 64 fps = a 4 s loop, as the old 64 at 16),
/// the share of a sprite's height
/// sunk below its foot (the frame's bottom is hidden in the surface), the share of the opacity that occludes (the rest only adds light:
/// overlapping sprites change little when their order swaps).
pub(super) const FPS: f32 = 64.0;
pub(super) const SINK: f32 = 0.2;
pub(super) const OVER: f32 = 0.6;
pub(super) const H_X: f32 = 2.0;
/// The share of the tongue's edge the sprites spread over (nfire24..27: 0.49, with the mirror's extra tongues; see `OVERLAP`).
pub(super) const W_X: f32 = 0.8;
/// The most sprites one quad gets at a Fire level (Low, Medium, High).
pub(super) fn max_per(level: u8) -> usize {
	match level {
		0 => 0,
		1 => 2,
		2 => 3,
		_ => 6,
	}
}
/// At most this many sprites a frame (the nearest are kept).
pub(super) const MAX_SPR: usize = 1024;

/// One sprite: GPU row (a: foot + height, b: width, frame offset, mirror, fade, c: tint, its tongue's seed (only the dev dump reads it)).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Spr {
	pub a: [f32; 4],
	pub b: [f32; 4],
	pub c: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct BookU {
	eye: [f32; 4],
	right: [f32; 4],
	t: [f32; 4],
	k: [f32; 4],
}

fn hash(seed: f32, i: u32) -> f32 {
	let mut x = (seed * 65536.0) as u32 ^ i.wrapping_mul(0x9E37_79B9);
	x ^= x >> 16;
	x = x.wrapping_mul(0x7FEB_352D);
	x ^= x >> 15;
	x = x.wrapping_mul(0x846C_A68B);
	x ^= x >> 16;
	(x >> 8) as f32 / 16_777_216.0
}

/// The sprites' height for a quad of side `side`.
pub(super) fn height_of(side: f32) -> f32 {
	side * tune("fb_h", H_X)
}

/// The spacing of the sprites on an edge under flames `h` m tall: one flame body less the overlap.
fn step_of(h: f32) -> f32 {
	(h * ASPECT * BODY * (1.0 - tune("fb_ovl", OVERLAP).clamp(0.0, 0.9))).max(0.05)
}

/// How many sprites stand on a bottom edge of `edge` m under flames `h` m tall (at most `max`): one per `step_of`.
pub(super) fn count(edge: f32, h: f32, max: usize) -> usize {
	((edge / step_of(h)).ceil() as usize).clamp(1, max.max(1))
}

/// The height factor of a sprite with the hash `g` (0..1) under the gap share `gap`: None = left out, `SMALL` = a small tongue, else 1.
/// A lone sprite (a quad of one flame body) stays whole: the gaps break rows, the big single tongues carry the fire.
pub(super) fn gap_of(g: f32, gap: f32, lone: bool) -> Option<f32> {
	if lone || g >= gap {
		Some(1.0)
	} else if g < 0.5 * gap {
		None
	} else {
		Some(SMALL)
	}
}

/// The sprites of one retail flame quad: foot `base` (the bottom edge's middle), `_view` (unused: the positions are fixed in the world), bottom edge `edge` m, side `side` m, the tongue's `seed`, `fade`, `tint`.
pub(super) fn place(base: [f32; 3], _view: [f32; 3], edge: f32, side: f32, seed: f32, fade: f32, tint: [f32; 3], level: u8, out: &mut Vec<Spr>) {
	let max = max_per(level);
	if max == 0 || fade <= 0.01 {
		return;
	}
	let edge = edge * tune("fb_w", W_X).clamp(0.2, 2.0);
	let h = height_of(side);
	let n = count(edge, h, max);
	let step = step_of(h);
	// Fewer sprites than the edge wants (Medium, Low): each a little wider, the edge still covered.
	let widen = (edge / (n as f32 * step)).clamp(1.0, 1.6);
	let var = tune("fb_hvar", H_VAR).clamp(0.0, 0.9);
	let gap = tune("fb_gap", GAP).clamp(0.0, 0.9);
	// The sprites stand on a disc of diameter `edge` round the foot, a sunflower spread turned per tongue: fixed in the world (a row along
	// the view's right slid about the foot when the camera turned) and as wide from every side.
	let nd = (n * 3).div_ceil(2).max(1);
	let turn = hash(seed, 777) * std::f32::consts::TAU;
	for i in 0..nd as u32 {
		let r = |k: u32| hash(seed, 8 * i + k);
		let Some(g) = gap_of(r(4), gap, nd == 1) else { continue };
		let (rad, ang) = (0.5 * edge * ((i as f32 + 0.5) / nd as f32).sqrt(), turn + i as f32 * 2.399_963);
		let hi = h * g * (1.0 - var + 2.0 * var * r(1));
		let p = [base[0] + rad * ang.cos(), base[1] + rad * ang.sin(), base[2]];
		let mirror = if r(2) < 0.5 { -1.0 } else { 1.0 };
		out.push(Spr { a: [p[0], p[1], p[2], hi], b: [hi * ASPECT * widen, r(3) * FRAMES as f32, mirror, fade], c: [tint[0], tint[1], tint[2], seed] });
	}
}

/// Far to near from `eye`; over `MAX_SPR` the nearest are kept.
pub(super) fn sort(v: &mut Vec<Spr>, eye: [f32; 3]) {
	let d = |s: &Spr| (s.a[0] - eye[0]).powi(2) + (s.a[1] - eye[1]).powi(2) + (s.a[2] + 0.5 * s.a[3] - eye[2]).powi(2);
	v.sort_by(|a, b| d(a).total_cmp(&d(b)));
	v.truncate(MAX_SPR);
	v.reverse();
}

/// The atlas with its mip chain: per 2 x 2 block the mean opacity and the opacity-weighted mean temperature.
fn mips() -> Vec<Vec<u8>> {
	let mut out = vec![inflate_atlas()];
	let (mut w, mut h) = (AW, AH);
	for _ in 1..MIPS {
		let src = out.last().expect("level");
		let (nw, nh) = (w / 2, h / 2);
		let mut dst = vec![0u8; (nw * nh * 2) as usize];
		// (Rows in parallel: the atlas is 32 MB of texels, the first fire frame must not hitch.)
		dst.par_chunks_mut((nw * 2) as usize).enumerate().for_each(|(y, row)| {
			let y = y as u32;
			for x in 0..nw {
				let (mut t, mut a) = (0u32, 0u32);
				for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
					let o = (((2 * y + dy) * w + 2 * x + dx) * 2) as usize;
					t += src[o] as u32 * src[o + 1] as u32;
					a += src[o + 1] as u32;
				}
				let o = (x * 2) as usize;
				row[o] = if a > 0 { (t / a) as u8 } else { 0 };
				row[o + 1] = ((a + 2) / 4) as u8;
			}
		});
		out.push(dst);
		(w, h) = (nw, nh);
	}
	out
}

struct Res {
	pipe: wgpu::RenderPipeline,
	/// The order-independent pair (knob fb_oi): first every sprite darkens what is behind it, then every sprite adds its light.
	dark: wgpu::RenderPipeline,
	light: wgpu::RenderPipeline,
	ubuf: wgpu::Buffer,
	sbuf: wgpu::Buffer,
	bind: wgpu::BindGroup,
}

impl Res {
	fn new(env: &FireEnv<'_>) -> Res {
		let device = env.device;
		let src = match std::env::var_os("RAC_FIREBOOK_WGSL").and_then(|p| std::fs::read_to_string(p).ok()) {
			Some(s) => std::borrow::Cow::Owned(s),
			None => std::borrow::Cow::Borrowed(WGSL),
		};
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("firebook"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("firebook", src)) });
		let size = wgpu::Extent3d { width: AW, height: AH, depth_or_array_layers: 1 };
		let tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("firebook atlas"), size, mip_level_count: MIPS, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rg8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
		let chain = take_mips();
		for (l, data) in chain.iter().enumerate() {
			let (w, h) = (AW >> l, AH >> l);
			env.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: l as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, data, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 2), rows_per_image: Some(h) }, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
		}
		drop(chain); // the CPU copy is not needed after the upload
		let view = tex.create_view(&Default::default());
		let cl = wgpu::AddressMode::ClampToEdge;
		let lin = wgpu::FilterMode::Linear;
		let samp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("firebook"), address_mode_u: cl, address_mode_v: cl, mag_filter: lin, min_filter: lin, mipmap_filter: wgpu::MipmapFilterMode::Linear, ..Default::default() });
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("firebook"), size: std::mem::size_of::<BookU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let sbuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("firebook sprites"), size: (MAX_SPR * std::mem::size_of::<Spr>()) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let (vs, fs, vf) = (wgpu::ShaderStages::VERTEX, wgpu::ShaderStages::FRAGMENT, wgpu::ShaderStages::VERTEX_FRAGMENT);
		let e = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("firebook"),
			entries: &[
				e(0, vf, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
				e(1, vs, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }),
				e(2, fs, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
				e(3, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
			],
		});
		let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("firebook"),
			layout: &layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 1, resource: sbuf.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view) },
				wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&samp) },
			],
		});
		let l0 = env.pipeline.get_bind_group_layout(0);
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("firebook"), bind_group_layouts: &[Some(&l0), Some(&layout), Some(env.partfx.soft_layout())], immediate_size: 0 });
		let bf = |src_factor, dst_factor| wgpu::BlendComponent { src_factor, dst_factor, operation: wgpu::BlendOperation::Add };
		use wgpu::BlendFactor::{One, OneMinusSrcAlpha, Zero};
		let (pre, add, dim, keep) = (bf(One, OneMinusSrcAlpha), bf(One, One), bf(Zero, OneMinusSrcAlpha), bf(Zero, One));
		let target = |blend: wgpu::BlendState| Some(wgpu::ColorTargetState { format: env.format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL });
		let glow = Some(wgpu::ColorTargetState { format: partfx::GLOW_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::ALL });
		let make = |entry: &str, targets: &[Option<wgpu::ColorTargetState>]| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some("firebook"),
				layout: Some(&pl),
				vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_book"), compilation_options: Default::default(), buffers: &[] },
				fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(entry), compilation_options: Default::default(), targets }),
				primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
				depth_stencil: Some(wgpu::DepthStencilState { format: env.depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
				multisample: wgpu::MultisampleState { count: msaa(), ..Default::default() },
				multiview_mask: None,
				cache: None,
			})
		};
		let pipe = make("fs_book", &[target(wgpu::BlendState { color: pre, alpha: pre }), glow.clone()]);
		// Order independent: the darkening (dst x (1 - a x over), alpha as before) of every sprite first, then the light (dst + e a, glow as before).
		// The pass has the glow target too: the darkening writes nothing there (fs_dark has no output for it).
		let no_glow = Some(wgpu::ColorTargetState { format: partfx::GLOW_FORMAT, blend: None, write_mask: wgpu::ColorWrites::empty() });
		let dark = make("fs_dark", &[target(wgpu::BlendState { color: dim, alpha: pre }), no_glow]);
		let light = make("fs_light", &[target(wgpu::BlendState { color: add, alpha: keep }), glow]);
		Res { pipe, dark, light, ubuf, sbuf, bind }
	}
}

pub(super) const WGSL: &str = include_str!("firebook.wgsl");

/// This frame's flipbook sprites and the GPU objects (made on the first frame that has a sprite).
#[derive(Default)]
pub struct FireBook {
	res: std::cell::OnceCell<Option<Res>>,
	pub(super) sprites: Vec<Spr>,
	/// The view's horizontal right of this frame (firefx.rs `gather`): the sprites face the camera about the vertical.
	pub(super) across: [f32; 3],
}

impl FireBook {
	/// Flipbooks replace the simulation's licks of the level's fires (fire simulation on; dev knob `fb` 0 = the licks as before).
	pub(super) fn on() -> bool {
		tune("fb", 1.0) > 0.5
	}

	/// Inside the particle pass (groups 0 and 2 bound by the caller), after the simulated fire: `sorted` (far to near, `sort`).
	/// `t` = the fire's draw clock (game time, deterministic under the test clock).
	pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, env: &FireEnv<'_>, cam: &Camera, t: f64, sorted: &[Spr]) {
		if sorted.is_empty() {
			return;
		}
		let res = self.res.get_or_init(|| crate::viewer::pipe_guard::guard(env.device, crate::viewer::pipe_guard::Effect::Fire, || Res::new(env)));
		let Some(res) = res.as_ref() else { return };
		let n = sorted.len().min(MAX_SPR);
		let right = self.across;
		// The loop's phase in frames, taken in f64 (no pop after hours).
		let phase = (t * tune("fb_fps", FPS) as f64).rem_euclid(FRAMES as f64) as f32;
		let u = BookU {
			eye: [cam.pos[0], cam.pos[1], cam.pos[2], 0.0],
			right: [right[0], right[1], right[2], 0.0],
			t: [phase, tune("fb_bright", 1.3), tune("fb_glow", 0.8), tune("fb_alpha", 1.6)],
			k: [0.0, tune("fb_soft", 0.5), tune("fb_sink", SINK), tune("fb_over", OVER)],
		};
		env.queue.write_buffer(&res.ubuf, 0, bytemuck::bytes_of(&u));
		env.queue.write_buffer(&res.sbuf, 0, bytemuck::cast_slice(&sorted[..n]));
		pass.set_bind_group(1, &res.bind, &[]);
		if tune("fb_oi", 0.0) > 0.5 {
			pass.set_pipeline(&res.dark);
			pass.draw(0..6, 0..n as u32);
			pass.set_pipeline(&res.light);
			pass.draw(0..6, 0..n as u32);
		} else {
			pass.set_pipeline(&res.pipe);
			pass.draw(0..6, 0..n as u32);
		}
		crate::viewer::passtime::lap(pass, "firebook");
	}
}

#[cfg(test)]
#[path = "firebook_tests.rs"]
mod tests;
