// [PC] Brightness and Contrast, Classic lighting without the upscaler slot (grade_pass.rs, viewer/picture_adjust.rs): the resolved
// copy of the game picture written back with the curve (`grade_curve`, grade_curve.wgsl, appended) into every sample of the MSAA
// colour target. Each pixel loads its own texel (exact, no filtering). At 50 / 50 this pass does not run.
struct U {
	k: vec4<f32>,
};
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> u: U;

@vertex
fn vs_grade(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_grade(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
	let dim = vec2<i32>(textureDimensions(src));
	let p = clamp(vec2<i32>(floor(pos.xy)), vec2<i32>(0), dim - vec2<i32>(1));
	let c = textureLoad(src, p, 0);
	return vec4<f32>(grade_curve(c.rgb, u.k), c.a);
}
