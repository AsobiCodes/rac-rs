//! [PC] Fire, ground fire after explosions, GPU half (the CPU half is firefx_blast.rs): one decal draw a frame inside the particle pass,
//! before the flames. groundfire.wgsl rebuilds the world point of every pixel from the depth the level wrote (the pass's soft-particle
//! block, group 2), keeps the pixels on a patch's plane and draws the scorch mark and the ember bed there. No geometry of its own, so it
//! follows slopes and steps and leaves walls, mobys and the hero alone. Fire Off / Low / Classic: no patch, so no pipeline, no draw.
//! `RAC_GROUNDFIRE_WGSL=<file>` loads the shader from disk (dev); a driver that refuses it turns only this effect off (`pipe_guard`).
//!
//! The uniform block (`ROWS` rows of 16 bytes; also the "small GPU buffer" stage 3 may bind, `FireFx::patch_buffer`, valid on frames the
//! fire drew; only the patches that are on screen are in it): row 0 = (patch count, clock seconds, scorch darkness, clock blend); rows
//! 1 + 4k.. for patch k < 16 = (position xyz, radius), (normal xyz, seed), (flame, scorch, 0, 0), (screen rectangle lo.x lo.y hi.x hi.y in
//! NDC); rows 65..67 = the inverse of the view-projection's x, y, w rows (xyz) with their offsets in .w; row 68 = the hero's feet (xyz) and
//! protect radius (0 = none); row 69 = (glow, plane tolerance, 0, 0).
//! The rectangle is made here, on the CPU (`screen_rect`): the patch's oriented box (`decal_box`: in the patch plane, so a slope does not
//! matter) clipped against the near plane, so a scorch under the camera costs its own pixels, not the whole screen.

use super::firefx::{FireEnv, FireFx};
use super::firefx_blast::{tangents, BlastFire, GroundFirePatch, Life, Phase, MAX_PATCHES};
use super::firefx_clock::Clock;
use super::*;
use crate::fire_option;

const WGSL: &str = include_str!("groundfire.wgsl");

/// Rows of the uniform block: head, four per patch, three inverse rows, hero, tuning.
pub(super) const ROWS: usize = 1 + 4 * MAX_PATCHES + 3 + 2;
const ROW_INV: usize = 1 + 4 * MAX_PATCHES;

/// The decal pipeline and its uniform block.
pub(super) struct GroundRes {
	pipeline: wgpu::RenderPipeline,
	ubuf: wgpu::Buffer,
	ubind: wgpu::BindGroup,
}

impl GroundRes {
	fn new(env: &FireEnv<'_>) -> GroundRes {
		let device = env.device;
		let src = match std::env::var_os("RAC_GROUNDFIRE_WGSL").and_then(|p| std::fs::read_to_string(p).ok()) {
			Some(s) => std::borrow::Cow::Owned(s),
			None => std::borrow::Cow::Borrowed(WGSL),
		};
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ground fire"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("groundfire", src)) });
		let own = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("ground fire"),
			entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
		});
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("ground fire"), size: (ROWS * 16) as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let ubind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ground fire"), layout: &own, entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }] });
		let l0 = env.pipeline.get_bind_group_layout(0);
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ground fire"), bind_group_layouts: &[Some(&l0), Some(&own), Some(env.partfx.soft_layout())], immediate_size: 0 });
		// Premultiplied colour (the scorch darkens, the embers add light); the glow target adds, as the fire's does.
		let pre = wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add } };
		let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("ground fire"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_decal"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState {
				module: &module,
				entry_point: Some("fs_decal"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState { format: env.format, blend: Some(pre), write_mask: wgpu::ColorWrites::ALL }), Some(wgpu::ColorTargetState { format: partfx::GLOW_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::ALL })],
			}),
			primitive: Default::default(),
			depth_stencil: Some(wgpu::DepthStencilState { format: env.depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }),
			multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
			multiview_mask: None,
			cache: None,
		});
		GroundRes { pipeline, ubuf, ubind }
	}
}

/// The inverse of the x, y and w rows of a column-major view-projection (`m[column][row]`), as three rows (xyz) with the rows' offsets
/// in .w: with `v = (x_ndc * w - off0, y_ndc * w - off1, w - off2)` the world point is `(row0 . v, row1 . v, row2 . v)`. None when singular.
pub(super) fn inverse_rows(m: &[[f32; 4]; 4]) -> Option<[[f32; 4]; 3]> {
	let src = [0usize, 1, 3];
	let a: [[f64; 3]; 3] = std::array::from_fn(|r| std::array::from_fn(|c| m[c][src[r]] as f64));
	let cof = [
		[a[1][1] * a[2][2] - a[1][2] * a[2][1], a[0][2] * a[2][1] - a[0][1] * a[2][2], a[0][1] * a[1][2] - a[0][2] * a[1][1]],
		[a[1][2] * a[2][0] - a[1][0] * a[2][2], a[0][0] * a[2][2] - a[0][2] * a[2][0], a[0][2] * a[1][0] - a[0][0] * a[1][2]],
		[a[1][0] * a[2][1] - a[1][1] * a[2][0], a[0][1] * a[2][0] - a[0][0] * a[2][1], a[0][0] * a[1][1] - a[0][1] * a[1][0]],
	];
	let det = a[0][0] * cof[0][0] + a[0][1] * cof[1][0] + a[0][2] * cof[2][0];
	if det.abs() < 1e-18 || !det.is_finite() {
		return None;
	}
	let off = [m[3][0], m[3][1], m[3][3]];
	Some(std::array::from_fn(|r| [(cof[r][0] / det) as f32, (cof[r][1] / det) as f32, (cof[r][2] / det) as f32, off[r]]))
}

/// The knobs that go to the shader: scorch darkness, glow, plane tolerance, hero protect radius.
struct Look {
	dark: f32,
	glow: f32,
	tol: f32,
	hero_r: f32,
}

fn look() -> Look {
	let t = crate::devtune::f32;
	Look { dark: t("gfire_dark", 0.8), glow: t("gfire_glow", 1.0), tol: t("gfire_tol", 1.0), hero_r: t("gfire_hero_r", 0.6) }
}

/// The plane tolerance of a patch (m): what `fs_decal` accepts above and below the plane is 1.6 times this.
fn plane_tol(radius: f32, look: &Look) -> f32 {
	look.tol * (0.3 + 0.12 * radius)
}

/// The corners of the box that holds every pixel `fs_decal` can still see of a patch: in the patch's own plane +-1.35 radius (the
/// scorch and the embers are gone at 1.0 radius plus the ragged edge's 0.35), and +-(1.6 tolerance + 0.1) m along the normal. Oriented,
/// so a slope (down to the 60 degrees `land` accepts) cuts nothing. Corner `c`: bit 0 = side along t1, bit 1 = along t2, bit 2 = normal.
fn decal_box(p: &GroundFirePatch, look: &Look) -> [[f32; 3]; 8] {
	let (t1, t2) = tangents(p.normal);
	let (a, b) = (1.35 * p.radius, 1.6 * plane_tol(p.radius, look) + 0.1);
	std::array::from_fn(|c| {
		let s = |bit: usize| if c & bit != 0 { 1.0 } else { -1.0 };
		std::array::from_fn(|k| p.pos[k] + t1[k] * s(1) * a + t2[k] * s(2) * a + p.normal[k] * s(4) * b)
	})
}

/// The x, y and w of a world point in clip space (`m` is column-major, `m[column][row]`).
fn xyw(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
	std::array::from_fn(|i| {
		let r = [0, 1, 3][i];
		m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r]
	})
}

/// The screen rectangle (lo.x, lo.y, hi.x, hi.y in NDC, clamped to the screen) that bounds the box `corners` (see `decal_box`), clipped
/// against the near plane (w = 0.05): the projected corners in front plus the points where the box edges cross that plane. None when
/// the box is behind the eye or off the screen (nothing to draw; never a full-screen fallback).
pub(super) fn screen_rect(m: &[[f32; 4]; 4], corners: &[[f32; 3]; 8]) -> Option<[f32; 4]> {
	const NEAR: f32 = 0.05;
	const EDGES: [(usize, usize); 12] = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
	let q: Vec<[f32; 3]> = corners.iter().map(|&p| xyw(m, p)).collect();
	let (mut lo, mut hi, mut any) = ([f32::MAX; 2], [f32::MIN; 2], false);
	let mut add = |x: f32, y: f32, w: f32| {
		(lo, hi, any) = ([lo[0].min(x / w), lo[1].min(y / w)], [hi[0].max(x / w), hi[1].max(y / w)], true);
	};
	for p in q.iter().filter(|p| p[2] >= NEAR) {
		add(p[0], p[1], p[2]);
	}
	for &(a, b) in &EDGES {
		let (pa, pb) = (q[a], q[b]);
		if (pa[2] >= NEAR) != (pb[2] >= NEAR) {
			// (The projection is linear in clip space: the crossing point is a lerp of the clip coordinates.)
			let s = (NEAR - pa[2]) / (pb[2] - pa[2]);
			add(pa[0] + (pb[0] - pa[0]) * s, pa[1] + (pb[1] - pa[1]) * s, NEAR);
		}
	}
	if !any || !(lo[0].is_finite() && lo[1].is_finite() && hi[0].is_finite() && hi[1].is_finite()) {
		return None;
	}
	let (lo, hi) = ([lo[0].clamp(-1.0, 1.0), lo[1].clamp(-1.0, 1.0)], [hi[0].clamp(-1.0, 1.0), hi[1].clamp(-1.0, 1.0)]);
	(hi[0] > lo[0] && hi[1] > lo[1]).then_some([lo[0], lo[1], hi[0], hi[1]])
}

/// A patch to draw this frame: the patch, its strengths and its screen rectangle.
type Lit = (GroundFirePatch, Phase, [f32; 4]);

/// The patches that get an instance: alive at `now` with a strength, and with a place on the screen (a patch behind the eye or off
/// screen costs nothing).
fn lit_patches(patches: &[GroundFirePatch], now: f64, life: &Life, m: &[[f32; 4]; 4], look: &Look) -> Vec<Lit> {
	let lit = |p: &GroundFirePatch| {
		let ph = p.phase(now, life).filter(|ph| ph.flame > 0.001 || ph.scorch > 0.001)?;
		Some((*p, ph, screen_rect(m, &decal_box(p, look))?))
	};
	patches.iter().filter_map(lit).take(MAX_PATCHES).collect()
}

/// The uniform rows of a frame (see the module doc for the layout): the patches in `lit` (up to `MAX_PATCHES`), the clock, the inverse
/// rows and the hero (None = no protection).
fn pack(lit: &[Lit], clock: Clock, look: &Look, inv: &[[f32; 4]; 3], hero: Option<[f32; 3]>) -> Vec<[f32; 4]> {
	let mut rows = vec![[0.0f32; 4]; ROWS];
	let n = lit.len().min(MAX_PATCHES);
	rows[0] = [n as f32, clock.t, look.dark, clock.blend];
	for (k, (p, ph, rect)) in lit.iter().take(n).enumerate() {
		rows[1 + 4 * k] = [p.pos[0], p.pos[1], p.pos[2], p.radius];
		rows[2 + 4 * k] = [p.normal[0], p.normal[1], p.normal[2], p.seed];
		rows[3 + 4 * k] = [ph.flame, ph.scorch, 0.0, 0.0];
		rows[4 + 4 * k] = *rect;
	}
	rows[ROW_INV..ROW_INV + 3].copy_from_slice(inv);
	rows[ROW_INV + 3] = hero.map_or([0.0; 4], |h| [h[0], h[1], h[2], look.hero_r]);
	rows[ROW_INV + 4] = [look.glow, look.tol, 0.0, 0.0];
	rows
}

/// Draws the frame's ground fire decals inside the particle pass (called from `FireFx::draw` when the fire draws, before the flames).
/// Nothing unless a patch is alive. `now`: the game time of the patches' ages (`GameClock::now`); `shown`: the shader clock's game
/// time (`GameClock::shown`, smooth between fields).
pub(super) fn draw(blast: &BlastFire, now: f64, shown: f64, pass: &mut wgpu::RenderPass<'_>, env: &FireEnv<'_>) {
	if fire_option::level() < 2 || blast.patches().is_empty() || super::pipe_guard::off(super::pipe_guard::Effect::GroundFire) {
		return;
	}
	let (look, life) = (look(), Life::tuned());
	let lit = lit_patches(blast.patches(), now, &life, &env.view_proj, &look);
	let Some(inv) = inverse_rows(&env.view_proj).filter(|_| !lit.is_empty()) else { return };
	warm(blast, env);
	let Some(res) = blast.res.get() else { return };
	let rows = pack(&lit, Clock::game(shown), &look, &inv, blast.hero());
	env.queue.write_buffer(&res.ubuf, 0, bytemuck::cast_slice(&rows));
	pass.set_pipeline(&res.pipeline);
	pass.set_bind_group(1, &res.ubind, &[]);
	pass.draw(0..6, 0..lit.len() as u32);
}

/// Makes the ground fire's pipeline once (Medium, High): `FireFx::draw` calls it on every frame the fire draws, so it is built on the
/// first such frame (the level's first), not at the first explosion mid-fight (review R7-3). A refused shader turns the effect off.
pub(super) fn warm(blast: &BlastFire, env: &FireEnv<'_>) {
	if blast.res.get().is_some() || fire_option::level() < 2 {
		return;
	}
	if let Some(r) = super::pipe_guard::guard(env.device, super::pipe_guard::Effect::GroundFire, || GroundRes::new(env)) {
		let _ = blast.res.set(r);
	}
}

impl FireFx {
	/// The GPU copy of the ground fire patches for stage 3 (the first rows of the block, module doc); None until the first frame that drew
	/// a patch, and its content is only valid on frames the fire drew.
	#[allow(dead_code)]
	pub(in crate::viewer) fn patch_buffer(&self) -> Option<&wgpu::Buffer> {
		self.blast.res.get().map(|r| &r.ubuf)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn parse() -> naga::Module {
		naga::front::wgsl::parse_str(WGSL).unwrap_or_else(|e| panic!("groundfire: {}", e.emit_to_string(WGSL)))
	}

	/// The decal shader parses and validates (naga) with both entry points; its uniform block is as large as the rust rows.
	#[test]
	fn groundfire_shader_validates() {
		let m = parse();
		let names: Vec<_> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
		for want in ["vs_decal", "fs_decal"] {
			assert!(names.contains(&want), "missing entry {want}");
		}
		let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
		v.validate(&m).unwrap_or_else(|e| panic!("groundfire validates: {e:?}"));
		let gu = m.types.iter().find(|(_, t)| t.name.as_deref() == Some("GU")).expect("struct GU");
		match &gu.1.inner {
			naga::TypeInner::Struct { span, .. } => assert_eq!(*span as usize, ROWS * 16, "the rust block and the shader block differ"),
			other => panic!("GU is {other:?}"),
		}
	}

	/// DX12 parity: the decal shader must also go through naga's HLSL writer (FXC itself is not run).
	#[cfg(windows)]
	#[test]
	fn groundfire_shader_writes_hlsl() {
		let m = parse();
		let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
		let (options, pipeline) = (naga::back::hlsl::Options::default(), naga::back::hlsl::PipelineOptions::default());
		let mut out = String::new();
		naga::back::hlsl::Writer::new(&mut out, &options, &pipeline).write(&m, &info, None).expect("HLSL writes");
		assert!(out.contains("vs_decal") && out.contains("fs_decal"), "the decal entry points are in the HLSL");
	}

	/// The text of `src` from the line that starts with `from` up to (not including) the line that starts with `to`.
	fn block<'a>(src: &'a str, from: &str, to: &str) -> &'a str {
		let a = src.find(from).unwrap_or_else(|| panic!("no {from:?}"));
		let b = a + src[a..].find(to).unwrap_or_else(|| panic!("no {to:?}"));
		&src[a..b]
	}

	/// The noise and the heat ramp are copies of firefx.wgsl (naga needs a self-contained module): a change in one must reach the other.
	#[test]
	fn the_noise_and_the_ramp_are_copies_of_the_fire_shader() {
		let fire = include_str!("firefx.wgsl");
		assert_eq!(block(WGSL, "fn hash11(", "// ---- end of the noise copy").trim(), block(fire, "fn hash11(", "// ---- the flame ----").trim());
		assert_eq!(block(WGSL, "fn ramp(", "// ---- end of the ramp copy").trim(), block(fire, "fn ramp(", "// A tongue of flame").trim());
		assert!(fire.contains("const PERIOD: f32 = 512.0;") && WGSL.contains("const PERIOD: f32 = 512.0;"));
	}

	#[test]
	fn the_row_layout_is_head_patches_inverse_hero_tuning() {
		assert_eq!(ROWS, 70);
		assert_eq!(ROW_INV, 65);
		let inv = [[1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0], [9.0, 10.0, 11.0, 12.0]];
		let look = Look { dark: 0.8, glow: 1.5, tol: 1.1, hero_r: 0.6 };
		let patch = GroundFirePatch { pos: [1.0, 2.0, 3.0], normal: [0.0, 0.0, 1.0], radius: 1.4, born: 0.0, seed: 0.25 };
		let lit = [(patch, Phase { flame: 0.9, scorch: 0.5 }, [-0.5, -0.25, 0.5, 0.75]), (patch, Phase { flame: 0.1, scorch: 1.0 }, [-1.0, -1.0, 1.0, 1.0])];
		let rows = pack(&lit, Clock { t: 7.0, blend: 0.5 }, &look, &inv, None);
		assert_eq!(rows.len(), ROWS);
		assert_eq!(rows[0], [2.0, 7.0, 0.8, 0.5]);
		assert_eq!(rows[1], [1.0, 2.0, 3.0, 1.4]);
		assert_eq!(rows[2], [0.0, 0.0, 1.0, 0.25]);
		assert_eq!(rows[3], [0.9, 0.5, 0.0, 0.0]);
		assert_eq!(rows[4], [-0.5, -0.25, 0.5, 0.75], "the screen rectangle is the fourth row of a patch");
		assert_eq!(rows[7], [0.1, 1.0, 0.0, 0.0], "the second patch starts four rows later (row 5)");
		assert_eq!(rows[8], [-1.0, -1.0, 1.0, 1.0]);
		assert_eq!(rows[9], [0.0; 4], "unused patch rows are zero");
		assert_eq!(&rows[65..68], &inv);
		assert_eq!(rows[68], [0.0; 4], "no hero = radius 0");
		assert_eq!(rows[69], [1.5, 1.1, 0.0, 0.0]);
		let with = pack(&lit, Clock { t: 0.0, blend: 0.0 }, &look, &inv, Some([4.0, 5.0, 6.0]));
		assert_eq!(with[68], [4.0, 5.0, 6.0, 0.6]);
		let many = vec![(patch, Phase { flame: 1.0, scorch: 1.0 }, [0.0; 4]); 40];
		assert_eq!(pack(&many, Clock { t: 0.0, blend: 0.0 }, &look, &inv, None)[0][0], MAX_PATCHES as f32, "the count is capped");
	}

	/// The rows rebuild the world point of a pixel from its depth (the way the shader does it), for a real camera of the viewer.
	#[test]
	fn the_inverse_rows_round_trip_a_perspective_view() {
		let cam = Camera { pos: [3.0, -2.0, 5.0], yaw: 0.7, pitch: -0.4, tan_half: TAN_HALF_FOV, up: [0.0, 0.0, 1.0] };
		let m = view_proj(&cam, 16.0 / 9.0, RETAIL_NEAR, RETAIL_FAR);
		let inv = inverse_rows(&m).expect("invertible");
		let f = [cam.pitch.cos() * cam.yaw.cos(), cam.pitch.cos() * cam.yaw.sin(), cam.pitch.sin()];
		let right = normalise(cross(f, cam.up));
		for (d, side, rise) in [(12.0f32, 1.5f32, -0.5f32), (4.0, -2.0, 0.7), (40.0, 6.0, -3.0)] {
			let p: [f32; 3] = std::array::from_fn(|k| cam.pos[k] + f[k] * d + right[k] * side + cam.up[k] * rise);
			let clip: [f32; 4] = std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r]);
			let (xn, yn, w) = (clip[0] / clip[3], clip[1] / clip[3], clip[3]);
			let v = [xn * w - inv[0][3], yn * w - inv[1][3], w - inv[2][3]];
			let q: [f32; 3] = std::array::from_fn(|r| inv[r][0] * v[0] + inv[r][1] * v[1] + inv[r][2] * v[2]);
			for k in 0..3 {
				assert!((q[k] - p[k]).abs() < 2e-3, "d {d}: {q:?} vs {p:?}");
			}
		}
		assert!(inverse_rows(&[[0.0; 4]; 4]).is_none(), "a singular matrix has no rows");
	}

	fn matrix(pos: [f32; 3], yaw: f32, pitch: f32) -> [[f32; 4]; 4] {
		let cam = Camera { pos, yaw, pitch, tan_half: TAN_HALF_FOV, up: [0.0, 0.0, 1.0] };
		view_proj(&cam, 16.0 / 9.0, RETAIL_NEAR, RETAIL_FAR)
	}

	fn flat_patch(pos: [f32; 3], normal: [f32; 3]) -> GroundFirePatch {
		GroundFirePatch { pos, normal, radius: 1.4, born: 0.0, seed: 0.5 }
	}

	fn tuning() -> Look {
		Look { dark: 0.8, glow: 1.0, tol: 1.0, hero_r: 0.6 }
	}

	fn ndc(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 2] {
		let q = xyw(m, p);
		[q[0] / q[2], q[1] / q[2]]
	}

	/// Review m6 / m7: a box in front of the camera gets exactly the bounds of its projected corners.
	#[test]
	fn a_box_in_front_gets_the_bounds_of_its_projection() {
		let m = matrix([0.0, 0.0, 2.0], 0.0, -0.1);
		let corners = decal_box(&flat_patch([12.0, 0.0, 0.0], [0.0, 0.0, 1.0]), &tuning());
		let r = screen_rect(&m, &corners).expect("on screen");
		let pr: Vec<[f32; 2]> = corners.iter().map(|&c| ndc(&m, c)).collect();
		let lo = [0, 1].map(|a| pr.iter().map(|p| p[a]).fold(f32::MAX, f32::min));
		let hi = [0, 1].map(|a| pr.iter().map(|p| p[a]).fold(f32::MIN, f32::max));
		assert!([r[0] - lo[0], r[1] - lo[1], r[2] - hi[0], r[3] - hi[1]].iter().all(|d| d.abs() < 1e-5), "{r:?} vs {lo:?} {hi:?}");
		assert!(r[0] > -1.0 && r[1] > -1.0 && r[2] < 1.0 && r[3] < 1.0, "a small patch far ahead is a small rectangle: {r:?}");
	}

	/// A corner behind the eye (hero standing on a scorch, low camera) gives a bounded rectangle from the near-plane clip, not the screen.
	#[test]
	fn a_corner_behind_the_eye_is_clipped_not_the_whole_screen() {
		let m = matrix([0.0, 0.0, 1.7], 0.0, 0.0);
		let corners = decal_box(&flat_patch([1.2, 0.0, 0.0], [0.0, 0.0, 1.0]), &tuning());
		assert!(corners.iter().any(|&c| xyw(&m, c)[2] < 0.05) && corners.iter().any(|&c| xyw(&m, c)[2] > 0.05), "some corners behind, some in front");
		let r = screen_rect(&m, &corners).expect("a scorch under the camera is on screen");
		assert!(r[3] < 0.0, "the scorch is below the horizon: the top of the rectangle is under the middle of the screen: {r:?}");
		assert_eq!(r[1], -1.0, "and it runs to the bottom edge");
		// In front corners project inside the rectangle.
		for c in corners.iter().filter(|&&c| xyw(&m, c)[2] >= 0.05) {
			let p = ndc(&m, *c);
			let p = [p[0].clamp(-1.0, 1.0), p[1].clamp(-1.0, 1.0)];
			assert!(p[0] >= r[0] - 1e-5 && p[0] <= r[2] + 1e-5 && p[1] >= r[1] - 1e-5 && p[1] <= r[3] + 1e-5, "{p:?} outside {r:?}");
		}
	}

	/// Only a patch that is alive, has a strength and is on screen gets an instance.
	#[test]
	fn only_live_visible_patches_get_an_instance() {
		let m = matrix([0.0, 0.0, 2.0], 0.0, -0.1);
		let life = Life { burn: 3.0, scorch: 8.0 };
		let on = flat_patch([12.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
		let behind = flat_patch([-12.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
		let old = GroundFirePatch { born: -30.0, ..on };
		let fresh = GroundFirePatch { born: 10.0, ..on };
		let got = lit_patches(&[on, behind, old, fresh], 1.0, &life, &m, &tuning());
		assert_eq!(got.len(), 1, "the one behind the eye, the expired one and the one with no strength yet draw nothing");
		assert_eq!(got[0].0.pos, on.pos);
		let many: Vec<_> = (0..40).map(|_| on).collect();
		assert_eq!(lit_patches(&many, 1.0, &life, &m, &tuning()).len(), MAX_PATCHES);
	}

	#[test]
	fn a_box_behind_or_off_screen_draws_nothing() {
		let m = matrix([0.0, 0.0, 2.0], 0.0, 0.0);
		let behind = decal_box(&flat_patch([-10.0, 0.0, 0.0], [0.0, 0.0, 1.0]), &tuning());
		assert_eq!(screen_rect(&m, &behind), None, "fully behind the eye");
		for y in [40.0, -40.0] {
			let side = decal_box(&flat_patch([12.0, y, 0.0], [0.0, 0.0, 1.0]), &tuning());
			assert_eq!(screen_rect(&m, &side), None, "far to the side (y {y})");
		}
		let below = decal_box(&flat_patch([12.0, 0.0, -80.0], [0.0, 0.0, 1.0]), &tuning());
		assert_eq!(screen_rect(&m, &below), None, "far below the view");
	}

	/// A 60 degree slope (the steepest `land` accepts): the plane's own points at 1.2 radius all lie inside the rectangle, and the box is
	/// oriented in the plane (the axis-aligned box of the first version cut the uphill and downhill parts).
	#[test]
	fn a_steep_slope_is_not_cut() {
		let n = normalise([0.866, 0.0, 0.5]);
		let p = flat_patch([0.0, 0.0, 0.0], n);
		let m = matrix([-8.0, -6.0, 3.0], 0.6435, -0.2915);
		let r = screen_rect(&m, &decal_box(&p, &tuning())).expect("on screen");
		let (t1, t2) = tangents(n);
		for k in 0..16 {
			let a = k as f32 / 16.0 * std::f32::consts::TAU;
			let q: [f32; 3] = std::array::from_fn(|i| p.pos[i] + 1.2 * p.radius * (a.cos() * t1[i] + a.sin() * t2[i]));
			let s = ndc(&m, q);
			assert!(s[0] >= r[0] && s[0] <= r[2] && s[1] >= r[1] && s[1] <= r[3], "plane point {k} {s:?} outside {r:?}");
		}
		// The box is oriented in the plane: corners 0 and 1 differ by the full width along t1.
		let c0 = decal_box(&p, &tuning());
		let along = |c: [f32; 3]| c[0] * t1[0] + c[1] * t1[1] + c[2] * t1[2];
		assert!((along(c0[1]) - along(c0[0]) - 2.0 * 1.35 * p.radius).abs() < 1e-4, "corners 0 and 1 differ along t1 by the full width");
	}
}
