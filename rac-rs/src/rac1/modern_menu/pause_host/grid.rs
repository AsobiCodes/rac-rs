//! The icon-grid pages (Weapons, Gadgets, Items) in the pause host (v0.5 screens stage D3), a child of `pause_host.rs` (it uses the
//! host's private state). An arrow moves the focus INSIDE the page's grid at once (a local focus plus a `Select` request); at the edge of
//! a grid the real D-pad press goes to the game, which wraps or jumps to the next group exactly as retail does (the local focus is
//! dropped, so the game's cursor shows). A hover selects a cell, a click or the action button confirms. `with_grid` puts the page's
//! panels into the draw list (the scrim gets the preview's hole) and `place_preview` draws the preview crop into that hole.
use super::{back, current, live, lock, select, up, State};
use crate::modern_menu::pause_grid::{holed_scrim, Dir, GridBuilt};
use crate::retail::menu_model::{submit, Command};
use crate::modern_menu::pause_quick::ring_steps;
use crate::retail::mouse_menu::{DOWN, L1, LEFT, R1, RIGHT, UP};
use rac_engine::ui::input::Action;
use rac_engine::ui::kit::shell::SHELL_SCRIM;
use rac_engine::ui::kit::Scale;
use rac_engine::ui::prim::Prim;

fn grid_of(st: &State) -> Option<&GridBuilt> {
	st.built.as_ref()?.grid.as_ref()
}

/// Is the page up an icon-grid page?
pub(super) fn is_grid(st: &State) -> bool {
	grid_of(st).is_some()
}

fn design(st: &State, x: f32, y: f32) -> (f32, f32) {
	Scale::fit(st.win.0, st.win.1).to_design(x, y)
}

/// A key, a pad press or a repeat on a grid page. Inside a grid the focus moves locally; at its edge the game gets the D-pad.
pub(super) fn act_grid(st: &mut State, a: Action, now: u64) {
	let (d, bit) = match a {
		Action::Up => (Dir::Up, UP),
		Action::Down => (Dir::Down, DOWN),
		Action::Left => (Dir::Left, LEFT),
		Action::Right => (Dir::Right, RIGHT),
		Action::Confirm | Action::Peek => return select(st, current(st, now), true, now),
		Action::Back => return back(st),
		// Quick Select: L1 / R1 (Q / E) turn the ring.
		Action::TabPrev | Action::TabNext => return turn_ring(st, matches!(a, Action::TabNext), 1),
		_ => return,
	};
	let cur = current(st, now);
	match grid_of(st).and_then(|g| g.step(cur, d)) {
		Some(i) if live(st, i) => select(st, i, false, now),
		_ => {
			st.local = None;
			submit(Command::Press(bit));
		}
	}
}

/// Quick Select: `n` L1 / R1 pulses turn the game's ring cursor (nothing on the other grid pages).
fn turn_ring(st: &State, next: bool, n: u32) {
	if grid_of(st).is_some_and(|g| g.ring.is_some()) {
		for _ in 0..n {
			submit(Command::Press(if next { R1 } else { L1 }));
		}
	}
}

/// The flat cell under window point (x, y).
pub(super) fn cell_at(st: &State, x: f32, y: f32) -> Option<usize> {
	let (dx, dy) = design(st, x, y);
	grid_of(st)?.cell_at(dx, dy)
}

/// A left click on a grid page: a cell confirms itself, the action button confirms the focused cell. True = the page took it.
pub(super) fn click_grid(st: &mut State, x: f32, y: f32, now: u64) -> bool {
	let (dx, dy) = design(st, x, y);
	let Some(g) = grid_of(st) else { return false };
	let (cell, button, ring) = (g.cell_at(dx, dy), g.on_button(dx, dy), g.ring.clone());
	if let Some(r) = ring {
		// The ring's arrows and slot discs turn the game's cursor by pulses (the slot's own item is picked in the grids).
		if r.prev.contains(dx, dy) || r.next.contains(dx, dy) {
			turn_ring(st, r.next.contains(dx, dy), 1);
			return true;
		}
		if let Some(slot) = r.slot_at(dx, dy) {
			let (next, n) = ring_steps(r.cursor, slot);
			turn_ring(st, next, n);
			return true;
		}
	}
	if let Some(i) = cell.filter(|&i| live(st, i)) {
		select(st, i, true, now);
	} else if button {
		select(st, current(st, now), true, now);
	} else {
		// The hint bar's Select / Back are the page's to handle (`click`, R3-4); any other click on a grid page is spent here.
		return !matches!(super::hit_at(st, x, y), rac_engine::ui::kit::page::PageHit::Hint(_));
	}
	true
}

/// The draw list of a grid page: the shell's giant scrim becomes four strips around the preview's hole, then the grid panels follow.
pub(super) fn with_grid(mut prims: Vec<Prim>, g: &GridBuilt) -> Vec<Prim> {
	if let Some(at) = prims.iter().position(|p| matches!(p, Prim::RRect { fill, .. } if *fill == SHELL_SCRIM)) {
		prims.splice(at..=at, holed_scrim(g.hole));
	}
	prims.extend(g.prims.iter().cloned());
	prims
}

/// `frame_ui::hide_retail`, after the retail overlay was dropped: draws the game's own Ratchet (or the gold bolt) into the page's hole,
/// a crop of the retail overlay picture, aspect kept.
pub fn place_preview(hud: &mut Vec<crate::overlay::Quad>, win: (u32, u32)) {
	if !up() {
		return;
	}
	let Some((hole, uv)) = lock().as_ref().and_then(grid_of).map(|g| (g.hole, g.uv)) else { return };
	if hole.w <= 0.0 || hole.h <= 0.0 {
		return;
	}
	crate::modern_menu::crop::place(hud, win, hole, uv);
}
