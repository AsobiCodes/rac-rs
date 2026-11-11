//! Classic Graphics -> More effects / modern Game -> Extras (moved 2026-11-08) -> Supporter drawings (user 2026-11-04): shows or hides the Ko-fi supporters' drawings on the Veldin garage
//! walls (`kofi.rs`). Off by default, independent of the Lighting row. Same shape as the Metal row (`pcmetal.rs`): a two-value row
//! (Off / On, retail's own texts) whose byte `SUPP_B` the host reads, remembered in pc_options.txt as `supporters=on|off` (an extra
//! key: older files without it, and older builds reading a file with it, are unaffected). The label is text id 0x7fbf (a free slot
//! of pcdisp's pool, which hands it here through `pcdisp::text`).
use super::pcctl::CBASE;
use super::pcopts::PcOptions;
use ee_oracle::mem::Mem;

/// The row's byte (pcctl's block; 0 Off = the default, 1 On).
pub(super) const SUPP_B: u32 = CBASE + 0x21;
pub(super) const ID_SUPP: u32 = 0x7fbf;
/// The options-file key.
const KEY: &str = "supporters";

#[rustfmt::skip]
static LABEL: [&[u8]; 5] = [b"Supporter drawings", b"Dessins des soutiens", b"Fan-Zeichnungen", b"Dibujos de mecenas", b"Disegni sostenitori"];

/// The label of `id` in language `l` (0 En .. 4 It); empty for any other id.
pub fn text(id: u32, l: usize) -> Vec<u8> {
	if id == ID_SUPP { LABEL[l.min(4)].to_vec() } else { Vec::new() }
}

/// `pcext::init`: the file's value is the start value (anything but `on` = Off).
pub fn init(opts: &PcOptions) {
	crate::kofi::set_shown(opts.extra_get(KEY).is_some_and(|v| v.trim().eq_ignore_ascii_case("on")));
}

pub fn enabled() -> bool {
	crate::kofi::shown()
}

fn remember(opts: &mut PcOptions, on: bool) {
	crate::kofi::set_shown(on);
	opts.extra_set(KEY, Some(if on { "on" } else { "off" }.to_string()));
	eprintln!("retail: PC options: Supporter drawings {}", if on { "On" } else { "Off" });
}

/// The modern screen set the row (`pcext/ui_bridge.rs`): applied and remembered; true = the file changed.
pub fn set_from_ui(opts: &mut PcOptions, on: bool) -> bool {
	if enabled() == on {
		return false;
	}
	remember(opts, on);
	true
}

/// Worker, every field: the classic row's byte changed the setting (live, no restart); true = the caller saves.
pub fn sync(mem: &mut Mem, opts: &mut PcOptions) -> bool {
	if !super::pcctl::gfx_built(mem) {
		return false;
	}
	let on = mem.rd8(SUPP_B) != 0;
	if enabled() == on {
		return false;
	}
	remember(opts, on);
	true
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn off_by_default_and_the_file_round_trips() {
		let _g = crate::retail::pcopts::globals_guard();
		init(&PcOptions::new());
		assert!(!enabled(), "a new player: Off");
		let mut o = PcOptions::new();
		assert!(!set_from_ui(&mut o, false) && o.extra_get(KEY).is_none(), "default Off writes no key");
		assert!(set_from_ui(&mut o, true) && enabled());
		assert_eq!(PcOptions::parse(&o.to_text()).extra_get(KEY), Some("on"));
		init(&PcOptions::parse(&o.to_text()));
		assert!(enabled());
		assert!(set_from_ui(&mut o, false) && !enabled());
		init(&PcOptions::parse(&o.to_text()));
		assert!(!enabled());
		init(&PcOptions::parse("supporters=maybe\n"));
		assert!(!enabled(), "garbage = Off");
		init(&PcOptions::new());
	}

	#[test]
	fn the_classic_row_byte_is_read_live() {
		let _g = crate::retail::pcopts::globals_guard();
		init(&PcOptions::new());
		let mut mem = Mem::new();
		let mut o = PcOptions::new();
		assert!(!sync(&mut mem, &mut o), "page not built");
		let rows: Vec<[u32; 6]> = (0..15u32).map(|i| [0x100 + i, 0x200 + i, 0x300, 0x301, 0, 0]).collect();
		super::super::pcctl::build_gfx(&mut mem, super::super::pcext::BASE + 0xaa0, &rows, true, false);
		assert_eq!(mem.rd8(SUPP_B), 0, "default RAM stays zero");
		assert!(!sync(&mut mem, &mut o));
		mem.wr8(SUPP_B, 1);
		assert!(sync(&mut mem, &mut o) && enabled() && !sync(&mut mem, &mut o));
		mem.wr8(SUPP_B, 0);
		assert!(sync(&mut mem, &mut o) && !enabled());
	}

	#[test]
	fn the_label_fits_the_menu() {
		assert!(LABEL.iter().all(|l| l.len() <= 22 && l.iter().all(|&b| (0x20..0x7f).contains(&b))));
		assert_eq!(text(ID_SUPP, 0), b"Supporter drawings");
		assert!(text(0x7fc0, 0).is_empty());
	}
}
