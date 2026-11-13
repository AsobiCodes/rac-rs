//! The modern Galactic Map of the ship (v0.5 screens stage F2, the approved look `MOCKUP_ship.dc.html`): `build` turns the planet list
//! the game shows (`menu_model::view::PageView`, page 0xf) into a draw list in the 1280x720 design space plus what a click on a row or a
//! marker does. Pure: no statics, no clock; the main-thread host is `ship_host.rs`, the glue is `host/handler/frame_ship.rs`. The game
//! stays in charge: choosing a planet is a row request to the retail list, opening the details (page 0x10) and blasting off are the
//! game's own Cross presses; nothing here writes game state. List stage (page 0xf): the galaxy painting (`ship_marks.rs`, laid under
//! the page by `ship_host::place_art`) with a marker per planet, the selected one ringed, the list of planets on the left and the
//! selected planet on the right. Details stage (page 0x10): the same painting (the mockup's one look; the world behind it shows only a
//! dark room), no list, no markers: only the right panel with its badge. Planets the game greys out (its enabled word is
//! 0) show "????", a hollow ring, and are never a target. Only facts the game keeps are shown (its location and planet texts, "you are
//! here" = the level the ship is over; the gold bolts found and the visited flag come from the worker's `extra.ship`, `ShipFacts`).
use super::pause_grid::holed_panel;
use super::ship_anim::{clock_s, pulse, travel};
use super::ship_marks::{gold_total, mark, planet_of};
use super::ship_pic;
use super::title::{word, BACK, SELECT};
use super::vendor::{key_name, rrect, text};
use super::vendor_text::fill;
use crate::retail::menu_keys::{GALAXY, GALAXY_CHILD};
use crate::retail::menu_model::extra::ShipFacts;
use crate::retail::menu_model::text::Btn;
use crate::retail::menu_model::view::{Body, PageView};
use rac_engine::ui::input::{Device, GlyphSet};
use rac_engine::ui::kit::text::{run_width, upper};
use rac_engine::ui::kit::TextMeasure;
use rac_engine::ui::prim::{Align, Border, FontRole, Prim, Rect, Rgba, Symbol};
use rac_engine::ui::theme::*;
use rac_engine::ui::T5;

// Not retail text: the words of the page that the disc does not have. Native-speaker review wanted.
pub const HEADER: T5 = ["Ship", "Vaisseau", "Schiff", "Nave", "Nave"];
pub const TITLE: T5 = ["Galactic map", "Carte galactique", "Galaxiekarte", "Mapa gal\u{e1}ctico", "Mappa galattica"];
pub const HERE: T5 = ["You are here", "Vous \u{ea}tes ici", "Du bist hier", "Est\u{e1}s aqu\u{ed}", "Sei qui"];
pub const BLAST: T5 = ["Blast off to {}", "D\u{e9}coller vers {}", "Abflug nach {}", "Despegar hacia {}", "Decolla per {}"];
pub const BLAST_OFF: T5 = ["Blast off", "D\u{e9}coller", "Abflug", "Despegar", "Decolla"];
pub const LAND: T5 = ["Land on {}", "Atterrir sur {}", "Auf {} landen", "Aterrizar en {}", "Atterra su {}"];
pub const FOUND: T5 = ["{} of {} planets found", "{} sur {} plan\u{e8}tes trouv\u{e9}es", "{} von {} Planeten gefunden", "{} de {} planetas encontrados", "{} pianeti su {} trovati"];
pub const GOLD: T5 = ["Gold bolts found", "Boulons d'or trouv\u{e9}s", "Goldbolzen gefunden", "Tornillos de oro hallados", "Bulloni d'oro trovati"];
pub const OF: T5 = ["{} of {}", "{} sur {}", "{} von {}", "{} de {}", "{} su {}"];
pub const STATUS: T5 = ["Status", "\u{c9}tat", "Status", "Estado", "Stato"];
pub const VISITED: T5 = ["Visited", "Visit\u{e9}e", "Besucht", "Visitado", "Visitato"];
pub const NOT_VISITED: T5 = ["Not visited yet", "Pas encore visit\u{e9}e", "Noch nicht besucht", "A\u{fa}n sin visitar", "Non ancora visitato"];

/// Retail's text ids on these pages [C: level_transition.md, ingame_pause_menu.md].
const T_TITLE: u32 = 0x4ef0;
const T_SELECT: u32 = 0x4f05;
const T_EXIT: u32 = 0x4ee0;
const T_INFOBOT: u32 = 0x4eef;
const T_BLAST: u32 = 0x4f71;
const T_UNKNOWN: u32 = 0x4f54;

pub const LIST: Rect = Rect::new(48.0, 116.0, 400.0, 540.0);
pub const DETAIL: Rect = Rect::new(872.0, 116.0, 360.0, 540.0);
/// The card at the top of the right panel: the planet picture (List stage) or the badge (Details stage).
pub const CARD: Rect = Rect::new(DETAIL.x + 22.0, DETAIL.y + 20.0, DETAIL.w - 44.0, 150.0);
const ROW_H: f32 = 52.0;
const ROW_GAP: f32 = 4.0;
/// Rows the list shows at once (more scroll to keep the selection in view).
const VISIBLE: usize = 9;
/// The hit square of a marker.
const MARK_HIT: f32 = 22.0;
const TEAL: Rgba = SYMBOL_TRIANGLE;
const STAR: Rgba = Rgba::rgb(0xCFE3FF);
const LOCKED: Rgba = Rgba::rgb(0x5E6A7E);
const CLEAR: Rgba = Rgba::rgba(0, 0, 0, 0.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
	/// Page 0xf: the planet list.
	List,
	/// Page 0x10: the chosen planet.
	Details,
}

/// The stage of a page id; None = not a ship page.
pub fn stage_of(page: u32) -> Option<Stage> {
	match page {
		GALAXY => Some(Stage::List),
		GALAXY_CHILD => Some(Stage::Details),
		_ => None,
	}
}

/// One planet of the game's list.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
	/// The planet id (the level number) from the row's location text; None = the row names no planet this table knows.
	pub planet: Option<u32>,
	pub location: String,
	pub name: String,
	pub enabled: bool,
}

/// The planet list as the game shows it on page 0xf.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Listing {
	/// The list item's address (the target of a row request).
	pub item: u32,
	pub rows: Vec<Row>,
	/// The game's cursor row.
	pub sel: usize,
}

/// The planet list of the page on screen: the first list item whose rows name planets; a greyed row shows only "????" (the game's own
/// text), so an undiscovered planet's place is never spelled out.
pub fn listing_of(v: &PageView, t: &dyn Fn(u32) -> String) -> Option<Listing> {
	let (item, rows, cursor) = v.items.iter().find_map(|it| match &it.body {
		Body::Rows { rows, cursor } if rows.iter().any(|r| planet_of(r.text_id as u32).is_some()) => Some((it.addr, rows, *cursor)),
		_ => None,
	})?;
	let unknown = Some(t(T_UNKNOWN)).filter(|s| !s.is_empty()).unwrap_or_else(|| "????".to_string());
	let rows: Vec<Row> = rows
		.iter()
		.map(|r| {
			let planet = planet_of(r.text_id as u32);
			if !r.enabled {
				return Row { planet, location: String::new(), name: unknown.clone(), enabled: false };
			}
			let name = r.name.as_ref().map(|n| n.plain().trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| unknown.clone());
			Row { planet, location: r.text.plain().trim().to_string(), name, enabled: true }
		})
		.collect();
	let sel = cursor.min(rows.len().saturating_sub(1));
	Some(Listing { item, rows, sel })
}

/// Does the page list a Circle button (the game's "Play Infobot")? Only then the hint bar names it.
pub fn infobot_of(v: &PageView) -> bool {
	v.items.iter().any(|it| matches!(&it.body, Body::Hint { button: Btn::Circle, .. }))
}

/// Everything `build` reads.
pub struct Facts<'a> {
	pub list: &'a Listing,
	pub stage: Stage,
	/// The row the player's mouse chose a moment ago (shown at once, before retail's cursor follows).
	pub focus: Option<usize>,
	/// The first planet row the list showed last frame (`Built::first`; 0 for a fresh page).
	pub scroll: usize,
	/// The level the ship is over (a planet row with this id is "you are here").
	pub here: u32,
	/// The planets' visited flags and gold bolts found (what the game's details page shows); empty = not read, the lines are left out.
	pub planets: &'a ShipFacts,
	/// The page lists the Circle button.
	pub infobot: bool,
	/// A retail text id as plain text in the player's language (empty = unknown).
	pub text: &'a dyn Fn(u32) -> String,
	pub lang: usize,
	pub device: Device,
	pub glyphs: GlyphSet,
}

/// What a click at a point does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
	None,
	Row(usize),
	/// A button of the page: the big accent button or a hint-bar entry, named by the pad button the game answers (Cross = open the
	/// details / blast off, Triangle = leave, Circle = Infobot).
	Act(Symbol),
}

/// The page and what its parts do.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
	pub prims: Vec<Prim>,
	/// The visible list rows the game would let the player choose: (row index, rect). Greyed rows are not here.
	pub rows: Vec<(usize, Rect)>,
	/// The markers of the choosable planets on the painting: (row index, hit square).
	pub marks: Vec<(usize, Rect)>,
	/// Per row of the list: can it be chosen?
	pub enabled: Vec<bool>,
	pub sel: usize,
	pub count: usize,
	/// The first planet row shown (the next frame's `Facts::scroll`).
	pub first: usize,
	pub stage: Stage,
	/// The list item's address.
	pub item: u32,
	/// The galaxy painting goes under the page (both stages).
	pub art: bool,
	/// The hole in the right panel's card (design space) where the host puts the game's own planet picture, a crop of the retail
	/// overlay (`ship_pic`, `ship_host::place_planet`); None on the details stage (discs) and while the page flies in.
	pub planet: Option<Rect>,
	/// The clickable buttons: the accent button (Cross) and the hint bar's entries, each with the pad button it stands for.
	pub acts: Vec<(Symbol, Rect)>,
}

impl Built {
	/// What a click at design point (x, y) does: a marker or a row of the list stage, else a button (the accent button, the hint bar).
	pub fn hit(&self, x: f32, y: f32) -> Hit {
		if self.stage == Stage::List {
			if let Some(r) = self.marks.iter().chain(&self.rows).find(|r| r.1.contains(x, y)) {
				return Hit::Row(r.0);
			}
		}
		self.acts.iter().find(|a| a.1.contains(x, y)).map_or(Hit::None, |a| Hit::Act(a.0))
	}
}

/// What decides whether the modern page is the picture this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gate {
	pub modern: bool,
	/// The modern Options page is up.
	pub ui_open: bool,
	/// The UI pipes were refused (no GPU for them).
	pub refused: bool,
	/// The in-level page on screen, or the one flying in (`Retail::level_menu`; None = no page menu).
	pub page: Option<u32>,
}

/// The one rule: the modern page is the picture on page 0xf / 0x10, also while it flies in (so the retail picture never shows for the
/// 12 fields of the opening or of 0xf -> 0x10); a closing menu has no page and is kept hidden by the host (`frame_ship`).
pub fn wanted(g: &Gate) -> bool {
	g.modern && !g.ui_open && !g.refused && g.page.and_then(stage_of).is_some()
}

/// The word of retail text `id`, else `fallback`.
fn label(f: &Facts, id: u32, fallback: &T5) -> String {
	Some((f.text)(id)).filter(|s| !s.is_empty()).unwrap_or_else(|| word(fallback, f.lang).to_string())
}

fn is_here(f: &Facts, r: &Row) -> bool {
	r.enabled && r.planet == Some(f.here)
}

fn header(b: &mut Built, f: &Facts) {
	let out = &mut b.prims;
	text(out, 48.0, 44.0, FontRole::SemiBold, 13.0, TEXT_MUTED, Align::Left, 400.0, 0.18, &upper(word(&HEADER, f.lang)));
	text(out, 48.0, 80.0, FontRole::Condensed, 34.0, TEXT, Align::Left, 640.0, 0.04, &upper(&label(f, T_TITLE, &TITLE)));
	let (found, n) = (f.list.rows.iter().filter(|r| r.enabled).count(), f.list.rows.len());
	if found < n {
		let s = rac_engine::localization::ui_text(word(&FOUND, f.lang)).replacen("{}", &found.to_string(), 1).replacen("{}", &n.to_string(), 1);
		text(out, 1232.0, 92.0, FontRole::Regular, 15.0, TEXT_MUTED, Align::Right, 360.0, 0.0, &s);
	}
}

/// The "You are here" chip with its right edge at `right`, centred on `cy`; returns its left edge.
fn here_chip(out: &mut Vec<Prim>, f: &Facts, m: &dyn TextMeasure, right: f32, cy: f32) -> f32 {
	let s = word(&HERE, f.lang);
	let w = run_width(m, FontRole::SemiBold, 12.0, s, 0.0) + 18.0;
	let chip = Rect::new(right - w, cy - 12.0, w, 24.0);
	rrect(out, chip, 12.0, TEAL.tint(0.14), None);
	text(out, chip.cx(), chip.cy(), FontRole::SemiBold, 12.0, TEAL, Align::Center, 0.0, 0.0, s);
	chip.x
}

fn list(b: &mut Built, f: &Facts, m: &dyn TextMeasure) {
	rrect(&mut b.prims, LIST, 16.0, PANEL, Some((1.0, HAIRLINE)));
	let rows = &f.list.rows;
	let n = rows.len();
	// Last frame's window moved as little as keeps the selection in view (`vendor::scroll_first`): with the window always the
	// selection's, its bottom row, a hover after the wheel walked the list down chose the row above, the list moved under the
	// pointer, and so on up to the top (user 2026-11-04 "the galactic map is buggy with the mouse, same issue as we had with the vendor").
	let first = super::vendor::scroll_first(f.scroll, b.sel, n, VISIBLE);
	b.first = first;
	for (k, i) in (first..n.min(first + VISIBLE)).enumerate() {
		let r = &rows[i];
		let rect = Rect::new(LIST.x + 12.0, LIST.y + 12.0 + k as f32 * (ROW_H + ROW_GAP), LIST.w - 24.0, ROW_H);
		let (on, here) = (i == b.sel && r.enabled, is_here(f, r));
		let out = &mut b.prims;
		if on {
			rrect(out, rect, 10.0, ACCENT.tint(0.13), Some((1.5, ACCENT.tint(0.6))));
		}
		let dot = Rect::new(rect.x + 14.0, rect.cy() - 6.0, 12.0, 12.0);
		if r.enabled {
			rrect(out, dot, 6.0, if here { TEAL } else { STAR }, None);
		} else {
			rrect(out, dot, 6.0, CLEAR, Some((1.5, LOCKED)));
		}
		let right = if here { here_chip(out, f, m, rect.right() - 14.0, rect.cy()) - 8.0 } else { rect.right() - 14.0 };
		let (x, ink) = (rect.x + 38.0, if !r.enabled { TEXT_DISABLED } else if on { ACCENT } else { TEXT });
		let max_w = (right - x).max(0.0);
		if r.location.is_empty() {
			text(out, x, rect.cy(), FontRole::SemiBold, 18.0, ink, Align::Left, max_w, 0.0, &r.name);
		} else {
			text(out, x, rect.y + 19.0, FontRole::SemiBold, 18.0, ink, Align::Left, max_w, 0.0, &r.name);
			text(out, x, rect.y + 38.0, FontRole::Regular, 13.0, TEXT_SECTION, Align::Left, max_w, 0.0, &r.location);
		}
		if r.enabled {
			b.rows.push((i, rect));
		}
	}
}

/// The design-space point of the marker of row `r` on the painting.
fn mark_at(r: &Row) -> Option<(f32, f32)> {
	let (x, y) = mark(r.planet?)?;
	Some((x * 1280.0, y * 720.0))
}

/// The markers on the painting: a dot per planet (a hollow ring for a greyed one), the chosen planet last with its ring and the leader
/// line to the right panel.
fn marks(b: &mut Built, f: &Facts) {
	let (rows, t) = (&f.list.rows, clock_s());
	let order = (0..rows.len()).filter(|&i| i != b.sel || !rows[i].enabled).chain((b.sel < rows.len() && rows[b.sel].enabled).then_some(b.sel));
	for i in order {
		let r = &rows[i];
		let Some((x, y)) = mark_at(r) else { continue };
		let out = &mut b.prims;
		let dot = Rect::new(x - 5.0, y - 5.0, 10.0, 10.0);
		if !r.enabled {
			rrect(out, dot, 5.0, CLEAR, Some((1.5, Rgba::rgba(255, 255, 255, 0.35))));
			continue;
		}
		let here = is_here(f, r);
		if i == b.sel {
			// Alive (ship_anim.rs, display time): the ring breathes, a light runs along the leader line to the card.
			let (p, line) = (pulse(t, 2.4, 0.0), DETAIL.x - x - 22.0);
			if line > 0.0 {
				rrect(out, Rect::new(x + 22.0, y - 1.0, line, 2.0), 0.0, ACCENT.tint(0.40 + 0.15 * p), None);
				let (k, w) = (travel(t, 2.2), 28.0_f32.min(line));
				let lx = (x + 22.0 + (line - w) * k).max(x + 22.0);
				rrect(out, Rect::new(lx, y - 2.0, w, 4.0), 2.0, ACCENT.tint(0.85 * (k * std::f32::consts::PI).sin()), None);
			}
			let d = 44.0 + 6.0 * p;
			rrect(out, Rect::new(x - d * 0.5, y - d * 0.5, d, d), d * 0.5, ACCENT.tint(0.06 + 0.06 * (1.0 - p)), Some((2.0, ACCENT.tint(0.55 + 0.45 * (1.0 - p)))));
		} else if here {
			// The current planet: a slow halo round its green dot.
			let (p, d) = (pulse(t, 3.2, 0.0), 14.0 + 12.0 * pulse(t, 3.2, 0.0));
			rrect(out, Rect::new(x - d * 0.5, y - d * 0.5, d, d), d * 0.5, CLEAR, Some((1.5, TEAL.tint(0.65 * (1.0 - p)))));
		}
		let col = if i == b.sel { ACCENT } else if here { TEAL.tint(0.82 + 0.18 * pulse(t, 3.2, 0.5)) } else { STAR.tint(0.72 + 0.28 * pulse(t, 2.0 + 0.37 * (i % 5) as f64, 0.173 * i as f64)) };
		rrect(out, dot, 5.0, col, None);
		b.marks.push((i, Rect::new(x - MARK_HIT * 0.5, y - MARK_HIT * 0.5, MARK_HIT, MARK_HIT)));
	}
}

/// The button along the bottom of the right panel: the Cross press the game answers (open the details on the list, blast off on the
/// details). A click target (`Built::acts`).
fn button(b: &mut Built, f: &Facts, m: &dyn TextMeasure, r: &Row) {
	let (x, w) = (DETAIL.x + 22.0, DETAIL.w - 44.0);
	let btn = Rect::new(x, DETAIL.bottom() - 22.0 - 50.0, w, 50.0);
	let out = &mut b.prims;
	let (label, live) = match (f.stage, r.enabled) {
		(Stage::Details, _) => (fill(if is_here(f, r) { &LAND } else { &BLAST }, f.lang, &r.name), true),
		(Stage::List, live) => (label_of(f, T_SELECT, &SELECT), live),
	};
	if !live {
		rrect(out, btn, 10.0, BUTTON_BG, None);
		text(out, btn.cx(), btn.cy(), FontRole::SemiBold, 18.0, TEXT_DISABLED, Align::Center, w - 24.0, 0.0, &label);
		return;
	}
	rrect(out, btn, 10.0, ACCENT, None);
	b.acts.push((Symbol::Cross, btn));
	let lw = lead_w(f, m, Symbol::Cross);
	let tw = run_width(m, FontRole::SemiBold, 18.0, &label, 0.0).min(w - 24.0 - 10.0 - lw);
	let x0 = btn.cx() - (lw + 10.0 + tw) * 0.5;
	lead(out, f, m, Symbol::Cross, x0, btn.cy()); // the Cross glyph with a pad, the Enter keycap with the keyboard and mouse
	text(out, x0 + lw + 10.0, btn.cy(), FontRole::SemiBold, 18.0, INK_ON_ACCENT, Align::Left, w - 24.0 - 10.0 - lw, 0.0, &label);
}

fn label_of(f: &Facts, id: u32, fallback: &T5) -> String {
	label(f, id, fallback)
}

/// The card at the top of the right panel. List stage: a hole where the game's own planet picture goes (`ship_pic`; the retail overlay
/// holds it only on the list page). Details stage: a badge of concentric discs in `tone` (the game draws its spinning gold bolt there).
fn planet_card(b: &mut Built, card: Rect, tone: Rgba) {
	if b.stage == Stage::List {
		let hole = ship_pic::hole_in(card);
		b.prims.extend(holed_panel(card, hole, 12.0, ship_pic::CARD_FILL, Border { width: 1.0, colour: HAIRLINE }));
		b.planet = Some(hole);
		return;
	}
	let out = &mut b.prims;
	rrect(out, card, 12.0, ship_pic::CARD_FILL, Some((1.0, HAIRLINE)));
	for (d, a) in [(116.0, 0.07), (80.0, 0.13), (44.0, 0.30)] {
		rrect(out, Rect::new(card.cx() - d * 0.5, card.cy() - d * 0.5, d, d), d * 0.5, tone.tint(a), None);
	}
}

/// The right panel's plate; on the List stage with the planet picture's hole in it (the plate is translucent, the picture is below).
fn plate(b: &mut Built) {
	if b.stage == Stage::List {
		b.prims.extend(holed_panel(DETAIL, ship_pic::hole_in(CARD), 16.0, PANEL, Border { width: 1.0, colour: HAIRLINE }));
	} else {
		rrect(&mut b.prims, DETAIL, 16.0, PANEL, Some((1.0, HAIRLINE)));
	}
}

fn detail(b: &mut Built, f: &Facts, m: &dyn TextMeasure) {
	plate(b);
	let Some(r) = f.list.rows.get(b.sel) else { return };
	let (x, w) = (DETAIL.x + 22.0, DETAIL.w - 44.0);
	let here = is_here(f, r);
	let card = CARD;
	planet_card(b, card, if here { TEAL } else if r.enabled { ACCENT } else { LOCKED });
	let out = &mut b.prims;
	let mut y = card.bottom() + 30.0;
	if !r.location.is_empty() {
		text(out, x, y, FontRole::SemiBold, 13.0, TEXT_MUTED, Align::Left, w, 0.14, &upper(&r.location));
	}
	y += 40.0;
	text(out, x, y, FontRole::Condensed, 40.0, WHITE, Align::Left, w, 0.0, &r.name);
	if here {
		let chip_w = run_width(m, FontRole::SemiBold, 12.0, word(&HERE, f.lang), 0.0) + 18.0;
		here_chip(out, f, m, x + chip_w, y + 34.0);
	}
	facts_rows(b, f, r, y + 84.0);
	button(b, f, m, r);
}

/// The two lines the mockup shows under the name: "Gold bolts found  N of T" and "Status  Visited / Not visited yet" (retail's details page
/// shows the gold line, its map marks the visited planets). Only for a planet the player may choose and whose facts were read.
fn facts_rows(b: &mut Built, f: &Facts, r: &Row, y: f32) {
	let (Some(p), true) = (r.planet, r.enabled) else { return };
	let Some(flag) = f.planets.flag(p) else { return };
	let (x, w) = (DETAIL.x + 22.0, DETAIL.w - 44.0);
	let gold = rac_engine::localization::ui_text(word(&OF, f.lang)).replacen("{}", &f.planets.gold_found(p).to_string(), 1).replacen("{}", &gold_total(p).to_string(), 1);
	let status = word(if flag.visited { &VISITED } else { &NOT_VISITED }, f.lang);
	for (k, (label, value)) in [(word(&GOLD, f.lang), gold.as_str()), (word(&STATUS, f.lang), status)].into_iter().enumerate() {
		let cy = y + 30.0 * k as f32;
		text(&mut b.prims, x, cy, FontRole::Regular, 15.0, TEXT_MUTED, Align::Left, w * 0.55, 0.0, label);
		text(&mut b.prims, x + w, cy, FontRole::SemiBold, 15.0, TEXT_BODY, Align::Right, w * 0.45, 0.0, value);
	}
}

/// A pad glyph or a keycap of `sym` with its left edge at `x`, centred on `cy`; returns its width.
fn lead(out: &mut Vec<Prim>, f: &Facts, m: &dyn TextMeasure, sym: Symbol, x: f32, cy: f32) -> f32 {
	let w = lead_w(f, m, sym);
	let rect = Rect::new(x, cy - 12.0, w, 24.0);
	if f.device == Device::Keyboard {
		rrect(out, rect, 6.0, CAP_BG, Some((1.0, CAP_BORDER)));
		text(out, rect.cx(), rect.cy(), FontRole::SemiBold, 13.0, TEXT, Align::Center, 0.0, 0.0, key_name(sym));
	} else {
		out.push(Prim::Glyph { rect, kind: f.glyphs.glyph(sym), colour: symbol_colour(sym) });
	}
	w
}

fn lead_w(f: &Facts, m: &dyn TextMeasure, sym: Symbol) -> f32 {
	if f.device == Device::Keyboard { (run_width(m, FontRole::SemiBold, 13.0, key_name(sym), 0.0) + 14.0).max(24.0) } else { 24.0 }
}

/// The hint bar along the bottom: Select (List) or Blast off (Details), Exit, and Play Infobot only where the game lists it.
fn hints(b: &mut Built, f: &Facts, m: &dyn TextMeasure) {
	let first = if f.stage == Stage::List { label(f, T_SELECT, &SELECT) } else { label(f, T_BLAST, &BLAST_OFF) };
	let mut items = vec![(Symbol::Cross, first), (Symbol::Triangle, label(f, T_EXIT, &BACK))];
	let infobot = (f.text)(T_INFOBOT);
	if f.infobot && !infobot.is_empty() {
		items.push((Symbol::Circle, infobot));
	}
	let widths: Vec<f32> = items.iter().map(|(sym, s)| lead_w(f, m, *sym) + 8.0 + run_width(m, FontRole::Medium, 15.0, s, 0.0)).collect();
	let mut x = 1232.0 - widths.iter().sum::<f32>() - 26.0 * (items.len() - 1) as f32;
	let cy = 688.0;
	for ((sym, s), w) in items.iter().zip(&widths) {
		b.acts.push((*sym, Rect::new(x - 6.0, cy - 16.0, w + 12.0, 32.0)));
		let lw = lead(&mut b.prims, f, m, *sym, x, cy);
		text(&mut b.prims, x + lw + 8.0, cy, FontRole::Medium, 15.0, TEXT_BODY, Align::Left, 0.0, 0.0, s);
		x += w + 26.0;
	}
}

/// The page of `f`; None while the game lists no planet.
pub fn build(f: &Facts, m: &dyn TextMeasure) -> Option<Built> {
	let n = f.list.rows.len();
	if n == 0 {
		return None;
	}
	let sel = f.focus.unwrap_or(f.list.sel).min(n - 1);
	let enabled = f.list.rows.iter().map(|r| r.enabled).collect();
	let mut b = Built { prims: Vec::new(), rows: Vec::new(), marks: Vec::new(), enabled, sel, count: n, first: 0, stage: f.stage, item: f.list.item, art: true, planet: None, acts: Vec::new() };
	header(&mut b, f);
	if f.stage == Stage::List {
		list(&mut b, f, m);
		marks(&mut b, f);
	}
	detail(&mut b, f, m);
	hints(&mut b, f, m);
	Some(b)
}

/// Every table, for the font-coverage test.
pub fn all() -> [&'static T5; 12] {
	[&HEADER, &TITLE, &HERE, &BLAST, &BLAST_OFF, &LAND, &FOUND, &GOLD, &OF, &STATUS, &VISITED, &NOT_VISITED]
}
