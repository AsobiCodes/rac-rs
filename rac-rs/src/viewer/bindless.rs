//! [PC] perf round 3 (no retail counterpart): the tie and shrub batches drawn without a texture bind group per draw. The
//! scene's textures sit in one binding array (`TEX_CAP` entries, the scene sampler) and every draw passes its texture slot
//! as its instance index (`vs` copies it into a flat varying the fragment stage indexes the array with). The draws, their
//! order, the vertex and fragment code and the texels sampled are the same as the per-draw bind group path; only the
//! per-draw bind group switch (the bulk of the command recording and submit cost of ~9k draws a frame) goes away.
//! Off with RAC_BINDLESS=0 or when the adapter lacks texture binding arrays with non-uniform indexing.

use super::*;

/// Binding array capacity (a scene with more texture slots draws with per-draw bind groups).
pub(super) const TEX_CAP: u32 = 4096;

/// The device features the path needs.
pub(super) fn features() -> wgpu::Features {
	wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
}

/// With INDIRECT_FIRST_INSTANCE as well, the draws of a list go as multi-draw-indirect runs (one call per run of the same
/// pipeline, same order).
pub(super) fn multi_wanted(adapter: &wgpu::Adapter) -> bool {
	adapter.features().contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
}

/// Whether to use the path on an adapter with these features and limits (RAC_BINDLESS=0 turns it off).
pub(super) fn wanted(adapter: &wgpu::Adapter) -> bool {
	!std::env::var("RAC_BINDLESS").is_ok_and(|v| v == "0") && adapter.features().contains(features()) && adapter.limits().max_binding_array_elements_per_shader_stage >= TEX_CAP
}

/// The level shader with the texture binding replaced by the array: `tex` becomes `texs[i.ti]`, `VOut` gains the flat slot
/// `ti`, and `vs` sets it from the instance index.
pub(super) fn shader_source(src: &str) -> String {
	let mut s = String::from("enable wgpu_binding_array;\n") + &src.replace("@group(1) @binding(0) var tex: texture_2d<f32>;", &format!("@group(1) @binding(0) var texs: binding_array<texture_2d<f32>, {TEX_CAP}>;"));
	s = s.replace("@location(7) @interpolate(flat) kmx: vec2<f32> };", "@location(7) @interpolate(flat) kmx: vec2<f32>, @location(8) @interpolate(flat) ti: u32 };");
	// Shrub meshes: the first instance's high half is the material's GS CLAMP wrap (`first_instance`); the array group has one
	// sampler per wrap. Repeat keeps the plain sample (uniform control flow); a clamp wrap samples with the same gradients.
	s = s.replace("@group(1) @binding(1) var samp: sampler;", "@group(1) @binding(1) var samp: sampler;\n@group(1) @binding(2) var samp_u: sampler;\n@group(1) @binding(3) var samp_v: sampler;\n@group(1) @binding(4) var samp_uv: sampler;");
	s = s.replace(
		"fn shrub_mesh_texel(i: VOut) -> vec4<f32> {\n\treturn textureSampleLevel(tex, samp, i.uv, gs_level(i));\n}",
		"fn shrub_mesh_texel(i: VOut) -> vec4<f32> {\n\tlet t = i.ti & 0xffffu;\n\tlet w = (i.ti >> 16u) & 3u;\n\tlet lv = gs_level(i);\n\tif (w == 1u) { return textureSampleLevel(texs[t], samp_u, i.uv, lv); }\n\tif (w == 2u) { return textureSampleLevel(texs[t], samp_v, i.uv, lv); }\n\tif (w == 3u) { return textureSampleLevel(texs[t], samp_uv, i.uv, lv); }\n\treturn textureSampleLevel(texs[t], samp, i.uv, lv);\n}",
	);
	// Ties: the same wrap samplers for the material's CLAMP wrap (`ties::tie_wrap`, the low bits of the word above the slot).
	s = s.replace(
		"\tif (i.l.w < -2.5 && i.l.w > -3.5 && i.kmx.y >= 0.0) {\n\t\treturn textureSampleLevel(tex, samp, i.uv, gs_level(i));\n\t}\n\treturn textureSampleGrad(tex, samp, i.uv, dx, dy);\n}",
		"\tlet t = i.ti & 0xffffu;\n\tlet w = (i.ti >> 16u) & 3u;\n\tif (i.l.w < -2.5 && i.l.w > -3.5 && i.kmx.y >= 0.0) {\n\t\tlet lv = gs_level(i);\n\t\tif (w == 1u) { return textureSampleLevel(texs[t], samp_u, i.uv, lv); }\n\t\tif (w == 2u) { return textureSampleLevel(texs[t], samp_v, i.uv, lv); }\n\t\tif (w == 3u) { return textureSampleLevel(texs[t], samp_uv, i.uv, lv); }\n\t\treturn textureSampleLevel(texs[t], samp, i.uv, lv);\n\t}\n\tif (w == 1u) { return textureSampleGrad(texs[t], samp_u, i.uv, dx, dy); }\n\tif (w == 2u) { return textureSampleGrad(texs[t], samp_v, i.uv, dx, dy); }\n\tif (w == 3u) { return textureSampleGrad(texs[t], samp_uv, i.uv, dx, dy); }\n\treturn textureSampleGrad(texs[t], samp, i.uv, dx, dy);\n}",
	);
	// The shrub material word sits above the texture slot here (`first_instance`); `vs_shrub` also passes the slot on.
	s = s.replace("fn shrub_inst(inst: u32) -> u32 {\n\treturn inst;\n}", "fn shrub_inst(inst: u32) -> u32 {\n\treturn inst >> 16u;\n}");
	s = s.replace("\tlet m = shrub_inst(inst);\n", "\to.ti = inst;\n\tlet m = shrub_inst(inst);\n");
	s = s.replace("textureSample(tex, samp,", "textureSample(texs[i.ti & 0xffffu], samp,").replace("textureSampleLevel(tex, samp,", "textureSampleLevel(texs[i.ti & 0xffffu], samp,").replace("textureSampleGrad(tex, samp,", "textureSampleGrad(texs[i.ti & 0xffffu], samp,");
	// `vs` (ties): the slot, next to the tie K word `tie_kmx` reads above it.
	s.replace("\to.kmx = tie_kmx(l, inst, o.kmx);\n", "\to.ti = inst;\n\to.kmx = tie_kmx(l, inst, o.kmx);\n")
}

/// The array layout and the pipelines the tie and shrub draws use: the two tie passes (`tie_pipelines`), opaque (no cull,
/// back, front) and the two faded shrub passes (`shrub_pipelines[0..2]`).
pub(super) struct Bindless {
	pub layout: wgpu::BindGroupLayout,
	pub tie: [[wgpu::RenderPipeline; 3]; 2],
	pub pipeline: wgpu::RenderPipeline,
	pub pipeline_cull: [wgpu::RenderPipeline; 2],
	pub shrub_faded: [wgpu::RenderPipeline; 2],
	/// The frame's indirect draw arguments and their capacity (None: one `draw_indexed` per draw).
	pub indirect: Option<(wgpu::Buffer, usize)>,
}

/// One list's multi-draw runs: (cull mode, byte offset into the indirect buffer, draw count).
pub(super) type Runs = Vec<(u8, u64, u32)>;

/// Appends the indirect arguments of `draws` (texture slot as the first instance) to `args`; consecutive draws with the same
/// cull mode form one run (`split_cull` false: one run, for the lists drawn with a single pipeline).
pub(super) fn push_args(args: &mut Vec<[u32; 5]>, draws: impl Iterator<Item = Draw>, split_cull: bool) -> Runs {
	let mut runs: Runs = Vec::new();
	for d in draws {
		let cull = if split_cull { d.cull } else { 0 };
		match runs.last_mut() {
			Some(r) if r.0 == cull => r.2 += 1,
			_ => runs.push((cull, (args.len() * 20) as u64, 1)),
		}
		args.push([d.count, 1, d.start, 0, first_instance(&d)]);
	}
	runs
}

/// A draw's first instance in the array pipelines: the texture slot, with the shrub mesh material word (`Draw::inst`: CLAMP
/// wrap, TEX1 K, top level; `vs_shrub`) above.
pub(super) fn first_instance(d: &Draw) -> u32 {
	d.texture as u32 | d.inst << 16
}

impl Bindless {
	pub(super) fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
		device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("texture array"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
					count: std::num::NonZeroU32::new(TEX_CAP),
				},
				wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
				// Shrub mesh CLAMP wraps 1..=3 (plain bindings: array elements count against the texture array's limit).
				wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
				wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
				wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
			],
		})
	}

	/// Writes this frame's indirect arguments (growing the buffer to the next power of two).
	pub(super) fn write_args(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, args: &[[u32; 5]]) {
		let Some((buf, cap)) = &mut self.indirect else { return };
		if args.len() > *cap {
			*cap = args.len().next_power_of_two();
			*buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("indirect draws"), size: (*cap * 20) as u64, usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		}
		if !args.is_empty() {
			queue.write_buffer(buf, 0, bytemuck::cast_slice(args));
		}
	}

	/// Draws `runs` (`push_args`) with the opaque pipelines of their cull mode, or with `fixed` for every run.
	pub(super) fn multi_draw(&self, pass: &mut wgpu::RenderPass, runs: &Runs, fixed: Option<&wgpu::RenderPipeline>) {
		let Some((buf, _)) = &self.indirect else { return };
		for &(cull, off, n) in runs {
			pass.set_pipeline(fixed.unwrap_or(if cull == 0 { &self.pipeline } else { &self.pipeline_cull[cull as usize - 1] }));
			pass.multi_draw_indexed_indirect(buf, off, n);
		}
	}

	/// Draws `runs` (`push_args`) with `set[cull mode]` (the tie passes).
	pub(super) fn multi_draw_set(&self, pass: &mut wgpu::RenderPass, runs: &Runs, set: &[wgpu::RenderPipeline; 3]) {
		let Some((buf, _)) = &self.indirect else { return };
		for &(cull, off, n) in runs {
			pass.set_pipeline(&set[cull as usize]);
			pass.multi_draw_indexed_indirect(buf, off, n);
		}
	}
}

pub(super) fn array_group(layout: &wgpu::BindGroupLayout, device: &wgpu::Device, binds: &[wgpu::BindGroup], views: &HashMap<wgpu::BindGroup, wgpu::TextureView>, sampler: &wgpu::Sampler, wrap: &[wgpu::Sampler; 3]) -> Option<wgpu::BindGroup> {
	if binds.is_empty() || binds.len() > TEX_CAP as usize {
		return None;
	}
	let mut list: Vec<&wgpu::TextureView> = binds.iter().map(|b| views.get(b)).collect::<Option<_>>()?;
	list.resize(TEX_CAP as usize, list[0]);
	Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
		label: Some("texture array"),
		layout,
		entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureViewArray(&list) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
			wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&wrap[0]) },
			wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&wrap[1]) },
			wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&wrap[2]) },
		],
	}))
}

#[cfg(test)]
mod tests {
	#[test]
	fn shader_source_rewrites_every_texture_read() {
		let s = super::shader_source(super::super::SHADER);
		assert!(!s.contains("(tex,") && !s.contains("var tex:"));
		assert!(s.contains("o.ti = inst;") && s.contains("ti: u32 };"));
		assert!(s.contains("samp_uv, i.uv") && s.contains("binding(4) var samp_uv"), "the shrub mesh texel is rewritten to the wrap samplers");
		assert!(s.contains("\to.ti = inst;\n\to.kmx = tie_kmx(l, inst, o.kmx);"), "vs (ties) passes the slot on");
		assert!(s.contains("textureSampleGrad(texs[t], samp_uv, i.uv, dx, dy)") && s.contains("textureSampleLevel(texs[t], samp_v, i.uv, lv)"), "the tie texel is rewritten to the wrap samplers");
		assert!(s.contains("return inst >> 16u;") && s.contains("\to.ti = inst;\n\tlet m = shrub_inst(inst);"), "vs_shrub reads the material word above the slot");
		naga::front::wgsl::parse_str(&s).expect("bindless wgsl parse");
	}
}
