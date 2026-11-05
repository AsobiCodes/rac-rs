//! [PC] The 36 achievement icons of R&C1's trophy list (user 2026-11-02): one 128 x 128 8-bit RGB PNG per trophy (`DEFS` order, the
//! file name is the trophy id) plus `hidden` for a locked hidden trophy. Source: 1024 x 1024 art downsized by
//! `scratch_perf_ach\icons\resize.py` into `rac-rs/assets/achievements/` (1.2 MB in all, embedded in the exe). The engine turns them
//! into one atlas (`engine/achievements/icons.rs`); the launcher bundles its own copy of the same files.
use crate::achievements::IconSet;

/// Edge of every PNG in pixels (the toast draws the badge 64 px at 1080 lines, 128 px at 4K; the resize script's `--size`).
pub const ICON_SIZE: u32 = 128;

macro_rules! icons {
	($($id:literal),* $(,)?) => {
		/// The trophy ids in `DEFS` order (a test holds them together).
		pub const IDS: [&str; 35] = [$($id),*];
		static PNG: [&[u8]; 35] = [$(include_bytes!(concat!("../../assets/achievements/", $id, ".png")) as &[u8]),*];
	};
}

icons!(
	"hit_me_more", "lombax_magnetic", "not_the_droids", "very_sucky", "all_qwarkd_up", "bolt_collector", "gadgeteer", "nano_this",
	"ultimate_animal", "unlock_sketchbook", "bolt_commando", "drek_this", "king_midas", "skillful", "speedy", "gadget_master", "take_aim",
	"transported", "destroyed", "sniper", "buried_treasure", "magician", "pest_control", "hey_over_here", "sitting_ducks",
	"careful_cruise", "strike_a_pose", "going_commando", "vehicle_bombardier", "cluck_cluck", "qwarktastic", "girl_trouble",
	"twisted_hoverboard", "vandal", "eat_lead",
);

/// Shown for a hidden trophy until it is unlocked.
static HIDDEN: &[u8] = include_bytes!("../../assets/achievements/hidden.png");

pub static SET: IconSet = IconSet { size: ICON_SIZE, png: &PNG, hidden: HIDDEN };

#[cfg(test)]
mod tests {
	use super::*;
	use crate::trophies::DEFS;
	use rac_engine::achievements::icons;

	#[test]
	fn every_trophy_has_its_icon_in_defs_order() {
		assert_eq!(IDS.len(), DEFS.len());
		for (d, id) in DEFS.iter().zip(IDS) {
			assert_eq!(d.id, id, "icon order follows DEFS");
		}
		assert_eq!(SET.png.len(), DEFS.len());
	}

	#[test]
	fn every_icon_decodes_to_the_chosen_size() {
		for (id, png) in IDS.iter().zip(SET.png) {
			let px = icons::decode(png, ICON_SIZE).unwrap_or_else(|e| panic!("{id}: {e}"));
			assert_eq!(px.len(), (ICON_SIZE * ICON_SIZE) as usize);
			assert!(px.iter().any(|p| p[0] != p[1] || p[1] != p[2]), "{id} is not a grey picture");
		}
		assert!(icons::decode(SET.hidden, ICON_SIZE).is_ok(), "hidden icon");
	}

	#[test]
	fn the_whole_set_stays_small() {
		let bytes: usize = SET.png.iter().map(|p| p.len()).sum::<usize>() + SET.hidden.len();
		assert!(bytes < 1_500_000, "icons are {bytes} bytes");
	}

	#[test]
	fn the_pack_builds_its_atlas_and_a_hidden_trophy_shows_the_hidden_icon() {
		let t = icons::build(&SET, DEFS.len()).unwrap();
		let l = icons::Layout::new(DEFS.len(), ICON_SIZE);
		assert_eq!((t.w, t.h), l.dims());
		let hidden = DEFS.iter().position(|d| d.hidden).unwrap();
		assert_eq!(l.tile_for(hidden, false, true), l.hidden());
		assert_eq!(l.tile_for(hidden, true, true), l.colour(hidden));
	}
}
