// [PC] Dynamic shadows (sunshadow.rs; u.sun.w = 0 when off, and nothing below runs).

// One map layer at map point uv, receiver depth z: the blocked fraction (High: 4x4 hardware PCF taps half a texel off the grid: a
// smooth 5-texel edge instead of 3x3's stair steps; Medium 3x3, Low 2x2: u.sun_q.x is the kernel side).
fn sun_pcf(uv: vec2<f32>, layer: i32, z: f32) -> f32 {
	let ts = 1.0 / u.sun_bias.w;
	let n = i32(u.sun_q.x);
	let h = (u.sun_q.x - 1.0) * 0.5;
	var lit = 0.0;
	for (var y = 0; y < n; y++) {
		for (var x = 0; x < n; x++) {
			lit += textureSampleCompareLevel(sun_map, sun_cmp, uv + (vec2<f32>(f32(x), f32(y)) - h) * ts, layer, z);
		}
	}
	return 1.0 - lit / (u.sun_q.x * u.sun_q.x);
}

// The first cascade whose map holds p answers, every caster at full strength however far it stands (no fade by blocker
// distance: a fade cut shadows off halfway, stacked half shadows under whole ones and lit odd patches of rooms; a roof darkens
// what it covers, indoors are dark). p moves 2 texels towards the light first (no acne under the 4x4 kernel); the last
// cascade fades out at its rim.
fn sun_occ(p: vec3<f32>) -> f32 {
	let nc = i32(u.sun_q.z);
	for (var k = 0; k < nc; k++) {
		let q = u.sun_vp[k] * vec4<f32>(p - u.sun.xyz * (u.sun_texel[k] * 2.0), 1.0);
		let e = max(abs(q.x), abs(q.y));
		if (e < 0.98 && q.z > 0.0 && q.z < 1.0) {
			let uv = vec2<f32>(q.x * 0.5 + 0.5, 0.5 - q.y * 0.5) * u.sun_q.y;
			let z = q.z - u.sun_bias[k];
			var occ = sun_pcf(uv, k, z);
			if (k == nc - 1) { occ *= 1.0 - smoothstep(0.8, 0.98, e); }
			return occ;
		}
	}
	return 0.0;
}

// Baked world colour under a dynamic shadow: 2002's vertex colours already hold the painted shadows, so the shadow takes
// STRENGTH x how lit the colour is (a painted shadow takes none).
fn sun_baked(shade: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
	if (u.sun.w <= 0.0) { return shade; }
	let lum = dot(shade, vec3<f32>(0.2126, 0.7152, 0.0722));
	// [PC] speed (exact): below luminance 0.3 the smoothstep is exactly 0, so the shadow term is x * 0 = 0 and the colour stays
	// `shade * 1.0`: the 4x4 PCF (up to 3 cascades) is not run for a painted-dark fragment.
	if (lum <= 0.3) { return shade; }
	return shade * (1.0 - sun_occ(p) * u.sun.w * smoothstep(0.3, 0.8, lum));
}

// Lit moby under a dynamic shadow: it keeps its ambient and loses its directional light.
fn sun_lit(i: VOut) -> vec3<f32> {
	if (u.sun.w <= 0.0) { return i.lit; }
	return i.amb + (i.lit - i.amb) * (1.0 - sun_occ(i.w) * min(u.sun.w * 1.4, 1.0));
}

