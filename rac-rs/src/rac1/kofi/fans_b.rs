//! Ko-fi drawings: the sheets of Ga, THUND3R_WOLF, MK, Ratchetenjoyer, Tonyswan and Luke4600 (user 2026-11-04: "we need drawing for each one").
//! Same felt-marker hand as the other sheets (scenes.rs, achieve.rs).

use super::canvas::{P, Pen};
use super::font;
use super::hand::Hand;
use super::people;
use super::rng::{cos, sin};
use super::shapes::{self, BLACK, BLUE, BROWN, GREEN, LBLUE, ORANGE, PINK, PURPLE, RED, YELLOW, ink, ring};

const GOLD: [f32; 3] = [1.0, 0.8, 0.1];
const RAINBOW: [[f32; 3]; 6] = [RED, ORANGE, GREEN, BLUE, PURPLE, [0.9, 0.3, 0.6]];
const WARM: [[f32; 3]; 4] = [RED, ORANGE, PURPLE, BLUE];
const GREY: [f32; 3] = [0.5, 0.58, 0.72];
const TAU: f32 = 6.283_185_5;

/// The supporter's name big at the top, shrunk to fit the page.
fn name_line(h: &mut Hand, n: &str, pal: &[[f32; 3]]) -> String {
	let mut hh = 96.0;
	let w = font::width(n, hh);
	if w > 960.0 {
		hh *= 960.0 / w;
	}
	font::text(h, n, 512.0 - font::width(n, hh) * 0.5, 30.0 + (96.0 - hh) * 0.3, hh, pal)
}

fn thanks(h: &mut Hand, pal: &[[f32; 3]]) -> String {
	let b = "THANK YOU!";
	font::text(h, b, 512.0 - font::width(b, 70.0) * 0.5, 672.0, 70.0, pal)
}

fn centred(h: &mut Hand, s: &str, y: f32, height: f32, cx: f32, pal: &[[f32; 3]]) -> String {
	font::text(h, s, cx - font::width(s, height) * 0.5, y, height, pal)
}

fn ground(h: &mut Hand, y: f32, x0: f32, x1: f32) {
	let mut pts = Vec::new();
	let mut x = x0;
	while x <= x1 {
		pts.push((x, y + h.rng.s() * 6.0));
		x += 90.0;
	}
	h.curve(&pts, &Pen::ink(GREEN).width(6.0).wobble(3.0));
	shapes::grass(h, x0 + 10.0, x1 - 10.0, y + 4.0);
}

fn gear(h: &mut Hand, c: P, r: f32, col: [f32; 3]) {
	h.colour_in(&ring(c, r * 0.85, r * 0.85, 12), col, false);
	let teeth: Vec<P> = (0..16).map(|i| {
		let a = i as f32 / 16.0 * TAU;
		let k = if i % 2 == 0 { 1.0 } else { 0.8 };
		(c.0 + cos(a) * r * k, c.1 + sin(a) * r * k)
	}).collect();
	h.outline(&teeth, &ink().width(4.0));
	h.ellipse(c, r * 0.28, r * 0.28, &ink().width(3.6));
}

fn confetti(h: &mut Hand, n: usize, x0: f32, x1: f32, y0: f32, y1: f32) {
	let cols = [RED, BLUE, ORANGE, GREEN, PURPLE, PINK, YELLOW];
	for i in 0..n {
		let p = (h.rng.range(x0, x1), h.rng.range(y0, y1));
		let a = h.rng.range(0.0, TAU);
		let l = h.rng.range(8.0, 15.0);
		h.line(p, (p.0 + cos(a) * l, p.1 + sin(a) * l), &Pen::ink(cols[i % cols.len()]).width(8.0).wobble(0.6));
	}
}

fn sparkle(h: &mut Hand, c: P, r: f32, col: [f32; 3]) {
	let pen = Pen::ink(col).width(5.0);
	h.line((c.0 - r, c.1), (c.0 + r, c.1), &pen);
	h.line((c.0, c.1 - r), (c.0, c.1 + r), &pen);
}

/// A fist with the thumb up, at `p`.
fn thumb_up(h: &mut Hand, p: P, s: f32) {
	let fist = [(p.0 - 22.0 * s, p.1 - 6.0 * s), (p.0 + 20.0 * s, p.1 - 10.0 * s), (p.0 + 26.0 * s, p.1 + 24.0 * s), (p.0 - 18.0 * s, p.1 + 28.0 * s)];
	h.colour_in(&fist, GREY, false);
	h.outline(&fist, &ink());
	let thumb = [(p.0 - 16.0 * s, p.1 - 6.0 * s), (p.0 - 8.0 * s, p.1 - 44.0 * s), (p.0 + 8.0 * s, p.1 - 44.0 * s), (p.0 + 10.0 * s, p.1 - 8.0 * s)];
	h.colour_in(&thumb, GREY, false);
	h.outline(&thumb, &ink());
	for k in 1..4 {
		let y = p.1 + (k as f32 * 7.0 - 8.0) * s;
		h.line((p.0 + 4.0 * s, y), (p.0 + 22.0 * s, y - 1.0 * s), &ink().width(3.2));
	}
}

fn medal(h: &mut Hand, c: P, r: f32) {
	let l = [(c.0 - r * 0.7, c.1 - r * 1.9), (c.0 - r * 0.2, c.1 - r * 1.9), (c.0 + r * 0.15, c.1 - r * 0.7), (c.0 - r * 0.45, c.1 - r * 0.75)];
	let rr = [(c.0 + r * 0.7, c.1 - r * 1.9), (c.0 + r * 0.2, c.1 - r * 1.9), (c.0 - r * 0.15, c.1 - r * 0.7), (c.0 + r * 0.45, c.1 - r * 0.75)];
	h.colour_in(&l, RED, false);
	h.outline(&l, &ink());
	h.colour_in(&rr, BLUE, false);
	h.outline(&rr, &ink());
	h.colour_in(&ring(c, r * 0.92, r * 0.92, 12), GOLD, true);
	h.ellipse(c, r, r * 0.97, &Pen::ink(ORANGE).width(7.0));
	h.ellipse(c, r * 0.78, r * 0.76, &Pen::ink(ORANGE).width(3.5));
	shapes::star(h, (c.0, c.1 + r * 0.05), r * 0.5, RED);
}

pub fn ga(h: &mut Hand) -> Vec<String> {
	let mut t = vec![name_line(h, "Ga", &RAINBOW)];
	shapes::sun(h, (900.0, 235.0), 38.0);
	shapes::cloud(h, (130.0, 210.0), 150.0);
	t.push(centred(h, "GREAT JOB!", 160.0, 58.0, 520.0, &WARM));
	ground(h, 620.0, 20.0, 1004.0);
	let c = (370.0, 385.0);
	people::clank(h, c, 1.9);
	thumb_up(h, (c.0 + 54.0 * 1.9 + 14.0, c.1 + 24.0 * 1.9 - 6.0), 1.5);
	medal(h, (690.0, 440.0), 78.0);
	people::ratchet(h, (880.0, 470.0), 0.85, true);
	for (x, y, r, col) in [(560.0, 280.0, 18.0, YELLOW), (250.0, 250.0, 14.0, ORANGE), (640.0, 250.0, 12.0, PINK)] {
		shapes::star(h, (x, y), r, col);
	}
	sparkle(h, (800.0, 330.0), 16.0, GOLD);
	sparkle(h, (560.0, 600.0), 14.0, GOLD);
	shapes::heart(h, (90.0, 420.0), 26.0);
	t.push(thanks(h, &WARM));
	t
}

fn moon(h: &mut Hand, c: P, r: f32) {
	h.colour_in(&ring(c, r * 0.92, r * 0.92, 14), [1.0, 0.94, 0.6], true);
	h.ellipse(c, r, r * 0.97, &Pen::ink(ORANGE).width(6.0));
	for (dx, dy, k) in [(-0.35, -0.3, 0.2), (0.3, 0.1, 0.26), (-0.1, 0.45, 0.14)] {
		h.ellipse((c.0 + dx * r, c.1 + dy * r), r * k, r * k * 0.9, &Pen::ink([0.8, 0.65, 0.3]).width(3.6));
	}
}

fn lightning(h: &mut Hand, a: P, s: f32) {
	let p: Vec<P> = [(0.0, 0.0), (-26.0, 50.0), (-2.0, 56.0), (-34.0, 120.0), (20.0, 52.0), (-4.0, 46.0), (22.0, 0.0)].iter().map(|q| (a.0 + q.0 * s, a.1 + q.1 * s)).collect();
	h.colour_in(&[p[0], p[1], p[2], p[5], p[6]], YELLOW, false);
	h.colour_in(&[p[2], p[3], p[4], p[5]], YELLOW, false);
	h.outline(&p, &Pen::ink(ORANGE).width(4.5));
}

fn wolf(h: &mut Hand, o: P) {
	let at = |x: f32, y: f32| -> P { (o.0 + x, o.1 + y) };
	let fur = [0.52, 0.56, 0.72];
	let dark = Pen::ink(BLACK).width(4.8);
	let tail = [at(-85.0, -75.0), at(-140.0, -112.0), at(-188.0, -92.0), at(-176.0, -42.0), at(-130.0, -58.0), at(-80.0, -34.0)];
	h.colour_in(&tail, fur, false);
	h.outline(&tail, &dark);
	for (x, y) in [(-60.0, 0.0), (-30.0, 6.0), (40.0, 6.0), (70.0, 0.0)] {
		let l = [at(x - 9.0, y - 40.0), at(x + 9.0, y - 40.0), at(x + 8.0, y + 2.0), at(x - 8.0, y + 2.0)];
		h.colour_in(&l, fur, false);
		h.outline(&l, &dark);
	}
	h.colour_in(&ring(at(0.0, -62.0), 88.0, 38.0, 12), fur, true);
	h.ellipse(at(0.0, -62.0), 92.0, 42.0, &dark);
	let neck = [at(30.0, -92.0), at(100.0, -70.0), at(138.0, -165.0), at(80.0, -200.0)];
	h.colour_in(&neck, fur, false);
	h.outline(&neck, &dark);
	let head = [at(66.0, -185.0), at(120.0, -205.0), at(214.0, -290.0), at(186.0, -318.0), at(98.0, -258.0), at(56.0, -215.0)];
	h.colour_in(&head, fur, false);
	h.outline(&head, &dark);
	let ear = [at(70.0, -218.0), at(52.0, -270.0), at(104.0, -244.0)];
	h.colour_in(&ear, fur, false);
	h.outline(&ear, &dark);
	h.line(at(190.0, -296.0), at(176.0, -266.0), &ink().width(3.6));
	h.dot(at(108.0, -232.0), 4.2, &ink());
	h.colour_in(&[at(30.0, -86.0), at(70.0, -78.0), at(60.0, -50.0), at(20.0, -52.0)], [0.9, 0.9, 0.97], false);
	for k in 0..4 {
		let p = at(230.0 + k as f32 * 18.0, -318.0 - k as f32 * 6.0);
		h.arc(p, 10.0 + k as f32 * 4.0, 18.0, -1.2, 1.2, &Pen::ink(PURPLE).width(4.0));
	}
}

pub fn thund3r_wolf(h: &mut Hand) -> Vec<String> {
	let mut t = vec![name_line(h, "THUND3R_WOLF", &[PURPLE, BLUE, RED, ORANGE])];
	moon(h, (700.0, 290.0), 105.0);
	for (x, y, r) in [(120.0, 200.0, 13.0), (330.0, 180.0, 16.0), (880.0, 170.0, 12.0), (900.0, 400.0, 14.0)] {
		shapes::star(h, (x, y), r, YELLOW);
	}
	lightning(h, (180.0, 170.0), 1.4);
	lightning(h, (555.0, 150.0), 1.0);
	h.curve(&[(20.0, 600.0), (150.0, 520.0), (330.0, 515.0), (520.0, 560.0), (760.0, 590.0), (1004.0, 570.0)], &Pen::ink(GREEN).width(7.0).wobble(3.0));
	shapes::grass(h, 40.0, 980.0, 600.0);
	h.hatch(&[(40.0, 590.0), (960.0, 590.0), (960.0, 625.0), (40.0, 625.0)], 0.1, 24.0, &Pen::fill(GREEN).width(8.0).strength(0.35));
	wolf(h, (270.0, 520.0));
	people::ratchet(h, (820.0, 440.0), 0.95, true);
	shapes::bolt(h, (600.0, 560.0), 22.0);
	shapes::bolt(h, (460.0, 585.0), 18.0);
	t.push(thanks(h, &[PURPLE, RED, BLUE, ORANGE]));
	t
}

pub fn mk(h: &mut Hand) -> Vec<String> {
	let mut t = vec![name_line(h, "MK", &RAINBOW)];
	shapes::sun(h, (930.0, 160.0), 34.0);
	shapes::cloud(h, (380.0, 190.0), 140.0);
	ground(h, 630.0, 20.0, 1004.0);
	h.line((650.0, 440.0), (630.0, 628.0), &ink().width(5.5));
	h.line((760.0, 440.0), (790.0, 628.0), &ink().width(5.5));
	h.line((610.0, 632.0), (670.0, 632.0), &ink().width(5.5));
	h.line((760.0, 632.0), (820.0, 632.0), &ink().width(5.5));
	people::ship(h, (700.0, 370.0), 1.3);
	people::ratchet(h, (170.0, 440.0), 1.1, true);
	people::clank(h, (400.0, 495.0), 1.1);
	// Clank holds a screwdriver out to Ratchet, who has just fixed the wing.
	let hand = (400.0 + 54.0 * 1.1, 495.0 + 24.0 * 1.1);
	h.line(hand, (hand.0 + 50.0, hand.1 - 30.0), &Pen::ink(RED).width(9.0));
	h.line((hand.0 + 50.0, hand.1 - 30.0), (hand.0 + 90.0, hand.1 - 54.0), &ink().width(4.0));
	shapes::wrench(h, (250.0, 580.0), (520.0, 604.0), 22.0, [0.75, 0.78, 0.9]);
	gear(h, (560.0, 215.0), 36.0, [0.7, 0.72, 0.8]);
	gear(h, (890.0, 330.0), 28.0, [0.8, 0.6, 0.3]);
	gear(h, (480.0, 380.0), 22.0, [0.7, 0.72, 0.8]);
	shapes::bolt(h, (840.0, 215.0), 24.0);
	shapes::bolt(h, (940.0, 480.0), 22.0);
	shapes::bolt(h, (330.0, 290.0), 20.0);
	for (a, b) in [((420.0, 305.0), (445.0, 325.0)), ((430.0, 340.0), (405.0, 345.0)), ((540.0, 440.0), (565.0, 450.0))] {
		h.line(a, b, &Pen::ink(YELLOW).width(6.0));
	}
	sparkle(h, (545.0, 330.0), 14.0, ORANGE);
	t.push(thanks(h, &WARM));
	t
}

pub fn ratchetenjoyer(h: &mut Hand) -> Vec<String> {
	let mut t = vec![name_line(h, "Ratchetenjoyer", &RAINBOW)];
	confetti(h, 46, 30.0, 990.0, 150.0, 640.0);
	let c = (512.0, 330.0);
	h.ellipse((512.0, 610.0), 110.0, 14.0, &Pen::ink([0.5, 0.5, 0.5]).width(5.0));
	for dx in [-60.0, 0.0, 60.0] {
		h.line((512.0 + dx, 585.0), (512.0 + dx * 1.2, 570.0), &Pen::ink(LBLUE).width(5.0));
	}
	people::ratchet(h, c, 1.6, true);
	// A huge open grin and rosy cheeks on top of the muzzle.
	let m = (c.0, c.1 + 22.0 * 1.6);
	let mouth = [(m.0 - 24.0, m.1 - 6.0), (m.0 + 24.0, m.1 - 6.0), (m.0 + 16.0, m.1 + 18.0), (m.0, m.1 + 24.0), (m.0 - 16.0, m.1 + 18.0)];
	h.colour_in(&mouth, RED, false);
	h.outline(&mouth, &ink().width(4.0));
	for sx in [-1.0, 1.0] {
		h.scribble_blob((c.0 + sx * 38.0 * 1.6, c.1 + 8.0 * 1.6), 10.0, 8.0, 2, &Pen::ink(PINK).width(6.0));
	}
	for (x, y, r) in [(110.0, 220.0, 28.0), (240.0, 400.0, 24.0), (860.0, 250.0, 30.0), (770.0, 450.0, 26.0), (150.0, 560.0, 22.0), (920.0, 560.0, 24.0), (330.0, 200.0, 20.0), (700.0, 190.0, 22.0)] {
		shapes::bolt(h, (x, y), r);
	}
	shapes::heart(h, (180.0, 380.0), 24.0);
	shapes::heart(h, (840.0, 380.0), 22.0);
	shapes::star(h, (300.0, 560.0), 18.0, YELLOW);
	shapes::star(h, (730.0, 580.0), 18.0, ORANGE);
	t.push(thanks(h, &RAINBOW));
	t
}

fn swan(h: &mut Hand, o: P) {
	let at = |x: f32, y: f32| -> P { (o.0 + x, o.1 + y) };
	let pen = ink().width(5.5);
	h.closed(&[at(-110.0, 0.0), at(-80.0, -40.0), at(20.0, -50.0), at(100.0, -34.0), at(130.0, -64.0), at(120.0, -10.0), at(60.0, 34.0), at(-50.0, 38.0)], &pen);
	h.curve(&[at(-20.0, -14.0), at(30.0, -36.0), at(80.0, -14.0)], &ink().width(3.6));
	h.curve(&[at(0.0, 4.0), at(50.0, -18.0), at(88.0, 2.0)], &ink().width(3.2));
	// The neck as two curves meeting in the head.
	h.curve(&[at(-86.0, -30.0), at(-120.0, -80.0), at(-86.0, -130.0), at(-96.0, -170.0), at(-112.0, -196.0)], &pen);
	h.curve(&[at(-56.0, -44.0), at(-84.0, -86.0), at(-56.0, -134.0), at(-66.0, -178.0), at(-84.0, -202.0)], &pen);
	let head = h.closed(&[at(-114.0, -198.0), at(-100.0, -218.0), at(-84.0, -208.0), at(-82.0, -198.0)], &pen);
	let _ = head;
	let beak = [at(-114.0, -202.0), at(-148.0, -190.0), at(-112.0, -184.0)];
	h.colour_in(&beak, ORANGE, false);
	h.outline(&beak, &Pen::ink(ORANGE).width(4.5));
	h.dot(at(-102.0, -204.0), 3.6, &ink());
}

fn reed(h: &mut Hand, x: f32, y: f32, hgt: f32, lean: f32) {
	h.line((x, y), (x + lean, y - hgt), &Pen::ink(GREEN).width(5.0));
	h.line((x + lean, y - hgt + 6.0), (x + lean * 1.05, y - hgt - 36.0), &Pen::ink(BROWN).width(11.0));
	h.line((x + lean * 0.6, y - hgt * 0.5), (x + lean * 0.6 + 28.0, y - hgt * 0.5 - 40.0), &Pen::ink(GREEN).width(4.0));
}

pub fn tonyswan(h: &mut Hand) -> Vec<String> {
	let mut t = vec![name_line(h, "Tonyswan", &[BLUE, PURPLE, LBLUE, RED])];
	shapes::sun(h, (900.0, 215.0), 36.0);
	shapes::cloud(h, (610.0, 210.0), 160.0);
	h.ellipse_turns((380.0, 500.0), 340.0, 118.0, 1.0, &Pen::ink(BLUE).width(6.0));
	// Ripples round and under the swan, kept clear of its white body.
	let rip = Pen::ink(LBLUE).width(5.0).wobble(1.8);
	h.arc((380.0, 470.0), 200.0, 54.0, 0.35, 2.8, &rip);
	h.arc((380.0, 470.0), 240.0, 68.0, 0.2, 2.95, &rip);
	for (x0, y0, x1) in [(110.0, 440.0, 220.0), (500.0, 430.0, 600.0), (150.0, 560.0, 330.0), (420.0, 575.0, 580.0), (200.0, 505.0, 270.0), (520.0, 500.0, 620.0)] {
		h.curve(&[(x0, y0), ((x0 + x1) * 0.5, y0 + 8.0), (x1, y0)], &rip);
	}
	swan(h, (400.0, 475.0));
	for (x, y, hg, l) in [(60.0, 470.0, 150.0, -10.0), (90.0, 490.0, 190.0, 12.0), (130.0, 450.0, 120.0, 8.0), (680.0, 520.0, 160.0, 10.0), (710.0, 540.0, 120.0, -8.0)] {
		reed(h, x, y, hg, l);
	}
	ground(h, 580.0, 720.0, 1004.0);
	h.hatch(&[(740.0, 590.0), (990.0, 590.0), (990.0, 640.0), (740.0, 640.0)], 0.1, 24.0, &Pen::fill(GREEN).width(8.0).strength(0.35));
	people::ratchet(h, (850.0, 390.0), 1.1, true);
	shapes::heart(h, (700.0, 340.0), 24.0);
	t.push(thanks(h, &[BLUE, PURPLE, RED, ORANGE]));
	t
}

fn mini_heads(h: &mut Hand, dome: P) {
	let fur = [0.98, 0.52, 0.1];
	let r = (dome.0 - 16.0, dome.1 + 6.0);
	for sx in [-1.0f32, 1.0] {
		let ear = [(r.0 + sx * 8.0, r.1 - 20.0), (r.0 + sx * 22.0, r.1 - 52.0), (r.0 + sx * 22.0, r.1 - 18.0)];
		h.colour_in(&ear, fur, false);
		h.outline(&ear, &ink().width(3.4));
	}
	h.colour_in(&ring(r, 22.0, 20.0, 10), fur, true);
	h.ellipse(r, 25.0, 22.0, &ink().width(3.6));
	h.dot((r.0 - 9.0, r.1 - 3.0), 3.4, &Pen::ink(GREEN).width(6.0));
	h.dot((r.0 + 9.0, r.1 - 3.0), 3.4, &Pen::ink(GREEN).width(6.0));
	h.arc((r.0, r.1 + 5.0), 9.0, 6.0, 0.3, 2.8, &ink().width(3.2));
	let c = (dome.0 + 26.0, dome.1 + 12.0);
	let head = [(c.0, c.1 - 15.0), (c.0 + 15.0, c.1 - 2.0), (c.0 + 12.0, c.1 + 14.0), (c.0 - 12.0, c.1 + 14.0), (c.0 - 15.0, c.1 - 2.0)];
	h.colour_in(&head, GREY, false);
	h.outline(&head, &ink().width(3.4));
	h.dot((c.0 - 5.0, c.1 - 1.0), 2.6, &ink());
	h.dot((c.0 + 5.0, c.1 - 1.0), 2.6, &ink());
	h.line((c.0 + 2.0, c.1 - 15.0), (c.0 + 5.0, c.1 - 30.0), &ink().width(3.0));
}

pub fn luke4600(h: &mut Hand) -> Vec<String> {
	let mut t = vec![name_line(h, "Luke4600", &RAINBOW)];
	shapes::planet(h, (140.0, 240.0), 52.0, ORANGE);
	shapes::planet(h, (870.0, 250.0), 62.0, PURPLE);
	shapes::planet(h, (820.0, 540.0), 38.0, GREEN);
	shapes::planet(h, (170.0, 560.0), 34.0, BLUE);
	for (x, y, r, c) in [(300.0, 210.0, 16.0, YELLOW), (700.0, 190.0, 14.0, ORANGE), (980.0, 400.0, 14.0, PINK), (480.0, 190.0, 12.0, YELLOW), (560.0, 600.0, 16.0, YELLOW), (330.0, 610.0, 13.0, ORANGE), (680.0, 520.0, 12.0, PINK), (960.0, 620.0, 12.0, YELLOW)] {
		shapes::star(h, (x, y), r, c);
	}
	people::ship(h, (420.0, 400.0), 2.0);
	mini_heads(h, (420.0 + 28.0 * 2.0, 400.0 - 26.0 * 2.0));
	for (y0, l) in [(330.0, 120.0), (470.0, 150.0), (520.0, 90.0)] {
		h.line((560.0, y0), (560.0 + l, y0 + 4.0), &Pen::ink(LBLUE).width(5.0));
	}
	sparkle(h, (650.0, 350.0), 14.0, GOLD);
	t.push(thanks(h, &WARM));
	t
}
