//! [PC] speed, lane tpre (no retail counterpart): an exact depth pre-pass for the tie batch, so the tie fragments that end up hidden
//! behind a nearer opaque tie texel are rejected by the early depth test instead of being lit (sun PCF, fire-shadow lookups, metal).
//!
//! Why the picture stays identical (per MSAA sample; ties use ZTST GEQUAL = LessEqual, every tie draw runs after the tfrags):
//! - The pre-pass draws the same tie draws with the same vertex entry (`vs`) from the same function text as the main tie pipelines of
//!   the frame (`text`: the plain bindless level source for Fls::Off / Plain, the fire module's text for Fls::Fire, each plus
//!   tiez.wgsl), so its depth is the main pass's depth; it writes Z only where `fs_tie` would draw a fully opaque texel (alpha 0x80,
//!   `world_alpha` 1.0: the blend then overwrites the colour, (Cs - Cd) x 1 + Cd = Cs on the unorm target), so it leaves the minimum
//!   of the tfrag Z and the opaque tie Z = the tie batch's final Z. With LessEqual the last opaque tie fragment at that Z still passes
//!   (the one that wins today), every fragment behind it now fails early instead of being drawn and then overwritten: the colour and
//!   the Z after the batch are the same.
//! - Not exact where a tie texel BLENDS and writes Z (alpha 0x40..0x7f, `fs_tie`): drawn before the nearest opaque texel, today it blends
//!   over whatever hidden tie came first. So a mark pass first stencils every sample a blending Z-writing tie texel reaches (depth-tested
//!   against the tfrags, as it is in the main pass), and the Z pass skips the marked samples (today's behaviour there).
//! - Worker 2: the Z pass writes its Z biased farther (`z_bias`): the pre-pass and the main tie pipelines are separate pipelines
//!   (other fragment entry, no fragment constants) with no shared @invariant on the position (adding it moves today's picture: needs
//!   the user's OK), so the driver need not give them the same last depth bits. The bound then never rejects the nearest opaque
//!   fragment, which writes its own Z, and a fragment within the bias behind it passes as today and is overwritten as today.
//!   REMAINING ASSUMPTION (review finding 2): the driver compiles the same `vs` text into the same clip-space x / y / w for both
//!   pipelines (same coverage) and its z differs by less than the bias. Proven only by md5 on NVIDIA Vulkan (tpre HANDOFF.md).
//! - Fire frames (review finding 1): the fire module shares one fire-shadow lookup per 2x2 quad only when `subgroupBallot` sees all
//!   4 lanes (flitquad.wgsl). The pre-pass turns hidden lanes into helper lanes; the picture stays only where helper lanes count in
//!   the ballot, shown on NVIDIA Vulkan only (DX12 leaves them out: its pictures moved). So `backend_ok`: Vulkan AND an NVIDIA adapter
//!   (vendor 0x10de); other vendors never build it. The auto case needs >= 3.3 Mpx anyway (`auto`), so the Steam Deck never qualifies.
//! - The main level modules are byte-identical to the pre-lane text on every backend: the pre-pass has its own modules (`TieZ::new`),
//!   made with its pipelines on the first frame that takes it, inside the build guard (`Effect::TieZ`: a refusal = off for the run).
//! - The stencil goes back to shadowvol.rs `STENCIL_CLEAR` with one full-screen triangle before the shadow volumes count in it.
//! - Only the bindless multi-draw path (bindless.rs) has it; the tie texels below 0x40 (`fs_tie_lt64`, no Z) are untouched.
//! Gate: `wanted` (RAC_TIEZ, RAC_TUNE tiez_ab), at the first tie batch of a pass.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;

/// The pre-pass entries, appended to the frame's bindless level text (`text`).
pub(super) const WGSL: &str = include_str!("tiez.wgsl");

/// The default Z-pass depth bias (farther, in depth units).
const Z_BIAS: i32 = 16;

/// Depth bias of the Z pass (farther, in depth units): covers last-bit depth differences between the pre-pass pipelines and the main
/// tie pipelines (no shared @invariant). RAC_TUNE tiez_bias=N (read when the pipelines are built), default `Z_BIAS`.
fn z_bias() -> i32 {
	crate::devtune::f32("tiez_bias", Z_BIAS as f32) as i32
}

/// Stencil value of a marked sample (one above the shadow-volume start value).
const MARK: u32 = super::shadowvol::STENCIL_CLEAR + 1;

/// NVIDIA's PCI vendor id.
const NVIDIA: u32 = 0x10de;

/// The pre-pass exists only on Vulkan with an NVIDIA adapter (review finding 1: the fire module's quad sharing with helper lanes is
/// proven there only; DX12 pictures moved). Elsewhere `Bindless::tiez` is None and the tie batch never asks `wanted`.
pub(super) fn backend_ok(ai: &wgpu::AdapterInfo) -> bool {
	ai.backend == wgpu::Backend::Vulkan && ai.vendor == NVIDIA
}

/// The pre-pass module text of a frame: the main tie pipelines' text (`main`: the plain bindless source, or the fire module's text
/// `flitquad::fire_source` of it) with the pre-pass entries after it, so `vs` and everything it calls are the same text.
fn text(main: String) -> String {
	main + WGSL
}

/// What the pipelines are built from (bindless.rs `Bindless::new`): the bindless level layout and source.
pub(super) struct Maker {
	pub device: wgpu::Device,
	pub layout: wgpu::PipelineLayout,
	/// The bindless level source (`bindless::shader_source`), the plain module's text.
	pub src: String,
	pub format: wgpu::TextureFormat,
	pub depth_format: wgpu::TextureFormat,
}

/// One pre-pass pipeline's states (pure, pinned by the tests: the exactness argument rests on them).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct State {
	vs: &'static str,
	fs: &'static str,
	depth_write: bool,
	depth_compare: wgpu::CompareFunction,
	stencil_compare: wgpu::CompareFunction,
	stencil_pass: wgpu::StencilOperation,
	stencil_write_mask: u32,
	/// Constant depth bias (0, or `z_bias` farther).
	bias: i32,
	/// Vertex buffers (the tie `Vertex` layout) or none (the full-screen triangle).
	buffers: bool,
}

/// The mark, Z and restore states (`ztest`: the tie test, gs_state.rs, LessEqual; `bias`: the Z pass's).
fn states(ztest: wgpu::CompareFunction, bias: i32) -> [State; 3] {
	use wgpu::{CompareFunction as C, StencilOperation as S};
	[
		// Mark: no Z write, the tie test against the tfrag Z; every passing sample's stencil = MARK.
		State { vs: "vs", fs: "fs_tiez_mark", depth_write: false, depth_compare: ztest, stencil_compare: C::Always, stencil_pass: S::Replace, stencil_write_mask: 0xff, bias: 0, buffers: true },
		// Z: the opaque texels write Z (biased farther) only where the stencil is still STENCIL_CLEAR (unmarked); stencil untouched.
		State { vs: "vs", fs: "fs_tiez_z", depth_write: true, depth_compare: ztest, stencil_compare: C::Equal, stencil_pass: S::Keep, stencil_write_mask: 0, bias, buffers: true },
		// Restore: every sample's stencil back to STENCIL_CLEAR, no depth test or write.
		State { vs: "vs_tiez_full", fs: "fs_tiez_none", depth_write: false, depth_compare: C::Always, stencil_compare: C::Always, stencil_pass: S::Replace, stencil_write_mask: 0xff, bias: 0, buffers: false },
	]
}

/// The pre-pass pipelines of one module variant (no cull, back, front: `Draw::cull`, as `bindless::Bindless::tie`) and the restore.
struct Pipes {
	mark: [wgpu::RenderPipeline; 3],
	z: [wgpu::RenderPipeline; 3],
	restore: wgpu::RenderPipeline,
}

impl Maker {
	fn make(&self, module: &wgpu::ShaderModule, s: State, cull: Option<wgpu::Face>) -> wgpu::RenderPipeline {
		let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4];
		let vb = [Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<super::Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })];
		let face = wgpu::StencilFaceState { compare: s.stencil_compare, fail_op: wgpu::StencilOperation::Keep, depth_fail_op: wgpu::StencilOperation::Keep, pass_op: s.stencil_pass };
		let cache = super::pipewarm::cache();
		self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some(s.fs),
			layout: Some(&self.layout),
			vertex: wgpu::VertexState { module, entry_point: Some(s.vs), compilation_options: Default::default(), buffers: if s.buffers { &vb } else { &[] } },
			fragment: Some(wgpu::FragmentState {
				module,
				entry_point: Some(s.fs),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState { format: self.format, blend: None, write_mask: wgpu::ColorWrites::empty() })],
			}),
			primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
			depth_stencil: Some(wgpu::DepthStencilState {
				format: self.depth_format,
				depth_write_enabled: Some(s.depth_write),
				depth_compare: Some(s.depth_compare),
				stencil: wgpu::StencilState { front: face, back: face, read_mask: 0xff, write_mask: s.stencil_write_mask },
				bias: wgpu::DepthBiasState { constant: s.bias, slope_scale: 0.0, clamp: 0.0 },
			}),
			multisample: wgpu::MultisampleState { count: super::targets::msaa(), ..Default::default() }, // the scene sample count (scene_msaa.rs; 4x or 1 for the run)
			multiview_mask: None,
			cache: cache.as_ref().map(|c| &c.cache),
		})
	}

	/// The variant's module (`fire`: the fire module's text) and its pipelines.
	fn build(&self, fire: bool) -> Pipes {
		let main = if fire { super::flitquad::fire_source("level bindless tiez", &self.src) } else { self.src.clone() };
		let label = if fire { "level bindless tiez fire" } else { "level bindless tiez" };
		let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(text(main).into()) });
		let [mark, z, restore] = states(crate::gs_state::ztest(), z_bias());
		let culls = [None, Some(wgpu::Face::Back), Some(wgpu::Face::Front)];
		Pipes { mark: culls.map(|c| self.make(&module, mark, c)), z: culls.map(|c| self.make(&module, z, c)), restore: self.make(&module, restore, None) }
	}
}

/// The pre-pass: its maker and the two module variants, each built on the first frame that takes it (None = refused).
pub(crate) struct TieZ {
	maker: Maker,
	/// [plain, fire]
	sets: [OnceLock<Option<Pipes>>; 2],
}

impl TieZ {
	pub(super) fn new(maker: Maker) -> TieZ {
		TieZ { maker, sets: Default::default() }
	}

	/// The pipelines of variant `fire`, built now inside the build guard on first use; None once the driver refused one.
	fn pipes(&self, fire: bool) -> Option<&Pipes> {
		if super::pipe_guard::off(super::pipe_guard::Effect::TieZ) {
			return None;
		}
		let d = &self.maker.device;
		self.sets[fire as usize].get_or_init(|| super::pipe_guard::guard(d, super::pipe_guard::Effect::TieZ, || self.maker.build(fire))).as_ref()
	}
}

impl Pipes {
	/// Before the main tie draws (`runs`, the tie list of the frame's multi-draw runs): mark, then the Z of the opaque texels.
	fn pre(&self, pass: &mut wgpu::RenderPass<'_>, bl: &super::bindless::Bindless, runs: &super::bindless::Runs) {
		pass.set_stencil_reference(MARK);
		bl.multi_draw_set(pass, runs, &self.mark);
		super::passtime::lap(pass, "tiez mark"); // [probe] RAC_GPU_PASSES lap (passtime.rs)
		pass.set_stencil_reference(super::shadowvol::STENCIL_CLEAR);
		bl.multi_draw_set(pass, runs, &self.z);
		super::passtime::lap(pass, "tiez z"); // [probe] RAC_GPU_PASSES lap (passtime.rs)
	}

	/// After the main tie draws: every sample's stencil back to the shadow-volume start value.
	fn restore(&self, pass: &mut wgpu::RenderPass<'_>) {
		pass.set_pipeline(&self.restore);
		pass.set_stencil_reference(super::shadowvol::STENCIL_CLEAR);
		pass.draw(0..3, 0..1);
	}
}

/// The multi-draw tie pass `tp` (gpu/draw/pass_world.rs), with the pre-pass around the Z-writing one (`tp` 0) where `wanted`
/// (`size`: the level pass's render target). The variant follows the frame's tie set (`metalpipe::fls_fire`).
pub(super) fn draw_ties(pass: &mut wgpu::RenderPass<'_>, bl: &super::bindless::Bindless, runs: &super::bindless::Runs, tp: usize, size: wgpu::Extent3d) {
	let fire = super::metalpipe::fls_fire();
	let tz = bl.tiez.as_ref().filter(|_| tp == 0 && wanted(fire, size.width * size.height)).and_then(|t| t.pipes(fire));
	if let Some(p) = tz {
		p.pre(pass, bl, runs);
	}
	bl.multi_draw_set(pass, runs, &bl.tie[tp]);
	if let Some(p) = tz {
		p.restore(pass);
	}
}

/// RAC_TIEZ: "0" never, "1" every frame; unset or any other value = `auto`.
fn mode() -> u8 {
	static M: OnceLock<u8> = OnceLock::new();
	*M.get_or_init(|| parse_mode(std::env::var("RAC_TIEZ").ok().as_deref()))
}

fn parse_mode(v: Option<&str>) -> u8 {
	match v {
		Some("0") => 0,
		Some("1") => 1,
		_ => 2,
	}
}

/// Whether this pass's tie batch takes the pre-pass. RAC_TUNE tiez_ab=N (A/B probe): N frames off, N frames on, ... counted in
/// frames (frame_clock.rs `stamp`; a split-view frame asks twice), prints `tiez ab: mode M` at each switch.
/// `px`: the level pass's render size in pixels (after render scale / upscaler).
pub(super) fn wanted(fire: bool, px: u32) -> bool {
	let n = crate::devtune::f32("tiez_ab", 0.0) as u32;
	if n > 0 {
		return ab(n);
	}
	match mode() {
		0 => false,
		1 => true,
		_ => auto(fire, px),
	}
}

fn ab(n: u32) -> bool {
	static FRAME: AtomicU32 = AtomicU32::new(0);
	static LAST: AtomicU64 = AtomicU64::new(u64::MAX);
	let st = super::frame_clock::stamp();
	let new = st == 0 || LAST.swap(st, Ordering::Relaxed) != st;
	let f = if new { FRAME.fetch_add(1, Ordering::Relaxed) } else { FRAME.load(Ordering::Relaxed).saturating_sub(1) };
	let m = (f / n) % 2;
	if new && f % n == 0 {
		eprintln!("tiez ab: mode {m}");
	}
	m == 1
}

/// Smallest render size (pixels) where `auto` takes the pre-pass: 2560x1440 native measured faster, DLSS Quality at 1440p
/// (1707x960) not (worker 3, tpre HANDOFF.md); about 0.9 x 1440p.
const AUTO_MIN_PX: u32 = 3_300_000;

/// The default rule (worker 3 A/B, tpre HANDOFF.md; review finding 5): only frames drawn with the fire module (Fls::Fire: a light
/// shades from a fire shadow map, so each hidden tie fragment saved skips the fire lookups) rendered at about 1440p or more.
/// Non-fire scenes split (faster where ties overdraw a lot, slower in light tie scenes, and the CPU cannot tell them apart), every
/// scene at DLSS Quality was within noise or slower.
fn auto(fire: bool, px: u32) -> bool {
	fire && px >= AUTO_MIN_PX
}

#[cfg(test)]
#[path = "tiez_tests.rs"]
mod tests;
