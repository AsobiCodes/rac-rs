//! The generic PS2 table, for a pack without its own (R&C2): the 24 functions with today's defaults, LMB = Circle, MMB = Triangle,
//! no PC actions, no menu layer. Action ids are the old function names (`binds::ID`).

use super::types::*;
use winit::keyboard::KeyCode as K;

const fn k(c: K) -> [Option<Input>; 2] {
	[Some(Input::Key(c)), None]
}
const fn km(m: Mouse, c: K) -> [Option<Input>; 2] {
	[Some(Input::Mouse(m)), Some(Input::Key(c))]
}
const fn mk(c: K, m: Mouse) -> [Option<Input>; 2] {
	[Some(Input::Key(c)), Some(Input::Mouse(m))]
}
/// `pad`: the controller button (`binds::PHYS` index) of today's default, `NO_PAD` for the stick directions.
const NO_PAD: u8 = 255;
const fn a(id: &'static str, section: u8, output: Output, defaults: [Option<Input>; 2], pad: u8) -> ActionDef {
	let ok = pad != NO_PAD;
	ActionDef { id, section, layer: Layer::Game, output, defaults, keep_one: false, pad_ok: ok, pad_defaults: [if ok { Some(pad) } else { None }, None], pad_follows: 0 }
}

pub static ACTIONS: [ActionDef; 24] = [
	a("cross", 0, Output::Pad(0x40), k(K::Space), 0),
	a("square", 0, Output::Pad(0x80), k(K::KeyF), 2),
	a("triangle", 0, Output::Pad(0x10), mk(K::KeyT, Mouse::Middle), 3),
	a("circle", 0, Output::Pad(0x20), km(Mouse::Left, K::KeyG), 1),
	a("l1", 0, Output::Pad(4), k(K::KeyC), 4),
	a("r1", 0, Output::Pad(8), k(K::ControlLeft), 5),
	a("l2", 0, Output::Pad(1), k(K::KeyV), 14),
	a("r2", 0, Output::Pad(2), k(K::KeyB), 15),
	a("l3", 0, Output::Pad(0x200), k(K::ShiftLeft), 8),
	a("r3", 0, Output::Pad(0x400), k(K::KeyR), 9),
	ActionDef { id: "start", section: 0, layer: Layer::Game, output: Output::Pad(0x800), defaults: [Some(Input::Key(K::Escape)), Some(Input::Key(K::Enter))], keep_one: true, pad_ok: true, pad_defaults: [Some(6), None], pad_follows: 0 },
	a("select", 0, Output::Pad(0x100), k(K::KeyM), 7),
	a("up", 1, Output::Pad(0x1000), k(K::ArrowUp), 10),
	a("down", 1, Output::Pad(0x4000), k(K::ArrowDown), 11),
	a("left", 1, Output::Pad(0x8000), k(K::ArrowLeft), 12),
	a("right", 1, Output::Pad(0x2000), k(K::ArrowRight), 13),
	a("forward", 2, Output::Stick(3, -1), k(K::KeyW), NO_PAD),
	a("back", 2, Output::Stick(3, 1), k(K::KeyS), NO_PAD),
	a("strafe_left", 2, Output::Stick(2, -1), k(K::KeyA), NO_PAD),
	a("strafe_right", 2, Output::Stick(2, 1), k(K::KeyD), NO_PAD),
	a("camera_up", 3, Output::Stick(1, -1), k(K::KeyX), NO_PAD),
	a("camera_down", 3, Output::Stick(1, 1), k(K::KeyZ), NO_PAD),
	a("camera_left", 3, Output::Stick(0, -1), k(K::KeyE), NO_PAD),
	a("camera_right", 3, Output::Stick(0, 1), k(K::KeyQ), NO_PAD),
];

pub static TABLE: Table = Table { actions: &ACTIONS, menu: &[] };
