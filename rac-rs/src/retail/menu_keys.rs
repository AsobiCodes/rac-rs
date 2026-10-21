//! Modern menu controls for keyboard and mouse in every retail page menu (pause menu and its pages, Quick Select, the maps,
//! the title front end), plus the per-screen table the button prompts name keys from (decomp/notes/button_prompts.md
//! "Modern menu controls").
//!
//! In a menu the keyboard no longer works as a DualShock: arrows (and WASD) move the selection, Enter / Space (and a left
//! click on no item, or anywhere in a dialog / the vendor / the front end's message box) confirm,
//! Esc / Backspace go back or answer No, Q / E are L1 / R1 (the ring, the planet selector), Tab is a screen's extra action
//! (the map's Play Infobot). What "confirm" and "back" press depends on the screen: the Quit Game page answers Yes on
//! Circle, so there Enter presses Circle; the pause root closes on Start, so there Esc presses Start (`screen`). The old
//! letter bindings (T, G, F, C, Ctrl ...) keep their pad bits, so nothing a player learnt stops working. Nothing is written
//! to the game: the result is pad bits only, exactly like a hand on the pad. Outside menus `retail::pad_from_keys` is
//! unchanged; a key whose meaning changes when a menu opens or closes and that is held across the switch counts for nothing
//! until it is released (`Layer`), so the Esc that opened the pause menu does not also go back in it.
use super::mouse_menu::{Kind, MenuSnap, CIRCLE, CROSS, DOWN, L1, LEFT, R1, RIGHT, TRIANGLE, UP};
use std::sync::atomic::{AtomicU64, Ordering};
use winit::keyboard::KeyCode as K;

pub const START: u16 = 0x800;
#[cfg(test)]
pub const SQUARE: u16 = 0x80;

/// Page ids (+0x3c of the page struct) of the in-level page menu (decomp/notes/ingame_pause_menu.md) [C].
pub const ROOT: u32 = 2;
pub const QUIT_GAME: u32 = 0xd;
pub const MAP: u32 = 0xb;
#[cfg(test)]
pub const QUICK_SELECT: u32 = 5;
pub const MISSIONS: u32 = 0xc;
pub const GALAXY: u32 = 0xf;
pub const GALAXY_CHILD: u32 = 0x10;
/// In-level Overwrite? page (save over a used slot): Circle = Yes, Triangle = No, no Yes / No text drawn [C, handoff menu1].
pub const OVERWRITE: u32 = 0x1a;

/// What the keys mean on one screen: the pad button confirm (Enter / Space / left click) presses, the one back (Esc /
/// Backspace / right click) presses, the extra action of Tab (0 = none), and whether WASD stays the left stick (the maps
/// track with it; arrows still steer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Screen {
	pub id: u32,
	pub confirm: u16,
	pub back: u16,
	pub alt: u16,
	pub sticks: bool,
}

/// Retail's default for a page of the page-menu engine: Cross selects, Triangle goes to the parent page (button/list update
/// 0x2781d0) [C].
pub const DEFAULT: Screen = Screen { id: 0, confirm: CROSS, back: TRIANGLE, alt: 0, sticks: false };

/// The per-screen table of the in-level pages whose buttons differ from `DEFAULT` [C unless noted]:
/// (page id, confirm, back, alt, sticks).
/// * Root 2: no legend; Start closes the pause menu (Triangle has no parent page to go to).
/// * Quit Game 0xd (item update 0x27bda0): Circle = '\x11 Yes' 0x524f quits to the title, Triangle = '\x12 No' 0x524b, Cross does
///   nothing. Its two answers are drawn by the item, not as hint items, so the table has to name them.
/// * Map 0xb / mission list 0xc: Cross '\x10 View Missions' 0x4eee, Circle '\x11 Play Infobot' 0x4eef, Triangle '\x12 Exit',
///   L1 / R1 other planets ('\x14/\x15' 0x4f55), right stick zooms ('\x18' 0x4f56), left stick tracks ('\x19' 0x4f57).
/// * Galactic map 0xf / 0x10 (ship travel): Cross travels, Triangle cancels, L1 / R1 planets [I: same engine and legend ids].
/// * Overwrite 0x1a: Circle = Yes, Triangle = No (like Quit Game).
pub const TABLE: [(u32, u16, u16, u16, bool); 7] = [
	(ROOT, CROSS, START, 0, false),
	(QUIT_GAME, CIRCLE, TRIANGLE, 0, false),
	(MAP, CROSS, TRIANGLE, CIRCLE, true),
	(MISSIONS, CROSS, TRIANGLE, CIRCLE, true),
	(GALAXY, CROSS, TRIANGLE, 0, true),
	(GALAXY_CHILD, CROSS, TRIANGLE, 0, true),
	(OVERWRITE, CIRCLE, TRIANGLE, 0, false),
];

/// The Gadgetron vendor's shop screen (not a page menu: `retail::Snap::shop` is Some while it is up). Cross buys
/// (' Buy'): Enter / Space; Triangle leaves (' Exit'): Esc / Backspace; D-pad left / right browse: arrows, A / D
/// [C: decomp/notes/vendor.md "Browse", "Purchase"]. Square (buy all ammo, vendor.rs PC extra) stays on F, a one-letter
/// name: the 89 px Buy plate wraps "Tab Remplir" / "Tab Ricarica" (font 2, measured 2026-10-13), so no Tab here. The legend
/// names on this screen are the short ones ("Ent", `prompts::substitute_text`). Its id is no page id.
pub const VENDOR: Screen = Screen { id: 0xfe, confirm: CROSS, back: TRIANGLE, alt: 0, sticks: false };

/// The mode-4 dialogs (game mode 0x15f5a4 == 4, opened by the dialog function, Veldin FUN_002012e0 / Eudora FUN_001f4840, which
/// stores its code in a per-level word, `DIALOG_CODE`). Not page menus; their keys from the update (Veldin FUN_002025e8,
/// pad edges 0x13cae4) [C]:
/// * 0 'Quit Race?' 0x4f6e: Cross ' Quit' 0x5248 quits, Circle / Start ' Continue' 0x5249 resumes.
/// * 1 / 4 'Quit?' 0x5229: Triangle ' Exit' 0x4ee0 leaves, Cross ' Continue' 0x524a resumes.
/// * 2 Continue only (Cross); 5 the save-icon help (Cross after a delay): Enter and Esc both press Cross.
/// * 3 memory-card status, 6 keep video mode: Cross / Triangle (`DEFAULT`).
/// Ids 0xf0 + code (no page id is that high).
pub fn dialog(code: u32) -> Screen {
	let (confirm, back) = match code {
		0 => (CROSS, CIRCLE),
		1 | 4 => (TRIANGLE, CROSS),
		2 | 5 => (CROSS, CROSS),
		_ => (CROSS, TRIANGLE),
	};
	Screen { id: 0xf0 + (code & 0xf), confirm, back, alt: 0, sticks: false }
}

/// A mode-4 dialog's screen (`dialog`).
pub fn is_dialog(s: &Screen) -> bool {
	(0xf0..0xf8).contains(&s.id)
}

/// Per level (0 Veldin .. 18 Veldin finale): the word the dialog function writes its code to right after mode = 4
/// (`DAT_0015f5a4 = 4; DAT_x = param_1;` in each overlay's dialog function, decomp/overlays/N_level/overlay.elf.c) [C].
pub const DIALOG_CODE: [u32; 19] = [
	0x173040, 0x173440, 0x1735c0, 0x1730c0, 0x173140, 0x173440, 0x173740, 0x173040, 0x1737c0, 0x173140,
	0x173440, 0x1739c0, 0x173440, 0x1732c0, 0x173740, 0x1735c0, 0x1733c0, 0x1738c0, 0x1739c0,
];
/// `DIALOG_CODE` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
pub const DIALOG_CODE_USA: [u32; 19] = [
	0x173040, 0x1734c0, 0x173640, 0x173140, 0x1731c0, 0x173440, 0x1737c0, 0x1730c0, 0x173840, 0x1731c0,
	0x1734c0, 0x173a40, 0x1734c0, 0x173340, 0x173740, 0x173640, 0x173440, 0x173940, 0x173a40,
];

/// The dialog on screen in game mode `mode` of `level` (`rd` reads EE RAM), None outside mode 4.
pub fn dialog_on(rd: &dyn Fn(u32) -> u32, level: u32, mode: u32) -> Option<Screen> {
	(mode == 4).then(|| (*crate::retail::disc::pick(&DIALOG_CODE, &DIALOG_CODE_USA)).get(level as usize)).flatten().map(|&a| dialog(rd(a)))
}

/// A hint line's role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
	Confirm,
	Back,
	Alt,
}

/// The hint and legend texts retail draws and what their button means on that screen [C: the strings, all_text.bin]:
/// (text id, pad button, role).
pub const HINTS: [(u16, u16, Role); 10] = [
	(0x524f, CIRCLE, Role::Confirm),   // '\x11 Yes'
	(0x524b, TRIANGLE, Role::Back),    // '\x12 No'
	(0x4ee0, TRIANGLE, Role::Back),    // '\x12 Exit'
	(0x4edf, CROSS, Role::Confirm),    // '\x10 Toggle'
	(0x4eec, CROSS, Role::Confirm),    // '\x10 Equip'
	(0x4eee, CROSS, Role::Confirm),    // '\x10 View Missions'
	(0x5248, CROSS, Role::Confirm),    // '\x10 Quit' (Quit Race?)
	(0x5249, CIRCLE, Role::Back),      // '\x11 Continue' (Quit Race?)
	(0x524a, CROSS, Role::Back),       // '\x10 Continue' (Quit?)
	(0x4eef, CIRCLE, Role::Alt),       // '\x11 Play Infobot'
];

/// The key table of the menu `m` (in the title front end, whose page ids are its own, only the hints count). The hints are
/// the hint items and any item that lists only hint texts: New Game's overwrite confirm in the front end (page 0x1d4db8,
/// id 0x1a) shows '\x11 Yes' 0x524f / '\x12 No' 0x524b as the two records of one label item [C, front_mirror.rs P_OVERWRITE],
/// so there Enter presses Circle (Yes) as on the in-level Overwrite page.
pub fn screen(m: &MenuSnap) -> Screen {
	let mut s = Screen { id: m.id, ..DEFAULT };
	let hints_only = |i: &&super::mouse_menu::Item| matches!(i.kind, Kind::Hint(_)) || (!i.texts.is_empty() && i.texts.iter().all(|t| HINTS.iter().any(|h| h.0 == *t)));
	for it in m.items.iter().filter(hints_only) {
		for &t in &it.texts {
			if let Some(&(_, b, r)) = HINTS.iter().find(|h| h.0 == t) {
				match r {
					Role::Confirm => s.confirm = b,
					Role::Back => s.back = b,
					Role::Alt => s.alt = b,
				}
			}
		}
	}
	if !m.front {
		if let Some(&(_, c, b, a, st)) = TABLE.iter().find(|r| r.0 == m.id) {
			(s.confirm, s.back, s.alt, s.sticks) = (c, b, a, st);
		}
	}
	s
}

// ---- the screen of the current field, for the prompts (worker thread) ------------------------------------------------

static CURRENT: AtomicU64 = AtomicU64::new(0);

pub fn pack(s: Screen) -> u64 {
	1 << 63 | (s.sticks as u64) << 62 | ((s.id & 0xff) as u64) << 48 | (s.alt as u64) << 32 | (s.back as u64) << 16 | s.confirm as u64
}

fn unpack(v: u64) -> Option<Screen> {
	(v >> 63 != 0).then(|| Screen { id: (v >> 48 & 0xff) as u32, confirm: v as u16, back: (v >> 16) as u16, alt: (v >> 32) as u16, sticks: v >> 62 & 1 != 0 })
}

/// Worker, after each field: the menu on screen (None = no page menu).
pub fn publish(s: Option<Screen>) {
	CURRENT.store(s.map_or(0, pack), Ordering::Relaxed);
}

/// The menu screen of the last field, None outside menus.
pub fn current() -> Option<Screen> {
	unpack(CURRENT.load(Ordering::Relaxed))
}

/// The key a player presses for pad button `bit` on screen `s`; None when the button has no menu key there (the prompt
/// then names the gameplay binding). `short`: the abbreviation of a long name for a tight legend box. Circle with no role on
/// the screen is G (its letter in `pad_from_keys`, which `menu_pad` keeps): the left mouse button, its play name, clicks the
/// menu instead.
pub fn key_for(s: &Screen, bit: u16, short: bool) -> Option<&'static str> {
	Some(if bit == s.confirm {
		if short { "Ent" } else { "Enter" }
	} else if bit == s.back {
		"Esc"
	} else if s.alt != 0 && bit == s.alt {
		"Tab"
	} else if bit == L1 {
		"Q"
	} else if bit == R1 {
		"E"
	} else if bit == CIRCLE {
		"G"
	} else {
		return None;
	})
}

/// The front end's modal message box (front mode 4, memory card and 60 Hz messages; frontend.rs `dialog_keys`) from the hint
/// texts it shows, each naming its button [C: all_text.bin]: '\x10 Yes' 0x524e, '\x11 Yes' 0x524f, '\x12 No' 0x524b,
/// '\x10 Continue' 0x524a, '\x12 Go back?' 0x4fa8. Yes / Continue / Go back is confirm, No is back; a box with one hint has
/// it on both (like `dialog` 2). None when it shows no hint (the keys stay the play keys). Id 0xf7, a `dialog` id (centred
/// lines: long names).
pub fn front_dialog(yes: u32, no: u32, hint: u32) -> Option<Screen> {
	let button = |t: u32| match t {
		0x524e | 0x524a => Some(CROSS),
		0x524f => Some(CIRCLE),
		0x524b | 0x4fa8 => Some(TRIANGLE),
		_ => None,
	};
	let one = button(hint);
	let confirm = button(yes).or(one)?;
	let back = button(no).or(one).unwrap_or(confirm);
	Some(Screen { id: 0xf7, confirm, back, alt: 0, sticks: false })
}

// ---- keys -> pad -----------------------------------------------------------------------------------------------------

/// The keys of the modern scheme (they do not reach `pad_from_keys` in a menu).
const MODERN: [K; 19] = [
	K::Space,
	K::Enter,
	K::NumpadEnter,
	K::Escape,
	K::Backspace,
	K::Tab,
	K::KeyQ,
	K::KeyE,
	K::KeyZ,
	K::KeyX,
	K::ArrowUp,
	K::ArrowDown,
	K::ArrowLeft,
	K::ArrowRight,
	K::KeyW,
	K::KeyA,
	K::KeyS,
	K::KeyD,
	K::Delete,
];

/// Keys whose pad meaning differs between a menu and play.
const SWITCH: [K; 8] = [K::Space, K::Enter, K::NumpadEnter, K::Escape, K::Backspace, K::Tab, K::KeyQ, K::KeyE];

/// The pad a keyboard gives on screen `s`: (buttons, sticks [rx, ry, lx, ly]).
pub fn menu_pad(s: &Screen, held: &dyn Fn(K) -> bool) -> (u16, [u8; 4]) {
	let (mut b, _) = super::pad_from_keys(|k| !MODERN.contains(&k) && held(k), false, false);
	let any = |ks: &[K]| ks.iter().any(|&k| held(k));
	let wasd = !s.sticks;
	for (arrow, letter, bit) in [(K::ArrowUp, K::KeyW, UP), (K::ArrowDown, K::KeyS, DOWN), (K::ArrowLeft, K::KeyA, LEFT), (K::ArrowRight, K::KeyD, RIGHT)] {
		if held(arrow) || (wasd && held(letter)) {
			b |= bit;
		}
	}
	if any(&[K::Enter, K::NumpadEnter, K::Space]) {
		b |= s.confirm;
	}
	if any(&[K::Escape, K::Backspace]) {
		b |= s.back;
	}
	if held(K::KeyQ) {
		b |= L1;
	}
	if held(K::KeyE) {
		b |= R1;
	}
	if s.alt != 0 && held(K::Tab) {
		b |= s.alt;
	}
	let sticks = if s.sticks { super::pad_from_keys(|k| !matches!(k, K::KeyQ | K::KeyE) && held(k), false, false).1 } else { [0x80; 4] };
	(b, sticks)
}

/// Dev log (`RAC_MENU_LOG=1`, off unless set): a line whenever the page or the pad bits of the keyboard change
/// ("menu: field F page P confirm C back B pad X"), the proof of scripted key runs.
pub fn log(field: u32, menu: Option<&MenuSnap>, other: Option<Screen>, pad: u16) {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	static LAST: std::sync::Mutex<(u32, u16)> = std::sync::Mutex::new((u32::MAX, 0));
	if !*ON.get_or_init(|| std::env::var_os("RAC_MENU_LOG").is_some()) {
		return;
	}
	let id = menu.map_or_else(|| other.map_or(u32::MAX, |s| s.id), |m| m.id);
	let Ok(mut l) = LAST.lock() else { return };
	if *l != (id, pad) {
		*l = (id, pad);
		match menu {
			Some(m) => {
				let s = screen(m);
				eprintln!("menu: field {field} page {id:#x} front {} confirm {:#x} back {:#x} pad {pad:#x}", m.front, s.confirm, s.back);
			}
			None => match other {
				Some(s) => eprintln!("menu: field {field} screen {id:#x} confirm {:#x} back {:#x} pad {pad:#x}", s.confirm, s.back),
				None => eprintln!("menu: field {field} no menu pad {pad:#x}"),
			},
		}
	}
}

/// Keys held across a menu opening or closing: they count for nothing until released.
#[derive(Debug, Default)]
pub struct Layer {
	was_menu: bool,
	stale: Vec<K>,
}

impl Layer {
	/// Once per field, before the keys are read.
	pub fn begin(&mut self, menu: bool, held: &dyn Fn(K) -> bool) {
		if menu != self.was_menu {
			self.was_menu = menu;
			self.stale = SWITCH.iter().copied().filter(|&k| held(k)).collect();
		}
		self.stale.retain(|&k| held(k));
	}

	pub fn live(&self, k: K) -> bool {
		!self.stale.contains(&k)
	}
}

#[cfg(test)]
mod tests {
	use super::super::mouse_menu::Item;
	use super::*;

	fn page(id: u32, hints: &[(u16, u16)]) -> MenuSnap {
		let items = hints.iter().enumerate().map(|(i, &(t, b))| Item { addr: 0x100 + i as u32, slot: i, rect: [0, 0, 10, 10], kind: Kind::Hint(b), texts: vec![t] }).collect();
		MenuSnap { page: 0x1b0000, id, state: id, cur: 0, items, front: false }
	}

	fn front(m: MenuSnap) -> MenuSnap {
		MenuSnap { front: true, ..m }
	}

	fn keys(ks: &[K]) -> impl Fn(K) -> bool + '_ {
		move |k| ks.contains(&k)
	}

	#[test]
	fn per_screen_table() {
		// Quit Game: Yes is Circle, No is Triangle; the answers are not hint items.
		let q = screen(&page(QUIT_GAME, &[]));
		assert_eq!((q.confirm, q.back), (CIRCLE, TRIANGLE));
		// Root closes with Start.
		assert_eq!(screen(&page(ROOT, &[])).back, START);
		// Options and the other pages: Cross / Triangle (legend '\x10 ...' / '\x12 Exit').
		let o = screen(&page(8, &[(0x4f05, CROSS), (0x4ee0, TRIANGLE)]));
		assert_eq!((o.confirm, o.back, o.alt, o.sticks), (CROSS, TRIANGLE, 0, false));
		// A front-end dialog with '\x11 Yes' / '\x12 No' hints: Enter = Circle; front ids never hit the in-level table.
		let f = screen(&front(page(QUIT_GAME, &[(0x524b, TRIANGLE), (0x524f, CIRCLE)])));
		assert_eq!((f.confirm, f.back), (CIRCLE, TRIANGLE));
		assert_eq!(screen(&front(page(ROOT, &[]))).back, TRIANGLE);
		// Quit Race?: '\x10 Quit' confirms, '\x11 Continue' backs out.
		let r = screen(&front(page(0x30, &[(0x5249, CIRCLE), (0x5248, CROSS)])));
		assert_eq!((r.confirm, r.back), (CROSS, CIRCLE));
		// The map: Tab plays the Infobot, WASD tracks.
		let m = screen(&page(MAP, &[]));
		assert_eq!((m.alt, m.sticks), (CIRCLE, true));
		// Overwrite: Enter = Yes (Circle), Esc = No (Triangle).
		let o = screen(&page(OVERWRITE, &[]));
		assert_eq!((o.confirm, o.back), (CIRCLE, TRIANGLE));
		// The front end's overwrite confirm: one label item lists '\x11 Yes' / '\x12 No' (no hint items): Enter = Circle.
		let mut fo = front(page(OVERWRITE, &[]));
		fo.items.push(Item { addr: 0x200, slot: 3, rect: [0, 0, 10, 10], kind: Kind::Label { rows: vec![(5.0, false), (10.0, false)], rowh: 5.0, cursor: 0 }, texts: vec![0x524f, 0x524b] });
		assert_eq!((screen(&fo).confirm, screen(&fo).back), (CIRCLE, TRIANGLE));
		// A label that also lists other texts is no legend.
		fo.items[0].texts.push(0x4f27);
		assert_eq!(screen(&fo).confirm, CROSS);
		// The front end's message box: its hints name the keys.
		assert_eq!(front_dialog(0x524f, 0x524b, 0).map(|s| (s.confirm, s.back)), Some((CIRCLE, TRIANGLE)));
		assert_eq!(front_dialog(0x524e, 0x524b, 0).map(|s| (s.confirm, s.back)), Some((CROSS, TRIANGLE)));
		assert_eq!(front_dialog(0, 0, 0x524a).map(|s| (s.confirm, s.back)), Some((CROSS, CROSS)));
		assert_eq!(front_dialog(0, 0, 0x4fa8).map(|s| (s.confirm, s.back)), Some((TRIANGLE, TRIANGLE)));
		assert_eq!(front_dialog(0, 0, 0), None);
		assert!(is_dialog(&front_dialog(0, 0, 0x524a).unwrap()));
		assert_eq!(menu_pad(&front_dialog(0x524f, 0x524b, 0).unwrap(), &keys(&[K::Enter])).0, CIRCLE);
		// The vendor: Enter buys, Esc leaves, F (its play key) buys all ammo, arrows browse.
		assert_eq!(menu_pad(&VENDOR, &keys(&[K::Enter])).0, CROSS);
		assert_eq!(menu_pad(&VENDOR, &keys(&[K::Escape])).0, TRIANGLE);
		assert_eq!(menu_pad(&VENDOR, &keys(&[K::KeyF])).0, SQUARE);
		assert_eq!(menu_pad(&VENDOR, &keys(&[K::ArrowRight])).0, RIGHT);
		assert_eq!(key_for(&VENDOR, SQUARE, false), None);
	}

	#[test]
	fn modern_keys_press_the_screens_buttons() {
		let q = screen(&page(QUIT_GAME, &[]));
		assert_eq!(menu_pad(&q, &keys(&[K::Enter])).0, CIRCLE);
		assert_eq!(menu_pad(&q, &keys(&[K::Space])).0, CIRCLE);
		assert_eq!(menu_pad(&q, &keys(&[K::Escape])).0, TRIANGLE);
		assert_eq!(menu_pad(&q, &keys(&[K::Backspace])).0, TRIANGLE);
		// Old letters keep their bits (G = Circle, T = Triangle); Esc and Enter are no longer Start in a menu.
		assert_eq!(menu_pad(&q, &keys(&[K::KeyG])).0, CIRCLE);
		assert_eq!(menu_pad(&q, &keys(&[K::KeyT])).0, TRIANGLE);
		assert_eq!(menu_pad(&q, &keys(&[K::Enter, K::Escape])).0 & START, 0);
		let qs = screen(&page(QUICK_SELECT, &[(0x4eec, CROSS), (0x4ee0, TRIANGLE)]));
		assert_eq!(menu_pad(&qs, &keys(&[K::KeyQ])), (L1, [0x80; 4]));
		assert_eq!(menu_pad(&qs, &keys(&[K::KeyE])), (R1, [0x80; 4]));
		assert_eq!(menu_pad(&qs, &keys(&[K::KeyW, K::ArrowLeft])), (UP | LEFT, [0x80; 4]));
		assert_eq!(menu_pad(&qs, &keys(&[K::KeyS, K::KeyD])).0, DOWN | RIGHT);
		assert_eq!(menu_pad(&qs, &keys(&[K::Enter])).0, CROSS);
		assert_eq!(menu_pad(&qs, &keys(&[K::Tab])).0, 0);
		// Root: Esc = Start (close).
		assert_eq!(menu_pad(&screen(&page(ROOT, &[])), &keys(&[K::Escape])).0, START);
		// Map: WASD is the left stick, arrows the d-pad, Tab = Circle.
		let m = screen(&page(MAP, &[]));
		let (b, s) = menu_pad(&m, &keys(&[K::KeyW, K::ArrowDown, K::Tab]));
		assert_eq!(b, DOWN | CIRCLE);
		assert_eq!(s[3], 0x01);
	}

	#[test]
	fn keys_held_across_a_switch_wait_for_release() {
		let mut l = Layer::default();
		// Esc opened the pause menu and is still down when the menu shows: no back press until it goes up.
		l.begin(false, &keys(&[K::Escape]));
		assert!(l.live(K::Escape));
		l.begin(true, &keys(&[K::Escape, K::KeyW]));
		assert!(!l.live(K::Escape));
		assert!(l.live(K::KeyW));
		l.begin(true, &keys(&[]));
		assert!(l.live(K::Escape));
		// Enter answered Yes and the game left the menu: Enter does not press Start in play until released.
		l.begin(false, &keys(&[K::Enter]));
		assert!(!l.live(K::Enter));
	}

	#[test]
	fn names_and_packing() {
		let q = screen(&page(QUIT_GAME, &[]));
		assert_eq!(key_for(&q, CIRCLE, false), Some("Enter"));
		assert_eq!(key_for(&q, TRIANGLE, false), Some("Esc"));
		assert_eq!(key_for(&q, CROSS, false), None);
		// Circle with no role on the screen: G (the left mouse button clicks the menu).
		assert_eq!(key_for(&DEFAULT, CIRCLE, false), Some("G"));
		assert_eq!(key_for(&DEFAULT, CROSS, true), Some("Ent"));
		assert_eq!(key_for(&DEFAULT, L1, false), Some("Q"));
		assert_eq!(key_for(&DEFAULT, R1, false), Some("E"));
		assert_eq!(key_for(&screen(&page(MAP, &[])), CIRCLE, false), Some("Tab"));
		for s in [q, DEFAULT, screen(&page(MAP, &[])), screen(&page(ROOT, &[]))] {
			assert_eq!(unpack(pack(s)), Some(s));
		}
		publish(None);
		assert_eq!(current(), None);
	}

	/// Mode-4 dialogs: the keys of each code, and the code words against the extracted overlays (the dialog function of every
	/// level stores mode 4, then its argument = code, through `lui` + `sw` in its first instructions).
	#[test]
	fn dialogs() {
		let quit_race = dialog(0);
		assert_eq!(menu_pad(&quit_race, &keys(&[K::Enter])).0, CROSS);
		assert_eq!(menu_pad(&quit_race, &keys(&[K::Escape])).0, CIRCLE);
		let quit = dialog(1);
		assert_eq!(menu_pad(&quit, &keys(&[K::Enter])).0, TRIANGLE);
		assert_eq!(menu_pad(&quit, &keys(&[K::Escape])).0, CROSS);
		assert_eq!(dialog(4), Screen { id: 0xf4, ..quit });
		assert_eq!(menu_pad(&dialog(2), &keys(&[K::Escape])).0, CROSS);
		assert_eq!(menu_pad(&dialog(5), &keys(&[K::Enter])).0, CROSS);
		assert_eq!((dialog(3).confirm, dialog(3).back), (CROSS, TRIANGLE));
		assert_eq!(dialog_on(&|_| 1, 4, 3), None);
		assert_eq!(dialog_on(&|a| if a == 0x173140 { 1 } else { 0 }, 4, 4), Some(quit));
		for s in [quit_race, quit, dialog(2)] {
			assert_eq!(unpack(pack(s)), Some(s));
		}
		let dir = [concat!(env!("CARGO_MANIFEST_DIR"), "/../extracted/levels")].into_iter().find(|d| std::path::Path::new(d).is_dir());
		let Some(dir) = dir else { return };
		let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().join("overlay.elf").exists()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
		names.sort_by_key(|n| n.split('_').next().and_then(|x| x.parse::<u32>().ok()).unwrap_or(99));
		assert_eq!(names.len(), 19);
		// The dialog function's entry per level (decomp/overlays/N_level/overlay.elf.c).
		const ENTRY: [u32; 19] = [
			0x2012e0, 0x223170, 0x2122c8, 0x1fabf8, 0x1f4840, 0x22ffc8, 0x219300, 0x2290c8, 0x213de0, 0x220f40,
			0x1f48c8, 0x232bc8, 0x224170, 0x20cd48, 0x2148f0, 0x1fa9d8, 0x1ffac8, 0x1fe308, 0x20a048,
		];
		/// `ENTRY` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
		const ENTRY_USA: [u32; 19] = [
			0x201730, 0x223640, 0x212798, 0x1fb0c8, 0x1f4d10, 0x230418, 0x2197d0, 0x229598, 0x2142b0, 0x221410,
			0x1f4d98, 0x233098, 0x224640, 0x20d218, 0x214d40, 0x1faea8, 0x1fff98, 0x1fe7d8, 0x20a518,
		];
		for (lv, name) in names.iter().enumerate() {
			let elf = std::fs::read(format!("{dir}/{name}/overlay.elf")).unwrap();
			let u = |o: usize| u32::from_le_bytes(elf[o..o + 4].try_into().unwrap()) as usize;
			let (phoff, phn) = (u(0x1c), u16::from_le_bytes([elf[0x2c], elf[0x2d]]) as usize);
			let word = |a: u32| -> u32 {
				(0..phn)
					.find_map(|i| {
						let p = phoff + 32 * i;
						let (ty, off, va, fs) = (u(p), u(p + 4), u(p + 8) as u32, u(p + 16));
						(ty == 1 && a >= va && (a - va) as usize + 4 <= fs).then(|| u(off + (a - va) as usize) as u32)
					})
					.unwrap()
			};
			let code = DIALOG_CODE[lv];
			let insns: Vec<u32> = (0..64).map(|i| word(ENTRY[lv] + 4 * i)).collect();
			// The first `sw` to `a` (lui base, 0x17 / 0x16, then sw lo(base)).
			let store = |a: u32| {
				let (hi, lo) = ((a + 0x8000) >> 16, a & 0xffff);
				insns.iter().enumerate().position(|(i, &w)| {
					let base = w >> 21 & 31;
					w >> 26 == 0x2b && w & 0xffff == lo && insns[..i].iter().any(|&l| l >> 16 == (0x0f << 10 | base) && l & 0xffff == hi)
				})
			};
			let (mode, at) = (store(0x15f5a4), store(code));
			assert!(mode.is_some() && at.is_some() && mode < at, "level {lv} {name}: mode {mode:?} code {at:?} ({code:#x}) at {:#x}", ENTRY[lv]);
		}
	}
}
