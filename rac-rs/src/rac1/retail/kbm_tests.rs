//! The equivalence oracle of the keyboard + mouse system (v0.5.1): v0.5.0's `KbBinds::pad`, `menu_pad` and ship-fight left-button
//! rules are kept here as a TEST-ONLY copy; with the R&C1 table, every single input and every pair of inputs must give the same
//! buttons and sticks as they did, in play, in each menu screen kind and in a ship fight, with the defaults and with converted
//! old `keys=` files.

use super::menu_keys::{self, Screen};
use crate::kbm_actions::{self, AIM, SHIP, TABLE, ZOOM_IN, ZOOM_OUT};
use rac_engine::input::binds::{KbBinds, BIT, DEFAULT_KEYS, KEYS, NK, NP};
use rac_engine::input::kbm::{self, Binds, Ctx, Input, Mouse, Wheel};
use winit::keyboard::KeyCode as K;

const STICK: [(usize, i32); 8] = [(3, -1), (3, 1), (2, -1), (2, 1), (1, -1), (1, 1), (0, -1), (0, 1)];

/// v0.5.0 `KbBinds::pad`.
fn legacy_pad(keys: &[[Option<K>; 3]; NK], held: &dyn Fn(K) -> bool, circle: bool, triangle: bool) -> (u16, [u8; 4]) {
	let on = |f: usize| keys[f].iter().flatten().any(|&k| held(k));
	let mut b = 0u16;
	for f in 0..NP {
		if on(f) {
			b |= BIT[f];
		}
	}
	if circle {
		b |= BIT[3];
	}
	if triangle {
		b |= BIT[2];
	}
	let mut d = [0i32; 4];
	for (i, (axis, dir)) in STICK.iter().enumerate() {
		if on(16 + i) {
			d[*axis] += dir;
		}
	}
	(b, d.map(|d| (0x80 + d * 0x7f).clamp(0, 0xff) as u8))
}

const MODERN: [K; 19] = [K::Space, K::Enter, K::NumpadEnter, K::Escape, K::Backspace, K::Tab, K::KeyQ, K::KeyE, K::KeyZ, K::KeyX, K::ArrowUp, K::ArrowDown, K::ArrowLeft, K::ArrowRight, K::KeyW, K::KeyA, K::KeyS, K::KeyD, K::Delete];

/// v0.5.0 `menu_pad`.
fn legacy_menu(keys: &[[Option<K>; 3]; NK], s: &Screen, held: &dyn Fn(K) -> bool) -> (u16, [u8; 4]) {
	use super::mouse_menu::{DOWN, L1, LEFT, R1, RIGHT, UP};
	let (mut b, _) = legacy_pad(keys, &|k| !MODERN.contains(&k) && held(k), false, false);
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
	let sticks = if s.sticks { legacy_pad(keys, &|k| !matches!(k, K::KeyQ | K::KeyE) && held(k), false, false).1 } else { [0x80; 4] };
	(b, sticks)
}

fn inputs() -> Vec<Input> {
	let mut v: Vec<Input> = KEYS.iter().map(|e| Input::Key(e.0)).collect();
	v.extend([Input::Mouse(Mouse::Left), Input::Mouse(Mouse::Middle)]);
	v
}

fn sets() -> Vec<Vec<Input>> {
	let ins = inputs();
	let mut v = vec![vec![]];
	for (n, &a) in ins.iter().enumerate() {
		v.push(vec![a]);
		for &c in &ins[n + 1..] {
			v.push(vec![a, c]);
		}
	}
	v
}

fn screens() -> Vec<Screen> {
	vec![menu_keys::DEFAULT, menu_keys::VENDOR, Screen { id: 1, confirm: 0x20, back: 0x10, alt: 0x80, sticks: true }, Screen { id: 2, confirm: 0x40, back: 0x20, alt: 0, sticks: false }]
}

/// Binds + the old key arrays for a `keys=` text (None = the defaults).
fn setup(text: Option<&str>) -> (Binds, [[Option<K>; 3]; NK]) {
	kbm_actions::install();
	match text {
		None => (Binds::defaults(&TABLE), DEFAULT_KEYS),
		Some(t) => (kbm::file::convert_old(&TABLE, t), KbBinds::from_text(t).0),
	}
}

fn check(text: Option<&str>) {
	let (b, old) = setup(text);
	let old = old.map(|f| f.map(|k| if k == Some(K::NumpadEnter) { Some(K::Enter) } else { k }));
	for set in sets() {
		let held_in = |i: Input| set.contains(&i);
		let held_k = |k: K| set.contains(&Input::Key(k));
		// Designed difference (V051_KBM_DESIGN.md): Numpad Enter counts as Enter, so the old side folds it too.
		let held_o = |k: K| held_k(k) || (k == K::Enter && held_k(K::NumpadEnter));
		let (lmb, mmb) = (held_in(Input::Mouse(Mouse::Left)), held_in(Input::Mouse(Mouse::Middle)));
		// Play.
		let o = kbm::eval(&TABLE, &b, &held_in, &Ctx::default());
		assert_eq!((o.buttons, o.sticks), legacy_pad(&old, &held_o, lmb, mmb), "play {set:?} {text:?}");
		// A ship fight: the left button is Cross, not Circle.
		let o = kbm::eval(&TABLE, &b, &held_in, &Ctx { contexts: 1 << SHIP, menu: None });
		let (pb, ps) = legacy_pad(&old, &held_o, false, mmb);
		assert_eq!((o.buttons, o.sticks), (pb | if lmb { 0x40 } else { 0 }, ps), "ship {set:?} {text:?}");
		// Menus (keys only: the mouse is the menu's own).
		let keys_only = |i: Input| matches!(i, Input::Key(k) if held_k(k));
		for s in screens() {
			assert_eq!(menu_keys::menu_pad_in(&b, &s, &held_k), legacy_menu(&old, &s, &held_o), "menu {s:?} {set:?} {text:?}");
			let _ = keys_only;
		}
	}
}

#[test]
fn defaults_equal_v050_in_play_ship_and_every_menu_screen() {
	check(None);
}

#[test]
fn converted_old_files_equal_v050() {
	for t in ["cross:KeyK", "circle:KeyH,triangle:Digit5", "start:KeyP,select:Enter", "forward:KeyI,back:KeyK,strafe_left:KeyJ,strafe_right:KeyL,camera_up:KeyU,camera_left:KeyY", "cross:KeyG,l1:KeyQ,r1:ArrowUp,up:KeyW"] {
		check(Some(t));
	}
}

#[test]
fn the_table_is_sound() {
	assert!(TABLE.actions.len() <= kbm::MAX_ACTIONS);
	assert_eq!(TABLE.actions.len(), kbm_actions::LABELS.len());
	for (n, a) in TABLE.actions.iter().enumerate() {
		assert_eq!(TABLE.index(a.id), Some(n), "unique id {}", a.id);
		assert!((a.section as usize) < kbm_actions::SECTIONS);
	}
	// All 24 pad functions have a Game action.
	for f in 0..NK {
		assert!(kbm::file::func_action(&TABLE, f).is_some(), "function {f}");
	}
	let pause = &TABLE.actions[TABLE.index("pause").unwrap()];
	assert!(pause.keep_one);
}

#[test]
fn aim_and_zoom_are_pc_actions_on_the_old_mouse_inputs() {
	let b = setup(None).0;
	let pc = |i: Input| kbm::eval(&TABLE, &b, &|x| x == i, &Ctx::default()).pc;
	assert_eq!(pc(Input::Mouse(Mouse::Right)), 1 << AIM);
	assert_eq!(pc(Input::Wheel(Wheel::Up)), 1 << ZOOM_IN);
	assert_eq!(pc(Input::Wheel(Wheel::Down)), 1 << ZOOM_OUT);
	assert_eq!(b.pc_actions(&TABLE, Input::Mouse(Mouse::Right)).collect::<Vec<_>>(), vec![AIM]);
}

#[test]
fn a_mouse_side_button_can_be_the_wrench() {
	let (mut b, _) = setup(None);
	b.assign(TABLE.index("wrench").unwrap(), 1, Input::Mouse(Mouse::Back));
	let o = kbm::eval(&TABLE, &b, &|i| i == Input::Mouse(Mouse::Back), &Ctx::default());
	assert_eq!(o.buttons, 0x80);
	let text = kbm::file::to_text(&TABLE, &b).unwrap();
	assert_eq!(text, "wrench:KeyF|MouseBack");
}

#[test]
fn the_old_page_adapter_edits_the_key_slot_and_keeps_the_mouse() {
	use rac_engine::input::binds as ob;
	// It rewrites the process-wide bindings: without the guard it raced the Controls page tests (a flaky `slot_one_and_slot_two_capture`).
	let _g = crate::retail::pcopts::globals_guard();
	kbm_actions::install();
	ob::init(None, None);
	assert_eq!(ob::kb(), KbBinds::default());
	ob::assign_key(ob::CIRCLE, K::KeyH);
	let b = kbm::binds();
	let fire = TABLE.index("fire").unwrap();
	assert_eq!(b.0[fire], [Some(Input::Mouse(Mouse::Left)), Some(Input::Key(K::KeyH))], "the left button stays on Fire");
	ob::init(None, None);
}

#[test]
fn a_converted_old_file_puts_the_key_first_and_keeps_the_mouse_button_after_it() {
	kbm_actions::install();
	let b = kbm::file::convert_old(&TABLE, "circle:KeyH,triangle:Digit5");
	let (fire, qs) = (TABLE.index("fire").unwrap(), TABLE.index("quick_select").unwrap());
	assert_eq!(b.0[fire], [Some(Input::Key(K::KeyH)), Some(Input::Mouse(Mouse::Left))]);
	assert_eq!(b.0[qs], [Some(Input::Key(K::Digit5)), Some(Input::Mouse(Mouse::Middle))]);
}
