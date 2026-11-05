// [PC] Upscaler inputs (upscale.rs, lane ndlss): per-pixel motion vectors and a single-sample depth from the level's 4x MSAA
// depth (sample 0), and a debug picture of the vectors. Motion = previous UV - current UV of the same world point (UV of the
// render picture, y down), the current one WITHOUT this frame's jitter.

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
@group(0) @binding(1) var depth_ms: texture_depth_multisampled_2d;
@group(1) @binding(0) var motion: texture_2d<f32>;

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn uv_of(c: vec4<f32>) -> vec2<f32> {
	let n = c.xy / c.w;
	return vec2<f32>(0.5 + 0.5 * n.x, 0.5 - 0.5 * n.y);
}

// The motion of the world point under pixel centre `uv` at depth `d` (`sky`: nothing drawn there, the sky's eye-at-origin matrices).
fn motion_of(uv: vec2<f32>, d: f32, sky: bool) -> vec2<f32> {
	var inv = u.inv_cur;
	var cur = u.cur;
	var prev = u.prev;
	if (sky) {
		inv = u.sky_inv_cur;
		cur = u.sky_cur;
		prev = u.sky_prev;
	}
	let h = inv * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
	let p = vec4<f32>(h.xyz / h.w, 1.0);
	let c = cur * p;
	let q = prev * p;
	// Behind last frame's eye (or degenerate): no usable history position, no motion.
	if (q.w <= 1e-6 || c.w <= 1e-6) {
		return vec2<f32>(0.0, 0.0);
	}
	return uv_of(q) - uv_of(c);
}

struct Out {
	@location(0) mv: vec2<f32>,
	@location(1) depth: f32,
};

@fragment fn fs_motion(@builtin(position) pos: vec4<f32>) -> Out {
	let d = textureLoad(depth_ms, vec2<i32>(pos.xy), 0);
	var o: Out;
	o.mv = motion_of(pos.xy * u.size.zw, d, d >= 1.0);
	o.depth = d;
	return o;
}

// Debug picture (RAC_UPSCALE_PREP=show): grey = still, red / green = motion along x / y (0.5 + px / 32), saturated at 16 render pixels.
@fragment fn fs_show(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
	let mv = textureLoad(motion, vec2<i32>(pos.xy), 0).xy;
	let c = clamp(vec2<f32>(0.5) + mv * u.misc.x * vec2<f32>(1.0, u.size.y * u.size.z), vec2<f32>(0.0), vec2<f32>(1.0));
	return vec4<f32>(c.x, c.y, 0.5 - 0.5 * u.misc.y, 1.0);
}
