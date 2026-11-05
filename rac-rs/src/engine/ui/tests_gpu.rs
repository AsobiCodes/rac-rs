//! The GPU objects of the modern UI over a menu opening and closing, on a real device (skipped when the machine has no Vulkan
//! adapter): the shader and pipelines are built once and kept; a frame without a scene frees only what scales with the window or
//! the page. The same `draw` the renderer calls, so wgpu's validation also checks the pass, the copy and the bind groups.
use super::demo;
use super::font::FontMeasure;
use super::gpu::{self, Held, UiGpu};
use super::prim::Prim;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
	let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
	desc.backends = wgpu::Backends::VULKAN;
	let inst = wgpu::Instance::new(desc);
	let adapter = pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
	pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

fn target(device: &wgpu::Device, format: wgpu::TextureFormat, (w, h): (u32, u32)) -> wgpu::Texture {
	device.create_texture(&wgpu::TextureDescriptor {
		label: Some("ui test window"),
		size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
		view_formats: &[],
	})
}

fn page() -> Vec<Prim> {
	demo::new_ui(demo::DemoSpec::default(), Box::new(FontMeasure)).frame().1
}

#[test]
fn the_pipelines_are_built_once_and_a_closed_menu_frees_only_the_window_size_objects() {
	let Some((device, queue)) = device() else {
		eprintln!("no Vulkan device: skipped");
		return;
	};
	let (fmt_a, fmt_b, win) = (wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm, (320, 180));
	let (win_a, win_b) = (target(&device, fmt_a, win), target(&device, fmt_b, win));
	let frame = |g: &mut UiGpu, scene: Option<Vec<Prim>>, tex: &wgpu::Texture, fmt| {
		if let Some(p) = scene {
			g.set_scene(p);
		}
		let mut enc = device.create_command_encoder(&Default::default());
		let drawn = g.draw(&device, &queue, &mut enc, tex, fmt, win);
		queue.submit([enc.finish()]);
		drawn
	};
	let mut g = UiGpu::default();
	assert!(!g.is_allocated());
	g.warm(&device, fmt_a);
	if gpu::refused() {
		eprintln!("this driver refuses the UI shader: skipped");
		return;
	}
	let kept = Held { pipes: true, cache: true, ..Held::default() };
	assert_eq!((g.pipe_builds(), g.held()), (1, Held { pipes: true, ..Held::default() }), "warm-up builds the pipelines and nothing else");

	assert!(frame(&mut g, Some(page()), &win_a, fmt_a), "the first menu frame draws");
	let open = Held { pipes: true, cache: true, atlas: true, backdrop: true, insts: true, ..Held::default() };
	assert_eq!((g.pipe_builds(), g.held()), (1, open), "the first open does not build them again");

	// The menu closes: no scene. The blur chain, the atlas texture and the instance buffer go; the pipelines and the glyphs stay.
	assert!(!frame(&mut g, None, &win_a, fmt_a));
	assert_eq!((g.pipe_builds(), g.held()), (1, kept));
	assert!(!frame(&mut g, None, &win_a, fmt_a), "a second closed frame changes nothing");
	assert_eq!((g.pipe_builds(), g.held()), (1, kept));

	// Open again, and a one-frame gap between two pages: still no rebuild.
	assert!(frame(&mut g, Some(page()), &win_a, fmt_a));
	assert_eq!((g.pipe_builds(), g.held()), (1, open));
	assert!(!frame(&mut g, None, &win_a, fmt_a));
	assert!(frame(&mut g, Some(page()), &win_a, fmt_a));
	assert_eq!((g.pipe_builds(), g.held()), (1, open), "a gap frame between pages costs no shader or pipeline build");

	// A different window format needs its own pipelines (and drops what was made with the old ones).
	assert!(frame(&mut g, Some(page()), &win_b, fmt_b));
	assert_eq!((g.pipe_builds(), g.held()), (2, open));

	// `release` frees everything, the pipelines included; drawing again builds them again.
	g.release();
	assert!(!g.is_allocated() && g.held() == Held::default());
	assert!(frame(&mut g, Some(page()), &win_b, fmt_b));
	assert_eq!(g.pipe_builds(), 3);
}
