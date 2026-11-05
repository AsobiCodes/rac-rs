// [PC] Render scale (rscale.rs): copies the offscreen picture (drawn at the render size) onto the window image, one full-screen
// triangle, linear filter. Scale above 100 %: the picture is bigger than the window, the sample sits between four texels
// (200 % = an exact 2x2 average); below 100 %: the picture is magnified, softly.
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

struct VOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) uv: vec2<f32>,
};

@vertex
fn vs_blit(@builtin(vertex_index) i: u32) -> VOut {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	var o: VOut;
	o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
	o.uv = vec2<f32>(x, y);
	return o;
}

@fragment
fn fs_blit(v: VOut) -> @location(0) vec4<f32> {
	return textureSampleLevel(src, smp, v.uv, 0.0);
}
