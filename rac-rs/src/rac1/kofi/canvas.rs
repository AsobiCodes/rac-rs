//! Ko-fi drawings: the paper canvas and the felt-marker stroke. A stroke is a hand-drawn line: the points are densified, pushed
//! sideways by slow noise (the wobble), drawn as a chain of round capsules whose width drifts with the "pressure", and laid on
//! the paper with a multiply blend (marker over marker goes darker, like ink). The canvas is drawn at `ss` times the size and
//! averaged down at the end for smooth edges.

use super::rng::{Rng, hash2, noise1, noise2};

pub type P = (f32, f32);

/// One marker: colour, line width in canvas units, strength, crayon grain (0 = clean marker) and hand wobble in canvas units.
#[derive(Clone, Copy)]
pub struct Pen {
	pub col: [f32; 3],
	pub w: f32,
	pub a: f32,
	pub grain: f32,
	pub wob: f32,
}

impl Pen {
	/// Outline marker (a black or dark colour pen).
	pub fn ink(col: [f32; 3]) -> Pen {
		Pen { col, w: 4.4, a: 0.95, grain: 0.07, wob: 2.6 }
	}

	/// Fat crayon / marker used to colour things in.
	pub fn fill(col: [f32; 3]) -> Pen {
		Pen { col, w: 7.5, a: 0.85, grain: 0.4, wob: 2.0 }
	}

	pub fn width(mut self, w: f32) -> Pen {
		self.w = w;
		self
	}

	pub fn wobble(mut self, wob: f32) -> Pen {
		self.wob = wob;
		self
	}

	pub fn strength(mut self, a: f32) -> Pen {
		self.a = a;
		self
	}
}

/// The stroke's coverage, 0 .. 1 per canvas pixel, over a bounding box.
struct Mask {
	x0: i32,
	y0: i32,
	w: usize,
	h: usize,
	v: Vec<f32>,
}

pub struct Canvas {
	pub w: usize,
	pub h: usize,
	pub ss: f32,
	px: Vec<[f32; 3]>,
}

impl Canvas {
	/// A blank sheet of `lw` x `lh` canvas units, drawn at `ss` pixels per unit.
	pub fn new(lw: usize, lh: usize, ss: usize, paper: [f32; 3]) -> Canvas {
		let (w, h) = (lw * ss, lh * ss);
		Canvas { w, h, ss: ss as f32, px: vec![paper; w * h] }
	}

	/// Lays one stroke through `pts` (canvas units).
	pub fn stroke(&mut self, pts: &[P], pen: &Pen, rng: &mut Rng) {
		if pts.len() < 2 {
			return;
		}
		let seed = (rng.next_u64() & 0xffff_ffff) as u32;
		let pen = &Pen { w: pen.w * rng.range(0.82, 1.2), ..*pen };
		let (dense, arc) = densify(pts, 4.0);
		let line: Vec<(P, f32)> = wobble(&dense, &arc, pen, seed);
		let m = self.capsules(&line);
		self.lay(&m, pen, seed);
	}

	/// A round blob (the dot of an eye, a stamp of the marker).
	pub fn dot(&mut self, c: P, r: f32, pen: &Pen, rng: &mut Rng) {
		let seed = (rng.next_u64() & 0xffff_ffff) as u32;
		let p = Pen { w: r * 2.0, ..*pen };
		let a = (c.0 - 0.3, c.1 - 0.2);
		let b = (c.0 + 0.3, c.1 + 0.2);
		let m = self.capsules(&[(a, p.w * 0.5), (b, p.w * 0.5)]);
		self.lay(&m, &p, seed);
	}

	/// Capsules (round-ended segments with a drifting radius) of the hand-drawn line, as a coverage mask.
	fn capsules(&self, line: &[(P, f32)]) -> Mask {
		let ss = self.ss;
		let rmax = line.iter().map(|l| l.1).fold(0.0, f32::max) * ss + 2.0;
		let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
		for ((x, y), _) in line {
			lo = (lo.0.min(*x * ss), lo.1.min(*y * ss));
			hi = (hi.0.max(*x * ss), hi.1.max(*y * ss));
		}
		let x0 = ((lo.0 - rmax).floor() as i32).max(0);
		let y0 = ((lo.1 - rmax).floor() as i32).max(0);
		let x1 = ((hi.0 + rmax).ceil() as i32).min(self.w as i32);
		let y1 = ((hi.1 + rmax).ceil() as i32).min(self.h as i32);
		if x1 <= x0 || y1 <= y0 {
			return Mask { x0: 0, y0: 0, w: 0, h: 0, v: Vec::new() };
		}
		let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
		let mut v = vec![0.0f32; w * h];
		for pair in line.windows(2) {
			let ((ax, ay), ar) = (pair[0].0, pair[0].1 * ss);
			let ((bx, by), br) = (pair[1].0, pair[1].1 * ss);
			let (ax, ay, bx, by) = (ax * ss, ay * ss, bx * ss, by * ss);
			let r = ar.max(br) + 1.0;
			let sx0 = ((ax.min(bx) - r).floor() as i32).max(x0);
			let sx1 = ((ax.max(bx) + r).ceil() as i32).min(x1);
			let sy0 = ((ay.min(by) - r).floor() as i32).max(y0);
			let sy1 = ((ay.max(by) + r).ceil() as i32).min(y1);
			let (dx, dy) = (bx - ax, by - ay);
			let len2 = (dx * dx + dy * dy).max(1e-6);
			for y in sy0..sy1 {
				for x in sx0..sx1 {
					let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
					let t = (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0);
					let (qx, qy) = (ax + dx * t - px, ay + dy * t - py);
					let d = (qx * qx + qy * qy).sqrt();
					let rr = ar + (br - ar) * t;
					let cov = (rr - d + 0.5).clamp(0.0, 1.0);
					let i = (y - y0) as usize * w + (x - x0) as usize;
					if cov > v[i] {
						v[i] = cov;
					}
				}
			}
		}
		Mask { x0, y0, w, h, v }
	}

	/// Puts the mask on the paper with the pen's colour: multiply blend with crayon grain and a slow streakiness.
	fn lay(&mut self, m: &Mask, pen: &Pen, seed: u32) {
		let s2 = seed ^ 0x5bd1_e995;
		for my in 0..m.h {
			for mx in 0..m.w {
				let c = m.v[my * m.w + mx];
				if c <= 0.0 {
					continue;
				}
				let (x, y) = (m.x0 + mx as i32, m.y0 + my as i32);
				let g = 1.0 - pen.grain * hash2(x, y, seed);
				let streak = 0.9 + 0.1 * noise2(x as f32 * 0.02, y as f32 * 0.02, s2);
				let a = (c * pen.a * g * streak).min(1.0);
				let p = &mut self.px[y as usize * self.w + x as usize];
				for k in 0..3 {
					p[k] *= 1.0 - a * (1.0 - pen.col[k]);
				}
			}
		}
	}

	/// The sheet averaged down to `self.w / ss` x `self.h / ss` pixels, as linear 0 .. 1 floats.
	pub fn downsample(&self) -> (usize, usize, Vec<[f32; 3]>) {
		let s = self.ss as usize;
		let (w, h) = (self.w / s, self.h / s);
		let mut out = vec![[0.0f32; 3]; w * h];
		let k = 1.0 / (s * s) as f32;
		for y in 0..h {
			for x in 0..w {
				let mut acc = [0.0f32; 3];
				for sy in 0..s {
					for sx in 0..s {
						let p = self.px[(y * s + sy) * self.w + x * s + sx];
						for c in 0..3 {
							acc[c] += p[c];
						}
					}
				}
				out[y * w + x] = [acc[0] * k, acc[1] * k, acc[2] * k];
			}
		}
		(w, h, out)
	}
}

/// The points of `pts` with at most `step` between neighbours, and the arc length at each.
fn densify(pts: &[P], step: f32) -> (Vec<P>, Vec<f32>) {
	let mut dense = vec![pts[0]];
	let mut arc = vec![0.0f32];
	for w in pts.windows(2) {
		let (a, b) = (w[0], w[1]);
		let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
		let n = ((len / step).ceil() as usize).max(1);
		for k in 1..=n {
			let t = k as f32 / n as f32;
			let last = *dense.last().unwrap();
			let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
			arc.push(arc.last().unwrap() + ((p.0 - last.0).powi(2) + (p.1 - last.1).powi(2)).sqrt());
			dense.push(p);
		}
	}
	(dense, arc)
}

/// The hand: slow sideways noise on the points and the drifting pen pressure (radius per point).
fn wobble(dense: &[P], arc: &[f32], pen: &Pen, seed: u32) -> Vec<(P, f32)> {
	let n = dense.len();
	let mut out = Vec::with_capacity(n);
	for i in 0..n {
		let (a, b) = (dense[i.saturating_sub(1)], dense[(i + 1).min(n - 1)]);
		let (tx, ty) = (b.0 - a.0, b.1 - a.1);
		let tl = (tx * tx + ty * ty).sqrt().max(1e-4);
		let (nx, ny) = (-ty / tl, tx / tl);
		let s = arc[i];
		let d = pen.wob * (noise1(s / 70.0 + 0.5, seed) + 0.7 * noise1(s / 23.0, seed.wrapping_add(1)) + 0.12 * noise1(s / 13.0, seed.wrapping_add(7)));
		let press = 0.84 + 0.2 * noise1(s / 60.0, seed.wrapping_add(3)) + if i < 4 { 0.13 } else { 0.0 };
		out.push(((dense[i].0 + nx * d, dense[i].1 + ny * d), 0.5 * pen.w * press));
	}
	out
}
