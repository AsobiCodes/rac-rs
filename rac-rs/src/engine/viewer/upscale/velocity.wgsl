// [PC] Object motion for the upscaler inputs (upscale/velocity.rs, lane ndlss): the props batch and the hero marker drawn again
// with this frame's and last frame's positions. Where they are the visible surface (their depth matches the scene depth copy of
// motion.wgsl) their vectors replace the camera vectors. Same units: previous UV - current UV, the current one without jitter.

struct U {
	inv_cur: mat4x4<f32>,
	cur: mat4x4<f32>,
	prev: mat4x4<f32>,
	sky_inv_cur: mat4x4<f32>,
	sky_cur: mat4x4<f32>,
	sky_prev: mat4x4<f32>,
	size: vec4<f32>,
	misc: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var depth_copy: texture_2d<f32>;

// The props slot record (viewer.rs / gpuskin.rs `PropInst`), this frame's and last frame's.
struct PropInst { m0: vec4<f32>, m1: vec4<f32>, m2: vec4<f32>, m3: vec4<f32>, light: vec4<f32>, tint: vec4<f32>, ptint: vec4<f32>, a: vec4<f32>, k: vec4<u32> };
@group(1) @binding(0) var<storage, read> insts: array<PropInst>;
@group(1) @binding(1) var<storage, read> poses: array<f32>;
@group(1) @binding(2) var<storage, read> prev_insts: array<PropInst>;
@group(1) @binding(3) var<storage, read> prev_poses: array<f32>;

struct VOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) cur: vec4<f32>,
	@location(1) prev: vec4<f32>,
};

// The level shader's clip position (viewer.rs `world_clip`: z rebuilt from w) moved by this frame's jitter (upscale.rs
// `jitter_matrix`), so the fragments land on the pixels and depths the level pass wrote.
fn raster(p: vec3<f32>) -> vec4<f32> {
	let m = u.cur;
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
	c.x += 2.0 * u.misc.z * u.size.z * c.w;
	c.y -= 2.0 * u.misc.w * u.size.w * c.w;
	return c;
}

fn out(cur: vec3<f32>, prev: vec3<f32>) -> VOut {
	var o: VOut;
	o.pos = raster(cur);
	o.cur = u.cur * vec4<f32>(cur, 1.0);
	// A reset frame (misc.y) has no history: the object's own motion is dropped too.
	o.prev = u.prev * vec4<f32>(select(prev, cur, u.misc.y != 0.0), 1.0);
	return o;
}

// A vertex the level pass did not draw (hidden slot): outside the depth range, clipped.
fn hidden() -> VOut {
	var o: VOut;
	o.pos = vec4<f32>(0.0, 0.0, -1.0, 1.0);
	o.cur = vec4<f32>(0.0, 0.0, 0.0, 1.0);
	o.prev = o.cur;
	return o;
}

// vs_props_pose (viewer.rs): kinds 1 (prop: scaled, placed) and 2 (actor: world offset) without the prompt flag are drawn.
fn live(s: PropInst) -> bool {
	let kind = s.k.x & 0xffu;
	return (kind == 1u || kind == 2u) && (s.k.x & 0x100u) == 0u;
}

fn place(s: PropInst, q: vec3<f32>) -> vec3<f32> {
	if ((s.k.x & 0xffu) == 1u) {
		let a = q * s.a.x;
		return s.m0.xyz * a.x + s.m1.xyz * a.y + s.m2.xyz * a.z + s.m3.xyz;
	}
	return q + s.m3.xyz;
}

@vertex fn vs_props(@location(0) p: vec3<f32>, @location(1) sj: vec2<u32>) -> VOut {
	let s = insts[sj.x];
	let j = sj.y;
	if (!live(s)) {
		return hidden();
	}
	var q = p;
	if (j < s.k.w) {
		let b = (s.k.z + j) * 6u;
		q = vec3<f32>(poses[b], poses[b + 1u], poses[b + 2u]);
	}
	let cur = place(s, q);
	var prev = cur;
	// The same slot last frame, when it held the same kind of record with the same posed vertex count (slot index = identity).
	if (sj.x < arrayLength(&prev_insts)) {
		let t = prev_insts[sj.x];
		if (live(t) && (t.k.x & 0xffu) == (s.k.x & 0xffu) && t.k.w == s.k.w) {
			var r = p;
			let b = (t.k.z + j) * 6u;
			if (j < t.k.w && b + 2u < arrayLength(&prev_poses)) {
				r = vec3<f32>(prev_poses[b], prev_poses[b + 1u], prev_poses[b + 2u]);
			}
			prev = place(t, r);
		}
	}
	return out(cur, prev);
}

// The hero marker: world-space vertices of this frame (`p`) and of last frame (`q`, same vertex order).
@vertex fn vs_hero(@location(0) p: vec3<f32>, @location(1) q: vec3<f32>) -> VOut {
	return out(p, q);
}

fn uv_of(c: vec4<f32>) -> vec2<f32> {
	let n = c.xy / c.w;
	return vec2<f32>(0.5 + 0.5 * n.x, 0.5 - 0.5 * n.y);
}

// Only where this surface is what the level pass kept: its depth within one pixel's depth slope of the copy (sample 0 of the
// MSAA depth sits up to ~0.4 px from the pixel centre), else the camera vector stays.
@fragment fn fs_object(v: VOut) -> @location(0) vec2<f32> {
	let tol = fwidth(v.pos.z) + 1.0e-6;
	let ds = textureLoad(depth_copy, vec2<i32>(v.pos.xy), 0).r;
	if (abs(v.pos.z - ds) > tol || v.cur.w <= 1.0e-6 || v.prev.w <= 1.0e-6) {
		discard;
	}
	return uv_of(v.prev) - uv_of(v.cur);
}
