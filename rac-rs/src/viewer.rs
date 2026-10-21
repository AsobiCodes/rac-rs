//! Real-time level viewer: terrain with textures on the GPU, free-fly camera.
//! Controls: hold right mouse + move to look, WASD to move, Q/E down/up, Shift faster,
//! PageUp/PageDown or 1-9/0 to switch level, Esc to quit.
//! P toggles play mode at the level's start (Shift+P: on the ground under the camera); WASD runs
//! relative to the camera, Space jumps (Ctrl+Space high jump), F or left mouse swings the wrench,
//! right mouse orbits. Bolts and health show in the window title.

use crate::level::{Level, Texture};
use crate::mesh::{Lit, LitKind, Mesh};
use crate::physics::World;
use crate::flow::Flow;
use crate::game::{self, Game, Prop};
use crate::overlay::{self, Quad};
use crate::player::{self, Input, Orbit};
use crate::hud as art;
use crate::help::{self, HelpLog};
use crate::settings::Settings;
use crate::{ambience, anim, audio, camera, dynlight, fmv, frontend, map, music, collision, gameplay, iso, mobies, particles, save, scene, shrubs, sky, sound, terrain, ties, toc};
use bytemuck::{Pod, Zeroable};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

mod travel;
use hud::*;
use hudart::*;
use popups::*;
mod hud;
mod hudart;
mod popups;
mod gpu;
mod gpuskin;
mod gpuhero;
mod bindless;
pub(crate) mod gs_state;
mod warpfx;
mod clip;
mod shadowvol;
mod post;
mod sunshadow;
mod waterfx;
mod underwater;
mod icefx;
mod mirror;
mod partfx;
mod vegfx;
mod app;
mod handler;
mod front_mirror;
mod hudsharp;
mod heroanim;
mod smooth;
mod herocache;
mod gpucache;
mod tiegpu;
mod interp;
mod profile;
mod pace;
mod presenter;
mod fardraw;

/// Particle texture number n is uploaded into `fx_tex` under key PART_TEX + n; effect types reach it
/// through `Gpu::part_defs` (level particle def table, decomp/notes/particles.md).
const PART_TEX: u32 = 0x2000;
/// fx_tex keys for the blast mobys (decomp/notes/particles.md): a white texel for the untextured shard faces, and
/// FX_MOBY_TEX + n for moby texture n.
const FX_WHITE: usize = 0x2fff;
const FX_MOBY_TEX: usize = 0x3000;

/// A blast moby's triangles: (fx_tex key, vertex indices) over `pos` / `uv`.
struct FxMesh {
	pos: Vec<[f32; 3]>,
	uv: Vec<[f32; 2]>,
	tris: Vec<(usize, [u32; 3])>,
}
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
	pos: [f32; 3],
	uv: [f32; 2],
	normal: [f32; 3],
	colour: [f32; 4],
	/// Mobies (colour alpha >= 0): ambient rgb (GS scale, 0x80 = 1.0) and the light pair packed as
	/// A * 16 + B + blend (see `moby_light`).
	light: [f32; 4],
}

/// One retail-mode ocean batch (`Gpu::ocean_ram`): its 64x64 texture (rewritten every field), the strips as triangles, the GS ALPHA FIX byte.
struct OceanBatch {
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
struct Uniforms {
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
	sun_q: [f32; 4],
}

/// Mobies lit by a dynamic light in one frame (the rest take none).
const MAX_DYN: usize = 128;

/// Registers the third light a moby at `origin` takes (MobyProc, `dynlight::moby_light`) and returns its 1-based id
/// for the vertex colour alpha, 0 when no light reaches it.
fn dyn_id(table: &[Option<dynlight::SlotLight>; 8], list: &mut Vec<([f32; 3], [f32; 3])>, origin: [f32; 3]) -> f32 {
	if list.len() >= MAX_DYN {
		return 0.0;
	}
	match dynlight::moby_light(origin, table) {
		Some(l) => {
			list.push(l);
			list.len() as f32
		}
		None => 0.0,
	}
}


fn moby_alpha_encoding(id: f32, alpha: u8) -> (f32, bool) {
	(id + (128.0 - alpha as f32) / 512.0, alpha < 0x80)
}

/// The tinted tail of a faded moby: baked -2 colour code with the draw alpha as a fraction below it (0x80 = exactly -2.0).
fn moby_baked_encoding(alpha: u8) -> f32 {
	-2.0 - (128.0 - alpha as f32) / 512.0
}

/// Moby draw-distance fade (FUN_0026a338): `depth1024` is the camera forward distance of the moby's sphere centre in 1/1024 m,
/// the limit `draw` is clamped to 500 m. None = beyond the limit (not drawn), else the fade byte 0..=0x80.
/// Entries of the moby draw's deferred list per frame.
const DEFERRED_CAP: usize = 48;

fn moby_fade(draw: i16, depth1024: f32) -> Option<u8> {
	let y = ((draw as i32).min(500) << 10) as f32 - depth1024;
	if y < 0.0 {
		return None;
	}
	Some(((y as i32 as u32) >> 7).min(0x80) as u8)
}

/// Fade byte of a moby on the deferred-list path (flags & 0xc00, in-game frame): `(L << 10) - trunc(z - w)` with L the unclamped
/// draw distance and `w` the sphere radius, >> 7 and capped at 0x80 (SCH-6b; the cull itself already used `moby_fade`).
fn moby_fade_alt(draw: i16, z: f32, w: f32) -> u8 {
	let y = ((draw as i32) << 10).wrapping_sub((z - w) as i32);
	(y >> 7).clamp(0, 0x80) as u8
}

/// SCH-6b: the draw-distance verdict of an effect moby or debris piece centred at `pos` (metres): None = culled,
/// else the fade byte (0x80 without a draw distance or a view). The centre is `pos * 1024` [I].
fn effect_fade(view: Option<([f32; 3], [f32; 3])>, draw: Option<i16>, pos: [f32; 3]) -> Option<u8> {
	let (Some((e, f)), Some(d)) = (view, draw) else { return Some(0x80) };
	let z: f32 = (0..3).map(|k| f[k] * (pos[k] * 1024.0 - e[k] * 1024.0)).sum();
	moby_fade(d, z)
}

/// The moby header alpha: fade times the moby's +0x23 byte, >> 7.
fn moby_draw_alpha(fade: u8, alpha: u8) -> u8 {
	((fade as u32 * alpha as u32) >> 7) as u8
}

/// NOV-35b metal alpha (FUN_0026a338 raw 0x26b498-0x26b4c4, moby_format.md 4.1): with t0 = trunc(z - w) (the sphere's view depth
/// and radius, 1/1024 m), d = (metal << 10) - t0; d <= 0 -> None (no metal part this frame), else min(d >> 7, 0x80). Not scaled by
/// the draw-distance fade or byte +0x23.
fn metal_alpha(metal: u8, z: f32, w: f32) -> Option<u8> {
	let d = ((metal as i32) << 10).wrapping_sub((z - w) as i32);
	(d > 0).then(|| ((d as u32) >> 7).min(0x80) as u8)
}

/// NOV-35b: moby +0x73 of a class with metal packets: 0x18 from the spawn init (FUN_00263020), 0x14 for the bolt pickups 13-16
/// (bolt_update FUN_002bb088, reward_bolt_create FUN_002bc6e8), 0x20 for the vendor orb 1143 (FUN_003092c0).
fn metal_byte(class_id: u32) -> u8 {
	match class_id {
		13..=16 => 0x14,
		1143 => 0x20,
		_ => 0x18,
	}
}

/// NOV-35b: `Vertex::light.w` of a metal vertex: its colour is built on the CPU (`light.rgb`), the shader adds no light.
const METAL_LIGHT: f32 = -4.0;

const SHADER: &str = r#"
struct U { view_proj: mat4x4<f32>, sky_view_proj: mat4x4<f32>, lights: array<array<vec4<f32>, 4>, 16>, ambient: array<vec4<f32>, 4>, fog_colour: vec4<f32>, fog: vec4<f32>, eye: vec4<f32>, points: array<vec4<f32>, 16>, dynl: array<vec4<f32>, 256>, sun_vp: array<mat4x4<f32>, 3>, sun: vec4<f32>, sun_bias: vec4<f32>, sun_texel: vec4<f32>, sun_q: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var sun_map: texture_depth_2d_array;
@group(0) @binding(2) var sun_cmp: sampler_comparison;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) n: vec3<f32>, @location(2) c: vec4<f32>, @location(3) w: vec3<f32>, @location(4) @interpolate(flat) l: vec4<f32>, @location(5) @interpolate(linear) fogf: f32, @location(6) lit: vec3<f32>, @location(9) amb: vec3<f32>, @location(7) @interpolate(flat) kmx: vec2<f32> };

// GS fog coefficient per world class (gameplay::fog_coefficient; lighting_fog.md "Fog per class"). The vertex light
// w tags the class: -1 tfrag (per vertex, view depth), -2 shrub (per instance, l.xyz = instance origin) / -3 tie
// (per instance, l.xyz = sphere centre [P]); all three use the same slope/intercept (the d058..d06c line).
// Anything else returns -1: the fragment falls back to the euclidean-distance fog.
fn fog_class(l: vec4<f32>, w: f32) -> f32 {
	// Effects layer (retail/efx_draw.rs `fog_light`): l.w = -5 carries the GS fog byte F in l.x, interpolated linearly in screen space as the GS does.
	if (l.w < -4.5 && l.w > -5.5) {
		return l.x;
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
	var k = i32(inst & 0xfffu);
	if (k >= 2048) { k = k - 4096; }
	o.kmx = vec2<f32>(f32(k) / 16.0, f32((inst >> 12u) & 3u));
	return o;
}

// Shrub mesh draws: the first instance's material word (`shrub_inst`; bindless keeps the texture slot below it) carries the
// CLAMP wrap (bits 0..2), the TEX1 K (bits 2..14, raw signed 12 bits, 1/16 mip) and the texture's top level (bits 14..16).
@vertex fn vs_shrub(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>, @location(4) l: vec4<f32>, @builtin(instance_index) inst: u32) -> VOut {
	var o = vs_main(p, uv, n, c, l);
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

// [PC] Dynamic shadows (sunshadow.rs; u.sun.w = 0 when off, and nothing below runs).

// One map layer at map point uv, receiver depth z: the blocked fraction (High: 4x4 hardware PCF taps half a texel off the grid: a
// smooth 5-texel edge instead of 3x3's stair steps; Medium 3x3, Low 2x2: u.sun_q.x is the kernel side).
fn sun_pcf(uv: vec2<f32>, layer: i32, z: f32) -> f32 {
	let ts = 1.0 / u.sun_bias.w;
	let n = i32(u.sun_q.x);
	let h = (u.sun_q.x - 1.0) * 0.5;
	var lit = 0.0;
	for (var y = 0; y < n; y++) {
		for (var x = 0; x < n; x++) {
			lit += textureSampleCompareLevel(sun_map, sun_cmp, uv + (vec2<f32>(f32(x), f32(y)) - h) * ts, layer, z);
		}
	}
	return 1.0 - lit / (u.sun_q.x * u.sun_q.x);
}

// The first cascade whose map holds p answers, every caster at full strength however far it stands (no fade by blocker
// distance: a fade cut shadows off halfway, stacked half shadows under whole ones and lit odd patches of rooms; a roof darkens
// what it covers, indoors are dark). p moves 2 texels towards the light first (no acne under the 4x4 kernel); the last
// cascade fades out at its rim.
fn sun_occ(p: vec3<f32>) -> f32 {
	let nc = i32(u.sun_q.z);
	for (var k = 0; k < nc; k++) {
		let q = u.sun_vp[k] * vec4<f32>(p - u.sun.xyz * (u.sun_texel[k] * 2.0), 1.0);
		let e = max(abs(q.x), abs(q.y));
		if (e < 0.98 && q.z > 0.0 && q.z < 1.0) {
			let uv = vec2<f32>(q.x * 0.5 + 0.5, 0.5 - q.y * 0.5) * u.sun_q.y;
			let z = q.z - u.sun_bias[k];
			var occ = sun_pcf(uv, k, z);
			if (k == nc - 1) { occ *= 1.0 - smoothstep(0.8, 0.98, e); }
			return occ;
		}
	}
	return 0.0;
}

// Baked world colour under a dynamic shadow: 2002's vertex colours already hold the painted shadows, so the shadow takes
// STRENGTH x how lit the colour is (a painted shadow takes none).
fn sun_baked(shade: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
	if (u.sun.w <= 0.0) { return shade; }
	let lum = dot(shade, vec3<f32>(0.2126, 0.7152, 0.0722));
	return shade * (1.0 - sun_occ(p) * u.sun.w * smoothstep(0.3, 0.8, lum));
}

// Lit moby under a dynamic shadow: it keeps its ambient and loses its directional light.
fn sun_lit(i: VOut) -> vec3<f32> {
	if (u.sun.w <= 0.0) { return i.lit; }
	return i.amb + (i.lit - i.amb) * (1.0 - sun_occ(i.w) * min(u.sun.w * 1.4, 1.0));
}

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
	// Lit moby texels below alpha 0x80 are blended in the moby pass (fs_moby_a / fs_moby_b), not drawn opaque here.
	if (i.c.a >= 0.0 && floor(c.a * 128.0 + 0.5) < 128.0) { discard; }
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
		return vec4<f32>(mix(u.fog_colour.rgb, ice_look(i, c.rgb * shade), f), 1.0);
	}
	let rgb = c.rgb * i.c.rgb * sun_lit(i);
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
fn shrub_rgba(i: VOut, c: vec4<f32>) -> vec4<f32> {
	let d = distance(i.w, u.eye.xyz);
	var f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	if (i.fogf >= 0.0) { f = i.fogf / 255.0; }
	let shade = sun_baked(min(i.c.rgb, vec3<f32>(255.0 / 128.0)), i.w);
	let a = round((-2.0 - i.c.a) * 256.0);
	let ta = floor(c.a * 128.0 + 0.5);
	return vec4<f32>(mix(u.fog_colour.rgb, ice_look(i, c.rgb * shade), f), floor(ta * a / 128.0) / 128.0);
}

// Billboard texel: the GS picks the mip from LOD = log2(32 vz) + K (Q = 1 / (32 vz)); K and the top level ride in the vertex normal.
fn shrub_bb_texel(i: VOut) -> vec4<f32> {
	return textureSampleLevel(tex, samp, i.uv, gs_level(i));
}

// GS TEX1 LCM 0, L 0, MMIN LINEAR_MIPMAP_NEAREST: level 0 when LOD <= 0, else the nearest level capped at MXL (i.kmx = K, MXL).
fn gs_level(i: VOut) -> f32 {
	let lod = log2(32.0 / i.pos.w) + i.kmx.x;
	var level = 0.0;
	if (lod > 0.0) { level = min(floor(lod + 0.5), i.kmx.y); }
	return level;
}

// Terrain fragment: the baked world colour of `fs` with the level-chosen texel (GS LOD = log2(32 vz) + K; REN-TEX-RESIDENCY).
// GS ALPHA 0x44 (Cs - Cd) * As + Cd with As = the texel alpha (vertex alpha 128) and TEST 0x5360b (ATE GEQUAL AREF 0x60, AFAIL
// RGB_ONLY): texels below alpha 0x60 still blend (colour only, no z), the rest blend and write z (fx_census_late.md, run 19).
fn tfrag_rgba(i: VOut) -> vec4<f32> {
	let c = shrub_bb_texel(i);
	let d = distance(i.w, u.eye.xyz);
	var f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	if (i.fogf >= 0.0) { f = i.fogf / 255.0; }
	let shade = sun_baked(min(i.c.rgb, vec3<f32>(255.0 / 128.0)), i.w);
	return vec4<f32>(mix(u.fog_colour.rgb, c.rgb * shade, f), min(floor(c.a * 128.0 + 0.5), 128.0) / 128.0);
}

@fragment fn fs_tfrag(i: VOut) -> @location(0) vec4<f32> {
	if (i.c.a < -2.0005) { discard; }
	let r = tfrag_rgba(i);
	if (r.a * 128.0 < 96.0) { discard; }
	return r;
}

@fragment fn fs_tfrag_lt96(i: VOut) -> @location(0) vec4<f32> {
	if (i.c.a < -2.0005) { discard; }
	let r = tfrag_rgba(i);
	if (r.a * 128.0 >= 96.0 || r.a <= 0.0) { discard; }
	return r;
}

// Tie fragment: the baked world colour of `fs` with the tie texel (`world_texel`: TEX1 level, CLAMP wrap) and its alpha. TieProc's
// draw packet (Pokitaru FUN_002b87c8, data 0x1c33b0) sets TEST_1 0x5340b (ATE GEQUAL AREF 0x40, AFAIL RGB_ONLY, ZTST GEQUAL) under
// ALPHA_1 0x44 (Cs - Cd) * As + Cd, As = texel alpha (vertex alpha 0x80): every texel blends, from alpha 0x40 it also writes Z.
// Drawn opaque, the fading feet of the Pokitaru shoal mounds were hard flat silhouettes (decomp/notes/fx_census_late.md).
fn tie_rgba(i: VOut) -> vec4<f32> {
	let c = world_texel(i);
	let d = distance(i.w, u.eye.xyz);
	var f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	if (i.fogf >= 0.0) { f = i.fogf / 255.0; }
	// [PC] Lighting Enhanced: the sun shadow / AO term of the other baked paths (`sun_baked`; identity in Classic).
	let shade = sun_baked(min(i.c.rgb, vec3<f32>(255.0 / 128.0)), i.w);
	return vec4<f32>(mix(u.fog_colour.rgb, ice_look(i, c.rgb * shade), f), min(floor(c.a * 128.0 + 0.5), 128.0) / 128.0);
}

@fragment fn fs_tie(i: VOut) -> @location(0) vec4<f32> {
	if (i.c.a < -2.0005) { discard; }
	let r = tie_rgba(i);
	if (r.a * 128.0 < 64.0) { discard; }
	return r;
}

@fragment fn fs_tie_lt64(i: VOut) -> @location(0) vec4<f32> {
	if (i.c.a < -2.0005) { discard; }
	let r = tie_rgba(i);
	if (r.a * 128.0 >= 64.0 || r.a <= 0.0) { discard; }
	return r;
}

// Shrub mesh texel: the draw's bind group samples with its material's GS CLAMP wrap (bindless.rs rewrites this one), at the
// level its TEX1 picks (`vs_shrub`; the slot holds the authored mip chain).
fn shrub_mesh_texel(i: VOut) -> vec4<f32> {
	return textureSampleLevel(tex, samp, i.uv, gs_level(i));
}

// [PC] Enhanced (u.sun_texel.w = 1; applied by the callers: bindless.rs rewrites `shrub_mesh_texel` by its text): a magnified leaf texture's alpha ramp is cut at half alpha over one screen pixel, so a leaf
// close to the camera keeps a sharp solid outline instead of a wide see-through blur; far away (minified) the ramp spans less
// than a pixel and the texel alpha is kept. Classic leaves the texel as the GS samples it.
fn crisp_leaf(c: vec4<f32>) -> vec4<f32> {
	if (u.sun_texel.w <= 0.0) { return c; }
	let w = fwidth(c.a);
	if (w >= 0.5) { return c; }
	return vec4<f32>(c.rgb, clamp((c.a - 0.5) / max(w, 1e-4) + 0.5, 0.0, 1.0));
}

@fragment fn fs_shrub_ge12(i: VOut) -> @location(0) vec4<f32> {
	let r = shrub_rgba(i, crisp_leaf(shrub_mesh_texel(i)));
	if (r.a * 128.0 < 12.0) { discard; }
	return r;
}

@fragment fn fs_shrub_lt12(i: VOut) -> @location(0) vec4<f32> {
	let r = shrub_rgba(i, crisp_leaf(shrub_mesh_texel(i)));
	if (r.a * 128.0 >= 12.0) { discard; }
	return r;
}

@fragment fn fs_shrub_ge96(i: VOut) -> @location(0) vec4<f32> {
	let r = shrub_rgba(i, shrub_bb_texel(i));
	if (r.a * 128.0 < 96.0) { discard; }
	return r;
}

@fragment fn fs_shrub_lt96(i: VOut) -> @location(0) vec4<f32> {
	let r = shrub_rgba(i, shrub_bb_texel(i));
	if (r.a * 128.0 >= 96.0) { discard; }
	return r;
}

@fragment fn fs_shrub_all(i: VOut) -> @location(0) vec4<f32> {
	return shrub_rgba(i, shrub_bb_texel(i));
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

fn moby_rgba(i: VOut) -> vec4<f32> {
	let c = moby_texel(i);
	var rgb = c.rgb * i.c.rgb * sun_lit(i);
	if (i.c.a < -1.5) {
		// Tinted tail packet: constant unlit colour, same alpha as the lit packets.
		rgb = c.rgb * min(i.c.rgb, vec3<f32>(255.0 / 128.0));
	}
	let d = distance(i.w, u.eye.xyz);
	let f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	let a = floor(floor(c.a * 128.0 + 0.5) * moby_alpha(i) / 128.0) / 128.0;
	return vec4<f32>(mix(u.fog_colour.rgb, rgb, f), a);
}

@fragment fn fs_moby_a(i: VOut) -> @location(0) vec4<f32> {
	let ta = floor(moby_texel(i).a * 128.0 + 0.5);
	if (moby_alpha(i) == 128.0 && (ta >= 128.0 || i.c.a < 0.0)) { discard; }
	let r = moby_rgba(i);
	if (r.a * 128.0 < 96.0) { discard; }
	return r;
}

@fragment fn fs_moby_b(i: VOut) -> @location(0) vec4<f32> {
	let ta = floor(moby_texel(i).a * 128.0 + 0.5);
	if (moby_alpha(i) == 128.0 && (ta >= 128.0 || i.c.a < 0.0)) { discard; }
	let r = moby_rgba(i);
	if (r.a * 128.0 >= 96.0) { discard; }
	return r;
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
"#;

#[derive(Clone, Copy)]
struct Draw {
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
struct ShrubQuad {
	pos: [[f32; 3]; 4],
	colour: [f32; 4],
	class_id: u32,
	lod_k: f32,
	mxl: f32,
	pass1: bool,
}

/// Vertex and index buffers with one draw per run of faces sharing a texture.
/// The sky batch's CPU side for the turning shells (`sky::shell_angle`).
struct SkyShells {
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
struct SkySpriteDraw {
	vbuf: wgpu::Buffer,
	/// Fixed: quad k is vertices 4k..4k+3, triangles (0, 1, 2), (1, 2, 3).
	ibuf: wgpu::Buffer,
	/// Bind groups of sky textures `4 << 16 | 2` and `4 << 16 | 3`.
	bind: [usize; 2],
	/// This frame's quads per texture (texture 2's first in the buffer).
	counts: [u32; 2],
}

struct Batch {
	vbuf: wgpu::Buffer,
	ibuf: wgpu::Buffer,
	draws: Vec<Draw>,
}

/// BRIDGE-45 (NOV-35b, moby_format.md 4.1): the metal packets of every class that has them, the chrome and glass textures (clamp
/// sampler; `Draw::texture` 0 = chrome, 1 = glass) and this frame's world batch of the drawn props' metal faces (rebuilt by
/// `Gpu::set_props`, capacity in vertices and indices). The shop icon / popup bolts build theirs at the panel draw.
#[derive(Default)]
struct MetalDraw {
	classes: HashMap<u32, Vec<mobies::MetalPacket>>,
	tex: Option<[wgpu::BindGroup; 2]>,
	props: Option<Batch>,
	cap: (usize, usize),
	/// [PC] perf round 3: the props' metal vertices of the last frame (allocation reused).
	scratch: Vec<Vertex>,
}

struct Scene {
	world: Batch,
	/// The world vertices as uploaded, and the dynamic-light assignment over them (`Gpu::relight`).
	world_verts: Vec<Vertex>,
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
	occ: crate::occlusion::Occlusion,
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
	ice_level: usize,
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
	tfrag_set: Vec<terrain::TfragLods>,
	tfrag_draws: Vec<Draw>,
	tfrag_ranges: Vec<[std::ops::Range<u32>; 3]>,
	tfrag_vranges: Vec<[std::ops::Range<usize>; 3]>,
	tfrag_verts: Vec<Vertex>,
	tfrag_cols: Vec<Vec<[f32; 4]>>,
	tfrag_b: f32,
	/// [PC] Dynamic shadows (sunshadow.rs): the LOD `Gpu::cull_tfrags` drew each fragment with this frame (u8::MAX = not drawn):
	/// a fragment casts with the mesh it receives on (a coarser LOD above the drawn one shadowed the ground under it).
	tfrag_lod: Vec<u8>,
	sky: Option<Batch>,
	/// NOV-31: the vertices of `sky` with each shell's range and unturned positions (`Gpu::set_sky_spin`).
	sky_shells: Option<SkyShells>,
	/// ARI-4: the sky sprite batch (skies with textures 2 and 3).
	sky_sprites: Option<SkySpriteDraw>,
	/// The player's model (Ratchet, or a box if he failed to decode): `marker_model` holds its
	/// model-space vertices, re-posed into `marker.vbuf` every frame; drawn when `marker_shown`.
	marker: Batch,
	marker_model: Vec<Vertex>,
	/// Ratchet's class tail colour (CLASS-tint): (rgb, first vertex) over the first `marker_hero_len` marker vertices; the appended
	/// weapon meshes are other mobies and stay lit.
	/// PC perf: the posed marker vertices of the last `set_marker` (buffer reused every frame).
	marker_verts: Vec<Vertex>,
	marker_tint: Option<([f32; 3], usize)>,
	marker_hero_len: usize,
	marker_shown: bool,
	/// BRIDGE-28: the marker carries translucent wrench afterimages this frame (set by `Gpu::set_marker`).
	marker_alpha: bool,
	/// Crates, bolts and enemies: one model copy per `game::Prop` slot (`props_ranges`), re-posed
	/// into `props.vbuf` every frame; hidden slots collapse to a point.
	props: Batch,
	/// [PC] perf (lane A7): the index buffer a pool growth last moved the props indices out of; the next growth copies them
	/// back into it (`Gpu::append_props`). None until a pool grows.
	props_ibuf_spare: Option<wgpu::Buffer>,
	props_model: Vec<Vertex>,
	props_ranges: Vec<std::ops::Range<usize>>,
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
	props_late: Vec<usize>,
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
	warp_shown: bool,
	/// BRIDGE-36 (c1): the purchase prompt's one-prop batch and its slot in `props` (UI-5d).
	prompt: Option<Batch>,
	prompt_slot: Option<usize>,
	/// BRIDGE-37 (b): the shop portrait (class 12) one-prop batch and its mesh-order base vertices.
	portrait: Option<Batch>,
	portrait_model: Vec<Vertex>,
	/// Pause-menu item previews by held model class: batch, centre, radius (decomp/notes/ingame_pause_menu.md).
	previews: HashMap<u32, (Batch, [f32; 3], f32)>,
	/// LIT-7a: the same preview models with the gold light word (slots 12 / 12) in their vertices.
	previews_gold: HashMap<u32, Batch>,
	/// Unposed vertices of the hero previews (keys >= HERO_PREVIEW), re-posed every frame by `set_preview_pose`.
	preview_models: HashMap<u32, Vec<Vertex>>,
	bind_groups: Vec<wgpu::BindGroup>,
	/// Texture key -> `bind_groups` index of every texture the scene uploaded: `Gpu::regrow_props` rebuilds the prop slots
	/// with it (retail pool growth) without a new upload.
	tex_slots: HashMap<u32, usize>,
	/// [PC] Dynamic shadows: per texture slot, whether most of its texels blend (PS2 alpha below 0x60, `fs_moby_b`): see-through
	/// effect shells (blast domes, glows) cast no sun shadow (sunshadow.rs).
	clear_tex: Vec<bool>,
	/// `bind_groups` index -> the first-instance word of a moby draw with that slot (TEX1 K and chain top level; 0 = plain sample).
	slot_words: Vec<u32>,
	/// [PC] perf round 3: `bind_groups` as one texture array (bindless.rs; None = per-draw bind groups).
	tex_array: Option<wgpu::BindGroup>,
	clear: wgpu::Color,
	centre: [f32; 3],
	radius: f32,
}

thread_local! {
	/// Scene-actor moby tints (+0x90, FUN_00206f78) by actor slot for the next `set_props`; decomp/notes/veldin_ambient.md section 5.
	/// (RGB with 0x80 = 1.0, first overridden vertex = `MobyClass::colour_start`): FUN_00255480 replaces the lit
	/// colour of packets [class byte 0x0a, LOD end) with +0x90, so only that subset changes and it is drawn unlit.
	/// Placed-prop moby tints by `game.props` slot (VEND-tint: the vendor's +0x90 grey), same meaning as `ACTOR_TINTS`.
	static PROP_TINTS: std::cell::RefCell<Vec<Option<([f32; 3], usize)>>> = const { std::cell::RefCell::new(Vec::new()) };
	static ACTOR_TINTS: std::cell::RefCell<Vec<Option<([f32; 3], usize)>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Checkpoint snapshot of the non-position environment (FUN_0029a540 -> FUN_0029a6f8; savegame.md SAV-1c).
#[derive(Clone, Copy)]
struct CkptEnv {
	hero_env: (usize, usize, f32, [f32; 3]),
	reverb: ambience::Reverb,
	track: i32,
}

struct Gpu {
	window: Arc<Window>,
	surface: wgpu::Surface<'static>,
	device: wgpu::Device,
	/// [PC] Gated on the deferred submit of the previous frame (presenter.rs `GatedQueue`): every use waits for it.
	queue: presenter::GatedQueue,
	/// [PC] Presents on a worker thread (presenter.rs); flush() before the surface is reconfigured.
	presenter: presenter::Presenter,
	config: wgpu::SurfaceConfiguration,
	pipeline: wgpu::RenderPipeline,
	pipeline_cull: [wgpu::RenderPipeline; 2],
	moby_pipelines: [wgpu::RenderPipeline; 2],
	/// `moby_pipelines` with the packet's VU1 cull mode (index 2 * (cull - 1) + pass; mobies::CULL_BACK / CULL_FRONT).
	moby_cull: [wgpu::RenderPipeline; 4],
	/// REN-CULL-SHRUB: faded mesh with / without Z write (texel alpha x A >= 12), billboard pass 1 with / without Z (>= 96), pass 2.
	shrub_pipelines: [wgpu::RenderPipeline; 5],
	/// Billboard textures clamp at the edges (GS CLAMP_1 = 5).
	clamp_sampler: wgpu::Sampler,
	/// Shrub mesh CLAMP wraps 1..=3 (u, v, both clamped); see `shrubs::WRAP_SHIFT`.
	wrap_samplers: [wgpu::Sampler; 3],
	tfrag_pipeline: wgpu::RenderPipeline,
	tfrag_pipeline_lt: wgpu::RenderPipeline,
	/// Tie draws (`fs_tie`, `fs_tie_lt64`): [pass][cull: none, back, front]; pass 0 texel alpha >= 0x40 (Z written), pass 1 below.
	tie_pipelines: [[wgpu::RenderPipeline; 3]; 2],
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
	preview: Option<(u32, [f32; 4], f32, Option<u8>)>,
	/// LIT-7a: the panel-3 item in `preview` is a gold weapon (drawn from `Scene.previews_gold` with the slot-12 light); set by the handler.
	preview_gold: bool,
	/// The category-3 pair moby (class 601) drawn with the Gadgets panel model, same placement.
	preview_pair: Option<(u32, [f32; 4], f32, Option<u8>)>,
	/// The worn batch over the hero panel (same camera, no depth reset).
	preview_worn: Option<(u32, [f32; 4], f32, Option<u8>)>,
	/// Second preview slot: the hero in the big panel (class key >= HERO_PREVIEW, fixed front camera).
	preview2: Option<(u32, [f32; 4], f32)>,
	/// NOV-13.3: the vendor shop's item models as world mobies (preview class, model matrix), uniform slots 2 and 3.
	shop_models: Vec<(u32, [f32; 16])>,
	/// REN-9b: the shop's panel-1 item (class, model matrix), drawn into `item_panel`.
	shop_panel_models: Vec<(u32, [f32; 16])>,
	/// REN-9b: the 512x128 off-screen item target blitted into shop panel 1.
	item_panel: ItemPanel,
	/// BRIDGE-36 (b): the purchase popup's 512x128 target (UI-5d), sampled by HUD quads with `POPUP_PANEL_TEX`, and the
	/// class-13 bolt drawn into it (model matrix, uniform slot 5); set by the handler.
	popup_panel: ItemPanel,
	popup_bolt: Option<[f32; 16]>,
	/// NOV-30d: the slot-0 class-13 bolt icon drawn into the item target after the item (model matrix, uniform slot 7).
	icon_bolt: Option<[f32; 16]>,
	/// NOV-35b: the draw spheres (centre and radius, 1/1024 m) of the icon and popup bolts (`Game::shop_bolt_spheres`).
	icon_bolt_sphere: Option<([f32; 3], f32)>,
	popup_bolt_sphere: Option<([f32; 3], f32)>,
	/// The preview class `pose_panel_item` last skinned (its bind vertices go back when the panel item changes).
	panel_posed: Option<u32>,
	/// BRIDGE-45: the metal packets, textures and this frame's metal batch.
	metal: MetalDraw,
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
	/// Overlay textures (slot 0 the built-in font), sampled nearest for crisp pixels.
	hud_textures: Vec<wgpu::BindGroup>,
	nearest: wgpu::Sampler,
	/// This frame's overlay, in window pixels.
	pub hud: Vec<Quad>,
	/// Retail mode: the GS overlay texture (width, rows) and its bind group (`RETAIL_OV_TEX`).
	retail_ov: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
	ubuf: wgpu::Buffer,
	ubind: wgpu::BindGroup,
	tex_layout: wgpu::BindGroupLayout,
	sampler: wgpu::Sampler,
	depth: wgpu::TextureView,
	msaa: wgpu::TextureView,
	scene: Option<Scene>,
	/// The level's lights, zones and fog, read after `load_scene`.
	lighting: Option<gameplay::Lighting>,
	/// Retail mode: level-light slots 12..15 from retail RAM (retail.rs `reserved_lights`), over the level's rows.
	reserved_lights: Option<[[[f32; 4]; 4]; 4]>,
	/// NOV-28: the frame's fog colour and [near, far, near int, far int] (camera zone or underwater, set by the
	/// handler one frame late); None = the level fog.
	fog_live: Option<([u8; 3], [f32; 4])>,
	/// The shrub draw-distance cap (m): `shrubs::SHRUB_CAP`, or retail's live word in retail mode (retail.rs `RetailFx::shrub_cap`).
	shrub_cap: f32,
	/// NOV-28.2: live pool-particle far cap (water.rs `PART_FAR` / `PART_FAR_UNDER`).
	part_far: f32,
	/// [PC] The camera water flag of this field (retail.rs `SwapWords::under`) and the underwater look (underwater.rs).
	under_flag: bool,
	under_surf: Option<f32>,
	under: underwater::Under,
	/// Dynamic point-light slots for the next frame's uniforms (`Uniforms::points`, scenery mobies).
	points: [[f32; 4]; 16],
	/// This frame's light table with allocation serials, and the posed mobies' third lights (`dyn_id`).
	table: [Option<dynlight::SlotLight>; 8],
	moby_dyn: Vec<([f32; 3], [f32; 3])>,
	/// The hero instance's (class 0) placed colour and light word (`hero_light`).
	hero_placed: ([u8; 3], u32),
	/// The hero's sticky zone environment (hero +0x38/+0x80), reset to the placed values on load.
	hero_env: (usize, usize, f32, [f32; 3]),
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
	post: post::Post,
	/// [PC] Water Enhanced (waterfx.rs).
	waterfx: waterfx::WaterFx,
	/// [PC] Water Enhanced: the planar mirrors' uniforms (mirror.rs), one per mirror pass of a frame (`mirror::MAX_MIRRORS`).
	mirror: Vec<mirror::Mirror>,
	/// [PC] Water Enhanced: the water bed's uniforms (mirror.rs `Gpu::draw_bed`).
	bed: mirror::Mirror,
	/// [PC] Particles Enhanced (partfx.rs).
	partfx: partfx::PartFx,
	/// [PC] PREVIEW: more vegetation (vegfx.rs, RAC_VEG).
	veg: vegfx::VegFx,
	modern_ok: bool,
	/// [PC] Dynamic shadows (sunshadow.rs): the cascade maps, bound in the level's group 0 with their compare sampler.
	sun: sunshadow::SunShadow,
	/// Retail mode: a ship fight is on (`shipmouse::in_ship_fight`): space, no sun to cast from (handler.rs sets it).
	ship_fight: bool,
	/// The loaded level (handler.rs; [PC] Water Enhanced gives Gaspar's lava its own look, waterfx.rs `LAVA_SLOT`).
	level: usize,
	/// SCN-3: the lighting word the scene actors copied from the hero when the scene started (FUN_00244e58); None outside a scene.
	scene_actor_light: Option<[f32; 4]>,
	/// A V offset for the hero model's vertices `[start, end)` this frame (the Pyrocitor nozzle's texture scroll).
	marker_v: Option<(usize, usize, f32)>,
	/// WPN-6.3e / BRIDGE-48: a V offset for one prop's vertices this frame (prop index, offset; the Visibomb aux 179 scroll).
	prop_v: Option<(usize, f32)>,
	/// Retail mode: the class UV scrolls of the RAM class blocks (`retail::asset_feed`), added to every instance of the class.
	class_uv: HashMap<u32, [f32; 2]>,
	/// BRIDGE-28: the two wrench afterimage vertex ranges of the next `set_marker` (first, end, alpha byte), then the three
	/// whole-hero afterimage ranges (BRIDGE-40, WPN-6.6a); taken by it.
	marker_ghosts: [Option<(usize, usize, u8)>; 5],
	/// [PC] perf (lane R2): pose generation numbers (now, before) of the next `set_marker` (0 = unknown); taken by it. See `HeroGpu::set_poses`.
	hero_gens: (u64, u64),
	/// WPN-3.2: the held bomb's marker vertex range (first, end) and its pulse colour scale (0x80 = 1); taken by the next `set_marker`.
	marker_bomb: Option<(usize, usize, [f32; 3])>,
	/// BRIDGE-75: the hero body tilt (roll, pitch) of the next `set_marker`; taken by it (the frontend draw is untilted).
	marker_tilt: [f32; 2],
	/// BRIDGE-21c: the Clank light's vertex range in the hero model and its pulse tint (drawn with the tint override).
	marker_light: Option<(usize, usize, [f32; 3])>,
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
	part_defs: Vec<u8>,
	/// Blast moby meshes (classes 112 fireball, 122 shard) by class id; faces carry an fx_tex key.
	fx_meshes: HashMap<u32, FxMesh>,
	/// The level's fx textures by index (blob shadow 0, fire 41/42) and this frame's textured quads
	/// (shadows.rs, particles.rs fire), drawn before the sprites in order.
	fx_tex: HashMap<usize, wgpu::BindGroup>,
	fx_quads: Vec<particles::WorldQuad>,
	/// Water quads, drawn with the scene fog (fs_part_fog).
	water_quads: Vec<particles::WorldQuad>,
	/// BRIDGE-23: this frame's FIX-blend world strips (NOV-15.1b).
	strip_draws: Vec<particles::WorldStrip>,
	/// This frame's queue-C water pools (Hoven, `queue_q1::fill`), drawn before the shrubs.
	pool_draws: Vec<particles::WorldStrip>,
	/// ARI-49b (BRIDGE-63): the Aridia ocean pipelines (0 = its own fog, 1 = the level fog while the Visibomb is live),
	/// its 64x64 animated texture, this frame's patch strips, the texels and game frame last uploaded. KER-30: 2, 3 =
	/// the same pair FIX-blended without Z write (Kerwan ocean, `fs_ocean_k`).
	ocean_pipeline: [wgpu::RenderPipeline; 6],
	ocean_tex: Option<(wgpu::Texture, wgpu::BindGroup)>,
	ocean_draws: Vec<particles::WorldStrip>,
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
	movie: crate::fmv::present::Presenter,
}

struct Camera {
	pos: [f32; 3],
	yaw: f32,
	pitch: f32,
	/// Tangent of half the vertical field of view.
	tan_half: f32,
	/// BRIDGE-56 (decomp/notes/port_bridges.md): the view's up hint, Z except under the Visibomb remote camera's roll.
	up: [f32; 3],
}

/// PAL vertical/horizontal projection ratio (decomp/notes/camera.md "REN-7a port spec", f32 0x3f418937 / 0.63).
pub(crate) const PROJ_Y_RATIO: f32 = 0.756;
/// Vertical half-angle tangent of the retail gameplay projection: 0.63 * 0.756 (REN-7a).
const TAN_HALF_FOV: f32 = crate::camera::PROJ_SCALAR * PROJ_Y_RATIO;
/// Effective retail world near/far planes in metres (REN-7a): B/(z_s range + 224)/1024 of the GS z mapping.
const RETAIL_NEAR: f32 = 0.03125;
const RETAIL_FAR: f32 = 728.1632;

struct App {
	/// Frame-profile bench (viewer/bench.rs): an invisible window and no audio output.
	headless: bool,
	iso_path: String,
	levels: Vec<toc::Level>,
	level: usize,
	gpu: Option<Gpu>,
	cam: Camera,
	keys: HashSet<KeyCode>,
	looking: bool,
	last: Instant,
	/// [PC] Frame cap of the uncapped present modes (viewer/pace.rs).
	pace: pace::Pace,
	/// (png path, frames to wait) for a one-frame capture.
	shot: Option<(String, u32)>,
	/// Camera override (x, y, z, yaw deg, pitch deg) applied after the level loads.
	start: Option<[f32; 5]>,
	/// Collision for play mode: terrain and ties (shrubs are walk-through).
	world: Option<World>,
	/// Play mode.
	game: Option<Game>,
	/// What a new game needs: placed mobies, crate class boxes, kill plane.
	instances: Vec<gameplay::MobyInstance>,
	/// Enemy territories (gameplay header +0x70).
	polygons: Vec<gameplay::Polygon>,
	/// KER-60: the level's grind rails (gameplay header +0x74).
	grind_rails: Vec<gameplay::GrindRail>,
	/// NOV-3.4: the level's raw moby group lists (`gameplay::group_lists`), handed to each new `Game`.
	groups: Vec<Option<Vec<u16>>>,
	cuboids: Vec<gameplay::Cuboid>,
	/// KER-5: the cuboid records' forward basis rows (`gameplay::cuboid_basis`).
	cuboid_basis: Vec<[[f32; 3]; 3]>,
	bounds: HashMap<u32, ([f32; 3], [f32; 3])>,
	death_height: f32,
	/// The level header's ship transform (gameplay::ship).
	ship: Option<([f32; 3], f32)>,
	title: String,
	orbit: Orbit,
	/// The game's follow camera and fixed cameras (camera.rs); the orbit is the fallback.
	rig: Option<camera::CameraRig>,
	/// Mouse x motion while right-dragging, consumed by the rig each frame.
	mouse_dx: f32,
	/// Modern camera (port extra): mouse y motion and wheel zoom, consumed by the rig each frame.
	mouse_dy: f32,
	zoom_acc: f32,
	/// Cursor grabbed and hidden for the Modern camera; window focus.
	grabbed: bool,
	focused: bool,
	/// The Display mode the window was last switched to (handler.rs `display_change`; true = borderless fullscreen).
	fullscreen: bool,
	/// Hero position last frame, to reset the rig on respawn.
	rig_hero: [f32; 3],
	/// Enter play mode right after the first level loads (--play).
	autoplay: bool,
	/// Ratchet's start (feet, yaw) from the gameplay data.
	spawn: Option<([f32; 3], f32)>,
	attack_mouse: bool,
	/// Port extra: the pointer in window pixels, and the left button held on a menu (volume drag).
	cursor: Option<(f32, f32)>,
	menu_drag: bool,
	/// The level script as loaded, copied fresh into `flow` for each play.
	flow_start: Option<Flow>,
	flow: Option<Flow>,
	/// Save-anywhere: where the next `toggle_play` puts the hero (set by a load, save.rs `Where`).
	resume: Option<save::Where>,
	/// Per level left this session: the cutscene controller's progress, help slots seen and saved
	/// bits, so a revisit does not replay the intro (the original's per-level first-visit state).
	visited: HashMap<usize, (Option<i32>, Vec<i16>, std::collections::HashSet<(u32, u32)>)>,
	/// Skip input last frame, so holding it skips one cutscene, not the chain.
	skip_held: bool,
	/// The FMV streaming while a movie scene plays (fmv/player.rs).
	movie: Option<fmv::player::Player>,
	/// Retail front end, mode 6 (New Game intro; viewer/front_mirror.rs): the capture's display-buffer picture (caption screens,
	/// fades) is up, and a movie that ended keeps its last picture under the fade until the next picture covers it.
	fm_ov: bool,
	fm_movie_hold: bool,
	/// The ship's departure sequence after the Galactic Map (viewer/travel.rs).
	travel: Option<travel::Travel>,
	/// The ship takeoff / landing stream playing (travel.rs).
	ship_play: Option<travel::ShipPlay>,
	/// The departure flight's level data: path points and the two camera poses (SCN-4.3a); None without a ship pad.
	flight_data: Option<gameplay::FlightData>,
	/// A level change the script asked for, done after this frame.
	pending_level: Option<usize>,
	/// Live transient debris mobys (decomp/notes/particles.md, FUN_00263be8).
	debris: Vec<particles::Debris>,
	/// Bounding spheres of the debris classes (`MobyClass::bsphere`), for the robot pieces' spin and blast.
	debris_bspheres: HashMap<u32, [f32; 4]>,
	/// WPN-6.6b1: class header +0x46 types of kind 5 / 9 (`MobyClass::kind`), copied to `Game::class_kinds`.
	class_kinds: HashMap<u32, u16>,
	/// BRIDGE-53: class header +0x24 scales (`MobyClass::scale * 1024`) of every loaded class, copied to `Game::class_scales`.
	class_scales: HashMap<u32, f32>,
	/// Class collision blocks for the hero push-out (Game::set_colliders).
	colliders: HashMap<u32, mobies::Collision>,
	/// Ratchet's class and animations; his and the held gadgets' clocks (heroanim.rs).
	hero_anim: Option<(mobies::MobyClass, anim::Animated)>,
	hero_rig: heroanim::HeroRig,
	/// RNG-4c: the hero moby's synced position and yaw (retail +0x10 / +0xc0, written by FUN_002299e8 after the hero's motion);
	/// the water effects of the next tick read it. `None` after a teleport, respawn or load.
	hero_moby: Option<([f32; 3], f32)>,
	/// Pause hero preview items: id -> (hero joint, item mesh in its joint frame).
	hero_items: HashMap<u8, HeroItem>,
	/// Seq index of menu clip 0 in `hero_anim` (None when the clips failed to load).
	menu_clip_base: Option<usize>,
	/// Pause hero extra actors by item id: (class, sequence, scale), drawn after the item in the preview mesh.
	hero_actors: HashMap<u8, Vec<(u32, usize, f32)>>,
	/// Their classes, with the aux clips installed in the sequence slots.
	menu_actor_classes: HashMap<u32, (mobies::MobyClass, anim::Animated)>,
	/// The pause hero preview's item, the clock when it was selected and the pose it fades in from.
	hero_preview_clip: Option<MenuHero>,
	/// The panel item model's class and the clock when it was spawned (respawned when the class changes).
	panel_model: Option<(u32, f32)>,
	/// Worn-item candidates of the hero preview (UI-3b) and the back item's clock.
	hero_worn: WornSet,
	worn_back: Option<anim::BackAnim>,
	/// Animated prop classes (enemies), looping their first sequence for now.
	prop_anims: HashMap<u32, (mobies::MobyClass, anim::Animated)>,
	/// Scene actors other than Ratchet, one model slot each after the game props: (scene id, actor
	/// index, class id); their classes are in `prop_anims`.
	actor_slots: Vec<(u32, usize, u32)>,
	clock: f32,
	/// Recent game messages with seconds left on screen.
	messages: Vec<(String, f32)>,
	/// Shown bolt count, rolling up toward the real one like the game's counter.
	shown_bolts: f32,
	/// Unsimulated time for the fixed-step game loop.
	accum: f32,
	/// PC-FPS-1: render interpolation between the last two game ticks (smooth.rs).
	smooth: smooth::Smooth,
	/// PC perf: (game frame counter, props address, props count) when the last frame's prop poses were built; a frame with
	/// no game tick and the same stamp moves those poses over instead of skinning them again (same pose keys, same anim state).
	pose_stamp: Option<(u32, usize, usize)>,
	/// PC perf (lane A B1): the hero's posed bundle of the last tick (viewer/herocache.rs).
	hero_cache: Option<herocache::HeroCache>,
	/// The wrench in Ratchet's hand: its vertices and normals in his wrist joint's frame.
	wrench: Option<Posed>,
	/// Wrench joints 0 and 1 in the held pose, in the wrist frame (the hit sweep's endpoints, FUN_00259918).
	wrench_axis: Option<[[f32; 3]; 2]>,
	/// The wrench's own model and animation (swing sequences 3-5 and 10 extend it, decomp/notes/wrench.md section 7).
	wrench_rig: Option<Held>,
	/// Retail mode (M3a): the hero parts of the last gameplay field (held wrench, back objects, +0x64 joint records,
	/// loose class-71 mobys), kept while frozen; and the clocks of the loose wrenches by moby address.
	retail_parts: Option<crate::retail::HeroParts>,
	loose_anims: HashMap<u32, anim::PropAnim>,
	/// The level's decoded cutscenes and their English speech, by scene id.
	scenes: HashMap<u32, (scene::Scene, Option<audio::Clip>)>,
	/// The scene whose speech is playing, and the voice.
	scene_voice: Option<(u32, Option<u64>)>,
	/// NOV-13.5: the playing fanfare voice of the gadget cinematic.
	gadget_voice: Option<u64>,
	/// BRIDGE-37 (d): the shop portrait's voice slot and the id waiting for the voice stream to be free.
	portrait_voice: Option<u64>,
	portrait_pending: Option<u16>,
	/// NOV-34a: the playing ship boarding / landing voice (40000-table cue).
	ship_voice: Option<u64>,
	/// NOV-13.5b: this tick's gadget cinematic frame (camera, fade, actors).
	gadget_frame: Option<game::gadget_scene::SceneFrame>,
	art: HudArt,
	/// Seconds left on the bolt and nanotech counters (they show on a change, then fade).
	hud_t: (f32, f32),
	/// Ammo HUD: the weapon shown, bar/icon fade f1 and text fade f2 (0..1 = bytes +0x70/+0x71 over 8).
	ammo_hud: (Option<u8>, f32, f32),
	hud_seen: (u32, i32),
	/// Nanotech meter tick state (lifetime, f1, f2, displayed; displayed -1 = not initialised) and the whole-tick accumulator.
	nano_hud: (i32, u8, u8, i32),
	/// NOV-30f: nanotech slot 1 holds a live request (cleared by the shop and on expiry, set by damage).
	nano_live: bool,
	nano_acc: f32,
	audio: Option<audio::Audio>,
	sfx: Sfx,
	/// The global bank's menu sounds for the pause menu, loaded the first time it opens in play.
	ui_sfx: Vec<Option<audio::Clip>>,
	/// HelpDesk voices by voice id (None = none on the disc), and the playing one: (help id, voice).
	help_clips: HashMap<usize, Option<audio::Clip>>,
	help_voice: Option<(u32, u64)>,
	/// The level's positional ambient emitters and, per emitter, its voice and retrigger countdown.
	emitters: Vec<ambience::Emitter>,
	emitter_state: Vec<(Option<u64>, f32)>,
	/// SCH-6b: the class table's loaded index per class (the effect mobys' scheduler rank).
	class_ranks: std::collections::HashMap<u32, u32>,
	/// [PC] The host-side parse of the level retail is travelling to, started early (`prefetch_level`): (level, thread).
	prefetch: Option<(usize, Box<dyn std::any::Any + Send>)>,
	/// [PC] The channels to that thread: what the slot order needs from it, and the order back (it builds the scene meanwhile).
	prefetch_link: Option<(usize, app::PrefetchLink)>,
	/// The level switch waits for the prefetch's scene build (no new picture meanwhile).
	switch_hold: bool,
	/// [PC] Frames left in which the world draws black after a retail level switch, until retail presents its first picture of the
	/// new level (its fade-in overlay is not up on the switch frame: the new level flashed in full brightness).
	arrival_black: u32,
	/// The level a prefetch was started for last (not started twice for one travel).
	prefetch_tried: Option<usize>,
	class_rows: std::collections::HashMap<u32, u32>,
	/// BRIDGE-24: box-emitter plays decided in the tick (emitter index, point, gain), started by the frame's voice code.
	ambient_plays: Vec<(usize, [f32; 3], f32)>,
	/// BRIDGE-24: volume boxes whose countdown fired in the tick.
	volume_start: Vec<bool>,
	/// Doppler / underwater listener state (sound.rs) and its 60 Hz tick accumulator.
	listener: sound::ListenerVelocity,
	/// NOV-28: the camera zone fog (sticky, reset to the level fog at load) and the fog swap computed last frame.
	fog_zone: ([u8; 3], [f32; 4]),
	fog_next: Option<([u8; 3], [f32; 4])>,
	/// NOV-28.2: next frame's pool-particle far cap, chosen with `fog_next`.
	part_far_next: f32,
	/// Voices of the level's volume boxes (type 1) and underwater loops (type 5), by zone.
	volume_voices: Vec<Option<u64>>,
	underwater_voices: Vec<Option<u64>>,
	/// Per toad/robot prop index: animation sound cue clock and loop-sound voice (sound.md "Animation sound cues").
	enemy_sfx: HashMap<usize, EnemySfx>,
	live_shots: Vec<LiveShot>,
	/// BRIDGE-50: the voice of the last `Event::HeldSound` (0: none).
	held_click: u64,
	/// NOV-6.3: looping class-sound voices by owner prop index (`game::Event::SoundLoop`).
	loops: HashMap<usize, u64>,
	/// AUD-11i: loop owners whose voice is wanted (their cue), driven each frame by `loop_act`.
	loop_want: HashMap<usize, sound::Cue>,
	/// BRIDGE-49: the bend value the game last wrote per loop owner (`game::Event::LoopBend`).
	loop_bend: HashMap<usize, i32>,
	/// The hero's death sequence trigger clock (sequence, position) for its class 0 cues.
	/// The level's music in slot order (intro, loop, ...), and the playing (voice, slot, seconds).
	music: Vec<audio::Clip>,
	/// Music controller (music.rs) and its main-stream and stinger voices as (voice, slot); the
	/// zones, spawn areas and the reverb request they feed (ambience.md).
	music_player: Option<music::Player>,
	music_voice: Option<(u64, usize)>,
	sting_voice: Option<(u64, usize)>,
	music_deaths: u32,
	zones: ambience::Zones,
	areas: Vec<ambience::Area>,
	reverb: ambience::Reverb,
	/// SAV-1c: checkpoint environment captured after the last commit (None until the first), and the serial seen.
	ckpt_env: Option<CkptEnv>,
	ckpt_seen: u32,
	/// The pause menu has frozen every voice (music.md "Pause audio").
	audio_paused: bool,
	/// Last frame's state, to turn changes into sounds.
	prev_swing: Option<usize>,
	prev_shown: Vec<bool>,
	/// Effect pool (particles.rs) and the level's class-760 smoke emitters with their volumes.
	particles: particles::Particles,
	smoke: Vec<(particles::SmokePvars, particles::Cuboid)>,
	/// Class 760 fire quads (fire.rs).
	fires: Vec<crate::fire::Fire>,
	/// Ripple water (Novalis only so far).
	water: Option<crate::water::Water>,
	/// NOV-14.5b: the cutscene ship gloss (Novalis only) and the quads of the tick it last ran.
	ship_gloss: Option<crate::shipgloss::ShipGloss>,
	/// ARI-49a / KER-30: the Aridia (class 854) or Kerwan (class 994) ocean; None on other levels.
	ocean: Option<crate::ocean::Ocean>,
	/// The level overlay image: Veldin's ship reflection tables (REN-2), Novalis' strip tables (NOV-15.1b / NOV-23.1).
	level_overlay: Option<Vec<u8>>,
	gloss_quads: Vec<particles::WorldQuad>,
	/// This frame's blast moby draws, and the scene-5 ship crash state (last tick handled, previous joint 5).
	effect_mobys: Vec<particles::EffectMoby>,
	sprites: Vec<particles::Sprite>,
	ribbons: Vec<particles::Ribbon>,
	/// KER-92: this frame's one-pixel pool lines (type-0x31 tracers).
	lines: Vec<particles::Line>,
	/// Load the front end (title scene and menu) instead of `level` on the next `load_level`.
	want_front: bool,
	front: Option<FrontEnd>,
	/// The save slot being played (None = not saving) and its image, kept current on level changes.
	save: Option<(Option<usize>, save::Save)>,
	/// Play timer A3 (1/60 s units, PAL step in save::tick_play), its 50 Hz accumulator and the
	/// post-menu skipped frame flag.
	a3_ticks: u32,
	a3_acc: f32,
	a3_skip: bool,
	a3_menu: bool,
	was_dead: bool,
	/// Circle on the left mouse button in retail play (middle in the clean-room game); the `--give` item; fixed steps since play started.
	fire_mouse: bool,
	/// Triangle (Quick Select) on the middle mouse button in retail play.
	quick_mouse: bool,
	give: Option<(u8, Option<u16>)>,
	play_ticks: u32,
	/// RAC_ATTACK_TICKS: play ticks the wrench is pressed for 3 ticks (scripted --shot scenes).
	attack_script: Vec<u32>,
	/// The two camera shake records (camera.md "Shake consumer").
	shake: camera::CamShake,
	/// The held weapons: per gadget class, its model (bind pose and animation) and the hero joint it
	/// follows (wrench 71 at the wrist 56, glove 192 at joint 54), plus the bomb in the glove.
	glove: Option<Held>,
	bomb: Option<(Vec<[f32; 3]>, Vec<[f32; 3]>)>,
	/// The held Pyrocitor (game/pyro.rs), the muzzle it put in the world last frame and its firing loop voice.
	pyro_rig: Option<PyroRig>,
	/// The held Suck Cannon (class 849, WPN-5.1b2), its suction loop voice and the held matrix it posed last frame.
	suck_rig: Option<PyroRig>,
	/// BRIDGE-38 (WPN-6.0): the generic held-weapon rigs by class (game/held.rs `HELD_RIG_CLASSES`) and the muzzle posed last frame.
	held_rigs: HashMap<u32, PyroRig>,
	/// BRIDGE-76 (ARI-47c): the Swingshot hook model (class 209), drawn at the game's hook with the hero mesh's held rigs.
	hook_rig: Option<Held>,
	held_muzzle: Option<[f32; 3]>,
	/// BRIDGE-38 (c) extension (WPN-6.2a): the posed held weapon's world matrix rows and position, posed last frame.
	held_xform: Option<([[f32; 3]; 3], [f32; 3])>,
	/// BRIDGE-38 (d) (WPN-6.4a): the posed held weapon's joint-0 rotation rows in the world, posed last frame.
	held_joint0: Option<[[f32; 3]; 3]>,
	/// BRIDGE-21: the hero's back objects d0 (class 607) and d4 (class 601) and the hero joint of attach point 5 they sit on.
	back_rig: Option<Held>,
	clank_rig: Option<Held>,
	back_joint: Option<usize>,
	/// BRIDGE-21c: the Clank light (class 1204) mesh and this frame's glow position in the world.
	light_rig: Option<mobies::MobyClass>,
	clank_light_glow: Option<[f32; 3]>,
	suck_voice: Option<u64>,
	suck_matrix: Option<[[f32; 3]; 3]>,
	pyro_muzzle: Option<[f32; 3]>,
	/// NOV-14.7b-draw: UV extent of the glint sprite, 32 texels over the width of fx texture 0x13.
	glint_u: f32,
	/// ARI-4: the Aridia sky sprite records (None until the first tick after a load creates them).
	sky_sprites: Option<Vec<sky::SkySprite>>,
	/// FRAME-1: the rig's newest tick camera plus shake (pos, yaw, pitch, tan_half) for the per-tick glint cull.
	glint_cam: Option<([f32; 3], f32, f32, f32)>,
	/// BRIDGE-56: the up of `glint_cam` (the rig's `up()`; Z when `glint_cam` is None).
	glint_up: [f32; 3],
	/// FRAME-1e: this tick's scene / ship stream / gadget camera (pos, yaw, pitch, tan_half) while one owns the view.
	over_cam: Option<([f32; 3], f32, f32, f32)>,
	/// FRAME-1c: a sim tick of this frame stepped the rig while it owned the view (glint cull and draw verdict ran per tick).
	tick_cam_live: bool,
	pyro_voice: Option<u64>,
	/// RNG-4b: voices granted at their request point in the tick, and the pitch draw of each running voice.
	voices: VoiceBook,
	pyro_frames: game::pyro::Frames,
	/// The Options values (settings.rs) and whether they still need writing to disk.
	settings: Settings,
	settings_dirty: bool,
	/// The pause menu or Galactic Map over the frozen game (decomp/notes/ingame_pause_menu.md), with
	/// the all_text strings and font it draws with.
	pause: Option<frontend::Menu>,
	/// SCN-6 (decomp/notes/camera.md "SCN-6 port spec"): the projection saved at pause open (retail 0x1ba1d8) and
	/// restored on every pause close; the paused backdrop uses the default 0.63 projection.
	pause_saved_tan_half: Option<f32>,
	ui_text: Option<frontend::Text>,
	ui_art: Option<frontend::Art>,
	help_log: HelpLog,
	/// Autosave enabled ([0x13d384]): set by New/Load Game and manual save, cleared by Quit and IO errors.
	autosave_on: bool,
	/// Seconds the HUD save icon still shows, and whether help 20011 has been shown this session.
	save_icon: f32,
	save_help: bool,
	/// A cutscene was playing last frame (its end is an autosave point).
	was_scene: bool,
	/// The pickup popup: text id, hold timer (50 Hz ticks) and fade alpha 0..=128 (decomp/notes/ingame_menu_leftovers.md P1-b).
	popup: Option<(u32, i32, i32)>,
	/// Triangle (T) held seconds, and the Quick Select wheel's sector while it is open.
	tri_held: Option<f32>,
	wheel: Option<usize>,
	/// The wheel's fade 0..1 (up while open, down after release) and the sector it shows.
	wheel_fade: (f32, usize),
	/// The level's Galactic Map (decomp/notes/ingame_menu_leftovers.md P1-c).
	map: Option<MapView>,
	/// RAC_PAUSE_KEYS: scripted pause-menu presses in play (for --shot) and frames until the next.
	pause_script: Vec<char>,
	pause_script_t: u32,
	/// Retail mode (retail.rs): the interpreter that runs the level instead of the rac-rs game step.
	retail: Option<crate::retail::Retail>,
	/// The running `Retail` across a retail level travel (`Retail::switch`), for the next `toggle_play`.
	retail_carry: Option<crate::retail::Retail>,
	/// Retail mode: the level's prop meshes (`load_level`), kept so a pool that outgrew its slots rebuilds only the prop
	/// slots (`regrow_props`), not the level.
	retail_prop_meshes: HashMap<u32, crate::mesh::Mesh>,
}

/// The Galactic Map: the bundle, the explored mask, its HUD texture slot and the world box it fits.
struct MapView {
	bundle: map::MapBundle,
	explored: map::Explored,
	/// The save codec's span table for this map (map bundle section 1).
	spans: Vec<u8>,
	slot: usize,
	lo: [f32; 2],
	hi: [f32; 2],
	/// The hero's map pixel and heading, for the marker.
	hero: [f32; 2],
	yaw: f32,
	affine: Option<[f32; 4]>,
	/// Select-map zoom (initial 0.65, clamped 0.65..4.0) and the map pixel at the panel centre
	/// (decomp/notes/ingame_menu_leftovers.md P1-c).
	zoom: f32,
	pan: [f32; 2],
}

/// BRIDGE-21: hero back objects (classes 607 / 601) and the hero attach point they follow.
const BACK_D0_CLASS: u32 = 607;
const BACK_D4_CLASS: u32 = 601;
const BACK_ATTACH_POINT: usize = 5;
/// BRIDGE-21c: the Clank light class, its d4 joint, class-scale factor, and the glow (half size, camera bias, colour; z lift).
const CLANK_LIGHT_CLASS: u32 = 1204;
const CLANK_LIGHT_JOINT: usize = 6;
const CLANK_LIGHT_SCALE: f32 = 1.3;
const CLANK_LIGHT_GLOW: (f32, f32, u32) = (0.057, 0.08, 0x280000c0);
const CLANK_LIGHT_GLOW_Z: f32 = 0.015;

/// Vertices after the suck cannon in the hero mesh: d0, d4 and the Clank light.
fn back_tail(back: &Option<Held>, clank: &Option<Held>, light: &Option<mobies::MobyClass>) -> usize {
	rig_len(back) + rig_len(clank) + light.as_ref().map_or(0, |c| c.mesh.positions.len())
}
/// BRIDGE-40 (WPN-6.6a): the three whole-hero afterimage copies, last in the hero mesh tail.
fn hero_ghost_len(hero: &mobies::MobyClass) -> usize {
	3 * hero.mesh.positions.len()
}
/// Vertices of a back rig's mesh (0 when absent): its slot in the hero mesh tail.
fn rig_len(r: &Option<Held>) -> usize {
	r.as_ref().map_or(0, |h| h.class.mesh.positions.len())
}

/// Vertices of every generic held rig mesh (their slot in the hero mesh, after the Suck Cannon).
fn held_len(rigs: &HashMap<u32, PyroRig>, hook: &Option<Held>) -> usize {
	game::held::rig_classes().into_iter().filter_map(|c| rigs.get(&c)).map(|r| r.held.class.mesh.positions.len()).sum::<usize>() + hook.as_ref().map_or(0, |h| h.class.mesh.positions.len())
}

/// A held gadget model posed by its own skeleton.
struct Held {
	class: mobies::MobyClass,
	anim: anim::Animated,
}

/// The Pyrocitor (gadget class 176) at the hero joint of its attach point, its own joint for attach point 0 (the
/// muzzle) and the class 179 nozzle in bind pose (mesh, positions, normals). decomp/notes/weapons_framework.md
/// "Pyrocitor".
struct PyroRig {
	held: Held,
	joint: usize,
	muzzle: Option<usize>,
	nozzle: Option<(Mesh, Vec<[f32; 3]>, Vec<[f32; 3]>)>,
}

/// The front end: menu state over the looping title scene (frontend.rs, notes/title_implementation_prompt.md).
struct FrontEnd {
	menu: frontend::Menu,
	text: frontend::Text,
	art: Option<frontend::Art>,
	/// Scene tick (50 per second), looping over the scene.
	tick: f32,
	/// Ambience: the looping bed and the timed sounds' voices.
	bed: Vec<u64>,
	timed: [Option<u64>; 2],
	/// RAC_MENU_KEYS: scripted presses (for --shot) and frames until the next one.
	script: Vec<frontend::Key>,
	script_t: u32,
	/// UI-11c: the title attract loop, its 50 Hz tick accumulator and the playing movie.
	att: frontend::Attract,
	acc: f32,
	attract_movie: Option<fmv::player::Player>,
}

/// Title ambience in the global bank [I]: looping bed sounds, then (sound, first tick, last tick) windows.
const TITLE_BED: [usize; 3] = [2, 3, 8];
/// Language slot of all_text per menu language (EN, FR, DE, ES, IT; slot 1 is unused).
const TEXT_SLOT: [usize; 5] = [0, 2, 3, 4, 5];

pub fn run(iso_path: &str, level: usize, shot: Option<String>, start: Option<[f32; 5]>, play: bool, front: bool, give: Option<(u8, Option<u16>)>) -> Result<(), Box<dyn std::error::Error>> {
	init_pool();
	let mut app = new_app(iso_path, level, shot, start, play, front, give)?;
	profile::init_from_env();
	let event_loop = EventLoop::new()?;
	event_loop.run_app(&mut app)?;
	Ok(())
}

/// [PC] the frame worker pool (rayon) that the per-item frame work (skinning, prop / tie / marker vertex rebuilds) spreads
/// over: RAC_THREADS sets its size (1 = everything on the calling thread), else the hardware threads up to `POOL_MAX`
/// (the per-frame parallel work is ~1.5 ms and stops gaining past 4; larger pools lose more to
/// preempted workers and to idle workers spinning on the render thread's sibling core, which raises the p99). The calling (render)
/// thread is worker 0, so it takes its share of every parallel section and steals while it waits instead of sleeping on
/// a latch (a blocked render thread lost its core and caches). Results are written to fixed slots, so the output does not
/// depend on the thread count. A second call (tests, the bench) keeps the first pool.
fn init_pool() {
	const POOL_MAX: usize = 4;
	let n = std::env::var("RAC_THREADS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()).min(POOL_MAX));
	let _ = rayon::ThreadPoolBuilder::new().num_threads(n).use_current_thread().thread_name(|i| format!("frame-worker-{i}")).build_global();
}

fn new_app(iso_path: &str, level: usize, shot: Option<String>, start: Option<[f32; 5]>, play: bool, front: bool, give: Option<(u8, Option<u16>)>) -> Result<App, Box<dyn std::error::Error>> {
	let mut disc = iso::Disc::open(std::path::Path::new(iso_path))?;
	let levels = toc::read_levels(&mut disc)?;
	Ok(App {
		headless: false,
		iso_path: iso_path.to_string(),
		levels,
		level,
		gpu: None,
		cam: Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: TAN_HALF_FOV, up: [0.0, 0.0, 1.0] },
		keys: HashSet::new(),
		looking: false,
		last: Instant::now(),
		pace: pace::Pace::new(),
		// RAC_SHOT_FRAMES waits longer before the capture (to get past the opening in play mode).
		shot: shot.map(|p| (p, std::env::var("RAC_SHOT_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(5))),
		start,
		world: None,
		game: None,
		instances: Vec::new(),
		polygons: Vec::new(),
		grind_rails: Vec::new(),
		groups: Vec::new(),
		cuboids: Vec::new(),
		cuboid_basis: Vec::new(),
		bounds: HashMap::new(),
		death_height: f32::MIN,
		ship: None,
		title: String::new(),
		orbit: Orbit { yaw: 0.0, pitch: player::CAMERA_PITCH, distance: player::CAMERA_DISTANCE },
		rig: None,
		mouse_dx: 0.0,
		mouse_dy: 0.0,
		zoom_acc: 0.0,
		grabbed: false,
		focused: true,
		fullscreen: false,
		rig_hero: [0.0; 3],
		autoplay: play,
		spawn: None,
		attack_mouse: false,
		cursor: None,
		menu_drag: false,
		flow_start: None,
		flow: None,
		resume: None,
		visited: HashMap::new(),
		skip_held: false,
		movie: None,
		fm_ov: false,
		fm_movie_hold: false,
		travel: None,
		ship_play: None,
		flight_data: None,
		debris: Vec::new(),
		pending_level: None,
		hero_anim: None,
		hero_rig: heroanim::HeroRig::new(),
		hero_moby: None,
		hero_items: HashMap::new(),
		menu_clip_base: None,
		hero_actors: HashMap::new(),
		menu_actor_classes: HashMap::new(),
		hero_preview_clip: None,
		panel_model: None,
		hero_worn: WornSet::default(),
		worn_back: None,
		prop_anims: HashMap::new(),
		actor_slots: Vec::new(),
		clock: 0.0,
		messages: Vec::new(),
		shown_bolts: 0.0,
		accum: 0.0,
		smooth: Default::default(),
		pose_stamp: None,
		hero_cache: None,
		wrench: None,
		wrench_axis: None,
		wrench_rig: None,
		retail_parts: None,
		loose_anims: HashMap::new(),
		scenes: HashMap::new(),
		scene_voice: None,
		gadget_voice: None,
		portrait_voice: None,
		portrait_pending: None,
		ship_voice: None,
		gadget_frame: None,
		art: HudArt::default(),
		hud_t: (0.0, 0.0),
		ammo_hud: (None, 0.0, 0.0),
		hud_seen: (u32::MAX, -1),
		nano_hud: (0, 0, 0, -1),
		nano_live: true,
		nano_acc: 0.0,
		audio: None,
		sfx: Sfx::default(),
		ui_sfx: Vec::new(),
		help_clips: HashMap::new(),
		help_voice: None,
		emitters: Vec::new(),
		emitter_state: Vec::new(),
		class_ranks: std::collections::HashMap::new(),
		prefetch: None,
		prefetch_link: None,
		switch_hold: false,
		arrival_black: 0,
		prefetch_tried: None,
		class_rows: std::collections::HashMap::new(),
		ambient_plays: Vec::new(),
		volume_start: Vec::new(),
		listener: sound::ListenerVelocity::default(),
		fog_zone: ([0; 3], [0.0, 1.0e9, 255.0, 255.0]),
		fog_next: None,
		part_far_next: crate::water::PART_FAR,
		volume_voices: Vec::new(),
		underwater_voices: Vec::new(),
		enemy_sfx: HashMap::new(),
		live_shots: Vec::new(),
		held_click: 0,
		loops: HashMap::new(),
		loop_want: HashMap::new(),
		loop_bend: HashMap::new(),
		music: Vec::new(),
		music_player: None,
		music_voice: None,
		sting_voice: None,
		music_deaths: 0,
		zones: ambience::Zones::default(),
		areas: Vec::new(),
		reverb: ambience::Reverb::default(),
		ckpt_env: None,
		ckpt_seen: 0,
		audio_paused: false,
		prev_swing: None,
		prev_shown: Vec::new(),
		particles: particles::Particles::new(),
		smoke: Vec::new(),
		fires: Vec::new(),
		water: None,
		ship_gloss: None,
		ocean: None,
		level_overlay: None,
		gloss_quads: Vec::new(),
		effect_mobys: Vec::new(),
		sprites: Vec::new(),
		ribbons: Vec::new(),
		lines: Vec::new(),
		want_front: front,
		front: None,
		save: None,
		a3_ticks: 0,
		a3_acc: 0.0,
		a3_skip: false,
		a3_menu: false,
		was_dead: false,
		fire_mouse: false,
		quick_mouse: false,
		give,
		play_ticks: 0,
		attack_script: std::env::var("RAC_ATTACK_TICKS").unwrap_or_default().split(',').filter_map(|v| v.trim().parse().ok()).collect(),
		debris_bspheres: HashMap::new(),
		class_kinds: HashMap::new(),
		class_scales: HashMap::new(),
		colliders: HashMap::new(),
		shake: camera::CamShake::default(),
		glove: None,
		bomb: None,
		pyro_rig: None,
		suck_rig: None,
		held_rigs: HashMap::new(),
		hook_rig: None,
		held_muzzle: None,
		held_xform: None,
		held_joint0: None,
		back_rig: None,
		clank_rig: None,
		back_joint: None,
		light_rig: None,
		clank_light_glow: None,
		suck_voice: None,
		suck_matrix: None,
		pyro_muzzle: None,
		glint_u: 1.0,
		sky_sprites: None,
		glint_cam: None,
		glint_up: [0.0, 0.0, 1.0],
		over_cam: None,
		tick_cam_live: false,
		pyro_voice: None,
		voices: VoiceBook::default(),
		pyro_frames: game::pyro::Frames::default(),
		settings: Settings::load(),
		settings_dirty: false,
		pause: None,
		pause_saved_tan_half: None,
		ui_text: None,
		ui_art: None,
		help_log: HelpLog::default(),
		autosave_on: false,
		save_icon: 0.0,
		save_help: false,
		was_scene: false,
		popup: None,
		tri_held: None,
		wheel: None,
		wheel_fade: (0.0, 0),
		map: None,
		pause_script: std::env::var("RAC_PAUSE_KEYS").unwrap_or_default().chars().collect(),
		pause_script_t: 0,
		retail: None,
		retail_carry: None,
		retail_prop_meshes: HashMap::new(),
	})
}

/// 4x MSAA for the scene pass (port extra: modern display, no PS2 aliasing).
const MSAA: u32 = 4;

/// REN-9b: the shop item-panel render target (retail 512x128 at GS 0x3be000): 4x MSAA colour and depth resolved into a
/// sampled texture. `bind` samples the resolved colour; the HUD draws it for `Quad.tex == ITEM_PANEL_TEX`.
struct ItemPanel {
	msaa: wgpu::TextureView,
	depth: wgpu::TextureView,
	resolved: wgpu::TextureView,
	bind: wgpu::BindGroup,
}
/// REN-9b: the shop's item poses split into world mobies and the panel-1 item (never both).
fn split_item_poses(poses: &[game::vendor::ItemPose]) -> (Vec<(u32, [f32; 16])>, Vec<(u32, [f32; 16])>) {
	let pick = |dest| poses.iter().filter(|p| p.dest == dest).map(|p| (p.class, place(p.pos, p.rot))).collect();
	(pick(game::vendor::ItemDest::World), pick(game::vendor::ItemDest::Panel))
}
/// Target size in texels (TW 9, TH 7).
const ITEM_PANEL_SIZE: (u32, u32) = (512, 128);
/// `Quad.tex` value that selects the item-panel target instead of an overlay texture.
pub(crate) const ITEM_PANEL_TEX: usize = usize::MAX;
/// BRIDGE-36 (b): `Quad.tex` selecting the purchase popup target.
pub(crate) const POPUP_PANEL_TEX: usize = usize::MAX - 1;
/// BRIDGE-37 (b): `Quad.tex` selecting the shop portrait target.
pub(crate) const PORTRAIT_PANEL_TEX: usize = usize::MAX - 2;
/// `Quad.tex` selecting the retail GS overlay (retail.rs): drawn with out = rgb + a * scene.
pub(crate) const RETAIL_OV_TEX: usize = usize::MAX - 3;
/// `Quad.tex` selecting the retail screen filter's untextured pipeline (retail/queue_b.rs `screen_quads`): GS ALPHA
/// 0x42 out = Cd - Cs (colour premultiplied by As on the CPU), clamped at 0 per channel.
pub(crate) const RETAIL_FILTER_TEX: usize = usize::MAX - 4;
/// NOV-30a: panel targets (item, popup, portrait) draw their vertices with the MAIN projection (retail
/// moby_draw_build reads the view x clip matrix the panel path never rebuilds), centred on the 512x128 target:
/// 256/0.63 px per unit tan horizontally, 224/(0.63*0.756) vertically. Over the 64 px target half height the
/// tan shrinks by 64/224, and the aspect makes the horizontal scalar 1/0.63 again (224 / (64 * 0.756)).
const PANEL_TAN_SCALE: f32 = 64.0 / 224.0;
const PANEL_ASPECT: f32 = 224.0 / (64.0 * PROJ_Y_RATIO);

/// The multisampled colour target, resolved into the swapchain image.
fn msaa_view(device: &wgpu::Device, format: wgpu::TextureFormat, w: u32, h: u32) -> wgpu::TextureView {
	device
		.create_texture(&wgpu::TextureDescriptor {
			label: Some("msaa"),
			size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: MSAA,
			dimension: wgpu::TextureDimension::D2,
			format,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			view_formats: &[],
		})
		.create_view(&Default::default())
}

/// Box-filtered mip chain below `t` (level 0 excluded); alpha is averaged too.
fn mip_chain(t: &Texture) -> Vec<(u32, u32, Vec<[u8; 4]>)> {
	let mut out = Vec::new();
	let (mut w, mut h, mut px) = (t.w as u32, t.h as u32, t.rgba.clone());
	while w > 1 || h > 1 {
		let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
		let mut next = Vec::with_capacity((nw * nh) as usize);
		for y in 0..nh {
			for x in 0..nw {
				let mut acc = [0u32; 4];
				for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
					let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
					let c = px[(sy * w + sx) as usize];
					for i in 0..4 {
						acc[i] += c[i] as u32;
					}
				}
				next.push(acc.map(|v| ((v + 2) / 4) as u8));
			}
		}
		out.push((nw, nh, next.clone()));
		(w, h, px) = (nw, nh, next);
	}
	out
}

fn depth_view(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::TextureView {
	device
		.create_texture(&wgpu::TextureDescriptor {
			label: Some("depth"),
			size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: MSAA,
			dimension: wgpu::TextureDimension::D2,
			format,
			// [PC] Sampled by the Enhanced image (post.rs): its occlusion reads the level's depth.
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		})
		.create_view(&Default::default())
}

/// Class 760: the crash-site smoke emitter.
const SMOKE: u32 = 760;

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalise(v: [f32; 3]) -> [f32; 3] {
	let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
	[v[0] / l, v[1] / l, v[2] / l]
}

/// Inverse of a 3x3 matrix; a cuboid's half-axes are the rows of its stored inverse's inverse.
fn invert3(m: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
	let c = [cross(m[1], m[2]), cross(m[2], m[0]), cross(m[0], m[1])];
	let det = m[0][0] * c[0][0] + m[0][1] * c[0][1] + m[0][2] * c[0][2];
	(det.abs() > 1e-12).then(|| std::array::from_fn(|i| std::array::from_fn(|j| c[j][i] / det)))
}

fn buffer(device: &wgpu::Device, label: &str, data: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
	let buf = device.create_buffer(&wgpu::BufferDescriptor {
		label: Some(label),
		size: (data.len() as u64).max(4).next_multiple_of(4),
		usage: usage | wgpu::BufferUsages::COPY_DST,
		mapped_at_creation: false,
	});
	buf
}



/// Column-major placement of a moby instance: rotation Z * Y * X (radians), then translation.
/// A tinted prop's vertex `j` from the tint's first vertex on: constant unlit colour (baked code, draw alpha below -2.0), else unchanged.
#[cfg(test)]
fn tint_override(colour: [f32; 4], j: usize, tint: Option<([f32; 3], usize)>, alpha: u8) -> [f32; 4] {
	match tint.filter(|&(_, s)| j >= s) {
		Some((c, _)) => [c[0], c[1], c[2], moby_baked_encoding(alpha)],
		None => colour,
	}
}

fn moby_matrix(m: &gameplay::MobyInstance) -> [f32; 16] {
	scale_rows(place(m.pos, m.rot), m.scale)
}

/// Retail scales the three rotation columns by the moby scale (class +0x24 * placement +0x1c); the translation stays.
fn scale_rows(mut at: [f32; 16], s: f32) -> [f32; 16] {
	for v in &mut at[..12] {
		*v *= s;
	}
	at
}

/// `place` of a retail prop: a mirrored one (moby +0x34 bit 0x8000, `Prop::mirror`) has retail's basis row 1 negated, a reflection in
/// model Y: column 1 of the placement (the image of model Y) is negated, `rot` stays the plain rotation.
fn place_mirrored(pos: [f32; 3], rot: [f32; 3], mirror: bool) -> [f32; 16] {
	let mut m = place(pos, rot);
	if mirror {
		for v in &mut m[4..8] {
			*v = -*v;
		}
	}
	m
}

fn place(pos: [f32; 3], rot: [f32; 3]) -> [f32; 16] {
	let (sx, cx) = rot[0].sin_cos();
	let (sy, cy) = rot[1].sin_cos();
	let (sz, cz) = rot[2].sin_cos();
	[
		cz * cy, sz * cy, -sy, 0.0,
		cz * sy * sx - sz * cx, sz * sy * sx + cz * cx, cy * sx, 0.0,
		cz * sy * cx + sz * sx, sz * sy * cx - cz * sx, cy * cx, 0.0,
		pos[0], pos[1], pos[2], 1.0,
	]
}

/// A 0.7 x 0.5 x 1.4 box standing on the origin: the player stand-in when Ratchet is unavailable.
fn box_mesh(texture: u32) -> Mesh {
	let mut m = Mesh::new("player box".into());
	for i in 0..8 {
		m.positions.push([if i & 1 != 0 { 0.35 } else { -0.35 }, if i & 2 != 0 { 0.25 } else { -0.25 }, if i & 4 != 0 { 1.4 } else { 0.0 }]);
		m.uvs.push([0.0, 0.0]);
	}
	for q in [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]] {
		m.faces.push((texture, [q[0], q[1], q[2]]));
		m.faces.push((texture, [q[0], q[2], q[3]]));
	}
	m
}

struct HudState {
	health: i32,
	bolts: u32,
	clock: f32,
	/// Bolt counter and nanotech opacity.
	alpha: (f32, f32),
	/// Nanotech fade bytes (f1 bar, f2 orbs).
	nano: (u8, u8),
	/// The ammo HUD: weapon shown, ammo, cap, fades f1 (bar, icon) and f2 (text); and the lock-on
	/// target in NDC.
	ammo: Option<(u8, u16, u16, f32, f32)>,
	reticle: Option<[f32; 2]>,
	/// NOV-14.2 RC range static: alpha and this tick's tile offsets.
	rc_static: Option<(u8, Vec<[u8; 2]>)>,
	/// NOV-14.2b marker prompt widget: ramps (a, b) and the frame counter.
	marker: (u8, u8, u32),
	/// HERO-3a: the breath meter's copied slot state (BRIDGE-14); None when nothing is installed.
	breath: Option<game::breath_hud::BreathSnapshot>,
	/// ARI-15b: Skid's counter widgets, slots 5 / 7: (display, max, f1, f2) while drawn.
	counters: [Option<(i32, i32, u8, u8)>; 2],
	/// WPN-6.2g2 / BRIDGE-39: this frame's screen sprites (FUN_0020fb40), each with its centre in window pixels.
	sprites: Vec<(crate::game::ScreenSprite, [f32; 2])>,
	/// ARI-42c: the Trespasser puzzle HUD strips of this tick.
	pad: Vec<crate::game::PadPrim>,
}

/// Hidden appended vertices (a held model not in hand): all collapsed, so their triangles vanish.
/// BRIDGE-21: append one back object (`n` vertices) through the shared back matrix; collapsed when there is none
/// (`clank_hidden`, no joint). `posed` is its skinned (positions, normals).
fn append_back_object(ps: &mut Vec<[f32; 3]>, ns: &mut Vec<[f32; 3]>, n: usize, posed: (Vec<[f32; 3]>, Vec<[f32; 3]>), bj: Option<&[f32; 12]>) {
	let Some(bj) = bj else { return hide(ps, ns, n) };
	let (mut gp, mut gn) = posed;
	gp.resize(n, [0.0; 3]);
	gn.resize(n, [0.0, 0.0, 1.0]);
	ps.extend(gp.iter().map(|&v| apply(bj, v, 1.0)));
	ns.extend(gn.iter().map(|&v| apply(bj, v, 0.0)));
}

fn hide(ps: &mut Vec<[f32; 3]>, ns: &mut Vec<[f32; 3]>, n: usize) {
	ps.extend(std::iter::repeat([0.0; 3]).take(n));
	ns.extend(std::iter::repeat([0.0, 0.0, 1.0]).take(n));
}

/// Clip-space projection of a world point; None behind the camera.
fn project(m: &[[f32; 4]; 4], p: [f32; 3]) -> Option<[f32; 2]> {
	let c: Vec<f32> = (0..4).map(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r]).collect();
	(c[3] > 0.1).then(|| [c[0] / c[3], c[1] / c[3]])
}

fn sprite(slot: usize, rect: [f32; 4], flip: bool, a: f32) -> Quad {
	Quad { tex: slot, rect, uv: if flip { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] }, colour: [1.0, 1.0, 1.0, a], angle: 0.0, corners: None }
}


/// Skinned model-space positions and normals.
type Posed = (Vec<[f32; 3]>, Vec<[f32; 3]>);

/// Ratchet's sequences by hero state (SetState, decomp/notes/veldin_enemies.md section 5).
const IDLE: usize = 0;
/// The joint that holds the wrench (right wrist), and the wrench's in-hand sequence.
/// Preview key of the posed hero (+ item id for the hero with that item attached): the pause menu's big panel
/// (decomp/notes/ingame_pause_menu.md "Item previews").
const HERO_PREVIEW: u32 = 0x10000;
/// Gold bolt class 1134, shown on the Galactic details page (frontend.rs `gold_preview`).
const GOLD_BOLT_PREVIEW: u32 = 1134;
/// Vertical tan-half-FOV a pause panel of w x h (512x448 units) shows (decomp/notes/ingame_pause_menu.md "Menu
/// camera"): the frozen pause matrix maps 224 px to 0.63 * 0.756 (PAL); FUN_00276c50 renders into a pow2 target
/// (u = 7 up to 128, else the next power; v likewise; v shrinks while u + v > 17) centred on the view, then blits
/// the fit-aspect crop (flag 4) into the panel.
/// Pause aux clip table 0x1b9ad8 by aux index: (class, sequence slot) the clip is installed in (FUN_00284028).
const MENU_AUX: [(u32, usize); 17] = [
	(168, 5), (657, 1), (74, 3), (497, 1), (497, 2), (497, 3), (203, 3), (186, 8), (186, 9), (186, 10), (483, 3), (634, 8), (188, 3),
	(433, 3), (163, 4), (619, 3), (270, 4),
];

/// Hero record 0x1b93e8 (0x30 per item id; decomp/notes/ingame_pause_menu.md "Hero preview records") for the items
/// with extra actors: (item, aux start +0x00, aux count +0x04, actors (class, sequence) from +0x18..+0x2c). Items 8
/// and 0x1e also list class 0 with sequence 0, left out [I].
const MENU_ACTORS: [(u8, usize, usize, &[(u32, usize)]); 6] = [
	(0x0a, 1, 1, &[(657, 1)]),
	(0x11, 2, 1, &[(74, 3)]),
	(0x14, 7, 3, &[(186, 8), (186, 9), (186, 10)]),
	(0x15, 16, 1, &[(270, 4)]),
	(0x19, 6, 1, &[(203, 3)]),
	(0x1f, 10, 2, &[(634, 8)]),
];

/// Level-light slot 14, filled at pause open for the menu mobies (FUN_0028ba40; decomp/notes/lighting_fog.md
/// "Reserved level-light slots 12-14"): colour, wrap -0.25, the direction it travels; slot B zero.
const MENU_LIGHT_SLOT: usize = 14;
const MENU_LIGHT_COLOUR: [f32; 3] = [0.9, 0.9, 0.6];
const MENU_LIGHT_DIR: [f32; 3] = [0.7155, 0.5367, -0.4472];

/// LIT-7a: the owned gold weapon in pause panel 3 is lit by level-light slot 12 (FUN_00206e78 with the frame moby's light
/// as B, decomp/notes/lighting_fog.md "LIT-7a port spec"). Ambient bytes (R, G) = (n * 4 / 10, n * 3 / 10) for n = 3 x 0x20 = 96
/// (integer divisions), blue 0; slot 12 colour A = (s / 2, 0.4 s, 0) with s the sum of slot 14's colour, wrap -0.3, slot 14's direction.
const GOLD_LIGHT_SLOT: usize = 12;
const GOLD_AMBIENT: [f32; 3] = [(96 * 4 / 10) as f32 / 128.0, (96 * 3 / 10) as f32 / 128.0, 0.0];
const GOLD_LIGHT_WRAP: f32 = -0.3;

/// Slot 12's colour A.
fn gold_light_colour() -> [f32; 3] {
	let s = MENU_LIGHT_COLOUR[0] + MENU_LIGHT_COLOUR[1] + MENU_LIGHT_COLOUR[2];
	[s * 0.5, s * 0.4, s * 0.0]
}

/// Whether panel-3 item `id` gets the gold light: a gold gadget that is not a kind 1-3 item (ids 1-7, 0x1c-0x1e) and not 0x18.
fn gold_lit(id: u8, golds: &[u8]) -> bool {
	golds.contains(&id) && !(1..=7).contains(&id) && !(0x1c..=0x1e).contains(&id) && id != 0x18
}

/// Panel-3 item view table 0x1c4488 by item id (decomp/notes/ingame_pause_menu.md "Panel 3 item model"):
/// (distance when flags bit 0, distance otherwise, y, z, rot x, rot y, a, b). The Weapons / Quick Select /
/// Gadgets panel item 0x1b4f28 has flags 0.
const PANEL3_VIEW: [[f32; 8]; 0x24] = {
	const H: f32 = -1.5708;
	[
		[6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
		[3.0, 5.0, 0.0, 0.0, 0.0, 0.2, 0.2, 0.0],
		[3.0, 5.0, 0.0, 0.0, 0.0, 0.2, 0.2, 0.0],
		[3.0, 5.0, 0.0, 0.0, 0.0, 0.2, 0.2, 0.0],
		[3.0, 5.0, 0.0, 0.0, 0.0, 0.2, 0.2, 0.0],
		[2.0, 3.0, 0.0, -0.2, 0.0, 0.0, 0.0, 0.0],
		[2.0, 3.0, 0.0, -0.2, 0.0, 0.0, 0.0, 0.0],
		[2.0, 3.0, 0.0, -0.2, 0.0, 0.0, 0.0, 0.0],
		[6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
		[4.0, 4.0, 0.0, 0.0, 0.0, H, 0.0, 0.5],
		[2.5, 2.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2],
		[5.0, 5.0, 0.0, -0.2, 0.0, H, 0.0, 0.4],
		[2.0, 3.0, 0.0, -0.1, 0.0, 0.0, 0.0, 0.1],
		[4.0, 4.0, 0.0, -0.05, 0.0, H, 0.0, 0.45],
		[6.0, 6.0, 0.0, -0.15, 0.0, H, 0.0, 0.3],
		[5.0, 5.0, 0.0, -0.18, 0.0, H, 0.0, 0.3],
		[6.0, 6.0, 0.0, -0.1, 0.0, H, 0.0, 0.5],
		[2.5, 2.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2],
		[3.0, 3.0, 0.0, -0.05, 0.0, 0.0, 0.0, 0.3],
		[5.0, 5.0, 0.0, -0.1, 0.0, H, 0.0, 0.3],
		[2.5, 2.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2],
		[4.0, 4.0, 0.0, -0.1, 0.0, H, 0.0, 0.3],
		[4.5, 6.0, 0.0, 0.0, 0.0, H, 0.0, 0.75],
		[7.0, 7.0, 0.0, -0.3, 0.0, H, 0.0, 0.3],
		[2.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
		[2.5, 2.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2],
		[3.0, 6.0, 0.0, -0.52, -1.5708, 0.0, 0.0, 0.0],
		[2.0, 5.0, 0.0, 0.45, 1.5708, 0.0, 0.2, 0.0],
		[2.0, 3.0, 0.0, 0.2, 0.0, 0.5, 0.0, 0.0],
		[2.0, 3.0, 0.0, 0.2, 0.0, 0.5, 0.0, 0.0],
		[6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
		[2.5, 5.0, 0.0, -0.25, 0.0, H, 0.0, 0.18],
		[1.2, 2.4, 0.0, -0.1, 0.0, H, -0.05, 0.15],
		[6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
		[6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
		[6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
	]
};

fn menu_tan_half_y(w: f32, h: f32) -> f32 {
	let (w, h) = (w.round().max(1.0) as i32, h.round().max(1.0) as i32);
	let lg = |x: i32| if x <= 128 { 7 } else { (8..31).find(|&n| 1 << n >= x).unwrap_or(30) };
	let (u, mut v) = (lg(w), lg(h));
	while u + v > 17 {
		v -= 1;
	}
	let rh = 1 << v;
	let src_h = if w < h { rh } else { rh - 2 * (rh / 2 - (h * rh) / (2 * w)) };
	src_h as f32 / 2.0 / 224.0 * 0.63 * 0.756
}

/// Pause hero clip table 0x1b93f0 (0x30-byte rows, indexed by item id; decomp/notes/ingame_pause_menu.md "Hero
/// preview clips"): (clip, loops, item seq, clip2). The clip plays `loops` times then clip2 loops; loops 0 loops
/// the clip. Rows 0 and >= 0x24 change nothing.
const MENU_CLIP_ROWS: [(u8, u8, u8, u8); 0x24] = [
	(0, 0, 0, 0), (7, 1, 1, 7), (7, 1, 1, 7), (7, 1, 1, 7), (7, 1, 1, 7), (0x19, 1, 1, 7), (0x1a, 1, 1, 7), (0x15, 1, 1, 7),
	(7, 1, 1, 7), (0x16, 1, 1, 6), (4, 1, 1, 7), (0, 1, 1, 6), (8, 1, 1, 7), (0xf, 1, 4, 7), (0xe, 1, 1, 7), (3, 1, 5, 7),
	(0xa, 1, 1, 6), (5, 0, 1, 7), (2, 0, 1, 7), (1, 1, 1, 7), (0xd, 1, 1, 7), (0x17, 1, 1, 7), (0x12, 1, 1, 6), (9, 1, 1, 6),
	(0x11, 1, 1, 7), (0x10, 1, 1, 7), (0x18, 1, 3, 7), (0x1b, 1, 1, 6), (0xb, 1, 1, 7), (0xc, 1, 1, 7), (6, 1, 1, 6), (0x13, 1, 3, 7),
	(0x14, 1, 1, 7), (6, 1, 1, 6), (6, 1, 1, 6), (6, 1, 1, 6),
];

/// A pause hero preview item: the hero joint it rides, its bind mesh in the joint frame, its class and the view
/// row's mode (0 = pose-copy from the hero, else own itemseq pose with normalised attach columns).
struct HeroItem {
	joint: usize,
	mesh: Mesh,
	class: u32,
	mode: u8,
}

/// Preview key of the worn-item batch (boots, back item, extra items), drawn over the hero panel.
const HERO_WORN: u32 = 0x20000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WornKind {
	/// Boot moby at hero attach point 2 / 3 (feet slot).
	BootA,
	BootB,
	/// The back item (attach point 5).
	Back,
	/// An extra item by id (35 / 33 / 34).
	Extra(u8),
}

/// One candidate moby of the worn batch: its vertices are `range` of the batch mesh (collapsed to a point while the
/// part is not worn).
struct WornPart {
	kind: WornKind,
	class: u32,
	joint: usize,
	range: std::ops::Range<usize>,
	mesh: Mesh,
}

/// The worn batch of a level: every candidate part in batch order, and the hero joints of attach points 0x16 / 0x17
/// whose scale is zeroed while boots are worn.
#[derive(Default)]
struct WornSet {
	parts: Vec<WornPart>,
	total: usize,
	scale_joints: [Option<usize>; 2],
}

/// Hero joint lists the two boot mobies copy their pose from (dat 0x17a650 / 0x17a668).
const BOOT_COPY: [[usize; 5]; 2] = [[99, 100, 101, 102, 103], [93, 94, 95, 96, 97]];

/// Hero record 0x1b93e8 (+0x00, +0x04): the aux clip range (start, count) in `MENU_AUX` per item id.
const ITEM_AUX: [(u8, u8); 37] = [
	(0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (1, 1), (0, 0), (0, 0), (14, 1), (0, 0), (0, 1), (0, 0), (2, 1), (0, 0), (0, 0), (7, 3), (16, 1), (0, 0), (0, 0), (0, 0), (6, 1), (12, 1), (0, 0), (0, 0), (0, 0), (0, 0), (10, 2), (15, 1), (0, 0), (0, 0), (0, 0), (0, 0),
];

/// The pause hero preview's clock: a new item's clip fades in over 10 ticks from the pose at the switch
/// (FUN_002575d8(hero, clip, 0, 10, 5)); the clip plays `loops` times (0 = forever) then clip2 fades in from its end.
/// Stepped at 50 Hz from the time the item was selected.
struct MenuHero {
	id: u8,
	pa: anim::PropAnim,
	/// The item's own clock (hand or head moby, BRIDGE-19): its requested itemseq, then sequence 1 once it ends.
	item: Option<anim::PropAnim>,
	start: f32,
	ticks: u32,
	plays: u32,
	second: bool,
}

/// (clip sequence, loops, clip2 sequence) of item `id`'s row; `base` = seq index of menu clip 0.
fn menu_clip(base: usize, id: u8) -> Option<(usize, u8, usize)> {
	MENU_CLIP_ROWS.get(id as usize).filter(|_| id != 0).map(|&(clip, loops, _, clip2)| (base + clip as usize, loops, base + clip2 as usize))
}
const WRIST: usize = 56;
const WRENCH_HELD: usize = 1;
/// The Bomb Glove model, the hero joint it rides (54, not the wrench's wrist 56), its draw sequence
/// (0, then 1 idle) and the glove joint holding the bomb with the bomb's offset in it.
const GLOVE_CLASS: u32 = 192;
const GLOVE_JOINT: usize = 54;
const GLOVE_BOMB_JOINT: usize = 2;
const GLOVE_BOMB_OFFSET: [f32; 3] = [0.0, -0.0856, -0.0199];



/// Animated props pose by (class, clock state), so props in step share one skinning (props fading from a snapshot are
/// posed on their own, handler.rs).
type PoseKey = (u32, usize, u8, i32, i32, u32);
fn pose_key(p: &Prop) -> PoseKey {
	(p.class_id, p.anim.seq, p.anim.clock.old_seq, p.anim.clock.cur, p.anim.clock.next, p.anim.clock.p.to_bits())
}

/// The game's HUD art as overlay slots: (slot, width, height) per frame.
#[derive(Default)]
struct HudArt {
	bolt: Vec<(usize, f32, f32)>,
	/// The breath meter's sheet 0x753f frames 0..2 (aquarium, low-air aquarium, bubble).
	breath: Vec<(usize, f32, f32)>,
	/// The Morph-o-Ray meter's sheet 0x7533 frames 0..1 (tank, bubble), WPN-6.7e4 (decomp/notes/weapons_framework.md).
	morph: Vec<(usize, f32, f32)>,
	bar: Vec<(usize, f32, f32)>,
	orb: Vec<(usize, f32, f32)>,
	/// The Bomb Glove's item icon (0xea60 + item 10) [I].
	glove: Vec<(usize, f32, f32)>,
	/// Ammo HUD weapon icons: frame 3 of 0xea60+id (hud.md "Ammo HUD").
	weapon_icons: HashMap<u8, (usize, f32, f32)>,
	/// The memory-card save icon 0x755d (decomp/notes/ingame_save_quit.md s6).
	save: Vec<(usize, f32, f32)>,
	/// Sound page volume fill 0xe99e frames 8/9, Quick Select backdrop 0xe934 frame 0 and highlight
	/// 0xe99c frame 0 (decomp/notes/ingame_menu_leftovers.md P1-d, ingame_other_screens.md s2).
	vol: [Option<(usize, f32, f32)>; 2],
	/// Menu panel art: underlay 0xe99e frame 7, fx 0x1a, 0x1c, 0x19 (frontend::Art::frame).
	frame: Option<[(usize, f32, f32); 4]>,
	/// Map hero marker 0xe99a frame 5 (decomp/notes/ingame_other_screens.md s1).
	marker: Option<(usize, f32, f32)>,
	/// ARI-15b: Skid's counter icons, HUD sheets 2000 (sharks) / 2001 (nests) frame 0.
	counters: [Option<(usize, f32, f32)>; 2],
	wheel_bg: Option<(usize, f32, f32)>,
	wheel_sel: Option<(usize, f32, f32)>,
	/// HelpDesk icon: fx texture record 4 (help.rs).
	help_icon: Option<usize>,
	/// NOV-14.2 RC static: fx texture 0x1e (slot, width, height).
	static_tex: Option<(usize, f32, f32)>,
	/// WPN-6.2g2: the screen-sprite fx textures 0x21..=0x27 (slot, width, height).
	screen_fx: [Option<(usize, f32, f32)>; 7],
	/// ARI-42c: the Trespasser puzzle HUD fx textures 7, 16, 104, 105 (slot, width, height).
	pad_fx: [Option<(usize, f32, f32)>; 4],
	/// Pause-menu pictures: help_ss / options_ss, the L1/R1 arrow, the map globe (frontend::Pics).
	pics: frontend::Pics,
	/// Item icons 0xea60 + id, for the Quick Select wheel [I] and the shop's weapon rows.
	items: HashMap<u8, (usize, f32, f32)>,
	/// Frame 2 of 0xea60 + id: the shop's ammo-row icon (FUN_002b1540).
	ammo_items: HashMap<u8, (usize, f32, f32)>,
	/// HUD sprite 0xe935 frame 0: the shop's LED ticker sheet (FUN_002b1168).
	led: Option<(usize, f32, f32)>,
	font: Option<(usize, Vec<art::Glyph>)>,
	/// Level font 3 (the marker prompt's "!", NOV-14.2b).
	font3: Option<(usize, Vec<art::Glyph>)>,
	/// Level font 2 (the shop panels' text, table boot 0x1df370).
	font2: Option<(usize, Vec<art::Glyph>)>,
}


/// The level's decoded samples and which one the game plays for each event (sound::events, read
/// from the game's play calls and animation triggers; decomp/notes/sound.md "Event table").
#[derive(Default)]
struct Sfx {
	clips: Vec<Option<audio::Clip>>,
	ev: sound::Events,
	/// False until the ambient loop's tone format is decoded.
	ambient_ok: bool,
}

/// Mix level of the music: the settings default 0x2cc of the 0x400 full scale (music.md; the
/// Settings gain scales it from there).
const MUSIC_VOLUME: f32 = 0x2cc as f32 / 1024.0;

/// A cue as (bank sound, volume) for the mixer.
fn cue(c: Option<&sound::Cue>) -> (Option<usize>, f32) {
	c.map_or((None, 0.0), |c| (Some(c.sound), c.volume))
}

fn load_sfx(l: &Level) -> Sfx {
	let clips = match sound::level_sounds(l) {
		Ok(samples) => samples
			.into_iter()
			.map(|s| (!s.pcm.is_empty()).then(|| audio::Clip { rate: s.rate, pcm: Arc::new(s.pcm), loop_start: s.loop_start, reverb: s.reverb }))
			.collect(),
		Err(e) => {
			eprintln!("sounds: {e}");
			return Sfx::default();
		}
	};
	let ev = sound::events(l).unwrap_or_else(|e| {
		eprintln!("sound events: {e}");
		sound::Events::default()
	});
	Sfx { clips, ev, ambient_ok: true }
}

/// The resident global bank (front-end and menu sounds), without level events.
fn global_sfx(disc: &mut iso::Disc) -> Sfx {
	match sound::global_sounds(disc) {
		Ok(samples) => Sfx {
			clips: samples.into_iter().map(|s| (!s.pcm.is_empty()).then(|| audio::Clip { rate: s.rate, pcm: Arc::new(s.pcm), loop_start: s.loop_start, reverb: s.reverb })).collect(),
			ev: sound::Events::default(),
			ambient_ok: true,
		},
		Err(e) => {
			eprintln!("global sounds: {e}");
			Sfx::default()
		}
	}
}

/// RNG-4b: which voice a request-point draw belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum VoiceKey {
	/// A wanted loop (`loop_want` owner).
	Loop(usize),
	/// An animation cue a prop's clock fired: (prop index, ordinal of the cue within the frame's list).
	EnemyCue(usize, usize),
	/// The looping sound of a prop's sequence.
	EnemyLoop(usize),
	Suck,
	Pyro,
}

/// RNG-4b / BRIDGE-24: the def's pitch draw of a voice is made when the voice is requested (in the tick, FUN_002a0cd0)
/// and kept in `pending` until the post-loop drain starts it; `rand` keeps the draw of a running voice for its updates.
#[derive(Default)]
struct VoiceBook {
	pending: HashMap<VoiceKey, i32>,
	rand: HashMap<VoiceKey, i32>,
}

impl VoiceBook {
	/// The draw a starting voice uses: the one made at its request point, else (no tick ran) drawn now.
	fn start(&mut self, key: VoiceKey, c: &sound::Cue) -> i32 {
		let r = self.pending.remove(&key).unwrap_or_else(|| c.random_shift(|n| particles::Rng::shared().small(n)));
		self.rand.insert(key, r);
		r
	}

	/// The draw of the running voice (updates draw nothing).
	fn current(&self, key: VoiceKey) -> i32 {
		self.rand.get(&key).copied().unwrap_or(0)
	}
}

/// Animation-driven sound state of one enemy prop: sequence and the loop voice (id, class sound).
struct EnemySfx {
	seq: usize,
	voice: Option<(u64, usize)>,
}

/// Retail start gate (FUN_002a0cd0) and out-of-range loop stop (FUN_0029ff68): a voice is audible while its
/// def's distance gain is at least 0x20 (decomp/notes/sound.md AUD-11h).
fn audible(c: &sound::Cue, dist: f32) -> bool {
	c.fall.level(dist) != 0
}

/// Position of a voice owned by a moby: its position raised one unit (OV:101080 / OV:100699).
fn voice_at(pos: [f32; 3]) -> [f32; 3] {
	[pos[0], pos[1], pos[2] + 1.0]
}

/// Distance from the listener (the camera) to `at`.
fn listener_dist(cam: &Camera, at: [f32; 3]) -> f32 {
	let d = [at[0] - cam.pos[0], at[1] - cam.pos[1], at[2] - cam.pos[2]];
	(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// Volume scale (relative to the cue's own volume), pan and pitch shift (1/128 semitones) of a sound at `at`
/// heard from the camera: the def's retail falloff (FUN_0029fc10 via `Cue::level`), the retail pan curve
/// (FUN_0029fd48), Doppler, underwater and occlusion effects and the def's random pitch (sound.md "Voice
/// update", FUN_0029ff68 / FUN_002a0cd0).
fn spatial(cam: &Camera, at: [f32; 3], cue: Option<&sound::Cue>, ln: &sound::ListenerVelocity) -> (f32, f32, i32) {
	spatial_for(cam, at, cue, ln, None)
}

/// `spatial` for a voice owned by prop `owner`, whose own solid its occlusion line skips (FUN_0029fb70).
fn spatial_for(cam: &Camera, at: [f32; 3], cue: Option<&sound::Cue>, ln: &sound::ListenerVelocity, owner: Option<usize>) -> (f32, f32, i32) {
	spatial_rand(cam, at, cue, ln, owner, None)
}

/// `spatial_for` with the def's random pitch already drawn at the request point (BRIDGE-24, `Game.sound_requests`);
/// `None` draws it here.
fn spatial_rand(cam: &Camera, at: [f32; 3], cue: Option<&sound::Cue>, ln: &sound::ListenerVelocity, owner: Option<usize>, drawn: Option<i32>) -> (f32, f32, i32) {
	let d = [at[0] - cam.pos[0], at[1] - cam.pos[1], at[2] - cam.pos[2]];
	let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
	let right = [cam.yaw.sin(), -cam.yaw.cos()];
	let fwd = [cam.yaw.cos(), cam.yaw.sin()];
	let pan = sound::pan_of(d[0] * right[0] + d[1] * right[1], d[0] * fwd[0] + d[1] * fwd[1]);
	// Only the pan is wanted of a def-less call (ambience emitters, which do their own gain).
	let Some(c) = cue else { return (1.0, pan, 0) };
	let scale = if c.volume > 0.0 { c.level(l) / c.volume } else { 0.0 };
	let (vf, shift) = ln.effects(cam.pos, at, [0.0; 3], c.fall.flags);
	// Start-time occlusion test (six probes, def flag 2 exempts) when the def attenuates by distance.
	let occ = if scale > 0.0 { ln.occlusion(None, cam.pos, at, c.fall.flags, c.looped, owner) } else { 1.0 };
	let random = drawn.unwrap_or_else(|| c.random_shift(|n| particles::Rng::shared().small(n)));
	(scale * vf * occ, pan, shift + random)
}

/// An event-site one-shot that is re-evaluated every frame with the live listener (gain, pan, Doppler and
/// the occlusion history; FUN_0029ff68 re-runs every voice each update), decomp/notes/sound.md.
struct LiveShot {
	id: u64,
	at: [f32; 3],
	cue: sound::Cue,
	owner: Option<usize>,
	/// The def's random pitch drawn at the start (1/128 semitones).
	random: i32,
	/// ARI-27b: a flags-0x10 voice's fixed level (1/1024): no falloff, no occlusion.
	level: Option<i32>,
}

/// Per-frame gain, pan and pitch shift of a running one-shot (`random` is added to the Doppler shift).
fn live_shot(cam: &Camera, s: &LiveShot, at: [f32; 3], ln: &sound::ListenerVelocity) -> (f32, f32, i32) {
	let d = [at[0] - cam.pos[0], at[1] - cam.pos[1], at[2] - cam.pos[2]];
	let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
	let right = [cam.yaw.sin(), -cam.yaw.cos()];
	let fwd = [cam.yaw.cos(), cam.yaw.sin()];
	let pan = sound::pan_of(d[0] * right[0] + d[1] * right[1], d[0] * fwd[0] + d[1] * fwd[1]);
	let c = &s.cue;
	let (vf, shift) = ln.effects(cam.pos, at, [0.0; 3], c.fall.flags);
	if let Some(level) = s.level {
		return (level as f32 / 1024.0 * vf, pan, shift + s.random);
	}
	let scale = if c.volume > 0.0 { c.level(l) / c.volume } else { 0.0 };
	let occ = if scale > 0.0 { ln.occlusion(Some(s.id as u32), cam.pos, at, c.fall.flags, false, s.owner) } else { 1.0 };
	(c.volume * scale * vf * occ, pan, shift + s.random)
}

/// 3x4 row-major transform of a point (`t` = 1) or a direction (`t` = 0).
/// Area-weighted vertex normals (the same sum `Gpu::batch` uses).
/// A decoded moby mesh gives its stored vertex normals instead (`Mesh::vertex_normals`).
fn face_normals(m: &Mesh) -> Vec<[f32; 3]> {
	m.vertex_normals()
}

/// Normalises each rotation column `(m[i], m[4+i], m[8+i])` to unit length (HeroItemsAttach basis fix, HERO-7.1).
fn norm_cols(mut m: [f32; 12]) -> [f32; 12] {
	for i in 0..3 {
		let l = (m[i] * m[i] + m[4 + i] * m[4 + i] + m[8 + i] * m[8 + i]).sqrt();
		if l != 0.0 && l.is_finite() {
			m[i] /= l;
			m[4 + i] /= l;
			m[8 + i] /= l;
		}
	}
	m
}


/// The attach joint matrix with its three axis columns scaled to unit length (view rows with mode != 0).
fn normalise_columns(mut m: [f32; 12]) -> [f32; 12] {
	for c in 0..3 {
		let l = (m[c] * m[c] + m[4 + c] * m[4 + c] + m[8 + c] * m[8 + c]).sqrt();
		if l > 1e-6 {
			for r in 0..3 {
				m[4 * r + c] /= l;
			}
		}
	}
	m
}

fn apply(m: &[f32; 12], v: [f32; 3], t: f32) -> [f32; 3] {
	[
		m[0] * v[0] + m[1] * v[1] + m[2] * v[2] + m[3] * t,
		m[4] * v[0] + m[5] * v[1] + m[6] * v[2] + m[7] * t,
		m[8] * v[0] + m[9] * v[1] + m[10] * v[2] + m[11] * t,
	]
}

fn gpu_has_preview(scene: &Option<Scene>, key: u32) -> bool {
	scene.as_ref().is_some_and(|s| s.previews.contains_key(&key))
}

fn length2(v: [f32; 3]) -> f32 {
	(v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// Model-space box of a mesh.
fn bounds(m: &Mesh) -> ([f32; 3], [f32; 3]) {
	let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
	for p in &m.positions {
		for j in 0..3 {
			lo[j] = lo[j].min(p[j]);
			hi[j] = hi[j].max(p[j]);
		}
	}
	if m.positions.is_empty() {
		return ([-0.5; 3], [0.5; 3]);
	}
	(lo, hi)
}

fn forward(cam: &Camera) -> [f32; 3] {
	[cam.yaw.cos() * cam.pitch.cos(), cam.yaw.sin() * cam.pitch.cos(), cam.pitch.sin()]
}

/// Column-major perspective * look-at, Z up, depth 0..1. The world passes (RETAIL_NEAR, RETAIL_FAR); the vertical
/// tan is `cam.tan_half` and the horizontal follows the window aspect [PC] (REN-7a).
fn view_proj(cam: &Camera, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
	let f = forward(cam);
	let up = cam.up;
	let norm = |v: [f32; 3]| {
		let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
		[v[0] / l, v[1] / l, v[2] / l]
	};
	let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
	let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
	let s = norm(cross(f, up));
	let u = cross(s, f);
	let e = cam.pos;
	// View matrix rows: s, u, -f.
	let view = [
		[s[0], u[0], -f[0], 0.0],
		[s[1], u[1], -f[1], 0.0],
		[s[2], u[2], -f[2], 0.0],
		[-dot(s, e), -dot(u, e), dot(f, e), 1.0],
	];
	let t = 1.0 / cam.tan_half;
	let proj = [
		[t / aspect, 0.0, 0.0, 0.0],
		[0.0, t, 0.0, 0.0],
		[0.0, 0.0, far / (near - far), -1.0],
		[0.0, 0.0, near * far / (near - far), 0.0],
	];
	let mut out = [[0f32; 4]; 4];
	for c in 0..4 {
		for r in 0..4 {
			out[c][r] = (0..4).map(|k| proj[k][r] * view[c][k]).sum();
		}
	}
	out
}



/// SCN-3: the scene actors' light, captured once when a scene starts (the hero's at that instant) and held until it ends.
fn scene_light_snapshot(held: &mut Option<[f32; 4]>, active: bool, now: impl FnOnce() -> [f32; 4]) -> Option<[f32; 4]> {
	if !active {
		*held = None;
		return None;
	}
	Some(*held.get_or_insert_with(now))
}

#[cfg(test)]
mod scene_light_tests {
	use super::*;

	/// HERO-AFTERIMAGE-15 test 4: the two afterimage alpha bytes encode as the spec's fractions and flag translucent.
	#[test]
	fn afterimage_alpha_encoding() {
		assert_eq!(moby_alpha_encoding(0.0, 0x30), (0.15625, true));
		assert_eq!(moby_alpha_encoding(0.0, 0x17), (0.20507812, true));
	}

	/// LIT-7a tests 1-3: ambient bytes (38, 28, 0), slot 12 colour (1.2, 0.96, 0) wrap -0.3, the item gate.
	#[test]
	fn gold_light_constants_and_gate() {
		assert_eq!(GOLD_AMBIENT, [0.296875, 0.21875, 0.0]);
		assert_eq!(((96 * 4 / 10) as u32, (96 * 3 / 10) as u32), (38, 28));
		assert_eq!(gold_light_colour(), [1.2f32, 0.96000004, 0.0]);
		assert_eq!(GOLD_LIGHT_WRAP, -0.3);
		assert_eq!(Gpu::pack_light(GOLD_AMBIENT, GOLD_LIGHT_SLOT, GOLD_LIGHT_SLOT, 0.0)[3], 204.0);
		assert!(gold_lit(0x10, &[0x10, 3]));
		assert!(!gold_lit(0x18, &[0x18]));
		assert!(!gold_lit(3, &[3]));
		assert!(!gold_lit(0x10, &[]));
	}

	#[test]
	fn snapshot_is_taken_once_per_scene() {
		let mut held = None;
		assert_eq!(scene_light_snapshot(&mut held, true, || [1.0, 2.0, 3.0, 4.0]), Some([1.0, 2.0, 3.0, 4.0]));
		assert_eq!(scene_light_snapshot(&mut held, true, || [9.0; 4]), Some([1.0, 2.0, 3.0, 4.0]));
		scene_light_snapshot(&mut held, false, || [0.0; 4]);
		assert_eq!(scene_light_snapshot(&mut held, true, || [5.0; 4]), Some([5.0; 4]));
	}
}

#[cfg(test)]
mod attach_tests {
	use super::*;
	#[test]
	fn norm_cols_normalises_columns_not_rows() {
		let m = norm_cols([2.0, 3.0, 0.0, 0.0, 0.0, 4.0, 0.0, 5.0, 0.0, 0.0, 1.0, 6.0]);
		let e = [1.0, 0.6, 0.0, 0.0, 0.0, 0.8, 0.0, 5.0, 0.0, 0.0, 1.0, 6.0];
		for i in 0..12 {
			assert!((m[i] - e[i]).abs() < 1e-6, "{i}: {m:?}");
		}
	}
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
		let m = naga::front::wgsl::parse_str(super::SHADER).expect("wgsl parse");
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	}

	/// NOV-35b: metal alpha min(((byte << 10) - trunc(z - w)) >> 7, 0x80), skipped at <= 0, and the per-class metal byte.
	#[test]
	fn metal_alpha_and_byte() {
		use super::{metal_alpha, metal_byte};
		assert_eq!(metal_alpha(0x18, 20000.0, 1000.0), Some(43));
		assert_eq!(metal_alpha(0x18, 5000.0, 1000.0), Some(128));
		assert_eq!(metal_alpha(0x18, 100.0, 1000.0), Some(128));
		assert_eq!(metal_alpha(0x14, 20000.0, 0.0), Some(3));
		assert_eq!(metal_alpha(0x14, 20480.0, 0.0), None);
		assert_eq!(metal_alpha(0x14, 21000.0, 500.0), None);
		assert_eq!((metal_byte(13), metal_byte(1143), metal_byte(601)), (0x14, 0x20, 0x18));
	}
}

#[cfg(test)]
mod proj_tests {
	use super::*;

	fn clip(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 4] {
		std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
	}

	// REN-7a tests 1, 2, 4, 5 (camera at the origin looking +X, so world x is depth, y is -right, z is up).
	#[test]
	fn retail_projection() {
		let cam = Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: TAN_HALF_FOV, up: [0.0, 0.0, 1.0] };
		assert!((1.0 / (0.63f32 * 0.756f32) - 2.0996053).abs() < 1e-6);
		assert_eq!(TAN_HALF_FOV, 0.47627997);
		for (aspect, p00) in [(16.0 / 9.0, 1.181028f32), (512.0 / 448.0, 1.8371546)] {
			let m = view_proj(&cam, aspect, RETAIL_NEAR, RETAIL_FAR);
			assert!((m[1][0].abs() - p00).abs() < 1e-6, "{aspect}");
			assert!((m[0][2] - 1.0000429).abs() < 1e-6);
			assert!((m[3][2] + 0.03125134).abs() < 1e-7);
			let ndc_y = |vz: f32| {
				let c = clip(&m, [vz, 0.0, TAN_HALF_FOV * vz]);
				c[1] / c[3]
			};
			assert!((ndc_y(1.0) - 1.0).abs() < 1e-6 && (ndc_y(37.0) - 1.0).abs() < 1e-6);
		}
		let m = view_proj(&cam, 512.0 / 448.0, RETAIL_NEAR, RETAIL_FAR);
		let c = clip(&m, [1.0, -0.63, 0.0]);
		assert!((c[0] / c[3] - 1.1574).abs() < 1e-4);
		let depth = |vz: f32| {
			let c = clip(&m, [vz, 0.0, 0.0]);
			c[2] / c[3]
		};
		assert!((depth(1.0) - 0.96879154).abs() < 1e-6);
		assert!((depth(10.0) - 0.9969178).abs() < 1e-6);
		assert!(depth(RETAIL_NEAR).abs() < 1e-6);
		assert!((depth(RETAIL_FAR) - 1.0).abs() < 1e-6);
		// REN-7a test 3: the same depth from the retail GS z mapping.
		let zs = |raw: f32| -224.0f32 + 536860160.0 / raw;
		assert_eq!(zs(32.0), 16776656.0);
		assert!(((16776656.0 - zs(1024.0)) / (16776656.0 - 496.0) - 0.96879154f32).abs() < 1e-6);
	}

	// REN-7b test 2: the scene key scalar converts to the vertical tan.
	#[test]
	fn proj_scalar_sources() {
		assert_eq!(crate::camera::PROJ_SCALAR.to_bits(), 0x3f2147ae);
		assert!((0.597f32 * PROJ_Y_RATIO - 0.451332).abs() < 1e-6);
	}
}

#[cfg(test)]
mod audible_tests {
	use super::*;

	// AUD-11h tests 1 and 2: the start gate edge and the owner voice offset.
	#[test]
	fn start_gate_and_voice_at() {
		let c = sound::Cue { fall: sound::Falloff { min_range: 0.0, max_range: 100.0, min_vol: 0, max_vol: 1228, min_pitch: 0, max_pitch: 0, flags: 0 }, sound: 0, volume: 1.0, looped: true, bend: (0, 0) };
		assert_eq!(c.fall.level(97.0), 36);
		assert!(audible(&c, 97.0));
		assert_eq!(c.fall.level(97.39), 32);
		assert!(audible(&c, 97.39));
		assert!(!audible(&c, 97.4));
		assert!(!audible(&c, 98.0));
		assert_eq!(c.fall.level(50.0), 614);
		assert!(audible(&c, 50.0));
		assert_eq!(voice_at([1.0, 2.0, 3.0]), [1.0, 2.0, 4.0]);
	}
}

#[cfg(test)]
mod moby_matrix_tests {
	use super::*;

	#[test]
	fn moby_matrix_scales_rows() {
		let at = scale_rows(place([1.0, 2.0, 3.0], [0.0, 0.0, 0.0]), 2.0);
		assert_eq!((at[0], at[5], at[10]), (2.0, 2.0, 2.0));
		assert_eq!(&at[12..], &[1.0, 2.0, 3.0, 1.0]);
		assert_eq!(at.iter().filter(|v| **v == 0.0).count(), 9);
		let base = place([1.0, 2.0, 3.0], [0.3, 0.2, 0.1]);
		assert_eq!(scale_rows(base, 1.0), base);
	}

	#[test]
	fn mirrored_placement_reflects_model_y_only() {
		let (pos, rot) = ([1.0, 2.0, 3.0], [0.3, 0.2, 0.1]);
		let (a, m) = (place(pos, rot), place_mirrored(pos, rot, true));
		assert_eq!(place_mirrored(pos, rot, false), a);
		assert_eq!((&m[0..4], &m[8..16]), (&a[0..4], &a[8..16]));
		assert_eq!(m[4..8], a[4..8].iter().map(|v| -v).collect::<Vec<_>>()[..]);
		// Left-handed: column 0 x column 1 . column 2 < 0 (a plain placement is right-handed).
		let det = |m: &[f32; 16]| m[0] * (m[5] * m[10] - m[6] * m[9]) - m[1] * (m[4] * m[10] - m[6] * m[8]) + m[2] * (m[4] * m[9] - m[5] * m[8]);
		assert!(det(&a) > 0.99 && det(&m) < -0.99);
	}
}

#[cfg(test)]
mod tint_tests {
	use super::*;

	#[test]
	fn tint_override_starts_at_colour_start() {
		let t = Some(([0.5, 0.5, 0.5], 23));
		assert_eq!(tint_override([1.0, 1.0, 1.0, 0.25], 22, t, 0x80), [1.0, 1.0, 1.0, 0.25]);
		assert_eq!(tint_override([1.0, 1.0, 1.0, 0.25], 23, t, 0x80), [0.5, 0.5, 0.5, -2.0]);
		assert_eq!(tint_override([1.0, 1.0, 1.0, 0.25], 99, None, 0x80), [1.0, 1.0, 1.0, 0.25]);
		// CLASS-tint-alpha test 3: a faded prop (alpha 64) carries its alpha in the tail code.
		assert_eq!(tint_override([1.0; 4], 23, t, 64)[3], -2.125);
	}

	// MOBY-FADE tests 1-6.
	#[test]
	fn moby_fade_distance_terms() {
		assert_eq!(moby_fade(64, 50.0 * 1024.0), Some(112));
		assert_eq!(moby_fade(64, 48.0 * 1024.0), Some(128));
		assert_eq!(moby_fade(64, 64.001 * 1024.0), None);
		assert_eq!(moby_fade(600, 490.0 * 1024.0), Some(80));
		assert_eq!(moby_fade(64, 63.95 * 1024.0), Some(0));
		assert_eq!((moby_draw_alpha(112, 0x80), moby_draw_alpha(64, 64)), (112, 32));
	}

	// SCH-6b: the deferred-list fade uses the unclamped distance and the radius (integers).
	#[test]
	fn moby_fade_alt_uses_radius_and_unclamped_distance() {
		assert_eq!(moby_fade_alt(64, 50.0 * 1024.0, 1024.0), 120);
		assert_eq!(moby_fade_alt(64, 0.0, 0.0), 0x80);
		// L = 600 is not clamped to 500: (600 << 10) - 490 * 1024 = 110 * 1024 -> capped.
		assert_eq!(moby_fade_alt(600, 490.0 * 1024.0, 0.0), 0x80);
		assert_eq!(moby_fade_alt(64, 63.0 * 1024.0, 0.0), 8);
	}

	// SCH-6b: effect mobys and debris pieces (draw 255) take the moby draw-distance verdict, centre pos * 1024.
	#[test]
	fn effect_fade_culls_past_255_metres() {
		let view = Some(([0.0; 3], [1.0, 0.0, 0.0]));
		assert_eq!(effect_fade(view, Some(255), [10.0, 0.0, 0.0]), Some(0x80));
		assert_eq!(effect_fade(view, Some(255), [256.0, 0.0, 0.0]), None);
		assert_eq!(effect_fade(view, Some(255), [254.5, 0.0, 0.0]), Some(4));
		assert_eq!(effect_fade(view, None, [900.0, 0.0, 0.0]), Some(0x80));
		assert_eq!(effect_fade(None, Some(255), [900.0, 0.0, 0.0]), Some(0x80));
	}

	// CLASS-tint-alpha tests 1 and 2.
	#[test]
	fn baked_alpha_encoding_round_trips() {
		for (a, e) in [(0x80u8, -2.0f32), (96, -2.0625), (64, -2.125), (0, -2.25)] {
			assert_eq!(moby_baked_encoding(a), e);
			assert_eq!(128.0 - ((e - (e + 0.5).floor()).abs() * 512.0).round(), a as f32);
		}
		let lit = 3.0625f32;
		assert_eq!(128.0 - ((lit - (lit + 0.5).floor()).abs() * 512.0).round(), 96.0);
	}
}

#[cfg(test)]
mod item_panel_tests {
	use super::*;

	fn proj() -> [[f32; 4]; 4] {
		let cam = Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: TAN_HALF_FOV * PANEL_TAN_SCALE, up: [0.0, 0.0, 1.0] };
		view_proj(&cam, PANEL_ASPECT, RETAIL_NEAR, RETAIL_FAR)
	}

	fn target_px(p: [f32; 4]) -> (f32, f32) {
		let m = proj();
		let c: Vec<f32> = (0..4).map(|r| (0..4).map(|k| m[k][r] * p[k]).sum()).collect();
		let (nx, ny) = (c[0] / c[3], c[1] / c[3]);
		let (w, h) = (ITEM_PANEL_SIZE.0 as f32, ITEM_PANEL_SIZE.1 as f32);
		((nx + 1.0) * 0.5 * w, (1.0 - ny) * 0.5 * h)
	}

	#[test]
	fn item_panel_projection_constants() {
		let m = proj();
		// NOV-30a: main projection on the target, out[column][row]: x row = -y column scaled by 1/0.63,
		// y row = z column scaled by 224/64 / 0.47628.
		assert!((TAN_HALF_FOV * PANEL_TAN_SCALE - 0.13608).abs() < 1e-6);
		assert!((PANEL_ASPECT - 4.6296296).abs() < 1e-5);
		assert!((m[1][0] + 1.5873016).abs() < 1e-5, "{}", m[1][0]);
		assert!((m[2][1] - 7.3486185).abs() < 1e-4, "{}", m[2][1]);
		assert!((m[0][3] - 1.0).abs() < 1e-6);
	}

	#[test]
	fn item_panel_projects_view_point() {
		// View point right 1, up 0.5, depth 4 (camera at origin looking +x: right = -y).
		let (px, py) = target_px([4.0, -1.0, 0.5, 1.0]);
		assert!((px - 357.5873).abs() < 1e-2, "{px}");
		assert!((py - 5.211052).abs() < 1e-2, "{py}");
		// Item 16 pivot (right -3.75, up -0.03, depth 7.2) lands in the left third of the source box.
		let (px, py) = target_px([7.2, 3.75, -0.03, 1.0]);
		assert!((px - 44.35977).abs() < 1e-2, "{px}");
		assert!((py - 65.95963).abs() < 1e-2, "{py}");
		// NOV-30d test 2: the bolt icon pivot (depth 3.8 + 10.2, right -4.25, up -1.5) sits right of the price, low in the box.
		let (px, py) = target_px([14.0, 4.25, -1.5, 1.0]);
		assert!((px - 132.64398).abs() < 1e-2, "{px}");
		assert!((py - 114.39053).abs() < 1e-2, "{py}");
	}

	#[test]
	fn item_poses_split_by_destination() {
		use game::vendor::{ItemDest, ItemPose};
		let pose = |class, dest| ItemPose { class, pos: [0.0; 3], rot: [0.0; 3], dest };
		let (world, panel) = split_item_poses(&[pose(1, ItemDest::World), pose(2, ItemDest::Panel), pose(3, ItemDest::World)]);
		assert_eq!(world.iter().map(|p| p.0).collect::<Vec<_>>(), [1, 3]);
		assert_eq!(panel.iter().map(|p| p.0).collect::<Vec<_>>(), [2]);
	}
}

#[cfg(test)]
mod back_object_tests {
	use super::*;

	#[test]
	fn back_object_uses_shared_matrix_and_hides() {
		let bj = norm_cols([2.0, 0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 2.0, 0.0, 0.0, 2.0, 3.0]);
		let posed = (vec![[1.0, 2.0, 3.0]], vec![[0.0, 0.0, 1.0]]);
		let (mut ps, mut ns) = (Vec::new(), Vec::new());
		append_back_object(&mut ps, &mut ns, 1, posed.clone(), Some(&bj));
		assert_eq!(ps[0], apply(&bj, [1.0, 2.0, 3.0], 1.0));
		append_back_object(&mut ps, &mut ns, 1, posed, None);
		assert_eq!(ps[1], [0.0; 3]);
		assert_eq!(ps.len(), 2);
	}

	#[test]
	fn clank_light_glow_constants_and_hide() {
		assert_eq!(CLANK_LIGHT_GLOW, (0.057, 0.08, 0x280000c0));
		assert_eq!(CLANK_LIGHT_GLOW_Z, 0.015);
		// Hidden (no back matrix): the light range collapses like the back objects.
		let (mut ps, mut ns) = (Vec::new(), Vec::new());
		append_back_object(&mut ps, &mut ns, 3, (vec![[1.0; 3]; 3], vec![[0.0; 3]; 3]), None);
		assert!(ps.iter().all(|p| *p == [0.0; 3]) && ps.len() == 3);
	}

	#[test]
	fn back_pulse_is_not_clamped() {
		let m = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
		let o = crate::player::pulse_item(m, 1.3, false, [0.0; 3]);
		assert!((o[0] - 1.3).abs() < 1e-6 && (o[5] - 1.3).abs() < 1e-6);
		assert_eq!(crate::player::pulse_item(m, 1.3, true, [0.0; 3])[0], 1.0);
	}
}
