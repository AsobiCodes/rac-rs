//! [PC] Frame generation (lane fgen, S1): our own DirectX 12 swapchain on the window (RAC_FG=null). A DXGI flip swapchain made for
//! the window's HWND on wgpu's ID3D12CommandQueue exactly as wgpu-hal makes its own (FLIP_DISCARD, latency + 1 buffers, the
//! frame-latency waitable object, ALLOW_TEARING when the factory has it, no DXGI Alt+Enter / window changes); the buffers are
//! wrapped as wgpu textures, so the frame's last encoder copies our backbuffer onto the current one and moves it to the PRESENT
//! state with wgpu's own barriers. Present on the present thread (D3D12 queues are free-threaded; acquire and present never
//! overlap: the presenter is flushed before every acquire). Present intervals and flags as wgpu-hal: Immediate (0, ALLOW_TEARING),
//! Mailbox (0), Fifo (1).
//!
//! [S2] RAC_FG=fsr (`fg`): FidelityFX's frame-generation proxy swapchain (`...ForHwndDX12`, an IDXGISwapChain4) replaces
//! `CreateSwapChainForHwnd`; everything else (buffers, ResizeBuffers, the waitable object, Present) goes through the proxy. Before a
//! resize its presents are waited for and the frame-generation context dropped; it is made again for the new size.
//!
//! [S3] RAC_FG=dlss2..4 (`sl`): the swapchain is made through Streamline's proxy of our DXGI factory (sl.rs `proxy_factory`), so
//! every call on it goes through SL's proxy swapchain. Made WITHOUT the frame-latency waitable object (SL guide 12.1: its pacer owns
//! the latency; Reflex's sleep paces our frames); DLSS-G is switched off before ResizeBuffers and before the swapchain goes.

use windows::core::Interface;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D12::ID3D12Resource;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::WaitForSingleObject;

struct Chain {
	sc: IDXGISwapChain3,
	images: Vec<wgpu::Texture>,
	waitable: HANDLE,
	extent: (u32, u32),
	count: u32,
}

pub(super) struct DxSwap {
	factory: IDXGIFactory2,
	queue: windows::Win32::Graphics::Direct3D12::ID3D12CommandQueue,
	hwnd: HWND,
	format: DXGI_FORMAT,
	tearing: bool,
	chain: Option<Chain>,
	mode: wgpu::PresentMode,
	/// A failed present: a new swapchain before the next acquire.
	stale: std::sync::Arc<std::sync::atomic::AtomicBool>,
	/// A frame dropped unpresented: its latency wait was taken and the buffer index did not move, so the next acquire skips the wait.
	held: std::sync::Arc<std::sync::atomic::AtomicBool>,
	/// [S2] RAC_FG=fsr: FidelityFX frame generation owns the swapchain.
	fg: Option<std::sync::Arc<super::ffx_fg::Fg>>,
	/// [S3] RAC_FG=dlss2..4: Streamline's proxy swapchain.
	sl: Option<&'static super::sl::Sl>,
}

/// One acquired buffer, presented on the present thread.
pub(super) struct DxFrame {
	sc: IDXGISwapChain3,
	interval: u32,
	flags: DXGI_PRESENT,
	stale: std::sync::Arc<std::sync::atomic::AtomicBool>,
	held: std::sync::Arc<std::sync::atomic::AtomicBool>,
	presented: bool,
}

// SAFETY: DXGI swapchains may be used from any thread; acquire and present never overlap (presenter flush).
unsafe impl Send for DxFrame {}
unsafe impl Send for DxSwap {}

pub(super) enum Got {
	Frame(wgpu::Texture, DxFrame),
	Skip,
	Lost,
}

/// Present interval and flags of a present mode (wgpu-hal's table; Immediate only with tearing).
pub(super) fn present_args(mode: wgpu::PresentMode, tearing: bool) -> (u32, DXGI_PRESENT) {
	match mode {
		wgpu::PresentMode::Immediate if tearing => (0, DXGI_PRESENT_ALLOW_TEARING),
		wgpu::PresentMode::Immediate | wgpu::PresentMode::Mailbox => (0, DXGI_PRESENT::default()),
		_ => (1, DXGI_PRESENT::default()),
	}
}

impl DxSwap {
	pub(super) fn new(adapter: &wgpu::Adapter, queue: &wgpu::Queue, hwnd: isize, format: wgpu::TextureFormat, fg: Option<std::sync::Arc<super::ffx_fg::Fg>>, sl: Option<&'static super::sl::Sl>) -> Result<DxSwap, String> {
		let format = match format {
			wgpu::TextureFormat::Bgra8Unorm => DXGI_FORMAT_B8G8R8A8_UNORM,
			wgpu::TextureFormat::Rgba8Unorm => DXGI_FORMAT_R8G8B8A8_UNORM,
			f => return Err(format!("format {f:?}")),
		};
		let factory: IDXGIFactory2 = {
			let a = unsafe { adapter.as_hal::<wgpu::hal::api::Dx12>() }.ok_or("not a DirectX 12 adapter")?;
			unsafe { a.as_raw().GetParent() }.map_err(|e| format!("adapter factory {e}"))?
		};
		let q = unsafe { queue.as_hal::<wgpu::hal::api::Dx12>() }.ok_or("not a DirectX 12 queue")?.as_raw().clone();
		let tearing = factory.cast::<IDXGIFactory5>().is_ok_and(|f5| {
			let mut allow = windows::core::BOOL(0);
			unsafe { f5.CheckFeatureSupport(DXGI_FEATURE_PRESENT_ALLOW_TEARING, &mut allow as *mut _ as *mut _, 4) }.is_ok() && allow.as_bool()
		});
		Ok(DxSwap { factory, queue: q, hwnd: HWND(hwnd as *mut _), format, tearing, chain: None, mode: wgpu::PresentMode::Fifo, stale: Default::default(), held: Default::default(), fg, sl })
	}

	fn flags(&self) -> DXGI_SWAP_CHAIN_FLAG {
		let mut f = if self.sl.is_some() { DXGI_SWAP_CHAIN_FLAG(0) } else { DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT };
		if self.tearing {
			f |= DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING;
		}
		f
	}

	/// The swapchain for `want` (buffers resized when there is one), `latency` frames in flight. Ok(false): no area (minimized).
	/// The caller flushed the presenter; the GPU is idled here before the buffers go.
	pub(super) fn configure(&mut self, device: &wgpu::Device, wformat: wgpu::TextureFormat, want: (u32, u32), latency: u32, mode: wgpu::PresentMode) -> Result<bool, String> {
		self.mode = mode;
		self.stale.store(false, std::sync::atomic::Ordering::Relaxed);
		self.held.store(false, std::sync::atomic::Ordering::Relaxed);
		if want.0 == 0 || want.1 == 0 {
			return Ok(false);
		}
		let count = (latency + 1).min(16);
		let old = self.chain.take();
		let sc = match old {
			Some(c) => {
				// [S2] FidelityFX: its presents done and the frame-generation context gone before the buffers change.
				if let Some(fg) = &self.fg {
					fg.wait_presents();
					fg.drop_ctx();
				}
				// [S3] Streamline: DLSS-G off before the buffers change (SL guide 12.0).
				if let Some(sl) = self.sl {
					sl.off_now();
				}
				// Every reference to the buffers must be gone before ResizeBuffers: drop the wrappers, let wgpu free them.
				drop(c.images);
				let _ = device.poll(wgpu::PollType::wait_indefinitely());
				if !c.waitable.is_invalid() {
					unsafe { CloseHandle(c.waitable) }.ok();
				}
				match unsafe { c.sc.ResizeBuffers(count, want.0, want.1, self.format, self.flags()) } {
					Ok(()) => c.sc,
					Err(e) => return Err(format!("ResizeBuffers {e}")),
				}
			}
			None => self.create(want, count)?,
		};
		let waitable = if self.sl.is_some() {
			HANDLE::default() // [S3] no waitable object: Streamline paces
		} else {
			unsafe { sc.SetMaximumFrameLatency(latency) }.map_err(|e| format!("SetMaximumFrameLatency {e}"))?;
			unsafe { sc.GetFrameLatencyWaitableObject() }
		};
		let mut images = Vec::with_capacity(count as usize);
		for i in 0..count {
			let r: ID3D12Resource = unsafe { sc.GetBuffer(i) }.map_err(|e| format!("GetBuffer {e}"))?;
			images.push(wrap(device, r, wformat, want));
		}
		let what = if self.fg.is_some() { "FidelityFX frame-generation DirectX 12 swapchain" } else if self.sl.is_some() { "Streamline (DLSS frame generation) DirectX 12 swapchain" } else { "own DirectX 12 swapchain" };
		eprintln!("fgen: {what} {}x{}, {count} buffers, {mode:?}, tearing {}", want.0, want.1, self.tearing);
		self.chain = Some(Chain { sc, images, waitable, extent: want, count });
		// A refused context: Err, the caller (present.rs) goes back to wgpu's surface for the rest of the run.
		if let Some(fg) = &self.fg {
			fg.make_ctx(want, wformat)?;
		}
		Ok(true)
	}

	fn create(&self, want: (u32, u32), count: u32) -> Result<IDXGISwapChain3, String> {
		let desc = DXGI_SWAP_CHAIN_DESC1 {
			Width: want.0,
			Height: want.1,
			Format: self.format,
			Stereo: false.into(),
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
			BufferCount: count,
			Scaling: DXGI_SCALING_STRETCH,
			SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
			AlphaMode: DXGI_ALPHA_MODE_IGNORE,
			Flags: self.flags().0 as u32,
		};
		if let Some(fg) = &self.fg {
			let raw = fg.create_swapchain_dx12(&desc as *const _ as *const _, self.hwnd.0, self.factory.as_raw(), self.queue.as_raw())?;
			// SAFETY: FidelityFX hands out one reference to its proxy IDXGISwapChain4; `from_raw` takes it over.
			let sc4 = unsafe { IDXGISwapChain4::from_raw(raw) };
			unsafe { self.factory.MakeWindowAssociation(self.hwnd, DXGI_MWA_NO_WINDOW_CHANGES | DXGI_MWA_NO_ALT_ENTER) }.ok();
			return sc4.cast::<IDXGISwapChain3>().map_err(|e| format!("IDXGISwapChain3 {e}"));
		}
		if let Some(sl) = self.sl {
			// [S3] Through Streamline's proxy factory: the swapchain it hands back is its proxy.
			let proxy = sl.proxy_factory(&self.factory)?;
			let sc1 = unsafe { proxy.CreateSwapChainForHwnd(&self.queue, self.hwnd, &desc, None, None) }.map_err(|e| format!("Streamline CreateSwapChainForHwnd {e}"))?;
			unsafe { self.factory.MakeWindowAssociation(self.hwnd, DXGI_MWA_NO_WINDOW_CHANGES | DXGI_MWA_NO_ALT_ENTER) }.ok();
			return sc1.cast::<IDXGISwapChain3>().map_err(|e| format!("IDXGISwapChain3 {e}"));
		}
		let sc1 = unsafe { self.factory.CreateSwapChainForHwnd(&self.queue, self.hwnd, &desc, None, None) }.map_err(|e| format!("CreateSwapChainForHwnd {e}"))?;
		unsafe { self.factory.MakeWindowAssociation(self.hwnd, DXGI_MWA_NO_WINDOW_CHANGES | DXGI_MWA_NO_ALT_ENTER) }.ok();
		sc1.cast::<IDXGISwapChain3>().map_err(|e| format!("IDXGISwapChain3 {e}"))
	}

	pub(super) fn needs_configure(&self) -> bool {
		self.chain.is_none() || self.stale.load(std::sync::atomic::Ordering::Relaxed)
	}

	#[allow(dead_code)] // stage S2: the vendors' swapchains read it
	pub(super) fn extent(&self) -> Option<(u32, u32)> {
		self.chain.as_ref().map(|c| c.extent)
	}

	/// The current buffer, after the frame-latency wait (1 s timeout, as a skipped frame).
	pub(super) fn acquire(&mut self) -> Got {
		let Some(c) = &self.chain else { return Got::Skip };
		if !self.held.swap(false, std::sync::atomic::Ordering::Relaxed) && !c.waitable.is_invalid() {
			match unsafe { WaitForSingleObject(c.waitable, 1000) } {
				WAIT_OBJECT_0 => {}
				windows::Win32::Foundation::WAIT_TIMEOUT => return Got::Skip,
				_ => return Got::Lost,
			}
		}
		let i = unsafe { c.sc.GetCurrentBackBufferIndex() } % c.count;
		let (interval, flags) = present_args(self.mode, self.tearing);
		Got::Frame(c.images[i as usize].clone(), DxFrame { sc: c.sc.clone(), interval, flags, stale: self.stale.clone(), held: self.held.clone(), presented: false })
	}
}

impl Drop for DxSwap {
	fn drop(&mut self) {
		// [S2] FidelityFX: presents done, the frame-generation context and then the swapchain context destroyed while our
		// references to its proxy still live; the proxy goes with them below.
		if let Some(fg) = &self.fg {
			fg.wait_presents();
			fg.drop_ctx();
			fg.destroy_swap_ctx();
		}
		if let Some(sl) = self.sl {
			sl.off_now(); // [S3] DLSS-G off before the swapchain goes
		}
		if let Some(c) = self.chain.take() {
			if !c.waitable.is_invalid() {
				unsafe { CloseHandle(c.waitable) }.ok();
			}
		}
	}
}

/// A swapchain buffer as a wgpu texture (it starts in COMMON = PRESENT: wgpu's uninitialized state on DX12).
fn wrap(device: &wgpu::Device, r: ID3D12Resource, format: wgpu::TextureFormat, extent: (u32, u32)) -> wgpu::Texture {
	use wgpu::hal;
	let size = wgpu::Extent3d { width: extent.0, height: extent.1, depth_or_array_layers: 1 };
	let raw = unsafe { hal::dx12::Device::texture_from_raw(r, format, wgpu::TextureDimension::D2, size, 1, 1) };
	let desc = wgpu::TextureDescriptor { label: Some("fgen swapchain buffer"), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] };
	unsafe { device.create_texture_from_hal::<hal::api::Dx12>(raw, &desc, wgpu::TextureUses::UNINITIALIZED) }
}

impl DxFrame {
	/// [S2] The swapchain as the IDXGISwapChain4* FidelityFX's configure names, and its present count so far.
	/// The cast's reference goes at the end of the expression; its pointer stays valid because `self.sc` holds the same object
	/// (QueryInterface keeps identity: FidelityFX's proxy is an IDXGISwapChain4).
	pub(super) fn raw(&self) -> *mut std::ffi::c_void {
		self.sc.cast::<IDXGISwapChain4>().map_or(self.sc.as_raw(), |s| s.as_raw())
	}

	pub(super) fn present_count(&self) -> u64 {
		unsafe { self.sc.GetLastPresentCount() }.map_or(0, |n| n as u64)
	}

	/// IDXGISwapChain::Present (present thread). A removed device or a failed present: logged; wgpu's lost-device report follows.
	pub(super) fn present(mut self) {
		self.presented = true;
		let hr = unsafe { self.sc.Present(self.interval, self.flags) };
		if hr.is_err() {
			eprintln!("fgen: Present {hr:?}");
			self.stale.store(true, std::sync::atomic::Ordering::Relaxed);
		}
	}
}

impl Drop for DxFrame {
	fn drop(&mut self) {
		if !self.presented {
			self.held.store(true, std::sync::atomic::Ordering::Relaxed);
		}
	}
}
