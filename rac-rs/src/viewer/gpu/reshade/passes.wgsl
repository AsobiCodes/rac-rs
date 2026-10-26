// [PC] ReShade add-on (reshade.rs): the three small full-screen passes around the effects.
//  fs_depth: the level's 4x MSAA depth -> a plain R32Float depth texture (the nearest of the 4 samples), the "DEPTH" ReShade reads.
//  fs_touch: reads that texture into a 1x1 target, only so the renderer leaves it in the shader-resource state ReShade expects.
//  fs_copy:  the picture ReShade returned (resolved colour target) -> every sample of the MSAA target, so the HUD draws over it.
const SAMPLES: i32 = 4;

@group(0) @binding(0) var depth_ms: texture_depth_multisampled_2d;
@group(0) @binding(1) var depth1: texture_2d<f32>;
@group(0) @binding(2) var picture: texture_2d<f32>;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
	let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
	return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_depth(@builtin(position) pos: vec4<f32>) -> @location(0) f32 {
	let c = vec2<i32>(pos.xy);
	var d = textureLoad(depth_ms, c, 0);
	for (var s = 1; s < SAMPLES; s++) {
		d = min(d, textureLoad(depth_ms, c, s));
	}
	return d;
}

@fragment
fn fs_touch() -> @location(0) f32 {
	return textureLoad(depth1, vec2<i32>(0, 0), 0).x;
}

@fragment
fn fs_copy(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
	return textureLoad(picture, vec2<i32>(pos.xy), 0);
}
