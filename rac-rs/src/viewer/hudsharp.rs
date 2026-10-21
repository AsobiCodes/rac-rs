// [PC] Sharp title / main menu (lane G2, plan_title.md option M-A). The front end the mirror draws (viewer/front_mirror.rs: logo,
// PRESS START, button text and panel plates, `frontend.rs Menu::draw`) is PS2-native art, 512x448, drawn as HUD quads magnified to the
// window and sampled nearest: blocky at 1080p. With the sharp HUD on (`retail::ui_sharp`: the option, the extension, a window at least
// 896 px tall) the art quads take `fs_hud_sharp` (hudsharp.wgsl, the edge-aware sampler of lane G's sharp HUD) instead of `fs_hud`:
// same positions, same blend states, same draw order; only the texture reconstruction differs. Off: nothing changes (same vertex data,
// same pipelines). The 3D backdrop is not touched.
//
// The state is module-level (one window, one device): `set` every front-end frame (front_mirror_frame), `wants` per quad while the
// HUD vertex buffer is built (gpu.rs), `pipelines` for the two sharp pipelines (alpha, additive), built on the first sharp frame.
use super::Vertex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::OnceLock;

/// The shader (vs_hud_s, fs_hud_sharp); RAC_WGSL_DIR=<dir> with hud_sharp.wgsl in it replaces it (devtune.rs).
const SHADER: &str = include_str!("hudsharp.wgsl");

static ON: AtomicBool = AtomicBool::new(false);
/// HUD texture slots that stay nearest: the noise sprite and the scanline sprite of the panel frames (tiled, one texel per row by design).
static FLAT: [AtomicUsize; 2] = [AtomicUsize::new(usize::MAX), AtomicUsize::new(usize::MAX)];
static PIPES: OnceLock<[wgpu::RenderPipeline; 2]> = OnceLock::new();

/// This frame's sharp state: `on`, and the slots of the sprites that keep the nearest look.
pub(super) fn set(on: bool, flat: [usize; 2]) {
	ON.store(on, Ordering::Relaxed);
	for (a, s) in FLAT.iter().zip(flat) {
		a.store(s, Ordering::Relaxed);
	}
}

/// Whether a quad of HUD texture `tex` (of `slots` slots) takes the sharp shader: slot 0 is the solid rectangles (the built-in font
/// cell), the retail overlay / filter and the item / popup / portrait panels have ids past the slots; the flat slots stay nearest.
pub(super) fn wants(tex: usize, slots: usize) -> bool {
	ON.load(Ordering::Relaxed) && tex >= 1 && tex < slots && FLAT.iter().all(|f| f.load(Ordering::Relaxed) != tex)
}

/// The sharp HUD pipelines [alpha blend, additive], built on first use. `like` is any pipeline made with the level layout (group 0 the
/// uniforms, group 1 `tex_layout`): its group 0 layout is shared so the pass keeps its bind groups.
pub(super) fn pipelines(device: &wgpu::Device, like: &wgpu::RenderPipeline, tex_layout: &wgpu::BindGroupLayout, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> &'static [wgpu::RenderPipeline; 2] {
	PIPES.get_or_init(|| {
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("hud sharp"), bind_group_layouts: &[Some(&like.get_bind_group_layout(0)), Some(tex_layout)], immediate_size: 0 });
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("hud sharp"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("hud sharp", SHADER.into())) });
		let additive = wgpu::BlendState {
			color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
			alpha: wgpu::BlendComponent::OVER,
		};
		let make = |blend: wgpu::BlendState| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some("hud sharp"),
				layout: Some(&layout),
				vertex: wgpu::VertexState {
					module: &shader,
					entry_point: Some("vs_hud_s"),
					compilation_options: Default::default(),
					buffers: &[Some(wgpu::VertexBufferLayout {
						array_stride: std::mem::size_of::<Vertex>() as u64,
						step_mode: wgpu::VertexStepMode::Vertex,
						attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4],
					})],
				},
				fragment: Some(wgpu::FragmentState {
					module: &shader,
					entry_point: Some("fs_hud_sharp"),
					compilation_options: Default::default(),
					targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
				}),
				primitive: Default::default(),
				depth_stencil: Some(wgpu::DepthStencilState {
					format: depth_format,
					depth_write_enabled: Some(false),
					depth_compare: Some(wgpu::CompareFunction::Always),
					stencil: Default::default(),
					bias: Default::default(),
				}),
				multisample: wgpu::MultisampleState { count: super::MSAA, ..Default::default() },
				multiview_mask: None,
				cache: None,
			})
		};
		[make(wgpu::BlendState::ALPHA_BLENDING), make(additive)]
	})
}

#[cfg(test)]
mod tests {
	/// The sharp shader parses and validates (naga), and its sampler bodies stay the ones of lane G's sharp HUD
	/// (retail/gscap/draw/gpu.wgsl): the same constants.
	#[test]
	fn shader_validates() {
		let m = naga::front::wgsl::parse_str(super::SHADER).expect("wgsl parse");
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	}

	#[test]
	fn tunables_match_the_gscap_sharp_hud() {
		let gs = include_str!("../retail/gscap/draw/gpu.wgsl");
		for line in super::SHADER.lines().filter(|l| l.starts_with("const HD_")) {
			assert!(gs.lines().any(|g| g.trim_end() == line.trim_end()), "gpu.wgsl lost or changed: {line}");
		}
	}

	#[test]
	fn quad_selection() {
		super::set(false, [3, 4]);
		assert!(!super::wants(1, 10));
		super::set(true, [3, 4]);
		assert!(super::wants(1, 10) && super::wants(9, 10) && super::wants(5, 10));
		assert!(!super::wants(0, 10), "solid rectangles");
		assert!(!super::wants(3, 10) && !super::wants(4, 10), "noise and scanline sprites");
		assert!(!super::wants(10, 10) && !super::wants(usize::MAX - 3, 10), "retail overlay and panels");
		super::set(false, [usize::MAX; 2]);
	}
}
