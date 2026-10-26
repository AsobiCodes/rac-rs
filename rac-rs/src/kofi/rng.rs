//! Ko-fi drawings: the seeded random numbers, smooth value noise and the sine / cosine the strokes use. Everything here is plain
//! integer and f32 arithmetic (no libm calls), so the pictures are the same on every machine and every run.

/// splitmix64: a small deterministic generator.
pub struct Rng(u64);

impl Rng {
	pub fn new(seed: u64) -> Rng {
		Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
	}

	pub fn next_u64(&mut self) -> u64 {
		self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
		let mut z = self.0;
		z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
		z ^ (z >> 31)
	}

	/// 0 .. 1
	pub fn f(&mut self) -> f32 {
		(self.next_u64() >> 40) as f32 / 16_777_216.0
	}

	/// a .. b
	pub fn range(&mut self, a: f32, b: f32) -> f32 {
		a + (b - a) * self.f()
	}

	/// -1 .. 1
	pub fn s(&mut self) -> f32 {
		self.f() * 2.0 - 1.0
	}

	/// A fresh generator for a sub-task (strokes of one shape keep their look when another shape changes).
	pub fn fork(&mut self) -> Rng {
		Rng::new(self.next_u64())
	}
}

/// A hash of an integer lattice point and a seed, 0 .. 1.
pub fn hash2(x: i32, y: i32, seed: u32) -> f32 {
	let mut h = (x as u32).wrapping_mul(0x27D4_EB2D) ^ (y as u32).wrapping_mul(0x1656_67B1) ^ seed.wrapping_mul(0x9E37_79B1);
	h ^= h >> 15;
	h = h.wrapping_mul(0x85EB_CA6B);
	h ^= h >> 13;
	h = h.wrapping_mul(0xC2B2_AE35);
	h ^= h >> 16;
	(h >> 8) as f32 / 16_777_216.0
}

fn smooth(t: f32) -> f32 {
	t * t * (3.0 - 2.0 * t)
}

/// Smooth 1D value noise, -1 .. 1, about one wiggle per unit of `x`.
pub fn noise1(x: f32, seed: u32) -> f32 {
	let i = x.floor();
	let t = smooth(x - i);
	let a = hash2(i as i32, 17, seed) * 2.0 - 1.0;
	let b = hash2(i as i32 + 1, 17, seed) * 2.0 - 1.0;
	a + (b - a) * t
}

/// Smooth 2D value noise, 0 .. 1.
pub fn noise2(x: f32, y: f32, seed: u32) -> f32 {
	let (ix, iy) = (x.floor(), y.floor());
	let (tx, ty) = (smooth(x - ix), smooth(y - iy));
	let (ix, iy) = (ix as i32, iy as i32);
	let a = hash2(ix, iy, seed);
	let b = hash2(ix + 1, iy, seed);
	let c = hash2(ix, iy + 1, seed);
	let d = hash2(ix + 1, iy + 1, seed);
	let top = a + (b - a) * tx;
	let bot = c + (d - c) * tx;
	top + (bot - top) * ty
}

const TAU: f32 = 6.283_185_5;

/// Sine of an angle in radians (reduced to -pi/2 .. pi/2, then a Taylor polynomial; error below 1e-5).
pub fn sin(a: f32) -> f32 {
	let mut x = a - TAU * (a / TAU + 0.5).floor();
	let half = TAU / 2.0;
	if x > half / 2.0 {
		x = half - x;
	} else if x < -half / 2.0 {
		x = -half - x;
	}
	let x2 = x * x;
	x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0 * (1.0 - x2 / 110.0)))))
}

pub fn cos(a: f32) -> f32 {
	sin(a + TAU / 4.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sin_cos_close_to_std() {
		for k in -400..400 {
			let a = k as f32 * 0.037;
			assert!((sin(a) - a.sin()).abs() < 2e-5, "sin {a}");
			assert!((cos(a) - a.cos()).abs() < 2e-5, "cos {a}");
		}
	}

	#[test]
	fn generator_is_deterministic() {
		let mut a = Rng::new(7);
		let mut b = Rng::new(7);
		for _ in 0..50 {
			assert_eq!(a.next_u64(), b.next_u64());
		}
		assert!(noise1(3.3, 5) >= -1.0 && noise1(3.3, 5) <= 1.0);
	}
}
