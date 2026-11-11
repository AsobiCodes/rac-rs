use super::generic::TABLE;
use super::names::*;
use super::*;
use crate::input::binds::{KbBinds, KEYS};
use winit::keyboard::KeyCode as K;

fn all_inputs() -> Vec<Input> {
	let mut v: Vec<Input> = KEYS.iter().map(|e| Input::Key(e.0)).collect();
	v.extend([Input::Mouse(Mouse::Left), Input::Mouse(Mouse::Middle)]);
	v
}

/// `KbBinds::pad` (today's mapping, kept as the oracle) against `eval` with the generic table, for a held set.
fn same(old: &KbBinds, b: &Binds, set: &[Input]) -> bool {
	let held = |i: Input| set.contains(&i);
	let (circle, tri) = (held(Input::Mouse(Mouse::Left)), held(Input::Mouse(Mouse::Middle)));
	let want = old.pad(|k| held(Input::Key(k)), circle, tri);
	let got = eval(&TABLE, b, &held, &Ctx::default());
	(got.buttons, got.sticks) == want
}

fn check_all(old: &KbBinds, b: &Binds) {
	let ins = all_inputs();
	for (n, &a) in ins.iter().enumerate() {
		assert!(same(old, b, &[a]), "single {a:?}");
		for &c in &ins[n + 1..] {
			assert!(same(old, b, &[a, c]), "pair {a:?} {c:?}");
		}
	}
	assert!(same(old, b, &[]));
}

#[test]
fn defaults_equal_todays_mapping_for_every_input_and_pair() {
	check_all(&KbBinds::default(), &Binds::defaults(&TABLE));
}

#[test]
fn converted_old_files_equal_the_old_mapping() {
	for text in ["cross:KeyK", "circle:KeyH,triangle:Digit5", "start:KeyP", "forward:KeyI,back:KeyK,strafe_left:KeyJ,strafe_right:KeyL,camera_up:KeyU", "cross:KeyG,circle:Space,l1:KeyG"] {
		let (old, new) = (KbBinds::from_text(text), file::convert_old(&TABLE, text));
		// The old mouse buttons stay on Circle / Triangle (their defaults' mouse slots), which old files could not move.
		check_all(&old, &new);
	}
}

#[test]
fn defaults_write_nothing_and_changes_round_trip() {
	let d = Binds::defaults(&TABLE);
	assert_eq!(file::to_text(&TABLE, &d), None);
	let mut b = d;
	b.assign(3, 0, Input::Mouse(Mouse::Back));
	b.assign(1, 1, Input::Key(K::KeyH));
	b.assign(0, 0, Input::Wheel(Wheel::Down));
	assert!(b.clear(&TABLE, 5, 0));
	let t = file::to_text(&TABLE, &b).unwrap();
	assert!(t.contains("circle:MouseBack|KeyG") && t.contains("square:KeyF|KeyH") && t.contains("cross:WheelDown|-") && t.contains("r1:-|-"), "{t}");
	assert_eq!(file::from_text(&TABLE, &t), b);
}

#[test]
fn unreadable_entries_are_skipped() {
	let b = file::from_text(&TABLE, "nonsense,cross:Nope|KeyK,circle:MouseLeft|KeyZ|KeyQ,l1:KeyP,:x,square");
	let mut want = Binds::defaults(&TABLE);
	want.assign(4, 0, Input::Key(K::KeyP));
	assert_eq!(b, want);
}

#[test]
fn pause_is_never_empty() {
	let mut b = Binds::defaults(&TABLE);
	let a = TABLE.index("start").unwrap();
	assert!(b.clear(&TABLE, a, 1));
	assert!(!b.clear(&TABLE, a, 0), "the last binding stays");
	assert_eq!(b.0[a], [Some(Input::Key(K::Escape)), None]);
	assert_eq!(file::from_text(&TABLE, "start:-|-"), Binds::defaults(&TABLE), "an empty Pause in a file is ignored");
}

#[test]
fn a_shared_input_is_listed_and_nothing_else_moves() {
	let mut b = Binds::defaults(&TABLE);
	let before = b;
	b.assign(0, 0, Input::Key(K::KeyG)); // Cross on G, which Circle has in slot 2
	assert_eq!(b.shared(&TABLE, 0), vec![3]);
	assert_eq!(b.shared(&TABLE, 3), vec![0]);
	for a in 1..MAX_ACTIONS {
		assert_eq!(b.0[a], before.0[a]);
	}
	let o = eval(&TABLE, &b, &|i| i == Input::Key(K::KeyG), &Ctx::default());
	assert_eq!(o.buttons, 0x40 | 0x20, "both actions fire");
}

#[test]
fn numpad_enter_is_enter() {
	let mut b = Binds::defaults(&TABLE);
	let start = |b: &Binds, k: K| eval(&TABLE, b, &|i| i == Input::Key(k), &Ctx::default()).buttons & 0x800 != 0;
	assert!(start(&b, K::Enter) && start(&b, K::NumpadEnter) && start(&b, K::Escape));
	b.assign(0, 0, Input::Key(K::NumpadEnter));
	assert_eq!(b.0[0][0], Some(Input::Key(K::Enter)), "stored as Enter");
	assert!(eval(&TABLE, &b, &|i| i == Input::Key(K::NumpadEnter), &Ctx::default()).buttons & 0x40 != 0);
	assert_eq!(from_name("NumpadEnter"), Some(Input::Key(K::Enter)));
}

#[test]
fn a_higher_layer_consumes_its_input() {
	use super::types::{ActionDef, Layer, Output, Table};
	static A: [ActionDef; 2] = [
		ActionDef { id: "fire", section: 0, layer: Layer::Game, output: Output::Pad(0x20), defaults: [Some(Input::Mouse(Mouse::Left)), Some(Input::Key(K::KeyG))], keep_one: false, pad_ok: false, pad_defaults: [None; 2], pad_follows: 0 },
		ActionDef { id: "ship", section: 0, layer: Layer::Context(0), output: Output::Pad(0x40), defaults: [Some(Input::Mouse(Mouse::Left)), None], keep_one: false, pad_ok: false, pad_defaults: [None; 2], pad_follows: 0 },
	];
	static T: Table = Table { actions: &A, menu: &[] };
	let b = Binds::defaults(&T);
	let lmb = |i: Input| i == Input::Mouse(Mouse::Left);
	let g = |i: Input| i == Input::Key(K::KeyG);
	assert_eq!(eval(&T, &b, &lmb, &Ctx::default()).buttons, 0x20);
	assert_eq!(eval(&T, &b, &lmb, &Ctx { contexts: 1, menu: None }).buttons, 0x40, "the ship layer takes the button");
	assert_eq!(eval(&T, &b, &g, &Ctx { contexts: 1, menu: None }).buttons, 0x20, "G still fires");
}

#[test]
fn capture_binds_the_first_capturable_input_and_ends() {
	let _lock = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	init(None, None, None, None);
	capture_begin(Cap::Kbm(0, 1));
	assert!(capturing());
	capture_poll(&[], 0);
	assert!(capturing(), "nothing pressed: keeps waiting");
	capture_poll(&[Input::Mouse(Mouse::Back)], 0);
	assert!(!capturing() && capture_ended());
	assert_eq!(binds().0[0], [Some(Input::Key(K::Space)), Some(Input::Mouse(Mouse::Back))]);
	capture_begin(Cap::Kbm(1, 0));
	capture_poll(&[Input::Wheel(Wheel::Up)], 0);
	assert_eq!(binds().0[1][0], Some(Input::Wheel(Wheel::Up)));
	assert!(take_changes().unwrap().0.unwrap().contains("cross:Space|MouseBack"));
	// Esc cancels, nothing changes; a key that cannot be bound is skipped.
	capture_begin(Cap::Kbm(2, 0));
	capture_poll(&[Input::Key(K::Escape), Input::Key(K::KeyK)], 0);
	assert!(!capturing());
	assert_ne!(binds().0[2][0], Some(Input::Key(K::KeyK)));
	capture_begin(Cap::Kbm(2, 0));
	capture_poll(&[Input::Key(K::F5), Input::Key(K::KeyK)], 0);
	assert_eq!(binds().0[2][0], Some(Input::Key(K::KeyK)), "F5 is skipped");
	// A timeout ends it.
	capture_begin(Cap::Kbm(2, 1));
	for _ in 0..=CAP_FIELDS {
		capture_poll(&[], 0);
	}
	assert!(!capturing() && binds().0[2][1] == t_default(2, 1));
}

fn t_default(a: usize, s: usize) -> Option<Input> {
	Binds::defaults(&TABLE).0[a][s]
}

#[test]
fn keys_line_is_converted_on_init_and_kbm_wins() {
	let _lock = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	init(None, Some("cross:KeyK"), None, None);
	assert_eq!(binds().0[0][0], Some(Input::Key(K::KeyK)));
	init(Some("cross:KeyJ|-"), Some("cross:KeyK"), None, None);
	assert_eq!(binds().0[0], [Some(Input::Key(K::KeyJ)), None]);
	init(None, None, None, None);
	assert_eq!(binds(), Binds::defaults(&TABLE));
}

#[test]
fn names_round_trip_and_are_short() {
	for i in all_inputs().into_iter().chain([Input::Mouse(Mouse::Right), Input::Mouse(Mouse::Back), Input::Mouse(Mouse::Forward), Input::Wheel(Wheel::Up), Input::Wheel(Wheel::Down)]) {
		let n = file_name(i).unwrap();
		assert_eq!(from_name(n), Some(i.norm()), "{n}");
		assert!(short_name(i).len() <= 10 && short_name(i).is_ascii());
	}
	assert_eq!(short_name(Input::Mouse(Mouse::Back)), "Mouse 4");
	assert_eq!(short_name(Input::Mouse(Mouse::Left)), "LMB");
	let names: Vec<_> = [Mouse::Middle, Mouse::Right, Mouse::Forward].map(|m| short_name(Input::Mouse(m))).into();
	assert_eq!(names, ["MMB", "RMB", "Mouse 5"]);
	assert_eq!((short_name(Input::Wheel(Wheel::Up)), short_name(Input::Wheel(Wheel::Down))), ("Wheel up".to_string(), "Wheel down".to_string()));
	assert_eq!((short_name(Input::Key(K::Space)), short_name(Input::Key(K::KeyG))), ("Space".to_string(), "G".to_string()), "QWERTY names until the host reads a layout");
}

/// An AZERTY keyboard prints A where QWERTY has Q: the cap follows the physical key (by scan code), other keys keep their names.
#[test]
fn a_key_cap_follows_the_keyboard_layout() {
	crate::ui::keycaps::set_layout(vec![(K::KeyQ, 'A'), (K::KeyW, 'Z'), (K::Digit1, '1')]);
	let n = (short_name(Input::Key(K::KeyQ)), short_name(Input::Key(K::KeyW)), short_name(Input::Key(K::KeyE)), short_name(Input::Key(K::Space)));
	crate::ui::keycaps::set_layout(Vec::new());
	assert_eq!(n, ("A".to_string(), "Z".to_string(), "E".to_string(), "Space".to_string()));
	assert_eq!(short_name(Input::Key(K::KeyQ)), "Q");
}

#[test]
fn legacy_view_of_the_defaults_is_the_old_defaults() {
	assert_eq!(file::legacy_view(&TABLE, &Binds::defaults(&TABLE)), KbBinds::default());
}

#[test]
fn a_pad_capture_ignores_the_press_that_opened_it() {
	let _lock = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	init(None, None, None, None);
	capture_begin(Cap::Pad(0, 0));
	capture_poll(&[], crate::input::binds::PHYS[3].1);
	assert!(capturing(), "the opening press is skipped");
	capture_poll(&[], crate::input::binds::PHYS[3].1);
	assert!(!capturing(), "the next poll with a button binds");
}
