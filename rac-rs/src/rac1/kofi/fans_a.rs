//! Ko-fi drawings: the sheets of DarkBlaze, Smoe05, eardrummer, Chris, Ryan Palmer, fleece and Justintime4u2bu (user 2026-11-04: "we need drawing for each one"). Same felt-marker hand as the other
//! sheets (scenes.rs, achieve.rs); every sheet has its own scene: a flame gun, a paint roller, a drum kit, a monitor, four game boxes, a sheep in a helmet, an alarm clock and a heart.

use super::canvas::{P, Pen};
use super::font;
use super::hand::Hand;
use super::people;
use super::rng::{cos, sin};
use super::shapes::{self, BLACK, BLUE, BROWN, GREEN, LBLUE, ORANGE, PINK, PURPLE, RED, YELLOW, ink, ring};

type C = [f32; 3];

const SILVER: C = [0.7, 0.74, 0.82];
const TAN: C = [0.95, 0.8, 0.62];
const FIRE: [C; 4] = [RED, ORANGE, PURPLE, [0.85, 0.15, 0.4]];
const RAINBOW: [C; 6] = [RED, ORANGE, GREEN, BLUE, PURPLE, PINK];
const TAU: f32 = 6.283_185_5;

fn pn(col: C, w: f32) -> Pen {
	Pen::ink(col).width(w)
}

/// A convex polygon coloured in and outlined.
fn solid(h: &mut Hand, pts: &[P], fill: C) {
	h.colour_in(pts, fill, false);
	h.outline(pts, &ink());
}

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> [P; 4] {
	[(x0, y0), (x1, y0 + 2.0), (x1 - 2.0, y1), (x0 + 2.0, y1 - 2.0)]
}

/// The name big at the top, shrunk so it fits the page.
fn title(h: &mut Hand, name: &str, pal: &[C]) -> String {
	let hgt = (960.0 / font::width(name, 1.0)).min(96.0);
	font::text(h, name, 512.0 - font::width(name, hgt) * 0.5, 30.0, hgt, pal)
}

fn thanks(h: &mut Hand, pal: &[C]) -> String {
	let b = "THANK YOU!";
	font::text(h, b, 512.0 - font::width(b, 70.0) * 0.5, 672.0, 70.0, pal)
}

/// A blob of paint with flying drops.
fn splat(h: &mut Hand, c: P, r: f32, col: C) {
	h.colour_in(&ring(c, r, r * 0.8, 10), col, false);
	h.ellipse(c, r * 1.05, r * 0.85, &pn(col, 5.0));
	for i in 0..6 {
		let a = h.rng.range(0.0, TAU);
		let d = r * h.rng.range(1.5, 2.1);
		let dr = r * h.rng.range(0.14, 0.26);
		h.dot((c.0 + cos(a) * d, c.1 + sin(a) * d * 0.8), dr, &pn(col, 8.0));
		if i < 2 {
			h.line((c.0 + cos(a) * r, c.1 + sin(a) * r * 0.8), (c.0 + cos(a) * d * 0.8, c.1 + sin(a) * d * 0.65), &pn(col, 5.0));
		}
	}
}

/// A music note (`beamed`: two eighth notes joined).
fn note(h: &mut Hand, c: P, s: f32, col: C, beamed: bool) {
	let head = |h: &mut Hand, p: P| {
		h.colour_in(&ring(p, 15.0 * s, 11.0 * s, 8), col, false);
		h.ellipse(p, 16.0 * s, 12.0 * s, &pn(col, 5.0));
	};
	head(h, c);
	h.line((c.0 + 14.0 * s, c.1 - 2.0 * s), (c.0 + 14.0 * s, c.1 - 64.0 * s), &pn(col, 5.5));
	if beamed {
		let p2 = (c.0 + 48.0 * s, c.1 - 10.0 * s);
		head(h, p2);
		h.line((p2.0 + 14.0 * s, p2.1 - 2.0 * s), (p2.0 + 14.0 * s, p2.1 - 62.0 * s), &pn(col, 5.5));
		h.line((c.0 + 14.0 * s, c.1 - 64.0 * s), (p2.0 + 14.0 * s, p2.1 - 62.0 * s), &pn(col, 9.0));
	} else {
		h.curve(&[(c.0 + 14.0 * s, c.1 - 64.0 * s), (c.0 + 30.0 * s, c.1 - 48.0 * s), (c.0 + 32.0 * s, c.1 - 30.0 * s)], &pn(col, 5.5));
	}
}

/// A flame tongue from `base` toward angle `ang` (radians, y down), `len` long and `wid` wide at the base.
fn flame(h: &mut Hand, base: P, ang: f32, len: f32, wid: f32, col: C) {
	let (d, n) = ((cos(ang), sin(ang)), (-sin(ang), cos(ang)));
	let bend = h.rng.s() * wid * 0.6;
	let at = |a: f32, b: f32| -> P { (base.0 + d.0 * a + n.0 * b, base.1 + d.1 * a + n.1 * b) };
	let pts = [at(0.0, wid), at(len * 0.42, wid * 1.25), at(len, bend), at(len * 0.42, -wid * 1.2), at(0.0, -wid)];
	h.colour_in(&pts, col, true);
	h.outline(&pts, &pn(if col == YELLOW { ORANGE } else { RED }, 5.0));
}

fn ground(h: &mut Hand, y: f32, col: C) {
	let mut pts = Vec::new();
	let mut x = 20.0;
	while x <= 1004.0 {
		pts.push((x, y + h.rng.s() * 5.0));
		x += 90.0;
	}
	h.curve(&pts, &pn(col, 6.0).wobble(3.0));
}

// ---------------------------------------------------------------------------------------------------------------- DarkBlaze

/// The chunky toy flame gun, muzzle at (436, 509), grip at Ratchet's hand.
fn flame_gun(h: &mut Hand) {
	let tank = ring((598.0, 446.0), 34.0, 24.0, 10);
	h.colour_in(&tank, LBLUE, true);
	h.ellipse((598.0, 446.0), 38.0, 27.0, &ink());
	h.curve(&[(598.0, 472.0), (570.0, 480.0), (600.0, 484.0)], &pn(BLACK, 6.0));
	solid(h, &[(648.0, 486.0), (556.0, 480.0), (552.0, 542.0), (646.0, 550.0)], YELLOW);
	solid(h, &[(556.0, 492.0), (470.0, 490.0), (468.0, 530.0), (554.0, 532.0)], RED);
	solid(h, &[(470.0, 468.0), (438.0, 464.0), (434.0, 554.0), (468.0, 550.0)], PURPLE);
	solid(h, &[(616.0, 546.0), (652.0, 552.0), (646.0, 612.0), (608.0, 604.0)], BLUE);
	for x in [585.0, 620.0] {
		h.dot((x, 512.0), 6.0, &pn(BLACK, 6.0));
	}
	h.line((540.0, 492.0), (538.0, 530.0), &pn(BLACK, 4.0));
	h.path(&[(568.0, 552.0), (578.0, 580.0), (596.0, 574.0)], &pn(BLACK, 5.0));
}

fn blaze(h: &mut Hand) {
	// The fan of big flames from the nozzle, longest in the middle.
	for (k, len, col) in [(-1.0f32, 250.0, PURPLE), (1.0, 250.0, RED), (0.0, 340.0, ORANGE)] {
		flame(h, (426.0, 509.0 + k * 10.0), 3.1416 + k * 0.42, len, 34.0, col);
	}
	flame(h, (426.0, 509.0), 3.1416, 190.0, 18.0, YELLOW);
	for (x, len, col) in [(70.0, 250.0, PURPLE), (150.0, 170.0, ORANGE), (40.0, 120.0, RED)] {
		let a = -1.57 + h.rng.s() * 0.15;
		flame(h, (x, 640.0), a, len, 38.0, col);
	}
	for (x, y) in [(300.0, 210.0), (190.0, 300.0), (120.0, 450.0), (330.0, 600.0), (230.0, 560.0), (410.0, 300.0), (520.0, 230.0)] {
		h.dot((x, y), 7.0, &pn(if x > 250.0 { YELLOW } else { ORANGE }, 12.0));
	}
	for (x, y) in [(60.0, 200.0), (460.0, 190.0)] {
		flame(h, (x, y + 70.0), -1.57, 90.0, 18.0, RED);
	}
}

pub fn darkblaze(h: &mut Hand) -> Vec<String> {
	let t0 = title(h, "DarkBlaze", &FIRE);
	blaze(h);
	people::ratchet(h, (720.0, 400.0), 1.2, true);
	flame_gun(h);
	h.line((680.0, 505.0), (642.0, 508.0), &ink());
	for x in [880.0, 940.0] {
		flame(h, (x, 640.0), -1.57, 70.0, 20.0, if x > 900.0 { PURPLE } else { RED });
	}
	shapes::bolt(h, (880.0, 250.0), 26.0);
	shapes::star(h, (940.0, 380.0), 20.0, ORANGE);
	let t1 = thanks(h, &FIRE);
	vec![t0, t1]
}

// ---------------------------------------------------------------------------------------------------------------- Smoe05

fn paint_bucket(h: &mut Hand, c: P) {
	h.curve(&[(c.0 - 62.0, c.1 - 14.0), (c.0, c.1 - 80.0), (c.0 + 62.0, c.1 - 14.0)], &pn(BLACK, 4.0));
	solid(h, &[(c.0 - 64.0, c.1), (c.0 + 64.0, c.1), (c.0 + 48.0, c.1 + 100.0), (c.0 - 48.0, c.1 + 100.0)], [0.72, 0.74, 0.82]);
	h.colour_in(&ring((c.0, c.1), 62.0, 14.0, 12), PINK, false);
	h.ellipse((c.0, c.1), 66.0, 16.0, &ink());
	for (dx, l) in [(-40.0, 50.0), (-8.0, 80.0), (30.0, 38.0)] {
		h.line((c.0 + dx, c.1 + 10.0), (c.0 + dx + 2.0, c.1 + 10.0 + l), &pn(PINK, 9.0));
		h.dot((c.0 + dx + 2.0, c.1 + 12.0 + l), 7.0, &pn(PINK, 8.0));
	}
}

fn roller(h: &mut Hand, hand: P) {
	h.line(hand, (710.0, 428.0), &pn(BROWN, 9.0));
	h.path(&[(622.0, 388.0), (608.0, 418.0), (710.0, 428.0)], &pn(BLACK, 4.0));
	let head = rect(628.0, 370.0, 790.0, 406.0);
	solid(h, &head, PINK);
	h.ellipse((790.0, 388.0), 8.0, 18.0, &ink());
	for x in [650.0, 710.0, 765.0] {
		let l = h.rng.range(30.0, 66.0);
		h.line((x, 406.0), (x + 3.0, 406.0 + l), &pn(PINK, 8.0));
	}
}

pub fn smoe05(h: &mut Hand) -> Vec<String> {
	let t0 = title(h, "Smoe05", &RAINBOW);
	people::ship(h, (700.0, 300.0), 1.35);
	// The fresh stripe on the hull, still wet.
	h.colour_in(&[(600.0, 322.0), (820.0, 318.0), (800.0, 350.0), (620.0, 352.0)], PINK, true);
	people::ratchet(h, (260.0, 425.0), 1.3, false);
	roller(h, (346.0, 529.0));
	paint_bucket(h, (880.0, 540.0));
	ground(h, 645.0, GREEN);
	h.colour_in(&ring((720.0, 640.0), 90.0, 12.0, 12), PINK, false);
	for (x, y, r, c) in [(120.0, 210.0, 30.0, RED), (450.0, 190.0, 24.0, BLUE), (950.0, 250.0, 34.0, GREEN), (90.0, 580.0, 26.0, YELLOW), (560.0, 600.0, 22.0, ORANGE), (850.0, 400.0, 20.0, PURPLE), (230.0, 150.0, 18.0, PINK)] {
		splat(h, (x, y), r, c);
	}
	shapes::star(h, (450.0, 330.0), 22.0, YELLOW);
	let t1 = thanks(h, &RAINBOW);
	vec![t0, t1]
}

// ---------------------------------------------------------------------------------------------------------------- eardrummer

fn drum(h: &mut Hand, c: P, rx: f32, hgt: f32, col: C) {
	let body = [(c.0 - rx, c.1), (c.0 + rx, c.1), (c.0 + rx, c.1 + hgt), (c.0 - rx, c.1 + hgt)];
	h.colour_in(&body, col, false);
	h.line(body[0], body[3], &ink());
	h.line(body[1], body[2], &ink());
	h.arc((c.0, c.1 + hgt), rx, rx * 0.28, 0.0, 3.14, &ink());
	for k in 0..5 {
		let x = c.0 - rx + (k as f32 + 0.5) / 5.0 * rx * 2.0;
		h.line((x, c.1 + 6.0), (x + 12.0, c.1 + hgt - 6.0), &pn(YELLOW, 5.0));
	}
	h.colour_in(&ring(c, rx, rx * 0.28, 12), [0.96, 0.94, 0.85], false);
	h.ellipse(c, rx, rx * 0.28, &ink());
}

fn cymbal(h: &mut Hand, c: P, rx: f32, foot: f32) {
	h.line((c.0, c.1 + 6.0), (c.0, foot), &pn(BLACK, 5.0));
	h.line((c.0, foot), (c.0 - 34.0, foot + 6.0), &pn(BLACK, 4.0));
	h.line((c.0, foot), (c.0 + 34.0, foot + 6.0), &pn(BLACK, 4.0));
	h.colour_in(&ring(c, rx, 13.0, 10), YELLOW, true);
	h.ellipse(c, rx, 14.0, &pn(ORANGE, 5.5));
	h.dot((c.0, c.1 - 5.0), 6.0, &pn(ORANGE, 6.0));
}

fn stick(h: &mut Hand, a: P, b: P) {
	h.line(a, b, &pn(BROWN, 7.0));
	h.dot(b, 6.0, &pn(BROWN, 8.0));
}

pub fn eardrummer(h: &mut Hand) -> Vec<String> {
	let t0 = title(h, "eardrummer", &[BLUE, RED, GREEN, ORANGE, PURPLE]);
	ground(h, 640.0, BROWN);
	// The kit behind: cymbals, snare and floor tom; Clank sits in front of the bass drum.
	cymbal(h, (255.0, 350.0), 66.0, 620.0);
	cymbal(h, (795.0, 345.0), 74.0, 620.0);
	drum(h, (370.0, 490.0), 68.0, 62.0, ORANGE);
	h.line((370.0, 560.0), (340.0, 625.0), &pn(BLACK, 4.0));
	h.line((370.0, 560.0), (404.0, 625.0), &pn(BLACK, 4.0));
	drum(h, (680.0, 500.0), 78.0, 70.0, PURPLE);
	h.line((620.0, 570.0), (606.0, 628.0), &pn(BLACK, 4.0));
	h.line((740.0, 570.0), (754.0, 628.0), &pn(BLACK, 4.0));
	people::clank(h, (500.0, 270.0), 1.3);
	stick(h, (432.0, 361.0), (366.0, 482.0));
	stick(h, (570.0, 301.0), (735.0, 340.0));
	// The big bass drum with a star on its skin.
	h.colour_in(&ring((520.0, 545.0), 108.0, 92.0, 14), LBLUE, true);
	h.ellipse((520.0, 545.0), 116.0, 98.0, &pn(RED, 9.0));
	h.ellipse((520.0, 545.0), 100.0, 84.0, &ink().width(3.4));
	shapes::star(h, (520.0, 545.0), 46.0, RED);
	people::ratchet(h, (115.0, 440.0), 0.95, true);
	h.arc((115.0, 340.0), 30.0, 12.0, 3.4, 6.0, &pn(YELLOW, 5.0));
	for (p, s, c, b) in [((70.0, 235.0), 1.1, RED, true), ((250.0, 245.0), 0.9, BLUE, false), ((640.0, 205.0), 1.2, PURPLE, true), ((880.0, 215.0), 1.0, GREEN, false), ((930.0, 450.0), 1.2, ORANGE, true), ((930.0, 590.0), 0.9, RED, false), ((450.0, 170.0), 0.8, PINK, false)] {
		note(h, p, s, c, b);
	}
	let t1 = thanks(h, &[BLUE, RED, GREEN, ORANGE, PURPLE]);
	vec![t0, t1]
}

// ---------------------------------------------------------------------------------------------------------------- Chris

fn monitor(h: &mut Hand) {
	let bez = [0.8, 0.82, 0.88];
	let (x0, y0, x1, y1) = (400.0, 290.0, 900.0, 515.0);
	h.colour_in(&rect(x0 - 22.0, y0 - 20.0, x1 + 22.0, y0), bez, false);
	h.colour_in(&rect(x0 - 22.0, y1, x1 + 22.0, y1 + 24.0), bez, false);
	h.colour_in(&rect(x0 - 22.0, y0, x0, y1), bez, false);
	h.colour_in(&rect(x1, y0, x1 + 22.0, y1), bez, false);
	h.outline(&rect(x0 - 22.0, y0 - 20.0, x1 + 22.0, y1 + 24.0), &ink().width(5.5));
	h.outline(&rect(x0, y0, x1, y1), &pn(BLUE, 4.0));
	// A bit of game on the screen: sky scribble, a planet and stars.
	h.hatch(&rect(x0 + 6.0, y0 + 6.0, x1 - 6.0, y1 - 6.0), 0.5, 26.0, &Pen::fill(LBLUE).strength(0.3));
	shapes::planet(h, (480.0, 420.0), 38.0, GREEN);
	shapes::star(h, (590.0, 350.0), 14.0, ORANGE);
	shapes::star(h, (840.0, 470.0), 14.0, PINK);
	h.line((620.0, 515.0), (600.0, 565.0), &pn(BLACK, 8.0));
	h.line((680.0, 515.0), (700.0, 565.0), &pn(BLACK, 8.0));
	solid(h, &[(560.0, 565.0), (740.0, 565.0), (770.0, 585.0), (530.0, 585.0)], bez);
	h.dot((850.0, 304.0), 4.0, &pn(GREEN, 8.0));
}

fn keyboard(h: &mut Hand) {
	let k = [(380.0, 604.0), (760.0, 598.0), (772.0, 650.0), (368.0, 652.0)];
	solid(h, &k, [0.86, 0.88, 0.94]);
	for row in 0..3 {
		for col in 0..9 {
			let (x, y) = (400.0 + col as f32 * 40.0 - row as f32 * 3.0, 614.0 + row as f32 * 12.0);
			h.line((x, y), (x + 24.0, y), &pn(BLUE, 4.0));
		}
	}
	let mouse = ring((850.0, 618.0), 24.0, 32.0, 10);
	h.colour_in(&mouse, [0.86, 0.88, 0.94], false);
	h.ellipse((850.0, 618.0), 26.0, 34.0, &ink());
	h.line((850.0, 586.0), (850.0, 612.0), &pn(BLACK, 3.5));
	h.curve(&[(850.0, 584.0), (840.0, 570.0), (790.0, 560.0), (745.0, 575.0)], &pn(BLACK, 3.5));
}

pub fn chris(h: &mut Hand) -> Vec<String> {
	let pal = [BLUE, RED, GREEN, PURPLE, ORANGE];
	let t0 = title(h, "Chris", &pal);
	monitor(h);
	keyboard(h);
	people::ratchet(h, (775.0, 215.0), 1.2, true);
	// Pop lines where he comes out of the screen.
	for (a, b) in [((630.0, 285.0), (590.0, 262.0)), ((880.0, 285.0), (925.0, 265.0)), ((620.0, 330.0), (572.0, 332.0)), ((870.0, 350.0), (925.0, 350.0))] {
		h.line(a, b, &pn(ORANGE, 6.0));
	}
	people::kid(h, (190.0, 385.0), 1.15, GREEN, BROWN, (60.0, 225.0), (320.0, 225.0));
	let w = "WOW!";
	let t2 = font::text(h, w, 60.0, 135.0, 62.0, &[RED, ORANGE, BLUE, GREEN]);
	for (p, r, c) in [((330.0, 170.0), 14.0, YELLOW), ((60.0, 400.0), 16.0, PINK), ((330.0, 420.0), 12.0, ORANGE)] {
		shapes::star(h, p, r, c);
	}
	shapes::heart(h, (310.0, 290.0), 20.0);
	let t1 = thanks(h, &pal);
	vec![t0, t2, t1]
}

// ---------------------------------------------------------------------------------------------------------------- Ryan Palmer

fn game_box(h: &mut Hand, x: f32, top: f32, n: usize, band: C) {
	let (w, bottom) = (150.0, 640.0);
	let body = [(x, top), (x + w, top + 3.0), (x + w - 3.0, bottom), (x + 3.0, bottom - 2.0)];
	h.outline(&body, &ink().width(5.5));
	h.colour_in(&rect(x + 4.0, top + 4.0, x + w - 4.0, top + 44.0), band, true);
	let s = format!("{}", n + 1);
	h.line((x + 4.0, top + 46.0), (x + w - 4.0, top + 46.0), &ink().width(3.5));
	let nx = x + w * 0.5 - font::width(&s, 84.0) * 0.5;
	font::text(h, &s, nx, top + 58.0, 84.0, &[RED, BLUE, GREEN, PURPLE][n..n + 1]);
	let c = (x + w * 0.5, top + 172.0);
	match n {
		0 => shapes::bolt(h, c, 24.0),
		1 => shapes::wrench(h, (c.0 - 28.0, c.1 + 14.0), (c.0 + 30.0, c.1 - 14.0), 8.0, SILVER),
		2 => shapes::planet(h, c, 20.0, PINK),
		_ => shapes::star(h, c, 26.0, ORANGE),
	}
}

pub fn ryan_palmer(h: &mut Hand) -> Vec<String> {
	let pal = [BLUE, ORANGE, RED, GREEN, PURPLE];
	let t0 = title(h, "Ryan Palmer", &pal);
	let bands = [BLUE, ORANGE, PURPLE, RED];
	for (i, band) in bands.iter().enumerate() {
		game_box(h, 350.0 + i as f32 * 164.0, 470.0 - i as f32 * 42.0, i, *band);
	}
	ground(h, 645.0, BROWN);
	people::ratchet(h, (150.0, 455.0), 1.2, true);
	// A fat arrow from Ratchet to the boxes and a bubble: all four!
	let shaft = rect(268.0, 540.0, 312.0, 568.0);
	h.colour_in(&shaft, YELLOW, false);
	h.outline(&shaft, &pn(ORANGE, 5.0));
	let head = [(312.0, 520.0), (346.0, 554.0), (312.0, 588.0)];
	h.colour_in(&head, YELLOW, false);
	h.outline(&head, &pn(ORANGE, 5.0));
	let bubble = h.closed(&[(40.0, 220.0), (140.0, 170.0), (260.0, 175.0), (330.0, 230.0), (300.0, 290.0), (190.0, 305.0), (80.0, 285.0)], &pn(BLUE, 5.5));
	let _ = bubble;
	h.path(&[(160.0, 300.0), (150.0, 345.0), (190.0, 304.0)], &pn(BLUE, 5.0));
	let t2 = font::text(h, "ALL 4!", 70.0, 205.0, 66.0, &[RED, BLUE, GREEN, ORANGE, PURPLE]);
	for (p, r, c) in [((620.0, 190.0), 24.0, YELLOW), ((780.0, 170.0), 18.0, ORANGE), ((930.0, 200.0), 26.0, PINK), ((890.0, 320.0), 16.0, YELLOW), ((500.0, 300.0), 16.0, PINK)] {
		shapes::star(h, p, r, c);
	}
	let t1 = thanks(h, &pal);
	vec![t0, t2, t1]
}

// ---------------------------------------------------------------------------------------------------------------- fleece

/// The woolly body: a ring of curly loops with swirls inside.
fn wool(h: &mut Hand, c: P) {
	for i in 0..11 {
		let a = i as f32 / 11.0 * TAU;
		let r = h.rng.range(52.0, 66.0);
		h.ellipse((c.0 + cos(a) * 150.0, c.1 + sin(a) * 92.0), r, r * 0.9, &pn(LBLUE, 5.5));
	}
	h.scribble_blob(c, 150.0, 85.0, 7, &Pen::ink(LBLUE).width(4.0).strength(0.7));
	h.ellipse(c, 175.0, 110.0, &pn(BLUE, 4.5));
}

fn helmet(h: &mut Hand, c: P) {
	let dome: Vec<P> = (0..=8).map(|i| (c.0 + cos(3.1416 + i as f32 / 8.0 * 3.1416) * 76.0, c.1 + sin(3.1416 + i as f32 / 8.0 * 3.1416) * 62.0)).collect();
	h.colour_in(&dome, SILVER, true);
	h.outline(&dome, &ink().width(5.0));
	let crest = [(c.0 - 60.0, c.1 - 38.0), (c.0 - 40.0, c.1 - 98.0), (c.0, c.1 - 118.0), (c.0 + 40.0, c.1 - 98.0), (c.0 + 60.0, c.1 - 38.0)];
	h.colour_in(&crest, RED, true);
	h.outline(&crest, &pn(RED, 5.0));
	for sx in [-1.0f32, 1.0] {
		solid(h, &[(c.0 + sx * 66.0, c.1 - 6.0), (c.0 + sx * 84.0, c.1 + 2.0), (c.0 + sx * 70.0, c.1 + 66.0), (c.0 + sx * 54.0, c.1 + 52.0)], SILVER);
	}
	h.line((c.0 - 44.0, c.1 - 10.0), (c.0 + 44.0, c.1 - 10.0), &pn(BLACK, 5.0));
	h.line((c.0, c.1 - 10.0), (c.0, c.1 + 30.0), &pn(BLACK, 5.0));
}

fn sheep_head(h: &mut Hand, c: P) {
	for sx in [-1.0f32, 1.0] {
		let e = (c.0 + sx * 84.0, c.1 + 10.0);
		h.colour_in(&ring(e, 28.0, 14.0, 8), TAN, false);
		h.ellipse(e, 30.0, 16.0, &ink());
	}
	h.colour_in(&ring(c, 66.0, 76.0, 12), TAN, true);
	h.ellipse(c, 72.0, 80.0, &ink());
	h.colour_in(&ring((c.0, c.1 + 40.0), 34.0, 22.0, 8), PINK, false);
	for sx in [-1.0f32, 1.0] {
		h.dot((c.0 + sx * 28.0, c.1 + 6.0), 6.5, &ink());
		h.dot((c.0 + sx * 10.0, c.1 + 40.0), 3.5, &ink());
	}
	h.arc((c.0, c.1 + 52.0), 22.0, 10.0, 0.2, 2.9, &ink().width(3.8));
	helmet(h, (c.0, c.1 - 24.0));
}

pub fn fleece(h: &mut Hand) -> Vec<String> {
	let pal = [BLUE, LBLUE, PURPLE, BLUE];
	let t0 = title(h, "fleece", &pal);
	shapes::sun(h, (930.0, 200.0), 44.0);
	shapes::cloud(h, (700.0, 180.0), 170.0);
	ground(h, 625.0, GREEN);
	shapes::grass(h, 30.0, 995.0, 628.0);
	let body = (730.0, 420.0);
	wool(h, body);
	for x in [625.0, 690.0, 780.0, 845.0] {
		h.line((x, 515.0), (x + 2.0, 610.0), &pn(BLACK, 7.0));
		h.path(&[(x - 10.0, 612.0), (x + 12.0, 612.0)], &pn(BLACK, 8.0));
	}
	h.curve(&[(900.0, 420.0), (930.0, 400.0), (920.0, 370.0)], &pn(LBLUE, 9.0));
	sheep_head(h, (440.0, 410.0));
	people::ratchet(h, (150.0, 450.0), 1.15, false);
	let t2 = font::text(h, "baa!", 360.0, 150.0, 60.0, &[PURPLE, BLUE, PURPLE, RED]);
	shapes::heart(h, (270.0, 240.0), 22.0);
	let t1 = thanks(h, &pal);
	vec![t0, t2, t1]
}

// ---------------------------------------------------------------------------------------------------------------- Justintime4u2bu

fn alarm_clock(h: &mut Hand, c: P, r: f32) {
	for sx in [-1.0f32, 1.0] {
		let bell = (c.0 + sx * r * 0.72, c.1 - r * 0.86);
		h.colour_in(&ring(bell, r * 0.3, r * 0.22, 10), RED, true);
		h.ellipse(bell, r * 0.33, r * 0.25, &ink());
		h.line((c.0 + sx * r * 0.55, c.1 - r * 0.62), (c.0 + sx * r * 0.45, c.1 - r * 0.9), &pn(BLACK, 5.0));
		h.line((bell.0 + sx * r * 0.42, bell.1 - r * 0.02), (bell.0 + sx * r * 0.56, bell.1 - r * 0.12), &pn(ORANGE, 5.0));
		h.line((bell.0 + sx * r * 0.42, bell.1 + r * 0.1), (bell.0 + sx * r * 0.58, bell.1 + r * 0.14), &pn(ORANGE, 5.0));
		h.line((c.0 + sx * r * 0.7, c.1 + r * 0.8), (c.0 + sx * r * 0.98, c.1 + r * 1.15), &pn(BLACK, 7.0));
	}
	h.line((c.0, c.1 - r * 1.05), (c.0 - 6.0, c.1 - r * 1.2), &pn(BLACK, 6.0));
	h.line((c.0 - r * 0.2, c.1 - r * 1.2), (c.0 + r * 0.2, c.1 - r * 1.2), &pn(BLACK, 6.0));
	h.colour_in(&ring(c, r, r, 14), YELLOW, true);
	h.ellipse(c, r * 1.05, r * 1.03, &pn(RED, 9.0));
	h.ellipse(c, r * 0.84, r * 0.82, &ink().width(3.5));
	for k in 0..12 {
		let a = k as f32 / 12.0 * TAU;
		let (r0, r1) = (r * 0.7, if k % 3 == 0 { r * 0.56 } else { r * 0.62 });
		h.line((c.0 + cos(a) * r0, c.1 + sin(a) * r0), (c.0 + cos(a) * r1, c.1 + sin(a) * r1), &pn(BLACK, if k % 3 == 0 { 5.5 } else { 3.6 }));
	}
	// Five to twelve: just in time.
	h.line(c, (c.0 - r * 0.06, c.1 - r * 0.52), &pn(BLACK, 7.0));
	h.line(c, (c.0 + r * 0.36, c.1 - r * 0.46), &pn(RED, 5.0));
	h.dot(c, 7.0, &ink());
}

/// An arm round the heart: from the shoulder, round its edge, with a spiky hand.
fn hug_arm(h: &mut Hand, from: P, via: P, to: P) {
	h.curve(&[from, via, to], &ink().width(5.0));
	for k in -1..=1 {
		let a = k as f32 * 0.5;
		h.spike(to, (to.0 + cos(a) * 11.0, to.1 + sin(a) * 11.0 - 8.0), &ink().width(3.6));
	}
}

pub fn justintime4u2bu(h: &mut Hand) -> Vec<String> {
	let pal = [RED, PINK, PURPLE, ORANGE, BLUE];
	let t0 = title(h, "Justintime4u2bu", &pal);
	alarm_clock(h, (190.0, 440.0), 120.0);
	let heart = (640.0, 390.0);
	shapes::heart(h, heart, 150.0);
	shapes::heart(h, (640.0, 372.0), 105.0);
	people::ratchet(h, (470.0, 360.0), 0.95, false);
	people::clank(h, (830.0, 360.0), 1.05);
	hug_arm(h, (497.0, 413.0), (540.0, 320.0), (578.0, 290.0));
	hug_arm(h, (800.0, 405.0), (780.0, 480.0), (738.0, 500.0));
	for (p, s) in [((330.0, 200.0), 18.0), ((420.0, 600.0), 22.0), ((900.0, 560.0), 26.0), ((960.0, 230.0), 20.0), ((640.0, 190.0), 24.0), ((70.0, 640.0), 16.0), ((560.0, 640.0), 18.0), ((300.0, 590.0), 14.0), ((760.0, 580.0), 16.0)] {
		shapes::heart(h, p, s);
	}
	for (p, r, c) in [((400.0, 280.0), 14.0, YELLOW), ((880.0, 140.0), 16.0, ORANGE), ((60.0, 215.0), 16.0, YELLOW)] {
		shapes::star(h, p, r, c);
	}
	let t1 = thanks(h, &pal);
	vec![t0, t1]
}
