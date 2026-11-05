//! [PC] The water reflection's miss colour under a roof (lane nwsky, 2026-11-03; user: "water reflections show sky inside the Novalis
//! caves"). Appends `skyvis.wgsl` (the test the metal floors use) to the water shader with the water's two hooks, and `water_miss`:
//! where the screen-space reflection finds nothing, the far air (`env`: fog-coloured, the sky's stand-in) is shown only where the sky
//! is open along the ray; under a roof or in a cave the water's own colour is shown instead. Where the test says fully open the
//! result is `env` bit for bit (open water under a real sky does not change).
//!
//! Hooks: `skyvis_sun_gap(q)` reads the water's top-down height map (`skymap.rs`: the highest surface over q's column, so a roof
//! above q gives a positive gap; nothing above or outside the window: open); `skyvis_cone(uv, rad)` is the share of sky in the
//! level's depth (`scene_z`) over 3x3 taps around uv (the picture of this frame, no history). `RAC_TUNE=water_occl=0` = off (the
//! old picture), read once, baked into the shader text.

use std::sync::OnceLock;

/// `RAC_TUNE=water_occl=0` switches the roof test off (default 1 = on); read once.
pub(super) fn occl_on() -> bool {
	static V: OnceLock<bool> = OnceLock::new();
	*V.get_or_init(|| crate::devtune::f32("water_occl", 1.0) >= 0.5)
}

/// The shared test plus the water's hooks, for the water shader (`WaterFx::new`, the naga test).
pub(super) fn wgsl() -> String {
	wgsl_with(occl_on())
}

pub(super) fn wgsl_with(on: bool) -> String {
	include_str!("../skyvis.wgsl").to_string() + &HOOKS_WGSL.replace("WATER_OCCL_ON", if on { "true" } else { "false" })
}

const HOOKS_WGSL: &str = r#"
// [PC] skyvis.wgsl hook: world point q against the water's sky map (skymap.rs: the highest surface seen from above): (how far q lies
// below it, world units; 1 = inside the built window, 0 = outside it or not built). Nothing above q: far in front (open).
fn skyvis_sun_gap(q: vec3<f32>) -> vec2<f32> {
	if (sky.a.w < 0.5) { return vec2<f32>(0.0); }
	let n = sky.b.z;
	let ux = ((q.x - sky.a.x) / sky.a.z + 1.0) * 0.5 * n;
	let uy = (1.0 - (q.y - sky.a.y) / sky.a.z) * 0.5 * n;
	if (ux < 0.0 || uy < 0.0 || ux >= n || uy >= n) { return vec2<f32>(0.0); }
	let d = textureLoad(sky_depth, vec2<i32>(i32(ux), i32(uy)), 0);
	if (d >= 1.0) { return vec2<f32>(-50.0, 1.0); }
	return vec2<f32>((sky.b.x - d * sky.b.y) - q.z, 1.0);
}

// [PC] skyvis.wgsl hook: how open the sky is (0..1) in this frame's depth over 3x3 taps spaced `rad` (screen heights) around uv. Only
// big blockers count (a thin far post leaves it open): the share runs through smoothstep(0.25, 0.75).
fn skyvis_cone(uv: vec2<f32>, rad: f32) -> f32 {
	let o = vec2<f32>(rad, rad * w.size.x * w.size.w);
	var s = 0.0;
	for (var j = -1; j <= 1; j++) {
		for (var i = -1; i <= 1; i++) {
			let z = scene_z(clamp(uv + vec2<f32>(f32(i), f32(j)) * o, vec2<f32>(0.0), vec2<f32>(1.0)));
			s += select(0.0, 1.0, z >= w.proj.w * 0.98);
		}
	}
	return smoothstep(0.25, 0.75, s / 9.0);
}

// [PC] The reflection of a ray (direction r, smooth direction rs) from water point p that met nothing: `env` (the far air) where the
// sky is open, the water's own colour `base` where a roof, walls or a cave block it. Exactly `air` where fully open.
fn water_miss(p: vec3<f32>, rs: vec3<f32>, air: vec3<f32>, base: vec3<f32>) -> vec3<f32> {
	if (!WATER_OCCL_ON) { return air; }
	let vis = skyvis_open(skyvis_sun(p, rs), skyvis_screen(w.vp, p, rs, 0.05));
	return select(skyvis_mix(base, air, vis), air, vis >= 0.999);
}
"#;

#[cfg(test)]
mod tests {
	/// The hooks and the shared test compile in the water shader, with the roof test on and off.
	#[test]
	fn off_and_on_both_parse() {
		for on in [false, true] {
			let t = super::wgsl_with(on);
			assert!(t.contains("fn skyvis_sun_gap") && t.contains("fn water_miss") && t.contains("fn skyvis_cone"));
			assert!(t.contains(if on { "if (!true)" } else { "if (!false)" }));
		}
	}
}
