//! The smoke quads of a level fire as volumes (a burning building's grey columns and volume sheets). The game draws them as flat
//! alpha-blended world quads, not as pool sprites, so `plan.rs` never sees them. While Smoke is on, every world quad the pack names
//! (`Pack::smoke_quads`, `smoke_rows::QuadRow`) is taken: left out of the quad pass (`VolSmoke::took`, gpu/draw/pass_water.rs), its
//! size, thickness (its vertex alpha) and colour handed to the plume of its emitter (plume.rs: the quads of one emitter are grouped,
//! and each group becomes a stream of puffs rising from the fire, widening and thinning out at the top). Smoke Off: nothing here
//! runs and the quads draw as retail does. A quad out of the smoke range stays a quad.

use super::plume::{self, Plume, Taken};
use super::plume_track::{PlumeTrack, NO_QUAD};
use super::volumes::Raw;
use crate::prims::WorldQuad;
use crate::smoke_rows::{self, QuadRow};

/// The most quads taken in one frame (a burning city block has dozens of fire emitters; the rest stay quads).
pub(super) const MAX_TAKEN: usize = 256;

/// What the smoke quads of one frame became.
#[derive(Default)]
pub(in crate::viewer) struct Gathered {
	/// The volumes: the plumes' puffs.
	pub raw: Vec<Raw>,
	/// Per volume: the index of an effect quad of its plume (the cap gives that quad back when it cuts the volume); `NO_QUAD` for a
	/// plume fading out (the game drew none of its quads).
	pub src: Vec<u32>,
	/// Per volume: the index into `groups` of the plume it belongs to.
	pub grp: Vec<u32>,
	/// Per plume: every effect quad it stands for (the cap or the range band gives all of them back, not only the cut puff's one).
	pub groups: Vec<Vec<u32>>,
	/// Per index of the frame's effect quads: it was taken and is left out of the quad pass. Empty = no quad was taken.
	pub took: Vec<bool>,
}

/// The look numbers (RAC_TUNE, ENV.md).
pub(super) struct Knobs {
	/// Smoke range in metres: a quad whose volume would lie beyond it stays a quad.
	pub range: f32,
	/// Albedo gain on the quad's colour.
	pub albedo: f32,
	/// Gain on the plumes' width.
	pub size: f32,
	/// Gain on the quads' thickness.
	pub thick: f32,
}

impl Knobs {
	pub(super) fn tuned() -> Knobs {
		let t = crate::devtune::f32;
		Knobs { range: t("smoke_range", 150.0), albedo: t("smoke_albedo", 0.75), size: t("smoke_quad_size", 1.0), thick: t("smoke_quad_k", 1.0) }
	}
}

/// The volumes of this frame's smoke quads, the plumes followed from last frame by `track`. `on`: the frame can draw volumes at all
/// (the caller's cheap test); empty when the option is Off, the pack names no quads or the pass is the Classic half of the split
/// view (then `track` is left alone; a gap starts it over). A frame with no quad steps it too: the plumes fade out.
pub(super) fn gather(rows: &[QuadRow], quads: &[WorldQuad], eye: [f32; 3], on: bool, track: &mut PlumeTrack) -> Gathered {
	let level = crate::smoke_option::level();
	if !on || rows.is_empty() || level == 0 || !crate::splitview::fx(true) {
		return Gathered::default();
	}
	gather_with(rows, quads, eye, &Knobs::tuned(), &plume::Knobs::tuned(level), super::super::volfog::frame::clock64(), track)
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
	((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// A quad's centre and the square root of its area (the quad's corners go round the loop: diagonals 0-2 and 1-3).
fn shape(q: &WorldQuad) -> ([f32; 3], f32) {
	let centre = std::array::from_fn(|a| q.pos.iter().map(|p| p[a]).sum::<f32>() * 0.25);
	let diag = |i: usize, j: usize| -> [f32; 3] { std::array::from_fn(|a| q.pos[i][a] - q.pos[j][a]) };
	let (d1, d2) = (diag(2, 0), diag(3, 1));
	let cross = [d1[1] * d2[2] - d1[2] * d2[1], d1[2] * d2[0] - d1[0] * d2[2], d1[0] * d2[1] - d1[1] * d2[0]];
	let area = 0.5 * (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
	(centre, area.sqrt())
}

/// A taken quad's share of its plume: thickness 0 when its four corners are (nearly) transparent or it is a sliver.
fn taken_of(i: usize, q: &WorldQuad, row: &QuadRow, radius: f32, k: &Knobs) -> Taken {
	let top = q.colour.iter().copied().fold([0.0f32; 4], |m, c| if c[3] > m[3] { c } else { m });
	let fade = (top[3] / row.alpha.max(1e-3)).clamp(0.0, 1.5);
	let thick = if fade < 0.02 || radius < 0.05 { 0.0 } else { row.density * fade * k.thick };
	Taken { index: i as u32, pos: q.pos, thick, albedo: [top[0].min(1.0) * k.albedo, top[1].min(1.0) * k.albedo, top[2].min(1.0) * k.albedo], billow: row.billow }
}

/// `gather` with the look numbers and the clock given (the tests).
pub(super) fn gather_with(rows: &[QuadRow], quads: &[WorldQuad], eye: [f32; 3], k: &Knobs, pk: &plume::Knobs, now: f64, track: &mut PlumeTrack) -> Gathered {
	let mut g = Gathered::default();
	let mut taken: Vec<Taken> = Vec::new();
	for (i, q) in quads.iter().enumerate() {
		let Some(row) = smoke_rows::quad_row(rows, crate::prims::plain_tex(q.tex), q.additive) else { continue };
		if taken.len() >= MAX_TAKEN {
			break;
		}
		let (centre, side) = shape(q);
		let radius = side * row.radius;
		if dist(centre, eye) - radius * k.size > k.range {
			continue;
		}
		if g.took.is_empty() {
			g.took = vec![false; quads.len()];
		}
		g.took[i] = true;
		taken.push(taken_of(i, q, row, radius, k));
	}
	let pk = plume::Knobs { width: pk.width * k.size, ..*pk };
	let mut fresh: Vec<Plume> = Vec::new();
	for ix in plume::group(&taken) {
		match Plume::of(&taken, &ix, eye) {
			Some(p) => fresh.push(p),
			// (A group too flat / narrow for a plume has nothing replacing its visible quads: they draw as retail; faded ones stay out.)
			None => ix.iter().filter(|&&i| taken[i].thick > 0.0).for_each(|&i| g.took[taken[i].index as usize] = false),
		}
	}
	for s in track.step(fresh, now) {
		let p = Plume { thick: s.p.thick * s.presence, ..s.p };
		let gi = g.groups.len() as u32;
		g.groups.push(p.src.clone());
		for (j, v) in p.puffs(now, &pk, s.seed).into_iter().enumerate() {
			g.raw.push(v);
			g.grp.push(gi);
			g.src.push(if p.src.is_empty() { NO_QUAD } else { p.src[j % p.src.len()] });
		}
	}
	g
}

#[cfg(test)]
mod tests {
	use super::*;

	const ROWS: [QuadRow; 2] = [
		QuadRow { key: 46, alpha: 0.3125, radius: 0.5, density: 0.8, rise: 1.5, billow: 0.4 },
		QuadRow { key: 47, alpha: 2.0, radius: 0.4, density: 0.6, rise: 1.0, billow: 0.5 },
	];

	const KNOBS: Knobs = Knobs { range: 150.0, albedo: 0.75, size: 1.0, thick: 1.0 };
	const PK: plume::Knobs = plume::Knobs { n: 8, period: 7.0, lean: 0.3, width: 1.0, thick: 1.0, billow: 1.0 };
	const EYE: [f32; 3] = [0.0, -40.0, 2.0];

	/// A retail-shaped smoke quad: corners round the loop, the two middle corners carry the alpha, a 2 x 4 m rectangle at x 0..2, z 0..4.
	fn column(key: usize, additive: bool, alpha: f32, x: f32) -> WorldQuad {
		let c = |a: f32| [0.75, 0.75, 0.75, a];
		WorldQuad { pos: [[x, 5.0, 0.0], [x + 2.0, 5.0, 0.0], [x + 2.0, 5.0, 4.0], [x, 5.0, 4.0]], uv: [[0.0; 2]; 4], colour: [c(0.0), c(0.0), c(alpha), c(alpha)], tex: key, additive }
	}

	fn gather_at(quads: &[WorldQuad], k: &Knobs, now: f64) -> Gathered {
		gather_with(&ROWS, quads, EYE, k, &PK, now, &mut PlumeTrack::default())
	}

	/// One smoke quad: taken, and its plume's puffs stand on it (foot z 0, 4 m tall), as thick as its alpha, coloured by it.
	#[test]
	fn a_smoke_quad_becomes_a_plume_of_puffs_from_its_foot() {
		let g = gather_at(&[column(46, false, 0.3125, 0.0)], &KNOBS, 10.0);
		assert_eq!(g.took, vec![true]);
		assert!(!g.raw.is_empty() && g.raw.len() <= PK.n && g.src.iter().all(|&s| s == 0));
		for r in &g.raw {
			assert!(r.pos[2] >= 0.2 * 4.0 - 1e-4 && r.pos[2] <= 4.0 * 1.25 + 1e-4 && (r.pos[0] - 1.0).abs() < 2.5, "{r:?}");
			assert!(r.thick <= 0.8 + 1e-5 && (r.albedo[0] - 0.5625).abs() < 1e-5 && r.billow == 0.4);
		}
	}

	/// Review LOW-2: a flat sheet gets no plume, so its quad is not taken (it draws as retail).
	#[test]
	fn a_flat_quad_without_a_plume_is_not_left_taken() {
		let mut flat = column(46, false, 0.3125, 0.0);
		flat.pos = [[0.0, 5.0, 1.0], [2.0, 5.0, 1.0], [2.0, 9.0, 1.0], [0.0, 9.0, 1.0]];
		let g = gather_at(&[flat, column(46, false, 0.3125, 20.0)], &KNOBS, 10.0);
		assert_eq!(g.took, vec![false, true]);
	}

	#[test]
	fn thickness_follows_the_alpha_and_a_faded_quad_is_taken_without_a_volume() {
		let full = gather_at(&[column(46, false, 0.3125, 0.0)], &KNOBS, 10.0);
		let half = gather_at(&[column(46, false, 0.15625, 0.0)], &KNOBS, 10.0);
		let sum = |g: &Gathered| g.raw.iter().map(|r| r.thick).sum::<f32>();
		assert!((sum(&half) - 0.5 * sum(&full)).abs() < 0.01 && sum(&full) > 0.0);
		let gone = gather_at(&[column(46, false, 0.0, 0.0)], &KNOBS, 10.0);
		assert_eq!(gone.took, vec![true], "the retail quad would be invisible: it stays out of the quad pass");
		assert!(gone.raw.is_empty());
	}

	#[test]
	fn only_alpha_quads_of_a_named_texture_are_taken_and_indices_line_up() {
		let quads = [column(46, true, 0.3, 0.0), column(48, false, 0.3, 0.0), column(47, false, 2.0, 10.0), column(46 | crate::prims::FX_CLAMP, false, 0.3, 20.0)];
		let g = gather_at(&quads, &KNOBS, 10.0);
		assert_eq!(g.took, vec![false, false, true, true], "the additive flame quad and an unnamed texture stay quads");
		assert!(g.src.iter().any(|&s| s == 2) && g.src.iter().any(|&s| s == 3) && g.src.iter().all(|&s| s == 2 || s == 3), "two fires, two plumes");
	}

	#[test]
	fn a_quad_beyond_the_range_stays_a_quad_and_the_count_is_capped() {
		let far = gather_at(&[column(46, false, 0.3, 500.0)], &KNOBS, 10.0);
		assert!(far.took.is_empty() && far.raw.is_empty());
		let many: Vec<WorldQuad> = (0..MAX_TAKEN + 20).map(|i| column(46, false, 0.3, i as f32 * 0.1)).collect();
		let g = gather_at(&many, &KNOBS, 10.0);
		assert_eq!(g.took.iter().filter(|t| **t).count(), MAX_TAKEN);
		assert_eq!(g.took.len(), many.len());
		assert!(g.raw.len() <= PK.n, "quads side by side are one emitter: one plume");
	}

	#[test]
	fn nothing_is_taken_without_rows_quads_or_the_frame_flag() {
		let q = [column(46, false, 0.3, 0.0)];
		assert!(gather(&[], &q, [0.0; 3], true, &mut PlumeTrack::default()).took.is_empty());
		assert!(gather(&ROWS, &[], [0.0; 3], true, &mut PlumeTrack::default()).took.is_empty());
		assert!(gather(&ROWS, &q, [0.0; 3], false, &mut PlumeTrack::default()).took.is_empty());
	}

	/// The game swaps a fire's sheets as the camera moves (one quad or three here): the plume keeps its seed and eases, so no puff
	/// jumps from one frame to the next (user 2026-11-04: "the smoke flickers and changes positions as u move").
	#[test]
	fn a_sheet_swap_moves_no_puff_at_once() {
		let mut track = PlumeTrack::default();
		let one = [column(46, false, 0.3125, 0.0)];
		let three = [column(46, false, 0.3125, 0.0), column(46, false, 0.3125, 1.2), column(46, false, 0.3125, 2.4)];
		let dt = 1.0 / 60.0;
		let mut last: Option<Gathered> = None;
		for f in 0..240 {
			let quads: &[WorldQuad] = if (f / 30) % 2 == 0 { &one } else { &three };
			let g = gather_with(&ROWS, quads, EYE, &KNOBS, &PK, 10.0 + f as f64 * dt, &mut track);
			assert!(!g.raw.is_empty());
			if let Some(l) = &last {
				for r in &g.raw {
					let near = l.raw.iter().any(|o| (0..3).map(|a| (o.pos[a] - r.pos[a]).powi(2)).sum::<f32>().sqrt() < 0.15 && (o.radius - r.radius).abs() < 0.08);
					assert!(near || r.thick < 0.02, "frame {f}: a puff jumped {r:?}");
				}
			}
			last = Some(g);
		}
	}

	#[test]
	fn the_knobs_scale_size_and_thickness() {
		let q = [column(46, false, 0.3125, 0.0)];
		let base = gather_at(&q, &KNOBS, 10.0);
		let big = gather_at(&q, &Knobs { size: 2.0, ..KNOBS }, 10.0);
		assert_eq!(base.raw.len(), big.raw.len());
		for (a, b) in base.raw.iter().zip(&big.raw) {
			assert!((b.radius - 2.0 * a.radius).abs() < 1e-4 && (b.thick - a.thick).abs() < 1e-5, "{a:?} {b:?}");
		}
		let thin = gather_at(&q, &Knobs { thick: 0.5, ..KNOBS }, 10.0);
		let sum = |g: &Gathered| g.raw.iter().map(|r| r.thick).sum::<f32>();
		assert!((sum(&thin) - 0.5 * sum(&base)).abs() < 0.01);
	}
}
