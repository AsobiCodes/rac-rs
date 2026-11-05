//! [PC] Fire: which burn-type puffs are a burning enemy (firefx_burn.rs: one flame per cluster) and which are a R.Y.N.O. rocket's
//! trail (user 2026-11-04, test build 10: "ryno still have sometimes this bug where it creates this big fire but only sometimes").
//! Retail makes both with the same type-4 ctor in nearly the same orange (the burn puffs decomp/notes/particles.md PAR-6, 0x7000a0ff;
//! the R.Y.N.O. rocket, class 457, update Kalebo FUN_002c82c0: 0x6f00afff, one puff a tick along its path). So for `fire_burn_after`
//! s after a Pyrocitor jet (firefx_jet_quads.rs `burning`, the "sometimes") a salvo's trails were taken as burn puffs, joined into one
//! cluster along the whole trail and drawn as ONE flame as wide as the trail: a wall of fire across the screen.
//! A burning enemy's puffs lie on its body (a blob, many new ones a tick); a trail's lie along the rocket's path (a line). Each frame's
//! burn picks are clustered as firefx_burn.rs does, and a cluster is:
//! - a trail (its puffs draw as retail) when one of its puffs was in a trail last frame, when it is longer than `MAX_LEN` (no burning
//!   enemy is), or, still undecided with at least `MIN_PUFFS` puffs, when it is a line (`thin`);
//! - a flame when one of its puffs was in a flame last frame, or it has at least `MIN_PUFFS` puffs and is no line;
//! - else undecided: too few puffs to tell (a trail's first ticks), drawn as retail this frame.
//!
//! The marks follow the pool slots (stable while a particle lives) and are rebuilt each frame from the slots seen, so a dead puff's
//! mark goes with it.

use std::collections::HashMap;

use super::firefx::tune;
use super::firefx_burn::{clusters, Puff};
use crate::fire_option::FireKind;

/// Fewer puffs than this in a cluster nothing marked yet: undecided (retail this frame).
const MIN_PUFFS: usize = 6;
/// A cluster longer than this (m) is no burning enemy, whatever it was before.
const MAX_LEN: f32 = 4.0;
/// A cluster at least this long (m) and at most `LINE_RATIO` x its length wide is a line (a trail).
const LINE_MIN: f32 = 0.6;
const LINE_RATIO: f32 = 0.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
	Flame,
	Trail,
}

/// The marks of the last frame's burn puffs by pool slot.
#[derive(Default)]
pub(super) struct BurnTrail {
	marks: HashMap<u16, Mark>,
	next: HashMap<u16, Mark>,
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The length along the main axis (from the middle to the farthest point) and the width across it (m) of the points `p`.
fn shape(p: &[[f32; 3]]) -> (f32, f32) {
	if p.is_empty() {
		return (0.0, 0.0);
	}
	let n = p.len() as f32;
	let c: [f32; 3] = std::array::from_fn(|k| p.iter().map(|q| q[k]).sum::<f32>() / n);
	let far = p.iter().copied().max_by(|a, b| dot(sub(*a, c), sub(*a, c)).total_cmp(&dot(sub(*b, c), sub(*b, c)))).unwrap_or(c);
	let l = dot(sub(far, c), sub(far, c)).sqrt();
	if !l.is_finite() || l < 1e-4 {
		return (0.0, 0.0);
	}
	let u = sub(far, c).map(|x| x / l);
	let (mut lo, mut hi, mut w) = (f32::MAX, f32::MIN, 0.0f32);
	for q in p {
		let d = sub(*q, c);
		let t = dot(d, u);
		(lo, hi) = (lo.min(t), hi.max(t));
		let across = sub(d, u.map(|x| x * t));
		w = w.max(dot(across, across).sqrt());
	}
	(hi - lo, 2.0 * w)
}

/// Whether a cluster of this length and width is a line (a rocket's trail).
fn thin(len: f32, wide: f32) -> bool {
	len >= LINE_MIN && wide <= LINE_RATIO * len
}

impl BurnTrail {
	/// Forgets the marks (a swapped world, the fire off, no burn picks).
	pub(super) fn forget(&mut self) {
		self.marks.clear();
	}

	/// Leaves the burn picks of the trails and of the undecided clusters to retail (None); the burning enemies' stay. `pos(si)` is
	/// sprite `si`'s place. Dev knob `fire_burn_trail` 0 = off (every burn pick stays, as before the fix).
	pub(super) fn filter(&mut self, picks: &mut [Option<(FireKind, u16)>], pos: impl Fn(usize) -> [f32; 3]) {
		let at: Vec<usize> = picks.iter().enumerate().filter(|(_, p)| matches!(p, Some((FireKind::Burn, _)))).map(|(si, _)| si).collect();
		if at.is_empty() || tune("fire_burn_trail", 1.0) < 0.5 {
			self.marks.clear();
			return;
		}
		let slot = |si: usize| picks[si].map_or(0, |p| p.1);
		let puffs: Vec<Puff> = at.iter().map(|&si| Puff { pos: pos(si), size: 0.0, colour: [0.0; 4], slot: slot(si) }).collect();
		let of = clusters(&puffs);
		let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
		for (i, &r) in of.iter().enumerate() {
			groups.entry(r).or_default().push(i);
		}
		self.next.clear();
		for g in groups.values() {
			let had = |m: Mark| g.iter().any(|&i| self.marks.get(&puffs[i].slot) == Some(&m));
			let pts: Vec<[f32; 3]> = g.iter().map(|&i| puffs[i].pos).collect();
			let (len, wide) = shape(&pts);
			let mark = if had(Mark::Trail) || len > MAX_LEN {
				Some(Mark::Trail)
			} else if had(Mark::Flame) {
				Some(Mark::Flame)
			} else if g.len() < MIN_PUFFS {
				None
			} else if thin(len, wide) {
				Some(Mark::Trail)
			} else {
				Some(Mark::Flame)
			};
			for &i in g {
				if let Some(m) = mark {
					self.next.insert(puffs[i].slot, m);
				}
				if mark != Some(Mark::Flame) {
					picks[at[i]] = None;
				}
			}
		}
		std::mem::swap(&mut self.marks, &mut self.next);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn burn(slot: u16) -> Option<(FireKind, u16)> {
		Some((FireKind::Burn, slot))
	}

	/// A rocket trail: `n` puffs 0.15 m apart along x, slots from `s0`.
	fn trail(n: usize, s0: u16) -> Vec<([f32; 3], u16)> {
		(0..n).map(|i| ([i as f32 * 0.15, 5.0, 1.0], s0 + i as u16)).collect()
	}

	/// A burning enemy: 20 puffs over a 1 x 1 x 1.4 m body at x 10, slots from `s0`.
	fn body(s0: u16) -> Vec<([f32; 3], u16)> {
		(0..20).map(|i| ([10.0 + (i % 3) as f32 * 0.45, (i / 3 % 3) as f32 * 0.45, 0.5 + (i / 9) as f32 * 0.6], s0 + i as u16)).collect()
	}

	fn run(t: &mut BurnTrail, pts: &[([f32; 3], u16)]) -> Vec<bool> {
		let mut picks: Vec<_> = pts.iter().map(|p| burn(p.1)).collect();
		t.filter(&mut picks, |si| pts[si].0);
		picks.iter().map(|p| p.is_some()).collect()
	}

	#[test]
	fn a_burning_enemy_stays_fire_and_a_trail_goes_retail() {
		let mut t = BurnTrail::default();
		let mut pts = body(100);
		pts.extend(trail(12, 300));
		let kept = run(&mut t, &pts);
		assert!(kept[..20].iter().all(|&k| k), "the burning enemy's puffs are one flame");
		assert!(kept[20..].iter().all(|&k| !k), "the trail's puffs draw as retail");
	}

	#[test]
	fn a_salvo_as_long_as_a_wall_is_never_one_flame() {
		// The user's wall: three trails fanning out 8 m from one muzzle, joined into one cluster at the muzzle.
		let mut pts = Vec::new();
		for (k, dir) in [[1.0f32, 0.0], [0.94, 0.34], [0.94, -0.34]].iter().enumerate() {
			for i in 0..40 {
				let d = i as f32 * 0.2;
				pts.push(([dir[0] * d, dir[1] * d, 1.0], (k * 64 + i) as u16));
			}
		}
		let kept = run(&mut BurnTrail::default(), &pts);
		assert!(kept.iter().all(|&k| !k), "longer than MAX_LEN: retail");
	}

	#[test]
	fn a_young_trail_is_undecided_then_a_trail_for_good() {
		let mut t = BurnTrail::default();
		assert!(run(&mut t, &trail(3, 0)).iter().all(|&k| !k), "too few puffs to tell: retail");
		assert!(run(&mut t, &trail(7, 0)).iter().all(|&k| !k), "a line: a trail");
		// Its tail end shrinks to a short blob-like stub: it stays a trail (the marks follow the slots).
		let stub: Vec<_> = (0..6).map(|i| ([4.0 + (i % 2) as f32 * 0.3, 5.0 + (i / 2) as f32 * 0.3, 1.0], i as u16)).collect();
		assert!(run(&mut t, &stub).iter().all(|&k| !k), "a marked trail stays retail");
	}

	#[test]
	fn a_burn_keeps_its_flame_when_its_puffs_thin_out() {
		let mut t = BurnTrail::default();
		assert!(run(&mut t, &body(100)).iter().all(|&k| k));
		// The burn dies down: three puffs left (fewer than MIN_PUFFS) stay one flame.
		assert!(run(&mut t, &body(100)[..3]).iter().all(|&k| k), "marked flame slots keep the flame");
		// A new puff in a slot nobody marked joins the flame's cluster: still a flame.
		let mut more = body(100)[..3].to_vec();
		more.push(([10.2, 0.2, 0.7], 900));
		assert!(run(&mut t, &more).iter().all(|&k| k));
	}

	#[test]
	fn the_marks_go_with_dead_puffs_and_other_picks_are_untouched() {
		let mut t = BurnTrail::default();
		run(&mut t, &trail(8, 0));
		run(&mut t, &[]);
		// The slots are reused by a burning enemy after a frame without them: no trail mark left over.
		let pts: Vec<_> = body(0);
		assert!(run(&mut t, &pts).iter().all(|&k| k));
		// A non-burn pick passes through.
		let mut picks = vec![Some((FireKind::Flame, 1)), burn(2), None];
		t.filter(&mut picks, |_| [0.0; 3]);
		assert_eq!(picks[0], Some((FireKind::Flame, 1)));
		assert_eq!(picks[2], None);
	}

	#[test]
	fn shape_measures_lines_and_blobs() {
		let (l, w) = shape(&trail(11, 0).iter().map(|p| p.0).collect::<Vec<_>>());
		assert!((l - 1.5).abs() < 1e-4 && w < 1e-4 && thin(l, w));
		let (l, w) = shape(&body(0).iter().map(|p| p.0).collect::<Vec<_>>());
		assert!(!thin(l, w), "a body is no line: {l} x {w}");
		assert_eq!(shape(&[]), (0.0, 0.0));
		assert_eq!(shape(&[[1.0, 2.0, 3.0]; 4]), (0.0, 0.0));
	}
}
