//! Ko-fi drawings: the people (and machines) of the pictures: the supporter as a stick figure with a big round head, Ratchet with
//! big pointy ears, Clank the little robot and Ratchet's ship. `c` is always the head centre (or the ship's middle) and `s` the
//! size (1 = a head 40 units across).

use super::canvas::{P, Pen};
use super::hand::Hand;
use super::rng::{cos, sin};
use super::shapes::{self, BLUE, BROWN, GREEN, LBLUE, ORANGE, PINK, RED, YELLOW, ink, ring};

fn at(c: P, s: f32, x: f32, y: f32) -> P {
	(c.0 + x * s, c.1 + y * s)
}

/// Dot eyes and a smile on a head at `c`.
fn face(h: &mut Hand, c: P, s: f32, big: bool) {
	let r = if big { 5.5 } else { 3.6 } * s;
	h.dot(at(c, s, -14.0, -6.0), r, &ink());
	h.dot(at(c, s, 14.0, -6.0), r, &ink());
	h.arc(at(c, s, 0.0, 6.0), 19.0 * s, 15.0 * s, 0.3, 2.85, &ink().width(4.0));
}

/// A hand with a few spiky fingers.
fn hand_at(h: &mut Hand, p: P, s: f32, dir: f32) {
	for k in -1..=1 {
		let a = dir + k as f32 * 0.55;
		h.spike(p, (p.0 + cos(a) * 11.0 * s, p.1 + sin(a) * 11.0 * s), &ink().width(3.6));
	}
}

fn atan2(y: f32, x: f32) -> f32 {
	y.atan2(x)
}

/// The supporter: a stick figure. `lh` / `rh` are the hands (absolute positions), `shirt` / `hair` the colours.
pub fn kid(h: &mut Hand, c: P, s: f32, shirt: [f32; 3], hair: [f32; 3], lh: P, rh: P) {
	let head = h.rng.range(0.95, 1.06);
	h.ellipse(c, 40.0 * s * head, 37.0 * s, &ink());
	for i in 0..8 {
		let a = -3.14 + (i as f32 + 0.5) / 8.0 * 3.14 + h.rng.s() * 0.08;
		let (r0, r1) = (36.0 * s, h.rng.range(52.0, 66.0) * s);
		h.spike((c.0 + cos(a) * r0 * 1.05, c.1 + sin(a) * r0), (c.0 + cos(a) * r1, c.1 + sin(a) * r1), &Pen::ink(hair).width(5.0));
	}
	face(h, c, s, false);
	let (ls, rs) = (at(c, s, -22.0, 52.0), at(c, s, 22.0, 52.0));
	h.line(at(c, s, 0.0, 38.0), at(c, s, 0.0, 56.0), &ink());
	let body = [at(c, s, -20.0, 54.0), at(c, s, 22.0, 52.0), at(c, s, 36.0, 128.0), at(c, s, -32.0, 130.0)];
	h.colour_in(&body, shirt, true);
	h.outline(&body, &Pen::ink(shirt).width(5.5));
	h.curve(&[ls, ((ls.0 + lh.0) * 0.5 + 4.0 * s, (ls.1 + lh.1) * 0.5 - 6.0 * s), lh], &ink());
	h.curve(&[rs, ((rs.0 + rh.0) * 0.5 - 4.0 * s, (rs.1 + rh.1) * 0.5 + 5.0 * s), rh], &ink());
	hand_at(h, lh, s, atan2(lh.1 - ls.1, lh.0 - ls.0));
	hand_at(h, rh, s, atan2(rh.1 - rs.1, rh.0 - rs.0));
	let shorts = [at(c, s, -30.0, 126.0), at(c, s, 34.0, 124.0), at(c, s, 30.0, 150.0), at(c, s, -28.0, 152.0)];
	h.colour_in(&shorts, BLUE, false);
	h.line(at(c, s, -16.0, 150.0), at(c, s, -24.0, 205.0), &ink());
	h.line(at(c, s, 16.0, 148.0), at(c, s, 26.0, 205.0), &ink());
	h.path(&[at(c, s, -24.0, 205.0), at(c, s, -40.0, 208.0)], &ink());
	h.path(&[at(c, s, 26.0, 205.0), at(c, s, 44.0, 209.0)], &ink());
}

/// Ratchet: orange, big long ears, green eyes, a wrench in the hand.
pub fn ratchet(h: &mut Hand, c: P, s: f32, wrench_up: bool) {
	let fur = [0.98, 0.52, 0.1];
	for sx in [-1.0f32, 1.0] {
		let ear = [at(c, s, sx * 26.0, -36.0), at(c, s, sx * 60.0, -108.0), at(c, s, sx * 54.0, -30.0)];
		h.colour_in(&ear, fur, false);
		h.outline(&ear, &ink());
		h.line(at(c, s, sx * 40.0, -40.0), at(c, s, sx * 52.0, -80.0), &Pen::ink(PINK).width(5.0));
	}
	h.colour_in(&ring(c, 40.0 * s, 36.0 * s, 10), fur, true);
	h.ellipse(c, 46.0 * s, 40.0 * s, &ink());
	let muzzle = at(c, s, 0.0, 14.0);
	h.colour_in(&ring(muzzle, 22.0 * s, 15.0 * s, 8), [0.96, 0.8, 0.5], false);
	h.ellipse(muzzle, 24.0 * s, 17.0 * s, &ink().width(3.8));
	for sx in [-1.0f32, 1.0] {
		let e = at(c, s, sx * 19.0, -10.0);
		h.ellipse(e, 9.0 * s, 10.0 * s, &ink().width(3.8));
		h.dot(at(c, s, sx * 19.0, -9.0), 4.2 * s, &Pen::ink(GREEN).width(8.0));
	}
	h.dot(at(c, s, 0.0, 6.0), 4.5 * s, &ink());
	h.arc(at(c, s, 0.0, 14.0), 13.0 * s, 9.0 * s, 0.35, 2.8, &ink().width(3.8));
	let body = [at(c, s, -26.0, 42.0), at(c, s, 28.0, 40.0), at(c, s, 34.0, 118.0), at(c, s, -34.0, 120.0)];
	h.colour_in(&body, fur, true);
	h.outline(&body, &ink());
	h.colour_in(&[at(c, s, -8.0, 62.0), at(c, s, 12.0, 62.0), at(c, s, 14.0, 100.0), at(c, s, -10.0, 100.0)], [0.97, 0.85, 0.55], false);
	h.line(at(c, s, -26.0, 56.0), at(c, s, -66.0, 90.0), &ink());
	let hand = at(c, s, if wrench_up { 84.0 } else { 66.0 }, if wrench_up { 14.0 } else { 80.0 });
	h.line(at(c, s, 28.0, 56.0), hand, &ink());
	let (w0, w1) = if wrench_up { (at(c, s, 86.0, -44.0), at(c, s, 92.0, 52.0)) } else { (at(c, s, 56.0, 70.0), at(c, s, 82.0, 156.0)) };
	shapes::wrench(h, w0, w1, 14.0 * s, [0.8, 0.82, 0.88]);
	h.line(at(c, s, -16.0, 118.0), at(c, s, -20.0, 160.0), &ink());
	h.line(at(c, s, 18.0, 118.0), at(c, s, 22.0, 160.0), &ink());
	h.path(&[at(c, s, -20.0, 160.0), at(c, s, -38.0, 164.0)], &ink());
	h.path(&[at(c, s, 22.0, 160.0), at(c, s, 40.0, 164.0)], &ink());
	let tail = [at(c, s, -30.0, 110.0), at(c, s, -62.0, 130.0), at(c, s, -76.0, 100.0), at(c, s, -94.0, 90.0)];
	h.curve(&tail, &Pen::ink(ORANGE).width(9.0));
	for k in 0..3 {
		let p = at(c, s, -62.0 - k as f32 * 6.0, 126.0 - k as f32 * 12.0);
		h.line((p.0 - 5.0 * s, p.1 - 6.0 * s), (p.0 + 6.0 * s, p.1 + 6.0 * s), &Pen::ink(BROWN).width(3.4));
	}
}

/// Clank: a small grey robot with an antenna and round eyes.
pub fn clank(h: &mut Hand, c: P, s: f32) {
	let grey = [0.5, 0.58, 0.72];
	let head = h.closed(&[at(c, s, 0.0, -30.0), at(c, s, 30.0, -4.0), at(c, s, 24.0, 28.0), at(c, s, -26.0, 28.0), at(c, s, -30.0, -4.0)], &ink());
	h.colour_in(&head, grey, true);
	h.line(at(c, s, 4.0, -30.0), at(c, s, 10.0, -58.0), &ink());
	h.dot(at(c, s, 10.0, -62.0), 6.0 * s, &Pen::ink(RED));
	for sx in [-1.0f32, 1.0] {
		h.ellipse(at(c, s, sx * 12.0, -2.0), 8.5 * s, 8.5 * s, &ink().width(4.0));
		h.dot(at(c, s, sx * 12.0 + 1.0, -1.0), 3.2 * s, &ink());
	}
	h.line(at(c, s, -9.0, 18.0), at(c, s, 9.0, 18.0), &ink().width(3.6));
	let body = [at(c, s, -26.0, 34.0), at(c, s, 28.0, 33.0), at(c, s, 30.0, 96.0), at(c, s, -28.0, 98.0)];
	h.colour_in(&body, grey, true);
	h.outline(&body, &ink());
	h.ellipse(at(c, s, 0.0, 62.0), 8.0 * s, 8.0 * s, &Pen::ink(YELLOW).width(6.0));
	h.line(at(c, s, -26.0, 46.0), at(c, s, -52.0, 70.0), &ink());
	h.line(at(c, s, 28.0, 46.0), at(c, s, 54.0, 24.0), &ink());
	h.line(at(c, s, -12.0, 98.0), at(c, s, -14.0, 130.0), &ink());
	h.line(at(c, s, 14.0, 98.0), at(c, s, 16.0, 130.0), &ink());
	h.path(&[at(c, s, -14.0, 130.0), at(c, s, -30.0, 132.0)], &ink());
	h.path(&[at(c, s, 16.0, 130.0), at(c, s, 32.0, 132.0)], &ink());
}

/// The ship, flying to the right: a blue egg body with a dome, a fin and a red-yellow flame behind.
pub fn ship(h: &mut Hand, c: P, s: f32) {
	let flame = [at(c, s, -92.0, -6.0), at(c, s, -140.0, -34.0), at(c, s, -122.0, -10.0), at(c, s, -172.0, 4.0), at(c, s, -124.0, 12.0), at(c, s, -146.0, 40.0), at(c, s, -92.0, 16.0)];
	h.colour_in(&flame, ORANGE, true);
	let core: Vec<P> = flame.iter().map(|p| (p.0 + (c.0 - 110.0 * s - p.0) * 0.4, p.1 + (c.1 + 2.0 * s - p.1) * 0.4)).collect();
	h.colour_in(&core, YELLOW, false);
	h.outline(&flame, &Pen::ink(RED).width(5.5));
	let body = h.closed(&[at(c, s, -96.0, 4.0), at(c, s, -50.0, -42.0), at(c, s, 40.0, -48.0), at(c, s, 112.0, -4.0), at(c, s, 60.0, 40.0), at(c, s, -40.0, 42.0)], &ink());
	h.colour_in(&body, LBLUE, true);
	let fin = [at(c, s, -64.0, -30.0), at(c, s, -110.0, -86.0), at(c, s, -26.0, -44.0)];
	h.colour_in(&fin, RED, false);
	h.outline(&fin, &ink());
	h.colour_in(&ring(at(c, s, 28.0, -26.0), 22.0 * s, 20.0 * s, 8), [0.95, 0.98, 1.0], false);
	h.ellipse(at(c, s, 28.0, -26.0), 28.0 * s, 25.0 * s, &ink());
	h.line(at(c, s, -72.0, 8.0), at(c, s, 70.0, 12.0), &Pen::ink(BLUE).width(4.5));
	for k in 0..3 {
		h.dot(at(c, s, -30.0 + k as f32 * 26.0, 26.0), 4.5 * s, &Pen::ink(YELLOW).width(8.0));
	}
}
