//! Words and layout of the repainted Help > Controls labels (`help_ctl.rs`): which words a gameplay action gets in the picture, and how
//! phrases are fitted into a label's free box (one line, one phrase per line, word wrap; the normal face, then two narrower ones; at last the first phrase + " x2" like the Controls page). Pure functions: the unit tests run every language and slot.

use super::menu_model::text::font_char;
use crate::kbm_actions as ka;
use super::help_ctl_font::{self as font, Face};

type W = [&'static str; 5];

// The words retail's picture / text bank use for these actions (En Fr De Es It; the picture's own wording where the bank differs).
const JUMP: W = ["Jump", "Sauter", "Springen", "Saltar", "Salta"];
const SWING: W = ["Swing wrench", "Attaque de clé", "Omni-Knochen-Angriff", "Ataque con llave", "Attacco chiave"];
const MENU: W = ["Menu", "Menu", "Menü", "Menú", "Menu"];
const CROUCH: W = ["Crouch", "S'accroupir", "Hocke", "Agachar", "Accovacciati"];
const QUICK: W = ["Quick select", "Sélect-prompt", "Schnellauswahl", "Selección rápida", "Selezione Rapida"];
const FIRE: W = ["Fire weapon", "Attaque d'armes", "Waffen feuern", "Disparar", "Spara"];
const GADGET: W = ["Use gadget", "Utiliser gadget", "Geräte benutzen", "Usar artilugio", "Usa gadget"];
const FPT: W = ["First person targeting", "Mode 1ère personne", "Erste-Person-Zielsicht", "Modo en 1a persona", "Modalità in 1a persona"];
const MAP: W = ["Map", "Carte", "Karte", "Mapa", "Mappa"];

/// The action's words in the picture's wording (the label slots).
pub fn main_words(id: &str, lang: usize) -> Vec<String> {
	let w = |t: &W| vec![t[lang].to_string()];
	match id {
		"jump" => w(&JUMP),
		"wrench" => w(&SWING),
		"fire" => w(&FIRE),
		"quick_select" => w(&QUICK),
		"look" | "look2" => w(&FPT),
		"crouch" | "crouch2" => w(&CROUCH),
		"pause" => w(&MENU),
		"map" | "r3" => w(&MAP),
		_ => vec![short(id, lang)],
	}
}

/// "Fire weapon" alone on a button also says what the circle button does in retail: "Use gadget".
pub fn gadget(lang: usize) -> String {
	GADGET[lang].to_string()
}

/// The action's short name (the Controls page label without its "(L1)" / "(PC)" tail): the extra labels and the actions retail has no word for.
pub fn short(id: &str, lang: usize) -> String {
	ka::install();
	let Some(i) = ka::ACTIONS.iter().position(|a| a.id == id) else { return id.to_string() };
	let s: String = ka::LABELS[i][lang].iter().filter_map(|&b| font_char(b)).collect();
	let main = id != "look" && id != "look2" && id != "crouch" && id != "crouch2";
	let cut = if main { s.find(" (").unwrap_or(s.len()) } else { s.find(" (").unwrap_or(s.len()) };
	s[..cut].to_string()
}

/// One placed line: text, face, top-left of its 13-row cell (cap top = y + 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
	pub text: String,
	pub role: Face,
	pub x: i32,
	pub y: i32,
}

pub const PITCH: i32 = 12;
const ROLES: [Face; 3] = [Face::Normal, Face::Tight, Face::Packed];

fn width(role: Face, s: &str) -> f32 {
	font::measure(role, s)
}

/// Width of `s` in the face (the extra lines' fitting).
pub fn text_width(role: Face, s: &str) -> f32 {
	width(role, s)
}

/// `s` as one line, left aligned in the slot `bx`.
pub fn place_one(role: Face, s: &str, bx: [u16; 4], orig: [u16; 4]) -> Vec<Line> {
	place(role, &[s.to_string()], bx, orig, true)
}

/// Greedy word wrap of `s` (breaks at spaces and after hyphens or slashes) into lines of at most `w` px; None if a word is wider.
fn wrap(role: Face, s: &str, w: f32) -> Option<Vec<String>> {
	let mut pieces: Vec<String> = Vec::new();
	for word in s.split(' ') {
		let mut cur = String::new();
		for part in word.split_inclusive(['-', '/']) {
			cur.push_str(part);
			if part.ends_with(['-', '/']) {
				pieces.push(std::mem::take(&mut cur));
			}
		}
		if !cur.is_empty() {
			pieces.push(cur);
		}
		pieces.push(String::from(" "));
	}
	pieces.pop();
	let mut lines: Vec<String> = vec![String::new()];
	for p in pieces {
		let last = lines.last_mut().unwrap();
		let glued = !last.ends_with(' ') && p != " " && !last.is_empty() && !last.ends_with(['-', '/']) && !last.ends_with(char::is_alphanumeric);
		let _ = glued;
		let t = format!("{last}{p}");
		if width(role, t.trim_end()) <= w || last.is_empty() {
			*last = t;
		} else if p != " " {
			lines.push(p);
		}
	}
	let lines: Vec<String> = lines.into_iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
	lines.iter().all(|l| width(role, l) <= w).then_some(lines)
}

/// Lines (as text) for `units`: one unit per line, else all joined by ", " on one line (when `join`), else each unit wrapped.
fn candidates(role: Face, units: &[String], join: bool, w: f32) -> Vec<Vec<String>> {
	let mut out = vec![units.to_vec()];
	if join && units.len() > 1 {
		out.push(vec![units.join(", ")]);
	}
	let wrapped: Option<Vec<String>> = units.iter().map(|u| wrap(role, u, w)).collect::<Option<Vec<_>>>().map(|v| v.concat());
	out.extend(wrapped);
	if join && units.len() > 1 {
		out.extend(wrap(role, &units.join(", "), w));
	}
	out
}

/// How many 12 px lines a box of height `h` holds.
pub fn max_lines(h: u16) -> usize {
	if h < 13 { 0 } else { (h as usize - 13) / PITCH as usize + 1 }
}

/// Fits `units` into the box `bx` (x, y, w, h): None when no way fits. `orig` is the old label's rect (the text keeps its centre / left edge),
/// `left` = left aligned at the old label's left edge (else centred on it).
pub fn layout(units: &[String], join: bool, bx: [u16; 4], orig: [u16; 4], left: bool) -> Option<Vec<Line>> {
	let (bw, maxl) = (bx[2] as f32, max_lines(bx[3]));
	for role in ROLES {
		for lines in candidates(role, units, join, bw) {
			if lines.len() <= maxl && lines.iter().all(|l| width(role, l) <= bw) {
				return Some(place(role, &lines, bx, orig, left));
			}
		}
	}
	None
}

/// The same, with `fallback` (the first words + " x2") when nothing fits; at last the first characters in the narrow face
/// (the flag is true then: something was cut off, the tests require it never happens).
pub fn layout_or_count(units: &[String], fallback: &str, join: bool, bx: [u16; 4], orig: [u16; 4], left: bool) -> (Vec<Line>, bool) {
	if let Some(l) = layout(units, join, bx, orig, left) {
		return (l, false);
	}
	if let Some(l) = layout(&[fallback.to_string()], false, bx, orig, left) {
		return (l, false);
	}
	let mut t = fallback.to_string();
	while t.chars().count() > 1 && width(Face::Packed, &t) > bx[2] as f32 {
		t.pop();
	}
	(place(Face::Packed, &[t], bx, orig, left), true)
}

fn place(role: Face, lines: &[String], bx: [u16; 4], orig: [u16; 4], left: bool) -> Vec<Line> {
	let bh = PITCH * (lines.len() as i32 - 1) + font::LINE_H as i32;
	let cy = orig[1] as i32 + orig[3] as i32 / 2;
	let ty = (cy - bh / 2).clamp(bx[1] as i32, (bx[1] as i32 + bx[3] as i32 - bh).max(bx[1] as i32));
	let cx = orig[0] as f32 + orig[2] as f32 / 2.0;
	lines
		.iter()
		.enumerate()
		.map(|(i, t)| {
			let w = width(role, t);
			let x = if left { orig[0] as f32 + 1.0 } else { cx - w / 2.0 };
			let x = x.clamp(bx[0] as f32, (bx[0] as f32 + bx[2] as f32 - w).max(bx[0] as f32));
			Line { text: t.clone(), role, x: x.round() as i32, y: ty + PITCH * i as i32 }
		})
		.collect()
}

/// Test hook.
#[cfg(test)]
pub(super) fn wrap_for_test(role: Face, s: &str, w: f32) -> Option<Vec<String>> {
	wrap(role, s, w)
}
