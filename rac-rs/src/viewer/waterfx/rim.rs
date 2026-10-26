//! [PC] The water's rim (WGSL, appended to `SHADER` in `WaterFx::new`): `finish`, the last step of every water pixel. A pixel with
//! little water round it used to blend towards retail's own picture ON that pixel, the old water texture, along every shore
//! (Pokitaru "old textures near edges"); it now blends towards what stands next to the water (rock, hero, wall).

pub(super) const RIM_WGSL: &str = r#"
// Back to the frame: a pixel with little water around blends towards what is next to the water (never retail's water picture),
// and particles over the water (a blast, smoke, splashes) stay as drawn.
fn finish(c: vec2<i32>, orig: vec3<f32>, shaded: vec3<f32>, geo: bool) -> vec4<f32> {
	var around = 0.0;
	var land = vec3<f32>(0.0);
	let uv0 = (vec2<f32>(c) + 0.5) * w.size.zw;
	let ring = array<vec2<i32>, 8>(vec2<i32>(-1, -1), vec2<i32>(0, -1), vec2<i32>(1, -1), vec2<i32>(-1, 0), vec2<i32>(1, 0), vec2<i32>(-1, 1), vec2<i32>(0, 1), vec2<i32>(1, 1));
	for (var k = 0; k < 8; k++) {
		// [PC] Geometric sea: a hole of the grid (`geo_gap`) is water too.
		if (wpos_at(c + ring[k]).a > 0.5 || (geo && geo_gap(c + ring[k]).a > 0.5)) { around += 1.0; } else {
			land += to_linear(textureSampleLevel(scene, lsamp, uv0 + vec2<f32>(ring[k]) * w.size.zw, 0.0).rgb);
		}
	}
	// [PC] The rim blends towards what stands NEXT to the water (rock, hero), never towards retail's own water picture on this
	// pixel (the "old water textures near the edges" along every shore, Agent 12).
	let nl = 8.0 - around;
	let rim = select(shaded, land / max(nl, 1.0), nl > 0.5);
	var col = mix(rim, shaded, smoothstep(2.0, 7.0, around));
	col = mix(col, orig, clamp(textureLoad(glow, c, 0).a * w.mirror.z * 1.5, 0.0, 1.0));
	return vec4<f32>(pow(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2)), 1.0);
}

// [PC] The planar mirror at a waterline (a cave or pool wall standing in the water): the mirror image stops at the contact line, so
// a tap (ripple bend, pixel footprint, bilinear half-resolution texel) crossing it read the mirror's clear colour: a thin pale line
// along the whole waterline (Novalis cave, Agent 12). Within MIRROR_RIM pixels of anything that is not water the tap moves
// away from it, back over the mirrored wall foot (uv units).
const MIRROR_RIM: i32 = 10;
fn wet_px(q: vec2<i32>) -> bool {
	return wpos_at(q).a > 0.5 || geo_gap(q).a > 0.5;
}
fn mirror_push(c: vec2<i32>) -> vec2<f32> {
	let ring = array<vec2<i32>, 8>(vec2<i32>(-1, -1), vec2<i32>(0, -1), vec2<i32>(1, -1), vec2<i32>(-1, 0), vec2<i32>(1, 0), vec2<i32>(-1, 1), vec2<i32>(0, 1), vec2<i32>(1, 1));
	var near = false;
	for (var k = 0; k < 8; k++) {
		if (!wet_px(c + ring[k] * MIRROR_RIM)) { near = true; }
	}
	if (!near) { return vec2<f32>(0.0); }
	var push = vec2<f32>(0.0);
	for (var k = 0; k < 8; k++) {
		for (var s = 1; s <= MIRROR_RIM; s++) {
			if (!wet_px(c + ring[k] * s)) {
				push -= normalize(vec2<f32>(ring[k])) * f32(MIRROR_RIM + 1 - s);
				break;
			}
		}
	}
	let len = length(push);
	return push * (min(len, f32(MIRROR_RIM)) / max(len, 1e-4)) * w.size.zw;
}
"#;
