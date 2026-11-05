// [PC] Fire simulation, the step (firesim.rs): one thread per particle of the pool, once per display frame with the frame's game
// time step. A slot inside this frame's spawn window of the ring (ring.x head, ring.y count) takes a new particle from its emitter
// (binary search of the emitters' running ends); every other live particle moves: it relaxes towards its rise (buoyancy x its
// temperature) plus a time-varying 3D curl-noise wind (divergence free: tongues lick and break up, nothing bunches), cools, and stops
// on the scene's surfaces (the level pass's depth: a jet hitting the floor splashes and rolls along it). Hot flame turns into thin
// smoke on a share of the particles (flag 4); the others burn out.

// Particle: a = (position, age s), b = (velocity, life s; <= 0 dead), c = (temperature 0..1, size m, seed, kind | smoke flag 4),
// d = (retail tint, strength), o = (leash centre, leash radius m: the farthest its middle may go from there, see `leash`)
struct P { a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32>, o: vec4<f32> };
// Emitter (firesim.rs `SimEmit`). flame: a = (foot, height), b = (foot radius, 0, 0, 0), c = (tint, fade). jet slab: a = (P0, r0),
// b = (P1, r1), c = (tint, alpha at P0), d = (s at P0, s at P1, s at the chain's end, alpha at P1). burst: a = (centre, radius),
// b = (age 1 new .. 0, the retail sprite's footprint radius x 1.2 (0: the radius), 0, 0), c = (tint, strength). e = (size gain, jet start d0, life gain, a big fire's sub-emitter phase 0..1 (0: none)). n = (running end, kind, 0, 0).
// pa = (a.xyz of the same emitter last display frame, 1 when matched), pb = (b.xyz last frame: a jet slab's P1).
struct E { a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32>, e: vec4<f32>, n: vec4<u32>, pa: vec4<f32>, pb: vec4<f32> };
// eye (xyz, dt)  fwd (xyz, tan of half the vertical fov)  right (xyz, aspect)  up (xyz, clock s)  ring (head, spawn, pool, frame)
// k (near, far, emitters, collide 0/1)  m (rise gain, turbulence gain, jet flow m/s, drag gain)  look, look2: the draw's (firesim.wgsl)
struct SimU { eye: vec4<f32>, fwd: vec4<f32>, right: vec4<f32>, up: vec4<f32>, ring: vec4<u32>, k: vec4<f32>, m: vec4<f32>, look: vec4<f32>, look2: vec4<f32> };

@group(0) @binding(0) var<uniform> su: SimU;
@group(0) @binding(1) var<storage, read_write> parts: array<P>;
@group(0) @binding(2) var<storage, read> emits: array<E>;
@group(0) @binding(3) var ntex: texture_3d<f32>;
@group(0) @binding(4) var nsamp: sampler;
@group(0) @binding(5) var depth: texture_depth_multisampled_2d;

// A smoke particle lives SMOKE_X x its flame part (firesim.rs mirrors it).
const SMOKE_X: f32 = 2.4;
// A fireball's puffs stay bigger than the density alone gives (a burst of fine flecks reads as confetti).
const BURST_X: f32 = 2.2;
// SIZE RULE (user 2026-11-03, "the explosion on the ground is too big", "the rockets from RYNO create this massive oversized fire
// trail"): every flame stays inside its retail sprite's footprint x 1.2. Each particle is leashed to its emitter's footprint (`P.o`):
// the leash radius is the footprint minus the particle's own drawn radius and the composite's warp reach (firesim_heat.wgsl: about
// WARP_REACH x the particle's size), so buoyancy, the wind and the splash can never carry a flame past it; at the leash it burns out.
const WARP_REACH: f32 = 1.0;
// A burst puff's drawn radius per metre of size (firesim.wgsl vs_part: 0.5 x up to 1.7 as it ages).
const BURST_R: f32 = 0.85;
// A level tongue's particle: its size at most this share of the tongue's reach across (`level_lick`), and how fast it cools over its
// own top (per second), and its life over a plain flame's (a lick that lives longer rises higher and cools slower).
const LICK_LIFE: f32 = 1.4;
const LICK_SIZE: f32 = 0.6;
const LICK_TOP_COOL: f32 = 7.0;
// The wind's two noise tiles (m), how fast its pattern rises (tiles per second) and its scale.
const TILE0: f32 = 1.6;
const TILE1: f32 = 0.55;
const TILE2: f32 = 0.2;
const SCROLL: f32 = 0.35;
const CURL_NORM: f32 = 0.22;
// How far behind the surface a particle still counts as touching it (m, plus its own step), how much of the push into the surface
// bounces back, and the tangential friction (per second).
const THICK: f32 = 0.45;
const BOUNCE: f32 = 0.15;
const FRICTION: f32 = 1.6;

fn pcg(v: u32) -> u32 {
	let s = v * 747796405u + 2891336453u;
	let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
	return (w >> 22u) ^ w;
}

fn rnd(s: ptr<function, u32>) -> f32 {
	*s = pcg(*s);
	return f32(*s >> 8u) / 16777216.0;
}

fn unit3(s: ptr<function, u32>) -> vec3<f32> {
	let z = rnd(s) * 2.0 - 1.0;
	let a = rnd(s) * 6.2831853;
	let r = sqrt(max(1.0 - z * z, 0.0));
	return vec3<f32>(r * cos(a), r * sin(a), z);
}

// Per kind (0 flame, 1 jet, 2 burst).
fn kpick(k: u32, a: f32, b: f32, c: f32) -> f32 {
	return select(select(a, b, k == 1u), c, k == 2u);
}

// The share of a kind's particles that turn into smoke.
fn smoke_share(k: u32) -> f32 {
	return kpick(k, 0.28, 0.12, 0.55) * su.look.y;
}

// First emitter whose running end lies past spawn index k.
fn find(k: u32, n: u32) -> u32 {
	var lo = 0u;
	var hi = n;
	loop {
		if (lo >= hi) {
			break;
		}
		let mid = (lo + hi) / 2u;
		if (emits[mid].n.x > k) {
			hi = mid;
		} else {
			lo = mid + 1u;
		}
	}
	return min(lo, n - 1u);
}

fn perp(d: vec3<f32>) -> vec3<f32> {
	let h = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(d.z) > 0.9);
	return normalize(cross(d, h));
}

fn spawn_flame(e: E, s: ptr<function, u32>) -> P {
	let h = max(e.a.w, 0.05);
	let r = max(e.b.x, 0.02);
	let ang = rnd(s) * 6.2831853;
	let q = sqrt(rnd(s));
	let off = vec3<f32>(cos(ang), sin(ang), 0.0) * q * r * 0.85;
	let pos = e.a.xyz + off + vec3<f32>(0.0, 0.0, rnd(s) * 0.15 * h);
	let life = (0.35 + 0.2 * h) * (0.7 + 0.5 * rnd(s)) * e.e.z;
	// A big fire's sub-emitter (firesim_tongues.rs, phase e.w > 0): its tongue rises and falls on its own beat (two waves) and leans a
	// little, so the fire's top is a row of separate licks, not one round mass.
	var pulse = 1.0;
	var lean = vec3<f32>(0.0);
	if (e.e.w > 0.0) {
		let t = su.up.w;
		let ph = e.e.w * 6.2831853;
		pulse = max(1.0 + 0.45 * sin(t * 8.2 + ph) + 0.25 * sin(t * 19.0 + 2.7 * ph), 0.35);
		lean = vec3<f32>(cos(ph * 3.0 + t * 1.3), sin(ph * 3.0 + t * 1.3), 0.0) * (0.12 * h / life);
	}
	// Up at about the speed that carries it to the tip in its life, in towards the axis (the flame narrows), a little scatter.
	let vel = vec3<f32>(0.0, 0.0, 0.62 * h / life * (0.7 + 0.5 * rnd(s)) * pulse) + lean - off * (0.6 / life) + unit3(s) * 0.25;
	let temp = (1.0 - 0.42 * q * q) * (0.9 + 0.1 * rnd(s));
	let gw = max(su.look2.w, 0.1);
	let size = min(r * (0.85 + 0.5 * rnd(s)) * e.e.x, 1.6 * r / gw);
	// The footprint: the volume itself (foot radius r, height h: already the retail sprite's square x 1.2, firefx.rs) from its foot's
	// middle.
	let lr = max(max(h, r) - (0.5 * gw + WARP_REACH) * size, 0.3 * h);
	if (e.n.z == 2u) {
		// A level tongue drawn as flipbooks (firebook.rs): only its embers.
		return ember_of(e, s, pos);
	}
	if (e.n.z == 1u) {
		return level_lick(e, s, pos, vel, life, temp, size, gw);
	}
	return P(vec4<f32>(pos, 0.0), vec4<f32>(vel, life), vec4<f32>(temp, size, rnd(s), 0.0), vec4<f32>(e.c.xyz, e.c.w), vec4<f32>(e.a.xyz, lr));
}

// A level fire tongue's particle (firesim.rs `want_of`, n.z 1; lane nfire agent 17, user: the burning Novalis building was "one big flat
// bright-yellow mass", wider than its dome): its blob is no wider than a share of the tongue's reach across (e.b.y), it spawns well
// inside it, and its leash is an upright cylinder (`leash`: o.w < 0 = minus its radius, o.z = the top), so every lick stays inside its
// retail quad x 1.2 and the licks stand apart. The top is the particle's own (1..1.2 x its lick's height, never over the quad's side x 1.2): above it the flame cools away
// (no flat cut). Flag 8 (c.w): the draw gives it a heat that falls with its temperature (only the cores reach yellow-white) and a short
// streak.
fn level_lick(e: E, s: ptr<function, u32>, pos0: vec3<f32>, vel: vec3<f32>, life: f32, temp: f32, size0: f32, gw: f32) -> P {
	let lat = max(e.b.y, 0.05);
	let size = min(size0, LICK_SIZE * lat / gw);
	let lr = max(lat - (0.5 * gw + WARP_REACH) * size, 0.15 * lat);
	var off = pos0.xy - e.a.xy;
	let ol = length(off);
	if (ol > 0.8 * lr) {
		off *= 0.8 * lr / ol;
	}
	let pos = vec3<f32>(e.a.xy + off, pos0.z);
	let top = e.a.z + max(e.b.z, 0.1) * (1.0 + 0.2 * rnd(s)) - 0.25 * gw * size;
	if (rnd(s) < EMBER_P) {
		return ember_of(e, s, pos);
	}
	return P(vec4<f32>(pos, 0.0), vec4<f32>(vel, life * LICK_LIFE), vec4<f32>(temp, size, rnd(s), 8.0), vec4<f32>(e.c.xyz, e.c.w), vec4<f32>(e.a.xy, top, -lr));
}

// An ember of a level fire (lane nfire agent 18, user: the fire wants "small glowing embers/sparks rising"): EMBER_P of a level lick's
// particles become a tiny hot spark instead (flag 16, never smoke, no leash: o.w 0): born somewhere on the lick's lower two thirds, it
// flies up fast with a sideways kick, lives EMBER_LIFE s, the wind carries it and it cools from orange to red (firesim.wgsl ember_vs).
const EMBER_P: f32 = 0.025;
const EMBER_LIFE: f32 = 1.6;
const EMBER_SIZE: f32 = 0.04;
fn ember_of(e: E, s: ptr<function, u32>, pos0: vec3<f32>) -> P {
	let h = max(e.b.z, 0.1);
	let pos = pos0 + vec3<f32>(0.0, 0.0, rnd(s) * 0.6 * h);
	let ang = rnd(s) * 6.2831853;
	let vel = vec3<f32>(cos(ang), sin(ang), 0.0) * (0.4 + 1.2 * rnd(s)) + vec3<f32>(0.0, 0.0, 1.5 + 2.0 * rnd(s));
	let life = EMBER_LIFE * (0.5 + 0.8 * rnd(s));
	return P(vec4<f32>(pos, 0.0), vec4<f32>(vel, life), vec4<f32>(0.9 + 0.1 * rnd(s), EMBER_SIZE * (0.7 + 0.6 * rnd(s)), rnd(s), 16.0), vec4<f32>(e.c.xyz, e.c.w), vec4<f32>(pos, 0.0));
}
fn spawn_jet(e: E, s: ptr<function, u32>) -> P {
	let t = rnd(s);
	let axis = e.b.xyz - e.a.xyz;
	let len = length(axis);
	let dir = select(vec3<f32>(1.0, 0.0, 0.0), axis / max(len, 1.0e-4), len > 1.0e-4);
	let bx = perp(dir);
	let by = cross(dir, bx);
	let sd = mix(e.d.x, e.d.y, t);
	let rr = max(mix(e.a.w, e.b.w, t), 0.02);
	let al = mix(e.c.w, e.d.w, t);
	let ang = rnd(s) * 6.2831853;
	let q = sqrt(rnd(s));
	let side = (bx * cos(ang) + by * sin(ang)) * q;
	let pos = e.a.xyz + axis * t + side * rr * 0.55;
	let flow = su.m.z * (0.8 + 0.4 * rnd(s));
	// It lives until it would pass the chain's end (the stream's reach is the weapon's), a little longer to splash.
	let rest = max(e.d.z - sd, 0.0);
	let life = max(min(0.2 + 0.12 * rnd(s), rest / flow + 0.1), 0.05) * e.e.z;
	let vel = dir * flow + side * 0.9 + unit3(s) * 0.5;
	let temp = (1.0 - 0.38 * clamp(sd / max(e.d.z, 0.1), 0.0, 1.0)) * (1.0 - 0.3 * q * q);
	// A piece cut off from the stream starts d0 down it: its start fades in (as firefx_vol.wgsl jet_at `ends`).
	let d0 = e.e.y;
	let fin = select(1.0, clamp((sd - d0) / (0.1 + 0.8 * min(d0, 1.0)), 0.0, 1.0), d0 > 0.0);
	let gw = max(su.look2.w, 0.1);
	let size = min(rr * (0.8 + 0.5 * rnd(s)) * e.e.x, 1.4 * rr / gw);
	// The footprint: the stream's own tube from here to the chain's end (rest metres down it) and 1.2 x its radius around that.
	let lr = max(rest + 1.2 * rr - (0.75 * gw + WARP_REACH) * size, 0.4 * rr);
	return P(vec4<f32>(pos, 0.0), vec4<f32>(vel, life), vec4<f32>(temp, size, rnd(s), 1.0), vec4<f32>(e.c.xyz, al * fin), vec4<f32>(e.a.xyz + axis * t, lr));
}

fn spawn_burst(e: E, s: ptr<function, u32>) -> P {
	let r = max(e.a.w, 0.05);
	let age = clamp(e.b.x, 0.0, 1.0);
	let dir = unit3(s);
	let pos = e.a.xyz + dir * r * 0.35 * pow(rnd(s), 0.333);
	let life = (0.3 + 0.12 * r) * (0.7 + 0.6 * rnd(s)) * e.e.z;
	let vel = dir * (1.6 * r / life) * (0.4 + 0.6 * rnd(s)) * (0.5 + 0.5 * age);
	let temp = (0.55 + 0.45 * age) * (0.85 + 0.15 * rnd(s));
	let gw = max(su.look2.w, 0.1);
	let cap = select(r, e.b.y, e.b.y > 0.0);
	let size = min(r * (0.45 + 0.35 * rnd(s)) * min(e.e.x * BURST_X, 1.6), 0.35 * cap / gw);
	let lr = max(cap - (BURST_R * gw + WARP_REACH) * size, 0.25 * cap);
	return P(vec4<f32>(pos, 0.0), vec4<f32>(vel, life), vec4<f32>(temp, size, rnd(s), 2.0), vec4<f32>(e.c.xyz, e.c.w), vec4<f32>(e.a.xyz, lr));
}

fn spawn(e: E, s: ptr<function, u32>) -> P {
	var p: P;
	if (e.n.y == 1u) {
		p = spawn_jet(e, s);
	} else if (e.n.y == 2u) {
		p = spawn_burst(e, s);
	} else {
		p = spawn_flame(e, s);
	}
	let k = e.n.y;
	if (p.c.w < 15.5 && rnd(s) < smoke_share(k)) {
		p.b.w *= SMOKE_X;
		p.c.w += 4.0;
	}
	return p;
}

// Keeps a flame's middle within its leash (`P.o`): back onto the leash sphere, the outward speed taken away, and it cools fast (burns
// out at its footprint's edge instead of piling up there). Cool smoke drifts free.
fn leash(p: ptr<function, P>, dt: f32, free: bool) {
	if ((*p).o.w < 0.0) {
		leash_lick(p, dt, free);
		return;
	}
	let off = (*p).a.xyz - (*p).o.xyz;
	let dl = length(off);
	if (free || (*p).o.w <= 0.0 || dl <= (*p).o.w) {
		return;
	}
	let nrm = off / dl;
	(*p).a = vec4<f32>((*p).o.xyz + nrm * (*p).o.w, (*p).a.w);
	let v = (*p).b.xyz;
	(*p).b = vec4<f32>(v - nrm * max(dot(v, nrm), 0.0), (*p).b.w);
	(*p).c.x *= exp(-10.0 * dt);
}

// A level tongue's leash (`level_lick`): across, back inside the cylinder of radius -o.w about (o.x, o.y), the outward speed taken away,
// cooling fast; over its own top (o.z) it is not held (a held row would be a flat lid) but cools away as it rises on.
fn leash_lick(p: ptr<function, P>, dt: f32, free: bool) {
	if (free) {
		return;
	}
	let r = -(*p).o.w;
	let off = (*p).a.xy - (*p).o.xy;
	let dl = length(off);
	if (dl > r) {
		let nrm = off / dl;
		(*p).a = vec4<f32>((*p).o.xy + nrm * r, (*p).a.z, (*p).a.w);
		let v = (*p).b.xy;
		(*p).b = vec4<f32>(v - nrm * max(dot(v, nrm), 0.0), (*p).b.z, (*p).b.w);
		(*p).c.x *= exp(-10.0 * dt);
	}
	if ((*p).a.z > (*p).o.z) {
		(*p).c.x *= exp(-LICK_TOP_COOL * dt);
	}
}

fn pot(q: vec3<f32>) -> vec3<f32> {
	return textureSampleLevel(ntex, nsamp, q, 0.0).rgb * 2.0 - 1.0;
}

// The curl of the noise potential at q (tile units), per tile.
fn curl_at(q: vec3<f32>) -> vec3<f32> {
	let e = 1.0 / 32.0;
	let dx = pot(q + vec3<f32>(e, 0.0, 0.0)) - pot(q - vec3<f32>(e, 0.0, 0.0));
	let dy = pot(q + vec3<f32>(0.0, e, 0.0)) - pot(q - vec3<f32>(0.0, e, 0.0));
	let dz = pot(q + vec3<f32>(0.0, 0.0, e)) - pot(q - vec3<f32>(0.0, 0.0, e));
	return vec3<f32>(dy.z - dz.y, dz.x - dx.z, dx.y - dy.x) * (0.5 / e);
}

// The wind at p (m/s per unit gain): two octaves rising with the flames.
fn wind(p: vec3<f32>, t: f32) -> vec3<f32> {
	let rise = vec3<f32>(0.03 * t, 0.0, -SCROLL * t);
	return (curl_at(p / TILE0 + rise) + 0.5 * curl_at(p / TILE1 + 1.7 * rise + vec3<f32>(0.37, 0.71, 0.13)) + 0.35 * curl_at(p / TILE2 + 3.1 * rise + vec3<f32>(0.61, 0.23, 0.47))) * CURL_NORM;
}

fn lin(z: f32) -> f32 {
	let n = su.k.x;
	let f = su.k.y;
	return n * f / (f - z * (f - n));
}

// The world point of pixel px at view depth d (along fwd).
fn point_at(px: vec2<i32>, dim: vec2<f32>, d: f32) -> vec3<f32> {
	let uv = (vec2<f32>(px) + 0.5) / dim;
	let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
	let ray = su.fwd.xyz + su.right.xyz * (ndc.x * su.fwd.w * su.right.w) + su.up.xyz * (ndc.y * su.fwd.w);
	return su.eye.xyz + ray * d;
}

// Stops a particle at the scene's surface it went into (within THICK behind it, on screen): back onto the surface, the push into it
// gone (a little bounce), friction along it, so a stream that hits the floor rolls out along the floor.
fn collide(p: ptr<function, P>, dt: f32) {
	let to = (*p).a.xyz - su.eye.xyz;
	let vz = dot(to, su.fwd.xyz);
	if (vz < su.k.x * 2.0) {
		return;
	}
	let sx = dot(to, su.right.xyz) / (vz * su.fwd.w * su.right.w);
	let sy = dot(to, su.up.xyz) / (vz * su.fwd.w);
	if (abs(sx) >= 0.995 || abs(sy) >= 0.995) {
		return;
	}
	let dim = vec2<f32>(textureDimensions(depth));
	let px = clamp(vec2<i32>((vec2<f32>(sx, -sy) * 0.5 + 0.5) * dim), vec2<i32>(0), vec2<i32>(dim) - vec2<i32>(3));
	let ls = lin(textureLoad(depth, px, 0));
	let pen = vz - ls;
	if (pen < 0.0 || pen > THICK + length((*p).b.xyz) * dt) {
		return;
	}
	let pc = point_at(px, dim, ls);
	let pxx = point_at(px + vec2<i32>(2, 0), dim, lin(textureLoad(depth, px + vec2<i32>(2, 0), 0)));
	let pyy = point_at(px + vec2<i32>(0, 2), dim, lin(textureLoad(depth, px + vec2<i32>(0, 2), 0)));
	var n = cross(pxx - pc, pyy - pc);
	let nl = length(n);
	if (nl < 1.0e-6) {
		return;
	}
	n = n / nl;
	if (dot(n, to) > 0.0) {
		n = -n;
	}
	// Back along its own view ray onto the surface (view depth ls), a hair in front of it.
	(*p).a = vec4<f32>(su.eye.xyz + to * (ls / vz) + n * 0.03, (*p).a.w);
	var v = (*p).b.xyz;
	let vn = dot(v, n);
	if (vn < 0.0) {
		v = v - n * vn * (1.0 + BOUNCE);
	}
	v = v * (1.0 - min(FRICTION * dt, 0.9));
	(*p).b = vec4<f32>(v, (*p).b.w);
}

@compute @workgroup_size(64)
fn cs_step(@builtin(global_invocation_id) gid: vec3<u32>) {
	let pool = su.ring.z;
	let i = gid.x;
	if (i >= pool) {
		return;
	}
	let dt = su.eye.w;
	let k = (i + pool - su.ring.x) % pool;
	let ne = u32(su.k.z);
	if (k < su.ring.y && ne > 0u) {
		var s = pcg(i * 9781u + su.ring.w * 6271u + 17u);
		// Born at a random moment of the frame (no bands of particles born together), f seconds before its end, where the emitter
		// stood then: between last frame's place and this frame's (a gun that turns or strafes leaves a continuous stream, not a
		// burst per frame at one nozzle point). Then it keeps its own world-space velocity (never parented to the gun).
		let f = rnd(&s) * dt;
		var e = emits[find(k, ne)];
		if (e.pa.w > 0.5) {
			let w = f / max(dt, 1.0e-5);
			e.a = vec4<f32>(mix(e.a.xyz, e.pa.xyz, w), e.a.w);
			if (e.n.y == 1u) {
				e.b = vec4<f32>(mix(e.b.xyz, e.pb.xyz, w), e.b.w);
			}
		}
		var p = spawn(e, &s);
		p.a = vec4<f32>(p.a.xyz + p.b.xyz * f, f);
		leash(&p, f, false);
		parts[i] = p;
		return;
	}
	var p = parts[i];
	if (p.b.w <= 0.0) {
		return;
	}
	p.a.w += dt;
	if (p.a.w >= p.b.w) {
		p.b.w = 0.0;
		parts[i] = p;
		return;
	}
	let flags = u32(p.c.w + 0.5);
	let kind = flags & 3u;
	let smoke = (flags & 4u) != 0u;
	let fl = select(p.b.w, p.b.w / SMOKE_X, smoke);
	p.c.x *= exp(-kpick(kind, 1.6, 1.0, 2.3) * dt / max(fl, 0.02));
	let temp = p.c.x;
	// Relax towards the rise (terminal speed x temperature; smoke drifts up slowly) plus the wind; drag sets how fast.
	// Buoyancy grows as the particle ages (hot gas accelerates up: tongues stretch and tear off), and so does the turbulence.
	let lf = clamp(p.a.w / max(fl, 0.02), 0.0, 1.0);
	let rise = select(kpick(kind, 2.6, 1.6, 2.2) * temp * (0.6 + 2.2 * lf), 0.7, smoke && temp < 0.28) * su.m.x;
	let gust = select(kpick(kind, 1.5, 1.2, 1.8) * (0.4 + temp) * (0.5 + 1.8 * lf), 0.6, smoke && temp < 0.28) * su.m.y;
	let goal = vec3<f32>(0.0, 0.0, rise) + wind(p.a.xyz, su.up.w) * gust;
	let drag = kpick(kind, 1.3, 2.4, 3.4) * su.m.w;
	let v = p.b.xyz + (goal - p.b.xyz) * min(drag * dt, 1.0);
	p.b = vec4<f32>(v, p.b.w);
	p.a = vec4<f32>(p.a.xyz + v * dt, p.a.w);
	if (su.k.w > 0.5) {
		collide(&p, dt);
	}
	leash(&p, dt, smoke && p.c.x < 0.28);
	parts[i] = p;
}
