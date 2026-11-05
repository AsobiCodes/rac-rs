//! [PC] Ko-fi drawings: every sheet lies flat on the garage wall that is really there (user 2026-11-04: "why are they floating").
//! The places in roster.rs are hand-picked spots, but the Veldin garage is an arched hall with raised wall panels: a flat sheet
//! at a guessed depth floated in front of the curved wall or sank behind a panel. At the Veldin load the host hands over the
//! visible geometry (terrain fragments and ties, the triangles its fallback collision uses too; host/app.rs `load_level_from`).
//! For each sheet five rays go along its wall normal (centre and corners); the sheet takes the plane the hits span (so it leans
//! with a leaning wall) and moves forward until no probed wall point is in front of it. A sheet whose wall is not found keeps
//! its hand-picked place. Mobys (crates, the ship) are no part of it. `RAC_TUNE kofi_log=1` prints each fit, `kofi_map=1` the
//! flat spots of the four garage walls (for picking new places).

use std::sync::Mutex;

use super::rng::{cos, sin};
use super::Placement;
use crate::physics::World;

/// A sheet on its wall: the centre on the wall (before the lift) and the wall normal toward the room (unit; may lean).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
	pub c: [f32; 3],
	pub n: [f32; 3],
}

/// The fits of the roster's sheets after the last Veldin load (per sheet None = no wall found).
static FITS: Mutex<Option<Vec<Option<Fit>>>> = Mutex::new(None);

/// The garage with a margin: only its triangles are kept.
const BOX_LO: [f32; 3] = [135.0, 83.0, 29.0];
const BOX_HI: [f32; 3] = [158.0, 106.0, 42.0];
/// How far in front of and behind the hand-picked plane the wall is looked for (m).
const FRONT: f32 = 0.6;
const BACK: f32 = 1.2;
/// The corner probes sit at this share of the sheet's half size.
const INSET: f32 = 0.85;
/// A fitted normal turned further than this from the hand-picked one (cosine) means the hits are on different things: the
/// hand-picked normal is kept.
const MAX_TURN_COS: f32 = 0.82;

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
	[a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit(a: [f32; 3]) -> Option<[f32; 3]> {
	let l = dot(a, a).sqrt();
	(l > 1e-6).then(|| scale(a, 1.0 / l))
}

/// The fits of the last Veldin load (None before the first one).
pub fn fits() -> Option<Vec<Option<Fit>>> {
	FITS.lock().ok()?.clone()
}

/// Called by the host for every level it loads, with that level's visible triangles; only Veldin's garage is fitted (the
/// triangles of other levels are not even read).
pub fn fit_walls(level: usize, tris: impl Iterator<Item = [[f32; 3]; 3]>) {
	if level != 0 {
		return;
	}
	let world = World::new(tris.filter(|t| t.iter().any(|v| (0..3).all(|k| (BOX_LO[k]..BOX_HI[k]).contains(&v[k])))), 4.0);
	let (tw, th) = super::sheet_size();
	let fits = super::ROSTER
		.iter()
		.zip(super::placements())
		.map(|(e, p)| {
			let f = fit(&world, &p, tw, th);
			if crate::devtune::f32("kofi_log", 0.0) > 0.5 {
				eprintln!("kofi: {} placed {:?} facing {:.2} -> {:?}", e.name, p.c, p.facing, f);
			}
			f.map(|(fit, _)| fit)
		})
		.collect();
	if crate::devtune::f32("kofi_map", 0.0) > 0.5 {
		map(&world, tw, th);
	}
	if let Ok(mut g) = FITS.lock() {
		*g = Some(fits);
	}
}

/// The wall under sheet `p` (texture `tw` x `th`) and how uneven it is there: the fit and the spread of the probed wall points
/// round the fitted plane (m). None when the centre ray finds no wall.
pub(super) fn fit(world: &World, p: &Placement, tw: usize, th: usize) -> Option<(Fit, f32)> {
	let n0 = [cos(p.facing), sin(p.facing), 0.0];
	let r0 = [-n0[1], n0[0], 0.0];
	let (hw, hh) = (p.w * 0.5 * INSET, p.w * 0.5 * th as f32 / tw as f32 * INSET);
	let probe = |a: f32, b: f32| -> Option<[f32; 3]> {
		let q = [p.c[0] + r0[0] * a * hw, p.c[1] + r0[1] * a * hw, p.c[2] + b * hh];
		world.line(add(q, scale(n0, FRONT)), sub(q, scale(n0, BACK)), 0x10).map(|h| h.p)
	};
	let centre = probe(0.0, 0.0)?;
	let c4 = [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)].map(|(a, b)| probe(a, b));
	let got: Vec<[f32; 3]> = c4.iter().flatten().copied().collect();
	let n = match (c4, got.len()) {
		([Some(tl), Some(tr), Some(br), Some(bl)], _) => unit(cross(sub(tr, bl), sub(tl, br))),
		(_, 3) => unit(cross(sub(got[1], got[0]), sub(got[2], got[0]))).map(|n| if dot(n, n0) < 0.0 { scale(n, -1.0) } else { n }),
		_ => None,
	}
	.filter(|n| dot(*n, n0) >= MAX_TURN_COS)
	.unwrap_or(n0);
	let hits: Vec<[f32; 3]> = std::iter::once(centre).chain(got).collect();
	let mid = scale(hits.iter().fold([0.0; 3], |s, h| add(s, *h)), 1.0 / hits.len() as f32);
	// Along the hand-picked normal onto the fitted plane: the sheet keeps its picked place and height.
	let on = add(p.c, scale(n0, dot(sub(mid, p.c), n) / dot(n0, n)));
	let e: Vec<f32> = hits.iter().map(|h| dot(sub(*h, on), n)).collect();
	let (lo, hi) = e.iter().fold((f32::MAX, f32::MIN), |m, &d| (m.0.min(d), m.1.max(d)));
	Some((Fit { c: add(on, scale(n, hi.max(0.0))), n }, hi - lo))
}

/// `kofi_map`: walks the four garage walls from the middle of the hall (every 0.5 m along, every 0.5 m up) and prints the
/// spots where a 0.9 m sheet lies flat (spread under 2 cm, the wall within 25 degrees of upright).
fn map(world: &World, tw: usize, th: usize) {
	let mid = [146.5, 95.0];
	for facing in [0.0f32, std::f32::consts::FRAC_PI_2, std::f32::consts::PI, -std::f32::consts::FRAC_PI_2] {
		let (n0, r0) = ([cos(facing), sin(facing), 0.0], [-sin(facing), cos(facing), 0.0]);
		for i in -14..=14 {
			for j in 0..12 {
				let (along, z) = (i as f32 * 0.5, 33.0 + j as f32 * 0.5);
				let from = [mid[0] + r0[0] * along, mid[1] + r0[1] * along, z];
				let Some(h) = world.line(from, sub(from, scale(n0, 12.0)), 0x10) else { continue };
				let p = Placement { c: h.p, facing, w: 0.9, tilt: 0.0 };
				if let Some((f, spread)) = fit(world, &p, tw, th) {
					if spread < 0.02 && f.n[2].abs() < 0.42 {
						eprintln!("kofi map: facing {facing:.2} along {along:+.1} z {z:.1} -> c [{:.2}, {:.2}, {:.2}] n [{:.2}, {:.2}, {:.2}]", f.c[0], f.c[1], f.c[2], f.n[0], f.n[1], f.n[2]);
					}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A wall at x = 100 looking toward -x and a sheet placed 0.4 m in front of it (it floated): the fit puts it on the wall.
	#[test]
	fn a_floating_sheet_goes_onto_the_wall() {
		let x = 100.0;
		let world = World::new([[[x, 40.0, 20.0], [x, 60.0, 20.0], [x, 60.0, 40.0]], [[x, 40.0, 20.0], [x, 60.0, 40.0], [x, 40.0, 40.0]]], 4.0);
		let p = Placement { c: [99.6, 50.0, 30.0], facing: std::f32::consts::PI, w: 1.0, tilt: 0.0 };
		let (f, spread) = fit(&world, &p, 1088, 832).expect("the wall is found");
		assert!((f.c[0] - x).abs() < 1e-3 && (f.c[1] - 50.0).abs() < 1e-3 && (f.c[2] - 30.0).abs() < 1e-3, "{f:?}");
		assert!((f.n[0] + 1.0).abs() < 1e-3 && spread < 1e-3, "{f:?}");
		// Sunk 0.3 m behind the wall: pulled forward onto it the same way.
		let (g, _) = fit(&world, &Placement { c: [100.3, 50.0, 30.0], ..p }, 1088, 832).unwrap();
		assert!((g.c[0] - x).abs() < 1e-3, "{g:?}");
	}

	/// A wall that leans back (the arched garage): the sheet takes the wall's slope; a raised strip in the middle pushes it forward
	/// so it never sinks behind it.
	#[test]
	fn a_sheet_follows_a_leaning_wall_and_stays_in_front_of_a_bump() {
		// The plane x = 100 + 0.3 * (z - 30): leaning back toward +x going up; the room is toward -x.
		let at = |y: f32, z: f32| [100.0 + 0.3 * (z - 30.0), y, z];
		let lean = World::new([[at(40.0, 20.0), at(60.0, 20.0), at(60.0, 40.0)], [at(40.0, 20.0), at(60.0, 40.0), at(40.0, 40.0)]], 4.0);
		let p = Placement { c: [99.5, 50.0, 30.0], facing: std::f32::consts::PI, w: 1.0, tilt: 0.0 };
		let (f, _) = fit(&lean, &p, 1088, 832).unwrap();
		let want = unit([-1.0, 0.0, 0.3]).unwrap();
		assert!(dot(f.n, want) > 0.999, "{f:?}");
		assert!((f.c[0] - 100.0).abs() < 1e-3, "{f:?}");
		// A flat wall at x = 100 with a 5 cm raised square round the centre (only the centre ray meets it).
		let (w, b) = (100.0, 99.95);
		let tris = vec![
			[[w, 40.0, 20.0], [w, 60.0, 20.0], [w, 60.0, 40.0]],
			[[w, 40.0, 20.0], [w, 60.0, 40.0], [w, 40.0, 40.0]],
			[[b, 49.9, 29.9], [b, 50.1, 29.9], [b, 50.1, 30.1]],
			[[b, 49.9, 29.9], [b, 50.1, 30.1], [b, 49.9, 30.1]],
		];
		let (g, spread) = fit(&World::new(tris, 4.0), &p, 1088, 832).unwrap();
		assert!(g.c[0] <= b + 1e-3 && (spread - 0.05).abs() < 1e-3, "in front of the bump: {g:?} {spread}");
	}

	#[test]
	fn no_wall_no_fit() {
		let p = Placement { c: [0.0, 0.0, 0.0], facing: 0.0, w: 1.0, tilt: 0.0 };
		assert!(fit(&World::new(std::iter::empty::<[[f32; 3]; 3]>(), 4.0), &p, 1088, 832).is_none());
	}
}
