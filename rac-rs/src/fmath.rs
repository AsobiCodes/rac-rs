//! [PC] Float maths for the PC code in retail/ (camera, aim, strafe, stick dead zone, moby state): the same bits on every OS and
//! toolchain. std's `sin`/`cos`/`atan2`/... call the C runtime (Windows UCRT or msvcrt, glibc on Linux), and those round
//! differently in the last bit: measured 2026-10-28 over all 2^32 inputs, UCRT `sinf` differs from libm's f64 `sin` rounded to
//! f32 on 2.9 M of them and glibc on others, so the Kerwan fight replay wrote different camera floats on Linux from field 999.
//! Every function here is the pure-Rust `libm` crate (pinned in Cargo.toml) in f64, rounded once to f32; on the 7 fullcheck
//! replays this gives the old Windows RAM hashes exactly. `sqrt`, `abs`, `floor`, `powi`, `rem_euclid`, `to_radians` and
//! `to_degrees` stay on std: they give the same bits on every target (exact IEEE operations, or the same integer-power loop
//! for `powi`). Some converted callers only draw (camera_arc, strafe/pack_fx, the cull and Euler helpers of moby_state);
//! retail/ uses Fm throughout so one rule holds, kept by `fmath_scan_tests`. A libm version bump can change bits: re-take
//! the test's GOLDEN and the fullcheck replay hashes before taking one.

pub trait Fm: Copy {
	fn fsin(self) -> Self;
	fn fcos(self) -> Self;
	fn ftan(self) -> Self;
	fn fasin(self) -> Self;
	fn fatan(self) -> Self;
	fn fexp(self) -> Self;
	fn fatan2(self, x: Self) -> Self;
	fn fhypot(self, y: Self) -> Self;
	fn fsin_cos(self) -> (Self, Self) {
		(self.fsin(), self.fcos())
	}
}

impl Fm for f32 {
	#[inline]
	fn fsin(self) -> f32 {
		libm::sin(self as f64) as f32
	}
	#[inline]
	fn fcos(self) -> f32 {
		libm::cos(self as f64) as f32
	}
	#[inline]
	fn ftan(self) -> f32 {
		libm::tan(self as f64) as f32
	}
	#[inline]
	fn fasin(self) -> f32 {
		libm::asin(self as f64) as f32
	}
	#[inline]
	fn fatan(self) -> f32 {
		libm::atan(self as f64) as f32
	}
	#[inline]
	fn fexp(self) -> f32 {
		libm::exp(self as f64) as f32
	}
	#[inline]
	fn fatan2(self, x: f32) -> f32 {
		libm::atan2(self as f64, x as f64) as f32
	}
	#[inline]
	fn fhypot(self, y: f32) -> f32 {
		libm::hypot(self as f64, y as f64) as f32
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The bits of every function over 200 000 inputs (camera angles, unit vectors, world distances), hashed. The constant was
	/// taken on Windows; the same test on Linux proves both give the same bits.
	#[test]
	fn the_maths_gives_the_same_bits_on_every_os() {
		let mut h: u64 = 0xcbf29ce484222325;
		let mut put = |v: f32| {
			for b in v.to_bits().to_le_bytes() {
				h ^= b as u64;
				h = h.wrapping_mul(0x100000001b3);
			}
		};
		let mut s: u32 = 0x2545F491;
		for i in 0..200_000u32 {
			s ^= s << 13;
			s ^= s >> 17;
			s ^= s << 5;
			let u = s as f32 / 4294967296.0;
			let x = match i % 3 {
				0 => (u - 0.5) * 2.0 * std::f32::consts::TAU,
				1 => u * 2.0 - 1.0,
				_ => (u - 0.5) * 8192.0,
			};
			let y = ((s >> 7) as f32 / 33554432.0 - 0.5) * 4.0;
			put(x.fsin());
			put(x.fcos());
			put(x.ftan());
			put(x.clamp(-1.0, 1.0).fasin());
			put(x.fatan());
			put((x * 0.01).fexp());
			put(x.fatan2(y));
			put(x.fhypot(y));
		}
		assert_eq!(h, GOLDEN, "fmath bits {h:#018x}");
	}

	/// Windows (MSVC) and Linux (glibc), 2026-10-28.
	const GOLDEN: u64 = 0x6d44bd81666795da;
}
