//! PS2 Insomniac engine maths the R&C games share (engine split stage E2, 2026-10-26: moved out of rac1/camera.rs so the
//! formats and the engine need not reach into the R&C1 pack): the retail projection scalar and the VU0 sine / cosine
//! microprogram. `camera::{PROJ_SCALAR, fast_cos, fast_sin}` still work (re-exported there).

/// The retail horizontal projection scalar (tan of the half horizontal FOV, decomp/notes/camera.md
/// "REN-7b port spec"): follow, free-look and pause cameras always use 0.63 (0x3f2147ae).
pub const PROJ_SCALAR: f32 = 0.63;

/// Native VU0 cosine of the shake curve (camera.md "NOV-5.3"): cos(x) = sin(x + pi/2) via the
/// odd sine polynomial, argument already wrapped to [-pi, pi) by the caller. Operation order
/// mirrors the microprogram accumulator (u, then c3..c7 terms, then c9).
pub fn fast_cos(x: f32) -> f32 {
	const H: f32 = f32::from_bits(0x3fc9_0fda);
	vu_sine(x + H)
}

/// FUN_00221240 = VU0 entry 0xc90 (the same microprogram past the `+ pi/2` step, vu0_rotation.txt): the odd sine
/// polynomial of the argument itself, wrapped to [-pi, pi) by the caller.
pub fn fast_sin(x: f32) -> f32 {
	vu_sine(x)
}

fn vu_sine(a: f32) -> f32 {
	const P: f32 = f32::from_bits(0x4049_0fda);
	const C3: f32 = f32::from_bits(0xbe2a_aaa4);
	const C5: f32 = f32::from_bits(0x3c08_873e);
	const C7: f32 = f32::from_bits(0xb94f_b21d);
	const C9: f32 = f32::from_bits(0x362e_9c14);
	let mut u = a.min(P - a);
	u = u.max(-P - a);
	let u2 = u * u;
	let u3 = u * u2;
	let u5 = u3 * u2;
	let u7 = u5 * u2;
	let u9 = u7 * u2;
	let mut r = u;
	r += C3 * u3;
	r += C5 * u5;
	r += C7 * u7;
	r + C9 * u9
}

/// The retail fast `atan2(b, a)` (FUN_002212f0(a, b), VU0 polynomial; NOV-14.5d): the ratio `t = (m - M)/(m + M)` of
/// the smaller and larger magnitude, an odd 15th-degree polynomial in `t` plus pi/4, then the octant row.
pub fn fast_atan2(a: f32, b: f32) -> f32 {
	const C: [f32; 8] = [0.9999993, -0.3332986, 0.1994654, -0.1390853, 0.09642004, -0.05590989, 0.02186123, -0.004054058];
	// Per row `2 * sign(a) + sign(b)`: (mul, add) for |a| >= |b|, then for |a| < |b| (table 0x1c2840).
	const ROWS: [[f32; 4]; 4] = [[1.0, 0.0, -1.0, 1.570796], [-1.0, 0.0, 1.0, -1.570796], [-1.0, 3.141593, 1.0, 1.570796], [1.0, -3.141593, -1.0, -1.570796]];
	let (m, big) = (a.abs().min(b.abs()), a.abs().max(b.abs()));
	if big <= 0.0 {
		return 0.0;
	}
	let t = (m - big) / (m + big);
	let (t2, t4) = (t * t, (t * t) * (t * t));
	let t8 = t4 * t4;
	let p = [t, t * t2, t * t4, (t * t4) * t2];
	let mut acc = p[0] * C[0];
	for i in 1..4 {
		acc += p[i] * C[i];
	}
	for i in 0..4 {
		acc += (p[i] * t8) * C[4 + i];
	}
	let r = acc + 0.7853982;
	let row = ROWS[2 * a.is_sign_negative() as usize + b.is_sign_negative() as usize];
	let (mul, add) = if (a.abs() - b.abs()).is_sign_negative() { (row[2], row[3]) } else { (row[0], row[1]) };
	r * mul + add
}
