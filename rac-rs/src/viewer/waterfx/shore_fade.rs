//! [PC] The geometric sea's waterline (WGSL, appended to `SHADER` in `WaterFx::new`): along a beach, a rock or a pillar the bed
//! lies 0..0.3 under the surface (a shelf), and its half-size depth texel often sat at or above the surface. The clear-shallows
//! test (`zb > zw + 0.02`, nearest texel) then switched off in stripes (a bright cyan dashed rim, retail's flat teal showing
//! between), and the refraction shift, full strength at depth 0, read the bed and the sand at shifted half-size spots (stair
//! steps, speckle at the waterline; Pokitaru, user report 2026-10-24 "choppy, flickery shores"). `fs_water` takes the signed
//! water depth over the bed (`shore_depth_w`, bilinear, 4 taps) and fades the refraction, the bed colour, the surf and the foam
//! in over `SHORE_FADE` units of depth, and reads the shallows through the bilinear bed depth (`bed_lz`, `shallows_k`) so they
//! reach depth 0 at the waterline. The swimming hero's body in the bed depth is no shore (`bed_tap`): no outline round him.
//! The fade is for the geometric sea only (`fs_water` gates `sfade` with `geo`): on other water a whole pool shallower than
//! SHORE_FADE (Novalis' cave floor, puddles) faded to its bed and the water vanished (user report 2026-10-25, runs\nvs_*).
//! The bilinear bed and the depth-0 shallows too (`geo` in `bed_lz`, `shallows_k`): other water keeps the nearest bed read.

pub(super) const SHORE_FADE_WGSL: &str = r#"
// [PC] Shore fade (lane ywmx, 2026-10-24; picked from beach pans, runs\p4_*, shband.py): the water depth (units) over the bed
// where the refraction, the bed colour, the surf and the foam are full; they fade to nothing at the waterline.
const SHORE_FADE: f32 = 0.6;
// Ground this many units ABOVE the surface (a wall, a pillar, the dry bank) switches the clear shallows off, fading from half of
// it; below that the shallows reach out to depth 0 (it was off for any bed texel at or above the surface).
const SHORE_UP: f32 = 0.75;

// The level's depth (view z) in the bed texture at uv, bilinear (the texture is half size; nearest made the waterline stair-step).
fn bed_lz(uv: vec2<f32>, geo: bool) -> f32 {
	let bd = vec2<i32>(textureDimensions(bed_depth));
	if (!geo) { return linear_depth(textureLoad(bed_depth, clamp(vec2<i32>(uv * vec2<f32>(bd)), vec2<i32>(0), bd - vec2<i32>(1)), 0)); }
	let t = uv * vec2<f32>(bd) - 0.5;
	let i0 = vec2<i32>(floor(t));
	let f = t - floor(t);
	let hi = bd - vec2<i32>(1);
	let z00 = linear_depth(textureLoad(bed_depth, clamp(i0, vec2<i32>(0), hi), 0));
	let z10 = linear_depth(textureLoad(bed_depth, clamp(i0 + vec2<i32>(1, 0), vec2<i32>(0), hi), 0));
	let z01 = linear_depth(textureLoad(bed_depth, clamp(i0 + vec2<i32>(0, 1), vec2<i32>(0), hi), 0));
	let z11 = linear_depth(textureLoad(bed_depth, clamp(i0 + vec2<i32>(1, 1), vec2<i32>(0), hi), 0));
	return mix(mix(z00, z10, f.x), mix(z01, z11, f.x), f.y);
}

// Where the clear shallows read the bed: at the refracted uv, or straight through (uvb) when the refracted point lies in front of
// the water (something standing in it).
fn bed_uv(uv: vec2<f32>, uvb: vec2<f32>, zw: f32, geo: bool) -> vec2<f32> {
	return select(uv, uvb, bed_lz(uv, geo) < zw + 0.02);
}

// The share of the clear shallows at bed depth zb under the water pixel pf (zw = its view z): 1 for ground below the surface, 0
// for ground SHORE_UP or more above it.
fn shallows_k(zb: f32, pf: vec3<f32>, zw: f32, geo: bool) -> f32 {
	if (!geo) { return select(0.0, 1.0, zb > zw + 0.02); }
	let hb = (w.eye.z - pf.z) * (1.0 - zb / zw);
	return 1.0 - smoothstep(0.5 * SHORE_UP, SHORE_UP, hb);
}

// The vertical water depth over the bed at view z z (-1..40; 40 = no ground in sight).
fn wl_d(z: f32, pf: vec3<f32>, zw: f32) -> f32 {
	if (z >= w.proj.w * 0.98) { return 40.0; }
	return clamp((w.eye.z - pf.z) * (z / zw - 1.0), -1.0, 40.0);
}

// One texel of the bed depth as a water depth. The swimming hero is in this depth too and is no ground (no outline round him).
fn bed_tap(i: vec2<i32>, pf: vec3<f32>, zw: f32) -> f32 {
	let bd = vec2<i32>(textureDimensions(bed_depth));
	let j = clamp(i, vec2<i32>(0), bd - vec2<i32>(1));
	let z = linear_depth(textureLoad(bed_depth, j, 0));
	if (hero_pixel(vec2<i32>((vec2<f32>(j) + 0.5) / vec2<f32>(bd) / w.size.zw), z, pf.z)) { return 40.0; }
	return wl_d(z, pf, zw);
}

// The signed water depth over the bed at uv (bilinear over the 4 nearest bed texels, each clamped first).
fn shore_depth(uv: vec2<f32>, pf: vec3<f32>, zw: f32) -> f32 {
	let bd = vec2<i32>(textureDimensions(bed_depth));
	let t = uv * vec2<f32>(bd) - 0.5;
	let i0 = vec2<i32>(floor(t));
	let f = t - floor(t);
	let d00 = bed_tap(i0, pf, zw);
	let d10 = bed_tap(i0 + vec2<i32>(1, 0), pf, zw);
	let d01 = bed_tap(i0 + vec2<i32>(0, 1), pf, zw);
	let d11 = bed_tap(i0 + vec2<i32>(1, 1), pf, zw);
	return mix(mix(d00, d10, f.x), mix(d01, d11, f.x), f.y);
}

// The same over a wider footprint (4 taps, 0.75 texel off), so the fade does not follow single texels.
fn shore_depth_w(uv: vec2<f32>, pf: vec3<f32>, zw: f32) -> f32 {
	let o = 0.75 / vec2<f32>(textureDimensions(bed_depth));
	return 0.25 * (shore_depth(uv + vec2<f32>(-o.x, -o.y), pf, zw) + shore_depth(uv + vec2<f32>(o.x, -o.y), pf, zw) + shore_depth(uv + vec2<f32>(-o.x, o.y), pf, zw) + shore_depth(uv + vec2<f32>(o.x, o.y), pf, zw));
}

// The fade at the water pixel pf (uv = its pixel): 0 at the waterline, 1 from SHORE_FADE units of water over the bed.
fn shore_fade(uv: vec2<f32>, pf: vec3<f32>, zw: f32) -> f32 {
	return smoothstep(0.0, SHORE_FADE, shore_depth_w(uv, pf, zw));
}

// The colour col fading to the bed picture at uv0 as the fade sfade goes to 0.
fn shore_bed(col: vec3<f32>, uv0: vec2<f32>, sfade: f32) -> vec3<f32> {
	if (sfade >= 1.0) { return col; }
	return mix(to_linear(textureSampleLevel(bed_tex, lsamp, uv0, 0.0).rgb), col, sfade);
}
"#;
