//! Ko-fi drawings: the things a kid puts on a picture: sun, clouds, house, bolts, wrench, hearts, stars, grass, a planet.
//! All of them are drawn with the `Hand`, so every line wobbles and overshoots.

use super::canvas::{P, Pen};
use super::hand::Hand;
use super::rng::{cos, sin};

pub const BLACK: [f32; 3] = [0.07, 0.07, 0.09];
pub const RED: [f32; 3] = [0.9, 0.1, 0.1];
pub const BLUE: [f32; 3] = [0.15, 0.3, 0.85];
pub const LBLUE: [f32; 3] = [0.35, 0.6, 0.95];
pub const YELLOW: [f32; 3] = [1.0, 0.82, 0.1];
pub const ORANGE: [f32; 3] = [0.98, 0.5, 0.08];
pub const GREEN: [f32; 3] = [0.15, 0.7, 0.2];
pub const PURPLE: [f32; 3] = [0.5, 0.2, 0.8];
pub const PINK: [f32; 3] = [0.95, 0.4, 0.65];
pub const BROWN: [f32; 3] = [0.5, 0.3, 0.15];

const TAU: f32 = 6.283_185_5;

pub fn ink() -> Pen {
	Pen::ink(BLACK)
}

/// Points on an ellipse, for polygons to colour in.
pub fn ring(c: P, rx: f32, ry: f32, n: usize) -> Vec<P> {
	(0..n).map(|i| (c.0 + cos(i as f32 / n as f32 * TAU) * rx, c.1 + sin(i as f32 / n as f32 * TAU) * ry)).collect()
}

pub fn sun(h: &mut Hand, c: P, r: f32) {
	let col = Pen::ink(ORANGE).width(5.5);
	h.colour_in(&ring(c, r * 0.9, r * 0.9, 12), YELLOW, true);
	h.ellipse(c, r, r * 0.96, &col);
	for i in 0..11 {
		let a = i as f32 / 11.0 * TAU + h.rng.s() * 0.1;
		let (r0, r1) = (r * 1.25, r * h.rng.range(1.55, 1.9));
		h.spike((c.0 + cos(a) * r0, c.1 + sin(a) * r0), (c.0 + cos(a) * r1, c.1 + sin(a) * r1), &col);
	}
	h.dot((c.0 - r * 0.3, c.1 - r * 0.15), r * 0.07, &ink());
	h.dot((c.0 + r * 0.3, c.1 - r * 0.15), r * 0.07, &ink());
	h.arc((c.0, c.1 - r * 0.05), r * 0.45, r * 0.4, 0.25, 2.9, &ink().width(4.0));
}

/// A bumpy cloud outline in a loose blue line.
pub fn cloud(h: &mut Hand, c: P, w: f32) {
	let pen = Pen::ink(BLUE).width(5.0).wobble(2.4);
	let u = w / 4.0;
	let pts = [
		(c.0 - 2.0 * u, c.1 + 0.3 * u),
		(c.0 - 1.7 * u, c.1 - 0.5 * u),
		(c.0 - 0.8 * u, c.1 - 0.95 * u),
		(c.0 + 0.1 * u, c.1 - 0.6 * u),
		(c.0 + 0.9 * u, c.1 - 0.9 * u),
		(c.0 + 1.8 * u, c.1 - 0.35 * u),
		(c.0 + 2.0 * u, c.1 + 0.35 * u),
		(c.0 + 0.5 * u, c.1 + 0.55 * u),
		(c.0 - 1.0 * u, c.1 + 0.5 * u),
	];
	h.closed(&pts, &pen);
	h.scribble_blob((c.0 - 0.2 * u, c.1 + 0.05 * u), u * 1.5, u * 0.35, 3, &Pen::ink(LBLUE).width(5.0));
}

/// A lopsided house: walls, a red roof, a door and a window, the roof a bit too big.
pub fn house(h: &mut Hand, x: f32, y: f32, w: f32, hh: f32) {
	let wall = [(x, y), (x + w * 1.02, y + 4.0), (x + w * 0.98, y + hh), (x + 6.0, y + hh - 5.0)];
	h.outline(&wall, &ink());
	h.colour_in(&[(x + 4.0, y + 6.0), (x + w - 4.0, y + 8.0), (x + w - 8.0, y + hh - 8.0), (x + 8.0, y + hh - 10.0)], [0.98, 0.9, 0.5], false);
	let roof = [(x - w * 0.12, y + 2.0), (x + w * 0.46, y - hh * 0.55), (x + w * 1.14, y + 6.0)];
	h.outline(&roof, &Pen::ink(RED).width(6.0));
	h.colour_in(&[(x + w * 0.1, y - 4.0), (x + w * 0.46, y - hh * 0.4), (x + w * 0.9, y - 2.0)], RED, false);
	let d = [(x + w * 0.12, y + hh), (x + w * 0.14, y + hh * 0.5), (x + w * 0.38, y + hh * 0.48), (x + w * 0.4, y + hh - 4.0)];
	h.path(&d, &ink());
	h.dot((x + w * 0.34, y + hh * 0.76), 4.0, &ink());
	let wd = [(x + w * 0.58, y + hh * 0.3), (x + w * 0.85, y + hh * 0.32), (x + w * 0.84, y + hh * 0.6), (x + w * 0.56, y + hh * 0.58)];
	h.outline(&wd, &Pen::ink(BLUE));
	h.line((x + w * 0.7, y + hh * 0.3), (x + w * 0.7, y + hh * 0.6), &Pen::ink(BLUE).width(3.5));
}

/// A bolt: a hexagon nut with a hole, gold.
pub fn bolt(h: &mut Hand, c: P, r: f32) {
	let hex: Vec<P> = (0..6).map(|i| (c.0 + cos(i as f32 / 6.0 * TAU + 0.3) * r, c.1 + sin(i as f32 / 6.0 * TAU + 0.3) * r * 0.9)).collect();
	h.colour_in(&hex, [1.0, 0.75, 0.05], true);
	h.outline(&hex, &ink().width(4.2));
	h.ellipse(c, r * 0.38, r * 0.36, &ink().width(3.6));
}

/// A wrench from `a` (the head end) to `b` (the handle end), `t` thick.
pub fn wrench(h: &mut Hand, a: P, b: P, t: f32, col: [f32; 3]) {
	let (dx, dy) = (b.0 - a.0, b.1 - a.1);
	let l = (dx * dx + dy * dy).sqrt().max(1.0);
	let (ux, uy) = (dx / l, dy / l);
	let (nx, ny) = (-uy, ux);
	let q = |s: f32, k: f32| (a.0 + ux * s + nx * k, a.1 + uy * s + ny * k);
	let handle = [q(t * 1.0, -t * 0.4), q(l - t * 0.4, -t * 0.4), q(l, 0.0), q(l - t * 0.4, t * 0.4), q(t * 1.0, t * 0.4)];
	h.colour_in(&handle, col, false);
	h.outline(&handle, &ink());
	let jaw = [q(t * 1.2, -t * 0.3), q(t * 0.9, -t * 1.2), q(-t * 0.3, -t * 1.3), q(-t * 0.9, -t * 0.5), q(-t * 0.2, -t * 0.15), q(-t * 0.1, t * 0.15), q(-t * 0.9, t * 0.5), q(-t * 0.3, t * 1.3), q(t * 0.9, t * 1.2), q(t * 1.2, t * 0.3)];
	h.colour_in(&jaw, col, false);
	h.path(&jaw, &ink());
}

pub fn heart(h: &mut Hand, c: P, s: f32) {
	let pts = [
		(c.0, c.1 + s * 0.9),
		(c.0 - s * 0.9, c.1 + s * 0.1),
		(c.0 - s * 0.75, c.1 - s * 0.6),
		(c.0 - s * 0.2, c.1 - s * 0.55),
		(c.0, c.1 - s * 0.2),
		(c.0 + s * 0.2, c.1 - s * 0.55),
		(c.0 + s * 0.75, c.1 - s * 0.6),
		(c.0 + s * 0.9, c.1 + s * 0.1),
	];
	let poly = h.closed(&pts, &Pen::ink(RED).width(5.5));
	h.colour_in(&poly, PINK, false);
}

/// A five-pointed star in one go (the way kids draw it).
pub fn star(h: &mut Hand, c: P, r: f32, col: [f32; 3]) {
	let p: Vec<P> = [0usize, 2, 4, 1, 3, 0].iter().map(|k| (c.0 + cos(*k as f32 / 5.0 * TAU - 1.57) * r, c.1 + sin(*k as f32 / 5.0 * TAU - 1.57) * r)).collect();
	let mut q = p.clone();
	q.push((p[1].0 * 0.9 + p[0].0 * 0.1, p[1].1 * 0.9 + p[0].1 * 0.1));
	h.path(&q, &Pen::ink(col).width(5.0));
}

pub fn grass(h: &mut Hand, x0: f32, x1: f32, y: f32) {
	let mut x = x0;
	while x < x1 {
		let hgt = h.rng.range(12.0, 30.0);
		let lean = h.rng.range(-8.0, 8.0);
		h.line((x, y), (x + lean, y - hgt), &Pen::ink(GREEN).width(4.0).wobble(1.2));
		x += h.rng.range(10.0, 22.0);
	}
}

/// A planet with a ring.
pub fn planet(h: &mut Hand, c: P, r: f32, col: [f32; 3]) {
	h.colour_in(&ring(c, r * 0.9, r * 0.9, 12), col, true);
	h.ellipse(c, r, r, &ink());
	h.ellipse_turns((c.0, c.1 + r * 0.1), r * 1.7, r * 0.4, 0.95, &Pen::ink(PURPLE).width(5.5));
}
