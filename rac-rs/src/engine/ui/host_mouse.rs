//! Mouse input of the options screen: hover focuses a row, left click presses the widget under the cursor, a slider is dragged,
//! the wheel scrolls the list, the right button is Back. Positions arrive in window pixels and are mapped through `Ui::scale`.
use super::host::Ui;
use super::input::{Action, Device, MouseButton, MouseEvent, Outcome};
use super::kit::layout::{CtrlBox, ItemBox};
use super::kit::{self, Hit, HintKind, Layout};
use super::prim::Rect;
use super::screen::ScreenMap;

/// Design units one wheel notch scrolls (3 rows).
const WHEEL_STEP: f32 = 3.0 * 54.0;

impl Ui {
	pub fn mouse(&mut self, ev: MouseEvent, now: u64) -> Outcome {
		self.model.now = now;
		if !matches!(ev, MouseEvent::Move { .. }) {
			self.apply_hover(); // the moves that came before a click, a release or a wheel notch are applied first, in order
		}
		match ev {
			MouseEvent::Move { x, y } => {
				let (dx, dy) = self.scale.to_design(x, y);
				if self.cursor != Some((dx, dy)) {
					self.cursor = Some((dx, dy));
					self.model.device = Device::Keyboard;
					// Not applied here: a hover builds the whole screen and its layout, and a fast mouse sends hundreds of moves a
					// second. The newest position waits for `tick` (once per display frame).
					self.hover_at = Some((dx, dy));
				}
				Outcome::Stay
			}
			MouseEvent::Down { button: MouseButton::Left, x, y } => {
				let (dx, dy) = self.scale.to_design(x, y);
				self.cursor = Some((dx, dy));
				self.model.device = Device::Keyboard;
				self.click(dx, dy)
			}
			MouseEvent::Down { button: MouseButton::Right, .. } => {
				self.model.device = Device::Keyboard;
				self.act(Action::Back)
			}
			MouseEvent::Down { .. } => Outcome::Stay,
			MouseEvent::Up { button: MouseButton::Left } => {
				self.model.drag = None;
				Outcome::Stay
			}
			MouseEvent::Up { .. } => Outcome::Stay,
			MouseEvent::Wheel { lines } => {
				if !self.model.peek {
					let (_, l, _) = self.laid_out();
					self.model.scroll = (l.scroll - lines * WHEEL_STEP).clamp(0.0, l.max_scroll);
				}
				Outcome::Stay
			}
		}
	}

	fn laid_out(&self) -> (kit::Screen, Layout, ScreenMap) {
		let (s, map) = self.screen();
		let l = kit::layout_screen(&s, &*self.measure);
		(s, l, map)
	}

	/// Applies the newest queued mouse move (focus follows the cursor; a slider drag follows x): one screen build per call.
	pub(super) fn apply_hover(&mut self) {
		if let Some((x, y)) = self.hover_at.take() {
			self.hover(x, y);
		}
	}

	fn hover(&mut self, x: f32, y: f32) {
		let (_, l, map) = self.laid_out();
		if let Some(row) = self.model.drag {
			if let Some(track) = map.row_item(row).and_then(|it| slider_track(&l, it)) {
				self.model.slider_to(row, kit::slider_t(&track, x));
			}
			return;
		}
		let item = match kit::hit(&l, self.model.peek, x, y) {
			Hit::Row(i) | Hit::Prev(i) | Hit::Next(i) | Hit::Switch(i) | Hit::Slider(i) | Hit::Chip(i, _) => i,
			_ => return,
		};
		if let Some(row) = map.row(item) {
			self.model.focus_to(row);
		}
	}

	fn click(&mut self, x: f32, y: f32) -> Outcome {
		let (_, l, map) = self.laid_out();
		let hit = kit::hit(&l, self.model.peek, x, y);
		let row_of = |i: usize| map.row(i);
		match hit {
			Hit::Tab(t) => self.model.set_tab(t),
			Hit::TabPrev => self.model.tab_step(-1),
			Hit::TabNext => self.model.tab_step(1),
			Hit::Row(i) => {
				if let Some(r) = row_of(i) {
					self.model.focus_to(r);
				}
			}
			Hit::Prev(i) | Hit::Next(i) => {
				if let Some(r) = row_of(i) {
					self.model.focus_to(r);
					self.model.step_row(r, if matches!(hit, Hit::Prev(_)) { -1 } else { 1 });
				}
			}
			Hit::Switch(i) => {
				if let Some(r) = row_of(i) {
					self.model.focus_to(r);
					self.model.step_row(r, 1);
				}
			}
			Hit::Slider(i) => {
				if let (Some(r), Some(track)) = (row_of(i), slider_track(&l, i)) {
					self.model.focus_to(r);
					self.model.drag = Some(r);
					self.model.slider_to(r, kit::slider_t(&track, x));
				}
			}
			Hit::Chip(i, c) => {
				if let Some(r) = row_of(i) {
					self.model.focus_to(r);
					if c < self.model.menu.presets.len() {
						self.model.apply_preset(c);
					}
				}
			}
			Hit::Banner(_) => {
				if let Some(rule) = self.model.tab_rule() {
					self.model.enhance(rule);
				}
			}
			Hit::DetailBox => {
				if let Some(rule) = self.model.rule(self.model.focus) {
					self.model.enhance(rule);
				}
			}
			// Peeking in or out ticks like Space / Triangle do (`act(Peek)`).
			Hit::DetailFooter | Hit::Hint(HintKind::Peek) => self.set_peek(true),
			Hit::Hint(HintKind::Reset) => self.model.reset(),
			Hit::PeekPill => self.set_peek(false),
			Hit::Hint(_) | Hit::List | Hit::None => {}
		}
		if !matches!(hit, Hit::Slider(_) | Hit::None | Hit::List) {
			self.fix_scroll();
		}
		Outcome::Stay
	}
}

/// The track rectangle of the slider in item `item`.
fn slider_track(l: &Layout, item: usize) -> Option<Rect> {
	match l.items.get(item)? {
		ItemBox::Row { ctrl: CtrlBox::Slider { track, .. }, .. } => Some(*track),
		_ => None,
	}
}
