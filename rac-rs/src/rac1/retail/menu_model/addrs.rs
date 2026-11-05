//! Where the menu model reads: the front end's page structs (PAL addresses, compared with `disc::unva` of the page on screen),
//! the per-level current-page pointer (`mouse_menu::page_ptr`, PAL or USA table by the disc that runs) and the core words that
//! sit at the same address on both discs.
pub use crate::retail::mouse_menu::{front_page_ptr, page_ptr};

/// Page structs of the front end (frontend_flow.md section 2): root, New Game, Load Game, Options, Language, Video, Sound.
pub const P_ROOT: u32 = 0x1d4548;
pub const P_NEW: u32 = 0x1d4c08;
/// New Game's overwrite confirm (FUN_002230b8: Cross on a used slot sets the next page 0x1d4db8, id 0x1a): title, slot list,
/// text 0x4fb3 panel (FUN_00221650), Circle Yes / Triangle No labels.
pub const P_OVERWRITE: u32 = 0x1d4db8;
pub const P_LOAD: u32 = 0x1d4b80;
pub const P_OPTIONS: [u32; 2] = [0x1d4708, 0x1d47c0];
pub const P_LANGUAGE: u32 = 0x1d48c0;
pub const P_VIDEO: u32 = 0x1d4a38;
pub const P_SOUND: u32 = 0x1d4838;

/// Core words (below 0x15ef00: the same address on the PAL and the USA disc).
/// The planets' unlocked / visited flags [I: byte arrays, 0x18 apart].
pub const UNLOCKED: u32 = 0x13dd40;
pub const VISITED: u32 = 0x13dd58;
/// The level number, the language word and the bolt count.
pub const LEVEL_WORD: u32 = 0x15ed84;
pub const LANG_WORD: u32 = 0x15ed88;
pub const BOLTS: u32 = 0x15ed98;

/// How many planets the flag arrays hold (their distance).
pub const PLANETS: u32 = 0x18;

/// One planet's flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanetFlag {
	pub unlocked: bool,
	pub visited: bool,
}

/// The unlocked / visited flag of each planet (`rd` = 32-bit word read).
pub fn planet_flags(rd: &dyn Fn(u32) -> u32) -> Vec<PlanetFlag> {
	let byte = |a: u32| (rd(a & !3) >> (8 * (a & 3))) & 0xff;
	(0..PLANETS).map(|p| PlanetFlag { unlocked: byte(UNLOCKED + p) != 0, visited: byte(VISITED + p) != 0 }).collect()
}
