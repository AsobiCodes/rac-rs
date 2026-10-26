//! Shared by the volumetric passes (fog now, clouds later): the uniform block, the bindings, and the WGSL of the light
//! scattering and the ray march. A medium (fog, a cloud layer) only supplies `fn medium_density(p: vec3<f32>) -> f32` (the
//! extinction per world unit at world point `p`); this file turns it into in-scattered light and transmittance along a view ray:
//! the Henyey-Greenstein phase of the sun, the sun's visibility from the cascaded shadow map, an ambient term, a jittered
//! logarithmic step distribution and the analytic step integral (energy conserving at any step size).

/// The uniform block, as WGSL (`struct F`). Mirrors `Uniform` in volfog.rs field for field.
pub const BLOCK: &str = r#"
struct F {
	cam: array<vec4<f32>, 4>,      // position, forward, right * tan_x, up * tan_y
	prev: array<vec4<f32>, 4>,     // the same of the previous frame (temporal reprojection); w: retail's GS fog unfaded (`gs` in volfog.rs)
	sun_vp: array<mat4x4<f32>, 3>, // sun shadow cascades (viewer/sunshadow.rs)
	sun: vec4<f32>,                // light travel direction, strength
	sun_bias: vec4<f32>,
	sun_texel: vec4<f32>,
	sun_q: vec4<f32>,              // pcf side, map uv scale, cascades, 1 when the shadow map answers
	fog_col: vec4<f32>,            // the level's retail fog colour
	sun_col: vec4<f32>,            // the key light's colour, phase anisotropy g in w
	amb_col: vec4<f32>,            // ambient in-scatter colour
	dens: vec4<f32>,               // extinction at the ground, height scale, ground z, haze floor (fraction left above the layer)
	sheet: vec4<f32>,              // fog sheet (Kerwan): plane z, height scale, extinction, 1 when present
	wind: vec4<f32>,               // wind direction xy, speed, time
	look: vec4<f32>,               // wisp amount, big noise scale (world units per tile), detail scale, sun gain
	size: vec4<f32>,               // fog target w, h, 1/w, 1/h
	march: vec4<f32>,              // steps, far cap, frame index, weight of the current frame
	proj: vec4<f32>,               // tan_x, tan_y, near, far
	lin: vec4<f32>,                // linear depth target w, h, fog divisor, caps a sky ray below the horizon may reach (RAC_TUNE fog_deep)
	phase: vec4<f32>,              // travel direction of the light the phase glow follows: the sun the sky paints (xyz), shafts keep `sun`
	floor_c: vec4<f32>,            // the fog floor's colour (floor.rs `LevelFloor::colour`)
};
@group(0) @binding(0) var<uniform> f: F;
@group(0) @binding(1) var lin: texture_2d<f32>;
@group(0) @binding(2) var noise: texture_3d<f32>;
@group(0) @binding(3) var nsamp: sampler;
@group(0) @binding(4) var sun_map: texture_depth_2d_array;
@group(0) @binding(5) var sun_cmp: sampler_comparison;
@group(0) @binding(6) var cur: texture_2d<f32>;
@group(0) @binding(7) var hist: texture_2d<f32>;
@group(0) @binding(8) var lsamp: sampler;

struct VO { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> VO {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	var o: VO;
	o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
	o.uv = vec2<f32>(x, y);
	return o;
}
"#;

/// The scattering and the march (needs `medium_density` from the medium's snippet, placed before this one).
pub const MARCH: &str = r#"
fn ign(p: vec2<f32>) -> f32 {
	return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// Henyey-Greenstein phase relative to the isotropic one (1 = isotropic); c = cosine between the light's travel direction and the
// direction the light leaves towards the eye.
fn phase_rel(c: f32, g: f32) -> f32 {
	let g2 = g * g;
	return (1.0 - g2) / pow(1.0 + g2 - 2.0 * g * c, 1.5);
}

fn noise_at(p: vec3<f32>) -> vec4<f32> {
	return textureSampleLevel(noise, nsamp, p, 0.0);
}

// Share of the sun that reaches p (1 lit, 0 blocked): the first cascade that holds p answers, one hardware-filtered compare tap.
fn sun_vis(p: vec3<f32>) -> f32 {
	if (f.sun_q.w < 0.5) { return 1.0; }
	let nc = i32(f.sun_q.z);
	for (var k = 0; k < nc; k++) {
		let q = f.sun_vp[k] * vec4<f32>(p - f.sun.xyz * (f.sun_texel[k] * 2.0), 1.0);
		let e = max(abs(q.x), abs(q.y));
		if (e < 0.98 && q.z > 0.0 && q.z < 1.0) {
			let uv = vec2<f32>(q.x * 0.5 + 0.5, 0.5 - q.y * 0.5) * f.sun_q.y;
			var lit = textureSampleCompareLevel(sun_map, sun_cmp, uv, k, q.z - f.sun_bias[k]);
			if (k == nc - 1) { lit = mix(lit, 1.0, smoothstep(0.8, 0.98, e)); }
			return lit;
		}
	}
	return 1.0;
}

// The view ray of fog pixel `pix` (not normalised: its forward part is 1, so the ray parameter is the view depth).
fn view_dir(pix: vec2<f32>) -> vec3<f32> {
	let n = vec2<f32>((pix.x + 0.5) * f.size.z * 2.0 - 1.0, 1.0 - (pix.y + 0.5) * f.size.w * 2.0);
	return f.cam[1].xyz + f.cam[2].xyz * n.x + f.cam[3].xyz * n.y;
}

// The linear view depth fog pixel c ends at: the scene depth at its place, at most the cap (the sky and far geometry end there).
// A sky pixel (no depth written: retail's bottomless canyon, a chasm) whose ray points down ends up to `f.lin.w` caps out, so the
// air below the pool top is deep lit fog there instead of a flat sky sheet; level and upward rays keep the cap. f.lin.w < 1: off.
fn end_depth(c: vec2<i32>) -> f32 {
	let lc = min(c * i32(f.lin.z), vec2<i32>(f.lin.xy) - vec2<i32>(1));
	let z = textureLoad(lin, lc, 0).r;
	let ws = surface_t(c);
	if (ws < z) { return min(ws, f.march.y); }
	if (z >= f.proj.w * 0.98) {
		let d = view_dir(vec2<f32>(c));
		let dz = d.z / length(d);
		let down = smoothstep(0.05, -0.08, dz);
		// Upward sky rays leave the fog layer: a short march, so the sky and the clouds stay clear (no white-out towards the sun).
		let up = smoothstep(0.0, 0.35, dz);
		// With a fog floor (floor.rs) a downward ray ends where it meets the floor's top: below, fs_apply lays the solid floor. A ray
		// that meets it only past the ground map's edge looks past the end of the level's mesh: it keeps the cap like the far sea or
		// ground it borders (the deep march there painted the void a light haze the mesh edge stood out against; user 2026-10-25).
		let fh = floor_hit(d);
		let deep = mix(max(f.lin.w, 1.0), 1.0, fh.y);
		return min(f.march.y * mix(mix(1.0, deep, down), 0.2, up), max(fh.x, f.march.y * 0.2));
	}
	return min(z, f.march.y);
}

// In-scatter colour at a point with sun visibility `vis`, seen along unit view direction `v`.
fn medium_light(vis: f32, v: vec3<f32>) -> vec3<f32> {
	let c = -dot(f.phase.xyz, v);
	// A strong forward lobe (the glow round the sun, the shafts) and a weak back lobe (a soft sheen away from it).
	// Capped, and the light kept below white, so a view towards the sun glows instead of washing out.
	let ph = min(0.8 * phase_rel(c, f.sun_col.w) + 0.2 * phase_rel(c, -0.3), 1.6);
	return min(f.amb_col.rgb + f.sun_col.rgb * (vis * ph * f.look.w), vec3<f32>(0.92));
}

// rgb = light scattered towards the eye, a = transmittance, along the pixel's ray up to the scene.
@fragment fn fs_march(v: VO) -> @location(0) vec4<f32> {
	let c = vec2<i32>(v.pos.xy);
	let t_end = end_depth(c);
	let d = view_dir(v.pos.xy);
	let len = length(d);
	let vdir = d / len;
	let t0 = 0.4;
	var t_acc = 1.0;
	var s_acc = vec3<f32>(0.0);
	if (t_end <= t0) { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
	let n = i32(f.march.x);
	let jit = fract(ign(v.pos.xy) + 0.618034 * f.march.z);
	let ratio = t_end / t0;
	for (var k = 0; k < n; k++) {
		let ta = t0 * pow(ratio, f32(k) / f32(n));
		let tb = t0 * pow(ratio, f32(k + 1) / f32(n));
		let p = f.cam[0].xyz + d * mix(ta, tb, jit);
		let sigma = medium_density(p);
		if (sigma > 1.0e-5) {
			let ext = exp(-sigma * (tb - ta) * len);
			// Deep in the fog and far out the haze takes the floor's colour (floor.rs `deep_k`): no sun shafts there to end in shapes.
			let k = deep_k(p, mix(ta, tb, jit) * len);
			var vis = 0.0;
			if (k < 0.999) { vis = sun_vis(p); }
			let l = mix(medium_light(vis, vdir), floor_col(), k);
			s_acc += t_acc * (1.0 - ext) * l;
			t_acc *= ext;
			if (t_acc < 0.01) { break; }
		}
	}
	return vec4<f32>(s_acc, t_acc);
}
"#;
