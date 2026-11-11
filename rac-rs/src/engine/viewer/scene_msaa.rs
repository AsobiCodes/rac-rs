//! [PC] The scene's sample count (lane noaa, user 2026-11-08 as a speed cut; no retail counterpart). The renderer draws the level,
//! its effects and the HUD pass into 4x MSAA targets (port extra: no PS2 aliasing). A temporal upscaler (DLSS incl. DLAA, FSR)
//! smooths the edges itself, so in mode "Off with DLSS/FSR" with an upscaler chosen at start-up the session runs every one of those
//! targets with ONE sample: measured the level pass is ~4.6 ms mean at native 1440p, MSAA ~15-30 % of it. No menu row since
//! 2026-11-08 (user: "too many options"): the mode is the pack's (`default_row`, R&C1 Off with DLSS/FSR, other games 4x); the
//! options file key `scene_msaa=4x|upscaler_off` still overrides it (no screen writes it).
//! - The count is latched for the whole run at its first use (`msaa`, called by the first target / pipeline build in
//!   gpu/setup.rs `Gpu::new`): the level pipelines (~45 big ones, pipewarm.rs) cannot be rebuilt live without a long hitch, so the
//!   mode applies at the next start. Upscaler changes stay live: an upscaler switched Off later runs at 1 sample (no edge
//!   smoothing) until the next start; one switched on later keeps 4x until the next start.
//! - ONE count for every target that was `targets::MSAA` (scene colour / depth, the HUD pass's targets with or without the
//!   upscaler slot, the shop item panel, the water mirror / bed, metal env / planar, sky capture, particle glow, shadow volumes): the
//!   HUD pass and the item panel draw with the level pipelines, so keeping them at 4x would need a second copy of every level
//!   pipeline set. The HUD is drawn at the window size after the upscaler, so its own edges (prompt model, item previews) lose
//!   MSAA in this mode; its 2D quads are axis aligned and look the same.
//! - Shaders: every module goes through devtune.rs `wgsl` -> `wgsl` here (and reshade/targets.rs): at 1 sample the
//!   multisampled depth / colour textures become single-sample ones (`to_single`: `textureLoad(t, p, 0)` = sample 0 / mip 0 in
//!   both, `textureNumSamples` -> `textureNumLevels` = 1, reshade's `SAMPLES` 1). Bind group layouts say `multisampled: msaa() > 1`.
//! - Resolves: a render pass may not resolve a single-sample target, so `att` turns a resolve into a copy queued here and recorded
//!   by `flush` right after the pass ends (callers: passes.rs drops, the self-contained passes, `Gpu::draw` before submit);
//!   `att_direct` draws straight into the resolve target for passes that clear their colour first. The copied textures carry
//!   `copy_usage()` (COPY_SRC / COPY_DST, only at 1 sample).
//! - At 4 samples nothing changes: `att` returns the attachment as before, `wgsl` the same text, `copy_usage` no flag; the frames
//!   are byte-identical to before the lane. A file without the key takes the pack's default (`default_row`, Pack
//!   `scene_msaa_default`: R&C1 Off with DLSS/FSR since 2026-11-08, other games and the unit tests 4x).
//! - RAC_SCENE_MSAA=1|4 (tests, ENV.md) wins over the mode and the upscaler.
//! - Review fixes (lane noaa): the latch needs the chosen upscaler to be able to run (DLSS: the device is NGX-ready, FSR: the
//!   FidelityFX DLL pair of the backend is next to the exe), else 4x and one log line; a later refusal is logged
//!   (`no_upscaler`). Passes that keep one picture over several passes or draw into an array layer / mip (`att_into`: water
//!   mirrors, bed, metal planar) draw straight into it at 1 sample (a copy only knows layer 0 / mip 0). The queued copies are
//!   dropped at the start of every frame (`frame_begin`). Unit tests inject the count (`with_count`; default 4).

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Options file key of the mode (absent = the pack's default, `default_row`).
pub const KEY: &str = "scene_msaa";
/// The mode's values in the file: 4x, Off with DLSS/FSR.
pub const NAMES: [&str; 2] = ["4x", "upscaler_off"];
/// The engine's default mode (0 = 4x) for a game whose pack names none; the mode without a file key is `default_row`.
pub const DEFAULT: u8 = 0;
/// Samples of the scene targets without the cut.
pub const FULL: u32 = 4;

#[cfg_attr(test, allow(dead_code))]
static LATCHED: OnceLock<u32> = OnceLock::new();
/// The window surface cannot be a copy destination (`device_made`): the HUD pass's resolve copy has nowhere to go, so 4x.
static NO_SURFACE_COPY: AtomicBool = AtomicBool::new(false);
/// `device_made` ran (the latch must come after it).
static DEVICE_SEEN: AtomicBool = AtomicBool::new(false);
/// The FidelityFX upscaler DLL pair of the device's backend is next to the exe (`device_made`).
static FSR_DLLS: AtomicBool = AtomicBool::new(false);
/// The resolves turned into copies at 1 sample (`att`), recorded by `flush`.
static PENDING: Mutex<Vec<(wgpu::Texture, wgpu::Texture)>> = Mutex::new(Vec::new());

/// The mode without a file key: the pack's `scene_msaa_default` (engine pack.rs; R&C1 Off with DLSS/FSR), 4x without a pack.
pub fn default_row() -> u8 {
	let d = crate::pack::get().scene_msaa_default;
	if (d as usize) < NAMES.len() { d } else { DEFAULT }
}

/// The mode of a file value (case and spaces ignored; missing or unknown = the default).
pub fn index(v: Option<&str>) -> u8 {
	v.and_then(|v| NAMES.iter().position(|n| n.eq_ignore_ascii_case(v.trim()))).map_or_else(default_row, |i| i as u8)
}

/// Gpu::new right after the device (after `dlss::device_made`), before the first target or pipeline: the window surface's
/// usages (without COPY_DST the run keeps 4x, one log line at the latch) and whether FSR's DLLs are there for `backend`.
pub fn device_made(usages: wgpu::TextureUsages, backend: wgpu::Backend) {
	NO_SURFACE_COPY.store(!usages.contains(wgpu::TextureUsages::COPY_DST), Ordering::Relaxed);
	let b = match backend {
		wgpu::Backend::Vulkan => Some(super::upscale::ffx::Backend::Vulkan),
		wgpu::Backend::Dx12 => Some(super::upscale::ffx::Backend::Dx12),
		_ => None,
	};
	let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
	let there = |b| {
		let (loader, upscaler) = super::upscale::ffx::dll_names(b);
		dir.as_ref().is_some_and(|d| d.join(loader).is_file() && d.join(upscaler).is_file())
	};
	FSR_DLLS.store(cfg!(windows) && b.is_some_and(there), Ordering::Relaxed);
	DEVICE_SEEN.store(true, Ordering::Relaxed);
}

/// Whether the Upscaler row value `kind` (choice.rs KINDS: 0 off, 1 FSR, 2 DLSS) can run on this device as far as is known
/// at the latch (NGX may still refuse its feature, FidelityFX its context: `no_upscaler`).
#[cfg_attr(test, allow(dead_code))]
fn runs(kind: u8) -> bool {
	match kind {
		0 => false,
		1 => FSR_DLLS.load(Ordering::Relaxed),
		_ => super::upscale::dlss::device_ready(),
	}
}

/// The upscaler slot found no upscaler running this frame (upscale/slot.rs `slot_begin`): at 1 sample, one log line.
pub fn no_upscaler() {
	static SAID: AtomicBool = AtomicBool::new(false);
	if msaa() == 1 && !SAID.swap(true, Ordering::Relaxed) {
		eprintln!("render: scene MSAA 1 sample but the chosen upscaler did not run: edges without anti-aliasing until a restart (then 4x)");
	}
}

/// Start of `Gpu::draw`: queued copies left by an earlier frame (a pass sequence that stopped early) are dropped.
pub fn frame_begin() {
	PENDING.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// No copy is queued (debug check after a flush point a caller relies on).
pub fn none_pending() -> bool {
	PENDING.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
}

/// The sample count for an env value, row value, whether a chosen upscaler can run and whether the surface takes copies (pure).
pub fn decide(env: Option<&str>, row: u8, upscaler: bool, surface_copy: bool) -> u32 {
	match env.map(str::trim) {
		_ if !surface_copy => FULL,
		Some("1") => 1,
		Some("4") => FULL,
		_ if row == 1 && upscaler => 1,
		_ => FULL,
	}
}

#[cfg(test)]
thread_local! {
	static TEST_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(FULL) };
}

/// Unit tests: `f` with the count `n` on this thread (the tests never read the options file or RAC_SCENE_MSAA; default 4).
#[cfg(test)]
pub fn with_count<R>(n: u32, f: impl FnOnce() -> R) -> R {
	let old = TEST_COUNT.with(|c| c.replace(n));
	let r = f();
	TEST_COUNT.with(|c| c.set(old));
	r
}

/// The scene's sample count for this run (1 or 4), latched at the first call.
pub fn msaa() -> u32 {
	#[cfg(test)]
	return TEST_COUNT.with(|c| c.get());
	#[cfg(not(test))]
	*LATCHED.get_or_init(latch)
}

/// The count of this run (`msaa`'s first call), after `device_made` (before it: 4x for the run, logged).
#[cfg_attr(test, allow(dead_code))]
fn latch() -> u32 {
	if !DEVICE_SEEN.load(Ordering::Relaxed) {
		debug_assert!(false, "scene_msaa::msaa() before device_made");
		eprintln!("render: scene MSAA asked before the device was made: 4x for this run");
		return FULL;
	}
	{
		let env = std::env::var("RAC_SCENE_MSAA").ok();
		// Gpu::new runs before the host's options init (choice.rs `init`): the mode and the Upscaler straight from the options file
		// (RAC_UPSCALE as slot.rs reads it), as fgen/choice.rs `file_value` does.
		let opts = crate::pcopts::PcOptions::load(&crate::pcopts::file());
		let row = index(opts.extra_get(KEY));
		let kind = match std::env::var_os("RAC_UPSCALE") {
			Some(_) => super::upscale::slot::setting().map_or(0, |s| if s.kind == super::upscale::slot::Kind::Fsr { 1 } else { 2 }),
			None => super::upscale::choice::kind_index(opts.extra_get(super::upscale::choice::KEY)),
		};
		let up = runs(kind);
		let copy = !NO_SURFACE_COPY.load(Ordering::Relaxed);
		let n = decide(env.as_deref(), row, up, copy);
		if !copy && decide(env.as_deref(), row, up, true) == 1 {
			eprintln!("render: scene MSAA stays 4x for this run: the window surface takes no copies (COPY_DST)");
		}
		if kind != 0 && !up && n == FULL && decide(env.as_deref(), row, true, copy) == 1 {
			eprintln!("render: scene MSAA stays 4x for this run: the chosen upscaler ({}) cannot run here", super::upscale::choice::KINDS[kind.min(2) as usize]);
		}
		if n != FULL || env.is_some() || row != DEFAULT || default_row() != DEFAULT {
			eprintln!("render: scene MSAA {n} sample(s) for this run (mode {}, upscaler {}, RAC_SCENE_MSAA {env:?})", NAMES[row as usize], if up { "runs" } else { "off or cannot run" });
		}
		n
	}
}

/// The WGSL text for 1 sample: multisampled textures as single-sample ones (pure; see the header).
pub fn to_single(src: &str) -> String {
	src.replace("texture_depth_multisampled_2d", "texture_depth_2d")
		.replace("texture_multisampled_2d<", "texture_2d<")
		.replace("textureNumSamples(", "textureNumLevels(")
		.replace("const SAMPLES: i32 = 4;", "const SAMPLES: i32 = 1;")
}

/// A shader module's source for this run: `src` itself at 4 samples (or without a multisampled texture), else `to_single`.
pub fn wgsl(src: Cow<'_, str>) -> Cow<'_, str> {
	if src.contains("multisampled_2d") && msaa() == 1 {
		Cow::Owned(to_single(&src))
	} else {
		src
	}
}

/// The usage a resolve source / destination needs for the copy at 1 sample (empty at 4: the descriptors stay as before).
pub fn copy_usage() -> wgpu::TextureUsages {
	if msaa() == 1 { wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST } else { wgpu::TextureUsages::empty() }
}

/// A colour attachment `view` resolving into `resolve`. A multisampled `view`: exactly that. A single-sample one: no resolve,
/// stored, and the copy `view` -> `resolve` queued for `flush` after the pass.
pub fn att<'a>(view: &'a wgpu::TextureView, resolve: Option<&'a wgpu::TextureView>, ops: wgpu::Operations<wgpu::Color>) -> wgpu::RenderPassColorAttachment<'a> {
	match resolve.filter(|_| view.texture().sample_count() == 1) {
		Some(r) => {
			PENDING.lock().unwrap_or_else(|e| e.into_inner()).push((view.texture().clone(), r.texture().clone()));
			wgpu::RenderPassColorAttachment { view, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: ops.load, store: wgpu::StoreOp::Store } }
		}
		None => wgpu::RenderPassColorAttachment { view, resolve_target: resolve, depth_slice: None, ops },
	}
}

/// For a pass that clears its colour (`ops.load` = Clear): a single-sample `view` draws straight into `resolve` (a face / layer
/// view works, no copy); a multisampled one is exactly `view` resolving into `resolve`.
pub fn att_direct<'a>(view: &'a wgpu::TextureView, resolve: &'a wgpu::TextureView, ops: wgpu::Operations<wgpu::Color>) -> wgpu::RenderPassColorAttachment<'a> {
	debug_assert!(matches!(ops.load, wgpu::LoadOp::Clear(_)), "att_direct is for passes that clear their colour");
	att_into(view, resolve, true, ops)
}

/// A pass of a picture kept in `resolve` over one or more passes (water mirror / bed, metal planar; `resolve` may be an array
/// layer or mip view): multisampled, `view` resolving into `resolve` when `resolve_now`, else `view` alone (as before); one
/// sample: straight into `resolve` with `ops.load`, stored (a later pass goes on with it).
pub fn att_into<'a>(view: &'a wgpu::TextureView, resolve: &'a wgpu::TextureView, resolve_now: bool, ops: wgpu::Operations<wgpu::Color>) -> wgpu::RenderPassColorAttachment<'a> {
	if view.texture().sample_count() == 1 {
		wgpu::RenderPassColorAttachment { view: resolve, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: ops.load, store: wgpu::StoreOp::Store } }
	} else {
		wgpu::RenderPassColorAttachment { view, resolve_target: resolve_now.then_some(resolve), depth_slice: None, ops }
	}
}

/// The size of a multisampled colour target that a 1-sample run never draws into (`att_direct` / `att_into` draw into the
/// resolve target instead): 1 x 1 then, `(w, h)` at 4x.
pub fn ms_size(w: u32, h: u32) -> (u32, u32) {
	if msaa() == 1 { (1, 1) } else { (w, h) }
}

/// Records the queued resolve copies (call right after a pass that used `att` ends). Nothing queued (4 samples): one lock.
pub fn flush(enc: &mut wgpu::CommandEncoder) {
	let list = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()));
	for (src, dst) in list {
		let ok = src.usage().contains(wgpu::TextureUsages::COPY_SRC) && dst.usage().contains(wgpu::TextureUsages::COPY_DST);
		if !ok || src.format() != dst.format() {
			static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
			if !WARNED.swap(true, Ordering::Relaxed) {
				eprintln!("render: scene MSAA 1: a resolve copy skipped ({:?} {:?} -> {:?} {:?})", src.format(), src.usage(), dst.format(), dst.usage());
			}
			continue;
		}
		let size = wgpu::Extent3d { width: src.width().min(dst.width()), height: src.height().min(dst.height()), depth_or_array_layers: 1 };
		enc.copy_texture_to_texture(src.as_image_copy(), dst.as_image_copy(), size);
	}
}

#[cfg(test)]
#[path = "scene_msaa_tests.rs"]
mod tests;
