//! [PC] The per-frame DLSS upscale (lane dlss2, upscaler stage 4; no retail counterpart): NVIDIA DLSS 4.5 through NGX (dlss.rs) in
//! the upscaler slot (slot.rs), in the place FSR 3's dispatch has (ffx_frame.rs; same inputs, same command-buffer rules):
//! - The feature is made for exact (render, window, quality) values when the player picks DLSS or a size / quality changes
//!   (`prepare`; the caller made the GPU idle when an old one goes), never per frame: its init work runs in a command buffer / list
//!   of its own and is waited for (a model's first create on Vulkan can take ~1.5 s once per PC: the driver compiles it).
//! - Per frame: wgpu transitions of the inputs to RESOURCE and of the output (the slot's window-size `mid`) to STORAGE_READ_WRITE,
//!   then the evaluate: Vulkan in a raw-only wgpu command buffer between the frame's wgpu parts (presenter.rs `Cuts::raw`), DX12 in
//!   our own lists between the two halves of the frame's submit (`Cuts::split`, ffx_dx12.rs `Lists`). NGX leaves the states as
//!   given, so wgpu's tracking stays right. Then slot_gpu.rs `copy` only copies (DLSS sharpens nothing, no second sharpening).
//! - A failed create or evaluate switches DLSS off for the run (`failed`, one log line, `label(None)`): the DLSS choice runs Off
//!   from then on (the frame in between gets slot_gpu.rs `scale`, a plain bilinear stretch).

use super::dlss::{self, Backend, Feature, Runtime};
use super::ffx_frame::{frame_ms, needs_reset, FrameIn, Sizes};
use rac_ngx_abi as abi;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// What a feature is made for: (render, window) sizes, the NGX quality mode and the NGX preset (a DLSS preset row change = a new
/// key: the feature is made again).
pub type Key = (Sizes, u32, u32);

/// The key of a frame at these sizes and window / render ratio, with DLSS preset row value `row` (dlss_preset.rs; the caller
/// passes the value latched once per frame, slot.rs `dlss_key`).
pub(super) fn key_of(sizes: Sizes, ratio: f32, row: u8) -> Key {
	let q = dlss::ngx_quality(ratio);
	(sizes, q, super::dlss_preset::preset_of(row, q))
}

/// The render-size inputs and the window-size output of one frame, as views (Vulkan wants the image views too).
pub(super) struct Views<'a> {
	pub color: &'a wgpu::TextureView,
	pub depth: &'a wgpu::TextureView,
	pub motion: &'a wgpu::TextureView,
	pub output: &'a wgpu::TextureView,
}

/// The Upscaler row's DLSS value text (ui/live.rs): the reported model ("DLSS 4.5"), "DLSS" until a feature reported one; None =
/// no DLSS on this PC / run: the value is withdrawn (choice.rs `set_runs`) and the quality row's 100 % value is "Native AA" again.
pub(super) fn label(text: Option<String>) {
	let Some(text) = text else {
		if !repeat(&LAST_LABEL, super::choice::label_gen(), NO_DLSS) {
			crate::ui::live::clear(super::choice::KEY_Q, 0);
			crate::ui::live::clear(super::choice::KEY, super::choice::DLSS_VALUE);
		}
		super::choice::set_runs(super::choice::DLSS_VALUE, false); // [PC] lane nfsr1: not offered, a DLSS choice is Off
		return;
	};
	super::choice::set_runs(super::choice::DLSS_VALUE, true);
	LAST_LABEL.store(0, Ordering::Relaxed);
	crate::ui::live::set(super::choice::KEY, super::choice::DLSS_VALUE, text);
}

/// The fixed texts `label` / `label_now` set last, as `choice::label_gen() << 2 | code` (0: a reported model since): the same
/// text again (every frame with DLSS chosen and no runtime, every frame the menu shows) skips the Mutex and String work until
/// the quality row's labels change (review R21 m5).
static LAST_LABEL: AtomicU64 = AtomicU64::new(0);
const NO_DLSS: u64 = 1;
const PLACEHOLDER: u64 = 2;

/// True when `code` at `gen` is what `last` holds already (and records it).
pub(super) fn repeat(last: &AtomicU64, generation: u32, code: u64) -> bool {
	let k = (generation as u64) << 2 | code;
	last.swap(k, Ordering::Relaxed) == k
}

/// The menu shows the Upscaler row: name the DLSS value from what the window's device got (no NGX call).
pub(super) fn label_now() {
	match dlss::probed() {
		Some(true) => {} // NGX runs: a feature's report (`DlssGpu::prepare`) or a refusal (`fail`) names it
		Some(false) => label(None),
		None if !dlss::device_ready() => label(None),
		None => {
			if !repeat(&LAST_LABEL, super::choice::label_gen(), PLACEHOLDER) {
				crate::ui::live::set(super::choice::KEY, super::choice::DLSS_VALUE, String::from("DLSS"));
			}
		}
	}
}

fn jitter_sign() -> f32 {
	static SIGN: OnceLock<f32> = OnceLock::new();
	*SIGN.get_or_init(|| dlss::jitter_sign(std::env::var("RAC_DLSS_JITTER").ok().as_deref()))
}

/// The DLSS side of the slot: the runtime, the feature and the failure flag.
pub(in crate::viewer) struct DlssGpu {
	rt: &'static Runtime,
	/// For the GPU wait before the feature goes (`Drop`).
	device: wgpu::Device,
	feat: Option<(Key, Arc<Mutex<Feature>>)>,
	/// The feature was made since the last evaluate (reset).
	fresh: bool,
	failed: Arc<AtomicBool>,
	last: Option<u32>,
	clock: Option<Instant>,
	queue: wgpu::Queue,
	#[cfg(windows)]
	lists: Option<Arc<Mutex<super::ffx_dx12::Lists>>>,
}

impl DlssGpu {
	fn new(rt: &'static Runtime, device: &wgpu::Device, queue: &wgpu::Queue) -> DlssGpu {
		DlssGpu { rt, device: device.clone(), feat: None, fresh: false, failed: Arc::new(AtomicBool::new(false)), last: None, clock: None, queue: queue.clone(), #[cfg(windows)] lists: None }
	}

	pub(super) fn usable(&self) -> bool {
		!self.failed.load(Ordering::Relaxed)
	}

	/// Whether a feature exists that a frame with `want` (None: no DLSS this frame) would drop: the caller makes the GPU idle first.
	/// A refused run's feature goes too (its VRAM is not held for the run).
	pub(super) fn needs_idle(&self, want: Option<Key>) -> bool {
		self.feat.as_ref().is_some_and(|(k, _)| want != Some(*k) || !self.usable())
	}

	/// Drops the feature (the GPU must be idle: `needs_idle`).
	pub(super) fn drop_feat(&mut self) {
		self.feat = None;
	}

	fn fail(&self, why: &str) {
		if !self.failed.swap(true, Ordering::Relaxed) {
			eprintln!("upscale: DLSS refused ({why}): no upscaler instead");
			label(None);
		}
	}

	/// The feature for `key` (made when missing or for other values; the old one must be idle). False = refused (logged, DLSS off).
	fn prepare(&mut self, device: &wgpu::Device, key: Key, row: u8) -> bool {
		if !self.usable() {
			self.feat = None; // the caller made the GPU idle (`needs_idle` is true for a refused feature)
			return false;
		}
		if self.feat.as_ref().is_some_and(|(k, _)| *k == key) {
			return true;
		}
		self.feat = None;
		let cr = dlss::create_desc(key.0, key.1, key.2);
		let t0 = Instant::now();
		let made = match self.rt.backend {
			Backend::Vulkan => self.create_vk(device, &cr),
			Backend::Dx12 => self.create_dx12(&cr),
		};
		match made {
			Ok(f) => {
				let q = abi::QUALITY_NAMES.iter().find(|n| n.0 == key.1).map_or("?", |n| n.1);
				let name = dlss::label_of(f.preset);
				let preset = char::from_u32(f.preset).filter(|_| f.preset != 0).unwrap_or('?');
				let row = super::dlss_preset::log_name(row);
				eprintln!("upscale: {name} (preset {preset}, NGX, {:?}), {q}, render {}x{} -> {}x{}, create {} ms, DLSS preset {row} (asked {})", self.rt.backend, key.0 .0 .0, key.0 .0 .1, key.0 .1 .0, key.0 .1 .1, t0.elapsed().as_millis(), abi::preset_letter(key.2));
				label(Some(name));
				self.feat = Some((key, Arc::new(Mutex::new(f))));
				self.fresh = true;
				true
			}
			Err(e) => {
				self.fail(&e);
				false
			}
		}
	}

	/// Vulkan: the create's init work in a raw-only command buffer, submitted and waited for. On the raw queue without a
	/// `GateMark` (presenter.rs; review R21 m6): the commands touch only NGX's own new resources, never a frame's targets, so their
	/// order against the deferred submit of the last frame does not matter, and the caller made the GPU idle before a re-create.
	#[cfg(not(target_vendor = "apple"))]
	fn create_vk(&self, device: &wgpu::Device, cr: &abi::RacNgxCreate) -> Result<Feature, String> {
		let mut raw = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("dlss create") });
		let rt = self.rt;
		let made = unsafe {
			raw.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|e| {
				let e = e.ok_or_else(|| String::from("no Vulkan encoder"))?;
				let cmd = std::mem::transmute_copy::<_, u64>(&e.raw_handle()) as usize as *mut c_void;
				Feature::create(rt, cmd, cr)
			})
		};
		self.queue.submit([raw.finish()]);
		let _ = device.poll(wgpu::PollType::wait_indefinitely());
		made
	}

	#[cfg(target_vendor = "apple")]
	fn create_vk(&self, _: &wgpu::Device, _: &abi::RacNgxCreate) -> Result<Feature, String> {
		Err(String::from("no Vulkan"))
	}

	/// DX12: the create's init work in one of our lists, executed on wgpu's queue and waited for.
	#[cfg(windows)]
	fn create_dx12(&mut self, cr: &abi::RacNgxCreate) -> Result<Feature, String> {
		let lists = self.lists()?;
		let mut l = lists.lock().map_err(|_| String::from("lists lock"))?;
		let mut made = None;
		let rt = self.rt;
		l.run(
			|list| {
				made = Some(unsafe { Feature::create(rt, list, cr) }?);
				Ok(())
			},
			Vec::new(),
		)?;
		l.wait_all()?;
		made.ok_or_else(|| String::from("no feature"))
	}

	#[cfg(not(windows))]
	fn create_dx12(&mut self, _: &abi::RacNgxCreate) -> Result<Feature, String> {
		Err(String::from("D3D12 only on Windows"))
	}

	#[cfg(windows)]
	fn lists(&mut self) -> Result<Arc<Mutex<super::ffx_dx12::Lists>>, String> {
		if self.lists.is_none() {
			self.lists = Some(Arc::new(Mutex::new(super::ffx_dx12::Lists::new(self.rt.device, &self.queue)?)));
		}
		Ok(self.lists.clone().expect("made above"))
	}

	/// This frame's evaluate values.
	fn eval(&mut self, render: (u32, u32), f: &FrameIn) -> abi::RacNgxEval {
		let now = Instant::now();
		let wall = self.clock.replace(now).map(|t| (now - t).as_secs_f32() * 1000.0);
		let clock = crate::viewer::test_clock::clock_on().then(|| (crate::field::field_dt(), crate::viewer::test_clock::sub()));
		let reset = needs_reset(f.cut, std::mem::take(&mut self.fresh), self.last, f.frame);
		self.last = Some(f.frame);
		dlss::eval_desc(render, f.jitter, jitter_sign(), reset, frame_ms(clock, wall))
	}

	/// The upscale of this frame from `v` into `v.output` (after the wgpu commands so far in `enc`). False = not recorded (no
	/// feature, refused now, or the frame's submit hook is taken): the caller draws slot_gpu.rs `scale` (plain bilinear) instead.
	pub(super) fn dispatch(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, cuts: &mut crate::viewer::presenter::Cuts, v: &Views<'_>, f: &FrameIn) -> bool {
		let Some((key, feat)) = self.feat.as_ref().filter(|_| self.usable()).map(|(k, c)| (*k, c.clone())) else { return false };
		if self.rt.backend == Backend::Dx12 && cuts.has_hook() {
			return false;
		}
		let read = wgpu::TextureUses::RESOURCE;
		let list = [(v.color.texture(), read), (v.depth.texture(), read), (v.motion.texture(), read), (v.output.texture(), wgpu::TextureUses::STORAGE_READ_WRITE)];
		enc.transition_resources(std::iter::empty(), list.iter().map(|&(texture, state)| wgpu::TextureTransition { texture, selector: None, state }));
		let eval = self.eval(key.0 .0, f);
		let r = match self.rt.backend {
			Backend::Vulkan => record_vk(device, enc, cuts, v, &eval, &feat),
			Backend::Dx12 => self.record_dx12(device, enc, cuts, v, eval, feat),
		};
		match r {
			Ok(()) => {
				if self.rt.backend == Backend::Dx12 {
					// As FSR 3 (review R16 F3): the same states again after the split, so wgpu keeps the textures until the second half ran.
					enc.transition_resources(std::iter::empty(), list.iter().map(|&(texture, state)| wgpu::TextureTransition { texture, selector: None, state }));
				}
				true
			}
			Err(e) => {
				self.fail(&e);
				false
			}
		}
	}

	/// DX12: the evaluate in our own list, executed between the two halves of the frame's submit.
	#[cfg(windows)]
	fn record_dx12(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, cuts: &mut crate::viewer::presenter::Cuts, v: &Views<'_>, eval: abi::RacNgxEval, feat: Arc<Mutex<Feature>>) -> Result<(), String> {
		use windows::core::Interface;
		let lists = self.lists()?;
		let res = |t: &wgpu::TextureView| -> Result<windows::Win32::Graphics::Direct3D12::ID3D12Resource, String> {
			let h = unsafe { t.texture().as_hal::<wgpu::hal::api::Dx12>() }.ok_or("no D3D12 resource")?;
			Ok(unsafe { h.raw_resource() }.clone())
		};
		let keep = vec![res(v.color)?, res(v.output)?, res(v.depth)?, res(v.motion)?];
		let job = Dx12Job { inputs: abi::RacNgxDx12Inputs { color: keep[0].as_raw(), output: keep[1].as_raw(), depth: keep[2].as_raw(), motion: keep[3].as_raw() }, eval, keep };
		let failed = self.failed.clone();
		cuts.split(
			device,
			enc,
			Box::new(move || {
				let job = job; // the whole job (Send), not its fields
				let Dx12Job { inputs, eval, keep } = job;
				let record = |list: *mut c_void| feat.lock().map_err(|_| String::from("feature lock")).and_then(|mut f| unsafe { f.eval_dx12(list, &inputs, &eval) });
				let r = lists.lock().map_err(|_| String::from("lists lock")).and_then(|mut l| l.run(record, keep));
				if let Err(e) = r {
					if !failed.swap(true, Ordering::Relaxed) {
						eprintln!("upscale: DLSS refused ({e}): no upscaler instead");
						label(None);
					}
				}
			}),
		);
		Ok(())
	}

	#[cfg(not(windows))]
	fn record_dx12(&mut self, _: &wgpu::Device, _: &mut wgpu::CommandEncoder, _: &mut crate::viewer::presenter::Cuts, _: &Views<'_>, _: abi::RacNgxEval, _: Arc<Mutex<Feature>>) -> Result<(), String> {
		Err(String::from("D3D12 only on Windows"))
	}
}

/// The GPU finishes with the feature before NGX releases it (shutdown, a Gpu rebuild; review R16 F4 for DLSS). The NGX context
/// (dlss.rs `Runtime`) lives until the window's Gpu drops: upscale.rs `Drop for Gpu` releases this first, then shuts NGX down.
impl Drop for DlssGpu {
	fn drop(&mut self) {
		if self.feat.is_some() {
			let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
			self.feat = None;
		}
	}
}

/// One frame's DX12 evaluate for the submit hook: the inputs (pointers of `keep`) and values.
#[cfg(windows)]
struct Dx12Job {
	inputs: abi::RacNgxDx12Inputs,
	eval: abi::RacNgxEval,
	keep: Vec<windows::Win32::Graphics::Direct3D12::ID3D12Resource>,
}

// The input pointers are the resources in `keep`, moved to the submitting thread with them and used only there.
#[cfg(windows)]
unsafe impl Send for Dx12Job {}

/// Vulkan: the evaluate in a raw-only wgpu command buffer between the frame's wgpu parts.
#[cfg(not(target_vendor = "apple"))]
fn record_vk(device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, cuts: &mut crate::viewer::presenter::Cuts, v: &Views<'_>, eval: &abi::RacNgxEval, feat: &Arc<Mutex<Feature>>) -> Result<(), String> {
	let img = |t: &wgpu::TextureView| -> Result<abi::RacNgxImageVk, String> {
		let view = unsafe { t.as_hal::<wgpu::hal::api::Vulkan>() }.ok_or("no Vulkan view")?;
		let tex = t.texture();
		let image = unsafe { tex.as_hal::<wgpu::hal::api::Vulkan>() }.ok_or("no Vulkan image")?;
		let vk_format = dlss::vk_format(tex.format());
		if vk_format == 0 {
			return Err(format!("DLSS input format {:?}", tex.format()));
		}
		Ok(abi::RacNgxImageVk {
			view: unsafe { std::mem::transmute_copy::<_, u64>(&view.raw_handle()) },
			image: unsafe { std::mem::transmute_copy::<_, u64>(&image.raw_handle()) },
			vk_format,
			aspect: 1,
			width: tex.width(),
			height: tex.height(),
		})
	};
	let inputs = abi::RacNgxVkInputs { color: img(v.color)?, output: img(v.output)?, depth: img(v.depth)?, motion: img(v.motion)? };
	let mut raw = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("dlss upscale") });
	let r = unsafe {
		raw.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|e| {
			let e = e.ok_or_else(|| String::from("no Vulkan encoder"))?;
			let cmd = std::mem::transmute_copy::<_, u64>(&e.raw_handle()) as usize as *mut c_void;
			feat.lock().map_err(|_| String::from("feature lock"))?.eval_vk(cmd, &inputs, eval)
		})
	};
	r?;
	cuts.raw(device, enc, raw.finish());
	Ok(())
}

#[cfg(target_vendor = "apple")]
fn record_vk(_: &wgpu::Device, _: &mut wgpu::CommandEncoder, _: &mut crate::viewer::presenter::Cuts, _: &Views<'_>, _: &abi::RacNgxEval, _: &Arc<Mutex<Feature>>) -> Result<(), String> {
	Err(String::from("no Vulkan"))
}

/// The DLSS side for `key`: made on first use (stays None without a runtime: DLSS withdrawn, `label(None)`), its feature ready.
/// `row` = the frame's latched DLSS preset row value (`key` was made from it), named in the create log line.
pub(super) fn ready(dlss: &mut Option<DlssGpu>, device: &wgpu::Device, queue: &crate::viewer::presenter::GatedQueue, key: Key, row: u8) -> bool {
	if dlss.is_none() {
		let Some(rt) = dlss::runtime(device) else {
			label(None);
			return false;
		};
		*dlss = Some(DlssGpu::new(rt, device, &queue.raw()));
	}
	dlss.as_mut().is_some_and(|d| d.prepare(device, key, row))
}
