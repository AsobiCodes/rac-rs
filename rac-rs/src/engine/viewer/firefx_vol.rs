//! [PC] Fire, true 3D: the flames of burning things (a level fire, a burning enemy, a burn puff), the fireballs and the Pyrocitor's
//! stream are volumes, not cards (firefx_vol.wgsl). A box around each one is rasterised; its pixels march through a turbulent flame
//! field (a small tiling 3D noise texture, domain-warped and rising, stretched upward into tongues) inside one of three kinds: a
//! narrowing column (flame, `KIND_FLAME`), a ball (fireball, `KIND_BALL`) or a slab of the jet (`KIND_JET`, firefx_jet_vol.rs).
//! The march ends at the scene depth, hard where something stands in front of the fire (it hides what lies behind it), softly where
//! the surface lies inside the fire (shader SOFT: no hard-edged hole round a ledge in a big fire). It looks the same
//! from every side, from straight below and from above; nothing faces the camera. Volumes are drawn far to near (box centres, ties by
//! seed: a stable order). Quality (`quality`): march steps and noise taps by the Fire level (a jet slab takes 1.5x the steps, 2x on
//! Low); on Low the noise is read from coarser mips (`NOISE_MIPS`, shader `lod`) where the long steps would alias its finest cells
//! into grain. Near the eye a jet slab fades out (shader NEAR), a fireball the eye is in fades as a whole (BALL_NEAR). Fire Off:
//! none of this exists. Dev knobs (`RAC_TUNE`, ENV.md): fire_vol_steps fire_vol_dens fire_vol_cut fire_vol_opac fire_vol_bright
//! fire_vol_rise fire_vol_r fire_vol_smoke; the jet's fire_jet_flow / core / turb / hot (`rows`); `RAC_FIRE_VOL_WGSL=<file>` loads
//! the shader from disk.

use super::firefx::{tune, FireEnv};
use super::*;

const WGSL: &str = include_str!("firefx_vol.wgsl");
/// Volumes a frame (level fires first, then the sprites' flames and fireballs).
pub(super) const MAX_VOLS: usize = 384;
/// Voxels per side of the noise texture (tiles; 128 KB).
const NOISE: u32 = 32;

/// The kinds of volume (b.w): an upright flame, a fireball, a slab of the Pyrocitor's jet (firefx_jet_vol.rs).
pub(super) const KIND_FLAME: f32 = 0.0;
pub(super) const KIND_BALL: f32 = 1.0;
pub(super) const KIND_JET: f32 = 2.0;

/// One volume, as the instance rows of the shader: a = (base or centre, height), b = (radius, seed, fade or age, kind), c = (tint,
/// strength); a jet slab uses all seven rows its own way (firefx_jet_vol.rs `slabs`), the others leave d..g zero.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Vol {
	pub a: [f32; 4],
	pub b: [f32; 4],
	pub c: [f32; 4],
	pub d: [f32; 4],
	pub e: [f32; 4],
	pub f: [f32; 4],
	pub g: [f32; 4],
}

fn is_jet(v: &Vol) -> bool {
	v.b[3] > 1.5
}

fn is_ball(v: &Vol) -> bool {
	v.b[3] > 0.5 && !is_jet(v)
}

/// An upright flame: foot centre `base`, height `h`, radius `r` at the foot.
pub(super) fn flame(base: [f32; 3], h: f32, r: f32, seed: f32, fade: f32, tint: [f32; 3]) -> Vol {
	let r = r * tune("fire_vol_r", 1.0);
	Vol { a: [base[0], base[1], base[2], h.max(0.05)], b: [r.max(0.02), seed, fade.clamp(0.0, 1.0), KIND_FLAME], c: [tint[0], tint[1], tint[2], 1.0], ..bytemuck::Zeroable::zeroed() }
}

/// firefx_vol.wgsl `STRETCH` (the flame's noise tiles stretched up).
const STRETCH: f32 = 2.6;

/// A flame's own noise (the `d` row, firefx_vol.wgsl `flame_at`): the tile size across (m) and the scroll in tiles per second at fire
/// speed 1, as `flame_at` derives them from the height `h` and the foot radius `r` (before `fire_vol_r`). A flame that keeps its
/// identity while its size changes (a tracked ground tongue, a burn cluster) keeps the noise it started with: the scroll is quantised
/// per rate (`scroll`), so a rate taken anew from a slightly changed size shifted the whole pattern (lane nfire, agent 6).
pub(super) fn noise_of(h: f32, r: f32) -> [f32; 2] {
	let r = (r * tune("fire_vol_r", 1.0)).max(0.02);
	let s = (r * 1.3).max(0.25);
	[s, (0.9 + 0.25 * h.max(0.05) / s) / (s * STRETCH)]
}

/// `flame` with its own noise (`noise_of`; [0, 0] = from its size, as `flame`).
pub(super) fn flame_own(base: [f32; 3], h: f32, r: f32, seed: f32, fade: f32, tint: [f32; 3], noise: [f32; 2]) -> Vol {
	Vol { d: [noise[0], noise[1], 0.0, 0.0], ..flame(base, h, r, seed, fade, tint) }
}

/// `v` marked as one of the level's fire tongues (d[2] = 1; read by firesim.rs `want_of`: the sim keeps it inside its retail quad as
/// separate licks, lane nfire agent 17). The volume path ignores the mark.
pub(super) fn level_tongue(v: Vol) -> Vol {
	Vol { d: [v.d[0], v.d[1], 1.0, v.d[3]], ..v }
}

/// `v` marked as a level fire tongue drawn as flipbooks (firebook.rs, d[2] = 2): the sim spawns only its embers (firesim.rs `want_of`,
/// firesim_step.wgsl `spawn_flame`). The volume path ignores the mark.
pub(super) fn ember_tongue(v: Vol) -> Vol {
	Vol { d: [v.d[0], v.d[1], 2.0, v.d[3]], ..v }
}

/// The size rule (user 2026-11-03): a simulated flame reaches at most this many times its retail sprite's half side.
pub(super) const FOOT_X: f32 = 1.2;

/// A fireball: centre, radius, seed, age (1 new .. 0 dying). The caller may set d[0], the retail footprint's radius (firesim.rs).
pub(super) fn ball(centre: [f32; 3], r: f32, seed: f32, age: f32, tint: [f32; 3]) -> Vol {
	Vol { a: [centre[0], centre[1], centre[2], 0.0], b: [r.max(0.02), seed, age.clamp(0.0, 1.0), KIND_BALL], c: [tint[0], tint[1], tint[2], 1.0], ..bytemuck::Zeroable::zeroed() }
}

/// The middle of a volume's box (a jet slab: the middle of its axis).
pub(super) fn centre(v: &Vol) -> [f32; 3] {
	if is_jet(v) {
		std::array::from_fn(|k| 0.5 * (v.a[k] + v.d[k]))
	} else if is_ball(v) {
		[v.a[0], v.a[1], v.a[2]]
	} else {
		[v.a[0], v.a[1], v.a[2] + 0.45 * v.a[3]]
	}
}

/// The dense core of a volume as an ellipsoid (centre, horizontal radius, vertical half-extent): where `flame_at` / `ball_at` in
/// firefx_vol.wgsl are thick enough to hide what lies behind them. A flame narrows to its tip, so its core sits low (foot to ~2/3 of
/// the height); a fireball's core shrinks as it dies; a jet slab's is a ball in its middle, 0.6 x its mean radius. The smoke pass ends
/// its march there (volsmoke/fire_cut.rs).
pub(super) fn core(v: &Vol) -> ([f32; 3], f32, f32) {
	let r = v.b[0];
	if is_jet(v) {
		let k = 0.3 * (v.a[3] + v.d[3]);
		(centre(v), k, k)
	} else if is_ball(v) {
		let k = r * (0.4 + 0.25 * v.b[2].clamp(0.0, 1.0));
		([v.a[0], v.a[1], v.a[2]], k, k)
	} else {
		([v.a[0], v.a[1], v.a[2] + 0.3 * v.a[3]], 0.7 * r, 0.35 * v.a[3])
	}
}

/// Far to near from `eye` (ties: by seed, then by position), so overlapping volumes blend the same way every frame.
pub(super) fn order(vols: &mut [Vol], eye: [f32; 3]) {
	let d = |v: &Vol| super::firefx::dist(centre(v), eye);
	vols.sort_by(|x, y| d(y).total_cmp(&d(x)).then(x.b[1].total_cmp(&y.b[1])).then(x.a[0].total_cmp(&y.a[0])));
}

/// The frame's draw list: far to near (`order`) and, over `MAX_VOLS`, the FARTHEST dropped (review R7-1: a volume's retail quad or
/// sprite is already hidden, so a cut must never take the closest fires). firefx.rs keeps the count under the cap; this is the guard.
pub(super) fn nearest(vols: &mut Vec<Vol>, eye: [f32; 3]) {
	order(vols, eye);
	vols.drain(..vols.len().saturating_sub(MAX_VOLS));
}

/// Dev knob `fire_vol_log=1`: the camera and the nearest volumes on stderr every 60th frame (aiming RAC_VEG_CAM for picture checks).
pub(super) fn log(vols: &[Vol], cam: &Camera) {
	static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
	if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 60 != 0 {
		return;
	}
	let p = cam.pos;
	eprintln!("fire_vol: cam {:.2} {:.2} {:.2} yaw {:.3} pitch {:.3}, {} volumes", p[0], p[1], p[2], cam.yaw, cam.pitch, vols.len());
	for v in vols.iter().rev().take(8) {
		if is_jet(v) {
			eprintln!("fire_vol:   kind 2 at {:.2} {:.2} {:.2} r {:.2} dir {:.3} {:.3} {:.3}", v.a[0], v.a[1], v.a[2], v.a[3], v.g[0], v.g[1], v.g[2]);
		} else {
			eprintln!("fire_vol:   kind {} at {:.2} {:.2} {:.2} h {:.2} r {:.2}", v.b[3], v.a[0], v.a[1], v.a[2], v.a[3], v.b[0]);
		}
	}
}

/// (march steps across the column's full width, noise taps) by Fire level: Low 12 / 1, Medium 24 / 2, High 40 / 3 (shorter chords
/// take fewer at the same spacing, firefx_vol.wgsl fs_vol). `fire_vol_steps` > 0 overrides the steps.
pub(super) fn quality(level: u8) -> (f32, f32) {
	let (steps, taps) = match level {
		0 | 1 => (12.0, 1.0),
		2 => (24.0, 2.0),
		_ => (40.0, 3.0),
	};
	let o = tune("fire_vol_steps", 0.0);
	(if o >= 2.0 { o.min(64.0) } else { steps }, taps)
}

/// The shader's uniform rows (struct VolU). fwd.w = 2 tan(half the vertical field of view): the shader's pixel footprint (`pix_scale`,
/// the noise taps' mip and band by pixel size: far fires do not alias). The march jitter is a fixed per-pixel pattern (a per-frame one
/// flickers 2-3x more with frame smoothing on, review R7-5). The last row is the Pyrocitor jet's (the cards' knobs, firefx.rs
/// `uniform_rows`): flow speed along it (m/s), how fast it cools, its warp, its core gain.
pub(super) fn rows(level: u8, t: f32, eye: [f32; 3], fwd: [f32; 3], tan_half: f32) -> [[f32; 4]; 6] {
	let (steps, taps) = quality(level);
	[
		[eye[0], eye[1], eye[2], t],
		[fwd[0], fwd[1], fwd[2], 2.0 * tan_half],
		[steps, taps, tune("fire_glow", 1.0), tune("fire_core", 1.0)],
		[tune("fire_turb", 1.0), tune("fire_vol_rise", 1.6), tune("fire_vol_dens", 3.0), tune("fire_tint", 0.35)],
		[tune("fire_vol_smoke", 1.0) * tune("fire_smoke", 1.0), tune("fire_vol_opac", 0.55), tune("fire_vol_bright", 1.0), tune("fire_vol_cut", 1.0)],
		[tune("fire_jet_flow", 5.0), tune("fire_jet_core", 1.25), tune("fire_jet_turb", 1.0), tune("fire_jet_hot", 1.0)],
	]
}

/// The noise: R and G, B three decorrelated value-noise fbm (the warp reads all three, the field R, the detail B), A unused.
fn noise_bytes(n: u32) -> Vec<u8> {
	use super::volfog::noise::fbm;
	let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
	let mut out = Vec::with_capacity((n * n * n * 4) as usize);
	for z in 0..n {
		for y in 0..n {
			for x in 0..n {
				let p = [x as f32 / n as f32, y as f32 / n as f32, z as f32 / n as f32];
				out.extend([to_u8(fbm(p, 4, 3, 3)), to_u8(fbm(p, 4, 3, 5)), to_u8(fbm(p, 8, 2, 13)), 255]);
			}
		}
	}
	out
}

/// The noise texture's mip levels below the full one (32 -> 16 -> 8).
const NOISE_MIPS: u32 = 3;

/// The next mip level of an n^3 RGBA8 noise: each texel the mean of its 2x2x2 block (n even, so the tiling holds). The shader reads
/// a coarser level where the march steps are too long for the finest cells (Low): band-limited, so they cannot alias into grain.
fn noise_half(src: &[u8], n: u32) -> Vec<u8> {
	let (n, h) = (n as usize, (n / 2) as usize);
	let mut out = Vec::with_capacity(h * h * h * 4);
	for z in 0..h {
		for y in 0..h {
			for x in 0..h {
				for ch in 0..4 {
					let mut s = 0u32;
					for (dz, dy, dx) in (0..8).map(|k| (k >> 2, (k >> 1) & 1, k & 1)) {
						s += src[(((2 * z + dz) * n + 2 * y + dy) * n + 2 * x + dx) * 4 + ch] as u32;
					}
					out.push(((s + 4) / 8) as u8);
				}
			}
		}
	}
	out
}

/// The GPU side: pipeline, uniform, noise texture, instance buffer.
pub(super) struct VolRes {
	pipe: wgpu::RenderPipeline,
	ubuf: wgpu::Buffer,
	bind: wgpu::BindGroup,
	inst: std::cell::RefCell<Option<(wgpu::Buffer, u64)>>,
}

impl VolRes {
	pub(super) fn new(env: &FireEnv<'_>) -> VolRes {
		let device = env.device;
		let src = match std::env::var_os("RAC_FIRE_VOL_WGSL").and_then(|p| std::fs::read_to_string(p).ok()) {
			Some(s) => std::borrow::Cow::Owned(s),
			None => std::borrow::Cow::Borrowed(WGSL),
		};
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("fire volumes"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("fire_vol", src)) });
		let ext = wgpu::Extent3d { width: NOISE, height: NOISE, depth_or_array_layers: NOISE };
		let tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("fire noise"), size: ext, mip_level_count: NOISE_MIPS, sample_count: 1, dimension: wgpu::TextureDimension::D3, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
		let (mut data, mut n) = (noise_bytes(NOISE), NOISE);
		for level in 0..NOISE_MIPS {
			let size = wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n };
			env.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, &data, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(n * 4), rows_per_image: Some(n) }, size);
			if level + 1 < NOISE_MIPS {
				data = noise_half(&data, n);
				n /= 2;
			}
		}
		let rep = wgpu::AddressMode::Repeat;
		let samp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("fire noise"), address_mode_u: rep, address_mode_v: rep, address_mode_w: rep, mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, ..Default::default() });
		let view = tex.create_view(&Default::default());
		let vis = wgpu::ShaderStages::VERTEX_FRAGMENT;
		let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty, count: None };
		let own = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("fire volumes"),
			entries: &[
				entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
				entry(1, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
				entry(2, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
			],
		});
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("fire volumes"), size: 6 * 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("fire volumes"),
			layout: &own,
			entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&samp) }],
		});
		let l0 = env.pipeline.get_bind_group_layout(0);
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fire volumes"), bind_group_layouts: &[Some(&l0), Some(&own), Some(env.partfx.soft_layout())], immediate_size: 0 });
		// Premultiplied (the flame adds its light, its smoke and body cover a little), the glow target adds; no depth test: the march
		// ends at the scene depth itself, and a box's far face may lie behind a wall the flame stands in front of.
		let pre = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
		let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
		let attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4];
		let vl = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vol>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &attrs };
		let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("fire volumes"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_vol"), compilation_options: Default::default(), buffers: &[Some(vl)] },
			fragment: Some(wgpu::FragmentState {
				module: &module,
				entry_point: Some("fs_vol"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState { format: env.format, blend: Some(wgpu::BlendState { color: pre, alpha: pre }), write_mask: wgpu::ColorWrites::ALL }), Some(wgpu::ColorTargetState { format: partfx::GLOW_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::ALL })],
			}),
			primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
			depth_stencil: Some(wgpu::DepthStencilState { format: env.depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }),
			multisample: wgpu::MultisampleState { count: msaa(), ..Default::default() },
			multiview_mask: None,
			cache: None,
		});
		VolRes { pipe, ubuf, bind, inst: Default::default() }
	}

	/// Draws `vols` (already far to near, `nearest`) inside the particle pass; the soft-particle group 2 is bound by the caller.
	pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, env: &FireEnv<'_>, vols: &[Vol], rows: &[[f32; 4]; 6]) {
		let vols = &vols[vols.len().saturating_sub(MAX_VOLS)..]; // (the nearest, should a caller ever pass more)
		if vols.is_empty() {
			return;
		}
		env.queue.write_buffer(&self.ubuf, 0, bytemuck::cast_slice(rows));
		let bytes: &[u8] = bytemuck::cast_slice(vols);
		let mut cell = self.inst.borrow_mut();
		if cell.as_ref().is_none_or(|b| b.1 < bytes.len() as u64) {
			let size = (bytes.len() as u64).next_power_of_two();
			let b = env.device.create_buffer(&wgpu::BufferDescriptor { label: Some("fire volume instances"), size, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
			*cell = Some((b, size));
		}
		let vb = &cell.as_ref().expect("fire volume buffer").0;
		env.queue.write_buffer(vb, 0, bytes);
		pass.set_pipeline(&self.pipe);
		pass.set_bind_group(1, &self.bind, &[]);
		pass.set_vertex_buffer(0, vb.slice(..bytes.len() as u64));
		pass.draw(0..36, 0..vols.len() as u32);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn module() -> (naga::Module, naga::valid::ModuleInfo) {
		let m = naga::front::wgsl::parse_str(WGSL).unwrap_or_else(|e| panic!("fire volumes: {}", e.emit_to_string(WGSL)));
		let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("fire volumes validate: {e:?}"));
		(m, info)
	}

	#[test]
	fn the_volume_shader_validates_with_its_entry_points() {
		let (m, _) = module();
		let names: Vec<_> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
		assert!(names.contains(&"vs_vol") && names.contains(&"fs_vol"));
		assert_eq!(std::mem::size_of::<Vol>(), 112, "seven vec4 instance rows");
	}

	#[test]
	fn a_flame_keeps_the_noise_it_is_given() {
		assert!(WGSL.contains(&format!("const STRETCH: f32 = {STRETCH};")), "noise_of mirrors the shader's stretch");
		let n = noise_of(4.0, 1.0);
		assert!((n[0] - 1.3).abs() < 1e-6 && (n[1] - (0.9 + 0.25 * 4.0 / 1.3) / (1.3 * STRETCH)).abs() < 1e-6);
		assert_eq!(noise_of(4.0, 0.01)[0], 0.25, "the smallest tile, as flame_at");
		let (v, w) = (flame([0.0; 3], 4.0, 1.0, 0.5, 1.0, [1.0; 3]), flame_own([0.0; 3], 4.0, 1.0, 0.5, 1.0, [1.0; 3], n));
		assert_eq!(v.d, [0.0; 4], "no noise row: from its size, as before");
		assert_eq!((w.a, w.b, w.c, w.d), (v.a, v.b, v.c, [n[0], n[1], 0.0, 0.0]));
	}

	#[cfg(windows)]
	#[test]
	fn the_volume_shader_writes_hlsl() {
		let (m, info) = module();
		let (options, pipeline) = (naga::back::hlsl::Options::default(), naga::back::hlsl::PipelineOptions::default());
		let mut out = String::new();
		naga::back::hlsl::Writer::new(&mut out, &options, &pipeline).write(&m, &info, None).expect("HLSL writes");
		assert!(out.contains("fs_vol"));
	}

	#[test]
	fn volumes_are_drawn_far_to_near_in_a_stable_order() {
		let mut v = vec![flame([1.0, 0.0, 0.0], 2.0, 0.5, 0.1, 1.0, [1.0; 3]), ball([9.0, 0.0, 0.0], 1.0, 0.2, 1.0, [1.0; 3]), flame([5.0, 0.0, 0.0], 2.0, 0.5, 0.3, 1.0, [1.0; 3])];
		order(&mut v, [0.0; 3]);
		assert_eq!(v.iter().map(|x| x.a[0]).collect::<Vec<_>>(), vec![9.0, 5.0, 1.0]);
		// Two at one distance: the seed decides, whatever order they came in.
		let (p, q) = (flame([3.0, 0.0, 0.0], 2.0, 0.5, 0.7, 1.0, [1.0; 3]), flame([-3.0, 0.0, 0.0], 2.0, 0.5, 0.2, 1.0, [1.0; 3]));
		let (mut x, mut y) = (vec![p, q], vec![q, p]);
		order(&mut x, [0.0; 3]);
		order(&mut y, [0.0; 3]);
		assert_eq!(x, y);
	}

	/// R7-1: more volumes than MAX_VOLS (in a scrambled order): the nearest MAX_VOLS are kept, still far to near; the far ones go.
	#[test]
	fn over_the_cap_the_nearest_volumes_are_kept() {
		let n = MAX_VOLS + 150;
		let mut v: Vec<Vol> = (0..n).map(|i| (i * 7919) % n).map(|d| flame([d as f32 + 1.0, 0.0, 0.0], 0.1, 0.5, 0.1, 1.0, [1.0; 3])).collect();
		nearest(&mut v, [0.0; 3]);
		let xs: Vec<f32> = v.iter().map(|x| x.a[0]).collect();
		assert_eq!(xs.len(), MAX_VOLS);
		assert_eq!((xs[0], xs[MAX_VOLS - 1]), (MAX_VOLS as f32, 1.0), "the farthest kept is the MAX_VOLS-th, the last drawn the nearest");
		assert!(xs.windows(2).all(|w| w[0] > w[1]), "far to near");
		let mut under: Vec<Vol> = v[..10].to_vec();
		nearest(&mut under, [0.0; 3]);
		assert_eq!(under.len(), 10, "under the cap nothing is dropped");
	}

	#[test]
	fn shapes_and_quality() {
		let f = flame([1.0, 2.0, 3.0], 4.0, 0.8, 0.5, 2.0, [1.0, 0.5, 0.2]);
		assert_eq!((f.b[3], f.b[2]), (0.0, 1.0), "a flame is kind 0, its fade clamped");
		assert_eq!(centre(&f), [1.0, 2.0, 3.0 + 1.8]);
		let b = ball([1.0, 2.0, 3.0], 1.5, 0.5, 0.4, [1.0; 3]);
		assert_eq!((b.b[3], centre(&b)), (1.0, [1.0, 2.0, 3.0]));
		assert_eq!(quality(1), (12.0, 1.0));
		assert_eq!(quality(2), (24.0, 2.0));
		assert_eq!(quality(3), (40.0, 3.0));
		assert!(rows(3, 1.0, [0.0; 3], [1.0, 0.0, 0.0], 0.5)[2][0] == 40.0);
		// A jet slab: kind 2, its box and its core in the middle of its axis.
		let j = Vol { a: [0.0, 0.0, 1.0, 0.2], b: [4.0, 0.3, 1.0, KIND_JET], d: [2.0, 0.0, 1.0, 0.6], ..f };
		assert!(is_jet(&j) && !is_ball(&j) && is_ball(&b) && !is_jet(&f) && !is_ball(&f));
		assert_eq!(centre(&j), [1.0, 0.0, 1.0]);
		assert!((core(&j).1 - 0.24).abs() < 1e-6 && (core(&j).2 - 0.24).abs() < 1e-6);
		// Cores: a flame's sits low and narrower than its foot, a fireball's shrinks as it dies.
		let (c, r, h) = core(&f);
		assert!((c[2] - (3.0 + 0.3 * 4.0)).abs() < 1e-5 && (r - 0.7 * f.b[0]).abs() < 1e-5 && (h - 1.4).abs() < 1e-5);
		let dying = ball([0.0; 3], 1.5, 0.5, 0.0, [1.0; 3]);
		assert!(core(&b).1 > core(&dying).1 && core(&dying).1 > 0.0);
	}

	#[test]
	fn the_noise_tiles_and_spans_its_range() {
		let n = 8;
		let d = noise_bytes(n);
		assert_eq!(d.len(), (n * n * n * 4) as usize);
		let r: Vec<u8> = d.chunks(4).map(|c| c[0]).collect();
		let (lo, hi) = (*r.iter().min().unwrap(), *r.iter().max().unwrap());
		assert!(hi - lo > 60, "the field has contrast ({lo}..{hi})");
	}

	#[test]
	fn a_noise_mip_is_the_mean_of_its_blocks() {
		let n = 4;
		let d: Vec<u8> = (0..n * n * n * 4).map(|i| (i % 251) as u8).collect();
		let h = noise_half(&d, n);
		assert_eq!(h.len(), 2 * 2 * 2 * 4);
		// Texel (1, 0, 1) channel 2: the block x 2..3, y 0..1, z 2..3.
		let at = |x: u32, y: u32, z: u32| d[(((z * n + y) * n + x) * 4 + 2) as usize] as u32;
		let s: u32 = (0..8).map(|k| at(2 + (k & 1), (k >> 1) & 1, 2 + (k >> 2))).sum();
		assert_eq!(h[5 * 4 + 2] as u32, (s + 4) / 8);
		// A flat field stays flat: the mean level does not drift.
		assert!(noise_half(&vec![77u8; 8 * 8 * 8 * 4], 8).iter().all(|&v| v == 77));
	}
}
