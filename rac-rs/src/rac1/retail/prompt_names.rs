//! The key names the button prompts show, read from the live bindings (`kbm`) and the R&C1 action table: a glyph's name is the
//! first bound input (slot 1, else slot 2) of the action that drives that pad button, so a rebound key, a mouse button ("LMB",
//! "Mouse 4", "Wheel up") and the keyboard layout (`kbm::names`) all reach the tutorial boxes, help texts and legends. Under the
//! default table the names are the ones the port always showed ("Space", "LMB", "T", "F", "C", "Ctrl", "V", "B", "WASD", "Q/E/Z/X").

use crate::kbm_actions as ka;
use rac_engine::input::kbm::{self, names, Input, Output};
use winit::keyboard::KeyCode;

/// What a prompt says for an action with no binding at all (the Controls page keeps Pause bound, but any other action can be empty).
pub const NONE: &str = "None";

fn action(id: &str) -> Option<usize> {
	ka::install();
	kbm::table().index(id)
}

/// The prompt name of action `a`: slot 1, else slot 2, else `NONE`.
pub fn action_name(a: usize) -> String {
	let b = kbm::binds();
	b.0.get(a).and_then(|s| s.iter().flatten().next().copied()).map_or_else(|| NONE.to_string(), names::short_name)
}

/// The name of the button glyph whose pad bit is `bit` (Cross 0x40, Circle 0x20, ...): the first Game-layer action that sets it.
pub fn glyph_name(bit: u16) -> Option<String> {
	ka::install();
	kbm::table().game_action(Output::Pad(bit)).map(action_name)
}

/// The name of a stick glyph. Under the default keys: "WASD" (forward, left, back, right, no separators) and "Q/E/Z/X" (the right
/// stick's keys in the order the manual lists them); rebound: the four names joined by "/" in the order forward, left, back, right
/// ("I/J/S/L"); the right stick likewise up, left, down, right.
pub fn stick_name(left: bool) -> String {
	let ids = if left { ["forward", "strafe_left", "back", "strafe_right"] } else { ["camera_up", "camera_left", "camera_down", "camera_right"] };
	let acts: Vec<usize> = ids.iter().filter_map(|i| action(i)).collect();
	if acts.len() != 4 {
		return NONE.to_string();
	}
	let (t, b) = (kbm::table(), kbm::binds());
	let defaults = kbm::Binds::defaults(t);
	let default = acts.iter().all(|&a| b.0[a] == defaults.0[a]);
	let join = |order: [usize; 4], sep: &str| order.iter().map(|&a| action_name(a)).collect::<Vec<_>>().join(sep);
	match (default, left) {
		(true, true) => join([acts[0], acts[1], acts[2], acts[3]], ""),
		(true, false) => join([acts[3], acts[1], acts[2], acts[0]], "/"),
		_ => join([acts[0], acts[1], acts[2], acts[3]], "/"),
	}
}

/// Whether Jump's slot 1 is Space (the short "Spc" / "Sp" legend rule of `prompts::substitute_text` applies only then).
pub fn jump_is_space() -> bool {
	action("jump").is_some_and(|a| kbm::binds().0[a][0] == Some(Input::Key(KeyCode::Space)))
}
