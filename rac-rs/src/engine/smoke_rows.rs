//! The smoke a game pack tells the engine about (`Pack::smoke`): which effect-pool slots are smoke puffs, as table rows. The engine
//! never names a game: a row says "pool slots of effect type `kind` (slot byte +0) drawn with particle texture `tex` (slot byte +2)
//! are smoke of this size and thickness". viewer/volsmoke.rs turns every matching slot into a volume while Smoke is not Off and
//! hides that slot's sprite. A pack without rows (the default, R&C2) never gets smoke volumes.

/// Any particle texture.
pub const ANY_TEX: u8 = 0xff;

/// One kind of smoke puff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmokeRow {
	/// The pool slot's effect type (byte +0).
	pub kind: u8,
	/// The slot's particle texture (byte +2), or `ANY_TEX`.
	pub tex: u8,
	/// Skip a sprite whose colour is strongly saturated (a fire ball or flame that shares the type with the grey smoke).
	pub grey_only: bool,
	/// Volume radius as a share of the sprite's drawn side (a soft round puff reaches about 0.5).
	pub radius: f32,
	/// Thickness through the centre at sprite alpha 1 (extinction per world unit times the radius; 1 = a puff that is half see-through).
	pub density: f32,
	/// How fast the billows rise through the volume, world units per second.
	pub rise: f32,
	/// How ragged the volume's edge gets (0 smooth .. 1 torn), on top of the age.
	pub billow: f32,
}

/// One kind of ALPHA-BLENDED world quad that is smoke (`Pack::smoke_quads`): the grey columns and volume sheets of a level fire (a
/// burning building), which the game draws as flat textured quads, not as pool sprites. While Smoke is not Off such quads are left
/// out of the quad pass and the quads of each emitter become one rising plume (viewer/volsmoke/columns.rs, plume.rs; Smoke Off:
/// drawn as retail).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadRow {
	/// The renderer's texture key of the quad (the fx table index; clamp / no-mip variant bits ignored). Additive quads never match.
	pub key: u32,
	/// The quad's vertex alpha (the largest of its four corners) at full fade: the volume's thickness follows alpha / `alpha`.
	pub alpha: f32,
	/// The quad's reach as a share of its drawn side (square root of mean width times height): a quad farther than the smoke range
	/// by more than that stays a quad.
	pub radius: f32,
	/// Thickness through the centre at full alpha (as `SmokeRow::density`): the plume's puffs take its emitter's mean.
	pub density: f32,
	/// Not read for quads (a plume's puffs rise at its height over its cycle, plume.rs); kept for the table's shape.
	pub rise: f32,
	/// How ragged the plume's edge gets (0 smooth .. 1 torn; times `smoke_plume_billow`).
	pub billow: f32,
}

/// The row of an alpha-blended world quad drawn with texture key `key` (`additive` quads are never smoke).
pub fn quad_row(rows: &[QuadRow], key: usize, additive: bool) -> Option<&QuadRow> {
	let k = crate::fire_option::plain_key(key);
	rows.iter().find(|r| !additive && r.key == k)
}

/// The row of a slot with effect type `kind` and particle texture `tex`.
pub fn find(rows: &[SmokeRow], kind: u8, tex: u8) -> Option<usize> {
	rows.iter().position(|r| r.kind == kind && (r.tex == ANY_TEX || r.tex == tex))
}

/// The most saturated channel spread (max - min over max) of a 0..1 colour: 0 grey, 1 pure hue.
pub fn saturation(c: [f32; 3]) -> f32 {
	let hi = c[0].max(c[1]).max(c[2]);
	if hi <= 1e-4 {
		return 0.0;
	}
	(hi - c[0].min(c[1]).min(c[2])) / hi
}

/// A grey-only row's saturation limit.
pub const GREY_LIMIT: f32 = 0.35;

#[cfg(test)]
mod tests {
	use super::*;

	const ROWS: [SmokeRow; 2] = [
		SmokeRow { kind: 4, tex: 0, grey_only: true, radius: 0.5, density: 1.0, rise: 1.0, billow: 0.3 },
		SmokeRow { kind: 6, tex: ANY_TEX, grey_only: false, radius: 0.5, density: 1.0, rise: 1.0, billow: 0.3 },
	];

	#[test]
	fn rows_match_by_type_and_texture() {
		assert_eq!(find(&ROWS, 4, 0), Some(0));
		assert_eq!(find(&ROWS, 4, 7), None);
		assert_eq!(find(&ROWS, 6, 33), Some(1));
		assert_eq!(find(&[], 6, 0), None);
	}

	#[test]
	fn quad_rows_match_alpha_quads_by_texture_key_only() {
		const Q: [QuadRow; 2] = [
			QuadRow { key: 46, alpha: 0.3125, radius: 0.5, density: 1.0, rise: 1.0, billow: 0.3 },
			QuadRow { key: 47, alpha: 2.0, radius: 0.5, density: 1.0, rise: 1.0, billow: 0.3 },
		];
		assert_eq!(quad_row(&Q, 46, false).map(|r| r.alpha), Some(0.3125));
		assert_eq!(quad_row(&Q, 47 | crate::prims::FX_CLAMP, false).map(|r| r.alpha), Some(2.0));
		assert!(quad_row(&Q, 46, true).is_none(), "the additive flame quad is not smoke");
		assert!(quad_row(&Q, 48, false).is_none());
		assert!(quad_row(&[], 46, false).is_none());
	}

	#[test]
	fn grey_is_unsaturated_and_orange_is_not() {
		assert!(saturation([0.8, 0.8, 0.8]) < GREY_LIMIT);
		assert!(saturation([1.9, 0.96, 0.21]) > GREY_LIMIT);
		assert_eq!(saturation([0.0; 3]), 0.0);
	}
}
