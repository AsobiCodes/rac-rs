// [PC] Brightness / Contrast (viewer/picture_adjust.rs `curve`): on display-encoded values in 0..1. k.x = gamma g (brightness),
// k.y = contrast exponent k (an S-curve about mid-grey; black, mid-grey and white fixed). Appended to every shader that grades.
fn grade_curve(c: vec3<f32>, k: vec4<f32>) -> vec3<f32> {
	let x = pow(max(c, vec3<f32>(0.0)), vec3<f32>(k.x));
	let lo = 0.5 * pow(max(2.0 * x, vec3<f32>(0.0)), vec3<f32>(k.y));
	let hi = 1.0 - 0.5 * pow(max(vec3<f32>(2.0) - 2.0 * x, vec3<f32>(0.0)), vec3<f32>(k.y));
	return select(hi, lo, x < vec3<f32>(0.5));
}
