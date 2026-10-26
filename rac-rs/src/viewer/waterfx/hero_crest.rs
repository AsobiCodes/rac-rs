//! [PC] The geometric sea's crest in front of the swimming hero (WGSL, appended to `SHADER` in `WaterFx::new`): `fs_water` reads
//! the picture under such a crest from the flat sea round him (`sea_pic`) and fills the crest's pin holes over his body
//! (`geo_fill`); both showed his body through the crest as dark lines and orange specks (Pokitaru, 2026-10-24).

pub(super) const HERO_CREST_WGSL: &str = r#"
// [PC] Geometric sea: a crest standing in front of the swimming hero has no flat sea under it on screen (its own pixel and its
// flat point's are his body): the picture of the nearest flat sea round it (rings of 8, SEA_PIC_R0 doubling 5 times) instead of
// his, which showed as dark lines and orange specks on the crest (user screenshot 2026-10-24).
const SEA_PIC_R0: f32 = 4.0;
fn sea_pic(uv: vec2<f32>, geo: bool) -> vec2<f32> {
	if (!geo || wet(uv, true)) { return uv; }
	let dirs = array<vec2<f32>, 8>(vec2<f32>(0.0, 1.0), vec2<f32>(-0.7071, 0.7071), vec2<f32>(0.7071, 0.7071), vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(-0.7071, -0.7071), vec2<f32>(0.7071, -0.7071), vec2<f32>(0.0, -1.0));
	var r = SEA_PIC_R0;
	for (var i = 0; i < 5; i++) {
		for (var k = 0; k < 8; k++) {
			let q = uv + dirs[k] * r * w.size.zw;
			if (all(q > vec2<f32>(0.0)) && all(q < vec2<f32>(1.0)) && wet(q, true)) { return q; }
		}
		r *= 2.0;
	}
	return uv;
}
// [PC] Geometric sea: a pin hole of a crest over the swimming hero (a fold of the grid there showed his body through it as dark
// lines and orange specks, user screenshot 2026-10-24) takes the nearest grid pixel within GAP_R along the axes when grid lies
// on both sides. Else `geo_gap`.
const GAP_R: i32 = 3;
fn geo_fill(c: vec2<i32>) -> vec4<f32> {
	let g = geo_gap(c);
	if (g.a > 0.5 || !hero_pixel(c, linear_depth(textureLoad(depth_ms, c, 0)), geo_top())) { return g; }
	let dirs = array<vec2<i32>, 4>(vec2<i32>(-1, 0), vec2<i32>(1, 0), vec2<i32>(0, -1), vec2<i32>(0, 1));
	var best = vec4<f32>(0.0);
	var side = vec4<f32>(0.0);
	for (var r = 1; r <= GAP_R; r++) {
		for (var k = 0; k < 4; k++) {
			let n = wpos_at(c + dirs[k] * r);
			if (n.a > 0.5 && (u32(n.a - 0.5) & 512u) != 0u) {
				if (best.a < 0.5) { best = n; }
				side[k] = 1.0;
			}
		}
	}
	return select(g, best, side.x * side.y + side.z * side.w > 0.5);
}
"#;
