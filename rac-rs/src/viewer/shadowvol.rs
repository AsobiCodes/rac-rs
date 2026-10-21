//! BRIDGE-64a (decomp/notes/shadows.md "BRIDGE-64a port spec"): the moby shadow-volume pass. Retail counts the volumes in
//! the framebuffer alpha (clear 0x7f, front faces up, back faces down) and darkens every pixel whose count rose above the
//! start; the port counts in the stencil aspect of the main depth target (clear 128, increment / decrement clamped, depth
//! test less-or-equal, no depth or colour write) and darkens where the stencil is above 128 (colour x 0.75, FIX 0x20).
//! The deferred-list props (the casters and the committed 0x800 props) are drawn after the darken, as retail draws its
//! deferred list after the volumes.

use super::*;
use crate::shadows::{self, VolumeCaster, VolumeSphere};

const SHADOW_SHADER: &str = r#"
struct U { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;

// The level shader's `world_clip` (viewer.rs): z rebuilt from w so the volumes test against the same precise depth.
fn world_clip(m: mat4x4<f32>, p: vec3<f32>) -> vec4<f32> {
	var c = m * vec4<f32>(p, 1.0);
	let wr = vec4<f32>(m[0][3], m[1][3], m[2][3], m[3][3]);
	let zr = vec4<f32>(m[0][2], m[1][2], m[2][2], m[3][2]);
	let ww = dot(wr.xyz, wr.xyz);
	if (ww > 1.0e-6) {
		let a = dot(zr.xyz, wr.xyz) / ww;
		let d = zr.xyz - a * wr.xyz;
		if (dot(d, d) <= 1.0e-10 * ww) {
			c.z = a * c.w + (zr.w - a * wr.w);
		}
	}
	return c;
}

@vertex fn vs_shadow(@location(0) p: vec3<f32>) -> @builtin(position) vec4<f32> {
	return world_clip(u.view_proj, p);
}

@fragment fn fs_shadow() -> @location(0) vec4<f32> {
	return vec4<f32>(0.0);
}

// One triangle covering the screen: (-1, -1), (3, -1), (-1, 3).
@vertex fn vs_shadow_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
	return vec4<f32>(f32((i << 1u) & 2u) * 2.0 - 1.0, f32(i & 2u) * 2.0 - 1.0, 0.0, 1.0);
}

// Blended SrcAlpha / OneMinusSrcAlpha: colour x 0.75.
@fragment fn fs_shadow_darken() -> @location(0) vec4<f32> {
	return vec4<f32>(0.0, 0.0, 0.0, 0.25);
}
"#;

/// The stencil value the level pass clears to (retail alpha 0x7f start, biased so both clamps keep 127 steps).
pub(super) const STENCIL_CLEAR: u32 = 128;

/// The increment, decrement and darken pipelines.
pub(super) struct ShadowPipes {
	inc: wgpu::RenderPipeline,
	dec: wgpu::RenderPipeline,
	darken: wgpu::RenderPipeline,
	/// M3a shadows: the hero (marker batch) is on retail's deferred draw path this frame: drawn after the volumes.
	pub(super) marker_late: bool,
}

impl ShadowPipes {
	pub(super) fn new(device: &wgpu::Device, ulayout: &wgpu::BindGroupLayout, colour: wgpu::TextureFormat, depth: wgpu::TextureFormat) -> ShadowPipes {
		crate::viewer::gs_state::guard_path("shadowvol");
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("shadow volumes"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("shadow volumes", SHADOW_SHADER.into())) });
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shadow volumes"), bind_group_layouts: &[Some(ulayout)], immediate_size: 0 });
		let make = |volume: bool, op: wgpu::StencilOperation, compare: wgpu::CompareFunction| {
			let face = wgpu::StencilFaceState { compare, fail_op: wgpu::StencilOperation::Keep, depth_fail_op: wgpu::StencilOperation::Keep, pass_op: op };
			let buffers = [Some(wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3] })];
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some("shadow volumes"),
				layout: Some(&layout),
				vertex: wgpu::VertexState {
					module: &shader,
					entry_point: Some(if volume { "vs_shadow" } else { "vs_shadow_full" }),
					compilation_options: Default::default(),
					buffers: if volume { &buffers } else { &[] },
				},
				fragment: Some(wgpu::FragmentState {
					module: &shader,
					entry_point: Some(if volume { "fs_shadow" } else { "fs_shadow_darken" }),
					compilation_options: Default::default(),
					targets: &[Some(wgpu::ColorTargetState {
						format: colour,
						blend: (!volume).then_some(wgpu::BlendState::ALPHA_BLENDING),
						write_mask: if volume { wgpu::ColorWrites::empty() } else { wgpu::ColorWrites::COLOR },
					})],
				}),
				// Both windings: the CPU sorts the faces into the increment and decrement lists (`shadows::volumes`).
				primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
				depth_stencil: Some(wgpu::DepthStencilState {
					format: depth,
					depth_write_enabled: Some(false),
					depth_compare: Some(if volume { wgpu::CompareFunction::LessEqual } else { wgpu::CompareFunction::Always }),
					stencil: wgpu::StencilState { front: face, back: face, read_mask: 0xff, write_mask: if volume { 0xff } else { 0 } },
					bias: Default::default(),
				}),
				multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
				multiview_mask: None,
				cache: None,
			})
		};
		ShadowPipes {
			inc: make(true, wgpu::StencilOperation::IncrementClamp, wgpu::CompareFunction::Always),
			dec: make(true, wgpu::StencilOperation::DecrementClamp, wgpu::CompareFunction::Always),
			// The test is `reference op stored`: reference 128 Less stored = the stored count is above 128.
			darken: make(false, wgpu::StencilOperation::Keep, wgpu::CompareFunction::Less),
			marker_late: false,
		}
	}

	/// The volume faces (`n.0` increment then `n.1` decrement vertices of `vb`) and the darken.
	pub(super) fn draw(&self, pass: &mut wgpu::RenderPass, vb: &wgpu::Buffer, n: (u32, u32)) {
		pass.set_vertex_buffer(0, vb.slice(..));
		pass.set_pipeline(&self.inc);
		pass.draw(0..n.0, 0..1);
		pass.set_pipeline(&self.dec);
		pass.draw(n.0..n.0 + n.1, 0..1);
		pass.set_pipeline(&self.darken);
		pass.set_stencil_reference(STENCIL_CLEAR);
		pass.draw(0..3, 0..1);
	}
}

/// Per slot of an unordered `Gpu::batch` over `meshes`, its faces' runs as (draw index, first index, index count): the
/// batch has one draw per texture key in ascending key order, and inside a key the faces of each mesh are contiguous and in
/// mesh order (the grouping `clip::replay_faces` replays).
pub(super) fn slot_runs(meshes: &[&Mesh]) -> Vec<Vec<(u32, u32, u32)>> {
	let mut count: BTreeMap<u32, u32> = BTreeMap::new();
	for m in meshes {
		for &(key, _) in &m.faces {
			*count.entry(key).or_default() += 3;
		}
	}
	let mut next: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
	let mut at = 0;
	for (d, (&key, &n)) in count.iter().enumerate() {
		next.insert(key, (d as u32, at));
		at += n;
	}
	meshes
		.iter()
		.map(|m| {
			let mut own: BTreeMap<u32, (u32, u32, u32)> = BTreeMap::new();
			for &(key, _) in &m.faces {
				let n = next.get_mut(&key).unwrap();
				own.entry(key).or_insert((n.0, n.1, 0)).2 += 3;
				n.1 += 3;
			}
			own.into_values().collect()
		})
		.collect()
}

/// The props batch's draws split into the main pass (every face but the late slots') and the late redraw (the late slots'
/// runs), each keeping its draw's texture, cull and instance word.
pub(super) fn split_late(draws: &[Draw], runs: &[Vec<(u32, u32, u32)>], late: &[usize]) -> (Vec<Draw>, Vec<Draw>) {
	let mut cut: Vec<(u32, u32, u32)> = late.iter().filter_map(|&s| runs.get(s)).flatten().copied().collect();
	cut.sort_unstable();
	let (mut main, mut tail) = (Vec::with_capacity(draws.len()), Vec::with_capacity(cut.len()));
	let mut k = 0;
	for (di, d) in draws.iter().enumerate() {
		let mut from = d.start;
		while k < cut.len() && cut[k].0 < di as u32 {
			k += 1;
		}
		while k < cut.len() && cut[k].0 == di as u32 {
			let (_, start, count) = cut[k];
			if start > from {
				main.push(Draw { start: from, count: start - from, ..*d });
			}
			if count > 0 {
				tail.push(Draw { start, count, ..*d });
			}
			from = from.max(start + count);
			k += 1;
		}
		if d.start + d.count > from {
			main.push(Draw { start: from, count: d.start + d.count - from, ..*d });
		}
	}
	(main, tail)
}

/// SCH-6b for one caster: the fade (0 = not in the list) and the straddle flag from the view-space depth of the draw
/// sphere (centre and radius in 1/1024 m).
pub(super) fn caster_fade(view: &super::clip::ClipView, dist: u8, centre: [f32; 3], w: f32) -> Option<(i32, bool)> {
	let d: [f32; 3] = std::array::from_fn(|k| centre[k] - view.e[k] * 1024.0);
	let z = view.f[0] * d[0] + view.f[1] * d[1] + view.f[2] * d[2];
	let diff = (z - w) as i32;
	// [I] the shadow-sphere frustum test of 0x26b2a8: the draw sphere's near edge within the shadow distance.
	if diff >= (dist as i32) << 10 {
		return None;
	}
	let fade = shadows::volume_fade(dist, diff);
	let v = view.view(centre.map(|c| c / 1024.0)).map(|c| c * 1024.0);
	// [I] straddle: the sphere is not wholly inside the view volume (the retail flag of the side test).
	let straddle = !view.sphere_wholly_inside(v, w);
	(fade != 0 || straddle).then_some((fade, straddle))
}

impl Gpu {
	/// BRIDGE-64a: this frame's shadow volumes. `casters` = (prop index, its sphere primitives in model space at class scale
	/// with the joint applied, band, shadow distance); `centres` / `radii` are the props' draw spheres (1/1024 m) as given
	/// to `set_props`, which must run first (it decides which slots are shown).
	pub(super) fn set_volumes(&mut self, props: &[Prop], casters: &[(usize, Vec<VolumeSphere>, [f32; 2], u8)], centres: &[[f32; 3]], radii: &[Option<f32>]) {
		self.volume_counts = (0, 0);
		self.shadow.marker_late = false;
		let Some(scene) = &mut self.scene else { return };
		scene.props_late.clear();
		let Some(view) = scene.clip_view else { return };
		// The hero's light word (BRIDGE-57) blends the two level lights' first direction; `volume_light` keeps its heading.
		let Some(lights) = self.lighting.as_ref().map(|l| &l.lights) else { return };
		let (a, b, t) = gameplay::light_word(self.hero_tint_out.map_or(self.hero_placed.1, |h| h.1));
		let (Some(la), Some(lb)) = (lights.get(a), lights.get(b)) else { return };
		let light = shadows::volume_light(std::array::from_fn(|k| la.dir[0][k] * (1.0 - t) + lb.dir[0][k] * t));
		let mut list = Vec::new();
		let mut late = Vec::new();
		for (pi, spheres, band, dist) in casters {
			let (pi, band, dist) = (*pi, *band, *dist);
			let Some(p) = props.get(pi) else { continue };
			if p.sched_flags() & 0x400 == 0 || band[1] <= 0.0 || spheres.is_empty() || !scene.props_vis.get(pi).copied().unwrap_or(false) {
				continue;
			}
			let (Some(&c), Some(w)) = (centres.get(pi), radii.get(pi).copied().flatten()) else { continue };
			let Some((fade, straddle)) = caster_fade(&view, dist, c, w) else { continue };
			let (m, scale) = super::smooth::prop_xf(pi).unwrap_or_else(|| (place_mirrored(p.pos, p.rot, p.mirror), p.scale));
			list.push(VolumeCaster { spheres: spheres.clone(), m, k: scale * fade as f32 / 4096.0, band, straddle });
			late.push(pi);
		}
		// The deferred list draws after the volumes: the casters and the shown 0x800 props.
		late.extend((0..props.len()).filter(|&pi| props[pi].sched_flags() & 0x800 != 0 && scene.props_vis.get(pi).copied().unwrap_or(false)));
		let band = view.band();
		let outside = |p: [f32; 3]| band.flags(view.view(p)) != 0;
		let vols = shadows::volumes(&list, light, view.e, &outside);
		let mut verts: Vec<[f32; 3]> = vols.iter().flat_map(|v| v.inc.iter().flatten().copied()).collect();
		let n_inc = verts.len() as u32;
		verts.extend(vols.iter().flat_map(|v| v.dec.iter().flatten().copied()));
		if verts.is_empty() {
			return;
		}
		scene.props_late = late;
		let bytes: &[u8] = bytemuck::cast_slice(&verts);
		if self.volume_buf.as_ref().is_none_or(|b| b.size() < bytes.len() as u64) {
			let size = (bytes.len() as u64).next_power_of_two().max(4096);
			self.volume_buf = Some(self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("shadow volumes"), size, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
		}
		self.queue.write_buffer(self.volume_buf.as_ref().unwrap(), 0, bytes);
		self.volume_counts = (n_inc, verts.len() as u32 - n_inc);
	}

	/// M3a shadows (retail mode): this frame's volumes from retail's own shadow records (`Retail::shadows`), replacing
	/// `set_volumes`; `late` = the props on retail's deferred draw path (+0x34 & 0xc00) and `hero_late` the hero's, drawn
	/// after the volumes and the darken.
	pub(super) fn set_volumes_retail(&mut self, casters: &[shadows::RecCaster], late: &[usize], hero_late: bool) {
		self.volume_counts = (0, 0);
		self.shadow.marker_late = false;
		let Some(scene) = &mut self.scene else { return };
		scene.props_late.clear();
		let Some(view) = scene.clip_view else { return };
		let band = view.band();
		let outside = |p: [f32; 3]| band.flags(view.view(p)) != 0;
		let vols = shadows::record_volumes(casters, view.e, &outside);
		let mut verts: Vec<[f32; 3]> = vols.iter().flat_map(|v| v.inc.iter().flatten().copied()).collect();
		let n_inc = verts.len() as u32;
		verts.extend(vols.iter().flat_map(|v| v.dec.iter().flatten().copied()));
		if verts.is_empty() {
			return;
		}
		scene.props_late = late.iter().copied().filter(|&pi| scene.props_vis.get(pi).copied().unwrap_or(false)).collect();
		self.shadow.marker_late = hero_late;
		let bytes: &[u8] = bytemuck::cast_slice(&verts);
		if self.volume_buf.as_ref().is_none_or(|b| b.size() < bytes.len() as u64) {
			let size = (bytes.len() as u64).next_power_of_two().max(4096);
			self.volume_buf = Some(self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("shadow volumes"), size, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
		}
		self.queue.write_buffer(self.volume_buf.as_ref().unwrap(), 0, bytes);
		self.volume_counts = (n_inc, verts.len() as u32 - n_inc);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn mesh(faces: &[u32]) -> Mesh {
		let mut m = Mesh::new(String::new());
		m.positions = vec![[0.0; 3]; 3];
		m.faces = faces.iter().map(|&k| (k, [0, 1, 2])).collect();
		m
	}

	#[test]
	fn shadow_shader_validates() {
		let m = naga::front::wgsl::parse_str(SHADOW_SHADER).expect("wgsl parse");
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	}

	/// Builds the pipelines on a real adapter and runs one stencil pass (wgpu validation panics on a mismatch). Needs a GPU:
	/// `cargo test shadow_pipes_on_adapter -- --ignored`.
	#[test]
	#[ignore]
	fn shadow_pipes_on_adapter() {
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
		let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("adapter");
		let stencil32 = adapter.features().contains(wgpu::Features::DEPTH32FLOAT_STENCIL8);
		let depth = if stencil32 { wgpu::TextureFormat::Depth32FloatStencil8 } else { wgpu::TextureFormat::Depth24PlusStencil8 };
		let desc = wgpu::DeviceDescriptor { required_features: if stencil32 { wgpu::Features::DEPTH32FLOAT_STENCIL8 } else { wgpu::Features::empty() }, ..Default::default() };
		let (device, queue) = pollster::block_on(adapter.request_device(&desc)).expect("device");
		let ulayout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: None,
			entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
		});
		let colour = wgpu::TextureFormat::Rgba8Unorm;
		let pipes = ShadowPipes::new(&device, &ulayout, colour, depth);
		let ident: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0];
		let ubuf = buffer(&device, "u", bytemuck::cast_slice(&ident), wgpu::BufferUsages::UNIFORM);
		queue.write_buffer(&ubuf, 0, bytemuck::cast_slice(&ident));
		let ubind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &ulayout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }] });
		let tri: [[f32; 3]; 6] = [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0], [-1.0, -1.0, 0.0], [0.0, 1.0, 0.0], [1.0, -1.0, 0.0]];
		let vb = buffer(&device, "v", bytemuck::cast_slice(&tri), wgpu::BufferUsages::VERTEX);
		queue.write_buffer(&vb, 0, bytemuck::cast_slice(&tri));
		let size = wgpu::Extent3d { width: 8, height: 8, depth_or_array_layers: 1 };
		let tex = |format| device.create_texture(&wgpu::TextureDescriptor { label: None, size, mip_level_count: 1, sample_count: MSAA, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] }).create_view(&Default::default());
		let (cv, dv) = (tex(colour), depth_view(&device, 8, 8, depth));
		let mut enc = device.create_command_encoder(&Default::default());
		{
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: None,
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &cv, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
					view: &dv,
					depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
					stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(STENCIL_CLEAR), store: wgpu::StoreOp::Discard }),
				}),
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_bind_group(0, &ubind, &[]);
			pipes.draw(&mut pass, &vb, (3, 3));
		}
		queue.submit([enc.finish()]);
		device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
	}

	#[test]
	fn slot_runs_follow_the_batch_grouping() {
		// Keys 1 and 5: draw 0 = key 1 (slot 0 two faces, slot 2 one face), draw 1 = key 5 (slot 1, slot 2).
		let (a, b, c) = (mesh(&[1, 1]), mesh(&[5]), mesh(&[5, 1]));
		let runs = slot_runs(&[&a, &b, &c]);
		assert_eq!(runs, vec![vec![(0, 0, 6)], vec![(1, 9, 3)], vec![(0, 6, 3), (1, 12, 3)]]);
		let draws = [Draw { inst: 0, cull: 0, texture: 7, start: 0, count: 9 }, Draw { inst: 0, cull: 1, texture: 8, start: 9, count: 6 }];
		let (main, late) = split_late(&draws, &runs, &[2]);
		let r = |v: &[Draw]| v.iter().map(|d| (d.texture, d.cull, d.start, d.count)).collect::<Vec<_>>();
		assert_eq!(r(&main), vec![(7, 0, 0, 6), (8, 1, 9, 3)]);
		assert_eq!(r(&late), vec![(7, 0, 6, 3), (8, 1, 12, 3)]);
		let (main, late) = split_late(&draws, &runs, &[]);
		assert_eq!((r(&main), late.len()), (r(&draws), 0));
	}

	#[test]
	fn caster_fade_near_far_and_out() {
		let view = super::super::clip::ClipView::new(&Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: 0.47627997, up: [0.0, 0.0, 1.0] }, 1.0);
		let r = 512.0;
		// 10 m ahead (z = 10240): diff 9728, (0x5800 - 9728) >> 1 = 6400 -> 0x1000; inside the view.
		assert_eq!(caster_fade(&view, 0x16, [10240.0, 0.0, 0.0], r), Some((0x1000, false)));
		// 20 m: diff 19968 -> (22528 - 19968) >> 1 = 1280.
		assert_eq!(caster_fade(&view, 0x16, [20480.0, 0.0, 0.0], r), Some((1280, false)));
		// Past the shadow distance: not in the list.
		assert_eq!(caster_fade(&view, 0x16, [30000.0, 0.0, 0.0], r), None);
		// Beside the camera: straddles.
		assert_eq!(caster_fade(&view, 0x16, [0.0, 0.0, 0.0], r).map(|f| f.1), Some(true));
	}
}
