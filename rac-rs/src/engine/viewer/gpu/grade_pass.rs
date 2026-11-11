//! [PC] Brightness and Contrast on the GPU (the option and the curve: viewer/picture_adjust.rs). `grade_begin` latches the sliders
//! once per frame (draw_passes) as the shaders' uniform (`grade_k`), so every place that grades this frame sees the same choice even
//! when the menu moves the slider mid-frame. The curve is folded into passes that already write the picture where it can be:
//! the upscaler slot's copy (upscale/slot_gpu.rs) when the slot runs, else Lighting Enhanced's composite (post.rs). Only Classic
//! lighting without the slot needs this file's own pass: after the level pass (passes.rs; the level pass keeps its picture then),
//! before ReShade and the frame generation inputs, the MSAA colour target is resolved into a copy and a full-screen triangle writes
//! the curve of the copy back into every sample; the HUD pass then draws the HUD, the retail overlay and the item previews over it
//! unchanged. At 50 / 50 nothing here runs and the frame is the frame of before (no pass, no target, the same pipelines).
//! Split view: no `splitview::fx` gate on purpose (a picture setting for either Lighting, not an Enhanced effect; picture_adjust.rs).

use super::*;
use std::sync::Mutex;

const SHADER: &str = include_str!("grade_pass.wgsl");

/// The pass's shader: its own part and the shared curve.
fn shader() -> String {
	format!("{SHADER}
{}", crate::viewer::picture_adjust::CURVE_WGSL)
}

struct Pipe {
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	uni: wgpu::Buffer,
	/// The target's format and sample count the pipeline was made for.
	key: (wgpu::TextureFormat, u32),
}

/// The single-sample copy of the target, its view and bind group (remade when the target's size or format changes).
struct Copy {
	tex: wgpu::Texture,
	view: wgpu::TextureView,
	bind: wgpu::BindGroup,
}

#[derive(Default)]
struct Inner {
	/// This frame's curve uniform (None: 50 / 50, nothing grades).
	on: Option<[f32; 4]>,
	pipe: Option<Pipe>,
	copy: Option<Copy>,
	/// The pipe guard refused the pass once: Classic frames stay ungraded for the run.
	refused: bool,
}

/// The grade state on `RScale` (a lock: `hud_pass` has `&Gpu`).
#[derive(Default)]
pub(in crate::viewer) struct Grade {
	inner: Mutex<Inner>,
}

/// The HUD pass target kept (Load / Store), optionally resolved into `resolve`.
fn attach<'a>(view: &'a wgpu::TextureView, resolve: Option<&'a wgpu::TextureView>) -> wgpu::RenderPassColorAttachment<'a> {
	crate::viewer::scene_msaa::att(view, resolve, wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }) // lane noaa: 1 sample = copy
}

fn make_pipe(device: &wgpu::Device, key: (wgpu::TextureFormat, u32)) -> Pipe {
	let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
		label: Some("grade"),
		entries: &[
			wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
			wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
		],
	});
	let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("grade"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
	let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("grade"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("grade", shader().into())) });
	let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
		label: Some("grade"),
		layout: Some(&pl),
		vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_grade"), compilation_options: Default::default(), buffers: &[] },
		fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_grade"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: key.0, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
		primitive: Default::default(),
		depth_stencil: None,
		multisample: wgpu::MultisampleState { count: key.1, ..Default::default() },
		multiview_mask: None,
		cache: None,
	});
	let uni = device.create_buffer(&wgpu::BufferDescriptor { label: Some("grade"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
	Pipe { pipeline, layout, uni, key }
}

fn make_copy(device: &wgpu::Device, pipe: &Pipe, format: wgpu::TextureFormat, size: (u32, u32)) -> Copy {
	let tex = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("grade copy"),
		size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	});
	let view = tex.create_view(&Default::default());
	let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
		label: Some("grade"),
		layout: &pipe.layout,
		entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: pipe.uni.as_entire_binding() }],
	});
	Copy { tex, view, bind }
}

impl Gpu {
	/// Start of `draw_passes`: this frame's sliders latched (true = the picture is graded this frame).
	pub(super) fn grade_begin(&self) -> bool {
		let on = crate::viewer::picture_adjust::frame_k();
		self.rscale.grade.inner.lock().unwrap_or_else(|e| e.into_inner()).on = on;
		on.is_some()
	}

	/// This frame's curve uniform (None at 50 / 50): the upscaler slot's copy and the Enhanced composite fold it in.
	pub(in crate::viewer) fn grade_k(&self) -> Option<[f32; 4]> {
		self.rscale.grade.inner.lock().unwrap_or_else(|e| e.into_inner()).on
	}

	/// Classic lighting without the upscaler slot (passes.rs), before ReShade and the frame generation inputs: the picture in the
	/// MSAA colour target `target` graded in place. Nothing at 50 / 50; nothing when the pipe guard refused the pass.
	pub(super) fn grade_encode(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
		let mut g = self.rscale.grade.inner.lock().unwrap_or_else(|e| e.into_inner());
		let Some(k) = g.on else { return };
		let t = target.texture();
		let (size, format, samples) = ((t.width(), t.height()), t.format(), t.sample_count());
		if g.pipe.as_ref().is_none_or(|p| p.key != (format, samples)) {
			if g.refused {
				return;
			}
			g.pipe = crate::viewer::pipe_guard::guard(&self.device, crate::viewer::pipe_guard::Effect::Grade, || make_pipe(&self.device, (format, samples)));
			g.copy = None;
			g.refused = g.pipe.is_none();
			eprintln!("picture: Brightness / Contrast pass on ({size:?}, {samples} samples){}", if g.refused { " refused: the Classic picture stays as it is" } else { "" });
		}
		let Inner { pipe: Some(pipe), copy, .. } = &mut *g else { return };
		if copy.as_ref().is_none_or(|c| (c.tex.width(), c.tex.height()) != size) {
			*copy = Some(make_copy(&self.device, pipe, format, size));
		}
		let Some(copy) = copy.as_ref() else { return };
		self.queue.write_buffer(&pipe.uni, 0, bytemuck::cast_slice(&k));
		drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("grade resolve"), color_attachments: &[Some(attach(target, Some(&copy.view)))], depth_stencil_attachment: None, timestamp_writes: crate::viewer::passtime::rw("grade resolve"), occlusion_query_set: None, multiview_mask: None }));
		crate::viewer::scene_msaa::flush(enc); // lane noaa: the resolve copy at 1 sample
		let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("grade"), color_attachments: &[Some(attach(target, None))], depth_stencil_attachment: None, timestamp_writes: crate::viewer::passtime::rw("grade"), occlusion_query_set: None, multiview_mask: None });
		p.set_pipeline(&pipe.pipeline);
		p.set_bind_group(0, &copy.bind, &[]);
		p.draw(0..3, 0..1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn shader_validates() {
		let m = naga::front::wgsl::parse_str(&shader()).expect("wgsl parse");
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	}

	#[test]
	fn the_shader_curve_is_picture_adjusts() {
		// The same maths as `picture_adjust::curve` (gamma lift, then the S-curve about mid-grey), FXC-safe (pow of max(x, 0)).
		let c = crate::viewer::picture_adjust::CURVE_WGSL;
		assert!(c.contains("pow(max(c, vec3<f32>(0.0)), vec3<f32>(k.x))"));
		assert!(c.contains("0.5 * pow(max(2.0 * x, vec3<f32>(0.0)), vec3<f32>(k.y))"));
		assert!(c.contains("1.0 - 0.5 * pow(max(vec3<f32>(2.0) - 2.0 * x, vec3<f32>(0.0)), vec3<f32>(k.y))"));
		assert!(c.contains("select(hi, lo, x < vec3<f32>(0.5))"));
		assert!(SHADER.contains("grade_curve(c.rgb, u.k)"));
	}
}
