// [PC] Gemlik's star, alive. Group 2: x = seconds, y = strength (RAC_TUNE star_q). The retail star texture (group 1) is the
// surface's detail layer (warped by a slow flow so it boils) and the corona's base; granulation, filaments, spots, streamers and
// the eruptions (prominence loops and plasma bursts, each event seeded by its index) are procedural.
struct SP { t: vec4<f32> };
@group(2) @binding(0) var<uniform> sp: SP;

struct SOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(2) c: vec4<f32> };

@vertex fn vs_star(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>) -> SOut {
	var o: SOut;
	// Pinned to the far plane like vs_sky, so the world covers it exactly as the retail patch.
	o.pos = (u.sky_view_proj * vec4<f32>(p, 1.0)).xyww;
	o.uv = uv;
	o.c = c;
	return o;
}

const R: f32 = 0.625;          // disc radius in p = (uv - 0.5) * 2 (the retail texture's is 0.60)
const R_OUT: f32 = 0.97;      // everything is gone by here (the patch edge is at 1.0)
const TAU: f32 = 6.2831853;

// Integer hash (Jarzynski / Olano): the same on every GPU, no sin().
fn hash3(v0: vec3<i32>) -> vec3<f32> {
	var v = bitcast<vec3<u32>>(v0) * 1664525u + vec3<u32>(1013904223u);
	v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
	v = v ^ (v >> vec3<u32>(16u));
	v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
	return vec3<f32>(v >> vec3<u32>(8u)) * (1.0 / 16777216.0);
}

fn vnoise(p: vec3<f32>) -> f32 {
	let i = floor(p);
	let f = p - i;
	let w = f * f * (3.0 - 2.0 * f);
	let c = vec3<i32>(i);
	var a = 0.0;
	for (var k = 0; k < 8; k++) {
		let o = vec3<i32>(k & 1, (k >> 1) & 1, (k >> 2) & 1);
		let wt = mix(1.0 - w, w, vec3<f32>(o));
		a += hash3(c + o).x * wt.x * wt.y * wt.z;
	}
	return a;
}

// Animated 3D cellular noise (cell size 1): every feature point circles slowly inside its cell (own phase and speed per cell,
// periods 20..40 s at t in seconds), so the cells boil and nothing flips. worley27 = x nearest, y second nearest distance over the
// 3x3x3 cells; worley8 = nearest only over the 2x2x2 cells (points 0.28..0.72 of their cell, which is why those suffice).
fn orbit(c: vec3<i32>, t: f32, amp: f32) -> vec3<f32> {
	let h = hash3(c * vec3<i32>(1, 3, 7) + vec3<i32>(11, 5, 2));
	let om = 0.15 + 0.15 * fract(h.x * 7.3);
	return 0.5 + amp * sin(h * TAU + t * om * vec3<f32>(1.0, 1.3, 0.8));
}

fn worley27(p: vec3<f32>, t: f32) -> vec2<f32> {
	let b = floor(p);
	let q = p - b;
	let bi = vec3<i32>(b);
	var d1 = 9.0;
	var d2 = 9.0;
	for (var k = 0; k < 27; k++) {
		let o = vec3<i32>(k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1);
		let pt = vec3<f32>(o) + orbit(bi + o, t, 0.4) - q;
		let d = dot(pt, pt);
		if (d < d1) { d2 = d1; d1 = d; } else if (d < d2) { d2 = d; }
	}
	return vec2<f32>(sqrt(d1), sqrt(d2));
}

fn worley8(p: vec3<f32>, t: f32) -> f32 {
	let b = floor(p - 0.5);
	let q = p - b;
	let bi = vec3<i32>(b);
	var d1 = 9.0;
	for (var k = 0; k < 8; k++) {
		let o = vec3<i32>(k & 1, (k >> 1) & 1, (k >> 2) & 1);
		let pt = vec3<f32>(o) + orbit(bi + o, t, 0.22) - q;
		d1 = min(d1, dot(pt, pt));
	}
	return sqrt(d1);
}

// The photosphere's colour for heat h (0 dark red ... 1 white-hot); retail's flat orange sits near h = 0.52.
fn ramp(h: f32) -> vec3<f32> {
	let c0 = vec3<f32>(0.40, 0.05, 0.005);
	let c1 = vec3<f32>(0.76, 0.19, 0.012);
	let c2 = vec3<f32>(0.96, 0.44, 0.03);
	let c3 = vec3<f32>(1.00, 0.64, 0.14);
	let c4 = vec3<f32>(1.00, 0.90, 0.55);
	if (h < 0.25) { return mix(c0, c1, h / 0.25); }
	if (h < 0.5) { return mix(c1, c2, (h - 0.25) / 0.25); }
	if (h < 0.75) { return mix(c2, c3, (h - 0.5) / 0.25); }
	return mix(c3, c4, clamp((h - 0.75) / 0.25, 0.0, 1.0));
}

// The retail star texture (green channel = brightness) at disc point p, flow-warped: two copies drift along a slow noise flow and
// cross-fade (the classic flow map), so the retail granulation boils and streams without ever jumping.
fn retail_surface(p: vec2<f32>, t: f32) -> f32 {
	let q = p * (0.60 / R) * 0.5 + 0.5;
	let fl = vec2<f32>(vnoise(vec3<f32>(p * 2.6, t * 0.04)), vnoise(vec3<f32>(p * 2.6 + 7.0, t * 0.04))) - 0.5;
	let ph = t / 16.0;
	let f0 = fract(ph);
	let f1 = fract(ph + 0.5);
	let w1 = abs(2.0 * f0 - 1.0);
	let g0 = textureSampleLevel(tex, samp, q + fl * (f0 - 0.5) * 0.14, 0.0).g;
	let g1 = textureSampleLevel(tex, samp, q + fl * (f1 - 0.5) * 0.14, 0.0).g;
	return mix(g0, g1, w1);
}

// Photosphere colour at sphere point `nrm`, seen at depth mu (1 = disc centre, 0 = limb), t seconds; `fp` = pixel size in sphere
// units (octaves finer than ~3 px fade out so they never shimmer when the star is small).
fn photosphere(nrm: vec3<f32>, mu: f32, t: f32, fp: f32, p: vec2<f32>) -> vec3<f32> {
	let flow = vec3<f32>(t * 0.017, -t * 0.011, t * 0.014);
	let m1 = vnoise(nrm * 5.0 + flow);
	let m2 = vnoise(nrm * 13.0 - flow * 1.7);
	// Fine layers are carried by a warped copy of the sphere, so lanes bend and wander.
	let nw = nrm + (m1 - 0.5) * vec3<f32>(0.12, 0.09, -0.10);
	let wa = worley27(nrm * 3.8 + vec3<f32>(5.0, 1.0, 3.0), t);        // supergranules, ~8 across the disc
	let wb = worley8(nw * 11.0 + vec3<f32>(2.0, 8.0, 4.0), t * 1.3);   // granules, ~22 across
	let wc = worley8(nw * 27.0 + vec3<f32>(9.0, 3.0, 6.0), t * 1.9);   // fine granules, ~54 across
	let rd = vnoise(nw * 15.0 + vec3<f32>(flow.y * 2.0, 4.0, flow.x * 2.0));
	let rd2 = vnoise(nw * 34.0 - vec3<f32>(3.0, flow.z * 3.0, 1.0));
	let spt = vnoise(nrm * 3.4 + vec3<f32>(9.0, 3.0, 5.0) + flow * 0.3);
	let fcl = vnoise(nrm * 8.0 + vec3<f32>(1.0, 6.0, 2.0) - flow * 0.5);
	let fpe = fp / max(mu, 0.2);
	let wB = 1.0 - smoothstep(0.30, 0.55, fpe * 11.0);
	let wC = 1.0 - smoothstep(0.30, 0.55, fpe * 27.0);
	let wR = 1.0 - smoothstep(0.30, 0.55, fpe * 34.0);
	let cellA = 1.0 - smoothstep(0.0, 0.8, wa.x);
	let laneA = 1.0 - smoothstep(0.0, 0.25, wa.y - wa.x);
	let cellB = (1.0 - smoothstep(0.08, 0.62, wb)) * wB;
	let laneB = smoothstep(0.34, 0.6, wb) * wB;
	let cellC = (1.0 - smoothstep(0.05, 0.55, wc)) * wC;
	let laneC = smoothstep(0.36, 0.6, wc) * wC;
	// Thin bright and dark veins (ridged noise) and fine mottle.
	let vein = pow(1.0 - abs(2.0 * rd - 1.0), 4.0);
	let vein2 = (1.0 - abs(2.0 * rd2 - 1.0)) * wR;
	let mott = m1 * 0.55 + m2 * 0.45;
	let rt = retail_surface(p, t) - 0.43;
	var heat = 0.58 + 0.13 * (cellA - 0.45) - 0.05 * laneA
		+ 0.11 * (cellB - 0.4) - 0.05 * laneB
		+ 0.10 * (cellC - 0.4) - 0.05 * laneC
		+ 0.06 * (vein - 0.2) + 0.03 * (vein2 - 0.5)
		+ 0.24 * (mott - 0.5) + 0.62 * rt;
	// Sunspots (dark umbra inside a lighter penumbra) and faculae (bright patches that crowd the limb).
	heat -= 0.08 * smoothstep(0.69, 0.77, spt) + 0.14 * smoothstep(0.76, 0.84, spt);
	heat += 0.30 * smoothstep(0.55, 0.78, fcl) * (1.0 - mu) * (1.0 - mu);
	// Crisper contrast, then a hotter core and darker limb.
	heat = 0.5 + (heat - 0.5) * 1.15;
	let limb = mix(0.55, 1.0, pow(mu, 0.5));
	heat = heat * limb + 0.07 * (mu * mu);
	var disc = ramp(clamp(heat, 0.0, 1.0));
	// Limb reddening: the rim keeps its hue but loses green.
	disc = mix(disc * vec3<f32>(1.0, 0.82, 0.6), disc, smoothstep(0.0, 0.5, mu));
	return disc;
}

// Colour of hot plasma for heat h (0 deep orange-red ... 1 near white).
fn plasma(h: f32) -> vec3<f32> {
	let a = mix(vec3<f32>(0.95, 0.20, 0.03), vec3<f32>(1.0, 0.52, 0.10), smoothstep(0.0, 0.45, h));
	return mix(a, vec3<f32>(1.0, 0.90, 0.62), smoothstep(0.45, 1.0, h));
}

// Eruption slot k at limb pixel (r, th), t seconds: x = intensity, y = heat. A slot runs one event per window of P seconds
// (20..36 s, own length per slot, slots staggered): the event's index n seeds everything about it (kind, position on the limb,
// size, shape, start, life), so no two events repeat. Every event starts and ends at zero intensity.
// Kind 0 = a prominence loop that rises, arcs, twists and falls back along its strands; kind 1 = a flare that bursts out a fan of
// plasma jets which stretch, curve, detach and dissipate.
fn eruption(k: i32, r: f32, th: f32, t: f32) -> vec2<f32> {
	let fk = f32(k);
	let P = 20.0 + 16.0 * fract(fk * 0.618 + 0.1);
	let tw = t + 11.3 * fk;
	let n = floor(tw / P);
	let u0 = tw - n * P;
	let h = hash3(vec3<i32>(i32(n), k * 31 + 7, 1234));
	let g = hash3(vec3<i32>(i32(n), k * 31 + 8, 4321));
	let e = hash3(vec3<i32>(i32(n), k * 31 + 9, 999));
	let isloop = h.x < 0.55;
	let st = h.y * 0.3 * P;
	var life = mix(0.30, 0.85, h.z) * P;
	if (!isloop) { life = mix(0.17, 0.40, h.z) * P; }
	life = min(life, 0.96 * P - st);
	let tau = (u0 - st) / life;
	if (tau <= 0.0 || tau >= 1.0) { return vec2<f32>(0.0); }
	var dth = th - g.x * TAU;
	dth = dth - TAU * round(dth / TAU);
	let x = R * dth;
	let y = r - R;
	if (abs(x) > 0.55 || y > 0.4 || y < -0.08) { return vec2<f32>(0.0); }
	if (isloop) {
		let W = mix(0.05, 0.13, g.y);
		let H = mix(0.12, 0.30, g.z);
		let env = smoothstep(0.0, 0.14, tau) * (1.0 - smoothstep(0.72, 1.0, tau));
		let hh = max(H * sin(3.14159 * pow(tau, 0.7)), 0.004);
		let a = W * (0.85 + 0.5 * tau);
		// Lean and a travelling twist, both strongest mid-life.
		let yp = max(y, 0.0);
		let xp = x - (e.x - 0.5) * 0.7 * yp * sin(3.14159 * tau)
			+ 0.005 * sin(yp * 28.0 + t * 1.6 + e.y * 6.0) * smoothstep(0.0, 0.5 * H, yp);
		let rho = sqrt((xp / a) * (xp / a) + (yp / hh) * (yp / hh));
		let phi = atan2(yp / hh, xp / a);
		let d = (rho - 1.0) * sqrt(a * hh);
		let uu = abs(phi - 1.5708);
		let foot = 1.0 + 1.4 * pow(abs(cos(phi)), 4.0);
		// Knots of plasma that drain from the apex down both legs.
		let kn = 0.40 + 1.2 * vnoise(vec3<f32>(uu * 2.2 - t * 0.3, e.z * 13.0, t * 0.07));
		var s = 0.0;
		for (var j = 0; j < 5; j++) {
			let fj = f32(j);
			let nz = vnoise(vec3<f32>(phi * 2.5 + fj * 5.1, fj * 1.7 + e.x * 9.0, t * 0.22)) - 0.5;
			let dj = d - (fj - 2.0) * 0.012 * (0.6 + 0.8 * e.z) + 0.015 * nz;
			let sw = 0.007 + 0.0035 * f32(j & 1);
			s += exp(-(dj / sw) * (dj / sw)) * (0.6 + 1.2 * (nz + 0.5));
		}
		let glow = exp(-(d / 0.07) * (d / 0.07)) * 0.30 * (0.5 + 0.5 * kn);
		let I = env * (s * 0.5 * foot * kn + glow);
		return vec2<f32>(I, clamp(0.22 + 0.34 * (foot - 1.0) / 1.4 + 0.12 * I, 0.0, 0.68));
	}
	// A flare: a flash on the limb, then a fan of jets (own reach, lean, curl) that run out and dissipate.
	let env = smoothstep(0.0, 0.05, tau);
	let fl = exp(-(x * x / 0.004 + y * y / 0.0016)) * exp(-tau * 5.0) * env;
	var I = fl * 1.2;
	var hm = fl;
	let travel = 1.0 - pow(1.0 - tau, 1.8);
	let fade = 1.0 - smoothstep(0.35, 1.0, tau);
	let q = vec2<f32>(x, y);
	for (var j = 0; j < 5; j++) {
		let b = hash3(vec3<i32>(i32(n), k * 31 + 20 + j, 77));
		let ox = (b.x - 0.5) * 0.12;
		let V = mix(0.16, 0.34, b.y) * (0.8 + 0.5 * e.z);
		let lean = (b.x - 0.5) + (e.x - 0.5) * 0.8;
		let bpos = vec2<f32>(ox + lean * V * travel * 0.6 + 0.04 * sin(tau * 6.0 + b.z * 6.0) * travel, V * travel);
		var s0 = vec2<f32>(ox, -0.01);
		s0 = s0 + (bpos - s0) * smoothstep(0.4, 1.0, tau) * 0.8;
		let sg = bpos - s0;
		let sp1 = clamp(dot(q - s0, sg) / max(dot(sg, sg), 1e-6), 0.0, 1.0);
		let dist = length(q - s0 - sp1 * sg);
		let sig = (0.012 + 0.03 * tau) * (0.5 + sp1);
		let cl = length(q - bpos) / (0.05 + 0.07 * tau);
		I += 0.22 * exp(-cl * cl) * fade * (1.0 - fl);
		if (dist < 3.0 * sig) {
			let tb = 0.55 + 0.9 * vnoise(vec3<f32>(q * 38.0, t * 0.9 + b.z * 20.0));
			let jet = exp(-(dist / sig) * (dist / sig)) * mix(0.35, 1.0, sp1) * tb * fade;
			I += jet * 1.3;
			hm = max(hm, jet * (0.4 + 0.6 * sp1));
		}
	}
	return vec2<f32>(I * env, clamp(hm, 0.0, 1.0));
}

// The corona at p: straight alpha returned, colour through `col`. The retail halo (the texture's own alpha) carries a third of it;
// the rest is a radial glow broken into streamers that drift outward and shear, plus the eruptions.
fn corona(p: vec2<f32>, r: f32, t: f32, col: ptr<function, vec3<f32>>) -> f32 {
	let dd = max(r - R, 0.0);
	let th = atan2(p.y, p.x);
	let dir = vec2<f32>(cos(th), sin(th));
	let zz = t * 0.05 - dd * 2.2;
	let s1 = vnoise(vec3<f32>(dir * 3.0, zz));
	let s2 = vnoise(vec3<f32>(dir * 8.0, zz * 1.3 + 5.0));
	let s3 = vnoise(vec3<f32>(dir * 21.0, zz * 1.7 + 11.0));
	let streak = 0.5 * s1 + 0.3 * s2 + 0.2 * s3 - 0.5;
	let radial = exp(-dd / 0.07) * 0.62 + exp(-dd / 0.17) * 0.30 + exp(-dd / 0.45) * 0.09;
	let fade = 1.0 - smoothstep(0.20, R_OUT - R, dd);
	let hq = textureSampleLevel(tex, samp, p * (0.60 / R) * 0.5 + 0.5 + dir * (0.012 * s1 - 0.006), 0.0);
	let ha = mix(radial, hq.a, 0.75 * smoothstep(0.0, 0.03, dd));
	let spk = vnoise(vec3<f32>(dir * 34.0, zz * 2.0 + 3.0)) - 0.5;
	var a = ha * (0.66 + (0.4 + 2.2 * smoothstep(0.0, 0.14, dd)) * streak * 2.4 + 0.5 * spk * exp(-dd / 0.04)) * fade;
	var c = mix(vec3<f32>(1.0, 0.42, 0.05), vec3<f32>(0.62, 0.07, 0.02), smoothstep(0.0, 0.28, dd));
	c = mix(c, hq.rgb, 0.75 * smoothstep(0.0, 0.05, dd));
	c = mix(c, vec3<f32>(1.0, 0.6, 0.15), 0.5 * exp(-dd / 0.012));
	// Eruptions over it.
	var er = vec2<f32>(0.0);
	if (dd < 0.4) {
		for (var k = 0; k < 8; k++) {
			let ev = eruption(k, r, th, t);
			er = vec2<f32>(er.x + ev.x, max(er.y, ev.y));
		}
	}
	let ef = 1.0 - smoothstep(0.30, R_OUT - R, dd);
	let iv = min(er.x, 1.0) * ef;
	let hot = clamp(er.y + 0.15 * max(er.x - 1.0, 0.0), 0.0, 1.0);
	let oa = iv + a * (1.0 - iv);
	let oc = (plasma(hot) * iv + c * a * (1.0 - iv)) / max(oa, 1e-4);
	*col = oc;
	return clamp(oa, 0.0, 1.0);
}

@fragment fn fs_star(i: SOut) -> @location(0) vec4<f32> {
	let t = sp.t.x;
	let p = (i.uv - vec2<f32>(0.5)) * 2.0;
	let r = length(p);
	let aa = max(fwidth(r), 1e-4);
	// Everything past the corona is transparent: no square edge, no cost.
	if (r > R_OUT + aa) { discard; }
	let da = 1.0 - smoothstep(R - aa, R + aa, r);

	var disc = vec3<f32>(0.0);
	if (da > 0.0) {
		// A rotating sphere (tilted axis, ~1.5 rad per minute) of boiling granulation.
		let pn = p / R;
		let rr = min(dot(pn, pn), 1.0);
		let mu = sqrt(1.0 - rr);
		var nrm = vec3<f32>(pn.x, -pn.y, mu);
		let rot = t * 0.026;
		let cs = cos(rot); let sn = sin(rot);
		nrm = vec3<f32>(cs * nrm.x + sn * nrm.z, nrm.y, -sn * nrm.x + cs * nrm.z);
		let tilt = 0.45;
		let ct = cos(tilt); let st = sin(tilt);
		nrm = vec3<f32>(nrm.x, ct * nrm.y - st * nrm.z, st * nrm.y + ct * nrm.z);
		disc = photosphere(nrm, mu, t, aa / R, p);
	}
	var ccol = vec3<f32>(0.0);
	var ca = 0.0;
	if (da < 1.0) {
		ca = corona(p, r, t, &ccol);
		ca = ca * (1.0 - da);
	}

	// Compose (straight alpha).
	var a = da + ca;
	var rgb = (disc * da + ccol * ca) / max(a, 1e-4);
	// Dither (fixed pattern, +-0.5/255).
	let dh = hash3(vec3<i32>(vec3<f32>(i.pos.xy, 3.0))).x - 0.5;
	rgb += vec3<f32>(dh / 255.0);
	a = a * i.c.a * sp.t.y;
	return vec4<f32>(rgb * i.c.rgb, a);
}
