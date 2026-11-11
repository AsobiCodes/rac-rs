// [PC] Aspect ratio box (aspect_pass.rs): copies the box picture into its rectangle of the window image. The pass clears the
// window black (the bars) and draws one triangle over the viewport = the rectangle; each pixel loads its own texel (the box picture
// has the rectangle's size), no filtering, so the copy is exact.
@group(0) @binding(0) var src: texture_2d<f32>;

struct VOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) uv: vec2<f32>,
};

@vertex
fn vs_box(@builtin(vertex_index) i: u32) -> VOut {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	var o: VOut;
	o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
	o.uv = vec2<f32>(x, y);
	return o;
}

@fragment
fn fs_box(v: VOut) -> @location(0) vec4<f32> {
	let dim = vec2<i32>(textureDimensions(src));
	let p = clamp(vec2<i32>(floor(v.uv * vec2<f32>(dim))), vec2<i32>(0), dim - vec2<i32>(1));
	return textureLoad(src, p, 0);
}

// [PC] Resolution (viewer/resolution.rs): the box picture has another size than the rectangle (fullscreen at a chosen size): a
// bilinear filter from four exact loads (the texture is bound unfilterable), texel centres at +0.5.
@fragment
fn fs_scale(v: VOut) -> @location(0) vec4<f32> {
	let dim = vec2<i32>(textureDimensions(src));
	let hi = dim - vec2<i32>(1);
	let f = v.uv * vec2<f32>(dim) - vec2<f32>(0.5);
	let b = floor(f);
	let t = f - b;
	let p0 = clamp(vec2<i32>(b), vec2<i32>(0), hi);
	let p1 = clamp(vec2<i32>(b) + vec2<i32>(1), vec2<i32>(0), hi);
	let c00 = textureLoad(src, p0, 0);
	let c10 = textureLoad(src, vec2<i32>(p1.x, p0.y), 0);
	let c01 = textureLoad(src, vec2<i32>(p0.x, p1.y), 0);
	let c11 = textureLoad(src, p1, 0);
	return mix(mix(c00, c10, t.x), mix(c01, c11, t.x), t.y);
}
