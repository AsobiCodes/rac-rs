//! Ko-fi drawings: the child's hand. Wobbly lines, smooth curves through a few points, loops that overshoot and do not close,
//! round dots and zig-zag scribble that colours a shape in (and goes a little over the edge). The shapes (people, ship, sun) and
//! the letters are written with these.

use super::canvas::{Canvas, P, Pen};
use super::rng::{Rng, cos, noise1, sin};

const TAU: f32 = 6.283_185_5;

pub struct Hand {
	pub c: Canvas,
	pub rng: Rng,
}

impl Hand {
	pub fn new(c: Canvas, seed: u64) -> Hand {
		Hand { c, rng: Rng::new(seed) }
	}

	/// A straight-ish line.
	pub fn line(&mut self, a: P, b: P, pen: &Pen) {
		self.path(&[a, b], pen);
	}

	/// A line through the points, corners kept sharp.
	pub fn path(&mut self, pts: &[P], pen: &Pen) {
		let mut v: Vec<P> = pts.iter().map(|p| (p.0 + self.rng.s() * pen.wob * 0.8, p.1 + self.rng.s() * pen.wob * 0.8)).collect();
		if v.len() >= 2 && pen.wob > 0.0 {
			let n = v.len();
			let over = self.rng.range(0.0, 0.07);
			let (a, b) = (v[0], v[1]);
			v[0] = (a.0 + (a.0 - b.0) * over, a.1 + (a.1 - b.1) * over);
			let over = self.rng.range(0.0, 0.07);
			let (a, b) = (v[n - 1], v[n - 2]);
			v[n - 1] = (a.0 + (a.0 - b.0) * over, a.1 + (a.1 - b.1) * over);
		}
		let mut r = self.rng.fork();
		self.c.stroke(&v, pen, &mut r);
	}

	/// A smooth curve through the points (Catmull-Rom).
	pub fn curve(&mut self, pts: &[P], pen: &Pen) {
		let sm = catmull(pts);
		let mut r = self.rng.fork();
		self.c.stroke(&sm, pen, &mut r);
	}

	/// A polygon outline drawn as one line that runs a little past its start.
	pub fn outline(&mut self, pts: &[P], pen: &Pen) {
		let mut v: Vec<P> = pts.to_vec();
		v.push(pts[0]);
		if pts.len() > 1 {
			let o = self.rng.range(0.1, 0.3);
			v.push((pts[0].0 + (pts[1].0 - pts[0].0) * o, pts[0].1 + (pts[1].1 - pts[0].1) * o));
		}
		self.path(&v, pen);
	}

	/// A round dot.
	pub fn dot(&mut self, c: P, r: f32, pen: &Pen) {
		let mut rr = self.rng.fork();
		self.c.dot(c, r, pen, &mut rr);
	}

	/// A looped ellipse, drawn in one go: it starts anywhere, turns `turns` times and does not meet its start.
	pub fn ellipse(&mut self, c: P, rx: f32, ry: f32, pen: &Pen) {
		let turns = self.rng.range(1.06, 1.18);
		self.ellipse_turns(c, rx, ry, turns, pen);
	}

	pub fn ellipse_turns(&mut self, c: P, rx: f32, ry: f32, turns: f32, pen: &Pen) {
		let a0 = self.rng.range(0.0, TAU);
		let dir = if self.rng.f() < 0.5 { 1.0 } else { -1.0 };
		let drift = self.rng.range(-0.06, 0.07);
		let seed = (self.rng.next_u64() & 0xffff) as u32;
		let n = (((rx + ry) * 0.5 * turns * 0.9) as usize).clamp(14, 90);
		let mut pts = Vec::with_capacity(n + 1);
		for i in 0..=n {
			let t = i as f32 / n as f32;
			let a = a0 + dir * turns * TAU * t;
			let k = 1.0 + 0.05 * noise1(t * 3.2, seed) + drift * t;
			pts.push((c.0 + cos(a) * rx * k, c.1 + sin(a) * ry * k));
		}
		self.path(&pts, pen);
	}

	/// An arc from angle `a0` to `a1` (radians, y down) of an ellipse, as a smooth line (smiles, eyebrows).
	pub fn arc(&mut self, c: P, rx: f32, ry: f32, a0: f32, a1: f32, pen: &Pen) {
		let n = (((a1 - a0).abs() * (rx + ry) * 0.12) as usize).clamp(6, 40);
		let pts: Vec<P> = (0..=n)
			.map(|i| {
				let a = a0 + (a1 - a0) * i as f32 / n as f32;
				(c.0 + cos(a) * rx, c.1 + sin(a) * ry)
			})
			.collect();
		self.path(&pts, pen);
	}

	/// Zig-zag scribble over a convex-ish polygon: the line goes back and forth across it at `angle` (radians), `gap` apart, and
	/// pokes a little out at the turns.
	pub fn hatch(&mut self, poly: &[P], angle: f32, gap: f32, pen: &Pen) {
		let (ca, sa) = (cos(angle), sin(angle));
		let local: Vec<P> = poly.iter().map(|p| (p.0 * ca + p.1 * sa, -p.0 * sa + p.1 * ca)).collect();
		let (ymin, ymax) = local.iter().fold((f32::MAX, f32::MIN), |m, p| (m.0.min(p.1), m.1.max(p.1)));
		let mut pts: Vec<P> = Vec::new();
		let mut k = 0;
		let mut y = ymin + gap * 0.3;
		while y < ymax {
			if let Some((x0, x1)) = span(&local, y) {
				let over = self.rng.range(-0.6, 2.2) * pen.w;
				let jitter = self.rng.s() * gap * 0.3;
				let x = if k % 2 == 0 { x0 - over } else { x1 + over };
				let (lx, ly) = (x, y + jitter);
				pts.push((lx * ca - ly * sa, lx * sa + ly * ca));
				k += 1;
			}
			y += gap * 0.5;
		}
		if pts.len() >= 2 {
			self.path(&pts, pen);
		}
	}

	/// Colours a polygon in with two crossing passes of scribble when `cross`, else one.
	pub fn colour_in(&mut self, poly: &[P], col: [f32; 3], cross: bool) {
		let pen = Pen::fill(col);
		let a = self.rng.range(0.5, 1.1);
		self.hatch(poly, a, pen.w * 1.8, &pen);
		if cross && self.rng.f() < 0.5 {
			self.hatch(poly, a + 1.5, pen.w * 2.6, &pen.strength(0.7));
		}
	}

	/// Loose scribble inside an ellipse (clouds, scribbled sky, bushes): a looping line that wanders across it.
	pub fn scribble_blob(&mut self, c: P, rx: f32, ry: f32, loops: usize, pen: &Pen) {
		let mut pts: Vec<P> = Vec::new();
		let n = loops * 8;
		let seed = (self.rng.next_u64() & 0xffff) as u32;
		for i in 0..=n {
			let t = i as f32 / n as f32;
			let a = t * loops as f32 * TAU;
			let sweep = -1.0 + 2.0 * t;
			let wob = 0.75 + 0.25 * noise1(t * 6.0, seed);
			pts.push((c.0 + (sweep * rx * 0.8 + cos(a) * rx * 0.28) * wob, c.1 + sin(a) * ry * 0.75 * wob + noise1(t * 4.0, seed + 9) * ry * 0.2));
		}
		self.curve(&pts, pen);
	}

	/// A closed smooth shape through the points, drawn as one line that runs a little past where it began. Returns the outline
	/// (for colouring in).
	pub fn closed(&mut self, pts: &[P], pen: &Pen) -> Vec<P> {
		let ring = catmull_ring(pts);
		let mut v = ring.clone();
		v.push(ring[0]);
		let over = (ring.len() / 10).max(2);
		let (dx, dy) = (self.rng.s() * pen.w * 1.5, self.rng.s() * pen.w * 1.5);
		for (i, p) in ring.iter().enumerate().skip(1).take(over) {
			let k = i as f32 / over as f32;
			v.push((p.0 + dx * k, p.1 + dy * k));
		}
		self.path(&v, pen);
		ring
	}

	/// A short spiky ray / hair line from `a` to `b`.
	pub fn spike(&mut self, a: P, b: P, pen: &Pen) {
		self.line(a, b, &pen.wobble(pen.wob * 0.5));
	}
}

/// The leftmost and rightmost x of the polygon on the horizontal line `y`.
fn span(poly: &[P], y: f32) -> Option<(f32, f32)> {
	let mut lo = f32::MAX;
	let mut hi = f32::MIN;
	let n = poly.len();
	for i in 0..n {
		let (a, b) = (poly[i], poly[(i + 1) % n]);
		if (a.1 <= y && b.1 > y) || (b.1 <= y && a.1 > y) {
			let x = a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0);
			lo = lo.min(x);
			hi = hi.max(x);
		}
	}
	(lo <= hi).then_some((lo, hi))
}

/// Uniform Catmull-Rom through `pts`, 8 samples a span.
fn catmull(pts: &[P]) -> Vec<P> {
	if pts.len() < 3 {
		return pts.to_vec();
	}
	let n = pts.len();
	let mut out = Vec::new();
	for i in 0..n - 1 {
		let p0 = pts[i.saturating_sub(1)];
		let (p1, p2) = (pts[i], pts[i + 1]);
		let p3 = pts[(i + 2).min(n - 1)];
		for s in 0..8 {
			let t = s as f32 / 8.0;
			let (t2, t3) = (t * t, t * t * t);
			let f = |a: f32, b: f32, c: f32, d: f32| 0.5 * ((2.0 * b) + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3);
			out.push((f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1)));
		}
	}
	out.push(pts[n - 1]);
	out
}

/// Catmull-Rom through `pts` as a closed ring, 8 samples a span.
fn catmull_ring(pts: &[P]) -> Vec<P> {
	let n = pts.len();
	let mut out = Vec::new();
	for i in 0..n {
		let (p0, p1, p2, p3) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
		for s in 0..8 {
			let t = s as f32 / 8.0;
			let (t2, t3) = (t * t, t * t * t);
			let f = |a: f32, b: f32, c: f32, d: f32| 0.5 * ((2.0 * b) + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3);
			out.push((f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1)));
		}
	}
	out
}
