//! The Controls page (Pause -> Options -> Controls): the menu that edits the bindings of `binds.rs`, and the section stepper that
//! the Graphics page (`pcext::patch_graphics`) shares.
//!
//! Both pages are built like the retail value pages: the Options list gains a row, and the row opens a copy of the retail Camera
//! Options page with its own header and multi-choice item, so it is drawn, steered and sounds like the retail value pages. The row
//! table (24-byte records `{label id, byte*, choice 0..3}`, ended by a zero record) is data, and the retail item reads it afresh on
//! every draw and every update (FUN_0027ea40 counts the records, FUN_0027e898 stops the cursor at the last one), so a page of any
//! length up to the panel's 8 graphics rows is a table the host rewrites.
//!
//! Controls use at most 7 rows; Graphics fits 8 in the taller panel, so pages with more settings have
//! sections: row 0 is "Page", a stepper whose value is the section's name. The record `{Page, byte, Name, Name}` has two equal
//! choice ids, so the retail item's Cross toggles the row byte 0 <-> 1 and the host reads each change as "next section" (the byte only
//! signals); Left / Right (`fin.buttons`, also the mouse wheel) step back / forward with the menu's own auto-repeat while the page's
//! cursor is on row 0 (`mouse_menu::read`); the steps wrap. The same trick as the Camera speed row of `pcext.rs`. The section is the
//! host's (`St`), the name is the dynamic text `ID_SELC` / `ID_SELG`, and a section change rewrites the table.
//!
//! Controls (10 sections of at most 6 rows, the Debug section is the 11th below): Keyboard buttons 1/2, 2/2, movement, camera, D-pad, PC
//! extras, Controller buttons 1/2, 2/2, D-pad, Reset. A function row is a two-choice item: choice 0 shows the bound key or button, choice 1 "Press a key" / "Press a
//! button" (every text is a dynamic id 0x7f40..0x7f7f that the text lookup hand answers, `pcext::text_hook` -> `text_hook`, from the
//! CURRENT bindings, so the page needs no redraw hook). Confirm flips the row byte to 1; `sync` (worker, every field) sees it and
//! starts a capture (`binds::capture_begin`): the host (`Retail::tick_keys`) gives the game a neutral pad and binds the next fresh
//! key or button (Esc or 8 s cancel), then `sync` sees the capture end and puts the byte back to 0, so the row shows the new binding.
//! A key or button another function has is shared, nothing else moves (`binds::KbBinds::assign`, v0.4.1); both rows show it, the value
//! naming the other function ("B / Circle", `shared_value`). The Reset section's rows restore the defaults and read "Done" for a moment. A
//! section change cancels a capture.
//!
//! Debug (the Controls page's 11th and last section, `SEC_DEBUG`): row 1 "Debug mode" Off / On (byte `DBGB`; `pcext::sync` hands it to
//! `devtools::set_menu` and the options file, `debug=on`), rows 2..6 list the keys it switches on (F1, F2, F5, F6, F8); those five rows
//! only show text (their byte is reset like a blank row's). Their texts are the ids 0x7fa0..0x7faa in a pool of their own (`DBG_POOL`).
//!
//! Graphics: Picture, Effects, More effects (Fur, Fur style), Screen, Display, System and Gameplay sections. Effects includes Vegetation and
//! Metal; Display includes Frame smoothing. Graphics fits seven settings plus the page selector.
//! the rows are `pcext::ROWS` (bytes and values unchanged) plus the Preset row, the HUD row and the Renderer row. HUD (`HUDB`: 0 Sharp,
//! 1 Original) is handed by `pcext::sync` to the options file and the renderer (`pcext::hud_sharp_now`); Renderer (`RENDB`: 0 Vulkan,
//! 1 DirectX 12) by `pcrender::sync` to the options file, and it applies after a restart. Preset (`PRESETB`: 0 Low, 1 Medium, 2 High,
//! 3 Custom) is a choice of the player only (`preset_chosen`): `pcext/presets.rs` copies the preset's values into the other rows, and
//! shows (`preset_show`) the preset that matches the rows now; its record has the 4th choice Custom only while the rows match no preset
//! (`St::custom`), so a player cannot pick Custom.
//!
//! Reserved RAM (`CBASE`, inside pcext's BASE window, above prompts.rs's pool): the row bytes, the Controls page copy, its header
//! and value items, its row table, the Options pictures and the text pool. All of it is written only after `build_page` /
//! `build_gfx` set their magic word.
use super::binds;
use super::pcctl_kbm as kb;
use super::pcctl_pad as pd;
use rac_engine::input::kbm;
use super::mouse_menu;
use super::pcdisp;
use super::{glyphs, pcinput};
use super::pcext::mstyle::{ID_MSTYLE, ID_MSTYLE_MODERN};
use super::pcext::{BASE, GPAGE, GVAL, ID_HIGH, ID_LIGHT_CLASSIC, ID_LOW, ID_MEDIUM};
use ee_oracle::cpu::Cpu;
use ee_oracle::mem::Mem;
#[cfg(not(test))]
use std::sync::Mutex;

/// The region: BASE + 0xA000 .. + 0xB600 (the canary of prompts.rs skips it).
pub const CBASE: u32 = BASE + 0xA000;
pub const RANGE: std::ops::Range<u32> = CBASE..CBASE + 0x1600;
/// The Debug section's text pool: ids `ID_DBG0..ID_DBG_END`, one 0x40-byte slot each, BASE + 0xB800 .. + 0xBC00 (the canary of
/// prompts.rs skips it too).
pub const DBG_POOL: u32 = BASE + 0xB800;
pub const DBG_RANGE: std::ops::Range<u32> = DBG_POOL..DBG_POOL + 0x400;
/// Controls: selector and at most six settings. Graphics also fits seven settings in the taller panel.
const ROWS: usize = 7;
/// Capacity of a row table: eight graphics rows and the zero record.
pub(super) const TABLE_RECS: usize = ROWS + 2;
/// The Graphics page's selector byte and its HUD row's byte (0 Sharp, 1 Original).
const G_SEL: u32 = CBASE + 0x10;
const HUDB: u32 = CBASE + 0x11;
/// The Graphics page's Preset row byte: 0 Low, 1 Medium, 2 High, 3 Custom (`pcext/presets.rs` chooses and detects).
pub(super) const PRESETB: u32 = CBASE + 0x14;
/// The Graphics page's Renderer row byte (0 Vulkan, 1 DirectX 12; pcrender.rs).
const RENDB: u32 = CBASE + 0x13;
/// The Graphics page's Skip intro row byte (0 Off, 1 On; pcintro.rs). Stays 0 with the default `intro=play`.
const INTROB: u32 = CBASE + 0x15;
/// The Graphics page's Start skips cutscenes row byte, INVERTED: 0 = On (the default), 1 = Off (pcskip.rs), so a default RAM stays zero.
pub(super) const SKIPB: u32 = CBASE + 0x16;
pub(super) const METAL_B: u32 = CBASE + 0x1e;
/// The System section's Menu style row byte (0 Modern = the default, 1 Classic; pcext/mstyle.rs). A default RAM stays zero.
pub(super) const MSTYLEB: u32 = CBASE + 0x1f;
/// The Debug section's row byte (0 Off, 1 On); set from `devtools::menu` when the page is built.
pub const DBGB: u32 = CBASE + 0x12;
/// Set by `build_gfx` / `build_page`: that page is in RAM, so `field` / `sync` may read and write its bytes.
const G_MAGIC_AT: u32 = CBASE + 0xf8;
const MAGIC_AT: u32 = CBASE + 0xfc;
const MAGIC: u32 = 0x4c54_4e43;
const CPAGE: u32 = CBASE + 0x100;
const CHDR: u32 = CBASE + 0x200;
const CVAL: u32 = CBASE + 0x280;
const CTAB: u32 = CBASE + 0x300;
/// The Options list's picture records (up to 14 x 8 bytes), moved here from pcext's block (it would overlap the Graphics page).
pub const PICS: u32 = CBASE + 0x500;
/// Text pool: one 0x40-byte slot per id.
const POOL: u32 = CBASE + 0x600;
const SLOT: u32 = 0x40;

/// Text ids: the Controls rows' labels 0x7f40 + r, their values 0x7f50 + r, their "press" texts 0x7f60 + r, then the fixed ones.
const ID0: u32 = 0x7f40;
pub const ID_LAST: u32 = 0x7f80;
pub(super) const ID_VALUE: u32 = 0x7f50;
const ID_PRESS: u32 = 0x7f60;
pub const ID_CONTROLS: u32 = 0x7f70;
const ID_HEADER: u32 = 0x7f71;
pub const ID_DESC: u32 = 0x7f72;
/// The stepper's label ("Page") and the dynamic values "< section name >" of the Controls and the Graphics page.
pub(super) const ID_PAGE: u32 = 0x7f73;
const ID_SELC: u32 = 0x7f74;
pub(super) const ID_SELG: u32 = 0x7f75;
/// The HUD row: its label and its two values.
const ID_HUD: u32 = 0x7f76;
const ID_SHARP: u32 = 0x7f77;
const ID_ORIGINAL: u32 = 0x7f78;
/// The Renderer row: its label and its two values (pcrender.rs words them: a value that differs from the running API says restart).
const ID_RENDERER: u32 = 0x7f79;
const ID_VULKAN: u32 = 0x7f7a;
const ID_DX12: u32 = 0x7f7b;
/// The Skip intro row's label (its values are retail's Off / On).
const ID_INTRO: u32 = 0x7f7e;
/// The Start skips cutscenes row's label (its values are retail's On / Off, in that order: the byte is inverted).
const ID_SKIPCUT: u32 = 0x7f7f;
/// The Preset row: its label and the 4th choice "Custom" (Low, Medium, High are pcext's ids). Custom is a choice only while the
/// state is Custom, so the row cycles Low, Medium, High and from Custom back to Low.
const ID_PRESET: u32 = 0x7f7c;
pub(super) const ID_CUSTOM: u32 = 0x7f7d;
/// The Debug section: ids 0x7fa0 (the mode row's label), 0x7fa1..0x7fa5 (the key rows' labels), 0x7fa6..0x7faa (their values F1 .. F8);
/// 0x7fab..0x7faf are the Button glyphs row's and Metal's (glyphs.rs, ID_METAL); the pool slots are not shared. Off / On are retail's own 0x4f5b / 0x4f5a.
pub const ID_DBG0: u32 = 0x7fa0;
pub const ID_DBG_END: u32 = 0x7fb0;
const ID_DBG_KEY: u32 = 0x7fa1;
const ID_DBG_VAL: u32 = 0x7fa6;
const ID_ON: u32 = 0x4f5a;
const ID_OFF: u32 = 0x4f5b;
const ID_METAL: u32 = 0x7faf;

/// Sections of the Controls page and of the Graphics page.
const NSEC: usize = 11;
/// The Debug section (the last one) and the keys it lists.
const SEC_DEBUG: usize = 10;
const NDKEYS: usize = 5;
const NGSEC: usize = 7;
/// The Display section (HUD position, HUD scale, FPS limit, Render scale; pcdisp.rs): the Graphics page's 4th section, System and
/// Gameplay follow.
const SEC_DISPLAY: usize = 4;
/// The HUD row, the Renderer row and the Preset row in `GSECS` (the 19 rows of `pcext::ROWS` are 0..=18).
pub(super) const HUD_ROW: usize = 19;
pub(super) const RENDER_ROW: usize = 20;
pub(super) const PRESET_ROW: usize = 21;
/// The Skip intro row and the Start skips cutscenes row, the System section (the other three are full at six rows).
pub(super) const INTRO_ROW: usize = 22;
pub(super) const SKIPCUT_ROW: usize = 23;
/// The Display section's rows (pcdisp.rs).
pub(super) const HUDPOS_ROW: usize = 24;
pub(super) const HUDSCALE_ROW: usize = 25;
pub(super) const FPS_ROW: usize = 26;
pub(super) const RSCALE_ROW: usize = 27;
/// The Quick Select mouse row and the Stick dead zone row (pcinput.rs), the System section's 3rd and 4th.
pub(super) const WHEEL_ROW: usize = 28;
pub(super) const STICK_ROW: usize = 29;
/// The Button glyphs row (glyphs.rs), the System section's 5th.
pub(super) const GLYPH_ROW: usize = 30;
/// The Metal row (Effects section, pcmetal.rs): after the last of the fur / fire / smoke-shifted rows.
pub(super) const METAL_ROW: usize = 31;
/// The Menu style row (Modern / Classic), the System section's 6th and last (pcext/mstyle.rs).
pub(super) const MSTYLE_ROW: usize = 32;
/// The Metal reflections row (pcmrefl.rs): first in the More effects section (the Effects section is full at seven rows).
pub(super) const MREFL_ROW: usize = 33;
/// The Supporter drawings row (pcsupp.rs): last in the More effects section.
pub(super) const SUPP_ROW: usize = 34;
/// The Aspect ratio and Upscaler rows (Display section, pcmore.rs), the Quick Select pause and Fast travel rows (Gameplay section).
pub(super) const ASP_ROW: usize = 35;
pub(super) const UPS_ROW: usize = 36;
pub(super) const QSP_ROW: usize = 37;
pub(super) const FT_ROW: usize = 38;
/// The Frame Generation row (Screen section, after Renderer; pcfgen.rs).
pub(super) const FG_ROW: usize = 39;
/// The More effects section.
const SEC_MORE: usize = 2;
/// The Screen section. The Renderer row (Vulkan / DirectX 12) is not offered on Linux: there is no DirectX 12 backend there
/// (viewer/gpu/renderer.rs ignores a `renderer=dx12` line).
#[cfg(not(target_os = "linux"))]
const GSCREEN: &[usize] = &[3, 12, 9, RENDER_ROW, FG_ROW, HUD_ROW];
#[cfg(target_os = "linux")]
const GSCREEN: &[usize] = &[3, 12, 9, FG_ROW, HUD_ROW];
/// The Graphics sections as indices of `pcext::ROWS` (0 Lighting, 1 Ambient occlusion, 2 Dynamic shadows, 3 Display mode, 4 Water,
/// 5 Particles, 6 Sun rays, 7 Frame smoothing, 8 Vegetation, 9 Culling, 10 Global illumination, 11 Waves, 12 Draw distance, 13 Volumetric fog, 14 Volumetric clouds, 15 Fur, 16 Fur style).
/// Picture: Preset, Lighting, Ambient occlusion, Dynamic shadows, Global illumination, Sun rays. Effects (seven settings, the taller panel):
/// Water, Waves, Particles, Vegetation, Metal, Volumetric fog, Volumetric clouds. More effects: Fur, Fur style. Screen: Display mode,
/// Draw distance, Culling, Renderer, HUD. Display: HUD position, HUD scale, FPS, Render scale, Frame smoothing. Gameplay (the modern
/// Game tab's group): Quick Select pause, Fast travel. Graphics has at most seven settings plus the page stepper.
const GSECS: [&[usize]; NGSEC] = [&[PRESET_ROW, 0, 1, 2, 10, 6], &[4, 11, 5, 8, METAL_ROW, 13, 14], &[MREFL_ROW, 15, 16, 17, 18, SUPP_ROW], GSCREEN, &[HUDPOS_ROW, HUDSCALE_ROW, FPS_ROW, RSCALE_ROW, 7, ASP_ROW, UPS_ROW], &[INTRO_ROW, SKIPCUT_ROW, WHEEL_ROW, STICK_ROW, GLYPH_ROW, MSTYLE_ROW], &[QSP_ROW, FT_ROW]];

/// What a row does in a Controls section.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
	/// Binds the controller slots of action `a` (table index; `pcctl_pad.rs`).
	PadAction(usize),
	/// Binds the keyboard + mouse action `a` (table index; `pcctl_kbm.rs`).
	Action(usize),
	/// A mouse invert row (`pcmouseinv.rs`): axis 0 up/down, 1 left/right; its own byte, a three-choice item.
	MouseInv(usize),
	ResetKeys,
	ResetPad,
	/// The Debug mode row (byte `DBGB`).
	DebugMode,
	/// A row of the Debug section that only lists key `n` (F1, F2, F5, F6, F8).
	DebugKey(usize),
	Blank,
}

/// The rows of Controls section `sec` (at most six).
fn rows(sec: usize) -> Vec<Target> {
	let pads = |r: usize| pd::rows(r).into_iter().map(Target::PadAction).collect::<Vec<_>>();
	match sec {
		0..=4 => kb::rows(sec).into_iter().map(Target::Action).collect(),
		5 => kb::rows(sec).into_iter().map(Target::Action).chain([Target::MouseInv(0), Target::MouseInv(1)]).collect(),
		6..=8 => pads(sec - 6),
		SEC_DEBUG => std::iter::once(Target::DebugMode).chain((0..NDKEYS).map(Target::DebugKey)).collect(),
		_ => vec![Target::ResetKeys, Target::ResetPad],
	}
}

/// The target of row `r` (1..=6) in Controls section `sec`.
fn target(sec: usize, r: usize) -> Target {
	rows(sec).get(r.wrapping_sub(1)).copied().unwrap_or(Target::Blank)
}

// ---- texts ---------------------------------------------------------------------------------------------------------------

type T5 = [&'static [u8]; 5];

/// "Press a button" (controller rows), the reset rows' "Done".
static PRESS_PAD: T5 = [b"Press a button", b"Appuyez sur un bouton", b"Knopf dr\xa7cken", b"Pulsa un bot\x9cn", b"Premi un pulsante"];
static DONE: T5 = [b"Done", b"Fait", b"Fertig", b"Hecho", b"Fatto"];
static RESET_KEYS: T5 = [b"Reset keyboard", b"R\x8cinit. clavier", b"Tastatur zur\xa7ck", b"Restaurar teclado", b"Ripristina tastiera"];
static RESET_PAD: T5 = [b"Reset controller", b"R\x8cinit. manette", b"Controller zur\xa7ck", b"Restaurar mando", b"Ripristina joypad"];
static CONTROLS: T5 = [b"Controls", b"Commandes", b"Steuerung", b"Controles", b"Controlli"];
static HEADER: T5 = [b"Controls Options", b"Options commandes", b"Steuerungs-Optionen", b"Opciones de controles", b"Opzioni controlli"];
static PAGE: T5 = [b"Page", b"Page", b"Seite", b"P\x84gina", b"Pagina"];
static HUD: T5 = [b"HUD", b"HUD", b"HUD", b"HUD", b"HUD"];
static SHARP: T5 = [b"Sharp", b"Net", b"Scharf", b"N\x94tido", b"Nitido"];
static RENDERER: T5 = [b"Renderer", b"Rendu", b"Renderer", b"Motor gr\x84fico", b"Renderer"];
static INTRO: T5 = [b"Skip intro", b"Passer intro", b"Intro \xa7berspringen", b"Saltar intro", b"Salta intro"];
static SKIPCUT: T5 = [b"Start skips cutscenes", b"Passer avec Start", b"Mit Start \xa7berspringen", b"Saltar con Start", b"Salta con Start"];
static PRESET: T5 = [b"Preset", b"Pr\x8cr\x8cglage", b"Voreinst.", b"Preajuste", b"Preset"];
static CUSTOM: T5 = [b"Custom", b"Perso.", b"Eigene", b"Personal", b"Person."];
static ORIGINAL: T5 = [b"Original", b"Original", b"Original", b"Original", b"Originale"];
static DBG_MODE: T5 = [b"Debug mode", b"Mode debug", b"Debug-Modus", b"Modo debug", b"Modo debug"];
/// The key rows' labels (F1, F2, F5, F6, F8), no '+' or '%' (the font's glyphs are not known for '+').
static DBG_KEYS: [T5; NDKEYS] = [
	[b"Planet warp", b"Saut de niveau", b"Planeten-Warp", b"Salto de planeta", b"Salto a pianeta"],
	[b"Free camera", b"Cam\x8cra libre", b"Freie Kamera", b"C\x84mara libre", b"Camera libera"],
	[b"God mode", b"Invincible", b"Unverwundbar", b"Invencible", b"Invincibile"],
	[b"Infinite ammo", b"Munitions infinies", b"Unbegr. Munition", b"Munici\x9cn infinita", b"Munizioni infinite"],
	[b"10000 more bolts", b"10000 boulons de plus", b"10000 Schrauben mehr", b"10000 tornillos m\x84s", b"Altri 10000 bulloni"],
];
/// The key rows' values.
static DBG_VALS: [&[u8]; NDKEYS] = [b"F1", b"F2", b"F5", b"F6", b"F8"];
/// The Controls sections' names (the stepper shows them between `<` and `>`).
static SECTIONS: [T5; NSEC - kb::NSEC - pd::NSEC] = [
	[b"Reset", b"R\x8cinitialiser", b"Zur\xa7cksetzen", b"Restablecer", b"Ripristina"],
	[b"Debug", b"Debug", b"Debug", b"Debug", b"Debug"],
];
/// The Graphics sections' names.
static GSECTIONS: [T5; NGSEC] = [
	[b"Picture", b"Image", b"Bild", b"Imagen", b"Immagine"],
	[b"Effects", b"Effets", b"Effekte", b"Efectos visuales", b"Effetti"],
	[b"More effects", b"Plus d'effets", b"Mehr Effekte", b"M\x84s efectos", b"Altri effetti"],
	[b"Screen", b"Affichage", b"Bildschirm", b"Pantalla", b"Schermo"],
	pcdisp::SECTION,
	[b"System", b"Syst\x8ame", b"System", b"Sistema", b"Sistema"],
	[b"Gameplay", b"Jouabilit\x8c", b"Spielablauf", b"Jugabilidad", b"Gameplay"],
];

/// The name of Controls section `sec` in language `l`.
fn section_name(sec: usize, l: usize) -> &'static [u8] {
	let sec = sec.min(NSEC - 1);
	if sec < kb::NSEC {
		kb::SECTION_NAMES[sec][l]
	} else if sec < kb::NSEC + pd::NSEC {
		pd::SECTION_NAMES[sec - kb::NSEC][l]
	} else {
		SECTIONS[sec - kb::NSEC - pd::NSEC][l]
	}
}

/// "< name >": the stepper's value (`<` and `>` are glyphs of all three fonts).
fn stepper(name: &[u8]) -> Vec<u8> {
	let mut v = b"< ".to_vec();
	v.extend_from_slice(name);
	v.extend_from_slice(b" >");
	v
}

/// The widest value a cell draws today ("Appuyez sur une touche").
pub(super) const VALUE_MAX: usize = 22;

/// Every text of the pages in `lang`: `csec` / `gsec` are the sections the Controls / Graphics page shows, `playstation` picks the
/// PlayStation button names for the controller values.
pub fn text(id: u32, lang: usize, csec: usize, gsec: usize, playstation: bool) -> Vec<u8> {
	let l = lang.min(4);
	let fixed = |t: &T5| t[l].to_vec();
	match id {
		ID_CONTROLS => fixed(&CONTROLS),
		ID_HEADER => fixed(&HEADER),
		ID_DESC => fixed(if (kb::NSEC..kb::NSEC + pd::NSEC).contains(&csec) { &pd::HINT } else { &kb::HINT }),
		ID_PAGE => fixed(&PAGE),
		ID_SELC => stepper(section_name(csec, l)),
		ID_SELG => stepper(GSECTIONS[gsec.min(NGSEC - 1)][l]),
		ID_METAL => fixed(&[b"Metal", b"M\x8ctal", b"Metall", b"Metal", b"Metallo"]),
		ID_HUD => fixed(&HUD),
		ID_SHARP => fixed(&SHARP),
		ID_ORIGINAL => fixed(&ORIGINAL),
		ID_PRESET => fixed(&PRESET),
		ID_CUSTOM => fixed(&CUSTOM),
		ID_RENDERER => fixed(&RENDERER),
		ID_INTRO => fixed(&INTRO),
		ID_SKIPCUT => fixed(&SKIPCUT),
		ID_VULKAN | ID_DX12 => super::pcrender::value_text(id == ID_DX12, l),
		ID_DBG0 => fixed(&DBG_MODE),
		_ if (super::pcmouseinv::ID_V0..super::pcmouseinv::ID_V_END).contains(&id) => super::pcmouseinv::value_text(id, l),
		_ if glyphs::IDS.contains(&id) => glyphs::text(id, l),
		_ if (ID_DBG_KEY..ID_DBG_KEY + NDKEYS as u32).contains(&id) => fixed(&DBG_KEYS[(id - ID_DBG_KEY) as usize]),
		_ if (ID_DBG_VAL..ID_DBG_VAL + NDKEYS as u32).contains(&id) => DBG_VALS[(id - ID_DBG_VAL) as usize].to_vec(),
		_ if (ID0 + 1..ID0 + ROWS as u32).contains(&id) => match target(csec, (id - ID0) as usize) {
			Target::PadAction(a) => kb::label(a, l),
			Target::Action(a) => kb::label(a, l),
			Target::ResetKeys => fixed(&RESET_KEYS),
			Target::ResetPad => fixed(&RESET_PAD),
			Target::MouseInv(a) => super::pcmouseinv::label(a, l),
			_ => Vec::new(),
		},
		_ if (ID_VALUE + 1..ID_VALUE + ROWS as u32).contains(&id) => match target(csec, (id - ID_VALUE) as usize) {
			Target::Action(a) => kb::value(a),
			Target::MouseInv(a) => super::pcmouseinv::value_text(super::pcmouseinv::ID_V0 + super::pcmouseinv::mode(a) as u32, l),
			Target::PadAction(a) => pd::value(a, playstation),
			_ => Vec::new(),
		},
		_ if (ID_PRESS + 1..ID_PRESS + ROWS as u32).contains(&id) => match target(csec, (id - ID_PRESS) as usize) {
			Target::Action(_) => fixed(&kb::PRESS),
			Target::PadAction(_) => fixed(&PRESS_PAD),
			Target::ResetKeys | Target::ResetPad => fixed(&DONE),
			_ => Vec::new(),
		},
		_ => Vec::new(),
	}
}

/// The text of `id` as the pages show it now (the sections and the bindings as they are): `text_hook` writes it into the game's
/// RAM, the modern Controls page (`menu_model/pcpage.rs`) reads it.
pub fn text_now(id: u32, lang: usize) -> Vec<u8> {
	let (csec, gsec) = state(|st| (st.ctl.sec, st.gfx.sec));
	text(id, lang, csec, gsec, !super::prompts::real_pads_off() && super::hidpad::connected())
}

/// The label of the Reset keyboard / Reset controller row in `lang`.
pub fn reset_label(pad: bool, lang: usize) -> Vec<u8> {
	(if pad { RESET_PAD } else { RESET_KEYS })[lang.min(4)].to_vec()
}

/// Whether the controller values show PlayStation names (a real pad is connected), as `text_now` decides.
pub fn playstation_names() -> bool {
	!super::prompts::real_pads_off() && super::hidpad::connected()
}

/// The text lookup hand for the pages' ids (`pcext::text_hook`, 0x7f40..0x7f7f): the string goes into the id's pool slot and its
/// address into v0, in the game's language `lang`.
pub fn text_hook(c: &mut Cpu, id: u32, lang: usize) -> bool {
	if pcdisp::serves(id) {
		return pcdisp::text_hook(c, id, lang);
	}
	let s = text_now(id, lang);
	let at = if (ID_DBG0..ID_DBG_END).contains(&id) { DBG_POOL + SLOT * (id - ID_DBG0) } else { POOL + SLOT * (id - ID0) };
	let n = s.len().min(SLOT as usize - 1);
	for (i, b) in s[..n].iter().enumerate() {
		c.mem.wr8(at + i as u32, *b);
	}
	c.mem.wr8(at + n as u32, 0);
	c.s(2, at as u64);
	true
}

// ---- the tables and the pages --------------------------------------------------------------------------------------------

pub(super) fn copy_words(mem: &mut Mem, from: u32, to: u32, len: u32) {
	for o in (0..len).step_by(4) {
		let w = mem.rd32(from + o);
		mem.wr32(to + o, w);
	}
}

/// Writes a row table (6-word records) at `at`, padded with zero records to the table's capacity (no stale record of a longer
/// section stays behind the zero record).
pub(super) fn write_table(mem: &mut Mem, at: u32, t: &[u32]) {
	let mut t = t.to_vec();
	t.resize(TABLE_RECS * 6, 0);
	for (i, w) in t.iter().enumerate() {
		let a = at + 4 * i as u32;
		if mem.rd32(a) != *w {
			mem.wr32(a, *w);
		}
	}
}

/// The Controls row table of section `sec`: the stepper, then the section's rows (the zero record ends it; `write_table` pads).
fn table(sec: usize) -> Vec<u32> {
	let mut t = vec![ID_PAGE, CBASE, ID_SELC, ID_SELC, 0, 0];
	for (i, tg) in rows(sec).into_iter().enumerate() {
		let r = i as u32 + 1;
		t.extend(match tg {
			Target::DebugMode => [ID_DBG0, DBGB, ID_OFF, ID_ON, 0, 0],
			Target::MouseInv(a) => [ID0 + r, [super::pcmouseinv::UD_B, super::pcmouseinv::LR_B][a], super::pcmouseinv::ID_V0, super::pcmouseinv::ID_V0 + 1, super::pcmouseinv::ID_V0 + 2, 0],
			Target::DebugKey(k) => [ID_DBG_KEY + k as u32, CBASE + r, ID_DBG_VAL + k as u32, ID_DBG_VAL + k as u32, 0, 0],
			_ => [ID0 + r, CBASE + r, ID_VALUE + r, ID_PRESS + r, 0, 0],
		});
	}
	t
}

/// One of the extra Graphics rows (not in `pcext::ROWS`): label id, byte, up to four value texts (0 = no such value). The pause page
/// and the title's copy (`pcext/titlegfx.rs`) both come from here, so the two cannot drift apart. `custom`: the Preset row's 4th choice.
fn gfx_row_def(i: usize, custom: bool) -> [u32; 6] {
	match i {
		HUD_ROW => [ID_HUD, HUDB, ID_SHARP, ID_ORIGINAL, 0, 0],
		RENDER_ROW => [ID_RENDERER, RENDB, ID_VULKAN, ID_DX12, 0, 0],
		INTRO_ROW => [ID_INTRO, INTROB, ID_OFF, ID_ON, 0, 0],
		SKIPCUT_ROW => [ID_SKIPCUT, SKIPB, ID_ON, ID_OFF, 0, 0],
		HUDPOS_ROW => [pcdisp::ID_HUDPOS, pcdisp::POS_B, pcdisp::ID_P43, pcdisp::ID_CORNERS, 0, 0],
		HUDSCALE_ROW => [pcdisp::ID_HUDSCALE, pcdisp::POS_B + 1, pcdisp::ID_VSCALE, pcdisp::ID_VSCALE, 0, 0],
		FPS_ROW => [pcdisp::ID_FPS, pcdisp::POS_B + 2, pcdisp::ID_VFPS, pcdisp::ID_VFPS, 0, 0],
		RSCALE_ROW => [pcdisp::ID_RSCALE, pcdisp::POS_B + 3, pcdisp::ID_VRS, pcdisp::ID_VRS, 0, 0],
		WHEEL_ROW => [pcinput::ID_WHEEL, pcinput::WHEEL_B, ID_OFF, ID_ON, 0, 0],
		STICK_ROW => [pcinput::ID_STICK, pcinput::STICK_B, ID_HIGH, ID_MEDIUM, ID_LOW, 0],
		METAL_ROW => [ID_METAL, METAL_B, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW],
		GLYPH_ROW => [glyphs::ID_GLYPHS, glyphs::GLYPH_B, glyphs::ID_PS, glyphs::ID_LETTERS, glyphs::ID_SWITCH, 0],
		PRESET_ROW => [ID_PRESET, PRESETB, ID_LOW, ID_MEDIUM, ID_HIGH, if custom { ID_CUSTOM } else { 0 }],
		MREFL_ROW => [super::pcmrefl::ID_MREFL, super::pcmrefl::REFL_B, super::pcmrefl::ID_VMREFL, super::pcmrefl::ID_VMREFL, 0, 0],
		SUPP_ROW => [super::pcsupp::ID_SUPP, super::pcsupp::SUPP_B, ID_OFF, ID_ON, 0, 0],
		ASP_ROW => [super::pcmore::ID_ASPECT, super::pcmore::ASP_B, super::pcmore::ID_VASPECT, super::pcmore::ID_VASPECT, 0, 0],
		UPS_ROW => [super::pcmore::ID_UPS, super::pcmore::UPS_B, super::pcmore::ID_VUPS, super::pcmore::ID_VUPS, 0, 0],
		QSP_ROW => [super::pcmore::ID_QSP, super::pcmore::QSP_B, ID_OFF, ID_ON, 0, 0],
		FT_ROW => [super::pcmore::ID_FT, super::pcmore::FT_B, ID_OFF, ID_ON, 0, 0],
		FG_ROW => [super::pcfgen::ID_FG, super::pcfgen::FG_B, super::pcfgen::ID_VFG, super::pcfgen::ID_VFG, 0, 0],
		MSTYLE_ROW => [ID_MSTYLE, MSTYLEB, ID_MSTYLE_MODERN, ID_LIGHT_CLASSIC, 0, 0],
		_ => [0; 6],
	}
}

/// A Graphics row by its index in `GSECS`: label id, byte, up to four value texts (the title's copy uses this too).
pub(super) fn gfx_row(i: usize, custom: bool) -> [u32; 6] {
	if i < super::pcext::ROWS.len() { super::pcext::ROWS[i] } else { gfx_row_def(i, custom) }
}

/// The Graphics page's sections (the pause page and the title's copy): how many, the rows of one and the Display one.
pub(super) fn gfx_sections() -> usize {
	NGSEC
}
pub(super) fn gfx_section(sec: usize) -> &'static [usize] {
	GSECS[sec.min(NGSEC - 1)]
}
pub(super) const GFX_DISPLAY: usize = SEC_DISPLAY;
pub(super) const GFX_MORE: usize = SEC_MORE;

/// The texts the title's Graphics mirror needs that are this file's (the pause page's come through the game's text hook): the page
/// stepper in `lang` for section `sec`, the HUD / Renderer / Preset / Skip intro / Skip cutscenes rows and their values.
pub(super) fn title_texts(lang: usize, sec: usize) -> Vec<(u32, String)> {
	[ID_PAGE, ID_SELG, ID_METAL, ID_HUD, ID_SHARP, ID_ORIGINAL, ID_PRESET, ID_CUSTOM, ID_RENDERER, ID_INTRO, ID_SKIPCUT, ID_VULKAN, ID_DX12, glyphs::ID_GLYPHS, glyphs::ID_PS, glyphs::ID_LETTERS, glyphs::ID_SWITCH]
		.into_iter()
		.map(|id| (id, text(id, lang, 0, sec, false).iter().map(|&b| b as char).collect()))
		.collect()
}

/// The Graphics row table of section `sec`: the stepper, then the section's rows of `rows` (`pcext::ROWS`) and the extra rows
/// (`gfx_row_def`). `custom`: the Preset row has its 4th choice "Custom" (the state is Custom).
fn gfx_table(rows: &[[u32; 6]], sec: usize, custom: bool) -> Vec<u32> {
	let mut t = vec![ID_PAGE, G_SEL, ID_SELG, ID_SELG, 0, 0];
	for &i in GSECS[sec.min(NGSEC - 1)] {
		t.extend(if i < rows.len() { rows[i] } else { gfx_row_def(i, custom) });
	}
	t
}

/// Builds the Controls page in reserved RAM: a copy of the Camera Options page `cp` (header item `hdr`, multi-choice item `val`),
/// the same way `pcext::patch_graphics` builds the Graphics page; `sound_seq` is the Sound page's slot-3 panel sequence (the taller
/// value panel that holds the 7 rows). The caller repoints the Options list afterwards.
pub fn build_page(mem: &mut Mem, cp: u32, hdr: u32, val: u32, sound_seq: Option<u32>) {
	copy_words(mem, cp, CPAGE, 0x60);
	for o in (0x60..0x100).step_by(4) {
		mem.wr32(CPAGE + o, 0);
	}
	if let Some(seq) = sound_seq {
		mem.wr32(CPAGE + 0xc, seq);
	}
	for o in (0x40..0x60).step_by(4) {
		match mem.rd32(CPAGE + o) {
			a if a == hdr => mem.wr32(CPAGE + o, CHDR),
			a if a == val => mem.wr32(CPAGE + o, CVAL),
			_ => {}
		}
	}
	copy_words(mem, hdr, CHDR, 0x58);
	mem.wr32(CHDR + 0x34, ID_HEADER);
	copy_words(mem, val, CVAL, 0x40);
	mem.wr32(CVAL + 0x34, CTAB);
	mem.wr32(CVAL + 0x38, 0);
	mem.wr32(CVAL + 0x3c, 0);
	let fresh = mem.rd32(MAGIC_AT) != MAGIC;
	if fresh {
		mem.wr8(DBGB, super::devtools::menu() as u8);
		super::pcmouseinv::seed(mem);
	}
	state(|st| {
		if fresh {
			st.ctl.seen = 0xff;
		}
		write_table(mem, CTAB, &table(st.ctl.sec));
	});
	mem.wr32(MAGIC_AT, MAGIC);
}

/// Writes the Graphics page's row table (`gtab`, the page's item table in pcext's block) for the section the page shows, from
/// `rows` (`pcext::ROWS`). On RAM that never held the page (`G_MAGIC_AT` unset: a start, a savestate load) the stepper byte and
/// the HUD and Renderer bytes are set up too (`hud_sharp`, `dx12`: the options file's rows); a rebuild after a level load keeps the
/// player's bytes.
pub fn build_gfx(mem: &mut Mem, gtab: u32, rows: &[[u32; 6]], hud_sharp: bool, dx12: bool) {
	let fresh = mem.rd32(G_MAGIC_AT) != MAGIC;
	if fresh {
		mem.wr8(G_SEL, 0);
		mem.wr8(HUDB, !hud_sharp as u8);
		mem.wr8(RENDB, dx12 as u8);
		mem.wr8(INTROB, super::pcintro::skip() as u8);
		mem.wr8(SKIPB, !super::pcskip::skip() as u8);
		pcdisp::build(mem);
		super::pcmrefl::build(mem);
		super::pcmore::build(mem);
		mem.wr8(pcinput::WHEEL_B, pcinput::wheel_byte());
		mem.wr8(pcinput::STICK_B, pcinput::stick_byte());
		mem.wr8(glyphs::GLYPH_B, glyphs::byte());
		mem.wr8(METAL_B, super::pcmetal::byte());
		mem.wr8(super::pcsupp::SUPP_B, super::pcsupp::enabled() as u8);
		mem.wr8(MSTYLEB, super::pcext::mstyle::byte());
		mem.wr32(G_MAGIC_AT, MAGIC);
	}
	state(|st| {
		if fresh {
			st.gfx.seen = 0xff;
			st.preset_seen = 0xff;
		}
		write_table(mem, gtab, &gfx_table(rows, st.gfx.sec, st.custom));
	});
}

/// The Graphics section on show (`SEC_SCREEN`: the one with the Culling row).
pub(super) const SEC_SCREEN: usize = 3;
pub(super) fn gfx_sec() -> usize {
	state(|st| st.gfx.sec)
}

/// The page's address (the Options record's argument).
pub const PAGE_AT: u32 = CPAGE;

/// The HUD row: true = Sharp. None until the Graphics page was built on this RAM.
pub fn hud_sharp(mem: &mut Mem) -> Option<bool> {
	(mem.rd32(G_MAGIC_AT) == MAGIC).then(|| mem.rd8(HUDB) == 0)
}

/// The Renderer row: true = DirectX 12. None until the Graphics page was built on this RAM.
pub fn renderer_dx12(mem: &mut Mem) -> Option<bool> {
	(mem.rd32(G_MAGIC_AT) == MAGIC).then(|| mem.rd8(RENDB) != 0)
}

/// The Graphics page was built on this RAM (its row bytes may be read).
pub(super) fn gfx_built(mem: &mut Mem) -> bool {
	mem.rd32(G_MAGIC_AT) == MAGIC
}

/// The Skip intro row: true = On. None until the Graphics page was built on this RAM.
pub fn intro_skip(mem: &mut Mem) -> Option<bool> {
	(mem.rd32(G_MAGIC_AT) == MAGIC).then(|| mem.rd8(INTROB) != 0)
}

/// The Start skips cutscenes row: true = On (the byte is inverted, 0 = On). None until the Graphics page was built on this RAM.
pub fn skipcut_on(mem: &mut Mem) -> Option<bool> {
	(mem.rd32(G_MAGIC_AT) == MAGIC).then(|| mem.rd8(SKIPB) == 0)
}

/// The Menu style row: true = Classic. None until the Graphics page was built on this RAM.
pub fn menu_classic(mem: &mut Mem) -> Option<bool> {
	(mem.rd32(G_MAGIC_AT) == MAGIC).then(|| mem.rd8(MSTYLEB) != 0)
}

/// The rows of this file the modern screen sets (`pcext/ui_bridge.rs`): `Renderer` 1 = DirectX 12, `Intro` 1 = skip, `SkipCut` 1 = On
/// (the byte is inverted), `MenuStyle` 1 = Classic.
#[derive(Clone, Copy)]
pub(super) enum UiByte {
	Renderer,
	Intro,
	SkipCut,
	MenuStyle,
}

/// Writes the modern screen's value into the row's byte when the Graphics page exists on this RAM, so the page's `sync` sees the new
/// value as the current one (a differing byte would be read as the player's press and undo the change).
pub(super) fn set_ui_byte(mem: &mut Mem, which: UiByte, v: bool) {
	if mem.rd32(G_MAGIC_AT) != MAGIC {
		return;
	}
	match which {
		UiByte::Renderer => mem.wr8(RENDB, v as u8),
		UiByte::Intro => mem.wr8(INTROB, v as u8),
		UiByte::SkipCut => mem.wr8(SKIPB, !v as u8),
		UiByte::MenuStyle => mem.wr8(MSTYLEB, v as u8),
	}
}

/// The Preset row, once per change: the choice the player made (0 Low, 1 Medium, 2 High) since the last look. None until the
/// Graphics page was built on this RAM; the first look is the baseline.
pub fn preset_chosen(mem: &mut Mem) -> Option<u8> {
	if mem.rd32(G_MAGIC_AT) != MAGIC {
		return None;
	}
	let b = mem.rd8(PRESETB);
	state(|st| {
		let seen = std::mem::replace(&mut st.preset_seen, b);
		(seen != 0xff && seen != b && b < 3).then_some(b)
	})
}

/// The title Graphics page wrote the Preset byte itself (`presets::title_open` / `choose_title`): that value is now the baseline
/// `preset_chosen` compares with, so the next level start does not read it as a press on the pause page.
pub fn preset_baseline(byte: u8) {
	state(|st| st.preset_seen = byte);
}

/// The Preset row shows `byte` (0 Low, 1 Medium, 2 High, 3 Custom): writes it, and the 4th choice appears with Custom only (a
/// changed table is rewritten). `rows` / `gtab`: the Graphics page's rows and table (`pcext`).
pub fn preset_show(mem: &mut Mem, rows: &[[u32; 6]], gtab: u32, byte: u8) {
	if mem.rd32(G_MAGIC_AT) != MAGIC {
		return;
	}
	if mem.rd8(PRESETB) != byte {
		mem.wr8(PRESETB, byte);
	}
	state(|st| {
		st.preset_seen = byte;
		if st.custom != (byte == 3) {
			st.custom = byte == 3;
			write_table(mem, gtab, &gfx_table(rows, st.gfx.sec, st.custom));
		}
	});
}

/// A preset sets the HUD row (true = Sharp).
pub fn set_hud(mem: &mut Mem, sharp: bool) {
	if mem.rd32(G_MAGIC_AT) == MAGIC {
		mem.wr8(HUDB, !sharp as u8);
	}
}

/// The Controls page was built on this RAM (its row bytes may be read).
pub(super) fn controls_built(mem: &mut Mem) -> bool {
	mem.rd32(MAGIC_AT) == MAGIC
}

/// The Debug mode row: true = On. None until the Controls page was built on this RAM.
pub fn debug_on(mem: &mut Mem) -> Option<bool> {
	(mem.rd32(MAGIC_AT) == MAGIC).then(|| mem.rd8(DBGB) != 0)
}

// ---- worker: the steppers and the rows' bytes ----------------------------------------------------------------------------

/// A page's section stepper: the section shown and the selector byte as last seen (0xff: not known yet, the next look is the
/// baseline).
pub(super) struct Pager {
	pub(super) sec: usize,
	pub(super) seen: u8,
}

struct St {
	ctl: Pager,
	gfx: Pager,
	/// The Controls row a capture waits for.
	row: Option<usize>,
	/// A reset row that reads "Done": (row, fields left).
	done: Option<(usize, u32)>,
	/// Fields Left / Right have been held (auto-repeat).
	held: [u32; 2],
	/// The Preset row reads Custom (its table has the 4th choice), and its byte as last seen (0xff: not known yet).
	custom: bool,
	preset_seen: u8,
}

impl St {
	const NEW: St = St { ctl: Pager { sec: 0, seen: 0xff }, gfx: Pager { sec: 0, seen: 0xff }, row: None, done: None, held: [0; 2], custom: false, preset_seen: 0xff };
}

/// The worker's state: a mutex in the game, a thread-local in tests (each test thread has its own, like `binds`).
#[cfg(not(test))]
fn state<R>(f: impl FnOnce(&mut St) -> R) -> R {
	static ST: Mutex<St> = Mutex::new(St::NEW);
	f(&mut ST.lock().unwrap_or_else(|e| e.into_inner()))
}
#[cfg(test)]
fn state<R>(f: impl FnOnce(&mut St) -> R) -> R {
	thread_local! { static ST: std::cell::RefCell<St> = const { std::cell::RefCell::new(St::NEW) }; }
	ST.with(|s| f(&mut s.borrow_mut()))
}

/// Fields a reset row keeps reading "Done".
const DONE_FIELDS: u32 = 50;

/// The page-menu mode word (`mouse_menu::MODE` while a pause page is open).
pub(super) const MENU_MODE: u32 = 0x15f5a4;

/// Whether the open pause page is `page` with `item` as its current item and its cursor on row 0, idle (not flying in or out).
pub(super) fn on_selector(mem: &mut Mem, level: u32, page: u32, item: u32) -> bool {
	open_page(mem, level, page, |rd, snap| snap.id == 0x1d && snap.state == 0x1d && snap.cur == item && rd(item + 0x38) == 0)
}

/// Whether the pause menu has `page` open (flying in or out included) and `more` agrees (it gets the RAM reader and the menu snapshot).
pub(super) fn open_page(mem: &mut Mem, level: u32, page: u32, more: impl FnOnce(&dyn Fn(u32) -> u32, &mouse_menu::MenuSnap) -> bool) -> bool {
	if mem.rd32(crate::retail::disc::va(MENU_MODE)) != mouse_menu::MODE {
		return false;
	}
	let ram = &mem.ram;
	let rd = |a: u32| -> u32 {
		let i = (a & 0x01ff_ffff) as usize;
		ram.get(i..i + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
	};
	match mouse_menu::read(&rd, level, mouse_menu::MODE) {
		Some(snap) => snap.page == page && more(&rd, &snap),
		None => false,
	}
}

/// Whether a held direction steps in this field: on the press, then after 12 fields every 4 (the menu's own auto-repeat feel).
pub(super) fn repeat_due(held: u32) -> bool {
	held == 1 || (held >= 13 && (held - 13) % 4 == 0)
}

/// One field of a stepper: the section after this field's input, if it changed. `sel` is the selector byte (the retail item's
/// Cross toggled it: the next section), `dir` Left / Right this field (the previous / next section), `on_row` (asked only when
/// there is an input) whether the page's cursor is on the selector. The steps wrap around the `n` sections.
pub(super) fn pager_step(p: &mut Pager, sel: u8, dir: [bool; 2], n: usize, on_row: impl FnOnce() -> bool) -> Option<usize> {
	let cross = p.seen != 0xff && p.seen != sel;
	p.seen = sel;
	if !(cross || dir[0] != dir[1]) || !on_row() {
		return None;
	}
	let next = if cross || dir[1] { (p.sec + 1) % n } else { (p.sec + n - 1) % n };
	(next != p.sec).then_some(next)
}

/// The Controls page shows section `sec`: a capture is off, the rows' bytes are back to 0 (they are other functions now), the
/// table is the section's.
fn go_ctl(mem: &mut Mem, st: &mut St, sec: usize) {
	st.ctl.sec = sec;
	binds::capture_cancel();
	binds::capture_ended();
	st.row = None;
	st.done = None;
	for r in 1..ROWS as u32 {
		if mem.rd8(CBASE + r) != 0 {
			mem.wr8(CBASE + r, 0);
		}
	}
	write_table(mem, CTAB, &table(sec));
}

/// The Graphics page shows section `sec`.
fn go_gfx(mem: &mut Mem, st: &mut St, sec: usize, rows: &[[u32; 6]], gtab: u32) {
	st.gfx.sec = sec;
	write_table(mem, gtab, &gfx_table(rows, sec, st.custom));
}

/// Worker, each field of a level (`pcext::pre_field`, after `sync`): the steppers of the pages that are in RAM. `buttons` is the
/// field's pad word (Left / Right, also the mouse wheel), `rows` / `gtab` the Graphics page's rows and table (`pcext`).
pub fn field(mem: &mut Mem, level: u32, buttons: u16, rows: &[[u32; 6]], gtab: u32) {
	let (c, g) = (mem.rd32(MAGIC_AT) == MAGIC, mem.rd32(G_MAGIC_AT) == MAGIC);
	if !c && !g {
		return;
	}
	state(|st| {
		let mut dir = [false; 2];
		for (i, bit) in [mouse_menu::LEFT, mouse_menu::RIGHT].into_iter().enumerate() {
			st.held[i] = if buttons & bit != 0 { st.held[i].saturating_add(1) } else { 0 };
			dir[i] = repeat_due(st.held[i]);
		}
		if c {
			keys_on_row(mem, level, st, buttons);
			let sel = mem.rd8(CBASE);
			if let Some(n) = pager_step(&mut st.ctl, sel, dir, NSEC, || on_selector(mem, level, CPAGE, CVAL)) {
				go_ctl(mem, st, n);
			}
		}
		if g {
			let sel = mem.rd8(G_SEL);
			if let Some(n) = pager_step(&mut st.gfx, sel, dir, NGSEC, || on_selector(mem, level, GPAGE, GVAL)) {
				go_gfx(mem, st, n, rows, gtab);
			}
			if st.gfx.sec == SEC_DISPLAY {
				pcdisp::field(mem, level, dir);
				super::pcmore::field(mem, level, dir);
			} else if st.gfx.sec == SEC_MORE {
				super::pcmrefl::field(mem, level, dir);
			} else if st.gfx.sec == SEC_SCREEN {
				super::pcfgen::field(mem, level, dir);
			}
		}
	});
}

/// Square / Tab (slot 2) and Delete (clear) on the keyboard row under the cursor (`pcctl_kbm.rs`); the requests are always drained.
fn keys_on_row(mem: &mut Mem, level: u32, st: &mut St, buttons: u16) {
	let ask = kb::request(buttons);
	if ask == kb::Edit::Nothing || !(0..kb::NSEC + pd::NSEC).contains(&st.ctl.sec) || binds::capturing() || crate::modern_menu::pause_host::table_up() {
		return;
	}
	let row = std::cell::Cell::new(0xffu32);
	if !open_page(mem, level, CPAGE, |rd, snap| {
		row.set(rd(CVAL + 0x38));
		snap.id == 0x1d && snap.state == 0x1d && snap.cur == CVAL
	}) {
		return;
	}
	let r = row.get() as usize;
	let t = target(st.ctl.sec, r);
	if let Target::Action(a) | Target::PadAction(a) = t {
		if ask == kb::Edit::Clear {
			if matches!(t, Target::PadAction(_)) {
				pd::clear(a);
			} else {
				kb::clear(a);
			}
		} else if mem.rd8(CBASE + r as u32) == 0 {
			kb::want_slot2();
			mem.wr8(CBASE + r as u32, 1);
		}
	}
}

/// Worker, every field after the page patch: reads the row bytes the retail item flipped and acts on them (module doc). Does
/// nothing until `build_page` ran on this RAM.
pub fn sync(mem: &mut Mem) {
	if mem.rd32(MAGIC_AT) == MAGIC {
		state(|st| sync_rows(mem, st));
	}
}

fn sync_rows(mem: &mut Mem, st: &mut St) {
	let sec = st.ctl.sec;
	if binds::capture_ended() {
		if let Some(r) = st.row.take() {
			mem.wr8(CBASE + r as u32, 0);
		}
	}
	if let Some((r, n)) = st.done {
		st.done = (n > 1).then_some((r, n - 1));
		if st.done.is_none() {
			mem.wr8(CBASE + r as u32, 0);
		}
	}
	for r in 1..ROWS {
		if mem.rd8(CBASE + r as u32) == 0 || st.row == Some(r) || st.done.is_some_and(|d| d.0 == r) {
			continue;
		}
		match target(sec, r) {
			Target::PadAction(_) | Target::Action(_) => {
				// A press on another row while one waits: that capture ends first.
				if let Some(old) = st.row.take() {
					binds::capture_cancel();
					binds::capture_ended();
					mem.wr8(CBASE + old as u32, 0);
				}
				match target(sec, r) {
					Target::Action(a) => kbm::capture_begin(kbm::Cap::Kbm(a, kb::take_slot())),
					Target::PadAction(a) => kbm::capture_begin(kbm::Cap::Pad(a, kb::take_slot())),
					_ => {}
				}
				st.row = Some(r);
			}
			Target::ResetKeys | Target::ResetPad => {
				if let Some((old, _)) = st.done.take() {
					mem.wr8(CBASE + old as u32, 0);
				}
				if target(sec, r) == Target::ResetKeys {
					binds::reset_keys();
				} else {
					binds::reset_pad();
				}
				st.done = Some((r, DONE_FIELDS));
			}
			// The Debug section's key rows only show text (the mode row's byte is `DBGB`, read by `debug_on`).
			Target::Blank | Target::DebugMode | Target::DebugKey(_) => mem.wr8(CBASE + r as u32, 0),
			Target::MouseInv(_) => {}
		}
	}
}

/// Test hook: RAM with the Controls page built, showing Controls section `sec`.
#[cfg(test)]
pub(super) fn test_page(sec: usize) -> Mem {
	let mut mem = Mem::new();
	mem.wr32(MAGIC_AT, MAGIC);
	state(|st| go_ctl(&mut mem, st, sec));
	mem
}

/// Test hook: the Graphics page shows section `sec` (what `field` does after a step).
#[cfg(test)]
pub fn test_gfx_section(mem: &mut Mem, sec: usize, rows: &[[u32; 6]], gtab: u32) {
	state(|st| go_gfx(mem, st, sec, rows, gtab));
}

#[cfg(test)]
mod tests {
	use super::*;
	use winit::keyboard::KeyCode as K;

	/// The 19 rows of `pcext::ROWS` as far as the tables care: distinct labels.
	fn fake_rows() -> Vec<[u32; 6]> {
		(0..19u32).map(|i| [0x100 + i, 0x200 + i, 0x300, 0x301, 0, 0]).collect()
	}

	fn go(mem: &mut Mem, sec: usize) {
		state(|st| go_ctl(mem, st, sec));
	}

	fn csec() -> usize {
		state(|st| st.ctl.sec)
	}

	#[test]
	fn region_and_ids_fit() {
		assert!(RANGE.end <= BASE + 0x10000);
		assert!(RANGE.start >= BASE + 0x2000 + 0x8000, "above the prompts pool (BASE+0x2000..0xA000)");
		assert!(POOL + SLOT * (ID_LAST - ID0) <= RANGE.end);
		// The Debug pool: 16 slots in BASE+0xB800..0xBC00, outside the Controls region, inside the canary range.
		assert_eq!(DBG_RANGE, BASE + 0xB800..BASE + 0xBC00);
		assert!(DBG_POOL + SLOT * (ID_DBG_END - ID_DBG0) == DBG_RANGE.end && DBG_RANGE.end <= BASE + 0x10000);
		assert!(DBG_RANGE.start >= RANGE.end || DBG_RANGE.end <= RANGE.start, "no overlap with the Controls region");
		assert!(RANGE.contains(&DBGB) && DBGB != HUDB && DBGB < G_MAGIC_AT && ID_DBG_VAL + NDKEYS as u32 <= ID_DBG_END && ID_DBG_KEY + NDKEYS as u32 <= ID_DBG_VAL);
		assert!(PICS + 8 * 14 <= POOL && CTAB + 24 * TABLE_RECS as u32 <= PICS && CVAL + 0x40 <= CTAB && CHDR + 0x58 <= CVAL && CPAGE + 0x100 <= CHDR && MAGIC_AT + 4 <= CPAGE);
		assert!(CBASE + ROWS as u32 <= G_SEL && HUDB < G_MAGIC_AT && G_MAGIC_AT + 4 <= MAGIC_AT);
		assert!(RANGE.contains(&RENDB) && RENDB != HUDB && RENDB != DBGB && RENDB != G_SEL && RENDB < G_MAGIC_AT && ID_DX12 < ID_LAST);
		assert!(RANGE.contains(&INTROB) && ![HUDB, RENDB, DBGB, G_SEL, PRESETB].contains(&INTROB) && INTROB < G_MAGIC_AT && ID_INTRO < ID_LAST);
		assert!(RANGE.contains(&SKIPB) && ![HUDB, RENDB, DBGB, G_SEL, PRESETB, INTROB].contains(&SKIPB) && SKIPB < G_MAGIC_AT && ID_SKIPCUT < ID_LAST && ID_SKIPCUT != ID_INTRO);
		assert!(RANGE.contains(&PRESETB) && ![HUDB, RENDB, DBGB, G_SEL].contains(&PRESETB) && PRESETB < G_MAGIC_AT && ID_CUSTOM < ID_LAST && ID_PRESET > ID_RENDERER);
		assert_eq!(table(0).len(), 7 * 6);
		assert!(ID_HUD < ID_LAST && ID_ORIGINAL < ID_LAST);
		assert_eq!(TABLE_RECS, 9, "the Graphics table (pcext GTAB) has room for 14 records");
	}

	#[test]
	fn every_text_is_short_font_bytes_in_every_language() {
		for lang in 0..5 {
			for csec in 0..NSEC {
				for gsec in 0..NGSEC {
					for id in ID0..ID_LAST {
						let s = text(id, lang, csec, gsec, false);
						assert!(s.len() < SLOT as usize - 1, "{id:#x} {csec} {lang}");
						assert!(s.iter().all(|&b| (0x20..0x7f).contains(&b) || (0x80..=0xb0).contains(&b)), "{id:#x} {csec} {lang}: {s:?}");
						assert!(!s.contains(&b'%'), "0x25 draws the Cross glyph: {s:?}");
					}
				}
			}
		}
		// The rows a section uses are never empty, the unused rows are (the Debug section has ids of its own, tested below).
		for lang in 0..5 {
			for csec in 0..SEC_DEBUG {
				for r in 1..ROWS {
					let used = r <= rows(csec).len();
					for base in [ID0, ID_VALUE, ID_PRESS] {
						let empty = text(base + r as u32, lang, csec, 0, false).is_empty();
						// A reset row has no value text (it only shows Done).
						let reset_value = base == ID_VALUE && csec == 9;
						// A mouse invert row cycles its own three choices: no press text.
						let mouse_press = base == ID_PRESS && csec == 5 && r >= 5;
						assert_eq!(empty, !used || reset_value || mouse_press, "{base:#x} row {r} section {csec} {lang}");
					}
				}
			}
			for id in [ID_CONTROLS, ID_HEADER, ID_DESC, ID_PAGE, ID_HUD, ID_SHARP, ID_ORIGINAL, ID_RENDERER, ID_VULKAN, ID_DX12, ID_PRESET, ID_CUSTOM, ID_INTRO, ID_SKIPCUT] {
				assert!(!text(id, lang, 0, 0, false).is_empty());
			}
		}
	}

	/// The Debug section: its rows, table, texts (all five languages, font bytes, short) and the text hook's pool.
	#[test]
	fn the_debug_section_has_its_own_ids() {
		assert_eq!(rows(SEC_DEBUG).len(), 6);
		assert_eq!(table(SEC_DEBUG).len(), 7 * 6);
		let t = table(SEC_DEBUG);
		assert_eq!(&t[6..12], [ID_DBG0, DBGB, ID_OFF, ID_ON, 0, 0], "the mode row: byte DBGB, retail's Off / On");
		assert_eq!(&t[12..18], [0x7fa1, CBASE + 2, 0x7fa6, 0x7fa6, 0, 0]);
		assert_eq!(&t[36..42], [0x7fa5, CBASE + 6, 0x7faa, 0x7faa, 0, 0]);
		for lang in 0..5 {
			for id in ID_DBG0..ID_DBG_END {
				let s = text(id, lang, SEC_DEBUG, 0, false);
				assert!(s.len() < SLOT as usize - 1, "{id:#x} {lang}");
				assert!(s.iter().all(|&b| (0x20..0x7f).contains(&b) || (0x80..=0xb0).contains(&b)), "{id:#x} {lang}: {s:?}");
				assert!(!s.contains(&b'%') && !s.contains(&b'+'), "{id:#x} {lang}");
				assert_eq!(s.is_empty(), id >= ID_DBG_VAL + NDKEYS as u32 && !glyphs::IDS.contains(&id) && id != ID_METAL, "{id:#x} {lang}: 0x7fa0..0x7faa have text, 0x7fab..0x7fae are the glyph row's (own text), the rest none");
				if (ID_DBG_KEY..ID_DBG_VAL).contains(&id) || id == ID_DBG0 {
					assert!(s.len() <= 24, "a label must fit the row: {id:#x} {lang}: {s:?}");
				}
			}
			assert_eq!(text(ID_SELC, lang, SEC_DEBUG, 0, false), b"< Debug >");
		}
		assert_eq!((text(ID_DBG0, 0, 10, 0, false), text(0x7fa2, 1, 10, 0, false), text(0x7fa4, 3, 10, 0, false), text(0x7fa8, 2, 10, 0, false)), (b"Debug mode".to_vec(), b"Cam\x8cra libre".to_vec(), b"Munici\x9cn infinita".to_vec(), b"F5".to_vec()));
		// The Debug section is the last one: the Page stepper wraps from Reset to it and from it to the first section.
		assert_eq!(SEC_DEBUG, NSEC - 1);
		let mut p = Pager { sec: 9, seen: 0 };
		assert_eq!(pager_step(&mut p, 1, [false; 2], NSEC, || true), Some(10));
		p.sec = 10;
		assert_eq!(pager_step(&mut p, 0, [false; 2], NSEC, || true), Some(0));
	}

	/// The Debug mode row: the byte is set from the menu switch on fresh RAM, `debug_on` reads it, the key rows' bytes are cleared.
	#[test]
	fn the_debug_row_byte_is_read_and_the_key_rows_are_inert() {
		let _g = crate::retail::pcopts::globals_guard(); // binds and the capture state are process-wide
		let mut mem = Mem::new();
		assert_eq!(debug_on(&mut mem), None, "before the page was built");
		let mut mem = page_mem();
		assert!(debug_on(&mut mem).is_some(), "fresh RAM takes the menu switch (the page is built)");
		go(&mut mem, SEC_DEBUG);
		let rec = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..6).map(|k| mem.rd32(CTAB + 24 * i + 4 * k)).collect() };
		assert_eq!(rec(&mut mem, 1), [ID_DBG0, DBGB, ID_OFF, ID_ON, 0, 0]);
		assert_eq!(rec(&mut mem, 6), [0x7fa5, CBASE + 6, 0x7faa, 0x7faa, 0, 0]);
		assert_eq!(rec(&mut mem, 7), [0; 6]);
		mem.wr8(DBGB, 1);
		assert_eq!(debug_on(&mut mem), Some(true));
		mem.wr8(DBGB, 0);
		assert_eq!(debug_on(&mut mem), Some(false));
		// A press on a key row changes nothing but its own byte (back to 0); no capture starts.
		mem.wr8(CBASE + 3, 1);
		sync(&mut mem);
		assert_eq!((mem.rd8(CBASE + 3), binds::capturing(), debug_on(&mut mem)), (0, false, Some(false)));
		go(&mut mem, 0);
	}

	/// The stepper's value shows the section's name between `<` and `>`.
	#[test]
	fn the_stepper_shows_the_section() {
		let _g = crate::retail::pcopts::globals_guard(); // binds and the capture state are process-wide
		assert_eq!(text(ID_SELC, 0, 0, 0, false), b"< Keyboard, mouse 1/2 >");
		assert_eq!(text(ID_SELC, 2, 8, 0, false), b"< Controller 3/3 >");
		assert_eq!(text(ID_SELG, 0, 0, 1, false), b"< Effects >");
		assert_eq!(text(ID_SELG, 3, 0, 3, false), b"< Pantalla >");
		assert_eq!(text(ID_SELG, 0, 0, 2, false), b"< More effects >");
		assert_eq!(text(ID_PAGE, 2, 0, 0, false), b"Seite");
		assert_eq!((text(ID_HUD, 0, 0, 0, false), text(ID_SHARP, 3, 0, 0, false), text(ID_ORIGINAL, 4, 0, 0, false)), (b"HUD".to_vec(), b"N\x94tido".to_vec(), b"Originale".to_vec()));
	}

	/// No two PC text ids are the same: every id a page draws belongs to one job (a collision once made the Button glyphs row read
	/// "Like camera", review B 2026-11-05).
	#[test]
	fn no_two_pc_text_ids_collide() {
		use std::ops::Range;
		let one = |id: u32| id..id + 1;
		let r = |a: u32, n: usize| a..a + n as u32;
		let mut all: Vec<(&str, Range<u32>)> = vec![
			("labels", r(ID0, ROWS)),
			("values", r(ID_VALUE, ROWS)),
			("press", r(ID_PRESS, ROWS)),
			("mouse values", super::super::pcmouseinv::ID_V0..super::super::pcmouseinv::ID_V_END),
			("glyphs", *glyphs::IDS.start()..*glyphs::IDS.end() + 1),
			("debug mode", one(ID_DBG0)),
			("debug keys", r(ID_DBG_KEY, NDKEYS)),
			("debug vals", r(ID_DBG_VAL, NDKEYS)),
			("camera speed rows (pcext/camopts.rs, lockstr.rs)", 0x7f80..0x7f89),
			("display (pcdisp)", pcdisp::ID0..pcdisp::ID_END),
			// pcdisp also serves the six pcmore row ids from its pool (0x7f98..=0x7f9d), outside ID0..ID_END.
			("display rows (pcmore, pcfgen)", 0x7f98..0x7fa0),
			("mods (pcmods)", super::super::pcmods::ID_FIRST..super::super::pcmods::ID_END),
		];
		for (n, id) in [("controls", ID_CONTROLS), ("header", ID_HEADER), ("desc", ID_DESC), ("page", ID_PAGE), ("selc", ID_SELC), ("selg", ID_SELG), ("hud", ID_HUD), ("sharp", ID_SHARP), ("original", ID_ORIGINAL), ("renderer", ID_RENDERER), ("vulkan", ID_VULKAN), ("dx12", ID_DX12), ("intro", ID_INTRO), ("skipcut", ID_SKIPCUT), ("preset", ID_PRESET), ("custom", ID_CUSTOM), ("metal", ID_METAL)] {
			all.push((n, one(id)));
		}
		assert!((0x7f98..0x7fa0).all(pcdisp::serves) && !pcdisp::serves(0x7f97) && !pcdisp::serves(0x7fa0), "the rows range is exactly the eight ids pcdisp serves from pcmore (and pcfgen)");
		for (i, (na, a)) in all.iter().enumerate() {
			for (nb, b) in &all[i + 1..] {
				assert!(a.end <= b.start || b.end <= a.start, "text ids of {na} {a:x?} and {nb} {b:x?} overlap");
			}
		}
		// The glyph ids answer with the glyph words (the arm order in `text` once let the mouse values shadow them).
		for l in 0..5 {
			for id in glyphs::IDS {
				assert_eq!(text(id, l, 0, 0, false), glyphs::text(id, l), "{id:#x}");
			}
		}
	}

	/// Every function appears on exactly one row of the sections that show its device, no section has more than six rows.
	#[test]
	fn sections_cover_the_functions() {
		let _g = crate::retail::pcopts::globals_guard(); // binds and the capture state are process-wide
		assert!((0..NSEC).all(|s| (1..=ROWS - 1).contains(&rows(s).len())));
		// Every action is on exactly one keyboard row, in the section the table says.
		let mut acts: Vec<usize> = (0..NSEC).flat_map(rows).filter_map(|t| if let Target::Action(a) = t { Some(a) } else { None }).collect();
		acts.sort();
		assert_eq!(acts, (0..kbm::table().actions.len()).collect::<Vec<_>>());
		let mut pad: Vec<usize> = (0..NSEC).flat_map(rows).filter_map(|t| if let Target::PadAction(a) = t { Some(a) } else { None }).collect();
		pad.sort();
		assert_eq!(pad, (0..kbm::table().actions.len()).filter(|&i| kbm::table().actions[i].pad_ok).collect::<Vec<_>>());
		assert_eq!((rows(9), target(9, 3)), (vec![Target::ResetKeys, Target::ResetPad], Target::Blank));
		// Device per section: 0..=5 keyboard + mouse, 6..=8 controller.
		assert!((0..6).all(|s| rows(s).iter().all(|t| matches!(t, Target::Action(_) | Target::MouseInv(_)))));
		assert_eq!(rows(5)[4..], [Target::MouseInv(0), Target::MouseInv(1)], "the PC section ends with the two mouse invert rows");
		assert!((6..9).all(|s| rows(s).iter().all(|t| matches!(t, Target::PadAction(_)))));
	}

	/// Every Graphics row (the 13 of pcext's `ROWS`, the HUD, Renderer and Preset rows) is on exactly one page of at most six rows.
	#[test]
	fn graphics_sections_cover_the_rows() {
		let _g = crate::retail::pcopts::globals_guard(); // binds and the capture state are process-wide
		let mut all: Vec<usize> = GSECS.iter().flat_map(|s| s.iter().copied()).collect();
		all.sort();
		let want: Vec<usize> = (0..=FG_ROW).filter(|&r| !cfg!(target_os = "linux") || r != RENDER_ROW).collect();
		assert_eq!(all, want);
		assert!(GSECS.iter().all(|s| (1..=ROWS).contains(&s.len())));
		assert!(GSECS[SEC_SCREEN].contains(&9), "SEC_SCREEN is the section of the Culling row (pcext GROWS index 9): cullwarn.rs shows its warning there");
		let rows = fake_rows();
		for sec in 0..NGSEC {
			let t = gfx_table(&rows, sec, false);
			assert_eq!(&t[..6], &[ID_PAGE, G_SEL, ID_SELG, ID_SELG, 0, 0], "the stepper row: two equal choices");
			assert_eq!(t.len(), 6 * (1 + GSECS[sec].len()));
		}
		let t = gfx_table(&rows, 0, false);
		assert_eq!(&t[6..12], &[ID_PRESET, PRESETB, ID_LOW, ID_MEDIUM, ID_HIGH, 0], "Preset opens the Picture page with three choices");
		assert_eq!(&gfx_table(&rows, 0, true)[6..12], &[ID_PRESET, PRESETB, ID_LOW, ID_MEDIUM, ID_HIGH, ID_CUSTOM], "Custom is the 4th choice while the state is Custom");
		assert_eq!(&t[12..18], &rows[0]);
		assert_eq!(&t[t.len() - 6..], &rows[6], "Sun rays close the Picture page");
		assert_eq!(&gfx_table(&rows, 1, false)[6..12], &rows[4], "Effects starts with Water");
		let s = gfx_table(&rows, SEC_SCREEN, false);
		assert_eq!(&s[s.len() - 6..], &[ID_HUD, HUDB, ID_SHARP, ID_ORIGINAL, 0, 0], "HUD closes the Screen page");
		#[cfg(not(target_os = "linux"))]
		assert_eq!(&s[s.len() - 18..s.len() - 12], &[ID_RENDERER, RENDB, ID_VULKAN, ID_DX12, 0, 0]);
		assert_eq!(&s[s.len() - 12..s.len() - 6], &[super::super::pcfgen::ID_FG, super::super::pcfgen::FG_B, super::super::pcfgen::ID_VFG, super::super::pcfgen::ID_VFG, 0, 0], "Frame Generation before HUD");
		assert_eq!(GSECS[2], [MREFL_ROW, 15, 16, 17, 18, SUPP_ROW], "More effects holds Metal reflections, the two Fur rows, Fire, Smoke, then Supporter drawings");
		let more = gfx_table(&rows, 2, false);
		assert_eq!(more.len(), 42, "the stepper and six rows");
		assert_eq!(&more[6..12], &[crate::retail::pcmrefl::ID_MREFL, crate::retail::pcmrefl::REFL_B, crate::retail::pcmrefl::ID_VMREFL, crate::retail::pcmrefl::ID_VMREFL, 0, 0], "Metal reflections: a stepper, two equal choices");
		assert_eq!([12, 18, 24, 30].map(|at| more[at..at + 6].to_vec()), [15, 16, 17, 18].map(|i| rows[i].to_vec()), "Fur, Fur style, Fire, Smoke");
		let disp = gfx_table(&rows, SEC_DISPLAY, false);
		assert_eq!(&disp[6..12], &[pcdisp::ID_HUDPOS, pcdisp::POS_B, pcdisp::ID_P43, pcdisp::ID_CORNERS, 0, 0], "Display starts with HUD position, 4:3 / Corners");
		for (i, id) in [pcdisp::ID_VSCALE, pcdisp::ID_VFPS, pcdisp::ID_VRS].into_iter().enumerate() {
			assert_eq!(&disp[12 + 6 * i + 2..12 + 6 * i + 4], &[id, id], "the other three rows are steppers: two equal choices");
		}
		assert_eq!(&disp[30..36], &rows[7], "Frame smoothing follows display scaling");
		assert_eq!(disp.len(), 8 * 6, "stepper, five rows, Aspect ratio, Upscaler (pcmore.rs)");
		let sys = gfx_table(&rows, 5, false);
		assert_eq!(&sys[6..12], &[ID_INTRO, INTROB, ID_OFF, ID_ON, 0, 0], "System starts with the Skip intro row, Off / On");
		assert_eq!(&sys[12..18], &[ID_SKIPCUT, SKIPB, ID_ON, ID_OFF, 0, 0], "then Start skips cutscenes, On / Off (the byte is inverted)");
		assert_eq!(&sys[18..24], &[pcinput::ID_WHEEL, pcinput::WHEEL_B, ID_OFF, ID_ON, 0, 0], "then Quick Select mouse, Off / On");
		assert_eq!(&sys[24..30], &[pcinput::ID_STICK, pcinput::STICK_B, ID_HIGH, ID_MEDIUM, ID_LOW, 0], "then Stick dead zone, High / Medium / Low");
		assert_eq!(&sys[30..36], &[glyphs::ID_GLYPHS, glyphs::GLYPH_B, glyphs::ID_PS, glyphs::ID_LETTERS, glyphs::ID_SWITCH, 0], "then Button glyphs, PlayStation / Letters / Letters (Switch)");
		assert_eq!(&sys[36..], &[ID_MSTYLE, MSTYLEB, ID_MSTYLE_MODERN, ID_LIGHT_CLASSIC, 0, 0], "closing with Menu style, Modern / Classic");
		let play = gfx_table(&rows, NGSEC - 1, false);
		assert_eq!(&play[6..12], &[crate::retail::pcmore::ID_QSP, crate::retail::pcmore::QSP_B, ID_OFF, ID_ON, 0, 0], "Gameplay: Quick Select pause, Off / On");
		assert_eq!(&play[12..], &[crate::retail::pcmore::ID_FT, crate::retail::pcmore::FT_B, ID_OFF, ID_ON, 0, 0], "then Fast travel, Off / On");
		assert_eq!(GSECTIONS[NGSEC - 1][0], b"Gameplay", "the modern Game tab's group name");
	}

	/// The Graphics table is rewritten for the section; a shorter section leaves no stale record behind its zero record.
	#[test]
	fn the_graphics_table_follows_the_section_and_the_hud_byte_survives() {
		let _g = crate::retail::pcopts::globals_guard();
		let mut mem = Mem::new();
		let rows = fake_rows();
		let gtab = BASE + 0xaa0;
		assert_eq!(hud_sharp(&mut mem), None, "not built yet");
		build_gfx(&mut mem, gtab, &rows, true, false);
		assert_eq!((hud_sharp(&mut mem), mem.rd8(G_SEL)), (Some(true), 0));
		let rec = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..6).map(|k| mem.rd32(gtab + 24 * i + 4 * k)).collect() };
		assert_eq!(rec(&mut mem, 0), [ID_PAGE, G_SEL, ID_SELG, ID_SELG, 0, 0]);
		assert_eq!(rec(&mut mem, 1), [ID_PRESET, PRESETB, ID_LOW, ID_MEDIUM, ID_HIGH, 0]);
		assert_eq!(rec(&mut mem, 6), rows[6]);
		assert_eq!(rec(&mut mem, 7), [0; 6]);
		test_gfx_section(&mut mem, 1, &rows, gtab);
		assert_eq!(rec(&mut mem, 1), rows[4]);
		assert_eq!(rec(&mut mem, 4), rows[8]);
		assert_eq!((rec(&mut mem, 5), rec(&mut mem, 6), rec(&mut mem, 7), rec(&mut mem, 8)), (vec![ID_METAL, METAL_B, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW], rows[13].to_vec(), rows[14].to_vec(), vec![0; 6]), "Volumetric clouds close the Effects page, no record of the Picture page is left");
		// The player picks Original; a rebuild (level load) keeps it, a fresh RAM takes the file's value.
		mem.wr8(HUDB, 1);
		assert_eq!(hud_sharp(&mut mem), Some(false));
		build_gfx(&mut mem, gtab, &rows, true, false);
		assert_eq!(hud_sharp(&mut mem), Some(false));
		assert_eq!(rec(&mut mem, 1), rows[4], "the table is the section's the player left");
		let mut fresh = Mem::new();
		build_gfx(&mut fresh, gtab, &rows, false, true);
		assert_eq!(hud_sharp(&mut fresh), Some(false), "hud=original in the file");
		assert_eq!((renderer_dx12(&mut mem), renderer_dx12(&mut fresh)), (Some(false), Some(true)), "renderer=dx12 in the file; a rebuild keeps the player's byte");
		assert_eq!(renderer_dx12(&mut Mem::new()), None, "not built yet");
		mem.wr8(RENDB, 1);
		build_gfx(&mut mem, gtab, &rows, true, false);
		assert_eq!(renderer_dx12(&mut mem), Some(true));
		test_gfx_section(&mut mem, 0, &rows, gtab);
	}

	/// The stepper: Cross (the byte toggles) steps on and wraps, Left / Right step back / on and wrap, only with the cursor on the
	/// row, and the first look is only a baseline.
	#[test]
	fn the_pager_steps_and_wraps() {
		let _g = crate::retail::pcopts::globals_guard(); // binds and the capture state are process-wide
		let mut p = Pager { sec: 0, seen: 0xff };
		let never = || -> bool { panic!("asked without an input") };
		assert_eq!(pager_step(&mut p, 0, [false; 2], 3, never), None);
		assert_eq!(p.seen, 0);
		assert_eq!(pager_step(&mut p, 1, [false; 2], 3, || true), Some(1), "Cross: the byte went 0 -> 1");
		p.sec = 1;
		assert_eq!(pager_step(&mut p, 0, [false; 2], 3, || true), Some(2), "and 1 -> 0 is a step too");
		p.sec = 2;
		assert_eq!(pager_step(&mut p, 1, [false; 2], 3, || true), Some(0), "Cross wraps from the last section");
		p.sec = 0;
		assert_eq!(pager_step(&mut p, 1, [true, false], 3, || true), Some(2), "Left from the first wraps to the last");
		p.sec = 2;
		assert_eq!(pager_step(&mut p, 1, [false, true], 3, || true), Some(0), "Right from the last wraps to the first");
		assert_eq!(pager_step(&mut p, 1, [true, true], 3, never), None, "both directions: nothing");
		p.sec = 1;
		assert_eq!(pager_step(&mut p, 0, [false; 2], 3, || false), None, "cursor on another row: the byte change is no step");
		assert_eq!(pager_step(&mut p, 0, [false, true], 3, || false), None, "nor is Right");
		assert_eq!(p.seen, 0, "the baseline still follows");
		p.sec = 0;
		assert_eq!(pager_step(&mut p, 0, [false, true], 1, || true), None, "a single section stays");
	}

	#[test]
	fn the_menus_auto_repeat() {
		let due: Vec<u32> = (1..=30).filter(|&h| repeat_due(h)).collect();
		assert_eq!(due, [1, 13, 17, 21, 25, 29]);
	}

	fn page_mem() -> Mem {
		let mut mem = Mem::new();
		mem.wr32(MAGIC_AT, MAGIC);
		mem
	}

	/// The retail item flips a row byte; `sync` starts a capture, a key ends it, the row byte goes back to 0, the text shows the key.
	#[test]
	fn a_pressed_row_captures_and_binds() {
		let _g = crate::retail::pcopts::globals_guard(); // binds and the capture state are process-wide
		binds::init(None, None);
		crate::kbm_actions::install();
		let mut mem = page_mem();
		let jump = kbm::table().index("jump").unwrap();
		mem.wr8(CBASE + 1, 1); // Jump row of section 0
		sync(&mut mem);
		assert!(binds::capturing());
		assert_eq!(mem.rd8(CBASE + 1), 1, "the row reads the capture prompt while it waits");
		kbm::capture_poll(&[kbm::Input::Key(K::KeyK)], 0);
		assert!(!binds::capturing());
		sync(&mut mem);
		assert_eq!(mem.rd8(CBASE + 1), 0);
		assert_eq!(text(ID_VALUE + 1, 0, 0, 0, false), b"K");
		assert_eq!(kbm::binds().0[jump], [Some(kbm::Input::Key(K::KeyK)), None]);
		assert_eq!(kbm::take_changes().map(|c| c.0), Some(Some("jump:KeyK|-".to_string())));
		// Esc ends a capture without a binding.
		mem.wr8(CBASE + 2, 1);
		sync(&mut mem);
		kbm::capture_poll(&[kbm::Input::Key(K::Escape)], 0);
		sync(&mut mem);
		assert_eq!((mem.rd8(CBASE + 2), kbm::take_changes()), (0, None));
		// A controller row waits for a button (section 6: Controller 1/3; row 1 = Jump, row 3 = Fire).
		go(&mut mem, 6);
		mem.wr8(CBASE + 1, 1);
		sync(&mut mem);
		assert_eq!(kbm::capture_state(), kbm::Cap::Pad(jump, 0));
		kbm::capture_poll(&[kbm::Input::Key(K::KeyZ)], 0);
		kbm::capture_poll(&[kbm::Input::Key(K::KeyZ)], 0);
		assert!(binds::capturing(), "a key does not bind a controller action");
		kbm::capture_poll(&[], 0x2000); // PHYS 1 = B
		sync(&mut mem);
		assert_eq!(text(ID_VALUE + 1, 0, 6, 0, false), b"B x2", "Jump now shares B with Fire");
		assert_eq!(text(ID_VALUE + 1, 0, 6, 0, true), b"Circle x2");
		assert_eq!(text(ID_VALUE + 3, 0, 6, 0, false), b"B x2", "Fire keeps B (shared, no swap)");
		assert_eq!(text(ID_VALUE + 2, 0, 6, 0, false), b"X", "the others are untouched");
		assert_eq!(text(ID_DESC, 0, 6, 0, false), pd::HINT[0], "controller hint on a controller section");
		assert_eq!(text(ID_DESC, 0, 0, 0, false), kb::HINT[0]);
		// Slot 2 by Square / Tab, then Delete clears slot 2.
		kb::want_slot2();
		mem.wr8(CBASE + 1, 1);
		sync(&mut mem);
		assert_eq!(kbm::capture_state(), kbm::Cap::Pad(jump, 1));
		kbm::capture_poll(&[], 0);
		kbm::capture_poll(&[], 0x1000); // some button
		sync(&mut mem);
		assert!(text(ID_VALUE + 1, 0, 6, 0, false).starts_with(b"B / "));
		assert!(pd::clear(jump) && text(ID_VALUE + 1, 0, 6, 0, false) == b"B x2");
		binds::init(None, None);
		go(&mut mem, 0);
	}

	/// A section change cancels a capture and rewrites the table; a press on a blank row is ignored; the reset rows restore the
	/// defaults and read Done.
	#[test]
	fn sections_reset_and_blank_rows() {
		let _g = crate::retail::pcopts::globals_guard(); // binds and the capture state are process-wide
		binds::init(Some("cross:KeyK"), Some("cross:b"));
		let mut mem = page_mem();
		mem.wr8(CBASE + 3, 1);
		sync(&mut mem);
		assert!(binds::capturing());
		go(&mut mem, 1);
		assert!(!binds::capturing());
		assert_eq!(mem.rd8(CBASE + 3), 0);
		let rec = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..6).map(|k| mem.rd32(CTAB + 24 * i + 4 * k)).collect() };
		assert_eq!(rec(&mut mem, 0), [ID_PAGE, CBASE, ID_SELC, ID_SELC, 0, 0]);
		assert_eq!(rec(&mut mem, 6), [ID0 + 6, CBASE + 6, ID_VALUE + 6, ID_PRESS + 6, 0, 0]);
		assert_eq!(rec(&mut mem, 7), [0; 6]);
		go(&mut mem, 9);
		assert_eq!((rec(&mut mem, 2)[0], rec(&mut mem, 3)), (ID0 + 2, vec![0; 6]), "two rows, then the zero record");
		assert_eq!(rec(&mut mem, 4), [0; 6]);
		mem.wr8(CBASE + 5, 1);
		sync(&mut mem);
		assert_eq!((mem.rd8(CBASE + 5), binds::capturing()), (0, false), "blank row");
		mem.wr8(CBASE + 1, 1);
		sync(&mut mem);
		assert_eq!(binds::kb(), binds::KbBinds::default());
		assert_eq!(binds::pad_map()[binds::CROSS], 1, "the controller is not touched by the keyboard reset");
		assert_eq!(mem.rd8(CBASE + 1), 1, "Done is shown");
		for _ in 0..DONE_FIELDS {
			sync(&mut mem);
		}
		assert_eq!(mem.rd8(CBASE + 1), 0);
		mem.wr8(CBASE + 2, 1);
		sync(&mut mem);
		assert_eq!(binds::pad_map()[binds::CROSS], binds::DEFAULT_PAD[binds::CROSS]);
		binds::init(None, None);
		go(&mut mem, 0);
		assert_eq!(csec(), 0);
	}

	/// The Preset row: the first look is the baseline, a change of the byte below 3 is a choice, `preset_show` writes the detected
	/// preset and gives Custom its 4th choice only while the state is Custom.
	#[test]
	fn the_preset_row_reports_choices_and_shows_custom_only_when_custom() {
		let _g = crate::retail::pcopts::globals_guard();
		let mut mem = Mem::new();
		let rows = fake_rows();
		let gtab = BASE + 0xaa0;
		assert_eq!(preset_chosen(&mut mem), None, "not built yet");
		build_gfx(&mut mem, gtab, &rows, true, false);
		assert_eq!(preset_chosen(&mut mem), None, "the first look is the baseline");
		mem.wr8(PRESETB, 1);
		assert_eq!(preset_chosen(&mut mem), Some(1));
		assert_eq!(preset_chosen(&mut mem), None, "reported once");
		let fourth = |mem: &mut Mem| mem.rd32(gtab + 24 + 20);
		preset_show(&mut mem, &rows, gtab, 3);
		assert_eq!((mem.rd8(PRESETB), fourth(&mut mem)), (3, ID_CUSTOM));
		assert_eq!(preset_chosen(&mut mem), None, "what the host shows is not a choice");
		mem.wr8(PRESETB, 0); // the retail item steps Custom -> the first choice
		assert_eq!(preset_chosen(&mut mem), Some(0));
		preset_show(&mut mem, &rows, gtab, 0);
		assert_eq!((mem.rd8(PRESETB), fourth(&mut mem)), (0, 0), "no Custom choice while a preset is shown");
		set_hud(&mut mem, false);
		assert_eq!(hud_sharp(&mut mem), Some(false));
	}

	/// Before `build_page` ran on this RAM nothing is read or written (the title screen, other overlays).
	#[test]
	fn sync_is_inert_without_the_page() {
		let _g = crate::retail::pcopts::globals_guard(); // binds and the capture state are process-wide
		let mut mem = Mem::new();
		mem.wr8(CBASE + 1, 1);
		sync(&mut mem);
		assert!(!binds::capturing());
		assert_eq!(mem.rd8(CBASE + 1), 1);
		field(&mut mem, 4, 0x2000, &fake_rows(), BASE + 0xaa0);
		assert_eq!(csec(), 0);
	}
}
