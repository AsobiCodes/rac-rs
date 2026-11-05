//! The modern Gadgetron vendor shop (v0.5 screens stage E, the approved look `MOCKUP_vendor.dc.html`): `build` turns a `ShopView`
//! (the game's list, prices, bolts, ammo and stage) into a draw list in the 1280x720 design space plus what each button does when
//! clicked. Pure: no statics, no clock; the main-thread host is `vendor_host.rs`, the glue is `host/handler/frame_vendor.rs`. The
//! game stays in charge: choosing a row, buying, the quantity, the refill and leaving are requests and pad presses sent to the retail
//! shop (`menu_model::cmd`); nothing here writes bolts or ammo. Layout: Buy all ammo and the vendor's stock on the left, the details
//! of the selected item on the right (no item preview: the retail panel quads are hidden, user 2026-11-02), the machine and the
//! hologram of the world stay free in the middle. Buy all ammo is a list entry like the stock rows (user 2026-11-03; the keys reach
//! it through `vendor_nav.rs`): selected, it has its own detail card (what it fills, the rounds and bolts, Buy all ammo / why not). The buy prompt is a modal box over a dimmed world. Texts: `vendor_text.rs`.
use super::icons;
use super::title::BOLTS;
use super::vendor_portrait;
use super::vendor_text::{self as t, commas, fill, word};
use crate::retail::shop_view::{ShopRow, ShopView};
use rac_engine::ui::input::{Device, GlyphSet};
use rac_engine::ui::keycaps::{self, Cap};
use rac_engine::ui::kit::text::{run_width, upper};
use rac_engine::ui::kit::TextMeasure;
use rac_engine::ui::prim::{Align, Border, FontRole, Prim, Rect, Rgba, Symbol, TextRun};
use rac_engine::ui::theme::*;

pub const LIST: Rect = Rect::new(48.0, 116.0, 520.0, 540.0);
pub const DETAIL: Rect = Rect::new(852.0, 116.0, 380.0, 540.0);
const ROW_H: f32 = 60.0;
const ROW_GAP: f32 = 4.0;
/// Stock rows the list shows at once under Buy all ammo (the retail list has at most 7; more scroll to keep the selection in view).
const VISIBLE: usize = 7;
const RED: Rgba = SYMBOL_CIRCLE;
const TEAL: Rgba = SYMBOL_TRIANGLE;

/// Everything `build` reads.
pub struct Facts<'a> {
	pub view: &'a ShopView,
	/// The retail name of each row's item (the disc's text bank), in row order.
	pub names: &'a [String],
	/// The row the player's mouse chose a moment ago (shown at once, before retail's cursor follows).
	pub focus: Option<usize>,
	/// The Buy all ammo row has the focus (`vendor_nav::focused`: the keys or the mouse chose it): drawn selected, its own card.
	pub buyall: bool,
	/// The first stock row the list showed last frame (`Built::first`, 0 for a new page): the list keeps that window while the
	/// selection is in it (`first_row`).
	pub scroll: usize,
	pub lang: usize,
	pub device: Device,
	pub glyphs: GlyphSet,
}

/// What a click at a point does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
	None,
	Row(usize),
	Buy,
	Refill,
	Less,
	More,
	Confirm,
	Cancel,
	/// A hint-bar entry, named by the pad button it stands for (Cross = Buy, Triangle = Leave / Cancel).
	Hint(Symbol),
}

/// The page and what its parts do.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
	pub prims: Vec<Prim>,
	/// The visible list rows: (row index, rect).
	pub rows: Vec<(usize, Rect)>,
	pub buy: Option<Rect>,
	/// The Buy all ammo row while it can be chosen (`choosable`; also when Square would buy nothing: the card says why).
	pub refill: Option<Rect>,
	/// The Buy all ammo row is shown selected (no stock row is; the detail card is its own).
	pub buyall: bool,
	pub less: Option<Rect>,
	pub more: Option<Rect>,
	pub confirm: Option<Rect>,
	pub cancel: Option<Rect>,
	/// The clickable hint-bar entries (Buy, Leave / Cancel): the pad button each stands for, and where it is.
	pub hints: Vec<(Symbol, Rect)>,
	/// The buy prompt is the screen.
	pub prompt: bool,
	/// The row shown selected.
	pub sel: usize,
	/// Rows in the shop's list (the visible ones are `rows`).
	pub count: usize,
	/// The first stock row shown (the next frame's `Facts::scroll`).
	pub first: usize,
	/// The hole in the detail card the vendor's live portrait shows in (`vendor_portrait.rs`).
	pub portrait: Option<Rect>,
}

impl Built {
	/// What a click at design point (x, y) does; while the prompt is up only its buttons act.
	pub fn hit(&self, x: f32, y: f32) -> Hit {
		let on = |r: &Option<Rect>| r.is_some_and(|r| r.contains(x, y));
		if self.prompt {
			return if on(&self.confirm) {
				Hit::Confirm
			} else if on(&self.cancel) {
				Hit::Cancel
			} else if on(&self.less) {
				Hit::Less
			} else if on(&self.more) {
				Hit::More
			} else {
				self.hint_at(x, y)
			};
		}
		if on(&self.buy) {
			return Hit::Buy;
		}
		if on(&self.refill) {
			return Hit::Refill;
		}
		self.rows.iter().find(|r| r.1.contains(x, y)).map_or_else(|| self.hint_at(x, y), |r| Hit::Row(r.0))
	}

	fn hint_at(&self, x: f32, y: f32) -> Hit {
		self.hints.iter().find(|h| h.1.contains(x, y)).map_or(Hit::None, |h| Hit::Hint(h.0))
	}
}

/// The shop's closing phase (`ShopView::phase`: 0 opening fade, 1 browse, 2 closing).
const CLOSING: u32 = 2;

/// What decides whether the modern page is the picture this frame.
pub struct Gate<'a> {
	pub modern: bool,
	/// The modern Options page is up.
	pub ui_open: bool,
	/// The UI pipes were refused (no GPU for them).
	pub refused: bool,
	/// The game is in the vendor screen (mode 5).
	pub vendor: bool,
	pub view: Option<&'a ShopView>,
}

/// The modern page replaces retail's picture from the opening fade on (phase 0, once the list reads) through the list phase; the closing
/// (phase 2) shows no page (`holds`).
pub fn wanted(g: &Gate) -> bool {
	holds(g) && g.view.is_some_and(|v| v.phase != CLOSING && !v.rows.is_empty())
}

/// The shop screen is up in the Modern style: while no page can be drawn (flying in before the list reads, or closing) the retail
/// overlay stays hidden behind an empty page, so no frame shows the classic shop [C: vendor_0093 close / reopen run 2026-11-02:
/// Triangle at field 30, list gone at 41, mode 5 -> 0 at 70; reopened mode 0 -> 5 at 137, list browsable at 174].
pub fn holds(g: &Gate) -> bool {
	g.modern && !g.ui_open && !g.refused && g.vendor
}

#[allow(clippy::too_many_arguments)]
pub(super) fn text(out: &mut Vec<Prim>, x: f32, y: f32, role: FontRole, px: f32, colour: Rgba, align: Align, max_w: f32, track: f32, s: &str) {
	out.push(Prim::Text(TextRun { x, y, role, px, colour, align, max_w, track, text: s.to_string() }));
}

pub(super) fn rrect(out: &mut Vec<Prim>, rect: Rect, radius: f32, fill: Rgba, border: Option<(f32, Rgba)>) {
	out.push(Prim::RRect { rect, radius, fill, border: border.map(|(width, colour)| Border { width, colour }) });
}

pub(super) fn key_name(sym: Symbol) -> &'static str {
	match sym {
		Symbol::Cross => keycaps::name(Cap::Enter),
		Symbol::Circle => "G",
		Symbol::Square => "F",
		Symbol::Triangle => keycaps::name(Cap::Esc),
	}
}

/// The width of a button's lead (a pad glyph or a keycap).
fn lead_w(f: &Facts, sym: Symbol, m: &dyn TextMeasure) -> f32 {
	if f.device == Device::Keyboard { (run_width(m, FontRole::SemiBold, 13.0, key_name(sym), 0.0) + 14.0).max(24.0) } else { 24.0 }
}

/// A pad glyph or a keycap of `sym` with its left edge at `x`, centred on `cy`.
fn lead(out: &mut Vec<Prim>, f: &Facts, sym: Symbol, x: f32, cy: f32, m: &dyn TextMeasure) -> f32 {
	let w = lead_w(f, sym, m);
	let rect = Rect::new(x, cy - 12.0, w, 24.0);
	if f.device == Device::Keyboard {
		rrect(out, rect, 6.0, CAP_BG, Some((1.0, CAP_BORDER)));
		text(out, rect.cx(), rect.cy(), FontRole::SemiBold, 13.0, TEXT, Align::Center, 0.0, 0.0, key_name(sym));
	} else {
		out.push(Prim::Glyph { rect, kind: f.glyphs.glyph(sym), colour: symbol_colour(sym) });
	}
	w
}

fn name_of<'a>(f: &'a Facts, i: usize) -> &'a str {
	f.names.get(i).map_or("", |s| s.as_str())
}

/// "Bolts", or "Bolt" for exactly one (per language).
fn bolts_word(f: &Facts, n: u32) -> &'static str {
	if n == 1 { word(&t::BOLT, f.lang) } else { word(&BOLTS, f.lang) }
}

fn header(out: &mut Vec<Prim>, f: &Facts, m: &dyn TextMeasure) {
	text(out, 48.0, 44.0, FontRole::SemiBold, 13.0, TEXT_MUTED, Align::Left, 400.0, 0.18, &upper(word(&t::HEADER, f.lang)));
	text(out, 48.0, 80.0, FontRole::Condensed, 34.0, TEXT, Align::Left, 700.0, 0.04, &upper(word(&t::TITLE, f.lang)));
	let n = commas(f.view.bolts);
	let w = 16.0 + 20.0 + 8.0 + run_width(m, FontRole::Condensed, 24.0, &n, 0.0) + 16.0;
	let r = Rect::new(1232.0 - w, 58.0, w, 44.0);
	rrect(out, r, 10.0, PANEL, Some((1.0, HAIRLINE)));
	rrect(out, Rect::new(r.x + 16.0, r.cy() - 10.0, 20.0, 20.0), 10.0, Rgba::rgba(0, 0, 0, 0.0), Some((2.0, TEXT_BODY)));
	rrect(out, Rect::new(r.x + 22.0, r.cy() - 4.0, 8.0, 8.0), 4.0, TEXT_BODY, None);
	text(out, r.x + 44.0, r.cy(), FontRole::Condensed, 24.0, TEXT, Align::Left, 0.0, 0.0, &n);
}

/// A chip: its word, text colour and background.
fn chip_of(f: &Facts, r: &ShopRow) -> (&'static str, Rgba, Rgba) {
	let bolts = f.view.bolts;
	if r.ammo {
		if r.full() {
			(word(&t::FULL, f.lang), TEXT_SECTION, Rgba::rgba(255, 255, 255, 0.06))
		} else if !r.can_buy(bolts) {
			(word(&t::NEED_BOLTS, f.lang), RED, RED.tint(0.12))
		} else {
			(word(&t::OWNED, f.lang), TEAL, TEAL.tint(0.14))
		}
	} else if r.can_buy(bolts) {
		(word(&t::NEW, f.lang), ACCENT, ACCENT.tint(0.16))
	} else {
		(word(&t::NEED_BOLTS, f.lang), RED, RED.tint(0.12))
	}
}

fn sub_of(f: &Facts, r: &ShopRow) -> String {
	let (bolts, each) = (bolts_word(f, r.price), word(&t::EACH, f.lang));
	if r.ammo {
		format!("{} {} / {} \u{b7} {} {} {}", word(&t::AMMO, f.lang), r.owned, r.max, commas(r.price), bolts, each)
	} else {
		format!("{} {}", commas(r.price), bolts)
	}
}

/// The Buy all ammo row can be chosen like a stock row (user 2026-11-03): the list browses, the page lists it, the exit is not pending.
fn choosable(v: &ShopView) -> bool {
	!v.prompt() && v.refill_on && !v.leaving
}

/// The Buy all ammo row at the top of the list (always there; user 2026-11-02): what Square buys now, or why it buys nothing, and the
/// Square lead while it would buy; the stock rows' highlight while it has the focus. Returns where the stock rows start.
fn buy_all_row(b: &mut Built, f: &Facts, m: &dyn TextMeasure) -> f32 {
	let (v, out) = (f.view, &mut b.prims);
	let rect = Rect::new(LIST.x + 12.0, LIST.y + 12.0, LIST.w - 24.0, ROW_H);
	if b.buyall {
		rrect(out, rect, 10.0, ACCENT.tint(0.13), Some((1.5, ACCENT.tint(0.6))));
	}
	let ready = !v.prompt() && v.can_refill();
	let ink = if ready { TEAL } else { TEXT_DISABLED };
	let icon = Rect::new(rect.x + 14.0, rect.cy() - 20.0, 40.0, 40.0);
	rrect(out, icon, 20.0, ink.tint(0.14), None);
	rrect(out, Rect::new(icon.cx() - 8.0, icon.cy() - 1.5, 16.0, 3.0), 1.5, ink, None);
	rrect(out, Rect::new(icon.cx() - 1.5, icon.cy() - 8.0, 3.0, 16.0), 1.5, ink, None);
	let (hungry, fill) = (v.rows.iter().any(|r| r.ammo && !r.full()), v.refill());
	let sub = match (hungry, fill.rounds) {
		(false, _) => word(&t::AMMO_FULL, f.lang).to_string(),
		(true, 0) => word(&t::NOT_ENOUGH, f.lang).to_string(),
		_ => format!("{} +{} \u{b7} {} {}", word(&t::AMMO, f.lang), fill.rounds, commas(fill.cost), bolts_word(f, fill.cost)),
	};
	let lw = if ready { lead_w(f, Symbol::Square, m) } else { 0.0 };
	if ready {
		lead(out, f, Symbol::Square, rect.right() - 14.0 - lw, rect.cy(), m);
	}
	let max_w = rect.right() - 14.0 - lw - 12.0 - (rect.x + 68.0);
	text(out, rect.x + 68.0, rect.y + 20.0, FontRole::SemiBold, 18.0, if ready { TEXT } else { TEXT_MUTED }, Align::Left, max_w, 0.0, word(&t::BUY_ALL, f.lang));
	text(out, rect.x + 68.0, rect.y + 41.0, FontRole::Regular, 14.0, TEXT_MUTED, Align::Left, max_w, 0.0, &sub);
	let line = rect.bottom() + 6.0;
	rrect(out, Rect::new(rect.x + 8.0, line, rect.w - 16.0, 1.0), 0.0, HAIRLINE, None);
	b.refill = choosable(v).then_some(rect);
	line + 7.0
}

/// A stock row's picture (v5i): the item's own menu icon from the level's HUD sheets (a blue weapon, a green gadget); the generic
/// ring when the level holds none.
fn row_icon(out: &mut Vec<Prim>, r: &ShopRow, icon: Rect, ink: Rgba) {
	let frame = if r.ammo { icons::FRAME_AMMO_ROW } else { icons::FRAME_MENU };
	if r.item < 0x100 && icons::draw(out, icons::item(r.item as u8, frame), icon, WHITE) {
		return;
	}
	rrect(out, icon, 20.0, Rgba::rgba(255, 255, 255, 0.07), None);
	rrect(out, icon.inset(9.0), 11.0, Rgba::rgba(0, 0, 0, 0.0), Some((2.2, ink)));
	rrect(out, icon.inset(15.5), 4.5, ink, None);
}

/// The first stock row shown: last frame's window (`prev`) moved as little as keeps row `sel` of `n` in view. A row chosen inside the
/// window never scrolls the list, so after the wheel walked the list down a hover keeps it there (user 2026-11-04: "when u scroll
/// the vendor with the mouse and then move the mouse it resets to the higher position": the window was the selection's, always with
/// the selection at its bottom row, so the hover chose the row above, the list moved under the pointer, and so on up to the top).
pub fn first_row(prev: usize, sel: usize, n: usize) -> usize {
	scroll_first(prev, sel, n, VISIBLE)
}

/// `first_row` for a list that shows `visible` rows (the shop's stock list, the Galactic Map's planets).
pub fn scroll_first(prev: usize, sel: usize, n: usize, visible: usize) -> usize {
	let first = prev.min(n.saturating_sub(visible));
	if sel < first {
		sel
	} else if sel >= first + visible {
		sel + 1 - visible
	} else {
		first
	}
}

fn list(b: &mut Built, f: &Facts, m: &dyn TextMeasure) {
	rrect(&mut b.prims, LIST, 16.0, PANEL, Some((1.0, HAIRLINE)));
	let top = buy_all_row(b, f, m);
	let out = &mut b.prims;
	let n = f.view.rows.len();
	// Scrolled to keep the selection in view; to the top while Buy all ammo has the focus.
	let first = if b.buyall { 0 } else { first_row(f.scroll, b.sel, n) };
	b.first = first;
	for (k, i) in (first..n.min(first + VISIBLE)).enumerate() {
		let r = &f.view.rows[i];
		let rect = Rect::new(LIST.x + 12.0, top + k as f32 * (ROW_H + ROW_GAP), LIST.w - 24.0, ROW_H);
		let on = !b.buyall && i == b.sel;
		if on {
			rrect(out, rect, 10.0, ACCENT.tint(0.13), Some((1.5, ACCENT.tint(0.6))));
		}
		let ink = if on { ACCENT } else { TEXT_BODY };
		let icon = Rect::new(rect.x + 14.0, rect.cy() - 20.0, 40.0, 40.0);
		row_icon(out, r, icon, ink);
		let (word, colour, fill) = chip_of(f, r);
		let w = run_width(m, FontRole::SemiBold, 13.0, word, 0.0) + 20.0;
		let chip = Rect::new(rect.right() - 14.0 - w, rect.cy() - 13.0, w, 26.0);
		rrect(out, chip, 13.0, fill, None);
		text(out, chip.cx(), chip.cy(), FontRole::SemiBold, 13.0, colour, Align::Center, 0.0, 0.0, word);
		let max_w = chip.x - 12.0 - (rect.x + 68.0);
		text(out, rect.x + 68.0, rect.y + 20.0, FontRole::SemiBold, 18.0, TEXT, Align::Left, max_w, 0.0, name_of(f, i));
		text(out, rect.x + 68.0, rect.y + 41.0, FontRole::Regular, 14.0, TEXT_MUTED, Align::Left, max_w, 0.0, &sub_of(f, r));
		b.rows.push((i, rect));
	}
}

/// The detail card's price line and buy button of row `r`.
fn buy_button(b: &mut Built, f: &Facts, m: &dyn TextMeasure, r: &ShopRow) {
	let out = &mut b.prims;
	let (x, w) = (DETAIL.x + 22.0, DETAIL.w - 44.0);
	let btn = Rect::new(x, DETAIL.bottom() - 22.0 - 46.0, w, 46.0);
	let bolts = f.view.bolts;
	let can = r.can_buy(bolts);
	let (price_label, price, price_red) = if r.ammo {
		let missing = r.max.saturating_sub(r.owned);
		if r.full() { (word(&t::AMMO_FULL, f.lang).to_string(), "\u{2014}".to_string(), false) } else { (fill(&t::FILL_UP, f.lang, &missing.to_string()), commas(missing * r.price), !can) }
	} else {
		(word(&t::PRICE, f.lang).to_string(), commas(r.price), !can)
	};
	price_line(out, btn, &price_label, &price, price_red);
	let label = match (r.ammo, can, r.full()) {
		(true, _, true) => word(&t::AMMO_FULL, f.lang),
		(_, false, _) => word(&t::NOT_ENOUGH, f.lang),
		(true, _, _) => word(&t::BUY_AMMO, f.lang),
		_ => word(&t::BUY_WEAPON, f.lang),
	};
	button(b, f, m, btn, label, can);
}

/// The price line above the detail card's button `btn`: its label on the left, the price on the right.
fn price_line(out: &mut Vec<Prim>, btn: Rect, label: &str, price: &str, red: bool) {
	let y = btn.y - 12.0 - 18.0;
	text(out, btn.x, y, FontRole::Regular, 14.0, TEXT_MUTED, Align::Left, 190.0, 0.0, label);
	text(out, btn.right(), y, FontRole::Condensed, 28.0, if red { RED } else { WHITE }, Align::Right, 130.0, 0.0, price);
}

/// The detail card's button: live (the accent with the Cross lead; a click is Buy) or disabled with the reason.
fn button(b: &mut Built, f: &Facts, m: &dyn TextMeasure, btn: Rect, label: &str, live: bool) {
	let out = &mut b.prims;
	if live {
		rrect(out, btn, 10.0, ACCENT, None);
		let tw = run_width(m, FontRole::SemiBold, 17.0, label, 0.0);
		let lw = lead_w(f, Symbol::Cross, m);
		let x0 = btn.cx() - (lw + 10.0 + tw) * 0.5;
		lead(out, f, Symbol::Cross, x0, btn.cy(), m); // the Cross glyph with a pad, the Enter keycap with the keyboard and mouse
		text(out, x0 + lw + 10.0, btn.cy(), FontRole::SemiBold, 17.0, INK_ON_ACCENT, Align::Left, btn.w - 24.0 - 10.0 - lw, 0.0, label);
		b.buy = Some(btn);
	} else {
		rrect(out, btn, 10.0, BUTTON_BG, None);
		text(out, btn.cx(), btn.cy(), FontRole::SemiBold, 17.0, TEXT_DISABLED, Align::Center, btn.w - 24.0, 0.0, label);
	}
}

/// `s` (Regular 14) over at most two lines of `w`, broken between words: the first line takes the words that fit, the second the rest
/// (squeezed by the renderer if still too wide).
fn two_lines(m: &dyn TextMeasure, s: &str, w: f32) -> Vec<String> {
	let fits = |t: &str| run_width(m, FontRole::Regular, 14.0, t, 0.0) <= w;
	let words: Vec<&str> = s.split(' ').collect();
	let mut k = 1;
	while k < words.len() && fits(&words[..k + 1].join(" ")) {
		k += 1;
	}
	[words[..k].join(" "), words[k..].join(" ")].into_iter().filter(|l| !l.is_empty()).collect()
}

/// The "Refills:" lines of the Buy all ammo card in two lines of `w` (Regular 14, no renderer squeeze or cut): every name when they
/// fit, else the first names and "+N more", at worst the count alone (a shop has up to 7 rows; R23 m2).
fn refill_lines(m: &dyn TextMeasure, lang: usize, names: &[&str], w: f32) -> Vec<String> {
	let n = names.len();
	let fits = |l: &[String]| l.iter().all(|s| run_width(m, FontRole::Regular, 14.0, s, 0.0) <= w);
	for k in (1..=n).rev() {
		let more = if k < n { format!(", {}", fill(&t::REFILLS_MORE, lang, &(n - k).to_string())) } else { String::new() };
		let l = two_lines(m, &fill(&t::REFILLS, lang, &format!("{}{more}", names[..k].join(", "))), w);
		if fits(&l) {
			return l;
		}
	}
	two_lines(m, &fill(&t::REFILLS, lang, &fill(&t::REFILLS_MORE, lang, &n.to_string())), w)
}

/// The detail card of the Buy all ammo row (it has the focus): what Square buys now (the ammo it fills, the rounds, the bolts) or why
/// it buys nothing, and a button that buys it (Cross on the row is Square for the game, `vendor_nav`; a click, `vendor_host`).
fn buyall_detail(b: &mut Built, f: &Facts, m: &dyn TextMeasure) {
	let v = f.view;
	let (x, w) = (DETAIL.x + 22.0, DETAIL.w - 44.0);
	let title = word(&t::BUY_ALL, f.lang);
	let mut px = 34.0;
	while px > 24.0 && run_width(m, FontRole::Condensed, px, title, 0.0) > w {
		px -= 2.0;
	}
	text(&mut b.prims, x, DETAIL.y + 38.0, FontRole::Condensed, px, WHITE, Align::Left, w, 0.0, title);
	let (rows, fill_up) = (v.refill_rows(), v.refill());
	let hungry = v.rows.iter().any(|r| r.ammo && !r.full());
	let lines: Vec<(String, Rgba)> = if !hungry {
		vec![(word(&t::AMMO_FULL, f.lang).to_string(), TEXT_DISABLED)]
	} else if rows.is_empty() {
		vec![(word(&t::NOT_ENOUGH, f.lang).to_string(), RED)]
	} else {
		let names: Vec<&str> = rows.iter().map(|&(i, _)| name_of(f, i)).collect();
		refill_lines(m, f.lang, &names, w).into_iter().map(|s| (s, TEXT_BODY)).collect()
	};
	for (k, (s, colour)) in lines.iter().enumerate() {
		text(&mut b.prims, x, DETAIL.y + 76.0 + 22.0 * k as f32, FontRole::Regular, 14.0, *colour, Align::Left, w, 0.0, s);
	}
	let btn = Rect::new(x, DETAIL.bottom() - 22.0 - 46.0, w, 46.0);
	let (label, price) = if fill_up.rounds > 0 {
		(format!("{} +{}", word(&t::AMMO, f.lang), fill_up.rounds), commas(fill_up.cost))
	} else {
		(word(&t::TOTAL, f.lang).to_string(), "\u{2014}".to_string())
	};
	price_line(&mut b.prims, btn, &label, &price, false);
	let ready = v.can_refill();
	let say = if ready { &t::BUY_ALL } else if hungry { &t::NOT_ENOUGH } else { &t::AMMO_FULL };
	button(b, f, m, btn, word(say, f.lang), ready);
}

fn detail(b: &mut Built, f: &Facts, m: &dyn TextMeasure) {
	let hole = vendor_portrait::hole(DETAIL);
	b.prims.extend(vendor_portrait::card(hole));
	b.portrait = Some(hole);
	if b.buyall {
		return buyall_detail(b, f, m);
	}
	let Some(r) = f.view.rows.get(b.sel) else { return };
	let (x, w) = (DETAIL.x + 22.0, DETAIL.w - 44.0);
	text(&mut b.prims, x, DETAIL.y + 38.0, FontRole::Condensed, 34.0, WHITE, Align::Left, w, 0.0, name_of(f, b.sel));
	if r.ammo {
		let y = DETAIL.y + 90.0;
		let out = &mut b.prims;
		text(out, x, y, FontRole::Regular, 14.0, TEXT_MUTED, Align::Left, 70.0, 0.0, word(&t::AMMO, f.lang));
		let bar = Rect::new(x + 78.0, y - 4.0, w - 78.0 - 76.0, 8.0);
		rrect(out, bar, 4.0, BAR_OFF, None);
		let frac = if r.max == 0 { 0.0 } else { (r.owned as f32 / r.max as f32).clamp(0.0, 1.0) };
		if frac > 0.0 {
			rrect(out, Rect::new(bar.x, bar.y, (bar.w * frac).max(8.0), 8.0), 4.0, ACCENT, None);
		}
		text(out, x + w, y, FontRole::SemiBold, 16.0, TEXT, Align::Right, 72.0, 0.0, &format!("{} / {}", r.owned, r.max));
	}
	buy_button(b, f, m, r);
}

/// The hint bar along the bottom: Buy, Leave (Cancel under the prompt). Buy all ammo is the list's first row.
fn hints(b: &mut Built, f: &Facts, m: &dyn TextMeasure) {
	let v = f.view;
	let items = [(Symbol::Cross, word(&t::BUY, f.lang).to_string()), (Symbol::Triangle, word(if v.prompt() { &t::CANCEL } else { &t::LEAVE }, f.lang).to_string())];
	let widths: Vec<f32> = items.iter().map(|(sym, s)| lead_w(f, *sym, m) + 8.0 + run_width(m, FontRole::Medium, 15.0, s, 0.0)).collect();
	let mut x = 1232.0 - widths.iter().sum::<f32>() - 26.0 * (items.len() - 1) as f32;
	let cy = 688.0;
	// Clickable except in the prompt's closing steps (3, 4), which are the game's alone.
	let clickable = !v.prompt() || v.stage == 2;
	for ((sym, s), w) in items.iter().zip(&widths) {
		if clickable {
			b.hints.push((*sym, Rect::new(x - 6.0, cy - 16.0, w + 12.0, 32.0)));
		}
		let lw = lead(&mut b.prims, f, *sym, x, cy, m);
		text(&mut b.prims, x + lw + 8.0, cy, FontRole::Medium, 15.0, TEXT_BODY, Align::Left, 0.0, 0.0, s);
		x += w + 26.0;
	}
}

fn prompt(b: &mut Built, f: &Facts, m: &dyn TextMeasure) {
	let Some(r) = f.view.rows.get(b.sel) else { return };
	let out = &mut b.prims;
	// Only the quantity prompt itself (stage 2) answers; the closing and resolving steps (3, 4) are the game's alone.
	let live = f.view.stage == 2;
	out.push(Prim::Backdrop { blur_px: BLUR_PEEK, scrim: Rgba::rgba(4, 6, 12, 0.62) });
	let h = if r.ammo { 258.0 } else { 206.0 };
	let d = Rect::new(400.0, 210.0, 480.0, h);
	rrect(out, d, 16.0, Rgba::rgba(14, 20, 34, 0.97), Some((1.0, Rgba::rgba(255, 255, 255, 0.10))));
	let (x, w) = (d.x + 26.0, d.w - 52.0);
	let title = fill(if r.ammo { &t::ASK_AMMO } else { &t::ASK_WEAPON }, f.lang, name_of(f, b.sel));
	text(out, x, d.y + 44.0, FontRole::Condensed, 30.0, TEXT, Align::Left, w, 0.0, &title);
	let mut y = d.y + 26.0 + 36.0 + 16.0;
	let qty = f.view.qty.max(1);
	if r.ammo {
		text(out, x, y + 18.0, FontRole::Regular, 16.0, TEXT_MUTED, Align::Left, 200.0, 0.0, word(&t::HOW_MANY, f.lang));
		let more = Rect::new(x + w - 36.0, y, 36.0, 36.0);
		let less = Rect::new(more.x - 10.0 - 54.0 - 10.0 - 36.0, y, 36.0, 36.0);
		for (rect, right) in [(less, false), (more, true)] {
			rrect(out, rect, 8.0, CHIP_BG, None);
			out.push(Prim::Chevron { rect: Rect::new(rect.cx() - 7.0, rect.cy() - 7.0, 14.0, 14.0), right, colour: CHEVRON });
		}
		text(out, less.right() + 10.0 + 27.0, y + 18.0, FontRole::Condensed, 28.0, WHITE, Align::Center, 54.0, 0.0, &qty.to_string());
		(b.less, b.more) = (live.then_some(less), live.then_some(more));
		y += 36.0 + 16.0;
	}
	rrect(out, Rect::new(x, y, w, 1.0), 0.0, HAIRLINE, None);
	let total = if r.ammo { qty * r.price } else { r.price };
	text(out, x, y + 24.0, FontRole::Regular, 15.0, TEXT_MUTED, Align::Left, 120.0, 0.0, word(&t::TOTAL, f.lang));
	text(out, x + w, y + 24.0, FontRole::Condensed, 28.0, WHITE, Align::Right, w - 130.0, 0.0, &format!("{} {}", commas(total), bolts_word(f, total)));
	let by = d.bottom() - 26.0 - 46.0;
	let half = (w - 12.0) * 0.5;
	let (ok, no) = (Rect::new(x, by, half, 46.0), Rect::new(x + half + 12.0, by, half, 46.0));
	rrect(out, ok, 10.0, ACCENT, None);
	rrect(out, no, 10.0, CHIP_BG, None);
	text(out, ok.cx(), ok.cy(), FontRole::SemiBold, 17.0, INK_ON_ACCENT, Align::Center, half - 16.0, 0.0, word(&t::BUY, f.lang));
	text(out, no.cx(), no.cy(), FontRole::SemiBold, 17.0, TEXT, Align::Center, half - 16.0, 0.0, word(&t::CANCEL, f.lang));
	(b.confirm, b.cancel) = (live.then_some(ok), live.then_some(no));
	let _ = m;
}

/// The page of `f`; None while the list is empty.
pub fn build(f: &Facts, m: &dyn TextMeasure) -> Option<Built> {
	let v = f.view;
	if v.rows.is_empty() {
		return None;
	}
	// The prompt names the game's own item (a mouse focus could be a moment old and the keys move the game's cursor).
	let sel = if v.prompt() { v.sel } else { f.focus.unwrap_or(v.sel) }.min(v.rows.len() - 1);
	let buyall = f.buyall && choosable(v);
	let mut b = Built { prims: Vec::new(), rows: Vec::new(), buy: None, refill: None, buyall, less: None, more: None, confirm: None, cancel: None, hints: Vec::new(), prompt: v.prompt(), sel, count: v.rows.len(), first: 0, portrait: None };
	header(&mut b.prims, f, m);
	list(&mut b, f, m);
	detail(&mut b, f, m);
	hints(&mut b, f, m);
	if b.prompt {
		prompt(&mut b, f, m);
	}
	Some(b)
}
