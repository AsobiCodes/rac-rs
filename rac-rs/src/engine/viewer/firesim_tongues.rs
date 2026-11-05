//! [PC] Fire simulation: big fires break into tongues. A flame volume wider than ~1.2 m is not one emitter (one round mass of big
//! particles pulled to one axis) but several sub-emitters spread over its base: count clamp(width / 0.6 m, 1, 8), each with its own
//! random phase (e.w: firesim_step.wgsl spawn_flame pulses the tongue's height with it) and a slightly different strength and height,
//! and smaller particles (foot radius / sqrt(count)); together they keep the volume's particle count (same total heat, same budget).

use super::firesim::{Want, EMIT_FLAME};

/// Width (m) one sub-emitter covers.
const SUB_W: f32 = 0.6;
/// Most sub-emitters per volume.
const SUB_MAX: usize = 8;

/// A number in 0..1 from (place, i): stable over frames for a fire that stays put.
fn hash01(p: [f32; 3], i: u32) -> f32 {
	let q = |v: f32| (v * 8.0).round() as i32 as u32;
	let mut x = q(p[0]).wrapping_mul(0x9E37_79B9) ^ q(p[1]).wrapping_mul(0x85EB_CA6B) ^ q(p[2]).wrapping_mul(0xC2B2_AE35) ^ i.wrapping_mul(0x27D4_EB2F);
	x ^= x >> 16;
	x = x.wrapping_mul(0x7FEB_352D);
	x ^= x >> 15;
	(x >> 8) as f32 / 16_777_216.0
}

/// How many sub-emitters a flame of foot radius `r` gets (dev knob fire_tongue_w: the width one covers, 0 = never split).
pub(super) fn sub_count(r: f32) -> usize {
	let w = super::firefx::tune("fire_tongue_w", SUB_W);
	if w <= 0.0 {
		return 1;
	}
	((2.0 * r / w).floor() as usize).clamp(1, SUB_MAX)
}

/// How many rows `w` would like (a wide flame: its sub-emitters; anything else 1).
fn wanted(w: &Want) -> usize {
	if w.e.n[1] == EMIT_FLAME && w.e.n[2] != 2 { sub_count(w.e.b[0]) } else { 1 }
}

/// The frame's emitters `src` into `out`, the wide flames split into tongues within a budget: `out` never has more than
/// max(`cap`, src.len()) rows and every emitter of `src` keeps at least one (a fire never vanishes because others split). The spare
/// rows (cap - src.len()) go to the flames that spawn most (big and near) first; a flame that does not get all it wants splits less.
/// Returns the rows the split would have made without a cap (the `fire_vol_log` line).
pub(super) fn split_all(src: &[Want], cap: usize, out: &mut Vec<Want>) -> usize {
	out.clear();
	let want: Vec<usize> = src.iter().map(wanted).collect();
	let full: usize = want.iter().sum();
	let mut n = vec![1usize; src.len()];
	let mut spare = cap.saturating_sub(src.len());
	if full <= cap {
		n.copy_from_slice(&want);
	} else if spare > 0 {
		let mut order: Vec<usize> = (0..src.len()).filter(|&i| want[i] > 1).collect();
		order.sort_by(|&a, &b| src[b].live.total_cmp(&src[a].live).then(a.cmp(&b)));
		for i in order {
			let extra = (want[i] - 1).min(spare);
			n[i] += extra;
			spare -= extra;
			if spare == 0 {
				break;
			}
		}
	}
	for (w, k) in src.iter().zip(n) {
		push_n(*w, k, out);
	}
	full
}

/// Pushes `w` into `out`, a wide flame as its sub-emitters (anything else, or a narrow flame, as it is).
#[cfg(test)]
pub(super) fn push_split(w: Want, out: &mut Vec<Want>) {
	let n = wanted(&w);
	push_n(w, n, out);
}

/// Pushes `w` into `out` as `n` sub-emitters (1: as it is).
fn push_n(w: Want, n: usize, out: &mut Vec<Want>) {
	let r = w.e.b[0];
	if n <= 1 {
		out.push(w);
		return;
	}
	let foot = [w.e.a[0], w.e.a[1], w.e.a[2]];
	let h = w.e.a[3];
	let wgt: Vec<f32> = (0..n as u32).map(|i| 0.75 + 0.5 * hash01(foot, 3 * i)).collect();
	let sum: f32 = wgt.iter().sum();
	let rs = r / (n as f32).sqrt() * 1.15;
	let turn = hash01(foot, 999) * std::f32::consts::TAU;
	for (i, k) in wgt.iter().enumerate() {
		let f = (i as f32 + 0.5) / n as f32;
		// Sunflower spread over the base disc (no two tongues on one spot), turned per fire.
		let (rad, ang) = (0.72 * r * f.sqrt(), turn + i as f32 * 2.399_963);
		let mut e = w.e;
		let level = w.e.n[2] == 1;
		// A level tongue's licks stay inside its retail quad (lane nfire agent 17): none taller than the quad, each one's reach across
		// what is left of the quad's at its spot (never past the quad's side x LEVEL_X), so the licks stand apart with gaps between them.
		let hk = if level { 0.7 + 0.3 * hash01(foot, 3 * i as u32 + 1) } else { 0.7 + 0.6 * hash01(foot, 3 * i as u32 + 1) };
		e.a = [foot[0] + rad * ang.cos(), foot[1] + rad * ang.sin(), foot[2], h * hk];
		e.b[0] = rs;
		if level {
			(e.b[1], e.b[2]) = ((w.e.b[1] - rad).max(0.5 * rs), h * hk);
		}
		e.e[3] = 0.01 + 0.99 * hash01(foot, 3 * i as u32 + 2);
		let s = k / sum;
		out.push(Want { e, rate: w.rate * s, live: w.live * s, weigh: w.weigh * s });
	}
}
