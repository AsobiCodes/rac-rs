//! The cutscene / stream ship "gloss" overlay (retail callback FUN_002a69d8, decomp/notes/novalis_checklist.md
//! "NOV-14.5b port spec" and "NOV-14.5c"): an environment-mapped quad overlay on the four ship classes 0x212..0x215.
//! Data comes from the level overlay image (Novalis); one `ShipGloss` is the retail global state shared by all ships.

use crate::particles::WorldQuad;

/// The overlay image starts 0x38 bytes into the ELF load address (same rule as `water.rs`).
const IMAGE_BASE: usize = 0x15ef38;
const MAX_VERTS: usize = 102;
/// Per-class tables (pointer arrays in the overlay): colour words, vertex counts, quad counts, reflector (B),
/// position (A) and quad index (D) table pointers.
const COLOURS: usize = 0x160560;
const COUNTS: usize = 0x160570;
const QUADS: usize = 0x160580;
const TABLE_B: usize = 0x160590;
const TABLE_A: usize = 0x1605a0;
const TABLE_D: usize = 0x1605b0;
/// fx texture index of the gloss (1 in the final sequence, mode 6 substate 4).
const TEX: usize = 0x15;
const TEX_FINAL: usize = 1;

fn at(img: &[u8], va: usize, len: usize) -> Option<&[u8]> {
	if crate::retail::disc::usa() {
		return crate::retail::disc::ov_bytes(img, va, len);
	}
	img.get(va.checked_sub(IMAGE_BASE)?..)?.get(..len)
}

fn u32_at(img: &[u8], va: usize) -> Option<u32> {
	// `va` = a PAL table address (Novalis, level 1); `at` also reads the USA pointers stored in them.
	Some(u32::from_le_bytes(at(img, crate::retail::disc::va_level(1, va as u32) as usize, 4)?.try_into().ok()?))
}

struct Ship {
	/// Ship-local positions and reflectors (`x, y, z`; w is 1).
	a: Vec<[f32; 3]>,
	b: Vec<[f32; 3]>,
	/// Vertex index of each of the four corners of each quad.
	quads: Vec<[usize; 4]>,
	colour: [f32; 4],
}

pub struct ShipGloss {
	ships: Vec<Ship>,
	first: u8,
	armed: u8,
	ctr: i32,
	cur: [[f32; 2]; MAX_VERTS],
	prev: [[f32; 2]; MAX_VERTS],
	vtx: [[f32; 3]; MAX_VERTS],
}

/// FPU divide (the retail `div`): a zero divisor gives +-FLT_MAX (sign of numerator xor divisor), not inf/NaN.
fn fdiv(a: f32, b: f32) -> f32 {
	if b == 0.0 {
		if a.is_sign_negative() != b.is_sign_negative() {
			-f32::MAX
		} else {
			f32::MAX
		}
	} else {
		a / b
	}
}

/// 3x4 row matrix times `(v, 1)`.
fn apply(j: &[f32; 12], v: [f32; 3]) -> [f32; 3] {
	[
		j[0] * v[0] + j[1] * v[1] + j[2] * v[2] + j[3],
		j[4] * v[0] + j[5] * v[1] + j[6] * v[2] + j[7],
		j[8] * v[0] + j[9] * v[1] + j[10] * v[2] + j[11],
	]
}

/// Sphere-map UV of one vertex: `v` its world point, `b` the world reflector point (`apply(B) + pos`), `eye` the
/// camera. `nn = 0.1 * b`, `R = e - 2 (nn . e) nn`, `R.z += 1`, `m = 2 sqrt(|2 R.z|)`, `uv = R.xy / m + 0.5`.
fn env_uv(v: [f32; 3], b: [f32; 3], eye: [f32; 3]) -> [f32; 2] {
	let e = [v[0] - eye[0], v[1] - eye[1], v[2] - eye[2]];
	let nn = [0.1 * b[0], 0.1 * b[1], 0.1 * b[2]];
	let d = nn[0] * e[0] + nn[1] * e[1] + nn[2] * e[2];
	let k = 2.0 * d;
	let r = [e[0] - k * nn[0], e[1] - k * nn[1], e[2] - k * nn[2] + 1.0];
	let m = 2.0 * (r[2] + r[2]).abs().sqrt();
	[fdiv(r[0], m) + 0.5, fdiv(r[1], m) + 0.5]
}

/// `u + (prev - u) * w`, the multiply-add order of the retail blend.
fn blend(new: [f32; 2], prev: [f32; 2], w: f32) -> [f32; 2] {
	[new[0] + (prev[0] - new[0]) * w, new[1] + (prev[1] - new[1]) * w]
}

impl ShipGloss {
	/// Read the four ships' tables from the level overlay image (None when the image does not hold them).
	pub fn new(overlay: &[u8]) -> Option<ShipGloss> {
		let mut ships = Vec::new();
		for c in 0..4 {
			let n = u32_at(overlay, COUNTS + 4 * c)? as usize;
			let nq = u32_at(overlay, QUADS + 4 * c)? as usize;
			if n > MAX_VERTS {
				return None;
			}
			let vecs = |table: usize| -> Option<Vec<[f32; 3]>> {
				let base = u32_at(overlay, table + 4 * c)? as usize;
				let raw = at(overlay, base, 16 * n)?;
				Some((0..n).map(|i| std::array::from_fn(|k| f32::from_le_bytes(raw[16 * i + 4 * k..16 * i + 4 * k + 4].try_into().unwrap()))).collect())
			};
			let d = u32_at(overlay, TABLE_D + 4 * c)? as usize;
			let raw = at(overlay, d, 16 * nq)?;
			let quads = (0..nq).map(|q| std::array::from_fn(|k| i16::from_le_bytes(raw[16 * q + 4 * k..16 * q + 4 * k + 2].try_into().unwrap()) as usize)).collect();
			let w = u32_at(overlay, COLOURS + 4 * c)?;
			let colour = std::array::from_fn(|k| ((w >> (8 * k)) & 0xff) as f32 / 128.0);
			ships.push(Ship { a: vecs(TABLE_A)?, b: vecs(TABLE_B)?, quads, colour });
		}
		Some(ShipGloss { ships, first: 1, armed: 1, ctr: 0, cur: [[0.0; 2]; MAX_VERTS], prev: [[0.0; 2]; MAX_VERTS], vtx: [[0.0; 3]; MAX_VERTS] })
	}

	/// One run of the retail callback (one game tick). `class` 0x212..=0x215, `j` the ship's combined 3x4 matrix,
	/// `pos` its position, `eye` the camera position, `mode` the game mode (6 in the ship streams, 2 in scenes),
	/// `final_seq` the mode-6 substate-4 flag.
	pub fn step(&mut self, class: u32, j: &[f32; 12], pos: [f32; 3], eye: [f32; 3], mode: u32, final_seq: bool) -> Vec<WorldQuad> {
		let Some(idx) = class.checked_sub(0x212).filter(|&i| i < 4) else { return Vec::new() };
		let tex = if mode == 6 && final_seq { TEX_FINAL } else { TEX };
		let ship = &self.ships[idx as usize];
		let n = ship.a.len();
		let world = |v: [f32; 3]| {
			let p = apply(j, v);
			[p[0] + pos[0], p[1] + pos[1], p[2] + pos[2]]
		};
		let mut near = mode != 0 || ((eye[0] - pos[0]).abs() < 16.0 && (eye[1] - pos[1]).abs() < 16.0);
		if mode == 6 && final_seq {
			near = false;
		}
		if near || self.first == 1 {
			self.armed = 1;
			if self.ctr != 0 {
				self.ctr -= 1;
			}
			let w = self.ctr as f32 / 50.0;
			for i in 0..n {
				let v = world(ship.a[i]);
				self.vtx[i] = v;
				let uv = env_uv(v, world(ship.b[i]), eye);
				self.cur[i] = if self.first == 1 || self.ctr == 0 { uv } else { blend(uv, self.prev[i], w) };
			}
			if self.first == 1 {
				self.first = 2;
			}
		} else {
			if self.armed == 1 {
				self.armed = 0;
				self.prev[..n].copy_from_slice(&self.cur[..n]);
			}
			for i in 0..n {
				self.vtx[i] = world(ship.a[i]);
			}
			self.ctr = 50;
		}
		ship
			.quads
			.iter()
			.map(|q| {
				let (p, t): (Vec<[f32; 3]>, Vec<[f32; 2]>) = q.iter().map(|&i| (self.vtx[i], self.cur[i])).unzip();
				WorldQuad { pos: [p[1], p[0], p[2], p[3]], uv: [t[1], t[0], t[2], t[3]], colour: [ship.colour; 4], tex, additive: false }
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A synthetic overlay image holding the four ships' tables at the retail addresses.
	fn fake_overlay() -> Vec<u8> {
		let mut img = vec![0u8; 0x1c3000 - IMAGE_BASE];
		let put = |img: &mut Vec<u8>, va: usize, b: &[u8]| img[va - IMAGE_BASE..va - IMAGE_BASE + b.len()].copy_from_slice(b);
		let n = [102u32, 39, 49, 42];
		let nq = [74u32, 24, 48, 30];
		let colours = [0x50807060u32, 0x30807060, 0x30807060, 0x302080ff];
		let a = [0x1be1e0u32, 0x1bf340, 0x1bf9a0, 0x1c02c0];
		let b = [0x1bece0u32, 0x1bf730, 0x1bffb0, 0x1c0740];
		let d = [0x1be840u32, 0x1bf5b0, 0x1bfcb0, 0x1c0560];
		for c in 0..4 {
			put(&mut img, COUNTS + 4 * c, &n[c].to_le_bytes());
			put(&mut img, QUADS + 4 * c, &nq[c].to_le_bytes());
			put(&mut img, COLOURS + 4 * c, &colours[c].to_le_bytes());
			put(&mut img, TABLE_A + 4 * c, &a[c].to_le_bytes());
			put(&mut img, TABLE_B + 4 * c, &b[c].to_le_bytes());
			put(&mut img, TABLE_D + 4 * c, &d[c].to_le_bytes());
			// Vertex i of A at (i, 0, 0, 1), of B at (0, 0, 10, 1); quad q = vertices (q, q+1, q+2, q+3).
			for i in 0..n[c] as usize {
				let va = a[c] as usize + 16 * i;
				put(&mut img, va, &(i as f32).to_le_bytes());
				put(&mut img, va + 12, &1.0f32.to_le_bytes());
				let vb = b[c] as usize + 16 * i;
				put(&mut img, vb + 8, &10.0f32.to_le_bytes());
				put(&mut img, vb + 12, &1.0f32.to_le_bytes());
			}
			for q in 0..nq[c] as usize {
				for (k, slot) in [0u16, 1, 3, 2].into_iter().enumerate() {
					let v = d[c] as usize + 16 * q + 4 * k;
					put(&mut img, v, &((q + k) as u16).to_le_bytes());
					put(&mut img, v + 2, &slot.to_le_bytes());
				}
			}
		}
		img
	}

	const ID: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

	/// Tests 1 and 2: sphere-map UVs.
	#[test]
	fn env_uv_values() {
		let uv = env_uv([2.0, 0.0, 0.0], [0.0, 0.0, 10.0], [0.0; 3]);
		assert!((uv[0] - 1.2071068).abs() < 1e-6 && (uv[1] - 0.5).abs() < 1e-6, "{uv:?}");
		let uv = env_uv([3.0, 4.0, -2.0], [1.0, 2.0, 3.0], [0.0; 3]);
		assert!((uv[0] - 1.3992519).abs() < 1e-5 && (uv[1] - 1.67833).abs() < 1e-5, "{uv:?}");
	}

	/// Test 3.
	#[test]
	fn blend_half() {
		let b = blend([0.6, 0.5], [0.2, 0.1], 25.0 / 50.0);
		assert!((b[0] - 0.4).abs() < 1e-6 && (b[1] - 0.3).abs() < 1e-6);
	}

	/// The retail divide by zero gives FLT_MAX with the xor sign.
	#[test]
	fn divide_by_zero_is_flt_max() {
		assert_eq!(fdiv(1.0, 0.0), f32::MAX);
		assert_eq!(fdiv(-1.0, 0.0), -f32::MAX);
		assert_eq!(fdiv(1.0, -0.0), -f32::MAX);
	}

	/// Tests 4, 5, 6: quad counts per class, point/UV order, colour, texture.
	#[test]
	fn quads_counts_order_and_colour() {
		let mut g = ShipGloss::new(&fake_overlay()).expect("tables");
		for (class, nq) in [(0x212u32, 74usize), (0x213, 24), (0x214, 48), (0x215, 30)] {
			assert_eq!(g.step(class, &ID, [0.0; 3], [0.0; 3], 2, false).len(), nq, "class {class:#x}");
		}
		let mut g = ShipGloss::new(&fake_overlay()).unwrap();
		let q = g.step(0x212, &ID, [0.0; 3], [0.0; 3], 2, false);
		// Quad 0 vertices (0, 1, 2, 3): pos = [P1, P0, P2, P3].
		assert_eq!(q[0].pos.map(|p| p[0]), [1.0, 0.0, 2.0, 3.0]);
		assert_eq!((q[0].tex, q[0].additive, q[0].colour[0]), (0x15, false, [0.75, 0.875, 1.0, 0.625]));
		assert_eq!(g.step(0x213, &ID, [0.0; 3], [0.0; 3], 2, false)[0].colour[0], [0.75, 0.875, 1.0, 0.375]);
		let q = g.step(0x215, &ID, [0.0; 3], [0.0; 3], 6, true);
		assert_eq!((q[0].tex, q[0].colour[0]), (1, [1.9921875, 1.0, 0.25, 0.375]));
		assert!(g.step(0x211, &ID, [0.0; 3], [0.0; 3], 2, false).is_empty());
	}

	/// Test 8: the scene call transforms by `j` and adds `pos`.
	#[test]
	fn scene_call_world_points() {
		let mut j = ID;
		j[3] = 0.5;
		let mut g = ShipGloss::new(&fake_overlay()).unwrap();
		let q = g.step(0x212, &j, [1.0, 2.0, 3.0], [0.0; 3], 2, false);
		// Quad 0: P0 = A[0] = (0,0,0) -> (1.5, 2, 3); P1 = A[1] = (1,0,0) -> (2.5, 2, 3).
		assert_eq!(q[0].pos[1], [1.5, 2.0, 3.0]);
		assert_eq!(q[0].pos[0], [2.5, 2.0, 3.0]);
	}

	/// Test 7: mode 0, ship 20 m away: the first call still takes the near branch; the next is not near.
	#[test]
	fn far_ship_state_machine() {
		let mut g = ShipGloss::new(&fake_overlay()).unwrap();
		g.step(0x212, &ID, [20.0, 0.0, 0.0], [0.0; 3], 0, false);
		assert_eq!((g.first, g.armed), (2, 1));
		g.step(0x212, &ID, [20.0, 0.0, 0.0], [0.0; 3], 0, false);
		assert_eq!((g.armed, g.ctr), (0, 50));
		assert_eq!(g.prev[0], g.cur[0]);
		// Back in range: the counter runs down from 50 and blends with the stale UVs.
		g.step(0x212, &ID, [0.0; 3], [0.0; 3], 0, false);
		assert_eq!((g.armed, g.ctr), (1, 49));
	}
}
