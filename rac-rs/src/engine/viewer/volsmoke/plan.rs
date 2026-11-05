//! What `set_particles` (gpu/particle_fx.rs) does with the pool's smoke sprites while Smoke is on: finds them by the pack's rows
//! (`Pack::smoke`: effect type + particle texture of each pool slot), turns each into a raw volume (`volumes::Raw`) and holds the
//! sprite's quad back instead of drawing it. `VolSmoke::finish` keeps both; `VolSmoke::settle` gives the quads back when the volumes
//! cannot be drawn this frame. Smoke Off (or a pack without rows): `Plan::off()`, which touches nothing.

use super::volumes::Raw;
use super::*;
use crate::smoke_rows::{self, SmokeRow};

/// `part_verts` key: (run, additive, texture key).
pub(in crate::viewer) type Key = (u32, bool, u32);

const NONE: u8 = 0xff;

/// A sprite quad held back for a volume.
#[derive(Clone, Copy)]
pub(in crate::viewer) struct Held {
	pub key: Key,
	pub quad: [Vertex; 4],
	/// Index of its volume in `Plan::raw` (None = it faded out and has no volume: it stays hidden).
	pub vol: Option<u32>,
	/// Share of the sprite that is drawn anyway (a puff in the size band where the retail sprite hands over to the volume,
	/// `trail_share`): the sprite at alpha x `keep` plus the volume at share (1 - `keep`), `Raw::share`. 0 = the volume alone.
	pub keep: f32,
}

/// [PC] The smallest drawn side (metres) at which a smoke puff starts to become a volume (RAC_TUNE `smoke_min_size`). Small fast
/// trails (the jet-pack nozzles' exhaust, gun and crate puffs, the first frames of a blast's puffs) stay the retail sprites: at the
/// grid's resolution a puff that small can only be a soft ball, while the retail texture carries its shape and motion (user report,
/// test build 9: "the smoke is also blobs ... the classic smoke from jetpack nozzles looks way better"). Over `TRAIL_BAND` times this
/// size the sprite fades out as the volume fades in (no pop while a puff grows), from there on the volume alone.
pub(in crate::viewer) const MIN_SIZE: f32 = 1.0;
pub(in crate::viewer) const TRAIL_BAND: f32 = 1.6;

/// Share of a puff drawn `size` wide that becomes a volume (0 = the retail sprite alone, 1 = the volume alone; smooth between).
pub(in crate::viewer) fn trail_share(size: f32, min_size: f32) -> f32 {
	if min_size <= 0.0 {
		return 1.0;
	}
	let x = ((size - min_size) / (min_size * (TRAIL_BAND - 1.0))).clamp(0.0, 1.0);
	x * x * (3.0 - 2.0 * x)
}

pub(in crate::viewer) struct Plan {
	/// Per sprite: the row of the pack's table or NONE.
	rows: Vec<u8>,
	table: &'static [SmokeRow],
	cam: [f32; 3],
	range: f32,
	albedo: f32,
	min_size: f32,
	pub(super) raw: Vec<Raw>,
	pub(super) held: Vec<Held>,
}

impl Plan {
	/// No smoke volumes: every sprite is drawn as before.
	pub(in crate::viewer) fn off() -> Plan {
		Plan { rows: Vec::new(), table: &[], cam: [0.0; 3], range: 0.0, albedo: 1.0, min_size: 0.0, raw: Vec::new(), held: Vec::new() }
	}

	/// The plan of a frame: `off()` unless Smoke is on, the pack has rows and the pool tells each sprite's type.
	pub(in crate::viewer) fn new(sprites: &[crate::prims::Sprite], fx: Option<&crate::fxpool::PoolFx<'_>>, cam: [f32; 3]) -> Plan {
		let table = crate::pack::get().smoke;
		let Some(fx) = fx else { return Plan::off() };
		if crate::smoke_option::level() == 0 || table.is_empty() || fx.sprite_types.is_empty() {
			return Plan::off();
		}
		let mut n = 0;
		let rows = sprites
			.iter()
			.map(|sp| {
				if sp.pool.is_none() {
					return NONE;
				}
				let at = n;
				n += 1;
				let row = fx.sprite_types.get(at).and_then(|&t| smoke_rows::find(table, t, fx.sprite_texs.get(at).copied().unwrap_or(0)));
				row.map_or(NONE, |r| r as u8)
			})
			.collect();
		let t = crate::devtune::f32;
		let min_size = t("smoke_min_size", MIN_SIZE);
		Plan { rows, table, cam, range: t("smoke_range", 150.0), albedo: t("smoke_albedo", 0.75), min_size, raw: Vec::new(), held: Vec::new() }
	}

	/// Sprite `i`, drawn `size` wide in `colour` (faded), becomes a volume and its `quad` is held back: true. False = draw the quad
	/// (a fire ball sharing the smoke's type, a puff beyond the smoke range, or a small trail puff: `trail_share`).
	pub(in crate::viewer) fn take(&mut self, i: usize, sp: &crate::prims::Sprite, size: f32, colour: [f32; 4], key: Key, quad: [Vertex; 4]) -> bool {
		// An additive sprite (the game's default blend, 0x48) glows; as an alpha-over volume it would turn into grey fog (A13).
		if key.1 {
			return false;
		}
		let Some(&row) = self.rows.get(i).filter(|&&r| r != NONE) else { return false };
		let r = &self.table[row as usize];
		if r.grey_only && smoke_rows::saturation([colour[0], colour[1], colour[2]]) > smoke_rows::GREY_LIMIT {
			return false;
		}
		let radius = size * r.radius;
		let d = (0..3).map(|k| (sp.pos[k] - self.cam[k]) * (sp.pos[k] - self.cam[k])).sum::<f32>().sqrt();
		if d - radius > self.range {
			return false;
		}
		let share = trail_share(size, self.min_size);
		if share <= 0.0 {
			return false;
		}
		// The sprite's fade is the volume's thickness (and age); the band's share scales its density after the clumping (`Raw::share`).
		let fade = (colour[3] / 0.8).clamp(0.0, 1.5);
		let vol = (colour[3] >= 0.02).then(|| self.raw.len() as u32);
		if vol.is_some() {
			self.raw.push(Raw {
				pos: sp.pos,
				radius,
				thick: r.density * fade,
				albedo: [colour[0].min(1.0) * self.albedo, colour[1].min(1.0) * self.albedo, colour[2].min(1.0) * self.albedo],
				share,
				age: 1.0 - fade.min(1.0),
				rise: r.rise,
				billow: r.billow,
				tall: 1.0,
			});
		}
		self.held.push(Held { key, quad, vol, keep: 1.0 - share });
		true
	}
}

#[cfg(test)]
impl Plan {
	/// A plan over hand-made rows (the tests): the smoke table row of each pool slot.
	pub(super) fn with_rows(rows: Vec<u8>, table: &'static [SmokeRow], cam: [f32; 3]) -> Plan {
		Plan { rows, table, cam, range: 150.0, albedo: 0.75, min_size: MIN_SIZE, raw: Vec::new(), held: Vec::new() }
	}
}
