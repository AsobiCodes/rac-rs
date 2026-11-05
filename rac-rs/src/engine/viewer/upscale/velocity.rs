//! [PC] Object motion of the upscaler inputs (lane ndlss stage 3; upscale.rs, RAC_UPSCALE_PREP). The camera pass (motion.rs) is
//! exact for everything static; what moves on its own gets its vectors here, in one pass right after it into the same motion
//! target: the props batch (`vs_props` posing: this frame's slot records and pose entries against a copy of last frame's, slot
//! index = identity; a slot that changed its kind or posed vertex count has no history) and the hero marker (world-space vertices
//! against a copy of last frame's). A fragment counts only where its depth matches the scene depth copy (the visible surface).
//! After the pass this frame's buffers are copied as next frame's history. Not covered (camera vectors / the reactive mask
//! instead): the CPU-clipped props, debris, effect quads, particles, water. Built only while RAC_UPSCALE_PREP is on.

use super::motion::{MotionGpu, MOTION_FORMAT};
use crate::viewer::Draw;

pub(super) const SHADER: &str = include_str!("velocity.wgsl");
/// `Vertex` (viewer.rs) stride; the position is its first field.
const VERTEX_STRIDE: u64 = 64;

/// The props batch: (model vertices, slot / vertex ids, indices, live draws, slot records, pose entries).
pub(in crate::viewer) type PropsIn<'a> = (&'a wgpu::Buffer, &'a wgpu::Buffer, &'a wgpu::Buffer, Vec<Draw>, &'a wgpu::Buffer, &'a wgpu::Buffer);
/// The hero marker: (world-space vertices, indices, draws).
pub(in crate::viewer) type HeroIn<'a> = (&'a wgpu::Buffer, &'a wgpu::Buffer, &'a [Draw]);

/// What the level pass drew this frame that moves on its own.
#[derive(Default)]
pub(in crate::viewer) struct Objects<'a> {
	pub props: Option<PropsIn<'a>>,
	pub hero: Option<HeroIn<'a>>,
}

impl<'a> Objects<'a> {
	/// The props and the hero as the level pass draws them (pass_world.rs: shown, no warp).
	pub(in crate::viewer) fn of(scene: &'a crate::viewer::Scene, pg: &'a crate::viewer::gpuskin::PropsGpu) -> Objects<'a> {
		let shown = |on: bool| on && !scene.warp_shown;
		let props = scene.props_sj.as_ref().filter(|_| shown(scene.props_shown)).map(|sj| {
			let (insts, pose) = pg.storage();
			(&scene.props.vbuf, sj, &scene.props.ibuf, scene.props_live_draws(|_| true), insts, pose)
		});
		let hero = shown(scene.marker_shown).then(|| (&scene.marker.vbuf, &scene.marker.ibuf, &scene.marker.draws[..]));
		Objects { props, hero }
	}
}

fn copyable(b: &wgpu::Buffer) -> bool {
	b.usage().contains(wgpu::BufferUsages::COPY_SRC)
}

/// A history buffer shaped like `src` (`usage` + COPY_DST).
fn history(device: &wgpu::Device, label: &str, src: &wgpu::Buffer, usage: wgpu::BufferUsages) -> wgpu::Buffer {
	device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: src.size(), usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
}

/// The history buffers for `cur` (made again when a size changed; then this frame's contents go in first: no object motion).
fn ensure(device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, slot: &mut Vec<wgpu::Buffer>, cur: &[&wgpu::Buffer], usage: wgpu::BufferUsages) {
	if slot.len() != cur.len() || slot.iter().zip(cur).any(|(h, c)| h.size() != c.size()) {
		*slot = cur.iter().map(|c| history(device, "upscale history", c, usage)).collect();
		for (h, c) in slot.iter().zip(cur) {
			enc.copy_buffer_to_buffer(c, 0, h, 0, c.size());
		}
	}
}

pub(in crate::viewer) struct VelocityGpu {
	l_frame: wgpu::BindGroupLayout,
	l_props: wgpu::BindGroupLayout,
	props_pipe: wgpu::RenderPipeline,
	hero_pipe: wgpu::RenderPipeline,
	/// Last frame's slot records and pose entries.
	prev_props: Vec<wgpu::Buffer>,
	/// Last frame's hero vertices.
	prev_hero: Vec<wgpu::Buffer>,
}

impl VelocityGpu {
	pub(super) fn new(device: &wgpu::Device) -> VelocityGpu {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("upscale objects"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("upscale objects", SHADER.into())) });
		let uniform = wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None };
		let depth = wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None };
		let l_frame = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("upscale objects"), entries: &[uniform, depth] });
		let storage = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None };
		let l_props = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("upscale props"), entries: &[storage(0), storage(1), storage(2), storage(3)] });
		let pipe = |entry: &'static str, layouts: &[Option<&wgpu::BindGroupLayout>], buffers: &[Option<wgpu::VertexBufferLayout>]| {
			let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(entry), bind_group_layouts: layouts, immediate_size: 0 });
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(entry),
				layout: Some(&pl),
				vertex: wgpu::VertexState { module: &shader, entry_point: Some(entry), compilation_options: Default::default(), buffers },
				fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_object"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: MOTION_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
				primitive: Default::default(),
				depth_stencil: None,
				multisample: Default::default(),
				multiview_mask: None,
				cache: None,
			})
		};
		let pos = |loc| wgpu::VertexBufferLayout { array_stride: VERTEX_STRIDE, step_mode: wgpu::VertexStepMode::Vertex, attributes: if loc == 0 { &wgpu::vertex_attr_array![0 => Float32x3] } else { &wgpu::vertex_attr_array![1 => Float32x3] } };
		let sj = wgpu::VertexBufferLayout { array_stride: 8, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![1 => Uint32x2] };
		let props_pipe = pipe("vs_props", &[Some(&l_frame), Some(&l_props)], &[Some(pos(0)), Some(sj)]);
		let hero_pipe = pipe("vs_hero", &[Some(&l_frame)], &[Some(pos(0)), Some(pos(1))]);
		VelocityGpu { l_frame, l_props, props_pipe, hero_pipe, prev_props: Vec::new(), prev_hero: Vec::new() }
	}

	/// After the camera pass of `mg`: the objects' vectors into its motion target, then this frame's buffers kept as history.
	pub(super) fn encode(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, mg: &MotionGpu, objs: &Objects) {
		let props = objs.props.as_ref().filter(|p| copyable(p.4) && copyable(p.5));
		let hero = objs.hero.filter(|h| copyable(h.0));
		match props {
			Some(p) => ensure(device, enc, &mut self.prev_props, &[p.4, p.5], wgpu::BufferUsages::STORAGE),
			None => self.prev_props.clear(),
		}
		match hero {
			Some(h) => ensure(device, enc, &mut self.prev_hero, &[h.0], wgpu::BufferUsages::VERTEX),
			None => self.prev_hero.clear(),
		}
		if props.is_none() && hero.is_none() {
			return;
		}
		let frame = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("upscale objects"),
			layout: &self.l_frame,
			entries: &[wgpu::BindGroupEntry { binding: 0, resource: mg.ubuf().as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(mg.depth_view()) }],
		});
		let props_bind = props.map(|p| {
			let bufs = [p.4, p.5, &self.prev_props[0], &self.prev_props[1]];
			let entries: Vec<wgpu::BindGroupEntry> = bufs.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() }).collect();
			device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("upscale props"), layout: &self.l_props, entries: &entries })
		});
		let load = Some(wgpu::RenderPassColorAttachment { view: mg.motion_view(), resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } });
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("upscale objects"), color_attachments: &[load], depth_stencil_attachment: None, timestamp_writes: crate::viewer::passtime::rw("upscale objects"), occlusion_query_set: None, multiview_mask: None });
		pass.set_bind_group(0, &frame, &[]);
		if let (Some(p), Some(bind)) = (props, &props_bind) {
			pass.set_pipeline(&self.props_pipe);
			pass.set_bind_group(1, bind, &[]);
			pass.set_vertex_buffer(0, p.0.slice(..));
			pass.set_vertex_buffer(1, p.1.slice(..));
			pass.set_index_buffer(p.2.slice(..), wgpu::IndexFormat::Uint32);
			for d in &p.3 {
				pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
			}
		}
		if let Some(h) = hero {
			pass.set_pipeline(&self.hero_pipe);
			pass.set_vertex_buffer(0, h.0.slice(..));
			pass.set_vertex_buffer(1, self.prev_hero[0].slice(..));
			pass.set_index_buffer(h.1.slice(..), wgpu::IndexFormat::Uint32);
			for d in h.2 {
				pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
			}
		}
		drop(pass);
		let keep = props.map(|p| [p.4, p.5]).into_iter().flatten().zip(self.prev_props.iter()).chain(hero.map(|h| h.0).into_iter().zip(self.prev_hero.iter()));
		for (cur, prev) in keep {
			enc.copy_buffer_to_buffer(cur, 0, prev, 0, cur.size());
		}
	}
}
