//! [PC] Draw distance (Pause -> Options -> Graphics -> Draw distance; user, 2026-10-17: "rn whole game uses ps2 lods which isnt
//! really necesary on modern pcs", 2026-10-20: a menu row with three levels). The row decides with either Lighting: Classic
//! no longer implies retail's draw distance.
//! - Low: retail. The authored draw distances, the PS2 far LODs and geomorph, the retail world far plane (the renderer's
//!   retail path; nothing here runs).
//! - Medium (the default): every terrain fragment and tie stays on its full-detail mesh (no far LOD, no geomorph) and ties,
//!   shrubs and mobys are drawn `Level::scale` = 4 times farther than their authored draw distances, up to `Level::cap` =
//!   720 m (just inside the retail world far plane `RETAIL_FAR`, 728.16 m, which stays).
//! - High: the same at 12 times, up to 2000 m, with the world far plane pushed out to `HIGH_FAR` (`far_plane`: the world
//!   draws and every pass that reads their depth back). The sky is drawn at the far plane's depth (`xyww`), so it stays behind
//!   everything at any far plane.
//! Shrubs keep their mesh `scale` times longer before the far billboard takes over. Renderer only: nothing here is read from
//! or written to EE RAM. Dev switch: F3 with RAC_DEBUG_KEYS=1 (off = Low, on = the menu's level, for before / after);
//! RAC_FAR_DRAW=0 forces Low.

use crate::ties::TieCull;
use std::sync::atomic::{AtomicBool, Ordering};

/// The world far plane of High (m): past its 2000 m cap, so the farthest drawn spheres are not cut by the far plane.
pub const HIGH_FAR: f32 = 2048.0;

/// A far draw level (Low is no far draw: `on` answers None).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
	Medium,
	High,
}

impl Level {
	/// The Graphics row's level (pcext `Graphics::draw_q`: 0 Low, 1 Medium, 2 High); None = Low (retail).
	pub fn from_q(q: u8) -> Option<Level> {
		match q {
			0 => None,
			1 => Some(Level::Medium),
			_ => Some(Level::High),
		}
	}

	/// How many times farther ties, shrubs and mobys are drawn than their authored distance.
	pub fn scale(self) -> f32 {
		match self {
			Level::Medium => 4.0,
			Level::High => 12.0,
		}
	}

	/// Every far draw stops here (m).
	pub fn cap(self) -> f32 {
		match self {
			Level::Medium => 720.0,
			Level::High => 2000.0,
		}
	}
}

/// The world far plane (m) for the frame's far draw: retail's for Low and Medium (their pictures are today's), `HIGH_FAR` for
/// High.
pub fn far_plane(far: Option<Level>) -> f32 {
	match far {
		Some(Level::High) => HIGH_FAR,
		_ => super::RETAIL_FAR,
	}
}

static OFF: AtomicBool = AtomicBool::new(false);
static INIT: std::sync::Once = std::sync::Once::new();

/// The far draw of this frame: the Graphics row's level `q` (0 Low, 1 Medium, 2 High) over a retail level drawn by rac-rs
/// (`ok`), unless switched off (F3, RAC_FAR_DRAW=0). None = Low: retail's distances.
pub fn on(q: u8, ok: bool) -> Option<Level> {
	INIT.call_once(|| {
		if std::env::var("RAC_FAR_DRAW").is_ok_and(|v| v == "0") {
			OFF.store(true, Ordering::Relaxed);
		}
	});
	if !ok || OFF.load(Ordering::Relaxed) {
		return None;
	}
	Level::from_q(q)
}

/// F3 (dev debug keys only): far draw off (Low) / on (the menu's level). Returns the new state (true = on).
pub fn toggle() -> bool {
	let off = !OFF.load(Ordering::Relaxed);
	OFF.store(off, Ordering::Relaxed);
	eprintln!("far draw {}", if off { "OFF (PS2 distances)" } else { "ON (the Draw distance row)" });
	!off
}

/// The tie choice under far draw `l` at view depth `pz` (m): LOD 0 unmorphed while the sphere's near edge is inside the scaled
/// distance, else None.
pub fn tie(l: Level, c: &TieCull, pz: f32) -> Option<(usize, f32)> {
	let d = (c.dist * l.scale()).min(l.cap());
	(pz - c.radius <= d).then_some((0, 0.0))
}

/// The terrain fragment cut under far draw `l`: the sphere at view depth `pz` with radius `r` (1/1024 m) lies past the cap.
pub fn tfrag_beyond(l: Level, pz: f32, r: f32) -> bool {
	pz - r > l.cap() * 1024.0
}

/// The shrub depth and cap word to hand `ShrubCull::state_cap` so its distances come out `scale` times longer, capped at `cap`.
pub fn shrub(l: Level, pz: f32, cap: f32) -> (f32, f32) {
	(pz / l.scale(), (cap * l.scale()).min(l.cap()) / l.scale())
}

/// A moby's view depth (1/1024 m) as the fade test sees it under far draw `l` (the authored distance and its 500 m clamp scale
/// by `scale`), or None past the cap.
pub fn moby_depth(l: Level, z: f32) -> Option<f32> {
	(z <= l.cap() * 1024.0).then_some(z / l.scale())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn scaled_distances_and_cap() {
		let m = Level::Medium;
		let c = TieCull { centre: [0.0; 3], radius: 2.0, dist: 60.0, lod: [10.0, 20.0, 40.0], occ: crate::occlusion::UNMAPPED };
		assert_eq!(tie(m, &c, 100.0), Some((0, 0.0)));
		assert_eq!(tie(m, &c, 242.0), Some((0, 0.0)));
		assert_eq!(tie(m, &c, 242.5), None);
		let far = TieCull { dist: 1000.0, ..c };
		assert!(tie(m, &far, 722.0).is_some() && tie(m, &far, 722.5).is_none());
		assert_eq!(shrub(m, 400.0, 500.0), (100.0, 180.0));
		assert_eq!(moby_depth(m, 721.0 * 1024.0), None);
		assert_eq!(moby_depth(m, 400.0 * 1024.0), Some(100.0 * 1024.0));
	}

	/// The three levels of the Graphics row: Low = no far draw (retail), Medium = today's 4x / 720 m at the retail far plane,
	/// High = 12x / 2000 m with the far plane past the cap.
	#[test]
	fn levels_of_the_row() {
		assert_eq!([0u8, 1, 2, 3].map(Level::from_q), [None, Some(Level::Medium), Some(Level::High), Some(Level::High)]);
		assert_eq!((Level::Medium.scale(), Level::Medium.cap()), (4.0, 720.0));
		assert_eq!((Level::High.scale(), Level::High.cap()), (12.0, 2000.0));
		assert_eq!(far_plane(None), super::super::RETAIL_FAR);
		assert_eq!(far_plane(Some(Level::Medium)), super::super::RETAIL_FAR);
		assert!(far_plane(Some(Level::High)) > Level::High.cap() + 8.0);
		let h = Level::High;
		let c = TieCull { centre: [0.0; 3], radius: 2.0, dist: 60.0, lod: [10.0, 20.0, 40.0], occ: crate::occlusion::UNMAPPED };
		assert!(tie(h, &c, 722.0).is_some() && tie(h, &c, 722.5).is_none(), "60 m * 12 = 720 m");
		let far = TieCull { dist: 1000.0, ..c };
		assert!(tie(h, &far, 2002.0).is_some() && tie(h, &far, 2002.5).is_none());
		assert!(!tfrag_beyond(h, 1500.0 * 1024.0, 0.0) && tfrag_beyond(Level::Medium, 1500.0 * 1024.0, 0.0));
		assert_eq!(moby_depth(h, 1200.0 * 1024.0), Some(100.0 * 1024.0));
		assert_eq!(moby_depth(h, 2001.0 * 1024.0), None);
	}
}
