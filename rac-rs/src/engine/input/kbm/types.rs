//! The vocabulary of the keyboard + mouse system: inputs, outputs, layers and the tables a game pack hands over.

use winit::keyboard::KeyCode as K;

/// A mouse button (the five a window reports).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Mouse {
	Left,
	Right,
	Middle,
	Back,
	Forward,
}

/// A wheel notch.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Wheel {
	Up,
	Down,
}

/// One physical input a slot can hold.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Input {
	Key(K),
	Mouse(Mouse),
	Wheel(Wheel),
}

impl Input {
	/// The input as stored and matched: Numpad Enter counts as Enter (one input, either key).
	pub fn norm(self) -> Input {
		match self {
			Input::Key(K::NumpadEnter) => Input::Key(K::Enter),
			i => i,
		}
	}

	/// Whether the host's `held` says this input is down (Enter is down when either Enter key is).
	pub fn is_down(self, held: &dyn Fn(Input) -> bool) -> bool {
		match self.norm() {
			Input::Key(K::Enter) => held(Input::Key(K::Enter)) || held(Input::Key(K::NumpadEnter)),
			i => held(i),
		}
	}
}

/// What an action does when one of its inputs is down.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Output {
	/// Sets these bits of the PS2 pad word.
	Pad(u16),
	/// Moves a stick: `axis` indexes `[rx, ry, lx, ly]`, `dir` is -1 or 1 (the byte moves from 0x80 by 0x7f times it).
	Stick(u8, i8),
	/// A PC action of the game pack (an id of its own; bit `1 << id` of `Out::pc`).
	Pc(u8),
}

/// The layer of an action. Evaluation runs from the top: Menu, the active Context layers, then Game; an input bound in an
/// active higher layer is consumed there and the lower layers do not see it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
	Game,
	/// A pack-defined condition `n` (< 32), e.g. R&C1's ship fight; active when bit `n` of `Ctx::contexts` is set.
	Context(u8),
}

/// One bindable action: a row of the Controls page.
#[derive(Clone, Copy, Debug)]
pub struct ActionDef {
	/// Name in the options file (`kbm=`).
	pub id: &'static str,
	/// Section of the Controls page (pack-defined numbering).
	pub section: u8,
	pub layer: Layer,
	pub output: Output,
	/// The two slots a fresh install has.
	pub defaults: [Option<Input>; 2],
	/// May not lose its last binding (Pause: the pause menu can always be opened). Applies to each device on its own.
	pub keep_one: bool,
	/// The controller may bind it (v0.5.1 pad lane): not the stick outputs, not keyboard-only PC actions, not Context-layer actions.
	pub pad_ok: bool,
	/// The two controller slots a fresh install has (`PadIn`: an index into `binds::PHYS`).
	pub pad_defaults: [Option<PadIn>; 2],
	/// Non-zero: the PS2 pad bit of the old controller function this action's slot 1 follows when an old `pad=` line is converted
	/// (R&C1's Aim followed "the pad's R2": 2); 0 = none.
	pub pad_follows: u16,
}

/// One controller button a slot can hold: an index into `crate::input::binds::PHYS` (0..16).
pub type PadIn = u8;

/// What a fixed menu key does (not bindable).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuOut {
	/// Only shadows the input from the game layer.
	None,
	Pad(u16),
	/// The open screen's confirm / back / alternate button (`MenuCtx`).
	Confirm,
	Back,
	Alt,
}

/// A key of the Menu layer.
#[derive(Clone, Copy, Debug)]
pub struct MenuKey {
	pub input: Input,
	pub out: MenuOut,
	/// Does nothing on a screen that takes the keys as a stick (the Galactic Map: WASD is the stick there).
	pub unless_sticks: bool,
	/// Also removed from the game layer's STICKS (Q / E: menu L1 / R1 while the sticks are on).
	pub stick_shadow: bool,
}

/// The open menu screen: its buttons and whether the keys steer a stick.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MenuCtx {
	pub confirm: u16,
	pub back: u16,
	pub alt: u16,
	pub sticks: bool,
}

/// What a game pack hands over: the actions (their order is the page's order within a section) and the Menu layer.
#[derive(Clone, Copy, Debug)]
pub struct Table {
	pub actions: &'static [ActionDef],
	pub menu: &'static [MenuKey],
}

/// The most actions a table may have (`Binds` is a fixed array so it stays `Copy`).
pub const MAX_ACTIONS: usize = 40;

/// The controller buttons a slot can name (`binds::PHYS`).
pub const PAD_BUTTONS: usize = 16;

impl Table {
	pub fn index(&self, id: &str) -> Option<usize> {
		self.actions.iter().position(|a| a.id.eq_ignore_ascii_case(id.trim()))
	}

	/// The first Game-layer action with this output (the action that drives PS2 function `output`).
	pub fn game_action(&self, output: Output) -> Option<usize> {
		self.actions.iter().position(|a| a.layer == Layer::Game && a.output == output)
	}
}
