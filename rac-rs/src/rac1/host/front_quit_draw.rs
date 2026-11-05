//! The Classic title's "Quit to Desktop" (v0.5 menu lane; plan `scratch_perf_menu/QUIT_PLAN.md`, state machine `retail/title_quit.rs`):
//! the host's FOURTH root entry and its question, drawn in the mirror renderer's own retail look. The front end's root page has three
//! retail panels (`frontend.rs` `ROOT_PANELS`); the fourth is one pitch below Options (`title_quit::PANEL`) and uses the panels'
//! own art (`Menu::draw_frame`: shadow, near-black fill, gold band, scanlines), label font and highlight glow, so only its fly-in is
//! missing: it fades in once the three have landed. The question is retail's Quit box (`Page::Quit`: the box at `title_quit::BOX`,
//! header, wrapped body, "No" left and "Yes" right) over the dimmed page. Called by `front_modern::draw_menu` right after
//! `Menu::draw` and before the 4:3 widening, so the quads are widened with the menu. Nothing here touches retail RAM; the picture of
//! the entry follows `title_quit::focus()`, the question follows `title_quit::ask()`.
use crate::frontend::{fit_text, Art, Menu, Page, Text, MOVE_TICKS, PANEL, SCREEN, SELECTED, UNSELECTED};
use crate::overlay::Quad;
use crate::retail::mouse_menu::MenuSnap;
use crate::retail::pcext::quitrow::{ID_QUITD, ID_QUITD_BODY, ID_QUITD_TITLE};
use crate::retail::{prompts, title_quit};

/// Retail's "No" and "Yes" words (the Quit Game page's answers).
const NO: u32 = 0x524b;
const YES: u32 = 0x524f;
/// The panels' drop shadow, 0x80100808 (`Menu::draw`).
const SHADOW: [f32; 4] = [16.0 / 255.0, 8.0 / 255.0, 8.0 / 255.0, 1.0];
/// The page behind the question is dimmed by this much black.
const DIM: f32 = 0.55;
/// The entry fades in over this many page ticks once the retail panels have landed (the glow's length).
const FADE_TICKS: f32 = 10.0;
/// The box's header bar (retail's `Page::Quit` title rect).
const TITLE: [f32; 4] = [173.0, 129.0, 166.0, 30.0];
/// The pulse slot of the box's frame (the root page uses slots 0..3 for its panels, 3 for the entry).
const BOX_SLOT: usize = 4;

/// The row the front end's highlight sits on: the host's entry while it is highlighted on the root page, else the game's own.
pub(super) fn sel(page: Page, game_sel: usize) -> usize {
	if page == Page::Root && title_quit::focus() && !crate::modern_menu::title_host::up() {
		title_quit::ROW
	} else {
		game_sel
	}
}

/// How much of the entry shows after `page_time` seconds on the root page: nothing while the retail panels fly in, then a linear fade.
fn fade(page_time: f32) -> f32 {
	((page_time * 50.0 - MOVE_TICKS) / FADE_TICKS).clamp(0.0, 1.0)
}

/// The label's colour: lavender, golden as the glow (0..1) rises (`Menu::draw`).
fn label_colour(glow: f32) -> [f32; 4] {
	std::array::from_fn(|j| UNSELECTED[j] + (SELECTED[j] - UNSELECTED[j]) * glow)
}

/// The header's place inside the title bar (`Menu::draw`: 6 px in across, 4 down).
fn header_rect() -> [f32; 4] {
	[TITLE[0] + 6.0, TITLE[1] + 4.0, TITLE[2] - 12.0, TITLE[3] - 8.0]
}

/// The body's first line (`Menu::draw`, `Page::Quit`).
fn body_top() -> f32 {
	title_quit::BOX[1] + 36.0
}

/// A body line's height for `n` lines: the room above the answers shared out, at most 16.
fn body_line(n: usize) -> f32 {
	((title_quit::BOX[1] + title_quit::BOX[3] - 40.0 - body_top()) / n.max(1) as f32).min(16.0)
}

/// The answers' places, "No" then "Yes" (`Menu::draw`, `Page::Quit`).
fn answers() -> [[f32; 4]; 2] {
	let r = title_quit::BOX;
	let y = r[1] + r[3] - 30.0;
	[[r[0] + 8.0, y, r[2] / 2.0 - 12.0, 20.0], [r[0] + r[2] / 2.0 + 4.0, y, r[2] / 2.0 - 12.0, 20.0]]
}

/// One panel like the retail ones: shadow, then the art (a flat fill when the title level has none).
fn framed(out: &mut Vec<Quad>, menu: &Menu, art: &Art, r: [f32; 4], slot: usize, k: f32, ox: f32) {
	crate::overlay::rect(out, [ox + (r[0] + 2.0) * k, (r[1] + 2.0) * k, (r[2] - 3.0).max(0.0) * k, (r[3] - 3.0).max(0.0) * k], SHADOW);
	match art.frame {
		Some(f) => menu.draw_frame(out, &f, r, k, ox, slot),
		None => crate::overlay::rect(out, [ox + r[0] * k, r[1] * k, (r[2] - 1.0).max(0.0) * k, (r[3] - 1.0).max(0.0) * k], PANEL),
	}
}

fn scale_alpha(quads: &mut [Quad], t: f32) {
	for q in quads {
		q.colour[3] *= t;
	}
}

/// The entry's panel and label, faded in by `t`.
fn entry(out: &mut Vec<Quad>, menu: &Menu, art: &Art, label: &str, (k, ox): (f32, f32), t: f32) {
	let n = out.len();
	let [x, y, w, h] = title_quit::PANEL;
	framed(out, menu, art, title_quit::PANEL, title_quit::ROW, k, ox);
	let glow = menu.glow.get(title_quit::ROW).copied().unwrap_or(0.0);
	fit_text(out, &art.font, label, [x + 4.0, y + 2.0, w - 8.0, h - 3.0], k, ox, label_colour(glow));
	scale_alpha(&mut out[n..], t);
}

/// The question over the dimmed page: retail's Quit box with this text.
fn question(out: &mut Vec<Quad>, menu: &Menu, art: &Art, text: &Text, (w, h): (f32, f32), (k, ox): (f32, f32)) {
	crate::overlay::rect(out, [0.0, 0.0, w, h], [0.0, 0.0, 0.0, DIM]);
	let r = title_quit::BOX;
	framed(out, menu, art, r, BOX_SLOT, k, ox);
	crate::frontend_lines::fit_header(out, &art.font, &prompts::for_text(text.get(ID_QUITD_TITLE)), header_rect(), k, ox, UNSELECTED, false);
	let lines = crate::help::wrap(&art.font, text.get(ID_QUITD_BODY), r[2] - 16.0);
	let lh = body_line(lines.len());
	for (i, l) in lines.iter().enumerate() {
		fit_text(out, &art.font, l, [r[0] + 8.0, body_top() + i as f32 * lh, r[2] - 16.0, lh], k, ox, UNSELECTED);
	}
	for (id, place) in [NO, YES].into_iter().zip(answers()) {
		fit_text(out, &art.font, &prompts::for_text(text.get(id)), place, k, ox, UNSELECTED);
	}
}

/// The Classic title's extra quads of this frame, after `Menu::draw`: the entry's panel (root page, once the retail panels have
/// landed) and, while the question is up, the box over the dimmed page. Only on the front end's root page; elsewhere, and while the
/// label is unknown, nothing.
pub(super) fn draw(out: &mut Vec<Quad>, (w, h): (f32, f32), menu: &Menu, art: &Art, text: &Text, snap: Option<&MenuSnap>) {
	if menu.page != Page::Root || !snap.is_some_and(title_quit::is_root) {
		return;
	}
	let label = prompts::for_text(text.get(ID_QUITD));
	if label.is_empty() {
		return;
	}
	let k = h / SCREEN[1];
	let at = (k, (w - SCREEN[0] * k) / 2.0);
	let t = fade(menu.page_time);
	if t > 0.0 {
		entry(out, menu, art, &label, at, t);
	}
	if title_quit::ask() {
		question(out, menu, art, text, (w, h), at);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::frontend::Pics;
	use crate::hud::Glyph;
	use crate::retail::menu_model::cmd::TEST_LOCK;
	use std::sync::MutexGuard;

	fn world() -> MutexGuard<'static, ()> {
		let g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		crate::modern_menu::title_host::set_up(false); // the Classic style: `sel` only steers then
		title_quit::clear();
		let _ = rac_engine::quit::take();
		g
	}

	/// A font of 8-unit letters (every byte), so a one-letter text is two quads (shadow, colour) in a 16-unit tall box.
	fn art() -> Art {
		let glyphs = (0..256).map(|_| Glyph { u: 0, v: 0, dy: 0, adv: 8 }).collect();
		Art { font: (3, glyphs), font2: None, logo: None, press: Vec::new(), vol: [None, None], frame: None, icons: Default::default(), pics: Pics::default() }
	}

	fn texts() -> Text {
		let mut t = Text::default();
		for (id, s) in [(ID_QUITD, "Q"), (ID_QUITD_TITLE, "H"), (ID_QUITD_BODY, "B"), (NO, "N"), (YES, "Y")] {
			t.insert(id, s.to_string());
		}
		t
	}

	fn root_menu(page_time: f32) -> Menu {
		let mut m = Menu::new(0);
		m.page = Page::Root;
		m.page_time = page_time;
		m
	}

	/// 512 x 448 window: k = 1, no side bars.
	const WIN: (f32, f32) = (512.0, 448.0);

	fn drawn(menu: &Menu, snap: Option<&MenuSnap>) -> Vec<Quad> {
		let mut out = Vec::new();
		draw(&mut out, WIN, menu, &art(), &texts(), snap);
		out
	}

	fn inside(q: &Quad, r: [f32; 4], slack: f32) -> bool {
		q.rect[0] >= r[0] - slack && q.rect[1] >= r[1] - slack && q.rect[0] + q.rect[2] <= r[0] + r[2] + slack && q.rect[1] + q.rect[3] <= r[1] + r[3] + slack
	}

	fn centre(q: &Quad) -> (f32, f32) {
		(q.rect[0] + q.rect[2] / 2.0, q.rect[1] + q.rect[3] / 2.0)
	}

	fn within(r: [f32; 4], (x, y): (f32, f32)) -> bool {
		x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
	}

	#[test]
	fn the_highlight_moves_to_the_entry_only_on_the_root_page_while_it_is_focused() {
		let _w = world();
		assert_eq!(sel(Page::Root, 2), 2, "not focused: the game's row");
		title_quit::set_focus(true);
		assert_eq!(sel(Page::Root, 2), title_quit::ROW, "focused on the root page: the entry");
		assert_eq!(sel(Page::Options, 2), 2, "another page keeps the game's row");
		assert_eq!(title_quit::ROW, 3, "the fourth row");
		title_quit::set_focus(false);
		assert_eq!(sel(Page::Root, 0), 0);
	}

	#[test]
	fn the_entry_fades_in_after_the_retail_panels_have_landed() {
		assert_eq!(fade(0.0), 0.0);
		assert!(fade(MOVE_TICKS / 50.0) < 1e-4, "the three panels are still flying in");
		let mid = fade((MOVE_TICKS + FADE_TICKS / 2.0) / 50.0);
		assert!((mid - 0.5).abs() < 1e-4, "{mid}");
		assert!((fade((MOVE_TICKS + FADE_TICKS) / 50.0) - 1.0).abs() < 1e-4);
		assert_eq!(fade(60.0), 1.0, "and stays");
		let steps: Vec<f32> = (0..40).map(|t| fade(t as f32 / 50.0)).collect();
		assert!(steps.windows(2).all(|p| p[0] <= p[1]), "never goes back");
	}

	#[test]
	fn the_label_glows_from_lavender_to_gold_like_the_retail_rows() {
		assert_eq!(label_colour(0.0), UNSELECTED);
		assert!(label_colour(1.0).iter().zip(SELECTED).all(|(a, b)| (a - b).abs() < 1e-6));
		let half = label_colour(0.5);
		assert!(half[0] > UNSELECTED[0] && half[0] < SELECTED[0] && half[2] < UNSELECTED[2] && half[2] > SELECTED[2]);
	}

	#[test]
	fn the_boxs_pieces_sit_where_retails_quit_box_has_them_and_the_answers_sit_in_their_zones() {
		let r = title_quit::BOX;
		let h = header_rect();
		assert!(h[0] >= TITLE[0] && h[1] >= TITLE[1] && h[0] + h[2] <= TITLE[0] + TITLE[2] && h[1] + h[3] <= TITLE[1] + TITLE[3], "header inside its bar");
		assert!(TITLE[0] == r[0] && TITLE[2] == r[2] && TITLE[1] > r[1], "the bar is the top of the box");
		let [no, yes] = answers();
		for (label, zone) in [(no, title_quit::NO_ZONE), (yes, title_quit::YES_ZONE)] {
			assert!(label[0] >= zone[0] && label[1] >= zone[1] && label[0] + label[2] <= zone[0] + zone[2] && label[1] + label[3] <= zone[1] + zone[3], "{label:?} in {zone:?}");
		}
		assert!(no[0] < yes[0], "No on the left, Yes on the right");
		assert!(no[1] + no[3] <= r[1] + r[3], "answers inside the box");
		// The body: lines fill the room above the answers, 16 high at most, never a division by zero.
		assert_eq!(body_line(0), body_line(1));
		assert_eq!(body_line(1), 16.0);
		assert_eq!(body_line(3), 16.0);
		let last = |n: usize| body_top() + n as f32 * body_line(n);
		assert!(last(3) <= no[1], "three lines end above the answers");
		assert!(last(7) <= no[1] + 1e-3, "many lines are squeezed, not pushed down");
		assert!(body_line(7) < 16.0);
	}

	#[test]
	fn nothing_is_drawn_off_the_root_page_without_the_label_or_before_the_panels_have_landed() {
		let _w = world();
		title_quit::set_ask(true);
		let root = crate::retail::title_quit::test_root(2);
		assert!(drawn(&root_menu(5.0), None).is_empty(), "no snapshot");
		let mut pause = root.clone();
		pause.front = false;
		assert!(drawn(&root_menu(5.0), Some(&pause)).is_empty(), "the pause menu is not the title");
		let mut other = root_menu(5.0);
		other.page = Page::Options;
		assert!(drawn(&other, Some(&root)).is_empty(), "another page, even with the question flag set");
		assert!(drawn(&root_menu(0.0), Some(&root)).len() < drawn(&root_menu(5.0), Some(&root)).len());
		let mut out = Vec::new();
		draw(&mut out, WIN, &root_menu(5.0), &art(), &Text::default(), Some(&root));
		assert!(out.is_empty(), "the label is unknown: neither the entry nor the question");
		title_quit::set_ask(false);
		assert!(drawn(&root_menu(0.1), Some(&root)).is_empty(), "the retail panels are still flying in");
	}

	#[test]
	fn the_entry_is_a_panel_one_pitch_below_options_with_its_label_in_it() {
		let _w = world();
		let root = crate::retail::title_quit::test_root(2);
		let out = drawn(&root_menu(5.0), Some(&root));
		// No frame art: shadow, fill, then the label (a shadow and a colour quad for its one letter).
		assert_eq!(out.len(), 4);
		let p = title_quit::PANEL;
		assert!(out.iter().all(|q| inside(q, p, 3.0)), "everything inside the panel: {:?}", out.iter().map(|q| q.rect).collect::<Vec<_>>());
		assert_eq!(out[1].rect[..2], [p[0], p[1]], "the fill is the panel's rect");
		let c = centre(&out[3]);
		assert!(within(p, c), "the letter sits in the panel: {c:?}");
		assert!(out.iter().all(|q| q.colour[3] == 1.0 || q.colour[3] == PANEL[3]), "fully faded in: retail's own alphas");
		let options = root.items[2].rect;
		assert!(p[1] >= (options[1] + options[3]) as f32 && p[1] - options[1] as f32 > options[3] as f32 - 1.0, "below Options at the root pitch");
	}

	#[test]
	fn the_entry_is_lavender_until_highlighted_then_golden() {
		let _w = world();
		let root = crate::retail::title_quit::test_root(2);
		let plain = drawn(&root_menu(5.0), Some(&root));
		let mut lit = root_menu(5.0);
		lit.glow = vec![0.0, 0.0, 0.0, 1.0];
		let glowing = drawn(&lit, Some(&root));
		assert_ne!(plain[3].colour, glowing[3].colour, "the glow changes the label's colour");
		let gain = 255.0 / 128.0;
		for j in 0..3 {
			assert!((glowing[3].colour[j] - SELECTED[j] * gain).abs() < 1e-4, "{j}");
			assert!((plain[3].colour[j] - UNSELECTED[j] * gain).abs() < 1e-4, "{j}");
		}
	}

	#[test]
	fn the_entry_fades_with_one_alpha_over_all_its_quads() {
		let _w = world();
		let root = crate::retail::title_quit::test_root(2);
		let half = (MOVE_TICKS + FADE_TICKS / 2.0) / 50.0;
		let (full, mid) = (drawn(&root_menu(5.0), Some(&root)), drawn(&root_menu(half), Some(&root)));
		assert_eq!(full.len(), mid.len());
		for (a, b) in full.iter().zip(&mid) {
			assert!((b.colour[3] - a.colour[3] * 0.5).abs() < 1e-5, "{:?} vs {:?}", a.colour, b.colour);
			assert_eq!(a.rect, b.rect, "same places");
		}
	}

	#[test]
	fn the_question_dims_the_page_and_draws_the_box_with_the_answers_in_their_zones() {
		let _w = world();
		let root = crate::retail::title_quit::test_root(2);
		let without = drawn(&root_menu(5.0), Some(&root));
		title_quit::set_focus(true);
		title_quit::set_ask(true);
		let out = drawn(&root_menu(5.0), Some(&root));
		assert!(out.len() > without.len() + 8, "the entry stays, the box comes on top");
		assert_eq!(out[..without.len()].iter().map(|q| q.rect).collect::<Vec<_>>(), without.iter().map(|q| q.rect).collect::<Vec<_>>(), "the entry is drawn first");
		let dim = &out[without.len()];
		assert_eq!((dim.rect, dim.colour), ([0.0, 0.0, WIN.0, WIN.1], [0.0, 0.0, 0.0, DIM]), "the whole window dimmed, before the box");
		let boxed = &out[without.len() + 1..];
		assert!(boxed.iter().all(|q| inside(q, title_quit::BOX, 3.0)), "the box and its texts stay inside the box");
		// Shadow, fill, then header (2), body (2), No (2), Yes (2) for one-letter texts.
		assert_eq!(boxed.len(), 2 + 2 + 2 + 2 + 2);
		let (no, yes) = (centre(&boxed[boxed.len() - 3]), centre(&boxed[boxed.len() - 1]));
		assert!(within(title_quit::NO_ZONE, no), "No inside its zone: {no:?}");
		assert!(within(title_quit::YES_ZONE, yes), "Yes inside its zone: {yes:?}");
		let header = centre(&boxed[3]);
		assert!(within(TITLE, header), "the header sits in the bar: {header:?}");
		let body = centre(&boxed[5]);
		assert!(body.1 > TITLE[1] + TITLE[3] && body.1 < no.1, "the body sits between the bar and the answers: {body:?}");
	}

	#[test]
	fn a_long_body_wraps_into_the_box_in_every_language_length() {
		let _w = world();
		title_quit::set_ask(true);
		let root = crate::retail::title_quit::test_root(2);
		let mut t = texts();
		// The longest body (Italian, 60 bytes) at the test font's 8 units a letter is 480: wraps into several lines.
		t.insert(ID_QUITD_BODY, "Uscire dal gioco? I progressi non salvati andranno persi.".to_string());
		let mut out = Vec::new();
		draw(&mut out, WIN, &root_menu(5.0), &art(), &t, Some(&root));
		let start = out.iter().position(|q| q.colour == [0.0, 0.0, 0.0, DIM]).expect("the dim quad") + 1;
		let boxed = &out[start..];
		assert!(boxed.iter().all(|q| inside(q, title_quit::BOX, 3.0)), "everything stays in the box");
		// Shadow, fill, header (2) first; the answers (2 + 2) last; the body's lines between.
		let body = &boxed[4..boxed.len() - 4];
		assert!(body.len() >= 6, "the sentence wraps into several lines: {} quads", body.len());
		let [no, _] = answers();
		assert!(body.iter().all(|q| q.rect[1] + q.rect[3] <= no[1] + 3.0), "no body line runs into the answers");
		let ys: Vec<i32> = body.iter().map(|q| q.rect[1].round() as i32).collect();
		assert!(ys.iter().max() > ys.iter().min(), "the lines stack");
	}

	#[test]
	fn the_window_shape_scales_and_centres_the_quads_like_the_menu() {
		let _w = world();
		let root = crate::retail::title_quit::test_root(2);
		let mut wide = Vec::new();
		draw(&mut wide, (1792.0, 896.0), &root_menu(5.0), &art(), &texts(), Some(&root));
		let narrow = drawn(&root_menu(5.0), Some(&root));
		assert_eq!(wide.len(), narrow.len());
		// k = 2, side bars (1792 - 1024) / 2 = 384.
		for (a, b) in narrow.iter().zip(&wide) {
			assert!((b.rect[0] - (384.0 + 2.0 * a.rect[0])).abs() < 1e-3 && (b.rect[1] - 2.0 * a.rect[1]).abs() < 1e-3, "{:?} -> {:?}", a.rect, b.rect);
		}
	}
}
