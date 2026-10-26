//! [PC] Water Enhanced (Split view: inside the water pass, `DrawFx.water_on`): the surface seen from below (WGSL, appended to `SHADER` in `WaterFx::new`). From above the water pass
//! replaces retail's sea / pond picture; from below it used to leave it (`fs_water` discarded every pixel of a surface the eye is
//! under), so a diver saw retail's pale wispy water texture across the top of the screen (user 2026-10-24, Pokitaru). Retail's quads
//! are drawn with the level either way; the pass now overpaints them from below too, as an underside: the same geometric waves
//! (the grid raises the surface for either side), and per pixel
//!  - Snell's window: a ray that meets the surface within the critical angle (about 48.6 degrees from the vertical) goes through
//!    and shows the sky (and the sun) as refracted, squeezed into that cone,
//!  - total internal reflection outside it: the surface is a mirror of the water below, read from the picture by the screen-space
//!    reflection (`reflect_ray`) and, where it finds nothing, a dark water colour,
//!  - the unpolarised Fresnel share between the two at every angle, so the window's rim is a soft bright ring, and the wave slope
//!    bends both.
//! `viewer/underwater.rs` then tints and fogs the whole picture by the water path to the surface as before.

pub(super) const UNDER_WGSL: &str = r#"
// Refractive index of water; the sun's glow seen through the window; the dark water colour a reflection that finds nothing shows
// (times the fog colour); the water path (world units) over which a reflected hit fades to it.
const UND_ETA: f32 = 1.33;
const UND_SUN: f32 = 2.2;
const UND_DEEP: vec3<f32> = vec3<f32>(0.10, 0.30, 0.34);
const UND_FADE: f32 = 40.0;

// Unpolarised Fresnel reflectance of the water-to-air surface for the angles `ci` (in the water) and `ct` (in the air).
fn und_fresnel(ci: f32, ct: f32) -> f32 {
	let rs = (UND_ETA * ci - ct) / (UND_ETA * ci + ct);
	let rp = (UND_ETA * ct - ci) / (UND_ETA * ct + ci);
	return 0.5 * (rs * rs + rp * rp);
}

// The sky a refracted ray `t` (going up) shows: the level's fog colour at the horizon, bluer and brighter overhead, the sun's glow.
fn und_sky(t: vec3<f32>) -> vec3<f32> {
	let up = smoothstep(0.0, 0.85, t.z);
	let hor = w.fog.rgb * 1.25 + vec3<f32>(0.03);
	let top = w.fog.rgb * vec3<f32>(0.75, 1.0, 1.35) * 1.3 + vec3<f32>(0.04, 0.07, 0.11);
	var c = mix(hor, top, up);
	if (w.sun.w > 0.0) {
		let s = max(dot(t, normalize(w.sun.xyz)), 0.0);
		c += vec3<f32>(1.0, 0.93, 0.78) * (pow(s, 24.0) * 0.25 + pow(s, 600.0) * 3.0) * UND_SUN * (0.4 + 0.6 * clamp(w.sun.w / 2.2, 0.0, 1.0));
	}
	return c;
}

// Only plain water gets an underside: not the lava, mud, sludge, frozen or cold looks (flow slot w), which keep retail's picture.
fn under_ok(a: f32) -> bool {
	let fw = w.flows[u32(a - 0.5) & 63u].w;
	return abs(fw) < 0.5;
}

// The surface point `p` (its flat point `pf`) seen from the eye below it, shaded in linear light and finished like the top side.
fn under_side(c: vec2<i32>, p: vec3<f32>, pf: vec3<f32>, geo: bool, sea: bool) -> vec4<f32> {
	let eye = w.eye.xyz;
	let time = w.eye.w;
	let uv0 = (vec2<f32>(c) + 0.5) * w.size.zw;
	let orig = to_linear(textureSampleLevel(scene, lsamp, uv0, 0.0).rgb);
	let jit = dither(vec2<f32>(c) + 0.5);
	let te = p - eye;
	let dist = max(length(te), 1e-3);
	let r = te / dist;
	let fp = dist * 2.0 * w.proj.y * w.size.w / max(r.z, 0.2);
	// The slope of the surface: the fine ripples and the wave trains (the geometric sea's own slope), calmed at the shores.
	var rs = ripple(p.xy, time, fp * (1.0 + FAR_RIPPLE * smoothstep(FAR_FROM, FAR_TO, dist))) * w.look.x * 0.7;
	if (geo) {
		let df = max(length(eye - pf), 1e-3);
		let fpi = df * 2.0 * w.proj.y * w.size.w * (1.0 + 3.0 * smoothstep(15.0, 40.0, dist));
		let sw = sea_w(p.xy, time, fpi, fpi * w.ggrid.w, geo_an(pf - eye), i32(w.wave.x));
		rs += sw.g * calm(pf).y;
	} else if (w.wave.x > 0.5 && sea) {
		rs += waves(p.xy, time, fp, 1.0, i32(w.wave.x)).yz * w.wave.z;
	}
	let nu = normalize(vec3<f32>(-rs.x, -rs.y, 1.0));
	let ci = clamp(dot(r, nu), 0.0, 1.0);
	let t = refract(r, -nu, UND_ETA);
	let tir = dot(t, t) < 1e-4;
	// The mirror: the water below, from the picture (a hit fades into the dark water colour with the path it travelled).
	let deep = UND_DEEP * (w.fog.rgb + vec3<f32>(0.12));
	var refl = deep;
	let rr = reflect(r, -nu);
	let hit = reflect_ray(p - nu * 0.05, rr, jit);
	if (hit.w > 0.0) {
		let seen = soft_scene(hit.xy, 0.002, jit);
		refl = mix(deep, mix(seen, deep, 1.0 - exp(-hit.z / UND_FADE)), hit.w);
	}
	var col = refl;
	if (!tir) {
		let f = und_fresnel(ci, clamp(dot(t, nu), 0.0, 1.0));
		col = mix(und_sky(normalize(t)), refl, f);
	}
	return finish(c, orig, col, geo);
}
"#;
