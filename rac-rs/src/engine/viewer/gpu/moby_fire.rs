//! [PC] Fire option, mesh-bound flames: the moby classes a game pack names as fire (`Pack::fire_mobys`: a fireball's sphere, a pilot
//! flame) draw where their meshes draw today, but with the realistic flame look instead of the plain texture.
//!
//! How: a shown prop of such a class takes the additive moby slot machinery of `moby_add.rs` (it leaves the opaque and blended
//! draws, the CPU clipper and the sun shadow caster list the same way) and is drawn by the two entry points below, additive
//! (Cs * As + Cd), no face culling, the same As = 96 split into pass a (writes Z) and pass b as `fs_moby_add_a` / `_b`. The mesh's
//! own lit texel times its cover (the light retail adds there) is the starting heat, so the flame keeps retail's extent and fade, and it
//! fades out towards the mesh's silhouette (no bubble outline, `mf_rim`); a turbulence field of the
//! world position scrolling upward (`mf_fbm`, octaves by the Fire level: Low 2, Medium 3, High 4) breaks it up and moves it;
//! the heat then runs through a black-red, orange, yellow, white ramp. The clock and the level ride in `keep_tail.z` / `.w`
//! of the level uniforms (`words`, zero while Fire is off). Off = none of this runs and nothing is flagged: today's picture.
//! Dev, no rebuild: `RAC_FIRE_MOBY_WGSL=<file>` replaces the text of `WGSL` below (the entry points are appended to the level
//! shader by `volfog::soft::level_source`); all look numbers (heat gain, ramp, turbulence scale and speed) are in `moby_fire_look`.

use super::*;

/// The two fragment entry points appended to the level shader (`volfog::soft::level_source`).
pub(in crate::viewer) const WGSL: &str = r#"
const MF_PERIOD: f32 = 512.0;
// Heat per unit of the light retail adds (luminance x cover); the facing band over which a mesh flame fades out at its silhouette.
const MF_GAIN: f32 = 2.0;
const MF_RIM_LO: f32 = 0.25;
const MF_RIM_HI: f32 = 0.8;

fn mf_hash(p: vec3<f32>) -> f32 {
	var q = fract(p * 0.1031);
	q = q + dot(q, q.yzx + 33.33);
	return fract((q.x + q.y) * q.z);
}

fn mf_noise(p: vec3<f32>) -> f32 {
	let ip = floor(p);
	let f = fract(p);
	let s = f * f * (3.0 - 2.0 * f);
	let a = mix(mix(mf_hash(ip), mf_hash(ip + vec3<f32>(1.0, 0.0, 0.0)), s.x), mix(mf_hash(ip + vec3<f32>(0.0, 1.0, 0.0)), mf_hash(ip + vec3<f32>(1.0, 1.0, 0.0)), s.x), s.y);
	let b = mix(mix(mf_hash(ip + vec3<f32>(0.0, 0.0, 1.0)), mf_hash(ip + vec3<f32>(1.0, 0.0, 1.0)), s.x), mix(mf_hash(ip + vec3<f32>(0.0, 1.0, 1.0)), mf_hash(ip + vec3<f32>(1.0, 1.0, 1.0)), s.x), s.y);
	return mix(a, b, s.z);
}

fn mf_fbm(p: vec3<f32>, octaves: i32) -> f32 {
	var sum = 0.0;
	var amp = 0.5;
	var q = p;
	for (var k = 0; k < 4; k = k + 1) {
		if (k >= octaves) { break; }
		sum = sum + amp * mf_noise(q);
		q = q * 2.03 + vec3<f32>(1.7, 9.2, 3.1);
		amp = amp * 0.5;
	}
	return sum / (1.0 - pow(0.5, f32(octaves)));
}

fn mf_ramp(h: f32) -> vec3<f32> {
	let r = clamp(h * 1.6, 0.0, 1.0);
	let g = clamp(h * 1.6 - 0.45, 0.0, 1.0);
	let b = clamp(h * 1.7 - 1.15, 0.0, 1.0);
	return vec3<f32>(r, g * 0.78, b * 0.6);
}

fn mf_heat(w: vec3<f32>, base: f32, t: f32, octaves: i32) -> f32 {
	let n = mf_fbm(w * 2.2 + vec3<f32>(0.0, 0.0, -t * 2.4), octaves);
	// (The noise term scales with the heat: no faint glow where retail draws nothing.)
	return clamp(base * (0.45 + 1.1 * n) + (n - 0.5) * 0.25 * min(base * 4.0, 1.0), 0.0, 1.0);
}

// Flame look of a moby texel: `c` the mesh texture, `ra` its cover (As). Returns the additive colour and alpha. The heat is the light
// retail itself adds here (the lit texel times its cover, `moby_rgb` x As), so the flame is never drawn where retail's sphere is
// faint (no bigger than its bright part) and fades with it; and it falls to zero towards the mesh's silhouette (`rim`, the view
// against the normal), noise-torn, so a fireball's sphere never shows its outline as a ring.
fn moby_fire_look(i: VOut, c: vec4<f32>, ra: f32) -> vec4<f32> {
	let octaves = clamp(i32(u.keep_tail.w + 0.5) + 1, 2, 4);
	let t = u.keep_tail.z;
	let blend = fract(u.keep_tail.w) * 4.0;
	let base = clamp(dot(moby_lit_rgb(i, c), vec3<f32>(0.3, 0.59, 0.11)) * ra * MF_GAIN, 0.0, 1.0);
	var heat = mf_heat(i.w, base, t, octaves);
	// The clock wraps every period: its last seconds ease into the picture the wrap lands on (firefx_clock.rs).
	if (blend > 0.0) {
		heat = mix(heat, mf_heat(i.w, base, t - MF_PERIOD, octaves), blend);
	}
	heat = heat * mf_rim(i, heat);
	let d = distance(i.w, u.eye.xyz);
	let f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	return vec4<f32>(mf_ramp(heat) * 1.15 * f, smoothstep(0.2, 0.6, heat));
}

// 0 at the mesh's silhouette (the surface seen edge-on) to 1 where it faces the eye; the edge band is torn by the heat's own noise
// (hotter spots reach further out), so the fade is not a clean circle either.
fn mf_rim(i: VOut, heat: f32) -> f32 {
	let n2 = dot(i.n, i.n);
	if (n2 < 1.0e-8) {
		return 1.0;
	}
	let facing = abs(dot(i.n * inverseSqrt(n2), normalize(u.eye.xyz - i.w)));
	return smoothstep(MF_RIM_LO, MF_RIM_HI, facing + (heat - 0.5) * 0.2);
}

@fragment fn fs_moby_fire_a(i: VOut) -> @location(0) vec4<f32> {
	let c = moby_texel(i);
	let ra = moby_a(i, c);
	if (ra * 128.0 < 96.0) { discard; }
	// Pass a writes Z: no depth where the flame look is dark and nearly clear, or effects behind the fireball would be cut out (A25).
	let o = moby_fire_look(i, c, ra);
	if (o.a < 0.1) { discard; }
	return o;
}

@fragment fn fs_moby_fire_b(i: VOut) -> @location(0) vec4<f32> {
	let c = moby_texel(i);
	let ra = moby_a(i, c);
	if (ra * 128.0 >= 96.0) { discard; }
	return moby_fire_look(i, c, ra);
}
"#;

/// The entry points' text: `WGSL`, or the file `RAC_FIRE_MOBY_WGSL` names (dev switch; a missing file falls back to the built-in text).
pub(in crate::viewer) fn wgsl() -> String {
	std::env::var("RAC_FIRE_MOBY_WGSL").ok().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_else(|| WGSL.to_string())
}

type MakeProps<'a> = &'a dyn Fn(&str, Option<wgpu::BlendState>, bool, wgpu::CompareFunction, Option<wgpu::Face>) -> wgpu::RenderPipeline;

/// The two fire pipelines (index = pass: 0 = a, 1 = b), additive, no face culling, built with the caller's `make_props`.
pub(super) fn pipelines(make: MakeProps) -> [wgpu::RenderPipeline; 2] {
	let add = Some(super::moby_add::ADD_BLEND);
	[make("fs_moby_fire_a", add, true, wgpu::CompareFunction::LessEqual, None), make("fs_moby_fire_b", add, false, wgpu::CompareFunction::LessEqual, None)]
}

/// What the props pipelines are made from (setup.rs `make_props`): the props layout, the level shader, the colour and depth formats.
/// `PropsGpu` keeps it so the two flame pipelines are built only when a level first shows a flame moby with Fire on (review R7-9: Fire
/// Off compiles nothing, and a driver that refuses them turns Fire off through `pipe_guard` instead of failing the props set-up).
#[derive(Clone)]
pub(in crate::viewer) struct PropsMaker {
	pub layout: wgpu::PipelineLayout,
	/// The level modules: plain and the fire-shadow sets' (flitquad.rs `Modules`; `make` = plain: single-set pipelines).
	pub mods: crate::viewer::flitquad::Modules,
	pub format: wgpu::TextureFormat,
	pub depth_format: wgpu::TextureFormat,
}

impl PropsMaker {
	/// A props pipeline: `vs_props`, the fragment entry point `fs`, `blend`, depth write / compare and the face culling.
	pub(super) fn make(&self, device: &wgpu::Device, fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool, compare: wgpu::CompareFunction, cull: Option<wgpu::Face>) -> wgpu::RenderPipeline {
		self.build(device, &self.mods.plain, crate::viewer::metalpipe::set(true, true), fs, blend, depth_write, compare, cull)
	}

	/// The lazily built Metal x fire-shadow sets (metalpipe.rs) of `set` over the props layout (`vs_props`; the set's vertex entry is unused).
	pub(in crate::viewer) fn pipes<T: 'static>(&self, device: &wgpu::Device, set: fn(crate::viewer::metalpipe::Mk, wgpu::CompareFunction) -> T, z: wgpu::CompareFunction) -> crate::viewer::metalpipe::Pipes<T> {
		let (m, dev) = (self.clone(), device.clone());
		crate::viewer::metalpipe::Pipes::new(move |s| set(&|_vs, fs, blend, dw, cmp, cull| m.make_on(&dev, s, fs, blend, dw, cmp, cull), z))
	}

	/// `make` with set `set`'s fragment constants (metalpipe.rs `constants`).
	#[allow(clippy::too_many_arguments)]
	pub(in crate::viewer) fn make_on(&self, device: &wgpu::Device, set: usize, fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool, compare: wgpu::CompareFunction, cull: Option<wgpu::Face>) -> wgpu::RenderPipeline {
		self.mods.with(set, |module| self.build(device, module, set, fs, blend, depth_write, compare, cull))
	}

	/// A props pipeline from `module`.
	#[allow(clippy::too_many_arguments)]
	fn build(&self, device: &wgpu::Device, module: &wgpu::ShaderModule, set: usize, fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool, compare: wgpu::CompareFunction, cull: Option<wgpu::Face>) -> wgpu::RenderPipeline {
		let cache = crate::viewer::pipewarm::cache();
		device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("vs_props"),
			layout: Some(&self.layout),
			vertex: wgpu::VertexState {
				module,
				entry_point: Some("vs_props"),
				compilation_options: Default::default(),
				buffers: &[
					Some(wgpu::VertexBufferLayout {
						array_stride: std::mem::size_of::<Vertex>() as u64,
						step_mode: wgpu::VertexStepMode::Vertex,
						attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4],
					}),
					Some(wgpu::VertexBufferLayout { array_stride: 8, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![5 => Uint32x2] }),
				],
			},
			fragment: Some(wgpu::FragmentState {
				module,
				entry_point: Some(fs),
				compilation_options: wgpu::PipelineCompilationOptions { constants: crate::viewer::metalpipe::constants(set), ..Default::default() },
				targets: &[Some(wgpu::ColorTargetState { format: self.format, blend, write_mask: wgpu::ColorWrites::ALL })],
			}),
			primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
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
		})
	}
}

impl crate::viewer::gpuskin::PropsGpu {
	/// Builds the two flame pipelines on the first call (`Gpu::set_props` on the first frame a level shows a flame moby with Fire on)
	/// through `pipe_guard::guard(Effect::Fire, ..)`. False = refused: Fire is off from then on and the mobys stay retail's additive ones.
	pub(super) fn fire_ready(&mut self, device: &wgpu::Device) -> bool {
		if self.fire.is_none() {
			let maker = &self.maker;
			let make = |fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool, compare: wgpu::CompareFunction, cull: Option<wgpu::Face>| {
				maker.make(device, fs, blend, depth_write, compare, cull)
			};
			self.fire = crate::viewer::pipe_guard::guard(device, crate::viewer::pipe_guard::Effect::Fire, || pipelines(&make));
		}
		self.fire.is_some()
	}
}

/// The level uniforms' `keep_tail.z` / `.w` words: the fire clock in seconds and the Fire level (0 = Off). Zero when `on` is false.
/// `game`: the fire's draw clock (`FireFx::shown`, game time), so the mesh flames stand still in a pause like the other flames (review R7-7).
pub(super) fn words(on: bool, game: f64) -> [f32; 2] {
	if on { pack_words(super::super::firefx_clock::Clock::game(game), crate::fire_option::level()) } else { [0.0; 2] }
}

/// The two words: the clock in the period, and the Fire level plus a quarter of the blend weight (the shader reads `floor` as the
/// level and `fract * 4` as the blend of the pattern one period earlier, firefx_clock.rs).
fn pack_words(c: super::super::firefx_clock::Clock, level: u8) -> [f32; 2] {
	[c.t, level as f32 + 0.25 * c.blend]
}

/// The moby classes of loaded level `level` that draw as flames now: the game pack's `fire_mobys`, empty while Fire is off.
pub(super) fn classes(level: usize) -> &'static [u32] {
	if super::super::firefx::active() { (crate::pack::get().fire_mobys)(level) } else { &[] }
}

/// Marks in `out` (one entry per slot, cleared first; left empty when none) the shown props whose class is one of `classes`.
pub(super) fn mark_fire(out: &mut Vec<bool>, props: &[Prop], shown: &[bool], classes: &[u32]) {
	out.clear();
	if classes.is_empty() {
		return;
	}
	out.extend(props.iter().zip(shown).map(|(p, &s)| s && classes.contains(&p.class_id)));
	if !out.contains(&true) {
		out.clear();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn prop(class_id: u32) -> Prop {
		Prop { class_id, pos: [0.0; 3], rot: [0.0; 3], mirror: false, order: 0, shown: false, retail_drawn: false, cast: false, anim: crate::anim::PropAnim::new(0), scale: 1.0, tint: None, light: None, index: None, sched: Default::default(), kind: crate::prop::Kind::Retail { alpha: 0x80, ovr: None, add: false } }
	}

	#[test]
	fn only_shown_props_of_a_fire_class_are_marked() {
		let mut out = vec![true; 7];
		mark_fire(&mut out, &[prop(112), prop(1), prop(112), prop(179)], &[true, true, false, true], &[112, 179]);
		assert_eq!(out, vec![true, false, false, true]);
		mark_fire(&mut out, &[prop(1)], &[true], &[112, 179]);
		assert!(out.is_empty(), "none marked: no vector");
		mark_fire(&mut out, &[prop(112)], &[true], &[]);
		assert!(out.is_empty(), "no classes (Off, or a game without fire mobys): nothing");
	}

	#[test]
	fn off_writes_zero_words() {
		assert_eq!(words(false, 123.0), [0.0; 2]);
	}

	#[test]
	fn the_words_carry_the_clock_level_and_blend() {
		use super::super::super::firefx_clock::Clock;
		assert_eq!(pack_words(Clock { t: 12.5, blend: 0.0 }, 3), [12.5, 3.0]);
		let w = pack_words(Clock { t: 1.0, blend: 1.0 }, 2)[1];
		assert_eq!((w as i32, (w.fract() * 4.0 - 1.0).abs() < 1e-6), (2, true), "level 2 and blend 1");
	}

	/// DX12 parity: the level shader with the mesh flames also goes through naga's HLSL writer (metalenv's test covers the late-fog
	/// build; this one the early-fog build).
	#[cfg(windows)]
	#[test]
	fn the_level_shader_with_the_mesh_flames_writes_hlsl() {
		let src = crate::viewer::volfog::soft::level_source(crate::viewer::SHADER, true);
		let out = crate::viewer::metalpipe::hlsl(&src, true);
		assert!(out.contains("fs_moby_fire_a"), "the mesh flame entry point is in the HLSL");
	}

	#[test]
	fn the_shader_with_the_fire_entry_points_validates() {
		for early in [false, true] {
			let src = crate::viewer::volfog::soft::level_source(crate::viewer::SHADER, early);
			assert!(src.contains("fn fs_moby_fire_a(") && src.contains("fn fs_moby_fire_b("));
			for s in [src.clone(), crate::viewer::bindless::shader_source(&src)] {
				let m = naga::front::wgsl::parse_str(&s).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&s)));
				naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validates");
			}
		}
	}

	/// The size rule and the bubble ring (user 2026-11-03, RYNO fireball ~1.5x retail with its sphere's outline drawn): the heat is the
	/// light retail adds (lit texel x cover, no floor that lit the faint shell), the noise adds nothing where retail draws nothing, and
	/// the flame fades out towards the mesh's silhouette.
	#[test]
	fn the_mesh_flame_follows_retail_light_and_has_no_rim() {
		assert!(WGSL.contains("dot(moby_lit_rgb(i, c), vec3<f32>(0.3, 0.59, 0.11)) * ra * MF_GAIN"), "heat from the light retail adds");
		assert!(!WGSL.contains("* 1.5 + 0.15"), "no heat floor on faint texels");
		assert!(WGSL.contains("heat = heat * mf_rim(i, heat);") && WGSL.contains("fn mf_rim("), "fades out at the silhouette");
		assert!(WGSL.contains("(n - 0.5) * 0.25 * min(base * 4.0, 1.0)"), "no noise glow where retail is dark");
	}
}
