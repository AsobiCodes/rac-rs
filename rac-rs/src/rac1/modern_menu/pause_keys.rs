//! The modern Controls tab (v0.5.1 stage 2b): a real table, action | slot 1 | slot 2, the Controls sections as sub-headers, in one
//! scrolling list (not the classic 7-row pages). Pure data and cell logic here (`build`, `cell_effect`, `clear_effect`, the geometry
//! and the draw list); the input (cursor, scroll, capture start) is `pause_host/keys.rs`. A cell click or Enter captures that slot
//! (`kbm::Cap::Kbm`, the same machine as the classic page), Tab captures slot 2, Delete / Backspace / the pad's Square clear the cell
//! (Pause's last binding is refused, with a notice), Esc cancels a capture. The first row is a device switch "Keyboard & mouse |
//! Controller" (`view_pad`; opens on the device in use): the controller view lists the controller-bindable actions with their two
//! buttons (`kbm::Cap::Pad`, Xbox or PlayStation names), a line saying the sticks are fixed, and Reset controller; the keyboard view
//! ends with Reset keyboard. Words: the game's own where it has them (`kbm_actions.rs`).
use super::pause::Facts;
use crate::retail::menu_model::text::decode_bytes;
use crate::retail::{pcctl, pcctl_kbm};
use std::sync::atomic::{AtomicU8, Ordering};
use rac_engine::input::kbm::{self, names, Binds, Table};
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::input::{Device, GlyphSet};
use rac_engine::ui::kit::layout::{cap_w, KEY_PX};
use rac_engine::ui::kit::text::{run_width, upper, TextMeasure};
use rac_engine::ui::prim::{Align, Border, FontRole, Prim, Rect, Rgba, Symbol, TextRun};
use rac_engine::ui::theme::*;
use rac_engine::ui::T5;

// Not retail text (the table's own words; native-speaker review wanted).
pub const SLOT_HEAD: [T5; 3] = [
	["Action", "Action", "Aktion", "Acci\u{f3}n", "Azione"],
	["Primary", "Principal", "Erste", "Principal", "Principale"],
	["Secondary", "Secondaire", "Zweite", "Secundaria", "Secondaria"],
];
const SECTIONS: [T5; 9] = [
	["Actions", "Actions", "Aktionen", "Acciones", "Azioni"],
	["More buttons", "Autres boutons", "Weitere Tasten", "M\u{e1}s botones", "Altri pulsanti"],
	["Movement", "Mouvement", "Bewegung", "Movimiento", "Movimento"],
	["Camera", "Cam\u{e9}ra", "Kamera", "C\u{e1}mara", "Telecamera"],
	["D-pad", "Croix", "Steuerkreuz", "Cruceta", "Croce"],
	["PC extras", "Extras PC", "PC-Extras", "Extras de PC", "Extra PC"],
	["Controller", "Manette", "Controller", "Mando", "Joypad"],
	["Controller D-pad", "Croix de la manette", "Controller-Steuerkreuz", "Cruceta del mando", "Croce del joypad"],
	["Reset", "R\u{e9}initialiser", "Zur\u{fc}cksetzen", "Restaurar", "Ripristina"],
];
const PRESS_KEY: T5 = ["Press a key or mouse button", "Touche ou bouton souris", "Taste oder Maustaste", "Tecla o bot\u{f3}n del rat\u{f3}n", "Tasto o pulsante mouse"];
const PRESS_PAD: T5 = ["Press a button", "Appuyez sur un bouton", "Knopf dr\u{fc}cken", "Pulsa un bot\u{f3}n", "Premi un pulsante"];
pub const MOUSE_LABEL: [T5; 2] = [
	["Mouse up/down", "Souris haut/bas", "Maus hoch/runter", "Rat\u{f3}n arriba/abajo", "Mouse su/gi\u{f9}"],
	["Mouse left/right", "Souris gauche/droite", "Maus links/rechts", "Rat\u{f3}n izquierda/derecha", "Mouse sinistra/destra"],
];
pub const MOUSE_VALUE: [T5; 3] = [
	["Like camera", "Comme la cam\u{e9}ra", "Wie Kamera", "Como la c\u{e1}mara", "Come la telecamera"],
	["Normal", "Normal", "Normal", "Normal", "Normale"],
	["Inverted", "Invers\u{e9}", "Invertiert", "Invertido", "Invertito"],
];
pub const KEY2: T5 = ["2nd key", "2e touche", "2. Taste", "2.\u{aa} tecla", "2\u{ba} tasto"];
pub const CLEAR: T5 = ["Clear", "Effacer", "Leeren", "Borrar", "Cancella"];
pub const REFUSED: T5 = [
	"Pause must keep at least one key or button.",
	"Pause doit garder au moins une touche ou un bouton.",
	"Pause muss mindestens eine Taste oder einen Knopf behalten.",
	"Pausa debe conservar al menos una tecla o bot\u{f3}n.",
	"Pausa deve mantenere almeno un tasto o pulsante.",
];
const DEVICE: [T5; 3] = [
	["Device", "Appareil", "Ger\u{e4}t", "Dispositivo", "Dispositivo"],
	["Keyboard & mouse", "Clavier et souris", "Tastatur & Maus", "Teclado y rat\u{f3}n", "Tastiera e mouse"],
	["Controller", "Manette", "Controller", "Mando", "Joypad"],
];
const STICKS: T5 = [
	"Sticks: left = move, right = camera (fixed)",
	"Sticks : gauche = d\u{e9}placement, droit = cam\u{e9}ra (fixes)",
	"Sticks: links = Bewegung, rechts = Kamera (fest)",
	"Sticks: izq. = mover, der. = c\u{e1}mara (fijos)",
	"Stick: sinistro = movimento, destro = camera (fissi)",
];

/// Which device the table shows: 0 keyboard + mouse, 1 controller, 2 not chosen yet (the device in use).
static VIEW: AtomicU8 = AtomicU8::new(2);

/// Whether the table shows the controller (`in_use` = the device the player uses when none was chosen). The first build of a visit
/// pins the device in use, so a key touched later does not swap the rows under the focus (until the page closes: `set_view(None)`).
pub fn view_pad(in_use: Device) -> bool {
	match VIEW.load(Ordering::Relaxed) {
		0 => false,
		1 => true,
		_ => {
			let pad = in_use == Device::Pad;
			VIEW.store(u8::from(pad), Ordering::Relaxed);
			pad
		}
	}
}

pub fn set_view(pad: Option<bool>) {
	VIEW.store(pad.map_or(2, u8::from), Ordering::Relaxed);
}

/// One row of the table.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
	Head(String),
	/// A keyboard + mouse action (table index) with the two slots' texts; `shared` = another action of the layer has the input too.
	/// `pad`: the slots are controller buttons.
	Action { a: usize, pad: bool, label: String, slots: [String; 2], shared: [bool; 2] },
	/// The device switch (false keyboard + mouse, true controller).
	Device { pad: bool, label: String, value: String },
	/// Reset keyboard (false) / Reset controller (true).
	Reset { pad: bool, label: String },
	/// A mouse invert row (`pcmouseinv.rs`): axis 0 up/down, 1 left/right, mode 0 Like camera / 1 Normal / 2 Inverted.
	Mouse { axis: usize, mode: u8, label: String, value: String },
}

impl Row {
	pub fn selectable(&self) -> bool {
		!matches!(self, Row::Head(_))
	}
}

/// The table as built.
#[derive(Clone, Debug, PartialEq)]
pub struct KeysBuilt {
	pub caption: String,
	pub cols: [String; 3],
	pub rows: Vec<Row>,
	pub lang: usize,
	pub device: Device,
	pub glyphs: GlyphSet,
}

fn game_text(b: &[u8]) -> String {
	decode_bytes(b).plain().trim().to_string()
}

/// A slot's text: the input's short name, "-" for an empty slot.
fn slot_text(i: Option<kbm::Input>) -> String {
	i.map_or_else(|| "\u{2014}".to_string(), names::short_name)
}

/// The rows of the table for these bindings (pure: the table, the bindings and the controller values are arguments); `pad` = the
/// controller view.
pub fn rows_of(t: &Table, b: &Binds, lang: usize, ps: bool, pad: bool) -> Vec<Row> {
	let l = lang.min(4);
	let mut rows = vec![Row::Device { pad, label: DEVICE[0][l].to_string(), value: DEVICE[if pad { 2 } else { 1 }][l].to_string() }];
	let head = |rows: &mut Vec<Row>, s: &str| rows.push(Row::Head(s.to_string()));
	let sections = t.actions.iter().map(|a| a.section as usize + 1).max().unwrap_or(0);
	for sec in 0..sections {
		let acts: Vec<usize> = (0..t.actions.len()).filter(|&a| t.actions[a].section as usize == sec && (!pad || t.actions[a].pad_ok)).collect();
		if acts.is_empty() {
			continue;
		}
		head(&mut rows, SECTIONS[sec.min(5)][l]);
		for a in acts {
			let (slots, shared) = if pad {
				let s = b.1.get(a).copied().unwrap_or([None, None]);
				let others = b.shared_pad(t, a);
				let n = |p: Option<u8>| p.map_or_else(|| "\u{2014}".to_string(), |p| rac_engine::input::binds::pad_shown(p, ps).to_string());
				([n(s[0]), n(s[1])], [0, 1].map(|k| s[k].is_some_and(|p| others.iter().any(|&o| b.has_pad(o, p)))))
			} else {
				let s = b.0.get(a).copied().unwrap_or([None, None]);
				let others = b.shared(t, a);
				([slot_text(s[0]), slot_text(s[1])], [0, 1].map(|k| s[k].is_some_and(|i| others.iter().any(|&o| b.has(o, i)))))
			};
			rows.push(Row::Action { a, pad, label: game_text(&pcctl_kbm::label(a, l)), slots, shared });
		}
	}
	if pad {
		head(&mut rows, STICKS[l]);
	} else {
		// The PC section's last rows: the mouse's own invert (Like camera, Normal, Inverted).
		for axis in 0..2 {
			let mode = crate::retail::pcmouseinv::mode(axis);
			rows.push(Row::Mouse { axis, mode, label: MOUSE_LABEL[axis][l].to_string(), value: MOUSE_VALUE[mode as usize][l].to_string() });
		}
	}
	head(&mut rows, SECTIONS[8][l]);
	rows.push(Row::Reset { pad, label: game_text(&pcctl::reset_label(pad, lang)) });
	rows
}

/// The table for the page facts (the live bindings).
pub(super) fn build(f: &Facts) -> KeysBuilt {
	super::super::kbm_actions::install();
	let l = f.lang.min(4);
	let caption = game_text(&pcctl::text_now(pcctl::ID_CONTROLS, l));
	KeysBuilt {
		caption,
		cols: [0, 1, 2].map(|i| SLOT_HEAD[i][l].to_string()),
		rows: rows_of(kbm::table(), &kbm::binds(), f.lang, pcctl::playstation_names(), view_pad(f.device)),
		lang: l,
		device: f.device,
		glyphs: f.glyphs,
	}
}

// ---- cell logic --------------------------------------------------------------------------------------------------------------

/// What choosing a cell does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
	None,
	Capture(kbm::Cap),
	/// Reset keyboard (false) or controller (true).
	Reset(bool),
	/// The device switch: show the keyboard (false) or the controller (true).
	View(bool),
	/// Set the mouse invert row `axis` to the mode (the next one in the cycle).
	Mouse(usize, u8),
}

/// Enter / click on `slot` of `row`.
pub fn cell_effect(row: &Row, slot: usize) -> Effect {
	match row {
		Row::Head(_) => Effect::None,
		Row::Action { a, pad: false, .. } => Effect::Capture(kbm::Cap::Kbm(*a, slot.min(1))),
		Row::Action { a, pad: true, .. } => Effect::Capture(kbm::Cap::Pad(*a, slot.min(1))),
		Row::Device { pad, .. } => Effect::View(!*pad),
		Row::Reset { pad, .. } => Effect::Reset(*pad),
		Row::Mouse { axis, mode, .. } => Effect::Mouse(*axis, (*mode + 1) % 3),
	}
}

/// What Delete does to a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clear {
	/// Nothing to clear (empty slot, a header, a reset row).
	Nothing,
	Cleared,
	/// Pause's last binding.
	Refused,
}

/// The result of clearing `slot` of `row` with these bindings (no change is made).
pub fn clear_effect(t: &Table, b: &Binds, row: &Row, slot: usize) -> Clear {
	match row {
		Row::Action { a, pad, .. } => {
			let k = slot.min(1);
			let has = if *pad { b.1.get(*a).is_some_and(|s| s[k].is_some()) } else { b.0.get(*a).is_some_and(|s| s[k].is_some()) };
			if !has {
				return Clear::Nothing;
			}
			let mut c = *b;
			if if *pad { c.clear_pad(t, *a, k) } else { c.clear(t, *a, k) } { Clear::Cleared } else { Clear::Refused }
		}
		_ => Clear::Nothing,
	}
}

/// The focus of the table: the row, the slot (0 / 1), the first visible row and the notice (text, since ms).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ui {
	pub row: usize,
	pub slot: usize,
	pub top: usize,
	pub notice: Option<(Clear, u64)>,
	/// The mouse invert mode just asked for per axis (mode, ms): the setting is applied by the worker a field later, so a second quick
	/// press steps from this and not from the old published mode.
	pub mouse_pending: [Option<(u8, u64)>; 2],
}

/// Notice lifetime (ms).
pub const NOTICE_MS: u64 = 3500;

impl Ui {
	/// Moves the row focus by `dir` over the selectable rows (wrapping), keeping it in view.
	pub fn step(&mut self, rows: &[Row], dir: isize) {
		let n = rows.len() as isize;
		if n == 0 {
			return;
		}
		for k in 1..=n {
			let i = (self.row as isize + dir * k).rem_euclid(n) as usize;
			if rows[i].selectable() {
				self.row = i;
				break;
			}
		}
		self.follow(rows);
	}

	/// Focus row `i` (a hover): selectable rows only.
	pub fn focus(&mut self, rows: &[Row], i: usize, slot: usize) {
		if rows.get(i).is_some_and(Row::selectable) {
			(self.row, self.slot) = (i, slot.min(1));
			self.follow(rows);
		}
	}

	/// Keeps the focused row (and its header, when it is the first row of its section) in the window; the first selectable row on first use.
	pub fn follow(&mut self, rows: &[Row]) {
		if rows.is_empty() {
			return;
		}
		self.row = self.row.min(rows.len() - 1);
		if !rows[self.row].selectable() {
			self.row = (self.row..rows.len()).find(|&i| rows[i].selectable()).unwrap_or(0);
		}
		let want = if self.row > 0 && !rows[self.row - 1].selectable() { self.row - 1 } else { self.row };
		if want < self.top {
			self.top = want;
		}
		if self.row >= self.top + VISIBLE {
			self.top = self.row + 1 - VISIBLE;
		}
		self.top = self.top.min(rows.len().saturating_sub(VISIBLE));
	}
}

// ---- geometry and drawing (design space 1280 x 720) ----------------------------------------------------------------------------

pub const PX: f32 = 48.0;
pub const PY: f32 = 116.0;
pub const PW: f32 = 776.0;
pub const PH: f32 = 540.0;
const LIST_Y: f32 = 182.0;
pub const RH: f32 = 34.0;
/// Rows in the window.
pub const VISIBLE: usize = 13;
const LABEL_X: f32 = 72.0;
const CELL_W: f32 = 232.0;
const CELL1_X: f32 = 324.0;
const CELL2_X: f32 = 564.0;
pub const HINT_Y: f32 = 688.0;

/// The cell rectangle of `slot` in the window's row `k`.
pub fn cell_rect(k: usize, slot: usize) -> Rect {
	Rect::new(if slot == 0 { CELL1_X } else { CELL2_X }, LIST_Y + k as f32 * RH + 3.0, CELL_W, RH - 6.0)
}

/// The (row, slot) under design point (x, y), when it is a selectable row (a click on a pad or reset row's value area is slot 0).
pub fn hit(rows: &[Row], top: usize, x: f32, y: f32) -> Option<(usize, usize)> {
	if !(PX..PX + PW).contains(&x) || y < LIST_Y || y >= LIST_Y + VISIBLE as f32 * RH {
		return None;
	}
	let k = ((y - LIST_Y) / RH) as usize;
	let i = top + k;
	if !rows.get(i)?.selectable() {
		return None;
	}
	Some((i, usize::from(x >= CELL2_X - 4.0 && matches!(rows[i], Row::Action { .. }))))
}

fn text(out: &mut Vec<Prim>, x: f32, y: f32, role: FontRole, px: f32, colour: Rgba, align: Align, max_w: f32, track: f32, s: &str) {
	out.push(Prim::Text(TextRun { x, y, role, px, colour, align, max_w, track, text: s.to_string() }));
}

fn rrect(out: &mut Vec<Prim>, rect: Rect, radius: f32, fill: Rgba, border: Option<(f32, Rgba)>) {
	out.push(Prim::RRect { rect, radius, fill, border: border.map(|(width, colour)| Border { width, colour }) });
}

/// A keycap or a pad glyph, then its word; returns the x after it.
fn chip(out: &mut Vec<Prim>, m: &dyn TextMeasure, x: f32, cap: Option<&str>, glyph: Option<(Symbol, GlyphSet)>, word_: &str) -> f32 {
	let cy = HINT_Y;
	let mut x = x;
	if let Some(c) = cap {
		let w = cap_w(m, c, 0.0);
		let r = Rect::new(x, cy - 11.0, w, 22.0);
		rrect(out, r, 6.0, CAP_BG, Some((1.0, CAP_BORDER)));
		text(out, r.cx(), r.cy(), FontRole::SemiBold, KEY_PX, TEXT, Align::Center, 0.0, 0.0, c);
		x += w;
	}
	if let Some((s, g)) = glyph {
		out.push(Prim::Glyph { rect: Rect::new(x, cy - 12.0, 24.0, 24.0), kind: g.glyph(s), colour: symbol_colour(s) });
		x += 24.0;
	}
	text(out, x + 8.0, cy, FontRole::Medium, 15.0, TEXT_BODY, Align::Left, 0.0, 0.0, word_);
	x + 8.0 + run_width(m, FontRole::Medium, 15.0, word_, 0.0) + 26.0
}

/// The hint chips at the left of the hint bar: keyboard Tab = 2nd key, Del = clear; pad Square = clear (the engine draws Select / Back).
fn chips(out: &mut Vec<Prim>, k: &KeysBuilt, m: &dyn TextMeasure) {
	let l = k.lang;
	let mut x = 48.0;
	match k.device {
		Device::Keyboard => {
			x = chip(out, m, x, Some("Tab"), None, KEY2[l]);
			chip(out, m, x, Some("Del"), None, CLEAR[l]);
		}
		Device::Pad => {
			chip(out, m, x, None, Some((Symbol::Square, k.glyphs)), CLEAR[l]);
		}
	}
}

/// What the live capture state says about a row's cell: Some(slot) (0 for a pad / action's slot) when the cell is waiting.
fn waiting(row: &Row, cap: kbm::Cap) -> Option<usize> {
	match (row, cap) {
		(Row::Action { a, pad: false, .. }, kbm::Cap::Kbm(c, s)) | (Row::Action { a, pad: true, .. }, kbm::Cap::Pad(c, s)) if *a == c => Some(s),
		_ => None,
	}
}

/// The draw list of the table over the page (the pause shell's list position), for the focus `ui` and the capture state `cap`.
pub fn draw(k: &KeysBuilt, ui: &Ui, cap: kbm::Cap, now: u64, mut out: Vec<Prim>) -> Vec<Prim> {
	let m = &FontMeasure;
	let l = k.lang;
	rrect(&mut out, Rect::new(PX, PY, PW, PH), 16.0, PANEL, Some((1.0, HAIRLINE)));
	text(&mut out, PX + 20.0, PY + 30.0, FontRole::Condensed, 15.0, TEXT_SECTION, Align::Left, 0.0, 0.16, &upper(&k.caption));
	let hy = LIST_Y - 14.0;
	text(&mut out, LABEL_X, hy, FontRole::Regular, 12.0, TEXT_DIM, Align::Left, 0.0, 0.1, &upper(&k.cols[0]));
	text(&mut out, CELL1_X + 12.0, hy, FontRole::Regular, 12.0, TEXT_DIM, Align::Left, 0.0, 0.1, &upper(&k.cols[1]));
	text(&mut out, CELL2_X + 12.0, hy, FontRole::Regular, 12.0, TEXT_DIM, Align::Left, 0.0, 0.1, &upper(&k.cols[2]));
	out.push(Prim::ClipPush { rect: Rect::new(PX + 8.0, LIST_Y, PW - 16.0, VISIBLE as f32 * RH), radius: 0.0 });
	for (kk, row) in k.rows.iter().skip(ui.top).take(VISIBLE).enumerate() {
		let i = ui.top + kk;
		let y = LIST_Y + kk as f32 * RH;
		let cy = y + RH * 0.5;
		let sel = i == ui.row;
		match row {
			Row::Head(h) => {
				text(&mut out, LABEL_X, cy + 6.0, FontRole::Condensed, 14.0, TEXT_SECTION, Align::Left, 0.0, 0.12, &upper(h));
				rrect(&mut out, Rect::new(LABEL_X, y + RH - 3.0, PW - 2.0 * (LABEL_X - PX), 1.0), 0.0, HAIRLINE, None);
			}
			_ => {
				if sel {
					rrect(&mut out, Rect::new(PX + 12.0, y + 1.0, PW - 34.0, RH - 2.0), 10.0, ACCENT.tint(0.13), Some((1.5, ACCENT.tint(0.6))));
				}
				let label = match row {
					Row::Action { label, .. } | Row::Device { label, .. } | Row::Reset { label, .. } | Row::Mouse { label, .. } => label,
					Row::Head(h) => h,
				};
				text(&mut out, LABEL_X, cy, FontRole::Medium, 17.0, if sel { WHITE } else { TEXT }, Align::Left, CELL1_X - LABEL_X - 12.0, 0.0, label);
				let slots = if matches!(row, Row::Action { .. }) { 2 } else if matches!(row, Row::Device { .. } | Row::Mouse { .. }) { 1 } else { 0 };
				for s in 0..slots {
					let r = cell_rect(kk, s);
					let wait = waiting(row, cap) == Some(s);
					let focus = sel && (ui.slot == s || slots == 1);
					let border = if wait { (1.5, ACCENT) } else if focus { (1.5, ACCENT.tint(0.8)) } else { (1.0, HAIRLINE) };
					rrect(&mut out, r, 8.0, if wait { ACCENT.tint(0.16) } else { WHITE.tint(if focus { 0.10 } else { 0.05 }) }, Some(border));
					let (s_text, colour) = match row {
						_ if wait => (if matches!(row, Row::Action { pad: true, .. }) { PRESS_PAD[l] } else { PRESS_KEY[l] }.to_string(), ACCENT),
						Row::Action { slots: sl, shared, .. } => {
							let empty = sl[s] == "\u{2014}";
							(if shared[s] { format!("{}  \u{d7}2", sl[s]) } else { sl[s].clone() }, if empty { TEXT_DISABLED } else if shared[s] { TEXT_BOX } else { TEXT })
						}
						Row::Device { value, .. } | Row::Mouse { value, .. } => (value.clone(), TEXT),
						_ => (String::new(), TEXT),
					};
					text(&mut out, r.x + 12.0, r.cy(), FontRole::SemiBold, 15.0, colour, Align::Left, CELL_W - 20.0, 0.0, &s_text);
				}
			}
		}
	}
	out.push(Prim::ClipPop);
	if k.rows.len() > VISIBLE {
		let track = Rect::new(PX + PW - 14.0, LIST_Y, 4.0, VISIBLE as f32 * RH);
		rrect(&mut out, track, 2.0, WHITE.tint(0.08), None);
		let th = (track.h * VISIBLE as f32 / k.rows.len() as f32).max(24.0);
		let ty = track.y + (track.h - th) * ui.top as f32 / (k.rows.len() - VISIBLE) as f32;
		rrect(&mut out, Rect::new(track.x, ty, 4.0, th), 2.0, SCROLLBAR, None);
	}
	if let Some((c, at)) = ui.notice.filter(|n| now.saturating_sub(n.1) < NOTICE_MS) {
		let msg = match c {
			Clear::Refused => REFUSED[l],
			_ => "",
		};
		if !msg.is_empty() {
			rrect(&mut out, Rect::new(PX + 20.0, LIST_Y + VISIBLE as f32 * RH + 4.0, PW - 40.0, 26.0), 9.0, Rgba::rgba(242, 139, 130, 0.16), Some((1.0, SYMBOL_CIRCLE)));
			text(&mut out, PX + 36.0, LIST_Y + VISIBLE as f32 * RH + 17.0, FontRole::Medium, 14.0, TEXT, Align::Left, PW - 72.0, 0.0, msg);
		}
	}
	chips(&mut out, k, m);
	out
}
