//! [PC] Fire: the PC vegetation bends away from the Pyrocitor's flame where it reaches the ground, and grass held in it long enough
//! catches fire (lane nfire; the user: "the grass doesnt react to pyrocitor"). Fire Medium and High only, like the ground fire
//! (Low / Off / Classic: no spot, count 0, the vegetation shader's jet loop never runs).
//!
//! Each display frame `note` takes the nodes of the drawn jets (firefx.rs `begin_sprites`; at most `PAINT_SAMPLES`, 0.4-0.8 m apart,
//! so the whole stream is covered); `probe` (firefx_blast.rs `ground_follow`, which has the collision world) finds the ground under
//! each: a node whose flame (its half width) comes within a metre of the ground touches it (strength 1 - smoothstep(0.2, 1.0, gap)).
//! Every touch heats the burn map under it (a disc of 1.28 x the flame's half width + `veg_burn_reach`; vegfx/burnmap.rs `touch`, through
//! `BlastFire::jet_burn`: only while Vegetation is on): held ~0.15 s it lights, so the burn follows the stream's path. The bending is
//! unchanged: `SAMPLES` of the nodes merge into at most `MAX_SPOTS` spots that fade over `FADE` s of game time after the jet leaves;
//! `fill` writes them into the vegetation uniform's jet rows (vegfx/burn.rs `BurnU::jh` / `jet`; vegfx/shader.rs `veg_touch`): the
//! blades bend away from the spot and along the stream.

use super::firefx_jet::Jet;
use super::vegfx::burn::{BurnU, MAX_JET_SPOTS};

/// Nodes that bend the grass a frame (spread evenly over the jets' chains).
const SAMPLES: usize = 12;
/// Nodes that paint the burn map a frame (every node up to this many, else evenly spread).
const PAINT_SAMPLES: usize = 64;
/// Under this strength a touch only bends (no heat).
const PAINT_MIN: f32 = 0.25;
/// Spots at most (the uniform's rows).
pub(super) const MAX_SPOTS: usize = MAX_JET_SPOTS;
/// Seconds a spot fades over after its last touch.
const FADE: f64 = 0.6;
/// How far below a node the ground is looked for (m).
const DROP: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot {
	pos: [f32; 3],
	reach: f32,
	strength: f32,
	dir: [f32; 2],
	seen: f64,
}

/// One touch of this frame on the burn map: ground point, disc radius (m), strength 0..1.
pub(super) type Stamp = ([f32; 3], f32, f32);

/// The jets' nodes of this frame (place, half width, stream direction, bends the grass) and the touch spots on the ground.
#[derive(Default)]
pub(super) struct JetGrass {
	nodes: Vec<([f32; 3], f32, [f32; 2], bool)>,
	spots: Vec<Spot>,
	/// The game time of the last `step` (the heat's dt).
	last: Option<f64>,
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
	let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
	t * t * (3.0 - 2.0 * t)
}

impl JetGrass {
	/// This frame's jets (`on` = Fire Medium or High with the fire drawn). Off forgets every spot.
	pub(super) fn note(&mut self, jets: &[Jet], on: bool) {
		self.nodes.clear();
		if !on {
			self.spots.clear();
			return;
		}
		let total: usize = jets.iter().map(|j| j.nodes.len()).sum();
		let (every, paint) = (total.div_ceil(SAMPLES).max(1), total.div_ceil(PAINT_SAMPLES).max(1));
		let mut bends = 0;
		for j in jets {
			for (i, n) in j.nodes.iter().enumerate() {
				let bend = i % every == 0 && n.alpha >= 0.05 && bends < SAMPLES;
				if !(bend || (i % paint == 0 && n.alpha >= 0.05)) {
					continue;
				}
				let (a, b) = (j.nodes[i.saturating_sub(1)].pos, j.nodes[(i + 1).min(j.nodes.len() - 1)].pos);
				let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
				let l = dx.hypot(dy);
				let dir = if l > 1e-4 { [dx / l, dy / l] } else { [0.0, 0.0] };
				bends += bend as usize;
				self.nodes.push((n.pos, n.half, dir, bend));
			}
		}
	}

	/// Whether there is anything for `probe` (a noted node or a fading spot).
	pub(super) fn busy(&self) -> bool {
		!(self.nodes.is_empty() && self.spots.is_empty())
	}

	/// Finds the ground under the noted nodes (`ground(from, max_drop)` = the ground's height, None = no ground or water), updates the
	/// spots at game time `now` and returns this frame's touches for the burn map (`Stamp`s; disc = half width + `reach_add`).
	pub(super) fn probe(&mut self, mut ground: impl FnMut([f32; 3], f32) -> Option<f32>, now: f64, reach_add: f32) -> Vec<Stamp> {
		self.spots.retain(|s| now - s.seen <= FADE);
		let mut stamps = Vec::new();
		for &(p, half, dir, bend) in &self.nodes {
			let Some(z) = ground(p, DROP + half) else { continue };
			let strength = 1.0 - smooth(0.2, 1.0, p[2] - half - z);
			if strength <= 0.05 {
				continue;
			}
			if strength >= PAINT_MIN {
				// The old lit disc: 0.8 x the spot reach (1.6 half + 0.4) = 1.28 half + 0.32 (veg_burn_reach).
				stamps.push(([p[0], p[1], z], 1.28 * half + reach_add, strength));
			}
			if !bend {
				continue;
			}
			let (pos, reach) = ([p[0], p[1], z], 1.6 * half + 0.4);
			let hit = self.spots.iter_mut().find(|s| (s.pos[0] - pos[0]).hypot(s.pos[1] - pos[1]) < 0.5 * s.reach.max(reach) && (s.pos[2] - z).abs() < 1.0);
			match hit {
				Some(s) if s.seen == now => {
					(s.reach, s.strength) = (s.reach.max(reach), s.strength.max(strength));
				}
				Some(s) => *s = Spot { pos, reach, strength, dir, seen: now },
				None => {
					let fresh = Spot { pos, reach, strength, dir, seen: now };
					if self.spots.len() < MAX_SPOTS {
						self.spots.push(fresh);
					} else if let Some(old) = self.spots.iter_mut().min_by(|a, b| a.seen.total_cmp(&b.seen)) {
						*old = fresh;
					}
				}
			}
		}
		stamps
	}

	/// Seconds of game time since the last call (the first: one 60 Hz frame; at most 0.1).
	fn step(&mut self, now: f64) -> f32 {
		let dt = self.last.map_or(1.0 / 60.0, |l| (now - l).clamp(0.0, 0.1)) as f32;
		self.last = Some(now);
		dt
	}

	/// The spots into the vegetation uniform at game time `now`: jh.x = count; per spot (place, reach) and (strength x fade, stream
	/// direction xy, 0).
	pub(super) fn fill(&self, u: &mut BurnU, now: f64) {
		let mut n = 0;
		for s in &self.spots {
			let fade = (1.0 - (now - s.seen) / FADE).clamp(0.0, 1.0) as f32;
			if fade <= 0.0 || n == MAX_SPOTS {
				continue;
			}
			u.jet[2 * n] = [s.pos[0], s.pos[1], s.pos[2], s.reach];
			u.jet[2 * n + 1] = [s.strength * fade, s.dir[0], s.dir[1], 0.0];
			n += 1;
		}
		u.jh = [n as f32, 0.0, 0.0, 0.0];
	}
}

impl super::firefx::FireFx {
	/// `ground_follow`'s jet half: the ground under this frame's jet nodes (no world: no ground), the spots, the grass to light.
	pub(super) fn jet_follow(&mut self, world: Option<&crate::physics::World>) {
		let (now, add) = (self.game.now(), crate::devtune::f32("veg_burn_reach", 0.32).max(0.0));
		let dt = self.jet_grass.step(now);
		let stamps = match world {
			// Water (collision surface id 0) is no grass, as for the ground fire.
			Some(w) => self.jet_grass.probe(|from, drop| w.ground_where(from, drop, |_| true).and_then(|(z, _, kind)| (kind & 0x1f != 0).then_some(z)), now, add),
			None => self.jet_grass.probe(|_, _| None, now, add),
		};
		if !stamps.is_empty() && crate::devtune::f32("fire_vol_log", 0.0) > 0.5 {
			let (p, r, st) = stamps[0];
			eprintln!("fire_grass: lit {} touches, first ({:.2} {:.2} {:.2}) r {r:.2} s {st:.2} dt {dt:.3} at {now:.2}", stamps.len(), p[0], p[1], p[2]);
		}
		for (pos, r, strength) in stamps {
			self.blast.jet_burn(pos, r, strength, dt, now);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::super::firefx_jet::Node;
	use super::*;

	fn jet(z: f32) -> Jet {
		let nodes = (0..6).map(|i| Node { pos: [i as f32, 0.0, z], half: 0.3, alpha: 1.0, tint: [1.0; 3], d: i as f32, s: i as f32 / 5.0 }).collect();
		Jet { nodes, len: 5.0, step: 1.0, members: vec![], root: [0.0, 0.0, z], seed: 0.5, d0: 0.0 }
	}

	fn flat(_: [f32; 3], _: f32) -> Option<f32> {
		Some(0.0)
	}

	#[test]
	fn a_jet_along_the_ground_makes_spots_that_fade() {
		let mut g = JetGrass::default();
		g.note(&[jet(0.5)], true);
		assert!(g.busy() && !g.probe(flat, 0.0, 0.25).is_empty(), "the first touch already heats the map");
		let mut u = BurnU::none();
		g.fill(&mut u, 0.0);
		let n = u.jh[0] as usize;
		assert!(n >= 1 && n <= MAX_SPOTS);
		assert!(u.jet[1][0] > 0.9 && (u.jet[1][1] - 1.0).abs() < 1e-5, "full strength, along the stream (+x)");
		// The jet goes: the spots fade over FADE s, then are gone.
		g.note(&[], true);
		g.probe(flat, 0.3, 0.25);
		g.fill(&mut u, 0.3);
		assert!(u.jh[0] as usize == n && u.jet[1][0] < 0.6);
		g.probe(flat, 1.0, 0.25);
		g.fill(&mut u, 1.0);
		assert_eq!(u.jh[0], 0.0);
		assert!(!g.busy());
	}

	#[test]
	fn high_jets_water_and_low_quality_touch_nothing() {
		let mut g = JetGrass::default();
		g.note(&[jet(3.0)], true);
		g.probe(flat, 0.0, 0.25);
		let mut u = BurnU::none();
		g.fill(&mut u, 0.0);
		assert_eq!(u.jh[0], 0.0, "a jet 3 m up does not reach the grass");
		g.note(&[jet(0.5)], true);
		g.probe(|_, _| None, 0.1, 0.25);
		g.fill(&mut u, 0.1);
		assert_eq!(u.jh[0], 0.0, "no ground (water) under it");
		g.note(&[jet(0.5)], true);
		g.probe(flat, 0.2, 0.25);
		g.note(&[jet(0.5)], false);
		g.fill(&mut u, 0.2);
		assert_eq!(u.jh[0], 0.0, "Low / Off forget the spots");
	}

	#[test]
	fn every_node_near_the_ground_paints_and_bending_keeps_its_samples() {
		// A 6-node jet at 0.4 m: every node touches (half width + reach), as many bend as before.
		let mut g = JetGrass::default();
		g.note(&[jet(0.4)], true);
		let st = g.probe(flat, 0.0, 0.25);
		assert_eq!(st.len(), 6, "the whole stream paints");
		assert!(st.iter().all(|(p, r, s)| p[2] == 0.0 && (*r - (1.28 * 0.3 + 0.25)).abs() < 1e-5 && *s > 0.9));
		// A long chain (3 jets of 28): at most PAINT_SAMPLES painted, at most SAMPLES bend.
		let long = |z: f32| {
			let mut j = jet(z);
			j.nodes = (0..28).map(|i| Node { pos: [i as f32 * 0.4, 0.0, z], half: 0.3, alpha: 1.0, tint: [1.0; 3], d: i as f32, s: 0.0 }).collect();
			j
		};
		let mut l = JetGrass::default();
		l.note(&[long(0.4), long(0.4), long(0.4)], true);
		assert!(l.nodes.len() <= PAINT_SAMPLES + SAMPLES && l.nodes.iter().filter(|n| n.3).count() <= SAMPLES);
		assert!(l.probe(flat, 0.0, 0.25).len() >= 40);
		// A stream at hip height (0.7 m under its flame) still heats; 3 m up does not.
		let mut h = JetGrass::default();
		h.note(&[jet(1.0)], true);
		assert!(!h.probe(flat, 0.0, 0.25).is_empty());
		h.note(&[jet(3.0)], true);
		assert!(h.probe(flat, 0.1, 0.25).is_empty());
	}

	#[test]
	fn the_step_is_game_time_and_capped() {
		let mut g = JetGrass::default();
		assert!((g.step(5.0) - 1.0 / 60.0).abs() < 1e-6);
		assert!((g.step(5.05) - 0.05).abs() < 1e-5);
		assert_eq!(g.step(5.05), 0.0, "paused: no heat");
		assert!((g.step(9.0) - 0.1).abs() < 1e-6);
	}
}
