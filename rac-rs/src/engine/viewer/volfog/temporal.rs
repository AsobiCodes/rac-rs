//! Shared by the volumetric passes: temporal accumulation. The march jitters its step positions per frame (interleaved gradient
//! noise plus a golden-ratio frame offset); `fs_resolve` reprojects the previous frame's result to the place the ray ends at,
//! clamps it to the colour range of the current frame's 3x3 neighbourhood (no ghost trails behind moving objects) and blends.
//! The blend weight of the current frame is `march.w` (1 when there is no usable history: first frame, new level, resized targets).

/// WGSL placed after `scatter::BLOCK` and `scatter::MARCH`.
pub const RESOLVE: &str = r#"
// Previous-frame texture coordinate of world point p and 1 when it lies inside that frame's view.
fn prev_uv(p: vec3<f32>) -> vec3<f32> {
	let v = p - f.prev[0].xyz;
	let z = dot(v, f.prev[1].xyz);
	if (z < 0.1) { return vec3<f32>(0.0); }
	let r = f.prev[2].xyz;
	let u = f.prev[3].xyz;
	let nx = dot(v, r) / (z * dot(r, r));
	let ny = dot(v, u) / (z * dot(u, u));
	let uv = vec2<f32>(nx * 0.5 + 0.5, 0.5 - ny * 0.5);
	let ok = uv.x > 0.0 && uv.y > 0.0 && uv.x < 1.0 && uv.y < 1.0;
	return vec3<f32>(uv, select(0.0, 1.0, ok));
}

@fragment fn fs_resolve(v: VO) -> @location(0) vec4<f32> {
	let c = vec2<i32>(v.pos.xy);
	let dims = vec2<i32>(textureDimensions(cur));
	let now = textureLoad(cur, c, 0);
	var mn = now;
	var mx = now;
	for (var y = -1; y <= 1; y++) {
		for (var x = -1; x <= 1; x++) {
			let s = textureLoad(cur, clamp(c + vec2<i32>(x, y), vec2<i32>(0), dims - vec2<i32>(1)), 0);
			mn = min(mn, s);
			mx = max(mx, s);
		}
	}
	let p = f.cam[0].xyz + view_dir(v.pos.xy) * end_depth(c);
	let pu = prev_uv(p);
	let h = clamp(textureSampleLevel(hist, lsamp, pu.xy, 0.0), mn, mx);
	let a = select(1.0, f.march.w, pu.z > 0.5);
	return mix(h, now, a);
}
"#;

/// Blend weight of the current frame once history exists (about 8 frames of averaging).
pub const BLEND: f32 = 0.12;
