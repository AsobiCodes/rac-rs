//! [PC] The words on the keyboard keycaps of the modern UI (v0.5 review R1-1). The tab keys are PHYSICAL keys (the two left and right
//! of W: Q / E on QWERTY; the hosts map `KeyCode::KeyQ / KeyE`), so their caps show what the player's own keyboard layout prints
//! there (A / E on a French AZERTY keyboard). The named keys (Enter, Esc, Backspace, Space) speak the language of the screen.
//! Game-neutral: the host refreshes the layout on keyboard input and focus (`refresh`) and names the language of the screen it
//! builds (`set_lang`, per thread: the screens are built on the window thread, each test on its own).
use super::rows::{pick, T5};
use std::cell::Cell;
use std::sync::RwLock;

/// A named key of the hint bar and the dialog buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
	Enter,
	Esc,
	Backspace,
	Space,
}

// Not retail text: the key names as the five languages' keyboards and manuals print them.
const ENTER: T5 = ["Enter", "Entr\u{e9}e", "Enter", "Intro", "Invio"];
const ESC: T5 = ["Esc", "\u{c9}chap", "Esc", "Esc", "Esc"];
const BACKSPACE: T5 = ["Backspace", "Retour arri\u{e8}re", "R\u{fc}cktaste", "Retroceso", "Backspace"];
const SPACE: T5 = ["Space", "Espace", "Leertaste", "Espacio", "Spazio"];

/// Set-1 scan codes of the physical Q and E keys.
const SCAN_Q: u32 = 0x10;
const SCAN_E: u32 = 0x12;

thread_local! {
	static LANG: Cell<usize> = const { Cell::new(0) };
}

/// The caps of the two tab keys in the player's keyboard layout (None = not read yet: Q / E).
static TABS: RwLock<Option<(String, String)>> = RwLock::new(None);

/// The language (0..LANGS) the keycaps of the screens built on this thread speak.
pub fn set_lang(lang: usize) {
	LANG.with(|l| l.set(lang));
}

/// The cap of a named key in this thread's screen language.
pub fn name(c: Cap) -> &'static str {
	name_in(c, LANG.with(Cell::get))
}

/// The cap of a named key in language `lang`.
pub fn name_in(c: Cap, lang: usize) -> &'static str {
	let t: &'static T5 = match c {
		Cap::Enter => &ENTER,
		Cap::Esc => &ESC,
		Cap::Backspace => &BACKSPACE,
		Cap::Space => &SPACE,
	};
	pick(t, lang)
}

/// The caps of the two tab keys (the physical Q and E): what the keyboard layout prints there, Q / E until the host first `refresh`es.
pub fn tab_keys() -> (String, String) {
	TABS.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(|| ("Q".to_string(), "E".to_string()))
}

/// Reads the tab keys' caps from the keyboard layout of the calling thread (the window thread: on keyboard input and focus, so a
/// layout switch shows at the next key).
pub fn refresh() {
	let t = (cap_text(os_char(SCAN_Q)).unwrap_or_else(|| "Q".to_string()), cap_text(os_char(SCAN_E)).unwrap_or_else(|| "E".to_string()));
	let mut g = TABS.write().unwrap_or_else(|e| e.into_inner());
	if g.as_ref() != Some(&t) {
		*g = Some(t);
	}
}

/// The cap of a layout character: upper case; None for nothing, a control or blank, or a letter the UI fonts may lack (outside Latin-1
/// and Latin Extended-A), so the cap falls back to the QWERTY letter.
pub fn cap_text(c: Option<char>) -> Option<String> {
	let c = c?;
	if c.is_control() || c.is_whitespace() || !(c.is_ascii_graphic() || ('\u{a1}'..='\u{17f}').contains(&c)) {
		return None;
	}
	Some(c.to_uppercase().collect())
}

/// The character the key at scan code `scan` types without modifiers in the calling thread's keyboard layout.
#[cfg(windows)]
fn os_char(scan: u32) -> Option<char> {
	use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VK_TO_CHAR, MAPVK_VSC_TO_VK};
	// SAFETY: two table lookups in the thread's keyboard layout; no pointers, no state changed.
	let vk = unsafe { MapVirtualKeyW(scan, MAPVK_VSC_TO_VK) };
	if (0x41..=0x5a).contains(&vk) {
		return char::from_u32(vk); // a letter key: its virtual key is the letter
	}
	if vk == 0 {
		return None;
	}
	// SAFETY: as above. Bit 31 marks a dead key (an accent waiting for its letter); the low word is the character either way.
	let ch = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_CHAR) } & 0xffff;
	char::from_u32(ch).filter(|c| *c != '\0')
}

#[cfg(not(windows))]
fn os_char(_scan: u32) -> Option<char> {
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn named_caps_speak_the_threads_language() {
		assert_eq!(name(Cap::Enter), "Enter", "a fresh thread speaks English");
		set_lang(1);
		assert_eq!((name(Cap::Enter), name(Cap::Esc)), ("Entr\u{e9}e", "\u{c9}chap"));
		set_lang(2);
		assert_eq!(name(Cap::Space), "Leertaste");
		set_lang(9);
		assert_eq!(name(Cap::Backspace), "Backspace", "an unknown language is English");
		set_lang(0);
		for lang in 0..5 {
			for c in [Cap::Enter, Cap::Esc, Cap::Backspace, Cap::Space] {
				assert!(!name_in(c, lang).is_empty());
			}
		}
	}

	#[test]
	fn layout_characters_become_caps_or_fall_back() {
		assert_eq!(cap_text(Some('a')).as_deref(), Some("A"), "AZERTY: the physical Q prints A");
		assert_eq!(cap_text(Some('\'')).as_deref(), Some("'"), "Dvorak: the physical Q prints '");
		assert_eq!(cap_text(Some('\u{e9}')).as_deref(), Some("\u{c9}"));
		assert_eq!(cap_text(Some('\u{439}')), None, "a letter the UI fonts may lack falls back");
		assert_eq!(cap_text(Some(' ')), None);
		assert_eq!(cap_text(Some('\u{8}')), None);
		assert_eq!(cap_text(None), None);
	}
}
