//! [PC] (Split view: never in the Classic pass, `frame_target` sets `rs` false there.) ReShade add-on: when a ReShade with add-on support is loaded into the process (dxgi.dll / d3d12.dll next to the exe, or its
//! Vulkan layer), the game registers with it and hands over the real depth buffer and the picture BEFORE the HUD, so ReShade
//! effects (depth of field, fog, SSAO, ...) get a correct DEPTH with no manual setup and never touch the HUD or the menus.
//! With no ReShade loaded nothing of this runs: `reshade_begin` is one atomic load per frame. RAC_RESHADE=0 never registers;
//! RAC_RESHADE=dry runs the passes below with no ReShade (a test of the picture path: the output must equal the normal one).
//!
//! Frame with the feature on (`rs`), in `draw_passes`: the level (and in Enhanced the post chain) is drawn as always; then
//! part A (targets.rs `encode`) copies the level's depth to a plain R32Float texture and resolves the MSAA picture into `rt`;
//! the encoder is split there (presenter.rs `Cuts::split`) with a hook that runs between the two queue submits: it calls
//! ReShade's `render_effects` on `rt` (the add-on API, abi.rs) and flushes ReShade's immediate command list; part B (`hud_pass`)
//! starts the HUD pass by copying `rt` (now with the effects) into every sample of the MSAA target, then the HUD draws over it
//! as before. ReShade skips its own present-time effects for a frame it rendered itself.
//! Files: `abi` (exports, event ids, virtual-table slots), `native` (the views ReShade takes: created by ReShade's own device on
//! D3D12, the image views of the wgpu textures on Vulkan),
//! `targets` (textures, pipelines, the passes; `passes.wgsl`). Threads: the hook runs on whichever thread submits the frame
//! (the render thread, or the present thread, the same one that calls present, like ReShade's own render).

use super::*;
use self::abi::{Exports, Runtime, API_NEWEST, API_OLDEST, EV_BEGIN_EFFECTS, EV_DESTROY_RUNTIME, EV_INIT_RUNTIME};
use self::native::{Native, Under};
use self::targets::{Pipes, Targets};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering::{AcqRel, Acquire, Relaxed, Release}};
use std::sync::{Arc, OnceLock};

mod abi;
mod native;
mod targets;

const OFF: u8 = 0;
const LIVE: u8 = 1;
const DRY: u8 = 2;

/// What ReShade told us and what we told it (STATE.md `reshade::LINK`). Written by the main thread at attach and by ReShade's
/// event callbacks, read by the render thread and (in the hook) by the present thread.
struct Link {
	mode: AtomicU8,
	/// 1 Vulkan, 2 DirectX 12 (the backend wgpu runs on).
	backend: AtomicU8,
	/// The current `effect_runtime*` (0 until init_effect_runtime fires, again 0 after destroy_effect_runtime).
	runtime: AtomicUsize,
	/// Counts runtime creations and destructions: a hook recorded under an older epoch does not render.
	epoch: AtomicU64,
	/// The epoch whose preprocessor definitions are set.
	defs_epoch: AtomicU64,
	/// The depth texture's shader resource view handle, for the begin_effects callback.
	depth_srv: AtomicU64,
	/// True while our render_effects call runs (the callback only rebinds DEPTH then).
	rendering: AtomicBool,
}

static LINK: Link = Link { mode: AtomicU8::new(OFF), backend: AtomicU8::new(0), runtime: AtomicUsize::new(0), epoch: AtomicU64::new(0), defs_epoch: AtomicU64::new(u64::MAX), depth_srv: AtomicU64::new(0), rendering: AtomicBool::new(false) };

/// The ReShade exports found at attach (None: no ReShade, or dry run).
static EXPORTS: OnceLock<Exports> = OnceLock::new();
/// True once the begin_effects callback is registered (it is registered late, see `on_init`).
static BEGIN_REGISTERED: AtomicBool = AtomicBool::new(false);

/// The depth convention ReShade is told (projection.rs `view_proj`: right-handed, depth 0..1, not reversed, not logarithmic, not
/// flipped, hyperbolic z/w; far / near = 728.1632 / 0.03125). A definition is only set when it differs (a change recompiles effects).
const DEFINITIONS: [(&std::ffi::CStr, &str); 4] = [
	(c"RESHADE_DEPTH_INPUT_IS_REVERSED", "0"),
	(c"RESHADE_DEPTH_INPUT_IS_UPSIDE_DOWN", "0"),
	(c"RESHADE_DEPTH_INPUT_IS_LOGARITHMIC", "0"),
	(c"RESHADE_DEPTH_LINEARIZATION_FAR_PLANE", "23301.0"),
];

/// RAC_RESHADE: `0` off, `dry` the passes without ReShade, anything else (or unset) on when ReShade is there.
fn wanted(setting: Option<&str>) -> Option<u8> {
	match setting {
		Some("0") => None,
		Some("dry") => Some(DRY),
		_ => Some(LIVE),
	}
}

fn backend_code(b: wgpu::Backend) -> u8 {
	match b {
		wgpu::Backend::Vulkan => 1,
		wgpu::Backend::Dx12 => 2,
		_ => 0,
	}
}

fn backend_of(code: u8) -> wgpu::Backend {
	match code {
		1 => wgpu::Backend::Vulkan,
		2 => wgpu::Backend::Dx12,
		_ => wgpu::Backend::Noop,
	}
}

/// Called once from `renderer::open`, before the swapchain exists (ReShade creates its runtime with the swapchain).
pub(in crate::viewer) fn attach(backend: wgpu::Backend) {
	let Some(mode) = wanted(std::env::var("RAC_RESHADE").ok().as_deref()) else { return };
	if backend_code(backend) == 0 {
		return;
	}
	LINK.backend.store(backend_code(backend), Relaxed);
	if mode == DRY {
		LINK.mode.store(DRY, Release);
		eprintln!("reshade: dry run (the add-on passes without ReShade), {backend:?}");
		return;
	}
	let Some(ex) = abi::find_reshade() else { return };
	let module = abi::exe_module();
	let accepted = (API_OLDEST..=API_NEWEST).rev().find(|&v| unsafe { (ex.register)(module, v) });
	let Some(api) = accepted else {
		eprintln!("reshade: found, but it refused the add-on registration (API {API_OLDEST}..{API_NEWEST}); running without it");
		return;
	};
	// begin_effects is registered later (in `on_init`): ReShade loads its add-ons when the device is created, and the callbacks of
	// an event run in registration order, so a depth add-on loaded by ReShade (generic_depth) has to run before ours.
	let events: [(u32, *mut c_void); 2] = [(EV_INIT_RUNTIME, on_init as *mut c_void), (EV_DESTROY_RUNTIME, on_destroy as *mut c_void)];
	for (ev, cb) in events {
		unsafe { (ex.register_event)(module, ev, cb) };
	}
	let _ = EXPORTS.set(ex);
	LINK.mode.store(LIVE, Release);
	eprintln!("reshade: registered as add-on (API {api}), {backend:?}");
}

unsafe extern "C" fn on_init(runtime: *mut c_void) {
	LINK.runtime.store(runtime as usize, Release);
	LINK.epoch.fetch_add(1, Release);
	eprintln!("reshade: effect runtime created");
	// The first runtime exists, so ReShade has loaded every add-on it loads: register now, after all of them. Not on the first
	// frame instead: ReShade's own present-time render can walk this event's list on another thread at that moment.
	if let Some(ex) = EXPORTS.get() {
		if !BEGIN_REGISTERED.swap(true, Relaxed) {
			unsafe { (ex.register_event)(abi::exe_module(), EV_BEGIN_EFFECTS, on_begin as *mut c_void) };
		}
	}
}

/// Ends the add-on's life with the renderer (`Slot::drop`): the events stop, the registration goes (ReShade warns at exit about
/// add-ons that are still registered). The submit hooks do nothing from here on.
fn detach() {
	if LINK.mode.swap(OFF, AcqRel) != LIVE {
		return;
	}
	LINK.runtime.store(0, Release);
	LINK.epoch.fetch_add(1, Release);
	if let Some(ex) = EXPORTS.get() {
		unsafe { (ex.unregister)(abi::exe_module()) };
		eprintln!("reshade: add-on unregistered");
	}
}

unsafe extern "C" fn on_destroy(runtime: *mut c_void) {
	let _ = LINK.runtime.compare_exchange(runtime as usize, 0, Release, Relaxed);
	LINK.epoch.fetch_add(1, Release);
}

/// ReShade's own depth add-on (generic_depth) binds its guess to DEPTH inside this event; ours comes later and wins.
unsafe extern "C" fn on_begin(runtime: *mut c_void, _list: *mut c_void, _rtv: u64, _rtv_srgb: u64) {
	if LINK.rendering.load(Acquire) && LINK.runtime.load(Acquire) == runtime as usize {
		let srv = LINK.depth_srv.load(Acquire);
		unsafe { Runtime(runtime).update_texture_bindings(c"DEPTH", srv, srv) };
	}
}

/// The submit hook: renders ReShade's effects on `rt` (see the module doc). Skips when the runtime changed since recording.
fn render(native: &Native, runtime: usize, epoch: u64) {
	if runtime == 0 || LINK.runtime.load(Acquire) != runtime || LINK.epoch.load(Acquire) != epoch {
		return;
	}
	let rt = Runtime(runtime as *mut c_void);
	unsafe {
		if LINK.defs_epoch.swap(epoch, Relaxed) != epoch {
			for (name, value) in DEFINITIONS {
				if rt.definition(name).as_deref() != Some(value) {
					rt.set_definition(name, value);
				}
			}
		}
		let queue = rt.command_queue();
		let list = queue.immediate_list();
		if list.is_null() {
			static ONCE: AtomicBool = AtomicBool::new(false);
			if !ONCE.swap(true, Relaxed) {
				eprintln!("reshade: the effect runtime's queue has no immediate command list; no effects");
			}
			return;
		}
		LINK.depth_srv.store(native.depth_srv, Release);
		LINK.rendering.store(true, Release);
		rt.render_effects(list, native.rtv, native.rtv_srgb);
		LINK.rendering.store(false, Release);
		queue.flush_immediate();
	}
	static FIRST: AtomicBool = AtomicBool::new(false);
	if !FIRST.swap(true, Relaxed) {
		eprintln!("reshade: first frame with effects rendered before the HUD ({:?})", backend_of(LINK.backend.load(Relaxed)));
	}
}

/// What the renderer keeps per size (STATE.md `Gpu.reshade`).
struct State {
	key: (u32, u32, wgpu::TextureFormat),
	targets: Targets,
	/// None in a dry run.
	native: Option<Arc<Native>>,
}

/// The renderer's part of the add-on (the `Gpu.reshade` field): inactive by default.
#[derive(Default)]
pub(in crate::viewer) struct Slot {
	pipes: Option<Pipes>,
	state: Option<State>,
	/// The size / format that could not be set up (logged once, retried when it changes).
	failed: Option<(u32, u32, wgpu::TextureFormat)>,
}

impl Drop for Slot {
	/// The renderer's end (after its present thread was joined: `presenter` is declared before `reshade` in `Gpu`).
	fn drop(&mut self) {
		self.state = None;
		detach();
	}
}

impl State {
	/// The hook for this frame: renders under the runtime and epoch current NOW (while recording).
	fn hook(&self) -> presenter::Hook {
		let call = self.native.clone().map(|n| (n, LINK.runtime.load(Acquire), LINK.epoch.load(Acquire)));
		Box::new(move || {
			if let Some((native, runtime, epoch)) = call {
				render(&native, runtime, epoch);
			}
		})
	}
}

impl Gpu {
	/// True when this frame takes the ReShade path. With no ReShade (the normal case) this is one atomic load.
	pub(in crate::viewer) fn reshade_begin(&mut self) -> bool {
		let mode = LINK.mode.load(Relaxed);
		let runtime = LINK.runtime.load(Acquire); // read once: `reshade_build` uses this value, never a second load that could see 0
		if mode == OFF || (mode == LIVE && runtime == 0) {
			return false;
		}
		let key = (self.config.width, self.config.height, self.config.format);
		if self.reshade.state.as_ref().is_some_and(|s| s.key == key) {
			return true;
		}
		if self.reshade.failed == Some(key) {
			return false;
		}
		self.reshade.state = None;
		match self.reshade_build(key, mode == DRY, runtime) {
			Ok(state) => {
				self.reshade.state = Some(state);
				true
			}
			Err(why) => {
				eprintln!("reshade: not usable at {}x{} {:?}: {why}", key.0, key.1, key.2);
				self.reshade.failed = Some(key);
				false
			}
		}
	}

	fn reshade_build(&mut self, key: (u32, u32, wgpu::TextureFormat), dry: bool, runtime: usize) -> Result<State, String> {
		// [PC] pipe_guard.rs: a driver that refuses the pass shaders (DX12 FXC) leaves ReShade unusable (the `failed` path below), no crash.
		if self.reshade.pipes.is_none() {
			self.reshade.pipes = crate::viewer::pipe_guard::guard(&self.device, crate::viewer::pipe_guard::Effect::Reshade, || Pipes::new(&self.device, key.2, self.depth_format));
		}
		let pipes = self.reshade.pipes.as_ref().ok_or("the driver refused the pass shaders")?;
		let targets = Targets::new(&self.device, pipes, key.0, key.1, key.2, &self.depth).ok_or("the swapchain format has no 8-bit sRGB pair")?;
		let native = match dry {
			true => None,
			false => {
				let under = Under { device: unsafe { Runtime(runtime as *mut c_void).device() } };
				Some(Arc::new(native::create(backend_of(LINK.backend.load(Relaxed)), &targets, key.2, Some(under))?))
			}
		};
		Ok(State { key, targets, native })
	}

	/// Begins the HUD pass (bind group 0 set). With `rs`: first part A and the encoder split with the ReShade hook, then a pass
	/// that starts from the picture ReShade returned; without, the old Load pass over the MSAA target. No pass may be open.
	pub(in crate::viewer) fn hud_pass<'e>(&self, enc: &'e mut wgpu::CommandEncoder, cuts: &mut presenter::Cuts, view: &wgpu::TextureView, rs: bool) -> wgpu::RenderPass<'e> {
		let on = self.reshade.state.as_ref().zip(self.reshade.pipes.as_ref()).filter(|_| rs);
		if let Some((st, pipes)) = on {
			st.targets.encode(pipes, enc, &self.msaa);
			cuts.split(&self.device, enc, st.hook());
		}
		let load = if on.is_some() { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load };
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("hud"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.msaa, resolve_target: Some(view), depth_slice: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Discard } })],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
				view: &self.depth,
				depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
				stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(super::shadowvol::STENCIL_CLEAR), store: wgpu::StoreOp::Discard }),
			}),
			timestamp_writes: crate::viewer::passtime::rw("hud"),
			occlusion_query_set: None,
			multiview_mask: None,
		});
		if let Some((st, pipes)) = on {
			st.targets.copy_back(pipes, &mut pass);
		}
		pass.set_bind_group(0, &self.ubind, &[]);
		pass
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_env_switch() {
		assert_eq!(wanted(None), Some(LIVE));
		assert_eq!(wanted(Some("1")), Some(LIVE));
		assert_eq!(wanted(Some("0")), None);
		assert_eq!(wanted(Some("dry")), Some(DRY));
	}

	#[test]
	fn the_far_plane_definition_is_far_over_near() {
		let (_, far) = DEFINITIONS[3];
		let v: f32 = far.parse().unwrap();
		assert!((v - super::projection::RETAIL_FAR / super::projection::RETAIL_NEAR).abs() < 1.0, "{v}");
		assert_eq!(&DEFINITIONS[..3].iter().map(|d| d.1).collect::<Vec<_>>()[..], ["0", "0", "0"]);
	}

	#[test]
	fn backends_round_trip() {
		for b in [wgpu::Backend::Vulkan, wgpu::Backend::Dx12] {
			assert_eq!(backend_of(backend_code(b)), b);
		}
		assert_eq!(backend_code(wgpu::Backend::Gl), 0);
	}

	/// The add-on's passes on a real adapter, as the renderer encodes them: part A, the split with a hook, the HUD pass with
	/// the copy-back (wgpu validation panics on a mismatch), and the native handles of the adapter's backend.
	/// Needs a GPU: `cargo test reshade_passes_on_adapter -- --ignored`.
	#[test]
	#[ignore]
	fn reshade_passes_on_adapter() {
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
		let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("adapter");
		let stencil32 = adapter.features().contains(wgpu::Features::DEPTH32FLOAT_STENCIL8);
		let depth_format = if stencil32 { wgpu::TextureFormat::Depth32FloatStencil8 } else { wgpu::TextureFormat::Depth24PlusStencil8 };
		let desc = wgpu::DeviceDescriptor { required_features: if stencil32 { wgpu::Features::DEPTH32FLOAT_STENCIL8 } else { wgpu::Features::empty() }, ..Default::default() };
		let (device, queue) = pollster::block_on(adapter.request_device(&desc)).expect("device");
		let (w, h, format) = (64, 48, wgpu::TextureFormat::Bgra8Unorm);
		let depth = depth_view(&device, w, h, depth_format);
		let msaa = msaa_view(&device, format, w, h);
		let pipes = Pipes::new(&device, format, depth_format);
		let targets = Targets::new(&device, &pipes, w, h, format, &depth).expect("targets");
		// D3D12 views are made by ReShade's own device: without ReShade there is none, and creating them reports that.
		match adapter.get_info().backend {
			wgpu::Backend::Dx12 => assert!(native::create(wgpu::Backend::Dx12, &targets, format, None).is_err()),
			backend => {
				let native = native::create(backend, &targets, format, None).expect("native handles");
				assert!(native.rtv != 0 && native.rtv_srgb != 0 && native.depth_srv != 0 && native.rtv != native.rtv_srgb);
			}
		}
		let out = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("out"),
			size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			view_formats: &[],
		});
		let out_view = out.create_view(&Default::default());
		let mut enc = device.create_command_encoder(&Default::default());
		let mut cuts = presenter::Cuts::new();
		// The level pass stand-in: clears the MSAA colour and the depth.
		drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("level"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &msaa, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::RED), store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
				view: &depth,
				depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
				stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0), store: wgpu::StoreOp::Discard }),
			}),
			timestamp_writes: crate::viewer::passtime::rw("reshade pre"),
			occlusion_query_set: None,
			multiview_mask: None,
		}));
		targets.encode(&pipes, &mut enc, &msaa);
		let ran = Arc::new(AtomicBool::new(false));
		let flag = ran.clone();
		cuts.split(&device, &mut enc, Box::new(move || flag.store(true, Relaxed)));
		{
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("hud"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &msaa, resolve_target: Some(&out_view), depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Discard } })],
				depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
					view: &depth,
					depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
					stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0), store: wgpu::StoreOp::Discard }),
				}),
				timestamp_writes: crate::viewer::passtime::rw("hud"),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			targets.copy_back(&pipes, &mut pass);
		}
		cuts.submit(&queue, enc.finish());
		device.poll(wgpu::PollType::wait_indefinitely()).ok();
		assert!(ran.load(Relaxed), "the hook runs between the two submits");
	}
}
