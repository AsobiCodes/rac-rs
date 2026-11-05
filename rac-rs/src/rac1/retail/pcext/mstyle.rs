//! The classic "Menu style" row (Pause / title Options -> Graphics -> System -> Menu style: Modern / Classic), retail side of the modern
//! menu lane. The modern screen has the same row (`modern_menu/rows_camera.rs`, id "menustyle"); both end in
//! `modern_menu::style::set_from_ui` and the options file's key `menustyle=classic` (Modern = no key, the default). The row's byte
//! lives in the pcctl block (`pcctl::MSTYLEB`, 0 Modern, 1 Classic), so a default RAM stays zero. `RAC_MENU_STYLE` wins over the file
//! for the pages that open (`style::modern`) but is never written back. The label and the Modern value are two texts of `texts.rs`
//! (the Classic value is the Lighting row's masculine "Classic", `ID_LIGHT_CLASSIC`).
use super::*;
use crate::modern_menu::style;

/// Text ids (free ids between the Achievements ones and the Lock-on block).
pub const ID_MSTYLE: u32 = 0x7f2d;
pub const ID_MSTYLE_MODERN: u32 = 0x7f2e;

/// The row's start byte (`pcctl::build_gfx` on fresh RAM): the file's choice.
pub fn byte() -> u8 {
	style::file_classic() as u8
}

/// Worker, every field (`pcext::sync`): a changed row byte becomes the choice and goes into the options file (true = the caller saves).
pub fn sync(mem: &mut Mem, opts: &mut PcOptions) -> bool {
	let Some(classic) = super::super::pcctl::menu_classic(mem) else { return false };
	if style::file_classic() == classic {
		return false;
	}
	eprintln!("retail: PC options: Menu style {}", if classic { "Classic" } else { "Modern" });
	style::set_from_ui(opts, classic);
	true
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::pcctl;

	fn page() -> Mem {
		let mut mem = Mem::new();
		let rows: Vec<[u32; 6]> = (0..15u32).map(|i| [0x100 + i, 0x200 + i, 0x300, 0x301, 0, 0]).collect();
		pcctl::build_gfx(&mut mem, BASE + 0xaa0, &rows, true, false);
		mem
	}

	/// The classic row and the file agree both ways: a pressed row is saved once, the default RAM byte stays zero, and the modern
	/// screen's `set_from_ui` writes the byte so `sync` does not undo it.
	#[test]
	fn menu_style_row_round_trips_through_the_file() {
		let _g = crate::retail::pcopts::globals_guard();
		style::init(&PcOptions::new());
		let mut mem = page();
		let mut o = PcOptions::new();
		assert_eq!(mem.rd8(pcctl::MSTYLEB), 0, "default RAM stays zero");
		assert!(!sync(&mut mem, &mut o), "nothing changed");
		mem.wr8(pcctl::MSTYLEB, 1);
		assert!(sync(&mut mem, &mut o) && o.extra_get("menustyle") == Some("classic") && style::file_classic());
		assert!(!sync(&mut mem, &mut o), "saved once");
		style::set_from_ui(&mut o, false);
		pcctl::set_ui_byte(&mut mem, pcctl::UiByte::MenuStyle, false);
		assert!(!sync(&mut mem, &mut o) && o.extra_get("menustyle").is_none() && mem.rd8(pcctl::MSTYLEB) == 0);
		let mut fresh = PcOptions::new();
		style::set_from_ui(&mut fresh, true);
		style::init(&fresh);
		assert_eq!(byte(), 1, "a file that says classic starts the row on Classic");
		style::init(&PcOptions::new());
	}

	/// The row's three texts exist in all five languages, in the cell widths of their neighbours (the label like the longest Graphics
	/// labels, ~21 letters; a value up to 12).
	#[test]
	fn texts_five_languages_lengths() {
		for (id, max) in [(ID_MSTYLE, 21), (ID_MSTYLE_MODERN, 12), (super::super::ID_LIGHT_CLASSIC, 12)] {
			let k = NEW_IDS.iter().position(|&n| n == id).expect("listed in NEW_IDS");
			for lang in 0..5 {
				let t = text_for(k, lang);
				assert!(!t.is_empty() && t.len() <= max, "{id:#x} lang {lang}: {} bytes", t.len());
				assert!(t.iter().all(|&b| b >= 0x20), "{id:#x} lang {lang}: plain font bytes");
			}
		}
		assert_eq!(text_for(NEW_IDS.iter().position(|&n| n == ID_MSTYLE).unwrap(), 0), b"Menu style");
		assert!(pool_slot(NEW_IDS.len() - 1) + 0x40 <= camopts::CO_POOL_END, "the last pool slot stays inside the block");
		let slots: std::collections::HashSet<u32> = (0..NEW_IDS.len()).map(pool_slot).collect();
		assert_eq!(slots.len(), NEW_IDS.len(), "no two texts share a slot");
	}
}
