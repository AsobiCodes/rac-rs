// [PC] Sky visibility of a reflected ray (shared WGSL: the metal look, metalfx.wgsl; meant for the water reflection too): whether a
// ray leaving a surface sees the open sky, or a roof, walls or a cave keep it off, so a reflection never shows the sky (or a planet in
// it) through walls. 1 = open, 0 = blocked; a blocked ray shows the lit scene around the surface instead (`skyvis_mix`).
// The including shader defines three hooks (WGSL resolves them anywhere in the module):
// - `skyvis_sun_gap(q: vec3<f32>) -> vec2<f32>`: x = how far world point q lies behind the first surface the key light meets above
//   it (world units, > 0 = shadowed), y = 1 where a shadow or height map covers q, 0 where none does (or there is no map);
// - `skyvis_cone(uv: vec2<f32>, rad: f32) -> f32`: how open the sky is (0..1) in what the camera saw (`skyvis_screen`'s picture)
//   over a footprint of radius `rad` (screen heights) around screen point uv (0..1): a cone around the ray, not one point, so a thin
//   far object (a window post) does not close it and only big blockers (walls, a ceiling) do; smooth as the camera moves.

// How far along the ray the map test looks (world units).
const SKYVIS_REACH: f32 = 12.0;

// From the key light's shadow or height map (`skyvis_sun_gap`): (open 1 .. blocked 0, 1 when a map covers some point of the ray).
// Points along the ray from p (out to SKYVIS_REACH) are tested against the surface the light meets above them. Blocked where the ray
// passes up THROUGH a surface (from just behind it to just in front: a roof or ceiling from below) or when every point stays behind
// surfaces (a closed room). Leaving a shadowed region sideways (a window, a doorway, out from under an awning or a hero's shadow)
// jumps from far behind to far in front and does not block. The thresholds are soft (no hard sky / roof line on a floor). No map:
// open. CPU twin: metalfx.rs `tests::sun_open`.
fn skyvis_sun(p: vec3<f32>, r: vec3<f32>) -> vec2<f32> {
	var prev = 0.0;
	var have = false;
	var all_behind = 1.0;
	var covered = false;
	var blocked = 0.0;
	var t0 = 0.0;
	for (var k = 0; k < 6; k++) {
		let x = f32(k) / 5.0;
		let t = 0.4 + SKYVIS_REACH * x * x;
		let g = skyvis_sun_gap(p + r * t);
		let tol = 1.5 + (t - t0);
		t0 = t;
		if (g.y < 0.5) { all_behind = 0.0; have = false; continue; }
		covered = true;
		let behind = smoothstep(-0.25, 0.25, g.x);
		all_behind = min(all_behind, behind);
		if (have) {
			let through = smoothstep(0.0, 0.25, prev) * (1.0 - smoothstep(0.6 * tol, tol, prev)) * (1.0 - smoothstep(0.6 * tol, tol, -g.x));
			blocked = max(blocked, through * (1.0 - behind));
		}
		prev = g.x;
		have = true;
	}
	if (covered) { blocked = max(blocked, all_behind); }
	return vec2<f32>(1.0 - blocked, select(0.0, 1.0, covered));
}

// What the camera (view-projection `vp` of the hook's picture) saw where the ray p + t r runs out of its view: (how open the sky is
// there over a footprint of radius `rad`, `skyvis_cone`; trust). With the ray's vanishing point on screen: the share there, trust 1 (a near wall or ceiling that way blocks, sky leaves it
// open). Otherwise the screen edge its screen path runs into (ndc direction c.xy - a c.w: toward the vanishing point, or away from
// it when the ray turns back past the camera); trust falls to 0 as the vanishing point moves a third of the screen past the edge
// and is 0 behind the camera, as the edge only says what lies that way ON screen. Off the picture: (1, 0). CPU twin: metalfx.rs
// `tests::screen_end`.
fn skyvis_screen(vp: mat4x4<f32>, p: vec3<f32>, r: vec3<f32>, rad: f32) -> vec2<f32> {
	let a = vp * vec4<f32>(p, 1.0);
	if (a.w <= 0.1) { return vec2<f32>(1.0, 0.0); }
	let na = a.xy / a.w;
	if (max(abs(na.x), abs(na.y)) >= 1.0) { return vec2<f32>(1.0, 0.0); }
	let c = vp * vec4<f32>(r, 0.0);
	var e = c.xy / max(c.w, 1e-4);
	var trust = 0.0;
	if (c.w > 1e-4) { trust = 1.0 - smoothstep(0.0, 0.6, max(abs(e.x), abs(e.y)) - 1.0); }
	if (c.w <= 1e-4 || max(abs(e.x), abs(e.y)) > 1.0) {
		let dir = c.xy - na * c.w;
		let dd = select(dir, vec2<f32>(1e-6), abs(dir) < vec2<f32>(1e-6));
		let t = (sign(dd) - na) / dd;
		e = clamp(na + dd * max(min(t.x, t.y), 0.0), vec2<f32>(-1.0), vec2<f32>(1.0));
	}
	return vec2<f32>(skyvis_cone(vec2<f32>(e.x * 0.5 + 0.5, 0.5 - e.y * 0.5), rad), trust);
}

// Both tests: open only where the map leaves it open and the screen does too; the screen counts where it can be trusted, and fully
// where no map covers the ray (levels without dynamic shadows).
fn skyvis_open(sun: vec2<f32>, scr: vec2<f32>) -> f32 {
	return sun.x * mix(1.0, scr.x, max(scr.y, 1.0 - sun.y));
}

// The light a ray that met nothing reflects: `sky` where it is open, `occ` (the lit scene around the surface) where it is blocked.
fn skyvis_mix(occ: vec3<f32>, sky: vec3<f32>, vis: f32) -> vec3<f32> {
	return mix(occ, sky, clamp(vis, 0.0, 1.0));
}
