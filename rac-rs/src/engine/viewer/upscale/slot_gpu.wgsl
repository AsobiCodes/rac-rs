// [PC] The upscaler slot's own passes (upscale/slot.rs, slot_gpu.rs; lane ndlss, FSR 1 removed in lane nfsr1). `fs_copy`: the
// window-size picture FSR 3 / 4 or DLSS wrote (`src`) into the HUD pass's MSAA target, with the scene depth upsampled (nearest)
// beside it. `fs_scale`: a plain bilinear upscale of the render-size scene (`src`) into the window-size picture, only for a frame
// in which neither upscaler recorded (its runtime refused just now: the slot is off from the next frame, slot.rs `active`).
// Texels are fetched with textureLoad (no sampler); the picture is gamma-space LDR.

struct U {
	// Render width, height; window width, height.
	size: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var depth_ms: texture_depth_multisampled_2d;

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn ld(p: vec2<i32>, size: vec2<f32>) -> vec3<f32> {
	return textureLoad(src, clamp(p, vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1)), 0).rgb;
}

// One window pixel from the four nearest render pixels, weighted by distance (bilinear).
@fragment fn fs_scale(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
	let rs = u.size.xy;
	let pf = pos.xy * rs / u.size.zw - vec2<f32>(0.5);
	let fp = floor(pf);
	let t = pf - fp;
	let p = vec2<i32>(fp);
	let top = mix(ld(p, rs), ld(p + vec2<i32>(1, 0), rs), t.x);
	let bottom = mix(ld(p + vec2<i32>(0, 1), rs), ld(p + vec2<i32>(1, 1), rs), t.x);
	return vec4<f32>(mix(top, bottom, t.y), 1.0);
}

struct CopyOut {
	@location(0) colour: vec4<f32>,
	@builtin(frag_depth) depth: f32,
};

// The window-size picture as it is (FSR 3 / 4 and DLSS sharpen themselves); depth = sample 0 of the render-size MSAA depth.
@fragment fn fs_copy(@builtin(position) pos: vec4<f32>) -> CopyOut {
	let ws = u.size.zw;
	var o: CopyOut;
	o.colour = vec4<f32>(ld(vec2<i32>(pos.xy), ws), 1.0);
	let rp = clamp(vec2<i32>(pos.xy * u.size.xy / ws), vec2<i32>(0), vec2<i32>(u.size.xy) - vec2<i32>(1));
	o.depth = textureLoad(depth_ms, rp, 0);
	return o;
}
