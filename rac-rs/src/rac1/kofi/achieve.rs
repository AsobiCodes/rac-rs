//! Ko-fi drawings: khalifax10's sheet (the biggest supporter so far, who suggested the achievements): Ratchet and Clank cheering round a
//! big gold trophy on a winner's step, a gold-star sticker, small trophies and stars. Same felt-marker hand as the other sheets.

use super::canvas::Pen;
use super::canvas::P;
use super::font;
use super::hand::Hand;
use super::people;
use super::shapes::{self, BLACK, BLUE, GREEN, ORANGE, PINK, PURPLE, RED, YELLOW, ink, ring};

const GOLD: [f32; 3] = [1.0, 0.8, 0.1];
const RAINBOW: [[f32; 3]; 6] = [RED, ORANGE, [0.1, 0.65, 0.2], BLUE, PURPLE, [0.9, 0.3, 0.6]];
const WARM: [[f32; 3]; 4] = [RED, ORANGE, PURPLE, BLUE];

/// A gold cup standing on `base` (bottom centre) and `s` times the size of the big one (about 250 high at s = 1).
pub fn trophy(h: &mut Hand, base: P, s: f32) {
	let at = |x: f32, y: f32| -> P { (base.0 + x * s, base.1 + y * s) };
	let outline = Pen::ink(BLACK).width((5.0 * s).max(3.4));
	for sx in [-1.0f32, 1.0] {
		let (a0, a1) = if sx > 0.0 { (-1.25, 1.25) } else { (3.14 - 1.25, 3.14 + 1.25) };
		h.arc(at(sx * 76.0, -190.0), 40.0 * s, 46.0 * s, a0, a1, &Pen::ink(ORANGE).width((9.0 * s).max(4.0)));
	}
	let bowl = [at(-82.0, -250.0), at(82.0, -250.0), at(64.0, -160.0), at(26.0, -116.0), at(-26.0, -116.0), at(-64.0, -160.0)];
	h.colour_in(&bowl, GOLD, true);
	h.outline(&bowl, &outline);
	let stem = [at(-12.0, -116.0), at(12.0, -116.0), at(14.0, -34.0), at(-14.0, -34.0)];
	h.colour_in(&stem, GOLD, false);
	h.outline(&stem, &outline);
	let foot = [at(-58.0, -34.0), at(58.0, -34.0), at(64.0, 0.0), at(-64.0, 0.0)];
	h.colour_in(&foot, [0.75, 0.52, 0.12], false);
	h.outline(&foot, &outline);
	shapes::star(h, at(0.0, -188.0), 38.0 * s, RED);
	h.line(at(-56.0, -236.0), at(-50.0, -190.0), &Pen::ink([1.0, 1.0, 0.7]).width((6.0 * s).max(3.0)));
}

/// The round gold sticker with a star on it.
fn sticker(h: &mut Hand, c: P, r: f32) {
	h.colour_in(&ring(c, r * 0.92, r * 0.92, 12), GOLD, true);
	h.ellipse(c, r, r * 0.97, &Pen::ink(ORANGE).width(7.0));
	h.ellipse(c, r * 0.8, r * 0.78, &Pen::ink(ORANGE).width(3.5));
	shapes::star(h, (c.0, c.1 + r * 0.04), r * 0.52, RED);
}

fn sparkle(h: &mut Hand, c: P, r: f32) {
	let pen = Pen::ink(GOLD).width(5.0);
	h.line((c.0 - r, c.1), (c.0 + r, c.1), &pen);
	h.line((c.0, c.1 - r), (c.0, c.1 + r), &pen);
}

pub fn khalifax10(h: &mut Hand) -> Vec<String> {
	let mut t = Vec::new();
	let n = "khalifax10";
	t.push(font::text(h, n, 512.0 - font::width(n, 96.0) * 0.5 - 30.0, 30.0, 96.0, &RAINBOW));
	sticker(h, (925.0, 135.0), 68.0);
	h.curve(&[(905.0, 205.0), (880.0, 270.0), (905.0, 330.0)], &Pen::ink(RED).width(7.0));
	h.curve(&[(945.0, 205.0), (968.0, 265.0), (950.0, 330.0)], &Pen::ink(BLUE).width(7.0));
	shapes::cloud(h, (140.0, 210.0), 150.0);
	shapes::sun(h, (905.0, 430.0), 40.0);
	// The ground and the winner's step with its big 1.
	let mut pts = Vec::new();
	let mut x = 20.0;
	while x <= 1004.0 {
		pts.push((x, 622.0 + h.rng.s() * 6.0));
		x += 90.0;
	}
	h.curve(&pts, &Pen::ink(GREEN).width(6.0).wobble(3.0));
	shapes::grass(h, 30.0, 995.0, 626.0);
	let step = [(402.0, 505.0), (622.0, 503.0), (626.0, 622.0), (398.0, 624.0)];
	h.colour_in(&step, [0.55, 0.62, 0.85], true);
	h.outline(&step, &ink());
	t.push(font::text(h, "1", 484.0, 530.0, 78.0, &WARM));
	trophy(h, (512.0, 503.0), 1.0);
	people::ratchet(h, (215.0, 440.0), 1.1, true);
	people::clank(h, (815.0, 482.0), 1.05);
	// Small trophies, stars and sparkles: the achievements.
	trophy(h, (95.0, 395.0), 0.34);
	trophy(h, (330.0, 270.0), 0.3);
	trophy(h, (715.0, 285.0), 0.32);
	for (x, y, r, c) in [(400.0, 190.0, 18.0, YELLOW), (640.0, 195.0, 15.0, ORANGE), (180.0, 330.0, 14.0, PINK), (860.0, 350.0, 17.0, YELLOW), (95.0, 560.0, 16.0, ORANGE), (960.0, 560.0, 14.0, PINK)] {
		shapes::star(h, (x, y), r, c);
	}
	for (x, y, r) in [(420.0, 270.0, 16.0), (610.0, 262.0, 14.0), (340.0, 470.0, 12.0), (690.0, 440.0, 13.0), (245.0, 150.0, 12.0)] {
		sparkle(h, (x, y), r);
	}
	shapes::heart(h, (560.0, 160.0), 20.0);
	let b = "THANK YOU!";
	t.push(font::text(h, b, 512.0 - font::width(b, 70.0) * 0.5, 672.0, 70.0, &WARM));
	t
}
