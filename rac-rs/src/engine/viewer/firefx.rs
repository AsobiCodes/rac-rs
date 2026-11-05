//! [PC] Fire Low / Medium / High (the Fire option, fire_option.rs; with Lighting Enhanced and Particles Enhanced; Split view: hangs on
//! `splitview::fx` through `active`): the game's fire drawn as realistic procedural flames instead of its flat billboards.
//! The pack names the fire draws as table rows (`Pack::fire_rows`); the retail code, particle pool and level fire quads are untouched.
//! - Level fires (a burning building): the additive flame quads of a fire (a `FireKind::Ground` row) become ONE upright tongue that
//!   faces the eye (firefx_ground.rs). Taller and wider than the retail quad (`fire_h`, `fire_w`). Only the quads that became a
//!   drawn tongue are left out of the quad pass (`skips`); the rest draw as retail. Its grey smoke columns stay retail's quads unless
//!   the Smoke option turns them into volumes (volsmoke/columns.rs).
//! - Effect fire (flamethrower, explosions, burning things): pool sprites whose EFFECT TYPE is a `Flame` row become tongues, a `Burst`
//!   row a fireball (the type is the slot's byte +0, `PoolFx::sprite_types`; one type is one def, so a flipbook is fire in all its
//!   frames or in none). A sprite taken over is given back to the particle pass on any frame the fire does not draw (`settle`).
//! The shader (firefx.wgsl) is turbulent noise, domain-warped and scrolling up so it never loops: white-yellow core, orange body, deep red
//! tips, smoky dark edges, soft where it meets walls and ground, a glow into the bloom, and from Medium rising embers. The fire's warm
//! light on the scene around it follows the Global illumination row, not this one (firefx_light.rs). Off draws and changes nothing.
//! Dev knobs (`RAC_TUNE`, ENV.md): fire_speed fire_turb fire_core fire_glow fire_h fire_w fire_tint fire_smoke fire_embers
//! fire_ember_size fire_far fire_ember_min (the light's: firefx_light.rs); `RAC_FIRE_WGSL=<file>` loads the shader from disk.
//! The shaders' clock wraps every 512 s without a jump (firefx_clock.rs).

use super::firefx_clock::{self, Clock};
use super::firefx_gametime::GameClock;
use super::firefx_ground::{self, ground_of};
use super::firefx_blast::BlastFire;
use super::firefx_jet;
use super::firefx_vol::{self, Vol};
use super::firefx_jet_quads::{self, JetState};
use super::firefx_jet_vol;
pub(super) use super::firefx_jet_quads::View;
use super::*;
use crate::fire_option::{self, FireKind};
use partfx::FxLights;
use std::collections::BTreeMap;

const WGSL: &str = include_str!("firefx.wgsl");
/// Tongues and fireballs a frame (ground fires, then sprites), emitters for embers and lights.
const MAX_GROUND: usize = 256;
const MAX_QUADS: usize = 1280;
const MAX_EMIT: usize = 128;
/// The lowest part of a tongue's quad below its base (uv.y), so the flame's foot has no hard line.
const FOOT: f32 = 0.06;

/// The key of `Gpu::part_verts`: (run, additive, texture key).
pub(super) type PartKey = (u32, bool, u32);

pub(super) fn tune(name: &str, d: f32) -> f32 {
	crate::devtune::f32(name, d)
}

/// Whether explosions leave a ground fire (dev knob `fire_ground_blast`, 0 = off; the shipped default is ON, the test
/// `explosions_leave_ground_fire_by_default` guards it).
pub(super) fn ground_blast_on() -> bool {
	tune("fire_ground_blast", 1.0) > 0.5
}

/// Whether Fire draws at all (the option, Lighting and Particles Enhanced, not the Classic half of the split view).
pub(in crate::viewer) fn active() -> bool {
	// (A1: a driver that refused the fire shader keeps the retail fire, sprites and quads alike: nothing is diverted.)
	if fire_option::level() == 0 || crate::viewer::pipe_guard::off(crate::viewer::pipe_guard::Effect::Fire) {
		return false;
	}
	let g = crate::gfx::graphics();
	g.lighting > 0 && g.particles && crate::splitview::fx(true)
}

/// Flames and fireballs as true 3D volumes (firefx_vol.rs); `fire_vol=0` (dev knob) draws the old upright cards for an A/B.
pub(super) fn volumes() -> bool {
	tune("fire_vol", 1.0) > 0.5
}

/// The flame volume of an upright card made by `tongue` (the ground fire's small flames, firefx_blast.rs): same foot, height, width.
fn card_volume(q: &[Vertex]) -> Vol {
	let (l, c) = (q[0].light, q[0].colour);
	let h = l[2];
	let base = std::array::from_fn(|k| (q[0].pos[k] + q[1].pos[k]) * 0.5 + if k == 2 { FOOT * h } else { 0.0 });
	firefx_vol::flame(base, h, 0.5 * l[1] * h, l[0], c[3], [c[0], c[1], c[2]])
}

/// A fire for embers and lights: base centre, flame height, width, seed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Emit {
	pub base: [f32; 3],
	pub h: f32,
	pub w: f32,
	pub seed: f32,
}

/// The shader's uniform block, as 16-byte rows: 5 rows of settings, then two rows per emitter.
const ROWS: usize = 5 + 2 * MAX_EMIT;

/// Everything `draw` needs from the renderer (field borrows, so the caller keeps its scene borrow).
pub(super) struct FireEnv<'a> {
	pub device: &'a wgpu::Device,
	pub queue: &'a GatedQueue,
	/// Any pipeline of the level shader: its group 0 layout is the viewer's uniform block.
	pub pipeline: &'a wgpu::RenderPipeline,
	pub partfx: &'a partfx::PartFx,
	pub format: wgpu::TextureFormat,
	pub depth_format: wgpu::TextureFormat,
	/// The level pass's view-projection (the ground fire's decal rebuilds world points from the depth, firefx_blast_gpu.rs).
	pub view_proj: [[f32; 4]; 4],
}

struct Res {
	flame: wgpu::RenderPipeline,
	ember: wgpu::RenderPipeline,
	ubuf: wgpu::Buffer,
	ubind: wgpu::BindGroup,
	vbuf: std::cell::RefCell<Option<(wgpu::Buffer, u64)>>,
	ibuf: wgpu::Buffer,
	vol: firefx_vol::VolRes,
}

/// The fire's per-frame lists and GPU objects.
#[derive(Default)]
pub struct FireFx {
	/// Made at the first draw (cells: the pass borrows the renderer, so `draw` takes `&self`).
	res: std::cell::OnceCell<Res>,
	/// This frame draws fire (decided by `begin_sprites`, then `gather`).
	on: bool,
	/// [PC] The fire simulation (firesim.rs): the volumes are its emitters.
	pub(super) sim: super::firesim::FireSim,
	/// [PC] The level's fire tongues as flipbook sprites (firebook.rs), with the simulation on.
	pub(super) book: super::firebook::FireBook,
	/// Tongues of the level's fire quads (`gather`), of the pool sprites (`add_sprite`); 4 vertices each.
	quads: Vec<Vertex>,
	sprites: Vec<Vertex>,
	/// The true-3D flames and fireballs (firefx_vol.rs): of the level's fires (`gather`) and of the pool sprites (`add_sprite`).
	vols: Vec<Vol>,
	svols: Vec<Vol>,
	/// The volumes the level wanted at the last `gather` that drew (its tongues before the cap, its blast flames): `add_sprite` keeps
	/// that many of `firefx_vol::MAX_VOLS` free, so the level's fires come first and the frame never has more volumes than the cap.
	reserve: usize,
	/// `draw`'s list of the frame's volumes (kept between frames: no allocation per frame).
	order: std::cell::RefCell<Vec<Vol>>,
	/// `draw`'s far-to-near list of the flipbook sprites (kept between frames).
	book_order: std::cell::RefCell<Vec<super::firebook::Spr>>,
	/// The ember emitters of the level's fires (`gather`: the faded-in tongues, at most `MAX_EMIT` / 2, then the blast flames) and of
	/// the sprites (jets, burn flames, big tongues); `lit` = what the fire lights weigh (every tongue by its fade, the blast flames).
	ground: Vec<Emit>,
	big: Vec<Emit>,
	lit: Vec<Emit>,
	/// The fire lights this frame (`gather`: firefx_light.rs `wanted`, Global illumination not Off), also with the Fire row Off.
	lights: bool,
	/// The frame's horizontal across-the-view direction (`begin_sprites`): the burn flames over the volume cap stand as cards facing it.
	across: [f32; 3],
	/// Per sprite of this frame's list (`set_particles`): the fire kind and pool slot of a pool sprite whose effect type is a fire row.
	picks: Vec<Option<(FireKind, u16)>>,
	/// The retail quads of the sprites taken over this frame, with their `part_verts` key: given back by `settle` when fire does not draw.
	held: Vec<(PartKey, [Vertex; 4])>,
	/// The quads the last `settle` appended to `part_verts` (a second pass of the frame removes them before it decides again).
	given: Vec<(PartKey, [Vertex; 4])>,
	/// Per effect quad: it became part of a drawn tongue (`gather`), so the quad pass leaves it out.
	took: Vec<bool>,
	/// The Pyrocitor's jets: the places they stood at lately (firefx_jet_quads.rs).
	jets: JetState,
	/// The jets of the last two fields, drawn at the frame smoothing's alpha (firefx_jet_lerp.rs).
	jet_lerp: super::firefx_jet_lerp::JetLerp,
	/// The level's tongues of the last frame: their seeds and noise (firefx_ground_track.rs).
	ground_track: super::firefx_ground_track::GroundTrack,
	/// This frame's burn puffs, drawn as one flame per cluster (firefx_burn.rs).
	burn: super::firefx_burn::Burn,
	/// The frame's sprite list with the pool sprites at the shown field's own places (`PoolFx::tick`): the jets are built from it
	/// (kept between frames: no allocation per frame).
	raw: Vec<crate::prims::Sprite>,
	/// Where the jets touch the grass (firefx_jet_grass.rs; Medium and High).
	pub(super) jet_grass: super::firefx_jet_grass::JetGrass,
	/// The ground fire after explosions (firefx_blast.rs): the explosions seen, the patches they made, their GPU objects.
	pub(super) blast: BlastFire,
	/// The game time of the ground fire, the burnt grass and the jets' memory: stands while the game stands (firefx_gametime.rs).
	pub(super) game: GameClock,
}

pub(super) fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
	((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The four vertices of an upright tongue: base centre `base`, unit `across`, width `w`, height `h`, world up.
pub(super) fn tongue(base: [f32; 3], across: [f32; 3], w: f32, h: f32, tint: [f32; 3], fade: f32, seed: f32, kind: f32) -> [Vertex; 4] {
	let at = |s: f32, z: f32| [base[0] + across[0] * w * 0.5 * s, base[1] + across[1] * w * 0.5 * s, base[2] + z];
	let v = |p: [f32; 3], uv: [f32; 2]| Vertex { pos: p, uv, normal: [0.0; 3], colour: [tint[0], tint[1], tint[2], fade], light: [seed, w / h.max(1e-3), h, kind] };
	[v(at(-1.0, -FOOT * h), [0.0, -FOOT]), v(at(1.0, -FOOT * h), [1.0, -FOOT]), v(at(1.0, h), [1.0, 1.0]), v(at(-1.0, h), [0.0, 1.0])]
}

/// What `FireFx::add_sprite` needs to know about a pool sprite: where it is, how big, its colour after the near / far fade, and the eye.
pub(super) struct SpriteAt {
	pub pos: [f32; 3],
	pub size: f32,
	pub colour: [f32; 4],
	pub right: [f32; 3],
	pub up: [f32; 3],
}

/// Per sprite of the list: the fire kind and pool slot when the n-th POOL sprite's effect type is a fire row (`types` / `slots` are
/// the pool's, parallel to the pool sprites), else None.
/// `blasts` (the fire simulation on): the explosions' types (`Blast` rows) are fire too, a burst emitter each (`add_sprite`).
fn classify(rows: &[fire_option::FireRow], pool: impl Iterator<Item = bool>, types: &[u8], slots: &[u16], blasts: bool) -> Vec<Option<(FireKind, u16)>> {
	let mut n = 0;
	pool.map(|is_pool| {
		if !is_pool {
			return None;
		}
		let at = n;
		n += 1;
		let ty = *types.get(at)?;
		if skipped(ty) {
			return None;
		}
		let kind = fire_option::sprite_kind(rows, ty).or_else(|| (blasts && fire_option::blast_type(rows, ty, false)).then_some(FireKind::Blast))?;
		Some((kind, slots.get(at).copied().unwrap_or(at as u16)))
	})
	.collect()
}

/// A flame-row sprite that looks like a flame (`add_sprite`): a warm or green saturated colour (white beams, grey smoke are not) and
/// at least `fire_flame_min` (0.25) big (the impacts' sparks and embers, 0.03..0.16, are not; dev knob, 0 = any size).
pub(super) fn flame_like(c: [f32; 4], size: f32) -> bool {
	fire_option::is_hot([c[0], c[1], c[2]]) && size >= tune("fire_flame_min", 0.25)
}

/// An explosion sprite the simulation takes (`add_sprite`, dev knob `fire_sim_blasts` 0 = none): warm (red to yellow) and saturated.
/// The same types also draw dim, green or purple glows that stay retail.
pub(super) fn blast_like(c: [f32; 4]) -> bool {
	let m = c[0].max(c[1]).max(c[2]);
	tune("fire_sim_blasts", 1.0) > 0.5 && fire_option::is_hot([c[0], c[1], c[2]]) && c[0] >= m * 0.95 && c[2] <= 0.6 * c[0]
}

/// Dev knobs `fire_skip1`..`fire_skip4` (RAC_TUNE): effect types left to retail whatever their fire row (an A/B of one type).
fn skipped(ty: u8) -> bool {
	(1..=4).any(|k| tune(["fire_skip1", "fire_skip2", "fire_skip3", "fire_skip4"][k - 1], -1.0) == ty as f32)
}

/// Dev log (`fire_pick_log` 1, ENV.md): per frame, each effect type of the pool's sprites with its count, how many the fire took and
/// as what, the mean size, the first colour and texture, the nearest to the hero (m).
fn log_picks(now: f64, sprites: &[crate::prims::Sprite], types: &[u8], picks: &[Option<(FireKind, u16)>], hero: Option<[f32; 3]>) {
	let mut by: BTreeMap<u8, (u32, u32, Option<FireKind>, f32, [f32; 4], u32, f32)> = BTreeMap::new();
	let mut n = 0;
	for (sp, p) in sprites.iter().zip(picks) {
		if sp.pool.is_none() {
			continue;
		}
		let Some(&ty) = types.get(n) else { break };
		n += 1;
		let d = hero.map_or(-1.0, |h| dist(h, sp.pos));
		let e = by.entry(ty).or_insert((0, 0, None, 0.0, sp.colour, sp.tex, f32::MAX));
		(e.0, e.3, e.6) = (e.0 + 1, e.3 + sp.size, e.6.min(d));
		if let Some((k, _)) = p {
			(e.1, e.2) = (e.1 + 1, Some(*k));
		}
	}
	for (ty, (count, took, kind, size, c, tex, d)) in by {
		eprintln!("fire_pick: t {now:.3} ty {ty} n {count} took {took} {kind:?} size {:.2} col {:.2} {:.2} {:.2} {:.2} tex {tex} hero {d:.1}", size / count as f32, c[0], c[1], c[2], c[3]);
	}
}

/// `sprites` into `out` with the n-th pool sprite's place, size and colour taken from `tick[n]` (the field's own, before the smoothing).
fn unblend(out: &mut Vec<crate::prims::Sprite>, sprites: &[crate::prims::Sprite], tick: &[crate::prims::Sprite]) {
	out.clear();
	out.extend_from_slice(sprites);
	let mut n = 0;
	for s in out.iter_mut().filter(|s| s.pool.is_some()) {
		if let Some(t) = tick.get(n) {
			(s.pos, s.size, s.colour, s.rot) = (t.pos, t.size, t.colour, t.rot);
		}
		n += 1;
	}
}

/// Removes one quad (4 vertices, compared bit for bit) from `verts`: the last place it appears.
pub(super) fn remove_quad(verts: &mut Vec<Vertex>, quad: &[Vertex; 4]) -> bool {
	let want: &[u8] = bytemuck::cast_slice(&quad[..]);
	match verts.chunks_exact(4).rposition(|c| bytemuck::cast_slice::<Vertex, u8>(c) == want) {
		Some(k) => {
			verts.drain(k * 4..k * 4 + 4);
			true
		}
		None => false,
	}
}

impl FireFx {
	/// Start of `Gpu::set_particles` (`part_verts` was just cleared): this frame's sprite list starts empty, the option is read once,
	/// and each pool sprite of `sprites` is looked up by its effect type in the level's fire rows (`fx`: the pool).
	/// `view`: the frame's camera, for the Pyrocitor's jets (firefx_jet_quads.rs: the stream segments are joined, their puffs go to `sprites`
	/// BEFORE the particle loop adds anything, the sub-bursts and embers far from a jet and the grey burn puffs go back to retail).
	pub(super) fn begin_sprites(&mut self, level: usize, sprites: &[crate::prims::Sprite], fx: Option<&crate::fxpool::PoolFx<'_>>, view: &View) {
		self.sprites.clear();
		self.svols.clear();
		self.big.clear();
		self.held.clear();
		self.given.clear();
		self.picks.clear();
		self.on = active();
		// The game time (firefx_gametime.rs); a swapped world (respawn, savestate, a gap with no pool) makes the fire forget what it kept.
		if !self.on {
			// (Fire Off: the clock still runs, so the retail-lit fire lights flicker on game time and freeze in a pause; a jump needs no forget.)
			self.game.update(firefx_clock::age(), fx.map(|f| f.frames), crate::field::field_dt() as f64);
			self.jets.forget();
			self.jet_lerp.forget();
			self.jet_grass.note(&[], false);
		} else if self.game.update(firefx_clock::age(), fx.map(|f| f.frames), crate::field::field_dt() as f64) {
			self.forget();
		}
		let right = cross(view.fwd, view.up);
		self.across = normalise([right[0], right[1], 0.0]);
		let now = self.game.now();
		// Ground fire after explosions (Medium and High): note the new explosions; Off / Low / Classic forget them (no cost).
		// The ground fire after explosions is ON (user 2026-11-04, "why dont explosions leave flame anymore"): it was off since 2026-11-03
		// by the default of the dev knob fire_ground_blast, a precaution of the Blaster-bullets-as-fire fix. Weapon hits start no patch:
		// only the Blast rows' sprites (types 5 and 8) at least `gfire_min_size` big do, never the Blaster's sparks (types 15, 21, 72).
		// The grass the Pyrocitor burns (firefx_jet_grass.rs) needs the burnt areas fed also while the blast fire is off: watching with
		// on = false reset them every frame and set no feed, so the held stream bent the grass but never lit it (user 2026-11-03).
		let ground = self.on && fire_option::level() >= 2;
		if ground && !ground_blast_on() {
			self.blast.burn_only(level);
		} else {
			self.blast.watch(ground, level, sprites, fx, view.eye, now);
		}
		if let (true, Some(fx)) = (self.on, fx) {
			self.picks = classify((crate::pack::get().fire_rows)(level), sprites.iter().map(|s| s.pool.is_some()), fx.sprite_types, fx.sprite_slots, volumes() && super::firesim::on());
			let q = firefx_jet::quality(fire_option::level());
			// The jets are built from the shown field's own sprites (`PoolFx::tick`) and blended once by `jet_lerp`; while the smoothing
			// does not blend (alpha 1, smoothing off) or the field's own list does not fit, from the shown sprites, not blended again.
			let alpha = super::firefx_jet_lerp::alpha();
			let raw = alpha < 1.0 && !fx.tick.is_empty() && fx.tick.len() == fx.sprite_slots.len();
			let hero = if raw { fx.tick_hero.or(fx.hero) } else { fx.hero };
			if raw {
				unblend(&mut self.raw, sprites, fx.tick);
			}
			let src = if raw { &self.raw[..] } else { sprites };
			let jets = self.jets.plan(src, &mut self.picks, view, hero, q, now);
			if tune("fire_pick_log", 0.0) > 0.5 {
				log_picks(now, src, fx.sprite_types, &self.picks, hero);
			}
			let jets = self.jet_lerp.blend(jets, fx.frames, if raw { alpha } else { 1.0 });
			self.jet_grass.note(&jets, fire_option::level() >= 2);
			self.push_jets(&jets, view, q);
		} else if self.on {
			// No pool this frame (a menu, a travel): no jet touches the grass.
			self.jet_grass.note(&[], false);
		}
	}

	/// The frame's Pyrocitor jets: as volumes (firefx_jet_vol.rs, the slabs go first into `svols`, under the cap the level's fires
	/// keep, `reserve`) with their ember emitters; as puffs (firefx_jet_quads.rs) for `fire_vol=0` or a jet that no longer fits. The
	/// slabs and emitters go straight into the frame's lists (no allocation per jet).
	fn push_jets(&mut self, jets: &[firefx_jet::Jet], view: &View, q: firefx_jet::Quality) {
		let vol = volumes();
		for jet in jets {
			let room = firefx_vol::MAX_VOLS.saturating_sub(self.reserve + self.svols.len());
			if vol && firefx_jet_vol::slabs_into(jet, room, &mut self.svols) {
				let room = MAX_EMIT.saturating_sub(self.big.len());
				self.big.extend(firefx_jet_quads::emitters(jet, jet.seed, q).take(room));
			} else {
				firefx_jet_quads::push(std::slice::from_ref(jet), view, q, &mut self.sprites, &mut self.big, (MAX_QUADS - MAX_GROUND) * 4, MAX_EMIT);
			}
		}
	}

	/// Forgets everything the fire kept from earlier frames: the ground fire patches, the burnt grass, the explosions seen and the jets'
	/// places. For a swapped world (respawn, savestate, a gap with no pool: `GameClock::update`) and the warp.
	pub(super) fn forget(&mut self) {
		self.blast.reset();
		self.jets.forget();
		self.jet_lerp.forget();
		self.ground_track.forget();
		self.burn.forget();
		self.sim.forget();
	}

	/// The fire kind and pool slot of sprite `si` of `set_particles`' list; None = draw it as the game does (always None while off).
	pub(super) fn sprite(&self, si: usize) -> Option<(FireKind, u16)> {
		if !self.on {
			return None;
		}
		self.picks.get(si).copied().flatten()
	}

	/// Takes a pool sprite over: a flame tongue (upright, base a little below the sprite, facing the eye) or a fireball. `retail` = the
	/// sprite's own quad and `part_verts` key, kept to give back on a frame where fire does not draw (`settle`). False = not taken (too
	/// many sprites): the caller draws the retail quad.
	pub(super) fn add_sprite(&mut self, pick: (FireKind, u16), s: &SpriteAt, retail: (PartKey, [Vertex; 4])) -> bool {
		if self.sprites.len() >= (MAX_QUADS - MAX_GROUND) * 4 {
			return false;
		}
		if s.colour[3] <= 0.004 {
			return true; // invisible: nothing to draw, nothing to give back
		}
		// The Pyrocitor (firefx_jet_quads.rs `JetState::plan` already decided): a `Jet` pick is a member of a joined stream, whose puffs are
		// in `sprites` since `begin_sprites`, so only its retail quad is kept to give back; the other jet kinds are round quads; a burn
		// puff is a flame tongue.
		let (kind, slot) = match pick {
			(FireKind::Jet, _) => {
				self.held.push(retail);
				return true;
			}
			// A burn puff: with volumes, one flame per cluster of puffs (firefx_burn.rs, `end_sprites`); else its own tongue card.
			(FireKind::Burn, slot) if volumes() => {
				self.held.push(retail);
				self.burn.add(super::firefx_burn::Puff { pos: s.pos, size: s.size, colour: s.colour, slot });
				return true;
			}
			(FireKind::Burn, slot) => (FireKind::Flame, slot),
			// An explosion (the fire simulation on, `classify`): a burst emitter under the volume cap, else the retail sprite.
			(FireKind::Blast, slot) => {
				if !volumes() || !blast_like(s.colour) || self.reserve + self.svols.len() >= firefx_vol::MAX_VOLS {
					return false;
				}
				self.held.push(retail);
				let tint = [s.colour[0], s.colour[1], s.colour[2]];
				// The size rule: the sim keeps the burst inside the sprite's own footprint x 1.2 (d[0], firesim_step.wgsl `leash`).
				let mut ball = firefx_vol::ball(s.pos, s.size * tune("fire_sim_blast", 0.6), (slot as u32 % 4096) as f32 / 4096.0, s.colour[3], tint);
				ball.d[0] = firefx_vol::FOOT_X * 0.5 * s.size;
				self.svols.push(ball);
				return true;
			}
			p => p,
		};
		// Only real fire is fire (user 2026-11-03, "why is blaster bullets rendered as fire"): the effect types of the flame rows are
		// shared with weapon effects (the Blaster's beam sprites, impact sparks and embers, the Devastator's muzzle smoke), so a flame
		// pick must also look like a flame (`flame_like`); else the caller draws it as retail does.
		if kind == FireKind::Flame && !flame_like(s.colour, s.size) {
			return false;
		}
		// The noise seed is the pool slot's (stable while the particle lives, whatever its spin); the shader uses seed * 4096.
		let seed = (slot as u32 % 4096) as f32 / 4096.0;
		let (pos, size, colour) = (s.pos, s.size, s.colour);
		let tint = [colour[0], colour[1], colour[2]];
		self.held.push(retail);
		// Round quads: a fireball (shader kind 2), a jet's sub-burst (smaller fireball), a jet's spark (kind 4: a small glowing dot).
		let round = match kind {
			FireKind::Burst => Some((size * 0.9, 2.0)),
			FireKind::JetBurst => Some((size * 0.9 * tune("fire_jet_sub", 0.7), 2.0)),
			FireKind::JetSpark => Some((size * tune("fire_spark_size", 0.5), 4.0)),
			_ => None,
		};
		// A fireball or a jet's sub-burst is a ball volume (firefx_vol.rs); a spark stays a glowing dot. Over the volume cap (shared with
		// the level's fires, `reserve`) a sprite stays a card.
		let vol_room = volumes() && self.reserve + self.svols.len() < firefx_vol::MAX_VOLS;
		if let Some((sz, _)) = round.filter(|r| r.1 < 3.0 && vol_room) {
			let mut ball = firefx_vol::ball(pos, sz, seed, colour[3], tint);
			ball.d[0] = firefx_vol::FOOT_X * 0.5 * size;
			self.svols.push(ball);
			return true;
		}
		if let Some((sz, shape)) = round {
			let (a, b) = (s.right, s.up);
			let c = |x: f32, y: f32, uv: [f32; 2]| Vertex { pos: std::array::from_fn(|k| pos[k] + (a[k] * x + b[k] * y) * sz), uv, normal: [0.0; 3], colour: [tint[0], tint[1], tint[2], colour[3]], light: [seed, 1.0, size, shape] };
			self.sprites.extend([c(-1.0, -1.0, [0.0, 1.0]), c(1.0, -1.0, [1.0, 1.0]), c(1.0, 1.0, [1.0, 0.0]), c(-1.0, 1.0, [0.0, 0.0])]);
			return true;
		}
		let across = normalise([s.right[0], s.right[1], 0.0]);
		// The size rule (user 2026-11-03, "the explosion on the ground is too big", RYNO's "massive oversized fire trail"): the flame
		// stands inside the retail sprite's square x FOOT_X (was 1.9 x its side tall from 0.4 below its middle: three times the sprite
		// above it). Foot at the square's bottom edge, top at FOOT_X x its half side above the middle.
		let base = [pos[0], pos[1], pos[2] - 0.5 * size];
		let h = size * (0.5 + 0.5 * firefx_vol::FOOT_X);
		if vol_room {
			self.svols.push(firefx_vol::flame(base, h, size * 0.55, seed, colour[3], tint));
		} else {
			self.sprites.extend(tongue(base, across, size * 1.1, h, tint, colour[3], seed, 1.0));
		}
		if h >= tune("fire_ember_min", 1.8) && self.big.len() < MAX_EMIT {
			self.big.push(Emit { base, h, w: size * 1.1, seed });
		}
		true
	}

	/// End of `Gpu::set_particles`' sprite loop: the frame's burn puffs become one flame per cluster (firefx_burn.rs), under the volume
	/// cap the level's fires keep (`reserve`); a flame over the cap is a tongue card (its puffs' retail quads are held).
	pub(super) fn end_sprites(&mut self) {
		let room = if self.on { firefx_vol::MAX_VOLS.saturating_sub(self.reserve + self.svols.len()) } else { 0 };
		let (cards, across, on) = (&mut self.sprites, self.across, self.on);
		let mut over = |f: &super::firefx_burn::Flame| {
			if on && cards.len() < (MAX_QUADS - MAX_GROUND) * 4 {
				cards.extend(tongue(f.base(), across, 2.0 * f.r, f.h, f.tint, f.fade, f.seed, 1.0));
			}
		};
		self.burn.flush(self.game.now(), room, &mut self.svols, &mut self.big, MAX_EMIT, &mut over);
	}

	/// Start of `draw_passes`, after `gather`: when fire does not draw this pass (post off, the warp, the Classic half of the split view)
	/// the sprites it took over go back into `part_verts`; when it draws, they stay out. Safe to call once per pass: it first removes what
	/// the previous pass gave back (`part_verts` is only rebuilt by `set_particles`, and the split view runs two passes on it).
	pub(super) fn settle(&mut self, verts: &mut BTreeMap<PartKey, Vec<Vertex>>) {
		for (key, quad) in self.given.drain(..) {
			if let Some(v) = verts.get_mut(&key) {
				remove_quad(v, &quad);
				if v.is_empty() {
					verts.remove(&key);
				}
			}
		}
		if self.on {
			return;
		}
		for (key, quad) in &self.held {
			verts.entry(*key).or_default().extend(*quad);
		}
		self.given = self.held.clone();
	}

	/// Start of `draw_passes`: decides whether fire draws this frame (`want`: Particles Enhanced pass with its targets, no warp) and builds
	/// the tongues of the level's fire quads (`quads` = the effect quads, `level` = the loaded level).
	pub(super) fn gather(&mut self, level: usize, quads: &[crate::prims::WorldQuad], cam: &Camera, want: bool) {
		self.quads.clear();
		self.vols.clear();
		self.ground.clear();
		self.lit.clear();
		self.took.clear();
		self.book.sprites.clear();
		self.on = want && active();
		self.lights = want && super::firefx_light::wanted() && !crate::viewer::pipe_guard::off(crate::viewer::pipe_guard::Effect::Fire);
		if !self.on {
			self.sim.idle();
			if self.lights {
				self.lit_retail(level, quads, cam);
			}
			return;
		}
		let rows = (crate::pack::get().fire_rows)(level);
		let f = [cam.pitch.cos() * cam.yaw.cos(), cam.pitch.cos() * cam.yaw.sin(), cam.pitch.sin()];
		let right = normalise(cross(f, cam.up));
		let across = normalise([right[0], right[1], 0.0]);
		let fires = quads.iter().enumerate().filter(|(_, q)| fire_option::quad_kind(rows, crate::prims::plain_tex(q.tex), q.additive) == Some(FireKind::Ground));
		// Tongues only from whole quads; the triangles are the mirror's copy of the same fires (firefx_ground.rs `is_tri`).
		let (tris, whole): (Vec<usize>, Vec<usize>) = fires.map(|(i, _)| i).partition(|&i| firefx_ground::is_tri(&quads[i]));
		let items = whole.iter().filter_map(|&i| ground_of(&quads[i]).map(|(e, tint, fade)| (i, e, tint, fade)));
		self.took = vec![false; quads.len()];
		let far = tune("fire_far", 120.0);
		// (Dev dump, RAC_TUNE fb_dump=1: the tongues before the cap too.)
		let all = if super::firebook_dump::on() { firefx_ground::tongues_of(items.clone(), cam.pos, far, usize::MAX) } else { Vec::new() };
		let mut tongues = firefx_ground::tongues_of(items, cam.pos, far, MAX_GROUND);
		// Volumes: the level's tongues (nearest first) share `firefx_vol::MAX_VOLS` with the sprites' volumes (R7-1); a tongue over the
		// cap is not drawn here and its quads stay retail (not `took`).
		let wanted = tongues.len();
		if volumes() {
			tongues.truncate(firefx_vol::MAX_VOLS.saturating_sub(self.svols.len()));
		}
		self.ground_track.follow(&mut tongues);
		// The mirror's triangles hide as far as the tongues reach (the nearest ones are kept over a cap: up to the last of them).
		let reach = if tongues.len() < wanted || wanted >= MAX_GROUND { tongues.last().map_or(0.0, |t| dist(t.emit.base, cam.pos)) } else { far };
		firefx_ground::hide_tris(quads, &tris, !whole.is_empty(), cam.pos, reach, &mut self.took);
		if super::firebook_dump::on() {
			super::firebook_dump::tongues(&all, &tongues, MAX_GROUND, firefx_vol::MAX_VOLS.saturating_sub(self.svols.len()), cam.pos, cam.yaw, cam.pitch);
			super::firebook_dump::quads(quads, &[&whole[..], &tris[..]].concat());
		}
		// With the simulation on, the tongues are flipbooks (firebook.rs) and the sim keeps only their embers.
		let book = volumes() && super::firesim::on() && super::firebook::FireBook::on();
		self.book.across = across;
		if book {
			super::firebook::prewarm(); // once per session, on the first frame with flipbook fire on (before any fire is in view): the flame atlas inflates and mips in the background, so the first draw does not wait for it
		}
		for t in tongues {
			if book {
				let (edge, side) = (t.emit.w / tune("fire_w", 1.25).max(0.1), t.emit.h / tune("fire_h", 1.0).max(0.1));
				super::firebook::place(t.emit.base, across, edge, side, t.emit.seed, t.fade, t.tint, fire_option::level(), &mut self.book.sprites);
				self.vols.push(firefx_vol::ember_tongue(firefx_vol::flame_own(t.emit.base, t.emit.h, t.emit.w * 0.5, t.emit.seed, t.fade, t.tint, t.noise)));
			} else if volumes() {
				self.vols.push(firefx_vol::level_tongue(firefx_vol::flame_own(t.emit.base, t.emit.h, t.emit.w * 0.5, t.emit.seed, t.fade, t.tint, t.noise)));
			} else {
				self.quads.extend(tongue(t.emit.base, across, t.emit.w, t.emit.h, t.tint, t.fade, t.emit.seed, 0.0));
			}
			// Embers only from a slot that has faded in (each slot fades in and out on its own: a fresh one is not a fire yet), the nearest
			// up to half the emitters (the jets', the burn flames' and the blast flames' keep the rest, review R25 m2); the lights weigh
			// every tongue by its fade, so a slot fading in or out does not step them.
			if t.fade > 0.3 && self.ground.len() < MAX_EMIT / 2 {
				self.ground.push(t.emit);
			}
			self.lit.push(Emit { w: t.emit.w * t.fade, ..t.emit });
			for &m in &t.members {
				self.took[m] = true;
			}
		}
		// The flames of the ground fires after explosions (nothing at Low, or with no patch); as volumes they take what is left of the cap.
		let mut blast = 0;
		if fire_option::level() >= 2 {
			let room = match volumes() {
				true => MAX_GROUND.min(firefx_vol::MAX_VOLS.saturating_sub(self.vols.len() + self.svols.len())),
				false => MAX_GROUND.saturating_sub(self.quads.len() / 4),
			};
			let emit_room = MAX_EMIT.saturating_sub(self.ground.len() + self.big.len());
			let (start, lit_from) = (self.quads.len(), self.ground.len());
			self.blast.flames(self.game.now(), fire_option::level(), across, &mut self.quads, &mut self.ground, room, emit_room);
			self.lit.extend_from_slice(&self.ground[lit_from..]);
			if volumes() {
				let cards: Vec<Vertex> = self.quads.drain(start..).collect();
				blast = cards.len() / 4;
				self.vols.extend(cards.chunks_exact(4).map(card_volume));
			}
		}
		self.reserve = if volumes() { wanted + blast } else { 0 };
	}

	/// True while effect quad `i` is replaced by this module's tongue: it is a member of a tongue drawn this frame (a flame billboard of a
	/// `Ground` row; the fire's grey columns are never skipped here: retail's quads, or Smoke's volumes, volsmoke/columns.rs). A quad
	/// that is flat, narrow, far or over the cap is not, and draws as retail.
	pub(super) fn skips(&self, i: usize) -> bool {
		self.on && self.took.get(i).copied().unwrap_or(false)
	}

	/// The dense cores of this frame's flames and fireballs (firefx_vol.rs `core`) for the smoke pass (volsmoke/fire_cut.rs: the
	/// smoke behind a flame must not cover it); none while the fire does not draw.
	pub(super) fn cores(&self) -> Vec<([f32; 3], f32, f32)> {
		if !self.on {
			return Vec::new();
		}
		self.vols.iter().chain(self.svols.iter()).map(firefx_vol::core).collect()
	}

	/// The draw clock of the fire shaders, the mesh flames and the High lights' flicker (`GameClock::shown`): game time that moves on
	/// every display frame, also between two fields.
	pub(super) fn shown(&self) -> f64 {
		self.game.shown(firefx_clock::age())
	}

	/// Before the particle pass: one step of the fire simulation from this frame's volumes (firesim.rs); off, Classic or the volumes'
	/// A/B (`fire_sim` 0) = nothing stepped, nothing drawn.
	pub(super) fn simulate(&mut self, env: &FireEnv<'_>, enc: &mut wgpu::CommandEncoder, cam: &Camera, at: &super::firesim::StepIn<'_>) {
		if !self.on || !volumes() || !super::firesim::on() {
			self.sim.idle();
			return;
		}
		let shown = self.shown();
		self.sim.step(env, enc, cam, self.vols.iter().chain(self.svols.iter()), shown, at);
	}

	/// The fire's lights (Global illumination level `gi`, firefx_light.rs) first, then the retail effect lights, `FX_LIGHTS` in all.
	pub(super) fn merge_lights(&self, retail: FxLights, gi: u8) -> FxLights {
		if !self.lights || super::firefx_light::count(gi) == 0 {
			return retail;
		}
		let mine = super::firefx_light::lights(self.lit.iter().chain(self.big.iter().filter(|_| self.on)), self.shown(), super::firefx_light::count(gi));
		super::firefx_light::log(self.lit.len(), self.big.len(), retail.n, &mine);
		let mut out = FxLights::default();
		for l in mine.iter().chain(retail.l[..retail.n].iter()).take(partfx::FX_LIGHTS) {
			out.l[out.n] = *l;
			out.n += 1;
		}
		out
	}
}

impl FireFx {
	/// Fire Off with the fire lights wanted: the level's flame billboards as the game draws them still light the scene (every tongue
	/// by its quad's fade, as `gather` weighs them; no tongue is drawn or taken).
	fn lit_retail(&mut self, level: usize, quads: &[crate::prims::WorldQuad], cam: &Camera) {
		let rows = (crate::pack::get().fire_rows)(level);
		let whole = quads.iter().enumerate().filter(|(_, q)| fire_option::quad_kind(rows, crate::prims::plain_tex(q.tex), q.additive) == Some(FireKind::Ground) && !firefx_ground::is_tri(q));
		let items = whole.filter_map(|(i, q)| ground_of(q).map(|(e, tint, fade)| (i, e, tint, fade)));
		for t in firefx_ground::tongues_of(items, cam.pos, tune("fire_far", 120.0), MAX_GROUND) {
			self.lit.push(Emit { w: t.emit.w * t.fade, ..t.emit });
		}
	}
}

impl Res {
	fn new(env: &FireEnv<'_>) -> Res {
		let device = env.device;
		let src = match std::env::var_os("RAC_FIRE_WGSL").and_then(|p| std::fs::read_to_string(p).ok()) {
			Some(s) => std::borrow::Cow::Owned(s),
			None => std::borrow::Cow::Borrowed(WGSL),
		};
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("fire"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("fire", src)) });
		let vis = wgpu::ShaderStages::VERTEX_FRAGMENT;
		let own = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("fire"),
			entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: vis, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
		});
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("fire"), size: (ROWS * 16) as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let ubind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("fire"), layout: &own, entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }] });
		let l0 = env.pipeline.get_bind_group_layout(0);
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fire"), bind_group_layouts: &[Some(&l0), Some(&own), Some(env.partfx.soft_layout())], immediate_size: 0 });
		// Premultiplied colour (the flame adds light, smoke darkens); the glow target adds, as the additive particles do.
		let pre = wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add } };
		let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
		let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4];
		let make = |label: &str, vs: &str, fs: &str, buffers: &[Option<wgpu::VertexBufferLayout<'_>>]| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(label),
				layout: Some(&pl),
				vertex: wgpu::VertexState { module: &module, entry_point: Some(vs), compilation_options: Default::default(), buffers },
				fragment: Some(wgpu::FragmentState {
					module: &module,
					entry_point: Some(fs),
					compilation_options: Default::default(),
					targets: &[Some(wgpu::ColorTargetState { format: env.format, blend: Some(pre), write_mask: wgpu::ColorWrites::ALL }), Some(wgpu::ColorTargetState { format: partfx::GLOW_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::ALL })],
				}),
				primitive: Default::default(),
				depth_stencil: Some(wgpu::DepthStencilState { format: env.depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
				multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
				multiview_mask: None,
				cache: None,
			})
		};
		let vl = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs };
		let flame = make("fire flame", "vs_fire", "fs_fire", &[Some(vl)]);
		let ember = make("fire embers", "vs_ember", "fs_ember", &[]);
		let idx: Vec<u32> = (0..MAX_QUADS as u32).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
		let ibuf = buffer(device, "fire indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
		env.queue.write_buffer(&ibuf, 0, bytemuck::cast_slice(&idx));
		let vol = firefx_vol::VolRes::new(env);
		Res { flame, ember, ubuf, ubind, vbuf: Default::default(), ibuf, vol }
	}
}

/// The settings and emitter rows of the shader's uniform block for a frame.
fn uniform_rows(level: u8, clock: Clock, right: [f32; 3], up: [f32; 3], emitters: &[Emit]) -> (Vec<[f32; 4]>, u32) {
	let embers = match level {
		1 => 0.0,
		2 => 4.0,
		_ => 8.0,
	} * tune("fire_embers", 1.0);
	let per = if embers > 0.0 { embers.round().max(1.0) } else { 0.0 };
	let oct = (level + 1).min(4) as f32;
	let mut rows = vec![
		[clock.t, oct, per.max(1.0), tune("fire_ember_size", 1.0)],
		// (k.x: how fast a jet's pattern flows along it, m/s; the speed knob scales the clock, `Clock::game`.)
		[tune("fire_jet_flow", 5.0), tune("fire_turb", 1.0), tune("fire_core", 1.0), tune("fire_glow", 1.0)],
		// (m.w: the blend weight of the pattern one PERIOD earlier, see firefx_clock.rs.)
		[tune("fire_tint", 0.35), tune("fire_smoke", 1.0), 1.0, clock.blend],
		// (right.w: how fast a jet cools along its length, `fire_jet_core`; up.w: the strength of its warp, `fire_jet_turb`.)
		[right[0], right[1], right[2], tune("fire_jet_core", 1.25)],
		[up[0], up[1], up[2], tune("fire_jet_turb", 1.0)],
	];
	for e in emitters.iter().take(MAX_EMIT) {
		rows.push([e.base[0], e.base[1], e.base[2], e.h]);
		rows.push([e.seed, e.w, 1.0, 0.0]);
	}
	rows.resize(ROWS, [0.0; 4]);
	(rows, per as u32)
}

impl FireFx {
	/// Draws the frame's fire inside the particle pass (after the pool, before the pool lines): the flames, then the embers.
	pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, env: &FireEnv<'_>, cam: &Camera) {
		if !self.on || env.partfx.soft_bind().is_none() {
			return;
		}
		// Warm-up (review R7-3): the pipelines and the noise are made on the first frame the fire is on (the level's first frame, or
		// the frame the option is switched on), not mid-game when the first flame comes into view.
		if self.res.get().is_none() {
			if let Some(r) = crate::viewer::pipe_guard::guard(env.device, crate::viewer::pipe_guard::Effect::Fire, || Res::new(env)) {
				let _ = self.res.set(r);
			}
		}
		super::firefx_blast_gpu::warm(&self.blast, env);
		let shown = self.shown();
		// The ground fires' scorch and embers go in first (under the flames); nothing unless a patch is alive (Medium, High).
		super::firefx_blast_gpu::draw(&self.blast, self.game.now(), shown, pass, env);
		// The simulated fire (firesim.rs) replaces the volumes; its smoke outlives the emitters, so it draws before the early return.
		let sim = volumes() && super::firesim::on();
		if sim {
			self.sim.draw(pass);
			let mut book = std::mem::take(&mut *self.book_order.borrow_mut());
			book.clear();
			book.extend_from_slice(&self.book.sprites);
			super::firebook::sort(&mut book, cam.pos);
			if super::firebook_dump::on() {
				super::firebook_dump::sprites(&book);
			}
			self.book.draw(pass, env, cam, shown, &book);
			*self.book_order.borrow_mut() = book;
		}
		let n =(self.quads.len() + self.sprites.len()) / 4;
		let fires = self.ground.len() + self.big.len();
		if n == 0 && fires == 0 && self.vols.is_empty() && self.svols.is_empty() {
			return;
		}
		// (A driver that refused the shader keeps the retail fire: pipe_guard marks the effect off and the draw ends here.)
		let Some(res) = self.res.get() else { return };
		let f = [cam.pitch.cos() * cam.yaw.cos(), cam.pitch.cos() * cam.yaw.sin(), cam.pitch.sin()];
		let right = normalise(cross(f, cam.up));
		let up = cross(right, f);
		let emitters: Vec<Emit> = self.ground.iter().chain(self.big.iter()).copied().collect();
		let clock = Clock::game(shown);
		let (rows, per) = uniform_rows(fire_option::level(), clock, right, up, &emitters);
		// The volumes first (far to near, the nearest kept should there be too many), then the cards (jets, sparks, the ground fire's
		// flames), then the embers.
		if !sim {
			let mut vols = self.order.borrow_mut();
			vols.clear();
			vols.extend(self.vols.iter().chain(self.svols.iter()));
			firefx_vol::nearest(&mut vols, cam.pos);
			if tune("fire_vol_log", 0.0) > 0.5 {
				firefx_vol::log(&vols, cam);
			}
			res.vol.draw(pass, env, &vols, &firefx_vol::rows(fire_option::level(), clock.t, cam.pos, f, cam.tan_half));
		}
		env.queue.write_buffer(&res.ubuf, 0, bytemuck::cast_slice(&rows));
		pass.set_bind_group(1, &res.ubind, &[]);
		let verts: Vec<Vertex> = self.quads.iter().chain(self.sprites.iter()).take(MAX_QUADS * 4).copied().collect();
		if !verts.is_empty() {
			let bytes: &[u8] = bytemuck::cast_slice(&verts);
			let mut cell = res.vbuf.borrow_mut();
			if cell.as_ref().is_none_or(|b| b.1 < bytes.len() as u64) {
				let size = (bytes.len() as u64).next_power_of_two();
				let b = env.device.create_buffer(&wgpu::BufferDescriptor { label: Some("fire vertices"), size, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
				*cell = Some((b, size));
			}
			let vb = &cell.as_ref().expect("fire vertex buffer").0;
			env.queue.write_buffer(vb, 0, bytes);
			pass.set_pipeline(&res.flame);
			pass.set_vertex_buffer(0, vb.slice(..bytes.len() as u64));
			pass.set_index_buffer(res.ibuf.slice(..), wgpu::IndexFormat::Uint32);
			pass.draw_indexed(0..(verts.len() / 4 * 6) as u32, 0, 0..1);
		}
		if per > 0 && !emitters.is_empty() {
			pass.set_pipeline(&res.ember);
			pass.draw(0..6, 0..emitters.len().min(MAX_EMIT) as u32 * per);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The firefx shader parses and validates (naga) with all four entry points; the rust uniform rows match the shader block.
	#[test]
	fn fire_shader_validates() {
		let m = naga::front::wgsl::parse_str(WGSL).unwrap_or_else(|e| panic!("fire: {}", e.emit_to_string(WGSL)));
		let names: Vec<_> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
		for want in ["vs_fire", "fs_fire", "vs_ember", "fs_ember"] {
			assert!(names.contains(&want), "missing entry {want}");
		}
		let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
		v.validate(&m).unwrap_or_else(|e| panic!("fire validates: {e:?}"));
		assert_eq!(ROWS * 16, 5 * 16 + 256 * 16);
	}

	/// DX12 parity: the fire shader must also go through naga's HLSL writer (the same check as metalenv::tests::level_shader_writes_hlsl;
	/// FXC itself is not run, the picture pass checks a DX12 run).
	#[cfg(windows)]
	#[test]
	fn fire_shader_writes_hlsl() {
		let m = naga::front::wgsl::parse_str(WGSL).expect("wgsl parse");
		let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
		let (options, pipeline) = (naga::back::hlsl::Options::default(), naga::back::hlsl::PipelineOptions::default());
		let mut out = String::new();
		naga::back::hlsl::Writer::new(&mut out, &options, &pipeline).write(&m, &info, None).expect("HLSL writes");
		assert!(out.contains("fs_fire") && out.contains("fs_ember"), "the fire entry points are in the HLSL");
	}

	fn v(x: f32) -> Vertex {
		Vertex { pos: [x, 0.0, 0.0], uv: [0.0; 2], normal: [0.0; 3], colour: [1.0; 4], light: [0.0; 4] }
	}

	fn at(pos: [f32; 3], size: f32, alpha: f32) -> SpriteAt {
		SpriteAt { pos, size, colour: [1.0, 0.6, 0.2, alpha], right: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0] }
	}

	/// The x of every vertex per run key (a Vertex has no PartialEq).
	fn snap(m: &BTreeMap<PartKey, Vec<Vertex>>) -> Vec<(PartKey, Vec<f32>)> {
		m.iter().map(|(k, v)| (*k, v.iter().map(|p| p.pos[0]).collect())).collect()
	}

	/// A retail quad to give back: four vertices at x = `x`.
	fn retail(x: f32) -> (PartKey, [Vertex; 4]) {
		((7, true, 0x2000 + 3), [v(x), v(x + 0.25), v(x + 0.5), v(x + 0.75)])
	}

	/// User 2026-11-04: the explosions' ground fire was off by a knob default of 0; the shipped default must stay on.
	#[test]
	fn explosions_leave_ground_fire_by_default() {
		assert!(ground_blast_on(), "fire_ground_blast defaults to 1: explosions leave flames and burn the grass");
	}

	#[test]
	fn off_takes_and_skips_nothing() {
		let f = FireFx::default();
		assert!(!f.on);
		assert_eq!(f.sprite(0), None);
		assert!(!f.skips(0));
		let l = FxLights::default();
		assert_eq!(f.merge_lights(l, 3).n, 0, "no fire lights while gather found none to want");
		// Off: settle changes nothing.
		let mut f = FireFx::default();
		let mut verts = BTreeMap::new();
		f.settle(&mut verts);
		assert!(verts.is_empty());
		// Off: no ground fire patch, no decal work (nothing to draw, no uniform buffer) and no tongues from `gather` (want = false).
		assert!(f.ground_patches().is_empty() && f.blast.patches().is_empty() && f.patch_buffer().is_none());
		let cam = Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: 0.5, up: [0.0, 0.0, 1.0] };
		f.gather(0, &[], &cam, false);
		assert!(!f.on && f.quads.is_empty() && f.ground.is_empty() && f.ground_patches().is_empty());
	}

	#[test]
	fn sprites_become_tongues_and_fireballs() {
		let mut f = FireFx::default();
		assert!(f.add_sprite((FireKind::Flame, 5), &at([1.0, 2.0, 3.0], 2.0, 0.5), retail(0.0)));
		// (Volumes by default, `fire_vol`: a flame volume and a fireball volume, no card quads.)
		assert_eq!((f.sprites.len(), f.svols.len()), (0, 1));
		assert_eq!(f.svols[0].b[3], 0.0, "a flame volume");
		assert!((f.svols[0].a[2] - (3.0 - 1.0)).abs() < 1e-4 && (f.svols[0].a[3] - 2.2).abs() < 1e-4, "the size rule: foot at the square's bottom, top 1.2 x its half side above the middle");
		assert_eq!(f.big.len(), 1, "a tall flame also sends embers");
		assert!(f.add_sprite((FireKind::Burst, 6), &at([0.0; 3], 1.0, 1.0), retail(1.0)));
		assert_eq!((f.sprites.len(), f.svols.len(), f.svols[1].b[3]), (0, 2, 1.0), "a fireball volume");
		assert_eq!(f.held.len(), 2, "both retail quads are kept to give back");
		// A dead sprite is taken (nothing to see) but adds and holds nothing; small flames send no embers.
		assert!(f.add_sprite((FireKind::Flame, 7), &at([0.0; 3], 0.2, 0.0), retail(2.0)));
		assert_eq!((f.svols.len(), f.held.len(), f.big.len()), (2, 2, 1));
	}

	/// The Pyrocitor's kinds (firefx_jet_quads.rs decided which sprites keep them): a joined stream segment only keeps its retail quad
	/// (the jet's puffs are in `sprites` already), a sub-burst is a smaller fireball, a spark is a small dot (shader kind 4), a burn puff a tongue.
	#[test]
	fn the_jet_kinds_draw_their_shapes() {
		let mut f = FireFx::default();
		assert!(f.add_sprite((FireKind::Jet, 1), &at([0.0; 3], 1.0, 0.8), retail(0.0)));
		assert_eq!((f.sprites.len(), f.held.len()), (0, 1), "the member adds no quad of its own, its retail quad is kept to give back");
		assert!(f.add_sprite((FireKind::Burst, 2), &at([0.0; 3], 1.0, 0.8), retail(1.0)));
		assert!(f.add_sprite((FireKind::JetBurst, 3), &at([0.0; 3], 1.0, 0.8), retail(2.0)));
		assert!(f.add_sprite((FireKind::JetSpark, 4), &at([0.0; 3], 1.0, 0.8), retail(3.0)));
		assert!(f.add_sprite((FireKind::Burn, 5), &at([0.0; 3], 1.0, 0.8), retail(4.0)));
		// (Volumes by default: fireball, smaller fireball and the burn tongue are volumes; only the spark is a quad, shader kind 4.)
		assert_eq!((f.sprites.len(), f.sprites[0].light[3]), (4, 4.0));
		assert_eq!(f.svols.len(), 2, "the burn puff waits for its cluster (firefx_burn.rs)");
		f.on = true;
		f.end_sprites();
		let kinds: Vec<f32> = f.svols.iter().map(|v| v.b[3]).collect();
		assert_eq!(kinds, vec![1.0, 1.0, 0.0], "fireball, smaller fireball, the burn cluster's flame");
		assert!((f.svols[0].b[0] - 0.9).abs() < 1e-5, "a stream fireball keeps today's size");
		assert!((f.svols[1].b[0] - 0.9 * 0.7).abs() < 1e-5, "sub-burst");
		assert!((f.sprites[2].pos[0] - f.sprites[0].pos[0] - 1.0).abs() < 1e-5, "spark: 0.5 x the sprite's size each way");
		assert_eq!(f.held.len(), 5);
	}

	/// With no fire rows (another game, a level without them) nothing is classified, joined or taken; the Jet lists stay empty.
	#[test]
	fn no_fire_rows_means_nothing_is_taken() {
		let sprites: Vec<crate::prims::Sprite> = (0..6).map(|i| crate::prims::Sprite { pos: [i as f32 * 0.1, 0.0, 1.0], size: 1.0, rot: 0.0, colour: [1.0, 0.6, 0.2, 0.8], tex: 0, additive: true, pool: Some(0xF0), flat: false }).collect();
		let (types, slots) = ([12u8; 6], [0u16, 1, 2, 3, 4, 5]);
		let fx = crate::fxpool::PoolFx { sprite_slots: &slots, sprite_types: &types, sprite_texs: &[], ribbons: &[], lines: &[], line_slots: &[], frames: 0, hero: None, movers: &[], shots: &[], blasts: &[], tick: &[], tick_hero: None };
		let mut f = FireFx::default();
		let view = View { eye: [0.0, -10.0, 1.0], fwd: [0.0, 1.0, 0.0], up: [0.0, 0.0, 1.0], far: 200.0 };
		f.begin_sprites(0, &sprites, Some(&fx), &view);
		assert!(f.sprites.is_empty() && f.big.is_empty() && f.held.is_empty());
		assert!((0..6).all(|i| f.sprite(i).is_none()));
	}

	/// The noise seed is the pool slot's: the same slot keeps its flame whatever it does, another slot gets another one (A11).
	#[test]
	fn the_seed_is_the_pool_slot() {
		let seed = |slot: u16, x: f32| {
			let mut f = FireFx::default();
			f.add_sprite((FireKind::Flame, slot), &at([x, 0.0, 0.0], 2.0, 0.5), retail(0.0));
			f.svols[0].b[1]
		};
		assert_eq!(seed(40, 0.0), seed(40, 9.5), "moving or spinning does not change it");
		assert_ne!(seed(40, 0.0), seed(41, 0.0));
		assert!((seed(4096 + 3, 0.0) - 3.0 / 4096.0).abs() < 1e-9 && seed(2047, 0.0) < 1.0);
	}

	/// Only POOL sprites are counted for the pool's parallel type list, and a type that is not a fire row stays retail (A21).
	#[test]
	fn sprites_are_classified_by_effect_type_over_pool_sprites_only() {
		let rows = [fire_option::FireRow { key: 19, kind: FireKind::Flame }, fire_option::FireRow { key: 12, kind: FireKind::Burst }, fire_option::FireRow { key: 46, kind: FireKind::Ground }];
		let pool = [false, true, true, false, true, true];
		let got = classify(&rows, pool.into_iter(), &[19, 5, 12, 46], &[100, 101, 102, 103], false);
		assert_eq!(got, vec![None, Some((FireKind::Flame, 100)), None, None, Some((FireKind::Burst, 102)), None], "type 5 stays retail, a Ground key is no sprite row; the 4th pool sprite has no type");
		assert!(classify(&rows, pool.into_iter(), &[], &[], false).iter().all(Option::is_none), "no type list (a build that fills none): everything retail");
	}

	/// A4 / A14: sprites taken over go back to the particle pass on every pass where fire does not draw, and out again when it does.
	#[test]
	fn settle_gives_the_sprites_back_when_fire_does_not_draw() {
		let mut f = FireFx::default();
		f.on = true;
		f.add_sprite((FireKind::Flame, 1), &at([0.0; 3], 2.0, 0.5), retail(0.0));
		f.add_sprite((FireKind::Burst, 2), &at([1.0; 3], 2.0, 0.5), retail(1.0));
		let mut verts: BTreeMap<PartKey, Vec<Vertex>> = BTreeMap::new();
		let key = retail(0.0).0;
		verts.entry(key).or_default().extend([v(9.0), v(9.0), v(9.0), v(9.0)]); // another sprite of the same run
		let before = verts.clone();
		f.settle(&mut verts);
		assert_eq!(snap(&verts), snap(&before), "fire draws: the quads stay out");
		// Post off / warp: not drawn this pass, the quads are back (after the run's own vertices).
		f.on = false;
		f.settle(&mut verts);
		assert_eq!(verts[&key].len(), 12);
		assert_eq!(verts[&key][4].pos[0], 0.0);
		// The Enhanced half of the split view comes next: they leave again, the other vertices stay.
		f.on = true;
		f.settle(&mut verts);
		assert_eq!(snap(&verts), snap(&before));
		// And the Classic half again: back again.
		f.on = false;
		f.settle(&mut verts);
		f.settle(&mut verts);
		assert_eq!(verts[&key].len(), 12, "a repeated settle does not double the quads");
	}

	#[test]
	fn too_many_sprites_stay_retail_quads() {
		let mut f = FireFx::default();
		// (The first MAX_VOLS fireballs are volumes, the rest round quads until the quad cap.)
		let cap = MAX_QUADS - MAX_GROUND + firefx_vol::MAX_VOLS;
		for k in 0..cap {
			assert!(f.add_sprite((FireKind::Burst, k as u16), &at([0.0; 3], 1.0, 1.0), retail(0.0)));
		}
		assert!(!f.add_sprite((FireKind::Burst, 1), &at([0.0; 3], 1.0, 1.0), retail(0.0)), "the caller draws its retail quad");
		assert!(f.quads.len() + f.sprites.len() <= MAX_QUADS * 4, "the draw never cuts a quad");
	}

	/// R7-1: the sprites' volumes share MAX_VOLS with the level's (`reserve`): over it a fireball stays a round card and a flame a tongue
	/// card (drawn, never dropped), and the frame never holds more volumes than the cap.
	#[test]
	fn sprite_volumes_leave_the_level_its_share_of_the_cap() {
		let mut f = FireFx::default();
		f.reserve = firefx_vol::MAX_VOLS - 3;
		for k in 0..3 {
			assert!(f.add_sprite((FireKind::Burst, k), &at([0.0; 3], 1.0, 1.0), retail(0.0)));
		}
		assert_eq!((f.svols.len(), f.sprites.len()), (3, 0));
		assert!(f.add_sprite((FireKind::Burst, 9), &at([0.0; 3], 1.0, 1.0), retail(0.0)));
		assert!(f.add_sprite((FireKind::Flame, 10), &at([0.0; 3], 1.0, 1.0), retail(0.0)));
		assert_eq!((f.svols.len(), f.sprites.len()), (3, 8), "over the cap: a round card and a tongue card");
		assert_eq!(f.held.len(), 5, "every retail quad is held (the cards draw in their place)");
		assert!(f.reserve + f.svols.len() <= firefx_vol::MAX_VOLS);
	}

	/// A2: only the quads that are members of a drawn tongue are skipped; the rest draw as retail.
	#[test]
	fn skips_follow_the_took_vector() {
		let mut f = FireFx::default();
		f.on = true;
		f.took = vec![true, false, true];
		assert!(f.skips(0) && !f.skips(1) && f.skips(2) && !f.skips(3));
		f.on = false;
		assert!(!f.skips(0));
	}

	#[test]
	fn quality_levels_pick_octaves_and_embers() {
		let e = [Emit { base: [0.0; 3], h: 1.0, w: 1.0, seed: 0.0 }];
		let c = Clock { t: 0.0, blend: 0.0 };
		let (r1, p1) = uniform_rows(1, c, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], &e);
		let (r2, p2) = uniform_rows(2, c, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], &e);
		let (r3, p3) = uniform_rows(3, c, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], &e);
		assert_eq!((r1[0][1], r2[0][1], r3[0][1]), (2.0, 3.0, 4.0));
		assert_eq!((p1, p2, p3), (0, 4, 8));
		assert_eq!(r3.len(), ROWS);
	}

	/// The jet's knobs ride the lanes the shader reads (jet_at): flow in k.x, cooling in right.w, warp in up.w; the embers' camera axes stay.
	#[test]
	fn the_jet_knobs_ride_the_free_lanes() {
		let (r, _) = uniform_rows(2, Clock { t: 0.0, blend: 0.0 }, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], &[]);
		assert_eq!((r[1][0], r[3][3], r[4][3]), (5.0, 1.25, 1.0));
		assert_eq!((r[3][..3].to_vec(), r[4][..3].to_vec()), (vec![1.0, 0.0, 0.0], vec![0.0, 0.0, 1.0]));
		let wgsl = include_str!("firefx.wgsl");
		assert!(wgsl.contains("fu.k.x") && wgsl.contains("fu.right.w") && wgsl.contains("fu.up.w"), "the shader reads the three lanes");
	}

	/// The clock rides in row 0 (t.x) and the blend weight in row 2 (m.w): the shader's `fu.t.x` and `fu.m.w`.
	#[test]
	fn the_clock_rows_carry_time_and_blend() {
		let e = [Emit { base: [0.0; 3], h: 1.0, w: 1.0, seed: 0.0 }];
		let (r, _) = uniform_rows(3, Clock { t: 77.5, blend: 0.25 }, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], &e);
		assert_eq!((r[0][0], r[2][3]), (77.5, 0.25));
	}

	/// Review LOW-6: with Fire Off the lights flicker on the game clock (frozen without a new field), not on the wall clock.
	#[test]
	fn fire_off_lights_hold_still_while_the_game_clock_stands() {
		let mut f = FireFx::default();
		f.lights = true;
		f.lit.push(Emit { base: [1.0, 2.0, 3.0], h: 2.0, w: 2.0, seed: 0.0 });
		let a = f.merge_lights(Default::default(), 3);
		std::thread::sleep(std::time::Duration::from_millis(80));
		let b = f.merge_lights(Default::default(), 3);
		assert!(a.n == 1 && a.l[0] == b.l[0], "no field passed: the light does not move");
	}

	#[test]
	fn the_jets_are_built_from_the_fields_own_pool_sprites() {
		let sp = |x: f32, pool: bool| crate::prims::Sprite { pos: [x, 0.0, 0.0], size: 1.0, rot: 0.0, colour: [1.0; 4], tex: 0, additive: true, pool: pool.then_some(0), flat: false };
		let shown = [sp(0.5, true), sp(9.0, false), sp(1.5, true)];
		let tick = [sp(1.0, true), sp(2.0, true)];
		let mut out = Vec::new();
		unblend(&mut out, &shown, &tick);
		let xs: Vec<f32> = out.iter().map(|s| s.pos[0]).collect();
		assert_eq!(xs, vec![1.0, 9.0, 2.0], "the n-th pool sprite takes the field's n-th, the others stay");
	}
}
