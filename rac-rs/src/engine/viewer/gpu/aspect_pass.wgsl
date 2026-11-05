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
