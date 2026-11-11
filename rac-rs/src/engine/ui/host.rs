//! `Ui`: the options screen with its input. The host calls `key` / `pad` / `mouse` / `tick` at DISPLAY rate with a millisecond
//! clock of its own, then `frame()` for the draw list; `take_changes()` hands the changed values to the game, `refresh()` takes the
//! game's values back. Mouse input is in `host_mouse.rs`, the thin `Pack::ui` wrappers at the end of this file.
use super::input::{key_action, pad_mask, Action, Device, GlyphSet, PadSample, Outcome, Repeater, UiKey, PAD_ACTIONS};
use super::kit::{self, Layout, Scale, TextMeasure};
use super::model::{Change, Model};
use super::prim::Prim;
use super::rows::{UiMenu, UiSound};
use super::screen::{self, ScreenMap};

/// Rows a page up / down key moves.
const PAGE_ROWS: usize = 5;

pub struct Ui {
	pub model: Model,
	pub(super) measure: Box<dyn TextMeasure + Send>,
	pub(super) rep: Repeater,
	held: Vec<u32>,
	shift: bool,
	pad_mask: u16,
	pub scale: Scale,
	/// Last mouse position in design units.
	pub(super) cursor: Option<(f32, f32)>,
	/// The newest mouse move not yet applied (design units): a hover costs a screen build and a layout, so the moves of one display
	/// frame are folded into this and applied once, by `tick` (or before any other input, to keep the order); see `host_mouse.rs`.
	pub(super) hover_at: Option<(f32, f32)>,
}

pub(super) fn merge(a: Outcome, b: Outcome) -> Outcome {
	if a == Outcome::Close || b == Outcome::Close {
		Outcome::Close
	} else {
		Outcome::Stay
	}
}

impl Ui {
	pub fn new(menu: &'static UiMenu, lang: usize, measure: Box<dyn TextMeasure + Send>) -> Ui {
		Ui {
			model: Model::new(menu, lang),
			measure,
			rep: Repeater::default(),
			held: Vec::new(),
			shift: false,
			pad_mask: 0,
			scale: Scale::fit(1280, 720),
			cursor: None,
			hover_at: None,
		}
	}

	/// The window size in pixels (mouse positions are mapped through it).
	pub fn set_window(&mut self, w: u32, h: u32) {
		self.scale = Scale::fit(w, h);
	}

	/// Starts a visit: `front` = from the title screen; `tab` = the tab id to open (None = the first); panels shown, no key held.
	pub fn begin(&mut self, front: bool, tab: Option<&str>) {
		self.model.front = front;
		self.model.peek = false;
		self.model.tab = 0;
		self.model.focus = self.model.first_shown();
		self.model.scroll = 0.0;
		if let Some(id) = tab {
			self.model.open_tab(id);
		}
		self.lose_focus();
		self.model.take_sounds();
	}

	/// The window lost focus or the screen closed: forget held keys and buttons, stop a drag.
	pub fn lose_focus(&mut self) {
		self.rep.clear();
		self.held.clear();
		self.pad_mask = 0;
		self.shift = false;
		self.model.drag = None;
		self.hover_at = None;
	}

	pub fn key(&mut self, k: UiKey, pressed: bool, now: u64) -> Outcome {
		self.model.now = now;
		if k == UiKey::Shift {
			self.shift = pressed;
			return Outcome::Stay;
		}
		let src = k as u32;
		if !pressed {
			self.held.retain(|s| *s != src);
			self.rep.release(src);
			return Outcome::Stay;
		}
		if self.held.contains(&src) {
			return Outcome::Stay; // the OS's own key repeat: the UI has its own
		}
		self.held.push(src);
		self.model.device = Device::Keyboard;
		match key_action(k, self.shift) {
			Some(a) => self.press(src, a, now),
			None => Outcome::Stay,
		}
	}

	/// One pad report (PS2 bits, left stick folded in as a d-pad): every newly pressed button acts once; held directions repeat.
	pub fn pad(&mut self, p: PadSample, now: u64) -> Outcome {
		self.model.now = now;
		let mask = pad_mask(p);
		let (down, up) = (mask & !self.pad_mask, self.pad_mask & !mask);
		self.pad_mask = mask;
		let mut out = Outcome::Stay;
		for (bit, action) in PAD_ACTIONS {
			let src = 1000 + bit.trailing_zeros();
			if up & bit != 0 {
				self.rep.release(src);
			}
			if down & bit != 0 {
				self.model.device = Device::Pad;
				out = merge(out, self.press(src, action, now));
			}
		}
		out
	}

	/// Takes a pad report as the state already held WITHOUT acting on it: a button down when the screen opened (the Cross that
	/// opened it) must not Confirm the first row, so the host primes the held mask with this before the first `pad` call.
	pub fn hold_pad(&mut self, p: PadSample) {
		self.pad_mask = pad_mask(p);
	}

	fn press(&mut self, src: u32, a: Action, now: u64) -> Outcome {
		self.rep.press(src, a, now);
		self.act(a)
	}

	/// The auto-repeat of a held direction, and the frame's mouse hover (call every display frame, before `frame()`).
	pub fn tick(&mut self, now: u64) -> Outcome {
		self.model.now = now;
		self.apply_hover();
		let mut out = Outcome::Stay;
		for a in self.rep.poll(now) {
			out = merge(out, self.act(a));
		}
		out
	}

	/// Peek on or off from the mouse, with the Move tick the keys' `act(Peek)` plays (nothing when it is already so).
	pub(super) fn set_peek(&mut self, on: bool) {
		if self.model.peek != on {
			self.model.peek = on;
			self.model.sound(UiSound::Move);
		}
	}

	/// What the user asked for. While peeking only Peek / Back / Confirm do anything (they bring the panels back).
	pub fn act(&mut self, a: Action) -> Outcome {
		self.apply_hover(); // a mouse move that came first has moved the focus before the action runs
		self.model.settle_focus(); // never act on a hidden row
		let m = &mut self.model;
		if m.peek {
			if matches!(a, Action::Peek | Action::Back | Action::Confirm) {
				m.peek = false;
				m.sound(UiSound::Move);
			}
			return Outcome::Stay;
		}
		match a {
			Action::Up => m.focus_step(-1),
			Action::Down => m.focus_step(1),
			Action::Left => m.step(-1),
			Action::Right => m.step(1),
			Action::Confirm => m.confirm(),
			Action::Back => {
				m.sound(UiSound::Back);
				return Outcome::Close;
			}
			Action::Reset => m.reset(),
			Action::Peek => {
				m.peek = true;
				m.sound(UiSound::Move);
			}
			Action::TabPrev => m.tab_step(-1),
			Action::TabNext => m.tab_step(1),
			Action::PageUp => m.focus_page(-1, PAGE_ROWS),
			Action::PageDown => m.focus_page(1, PAGE_ROWS),
			Action::Home => m.focus_edge(false),
			Action::End => m.focus_edge(true),
		}
		self.fix_scroll();
		Outcome::Stay
	}

	/// Scrolls the list so the focused row is fully visible (the first / last row of a tab goes to the very top / bottom).
	pub fn fix_scroll(&mut self) {
		self.model.settle_focus(); // a change may have hidden rows (never the focused one; a direct write may)
		let (s, map) = screen::build(&self.model);
		let l = kit::layout_screen(&s, &*self.measure);
		if let Some(item) = map.row_item(self.model.focus) {
			self.model.scroll = l.scroll_to_show(item, self.model.is_first(), self.model.is_last());
		} else {
			self.model.scroll = l.scroll;
		}
	}

	pub fn screen(&self) -> (kit::Screen, ScreenMap) {
		screen::build(&self.model)
	}

	/// The layout and the draw list of the current state, in design units.
	pub fn frame(&self) -> (Layout, Vec<Prim>) {
		super::keycaps::set_lang(self.model.lang);
		kit::frame(&screen::build(&self.model).0, &*self.measure)
	}

	pub fn take_changes(&mut self) -> Vec<Change> {
		self.model.take_changes()
	}

	pub fn take_sounds(&mut self) -> Vec<UiSound> {
		self.model.take_sounds()
	}

	/// Takes the game's current values (see `Model::refresh`).
	pub fn refresh(&mut self, get: &dyn Fn(&str) -> Option<i32>) {
		let focus = self.model.focus;
		self.model.refresh(get);
		if self.model.focus != focus {
			self.fix_scroll(); // a value from the game hid the focused row: keep its successor in view
		}
	}

	pub fn set_glyphs(&mut self, g: GlyphSet) {
		self.model.glyphs = g;
	}
}

// -- the Pack::ui wrappers (the game side of the hooks) ---------------------------------------------------------------------

impl Ui {
	/// The UI of the active game pack (None = the pack has no modern menu); values and language read at once.
	pub fn from_pack(measure: Box<dyn TextMeasure + Send>) -> Option<Ui> {
		let p = crate::pack::get().ui?;
		let mut ui = Ui::new((p.menu)(), (p.lang)(), measure);
		ui.sync_pack();
		Some(ui)
	}

	/// Reads the game's values, language and glyph option through the pack.
	pub fn sync_pack(&mut self) {
		if let Some(p) = crate::pack::get().ui {
			self.model.lang = (p.lang)().min(super::rows::LANGS - 1);
			self.model.glyphs = GlyphSet::from_index((p.glyphs)());
			self.refresh(&|id| (p.get)(id));
		}
	}

	/// Passes the values changed since the last call to `Pack::ui.set` and the sounds to `Pack::ui_sound`.
	pub fn flush_pack(&mut self) {
		let pack = crate::pack::get();
		for c in self.take_changes() {
			if let Some(p) = pack.ui {
				(p.set)(c.id, c.value);
			}
		}
		for s in self.take_sounds() {
			(pack.ui_sound)(s);
		}
	}
}
