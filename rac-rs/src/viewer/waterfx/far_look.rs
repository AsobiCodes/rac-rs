//! [PC] Where water takes its far look (WGSL, appended to `SHADER` in `WaterFx::new`): `fs_water` faded the ripples, the bend,
//! the bed path and the sharp mirror out by the camera distance (FAR_FROM 25 .. FAR_TO 50 units). Kalebo III's camera stands
//! 22-35 units above the sea, so every visible sea point lay inside that band, and a few units of camera height (orbiting the
//! camera, looking more down) switched the whole sea between a light smooth look with big soft reflections and a dark choppy one
//! (user report 2026-10-24, screenshots 35..38). `far_dist` gives the distance the fade reads; `bed_at` the bed when the level's
//! depth behind the water has none (that pixel switched to the opaque path with a hard edge). Both for the geometric sea only
//! (`geo`): other water keeps the plain camera distance and bed (2026-10-25).

pub(super) const FAR_WGSL: &str = r#"
// [PC] The distance the far look of water fades by (FAR_FROM .. FAR_TO, the geometric sea's wave footprint): the camera distance
// on a grazing view (view z <= 0.2: the normal low hero views on Pokitaru, Hoven and Rilgar keep their look exactly), and on a
// view from above the distance a grazing view would have for the same pixel footprint (dist * 0.2 / view z), so the camera
// height alone no longer moves the whole sea across the band; very high views (dist - 60) still go far.
// [C: orbit sweeps of state kal/16_0084 round the hero, pitch -5..-85, runs\ob_kal vs runs\ob_f3, 2026-10-24: one look
// at all pitches with f3, the light/dark switch at 3 units of camera height with the plain distance].
fn far_dist(dist: f32, vz: f32, geo: bool) -> f32 {
	if (!geo) { return dist; }
	return max(dist * min(0.2 / max(vz, 1e-3), 1.0), dist - 60.0);
}

// [PC] The bed under water pixel pf (eye = camera, zs = the level's depth behind it, zw = the water's own depth). Where the depth
// holds no bed (zs at the far plane) the water is deep (40 units down), not a sudden switch to the opaque path with a hard edge.
fn bed_at(eye: vec3<f32>, pf: vec3<f32>, zs: f32, zw: f32, geo: bool) -> vec3<f32> {
	return select(pf - vec3<f32>(0.0, 0.0, 40.0), eye + (pf - eye) * (zs / zw), zs < w.proj.w * 0.98 || !geo);
}
"#;
