//! Gpu::new (instance, adapter, device, surface, every pipeline and buffer), resize, surface configuration and present mode.

use super::*;

impl Gpu {
	pub(in crate::viewer) fn new(window: Arc<Window>) -> Gpu {
		// [PC] The graphics API: the Renderer row's `renderer=` in the options file, WGPU_BACKEND overrides (renderer.rs).
		let (_instance, surface, adapter) = super::renderer::open(&window);
		super::tiegpu::note_adapter(&adapter);
		// [PC] perf round 3 (bindless.rs): texture binding arrays for the tie / shrub draws when the adapter has them.
		let bindless = super::bindless::wanted(&adapter);
		let multi = bindless && super::bindless::multi_wanted(&adapter);
		// BRIDGE-64a: the shadow volumes count in a stencil aspect of the main depth target (Depth32FloatStencil8 where the
		// adapter has it, else the core Depth24PlusStencil8).
		let stencil32 = adapter.features().contains(wgpu::Features::DEPTH32FLOAT_STENCIL8);
		let depth_format = if stencil32 { wgpu::TextureFormat::Depth32FloatStencil8 } else { wgpu::TextureFormat::Depth24PlusStencil8 };
		let stencil_feature = if stencil32 { wgpu::Features::DEPTH32FLOAT_STENCIL8 } else { wgpu::Features::empty() } | crate::texpack::gpu_features(&adapter) | crate::retail::gpu_capture_features(&adapter) | crate::viewer::volfog::gpu_features(&adapter) | super::passtime::features(&adapter); // [PC] texture packs: BC formats when the adapter has them; GS capture (shares this device on Vulkan): pipeline cache
		// [PC] perf (lane D5b, tiegpu.rs): the tie vertex buffer (64 B x ~3M vertices) is bound as one storage buffer, which needs more than
		// the 128 MiB default binding size where the adapter offers it (else the path stays off).
		let storage_bind = adapter.limits().max_storage_buffer_binding_size.min(512 << 20).max(wgpu::Limits::default().max_storage_buffer_binding_size);
		let desc = match bindless {
			true => wgpu::DeviceDescriptor {
				required_features: super::bindless::features() | stencil_feature | if multi { wgpu::Features::INDIRECT_FIRST_INSTANCE } else { wgpu::Features::empty() },
				required_limits: wgpu::Limits { max_binding_array_elements_per_shader_stage: super::bindless::TEX_CAP, max_storage_buffer_binding_size: storage_bind, ..Default::default() },
				..Default::default()
			},
			false => wgpu::DeviceDescriptor { required_features: stencil_feature, required_limits: wgpu::Limits { max_storage_buffer_binding_size: storage_bind, ..Default::default() }, ..Default::default() },
		};
		let (device, queue) = pollster::block_on(adapter.request_device(&desc)).expect("device");
		// [PC] wgpu drops a lost device's errors (driver reset, GPU timeout) and the window just stays blank: say so in the log.
		device.set_device_lost_callback(|reason, msg| {
			if reason != wgpu::DeviceLostReason::Destroyed {
				eprintln!("gpu: the graphics device was lost ({reason:?}): {msg}");
			}
		});
		super::gpuhero::device_created(&adapter, &device);  super::passtime::init(&device, &queue); // [PC] RAC_GPU_PASSES (passtime.rs)
		let size = window.inner_size();
		let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
		config.usage |= wgpu::TextureUsages::COPY_SRC; // for --shot
		// The GS blends and modulates raw 8-bit values: render in gamma space like it does.
		config.format = config.format.remove_srgb_suffix();
		config.present_mode = present_mode(&surface.get_capabilities(&adapter).present_modes, std::env::var("RAC_VSYNC").is_ok_and(|v| v == "1"), std::env::var("RAC_PRESENT").ok().as_deref());
		// [PC] Swapchain frames in flight: wgpu's 2, 3 on DirectX 12 (renderer.rs); RAC_FRAME_LATENCY (1..3) is the A/B knob.
		config.desired_maximum_frame_latency = super::renderer::frame_latency(adapter.get_info().backend, std::env::var("RAC_FRAME_LATENCY").ok().and_then(|v| v.parse::<u32>().ok()), config.desired_maximum_frame_latency);
		configure_surface(&surface, &device, &config);  let win = super::rscale::start_size(&device, &mut config); // [PC] Render scale (rscale.rs): `config` holds the RENDER size from here on, the swapchain above got the window size
		eprintln!("gpu: {:?} {}, present mode {:?}, frame latency {}", adapter.get_info().backend, adapter.get_info().name, config.present_mode, config.desired_maximum_frame_latency);
		// The driver (bug reports: a fault on one driver version only).
		let ai = adapter.get_info();
		eprintln!("gpu: driver {} {}, vendor {:#06x} device {:#06x}, {:?}", ai.driver, ai.driver_info, ai.vendor, ai.device, ai.device_type);
		// The GS capture shares this device when it is Vulkan, else it is on by default on NVIDIA only (gscap draw/gpu_shared.rs).
		crate::retail::gpu_capture_window(&device, &queue, &ai, &adapter);

		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("level"),
			source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("level", super::volfog::soft::level_source(SHADER, super::volfog::soft::early(&device)).into())),
		});
		// [PC] Bindings 1 and 2: the dynamic-shadow cascades and their compare sampler (sunshadow.rs; read only while shadows run); 3 volfog/soft.rs.
		let ulayout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("uniforms"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
					ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
					count: None,
				},
				wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None }, super::volfog::soft::LAYOUT_ENTRY,
			],
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
		let line_pipeline = make_prim("vs", "fs_line", Some(wgpu::BlendState::ALPHA_BLENDING), false, super::gs_state::ztest(), wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() });
		let pipeline = make("vs", "fs", None, true, super::gs_state::ztest());
		// Moby packets whose VU1 cull mode is 2 (back) or 0 (front) (NOV-5.9a).
		let pipeline_cull = [
			make_cull("vs", "fs", None, true, super::gs_state::ztest(), Some(wgpu::Face::Back)),
			make_cull("vs", "fs", None, true, super::gs_state::ztest(), Some(wgpu::Face::Front)),
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
		let tfrag_pipeline = make("vs_tfrag", "fs_tfrag", Some(wgpu::BlendState::ALPHA_BLENDING), true, super::gs_state::ztest());
		// Texels below alpha 0x60 (AFAIL RGB_ONLY): blended, no z write; drawn after the other batches.
		let tfrag_pipeline_lt = make("vs_tfrag", "fs_tfrag_lt96", Some(wgpu::BlendState::ALPHA_BLENDING), false, super::gs_state::ztest());
		// Ties (TieProc TEST_1 0x5340b, ALPHA_1 0x44): texels from alpha 0x40 blend and write Z, the rest blend without Z (drawn after
		// the other batches, as `tfrag_pipeline_lt`); per pass no cull, back, front (the packet's VU1 cull mode, `Draw::cull`).
		let tie_pipelines = [("fs_tie", true), ("fs_tie_lt64", false)].map(|(fs, z)| [None, Some(wgpu::Face::Back), Some(wgpu::Face::Front)].map(|c| make_cull("vs", fs, Some(wgpu::BlendState::ALPHA_BLENDING), z, super::gs_state::ztest(), c)));
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
			make("vs", "fs_part", Some(wgpu::BlendState::ALPHA_BLENDING), false, super::gs_state::ztest()),
			make("vs", "fs_part", Some(additive), false, super::gs_state::ztest()),
			// Water (prog10 fogs per vertex like tfrag, lighting_fog.md): alpha blend with the scene fog.
			make("vs", "fs_part_fog", Some(wgpu::BlendState::ALPHA_BLENDING), false, super::gs_state::ztest()),
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
			make("vs", "fs_ocean", None, true, super::gs_state::ztest()),
			make("vs", "fs_part_fix", None, true, super::gs_state::ztest()),
			make("vs", "fs_ocean_k", Some(fix_blend), false, super::gs_state::ztest()),
			make("vs", "fs_part_fix", Some(fix_blend), false, super::gs_state::ztest()),
			// Retail-mode RAM oceans (`fs_ocean_f`): 4 = opaque (FIX 0x80), 5 = FIX-blended.
			make("vs", "fs_ocean_f", None, true, super::gs_state::ztest()),
			make("vs", "fs_ocean_f", Some(fix_blend), false, super::gs_state::ztest()),
		];
		// Retail mode: the oceans of Rilgar, Umbris and Gaspar (own fog per vertex, `fs_ocean_g`): opaque with Z, or FIX-blended without Z.
		// 2 / 3 = the Gaspar second lava pass (coplanar with the first: GS TEST ZTST GEQUAL = nearer or equal passes).
		let ocean_g_pipeline = [
			make("vs", "fs_ocean_g", None, true, super::gs_state::ztest()),
			make("vs", "fs_ocean_g", Some(fix_blend), false, super::gs_state::ztest()),
			make("vs", "fs_ocean_g", None, true, wgpu::CompareFunction::LessEqual),
			make("vs", "fs_ocean_g", Some(fix_blend), false, wgpu::CompareFunction::LessEqual),
		];
		let bindless = bindless.then(|| {
			let bl_layout = super::bindless::Bindless::layout(&device);
			let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("level bindless"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("level bindless", super::bindless::shader_source(&super::volfog::soft::level_source(SHADER, super::volfog::soft::early(&device))).into())) });
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
						depth_compare: Some(compare),
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
				tie: [("fs_tie", true), ("fs_tie_lt64", false)].map(|(fs, z)| [None, Some(wgpu::Face::Back), Some(wgpu::Face::Front)].map(|c| make_bl("vs", fs, Some(wgpu::BlendState::ALPHA_BLENDING), z, c, super::gs_state::ztest()))),
				pipeline: make_bl("vs", "fs", None, true, None, super::gs_state::ztest()),
				pipeline_cull: [make_bl("vs", "fs", None, true, Some(wgpu::Face::Back), super::gs_state::ztest()), make_bl("vs", "fs", None, true, Some(wgpu::Face::Front), super::gs_state::ztest())],
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
		let props_gpu = (|| {
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
			let opaque = make_props("fs", None, true, super::gs_state::ztest(), None);
			let cull = [
				make_props("fs", None, true, super::gs_state::ztest(), Some(wgpu::Face::Back)),
				make_props("fs", None, true, super::gs_state::ztest(), Some(wgpu::Face::Front)),
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
			super::gpuskin::PropsGpu::new(&device, skin_layout, opaque, cull, moby, [mc0, mc1, mc2, mc3], super::moby_add::pipelines(&make_props))
		})();
		let sun = super::pipe_guard::build(&device, super::pipe_guard::Effect::SunShadow, || super::sunshadow::SunShadow::new(&device, Some(&props_gpu.layout), bindless.as_ref().filter(|b| b.indirect.is_some()).map(|b| &b.layout)));
		let ubuf = buffer(&device, "uniforms", bytemuck::bytes_of(&Uniforms::zeroed()), wgpu::BufferUsages::UNIFORM);
		let ubind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("uniforms"), layout: &ulayout, entries: &sun.entries(&ubuf) });
		let mirror: Vec<super::mirror::Mirror> = (0..super::mirror::MAX_MIRRORS).map(|_| super::mirror::Mirror::new(&device, &ulayout, &sun)).collect();
		let bed = super::mirror::Mirror::new(&device, &ulayout, &sun);
		let pbuf = [0, 1, 2, 3, 4, 5, 6, 7].map(|_| buffer(&device, "preview uniforms", bytemuck::bytes_of(&Uniforms::zeroed()), wgpu::BufferUsages::UNIFORM));
		let pbind = [0, 1, 2, 3, 4, 5, 6, 7].map(|i| {
			device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("preview uniforms"),
				layout: &ulayout,
				entries: &sun.entries(&pbuf[i]),
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
		let shadow = super::pipe_guard::build(&device, super::pipe_guard::Effect::ShadowVol, || super::shadowvol::ShadowPipes::new(&device, &ulayout, config.format, depth_format));
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
		let post = super::pipe_guard::build(&device, super::pipe_guard::Effect::Post, || super::post::Post::new(&device, config.format)); let volfog = super::volfog::VolFog::new(config.format, &ulayout); let volclouds = super::volclouds::VolClouds::new(config.format, depth_format);
		let waterfx = super::pipe_guard::build(&device, super::pipe_guard::Effect::Water, || super::waterfx::WaterFx::new(&device, config.format, depth_format, &tex_layout));
		let under = super::underwater::Under::new(&device, config.format);
		let partfx = super::partfx::PartFx::new(&device, &shader, &ulayout, &tex_layout, config.format, depth_format);
		let veg = super::pipe_guard::build(&device, super::pipe_guard::Effect::Veg, || super::vegfx::VegFx::new(&device, &ulayout, &tex_layout, config.format, depth_format));
		let presenter = super::presenter::Presenter::new(queue.clone(), ai.vendor);
		let queue = presenter.gated();
		let mut gpu = Gpu { window, surface, device, queue, presenter, config, pipeline, pipeline_cull, moby_pipelines, moby_cull, shrub_pipelines, clamp_sampler, wrap_samplers, tfrag_pipeline, tfrag_pipeline_lt, tie_pipelines, sky_pipeline, sky_add_pipeline, hud_pipeline, clear_pipeline, pbuf, pbind, preview: None, preview_gold: false, preview_pair: None, preview_worn: None, preview2: None, shop_models: Vec::new(), shop_panel_models: Vec::new(), item_panel, popup_panel, portrait_panel, portrait_shown: false, popup_bolt: None, icon_bolt: None, icon_bolt_sphere: None, popup_bolt_sphere: None, panel_posed: None, metal: MetalDraw::default(), props_gpu, bindless, hud_split: None, hud_textures: Vec::new(), nearest, hud: Vec::new(), retail_ov: None, ubuf, ubind, tex_layout, sampler, depth, msaa, scene: None, lighting: None, reserved_lights: None, fog_live: None, shrub_cap: shrubs::SHRUB_CAP, part_far: crate::water::PART_FAR, under_flag: false, under_surf: None, under, points: [[0.0; 4]; 16], table: [None; 8], moby_dyn: Vec::new(), hero_placed: (gameplay::SPAWN_AMBIENT, 0), hero_env: gameplay::placed_env(gameplay::SPAWN_AMBIENT, 0), scene_actor_light: None, marker_v: None, prop_v: None, class_uv: HashMap::new(), marker_ghosts: [None; 5], marker_loose: [None; 3], hero_gens: (0, 0), marker_bomb: None, marker_light: None, part_pipelines, strip_pipeline, pool_pipeline, pool_add_pipeline, part_tex: None, part_verts: BTreeMap::new(), line_pipeline, line_verts: Vec::new(), part_defs: Vec::new(), fx_meshes: HashMap::new(), fx_tex: HashMap::new(), orig_gpu: Default::default(), fx_quads: Vec::new(), water_quads: Vec::new(), strip_draws: Vec::new(), pool_draws: Vec::new(), ocean_pipeline, ocean_tex: None, ocean_draws: Vec::new(), ocean_rgba: Vec::new(), ocean_frame: None, ocean_level_fog: false, ocean_fog: None, ocean_g_pipeline, ocean_ram: Vec::new(), warp_fx: None, movie, hero_tint_out: None, depth_format, shadow, volume_buf: None, volume_counts: (0, 0), marker_tilt: [0.0; 2], post, volfog, volclouds, waterfx, mirror, bed, partfx, veg, level: usize::MAX, modern_ok: false, sun, ship_fight: false, hud_place: None, retail_ov_guard: Default::default(), reshade: Default::default(), rscale: Default::default() }; gpu.rscale_plan(win);
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
		crate::loadlog::say(|| "host: gpu ready (pipelines built)".into());
		gpu
	}

	pub(in crate::viewer) fn resize(&mut self, w: u32, h: u32) {
		if w == 0 || h == 0 {
			return;
		}
		// [PC] Render scale (rscale.rs): (w, h) is the WINDOW size; `config` takes the render size, the swapchain the window size.
		self.presenter.flush();
		let (rw, rh) = self.rscale_plan((w, h));
		self.config.width = rw;
		self.config.height = rh;
		self.reconfigure_surface();
		self.depth = depth_view(&self.device, rw, rh, self.depth_format);
		self.msaa = msaa_view(&self.device, self.config.format, rw, rh);
		self.post.reset();
	}
}

/// [PC] (user request: high frame rates) the swapchain present mode: uncapped by default, Mailbox if the surface has it,
/// else Immediate, else Fifo; `vsync` (RAC_VSYNC=1) forces Fifo. `named` (RAC_PRESENT=immediate / mailbox / fifo) picks one
/// when the surface has it (Immediate tears but costs the least CPU per present). Only the render rate changes: the game
/// still steps its fixed 50 Hz ticks from the frame-time accumulator (handler.rs RedrawRequested).
/// [PC] (Re)creates the swapchain. An uncapped mode is configured over a Fifo swapchain: on Vulkan / NVIDIA (RTX 5070 Ti)
/// an Immediate swapchain created that way costs ~0.35 ms less CPU in submit + present per frame and has fewer present
/// stalls (bench medians: present 1.75 -> 1.42 ms, p99 18.8 -> 15.0 ms) than one created directly; harmless elsewhere.
pub(super) fn configure_surface(surface: &wgpu::Surface, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) {
	let _no_capture_submit = crate::retail::gpu_capture_submit_lock(); // wgpu fails the configure's wait for idle when another thread submits meanwhile
	if config.present_mode != wgpu::PresentMode::Fifo {
		surface.configure(device, &wgpu::SurfaceConfiguration { present_mode: wgpu::PresentMode::Fifo, ..config.clone() });
	}
	surface.configure(device, config);
}

pub(super) fn present_mode(supported: &[wgpu::PresentMode], vsync: bool, named: Option<&str>) -> wgpu::PresentMode {
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
