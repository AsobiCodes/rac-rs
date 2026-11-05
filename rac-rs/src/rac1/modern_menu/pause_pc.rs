//! The PC Controls and Mods pages of the pause Options list in the modern pause menu (v0.5 test build 4, lane v5p): the rows the
//! classic page shows (`menu_model/pcpage.rs` reads the retail table and resolves its texts), drawn as the Sound page's list. The page
//! is retail's own value page behind the picture: Up / Down, Select, Left / Right (the section stepper) and Back reach it as for
//! the other pages, so rebinding (a capture of the next key or button), conflicts (a key two functions share), Reset, mod switches and
//! mod settings are the classic code's and write the same `pc_options.txt`, `mods/active.txt` and `mods/settings.json`.
use super::pause::{list, Facts, Parts};
use super::title::word;
use crate::retail::menu_model::pages::Page;
use rac_engine::ui::kit::page::{PageRow, Panel};
use rac_engine::ui::T5;

// Not retail text: the line under the Controls rows (modern-only; native-speaker review wanted).
const HOW: T5 = [
	"Choose a function, then press the new key or button. Esc cancels.",
	"Choisissez une fonction, puis appuyez sur la nouvelle touche ou le nouveau bouton. \u{c9}chap annule.",
	"W\u{e4}hle eine Funktion und dr\u{fc}cke dann die neue Taste oder den neuen Knopf. Esc bricht ab.",
	"Elige una funci\u{f3}n y pulsa la nueva tecla o bot\u{f3}n. Esc cancela.",
	"Scegli una funzione, poi premi il nuovo tasto o pulsante. Esc annulla.",
];

/// The stepper row's value between its two arrows.
fn stepped(v: &str) -> String {
	format!("\u{2039}  {v}  \u{203a}")
}

/// Retail text ids are below this; the PC pages' own ids (pcctl 0x7f50.., pcmods 0x7fc0..) come after.
const OWN_IDS: u32 = 0x7000;

/// The words of a row: what the page module read, or for a retail id it could not answer (On / Off) the game's own text.
fn said(f: &Facts, read: &str, id: u32) -> String {
	if read.is_empty() && id != 0 && id < OWN_IDS {
		(f.text)(id)
	} else {
		read.to_string()
	}
}

/// The panel of a PC page: one row per table record, the game's cursor; a row without a value (a note, a blank) is inert.
pub(super) fn pc_parts(f: &Facts) -> Option<Parts> {
	let pc = f.extra.pc.as_ref().filter(|p| p.kind.page() == f.view.page)?;
	let l = list(f.view).filter(|l| l.n == pc.rows.len())?;
	let shown: Vec<(String, String)> = pc.rows.iter().map(|r| (said(f, &r.label, r.label_id), said(f, &r.value, r.value_id))).collect();
	let rows = pc
		.rows
		.iter()
		.zip(&shown)
		.map(|(r, (label, value))| PageRow { label: label.clone(), value: if r.stepper { stepped(value) } else { value.clone() }, dim: value.is_empty(), ..Default::default() })
		.collect();
	let targets = shown.iter().enumerate().map(|(r, (_, value))| (!value.is_empty()).then_some((l.addr, r as u32))).collect();
	Some(Parts { panel: Panel::Rows { caption: pc.title.clone(), rows }, targets, dialog: None, sel: l.cursor, focus: true, grid: None })
}

/// The note line of the page (empty = none).
pub(super) fn footer(f: &Facts) -> String {
	match f.view.page {
		Page::PcControls => word(&HOW, f.lang).to_string(),
		_ => String::new(),
	}
}
