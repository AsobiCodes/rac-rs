//! The cloud WGSL: the uniform block, the light of the genera (`genera.rs` has their densities) and the three entry points.
//! `fs_cloud` marches one grid pixel through the layers in order (the cloud shell of cumulus and stratocumulus between a base and a
//! top altitude on a sphere, so the clouds recede to the horizon; the altocumulus band; the cirrus / cirrocumulus veil): a short
//! march toward the sun (Beer-Lambert with powder, dual-lobe Henyey-Greenstein softly capped so a heap before the sun never burns
//! out, multi-scatter octaves), ambient from the sky model (zenith on the tops, a darker grey-blue on the undersides and shaded
//! sides, neutral on a graded level), aerial perspective per layer. Each frame's rays are offset inside their grid pixel (`jit`, Halton) and along the march
//! (interleaved gradient noise plus a stratified step that repeats every 8 frames with the Halton set, so the history settles);
//! `fs_resolve` reprojects the history to where the clouds were (camera and wind drift), clips it to the current neighbourhood's
//! spread and blends a little of the new frame in, so cloud edges stay still. `fs_apply_clouds` lays the history over the backdrop
//! in the level pass, before any world draw.

/// WGSL: the uniform block (`Uniform` in volclouds.rs, field for field) and the bindings.
pub const BLOCK: &str = r#"
struct C {
	cam: array<vec4<f32>, 4>,      // position, forward, right * tan_x, up * tan_y
	prev: array<vec4<f32>, 4>,     // the same of the previous frame
	sun: vec4<f32>,                // toward the sun xyz, sun gain
	shell: vec4<f32>,              // planet radius, base radius, top radius, planet centre z
	cover: vec4<f32>,              // cumulus coverage, stratocumulus coverage, extinction per world unit at density 1, detail erosion
	scale: vec4<f32>,              // weather tile, shape tile, detail tile (world units), evolution speed
	wind: vec4<f32>,               // wind offset xy (world units so far), time (s), height shear (world units)
	light: vec4<f32>,              // forward g, back g, back lobe weight, powder
	haze: vec4<f32>,               // aerial perspective distance, march cap, ambient gain, exposure
	size: vec4<f32>,               // grid w, h, 1/w, 1/h
	march: vec4<f32>,              // steps, light steps, frame index mod 8, weight of the current frame
	lin: vec4<f32>,                // altocumulus steps, how thin the cloud is to the sun light, grid divisor, far plane
	tint: vec4<f32>,               // the level key light's colour (brightest channel 1), weight of the painted sun's hue
	dusk: vec4<f32>,               // dusk amount, fringe light gain, multi-scatter attenuation per octave, dusk ambient gain
	genus: vec4<f32>,              // altocumulus, cirrus, cirrocumulus coverage, cumulus vertical development
	alt: vec4<f32>,                // altocumulus band base radius, its thickness, cirrus veil radius, grade (Look::pale)
	jit: vec4<f32>,                // sub-pixel ray offset (grid pixels), the wind's drift since the last frame xy (world units)
	flow: vec4<f32>,               // wind direction xy, stratocumulus deck thickness (shell fraction), roll wavelength (world units)
	form: vec4<f32>,               // cumulonimbus form (Look::form), silver lining, underside light, surface softness
	reuse: vec4<f32>,              // x = 1: this frame marched nothing, `fs_apply_clouds` turns the stored result from the `prev` view to this one (gfxq.rs)
};
@group(0) @binding(0) var<uniform> cu: C;
@group(0) @binding(2) var shape: texture_3d<f32>;
@group(0) @binding(3) var detail: texture_3d<f32>;
@group(0) @binding(4) var weather: texture_2d<f32>;
@group(0) @binding(5) var nsamp: sampler;
@group(0) @binding(6) var cur: texture_2d<f32>;
@group(0) @binding(7) var hist: texture_2d<f32>;
@group(0) @binding(8) var lsamp: sampler;
@group(0) @binding(9) var skymap: texture_2d<f32>;

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

/// WGSL of the light and the entry points (after BLOCK, `sky::LOOKUP` and `genera::GENERA`).
pub const CLOUDS: &str = r#"
// The warm tint of the sunlit sides (cool undersides come from the ambient).
const WARM: vec3<f32> = vec3<f32>(1.0, 0.97, 0.92);
fn ign(p: vec2<f32>) -> f32 {
	return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}
fn to_lin(c: vec3<f32>) -> vec3<f32> { return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2)); }
fn to_gam(c: vec3<f32>) -> vec3<f32> { return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)); }

// The view ray through grid point `pix` (pixel corners at integers; not normalised, its forward part is 1).
fn view_dir(pix: vec2<f32>) -> vec3<f32> {
	let n = vec2<f32>(pix.x * cu.size.z * 2.0 - 1.0, 1.0 - pix.y * cu.size.w * 2.0);
	return cu.cam[1].xyz + cu.cam[2].xyz * n.x + cu.cam[3].xyz * n.y;
}

// Ray from o (relative to the sphere centre) along unit d against the sphere of radius r: (near, far); far < near = missed.
fn sphere(o: vec3<f32>, d: vec3<f32>, r: f32) -> vec2<f32> {
	let b = dot(o, d);
	let h = b * b - (dot(o, o) - r * r);
	if (h < 0.0) { return vec2<f32>(1.0, -1.0); }
	let s = sqrt(h);
	return vec2<f32>(-b - s, -b + s);
}

// Henyey-Greenstein relative to the isotropic phase (1 = isotropic), softly capped (at most 4) so the forward peak only glows:
// a thin heap in front of the sun never burns out into a bright blob.
fn hg(c: f32, g: f32) -> f32 {
	let g2 = g * g;
	let x = (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * g * c, 1.0e-4), 1.5);
	return x * 4.0 / (x + 4.0);
}

// The light of a ray: sun colour, sky ambient (zenith, horizon, ground; the dusk sky away from the sun), cos(view, sun).
struct Lights { sunc: vec3<f32>, zen: vec3<f32>, hor: vec3<f32>, gnd: vec3<f32>, dsky: vec3<f32>, cosv: f32 };

fn lights(d: vec3<f32>) -> Lights {
	let sun = cu.sun.xyz;
	// The sun's colour: the level's key light, tinted toward the painted sun's hue, at full strength.
	let sraw = sky_info(1).rgb;
	var sunc = to_lin(mix(cu.tint.rgb, sraw / max(max(sraw.r, max(sraw.g, sraw.b)), 0.05), cu.tint.w)) * cu.sun.w;
	// (A graded level's sun is near white, a touch warm: grey-white tops like its softer sky.)
	sunc = mix(sunc, vec3<f32>(dot(sunc, vec3<f32>(0.2126, 0.7152, 0.0722))) * vec3<f32>(1.02, 1.0, 0.97), cu.alt.w * 0.85);
	// At dusk the bodies take the sky away from the sun (the blue side), and only the thin fringes catch the low sun.
	let dsky = to_lin(sky_model(normalize(vec3<f32>(-sun.x, -sun.y, 1.2)), sun)) * cu.dusk.w;
	return Lights(sunc, to_lin(sky_info(2).rgb), to_lin(sky_info(3).rgb), to_lin(sky_info(4).rgb), dsky, dot(d, sun));
}

// Sun light reaching a point behind optical depth tau: the dual-lobe phase (silver lining toward the sun) through Beer-Lambert,
// and the light scattered many times inside the cloud (`dusk.z`), softer and wider (the painted, soft shading of the bodies).
fn sun_light(tau: f32, cosv: f32) -> f32 {
	let ph0 = mix(hg(cosv, cu.light.x), hg(cosv, cu.light.y), cu.light.z);
	// (Under an overcast (`form.y` below 1) the forward glow is mostly gone: no silver lining, no bright rims toward the sun.)
	let ph = select(ph0, 1.0 + (ph0 - 1.0) * cu.form.y, cu.form.y < 1.0);
	// (A third, far-reaching term: deep inside a big cloud and on its underside the light still falls off gradually with the
	// cloud's thickness, a soft gradient instead of one flat ambient grey.)
	let ms = (exp(-tau * 0.3) * 0.55 + exp(-tau * 0.08) * 0.35 + exp(-tau * 0.02) * 0.1) * mix(1.0, ph, 0.3);
	return ph * exp(-tau) + cu.dusk.z * ms;
}

// Optical depth toward the sun from p in the shell (a short march with growing steps).
fn sun_tau(p: vec3<f32>, jit: f32, fp: f32) -> f32 {
	let sun = cu.sun.xyz;
	var step = (cu.shell.z - cu.shell.y) * 0.03;
	var t = 0.0;
	var tau = 0.0;
	let n = i32(cu.march.y);
	for (var i = 0; i < n; i++) {
		let q = p + sun * (t + step * (0.3 + 0.5 * jit));
		let h = height_of(q);
		if (h >= 1.0) { break; }
		tau += shell_density(q, h, fp, i >= 2).d * step;
		t += step;
		step *= 1.8;
	}
	// (The light sees the cloud thinner than the eye: a soft lit skin and soft self-shadows, not a hard terminator.)
	return tau * cu.cover.z * cu.lin.y;
}

// One layer's result: scattered light (linear), transmittance, the light-weighted distance.
struct Seg { s: vec3<f32>, tr: f32, dist: f32 };

// Aerial perspective: a far layer takes the colour of the sky behind it (lighter, bluer, flatter) and keeps most of its body, so
// the far clouds pile into a pale band low in the sky, as on the painted skies; only the farthest thin out. At the horizon
// everything ends in the haze. `reach` scales the haze distance.
fn fade(g: Seg, d: vec3<f32>, reach: f32) -> Seg {
	let f = 1.0 - exp(-g.dist / (cu.haze.x * reach));
	let hz = 1.0 - smoothstep(0.0, 0.08, d.z + 0.01);
	let a = 1.0 - g.tr;
	let sky = to_lin(sky_model(d, cu.sun.xyz));
	let lum = dot(sky, vec3<f32>(0.2126, 0.7152, 0.0722));
	let haze = mix(sky, vec3<f32>(lum), 0.25) * 1.35 + vec3<f32>(0.04);
	let k = sat(f * 1.1);
	let s = mix(g.s, haze * a, k * 0.9);
	let tr = mix(g.tr, 1.0, max(k * k * k * 0.7, hz));
	return Seg(s * (1.0 - hz), tr, g.dist);
}

// The march through the cloud shell from t0 to t1: fine steps inside a cloud's outline (their length grows with distance, so the
// footprint of a step stays about the same on screen; short enough that the lit skin under a cloud's surface shows no step
// noise), steps 5 times as long through clear sky; at most 4 x `steps` samples.
fn march_shell(cam: vec3<f32>, d: vec3<f32>, t0: f32, t1: f32, jit: f32, L: Lights) -> Seg {
	let n = i32(cu.march.x);
	let thick = cu.shell.z - cu.shell.y;
	var tr = 1.0;
	var s = vec3<f32>(0.0);
	var tw = 0.0;
	var tws = 0.0;
	// Fine steps left (refilled inside an outline; four more after leaving it).
	var hold = 0;
	var t = t0 + max(thick / cu.march.x, t0 * 1.2 / cu.march.x) * 2.5 * jit;
	for (var k = 0; k < n * 4; k++) {
		if (t >= t1) { break; }
		let fine = max(thick / cu.march.x, t * 1.2 / cu.march.x) * 0.5;
		var dt = select(fine * 5.0, fine, hold > 0);
		let p = cam + d * t;
		let h = height_of(p);
		let fp = footprint(t);
		let den = shell_density(p, h, fp, false);
		if (den.e > 0.0) {
			if (hold == 0) {
				// Entering an outline from a long step: back up and sample it finely.
				hold = 6;
				t = max(t - fine * 5.0, t0);
				continue;
			}
			hold = 4;
		} else {
			hold = max(hold - 1, 0);
		}
		if (den.d > 0.002) {
			let tau = sun_tau(p, jit, fp);
			let powder = mix(1.0, 1.0 - exp(-den.d * 6.0), cu.light.w * (0.6 - 0.4 * L.cosv));
			// Ambient: the sky on the tops; cool blue-grey on the undersides (the horizon's light, greyed), darker toward each
			// cloud's flat base.
			let up = smoothstep(0.0, 1.0, den.hr);
			let hl = dot(L.hor, vec3<f32>(0.2126, 0.7152, 0.0722));
			let under = mix(vec3<f32>(hl), L.hor, 0.5) * (0.55 * cu.form.z) + L.gnd * 0.05;
			let a0 = mix(under, L.zen * 0.6 + L.hor * 0.4, up);
			// (A graded level's shade is a neutral blue-grey, not the sky's cyan.)
			let amb = mix(a0, vec3<f32>(dot(a0, vec3<f32>(0.2126, 0.7152, 0.0722))) * vec3<f32>(1.0, 1.0, 1.08), cu.alt.w * 0.75) * cu.haze.z * mix(0.6, 1.0, up) * mix(0.8, 1.2, den.k);
			var l = L.sunc * mix(WARM, vec3<f32>(1.0), cu.alt.w) * (sun_light(tau, L.cosv) * powder) + mix(amb, L.dsky * mix(0.6 * cu.form.z, 1.0, up), cu.dusk.x);
			if (cu.dusk.y > 0.0) {
				l += L.sunc * cu.dusk.y * (1.0 - sat(den.d * 4.0)) * hg(L.cosv, 0.5) * exp(-tau * 0.1);
			}
			let e = exp(-den.d * cu.cover.z * dt);
			let w = tr * (1.0 - e);
			s += w * l;
			tw += w * t;
			tws += w;
			tr *= e;
			if (tr < 0.01) { break; }
		}
		t += dt;
	}
	return Seg(s, tr, select(t0, tw / max(tws, 1.0e-6), tws > 1.0e-4));
}

// The march through the altocumulus band (light from one sample toward the sun: the ripples' sunlit and shaded sides).
fn march_mid(cam: vec3<f32>, d: vec3<f32>, t0: f32, t1: f32, jit: f32, L: Lights) -> Seg {
	let n = i32(cu.lin.x);
	let dt = (t1 - t0) / f32(n);
	let sig = cu.cover.z * 0.12;
	let sun = cu.sun.xyz;
	let reach = cu.alt.y * 0.5 / max(sun.z, 0.25);
	var tr = 1.0;
	var s = vec3<f32>(0.0);
	let fp = footprint(t0);
	for (var k = 0; k < n; k++) {
		let p = cam + d * (t0 + dt * (f32(k) + jit));
		let dens = mid_density(p, fp);
		if (dens > 0.002) {
			let tau = (dens * 0.5 + mid_density(p + sun * reach, fp)) * sig * reach;
			let powder = mix(1.0, 1.0 - exp(-dens * 6.0), cu.light.w * (0.6 - 0.4 * L.cosv));
			let amb = (L.zen * 0.8 + L.hor * 0.2) * cu.haze.z * 0.85;
			let l = L.sunc * (sun_light(tau, L.cosv) * powder) + mix(amb, L.dsky, cu.dusk.x);
			let e = exp(-dens * sig * dt);
			s += tr * (1.0 - e) * l;
			tr *= e;
		}
	}
	return Seg(s, tr, t0);
}

// The march of one grid pixel: rgb = light relative to the backdrop, a = transmittance.
@fragment fn fs_cloud(v: VO) -> @location(0) vec4<f32> {
	let none = vec4<f32>(0.0, 0.0, 0.0, 1.0);
	let d = normalize(view_dir(v.pos.xy + cu.jit.xy));
	let cam = cu.cam[0].xyz;
	let o = cam - centre();
	if (sphere(o, d, cu.shell.x).x > 0.0) { return none; }
	// Along-ray offset: stratified over 8 frames in step with the Halton jitter (period 8). An aperiodic sequence never averages
	// out in the history: far cloud edges boiled.
	let jit = fract(ign(v.pos.xy) + cu.march.z * 0.375);
	let L = lights(d);
	var s = vec3<f32>(0.0);
	var tr = 1.0;
	// The cloud shell: cumulus and stratocumulus.
	let rb = sphere(o, d, cu.shell.y);
	let rt = sphere(o, d, cu.shell.z);
	let ro = length(o);
	var t0 = 0.0;
	var t1 = rt.y;
	var hit = true;
	if (ro < cu.shell.y) {
		t0 = rb.y;
	} else if (ro > cu.shell.z) {
		hit = rt.x >= 0.0 && rt.y >= rt.x;
		t0 = rt.x;
		t1 = select(rt.y, rb.x, rb.x > 0.0 && rb.y > rb.x);
	} else if (rb.x > 0.0 && rb.y > rb.x) {
		t1 = rb.x;
	}
	t1 = min(t1, t0 + cu.haze.y * 0.5);
	if (hit && t1 > t0 && t0 < cu.haze.y && (cu.cover.x > 0.0 || cu.cover.y > 0.0)) {
		let g = fade(march_shell(cam, d, t0, t1, jit, L), d, 1.0);
		s += g.s;
		tr *= g.tr;
	}
	// The altocumulus band above it.
	if (tr > 0.01 && cu.genus.x > 0.0) {
		let m0 = sphere(o, d, cu.alt.x).y;
		let m1 = min(sphere(o, d, cu.alt.x + cu.alt.y).y, m0 + cu.alt.y * 6.0);
		if (m0 > 0.0 && m1 > m0 && m0 < cu.haze.y) {
			let g = fade(march_mid(cam, d, m0, m1, jit, L), d, 1.4);
			s += tr * g.s;
			tr *= g.tr;
		}
	}
	// The cirrus / cirrocumulus veil at the top.
	if (tr > 0.01 && cu.genus.y + cu.genus.z > 0.0) {
		let th = sphere(o, d, cu.alt.z).y;
		if (th > 0.0 && th < cu.haze.y * 1.5) {
			let op = high_opacity(cam + d * th, footprint(th));
			let col = L.sunc * (0.4 + 0.3 * hg(L.cosv, 0.6)) + mix(L.zen * cu.haze.z * 0.75, L.dsky, cu.dusk.x);
			let g = fade(Seg(col * op, 1.0 - op, th), d, 1.8);
			s += tr * g.s;
			tr *= g.tr;
		}
	}
	if (tr > 0.999 && dot(s, s) < 1.0e-8) { return none; }
	let skyg = sky_model(d, cu.sun.xyz);
	let col = to_gam(s * cu.haze.w + to_lin(skyg) * tr);
	// Soft shoulder toward white.
	let m = max(col.r, max(col.g, col.b));
	let shaped = select(col, col * ((0.85 + 0.15 * (1.0 - exp(-(m - 0.85) / 0.15))) / m), m > 0.85);
	return vec4<f32>(max(shaped - skyg * tr, vec3<f32>(0.0)), tr);
}

// Temporal accumulation: the previous result where this pixel's cloud was (at the cloud base distance, moved back by the wind's
// drift), clipped to the spread of the current 3x3 neighbourhood (mean +- 2 sigma inside its min / max while the view moves, up
// to 4 sigma when it is still), blended in.
@fragment fn fs_resolve(v: VO) -> @location(0) vec4<f32> {
	let c = vec2<i32>(v.pos.xy);
	let dims = vec2<i32>(textureDimensions(cur));
	let now = textureLoad(cur, c, 0);
	var m1 = vec4<f32>(0.0);
	var m2 = vec4<f32>(0.0);
	var mn = now;
	var mx = now;
	for (var y = -1; y <= 1; y++) {
		for (var x = -1; x <= 1; x++) {
			let s = textureLoad(cur, clamp(c + vec2<i32>(x, y), vec2<i32>(0), dims - vec2<i32>(1)), 0);
			m1 += s;
			m2 += s * s;
			mn = min(mn, s);
			mx = max(mx, s);
		}
	}
	m1 /= 9.0;
	let sd = sqrt(max(m2 / 9.0 - m1 * m1, vec4<f32>(0.0)));
	let d = normalize(view_dir(v.pos.xy));
	let rb = sphere(cu.cam[0].xyz - centre(), d, cu.shell.y);
	let t = select(cu.haze.y, rb.y, rb.y > 0.0);
	let pv = cu.cam[0].xyz + d * t - vec3<f32>(cu.jit.zw, 0.0) - cu.prev[0].xyz;
	let z = dot(pv, cu.prev[1].xyz);
	var a = 1.0;
	var uv = vec2<f32>(0.0);
	if (z > 0.1) {
		let rr = cu.prev[2].xyz;
		let uu = cu.prev[3].xyz;
		uv = vec2<f32>(dot(pv, rr) / (z * dot(rr, rr)) * 0.5 + 0.5, 0.5 - dot(pv, uu) / (z * dot(uu, uu)) * 0.5);
		if (uv.x > 0.0 && uv.y > 0.0 && uv.x < 1.0 && uv.y < 1.0) { a = cu.march.w; }
	}
	// A still view keeps its history nearly unclipped (the jittered frames average out, edges stay put); the clip tightens as the
	// history moves on screen.
	let moved = length((uv - v.uv) * vec2<f32>(dims));
	let k = mix(4.0, 2.0, smoothstep(0.05, 0.6, moved));
	let lo = max(mn - sd * (k - 2.0), m1 - sd * k);
	let hi = min(mx + sd * (k - 2.0), m1 + sd * k);
	let h = clamp(textureSampleLevel(hist, lsamp, uv, 0.0), lo, hi);
	// Half the new frame's weight while the history is nearly still (more frames averaged: no shimmer on far edges); full weight once
	// it moves (no smear), fresh pixels stay 1.
	let a2 = select(a * mix(0.5, 1.0, smoothstep(0.05, 0.6, moved)), a, a >= 1.0);
	return mix(h, now, a2);
}

// The clouds over the backdrop (in the level pass right after the sky; every world draw and effect lands on top), upsampled with a
// cubic B-spline (four linear taps): smooth edges instead of grid-pixel steps.
@fragment fn fs_apply_clouds(v: VO) -> @location(0) vec4<f32> {
	let dims = vec2<f32>(textureDimensions(cur));
	var uv = v.uv;
	if (cu.reuse.x > 0.5) {
		// A frame without a march: the stored result, drawn from the view it was marched in, looked up where this pixel's ray (at
		// the cloud base, as `fs_resolve` does) was in that view. The clouds are far away: the view's turn is all that matters.
		let n = vec2<f32>(v.uv.x * 2.0 - 1.0, 1.0 - v.uv.y * 2.0);
		let d = normalize(cu.cam[1].xyz + cu.cam[2].xyz * n.x + cu.cam[3].xyz * n.y);
		let rb = sphere(cu.cam[0].xyz - centre(), d, cu.shell.y);
		let t = select(cu.haze.y, rb.y, rb.y > 0.0);
		let pv = cu.cam[0].xyz + d * t - cu.prev[0].xyz;
		let z = dot(pv, cu.prev[1].xyz);
		if (z > 0.1) {
			let rr = cu.prev[2].xyz;
			let uu = cu.prev[3].xyz;
			uv = vec2<f32>(dot(pv, rr) / (z * dot(rr, rr)) * 0.5 + 0.5, 0.5 - dot(pv, uu) / (z * dot(uu, uu)) * 0.5);
		}
	}
	let q = uv * dims - 0.5;
	let i = floor(q);
	let f = q - i;
	let f2 = f * f;
	let f3 = f2 * f;
	let w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
	let w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
	let w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
	let w3 = f3 / 6.0;
	let g0 = w0 + w1;
	let g1 = w2 + w3;
	let h0 = (i - 0.5 + w1 / g0) / dims;
	let h1 = (i + 1.5 + w3 / g1) / dims;
	let a = textureSampleLevel(cur, lsamp, vec2<f32>(h0.x, h0.y), 0.0) * g0.x + textureSampleLevel(cur, lsamp, vec2<f32>(h1.x, h0.y), 0.0) * g1.x;
	let b = textureSampleLevel(cur, lsamp, vec2<f32>(h0.x, h1.y), 0.0) * g0.x + textureSampleLevel(cur, lsamp, vec2<f32>(h1.x, h1.y), 0.0) * g1.x;
	return a * g0.y + b * g1.y;
}
"#;
