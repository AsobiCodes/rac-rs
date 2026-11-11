//! [PC] Mirrored planes held while their water is still in view (lane pwat2, 2026-11-08). A water mesh that is only partly flat
//! (a stream whose flat bed holds about a quarter of its vertices, the rest running up its banks or rippling) passes the flat-plane
//! test (`plane_of`: a quarter of the vertices at one height) on some frames and fails it on others as retail animates it. The
//! plane then left the mirror pick, its flat pixels fell from the planar mirror to the screen-space reflection, and back: the
//! water flickered between two looks (Novalis cave, the water inside the broken pipe: 67.52 present / absent in RAC_WATER_LOG,
//! bright and dark frames in turn; user 2026-11-07/08). Now a plane mirrored last frame stays a candidate while ONE still, low
//! water mesh without a flat plane holds at least half of the plane's box (`covers`), for at most `UNFOUND_CAP` frames since the
//! estimate last found it (its boxes and height are refreshed whenever it is found): the pick, and so the reflection, no longer
//! depend on that frame's estimate. Planes the estimate finds are unchanged; nothing is added for water that has left the view
//! (no covering mesh drawn), for waterfalls or tall strips (`MAX_RISE`), for rivers (`WaterDraw::flowing`) or other liquids.
//! The memory is cleared on a frame without a pick (`Memory::forget`), a level change, a Water setting change or a camera jump.

use super::super::waterfx::WaterDraw;

/// How far (units) a covering mesh may lie outside the held plane's box, and its height outside the mesh's height range.
const SLACK: f32 = 0.5;
/// The share of the held plane's XY box one covering mesh's XY box must hold.
const COVER_SHARE: f32 = 0.5;
/// A covering mesh's height range may be at most this share of the plane's longest side and of half its own longest side (it
/// is low and not steep). The Novalis pipe's plane (13.6 x 17.5) is held by a stream strip running down through the pipe,
/// 170.8..237.1 x 111.4..138.3 x 59.7..67.6 (rise 7.9 = 0.45 of the plane's side, 0.24 of its own 66.3; RAC_WATER_LOG `unflat`,
/// 2026-11-08): margin 2x. A waterfall (taller than it is long) or a strip taller than the plane is wide never holds it.
const MAX_RISE: f32 = 1.0;
/// Frames a held plane stays a candidate after the estimate last found it. The longest the Novalis pipe's 67.52 went unfound in
/// the user's replay was 150 frames (pp\run_a_lane, about 44 fields at 3.4 frames per field); 3x that.
pub(in crate::viewer) const UNFOUND_CAP: u32 = 450;
/// Frames a picked plane the estimate and the hold both lose stays mirrored while a slot is free (as before lane pwat2).
const MIRROR_HOLD: u32 = 12;
/// An eye move (units) between two frames that counts as a camera cut / teleport: the memory is cleared.
const JUMP: f32 = 16.0;

/// One mirrored plane: its height, every plane box within 0.4 of it that the pick saw, frames since it was last picked and since
/// the estimate last found it.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::viewer) struct Held {
	pub(in crate::viewer) height: f32,
	pub(in crate::viewer) boxes: Vec<[f32; 5]>,
	age: u32,
	unfound: u32,
}

/// The planes mirrored last frame, with what they were picked under (level, slot count, eye).
#[derive(Debug)]
pub(in crate::viewer) struct Memory {
	level: usize,
	n: usize,
	eye: [f32; 3],
	pub(in crate::viewer) held: Vec<Held>,
}

impl Held {
	/// Frames since the estimate last found this plane.
	pub(in crate::viewer) fn unfound(&self) -> u32 {
		self.unfound
	}
}

impl Memory {
	pub(in crate::viewer) const fn new() -> Self {
		Self { level: usize::MAX, n: 0, eye: [0.0; 3], held: Vec::new() }
	}

	/// No mirror pick this frame (no water drawn, Water Low/Classic): nothing carries over to the next pick.
	pub(in crate::viewer) fn forget(&mut self) {
		self.held.clear();
	}

	/// Start a frame's pick: a new level, slot count (Water setting) or a camera jump drops what was held.
	pub(in crate::viewer) fn start(&mut self, level: usize, n: usize, eye: [f32; 3]) {
		let d = (0..3).map(|i| (eye[i] - self.eye[i]).powi(2)).sum::<f32>();
		if level != self.level || n != self.n || !(d <= JUMP * JUMP) {
			self.held.clear();
		}
		(self.level, self.n, self.eye) = (level, n, eye);
	}

	/// Last frame's mirrored heights, in pick order.
	pub(in crate::viewer) fn heights(&self) -> Vec<f32> {
		self.held.iter().map(|p| p.height).collect()
	}

	/// After the pick: `fresh` (heights picked from `boxes`) become the held set, each with every box within 0.4 of it, found
	/// again when a plane of `found` (this frame's estimate) lies within 0.5; then last frame's planes not picked again stay
	/// for `MIRROR_HOLD` frames while a slot of `n` is free.
	pub(in crate::viewer) fn update(&mut self, found: &[[f32; 5]], boxes: &[[f32; 5]], fresh: &[f32], n: usize) {
		let mut now: Vec<Held> = fresh
			.iter()
			.map(|&h| {
				let seen = found.iter().any(|p| (p[0] - h).abs() < 0.5);
				let unfound = if seen { 0 } else { self.held.iter().find(|p| (p.height - h).abs() < 0.5).map_or(0, |p| p.unfound.saturating_add(1)) };
				Held { height: h, boxes: boxes.iter().filter(|b| (b[0] - h).abs() < 0.4).copied().collect(), age: 0, unfound }
			})
			.collect();
		for p in &self.held {
			if now.len() < n && p.age < MIRROR_HOLD && now.iter().all(|q| (q.height - p.height).abs() >= 0.5) {
				now.push(Held { age: p.age + 1, unfound: p.unfound.saturating_add(1), ..p.clone() });
			}
		}
		self.held = now;
	}
}

/// This frame's flat planes (`WaterDraw::plane`).
pub(in crate::viewer) fn found(draws: &[WaterDraw]) -> Vec<[f32; 5]> {
	draws.iter().filter_map(|d| d.plane).collect()
}

/// The boxes of this frame's still water meshes without a flat plane (the meshes that may hold a plane).
pub(in crate::viewer) fn unflat(draws: &[WaterDraw]) -> Vec<[f32; 6]> {
	draws.iter().filter(|d| d.plane.is_none() && d.flow == 0).filter_map(|d| d.bounds).collect()
}

/// This frame's flat planes and the boxes of each held plane that no plane of this frame matches but a mesh holds (`merge`).
pub(in crate::viewer) fn planes(draws: &[WaterDraw], held: &[Held]) -> Vec<[f32; 5]> {
	merge(found(draws), &unflat(draws), held)
}

/// `found` plus, of each of `held` still within `UNFOUND_CAP` that no plane of `found` matches (0.5), every box one box of
/// `unflat` covers.
fn merge(mut out: Vec<[f32; 5]>, unflat: &[[f32; 6]], held: &[Held]) -> Vec<[f32; 5]> {
	let n = out.len();
	for p in held {
		if p.unfound < UNFOUND_CAP && !out[..n].iter().any(|f| (f[0] - p.height).abs() < 0.5) {
			out.extend(p.boxes.iter().filter(|h| unflat.iter().any(|b| covers(b, h))));
		}
	}
	out
}

/// A mesh box (min x, y, z, max x, y, z) covers a plane ([height, x0, y0, x1, y1]): the mesh is low and flat enough
/// (`MAX_RISE`), the plane's height lies in its height range and its XY box (grown by `SLACK`) holds at least `COVER_SHARE` of the
/// plane's XY box. An inverted or NaN box never covers.
fn covers(b: &[f32; 6], p: &[f32; 5]) -> bool {
	let (w, d) = (p[3] - p[1], p[4] - p[2]);
	let rise = b[5] - b[2];
	let low = rise <= MAX_RISE * w.max(d) && rise <= MAX_RISE * 0.5 * (b[3] - b[0]).max(b[4] - b[1]);
	if !(w >= 0.0 && d >= 0.0 && low && p[0] >= b[2] - SLACK && p[0] <= b[5] + SLACK) {
		return false;
	}
	let ox = p[3].min(b[3] + SLACK) - p[1].max(b[0] - SLACK);
	let oy = p[4].min(b[4] + SLACK) - p[2].max(b[1] - SLACK);
	ox >= 0.0 && oy >= 0.0 && ox * oy >= COVER_SHARE * w * d
}

#[cfg(test)]
mod tests;
