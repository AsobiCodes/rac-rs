//! Small vector / matrix / mesh helpers: cross, normalise, invert3, moby placement matrices, 3x4 transforms,
//! mesh bounds and normals, the player stand-in box; with their tests.

use super::*;

pub(super) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub(super) fn normalise(v: [f32; 3]) -> [f32; 3] {
	let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
	[v[0] / l, v[1] / l, v[2] / l]
}

/// Inverse of a 3x3 matrix; a cuboid's half-axes are the rows of its stored inverse's inverse.
pub(super) fn invert3(m: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
	let c = [cross(m[1], m[2]), cross(m[2], m[0]), cross(m[0], m[1])];
	let det = m[0][0] * c[0][0] + m[0][1] * c[0][1] + m[0][2] * c[0][2];
	(det.abs() > 1e-12).then(|| std::array::from_fn(|i| std::array::from_fn(|j| c[j][i] / det)))
}

pub(super) fn moby_matrix(m: &gameplay::MobyInstance) -> [f32; 16] {
	scale_rows(place(m.pos, m.rot), m.scale)
}

/// Retail scales the three rotation columns by the moby scale (class +0x24 * placement +0x1c); the translation stays.
pub(super) fn scale_rows(mut at: [f32; 16], s: f32) -> [f32; 16] {
	for v in &mut at[..12] {
		*v *= s;
	}
	at
}

/// `place` of a retail prop: a mirrored one (moby +0x34 bit 0x8000, `Prop::mirror`) has retail's basis row 1 negated, a reflection in
/// model Y: column 1 of the placement (the image of model Y) is negated, `rot` stays the plain rotation.
pub(super) fn place_mirrored(pos: [f32; 3], rot: [f32; 3], mirror: bool) -> [f32; 16] {
	let mut m = place(pos, rot);
	if mirror {
		for v in &mut m[4..8] {
			*v = -*v;
		}
	}
	m
}

pub(super) fn place(pos: [f32; 3], rot: [f32; 3]) -> [f32; 16] {
	let (sx, cx) = rot[0].sin_cos();
	let (sy, cy) = rot[1].sin_cos();
	let (sz, cz) = rot[2].sin_cos();
	[
		cz * cy, sz * cy, -sy, 0.0,
		cz * sy * sx - sz * cx, sz * sy * sx + cz * cx, cy * sx, 0.0,
		cz * sy * cx + sz * sx, sz * sy * cx - cz * sx, cy * cx, 0.0,
		pos[0], pos[1], pos[2], 1.0,
	]
}

/// A 0.7 x 0.5 x 1.4 box standing on the origin: the player stand-in when Ratchet is unavailable.
pub(super) fn box_mesh(texture: u32) -> Mesh {
	let mut m = Mesh::new("player box".into());
	for i in 0..8 {
		m.positions.push([if i & 1 != 0 { 0.35 } else { -0.35 }, if i & 2 != 0 { 0.25 } else { -0.25 }, if i & 4 != 0 { 1.4 } else { 0.0 }]);
		m.uvs.push([0.0, 0.0]);
	}
	for q in [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]] {
		m.faces.push((texture, [q[0], q[1], q[2]]));
		m.faces.push((texture, [q[0], q[2], q[3]]));
	}
	m
}

/// 3x4 row-major transform of a point (`t` = 1) or a direction (`t` = 0).
/// Area-weighted vertex normals (the same sum `Gpu::batch` uses).
/// A decoded moby mesh gives its stored vertex normals instead (`Mesh::vertex_normals`).
pub(super) fn face_normals(m: &Mesh) -> Vec<[f32; 3]> {
	m.vertex_normals()
}

/// Normalises each rotation column `(m[i], m[4+i], m[8+i])` to unit length (HeroItemsAttach basis fix, HERO-7.1).
pub(super) fn norm_cols(mut m: [f32; 12]) -> [f32; 12] {
	for i in 0..3 {
		let l = (m[i] * m[i] + m[4 + i] * m[4 + i] + m[8 + i] * m[8 + i]).sqrt();
		if l != 0.0 && l.is_finite() {
			m[i] /= l;
			m[4 + i] /= l;
			m[8 + i] /= l;
		}
	}
	m
}


/// The attach joint matrix with its three axis columns scaled to unit length (view rows with mode != 0).
pub(super) fn normalise_columns(mut m: [f32; 12]) -> [f32; 12] {
	for c in 0..3 {
		let l = (m[c] * m[c] + m[4 + c] * m[4 + c] + m[8 + c] * m[8 + c]).sqrt();
		if l > 1e-6 {
			for r in 0..3 {
				m[4 * r + c] /= l;
			}
		}
	}
	m
}

pub(super) fn apply(m: &[f32; 12], v: [f32; 3], t: f32) -> [f32; 3] {
	[
		m[0] * v[0] + m[1] * v[1] + m[2] * v[2] + m[3] * t,
		m[4] * v[0] + m[5] * v[1] + m[6] * v[2] + m[7] * t,
		m[8] * v[0] + m[9] * v[1] + m[10] * v[2] + m[11] * t,
	]
}

pub(super) fn length2(v: [f32; 3]) -> f32 {
	(v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// Model-space box of a mesh.
pub(super) fn bounds(m: &Mesh) -> ([f32; 3], [f32; 3]) {
	let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
	for p in &m.positions {
		for j in 0..3 {
			lo[j] = lo[j].min(p[j]);
			hi[j] = hi[j].max(p[j]);
		}
	}
	if m.positions.is_empty() {
		return ([-0.5; 3], [0.5; 3]);
	}
	(lo, hi)
}

#[cfg(test)]
mod attach_tests {
	use super::*;
	#[test]
	fn norm_cols_normalises_columns_not_rows() {
		let m = norm_cols([2.0, 3.0, 0.0, 0.0, 0.0, 4.0, 0.0, 5.0, 0.0, 0.0, 1.0, 6.0]);
		let e = [1.0, 0.6, 0.0, 0.0, 0.0, 0.8, 0.0, 5.0, 0.0, 0.0, 1.0, 6.0];
		for i in 0..12 {
			assert!((m[i] - e[i]).abs() < 1e-6, "{i}: {m:?}");
		}
	}
}

#[cfg(test)]
mod moby_matrix_tests {
	use super::*;

	#[test]
	fn moby_matrix_scales_rows() {
		let at = scale_rows(place([1.0, 2.0, 3.0], [0.0, 0.0, 0.0]), 2.0);
		assert_eq!((at[0], at[5], at[10]), (2.0, 2.0, 2.0));
		assert_eq!(&at[12..], &[1.0, 2.0, 3.0, 1.0]);
		assert_eq!(at.iter().filter(|v| **v == 0.0).count(), 9);
		let base = place([1.0, 2.0, 3.0], [0.3, 0.2, 0.1]);
		assert_eq!(scale_rows(base, 1.0), base);
	}

	#[test]
	fn mirrored_placement_reflects_model_y_only() {
		let (pos, rot) = ([1.0, 2.0, 3.0], [0.3, 0.2, 0.1]);
		let (a, m) = (place(pos, rot), place_mirrored(pos, rot, true));
		assert_eq!(place_mirrored(pos, rot, false), a);
		assert_eq!((&m[0..4], &m[8..16]), (&a[0..4], &a[8..16]));
		assert_eq!(m[4..8], a[4..8].iter().map(|v| -v).collect::<Vec<_>>()[..]);
		// Left-handed: column 0 x column 1 . column 2 < 0 (a plain placement is right-handed).
		let det = |m: &[f32; 16]| m[0] * (m[5] * m[10] - m[6] * m[9]) - m[1] * (m[4] * m[10] - m[6] * m[8]) + m[2] * (m[4] * m[9] - m[5] * m[8]);
		assert!(det(&a) > 0.99 && det(&m) < -0.99);
	}
}
