//! [PC] Parts carried onto a redrawn hero pose, and 3x4 affine helpers (engine split stage E2 batch 4b, 2026-10-26: moved out
//! of rac1/retail/strafe.rs, which builds the `PoseFix` and re-exports these): the smoothing draw (viewer/smooth.rs) moves each
//! attached part with the joint nearest it.

/// Retail hangs Clank, the back object, the head items, the bulb mesh and the hero glows on ITS pose of the hero (the clip
/// playing forward, no twist, its leg bends), `turn_parts` only turns them to the drawn yaw. While the strafe draw changes the
/// pose (the backpedal plays the run clip reversed, so the whole upper body bobs and leans at another phase) they floated off
/// the drawn body: Clank low on the back. `PoseFix` holds, per joint the twist leaves alone (the upper body), where retail's pose
/// has it and its move onto the drawn pose (hero model space, 3x4 row major like the skin matrices); a part follows the move of
/// its nearest such joint (viewer/smooth.rs `attach`, `hero_point`). The leg joints are kept apart (`legs`): only the Magneboots
/// ride them (`place_leg`), the parts near the hips (Clank, the pack) stay on the upper body. Draw only.
pub struct PoseFix {
	pub joints: Vec<([f32; 3], [f32; 12])>,
	pub legs: Vec<([f32; 3], [f32; 12])>,
}

impl PoseFix {
	fn near(&self, p: [f32; 3]) -> Option<&[f32; 12]> {
		nearest(&self.joints, p)
	}

	/// A part retail placed on a leg (a Magneboot on its foot, hero model space), carried onto the drawn leg by the leg joint
	/// nearest its origin: the strafe draw twists the legs toward the path and plays its own leg clip.
	pub fn place_leg(&self, rel: [f32; 16]) -> [f32; 16] {
		nearest(&self.legs, [rel[12], rel[13], rel[14]]).map_or(rel, |m| place34(m, rel))
	}

	/// A point retail placed on the hero (hero model space), carried onto the drawn pose.
	pub fn point(&self, p: [f32; 3]) -> [f32; 3] {
		self.near(p).map_or(p, |m| mul34(m, p, 1.0))
	}

	/// A part's placement (column-major 4x4, hero model space), carried onto the drawn pose by the joint nearest its origin.
	pub fn place(&self, rel: [f32; 16]) -> [f32; 16] {
		self.near([rel[12], rel[13], rel[14]]).map_or(rel, |m| place34(m, rel))
	}
}

/// `m * rel` of a 3x4 row-major affine matrix and a column-major 4x4 placement.
pub fn place34(m: &[f32; 12], rel: [f32; 16]) -> [f32; 16] {
	let mut o = rel;
	for c in 0..4 {
		let v = mul34(m, [rel[c * 4], rel[c * 4 + 1], rel[c * 4 + 2]], if c == 3 { 1.0 } else { 0.0 });
		o[c * 4..c * 4 + 3].copy_from_slice(&v);
	}
	o
}

/// `m * (v, w)` of a 3x4 row-major affine matrix.
pub fn mul34(m: &[f32; 12], v: [f32; 3], w: f32) -> [f32; 3] {
	std::array::from_fn(|r| m[r * 4] * v[0] + m[r * 4 + 1] * v[1] + m[r * 4 + 2] * v[2] + m[r * 4 + 3] * w)
}

/// `a * inverse(b)` of two 3x4 row-major affine matrices (None when `b` is singular).
pub fn mul_inv34(a: &[f32; 12], b: &[f32; 12]) -> Option<[f32; 12]> {
	let (p, q, r, s, t, u, v, w, x) = (b[0], b[1], b[2], b[4], b[5], b[6], b[8], b[9], b[10]);
	let det = p * (t * x - u * w) - q * (s * x - u * v) + r * (s * w - t * v);
	if det.abs() < 1e-12 {
		return None;
	}
	let k = 1.0 / det;
	let i = [(t * x - u * w) * k, (r * w - q * x) * k, (q * u - r * t) * k, (u * v - s * x) * k, (p * x - r * v) * k, (r * s - p * u) * k, (s * w - t * v) * k, (q * v - p * w) * k, (p * t - q * s) * k];
	let mut inv = [0.0; 12];
	for row in 0..3 {
		inv[row * 4..row * 4 + 3].copy_from_slice(&i[row * 3..row * 3 + 3]);
		inv[row * 4 + 3] = -(i[row * 3] * b[3] + i[row * 3 + 1] * b[7] + i[row * 3 + 2] * b[11]);
	}
	Some(std::array::from_fn(|n| {
		let (row, col) = (n / 4, n % 4);
		a[row * 4] * inv[col] + a[row * 4 + 1] * inv[4 + col] + a[row * 4 + 2] * inv[8 + col] + if col == 3 { a[row * 4 + 3] } else { 0.0 }
	}))
}

/// The move of the joint in `joints` nearest `p`.
fn nearest(joints: &[([f32; 3], [f32; 12])], p: [f32; 3]) -> Option<&[f32; 12]> {
	let d2 = |a: &[f32; 3]| (a[0] - p[0]).powi(2) + (a[1] - p[1]).powi(2) + (a[2] - p[2]).powi(2);
	joints.iter().min_by(|a, b| d2(&a.0).total_cmp(&d2(&b.0))).map(|j| &j.1)
}
