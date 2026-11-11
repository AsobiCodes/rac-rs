// [PC] Fire light v2 stage 4 (lane dlit3, FIRE_LIGHT_V2.md 3.3; sunshadow/omni_mom.rs): the fire shadow maps' MOMENT cubes.
// cs_mom: per moment texel of cube face f of map k (array layer 6 k + f, MOM_N texels a side, the GPU's cube-face convention) the
// mean over n x n sub-directions of the 4 moments (z, z^2, z^3, z^4) of the depth the drawn face tile (sunshadow/omni.rs `face`,
// tile 6 k + f of the atlas, a little wider than 90 degrees: SCALE) holds in that direction; z = the RADIAL distance from the map's
// origin mapped to [0, 1] over [near, far] (continuous across face edges, so the hardware's seamless cube filter blends like
// with like); written with Peters & Klein 2015's quantisation transform (MOM_F, MOM_OFS: linear, so box-filtered mips stay valid).
// cs_down: one mip from the one above, 2 x 2 box mean. FXC-safe: no local arrays, textureLoad only, textureDimensions outside loops.

struct MomU { q: vec4<f32>, m: array<vec4<f32>, 4> };

const MOM_N: u32 = 128u;
const MOM_SCALE: f32 = 0.97;
const MOM_OFS: f32 = 0.035955884801;
// Rows of Peters & Klein 2015's 4 x 4 transform (optimized[j] = sum_i b[i] x row i [j]): WGSL columns, so MOM_F * b.
const MOM_F: mat4x4<f32> = mat4x4<f32>(
	vec4<f32>(-2.07224649, 13.7948857237, 0.105877704, 9.7924062118),
	vec4<f32>(32.23703778, -59.4683975703, -1.9077466311, -33.7652110555),
	vec4<f32>(-68.571074599, 82.0359750338, 9.3496555107, 47.9456096605),
	vec4<f32>(39.3703274134, -35.364903257, -6.6543490496, -23.9728048165));

@group(0) @binding(0) var<uniform> mu: MomU;
@group(0) @binding(1) var mom_atlas: texture_depth_2d;
@group(0) @binding(2) var mom_out: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(3) var mom_in: texture_2d_array<f32>;

// The direction of cube face f at (s, t) in [-1, 1] (s right, t down: the GPU's cube-map convention, Vulkan / D3D / Metal alike).
fn mom_dir(f: u32, st: vec2<f32>) -> vec3<f32> {
	let s = st.x;
	let t = st.y;
	if (f == 0u) { return vec3<f32>(1.0, -t, -s); }
	if (f == 1u) { return vec3<f32>(-1.0, -t, s); }
	if (f == 2u) { return vec3<f32>(s, 1.0, t); }
	if (f == 3u) { return vec3<f32>(s, -1.0, -t); }
	if (f == 4u) { return vec3<f32>(s, -t, 1.0); }
	return vec3<f32>(-s, -t, -1.0);
}

// mu.q = (face tile texels, tiles per atlas side, sub-directions per texel side n, 0); mu.m[k] = (near, far, 0, 0) of map k.
@compute @workgroup_size(8, 8, 1)
fn cs_mom(@builtin(global_invocation_id) id: vec3<u32>) {
	if (id.x >= MOM_N || id.y >= MOM_N) { return; }
	let layer = id.z;
	let k = layer / 6u;
	let f = layer - 6u * k;
	let ax = f / 2u;
	let tile = mu.q.x;
	let side = mu.q.y;
	let n = u32(mu.q.z);
	let mp = mu.m[k];
	let near = mp.x;
	let far = max(mp.y, near * 2.0);
	let a = far / (far - near);
	// The face tile's atlas texel origin (tile index 6 k + f = the layer; omni.rs `tile_xy`), the row in float as flitshadow.wgsl.
	let ti = f32(layer);
	let row = floor((ti + 0.5) / side);
	let cell = vec2<f32>(ti - row * side, row) * tile;
	let lim = vec2<f32>(0.5 / tile);
	var acc = vec4<f32>(0.0);
	for (var j = 0u; j < n * n; j++) {
		let sub = vec2<f32>(f32(j % n), f32(j / n)) + 0.5;
		let st = (vec2<f32>(id.xy) + sub / f32(n)) * (2.0 / f32(MOM_N)) - 1.0;
		let d = mom_dir(f, st);
		// The drawn face's right / up (axes ax + 1, ax + 2) over its axis distance (1 here) x SCALE (omni.rs `face`).
		var xy = d.yz;
		if (ax == 1u) { xy = vec2<f32>(d.z, d.x); }
		if (ax == 2u) { xy = d.xy; }
		let ndc = xy * MOM_SCALE;
		let t = clamp(vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5), lim, vec2<f32>(1.0) - lim);
		let zb = textureLoad(mom_atlas, vec2<i32>(cell + t * tile), 0);
		// Axis distance from the face's depth (flitshadow.wgsl: near / (1 - zb / a); a cleared texel = far), then radial.
		let dax = near * a / max(a - zb, 1e-6);
		let z = clamp((dax * length(d) - near) / (far - near), 0.0, 1.0);
		let z2 = z * z;
		acc += vec4<f32>(z, z2, z2 * z, z2 * z2);
	}
	let b = acc / f32(n * n);
	textureStore(mom_out, vec2<i32>(id.xy), i32(layer), MOM_F * b + vec4<f32>(MOM_OFS, 0.0, 0.0, 0.0));
}

// One mip level (mom_out) from the level above (mom_in): the 2 x 2 box mean of the quantised moments.
@compute @workgroup_size(8, 8, 1)
fn cs_down(@builtin(global_invocation_id) id: vec3<u32>) {
	let sz = textureDimensions(mom_out).x;
	if (id.x >= sz || id.y >= sz) { return; }
	let p = vec2<i32>(id.xy) * 2;
	let l = i32(id.z);
	let s = textureLoad(mom_in, p, l, 0) + textureLoad(mom_in, p + vec2<i32>(1, 0), l, 0) + textureLoad(mom_in, p + vec2<i32>(0, 1), l, 0) + textureLoad(mom_in, p + vec2<i32>(1, 1), l, 0);
	textureStore(mom_out, vec2<i32>(id.xy), l, s * 0.25);
}
