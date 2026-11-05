
// [PC] Fire simulation, the flames' heat buffer (firesim_heat.rs; this file is appended to firesim.wgsl, so vs_part, blackbody and
// tinted are shared). The flames are not shaded one sprite at a time: pass A (fs_heat) draws every flame particle as a soft round blob
// of heat into a small Rgba16Float target with MAX blending (r heat, g 1 / view depth: the nearest, b/a the retail tint's
// chromaticity packed, later the particle size), tested against the scene's depth (max, not a sum: a stream of hundreds of overlapping particles or a far fire never
// saturates into a flat disc; the particles' own temperatures give the core and the tips); pass B (vs_comp / fs_comp, inside the particle pass) reads it back through a rising, time-varying, world-sized
// domain warp (the edges lick up as tongues), compresses the heat (1 - exp(-k h): a dense fire keeps its structure, never a flat white
// disc) and maps it through a blackbody ramp. The flames add light; a thin soft cover only where the heat is low (no black edges).

// dims (full w, full h, heat w, heat h)  k (tan of half the vertical fov, warp octaves 0..2, clock s, 0)
// w (compress k, warp reach m, brightness, tongue size m)
struct HeatU { dims: vec4<f32>, k: vec4<f32>, w: vec4<f32> };
@group(3) @binding(0) var<uniform> hu: HeatU;
@group(3) @binding(1) var heat_tex: texture_2d<f32>;
@group(3) @binding(2) var heat_samp: sampler;

// Below this compressed heat nothing shows.
const HEAT_MIN: f32 = 0.08;
// How fast the tongue pattern rises (m/s).
const HEAT_RISE: f32 = 1.3;
// The particle size (m) at which the tongues have their base size (fire_heat_tongue); bigger particles (a big fire) scale them up.
const SIZE_REF: f32 = 0.45;

// Pass A: soft.p = (near, far, full-res pixels per heat pixel, heat per particle).
@fragment fn fs_heat(i: VOut) -> @location(0) vec4<f32> {
	let d2 = dot(i.uv, i.uv);
	let lick = i.kind >= 8u;
	if ((d2 >= 1.0 && !lick) || i.col.w <= 0.0) {
		discard;
	}
	var g = max(exp(-2.4 * d2) - 0.09, 0.0) * 1.1;
	if (lick) {
		g = lick_heat(i); // only a lick pays for its tongue pattern
	}
	let dim = vec2<i32>(textureDimensions(soft_depth)) - vec2<i32>(1);
	let px = clamp(vec2<i32>(i.pos.xy * soft.p.z), vec2<i32>(0), dim);
	let n = soft.p.x;
	let f = soft.p.y;
	let ls = n * f / (f - textureLoad(soft_depth, px, 0) * (f - n));
	let lf = n * f / (f - i.pos.z * (f - n));
	let sf = clamp((ls - lf) / SOFT_D, 0.0, 1.0);
	let h = g * i.col.w * sf * soft.p.w;
	if (h <= 1.0e-4) {
		discard;
	}
	let t = max(i.col.rgb, vec3<f32>(0.0));
	let sum = max(t.r + t.g + t.b, 1.0e-3);
	// b: the particle's size (m); a: the tint's chromaticity packed into one number (r in the whole part, g in the fraction), so the
	// max keeps one particle's tint whole. 0 where the heat is too thin to count.
	let on = step(0.02, h);
	// (+ LICK_TAG on a level lick: the composite gives its pixels their own warp and colour, `tint_at`.)
	let packed = 1.0 + floor(t.r / sum * 30.0 + 0.5) + min(t.g / sum, 1.0) * 0.98 + select(0.0, LICK_TAG, lick);
	// The size and the inverse depth fade in over the blob's outer part (not a step at its rim): the max over overlapping blobs stays
	// continuous, so the noise scale (metres per pixel) and the tongue size read from them do not jump along every blob's edge (thin
	// dark seams around each lobe of a stream).
	let rim = smoothstep(0.0, 0.35, g);
	return vec4<f32>(h, mix(0.6, 1.0, rim) / max(lf, 0.05), i.sm.x * rim, packed * on);
}

// A level lick's flame shape inside its upright quad (uv.y -1 foot .. 1 tip, uv.x across): a tongue widest just over its foot,
// narrowing to its tip, its edge torn by two octaves of domain-warped noise laid in the lick's own space and scrolling up with its age
// (q.z), cut with a narrow smoothstep (crisp, wispy edges; the tip splits into fingers); its heat falls from foot to tip (a yellow
// root, an orange body, red tips once the composite ramps it) times LICK_HEAT (one lick alone never reaches the near-white end).
const LICK_HEAT: f32 = 0.7;
const LICK_TAG: f32 = 64.0;
fn lick_heat(i: VOut) -> f32 {
	let uv = i.uv;
	let v = uv.y * 0.5 + 0.5;
	let age = i.q.z;
	let sd = i.q.xy;
	let w1 = textureSampleLevel(ntex, nsamp, vec3<f32>(uv.x * 0.3 + sd.x, uv.y * 0.4 - age * 0.7 + sd.y, age * 0.3 + sd.x * 0.37), 0.0);
	let wo = (w1.rg - vec2<f32>(0.5)) * 2.0;
	let n2 = textureSampleLevel(ntex, nsamp, vec3<f32>(uv.x * 0.62 + wo.x * 0.18 + sd.y, uv.y * 0.8 - age * 1.3 + wo.y * 0.15, age * 0.5 + 0.5), 0.0);
	let n3 = textureSampleLevel(ntex, nsamp, vec3<f32>(uv.x * 1.3 + wo.y * 0.2 + sd.x, uv.y * 1.6 - age * 2.2, age * 0.8 + 0.25), 0.0);
	// The body sways more towards the tip, curling (an S wave travelling up); the root is wider and hotter so neighbouring roots
	// join into one glowing band along the burning edge.
	let x = uv.x + (wo.x * 0.45 + (n2.r - 0.5) * 0.8) * v + sin(v * 4.5 - age * 5.0 + sd.x * 6.283) * 0.34 * v * v;
	let width = mix(0.95, 0.62, smoothstep(0.0, 0.3, v)) * pow(max(1.0 - v, 0.0), 0.6);
	// Mostly eating in (never past the quad's end: no flat top), most at the tip, where the fingers split.
	let tear = ((n2.a - 0.58) * 0.7 + (n3.g - 0.55) * 0.6) * (0.25 + 1.1 * v);
	let edge = width - abs(x) + tear * 0.6 - 0.25 * smoothstep(0.7, 1.0, v);
	let shape = smoothstep(0.0, 0.1, edge) * smoothstep(0.0, 0.1, v + (n3.b - 0.5) * 0.1);
	return shape * mix(1.25, 0.22, pow(v, 0.6)) * LICK_HEAT;
}

// The tint packed by fs_heat (0: none: a plain warm fire; + LICK_TAG: a level lick's).
fn unpack_tint(a0: f32) -> vec3<f32> {
	let a = select(a0, a0 - LICK_TAG, a0 >= LICK_TAG);
	if (a < 1.0) {
		return vec3<f32>(1.0, 0.5, 0.15);
	}
	let r = clamp((floor(a) - 1.0) / 30.0, 0.0, 1.0);
	let g = clamp(fract(a) / 0.98, 0.0, 1.0);
	return vec3<f32>(r, g, max(1.0 - r - g, 0.0));
}

// One heat pixel's tint (`unpack_tint`; none there: the pixel below it, as a tongue licking up takes its colour from below).
fn tint_px(q: vec2<i32>) -> vec4<f32> {
	let hdim = vec2<i32>(textureDimensions(heat_tex)) - vec2<i32>(1);
	let tp = textureLoad(heat_tex, clamp(q, vec2<i32>(0), hdim), 0).a;
	let tb = textureLoad(heat_tex, clamp(q + vec2<i32>(0, i32(0.03 * hu.dims.w + 0.5)), vec2<i32>(0), hdim), 0).a;
	let a = select(tb, tp, tp >= 1.0);
	return vec4<f32>(unpack_tint(a), select(0.0, 1.0, a >= 1.0));
}

// The share of level licks (LICK_TAG) at `uv`, the four nearest heat pixels blended bilinearly.
fn lick_at(uv: vec2<f32>) -> f32 {
	let f = uv * hu.dims.zw - vec2<f32>(0.5);
	let b = vec2<i32>(floor(f));
	let w = f - floor(f);
	let l00 = lick_px(b);
	let l10 = lick_px(b + vec2<i32>(1, 0));
	let l01 = lick_px(b + vec2<i32>(0, 1));
	let l11 = lick_px(b + vec2<i32>(1, 1));
	let k = vec4<f32>((1.0 - w.x) * (1.0 - w.y), w.x * (1.0 - w.y), (1.0 - w.x) * w.y, w.x * w.y) * vec4<f32>(l00.y, l10.y, l01.y, l11.y);
	let sum = k.x + k.y + k.z + k.w;
	if (sum < 1.0e-4) {
		return 0.0;
	}
	return dot(k, vec4<f32>(l00.x, l10.x, l01.x, l11.x)) / sum;
}

// One heat pixel (or, empty, the one a little below it, as `tint_px`): (a level lick 0/1, any fire 0/1).
fn lick_px(q: vec2<i32>) -> vec2<f32> {
	let hdim = vec2<i32>(textureDimensions(heat_tex)) - vec2<i32>(1);
	let tp = textureLoad(heat_tex, clamp(q, vec2<i32>(0), hdim), 0).a;
	let tb = textureLoad(heat_tex, clamp(q + vec2<i32>(0, i32(0.03 * hu.dims.w + 0.5)), vec2<i32>(0), hdim), 0).a;
	let a = select(tb, tp, tp >= 1.0);
	return vec2<f32>(step(LICK_TAG, a), step(1.0, a));
}

// The tint at `uv`, the four nearest heat pixels' tints blended bilinearly (pixels with no tint left out).
fn tint_at(uv: vec2<f32>) -> vec3<f32> {
	let f = uv * hu.dims.zw - vec2<f32>(0.5);
	let b = vec2<i32>(floor(f));
	let w = f - floor(f);
	let t00 = tint_px(b);
	let t10 = tint_px(b + vec2<i32>(1, 0));
	let t01 = tint_px(b + vec2<i32>(0, 1));
	let t11 = tint_px(b + vec2<i32>(1, 1));
	let k00 = (1.0 - w.x) * (1.0 - w.y) * t00.w;
	let k10 = w.x * (1.0 - w.y) * t10.w;
	let k01 = (1.0 - w.x) * w.y * t01.w;
	let k11 = w.x * w.y * t11.w;
	let sum = k00 + k10 + k01 + k11;
	if (sum < 1.0e-4) {
		return unpack_tint(0.0);
	}
	return (t00.rgb * k00 + t10.rgb * k10 + t01.rgb * k01 + t11.rgb * k11) / sum;
}

// Pass B: one triangle over the screen.
@vertex fn vs_comp(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
	let x = f32((vi << 1u) & 2u);
	let y = f32(vi & 2u);
	return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn heat_at(uv: vec2<f32>) -> vec4<f32> {
	return textureSampleLevel(heat_tex, heat_samp, uv, 0.0);
}

// The fire ramp of compressed heat x (HEAT_MIN..1): deep red, orange, yellow, a near-white core.
fn fire_ramp(x: f32) -> vec3<f32> {
	let t = clamp((x - HEAT_MIN) / (1.0 - HEAT_MIN), 0.0, 1.0);
	let c0 = vec3<f32>(0.42, 0.03, 0.0);
	let c1 = vec3<f32>(1.0, 0.32, 0.02);
	let c2 = vec3<f32>(1.0, 0.72, 0.16);
	let c3 = vec3<f32>(1.0, 0.93, 0.7);
	var c = mix(c0, c1, smoothstep(0.0, 0.38, t));
	c = mix(c, c2, smoothstep(0.38, 0.75, t));
	c = mix(c, c3, smoothstep(0.82, 1.0, t));
	return c * (0.25 + 0.95 * t);
}

// The game's own colour bends the ramp only where it is not a red-orange-yellow fire (the upgraded Pyrocitor's green flame): the core
// stays light, the body takes the tint. A warm tint keeps the ramp as it is.
fn heat_tint(c: vec3<f32>, tint: vec3<f32>, t: f32) -> vec3<f32> {
	let tn = tint / max(max(tint.r, tint.g), max(tint.b, 1.0e-3));
	let k = clamp((max(tn.g, tn.b) - tn.r) * 4.0 + 0.5 * su.look.w, 0.0, 1.0);
	let luma = dot(c, vec3<f32>(0.3, 0.55, 0.15));
	let body = tn * luma * 1.35 + vec3<f32>(smoothstep(0.8, 1.0, t) * 0.35);
	return mix(c, body, k);
}

// The noise coordinates of one pixel: the pixel at the lower power-of-two scale, the blend to the next, the rise.
struct NQ { p: vec2<f32>, f: f32, rise: vec2<f32> };

// The wind noise at `nq` (each scale's pattern x `mul` + `add`, minus the rise), blended between the two scales.
fn noise_q(nq: NQ, mul: vec2<f32>, add: vec2<f32>, z: f32) -> vec4<f32> {
	let a = textureSampleLevel(ntex, nsamp, vec3<f32>((nq.p - nq.rise) * mul + add, z), 0.0);
	let b = textureSampleLevel(ntex, nsamp, vec3<f32>((nq.p * 2.0 - nq.rise) * mul + add, z), 0.0);
	return mix(a, b, nq.f);
}

// A level lick's ramp (real flame colours): deep red-orange tips, an orange body, yellow (never white) at the root; dimmer at the
// tips than the plain ramp, and a little cover (LICK_COVER) under the body so the fire is not washed out over a bright sky.
const LICK_WARP: f32 = 0.3;
const LICK_COVER: f32 = 0.35;
fn lick_ramp(x: f32) -> vec3<f32> {
	let t = clamp((x - HEAT_MIN) / (0.8 - HEAT_MIN), 0.0, 1.0);
	var c = mix(vec3<f32>(0.5, 0.06, 0.0), vec3<f32>(1.0, 0.3, 0.02), smoothstep(0.0, 0.45, t));
	c = mix(c, vec3<f32>(1.0, 0.62, 0.08), smoothstep(0.45, 0.8, t));
	c = mix(c, vec3<f32>(1.0, 0.85, 0.3), smoothstep(0.85, 1.0, t));
	return c * (0.3 + 0.85 * t);
}

@fragment fn fs_comp(@builtin(position) pos: vec4<f32>) -> FireOut {
	let full = hu.dims.xy;
	let uv = pos.xy / full;
	// The heat here or a little below (a tongue licking up into an empty pixel takes its depth from below).
	let h0 = heat_at(uv);
	let hb = heat_at(uv + vec2<f32>(0.0, 0.03));
	// Blended, not picked (a hard pick switches depth and size along a line inside the body: a seam).
	let src = mix(hb, h0, smoothstep(-0.03, 0.03, h0.r - hb.r));
	if (src.r < 0.003) {
		discard;
	}
	let depth = 1.0 / max(src.g, 1.0e-3);
	// Metres per pixel at the fire's depth: the noise is world-sized (a big fire gets many tongues, a far one fewer pixels each).
	let mpp = max(depth, 0.1) * 2.0 * hu.k.x / full.y;
	let t = hu.k.z;
	// The fire's own scale from its particles' size (a big fire's flames are bigger: taller tongues, a stronger warp, a raggeder top).
	let sc = clamp(src.b / SIZE_REF, 0.7, 2.6);
	let tongue = max(hu.w.w, 0.05) * sc;
	// The noise is laid in screen space at the two power-of-two scales around the wanted one (metres per pixel / tongue size) and
	// blended: inside one scale band the pattern is a plain linear map of the pixel, so a scale that changes across the fire (depth
	// and size differ blob to blob) only shifts the blend, never squeezes the pattern into thin dark seams along the blobs' rims.
	let lv = log2(max(mpp / tongue, 1.0e-6));
	let l0 = floor(lv);
	let nq = NQ(vec2<f32>(pos.x, -pos.y) * exp2(l0), lv - l0, vec2<f32>(0.0, t * HEAT_RISE / max(hu.w.w, 0.05)));
	var h = h0.r;
	let oct = hu.k.y;
	// A level lick's own shape is already a tongue (lick_heat): it takes only LICK_WARP of the warp and erosion, and its own ramp.
	let lk = lick_at(uv);
	if (oct > 0.5) {
		// Stronger warp where the heat is thin: the edges become licking tongues, the core holds.
		let hn = 1.0 - exp(-hu.w.x * h0.r);
		// The warp's reach scales with the particles' own size with no floor (a floor of 0.7 x SIZE_REF pulled the heat of a fine stream
		// or a small burst out far past its particles: the size rule, firesim_step.wgsl WARP_REACH).
		let reach = clamp(src.b / SIZE_REF, 0.15, 2.6);
		let amp = hu.w.y * reach * (1.3 - 0.8 * hn) * mix(1.0, LICK_WARP, lk);
		let n1 = noise_q(nq, vec2<f32>(0.5), vec2<f32>(0.0), t * 0.23);
		var off = (n1.rg - vec2<f32>(0.5)) * vec2<f32>(1.0, 1.6) * amp;
		if (oct > 1.5) {
			let n2 = noise_q(nq, vec2<f32>(1.4), vec2<f32>(0.31, 0.77), t * 0.41 + 0.5);
			off += (n2.gb - vec2<f32>(0.5)) * vec2<f32>(0.6, 1.0) * amp;
		}
		// Sampled from below (world down): the heat is carried up into tongues.
		off.y -= amp * 0.35;
		h = heat_at(uv + vec2<f32>(off.x, -off.y) / (mpp * full)).r;
		// Erosion: noise stretched upright (narrow across, long up) tears the rim into separate tongues, hardest at the fire's top
		// edge (more heat below than here) so a big round fire ends in flames licking up, not a ball.
		let nf = noise_q(nq, vec2<f32>(1.6, 0.5), vec2<f32>(0.0), t * 0.37).a;
		// "Below" is half a tongue down in the world (not a fixed share of the screen): only a real top edge erodes.
		let hd = heat_at(uv + vec2<f32>(0.0, 0.5 * tongue / (mpp * full.y))).r;
		// Only where the heat here is thin (a real rim): inside a dense body (a flame stream) the max of overlapping blobs steps at every
		// blob's edge, and eroding there drew dark seams around each lobe.
		let top = clamp((hd - h0.r) * 3.0, 0.0, 1.0) * (1.0 - smoothstep(0.35, 0.7, hn));
		let er = clamp((1.0 - 0.45 * hn + 0.6 * top) * (0.8 + 0.2 * sc), 0.0, 1.0) * mix(1.0, LICK_WARP, lk);
		h *= mix(1.0, 0.05 + 1.9 * nf, er);
	}
	let hc = 1.0 - exp(-hu.w.x * h);
	if (hc < HEAT_MIN) {
		discard;
	}
	// The heat target is smaller than the screen: its occlusion (tested per heat pixel in pass A) is redone here per screen pixel, so
	// the fire's edge against a nearer wall or building is smooth, not stepped in heat pixels. A pixel whose scene lies in front of the
	// fire's nearest flame (by more than the soft distance) fades out.
	let dfull = vec2<i32>(textureDimensions(soft_depth)) - vec2<i32>(1);
	let zs = textureLoad(soft_depth, clamp(vec2<i32>(pos.xy), vec2<i32>(0), dfull), 0);
	let ls = soft.p.x * soft.p.y / (soft.p.y - zs * (soft.p.y - soft.p.x));
	// (The stored inverse depth fades to 0.6 x over a blob's rim, so the nearest flame lies between 0.6 x and 1 x `depth`: only a scene
	// in front of the nearer bound hides it.)
	let vis = clamp((ls - 0.6 * depth) / SOFT_D + 1.0, 0.0, 1.0);
	if (vis <= 0.0) {
		discard;
	}
	// The tint: each heat pixel's packed tint unpacked, then blended bilinearly (a packed tint cannot be filtered; read nearest it drew
	// the colour in heat-pixel blocks).
	let tint = tint_at(uv);
	let tt = clamp((hc - HEAT_MIN) / (1.0 - HEAT_MIN), 0.0, 1.0);
	let e = heat_tint(mix(fire_ramp(hc), lick_ramp(hc), lk), tint, tt) * hu.w.z * smoothstep(HEAT_MIN, HEAT_MIN + 0.1, hc) * vis;
	// A thin cover only at low heat (the flame's body hides a little of what is behind; no dark rim).
	let a = mix(0.1 * smoothstep(HEAT_MIN, 0.3, hc) * (1.0 - tt), LICK_COVER * smoothstep(HEAT_MIN, 0.4, hc), lk) * vis;
	var o: FireOut;
	o.c = vec4<f32>(e, a);
	o.glow = vec4<f32>(clamp(e * soft.p.w * su.look.z * 0.6, vec3<f32>(0.0), vec3<f32>(1.0)), a);
	return o;
}
