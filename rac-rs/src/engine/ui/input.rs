//! Input of the modern UI, shared by every screen built on the kit: keyboard / pad / mouse events become `Action`s at DISPLAY
//! rate (not field rate), with the UI's own auto-repeat (400 ms delay, 60 ms interval, driven by a clock the caller passes in, so
//! tests use a fake one). The host maps winit keys and the pad report to the engine-neutral `UiKey` / `PadSample`; nothing here
//! names a window system or a game.
use super::prim::{GlyphKind, Symbol};

/// What the user asked for, whichever device it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
	Up,
	Down,
	Left,
	Right,
	/// Enter / Cross: change the focused row (Choice: next value, Toggle: flip, Preset: next preset).
	Confirm,
	/// Esc / Circle / right mouse button.
	Back,
	/// Backspace / Square: focused row back to its default.
	Reset,
	/// Space / Triangle: hide the panels to look at the game behind, and back.
	Peek,
	TabPrev,
	TabNext,
	PageUp,
	PageDown,
	Home,
	End,
}

impl Action {
	/// Held directions repeat; everything else acts once per press.
	pub fn repeats(self) -> bool {
		matches!(self, Action::Up | Action::Down | Action::Left | Action::Right)
	}
}

/// Keys the UI listens to (the host maps its window system's keys to these; everything else is not the UI's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiKey {
	Up,
	Down,
	Left,
	Right,
	W,
	A,
	S,
	D,
	Enter,
	Space,
	Esc,
	Backspace,
	Q,
	E,
	Tab,
	/// Either shift key (Shift+Tab = previous tab).
	Shift,
	PageUp,
	PageDown,
	Home,
	End,
}

/// The action of a key press. W A S D work as the arrows; Q / E and Tab / Shift+Tab switch the tab.
pub fn key_action(k: UiKey, shift: bool) -> Option<Action> {
	Some(match k {
		UiKey::Up | UiKey::W => Action::Up,
		UiKey::Down | UiKey::S => Action::Down,
		UiKey::Left | UiKey::A => Action::Left,
		UiKey::Right | UiKey::D => Action::Right,
		UiKey::Enter => Action::Confirm,
		UiKey::Space => Action::Peek,
		UiKey::Esc => Action::Back,
		UiKey::Backspace => Action::Reset,
		UiKey::Q => Action::TabPrev,
		UiKey::E => Action::TabNext,
		UiKey::Tab if shift => Action::TabPrev,
		UiKey::Tab => Action::TabNext,
		UiKey::PageUp => Action::PageUp,
		UiKey::PageDown => Action::PageDown,
		UiKey::Home => Action::Home,
		UiKey::End => Action::End,
		UiKey::Shift => return None,
	})
}

// PS2 pad bits (engine/input/pad.rs `PadState::buttons`).
pub const P_L1: u16 = 0x04;
pub const P_R1: u16 = 0x08;
pub const P_TRIANGLE: u16 = 0x10;
pub const P_CIRCLE: u16 = 0x20;
pub const P_CROSS: u16 = 0x40;
pub const P_SQUARE: u16 = 0x80;
pub const P_UP: u16 = 0x1000;
pub const P_RIGHT: u16 = 0x2000;
pub const P_DOWN: u16 = 0x4000;
pub const P_LEFT: u16 = 0x8000;

/// The pad bits the UI reads and what each does: Cross change, Circle back, Square reset, Triangle view game, L1 / R1 tabs.
pub const PAD_ACTIONS: [(u16, Action); 10] = [
	(P_UP, Action::Up),
	(P_DOWN, Action::Down),
	(P_LEFT, Action::Left),
	(P_RIGHT, Action::Right),
	(P_CROSS, Action::Confirm),
	(P_CIRCLE, Action::Back),
	(P_SQUARE, Action::Reset),
	(P_TRIANGLE, Action::Peek),
	(P_L1, Action::TabPrev),
	(P_R1, Action::TabNext),
];

/// One pad report: PS2 button bits and the left stick (0x80 = centre; y up = low).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadSample {
	pub buttons: u16,
	pub lx: u8,
	pub ly: u8,
}

impl PadSample {
	pub const IDLE: PadSample = PadSample { buttons: 0, lx: 0x80, ly: 0x80 };
}

/// The buttons of a report with the left stick folded in as a d-pad (dead zone |v - 0x7f| > 0x2f, as the achievements list).
pub fn pad_mask(p: PadSample) -> u16 {
	let mut m = p.buttons;
	let dx = p.lx as i32 - 0x7f;
	let dy = p.ly as i32 - 0x7f;
	if dx.abs() > 0x2f {
		m |= if dx < 0 { P_LEFT } else { P_RIGHT };
	}
	if dy.abs() > 0x2f {
		m |= if dy < 0 { P_UP } else { P_DOWN };
	}
	m
}

/// The last device the user touched: decides the hint bar (keycaps or pad glyphs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Device {
	#[default]
	Keyboard,
	Pad,
}

/// Which button symbols the pad hints show (the game's Button glyphs option).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GlyphSet {
	#[default]
	PlayStation,
	Letters,
	/// Letters, Nintendo layout (Cross = B, Circle = A, Square = Y, Triangle = X).
	Switch,
}

impl GlyphSet {
	/// 0 PlayStation, 1 Letters, 2 Letters (Switch); anything else PlayStation.
	pub fn from_index(i: u8) -> GlyphSet {
		match i {
			1 => GlyphSet::Letters,
			2 => GlyphSet::Switch,
			_ => GlyphSet::PlayStation,
		}
	}
	pub fn glyph(self, s: Symbol) -> GlyphKind {
		let l = match (self, s) {
			(GlyphSet::PlayStation, s) => return GlyphKind::Symbol(s),
			(GlyphSet::Letters, Symbol::Cross) | (GlyphSet::Switch, Symbol::Circle) => 'A',
			(GlyphSet::Letters, Symbol::Circle) | (GlyphSet::Switch, Symbol::Cross) => 'B',
			(GlyphSet::Letters, Symbol::Square) | (GlyphSet::Switch, Symbol::Triangle) => 'X',
			(GlyphSet::Letters, Symbol::Triangle) | (GlyphSet::Switch, Symbol::Square) => 'Y',
		};
		GlyphKind::Letter(l)
	}
	/// Keycap names of the two shoulder buttons L1 / R1 in this set.
	pub fn shoulders(self) -> (&'static str, &'static str) {
		match self {
			GlyphSet::PlayStation => ("L1", "R1"),
			GlyphSet::Letters => ("LB", "RB"),
			GlyphSet::Switch => ("L", "R"),
		}
	}
	/// Keycap names of the two triggers L2 / R2 in this set.
	pub fn triggers(self) -> (&'static str, &'static str) {
		match self {
			GlyphSet::PlayStation => ("L2", "R2"),
			GlyphSet::Letters => ("LT", "RT"),
			GlyphSet::Switch => ("ZL", "ZR"),
		}
	}
	/// Keycap names of the left / right stick (pushed, not clicked) in every set.
	pub fn sticks(self) -> (&'static str, &'static str) {
		("LS", "RS")
	}
}

pub const REPEAT_DELAY_MS: u64 = 400;
pub const REPEAT_INTERVAL_MS: u64 = 60;
/// Repeats one `poll` may return (a hitch must not run the focus across the list).
const MAX_PER_POLL: u64 = 4;

#[derive(Clone, Copy, Debug)]
struct Rep {
	src: u32,
	action: Action,
	next: u64,
}

/// Auto-repeat of one held direction: the first press acts at once, the repeats start `REPEAT_DELAY_MS` later, one per
/// `REPEAT_INTERVAL_MS`. The latest direction pressed owns the repeat; releasing its key stops it. Times are milliseconds on any
/// monotonic clock the caller passes in.
#[derive(Debug, Default)]
pub struct Repeater {
	cur: Option<Rep>,
}

impl Repeater {
	pub fn press(&mut self, src: u32, action: Action, now: u64) {
		self.cur = action.repeats().then_some(Rep { src, action, next: now + REPEAT_DELAY_MS });
	}
	pub fn release(&mut self, src: u32) {
		if self.cur.is_some_and(|r| r.src == src) {
			self.cur = None;
		}
	}
	pub fn clear(&mut self) {
		self.cur = None;
	}
	pub fn is_holding(&self) -> bool {
		self.cur.is_some()
	}
	/// The repeats due at `now` (several if frames were slow, at most `MAX_PER_POLL`).
	pub fn poll(&mut self, now: u64) -> Vec<Action> {
		let Some(r) = self.cur.as_mut() else { return Vec::new() };
		if now < r.next {
			return Vec::new();
		}
		let due = ((now - r.next) / REPEAT_INTERVAL_MS + 1).min(MAX_PER_POLL);
		r.next = if (now - r.next) / REPEAT_INTERVAL_MS + 1 > MAX_PER_POLL { now + REPEAT_INTERVAL_MS } else { r.next + due * REPEAT_INTERVAL_MS };
		vec![r.action; due as usize]
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
	Left,
	Right,
	Middle,
}

/// Mouse events in WINDOW pixels (the UI converts them to design units with its `Scale`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MouseEvent {
	Move { x: f32, y: f32 },
	Down { button: MouseButton, x: f32, y: f32 },
	Up { button: MouseButton },
	/// Wheel notches (positive = away from the user = list up); a pixel delta divided by ~120.
	Wheel { lines: f32 },
}

/// What a screen asks of its host after an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Outcome {
	#[default]
	Stay,
	/// The screen was left (Back, or a click on the close area).
	Close,
}
