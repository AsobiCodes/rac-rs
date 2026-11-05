//! The modern Options pages' door (v0.5 modern menu, stage S3): Cross on the Graphics or Camera row of the retail Options list
//! (and, in the pause list, on the Display, Game and Camera rows) opens the engine's modern page instead of the retail one. Cross reaches the game from three places (keyboard / pad on the main
//! thread, the mouse's pulses on the main thread, and the worker's instant selection that adds CROSS for a click on the selected
//! row, mouse_instant.rs), and the only point after all of them is the worker's pad of the field, so the press is taken HERE,
//! between `mouse_instant::field` and `set_pad` (worker/field_run.rs). The menu is read from CURRENT RAM (the cursor the game acts
//! on in this field: a quick Down + Cross decides on the row the cursor really is on, as retail does), the field's menu buttons
//! are cleared so the game never sees the press (no retail Camera / Graphics page, no cursor step), and an open request is queued
//! for the main thread (`take_open`, polled by the display-frame step in `modern_menu::host`).
//!
//! Off (the Classic menu style, the UI pipes refused, the UI already up, a snapshot that is not the idle Options list): the pad
//! is returned unchanged, bit for bit. `decide` is the pure core (no Machine) so the tests below need no game.
use super::mouse_menu::{Kind, MenuSnap, CROSS, MENU_BTNS};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The Options page's id in the menu snapshot (state = the same id while it is idle).
const OPTIONS_PAGE: u32 = 8;
/// Text ids of the list rows (pcext.rs `ID_GRAPHICS`, `ID_CAMERA`; pcext/tabrows.rs: Display and Game, pause list only).
const ID_GRAPHICS: u16 = 0x7f06;
const ID_CAMERA: u16 = 0x4f23;
const ID_DISPLAY: u16 = crate::retail::pcext::tabrows::ID_DISPLAY_TAB as u16;
const ID_GAME: u16 = crate::retail::pcext::tabrows::ID_GAME_TAB as u16;

/// The modern page to open: the tab id is the one of the rows table (`modern_menu` tests keep them equal).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
	Graphics,
	Camera,
	Display,
	Game,
}

impl Tab {
	pub fn id(self) -> &'static str {
		match self {
			Tab::Graphics => "graphics",
			Tab::Camera => "camera",
			Tab::Display => "display",
			Tab::Game => "game",
		}
	}
}

/// An open request for the main thread: `front` = from the title screen's Options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Open {
	pub tab: Tab,
	pub front: bool,
}

/// The oldest request not yet taken (a held Cross asks again every field: the request stays one).
static REQUEST: Mutex<Option<Open>> = Mutex::new(None);
/// The render thread's verdict on the UI pipes (`ui::gpu::refused()`), set each display frame: the worker cannot see the Gpu.
static GPU_REFUSED: AtomicBool = AtomicBool::new(false);

pub fn set_gpu_refused(refused: bool) {
	GPU_REFUSED.store(refused, Ordering::Relaxed);
}

/// Main thread: takes the pending open request.
pub fn take_open() -> Option<Open> {
	REQUEST.lock().ok().and_then(|mut r| r.take())
}

/// Whether the press `pad` on this snapshot is one the modern menu takes: the idle Options page, its list item current, the
/// cursor on the Graphics row (any list) or the Camera, Display or Game row (the pause Options list), Cross down. `modern` = the menu style and
/// `gpu_ok` = the UI pipes work; either false = never.
pub fn decide(menu: Option<&MenuSnap>, pad: u16, modern: bool, gpu_ok: bool) -> Option<Open> {
	if !modern || !gpu_ok || pad & CROSS == 0 {
		return None;
	}
	let m = menu?;
	if m.busy() {
		return None;
	}
	// The pause menu's Options page has id 8; the title front end's pages have ids of their own (menu_keys::screen), so there the
	// row's text id alone decides (0x7f06 is the PC Graphics row, used by no other list).
	if !m.front && !(m.id == OPTIONS_PAGE && m.state == OPTIONS_PAGE) {
		return None;
	}
	let it = m.items.iter().find(|i| i.addr == m.cur)?;
	let Kind::Label { cursor, .. } = it.kind else { return None };
	let tab = match it.texts.get(cursor as usize).copied()? {
		ID_GRAPHICS => Tab::Graphics,
		ID_CAMERA if !m.front => Tab::Camera,
		ID_DISPLAY if !m.front => Tab::Display,
		ID_GAME if !m.front => Tab::Game,
		_ => return None,
	};
	Some(Open { tab, front: m.front })
}

/// Worker, once per field after `mouse_instant::field`: the pad for the game. A press the modern menu takes clears the menu
/// buttons and queues the open request; everything else returns `pad` unchanged.
pub fn filter(m: &ee_oracle::Machine, front: bool, pad: u16) -> u16 {
	if pad & CROSS == 0 || rac_engine::ui::is_open() {
		return pad;
	}
	let modern = crate::modern_menu::style::modern();
	let gpu_ok = !GPU_REFUSED.load(Ordering::Relaxed);
	// The pack has no modern menu bound (pack_bind.rs `ui: None` until the host is wired): the game keeps its own pages.
	if !modern || !gpu_ok || rac_engine::pack::get().ui.is_none() {
		return pad;
	}
	let snap = {
		let rd = |a: u32| m.rd32(a);
		if front {
			super::mouse_menu::read_front(&rd, rd(super::title_boot::front_mode()))
		} else {
			super::mouse_menu::read(&rd, m.level(), rd(super::disc::va(0x15f5a4)))
		}
	};
	match decide(snap.as_ref(), pad, modern, gpu_ok) {
		Some(open) => {
			request(open);
			pad & !MENU_BTNS
		}
		None => pad,
	}
}

fn request(open: Open) {
	if let Ok(mut r) = REQUEST.lock() {
		r.get_or_insert(open);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::mouse_menu::Item;

	use crate::retail::mouse_menu::DOWN;
	/// The pause Options list as the game holds it: the retail six, Graphics, Controls, Mods, Achievements, Quit Game.
	const IDS: [u16; 10] = [0x4f1e, 0x4f1f, 0x4f20, 0x4f22, 0x4f23, 0x4f24, 0x7f06, 0x7f13, 0x7f70, 0x4f26];
	/// ... and with the Modern style's two extra rows: Display before Graphics, Game after it.
	const IDS_TABS: [u16; 12] = [0x4f1e, 0x4f1f, 0x4f20, 0x4f22, 0x4f23, 0x4f24, 0x7f34, 0x7f06, 0x7f35, 0x7f13, 0x7f70, 0x4f26];

	fn list(id: u32, state: u32, cursor: u32, front: bool) -> MenuSnap {
		list_of(&IDS, id, state, cursor, front)
	}

	fn list_of(ids: &[u16], id: u32, state: u32, cursor: u32, front: bool) -> MenuSnap {
		let texts = ids.to_vec();
		let rows = texts.iter().enumerate().map(|(i, _)| (30.0 + 12.0 * i as f32, true)).collect();
		let it = Item { addr: 0x40, slot: 1, rect: [0, 0, 100, 100], kind: Kind::Label { rows, rowh: 12.0, cursor }, texts, planet_texts: Vec::new() };
		MenuSnap { page: 1, id, state, cur: 0x40, items: vec![it], front }
	}

	fn pause(cursor: u32) -> MenuSnap {
		list(8, 8, cursor, false)
	}

	#[test]
	fn cross_on_graphics_and_camera_opens_the_modern_page() {
		assert_eq!(decide(Some(&pause(6)), CROSS, true, true), Some(Open { tab: Tab::Graphics, front: false }));
		assert_eq!(decide(Some(&pause(4)), CROSS, true, true), Some(Open { tab: Tab::Camera, front: false }));
		// A pad Cross with other buttons down (the mouse's pulse is CROSS alone; a held stick / shoulder rides along).
		assert_eq!(decide(Some(&pause(6)), CROSS | 0x4 | 0x2000, true, true), Some(Open { tab: Tab::Graphics, front: false }));
	}

	#[test]
	fn cross_on_any_other_row_passes_through() {
		for cursor in [0u32, 1, 2, 3, 5, 7, 8, 9] {
			assert_eq!(decide(Some(&pause(cursor)), CROSS, true, true), None, "row {cursor}");
		}
		assert_eq!(decide(Some(&pause(6)), 0, true, true), None, "no Cross");
		assert_eq!(decide(Some(&pause(6)), 0x20, true, true), None, "Circle is back, not a press");
		assert_eq!(decide(None, CROSS, true, true), None);
	}

	#[test]
	fn mouse_submit_confirm_is_swallowed_on_the_worker() {
		// The mouse's click on the already selected row: mouse_instant::field returns pad | CROSS (a pulse of Cross alone).
		let from_mouse = 0 | CROSS;
		assert_eq!(decide(Some(&pause(6)), from_mouse, true, true), Some(Open { tab: Tab::Graphics, front: false }));
		// ... and the same click on a row that is not ours is the game's.
		assert_eq!(decide(Some(&pause(7)), from_mouse, true, true), None);
	}

	#[test]
	fn pad_cross_is_swallowed_and_no_cross_reaches_retail() {
		let open = decide(Some(&pause(4)), CROSS | DOWN, true, true);
		assert!(open.is_some());
		// The pad the game gets: every menu button of the field cleared (Cross and the Down that rode along), no Cross left.
		assert_eq!((CROSS | DOWN) & !MENU_BTNS, 0);
		assert_eq!(MENU_BTNS & CROSS, CROSS);
	}

	#[test]
	fn quick_down_then_cross_decides_on_current_ram() {
		// RAM at the start of field N already holds field N-1's cursor move. Down in N-1 moved the cursor from Camera (4) onto row 5:
		// a Cross in N acts on row 5 (the game's Controls-style row), not on Camera and not on Graphics.
		assert_eq!(decide(Some(&pause(5)), CROSS, true, true), None);
		// Down again in N moved it onto Graphics (6): the Cross of N+1 opens the modern page.
		assert_eq!(decide(Some(&pause(6)), CROSS, true, true), Some(Open { tab: Tab::Graphics, front: false }));
		// Down + Cross in the SAME field acts on the cursor the field started with (retail's own rule): row 5 is not ours.
		assert_eq!(decide(Some(&pause(5)), CROSS | DOWN, true, true), None);
		assert_eq!(decide(Some(&pause(6)), CROSS | DOWN, true, true), Some(Open { tab: Tab::Graphics, front: false }));
	}

	#[test]
	fn modern_vs_classic_style_selects_path() {
		assert_eq!(decide(Some(&pause(6)), CROSS, false, true), None, "Classic style");
		assert_eq!(decide(Some(&pause(6)), CROSS, true, false), None, "UI pipes refused: the classic page opens");
		assert_eq!(decide(Some(&pause(6)), CROSS, false, false), None, "Classic and refused");
		assert_eq!(decide(Some(&pause(6)), CROSS, true, true), Some(Open { tab: Tab::Graphics, front: false }), "Modern and the pipes ok");
	}

	#[test]
	fn only_the_idle_options_page_counts() {
		assert_eq!(decide(Some(&list(8, 1, 6, false)), CROSS, true, true), None, "flying in");
		assert_eq!(decide(Some(&list(8, 0x14, 6, false)), CROSS, true, true), None, "closing");
		assert_eq!(decide(Some(&list(3, 3, 6, false)), CROSS, true, true), None, "another page");
	}

	#[test]
	fn title_options_opens_graphics_only() {
		let t = list(2, 2, 6, true);
		assert_eq!(decide(Some(&t), CROSS, true, true), Some(Open { tab: Tab::Graphics, front: true }));
		// The title's Options has no modern Camera page (the game's own Camera row there stays the game's).
		assert_eq!(decide(Some(&list(2, 2, 4, true)), CROSS, true, true), None);
		assert_eq!(decide(Some(&list(2, 1, 6, true)), CROSS, true, true), None, "busy");
	}

	#[test]
	fn cross_on_display_and_game_opens_their_tabs_in_the_pause_list_only() {
		let at = |cursor| list_of(&IDS_TABS, 8, 8, cursor, false);
		assert_eq!(decide(Some(&at(6)), CROSS, true, true), Some(Open { tab: Tab::Display, front: false }));
		assert_eq!(decide(Some(&at(7)), CROSS, true, true), Some(Open { tab: Tab::Graphics, front: false }));
		assert_eq!(decide(Some(&at(8)), CROSS, true, true), Some(Open { tab: Tab::Game, front: false }));
		for cursor in [0u32, 1, 2, 3, 9, 10, 11] {
			assert_eq!(decide(Some(&at(cursor)), CROSS, true, true), None, "row {cursor}");
		}
		assert_eq!(decide(Some(&at(6)), CROSS, false, true), None, "Classic style: the game's own page");
		assert_eq!(decide(Some(&list_of(&IDS_TABS, 2, 2, 6, true)), CROSS, true, true), None, "the title list has no such rows");
		assert_eq!((Tab::Display.id(), Tab::Game.id()), ("display", "game"));
	}

	#[test]
	fn request_stays_one_until_taken() {
		take_open();
		request(Open { tab: Tab::Graphics, front: false });
		request(Open { tab: Tab::Camera, front: true });
		assert_eq!(take_open(), Some(Open { tab: Tab::Graphics, front: false }));
		assert_eq!(take_open(), None);
	}
}
