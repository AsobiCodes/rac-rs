//! Ko-fi drawings: the three scenes (one per supporter) and the finished sheet: paper grain, a soft shadow in a transparent margin
//! and two strips of tape. `draw(i)` is deterministic: the same pixels on every run and machine.

use super::canvas::{Canvas, P, Pen};
use super::font;
use super::hand::Hand;
use super::people;
use super::rng::{hash2, noise2};
use super::shapes::{self, BLACK, BLUE, BROWN, GREEN, ORANGE, PINK, PURPLE, RED, YELLOW, ink, ring};

pub const SUPPORTERS: [&str; 3] = ["Scrolt", "korixx", "reTokyo"];

/// Drawing area in canvas units, the transparent margin round it (for the shadow and the tape) and the supersampling.
pub const W: usize = 1024;
pub const H: usize = 768;
pub const MARGIN: usize = 32;
const SS: usize = 2;
const PAPER: [f32; 3] = [0.985, 0.985, 0.972];

pub struct Drawing {
	#[allow(dead_code)] // read by the tests
	pub name: &'static str,
	/// Every text written on the sheet, exactly as spelled.
	#[allow(dead_code)] // read by the tests
	pub texts: Vec<String>,
	pub w: usize,
	pub h: usize,
	pub rgba: Vec<[u8; 4]>,
}

const RAINBOW: [[f32; 3]; 6] = [RED, ORANGE, [0.1, 0.65, 0.2], BLUE, PURPLE, [0.9, 0.3, 0.6]];
const WARM: [[f32; 3]; 4] = [RED, ORANGE, PURPLE, BLUE];

fn centred(s: &str, height: f32, cx: f32) -> f32 {
	cx - font::width(s, height) * 0.5
}

fn ground(h: &mut Hand, y: f32, x0: f32, x1: f32) {
	let mut pts = Vec::new();
	let mut x = x0;
	while x <= x1 {
		pts.push((x, y + h.rng.s() * 7.0));
		x += 90.0;
	}
	h.curve(&pts, &Pen::ink(GREEN).width(6.0).wobble(3.0));
	shapes::grass(h, x0 + 10.0, x1 - 10.0, y + 4.0);
}

fn scrolt(h: &mut Hand) -> Vec<String> {
	let mut t = Vec::new();
	shapes::sun(h, (900.0, 105.0), 52.0);
	shapes::cloud(h, (140.0, 175.0), 170.0);
	shapes::cloud(h, (640.0, 205.0), 150.0);
	let s = "THANK YOU";
	t.push(font::text(h, s, centred(s, 78.0, 430.0), 38.0, 78.0, &RAINBOW));
	shapes::house(h, 790.0, 440.0, 190.0, 150.0);
	ground(h, 596.0, 30.0, 995.0);
	people::ratchet(h, (270.0, 345.0), 1.1, false);
	people::kid(h, (512.0, 335.0), 1.2, RED, BROWN, (392.0, 452.0), (642.0, 440.0));
	people::clank(h, (720.0, 405.0), 1.05);
	shapes::heart(h, (520.0, 235.0), 26.0);
	shapes::star(h, (660.0, 320.0), 20.0, YELLOW);
	let n = "Scrolt!";
	t.push(font::text(h, n, centred(n, 105.0, 512.0), 625.0, 105.0, &RAINBOW));
	t
}

fn korixx(h: &mut Hand) -> Vec<String> {
	let mut t = Vec::new();
	// The night-sky scribble: sparse, light, and drawn in two patches that leave the title, the supporter and the thank-you line white.
	let pen = Pen::fill([0.18, 0.18, 0.52]).width(7.0).strength(0.28);
	h.hatch(&[(30.0, 205.0), (722.0, 200.0), (726.0, 668.0), (34.0, 672.0)], 0.3, 78.0, &pen);
	h.hatch(&[(765.0, 195.0), (1000.0, 192.0), (1000.0, 318.0), (768.0, 322.0)], 0.3, 80.0, &pen);
	let n = "korixx";
	t.push(font::text(h, n, centred(n, 100.0, 512.0), 34.0, 100.0, &RAINBOW));
	shapes::planet(h, (860.0, 250.0), 62.0, PURPLE);
	shapes::planet(h, (140.0, 540.0), 40.0, ORANGE);
	for (x, y, r, c) in [(120.0, 230.0, 18.0, YELLOW), (300.0, 190.0, 14.0, ORANGE), (640.0, 190.0, 20.0, YELLOW), (980.0, 470.0, 15.0, PINK), (330.0, 600.0, 17.0, YELLOW), (700.0, 590.0, 13.0, ORANGE)] {
		shapes::star(h, (x, y), r, c);
	}
	people::ship(h, (470.0, 380.0), 1.85);
	let w = (470.0 + 28.0 * 1.85, 380.0 - 26.0 * 1.85);
	let (rc, cc) = ((w.0 - 20.0, w.1 + 6.0), (w.0 + 22.0, w.1 + 12.0));
	h.colour_in(&ring(rc, 15.0, 14.0, 8), [0.98, 0.52, 0.1], false);
	h.ellipse(rc, 19.0, 18.0, &ink().width(3.6));
	h.path(&[(rc.0 - 15.0, rc.1 - 12.0), (rc.0 - 26.0, rc.1 - 36.0), (rc.0 - 4.0, rc.1 - 18.0)], &ink().width(3.6));
	h.path(&[(rc.0 + 5.0, rc.1 - 18.0), (rc.0 + 14.0, rc.1 - 40.0), (rc.0 + 18.0, rc.1 - 10.0)], &ink().width(3.6));
	h.colour_in(&ring(cc, 13.0, 12.0, 8), [0.55, 0.62, 0.78], false);
	h.ellipse(cc, 16.0, 15.0, &ink().width(3.6));
	h.line((cc.0 + 3.0, cc.1 - 15.0), (cc.0 + 8.0, cc.1 - 32.0), &ink().width(3.6));
	for p in [(rc.0 - 7.0, rc.1 - 3.0), (rc.0 + 7.0, rc.1 - 3.0), (cc.0 - 6.0, cc.1 - 2.0), (cc.0 + 6.0, cc.1 - 2.0)] {
		h.dot(p, 2.6, &ink());
	}
	people::kid(h, (850.0, 440.0), 1.2, [0.1, 0.6, 0.3], BLACK, (755.0, 330.0), (950.0, 345.0));
	let b = "THANK YOU!";
	t.push(font::text(h, b, centred(b, 70.0, 512.0), 672.0, 70.0, &WARM));
	t
}

fn retokyo(h: &mut Hand) -> Vec<String> {
	let mut t = Vec::new();
	let n = "reTokyo";
	t.push(font::text(h, n, centred(n, 100.0, 512.0), 36.0, 100.0, &RAINBOW));
	shapes::sun(h, (940.0, 125.0), 34.0);
	shapes::cloud(h, (130.0, 215.0), 150.0);
	ground(h, 600.0, 20.0, 1004.0);
	people::kid(h, (210.0, 355.0), 1.2, [0.95, 0.75, 0.1], BLACK, (90.0, 270.0), (332.0, 290.0));
	people::ratchet(h, (835.0, 365.0), 1.1, true);
	let bubble = [(300.0, 250.0), (400.0, 178.0), (520.0, 168.0), (640.0, 185.0), (712.0, 250.0), (660.0, 318.0), (540.0, 335.0), (390.0, 322.0)];
	h.closed(&bubble, &Pen::ink(BLUE).width(5.5));
	h.path(&[(660.0, 316.0), (748.0, 372.0), (705.0, 300.0)], &Pen::ink(BLUE).width(5.5));
	let s = "THANK YOU";
	t.push(font::text(h, s, centred(s, 46.0, 506.0), 228.0, 46.0, &WARM));
	shapes::heart(h, (210.0, 175.0), 24.0);
	for (x, y, r) in [(430.0, 640.0, 34.0), (520.0, 650.0, 38.0), (610.0, 645.0, 33.0), (470.0, 700.0, 36.0), (565.0, 705.0, 34.0), (655.0, 690.0, 30.0)] {
		shapes::bolt(h, (x, y), r);
	}
	shapes::wrench(h, (110.0, 690.0), (400.0, 650.0), 30.0, [0.75, 0.78, 0.9]);
	t
}

/// The scene `i` (0 = Scrolt, 1 = korixx, 2 = reTokyo) on its sheet with shadow and tape.
pub fn draw(i: usize) -> Drawing {
	let mut h = Hand::new(Canvas::new(W, H, SS, PAPER), 0xC0F1_0000 + i as u64 * 7919);
	let texts = match i {
		0 => scrolt(&mut h),
		1 => korixx(&mut h),
		_ => retokyo(&mut h),
	};
	let (w, hh, px) = h.c.downsample();
	let (w2, h2, rgba) = finish(w, hh, &px, i as u32);
	Drawing { name: SUPPORTERS[i], texts, w: w2, h: h2, rgba }
}

/// Paper grain, edge shade, the soft shadow in the margin and the tape.
fn finish(w: usize, h: usize, px: &[[f32; 3]], seed: u32) -> (usize, usize, Vec<[u8; 4]>) {
	let (tw, th) = (w + MARGIN * 2, h + MARGIN * 2);
	let mut f = vec![[0.0f32; 4]; tw * th];
	for y in 0..th {
		for x in 0..tw {
			let (sx, sy) = (x as f32 - MARGIN as f32 - 7.0, y as f32 - MARGIN as f32 - 9.0);
			let dx = (sx.max(0.0) - (w as f32 - 1.0)).max(0.0).max(-sx);
			let dy = (sy.max(0.0) - (h as f32 - 1.0)).max(0.0).max(-sy);
			let d = (dx * dx + dy * dy).sqrt();
			let a = (1.0 - d / 16.0).clamp(0.0, 1.0);
			f[y * tw + x] = [0.0, 0.0, 0.0, 0.42 * a * a];
		}
	}
	for y in 0..h {
		for x in 0..w {
			let p = px[y * w + x];
			let e = (x.min(w - 1 - x).min(y).min(h - 1 - y) as f32 / 40.0).min(1.0);
			let shade = 1.0 - 0.05 * (1.0 - e) * (1.0 - e) - 0.025 * noise2(x as f32 * 0.01, y as f32 * 0.01, seed) - 0.03 * hash2(x as i32, y as i32, seed + 5);
			f[(y + MARGIN) * tw + x + MARGIN] = [p[0] * shade, p[1] * shade, p[2] * shade, 1.0];
		}
	}
	tape(&mut f, tw, (MARGIN as f32 + 150.0, MARGIN as f32 + 6.0), -0.35);
	tape(&mut f, tw, ((tw - MARGIN) as f32 - 150.0, MARGIN as f32 + 4.0), 0.4);
	let rgba = f.iter().map(|p| [(p[0] * 255.0 + 0.5) as u8, (p[1] * 255.0 + 0.5) as u8, (p[2] * 255.0 + 0.5) as u8, (p[3] * 255.0 + 0.5) as u8]).collect();
	(tw, th, rgba)
}

/// A strip of translucent beige tape centred at `c`, tilted by `ang`.
fn tape(f: &mut [[f32; 4]], tw: usize, c: P, ang: f32) {
	let (ca, sa) = (super::rng::cos(ang), super::rng::sin(ang));
	let (hl, hw) = (62.0f32, 19.0f32);
	for y in (c.1 - 90.0) as usize..(c.1 + 90.0) as usize {
		for x in (c.0 - 110.0) as usize..(c.0 + 110.0) as usize {
			let (dx, dy) = (x as f32 - c.0, y as f32 - c.1);
			let (u, v) = (dx * ca + dy * sa, -dx * sa + dy * ca);
			let cu = (hl - u.abs()).clamp(0.0, 1.0);
			let cv = (hw - v.abs()).clamp(0.0, 1.0);
			let cov = cu * cv;
			if cov <= 0.0 {
				continue;
			}
			let a = 0.62 * cov * (0.92 + 0.08 * hash2(x as i32, y as i32, 99));
			let edge = if v.abs() > hw - 3.0 { 0.06 } else { 0.0 };
			let col = [0.93 + edge, 0.86 + edge, 0.62 + edge];
			let p = &mut f[y * tw + x];
			let oa = a + p[3] * (1.0 - a);
			for k in 0..3 {
				p[k] = (col[k] * a + p[k] * p[3] * (1.0 - a)) / oa;
			}
			p[3] = oa;
		}
	}
}
