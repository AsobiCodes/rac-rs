//! RAC_RETAIL_MOUSE: a scripted mouse pointer for menu runs without a real mouse (lane mmenu, v0.4.2; the dev proof of the instant
//! selection in `mouse_instant.rs`, captured frame by frame with RAC_SHOT_SEQ). Entries `FIELD:fx,fy[:click|:back]` separated by `;`:
//! from retail field FIELD on the pointer is at (fx, fy), fractions of the window's client area (0..1, 0 = left / top); `click` presses
//! the left button (and keeps it held until `up`; a Sound Options bar drag), `back` presses the right one, `up` lets the left go. The pointer goes through the same `Retail::mouse_cursor` / `mouse_click` /
//! `mouse_back` the window's events use. Nothing happens without the variable.

use super::Retail;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Entry {
	field: u32,
	at: (f32, f32),
	click: bool,
	back: bool,
	up: bool,
}

/// The entries not yet applied, earliest first (None: the variable was not read yet).
static SCRIPT: Mutex<Option<Vec<Entry>>> = Mutex::new(None);

fn parse(v: &str) -> Vec<Entry> {
	let mut out: Vec<Entry> = v
		.split(';')
		.filter_map(|e| {
			let mut f = e.trim().split(':');
			let field = f.next()?.trim().parse().ok()?;
			let (x, y) = f.next()?.split_once(',')?;
			let act = f.next().unwrap_or("");
			Some(Entry { field, at: (x.trim().parse().ok()?, y.trim().parse().ok()?), click: act == "click", back: act == "back", up: act == "up" })
		})
		.collect();
	out.sort_by_key(|e| e.field);
	out
}

/// Before each field of `Retail::tick_keys`: applies the entries whose field has come.
pub(super) fn poll(r: &mut Retail) {
	let Ok(mut g) = SCRIPT.lock() else { return };
	let list = g.get_or_insert_with(|| parse(&std::env::var("RAC_RETAIL_MOUSE").unwrap_or_default()));
	if list.is_empty() {
		return;
	}
	let due = list.iter().take_while(|e| e.field <= r.fields).count();
	for e in list.drain(..due).collect::<Vec<_>>() {
		r.mouse_cursor(e.at.0, e.at.1);
		if e.click {
			r.mouse_click();
		}
		if e.back {
			r.mouse_back();
		}
		if e.up {
			r.mouse_release();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn entries_parse_and_sort() {
		let v = parse("30:0.5,0.25:click; 10:0.1,0.2 ;bad; 50:0.9,0.9:back");
		assert_eq!(v.len(), 3);
		assert_eq!(v[0], Entry { field: 10, at: (0.1, 0.2), click: false, back: false, up: false });
		assert_eq!((v[1].field, v[1].click), (30, true));
		assert_eq!((v[2].field, v[2].back), (50, true));
		assert!(parse("").is_empty());
	}
}
