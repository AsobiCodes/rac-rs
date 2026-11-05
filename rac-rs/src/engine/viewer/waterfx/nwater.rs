//! [PC] Exact cuts of the water shade pass (lane nwater, 2026-10-29; WGSL appended to `SHADER` in `WaterFx::new`): helper
//! functions the cuts in `fs_water` call (`mirror_layer`: the sea asks its mirror layer before the wave sum so it can drop the
//! slope that layer never reads; `twinkle_at`: the glint's noise, evaluated only where the glint is not 0). The picture is
//! byte for byte the one of before; the proofs are in `scratch_v042\night_tools\REPORT_nwater.md`.

pub(super) const NWATER_WGSL: &str = r#"
// [PC] perf (lane nwater, 2026-10-29): helpers of exact cuts. Same arithmetic as the code they were lifted from.
// The mirrored plane layer (-1 = none) of a water pixel at height z whose surface normal has up component gz (retail's flat water on
// a mirrored plane, `fs_water`). Pure in z, gz and the uniforms: the geometric sea (gz = 1) asks it before its wave sum.
fn mirror_layer(z: f32, gz: f32) -> i32 {
	var ml = -1;
	var mbest = 0.4;
	if (w.mirror.y > 0.5 && gz > 0.96) {
		if (abs(z - w.mirror.x) < mbest) {
			ml = 0;
			mbest = abs(z - w.mirror.x);
		}
		if (w.mirror2.z > 1.5 && abs(z - w.mirror2.x) < mbest) {
			ml = 1;
			mbest = abs(z - w.mirror2.x);
		}
		if (w.mirror2.z > 2.5 && abs(z - w.mirror2.y) < mbest) {
			ml = 2;
			mbest = abs(z - w.mirror2.y);
		}
	}
	return ml;
}
// The glint's twinkle factor (0.25..1) at wave-plane XY q.
fn twinkle_at(q: vec2<f32>, time: f32) -> f32 {
	return 0.25 + 0.75 * smoothstep(0.7, 0.95, vnoise(q * 5.0 + vec2<f32>(time * 1.3, -time * 0.9)));
}
"#;
