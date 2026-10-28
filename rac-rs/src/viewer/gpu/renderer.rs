//! The graphics API of the window (Pause -> Options -> Graphics -> Screen -> Renderer: Vulkan / DirectX 12; `renderer=vulkan|dx12` in
//! the player's options file, `retail/pcopts.rs`; an outside launcher writes the same key). Read before the window's `Gpu::new`, so a
//! change in the menu applies at the next start. `WGPU_BACKEND` (wgpu's own switch) still overrides the file.
//!
//! DirectX 12 compiles the shaders with Microsoft's DXC when `dxcompiler.dll` sits in the exe's folder (setup.ps1 fetches it; about 7 s
//! faster to start than the old FXC compiler that DX12 uses otherwise, same speed and picture class afterwards), else with FXC.
//! `WGPU_DX12_COMPILER` (fxc / dxc) still overrides. wgpu's own `Auto` would use any `dxcompiler.dll` on the PATH and refuse DX12 when
//! that one is too old, so the compiler is always named here.
//!
//! Vulkan frames in flight stay at wgpu's 2. DX12 uses 3: wgpu's DX12 swapchain keeps only 2 frames in flight at latency 2 (Mailbox is
//! `Present(0)` there, with a wait on the frame-latency object before each acquire) and a composed window starves the GPU (RTX 5070 Ti:
//! 187 fps against Vulkan's 232 at 1440p); latency 3 gave 238 with no tearing (scratch_dx12ab/HANDOFF.md).
use super::*;

/// The player's choice from the options file: Some(true) = DirectX 12, Some(false) = Vulkan, None = not chosen (Vulkan first, then wgpu's own order).
fn chosen() -> Option<bool> {
	crate::retail::pcopts::PcOptions::load(&crate::retail::pcopts::file()).renderer
}

/// `dxcompiler.dll` next to the exe, when it is there.
fn dxc_next_to_exe() -> Option<String> {
	let dll = std::env::current_exe().ok()?.parent()?.join("dxcompiler.dll");
	dll.is_file().then(|| dll.to_string_lossy().into_owned())
}

/// The DX12 shader compiler for an exe folder that has (Some) or has not (None) a `dxcompiler.dll`.
fn dx12_compiler(dll: Option<String>) -> wgpu::Dx12Compiler {
	match dll {
		Some(dxc_path) => wgpu::Dx12Compiler::DynamicDxc { dxc_path },
		None => wgpu::Dx12Compiler::Fxc,
	}
}

/// The backends for the file's choice: exactly that one, or wgpu's default set when nothing is chosen or `default_set` (the
/// fallback after the chosen one found no adapter).
#[cfg(not(target_os = "linux"))]
fn backends_for(choice: Option<bool>, default_set: bool) -> wgpu::Backends {
	match choice {
		Some(true) if !default_set => wgpu::Backends::DX12,
		// Not chosen = Vulkan first, like the options file's documented default. Not wgpu's whole set: its DX12 start-up asks for
		// IDXGIFactoryMedia, and ReShade 6.8.0's CreateDXGIFactory1 hook crashes on that (also when ReShade is only the Vulkan layer).
		Some(false) | None if !default_set => wgpu::Backends::VULKAN,
		_ => wgpu::Backends::default(),
	}
}

/// Linux: Vulkan is the only backend offered (the Renderer row is not shown either). A `renderer=dx12` line left in a shared options
/// file is ignored; the second try after no Vulkan adapter is wgpu's own set.
#[cfg(target_os = "linux")]
fn backends_for(choice: Option<bool>, default_set: bool) -> wgpu::Backends {
	let _ = choice;
	if default_set { wgpu::Backends::default() } else { wgpu::Backends::VULKAN }
}

/// The instance descriptor, and whether DX12 can be opened with the old FXC compiler (no `dxcompiler.dll`, no WGPU_DX12_COMPILER=dxc):
/// the compiler is named for every set that holds DX12 (also wgpu's default set of the fallback).
fn descriptor(choice: Option<bool>, default_set: bool) -> (wgpu::InstanceDescriptor, bool) {
	let mut d = wgpu::InstanceDescriptor::new_without_display_handle();
	d.backends = backends_for(choice, default_set);
	let mut d = d.with_env(); // WGPU_BACKEND, WGPU_DX12_COMPILER ... override the file
	if d.backends.contains(wgpu::Backends::DX12) && std::env::var_os("WGPU_DX12_COMPILER").is_none() {
		d.backend_options.dx12.shader_compiler = dx12_compiler(dxc_next_to_exe());
	}
	let fxc = d.backends.contains(wgpu::Backends::DX12) && matches!(d.backend_options.dx12.shader_compiler, wgpu::Dx12Compiler::Fxc);
	(d, fxc)
}

use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
/// The window thread is still inside `Gpu::new` (the FXC start-up wait): set by `fxc_notice`, cleared by `start_done`.
static FXC_WAIT: AtomicBool = AtomicBool::new(false);
/// The wait box is on screen or being created (so `start_done` closes it).
static FXC_BOX: AtomicBool = AtomicBool::new(false);
/// The box's own caption: no other box (the GPU capture note, a second rac-rs process) has it.
const FXC_CAPTION: &str = "Ratchet & Clank (rac-rs) - DirectX 12 is starting";
const FXC_LOG: &str = "gpu: DirectX 12 without dxcompiler.dll next to rac-rs.exe: the shaders compile with Windows' old FXC compiler, the start takes about 2 minutes or more";
const FXC_TEXT: &str = "DirectX 12 is starting without dxcompiler.dll next to rac-rs.exe, so the graphics shaders are compiled with Windows' old compiler. The start takes about 2 minutes or more on slower PCs (every start) and the window may say \"Not Responding\" meanwhile; the game is not frozen, please wait.

To make it fast: run Setup again (it downloads dxcompiler.dll next to rac-rs.exe), or choose Vulkan in Pause -> Options -> Graphics -> Screen -> Renderer.

This box closes by itself when the game has started.";

/// [PC] The adapter is DX12 and the compiler is FXC: the start-up compiles the water shader alone for ~80 s on the window thread, so the
/// window cannot answer. One run-log line now; if the start is still compiling after 10 s, a short box on a thread of its own
/// (`gscap_guard::message_box`, the game does not wait for it) says so and `start_done` closes it. Tests (hidden window) only log.
fn fxc_notice() {
	static ONCE: std::sync::Once = std::sync::Once::new();
	ONCE.call_once(|| {
		eprintln!("{FXC_LOG}");
		FXC_WAIT.store(true, SeqCst);
		if !crate::rtss_crash::test_mode() {
			let _ = std::thread::Builder::new().name("fxc note".into()).spawn(|| {
				std::thread::sleep(std::time::Duration::from_secs(10));
				if !FXC_WAIT.load(SeqCst) {
					return;
				}
				FXC_BOX.store(true, SeqCst);
				if !FXC_WAIT.load(SeqCst) {
					// The start finished between the two reads; `start_done` may or may not have seen the flag: nothing is shown.
					FXC_BOX.store(false, SeqCst);
					return;
				}
				eprintln!("gpu: fxc wait box shown");
				// MB_OK | MB_ICONINFORMATION | MB_TOPMOST. No MB_SETFOREGROUND: the box does not pull the focus to itself; it stays on top
				// of the unresponsive game window, and closing it changes nobody's focus.
				crate::gscap_guard::message_box(FXC_TEXT, FXC_CAPTION, 0x40 | 0x4_0000);
				FXC_BOX.store(false, SeqCst);
			});
		}
	});
}

/// `Gpu::new` is done (handler.rs `resumed`): the FXC wait box, if it has not shown yet, will not; if it is up or being created, a
/// short-lived helper thread closes it (the window thread never waits for it).
pub(crate) fn start_done() {
	FXC_WAIT.store(false, SeqCst);
	if FXC_BOX.load(SeqCst) {
		let _ = std::thread::Builder::new().name("fxc close".into()).spawn(close_box);
	}
}

/// Closes this process's own wait box (a `#32770` dialog with `FXC_CAPTION`) with WM_CLOSE; nothing else is touched. The box may still
/// be in creation, so the lookup is retried every 50 ms (at most 10 s) while the box flag is up.
#[cfg(windows)]
fn close_box() {
	#[link(name = "user32")]
	unsafe extern "system" {
		fn FindWindowExW(parent: *mut std::ffi::c_void, after: *mut std::ffi::c_void, class: *const u16, title: *const u16) -> *mut std::ffi::c_void;
		fn GetWindowThreadProcessId(hwnd: *mut std::ffi::c_void, pid: *mut u32) -> u32;
		fn PostMessageW(hwnd: *mut std::ffi::c_void, msg: u32, w: usize, l: isize) -> i32;
	}
	let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
	let (class, title) = (wide("#32770"), wide(FXC_CAPTION));
	// Up to 10 s while the box thread still says its box is up (a loaded PC can be slow to create it); every box with the caption
	// is checked, so another rac-rs starting at the same time never hides ours.
	for _ in 0..200 {
		if !FXC_BOX.load(SeqCst) {
			return;
		}
		// SAFETY: NUL-terminated UTF-16 strings that outlive the calls; the handles are only used for the id lookup and one posted message.
		unsafe {
			let mut hwnd = FindWindowExW(std::ptr::null_mut(), std::ptr::null_mut(), class.as_ptr(), title.as_ptr());
			while !hwnd.is_null() {
				let mut pid = 0u32;
				if GetWindowThreadProcessId(hwnd, &mut pid) != 0 && pid == std::process::id() {
					PostMessageW(hwnd, 0x0010, 0, 0); // WM_CLOSE
					eprintln!("gpu: fxc wait box closed (the start is done)");
					return;
				}
				hwnd = FindWindowExW(std::ptr::null_mut(), hwnd, class.as_ptr(), title.as_ptr());
			}
		}
		std::thread::sleep(std::time::Duration::from_millis(50));
	}
}

#[cfg(not(windows))]
fn close_box() {}

/// DEV: RAC_ADAPTER=<part of an adapter's name> puts the window on that adapter (A/B on a PC with two GPUs, e.g. the integrated AMD one
/// next to an NVIDIA card; no Windows graphics setting changes). None when unset or when no adapter that can present has the name.
fn named(instance: &wgpu::Instance, surface: &wgpu::Surface<'static>) -> Option<wgpu::Adapter> {
	let want = std::env::var("RAC_ADAPTER").ok().filter(|w| !w.is_empty())?.to_lowercase();
	let found = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all())).into_iter().find(|a| a.get_info().name.to_lowercase().contains(&want) && a.is_surface_supported(surface));
	if found.is_none() {
		eprintln!("gpu: RAC_ADAPTER={want}: no adapter of that name can present, using the default one");
	}
	found
}

/// Instance, surface and adapter of the window. A chosen renderer that has no adapter on this PC falls back to wgpu's default set
/// (so a stale `renderer=` line never leaves the player without a picture).
pub(super) fn open(window: &Arc<Window>) -> (wgpu::Instance, wgpu::Surface<'static>, wgpu::Adapter) {
	let choice = chosen();
	crate::rtss_crash::note_backend(choice != Some(true)); // until the adapter is known (a freeze in the instance)
	let try_open = |default_set: bool| {
		let (desc, fxc) = descriptor(choice, default_set);
		let instance = wgpu::Instance::new(desc);
		let surface = instance.create_surface(window.clone()).ok()?;
		let adapter = match named(&instance, &surface) {
			Some(a) => a,
			None => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
				power_preference: wgpu::PowerPreference::HighPerformance,
				compatible_surface: Some(&surface),
				force_fallback_adapter: false,
				apply_limit_buckets: false,
			}))
			.ok()?,
		};
		Some((instance, surface, adapter, fxc))
	};
	let found = try_open(false).or_else(|| {
		eprintln!("gpu: no {} adapter on this PC, trying the default order", if choice == Some(true) { "DirectX 12" } else { "Vulkan" });
		try_open(true)
	});
	let (instance, surface, adapter, fxc) = found.expect("no GPU adapter");
	if fxc && adapter.get_info().backend == wgpu::Backend::Dx12 {
		fxc_notice(); // only now that the adapter really is DX12 (a fallback to Vulkan never shows it)
	}
	crate::retail::pcrender::note_running(adapter.get_info().backend == wgpu::Backend::Dx12);
	crate::rtss_crash::note_backend(adapter.get_info().backend == wgpu::Backend::Vulkan);
	crate::rtss_layer::note();
	super::reshade::attach(adapter.get_info().backend);
	(instance, surface, adapter)
}

/// The frames in flight the swapchain asks for: 3 on DirectX 12 (module doc), wgpu's 2 elsewhere.
pub(super) fn frame_latency(backend: wgpu::Backend, env: Option<u32>, wgpu_default: u32) -> u32 {
	match env {
		Some(n) => n.clamp(1, 3),
		None if backend == wgpu::Backend::Dx12 => 3,
		None => wgpu_default,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_files_choice_picks_the_backends() {
		#[cfg(not(target_os = "linux"))]
		assert_eq!(backends_for(Some(true), false), wgpu::Backends::DX12);
		#[cfg(target_os = "linux")]
		assert_eq!(backends_for(Some(true), false), wgpu::Backends::VULKAN, "Linux has no DirectX 12 backend");
		assert_eq!(backends_for(Some(false), false), wgpu::Backends::VULKAN);
		assert_eq!(backends_for(None, false), wgpu::Backends::VULKAN, "not chosen: Vulkan first, never the DX12 start-up (ReShade crash)");
		assert_eq!(backends_for(Some(true), true), wgpu::Backends::default(), "the fallback is wgpu's own order");
		assert!(wgpu::Backends::default().contains(wgpu::Backends::VULKAN), "Vulkan stays in today's default set");
	}

	#[test]
	fn dx12_uses_dxc_only_when_the_dll_is_there() {
		assert!(matches!(dx12_compiler(None), wgpu::Dx12Compiler::Fxc));
		assert!(matches!(dx12_compiler(Some("C:/x/dxcompiler.dll".into())), wgpu::Dx12Compiler::DynamicDxc { dxc_path } if dxc_path == "C:/x/dxcompiler.dll"));
	}

	#[test]
	fn frame_latency_is_three_on_dx12_unless_the_knob_is_set() {
		use wgpu::Backend::{Dx12, Vulkan};
		assert_eq!(frame_latency(Dx12, None, 2), 3);
		assert_eq!(frame_latency(Vulkan, None, 2), 2, "Vulkan unchanged");
		assert_eq!(frame_latency(Dx12, Some(2), 2), 2, "RAC_FRAME_LATENCY wins");
		assert_eq!(frame_latency(Vulkan, Some(9), 2), 3);
		assert_eq!(frame_latency(Vulkan, Some(0), 2), 1);
	}
}
