//! The extra lines of the repainted Help > Controls picture (`help_ctl.rs`): the buttons the art has no label for (d-pad, L3, R3) and the
//! words that do not fit their label ("Select: Map") go into the picture's free line slots. Every entry gets a slot: when there are more
//! entries than slots the shortest ones are packed two to a line (compact " xN" forms, narrower faces), nothing is dropped.

use super::help_ctl_font::Face;
use super::help_ctl_text::{self as tx, Line};

const FACES: [Face; 3] = [Face::Normal, Face::Tight, Face::Packed];
const JOIN: &str = "; ";

/// One line to place: its wordings, longest first.
struct Entry {
	forms: Vec<String>,
}

impl Entry {
	fn min_width(&self) -> f32 {
		self.forms.iter().map(|f| tx::text_width(Face::Packed, f)).fold(f32::MAX, f32::min)
	}
}

/// `queue`: the wordings of each entry, longest first. Returns the lines and how many entries were cut short (0 in every case the tests build).
pub fn place(queue: Vec<Vec<String>>, slots: &[[u16; 4]]) -> (Vec<Line>, usize) {
	let mut entries: Vec<Entry> = queue.into_iter().map(|forms| Entry { forms }).collect();
	while entries.len() > slots.len() && entries.len() > 1 {
		entries.sort_by(|a, b| a.min_width().total_cmp(&b.min_width()));
		let (a, b) = (entries.remove(0), entries.remove(0));
		let forms = a.forms.iter().zip(&b.forms).map(|(x, y)| format!("{x}{JOIN}{y}")).collect();
		entries.push(Entry { forms });
	}
	let mut order: Vec<&[u16; 4]> = slots.iter().collect();
	order.sort_by_key(|s| std::cmp::Reverse(s[2]));
	entries.sort_by(|a, b| b.min_width().total_cmp(&a.min_width()));
	let (mut lines, mut lost) = (Vec::new(), 0);
	for (e, s) in entries.iter().zip(order) {
		let orig = [s[0], s[1], 1, s[3]];
		let fit = e.forms.iter().find_map(|f| FACES.iter().find(|&&fc| tx::text_width(fc, f) <= s[2] as f32).map(|&fc| (f, fc)));
		match fit {
			Some((f, fc)) => lines.extend(tx::place_one(fc, f, *s, orig)),
			None => {
				lost += 1;
				#[cfg(test)]
				eprintln!("CUT {:?} slot {s:?} of {} entries", e.forms, entries.len());
				let (l, _) = tx::layout_or_count(&e.forms[e.forms.len() - 1..], &e.forms[e.forms.len() - 1], false, *s, orig, true);
				lines.extend(l);
			}
		}
	}
	(lines, lost)
}
