//! KER-92 (decomp/notes/particles.md "KER-92 port spec"): pool type 0x31, the heli shot tracer (ctor FUN_0025fa48,
//! update FUN_0025fb28, Kerwan) and its draw, mode 2 (one-pixel line) of the shared pool draw FUN_00255768.

use super::*;
use crate::physics::add;

/// Byte +9 of a tracer: near nibble 2 (0.5 world units), far nibble 7 (224 world units).
const B9_TRACER31: u8 = 0x72;
/// Colour words A (head, 0x7fffffff) and B (tail, 0x0fffffff): white, alpha 127 and 15 (over 128).
const ALPHA_HEAD: f32 = 127.0;
const ALPHA_TAIL: f32 = 15.0;

/// Type 0x31 tracer: a two-point segment moving by `vel` each update. `head` is P1 (+0x10), `tail` P2 (+0x20).
#[derive(Clone, Debug)]
pub struct Tracer31 {
	pub head: [f32; 3],
	pub tail: [f32; 3],
}


/// A one-pixel Gouraud line (pool draw mode 2): colour `col_a` at `a`, `col_b` at `b` (alpha over 128), drawn alpha-over
/// with no texture and no fog while the midpoint view depth lies strictly inside (`near`, `far`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
	pub a: [f32; 3],
	pub b: [f32; 3],
	pub col_a: [f32; 4],
	pub col_b: [f32; 4],
	pub near: f32,
	pub far: f32,
}



impl Particles {


	/// The mode-2 line draw feed: one line per live tracer, head (alpha 127) to tail (alpha 15).
	pub fn lines(&self, out: &mut Vec<Line>) {
		let near = (B9_TRACER31 & 15) as f32 / 4.0;
		let far = crate::water::pool_far(B9_TRACER31, f32::MAX);
		for e in self.slots.iter().flatten() {
			if let Effect::Tracer31(t) = e {
				out.push(Line { a: t.head, b: t.tail, col_a: [1.0, 1.0, 1.0, ALPHA_HEAD / FULL], col_b: [1.0, 1.0, 1.0, ALPHA_TAIL / FULL], near, far });
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const S: [f32; 3] = [10.0, 20.0, 5.0];
	const J: [f32; 3] = [12.0, 20.0, 5.0];
	const W: [f32; 3] = [2.0, 0.0, 0.0];

	fn tracer(p: &Particles, i: usize) -> Tracer31 {
		match p.slots[i].as_ref() {
			Some(Effect::Tracer31(t)) => t.clone(),
			_ => panic!("no tracer in slot {i}"),
		}
	}

	fn at(p: [f32; 3], k: f32) -> [f32; 3] {
		let mut q = p;
		for _ in 0..k as usize {
			q = add(q, W);
		}
		q
	}

}
