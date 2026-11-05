//! [PC] Metal On / Off pipeline sets (review R24 M3). The level shader holds the metal look (metalfx.wgsl) behind the override
//! `MK_ON`: the On set is built with it true (the shader as written), the Off set with it false, so Metal Off and Lighting Classic run
//! the level shader without the metal code (no register pressure from it on the Steam Deck). `metal_look` returns the retail colour at
//! its first lines in both cases, so the Off picture is the same.
//!
//! Each `Pipes` builds a set on its first use (a game started with Metal Off never builds the On set and the other way round);
//! `Deref` reads the set of the current pass (`set_on`, prepare.rs `frame_uniforms`: a split frame's Classic pass is recorded and
//! submitted before its Enhanced pass, so one flag per prepare is right). Only the pipelines whose fragment entry points can reach
//! `metal_look` are kept twice: the level's opaque, cull, moby, shrub, tfrag and tie sets (Gpu, bindless.rs `Bindless`, gpuskin.rs
//! `PropsGpu`); the others are built once with the default (On).
//! Split view: the set follows the pass (`set_on` from prepare.rs per pass), so the Classic half runs the Off set.

use super::*;
use std::cell::OnceCell;
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(true);

/// The set the next passes draw with: Metal On in this pass (`u.keep_tail.y` > 0).
pub(crate) fn set_on(on: bool) {
	ON.store(on, Ordering::Relaxed);
}

fn on() -> bool {
	ON.load(Ordering::Relaxed)
}

/// The fragment stage's pipeline constants of a set (wgpu `PipelineCompilationOptions::constants`; a bool override takes 0 / 1).
pub(crate) fn constants(on: bool) -> &'static [(&'static str, f64)] {
	if on { &[] } else { &[("MK_ON", 0.0)] }
}

/// One pipeline (or array of them) per set, each built on its first use.
pub(crate) struct Pipes<T> {
	sets: [OnceCell<T>; 2],
	make: Box<dyn Fn(bool) -> T + Send>,
}

impl<T> Pipes<T> {
	pub(crate) fn new(make: impl Fn(bool) -> T + Send + 'static) -> Pipes<T> {
		Pipes { sets: [OnceCell::new(), OnceCell::new()], make: Box::new(make) }
	}

	/// The On (`on`) or Off set, built now if it is not yet.
	pub(crate) fn get(&self, on: bool) -> &T {
		self.sets[on as usize].get_or_init(|| (self.make)(on))
	}
}

impl<T> std::ops::Deref for Pipes<T> {
	type Target = T;
	fn deref(&self) -> &T {
		self.get(on())
	}
}

/// A level pipeline maker: (set On, vertex and fragment entry points, blend, depth write, depth compare, face culling).
pub(crate) type Mk<'a> = &'a dyn Fn(&str, &str, Option<wgpu::BlendState>, bool, wgpu::CompareFunction, Option<wgpu::Face>) -> wgpu::RenderPipeline;

/// What the level pipelines (and the bindless ones, over their own layout and module) are made from.
#[derive(Clone)]
pub(crate) struct LevelMaker {
	pub device: wgpu::Device,
	pub layout: wgpu::PipelineLayout,
	pub shader: wgpu::ShaderModule,
	pub format: wgpu::TextureFormat,
	pub depth_format: wgpu::TextureFormat,
}

impl LevelMaker {
	/// A level pipeline over the `Vertex` layout with the `on` set's fragment constants.
	#[allow(clippy::too_many_arguments)]
	pub(crate) fn prim(&self, on: bool, vs: &str, fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool, compare: wgpu::CompareFunction, primitive: wgpu::PrimitiveState) -> wgpu::RenderPipeline {
		self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some(vs),
			layout: Some(&self.layout),
			vertex: wgpu::VertexState {
				module: &self.shader,
				entry_point: Some(vs),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout {
					array_stride: std::mem::size_of::<Vertex>() as u64,
					step_mode: wgpu::VertexStepMode::Vertex,
					attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4],
				})],
			},
			fragment: Some(wgpu::FragmentState {
				module: &self.shader,
				entry_point: Some(fs),
				compilation_options: wgpu::PipelineCompilationOptions { constants: constants(on), ..Default::default() },
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
			multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
			multiview_mask: None,
			cache: None,
		})
	}

	/// The lazily built On / Off sets of `set` (one of the set functions below; `z` = the retail depth test).
	pub(crate) fn pipes<T: 'static>(&self, set: fn(Mk, wgpu::CompareFunction) -> T, z: wgpu::CompareFunction) -> Pipes<T> {
		let m = self.clone();
		Pipes::new(move |on| set(&|vs, fs, blend, dw, cmp, cull| m.prim(on, vs, fs, blend, dw, cmp, wgpu::PrimitiveState { cull_mode: cull, ..Default::default() }), z))
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

/// Test helper: the level shader source `src` through naga's HLSL writer (DX12 parity) with the `on` set's overrides applied.
#[cfg(all(test, windows))]
pub(crate) fn hlsl(src: &str, on: bool) -> String {
	let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
	let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	let pc: naga::back::PipelineConstants = constants(on).iter().map(|(k, v)| (k.to_string(), *v)).collect();
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
	use std::sync::Arc;

	#[test]
	fn each_set_is_built_once_on_first_use_and_deref_follows_the_pass() {
		let built = Arc::new([AtomicU32::new(0), AtomicU32::new(0)]);
		let b = built.clone();
		let p = Pipes::new(move |on| {
			b[on as usize].fetch_add(1, Ordering::Relaxed);
			if on { 1 } else { 0 }
		});
		let count = || built.iter().map(|n| n.load(Ordering::Relaxed)).collect::<Vec<_>>();
		assert_eq!(count(), [0, 0], "nothing before the first use");
		assert_eq!((*p.get(false), *p.get(false), *p.get(true), *p.get(true)), (0, 0, 1, 1));
		assert_eq!(count(), [1, 1], "each set once");
		set_on(false);
		let off = *p;
		set_on(true);
		assert_eq!((off, *p), (0, 1), "Deref reads the current pass's set");
	}

	#[test]
	fn off_constants_name_the_metal_override() {
		assert!(constants(true).is_empty(), "On = the shader as written");
		assert_eq!(constants(false), &[("MK_ON", 0.0)]);
		assert!(super::super::metalfx::METAL_WGSL.contains("override MK_ON: bool = true;"));
	}
}
