//! Wakes behind the hero and movers, the churn where falling water lands, and the constants of the contact effects.

/// Wake points at most (each mover's head and the path behind it), the spacing of a path's points (s), how long a wake lasts
/// (s), the movers followed besides the hero, a mover's wake strength (the hero's is 1) and the field id of the hero.
pub(super) const TRAIL: usize = 48;
const TRAIL_GAP: f32 = 0.1;
const WAKE_LIFE: f32 = 2.6;
const MOVERS: usize = 6;
const MOVER_WAKE: f32 = 0.85;
/// A moby's wake starts this share of its bounding radius wide each side, at most MOVER_HALF_MAX.
const MOVER_HALF: f32 = 0.6;
const MOVER_HALF_MAX: f32 = 5.0;
const HERO_ID: u32 = u32::MAX;
/// Churn spots at most, the churn grid (world units), the churn's time constant (s) and the particle hits per second at which
/// a spot starts and is full.
pub(super) const CHURN: usize = 12;
/// The pool particles the churn cells look at each field.
pub(super) const PARTS: usize = 2048;
pub(super) const CHURN_CELL: f32 = 2.0;
const CHURN_TAU: f32 = 2.0;
const CHURN_RATE: (f32, f32) = (60.0, 400.0);
/// The particle hits of one field in one churn cell that make a heavy splash (on top of the spot's own rate).
const BURST: f32 = 10.0;
/// Particles this close to a mover (world units) are its own spray.
const CHURN_SKIP: f32 = 2.5;
/// A waterfall foot's steady churn (hits per second).
const FALL_RATE: f32 = 600.0;
/// [PC] How strongly a river's steep quads (`quad_flows`, opaque in retail) take the falling water (`fs_fall`'s vertex alpha
/// scale: 1.4 = full).
pub(super) const RIVER_FALL_ALPHA: f32 = 1.4;
/// [PC] A river strip's triangles this steep (normal z under this share of its length, about 40 degrees) take the falling
/// water; the steeper ones (under 0.5, 60 degrees) are the fall whose foot churns. Gentler slopes (Novalis's rapids) keep
/// the approved river look.
pub(super) const FALL_STEEP: f32 = 0.75;
/// [PC] Over each waterfall foot (`fs_fall`): FALL_PUFFS mist puffs (half size FALL_PUFF_SIZE world units), FALL_DROPS spray
/// drops (half width FALL_DROP_SIZE); on each foot (at most FALL_BOILS) a boiling foam disc (radius FALL_BOIL_SIZE).
pub(super) const FALL_PUFFS: usize = 6;
pub(super) const FALL_PUFF_SIZE: f32 = 3.0;
pub(super) const FALL_DROPS: usize = 20;
pub(super) const FALL_DROP_SIZE: f32 = 0.04;
pub(super) const FALL_BOIL_SIZE: f32 = 2.5;
pub(super) const FALL_BOILS: usize = 48;
/// [PC] A waterfall foot this far (world units) under a flat water's plane is moved up onto it.
pub(super) const FALL_DEEP: f32 = 4.0;
/// [PC] The waterfall feet in one square this wide (world units) share one set; at most FALL_SPOTS sets a frame.
pub(super) const FALL_PUFF_MERGE: f32 = 6.0;
pub(super) const FALL_SPOTS: usize = 16;
/// A flat water's span (world units) from which it is open sea (`WaterDraw::sea`).
pub(super) const SEA_SIZE: f32 = 120.0;
/// (Lakes start at 48: the 32-unit tiled tanks and pools of Pokitaru / Oltanis stay still like retail.)
pub(super) const LAKE_SIZE: f32 = 48.0;

/// [PC] Wakes, churn and heavy splashes (Water Enhanced; retail draws none of them, its own splash particles stay as drawn).
/// Every game field (`feed`) the hero's feet and the drawn mobys are matched against the water boxes (`WaterDraw::wake_box`;
/// the hero from 1.6 below to 0.25 above the surface, as `Rings`). A mover in the water leaves a path of points (`TRAIL_GAP`
/// apart, kept `WAKE_LIFE` s); `fs_water` draws a V behind it from that path (`wake_at`). Something that comes down into
/// flat water fast (the hero jumping in, a falling moby) makes a heavy splash (`Rings::splash`). The pool particles that
/// hit the water fill a grid of churn cells (hits per second, `CHURN_TAU` s memory): where they keep falling (the spray at a
/// waterfall's foot, rapids throwing spray) the water churns (`churn_at`: turbulence and foam), and a sudden burst of hits
/// (a blast in the water) is a heavy splash too.
#[derive(Default)]
pub(super) struct Wakes {
	tracks: Vec<Track>,
	/// The last field's movers not yet placed (id, position, size) and pool particles, the field count, and its time step (s).
	cands: Vec<(u32, [f32; 3], f32)>,
	pub(super) parts: Vec<[f32; 3]>,
	pub(super) frame: u32,
	dt: f32,
	/// The hero's feet (`swell` stays off near him).
	pub(super) hero: Option<[f32; 3]>,
	cells: std::collections::HashMap<(i32, i32), Cell>,
	/// Where the hero and the movers were in the water lately (their spray never churns).
	own: super::own_spray::OwnPath,
}

struct Track {
	id: u32,
	/// Its path, newest first: (point on the surface, time, motion direction, speed).
	pts: std::collections::VecDeque<([f32; 3], f32, [f32; 2], f32)>,
	/// Where it is now while it moves in the water (point, direction, speed).
	head: Option<([f32; 3], [f32; 2], f32)>,
	last: Option<[f32; 3]>,
	/// Its wake's half width at the mover.
	half: f32,
	/// Last field it was over the water and clear of it (a splash: it came down into it).
	above: bool,
	seen: f32,
}

#[derive(Clone, Copy, Default)]
struct Cell {
	/// Hits per second (decaying), and their position sums decaying the same way.
	rate: f32,
	sum: [f32; 3],
	/// The last burst's time.
	burst: f32,
}

impl Wakes {
	/// This field's movers (the hero's feet, the drawn mobys) and pool particles; nothing new while the field count stands still.
	pub(super) fn feed(&mut self, frame: u32, hero: Option<[f32; 3]>, movers: &[(u32, [f32; 3], f32)], parts: &[[f32; 3]]) {
		if frame == self.frame {
			return;
		}
		let step = frame.wrapping_sub(self.frame);
		self.dt = if (1..=5).contains(&step) { step as f32 / 50.0 } else { 0.0 };
		self.frame = frame;
		self.hero = hero;
		self.cands.clear();
		self.cands.extend(hero.map(|h| (HERO_ID, h, 0.0)));
		// What rides with the hero (Clank, the held item) leaves no wake of its own.
		let near = |p: &[f32; 3]| hero.is_some_and(|h| (h[0] - p[0]).powi(2) + (h[1] - p[1]).powi(2) + (h[2] - p[2]).powi(2) < 2.25);
		self.cands.extend(movers.iter().filter(|m| !near(&m.1)).copied());
		self.parts.clear();
		self.parts.extend_from_slice(parts);
	}

	/// Places the waiting movers and particles on the water `boxes` at time `now` (s): the shader's rows, and the heavy
	/// splashes to make (flat water only: point on the surface, size).
	pub(super) fn update(&mut self, now: f32, boxes: &[[f32; 6]], falls: &[[f32; 3]]) -> (WakeRows, Vec<([f32; 3], f32)>) {
		let mut splashes = Vec::new();
		let dt = std::mem::take(&mut self.dt);
		// A new field: a mover not seen in it (gone, hidden) has no head any more.
		if !self.cands.is_empty() {
			for t in &mut self.tracks {
				t.head = None;
			}
		}
		let inside = |b: &[f32; 6], p: [f32; 3], m: f32| p[0] >= b[0] - m && p[0] <= b[3] + m && p[1] >= b[1] - m && p[1] <= b[4] + m;
		for (id, p, size) in std::mem::take(&mut self.cands) {
			// The hero's feet from 1.6 below to 0.25 above the surface; a moby's origin (a raft's, a fish's) from 2 below to 1.5 above.
			let (lo, hi) = if id == HERO_ID { (1.6, 0.25) } else { (2.0, 1.5) };
			let wet = boxes.iter().find(|b| inside(b, p, 0.3) && p[2] >= b[2] - lo && p[2] <= b[5] + hi);
			let k = match self.tracks.iter().position(|t| t.id == id) {
				Some(k) => k,
				None if wet.is_some() && (id == HERO_ID || self.tracks.iter().filter(|t| t.id != HERO_ID).count() < MOVERS) => {
					self.tracks.push(Track { id, pts: Default::default(), head: None, last: None, half: 0.3, above: false, seen: now });
					self.tracks.len() - 1
				}
				None => continue,
			};
			if wet.is_some() {
				self.own.add(p, now);
			}
			let t = &mut self.tracks[k];
			// The hero's is narrow; a moby's starts at about its width (its bounding sphere, a raft's spans its hull).
			t.half = if id == HERO_ID { 0.3 } else { (size * MOVER_HALF).clamp(0.5, MOVER_HALF_MAX) };
			let v = match t.last {
				Some(l) if dt > 0.0 => [(p[0] - l[0]) / dt, (p[1] - l[1]) / dt, (p[2] - l[2]) / dt],
				_ => [0.0; 3],
			};
			// A jump (a respawn, a cut) is no motion.
			let v = if v[0].abs() + v[1].abs() + v[2].abs() > 60.0 { [0.0; 3] } else { v };
			let speed = (v[0] * v[0] + v[1] * v[1]).sqrt();
			let dir = if speed > 1e-3 { [v[0] / speed, v[1] / speed] } else { [0.0, 0.0] };
			t.head = None;
			if let Some(b) = wet {
				if t.above && v[2] < -4.0 && b[2] == b[5] {
					splashes.push(([p[0], p[1], b[2]], (-v[2] / 8.0).clamp(0.6, 1.6) * if id == HERO_ID { 1.0 } else { MOVER_WAKE }));
				}
				if speed > 0.3 {
					// The wake lies on the surface: a flat water's height, else the hero's feet raised to about the waterline
					// (a moby's origin) within the water's box.
					let surf = if b[2] == b[5] { b[2] } else { (p[2] + if id == HERO_ID { 0.7 } else { 0.0 }).clamp(b[2], b[5]) };
					let w = [p[0], p[1], surf];
					if t.pts.front().is_none_or(|q| now - q.1 >= TRAIL_GAP || now < q.1) {
						t.pts.push_front((w, now, dir, speed));
					}
					t.head = Some((w, dir, speed));
				}
			}
			t.above = wet.is_none() && boxes.iter().any(|b| inside(b, p, 0.0) && p[2] > b[5] + 0.25 && p[2] < b[5] + 8.0);
			t.last = Some(p);
			t.seen = now;
		}
		for t in &mut self.tracks {
			while t.pts.back().is_some_and(|q| now - q.1 > WAKE_LIFE || now < q.1 - 1.0) {
				t.pts.pop_back();
			}
		}
		self.own.prune(now);
		self.tracks.retain(|t| !t.pts.is_empty() || t.head.is_some() || now - t.seen < 1.0);
		// The churn cells: decay, then this field's hits (a particle within the water's box, from its surface to 1 above it;
		// bubbles rising under it do not count).
		if dt > 0.0 {
			let keep = (-dt / CHURN_TAU).exp();
			for c in self.cells.values_mut() {
				c.rate *= keep;
				c.sum = c.sum.map(|s| s * keep);
			}
			let mut field: std::collections::HashMap<(i32, i32), (f32, [f32; 3])> = std::collections::HashMap::new();
			// The spray a mover throws up (the hero swimming) is its wake's, not churn.
			let heads: Vec<[f32; 3]> = self.tracks.iter().filter_map(|t| t.head.map(|h| h.0)).chain(self.hero).collect();
			// ... and so is what it left behind on its way (its spray lies on the water for seconds after it passed).
			for &p in self.parts.iter().filter(|p| !heads.iter().any(|h| (h[0] - p[0]).powi(2) + (h[1] - p[1]).powi(2) < CHURN_SKIP * CHURN_SKIP) && !self.own.owns(**p)) {
				if let Some(b) = boxes.iter().find(|b| inside(b, p, 0.3) && p[2] >= b[2] - 0.15 && p[2] <= b[5] + 1.0) {
					let e = field.entry(((p[0] / CHURN_CELL).floor() as i32, (p[1] / CHURN_CELL).floor() as i32)).or_default();
					e.0 += 1.0;
					e.1 = [e.1[0] + p[0], e.1[1] + p[1], e.1[2] + p[2].min(b[5])];
				}
			}
			// A waterfall's foot on the water churns steadily, spray or not.
			for &f in falls {
				if let Some(b) = boxes.iter().find(|b| inside(b, f, 0.5) && f[2] >= b[2] - 1.0 && f[2] <= b[5] + 2.0) {
					let k = FALL_RATE * dt;
					let e = field.entry(((f[0] / CHURN_CELL).floor() as i32, (f[1] / CHURN_CELL).floor() as i32)).or_default();
					e.0 += k;
					e.1 = [e.1[0] + f[0] * k, e.1[1] + f[1] * k, e.1[2] + f[2].min(b[5]) * k];
				}
			}
			for (key, (n, s)) in field {
				let c = self.cells.entry(key).or_default();
				if n >= BURST + 4.0 * c.rate * dt && now - c.burst > 1.0 {
					c.burst = now;
					let at = [s[0] / n, s[1] / n, s[2] / n];
					if let Some(b) = boxes.iter().find(|b| b[2] == b[5] && inside(b, at, 0.0) && (at[2] - b[2]).abs() < 1.0) {
						splashes.push(([at[0], at[1], b[2]], (n / 20.0).clamp(0.6, 1.6)));
					}
				}
				c.rate += n / CHURN_TAU;
				c.sum = [c.sum[0] + s[0] / CHURN_TAU, c.sum[1] + s[1] / CHURN_TAU, c.sum[2] + s[2] / CHURN_TAU];
			}
			self.cells.retain(|_, c| c.rate > 0.05);
		}
		(self.rows(now), splashes)
	}

	/// The shader's rows: per wake point (x, y, surface z, age), (half width at the mover, 0, speed, strength when the next
	/// point is the same mover's, else 0), their box grown by the widest wake and the furthest drift, and per churn spot (x, y, z, radius),
	/// (strength, 0, 0, 0).
	pub(super) fn rows(&self, now: f32) -> WakeRows {
		let mut r = WakeRows { trail: [[0.0; 4]; 2 * TRAIL], n: 0, wbox: [0.0; 4], churn: [[0.0; 4]; 2 * CHURN], cn: 0 };
		let mut bx = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
		for t in &self.tracks {
			let (str, half) = (if t.id == HERO_ID { 1.0 } else { MOVER_WAKE }, t.half);
			let head = t.head.map(|(p, d, s)| (p, now, d, s));
			let pts: Vec<_> = head.into_iter().chain(t.pts.iter().copied().filter(|q| head.is_none_or(|h| q.0 != h.0))).collect();
			if pts.len() < 2 || r.n + 2 > TRAIL {
				continue;
			}
			let take = pts.len().min(TRAIL - r.n);
			for (k, q) in pts.iter().take(take).enumerate() {
				let age = (now - q.1).max(0.0);
				r.trail[2 * r.n] = [q.0[0], q.0[1], q.0[2], age];
				r.trail[2 * r.n + 1] = [half, 0.0, q.3, if k + 1 == take { 0.0 } else { str }];
				let grow = half + 1.3 + age * (0.35 * q.3).min(2.2);
				bx = [bx[0].min(q.0[0] - grow), bx[1].min(q.0[1] - grow), bx[2].max(q.0[0] + grow), bx[3].max(q.0[1] + grow)];
				r.n += 1;
			}
		}
		if r.n > 0 {
			let g = 6.0 * WAKE_LIFE;
			r.wbox = [bx[0] - g, bx[1] - g, bx[2] + g, bx[3] + g];
		}
		let mut cells: Vec<_> = self.cells.values().filter(|c| c.rate >= CHURN_RATE.0).collect();
		cells.sort_by(|a, b| b.rate.total_cmp(&a.rate));
		for c in cells.into_iter().take(CHURN) {
			let at = c.sum.map(|v| v / c.rate);
			r.churn[2 * r.cn] = [at[0], at[1], at[2], CHURN_CELL * 1.4];
			r.churn[2 * r.cn + 1] = [((c.rate - CHURN_RATE.0) / (CHURN_RATE.1 - CHURN_RATE.0)).clamp(0.0, 1.0), 0.0, 0.0, 0.0];
			r.cn += 1;
		}
		r
	}
}

/// What `Wakes` gives the shader (`WaterU`).
pub(super) struct WakeRows {
	pub(super) trail: [[f32; 4]; 2 * TRAIL],
	pub(super) n: usize,
	pub(super) wbox: [f32; 4],
	pub(super) churn: [[f32; 4]; 2 * CHURN],
	pub(super) cn: usize,
}
