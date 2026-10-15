use super::*;


impl Particles {



	// ---- robot lightning (class 1471) ------------------------------------------------------





}

/// FUN_002e1bf8 strand frame: x = unit (end - origin), zero when the arc has no length; y = x cross g and z = x cross y with
/// gravity g = (0, 0, -1), both left unnormalised (length = horizontal part of x). Retail draws the strands from this frame
/// (FUN_002e2630) [C].
pub fn arc_frame(origin: [f32; 3], end: [f32; 3]) -> [[f32; 3]; 3] {
	let d = sub(end, origin);
	let x = if len(d) == 0.0 { [0.0; 3] } else { norm(d) };
	let y = cross([0.0, 0.0, 1.0], x);
	[x, y, cross(x, y)]
}

/// A strand point in strand space (along, sideways, up) to world space.
pub fn arc_world(origin: [f32; 3], frame: &[[f32; 3]; 3], p: [f32; 3]) -> [f32; 3] {
	add(origin, add(scale(frame[0], p[0]), add(scale(frame[1], p[1]), scale(frame[2], p[2]))))
}
