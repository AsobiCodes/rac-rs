//! The Display and Game rows of the pause Options list (v0.5, Modern menu style): "also we need to put game and display in this
//! menu" (user 2026-11-04). The Modern Options screen has four tabs (Display, Graphics, Camera, Game, `modern_menu/tabs.rs`) and the
//! pause list reached two of them (Graphics, Camera); these rows reach the other two. Order: ... Subtitles, Display, Graphics, Game,
//! Controls, Mods, Achievements, Quit to Desktop, Quit Game.
//!
//! The rows exist only when the menu style is Modern at the moment the list is built (`patch_graphics`, once per planet load, like
//! the Mods row); with Classic the list, its descriptions and its pictures are exactly what they were. Their records are type-3
//! "go to the page at arg" rows pointing at the Graphics page (`GPAGE`), so a player who switches to Classic mid-planet and presses
//! Cross on one lands on the Classic Graphics page (which holds the Display settings in Classic) instead of nothing. In Modern the
//! press never reaches the game: `modern_swallow::decide` opens the modern screen at the tab. Only the pause list gets them; the
//! title screen's Options list is built elsewhere (`title_options`) and is not touched.
//!
//! Texts: four pool slots (`texts.rs`), the labels use the tab names of `tabs.rs`.

/// Text ids: the two row labels and their descriptions (free ids between Quit to Desktop's 0x7f33 and the Controls block 0x7f40).
pub const ID_DISPLAY_TAB: u32 = 0x7f34;
pub const ID_GAME_TAB: u32 = 0x7f35;
pub const ID_DISPLAY_TAB_DESC: u32 = 0x7f36;
pub const ID_GAME_TAB_DESC: u32 = 0x7f37;

/// Whether the list being built gets the rows: the menu style now. Tests build the Classic list unless `force` says otherwise
/// (the sync-based tests read the retail-shaped list; the Modern one is tested through `add` and `FORCE_MODERN`).
pub(super) fn modern_at_build() -> bool {
	#[cfg(test)]
	return FORCE_MODERN.load(std::sync::atomic::Ordering::Relaxed);
	#[cfg(not(test))]
	crate::modern_menu::style::modern()
}

/// Tests only: build the Modern list (set and reset inside the tests' serial lock).
#[cfg(test)]
pub(super) static FORCE_MODERN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `patch_graphics`: with the Modern style, inserts Display at `at` (just before the Graphics row) and Game right after Graphics,
/// in the pictures (the planet picture, like Graphics), the records (to the Graphics page) and the descriptions. Classic: nothing.
pub(super) fn add(at: usize, pics: &mut Vec<[u32; 2]>, planet: [u32; 2], recs: &mut Vec<[u32; 3]>, desc: &mut Vec<u32>) {
	if !modern_at_build() {
		return;
	}
	insert(pics, at, planet, planet);
	insert(recs, at, [ID_DISPLAY_TAB | 3 << 16, super::GPAGE, 0], [ID_GAME_TAB | 3 << 16, super::GPAGE, 0]);
	insert(desc, at, ID_DISPLAY_TAB_DESC, ID_GAME_TAB_DESC);
}

/// `display` goes at `at`, `game` right after the entry that was at `at` (Graphics).
fn insert<T>(v: &mut Vec<T>, at: usize, display: T, game: T) {
	v.insert(at + 1, game);
	v.insert(at, display);
}

/// The texts (En, Fr, De, Es, It), in `NEW_IDS` order: the labels are the tab names; the descriptions say what the tab holds
/// (`modern_menu/tabs.rs` DISPLAY_SECS: Screen, HUD, Renderer; GAME_SECS: Gameplay, Vendor, Start, Menus). Game font bytes: e-acute
/// 0x8c, a-umlaut 0x87, u-umlaut 0xa7, a-acute 0x84. [I] wording, native review wanted.
pub(super) const TEXTS: [[&[u8]; 5]; 4] = [
	[b"Display", b"Affichage", b"Anzeige", b"Pantalla", b"Schermo"],
	[b"Game", b"Jeu", b"Spiel", b"Juego", b"Gioco"],
	[b"Adjust screen, HUD and renderer", b"R\x8cgler \x8ccran, ATH et rendu", b"Bildschirm, HUD, Renderer \x87ndern", b"Ajusta pantalla, HUD y motor gr\x84fico", b"Regola schermo, HUD e renderer"],
	[b"Adjust gameplay, vendor and menus", b"R\x8cgler jeu, vendeur et menus", b"Spielablauf, H\x87ndler, Men\xa7s", b"Ajusta el juego y el vendedor", b"Regola gioco, venditore e menu"],
];

#[cfg(test)]
mod tests {
	use super::super::{NEW_IDS, OPT_DESC, OPT_RECS, TEXTS as ALL, GPAGE};
	use super::*;

	fn lists() -> (Vec<[u32; 2]>, Vec<[u32; 3]>, Vec<u32>) {
		// Graphics (index 6), Controls, Quit Game, as `patch_graphics` has them before this step (the zero record last).
		let g = [0x7f06 | 3 << 16, GPAGE, 0];
		(vec![[1, 1], [2, 2], [3, 3]], vec![[1, 0, 0], g, [2, 0, 0], [0; 3]], vec![0x4f31, 0x7f08, 0x4f39, 0])
	}

	#[test]
	fn classic_leaves_every_list_byte_identical() {
		let _t = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		FORCE_MODERN.store(false, std::sync::atomic::Ordering::Relaxed);
		let (mut p, mut r, mut d) = lists();
		let before = (p.clone(), r.clone(), d.clone());
		add(1, &mut p, [9, 9], &mut r, &mut d);
		assert_eq!((p, r, d), before);
	}

	#[test]
	fn modern_puts_display_before_graphics_and_game_after_it() {
		let _t = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		FORCE_MODERN.store(true, std::sync::atomic::Ordering::Relaxed);
		let (mut p, mut r, mut d) = lists();
		add(1, &mut p, [9, 9], &mut r, &mut d);
		FORCE_MODERN.store(false, std::sync::atomic::Ordering::Relaxed);
		assert_eq!(p, [[1, 1], [9, 9], [2, 2], [9, 9], [3, 3]], "Display and Game wear the planet picture");
		let ids: Vec<u32> = r.iter().map(|x| x[0]).collect();
		assert_eq!(ids, [1, ID_DISPLAY_TAB | 3 << 16, 0x7f06 | 3 << 16, ID_GAME_TAB | 3 << 16, 2, 0]);
		assert_eq!((r[1][1], r[3][1]), (GPAGE, GPAGE), "both open the Graphics page when the menu is Classic");
		assert_eq!(d, [0x4f31, ID_DISPLAY_TAB_DESC, 0x7f08, ID_GAME_TAB_DESC, 0x4f39, 0]);
	}

	#[test]
	fn the_pool_and_the_tables_have_room_for_14_rows() {
		assert!(OPT_RECS + 12 * 15 <= OPT_DESC && OPT_DESC + 4 * 15 <= GPAGE, "14 rows + the zero record fit before the Graphics page");
	}

	#[test]
	fn texts_are_listed_five_languages_and_short() {
		for (i, id) in [ID_DISPLAY_TAB, ID_GAME_TAB, ID_DISPLAY_TAB_DESC, ID_GAME_TAB_DESC].into_iter().enumerate() {
			let k = NEW_IDS.iter().position(|&n| n == id).expect("listed in NEW_IDS");
			assert_eq!(ALL[k], TEXTS[i], "{id:#x} is the same text in the pool table");
			for (lang, t) in TEXTS[i].iter().enumerate() {
				let limit = if i < 2 { 12 } else { 40 }; // a label next to the cursor, a description under the list
				assert!(!t.is_empty() && t.len() <= limit, "{id:#x} language {lang}: {} bytes", t.len());
			}
		}
		assert_eq!(TEXTS[0][0], b"Display");
		assert_eq!(TEXTS[1][2], b"Spiel");
	}
}
