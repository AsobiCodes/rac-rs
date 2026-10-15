use super::*;
use rayon::prelude::*;

impl Gpu {
	pub(super) fn new(window: Arc<Window>) -> Gpu {
		// [PC] WGPU_BACKEND (vulkan / dx12 / ...) picks the backend, as wgpu's other WGPU_* switches.
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
		let surface = instance.create_surface(window.clone()).expect("surface");
		let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
			power_preference: wgpu::PowerPreference::HighPerformance,
			compatible_surface: Some(&surface),
			force_fallback_adapter: false,
			apply_limit_buckets: false,
		}))
		.expect("no GPU adapter");
		// [PC] perf round 3 (bindless.rs): texture binding arrays for the tie / shrub draws when the adapter has them.
		let bindless = super::bindless::wanted(&adapter);
		let multi = bindless && super::bindless::multi_wanted(&adapter);
		// BRIDGE-64a: the shadow volumes count in a stencil aspect of the main depth target (Depth32FloatStencil8 where the
		// adapter has it, else the core Depth24PlusStencil8).
		let stencil32 = adapter.features().contains(wgpu::Features::DEPTH32FLOAT_STENCIL8);
		let depth_format = if stencil32 { wgpu::TextureFormat::Depth32FloatStencil8 } else { wgpu::TextureFormat::Depth24PlusStencil8 };
		let stencil_feature = if stencil32 { wgpu::Features::DEPTH32FLOAT_STENCIL8 } else { wgpu::Features::empty() };
		let desc = match bindless {
			true => wgpu::DeviceDescriptor {
				required_features: super::bindless::features() | stencil_feature | if multi { wgpu::Features::INDIRECT_FIRST_INSTANCE } else { wgpu::Features::empty() },
				required_limits: wgpu::Limits { max_binding_array_elements_per_shader_stage: super::bindless::TEX_CAP, ..Default::default() },
				..Default::default()
			},
			false => wgpu::DeviceDescriptor { required_features: stencil_feature, ..Default::default() },
		};
		let (device, queue) = pollster::block_on(adapter.request_device(&desc)).expect("device");
		let size = window.inner_size();
		let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
		config.usage |= wgpu::TextureUsages::COPY_SRC; // for --shot
		// The GS blends and modulates raw 8-bit values: render in gamma space like it does.
		config.format = config.format.remove_srgb_suffix();
		config.present_mode = present_mode(&surface.get_capabilities(&adapter).present_modes, std::env::var("RAC_VSYNC").is_ok_and(|v| v == "1"), std::env::var("RAC_PRESENT").ok().as_deref());
		configure_surface(&surface, &device, &config);
		eprintln!("gpu: {:?} {}, present mode {:?}, frame latency {}", adapter.get_info().backend, adapter.get_info().name, config.present_mode, config.desired_maximum_frame_latency);

		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("level"),
			source: wgpu::ShaderSource::Wgsl(SHADER.into()),
		});
		let ulayout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("uniforms"),
			entries: &[wgpu::BindGroupLayoutEntry {
				binding: 0,
				visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
				ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
				count: None,
			}],
		});
		let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("texture"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
					count: None,
				},
			],
		});
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("level"),
			bind_group_layouts: &[Some(&ulayout), Some(&tex_layout)],
			immediate_size: 0,
		});
		let make_prim = |vs: &str, fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool, compare: wgpu::CompareFunction, primitive: wgpu::PrimitiveState| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(vs),
				layout: Some(&layout),
				vertex: wgpu::VertexState {
					module: &shader,
					entry_point: Some(vs),
					compilation_options: Default::default(),
					buffers: &[Some(wgpu::VertexBufferLayout {
						array_stride: std::mem::size_of::<Vertex>() as u64,
						step_mode: wgpu::VertexStepMode::Vertex,
						attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4],
					})],
				},
				fragment: Some(wgpu::FragmentState {
					module: &shader,
					entry_point: Some(fs),
					compilation_options: Default::default(),
					targets: &[Some(wgpu::ColorTargetState { format: config.format, blend, write_mask: wgpu::ColorWrites::ALL })],
				}),
				primitive,
				depth_stencil: Some(wgpu::DepthStencilState {
					format: depth_format,
					depth_write_enabled: Some(depth_write),
					depth_compare: Some(compare),
					stencil: Default::default(),
					bias: Default::default(),
				}),
				multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
				multiview_mask: None,
				cache: None,
			})
		};
		let make_cull = |vs: &str, fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool, compare: wgpu::CompareFunction, cull: Option<wgpu::Face>| make_prim(vs, fs, blend, depth_write, compare, wgpu::PrimitiveState { cull_mode: cull, ..Default::default() });
		let make = |vs: &str, fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool, compare: wgpu::CompareFunction| make_cull(vs, fs, blend, depth_write, compare, None);
		// KER-92: pool draw mode 2, GS PRIM line + IIP, ALPHA_1 0x44 (alpha-over), depth-tested as the particles, no writes.
		let line_pipeline = make_prim("vs", "fs_line", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::Less, wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() });
		let pipeline = make("vs", "fs", None, true, wgpu::CompareFunction::Less);
		// Moby packets whose VU1 cull mode is 2 (back) or 0 (front) (NOV-5.9a).
		let pipeline_cull = [
			make_cull("vs", "fs", None, true, wgpu::CompareFunction::Less, Some(wgpu::Face::Back)),
			make_cull("vs", "fs", None, true, wgpu::CompareFunction::Less, Some(wgpu::Face::Front)),
		];
		let moby_pipelines = [
			make("vs", "fs_moby_a", Some(wgpu::BlendState::ALPHA_BLENDING), true, wgpu::CompareFunction::LessEqual),
			make("vs", "fs_moby_b", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::LessEqual),
		];
		// The ABE pass keeps each packet's VU1 cull mode, as the opaque pass does (a culled packet sends no back / front faces
		// to the GS at all, whatever the texel alpha).
		let moby_cull = [wgpu::Face::Back, wgpu::Face::Front].map(|f| {
			[
				make_cull("vs", "fs_moby_a", Some(wgpu::BlendState::ALPHA_BLENDING), true, wgpu::CompareFunction::LessEqual, Some(f)),
				make_cull("vs", "fs_moby_b", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::LessEqual, Some(f)),
			]
		});
		let [[mc0, mc1], [mc2, mc3]] = moby_cull;
		let moby_cull = [mc0, mc1, mc2, mc3];
		// REN-CULL-SHRUB: GS alpha-test Z split of the faded shrub meshes (>= 12) and of the billboards (pass 1 >= 96, pass 2 never).
		// Retail shrub TEST_1 ZTST GEQUAL (BRIDGE-27) = LessEqual here (as `moby_pipelines`); Less dropped coplanar decal shrubs
		// (Eudora dirt paths) along terrain edges, leaving thin straight lines.
		let shrub_pipelines = [
			make("vs_shrub", "fs_shrub_ge12", Some(wgpu::BlendState::ALPHA_BLENDING), true, wgpu::CompareFunction::LessEqual),
			make("vs_shrub", "fs_shrub_lt12", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::LessEqual),
			make("vs", "fs_shrub_ge96", Some(wgpu::BlendState::ALPHA_BLENDING), true, wgpu::CompareFunction::LessEqual),
			make("vs", "fs_shrub_lt96", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::LessEqual),
			make("vs", "fs_shrub_all", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::LessEqual),
		];
		// Terrain fragments: explicit mip level from the per-draw K / top level (REN-TEX-RESIDENCY).
		let tfrag_pipeline = make("vs_tfrag", "fs_tfrag", Some(wgpu::BlendState::ALPHA_BLENDING), true, wgpu::CompareFunction::Less);
		// Texels below alpha 0x60 (AFAIL RGB_ONLY): blended, no z write; drawn after the other batches.
		let tfrag_pipeline_lt = make("vs_tfrag", "fs_tfrag_lt96", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::Less);
		let sky_pipeline = make("vs_sky", "fs_sky", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::LessEqual);
		let clear_pipeline = make("vs_hudz", "fs_hud", Some(wgpu::BlendState::ALPHA_BLENDING), true, wgpu::CompareFunction::Always);
		// GS draw flags 0x48 (alpha) and 0x44 (additive) [I]: depth-tested, no depth writes.
		let additive = wgpu::BlendState {
			color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
			alpha: wgpu::BlendComponent::OVER,
		};
		// ARI-4: the sky sprites blend additively (ALPHA_1 0x48) at the far plane like the shells.
		let sky_add_pipeline = make("vs_sky", "fs_sky", Some(additive), false, wgpu::CompareFunction::LessEqual);
		let hud_pipeline = [
			make("vs_hud", "fs_hud", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::Always),
			make("vs_hud", "fs_hud", Some(additive), false, wgpu::CompareFunction::Always),
			// REN-9b: the item-panel blit is opaque (GS ALPHA FIX 0x80 = Cs).
			make("vs_hud", "fs_hud", None, false, wgpu::CompareFunction::Always),
			// Retail mode: the GS overlay is premultiplied colour plus transmission, out = Cs + As * Cd.
			make(
				"vs_hud",
				"fs_hud",
				Some(wgpu::BlendState {
					color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::SrcAlpha, operation: wgpu::BlendOperation::Add },
					alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
				}),
				false,
				wgpu::CompareFunction::Always,
			),
			// Retail screen filter (`RETAIL_FILTER_TEX`): GS ALPHA 0x42 Cd - Cs*As (unorm target: clamped at 0).
			make("vs_hud", "fs_hud_flat", Some(wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::ReverseSubtract }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add } }), false, wgpu::CompareFunction::Always),
		];
		let part_pipelines = [
			make("vs", "fs_part", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::Less),
			make("vs", "fs_part", Some(additive), false, wgpu::CompareFunction::Less),
			// Water (prog10 fogs per vertex like tfrag, lighting_fog.md): alpha blend with the scene fog.
			make("vs", "fs_part_fog", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::Less),
		];
		// BRIDGE-23: GS ALPHA A=Cs B=Cd C=FIX D=Cd, out = Cs * k + Cd * (1 - k) with k = the blend constant (FIX / 128).
		let fix_blend = wgpu::BlendState {
			color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Constant, dst_factor: wgpu::BlendFactor::OneMinusConstant, operation: wgpu::BlendOperation::Add },
			alpha: wgpu::BlendComponent::OVER,
		};
		// The strip callbacks inherit the frame's TEST_1 0x5360b (ZTST GEQUAL, set after the sky pass; decomp/notes/aridia_presentation.md):
		// the nearer or equal z passes = LessEqual on rac-rs depth (decomp/notes/draw_producers.md, Kalebo glass).
		let strip_pipeline = make("vs", "fs_part_fix", Some(fix_blend), false, wgpu::CompareFunction::LessEqual);
		// The Hoven pools: TEST_1/2 0x50000 (ZTE, ZTST GEQUAL: the nearer or equal z passes, so pass 2 lands on pass 1) and Z written
		// (decomp/notes/fx_census_late.md, Hoven water pools).
		let pool_pipeline = make("vs", "fs_part_fix", Some(fix_blend), true, wgpu::CompareFunction::LessEqual);
		// Effects layer, ALPHA A=Cs B=0 C=FIX D=Cd (0x68): out = Cs * k + Cd, no Z write (WorldStrip.add).
		let fix_add = wgpu::BlendState {
			color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Constant, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
			alpha: wgpu::BlendComponent::OVER,
		};
		let pool_add_pipeline = make("vs", "fs_part_fix", Some(fix_add), false, wgpu::CompareFunction::LessEqual);
		// ARI-49b: the Aridia ocean is opaque (ALPHA FIX 0x80 = Cs) and writes Z (TEST_1 0x5360a, no alpha test); its
		// own fog words and FOGCOL, or the level fog while the Visibomb is live.
		// KER-30: the Kerwan ocean (FIX 0x20 < 0x80) blends (Cs - Cd) * FIX / 128 + Cd with the BRIDGE-23 constant blend,
		// depth-tested without Z write (TEST_1 0x5370b: vertex alpha 0 fails the alpha test, RGB only); its own fog
		// (fs_ocean_k), or the level fog while the Visibomb is live. Index = (fix < 0x80) * 2 + level fog.
		let ocean_pipeline = [
			make("vs", "fs_ocean", None, true, wgpu::CompareFunction::Less),
			make("vs", "fs_part_fix", None, true, wgpu::CompareFunction::Less),
			make("vs", "fs_ocean_k", Some(fix_blend), false, wgpu::CompareFunction::Less),
			make("vs", "fs_part_fix", Some(fix_blend), false, wgpu::CompareFunction::Less),
			// Retail-mode RAM oceans (`fs_ocean_f`): 4 = opaque (FIX 0x80), 5 = FIX-blended.
			make("vs", "fs_ocean_f", None, true, wgpu::CompareFunction::Less),
			make("vs", "fs_ocean_f", Some(fix_blend), false, wgpu::CompareFunction::Less),
		];
		// Retail mode: the oceans of Rilgar, Umbris and Gaspar (own fog per vertex, `fs_ocean_g`): opaque with Z, or FIX-blended without Z.
		// 2 / 3 = the Gaspar second lava pass (coplanar with the first: GS TEST ZTST GEQUAL = nearer or equal passes).
		let ocean_g_pipeline = [
			make("vs", "fs_ocean_g", None, true, wgpu::CompareFunction::Less),
			make("vs", "fs_ocean_g", Some(fix_blend), false, wgpu::CompareFunction::Less),
			make("vs", "fs_ocean_g", None, true, wgpu::CompareFunction::LessEqual),
			make("vs", "fs_ocean_g", Some(fix_blend), false, wgpu::CompareFunction::LessEqual),
		];
		let bindless = bindless.then(|| {
			let bl_layout = super::bindless::Bindless::layout(&device);
			let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("level bindless"), source: wgpu::ShaderSource::Wgsl(super::bindless::shader_source(SHADER).into()) });
			let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("level bindless"), bind_group_layouts: &[Some(&ulayout), Some(&bl_layout)], immediate_size: 0 });
			let make_bl = |vs: &str, fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool, cull: Option<wgpu::Face>, compare: wgpu::CompareFunction| {
				device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
					label: Some("bindless"),
					layout: Some(&pl),
					vertex: wgpu::VertexState {
						module: &module,
						entry_point: Some(vs),
						compilation_options: Default::default(),
						buffers: &[Some(wgpu::VertexBufferLayout {
							array_stride: std::mem::size_of::<Vertex>() as u64,
							step_mode: wgpu::VertexStepMode::Vertex,
							attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4],
						})],
					},
					fragment: Some(wgpu::FragmentState {
						module: &module,
						entry_point: Some(fs),
						compilation_options: Default::default(),
						targets: &[Some(wgpu::ColorTargetState { format: config.format, blend, write_mask: wgpu::ColorWrites::ALL })],
					}),
					primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
					depth_stencil: Some(wgpu::DepthStencilState {
						format: depth_format,
						depth_write_enabled: Some(depth_write),
						depth_compare: Some(wgpu::CompareFunction::Less),
						stencil: Default::default(),
						bias: Default::default(),
					}),
					multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
					multiview_mask: None,
					cache: None,
				})
			};
			// The same states as `pipeline`, `pipeline_cull` and `shrub_pipelines[0..2]`.
			super::bindless::Bindless {
				pipeline: make_bl("vs", "fs", None, true, None, wgpu::CompareFunction::Less),
				pipeline_cull: [make_bl("vs", "fs", None, true, Some(wgpu::Face::Back), wgpu::CompareFunction::Less), make_bl("vs", "fs", None, true, Some(wgpu::Face::Front), wgpu::CompareFunction::Less)],
				shrub_faded: [
					make_bl("vs_shrub", "fs_shrub_ge12", Some(wgpu::BlendState::ALPHA_BLENDING), true, None, wgpu::CompareFunction::LessEqual),
					make_bl("vs_shrub", "fs_shrub_lt12", Some(wgpu::BlendState::ALPHA_BLENDING), false, None, wgpu::CompareFunction::LessEqual),
				],
				layout: bl_layout,
				indirect: multi.then(|| (device.create_buffer(&wgpu::BufferDescriptor { label: Some("indirect draws"), size: 20 * 1024, usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }), 1024)),
			}
		});
		// [PC] perf round 3 (gpuskin.rs): the props batch's pipelines, posed in `vs_props` from the static model, the (slot, vertex)
		// attribute and the storage records; same fragment states as `pipeline`, `pipeline_cull` and `moby_pipelines`.
		let props_gpu = (!super::gpuskin::cpu_skin()).then(|| {
			let skin_layout = super::gpuskin::PropsGpu::layout(&device);
			let props_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
				label: Some("props skin"),
				bind_group_layouts: &[Some(&ulayout), Some(&tex_layout), Some(&skin_layout)],
				immediate_size: 0,
			});
			let make_props = |fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool, compare: wgpu::CompareFunction, cull: Option<wgpu::Face>| {
				device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
					label: Some("vs_props"),
					layout: Some(&props_layout),
					vertex: wgpu::VertexState {
						module: &shader,
						entry_point: Some("vs_props"),
						compilation_options: Default::default(),
						buffers: &[
							Some(wgpu::VertexBufferLayout {
								array_stride: std::mem::size_of::<Vertex>() as u64,
								step_mode: wgpu::VertexStepMode::Vertex,
								attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4],
							}),
							Some(wgpu::VertexBufferLayout { array_stride: 8, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![5 => Uint32x2] }),
						],
					},
					fragment: Some(wgpu::FragmentState {
						module: &shader,
						entry_point: Some(fs),
						compilation_options: Default::default(),
						targets: &[Some(wgpu::ColorTargetState { format: config.format, blend, write_mask: wgpu::ColorWrites::ALL })],
					}),
					primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
					depth_stencil: Some(wgpu::DepthStencilState {
						format: depth_format,
						depth_write_enabled: Some(depth_write),
						depth_compare: Some(compare),
						stencil: Default::default(),
						bias: Default::default(),
					}),
					multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
					multiview_mask: None,
					cache: None,
				})
			};
			let opaque = make_props("fs", None, true, wgpu::CompareFunction::Less, None);
			let cull = [
				make_props("fs", None, true, wgpu::CompareFunction::Less, Some(wgpu::Face::Back)),
				make_props("fs", None, true, wgpu::CompareFunction::Less, Some(wgpu::Face::Front)),
			];
			let moby = [
				make_props("fs_moby_a", Some(wgpu::BlendState::ALPHA_BLENDING), true, wgpu::CompareFunction::LessEqual, None),
				make_props("fs_moby_b", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::LessEqual, None),
			];
			let [[mc0, mc1], [mc2, mc3]] = [wgpu::Face::Back, wgpu::Face::Front].map(|f| {
				[
					make_props("fs_moby_a", Some(wgpu::BlendState::ALPHA_BLENDING), true, wgpu::CompareFunction::LessEqual, Some(f)),
					make_props("fs_moby_b", Some(wgpu::BlendState::ALPHA_BLENDING), false, wgpu::CompareFunction::LessEqual, Some(f)),
				]
			});
			super::gpuskin::PropsGpu::new(&device, skin_layout, opaque, cull, moby, [mc0, mc1, mc2, mc3])
		});
		let ubuf = buffer(&device, "uniforms", bytemuck::bytes_of(&Uniforms::zeroed()), wgpu::BufferUsages::UNIFORM);
		let ubind = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("uniforms"),
			layout: &ulayout,
			entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
		});
		let pbuf = [0, 1, 2, 3, 4, 5, 6, 7].map(|_| buffer(&device, "preview uniforms", bytemuck::bytes_of(&Uniforms::zeroed()), wgpu::BufferUsages::UNIFORM));
		let pbind = [0, 1, 2, 3, 4, 5, 6, 7].map(|i| {
			device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("preview uniforms"),
				layout: &ulayout,
				entries: &[wgpu::BindGroupEntry { binding: 0, resource: pbuf[i].as_entire_binding() }],
			})
		});
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			address_mode_u: wgpu::AddressMode::Repeat,
			address_mode_v: wgpu::AddressMode::Repeat,
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			mipmap_filter: wgpu::MipmapFilterMode::Linear,
			anisotropy_clamp: 8,
			..Default::default()
		});
		// Shrub billboards: GS CLAMP_1 = 5 clamps both axes; the mip level is chosen in the shader, so the sampler only filters within it.
		let clamp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			address_mode_u: wgpu::AddressMode::ClampToEdge,
			address_mode_v: wgpu::AddressMode::ClampToEdge,
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			mipmap_filter: wgpu::MipmapFilterMode::Nearest,
			..Default::default()
		});
		// Shrub meshes: the material's GS CLAMP_1 (shrubs::WRAP_SHIFT) clamps u (wrap 1), v (wrap 2) or both (wrap 3); same filtering
		// as `sampler`. Repeat (wrap 0) is `sampler` itself.
		let wrap_samplers = [(true, false), (false, true), (true, true)].map(|(cu, cv)| {
			let mode = |c: bool| if c { wgpu::AddressMode::ClampToEdge } else { wgpu::AddressMode::Repeat };
			device.create_sampler(&wgpu::SamplerDescriptor {
				address_mode_u: mode(cu),
				address_mode_v: mode(cv),
				mag_filter: wgpu::FilterMode::Linear,
				min_filter: wgpu::FilterMode::Linear,
				mipmap_filter: wgpu::MipmapFilterMode::Linear,
				anisotropy_clamp: 8,
				..Default::default()
			})
		});
		// Nearest, repeating: menu frame sprites tile (0x1a noise, 0x1c scanlines).
		let nearest = device.create_sampler(&wgpu::SamplerDescriptor { address_mode_u: wgpu::AddressMode::Repeat, address_mode_v: wgpu::AddressMode::Repeat, ..Default::default() });
		let depth = depth_view(&device, config.width, config.height, depth_format);
		let shadow = super::shadowvol::ShadowPipes::new(&device, &ulayout, config.format, depth_format);
		let msaa = msaa_view(&device, config.format, config.width, config.height);
		let movie = crate::fmv::present::Presenter::new(&device, config.format);
		let new_panel = || {
			let (w, h) = ITEM_PANEL_SIZE;
			let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
			let tex = |label, format, sample_count, usage| device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size, mip_level_count: 1, sample_count, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
			let resolved = tex("item panel", config.format, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
			let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("item panel"),
				layout: &tex_layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&resolved) },
					wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&nearest) },
				],
			});
			ItemPanel {
				msaa: tex("item panel msaa", config.format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default()),
				depth: tex("item panel depth", depth_format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default()),
				resolved,
				bind,
			}
		};
		let (item_panel, popup_panel, portrait_panel) = (new_panel(), new_panel(), new_panel());
		let mut gpu = Gpu { window, surface, device, queue, config, pipeline, pipeline_cull, moby_pipelines, moby_cull, shrub_pipelines, clamp_sampler, wrap_samplers, tfrag_pipeline, tfrag_pipeline_lt, sky_pipeline, sky_add_pipeline, hud_pipeline, clear_pipeline, pbuf, pbind, preview: None, preview_gold: false, preview_pair: None, preview_worn: None, preview2: None, shop_models: Vec::new(), shop_panel_models: Vec::new(), item_panel, popup_panel, portrait_panel, portrait_shown: false, popup_bolt: None, icon_bolt: None, icon_bolt_sphere: None, popup_bolt_sphere: None, panel_posed: None, metal: MetalDraw::default(), props_gpu, bindless, hud_split: None, hud_textures: Vec::new(), nearest, hud: Vec::new(), retail_ov: None, ubuf, ubind, tex_layout, sampler, depth, msaa, scene: None, lighting: None, reserved_lights: None, fog_live: None, shrub_cap: shrubs::SHRUB_CAP, part_far: crate::water::PART_FAR, points: [[0.0; 4]; 16], table: [None; 8], moby_dyn: Vec::new(), hero_placed: (gameplay::SPAWN_AMBIENT, 0), hero_env: gameplay::placed_env(gameplay::SPAWN_AMBIENT, 0), scene_actor_light: None, marker_v: None, prop_v: None, class_uv: HashMap::new(), marker_ghosts: [None; 5], marker_bomb: None, marker_light: None, part_pipelines, strip_pipeline, pool_pipeline, pool_add_pipeline, part_tex: None, part_verts: BTreeMap::new(), line_pipeline, line_verts: Vec::new(), part_defs: Vec::new(), fx_meshes: HashMap::new(), fx_tex: HashMap::new(), fx_quads: Vec::new(), water_quads: Vec::new(), strip_draws: Vec::new(), pool_draws: Vec::new(), ocean_pipeline, ocean_tex: None, ocean_draws: Vec::new(), ocean_rgba: Vec::new(), ocean_frame: None, ocean_level_fog: false, ocean_fog: None, ocean_g_pipeline, ocean_ram: Vec::new(), warp_fx: None, movie, hero_tint_out: None, depth_format, shadow, volume_buf: None, volume_counts: (0, 0), marker_tilt: [0.0; 2] };
		let font = gpu.upload(Some(&overlay::font_texture()), &gpu.nearest);
		gpu.hud_textures.push(font);
		// prog2 maps each sprite's full texture (decomp/notes/particles.md "Draw size"); this soft disc is only
		// the fallback for an effect type with no texture mapped [P].
		let disc = (0..64 * 64)
			.map(|i| {
				let (x, y) = ((i % 64) as f32 / 31.5 - 1.0, (i / 64) as f32 / 31.5 - 1.0);
				let a = (1.0 - (x * x + y * y)).max(0.0);
				[255, 255, 255, (a * a * 255.0) as u8]
			})
			.collect();
		gpu.part_tex = Some(gpu.upload_texture(Some(&Texture { w: 64, h: 64, rgba: disc })));
		gpu
	}

	pub(super) fn ctx(&self) -> SceneCtx {
		SceneCtx { device: self.device.clone(), queue: self.queue.clone(), sampler: self.sampler.clone(), wrap_samplers: self.wrap_samplers.clone(), tex_layout: self.tex_layout.clone(), tex_log: std::cell::RefCell::new(None), bindless_layout: self.bindless.as_ref().map(|b| b.layout.clone()), gpu_posing: self.props_gpu.is_some(), paced: false }
	}

	pub(super) fn upload(&self, t: Option<&Texture>, sampler: &wgpu::Sampler) -> wgpu::BindGroup {
		self.ctx().upload(t, sampler)
	}

	pub(super) fn upload_texture(&self, t: Option<&Texture>) -> wgpu::BindGroup {
		self.ctx().upload_texture(t)
	}

	pub(super) fn upload_chain(&self, chain: &[Texture], sampler: &wgpu::Sampler) -> wgpu::BindGroup {
		self.ctx().upload_chain(chain, sampler)
	}

	pub(super) fn batch(&self, meshes: &[&Mesh], colours: Option<&[Vec<[f32; 4]>]>, ordered: bool, textures: &HashMap<u32, Texture>, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>) -> (Batch, [f32; 3], [f32; 3], Vec<Vertex>) {
		self.ctx().batch(meshes, colours, ordered, textures, bind, slots)
	}

	fn prop_slots(&self, props: &[u32], prop_meshes: &HashMap<u32, Mesh>, textures: &HashMap<u32, Texture>, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>) -> PropSlots {
		self.ctx().prop_slots(props, prop_meshes, textures, bind, slots)
	}


	/// Replaces the level's HUD art (overlay slots from 1; slot 0 stays the built-in font).
	pub(super) fn set_hud_art(&mut self, ts: &[Texture]) {
		self.hud_textures.truncate(1);
		for t in ts {
			let b = self.upload(Some(t), &self.nearest);
			self.hud_textures.push(b);
		}
	}

	/// Uploads `t` into HUD slot `slot` (None = a new slot after the others); returns the slot.
	pub(super) fn hud_texture(&mut self, slot: Option<usize>, t: &Texture) -> usize {
		let b = self.upload(Some(t), &self.nearest);
		match slot.filter(|&s| s < self.hud_textures.len()) {
			Some(s) => {
				self.hud_textures[s] = b;
				s
			}
			None => {
				self.hud_textures.push(b);
				self.hud_textures.len() - 1
			}
		}
	}

	/// Retail mode: replaces the GS overlay image (`w` x `rows` RGBA8).
	pub(super) fn set_retail_overlay(&mut self, w: u32, rows: u32, px: &[[u8; 4]]) {
		if !self.retail_ov.as_ref().is_some_and(|o| o.2 == w && o.3 == rows) {
			let tex = self.device.create_texture(&wgpu::TextureDescriptor {
				label: Some("retail overlay"),
				size: wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: wgpu::TextureFormat::Rgba8Unorm,
				usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
				view_formats: &[],
			});
			let view = tex.create_view(&Default::default());
			let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: None,
				layout: &self.tex_layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
					wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.clamp_sampler) },
				],
			});
			self.retail_ov = Some((tex, bind, w, rows));
		}
		let tex = &self.retail_ov.as_ref().unwrap().0;
		self.queue.write_texture(
			tex.as_image_copy(),
			bytemuck::cast_slice(px),
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(rows) },
			wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 },
		);
	}

	/// Whether a retail overlay image has arrived.
	pub(super) fn has_retail_overlay(&self) -> bool {
		self.retail_ov.is_some()
	}






	/// Registers the debris classes: unposed vertices and faces by texture slot per class, and the per-frame
	/// instance buffers sized for the most pieces the moby pool can hold (`MOBY_LIMIT`) of the largest class.
	pub(super) fn load_debris(&mut self, classes: &[u32], prop_meshes: &HashMap<u32, Mesh>, textures: &HashMap<u32, Texture>) {
		let Some(mut scene) = self.scene.take() else { return };
		let mut model = HashMap::new();
		let (mut max_v, mut max_i) = (0, 0);
		for &c in classes {
			let Some(m) = prop_meshes.get(&c) else { continue };
			let mut slots = HashMap::new();
			let (_, _, _, verts) = self.batch(&[m], None, false, textures, &mut scene.bind_groups, &mut slots);
			let mut by_tex: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
			let mut phases: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
			let phased = m.strip_phase.len() == m.faces.len();
			for (fi, &(tex, f)) in m.faces.iter().enumerate() {
				let slot = slots[&(tex & crate::mobies::KEY_MASK)];
				by_tex.entry(slot).or_default().extend(f);
				phases.entry(slot).or_default().push(if phased { m.strip_phase[fi] } else { crate::mesh::NO_PHASE });
			}
			max_v = max_v.max(verts.len());
			max_i = max_i.max(m.faces.len() * 3);
			model.insert(c, (verts, by_tex.into_iter().collect::<Vec<_>>(), phases.into_iter().collect::<Vec<_>>()));
		}
		let cap = (max_v * particles::MOBY_LIMIT, max_i * particles::MOBY_LIMIT);
		scene.debris = Some(Batch {
			vbuf: buffer(&self.device, "debris vertices", &vec![0u8; cap.0 * std::mem::size_of::<Vertex>()], wgpu::BufferUsages::VERTEX),
			ibuf: buffer(&self.device, "debris indices", &vec![0u8; cap.1 * 4], wgpu::BufferUsages::INDEX),
			draws: Vec::new(),
		});
		scene.debris_model = model;
		scene.debris_cap = cap;
		self.scene = Some(scene);
	}

	/// BRIDGE-35: registers the warp bundle's ship classes. Their faces use moby texture keys 0..2, which the level's
	/// own textures also use, so they get their own texture map and fresh bind slots appended to the scene's list.
	pub(super) fn load_warp_actors(&mut self, assets: &crate::scene::WarpAssets) {
		let Some(mut scene) = self.scene.take() else { return };
		let textures: HashMap<u32, Texture> = assets
			.moby_textures
			.iter()
			.enumerate()
			.filter_map(|(k, t)| t.as_ref().map(|t| (crate::level::texture_key(crate::level::TextureGroup::Moby, k as u32), Texture { w: t.w, h: t.h, rgba: t.rgba.clone() })))
			.collect();
		let mut model = HashMap::new();
		let (mut max_v, mut max_i) = (0, 0);
		for (id, class, _) in &assets.classes {
			let mut slots = HashMap::new();
			let (_, _, _, verts) = self.batch(&[&class.mesh], None, false, &textures, &mut scene.bind_groups, &mut slots);
			let mut by_tex: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
			let mut phases: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
			let phased = class.mesh.strip_phase.len() == class.mesh.faces.len();
			for (fi, &(tex, f)) in class.mesh.faces.iter().enumerate() {
				let slot = slots[&(tex & crate::mobies::KEY_MASK)];
				by_tex.entry(slot).or_default().extend(f);
				phases.entry(slot).or_default().push(if phased { class.mesh.strip_phase[fi] } else { crate::mesh::NO_PHASE });
			}
			max_v = max_v.max(verts.len());
			max_i = max_i.max(class.mesh.faces.len() * 3);
			model.insert(*id, (verts, by_tex.into_iter().collect::<Vec<_>>(), phases.into_iter().collect::<Vec<_>>()));
		}
		// One actor per stream and class, a few spare for the segment change.
		let n = 4;
		scene.warp_batch = Some(Batch {
			vbuf: buffer(&self.device, "warp vertices", &vec![0u8; max_v * n * std::mem::size_of::<Vertex>()], wgpu::BufferUsages::VERTEX),
			ibuf: buffer(&self.device, "warp indices", &vec![0u8; max_i * n * 4], wgpu::BufferUsages::INDEX),
			draws: Vec::new(),
		});
		scene.warp_model = model;
		self.scene = Some(scene);
	}

	/// BRIDGE-35: this frame's posed warp actors (class, skinned positions and normals, actor position); `hidden`
	/// draws nothing (the quiet warp hides the ship). Turns the level draw off until `clear_warp_actors`.
	/// `lights[i]`: actor i's (ambient bytes, light word) from retail RAM (retail mirror); missing = the spawn ambient.
	pub(super) fn set_warp_actors(&mut self, actors: &[(u32, Posed, [f32; 3])], hidden: bool, lights: &[Option<([u8; 3], u32)>]) {
		let Some(mut scene) = self.scene.take() else { return };
		scene.warp_shown = true;
		if let Some(batch) = &mut scene.warp_batch {
			let (mut verts, mut by_tex): (Vec<Vertex>, BTreeMap<usize, Vec<u32>>) = (Vec::new(), BTreeMap::new());
			for (ai, (class, (ps, ns), pos)) in actors.iter().enumerate().filter(|_| !hidden) {
				let ram = lights.get(ai).copied().flatten();
				let light = Self::moby_light(ram);
				let Some((model, faces, _)) = scene.warp_model.get(class) else { continue };
				// Retail mirror: the ship is a lit moby (REN-5c: ambient + light pair, vertex colour bytes 0x80), so the
				// vertices take the moby code (id 0 = no dynamic light, full alpha), not the mesh's baked -2 colours.
				let lit_moby = ram.is_some();
				let base = verts.len() as u32;
				if std::env::var_os("RAC_SCENE_LOG").is_some() {
					eprintln!("retail: warp actor {ai} class {class:#x} model verts {} skinned {} normals {}", model.len(), ps.len(), ns.len());
				}
				for (i, v) in model.iter().enumerate() {
					let mut v = *v;
					if let (Some(p), Some(n)) = (ps.get(i), ns.get(i)) {
						v.pos = [p[0] + pos[0], p[1] + pos[1], p[2] + pos[2]];
						v.normal = *n;
					}
					v.light = light;
					if lit_moby {
						v.colour = [1.0, 1.0, 1.0, moby_alpha_encoding(0.0, 0x80).0];
					}
					verts.push(v);
				}
				for (t, idx) in faces {
					by_tex.entry(*t).or_default().extend(idx.iter().map(|i| i + base));
				}
			}
			let (mut indices, mut draws) = (Vec::new(), Vec::new());
			for (texture, idx) in by_tex {
				draws.push(Draw { inst: 0, cull: 0, texture, start: indices.len() as u32, count: idx.len() as u32 });
				indices.extend(idx);
			}
			if !verts.is_empty() {
				self.queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(&verts));
				self.queue.write_buffer(&batch.ibuf, 0, bytemuck::cast_slice(&indices));
			}
			batch.draws = draws;
		}
		self.scene = Some(scene);
	}

	/// BRIDGE-35: the warp is over; the level draws again.
	pub(super) fn clear_warp_actors(&mut self) {
		if let Some(scene) = &mut self.scene {
			scene.warp_shown = false;
			scene.warp_batch = None;
			scene.warp_model.clear();
		}
	}

	/// ARI-49b (BRIDGE-63, decomp/notes/aridia_presentation.md): this frame's Aridia ocean patches (pure: camera, region
	/// cuboids, the 0x15f5e8 override `all`) and, when the game frame `frames` changed, the cross-faded 64x64 texture.
	/// `level_fog` = Visibomb live (0x15f458 == 1): the level fog words and colour stay instead of the ocean's.
	pub(super) fn set_ocean(&mut self, ocean: Option<&crate::ocean::Ocean>, eye: [f32; 3], cuboids: &[gameplay::Cuboid], all: bool, frames: u32, level_fog: bool) {
		let Some(ocean) = ocean else {
			self.ocean_draws.clear();
			self.ocean_frame = None;
			return;
		};
		self.ocean_draws = ocean.patches(eye, cuboids, all);
		self.ocean_level_fog = level_fog;
		self.ocean_fog = ocean.fog;
		if self.ocean_frame == Some(frames) && self.ocean_tex.is_some() {
			return;
		}
		self.ocean_frame = Some(frames);
		self.ocean_rgba = ocean.texture(frames);
		let size = wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 };
		if self.ocean_tex.is_none() {
			let tex = self.device.create_texture(&wgpu::TextureDescriptor {
				label: Some("ocean"),
				size,
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: wgpu::TextureFormat::Rgba8Unorm,
				usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
				view_formats: &[],
			});
			let view = tex.create_view(&Default::default());
			// [I] repeat + bilinear: the packet leaves CLAMP_1/TEX1_1 at the frame's values; ST spans -0.5..0.5 per patch.
			let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("ocean"),
				layout: &self.tex_layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
					wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
				],
			});
			self.ocean_tex = Some((tex, bind));
		}
		let Some((tex, _)) = &self.ocean_tex else { return };
		self.queue.write_texture(
			tex.as_image_copy(),
			bytemuck::cast_slice(&self.ocean_rgba),
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(64 * 4), rows_per_image: Some(64) },
			size,
		);
	}

	/// Retail mode: the retail oceans of Rilgar, Umbris and Gaspar (retail/oldsim.rs `RamOcean`): this field's
	/// patch strips (camera distance cull), the texture and the fog packed per vertex for `fs_ocean_g`.
	pub(super) fn set_ocean_ram(&mut self, oceans: &[crate::retail::oldsim::RamOcean], eye: [f32; 3]) {
		self.ocean_ram.truncate(oceans.len());
		for (k, o) in oceans.iter().enumerate() {
			let strips = crate::ocean::ram_strips(&o.desc, &o.st, &o.idx, o.fix, eye);
			let n = [o.fog[0], o.fog[1], 0.0];
			let l = [o.fog[2], o.fog[3], o.fogcol as f32, o.level_fog as u8 as f32];
			let (mut verts, mut idx) = (Vec::new(), Vec::new());
			for s in strips.iter().filter(|s| s.pos.len() >= 3) {
				let base = verts.len() as u32;
				verts.extend((0..s.pos.len()).map(|j| Vertex { pos: s.pos[j], uv: s.uv[j], normal: n, colour: s.colour[j], light: l }));
				idx.extend((0..s.pos.len() as u32 - 2).flat_map(|t| [base + t, base + t + 1, base + t + 2]));
			}
			let (mut pverts, mut pidx) = (Vec::new(), Vec::new());
			for p in o.patches.iter().filter(|p| p.pos.len() >= 3) {
				let base = pverts.len() as u32;
				pverts.extend((0..p.pos.len()).map(|j| Vertex { pos: p.pos[j], uv: p.uv[j], normal: n, colour: o.patch_colour, light: l }));
				pidx.extend((0..p.pos.len() as u32 - 2).flat_map(|t| [base + t, base + t + 1, base + t + 2]));
			}
			let size = wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 };
			if self.ocean_ram.len() <= k {
				let tex = self.device.create_texture(&wgpu::TextureDescriptor {
					label: Some("ocean ram"),
					size,
					mip_level_count: 1,
					sample_count: 1,
					dimension: wgpu::TextureDimension::D2,
					format: wgpu::TextureFormat::Rgba8Unorm,
					usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
					view_formats: &[],
				});
				let view = tex.create_view(&Default::default());
				let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
					label: Some("ocean ram"),
					layout: &self.tex_layout,
					entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) }],
				});
				self.ocean_ram.push(OceanBatch { tex, bind, verts: Vec::new(), idx: Vec::new(), pverts: Vec::new(), pidx: Vec::new(), fix: o.fix });
			}
			let b = &mut self.ocean_ram[k];
			(b.verts, b.idx, b.pverts, b.pidx, b.fix) = (verts, idx, pverts, pidx, o.fix);
			self.queue.write_texture(b.tex.as_image_copy(), bytemuck::cast_slice(&o.rgba), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(64 * 4), rows_per_image: Some(64) }, size);
		}
	}

	/// NOV-31: poses the sky shells for game frame `frames` on level `level` (the loaded level index): a shell with a
	/// `sky::shell_angle` turns about z by it, every other shell keeps its loaded positions. Rewrites the sky vertices
	/// only when the pose changes (once per game tick at most).
	pub(super) fn set_sky_spin(&mut self, level: usize, frames: u32) {
		let Some(scene) = &mut self.scene else { return };
		let (Some(batch), Some(s)) = (&scene.sky, &mut scene.sky_shells) else { return };
		if s.posed == Some((level, frames)) {
			return;
		}
		s.posed = Some((level, frames));
		let mut changed = false;
		for (k, (range, base)) in s.ranges.iter().zip(&s.base).enumerate() {
			let angle = sky::shell_angle(level, k, frames);
			let pos = match angle {
				Some(a) => sky::fan_positions(base, particles::euler_m([0.0, 0.0, a]), [0.0; 3]),
				None if s.turned[k] => base.clone(),
				None => continue,
			};
			s.turned[k] = angle.is_some();
			for (v, p) in s.verts[range.clone()].iter_mut().zip(pos) {
				v.pos = p;
			}
			changed = true;
		}
		if changed {
			self.queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(&s.verts));
		}
	}

	/// ARI-4: this frame's sky sprites from the held records (pure, no rand()): `view` rows are the camera right, up and forward
	/// axes, `tan_x` the port camera's horizontal half-angle tangent ([PC] widescreen x cull; retail 0.63 at 4:3).
	pub(super) fn set_sky_sprites(&mut self, sprites: &[sky::SkySprite], view: [[f32; 3]; 3], right: [f32; 3], up: [f32; 3], tan_x: f32) {
		let Some(scene) = &mut self.scene else { return };
		let Some(d) = &mut scene.sky_sprites else { return };
		let draws = sky::sprite_draws(sprites, view, right, up, tan_x, (1.0 + tan_x * tan_x).sqrt());
		let mut verts = Vec::with_capacity(draws.len() * 4);
		d.counts = [0; 2];
		// Grouped by texture (additive blending commutes).
		for (k, t) in [2u8, 3].into_iter().enumerate() {
			for (_, quad, colour) in draws.iter().filter(|q| q.0 == t) {
				verts.extend(quad.iter().map(|&(pos, uv)| Vertex { pos, uv, normal: [0.0; 3], colour: *colour, light: [0.0; 4] }));
				d.counts[k] += 1;
			}
		}
		if !verts.is_empty() {
			self.queue.write_buffer(&d.vbuf, 0, bytemuck::cast_slice(&verts));
		}
	}


	/// [PC] Retail pool growth (retail.rs `sync` / `take_grow`): rebuilds only the loaded scene's prop slots for the new
	/// slot order, with the textures the scene already uploaded, so the level, its audio and the other batches stay as
	/// they are. The slots come out as `load_scene` would build them for `props`. False (scene untouched) when there is
	/// no scene or a slot needs a texture the scene never uploaded; the caller then reloads the level.
	pub(super) fn regrow_props(&mut self, props: &[u32], prop_meshes: &HashMap<u32, Mesh>) -> bool {
		let Some(scene) = self.scene.as_ref() else { return false };
		let empty = Mesh::new(String::new());
		let known = slot_meshes(props, prop_meshes, &empty).iter().all(|m| m.faces.iter().all(|&(t, _)| scene.tex_slots.contains_key(&(t & crate::mobies::KEY_MASK))));
		let had_prompt = scene.prompt.is_some();
		if !known || (!had_prompt && props.contains(&game::vendor::PROMPT_CLASS) && prop_meshes.get(&game::vendor::PROMPT_CLASS).is_some_and(|m| !m.positions.is_empty())) {
			return false;
		}
		let mut slots = std::mem::take(&mut self.scene.as_mut().unwrap().tex_slots);
		let mut bind = Vec::new();
		let p = self.prop_slots(props, prop_meshes, &HashMap::new(), &mut bind, &mut slots);
		debug_assert!(bind.is_empty());
		let scene = self.scene.as_mut().unwrap();
		scene.tex_slots = slots;
		let n = p.ranges.len();
		scene.props_verts = if self.props_gpu.is_some() { Vec::new() } else { p.model.clone() };
		scene.props_hidden = vec![false; n];
		scene.props_key = vec![None; n];
		scene.props = p.batch;
		scene.props_model = p.model;
		scene.props_sj = p.sj;
		scene.props_rad = p.rad;
		scene.props_inst = Vec::new();
		scene.props_pose = Vec::new();
		scene.props_ranges = p.ranges;
		scene.props_shown = false;
		scene.props_alpha = false;
		scene.props_drawn = Vec::new();
		scene.props_vis = Vec::new();
		scene.slot_runs = p.slot_runs;
		scene.props_order = (0..p.classes.len()).collect();
		scene.props_class = p.classes;
		scene.class_faces.extend(p.class_faces);
		scene.props_late = Vec::new();
		scene.prop_faces = p.faces;
		scene.clip_masked = Vec::new();
		scene.clip_view = None;
		scene.clip_queue = Vec::new();
		scene.prompt_slot = p.prompt_slot.filter(|_| scene.prompt.is_some());
		true
	}

	/// [PC] Retail pool growth without a rebuild: one new prop slot per class in `classes`, inserted at slot `at` (after the
	/// game's props, before the scene actors); `order` is the new slot order the faces are drawn in (a rebuilt scene's:
	/// each class's new slots after its old ones). The existing slots keep their vertices; the new slots' vertices go into
	/// the buffer's spare room after them, and the indices, draws, `slot_runs` and `prop_faces` are rebuilt for `order` as
	/// `load_scene` builds them for that slot order. False (scene untouched) when there is no scene, a
	/// slot needs a texture the scene never uploaded, or a class is the purchase prompt; the caller then rebuilds the slots.
	/// Short of room the prop buffers grow first (GPU copy of the old slots).
	pub(super) fn append_props(&mut self, at: usize, classes: &[u32], prop_meshes: &HashMap<u32, Mesh>, order: &[usize]) -> bool {
		let gpu_posing = self.props_gpu.is_some();
		let (device, queue, Some(scene)) = (&self.device, &self.queue, self.scene.as_mut()) else { return false };
		let empty = Mesh::new(String::new());
		let meshes = slot_meshes(classes, prop_meshes, &empty);
		let n = scene.props_ranges.len() + classes.len();
		if at > scene.props_ranges.len() || scene.props_class.len() != scene.props_ranges.len() || order.len() != n || classes.contains(&game::vendor::PROMPT_CLASS) || !meshes.iter().all(|m| m.faces.iter().all(|&(t, _)| scene.tex_slots.contains_key(&(t & crate::mobies::KEY_MASK)))) {
			return false;
		}
		let mut all_classes = scene.props_class.clone();
		all_classes.splice(at..at, classes.iter().copied());
		let all_meshes = slot_meshes(&all_classes, prop_meshes, &empty);
		let n_idx: usize = all_meshes.iter().map(|m| m.faces.len() * 3).sum();
		let v0 = scene.props_model.len();
		let (mut verts, mut ranges) = (Vec::new(), Vec::new());
		let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
		for (mi, m) in meshes.iter().enumerate() {
			let base = v0 + verts.len();
			push_mesh_vertices(m, mi, None, &mut verts, &mut lo, &mut hi);
			ranges.push(base..v0 + verts.len());
		}
		// Room short: larger buffers (a 16th spare again), the old slots' vertices and (slot, vertex) attributes moved over by
		// a GPU copy; the indices are written whole below.
		let vsz = std::mem::size_of::<Vertex>();
		let need = v0 + verts.len();
		let vcap = (scene.props.vbuf.size() as usize / vsz).min(scene.props_sj.as_ref().map_or(usize::MAX, |b| b.size() as usize / 8));
		if need > vcap || n_idx > scene.props.ibuf.size() as usize / 4 {
			let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("props grow") });
			if need > vcap {
				let cap = need + (need / 16).max(1 << 16);
				let vbuf = sized_buffer(device, "vertices", cap * vsz, wgpu::BufferUsages::VERTEX);
				enc.copy_buffer_to_buffer(&scene.props.vbuf, 0, &vbuf, 0, (v0 * vsz) as u64);
				scene.props.vbuf = vbuf;
				if let Some(old) = scene.props_sj.as_ref() {
					let sj = sized_buffer(device, "props slot attribute", cap * 8, wgpu::BufferUsages::VERTEX);
					enc.copy_buffer_to_buffer(old, 0, &sj, 0, (v0 * 8) as u64);
					scene.props_sj = Some(sj);
				}
			}
			if n_idx > scene.props.ibuf.size() as usize / 4 {
				scene.props.ibuf = sized_buffer(device, "indices", (n_idx + (n_idx / 16).max(3 << 16)) * 4, wgpu::BufferUsages::INDEX);
			}
			queue.submit(Some(enc.finish()));
		}
		let mut all_ranges = scene.props_ranges.clone();
		all_ranges.splice(at..at, ranges.iter().cloned());
		// Each class's faces by texture key and its clippable faces (kept in the scene: the class meshes never change).
		for (&c, &m) in all_classes.iter().zip(&all_meshes) {
			scene.class_faces.entry(c).or_insert_with(|| ClassFaces::of(m));
		}
		let mut count: BTreeMap<u32, u32> = BTreeMap::new();
		for c in &all_classes {
			for (key, f) in &scene.class_faces[c].groups {
				*count.entry(*key).or_default() += f.len() as u32;
			}
		}
		// One draw per texture key in ascending order (`batch`); `pos` = each draw's next free index.
		let mut draws = Vec::with_capacity(count.len());
		let mut draw_of: HashMap<u32, usize> = HashMap::with_capacity(count.len());
		let (mut pos, mut start) = (Vec::with_capacity(count.len()), 0);
		for (d, (&key, &c)) in count.iter().enumerate() {
			draws.push(Draw { inst: 0, cull: ((key >> 24) & 3) as u8, texture: scene.tex_slots[&(key & crate::mobies::KEY_MASK)], start, count: c });
			draw_of.insert(key, d);
			pos.push(start);
			start += c;
		}
		// Per slot (in `order`) its runs; per draw the (vertex base, faces) it holds in order.
		let mut slot_runs: Vec<Vec<(u32, u32, u32)>> = vec![Vec::new(); n];
		let mut per_draw: Vec<Vec<(u32, &[u32])>> = vec![Vec::new(); draws.len()];
		for &s in order {
			let base = all_ranges[s].start as u32;
			for (key, f) in &scene.class_faces[&all_classes[s]].groups {
				let d = draw_of[key];
				slot_runs[s].push((d as u32, pos[d], f.len() as u32));
				per_draw[d].push((base, f));
				pos[d] += f.len() as u32;
			}
		}
		let mut indices = vec![0u32; n_idx];
		{
			let mut rest: &mut [u32] = &mut indices;
			let mut outs = Vec::with_capacity(draws.len());
			for d in &draws {
				let (a, b) = rest.split_at_mut(d.count as usize);
				outs.push(a);
				rest = b;
			}
			outs.into_par_iter().zip(per_draw.par_iter()).for_each(|(out, segs)| {
				let mut k = 0;
				for &(base, f) in segs {
					for (o, &i) in out[k..k + f.len()].iter_mut().zip(f) {
						*o = i + base;
					}
					k += f.len();
				}
			});
		}
		// NOV-5.9c: the clippable faces' index positions (`clip::replay_faces`); the old slots' faces only move.
		let m = classes.len();
		let mut prop_faces = std::mem::take(&mut scene.prop_faces);
		prop_faces.splice(at..at, (0..m).map(|_| Vec::new()));
		let (cf, tex_slots) = (&scene.class_faces, &scene.tex_slots);
		prop_faces.par_iter_mut().enumerate().for_each(|(s, faces)| {
			let cf = &cf[&all_classes[s]];
			let first = &slot_runs[s];
			let base = all_ranges[s].start as u32;
			if faces.len() == cf.clipped.len() && (s < at || s >= at + m) {
				for (f, &(g, rank, _, _)) in faces.iter_mut().zip(&cf.clipped) {
					f.slot = first[g].1 + rank * 3;
				}
			} else {
				*faces = cf.clipped.iter().map(|&(g, rank, idx, phase)| {
					let key = cf.groups[g].0;
					super::clip::PropFace { slot: first[g].1 + rank * 3, idx: idx.map(|i| i + base), phase, tex: tex_slots.get(&(key & crate::mobies::KEY_MASK)).copied().unwrap_or(0), cull: ((key >> 24) & 3) as u8 }
				}).collect();
			}
		});
		queue.write_buffer(&scene.props.vbuf, (v0 * std::mem::size_of::<Vertex>()) as u64, bytemuck::cast_slice(&verts));
		queue.write_buffer(&scene.props.ibuf, 0, bytemuck::cast_slice(&indices));
		if let Some(sj) = &scene.props_sj {
			// The new slots' (slot, vertex) attribute, and the scene actors' slot numbers moved up by `m`.
			let attr = |s: usize, r: &std::ops::Range<usize>| (0..r.len() as u32).map(|j| [s as u32, j]).collect::<Vec<[u32; 2]>>();
			for (k, r) in ranges.iter().enumerate().filter(|(_, r)| !r.is_empty()) {
				queue.write_buffer(sj, r.start as u64 * 8, bytemuck::cast_slice(&attr(at + k, r)));
			}
			for (s, r) in scene.props_ranges.iter().enumerate().skip(at).filter(|(_, r)| !r.is_empty()) {
				queue.write_buffer(sj, r.start as u64 * 8, bytemuck::cast_slice(&attr(s + m, r)));
			}
		}
		if gpu_posing {
			let rad: Vec<f32> = ranges.iter().map(|r| verts[r.start - v0..r.end - v0].iter().map(|v| (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1] + v.pos[2] * v.pos[2]).sqrt()).fold(0.0, f32::max)).collect();
			scene.props_rad.splice(at..at, rad);
		} else {
			scene.props_verts.extend_from_slice(&verts);
		}
		scene.props.draws = draws;
		scene.slot_runs = slot_runs;
		scene.prop_faces = prop_faces;
		scene.props_model.extend(verts);
		scene.props_ranges = all_ranges;
		scene.props_class = all_classes;
		scene.props_order = order.to_vec();
		scene.props_hidden.splice(at..at, vec![false; m]);
		scene.props_key.splice(at..at, vec![None; m]);
		// The indices were written whole: nothing is masked for the clipper (as after a rebuild).
		scene.clip_masked = Vec::new();
		scene.prompt_slot = scene.prompt_slot.map(|p| if p >= at { p + m } else { p });
		for pi in scene.props_late.iter_mut().filter(|pi| **pi >= at) {
			*pi += m;
		}
		if scene.props_vis.len() >= at {
			scene.props_vis.splice(at..at, vec![false; m]);
		}
		true
	}

	pub(super) fn load_scene(&mut self, meshes: &[Mesh], shrub_set: &[shrubs::ShrubInstance], tie_set: &[ties::LitTie], textures: &HashMap<u32, Texture>, sky: Option<&sky::Sky>, hero: Option<&Mesh>, hero_tint: Option<([f32; 3], usize, usize)>, props: &[u32], prop_meshes: &HashMap<u32, Mesh>, preview_meshes: &HashMap<u32, Mesh>, occ: crate::occlusion::Occlusion, tfrag_set: Vec<terrain::TfragLods>, tfrag_b: f32, terrain_chains: &[Option<Vec<Texture>>], shrub_chains: &[Option<Vec<Texture>>], tie_chains: &[Option<Vec<Texture>>]) {
		let order = props.to_vec();
		let scene = self.ctx().build_scene(meshes, shrub_set, tie_set, textures, sky, hero, hero_tint, move |_| Some(order.clone()), prop_meshes, preview_meshes, occ, tfrag_set, tfrag_b, terrain_chains, shrub_chains, tie_chains);
		self.scene = scene;
	}

	/// Poses the player model at `feet`, turned to `yaw` (None hides it); `skinned` replaces the
	/// model-space positions and normals with an animated pose.
	/// Re-poses hero preview `key` (model-space positions and normals, one per vertex of its mesh).
	pub(super) fn set_preview_pose(&mut self, key: u32, ps: &[[f32; 3]], ns: &[[f32; 3]]) {
		let Some(scene) = &self.scene else { return };
		let (Some(model), Some((batch, ..))) = (scene.preview_models.get(&key), scene.previews.get(&key)) else { return };
		if model.len() != ps.len() || ns.len() != ps.len() {
			return;
		}
		let verts: Vec<Vertex> = model.iter().zip(ps.iter().zip(ns)).map(|(v, (p, n))| Vertex { pos: *p, normal: *n, ..*v }).collect();
		self.queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(&verts));
	}

	/// NOV-30d (1)/(3): the shop panel-1 item A (slot +0x100) is an animated moby (seq 0, then seq 1 when the class has
	/// more, stepped every tick) that retail draws skinned into the item target; `posed` = (class, model-space positions,
	/// normals) at its retail clock. The class posed before gets its bind vertices back when the item changes or goes.
	pub(super) fn pose_panel_item(&mut self, posed: Option<(u32, Vec<[f32; 3]>, Vec<[f32; 3]>)>) {
		let key = posed.as_ref().map(|p| p.0);
		if let (Some(prev), Some(scene)) = (self.panel_posed.filter(|&c| Some(c) != key), &self.scene) {
			if let (Some(model), Some((batch, ..))) = (scene.preview_models.get(&prev), scene.previews.get(&prev)) {
				self.queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(model));
			}
		}
		self.panel_posed = key;
		if let Some((c, ps, ns)) = posed {
			self.set_preview_pose(c, &ps, &ns);
		}
	}

	/// One frame of the dynamic lights on the world: follow the light table, then rewrite the colours of the visible
	/// ties, shrubs and tfrags that hold a light or just lost one (`dynlight::LitWorld`, lighting_fog.md 5b).
	pub(super) fn relight(&mut self, view: Option<&dynlight::View>) {
		let _s = super::profile::span("relight");
		let Some(scene) = &mut self.scene else { return };
		for r in scene.lit.frame(&self.table, view) {
			if r.kind == crate::mesh::LitKind::Tie {
				// BRIDGE-32: a tie returns its 64 slot colours; the vertices of whichever LOD is drawn are recoloured from
				// them in `cull_ties` (its Q picks the blend).
				scene.tie_cols[r.at] = r.colours;
				scene.tie_stale[r.at] = [true; 3];
				continue;
			}
			if r.kind == crate::mesh::LitKind::Tfrag {
				// REN-CULL-TFRAG: per position record colours; `cull_tfrags` recolours the drawn LOD from them (after the morph).
				scene.tfrag_cols[r.at] = r.colours;
				continue;
			}
			let (space, first) = relight_space(r.first, scene.tie_first, scene.shrub_first);
			let (verts, buf) = match space {
				VertexSpace::Shrub => (&mut scene.shrub_verts, &scene.shrubs.vbuf),
				VertexSpace::Tie => (&mut scene.tie_verts, &scene.ties.vbuf),
				VertexSpace::World => (&mut scene.world_verts, &scene.world.vbuf),
			};
			for (k, c) in r.colours.iter().enumerate() {
				// A shrub keeps the alpha carrier of its current fade (colour w), world and tie vertices stay -2.
				verts[first + k].colour = [c[0] / 128.0, c[1] / 128.0, c[2] / 128.0, verts[first + k].colour[3]];
			}
			let range = first..first + r.colours.len();
			self.queue.write_buffer(buf, (first * std::mem::size_of::<Vertex>()) as u64, bytemuck::cast_slice(&verts[range]));
		}
	}

	/// BRIDGE-29 / REN-CULL-TIE, once per drawn frame before the scene is borrowed: per tie instance the LOD and geomorph
	/// factor from the view depth of its sphere centre (`ties::tie_pick`; with no clip view every tie draws at LOD 0 unmorphed
	/// [I]), the picked LOD's draws rebuilt into `scene.ties.draws`, and the picked range's positions rewritten when its Q
	/// changed. A tie whose occlusion bytes fail the frame's mask (REN-CULL-OCC, skipped with no clip view) is not drawn.
	pub(super) fn cull_ties(&mut self) {
		let _s = super::profile::span("cull_ties");
		let Some(scene) = &mut self.scene else { return };
		if scene.tie_cull.is_empty() {
			return;
		}
		let mut draws = Vec::new();
		// [PC] perf: the vertex rewrites (tie, LOD, Q, moved, all) run on the frame worker pool after the pick loop.
		let mut jobs: Vec<(usize, usize, f32, bool, bool)> = Vec::new();
		for i in 0..scene.tie_cull.len() {
			let c = scene.tie_cull[i];
			let pick = match &scene.clip_view {
				Some(v) => ties::tie_pick(&c, v.view(c.centre)[2]),
				None => Some((0, 0.0)),
			};
			let Some((lod, q)) = pick else { continue };
			if scene.clip_view.is_some() && !scene.occ.passes(c.occ) {
				continue;
			}
			draws.extend_from_slice(&scene.tie_draws[scene.tie_ranges[i][lod].start as usize..scene.tie_ranges[i][lod].end as usize]);
			// Positions follow Q for fat vertices; colours follow Q too, and a relight leaves every range stale (BRIDGE-32).
			let moved = scene.tie_fat[i][lod] && q != scene.tie_q[i][lod];
			if moved || scene.tie_stale[i][lod] {
				let all = scene.tie_stale[i][lod] || !scene.tie_full[i][lod];
				jobs.push((i, lod, q, moved, all));
				scene.tie_q[i][lod] = q;
				scene.tie_stale[i][lod] = false;
				scene.tie_full[i][lod] = true;
			}
		}
		{
			let (base, morph, slots, cols, qdep, tri, tris) = (&scene.tie_base, &scene.tie_morph, &scene.tie_slots, &scene.tie_cols, &scene.tie_qdep, &scene.tie_tri, &scene.tie_tris);
			let rewrite = |verts: &mut [Vertex], at: usize, (i, lod, q, moved, all): (usize, usize, f32, bool, bool)| {
				// [PC] perf round 3: the slot products once per piece (`ties::TieColours`, same bytes as `tie_colour`), and each
				// distinct fat-vertex slot triple of the LOD blended once (`tie_triples`).
				let tc = ties::TieColours::new(&cols[i], q);
				let fat: Vec<[f32; 4]> = tris[i][lod].iter().map(|&t| tc.colour((t, true))).collect();
				for (d, v) in verts.iter_mut().enumerate() {
					let k = at + d;
					if !all && !qdep[k] {
						continue;
					}
					if moved {
						v.pos = ties::morphed(base[k], morph[k], q);
					}
					let c = match slots[k].1 {
						true => fat[tri[k] as usize],
						false => tc.colour(slots[k]),
					};
					v.colour = [c[0] / 128.0, c[1] / 128.0, c[2] / 128.0, -2.0];
				}
			};
			let ranges: Vec<std::ops::Range<usize>> = jobs.iter().map(|&(i, lod, ..)| scene.tie_vranges[i][lod].clone()).collect();
			par_chunked(&mut scene.tie_verts, &ranges, |k, off, verts| rewrite(verts, ranges[k].start + off, jobs[k]));
			for vr in ranges {
				self.queue.write_buffer(&scene.ties.vbuf, (vr.start * std::mem::size_of::<Vertex>()) as u64, bytemuck::cast_slice(&scene.tie_verts[vr]));
			}
		}
		scene.ties.draws = draws;
	}

	/// BRIDGE-27 / REN-CULL-SHRUB, first thing in `draw`: per shrub instance (class load-slot order) the view depth of its sphere
	/// centre gives culled / opaque / faded (`ShrubCull::state`); a changed fade alpha is rewritten into the instance's vertex
	/// colours, the draw lists are rebuilt and the far-LOD billboards of the frame are queued (pass 1 before pass 2).
	pub(super) fn cull_shrubs(&mut self, cam: &Camera) {
		let _s = super::profile::span("cull_shrubs");
		let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
		let cap = self.shrub_cap;
		let Some(scene) = &mut self.scene else { return };
		scene.shrub_opaque.clear();
		scene.shrub_faded.clear();
		scene.shrub_bb_quads.clear();
		if scene.shrub_cull.is_empty() {
			return;
		}
		let cv = super::clip::ClipView::new(cam, aspect);
		let (mut pass1, mut pass2) = (Vec::new(), Vec::new());
		for k in 0..scene.shrub_order.len() {
			let i = scene.shrub_order[k];
			let c = scene.shrub_cull[i];
			let Some(st) = c.state_cap(cv.view(c.centre)[2], cap) else { continue };
			if let Some(a) = st.mesh {
				if let Some(w) = shrub_rewrite(scene.shrub_a[i], a) {
					let vr = scene.shrub_ranges[i].1.clone();
					for v in &mut scene.shrub_verts[vr.clone()] {
						v.colour[3] = w;
					}
					self.queue.write_buffer(&scene.shrubs.vbuf, (vr.start * std::mem::size_of::<Vertex>()) as u64, bytemuck::cast_slice(&scene.shrub_verts[vr]));
					scene.shrub_a[i] = a;
				}
				// Unfaded meshes blend their texel alpha too: retail's partial-alpha grass patches are soft and faint
				// (PCSX2 Eudora capture s07, 2026-10-07) [I], so every mesh takes the alpha path with A = 0x80.
				scene.shrub_faded.extend(scene.shrub_ranges[i].0.clone());
			}
			if st.bb == 0 {
				continue;
			}
			let Some(b) = &scene.shrub_bb[i] else { continue };
			let Some(&(_, mxl)) = scene.shrub_bb_tex.get(&b.class_id) else { continue };
			let q = b.corners(cam.pos);
			let colour = [b.colour[0] as f32 / 128.0, b.colour[1] as f32 / 128.0, b.colour[2] as f32 / 128.0, shrub_carrier(st.bb)];
			let quad = ShrubQuad { pos: [q[0], q[1], q[3], q[2]], colour, class_id: b.class_id, lod_k: b.lod_k, mxl: mxl as f32, pass1: st.bb & 0x80 != 0 };
			if quad.pass1 {
				pass1.push(quad);
			} else {
				pass2.push(quad);
			}
		}
		scene.shrub_bb_quads = pass1;
		scene.shrub_bb_quads.extend(pass2);
	}

	/// REN-CULL-SHRUB-BBTEX: the billboard texture chain of each shrub class (key = class id), uploaded level by level with the
	/// clamping sampler; the mip count - 1 goes to the shader as the top level.
	pub(super) fn set_shrub_bb_textures(&mut self, chains: &HashMap<u32, Vec<Texture>>) {
		let Some(mut scene) = self.scene.take() else { return };
		for (&class, chain) in chains.iter().filter(|(_, c)| !c.is_empty()) {
			scene.shrub_bb_tex.insert(class, (self.upload_chain(chain, &self.clamp_sampler), chain.len() as u32 - 1));
		}
		self.scene = Some(scene);
	}

	/// BRIDGE-31 / REN-CULL-TFRAG, once per drawn frame after `relight` and `set_clip_view`: per terrain fragment the occlusion
	/// mask, the 500 m cap, the box test (a fragment partly outside the guard band draws its near mesh unmorphed), the LOD
	/// code, and the picked mesh's morphed / collapsed vertices (`terrain::pose`), rewritten into `scene.tfrags.vbuf` only
	/// where they changed. With no clip view every fragment draws its plain near mesh [I].
	pub(super) fn cull_tfrags(&mut self) {
		let _s = super::profile::span("cull_tfrags");
		let fog = self.lighting.as_ref().map_or([0.0, 524288.0, 255.0, 0.0], |l| l.fog);
		let Some(scene) = &mut self.scene else { return };
		if scene.tfrag_set.is_empty() {
			return;
		}
		// FUN_001f7900: the clip-w depth scale (the fog slope per 1024 world units).
		let cc60 = (fog[3] - fog[2]) / ((fog[1] - fog[0]).max(0.001) / 1024.0);
		let (t, ints) = terrain::lod_distances(scene.tfrag_b);
		let mut draws = Vec::new();
		let mut st = [0u32; 4];
		// [PC] perf: (fragment, LOD code, LOD) of the drawn fragments; the posing and the vertex compare run on the frame worker pool.
		let mut jobs: Vec<(usize, u8, usize)> = Vec::new();
		for i in 0..scene.tfrag_set.len() {
			let l = &scene.tfrag_set[i];
			let c = &l.cull;
			if tfrag_probe_skip(i) {
				continue;
			}
			if tfrag_probe_info(i) {
				if let Some(v) = &scene.clip_view {
					let p = v.view(c.centre).map(|x| x * 1024.0);
					let r = c.radius * 1024.0;
					let flags: Vec<u8> = c.corners.iter().map(|&k| v.viewport().flags(v.view(k))).collect();
					let bands: Vec<u8> = c.corners.iter().map(|&k| v.band().flags(v.view(k))).collect();
					eprintln!(
						"TFINFO i={i} centre={:?} r={r} view_p={p:?} cap={} near={} occ={} inside={} flags={flags:?} band={bands:?} force_far={} lod_code={:#x} side_out_x={} side_out_y={}",
						c.centre,
						terrain::beyond_draw_cap(p[2], r),
						32.0 - (p[2] + r) >= 0.0,
						scene.occ.passes(c.occ),
						v.sphere_wholly_inside(p, r),
						c.force_far,
						terrain::lod_code(c.force_far, (p[2] - r) as i32, (p[2] + r) as i32, ints),
						0.63 * p[2] - (p[0].abs() - r * 1.1819053) < 0.0,
						0.47628 * p[2] - (p[1].abs() - r * 1.1076293) < 0.0
					);
				}
			}
			let code = match &scene.clip_view {
				None => 0x14,
				Some(v) => {
					let p = v.view(c.centre).map(|x| x * 1024.0);
					let r = c.radius * 1024.0;
					if terrain::beyond_draw_cap(p[2], r) { st[0] += 1; continue; }
					if 32.0 - (p[2] + r) >= 0.0 { st[1] += 1; continue; }
					if !scene.occ.passes(c.occ) { st[2] += 1; continue; }
					let mut near_only = false;
					if !v.sphere_wholly_inside(p, r) {
						let flags: Vec<u8> = c.corners.iter().map(|&k| v.viewport().flags(v.view(k))).collect();
						if flags.iter().fold(0x3f, |a, f| a & f) != 0 {
							st[3] += 1;
							continue;
						}
						near_only = c.corners.iter().any(|&k| v.band().flags(v.view(k)) != 0);
					}
					if near_only {
						0x14
					} else {
						terrain::lod_code(c.force_far, (p[2] - r) as i32, (p[2] + r) as i32, ints)
					}
				}
			};
			let lod = match code {
				6 => 0,
				8 | 0xa => 1,
				_ => 2,
			};
			draws.extend_from_slice(&scene.tfrag_draws[scene.tfrag_ranges[i][lod].start as usize..scene.tfrag_ranges[i][lod].end as usize]);
			jobs.push((i, code, lod));
		}
		if std::env::var_os("RAC_TFRAG_STATS").is_some() { eprintln!("TFSTATS total={} cap={} near={} occ={} frustum={} drawn={} occ_enabled={}", scene.tfrag_set.len(), st[0], st[1], st[2], st[3], jobs.len(), scene.occ.enabled); }
		let ranges: Vec<std::ops::Range<usize>> = jobs.iter().map(|&(i, _, lod)| scene.tfrag_vranges[i][lod].clone()).collect();
		let dirty = {
			let (set, cols, view) = (&scene.tfrag_set, &scene.tfrag_cols, scene.clip_view);
			par_ranges(&mut scene.tfrag_verts, &ranges, |j, verts| {
				let (i, code, _) = jobs[j];
				let posed = match &view {
					Some(v) => terrain::pose(&set[i], code, t, cc60, &cols[i], &|p| v.view(p)[2]),
					None => terrain::pose(&set[i], code, t, cc60, &cols[i], &|_| 0.0),
				};
				let coloured = !cols[i].is_empty();
				let mut changed = false;
				for (k, v) in verts.iter_mut().enumerate() {
					let colour = if coloured { [posed.col[k][0] / 128.0, posed.col[k][1] / 128.0, posed.col[k][2] / 128.0, -2.0] } else { v.colour };
					if v.pos != posed.pos[k] || v.uv != posed.uv[k] || v.colour != colour {
						(v.pos, v.uv, v.colour) = (posed.pos[k], posed.uv[k], colour);
						changed = true;
					}
				}
				changed
			})
		};
		for (vr, _) in ranges.into_iter().zip(&dirty).filter(|(_, d)| **d) {
			self.queue.write_buffer(&scene.tfrags.vbuf, (vr.start * std::mem::size_of::<Vertex>()) as u64, bytemuck::cast_slice(&scene.tfrag_verts[vr]));
		}
		scene.tfrags.draws = draws;
	}

	/// Vertex count of the marker model (Ratchet with the items appended): a posed hero must cover all of it (`set_marker`).
	pub(super) fn marker_len(&self) -> Option<usize> {
		self.scene.as_ref().map(|s| s.marker_model.len())
	}

	pub(super) fn set_marker(&mut self, at: Option<([f32; 3], f32)>, skinned: Option<&Posed>) {
		let _s = super::profile::span("set_marker");
		// The first posing call of a frame: the posed mobies' third lights start over.
		self.moby_dyn.clear();
		// The afterimage ranges are for this call only, whether or not the marker shows (BRIDGE-28).
		let ghosts = std::mem::take(&mut self.marker_ghosts);
		let bomb = self.marker_bomb.take();
		let tilt = std::mem::take(&mut self.marker_tilt);
		let clank_light = self.marker_light.take();
		let Some(scene) = &mut self.scene else { return };
		// NOV-5.9d: the clip queue is rebuilt every frame, marker polygons first.
		scene.clip_queue.clear();
		let Some((feet, yaw)) = at else {
			scene.marker_shown = false;
			scene.marker_alpha = false;
			return;
		};
		// The skinned pose replaces the model's positions and normals (PC perf: per vertex, into a reused buffer).
		let skin = skinned.filter(|(ps, _)| ps.len() == scene.marker_model.len());
		let (c, sn) = (yaw.cos(), yaw.sin());
		// BRIDGE-75: a tilted hero uses the full euler basis (rows of euler_m); untilted keeps the yaw-only turn.
		let rows = (tilt != [0.0; 2]).then(|| crate::particles::euler_m([tilt[0], tilt[1], yaw]));
		let turn = |v: [f32; 3]| match &rows {
			Some(r) => [
				v[0] * r[0][0] + v[1] * r[1][0] + v[2] * r[2][0],
				v[0] * r[0][1] + v[1] * r[1][1] + v[2] * r[2][1],
				v[0] * r[0][2] + v[1] * r[1][2] + v[2] * r[2][2],
			],
			None => [v[0] * c - v[1] * sn, v[0] * sn + v[1] * c, v[2]],
		};
		let (light, tint) = Self::hero_light_tint(&self.lighting, feet, self.hero_placed, &mut self.hero_env);
		self.hero_tint_out = Some(tint);
		let id = dyn_id(&self.table, &mut self.moby_dyn, feet);
		let shift = self.marker_v.take();
		let mut verts = std::mem::take(&mut scene.marker_verts);
		verts.clear();
		verts.extend(scene.marker_model
			.iter()
			.enumerate()
			.take(skin.map_or(usize::MAX, |(_, ns)| ns.len()))
			.map(|(k, v)| {
				let v = &match skin {
					Some((ps, ns)) => Vertex { pos: ps[k], normal: ns[k], ..*v },
					None => *v,
				};
				let p = turn(v.pos);
				let colour = match scene.marker_tint.filter(|&(_, s)| k >= s && k < scene.marker_hero_len) {
					Some((t, _)) => [t[0], t[1], t[2], -2.0],
					None => match clank_light.filter(|l| (l.0..l.1).contains(&k)) {
						Some((_, _, t)) => [t[0], t[1], t[2], -2.0],
						None => match ghosts.iter().flatten().find(|g| (g.0..g.1).contains(&k)) {
							Some(&(_, _, a)) => [v.colour[0], v.colour[1], v.colour[2], moby_alpha_encoding(id, a).0],
							None => [v.colour[0], v.colour[1], v.colour[2], id],
						},
					},
				};
				// WPN-3.2: the held bomb's colour scale rides the same per-range vertex colour path.
				let colour = match bomb.filter(|b| (b.0..b.1).contains(&k)) {
					Some((_, _, t)) => [colour[0] * t[0], colour[1] * t[1], colour[2] * t[2], colour[3]],
					None => colour,
				};
				let uv = match shift {
					Some((a, b, dv)) if (a..b).contains(&k) => [v.uv[0], v.uv[1] + dv],
					_ => v.uv,
				};
				Vertex { pos: [p[0] + feet[0], p[1] + feet[1], p[2] + feet[2]], uv, normal: turn(v.normal), colour, light, ..*v }
			}));
		self.queue.write_buffer(&scene.marker.vbuf, 0, bytemuck::cast_slice(&verts));
		clip_marker(&self.queue, scene, &verts);
		scene.marker_verts = verts;
		scene.marker_shown = true;
		scene.marker_alpha = ghosts.iter().any(Option::is_some);
	}

	/// NOV-5.9c: this frame's view basis for the CPU clipper (None disables it for the calls that follow).
	pub(super) fn set_clip_view(&mut self, view: Option<(&Camera, f32)>) {
		if let Some(scene) = &mut self.scene {
			scene.clip_view = view.map(|(cam, aspect)| super::clip::ClipView::new(cam, aspect));
			// REN-CULL-OCC: the mask is rebuilt once per frame, before any tie / fragment / moby test; with no view (menus) it
			// stays as it was and nothing tests it.
			if let Some(v) = &scene.clip_view {
				scene.occ.forced = crate::retail::OCC_MASK.lock().ok().and_then(|g| *g);
				scene.occ.update(v.e);
			}
		}
	}

	/// BRIDGE-33: per prop slot, this frame's draw verdict (empty before the first `set_props` of a scene).
	pub(super) fn props_drawn(&self) -> &[bool] {
		self.scene.as_ref().map_or(&[], |s| &s.props_drawn)
	}

	/// FRAME-1a: the draw verdict of one tick's render from that tick's camera, with its own occlusion state (`occ_tick`) so the
	/// display frame's mask rebuild does not disturb it. Empty without a scene.
	pub(super) fn props_verdict(&mut self, cam: &Camera, aspect: f32, props: &[Prop], centres: &[[f32; 3]], radii: &[Option<f32>]) -> Vec<bool> {
		let Some(scene) = &mut self.scene else { return Vec::new() };
		let v = super::clip::ClipView::new(cam, aspect);
		scene.occ_tick.update(v.e);
		// As `set_props`, only the scene's prop slots are tested; the rest stay not drawn.
		let n = props.len().min(scene.props_ranges.len());
		let mut drawn = prop_verdicts(&v, &scene.occ_tick.mask, &props[..n], &prop_alphas(Some((v.e, v.f)), &props[..n], centres, radii), centres, radii);
		drawn.resize(props.len(), false);
		drawn
	}

	/// Poses every gameplay prop (None hides them all, outside play mode). `posed` holds animated
	/// model-space (positions, normals) per class, shared by all its instances.
	/// `actors` fills the scene-actor slots after the props: a skinned pose (root-relative, facing
	/// baked in) at a world position, or None to hide.
	pub(super) fn set_props(&mut self, props: Option<&[Prop]>, posed: &HashMap<PoseKey, Posed>, fx_posed: &HashMap<usize, Posed>, actors: &[Option<(Posed, [f32; 3])>], debris: &[particles::Debris], insts: &[particles::WorldInst], centres: &[[f32; 3]], radii: &[Option<f32>]) {
		let _s = super::profile::span("set_props");
		let prop_v = self.prop_v.take();
		let Some(scene) = &mut self.scene else { return };
		let Some(props) = props else {
			scene.props_shown = false;
			scene.props_alpha = false;
			scene.clip_queue.clear();
			scene.clip_batch.draws.clear();
			return;
		};
		// Draw-distance fade (MOBY-FADE): None = past the limit (culled), else the moby draw alpha.
		let view = scene.clip_view.as_ref().map(|v| (v.e, v.f));
		let alphas = prop_alphas(view, props, centres, radii);
		scene.props_alpha = props.iter().zip(&alphas).any(|(p, a)| p.draw_shown() && a.is_some_and(|a| moby_alpha_encoding(0.0, a).1));
		let n_slots = scene.props_ranges.len();
		let mut shown = vec![false; n_slots];
		scene.props_drawn = vec![false; props.len()];
		let masked = scene.clip_view.is_some();
		// SCH-6b: the deferred list holds 48 committed entries per frame (flags 0x800 props that passed the side test).
		let mut committed = 0;
		// BRIDGE-45: the drawn props whose metal packets `set_metal_props` builds (prop, placement, scale, dynamic-light id).
		let mut metal_jobs = Vec::new();
		// [PC] perf: what each slot's vertices are built from this frame, in slot order: written by the CPU path below
		// (RAC_CPU_SKIN=1) or posed by `vs_props` from one record per slot (gpuskin.rs).
		let mut srcs: Vec<SlotSrc> = Vec::with_capacity(n_slots);
		for (pi, (p, r)) in props.iter().zip(&scene.props_ranges).enumerate() {
			// REN-CULL-OCC-MOBY: a prop whose occlusion bytes fail this frame's mask is not drawn (no view: nothing is tested).
			let Some(alpha) = alphas[pi].filter(|_| p.draw_shown() && (!masked || crate::occlusion::occ_pass(&scene.occ.mask, p.sched_occ()))) else {
				srcs.push(SlotSrc::Hidden);
				continue;
			};
			// NOV-35b: a pose may carry the class's metal vertices after the LOD ones (`anim::skin_all`). Its first `posed_n`
			// positions and normals replace the model's.
			let pose = fx_posed.get(&pi).or_else(|| posed.get(&pose_key(p))).filter(|(ps, _)| ps.len() >= r.len());
			let posed_n = pose.map_or(0, |(ps, ns)| ps.len().min(ns.len()));
			let (ps, ns): (&[[f32; 3]], &[[f32; 3]]) = pose.map_or((&[], &[]), |(ps, ns)| (ps, ns));
			// REN-MOBY-DRAWN: the verdict also needs the near / side planes (the GPU clips, so the prop is still drawn).
			let side_out = match (&scene.clip_view, centres.get(pi), radii.get(pi).copied().flatten()) {
				(Some(v), Some(&c), Some(r)) => v.moby_side_near_out(c, r),
				_ => false,
			};
			scene.props_drawn[pi] = !side_out;
			if p.sched_flags() & 0x800 != 0 && !side_out {
				committed += 1;
				if committed > DEFERRED_CAP {
					// List full: not drawn; the verdict is 0 when the override was set, else the side pass stands.
					scene.props_drawn[pi] = p.sched_over() == 0;
					srcs.push(SlotSrc::Deferred { ps: &ps[..posed_n], ns: &ns[..posed_n] });
					continue;
				}
			}
			shown[pi] = true;
			// PC-FPS-1: the placement drawn between the last two ticks when smooth motion blends it.
			let (m, scale) = super::smooth::prop_xf(pi).unwrap_or_else(|| (place_mirrored(p.pos, p.rot, p.mirror), p.scale));
			let light = Self::moby_light(p.light);
			let id = dyn_id(&self.table, &mut self.moby_dyn, [m[12], m[13], m[14]]);
			let tint = PROP_TINTS.with(|t| t.borrow().get(pi).copied().flatten());
			metal_jobs.push((pi, m, scale, id));
			let alpha_code = moby_alpha_encoding(id, alpha).0;
			if crate::retail::dbg_class() == Some(p.class_id as u32) {
				eprintln!("DBGDRAW {} slot {pi} alpha {alpha} moby_alpha {} light {light:?} dyn {id} dl {:?} scale {scale} l0 {:?}", p.class_id, p.moby_alpha(), (id > 0.0).then(|| self.moby_dyn.get(id as usize - 1)), self.lighting.as_ref().and_then(|lt| lt.lights.first()).map(|l| (l.colour, l.wrap, l.dir)));
			}
			// WPN-6.3e: the aux 179 texture scroll (FUN_00263928 adds the delta to every vertex V, as the Pyrocitor nozzle);
			// a scrolled slot is rebuilt every frame.
			let cuv = self.class_uv.get(&(p.class_id as u32)).copied().unwrap_or([0.0; 2]);
			let (du, dv) = (cuv[0], cuv[1] + prop_v.filter(|&(i, _)| i == pi).map_or(0.0, |(_, d)| d));
			srcs.push(SlotSrc::Prop(SlotJob { range: r.clone(), ps: &ps[..posed_n], ns: &ns[..posed_n], m, scale, light, alpha_code, prop_tint: p.tint, tint, alpha, dv, du }));
		}
		// Debris: every live piece is instanced into the per-frame debris batch (the moby pool bounds the count).
		if let Some(batch) = &mut scene.debris {
			let (mut dv, mut by_tex): (Vec<Vertex>, BTreeMap<usize, Vec<u32>>) = (Vec::new(), BTreeMap::new());
			let mut di = 0;
			// NOV-10.5a-draw: game-owned instances (teleporter rings) ride the same batch.
			for (class, pos, mm, sc, draw) in debris.iter().map(|l| (l.class, l.pos, l.m, l.scale, Some(l.draw))).chain(insts.iter().map(|i| (i.class, i.pos, i.m, i.scale, None))) {
				let Some((model, faces, phases)) = scene.debris_model.get(&class) else { continue };
				// SCH-6b: past the piece's draw distance it is not drawn (the opaque batch carries no fade).
				if effect_fade(view, draw, pos).is_none() {
					continue;
				}
				let n_idx: usize = faces.iter().map(|f| f.1.len()).sum();
				if dv.len() + model.len() > scene.debris_cap.0 || di + n_idx > scene.debris_cap.1 {
					break;
				}
				di += n_idx;
				let base = dv.len() as u32;
				// The crate sink fade (alpha -4 per tick) is not drawn: the batch is opaque, and the 0.9 per tick
				// shrink hides the piece first [P].
				let (c, p) = (mm, pos);
				let m = [c[0][0], c[0][1], c[0][2], 0.0, c[1][0], c[1][1], c[1][2], 0.0, c[2][0], c[2][1], c[2][2], 0.0, p[0], p[1], p[2], 1.0];
				let light = Self::moby_light(None);
				let id = dyn_id(&self.table, &mut self.moby_dyn, p);
				for v in model {
					let mut v = *v;
					v.colour[3] = id;
					v.light = light;
					let (a, n) = (v.pos.map(|c| c * sc), v.normal);
					for j in 0..3 {
						v.pos[j] = m[j] * a[0] + m[4 + j] * a[1] + m[8 + j] * a[2] + m[12 + j];
						v.normal[j] = m[j] * n[0] + m[4 + j] * n[1] + m[8 + j] * n[2];
					}
					dv.push(v);
				}
				for (k, (t, idx)) in faces.iter().enumerate() {
					for (fi, f) in idx.chunks_exact(3).enumerate() {
						let phase = phases[k].1[fi];
						let tri = [f[0], f[1], f[2]].map(|i| dv[(i + base) as usize]);
						// NOV-5.9d path 1: a face the retail clipper takes leaves the batch; its polygon goes to the overlay.
						if let Some(poly) = scene.clip_view.filter(|_| phase != crate::mesh::NO_PHASE).and_then(|view| super::clip::clip_face(&view, tri, phase, *t, 0)) {
							scene.clip_queue.push(poly);
						} else {
							by_tex.entry(*t).or_default().extend(f.iter().map(|i| i + base));
						}
					}
				}
			}
			let (mut indices, mut draws) = (Vec::new(), Vec::new());
			for (texture, idx) in by_tex {
				draws.push(Draw { inst: 0, cull: 0, texture, start: indices.len() as u32, count: idx.len() as u32 });
				indices.extend(idx);
			}
			if !dv.is_empty() {
				self.queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(&dv));
				self.queue.write_buffer(&batch.ibuf, 0, bytemuck::cast_slice(&indices));
			}
			batch.draws = draws;
		}
		for (i, r) in scene.props_ranges.iter().enumerate().skip(props.len()) {
			match actors.get(i - props.len()).and_then(|a| a.as_ref()).filter(|((ps, _), _)| ps.len() == r.len()) {
				Some(((ps, ns), at)) => {
					shown[i] = true;
					let light = self.scene_actor_light.unwrap_or_else(|| Self::moby_light(Some(([0x38; 3], 0)))); // SCN-3: the hero's word at scene start, else the 0x38 constant
					let tint = ACTOR_TINTS.with(|t| t.borrow().get(i - props.len()).copied().flatten());
					let id = dyn_id(&self.table, &mut self.moby_dyn, *at);
					srcs.push(SlotSrc::Actor { ps, ns, at: *at, light, id, tint });
				}
				None => srcs.push(SlotSrc::Hidden),
			}
		}
		scene.props_vis = shown.clone();
		if let Some(pg) = &mut self.props_gpu {
			// [PC] perf round 3: one record per slot and each distinct pose once; `vs_props` poses the static model.
			let (mut inst, mut pose, mut rad) = (std::mem::take(&mut scene.props_inst), std::mem::take(&mut scene.props_pose), Vec::new());
			{
				let _s = super::profile::span("props_slots");
				build_records(&srcs, &scene.props_ranges, &scene.props_rad, &mut inst, &mut pose, &mut rad);
			}
			// BRIDGE-36 (c2): with a HUD split the posed prompt goes to its own batch and leaves the props batch (and the clip overlay).
			if let (Some(_), Some(pi), Some(prompt)) = (self.hud_split, scene.prompt_slot, scene.prompt.as_ref()) {
				if pi < props.len() {
					let r = scene.props_ranges[pi].clone();
					let vs: Vec<Vertex> = scene.props_model[r].iter().enumerate().map(|(j, m0)| super::gpuskin::emulate(&inst[pi], &pose, m0, j)).collect();
					self.queue.write_buffer(&prompt.vbuf, 0, bytemuck::cast_slice(&vs));
					inst[pi].k[0] |= super::gpuskin::FLAG_PROMPT;
				}
			}
			{
				let _s = super::profile::span("props_clip");
				clip_props(&self.queue, scene, ClipSrc::Gpu(&inst, &pose, &rad), &shown);
				flush_clip(&self.device, &self.queue, scene);
			}
			let _u = super::profile::span("props_upload");
			pg.upload(&self.device, &self.queue, &inst, &pose);
			(scene.props_inst, scene.props_pose) = (inst, pose);
			scene.props_shown = true;
		} else {
			// PC perf: the mirror of `props.vbuf`; every slot range this frame changes is rebuilt from the model and marked
			// `dirty`, a slot already holding the plain hidden copy is left alone (same buffer contents as a full rebuild).
			let mut verts = std::mem::take(&mut scene.props_verts);
			let mut dirty = vec![false; n_slots];
			{
				let model = &scene.props_model;
				let hidden = &mut scene.props_hidden;
				let keys = &mut scene.props_key;
				// [PC] perf: the slot rebuilds, run on the frame worker pool after the loop (`SlotJob::fill`).
				let mut jobs: Vec<&SlotJob> = Vec::new();
				for (i, src) in srcs.iter().enumerate() {
					let r = scene.props_ranges[i].clone();
					match src {
						SlotSrc::Hidden => {
							keys[i] = None;
							hide_slot(&mut verts, model, hidden, &mut dirty, i, r);
						}
						SlotSrc::Deferred { ns, .. } => {
							hidden[i] = false;
							keys[i] = None;
							dirty[i] = true;
							for (j, (v, m0)) in verts[r.clone()].iter_mut().zip(&model[r]).enumerate() {
								*v = *m0;
								if j < ns.len() {
									v.normal = ns[j];
								}
								v.pos = HIDDEN_POS;
							}
						}
						SlotSrc::Prop(job) => {
							// An unposed slot built last frame from the very same inputs already holds this frame's vertices (PC perf).
							let key = (job.ps.is_empty() && job.dv == 0.0 && job.du == 0.0).then(|| slot_key(&job.m, job.scale, job.light, job.alpha_code, job.alpha, job.tint, job.prop_tint));
							if key.is_some() && !hidden[i] && keys[i] == key {
								continue;
							}
							keys[i] = key;
							hidden[i] = false;
							dirty[i] = true;
							jobs.push(job);
						}
						SlotSrc::Actor { ps, ns, at, light, id, tint } => {
							verts[r.clone()].copy_from_slice(&model[r.clone()]);
							hidden[i] = false;
							keys[i] = None;
							dirty[i] = true;
							for (j, (v, (q, n))) in verts[r].iter_mut().zip(ps.iter().zip(ns.iter())).enumerate() {
								v.pos = [q[0] + at[0], q[1] + at[1], q[2] + at[2]];
								v.normal = *n;
								v.colour[3] = *id;
								v.light = *light;
								if let Some((c, _)) = tint.filter(|&(_, start)| j >= start) {
									v.colour = [c[0], c[1], c[2], -2.0]; // baked-colour path: constant RGB, no lights
								}
							}
						}
					}
				}
				let _s = super::profile::span("props_slots");
				let ranges: Vec<std::ops::Range<usize>> = jobs.iter().map(|j| j.range.clone()).collect();
				par_chunked(&mut verts, &ranges, |k, off, dst| jobs[k].fill(dst, &model[ranges[k].start + off..ranges[k].end], off));
			}
			// BRIDGE-36 (c2): with a HUD split the posed prompt goes to its own batch and leaves the props batch (and the clip overlay).
			if let (Some(_), Some(pi), Some(prompt)) = (self.hud_split, scene.prompt_slot, scene.prompt.as_ref()) {
				if pi < props.len() {
					let r = scene.props_ranges[pi].clone();
					self.queue.write_buffer(&prompt.vbuf, 0, bytemuck::cast_slice(&verts[r.clone()]));
					for v in &mut verts[r] {
						v.pos = HIDDEN_POS;
					}
					scene.props_key[pi] = None;
					dirty[pi] = true;
				}
			}
			{
				let _s = super::profile::span("props_clip");
				clip_props(&self.queue, scene, ClipSrc::Cpu(&verts), &shown);
				flush_clip(&self.device, &self.queue, scene);
			}
			// The rewritten slots, adjacent ones merged into one write.
			let _u = super::profile::span("props_upload");
			let mut k = 0;
			while k < dirty.len() {
				if !dirty[k] {
					k += 1;
					continue;
				}
				let start = scene.props_ranges[k].start;
				let mut end = scene.props_ranges[k].end;
				k += 1;
				while k < dirty.len() && dirty[k] && scene.props_ranges[k].start == end {
					end = scene.props_ranges[k].end;
					k += 1;
				}
				self.queue.write_buffer(&scene.props.vbuf, (start * std::mem::size_of::<Vertex>()) as u64, bytemuck::cast_slice(&verts[start..end]));
			}
			scene.props_verts = verts;
			scene.props_shown = true;
		}
		let _m = super::profile::span("props_metal");
		self.set_metal_props(props, &metal_jobs, posed, fx_posed, centres, radii);
	}

	pub(super) fn resize(&mut self, w: u32, h: u32) {
		if w == 0 || h == 0 {
			return;
		}
		self.config.width = w;
		self.config.height = h;
		configure_surface(&self.surface, &self.device, &self.config);
		self.depth = depth_view(&self.device, w, h, self.depth_format);
		self.msaa = msaa_view(&self.device, self.config.format, w, h);
	}

	/// Camera, level lights and fog (decomp/notes/lighting_fog.md); a neutral rig without them. The fog is the
	/// live one (`fog_live`: camera zone or underwater, NOV-28) once the handler has set it.
	pub(super) fn uniforms(&self, cam: &Camera, eye: &Camera, aspect: f32, range: (f32, f32)) -> Uniforms {
		self.uniforms_fog(cam, eye, aspect, range, true)
	}

	/// `uniforms`, with `live` false keeping the static level fog (the pause item panel).
	fn uniforms_fog(&self, cam: &Camera, eye: &Camera, aspect: f32, range: (f32, f32), live: bool) -> Uniforms {
		let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
		let (lights, ambient, fog_colour, fog) = match &self.lighting {
			Some(lt) if !lt.lights.is_empty() => {
				let lights = self.light_rows();
				let spawn = v4(gameplay::SPAWN_AMBIENT.map(|c| c as f32 / 128.0));
				let (colour, fog) = match self.fog_live.filter(|_| live) {
					Some((colour, fog)) => (colour, fog),
					None => (lt.fog_colour, lt.fog),
				};
				let c = crate::water::gs_rgb(colour); // the target is gamma-space Unorm: raw GS bytes
				(lights, [spawn; 4], [c[0], c[1], c[2], 1.0], fog)
			}
			_ => (self.light_rows(), [[0.4, 0.4, 0.4, 0.0]; 4], [0.0; 4], [0.0, 1.0e9, 255.0, 255.0]),
		};
		// The arrival warp sets its own fog (retail FOGCOL 0,0,0x10; range 0..0x80000) over the departure level's.
		let (fog_colour, fog) = if self.scene.as_ref().is_some_and(|s| s.warp_shown) { ([0.0, 0.0, 0x10 as f32 / 255.0, 1.0], [0.0, 524288.0, 255.0, 128.0]) } else { (fog_colour, fog) };
		let mut dynl = [[0.0; 4]; 2 * MAX_DYN];
		for (k, (c, d)) in self.moby_dyn.iter().enumerate() {
			dynl[2 * k] = [c[0], c[1], c[2], 0.0];
			dynl[2 * k + 1] = [d[0], d[1], d[2], 0.0];
		}
		Uniforms {
			view_proj: view_proj(cam, aspect, range.0, range.1),
			sky_view_proj: view_proj(eye, aspect, range.0, range.1),
			lights,
			ambient,
			fog_colour,
			fog,
			eye: [cam.pos[0], cam.pos[1], cam.pos[2], 1.0],
			points: self.points,
			dynl,
		}
	}

	/// The uniform light slots (`Uniforms::lights`): per level light (colour A + wrap, to-light A, colour B + wrap, to-light B), zero past
	/// the level's lights; without level lights one default slot 0.
	fn light_rows(&self) -> [[[f32; 4]; 4]; 16] {
		let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
		let pair = |l: &gameplay::DirLight| {
			let c = |k: usize| [l.colour[k][0], l.colour[k][1], l.colour[k][2], l.wrap[k]];
			[c(0), v4(l.dir[0].map(|x| -x)), c(1), v4(l.dir[1].map(|x| -x))]
		};
		match &self.lighting {
			Some(lt) if !lt.lights.is_empty() => std::array::from_fn(|k| match self.reserved_lights {
				Some(r) if k >= 12 => r[k - 12],
				_ => lt.lights.get(k).map_or([[0.0; 4]; 4], pair),
			}),
			_ => {
				let mut lights = [[[0.0; 4]; 4]; 16];
				lights[0] = [[0.8, 0.8, 0.8, 0.0], [0.3, 0.3, 0.9, 0.0], [0.0; 4], [0.0; 4]];
				lights
			}
		}
	}

	/// NOV-35b: the three light rows (world to-light direction, colour) a metal vertex of a moby sums (FUN_00210358 vf25..vf30, the
	/// moby's own record): its light pair blended as REN-5d (`slot_blend`; colours and directions lerp by t, directions renormalised,
	/// t = 0 uses A as is) and its dynamic third light (`dyn_id` id, 0 = none). The wrap lanes are not used.
	fn metal_rows(&self, placed: Option<([u8; 3], u32)>, id: f32) -> [([f32; 3], [f32; 3]); 3] {
		let lights = self.light_rows();
		let (_, word) = placed.unwrap_or((gameplay::SPAWN_AMBIENT, 0));
		let (a, b, t) = gameplay::light_word(word);
		let (la, lb, t) = (lights[a.min(15)], lights[b.min(15)], t.clamp(0.0, 0.999));
		let xyz = |v: [f32; 4]| [v[0], v[1], v[2]];
		let mix = |x: [f32; 4], y: [f32; 4]| -> [f32; 3] { std::array::from_fn(|k| x[k] * (1.0 - t) + y[k] * t) };
		let pair = |j: usize| -> ([f32; 3], [f32; 3]) {
			if t == 0.0 {
				return (xyz(la[2 * j + 1]), xyz(la[2 * j]));
			}
			let d = mix(la[2 * j + 1], lb[2 * j + 1]);
			let l = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
			if l < 1e-12 {
				return ([0.0; 3], [0.0; 3]);
			}
			(d.map(|x| x / l.sqrt()), mix(la[2 * j], lb[2 * j]))
		};
		let third = match (id.round() as usize).checked_sub(1).and_then(|i| self.moby_dyn.get(i)) {
			Some((c, d)) => (d.map(|x| -x), *c),
			None => ([0.0; 3], [0.0; 3]),
		};
		[pair(0), pair(1), third]
	}

	/// BRIDGE-45 (NOV-35b): `load_scene`'s metal half: every class's metal packets and the chrome / glass textures (`CHROME_KEY`,
	/// `GLASS_KEY` in `textures`), uploaded with the clamp sampler (CLAMP_1 = 5, TEX1 MMAG linear; FUN_00253c78 OV:44617-44620).
	pub(super) fn load_metal(&mut self, classes: HashMap<u32, Vec<mobies::MetalPacket>>, textures: &HashMap<u32, Texture>) {
		let tex = match (textures.get(&crate::level::CHROME_KEY), textures.get(&crate::level::GLASS_KEY)) {
			(Some(c), Some(g)) => Some([self.upload(Some(c), &self.clamp_sampler), self.upload(Some(g), &self.clamp_sampler)]),
			_ => None,
		};
		self.metal = MetalDraw { classes, tex, props: None, cap: (0, 0), scratch: Vec::new() };
	}

	/// BRIDGE-45 (NOV-35b): this frame's metal faces of the drawn props. `jobs` = (prop index, placement, scale, dynamic-light id)
	/// of every prop `set_props` drew; per prop: metal alpha from the MOBY-FADE view depth of its sphere (None: no metal part),
	/// env matrix from the sphere centre and the camera, then the skinned metal vertices (the pose tail after the LOD vertices,
	/// else the stored ones). Without a view (menus) nothing is built.
	fn set_metal_props(&mut self, props: &[Prop], jobs: &[(usize, [f32; 16], f32, f32)], posed: &HashMap<PoseKey, Posed>, fx_posed: &HashMap<usize, Posed>, centres: &[[f32; 3]], radii: &[Option<f32>]) {
		let (mut verts, mut runs) = (Vec::new(), BTreeMap::new());
		if let Some((view, ranges)) = self.scene.as_ref().and_then(|s| Some((s.clip_view?, &s.props_ranges))) {
			let mut todo = Vec::new();
			for &(pi, m, scale, id) in jobs {
				let p = &props[pi];
				let (Some(packets), Some(&c), Some(r)) = (self.metal.classes.get(&p.class_id), centres.get(pi), ranges.get(pi)) else { continue };
				// The MOBY-FADE z of `set_props` and the sphere radius (0 without one).
				let d: [f32; 3] = std::array::from_fn(|k| c[k] - view.e[k] * 1024.0);
				let z = view.f[0] * d[0] + view.f[1] * d[1] + view.f[2] * d[2];
				let Some(alpha) = metal_alpha(metal_byte(p.class_id), z, radii.get(pi).copied().flatten().unwrap_or(0.0)) else { continue };
				let total: usize = packets.iter().map(|q| q.positions.len()).sum();
				let tail = fx_posed.get(&pi).or_else(|| posed.get(&pose_key(p))).filter(|(ps, _)| ps.len() == r.len() + total).map(|(ps, ns)| (&ps[r.len()..], &ns[r.len()..]));
				let e = mobies::metal_env(view.e, metal_view_rows(&view), c, axes(&m));
				todo.push((packets.as_slice(), tail, self.metal_rows(p.light, id), e, m, scale, alpha));
			}
			// [PC] perf round 3: one reused vertex buffer; each prop fills its own slice (`fill_metal`) on the frame worker pool.
			let counts: Vec<usize> = todo.iter().map(|t| metal_count(t.0)).collect();
			verts = std::mem::take(&mut self.metal.scratch);
			verts.clear();
			verts.resize(counts.iter().sum(), Vertex::zeroed());
			let mut slices: Vec<(&mut [Vertex], u32)> = Vec::with_capacity(counts.len());
			let (mut rest, mut first) = (&mut verts[..], 0u32);
			for &c in &counts {
				let (a, b) = std::mem::take(&mut rest).split_at_mut(c);
				slices.push((a, first));
				(rest, first) = (b, first + c as u32);
			}
			let built: Vec<BTreeMap<(u8, usize), Vec<u32>>> = todo
				.par_iter()
				.zip(slices.into_par_iter())
				.map(|((packets, tail, rows, e, m, scale, alpha), (dst, first))| {
					let mut r = BTreeMap::new();
					fill_metal(dst, first, &mut r, packets, *tail, rows, *e, m, *scale, *alpha, true);
					r
				})
				.collect();
			for r in built {
				for (key, idx) in r {
					runs.entry(key).or_insert_with(Vec::new).extend(idx);
				}
			}
		}
		upload_metal(&self.device, &self.queue, &mut self.metal.props, &mut self.metal.cap, &verts, runs);
		self.metal.scratch = verts;
	}

	/// SCN-3: capture the hero's light for the scene actors when a scene starts, release it when the scene ends.
	pub(super) fn scene_actor_start(&mut self, active: bool, feet: [f32; 3]) {
		let mut held = self.scene_actor_light;
		let (lighting, placed, env) = (&self.lighting, self.hero_placed, &mut self.hero_env);
		scene_light_snapshot(&mut held, active, || Self::hero_light(lighting, feet, placed, env));
		self.scene_actor_light = held;
	}

	/// A moby's `Vertex::light` (decomp/notes/lighting_fog.md "Moby lighting"): a placed moby's own colour and
	/// light word (instance +0x64.. / +0x70), else the spawn default (64, 64, 64) and word 0. Zones and point
	/// lights are the hero's only (`hero_light`).
	pub(super) fn moby_light(placed: Option<([u8; 3], u32)>) -> [f32; 4] {
		let (col, word) = placed.unwrap_or((gameplay::SPAWN_AMBIENT, 0));
		let (a, b, t) = gameplay::light_word(word);
		Self::pack_light(col.map(|c| c as f32 / 128.0), a, b, t)
	}

	/// The hero's `Vertex::light`: HeroEnvLighting (zones, else its placed colour and word, plus point lights).
	pub(super) fn hero_light(lighting: &Option<gameplay::Lighting>, p: [f32; 3], placed: ([u8; 3], u32), env: &mut (usize, usize, f32, [f32; 3])) -> [f32; 4] {
		Self::hero_light_tint(lighting, p, placed, env).0
	}

	/// `hero_light` and BRIDGE-57's copy of the hero's +0x3c ambient bytes / +0x38 light word (FUN_00271c10 reads them;
	/// decomp/notes/port_bridges.md BRIDGE-57). [I] a blended zone ambient quantises by truncation.
	pub(super) fn hero_light_tint(lighting: &Option<gameplay::Lighting>, p: [f32; 3], placed: ([u8; 3], u32), env: &mut (usize, usize, f32, [f32; 3])) -> ([f32; 4], ([u8; 3], u32)) {
		match lighting {
			Some(lt) => {
				let (a, b, t, amb) = lt.hero(p, env);
				let bytes = amb.map(|c| (c * 128.0).min(255.0) as u8);
				(Self::pack_light(amb, a, b, t), (bytes, a as u32 | (b as u32) << 8 | ((t * 256.0) as u32) << 16))
			}
			None => (Self::moby_light(Some(placed)), placed),
		}
	}

	/// Slots past 15 read as slot 15 (zero unless the level has 16 lights).
	pub(super) fn pack_light(amb: [f32; 3], a: usize, b: usize, t: f32) -> [f32; 4] {
		[amb[0], amb[1], amb[2], (a.min(15) * 16 + b.min(15)) as f32 + t.clamp(0.0, 0.999)]
	}

	/// Camera-facing quads for this frame's particles.
	/// `fx`: retail mode's effect pool (retail.rs `RetailFx`): its lines (mode 2) and ribbons (mode 3) join the pool
	/// sprites in one depth-sorted walk (queue_b.rs `pool_walk`); `lines` / `ribbons` are rac-rs's own.
	pub(super) fn set_particles(&mut self, sprites: &[particles::Sprite], ribbons: &[particles::Ribbon], lines: &[particles::Line], mobys: &[particles::EffectMoby], cam: &Camera, fx: Option<&crate::retail::RetailFx>) {
		let _s = super::profile::span("set_particles");
		let f = [cam.pitch.cos() * cam.yaw.cos(), cam.pitch.cos() * cam.yaw.sin(), cam.pitch.sin()];
		let (slots, pool_ribbons, pool_lines, line_slots) = fx.map_or((&[][..], &[][..], &[][..], &[][..]), |x| (&x.sprite_slots[..], &x.ribbons[..], &x.lines[..], &x.line_slots[..]));
		let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
		let m = view_proj(cam, aspect, RETAIL_NEAR, RETAIL_FAR);
		// KER-92: pool lines, kept when both points pass the clip test and the midpoint view depth is inside (near, far).
		self.line_verts.clear();
		if !lines.is_empty() {
			for l in lines.iter().filter(|l| line_shown(&m, cam.pos, f, l, self.part_far)) {
				self.line_verts.push(Vertex { pos: l.a, uv: [0.0; 2], normal: [0.0; 3], colour: l.col_a, light: [0.0; 4] });
				self.line_verts.push(Vertex { pos: l.b, uv: [0.0; 2], normal: [0.0; 3], colour: l.col_b, light: [0.0; 4] });
			}
		}
		// BRIDGE-56: billboards face the rolled view.
		let right = normalise(cross(f, cam.up));
		let up = cross(right, f);
		self.part_verts.clear();
		let view = self.scene.as_ref().and_then(|s| s.clip_view.as_ref()).map(|v| (v.e, v.f));
		let defs = &self.part_defs;
		// Effect type -> texture key; FX_TEX + n is an fx texture, an unmapped type falls back to the disc.
		let key = |t: u32| if t >= particles::FX_TEX { t - particles::FX_TEX } else { defs.get(t as usize).map_or(u32::MAX, |&n| PART_TEX + n as u32) };
		let at = |p: [f32; 3], a: [f32; 3], s: f32, b: [f32; 3], t: f32| [p[0] + a[0] * s + b[0] * t, p[1] + a[1] * s + b[1] * t, p[2] + a[2] * s + b[2] * t];
		// Retail pool draw order (retail/queue_b.rs `pool_walk`): pool sprites, ribbons and lines draw back to front in
		// runs of one (blend, texture); every other sprite keeps run 0, grouped by (blend, texture) as before.
		let mut run: (u32, Option<(bool, u32)>) = (0, None);
		let mut next_run = |additive: bool, k: u32| {
			if run.1 != Some((additive, k)) {
				run = (run.0 + 1, Some((additive, k)));
			}
			run.0
		};
		use crate::retail::queue_b::PoolItem;
		for item in crate::retail::queue_b::pool_walk(sprites, slots, pool_ribbons, pool_lines, line_slots, cam.pos, f) {
			let sp = match item {
				PoolItem::Sprite(i) => &sprites[i],
				// Mode 3 (FUN_00259e18 sort, overlay.elf.c 81947-81985): both points clip-tested, midpoint key strict in
				// (near, far); the build fades both alphas by the sprite ramp (retail.rs `pool_ribbon_quad`) [C].
				PoolItem::Ribbon(i) => {
					let r = &pool_ribbons[i];
					let vz = (0..3).map(|k| ((r.head[k] + r.tail[k]) * 0.5 - cam.pos[k]) * f[k]).sum::<f32>();
					let (near, far) = ((r.b9 & 15) as f32 / 4.0, crate::water::pool_far(r.b9, self.part_far));
					if vz - near <= 0.0 || vz - far >= 0.0 || !clip_inside(&m, r.head) || !clip_inside(&m, r.tail) {
						continue;
					}
					let q = crate::retail::pool_ribbon_quad(r, cam.pos, (vz - near).min((far - vz) / 16.0).clamp(0.0, 1.0));
					let out = self.part_verts.entry((next_run(q.additive, q.tex as u32), q.additive, q.tex as u32)).or_default();
					for k in 0..4 {
						out.push(Vertex { pos: q.pos[k], uv: q.uv[k], normal: [0.0; 3], colour: q.colour[k], light: [0.0; 4] });
					}
					continue;
				}
				// Mode 2 (KER-92 cull, `line_shown`): no fade.
				PoolItem::Line(i) => {
					let l = &pool_lines[i];
					if line_shown(&m, cam.pos, f, l, self.part_far) {
						let out = self.part_verts.entry((next_run(false, LINE_KEY), false, LINE_KEY)).or_default();
						out.push(Vertex { pos: l.a, uv: [0.0; 2], normal: [0.0; 3], colour: l.col_a, light: [0.0; 4] });
						out.push(Vertex { pos: l.b, uv: [0.0; 2], normal: [0.0; 3], colour: l.col_b, light: [0.0; 4] });
					}
					continue;
				}
			};
			let run = match sp.pool {
				Some(_) => next_run(sp.additive, key(sp.tex)),
				None => 0,
			};
			// Pool sprites (decomp/notes/particles.md "Draw size"): size * vz/(vz+0.5), alpha * the byte +9 near/far
			// ramp (culled outside it), texture (0,0) corner at screen angle rot clockwise, i.e. this quad's
			// rot + 135 deg counter-clockwise = -rot. PAL's 0.918 vertical squash is dropped (modern display).
			let (size, rot, colour) = match sp.pool {
				Some(b9) => {
					let vz = (sp.pos[0] - cam.pos[0]) * f[0] + (sp.pos[1] - cam.pos[1]) * f[1] + (sp.pos[2] - cam.pos[2]) * f[2];
					let (near, far) = ((b9 & 15) as f32 / 4.0, crate::water::pool_far(b9, self.part_far));
					if vz <= near || vz >= far {
						continue;
					}
					let fade = (vz - near).min((far - vz) / 16.0).clamp(0.0, 1.0);
					let mut c = sp.colour;
					c[3] *= fade;
					// NOV-22.12 mode 1: same cull and fade, then a world-XY quad (no perspective term, no 135 deg turn).
					if sp.flat {
						let out = self.part_verts.entry((run, sp.additive, key(sp.tex))).or_default();
						for (p, uv) in flat_corners(sp.pos, sp.size, sp.rot).into_iter().zip([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]) {
							out.push(Vertex { pos: p, uv, normal: [0.0; 3], colour: c, light: [0.0; 4] });
						}
						continue;
					}
					(sp.size * vz / (vz + 0.5), -sp.rot - std::f32::consts::FRAC_PI_4 * 3.0, c)
				}
				None => (sp.size, sp.rot, sp.colour),
			};
			let h = size / 2.0;
			let (c, sn) = (rot.cos() * h, rot.sin() * h);
			let r = [right[0] * c + up[0] * sn, right[1] * c + up[1] * sn, right[2] * c + up[2] * sn];
			let u = [up[0] * c - right[0] * sn, up[1] * c - right[1] * sn, up[2] * c - right[2] * sn];
			let out = self.part_verts.entry((run, sp.additive, key(sp.tex))).or_default();
			for (s, t, uv) in [(-1.0, -1.0, [0.0, 1.0]), (1.0, -1.0, [1.0, 1.0]), (1.0, 1.0, [1.0, 0.0]), (-1.0, 1.0, [0.0, 0.0])] {
				out.push(Vertex { pos: at(sp.pos, r, s, u, t), uv, normal: [0.0; 3], colour, light: [0.0; 4] });
			}
		}
		// Blast mobys (decomp/notes/particles.md "Ship crash"): triangles as degenerate quads, alpha-over [I].
		for m in mobys {
			let Some(mesh) = self.fx_meshes.get(&m.class) else { continue };
			let Some(fade) = effect_fade(view, m.draw, m.pos) else { continue };
			let (sx, cx) = m.rot[0].sin_cos();
			let (sy, cy) = m.rot[1].sin_cos();
			let (sz, cz) = m.rot[2].sin_cos();
			let colour = [m.tint[0], m.tint[1], m.tint[2], moby_draw_alpha(fade, m.alpha as u8) as f32 / 128.0];
			for (tex, f) in &mesh.tris {
				let out = self.part_verts.entry((0, false, *tex as u32)).or_default();
				let v = |i: u32| {
					let p = mesh.pos[i as usize];
					let (x, y, z) = (p[0] * m.scale, p[1] * m.scale, p[2] * m.scale);
					let (y, z) = (y * cx - z * sx, y * sx + z * cx);
					let (x, z) = (x * cy + z * sy, -x * sy + z * cy);
					let (x, y) = (x * cz - y * sz, x * sz + y * cz);
					Vertex { pos: [m.pos[0] + x, m.pos[1] + y, m.pos[2] + z], uv: mesh.uv.get(i as usize).copied().unwrap_or([0.0; 2]), normal: [0.0; 3], colour, light: [0.0; 4] }
				};
				out.extend([v(f[0]), v(f[1]), v(f[2]), v(f[2])]);
			}
		}
		for rb in ribbons {
			let n = rb.points.len();
			let out = self.part_verts.entry((0, rb.additive, key(rb.tex))).or_default();
			for k in (if rb.arc { 2 } else { 1 })..n {
				let (a, b) = (rb.points[k - 1], rb.points[k]);
				let side = normalise(cross([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [cam.pos[0] - a[0], cam.pos[1] - a[1], cam.pos[2] - a[2]]));
				let edge = |p: [f32; 3], j: usize, u: f32| {
					let t = j as f32 / (n - 1).max(1) as f32;
					let w = if rb.arc {
						let t = j as f32 / 20.0;
						rb.width0 + (rb.width1 - rb.width0) * (-3.0 * t * t * t + 5.0 * t * t - t) // Hermite(-1, 0, 1, 0)
					} else {
						rb.width0 + (rb.width1 - rb.width0) * t
					};
					let c: [f32; 4] = std::array::from_fn(|i| rb.colour[i] + (rb.tip_colour[i] - rb.colour[i]) * t);
					let u = if rb.arc { u } else { 0.5 };
					[Vertex { pos: at(p, side, -w, side, 0.0), uv: [u, 1.0], normal: [0.0; 3], colour: c, light: [0.0; 4] }, Vertex { pos: at(p, side, w, side, 0.0), uv: [u, 0.0], normal: [0.0; 3], colour: c, light: [0.0; 4] }]
				};
				let ([a0, a1], [b0, b1]) = (edge(a, k - 1, rb.uv_scroll), edge(b, k, rb.uv_scroll + 1.0));
				out.extend([a0, b0, b1, a1]);
			}
		}
	}

	/// BRIDGE-37 (b): poses the shop portrait (class 12) for the next frame: skinned vertices placed at `pos` / `rot` into the
	/// one-prop batch (alpha 0x80, light word colour 0x20; vendor.md UI-5a). `None` hides it (the target is still cleared).
	pub(super) fn pose_portrait(&mut self, pose: Option<(&Posed, [f32; 3], [f32; 3])>) {
		self.portrait_shown = false;
		let (Some(scene), Some(((ps, ns), pos, rot))) = (self.scene.as_ref(), pose) else { return };
		let Some(batch) = scene.portrait.as_ref().filter(|_| ps.len() == scene.portrait_model.len()) else { return };
		let m = place(pos, rot);
		let light = Self::moby_light(Some(([0x20; 3], 0)));
		let id = dyn_id(&self.table, &mut self.moby_dyn, pos);
		let mut verts = scene.portrait_model.clone();
		for (v, (q, n)) in verts.iter_mut().zip(ps.iter().zip(ns)) {
			v.colour[3] = moby_alpha_encoding(id, 0x80).0;
			v.light = light;
			for i in 0..3 {
				v.pos[i] = m[i] * q[0] + m[4 + i] * q[1] + m[8 + i] * q[2] + m[12 + i];
				v.normal[i] = m[i] * n[0] + m[4 + i] * n[1] + m[8 + i] * n[2];
			}
		}
		self.queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(&verts));
		self.portrait_shown = true;
	}

	/// Draws one frame; with `capture`, also returns it as an image.
	/// REN-9b: the shop item-panel pass. Clears the 512x128 target (colour 0, depth far) and draws the panel-1 item with the
	/// main camera's view and the target projection (tan 0.756, horizontal scalar 1.0, near 0.03125, far 512), then resolves it.
	/// The projection leaves the world projection alone: it is only this pass's own uniform slot (4).
	fn draw_item_panel(&self, enc: &mut wgpu::CommandEncoder, cam: &Camera, scene: &Scene) {
		if !(self.shop_panel_models.is_empty() && !self.hud.iter().any(|q| q.tex == ITEM_PANEL_TEX)) {
			// NOV-30d: one clear, then the item (slot 4) and the slot-0 bolt icon (slot 7), also when no item model is posed.
			let item = self.shop_panel_models.first().and_then(|(c, m)| Some((4, &scene.previews.get(c)?.0, *m, None)));
			// NOV-35b: the icon bolt's class-13 metal (glass) faces.
			let icon_metal = self.icon_bolt.and_then(|m| self.panel_metal(scene, &m, self.icon_bolt_sphere));
			let icon = self.icon_bolt.and_then(|m| Some((7, &scene.previews.get(&13)?.0, m, icon_metal.as_ref())));
			let models: Vec<_> = item.into_iter().chain(icon).collect();
			self.draw_panel_target(enc, cam, scene, &self.item_panel, &models);
		}
		// BRIDGE-36 (b): the purchase popup's 512x128 target, cleared, plus the class-13 bolt when one is posed (UI-5d).
		if self.hud.iter().any(|q| q.tex == POPUP_PANEL_TEX) {
			let bolt_metal = self.popup_bolt.and_then(|m| self.panel_metal(scene, &m, self.popup_bolt_sphere));
			let bolt = self.popup_bolt.and_then(|m| Some((5, &scene.previews.get(&13)?.0, m, bolt_metal.as_ref())));
			self.draw_panel_target(enc, cam, scene, &self.popup_panel, bolt.as_slice());
		}
		// BRIDGE-37 (b): the shop portrait target, cleared every shop frame, the posed actor (world-space vertices) when present.
		if self.hud.iter().any(|q| q.tex == PORTRAIT_PANEL_TEX) {
			let ident = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
			let actor = scene.portrait.as_ref().filter(|_| self.portrait_shown).map(|b| (6, b, ident, None));
			self.draw_panel_target(enc, cam, scene, &self.portrait_panel, actor.as_slice());
		}
	}

	/// NOV-35b / BRIDGE-45: the class-13 metal faces of a shop icon or popup bolt placed at `m` with draw sphere `sphere`
	/// (`Game::shop_bolt_spheres`): model space and the stored (bind) vertices like its LOD preview, alpha from metal byte 0x18
	/// (no bolt_update on these slots) and the frame view's depth, env matrix from that view, colour from the preview's light
	/// word (menu slot 14, no third light). None: no class-13 metal, no view or sphere, or the alpha skips the metal part.
	fn panel_metal(&self, scene: &Scene, m: &[f32; 16], sphere: Option<([f32; 3], f32)>) -> Option<Batch> {
		let (packets, view, (c, w)) = (self.metal.classes.get(&13)?, scene.clip_view?, sphere?);
		let d: [f32; 3] = std::array::from_fn(|k| c[k] - view.e[k] * 1024.0);
		let alpha = metal_alpha(0x18, view.f[0] * d[0] + view.f[1] * d[1] + view.f[2] * d[2], w)?;
		let e = mobies::metal_env(view.e, metal_view_rows(&view), c, axes(m));
		// Slot 14 as `draw_panel_target` lights the LOD faces (retail level-light row in retail mode, else the menu row).
		let a = match self.reserved_lights {
			Some(r) => {
				let r = r[MENU_LIGHT_SLOT - 12];
				([r[1][0], r[1][1], r[1][2]], [r[0][0], r[0][1], r[0][2]])
			}
			None => (MENU_LIGHT_DIR.map(|x| -x), MENU_LIGHT_COLOUR),
		};
		let rows = [a, ([0.0; 3], [0.0; 3]), ([0.0; 3], [0.0; 3])];
		let (mut verts, mut runs) = (Vec::new(), BTreeMap::new());
		push_metal(&mut verts, &mut runs, packets, None, &rows, e, m, 1.0, alpha, false);
		let mut batch = None;
		upload_metal(&self.device, &self.queue, &mut batch, &mut (0, 0), &verts, runs);
		batch
	}

	/// One 512x128 target pass: clear (colour 0, depth far), then each `(uniform slot, batch, model, metal batch)` in order,
	/// depth-tested, with the main projection centred on the target (NOV-30a); a model's metal faces follow its LOD faces.
	fn draw_panel_target(&self, enc: &mut wgpu::CommandEncoder, cam: &Camera, scene: &Scene, target: &ItemPanel, models: &[(usize, &Batch, [f32; 16], Option<&Batch>)]) {
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("target panel"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: &target.msaa,
				resolve_target: Some(&target.resolved),
				depth_slice: None,
				ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Discard },
			})],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
				view: &target.depth,
				depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
				stencil_ops: None,
			}),
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		for &(slot, batch, ref m, metal) in models {
			// NOV-30a: the main projection (scalar 0.63, PAL 0.756) centred on the target, not a target projection.
			let pcam = Camera { tan_half: cam.tan_half * PANEL_TAN_SCALE, ..*cam };
			let mut pu = self.uniforms_fog(&pcam, &Camera { pos: [0.0; 3], ..pcam }, PANEL_ASPECT, (RETAIL_NEAR, RETAIL_FAR), false);
			let vp = pu.view_proj;
			for (c, col) in pu.view_proj.iter_mut().enumerate() {
				*col = [0, 1, 2, 3].map(|row| (0..4).map(|j| vp[j][row] * m[4 * c + j]).sum());
			}
			let local = |p: [f32; 4]| {
				let d = [p[0] - m[12], p[1] - m[13], p[2] - m[14]];
				let l = [0, 1, 2].map(|c| m[4 * c] * d[0] + m[4 * c + 1] * d[1] + m[4 * c + 2] * d[2]);
				[l[0], l[1], l[2], p[3]]
			};
			pu.eye = local(pu.eye);
			for i in (0..pu.points.len()).step_by(2) {
				pu.points[i] = local(pu.points[i]);
			}
			// Light slot 14 (the panel models' light word 0x0e0e): retail mode reads the live level-light row (retail.rs
			// `reserved_lights`, Eudora (0.9, 0.9, 0.6) towards (-0.80, -0.14, -0.58)) as the moby draw does; else the menu row.
			// To-light directions go to model space through the unscaled rotation (m = s R: R^T d = m^T d / s).
			let s2 = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).max(1e-12);
			let local = |d: [f32; 4]| -> [f32; 4] {
				let v = [0, 1, 2].map(|k| (m[4 * k] * d[0] + m[4 * k + 1] * d[1] + m[4 * k + 2] * d[2]) / s2.sqrt());
				[v[0], v[1], v[2], 0.0]
			};
			pu.lights[MENU_LIGHT_SLOT] = match self.reserved_lights {
				Some(r) => {
					let r = r[MENU_LIGHT_SLOT - 12];
					[r[0], local(r[1]), r[2], local(r[3])]
				}
				None => {
					let (l, c) = (MENU_LIGHT_DIR, MENU_LIGHT_COLOUR);
					[[c[0], c[1], c[2], -0.25], local([-l[0], -l[1], -l[2], 0.0]), [0.0; 4], [0.0; 4]]
				}
			};
			self.queue.write_buffer(&self.pbuf[slot], 0, bytemuck::bytes_of(&pu));
			pass.set_bind_group(0, &self.pbind[slot], &[]);
			pass.set_vertex_buffer(0, batch.vbuf.slice(..));
			pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
			for pipeline in std::iter::once(&self.pipeline).chain(&self.moby_pipelines) {
				pass.set_pipeline(pipeline);
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				for d in &batch.draws {
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
				}
				if let (Some(mb), Some(tex)) = (metal, &self.metal.tex) {
					pass.set_vertex_buffer(0, mb.vbuf.slice(..));
					pass.set_index_buffer(mb.ibuf.slice(..), wgpu::IndexFormat::Uint32);
					for d in &mb.draws {
						pass.set_bind_group(1, &tex[d.texture], &[]);
						pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
					}
				}
			}
		}
	}

	pub(super) fn draw(&mut self, cam: &Camera, capture: bool) -> Option<crate::render::Image> {
		self.cull_shrubs(cam);
		self.cull_ties();
		self.cull_tfrags();
		let scene = self.scene.as_ref()?;
		let aspect = self.config.width as f32 / self.config.height as f32;
		let eye = Camera { pos: [0.0; 3], yaw: cam.yaw, pitch: cam.pitch, tan_half: cam.tan_half, up: cam.up };
		let u = self.uniforms(cam, &eye, aspect, (RETAIL_NEAR, RETAIL_FAR));
		self.queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));
		self.write_warp_uniforms(cam, aspect);
		let warp_hud = self.warp_hud(self.config.width as f32, self.config.height as f32);
		super::profile::lap(super::profile::Phase::Cull);
		let frame = match self.surface.get_current_texture() {
			wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
			wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
				configure_surface(&self.surface, &self.device, &self.config);
				return None;
			}
			_ => return None,
		};
		super::profile::lap(super::profile::Phase::Acquire);
		let view = frame.texture.create_view(&Default::default());
		let mut enc = self.device.create_command_encoder(&Default::default());
		self.draw_item_panel(&mut enc, cam, scene);
		// Overlay quads to clip space, one draw per run of the same texture.
		let (sw, sh) = (self.config.width as f32, self.config.height as f32);
		let (mut hv, mut hi, mut hd) = (Vec::new(), Vec::new(), Vec::<(Draw, bool)>::new());
		// BRIDGE-36 (c4): the draw group of the first quad after the prompt model (`hud_split`).
		let (split, mut split_group) = (self.hud_split.filter(|&s| s <= self.hud.len()), None);
		for (qi, q) in self.hud.iter().chain(&warp_hud).enumerate() {
			let at_split = split == Some(qi);
			if at_split {
				split_group = Some(hd.len());
			}
			let [x, y, w, h] = q.rect;
			let base = hv.len() as u32;
			let (cx, cy, (sa, ca)) = (x + w / 2.0, y + h / 2.0, q.angle.sin_cos());
			for (k, (px, py, u, v)) in [(x, y, q.uv[0], q.uv[1]), (x + w, y, q.uv[2], q.uv[1]), (x + w, y + h, q.uv[2], q.uv[3]), (x, y + h, q.uv[0], q.uv[3])].into_iter().enumerate() {
				// BRIDGE-74: four free corners (window pixels) replace the rotated rect, with the same UV pattern.
				let (px, py) = match q.corners {
					Some(c) => (c[k][0], c[k][1]),
					None => {
						let (dx, dy) = (px - cx, py - cy);
						(cx + dx * ca - dy * sa, cy + dx * sa + dy * ca)
					}
				};
				let [r, g, b, a] = q.colour;
				hv.push(Vertex { pos: [px / sw * 2.0 - 1.0, 1.0 - py / sh * 2.0, 0.0], uv: [u, v], normal: [0.0; 3], colour: [r, g, b, a.abs()], light: [0.0; 4] });
			}
			let add = q.colour[3] < 0.0;
			if let Some((d, _)) = hd.last_mut().filter(|(d, a)| !at_split && d.texture == q.tex && *a == add) {
				d.count += 6;
			} else {
				hd.push((Draw { inst: 0, cull: 0, texture: q.tex, start: hi.len() as u32, count: 6 }, add));
			}
			hi.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
		}
		if split == Some(self.hud.len()) && split_group.is_none() {
			split_group = Some(hd.len());
		}
		// [PC] perf round 3 (bindless.rs): this frame's tie and shrub draw lists as indirect arguments (multi-draw runs).
		let multi = match (&mut self.bindless, &scene.tex_array) {
			(Some(bl), Some(_)) if bl.indirect.is_some() => {
				let mut args = Vec::with_capacity(scene.ties.draws.len() + scene.shrub_opaque.len() + scene.shrub_faded.len());
				let ties = super::bindless::push_args(&mut args, scene.ties.draws.iter().copied(), true);
				let opaque = super::bindless::push_args(&mut args, scene.shrub_opaque.iter().map(|&k| scene.shrubs.draws[k]), true);
				let faded = super::bindless::push_args(&mut args, scene.shrub_faded.iter().map(|&k| scene.shrubs.draws[k]), false);
				bl.write_args(&self.device, &self.queue, &args);
				Some((ties, opaque, faded))
			}
			_ => None,
		};
		let hud_buffers = (!hv.is_empty()).then(|| {
			let vb = buffer(&self.device, "hud vertices", bytemuck::cast_slice(&hv), wgpu::BufferUsages::VERTEX);
			self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&hv));
			let ib = buffer(&self.device, "hud indices", bytemuck::cast_slice(&hi), wgpu::BufferUsages::INDEX);
			self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&hi));
			(vb, ib)
		});
		{
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("level"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &self.msaa,
					resolve_target: Some(&view),
					depth_slice: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(self.warp_fx.as_ref().filter(|_| scene.warp_shown).map_or(scene.clear, |w| w.clear)),
						store: wgpu::StoreOp::Discard,
					},
				})],
				depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
					view: &self.depth,
					depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
					// BRIDGE-64a: the shadow-volume counter starts at 128 every frame.
					stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(super::shadowvol::STENCIL_CLEAR), store: wgpu::StoreOp::Discard }),
				}),
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_bind_group(0, &self.ubind, &[]);
			// BRIDGE-35: while the arrival warp plays only its ship batch is drawn.
			let warp = scene.warp_shown;
			if warp {
				self.draw_warp_fan(&mut pass);
			}
			// The sky first, as retail: the sky pass FUN_0023fbb8 is the first draw of the frame render (aridia_presentation.md;
			// GSPIX play/eudora1_0039 (384,37): sky texel, then the tfrag texel blended over it). It writes no Z, so every world draw
			// covers it, and blended world texels (tfrag texels below alpha 0x60 write no Z) mix with the sky, not the clear colour
			// (Eudora cliff undersides, fx_census_late.md run 21). ARI-4 (FUN_00289d78): shells 0 and 1, the sky sprites (additive),
			// then shells 2 and 3.
			if let Some(sky) = scene.sky.as_ref().filter(|_| !warp && !dskip("sky")) {
				let split = scene.sky_shells.as_ref().map_or(sky.draws.len(), |s| s.split.min(sky.draws.len()));
				let shells = |pass: &mut wgpu::RenderPass, draws: &[Draw]| {
					pass.set_pipeline(&self.sky_pipeline);
					pass.set_vertex_buffer(0, sky.vbuf.slice(..));
					pass.set_index_buffer(sky.ibuf.slice(..), wgpu::IndexFormat::Uint32);
					for d in draws {
						pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
						pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
					}
				};
				shells(&mut pass, &sky.draws[..split]);
				if let Some(sp) = scene.sky_sprites.as_ref().filter(|sp| sp.counts.iter().any(|&c| c > 0)) {
					pass.set_pipeline(&self.sky_add_pipeline);
					pass.set_vertex_buffer(0, sp.vbuf.slice(..));
					pass.set_index_buffer(sp.ibuf.slice(..), wgpu::IndexFormat::Uint32);
					let mut first = 0;
					for (k, &n) in sp.counts.iter().enumerate() {
						if n > 0 {
							pass.set_bind_group(1, &scene.bind_groups[sp.bind[k]], &[]);
							pass.draw_indexed(first * 6..(first + n) * 6, 0, 0..1);
						}
						first += n;
					}
				}
				shells(&mut pass, &sky.draws[split..]);
			}
			let warp_batch = scene.warp_batch.as_ref().filter(|_| warp);
			let marker = (scene.marker_shown && !warp).then_some(&scene.marker);
			let props = (scene.props_shown && !warp).then_some(&scene.props);
			let debris = scene.debris.as_ref().filter(|_| scene.props_shown && !warp);
			let clipped = (scene.props_shown && !warp && !scene.clip_batch.draws.is_empty()).then_some(&scene.clip_batch);
			let skinned = self.props_gpu.as_ref().zip(scene.props_sj.as_ref());
			let array = self.bindless.as_ref().zip(scene.tex_array.as_ref());
			// BRIDGE-64a: with volumes this frame the deferred-list slots leave the opaque props draws for the redraw after them.
			// M3a shadows: in retail mode a frame whose only deferred caster is the hero still draws its volumes.
			let volumes = self.volume_buf.as_ref().filter(|_| self.volume_counts != (0, 0) && (props.is_some() && !scene.props_late.is_empty() || self.shadow.marker_late && marker.is_some()));
			let marker_late = marker.filter(|_| volumes.is_some() && self.shadow.marker_late);
			let (props_main, props_late) = match volumes {
				Some(_) => super::shadowvol::split_late(&scene.props.draws, &scene.slot_runs, &scene.props_late),
				None => (Vec::new(), Vec::new()),
			};
			for (bi, (pipeline, batch)) in [(&self.pipeline, Some(&scene.world).filter(|_| !warp)), (&self.pipeline, Some(&scene.ties).filter(|_| !warp)), (&self.tfrag_pipeline, Some(&scene.tfrags).filter(|_| !warp)), (&self.pipeline, marker.filter(|_| marker_late.is_none())), (&self.pipeline, props), (&self.pipeline, clipped), (&self.pipeline, debris), (&self.pipeline, warp_batch), (&self.tfrag_pipeline_lt, Some(&scene.tfrags).filter(|_| !warp))].into_iter().enumerate() {
				let Some(batch) = batch else { continue };
				if dskip(["world", "ties", "tfrags", "marker", "props", "clipped", "debris", "warp", "tfrags"][bi]) {
					continue;
				}
				// [PC] perf round 3 (bindless.rs): the tie draws through the texture array (their instance word is 0).
				if let Some((bl, tex_array)) = array.filter(|_| std::ptr::eq(batch, &scene.ties)) {
					pass.set_bind_group(1, tex_array, &[]);
					pass.set_pipeline(&bl.pipeline);
					pass.set_vertex_buffer(0, batch.vbuf.slice(..));
					pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
					if let Some((ties, _, _)) = &multi {
						bl.multi_draw(&mut pass, ties, None);
						continue;
					}
					for d in &batch.draws {
						if d.cull != 0 {
							pass.set_pipeline(&bl.pipeline_cull[d.cull as usize - 1]);
						}
						pass.draw_indexed(d.start..d.start + d.count, 0, d.texture as u32..d.texture as u32 + 1);
						if d.cull != 0 {
							pass.set_pipeline(&bl.pipeline);
						}
					}
					continue;
				}
				// [PC] perf round 3: the props batch is posed by `vs_props` (gpuskin.rs) unless RAC_CPU_SKIN=1.
				let (pipeline, cull) = match skinned.filter(|_| std::ptr::eq(batch, &scene.props)) {
					Some((pg, sj)) => {
						pass.set_bind_group(2, &pg.bind, &[]);
						pass.set_vertex_buffer(1, sj.slice(..));
						(&pg.pipeline, &pg.pipeline_cull)
					}
					None => (pipeline, &self.pipeline_cull),
				};
				pass.set_pipeline(pipeline);
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				let draws: &[Draw] = if volumes.is_some() && std::ptr::eq(batch, &scene.props) { &props_main } else { &batch.draws };
				for d in draws {
					if d.cull != 0 {
						pass.set_pipeline(&cull[d.cull as usize - 1]);
					}
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
					if d.cull != 0 {
						pass.set_pipeline(pipeline);
					}
				}
			}
			// BRIDGE-45 (NOV-35b): the props' metal faces (chrome / glass, clamp sampler) after the LOD batches.
			let metal = self.metal.props.as_ref().zip(self.metal.tex.as_ref()).filter(|_| scene.props_shown && !warp);
			if let Some((batch, tex)) = metal.filter(|_| !dskip("metal")) {
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				for d in &batch.draws {
					pass.set_pipeline(if d.cull != 0 { &self.pipeline_cull[d.cull as usize - 1] } else { &self.pipeline });
					pass.set_bind_group(1, &tex[d.texture], &[]);
					pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
				}
			}
			// Retail mode, Hoven: the queue-C water pools. The frame pass FUN_0021af28 (overlay.elf.c 10606-10700) flushes queue C
			// (FUN_0021be68) after the tfrag and tie passes and before the shrub pass FUN_0029f438, the moby pass and queue A; the shrubs
			// (ice floes) then blend over the pool (GSPIX at 12_0582, decomp/notes/fx_census_late.md).
			if !warp && !dskip("pools") {
				draw_strips(&self.device, &self.queue, &mut pass, &self.fx_tex, &self.pool_draws, &self.pool_pipeline, &self.pool_add_pipeline);
			}
			// REN-CULL-SHRUB: the shrub meshes after the sky (they are depth tested, and the faded / billboard fragments write no Z
			// everywhere), then the billboards: pass 1 with the Z-writing split, pass 2 never writing Z.
			if !warp && !dskip("shrub") && (!scene.shrub_opaque.is_empty() || !scene.shrub_faded.is_empty()) {
				pass.set_vertex_buffer(0, scene.shrubs.vbuf.slice(..));
				pass.set_index_buffer(scene.shrubs.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				// [PC] perf round 3 (bindless.rs): one texture-array bind group, the slot as the instance index.
				match array {
					Some((bl, tex_array)) if multi.is_some() => {
						pass.set_bind_group(1, tex_array, &[]);
						let (_, opaque, faded) = multi.as_ref().unwrap();
						bl.multi_draw(&mut pass, opaque, None);
						for p in &bl.shrub_faded {
							bl.multi_draw(&mut pass, faded, Some(p));
						}
					}
					Some((bl, tex_array)) => {
						pass.set_bind_group(1, tex_array, &[]);
						pass.set_pipeline(&bl.pipeline);
						for &k in &scene.shrub_opaque {
							let d = scene.shrubs.draws[k];
							if d.cull != 0 {
								pass.set_pipeline(&bl.pipeline_cull[d.cull as usize - 1]);
							}
							pass.draw_indexed(d.start..d.start + d.count, 0, d.texture as u32..d.texture as u32 + 1);
							if d.cull != 0 {
								pass.set_pipeline(&bl.pipeline);
							}
						}
						for p in &bl.shrub_faded {
							pass.set_pipeline(p);
							for &k in &scene.shrub_faded {
								let d = scene.shrubs.draws[k];
								let inst = super::bindless::first_instance(&d);
								pass.draw_indexed(d.start..d.start + d.count, 0, inst..inst + 1);
							}
						}
					}
					None => {
						pass.set_pipeline(&self.pipeline);
						for &k in &scene.shrub_opaque {
							let d = scene.shrubs.draws[k];
							if d.cull != 0 {
								pass.set_pipeline(&self.pipeline_cull[d.cull as usize - 1]);
							}
							pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
							pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
							if d.cull != 0 {
								pass.set_pipeline(&self.pipeline);
							}
						}
						for p in &self.shrub_pipelines[0..2] {
							pass.set_pipeline(p);
							for &k in &scene.shrub_faded {
								let d = scene.shrubs.draws[k];
								pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
								pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
							}
						}
					}
				}
			}
			if !warp && !dskip("bb") && !scene.shrub_bb_quads.is_empty() {
				let mut verts = Vec::with_capacity(scene.shrub_bb_quads.len() * 4);
				let mut runs: Vec<(u32, bool, u32, u32)> = Vec::new();
				for q in &scene.shrub_bb_quads {
					let base = verts.len() as u32 / 4 * 6;
					for (k, uv) in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].into_iter().enumerate() {
						verts.push(Vertex { pos: q.pos[k], uv, normal: [q.lod_k, q.mxl, 0.0], colour: q.colour, light: [0.0, 0.0, 0.0, -1.0] });
					}
					match runs.last_mut() {
						Some(r) if r.0 == q.class_id && r.1 == q.pass1 => r.3 += 6,
						_ => runs.push((q.class_id, q.pass1, base, 6)),
					}
				}
				let idx: Vec<u32> = (0..verts.len() as u32 / 4).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
				let vb = buffer(&self.device, "shrub billboard vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
				self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
				let ib = buffer(&self.device, "shrub billboard indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
				self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
				pass.set_vertex_buffer(0, vb.slice(..));
				pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
				for (pipes, want) in [(&self.shrub_pipelines[2..4], true), (&self.shrub_pipelines[4..5], false)] {
					for p in pipes {
						pass.set_pipeline(p);
						for &(class, pass1, start, count) in runs.iter().filter(|r| r.1 == want) {
							let _ = pass1;
							pass.set_bind_group(1, &scene.shrub_bb_tex[&class].0, &[]);
							pass.draw_indexed(start..start + count, 0, 0..1);
						}
					}
				}
			}
			// BRIDGE-64a: the shadow volumes and the darken, then the deferred-list slots drawn opaque over them.
			if let Some(vb) = volumes.filter(|_| !dskip("vol")) {
				pass.set_bind_group(0, &self.ubind, &[]);
				self.shadow.draw(&mut pass, vb, self.volume_counts);
			}
			// M3a shadows: the hero on retail's deferred path, drawn opaque after the darken.
			if let Some(batch) = marker_late.filter(|_| !dskip("hlate")) {
				pass.set_pipeline(&self.pipeline);
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				for d in &batch.draws {
					pass.set_pipeline(if d.cull != 0 { &self.pipeline_cull[d.cull as usize - 1] } else { &self.pipeline });
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
				}
			}
			if let (Some(_), Some(batch)) = (volumes.filter(|_| !dskip("plate")), props) {
				let (pipeline, cull) = match skinned {
					Some((pg, sj)) => {
						pass.set_bind_group(2, &pg.bind, &[]);
						pass.set_vertex_buffer(1, sj.slice(..));
						(&pg.pipeline, &pg.pipeline_cull)
					}
					None => (&self.pipeline, &self.pipeline_cull),
				};
				pass.set_pipeline(pipeline);
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				for d in &props_late {
					pass.set_pipeline(if d.cull != 0 { &cull[d.cull as usize - 1] } else { pipeline });
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
				}
			}
			// Retail draws every moby texel through one ABE pass: texels below alpha 0x80 blend here (MOBY-GS).
			if !dskip("abe") && (scene.props_shown || scene.marker_shown || warp) {
				for batch in [marker, props, clipped, debris, warp_batch].into_iter().flatten() {
					pass.set_vertex_buffer(0, batch.vbuf.slice(..));
					pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
					let (pipelines, culled) = match skinned.filter(|_| std::ptr::eq(batch, &scene.props)) {
						Some((pg, sj)) => {
							pass.set_bind_group(2, &pg.bind, &[]);
							pass.set_vertex_buffer(1, sj.slice(..));
							(&pg.moby_pipelines, &pg.moby_cull)
						}
						None => (&self.moby_pipelines, &self.moby_cull),
					};
					for (k, pipeline) in pipelines.iter().enumerate() {
						pass.set_pipeline(pipeline);
						for d in &batch.draws {
							// Retail culls in VU1 before the GS: the blended texels of a culled packet keep its cull mode.
							if d.cull != 0 {
								pass.set_pipeline(&culled[2 * (d.cull as usize - 1) + k]);
							}
							pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
							pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
							if d.cull != 0 {
								pass.set_pipeline(pipeline);
							}
						}
					}
				}
				// BRIDGE-45: the metal texels below alpha 0x80 and every metal face drawn with a metal alpha below 0x80.
				if let Some((batch, tex)) = metal {
					pass.set_vertex_buffer(0, batch.vbuf.slice(..));
					pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
					for pipeline in &self.moby_pipelines {
						pass.set_pipeline(pipeline);
						for d in &batch.draws {
							pass.set_bind_group(1, &tex[d.texture], &[]);
							pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
						}
					}
				}
			}
			pass.set_bind_group(0, &self.ubind, &[]);
			// ARI-49b (BRIDGE-63): the Aridia ocean, opaque and Z-writing after the world and mobies, before the
			// translucent passes; every patch strip as triangles (i, i+1, i+2), one draw.
			if let (false, Some((_, bind))) = (warp || self.ocean_draws.is_empty() || dskip("ocean"), &self.ocean_tex) {
				let mut verts = Vec::new();
				let mut idx: Vec<u32> = Vec::new();
				let ocean_light = self.ocean_fog.map_or([0.0; 4], |f| [f[0] as f32 / 255.0, f[1] as f32 / 255.0, f[2] as f32 / 255.0, 0.0]);
				for s in self.ocean_draws.iter().filter(|s| s.pos.len() >= 3) {
					let base = verts.len() as u32;
					verts.extend((0..s.pos.len()).map(|j| Vertex { pos: s.pos[j], uv: s.uv[j], normal: [0.0; 3], colour: s.colour[j], light: ocean_light }));
					idx.extend((0..s.pos.len() as u32 - 2).flat_map(|t| [base + t, base + t + 1, base + t + 2]));
				}
				if !idx.is_empty() {
					let vb = buffer(&self.device, "ocean vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
					let ib = buffer(&self.device, "ocean indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
					self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
					self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
					// KER-30: every strip carries the ocean's FIX; below 0x80 the blended pair with constant FIX / 128.
					let fix = self.ocean_draws[0].fix;
					let blended = fix < 0x80;
					if blended {
						let k = crate::ocean::blend_constant(fix);
						pass.set_blend_constant(wgpu::Color { r: k, g: k, b: k, a: 1.0 });
					}
					pass.set_pipeline(&self.ocean_pipeline[if self.ocean_fog.is_some() && !self.ocean_level_fog { 4 + blended as usize } else { blended as usize * 2 + self.ocean_level_fog as usize }]);
					pass.set_vertex_buffer(0, vb.slice(..));
					pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
					pass.set_bind_group(1, bind, &[]);
					pass.draw_indexed(0..idx.len() as u32, 0, 0..1);
				}
			}
			// Retail mode: the Rilgar, Umbris and Gaspar oceans, one draw per batch (own texture, FIX blend without Z when FIX < 0x80).
			if !warp && !dskip("oceang") {
				for b in self.ocean_ram.iter().filter(|b| !b.idx.is_empty() || !b.pidx.is_empty()) {
					let blended = b.fix < 0x80;
					if blended {
						let k = crate::ocean::blend_constant(b.fix);
						pass.set_blend_constant(wgpu::Color { r: k, g: k, b: k, a: 1.0 });
					}
					pass.set_bind_group(1, &b.bind, &[]);
					if !b.idx.is_empty() {
						let vb = buffer(&self.device, "ocean ram vertices", bytemuck::cast_slice(&b.verts), wgpu::BufferUsages::VERTEX);
						let ib = buffer(&self.device, "ocean ram indices", bytemuck::cast_slice(&b.idx), wgpu::BufferUsages::INDEX);
						self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&b.verts));
						self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&b.idx));
						pass.set_pipeline(&self.ocean_g_pipeline[blended as usize]);
						pass.set_vertex_buffer(0, vb.slice(..));
						pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
						pass.draw_indexed(0..b.idx.len() as u32, 0, 0..1);
					}
					if !b.pidx.is_empty() {
						let pvb = buffer(&self.device, "ocean ram patch vertices", bytemuck::cast_slice(&b.pverts), wgpu::BufferUsages::VERTEX);
						let pib = buffer(&self.device, "ocean ram patch indices", bytemuck::cast_slice(&b.pidx), wgpu::BufferUsages::INDEX);
						self.queue.write_buffer(&pvb, 0, bytemuck::cast_slice(&b.pverts));
						self.queue.write_buffer(&pib, 0, bytemuck::cast_slice(&b.pidx));
						pass.set_pipeline(&self.ocean_g_pipeline[2 + blended as usize]);
						pass.set_vertex_buffer(0, pvb.slice(..));
						pass.set_index_buffer(pib.slice(..), wgpu::IndexFormat::Uint32);
						pass.draw_indexed(0..b.pidx.len() as u32, 0, 0..1);
					}
				}
			}
			// Textured world quads, one draw per run of the same texture and blend.
			for fogged in [false, true].into_iter().filter(|_| !dskip("quads")) {
				let quads = if fogged { &self.water_quads } else { &self.fx_quads };
				// SCN-4.2: while the warp plays its own quads (big quad, ribbons, gloss) use the warp's texture table.
				let warp_fx = self.warp_fx.as_ref().filter(|_| warp);
				let table = warp_fx.map_or(&self.fx_tex, |w| &w.quad_tex);
				let quads: &[_] = if warp { warp_fx.filter(|_| !fogged).map_or(&[], |w| &w.quads) } else { quads };
				let mut verts = Vec::with_capacity(quads.len() * 4);
				let mut runs: Vec<(usize, bool, u32, u32)> = Vec::new();
				for q in quads {
					if !table.contains_key(&q.tex) {
						continue;
					}
					let base = verts.len() as u32 / 4 * 6;
					for k in 0..4 {
						verts.push(Vertex { pos: q.pos[k], uv: q.uv[k], normal: [0.0; 3], colour: q.colour[k], light: [0.0; 4] });
					}
					match runs.last_mut() {
						Some(r) if r.0 == q.tex && r.1 == q.additive => r.3 += 6,
						_ => runs.push((q.tex, q.additive, base, 6)),
					}
				}
				if !verts.is_empty() {
					let idx: Vec<u32> = (0..verts.len() as u32 / 4).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
					let vb = buffer(&self.device, "fx quad vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
					self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
					let ib = buffer(&self.device, "fx quad indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
					self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
					pass.set_vertex_buffer(0, vb.slice(..));
					pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
					for (tex, additive, start, count) in runs {
						pass.set_pipeline(&self.part_pipelines[if fogged { 2 } else { additive as usize }]);
						pass.set_bind_group(1, &table[&tex], &[]);
						pass.draw_indexed(start..start + count, 0, 0..1);
					}
				}
			}
			// BRIDGE-23: FIX-blend world strips (NOV-15.1b), triangles (i, i+1, i+2), one draw per strip.
			if !warp && !dskip("strips") {
				draw_strips(&self.device, &self.queue, &mut pass, &self.fx_tex, &self.strip_draws, &self.strip_pipeline, &self.strip_pipeline);
			}
			// NOV-13.3: the vendor shop's item models, world mobies seen by the world camera (decomp/notes/vendor.md
			// "NOV-13.3"). Their meshes are the model-space preview batches, so the view-projection takes the model
			// matrix and the eye, the point lights and the light direction move into the model frame. Lit by the
			// preview's fixed menu light [P] (the shop slots' moby lighting is not mapped).
			for (k, (class, m)) in self.shop_models.iter().take(if warp { 0 } else { 2 }).enumerate() {
				let Some((batch, ..)) = scene.previews.get(class) else { continue };
				let slot = 2 + k;
				let mut pu = self.uniforms(cam, &eye, aspect, (RETAIL_NEAR, RETAIL_FAR));
				let vp = pu.view_proj;
				for (c, col) in pu.view_proj.iter_mut().enumerate() {
					*col = [0, 1, 2, 3].map(|row| (0..4).map(|j| vp[j][row] * m[4 * c + j]).sum());
				}
				let local = |p: [f32; 4]| {
					let d = [p[0] - m[12], p[1] - m[13], p[2] - m[14]];
					let l = [0, 1, 2].map(|c| m[4 * c] * d[0] + m[4 * c + 1] * d[1] + m[4 * c + 2] * d[2]);
					[l[0], l[1], l[2], p[3]]
				};
				pu.eye = local(pu.eye);
				for i in (0..pu.points.len()).step_by(2) {
					pu.points[i] = local(pu.points[i]);
				}
				let (l, c) = (MENU_LIGHT_DIR, MENU_LIGHT_COLOUR);
				let to_light = [0, 1, 2].map(|k| -(m[4 * k] * l[0] + m[4 * k + 1] * l[1] + m[4 * k + 2] * l[2]));
				pu.lights[MENU_LIGHT_SLOT] = [[c[0], c[1], c[2], -0.25], [to_light[0], to_light[1], to_light[2], 0.0], [0.0; 4], [0.0; 4]];
				self.queue.write_buffer(&self.pbuf[slot], 0, bytemuck::bytes_of(&pu));
				pass.set_pipeline(&self.pipeline);
				pass.set_bind_group(0, &self.pbind[slot], &[]);
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				for d in &batch.draws {
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
				}
				// Texels below alpha 0x80 blend (MOBY-GS), as in the scene pass.
				for pipeline in &self.moby_pipelines {
					pass.set_pipeline(pipeline);
					for d in &batch.draws {
						pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
						pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
					}
				}
			}
			// One vertex / index buffer for every run (the retail pool order makes many small runs), one draw per run;
			// retail pool lines (`LINE_KEY` runs) come from their own vertex buffer, drawn in walk order.
			let mut all: Vec<Vertex> = Vec::new();
			let mut lall: Vec<Vertex> = Vec::new();
			let mut runs: Vec<(bool, bool, u32, u32, u32)> = Vec::new();
			for ((_, additive, key), verts) in self.part_verts.iter().filter(|_| !warp) {
				if verts.is_empty() {
					continue;
				}
				if *key == LINE_KEY {
					runs.push((true, false, *key, lall.len() as u32, verts.len() as u32));
					lall.extend_from_slice(verts);
					continue;
				}
				if !(self.fx_tex.contains_key(&(*key as usize)) || self.part_tex.is_some()) {
					continue;
				}
				runs.push((false, *additive, *key, all.len() as u32 / 4 * 6, verts.len() as u32 / 4 * 6));
				all.extend_from_slice(verts);
			}
			if !runs.is_empty() {
				let tris = (!all.is_empty()).then(|| {
					let idx: Vec<u32> = (0..all.len() as u32 / 4).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
					let vb = buffer(&self.device, "particle vertices", bytemuck::cast_slice(&all), wgpu::BufferUsages::VERTEX);
					self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&all));
					let ib = buffer(&self.device, "particle indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
					self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
					(vb, ib)
				});
				let lvb = (!lall.is_empty()).then(|| {
					let vb = buffer(&self.device, "pool lines", bytemuck::cast_slice(&lall), wgpu::BufferUsages::VERTEX);
					self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&lall));
					vb
				});
				let any_tex = self.part_tex.as_ref().or_else(|| self.fx_tex.values().next());
				for (line, additive, key, first, count) in runs {
					if line {
						let (Some(vb), Some(tex)) = (lvb.as_ref(), any_tex) else { continue };
						pass.set_pipeline(&self.line_pipeline);
						pass.set_bind_group(1, tex, &[]);
						pass.set_vertex_buffer(0, vb.slice(..));
						pass.draw(first..first + count, 0..1);
						continue;
					}
					let (Some((vb, ib)), Some(tex)) = (tris.as_ref(), self.fx_tex.get(&(key as usize)).or(self.part_tex.as_ref())) else { continue };
					pass.set_pipeline(&self.part_pipelines[additive as usize]);
					pass.set_bind_group(1, tex, &[]);
					pass.set_vertex_buffer(0, vb.slice(..));
					pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
					pass.draw_indexed(first..first + count, 0, 0..1);
				}
			}
			// KER-92: the pool lines (untextured; any texture bind group satisfies the layout).
			if let (false, Some(tex)) = (warp || self.line_verts.is_empty(), self.part_tex.as_ref().or_else(|| self.fx_tex.values().next())) {
				let vb = buffer(&self.device, "particle lines", bytemuck::cast_slice(&self.line_verts), wgpu::BufferUsages::VERTEX);
				self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&self.line_verts));
				pass.set_pipeline(&self.line_pipeline);
				pass.set_bind_group(1, tex, &[]);
				pass.set_vertex_buffer(0, vb.slice(..));
				pass.draw(0..self.line_verts.len() as u32, 0..1);
			}
			// BRIDGE-36 (c5): the HUD groups before the split, then the prompt model (world depth, world uniforms), then the rest.
			let draw_hud = |pass: &mut wgpu::RenderPass<'_>, groups: &[(Draw, bool)]| {
				let Some((vb, ib)) = &hud_buffers else { return };
				pass.set_vertex_buffer(0, vb.slice(..));
				pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
				for (d, add) in groups {
					if d.texture == RETAIL_OV_TEX {
						let Some((_, bind, _, _)) = &self.retail_ov else { continue };
						pass.set_pipeline(&self.hud_pipeline[3]);
						pass.set_bind_group(1, bind, &[]);
					} else if d.texture == RETAIL_FILTER_TEX {
						let Some((_, bind, _, _)) = &self.retail_ov else { continue };
						pass.set_pipeline(&self.hud_pipeline[4]);
						pass.set_bind_group(1, bind, &[]);
					} else if d.texture == ITEM_PANEL_TEX || d.texture == POPUP_PANEL_TEX || d.texture == PORTRAIT_PANEL_TEX {
						// Retail mode: added onto the GS overlay's black panel interior (the overlay holds retail's panel text).
						pass.set_pipeline(&self.hud_pipeline[if self.retail_ov.is_some() { 1 } else { 2 }]);
						pass.set_bind_group(1, if d.texture == ITEM_PANEL_TEX { &self.item_panel.bind } else if d.texture == POPUP_PANEL_TEX { &self.popup_panel.bind } else { &self.portrait_panel.bind }, &[]);
					} else {
						pass.set_pipeline(&self.hud_pipeline[*add as usize]);
						pass.set_bind_group(1, &self.hud_textures[d.texture.min(self.hud_textures.len() - 1)], &[]);
					}
					pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
				}
			};
			let g = split_group.unwrap_or(hd.len());
			draw_hud(&mut pass, &hd[..g]);
			if let (Some(_), Some(prompt), false) = (split_group, scene.prompt.as_ref(), warp) {
				pass.set_bind_group(0, &self.ubind, &[]);
				pass.set_vertex_buffer(0, prompt.vbuf.slice(..));
				pass.set_index_buffer(prompt.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				pass.set_pipeline(&self.pipeline);
				for d in &prompt.draws {
					if d.cull != 0 {
						pass.set_pipeline(&self.pipeline_cull[d.cull as usize - 1]);
					}
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
					if d.cull != 0 {
						pass.set_pipeline(&self.pipeline);
					}
				}
				for pipeline in &self.moby_pipelines {
					pass.set_pipeline(pipeline);
					for d in &prompt.draws {
						pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
						pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
					}
				}
			}
			draw_hud(&mut pass, &hd[g..]);
			// Pause-menu item preview: a scoped viewport inside the panel, depth reset first. The hero
			// (keys >= HERO_PREVIEW) and the panel-3 item use the frozen menu camera (ingame_pause_menu.md "Menu
			// camera", "Panel 3 item model"); the vendor preview keeps a fitted turntable camera [P].
			for (slot, (class, r, t, item)) in [self.preview, self.preview2.map(|(c, r, t)| (c, r, t, None)), self.preview_pair, self.preview_worn].into_iter().enumerate().filter_map(|(i, p)| Some((i, p?))) {
				if let Some((batch, centre, radius)) = scene.previews.get(&class) {
					let (x0, y0) = (r[0].max(0.0).floor(), r[1].max(0.0).floor());
					let (x1, y1) = ((r[0] + r[2]).min(sw).floor(), (r[1] + r[3]).min(sh).floor());
					if x1 > x0 + 1.0 && y1 > y0 + 1.0 {
						let (vw, vh) = (x1 - x0, y1 - y0);
						let q = |x: f32, y: f32| Vertex { pos: [x / sw * 2.0 - 1.0, 1.0 - y / sh * 2.0, 1.0], uv: [0.0, 0.0], normal: [0.0; 3], colour: [0.0; 4], light: [0.0; 4] };
						let verts = [q(x0, y0), q(x1, y0), q(x1, y1), q(x0, y1)];
						let vb = buffer(&self.device, "preview clear vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
						self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
						let ib = buffer(&self.device, "preview clear indices", bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]), wgpu::BufferUsages::INDEX);
						self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]));
						let fixed = class >= HERO_PREVIEW || item.is_some();
						let mut cam = Camera { pos: [0.0; 3], yaw: if fixed { std::f32::consts::PI } else { t * 0.9 }, pitch: if fixed { 0.0 } else { -0.25 }, tan_half: 0.5, up: [0.0, 0.0, 1.0] };
						if fixed {
							// Menu scene (FUN_00275ab8 / FUN_002816a8) [C]: camera at (256,256,64), identity basis
							// (looks +X), projected with the matrix frozen at pause open (zoom 0.63, PAL 0.756; the
							// RT bind does not rebuild it). Ratchet at camera + (4, 0, -0.6), yaw pi: in his frame the
							// camera sits at (4, 0, 0.6) facing -X. Square pixels [PC] port choice.
							cam.pos = [4.0, 0.0, 0.6];
							cam.tan_half = menu_tan_half_y(r[2] * 448.0 / sh, r[3] * 448.0 / sh);
							if item.is_some() {
								// Panel 3 blits with flag 8, a centred crop of the RT at the full-screen scale [I].
								cam = Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: r[3] * 448.0 / sh / 2.0 / 224.0 * 0.63 * 0.756, up: [0.0, 0.0, 1.0] };
							}
						} else {
							let f = forward(&cam);
							let dist = radius / (cam.tan_half * (vw / vh).min(1.0)) * 1.15;
							cam.pos = [centre[0] - f[0] * dist, centre[1] - f[1] * dist, centre[2] - f[2] * dist];
						}
						let mut pu = self.uniforms(&cam, &cam, vw / vh, (0.1, (*radius * 4.0).max(100.0))); // [PC] model preview range
						pu.fog = [0.0, 1.0e9, 255.0, 255.0];
						// Slot 14, set at pause open (FUN_0028ba40), in world axes = the menu camera's; turned into
						// the model's frame below (the hero stands at yaw pi; the panel-3 model by its rotation).
						let d = MENU_LIGHT_DIR;
						let mut to_light = match item {
							Some(_) => [0.0; 3],
							None if fixed => [d[0], d[1], -d[2]],
							None => [-d[0], -d[1], -d[2]],
						};
						if let Some(id) = item {
							// FUN_0027bc70: rot (row rx, row ry, t), pos = camera + (d + a cos t - b sin t,
							// y + a sin t + b cos t, z); normals stay unrotated [P].
							let [_, d, y, z, rx, ry, a, b] = PANEL3_VIEW[(id as usize).min(PANEL3_VIEW.len() - 1)];
							let (s, c) = t.sin_cos();
							let m = place([d + a * c - b * s, y + a * s + b * c, z], [rx, ry, t]);
							let l = MENU_LIGHT_DIR;
							to_light = [0, 1, 2].map(|k| -(m[4 * k] * l[0] + m[4 * k + 1] * l[1] + m[4 * k + 2] * l[2]));
							let vp = pu.view_proj;
							for (k, col) in pu.view_proj.iter_mut().enumerate() {
								*col = [0, 1, 2, 3].map(|row| (0..4).map(|j| vp[j][row] * m[4 * k + j]).sum());
							}
						}
						let c = MENU_LIGHT_COLOUR;
						pu.lights[MENU_LIGHT_SLOT] = [[c[0], c[1], c[2], -0.25], [to_light[0], to_light[1], to_light[2], 0.0], [0.0; 4], [0.0; 4]];
						// LIT-7a: the gold weapon's slot-12 light, same direction as slot 14, wrap -0.3, colour B zero.
						let gold = slot == 0 && self.preview_gold && item.is_some();
						let batch = if gold { scene.previews_gold.get(&class).unwrap_or(batch) } else { batch };
						if gold {
							let g = gold_light_colour();
							pu.lights[GOLD_LIGHT_SLOT] = [[g[0], g[1], g[2], GOLD_LIGHT_WRAP], [to_light[0], to_light[1], to_light[2], 0.0], [0.0; 4], [0.0; 4]];
						}
						self.queue.write_buffer(&self.pbuf[slot], 0, bytemuck::bytes_of(&pu));
						pass.set_scissor_rect(x0 as u32, y0 as u32, vw as u32, vh as u32);
						// The worn batch draws over the hero without resetting the depth.
						if class != HERO_WORN {
							pass.set_pipeline(&self.clear_pipeline);
							pass.set_bind_group(0, &self.ubind, &[]);
							pass.set_bind_group(1, &self.hud_textures[0], &[]);
							pass.set_vertex_buffer(0, vb.slice(..));
							pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
							pass.draw_indexed(0..6, 0, 0..1);
						}
						pass.set_viewport(x0, y0, vw, vh, 0.0, 1.0);
						pass.set_pipeline(&self.pipeline);
						pass.set_bind_group(0, &self.pbind[slot], &[]);
						pass.set_vertex_buffer(0, batch.vbuf.slice(..));
						pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
						for d in &batch.draws {
							pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
							pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
						}
						for pipeline in &self.moby_pipelines {
							pass.set_pipeline(pipeline);
							for d in &batch.draws {
								pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
								pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
							}
						}
					}
				}
			}
		}
		self.movie.draw(&self.queue, &mut enc, &view, sw, sh);
		let (w, h) = (self.config.width, self.config.height);
		let row = (w * 4).next_multiple_of(256);
		let readback = capture.then(|| {
			let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("shot"), size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
			enc.copy_texture_to_buffer(
				frame.texture.as_image_copy(),
				wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
				wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
			);
			buf
		});
		super::profile::lap(super::profile::Phase::Encode);
		let cmd = enc.finish();
		let _t = super::profile::span("submit");
		self.queue.submit([cmd]);
		drop(_t);
		self.queue.present(frame);
		super::profile::lap(super::profile::Phase::Present);
		let buf = readback?;
		buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
		self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
		let data = buf.slice(..).get_mapped_range().ok()?;
		let bgr = matches!(self.config.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
		let mut rgb = Vec::with_capacity((w * h * 3) as usize);
		for y in 0..h as usize {
			for px in data[y * row as usize..][..w as usize * 4].chunks_exact(4) {
				rgb.extend(if bgr { [px[2], px[1], px[0]] } else { [px[0], px[1], px[2]] });
			}
		}
		Some(crate::render::Image { w: w as usize, h: h as usize, rgb })
	}
}

/// KER-92: pool draw mode 2 cull. The depth key is the view z of the segment midpoint (`f` = view forward); the line is
/// drawn iff `key - near > 0` and `key - far < 0` with far = min(line far, the pool clamp `part_far`), and both points lie
/// inside the view frustum (`m` = view-projection, depth 0..1).
/// BRIDGE-23: FIX-blend strips, triangles (i, i+1, i+2), one draw per strip; the blend constant is FIX / 128.
fn draw_strips(device: &wgpu::Device, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, fx_tex: &HashMap<usize, wgpu::BindGroup>, strips: &[particles::WorldStrip], pipeline: &wgpu::RenderPipeline, add_pipeline: &wgpu::RenderPipeline) {
	for s in strips {
		let Some(bind) = fx_tex.get(&s.tex) else { continue };
		if s.pos.len() < 3 {
			continue;
		}
		let verts: Vec<Vertex> = (0..s.pos.len()).map(|j| Vertex { pos: s.pos[j], uv: s.uv[j], normal: [0.0; 3], colour: s.colour[j], light: [0.0; 4] }).collect();
		let idx: Vec<u32> = (0..s.pos.len() as u32 - 2).flat_map(|t| [t, t + 1, t + 2]).collect();
		let vb = buffer(device, "fx strip vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
		queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
		let ib = buffer(device, "fx strip indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
		queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
		pass.set_pipeline(if s.add { add_pipeline } else { pipeline });
		let k = s.fix as f64 / 128.0;
		pass.set_blend_constant(wgpu::Color { r: k, g: k, b: k, a: 1.0 });
		pass.set_vertex_buffer(0, vb.slice(..));
		pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
		pass.set_bind_group(1, bind, &[]);
		pass.draw_indexed(0..idx.len() as u32, 0, 0..1);
	}
}

/// [probe] RAC_DRAWSKIP=a,b,.. leaves those main-pass draw groups out (fx lane run 16): world ties tfrags marker props clipped
/// debris warp sky metal pools shrub bb vol hlate plate abe ocean oceang quads strips.
fn dskip(name: &str) -> bool {
	static S: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
	S.get_or_init(|| std::env::var("RAC_DRAWSKIP").map(|v| v.split(',').map(str::to_owned).collect()).unwrap_or_default()).iter().any(|s| s == name)
}

/// Silent probes (env-gated): RAC_TFRAG_SKIP=i skips fragment i, RAC_TFRAG_SKIPBIT=b skips every fragment whose index has bit b set
/// (bisect which fragment draws a pixel), RAC_TFRAG_INFO=i prints that fragment's cull facts each frame.
fn tfrag_probe_skip(i: usize) -> bool {
	static P: std::sync::OnceLock<(Option<usize>, Option<u32>)> = std::sync::OnceLock::new();
	let p = P.get_or_init(|| (std::env::var("RAC_TFRAG_SKIP").ok().and_then(|v| v.parse().ok()), std::env::var("RAC_TFRAG_SKIPBIT").ok().and_then(|v| v.parse().ok())));
	p.0 == Some(i) || p.1.is_some_and(|b| (i >> b) & 1 == 1)
}

fn tfrag_probe_info(i: usize) -> bool {
	static P: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
	*P.get_or_init(|| std::env::var("RAC_TFRAG_INFO").ok().and_then(|v| v.parse().ok())) == Some(i)
}

fn line_shown(m: &[[f32; 4]; 4], cam: [f32; 3], f: [f32; 3], l: &particles::Line, part_far: f32) -> bool {
	let mid: [f32; 3] = std::array::from_fn(|k| (l.a[k] + l.b[k]) * 0.5 - cam[k]);
	let key = mid[0] * f[0] + mid[1] * f[1] + mid[2] * f[2];
	let far = l.far.min(part_far);
	if key - l.near <= 0.0 || key - far >= 0.0 {
		return false;
	}
	clip_inside(m, l.a) && clip_inside(m, l.b)
}

/// The pool's two-point clip test (`vclip` of each point, no flag set): inside the view frustum.
fn clip_inside(m: &[[f32; 4]; 4], p: [f32; 3]) -> bool {
	let c: [f32; 4] = std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r]);
	c[3] > 0.0 && c[0].abs() <= c[3] && c[1].abs() <= c[3] && (0.0..=c[3]).contains(&c[2])
}

/// `part_verts` texture key of a retail pool line run (drawn with `line_pipeline`, 2 vertices per line).
const LINE_KEY: u32 = u32::MAX - 1;

/// `set_props`: everything an unposed prop slot's vertices are built from besides its model, as raw bits (placement,
/// scale, light word, alpha code and byte, class tint, prop tint).
type SlotKey = [u32; 32];

fn slot_key(m: &[f32; 16], scale: f32, light: [f32; 4], alpha_code: f32, alpha: u8, tint: Option<([f32; 3], usize)>, prop_tint: Option<[f32; 3]>) -> SlotKey {
	let mut k = [0u32; 32];
	for i in 0..16 {
		k[i] = m[i].to_bits();
	}
	k[16] = scale.to_bits();
	for i in 0..4 {
		k[17 + i] = light[i].to_bits();
	}
	k[21] = alpha_code.to_bits();
	k[22] = alpha as u32;
	if let Some((c, start)) = tint {
		k[23] = 1;
		k[24..27].copy_from_slice(&c.map(f32::to_bits));
		k[27] = start as u32;
	}
	if let Some(c) = prop_tint {
		k[28] = 1;
		k[29..32].copy_from_slice(&c.map(f32::to_bits));
	}
	k
}

/// A hidden prop slot's vertices sit far below the world (collapsed to a point).
pub(super) const HIDDEN_POS: [f32; 3] = [0.0, 0.0, -1.0e6];

/// `set_props`: slot `slot` (vertex range `r`) becomes the plain hidden copy of the model, rewritten only when it is not
/// one already.
/// `set_props`: one shown prop slot's rebuild from the model (pose, placement, colour).
pub(super) struct SlotJob<'a> {
	pub(super) range: std::ops::Range<usize>,
	/// The pose's first positions and normals (empty: the model's).
	pub(super) ps: &'a [[f32; 3]],
	pub(super) ns: &'a [[f32; 3]],
	pub(super) m: [f32; 16],
	pub(super) scale: f32,
	pub(super) light: [f32; 4],
	pub(super) alpha_code: f32,
	pub(super) prop_tint: Option<[f32; 3]>,
	pub(super) tint: Option<([f32; 3], usize)>,
	pub(super) alpha: u8,
	/// WPN-6.3e: the aux 179 texture scroll added to every vertex V.
	pub(super) dv: f32,
	/// The class U scroll of a retail-mode class block (`retail::asset_feed`), added to every vertex U.
	pub(super) du: f32,
}

impl SlotJob<'_> {
	/// One pass per vertex: model copy, pose, placement, colour (PC perf: no separate copy and pose passes).
	/// `dst` / `model`: the slot's vertices from `off` on.
	pub(super) fn fill(&self, dst: &mut [Vertex], model: &[Vertex], off: usize) {
		let (m, scale) = (&self.m, self.scale);
		for (j, (v, m0)) in dst.iter_mut().zip(model).enumerate().map(|(j, x)| (j + off, x)) {
			*v = *m0;
			if j < self.ps.len() {
				v.pos = self.ps[j];
				v.normal = self.ns[j];
			}
			v.uv[0] += self.du;
			v.uv[1] += self.dv;
			v.colour[3] = self.alpha_code;
			v.light = self.light;
			let (a, n) = (v.pos.map(|c| c * scale), v.normal);
			for i in 0..3 {
				v.pos[i] = m[i] * a[0] + m[4 + i] * a[1] + m[8 + i] * a[2] + m[12 + i];
				v.normal[i] = m[i] * n[0] + m[4 + i] * n[1] + m[8 + i] * n[2];
			}
			if let Some(c) = self.prop_tint {
				for i in 0..3 {
					v.colour[i] *= c[i];
				}
			}
			v.colour = tint_override(v.colour, j, self.tint, self.alpha);
		}
	}
}

/// `set_props`: what one slot's vertices are built from this frame.
enum SlotSrc<'a> {
	/// The plain hidden copy of the model.
	Hidden,
	/// SCH-6b deferred-list overflow: the model with the pose's normals, collapsed to `HIDDEN_POS`.
	Deferred { ps: &'a [[f32; 3]], ns: &'a [[f32; 3]] },
	Prop(SlotJob<'a>),
	/// A scene actor: root-relative skinned pose at a world offset, its light word, dynamic-light id and tint.
	Actor { ps: &'a [[f32; 3]], ns: &'a [[f32; 3]], at: [f32; 3], light: [f32; 4], id: f32, tint: Option<([f32; 3], usize)> },
}

/// [PC] perf round 3: the slot records of `vs_props` (gpuskin.rs) and the pose entries they read, each distinct pose
/// (same position and normal buffers, same length) once.
/// `rad` gets each slot's bounding radius about its model origin for the clipper's sphere pre-test (`model_rad`: the model's;
/// infinite for an actor with unposed vertices).
fn build_records(srcs: &[SlotSrc], ranges: &[std::ops::Range<usize>], model_rad: &[f32], inst: &mut Vec<super::gpuskin::PropInst>, pose: &mut Vec<super::gpuskin::PoseEntry>, rad: &mut Vec<f32>) {
	use super::gpuskin::{PropInst, KIND_ACTOR, KIND_DEFERRED, KIND_PROP};
	inst.clear();
	pose.clear();
	rad.clear();
	let mut bases: HashMap<(usize, usize, usize), (u32, f32)> = HashMap::new();
	let mut add = |ps: &[[f32; 3]], ns: &[[f32; 3]], n: usize| -> (u32, f32) {
		*bases.entry((ps.as_ptr() as usize, ns.as_ptr() as usize, n)).or_insert_with(|| {
			let b = pose.len() as u32;
			pose.extend(ps[..n].iter().zip(&ns[..n]).map(|(p, q)| [p[0], p[1], p[2], q[0], q[1], q[2]]));
			(b, ps[..n].iter().map(|p| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()).fold(0.0, f32::max))
		})
	};
	for (si, (src, r)) in srcs.iter().zip(ranges).enumerate() {
		let len = r.len();
		let mrad = model_rad.get(si).copied().unwrap_or(f32::INFINITY);
		let mut bound = mrad;
		let mut add = |ps: &[[f32; 3]], ns: &[[f32; 3]], n: usize, actor: bool| -> u32 {
			let (b, pr) = add(ps, ns, n);
			bound = match (n == len, actor) {
				(true, _) => pr,
				(false, false) => pr.max(mrad),
				(false, true) => f32::INFINITY,
			};
			b
		};
		inst.push(match src {
			SlotSrc::Hidden => PropInst::hidden(),
			SlotSrc::Deferred { ps, ns } => {
				let n = ps.len().min(ns.len()).min(len);
				PropInst { k: [KIND_DEFERRED, u32::MAX, add(ps, ns, n, false), n as u32], ..PropInst::hidden() }
			}
			SlotSrc::Prop(job) => {
				let n = job.ps.len().min(job.ns.len()).min(len);
				PropInst {
					m: job.m,
					light: job.light,
					tint: job.tint.map_or([0.0; 4], |(c, _)| [c[0], c[1], c[2], moby_baked_encoding(job.alpha)]),
					ptint: job.prop_tint.map_or([0.0; 4], |c| [c[0], c[1], c[2], 1.0]),
					a: [job.scale, job.alpha_code, job.dv, job.du],
					k: [KIND_PROP, job.tint.map_or(u32::MAX, |(_, s)| s.min(u32::MAX as usize) as u32), add(job.ps, job.ns, n, false), n as u32],
				}
			}
			SlotSrc::Actor { ps, ns, at, light, id, tint } => {
				let n = ps.len().min(ns.len()).min(len);
				let mut m = [0.0; 16];
				m[12..15].copy_from_slice(at);
				PropInst {
					m,
					light: *light,
					tint: tint.map_or([0.0; 4], |(c, _)| [c[0], c[1], c[2], -2.0]),
					ptint: [0.0; 4],
					a: [0.0, *id, 0.0, 0.0],
					k: [KIND_ACTOR, tint.map_or(u32::MAX, |(_, s)| s.min(u32::MAX as usize) as u32), add(ps, ns, n, true), n as u32],
				}
			}
		});
		rad.push(bound);
	}
}

/// `clip_props`: the slots' vertices, as the CPU mirror or as the records `vs_props` poses.
#[derive(Clone, Copy)]
enum ClipSrc<'a> {
	Cpu(&'a [Vertex]),
	/// Records, pose entries and per-slot model-space bounding radii.
	Gpu(&'a [super::gpuskin::PropInst], &'a [super::gpuskin::PoseEntry], &'a [f32]),
}

fn hide_slot(verts: &mut [Vertex], model: &[Vertex], hidden: &mut [bool], dirty: &mut [bool], slot: usize, r: std::ops::Range<usize>) {
	if !hidden[slot] {
		for (v, m) in verts[r.clone()].iter_mut().zip(&model[r]) {
			*v = Vertex { pos: HIDDEN_POS, ..*m };
		}
		hidden[slot] = true;
		dirty[slot] = true;
	}
}

/// Initial capacity of the clipped-polygon overlay (vertices, indices); it only grows (NOV-5.9c Order 7).
const CLIP_VERTS: usize = 1024;
const CLIP_INDICES: usize = 3072;

/// NOV-5.9c driver: finds this frame's faces the retail clipper would take, queues their clipped polygons and masks the
/// original triangles out of `props.ibuf` (restoring those that are no longer clipped). `shown[pi]` = the slot is drawn.
/// MOBY-FADE: per prop the draw-distance fade alpha from the view (eye, forward) basis; None = past the limit (culled).
/// `Gpu::prop_slots`'s output, moved into the `Scene` fields of the same names.
struct PropSlots {
	ranges: Vec<std::ops::Range<usize>>,
	prompt_slot: Option<usize>,
	batch: Batch,
	model: Vec<Vertex>,
	slot_runs: Vec<Vec<(u32, u32, u32)>>,
	faces: Vec<Vec<super::clip::PropFace>>,
	sj: Option<wgpu::Buffer>,
	rad: Vec<f32>,
	classes: Vec<u32>,
	class_faces: HashMap<u32, ClassFaces>,
}

/// A class mesh's faces as `Gpu::batch` groups them: per texture key (ascending) its indices in mesh order, and the
/// clippable faces in mesh order as (group, rank in the group, mesh indices, strip phase) (`clip::replay_faces`).
pub(super) struct ClassFaces {
	groups: Vec<(u32, Vec<u32>)>,
	clipped: Vec<(usize, u32, [u32; 3], u8)>,
}

impl ClassFaces {
	fn of(m: &Mesh) -> ClassFaces {
		let mut by: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
		for &(key, f) in &m.faces {
			by.entry(key).or_default().extend(f);
		}
		let gi: HashMap<u32, usize> = by.keys().enumerate().map(|(g, &k)| (k, g)).collect();
		let phased = m.strip_phase.len() == m.faces.len();
		let mut rank = vec![0u32; gi.len()];
		let mut clipped = Vec::new();
		for (i, &(key, f)) in m.faces.iter().enumerate() {
			let g = gi[&key];
			if phased && m.strip_phase[i] != crate::mesh::NO_PHASE {
				clipped.push((g, rank[g], f, m.strip_phase[i]));
			}
			rank[g] += 1;
		}
		ClassFaces { groups: by.into_iter().collect(), clipped }
	}
}

/// `buffer` of `bytes` (rounded up to 4, at least 4) for data written later (copyable: `append_props` grows the prop
/// buffers by a GPU copy).
fn sized_buffer(device: &wgpu::Device, label: &str, bytes: usize, usage: wgpu::BufferUsages) -> wgpu::Buffer {
	device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (bytes as u64).max(4).next_multiple_of(4), usage: usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false })
}

/// The mesh of each prop slot (the bolt classes fall back to the bolt mesh, a class without one is empty).
fn slot_meshes<'a>(props: &[u32], prop_meshes: &'a HashMap<u32, Mesh>, empty: &'a Mesh) -> Vec<&'a Mesh> {
	let fallback = prop_meshes.get(&game::BOLT);
	props.iter().map(|&c| prop_meshes.get(&c).or(if (game::BOLT..game::BOLT + 4).contains(&c) { fallback } else { None }).unwrap_or(empty)).collect()
}

fn prop_alphas(view: Option<([f32; 3], [f32; 3])>, props: &[Prop], centres: &[[f32; 3]], radii: &[Option<f32>]) -> Vec<Option<u8>> {
	props
		.iter()
		.enumerate()
		.map(|(pi, p)| {
			let fade = match (view, centres.get(pi)) {
				(Some((e, f)), Some(c)) => {
					let d: [f32; 3] = std::array::from_fn(|k| c[k] - e[k] * 1024.0);
					let z = f[0] * d[0] + f[1] * d[1] + f[2] * d[2];
					let fade = moby_fade(p.draw_dist(), z)?;
					// SCH-6b: an in-game prop on the deferred-list path (flags 0x400 / 0x800) fades by the unclamped distance and the
					// sphere's near edge, in integers.
					match radii.get(pi).copied().flatten() {
						Some(w) if p.sched_flags() & 0xc00 != 0 => moby_fade_alt(p.draw_dist(), z, w),
						_ => fade,
					}
				}
				_ => 0x80,
			};
			Some(moby_draw_alpha(fade, p.moby_alpha()))
		})
		.collect()
}

/// FRAME-1a / BRIDGE-33: the draw verdict of `set_props` without the vertex writes: occlusion mask, near / side planes and
/// the 48-entry deferred list (SCH-6b).
fn prop_verdicts(view: &super::clip::ClipView, mask: &[u8; 128], props: &[Prop], alphas: &[Option<u8>], centres: &[[f32; 3]], radii: &[Option<f32>]) -> Vec<bool> {
	let mut drawn = vec![false; props.len()];
	let mut committed = 0;
	for (pi, p) in props.iter().enumerate() {
		if alphas.get(pi).copied().flatten().is_none() || !p.draw_shown() || !crate::occlusion::occ_pass(mask, p.sched_occ()) {
			continue;
		}
		let side_out = match (centres.get(pi), radii.get(pi).copied().flatten()) {
			(Some(&c), Some(r)) => view.moby_side_near_out(c, r),
			_ => false,
		};
		drawn[pi] = !side_out;
		if p.sched_flags() & 0x800 != 0 && !side_out {
			committed += 1;
			if committed > DEFERRED_CAP {
				drawn[pi] = p.sched_over() == 0;
			}
		}
	}
	drawn
}

fn clip_props(queue: &wgpu::Queue, scene: &mut Scene, src: ClipSrc, shown: &[bool]) {
	let mut taken: Vec<(u32, u32)> = Vec::new();
	if let Some(view) = scene.clip_view {
		let (band, port) = (view.band(), view.viewport());
		// [PC] perf: the slots are scanned and clipped on the frame worker pool, then queued in slot / face order.
		let (ranges, model) = (&scene.props_ranges, &scene.props_model);
		let per_slot: Vec<Vec<(u32, super::clip::ClipPoly)>> = scene
			.prop_faces
			.par_iter()
			.enumerate()
			.map(|(pi, faces)| {
				if faces.is_empty() || !shown.get(pi).copied().unwrap_or(false) {
					return Vec::new();
				}
				// One flag scan per slot replaces a bounding sphere: nothing outside the band, or everything past one viewport plane.
				// (PC perf: one pass, no projected copy.)
				let straddles = |positions: &mut dyn Iterator<Item = [f32; 3]>| {
					let (mut inside, mut all_out) = (true, 0x3fu8);
					for q in positions {
						let p = view.view(q);
						inside &= band.flags(p) == 0;
						all_out &= port.flags(p);
					}
					!inside && all_out == 0
				};
				// `slot` holds the vertices from index `base` on (the faces store batch indices).
				let clip = |slot: &[Vertex], base: usize| faces.iter().enumerate().filter_map(|(fi, f)| super::clip::clip_face(&view, f.idx.map(|i| slot[i as usize - base]), f.phase, f.tex, f.cull).map(|poly| (fi as u32, poly))).collect();
				let r = ranges[pi].clone();
				match src {
					ClipSrc::Cpu(verts) => {
						if !straddles(&mut verts[r].iter().map(|v| v.pos)) {
							return Vec::new();
						}
						clip(verts, 0)
					}
					// [PC] perf round 3: the slot's vertices as `vs_props` poses them (positions only for the scan).
					ClipSrc::Gpu(inst, pose, rad) => {
						let s = &inst[pi];
						// A slot whose bounding sphere settles the scan skips it (same verdict, no per-vertex work).
						if super::gpuskin::slot_sphere(s, rad[pi]).is_some_and(|(c, r)| super::gpuskin::sphere_skips(&view, c, r)) {
							return Vec::new();
						}
						if !straddles(&mut model[r.clone()].iter().enumerate().map(|(j, m0)| super::gpuskin::emulate_pos(s, pose, m0.pos, j))) {
							return Vec::new();
						}
						let slot: Vec<Vertex> = model[r.clone()].iter().enumerate().map(|(j, m0)| super::gpuskin::emulate(s, pose, m0, j)).collect();
						clip(&slot, r.start)
					}
				}
			})
			.collect();
		for (pi, polys) in per_slot.into_iter().enumerate() {
			for (fi, poly) in polys {
				taken.push((pi as u32, fi));
				scene.clip_queue.push(poly);
			}
		}
	}
	if taken != scene.clip_masked {
		let write = |(pi, fi): (u32, u32), idx: Option<[u32; 3]>, scene: &Scene| {
			let f = &scene.prop_faces[pi as usize][fi as usize];
			queue.write_buffer(&scene.props.ibuf, f.slot as u64 * 4, bytemuck::cast_slice(&idx.unwrap_or([0; 3])));
		};
		let (old, new): (HashSet<_>, HashSet<_>) = (scene.clip_masked.iter().copied().collect(), taken.iter().copied().collect());
		for &k in old.difference(&new) {
			write(k, Some(scene.prop_faces[k.0 as usize][k.1 as usize].idx), scene);
		}
		for &k in new.difference(&old) {
			write(k, None, scene);
		}
		scene.clip_masked = taken;
	}
}

/// Uploads the queued clipped polygons into `clip_batch` (growing both buffers to the next power of two when needed) and
/// builds one draw per (cull, texture) run; an empty queue leaves no draws.
fn flush_clip(device: &wgpu::Device, queue: &wgpu::Queue, scene: &mut Scene) {
	let mut verts: Vec<Vertex> = Vec::new();
	let mut by_run: BTreeMap<(u8, usize), Vec<u32>> = BTreeMap::new();
	for poly in scene.clip_queue.drain(..) {
		let base = verts.len();
		verts.extend(poly.verts);
		by_run.entry((poly.cull, poly.tex)).or_default().extend(poly.tris.iter().flatten().map(|&i| (base + i) as u32));
	}
	scene.clip_batch.draws.clear();
	if verts.is_empty() {
		return;
	}
	let (mut indices, mut draws) = (Vec::new(), Vec::new());
	for ((cull, texture), idx) in by_run {
		draws.push(Draw { inst: 0, cull, texture, start: indices.len() as u32, count: idx.len() as u32 });
		indices.extend(idx);
	}
	if verts.len() > scene.clip_cap.0 || indices.len() > scene.clip_cap.1 {
		let cap = (verts.len().next_power_of_two().max(scene.clip_cap.0), indices.len().next_power_of_two().max(scene.clip_cap.1));
		scene.clip_batch.vbuf = buffer(device, "clip vertices", &vec![0u8; cap.0 * std::mem::size_of::<Vertex>()], wgpu::BufferUsages::VERTEX);
		scene.clip_batch.ibuf = buffer(device, "clip indices", &vec![0u8; cap.1 * 4], wgpu::BufferUsages::INDEX);
		scene.clip_cap = cap;
	}
	queue.write_buffer(&scene.clip_batch.vbuf, 0, bytemuck::cast_slice(&verts));
	queue.write_buffer(&scene.clip_batch.ibuf, 0, bytemuck::cast_slice(&indices));
	scene.clip_batch.draws = draws;
}

/// NOV-5.9d path 2: the hero marker (body and held gadgets, world-space `verts`) takes the same per-face clip; masked faces
/// are zeroed in `marker.ibuf` with the compare-and-write rule of `clip_props`.
fn clip_marker(queue: &wgpu::Queue, scene: &mut Scene, verts: &[Vertex]) {
	let mut taken: Vec<u32> = Vec::new();
	if let Some(view) = scene.clip_view {
		let (band, port) = (view.band(), view.viewport());
		let (mut inside, mut all_out) = (true, 0x3fu8);
		for v in verts {
			let p = view.view(v.pos);
			inside &= band.flags(p) == 0;
			all_out &= port.flags(p);
		}
		if !inside && all_out == 0 {
			for (fi, f) in scene.marker_faces.iter().enumerate() {
				if let Some(poly) = super::clip::clip_face(&view, f.idx.map(|i| verts[i as usize]), f.phase, f.tex, f.cull) {
					taken.push(fi as u32);
					scene.clip_queue.push(poly);
				}
			}
		}
	}
	if taken != scene.marker_masked {
		let (old, new): (HashSet<u32>, HashSet<u32>) = (scene.marker_masked.iter().copied().collect(), taken.iter().copied().collect());
		for &k in old.difference(&new) {
			let f = &scene.marker_faces[k as usize];
			queue.write_buffer(&scene.marker.ibuf, f.slot as u64 * 4, bytemuck::cast_slice(&f.idx));
		}
		for &k in new.difference(&old) {
			queue.write_buffer(&scene.marker.ibuf, scene.marker_faces[k as usize].slot as u64 * 4, bytemuck::cast_slice(&[0u32; 3]));
		}
		scene.marker_masked = taken;
	}
}

/// Appends `m`'s vertices (normals from its faces, fog class tag and baked colours as `Gpu::batch` documents) and grows the
/// bounds; `mi` indexes `colours` when the caller supplies per-vertex colours.
/// NOV-35b: the world-to-view rotation rows of the retail camera (0x167080 +0x00/+0x10/+0x20) from the view basis: the rows are
/// (-row 1, -row 2, row 0) of the camera rows (forward, left, up) (BRIDGE-9 tags, camera.md NOV-5.2), i.e. (right, down, forward);
/// row 1 = left is the hero / moby Euler order the camera record shares [I].
fn metal_view_rows(v: &super::clip::ClipView) -> [[f32; 3]; 3] {
	[v.s, v.u.map(|x| -x), v.f]
}

/// The three model axes in world space (columns 0..2 of a column-major placement), retail moby +0xc0 / +0xd0 / +0xe0.
fn axes(m: &[f32; 16]) -> [[f32; 3]; 3] {
	[0, 1, 2].map(|k| [m[4 * k], m[4 * k + 1], m[4 * k + 2]])
}

/// NOV-35b metal vertex colour (bytes / 128; FUN_00210358 metal branch, VU0 prog0 0x0a18-0x0a30, EE ppacb): per channel
/// v = 0.875 + sum_k 0.25 col_k max(to_k . n, 0) (the 0x47800070 bias; no ambient, no wrap term, `n` not renormalised) and
/// byte = trunc(128 v) & 0xff.
fn metal_colour(rows: &[([f32; 3], [f32; 3]); 3], n: [f32; 3]) -> [f32; 3] {
	let l = rows.map(|(d, _)| (d[0] * n[0] + d[1] * n[1] + d[2] * n[2]).max(0.0));
	std::array::from_fn(|ch| {
		let mut acc = 0.0f32;
		for k in 0..3 {
			acc += rows[k].1[ch] * 0.25 * l[k];
		}
		(((0.875 + acc as f64) * 128.0).floor() as i64 & 0xff) as f32 / 128.0
	})
}

/// BRIDGE-45: appends one moby's metal faces. Per vertex: the position (`world`: placement `m` times the `scale`d skinned position;
/// else model space for a panel draw whose uniform carries `m`), the sphere-map UV `metal_uv(e, n')`, the CPU colour from `rows`
/// on the world normal (axes of `m` times n') and the metal alpha (vertex alpha / 128 like the LOD alpha). Faces go by (cull,
/// texture): slot -3 glass (1), other slots < -1 chrome (0) (every metal face on the disc has one, mobies test).
fn push_metal(verts: &mut Vec<Vertex>, runs: &mut BTreeMap<(u8, usize), Vec<u32>>, packets: &[mobies::MetalPacket], posed: Option<(&[[f32; 3]], &[[f32; 3]])>, rows: &[([f32; 3], [f32; 3]); 3], e: [[f32; 3]; 3], m: &[f32; 16], scale: f32, alpha: u8, world: bool) {
	let b = verts.len();
	verts.resize(b + metal_count(packets), Vertex::zeroed());
	fill_metal(&mut verts[b..], b as u32, runs, packets, posed, rows, e, m, scale, alpha, world);
}

/// The vertices `push_metal` appends for `packets`.
fn metal_count(packets: &[mobies::MetalPacket]) -> usize {
	packets.iter().map(|pk| pk.positions.len().min(pk.normals.len())).sum()
}

/// `push_metal` into `dst` (`metal_count` vertices), whose first vertex is batch vertex `first` ([PC] perf round 3: the props'
/// metal batch fills disjoint slices on the frame worker pool).
fn fill_metal(dst: &mut [Vertex], first: u32, runs: &mut BTreeMap<(u8, usize), Vec<u32>>, packets: &[mobies::MetalPacket], posed: Option<(&[[f32; 3]], &[[f32; 3]])>, rows: &[([f32; 3], [f32; 3]); 3], e: [[f32; 3]; 3], m: &[f32; 16], scale: f32, alpha: u8, world: bool) {
	let (mut at, mut out) = (0, 0);
	for pk in packets {
		let base = first + out as u32;
		for (k, (p, n)) in pk.positions.iter().zip(&pk.normals).enumerate() {
			let (p, n) = posed.and_then(|(ps, ns)| Some((*ps.get(at + k)?, *ns.get(at + k)?))).unwrap_or((*p, *n));
			let a = p.map(|c| c * scale);
			let wn: [f32; 3] = std::array::from_fn(|i| m[i] * n[0] + m[4 + i] * n[1] + m[8 + i] * n[2]);
			let pos = if world { std::array::from_fn(|i| m[i] * a[0] + m[4 + i] * a[1] + m[8 + i] * a[2] + m[12 + i]) } else { a };
			let c = metal_colour(rows, wn);
			dst[out] = Vertex { pos, uv: mobies::metal_uv(e, n), normal: wn, colour: [1.0, 1.0, 1.0, moby_alpha_encoding(0.0, alpha).0], light: [c[0], c[1], c[2], METAL_LIGHT] };
			out += 1;
		}
		at += pk.positions.len();
		let cull = (mobies::cull_bits(pk.cull) >> 24) as u8;
		for &(slot, f) in &pk.faces {
			if slot >= -1 {
				continue;
			}
			runs.entry((cull, (slot == -3) as usize)).or_default().extend(f.map(|i| i + base));
		}
	}
}

/// Uploads a metal batch (growing its buffers to the next power of two) with one draw per (cull, texture) run.
fn upload_metal(device: &wgpu::Device, queue: &wgpu::Queue, slot: &mut Option<Batch>, cap: &mut (usize, usize), verts: &[Vertex], runs: BTreeMap<(u8, usize), Vec<u32>>) {
	let (mut indices, mut draws) = (Vec::new(), Vec::new());
	for ((cull, texture), idx) in runs {
		draws.push(Draw { inst: 0, cull, texture, start: indices.len() as u32, count: idx.len() as u32 });
		indices.extend(idx);
	}
	if slot.is_none() || verts.len() > cap.0 || indices.len() > cap.1 {
		*cap = (verts.len().next_power_of_two().max(cap.0).max(64), indices.len().next_power_of_two().max(cap.1).max(192));
		*slot = Some(Batch { vbuf: buffer(device, "metal vertices", &vec![0u8; cap.0 * std::mem::size_of::<Vertex>()], wgpu::BufferUsages::VERTEX), ibuf: buffer(device, "metal indices", &vec![0u8; cap.1 * 4], wgpu::BufferUsages::INDEX), draws: Vec::new() });
	}
	let Some(batch) = slot else { return };
	if !verts.is_empty() {
		queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(verts));
		queue.write_buffer(&batch.ibuf, 0, bytemuck::cast_slice(&indices));
	}
	batch.draws = draws;
}

fn push_mesh_vertices(m: &Mesh, mi: usize, colours: Option<&[Vec<[f32; 4]>]>, verts: &mut Vec<Vertex>, lo: &mut [f32; 3], hi: &mut [f32; 3]) {
	// Moby origin (w 1), or the world fog class tag in w (-1 tfrag, -2 shrub, -3 tie; instance centre in xyz).
	let origin = m.lit.as_ref().map_or([0.0; 4], |l| {
		let c = [l.sphere[0], l.sphere[1], l.sphere[2]];
		match l.kind {
			LitKind::Moby => [c[0], c[1], c[2], 1.0],
			_ if colours.is_some() => [0.0; 4],
			LitKind::Tfrag => [0.0, 0.0, 0.0, -1.0],
			LitKind::Shrub => [l.origin[0], l.origin[1], l.origin[2], -2.0],
			LitKind::Tie => [c[0], c[1], c[2], -3.0],
		}
	});
	// A moby mesh keeps its decoded vertex normals (retail VU0 prog0 lights with them, lighting_fog.md REN-5c); other
	// meshes sum their face normals.
	let normals = m.vertex_normals();
	for (i, p) in m.positions.iter().enumerate() {
		for j in 0..3 {
			lo[j] = lo[j].min(p[j]);
			hi[j] = hi[j].max(p[j]);
		}
		// Baked world colours (PS2 bytes, 0x80 = 1.0) are flagged with alpha -2 for the shader.
		let colour = match (colours, m.colours.get(i)) {
			(Some(c), _) => c[mi][i],
			(None, Some(b)) => [b[0] / 128.0, b[1] / 128.0, b[2] / 128.0, -2.0],
			(None, None) => [1.0, 1.0, 1.0, -1.0],
		};
		verts.push(Vertex { pos: *p, uv: m.uvs[i], normal: normals[i], colour, light: origin });
	}
}

/// Index and draw layout of `Gpu::batch_instances`: each mesh's faces grouped by texture key (ascending), meshes in order.
/// Returns the index buffer, the draws as (key, first index, index count), and per mesh its range in the draw list and its
/// vertex range.
fn instance_layout(meshes: &[&Mesh]) -> (Vec<u32>, Vec<(u32, u32, u32)>, Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>) {
	let (mut indices, mut draws, mut ranges) = (Vec::new(), Vec::new(), Vec::new());
	let mut base = 0u32;
	for m in meshes {
		// Ordered by key without the shrub material bits (then the full key): keys without them keep their order.
		let mut by_tex: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new();
		for &(key, f) in &m.faces {
			let bare = if crate::shrubs::key_k(key).is_some() { key & !crate::shrubs::MATERIAL_BITS } else { key & !(3 << crate::shrubs::WRAP_SHIFT) };
			by_tex.entry((bare, key)).or_default().extend(f.map(|i| i + base));
		}
		let first = draws.len();
		for ((_, key), idx) in by_tex {
			draws.push((key, indices.len() as u32, idx.len() as u32));
			indices.extend(idx);
		}
		let n = m.positions.len();
		ranges.push((first..draws.len(), base as usize..base as usize + n));
		base += n as u32;
	}
	(indices, draws, ranges)
}

/// The alpha carrier of a faded shrub mesh or a billboard in the vertex colour alpha: -2 - a / 256 (a 1..=0x80), decoded by the
/// shrub fragment entries; every value stays below the -1.5 baked-colour tag.
fn shrub_carrier(a: u8) -> f32 {
	-2.0 - a as f32 / 256.0
}

/// Mip state of the terrain textures of a tfrag batch: the authored chains by texture index and the material records of
/// each mesh of the batch (by mesh index), which give the draw its K.
pub(super) struct TerrainRule<'a> {
	chains: &'a [Option<Vec<Texture>>],
	materials: Vec<&'a [terrain::TfragMaterial]>,
}

/// A tfrag mesh copy whose face keys carry the material record index in bits 8..16 next to the texture index.
fn terrain_face_keys(m: &Mesh, mats: &[u8]) -> Mesh {
	let mut m = m.clone();
	for (f, &mat) in m.faces.iter_mut().zip(mats) {
		f.0 |= (mat as u32) << 8;
	}
	m
}

/// The first-instance word of a tfrag draw: the raw 12-bit K (signed, 1/16 mip) and the top mip level in bits 12..14.
fn terrain_inst(k: f32, mxl: usize) -> u32 {
	((mxl.min(3) as u32) << 12) | ((k * 16.0) as i32 as u32 & 0xfff)
}

/// Which vertex buffer a `LitWorld` vertex address falls in, and the offset inside it: world below `tie_first`, tie LOD vertices
/// from there, shrubs from `shrub_first` on (>=, the boundaries belong to the later buffer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VertexSpace {
	World,
	Tie,
	Shrub,
}

/// [PC] perf: the vertex count of one work item of the frame worker pool (`par_chunked`, the marker build).
const PAR_CHUNK: usize = 4096;

/// [PC] perf round 3: per tie vertex the index of its slot triple in its (instance, LOD) list of distinct fat-vertex triples
/// (u32::MAX for a regular vertex), and those lists: `cull_ties` blends each triple once per rewrite.
fn tie_triples(slots: &[ties::TieSlot], vranges: &[[std::ops::Range<usize>; 3]]) -> (Vec<u32>, Vec<[Vec<[usize; 3]>; 3]>) {
	let mut tri = vec![u32::MAX; slots.len()];
	let lists = vranges
		.iter()
		.map(|lods| {
			lods.clone().map(|r| {
				let (mut list, mut at): (Vec<[usize; 3]>, HashMap<[usize; 3], u32>) = (Vec::new(), HashMap::new());
				for k in r {
					if slots[k].1 {
						tri[k] = *at.entry(slots[k].0).or_insert_with(|| {
							list.push(slots[k].0);
							list.len() as u32 - 1
						});
					}
				}
				list
			})
		})
		.collect();
	(tri, lists)
}

/// [PC] perf: `par_ranges` with every range cut into pieces of at most `PAR_CHUNK` vertices: `f(job, offset in the job's
/// range, piece)`. For per-vertex work that does not depend on the other vertices of its range.
fn par_chunked(verts: &mut [Vertex], ranges: &[std::ops::Range<usize>], f: impl Fn(usize, usize, &mut [Vertex]) + Sync) {
	if ranges.windows(2).any(|w| w[1].start < w[0].end) {
		for (k, r) in ranges.iter().enumerate() {
			f(k, 0, &mut verts[r.clone()]);
		}
		return;
	}
	let mut pieces: Vec<(usize, usize, &mut [Vertex])> = Vec::with_capacity(ranges.len());
	let (mut rest, mut at) = (verts, 0);
	for (k, r) in ranges.iter().enumerate() {
		let (_, tail) = std::mem::take(&mut rest).split_at_mut(r.start - at);
		let (mut mid, tail) = tail.split_at_mut(r.len());
		let mut off = 0;
		while mid.len() > PAR_CHUNK {
			let (a, b) = std::mem::take(&mut mid).split_at_mut(PAR_CHUNK);
			pieces.push((k, off, a));
			(mid, off) = (b, off + PAR_CHUNK);
		}
		pieces.push((k, off, mid));
		(rest, at) = (tail, r.end);
	}
	pieces.into_par_iter().for_each(|(k, off, v)| f(k, off, v));
}

/// [PC] perf: `f(job, slice)` for every range of `verts` on the frame worker pool (`init_pool`), results in job order. Each
/// job owns its slice, so the vertices are the same as one thread's; ranges that overlap or are out of ascending order run
/// on this thread in job order.
fn par_ranges<R: Send>(verts: &mut [Vertex], ranges: &[std::ops::Range<usize>], f: impl Fn(usize, &mut [Vertex]) -> R + Sync) -> Vec<R> {
	if ranges.windows(2).any(|w| w[1].start < w[0].end) {
		return ranges.iter().enumerate().map(|(k, r)| f(k, &mut verts[r.clone()])).collect();
	}
	let mut slices: Vec<&mut [Vertex]> = Vec::with_capacity(ranges.len());
	let (mut rest, mut at) = (verts, 0);
	for r in ranges {
		let (_, tail) = std::mem::take(&mut rest).split_at_mut(r.start - at);
		let (mid, tail) = tail.split_at_mut(r.len());
		slices.push(mid);
		(rest, at) = (tail, r.end);
	}
	slices.into_par_iter().enumerate().map(|(k, v)| f(k, v)).collect()
}

/// [PC] (user request: high frame rates) the swapchain present mode: uncapped by default, Mailbox if the surface has it,
/// else Immediate, else Fifo; `vsync` (RAC_VSYNC=1) forces Fifo. `named` (RAC_PRESENT=immediate / mailbox / fifo) picks one
/// when the surface has it (Immediate tears but costs the least CPU per present). Only the render rate changes: the game
/// still steps its fixed 50 Hz ticks from the frame-time accumulator (handler.rs RedrawRequested).
/// [PC] (Re)creates the swapchain. An uncapped mode is configured over a Fifo swapchain: on Vulkan / NVIDIA (RTX 5070 Ti)
/// an Immediate swapchain created that way costs ~0.35 ms less CPU in submit + present per frame and has fewer present
/// stalls (bench medians: present 1.75 -> 1.42 ms, p99 18.8 -> 15.0 ms) than one created directly; harmless elsewhere.
fn configure_surface(surface: &wgpu::Surface, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) {
	if config.present_mode != wgpu::PresentMode::Fifo {
		surface.configure(device, &wgpu::SurfaceConfiguration { present_mode: wgpu::PresentMode::Fifo, ..config.clone() });
	}
	surface.configure(device, config);
}

fn present_mode(supported: &[wgpu::PresentMode], vsync: bool, named: Option<&str>) -> wgpu::PresentMode {
	use wgpu::PresentMode::{Fifo, Immediate, Mailbox};
	if vsync {
		return Fifo;
	}
	let named = match named {
		Some("immediate") => Some(Immediate),
		Some("mailbox") => Some(Mailbox),
		Some("fifo") => Some(Fifo),
		_ => None,
	};
	named.into_iter().chain([Mailbox, Immediate]).find(|m| supported.contains(m)).unwrap_or(Fifo)
}

fn relight_space(first: usize, tie_first: usize, shrub_first: usize) -> (VertexSpace, usize) {
	if first >= shrub_first {
		(VertexSpace::Shrub, first - shrub_first)
	} else if first >= tie_first {
		(VertexSpace::Tie, first - tie_first)
	} else {
		(VertexSpace::World, first)
	}
}

/// The vertex colour alpha to write when a shrub's fade state changes from `cached` to `a` (0x80 = opaque), None when unchanged.
fn shrub_rewrite(cached: u8, a: u8) -> Option<f32> {
	(a != cached).then(|| shrub_carrier(a))
}

/// Instance indices in retail's walk order: classes by load slot, records in gameplay order inside a class (stable sort).
fn shrub_order(set: &[shrubs::ShrubInstance]) -> Vec<usize> {
	let mut order: Vec<usize> = (0..set.len()).collect();
	order.sort_by_key(|&i| set[i].class_slot);
	order
}

/// NOV-22.12 (BRIDGE-44): the mode-1 flat pool sprite corners (VU1 prog2 0x218-0x240) in quad-cycle order P0, P1,
/// P3, P2 = pos + (a, b), (-b, a), (-a, -b), (b, -a), all at pos.z; half-diagonal R = raw size / 420000 =
/// `size * sqrt2 / 4` for the billboard size convention, a = R cos(rot), b = R sin(rot).
fn flat_corners(pos: [f32; 3], size: f32, rot: f32) -> [[f32; 3]; 4] {
	let r = size * (std::f32::consts::SQRT_2 / 4.0);
	let (a, b) = (r * rot.cos(), r * rot.sin());
	[[pos[0] + a, pos[1] + b, pos[2]], [pos[0] - b, pos[1] + a, pos[2]], [pos[0] - a, pos[1] - b, pos[2]], [pos[0] + b, pos[1] - a, pos[2]]]
}

#[cfg(test)]
mod line_tests {
	use super::*;

	/// KER-92 test (5): near 0.5 / far 224 strict on the midpoint depth key; the pool clamp lowers far; both ends clip.
	#[test]
	fn tracer_line_depth_key_and_clip() {
		let cam = Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: 0.5, up: [0.0, 0.0, 1.0] };
		let f = [1.0, 0.0, 0.0];
		let m = view_proj(&cam, 16.0 / 9.0, RETAIL_NEAR, RETAIL_FAR);
		let line = |x: f32| particles::Line { a: [x + 1.0, 0.0, 0.0], b: [x - 1.0, 0.0, 0.0], col_a: [1.0; 4], col_b: [1.0; 4], near: 0.5, far: 224.0 };
		let shown = |l: &particles::Line, clamp: f32| line_shown(&m, cam.pos, f, l, clamp);
		assert!(shown(&line(10.0), 500.0));
		assert!(!shown(&line(0.4), 500.0));
		assert!(!shown(&line(230.0), 500.0));
		assert!(!shown(&line(224.0), 500.0));
		assert!(shown(&line(200.0), 500.0));
		assert!(!shown(&line(70.0), 64.0));
		// One end outside the frustum (far to the side): skipped.
		let side = particles::Line { a: [10.0, 0.0, 0.0], b: [10.0, 50.0, 0.0], ..line(10.0) };
		assert!(!shown(&side, 500.0));
	}
}

#[cfg(test)]
mod instance_tests {
	use super::*;

	/// NOV-22.12 tests 1-2: flat corners for raw sizes 420000 and 210000.
	#[test]
	fn flat_pool_sprite_corners() {
		let scale = std::f32::consts::SQRT_2 / 210_000.0;
		let near = |a: [[f32; 3]; 4], b: [[f32; 3]; 4], e: f32| a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| (x - y).abs() <= e);
		let c = flat_corners([0.0; 3], 420_000.0 * scale, 0.0);
		let o = 0.99999994;
		assert!(near(c, [[o, 0.0, 0.0], [0.0, o, 0.0], [-o, 0.0, 0.0], [0.0, -o, 0.0]], 1e-6), "{c:?}");
		let c = flat_corners([10.0, 20.0, 5.0], 210_000.0 * scale, 32.0 * std::f32::consts::TAU / 256.0);
		let want = [[10.353554, 20.353554, 5.0], [9.646447, 20.353554, 5.0], [9.646446, 19.646446, 5.0], [10.353554, 19.646446, 5.0]];
		assert!(near(c, want, 2e-6), "{c:?}");
	}

	fn mesh(n: usize, faces: &[(u32, [u32; 3])]) -> Mesh {
		let mut m = Mesh::new(String::new());
		m.positions = vec![[0.0; 3]; n];
		m.uvs = vec![[0.0; 2]; n];
		m.faces = faces.to_vec();
		m
	}

	/// BRIDGE-27 test 3 / BRIDGE-29: one draw per (mesh, texture), ascending key within a mesh, per-mesh ranges.
	#[test]
	fn instance_layout_keeps_per_mesh_ranges() {
		let a = mesh(4, &[(5, [0, 1, 2]), (3, [1, 2, 3]), (5, [2, 3, 0])]);
		let b = mesh(3, &[(5, [0, 1, 2])]);
		let (idx, draws, ranges) = instance_layout(&[&a, &b]);
		assert_eq!(draws, vec![(3, 0, 3), (5, 3, 6), (5, 9, 3)]);
		assert_eq!(ranges, vec![(0..2, 0..4), (2..3, 4..7)]);
		assert_eq!(&idx[9..], &[4, 5, 6], "the second mesh's indices are offset by the first's vertex count");
	}

	/// An empty mesh (a LOD with no packets) has an empty draw range and advances nothing.
	#[test]
	fn instance_layout_handles_empty_meshes() {
		let a = mesh(0, &[]);
		let b = mesh(3, &[(1, [0, 1, 2])]);
		let (_, draws, ranges) = instance_layout(&[&a, &b]);
		assert_eq!(draws.len(), 1);
		assert_eq!(ranges, vec![(0..0, 0..0), (0..1, 0..3)]);
	}
	/// The shrub fragment alpha rule as the shader computes it: As = (texel alpha x A) >> 7 with the texel in PS2 units
	/// (`floor(c.a * 128 + 0.5)`), A decoded from the carrier. Returns As and which Z split of `lim` it falls on.
	fn shrub_alpha(texel_ps2: u32, carrier: f32) -> (u32, bool) {
		let a = ((-2.0 - carrier) * 256.0).round() as u32;
		(texel_ps2 * a / 128, true)
	}

	/// BRIDGE-27 test (1): the alpha carrier round-trips.
	#[test]
	fn shrub_carrier_values() {
		for (a, w) in [(74u8, -2.2890625f32), (127, -2.4960938), (0x80, -2.5), (0, -2.0)] {
			assert_eq!(shrub_carrier(a), w);
			assert_eq!(((-2.0 - w) * 256.0).round() as u8, a);
		}
	}

	/// Test (2): fragment alpha and Z split (the shader's As rule, entries ge12 / lt12 and ge96 / lt96).
	#[test]
	fn shrub_fragment_alpha_split() {
		let (as_, _) = shrub_alpha(0x80, shrub_carrier(74));
		assert_eq!(as_, 74);
		assert!(as_ >= 12);
		assert_eq!(shrub_alpha(0x80, shrub_carrier(5)).0, 5);
		assert!(shrub_alpha(0x80, shrub_carrier(5)).0 < 12);
		assert_eq!(shrub_alpha(0x14, shrub_carrier(74)).0, 11);
		assert_eq!(shrub_alpha(96, shrub_carrier(0x80)).0, 96);
		assert!(shrub_alpha(95, shrub_carrier(0x80)).0 < 96);
		// The shader text carries the same constants.
		assert!(super::super::SHADER.contains("floor(ta * a / 128.0)"));
		assert!(super::super::SHADER.contains("r.a * 128.0 < 12.0") && super::super::SHADER.contains("r.a * 128.0 < 96.0"));
	}

	#[test]
	fn present_mode_prefers_uncapped() {
		use wgpu::PresentMode::{Fifo, FifoRelaxed, Immediate, Mailbox};
		assert_eq!(present_mode(&[Fifo, Immediate, Mailbox], false, None), Mailbox);
		assert_eq!(present_mode(&[Fifo, FifoRelaxed, Immediate], false, None), Immediate);
		assert_eq!(present_mode(&[Fifo], false, None), Fifo);
		assert_eq!(present_mode(&[Fifo, Immediate, Mailbox], true, None), Fifo);
		assert_eq!(present_mode(&[Fifo, Immediate, Mailbox], false, Some("immediate")), Immediate);
		assert_eq!(present_mode(&[Fifo, Immediate], false, Some("mailbox")), Immediate);
		assert_eq!(present_mode(&[Fifo, Immediate, Mailbox], true, Some("immediate")), Fifo);
	}

	/// Test (4): which buffer a relit vertex address lands in.
	#[test]
	fn relight_addresses_split_at_the_shrub_boundary() {
		assert_eq!(relight_space(1012, 800, 1000), (VertexSpace::Shrub, 12));
		assert_eq!(12 * std::mem::size_of::<Vertex>(), 768);
		assert_eq!(relight_space(1000, 800, 1000), (VertexSpace::Shrub, 0));
		assert_eq!(relight_space(40, 800, 1000), (VertexSpace::World, 40));
		assert_eq!(relight_space(900, 800, 1000), (VertexSpace::Tie, 100));
	}

	/// Test (6): a shrub rewrites its vertex colours only when its fade state changes.
	#[test]
	fn shrub_fade_cache_rewrites_on_change_only() {
		assert_eq!(shrub_rewrite(0x80, 74), Some(-2.2890625));
		assert_eq!(shrub_rewrite(74, 74), None);
		assert_eq!(shrub_rewrite(74, 0x80), Some(-2.5));
	}

	/// Test (7): the draw order follows the class load slot, stable within a slot.
	#[test]
	fn shrub_order_sorts_by_class_slot() {
		let inst = |slot| shrubs::ShrubInstance { mesh: Mesh::new(String::new()), cull: shrubs::ShrubCull { centre: [0.0; 3], radius: 0.0, dist: 0.0, bb: None }, bb: None, class_slot: slot };
		assert_eq!(shrub_order(&[inst(2), inst(0), inst(1)]), vec![1, 2, 0]);
		assert_eq!(shrub_order(&[inst(1), inst(1), inst(0), inst(1)]), vec![2, 0, 1, 3]);
	}
	/// The shader's level choice (`shrub_bb_texel`): lod = log2(32 vz) + K, level 0 when lod <= 0, else the nearest level capped at `mxl`.
	fn mip_level(vz: f32, k: f32, mxl: u32) -> u32 {
		let lod = (32.0 * vz).log2() + k;
		if lod > 0.0 {
			((lod + 0.5).floor() as u32).min(mxl)
		} else {
			0
		}
	}

	/// REN-CULL-SHRUB-BB-MIP tests (1)-(5) (also the tfrag pass of REN-TEX-RESIDENCY: same Q = 1 / (32 vz)).
	#[test]
	fn mip_level_switches_at_fixed_depths() {
		assert!(super::super::SHADER.contains("log2(32.0 / i.pos.w) + i.kmx.x") && super::super::SHADER.contains("floor(lod + 0.5), i.kmx.y"));
		let k = -10.625;
		assert_eq!(mip_level(69.7, k, 3), 0);
		assert_eq!(mip_level(69.8, k, 3), 1);
		assert_eq!(mip_level(139.5, k, 3), 1);
		assert_eq!(mip_level(139.7, k, 3), 2);
		assert_eq!(mip_level(279.3, k, 3), 3);
		assert_eq!(mip_level(279.3, k, 2), 2);
		assert_eq!(mip_level(100.0, k, 3), 1);
		assert_eq!(mip_level(100.0, -11.625, 3), 0);
		assert_eq!(mip_level(200.0, -11.625, 3), 1);
		assert_eq!(mip_level(400.0, -12.25, 3), 1);
		assert_eq!(mip_level(450.0, -12.25, 3), 2);
		assert_eq!(mip_level(50.0, -10.625, 3), 0);
		assert_eq!(mip_level(50.0, -12.25, 3), 0);
		// Within the 500 m cap K = -11.625 never reaches level 3 (558.34 m).
		assert_eq!(mip_level(500.0, -11.625, 3), 2);
	}

	/// REN-TEX-RESIDENCY: the tfrag draw's first-instance word round-trips K and the top level through the vertex shader decode.
	#[test]
	fn tfrag_instance_word() {
		let w = terrain_inst(-7.8125, 3);
		assert_eq!(w, 0x3f83);
		let mut k = (w & 0xfff) as i32;
		if k >= 2048 {
			k -= 4096;
		}
		assert_eq!((k as f32 / 16.0, (w >> 12) & 3), (-7.8125, 3));
		assert_eq!(terrain_inst(-8.5625, 2), 0x2f77);
		assert_eq!(terrain_inst(3.0, 0), 48);
		assert!(super::super::SHADER.contains("f32((inst >> 12u) & 3u)"));
	}

	/// A terrain mesh copy keys each face with its material index, leaving the texture index in the low byte.
	#[test]
	fn terrain_face_keys_carry_the_material() {
		let mut m = Mesh::new(String::new());
		m.faces = vec![(13, [0, 1, 2]), (13, [1, 2, 3]), (7, [2, 3, 4])];
		let k = terrain_face_keys(&m, &[0, 5, 2]);
		assert_eq!(k.faces.iter().map(|f| f.0).collect::<Vec<_>>(), vec![13, 13 | 5 << 8, 7 | 2 << 8]);
	}
}

#[cfg(test)]
mod verdict_tests {
	use super::*;

}

/// [PC] The device-side handles the scene build needs (wgpu handles are cheap clones and Send + Sync), so `build_scene` can run on a
/// background thread (the level prefetch) while the viewer keeps drawing the level that is still playing.
pub(super) struct SceneCtx {
	pub(super) device: wgpu::Device,
	pub(super) queue: wgpu::Queue,
	pub(super) sampler: wgpu::Sampler,
	pub(super) wrap_samplers: [wgpu::Sampler; 3],
	pub(super) tex_layout: wgpu::BindGroupLayout,
	/// While `build_scene` runs: the texture view of every bind group `upload` / `upload_chain` made with the scene sampler (bindless.rs).
	pub(super) tex_log: std::cell::RefCell<Option<HashMap<wgpu::BindGroup, wgpu::TextureView>>>,
	/// The bindless array layout (None: per-draw bind groups).
	pub(super) bindless_layout: Option<wgpu::BindGroupLayout>,
	/// The props are posed in the vertex shader (`Gpu::props_gpu` is Some).
	pub(super) gpu_posing: bool,
	/// Set on the level-prefetch thread: big buffer writes go in 4 MB pieces (see `wb`).
	pub(super) paced: bool,
}

impl SceneCtx {
	/// `queue.write_buffer`, but on the level-prefetch thread (`paced`) in 4 MB pieces with a 1 ms pause after each:
	/// wgpu copies a write into its staging memory while holding the queue's pending-writes lock, so one 100 MB
	/// write would stall the main thread's acquire/submit for 100+ ms. Same bytes, same buffer, same order.
	fn wb(&self, buf: &wgpu::Buffer, off: u64, data: &[u8]) {
		const CHUNK: usize = 4 << 20;
		if !self.paced || data.len() <= CHUNK {
			self.queue.write_buffer(buf, off, data);
			return;
		}
		for (k, c) in data.chunks(CHUNK).enumerate() {
			self.queue.write_buffer(buf, off + (k * CHUNK) as u64, c);
			std::thread::sleep(std::time::Duration::from_millis(1));
		}
	}

	pub(super) fn upload_texture(&self, t: Option<&Texture>) -> wgpu::BindGroup {
		self.upload(t, &self.sampler)
	}

	pub(super) fn upload(&self, t: Option<&Texture>, sampler: &wgpu::Sampler) -> wgpu::BindGroup {
		let fallback = Texture { w: 1, h: 1, rgba: vec![[200, 0, 200, 255]] };
		let t = t.unwrap_or(&fallback);
		let size = wgpu::Extent3d { width: t.w as u32, height: t.h as u32, depth_or_array_layers: 1 };
		// Mipmaps for the scene sampler only (HUD art is drawn near 1:1 with `nearest`).
		let mips = if *sampler == self.sampler { mip_chain(t) } else { Vec::new() };
		let tex = self.device.create_texture(&wgpu::TextureDescriptor {
			label: None,
			size,
			mip_level_count: 1 + mips.len() as u32,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba8Unorm,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		self.queue.write_texture(
			tex.as_image_copy(),
			bytemuck::cast_slice(&t.rgba),
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(t.w as u32 * 4), rows_per_image: Some(t.h as u32) },
			size,
		);
		for (i, (w, h, px)) in mips.iter().enumerate() {
			self.queue.write_texture(
				wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: i as u32 + 1, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
				bytemuck::cast_slice(px),
				wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(*h) },
				wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
			);
		}
		let view = tex.create_view(&Default::default());
		let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: None,
			layout: &self.tex_layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
			],
		});
		// bindless.rs: `load_scene` collects the views of the scene's slots (any sampler: the array shader picks the shrub wrap
		// sampler itself; a missing view turned the array path off on levels with clamped shrub materials).
		if let Some(log) = self.tex_log.borrow_mut().as_mut() {
			log.insert(bind.clone(), view);
		}
		bind
	}

	/// Faces grouped by texture (sorted by key), or in mesh order when `ordered` (sky shells blend in
	/// sequence). `colours` gives per-mesh vertex colours; otherwise white.
	pub(super) fn batch(&self, meshes: &[&Mesh], colours: Option<&[Vec<[f32; 4]>]>, ordered: bool, textures: &HashMap<u32, Texture>, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>) -> (Batch, [f32; 3], [f32; 3], Vec<Vertex>) {
		self.batch_spare(meshes, colours, ordered, textures, bind, slots, (0, 0))
	}

	/// `batch` with buffers `spare` (vertices, indices) larger than the data: the prop slots append grown retail pools
	/// there (`append_props`).
	fn batch_spare(&self, meshes: &[&Mesh], colours: Option<&[Vec<[f32; 4]>]>, ordered: bool, textures: &HashMap<u32, Texture>, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>, spare: (usize, usize)) -> (Batch, [f32; 3], [f32; 3], Vec<Vertex>) {
		let mut verts: Vec<Vertex> = Vec::new();
		let mut runs: Vec<(u32, Vec<u32>)> = Vec::new();
		let mut by_tex: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
		let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
		// ARI-4: an ordered run never spans two meshes (the sky sprites draw between shells 1 and 2).
		let mut run_mesh = usize::MAX;
		for (mi, m) in meshes.iter().enumerate() {
			let base = verts.len() as u32;
			push_mesh_vertices(m, mi, colours, &mut verts, &mut lo, &mut hi);
			for &(tex, f) in &m.faces {
				let f = f.map(|i| i + base);
				if !ordered {
					by_tex.entry(tex).or_default().extend(f);
				} else if let Some(r) = runs.last_mut().filter(|r| r.0 == tex && run_mesh == mi) {
					r.1.extend(f);
				} else {
					runs.push((tex, f.to_vec()));
					run_mesh = mi;
				}
			}
		}
		if spare.0 > 0 { crate::loadlog::say(|| "bs: verts+groups".into()); }
		if !ordered {
			runs = by_tex.into_iter().collect();
		}
		let mut indices = Vec::new();
		let mut draws = Vec::new();
		for (tex, idx) in runs {
			let cull = ((tex >> 24) & 3) as u8;
			let tex = tex & crate::mobies::KEY_MASK;
			let texture = match slots.get(&tex) {
				Some(&i) => i,
				None => {
					bind.push(self.upload_texture(textures.get(&tex)));
					slots.insert(tex, bind.len() - 1);
					bind.len() - 1
				}
			};
			draws.push(Draw { inst: 0, cull, texture, start: indices.len() as u32, count: idx.len() as u32 });
			indices.extend(idx);
		}
		if spare.0 > 0 { crate::loadlog::say(|| "bs: draws+indices".into()); }
		let vbuf = sized_buffer(&self.device, "vertices", (verts.len() + spare.0) * std::mem::size_of::<Vertex>(), wgpu::BufferUsages::VERTEX);
		self.wb(&vbuf, 0, bytemuck::cast_slice(&verts));
		if spare.0 > 0 { crate::loadlog::say(|| format!("bs: vbuf written ({} verts)", verts.len())); }
		let ibuf = sized_buffer(&self.device, "indices", (indices.len() + spare.1) * 4, wgpu::BufferUsages::INDEX);
		self.wb(&ibuf, 0, bytemuck::cast_slice(&indices));
		(Batch { vbuf, ibuf, draws }, lo, hi, verts)
	}

	/// A texture with its own mip chain (level 0 first, each level half the size of the previous): no resampling, the levels are
	/// exactly the ones given (REN-CULL-SHRUB-BB-MIP, the GS reads authored levels).
	pub(super) fn upload_chain(&self, chain: &[Texture], sampler: &wgpu::Sampler) -> wgpu::BindGroup {
		let t = &chain[0];
		let tex = self.device.create_texture(&wgpu::TextureDescriptor {
			label: None,
			size: wgpu::Extent3d { width: t.w as u32, height: t.h as u32, depth_or_array_layers: 1 },
			mip_level_count: chain.len() as u32,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba8Unorm,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		for (k, m) in chain.iter().enumerate() {
			self.queue.write_texture(
				wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: k as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
				bytemuck::cast_slice(&m.rgba),
				wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(m.w as u32 * 4), rows_per_image: Some(m.h as u32) },
				wgpu::Extent3d { width: m.w as u32, height: m.h as u32, depth_or_array_layers: 1 },
			);
		}
		let view = tex.create_view(&Default::default());
		let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: None,
			layout: &self.tex_layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
			],
		});
		// bindless.rs: `load_scene` collects the views of the scene's slots (any sampler: the array shader picks the shrub wrap
		// sampler itself; a missing view turned the array path off on levels with clamped shrub materials).
		if let Some(log) = self.tex_log.borrow_mut().as_mut() {
			log.insert(bind.clone(), view);
		}
		bind
	}

	/// The instance-aware builder (BRIDGE-29, shared with the shrub batch of BRIDGE-27): like `batch` over unordered world
	/// meshes, but every mesh keeps its own draws, one per (mesh, texture) in ascending texture key, in mesh order. Returns
	/// the batch (its `draws` are that static list), the bounds, the vertices as uploaded, and per mesh its range in
	/// `draws` and its vertex range.
	pub(super) fn batch_instances(&self, meshes: &[&Mesh], textures: &HashMap<u32, Texture>, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>, terrain: Option<&TerrainRule>, shrub_chains: &[Option<Vec<Texture>>]) -> (Batch, [f32; 3], [f32; 3], Vec<Vertex>, Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>) {
		let mut verts: Vec<Vertex> = Vec::new();
		let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
		for (mi, m) in meshes.iter().enumerate() {
			push_mesh_vertices(m, mi, None, &mut verts, &mut lo, &mut hi);
		}
		let (indices, runs, ranges) = instance_layout(meshes);
		let mut draws = Vec::new();
		for (mi, (dr, _)) in ranges.iter().enumerate() {
			for &(key, start, count) in &runs[dr.clone()] {
				let cull = ((key >> 24) & 3) as u8;
				let mut wrap = (key >> crate::shrubs::WRAP_SHIFT) & 3;
				let shrub_k = crate::shrubs::key_k(key);
				let key = if shrub_k.is_some() { key & crate::mobies::KEY_MASK & !crate::shrubs::MATERIAL_BITS } else { key & crate::mobies::KEY_MASK };
				// A terrain face key is its texture index, with the material record index in bits 8..16 (`terrain_face_keys`).
				let (key, inst, chain) = match terrain {
					Some(t) => {
						let (tex, mat) = (key & 0xff, (key >> 8) & 0xff);
						let chain = t.chains.get(tex as usize).and_then(|c| c.as_ref());
						let material = t.materials[mi].get(mat as usize);
						let inst = material.map_or(0, |m| terrain_inst(m.k, chain.map_or(0, |c| c.len() - 1)));
						// The record's GS CLAMP_1 (REPEAT or CLAMP per axis): a REPEAT v at a fading cliff edge drew the top row of the
						// texture as a thin line there (Eudora cliff undersides, fx_census_late.md run 21).
						wrap = material.map_or(0, |m| m.wrap);
						(tex, inst, chain)
					}
					// A shrub mesh draw's first instance is its material word (`vs_shrub`): CLAMP wrap, TEX1 K and the top level of
					// its texture's authored chain (MXL = mips - 1); its slot has the wrap's sampler.
					None => match shrub_k {
						Some(k) => {
							let chain = shrub_chains.get((key & 0xff) as usize).and_then(|c| c.as_ref());
							// A tie (key group 2) without an authored chain keeps the plain sample (word 0, `tie_kmx`).
							let word = if chain.is_none() && (key >> 16) == 2 { 0 } else { wrap | k << 2 | (chain.map_or(0, |c| c.len() as u32 - 1)) << 14 };
							(key, word, chain)
						}
						None => (key, wrap, None),
					},
				};
				let slot_key = key | wrap << crate::shrubs::WRAP_SHIFT;
				let texture = match slots.get(&slot_key) {
					Some(&i) => i,
					None => {
						bind.push(match chain {
							Some(c) if wrap != 0 => self.upload_chain(c, &self.wrap_samplers[wrap as usize - 1]),
							Some(c) => self.upload_chain(c, &self.sampler),
							None if wrap != 0 => self.upload(textures.get(&key), &self.wrap_samplers[wrap as usize - 1]),
							None => self.upload_texture(textures.get(&key)),
						});
						slots.insert(slot_key, bind.len() - 1);
						bind.len() - 1
					}
				};
				draws.push(Draw { inst, cull, texture, start, count });
			}
		}
		let vbuf = buffer(&self.device, "instance vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
		self.wb(&vbuf, 0, bytemuck::cast_slice(&verts));
		let ibuf = buffer(&self.device, "instance indices", bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX);
		self.wb(&ibuf, 0, bytemuck::cast_slice(&indices));
		(Batch { vbuf, ibuf, draws }, lo, hi, verts, ranges)
	}

	/// The gameplay prop slots of a scene: one copy of the class mesh per slot in `props` order (`load_scene`, and
	/// `regrow_props` when a retail pool grows).
	fn prop_slots(&self, props: &[u32], prop_meshes: &HashMap<u32, Mesh>, textures: &HashMap<u32, Texture>, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>) -> PropSlots {
		let empty = Mesh::new(String::new());
		let slot_meshes = slot_meshes(props, prop_meshes, &empty);
		let mut ranges = Vec::new();
		let mut at = 0;
		for m in &slot_meshes {
			ranges.push(at..at + m.positions.len());
			at += m.positions.len();
		}
		let prompt_slot = props.iter().position(|&c| c == game::vendor::PROMPT_CLASS);
		// [PC] Room for grown retail pools (`append_props`): a 16th more vertices and indices, at least 64K / 192K.
		let spare = (at / 16).max(1 << 16) * (crate::retail::active() as usize);
		let n_idx: usize = slot_meshes.iter().map(|m| m.faces.len() * 3).sum();
		let (batch, _, _, model) = self.batch_spare(&slot_meshes, None, false, textures, bind, slots, (spare, (n_idx / 16).max(3 << 16) * (spare > 0) as usize));
		crate::loadlog::say(|| "ps: batch_spare done".into());
		let slot_runs = super::shadowvol::slot_runs(&slot_meshes);
		crate::loadlog::say(|| "ps: slot_runs done".into());
		// NOV-5.9c: where each clippable face sits in `props.ibuf`, and the overlay batch its clipped polygons go to.
		let faces = super::clip::replay_faces(&slot_meshes, slots);
		crate::loadlog::say(|| "ps: replay_faces done".into());
		// [PC] perf round 3: the static (slot, vertex) attribute of the vertex-shader posing (gpuskin.rs).
		let sj = self.gpu_posing.then(|| {
			let sj = super::gpuskin::slot_attr(&ranges);
			let b = sized_buffer(&self.device, "props slot attribute", (sj.len() + spare) * 8, wgpu::BufferUsages::VERTEX);
			self.wb(&b, 0, bytemuck::cast_slice(&sj));
			b
		});
		crate::loadlog::say(|| "ps: sj done".into());
		let rad: Vec<f32> = match self.gpu_posing {
			true => ranges.iter().map(|r| model[r.clone()].iter().map(|v| (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1] + v.pos[2] * v.pos[2]).sqrt()).fold(0.0, f32::max)).collect(),
			false => Vec::new(),
		};
		crate::loadlog::say(|| "ps: rad done".into());
		// [PC] Retail: each class's faces as `append_props` regroups them, built here (level load) so a pool growth does not.
		let mut uniq: Vec<(u32, &Mesh)> = if spare > 0 { props.iter().copied().zip(slot_meshes.iter().copied()).collect() } else { Vec::new() };
		uniq.sort_unstable_by_key(|u| u.0);
		uniq.dedup_by_key(|u| u.0);
		let class_faces: HashMap<u32, ClassFaces> = uniq.par_iter().map(|&(c, m)| (c, ClassFaces::of(m))).collect();
		PropSlots { ranges, prompt_slot, batch, model, slot_runs, faces, sj, rad, classes: props.to_vec(), class_faces }
	}

	pub(super) fn build_scene(&self, meshes: &[Mesh], shrub_set: &[shrubs::ShrubInstance], tie_set: &[ties::LitTie], textures: &HashMap<u32, Texture>, sky: Option<&sky::Sky>, hero: Option<&Mesh>, hero_tint: Option<([f32; 3], usize, usize)>, mut props_fn: impl FnMut(Option<&[u32]>) -> Option<Vec<u32>>, prop_meshes: &HashMap<u32, Mesh>, preview_meshes: &HashMap<u32, Mesh>, occ: crate::occlusion::Occlusion, tfrag_set: Vec<terrain::TfragLods>, tfrag_b: f32, terrain_chains: &[Option<Vec<Texture>>], shrub_chains: &[Option<Vec<Texture>>], tie_chains: &[Option<Vec<Texture>>]) -> Option<Scene> {
		let (mut bind, mut slots) = (Vec::new(), HashMap::new());
		*self.tex_log.borrow_mut() = self.bindless_layout.is_some().then(HashMap::new);
		let refs: Vec<&Mesh> = meshes.iter().collect();
		let (world, lo, hi, world_verts) = self.batch(&refs, None, false, textures, &mut bind, &mut slots);
		crate::loadlog::say(|| "gpu: world batch".into());
		// Ties: three LOD meshes per instance in their own batch (BRIDGE-29); the bounds still take LOD 0 so the scene
		// centre and radius are what the merged world batch gave.
		let tie_refs: Vec<&Mesh> = tie_set.iter().flat_map(|t| t.lods.iter()).collect();
		let (mut ties, _, _, tie_verts, tie_vr) = self.batch_instances(&tie_refs, textures, &mut bind, &mut slots, None, tie_chains);
		crate::loadlog::say(|| "gpu: ties batch".into());
		let tie_draws = std::mem::take(&mut ties.draws);
		let (mut lo, mut hi) = (lo, hi);
		for (_, vr) in tie_vr.iter().step_by(3) {
			for p in &tie_verts[vr.clone()] {
				for j in 0..3 {
					lo[j] = lo[j].min(p.pos[j]);
					hi[j] = hi[j].max(p.pos[j]);
				}
			}
		}
		let tie_ranges: Vec<[std::ops::Range<u32>; 3]> = tie_vr.chunks(3).map(|c| std::array::from_fn(|k| c[k].0.start as u32..c[k].0.end as u32)).collect();
		let tie_vranges: Vec<[std::ops::Range<usize>; 3]> = tie_vr.chunks(3).map(|c| std::array::from_fn(|k| c[k].1.clone())).collect();
		let tie_morph: Vec<[f32; 3]> = tie_set.iter().flat_map(|t| t.morph.iter().flatten().copied()).collect();
		let tie_fat: Vec<[bool; 3]> = tie_set.iter().map(|t| std::array::from_fn(|k| t.morph[k].iter().any(|d| d.iter().any(|&c| c != 0.0)))).collect();
		let tie_lit: Vec<[&Mesh; 3]> = tie_set.iter().map(|t| [&t.lods[0], &t.lods[1], &t.lods[2]]).collect();
		// Terrain fragments (BRIDGE-31): all three LOD meshes of each in their own batch; the near meshes join the scene bounds.
		// Their face keys carry the material record index (REN-TEX-RESIDENCY: K differs per record) and the textures upload their
		// authored mip chains; the tfrag pass samples an explicit level.
		let tf_meshes: Vec<Mesh> = tfrag_set.iter().flat_map(|t| [(&t.far, &t.face_mat[0]), (&t.mid, &t.face_mat[1]), (&t.near, &t.face_mat[2])]).map(|(m, mats)| terrain_face_keys(m, mats)).collect();
		let tf_refs: Vec<&Mesh> = tf_meshes.iter().collect();
		let rule = TerrainRule { chains: terrain_chains, materials: tfrag_set.iter().flat_map(|t| [&t.materials[..]; 3]).collect() };
		let (mut tfrags, _, _, mut tfrag_verts, tf_vr) = self.batch_instances(&tf_refs, textures, &mut bind, &mut slots, Some(&rule), &[]);
		crate::loadlog::say(|| "gpu: tfrag batch".into());
		let tfrag_draws = std::mem::take(&mut tfrags.draws);
		for (_, vr) in tf_vr.iter().skip(2).step_by(3) {
			for p in &tfrag_verts[vr.clone()] {
				for j in 0..3 {
					lo[j] = lo[j].min(p.pos[j]);
					hi[j] = hi[j].max(p.pos[j]);
				}
			}
		}
		let tfrag_ranges: Vec<[std::ops::Range<u32>; 3]> = tf_vr.chunks(3).map(|c| std::array::from_fn(|k| c[k].0.start as u32..c[k].0.end as u32)).collect();
		let tfrag_vranges: Vec<[std::ops::Range<usize>; 3]> = tf_vr.chunks(3).map(|c| std::array::from_fn(|k| c[k].1.clone())).collect();
		let tfrag_cols: Vec<Vec<[f32; 4]>> = tfrag_set.iter().map(|t| t.lit.colours.clone()).collect();
		let tfrag_lit: Vec<&Mesh> = tfrag_set.iter().map(|t| &t.lit).collect();
		// Shrubs (BRIDGE-27): one batch with per-instance ranges, bounds merged into the scene's, chained last in the light world.
		let shrub_refs: Vec<&Mesh> = shrub_set.iter().map(|s| &s.mesh).collect();
		let (shrub_batch, slo, shi, shrub_verts, shrub_ranges) = self.batch_instances(&shrub_refs, textures, &mut bind, &mut slots, None, shrub_chains);
		crate::loadlog::say(|| "gpu: shrub batch".into());
		for j in 0..3 {
			lo[j] = lo[j].min(slo[j]);
			hi[j] = hi[j].max(shi[j]);
		}
		let lit = dynlight::LitWorld::chain_tfrags(&tfrag_lit, meshes, &tie_lit, shrub_refs.iter().copied());
		crate::loadlog::say(|| "gpu: lit chain".into());
		let tie_first = world_verts.len();
		let shrub_first = tie_first + tie_verts.len();
		let tie_base: Vec<[f32; 3]> = tie_verts.iter().map(|v| v.pos).collect();
		// The LOD 1 / 2 meshes carry no `Lit`, so they take LOD 0's fog class tag and instance centre here.
		let mut tie_verts = tie_verts;
		for r in &tie_vranges {
			if let Some(l) = tie_verts.get(r[0].start).map(|v| v.light) {
				for lod in 1..3 {
					for v in &mut tie_verts[r[lod].clone()] {
						v.light = l;
					}
				}
			}
		}
		self.wb(&ties.vbuf, 0, bytemuck::cast_slice(&tie_verts));
		let tie_slots: Vec<ties::TieSlot> = tie_set.iter().flat_map(|t| t.slots.iter().flatten().copied()).collect();
		let tie_cols: Vec<Vec<[f32; 4]>> = tie_set.iter().map(|t| t.cols.clone()).collect();
		let tie_stale = vec![[false; 3]; tie_set.len()];
		let mut clear = wgpu::Color { r: 0.05, g: 0.07, b: 0.15, a: 1.0 };
		let sky_src = sky;
		crate::loadlog::say(|| "gpu: ties written".into());
		let (sky, sky_shells) = sky
			.map(|s| {
				let mut t: HashMap<u32, Texture> = s.textures.iter().map(|(k, v)| (*k, Texture { w: v.w, h: v.h, rgba: v.rgba.clone() })).collect();
				t.insert(sky::UNTEXTURED, Texture { w: 1, h: 1, rgba: vec![[255; 4]] });
				let c = s.colour.map(|x| x as f64 / 255.0); // the target is gamma-space Unorm: raw GS bytes
				clear = wgpu::Color { r: c[0], g: c[1], b: c[2], a: 1.0 };
				let refs: Vec<&Mesh> = s.meshes.iter().collect();
				let (batch, _, _, verts) = self.batch(&refs, Some(&s.colours), true, &t, &mut bind, &mut slots);
				// The shells' vertices follow each other in shell order (an ordered batch).
				let mut start = 0;
				let ranges = s
					.meshes
					.iter()
					.map(|m| {
						let r = start..start + m.positions.len();
						start = r.end;
						r
					})
					.collect();
				let base = s.meshes.iter().map(|m| m.positions.clone()).collect();
				// ARI-4: draws of shells 0 and 1 (no run straddles a shell).
				let first_two = s.meshes.iter().take(2).map(|m| m.faces.len() as u32 * 3).sum::<u32>();
				let split = batch.draws.iter().take_while(|d| d.start < first_two).count();
				(batch, SkyShells { verts, ranges, base, turned: vec![false; s.meshes.len()], posed: None, split })
			})
			.unzip();
		let sky_sprites = sky_src.and_then(|s| {
			let keys = [4 << 16 | 2, 4 << 16 | 3];
			if !keys.iter().all(|k| s.textures.contains_key(k)) {
				return None;
			}
			let bind_of = |k: u32, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>| match slots.get(&k) {
				Some(&i) => i,
				None => {
					bind.push(self.upload_texture(s.textures.get(&k)));
					slots.insert(k, bind.len() - 1);
					bind.len() - 1
				}
			};
			let tex = keys.map(|k| bind_of(k, &mut bind, &mut slots));
			let n = sky::SKY_SPRITES;
			let vbuf = buffer(&self.device, "sky sprites", bytemuck::cast_slice(&vec![Vertex::zeroed(); n * 4]), wgpu::BufferUsages::VERTEX);
			let idx: Vec<u32> = (0..n as u32).flat_map(|q| [0, 1, 2, 1, 2, 3].map(|i| q * 4 + i)).collect();
			let ibuf = buffer(&self.device, "sky sprite indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
			self.wb(&ibuf, 0, bytemuck::cast_slice(&idx));
			Some(SkySpriteDraw { vbuf, ibuf, bind: tex, counts: [0; 2] })
		});
		crate::loadlog::say(|| "gpu: sky done".into());
		let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
		let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
		let radius = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / 2.0;
		// The player model, in model space (feet at the origin, facing +X).
		const BOX: u32 = u32::MAX;
		let (boxed, mut t) = (box_mesh(BOX), HashMap::new());
		let hero = match hero {
			Some(m) => (m, textures),
			None => {
				t.insert(BOX, Texture { w: 1, h: 1, rgba: vec![[255, 140, 30, 255]] });
				(&boxed, &t)
			}
		};
		let (marker, _, _, marker_model) = self.batch(&[hero.0], None, false, hero.1, &mut bind, &mut slots);
		crate::loadlog::say(|| "gpu: marker".into());
		let marker_faces = super::clip::replay_faces(&[hero.0], &slots).pop().unwrap_or_default();
		// Gameplay props: one mesh copy per slot, posed every frame.
		// [PC] The level prefetch may build them for a speculative order (retail's pools a few fields before arrival): after the build
		// `props_fn(Some(built))` answers the order to keep (== built) or a newer one, rebuilt from the same texture slots (`bind`/`slots`
		// back to what they were here), so the result is the build of the final order alone.
		// (Before the prop slots: the prefetch thread has them done when the final order comes.)
		let tie_qdep: Vec<bool> = tie_slots.iter().zip(&tie_morph).map(|(s, d)| s.1 || *d != [0.0; 3]).collect();
		let (tie_tri, tie_tris) = tie_triples(&tie_slots, &tie_vranges);
		let (bind0, slots0) = (bind.len(), slots.clone());
		let mut order_p = props_fn(None)?;
		let (ranges, prompt_slot, props, props_model, slot_runs, prop_faces, props_sj, props_rad, props_class, class_faces, prompt, portrait, portrait_model, clip_cap, clip_batch, previews, preview_models, previews_gold) = loop {
		let PropSlots { ranges, prompt_slot, batch: props, model: props_model, slot_runs, faces: prop_faces, sj: props_sj, rad: props_rad, classes: props_class, class_faces } = self.prop_slots(&order_p, prop_meshes, textures, &mut bind, &mut slots);
		crate::loadlog::say(|| "gpu: prop_slots".into());
		// BRIDGE-36 (c1): the purchase prompt (class 1137) as its own one-prop batch, drawn between HUD groups at `hud_split`.
		let prompt = prompt_slot.and_then(|_| prop_meshes.get(&game::vendor::PROMPT_CLASS)).filter(|m| !m.positions.is_empty()).map(|m| self.batch(&[m], None, false, textures, &mut bind, &mut slots).0);
		let prompt_slot = prompt_slot.filter(|_| prompt.is_some());
		// BRIDGE-37 (b): the shop portrait (class 12) as a one-prop batch, posed by `pose_portrait`.
		let (portrait, portrait_model) = match prop_meshes.get(&game::vendor::PORTRAIT_CLASS).filter(|m| !m.positions.is_empty()) {
			Some(m) => {
				let (b, _, _, verts) = self.batch(&[m], None, false, textures, &mut bind, &mut slots);
				(Some(b), verts)
			}
			None => (None, Vec::new()),
		};
		crate::loadlog::say(|| "gpu: prompt/portrait".into());
		let clip_cap = (CLIP_VERTS, CLIP_INDICES);
		let clip_batch = Batch { vbuf: buffer(&self.device, "clip vertices", &vec![0u8; CLIP_VERTS * std::mem::size_of::<Vertex>()], wgpu::BufferUsages::VERTEX), ibuf: buffer(&self.device, "clip indices", &vec![0u8; CLIP_INDICES * 4], wgpu::BufferUsages::INDEX), draws: Vec::new() };
		crate::loadlog::say(|| "gpu: clip batch".into());
		// Menu item previews: bind pose, centred by bounding box, lit as menu mobies (FUN_00282a70: ambient 0x20,
		// light slots 14/14, blend 0; lighting_fog.md "Reserved level-light slots 12-14").
		let mut previews = HashMap::new();
		let mut preview_models = HashMap::new();
		let mut previews_gold = HashMap::new();
		for (&class, m) in preview_meshes.iter().filter(|(_, m)| !m.positions.is_empty()) {
			let (batch, lo, hi, mut verts) = self.batch(&[m], None, false, textures, &mut bind, &mut slots);
			for v in &mut verts {
				v.colour[3] = 0.0;
				v.light = Gpu::pack_light([0.25; 3], MENU_LIGHT_SLOT, MENU_LIGHT_SLOT, 0.0);
			}
			self.wb(&batch.vbuf, 0, bytemuck::cast_slice(&verts));
			// LIT-7a: a second copy lit by the gold-weapon light word (ambient 38 / 28 / 0, slots 12 / 12).
			let (gold, _, _, mut gverts) = self.batch(&[m], None, false, textures, &mut bind, &mut slots);
			for v in &mut gverts {
				v.colour[3] = 0.0;
				v.light = Gpu::pack_light(GOLD_AMBIENT, GOLD_LIGHT_SLOT, GOLD_LIGHT_SLOT, 0.0);
			}
			self.wb(&gold.vbuf, 0, bytemuck::cast_slice(&gverts));
			previews_gold.insert(class, gold);
			// Every class keeps its bind vertices: hero previews re-pose them, and the shop panel item A (`pose_panel_item`).
			preview_models.insert(class, verts);
			let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
			previews.insert(class, (batch, [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0], (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / 2.0));
		}
		let keep = props_fn(Some(&order_p))?;
		if keep == order_p {
			break (ranges, prompt_slot, props, props_model, slot_runs, prop_faces, props_sj, props_rad, props_class, class_faces, prompt, portrait, portrait_model, clip_cap, clip_batch, previews, preview_models, previews_gold);
		}
		crate::loadlog::say(|| "gpu: prop slots rebuilt for a newer order".into());
		bind.truncate(bind0);
		slots = slots0.clone();
		order_p = keep;
		};
		// [PC] perf round 3 (bindless.rs): the scene's texture slots as one array for the tie and shrub draws.
		let views = self.tex_log.borrow_mut().take();
		crate::loadlog::say(|| "gpu: previews done".into());
		let tex_array = self.bindless_layout.as_ref().zip(views.as_ref()).and_then(|(l, v)| super::bindless::array_group(l, &self.device, &bind, v, &self.sampler, &self.wrap_samplers));
		Some(Scene { previews, previews_gold, preview_models, world, world_verts, lit, ties, tie_draws, tie_ranges, tie_cull: tie_set.iter().map(|t| t.cull).collect(), occ_tick: occ.clone(), occ, tfrags, tfrag_set, tfrag_draws, tfrag_ranges, tfrag_vranges, tfrag_verts, tfrag_cols, tfrag_b, tie_verts, tie_base, tie_morph, tie_vranges, tie_fat, tie_q: vec![[0.0; 3]; tie_set.len()], tie_qdep, tie_full: vec![[false; 3]; tie_set.len()], tie_slots, tie_tri, tie_tris, tie_cols, tie_stale, tie_first, shrubs: shrub_batch, shrub_ranges, shrub_cull: shrub_set.iter().map(|s| s.cull).collect(), shrub_order: shrub_order(shrub_set), shrub_bb: shrub_set.iter().map(|s| s.bb).collect(), shrub_verts, shrub_first, shrub_a: vec![0; shrub_set.len()], shrub_opaque: Vec::new(), shrub_faded: Vec::new(), shrub_bb_quads: Vec::new(), shrub_bb_tex: HashMap::new(), sky, sky_shells, sky_sprites, marker, marker_verts: Vec::new(), marker_model, marker_tint: hero_tint.map(|(c, s, _)| (c, s)), marker_hero_len: hero_tint.map_or(0, |t| t.2), marker_shown: false, marker_alpha: false, props_verts: if self.gpu_posing { Vec::new() } else { props_model.clone() }, props_hidden: vec![false; ranges.len()], props_key: vec![None; ranges.len()], props, props_model, props_sj, props_rad, props_inst: Vec::new(), props_pose: Vec::new(), props_ranges: ranges, props_shown: false, props_alpha: false, props_drawn: Vec::new(), props_vis: Vec::new(), slot_runs, props_order: (0..props_class.len()).collect(), props_class, props_late: Vec::new(), prop_faces, class_faces, clip_masked: Vec::new(), clip_batch, clip_cap, clip_view: None, clip_queue: Vec::new(), marker_faces, marker_masked: Vec::new(), debris_model: HashMap::new(), debris: None, debris_cap: (0, 0), warp_model: HashMap::new(), warp_batch: None, warp_shown: false, prompt, prompt_slot, portrait, portrait_model, bind_groups: bind, tex_slots: slots, tex_array, clear, centre, radius })
	}
}
