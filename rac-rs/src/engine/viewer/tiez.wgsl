
// [PC] speed, lane tpre (tiez.rs): the exact tie depth pre-pass, appended to the frame's bindless level text in the pre-pass's own module (same `vs`, `world_texel` and
// `world_alpha` as `fs_tie`, so the same fragments pass the same tests). No colour output (the pipelines write no colour).
// Mark: tie texels that blend AND write Z in `fs_tie` (alpha 0x40..0x7f) mark their samples (stencil): there the pre-pass writes no Z.
@fragment fn fs_tiez_mark(i: VOut) {
	if (i.c.a < -2.0005) { discard; }
	let ra = world_alpha(world_texel(i));
	if (ra * 128.0 < 64.0 || ra >= 1.0) { discard; }
}

// Z: the fully opaque tie texels (alpha 0x80: `fs_tie` overwrites the colour) write their Z on the unmarked samples.
@fragment fn fs_tiez_z(i: VOut) {
	if (i.c.a < -2.0005) { discard; }
	if (world_alpha(world_texel(i)) < 1.0) { discard; }
}

// Restore: one full-screen triangle puts the stencil back to the shadow-volume start value (shadowvol.rs STENCIL_CLEAR).
@vertex fn vs_tiez_full(@builtin(vertex_index) k: u32) -> @builtin(position) vec4<f32> {
	return vec4<f32>(f32((k << 1u) & 2u) * 2.0 - 1.0, f32(k & 2u) * 2.0 - 1.0, 0.0, 1.0);
}

@fragment fn fs_tiez_none() {
}
