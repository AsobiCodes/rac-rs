// GS TEX1 LCM 0: keep the depth LOD, K bias and MXL cap. Smooth uses fractional levels;
// Classic preserves the retail LINEAR_MIPMAP_NEAREST expression exactly.
// sun_q.w is unused by shadows and is set after their uniforms (0 Classic, 1 Smooth).
fn gs_level(i: VOut) -> f32 {
	let lod = log2(32.0 / i.pos.w) + i.kmx.x;
	if (u.sun_q.w > 0.5) { return clamp(lod, 0.0, max(i.kmx.y, 0.0)); }
	var level = 0.0;
	if (lod > 0.0) { level = min(floor(lod + 0.5), i.kmx.y); }
	return level;
}

