//! [PC] Starfield (Enhanced only; no retail counterpart). The retail sky draws its stars as single texels of one 256 x 256 sky texture
//! (alpha = brightness), stretched over the sky shell: flat, blurry squares. In Enhanced the level load finds the star texels of every
//! sky texture (`detect`: isolated bright points on a dark ground, in a texture that is mostly dark ground; planets, nebulae and
//! painted clouds are not star textures and stay as they are), takes them out of a copy of the texture (the "dead" texture) and keeps
//! one mask texel per star (position inside the 3x3, tint, energy). The shell draw of such a texture then runs `fs_star`
//! (starfield.wgsl) instead of `fs_sky`: the dead texture as the shell blends it, plus every star in reach redrawn as a small glowing
//! point with a bright core and a faint halo, sized in screen pixels (crisp at any resolution, scaled by height / 1080), integrated
//! over the pixel so a star that moves with the camera keeps its energy (no crawl). A fifth of the stars dip slowly now and then
//! (periods 3 to 9 s, dip depth `twinkle`); on the space levels (Oltanis orbit, Veldin orbit) the dip is softer (no air up there:
//! a slow variable-star pulse). The draw order, the blend of the dead texture, the shell geometry and the volumetric-cloud skip are
//! the retail sky's; Classic runs none of this (the build only reads the sky textures, the draw checks Enhanced). Levels with a star texture: Veldin, Aridia, Rilgar, Blarg, Umbris,
//! Gaspar, Oltanis orbit (Gemlik), Veldin orbit, Veldin finale.
//!
//! Hooks: `build_scene` (`starfield::build`, field `Scene::stars`), `Gpu::pass_sky` (`star_draw` per shell draw).

use super::*;
use std::sync::OnceLock;

/// The star texels of a texture: the texture without them and the per-star mask.
pub(super) struct StarSet {
	pub dead: Texture,
	pub mask: Texture,
	pub count: usize,
}

/// One star texture of the level: the sky draw slot it belongs to and its pictures.
struct Item {
	slot: usize,
	set: StarSet,
}

/// The scene's star textures (CPU side) and their GPU objects, made on the first Enhanced frame.
pub(in crate::viewer) struct StarMaps {
	items: Vec<Item>,
	gpu: OnceLock<StarGpu>,
}

struct StarGpu {
	pipeline: wgpu::RenderPipeline,
	ubuf: wgpu::Buffer,
	groups: Vec<(wgpu::BindGroup, wgpu::BindGroup)>,
}

// Detection tuning (premultiplied energy P = max(r, g, b) * alpha, 0..1).
const MIN_PEAK: f32 = 0.03;
const MAX_LIT_FRACTION: f32 = 0.12;
const MIN_STARS: usize = 80;
const MIN_EXPLAINED: f32 = 0.35;

fn energy(t: &Texture, x: i64, y: i64) -> f32 {
	let (x, y) = (x.clamp(0, t.w as i64 - 1) as usize, y.clamp(0, t.h as i64 - 1) as usize);
	let p = t.rgba[y * t.w + x];
	p[0].max(p[1]).max(p[2]) as f32 / 255.0 * (p[3] as f32 / 255.0)
}

/// The 24 texels around the 7 x 7 square of (x, y).
fn ring(x: i64, y: i64) -> impl Iterator<Item = (i64, i64)> {
	(-3..=3i64).flat_map(move |dy| (-3..=3i64).map(move |dx| (dx, dy))).filter(|&(dx, dy)| dx.abs().max(dy.abs()) == 3).map(move |(dx, dy)| (x + dx, y + dy))
}

/// Is (x, y) a star: the peak of its 3 x 3 (a tie goes to the first in scan order), `MIN_PEAK` above the ground (the median of the ring),
/// the ring's brightest at most half way up, few lit texels around, and not a bright ground. Returns the ground energy.
fn star_at(t: &Texture, x: i64, y: i64) -> Option<f32> {
	let p = energy(t, x, y);
	if p < MIN_PEAK {
		return None;
	}
	let mut sum8 = 0.0;
	for (dx, dy) in [(-1i64, -1i64), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
		let n = energy(t, x + dx, y + dy);
		let earlier = dy < 0 || (dy == 0 && dx < 0);
		if n > p || (earlier && n == p) {
			return None;
		}
		sum8 += n;
	}
	let mut r: Vec<f32> = ring(x, y).map(|(a, b)| energy(t, a, b)).collect();
	r.sort_by(|a, b| a.total_cmp(b));
	let (bg, rmax) = ((r[11] + r[12]) * 0.5, r[23]);
	if bg > 0.12 || p - bg < MIN_PEAK || sum8 / 8.0 >= 0.6 * p + bg || rmax > bg + 0.5 * (p - bg) {
		return None;
	}
	let lit = (-2..=2i64).flat_map(|dy| (-2..=2i64).map(move |dx| (dx, dy))).filter(|&(dx, dy)| energy(t, x + dx, y + dy) - bg > MIN_PEAK * 0.5).count();
	(lit <= 6).then_some(bg)
}

/// The stars of `t`, or None when `t` is not a star texture.
pub(super) fn detect(t: &Texture) -> Option<StarSet> {
	let n = t.w * t.h;
	if t.w < 64 || t.h < 64 || n > 1 << 20 || t.rgba.len() != n {
		return None;
	}
	let lit_all = (0..n).filter(|&i| energy(t, (i % t.w) as i64, (i / t.w) as i64) > 0.015).count();
	if lit_all as f32 > MAX_LIT_FRACTION * n as f32 {
		return None;
	}
	let mut stars = Vec::new();
	for y in 0..t.h as i64 {
		for x in 0..t.w as i64 {
			if let Some(bg) = star_at(t, x, y) {
				stars.push((x, y, bg));
			}
		}
	}
	let mut dead = t.rgba.clone();
	let mut mask = vec![[0u8; 4]; n];
	let mut explained = vec![false; n];
	for &(x, y, bg) in &stars {
		// The replacement texel: the darkest of the ring.
		let at = |(a, b): (i64, i64)| (a.clamp(0, t.w as i64 - 1), b.clamp(0, t.h as i64 - 1));
		let darkest = ring(x, y).map(at).min_by(|a, b| energy(t, a.0, a.1).total_cmp(&energy(t, b.0, b.1)));
		let fill = darkest.map_or([0; 4], |(a, b)| t.rgba[b as usize * t.w + a as usize]);
		let (mut flux, mut cx, mut cy, mut rgb) = (0.0f32, 0.0f32, 0.0f32, [0.0f32; 3]);
		for dy in -1..=1i64 {
			for dx in -1..=1i64 {
				let (a, b) = (x + dx, y + dy);
				if a < 0 || b < 0 || a >= t.w as i64 || b >= t.h as i64 {
					continue;
				}
				let i = b as usize * t.w + a as usize;
				let e = (energy(t, a, b) - bg).max(0.0);
				flux += e;
				cx += e * dx as f32;
				cy += e * dy as f32;
				for k in 0..3 {
					rgb[k] += e * t.rgba[i][k] as f32;
				}
				explained[i] = true;
				if e > MIN_PEAK * 0.25 {
					dead[i] = fill;
				}
			}
		}
		if flux <= 0.0 {
			continue;
		}
		let tn = ((rgb[2] - rgb[0]) / rgb[0].max(rgb[1]).max(rgb[2]).max(1.0)).clamp(-1.0, 1.0);
		let enc = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
		mask[y as usize * t.w + x as usize] = [enc(cx / flux * 0.5 + 0.5), enc(cy / flux * 0.5 + 0.5), enc(tn * 0.5 + 0.5), enc(flux.min(1.0).sqrt()).max(1)];
	}
	let lit_expl = (0..n).filter(|&i| explained[i] && energy(t, (i % t.w) as i64, (i / t.w) as i64) > 0.015).count();
	let count = mask.iter().filter(|m| m[3] > 0).count();
	if count < MIN_STARS || (lit_expl as f32) < MIN_EXPLAINED * lit_all.max(1) as f32 {
		return None;
	}
	Some(StarSet { dead: Texture { w: t.w, h: t.h, rgba: dead }, mask: Texture { w: t.w, h: t.h, rgba: mask }, count })
}

/// The scene build's hook: the star textures of `sky` and the draw slots of their shell draws. None with RAC_STARS=0 or when no sky texture is a
/// star texture (Enhanced is checked at the draw: the scene is built before the options are known).
pub(in crate::viewer) fn build(sky: Option<&crate::sky::Sky>, slots: &HashMap<u32, usize>) -> Option<StarMaps> {
	dbg_dump(sky);
	let sky = sky?;
	if std::env::var("RAC_STARS").is_ok_and(|v| v == "0") {
		return None;
	}
	let mask = crate::mobies::KEY_MASK | 3 << crate::shrubs::WRAP_SHIFT;
	let mut keys: Vec<&u32> = sky.textures.keys().collect();
	keys.sort();
	let items: Vec<Item> = keys
		.into_iter()
		.filter_map(|k| {
			let slot = *slots.get(&((k | 3 << crate::shrubs::WRAP_SHIFT) & mask))?;
			let set = detect(&sky.textures[k])?;
			eprintln!("starfield: texture {k:#x} slot {slot}: {} stars", set.count);
			Some(Item { slot, set })
		})
		.collect();
	(!items.is_empty()).then(|| StarMaps { items, gpu: OnceLock::new() })
}

/// Dev only (RAC_STAR_DBG=<dir>): every sky texture of the level as a PNG (colour and alpha), one stderr line per texture.
pub(in crate::viewer) fn dbg_dump(sky: Option<&crate::sky::Sky>) {
	let (Some(dir), Some(sky)) = (std::env::var_os("RAC_STAR_DBG"), sky) else { return };
	let _ = std::fs::create_dir_all(&dir);
	for (k, t) in &sky.textures {
		let rgb: Vec<u8> = t.rgba.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
		let path = std::path::Path::new(&dir).join(format!("tex_{:x}_{}x{}.png", k, t.w, t.h));
		let _ = std::fs::write(&path, crate::render::png(&crate::render::Image { w: t.w, h: t.h, rgb }));
		let a: Vec<u8> = t.rgba.iter().flat_map(|p| [p[3]; 3]).collect();
		let _ = std::fs::write(path.with_extension("a.png"), crate::render::png(&crate::render::Image { w: t.w, h: t.h, rgb: a }));
		eprintln!("star dbg: texture {:#x} {}x{}", k, t.w, t.h);
	}
}

/// The twinkle dip depth of a level: the planets with air dip fully, the space levels (no air) softly.
fn twinkle(level: usize) -> f32 {
	if matches!(level, 13 | 17) { 0.2 } else { 0.35 }
}

const WGSL: &str = include_str!("starfield.wgsl");

/// How far (texels) a star's light reaches for the list: the centroid offset (1) + the halo.
const LIST_REACH: i64 = 3;

/// The per-texel list of the two nearest stars of `mask` (a star = a texel with alpha > 0) within `LIST_REACH`, as u16 (x + 1, y + 1) pairs; 0 = none.
fn star_list(mask: &Texture) -> Vec<[u16; 4]> {
	let mut best = vec![[(i64::MAX, 0u16, 0u16); 2]; mask.w * mask.h];
	for y in 0..mask.h as i64 {
		for x in 0..mask.w as i64 {
			if mask.rgba[y as usize * mask.w + x as usize][3] == 0 {
				continue;
			}
			for dy in -LIST_REACH..=LIST_REACH {
				for dx in -LIST_REACH..=LIST_REACH {
					let (u, v) = (x + dx, y + dy);
					if u < 0 || v < 0 || u >= mask.w as i64 || v >= mask.h as i64 {
						continue;
					}
					let slot = &mut best[v as usize * mask.w + u as usize];
					let cand = (dx * dx + dy * dy, x as u16 + 1, y as u16 + 1);
					if cand.0 < slot[0].0 {
						slot[1] = slot[0];
						slot[0] = cand;
					} else if cand.0 < slot[1].0 {
						slot[1] = cand;
					}
				}
			}
		}
	}
	best.into_iter().map(|b| [b[0].1, b[0].2, b[1].1, b[1].2]).collect()
}

impl StarGpu {
	fn new(gpu: &Gpu, maps: &StarMaps) -> StarGpu {
		let device = &gpu.device;
		let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None };
		let layout2 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("starfield"),
			entries: &[
				entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
				entry(1, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
				entry(2, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
			],
		});
		let layout0 = gpu.pipeline.get_bind_group_layout(0);
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("starfield"), bind_group_layouts: &[Some(&layout0), Some(&gpu.tex_layout), Some(&layout2)], immediate_size: 0 });
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("starfield"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("starfield", WGSL.into())) });
		let premult = wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent::OVER };
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("starfield"),
			layout: Some(&pl),
			vertex: wgpu::VertexState {
				module: &module,
				entry_point: Some("vs_star"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4] })],
			},
			fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs_star"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: gpu.config.format, blend: Some(premult), write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(),
			depth_stencil: Some(wgpu::DepthStencilState { format: gpu.depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
			multisample: wgpu::MultisampleState { count: super::targets::MSAA, ..Default::default() },
			multiview_mask: None,
			cache: None,
		});
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("starfield"), size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let upload = |t: &Texture| {
			let size = wgpu::Extent3d { width: t.w as u32, height: t.h as u32, depth_or_array_layers: 1 };
			let tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("starfield"), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
			gpu.queue.write_texture(tex.as_image_copy(), bytemuck::cast_slice(&t.rgba), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(t.w as u32 * 4), rows_per_image: Some(t.h as u32) }, size);
			tex.create_view(&Default::default())
		};
		// The star list: per texel the (x + 1, y + 1) of the two nearest stars within `LIST_REACH` texels (0 = none), so a pixel reads 1 + 2 texels.
		let upload_list = |t: &Texture| {
			let size = wgpu::Extent3d { width: t.w as u32, height: t.h as u32, depth_or_array_layers: 1 };
			let tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("starfield list"), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba16Uint, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
			gpu.queue.write_texture(tex.as_image_copy(), bytemuck::cast_slice(&star_list(t)), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(t.w as u32 * 8), rows_per_image: Some(t.h as u32) }, size);
			tex.create_view(&Default::default())
		};
		let groups = maps
			.items
			.iter()
			.map(|it| {
				// The shell sampler of the retail sky draws (CLAMP_1 = 5: both axes clamped), no mips, as the original slot.
				let dead = device.create_bind_group(&wgpu::BindGroupDescriptor {
					label: Some("starfield dead"),
					layout: &gpu.tex_layout,
					entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&upload(&it.set.dead)) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&gpu.wrap_samplers[2]) }],
				});
				let mask = device.create_bind_group(&wgpu::BindGroupDescriptor {
					label: Some("starfield mask"),
					layout: &layout2,
					entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&upload(&it.set.mask)) }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&upload_list(&it.set.mask)) }],
				});
				(dead, mask)
			})
			.collect();
		StarGpu { pipeline, ubuf, groups }
	}
}

impl Gpu {
	/// Draws the sky shell draw `d` with the starfield look when its texture is a star texture and Enhanced is on; true = drawn
	/// (the sky pipeline is set again for the next draw).
	pub(in crate::viewer) fn star_draw(&self, pass: &mut wgpu::RenderPass<'_>, scene: &Scene, d: &Draw) -> bool {
		let Some(maps) = scene.stars.as_ref().filter(|_| crate::retail::splitview::fx(crate::retail::pcext::graphics().enhanced())) else { return false };
		let Some(k) = maps.items.iter().position(|it| it.slot == d.texture) else { return false };
		let st = maps.gpu.get_or_init(|| StarGpu::new(self, maps));
		let t = super::test_clock::secs(|| {
			static T0: OnceLock<std::time::Instant> = OnceLock::new();
			T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f32()
		}) % 3600.0;
		let tune = |n: &str, v: f32| crate::devtune::f32(n, v);
		let vals: [f32; 8] = [t, twinkle(self.level) * tune("star_twinkle", 1.0), (self.config.height as f32 / 1080.0).max(0.5), tune("star_gain", 1.0), tune("star_halo", 1.0), 0.0, 0.0, 0.0];
		self.queue.write_buffer(&st.ubuf, 0, bytemuck::cast_slice(&vals));
		let (dead, mask) = &st.groups[k];
		pass.set_pipeline(&st.pipeline);
		pass.set_bind_group(1, dead, &[]);
		pass.set_bind_group(2, mask, &[]);
		pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
		pass.set_pipeline(&self.sky_pipeline);
		true
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A star texture as the retail one: dark ground (alpha 0) with isolated points, a few of them two texels wide.
	fn field() -> Texture {
		let (w, h) = (128usize, 128usize);
		let mut rgba = vec![[5u8, 5, 5, 0]; w * h];
		for k in 0..200usize {
			let (x, y) = ((k * 37 + 11) % 120 + 4, (k * 53 + 7) % 120 + 4);
			rgba[y * w + x] = [200, 210, 230, 120 + (k % 100) as u8];
			if k % 5 == 0 {
				rgba[y * w + x + 1] = [90, 100, 120, 60];
			}
		}
		Texture { w, h, rgba }
	}

	#[test]
	fn finds_isolated_points_and_empties_them_from_the_dead_texture() {
		let t = field();
		let set = detect(&t).expect("a star field");
		assert!(set.count >= 110, "found {}", set.count);
		let lit = |t: &Texture| t.rgba.iter().filter(|p| p[3] > 0).count();
		assert!(lit(&set.dead) < lit(&t) / 8, "dead texture still holds {} of {} lit texels", lit(&set.dead), lit(&t));
	}

	#[test]
	fn a_painted_texture_is_not_a_star_field() {
		let (w, h) = (128usize, 128usize);
		let rgba: Vec<[u8; 4]> = (0..w * h).map(|i| { let (x, y) = (i % w, i / w); let v = (128.0 + 100.0 * ((x as f32 * 0.2).sin() * (y as f32 * 0.17).cos())) as u8; [v, v / 2, 200, 255] }).collect();
		assert!(detect(&Texture { w, h, rgba }).is_none());
		let mut sparse = vec![[0u8, 0, 0, 0]; w * h];
		sparse[1000] = [255, 255, 255, 255];
		assert!(detect(&Texture { w, h, rgba: sparse }).is_none(), "one point is no star field");
	}

	#[test]
	fn the_shader_parses_and_validates() {
		let module = naga::front::wgsl::parse_str(WGSL).expect("starfield.wgsl parses");
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).expect("starfield.wgsl validates");
	}
}
