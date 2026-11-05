//! The window and the wgpu renderer: everything drawn on screen. The game host that drives it lives in the game's pack (R&C1:
//! `rac1/host`, engine split stage E2, 2026-10-26): its `run` opens the winit window and event loop and every display frame steps the
//! game, builds the frame and draws it through the `Gpu` renderer here (gpu.rs). Retail mode is the only game path: the picture is the
//! retail game's own state drawn natively.
//!
//! Children, one job each (`use super::*;` in each; most items are re-exported here):
//! - frame services: `presenter` (present on a worker thread), `pace` (frame cap), `profile` (frame timing), `frame_clock` (the effect
//!   clock's frame stamp), `test_clock` (test-only switches of the regression gate)
//! - GPU: `gpu` (the wgpu renderer, split into gpu/), `gpuskin` (props posed in the vertex shader), `gpuhero` (hero marker by compute),
//!   `gpucache` (same-inputs keys for marker and tie culling), `tiegpu` (tie vertex rewrite on the GPU), `bindless` (tie and shrub batches
//!   without a bind group per draw), `clip` (moby guard-band clipper; the GS draw-state decode is `formats/gs_state.rs` since E2), `warpfx` (arrival
//!   warp draw), `shadowvol` (moby shadow volumes), `projection` (PAL projection, `view_proj`), `targets` (MSAA / depth views, mips, buffers),
//!   `maths` (vector, matrix and mesh helpers)
//! - front end: `hudsharp` (sharp title)
//! - what is drawn: `hero_pose` (back objects, held gadget rigs), `pause_hero` (pause-menu hero preview), `lights` (pause / gold level-light
//!   slots), `moby_fade` (moby fade and alpha encodings), `smooth` (frame smoothing: drawing between two game ticks; R&C1 blends its own draw lists in rac1/interp.rs), `fardraw`
//!   (draw-distance option)
//! - PC graphics options: `post` (Enhanced image), `sunshadow` (dynamic shadows), `waterfx` and `underwater` (water looks), `mirror` (planar
//!   water mirror), `icefx` (ice gloss), `partfx` (particles Enhanced), `vegfx` (vegetation)
//! Kept in this file: the main WGSL `SHADER` (and its tests), and the GPU state structs whose fields the other renderer files and the host
//! read (Vertex, Uniforms, Scene, Gpu, Camera).

use crate::level::Texture;
use crate::mesh::{LitKind, Mesh};
use crate::physics::World;
use crate::prop::Prop;
use crate::overlay::{self, Quad};
use crate::{anim, dynlight, gameplay, mobies, shrubs, sky, terrain, ties};
use bytemuck::{Pod, Zeroable};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

pub mod gpu;
pub(crate) mod gpuskin;
pub(crate) mod gpuhero;
pub(crate) mod furfx;
pub(crate) mod bindless;
pub(crate) mod warpfx;
pub(crate) mod clip;
pub(crate) mod shadowvol;
pub(crate) mod post; pub(crate) mod volclouds; pub mod volfog; pub(crate) mod volsmoke; pub(crate) mod star; pub(crate) mod starfield; pub mod pipe_guard;
pub mod sunshadow;
pub(crate) mod waterfx;
pub mod aspect_box; pub mod under_choice; pub mod under_detect; pub mod underwater;
pub(crate) mod icefx; pub(crate) mod texfilter; pub(crate) mod metalfx; pub(crate) mod metalenv; pub(crate) mod metalpipe; pub(crate) mod metaldraw; pub(crate) use metaldraw::MetalDraw;
pub(crate) mod mirror;
pub(crate) mod partfx; pub(crate) mod firefx; pub(crate) mod firefx_light; pub(crate) mod firefx_vol; pub(crate) mod firefx_ground; pub(crate) mod firefx_ground_track; pub(crate) mod firefx_burn; pub(crate) mod firefx_burn_trail; pub(crate) mod firefx_grid; pub(crate) mod firefx_clock; pub(crate) mod firefx_gametime; pub(crate) mod firefx_jet; pub(crate) mod firefx_jet_quads; pub(crate) mod firefx_jet_lerp; pub(crate) mod firefx_jet_grass; pub(crate) mod firefx_jet_vol; pub(crate) mod firefx_blast; pub(crate) mod firefx_blast_gpu; pub(crate) mod firesim; pub(crate) mod firesim_heat; pub(crate) mod firesim_tongues; pub(crate) mod firebook; pub(crate) mod firebook_dump;
pub(crate) mod shotlight; // [PC] weapon lights (lane nwlit)
pub mod vegfx;
pub mod hudsharp;
pub mod smooth;
pub mod smooth_lead;
pub(crate) mod gpucache;
pub(crate) mod tiegpu;
pub mod profile; pub(crate) mod passtime; pub(crate) mod gfxq; pub mod framecsv;
pub mod pace;
pub mod test_clock;
pub(crate) mod presenter; pub(crate) mod qdefer; use presenter::GatedQueue; pub(crate) mod gpu_errors;
pub mod fardraw;

pub(crate) mod projection;
pub mod targets;
pub(crate) mod upscale; pub use upscale::choice as upscale_choice; // [PC] upscaler: inputs (RAC_UPSCALE_PREP), slot (RAC_UPSCALE), the menu choice (choice.rs)
pub mod maths;
pub(crate) mod moby_fade;
pub mod hero_pose; pub mod loose_range;
// HUD position / scale (dev s138): the pure parts the Gpu holds; the host's live parts stay in rac1/host/handler/hud_place.rs.
pub mod hud_place;
pub mod hud_hold;
pub mod bgpool;
pub(crate) mod pause_hero;
pub(crate) mod lights;
pub mod frame_clock;
pub use self::projection::*;
pub use self::targets::*;
pub use self::maths::*;
pub(crate) use self::moby_fade::*;
pub use self::hero_pose::*;
pub use self::pause_hero::*;
pub use self::lights::*;
pub use self::projection::{PROJ_Y_RATIO};
pub use self::targets::{ITEM_PANEL_TEX, MENU_ART_TEX, POPUP_PANEL_TEX, PORTRAIT_PANEL_TEX, RETAIL_OV_TEX, RETAIL_HD_TEX, RETAIL_FILTER_TEX};

/// The change tokens of this frame's prop poses (`Gpu::set_props` / gpu/pose_reuse.rs): `of(in_fx, pi, key)` is the "born" stamp of the pose
/// prop `pi` takes (`fx_posed`'s when `in_fx`, else the one of `key()`'s pose); equal tokens = bit-equal poses, 0 = no token (always
/// written). The game host makes the tokens (R&C1: `rac1/host/handler/pose_born.rs`).
pub trait PoseTokens {
	fn of(&self, in_fx: bool, pi: usize, key: &dyn Fn() -> PoseKey) -> u64;
}

/// No tokens: every pose is written (menu and front-end previews).
impl PoseTokens for () {
	fn of(&self, _in_fx: bool, _pi: usize, _key: &dyn Fn() -> PoseKey) -> u64 {
		0
	}
}

/// Particle texture number n is uploaded into `fx_tex` under key PART_TEX + n; effect types reach it
/// through `Gpu::part_defs` (level particle def table, decomp/notes/particles.md).
pub const PART_TEX: u32 = 0x2000;
/// fx_tex keys for the blast mobys (decomp/notes/particles.md): a white texel for the untextured shard faces, and
/// FX_MOBY_TEX + n for moby texture n.
pub const FX_WHITE: usize = 0x2fff;
pub const FX_MOBY_TEX: usize = 0x3000;

/// A blast moby's triangles: (fx_tex key, vertex indices) over `pos` / `uv`.
pub struct FxMesh {
	pub pos: Vec<[f32; 3]>,
	pub uv: Vec<[f32; 2]>,
	pub tris: Vec<(usize, [u32; 3])>,
}
use winit::window::Window;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
	pos: [f32; 3],
	uv: [f32; 2],
	normal: [f32; 3],
	colour: [f32; 4],
	/// Mobies (colour alpha >= 0): ambient rgb (GS scale, 0x80 = 1.0) and the light pair packed as A * 16 + B + blend (see `moby_light`).
	light: [f32; 4],
}

/// One retail-mode ocean batch (`Gpu::ocean_ram`): its 64x64 texture (rewritten every field), the strips as triangles, the GS ALPHA FIX byte.
pub(crate) struct OceanBatch {
	tex: wgpu::Texture,
	bind: wgpu::BindGroup,
	verts: Vec<Vertex>,
	idx: Vec<u32>,
	/// The second lava pass (Gaspar): its strips as triangles, drawn after the first pass with the GS GEQUAL depth test (`ocean_g_pipeline` 2 / 3).
	pverts: Vec<Vertex>,
	pidx: Vec<u32>,
	fix: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Uniforms {
	view_proj: [[f32; 4]; 4],
	/// The same camera without translation: the sky is centred on the eye.
	sky_view_proj: [[f32; 4]; 4],
	/// The level's directional light slots (colour A + wrap in w, to-light A, colour B + wrap, to-light B);
	/// slots past the level's lights are zero (decomp/notes/lighting_fog.md "Moby lighting").
	lights: [[[f32; 4]; 4]; 16],
	/// Uncoloured world geometry (scenery mobies; the moby ambient), moby default ambient, zone ambient A and B (rgb, GS scale 1.0 = 0x80).
	ambient: [[f32; 4]; 4],
	/// Linear fog colour; (near, far, coefficient at near, at far); eye position.
	fog_colour: [f32; 4],
	fog: [f32; 4],
	eye: [f32; 4],
	/// The 8 dynamic point-light slots (decomp/notes/lighting_fog.md 5b): (pos, radius), (rgb, intensity); radius 0 = free.
	points: [[f32; 4]; 16],
	/// The dynamic third light each moby id (colour alpha, 1-based) takes at its origin: (rgb, travel direction) pairs.
	dynl: [[f32; 4]; 2 * MAX_DYN],
	/// [PC] Dynamic shadows (sunshadow.rs): the cascades' light view-projections, (light travel direction, strength; 0 = off),
	/// per cascade the receiver depth bias (x..z; w = map size), per cascade the world size of a map texel.
	sun_vp: [[[f32; 4]; 4]; 3],
	sun: [f32; 4],
	sun_bias: [f32; 4],
	sun_texel: [f32; 4],
	/// [PC] Dynamic shadows Low / Medium / High (sunshadow.rs `Quality`): PCF kernel side, map uv scale, cascades read.
	sun_q: [f32; 4], keep_tail: [f32; 4], // [PC] #29: keep_tail.x = 1 in the water bed / mirror passes (mirror.rs; no blend pass there): a moby's tail texels draw opaque as before
	/// [PC] Metal On (metalfx.wgsl `u.metal`, filled by `metalenv::MetalEnv::write_uniforms`; zero = off).
	metal: [[f32; 4]; 14],
}

/// Mobies lit by a dynamic light in one frame (the rest take none).
pub(crate) const MAX_DYN: usize = 128;

pub(crate) const SHADER: &str = concat!(r#"
struct U { view_proj: mat4x4<f32>, sky_view_proj: mat4x4<f32>, lights: array<array<vec4<f32>, 4>, 16>, ambient: array<vec4<f32>, 4>, fog_colour: vec4<f32>, fog: vec4<f32>, eye: vec4<f32>, points: array<vec4<f32>, 16>, dynl: array<vec4<f32>, 256>, sun_vp: array<mat4x4<f32>, 3>, sun: vec4<f32>, sun_bias: vec4<f32>, sun_texel: vec4<f32>, sun_q: vec4<f32>, keep_tail: vec4<f32>, metal: array<vec4<f32>, 14> };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var sun_map: texture_depth_2d_array;
@group(0) @binding(2) var sun_cmp: sampler_comparison;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) n: vec3<f32>, @location(2) c: vec4<f32>, @location(3) w: vec3<f32>, @location(4) @interpolate(flat) l: vec4<f32>, @location(5) @interpolate(linear) fogf: f32, @location(6) lit: vec3<f32>, @location(9) amb: vec3<f32>, @location(10) @interpolate(flat) metal: u32, @location(7) @interpolate(flat) kmx: vec2<f32> };

// GS fog coefficient per world class (gameplay::fog_coefficient; lighting_fog.md "Fog per class"). The vertex light
// w tags the class: -1 tfrag (per vertex, view depth), -2 shrub (per instance, l.xyz = instance origin) / -3 tie
// (per instance, l.xyz = sphere centre [P]); all three use the same slope/intercept (the d058..d06c line).
// Anything else returns -1: the fragment falls back to the euclidean-distance fog.
fn fog_class(l: vec4<f32>, w: f32) -> f32 {
	// Effects layer (retail/efx_draw.rs `fog_light`): l.w = -5 carries the GS fog byte F in l.x, interpolated linearly in screen space as the GS does.
	if (l.w < -4.5 && l.w > -5.5) {
		return 255.0 * (1.0 - u.fog_colour.w) + u.fog_colour.w * l.x; // [PC] fog_colour.w = the retail fog's scale (1.0 = retail; volfog/frame.rs `fade_retail_fog`)
	}
	let span = max(u.fog.y - u.fog.x, 0.001);
	let slope = (u.fog.w - u.fog.z) / span;
	let icpt = (u.fog.z * u.fog.y - u.fog.w * u.fog.x) / span;
	var raw = -1.0;
	if (l.w > -1.5 && l.w < -0.5) {
		raw = w * slope + icpt;
	} else if (l.w < -1.5 && l.w > -3.5) {
		raw = (u.view_proj * vec4<f32>(l.xyz, 1.0)).w * slope + icpt;
	} else {
		return -1.0;
	}
	return min(max(raw, u.fog.w), u.fog.z);
}

@vertex fn vs(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>, @location(4) l: vec4<f32>, @builtin(instance_index) inst: u32) -> VOut {
	var o = vs_main(p, uv, n, c, l);
	o.metal = (inst >> 16u) & 15u;
	o.kmx = tie_kmx(l, inst, o.kmx);
	if (c.a >= 0.0) { o.kmx = moby_kmx(inst); }
	return o;
}

// Moby draws (vertex colour alpha >= 0): the first-instance word of the draw (`slot_words` in gpu.rs) carries the material's
// TEX1 K (bits 2..14, raw signed 12 bits, 1/16 mip) and the top level of the texture's authored chain (bits 14..16), as for a tie.
// No word (0) gives MXL -1: the fragment takes the plain sample.
fn moby_kmx(w: u32) -> vec2<f32> {
	if ((w & 0xfffcu) == 0u) { return vec2<f32>(0.0, -1.0); }
	var k = i32((w >> 2u) & 0xfffu);
	if (k >= 2048) { k = k - 4096; }
	return vec2<f32>(f32(k) / 16.0, f32((w >> 14u) & 3u));
}

// Tie draws (vertex light w = -3): the first instance's material word (`shrub_inst`) carries the material's GS CLAMP wrap (bits
// 0..2), TEX1 K (bits 2..14, raw signed 12 bits, 1/16 mip) and the top level of the texture's authored chain (bits 14..16), as for
// a shrub mesh. A word without K and level (no authored chain) gives MXL -1: `world_texel` takes the plain sample. Every other
// draw keeps `vs_main`'s `kmx`.
fn tie_kmx(l: vec4<f32>, inst: u32, kmx: vec2<f32>) -> vec2<f32> {
	if (l.w < -2.5 && l.w > -3.5) {
		let w = shrub_inst(inst);
		var k = i32((w >> 2u) & 0xfffu);
		if (k >= 2048) { k = k - 4096; }
		return vec2<f32>(f32(k) / 16.0, select(-1.0, f32((w >> 14u) & 3u), (w & 0xfffcu) != 0u));
	}
	return kmx;
}

// Terrain fragments: the draw's first-instance word carries K (raw signed 12 bits, 1/16 mip) and the top mip level.
@vertex fn vs_tfrag(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>, @location(4) l: vec4<f32>, @builtin(instance_index) inst: u32) -> VOut {
	var o = vs_main(p, uv, n, c, l);
	o.metal = (inst >> 16u) & 15u;
	var k = i32(inst & 0xfffu);
	if (k >= 2048) { k = k - 4096; }
	o.kmx = vec2<f32>(f32(k) / 16.0, f32((inst >> 12u) & 3u));
	return o;
}

// Shrub mesh draws: the first instance's material word (`shrub_inst`; bindless keeps the texture slot below it) carries the
// CLAMP wrap (bits 0..2), the TEX1 K (bits 2..14, raw signed 12 bits, 1/16 mip) and the texture's top level (bits 14..16).
@vertex fn vs_shrub(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>, @location(4) l: vec4<f32>, @builtin(instance_index) inst: u32) -> VOut {
	var o = vs_main(p, uv, n, c, l);
	o.metal = (inst >> 16u) & 15u;
	let m = shrub_inst(inst);
	var k = i32((m >> 2u) & 0xfffu);
	if (k >= 2048) { k = k - 4096; }
	o.kmx = vec2<f32>(f32(k) / 16.0, f32((m >> 14u) & 3u));
	return o;
}

fn shrub_inst(inst: u32) -> u32 {
	return inst;
}

// [PC] perf round 3: the props batch posed here (gpuskin.rs; `gpuskin::emulate` is the CPU twin of the shader
// path). The vertex buffer holds the static model, `sj` = (slot, vertex in slot); the slot's record and its pose entries
// (model-space position, normal) come from storage buffers.
struct PropInst { m0: vec4<f32>, m1: vec4<f32>, m2: vec4<f32>, m3: vec4<f32>, light: vec4<f32>, tint: vec4<f32>, ptint: vec4<f32>, a: vec4<f32>, k: vec4<u32> };
@group(2) @binding(0) var<storage, read> insts: array<PropInst>;
@group(2) @binding(1) var<storage, read> poses: array<f32>;

@vertex fn vs_props(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>, @location(4) l: vec4<f32>, @location(5) sj: vec2<u32>, @builtin(instance_index) inst: u32) -> VOut {
	var o = vs_props_pose(p, uv, n, c, l, sj);
	o.metal = (inst >> 16u) & 15u;
	if (o.c.a >= 0.0) { o.kmx = moby_kmx(inst); }
	return o;
}

fn vs_props_pose(p: vec3<f32>, uv: vec2<f32>, n: vec3<f32>, c: vec4<f32>, l: vec4<f32>, sj: vec2<u32>) -> VOut {
	let s = insts[sj.x];
	let j = sj.y;
	let kind = s.k.x & 0xffu;
	var q = p;
	var nn = n;
	if (j < s.k.w) {
		let b = (s.k.z + j) * 6u;
		q = vec3<f32>(poses[b], poses[b + 1u], poses[b + 2u]);
		nn = vec3<f32>(poses[b + 3u], poses[b + 4u], poses[b + 5u]);
	}
	if ((kind == 1u || kind == 2u) && (s.k.x & 0x100u) == 0u) {
		var t = uv;
		var col = c;
		var pos = q + s.m3.xyz;
		var nrm = nn;
		if (kind == 1u) {
			t.x = uv.x + s.a.w;
			t.y = uv.y + s.a.z;
			let a = q * s.a.x;
			pos = s.m0.xyz * a.x + s.m1.xyz * a.y + s.m2.xyz * a.z + s.m3.xyz;
			nrm = s.m0.xyz * nn.x + s.m1.xyz * nn.y + s.m2.xyz * nn.z;
			if (s.ptint.w != 0.0) { col = vec4<f32>(col.rgb * s.ptint.rgb, col.a); }
		}
		col.a = s.a.y;
		if (j >= s.k.y) { col = s.tint; }
		return vs_main(pos, t, nrm, col, s.light);
	}
	return vs_main(vec3<f32>(0.0, 0.0, -1.0e6), uv, nn, c, l);
}

// [PC] depth precision (Veldin ground-patch / rock flicker, 2026-10-20): the clip position with z rebuilt from w as the
// perspective's own z = A w + B (A, B read off the matrix rows), so the depth carries only the rounding of w, like the GS's
// K / w. The full z row cancels two ~500 m terms per vertex (world coordinates) and loses ~1 cm at 10 m, enough for coplanar
// terrain layers (decal patches over the ground) to z-fight as the camera moves. Mathematically the same z; a matrix whose z
// row is not A * w row + const (ortho, oblique) keeps its own z.
fn world_clip(m: mat4x4<f32>, p: vec3<f32>) -> vec4<f32> {
	var c = m * vec4<f32>(p, 1.0);
	let wr = vec4<f32>(m[0][3], m[1][3], m[2][3], m[3][3]);
	let zr = vec4<f32>(m[0][2], m[1][2], m[2][2], m[3][2]);
	let ww = dot(wr.xyz, wr.xyz);
	if (ww > 1.0e-6) {
		let a = dot(zr.xyz, wr.xyz) / ww;
		let d = zr.xyz - a * wr.xyz;
		if (dot(d, d) <= 1.0e-10 * ww) {
			c.z = a * c.w + (zr.w - a * wr.w);
		}
	}
	return c;
}

fn vs_main(p: vec3<f32>, uv: vec2<f32>, n: vec3<f32>, c: vec4<f32>, l: vec4<f32>) -> VOut {
	var o: VOut;
	o.pos = world_clip(u.view_proj, p);
	o.w = p;
	o.uv = uv;
	o.n = n;
	o.kmx = n.xy;
	o.metal = 0u;
	o.c = c;
	o.l = l;
	o.fogf = fog_class(l, o.pos.w);
	// NOV-35b metal vertex (light w = -4): the colour bytes / 128 come in l.rgb (FUN_00210358 metal branch, CPU-built).
	if (c.a >= 0.0 && l.w < -3.5) {
		o.lit = l.rgb;
		o.amb = l.rgb;
		return o;
	}
	// Per-vertex lighting (VU0 prog0 + FUN_00210358): the light pair is blended, shaded once, summed with the dynamic
	// lights, then truncated to the 1/128 grid on top of the ambient byte and clamped to 255/128. Baked colours (-2) skip it.
	if (c.a > -1.5) {
		let nn = normalize(n);
		var s = vec3<f32>(0.0);
		var amb = u.ambient[0].rgb;
		if (c.a >= 0.0) {
			let base = floor(l.w);
			let ab = u32(base);
			s = slot_blend(min(ab / 16u, 15u), ab % 16u, l.w - base, nn) + dyn_moby(u32(c.a + 0.5), nn);
			amb = l.rgb;
		} else {
			s = slot(0u, nn);
			if (l.w > 0.5) { s += points_moby(l.xyz, nn); }
		}
		o.lit = min(amb + floor(128.0 * max(s, vec3<f32>(0.0))) / 128.0, vec3<f32>(255.0 / 128.0));
		o.amb = min(amb, o.lit);
	}
	return o;
}

"#, include_str!("viewer/sunrecv.wgsl"), include_str!("viewer/skyvis.wgsl"), include_str!("viewer/metalfx.wgsl"), r#"
// One light slot: colour * max(q, q * wrap) for both of its directions (prog11).
fn slot(k: u32, n: vec3<f32>) -> vec3<f32> {
	let l = u.lights[k];
	let qa = dot(n, l[1].xyz);
	let qb = dot(n, l[3].xyz);
	return l[0].rgb * max(qa, qa * -abs(l[0].w)) + l[2].rgb * max(qb, qb * -abs(l[2].w));
}

// Light pair blended before shading (FUN_0026a338): colours (wrap lane included) and directions lerp A -> B by t = b/256,
// the blended directions are renormalised; t = 0 uses A as is.
fn slot_blend(a: u32, b: u32, t: f32, n: vec3<f32>) -> vec3<f32> {
	if (t == 0.0) { return slot(a, n); }
	let la = u.lights[a];
	let lb = u.lights[b];
	let ca = mix(la[0], lb[0], t);
	let cb = mix(la[2], lb[2], t);
	let da = mix(la[1].xyz, lb[1].xyz, t);
	let db = mix(la[3].xyz, lb[3].xyz, t);
	var r = vec3<f32>(0.0);
	if (dot(da, da) >= 1e-12) {
		let qa = dot(n, normalize(da));
		r += ca.rgb * max(qa, qa * -abs(ca.w));
	}
	if (dot(db, db) >= 1e-12) {
		let qb = dot(n, normalize(db));
		r += cb.rgb * max(qb, qb * -abs(cb.w));
	}
	return r;
}

// Dynamic point lights on mobies (MobyProc): each light in range adds rgb w (w = 1 - I d), merged into one third
// light with dir = normalize(sum dir w) and wrap 0, evaluated at the moby origin `p`. Scenery mobies (alpha -1,
// origin in the light attribute) run this here; the posed mobies get the same result from `dynlight::moby_light`
// on the CPU through `dynl`. Tfrags, ties and shrubs are relit per vertex on the CPU (`dynlight::LitWorld`).
fn points_moby(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
	var col = vec3<f32>(0.0);
	var dir = vec3<f32>(0.0);
	for (var k = 0u; k < 8u; k++) {
		let a = u.points[2u * k];
		let b = u.points[2u * k + 1u];
		let d = distance(p, a.xyz);
		if (a.w <= 0.0 || d >= a.w || d < 0.0001) { continue; }
		let w = 1.0 - b.w * d;
		col += b.rgb * w;
		dir += (p - a.xyz) / d * w;
	}
	if (dot(dir, dir) < 1e-8) { return vec3<f32>(0.0); }
	return col * max(dot(n, -normalize(dir)), 0.0);
}

// The third light of a posed moby (colour alpha = its 1-based id, 0 = none).
fn dyn_moby(id: u32, n: vec3<f32>) -> vec3<f32> {
	if (id == 0u) { return vec3<f32>(0.0); }
	return u.dynl[2u * (id - 1u)].rgb * max(dot(n, -u.dynl[2u * (id - 1u) + 1u].xyz), 0.0);
}

// World texel: a tie (`tie_kmx`) is read at the level its TEX1 picks (GS LINEAR_MIPMAP_NEAREST, LOD = log2(32 vz) + K, capped at
// MXL; Kalebo mountains are level 3 at 250 m), every other fragment is the plain sample. The gradients are taken first so the
// explicit-gradient read, identical to `textureSample`, can sit in a branch.
fn world_texel(i: VOut) -> vec4<f32> {
	let dx = dpdx(i.uv);
	let dy = dpdy(i.uv);
	if (i.l.w < -2.5 && i.l.w > -3.5 && i.kmx.y >= 0.0) {
		return textureSampleLevel(tex, samp, i.uv, gs_level(i));
	}
	return textureSampleGrad(tex, samp, i.uv, dx, dy);
}

// [PC] icefx.rs: Hoven's ice (ties and shrubs whose class tag carries the ice mark, w -3.25 / -2.25) while its gate is on (eye w
// = 1 + the look, `icefx::ICE_LOOK`): look 1 a cold gloss that follows the camera and a fresnel
// rim, 2 also light through the thin edges and sparkles, 3 all of it stronger. It only adds light: the retail texel x baked
// colour stays the base.
const ICE_RIM_POW: f32 = 3.0;
const ICE_GLOSS_POW: f32 = 40.0;
const ICE_GLOSS: f32 = 0.35;
const ICE_RIM: f32 = 0.18;
const ICE_THROUGH: f32 = 0.35;
const ICE_SPARK: f32 = 1.5;
const ICE_GRAIN: f32 = 4.0;
const ICE_TINT: vec3<f32> = vec3<f32>(0.80, 0.92, 1.0);
fn ice_look(i: VOut, rgb: vec3<f32>) -> vec3<f32> {
	let t = i.l.w;
	if (u.eye.w < 1.5 || !((t < -2.2 && t > -2.3) || (t < -3.2 && t > -3.3))) { return rgb; }
	let look = u.eye.w - 1.0;
	let k = select(1.0, 1.8, look > 2.5);
	let more = select(0.0, k, look > 1.5);
	let ln = length(i.n);
	if (ln < 1e-4) { return rgb; }
	let v = normalize(u.eye.xyz - i.w);
	var n = i.n / ln;
	if (dot(n, v) < 0.0) { n = -n; }
	let ndv = clamp(dot(n, v), 0.0, 1.0);
	let rim = pow(1.0 - ndv, ICE_RIM_POW);
	// The gloss light sits a little above the camera, so the highlight follows the view.
	let lc = normalize(v + vec3<f32>(0.0, 0.0, 0.6));
	let gloss = pow(max(dot(n, normalize(lc + v)), 0.0), ICE_GLOSS_POW);
	// Thin edges: the rim, brighter where the level's key light shines from behind the ice.
	let kl = u.lights[0][1].xyz;
	let back = rim * (0.4 + 0.6 * clamp(dot(-v, kl / max(length(kl), 1e-4)), 0.0, 1.0));
	// Sparkles: a hashed facet per world cell (1 / ICE_GRAIN) that catches the camera light.
	let cell = floor(i.w * ICE_GRAIN);
	let h = fract(sin(vec3<f32>(dot(cell, vec3<f32>(127.1, 311.7, 74.7)), dot(cell, vec3<f32>(269.5, 183.3, 246.1)), dot(cell, vec3<f32>(113.5, 271.9, 124.6)))) * 43758.5453);
	let fnrm = normalize(n + (h - 0.5) * 1.2);
	let spark = pow(max(dot(reflect(-v, fnrm), lc), 0.0), 300.0) * step(0.8, h.x);
	let lum = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
	let amt = clamp(lum * 1.6, 0.35, 1.0);
	return rgb * (1.0 + ICE_THROUGH * back * more) + ICE_TINT * (ICE_GLOSS * gloss * k + ICE_RIM * rim * k + ICE_SPARK * spark * more) * amt;
}

@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
	var c = world_texel(i);
	if (i.c.a >= 0.0 && i.kmx.y >= 0.0) { c = textureSampleLevel(tex, samp, i.uv, gs_level(i)); }
	if (c.a < 0.25) { discard; }
	// Lit moby texels and a moby's tinted-tail texels (`moby_tail`) below alpha 0x80 are blended in the moby pass (fs_moby_a / fs_moby_b), not drawn opaque here (the water bed and mirror passes have no blend pass: u.keep_tail.x = 1 keeps the tail texels opaque there, as before #29).
	if ((i.c.a >= 0.0 || (moby_tail(i) && u.keep_tail.x <= 0.0)) && floor(c.a * 128.0 + 0.5) < 128.0) { discard; }
	if (i.c.a >= 0.0 && i.c.a - floor(i.c.a + 0.5) > 0.0005) { discard; }
	// Alpha says how to light it: -2 baked world colour in rgb (terrain, ties, shrubs:
	// vertex_colours.md), -1 other world geometry (spawned-moby ambient, light 0), >= 0 a moby lit by
	// its own ambient and light pair (i.l). Rgb is white except for tinted mobies.
	if (i.c.a < -1.5) {
		if (i.c.a < -2.0005) { discard; }
		let d = distance(i.w, u.eye.xyz);
		var f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
		if (i.fogf >= 0.0) { f = i.fogf / 255.0; }
		let shade = sun_baked(min(i.c.rgb, vec3<f32>(255.0 / 128.0)), i.w);
		if (u.keep_tail.y <= 0.0 || i.metal == 0u) { return vec4<f32>(mix(u.fog_colour.rgb, ice_look(i, c.rgb * shade), f), 1.0); }
		return vec4<f32>(mix(u.fog_colour.rgb, ice_look(i, metal_look(i, c.rgb * shade)), f), 1.0);
	}
	var rgb = c.rgb * i.c.rgb * sun_lit(i);
	if (u.keep_tail.y > 0.0 && i.metal != 0u) { rgb = metal_look(i, rgb); }
	// GS fog: coefficient linear in distance, 255 = unfogged (FUN_001f7900).
	let d = distance(i.w, u.eye.xyz);
	let f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	// [PC] Global illumination (post.rs `moby_cover`): with the post chain on (u.sun_texel.w = 1) an opaque moby (Ratchet, enemies,
	// crates, pickups: colour alpha >= 0) leaves alpha 0 in the post scene, every other opaque pixel leaves 1, so the bounce light
	// neither starts from a moby nor lands on one. Nothing else reads the post scene's alpha.
	return vec4<f32>(mix(u.fog_colour.rgb, rgb, f), select(1.0, 0.0, i.c.a >= 0.0 && u.sun_texel.w > 0.0));
}

// Shrub fragments with alpha (faded mesh, billboards): the baked world colour of `fs` with the GS alpha As = texel alpha x A >> 7,
// A carried below -2 in the colour alpha (-2 - A / 256). The entries choose the Z-write split of the GS alpha test.
// [PC] speed (exact): the alpha is its own function, so the entries test it before the colour (the shadow's PCF) is computed.
fn shrub_alpha(i: VOut, c: vec4<f32>) -> f32 {
	let a = round((-2.0 - i.c.a) * 256.0);
	let ta = floor(c.a * 128.0 + 0.5);
	return floor(ta * a / 128.0) / 128.0;
}

fn shrub_rgb(i: VOut, c: vec4<f32>) -> vec3<f32> {
	let d = distance(i.w, u.eye.xyz);
	var f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	if (i.fogf >= 0.0) { f = i.fogf / 255.0; }
	let shade = sun_baked(min(i.c.rgb, vec3<f32>(255.0 / 128.0)), i.w);
	if (u.keep_tail.y <= 0.0 || i.metal == 0u) { return mix(u.fog_colour.rgb, ice_look(i, c.rgb * shade), f); }
	return mix(u.fog_colour.rgb, ice_look(i, metal_look(i, c.rgb * shade)), f);
}

// Billboard texel: the GS picks the mip from LOD = log2(32 vz) + K (Q = 1 / (32 vz)); K and the top level ride in the vertex normal.
fn shrub_bb_texel(i: VOut) -> vec4<f32> {
	return textureSampleLevel(tex, samp, i.uv, gs_level(i));
}

"#, include_str!("viewer/texfilter.wgsl"), include_str!("viewer/moby_tail.wgsl"), r#"

// Terrain fragment: the baked world colour of `fs` with the level-chosen texel (GS LOD = log2(32 vz) + K; REN-TEX-RESIDENCY).
// GS ALPHA 0x44 (Cs - Cd) * As + Cd with As = the texel alpha (vertex alpha 128) and TEST 0x5360b (ATE GEQUAL AREF 0x60, AFAIL
// RGB_ONLY): texels below alpha 0x60 still blend (colour only, no z), the rest blend and write z (fx_census_late.md, run 19).
// [PC] speed (exact): the texel alpha is read first and the entries test it before the colour (fog, the shadow's PCF) is computed.
fn world_alpha(c: vec4<f32>) -> f32 {
	return min(floor(c.a * 128.0 + 0.5), 128.0) / 128.0;
}

fn tfrag_rgb(i: VOut, c: vec4<f32>) -> vec3<f32> {
	let d = distance(i.w, u.eye.xyz);
	var f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	if (i.fogf >= 0.0) { f = i.fogf / 255.0; }
	let shade = sun_baked(min(i.c.rgb, vec3<f32>(255.0 / 128.0)), i.w);
	if (u.keep_tail.y <= 0.0 || i.metal == 0u) { return mix(u.fog_colour.rgb, c.rgb * shade, f); }
	return mix(u.fog_colour.rgb, metal_look(i, c.rgb * shade), f);
}

@fragment fn fs_tfrag(i: VOut) -> @location(0) vec4<f32> {
	if (i.c.a < -2.0005) { discard; }
	let c = shrub_bb_texel(i);
	let ra = world_alpha(c);
	if (ra * 128.0 < 96.0) { discard; }
	return vec4<f32>(tfrag_rgb(i, c), ra);
}

@fragment fn fs_tfrag_lt96(i: VOut) -> @location(0) vec4<f32> {
	if (i.c.a < -2.0005) { discard; }
	let c = shrub_bb_texel(i);
	let ra = world_alpha(c);
	if (ra * 128.0 >= 96.0 || ra <= 0.0) { discard; }
	let r = vec4<f32>(tfrag_rgb(i, c), ra);
	soft_mark(i, r.a); return r; // [PC] the fog's no-Z texel (volfog/soft.rs)
}

// Tie fragment: the baked world colour of `fs` with the tie texel (`world_texel`: TEX1 level, CLAMP wrap) and its alpha. TieProc's
// draw packet (Pokitaru FUN_002b87c8, data 0x1c33b0) sets TEST_1 0x5340b (ATE GEQUAL AREF 0x40, AFAIL RGB_ONLY, ZTST GEQUAL) under
// ALPHA_1 0x44 (Cs - Cd) * As + Cd, As = texel alpha (vertex alpha 0x80): every texel blends, from alpha 0x40 it also writes Z.
// Drawn opaque, the fading feet of the Pokitaru shoal mounds were hard flat silhouettes (decomp/notes/fx_census_late.md).
fn tie_rgb(i: VOut, c: vec4<f32>) -> vec3<f32> {
	let d = distance(i.w, u.eye.xyz);
	var f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	if (i.fogf >= 0.0) { f = i.fogf / 255.0; }
	// [PC] Lighting Enhanced: the sun shadow / AO term of the other baked paths (`sun_baked`; identity in Classic).
	let shade = sun_baked(min(i.c.rgb, vec3<f32>(255.0 / 128.0)), i.w);
	if (u.keep_tail.y <= 0.0 || i.metal == 0u) { return mix(u.fog_colour.rgb, ice_look(i, c.rgb * shade), f); }
	return mix(u.fog_colour.rgb, ice_look(i, metal_look(i, c.rgb * shade)), f);
}

@fragment fn fs_tie(i: VOut) -> @location(0) vec4<f32> {
	if (i.c.a < -2.0005) { discard; }
	let c = world_texel(i);
	let ra = world_alpha(c);
	if (ra * 128.0 < 64.0) { discard; }
	return vec4<f32>(tie_rgb(i, c), ra);
}

@fragment fn fs_tie_lt64(i: VOut) -> @location(0) vec4<f32> {
	if (i.c.a < -2.0005) { discard; }
	let c = world_texel(i);
	let ra = world_alpha(c);
	if (ra * 128.0 >= 64.0 || ra <= 0.0) { discard; }
	let r = vec4<f32>(tie_rgb(i, c), ra);
	soft_mark(i, r.a); return r; // [PC] the fog's no-Z texel (volfog/soft.rs)
}

// Shrub mesh texel: the draw's bind group samples with its material's GS CLAMP wrap (bindless.rs rewrites this one), at the
// level its TEX1 picks (`vs_shrub`; the slot holds the authored mip chain).
fn shrub_mesh_texel(i: VOut) -> vec4<f32> {
	return textureSampleLevel(tex, samp, i.uv, gs_level(i));
}

// [PC] Enhanced (u.sun_texel.w = 1; applied by the callers: bindless.rs rewrites `shrub_mesh_texel` by its text): a magnified leaf texture's alpha ramp is cut at half alpha over one screen pixel, so a leaf
// close to the camera keeps a sharp solid outline instead of a wide see-through blur; far away (minified) the ramp spans less
// than a pixel and the texel alpha is kept. Classic leaves the texel as the GS samples it.
fn crisp_leaf(i: VOut, c: vec4<f32>) -> vec4<f32> {
	let w = fwidth(c.a);
	if (u.sun_texel.w <= 0.0 || (i.l.w < -2.1 && i.l.w > -2.15) || w >= 0.5) { return c; }
	return vec4<f32>(c.rgb, clamp((c.a - 0.5) / max(w, 1e-4) + 0.5, 0.0, 1.0));
}

@fragment fn fs_shrub_ge12(i: VOut) -> @location(0) vec4<f32> {
	let c = crisp_leaf(i, shrub_mesh_texel(i));
	let ra = shrub_alpha(i, c);
	if (ra * 128.0 < 12.0) { discard; }
	return vec4<f32>(shrub_rgb(i, c), ra);
}

@fragment fn fs_shrub_lt12(i: VOut) -> @location(0) vec4<f32> {
	let c = crisp_leaf(i, shrub_mesh_texel(i));
	let ra = shrub_alpha(i, c);
	if (ra * 128.0 >= 12.0) { discard; }
	let r = vec4<f32>(shrub_rgb(i, c), ra);
	soft_mark(i, r.a); return r; // [PC] the fog's no-Z texel (volfog/soft.rs)
}

@fragment fn fs_shrub_ge96(i: VOut) -> @location(0) vec4<f32> {
	let c = shrub_bb_texel(i);
	let ra = shrub_alpha(i, c);
	if (ra * 128.0 < 96.0) { discard; }
	return vec4<f32>(shrub_rgb(i, c), ra);
}

@fragment fn fs_shrub_lt96(i: VOut) -> @location(0) vec4<f32> {
	let c = shrub_bb_texel(i);
	let ra = shrub_alpha(i, c);
	if (ra * 128.0 >= 96.0) { discard; }
	let r = vec4<f32>(shrub_rgb(i, c), ra);
	soft_mark(i, r.a); return r; // [PC] the fog's no-Z texel (volfog/soft.rs)
}

@fragment fn fs_shrub_all(i: VOut) -> @location(0) vec4<f32> {
	let c = shrub_bb_texel(i);
	return vec4<f32>(shrub_rgb(i, c), shrub_alpha(i, c));
}

fn moby_alpha(i: VOut) -> f32 {
	return 128.0 - round(abs(i.c.a - floor(i.c.a + 0.5)) * 512.0);
}

// A moby texel: the plain sample, or the level the material's TEX1 picks (`gs_level`; i.kmx.y >= 0 only with a draw word).
fn moby_texel(i: VOut) -> vec4<f32> {
	let c = textureSample(tex, samp, i.uv);
	if (i.c.a >= 0.0 && i.kmx.y >= 0.0) { return textureSampleLevel(tex, samp, i.uv, gs_level(i)); }
	return c;
}

fn moby_a(i: VOut, c: vec4<f32>) -> f32 {
	return floor(floor(c.a * 128.0 + 0.5) * moby_alpha(i) / 128.0) / 128.0;
}

fn moby_rgb(i: VOut, c: vec4<f32>) -> vec3<f32> {
	var rgb = c.rgb * i.c.rgb * sun_lit(i);
	if (i.c.a < -1.5) {
		// Tinted tail packet: constant unlit colour, same alpha as the lit packets.
		rgb = c.rgb * sun_baked(min(i.c.rgb, vec3<f32>(255.0 / 128.0)), i.w); // [PC] Enhanced sun shadow, as the opaque tail texels of `fs`
	}
	if (u.keep_tail.y > 0.0 && i.metal != 0u) { rgb = metal_look(i, rgb); }
	let d = distance(i.w, u.eye.xyz);
	let f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	return mix(u.fog_colour.rgb, rgb, f);
}

@fragment fn fs_moby_a(i: VOut) -> @location(0) vec4<f32> {
	let ta = floor(moby_texel(i).a * 128.0 + 0.5);
	if (moby_alpha(i) == 128.0 && (ta >= 128.0 || (i.c.a < 0.0 && !moby_tail(i)))) { discard; }
	let c = moby_texel(i);
	let ra = moby_a(i, c);
	if (ra * 128.0 < 96.0) { discard; }
	return vec4<f32>(moby_rgb(i, c), ra);
}

@fragment fn fs_moby_b(i: VOut) -> @location(0) vec4<f32> {
	let ta = floor(moby_texel(i).a * 128.0 + 0.5);
	if (moby_alpha(i) == 128.0 && (ta >= 128.0 || (i.c.a < 0.0 && !moby_tail(i)))) { discard; }
	let c = moby_texel(i);
	let ra = moby_a(i, c);
	if (ra * 128.0 >= 96.0) { discard; }
	return vec4<f32>(moby_rgb(i, c), ra);
}

// Particles: billboards and ribbons built on the CPU, texture times vertex colour.
@fragment fn fs_part(i: VOut) -> @location(0) vec4<f32> {
	let c = textureSample(tex, samp, i.uv) * i.c;
	return vec4<f32>(gs_fog(i, c.rgb), c.a);
}

// GS fog (PRIM FGE) of an effects-layer prim, l.w = -5 (`fog_class`): C = (F * C + (255 - F) * FOGCOL) >> 8 on the 8-bit colour, F
// the vertex fog byte interpolated (truncated to an integer), FOGCOL in l.y (r | g << 8 | b << 16; -1 = the level fog colour).
fn gs_fog(i: VOut, rgb: vec3<f32>) -> vec3<f32> {
	if (i.l.w > -4.5 || i.l.w < -5.5) {
		return rgb;
	}
	let f = floor(clamp(i.fogf, 0.0, 255.0));
	let c8 = floor(min(rgb, vec3<f32>(1.0)) * 255.0 + 0.001);
	var fc = floor(u.fog_colour.rgb * 255.0 + 0.5);
	if (i.l.y >= 0.0) {
		let pk = u32(i.l.y);
		fc = vec3<f32>(f32(pk & 255u), f32((pk >> 8u) & 255u), f32((pk >> 16u) & 255u));
	}
	return floor((c8 * f + fc * (255.0 - f)) / 256.0) / 255.0;
}

// [PC] Particles Enhanced (partfx.rs; with Lighting Enhanced): the same particles in their own pass, fading out where they
// meet the scene (soft.p = (near, far, fade distance, glow gain)); the additive ones also write their light into the glow
// target the bloom adds (post.rs `fs_bright`); its alpha is their cover (the water pass keeps them). Classic draws `fs_part` only.
struct SoftU { p: vec4<f32>, n: vec4<f32>, l: array<vec4<f32>, 16> };
@group(2) @binding(0) var<uniform> soft: SoftU;
@group(2) @binding(1) var soft_depth: texture_depth_multisampled_2d;
struct SoftOut { @location(0) c: vec4<f32>, @location(1) glow: vec4<f32> };

fn soft_fade(pos: vec4<f32>) -> f32 {
	let n = soft.p.x;
	let f = soft.p.y;
	let zs = textureLoad(soft_depth, vec2<i32>(pos.xy), 0);
	let ls = n * f / (f - zs * (f - n));
	let lf = n * f / (f - pos.z * (f - n));
	return clamp((ls - lf) / soft.p.z, 0.0, 1.0);
}

// [PC] The effect lights (partfx.rs `lights`, soft.n.x in use) at a smoke particle: no facing, reach falloff squared.
fn fx_smoke(p: vec3<f32>) -> vec3<f32> {
	var acc = vec3<f32>(0.0);
	for (var k = 0u; k < u32(soft.n.x); k++) {
		let a = soft.l[2u * k];
		let d = distance(a.xyz, p);
		if (d >= a.w) { continue; }
		let f = 1.0 - d / a.w;
		acc += soft.l[2u * k + 1u].rgb * f * f;
	}
	return acc;
}

@fragment fn fs_part_soft(i: VOut) -> SoftOut {
	var c = textureSample(tex, samp, i.uv) * i.c;
	c.a *= soft_fade(i.pos);
	if (soft.n.x > 0.0) {
		// Grey smoke glows in the light of the fire beside it; coloured (flame) texels keep their colour.
		let mx = max(c.r, max(c.g, c.b));
		let sat = (mx - min(c.r, min(c.g, c.b))) / max(mx, 1e-4);
		c = vec4<f32>(c.rgb + (c.rgb + vec3<f32>(0.03)) * fx_smoke(i.w) * soft.n.y * (1.0 - sat), c.a);
	}
	var o: SoftOut;
	o.c = c;
	// Alpha: the particle's cover, so the water pass leaves particles over water as drawn (waterfx.rs `fs_water`).
	o.glow = vec4<f32>(0.0, 0.0, 0.0, c.a);
	return o;
}

@fragment fn fs_part_soft_add(i: VOut) -> SoftOut {
	var c = textureSample(tex, samp, i.uv) * i.c;
	c.a *= soft_fade(i.pos);
	var o: SoftOut;
	o.c = c;
	o.glow = vec4<f32>(c.rgb * c.a * soft.p.w, max(c.r, max(c.g, c.b)) * c.a);
	return o;
}

// [PC] Water Enhanced with Particles Classic: the pool once more, writing only its cover into the glow alpha (colour masked off),
// so the water pass keeps the particles drawn over the water as they are (waterfx.rs `fs_water`).
@fragment fn fs_part_cover(i: VOut) -> SoftOut {
	let c = textureSample(tex, samp, i.uv) * i.c;
	var o: SoftOut;
	o.c = c;
	o.glow = vec4<f32>(0.0, 0.0, 0.0, c.a);
	return o;
}

@fragment fn fs_part_cover_add(i: VOut) -> SoftOut {
	let c = textureSample(tex, samp, i.uv) * i.c;
	var o: SoftOut;
	o.c = c;
	o.glow = vec4<f32>(0.0, 0.0, 0.0, max(c.r, max(c.g, c.b)) * c.a);
	return o;
}

// KER-92: pool lines (mode 2): untextured Gouraud vertex colour, no fog.
@fragment fn fs_line(i: VOut) -> @location(0) vec4<f32> {
	return i.c;
}

// BRIDGE-23: GS FIX blend strips. The blend factor is the pipeline constant; texture and vertex alpha are ignored
// (alpha 1), the colour is fogged like the world.
@fragment fn fs_part_fix(i: VOut) -> @location(0) vec4<f32> {
	let c = textureSample(tex, samp, i.uv) * i.c;
	// Effects-layer strips: GS fog (l.w = -5) or none (l.w = -6, PRIM FGE 0), see `gs_fog`.
	if (i.l.w < -4.5 && i.l.w > -5.5) {
		return vec4<f32>(gs_fog(i, c.rgb), 1.0);
	}
	if (i.l.w < -5.5 && i.l.w > -6.5) {
		return vec4<f32>(c.rgb, 1.0);
	}
	let d = distance(i.w, u.eye.xyz);
	let f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	return vec4<f32>(mix(u.fog_colour.rgb, c.rgb, f), 1.0);
}

// ARI-49b: the Aridia ocean (2a3a20, decomp/notes/aridia_presentation.md): texture x vertex colour (MODULATE, the
// colour is already /128), opaque, with its own fog words (near 0, far 260080 / 1024 world units, 255 -> 0) and
// FOGCOL (15, 15, 25). [I] fog distance = Euclidean eye distance, as the world shaders.
@fragment fn fs_ocean(i: VOut) -> @location(0) vec4<f32> {
	let c = textureSample(tex, samp, i.uv) * i.c;
	let d = distance(i.w, u.eye.xyz);
	let f = clamp(255.0 + d * (0.0 - 255.0) / 253.98438, 0.0, 255.0) / 255.0;
	return vec4<f32>(mix(vec3<f32>(15.0, 15.0, 25.0) / 255.0, c.rgb, f), 1.0);
}

// KER-30: the Kerwan ocean (0x290b48, decomp/notes/kerwan_environment.md): as fs_ocean with its fog words near 10000,
// far 260080 (/ 1024 world units), 255 -> 0, FOGCOL (96, 96, 96). Alpha 1: the FIX blend uses the pipeline constant.
@fragment fn fs_ocean_k(i: VOut) -> @location(0) vec4<f32> {
	let c = textureSample(tex, samp, i.uv) * i.c;
	let d = distance(i.w, u.eye.xyz);
	let f = clamp(255.0 + (d - 9.765625) * (0.0 - 255.0) / 244.21875, 0.0, 255.0) / 255.0;
	return vec4<f32>(mix(vec3<f32>(96.0, 96.0, 96.0) / 255.0, c.rgb, f), 1.0);
}

// Retail-mode oceans of Rilgar, Umbris and Gaspar (retail/oldsim.rs `RamOcean`): as fs_ocean with the fog the
// ocean's setup installs, carried per vertex: n.xy = near / far (world units), l = (intensity near, intensity far, FOGCOL
// r | g << 8 | b << 16, 1 = the level fog stays while the Visibomb is live).
@fragment fn fs_ocean_g(i: VOut) -> @location(0) vec4<f32> {
	let c = textureSample(tex, samp, i.uv) * i.c;
	let d = distance(i.w, u.eye.xyz);
	var near = i.n.x;
	var far = i.n.y;
	var f0 = i.l.x;
	var f1 = i.l.y;
	let p = u32(i.l.z + 0.5);
	var col = vec3<f32>(f32(p & 255u), f32((p >> 8u) & 255u), f32((p >> 16u) & 255u)) / 255.0;
	if (i.l.w > 0.5) {
		near = u.fog.x;
		far = u.fog.y;
		f0 = u.fog.z;
		f1 = u.fog.w;
		col = u.fog_colour.rgb;
	}
	let f = clamp(f0 + (d - near) * (f1 - f0) / max(far - near, 0.001), min(f0, f1), 255.0) / 255.0;
	return vec4<f32>(mix(col, c.rgb, f), 1.0);
}

// Retail-mode oceans built from RAM (Batalia, Oltanis): the fs_ocean_k fog words (identical in every ocean descriptor,
// +0x20..+0x2c) with the descriptor's FOGCOL carried in the flat vertex light (queue_q1::ocean).
@fragment fn fs_ocean_f(i: VOut) -> @location(0) vec4<f32> {
	let c = textureSample(tex, samp, i.uv) * i.c;
	let d = distance(i.w, u.eye.xyz);
	let f = clamp(255.0 + (d - 9.765625) * (0.0 - 255.0) / 244.21875, 0.0, 255.0) / 255.0;
	return vec4<f32>(mix(i.l.rgb, c.rgb, f), 1.0);
}

// Water: the particle colour with the scene fog (GS fog before the alpha blend, same coefficient as the world).
@fragment fn fs_part_fog(i: VOut) -> @location(0) vec4<f32> {
	let c = textureSample(tex, samp, i.uv) * i.c;
	let d = distance(i.w, u.eye.xyz);
	let f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	return vec4<f32>(mix(u.fog_colour.rgb, c.rgb, f), c.a);
}

@vertex fn vs_sky(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>) -> VOut {
	var o: VOut;
	// Pinned to the far plane so the world always draws over it.
	o.pos = (u.sky_view_proj * vec4<f32>(p, 1.0)).xyww;
	o.uv = uv;
	o.n = n;
	o.c = c;
	return o;
}

@fragment fn fs_sky(i: VOut) -> @location(0) vec4<f32> {
	return textureSample(tex, samp, i.uv) * i.c;
}

// Overlay: positions are already in clip space.
@vertex fn vs_hud(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>) -> VOut {
	var o: VOut;
	o.pos = vec4<f32>(p.xy, 0.0, 1.0);
	o.uv = uv;
	o.n = n;
	o.c = c;
	return o;
}

// Depth reset for the pause-menu item preview: clip z = 1 (far), written under compare Always.
@vertex fn vs_hudz(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>) -> VOut {
	var o: VOut;
	o.pos = vec4<f32>(p.xy, 1.0, 1.0);
	o.uv = uv;
	o.n = n;
	o.c = c;
	return o;
}

@fragment fn fs_hud(i: VOut) -> @location(0) vec4<f32> {
	return textureSample(tex, samp, i.uv) * i.c;
}

// Retail screen filter (retail/queue_b.rs `screen_quads`): untextured, the vertex colour as is.
@fragment fn fs_hud_flat(i: VOut) -> @location(0) vec4<f32> {
	return i.c;
}
"#);
#[derive(Clone, Copy)]
pub(crate) struct Draw {
	/// First-instance value: the tfrag pass reads its mip bias and top level from it (`vs_tfrag`).
	inst: u32,
	/// 0 none, 1 cull back faces, 2 cull front faces (the moby packet's VU1 cull mode, NOV-5.9a).
	cull: u8,
	texture: usize,
	start: u32,
	count: u32,
}

/// One far-LOD shrub billboard of the frame: corners in fan order (c0, c1, c3, c2), the class texture key, the vertex colour
/// (rgb / 128, alpha carrier -2 - a / 256) and which GS pass it belongs to (a 0x80 alpha = pass 1).
pub(crate) struct ShrubQuad {
	pos: [[f32; 3]; 4],
	colour: [f32; 4],
	class_id: u32,
	lod_k: f32,
	mxl: f32,
	pass1: bool,
}

/// Vertex and index buffers with one draw per run of faces sharing a texture.
/// The sky batch's CPU side for the turning shells (`Looks::shell_angle`).
pub(crate) struct SkyShells {
	verts: Vec<Vertex>,
	ranges: Vec<std::ops::Range<usize>>,
	base: Vec<Vec<[f32; 3]>>,
	/// Per shell: its vertices hold a turned pose (to restore when it stops turning).
	turned: Vec<bool>,
	/// (level, frames) of the last pose written.
	posed: Option<(usize, u32)>,
	/// ARI-4: the number of the sky batch's draws that belong to shells 0 and 1 (the sky sprites draw between them and shell 2).
	split: usize,
}

/// ARI-4: the Aridia sky sprites (`sky::sprite_draws`), grouped by texture: up to 256 quads, sky textures 2 then 3.
pub(crate) struct SkySpriteDraw {
	vbuf: wgpu::Buffer,
	/// Fixed: quad k is vertices 4k..4k+3, triangles (0, 1, 2), (1, 2, 3).
	ibuf: wgpu::Buffer,
	/// Bind groups of sky textures `4 << 16 | 2` and `4 << 16 | 3`.
	bind: [usize; 2],
	/// This frame's quads per texture (texture 2's first in the buffer).
	counts: [u32; 2],
}

pub struct Batch {
	pub vbuf: wgpu::Buffer,
	ibuf: wgpu::Buffer,
	draws: Vec<Draw>,
}

pub struct Scene {
	world: Batch,
	/// [PC] Metal On: the level's flat metal floors (metalplanar.rs, the planar floor reflection).
	metal_floors: metalenv::Floors,
	/// The world vertices as uploaded, and the dynamic-light assignment over them (`Gpu::relight`).
	pub world_verts: Vec<Vertex>,
	lit: dynlight::LitWorld,
	/// BRIDGE-29 per-instance ties (decomp/notes/novalis_enemies.md REN-CULL-TIE): all three LOD meshes of every instance in
	/// `ties`, whose `draws` `Gpu::cull_ties` rebuilds each frame from the static per (instance, LOD, texture) `tie_draws`
	/// (`tie_ranges[i][lod]` = the range of that LOD in `tie_draws`). `tie_verts` hold the CURRENT vertices (morphed positions,
	/// relit colours), `tie_base` the unmorphed positions, `tie_morph` each vertex's world delta, `tie_vranges[i][lod]` the
	/// vertex range of a LOD mesh, `tie_fat[i][lod]` whether it has any delta, and `tie_q[i][lod]` the geomorph factor last
	/// written into it. `tie_first` is the first tie vertex in the `LitWorld` address space (= the world vertex count).
	ties: Batch,
	tie_draws: Vec<Draw>,
	tie_ranges: Vec<[std::ops::Range<u32>; 3]>,
	tie_cull: Vec<ties::TieCull>,
	/// [PC] Dynamic shadows (sunshadow.rs): the LOD `Gpu::cull_ties` drew each tie with this frame (u8::MAX = not drawn), so a
	/// tie casts with the very mesh it is seen as.
	tie_lod: Vec<u8>,
	/// REN-CULL-OCC / BRIDGE-30: the level's occlusion grid and this frame's mask (`Gpu::set_clip_view` updates it).
	pub occ: crate::occlusion::Occlusion,
	/// FRAME-1a: the occlusion state of the per-tick draw verdict (its own `last` fallback, rebuilt once per tick).
	occ_tick: crate::occlusion::Occlusion,
	tie_verts: Vec<Vertex>,
	tie_base: Vec<[f32; 3]>,
	tie_morph: Vec<[f32; 3]>,
	tie_vranges: Vec<[std::ops::Range<usize>; 3]>,
	tie_fat: Vec<[bool; 3]>,
	tie_q: Vec<[f32; 3]>,
	/// BRIDGE-32: each tie vertex's colour slots (parallel to `tie_verts`), each instance's current 64 slot colours, and per
	/// (instance, LOD) whether the range's vertex colours are stale after a relight.
	tie_slots: Vec<ties::TieSlot>,
	/// [PC] perf round 3: per tie vertex its fat slot triple's index in `tie_tris[i][lod]` (distinct triples; u32::MAX regular).
	tie_tri: Vec<u32>,
	tie_tris: Vec<[Vec<[usize; 3]>; 3]>,
	tie_cols: Vec<Vec<[f32; 4]>>,
	tie_stale: Vec<[bool; 3]>,
	/// PC perf: per tie vertex whether its colour or position depends on the geomorph factor Q (a fat colour slot or a
	/// non-zero morph delta), and per tie LOD whether `cull_ties` has rewritten the whole range once. A Q-only change of a
	/// fully rewritten range touches just the Q-dependent vertices (the others would be rewritten to the same values).
	tie_qdep: Vec<bool>,
	tie_full: Vec<[bool; 3]>,
	tie_first: usize,
	/// BRIDGE-27 / REN-CULL-SHRUB: shrub instances outside the merged world. `shrubs.draws` is the static per (instance, texture)
	/// list, `shrub_ranges[i]` = (range in it, vertex range in `shrub_verts`); `shrub_order` walks the instances by class load
	/// slot; `shrub_a[i]` is the alpha last written into instance i's vertex colours (0x80 opaque); `shrub_opaque` / `shrub_faded`
	/// hold this frame's indices into `shrubs.draws`; `shrub_bb_quads` this frame's billboard quads (pass 1 then pass 2);
	/// `shrub_bb_tex` the class billboard texture chains (bind group, mip count - 1). `shrub_first` is the first shrub vertex in
	/// the `LitWorld` address space.
	shrubs: Batch,
	shrub_ranges: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>,
	shrub_cull: Vec<shrubs::ShrubCull>,
	shrub_order: Vec<usize>,
	shrub_bb: Vec<Option<shrubs::ShrubBillboard>>,
	shrub_verts: Vec<Vertex>,
	shrub_first: usize,
	/// [PC] icefx.rs: per tie / shrub vertex the texture key of a face using it, and the level whose ice marks the vertices carry.
	tie_vkey: Vec<u32>,
	shrub_vkey: Vec<u32>,
	ice_level: usize, shrub_soft: Vec<u32>,
	shrub_a: Vec<u8>,
	shrub_opaque: Vec<usize>,
	shrub_faded: Vec<usize>,
	shrub_bb_quads: Vec<ShrubQuad>,
	shrub_bb_tex: HashMap<u32, (wgpu::BindGroup, u32)>,
	/// BRIDGE-31 / REN-CULL-TFRAG: the terrain fragments, outside the merged world. `tfrag_set` holds every fragment's LOD
	/// meshes and morph tables, `tfrags` the batch of all three LOD meshes of every fragment whose `draws` `Gpu::cull_tfrags`
	/// rebuilds each frame from the static `tfrag_draws` (`tfrag_ranges[i][lod]`); `tfrag_verts` is what was last written
	/// (`tfrag_vranges[i][lod]` the vertex range of a LOD), `tfrag_cols[i]` the current colour of each position record of
	/// fragment i (relit by `Gpu::relight`) and `tfrag_b` the block's LOD scale.
	tfrags: Batch,
	pub tfrag_set: Vec<terrain::TfragLods>,
	tfrag_draws: Vec<Draw>,
	tfrag_ranges: Vec<[std::ops::Range<u32>; 3]>,
	tfrag_vranges: Vec<[std::ops::Range<usize>; 3]>,
	tfrag_verts: Vec<Vertex>,
	tfrag_cols: Vec<Vec<[f32; 4]>>,
	tfrag_b: f32,
	/// [PC] Dynamic shadows (sunshadow.rs): the LOD `cull_tfrags` drew each fragment with (u8::MAX = not drawn); it casts with that mesh (a coarser one shadowed the ground under it).
	tfrag_lod: Vec<u8>,
	tfrag_memo: Vec<[u8; 3]>, // [PC] perf (ncull, gpu/tfrag_memo.rs): per fragment and LOD range the plain code (6 / 0x14) last posed into it, else 0
	sky: Option<Batch>,
	/// NOV-31: the vertices of `sky` with each shell's range and unturned positions (`Gpu::set_sky_spin`).
	sky_shells: Option<SkyShells>,
	/// ARI-4: the sky sprite batch (skies with textures 2 and 3).
	sky_sprites: Option<SkySpriteDraw>, stars: Option<starfield::StarMaps>, // [PC] stars: starfield.rs, Enhanced only
	/// The player's model (Ratchet, or a box if he failed to decode): `marker_model` holds its
	/// model-space vertices, re-posed into `marker.vbuf` every frame; drawn when `marker_shown`.
	marker: Batch,
	marker_model: Vec<Vertex>,
	/// Ratchet's class tail colour (CLASS-tint): (rgb, first vertex) over the first `marker_hero_len` marker vertices; the appended
	/// weapon meshes are other mobies and stay lit. PC perf: the posed marker vertices of the last `set_marker` (buffer reused every frame).
	marker_verts: Vec<Vertex>,
	marker_tint: Option<([f32; 3], usize)>,
	marker_hero_len: usize,
	/// [PC] Fur (viewer/furfx.rs): per marker draw, the fur part its texture is (0 = none), from the pack's `fur` table.
	marker_fur: Option<furfx::FurScene>,
	marker_shown: bool,
	/// BRIDGE-28: the marker carries translucent wrench afterimages this frame (set by `Gpu::set_marker`).
	marker_alpha: bool,
	/// Crates, bolts and enemies: one model copy per `game::Prop` slot (`props_ranges`), re-posed
	/// into `props.vbuf` every frame; hidden slots collapse to a point.
	pub props: Batch,
	/// [PC] perf (lane A7): the index buffer a pool growth last moved the props indices out of; the next growth copies them
	/// back into it (`Gpu::append_props`). None until a pool grows.
	props_ibuf_spare: Option<wgpu::Buffer>,
	pub props_model: Vec<Vertex>,
	pub props_ranges: Vec<std::ops::Range<usize>>,
	/// [PC] perf round 3 (gpuskin.rs): the static (slot, vertex) attribute of the props batch, and this
	/// frame's slot records and pose entries as uploaded (the clipper, the prompt and the bench mirror read them).
	props_sj: Option<wgpu::Buffer>,
	/// Per slot the largest distance of a model vertex from the model origin (the clipper's sphere pre-test, gpuskin.rs).
	props_rad: Vec<f32>,
	props_inst: Vec<gpuskin::PropInst>,
	props_pose: Vec<gpuskin::PoseEntry>,
	props_shown: bool,
	props_alpha: bool,
	/// BRIDGE-33 / REN-MOBY-DRAWN: per prop slot whether this frame's moby draw would have drawn it (shown, not occluded, inside
	/// the draw distance, inside the near / side planes); `Game::set_draw_verdict` feeds it back to the scheduler override.
	props_drawn: Vec<bool>,
	/// BRIDGE-64a: per props slot whether `set_props` shows it this frame, each slot's runs in `props` (draw, first index,
	/// count; `shadowvol::slot_runs`) and the slots `set_volumes` moved after the shadow volumes (empty = no split).
	props_vis: Vec<bool>,
	slot_runs: Vec<Vec<(u32, u32, u32)>>,
	/// [PC] The class of each prop slot and the order their faces sit in `props` (slot indices; `Gpu::append_props`
	/// rebuilds the draws in it after a retail pool grew).
	props_class: Vec<u32>,
	props_order: Vec<usize>,
	/// [PC] Each prop class's faces as the props batch groups them (`Gpu::append_props`).
	class_faces: HashMap<u32, gpu::ClassFaces>,
	props_late: Vec<usize>, tail: gpu::tail_order::TailOrder, // [PC] #29: the mobys drawn after the blended no-Z texels (gpu/tail_order.rs)
	/// NOV-5.9c: per props slot the faces the CPU clipper may take (`clip::PropFace`), the (slot, face) pairs currently
	/// masked out of `props.ibuf`, the growable overlay batch of the clipped polygons (capacity in vertices, indices),
	/// this frame's view basis (None = no clipping) and the polygons queued for the next flush.
	prop_faces: Vec<Vec<clip::PropFace>>,
	clip_masked: Vec<(u32, u32)>,
	clip_batch: Batch,
	clip_cap: (usize, usize),
	clip_view: Option<clip::ClipView>,
	clip_queue: Vec<clip::ClipPoly>,
	/// NOV-5.9d: the hero marker's clippable faces (marker model incl. the held gadgets) and those masked in `marker.ibuf`.
	marker_faces: Vec<clip::PropFace>,
	marker_masked: Vec<u32>,
	/// PC perf (lane A, B2): the inputs and skinned pose the kept `marker_verts` were built from (viewer/gpucache.rs).
	marker_key: Option<gpucache::MarkerKey>,
	marker_skin: (Vec<[f32; 3]>, Vec<[f32; 3]>),
	/// [PC] perf (lane R2): the marker built by a compute pass (gpuhero.rs); None = CPU path only.
	marker_gpu: Option<gpuhero::HeroGpu>,
	/// PC perf (lane A, B3): the inputs of the last `cull_ties` pass and its reused job list.
	tie_pass: Option<gpucache::TiePassKey>,
	tie_jobs: Vec<tiegpu::Job>,
	/// [PC] perf (lane D5b): the GPU vertex rewrite of the ties (None = the CPU rewrite does it all).
	tie_gpu: Option<tiegpu::TieGpu>,
	/// Debris pieces: one unposed model per class (vertices, and face indices by texture slot). Each frame the live
	/// pieces are instanced into `debris` (capacity MOBY_LIMIT pieces of the largest class), so no per-class cap.
	/// Third element: per texture slot one strip-phase byte per face, parallel to the index list (255 = never clip).
	debris_model: HashMap<u32, (Vec<Vertex>, Vec<(usize, Vec<u32>)>, Vec<(usize, Vec<u8>)>)>,
	debris: Option<Batch>,
	debris_cap: (usize, usize),
	/// BRIDGE-35: the arrival warp's ship classes (own textures, fresh bind slots) and the per-frame skinned batch;
	/// while `warp_shown` the level's batches and world quads are not drawn.
	warp_model: HashMap<u32, (Vec<Vertex>, Vec<(usize, Vec<u32>)>, Vec<(usize, Vec<u8>)>)>,
	warp_batch: Option<Batch>,
	pub warp_shown: bool,
	/// BRIDGE-36 (c1): the purchase prompt's one-prop batch and its slot in `props` (UI-5d).
	prompt: Option<Batch>,
	prompt_slot: Option<usize>,
	/// BRIDGE-37 (b): the shop portrait (class 12) one-prop batch and its mesh-order base vertices.
	portrait: Option<Batch>,
	portrait_model: Vec<Vertex>,
	/// Pause-menu item previews by held model class: batch, centre, radius (decomp/notes/ingame_pause_menu.md).
	pub(crate) previews: HashMap<u32, (Batch, [f32; 3], f32)>,
	/// LIT-7a: the same preview models with the gold light word (slots 12 / 12) in their vertices.
	previews_gold: HashMap<u32, Batch>,
	/// Unposed vertices of the hero previews (keys >= HERO_PREVIEW), re-posed every frame by `set_preview_pose`.
	preview_models: HashMap<u32, Vec<Vertex>>,
	bind_groups: Vec<wgpu::BindGroup>,
	/// Texture key -> `bind_groups` index of every texture the scene uploaded: `Gpu::regrow_props` rebuilds the prop slots with it (retail pool growth) without a new upload.
	tex_slots: HashMap<u32, usize>,
	/// [PC] Dynamic shadows: per texture slot, whether most of its texels blend (PS2 alpha below 0x60, `fs_moby_b`): see-through
	/// effect shells (blast domes, glows) cast no sun shadow (sunshadow.rs).
	clear_tex: Vec<bool>, /** [PC] lane v042_shadow: per texture slot the smallest texel alpha (texture value 0..255; 0 = unknown; all 0 with a textures mod only when its disc originals are not kept: lane nsun, gpu/split_orig.rs `sun_disc`; the cuts then sample `Scene::sun_tex`): a caster whose slot never goes below the cut casts depth-only (sunshadow.rs) */ sun_alpha: Vec<u8>, /** [PC] lane v042_shadow: per props slot the radius of its sphere, parallel to `props_inst` (the records the GPU holds; infinite = unbounded; shorter or empty = no per-slot test), for the cascade cull (sunshadow/props.rs) */ sun_rad: Vec<f32>, /** [PC] lane v042_shadow: tight boxes of the tie LODs and shrubs, built with the scene (sunshadow/bounds.rs) */ sun_bb: sunshadow::bounds::Boxes,
	/// `bind_groups` index -> the first-instance word of a moby draw with that slot (TEX1 K and chain top level; 0 = plain sample).
	slot_words: Vec<u32>,
	/// [PC] perf round 3: `bind_groups` as one texture array (bindless.rs; None = per-draw bind groups).
	tex_array: Option<wgpu::BindGroup>,
	repl: Option<Box<gpu::repl_upload::ReplScene>>, // [PC] texture packs: slots still waiting for their replacement (None without a textures mod)
	orig: Option<Box<gpu::split_orig::OrigSet>>, // [PC] split view with a pack: the scene's slots with the original disc textures (None unless kept)
	clear: wgpu::Color,
	pub centre: [f32; 3],
	pub radius: f32,
}

thread_local! {
	/// Scene-actor moby tints (+0x90, FUN_00206f78) by actor slot for the next `set_props`; decomp/notes/veldin_ambient.md section 5.
	/// (RGB with 0x80 = 1.0, first overridden vertex = `MobyClass::colour_start`): FUN_00255480 replaces the lit
	/// colour of packets [class byte 0x0a, LOD end) with +0x90, so only that subset changes and it is drawn unlit.
	/// Placed-prop moby tints by `game.props` slot (VEND-tint: the vendor's +0x90 grey), same meaning as `ACTOR_TINTS`.
	pub static PROP_TINTS: std::cell::RefCell<Vec<Option<([f32; 3], usize)>>> = const { std::cell::RefCell::new(Vec::new()) };
	pub static ACTOR_TINTS: std::cell::RefCell<Vec<Option<([f32; 3], usize)>>> = const { std::cell::RefCell::new(Vec::new()) };
}


pub struct Gpu {
	/// [PC] Presents on a worker thread (presenter.rs); flush() before the surface is reconfigured. Before `surface`: fields drop in order, so the thread is joined before the surface goes (else a window close destroyed it under the thread's SurfaceTexture: wgpu panic, exit 101).
	presenter: presenter::Presenter,
	pub window: Arc<Window>,
	surface: wgpu::Surface<'static>,
	pub device: wgpu::Device,
	/// [PC] Gated on the deferred submit of the previous frame (presenter.rs `GatedQueue`): every use waits for it.
	pub queue: presenter::GatedQueue,
	pub config: wgpu::SurfaceConfiguration,
	/// [PC] This and the other `metalpipe::Pipes` fields: one per Metal On / Off set (metalpipe.rs), the current pass's by Deref.
	pipeline: metalpipe::Pipes<wgpu::RenderPipeline>,
	pipeline_cull: metalpipe::Pipes<[wgpu::RenderPipeline; 2]>,
	moby_pipelines: metalpipe::Pipes<[wgpu::RenderPipeline; 2]>,
	/// `moby_pipelines` with the packet's VU1 cull mode (index 2 * (cull - 1) + pass; mobies::CULL_BACK / CULL_FRONT).
	moby_cull: metalpipe::Pipes<[wgpu::RenderPipeline; 4]>,
	/// REN-CULL-SHRUB: faded mesh with / without Z write (texel alpha x A >= 12), billboard pass 1 with / without Z (>= 96), pass 2.
	shrub_pipelines: metalpipe::Pipes<[wgpu::RenderPipeline; 5]>,
	/// Billboard textures clamp at the edges (GS CLAMP_1 = 5).
	pub clamp_sampler: wgpu::Sampler,
	/// Shrub mesh CLAMP wraps 1..=3 (u, v, both clamped); see `shrubs::WRAP_SHIFT`.
	wrap_samplers: [wgpu::Sampler; 3],
	tfrag_pipeline: metalpipe::Pipes<wgpu::RenderPipeline>,
	tfrag_pipeline_lt: metalpipe::Pipes<wgpu::RenderPipeline>,
	/// Tie draws (`fs_tie`, `fs_tie_lt64`): [pass][cull: none, back, front]; pass 0 texel alpha >= 0x40 (Z written), pass 1 below.
	tie_pipelines: metalpipe::Pipes<[[wgpu::RenderPipeline; 3]; 2]>,
	sky_pipeline: wgpu::RenderPipeline,
	/// ARI-4: the sky sprites (ALPHA_1 0x48 = Cs * As + Cd, MODULATE), at the far plane without depth writes.
	sky_add_pipeline: wgpu::RenderPipeline,
	/// Overlay quads: [alpha blended, additive (negative colour alpha)].
	hud_pipeline: [wgpu::RenderPipeline; 5],
	/// Preview depth reset, its own camera uniforms, and the item preview to draw: (class, window-pixel
	/// rect, turntable seconds).
	clear_pipeline: wgpu::RenderPipeline,
	/// Uniforms per preview slot: both draw in one pass, so they cannot share a buffer.
	pbuf: [wgpu::Buffer; 8],
	pbind: [wgpu::BindGroup; 8],
	/// (class, rect, angle or clock, pause panel-3 item id: placed by view table 0x1c4488 before the menu camera).
	pub preview: Option<(u32, [f32; 4], f32, Option<u8>)>,
	/// LIT-7a: the panel-3 item in `preview` is a gold weapon (drawn from `Scene.previews_gold` with the slot-12 light); set by the handler.
	pub preview_gold: bool,
	/// The category-3 pair moby (class 601) drawn with the Gadgets panel model, same placement.
	pub preview_pair: Option<(u32, [f32; 4], f32, Option<u8>)>,
	/// The worn batch over the hero panel (same camera, no depth reset).
	pub preview_worn: Option<(u32, [f32; 4], f32, Option<u8>)>,
	/// Second preview slot: the hero in the big panel (class key >= HERO_PREVIEW, fixed front camera).
	pub preview2: Option<(u32, [f32; 4], f32)>,
	/// NOV-13.3: the vendor shop's item models as world mobies (preview class, model matrix), uniform slots 2 and 3.
	pub shop_models: Vec<(u32, [f32; 16])>,
	/// REN-9b: the shop's panel-1 item (class, model matrix), drawn into `item_panel`.
	pub shop_panel_models: Vec<(u32, [f32; 16])>,
	/// REN-9b: the 512x128 off-screen item target blitted into shop panel 1.
	item_panel: ItemPanel,
	/// BRIDGE-36 (b): the purchase popup's 512x128 target (UI-5d), sampled by HUD quads with `POPUP_PANEL_TEX`, and the
	/// class-13 bolt drawn into it (model matrix, uniform slot 5); set by the handler.
	popup_panel: ItemPanel,
	pub popup_bolt: Option<[f32; 16]>,
	/// NOV-30d: the slot-0 class-13 bolt icon drawn into the item target after the item (model matrix, uniform slot 7).
	pub icon_bolt: Option<[f32; 16]>,
	/// NOV-35b: the draw spheres (centre and radius, 1/1024 m) of the icon and popup bolts (`Game::shop_bolt_spheres`).
	pub icon_bolt_sphere: Option<([f32; 3], f32)>,
	pub popup_bolt_sphere: Option<([f32; 3], f32)>,
	/// The preview class `pose_panel_item` last skinned (its bind vertices go back when the panel item changes).
	panel_posed: Option<u32>,
	/// BRIDGE-45: the metal packets, textures and this frame's metal batch.
	metal: MetalDraw,
	/// [PC] Metal On: the sky faces and screen-space reflection source (metalenv.rs).
	metalenv: metalenv::MetalEnv,
	/// [PC] perf round 3: the props pipelines and storage buffers of the vertex-shader posing .
	props_gpu: gpuskin::PropsGpu,
	/// [PC] perf round 3 (bindless.rs): the texture-array pipelines of the tie and shrub draws (None: per-draw bind groups), and
	/// while `load_scene` runs the texture view of every bind group `upload` / `upload_chain` made with the scene sampler.
	bindless: Option<bindless::Bindless>,
	/// BRIDGE-37 (b): the shop portrait's third 512x128 target (uniform slot 6) and whether its actor is posed this frame.
	portrait_panel: ItemPanel,
	portrait_shown: bool,
	/// BRIDGE-36 (c2): index into `hud` of the first quad drawn after the prompt model (None: the prompt stays in the props batch).
	pub hud_split: Option<usize>,
	/// [PC] The host left the game's overlay out this frame (a modern page covers it): the mods' draw layer, laid out on it, goes too.
	pub mods_hidden: bool,
	/// Overlay textures (slot 0 the built-in font), sampled nearest for crisp pixels.
	hud_textures: Vec<wgpu::BindGroup>,
	nearest: wgpu::Sampler,
	/// This frame's overlay, in window pixels.
	pub hud: Vec<Quad>,
	/// Retail mode: the GS overlay texture (width, rows), its bind group (`RETAIL_OV_TEX`) and the persistent staging buffer of its upload (gpu/ovwrite.rs).
	pub retail_ov: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32, Option<wgpu::Buffer>)>,
	/// The sharp HUD picture laid over `retail_ov` (width, rows; `RETAIL_HD_TEX`); None: only the overlay.
	pub retail_hd: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
	/// The menu art picture (width, rows; `MENU_ART_TEX`, `set_menu_art`); None: no modern page lays one over the world.
	pub menu_art: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
	/// The overlay's effect depth (gpu/ov_depth.rs; R&C2), made at its first use.
	ov_depth: Option<gpu::ov_depth::OvDepth>,
	ubuf: wgpu::Buffer,
	ubind: wgpu::BindGroup,
	tex_layout: wgpu::BindGroupLayout,
	sampler: wgpu::Sampler,
	depth: wgpu::TextureView,
	msaa: wgpu::TextureView,
	pub scene: Option<Scene>,
	/// The level's lights, zones and fog, read after `load_scene`.
	pub lighting: Option<gameplay::Lighting>,
	/// Retail mode: level-light slots 12..15 from retail RAM (retail.rs `reserved_lights`), over the level's rows.
	pub reserved_lights: Option<[[[f32; 4]; 4]; 4]>,
	/// NOV-28: the frame's fog colour and [near, far, near int, far int] (camera zone or underwater, set by the
	/// handler one frame late); None = the level fog.
	pub fog_live: Option<([u8; 3], [f32; 4])>,
	/// The shrub draw-distance cap (m): `shrubs::SHRUB_CAP`, or retail's live word in retail mode (retail.rs `RetailFx::shrub_cap`).
	pub shrub_cap: f32,
	/// NOV-28.2: live pool-particle far cap (water.rs `PART_FAR` / `PART_FAR_UNDER`).
	pub part_far: f32,
	/// [PC] The camera water flag of this field (retail.rs `SwapWords::under`) and the underwater look (underwater.rs).
	pub under_flag: bool,
	pub under_surf: Option<f32>,
	under: underwater::Under,
	/// Dynamic point-light slots for the next frame's uniforms (`Uniforms::points`, scenery mobies).
	pub points: [[f32; 4]; 16],
	/// This frame's light table with allocation serials, and the posed mobies' third lights (`dyn_id`).
	pub table: [Option<dynlight::SlotLight>; 8],
	moby_dyn: Vec<([f32; 3], [f32; 3])>,
	/// The hero instance's (class 0) placed colour and light word (`hero_light`).
	pub hero_placed: ([u8; 3], u32),
	/// The hero's sticky zone environment (hero +0x38/+0x80), reset to the placed values on load.
	pub hero_env: (usize, usize, f32, [f32; 3]),
	/// BRIDGE-57: the hero's ambient bytes and light word of the last hero draw (copied into `Game.hero_tint`).
	hero_tint_out: Option<([u8; 3], u32)>,
	/// BRIDGE-64a: the main depth target's format (with a stencil aspect), the shadow-volume pipelines, and this frame's
	/// volume vertices (increment then decrement triangles; counts in vertices).
	depth_format: wgpu::TextureFormat,
	shadow: shadowvol::ShadowPipes,
	volume_buf: Option<wgpu::Buffer>,
	volume_counts: (u32, u32),
	/// [PC] Graphics -> Lighting Enhanced (post.rs): the post-processing targets and pipelines, and whether this frame may use them
	/// (a retail level is on screen: not the front end, a movie or the GS-full reference picture; set by the handler).
	post: post::Post, volfog: volfog::VolFog, volclouds: volclouds::VolClouds, volsmoke: volsmoke::VolSmoke,
	/// [PC] Water Enhanced (waterfx.rs).
	waterfx: waterfx::WaterFx,
	/// [PC] Fur (furfx.rs): the shell pipeline, built on the first frame with fur.
	fur: furfx::FurGpu,
	/// [PC] The achievement icon atlas (gpu/ach_icons.rs), made at the first frame that draws an icon (`ICON_TEX`).
	ach_icons: gpu::ach_icons::AchIcons,
	/// [PC] Water Enhanced: the planar mirrors' uniforms (mirror.rs), one per mirror pass of a frame (`mirror::MAX_MIRRORS`).
	mirror: Vec<mirror::Mirror>,
	/// [PC] Water Enhanced: the water bed's uniforms (mirror.rs `Gpu::draw_bed`).
	bed: mirror::Mirror,
	/// [PC] Particles Enhanced (partfx.rs).
	pub partfx: partfx::PartFx,
	/// [PC] Fire Low / Medium / High (firefx.rs).
	pub firefx: firefx::FireFx,
	/// [PC] PREVIEW: more vegetation (vegfx.rs, RAC_VEG).
	pub veg: vegfx::VegFx,
	pub modern_ok: bool,
	/// [PC] Dynamic shadows (sunshadow.rs): the cascade maps, bound in the level's group 0 with their compare sampler.
	pub sun: sunshadow::SunShadow,
	/// Retail mode: a ship fight is on (`shipmouse::in_ship_fight`): space, no sun to cast from (handler.rs sets it).
	pub ship_fight: bool, pub marker_loose: [Option<loose_range::LooseRange>; loose_range::MAX], pub hud_place: Option<hud_place::Place>, pub retail_ov_guard: hud_hold::Held, reshade: gpu::reshade::Slot, rscale: gpu::rscale::RScale, upscale: upscale::Prep, pub ui_gpu: crate::ui::gpu::UiGpu, // [PC] ReShade add-on (gpu/reshade.rs); render scale state (gpu/rscale.rs); modern menu pass (ui/gpu.rs, drawn by gpu/ui_pass.rs)
	/// The loaded level (handler.rs; [PC] Water Enhanced gives Gaspar's lava its own look, waterfx.rs `LAVA_SLOT`).
	pub level: usize,
	/// SCN-3: the lighting word the scene actors copied from the hero when the scene started (FUN_00244e58); None outside a scene.
	scene_actor_light: Option<[f32; 4]>,
	/// A V offset for the hero model's vertices `[start, end)` this frame (the Pyrocitor nozzle's texture scroll).
	pub marker_v: Option<(usize, usize, f32)>,
	/// WPN-6.3e / BRIDGE-48: a V offset for one prop's vertices this frame (prop index, offset; the Visibomb aux 179 scroll).
	pub prop_v: Option<(usize, f32)>,
	/// Retail mode: the class UV scrolls of the RAM class blocks (`retail::asset_feed`), added to every instance of the class.
	pub class_uv: HashMap<u32, [f32; 2]>,
	/// BRIDGE-28: the two wrench afterimage vertex ranges of the next `set_marker` (first, end, alpha byte), then the three
	/// whole-hero afterimage ranges (BRIDGE-40, WPN-6.6a); taken by it.
	pub marker_ghosts: [Option<(usize, usize, u8)>; 5],
	/// [PC] perf (lane R2): pose generation numbers (now, before) of the next `set_marker` (0 = unknown); taken by it. See `HeroGpu::set_poses`.
	pub hero_gens: (u64, u64),
	/// WPN-3.2: the held bomb's marker vertex range (first, end) and its pulse colour scale (0x80 = 1); taken by the next `set_marker`.
	pub marker_bomb: Option<(usize, usize, [f32; 3])>,
	/// BRIDGE-75: the hero body tilt (roll, pitch) of the next `set_marker`; taken by it (the frontend draw is untilted).
	pub marker_tilt: [f32; 2],
	/// [PC] fur: the hero is hidden (L1 look) with only a loose wrench kept (`hero_pose::collapse_outside`): its collapsed vertices sit
	/// at the feet, so the fur shells are not drawn (set every frame by the host before `set_marker`; furfx.rs `fur_wanted`).
	pub marker_fur_off: bool,
	/// BRIDGE-21c: the Clank light's vertex range in the hero model and its pulse tint (drawn with the tint override).
	pub marker_light: Option<(usize, usize, [f32; 3])>,
	/// Alpha and additive particle pipelines, the sprite texture, and this frame's vertices
	/// (quads, 4 per sprite or ribbon segment) per pipeline.
	part_pipelines: [wgpu::RenderPipeline; 3],
	/// BRIDGE-23: GS FIX-blend strips (blend constant = FIX / 128).
	strip_pipeline: wgpu::RenderPipeline,
	/// The Hoven queue-C pools (FUN_002bbb40 GS block 0x1cb6a0: TEST 0x50000 = ZTST GEQUAL, Z written): FIX blend, depth
	/// LessEqual, depth write.
	pool_pipeline: wgpu::RenderPipeline,
	pool_add_pipeline: wgpu::RenderPipeline,
	part_tex: Option<wgpu::BindGroup>,
	/// This frame's particle vertices by (run, additive, fx_tex key; u32::MAX = soft-disc fallback); run 0 = grouped,
	/// runs 1.. = retail pool draw order (gpu.rs `set_particles`).
	part_verts: BTreeMap<(u32, bool, u32), Vec<Vertex>>,
	/// KER-92: the pool line draw (mode 2): untextured Gouraud line list, alpha-over, depth-tested, no depth writes.
	line_pipeline: wgpu::RenderPipeline,
	/// This frame's line vertices, 2 per line.
	line_verts: Vec<Vertex>,
	/// Effect type -> particle texture number (level particle def table).
	pub part_defs: Vec<u8>,
	/// Blast moby meshes (classes 112 fireball, 122 shard) by class id; faces carry an fx_tex key.
	pub fx_meshes: HashMap<u32, FxMesh>,
	/// The level's fx textures by index (blob shadow 0, fire 41/42) and this frame's textured quads
	/// (shadows.rs, particles.rs fire), drawn before the sprites in order.
	pub fx_tex: HashMap<usize, wgpu::BindGroup>,
	orig_gpu: gpu::split_orig::GpuOrig, // [PC] split view with a pack: the disc's fx, particle and chrome textures (gpu/split_orig.rs)
	pub fx_quads: Vec<crate::prims::WorldQuad>,
	/// Water quads, drawn with the scene fog (fs_part_fog).
	pub water_quads: Vec<crate::prims::WorldQuad>,
	/// BRIDGE-23: this frame's FIX-blend world strips (NOV-15.1b).
	pub strip_draws: Vec<crate::prims::WorldStrip>,
	/// This frame's queue-C water pools (Hoven, `queue_q1::fill`), drawn before the shrubs.
	pub pool_draws: Vec<crate::prims::WorldStrip>,
	/// ARI-49b (BRIDGE-63): the Aridia ocean pipelines (0 = its own fog, 1 = the level fog while the Visibomb is live),
	/// its 64x64 animated texture, this frame's patch strips, the texels and game frame last uploaded. KER-30: 2, 3 =
	/// the same pair FIX-blended without Z write (Kerwan ocean, `fs_ocean_k`).
	ocean_pipeline: [wgpu::RenderPipeline; 6],
	ocean_tex: Option<(wgpu::Texture, wgpu::BindGroup)>,
	ocean_draws: Vec<crate::prims::WorldStrip>,
	ocean_rgba: Vec<[u8; 4]>,
	ocean_frame: Option<u32>,
	ocean_level_fog: bool,
	/// FOGCOL of a RAM-built ocean (`Ocean::fog`), None for the baked oceans.
	ocean_fog: Option<[u8; 3]>,
	/// Retail mode: the Rilgar, Umbris and Gaspar oceans (`fs_ocean_g`: own fog per vertex), opaque + Z or FIX-blended without Z (`ocean_g_pipeline` 0 / 1).
	ocean_g_pipeline: [wgpu::RenderPipeline; 4],
	ocean_ram: Vec<OceanBatch>,
	/// The arrival warp's fan, quad textures and screen quads (warpfx.rs, SCN-4.2).
	warp_fx: Option<warpfx::WarpFx>,
	/// FMV picture (fmv/present.rs), drawn over everything while a movie plays.
	pub movie: crate::fmv::present::Presenter,
}

pub struct Camera {
	pub pos: [f32; 3],
	pub yaw: f32,
	pub pitch: f32,
	/// Tangent of half the vertical field of view.
	pub tan_half: f32,
	/// BRIDGE-56 (decomp/notes/port_bridges.md): the view's up hint, Z except under the Visibomb remote camera's roll.
	pub up: [f32; 3],
}



#[cfg(test)]
mod shader_tests {
	#[test]
	fn moby_alpha_encodes_vertices_and_depth_split() {
		for (alpha, texel, expected, write_z) in [(64u8, 128u8, 64u8, false), (100, 128, 100, true), (127, 64, 63, false), (0, 128, 0, false)] {
			let (encoded, transparent) = super::moby_alpha_encoding(3.0, alpha);
			let decoded = 128.0 - ((encoded - (encoded + 0.5).floor()) * 512.0).round();
			assert_eq!(decoded, alpha as f32);
			let a = ((texel as f32 * decoded / 128.0).floor()) as u8;
			assert_eq!(a, expected);
			assert_eq!(a >= 96, write_z);
			assert!(transparent);
		}
		let (encoded, props_alpha) = super::moby_alpha_encoding(3.0, 64);
		let mut vertex = <super::Vertex as bytemuck::Zeroable>::zeroed();
		vertex.colour[3] = encoded;
		assert_eq!(vertex.colour[3] - 3.0, 0.125);
		assert!(props_alpha);
		assert_eq!(super::moby_alpha_encoding(3.0, 128), (3.0, false));
	}

	/// REN-5c / REN-5d arithmetic as the vertex shader writes it (f32 mirror of `vs` / `slot_blend`; the WGSL itself is
	/// checked by `shader_validates`).
	fn lit_byte(amb: f32, col: f32, q: f32, wrap: f32) -> f32 {
		(amb + (128.0 * (col * q.max(q * -wrap.abs())).max(0.0)).floor() / 128.0).min(255.0 / 128.0)
	}

	#[test]
	fn vertex_lighting_grid_and_wrap() {
		let r = lit_byte(55.0 / 128.0, 0.835, 0.5, 0.0);
		assert_eq!(r, 108.0 / 128.0);
		assert_eq!(lit_byte(0.5, 1.6, 1.0, 0.0), 255.0 / 128.0);
		assert_eq!(lit_byte(0.5, 1.0, -0.5, 0.3), 83.0 / 128.0);
		assert_eq!(lit_byte(0.5, 1.0, -0.5, -0.3), 83.0 / 128.0);
	}

	#[test]
	fn light_pair_blends_before_shading() {
		let norm = |d: [f32; 3]| {
			let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
			d.map(|x| x / l)
		};
		let (da, db) = ([-0.397f32, 0.852, -0.342], [0.884f32, 0.359, -0.3]);
		let t = 0.5f32;
		let d = norm(std::array::from_fn(|k| da[k] * (1.0 - t) + db[k] * t));
		assert!((d[1] - 0.83253264).abs() < 1e-6);
		let (ca, cb) = ([0.835f32, 0.8, 0.627], [0.627f32, 0.573, 0.49]);
		let r: [f32; 3] = std::array::from_fn(|k| (ca[k] * (1.0 - t) + cb[k] * t) * d[1]);
		for (x, e) in r.iter().zip([0.6085813, 0.5715337, 0.4649695]) {
			assert!((x - e).abs() < 1e-5, "{x} {e}");
		}
	}

	// MOBY-GS tests 1 and 2.
	#[test]
	fn moby_gs_test_register_and_texel_split() {
		let v = 0x5360bu64;
		assert_eq!((v & 1, (v >> 1) & 7, (v >> 4) & 0xff, (v >> 12) & 3, (v >> 16) & 1, (v >> 17) & 3), (1, 5, 0x60, 3, 1, 2));
		assert_eq!((0x103e4000u32 >> 15) & 0x7ff, 0x7c);
		// (opaque pass keeps it, blended pass draws it with Z write, blended pass without Z)
		let classify = |texel: f32, alpha: f32| {
			let a = (texel * alpha / 128.0).floor();
			(texel >= 128.0 && alpha == 128.0, a >= 96.0, a < 96.0 && !(texel >= 128.0 && alpha == 128.0))
		};
		assert_eq!(classify(64.0, 128.0), (false, false, true));
		assert_eq!(classify(112.0, 128.0), (false, true, false));
		assert_eq!(classify(128.0, 128.0), (true, true, false));
		assert_eq!(classify(128.0, 64.0), (false, false, true));
	}

	/// The main WGSL shader parses and validates (pipeline creation would fail at startup otherwise).
	#[test]
	fn shader_validates() {
		let m = naga::front::wgsl::parse_str(&super::volfog::soft::level_source(super::SHADER, false)).expect("wgsl parse");
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	}
}

