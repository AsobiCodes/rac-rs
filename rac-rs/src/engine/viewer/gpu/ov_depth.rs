//! The GS overlay's effect depth (engine split stage E5, 2026-10-27): a game whose HUD overlay also carries the game's own
//! effects (R&C2: water, particles, drawn by the GS in overlay mode, engine/gsfull/gs_overlay.rs) gives each overlay pixel the
//! view depth of the effect that drew it (0 = HUD, always in front). The overlay quad (`RETAIL_OV_TEX`) is then drawn with that
//! depth against the world's depth buffer, so an effect behind a wall or a moby is left out. Same blend as the plain overlay
//! (premultiplied colour plus transmission). From lane rc2's window.rs `fs_overlay` (there a discard against the linearised
//! native depth, here the depth test).
//! Edges: the colour is a bilinear sample, so a pixel next to an effect texel takes some of its colour; it gets that effect's
//! depth (`fx_depth`). With the depth of its own texel only (a HUD or empty texel: in front), an effect behind the world
//! showed as thin lines along its lower and right borders [C: R&C2 level 12 field 4000, a world layer at 230 m behind a
//! cliff face at 24 m, debug picture 2026-10-27].

use super::*;

const SHADER: &str = "
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(1) @binding(0) var fxd: texture_2d<f32>;
@group(1) @binding(1) var<uniform> nf: vec4<f32>;
struct V { @builtin(position) p: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>) -> V {
	var o: V;
	o.p = vec4<f32>(p.xy, 0.0, 1.0);
	o.uv = uv;
	return o;
}
// The nearest effect depth among the 2 x 2 texels the colour's bilinear sample blends (0: none of them is an effect), so a
// pixel that takes some of an effect texel's colour is depth-tested as that effect.
fn fx_depth(uv: vec2<f32>, s: vec2<f32>) -> f32 {
	let t = floor(uv * s - vec2<f32>(0.5));
	var fd = 0.0;
	for (var k = 0; k < 4; k++) {
		let q = clamp(t + vec2<f32>(f32(k & 1), f32(k >> 1)), vec2<f32>(0.0), s - vec2<f32>(1.0));
		let z = textureLoad(fxd, vec2<i32>(q), 0).r;
		if (z > 0.0 && (fd == 0.0 || z < fd)) {
			fd = z;
		}
	}
	return fd;
}
struct F { @location(0) c: vec4<f32>, @builtin(frag_depth) d: f32 };
@fragment fn fs(v: V) -> F {
	let c = textureSample(tex, samp, v.uv);
	let s = vec2<f32>(textureDimensions(fxd));
	let fd = fx_depth(v.uv, s);
	var d = 0.0;
	if (fd > 0.0) {
		// A little in front of its own depth (the GS and this renderer round the same surface differently).
		let z = max(fd / 1.002 - 0.05, nf.x);
		d = clamp(nf.y * (z - nf.x) / (z * (nf.y - nf.x)), 0.0, 1.0);
	}
	return F(c, d);
}
";

pub(in crate::viewer) struct OvDepth {
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	/// (near, far, 0, 0) of this frame's projection.
	ubuf: wgpu::Buffer,
	/// The depth texture (width, rows).
	tex: Option<(u32, u32, wgpu::Texture)>,
	/// Group 1 of the overlay draw (`tex`, `ubuf`).
	bind: Option<wgpu::BindGroup>,
	/// The overlay of this frame has a depth (else the plain overlay draw).
	on: bool,
}

impl OvDepth {
	fn new(device: &wgpu::Device, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat, tex_layout: &wgpu::BindGroupLayout) -> OvDepth {
		let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None };
		let plain = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("overlay depth"),
			entries: &[entry(0, plain), entry(1, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None })],
		});
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("overlay depth"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("overlay depth"), bind_group_layouts: &[Some(tex_layout), Some(&layout)], immediate_size: 0 });
		// As hud_pipeline[3]: out = Cs + As * Cd.
		let blend = wgpu::BlendState {
			color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::SrcAlpha, operation: wgpu::BlendOperation::Add },
			alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
		};
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("overlay depth"),
			layout: Some(&pl),
			vertex: wgpu::VertexState {
				module: &module,
				entry_point: Some("vs"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2] })],
			},
			fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: wgpu::PrimitiveState::default(),
			depth_stencil: Some(wgpu::DepthStencilState { format: depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
			multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
			multiview_mask: None,
			cache: None,
		});
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("overlay depth range"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		OvDepth { pipeline, layout, ubuf, tex: None, bind: None, on: false }
	}
}

fn r32(device: &wgpu::Device, w: u32, rows: u32) -> wgpu::Texture {
	device.create_texture(&wgpu::TextureDescriptor {
		label: Some("overlay depth"),
		size: wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: wgpu::TextureFormat::R32Float,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	})
}

impl Gpu {
	/// The effect depth of the overlay `set_retail_overlay` set (`w` x `rows`, view depth per pixel, 0 = HUD), or None: the
	/// overlay is drawn over everything.
	pub fn set_retail_overlay_depth(&mut self, w: u32, rows: u32, depth: Option<&[f32]>) {
		let Some(d) = depth.filter(|d| d.len() == (w * rows) as usize) else {
			if let Some(o) = &mut self.ov_depth {
				o.on = false;
			}
			return;
		};
		let o = self.ov_depth.get_or_insert_with(|| OvDepth::new(&self.device, self.config.format, self.depth_format, &self.tex_layout));
		if !o.tex.as_ref().is_some_and(|t| t.0 == w && t.1 == rows) {
			o.tex = Some((w, rows, r32(&self.device, w, rows)));
			o.bind = None;
		}
		let t = &o.tex.as_ref().unwrap().2;
		if o.bind.is_none() {
			let view = t.create_view(&Default::default());
			let entries = [(0, wgpu::BindingResource::TextureView(&view)), (1, o.ubuf.as_entire_binding())];
			let entries = entries.map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource });
			o.bind = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("overlay depth"), layout: &o.layout, entries: &entries }));
		}
		self.queue.write_texture(t.as_image_copy(), bytemuck::cast_slice(d), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(rows) }, wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 });
		o.on = true;
	}

	/// The overlay quad's draw with the effect depth (pass_hud, `RETAIL_OV_TEX`): false when this frame's overlay has none. Leaves
	/// bind group 0 as the HUD draws had it.
	pub(super) fn draw_overlay_depth(&self, pass: &mut wgpu::RenderPass<'_>, ov: &wgpu::BindGroup, range: std::ops::Range<u32>) -> bool {
		let Some((o, b)) = self.ov_depth.as_ref().filter(|o| o.on).and_then(|o| Some((o, o.bind.as_ref()?))) else { return false };
		self.queue.write_buffer(&o.ubuf, 0, bytemuck::cast_slice(&[RETAIL_NEAR, self.world_far(), 0.0, 0.0]));
		pass.set_pipeline(&o.pipeline);
		pass.set_bind_group(0, ov, &[]);
		pass.set_bind_group(1, b, &[]);
		pass.draw_indexed(range, 0, 0..1);
		pass.set_bind_group(0, &self.ubind, &[]);
		true
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The shader compiles and validates (naga).
	#[test]
	fn shaders_validate() {
		for (name, src) in [("overlay depth", SHADER)] {
			let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
			let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
			v.validate(&m).unwrap_or_else(|e| panic!("{name} validates: {e:?}"));
		}
	}
}
