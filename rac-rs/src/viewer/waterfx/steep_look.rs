//! [PC] The geometric sea's waves seen from above (WGSL, appended to `SHADER` in `WaterFx::new`): the waves were in the normals
//! at every camera pitch, but from a steep view the Fresnel reflection is ~0.02 (no reflection to bend), the grid displacement
//! cannot be seen and the face light (`GEO_FACE * dot(wsd, lh)`, about +-20 %) was the only cue left, so tilting the camera down
//! (Kalebo III walkways, Pokitaru with a high camera) made the near sea look flat (user reports 2026-10-24). `fs_water` takes its
//! face light, crest light and reflection share through these; grazing views (view z <= STEEP_LO) keep their look exactly.
//! Geometric sea only (`geo`; also SEA_TRAIN_FAR in `fs_water`): on other water they put waves on Novalis' calm cave pools
//! (user report 2026-10-25).

pub(super) const STEEP_WGSL: &str = r#"
// [PC] Steep-view wave shading (lane ywmx, 2026-10-24; picked from pitch strips and water-only pan pops on Kalebo and Pokitaru,
// runs\kp_*, pkp_*, kp2_*): the view z (up component of the unit vector from the water to the eye) where it starts and is full.
const STEEP_LO: f32 = 0.25;
const STEEP_HI: f32 = 0.75;
// Extra face light at a full steep view: the face term times (1 + STEEP_FACE).
const STEEP_FACE: f32 = 1.5;
// Crest light at a full steep view: + STEEP_CREST * wave height (clamped to +-1), crests lighter, troughs darker.
const STEEP_CREST: f32 = 0.12;
// Reflection share a wave slope adds at a full steep view: STEEP_SHEEN * (view z - n.v), so a tilted face shows the sky.
const STEEP_SHEEN: f32 = 0.6;
// How far the shading trains of the far geometric sea fade (their footprint grows by 1 + SEA_TRAIN_FAR at 40 units of far
// distance; it was 3 like the grid's own, which smoothed the far sea flat from above; 1 keeps the pan pops at ~1.2 permil).
const SEA_TRAIN_FAR: f32 = 1.0;

fn steep_k(vz: f32, geo: bool) -> f32 {
	if (!geo) { return 0.0; }
	return smoothstep(STEEP_LO, STEEP_HI, vz);
}

// The face light factor of the sea's wave slope (times GEO_FACE / WAVE_FACE).
fn steep_face(vz: f32, geo: bool) -> f32 {
	return 1.0 + STEEP_FACE * steep_k(vz, geo);
}

// The crest light added at wave height wh.
fn steep_crest(vz: f32, wh: f32, geo: bool) -> f32 {
	return STEEP_CREST * steep_k(vz, geo) * clamp(wh, -1.0, 1.0);
}

// The reflection share: Fresnel plus the wave slope's sheen (ndv = n.v of the waved normal).
fn steep_fresnel(fresnel: f32, vz: f32, ndv: f32, geo: bool) -> f32 {
	return fresnel + STEEP_SHEEN * steep_k(vz, geo) * max(vz - ndv, 0.0);
}
"#;
