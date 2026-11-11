//! [PC] Fire light v2 stage 5 (lane dlit3, FIRE_LIGHT_V2.md 3.3): CPU twins of flitshadow.wgsl's moment lookup (the radial z of a
//! receiver, the tap mip of a cone, the Hamburger 4-moment solve) and their tests: the solve against exact answers on known depth
//! mixes, through the stored (quantised, half-float) texels of omni_mom.rs.

use super::omni_mom::{optimized, unoptimized, MIPS, MOM};

/// Twin of flitshadow.wgsl `FLS_TEX0`: a mip-0 moment texel's angle at a face's centre (rad).
const TEX0: f32 = 2.0 / MOM as f32;

/// A receiver's z at distance `dist` from the map's origin (flitshadow.wgsl `fls_cone` / omni_mom.wgsl `cs_mom`): radial, over
/// [near, far] with near = max(row near, 0.25) and far = max(row far, 2 near).
pub(in crate::viewer) fn z_of(dist: f32, near_row: f32, far_row: f32) -> f32 {
	let near = near_row.max(super::omni::NEAR);
	let far = far_row.max(2.0 * near);
	(dist - near) / (far - near)
}

/// The mip a cone of half-angle `phi` reads (flitshadow.wgsl `fls_cone`: the texel = the cone, + the lod bias, 0..MIPS - 1).
pub(in crate::viewer) fn lod_of(phi: f32, bias: f32) -> f32 {
	((phi.max(1e-6) / TEX0).log2() + bias).clamp(0.0, (MIPS - 1) as f32)
}

/// Twin of flitshadow.wgsl `fls_ham`: of the depths with moments `b0` (moment bias `mb`), (the share nearer than `zf`, that share
/// x their mean depth).
pub(in crate::viewer) fn hamburger(b0: [f32; 4], zf: f32, mb: f32) -> (f32, f32) {
	let b: [f32; 4] = std::array::from_fn(|i| b0[i] + (0.5 - b0[i]) * mb);
	let d22 = (b[1] - b[0] * b[0]).max(1e-12);
	let l32d22 = b[2] - b[0] * b[1];
	let l32 = l32d22 / d22;
	let d33 = (b[3] - b[1] * b[1] - l32d22 * l32).max(1e-14);
	let c1a = zf - b[0];
	let c2 = (zf * zf - b[1] - l32 * c1a) / d33;
	let c1 = c1a / d22 - l32 * c2;
	let c0 = 1.0 - b[0] * c1 - b[1] * c2;
	let ic2 = 1.0 / if c2.abs() < 1e-12 { 1e-12 } else { c2 };
	let p = c1 * ic2;
	let r = (0.25 * p * p - c0 * ic2).max(0.0).sqrt();
	let (z2, z3) = (-0.5 * p - r, -0.5 * p + r);
	let dn2 = (z2 - zf) * (z2 - z3);
	let w2 = (b[1] - b[0] * (zf + z3) + zf * z3) / if dn2.abs() < 1e-12 { 1e-12 } else { dn2 };
	let wf = (b[1] - b[0] * (z2 + z3) + z2 * z3) / ((zf - z2) * (zf - z3)).max(1e-12);
	if z3 < zf {
		let s = (1.0 - wf).clamp(0.0, 1.0);
		let w3 = s - w2;
		return (s, (w2 * z2 + w3 * z3).clamp(0.0, s * zf));
	}
	if z2 < zf {
		let s = w2.clamp(0.0, 1.0);
		return (s, s * z2.clamp(0.0, zf));
	}
	(0.0, 0.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A half float's rounding (10 mantissa bits, round to nearest): what an Rgba16Float texel keeps.
	fn half(v: f32) -> f32 {
		if v == 0.0 {
			return 0.0;
		}
		let e = v.abs().log2().floor().max(-14.0);
		let q = (2.0f32).powf(e - 10.0);
		(v / q).round() * q
	}

	/// The stored texel of a depth mix (weights, depths), read back as the shader does (optionally through a half float).
	fn mix_moments(mix: &[(f32, f32)], f16: bool) -> [f32; 4] {
		let mut o = [0.0f32; 4];
		for &(w, z) in mix {
			let t = optimized(z);
			for c in 0..4 {
				o[c] += w * t[c];
			}
		}
		if f16 {
			o = o.map(half);
		}
		unoptimized(o)
	}

	#[test]
	fn the_shader_twins_match() {
		let s = include_str!("../flitshadow.wgsl");
		assert!(s.contains(&format!("const FLS_TEX0: f32 = {TEX0};")));
		assert!(s.contains(&format!("const FLS_LODMAX: f32 = {}.0;", MIPS - 1)));
		assert!(s.contains("let b = mix(b0, vec4<f32>(0.5), mb);"));
	}

	#[test]
	fn z_is_radial_over_near_far() {
		assert_eq!(z_of(0.25, 0.0, 10.0), 0.0);
		assert!((z_of(10.0, 0.25, 10.0) - 1.0).abs() < 1e-6);
		assert!((z_of(5.125, 0.25, 10.0) - 0.5).abs() < 1e-6);
		// far below 2 near: far = 2 near.
		assert!((z_of(2.0, 1.0, 1.5) - 1.0).abs() < 1e-6);
	}

	#[test]
	fn the_mip_follows_the_cone() {
		assert_eq!(lod_of(TEX0, 0.0), 0.0);
		assert!((lod_of(4.0 * TEX0, 0.0) - 2.0).abs() < 1e-5);
		assert_eq!(lod_of(1.0, 0.0), (MIPS - 1) as f32, "a wide cone reads the last mip");
		assert_eq!(lod_of(1e-5, 0.0), 0.0);
		assert!((lod_of(2.0 * TEX0, 1.0) - 2.0).abs() < 1e-5);
	}

	/// Two depths (a 2-point mix is fixed by its moments): the share in front of the receiver and its mean depth are exact.
	#[test]
	fn the_solve_is_exact_on_two_depths() {
		for &(wa, za, zb) in &[(0.3f32, 0.2f32, 0.8f32), (0.7, 0.1, 0.5), (0.5, 0.4, 0.95), (0.1, 0.05, 0.6)] {
			let b = mix_moments(&[(wa, za), (1.0 - wa, zb)], false);
			let zf = 0.5 * (za + zb);
			let (s, sz) = hamburger(b, zf, 1e-6);
			assert!((s - wa).abs() < 0.02, "share {s} vs {wa} ({za}, {zb})");
			assert!((sz / s - za).abs() < 0.02, "blocker depth {} vs {za}", sz / s);
			// In front of both: lit; behind both: fully blocked, mean depth the mix's mean.
			assert!(hamburger(b, 0.5 * za, 1e-6).0 < 0.02);
			let (s2, z2) = hamburger(b, 0.5 * (zb + 1.0), 1e-6);
			assert!(s2 > 0.98 && (z2 / s2 - (wa * za + (1.0 - wa) * zb)).abs() < 0.02, "{s2} {}", z2 / s2);
		}
	}

	/// A uniform spread of depths: the solve is a lower bound of the share in front (Hamburger), within its known slack.
	#[test]
	fn the_solve_bounds_a_spread() {
		let mix: Vec<(f32, f32)> = (0..64).map(|i| (1.0 / 64.0, 0.2 + 0.6 * (i as f32 + 0.5) / 64.0)).collect();
		let b = mix_moments(&mix, false);
		for &zf in &[0.3f32, 0.5, 0.7] {
			let truth = (zf - 0.2) / 0.6;
			let (s, _) = hamburger(b, zf, 3e-5);
			assert!(s <= truth + 0.02 && s >= truth - 0.3, "zf {zf}: {s} vs {truth}");
		}
	}

	/// Half-float texels (Rgba16Float, stage 5's format check): over random 3-depth mixes the share moves by little next to the
	/// 4-moment solve's own slack (scratchpad sweep: mean 0.0015 vs 0.04 at moment bias 1e-4), and an occluder 0.3 in front of the
	/// background near z = 1 still shadows a receiver just behind it (at least 0.2 where the two depths are 0.3 apart); a plane's own
	/// depth reads lit.
	#[test]
	fn half_floats_keep_the_solve() {
		let mb = super::super::omni::MBIAS;
		let mut seed = 12345u32;
		let mut rnd = || {
			seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
			(seed >> 8) as f32 / (1u32 << 24) as f32
		};
		let (mut sum, mut n) = (0.0f32, 0);
		for _ in 0..2000 {
			let z = [rnd(), rnd(), rnd()];
			let w = [rnd() + 0.05, rnd() + 0.05, rnd() + 0.05];
			let ws = w[0] + w[1] + w[2];
			let mix: Vec<(f32, f32)> = (0..3).map(|i| (w[i] / ws, z[i])).collect();
			let zf = rnd();
			if z.iter().any(|v| (v - zf).abs() < 0.05) {
				continue;
			}
			sum += (hamburger(mix_moments(&mix, true), zf, mb).0 - hamburger(mix_moments(&mix, false), zf, mb).0).abs();
			n += 1;
		}
		assert!(sum / (n as f32) < 0.01, "mean half-float change {}", sum / n as f32);
		for &(zo, zbk) in &[(0.6f32, 1.0f32), (0.3, 0.98), (0.65, 0.95)] {
			let b = mix_moments(&[(0.5, zo), (0.5, zbk)], true);
			let (s, _) = hamburger(b, zo + 0.05, mb);
			// Moment bias 5e-4 (worker 9, one half-float step: no grain on the sand) costs the hardest mix (0.65 over 0.95, 0.3 apart)
			// half its share (0.24; 0.45 at 1e-4); the wide ones keep ~0.5 (0.42 / 0.48).
			assert!(s > if zo > 0.62 { 0.2 } else { 0.35 }, "occluder {zo}: share {s}");
			let flat = mix_moments(&[(1.0, zo)], true);
			assert!(hamburger(flat, zo + 0.02, mb).0 > 0.8, "a plane just in front still shadows ({zo})");
			assert!(hamburger(flat, zo - 0.01, mb).0 < 0.1, "a plane's own depth reads lit ({zo})");
		}
	}
}
