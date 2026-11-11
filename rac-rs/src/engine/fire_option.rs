//! Shared, live player setting for the realistic fire look (viewer/firefx.rs).
//! 0 Off = today's picture (default), 1 Low, 2 Medium, 3 High. `RAC_FIRE=0..3` wins over the setting (tests, dev).
//! The game pack names which draws are fire as table rows (`Pack::fire_rows`, default none).
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

static LEVEL: AtomicU8 = AtomicU8::new(0);

pub fn set(level: u8) {
	LEVEL.store(level.min(3), Ordering::Relaxed);
}

/// The fire level in force: `RAC_FIRE` when set, else the player's setting.
pub fn level() -> u8 {
	static ENV: OnceLock<Option<u8>> = OnceLock::new();
	match ENV.get_or_init(|| std::env::var("RAC_FIRE").ok().and_then(|v| v.trim().parse::<u8>().ok()).map(|v| v.min(3))) {
		Some(v) => *v,
		None => LEVEL.load(Ordering::Relaxed),
	}
}

pub fn enabled() -> bool {
	level() > 0
}

/// What a fire row makes of a draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireKind {
	/// A pool sprite drawn as an upright flame tongue (the game's flame textures).
	Flame,
	/// A pool sprite drawn as a fireball (explosion textures).
	Burst,
	/// An ADDITIVE world quad (a level fire's main billboard) drawn as a big flame tongue. (The fire's ALPHA-BLENDED grey columns
	/// are no fire row: they stay the game's quads, or become the Smoke option's volumes, `Pack::smoke_quads`.)
	Ground,
	/// A segment of a flame jet (the Pyrocitor's stream): the segments of one stream are joined, in stream order, into ONE continuous
	/// jet (viewer/firefx_jet.rs). One that stands alone (too few neighbours) is drawn as a `Burst` fireball.
	Jet,
	/// A small burst that belongs to a jet (the stream's sub-bursts): a small fireball, but only while a jet is close by (the same
	/// effect type is also used by things that are not fire, which stay retail).
	JetBurst,
	/// A spark of a jet (the stream's embers): a small glowing spark, only while a jet is close by.
	JetSpark,
	/// A burn puff (an enemy set alight): a small flame while its colour is warm and saturated; the grey puffs of the same type stay
	/// retail (or Smoke's).
	Burn,
	/// An explosion that keeps its retail look but starts a ground fire where it lands (viewer/firefx_blast.rs): never drawn by the
	/// fire code itself, only watched.
	Blast,
}

impl FireKind {
	/// Whether a pool sprite of this kind is drawn by `viewer/firefx.rs` as a pool sprite (all but the level fire's world quad and
	/// the watched-only `Blast`).
	pub fn is_sprite(self) -> bool {
		self != FireKind::Ground && self != FireKind::Blast
	}
}

/// One fire row of a level. `key` of a `Ground` row is the renderer's texture key (the fx table index of a world quad). `key` of a `Flame`
/// or `Burst` row is the pool sprite's EFFECT TYPE (the slot's byte +0, `PoolFx::sprite_types`, as Smoke's rows): one type is one particle
/// def, so all frames of a flipbook are fire together or none is (an image can be shared with an explosion's frames, a type cannot).
#[derive(Clone, Copy, Debug)]
pub struct FireRow {
	pub key: u32,
	pub kind: FireKind,
	/// Drawn as its kind, but never weighed for the fire light (lane rfire review: the fire image's flying embers, which pass the
	/// colour and size gates at their largest; viewer/firefx.rs `add_sprite`).
	pub dark: bool,
}

/// The texture key without the clamp / no-mip variant bits.
pub fn plain_key(key: usize) -> u32 {
	(key & !(crate::prims::FX_CLAMP | crate::prims::FX_NOMIP | crate::prims::FX_REFILL)) as u32
}

/// The kind of a world quad of texture `key` (`additive` = the GS blend): `Ground` for a fire's additive flame billboard, None = not fire.
pub fn quad_kind(rows: &[FireRow], key: usize, additive: bool) -> Option<FireKind> {
	let k = plain_key(key);
	rows.iter().find(|r| additive && r.key == k && r.kind == FireKind::Ground).map(|r| r.kind)
}

/// The kind of a pool sprite of effect type `ty` (the slot's byte +0): any kind but `Ground`, None = not fire.
pub fn sprite_kind(rows: &[FireRow], ty: u8) -> Option<FireKind> {
	rows.iter().find(|r| r.key == ty as u32 && r.kind.is_sprite()).map(|r| r.kind)
}

/// Whether a pool sprite of effect type `ty` is drawn as fire but lights nothing (a `dark` sprite row).
pub fn sprite_dark(rows: &[FireRow], ty: u8) -> bool {
	rows.iter().any(|r| r.key == ty as u32 && r.kind.is_sprite() && r.dark)
}

/// Whether a pool sprite of effect type `ty` is an explosion that starts a ground fire: a `Blast` row; with `bursts` (the dev knob
/// `fire_blast55`) the `Burst` rows too (their type is not verified as an explosion, so they stay off by default).
pub fn blast_type(rows: &[FireRow], ty: u8, bursts: bool) -> bool {
	rows.iter().any(|r| r.key == ty as u32 && (r.kind == FireKind::Blast || (bursts && r.kind == FireKind::Burst)))
}

/// Whether a colour (0..1 channels) is a flame's warm or green, saturated colour: the burn puffs of a type that is grey smoke elsewhere.
pub fn is_hot(c: [f32; 3]) -> bool {
	crate::smoke_rows::saturation(c) > crate::smoke_rows::GREY_LIMIT && c[0].max(c[1]).max(c[2]) > 0.25
}

#[cfg(test)]
mod tests {
	use super::*;

	const ROWS: [FireRow; 3] = [
		FireRow { key: 46, kind: FireKind::Ground, dark: false },
		FireRow { key: 19, kind: FireKind::Flame, dark: false },
		FireRow { key: 12, kind: FireKind::Burst, dark: false },
	];

	#[test]
	fn quads_are_additive_only_and_sprites_by_effect_type() {
		assert_eq!(quad_kind(&ROWS, 46, true), Some(FireKind::Ground));
		assert_eq!(quad_kind(&ROWS, 46, false), None, "the alpha column is not the fire's");
		assert_eq!(quad_kind(&ROWS, 46 | crate::prims::FX_CLAMP, true), Some(FireKind::Ground));
		assert_eq!(quad_kind(&ROWS, 47, true), None);
		assert_eq!(sprite_kind(&ROWS, 19), Some(FireKind::Flame));
		assert_eq!(sprite_kind(&ROWS, 12), Some(FireKind::Burst));
		assert_eq!(sprite_kind(&ROWS, 5), None, "another type stays retail");
		// A world-quad row never makes a sprite (type 46 is not a Ground row's key) and the other way round.
		assert_eq!(sprite_kind(&ROWS, 46), None);
		assert_eq!(quad_kind(&ROWS, 19, true), None);
		assert_eq!(quad_kind(&[], 46, true), None);
	}

	/// The Pyrocitor's kinds are pool-sprite kinds like Flame and Burst; only the level fire's world quad is not.
	#[test]
	fn the_jet_kinds_are_sprite_kinds() {
		let rows = [
			FireRow { key: 12, kind: FireKind::Jet, dark: false },
			FireRow { key: 2, kind: FireKind::JetBurst, dark: false },
			FireRow { key: 25, kind: FireKind::JetSpark, dark: false },
			FireRow { key: 4, kind: FireKind::Burn, dark: false },
			FireRow { key: 46, kind: FireKind::Ground, dark: false },
		];
		for (ty, kind) in [(12, FireKind::Jet), (2, FireKind::JetBurst), (25, FireKind::JetSpark), (4, FireKind::Burn)] {
			assert_eq!(sprite_kind(&rows, ty), Some(kind));
			assert!(kind.is_sprite());
			assert_eq!(quad_kind(&rows, ty as usize, true), None, "no world quad of a sprite row");
		}
		assert!(!FireKind::Ground.is_sprite() && FireKind::Flame.is_sprite() && FireKind::Burst.is_sprite());
		assert_eq!(sprite_kind(&rows, 46), None);
	}

	/// A `Blast` row is only watched: never a sprite kind, never a world quad, found by `blast_type`; a `Burst` row only with the knob.
	#[test]
	fn blast_rows_are_watched_not_drawn() {
		let rows = [
			FireRow { key: 5, kind: FireKind::Blast, dark: false },
			FireRow { key: 55, kind: FireKind::Burst, dark: false },
			FireRow { key: 12, kind: FireKind::Jet, dark: false },
			FireRow { key: 46, kind: FireKind::Ground, dark: false },
		];
		assert!(!FireKind::Blast.is_sprite());
		assert_eq!(sprite_kind(&rows, 5), None, "a blast keeps its retail draw");
		assert_eq!(quad_kind(&rows, 5, true), None);
		assert!(blast_type(&rows, 5, false) && blast_type(&rows, 5, true));
		assert!(!blast_type(&rows, 55, false), "a burst row is no blast by default");
		assert!(blast_type(&rows, 55, true), "the dev knob takes it");
		for ty in [12u8, 2, 25, 4, 46, 0] {
			assert!(!blast_type(&rows, ty, true), "type {ty} is no blast");
		}
		assert!(!blast_type(&[], 5, true));
	}

	/// Burn puffs (type 4) are fire only while their colour is warm or green and bright; the grey and the dark ones are smoke.
	#[test]
	fn hot_colours_are_orange_and_green_not_grey() {
		assert!(is_hot([1.0, 0.63, 0.0]), "0x7000a0ff: orange");
		assert!(is_hot([0.63, 1.0, 0.0]), "0x7000ffa0: green (the upgraded gun)");
		assert!(is_hot([1.9, 0.96, 0.21]), "an HDR flame colour");
		assert!(!is_hot([0.5, 0.5, 0.5]) && !is_hot([0.8, 0.78, 0.8]), "grey smoke");
		assert!(!is_hot([0.1, 0.04, 0.0]), "too dark to be a flame");
		assert!(!is_hot([0.0; 3]));
	}

	#[test]
	fn level_is_capped_and_off_by_default() {
		set(9);
		assert!(level() <= 3);
		set(0);
		// (RAC_FIRE is not set in the unit tests: test.ps1 clears every RAC_* variable.)
		assert_eq!(level(), 0);
		assert!(!enabled());
	}
}
