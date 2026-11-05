//! The kit's "page" screen (v0.5 modern menu, screens stage B): a left menu column of big buttons, one panel on the right (save
//! cards, or label / value rows) and an optional centred dialog, over the scene (no blur backdrop: the title scene stays sharp).
//! Plain data in (`PageScreen`), rectangles out (`layout`), prims in `page_draw.rs`, hit test `PageLayout::hit`. Numbers are read
//! off the approved title mockup in the 1280x720 design space. Nothing here knows a game: the title screens of a pack fill it.
use super::data::{Hint, HintKind};
use super::layout::{leads, place_leads, Lead, LeadWhat, Line};
use super::shell::{shell_box, tab_boxes, Shell, ShellBox, SHELL_ROWS_W};
use super::text::{run_width, upper, wrap, TextMeasure};
use crate::ui::input::{Device, GlyphSet};
use crate::ui::prim::{FontRole, Rect, Rgba, Symbol};
use crate::ui::theme;

pub const MENU_X: f32 = 48.0;
pub const MENU_Y: f32 = 232.0;
pub const MENU_W: f32 = 360.0;
pub const MENU_H: f32 = 64.0;
pub const MENU_GAP: f32 = 8.0;
pub const MENU_PX: f32 = 30.0;
pub const MENU_TRACK: f32 = 0.04;
pub const PANEL_X: f32 = 456.0;
pub const CARDS_W: f32 = 776.0;
pub const ROWS_W: f32 = 560.0;
pub const PANEL_PAD: f32 = 20.0;
pub const CAPTION_PX: f32 = 15.0;
pub const CAPTION_TRACK: f32 = 0.16;
pub const SIDE_PX: f32 = 14.0;
pub const CARD_PX: f32 = 24.0;
pub const STAT_LABEL_PX: f32 = 12.0;
pub const STAT_VALUE_PX: f32 = 15.0;
pub const ROW_H: f32 = 56.0;
pub const ROW_GAP: f32 = 2.0;
pub const ROW_LABEL_PX: f32 = 19.0;
pub const ROW_VALUE_PX: f32 = 16.0;
pub const HINT_Y: f32 = 688.0;
pub const DIALOG_W: f32 = 520.0;
pub const DIALOG_PX: f32 = 19.0;
pub const BUTTON_PX: f32 = 17.0;
/// A slider row's bar track.
pub const BAR_W: f32 = 150.0;
/// The left scrim: alpha at the left edge, at 576 and at the end (960), colour (5, 8, 15).
pub const SCRIM_COLOUR: Rgba = Rgba::rgba(5, 8, 15, 1.0);
pub const SCRIM_ALPHA: [f32; 3] = [0.88, 0.55, 0.0];
pub const DIALOG_SCRIM: Rgba = Rgba::rgba(5, 8, 15, 0.6);

/// One button of the left column.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuButton {
	pub text: String,
	pub selected: bool,
	/// Clickable: false = the entry that led to the current page (kept on screen, inert).
	pub live: bool,
}

/// One column of a card: a small label over a value.
#[derive(Clone, Debug, PartialEq)]
pub struct Stat {
	pub label: String,
	pub value: String,
}

/// A card of the cards panel (stats) or a row of the rows panel (value); `extra` rows use `label` only.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PageRow {
	pub label: String,
	pub value: String,
	pub stats: Vec<Stat>,
	pub selected: bool,
	/// Drawn dimmed (an empty save slot, an inert row).
	pub dim: bool,
	/// A slider: how full its bar is, 0..1 (rows only; the bar sits left of the value text).
	pub bar: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Panel {
	None,
	/// Save cards plus an optional button row below them ("Continue without saving").
	Cards { caption: String, side: String, cards: Vec<PageRow>, extra: Option<PageRow> },
	Rows { caption: String, rows: Vec<PageRow> },
}

/// A dialog button: the pad symbol, the keyboard cap and the word.
#[derive(Clone, Debug, PartialEq)]
pub struct DialogButton {
	pub symbol: Symbol,
	pub key: String,
	pub text: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Dialog {
	/// Wrapped to the box; an empty string is a blank line between two texts.
	pub lines: Vec<String>,
	pub buttons: Vec<DialogButton>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageScreen {
	pub menu: Vec<MenuButton>,
	pub panel: Panel,
	pub dialog: Option<Dialog>,
	pub hints: Vec<Hint>,
	/// A small note at the bottom left (empty = none).
	pub footer: String,
	pub device: Device,
	pub glyphs: GlyphSet,
	pub accent: Rgba,
	/// The pause shell: `menu` is then the tab bar across the top and the panel sits at the list position (`shell.rs`).
	pub shell: Option<Shell>,
}

impl Default for PageScreen {
	fn default() -> PageScreen {
		PageScreen { menu: Vec::new(), panel: Panel::None, dialog: None, hints: Vec::new(), footer: String::new(), device: Device::Keyboard, glyphs: GlyphSet::PlayStation, accent: theme::ACCENT, shell: None }
	}
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuBox {
	pub rect: Rect,
	pub bar: Rect,
	pub text: Line,
}

/// A card or a row placed: label at the left, stats (label line, value line) right to left, or one value line.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemBox {
	pub rect: Rect,
	pub label: Line,
	/// Width the label may use.
	pub label_w: f32,
	pub stats: Vec<(Line, Line)>,
	pub value: Option<Line>,
	pub card: bool,
	/// A slider's bar (the whole track); the fill is `PageRow::bar` of it.
	pub bar: Option<Rect>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PanelBox {
	pub rect: Rect,
	pub caption: Line,
	pub side: Option<Line>,
	/// Cards (or rows) then the extra row: the index is the hit index.
	pub items: Vec<ItemBox>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ButtonBox {
	pub rect: Rect,
	pub lead: Lead,
	pub text: Line,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DialogBox {
	pub rect: Rect,
	pub lines: Vec<Line>,
	pub buttons: Vec<ButtonBox>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HintBox {
	pub kind: HintKind,
	/// The clickable area: the leads and the text, with a little margin.
	pub rect: Rect,
	pub leads: Vec<Lead>,
	pub text: Line,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageLayout {
	pub menu: Vec<MenuBox>,
	pub panel: Option<PanelBox>,
	pub dialog: Option<DialogBox>,
	pub hints: Vec<HintBox>,
	pub footer: Option<Line>,
	pub shell: Option<ShellBox>,
}

/// What a design-space point is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageHit {
	None,
	/// A live button of the left column.
	Menu(usize),
	/// A card, row or the extra row of the panel.
	Item(usize),
	/// A dialog button (a dialog swallows every other hit).
	Button(usize),
	/// A hint-bar entry (Change = Enter, Back = Esc; the Tabs hint is not clickable).
	Hint(HintKind),
}

impl PageLayout {
	pub fn hit(&self, s: &PageScreen, x: f32, y: f32) -> PageHit {
		if let Some(d) = &self.dialog {
			return d.buttons.iter().position(|b| b.rect.contains(x, y)).map_or(PageHit::None, PageHit::Button);
		}
		if let Some(i) = self.menu.iter().position(|b| b.rect.contains(x, y)) {
			return if s.menu.get(i).is_some_and(|b| b.live) { PageHit::Menu(i) } else { PageHit::None };
		}
		// The hint bar is drawn over the panel: a click on a hint is the hint's (R1-9), even where a long panel reaches under it.
		if let Some(h) = self.hints.iter().find(|h| h.kind != HintKind::Tabs && h.rect.contains(x, y)) {
			return PageHit::Hint(h.kind);
		}
		self.panel.as_ref().and_then(|p| p.items.iter().position(|it| it.rect.contains(x, y))).map_or(PageHit::None, PageHit::Item)
	}
}

fn line(text: &str, x: f32, y: f32) -> Line {
	Line { text: text.to_string(), x, y }
}

fn menu_boxes(s: &PageScreen) -> Vec<MenuBox> {
	s.menu
		.iter()
		.enumerate()
		.map(|(i, b)| {
			let rect = Rect::new(MENU_X, MENU_Y + i as f32 * (MENU_H + MENU_GAP), MENU_W, MENU_H);
			MenuBox { rect, bar: Rect::new(rect.x + 20.0, rect.cy() - 14.0, 4.0, 28.0), text: line(&upper(&b.text), rect.x + 38.0, rect.cy()) }
		})
		.collect()
}

/// The stat columns of a card from the right edge `right` going left: (label line, value line), widest-first measured.
fn stat_boxes(stats: &[Stat], right: f32, cy: f32, m: &dyn TextMeasure) -> (Vec<(Line, Line)>, f32) {
	let mut out = Vec::new();
	let mut x = right;
	for (i, st) in stats.iter().enumerate().rev() {
		let min_w = if i + 1 == stats.len() && i > 0 { 96.0 } else { 0.0 };
		let w = run_width(m, FontRole::Regular, STAT_LABEL_PX, &st.label, 0.0).max(run_width(m, FontRole::SemiBold, STAT_VALUE_PX, &st.value, 0.0)).max(min_w);
		out.push((line(&st.label, x, cy - 9.0), line(&st.value, x, cy + 9.0)));
		x -= w + 22.0;
	}
	out.reverse();
	(out, x)
}

fn item_box(r: &PageRow, rect: Rect, card: bool, m: &dyn TextMeasure) -> ItemBox {
	if card {
		let (stats, left) = stat_boxes(&r.stats, rect.right() - PANEL_PAD, rect.cy(), m);
		let label_w = (left - (rect.x + PANEL_PAD) - 10.0).clamp(60.0, 300.0);
		return ItemBox { rect, label: line(&r.label, rect.x + PANEL_PAD, rect.cy()), label_w, stats, value: None, card, bar: None };
	}
	let bar = r.bar.map(|_| Rect::new(rect.right() - PANEL_PAD - BAR_W, rect.cy() - 4.0, BAR_W, 8.0));
	let value = (!r.value.is_empty()).then(|| line(&r.value, bar.map_or(rect.right() - PANEL_PAD, |b| b.x - 14.0), rect.cy()));
	let vw = value.as_ref().map_or(0.0, |_| run_width(m, FontRole::SemiBold, ROW_VALUE_PX, &r.value, 0.0)) + bar.map_or(0.0, |_| BAR_W + 14.0);
	ItemBox { rect, label: line(&r.label, rect.x + PANEL_PAD, rect.cy()), label_w: (rect.w - 2.0 * PANEL_PAD - vw - 16.0).clamp(60.0, 330.0), stats: Vec::new(), value, card, bar }
}

fn panel_box(s: &PageScreen, m: &dyn TextMeasure) -> Option<PanelBox> {
	let head = |x: f32, y: f32, w: f32, caption: &str, side: &str| (line(&upper(caption), x + PANEL_PAD, y + PANEL_PAD + 10.0), (!side.is_empty()).then(|| line(side, x + w - PANEL_PAD, y + PANEL_PAD + 10.0)));
	match &s.panel {
		Panel::None => None,
		Panel::Cards { caption, side, cards, extra } => {
			let (x, y) = (if s.shell.is_some() { MENU_X } else { PANEL_X }, 116.0);
			let (ch, gap) = if extra.is_some() { (68.0, 8.0) } else { (80.0, 10.0) };
			let mut items = Vec::new();
			let top = y + PANEL_PAD + 20.0 + 10.0;
			for (i, c) in cards.iter().enumerate() {
				items.push(item_box(c, Rect::new(x + PANEL_PAD, top + i as f32 * (ch + gap), CARDS_W - 2.0 * PANEL_PAD, ch), true, m));
			}
			let mut bottom = top + cards.len() as f32 * (ch + gap) - gap;
			if let Some(e) = extra {
				items.push(item_box(e, Rect::new(x + PANEL_PAD, bottom + 10.0, CARDS_W - 2.0 * PANEL_PAD, 52.0), false, m));
				bottom += 10.0 + 52.0;
			}
			let (caption, side) = head(x, y, CARDS_W, caption, side);
			Some(PanelBox { rect: Rect::new(x, y, CARDS_W, bottom + PANEL_PAD - y), caption, side, items })
		}
		Panel::Rows { caption, rows } => {
			let shell = s.shell.is_some();
			let (x, w) = if shell { (MENU_X, SHELL_ROWS_W) } else { (PANEL_X, ROWS_W) };
			let y = if shell || rows.len() > 6 { 116.0 } else { MENU_Y };
			let top = y + PANEL_PAD + if shell && caption.is_empty() { 0.0 } else { 30.0 };
			// A long list shrinks its rows to stay above the hint bar (the mockup's panel ends at y 656): the shell's and the title's
			// (a title Mods section has up to 13 rows; at ROW_H it ran off the screen). Up to 8 rows keep ROW_H; 12 rows still fit at 37 px, the pause Options list's 14 (Modern: Display and Game) at 31.
			let rh = ((656.0 - PANEL_PAD - top + ROW_GAP) / rows.len().max(1) as f32 - ROW_GAP).clamp(30.0, ROW_H);
			let items: Vec<ItemBox> = rows.iter().enumerate().map(|(i, r)| item_box(r, Rect::new(x + PANEL_PAD, top + i as f32 * (rh + ROW_GAP), w - 2.0 * PANEL_PAD, rh), false, m)).collect();
			let bottom = top + rows.len() as f32 * (rh + ROW_GAP) - ROW_GAP;
			let (caption, side) = head(x, y, w, caption, "");
			Some(PanelBox { rect: Rect::new(x, y, w, bottom + PANEL_PAD - y), caption, side, items })
		}
	}
}

fn dialog_box(s: &PageScreen, d: &Dialog, m: &dyn TextMeasure) -> DialogBox {
	let inner = DIALOG_W - 56.0;
	let mut texts: Vec<(String, f32)> = Vec::new();
	for l in &d.lines {
		if l.is_empty() {
			texts.push((String::new(), 14.0));
		}
		for w in wrap(m, FontRole::Medium, DIALOG_PX, l, inner) {
			texts.push((w, 26.0));
		}
	}
	let text_h: f32 = texts.iter().map(|t| t.1).sum();
	let btn_h = if d.buttons.is_empty() { 0.0 } else { 22.0 + 44.0 };
	let h = 28.0 + text_h + btn_h + 28.0;
	let rect = Rect::new((theme::DESIGN_W - DIALOG_W) * 0.5, (theme::DESIGN_H - h) * 0.5, DIALOG_W, h);
	let mut y = rect.y + 28.0;
	let mut lines = Vec::new();
	for (t, lh) in texts {
		lines.push(line(&t, rect.cx(), y + lh * 0.5));
		y += lh;
	}
	let widths: Vec<f32> = d.buttons.iter().map(|b| 18.0 + 24.0 + 8.0 + run_width(m, FontRole::Medium, BUTTON_PX, &b.text, 0.0) + 18.0).collect();
	let total: f32 = widths.iter().sum::<f32>() + 16.0 * d.buttons.len().saturating_sub(1) as f32;
	let mut x = rect.cx() - total * 0.5;
	let mut buttons = Vec::new();
	for (b, w) in d.buttons.iter().zip(widths) {
		let r = Rect::new(x, y + 22.0, w, 44.0);
		let what = match s.device {
			Device::Pad => LeadWhat::Pad(b.symbol),
			Device::Keyboard => LeadWhat::Key(b.key.clone()),
		};
		let (ls, end) = place_leads(m, &[what], r.x + 18.0, r.cy(), 24.0, 22.0, 6.0);
		buttons.push(ButtonBox { rect: r, lead: ls[0].clone(), text: line(&b.text, end + 8.0, r.cy()) });
		x += w + 16.0;
	}
	DialogBox { rect, lines, buttons }
}

fn hint_boxes(s: &PageScreen, m: &dyn TextMeasure) -> Vec<HintBox> {
	let mut end = 1232.0;
	let mut out = Vec::new();
	for h in s.hints.iter().rev() {
		let whats = leads(h.kind, s.device, s.glyphs);
		let (_, lead_w) = place_leads(m, &whats, 0.0, 0.0, 24.0, 22.0, 6.0);
		let w = lead_w + 8.0 + run_width(m, FontRole::Medium, 15.0, &h.text, 0.0);
		let x = end - w;
		let (ls, lead_end) = place_leads(m, &whats, x, HINT_Y, 24.0, 22.0, 6.0);
		let rect = Rect::new(x - 8.0, HINT_Y - 17.0, w + 16.0, 34.0);
		out.push(HintBox { kind: h.kind, rect, leads: ls, text: line(&h.text, lead_end + 8.0, HINT_Y) });
		end = x - 26.0;
	}
	out.reverse();
	out
}

pub fn layout(s: &PageScreen, m: &dyn TextMeasure) -> PageLayout {
	let (menu, shell) = match &s.shell {
		Some(sh) => {
			let (tabs, px) = tab_boxes(s, sh, m);
			(tabs, Some(shell_box(sh, m, px)))
		}
		None => (menu_boxes(s), None),
	};
	PageLayout {
		menu,
		panel: panel_box(s, m),
		dialog: s.dialog.as_ref().map(|d| dialog_box(s, d, m)),
		hints: hint_boxes(s, m),
		footer: (!s.footer.is_empty()).then(|| line(&s.footer, MENU_X, HINT_Y)),
		shell,
	}
}
