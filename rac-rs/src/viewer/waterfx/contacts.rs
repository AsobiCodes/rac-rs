//! What touches the water: contact rings (`Rings`) and the movers (`WaterFx::contacts`, `track_props`, `shore_movers`).

use super::*;

/// Contact rings at most; the hero's ring spacing moving and standing still (s); a particle ring's spacing (world units, s).
pub(super) const RINGS: usize = 16;
const HERO_GAP: f32 = 0.3;
const IDLE_GAP: f32 = 1.4;
const PART_GAP: (f32, f32) = (0.5, 0.3);

/// [PC] Contact rings: ripples spreading from where something meets still water (a flat `WaterDraw::plane`). The hero wading
/// or swimming leaves a ring with a froth crest every `HERO_GAP` s while moving (a faint one every `IDLE_GAP` s standing
/// still), and the retail pool particles that touch the surface (splashes, drops, spray) leave smaller ones. Contacts are
/// taken once per game field (`feed`), so a paused game leaves none. Retail draws no such rings; the particles stay as drawn.
#[derive(Default)]
pub(super) struct Rings {
	live: Vec<Ring>,
	/// The last field's contacts not yet placed (point, hero).
	contacts: Vec<([f32; 3], bool)>,
	pub(super) frame: u32,
	hero_last: Option<[f32; 3]>,
	hero_t: f32,
}

#[derive(Clone, Copy, Debug)]
struct Ring {
	p: [f32; 3],
	born: f32,
	slope: f32,
	froth: f32,
	life: f32,
	/// [PC] Heavy splashes (`Wakes`): how far and fast it spreads (1 = a contact ring) and the foam left inside it.
	size: f32,
	patch: f32,
}

impl Rings {
	/// This field's contact candidates: the hero's feet and the pool particles; nothing new while the field count stands still.
	pub(super) fn feed(&mut self, frame: u32, hero: Option<[f32; 3]>, parts: impl Iterator<Item = [f32; 3]>) {
		if frame == self.frame {
			return;
		}
		self.frame = frame;
		self.contacts.clear();
		self.contacts.extend(hero.map(|h| (h, true)));
		self.contacts.extend(parts.take(256).map(|p| (p, false)));
	}

	/// Places the waiting contacts on the flat water `planes` (`WaterDraw::plane`; the hero from 1.6 below to 0.25 above one, a particle within
	/// 0.4 of it, inside its box) at time `now` (s), drops spent rings, and gives the shader rows and count.
	pub(super) fn update(&mut self, now: f32, planes: impl Iterator<Item = [f32; 5]>) -> ([[f32; 4]; 2 * RINGS], usize) {
		self.live.retain(|r| now - r.born < r.life && now >= r.born - 1.0);
		let planes: Vec<[f32; 5]> = planes.collect();
		for (p, hero) in std::mem::take(&mut self.contacts) {
			let (lo, hi) = if hero { (-1.6, 0.25) } else { (-0.4, 0.4) };
			let on = planes.iter().find(|b| p[0] >= b[1] - 0.3 && p[0] <= b[3] + 0.3 && p[1] >= b[2] - 0.3 && p[1] <= b[4] + 0.3 && p[2] - b[0] >= lo && p[2] - b[0] <= hi);
			let Some(b) = on else {
				if hero {
					self.hero_last = None;
				}
				continue;
			};
			let c = [p[0], p[1], b[0]];
			if hero {
				let moved = self.hero_last.map_or(1.0, |q| ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt());
				self.hero_last = Some(p);
				let (gap, slope, froth) = if moved > 0.02 { (HERO_GAP, 0.8, 0.9) } else { (IDLE_GAP, 0.3, 0.2) };
				if now - self.hero_t < gap && now >= self.hero_t {
					continue;
				}
				self.hero_t = now;
				self.push(Ring { p: c, born: now, slope, froth, life: 1.8, size: 1.0, patch: 0.0 }, true);
			} else {
				let near = |r: &Ring| (r.p[0] - c[0]).powi(2) + (r.p[1] - c[1]).powi(2) < PART_GAP.0 * PART_GAP.0 && now - r.born < PART_GAP.1;
				if !self.live.iter().any(near) {
					self.push(Ring { p: c, born: now, slope: 0.4, froth: 0.6, life: 1.2, size: 1.0, patch: 0.0 }, false);
				}
			}
		}
		let mut rows = [[0.0; 4]; 2 * RINGS];
		let mut k = 0;
		for r in self.live.iter().filter(|r| now >= r.born) {
			let age = now - r.born;
			let a = age / r.size.sqrt();
			let rad = (0.15 + 1.3 * a - 0.15 * a * a) * r.size;
			let fade = (1.0 - age / r.life).powi(2);
			rows[2 * k] = [r.p[0], r.p[1], r.p[2], rad];
			rows[2 * k + 1] = [r.slope * fade / (1.0 + 0.5 * rad / r.size), (0.22 + 0.15 * a) * r.size.sqrt(), r.froth * (1.0 - a / 0.9).max(0.0), r.patch * fade];
			k += 1;
		}
		(rows, k)
	}

	/// [PC] Heavy splashes (`Wakes::update`: something came down into still water fast, or a burst of particles hit it): two
	/// big rings, the second a moment later, with a froth crest and foam left where it went in.
	pub(super) fn splash(&mut self, now: f32, p: [f32; 3], size: f32) {
		self.push(Ring { p, born: now, slope: 1.5 * size.min(1.2), froth: 1.0, life: 2.4 + size, size: 1.0 + size, patch: 0.8 }, true);
		self.push(Ring { p, born: now + 0.22, slope: 0.8 * size.min(1.2), froth: 0.7, life: 2.0 + size, size: 0.6 + 0.6 * size, patch: 0.0 }, true);
	}

	/// A new ring; when full, the hero's replaces the oldest and a particle's is dropped.
	fn push(&mut self, r: Ring, hero: bool) {
		if self.live.len() < RINGS {
			self.live.push(r);
		} else if hero {
			if let Some(o) = self.live.iter_mut().min_by(|a, b| a.born.total_cmp(&b.born)) {
				*o = r;
			}
		}
	}
}

impl WaterFx {
	/// This field's contact candidates for the rings (`Rings::feed`) and the wakes (`Wakes::feed`): the game's field count, the
	/// hero's feet, pool particles, and the drawn mobys (id, position).
	pub(in crate::viewer) fn contacts(&mut self, frame: u32, hero: Option<[f32; 3]>, parts: impl Iterator<Item = [f32; 3]>, movers: &[(u32, [f32; 3], f32)]) {
		if frame == self.wakes.frame {
			return;
		}
		// The rings look at the first 256 (v0.2.1); the churn cells at up to PARTS (a waterfall's spray comes late in the list).
		let parts: Vec<[f32; 3]> = parts.take(PARTS).collect();
		self.rings.feed(frame, hero, parts.iter().take(256).copied());
		self.wakes.feed(frame, hero, movers, &parts);
	}

	/// [PC] Geometric sea: this frame's props (`Gpu::set_props`). A moby that moved or turned within SHORE_STILL frames is a mover:
	/// the bed draws it after the shore map (mirror.rs `draw_bed`), so a fish or a boat in the sea makes no surf of its own
	/// (its wake marks it; its bobbing made the surf grow and vanish frame to frame) while a moby that keeps still (a post, a
	/// hull, a crate) keeps its surf.
	pub(in crate::viewer) fn track_props(&mut self, props: &[crate::game::Prop]) {
		self.shore_movers.clear();
		self.shore_still.resize(props.len(), ([f32::NAN; 6], 0));
		for (pi, p) in props.iter().enumerate() {
			let now = [p.pos[0], p.pos[1], p.pos[2], p.rot[0], p.rot[1], p.rot[2]];
			let e = &mut self.shore_still[pi];
			e.1 = if now.iter().zip(&e.0).all(|(a, b)| (a - b).abs() <= SHORE_MOVED) { e.1.saturating_add(1) } else { 0 };
			e.0 = now;
			if p.shown && e.1 < SHORE_STILL {
				self.shore_movers.push(pi);
			}
		}
	}

	/// [PC] Geometric sea: the prop slots the bed draws after the shore map (`track_props`); none without sea waves.
	pub(in crate::viewer) fn shore_movers(&self) -> &[usize] {
		if self.shore_split() { &self.shore_movers } else { &[] }
	}

	/// [PC] Geometric sea: the bed is drawn in two passes round the shore map (movers and the hero after it, mirror.rs `draw_bed`).
	pub(in crate::viewer) fn shore_split(&self) -> bool {
		self.geo_draw.is_some() && crate::devtune::f32("SHORE_MOVERS", 1.0) > 0.5
	}
}
