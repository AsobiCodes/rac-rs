//! Layout snapshots: the window mapping at four window shapes and the draw list of the fake menu at 1280x720 against a checked-in
//! text file (`layout_snapshot.txt`; if it is missing the test writes it and fails once; if it differs the new text goes to
//! `layout_snapshot.new.txt` next to it: copy it over after looking at the picture).
use super::input::{MouseButton, MouseEvent};
use super::kit::Scale;
use super::prim::{dump, Rect};
use super::testkit::new_ui;
use super::theme;

const LIST: Rect = Rect::new(48.0, 116.0, 760.0, 540.0);
const DETAIL: Rect = Rect::new(832.0, 116.0, 400.0, 540.0);

fn check(w: u32, h: u32, s: f32, ox: f32, oy: f32, list: Rect, detail: Rect, hint_y: f32) {
	let sc = Scale::fit(w, h);
	assert_eq!((sc.s, sc.ox, sc.oy), (s, ox, oy), "{w}x{h}");
	assert_eq!(sc.to_window(LIST), list, "{w}x{h} list");
	assert_eq!(sc.to_window(DETAIL), detail, "{w}x{h} detail");
	let mut ui = new_ui();
	ui.set_window(w, h);
	let (l, _) = ui.frame();
	assert!(!l.hints.is_empty());
	for hint in &l.hints {
		assert_eq!(sc.to_window(Rect::new(0.0, hint.text.y, 1.0, 0.0)).y, hint_y, "{w}x{h} hint bar centre");
		assert!(hint.rect.x >= 0.0 && hint.rect.right() <= theme::DESIGN_W, "{w}x{h} hint inside the safe area");
	}
	let safe = sc.safe_area();
	assert_eq!((safe.w, safe.h), (1280.0 * s, 720.0 * s));
	assert_eq!(sc.to_design(ox + 100.0 * s, oy + 50.0 * s), (100.0, 50.0));
}

#[test]
fn layout_snapshot_1280x720() {
	check(1280, 720, 1.0, 0.0, 0.0, Rect::new(48.0, 116.0, 760.0, 540.0), Rect::new(832.0, 116.0, 400.0, 540.0), 688.0);
}

#[test]
fn layout_snapshot_1280x800_steam_deck() {
	check(1280, 800, 1.0, 0.0, 40.0, Rect::new(48.0, 156.0, 760.0, 540.0), Rect::new(832.0, 156.0, 400.0, 540.0), 728.0);
}

#[test]
fn layout_snapshot_1920x1080() {
	check(1920, 1080, 1.5, 0.0, 0.0, Rect::new(72.0, 174.0, 1140.0, 810.0), Rect::new(1248.0, 174.0, 600.0, 810.0), 1032.0);
}

#[test]
fn layout_snapshot_3440x1440_21x9() {
	check(3440, 1440, 2.0, 440.0, 0.0, Rect::new(536.0, 232.0, 1520.0, 1080.0), Rect::new(2104.0, 232.0, 800.0, 1080.0), 1376.0);
}

#[test]
fn layout_snapshot_draw_list_1280x720() {
	let mut ui = new_ui();
	ui.set_window(1280, 720);
	let actual = dump(&ui.frame().1);
	let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/engine/ui/");
	let want = std::fs::read_to_string(format!("{dir}layout_snapshot.txt")).map(|s| s.replace("\r\n", "\n"));
	match want {
		Err(_) => {
			std::fs::write(format!("{dir}layout_snapshot.txt"), &actual).unwrap();
			panic!("layout_snapshot.txt was missing: written, run the test again");
		}
		Ok(w) if w == actual => {}
		Ok(w) => {
			std::fs::write(format!("{dir}layout_snapshot.new.txt"), &actual).unwrap();
			let n = w.lines().zip(actual.lines()).position(|(a, b)| a != b).unwrap_or(w.lines().count().min(actual.lines().count()));
			panic!("draw list differs from layout_snapshot.txt at line {}: want {:?}, got {:?}", n + 1, w.lines().nth(n), actual.lines().nth(n));
		}
	}
}

#[test]
fn mouse_maps_window_pixels_through_the_scale() {
	let mut ui = new_ui();
	for (w, h) in [(1280, 720), (1280, 800), (1920, 1080), (3440, 1440)] {
		ui.set_window(w, h);
		ui.model.set_tab(0);
		let tab = ui.frame().0.head.tabs[2].rect;
		let r = ui.scale.to_window(tab);
		ui.mouse(MouseEvent::Down { button: MouseButton::Left, x: r.cx(), y: r.cy() }, 0);
		ui.mouse(MouseEvent::Up { button: MouseButton::Left }, 0);
		assert_eq!(ui.model.tab, 2, "{w}x{h}");
		// a click in the letterbox band hits nothing
		ui.model.set_tab(0);
		ui.mouse(MouseEvent::Down { button: MouseButton::Left, x: 2.0, y: 2.0 }, 0);
		assert_eq!(ui.model.tab, 0, "{w}x{h} band");
	}
}
