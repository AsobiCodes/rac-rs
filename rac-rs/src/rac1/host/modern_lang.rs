//! The language of the modern screens (v0.5 screens stage F1): the pause menu, the shop, the Galactic Map and the text bank they read
//! (`App::ui_text`) speak the GAME's own language (the language word 0x15ed88 that the Options > Language page sets, read into every
//! snapshot as `Snap::lang`, `Retail::game_lang`), not the PC settings' `lang`. Without a running retail game the settings' language is used.
//! The title needs nothing here: `front_mirror_frame` already follows the same word and reloads its own text bank.
use super::*;

/// The text index (0..=4: English, French, German, Spanish, Italian) to speak: the game's, else the settings' (cut to 0..=4).
fn pick(game: Option<usize>, settings: u8) -> usize {
	game.unwrap_or(settings as usize).min(4)
}

/// `ui_text` has to be read again: it was never loaded, or it holds another language than the one wanted.
fn stale(loaded: Option<usize>, want: usize) -> bool {
	loaded != Some(want)
}

/// The text index the in-level text bank should hold now (the game's language when a retail game is up, else the settings'). A free
/// function over the two fields it reads, so the level load (which holds `&mut self.gpu`) can call it.
pub(super) fn text_lang(retail: Option<&crate::retail::Retail>, settings_lang: u8) -> usize {
	pick(retail.and_then(|r| r.game_lang()), settings_lang)
}

/// Reads the in-level text bank of one language from the disc (`lang` is the text index, `Text::load` maps it to the all_text slot
/// itself). None = the read failed (printed).
pub(super) fn read_text(iso_path: &str, lang: usize) -> Option<frontend::Text> {
	iso::Disc::open(std::path::Path::new(iso_path)).map_err(|e| e.to_string()).and_then(|mut d| frontend::Text::load(&mut d, lang.min(4))).map_err(|e| eprintln!("text {lang}: {e}")).ok()
}

impl App {
	/// The language the modern screens speak this frame; reloads `ui_text` when the game's language is not the one it was loaded with.
	/// A failed read keeps the old bank and is not retried until the language changes.
	pub(super) fn modern_lang(&mut self) -> usize {
		let lang = text_lang(self.retail.as_ref(), self.settings.lang);
		if stale(self.ui_text_lang, lang) {
			self.ui_text = read_text(&self.iso_path, lang).or(self.ui_text.take());
			self.ui_text_lang = Some(lang);
		}
		rac_engine::ui::keycaps::set_lang(lang); // the named keycaps (Enter, Esc) speak the screen's language
		lang
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_games_language_wins_over_the_settings() {
		assert_eq!(pick(Some(2), 0), 2, "a German game with English settings speaks German");
		assert_eq!(pick(None, 3), 3, "no game yet: the settings");
		assert_eq!(pick(None, 9), 4, "an out-of-range setting is cut");
		assert_eq!(pick(Some(7), 0), 4);
	}

	#[test]
	fn the_bank_is_read_again_only_when_the_language_differs() {
		assert!(stale(None, 0), "never loaded");
		assert!(stale(Some(0), 2), "English bank, German game");
		assert!(!stale(Some(2), 2));
	}

	#[test]
	fn retail_language_words_pick_the_text_index() {
		use crate::retail::menu_model::text::lang_index;
		// En 0, Fr 2, De 3, Es 4, It 5 (the Options > Language rows); the German savestates hold 3.
		assert_eq!([0u32, 2, 3, 4, 5].map(|w| pick(Some(lang_index(w)), 0)), [0, 1, 2, 3, 4]);
	}
}
