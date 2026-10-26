//! Making the water pass: pipelines, buffers, bind-group layouts (`WaterFx::new`) and the targets that follow the window
//! and the post chain (`prepare`).

use super::*;

impl WaterFx {
	pub(in crate::viewer) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat, tex_layout: &wgpu::BindGroupLayout) -> WaterFx {
		// RAC_WATER_DBG=N (dev aid, off by default): fs_water returns one of its inputs as a colour (seam hunting).
		let dbg = match std::env::var("RAC_WATER_DBG").ok().and_then(|v| v.parse::<u32>().ok()) {
			Some(1) => "return vec4<f32>(clamp(thick / 4.0, 0.0, 1.0), select(0.0, 1.0, blended), fract((p.z - 223.0) * 50.0), 1.0);",
			Some(2) => "return vec4<f32>(g.x * 4.0 + 0.5, g.y * 4.0 + 0.5, g.z, 1.0);",
			Some(3) => "return vec4<f32>((uv - uv0) * 60.0 + vec2<f32>(0.5), clamp((zs - zw) / 2.0, 0.0, 1.0), 1.0);",
			Some(4) => "return vec4<f32>(rs * 3.0 + vec2<f32>(0.5), 0.5, 1.0);",
			Some(5) => "return vec4<f32>(orig, 1.0);",
			_ => "",
		};
		let src = SHADER.replace("//DBGHOOK", dbg) + &skymap::wgsl(12, 13) + edge_calm::EDGE_WGSL + far_look::FAR_WGSL + steep_look::STEEP_WGSL + shore_fade::SHORE_FADE_WGSL + shore_map::SHORE_DIST_WGSL + hero_crest::HERO_CREST_WGSL + rim::RIM_WGSL + underside::UNDER_WGSL + dry_tiles::DRY_WGSL;
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("water fx"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("water fx", src.into())) });
		let tex = |binding: u32, sample_type: wgpu::TextureSampleType, multisampled: bool| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled },
			count: None,
		};
		let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("water fx"),
			entries: &[
				wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
				wgpu::BindGroupLayoutEntry { visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ..tex(1, wgpu::TextureSampleType::Depth, true) },
				tex(2, wgpu::TextureSampleType::Float { filterable: true }, false),
				tex(3, unfiltered, false),
				tex(4, unfiltered, false),
				wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
				wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None },
				tex(7, unfiltered, false),
				tex(8, wgpu::TextureSampleType::Float { filterable: true }, false),
				tex(9, wgpu::TextureSampleType::Depth, true),
				wgpu::BindGroupLayoutEntry { visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ..tex(10, unfiltered, false) },
				wgpu::BindGroupLayoutEntry { binding: 11, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None }, tex(12, wgpu::TextureSampleType::Depth, true), wgpu::BindGroupLayoutEntry { binding: 13, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
			],
		});
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water fx"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		let p_pos = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("water positions"),
			layout: Some(&pl),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_pos"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &[wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }] })],
			},
			fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_pos"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: WPOS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(),
			depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let p_shade = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("water shade"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_water"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(),
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let p_ao_sea = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("water sea occlusion"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs_ao_sea"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R8Unorm, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
			}),
			primitive: Default::default(),
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		// [PC] Geometric sea waves: the flat sea mask, the projected grid, the shore map.
		let p_mask = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("water sea mask"),
			layout: Some(&pl),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_pos"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &[wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }] })],
			},
			fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_mask"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Float, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(), depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let dry = DryTiles::new(device, &shader, &layout);
		let p_grid = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("water sea grid"),
			layout: Some(&dry.grid_pl),
			vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_grid"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_grid"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: WPOS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(),
			depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let shore_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("water shore map"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("water shore map", SHORE_SHADER.into())) });
		let shore_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("water shore map"),
			entries: &[
				wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
				wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: true }, count: None },
				wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
			],
		});
		let shore_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water shore map"), bind_group_layouts: &[Some(&shore_layout)], immediate_size: 0 });
		let shore_pipe = |entry: &str| {
			device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
				label: Some("water shore map"),
				layout: Some(&shore_pl),
				module: &shore_shader,
				entry_point: Some(entry),
				compilation_options: Default::default(),
				cache: None,
			})
		};
		let (p_shore, p_shore_keep, p_shore_h, p_shore_v) = (shore_pipe("cs_shore"), shore_pipe("cs_shore_keep"), shore_pipe("cs_shore_h"), shore_pipe("cs_shore_v"));
		// The waterfalls also see retail's waterfall texture (group 1, a level texture's bind group).
		let pl_fall = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water falls"), bind_group_layouts: &[Some(&layout), Some(tex_layout)], immediate_size: 0 });
		let p_fall = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("water falls"),
			layout: Some(&pl_fall),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_fall"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout {
					array_stride: std::mem::size_of::<Vertex>() as u64,
					step_mode: wgpu::VertexStepMode::Vertex,
					attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4],
				})],
			},
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs_fall"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState {
					format,
					blend: Some(wgpu::BlendState {
						color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
						alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
					}),
					write_mask: wgpu::ColorWrites::ALL,
				})],
			}),
			primitive: Default::default(),
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let fall_dummy = {
			let view = device
				.create_texture(&wgpu::TextureDescriptor { label: Some("water falls dummy"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] })
				.create_view(&Default::default());
			let sampler = device.create_sampler(&Default::default());
			device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("water falls dummy"), layout: tex_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }] })
		};
		let small = |format| {
			device
				.create_texture(&wgpu::TextureDescriptor { label: Some("water fx dummy"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] })
				.create_view(&Default::default())
		};
		WaterFx {
			layout,
			ubuf: device.create_buffer(&wgpu::BufferDescriptor { label: Some("water fx uniforms"), size: std::mem::size_of::<WaterU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
			sampler: device.create_sampler(&wgpu::SamplerDescriptor { label: Some("water fx linear"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() }),
			p_pos,
			p_shade,
			p_ao_sea,
			dummy: small(format),
			dummy_arr: device
				.create_texture(&wgpu::TextureDescriptor { label: Some("water fx dummy array"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] })
				.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() }),
			dummy_r32: small(WPOS_FORMAT),
			format,
			depth_format,
			start: std::time::Instant::now(),
			t: None,
			drift: std::collections::HashMap::new(),
			flows: vec![[0.0; 2]],
			river_uv: std::collections::HashMap::new(),
			river_step: [0.0; 2],
			phased: false, heave: Default::default(),
			rings: Rings::default(),
			wakes: Wakes::default(),
			falls: Vec::new(),
			falls_fresh: false,
			p_fall,
			fall_dummy,
			fall_verts: Vec::new(),
			fall: None,
			sheet_mem: SheetMem::default(),
			sheet_level: usize::MAX,
			p_mask,
			p_grid, dry,
			p_shore, p_shore_keep,
			p_shore_h,
			p_shore_v,
			shore_layout,
			shore: device.create_buffer(&wgpu::BufferDescriptor { label: Some("water shore map"), size: (8 * SHORE_N * SHORE_N * 4) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
			shore_u: device.create_buffer(&wgpu::BufferDescriptor { label: Some("water shore map"), size: std::mem::size_of::<ShoreU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
			grid_ib: None,
			geo_draw: None,
			shore_still: Vec::new(),
			shore_movers: Vec::new(), sky: SkyMap::new(device, format, depth_format),
		}
	}

	/// The targets for the post's current ones (`post::Post::water_inputs`): (re)made when the window or the post targets change.
	pub(super) fn prepare(&mut self, device: &wgpu::Device, depth: &wgpu::TextureView, scene: &wgpu::TextureView, lin: &wgpu::TextureView, glow: &wgpu::TextureView, generation: u64) {
		let tex = scene.texture();
		let (w, h) = (tex.width(), tex.height());
		if self.t.as_ref().is_some_and(|t| t.key == (w, h, generation)) {
			return;
		}
		let make = |label, format, usage| {
			device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
		};
		let wpos = make("water positions", WPOS_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
		let wdepth = make("water depth", wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
		let copy = make("water scene copy", self.format, wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING);
		let copy_v = copy.create_view(&Default::default());
		let depth_only = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		let half = |label, format, samples, usage| {
			let size = wgpu::Extent3d { width: (w / 2).max(1), height: (h / 2).max(1), depth_or_array_layers: 1 };
			device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] }).create_view(&Default::default())
		};
		let mirror_ms = half("water mirror msaa", self.format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT);
		let mirror_tex = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("water mirror"),
			size: wgpu::Extent3d { width: (w / 2).max(1), height: (h / 2).max(1), depth_or_array_layers: super::mirror::MAX_MIRRORS as u32 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: self.format,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		});
		let mirror = mirror_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
		let mirror_layers: Vec<wgpu::TextureView> = (0..super::mirror::MAX_MIRRORS as u32)
			.map(|k| mirror_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: k, array_layer_count: Some(1), ..Default::default() }))
			.collect();
		let mirror_depth = half("water mirror depth", self.depth_format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT);
		let bed_ms = half("water bed msaa", self.format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT);
		let bed = half("water bed", self.format, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
		let bed_depth = half("water bed depth", self.depth_format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
		let bed_depth_only = bed_depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		let seamask = make("water sea mask", wgpu::TextureFormat::R32Float, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
		let group = |scene_v: &wgpu::TextureView, lin_v: &wgpu::TextureView, wpos_v: &wgpu::TextureView, mirror_v: &wgpu::TextureView, glow_v: &wgpu::TextureView, bed_v: &wgpu::TextureView, mask_v: &wgpu::TextureView| {
			device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("water fx"),
				layout: &self.layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&depth_only) },
					wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(scene_v) },
					wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(lin_v) },
					wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(wpos_v) },
					wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.sampler) },
					wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(mirror_v) },
					wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(glow_v) },
					wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(bed_v) },
					wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&bed_depth_only) },
					wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(mask_v) },
					wgpu::BindGroupEntry { binding: 11, resource: self.shore.as_entire_binding() }, wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(self.sky.depth_view()) }, wgpu::BindGroupEntry { binding: 13, resource: self.sky.uniform().as_entire_binding() },
				],
			})
		};
		let g_pos = group(&self.dummy, &self.dummy_r32, &self.dummy_r32, &self.dummy_arr, &self.dummy, &self.dummy, &self.dummy_r32);
		let g_grid = group(&self.dummy, &self.dummy_r32, &self.dummy_r32, &self.dummy_arr, &self.dummy, &self.dummy, &seamask);
		let g_shade = group(&copy_v, lin, &wpos, &mirror, glow, &bed, &seamask);
		let dry = self.dry.groups(device, (w, h), &self.ubuf, &depth_only, &seamask);
		let g_shore = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("water shore map"),
			layout: &self.shore_layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: self.shore_u.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&bed_depth_only) },
				wgpu::BindGroupEntry { binding: 2, resource: self.shore.as_entire_binding() },
			],
		});
		self.t = Some(Targets { key: (w, h, generation), wpos, wdepth, copy, g_pos, g_shade, mirror_ms, mirror_layers, mirror_depth, bed_ms, bed, bed_depth, seamask, g_grid, g_shore, dry });
	}
}
