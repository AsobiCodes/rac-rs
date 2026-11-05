//! [PC] Temporal upscaler inputs (lane ndlss, stage 1; no retail counterpart: the game never sees any of this). What DLSS / FSR
//! need that the renderer did not make: a sub-pixel camera jitter, per-pixel motion vectors, a single-sample depth and a
//! "history is invalid" flag. Dev switch `RAC_UPSCALE_PREP` (ENV.md), default off: then nothing here runs, no matrix, target or
//! pass changes, and the picture is exactly the old one. Plan and later stages: scratch_perf_ndlss\PLAN.md.
//! - Jitter: Halton(2, 3) sub-pixel offsets, phase count `ceil(8 * (display / render)^2)` (the FSR rule), applied as a clip-space
//!   translation to `Uniforms::view_proj` and `sky_view_proj` after `uniforms()` (`Prep::apply`). Only the renderer's matrices
//!   move; CPU culling, the GS capture and the HUD keep the plain camera.
//! - Motion vectors (`motion.rs`): one full-screen pass before the HUD pass reprojects each depth pixel with this frame's jittered
//!   inverse matrix and last DISPLAY frame's plain matrix (depth 1.0, nothing drawn: the sky matrices). Exact for everything
//!   static; moving mobys and the hero get their own pass in stage 2. Units: UV (0..1) of the render picture, current ->
//!   previous (prev_uv - cur_uv, y down), the convention of both FSR (`motionVectorScale` = render size) and DLSS.
//! - Reset (`is_cut`): no previous frame, another level, another render / window size, or the eye jumped / turned like a cut.
//! - Split view (F10): the Classic half (gpu/draw/split.rs) runs between two `Gpu::draw`s with this state paused (`Prep::pause`:
//!   no slot, no jitter, no motion pass, the history untouched) and restored after; `upscale_begin` gates on `splitview::fx`.

use super::*;
use std::sync::OnceLock;

pub(in crate::viewer) mod motion;
pub mod choice;
pub(in crate::viewer) mod dlss;
mod dlss_frame;
mod ffx;
mod ffx_abi;
#[cfg(windows)]
mod ffx_dx12;
mod ffx_frame;
mod ffx_reactive;
pub(in crate::viewer) mod slot;
mod slot_gpu;
pub(in crate::viewer) mod velocity;
#[cfg(test)]
mod tests;

type M4 = [[f32; 4]; 4];

/// A camera move longer than this in one display frame is a cut (the frame smoothing's `SNAP_DIST`, metres).
pub const CUT_DIST: f32 = super::smooth::SNAP_DIST;
/// A view turn sharper than this in one display frame is a cut (cosine of 45 degrees: a fast mouse turn at 30 fps stays under it).
pub const CUT_COS: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// The shortest jitter cycle (FSR's rule gives 8 at native size, more when the picture is upscaled).
pub const MIN_PHASES: u32 = 8;

/// What `RAC_UPSCALE_PREP` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
	/// Unset / 0: nothing runs (the old picture, exactly).
	Off,
	/// 1: jitter + motion vectors + depth copy.
	On,
	/// still: motion vectors + depth copy without jitter (to check the vectors on a steady picture).
	Still,
	/// show: as On, and the motion vectors drawn over the scene (grey = still; red / green = x / y motion) under the HUD.
	Show,
}

/// The mode for an env value.
pub fn parse_mode(v: Option<&str>) -> Mode {
	match v.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
		None | Some("") | Some("0") | Some("off") => Mode::Off,
		Some("still") => Mode::Still,
		Some("show") => Mode::Show,
		Some(_) => Mode::On,
	}
}

/// `RAC_UPSCALE_PREP`, read once.
pub fn mode() -> Mode {
	static M: OnceLock<Mode> = OnceLock::new();
	*M.get_or_init(|| parse_mode(std::env::var("RAC_UPSCALE_PREP").ok().as_deref()))
}

/// The radical inverse of `index` in `base` (Halton sequence, 0 < value < 1 for index >= 1).
pub fn halton(mut index: u32, base: u32) -> f32 {
	let (mut f, mut r) = (1.0f64, 0.0f64);
	while index > 0 {
		f /= base as f64;
		r += f * (index % base) as f64;
		index /= base;
	}
	r as f32
}

/// Jitter phases for a render width upscaled to a display width: ceil(8 * ratio^2), at least `MIN_PHASES`.
pub fn phase_count(render_w: u32, display_w: u32) -> u32 {
	let ratio = display_w.max(1) as f64 / render_w.max(1) as f64;
	((8.0 * ratio * ratio).ceil() as u32).max(MIN_PHASES)
}

/// The jitter of frame `frame` in render pixels, each axis in (-0.5, 0.5): Halton(2) for x, Halton(3) for y, index 1..=phases.
pub fn jitter_px(frame: u32, phases: u32) -> [f32; 2] {
	let i = frame % phases.max(1) + 1;
	[halton(i, 2) - 0.5, halton(i, 3) - 0.5]
}

/// `m` (column-major view-projection) moved by `j` render pixels on a `size` picture: the clip-space translation
/// x += 2 jx / w * clip_w, y -= 2 jy / h * clip_w (pixel y runs down, NDC y up).
pub fn jitter_matrix(m: &M4, j: [f32; 2], size: (u32, u32)) -> M4 {
	let (ox, oy) = (2.0 * j[0] / size.0.max(1) as f32, -2.0 * j[1] / size.1.max(1) as f32);
	let mut out = *m;
	for c in out.iter_mut() {
		c[0] += ox * c[3];
		c[1] += oy * c[3];
	}
	out
}

/// Where the camera stood and what it drew into, to tell a cut from a move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
	pub level: usize,
	pub render: (u32, u32),
	pub display: (u32, u32),
	pub eye: [f32; 3],
	pub fwd: [f32; 3],
}

/// Whether the history must be dropped going from `prev` to `cur` (no previous view counts as a cut).
pub fn is_cut(prev: Option<&View>, cur: &View) -> bool {
	let Some(p) = prev else { return true };
	let d = [cur.eye[0] - p.eye[0], cur.eye[1] - p.eye[1], cur.eye[2] - p.eye[2]];
	let dot = cur.fwd[0] * p.fwd[0] + cur.fwd[1] * p.fwd[1] + cur.fwd[2] * p.fwd[2];
	p.level != cur.level || p.render != cur.render || p.display != cur.display || (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) > CUT_DIST * CUT_DIST || dot < CUT_COS
}

/// The plain (unjittered) world and sky matrices of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mats {
	pub vp: M4,
	pub sky: M4,
}

/// The motion pass's uniform block (motion.wgsl `U`; 416 bytes, std140-compatible).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct MotionUniforms {
	/// Inverse of this frame's jittered world matrix (what drew the depth).
	pub inv_cur: M4,
	/// This frame's world matrix without jitter.
	pub cur: M4,
	/// Last frame's world matrix without jitter (`cur` after a cut: zero motion).
	pub prev: M4,
	pub sky_inv_cur: M4,
	pub sky_cur: M4,
	pub sky_prev: M4,
	/// Render width, height, 1 / width, 1 / height.
	pub size: [f32; 4],
	/// x: show scale (UV -> colour), y: 1 on a reset frame, zw: the jitter in render pixels.
	pub misc: [f32; 4],
}

/// One frame's uniform block: `cur` / `prev` plain matrices, `jit` the jittered pair this frame draws with.
pub fn motion_uniforms(cur: &Mats, prev: &Mats, jit: &Mats, size: (u32, u32), jitter: [f32; 2], reset: bool) -> MotionUniforms {
	let inv = |m: &M4| super::mirror::inverse(m).unwrap_or(*m);
	let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
	MotionUniforms {
		inv_cur: inv(&jit.vp),
		cur: cur.vp,
		prev: prev.vp,
		sky_inv_cur: inv(&jit.sky),
		sky_cur: cur.sky,
		sky_prev: prev.sky,
		size: [w, h, 1.0 / w, 1.0 / h],
		misc: [w / 32.0, reset as u8 as f32, jitter[0], jitter[1]],
	}
}

/// The Gpu's upscaler-input state (`Gpu::upscale`).
#[derive(Default)]
pub(in crate::viewer) struct Prep {
	/// This frame runs the passes (`begin` ran; false in an idle frame: switch off, the Classic half of a split view).
	on: bool,
	frame: u32,
	jitter: [f32; 2],
	render: (u32, u32),
	reset: bool,
	view: Option<View>,
	last: Option<(Mats, View)>,
	gpu: Option<motion::MotionGpu>,
	/// The upscaler slot (slot.rs, RAC_UPSCALE): its GPU side and whether this frame runs it.
	slot: Option<slot_gpu::SlotGpu>,
	slot_on: bool,
	/// The object motion pass (velocity.rs) and the opaque-only picture before the particles (`opaque_copy`), render size.
	vel: Option<velocity::VelocityGpu>,
	opaque: Option<((u32, u32), wgpu::TextureView)>,
	/// `opaque_copy` ran this frame (else the picture is an old frame's: no reactive mask).
	opaque_fresh: std::cell::Cell<bool>,
	/// [PC] FSR 3 only: the scene after the particles and before the post passes (`alpha_copy`), render size, and whether it was
	/// copied this frame: the reactive mask compares it with `opaque`, like with like (review R16 F2; without post the slot's
	/// resolved input is that same picture).
	alpha: Option<((u32, u32), wgpu::TextureView)>,
	alpha_fresh: std::cell::Cell<bool>,
	/// [PC] FSR 3.1 / 4 (ffx_frame.rs): the FidelityFX side (None until first chosen, and while the runtime is missing) and
	/// whether this frame runs it; the slot's sharpening (stops), the camera's far plane and vertical fov for the dispatch.
	ffx: Option<ffx_frame::FfxGpu>,
	ffx_on: bool,
	/// [PC] DLSS (dlss_frame.rs): the NGX side (None until first chosen, and while NGX is missing) and whether this frame runs it.
	dlss: Option<dlss_frame::DlssGpu>,
	dlss_on: bool,
	sharp: f32,
	cam: (f32, f32),
}

impl Prep {
	/// First step of a frame (`Gpu::draw`): the cut test, the frame's jitter and the motion targets at the render size.
	/// `m`: `mode()`, or On while FSR 3.1 / 4 runs (`upscale_begin`).
	pub(in crate::viewer) fn begin(&mut self, device: &wgpu::Device, depth: &wgpu::TextureView, format: wgpu::TextureFormat, view: View, m: Mode) {
		self.on = m != Mode::Off;
		if !self.on {
			return;
		}
		self.reset = is_cut(self.last.as_ref().map(|l| &l.1), &view);
		self.frame = self.frame.wrapping_add(1);
		self.jitter = if m == Mode::Still { [0.0; 2] } else { jitter_px(self.frame, phase_count(view.render.0, view.display.0)) };
		self.render = view.render;
		self.view = Some(view);
		self.gpu.get_or_insert_with(|| motion::MotionGpu::new(device, format)).prepare(device, depth, view.render);
		self.vel.get_or_insert_with(|| velocity::VelocityGpu::new(device));
		if self.opaque.as_ref().map_or(true, |o| o.0 != view.render) {
			self.opaque = Some((view.render, opaque_target(device, format, view.render)));
		}
		if !(self.ffx_on && self.slot_on) {
			self.alpha = None;
		} else if self.alpha.as_ref().map_or(true, |a| a.0 != view.render) {
			self.alpha = Some((view.render, opaque_target(device, format, view.render)));
		}
	}

	/// In `frame_uniforms`, right after `uniforms()`: keeps the plain matrices, jitters `u`'s, writes the motion pass's block.
	pub(in crate::viewer) fn apply(&mut self, queue: &super::presenter::GatedQueue, u: &mut Uniforms) {
		let (Some(view), Some(gpu)) = (self.view.take(), self.gpu.as_ref()) else { return };
		let cur = Mats { vp: u.view_proj, sky: u.sky_view_proj };
		let prev = match (&self.last, self.reset) {
			(Some((p, _)), false) => *p,
			_ => cur,
		};
		u.view_proj = jitter_matrix(&cur.vp, self.jitter, self.render);
		u.sky_view_proj = jitter_matrix(&cur.sky, self.jitter, self.render);
		let jit = Mats { vp: u.view_proj, sky: u.sky_view_proj };
		gpu.write(queue, &motion_uniforms(&cur, &prev, &jit, self.render, self.jitter, self.reset));
		self.last = Some((cur, view));
	}

	/// Before the HUD pass: the motion vectors and depth copy (and with `show` their picture into the MSAA target `msaa`).
	/// `objs`: what moves on its own this frame (velocity.rs), drawn over the camera vectors.
	pub(in crate::viewer) fn encode(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, cuts: &mut super::presenter::Cuts, msaa: &wgpu::TextureView, objs: velocity::Objects<'_>) {
		if let (true, Some(gpu)) = (self.on, self.gpu.as_ref()) {
			let vel = &mut self.vel;
			gpu.encode(enc, msaa, mode() == Mode::Show, |enc, mg| {
				if let Some(v) = vel.as_mut() {
					v.encode(device, enc, mg, &objs);
				}
			});
		}
		self.slot_encode(device, enc, cuts, msaa); // [PC] RAC_UPSCALE (slot.rs): the upscaler into the window-size HUD targets
	}

	/// [PC] At the end of the level pass's opaque part (passes.rs, Particles Enhanced path, before the particles): the MSAA scene so
	/// far resolved into the render-size opaque picture, the input of an upscaler's reactive mask (what the particles and
	/// see-through effects add on top). An empty pass whose resolve is the copy; nothing while the inputs are off.
	pub(in crate::viewer) fn opaque_copy(&self, enc: &mut wgpu::CommandEncoder, msaa: &wgpu::TextureView) {
		let Some((_, view)) = self.opaque.as_ref().filter(|_| self.on) else { return };
		self.opaque_fresh.set(true);
		let att = wgpu::RenderPassColorAttachment { view: msaa, resolve_target: Some(view), depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } };
		drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("upscale opaque"), color_attachments: &[Some(att)], depth_stencil_attachment: None, timestamp_writes: super::passtime::rw("upscale opaque"), occlusion_query_set: None, multiview_mask: None }));
	}

	/// [PC] FSR 3, at the start of the post passes (passes.rs, after the particles, fire and pool lines): the MSAA scene resolved
	/// into `alpha`, the reactive mask's other input (review R16 F2: post-processing must not count as reactive). Without that
	/// target this frame gets no mask at all (`opaque_fresh` cleared) rather than one against the post-processed picture.
	pub(in crate::viewer) fn alpha_copy(&self, enc: &mut wgpu::CommandEncoder, msaa: &wgpu::TextureView) {
		if !self.opaque_fresh.get() {
			return;
		}
		let Some((_, view)) = self.alpha.as_ref().filter(|_| self.on) else {
			self.opaque_fresh.set(false);
			return;
		};
		self.alpha_fresh.set(true);
		let att = wgpu::RenderPassColorAttachment { view: msaa, resolve_target: Some(view), depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } };
		drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("upscale alpha"), color_attachments: &[Some(att)], depth_stencil_attachment: None, timestamp_writes: super::passtime::rw("upscale alpha"), occlusion_query_set: None, multiview_mask: None }));
	}

	/// The FidelityFX side refused a create or dispatch this run (the menu then stops offering FSR, review R16 F5).
	fn ffx_refused(&self) -> bool {
		self.ffx.as_ref().is_some_and(|f| !f.usable())
	}

	/// [PC] Split view (gpu/draw/split.rs `split_classic_pass`): the Classic half draws with the upscaler and its inputs off (the
	/// window-size HUD targets would not fit its render-size target; its motion pass would overwrite the history). Returns this
	/// frame's (on, slot_on) for `resume`.
	pub(in crate::viewer) fn pause(&mut self) -> (bool, bool) {
		(std::mem::take(&mut self.on), std::mem::take(&mut self.slot_on))
	}

	/// After the Classic half: the Enhanced state `pause` returned.
	pub(in crate::viewer) fn resume(&mut self, was: (bool, bool)) {
		(self.on, self.slot_on) = was;
	}

	/// Whether this frame records the motion pass (the level pass then keeps its MSAA colour for the HUD pass to load).
	pub(in crate::viewer) fn active(&self) -> bool {
		self.on && self.gpu.is_some() || self.slot_on()
	}

	/// For the upscaler (stage 3): motion vectors (Rg16Float, UV units, current -> previous), depth (R32Float, 0 near .. 1 far),
	/// the jitter in render pixels and whether the history must be dropped.
	#[allow(dead_code)]
	pub(in crate::viewer) fn outputs(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView, [f32; 2], bool)> {
		let g = self.gpu.as_ref().filter(|_| self.on)?;
		Some((g.motion_view(), g.depth_view(), self.jitter, self.reset))
	}
}

/// The render-size single-sample picture `opaque_copy` resolves into (the MSAA colour's format).
fn opaque_target(device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32)) -> wgpu::TextureView {
	let size = wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 };
	let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
	device.create_texture(&wgpu::TextureDescriptor { label: Some("upscale opaque"), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] }).create_view(&Default::default())
}

/// [PC] The Upscaler row's FidelityFX value (ui/live.rs): the version's short name ("FSR 3.1"), offered; `None` (no runtime or
/// no version on this PC, or a refusal this run) = FSR cannot run here: not offered, a choice of it is Off (choice.rs `set_runs`).
fn ffx_label(name: Option<&str>) {
	match name {
		// The runtime is probed once per run (ffx.rs `runtime`), so its text is set once, not at every plan (review R28 m6).
		Some(n) if !FFX_NAMED.swap(true, std::sync::atomic::Ordering::Relaxed) => crate::ui::live::set(choice::KEY, choice::FFX_VALUE, ffx_frame::short_label(n)),
		Some(_) => {}
		None => {
			FFX_NAMED.store(false, std::sync::atomic::Ordering::Relaxed);
			crate::ui::live::clear(choice::KEY, choice::FFX_VALUE);
		}
	}
	choice::set_runs(choice::FFX_VALUE, name.is_some());
}

/// `ffx_label` set the version's text and nothing cleared it since.
static FFX_NAMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl Gpu {
	/// [PC] Right after the window's device is made (gpu/setup.rs `Gpu::new`, review R28 m2): whether FSR and DLSS can run here is
	/// known before any menu opens, so the Upscaler row offers only those from its first frame and a press can never pick one that
	/// is withdrawn a frame later. FidelityFX: its runtime, probed once per run (ffx.rs, the probe a plan or the menu asks for
	/// anyway); DLSS: what the device got (`label_now`, no NGX call). Only for a game with a modern menu (`Pack::ui`; R&C2 has none
	/// and loads no FidelityFX DLL).
	pub(in crate::viewer) fn upscale_probe(&self) {
		if crate::pack::get().ui.is_some() {
			self.ffx_probe();
			dlss_frame::label_now();
		}
	}

	/// [PC] Names the FidelityFX value (or withdraws it: no runtime, no version, a refusal this run).
	fn ffx_probe(&self) {
		let refused = self.upscale.ffx_refused();
		ffx_label(ffx::runtime(&self.device).filter(|_| !refused).and_then(|rt| ffx_frame::pick_version(&rt.versions)).map(|v| v.1));
	}

	/// [PC] Before the HUD pass (both style paths): the upscaler inputs with this frame's moving objects (velocity.rs), then the slot.
	pub(in crate::viewer) fn upscale_encode(&mut self, enc: &mut wgpu::CommandEncoder, cuts: &mut super::presenter::Cuts) {
		let objs = match self.scene.as_ref().filter(|_| self.upscale.on) {
			Some(scene) => velocity::Objects::of(scene, &self.props_gpu),
			None => velocity::Objects::default(),
		};
		self.upscale.encode(&self.device, enc, cuts, &self.msaa, objs);
	}

	/// [PC] FSR 3.1 / 4: before a FidelityFX context is dropped, nothing of it may still run on the GPU (presenter flush: the
	/// last submit is on the queue; device poll: the queue is idle).
	fn upscale_idle(&self) {
		self.presenter.flush();
		let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
	}

	/// [PC] Before the sizes are planned (gpu/rscale.rs `rscale_plan`): the upscaler slot's pipelines when one is chosen (`on`),
	/// pipe-guarded (slot.rs `slot_build`), else the slot freed.
	pub(in crate::viewer) fn upscale_build(&mut self, on: bool) {
		if on {
			self.upscale.slot_build(&self.device, (self.config.format, self.depth_format));
			match slot::active().map(|s| s.kind) {
				// [PC] DLSS: NGX first, so the plan gets its optimal render size (slot.rs `render_size`); none = Off from this plan on.
				Some(slot::Kind::Dlss) if dlss::runtime(&self.device).is_none() => dlss_frame::label(None),
				// [PC] FSR: the FidelityFX runtime first (once), so a PC without it plans the plain picture (slot.rs `active`).
				Some(slot::Kind::Fsr) => self.ffx_probe(),
				_ => {}
			}
		} else {
			if self.upscale.upscaler_needs_idle(None, ((0, 0), (0, 0))) {
				self.upscale_idle();
			}
			self.upscale.slot_drop();
		}
	}

	/// [PC] RAC_UPSCALE_PREP (upscale.rs): the frame's cut test, jitter and motion targets; nothing when the switch is off.
	pub(in crate::viewer) fn upscale_begin(&mut self, cam: &Camera) {
		let (sizes, enhanced) = (((self.config.width, self.config.height), self.win_size()), crate::splitview::fx(true));
		// [PC] The upscaler the sizes were planned with (rscale.rs `planned`), never a fresh read of the option: `config` is its render size.
		let planned = self.rscale.planned().filter(|_| enhanced);
		if enhanced && self.upscale.upscaler_needs_idle(planned, sizes) {
			self.upscale_idle(); // [PC] FSR 3.1 / 4 / DLSS: the old context / feature goes in slot_begin (other sizes, or none now)
		}
		if crate::ui::live::seen(choice::KEY) {
			// [PC] The menu shows the Upscaler row: name the FidelityFX version (the runtime is probed once) and the DLSS value
			// (known since `upscale_probe`; this keeps them current after a refusal or a reported model).
			self.ffx_probe();
			dlss_frame::label_now();
		}
		self.upscale.slot_begin(&self.device, &self.queue, &self.depth, (self.config.format, self.depth_format), sizes, planned); // [PC] RAC_UPSCALE (slot.rs)
		self.upscale.opaque_fresh.set(false);
		self.upscale.alpha_fresh.set(false);
		// [PC] FSR 3.1 / 4 needs the temporal inputs: jitter, motion vectors (camera + objects), depth, the opaque picture.
		let m = match mode() {
			Mode::Off if self.upscale.temporal_on() => Mode::On,
			m => m,
		};
		if m == Mode::Off || !enhanced {
			self.upscale.on = false;
			return;
		}
		self.upscale.cam = (self.world_far(), 2.0 * cam.tan_half.atan());
		let view = View { level: self.level, render: (self.config.width, self.config.height), display: self.win_size(), eye: cam.pos, fwd: super::forward(cam) };
		self.upscale.begin(&self.device, &self.depth, self.config.format, view, m);
	}

	/// The height the sharp HUD overlay is drawn at (`set_hud_target`): the window's while the upscaler slot runs (the HUD pass
	/// is at the window size then, slot.rs), else the render size's. Asked before the frame's `upscale_begin`: from the plan
	/// (rscale.rs `planned`, what this frame's slot will run), not from the last frame's slot.
	pub fn hud_height(&self) -> u32 {
		if self.rscale.planned().is_some() { self.win_size().1 } else { self.config.height }
	}
}

/// [PC] The window's Gpu goes (review R21 m3, m4): the present thread's last submit (on DX12 its hook can hold the last ref of a
/// DLSS feature or FidelityFX context) and the GPU finish first, then the slot's contexts are released and NGX is shut down while
/// the device still lives. The fields drop after this, in their order (presenter first).
impl Drop for Gpu {
	fn drop(&mut self) {
		self.upscale_idle();
		self.upscale.slot_drop();
		dlss::shutdown(&self.device);
	}
}
