// [PC] Fire simulation, the draw (firesim.rs): every live particle of the pool (firesim_step.wgsl) as a small soft billboard of its
// own, stretched along its motion on screen (a fast one is a streak, a rising one an upright lick), with a procedural flame outline
// (a teardrop torn by noise that changes as it ages) and the colour of its temperature: white-yellow core, orange, deep red, then thin
// dark smoke. One draw of 2 x pool instances: the first half the particles that are smoke now, the second the flames, so the flames'
// light lies over the smoke. Group 0 the level shader's block (view_proj), group 2 the soft-particle block of partfx.rs (scene depth).
struct U { view_proj: mat4x4<f32>, sky_view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;

struct P { a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32>, o: vec4<f32> };
// (firesim_step.wgsl SimU) look: (brightness, smoke gain, glow gain, tint mix floor)  look2: (streak s, smoke opacity, flame heat blob min size per metre of distance, size)
struct SimU { eye: vec4<f32>, fwd: vec4<f32>, right: vec4<f32>, up: vec4<f32>, ring: vec4<u32>, k: vec4<f32>, m: vec4<f32>, look: vec4<f32>, look2: vec4<f32> };
@group(1) @binding(0) var<uniform> su: SimU;
@group(1) @binding(1) var<storage, read> parts: array<P>;
@group(1) @binding(2) var ntex: texture_3d<f32>;
@group(1) @binding(3) var nsamp: sampler;

struct SoftU { p: vec4<f32>, n: vec4<f32>, l: array<vec4<f32>, 16> };
@group(2) @binding(0) var<uniform> soft: SoftU;
@group(2) @binding(1) var soft_depth: texture_depth_multisampled_2d;

const SMOKE_X: f32 = 2.4;
// Below this temperature a smoke particle is smoke; the others burn out around it.
const TS: f32 = 0.28;
// The soft fade against the scene (m).
const SOFT_D: f32 = 0.3;
// The smoke's albedo (a light grey: the fire and the scene stay visible through it) and how early in its life it fades out.
const SMOKE_GREY: f32 = 0.3;
const SMOKE_FADE0: f32 = 0.3;
const SMOKE_FADE1: f32 = 0.85;

// The depth rebuilt from w (viewer.rs world_clip, as firefx_vol.wgsl): the billboards' depth matches the level's.
fn world_clip(m: mat4x4<f32>, p: vec3<f32>) -> vec4<f32> {
	var c = m * vec4<f32>(p, 1.0);
	let wr = vec4<f32>(m[0][3], m[1][3], m[2][3], m[3][3]);
	let zr = vec4<f32>(m[0][2], m[1][2], m[2][2], m[3][2]);
	let ww = dot(wr.xyz, wr.xyz);
	if (ww > 1.0e-6) {
		let a = dot(zr.xyz, wr.xyz) / ww;
		let d = zr.xyz - a * wr.xyz;
		if (dot(d, d) <= 1.0e-10 * ww) {
			c.z = a * c.w + (zr.w - a * wr.w);
		}
	}
	return c;
}

fn kpick(k: u32, a: f32, b: f32, c: f32) -> f32 {
	return select(select(a, b, k == 1u), c, k == 2u);
}

// A blackbody-like ramp of temperature 0..1: deep red, orange, yellow, white-yellow.
fn blackbody(t: f32) -> vec3<f32> {
	let r = smoothstep(0.05, 0.45, t);
	let g = smoothstep(0.36, 0.86, t) * 0.92;
	let b = smoothstep(0.76, 1.05, t) * 0.72;
	return vec3<f32>(r, g, b);
}

// The game's own colour of the fire bends the ramp where it is not a warm orange (the upgraded Pyrocitor's green fire): the core
// stays light, the body takes the tint.
fn tinted(c: vec3<f32>, tint: vec3<f32>, t: f32) -> vec3<f32> {
	let m = max(max(tint.r, tint.g), max(tint.b, 1.0e-3));
	let tn = tint / m;
	let off = clamp((1.0 - dot(normalize(tint + vec3<f32>(1.0e-4)), normalize(vec3<f32>(1.0, 0.5, 0.15)))) * 6.0, 0.0, 1.0);
	let k = max(off, su.look.w);
	let luma = dot(c, vec3<f32>(0.3, 0.55, 0.15));
	let body = tn * luma * 1.35 + vec3<f32>(smoothstep(0.8, 1.05, t) * 0.35);
	return mix(c, body, k);
}

struct VOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) uv: vec2<f32>,
	// emission rgb (premultiplied by the fades), the flame's cover
	@location(1) @interpolate(flat) col: vec4<f32>,
	// smoke rgb, smoke opacity (0: a flame; then x = the particle's size for the heat pass)
	@location(2) @interpolate(flat) sm: vec4<f32>,
	// noise offset xy (seed), noise z (age), heat
	@location(3) @interpolate(flat) q: vec4<f32>,
	// the kind (0 flame, 1 jet, 2 burst)
	@location(4) @interpolate(flat) kind: u32,
};

const CORNER = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));

fn culled() -> VOut {
	var o: VOut;
	o.pos = vec4<f32>(2.0, 2.0, 0.5, 1.0);
	return o;
}

@vertex fn vs_part(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
	let pool = su.ring.z;
	if (pool == 0u) {
		return culled();
	}
	let p = parts[ii % pool];
	let phase = ii / pool;
	if (p.b.w <= 0.0) {
		return culled();
	}
	let flags = u32(p.c.w + 0.5);
	let kind = flags & 3u;
	let smoke = (flags & 4u) != 0u;
	let lick = (flags & 8u) != 0u;
	let temp = p.c.x;
	let fl = select(p.b.w, p.b.w / SMOKE_X, smoke);
	let now_smoke = smoke && temp < TS;
	// An ember (flag 16, firesim_step.wgsl `ember_of`): a spark drawn straight into the colour (with the smoke, phase 0).
	if ((flags & 16u) != 0u) {
		if (phase != 0u) {
			return culled();
		}
		return ember_vs(p, vi);
	}
	if ((phase == 0u) != now_smoke) {
		return culled();
	}
	let f = clamp(p.a.w / max(fl, 0.01), 0.0, 1.0);
	let life = clamp(p.a.w / p.b.w, 0.0, 1.0);
	// Size over its life: a flame lick narrows to its tip, a jet puff widens as it travels, a burst swells; smoke spreads.
	var size = p.c.y * kpick(kind, 1.0 - 0.55 * f, 0.8 + 0.7 * f, 0.6 + 1.1 * sqrt(f)) * mix(0.5, 1.0, smoothstep(0.0, 0.12, f));
	var o: VOut;
	if (now_smoke) {
		let g = clamp((p.a.w - 0.6 * fl) / max(p.b.w - 0.6 * fl, 0.01), 0.0, 1.0);
		size = p.c.y * (0.9 + 2.2 * g);
		let a = su.look2.y * su.look.y * (1.0 - smoothstep(TS - 0.1, TS, temp)) * (1.0 - smoothstep(SMOKE_FADE0, SMOKE_FADE1, life)) * p.d.w;
		o.sm = vec4<f32>(vec3<f32>(SMOKE_GREY, SMOKE_GREY * 0.96, SMOKE_GREY * 0.92) + blackbody(temp) * 0.12, a);
		o.col = vec4<f32>(0.0);
	} else {
		// Fades in at birth; the ones that do not smoke burn out as they cool, the smoking ones hand over to their smoke. The heat is
		// a max, not a sum (firesim_heat.wgsl): the retail strength only gates (a faint retail sprite is still a full flame).
		let fade = smoothstep(0.0, kpick(kind, 0.1, 0.02, 0.05), f) * smoothstep(TS - 0.06, TS + 0.12, temp) * (1.0 - smoothstep(0.88, 1.0, life)) * smoothstep(0.0, 0.2, p.d.w);
		// For the heat pass (firesim_heat.wgsl fs_heat): the retail tint and the particle's heat.
		// A level tongue's particle (flag 8, firesim_step.wgsl `level_lick`): its heat falls with its temperature to the power 1.5, so only
		// the fresh cores reach the yellow-white end and the cooling licks go orange then red (overlapping licks take the max, so a big
		// fire no longer reads as one flat yellow sheet).
		let hot = select(temp, temp * sqrt(temp), lick);
		o.col = vec4<f32>(p.d.xyz, fade * hot);
		// sm.x: the particle's own size (m) for the heat pass (a big fire's tongues and erosion scale with it); sm.w stays 0.
		o.sm = vec4<f32>(p.c.y, 0.0, 0.0, 0.0);
	}
	size *= su.look2.w;
	let to = p.a.xyz - su.eye.xyz;
	let dist = length(to);
	// A flame's heat blob is never thinner than a few pixels (look2.z: metres per metre of distance): the thin stream at the nozzle
	// and far fires still reach the heat buffer (at most 3 x its own size: a far fire does not swell).
	// (A level lick keeps its own size far away: LICK_FAR_X, more small tongues stay apart instead of a few swollen blobs.)
	if (!now_smoke) {
		size = max(size, min(su.look2.z * dist, select(3.0, LICK_FAR_X, lick) * size));
	}
	let nv = to / max(dist, 1.0e-3);
	// Near the eye a particle fades (no screen-filling blobs).
	let near = smoothstep(0.25, 1.0, dist);
	o.col *= near;
	o.sm.w *= near;
	if (o.col.w + o.sm.w <= 1.0e-4) {
		return culled();
	}
	// Stretched along its motion across the view (a fast one streaks), turned by its seed while slow.
	let v = p.b.xyz;
	let vp = v - nv * dot(v, nv);
	let sp = length(vp);
	let ang = (p.c.z - 0.5) * 2.4 + p.a.w * (p.c.z - 0.5) * 3.0;
	let r0 = su.right.xyz * cos(ang) + su.up.xyz * sin(ang);
	var ax = vp + r0 * 0.35;
	ax = ax - nv * dot(ax, nv);
	ax = ax / max(length(ax), 1.0e-4);
	let ay = cross(nv, ax);
	// (A level tongue's lick streaks less: its fast rise drew long vertical stripes.)
	let streak = select(min(sp * su.look2.x, size * select(1.5, 0.35, lick)), 0.0, now_smoke);
	var corner = CORNER;
	let c = corner[vi % 6u];
	o.uv = c;
	o.kind = kind;
	o.q = vec4<f32>(p.c.z * 7.31, p.c.z * 3.77, p.c.z * 5.0 + p.a.w * 0.9, temp);
	if (lick) {
		return lick_vs(p, o, c, nv, size);
	}
	let pos = p.a.xyz + ax * c.x * (0.5 * size + streak) + ay * c.y * (0.5 * size);
	o.pos = world_clip(u.view_proj, pos);
	return o;
}

// A level lick (flag 8) on the draw (lane nfire agent 18; user: the burning Novalis domes were "one big round fluffy blob", "we need sth
// more like" tall thin tongues licking up with dark gaps): not a round heat blob but an upright tongue, its quad stood along the world's
// up (leaning a little with its drift), LICK_W x its size across and from LICK_DOWN below it to LICK_UP above it; fs_heat cuts the
// flame shape out of it (`lick_heat`). kind + 8 tells the heat pass; q.z is the particle's age (the shape's noise scrolls up with it).
const LICK_W: f32 = 0.6;
const LICK_DOWN: f32 = 0.3;
const LICK_UP: f32 = 1.5;
// The most a lick's heat footprint grows far away (x its size) to stay a few pixels wide (a plain flame: 3).
const LICK_FAR_X: f32 = 1.3;
fn lick_vs(p: P, o0: VOut, c: vec2<f32>, nv: vec3<f32>, size: f32) -> VOut {
	var o = o0;
	let v = p.b.xyz;
	var up = vec3<f32>(0.0, 0.0, 1.0) + vec3<f32>(v.xy, 0.0) * 0.12;
	up = up - nv * dot(up, nv);
	let ul = length(up);
	// Seen from straight below or above: no upright to stand on, the plain round quad.
	let ay = select(su.up.xyz, up / max(ul, 1.0e-4), ul > 0.2);
	let ax = normalize(cross(ay, nv));
	let along = mix(-LICK_DOWN, LICK_UP, c.y * 0.5 + 0.5) * size;
	let pos = p.a.xyz + ax * c.x * LICK_W * size + ay * along;
	o.pos = world_clip(u.view_proj, pos);
	o.kind = o0.kind + 8u;
	o.q = vec4<f32>(p.c.z * 7.31, p.c.z * 3.77, p.a.w, p.c.x);
	return o;
}

// An ember (flag 16): a small bright spark, a few pixels at least, streaked a little along its flight; orange while hot, red as it
// cools, fading out over the last part of its life. Into the colour directly (fs_part, kind 16).
fn ember_vs(p: P, vi: u32) -> VOut {
	let to = p.a.xyz - su.eye.xyz;
	let dist = length(to);
	let nv = to / max(dist, 1.0e-3);
	let life = clamp(p.a.w / p.b.w, 0.0, 1.0);
	let t = p.c.x;
	let a = smoothstep(0.0, 0.06, life) * (1.0 - smoothstep(0.45, 1.0, life)) * smoothstep(0.15, 0.4, t) * smoothstep(0.4, 1.5, dist);
	if (a <= 1.0e-3) {
		return culled();
	}
	// At least ~2 px (1080 lines) wide.
	let size = max(p.c.y, 4.0 * su.fwd.w * dist / 1080.0);
	let v = p.b.xyz;
	let vp = v - nv * dot(v, nv);
	let sp = length(vp);
	var ax = select(su.up.xyz, vp / max(sp, 1.0e-4), sp > 1.0e-3);
	let ay = normalize(cross(nv, ax));
	ax = cross(ay, nv);
	let streak = min(sp * 0.03, 3.0 * size);
	var corner = CORNER;
	let c = corner[vi % 6u];
	var o: VOut;
	o.pos = world_clip(u.view_proj, p.a.xyz + ax * c.x * (0.5 * size + streak) + ay * c.y * 0.5 * size);
	o.uv = c;
	let hot = vec3<f32>(1.0, 0.55, 0.12);
	let cold = vec3<f32>(0.75, 0.1, 0.02);
	o.sm = vec4<f32>(mix(cold, hot, smoothstep(0.25, 0.85, t)) * EMBER_BRIGHT, a);
	o.col = vec4<f32>(0.0);
	o.kind = 16u;
	o.q = vec4<f32>(0.0);
	return o;
}
const EMBER_BRIGHT: f32 = 2.2;

// Depth fade where a particle meets a wall or the ground (as firefx.wgsl soft_fade, over SOFT_D).
fn soft_fade(pos: vec4<f32>) -> f32 {
	let n = soft.p.x;
	let f = soft.p.y;
	let zs = textureLoad(soft_depth, vec2<i32>(pos.xy), 0);
	let ls = n * f / (f - zs * (f - n));
	let lf = n * f / (f - pos.z * (f - n));
	return clamp((ls - lf) / SOFT_D, 0.0, 1.0);
}

struct FireOut { @location(0) c: vec4<f32>, @location(1) glow: vec4<f32> };

// The smoke (flames go through the heat buffer, firesim_heat.wgsl): a soft round puff, its rim torn a little by noise.
@fragment fn fs_part(i: VOut) -> FireOut {
	let uv = i.uv;
	let d = length(uv);
	if (i.kind == 16u) {
		// An ember: a hot dot with a soft edge, adding its light (a faint cover only).
		let e = (1.0 - smoothstep(0.15, 1.0, d)) * i.sm.w * soft_fade(i.pos);
		if (e <= 0.003) {
			discard;
		}
		var oe: FireOut;
		oe.c = vec4<f32>(i.sm.rgb * e, 0.1 * e);
		oe.glow = vec4<f32>(clamp(i.sm.rgb * e * 0.5, vec3<f32>(0.0), vec3<f32>(1.0)), 0.0);
		return oe;
	}
	let nz = textureSampleLevel(ntex, nsamp, vec3<f32>(uv * 0.45 + i.q.xy, i.q.z), 0.0);
	let nz2 = textureSampleLevel(ntex, nsamp, vec3<f32>(uv * 1.1 + i.q.yx, i.q.z * 1.7 + 0.5), 0.0);
	let rim = d + ((nz.a - 0.5) * 1.1 + (nz2.r - 0.5) * 0.6) * 0.5;
	let a = 1.0 - smoothstep(0.25, 0.9, rim);
	if (a <= 0.003 || i.sm.w <= 0.0) {
		discard;
	}
	let s = a * a * i.sm.w * soft_fade(i.pos);
	var o: FireOut;
	o.c = vec4<f32>(i.sm.rgb * s, s);
	o.glow = vec4<f32>(0.0, 0.0, 0.0, s);
	return o;
}
