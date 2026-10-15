//! A triangle mesh in game units (1.0 = 1024 fixed-point), Z up as on the PS2.

use std::fmt::Write as _;

/// `Mesh.strip_phase` value of a face the CPU clipper must leave to the GPU.
pub const NO_PHASE: u8 = 255;

#[derive(Clone)]
pub struct Mesh {
	pub name: String,
	pub positions: Vec<[f32; 3]>,
	pub uvs: Vec<[f32; 2]>,
	/// (texture index, vertex indices).
	pub faces: Vec<(u32, [u32; 3])>,
	/// Baked per-vertex RGBA in PS2 bytes (0x80 = 1.0), parallel to `positions`; empty when the
	/// mesh is lit at runtime (decomp/notes/vertex_colours.md).
	pub colours: Vec<[f32; 4]>,
	/// Runtime-relight data (the world meshes the dynamic lights touch); None for everything else.
	pub lit: Option<Lit>,
	/// NOV-5.9c: per face of a moby mesh, the strip position (`i % 2` of the retail strip loop) it came from, parallel to
	/// `faces`; empty means no phase information (that mesh is never CPU-clipped).
	pub strip_phase: Vec<u8>,
	/// A moby mesh's decoded vertex normals (azimuth / elevation bytes, mobies.rs `normal`), parallel to `positions`; empty
	/// for meshes without stored normals. Retail lights every moby vertex with this normal (VU0 prog0, lighting_fog.md
	/// REN-5c), never with one taken from the face winding.
	pub normals: Vec<[f32; 3]>,
}

/// Which retail object list a lit mesh belongs to (the order the light assignment scans them in).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LitKind {
	Tie,
	Tfrag,
	Shrub,
	/// A scenery moby: lit at its origin (`sphere` centre) by the shader; not part of the assignment.
	Moby,
}

/// What the dynamic-light pass needs of a world mesh (decomp/notes/lighting_fog.md 5b): the bounding sphere the
/// assignment tests, the static colour before the dynamic third light and the per-vertex world normals.
#[derive(Clone, Debug)]
pub struct Lit {
	pub kind: LitKind,
	/// World-space bounding sphere (centre, radius).
	pub sphere: [f32; 4],
	/// Instance origin (matrix translation): the shrub fog depth input; equals the sphere centre elsewhere.
	pub origin: [f32; 3],
	/// Colour cap in PS2 bytes (243 ties and shrubs, 255 tfrags).
	pub cap: f32,
	pub normals: Vec<[f32; 3]>,
	/// Static colour per vertex: uncapped for ties and shrubs (the cap applies to the sum with the dynamic term),
	/// the capped static colour for tfrags.
	pub raw: Vec<[f32; 4]>,
}

impl Mesh {
	/// NOV-5.9d: call right after `faces` gained `src`'s faces. When `strip_phase` was parallel to `faces` before
	/// (`was_aligned`) it stays parallel: `src`'s phases are appended, or 255 (never clip) for a source without them.
	pub fn extend_phases(&mut self, src: &Mesh, was_aligned: bool) {
		if was_aligned {
			if src.strip_phase.len() == src.faces.len() {
				self.strip_phase.extend(src.strip_phase.iter().copied());
			} else {
				self.strip_phase.extend(std::iter::repeat(NO_PHASE).take(src.faces.len()));
			}
		}
	}

	pub fn new(name: String) -> Mesh {
		Mesh { name, positions: Vec::new(), uvs: Vec::new(), faces: Vec::new(), colours: Vec::new(), lit: None, strip_phase: Vec::new(), normals: Vec::new() }
	}

	/// Per-vertex normals: the stored ones when parallel to `positions` (a decoded moby), else the area-weighted sum of
	/// the face normals of each vertex (face winding).
	pub fn vertex_normals(&self) -> Vec<[f32; 3]> {
		if !self.positions.is_empty() && self.normals.len() == self.positions.len() {
			return self.normals.clone();
		}
		let mut normals = vec![[0f32; 3]; self.positions.len()];
		for &(_, [a, b, c]) in &self.faces {
			let (pa, pb, pc) = (self.positions[a as usize], self.positions[b as usize], self.positions[c as usize]);
			let (e1, e2) = ([pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]], [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]]);
			let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
			for i in [a, b, c] {
				for j in 0..3 {
					normals[i as usize][j] += n[j];
				}
			}
		}
		normals
	}
}

/// Wavefront OBJ of several meshes, one object each, one material per texture index.
pub fn to_obj(meshes: &[Mesh]) -> String {
	let mut s = String::new();
	let mut base = 1;
	for m in meshes {
		let _ = writeln!(s, "o {}", m.name);
		for p in &m.positions {
			// OBJ viewers expect Y up: (x, y, z) -> (x, z, -y).
			let _ = writeln!(s, "v {} {} {}", p[0], p[2], -p[1]);
		}
		for t in &m.uvs {
			let _ = writeln!(s, "vt {} {}", t[0], 1.0 - t[1]);
		}
		let mut current = None;
		for &(tex, f) in &m.faces {
			let tex = tex & crate::mobies::KEY_MASK;
			if current != Some(tex) {
				let _ = writeln!(s, "usemtl terrain_{tex}");
				current = Some(tex);
			}
			let [a, b, c] = f.map(|i| i as usize + base);
			let _ = writeln!(s, "f {a}/{a} {b}/{b} {c}/{c}");
		}
		base += m.positions.len();
	}
	s
}
