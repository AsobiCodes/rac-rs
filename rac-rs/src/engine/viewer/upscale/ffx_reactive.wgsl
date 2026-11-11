// [PC] The FidelityFX upscaler's reactive mask (ffx_reactive.rs; lane nups2). Per render pixel: the opaque scene before the
// particles (upscale.rs `opaque_copy`) against the finished scene (the slot's resolved input), both tonemapped c / (1 + max(c)),
// the largest channel difference; at least `THRESHOLD` = `BINARY` (FFX marks the pixel as reactive: less history), else 0.
// The defaults of FFX's own GenerateReactiveMask in the SDK's FSR sample (scale 1, cutoff 0.2, binary 0.9, tonemap, max of
// the channels). Ours as a render pass: wgpu has no storage writes to R8Unorm without adapter-specific format features.

@group(0) @binding(0) var opaque: texture_2d<f32>;
@group(0) @binding(1) var scene: texture_2d<f32>;
// [PC] The heat haze's pixels (viewer/heathaze.rs `take_mask`, render size; a 1 x 1 zero texture when the haze did not draw): the
// wobble has no motion vectors, so its pixels take little history (review S5).
@group(0) @binding(2) var heat: texture_2d<f32>;

const THRESHOLD: f32 = 0.2;
const BINARY: f32 = 0.9;
// Heat at which a hazed pixel is fully reactive.
const HEAT_FULL: f32 = 0.25;

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn tonemap(c: vec3<f32>) -> vec3<f32> {
	return c / (1.0 + max(max(c.r, c.g), c.b));
}

@fragment fn fs_reactive(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
	let p = vec2<i32>(pos.xy);
	let a = tonemap(textureLoad(opaque, p, 0).rgb);
	let b = tonemap(textureLoad(scene, p, 0).rgb);
	let d = abs(a - b);
	let m = max(max(d.r, d.g), d.b);
	let hd = vec2<i32>(textureDimensions(heat)) - vec2<i32>(1);
	let h = BINARY * clamp(textureLoad(heat, min(p, hd), 0).r / HEAT_FULL, 0.0, 1.0);
	return vec4<f32>(max(select(0.0, BINARY, m >= THRESHOLD), h), 0.0, 0.0, 1.0);
}
