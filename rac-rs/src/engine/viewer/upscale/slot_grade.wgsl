// [PC] Brightness / Contrast (viewer/picture_adjust.rs; slot_gpu.rs `graded_shader` appends this to slot_gpu.wgsl, `U` gains `k`,
// and the shared curve `grade_curve`): `fs_copy` with the curve on the colour. Only while a slider is off 50.
@fragment fn fs_copy_grade(@builtin(position) pos: vec4<f32>) -> CopyOut {
	let ws = u.size.zw;
	var o: CopyOut;
	o.colour = vec4<f32>(grade_curve(ld(vec2<i32>(pos.xy), ws), u.k), 1.0);
	let rp = clamp(vec2<i32>(pos.xy * u.size.xy / ws), vec2<i32>(0), vec2<i32>(u.size.xy) - vec2<i32>(1));
	o.depth = textureLoad(depth_ms, rp, 0);
	return o;
}
