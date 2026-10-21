//! The Suck Cannon's per-victim component (decomp/notes/weapons_framework.md WPN-5.1e / 5.1e2): the registration and
//! capture / fire / cancel / clear entry points the held weapon calls on a Peckbot or Amoeboid, and the per-tick state
//! machine `suck_step` (pull, stored, flight, landing). Per-tick units throughout (retail game ticks at 50 Hz).

use crate::physics::{add, cross, dot, scale};












/// 0x2218a8: Hamilton product `a * b`.
pub(super) fn qmul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
	let (av, bv) = ([a[0], a[1], a[2]], [b[0], b[1], b[2]]);
	let v = add(add(scale(bv, a[3]), scale(av, b[3])), cross(av, bv));
	[v[0], v[1], v[2], a[3] * b[3] - dot(av, bv)]
}











