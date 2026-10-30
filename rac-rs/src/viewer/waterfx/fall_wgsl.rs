//! [PC] The waterfall sheets' shader (WGSL: `vs_fall`, `fs_fall` and their helpers), moved out of `shader.rs` unchanged (its size cap).
//! `WaterFx::new` appends it right after `SHADER`, so the assembled water module is the same text as before the move.

pub(super) const FALL_WGSL: &str = r#"// [PC] Waterfalls (`WaterFx::sheet_falls`, `fs_fall`), drawn over the shaded picture. Pokitaru's sheets replace retail's own
// (additive, glassy) draw; Novalis's steep strips and Batalia's steep river quads are drawn over theirs. The body: aerated
// white water in three layers of falling streaks (FALL_V units a second down the sheet, FALL_L1..3 cells a unit along,
// FALL_ACROSS cells a uv unit across), thin and see-through between them (FALL_THIN), torn into strands where retail's own
// cover fades out at a ribbon's side (FALL_FRAY); a glassy bright band where it bends over the lip (FALL_LIP); white froth
// within FALL_FOOT units above a churning foot, and foam where the sheet spreads flat over the water. At each foot: a
// boiling foam disc drifting outward (FALL_BOIL), mist rising and drifting away (FALL_MIST) and spray drops thrown out in
// arcs (FALL_DROP_*).
const FALL_ACROSS: f32 = 40.0;
const FALL_V: f32 = 7.0;
const FALL_L1: f32 = 0.22;
const FALL_L2: f32 = 0.6;
const FALL_L3: f32 = 1.6;
const FALL_THIN: f32 = 0.35;
const FALL_THICK: f32 = 0.97;
const FALL_AER: vec2<f32> = vec2<f32>(0.3, 0.75);
const FALL_FRAY: f32 = 0.3;
const FALL_LIP: f32 = 0.35;
const FALL_FOOT: f32 = 3.0;
const FALL_MIST: f32 = 0.1;
const FALL_MIST_LIFE: f32 = 4.5;
const FALL_MIST_RISE: f32 = 5.0;
const FALL_MIST_DRIFT: f32 = 3.0;
const FALL_BOIL: f32 = 0.8;
const FALL_BOIL_RATE: f32 = 0.35;
const FALL_DROP_LIFE: f32 = 0.9;
const FALL_DROP_SPEED: f32 = 2.0;
const FALL_DROP_UP: f32 = 3.5;
const FALL_DROP_BLUR: f32 = 0.02;
const FALL_DROP_A: f32 = 0.5;
const FALL_G: f32 = 9.8;
const FALL_NEAR: vec2<f32> = vec2<f32>(3.0, 10.0);
const FALL_WHITE: vec3<f32> = vec3<f32>(0.88, 0.95, 1.0);
const FALL_CLEAR: vec3<f32> = vec3<f32>(0.5, 0.68, 0.72);
const FALL_TEX_V: f32 = 0.45;
const FALL_TEX_GAIN: f32 = 1.3;
const FALL_GLOW: f32 = 0.15;
// [PC] Pokitaru's sheets (`shape_sheets`): retail's own flat look FALL_BRINK units (from..to) above the brink; thick smooth
// water bending over it (FALL_TONGUE colour, FALL_GLASS cover, FALL_SHEEN streaks of light, a FALL_CREST bright crest on
// the bend) that tears into white by FALL_ONSET units below it.
const FALL_BRINK: vec2<f32> = vec2<f32>(0.6, 2.0);
const FALL_TONGUE: vec3<f32> = vec3<f32>(0.42, 0.72, 0.7);
const FALL_GLASS: f32 = 0.7;
const FALL_SHEEN: f32 = 0.35;
const FALL_CREST: f32 = 0.45;
const FALL_ONSET: f32 = 1.2;

struct FO { @builtin(position) pos: vec4<f32>, @location(0) world: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) col: vec4<f32>, @location(3) @interpolate(flat) kind: vec3<f32>, @location(4) shape: vec4<f32> };
// Retail's waterfall texture (`fall_fx_tex`; a blank one elsewhere).
@group(1) @binding(0) var fall_tex: texture_2d<f32>;
@group(1) @binding(1) var fall_smp: sampler;

// A sheet vertex as retail draws it (n.x = 0; n.z = 1: its fall runs along u, not v), or a corner (uv -1..1) of one of a
// foot's quads at p (n.x = 1 mist puff, 2 spray drop, 3 boil disc; n.y its size, n.z its seed 0..1). Puffs and drops move
// here, looping on the clock: puffs rise and drift away growing, drops fly out and fall back, stretched along their motion.
// sh: a sheet vertex's shape (`shape_sheets`; z = 0 where unknown).
@vertex fn vs_fall(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>, @location(4) sh: vec4<f32>) -> FO {
	var q = p;
	var o: FO;
	o.col = c;
	let t = w.eye.w;
	if (n.x > 2.5) {
		q = p + vec3<f32>(uv * n.y, 0.03);
	} else if (n.x > 0.5) {
		let d = normalize(p - w.eye.xyz);
		var r = normalize(cross(d, vec3<f32>(0.0, 0.0, 1.0)) + vec3<f32>(1e-4, 0.0, 0.0));
		var up = cross(r, d);
		let ang = n.z * 6.2831 * 7.0;
		let dir = vec3<f32>(cos(ang), sin(ang), 0.0);
		if (n.x < 1.5) {
			let ph = fract(t / FALL_MIST_LIFE + n.z);
			let size = n.y * (0.6 + 0.8 * ph);
			q = p + dir * (0.5 + FALL_MIST_DRIFT * ph) + vec3<f32>(0.0, 0.0, FALL_MIST_RISE * ph) + (r * uv.x + up * uv.y) * size;
			o.col = vec4<f32>(c.rgb, sin(ph * 3.14159));
		} else {
			let life = FALL_DROP_LIFE * (0.7 + 0.6 * fract(n.z * 13.1));
			let ph = fract(t / life + n.z);
			let tt = ph * life;
			let v0 = dir * FALL_DROP_SPEED * (0.5 + fract(n.z * 7.7)) + vec3<f32>(0.0, 0.0, FALL_DROP_UP * (0.6 + 0.8 * fract(n.z * 3.3)));
			let at = p + v0 * tt + vec3<f32>(0.0, 0.0, -0.5 * FALL_G * tt * tt);
			let vel = v0 + vec3<f32>(0.0, 0.0, -FALL_G * tt);
			let ax = vel - d * dot(vel, d);
			let al = length(ax);
			if (al > 1e-3) {
				up = ax / al;
				r = normalize(cross(d, up));
			}
			q = at + r * uv.x * n.y + up * uv.y * (n.y + al * FALL_DROP_BLUR);
			// Gone once back under the water.
			o.col = vec4<f32>(c.rgb, (1.0 - ph) * step(p.z - 0.1, at.z));
		}
	}
	o.pos = w.vp * vec4<f32>(q, 1.0);
	o.world = q;
	o.uv = uv;
	o.kind = n;
	o.shape = sh;
	return o;
}

// Value noise repeating every `per` cells.
fn tnoise(p: vec2<f32>, per: vec2<f32>) -> f32 {
	let i = floor(p);
	let f = p - i;
	let u = f * f * (3.0 - 2.0 * f);
	let i0 = i - per * floor(i / per);
	let i1 = i0 + vec2<f32>(1.0) - per * floor((i0 + vec2<f32>(1.0)) / per);
	return mix(mix(hash(i0), hash(vec2<f32>(i1.x, i0.y)), u.x), mix(hash(vec2<f32>(i0.x, i1.y)), hash(i1), u.x), u.y);
}

// One streak layer: `l` cells a unit along, scrolling down at `v` units a second (the scroll wraps every 64 cells, where
// the noise repeats, so the clock never grows its coordinates).
fn fall_layer(x: f32, z: f32, l: f32, v: f32, t: f32) -> f32 {
	return tnoise(vec2<f32>(x, z * l + fract(t * v * l / 64.0) * 64.0), vec2<f32>(4096.0, 64.0));
}

// Foam on the boil disc, drifting out from its middle (two phases of FALL_BOIL_RATE a second, crossfaded).
fn boil_foam(xy: vec2<f32>, o: vec2<f32>, t: f32) -> f32 {
	let f0 = fract(t * FALL_BOIL_RATE);
	let f1 = fract(t * FALL_BOIL_RATE + 0.5);
	let a0 = vnoise(xy * 1.5 - o * f0 * 2.5) * 0.6 + vnoise(xy * 3.4 - o * f0 * 3.0 + 7.0) * 0.4;
	let a1 = vnoise(xy * 1.5 - o * f1 * 2.5 + 13.0) * 0.6 + vnoise(xy * 3.4 - o * f1 * 3.0 + 21.0) * 0.4;
	let w0 = 1.0 - abs(2.0 * f0 - 1.0);
	return a0 * w0 + a1 * (1.0 - w0);
}

// A waterfall point under the water drawn in front of it at pixel c (the water pass's positions).
fn fall_under(c: vec2<i32>, p: vec3<f32>, below: f32) -> bool {
	let wp = wpos_at(c);
	return wp.w > 0.5 && p.z < wp.z - below && distance(wp.xyz, w.eye.xyz) < distance(p, w.eye.xyz);
}

// Premultiplied over the picture.
@fragment fn fs_fall(v: FO) -> @location(0) vec4<f32> {
	let p = v.world;
	let nrm = cross(dpdx(p), dpdy(p));
	// Across the fall in cells, and how many cells / units a pixel spans (before any branch: derivatives).
	let gx = select(v.uv.x, v.uv.y, v.kind.z > 0.5) * FALL_ACROSS;
	let fwx = fwidth(gx);
	let fwz = fwidth(p.z);
	let c = vec2<i32>(v.pos.xy);
	let zs = linear_depth(textureLoad(depth_ms, c, 0));
	let zw = linear_depth(v.pos.z);
	let t = w.eye.w;
	// Retail's texture (sheets with n.y = 2) at its own scroll, and again finer and faster on top.
	let k1 = textureSample(fall_tex, fall_smp, v.uv);
	let k2 = textureSample(fall_tex, fall_smp, v.uv * vec2<f32>(1.6, 1.3) + vec2<f32>(0.37, fract(t * FALL_TEX_V)));
	if (v.kind.x > 2.5) {
		// Only on the water it lies on.
		let r = length(v.uv);
		let wp = wpos_at(c);
		if (r > 1.0 || zw > zs * 1.004 + 0.05 || wp.w < 0.5 || abs(wp.z - p.z) > 1.5) { discard; }
		let foam = smoothstep(0.4, 0.65, boil_foam(p.xy, v.uv / max(r, 1e-3), t) + (1.0 - r) * 0.45) * (1.0 - smoothstep(0.5, 1.0, r));
		let a = foam * FALL_BOIL * (0.5 + 0.5 * (1.0 - r));
		return vec4<f32>(FALL_WHITE * a, a);
	}
	if (v.kind.x > 1.5) {
		if (zw > zs || fall_under(c, p, 0.05)) { discard; }
		let a = (1.0 - v.uv.x * v.uv.x) * (1.0 - v.uv.y * v.uv.y) * v.col.a * FALL_DROP_A * smoothstep(1.0, 3.0, zw);
		return vec4<f32>(FALL_WHITE * a, a);
	}
	if (v.kind.x > 0.5) {
		let r2 = dot(v.uv, v.uv);
		if (r2 > 1.0 || zw > zs || fall_under(c, p, 0.3)) { discard; }
		// Soft where it meets the level, and gone right in front of the eye.
		let soft = clamp((zs - zw) / 2.0, 0.0, 1.0) * smoothstep(1.5, 6.0, zw);
		let billow = vnoise(v.uv * 1.7 + vec2<f32>(v.kind.z * 73.0, -t * 0.45)) * 0.6 + vnoise(v.uv * 3.9 + vec2<f32>(t * 0.2, v.kind.z * 31.0 - t * 0.8)) * 0.4;
		let a = (1.0 - r2) * (1.0 - r2) * soft * smoothstep(0.25, 0.75, billow) * FALL_MIST * v.col.a;
		return vec4<f32>(FALL_WHITE * a, a);
	}
	if (zw > zs * 1.004 + 0.05) { discard; }
	// 0 on a flat stretch of the sheet, 1 where it falls straight down.
	let s = 1.0 - abs(nrm.z) / max(length(nrm), 1e-9);
	let fall = smoothstep(0.5, 0.85, s);
	let flat = 1.0 - smoothstep(0.35, 0.6, s);
	let lip = smoothstep(0.35, 0.6, s) * (1.0 - smoothstep(0.7, 0.9, s));
	var foot = 0.0;
	for (var k = 0; k < i32(w.rn.z); k++) {
		let ch = w.churn[2 * k];
		let h = p.z - ch.z;
		let dh = length(p.xy - ch.xy);
		foot = max(foot, w.churn[2 * k + 1].x * (1.0 - smoothstep(0.0, FALL_FOOT, h)) * step(-0.5, h) * (1.0 - smoothstep(ch.w, ch.w + 2.5, dh)));
	}
	// Streaks finer than about two pixels fade to their mean instead of shimmering.
	let d2 = 1.0 - smoothstep(0.35, 0.8, max(fwx * 0.6, fwz * FALL_L2));
	let d3 = 1.0 - smoothstep(0.35, 0.8, max(fwx * 1.5, fwz * FALL_L3));
	let n1 = fall_layer(gx * 0.25, p.z, FALL_L1, FALL_V, t);
	let n2 = mix(0.5, fall_layer(gx * 0.6 + 17.0, p.z, FALL_L2, FALL_V * 1.25, t), d2);
	let n3 = mix(0.5, fall_layer(gx * 1.5 + 5.0, p.z, FALL_L3, FALL_V * 1.5, t), d3);
	let tl = (luma(k1.rgb) * min(k1.a * 2.0, 1.0) * 0.55 + luma(k2.rgb) * min(k2.a * 2.0, 1.0) * 0.45) * FALL_TEX_GAIN;
	// Retail's texture is mostly dark (made to be added): its streaks only shade a mostly solid body.
	let dens = select(n1 * 0.5 + n2 * 0.33 + n3 * 0.17, clamp(0.45 + 0.55 * smoothstep(0.05, 0.6, tl) + 0.35 * (n1 - 0.5), 0.0, 1.0), v.kind.y > 1.5);
	let a = clamp(v.col.a / 1.4, 0.0, 1.0);
	let lit = clamp(v.col.rgb * 2.0, vec3<f32>(0.0), vec3<f32>(1.25)) * 0.95;
	let under = select(1.0, 0.3, fall_under(c, p, 0.05));
	let near = mix(0.35, 1.0, smoothstep(FALL_NEAR.x, FALL_NEAR.y, zw)) * under;
	if (v.shape.z > 0.5) {
		// Pokitaru's sheets, shaped per vertex (`shape_sheets`): steepness and distance d from the brink (< 0 on the water
		// running to it) vary smoothly over the sheet, so its facets never show.
		let ss = v.shape.x;
		let d = v.shape.y;
		let sfall = smoothstep(0.45, 0.8, ss);
		let sflat = (1.0 - smoothstep(0.25, 0.5, ss)) * max(smoothstep(FALL_BRINK.x, FALL_BRINK.y, -d), v.shape.w);
		// The water tears up unevenly: in some columns the white starts right at the brink, in others lower down.
		let tear = tnoise(vec2<f32>(gx * 0.3, fract(t * 0.004) * 64.0), vec2<f32>(4096.0, 64.0));
		let onset = smoothstep(-0.5, FALL_ONSET, d + (tear - 0.5) * FALL_ONSET);
		// Over the brink: streaks of light running down the smooth water (along d), a bright crest on the bend.
		let sheen = smoothstep(0.6, 0.85, fall_layer(gx * 0.7 + 3.0, -d, 0.35, FALL_V * 0.4, t)) * d2;
		let crest = exp(-d * d * 6.0) * (0.6 + 0.4 * n2);
		let aer = clamp(sfall * smoothstep(FALL_AER.x, FALL_AER.y, dens) * onset + foot * 0.9, 0.0, 1.0);
		let cov = smoothstep(0.0, FALL_FRAY, a * 1.25 - (1.0 - dens) * 0.9 * onset + 0.15 + (1.0 - onset) * 0.3);
		// Between the white streaks: the thick smooth water near the brink, thin clear water further down.
		let glass = FALL_TONGUE * lit * (0.85 + 0.15 * n1) + FALL_WHITE * lit * (sheen * FALL_SHEEN + crest * FALL_CREST);
		let thin = mix(glass, FALL_CLEAR * lit * (0.85 + 0.15 * dens), onset);
		let col = mix(thin, FALL_WHITE * lit * (0.85 + 0.15 * dens), aer);
		let alpha = cov * mix(mix(FALL_GLASS + 0.15 * sheen + 0.15 * crest, FALL_THIN, onset), FALL_THICK, aer) * near;
		let sheet = vec4<f32>(col * alpha + FALL_WHITE * lit * dens * cov * FALL_GLOW * onset * under, alpha);
		return mix(sheet, vec4<f32>(k1.rgb * k1.a * v.col.rgb * v.col.a, 0.0), sflat);
	}
	// Retail's cover (0 at a ribbon's outer side) torn by the streaks; the lip holds together.
	let cov = smoothstep(0.0, FALL_FRAY, a * 1.25 - (1.0 - dens) * 0.9 + 0.15 + lip * 0.3);
	let aer = clamp(fall * smoothstep(FALL_AER.x, FALL_AER.y, dens) + lip * 0.15 + foot * 0.9, 0.0, 1.0);
	// Lit as retail tints the sheet (0x40 = 1.0, `lit`); the same on every facet, so the sheet's corners do not show.
	var col = mix(FALL_CLEAR, FALL_WHITE, aer) * lit * (0.85 + 0.15 * dens) + FALL_WHITE * lip * FALL_LIP * smoothstep(0.5, 0.8, dens);
	var alpha = cov * mix(FALL_THIN, FALL_THICK, aer);
	// Thinner right in front of the eye (walking through a fall), and seen through the water below its surface.
	alpha *= near;
	// Premultiplied, plus a little of retail's glow; where the sheet spreads flat over the water, retail's own additive
	// glow (`fs_part`: texture x vertex colour, added times its alpha; the foam disc covers the foot).
	let sheet = vec4<f32>(col * alpha + FALL_WHITE * lit * dens * cov * FALL_GLOW * under, alpha);
	let retail = vec4<f32>(select(vec3<f32>(dens * 0.5), k1.rgb * k1.a, v.kind.y > 1.5) * v.col.rgb * v.col.a, 0.0);
	return mix(sheet, retail, flat);
}
"#;
