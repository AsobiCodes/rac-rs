//! [PC] Fire, ground fire after explosions (stage 2 of the v0.5 fire extras): where an explosion lands on the ground a scorch mark with a
//! bed of embers and a few flame tongues stays for a while. The explosion itself keeps its retail look.
//! CPU half (pure, tested): `BlastFire::watch` finds the explosions (pool sprites of a `FireKind::Blast` row, new since the last frame,
//! and the pack's `fxpool::BlastPoint`s: explosions without such sprites, R&C1's Bomb Glove bombs, rac1/retail/bomb_blast.rs),
//! `land` puts each on the ground (a probe into the collision world: nothing for mid-air blasts, water or steep faces) and keeps the
//! patches (`GroundFirePatch`, at most `MAX_PATCHES`), `flames` builds the tongues and ember emitters of the burning ones.
//! The GPU half (firefx_blast_gpu.rs, groundfire.wgsl) draws the scorch and the embers into the depth the level wrote.
//!
//! Stage 3 (burning grass, vegfx.rs) API: `FireFx::ground_patches()` is the live list (expired patches are dropped by the next `land`;
//! `GroundFirePatch::phase(now, &Life::tuned())` is None for those), `FireFx.game.now()` (firefx_gametime.rs: GAME time, standing while
//! the game stands) is `now`, and `FireFx::patch_buffer()` is the GPU copy (firefx_blast_gpu.rs: `ROWS`, the layout of its first rows).
//! Fire Off / Low and the Classic half: the list stays empty.
//! Everything is forgotten (and the next call only primes `seen`) on Fire Off / Low / Classic, a level change, a frame-counter jump of the
//! effects pool (respawn, savestate load), a gap with no pool and the warp (`FireFx::forget`, firefx_gametime.rs).
//! Dev knobs (`RAC_TUNE`, ENV.md): gfire_radius gfire_h gfire_burn gfire_scorch gfire_max gfire_dark gfire_reach gfire_min_size
//! gfire_glow gfire_tol gfire_hero_r fire_blast55 gfire_log.

use super::firefx::{dist, tongue, Emit, FireFx};
use super::vegfx::burn::{BurnU, Times};
use super::vegfx::burnmap::BurnMap;
use super::*;
use crate::fire_option;

/// Hard cap of live patches (the shader block holds this many).
pub(super) const MAX_PATCHES: usize = 16;
/// Water surfaces kept for the ocean test (a level draws a handful).
const MAX_WATER: usize = 24;

fn tune(name: &str, d: f32) -> f32 {
	crate::devtune::f32(name, d)
}

/// The knobs, read once a call (the lookups are map reads).
#[derive(Clone, Copy, Debug)]
struct Tune {
	radius: f32,
	h: f32,
	max: usize,
	reach: f32,
	min_size: f32,
	bursts: bool,
}

/// Dev knob `gfire_log` (ENV.md): the pack's blasts and every landing try on stderr.
fn log_on() -> bool {
	tune("gfire_log", 0.0) > 0.5
}

fn tuned() -> Tune {
	Tune {
		radius: tune("gfire_radius", 1.4),
		h: tune("gfire_h", 0.9),
		max: (tune("gfire_max", 8.0).max(1.0) as usize).min(MAX_PATCHES),
		reach: tune("gfire_reach", 1.5),
		min_size: tune("gfire_min_size", 0.6),
		bursts: tune("fire_blast55", 0.0) > 0.5,
	}
}

/// Where an explosion landed: the ground point, the face normal, the size of the fire bed, when it started (the fire's game time, s)
/// and a stable seed for the patterns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::viewer) struct GroundFirePatch {
	pub pos: [f32; 3],
	pub normal: [f32; 3],
	pub radius: f32,
	pub born: f64,
	pub seed: f32,
}

/// How long (s) a patch flames and how long its scorch mark stays after that.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::viewer) struct Life {
	pub burn: f32,
	pub scorch: f32,
}

impl Life {
	pub(in crate::viewer) fn tuned() -> Life {
		Life { burn: tune("gfire_burn", 3.0).max(0.5), scorch: tune("gfire_scorch", 8.0).max(0.5) }
	}
}

/// The strength (0..1) of a patch's flames and of its scorch mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::viewer) struct Phase {
	pub flame: f32,
	pub scorch: f32,
}

fn ss(a: f32, b: f32, x: f32) -> f32 {
	let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
	t * t * (3.0 - 2.0 * t)
}

impl GroundFirePatch {
	/// The strengths at time `now` (fire clock age, s); None before the start or once the scorch is gone. The flames rise in 0.2 s and
	/// die down from 45 % of the burn time; the scorch rises in 0.5 s and fades after the burn time.
	pub(in crate::viewer) fn phase(&self, now: f64, life: &Life) -> Option<Phase> {
		let a = (now - self.born) as f32;
		if !(0.0..life.burn + life.scorch).contains(&a) {
			return None;
		}
		Some(Phase { flame: ss(0.0, 0.2, a) * (1.0 - ss(0.45 * life.burn, life.burn, a)), scorch: ss(0.0, 0.5, a) * (1.0 - ss(life.burn, life.burn + life.scorch, a)) })
	}
}

/// A well-mixed 0..1 number from a seed and an index (integer hash; the same every run).
fn h01(seed: u32, i: u32) -> f32 {
	let mut x = seed.wrapping_mul(2654435761) ^ i.wrapping_mul(2246822519).wrapping_add(374761393);
	x ^= x >> 15;
	x = x.wrapping_mul(739982445);
	x ^= x >> 12;
	x = x.wrapping_mul(696491321);
	x ^= x >> 15;
	(x >> 8) as f32 / 16_777_216.0
}

/// One blast-type pool sprite of this frame: the pool slot and effect type (its identity), where, how big, and whether it is big and
/// visible enough to start a fire (`big`: size at the knob, alpha above the draw threshold).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cand {
	slot: u32,
	ty: u8,
	pos: [f32; 3],
	size: f32,
	big: bool,
}

/// What `BlastFire::note` keeps per blast-type sprite: its identity, where it was at the last call, whether it has started a fire
/// (a primed-in sprite counts as started) and when (game time, s).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Entry {
	slot: u32,
	ty: u8,
	pos: [f32; 3],
	triggered: bool,
	t: f64,
}

/// A pool slot taken over by a NEW explosion of the same type shows as a jump of the sprite: farther than this plus half its size.
const REBIRTH_HOP: f32 = 1.0;
/// ... and only this long (s) after the sprite last started a fire.
const REBIRTH_AFTER: f64 = 0.4;

/// An explosion that appeared since the last call and waits for `land`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Blast {
	pos: [f32; 3],
	size: f32,
}

/// The explosions seen so far and the patches they made.
#[derive(Default)]
pub(in crate::viewer) struct BlastFire {
	patches: Vec<GroundFirePatch>,
	/// The blast-type sprites of the previous call (every one, the gate only decides the trigger); the first call after a reset only fills it.
	seen: Vec<Entry>,
	primed: bool,
	pending: Vec<Blast>,
	/// The level the list belongs to; None while off.
	level: Option<usize>,
	eye: [f32; 3],
	hero: Option<[f32; 3]>,
	made: u32,
	/// Last frame's flat water surfaces as (min x, min y, max x, max y, top z): the oceans that are no collision faces (`set_water`).
	water: Vec<[f32; 5]>,
	/// Stage 3 (vegfx/burnmap.rs): the burn map of the PC vegetation (ground fires and the Pyrocitor's flame paint it); outlives the
	/// patches (grass stays burnt ~60 s). Fed only while Vegetation is on (`burn_feed`), cleared with the patches.
	burnt: BurnMap,
	burn_feed: bool,
	/// The GPU half (firefx_blast_gpu.rs), made at the first draw.
	pub(super) res: std::cell::OnceCell<super::firefx_blast_gpu::GroundRes>,
}

/// The `ty` of a pack blast (`fxpool::BlastPoint`, its `id` as the slot): no effect type of the pool, so its entries never meet a sprite's.
const PACK_TY: u8 = 0xff;

/// The pool sprites of a blast type this frame (every one, big or not), in the order of the pool, then the pack's blasts (always big).
/// `rows` = the level's fire rows.
fn candidates(rows: &[fire_option::FireRow], sprites: &[crate::prims::Sprite], types: &[u8], slots: &[u16], blasts: &[crate::fxpool::BlastPoint], t: &Tune) -> Vec<Cand> {
	let mut n = 0;
	let mut out = Vec::new();
	for s in sprites.iter().filter(|s| s.pool.is_some()) {
		let at = n;
		n += 1;
		let Some(&ty) = types.get(at) else { continue };
		if !fire_option::blast_type(rows, ty, t.bursts) {
			continue;
		}
		let big = s.size >= t.min_size && s.colour[3] > 0.004;
		out.push(Cand { slot: slots.get(at).copied().unwrap_or(at as u16) as u32, ty, pos: s.pos, size: s.size, big });
	}
	out.extend(blasts.iter().map(|b| Cand { slot: b.id, ty: PACK_TY, pos: b.pos, size: b.size, big: true }));
	out
}

impl BlastFire {
	/// Forgets everything (Off / Low / Classic, another level, a swapped world: `FireFx::forget`); the next `note` only primes.
	pub(super) fn reset(&mut self) {
		self.patches.clear();
		self.seen.clear();
		self.pending.clear();
		self.primed = false;
		self.level = None;
		self.hero = None;
		self.water.clear();
		self.burnt.clear();
		self.burn_feed = false;
	}

	/// Stage 3: this frame's burnt-area uniform for the vegetation shaders (count 0 with no area).
	fn burn_u(&self, now: f64) -> BurnU {
		if self.burnt.is_empty() {
			return BurnU::none();
		}
		self.burnt.pack(now, &Times::tuned(Life::tuned().burn))
	}

	/// Last frame's water surfaces (a box per flat water draw, `FireFx::set_water`); only kept while the fire is on.
	fn set_water(&mut self, boxes: impl Iterator<Item = [f32; 5]>) {
		self.water.clear();
		if self.level.is_some() {
			self.water.extend(boxes.take(MAX_WATER));
		}
	}

	/// Is this ground point under a known water surface (inside its box, not above its top)? The RAM oceans of Rilgar, Umbris and
	/// Gaspar (and the mud and lava ones) are drawn meshes, not collision faces: the seabed under them is found by the probe.
	fn under_water(&self, p: [f32; 3]) -> bool {
		self.water.iter().any(|w| p[0] >= w[0] && p[0] <= w[2] && p[1] >= w[1] && p[1] <= w[3] && p[2] < w[4] + 0.05)
	}

	/// Every frame instead of `watch` while Fire is Medium or High but the ground fire after explosions is switched off (the dev knob
	/// `fire_ground_blast` 0; the default is on): no patches, only the burnt areas the Pyrocitor's held stream lights (`jet_burn`), fed while Vegetation is on;
	/// another level forgets them.
	pub(in crate::viewer) fn burn_only(&mut self, level: usize) {
		if self.level != Some(level) {
			self.reset();
			self.level = Some(level);
		}
		if !(self.patches.is_empty() && self.pending.is_empty() && self.seen.is_empty()) || self.primed {
			self.patches.clear();
			self.pending.clear();
			self.seen.clear();
			self.primed = false;
		}
		self.burn_feed = super::vegfx::want() > 0;
		if !self.burn_feed {
			self.burnt.clear();
		}
	}

	/// Every frame, in `FireFx::begin_sprites`: `on` = Fire at Medium or High, drawn this frame; `now` = the fire's game time
	/// (firefx_gametime.rs). Off (or another level) forgets every patch; the first call after that only notes what is on screen (a
	/// savestate loaded in an explosion starts no fire).
	pub(in crate::viewer) fn watch(&mut self, on: bool, level: usize, sprites: &[crate::prims::Sprite], fx: Option<&crate::fxpool::PoolFx<'_>>, eye: [f32; 3], now: f64) {
		if !on {
			if self.level.is_some() || !self.patches.is_empty() || self.primed {
				self.reset();
			}
			return;
		}
		if self.level != Some(level) {
			self.reset();
			self.level = Some(level);
		}
		self.eye = eye;
		self.burn_feed = super::vegfx::want() > 0;
		if !self.burn_feed {
			self.burnt.clear();
		}
		let Some(fx) = fx else {
			self.seen.clear();
			self.pending.clear();
			self.primed = false;
			return;
		};
		self.hero = fx.hero;
		let rows = (crate::pack::get().fire_rows)(level);
		if !fx.blasts.is_empty() && log_on() {
			eprintln!("gfire: t {now:.2} pack blasts {:?}", fx.blasts);
		}
		let cands = candidates(rows, sprites, fx.sprite_types, fx.sprite_slots, fx.blasts, &tuned());
		self.note(&cands, now);
	}

	/// Every blast-type sprite is followed by (slot, type); each starts ONE fire, the first time it is `big` (a flipbook that dips under the
	/// gate and comes back does not light again; one that starts small lights when it passes the gate). A slot taken over by a new
	/// explosion of the same type shows as a jump of the sprite (more than `REBIRTH_HOP` + half its size, `REBIRTH_AFTER` s after it last
	/// started a fire): that one is new again. After a reset the first call only primes (everything on screen counts as started).
	fn note(&mut self, cands: &[Cand], now: f64) {
		let old = std::mem::take(&mut self.seen);
		for c in cands {
			let mut e = match old.iter().find(|e| e.slot == c.slot && e.ty == c.ty) {
				Some(&prev) => {
					let hopped = dist(prev.pos, c.pos) > REBIRTH_HOP + 0.5 * c.size;
					Entry { pos: c.pos, triggered: prev.triggered && !(hopped && now - prev.t >= REBIRTH_AFTER), ..prev }
				}
				None => Entry { slot: c.slot, ty: c.ty, pos: c.pos, triggered: !self.primed, t: now },
			};
			if !e.triggered && c.big {
				if self.pending.len() < 8 {
					self.pending.push(Blast { pos: c.pos, size: c.size });
				}
				(e.triggered, e.t) = (true, now);
			}
			self.seen.push(e);
		}
		self.primed = true;
	}

	/// Is there anything for `land` and the draw to do: blasts waiting, patches alive, grass burnt? (`ground_follow` returns first when not.)
	fn has_work(&self) -> bool {
		!(self.pending.is_empty() && self.patches.is_empty() && self.burnt.is_empty())
	}

	/// Puts the pending blasts on the ground. `probe(from, max_drop)` = the highest ground at or below `from` within `max_drop` as
	/// (height, normal), None for no ground or water. Also drops the patches that expired.
	pub(in crate::viewer) fn land(&mut self, probe: &mut dyn FnMut([f32; 3], f32) -> Option<(f32, [f32; 3])>, now: f64) {
		let (t, life) = (tuned(), Life::tuned());
		self.patches.retain(|p| p.phase(now, &life).is_some());
		if !self.burnt.is_empty() {
			self.burnt.expire(now, &Times::tuned(life.burn));
		}
		let mut pend = std::mem::take(&mut self.pending);
		pend.sort_by(|a, b| b.size.total_cmp(&a.size));
		let log = !pend.is_empty() && log_on();
		for b in pend {
			let hit = probe([b.pos[0], b.pos[1], b.pos[2] + 0.4], t.reach + 0.3 * b.size + 0.4);
			if log {
				eprintln!("gfire: t {now:.2} land {:?} size {:.2} ground {hit:?} wet {}", b.pos, b.size, hit.is_some_and(|(z, _)| self.under_water([b.pos[0], b.pos[1], z])));
			}
			let Some((z, n)) = hit else { continue };
			if n[2] < 0.5 || self.under_water([b.pos[0], b.pos[1], z]) {
				continue;
			}
			self.ignite([b.pos[0], b.pos[1], z], n, t.radius * (b.size / 3.0).clamp(0.6, 1.5), now, t.max);
		}
	}

	/// The Pyrocitor's flame on the grass (firefx_jet_grass.rs) heats the burn map under it (disc `radius`, `strength` 0..1, for `dt`
	/// s; only while Vegetation is on); held a few frames, it lights.
	pub(super) fn jet_burn(&mut self, pos: [f32; 3], radius: f32, strength: f32, dt: f32, now: f64) {
		if self.burn_feed {
			self.burnt.touch(pos, radius, strength, dt, now, &Times::tuned(Life::tuned().burn));
		}
	}

	/// The burn map (the camera keeps it centred: `FireFx::burn_follow`; the vegetation uploads it).
	pub(in crate::viewer) fn burn_map(&mut self) -> &mut BurnMap {
		&mut self.burnt
	}

	/// Adds a patch, or refreshes the one it lies on: the same explosion (a patch younger than 0.5 s) changes nothing, a later blast
	/// lights it again at a quarter second into its life (flames at full, no flicker out). Over the cap the farthest from the eye goes.
	fn ignite(&mut self, pos: [f32; 3], normal: [f32; 3], radius: f32, now: f64, cap: usize) {
		if self.burn_feed {
			// The patch's disc plus the old area margin (veg_burn_margin), lit at once.
			let r = 0.95 * (radius + tune("veg_burn_margin", 0.4).max(0.0));
			self.burnt.ignite(pos, r, now, &Times::tuned(Life::tuned().burn));
		}
		let near = |p: &GroundFirePatch| (p.pos[0] - pos[0]).hypot(p.pos[1] - pos[1]) < 0.9 * p.radius.max(radius) && (p.pos[2] - pos[2]).abs() < 1.0;
		if let Some(p) = self.patches.iter_mut().find(|p| near(p)) {
			if now - p.born >= 0.5 {
				p.born = now - 0.25;
				p.radius = p.radius.max(radius);
			}
			return;
		}
		self.made += 1;
		self.patches.push(GroundFirePatch { pos, normal, radius, born: now, seed: h01(self.made, 7) });
		while self.patches.len() > cap.min(MAX_PATCHES) {
			let far = (0..self.patches.len()).max_by(|&a, &b| dist(self.patches[a].pos, self.eye).total_cmp(&dist(self.patches[b].pos, self.eye)));
			self.patches.remove(far.unwrap_or(0));
		}
	}

	/// Forgets pending blasts (no collision world to put them on).
	pub(in crate::viewer) fn drop_pending(&mut self) {
		self.pending.clear();
	}

	pub(in crate::viewer) fn patches(&self) -> &[GroundFirePatch] {
		&self.patches
	}

	pub(super) fn hero(&self) -> Option<[f32; 3]> {
		self.hero
	}

	/// The tongues (`quads`, 4 vertices each, at most `quad_room`) and ember emitters (`ground`, at most `emit_room`) of the patches that
	/// still flame: 3 tongues each at Medium, 5 at High; the first two also get an emitter (embers, and the High light).
	pub(super) fn flames(&self, now: f64, level: u8, across: [f32; 3], quads: &mut Vec<Vertex>, ground: &mut Vec<Emit>, quad_room: usize, emit_room: usize) {
		if self.patches.is_empty() {
			return; // (no knob lookups at Medium / High with nothing burning)
		}
		let (t, life) = (tuned(), Life::tuned());
		let (mut quad_room, mut emit_room) = (quad_room, emit_room);
		for p in &self.patches {
			let Some(ph) = p.phase(now, &life) else { continue };
			let f = ph.flame.powf(0.8);
			if f < 0.02 {
				continue;
			}
			let (t1, t2) = tangents(p.normal);
			let key = (p.seed * 65536.0) as u32;
			for i in 0..if level >= 3 { 5u32 } else { 3 } {
				if quad_room == 0 {
					return;
				}
				let (ang, rr, hh) = (h01(key, i * 3) * std::f32::consts::TAU, 0.55 * p.radius * h01(key, i * 3 + 1).sqrt(), h01(key, i * 3 + 2));
				let h = t.h * (0.55 + 0.6 * hh) * f;
				if h < 0.1 {
					continue;
				}
				let base = std::array::from_fn(|k| p.pos[k] + t1[k] * ang.cos() * rr + t2[k] * ang.sin() * rr);
				let seed = (p.seed + i as f32 * 0.137).fract();
				quads.extend(tongue(base, across, 0.7 * h, h, [1.0, 0.6, 0.2], f, seed, 1.0));
				quad_room -= 1;
				if i < 2 && h >= 0.15 && emit_room > 0 {
					ground.push(Emit { base, h, w: 0.7 * h, seed });
					emit_room -= 1;
				}
			}
		}
	}
}

/// Two unit axes in the plane with normal `n`.
pub(super) fn tangents(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
	let a = if n[2].abs() > 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
	let t1 = normalise(cross(n, a));
	(t1, cross(n, t1))
}

impl FireFx {
	/// Stage 3, every frame before the level pass (`draw/prepare.rs`): the burnt areas for the vegetation shaders; count 0 (nothing to
	/// do, nothing burnt) with Fire Low / Off, Classic lighting or Vegetation Off.
	/// Every frame before `burn_rows`: keeps the burn map around the camera (vegfx/burnmap.rs `follow`) and hands it to the
	/// vegetation for its upload.
	pub(in crate::viewer) fn burn_follow(&mut self, eye: [f32; 3]) -> &mut BurnMap {
		self.blast.burnt.follow(eye);
		self.blast.burn_map()
	}

	pub(in crate::viewer) fn burn_rows(&self) -> BurnU {
		// (The Classic half of the split view draws the retail grass: count 0 = the old code path.)
		if !crate::splitview::fx(true) {
			return BurnU::none();
		}
		let mut u = self.blast.burn_u(self.game.now());
		self.jet_grass.fill(&mut u, self.game.now());
		u
	}

	/// Every frame the level pass made water draws (`draw_passes`): their flat surfaces (a box up to 1 m tall counts as flat) become
	/// the ocean test of `land`: no ground fire on a seabed under water. Kept only while Fire is on.
	pub(in crate::viewer) fn set_water(&mut self, draws: &[super::waterfx::WaterDraw]) {
		self.blast.set_water(draws.iter().filter_map(|d| d.bounds).filter(|b| b[5] - b[2] < 1.0).map(|b| [b[0], b[1], b[3], b[4], b[5]]));
	}

	/// The live ground fire patches (stage 3 reads them; see the module doc).
	#[allow(dead_code)]
	pub(in crate::viewer) fn ground_patches(&self) -> &[GroundFirePatch] {
		self.blast.patches()
	}

	/// Every frame, after `set_particles` (the host calls `Gpu::fire_ground` with its collision world): puts this frame's new
	/// explosions on the ground (without a world they are dropped) and finds the grass under the Pyrocitor (firefx_jet_grass.rs).
	pub(in crate::viewer) fn ground_follow(&mut self, world: Option<&crate::physics::World>) {
		if self.jet_grass.busy() {
			self.jet_follow(world);
		}
		if !self.blast.has_work() {
			return; // (Fire Off / Low / Classic and every quiet frame: no knob lookups, no list walks)
		}
		let Some(world) = world else {
			self.blast.drop_pending();
			return;
		};
		// A hit that is not ground is None: water is collision surface id 0 (the highest face of any kind decides), so a blast over
		// water starts no fire. (The oceans that are no collision faces are not known here: a shallow seabed within reach lights.)
		let mut probe = |from: [f32; 3], max_drop: f32| {
			let (z, n, kind) = world.ground_where(from, max_drop, |_| true)?;
			(kind & 0x1f != 0).then_some((z, n))
		};
		self.blast.land(&mut probe, self.game.now());
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::fire_option::{FireKind, FireRow};

	fn flat(_: [f32; 3], _: f32) -> Option<(f32, [f32; 3])> {
		Some((0.0, [0.0, 0.0, 1.0]))
	}

	fn cand(slot: u32, x: f32, size: f32) -> Cand {
		Cand { slot, ty: 5, pos: [x, 0.0, 0.5], size, big: true }
	}

	impl BlastFire {
		/// `note` at game time 0 (most tests have no clock).
		fn note0(&mut self, cands: &[Cand]) {
			self.note(cands, 0.0);
		}
	}

	/// A blast fire that has seen one empty frame (so the next new candidate counts).
	fn primed() -> BlastFire {
		let mut b = BlastFire::default();
		b.note0(&[]);
		b
	}

	fn sprite(size: f32, alpha: f32, pool: bool) -> crate::prims::Sprite {
		crate::prims::Sprite { pos: [1.0, 2.0, 3.0], size, rot: 0.0, colour: [1.0, 1.0, 1.0, alpha], tex: 0, additive: true, pool: pool.then_some(0), flat: false }
	}

	#[test]
	fn burnt_grass_outlives_the_patch_and_needs_fire_and_vegetation() {
		let mut b = primed();
		b.level = Some(1);
		b.burn_feed = true;
		b.note0(&[cand(1, 3.0, 2.0)]);
		b.land(&mut flat, 10.0);
		assert_eq!((b.patches().len(), b.burn_u(11.0).head[0]), (1, 1.0), "a new patch also burns an area");
		// The patch is gone after burn + scorch (11 s); the area stays 60 s after the flames and regrows over 15 s.
		b.land(&mut flat, 22.0);
		assert!(b.patches().is_empty());
		assert_eq!(b.burn_u(40.0).head[0], 1.0);
		assert_eq!(b.burn_u(10.0 + 3.0 + 60.0 + 15.0 + 2.0).head[0], 0.0, "regrown");
		b.land(&mut flat, 100.0);
		assert!(b.burnt.is_empty(), "expired");
		// Vegetation Off (burn_feed false): the fire starts, nothing is burnt and the uniform stays empty.
		let mut b = primed();
		b.level = Some(1);
		b.note0(&[cand(1, 3.0, 2.0)]);
		b.land(&mut flat, 10.0);
		assert_eq!(b.patches().len(), 1);
		assert!(b.burnt.is_empty());
		assert_eq!((b.burn_u(11.0).head, b.burn_u(11.0).look), ([0.0; 4], [0.0; 4]));
		// Fire Off / Low / Classic (watch with on = false) and a level change forget the burnt areas.
		b.burn_feed = true;
		b.note0(&[cand(1, 3.0, 2.0), cand(2, 30.0, 2.0)]);
		b.land(&mut flat, 10.0);
		assert!(!b.burnt.is_empty());
		b.watch(false, 1, &[], None, [0.0; 3], 0.0);
		assert!(b.burnt.is_empty() && b.patches().is_empty() && !b.burn_feed);
		assert_eq!(b.burn_u(11.0).head, [0.0; 4]);
	}

	#[test]
	fn a_seabed_under_a_flat_water_surface_starts_no_fire() {
		let mut b = primed();
		b.level = Some(1);
		// A 100 m sea at height 5 (x 0..100): the ground at 0 is its seabed.
		b.set_water([[0.0, -50.0, 100.0, 50.0, 5.0]].into_iter());
		b.note0(&[cand(1, 3.0, 2.0)]);
		b.land(&mut flat, 10.0);
		assert!(b.patches().is_empty(), "the seabed under the sea does not burn");
		// Beside the sea (x -10) the same ground burns; and ground above the surface (shore at 6) burns too.
		b.note0(&[cand(2, -10.0, 2.0)]);
		b.land(&mut flat, 10.0);
		assert_eq!(b.patches().len(), 1);
		b.note0(&[cand(3, 20.0, 2.0)]);
		b.land(&mut |_, _| Some((6.0, [0.0, 0.0, 1.0])), 10.0);
		assert_eq!(b.patches().len(), 2, "ground above the water top is a shore");
		// Off (level None) keeps no surfaces; a reset forgets them.
		b.reset();
		b.set_water([[0.0; 5]].into_iter());
		assert!(b.water.is_empty(), "nothing is kept while the fire is off");
	}

	#[test]
	fn the_life_timeline_goes_flame_scorch_gone_without_a_jump() {
		let life = Life { burn: 3.0, scorch: 8.0 };
		let p = GroundFirePatch { pos: [0.0; 3], normal: [0.0, 0.0, 1.0], radius: 1.0, born: 10.0, seed: 0.5 };
		assert_eq!(p.phase(9.9, &life), None, "before the start");
		let a = p.phase(10.0, &life).unwrap();
		assert_eq!((a.flame, a.scorch), (0.0, 0.0));
		let mid = p.phase(11.0, &life).unwrap();
		assert!(mid.flame > 0.95 && mid.scorch > 0.95, "{mid:?}");
		let late = p.phase(12.9, &life).unwrap();
		assert!(late.flame < 0.02 && late.scorch > 0.99, "flames dead, scorch still there: {late:?}");
		let fading = p.phase(16.0, &life).unwrap();
		assert!(fading.flame == 0.0 && fading.scorch > 0.0 && fading.scorch < 1.0, "{fading:?}");
		assert_eq!(p.phase(10.0 + 11.0, &life), None, "gone at burn + scorch");
		// No jump between neighbouring times, flame and scorch never rise again once they fall.
		let (mut prev, mut fell_f, mut fell_s) = (p.phase(10.0, &life).unwrap(), false, false);
		for k in 1..1100 {
			let cur = p.phase(10.0 + k as f64 * 0.01, &life).unwrap();
			assert!((cur.flame - prev.flame).abs() < 0.2 && (cur.scorch - prev.scorch).abs() < 0.1, "jump at step {k}: {prev:?} -> {cur:?}");
			fell_f |= cur.flame < prev.flame - 1e-6;
			fell_s |= cur.scorch < prev.scorch - 1e-6;
			assert!(!fell_f || cur.flame <= prev.flame + 1e-6, "the flame rose again");
			assert!(!fell_s || cur.scorch <= prev.scorch + 1e-6, "the scorch rose again");
			prev = cur;
		}
	}

	#[test]
	fn explosions_are_noted_when_new_and_the_first_call_only_primes() {
		let mut b = BlastFire::default();
		b.note0(&[cand(1, 0.0, 3.0)]);
		assert!(b.pending.is_empty(), "the first call after a reset starts no fire");
		b.note0(&[cand(1, 0.0, 3.2), cand(2, 5.0, 2.0)]);
		assert_eq!(b.pending.len(), 1, "slot 1 was seen already, slot 2 is new");
		assert_eq!(b.pending[0].pos[0], 5.0);
		b.note0(&[cand(1, 0.0, 3.4), cand(2, 5.0, 2.5)]);
		assert_eq!(b.pending.len(), 1, "nothing new");
		// A different type in the same slot is a new particle.
		b.note0(&[Cand { ty: 8, ..cand(1, 0.0, 3.0) }]);
		assert_eq!(b.pending.len(), 2);
	}

	/// Review m5: a pool slot taken over by a new explosion of the same type lights again; the same sprite dipping under the gate and
	/// coming back does not; a sprite that starts small lights once when it passes the gate; a short hop does not count as new.
	#[test]
	fn detection_follows_every_blast_sprite_and_lights_each_once() {
		let small = |x: f32| Cand { big: false, ..cand(1, x, 0.3) };
		// Slot reuse: the same slot and type, a new place, long after the first fire.
		let mut b = primed();
		b.note(&[cand(1, 0.0, 3.0)], 10.0);
		assert_eq!(b.pending.len(), 1);
		b.pending.clear();
		b.note(&[cand(1, 0.1, 3.2)], 10.2);
		assert!(b.pending.is_empty(), "the same explosion growing");
		b.note(&[cand(1, 8.0, 3.0)], 11.0);
		assert_eq!(b.pending.len(), 1, "the slot was taken over by an explosion 8 m away");
		assert_eq!(b.pending[0].pos[0], 8.0);
		// A hop inside the first 0.4 s is the same sprite.
		let mut b = primed();
		b.note(&[cand(1, 0.0, 3.0)], 10.0);
		b.pending.clear();
		b.note(&[cand(1, 8.0, 3.0)], 10.3);
		assert!(b.pending.is_empty(), "too soon after the first fire to be a new explosion");
		// A flipbook that fades under the gate and comes back keeps its one fire.
		let mut b = primed();
		b.note(&[cand(1, 0.0, 3.0)], 10.0);
		b.pending.clear();
		for k in 1..=10 {
			b.note(&[small(0.0)], 10.0 + 0.1 * k as f64);
		}
		b.note(&[cand(1, 0.0, 3.0)], 11.5);
		assert!(b.pending.is_empty(), "no second fire from the same sprite");
		// A sprite that starts small lights once when it passes the gate (and not again).
		let mut b = primed();
		b.note(&[small(0.0)], 1.0);
		b.note(&[small(0.0)], 1.1);
		assert!(b.pending.is_empty());
		b.note(&[cand(1, 0.0, 3.0)], 1.2);
		assert_eq!(b.pending.len(), 1);
		b.note(&[cand(1, 0.0, 3.1)], 1.3);
		assert_eq!(b.pending.len(), 1, "exactly one");
		// A sprite that left the list and came back is a new one.
		b.note(&[], 1.4);
		b.note(&[cand(1, 0.0, 3.0)], 1.5);
		assert_eq!(b.pending.len(), 2);
		// The first call after a reset primes everything on screen, big or not.
		let mut b = BlastFire::default();
		b.note(&[cand(1, 0.0, 3.0), Cand { slot: 3, ..small(5.0) }], 2.0);
		assert!(b.pending.is_empty());
		b.note(&[cand(1, 0.0, 3.0), Cand { slot: 3, ..cand(1, 5.0, 3.0) }, cand(2, 9.0, 3.0)], 2.1);
		assert_eq!(b.pending.len(), 1, "slots 1 and 3 were primed (started), slot 2 is new and big");
		assert_eq!(b.pending[0].pos[0], 9.0);
	}

	/// Review m1 / m4: nothing alive = no work; `reset` (what `FireFx::forget` calls) clears patches, grass, blasts and the seen list.
	#[test]
	fn quiet_frames_do_no_work_and_forget_clears_everything() {
		let mut b = primed();
		assert!(!b.has_work());
		b.note0(&[cand(1, 0.0, 3.0)]);
		assert!(b.has_work(), "a blast waits");
		b.land(&mut flat, 1.0);
		assert!(b.has_work(), "a patch burns");
		b.land(&mut flat, 50.0);
		assert!(!b.has_work(), "gone");
		// ground_follow returns first with no work (even with a world it would need), and drops pending blasts without one.
		let mut f = FireFx::default();
		f.ground_follow(None);
		assert!(!f.blast.has_work());
		f.blast.pending.push(Blast { pos: [0.0; 3], size: 3.0 });
		f.ground_follow(None);
		assert!(f.blast.pending.is_empty(), "no world: the blast is dropped");
		// forget: everything the fire kept is gone, and the next note only primes.
		f.blast.level = Some(2);
		f.blast.burn_feed = true;
		f.blast.note(&[], 4.0);
		f.blast.note(&[cand(1, 0.0, 3.0)], 5.0);
		f.blast.land(&mut flat, 5.0);
		assert!(!f.blast.patches.is_empty() && !f.blast.burnt.is_empty());
		f.forget();
		assert!(f.blast.patches.is_empty() && f.blast.burnt.is_empty() && f.blast.pending.is_empty() && f.blast.seen.is_empty() && !f.blast.primed);
		f.blast.note(&[cand(1, 0.0, 3.0)], 6.0);
		assert!(f.blast.pending.is_empty(), "the first call after a forget only primes");
	}

	#[test]
	fn off_and_a_level_change_forget_everything() {
		let mut b = primed();
		b.level = Some(3);
		b.note0(&[cand(1, 0.0, 3.0)]);
		b.land(&mut flat, 1.0);
		assert_eq!(b.patches().len(), 1);
		b.watch(false, 3, &[], None, [0.0; 3], 0.0);
		assert!(b.patches().is_empty() && b.pending.is_empty() && !b.primed && b.level.is_none(), "off clears");
		b.watch(true, 3, &[], None, [0.0; 3], 0.0);
		assert_eq!(b.level, Some(3));
		assert!(!b.primed, "without a pool nothing is primed");
		b.note0(&[]);
		b.note0(&[cand(1, 0.0, 3.0)]);
		b.land(&mut flat, 2.0);
		assert_eq!(b.patches().len(), 1);
		b.watch(true, 4, &[], None, [0.0; 3], 0.0);
		assert!(b.patches().is_empty(), "another level starts empty");
	}

	/// With the default rows of the test pack (none) no sprite is an explosion; with a Blast row every pool sprite of that type is followed
	/// and the big, visible ones are marked `big` (they alone start a fire); a Burst row only with the knob.
	#[test]
	fn candidates_are_the_pool_sprites_of_blast_rows_and_the_big_visible_ones_are_marked() {
		let rows = [FireRow { key: 5, kind: FireKind::Blast }, FireRow { key: 55, kind: FireKind::Burst }, FireRow { key: 12, kind: FireKind::Jet }];
		let t = Tune { radius: 1.4, h: 0.9, max: 8, reach: 1.5, min_size: 0.6, bursts: false };
		// Pool sprites: [0] type 5 big, [1] type 5 tiny, [2] type 12, [3] type 5 invisible, [4] type 55; the 3rd entry is no pool sprite.
		let sprites = [sprite(2.0, 0.5, true), sprite(0.3, 0.5, true), sprite(2.0, 0.5, false), sprite(2.0, 0.5, true), sprite(2.0, 0.0, true), sprite(2.0, 0.5, true)];
		let types = [5u8, 5, 12, 5, 55];
		let slots = [10u16, 11, 12, 13, 14];
		let got = candidates(&rows, &sprites, &types, &slots, &[], &t);
		assert_eq!(got.iter().map(|c| (c.slot, c.ty, c.big)).collect::<Vec<_>>(), vec![(10, 5, true), (11, 5, false), (13, 5, false)], "the tiny and the invisible ones come back unmarked; the jet and the burst ones are out");
		let with55 = candidates(&rows, &sprites, &types, &slots, &[], &Tune { bursts: true, ..t });
		assert_eq!(with55.iter().map(|c| c.slot).collect::<Vec<_>>(), vec![10, 11, 13, 14]);
		assert!(candidates(&[], &sprites, &types, &slots, &[], &t).is_empty(), "no rows, no ground fire (R&C2)");
		assert!(candidates(&rows, &sprites, &[], &[], &[], &t).is_empty(), "no types, no ground fire");
	}

	/// A pack's blast (R&C1: a thrown bomb that went off, no pool sprite; user 2026-11-04 "bomb glove doesnt leave fire") is a big
	/// candidate of its own identity space, also without fire rows: ONE fire however many frames it lasts, a second id a second fire.
	#[test]
	fn pack_blasts_light_once_per_id() {
		let t = Tune { radius: 1.4, h: 0.9, max: 8, reach: 1.5, min_size: 0.6, bursts: false };
		let bomb = crate::fxpool::BlastPoint { id: 0x1a2b00, pos: [1.0, 2.0, 0.5], size: 3.0 };
		let got = candidates(&[], &[], &[], &[], &[bomb], &t);
		assert_eq!(got.iter().map(|c| (c.slot, c.ty, c.big)).collect::<Vec<_>>(), vec![(0x1a2b00, PACK_TY, true)]);
		let mut b = primed();
		b.note0(&got);
		b.note0(&got);
		assert_eq!(b.pending.len(), 1, "the same bomb on later frames is the same blast");
		let two = [bomb, crate::fxpool::BlastPoint { id: 0x1a2c00, pos: [6.0, 2.0, 0.5], size: 3.0 }];
		b.note0(&candidates(&[], &[], &[], &[], &two, &t));
		assert_eq!(b.pending.len(), 2, "a second bomb is a second blast");
		b.land(&mut flat, 0.0);
		assert_eq!(b.patches().len(), 2);
	}

	#[test]
	fn land_rejects_the_air_water_and_steep_faces() {
		let mut b = primed();
		b.note0(&[cand(1, 0.0, 3.0)]);
		b.land(&mut |_, _| None, 1.0);
		assert!(b.patches().is_empty(), "no ground under a blast in the air (or water, which the probe answers with None)");
		assert!(b.pending.is_empty(), "the blast is spent");
		b.note0(&[cand(1, 0.0, 3.0), cand(2, 4.0, 3.0)]);
		b.land(&mut |_, _| Some((0.0, [0.9, 0.0, 0.4])), 1.0);
		assert!(b.patches().is_empty(), "a wall-like face is no ground");
		b.note0(&[]);
		b.note0(&[cand(3, 8.0, 3.0)]);
		let mut seen_from = [0.0f32; 3];
		b.land(
			&mut |from, max_drop| {
				seen_from = from;
				assert!(max_drop > 1.5, "the probe reaches at least the reach knob below");
				Some((0.1, [0.0, 0.0, 1.0]))
			},
			1.0,
		);
		assert_eq!(b.patches().len(), 1);
		assert!((seen_from[0] - 8.0).abs() < 1e-6 && seen_from[1] == 0.0 && (seen_from[2] - 0.9).abs() < 1e-5, "probed from 0.4 m above the sprite: {seen_from:?}");
		assert_eq!(b.patches()[0].pos, [8.0, 0.0, 0.1], "the patch sits on the ground point");
		assert!((b.patches()[0].radius - 1.4 * 1.0).abs() < 1e-5, "a size 3 blast = the radius knob");
	}

	#[test]
	fn the_same_explosion_merges_a_later_one_relights_and_the_cap_drops_the_farthest() {
		let mut b = primed();
		b.eye = [0.0; 3];
		b.note0(&[cand(1, 0.0, 3.0), cand(2, 0.5, 2.0)]);
		b.land(&mut flat, 10.0);
		assert_eq!(b.patches().len(), 1, "two sprites of one explosion = one patch");
		assert_eq!(b.patches()[0].born, 10.0);
		// A new blast on the same spot 0.2 s later is the same explosion; 2 s later it lights the patch again.
		b.note0(&[]);
		b.note0(&[cand(3, 0.2, 3.0)]);
		b.land(&mut flat, 10.2);
		assert_eq!(b.patches()[0].born, 10.0);
		b.note0(&[]);
		b.note0(&[cand(4, 0.2, 3.0)]);
		b.land(&mut flat, 12.0);
		assert_eq!(b.patches().len(), 1);
		assert_eq!(b.patches()[0].born, 11.75, "relit a quarter second into its life: no flicker out");
		// The cap: nine far patches over a cap of 8 keep the eight nearest to the eye.
		let mut b = primed();
		b.eye = [0.0; 3];
		for k in 0..9u32 {
			b.note0(&[]);
			b.note0(&[cand(k + 1, 10.0 + 5.0 * k as f32, 3.0)]);
			b.land(&mut flat, 20.0 + k as f64 * 0.01);
		}
		assert_eq!(b.patches().len(), 8);
		assert!(b.patches().iter().all(|p| p.pos[0] < 10.0 + 5.0 * 8.0 - 1.0), "the farthest (x = 50) went");
		for k in 0..40u32 {
			b.note0(&[]);
			b.note0(&[cand(100 + k, -3.0 * k as f32, 3.0)]);
			b.land(&mut flat, 21.0);
		}
		assert!(b.patches().len() <= tuned().max.min(MAX_PATCHES));
	}

	#[test]
	fn patches_expire() {
		let mut b = primed();
		b.note0(&[cand(1, 0.0, 3.0)]);
		b.land(&mut flat, 5.0);
		assert_eq!(b.patches().len(), 1);
		b.land(&mut flat, 5.0 + 10.9);
		assert_eq!(b.patches().len(), 1, "still scorched");
		b.land(&mut flat, 5.0 + 11.1);
		assert!(b.patches().is_empty());
	}

	fn vertex_count(level: u8, now: f64, b: &BlastFire) -> (usize, usize) {
		let (mut q, mut e) = (Vec::new(), Vec::new());
		b.flames(now, level, [1.0, 0.0, 0.0], &mut q, &mut e, 256, 128);
		(q.len() / 4, e.len())
	}

	#[test]
	fn flames_are_three_tongues_at_medium_five_at_high_and_only_while_burning() {
		let mut b = primed();
		b.note0(&[cand(1, 0.0, 3.0)]);
		b.land(&mut flat, 100.0);
		assert_eq!(vertex_count(2, 101.0, &b), (3, 2), "Medium: 3 tongues, 2 emitters");
		assert_eq!(vertex_count(3, 101.0, &b), (5, 2), "High: 5 tongues");
		assert_eq!(vertex_count(2, 100.0, &b), (0, 0), "not lit yet at t = 0");
		assert_eq!(vertex_count(2, 104.0, &b), (0, 0), "burnt out after 3 s: only the scorch stays");
		let (mut q, mut e) = (Vec::new(), Vec::new());
		b.flames(101.0, 3, [1.0, 0.0, 0.0], &mut q, &mut e, 2, 1);
		assert_eq!((q.len() / 4, e.len()), (2, 1), "the room caps both lists");
		// Tongues stand on the plane inside 0.55 radius.
		let p = b.patches()[0];
		for t in q.chunks(4) {
			let base_x = (t[0].pos[0] + t[1].pos[0]) * 0.5;
			assert!((base_x - p.pos[0]).abs() <= 0.55 * p.radius + 1e-4, "{base_x}");
		}
	}

	#[test]
	fn watch_off_changes_nothing_and_builds_nothing() {
		let mut b = BlastFire::default();
		b.watch(false, 1, &[sprite(3.0, 1.0, true)], None, [1.0, 2.0, 3.0], 0.0);
		assert!(b.patches().is_empty() && b.pending.is_empty() && !b.primed && b.level.is_none());
		assert_eq!(b.eye, [0.0; 3], "nothing was stored");
		assert_eq!(vertex_count(3, 5.0, &b), (0, 0));
	}

	#[test]
	fn the_hash_is_stable_and_spread() {
		assert_eq!(h01(3, 4), h01(3, 4));
		let v: Vec<f32> = (0..64).map(|i| h01(9, i)).collect();
		assert!(v.iter().all(|x| (0.0..1.0).contains(x)));
		let mean = v.iter().sum::<f32>() / 64.0;
		assert!((0.35..0.65).contains(&mean), "{mean}");
	}

	#[test]
	fn tangents_are_unit_and_in_the_plane() {
		for n in [[0.0, 0.0, 1.0], normalise([0.3, -0.2, 0.9]), normalise([0.0, 0.7, 0.7])] {
			let (a, b) = tangents(n);
			for t in [a, b] {
				let l = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
				assert!((l - 1.0).abs() < 1e-4 && (t[0] * n[0] + t[1] * n[1] + t[2] * n[2]).abs() < 1e-4);
			}
		}
	}
}
