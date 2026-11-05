//! The unlock pop-up (top right, about 5 s, queued: the entry's icon in its grade ring, `badge.rs`) and the procedural grade cups
//! (the stand-in for a game without icons). Time comes from the wall clock at draw time, so the pop-up looks the same on stepped and
//! interpolated frames.
use super::badge::badge;
use super::{AchPack, Grade};
use crate::overlay::{self, Quad};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// Seconds a pop-up stays (fade in 0.35, fade out 0.5 inside it).
pub const SHOW: f32 = 5.0;
const FADE_IN: f32 = 0.35;
const FADE_OUT: f32 = 0.5;

/// The pop-up being shown: (entry index, start).
static CURRENT: Mutex<Option<(usize, Instant)>> = Mutex::new(None);
/// The game shows a full page of its own (`hold`).
static HELD: AtomicBool = AtomicBool::new(false);

/// Holds the pop-up while the game shows a full-screen page of its own whose header and panels fill the top right (R&C1's modern pause,
/// shop, Galactic Map and Options pages): nothing is drawn, the queue waits, and the pop-up that was showing starts over once the hold
/// ends, so it is seen whole. Set before `draw` each frame.
pub fn hold(on: bool) {
	HELD.store(on, Ordering::Relaxed);
}

/// One frame of the pop-up clock at `now`: the entry to show and its age in seconds, or None. Held: nothing shows, `next` (the queue)
/// is not asked, and the pop-up showing restarts its clock.
pub(super) fn tick(cur: &mut Option<(usize, Instant)>, held: bool, now: Instant, next: impl FnOnce() -> Option<usize>) -> Option<(usize, f32)> {
	if held {
		if let Some((_, t0)) = cur.as_mut() {
			*t0 = now;
		}
		return None;
	}
	if cur.is_some_and(|(_, t0)| now.duration_since(t0).as_secs_f32() >= SHOW) {
		*cur = None;
	}
	if cur.is_none() {
		*cur = next().map(|i| (i, now));
	}
	cur.map(|(i, t0)| (i, now.duration_since(t0).as_secs_f32()))
}

/// Opacity 0..1 and slide-in 0..1 (1 = fully out) `t` seconds after the pop-up started; None once it is over.
pub fn phase(t: f32) -> Option<(f32, f32)> {
	if !(0.0..SHOW).contains(&t) {
		return None;
	}
	let a = (t / FADE_IN).min(1.0) * ((SHOW - t) / FADE_OUT).min(1.0);
	let e = (t / FADE_IN).min(1.0);
	Some((a, 1.0 - (1.0 - e) * (1.0 - e)))
}

/// Draws the current pop-up (taking the next from the queue when none shows) into the HUD quads.
pub fn draw(out: &mut Vec<Quad>, w: f32, h: f32) {
	let mut hub = super::hub();
	let Some(hb) = hub.as_mut() else { return };
	let mut cur = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
	let Some((i, t)) = tick(&mut cur, HELD.load(Ordering::Relaxed), Instant::now(), || hb.queue.pop_front()) else { return };
	let Some((a, slide)) = phase(t) else { return };
	if i < hb.pack.defs.len() {
		panel(out, w, h, (hb.pack, i), a, slide);
	}
}

/// Layout scale: 1.0 at 1080 lines.
pub fn scale(h: f32) -> f32 {
	(h / 1080.0).clamp(0.6, 3.0)
}

/// The largest text size (at most `max`) that keeps `s` inside `width` pixels.
pub fn fit(s: &str, width: f32, max: f32) -> f32 {
	let n = s.chars().count().max(1) as f32;
	max.min(width / (n * 0.75))
}

/// `entry` = the game's pack and the index of the unlocked entry. The badge (icon in its grade ring) is 64 px at 1080 lines, 128 at 4K.
fn panel(out: &mut Vec<Quad>, w: f32, h: f32, entry: (&AchPack, usize), a: f32, slide: f32) {
	let (p, i) = entry;
	let d = &p.defs[i];
	let k = scale(h);
	let (pw, ph) = (420.0 * k, 80.0 * k);
	let x = w - pw - 18.0 * k + (1.0 - slide) * 60.0 * k;
	let y = 56.0 * k;
	let c = d.grade.colour();
	overlay::rect(out, [x - 2.0 * k, y - 2.0 * k, pw + 4.0 * k, ph + 4.0 * k], [c[0], c[1], c[2], 0.85 * a]);
	overlay::rect(out, [x, y, pw, ph], [0.04, 0.05, 0.08, 0.88 * a]);
	badge(out, p, i, true, [x + 8.0 * k, y + 8.0 * k, 64.0 * k], a);
	let tx = x + 84.0 * k;
	let avail = pw - 94.0 * k;
	overlay::text(out, "ACHIEVEMENT UNLOCKED", tx, y + 12.0 * k, 12.0 * k, [c[0], c[1], c[2], a]);
	// A long name wraps to two lines at a space instead of shrinking to unreadable (fix round B16).
	let (lines, size) = name_lines(d.name, avail, 20.0 * k, 14.0 * k, 15.0 * k);
	let (top, label_y) = if lines.len() > 1 { (26.0, 62.0) } else { (30.0, 56.0) };
	for (n, l) in lines.iter().enumerate() {
		overlay::text(out, l, tx, y + top * k + n as f32 * size * 1.05, size, [1.0, 1.0, 1.0, a]);
	}
	let line = d.grade.label();
	overlay::text(out, line, tx, y + label_y * k, 11.0 * k, [0.75, 0.78, 0.85, a]);
}

/// The pop-up's name as one line at up to `one_max`, or, when that would be smaller than `floor`, as two lines split at the space
/// that leaves the shorter longest line, at up to `two_max`. No space = one line, whatever its size.
pub fn name_lines(name: &str, avail: f32, one_max: f32, floor: f32, two_max: f32) -> (Vec<&str>, f32) {
	let one = fit(name, avail, one_max);
	if one >= floor {
		return (vec![name], one);
	}
	let len = |s: &str| s.chars().count();
	let Some((_, i)) = name.char_indices().filter(|&(_, c)| c == ' ').map(|(i, _)| (len(&name[..i]).max(len(&name[i + 1..])), i)).min() else { return (vec![name], one) };
	let (a, b) = (&name[..i], &name[i + 1..]);
	(vec![a, b], fit(if len(a) >= len(b) { a } else { b }, avail, two_max))
}

/// A trophy cup in a `s` x `s` box at (x, y), tinted by the grade: bowl, handles, stem, base, a light stripe.
pub fn cup(out: &mut Vec<Quad>, x: f32, y: f32, s: f32, g: Grade, a: f32) {
	let c = g.colour();
	let col = [c[0], c[1], c[2], a];
	let dark = [c[0] * 0.6, c[1] * 0.6, c[2] * 0.6, a];
	let light = [(c[0] + 0.3).min(1.0), (c[1] + 0.3).min(1.0), (c[2] + 0.3).min(1.0), a];
	let cx = x + s * 0.5;
	let rows = [0.64, 0.60, 0.54, 0.46, 0.36, 0.24];
	let rh = s * 0.085;
	for (i, wf) in rows.iter().enumerate() {
		let rw = s * wf;
		overlay::rect(out, [cx - rw * 0.5, y + s * 0.08 + i as f32 * rh, rw, rh + 0.5], col);
	}
	// Handles: an arm out of the bowl's top corners and a drop back in.
	for side in [-1.0f32, 1.0] {
		let hx = cx + side * s * 0.32;
		let (ox, ow) = if side < 0.0 { (hx - s * 0.16, s * 0.16) } else { (hx, s * 0.16) };
		overlay::rect(out, [ox, y + s * 0.12, ow, s * 0.07], dark);
		overlay::rect(out, [if side < 0.0 { ox } else { ox + ow - s * 0.07 }, y + s * 0.12, s * 0.07, s * 0.26], dark);
		overlay::rect(out, [ox, y + s * 0.31, ow, s * 0.07], dark);
	}
	overlay::rect(out, [cx - s * 0.06, y + s * 0.08 + 6.0 * rh, s * 0.12, s * 0.17], dark);
	overlay::rect(out, [cx - s * 0.22, y + s * 0.76, s * 0.44, s * 0.09], col);
	overlay::rect(out, [cx - s * 0.28, y + s * 0.85, s * 0.56, s * 0.07], dark);
	overlay::rect(out, [cx - s * 0.22, y + s * 0.12, s * 0.07, s * 0.28], light);
}
