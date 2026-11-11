//! Vegetation (viewer/vegfx.rs): the vegetation shader (WGSL, one string literal; moved out of vegfx.rs unchanged, with the burn part
//! of vegfx/burn.rs added (the burn map: vegfx/burnmap.rs): `veg_burn`, `veg_char`, `veg_glow`; every call of them and every term computed only for them sits behind the
//! uniform branch `veg.burn.head.x > 0.0` (vertex and fragment), so with no burnt area (count 0) the pre-burn code runs; a unit test in
//! burn.rs fails if a burn term is outside that branch).
//! It is appended to the level shader (`level_source(SHADER, ..)`) and can be replaced from disk (`RAC_WGSL_DIR`, `devtune::wgsl("veg", ..)`).

pub(super) const WGSL: &str = r#"
struct BurnU { head: vec4<f32>, look: vec4<f32>, map: array<vec4<f32>, 2>, jh: vec4<f32>, jet: array<vec4<f32>, 16> };
struct VegU { t: vec4<f32>, f: vec4<f32>, k: vec4<f32>, a: vec4<f32>, l: vec4<f32>, act: array<vec4<f32>, 48>, burn: BurnU };
@group(3) @binding(0) var<uniform> veg: VegU;
@group(3) @binding(1) var veg_burnmap: texture_2d<f32>;
struct VegOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) c: vec3<f32>, @location(2) w: vec3<f32>, @location(3) @interpolate(linear) fogf: f32, @location(4) gl: f32, @location(5) bn: vec4<f32> };

fn veg_yaw(v: vec2<f32>, a: f32) -> vec2<f32> {
	let s = sin(a);
	let c = cos(a);
	return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}

// Wind: a slow gust front rolling along the wind direction, a per-plant flutter, a little sideways sway (tip offset per m of height).
fn veg_wind(p: vec3<f32>, phase: f32) -> vec2<f32> {
	let t = veg.t.x;
	let dir = veg.t.yz;
	let along = dot(p.xy, dir);
	let gust = 0.5 + 0.5 * sin(t * 0.8 - along * 0.12);
	let flutter = sin(t * 2.6 + phase + along * 0.4);
	let cross = sin(t * 3.3 + phase * 1.3);
	return (dir * (0.25 + 0.75 * gust) * (0.7 + 0.3 * flutter) + vec2<f32>(-dir.y, dir.x) * cross * 0.2) * veg.t.w;
}

// Touch at a plant standing at c: (bend direction x strength 0..1, height kept 0..1). The hero's path (trail points joined
// into segments, strength along them) bends within a.z, away from it; a drawn moby bends within a.w of its footprint and
// shrinks the plants inside it.
fn veg_touch(c: vec3<f32>) -> vec3<f32> {
	var best = 0.0;
	var dir = vec2<f32>(0.0, 0.0);
	var keep = 1.0;
	let nt = i32(veg.a.x);
	for (var i = 0; i < nt; i++) {
		let q0 = veg.act[i];
		let q1 = veg.act[min(i + 1, nt - 1)];
		let e = q1.xy - q0.xy;
		let ee = dot(e, e);
		let u = select(0.0, clamp(dot(c.xy - q0.xy, e) / max(ee, 1e-6), 0.0, 1.0), ee > 1e-6);
		let v = c.xy - (q0.xy + e * u);
		let d = length(v);
		if (d > veg.a.z || abs(c.z - mix(q0.z, q1.z, u)) > 1.2) { continue; }
		let s = mix(q0.w, q1.w, u) * (1.0 - smoothstep(veg.a.z * 0.45, veg.a.z, d));
		if (s > best) { best = s; dir = select(veg.t.yz, v / d, d > 1e-3); }
	}
	let no = i32(veg.a.y);
	for (var j = 0; j < no; j++) {
		let q = veg.act[8 + j];
		let v = c.xy - q.xy;
		let d = length(v);
		if (d > q.w + veg.a.w || c.z - q.z > 1.0 || q.z - c.z > 2.5) { continue; }
		keep = min(keep, smoothstep(q.w * 0.75, q.w, d));
		let s = 1.0 - smoothstep(q.w, q.w + veg.a.w, d);
		if (s > best) { best = s; dir = select(veg.t.yz, v / d, d > 1e-3); }
	}
	// The Pyrocitor's flame on the ground (firefx_jet_grass.rs): away from the spot and along the stream; none while jh.x is 0.
	if (veg.burn.jh.x > 0.0) {
		let nj = min(i32(veg.burn.jh.x), 8);
		for (var k = 0; k < nj; k++) {
			let a = veg.burn.jet[2 * k];
			let b = veg.burn.jet[2 * k + 1];
			let v = c.xy - a.xy;
			let d = length(v);
			if (d > a.w || abs(c.z - a.z) > 1.5) { continue; }
			let s = b.x * (1.0 - smoothstep(a.w * 0.3, a.w, d));
			let away = select(b.yz, v / d, d > 1e-3) + 0.8 * b.yz;
			if (s > best && dot(away, away) > 1e-6) { best = s; dir = normalize(away); }
		}
	}
	return vec3<f32>(dir * best, keep);
}

fn veg_ss(a: f32, b: f32, x: f32) -> f32 {
	let t = clamp((x - a) / (b - a), 0.0, 1.0);
	return t * t * (3.0 - 2.0 * t);
}

fn veg_hash(p: vec2<f32>) -> f32 {
	return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// Burning (vegfx/burn.rs, map vegfx/burnmap.rs): the state of a plant rooted at c on the burn map (read a little off the root: a
// ragged edge), lighting up to 0.3 s after its texel (its own hash): x = char 0..1 (black from the tip down; falls back while it
// regrows), y = tip flame 0..1 (while it burns), z = faint ember 0..1 (a few plants, after the flames), w = a 0..1 number of this
// plant (its bend direction). Called only while the map has a live burn (head.x > 0); off the map or unburnt: all zero.
fn veg_burn(c: vec3<f32>) -> vec4<f32> {
	let burn = veg.burn.head.y;
	let regrow_at = veg.burn.head.y + veg.burn.head.z;
	let hh = veg_hash(c.xy * 3.7);
	let jit = vec2<f32>(veg_hash(c.yx * 7.1 + vec2<f32>(3.0, 1.0)), veg_hash(c.xy * 5.9 + vec2<f32>(11.0, 2.0))) - vec2<f32>(0.5);
	let q = floor((c.xy + jit * 0.4 - veg.burn.map[0].xy) * veg.burn.map[0].z);
	if (q.x < 0.0 || q.y < 0.0 || q.x >= veg.burn.map[0].w || q.y >= veg.burn.map[0].w) { return vec4<f32>(0.0); }
	let m = textureLoad(veg_burnmap, vec2<i32>(q), 0);
	if (m.x <= 0.0 || abs(c.z - m.y) > 1.5) { return vec4<f32>(0.0); }
	let t = veg.burn.map[1].x - m.x - hh * 0.3;
	if (t <= 0.0 || t > regrow_at + veg.burn.head.w) { return vec4<f32>(0.0); }
	let p = veg_ss(0.0, burn, t);
	let g = veg_ss(0.0, 1.0, (t - regrow_at) / veg.burn.head.w);
	let cm = p * (1.0 - g);
	let he = veg_hash(c.yx * 5.3 + vec2<f32>(m.x * 13.0, 7.0));
	let fl = veg_ss(0.0, 0.25, t) * (1.0 - veg_ss(0.45 * burn, burn, t)) * (0.7 + 0.3 * sin(veg.t.x * 15.0 + hh * 40.0));
	let em = step(0.78, he) * veg_ss(0.3 * burn, 0.7 * burn, t) * (1.0 - veg_ss(burn, burn + 9.0, t)) * (0.65 + 0.35 * sin(veg.t.x * 2.3 + he * 30.0));
	return vec4<f32>(cm, fl, em, hh);
}

// The plant's colour at height fraction bn.w under its burn state: black from the tip down to the char front.
fn veg_char(col: vec3<f32>, bn: vec4<f32>) -> vec3<f32> {
	let front = 1.0 - bn.x * veg.burn.look.x;
	let m = veg_ss(front - 0.14, front, bn.w) * clamp(bn.x * 5.0, 0.0, 1.0);
	return mix(col, vec3<f32>(0.04, 0.034, 0.03), m);
}

// The light of the tip flames (orange, yellow at the very tip) and the faint embers; added after the fog, they glow through it.
fn veg_glow(bn: vec4<f32>) -> vec3<f32> {
	let hot = mix(vec3<f32>(1.0, 0.42, 0.07), vec3<f32>(1.0, 0.86, 0.42), veg_ss(0.78, 1.0, bn.w));
	return hot * (bn.y * veg_ss(0.5, 0.95, bn.w) * 1.6) + vec3<f32>(0.9, 0.2, 0.03) * (bn.z * veg_ss(0.3, 0.9, bn.w) * veg.burn.look.w);
}

// 1 = fully grown, 0 = gone: keep(d) falls from 1.1 at `full` to 0 at `end`; ranks just under keep shrink.
fn veg_fade(d: f32, full: f32, end: f32, rank: f32) -> f32 {
	let keep = 1.1 * clamp((end - d) / max(end - full, 0.001), 0.0, 1.0);
	return clamp((keep - rank) / 0.1, 0.0, 1.0);
}

// Flower colours (gamma space, as the GS look): white, yellow, purple, pink; flag 2 = the disc centre.
fn veg_petal(i: f32, flag: f32) -> vec3<f32> {
	if (flag > 1.5) {
		return select(vec3<f32>(1.0, 0.78, 0.2), vec3<f32>(0.85, 0.42, 0.1), i > 0.5 && i < 1.5);
	}
	var p = vec3<f32>(0.96, 0.94, 0.86);
	if (i > 0.5) { p = vec3<f32>(1.0, 0.84, 0.22); }
	if (i > 1.5) { p = vec3<f32>(0.66, 0.46, 0.95); }
	if (i > 2.5) { p = vec3<f32>(1.0, 0.55, 0.8); }
	return p;
}

fn veg_fog(i: VegOut) -> f32 {
	let d = distance(i.w, u.eye.xyz);
	var f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	if (i.fogf >= 0.0) { f = i.fogf / 255.0; }
	return f;
}

@vertex fn vs_veg_blade(@location(0) p: vec3<f32>, @location(1) root: vec2<f32>, @location(2) uv: vec2<f32>, @location(3) c: vec4<f32>, @location(4) side: vec2<f32>, @location(5) ia: vec4<f32>, @location(6) ib: vec4<f32>, @location(7) ic: vec4<f32>, @location(8) id: vec4<f32>) -> VegOut {
	var o: VegOut;
	let d = distance(ia.xyz, u.eye.xyz);
	let f = veg_fade(d, veg.f.x, veg.f.y, ib.z);
	let widen = 1.0 + max(d - veg.f.x * 0.5, 0.0) / max(veg.f.x, 1.0) * veg.k.y;
	let h = uv.y;
	let touch = veg_touch(ia.xyz);
	// Every burn term sits behind this uniform branch: with no burnt area (count 0) the code that runs is the pre-burn code, no burn maths.
	let burning = veg.burn.head.x > 0.0;
	var bn = vec4<f32>(0.0);
	if (burning) { bn = veg_burn(vec3<f32>(ia.xy + veg_yaw(root * ib.x, ia.w), ia.z)); }
	// Blades shrink away right at the lens (a low camera in a meadow looked through a wall of giant blades: Eudora, by the ship).
	var height = ib.y * f * touch.z * smoothstep(0.8, 2.4, d);
	// Burnt blades stay shorter (veg.burn.look.z of their height).
	if (burning) { height = height * (1.0 - (1.0 - veg.burn.look.z) * bn.x); }
	// Bent by touch: the blade follows an arc whose angle grows to `th` at the tip (its length kept); the wind calms on it.
	let th = min(length(touch.xy), 1.0) * 1.45;
	let bd = touch.xy / max(length(touch.xy), 1e-4);
	var l = root * ib.x * mix(0.4, 1.0, f) + p.xy * height + side * uv.x * widen * f;
	var wind = veg_wind(ia.xyz, ib.w) * height * h * h * (1.0 - 0.6 * th / 1.45);
	if (id.w > 0.5) {
		// Seaweed: no wind; the current waves it, a slow wave running up each ribbon.
		let t = veg.t.x;
		wind = vec2<f32>(sin(t * 0.9 + ib.w - h * 3.0), sin(t * 0.63 + ib.w * 1.7 - h * 2.3)) * 0.2 * height * h * (1.0 - 0.6 * th / 1.45);
	}
	// Burnt blades lean over, each its own way.
	if (burning) { wind = wind + vec2<f32>(cos(bn.w * 6.2832), sin(bn.w * 6.2832)) * (0.3 * height * h * h * bn.x); }
	l = veg_yaw(l, ia.w) + wind;
	var z = p.z * height - dot(wind, wind) * 0.5 / max(height, 0.05);
	if (th > 1e-3) {
		l = l + bd * height * (1.0 - cos(th * p.z)) / th;
		z = z - p.z * height + height * sin(th * p.z) / th;
	}
	// The tip flame: the top of a burning blade stretches up into a small flame (veg.burn.look.y m).
	if (burning) { z = z + bn.y * veg.burn.look.y * veg_ss(0.6, 1.0, h); }
	let wp = vec3<f32>(ia.x + l.x, ia.y + l.y, ia.z + z + id.x * l.x + id.y * l.y - 0.03);
	o.pos = world_clip(u.view_proj, wp);
	o.w = wp;
	o.uv = uv;
	var col = ic.rgb * c.rgb * (mix(veg.k.z, 1.0, h) + veg.k.w * h * h);
	// A touch more saturation than the ground (the blades stand out of it, as the painted grass would).
	let lg = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
	col = max(mix(vec3<f32>(lg), col, veg.k.x), vec3<f32>(0.0));
	if (c.a > 0.5) {
		col = veg_petal(ic.w, c.a) * clamp(id.z, 0.35, 1.25);
	}
	o.c = col;
	o.gl = id.z;
	o.bn = vec4<f32>(bn.xyz, h);
	o.fogf = fog_class(vec4<f32>(0.0, 0.0, 0.0, -1.0), o.pos.w);
	return o;
}

@fragment fn fs_veg_blade(i: VegOut) -> @location(0) vec4<f32> {
	let k = sun_baked(vec3<f32>(i.gl), i.w).x / max(i.gl, 1e-3);
	let fog = veg_fog(i);
	if (veg.burn.head.x > 0.0) {
		return vec4<f32>(mix(u.fog_colour.rgb, flit_add(veg_char(i.c, i.bn) * k, veg_char(i.c, i.bn), i.w, vec3<f32>(0.0)), fog) + veg_glow(i.bn) * (1.0 - fog), 1.0);
	}
	return vec4<f32>(mix(u.fog_colour.rgb, flit_add(i.c * k, i.c, i.w, vec3<f32>(0.0)), fog), 1.0); // [PC] fire light (flit.wgsl)
}

@vertex fn vs_veg_mesh(@location(0) p: vec3<f32>, @location(1) root: vec2<f32>, @location(2) uv: vec2<f32>, @location(3) c: vec4<f32>, @location(4) side: vec2<f32>, @location(5) ia: vec4<f32>, @location(6) ib: vec4<f32>, @location(7) ic: vec4<f32>, @location(8) id: vec4<f32>) -> VegOut {
	var o: VegOut;
	let d = distance(ia.xyz, u.eye.xyz);
	let f = veg_fade(d, veg.f.z, veg.f.w, ib.z);
	let touch = veg_touch(ia.xyz);
	// Every burn term sits behind this uniform branch (see vs_veg_blade): count 0 runs the pre-burn code.
	let burning = veg.burn.head.x > 0.0;
	var bn = vec4<f32>(0.0);
	if (burning) { bn = veg_burn(ia.xyz); }
	var s = ib.y * mix(0.15, 1.0, f) * select(0.0, 1.0, f > 0.0) * touch.z * smoothstep(1.0, 3.0, d);
	if (burning) { s = s * (1.0 - (1.0 - veg.burn.look.z) * bn.x); }
	// Wind and touch lean the top (sway weight c.a) away; the touch gives up to 0.6 m per m of height; a burnt plant leans over too.
	var lean = veg_wind(ia.xyz, ib.w) * 0.5 + touch.xy * 0.6;
	if (burning) { lean = lean + vec2<f32>(cos(bn.w * 6.2832), sin(bn.w * 6.2832)) * (0.5 * bn.x); }
	let wind = lean * c.a * c.a * max(p.z, 0.0) * s;
	let l = veg_yaw(p.xy * s, ia.w) + wind;
	let wp = vec3<f32>(ia.x + l.x, ia.y + l.y, ia.z + p.z * s - dot(wind, wind) * 0.3 - 0.03);
	o.pos = world_clip(u.view_proj, wp);
	o.w = wp;
	o.uv = uv;
	o.c = c.rgb * ic.rgb;
	o.gl = 1.0;
	o.bn = vec4<f32>(bn.xyz, clamp(p.z, 0.0, 1.0));
	o.fogf = fog_class(vec4<f32>(0.0, 0.0, 0.0, -1.0), o.pos.w);
	return o;
}

// A leaf cluster on a crown: model z along the crown's outward normal (id.xyz), rolled by ia.w round it; the leaves flutter,
// their tips most (c.a). Colour: the crown as drawn there (ic) x the leaf's own shade (c); id.w the crown's baked luminance.
@vertex fn vs_veg_leaf(@location(0) p: vec3<f32>, @location(1) root: vec2<f32>, @location(2) uv: vec2<f32>, @location(3) c: vec4<f32>, @location(4) side: vec2<f32>, @location(5) ia: vec4<f32>, @location(6) ib: vec4<f32>, @location(7) ic: vec4<f32>, @location(8) id: vec4<f32>) -> VegOut {
	var o: VegOut;
	let d = distance(ia.xyz, u.eye.xyz);
	let f = veg_fade(d, veg.l.x, veg.l.y, ib.z);
	let s = ib.x * mix(0.35, 1.0, f) * select(0.0, 1.0, f > 0.0) * smoothstep(0.6, 2.0, d);
	let n = normalize(id.xyz);
	let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.z) < 0.9);
	let a0 = normalize(cross(up, n));
	let b0 = cross(n, a0);
	let t1 = a0 * cos(ia.w) + b0 * sin(ia.w);
	let t2 = b0 * cos(ia.w) - a0 * sin(ia.w);
	let fl = sin(veg.t.x * 4.1 + ib.w * 3.0 + p.x * 9.0 + p.y * 7.0);
	let wob = (vec3<f32>(veg_wind(ia.xyz, ib.w), 0.0) * 0.18 + n * fl * 0.035) * c.a;
	let wp = ia.xyz + (t1 * p.x + t2 * p.y + n * p.z + wob) * s;
	o.pos = world_clip(u.view_proj, wp);
	o.w = wp;
	o.uv = uv;
	o.c = ic.rgb * c.rgb;
	o.gl = id.w;
	o.fogf = fog_class(vec4<f32>(0.0, 0.0, 0.0, -1.0), o.pos.w);
	return o;
}

@fragment fn fs_veg_mesh(i: VegOut) -> @location(0) vec4<f32> {
	let t = textureSample(tex, samp, i.uv);
	let a = clamp((t.a - 0.3) / 0.35, 0.0, 1.0);
	if (a <= 0.01) { discard; }
	let shade = sun_baked(min(i.c, vec3<f32>(255.0 / 128.0)), i.w);
	let fog = veg_fog(i);
	if (veg.burn.head.x > 0.0) {
		return vec4<f32>(mix(u.fog_colour.rgb, flit_add(veg_char(t.rgb, i.bn) * shade, veg_char(t.rgb, i.bn), i.w, vec3<f32>(0.0)), fog) + veg_glow(i.bn) * (1.0 - fog), a);
	}
	return vec4<f32>(mix(u.fog_colour.rgb, flit_add(t.rgb * shade, t.rgb, i.w, vec3<f32>(0.0)), fog), a); // [PC] fire light (flit.wgsl)
}
"#;
