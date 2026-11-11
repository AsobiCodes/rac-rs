//! [PC] Lane fshade, user 2026-11-08 ("Yes, both", after worker 3's crops): the fire-shadow lookup made cheaper where a fire shadow
//! map is read. (a) The filter stage of flitshadow.wgsl's `fls_cone` takes the diagonal pair of taps only (2 instead of 4; the
//! blocker search keeps 4; RAC_TUNE flit_debug -2 = 4 taps, dev A/B). (b) Where the device has wgpu `Features::SUBGROUP` (Vulkan
//! with quad ops in fragment shaders, DX12 with DXC shader model 6) the term is shared per 2x2 pixel quad (flitquad.wgsl
//! `flit_quad`: one lookup per pixel instead of up to 4; RAC_TUNE fsh_quad=0 at start = never).
//!
//! Both live ONLY in the "fire" module of each maker (`Modules`): the level, level bindless, fur and vegetation pipelines of the
//! fire-shadow sets (metalpipe.rs `set(_, true)`) are built from `fire_source` (the plain source with (a), plus (b) on a subgroup
//! device), every other pipeline (the sets without the lookup, the single-set pipelines: lines, particles, props extras) from the
//! plain source, which is the text from before the lane. So a frame that reads no fire shadow map draws with exactly the
//! pipelines it had before (worker 6: ANY edit of the shared text, even dead code behind `FLS_ON = false`, moved texels in no-fire
//! scenes on NVIDIA, up to 18 levels in a test level: the driver's code generation follows the whole module).
//!
//! Exposure (review W4 finding 8): WGSL wants `enable subgroups;` and subgroup ops in uniform control flow; naga 30 enforces
//! neither (`flit_quad` runs after discards and per-pixel branches). A naga upgrade may reject the twin: `device_created`'s probe
//! then turns (b) off (the fire modules fall back to (a) per pixel), and the unit tests here fail first.

use std::sync::atomic::{AtomicBool, Ordering};

/// The device was made with `Features::SUBGROUP` and the probe pipeline built (`device_created`): fire modules take (b).
static ON: AtomicBool = AtomicBool::new(false);

/// RAC_TUNE fsh_quad (dev): 0 = never ask for subgroups (the fallback path on any device).
fn wanted() -> bool {
	crate::devtune::f32("fsh_quad", 1.0) > 0.5
}

/// The feature the window's device asks for: SUBGROUP only when the adapter has it (device creation never fails for it).
pub(super) fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
	if wanted() { adapter.features() & wgpu::Features::SUBGROUP } else { wgpu::Features::empty() }
}

/// (b) is on: the fire modules are the subgroup twins.
pub(in crate::viewer) fn on() -> bool {
	ON.load(Ordering::Relaxed)
}

/// After `request_device`, before any level / fur / vegetation module and before pipewarm's cache key: whether the fire modules
/// take (b). Review W4 finding 1: one small pipeline with flitquad.wgsl's exact subgroup code is built inside error scopes first
/// (DX12 compiles HLSL with DXC at pipeline creation; a driver or DXC that refuses the wave / quad ops would otherwise fail the
/// level pipelines); a refusal leaves (b) off, one log line.
pub(super) fn device_created(device: &wgpu::Device) {
	let has = wanted() && device.features().contains(wgpu::Features::SUBGROUP);
	let probe = if has { probe(device) } else { Ok(()) };
	ON.store(has && probe.is_ok(), Ordering::Relaxed);
	match probe {
		Err(e) => eprintln!("flitquad: subgroup probe refused, fire shadows per pixel: {}", e.lines().next().unwrap_or("")),
		Ok(()) => eprintln!("flitquad: fire shadows {}", if has { "shared per 2x2 quad (subgroups)" } else { "per pixel (no subgroups)" }),
	}
}

/// The probe's fragment code around flitquad.wgsl (`flit_shadow` stands in for flitshadow.wgsl's lookup).
const PROBE: &str = "
fn flit_shadow(k: u32, w: vec3<f32>, n: vec3<f32>, has_n: bool) -> f32 { return w.x + f32(k) + select(0.0, n.z, has_n); }
@vertex fn vs(@builtin(vertex_index) v: u32) -> @builtin(position) vec4<f32> { return vec4<f32>(f32(v & 1u), f32(v >> 1u), 0.0, 1.0); }
@fragment fn fs(@builtin(position) p: vec4<f32>, @builtin(subgroup_invocation_id) flq_in: u32) -> @location(0) vec4<f32> {
	flq_sgi = flq_in;
	return flit_quad(p.xyz, vec3<f32>(0.0, 0.0, 1.0), true, 1u);
}
";

fn probe_source() -> String {
	format!("{}{PROBE}", include_str!("flitquad.wgsl"))
}

fn probe(device: &wgpu::Device) -> Result<(), String> {
	let (sv, si) = (device.push_error_scope(wgpu::ErrorFilter::Validation), device.push_error_scope(wgpu::ErrorFilter::Internal));
	let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("flitquad probe"), source: wgpu::ShaderSource::Wgsl(probe_source().into()) });
	let _p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
		label: Some("flitquad probe"),
		layout: None,
		vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
		fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
		primitive: Default::default(),
		depth_stencil: None,
		multisample: Default::default(),
		multiview_mask: None,
		cache: None,
	});
	// Popped last in, first out (error scopes are per thread).
	let (ei, ev) = (pollster::block_on(si.pop()), pollster::block_on(sv.pop()));
	match ei.or(ev) {
		Some(e) => Err(e.to_string()),
		None => Ok(()),
	}
}

/// Review W7 item 4: a fire set's pipeline (or the fire module) was refused (error scopes in `Modules::with`): `Gpu::fls_sets`
/// keeps Fire at Plain from then on (one log line).
static BAD: AtomicBool = AtomicBool::new(false);

/// A fire set was refused this session (`Modules::with`).
pub(in crate::viewer) fn fire_bad() -> bool {
	BAD.load(Ordering::Relaxed)
}

/// The plain and the fire module of one maker (level, level bindless, fur, vegetation).
#[derive(Clone)]
pub(crate) struct Modules {
	/// The source as before the lane: the sets without the lookup and every single-set pipeline.
	pub plain: wgpu::ShaderModule,
	/// `fire_source`: the fire-shadow sets' module, made on the first `FIRE` set (review W7 item 6: no second ~130 KB parse at start
	/// or on the render thread at the first fur draw; GI Low / Off and Classic never make it).
	fire: std::sync::Arc<Fire>,
}

struct Fire {
	device: wgpu::Device,
	label: String,
	src: String,
	module: std::sync::OnceLock<wgpu::ShaderModule>,
}

fn make(device: &wgpu::Device, label: &str, src: String) -> wgpu::ShaderModule {
	device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl(label, src.into())) })
}

impl Modules {
	/// The plain module of `src` now, the fire one on first use (RAC_WGSL_DIR / RAC_WGSL_DUMP labels `label` and `label fire`).
	pub(in crate::viewer) fn new(device: &wgpu::Device, label: &str, src: String) -> Modules {
		let plain = make(device, label, src.clone());
		Modules { plain, fire: std::sync::Arc::new(Fire { device: device.clone(), label: format!("{label} fire"), src, module: Default::default() }) }
	}

	/// The module of metalpipe.rs set `set`: the fire module for the `FIRE` sets only (worker 7: the plain lookup-on sets draw the frames
	/// with maps but no light that reads one, byte-identical to the pre-lane pipelines).
	pub(in crate::viewer) fn of(&self, set: usize) -> &wgpu::ShaderModule {
		if set & super::metalpipe::FIRE == 0 {
			return &self.plain;
		}
		let f = &*self.fire;
		f.module.get_or_init(|| make(&f.device, &f.label, fire_source(&f.label, &f.src)))
	}

	/// `build` with set `set`'s module. A `FIRE` set (its module made on first use) is built inside Validation + Internal error scopes
	/// (per thread: the pool's on prewarm); an error sets `fire_bad` (review W7 item 4: a driver or compiler that refuses only the big
	/// fire module would otherwise leave the level without draws in every fire-lit frame).
	pub(in crate::viewer) fn with<T>(&self, set: usize, build: impl FnOnce(&wgpu::ShaderModule) -> T) -> T {
		if set & super::metalpipe::FIRE == 0 {
			return build(&self.plain);
		}
		let d = &self.fire.device;
		let (sv, si) = (d.push_error_scope(wgpu::ErrorFilter::Validation), d.push_error_scope(wgpu::ErrorFilter::Internal));
		let t = build(self.of(set));
		let (ei, ev) = (pollster::block_on(si.pop()), pollster::block_on(sv.pop()));
		if let Some(e) = ei.or(ev) {
			if !BAD.swap(true, Ordering::Relaxed) {
				eprintln!("flitquad: {} set refused, fire shadows stay on the plain lookup: {}", self.fire.label, e.to_string().lines().next().unwrap_or(""));
			}
		}
		t
	}
}

/// (a) as edits of flitshadow.wgsl's text: `fls_cone` takes `two` (the diagonal pair (-1, -1), (+1, +1), mean of 2), the blocker
/// search passes false, the filter stage true unless FLIT_SHAPE .y < -1.5 (flit_debug -2).
const CONE: (&str, &str) = ("fn fls_cone(k: u32, r: FlsRecv, phi: f32) -> vec2<f32> {", "fn fls_cone(k: u32, r: FlsRecv, phi: f32, two: bool) -> vec2<f32> {");
const TAP_LOOP: &str = "for (var i = 0u; i < 4u; i++) {";
const TAP_SKIP: &str = " if (two && (i == 1u || i == 2u)) { continue; }";
const CONE_RET: (&str, &str) = ("return acc * 0.25;", "return acc * select(0.25, 0.5, two);");
const CALLS: [(&str, &str); 2] = [
	("let bs = fls_cone(k, r, cs);", "let bs = fls_cone(k, r, cs, false);"),
	("let lit = 1.0 - fls_cone(k, r, phi).x;", "let lit = 1.0 - fls_cone(k, r, phi, u.flit[FLIT_SHAPE].y > -1.5).x;"),
];
/// (b): flitquad.wgsl goes in before flit.wgsl's `flit_light`, which calls it before its light loop (FLIT_SHAPE .y > -0.5: -1 / -2
/// = the per-pixel reads, dev A/B; a -1 result = the lazy per-pixel reads).
const LIGHT: &str = "fn flit_light(";
const SHC: &str = "var shc = vec4<f32>(-1.0);";
const QUAD_CALL: &str = " if (FLS_ON && sh.y > -0.5) { shc = flit_quad(w, n0 / max(ln, 1e-6), ln > 1e-4, m); }";
const ID: &str = ", @builtin(subgroup_invocation_id) flq_in: u32)";

/// `src` with `from` (exactly once) replaced by `to`.
fn once(src: &str, from: &str, to: &str) -> Option<String> {
	(src.matches(from).count() == 1).then(|| src.replacen(from, to, 1))
}

/// (a) on `src`, plus (b) when `quads`; None where an anchor is missing or not unique (the text moved on: the tests fail).
pub(in crate::viewer) fn fire_text(src: &str, quads: bool) -> Option<(String, usize, usize)> {
	let mut s = once(src, CONE.0, CONE.1)?;
	let f = s.find(CONE.1)?;
	let l = f + s[f..].find(TAP_LOOP)? + TAP_LOOP.len();
	s.insert_str(l, TAP_SKIP);
	let r = f + s[f..].find(CONE_RET.0)?;
	s.replace_range(r..r + CONE_RET.0.len(), CONE_RET.1);
	for (a, b) in CALLS {
		s = once(&s, a, b)?;
	}
	if !quads {
		return Some((s, 0, 0));
	}
	s = once(&s, LIGHT, &format!("{}\n{LIGHT}", include_str!("flitquad.wgsl")))?;
	s = once(&s, SHC, &format!("{SHC}{QUAD_CALL}"))?;
	Some(entries(&s))
}

/// Every one-line `@fragment` entry header taking `i: VOut` / `i: VegOut` and ending in `{` takes the subgroup invocation id and
/// stores it in `flq_sgi` first; (text, entries patched, fragment entries). An entry left out keeps `flq_sgi` at its start value,
/// so `flit_quad` returns -1 there (per pixel: correct, no gain); the tests check every fire-lit entry through naga.
fn entries(src: &str) -> (String, usize, usize) {
	let (mut out, mut n, mut all) = (String::with_capacity(src.len() + 4096), 0, 0);
	for line in src.split_inclusive('\n') {
		let body = line.trim_end_matches(['\n', '\r']);
		all += body.contains("@fragment") as usize;
		let arg = ["(i: VOut)", "(i: VegOut)"].into_iter().find(|a| body.contains(a));
		match arg {
			Some(a) if body.contains("@fragment") && body.trim_end().ends_with('{') => {
				out.push_str(&body.replacen(a, &format!("{}{ID}", &a[..a.len() - 1]), 1));
				out.push_str(" flq_sgi = flq_in;");
				out.push_str(&line[body.len()..]);
				n += 1;
			}
			_ => out.push_str(line),
		}
	}
	(out, n, all)
}

/// The fire module's source of `src` on this device (`label` for the log): (a), plus (b) when `on`. The plain source (= no change
/// at all) where the text no longer matches.
pub(in crate::viewer) fn fire_source(label: &str, src: &str) -> String {
	match fire_text(src, on()) {
		Some((s, n, all)) => {
			if on() {
				eprintln!("flitquad: {label}: quad id in {n} of {all} fragment entries");
			}
			s
		}
		None => {
			eprintln!("flitquad: {label}: the fire-shadow edits do not match the source, plain lookup");
			src.to_string()
		}
	}
}

#[cfg(test)]
#[path = "flitquad_tests.rs"]
mod tests;
