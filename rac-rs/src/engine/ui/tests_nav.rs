//! Tests of the `Ui` host: key repeat timing, pad edges, mouse hit tests and drags, peek, and the kit used without the options model.
use super::input::{Action, MouseButton, MouseEvent, Outcome, PadSample, Repeater, UiKey, P_CROSS, P_DOWN};
use super::kit::layout::{CtrlBox, ItemBox, THUMB_R};
use super::kit::{self, Chip, Control, Detail, FixedAdvance, Hit, Item, RowData, Screen};
use super::prim::{Prim, Rect};
use super::testkit::new_ui;
use super::theme;
use super::Ui;

fn click(ui: &mut Ui, x: f32, y: f32) -> Outcome {
	let o = ui.mouse(MouseEvent::Down { button: MouseButton::Left, x, y }, 0);
	ui.mouse(MouseEvent::Up { button: MouseButton::Left }, 0);
	o
}

fn click_rect(ui: &mut Ui, r: Rect) -> Outcome {
	click(ui, r.cx(), r.cy())
}

fn item(ui: &Ui, i: usize) -> ItemBox {
	ui.frame().0.items[i].clone()
}

#[test]
fn input_repeat_timing() {
	let mut ui = new_ui();
	ui.begin(false, Some("sys"));
	assert_eq!(ui.model.focus, 9);
	ui.key(UiKey::Down, true, 0);
	assert_eq!(ui.model.focus, 10, "the press acts at once");
	ui.key(UiKey::Down, true, 50);
	assert_eq!(ui.model.focus, 10, "the OS's own repeat is ignored");
	ui.tick(399);
	assert_eq!(ui.model.focus, 10);
	ui.tick(400);
	assert_eq!(ui.model.focus, 11, "first repeat after 400 ms");
	ui.tick(459);
	assert_eq!(ui.model.focus, 11);
	ui.tick(460);
	assert_eq!(ui.model.focus, 12, "then every 60 ms");
	ui.key(UiKey::Down, false, 470);
	ui.tick(2000);
	assert_eq!(ui.model.focus, 12, "release stops the repeat");
	ui.key(UiKey::Down, true, 3000);
	assert_eq!(ui.model.focus, 13, "and the key works again");
	ui.lose_focus();
	ui.tick(9000);
	assert_eq!(ui.model.focus, 13);
	// a long hitch must not run the focus across the list
	let mut r = Repeater::default();
	r.press(1, Action::Down, 0);
	assert_eq!(r.poll(1000).len(), 4);
	assert!(r.poll(1010).is_empty());
	r.press(2, Action::Confirm, 2000);
	assert!(!r.is_holding(), "only directions repeat");
}

#[test]
fn pad_buttons_act_once_and_directions_repeat() {
	let mut ui = new_ui();
	ui.begin(false, Some("sys"));
	let cross = PadSample { buttons: P_CROSS, ..PadSample::IDLE };
	ui.pad(cross, 0);
	assert_eq!(ui.model.value(9), 1, "Cross changes the focused row");
	ui.pad(cross, 10);
	assert_eq!(ui.model.value(9), 1, "a held button does not act again");
	ui.pad(PadSample::IDLE, 20);
	ui.pad(cross, 30);
	assert_eq!(ui.model.value(9), 0);
	ui.pad(PadSample { buttons: P_DOWN, ..PadSample::IDLE }, 100);
	assert_eq!(ui.model.focus, 10);
	ui.tick(500);
	assert_eq!(ui.model.focus, 11);
	ui.pad(PadSample::IDLE, 520);
	ui.tick(900);
	assert_eq!(ui.model.focus, 11);
	let o = ui.pad(PadSample { buttons: super::input::P_CIRCLE, ..PadSample::IDLE }, 1000);
	assert_eq!(o, Outcome::Close, "Circle is Back");
	assert!(ui.take_sounds().contains(&super::UiSound::Back));
}

/// No lag between the pad and the screen: a button already down when the screen opened (`hold_pad`) does not act, but the next
/// press, after a release, acts inside the very `pad` call that reports it (value and change are there when the call returns).
#[test]
fn ui_pad_same_frame_reaction() {
	let mut ui = new_ui();
	ui.begin(false, Some("sys"));
	let cross = PadSample { buttons: P_CROSS, ..PadSample::IDLE };
	ui.hold_pad(cross);
	ui.pad(cross, 0);
	ui.pad(cross, 10);
	assert_eq!(ui.model.value(9), 0, "the Cross that opened the screen does not Confirm the first row");
	assert!(ui.take_changes().is_empty());
	ui.pad(PadSample::IDLE, 20);
	assert_eq!(ui.model.value(9), 0, "the release acts on nothing");
	ui.pad(cross, 30);
	assert_eq!(ui.model.value(9), 1, "pressed again: acted in the same call");
	assert_eq!(ui.take_changes().len(), 1, "and the change is already queued for the game");
	ui.hold_pad(PadSample::IDLE);
	ui.pad(P_DOWN_SAMPLE, 40);
	assert_eq!(ui.model.focus, 10, "a direction after an idle hold moves in the same call");
}

const P_DOWN_SAMPLE: PadSample = PadSample { buttons: P_DOWN, ..PadSample::IDLE };

#[test]
fn mouse_hit_test_rows_tabs_chevrons_switch_slider_chips() {
	let mut ui = new_ui();
	// items with Lighting Classic: 0 banner, 1 section, 2 preset, 3 lighting, 4 shadows, 5 bloom, 6 section, 7 fov
	let tab1 = ui.frame().0.head.tabs[1].rect;
	click_rect(&mut ui, tab1);
	assert_eq!((ui.model.tab, ui.model.focus), (1, 5), "a tab click opens the tab");
	let next = ui.frame().0.head.tab_next.rect;
	click_rect(&mut ui, next);
	assert_eq!(ui.model.tab, 2);
	let prev = ui.frame().0.head.tab_prev.rect;
	click_rect(&mut ui, prev);
	click_rect(&mut ui, prev);
	assert_eq!(ui.model.tab, 0);
	// hover focuses the row, a click on the label only focuses
	let ItemBox::Row { label_hit, .. } = item(&ui, 5) else { panic!("row") };
	ui.mouse(MouseEvent::Move { x: label_hit.cx(), y: label_hit.cy() }, 0);
	ui.tick(0); // the hover is applied once per display frame
	assert_eq!(ui.model.focus, 3, "bloom");
	click_rect(&mut ui, label_hit);
	assert_eq!(ui.model.value(3), 1, "a click on the label does not change the value");
	// chevrons of the Lighting choice
	let ItemBox::Row { ctrl: CtrlBox::Choice { next, .. }, .. } = item(&ui, 3) else { panic!("choice") };
	click_rect(&mut ui, next);
	assert_eq!((ui.model.value(1), ui.model.focus), (1, 1));
	// Lighting is Enhanced now: the banner is gone, rows moved up by one item
	let ItemBox::Row { ctrl: CtrlBox::Choice { prev, .. }, .. } = item(&ui, 2) else { panic!("choice") };
	click_rect(&mut ui, prev);
	assert_eq!(ui.model.value(1), 0);
	// the switch of Bloom (item 5 again)
	let ItemBox::Row { ctrl: CtrlBox::Toggle { switch, .. }, .. } = item(&ui, 5) else { panic!("toggle") };
	click_rect(&mut ui, switch);
	assert_eq!((ui.model.value(3), ui.model.focus), (0, 3));
	// chips of the Preset row: High applies, the disabled Custom chip does nothing
	let ItemBox::Row { ctrl: CtrlBox::Chips(chips), .. } = item(&ui, 2) else { panic!("chips") };
	assert_eq!(chips.len(), 4);
	click_rect(&mut ui, chips[3]);
	assert_eq!(ui.model.preset_index(), None);
	click_rect(&mut ui, chips[2]);
	assert_eq!(ui.model.preset_index(), Some(2));
	// the banner is gone (Lighting is Enhanced); put it back, then use its button
	ui.model.set_value(1, 0);
	let ItemBox::Banner { button, .. } = item(&ui, 0) else { panic!("banner") };
	click_rect(&mut ui, button);
	assert_eq!(ui.model.value(1), 1);
	// the box button of the detail panel
	ui.model.set_value(1, 0);
	ui.model.focus_to(2);
	let b = ui.frame().0.detail.unwrap().boxed.unwrap().button;
	click_rect(&mut ui, b);
	assert_eq!(ui.model.value(1), 1);
	assert_eq!(ui.model.focus, 2, "enhancing keeps the focus");
	// hit kinds straight from the kit
	let (l, _) = ui.frame();
	assert_eq!(kit::hit(&l, false, 5.0, 5.0), Hit::None);
	assert_eq!(kit::hit(&l, false, l.viewport.cx(), l.viewport.bottom() - 1.0), Hit::List);
	// right button = Back
	let o = ui.mouse(MouseEvent::Down { button: MouseButton::Right, x: 1.0, y: 1.0 }, 0);
	assert_eq!(o, Outcome::Close);
}

#[test]
fn slider_drag_maps_x_to_value() {
	let mut ui = new_ui();
	ui.model.set_value(1, 1); // no banner: fov is item 6
	let ItemBox::Row { ctrl: CtrlBox::Slider { track, .. }, .. } = item(&ui, 6) else { panic!("slider") };
	let span = track.w - 2.0 * THUMB_R;
	let at = |t: f32| track.x + THUMB_R + t * span;
	let y = track.cy();
	ui.mouse(MouseEvent::Down { button: MouseButton::Left, x: at(0.0), y }, 0);
	assert_eq!((ui.model.value(4), ui.model.drag), (60, Some(4)));
	ui.mouse(MouseEvent::Move { x: at(0.5), y: y + 30.0 }, 0);
	ui.tick(0);
	assert_eq!(ui.model.value(4), 85, "the drag follows x, y does not matter");
	ui.mouse(MouseEvent::Move { x: track.right() + 200.0, y }, 0);
	ui.tick(0);
	assert_eq!(ui.model.value(4), 110, "clamped at the end");
	ui.mouse(MouseEvent::Up { button: MouseButton::Left }, 0);
	assert_eq!(ui.model.drag, None);
	ui.mouse(MouseEvent::Move { x: at(0.0), y }, 0);
	ui.tick(0);
	assert_eq!(ui.model.value(4), 110, "no drag, no change");
	assert!((kit::slider_t(&track, at(0.25)) - 0.25).abs() < 1e-5);
}

#[test]
fn wheel_scrolls_within_bounds() {
	let mut ui = new_ui();
	ui.begin(false, Some("sys"));
	ui.mouse(MouseEvent::Wheel { lines: -1.0 }, 0);
	assert_eq!(ui.model.scroll, 162.0, "one notch down = 3 rows");
	ui.mouse(MouseEvent::Wheel { lines: -50.0 }, 0);
	assert_eq!(ui.model.scroll, ui.frame().0.max_scroll);
	ui.mouse(MouseEvent::Wheel { lines: 50.0 }, 0);
	assert_eq!(ui.model.scroll, 0.0);
}

/// Hover rects of the page's rows (item index, row index, label rect).
fn row_hits(ui: &Ui) -> Vec<(usize, usize, Rect)> {
	let map = ui.screen().1;
	ui.frame().0.items.iter().enumerate().filter_map(|(i, b)| match b {
		ItemBox::Row { label_hit, .. } => map.row(i).map(|r| (i, r, *label_hit)),
		_ => None,
	}).collect()
}

/// A mouse move only queues the hover (a hover builds the screen and its layout; a 1000 Hz mouse sends a move every millisecond):
/// the moves of one frame leave one pending position, `tick` applies it once, and any other input applies it first so the order
/// of the events is the same as when every move was applied at once.
#[test]
fn mouse_moves_are_folded_into_one_hover_per_frame() {
	let mut ui = new_ui();
	ui.begin(false, None);
	let start = ui.model.focus;
	let rows: Vec<_> = row_hits(&ui).into_iter().filter(|r| r.1 != start).take(2).collect();
	assert_eq!(rows.len(), 2);
	let (a, b) = (rows[0], rows[1]);
	for i in 0..40 {
		let f = i as f32 / 40.0;
		ui.mouse(MouseEvent::Move { x: a.2.cx() + (b.2.cx() - a.2.cx()) * f, y: a.2.cy() + (b.2.cy() - a.2.cy()) * f }, 0);
	}
	ui.mouse(MouseEvent::Move { x: b.2.cx(), y: b.2.cy() }, 0);
	assert_eq!(ui.model.focus, start, "nothing is rebuilt per event: the focus waits for the frame");
	assert!(ui.hover_at.is_some());
	ui.tick(0);
	assert_eq!((ui.model.focus, ui.hover_at), (b.1, None), "the last position of the frame wins, applied once");
	ui.tick(0);
	assert_eq!(ui.model.focus, b.1);
	// Order: a move that came first moves the focus before a key, a click or a release is handled, as when it was applied at once.
	let after = |between: bool, then: &dyn Fn(&mut Ui)| {
		let mut ui = new_ui();
		ui.begin(false, None);
		ui.mouse(MouseEvent::Move { x: a.2.cx(), y: a.2.cy() }, 0);
		if between {
			ui.tick(0);
		}
		then(&mut ui);
		(ui.model.focus, ui.model.scroll, ui.hover_at)
	};
	let key = |ui: &mut Ui| {
		ui.key(UiKey::Down, true, 0);
	};
	let click_b = |ui: &mut Ui| {
		click(ui, b.2.cx(), b.2.cy());
	};
	let wheel = |ui: &mut Ui| {
		ui.mouse(MouseEvent::Wheel { lines: 1.0 }, 0);
	};
	for then in [&key as &dyn Fn(&mut Ui), &click_b, &wheel] {
		assert_eq!(after(false, then), after(true, then));
		assert_eq!(after(false, then).2, None, "the pending move is used up by the input after it");
	}
	// A window that loses focus drops a pending move.
	ui.mouse(MouseEvent::Move { x: a.2.cx(), y: a.2.cy() }, 0);
	ui.lose_focus();
	assert_eq!(ui.hover_at, None);
}

#[test]
fn peek_toggle_hides_panels() {
	let mut ui = new_ui();
	let full = ui.frame().1;
	assert!(full.iter().any(|p| matches!(p, Prim::ClipPush { .. })));
	ui.act(Action::Peek);
	assert!(ui.model.peek);
	let (l, peeked) = ui.frame();
	assert!(matches!(peeked[0], Prim::Backdrop { blur_px, .. } if blur_px == theme::BLUR_PEEK));
	assert!(peeked.len() < 12 && peeked.len() < full.len());
	assert!(!peeked.iter().any(|p| matches!(p, Prim::ClipPush { .. })), "no list while peeking");
	assert!(peeked.iter().any(|p| matches!(p, Prim::Text(t) if t.text == "Back to the menu")));
	assert!(!peeked.iter().any(|p| matches!(p, Prim::Text(t) if t.text == "Graphics" || t.text == "GRAPHICS")));
	// nothing but the way back works while peeking
	let (focus, v) = (ui.model.focus, ui.model.vals.clone());
	for a in [Action::Down, Action::Right, Action::Reset, Action::TabNext] {
		assert_eq!(ui.act(a), Outcome::Stay);
	}
	assert_eq!((ui.model.focus, &ui.model.vals), (focus, &v));
	assert_eq!(ui.act(Action::Back), Outcome::Stay, "Back only brings the panels back");
	assert!(!ui.model.peek);
	ui.act(Action::Peek);
	ui.act(Action::Confirm);
	assert!(!ui.model.peek);
	ui.act(Action::Peek);
	ui.take_sounds();
	click_rect(&mut ui, l.pill);
	assert!(!ui.model.peek, "the pill brings them back");
	assert!(ui.take_sounds().contains(&super::UiSound::Move), "a click peeks out with the tick the keys play (R1-6)");
	assert_eq!(ui.act(Action::Back), Outcome::Close);
}

/// R1-7: a focus written outside the current tab steps from the tab's first row instead of underflowing.
#[test]
fn focus_step_with_a_focus_outside_the_tab_stays_in_the_tab() {
	let mut ui = new_ui();
	ui.begin(false, Some("sys"));
	let r = ui.model.cur_rows();
	assert!(r.start > 0 && r.len() > 1);
	ui.model.focus = 0;
	ui.model.focus_step(1);
	assert_eq!(ui.model.focus, r.start + 1);
	ui.model.focus = usize::MAX;
	ui.model.focus_step(-1);
	assert!(r.contains(&ui.model.focus));
}

#[test]
fn kit_lays_out_a_screen_without_the_options_model() {
	let row = |label: &str, control| Item::Row(RowData { label: label.into(), dim: false, control });
	let s = Screen {
		title: "Test".into(),
		tabs: vec!["One".into(), "Two".into()],
		items: vec![
			Item::Section("Things".into()),
			row("Plain", Control::None),
			row("Pick", Control::Choice { value: "B".into(), index: 1, count: 3 }),
			row("Chips", Control::Chips(vec![Chip { text: "x".into(), selected: true, disabled: false }])),
		],
		focus: Some(2),
		detail: Some(Detail { title: "Pick".into(), body: "Text".into(), ..Detail::default() }),
		..Screen::default()
	};
	let (l, prims) = kit::frame(&s, &FixedAdvance(0.5));
	assert_eq!(l.items.len(), 4);
	assert!(matches!(prims[0], Prim::Backdrop { .. }));
	let r = l.items[1].rect();
	assert_eq!(kit::hit(&l, false, r.cx(), r.cy()), Hit::Row(1));
	assert!(matches!(l.items[2], ItemBox::Row { ctrl: CtrlBox::Choice { .. }, .. }));
	assert!(l.detail.is_some() && l.hints.is_empty());
}
