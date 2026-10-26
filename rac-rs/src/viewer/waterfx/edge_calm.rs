//! [PC] The geometric sea's calm at a waterline against level geometry (WGSL, appended to `SHADER` in `WaterFx::new`): a crest
//! in front of a wall, pipe, pillar or rock base stood over it as a hard-edged teal hump that changed with every wave
//! (Pokitaru). `vs_grid` takes its wave height times `calm_edge`.

pub(super) const EDGE_WGSL: &str = r#"
// [PC] Calm at the waterline against level geometry (a wall, pipe, pillar, rock or bank standing in the sea): `edge_calm` looks for
// pixels round the flat point's pixel (rings of 8, EDGE_R0..EDGE_R1 pixels out) where the level stands nearer than the plane
// (the sea's own pixels see the bed behind it) and keeps the waves down within EDGE_C1 pixels of it. A crest there stood over the
// wall as a hard-edged teal hump (its picture is the flat sea's, read below it) that changed shape with every wave.
const EDGE_C0: f32 = 16.0;
const EDGE_C1: f32 = 56.0;
// Does the level depth zs at pixel q belong to the hero in the water of the plane z = z0 (w.hero = his position, w = 1 when
// known)? His body in the depth is no waterline: the waves go on round and through him (no calm circle, the user asked).
// [PC] Only while he is in that water (his position at most HERO_WET over its plane: retail's swimming hero rides 0.12 under
// the surface, hero_swim.md): standing on a bank over the sea, the bank and the wall under him inside his body's cylinder
// counted as "no ground", and the shore fade cut a slab with vertical stair edges out of the water below him (user's
// jagged edge on pkg6, lane ywmx agent 16).
const HERO_WET: f32 = 0.15;
fn hero_pixel(q: vec2<i32>, zs: f32, z0: f32) -> bool {
	if (w.hero.w < 0.5 || w.hero.z > z0 + HERO_WET) { return false; }
	let ndc = vec2<f32>((f32(q.x) + 0.5) * w.size.z * 2.0 - 1.0, 1.0 - (f32(q.y) + 0.5) * w.size.w * 2.0);
	let a = w.ivp * vec4<f32>(ndc, 0.5, 1.0);
	let dir = a.xyz / a.w - w.eye.xyz;
	let pw = w.eye.xyz + dir * (zs / max(project(w.eye.xyz + dir).z, 1e-4));
	let d = pw - w.hero.xyz;
	return dot(d.xy, d.xy) < HERO_BODY * HERO_BODY && abs(d.z) < HERO_BODY * 1.4;
}
const HERO_BODY: f32 = 1.7;
// The highest geometric sea plane this frame (`hero_pixel` where no plane is at hand).
fn geo_top() -> f32 {
	var top = -1e9;
	for (var k = 0; k < 4; k++) {
		if (w.geo[k].y > 0.5) { top = max(top, w.geo[k].x); }
	}
	return top;
}

fn edge_calm(px: vec2<f32>, z0: f32) -> f32 {
	let dim = vec2<i32>(textureDimensions(depth_ms));
	var keep = 1.0;
	// Ring 7 (radius 64.5 px) lies past EDGE_C1, its weight is exactly 0: skipping it is exact.
	for (var r = 0; r < 7; r++) {
		let rad = 3.0 * pow(1.55, f32(r));
		var dry = 0.0;
		for (var k = 0; k < 8; k++) {
			let a = (f32(k) + 0.5 * f32(r & 1)) * 0.7854;
			let q = vec2<i32>(px + vec2<f32>(cos(a), sin(a)) * rad);
			if (any(q < vec2<i32>(0)) || any(q >= dim)) { continue; }
			let d = textureLoad(depth_ms, q, 0);
			let zs = linear_depth(d);
			if (zs >= w.proj.w * 0.98) { continue; }
			// The retail sea itself (its mesh passed the level's depth there: the sea mask is set) is water, never a bank: its own
			// depth, a few hundredths off the plane and moving with retail's steps, read as "dry" at grazing views and made the
			// waves round every shore flicker.
			if (textureLoad(seamask, q, 0).r > -1e8) { continue; }
			let zp = plane_depth(vec2<f32>(q) + 0.5, z0);
			if (zs < zp - 0.05 - zp * 0.01 && !hero_pixel(q, zs, z0)) { dry += 0.125; }
		}
		keep = min(keep, 1.0 - dry * (1.0 - smoothstep(EDGE_C0, EDGE_C1, rad)));
	}
	return keep;
}

// The waves' rise at the flat point f, calmed at a waterline against level geometry (`vs_grid`; ndc = the vertex, z0 = its plane, gi = its index).
fn calm_edge(f: vec3<f32>, ndc: vec2<f32>, z0: f32, gi: u32) -> f32 {
	let c = calm(f).x;
	if (c <= 0.001) { return c; }
	let px = (ndc * vec2<f32>(0.5, -0.5) + 0.5) * w.size.xy;
	// No dry tile near (dry_tiles.rs): every sample of edge_calm is skipped, it returns exactly 1.0.
	if (!dry_near(px, gi)) { return c; }
	return c * edge_calm(px, z0);
}
"#;
