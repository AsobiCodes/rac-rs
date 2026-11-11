//! [PC] The upscaler slot (lane ndlss, stage 2; no retail counterpart, the game never sees it). With the dev env `RAC_UPSCALE`
//! (ENV.md) the scene is drawn at a lower internal size (`config`, as the render scale does: rscale.rs `planned_size`) and at
//! the ReShade split point (right before the HUD pass, upscale.rs `Prep::encode`) the upscaler turns the resolved scene into a
//! window-size picture in a window-size MSAA target, with the scene depth upsampled (nearest) into a window-size depth target.
//! The HUD pass (reshade.rs `hud_pass`) loads those and draws the HUD, the retail overlay and the item previews at the window
//! size (`Prep::hud_rect` scales their pixel viewports). Upscalers: FSR 3.1 / 4 through the FidelityFX DLLs (ffx_frame.rs) and
//! DLSS 4.5 (dlss_frame.rs); the slot's own passes are slot_gpu.rs. One that cannot run on this PC (DLLs missing, the GPU or the
//! runtime refuses) is no upscaler: `active` = None, the plain picture, and the menu stops offering it (choice.rs `set_runs`).
//! FSR 1 was removed in lane nfsr1. Unset: the player's choice (choice.rs, the Upscaler rows of the modern menu; default Off:
//! nothing here runs, no target, size or pass changes).

use super::{slot_gpu, Prep};
use std::sync::OnceLock;

/// Which upscaler runs in the slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	/// AMD FSR through the FidelityFX API DLLs (ffx.rs, ffx_frame.rs: the newest version the runtime offers, 4.x where the GPU
	/// runs it, else 3.1); no upscaler when the DLLs are missing or the runtime refuses (one log line).
	Fsr,
	/// NVIDIA DLSS 4.5 through NGX (dlss.rs, dlss_frame.rs; render size NGX's optimal one); no upscaler without its DLLs, on
	/// another GPU or when NGX refuses (one log line).
	Dlss,
}

/// One `RAC_UPSCALE` value: the upscaler, the window / render ratio per side and FidelityFX's sharpening (in stops: 0 =
/// sharpest, 2 = none; ffx_frame.rs `sharpness_of_stops`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
	pub kind: Kind,
	pub ratio: f32,
	pub sharp: f32,
}

/// The quality names and their window / render ratios per side (the table FSR 2+ and DLSS share).
pub const QUALITY: [(&str, f32); 5] = [("native", 1.0), ("quality", 1.5), ("balanced", 1.7), ("performance", 2.0), ("ultra", 3.0)];
/// The ratio when no quality is named (Quality).
pub const DEFAULT_RATIO: f32 = 1.5;
/// The usual sharpening in stops (FidelityFX sharpness 0.9).
pub const SHARP: f32 = 0.2;

/// `kind[:quality|percent[:sharpness]]`, e.g. `fsr`, `fsr:performance`, `fsr:60`, `fsr:quality:0.5`, `dlss:native`; None for
/// anything else (unset, 0, off, a typo): the slot stays off. `fsr1` (FSR 1, removed in lane nfsr1) = `fsr`, as in the
/// options file (choice.rs).
pub fn parse(v: Option<&str>) -> Option<Setting> {
	let v = v?.trim().to_ascii_lowercase();
	let mut it = v.split(':').map(str::trim);
	let kind = match it.next()? {
		"fsr" | "fsr1" => Kind::Fsr,
		"dlss" => Kind::Dlss,
		_ => return None,
	};
	let ratio = match it.next() {
		None | Some("") => DEFAULT_RATIO,
		Some(q) => match QUALITY.iter().find(|(n, _)| *n == q) {
			Some(&(_, r)) => r,
			None => 100.0 / q.trim_end_matches('%').parse::<f32>().ok().filter(|p| (25.0..=100.0).contains(p))?,
		},
	};
	let sharp = it.next().and_then(|s| s.parse::<f32>().ok()).filter(|s| s.is_finite()).unwrap_or(SHARP).clamp(0.0, 2.0);
	Some(Setting { kind, ratio, sharp })
}

/// `RAC_UPSCALE` when it is set (read once; a value that does not parse = off), else the player's choice (choice.rs: the
/// Upscaler / Upscale quality rows, read every call so a change applies at the next frame).
pub fn setting() -> Option<Setting> {
	static ENV: OnceLock<Option<Option<Setting>>> = OnceLock::new();
	ENV.get_or_init(|| std::env::var("RAC_UPSCALE").ok().map(|v| parse(Some(&v)))).unwrap_or_else(super::choice::setting)
}

/// The setting in force: `setting`, unless the driver refused the slot's shaders (pipe_guard.rs `Effect::Upscale`) or its
/// upscaler cannot run on this PC (choice.rs `runs`: no DLLs, the GPU or the runtime refused): then none, and the scene is
/// planned at the window size again (render scale as chosen), no crash.
pub fn active() -> Option<Setting> {
	active_of(setting(), crate::viewer::pipe_guard::off(crate::viewer::pipe_guard::Effect::Upscale))
}

/// `active` for a setting and whether the slot was refused.
pub fn active_of(s: Option<Setting>, refused: bool) -> Option<Setting> {
	s.filter(|s| !refused && super::choice::runs(s.kind))
}

impl Setting {
	/// The internal render size for a window size: each side / ratio, rounded, between 1 px and `max` (the device limit). DLSS:
	/// NGX's optimal size for the quality mode once NGX runs (dlss.rs `optimal_size`; upscale.rs `upscale_build` starts it first).
	pub fn render_size(&self, win: (u32, u32), max: u32) -> (u32, u32) {
		if let Some((w, h)) = Some(self.kind).filter(|k| *k == Kind::Dlss).and_then(|_| super::dlss::optimal_size(win, self.ratio)) {
			return (w.clamp(1, max.max(1)), h.clamp(1, max.max(1)));
		}
		let side = |v: u32| ((v as f64 / self.ratio as f64).round() as u32).clamp(1, max.max(1));
		(side(win.0), side(win.1))
	}
}

/// A HUD pixel rectangle (x, y, w, h at the render size) at the window size: scaled by `k` (window / render per axis), edges
/// rounded and kept inside the `win` picture.
pub fn scale_rect(r: [f32; 4], k: [f32; 2], win: (u32, u32)) -> [f32; 4] {
	let (ww, wh) = (win.0 as f32, win.1 as f32);
	let x0 = (r[0] * k[0]).round().clamp(0.0, ww);
	let y0 = (r[1] * k[1]).round().clamp(0.0, wh);
	let x1 = ((r[0] + r[2]) * k[0]).round().clamp(x0, ww);
	let y1 = ((r[1] + r[3]) * k[1]).round().clamp(y0, wh);
	[x0, y0, x1 - x0, y1 - y0]
}

impl Prep {
	/// The slot's pipelines, built once under the pipe guard (gpu/rscale.rs `rscale_plan` calls this before it plans the sizes,
	/// so a refusal plans the window size at once): a driver that refuses the slot's shaders switches the upscaler off for the run
	/// (`active` = None, one log line) instead of a crash. `formats` = (colour, depth).
	pub(in crate::viewer) fn slot_build(&mut self, device: &wgpu::Device, formats: (wgpu::TextureFormat, wgpu::TextureFormat)) {
		if self.slot.is_none() {
			self.slot = crate::viewer::pipe_guard::guard(device, crate::viewer::pipe_guard::Effect::Upscale, || slot_gpu::SlotGpu::new(device, formats.0, formats.1));
		}
	}

	/// No upscaler chosen: the slot's targets and pipelines are freed (built again when one is chosen).
	pub(in crate::viewer) fn slot_drop(&mut self) {
		self.slot = None;
		self.slot_on = false;
		self.ffx_on = false;
		self.dlss_on = false;
		if let Some(f) = self.ffx.as_mut() {
			f.drop_ctx(); // the caller made the GPU idle (upscale.rs `upscale_build`)
		}
		if let Some(d) = self.dlss.as_mut() {
			d.drop_feat();
		}
	}

	/// Start of a frame: whether the slot runs (`planned`: the setting the sizes were planned with, None in the Classic half of a
	/// split view), its targets at `sizes` = (render, window) and its constants. `depth` = the level's MSAA depth; `formats` =
	/// (colour, depth). Never `active` here: the option can change after the sizes were planned (gpu/rscale.rs `planned`).
	pub(in crate::viewer) fn slot_begin(&mut self, device: &wgpu::Device, queue: &crate::viewer::presenter::GatedQueue, depth: &wgpu::TextureView, formats: (wgpu::TextureFormat, wgpu::TextureFormat), sizes: ((u32, u32), (u32, u32)), planned: Option<Setting>) {
		let s = planned;
		self.slot_on = s.is_some();
		self.ffx_on = false;
		self.dlss_on = false;
		let Some(s) = s else { return };
		self.slot_build(device, formats);
		let Some(gpu) = self.slot.as_mut() else {
			self.slot_on = false; // refused just now (pipe_guard): rscale_sync plans the window size from the next frame
			return;
		};
		// [PC] FSR 3.1 / 4 (ffx_frame.rs): the runtime once (FidelityFX loader next to the exe, else no upscaler, logged), the
		// context for these sizes (the caller made the GPU idle if an old one goes: `ffx_needs_idle`).
		if s.kind == Kind::Fsr {
			self.ffx_on = ffx_ready(&mut self.ffx, device, queue, sizes);
		} else if let Some(f) = self.ffx.as_mut() {
			f.drop_ctx();
		}
		// [PC] DLSS (dlss_frame.rs): NGX once on the window's device (else no upscaler, logged), the feature for these sizes and mode.
		if s.kind == Kind::Dlss {
			let key = super::dlss_frame::key_of(sizes, s.ratio, self.dlss_row); // = `dlss_key` (the borrow of `slot` is live here)
			self.dlss_on = super::dlss_frame::ready(&mut self.dlss, device, queue, key, self.dlss_row);
		} else if let Some(d) = self.dlss.as_mut() {
			d.drop_feat();
		}
		if !(self.ffx_on || self.dlss_on) {
			crate::viewer::scene_msaa::no_upscaler(); // lane noaa: a 1-sample run whose upscaler refused (one log line)
		}
		gpu.prepare(device, queue, depth, sizes);
		self.sharp = s.sharp;
	}

	/// Whether this frame drops a FidelityFX context or a DLSS feature (no FSR 3.1 / 4 / DLSS in `want`, or other sizes / mode):
	/// the GPU must be idle first.
	pub(in crate::viewer) fn upscaler_needs_idle(&self, want: Option<Setting>, sizes: ((u32, u32), (u32, u32))) -> bool {
		let ffx = want.filter(|s| s.kind == Kind::Fsr).map(|_| sizes);
		let dlss = want.filter(|s| s.kind == Kind::Dlss).map(|s| self.dlss_key(sizes, s.ratio));
		self.ffx.as_ref().is_some_and(|f| f.needs_idle(ffx)) || self.dlss.as_ref().is_some_and(|d| d.needs_idle(dlss))
	}

	/// The DLSS feature key of this frame at `sizes` and window / render `ratio`, with the preset latched in `upscale_begin`
	/// (`dlss_row`), never a fresh read: the idle check and `slot_begin` must see the same key (review REVIEW3 F1).
	pub(super) fn dlss_key(&self, sizes: ((u32, u32), (u32, u32)), ratio: f32) -> super::dlss_frame::Key {
		super::dlss_frame::key_of(sizes, ratio, self.dlss_row)
	}

	/// This frame runs FSR 3.1 / 4 (FidelityFX) or DLSS in the slot, so it needs the temporal inputs (jitter, motion vectors, depth).
	pub(in crate::viewer) fn temporal_on(&self) -> bool {
		(self.ffx_on || self.dlss_on) && self.slot_on()
	}

	/// This frame runs the upscaler slot.
	pub(in crate::viewer) fn slot_on(&self) -> bool {
		self.slot_on && self.slot.is_some()
	}

	fn slot(&self) -> Option<&slot_gpu::SlotGpu> {
		self.slot.as_ref().filter(|_| self.slot_on)
	}

	/// The render-size picture the level pass resolves into while the slot runs (in Classic the frame's `view` is the window's).
	pub(in crate::viewer) fn scene_view(&self) -> Option<&wgpu::TextureView> {
		self.slot().map(|g| g.input())
	}

	/// The window-size MSAA colour and depth the HUD pass loads while the slot runs.
	pub(in crate::viewer) fn hud_targets(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView)> {
		self.slot().map(|g| g.hud_targets())
	}

	/// While the slot runs: its (render, window, HUD MSAA colour) sizes, for gpu/frame_sizes.rs.
	pub(in crate::viewer) fn slot_sizes(&self) -> Option<((u32, u32), (u32, u32), (u32, u32))> {
		self.slot().map(|g| {
			let (render, win) = g.sizes();
			let hud = g.hud_targets().0.texture();
			(render, win, (hud.width(), hud.height()))
		})
	}

	/// A HUD pixel rectangle (render size) for the HUD pass: the same rectangle when the slot is off, else at the window size.
	pub(in crate::viewer) fn hud_rect(&self, r: [f32; 4]) -> [f32; 4] {
		match self.slot() {
			Some(g) => {
				let (render, win) = g.sizes();
				scale_rect(r, [win.0 as f32 / render.0.max(1) as f32, win.1 as f32 / render.1.max(1) as f32], win)
			}
			None => r,
		}
	}

	/// At the split point: the upscaler from the scene's MSAA colour `msaa` into the window-size targets. FSR 3.1 / 4: resolve,
	/// reactive mask, the FidelityFX dispatch (ffx_frame.rs), the copy; DLSS the same without the mask. A frame in which neither
	/// records (its runtime refused just now; the slot is off from the next plan, `active`) gets the plain bilinear scale.
	pub(super) fn slot_encode(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, cuts: &mut crate::viewer::presenter::Cuts, msaa: &wgpu::TextureView) {
		let grade = self.grade.is_some();
		let Some(g) = self.slot.as_ref().filter(|_| self.slot_on) else { return };
		if !self.ffx_on && !self.dlss_on {
			g.encode_plain(device, enc, msaa, grade);
			return;
		}
		g.resolve(enc, msaa);
		g.mid_init(enc);
		let f = super::ffx_frame::FrameIn { jitter: self.jitter, cut: self.reset, frame: self.frame, far: self.cam.0, fov_y: self.cam.1, sharp: self.sharp };
		// One upscaler per frame: only the chosen one records, so FSR 3 and DLSS never both take the DX12 submit hook (review R16 F6).
		debug_assert!(!(self.ffx_on && self.dlss_on), "FSR 3 and DLSS in one frame");
		let dlss = self.dlss.as_mut().filter(|_| self.dlss_on);
		let ran = match (self.ffx.as_mut(), dlss, self.gpu.as_ref().filter(|_| self.on), g.ffx_textures()) {
			// [PC] DLSS (dlss_frame.rs) from the same inputs as FSR 3 (no reactive mask), into the same window-size `mid`.
			(_, Some(d), Some(mg), Some(_)) => {
				let (color, output) = g.views();
				d.dispatch(device, enc, cuts, &super::dlss_frame::Views { color, depth: mg.depth_view(), motion: mg.motion_view(), output }, &f)
			}
			(Some(ffx), None, Some(mg), Some((color, output))) => {
				let (render, _) = g.sizes();
				let opaque = self.opaque.as_ref().filter(|_| self.opaque_fresh.get()).map(|o| &o.1);
				// Like with like (review R16 F2): the scene before post when the post passes ran (`alpha_copy`), else the resolved input.
				let after = self.alpha.as_ref().filter(|_| self.alpha_fresh.get()).map_or(g.input(), |a| &a.1);
				let reactive = opaque.map(|o| ffx.reactive.encode(device, enc, (o, after, self.heat.take().as_ref()), render).clone());
				let io = super::ffx_frame::Io { color, depth: mg.depth_view().texture(), motion: mg.motion_view().texture(), reactive: reactive.as_ref(), output };
				ffx.dispatch(device, enc, cuts, &io, &f)
			}
			_ => false,
		};
		if !ran {
			g.scale(enc);
		}
		g.copy(device, enc, grade);
	}

	/// [PC] Brightness / Contrast (gpu/grade_pass.rs `grade_k`, upscale.rs `upscale_encode`): this frame's curve for the slot's copy,
	/// its uniform written now; None = the plain copy (50 / 50).
	pub(in crate::viewer) fn set_grade(&mut self, queue: &crate::viewer::presenter::GatedQueue, k: Option<[f32; 4]>) {
		self.grade = k;
		if let (Some(k), Some(g)) = (k, self.slot()) {
			g.write_grade(queue, k);
		}
	}
}

/// The FidelityFX side for `sizes`: made on first use (None stays None when the runtime is missing: no upscaler, `ffx_label`
/// marks FSR as not running), its context ready.
fn ffx_ready(ffx: &mut Option<super::ffx_frame::FfxGpu>, device: &wgpu::Device, queue: &crate::viewer::presenter::GatedQueue, sizes: ((u32, u32), (u32, u32))) -> bool {
	if ffx.is_none() {
		let Some(rt) = super::ffx::runtime(device) else {
			super::ffx_label(None);
			return false;
		};
		let f = super::ffx_frame::FfxGpu::new(device, &queue.raw(), rt);
		super::ffx_label(Some(&f.name));
		*ffx = Some(f);
	}
	ffx.as_mut().is_some_and(|f| f.prepare(sizes))
}

#[cfg(test)]
#[path = "slot_tests.rs"]
mod slot_tests;
