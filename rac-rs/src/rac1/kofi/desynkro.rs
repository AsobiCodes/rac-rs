//! Ko-fi drawings: DeSynkro's sheet (user 2026-11-04: DeSynkro helped so much with the Linux support, so a penguin): a big penguin
//! waving next to Ratchet, a computer whose screen shows a terminal prompt and "rac-rs", Clank, snowflakes on a snowy ground. Same
//! felt-marker hand as the other sheets. The marker blends by multiply (canvas.rs), so nothing light can go over dark: the
//! penguin's belly and face and the computer's screen are the paper left white.

use super::canvas::{P, Pen};
use super::font;
use super::hand::Hand;
use super::people;
use super::rng::{cos, sin};
use super::shapes::{self, BLACK, BLUE, GREEN, LBLUE, ORANGE, PURPLE, RED, ink};

const RAINBOW: [[f32; 3]; 6] = [RED, ORANGE, [0.1, 0.65, 0.2], BLUE, PURPLE, [0.9, 0.3, 0.6]];
const WARM: [[f32; 3]; 4] = [RED, ORANGE, PURPLE, BLUE];
/// The penguin's back: a dark marker, not quite black.
const DARK: [f32; 3] = [0.16, 0.17, 0.24];
const BEAK: [f32; 3] = [1.0, 0.62, 0.08];

fn at(c: P, s: f32, x: f32, y: f32) -> P {
	(c.0 + x * s, c.1 + y * s)
}

/// A penguin waving its left flipper: dark back, flippers and head cap; the belly and face are the white paper; orange beak and
/// feet. `c` = the head centre; about 320 units from the top of the head to the feet at s = 1.
pub(super) fn penguin(h: &mut Hand, c: P, s: f32) {
	let p = |x: f32, y: f32| at(c, s, x, y);
	for sx in [-1.0f32, 1.0] {
		let foot = [p(sx * 10.0, 250.0), p(sx * 62.0, 252.0), p(sx * 66.0, 268.0), p(sx * 8.0, 268.0)];
		h.colour_in(&foot, BEAK, false);
		h.outline(&foot, &ink().width(4.0));
	}
	let up = [p(-66.0, 92.0), p(-128.0, 22.0), p(-146.0, 34.0), p(-84.0, 128.0)];
	let down = [p(70.0, 92.0), p(118.0, 168.0), p(104.0, 182.0), p(76.0, 146.0)];
	for f in [&up, &down] {
		h.colour_in(f, DARK, true);
		h.outline(f, &ink());
	}
	// The body's outline (open at the neck), then the back as two convex sides: the belly between them stays paper.
	h.curve(&[p(-38.0, 44.0), p(-78.0, 110.0), p(-90.0, 182.0), p(-70.0, 238.0), p(0.0, 258.0), p(70.0, 238.0), p(90.0, 182.0), p(78.0, 110.0), p(38.0, 44.0)], &ink());
	for sx in [-1.0f32, 1.0] {
		let side = [p(sx * 38.0, 44.0), p(sx * 78.0, 112.0), p(sx * 88.0, 184.0), p(sx * 68.0, 236.0), p(sx * 44.0, 244.0), p(sx * 30.0, 60.0)];
		h.colour_in(&side, DARK, true);
	}
	// The head: a dark cap and cheeks round the white face, eyes, the beak.
	h.ellipse(c, 56.0 * s, 50.0 * s, &ink());
	h.colour_in(&[p(-54.0, -14.0), p(-40.0, -40.0), p(0.0, -52.0), p(40.0, -40.0), p(54.0, -14.0)], DARK, true);
	for sx in [-1.0f32, 1.0] {
		h.colour_in(&[p(sx * 56.0, -14.0), p(sx * 40.0, -14.0), p(sx * 34.0, 30.0), p(sx * 48.0, 26.0)], DARK, false);
		h.ellipse(p(sx * 16.0, -2.0), 9.0 * s, 10.0 * s, &ink().width(3.6));
		h.dot(p(sx * 15.0, 0.0), 4.4 * s, &ink());
	}
	let beak = [p(-20.0, 14.0), p(20.0, 14.0), p(0.0, 34.0)];
	h.colour_in(&beak, BEAK, false);
	h.outline(&beak, &ink().width(3.6));
}

/// A computer on its stand, the screen left white: a terminal prompt `>_` and "rac-rs" on it. Returns the screen's text.
fn computer(h: &mut Hand) -> String {
	let screen = [(700.0, 398.0), (902.0, 390.0), (908.0, 532.0), (694.0, 538.0)];
	h.outline(&screen, &Pen::ink([0.45, 0.48, 0.56]).width(14.0));
	h.outline(&screen, &ink());
	let prompt = ink().width(6.0);
	h.path(&[(726.0, 422.0), (750.0, 440.0), (726.0, 458.0)], &prompt);
	h.line((760.0, 460.0), (790.0, 460.0), &prompt);
	let t = font::text(h, "rac-rs", 728.0, 474.0, 40.0, &[GREEN, [0.1, 0.55, 0.2]]);
	h.line((800.0, 540.0), (800.0, 600.0), &Pen::ink(BLACK).width(9.0));
	let foot = [(746.0, 600.0), (854.0, 600.0), (866.0, 620.0), (734.0, 620.0)];
	h.colour_in(&foot, [0.55, 0.58, 0.66], false);
	h.outline(&foot, &ink());
	t
}

fn snowflake(h: &mut Hand, c: P, r: f32) {
	let pen = Pen::ink(LBLUE).width(4.0);
	for k in 0..3 {
		let a = 0.2 + k as f32 * 1.047;
		h.line((c.0 - cos(a) * r, c.1 - sin(a) * r), (c.0 + cos(a) * r, c.1 + sin(a) * r), &pen);
	}
}

pub fn desynkro(h: &mut Hand) -> Vec<String> {
	let mut t = Vec::new();
	let n = "DeSynkro";
	t.push(font::text(h, n, 512.0 - font::width(n, 96.0) * 0.5, 30.0, 96.0, &RAINBOW));
	shapes::cloud(h, (130.0, 200.0), 150.0);
	shapes::cloud(h, (905.0, 215.0), 130.0);
	for (x, y, r) in [(300.0, 200.0, 17.0), (660.0, 190.0, 15.0), (215.0, 320.0, 13.0), (705.0, 305.0, 16.0), (965.0, 345.0, 12.0), (395.0, 262.0, 11.0), (60.0, 430.0, 14.0)] {
		snowflake(h, (x, y), r);
	}
	// Snowy ground: a light blue line, no grass.
	let mut pts = Vec::new();
	let mut x = 20.0;
	while x <= 1004.0 {
		pts.push((x, 622.0 + h.rng.s() * 6.0));
		x += 90.0;
	}
	h.curve(&pts, &Pen::ink(LBLUE).width(6.0).wobble(3.0));
	t.push(computer(h));
	penguin(h, (500.0, 356.0), 1.0);
	people::ratchet(h, (190.0, 456.0), 1.0, true);
	people::clank(h, (958.0, 508.0), 0.85);
	shapes::heart(h, (626.0, 262.0), 22.0);
	let b = "THANK YOU!";
	t.push(font::text(h, b, 512.0 - font::width(b, 70.0) * 0.5, 672.0, 70.0, &WARM));
	t
}
