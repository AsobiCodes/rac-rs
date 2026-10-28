//! Vegetation (viewer/vegfx.rs): the plant meshes built in code, unit height, in model space: the grass clump, the seaweed ribbon
//! clump and the leaf cluster for tree crowns and bushes. Pure vertex / index builders (seeded `Rng`, no GPU, no level data).

use super::*;

/// One clump of `n` blades (unit height; `w` base width m, `r` clump radius m).
pub(super) fn clump(n: usize, w: f32, r: f32, heads: usize, verts: &mut Vec<VegVertex>, idx: &mut Vec<u32>) {
	let mut g = Rng::new(0x6772_6173_73, (n + 100 * heads) as u64);
	let tau = std::f32::consts::TAU;
	for _ in 0..n {
		let ang = g.next() * tau;
		let rad = r * g.next().sqrt();
		let root = [rad * ang.cos(), rad * ang.sin()];
		let face = g.next() * tau;
		let side = [face.cos(), face.sin()];
		let lean_a = ang + (g.next() - 0.5) * 1.6;
		let lean = 0.1 + 0.3 * g.next();
		let h = 0.55 + 0.45 * g.next();
		let v = 0.82 + 0.3 * g.next();
		let warm = g.next() * 0.14;
		let col = [v * (1.0 + warm), v, v * (1.0 - warm * 0.6), 0.0];
		let base = verts.len() as u32;
		for (k, &f) in [0.0f32, 0.35, 0.7, 1.0].iter().enumerate() {
			let hw = w * 0.5 * (1.0 - f).powf(0.7);
			let off = lean * h * f * f;
			let c = [lean_a.cos() * off, lean_a.sin() * off, h * f];
			if k == 3 {
				verts.push(VegVertex { pos: c, root, uv: [0.0, f], col, side });
			} else {
				for s in [-1.0f32, 1.0] {
					verts.push(VegVertex { pos: c, root, uv: [s * hw, f], col, side });
				}
			}
		}
		idx.extend([0, 1, 2, 1, 3, 2, 2, 3, 4, 3, 5, 4, 4, 5, 6].map(|i| base + i));
	}
	// Flower heads: a thin stem a little over the blades, a flat six-petal disc on top (colour flag in col.w: 1 petal rim,
	// 2 centre; the colour itself is the instance's patch colour, vs_veg_blade).
	for _ in 0..heads {
		let ang = g.next() * tau;
		let rad = r * 0.8 * g.next().sqrt();
		let root = [rad * ang.cos(), rad * ang.sin()];
		let face = g.next() * tau;
		let side = [face.cos(), face.sin()];
		let lean_a = ang + (g.next() - 0.5) * 1.0;
		let lean = 0.05 + 0.15 * g.next();
		let h = 1.0 + 0.2 * g.next();
		let col = [0.9, 1.0, 0.85, 0.0];
		let base = verts.len() as u32;
		let top = |f: f32| [lean_a.cos() * lean * h * f * f, lean_a.sin() * lean * h * f * f, h * f];
		for (k, &f) in [0.0f32, 0.5, 1.0].iter().enumerate() {
			if k == 2 {
				verts.push(VegVertex { pos: top(f), root, uv: [0.0, f], col, side });
			} else {
				for sgn in [-1.0f32, 1.0] {
					verts.push(VegVertex { pos: top(f), root, uv: [sgn * w * 0.25, f], col, side });
				}
			}
		}
		idx.extend([0, 1, 2, 1, 3, 2, 2, 3, 4].map(|i| base + i));
		let c = verts.len() as u32;
		let tilt = g.next() * tau;
		verts.push(VegVertex { pos: top(1.0), root, uv: [0.0, 1.0], col: [1.0, 1.0, 1.0, 2.0], side: [0.0; 2] });
		let size = HEAD_R * (0.7 + 0.5 * g.next());
		// Five petals: the rim alternates petal tips and notches (a star, not a coin), a little raised (a shallow cup).
		let cup = [0.0, 0.0, 0.08];
		for k in 0..10 {
			let a = tilt + k as f32 * tau / 10.0;
			let len = if k % 2 == 0 { size } else { size * 0.55 };
			let p = top(1.0);
			verts.push(VegVertex { pos: [p[0] + cup[0], p[1] + cup[1], p[2] + cup[2] * (k % 2 == 0) as u32 as f32], root, uv: [len, 1.0], col: [1.0, 1.0, 1.0, 1.0], side: [a.cos(), a.sin()] });
		}
		for k in 0..10u32 {
			idx.extend([c, c + 1 + k, c + 1 + (k + 1) % 10]);
		}
	}
}

/// One seaweed clump of `n` ribbons (unit height; `w` base width m, `r` clump radius m): long, a little wider at the middle,
/// eight segments so the current can wave it (vs_veg_blade, sea flag).
pub(super) fn kelp(n: usize, w: f32, r: f32, verts: &mut Vec<VegVertex>, idx: &mut Vec<u32>) {
	let mut g = Rng::new(0x6b65_6c70, n as u64);
	let tau = std::f32::consts::TAU;
	const SEG: usize = 8;
	for _ in 0..n {
		let ang = g.next() * tau;
		let rad = r * g.next().sqrt();
		let root = [rad * ang.cos(), rad * ang.sin()];
		let face = g.next() * tau;
		let side = [face.cos(), face.sin()];
		let lean_a = ang + (g.next() - 0.5) * 1.2;
		let lean = 0.05 + 0.2 * g.next();
		let h = 0.6 + 0.4 * g.next();
		let v = 0.8 + 0.35 * g.next();
		let col = [v, v, v * (0.8 + 0.3 * g.next()), 0.0];
		let base = verts.len() as u32;
		for k in 0..=SEG {
			let f = k as f32 / SEG as f32;
			let hw = w * 0.5 * (0.55 + 0.45 * (std::f32::consts::PI * f).sin()) * (1.0 - f * f * f);
			let off = lean * h * f;
			let c = [lean_a.cos() * off, lean_a.sin() * off, h * f];
			if k == SEG {
				verts.push(VegVertex { pos: c, root, uv: [0.0, f], col, side });
			} else {
				for sg in [-1.0f32, 1.0] {
					verts.push(VegVertex { pos: c, root, uv: [sg * hw, f], col, side });
				}
			}
		}
		for k in 0..SEG as u32 - 1 {
			idx.extend([0, 1, 2, 1, 3, 2].map(|i| base + 2 * k + i));
		}
		let t = 2 * (SEG as u32 - 1);
		idx.extend([t, t + 1, t + 2].map(|i| base + i));
	}
}

/// One leaf cluster of `n` leaves (model z = the crown's outward normal, m): each leaf starts a little inside the crown within
/// `LEAF_SPREAD` of the centre and points outward, tilted 25 to 80 degrees off the normal; a pointed oval folded along its vein.
/// uv: (signed width, length fraction); col: the leaf's shade, a = length fraction (flutter weight).
pub(super) fn leaf_cluster(n: usize, verts: &mut Vec<VegVertex>, idx: &mut Vec<u32>) {
	let mut g = Rng::new(0x6c65_6166, n as u64);
	let tau = std::f32::consts::TAU;
	for _ in 0..n {
		let (ang, rad) = (g.next() * tau, LEAF_SPREAD * g.next().sqrt());
		let base = [rad * ang.cos(), rad * ang.sin(), -0.08 + 0.1 * g.next()];
		let (tilt, az) = ((25.0 + 55.0 * g.next()).to_radians(), g.next() * tau);
		let dir = [tilt.sin() * az.cos(), tilt.sin() * az.sin(), tilt.cos()];
		let h = if dir[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
		let s0 = norm3(cross3(h, dir));
		let s1 = cross3(dir, s0);
		let roll = g.next() * tau;
		let side: [f32; 3] = std::array::from_fn(|k| s0[k] * roll.cos() + s1[k] * roll.sin());
		let fold = cross3(dir, side);
		let len = LEAF_LEN.0 + (LEAF_LEN.1 - LEAF_LEN.0) * g.next();
		let wid = len * (0.38 + 0.14 * g.next());
		let v = 0.75 + 0.4 * g.next();
		let warm = 0.95 + 0.12 * g.next();
		let first = verts.len() as u32;
		// Outline: stem, two points down each side, tip; the sides drop a little behind the vein (the fold).
		for (fa, wa) in [(0.0f32, 0.0f32), (0.3, -0.5), (0.68, -0.4), (1.0, 0.0), (0.68, 0.4), (0.3, 0.5)] {
			let drop = -0.12 * wid * (wa.abs() * 2.0);
			let pos: [f32; 3] = std::array::from_fn(|k| base[k] + dir[k] * len * fa + side[k] * wid * wa + fold[k] * drop);
			let sh = v * (0.78 + 0.3 * fa);
			verts.push(VegVertex { pos, root: [0.0; 2], uv: [wa, fa], col: [sh * warm, sh, sh * 0.92, fa], side: [0.0; 2] });
		}
		idx.extend([0, 1, 5, 1, 4, 5, 1, 2, 4, 2, 3, 4].map(|i| first + i));
	}
}
