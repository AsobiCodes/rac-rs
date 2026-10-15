//! Class 760 fire quads (decomp/notes/particles.md "Fire quads (760/809)"): per emitter N camera-facing
//! flame billboards (fx texture 41, additive), flame columns of two quads each (texture 41, alpha), and
//! six volume quads when the emitter makes no puffs (texture 42, alpha, V scrolled by class 809).

use crate::particles::{Cuboid, Rng, WorldQuad};

type V3 = [f32; 3];

/// Fade in, per-slot hold step and fade out: fead0(60) = 50 draws each.
const FADE: u32 = 50;
/// Vertex alpha scale at full fade (iGp ad58 = 127).
const ALPHA_SCALE: u32 = 127;
/// UV offsets wrap by +8 at <= -8.
const WRAP: f32 = 8.0;
/// Column scroll speed range (UV units per tick, before the 1.2 every fire accumulator applies).
const COLUMN_SCROLL: (f32, f32) = (0.003, 0.005);
/// Every fire scroll steps rate * 1.2 per tick (decomp/notes/effects_lighting_mapping.md §3).
const SCROLL_STEP: f32 = 1.2;
/// Column and volume colour 0x606060 (+0x5d..+0x5f); column alpha +0x5c = 0x28 and volume alpha 0xff, both on
/// the middle-row verts 8, 9 only.
const SHEET_RGB: f32 = 0x60 as f32 / 128.0;
const COLUMN_ALPHA: f32 = 0x28 as f32 / 128.0;
const VOLUME_ALPHA: f32 = 0xff as f32 / 128.0;

/// Main quad template DAT_001e1120 in strip order, (y, z), and its UVs DAT_001e1160.
const MAIN: [[f32; 2]; 4] = [[-0.5, 1.0], [-0.5, 0.0], [0.5, 1.0], [0.5, 0.0]];
const MAIN_UV: [[f32; 2]; 4] = [[0.0, 0.1], [1.0, 0.25], [0.0, 0.9], [1.0, 0.75]];
/// Column vertices 1, 2, 8, 9, 5, 6 of DAT_001e1240 (y, z) and DAT_001e13c0 UVs; quads [1,2,8,9] [8,9,5,6].
const COLUMN: [[f32; 2]; 6] = [[-0.5, 0.0], [0.5, 0.0], [-1.25, 1.0], [1.25, 1.0], [-0.5, 2.0], [0.5, 2.0]];
const COLUMN_UV: [[f32; 2]; 6] = [[0.0, 0.0], [0.75, 0.0], [0.0, 0.5], [1.0, 0.5], [0.25, 1.0], [0.75, 1.0]];
const COLUMN_QUADS: [[usize; 4]; 2] = [[0, 1, 2, 3], [2, 3, 4, 5]];
/// The column's middle-row verts (table indices 8, 9).
const COLUMN_MID: [usize; 2] = [2, 3];
/// The volume's middle-row verts.
const VOLUME_MID: [usize; 2] = [8, 9];
/// Volume vertices DAT_001e1180 (y, z), UVs DAT_001e1360, quads from DAT_001e1300 (strip order).
const VOLUME: [[f32; 2]; 12] = [
	[-0.75, 0.0],
	[-0.375, 0.0],
	[0.375, 0.0],
	[0.75, 0.0],
	[-2.5, 2.0],
	[-1.5, 2.0],
	[1.5, 2.0],
	[2.5, 2.0],
	[-0.75, 1.0],
	[0.75, 1.0],
	[-1.625, 0.5],
	[1.625, 0.5],
];
const VOLUME_UV: [[f32; 2]; 12] = [
	[0.0, 0.0],
	[0.25, 0.0],
	[0.75, 0.0],
	[1.0, 0.0],
	[0.0, 1.0],
	[0.25, 1.0],
	[0.75, 1.0],
	[1.0, 1.0],
	[0.25, 0.5],
	[0.75, 0.5],
	[0.0, 0.5],
	[1.0, 0.5],
];
const VOLUME_QUADS: [[usize; 4]; 6] = [[0, 1, 10, 8], [10, 8, 4, 5], [1, 2, 8, 9], [8, 9, 5, 6], [9, 2, 11, 3], [6, 9, 7, 11]];

/// The class 760 pvar fields the quads use (96 bytes, little-endian).
#[derive(Clone, Copy, Debug)]
pub struct FirePvars {
	/// +0x02 regular quads, +0x40 flame columns.
	pub quads: usize,
	pub columns: usize,
	/// +0x08/+0x0c scroll, +0x10/+0x14 width, +0x18/+0x1c height ranges.
	pub scroll: (f32, f32),
	pub width: (f32, f32),
	pub height: (f32, f32),
	/// +0x3c rgb bytes (0x80 = 1.0).
	pub rgb: [f32; 3],
	/// +0x42: puffs instead of volume quads.
	pub puffs: bool,
	/// +0x4c: column height base and volume z scale.
	pub extra: f32,
	/// fx texture slots 0x28 + i32 +0x54 (main quads and columns) and 0x28 + i32 +0x58 (volume quads): Veldin 0x29 / 0x2a,
	/// Novalis 0x2e / 0x2f (NOV-31).
	pub tex: [usize; 2],
	/// The moby's own position (the facing basis is taken from it, set by the caller).
	pub origin: V3,
}

impl FirePvars {
	pub fn parse(b: &[u8]) -> Option<FirePvars> {
		if b.len() < 0x5c {
			return None;
		}
		let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
		let h = |o: usize| i16::from_le_bytes([b[o], b[o + 1]]).max(0) as usize;
		let slot = |o: usize| (0x28 + i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])).max(0) as usize;
		Some(FirePvars {
			quads: h(0x02),
			columns: h(0x40),
			scroll: (f(0x08), f(0x0c)),
			width: (f(0x10), f(0x14)),
			height: (f(0x18), f(0x1c)),
			rgb: [b[0x3c] as f32 / 128.0, b[0x3d] as f32 / 128.0, b[0x3e] as f32 / 128.0],
			puffs: h(0x42) != 0,
			extra: f(0x4c),
			tex: [slot(0x54), slot(0x58)],
			origin: [0.0; 3],
		})
	}
}

#[derive(Clone, Copy, Debug)]
struct Slot {
	pos: V3,
	w: f32,
	h: f32,
	offset: f32,
	speed: f32,
	/// Fade-in, hold and delay timers (draws left); columns have none.
	fade_in: u32,
	hold: u32,
	delay: u32,
	/// Vertex alpha byte of the last draw (0..127).
	alpha: u32,
	/// The per-slot flip byte: mirrors the basis to (-X, -Y, Z).
	flip: bool,
}

/// One emitter's quad slots (FUN_002d68d0 init, FUN_002d7598 / FUN_002d7700 per slot).
pub struct Fire {
	pv: FirePvars,
	vol: Cuboid,
	/// +0x48 horizontal extent, +0x44 column width base.
	extent: f32,
	column_w: f32,
	slots: Vec<Slot>,
	rng: Rng,
}

/// FUN_001feb48: a zero timer counts as expired and stays; otherwise it steps and expires on reaching zero.
fn expired(t: &mut u32) -> bool {
	if *t == 0 {
		return true;
	}
	*t -= 1;
	*t == 0
}

impl Fire {
	pub fn new(pv: FirePvars, vol: Cuboid) -> Fire {
		let l = |v: V3| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
		let extent = l(vol.axes[1]).max(l(vol.axes[0]));
		let column_w = if pv.columns != 0 { extent } else { 3.0 };
		let mut f = Fire { pv, vol, extent, column_w, slots: Vec::new(), rng: Rng::shared() };
		for i in 0..pv.quads + pv.columns {
			// Init order: the flip byte first, then the slot roll; the first cycle has no fade-in.
			let flip = f.rng.int(0, 1) != 0;
			let mut s = f.roll(i);
			s.flip = flip;
			if i < pv.quads {
				s.fade_in = 0;
				s.alpha = ALPHA_SCALE;
			}
			f.slots.push(s);
		}
		f
	}

	/// FUN_002d7598 (regular) / FUN_002d7700 (column): position, size and speed draws; the flip is the caller's.
	fn roll(&mut self, i: usize) -> Slot {
		let r = &mut self.rng;
		let mut pos = self.vol.centre;
		for a in self.vol.axes {
			let k = r.sym(1.0);
			pos = [pos[0] + a[0] * k, pos[1] + a[1] * k, pos[2] + a[2] * k];
		}
		let (w, h, speed) = if i < self.pv.quads {
			(r.float(self.pv.width.0, self.pv.width.1), r.float(self.pv.height.0, self.pv.height.1), r.float(self.pv.scroll.0, self.pv.scroll.1))
		} else {
			(r.float(self.column_w, self.column_w + 2.0), r.float(self.pv.extra, self.pv.extra + 4.0), r.float(COLUMN_SCROLL.0, COLUMN_SCROLL.1))
		};
		Slot { pos, w, h, offset: 0.0, speed, fade_in: FADE, hold: 0, delay: Self::hold(i), alpha: 0, flip: false }
	}

	/// Stagger delay of slot `i`: (slot - base + 1) * 50 draws.
	fn hold(i: usize) -> u32 {
		(i as u32 + 1) * FADE
	}

	/// One draw call (the caller ticks only while the emitter is visible): scroll every slot; regular slots step
	/// fade-in, delay and fade-out and are re-rolled (roll, then flip) when faded out; columns only scroll.
	pub fn tick(&mut self) {
		for i in 0..self.slots.len() {
			let s = &mut self.slots[i];
			s.offset -= s.speed * SCROLL_STEP;
			if s.offset <= -WRAP {
				s.offset += WRAP;
			}
			if i >= self.pv.quads {
				continue;
			}
			if s.fade_in != 0 {
				s.fade_in -= 1;
				s.alpha = ((1.0 - s.fade_in as f32 / FADE as f32) * ALPHA_SCALE as f32) as u32;
			} else if s.hold == 0 {
				let done = expired(&mut s.delay);
				s.alpha = ALPHA_SCALE;
				if done {
					s.hold = FADE;
				}
			} else if !expired(&mut s.hold) {
				s.alpha = (s.hold as f32 / FADE as f32 * ALPHA_SCALE as f32) as u32;
			} else {
				let mut n = self.roll(i);
				n.flip = self.rng.int(0, 1) != 0;
				self.slots[i] = n;
			}
		}
	}

	/// The update's visibility test: the volume centre + (0, 0, 9) inside the camera frustum (FUN_001ffe90).
	pub fn visible(&self, view: &crate::dynlight::View) -> bool {
		let c = self.vol.centre;
		view.sphere([c[0], c[1], c[2] + 9.0], 0.0) >= 0
	}

	/// Main-quad bottom-vertex alpha (0x80 = 1.0).
	fn fade(&self, i: usize) -> f32 {
		self.slots[i].alpha as f32 / 128.0
	}

	/// The emitter's quads for a camera at `eye`; `scroll` is class 809's shared V offset.
	pub fn quads(&self, eye: V3, scroll: f32, out: &mut Vec<WorldQuad>) {
		// FUN_002d6d90: one basis per draw from the moby position: X = setlen(cam - p) in 3D, Y = setlen(cross(X, up)),
		// Z = cross(Y, X); the template's (y, z) map onto Y and Z, a flip mirrors Y.
		let o = self.pv.origin;
		let x = norm([eye[0] - o[0], eye[1] - o[1], eye[2] - o[2]]);
		let y = norm(cross(x, [0.0, 0.0, 1.0]));
		let basis = (y, cross(y, x));
		let mirrored = (y.map(|c| -c), basis.1);
		let quad = |p: V3, (by, bz): (V3, V3), sy: f32, sz: f32, v: [[f32; 2]; 4], uv: [[f32; 2]; 4], c: [[f32; 4]; 4], tex: usize, additive: bool| {
			// Strip order (a, b, c, d) to fan order (a, b, d, c).
			let at = |k: usize| std::array::from_fn(|j| p[j] + by[j] * v[k][0] * sy + bz[j] * v[k][1] * sz);
			WorldQuad { pos: [at(0), at(1), at(3), at(2)], uv: [uv[0], uv[1], uv[3], uv[2]], colour: [c[0], c[1], c[3], c[2]], tex, additive }
		};
		let sheet = |q: [usize; 4], mid: [usize; 2], a: f32| q.map(|k| [SHEET_RGB, SHEET_RGB, SHEET_RGB, if mid.contains(&k) { a } else { 0.0 }]);
		for (i, s) in self.slots.iter().enumerate() {
			let b = if s.flip { mirrored } else { basis };
			if i < self.pv.quads {
				let uv = MAIN_UV.map(|[u, v]| [u - s.offset, v]);
				// Vertex alpha on the bottom (z = 0) verts, 0 on the top.
				let c = MAIN.map(|[_, z]| [self.pv.rgb[0], self.pv.rgb[1], self.pv.rgb[2], if z == 0.0 { self.fade(i) } else { 0.0 }]);
				out.push(quad(s.pos, b, s.w, s.h, MAIN, uv, c, self.pv.tex[0], true));
			} else {
				// Raw sizes; UV swapped: u = table V + offset, v = table U.
				for q in COLUMN_QUADS {
					let (v, uv) = (q.map(|k| COLUMN[k]), q.map(|k| [COLUMN_UV[k][1] + s.offset, COLUMN_UV[k][0]]));
					out.push(quad(s.pos, b, s.w, s.h, v, uv, sheet(q, COLUMN_MID, COLUMN_ALPHA), self.pv.tex[0], false));
				}
			}
		}
		if !self.pv.puffs {
			// Base = cuboid centre z - 0.5 |axis2|; y scaled by the extent, z by +0x4c, raw; UV swapped; plain basis.
			let a = self.vol.axes[2];
			let half = 0.5 * (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
			let p = [self.vol.centre[0], self.vol.centre[1], self.vol.centre[2] - half];
			for q in VOLUME_QUADS {
				let (v, uv) = (q.map(|k| VOLUME[k]), q.map(|k| [VOLUME_UV[k][1] + scroll, VOLUME_UV[k][0]]));
				out.push(quad(p, basis, self.extent, self.pv.extra, v, uv, sheet(q, VOLUME_MID, VOLUME_ALPHA), self.pv.tex[1], false));
			}
		}
	}
}

fn cross(a: V3, b: V3) -> V3 {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(v: V3) -> V3 {
	let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
	v.map(|c| c / l)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{gameplay, iso, level::Level, toc};
	use std::path::Path;

	const ISO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");

}
