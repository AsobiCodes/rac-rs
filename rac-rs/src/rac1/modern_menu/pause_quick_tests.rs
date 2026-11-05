//! Tests of the modern Quick Select page (`pause_quick.rs`, v5i): the two grids and the ring on synthetic facts, the hit areas of the
//! eight slots and the short way round the ring cursor. Fixtures from `pause_grid_tests` and `pause_tests`.
use super::pause_grid_tests::{extra, grid, view, weapons, GRID};
use super::pause_quick::*;
use super::pause_tests::{facts, text_of};
use crate::retail::menu_model::extra::{GridCell, RingFacts};
use crate::retail::menu_model::pages::Page;
use rac_engine::ui::prim::Prim;

const RING: u32 = 0x1b34e8;

fn gadgets() -> crate::retail::menu_model::extra::GridFacts {
	grid(GRID + 0x100, 2, 3, (0..6u8).map(|k| GridCell { id: 20 + k, kind: 0, owned: k % 2 == 0, ammo: None }).collect())
}

fn built(cursor: u32) -> super::pause_grid::GridBuilt {
	let (w, g) = (weapons(), gadgets());
	let v = view(Page::QuickSelect, &[w.clone(), g.clone()], &[0, 0], 0, None);
	let mut x = extra(vec![w, g]);
	x.ring = Some(RingFacts { addr: RING, cursor, ids: [10, 22, 0, 0, 0, 0, 0, 0] });
	build(&facts(&v, &x, &text_of)).expect("the Quick Select page builds")
}

#[test]
fn the_page_lists_the_weapons_grid_then_the_gadgets_grid_and_eight_ring_slots() {
	let g = built(2);
	assert_eq!(g.cells.len(), 21);
	assert_eq!(g.groups.len(), 2);
	assert_eq!((g.groups[0].rows, g.groups[0].cols, g.groups[0].first), (3, 5, 0));
	assert_eq!((g.groups[1].rows, g.groups[1].cols, g.groups[1].first), (2, 3, 15));
	assert_eq!(g.targets[15], Some((GRID + 0x100, 0)));
	let r = g.ring.as_ref().expect("a ring");
	assert_eq!(r.cursor, 2);
	for i in 0..8u32 {
		let (x, y) = slot_centre(i);
		assert_eq!(r.slot_at(x, y), Some(i), "slot {i} answers a hit at its centre");
	}
	assert!(r.prev.w > 0.0 && r.next.w > 0.0 && r.slot_at(r.prev.cx(), r.prev.cy()).is_none());
	assert_eq!(g.hole.w, 0.0, "no 3D preview on this page");
}

#[test]
fn items_already_in_the_ring_wear_a_dot() {
	// Ring ids 10 (weapon cell 0, owned) and 22 (gadget cell 2, owned): two dots; 0 never counts.
	let dots = built(0).prims.iter().filter(|p| matches!(p, Prim::RRect { rect, border: None, .. } if rect.w == 8.0 && rect.h == 8.0)).count();
	assert_eq!(dots, 2);
}

#[test]
fn the_ring_cursor_turns_the_short_way() {
	assert_eq!(ring_steps(0, 0), (true, 0));
	assert_eq!(ring_steps(0, 3), (true, 3));
	assert_eq!(ring_steps(0, 4), (true, 4));
	assert_eq!(ring_steps(0, 5), (false, 3));
	assert_eq!(ring_steps(1, 0), (false, 1));
	assert_eq!(ring_steps(6, 1), (true, 3));
	assert_eq!(ring_steps(7, 6), (false, 1));
}

#[test]
fn a_page_without_a_ring_or_with_one_grid_stays_classic() {
	let (w, g) = (weapons(), gadgets());
	let v = view(Page::QuickSelect, &[w.clone(), g.clone()], &[0, 0], 0, None);
	assert!(build(&facts(&v, &extra(vec![w.clone(), g]), &text_of)).is_none(), "no ring facts");
	let v1 = view(Page::QuickSelect, &[w.clone()], &[0], 0, None);
	let mut x = extra(vec![w]);
	x.ring = Some(RingFacts { addr: RING, cursor: 0, ids: [0; 8] });
	assert!(build(&facts(&v1, &x, &text_of)).is_none(), "one grid");
}

#[test]
fn slot_k_sits_at_k_times_45_degrees_clockwise_from_the_top() {
	// The page ring and the in-game wheel (FUN_0027e100, FUN_00237e80): slot k at k x 45 degrees - 90 degrees, labels 1..8 = slots 0..7.
	let (top, bottom) = (slot_centre(0), slot_centre(4));
	let (cx, cy) = ((top.0 + bottom.0) / 2.0, (top.1 + bottom.1) / 2.0);
	assert!((top.0 - cx).abs() < 0.01 && top.1 < cy, "slot 0 (label 1) on top");
	let right = slot_centre(2);
	assert!(right.0 > cx && (right.1 - cy).abs() < 0.01, "slot 2 (label 3) on the right: clockwise");
	let (x5, y5) = slot_centre(5);
	assert!(x5 < cx && y5 > cy, "slot 5 (label 6) bottom-left, where the in-game wheel shows it");
	let left = slot_centre(6);
	assert!(left.0 < cx && (left.1 - cy).abs() < 0.01, "slot 6 (label 7) on the left");
}

#[test]
fn the_entry_cursor_is_the_first_empty_slot() {
	use crate::retail::menu_model::extra::entry_cursor;
	assert_eq!(entry_cursor(&[10, 12, 0, 0, 0, 0, 0, 0]), 2);
	assert_eq!(entry_cursor(&[0; 8]), 0);
	assert_eq!(entry_cursor(&[1, 2, 3, 4, 5, 6, 7, 0]), 7);
	assert_eq!(entry_cursor(&[0, 2, 0, 4, 5, 6, 7, 8]), 0);
	assert_eq!(entry_cursor(&[1, 2, 3, 4, 5, 6, 7, 8]), 0, "a full ring: slot 0");
}

/// The Quick Select page (id 5) at `PAGE` with only its ring item (game cursor `cursor`), as the worker reads it.
fn quick_snap(cursor: u32, state: u32) -> crate::retail::mouse_menu::MenuSnap {
	use crate::retail::mouse_menu::{Item, Kind, MenuSnap};
	let ring = Item { addr: RING, slot: 1, rect: [300, 40, 200, 200], kind: Kind::Ring { cursor }, texts: Vec::new(), planet_texts: Vec::new() };
	MenuSnap { page: PAGE, id: 5, state, cur: RING, items: vec![ring], front: false }
}

const PAGE: u32 = 0x1b2608;

fn ring_read(ram: &std::collections::HashMap<u32, u32>, m: &crate::retail::mouse_menu::MenuSnap, level: u32) -> Option<RingFacts> {
	crate::retail::menu_model::extra::read_at(Some(m), level, &|a| ram.get(&a).copied().unwrap_or(0))?.ring
}

#[test]
fn a_landed_page_shows_the_ring_items_copy_and_a_flying_one_what_its_entry_copies() {
	use crate::retail::menu_model::extra::RING_IDS;
	let pp = crate::retail::mouse_menu::page_ptr(3).expect("Kerwan's current-page word");
	let mut ram = std::collections::HashMap::new();
	// 0x141ea0 as the page found it (retail writes it back only when the page is left): Bomb Glove 10 in slot 0, Blaster 12 in slot 1.
	for (i, id) in [10u32, 12, 0, 0, 0, 0, 0, 0].into_iter().enumerate() {
		ram.insert(0x141ea0 + 4 * i as u32, id);
	}
	// The ring item's copy after one Equip of the glove into slot 5 (the glove left slot 0; retail turned the cursor on to 6).
	for (i, id) in [0u32, 12, 0, 0, 0, 10, 0, 0].into_iter().enumerate() {
		ram.insert(RING + RING_IDS + 4 * i as u32, id);
	}
	ram.insert(pp, PAGE);
	let live = RingFacts { addr: RING, cursor: 6, ids: [0, 12, 0, 0, 0, 10, 0, 0] };
	assert_eq!(ring_read(&ram, &quick_snap(6, 5), 3), Some(live), "landed: the Equip shows in the field it happens (not 0x141ea0)");
	assert_eq!(ring_read(&ram, &quick_snap(6, 5), u32::MAX), Some(live), "an unknown level reads the copy too");
	// Flying in: the current-page word is 0 (the target page sits at pp + 4), the copy and the cursor are the last visit's; the page
	// shows what its enter callback will copy: 0x141ea0 and the first empty slot.
	ram.insert(pp, 0);
	ram.insert(pp + 4, PAGE);
	assert_eq!(ring_read(&ram, &quick_snap(6, 1), 3), Some(RingFacts { addr: RING, cursor: 2, ids: [10, 12, 0, 0, 0, 0, 0, 0] }));
}
