// [PC] Sharp title / main menu (lane G2; viewer/hudsharp.rs). The front end's art quads (logo, PRESS START, button text, panel
// plates) are 512x448 PS2 textures drawn magnified (2.0 - 3.6 window pixels per texel). `fs_hud_sharp` reconstructs them with the
// edge-aware sampler of lane G's sharp HUD (retail/gscap/draw/gpu.wgsl `hd_tex_mag` and its `hq_*` helpers: keep the bodies
// the same; the tunables HD_* too), on straight-alpha Rgba8Unorm texels (rgb 0..1, alpha 0..1) instead of GS texels (rgb 0..255, alpha
// 0..128). Not magnified (a texel spans one pixel or less): the plain nearest sample, as the normal HUD pipeline takes it.
// Same blend states as `fs_hud` (alpha / additive): this shader returns the straight colour times the vertex colour.
// Per-quad texel rectangle: the quad's UV rectangle comes in the flat vertex light (u0, v0, u1, v1); the filter never reads a
// texel outside it (no bleed between the font sheet's glyph cells). A rectangle that leaves [0,1] is a tiled sprite: it wraps.
// Tunables are consts: try them with RAC_WGSL_DIR=<dir> (hud_sharp.wgsl in it replaces this file), no rebuild.

struct VOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) uv: vec2<f32>,
	@location(2) c: vec4<f32>,
	@location(4) @interpolate(flat) l: vec4<f32>,
};

@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

@vertex fn vs_hud_s(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(3) c: vec4<f32>, @location(4) l: vec4<f32>) -> VOut {
	var o: VOut;
	o.pos = vec4<f32>(p.xy, 0.0, 1.0);
	o.uv = uv;
	o.c = c;
	o.l = l;
	return o;
}

const HD_TLEN: f32 = 2.0;
const HD_TWIN: f32 = 2.2;
const HD_BC: f32 = 0.4;
const HD_B0: f32 = 0.12;
const HD_B1: f32 = 0.25;
const HD_C0: f32 = 0.25;
const HD_C1: f32 = 0.6;
const HD_NDEL: f32 = 0.5;
const HD_AAW: f32 = 1.0;
const HD_E0: f32 = 0.2;
const HD_E1: f32 = 0.5;
const HD_T0: f32 = 0.002;
const HD_T1: f32 = 0.02;
const HD_FPMIN: f32 = 0.02;
const HD_LOWC: f32 = 0.04;

const HN: u32 = 6u;
const HC: f32 = 2.0;
var<private> hq: array<vec4<f32>, 36>;
// The quad's texel rectangle (inclusive), whether it wraps, and the texture size.
var<private> g_rc: vec4<i32>;
var<private> g_wrap: bool;
var<private> g_dim: vec2<i32>;

// Alpha-premultiplied colour and alpha of a texel.
fn hd_q(t: vec4<f32>) -> vec4<f32> {
	return vec4<f32>(t.rgb * t.a, t.a);
}

// Bilinear in the 6 x 6 (index space 0..5; the point lies in 2..3).
fn hq_bil(x: vec2<f32>) -> vec4<f32> {
	let p = clamp(x, vec2<f32>(0.0), vec2<f32>(4.999));
	let fl = floor(p);
	let f = p - fl;
	let i = u32(fl.y) * HN + u32(fl.x);
	return mix(mix(hq[i], hq[i + 1u], f.x), mix(hq[i + HN], hq[i + HN + 1u], f.x), f.y);
}

// The smooth field at x averaged along the tangent t over +-l (tent of 7 taps).
fn hq_tan(x: vec2<f32>, t: vec2<f32>, l: f32) -> vec4<f32> {
	let h = t * (l / 3.0);
	return (hq_bil(x - 3.0 * h) + 2.0 * hq_bil(x - 2.0 * h) + 3.0 * hq_bil(x - h) + 4.0 * hq_bil(x) + 3.0 * hq_bil(x + h) + 2.0 * hq_bil(x + 2.0 * h) + hq_bil(x + 3.0 * h)) / 16.0;
}

// Catmull-Rom weights at t.
fn hq_cr(t: f32) -> vec4<f32> {
	let t2 = t * t;
	let t3 = t2 * t;
	return vec4<f32>(-0.5 * t3 + t2 - 0.5 * t, 1.5 * t3 - 2.5 * t2 + 1.0, -1.5 * t3 + 2.0 * t2 + 0.5 * t, 0.5 * t3 - 0.5 * t2);
}

// Bicubic (Catmull-Rom) of the 4 x 4 around the point (the 6 x 6's inner block), limited to the min..max of the 2 x 2 cell it is in
// so edges do not ring.
fn hq_bicubic(f: vec2<f32>) -> vec4<f32> {
	let wx = hq_cr(f.x);
	let wy = hq_cr(f.y);
	var r = vec4<f32>(0.0);
	for (var j = 0u; j < 4u; j++) {
		let b = HN + 1u + j * HN;
		let row = hq[b] * wx.x + hq[b + 1u] * wx.y + hq[b + 2u] * wx.z + hq[b + 3u] * wx.w;
		r += row * select(select(select(wy.x, wy.y, j == 1u), wy.z, j == 2u), wy.w, j == 3u);
	}
	let c = 2u * HN + 2u;
	let a = hq[c];
	let b = hq[c + 1u];
	let d = hq[c + HN];
	let e = hq[c + HN + 1u];
	return clamp(r, min(min(a, b), min(d, e)), max(max(a, b), max(d, e)));
}

fn hq_fetch(i0: vec2<i32>, j: u32) {
	var c = vec2<i32>(i0.x + i32(j % HN) - 2, i0.y + i32(j / HN) - 2);
	if (g_wrap) {
		c = ((c % g_dim) + g_dim) % g_dim;
	} else {
		c = clamp(c, g_rc.xy, g_rc.zw);
	}
	hq[j] = hd_q(textureLoad(tex, c, 0));
}

// Magnified sampling at texel coordinates `uv`, `fp` texels (u, v) per window pixel (both under 1). Straight rgb 0..1 and alpha 0..1.
// The inner 4 x 4 of the 6 x 6 texels around the point is fetched first; a flat 4 x 4 returns its texel, a 4 x 4 below HD_LOWC contrast
// is the plain bicubic, only the rest fetches the outer ring. Then:
//  1. a windowed structure tensor of (alpha, luma) tells the edge direction (tangent t, normal n) and how line-like it is;
//  2. the field is reconstructed bilinearly but averaged ALONG the tangent (length HD_TLEN * coherence): stair-steps of
//     diagonals and curves are smoothed away, the profile across the edge is kept;
//  3. where the neighbourhood has a strong, line-like contrast, each channel is re-projected onto an edge of HD_AAW window
//     pixels width: the signed distance to the mid-level contour is estimated from the slope of the smoothed field along n
//     and turned into smooth coverage, limited to the neighbourhood's own min..max (no overshoot);
//  4. smooth shaded art (no strong line-like contrast) is left as the smooth bicubic field.
fn hd_tex_mag(uv: vec2<f32>, fp: vec2<f32>) -> vec4<f32> {
	let pos = uv - vec2<f32>(0.5);
	let fl = floor(pos);
	let p = pos - fl + vec2<f32>(HC);
	let i0 = vec2<i32>(fl);
	let ci = 2u * HN + 2u;
	// The inner 4 x 4 first (all that bicubic needs); the outer ring only where the 4 x 4 has contrast.
	for (var k = 0u; k < 16u; k++) {
		hq_fetch(i0, (1u + (k >> 2u)) * HN + 1u + (k & 3u));
	}
	var lo4 = hq[ci - HN - 1u];
	var hi4 = lo4;
	for (var k = 1u; k < 16u; k++) {
		let c = hq[ci - HN - 1u + (k >> 2u) * HN + (k & 3u)];
		lo4 = min(lo4, c);
		hi4 = max(hi4, c);
	}
	let rg4 = hi4 - lo4;
	let rmax = max(max(rg4.x, rg4.y), max(rg4.z, rg4.w));
	if (rmax == 0.0) {
		let tx = hq[ci];
		return vec4<f32>(select(tx.rgb / tx.a, vec3<f32>(0.0), tx.a < 0.0001), tx.a);
	}
	if (rmax < HD_LOWC) {
		// no edge in reach: the structure tensor would be below its noise floor, the result is the plain bicubic
		let o = hq_bicubic(p - vec2<f32>(HC));
		return vec4<f32>(clamp(o.rgb / max(o.a, 0.04), vec3<f32>(0.0), vec3<f32>(1.0)), o.a);
	}
	for (var k = 0u; k < 36u; k++) {
		let x = k % HN;
		let y = k / HN;
		if (x >= 1u && x <= 4u && y >= 1u && y <= 4u) {
			continue;
		}
		hq_fetch(i0, k);
	}
	// 1. structure tensor of (alpha, luma): quad (qx, qy) spans texels (qx, qy)..(qx + 1, qy + 1)
	var jxx = 0.0;
	var jxy = 0.0;
	var jyy = 0.0;
	var wsum = 0.0;
	for (var qy = 0u; qy < HN - 1u; qy++) {
		for (var qx = 0u; qx < HN - 1u; qx++) {
			let i = qy * HN + qx;
			let a = hq[i];
			let b = hq[i + 1u];
			let c = hq[i + HN];
			let d = hq[i + HN + 1u];
			let gx = ((b - a) + (d - c)) * 0.5;
			let gy = ((c - a) + (d - b)) * 0.5;
			let lx = dot(gx.rgb, vec3<f32>(0.299, 0.587, 0.114));
			let ly = dot(gy.rgb, vec3<f32>(0.299, 0.587, 0.114));
			let w = max(0.0, HD_TWIN - length(p - vec2<f32>(f32(qx) + 0.5, f32(qy) + 0.5)));
			jxx += w * (gx.a * gx.a + lx * lx);
			jxy += w * (gx.a * gy.a + lx * ly);
			jyy += w * (gy.a * gy.a + ly * ly);
			wsum += w;
		}
	}
	jxx /= wsum;
	jxy /= wsum;
	jyy /= wsum;
	let en = jxx + jyy;
	let dm = sqrt((jxx - jyy) * (jxx - jyy) + 4.0 * jxy * jxy);
	let coh = dm / max(en, 1.0e-6) * smoothstep(HD_T0, HD_T1, en);
	var nv = vec2<f32>(jxx - jyy + dm, 2.0 * jxy);
	let nv2 = vec2<f32>(2.0 * jxy, jyy - jxx + dm);
	if (dot(nv2, nv2) > dot(nv, nv)) {
		nv = nv2;
	}
	nv = select(vec2<f32>(1.0, 0.0), normalize(nv), dot(nv, nv) > 1.0e-12);
	let tv = vec2<f32>(-nv.y, nv.x);
	// 2. smooth field along the tangent: at p and one step either side along the normal
	let l = HD_TLEN * coh;
	let v0 = hq_tan(p, tv, l);
	let vm = hq_tan(p - nv * HD_NDEL, tv, l);
	let vp = hq_tan(p + nv * HD_NDEL, tv, l);
	// 3. edge re-projection between the extremes of the 4 x 4 around p. Alpha is one channel of its own. Colour moves along the
	// line between the colours of the darkest and the brightest texel with ONE shared coverage (per-channel contours made hue noise).
	var lo = hq[ci - HN - 1u];
	var hi = lo;
	var cylo = lo.rgb;
	var cyhi = lo.rgb;
	var ylo = 1.0e9;
	var yhi = -1.0e9;
	for (var j = 0u; j < 16u; j++) {
		let c = hq[ci - HN - 1u + (j >> 2u) * HN + (j & 3u)];
		lo = min(lo, c);
		hi = max(hi, c);
		let y = dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
		if (y < ylo) {
			ylo = y;
			cylo = c.rgb;
		}
		if (y > yhi) {
			yhi = y;
			cyhi = c.rgb;
		}
	}
	// two-levelness of the 4 x 4: mean distance of a texel from the nearer extreme, relative to the range (0 = a pure edge between
	// two flat levels: glyph, outline; about 0.25 = shaded art with detail at texel size, which stays smooth)
	var my = 0.0;
	var ma = 0.0;
	for (var j = 0u; j < 16u; j++) {
		let c = hq[ci - HN - 1u + (j >> 2u) * HN + (j & 3u)];
		let y = dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
		my += min(y - ylo, yhi - y);
		ma += min(c.a - lo.a, hi.a - c.a);
	}
	let by = 1.0 - smoothstep(HD_B0, HD_B1, my / 16.0 / max(yhi - ylo, 1.0e-4));
	let ba = 1.0 - smoothstep(HD_B0, HD_B1, ma / 16.0 / max(hi.a - lo.a, 1.0e-4));
	let ch = smoothstep(HD_C0, HD_C1, coh);
	let ra = hi.a - lo.a;
	let ea = smoothstep(HD_E0, HD_E1, ra) * ch * ba;
	// colour is re-projected only where alpha is uniform; across an alpha edge it stays the alpha-weighted smooth field (no hue
	// shifts from the colour of transparent texels)
	let ec = smoothstep(HD_E0, HD_E1, yhi - ylo) * ch * by * (1.0 - smoothstep(0.1, 0.35, ra));
	let fpn = length(fp * nv);
	// away from edges: bicubic (sharper than bilinear, same smoothness); along edges: the tangent-smoothed field
	let wb = smoothstep(0.0, HD_BC, coh);
	var o = mix(hq_bicubic(p - vec2<f32>(HC)), v0, wb);
	// colour (premultiplied) along the line between the extreme colours
	var pcol = o.rgb;
	{
		let y0 = dot(v0.rgb, vec3<f32>(0.299, 0.587, 0.114));
		let g = abs(dot(vp.rgb - vm.rgb, vec3<f32>(0.299, 0.587, 0.114))) / (2.0 * HD_NDEL) * fpn;
		let ym = 0.5 * (ylo + yhi);
		var d = select(-8.0, 8.0, y0 > ym);
		if (g >= 0.0005) {
			d = clamp((y0 - ym) / g, -8.0, 8.0);
		}
		let t = smoothstep(0.0, 1.0, clamp(d / HD_AAW + 0.5, 0.0, 1.0));
		let ts = clamp((y0 - ylo) / max(yhi - ylo, 1.0e-4), 0.0, 1.0);
		pcol = max(pcol + ec * (t - ts) * (cyhi - cylo), vec3<f32>(0.0));
	}
	// alpha: smooth coverage on an edge of HD_AAW window pixels; the colour stays the straight colour of the smooth field (taken
	// before the alpha moves, so no division by the re-projected alpha)
	let scol = clamp(pcol / max(o.a, 0.04), vec3<f32>(0.0), vec3<f32>(1.0));
	var aout = o.a;
	{
		let g = abs(vp.a - vm.a) / (2.0 * HD_NDEL) * fpn;
		let am = 0.5 * (lo.a + hi.a);
		var d = select(-8.0, 8.0, v0.a > am);
		if (g >= 0.0005) {
			d = clamp((v0.a - am) / g, -8.0, 8.0);
		}
		let t = smoothstep(0.0, 1.0, clamp(d / HD_AAW + 0.5, 0.0, 1.0));
		aout = mix(o.a, lo.a + (hi.a - lo.a) * t, ea);
	}
	return vec4<f32>(scol, aout);
}

@fragment fn fs_hud_sharp(i: VOut) -> @location(0) vec4<f32> {
	// Everything that needs derivatives or the sampler first, in uniform control flow.
	let dimf = vec2<f32>(textureDimensions(tex));
	let uvt = i.uv * dimf;
	let fp = max(vec2<f32>(length(vec2<f32>(dpdx(uvt.x), dpdy(uvt.x))), length(vec2<f32>(dpdx(uvt.y), dpdy(uvt.y)))), vec2<f32>(HD_FPMIN));
	let base = textureSample(tex, samp, i.uv);
	if (max(fp.x, fp.y) >= 1.0) {
		return base * i.c;
	}
	// The quad's texel rectangle (inclusive) from its UV rectangle; a rectangle outside the texture is a tiled sprite (wraps).
	let ra = min(i.l.xy, i.l.zw) * dimf;
	let rb = max(i.l.xy, i.l.zw) * dimf;
	g_dim = vec2<i32>(dimf);
	g_wrap = any(ra < vec2<f32>(-0.01)) || any(rb > dimf + vec2<f32>(0.01));
	let a = vec2<i32>(floor(ra + vec2<f32>(0.001)));
	g_rc = vec4<i32>(a, max(vec2<i32>(ceil(rb - vec2<f32>(0.001))) - vec2<i32>(1), a));
	let t = hd_tex_mag(uvt, fp);
	return t * i.c;
}
