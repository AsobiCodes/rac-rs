//! The Achievements page's input and card texts, a child of `pause_host.rs` (lane nach, 2026-11-03). While `Built::ach` is set
//! (`pause_ach.rs`: the open achievements list as a page of the modern pause menu) the page's keys, pad and mouse land here, never on
//! the retail menu behind: Up / Down (the wheel; held keys repeat) move the focus, Left / Right / Page Up / Page Down a page, Home /
//! End the ends, a hover focuses the card under the pointer, Back (Esc, Circle, Triangle, a right click, the hint) closes the list so
//! the page under it is the picture again. A click on a tab closes the list, then is that page's tab click (`child_tabs::after_list`:
//! another tab is switched to, the page's own tab stays or, from a sub page, goes back to the tab's main page). The focus and the
//! window live in the engine (`achievements::cursor`, reset by every `open`); the Classic style's HUD list keeps its own scroll
//! (`screen.rs` SCROLL). The page going away (the menu closing, `set_up(false)`) closes the list (`pause_host::close_ach`).
use super::{child_tabs, hit_at, State};
use crate::achievements;
use crate::modern_menu::pause_ach::AchBuilt;
use rac_engine::ui::input::{Action, MouseButton, MouseEvent};
use rac_engine::ui::kit::page::{PageHit, PageLayout, PanelBox, PANEL_PAD};
use rac_engine::ui::kit::HintKind;
use rac_engine::ui::prim::{Align, Border, FontRole, Prim, Rect, Rgba, TextRun};
use rac_engine::ui::theme::{ACCENT, HAIRLINE, SCROLLBAR, TEXT_BODY, TEXT_DISABLED, TEXT_MUTED, WHITE};

/// The page of the last frame is the achievements page.
pub(super) fn is_ach(st: &State) -> bool {
	st.built.as_ref().is_some_and(|b| b.ach.is_some())
}

fn ach(st: &State) -> Option<&AchBuilt> {
	st.built.as_ref()?.ach.as_ref()
}

/// The focus step of a menu action (None = not a move): rows, pages, the ends.
pub(super) fn step_of(a: Action, page: usize) -> Option<isize> {
	let page = page.max(1) as isize;
	match a {
		Action::Up => Some(-1),
		Action::Down => Some(1),
		Action::Left | Action::TabPrev | Action::PageUp => Some(-page),
		Action::Right | Action::TabNext | Action::PageDown => Some(page),
		Action::Home => Some(isize::MIN),
		Action::End => Some(isize::MAX),
		_ => None,
	}
}

/// One menu action (a key, its repeat, a pad press): a move, or Back closing the list; Confirm and the rest do nothing.
pub(super) fn act(st: &State, a: Action) {
	let Some(b) = ach(st) else { return };
	if matches!(a, Action::Back) {
		return achievements::close();
	}
	if let Some(d) = step_of(a, b.page) {
		achievements::move_focus(d, b.page, b.n);
	}
}

/// The list row of the card under window point (x, y); None on the banner, the shell or nothing.
fn row_at(st: &State, x: f32, y: f32) -> Option<usize> {
	match hit_at(st, x, y) {
		PageHit::Item(k) => ach(st)?.row_at(k),
		_ => None,
	}
}

fn focus_at(st: &State, x: f32, y: f32) {
	if let (Some(i), Some(b)) = (row_at(st, x, y), ach(st)) {
		achievements::focus_on(i, b.page, b.n);
	}
}

/// A left click: a tab closes the list, then acts on the page under it (`child_tabs::after_list`); the hint's Back closes; a card
/// takes the focus.
fn click(st: &mut State, x: f32, y: f32, now: u64) {
	if let Some(tab) = child_tabs::tab_at(st, x, y) {
		achievements::close();
		return child_tabs::after_list(st, tab, now);
	}
	match hit_at(st, x, y) {
		PageHit::Hint(HintKind::Back) => achievements::close(),
		_ => focus_at(st, x, y),
	}
}

/// A mouse event while the achievements page is up (all of them are the page's).
pub(super) fn mouse(st: &mut State, ev: MouseEvent, now: u64) {
	match ev {
		MouseEvent::Move { x, y } => focus_at(st, x, y),
		MouseEvent::Down { button: MouseButton::Left, x, y } => click(st, x, y, now),
		MouseEvent::Down { button: MouseButton::Right, .. } => achievements::close(),
		MouseEvent::Wheel { lines } if lines != 0.0 => act(st, if lines > 0.0 { Action::Up } else { Action::Down }),
		_ => {}
	}
}

fn rgba(c: [f32; 3], a: f32) -> Rgba {
	let u = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
	Rgba::rgba(u(c[0]), u(c[1]), u(c[2]), a)
}

fn text(x: f32, y: f32, role: FontRole, px: f32, colour: Rgba, max_w: f32, s: &str) -> Prim {
	Prim::Text(TextRun { x, y, role, px, colour, align: Align::Left, max_w, track: 0.0, text: s.to_string() })
}

/// The width the stat column (grade over status) keeps free at a card's right.
const STAT_W: f32 = 170.0;

/// The page's own prims over the kit's cards: the banner, per card the grade strip and the name over the description, the scroll
/// bar. The kit drew the cards, their selection and the grade / status stat.
pub(super) fn with_ach(mut prims: Vec<Prim>, layout: &PageLayout, a: &AchBuilt) -> Vec<Prim> {
	let Some(panel) = layout.panel.as_ref() else { return prims };
	if let (Some(words), Some(item)) = (&a.banner, panel.items.first()) {
		let r = item.rect;
		prims.push(Prim::RRect { rect: r, radius: 12.0, fill: ACCENT.tint(0.10), border: Some(Border { width: 1.5, colour: ACCENT }) });
		prims.push(text(r.x + PANEL_PAD, r.cy(), FontRole::SemiBold, 17.0, ACCENT, r.w - 2.0 * PANEL_PAD, words));
	}
	for c in &a.cards {
		let Some(item) = a.item_of(c.index).and_then(|k| panel.items.get(k)) else { continue };
		let r = item.rect;
		let (alpha, name, desc) = if c.unlocked { (1.0, WHITE, TEXT_BODY) } else { (0.35, TEXT_MUTED, TEXT_DISABLED) };
		prims.push(Prim::RRect { rect: Rect::new(r.x + 10.0, r.y + 14.0, 4.0, r.h - 28.0), radius: 2.0, fill: rgba(c.grade, alpha), border: None });
		let (x, w) = (r.x + PANEL_PAD + 8.0, r.w - 2.0 * PANEL_PAD - 8.0 - STAT_W);
		prims.push(text(x, r.cy() - 12.0, FontRole::Condensed, 22.0, name, w, &c.name));
		prims.push(text(x, r.cy() + 14.0, FontRole::Regular, 14.0, desc, w, &c.desc));
	}
	scroll_bar(&mut prims, panel, a);
	prims
}

/// The thin bar right of the achievement cards (only when the list is longer than the window): the track and the window's thumb.
fn scroll_bar(prims: &mut Vec<Prim>, panel: &PanelBox, a: &AchBuilt) {
	if a.n <= a.page {
		return;
	}
	let skip = usize::from(a.banner.is_some());
	let (Some(top), Some(last)) = (panel.items.get(skip), panel.items.last()) else { return };
	let (x, y0) = (top.rect.right() + 6.0, top.rect.y);
	let track = last.rect.bottom() - y0;
	let thumb = (track * a.page as f32 / a.n as f32).max(24.0).min(track);
	let y = y0 + (track - thumb) * a.first as f32 / (a.n - a.page) as f32;
	prims.push(Prim::RRect { rect: Rect::new(x, y0, 4.0, track), radius: 2.0, fill: HAIRLINE, border: None });
	prims.push(Prim::RRect { rect: Rect::new(x, y, 4.0, thumb), radius: 2.0, fill: SCROLLBAR, border: None });
}
