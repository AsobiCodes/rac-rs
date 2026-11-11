// [PC] Fire + weapon lights on the world surfaces (lane flit, 2026-11-05; user: "the flames should brighten up the shadows"; lane
// dlit, 2026-11-05: real point lights; lane dlit2, 2026-11-06: "i want the fire to be a true source of light bro").
// u.flit = the frame's surface light list (flit.rs, flitlist.rs): rows 2k = (centre, reach), 2k + 1 = (rgb x intensity, body radius
// + 8 x the shadow code (flit.rs `slot_code`: map slot 0 none / 1..4, a second slot and its blend share)) for up to 32 lights; FLIT_SHAPE = (on, debug, sheen, deep); FLIT_GRID = (origin x, y of the grid,
// 1 / cell, cells per side; 0 = no light at all: the surface as before, untouched); u.flit_grid = per world cell the bit mask of the
// lights reaching it, so a pixel loops over its cell's lights only.
// Each light is a uniform SPHERE EMITTER (fire light v2, lane dlit3, FIRE_LIGHT_V2.md 2: the flame volume, radius rs) on the
// surface's TEXTURE colour (its own albedo), added to its sun + ambient light before fog, so a surface in the sun's shadow next to a
// fire is lit as much as a sunlit one while black stays darker than sand: E = I / max(d, rs)^2 x F x (1 - (d / reach)^4)^2, F = the
// sphere's projected solid angle with the receiver's horizon (`flit_form`: plain cosine while the whole sphere is above the
// horizon, the exact partial form while it sinks below it, 0 once it is wholly below: the side turned away gets nothing), windowed smoothly
// to 0 at the reach (no edge). Colour: the light's own (a blackbody at the flame's temperature, flitcolour.rs); flit_deep > 0 (old
// look, knob only) deepens it past the body. Post.rs `fx_light` adds nothing for these rows on the level pass's pixels (no screen
// haze). A light of a fire with a cube shadow map reads it (flitshadow.wgsl `flit_shadow`).

// Twins of flit.rs SHAPE, GRID, SHAPE2 (= (wrap, roll, 0, 0): extra facing past a light's horizon, RAC_TUNE flit_wrap, 0 = none
// (v2); the soft roll-off of the added light, flit_roll, 0 = none (v2)).
const FLIT_SHAPE: u32 = 64u;
const FLIT_GRID: u32 = 73u;
const FLIT_SHAPE2: u32 = 74u;

// The facing for a surface without a normal (grass blades, oceans).
const FLIT_FACE0: f32 = 0.6;

// The bit mask of the lights reaching world point `w`'s grid cell (cells past the grid: the border cell, which holds them).
fn flit_cell(w: vec3<f32>) -> u32 {
	let g = u.flit[FLIT_GRID];
	let n = i32(g.w);
	let c = clamp(vec2<i32>(floor((w.xy - g.xy) * g.z)), vec2<i32>(0), vec2<i32>(n - 1));
	let i = u32(c.y * n + c.x);
	return u.flit_grid[i >> 2u][i & 3u];
}

// Sphere form factor (FIRE_LIGHT_V2.md 2.3): the irradiance of a uniform sphere / (pi x its radiance x s^2), c = n . l to the
// centre, s = sin of the sphere's angular radius (rs / d, 1 inside): c while the whole sphere is above the receiver's horizon, 0
// once it is wholly below, the exact partial-horizon form in between (Snyder 1996; Lagarde & de Rousiers 2014 4.7.2: the
// (c + s)^2 / 4s fit is 25 % off near s = 1, i.e. by the flames; inside the sphere it is (1 + c) / 2). Twin: flit.rs tests `form`.
fn flit_form(c: f32, s: f32) -> f32 {
	if (c >= s) { return c; }
	if (c <= -s) { return 0.0; }
	let s2 = s * s;
	let st = max(sqrt(max(1.0 - c * c, 0.0)), 1e-4);
	let x = sqrt(max(1.0 / s2 - 1.0, 0.0));
	let y = clamp(-x * c / st, -1.0, 1.0);
	let q = st * sqrt(max(1.0 - y * y, 0.0));
	return max((c * acos(y) - x * q) * s2 + atan2(q, x), 0.0) / (3.14159265 * s2);
}

// Lane rfire review: the share of `flit_light`'s last result that came from FIRE lights (a negative reach in the light's row,
// flit.rs `rows`; weapon / effect lights have a positive one): `flit_mix` takes the warm mix only by it.
var<private> flit_fire_s: vec3<f32>;

// The lights reaching world point `w` with normal `n0` (zero normal: a fixed mid facing), linear light (1 = full sun); the fire
// lights' part of it in `flit_fire_s`.
fn flit_light(w: vec3<f32>, n0: vec3<f32>) -> vec3<f32> {
	var acc = vec3<f32>(0.0);
	flit_fire_s = vec3<f32>(0.0);
	let sh = u.flit[FLIT_SHAPE];
	// RAC_TUNE flit_on=0: this pass off; post then keeps the whole old screen term (post.rs `fx_light`): never counted twice.
	if (sh.x <= 0.0) { return acc; }
	let ln = length(n0);
	var n = n0 / max(ln, 1e-6);
	if (dot(n, u.eye.xyz - w) < 0.0) { n = -n; }
	var m = flit_cell(w);
	// Each fire's shadow map is read once per pixel (all its lights share it; -1 = not read yet).
	var shc = vec4<f32>(-1.0);
	let wrap = u.flit[FLIT_SHAPE2].x;
	loop {
		if (m == 0u) { break; }
		let k = firstTrailingBit(m);
		m = m & (m - 1u);
		let a = u.flit[2u * k];
		let d = a.xyz - w;
		let d2 = dot(d, d);
		// (a.w < 0: a fire light, flit.rs `rows`; only its square is used below.)
		if (d2 >= a.w * a.w) { continue; }
		let c = u.flit[2u * k + 1u];
		// Shadow code (flit.rs `slot_code`): slot + 5 x second slot + 25 x 40 x the second's share (fire light v2 stage 6, flitsplit.rs).
		let code = floor(c.w / 8.0);
		let rs = c.w - 8.0 * code;
		let ci = u32(code);
		let slot = ci % 5u;
		let dist = sqrt(d2);
		let x = d2 / (a.w * a.w);
		let win = (1.0 - x * x) * (1.0 - x * x);
		var face = FLIT_FACE0;
		if (ln > 1e-4) {
			// The sphere's form factor down to the receiver's horizon (inside the sphere s = 1: lit from its own side only).
			let s = min(rs / max(dist, 1e-4) + wrap, 1.0);
			face = flit_form(dot(n, d / max(dist, 1e-4)), max(s, 1e-4));
		}
		let dd = max(dist, rs);
		var e = win * face / max(dd * dd, 1e-4);
		if (e <= 0.0) { continue; }
		if (FLS_ON && slot > 0u) {
			let j = min(slot - 1u, 3u);
			// FXC-safe: no dynamic write into the vector (X3500 + a forced unroll that fails, X3511); a masked select writes slot j.
			var sv = shc[j];
			if (sv < 0.0) {
				sv = flit_shadow(j, w, n, ln > 1e-4);
				shc = select(shc, vec4<f32>(sv), vec4<u32>(0u, 1u, 2u, 3u) == vec4<u32>(j));
			}
			// RAC_TUNE flsh_mean (FLIT_SHAPE2 .w > 0): the fire's power-weighted mean over its origins' maps.
			if (u.flit[FLIT_SHAPE2].w > 0.0) { sv = flit_fire_vis(j, sv, w, n, ln > 1e-4, &shc); }
			// Else a light near the bisector of its fire's two nearest origins blends their maps (share 0..0.5: no switch).
			let s2 = select((ci / 5u) % 5u, 0u, u.flit[FLIT_SHAPE2].w > 0.0);
			let w2 = f32(ci / 25u) * 0.025;
			if (s2 > 0u && w2 > 0.0) {
				let j2 = min(s2 - 1u, 3u);
				var sv2 = shc[j2];
				if (sv2 < 0.0) {
					sv2 = flit_shadow(j2, w, n, ln > 1e-4);
					shc = select(shc, vec4<f32>(sv2), vec4<u32>(0u, 1u, 2u, 3u) == vec4<u32>(j2));
				}
				sv = mix(sv, sv2, w2);
			}
			e *= sv;
		}
		var col = c.rgb;
		// RAC_TUNE flit_deep > 0 (the old look; v2 default 0: clear air does not redden light over metres): deeper past the body.
		if (sh.w > 0.0) {
			let pk = max(c.r, max(c.g, c.b));
			col = mix(c.rgb, c.rgb * c.rgb / max(pk, 1e-6), smoothstep(rs, rs + sh.w, dist));
		}
		acc += col * e;
		if (a.w < 0.0) { flit_fire_s += col * e; }
	}
	return acc;
}

// Fire light v2 stage 6 (flitsplit.rs, RAC_TUNE flsh_mean=1): the lit share of map j's fire = the power-weighted mean of the maps of
// its origins (FLIT_SHAPE2 .z: per map 2 bits, the first map of its fire; .w: per map 6 bits, 1 + 62 x its share, 0 = none; flit.rs
// `fire_code`), as the stage-3 reference weighs every flame tongue for every light of the fire. `sj` = map j's share (already read);
// each map is read once per pixel (`shc`, -1 = not yet).
fn flit_fire_vis(j: u32, sj: f32, w: vec3<f32>, n: vec3<f32>, has_n: bool, shc: ptr<function, vec4<f32>>) -> f32 {
	let s2 = u.flit[FLIT_SHAPE2];
	let cls = u32(s2.z);
	let wq = u32(s2.w);
	let cj = (cls >> (2u * j)) & 3u;
	var acc = 0.0;
	var ws = 0.0;
	for (var m = 0u; m < 4u; m++) {
		let q = (wq >> (6u * m)) & 63u;
		// q 0: no map; 1: a share-0 origin (weight 0, review N5: its map is not read).
		if (q <= 1u || ((cls >> (2u * m)) & 3u) != cj) { continue; }
		var sv = (*shc)[m];
		if (m == j) { sv = sj; }
		if (sv < 0.0) {
			sv = flit_shadow(m, w, n, has_n);
			*shc = select(*shc, vec4<f32>(sv), vec4<u32>(0u, 1u, 2u, 3u) == vec4<u32>(m));
		}
		let wt = f32(q - 1u);
		acc += wt * sv;
		ws += wt;
	}
	return select(sj, acc / max(ws, 1e-6), ws > 0.0);
}

// The lit colour `rgb` (gamma, as the level pass writes it) plus the fire light on the albedo `alb` (gamma texel x tint), added
// in linear light as the post composite does. World geometry takes the diffuse term alone (lane dlit3 worker 3 tried the objects'
// Schlick share here: the sand seen at grazing turned a white veil and the black fuel puddle vanished, pp/f_crash120_worldsheen.png;
// the burning wing of the crash cutscene is a moby and already takes the share).
fn flit_add(rgb: vec3<f32>, alb: vec3<f32>, w: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
	return flit_mix(rgb, alb, w, n, 0.0);
}

// `flit_add` for objects (mobys): plus the rough specular share of the broad flames, Schlick with F0 = the sheen (FLIT_SHAPE .z,
// flit_sheen) rising to FLIT_F90 at grazing (a rough surface's damped grazing Fresnel), added to the albedo the fire lights: a
// charred near-black hull (albedo ~0.01) still shows its fire as a dielectric does (stage 1 follow-up, lane dlit3: the rim-only
// sheen left the burning hulls black; tb5's flat 0.08 x a 5-10x too strong light made them a grey veil). Terrain keeps its texture
// alone (the black fuel puddle stays black).
fn flit_add_obj(rgb: vec3<f32>, alb: vec3<f32>, w: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
	return flit_mix(rgb, alb, w, n, u.flit[FLIT_SHAPE].z);
}

const FLIT_F90: f32 = 0.4;

fn flit_mix(rgb: vec3<f32>, alb: vec3<f32>, w: vec3<f32>, n: vec3<f32>, sheen: f32) -> vec3<f32> {
	if (u.flit[FLIT_GRID].w <= 0.0) { return rgb; }
	// RAC_TUNE flit_debug=1: the light alone (which surfaces take it); 2: the albedo it lights; 3: the fire shadow maps (map 0 red,
	// 1 green, 2 blue, 3 yellow: bright = lit, dim = shadowed, none = outside its far); 4: the normal the light sees; 5: map 0's
	// moment cube (flitshadow.wgsl `flit_debug_mom`).
	let dbg = u.flit[FLIT_SHAPE].y;
	if (dbg > 4.5) { return flit_debug_mom(w); }
	if (dbg > 3.5) { return n / max(length(n), 1e-6) * 0.5 + vec3<f32>(0.5); }
	if (dbg > 2.5) { return flit_debug_maps(w, n); }
	if (dbg > 1.5) { return alb; }
	let s = flit_light(w, n);
	if (dbg > 0.0) { return s + vec3<f32>(0.0, 0.0, 0.25); }
	if (all(s <= vec3<f32>(0.0))) { return rgb; }
	let g = vec3<f32>(2.2);
	var rim = 0.0;
	if (sheen > 0.0) {
		let nn = n / max(length(n), 1e-6);
		let nv = abs(dot(nn, normalize(u.eye.xyz - w)));
		rim = sheen + (max(FLIT_F90, sheen) - sheen) * pow(1.0 - clamp(nv, 0.0, 1.0), 5.0);
	}
	let a = pow(clamp(alb, vec3<f32>(0.0), vec3<f32>(1.0)), g) + vec3<f32>(rim);
	// The shoulder takes only what the light adds: base + (shoulder(base + add) - shoulder(base)) = base at add 0, so a surface
	// already brighter than the knee (white walls, snow) is not darkened where a light's reach / facing starts (a hard disc and
	// line before, fix tb4 B1); below the knee shoulder(base) = base: the same as the whole-colour shoulder (crash-site look kept).
	let base = pow(max(rgb, vec3<f32>(0.0)), g);
	// The roll-off (FLIT_SHAPE2 .y, RAC_TUNE flit_roll; 0 = none): the added light x 1 / (1 + its peak / roll), so the sand right by a
	// big fire keeps a gradient (yellow at the foot, orange further out) instead of every pixel past the knee going flat, while the
	// faint far tail (peak << roll) keeps its full strength.
	var add = a * s;
	let roll = u.flit[FLIT_SHAPE2].y;
	if (roll > 0.0) { add = add / (1.0 + max(add.r, max(add.g, add.b)) / roll); }
	let lit = base + (flit_shoulder(base + add) - flit_shoulder(base));
	// FLIT_SHAPE .x >= 2 (flit.rs `frame_rows`: the Fire row draws and a fire light is in the list): per pixel, the fire lights'
	// share of the light it takes (`flit_fire_s`) blends test build 7's result towards the warm mix (`flit_warm`), so a pixel lit by
	// weapon / effect lights alone keeps test build 7's shoulder exactly and nothing pops when a fire light enters or leaves.
	let ls = dot(s, FLIT_LUM);
	let fs = clamp(dot(flit_fire_s, FLIT_LUM) / max(ls, 1e-6), 0.0, 1.0);
	if (u.flit[FLIT_SHAPE].x > 1.5 && fs > 0.0) {
		// The fire lights' part of `add` (the same roll-off divisor).
		let as0 = a * s;
		let rd = select(1.0, 1.0 + max(as0.r, max(as0.g, as0.b)) / roll, roll > 0.0);
		let warm = flit_warm(base, add, a * flit_fire_s / rd, dot(flit_fire_s, FLIT_LUM), u.flit[FLIT_SHAPE].x - 2.0);
		return pow(max(mix(lit, warm, fs), vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
	}
	return pow(max(lit, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

// Lane rfire worker 3 (user on test build 7, "still kinda uneven"): the fire outshines the sun. Next to a big fire the sun's own
// light (and so its shadow edges) is a small share of what lights the ground; with the plain sum the sun-shadowed sand took the pure
// fire colour (deep orange) and the sunlit sand beside it kept its tan: every sun-shadow edge (and every base crease) cut the pool as
// a hue edge. `base0` (sun + ambient on the albedo, linear) x (1 - k), k = 1 - 1 / (1 + q x the FIRE light's luminance `lf`), q =
// RAC_TUNE flit_adapt (flit.rs `frame_rows`; 0 = the plain sum), and the luminance it took comes back in the fire-lit colour's hue
// (`fadd`: the fire lights' part of the added light): it follows the fire light's own smooth field, so the sun's edges fade into one
// warm pool near the fires and come back smoothly farther out. Lane rfire review: only on dark bases (k fades out between base
// luminance FLIT_ADAPT_LO and FLIT_ADAPT_HI: sunlit sand ~0.1 takes it whole, sunlit fur / snow from 0.3 not at all: they keep
// their own colour and only the shoulder warms them, never more red than without the adaptation); the re-hued base never brighter
// in any channel than max(base0's peak, FLIT_KNEE); and the whole new colour goes through the shoulder against the original base
// (continuous at k = 0), every channel at most 1 (or base0's own, if brighter).
const FLIT_ADAPT_LO: f32 = 0.18;
const FLIT_ADAPT_HI: f32 = 0.3;
fn flit_warm(base0: vec3<f32>, add: vec3<f32>, fadd: vec3<f32>, lf: f32, q: f32) -> vec3<f32> {
	var b = base0;
	let la = dot(fadd, FLIT_LUM);
	let lb = dot(base0, FLIT_LUM);
	let k = (1.0 - 1.0 / (1.0 + max(q, 0.0) * lf)) * (1.0 - smoothstep(FLIT_ADAPT_LO, FLIT_ADAPT_HI, lb));
	if (k > 0.0 && la > 1e-5) {
		b = base0 * (1.0 - k) + fadd * (k * lb / la);
		let pk = max(b.r, max(b.g, b.b));
		let cap = max(max(base0.r, max(base0.g, base0.b)), FLIT_KNEE);
		if (pk > cap) { b = b * (cap / pk); }
	}
	let c = base0 + (flit_shoulder_warm(b + add) - flit_shoulder_warm(base0));
	return min(c, max(base0, vec3<f32>(1.0)));
}

// flit_debug=3: per fire shadow map its colour x (0.15 + 0.85 x the lit share) inside its far, summed (fire light v2 stage 2).
fn flit_debug_maps(w: vec3<f32>, n0: vec3<f32>) -> vec3<f32> {
	var acc = vec3<f32>(0.0, 0.0, 0.1);
	if (!FLS_ON) { return acc; }
	let ln = length(n0);
	var n = n0 / max(ln, 1e-6);
	if (dot(n, u.eye.xyz - w) < 0.0) { n = -n; }
	for (var k = 0u; k < 4u; k++) {
		let m = u.flit[FLS_MAPS + k];
		// The map's colour without a local array (FXC: no dynamic array access in loops).
		let col = vec3<f32>(select(0.0, 1.0, k == 0u || k == 3u), select(0.0, 1.0, k == 1u || k == 3u), select(0.0, 1.0, k == 2u));
		if (m.w > 0.0 && distance(w, m.xyz) < m.w) { acc += col * (0.15 + 0.85 * flit_shadow(k, w, n, ln > 1e-4)); }
	}
	return acc;
}

// Above FLIT_KNEE (linear) EACH channel rolls off softly towards 1 (no channel clips), and the luminance the roll-off took away
// (FLIT_SPILL x it, `1 - exp`) lifts every channel towards white. Over-exposed warm light thus walks orange -> yellow -> cream -> white
// as film / sensors do: the red channel saturates first, green catches up, blue comes last (lane rfire, 2026-11-07, user on test
// build 7: "can we somehow prevent ratchet from going red"; the old peak-only roll-off mixed the still-orange colour with white of
// its peak: red 1, green and blue raised alike = salmon pink fur and flat orange sand). Twin: flit.rs tests `shoulder`. Only on the
// light's added share (`flit_mix`); below the knee every channel is unchanged.
const FLIT_KNEE: f32 = 0.7;
const FLIT_LUM: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
const FLIT_SPILL: f32 = 0.6;
fn flit_knee1(x: f32) -> f32 {
	let r = 1.0 - FLIT_KNEE;
	return select(x, FLIT_KNEE + r * (1.0 - exp(-(x - FLIT_KNEE) / r)), x > FLIT_KNEE);
}
// FLIT_SHAPE .x >= 2 (flit.rs `frame_rows`: a fire light in the list) = this warm-white shoulder (`flit_mix`); 1 = test build 7's
// peak shoulder (`flit_shoulder`), kept for frames with weapon / effect lights only (Fire Off).
fn flit_shoulder_warm(c: vec3<f32>) -> vec3<f32> {
	let s = vec3<f32>(flit_knee1(c.r), flit_knee1(c.g), flit_knee1(c.b));
	let lost = max(dot(c - s, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0);
	return s + (vec3<f32>(1.0) - s) * (1.0 - exp(-FLIT_SPILL * lost));
}

// Test build 7's shoulder: the peak rolls off and the colour mixes towards white of that peak.
fn flit_shoulder(c: vec3<f32>) -> vec3<f32> {
	let m = max(c.r, max(c.g, c.b));
	if (m <= FLIT_KNEE) { return c; }
	let r = 1.0 - FLIT_KNEE;
	let mc = FLIT_KNEE + r * (1.0 - exp(-(m - FLIT_KNEE) / r));
	return mix(c * (mc / m), vec3<f32>(mc), 1.0 - mc / m);
}
