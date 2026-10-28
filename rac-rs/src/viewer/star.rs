//! [PC] Enhanced: the big orange star of Gemlik Base's sky, alive (user 2026-10-25).
//!
//! Retail paints the star as one 256x256 texture (sky shell 2, face key 0x40005, texture hash 1885e61187b6ebe4, only level 13)
//! on a curved patch whose uv (0.5, 0.5) is the disc centre; the disc has radius 0.60 in `p = (uv - 0.5) * 2` and a red halo
//! fades out by 0.90. With Lighting Enhanced the patch's draw takes `fs_star` instead of `fs_sky`: the same patch, same depth
//! test and draw order (towers and asteroids in front cover it exactly as before), but the star is alive: a slowly rotating sphere
//! of boiling granulation (three scales of animated cells, dark lanes, veins, faculae, rare sunspots; the retail texture itself is
//! the flow-warped detail layer), a corona built on the retail halo with drifting streamers, and eruptions at the limb: 8 slots
//! each run one event per 20..36 s window, the event index seeding its kind (prominence loop or plasma burst), place on the limb,
//! size, shape, start and life (5..30 s), so 2-5 are visible at once and none repeats. One clock drives it all
//! (`test_clock::secs`). Classic never gets here. `RAC_TUNE=star_q=0` turns it off.

use super::*;
use std::cell::{Cell, OnceCell};

/// The retail face key of the star patch (sky shell 2), with the sky's CLAMP wrap bits as `scene_build` stores it.
const KEY: u32 = 0x40005 | 3 << crate::shrubs::WRAP_SHIFT;
/// The level whose sky holds the star.
const LEVEL: usize = 13;

struct Res {
	pipeline: wgpu::RenderPipeline,
	params: wgpu::Buffer,
	group: wgpu::BindGroup,
}

/// The star's per-frame state: its GPU objects (made at the first frame it is on) and this frame's texture slot.
#[derive(Default)]
pub(in crate::viewer) struct StarFx {
	res: OnceCell<Res>,
	/// This frame: the scene texture slot of the star patch while the animated star is on.
	slot: Cell<Option<usize>>,
}

impl StarFx {
	/// Sets the star's pipeline and parameters when draw texture `texture` is the star patch this frame; true then (the caller
	/// puts the sky pipeline back after the draw).
	pub(in crate::viewer) fn begin(&self, pass: &mut wgpu::RenderPass<'_>, texture: usize) -> bool {
		match (self.slot.get(), self.res.get()) {
			(Some(s), Some(r)) if s == texture && crate::retail::splitview::fx(true) => {
				pass.set_pipeline(&r.pipeline);
				pass.set_bind_group(2, &r.group, &[]);
				true
			}
			_ => false,
		}
	}
}

/// Seconds of the star's clock (wall time, or the test clock's), wrapped at 10000 s (one jump every 2.8 hours). RAC_TUNE
/// star_ts=N runs the star N times faster, star_t0=S starts it S seconds in (tests).
fn seconds() -> f32 {
	static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
	crate::viewer::test_clock::secs(|| (T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() % 10000.0) as f32) * crate::devtune::f32("star_ts", 1.0) + crate::devtune::f32("star_t0", 0.0)
}

impl Gpu {
	/// Every pass before the level pass (`draw_passes`, also the split view's Classic pass): decides whether the animated star is on and writes its clock.
	pub(in crate::viewer) fn star_frame(&self, scene: &Scene, gfx: &crate::retail::pcext::Graphics) {
		let sf = &self.volclouds.sky.star;
		let q = crate::devtune::f32("star_q", 1.0);
		let slot = (gfx.enhanced() && self.modern_ok && !crate::viewer::pipe_guard::off(crate::viewer::pipe_guard::Effect::Post) && self.level == LEVEL && !scene.warp_shown && !self.ship_fight && q > 0.0)
			.then(|| scene.tex_slots.get(&KEY).copied())
			.flatten()
			.filter(|_| scene.sky.is_some());
		sf.slot.set(slot);
		if slot.is_some() {
			// [PC] pipe_guard.rs: a driver that refuses the star shader (DX12 FXC) keeps the plain sky texture, it does not crash the game.
			let built = || crate::viewer::pipe_guard::guard(&self.device, crate::viewer::pipe_guard::Effect::Star, || self.star_res()).map(|r| sf.res.get_or_init(|| r));
			let Some(r) = sf.res.get().or_else(built) else {
				sf.slot.set(None);
				return;
			};
			let v = [seconds(), q, 0.0, 0.0];
			self.queue.write_buffer(&r.params, 0, bytemuck::cast_slice(&v));
		}
	}

	fn star_res(&self) -> Res {
		let device = &self.device;
		let src = format!("{}{}", &crate::viewer::SHADER[..crate::viewer::SHADER.find("struct VOut").expect("struct VOut")], WGSL);
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("star"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("star", src.into())) });
		let params = device.create_buffer(&wgpu::BufferDescriptor { label: Some("star params"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let layout2 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("star"),
			entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
		});
		let group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("star"), layout: &layout2, entries: &[wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() }] });
		let l0 = self.sky_pipeline.get_bind_group_layout(0);
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("star"), bind_group_layouts: &[Some(&l0), Some(&self.tex_layout), Some(&layout2)], immediate_size: 0 });
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("star"),
			layout: Some(&layout),
			vertex: wgpu::VertexState {
				module: &module,
				entry_point: Some("vs_star"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout {
					array_stride: std::mem::size_of::<Vertex>() as u64,
					step_mode: wgpu::VertexStepMode::Vertex,
					attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4],
				})],
			},
			fragment: Some(wgpu::FragmentState {
				module: &module,
				entry_point: Some("fs_star"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState { format: self.config.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
			}),
			primitive: Default::default(),
			depth_stencil: Some(wgpu::DepthStencilState { format: self.depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
			multisample: wgpu::MultisampleState { count: crate::viewer::targets::MSAA, ..Default::default() },
			multiview_mask: None,
			cache: None,
		});
		Res { pipeline, params, group }
	}
}

/// The star's shaders, after the level shader's head (struct U, the sky texture at group 1) and the star's group 2.
pub(in crate::viewer) const WGSL: &str = include_str!("star.wgsl");

#[cfg(test)]
mod tests {
	use super::*;

	/// The star shader compiles (naga) after the level shader's head with both entry points.
	#[test]
	fn star_shader_parses() {
		let head = &crate::viewer::SHADER[..crate::viewer::SHADER.find("struct VOut").expect("struct VOut")];
		let src = format!("{head}{WGSL}");
		let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("star: {}", e.emit_to_string(&src)));
		let names: Vec<_> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
		assert!(names.contains(&"vs_star") && names.contains(&"fs_star"));
		let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
		v.validate(&m).unwrap_or_else(|e| panic!("star validates: {e:?}"));
	}
}
