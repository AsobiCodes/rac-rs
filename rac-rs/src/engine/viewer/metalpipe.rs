//! [PC] Metal On / Off pipeline sets (review R24 M3). The level shader holds the metal look (metalfx.wgsl) behind the override
//! `MK_ON`: the On set is built with it true (the shader as written), the Off set with it false, so Metal Off and Lighting Classic run
//! the level shader without the metal code (no register pressure from it on the Steam Deck). `metal_look` returns the retail colour at
//! its first lines in both cases, so the Off picture is the same.
//! Lane dlit2-9 (GPU cost): the same for the fire-shadow lookup (flitshadow.wgsl `FLS_ON`): four sets, index `set(metal, fls)`; a
//! frame whose light rows read no fire shadow map (flit.rs `reads_maps`: GI Low / Off, Classic, no fire with a map) draws with the
//! sets without the lookup (`set_fls`, draw/passes.rs), the same picture without the code's register cost. Where a map can be read
//! (GI Medium / High with Dynamic shadows) both fire-shadow sets are built in the background from the first frame with fire lights
//! (`Pipes::prewarm`, pipewarm.rs; no stall later when a fire's map starts or stops); a frame that reads a map before they are all
//! built draws without the lookup (`Gpu::fls_sets`, test build 7). Lane fshade worker 7: a third state per Metal set (`Fls`, `FIRE`):
//! the lookup from the fire module (flitquad.rs) only while a light shades from a map, else the plain module's (the pre-lane sets).
//!
//! Each `Pipes` builds a set on its first use (a game started with Metal Off never builds the On set and the other way round);
//! `Deref` reads the set of the current pass (`set_on`, prepare.rs `frame_uniforms`: a split frame's Classic pass is recorded and
//! submitted before its Enhanced pass, so one flag per prepare is right). Only the pipelines whose fragment entry points can reach
//! `metal_look` are kept twice: the level's opaque, cull, moby, shrub, tfrag and tie sets (Gpu, bindless.rs `Bindless`, gpuskin.rs
//! `PropsGpu`); the others are built once with the default (On). The vegetation and fur pipelines (no metal set) keep the two
//! fire-shadow sets only (`Pipes::fls`, `fls_constants`).
//! Split view: the set follows the pass (`set_on` from prepare.rs per pass), so the Classic half runs the Off set.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

static ON: AtomicBool = AtomicBool::new(true);
/// The frame's fire-shadow state (`Fls` as u8; lane dlit2-9, lane fshade).
static FLS: AtomicU8 = AtomicU8::new(0);

/// Lane fshade worker 7: the fire-shadow half of a frame's sets. `Plain` = the lookup compiled in, the plain module (exactly the
/// pre-lane pipelines): the rows read a map but no light row codes one (flit.rs `lit_maps`: no lookup runs, yet any text change of
/// these pipelines moves NVIDIA's codegen); `Fire` = the fire module (flitquad.rs: the cheaper filter + quad sharing), only while at
/// least one light shades from a map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fls {
	Off,
	Plain,
	Fire,
}

/// Set bit 2: the fire module (flitquad.rs `Modules::of`), only with bit 1 (the lookup) set.
pub(crate) const FIRE: usize = 4;

/// The set bits of `Fls` (bit 1 the lookup, bit 2 the fire module).
const fn fls_bits(f: Fls) -> usize {
	match f {
		Fls::Off => 0,
		Fls::Plain => 2,
		Fls::Fire => 2 | FIRE,
	}
}

/// The set the next passes draw with: Metal On in this pass (`u.keep_tail.y` > 0).
pub(crate) fn set_on(on: bool) {
	ON.store(on, Ordering::Relaxed);
}

fn on() -> bool {
	ON.load(Ordering::Relaxed)
}

/// The fire-shadow half of the set the next passes draw with: the frame's light rows read a map (flit.rs `reads_maps`, written by
/// draw/passes.rs before the level pass; the passes recorded before it use the previous frame's flag, one frame).
pub(crate) fn set_fls(fls: Fls) {
	FLS.store(fls as u8, Ordering::Relaxed);
}

fn fls() -> Fls {
	match FLS.load(Ordering::Relaxed) {
		1 => Fls::Plain,
		2 => Fls::Fire,
		_ => Fls::Off,
	}
}

/// [PC] lane tpre (tiez.rs gate and module choice): this frame's level sets are the fire module's (`Fls::Fire`).
pub(crate) fn fls_fire() -> bool {
	fls() == Fls::Fire
}

/// The set index: bit 0 Metal On, bit 1 the fire-shadow lookup compiled in (plain module; `| FIRE` = the fire module's).
pub(crate) const fn set(metal: bool, fls: bool) -> usize {
	metal as usize | (fls as usize) << 1
}

/// The set of Metal `metal` and fire-shadow state `f`.
pub(crate) const fn set_of(metal: bool, f: Fls) -> usize {
	metal as usize | fls_bits(f)
}

/// The sets `Op`s work on: the lookup-off and the plain lookup-on set of Metal `metal`, or (`fire`) its fire-module set (twice: a
/// fixed-size pair).
pub(crate) const fn group(metal: bool, fire: bool) -> [usize; 2] {
	if fire { [set_of(metal, Fls::Fire); 2] } else { [set_of(metal, Fls::Off), set_of(metal, Fls::Plain)] }
}

/// The fragment stage's pipeline constants of a set (wgpu `PipelineCompilationOptions::constants`; a bool override takes 0 / 1).
/// RAC_METAL_CUT_OLD=1: the On set without the exact metal cuts (`MK_CUT` false in metalfx.wgsl; for A/B and picture identity).
pub(crate) fn constants(set: usize) -> &'static [(&'static str, f64)] {
	match (set & 1 != 0, set & 2 != 0, cut_old()) {
		(false, true, _) => &[("MK_ON", 0.0)],
		(false, false, _) => &[("MK_ON", 0.0), ("FLS_ON", 0.0)],
		(true, true, false) => &[],
		(true, true, true) => &[("MK_CUT", 0.0)],
		(true, false, false) => &[("FLS_ON", 0.0)],
		(true, false, true) => &[("MK_CUT", 0.0), ("FLS_ON", 0.0)],
	}
}

/// The fire-shadow constants alone, for the shaders without a metal set (vegetation, fur): `set & 2` = the lookup compiled in.
pub(crate) fn fls_constants(set: usize) -> &'static [(&'static str, f64)] {
	if set & 2 != 0 { &[] } else { &[("FLS_ON", 0.0)] }
}

fn cut_old() -> bool {
	static OLD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*OLD.get_or_init(|| std::env::var("RAC_METAL_CUT_OLD").is_ok_and(|v| v == "1"))
}

/// One pipeline (or array of them) per set, each built on its first use (or earlier by the background pool, pipewarm.rs).
pub(crate) struct Pipes<T> {
	inner: Arc<Inner<T>>,
}

struct Inner<T> {
	/// Index `set_of` (4 and 5 unused: the fire bit comes only with the lookup bit).
	sets: [OnceLock<T>; 8],
	make: Box<dyn Fn(usize) -> T + Send + Sync>,
	/// Bit `set`: a background build of the set was queued (`Pipes::prewarm`).
	queued: AtomicU8,
}

impl<T> Pipes<T> {
	pub(crate) fn new(make: impl Fn(usize) -> T + Send + Sync + 'static) -> Pipes<T> {
		Pipes { inner: Arc::new(Inner { sets: Default::default(), make: Box::new(make), queued: AtomicU8::new(0) }) }
	}

	/// Set `set` (`set(metal, fls)`), built now if it is not yet (waits for a build another thread has started).
	pub(crate) fn get(&self, set: usize) -> &T {
		let c = &self.inner.sets[set];
		if let Some(t) = c.get() {
			return t;
		}
		// [PC] RAC_DIAG spikes.txt: a build (or the wait for one under way) off the background pool held this thread's frame.
		if !std::thread::current().name().is_some_and(|n| n.starts_with("pipewarm")) {
			crate::diag::cause(crate::diag::PIPELINE);
		}
		c.get_or_init(|| (self.inner.make)(set))
	}

	/// Builds the sets of `group(metal, fire)` now (no build stall later when a fire's map starts or stops being read).
	pub(crate) fn warm(&self, metal: bool, fire: bool) {
		for s in group(metal, fire) {
			self.get(s);
		}
	}

	/// The sets of `group(metal, fire)` are built.
	pub(crate) fn warmed(&self, metal: bool, fire: bool) -> bool {
		group(metal, fire).iter().all(|&s| self.inner.sets[s].get().is_some())
	}

	/// The current fire-shadow set with Metal's default (On): the pipelines without a metal set (vegetation, fur).
	pub(crate) fn fls(&self) -> &T {
		self.get(set_of(true, fls()))
	}
}

/// What `Gpu::fls_sets` does to the two fire-shadow sets of a pipeline (`Pipes::op`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Op {
	/// Queue their background build (pipewarm.rs).
	Prewarm,
	/// Build them now (waits for a build under way).
	Warm,
	/// Are both built?
	Warmed,
}

impl<T: Send + Sync + 'static> Pipes<T> {
	/// Queues the background build (pipewarm.rs) of the sets of `group(metal, fire)`, once per set.
	pub(crate) fn prewarm(&self, metal: bool, fire: bool) {
		self.prewarm_in(metal, fire, None);
	}

	/// `prewarm`, each build inside effect `guard`'s build guard (pipe_guard.rs; its error scopes are per thread, so they open on the
	/// pool's). The lookup-off set is queued first: `Gpu::fls_sets` draws with it while the other one still builds.
	pub(crate) fn prewarm_in(&self, metal: bool, fire: bool, guard: Option<(&wgpu::Device, super::pipe_guard::Effect)>) {
		for s in group(metal, fire) {
			if self.inner.queued.fetch_or(1 << s, Ordering::Relaxed) & (1 << s) == 0 {
				let (inner, guard) = (self.inner.clone(), guard.map(|(d, e)| (d.clone(), e)));
				super::pipewarm::submit(Box::new(move || {
					let build = || {
						inner.sets[s].get_or_init(|| (inner.make)(s));
					};
					match guard {
						Some((d, e)) => drop(super::pipe_guard::guard(&d, e, build)),
						None => build(),
					}
				}));
			}
		}
	}

	/// `op` on the sets of `group(metal, fire)`; false only for `Op::Warmed` with a set not built yet.
	pub(crate) fn op(&self, op: Op, metal: bool, fire: bool) -> bool {
		match op {
			Op::Prewarm => self.prewarm(metal, fire),
			Op::Warm => self.warm(metal, fire),
			Op::Warmed => return self.warmed(metal, fire),
		}
		true
	}
}

impl<T> std::ops::Deref for Pipes<T> {
	type Target = T;
	fn deref(&self) -> &T {
		self.get(set_of(on(), fls()))
	}
}


impl Gpu {
	/// Lane dlit2-9: this frame's fire-shadow set (`set_fls`: the light rows `rows` read a map). While a map can be drawn (`can`: Dynamic
	/// shadows with post; `gi` with fire maps) both fire-shadow sets of every pipeline that holds the lookup are queued for the background
	/// build (pipewarm.rs, from the first frame with fire lights; the title screen counts). Test build 7 (review of the load freeze): a
	/// frame whose rows read a map (`maps`) before the pool has built every set draws without the lookup (the lights still light, their
	/// shadows start a moment later) instead of building ~45 pipelines on this thread; with the test clock, or with the pool idle and a
	/// set still missing (a failed job), it builds them now (the same pictures every test run).
	/// Lane fshade worker 7: three states (`Fls`): rows that read a map but whose lights code none (`lit_maps` false) draw with the
	/// plain module's lookup-on sets (the pre-lane pipelines, byte-identical no-fire frames); the fire module's set is queued for the
	/// pool only from the first frame a light shades (no ~45 extra pipelines in levels whose maps never get a light) and the frame
	/// draws with the plain set until it is built (the pre-lane look; with the test clock or an idle pool it is built now).
	pub(super) fn fls_sets(&self, rows: &super::flit::FlitU, (_maps, can): (bool, bool), gi: u8) {
		super::pipewarm::tick();
		// Review W7 item 3 (accepted): Fire is chosen per frame, by the light rows (a map slot coded), not per pixel or by visibility.
		let lit = super::flit::lit_maps(rows) && !super::flitquad::fire_bad();
		let want = if !super::flit::reads_maps(rows) { Fls::Off } else if lit { Fls::Fire } else { Fls::Plain };
		let (m, fire) = (on(), want == Fls::Fire);
		if can && super::sunshadow::omni::lights(gi) > 0 {
			self.fls_op(Op::Prewarm, m, false);
			if fire {
				self.fls_op(Op::Prewarm, m, true);
			}
		}
		// Review W7 item 7: the built check runs on every path (RAC_TUNE flsh_lights > 0 at GI Low / Off reads maps with no prewarm):
		// a set that is not built is never made on this thread outside the test clock / an idle pool (so `maps` no longer gates it).
		set_fls(if want == Fls::Off { want } else { self.fls_ready(m, fire, want) });
	}

	/// The state a frame that wants `want` (not Off) draws with: built now with the test clock or an idle pool, else the lookup off
	/// until the plain sets are built and Plain until the fire set is (or for good once a fire set was refused, flitquad.rs `fire_bad`).
	fn fls_ready(&self, m: bool, fire: bool, want: Fls) -> Fls {
		if super::test_clock::clock_on() || super::pipewarm::idle() {
			self.fls_op(Op::Warm, m, false);
			if fire {
				self.fls_op(Op::Warm, m, true);
			}
		}
		if !self.fls_op(Op::Warmed, m, false) {
			Fls::Off
		} else if fire && (super::flitquad::fire_bad() || !self.fls_op(Op::Warmed, m, true)) {
			Fls::Plain
		} else {
			want
		}
	}

	/// `op` on the sets `group(m, fire)` of every pipeline that holds the lookup (Metal `m`; vegetation and fur use Metal's default
	/// set); false only for `Op::Warmed` with a set not built yet.
	fn fls_op(&self, op: Op, m: bool, fire: bool) -> bool {
		let (p, g, f) = (&self.props_gpu, self, fire);
		let level = [
			g.pipeline.op(op, m, f), g.pipeline_cull.op(op, m, f), g.moby_pipelines.op(op, m, f), g.moby_cull.op(op, m, f), g.shrub_pipelines.op(op, m, f),
			g.tfrag_pipeline.op(op, m, f), g.tfrag_pipeline_lt.op(op, m, f), g.tie_pipelines.op(op, m, f),
			p.pipeline.op(op, m, f), p.pipeline_cull.op(op, m, f), p.moby_pipelines.op(op, m, f), p.moby_cull.op(op, m, f),
		];
		let bindless = self.bindless.as_ref().is_none_or(|b| [b.tie.op(op, m, f), b.pipeline.op(op, m, f), b.pipeline_cull.op(op, m, f), b.shrub_faded.op(op, m, f)].iter().all(|&x| x));
		let (veg, fur) = (self.veg.fls_op(op, f), self.fur.fls_op(op, f, &self.device));
		level.iter().all(|&x| x) && bindless && veg && fur
	}
}

/// A level pipeline maker: (set index, vertex and fragment entry points, blend, depth write, depth compare, face culling).
pub(crate) type Mk<'a> = &'a dyn Fn(&str, &str, Option<wgpu::BlendState>, bool, wgpu::CompareFunction, Option<wgpu::Face>) -> wgpu::RenderPipeline;

/// What the level pipelines (and the bindless ones, over their own layout and module) are made from.
#[derive(Clone)]
pub(crate) struct LevelMaker {
	pub device: wgpu::Device,
	pub layout: wgpu::PipelineLayout,
	/// The plain module (every set without `FIRE`) and the `FIRE` sets' (flitquad.rs `Modules`).
	pub mods: super::flitquad::Modules,
	pub format: wgpu::TextureFormat,
	pub depth_format: wgpu::TextureFormat,
}

impl LevelMaker {
	/// A level pipeline over the `Vertex` layout with set `set`'s fragment constants.
	#[allow(clippy::too_many_arguments)]
	pub(crate) fn prim(&self, set: usize, vs: &str, fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool, compare: wgpu::CompareFunction, primitive: wgpu::PrimitiveState) -> wgpu::RenderPipeline {
		let cache = super::pipewarm::cache();
		self.mods.with(set, |module| self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some(vs),
			layout: Some(&self.layout),
			vertex: wgpu::VertexState {
				module: module,
				entry_point: Some(vs),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout {
					array_stride: std::mem::size_of::<Vertex>() as u64,
					step_mode: wgpu::VertexStepMode::Vertex,
					attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4],
				})],
			},
			fragment: Some(wgpu::FragmentState {
				module: module,
				entry_point: Some(fs),
				compilation_options: wgpu::PipelineCompilationOptions { constants: constants(set), ..Default::default() },
				targets: &[Some(wgpu::ColorTargetState { format: self.format, blend, write_mask: wgpu::ColorWrites::ALL })],
			}),
			primitive,
			depth_stencil: Some(wgpu::DepthStencilState {
				format: self.depth_format,
				depth_write_enabled: Some(depth_write),
				depth_compare: Some(compare),
				stencil: Default::default(),
				bias: Default::default(),
			}),
			multisample: wgpu::MultisampleState { count: msaa(), ..Default::default() },
			multiview_mask: None,
			cache: cache.as_ref().map(|c| &c.cache),
		}))
	}

	/// The lazily built sets (Metal x fire shadows) of `set` (one of the set functions below; `z` = the retail depth test).
	pub(crate) fn pipes<T: 'static>(&self, set: fn(Mk, wgpu::CompareFunction) -> T, z: wgpu::CompareFunction) -> Pipes<T> {
		let m = self.clone();
		Pipes::new(move |s| set(&|vs, fs, blend, dw, cmp, cull| m.prim(s, vs, fs, blend, dw, cmp, wgpu::PrimitiveState { cull_mode: cull, ..Default::default() }), z))
	}
}

const ALPHA: Option<wgpu::BlendState> = Some(wgpu::BlendState::ALPHA_BLENDING);
const LE: wgpu::CompareFunction = wgpu::CompareFunction::LessEqual;

/// Opaque world draws.
pub(crate) fn opaque(mk: Mk, z: wgpu::CompareFunction) -> wgpu::RenderPipeline {
	mk("vs", "fs", None, true, z, None)
}

/// Moby packets whose VU1 cull mode is 2 (back) or 0 (front) (NOV-5.9a).
pub(crate) fn opaque_cull(mk: Mk, z: wgpu::CompareFunction) -> [wgpu::RenderPipeline; 2] {
	[wgpu::Face::Back, wgpu::Face::Front].map(|f| mk("vs", "fs", None, true, z, Some(f)))
}

/// The moby ABE passes (texel alpha pass a with Z writes, b without).
pub(crate) fn moby(mk: Mk, _z: wgpu::CompareFunction) -> [wgpu::RenderPipeline; 2] {
	[mk("vs", "fs_moby_a", ALPHA, true, LE, None), mk("vs", "fs_moby_b", ALPHA, false, LE, None)]
}

/// The ABE pass keeps each packet's VU1 cull mode, as the opaque pass does (a culled packet sends no back / front faces to the GS at
/// all, whatever the texel alpha): index 2 * (cull - 1) + pass.
pub(crate) fn moby_cull(mk: Mk, _z: wgpu::CompareFunction) -> [wgpu::RenderPipeline; 4] {
	let [[a, b], [c, d]] = [wgpu::Face::Back, wgpu::Face::Front].map(|f| [mk("vs", "fs_moby_a", ALPHA, true, LE, Some(f)), mk("vs", "fs_moby_b", ALPHA, false, LE, Some(f))]);
	[a, b, c, d]
}

/// REN-CULL-SHRUB: the faded shrub meshes with / without Z write (texel alpha x A >= 12). Retail shrub TEST_1 ZTST GEQUAL (BRIDGE-27)
/// = LessEqual here (as `moby`); Less dropped coplanar decal shrubs (Eudora dirt paths) along terrain edges, leaving thin lines.
pub(crate) fn shrub_faded(mk: Mk, _z: wgpu::CompareFunction) -> [wgpu::RenderPipeline; 2] {
	[mk("vs_shrub", "fs_shrub_ge12", ALPHA, true, LE, None), mk("vs_shrub", "fs_shrub_lt12", ALPHA, false, LE, None)]
}

/// `shrub_faded`, then the billboards: pass 1 with / without Z (>= 96), pass 2 never.
pub(crate) fn shrubs(mk: Mk, z: wgpu::CompareFunction) -> [wgpu::RenderPipeline; 5] {
	let [a, b] = shrub_faded(mk, z);
	[a, b, mk("vs", "fs_shrub_ge96", ALPHA, true, LE, None), mk("vs", "fs_shrub_lt96", ALPHA, false, LE, None), mk("vs", "fs_shrub_all", ALPHA, false, LE, None)]
}

/// Terrain fragments: explicit mip level from the per-draw K / top level (REN-TEX-RESIDENCY).
pub(crate) fn tfrag(mk: Mk, z: wgpu::CompareFunction) -> wgpu::RenderPipeline {
	mk("vs_tfrag", "fs_tfrag", ALPHA, true, z, None)
}

/// Terrain texels below alpha 0x60 (AFAIL RGB_ONLY): blended, no z write; drawn after the other batches.
pub(crate) fn tfrag_lt(mk: Mk, z: wgpu::CompareFunction) -> wgpu::RenderPipeline {
	mk("vs_tfrag", "fs_tfrag_lt96", ALPHA, false, z, None)
}

/// Ties (TieProc TEST_1 0x5340b, ALPHA_1 0x44): texels from alpha 0x40 blend and write Z, the rest blend without Z (drawn after the
/// other batches, as `tfrag_lt`); per pass no cull, back, front (the packet's VU1 cull mode, `Draw::cull`).
pub(crate) fn ties(mk: Mk, z: wgpu::CompareFunction) -> [[wgpu::RenderPipeline; 3]; 2] {
	[("fs_tie", true), ("fs_tie_lt64", false)].map(|(fs, dw)| [None, Some(wgpu::Face::Back), Some(wgpu::Face::Front)].map(|c| mk("vs", fs, ALPHA, dw, z, c)))
}

/// The level's twice-kept sets (`Gpu` fields of the same names).
pub(crate) struct LevelSets {
	pub pipeline: Pipes<wgpu::RenderPipeline>,
	pub pipeline_cull: Pipes<[wgpu::RenderPipeline; 2]>,
	pub moby_pipelines: Pipes<[wgpu::RenderPipeline; 2]>,
	pub moby_cull: Pipes<[wgpu::RenderPipeline; 4]>,
	pub shrub_pipelines: Pipes<[wgpu::RenderPipeline; 5]>,
	pub tfrag_pipeline: Pipes<wgpu::RenderPipeline>,
	pub tfrag_pipeline_lt: Pipes<wgpu::RenderPipeline>,
	pub tie_pipelines: Pipes<[[wgpu::RenderPipeline; 3]; 2]>,
}

pub(crate) fn level_sets(m: &LevelMaker, z: wgpu::CompareFunction) -> LevelSets {
	LevelSets {
		pipeline: m.pipes(opaque, z),
		pipeline_cull: m.pipes(opaque_cull, z),
		moby_pipelines: m.pipes(moby, z),
		moby_cull: m.pipes(moby_cull, z),
		shrub_pipelines: m.pipes(shrubs, z),
		tfrag_pipeline: m.pipes(tfrag, z),
		tfrag_pipeline_lt: m.pipes(tfrag_lt, z),
		tie_pipelines: m.pipes(ties, z),
	}
}

/// Test helper: the level shader source `src` through naga's HLSL writer (DX12 parity) with the Metal `on` set's overrides applied
/// (fire-shadow lookup in).
#[cfg(all(test, windows))]
pub(crate) fn hlsl(src: &str, on: bool) -> String {
	hlsl_set(src, constants(set(on, true)))
}

/// `hlsl` with the pipeline constants `pc`.
#[cfg(all(test, windows))]
pub(crate) fn hlsl_set(src: &str, pc: &[(&str, f64)]) -> String {
	let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
	let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	let pc: naga::back::PipelineConstants = pc.iter().map(|(k, v)| (k.to_string(), *v)).collect();
	let (m, info) = naga::back::pipeline_constants::process_overrides(&m, &info, None, &pc).expect("overrides");
	let (options, pipeline) = (naga::back::hlsl::Options::default(), naga::back::hlsl::PipelineOptions::default());
	let mut out = String::new();
	naga::back::hlsl::Writer::new(&mut out, &options, &pipeline).write(&m, &info, None).expect("HLSL writes");
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::AtomicU32;

	#[test]
	fn each_set_is_built_once_on_first_use_and_deref_follows_the_pass() {
		let built = Arc::new([AtomicU32::new(0), AtomicU32::new(0)]);
		let b = built.clone();
		let p = Pipes::new(move |s| {
			b[s & 1].fetch_add(1, Ordering::Relaxed);
			s
		});
		let count = || built.iter().map(|n| n.load(Ordering::Relaxed)).collect::<Vec<_>>();
		assert_eq!(count(), [0, 0], "nothing before the first use");
		let (off, on) = (set(false, false), set(true, false));
		assert_eq!((*p.get(off), *p.get(off), *p.get(on), *p.get(on)), (off, off, on, on));
		assert_eq!(count(), [1, 1], "each set once");
		set_on(false);
		let a = *p;
		set_on(true);
		assert_eq!((a, *p), (off, on), "Deref reads the current pass's set");
		p.warm(true, false);
		assert_eq!(count(), [1, 2], "warm builds the fire-shadow set of the same Metal set");
		set_fls(Fls::Plain);
		assert_eq!((*p, *p.fls()), (set(true, true), set(true, true)), "the frame's fire-shadow state picks the set");
		set_fls(Fls::Fire);
		assert_eq!((*p, *p.fls()), (set(true, true) | FIRE, set(true, true) | FIRE), "the fire module's set");
		set_fls(Fls::Off);
		assert_eq!(*p.fls(), set(true, false));
	}

	/// Worker 7: the fire module only for the `FIRE` sets; the constants of a fire set are its plain twin's.
	#[test]
	fn the_fire_sets_differ_from_their_plain_twins_by_the_module_only() {
		for m in [false, true] {
			assert_eq!(group(m, false), [set(m, false), set(m, true)]);
			assert_eq!(group(m, true), [set(m, true) | FIRE; 2]);
			assert_eq!(constants(set_of(m, Fls::Fire)), constants(set_of(m, Fls::Plain)));
			assert_eq!(fls_constants(set_of(m, Fls::Fire)), fls_constants(set_of(m, Fls::Plain)));
			assert_eq!(set_of(m, Fls::Off) & FIRE, 0);
			assert_eq!(set_of(m, Fls::Plain) & FIRE, 0);
		}
	}

	#[test]
	fn off_constants_name_the_metal_override() {
		if !cut_old() {
			assert!(constants(set(true, true)).is_empty(), "On = the shader as written");
			assert_eq!(constants(set(true, false)), &[("FLS_ON", 0.0)]);
		}
		assert_eq!(constants(set(false, true)), &[("MK_ON", 0.0)]);
		assert_eq!(constants(set(false, false)), &[("MK_ON", 0.0), ("FLS_ON", 0.0)]);
		assert_eq!((fls_constants(set(true, true)), fls_constants(set(true, false))), (&[][..], &[("FLS_ON", 0.0)][..]));
		assert!(include_str!("flitshadow.wgsl").contains("override FLS_ON: bool = true;"));
		assert!(super::super::metalfx::METAL_WGSL.contains("override MK_ON: bool = true;"));
		assert!(super::super::metalfx::METAL_WGSL.contains("override MK_CUT: bool = true;"));
	}

	/// tb5 review N4: every module a pipeline gets the fire-shadow constants for declares `override FLS_ON` (wgpu rejects a pipeline
	/// constant the module does not declare: naga `process_overrides` NotFound): the level and props sources (one module, gpu/setup.rs)
	/// and bindless with `constants`, vegetation and fur with `fls_constants`; every set, both forced-early-depth choices.
	#[test]
	fn every_module_takes_the_fire_shadow_constants() {
		for early in [false, true] {
			let level = super::super::volfog::soft::level_source(super::super::SHADER, early);
			let sources: [(&str, String, fn(usize) -> &'static [(&'static str, f64)]); 5] = [
				("level", level.clone(), constants),
				("props", level.clone(), constants),
				("bindless", super::super::bindless::shader_source(&level), constants),
				("vegetation", super::super::vegfx::pipes::source_with(early), fls_constants),
				("fur", super::super::furfx::pipes::source_with(early), fls_constants),
			];
			for (name, src, pcs) in &sources {
				let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
				let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{name}: {e:?}"));
				for s in 0..4 {
					let pc: naga::back::PipelineConstants = pcs(s).iter().map(|(k, v)| (k.to_string(), *v)).collect();
					if let Err(e) = naga::back::pipeline_constants::process_overrides(&m, &info, None, &pc) {
						panic!("{name} (early {early}) set {s}: {e}");
					}
				}
				assert!(pcs(set(true, false)).contains(&("FLS_ON", 0.0)), "{name}: the lookup-off set names FLS_ON");
			}
		}
	}

	/// DX12 parity: the level shader with each set's constants (the fire-shadow lookup out too) goes through naga's HLSL writer.
	#[cfg(windows)]
	#[test]
	fn every_set_writes_hlsl() {
		let src = super::super::volfog::soft::level_source(super::super::SHADER, false);
		for s in 0..4 {
			assert!(hlsl_set(&src, constants(s)).contains("fs_tie"), "set {s}");
		}
	}

	/// Test build 7 (review of the load freeze): `Op::Warmed` is false until both fire-shadow sets of the Metal setting are built,
	/// `Op::Warm` builds exactly those two, `Op::Prewarm` builds the other two on the pool and never a set twice.
	#[test]
	fn ops_build_both_fire_shadow_sets_once() {
		let built = Arc::new(AtomicU32::new(0));
		let b = built.clone();
		let p = Pipes::new(move |s| {
			b.fetch_add(1, Ordering::SeqCst);
			s
		});
		assert!(!p.op(Op::Warmed, true, false));
		assert!(p.op(Op::Warm, true, false) && p.op(Op::Warmed, true, false) && !p.op(Op::Warmed, false, false) && !p.op(Op::Warmed, true, true));
		assert_eq!(built.load(Ordering::SeqCst), 2);
		assert!(p.op(Op::Prewarm, false, false) && p.op(Op::Prewarm, false, false) && p.op(Op::Prewarm, true, false) && p.op(Op::Prewarm, true, true));
		let t = std::time::Instant::now();
		while !(p.op(Op::Warmed, false, false) && p.op(Op::Warmed, true, true) && super::super::pipewarm::idle()) {
			assert!(t.elapsed().as_secs() < 20, "the pool builds the queued sets");
			std::thread::sleep(std::time::Duration::from_millis(5));
		}
		assert_eq!(built.load(Ordering::SeqCst), 5, "each set built once (the fire group is one set)");
		assert_eq!(*p.get(set(false, false)), set(false, false));
	}
}
