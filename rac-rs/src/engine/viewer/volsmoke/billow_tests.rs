//! CPU replicas of `smoke_at`'s noise (shader.rs) on the real noise volume (volfog/noise.rs `generate`, sampled like the GPU:
//! trilinear, repeating): the statistics the shader's constants assume (review R18 m4), the tile blend's continuity where the mean
//! radius crosses a power of two (B1), and the share applied after the clumping (M1) with the shape's knee (R29 M1). Each replica is
//! tied to the WGSL by its text.

use super::shader;
use super::volfog::noise;

/// The noise volume as the shader samples it.
struct Tex {
	n: usize,
	d: Vec<u8>,
}

impl Tex {
	fn new() -> Tex {
		Tex { n: noise::SIZE as usize, d: noise::generate(noise::SIZE) }
	}

	/// `noise_at(p)`: trilinear between texel centres, repeat addressing, 0..1 per channel.
	fn tap(&self, p: [f32; 3]) -> [f32; 4] {
		let n = self.n as i64;
		let mut i0 = [0i64; 3];
		let mut fr = [0f32; 3];
		for k in 0..3 {
			let x = p[k] * self.n as f32 - 0.5;
			let f = x.floor();
			(i0[k], fr[k]) = (f as i64, x - f);
		}
		let mut out = [0f32; 4];
		for c in 0..8 {
			let o = [c & 1, (c >> 1) & 1, (c >> 2) & 1];
			let ix = [0, 1, 2].map(|k| (i0[k] + o[k] as i64).rem_euclid(n) as usize);
			let w: f32 = (0..3).map(|k| if o[k] == 1 { fr[k] } else { 1.0 - fr[k] }).product();
			let at = ((ix[2] * self.n + ix[1]) * self.n + ix[0]) * 4;
			for ch in 0..4 {
				out[ch] += w * self.d[at + ch] as f32 / 255.0;
			}
		}
		out
	}
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
	[a[0] * k, a[1] * k, a[2] * k]
}

/// The shader's noise statistics (the literals in `smoke_at`).
const BIG: (f32, f32) = (0.488, 0.132);
const MED: (f32, f32) = (0.484, 0.171);
const DET_STD: f32 = 0.121;

/// `smoke_at`'s big and medium lumps at noise coordinate `q` and tile level `lv` (medium at full band weight). `parity` = the
/// shader's offsets (by the parity of the level); false = the old ones (fixed per slot) the review found jumping.
fn lumps(t: &Tex, q: [f32; 3], lv: f32, parity: bool) -> (f32, f32) {
	let l0 = lv.floor();
	let fk = lv - l0;
	let ta = l0.exp2();
	let odd = parity && (l0 as i32 & 1) != 0;
	let pick = |e: [f32; 3], o: [f32; 3]| if odd { (o, e) } else { (e, o) };
	let (a, b) = pick([0.37, 0.11, 0.73], [0.61, 0.83, 0.19]);
	let na = t.tap(add(scale(q, 1.0 / ta), a));
	let nb = t.tap(add(scale(q, 1.0 / (ta * 2.0)), b));
	let vn = 1.0 / (fk * fk + (1.0 - fk) * (1.0 - fk)).sqrt();
	let mix = |x: f32, y: f32| x + (y - x) * fk;
	let big = (mix(na[1] * 0.75 + na[0] * 0.25, nb[1] * 0.75 + nb[0] * 0.25) - BIG.0) * vn / BIG.1;
	let (ma, mb) = pick([0.71, 0.29, 0.53], [0.23, 0.47, 0.91]);
	let ma = t.tap(add(scale(q, 3.0 / ta), ma))[1];
	let mb = t.tap(add(scale(q, 1.5 / ta), mb))[1];
	(big, (mix(ma, mb) - MED.0) * vn / MED.1)
}

/// The replicas are the WGSL's.
#[test]
fn the_replicas_are_the_shader() {
	let src = shader::built_in();
	for line in [
		"let lv = min(log2(s * max(rad * inv, 0.25)), 9.99);",
		"let odd = (i32(l0) & 1) != 0;",
		"let na = noise_at(q / ta + select(oe, oo, odd));",
		"let nb = noise_at(q / (ta * 2.0) + select(oo, oe, odd));",
		"let ma = noise_at(q * 3.0 / ta + select(me, mo, odd)).g;",
		"let mb = noise_at(q * 1.5 / ta + select(mo, me, odd)).g;",
		"- 0.488) * vn * (1.0 / 0.132);",
		"- 0.484) * vn * (w2 / 0.171);",
		"(det - 0.5) * (0.05 / 0.121)",
		"let q = p - g.flow.xyz;",
		"noise_at((p - g.flowd.xyz) / g.tune.z",
		"let w = cov * vv.col.w * vv.c.w * vv.glow.w + 1.0e-6;",
		"let ce = cov * min(vv.glow.w * 4.0, 1.0);",
		"miss *= 1.0 - ce;",
		"smiss *= 1.0 - cov * vv.glow.w;",
		"ext += w * vv.col.w;",
		"age += w * vv.mo.z;",
		"bil += w * vv.mo.y;",
		"let cov = 1.0 - miss;",
		"let shr = (1.0 - smiss) / max(cov, 1.0e-6);",
		"let e0 = ext * inv;",
		"let clump = clamp(0.6 / max(e0, 1.0e-4), 1.0, 4.0);",
		"let er = g.amb.w * (0.8 + 0.4 * age * inv) * (0.75 + 0.25 * bil * inv) * (0.55 + 0.2 * clump);",
		"var d = cov - (1.0 - lump) * er;",
		"let dens = smoothstep(0.0, 0.1, d) * (0.6 + 0.4 * smoothstep(0.05, 0.4, d)) * e0 * clump * shr * g.tune.x;",
	] {
		assert!(src.contains(line), "shader.rs lost: {line}");
	}
	assert!(src.contains(&format!("), {});", super::flow::LV_MAX)), "the tile level bound is flow.rs LV_MAX");
	// The uniform fields the CPU no longer fills (volsmoke.rs `uniform` / `extra` write 0; review R29 m2): nothing reads them.
	for dead in ["tune3.x", "g.tune.w", "f.wind"] {
		assert!(!src.contains(dead), "the smoke's WGSL reads {dead}, which the CPU leaves 0");
	}
}

/// Review R18 m4: the shader's mean / deviation literals are those of the noise volume as the GPU samples it (a change of the fog's
/// noise must not silently flatten or over-sharpen the smoke's billows).
#[test]
fn the_noise_statistics_match_the_shader_constants() {
	let t = Tex::new();
	let mut seed = 0x2545_f491u32;
	let mut rnd = || {
		seed ^= seed << 13;
		seed ^= seed >> 17;
		seed ^= seed << 5;
		seed as f32 / u32::MAX as f32
	};
	let n = 40_000;
	let (mut s, mut s2) = ([0f64; 3], [0f64; 3]);
	for _ in 0..n {
		let c = t.tap([rnd(), rnd(), rnd()]);
		let v = [c[1] * 0.75 + c[0] * 0.25, c[1], c[2]];
		for k in 0..3 {
			(s[k], s2[k]) = (s[k] + v[k] as f64, s2[k] + (v[k] * v[k]) as f64);
		}
	}
	let stat = |k: usize| {
		let m = s[k] / n as f64;
		(m as f32, ((s2[k] / n as f64 - m * m).max(0.0)).sqrt() as f32)
	};
	let (bm, bs) = stat(0);
	let (mm, ms) = stat(1);
	let (dm, ds) = stat(2);
	assert!((bm - BIG.0).abs() < 0.015 && (bs / BIG.1 - 1.0).abs() < 0.12, "0.75 G + 0.25 R: mean {bm} std {bs} vs {BIG:?}");
	assert!((mm - MED.0).abs() < 0.015 && (ms / MED.1 - 1.0).abs() < 0.12, "G: mean {mm} std {ms} vs {MED:?}");
	assert!((dm - 0.5).abs() < 0.03 && (ds / DET_STD - 1.0).abs() < 0.12, "B: mean {dm} std {ds} vs 0.5 / {DET_STD}");
}

/// Review R18 B1: across an integer tile level (the mean radius crossing a power of two / smoke_scale) the big and the medium lumps
/// do not jump (no seam where weighted radii meet, no pop when a merged volume's radius eases across); the old per-slot offsets did.
#[test]
fn the_tile_blend_is_continuous_across_a_power_of_two() {
	let t = Tex::new();
	let (mut worst, mut old) = (0f32, 0f32);
	for i in 0..64 {
		let q = [3.1 + i as f32 * 1.37, -7.9 + i as f32 * 0.61, 12.3 + i as f32 * 2.03];
		for l in -1..=8 {
			let (lo, hi) = (l as f32 - 1.0e-4, l as f32 + 1.0e-4);
			let (a, b) = (lumps(&t, q, lo, true), lumps(&t, q, hi, true));
			worst = worst.max((a.0 - b.0).abs()).max((a.1 - b.1).abs());
			let (a, b) = (lumps(&t, q, lo, false), lumps(&t, q, hi, false));
			old = old.max((a.0 - b.0).abs()).max((a.1 - b.1).abs());
		}
	}
	assert!(worst < 0.02, "the lumps jump by {worst} std at a power of two");
	assert!(old > 0.5, "the test bites: the old offsets jumped by {old} std");
	// Inside a level the blend is smooth too: a sweep of lv in steps of 1/200 never moves a lump by more than a small step.
	let q = [5.3, 2.2, -4.4];
	let mut last = lumps(&t, q, 1.5, true);
	for k in 1..=600 {
		let cur = lumps(&t, q, 1.5 + k as f32 / 200.0, true);
		assert!((cur.0 - last.0).abs() < 0.1 && (cur.1 - last.1).abs() < 0.15, "step {k}: {last:?} -> {cur:?}");
		last = cur;
	}
}

/// One hit volume of `smoke_at` at a point: coverage (1 at the centre .. 0 at the surface), share (`glow.w`), extinction (`col.w`),
/// radius, age, billow.
#[derive(Clone, Copy)]
struct Hit {
	cov: f32,
	share: f32,
	ext: f32,
	rad: f32,
	age: f32,
	bil: f32,
}

fn hit(cov: f32, share: f32, ext: f32, rad: f32) -> Hit {
	Hit { cov, share, ext, rad, age: 0.3, bil: 0.4 }
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
	let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
	t * t * (3.0 - 2.0 * t)
}

/// `smoke_at`'s joined coverage and share factor over `hits`. `knee`: the shape takes a volume by min(4 x share, 1) (the shader);
/// false = whole (before review R29 M1).
fn shape(hits: &[Hit], knee: bool) -> (f32, f32) {
	let (mut miss, mut smiss) = (1.0f32, 1.0f32);
	for h in hits {
		let ce = if knee { h.cov * (h.share * 4.0).min(1.0) } else { h.cov };
		(miss, smiss) = (miss * (1.0 - ce), smiss * (1.0 - h.cov * h.share));
	}
	let cov = 1.0 - miss;
	(cov, (1.0 - smiss) / cov.max(1.0e-6))
}

/// `smoke_at`'s density (gain 1, erosion 0.8, without the wisps) where the billows are `lump` (0 crevice .. 1 lump top).
fn density(hits: &[Hit], lump: f32, knee: bool) -> f32 {
	let (mut wsum, mut ext, mut age, mut bil) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
	for h in hits {
		let w = h.cov * h.ext * h.rad * h.share + 1.0e-6;
		(wsum, ext, age, bil) = (wsum + w, ext + w * h.ext, age + w * h.age, bil + w * h.bil);
	}
	let inv = 1.0 / wsum;
	let (cov, shr) = shape(hits, knee);
	let e0 = ext * inv;
	let clump = (0.6 / e0.max(1.0e-4)).clamp(1.0, 4.0);
	let er = 0.8 * (0.8 + 0.4 * age * inv) * (0.75 + 0.25 * bil * inv) * (0.55 + 0.2 * clump);
	let d = cov - (1.0 - lump) * er;
	smooth(0.0, 0.1, d) * (0.6 + 0.4 * smooth(0.05, 0.4, d)) * e0 * clump * shr
}

/// Review R18 M1 / R29 M1: the density is the full puff's times its share (applied after the clump, which sees the unshared
/// extinction). A lone band puff from share 0.25 draws exactly that share of its full density (it cross-fades with its sprite at
/// 1 - share); below 0.25 its shape erodes (summed over the puff and the billows: less than the share, ~0 at share ~0) while its
/// sprite (keep >= 0.75) carries the look. Two full volumes stay full, and the share factor never exceeds 1.
#[test]
fn the_share_scales_the_density_after_the_clump() {
	let lumps = (0..=10).map(|i| i as f32 / 10.0);
	for s in [0.25, 0.5, 0.9, 1.0] {
		for c in [0.02, 0.3, 0.9] {
			for l in lumps.clone() {
				let (part, full) = (density(&[hit(c, s, 1.0, 2.0)], l, true), density(&[hit(c, 1.0, 1.0, 2.0)], l, true));
				assert!((part - s * full).abs() < 1.0e-5, "lone puff cov {c} share {s} lump {l}: {part} vs {}", s * full);
			}
		}
	}
	// Below the knee: the puff summed over its radius (the r^2 shells) and the billows draws at most its share, ~nothing at ~0.
	let summed = |s: f32, ext: f32| -> f32 {
		let r = |i: i32| (i as f32 + 0.5) / 20.0;
		(0..20).map(|i| lumps.clone().map(|l| r(i) * r(i) * density(&[hit(1.0 - r(i), s, ext, 2.0)], l, true)).sum::<f32>()).sum()
	};
	for ext in [0.2, 0.6, 1.5] {
		let full = summed(1.0, ext);
		for s in [0.01, 0.05, 0.1, 0.2, 0.24] {
			assert!(summed(s, ext) <= s * full * 1.0001, "share {s} ext {ext}: {} > {}", summed(s, ext), s * full);
		}
		assert!(summed(1.0e-4, ext) < 1.0e-3 * full, "a puff at share ~0 draws ~nothing");
	}
	assert!((shape(&[hit(0.6, 1.0, 1.0, 1.0), hit(0.4, 1.0, 1.0, 1.0)], true).1 - 1.0).abs() < 1.0e-6);
	for s in [1.0e-4, 0.05, 0.2, 0.3, 0.7] {
		for c in [0.1, 0.5, 0.9] {
			assert!(shape(&[hit(0.3, 1.0, 1.0, 1.0), hit(c, s, 1.0, 1.0)], true).1 <= 1.0 + 1.0e-6);
			assert!(shape(&[hit(c, s, 1.0, 1.0)], true).1 <= 1.0 + 1.0e-6);
		}
	}
}

/// Review R29 M1: a band puff entering at share ~0 leaves the volume it overlaps as it was (no step in its billows or thickness on
/// the frame it enters): A alone vs A + B at share 1e-6 within 1e-3, over A's dense part, its outer layer and its crevices (the
/// change is linear in the share: ~1e-2 at 1e-4 where the smoothstep is steepest). Counted whole in the shape (before the knee), B
/// thinned A (11 % at A 0.6, B 0.2) and filled A's crevices (~0.35).
#[test]
fn a_puff_entering_at_share_zero_leaves_the_smoke_it_overlaps_as_it_was() {
	let (mut worst, mut old) = (0f32, 0f32);
	for ca in [0.1, 0.3, 0.6, 0.9] {
		for cb in [0.1, 0.5, 0.9] {
			for (ea, eb, ra, rb) in [(0.3, 1.5, 1.0, 3.0), (1.5, 0.3, 3.0, 1.0), (1.0, 1.0, 2.0, 2.0), (0.3, 0.3, 1.0, 1.0)] {
				for l in (0..=10).map(|i| i as f32 / 10.0) {
					let a = density(&[hit(ca, 1.0, ea, ra)], l, true);
					let ab = [hit(ca, 1.0, ea, ra), hit(cb, 1.0e-6, eb, rb)];
					worst = worst.max((density(&ab, l, true) - a).abs());
					old = old.max((density(&ab, l, false) - a).abs());
				}
			}
		}
	}
	assert!(worst < 1.0e-3, "a share-0 puff changes the smoke it overlaps by {worst}");
	assert!(old > 0.3, "the test bites: counted whole in the shape it changed it by {old}");
	// The review's cases. A's dense part (A 0.6, B 0.2 at share 0.1, a lump top): the old shape thinned it by 11 %, the knee by
	// less than 4 % (B at share 0.1 is in the shape by 0.4). A crevice of A's outer layer (A 0.3, B 0.8, lump 0.3: no density
	// without B) stays empty.
	let dense = |knee| density(&[hit(0.6, 1.0, 1.0, 2.0), hit(0.2, 0.1, 1.0, 2.0)], 1.0, knee);
	let alone = density(&[hit(0.6, 1.0, 1.0, 2.0)], 1.0, true);
	assert!(alone - dense(false) > 0.1 * alone && alone - dense(true) < 0.04 * alone, "{alone} {} {}", dense(true), dense(false));
	assert_eq!(density(&[hit(0.3, 1.0, 1.0, 2.0)], 0.3, true), 0.0);
	assert!(density(&[hit(0.3, 1.0, 1.0, 2.0), hit(0.8, 1.0e-5, 1.0, 2.0)], 0.3, true) < 1.0e-6);
	assert!(density(&[hit(0.3, 1.0, 1.0, 2.0), hit(0.8, 1.0e-5, 1.0, 2.0)], 0.3, false) > 0.3);
}
