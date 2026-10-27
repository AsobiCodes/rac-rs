//! The Controls page (Pause -> Options -> Controls): the menu that edits the bindings of `binds.rs`, and the section stepper that
//! the Graphics page (`pcext::patch_graphics`) shares.
//!
//! Both pages are built like the retail value pages: the Options list gains a row, and the row opens a copy of the retail Camera
//! Options page with its own header and multi-choice item, so it is drawn, steered and sounds like the retail value pages. The row
//! table (24-byte records `{label id, byte*, choice 0..3}`, ended by a zero record) is data, and the retail item reads it afresh on
//! every draw and every update (FUN_0027ea40 counts the records, FUN_0027e898 stops the cursor at the last one), so a page of any
//! length up to the panel's 7 rows is a table the host rewrites.
//!
//! One page holds at most 7 rows (a taller page would pack the rows closer than the font is high), so a page with more settings has
//! sections: row 0 is "Page", a stepper whose value is the section's name. The record `{Page, byte, Name, Name}` has two equal
//! choice ids, so the retail item's Cross toggles the row byte 0 <-> 1 and the host reads each change as "next section" (the byte only
//! signals); Left / Right (`fin.buttons`, also the mouse wheel) step back / forward with the menu's own auto-repeat while the page's
//! cursor is on row 0 (`mouse_menu::read`); the steps wrap. The same trick as the Camera speed row of `pcext.rs`. The section is the
//! host's (`St`), the name is the dynamic text `ID_SELC` / `ID_SELG`, and a section change rewrites the table.
//!
//! Controls (9 sections of at most 6 functions): Keyboard buttons 1/2, 2/2, movement, camera, D-pad, Controller buttons 1/2, 2/2,
//! D-pad, Reset. A function row is a two-choice item: choice 0 shows the bound key or button, choice 1 "Press a key" / "Press a
//! button" (every text is a dynamic id 0x7f40..0x7f7f that the text lookup hand answers, `pcext::text_hook` -> `text_hook`, from the
//! CURRENT bindings, so the page needs no redraw hook). Confirm flips the row byte to 1; `sync` (worker, every field) sees it and
//! starts a capture (`binds::capture_begin`): the host (`Retail::tick_keys`) gives the game a neutral pad and binds the next fresh
//! key or button (Esc or 8 s cancel), then `sync` sees the capture end and puts the byte back to 0, so the row shows the new binding.
//! A taken key swaps (`binds::KbBinds::assign`). The Reset section's rows restore the defaults and read "Done" for a moment. A
//! section change cancels a capture.
//!
//! Debug (the Controls page's 10th and last section, `SEC_DEBUG`): row 1 "Debug mode" Off / On (byte `DBGB`; `pcext::sync` hands it to
//! `devtools::set_menu` and the options file, `debug=on`), rows 2..6 list the keys it switches on (F1, F2, F5, F6, F8); those five rows
//! only show text (their byte is reset like a blank row's). Their texts are the ids 0x7fa0..0x7faf in a pool of their own (`DBG_POOL`).
//!
//! Graphics (3 sections: Picture, Effects, Screen): the rows are `pcext::ROWS` (bytes and values unchanged) plus the HUD row
//! (`HUDB`: 0 Sharp, 1 Original) and the Button glyphs row (0 PlayStation, 1 Xbox, 2 Switch; its byte is `pcext::GLYPHS`, with the
//! other Graphics rows' bytes, so the title screen's Graphics page shares it) that `pcext::sync` hands to the options file and the
//! renderer (`pcext::hud_sharp_now`, `pcext::glyph_style_now`).
//!
//! Reserved RAM (`CBASE`, inside pcext's BASE window, above prompts.rs's pool): the row bytes, the Controls page copy, its header
//! and value items, its row table, the Options pictures and the text pool. All of it is written only after `build_page` /
//! `build_gfx` set their magic word.
use super::binds::{self, Cap};
use super::mouse_menu;
use super::pcext::{BASE, GPAGE, GVAL};
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
/// Rows of a page: the section selector (row 0) and at most six settings. Controls: row bytes CBASE + r.
const ROWS: usize = 7;
/// Records of a row table: the rows and the zero record.
const TABLE_RECS: usize = ROWS + 1;
/// The Graphics page's selector byte and its HUD row's byte (0 Sharp, 1 Original).
const G_SEL: u32 = CBASE + 0x10;
const HUDB: u32 = CBASE + 0x11;
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
/// The Options list's picture records (9 x 8 bytes), moved here from pcext's block (it would overlap the Graphics page).
pub const PICS: u32 = CBASE + 0x500;
/// Text pool: one 0x40-byte slot per id.
const POOL: u32 = CBASE + 0x600;
const SLOT: u32 = 0x40;

/// Text ids: the Controls rows' labels 0x7f40 + r, their values 0x7f50 + r, their "press" texts 0x7f60 + r, then the fixed ones.
const ID0: u32 = 0x7f40;
pub const ID_LAST: u32 = 0x7f80;
const ID_VALUE: u32 = 0x7f50;
const ID_PRESS: u32 = 0x7f60;
pub const ID_CONTROLS: u32 = 0x7f70;
const ID_HEADER: u32 = 0x7f71;
pub const ID_DESC: u32 = 0x7f72;
/// The stepper's label ("Page") and the dynamic values "< section name >" of the Controls and the Graphics page.
const ID_PAGE: u32 = 0x7f73;
const ID_SELC: u32 = 0x7f74;
const ID_SELG: u32 = 0x7f75;
/// The HUD row: its label and its two values.
const ID_HUD: u32 = 0x7f76;
const ID_SHARP: u32 = 0x7f77;
const ID_ORIGINAL: u32 = 0x7f78;
/// The Button glyphs row: its label and its three values. The title screen's Graphics page uses these too, so the mirror's text
/// map has them (the pcext's own pool has no room left).
pub const ID_GLYPHS: u32 = 0x7f79;
pub const ID_PLAYSTATION: u32 = 0x7f7a;
pub const ID_XBOX: u32 = 0x7f7b;
pub const ID_SWITCH: u32 = 0x7f7c;
/// The Debug section: ids 0x7fa0 (the mode row's label), 0x7fa1..0x7fa5 (the key rows' labels), 0x7fa6..0x7faa (their values F1 .. F8);
/// 0x7fab..0x7faf are free. Off / On are retail's own 0x4f5b / 0x4f5a.
pub const ID_DBG0: u32 = 0x7fa0;
pub const ID_DBG_END: u32 = 0x7fb0;
const ID_DBG_KEY: u32 = 0x7fa1;
const ID_DBG_VAL: u32 = 0x7fa6;
const ID_ON: u32 = 0x4f5a;
const ID_OFF: u32 = 0x4f5b;

/// Sections of the Controls page and of the Graphics page.
const NSEC: usize = 10;
/// The Debug section (the last one) and the keys it lists.
const SEC_DEBUG: usize = 9;
const NDKEYS: usize = 5;
const NGSEC: usize = 3;
/// The HUD row in `GSECS` (the 13 rows of `pcext::ROWS` are 0..=12).
const HUD_ROW: usize = 13;
/// The Button glyphs row in `GSECS` (after the HUD row).
const GLYPH_ROW: usize = 14;
/// The Graphics sections as indices of `pcext::ROWS` (0 Lighting, 1 Ambient occlusion, 2 Dynamic shadows, 3 Display mode, 4 Water,
/// 5 Particles, 6 Sun rays, 7 Frame smoothing, 8 Vegetation, 9 Culling, 10 Global illumination, 11 Waves, 12 Draw distance).
/// Picture: Lighting, Ambient occlusion, Dynamic shadows, Global illumination, Sun rays, HUD. Effects: Water, Waves, Particles,
/// Vegetation. Screen: Display mode, Frame smoothing, Draw distance, Culling, Button glyphs.
const GSECS: [&[usize]; NGSEC] = [&[0, 1, 2, 10, 6, HUD_ROW], &[4, 11, 5, 8], &[3, 7, 12, 9, GLYPH_ROW]];

/// What a row does in a Controls section.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
	/// Binds function `Cap::Key(f)` / `Cap::Pad(f)`.
	Func(Cap),
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
	let keys = |r: std::ops::Range<usize>| r.map(|f| Target::Func(Cap::Key(f))).collect::<Vec<_>>();
	let pads = |r: std::ops::Range<usize>| r.map(|f| Target::Func(Cap::Pad(f))).collect::<Vec<_>>();
	match sec {
		0 => keys(0..6),
		1 => keys(6..12),
		2 => keys(binds::LSTICK..binds::LSTICK + 4),
		3 => keys(binds::RSTICK..binds::RSTICK + 4),
		4 => keys(binds::DPAD..binds::DPAD + 4),
		5 => pads(0..6),
		6 => pads(6..12),
		7 => pads(binds::DPAD..binds::DPAD + 4),
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

/// Function names (index = `binds` function), En Fr De Es It, in the game's font bytes (a-acute 0x84, e-acute 0x8c, u-umlaut 0xa7,
/// a-umlaut 0x87, u-grave 0xa5, i-acute 0x94 [I]). The pad's own names, as the printed manual has them.
#[rustfmt::skip]
static LABELS: [T5; binds::NK] = [
	[b"Cross", b"Croix", b"Kreuz", b"Cruz", b"Croce"],
	[b"Square", b"Carr\x8c", b"Quadrat", b"Cuadrado", b"Quadrato"],
	[b"Triangle", b"Triangle", b"Dreieck", b"Tri\x84ngulo", b"Triangolo"],
	[b"Circle", b"Rond", b"Kreis", b"C\x94rculo", b"Cerchio"],
	[b"L1", b"L1", b"L1", b"L1", b"L1"],
	[b"R1", b"R1", b"R1", b"R1", b"R1"],
	[b"L2", b"L2", b"L2", b"L2", b"L2"],
	[b"R2", b"R2", b"R2", b"R2", b"R2"],
	[b"L3", b"L3", b"L3", b"L3", b"L3"],
	[b"R3", b"R3", b"R3", b"R3", b"R3"],
	[b"Start", b"Start", b"Start", b"Start", b"Start"],
	[b"Select", b"Select", b"Select", b"Select", b"Select"],
	[b"D-pad up", b"Croix haut", b"Steuerkreuz hoch", b"Cruceta arriba", b"Croce su"],
	[b"D-pad down", b"Croix bas", b"Steuerkreuz runter", b"Cruceta abajo", b"Croce gi\xa5"],
	[b"D-pad left", b"Croix gauche", b"Steuerkreuz links", b"Cruceta izq.", b"Croce sinistra"],
	[b"D-pad right", b"Croix droite", b"Steuerkreuz rechts", b"Cruceta der.", b"Croce destra"],
	[b"Move forward", b"Avancer", b"Vorw\x87rts", b"Avanzar", b"Avanti"],
	[b"Move back", b"Reculer", b"R\xa7ckw\x87rts", b"Retroceder", b"Indietro"],
	[b"Move left", b"Aller \x85 gauche", b"Nach links", b"Ir a la izq.", b"Vai a sinistra"],
	[b"Move right", b"Aller \x85 droite", b"Nach rechts", b"Ir a la der.", b"Vai a destra"],
	[b"Camera up", b"Cam\x8cra haut", b"Kamera hoch", b"C\x84mara arriba", b"Camera su"],
	[b"Camera down", b"Cam\x8cra bas", b"Kamera runter", b"C\x84mara abajo", b"Camera gi\xa5"],
	[b"Camera left", b"Cam\x8cra gauche", b"Kamera links", b"C\x84mara izq.", b"Camera sinistra"],
	[b"Camera right", b"Cam\x8cra droite", b"Kamera rechts", b"C\x84mara der.", b"Camera destra"],
];

/// "Press a key" (keyboard rows), "Press a button" (controller rows), the reset rows' "Done".
static PRESS_KEY: T5 = [b"Press a key", b"Appuyez sur une touche", b"Taste dr\xa7cken", b"Pulsa una tecla", b"Premi un tasto"];
static PRESS_PAD: T5 = [b"Press a button", b"Appuyez sur un bouton", b"Knopf dr\xa7cken", b"Pulsa un bot\x9cn", b"Premi un pulsante"];
static DONE: T5 = [b"Done", b"Fait", b"Fertig", b"Hecho", b"Fatto"];
static RESET_KEYS: T5 = [b"Reset keyboard", b"R\x8cinit. clavier", b"Tastatur zur\xa7ck", b"Restaurar teclado", b"Ripristina tastiera"];
static RESET_PAD: T5 = [b"Reset controller", b"R\x8cinit. manette", b"Controller zur\xa7ck", b"Restaurar mando", b"Ripristina joypad"];
static CONTROLS: T5 = [b"Controls", b"Commandes", b"Steuerung", b"Controles", b"Controlli"];
static HEADER: T5 = [b"Controls Options", b"Options commandes", b"Steuerungs-Optionen", b"Opciones de controles", b"Opzioni controlli"];
static DESC: T5 = [b"Remap keys and controller buttons", b"Changer touches et boutons", b"Tasten und Belegung \x87ndern", b"Cambia teclas y botones", b"Cambia tasti e pulsanti"];
static PAGE: T5 = [b"Page", b"Page", b"Seite", b"P\x84gina", b"Pagina"];
static HUD: T5 = [b"HUD", b"HUD", b"HUD", b"HUD", b"HUD"];
static SHARP: T5 = [b"Sharp", b"Net", b"Scharf", b"N\x94tido", b"Nitido"];
static ORIGINAL: T5 = [b"Original", b"Original", b"Original", b"Original", b"Originale"];
/// The Button glyphs row: its label and its three styles.
static GLYPHS: T5 = [b"Button glyphs", b"Touches de manette", b"Tastensymbole", b"Botones del mando", b"Tasti del gamepad"];
static PLAYSTATION: T5 = [b"PlayStation", b"PlayStation", b"PlayStation", b"PlayStation", b"PlayStation"];
static XBOX: T5 = [b"Xbox", b"Xbox", b"Xbox", b"Xbox", b"Xbox"];
static SWITCH: T5 = [b"Switch", b"Switch", b"Switch", b"Switch", b"Switch"];
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
static SECTIONS: [T5; NSEC] = [
	[b"Keyboard buttons 1/2", b"Clavier: boutons 1/2", b"Tastatur: Tasten 1/2", b"Teclado: botones 1/2", b"Tastiera: pulsanti 1/2"],
	[b"Keyboard buttons 2/2", b"Clavier: boutons 2/2", b"Tastatur: Tasten 2/2", b"Teclado: botones 2/2", b"Tastiera: pulsanti 2/2"],
	[b"Keyboard movement", b"Clavier: mouvement", b"Tastatur: Bewegung", b"Teclado: movimiento", b"Tastiera: movimento"],
	[b"Keyboard camera", b"Clavier: cam\x8cra", b"Tastatur: Kamera", b"Teclado: c\x84mara", b"Tastiera: camera"],
	[b"Keyboard D-pad", b"Clavier: croix", b"Tastatur: Kreuz", b"Teclado: cruceta", b"Tastiera: croce"],
	[b"Controller buttons 1/2", b"Manette: boutons 1/2", b"Controller: Tasten 1/2", b"Mando: botones 1/2", b"Joypad: pulsanti 1/2"],
	[b"Controller buttons 2/2", b"Manette: boutons 2/2", b"Controller: Tasten 2/2", b"Mando: botones 2/2", b"Joypad: pulsanti 2/2"],
	[b"Controller D-pad", b"Manette: croix", b"Controller: Kreuz", b"Mando: cruceta", b"Joypad: croce"],
	[b"Reset", b"R\x8cinitialiser", b"Zur\xa7cksetzen", b"Restablecer", b"Ripristina"],
	[b"Debug", b"Debug", b"Debug", b"Debug", b"Debug"],
];
/// The Graphics sections' names.
static GSECTIONS: [T5; NGSEC] = [
	[b"Picture", b"Image", b"Bild", b"Imagen", b"Immagine"],
	[b"Effects", b"Effets", b"Effekte", b"Efectos visuales", b"Effetti"],
	[b"Screen", b"Affichage", b"Bildschirm", b"Pantalla", b"Schermo"],
];

/// "< name >": the stepper's value (`<` and `>` are glyphs of all three fonts).
fn stepper(name: &[u8]) -> Vec<u8> {
	let mut v = b"< ".to_vec();
	v.extend_from_slice(name);
	v.extend_from_slice(b" >");
	v
}

/// Every text of the pages in `lang`: `csec` / `gsec` are the sections the Controls / Graphics page shows, `playstation` picks the
/// PlayStation button names for the controller values.
pub fn text(id: u32, lang: usize, csec: usize, gsec: usize, playstation: bool) -> Vec<u8> {
	let l = lang.min(4);
	let fixed = |t: &T5| t[l].to_vec();
	match id {
		ID_CONTROLS => fixed(&CONTROLS),
		ID_HEADER => fixed(&HEADER),
		ID_DESC => fixed(&DESC),
		ID_PAGE => fixed(&PAGE),
		ID_SELC => stepper(SECTIONS[csec.min(NSEC - 1)][l]),
		ID_SELG => stepper(GSECTIONS[gsec.min(NGSEC - 1)][l]),
		ID_HUD => fixed(&HUD),
		ID_SHARP => fixed(&SHARP),
		ID_ORIGINAL => fixed(&ORIGINAL),
		ID_GLYPHS => fixed(&GLYPHS),
		ID_PLAYSTATION => fixed(&PLAYSTATION),
		ID_XBOX => fixed(&XBOX),
		ID_SWITCH => fixed(&SWITCH),
		ID_DBG0 => fixed(&DBG_MODE),
		_ if (ID_DBG_KEY..ID_DBG_KEY + NDKEYS as u32).contains(&id) => fixed(&DBG_KEYS[(id - ID_DBG_KEY) as usize]),
		_ if (ID_DBG_VAL..ID_DBG_VAL + NDKEYS as u32).contains(&id) => DBG_VALS[(id - ID_DBG_VAL) as usize].to_vec(),
		_ if (ID0 + 1..ID0 + ROWS as u32).contains(&id) => match target(csec, (id - ID0) as usize) {
			Target::Func(Cap::Key(f) | Cap::Pad(f)) => fixed(&LABELS[f]),
			Target::ResetKeys => fixed(&RESET_KEYS),
			Target::ResetPad => fixed(&RESET_PAD),
			_ => Vec::new(),
		},
		_ if (ID_VALUE + 1..ID_VALUE + ROWS as u32).contains(&id) => match target(csec, (id - ID_VALUE) as usize) {
			Target::Func(Cap::Key(f)) => binds::kb().shown(f).into_bytes(),
			Target::Func(Cap::Pad(f)) => binds::pad_shown(binds::pad_map()[f], playstation).as_bytes().to_vec(),
			_ => Vec::new(),
		},
		_ if (ID_PRESS + 1..ID_PRESS + ROWS as u32).contains(&id) => match target(csec, (id - ID_PRESS) as usize) {
			Target::Func(Cap::Key(_)) => fixed(&PRESS_KEY),
			Target::Func(Cap::Pad(_)) => fixed(&PRESS_PAD),
			Target::ResetKeys | Target::ResetPad => fixed(&DONE),
			_ => Vec::new(),
		},
		_ => Vec::new(),
	}
}

/// The text lookup hand for the pages' ids (`pcext::text_hook`, 0x7f40..0x7f7f): the string goes into the id's pool slot and its
/// address into v0, in the game's language `lang`.
pub fn text_hook(c: &mut Cpu, id: u32, lang: usize) -> bool {
	let (csec, gsec) = state(|st| (st.ctl.sec, st.gfx.sec));
	let s = text(id, lang, csec, gsec, super::pspad::connected());
	let at = if (ID_DBG0..ID_DBG_END).contains(&id) { DBG_POOL + SLOT * (id - ID_DBG0) } else { POOL + SLOT * (id - ID0) };
	let n = s.len().min(SLOT as usize - 1);
	for (i, b) in s[..n].iter().enumerate() {
		c.mem.wr8(at + i as u32, *b);
	}
	c.mem.wr8(at + n as u32, 0);
	c.s(2, at as u64);
	true
}

/// The texts that do not follow a section (the pages' own rows and the fixed ones): the mirror's text map takes them, so the
/// title screen's Graphics page can show the Button glyphs row.
pub fn fixed_texts(lang: usize) -> impl Iterator<Item = (u32, String)> {
	let l = lang.min(4);
	[
		(ID_CONTROLS, CONTROLS[l]),
		(ID_HEADER, HEADER[l]),
		(ID_DESC, DESC[l]),
		(ID_PAGE, PAGE[l]),
		(ID_HUD, HUD[l]),
		(ID_SHARP, SHARP[l]),
		(ID_ORIGINAL, ORIGINAL[l]),
		(ID_GLYPHS, GLYPHS[l]),
		(ID_PLAYSTATION, PLAYSTATION[l]),
		(ID_XBOX, XBOX[l]),
		(ID_SWITCH, SWITCH[l]),
	]
	.into_iter()
	.map(|(id, t)| (id, t.iter().map(|&b| b as char).collect()))
}

// ---- the tables and the pages --------------------------------------------------------------------------------------------

fn copy_words(mem: &mut Mem, from: u32, to: u32, len: u32) {
	for o in (0..len).step_by(4) {
		let w = mem.rd32(from + o);
		mem.wr32(to + o, w);
	}
}

/// Writes a row table (6-word records) at `at`, padded with zero records to the table's capacity (no stale record of a longer
/// section stays behind the zero record).
fn write_table(mem: &mut Mem, at: u32, t: &[u32]) {
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
			Target::DebugKey(k) => [ID_DBG_KEY + k as u32, CBASE + r, ID_DBG_VAL + k as u32, ID_DBG_VAL + k as u32, 0, 0],
			_ => [ID0 + r, CBASE + r, ID_VALUE + r, ID_PRESS + r, 0, 0],
		});
	}
	t
}

/// The Graphics row table of section `sec`: the stepper, then the section's rows of `rows` (`pcext::ROWS`) and the HUD row.
fn gfx_table(rows: &[[u32; 6]], sec: usize) -> Vec<u32> {
	let mut t = vec![ID_PAGE, G_SEL, ID_SELG, ID_SELG, 0, 0];
	for &i in GSECS[sec.min(NGSEC - 1)] {
		t.extend(match i {
			HUD_ROW => [ID_HUD, HUDB, ID_SHARP, ID_ORIGINAL, 0, 0],
			// The Button glyphs row: its byte is in pcext's block, with the other Graphics rows' bytes, so the title screen's
			// Graphics page shares it.
			GLYPH_ROW => [ID_GLYPHS, super::pcext::GLYPHS, ID_PLAYSTATION, ID_XBOX, ID_SWITCH, 0],
			_ => rows[i],
		});
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
/// the HUD byte are set up too (`hud_sharp`: the options file's row); a rebuild after a level load keeps the player's bytes.
pub fn build_gfx(mem: &mut Mem, gtab: u32, rows: &[[u32; 6]], hud_sharp: bool) {
	let fresh = mem.rd32(G_MAGIC_AT) != MAGIC;
	if fresh {
		mem.wr8(G_SEL, 0);
		mem.wr8(HUDB, !hud_sharp as u8);
		mem.wr32(G_MAGIC_AT, MAGIC);
	}
	state(|st| {
		if fresh {
			st.gfx.seen = 0xff;
		}
		write_table(mem, gtab, &gfx_table(rows, st.gfx.sec));
	});
}

/// The page's address (the Options record's argument).
pub const PAGE_AT: u32 = CPAGE;

/// The HUD row: true = Sharp. None until the Graphics page was built on this RAM.
pub fn hud_sharp(mem: &mut Mem) -> Option<bool> {
	(mem.rd32(G_MAGIC_AT) == MAGIC).then(|| mem.rd8(HUDB) == 0)
}

/// The Debug mode row: true = On. None until the Controls page was built on this RAM.
pub fn debug_on(mem: &mut Mem) -> Option<bool> {
	(mem.rd32(MAGIC_AT) == MAGIC).then(|| mem.rd8(DBGB) != 0)
}

// ---- worker: the steppers and the rows' bytes ----------------------------------------------------------------------------

/// A page's section stepper: the section shown and the selector byte as last seen (0xff: not known yet, the next look is the
/// baseline).
struct Pager {
	sec: usize,
	seen: u8,
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
}

impl St {
	const NEW: St = St { ctl: Pager { sec: 0, seen: 0xff }, gfx: Pager { sec: 0, seen: 0xff }, row: None, done: None, held: [0; 2] };
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
const MENU_MODE: u32 = 0x15f5a4;

/// Whether the open pause page is `page` with `item` as its current item and its cursor on row 0, idle (not flying in or out).
fn on_selector(mem: &mut Mem, level: u32, page: u32, item: u32) -> bool {
	if mem.rd32(crate::retail::disc::va(MENU_MODE)) != mouse_menu::MODE {
		return false;
	}
	let ram = &mem.ram;
	let rd = |a: u32| -> u32 {
		let i = (a & 0x01ff_ffff) as usize;
		ram.get(i..i + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
	};
	match mouse_menu::read(&rd, level, mouse_menu::MODE) {
		Some(snap) => snap.page == page && snap.id == 0x1d && snap.state == 0x1d && snap.cur == item && rd(item + 0x38) == 0,
		None => false,
	}
}

/// Whether a held direction steps in this field: on the press, then after 12 fields every 4 (the menu's own auto-repeat feel).
fn repeat_due(held: u32) -> bool {
	held == 1 || (held >= 13 && (held - 13) % 4 == 0)
}

/// One field of a stepper: the section after this field's input, if it changed. `sel` is the selector byte (the retail item's
/// Cross toggled it: the next section), `dir` Left / Right this field (the previous / next section), `on_row` (asked only when
/// there is an input) whether the page's cursor is on the selector. The steps wrap around the `n` sections.
fn pager_step(p: &mut Pager, sel: u8, dir: [bool; 2], n: usize, on_row: impl FnOnce() -> bool) -> Option<usize> {
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
	write_table(mem, gtab, &gfx_table(rows, sec));
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
		}
	});
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
			Target::Func(cap) => {
				// A press on another row while one waits: that capture ends first.
				if let Some(old) = st.row.take() {
					binds::capture_cancel();
					binds::capture_ended();
					mem.wr8(CBASE + old as u32, 0);
				}
				binds::capture_begin(cap);
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
		}
	}
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

	/// The 13 rows of `pcext::ROWS` as far as the tables care: distinct labels.
	fn fake_rows() -> Vec<[u32; 6]> {
		(0..13u32).map(|i| [0x100 + i, 0x200 + i, 0x300, 0x301, 0, 0]).collect()
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
		assert!(PICS + 9 * 8 <= POOL && CTAB + 24 * TABLE_RECS as u32 <= PICS && CVAL + 0x40 <= CTAB && CHDR + 0x58 <= CVAL && CPAGE + 0x100 <= CHDR && MAGIC_AT + 4 <= CPAGE);
		assert!(CBASE + ROWS as u32 <= G_SEL && HUDB < G_MAGIC_AT && G_MAGIC_AT + 4 <= MAGIC_AT);
		assert_eq!(table(0).len(), 7 * 6);
		assert!(ID_HUD < ID_LAST && ID_ORIGINAL < ID_LAST);
		assert_eq!(TABLE_RECS, 8, "the Graphics table (pcext GTAB) has room for 14 records");
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
						let reset_value = base == ID_VALUE && csec == 8;
						assert_eq!(empty, !used || reset_value, "{base:#x} row {r} section {csec} {lang}");
					}
				}
			}
			for id in [ID_CONTROLS, ID_HEADER, ID_DESC, ID_PAGE, ID_HUD, ID_SHARP, ID_ORIGINAL] {
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
				assert_eq!(s.is_empty(), id >= ID_DBG_VAL + NDKEYS as u32, "{id:#x} {lang}: 0x7fa0..0x7faa have text, 0x7fab..0x7faf none");
				if (ID_DBG_KEY..ID_DBG_VAL).contains(&id) || id == ID_DBG0 {
					assert!(s.len() <= 24, "a label must fit the row: {id:#x} {lang}: {s:?}");
				}
			}
			assert_eq!(text(ID_SELC, lang, SEC_DEBUG, 0, false), b"< Debug >");
		}
		assert_eq!((text(ID_DBG0, 0, 9, 0, false), text(0x7fa2, 1, 9, 0, false), text(0x7fa4, 3, 9, 0, false), text(0x7fa8, 2, 9, 0, false)), (b"Debug mode".to_vec(), b"Cam\x8cra libre".to_vec(), b"Munici\x9cn infinita".to_vec(), b"F5".to_vec()));
		// The Debug section is the last one: the Page stepper wraps from Reset to it and from it to the first section.
		assert_eq!(SEC_DEBUG, NSEC - 1);
		let mut p = Pager { sec: 8, seen: 0 };
		assert_eq!(pager_step(&mut p, 1, [false; 2], NSEC, || true), Some(9));
		p.sec = 9;
		assert_eq!(pager_step(&mut p, 0, [false; 2], NSEC, || true), Some(0));
	}

	/// The Debug mode row: the byte is set from the menu switch on fresh RAM, `debug_on` reads it, the key rows' bytes are cleared.
	#[test]
	fn the_debug_row_byte_is_read_and_the_key_rows_are_inert() {
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
		assert_eq!(text(ID_SELC, 0, 0, 0, false), b"< Keyboard buttons 1/2 >");
		assert_eq!(text(ID_SELC, 2, 7, 0, false), b"< Controller: Kreuz >");
		assert_eq!(text(ID_SELG, 0, 0, 1, false), b"< Effects >");
		assert_eq!(text(ID_SELG, 3, 0, 2, false), b"< Pantalla >");
		assert_eq!(text(ID_PAGE, 2, 0, 0, false), b"Seite");
		assert_eq!((text(ID_HUD, 0, 0, 0, false), text(ID_SHARP, 3, 0, 0, false), text(ID_ORIGINAL, 4, 0, 0, false)), (b"HUD".to_vec(), b"N\x94tido".to_vec(), b"Originale".to_vec()));
	}

	/// Every function appears on exactly one row of the sections that show its device, no section has more than six rows.
	#[test]
	fn sections_cover_the_functions() {
		assert!((0..NSEC).all(|s| (1..=ROWS - 1).contains(&rows(s).len())));
		let mut keys: Vec<usize> = (0..NSEC).flat_map(rows).filter_map(|t| if let Target::Func(Cap::Key(f)) = t { Some(f) } else { None }).collect();
		keys.sort();
		assert_eq!(keys, (0..binds::NK).collect::<Vec<_>>());
		let mut pad: Vec<usize> = (0..NSEC).flat_map(rows).filter_map(|t| if let Target::Func(Cap::Pad(f)) = t { Some(f) } else { None }).collect();
		pad.sort();
		assert_eq!(pad, (0..binds::NP).collect::<Vec<_>>());
		assert_eq!((rows(8), target(8, 3)), (vec![Target::ResetKeys, Target::ResetPad], Target::Blank));
		// Device per section: 0..=4 keyboard, 5..=7 controller.
		assert!((0..5).all(|s| rows(s).iter().all(|t| matches!(t, Target::Func(Cap::Key(_))))));
		assert!((5..8).all(|s| rows(s).iter().all(|t| matches!(t, Target::Func(Cap::Pad(_))))));
	}

	/// Every Graphics row (the 13 of pcext's `ROWS`, the HUD row and the Button glyphs row) is on exactly one page of at most six rows.
	#[test]
	fn graphics_sections_cover_the_rows() {
		let mut all: Vec<usize> = GSECS.iter().flat_map(|s| s.iter().copied()).collect();
		all.sort();
		assert_eq!(all, (0..=GLYPH_ROW).collect::<Vec<_>>());
		assert!(GSECS.iter().all(|s| (1..=ROWS - 1).contains(&s.len())));
		let rows = fake_rows();
		for sec in 0..NGSEC {
			let t = gfx_table(&rows, sec);
			assert_eq!(&t[..6], &[ID_PAGE, G_SEL, ID_SELG, ID_SELG, 0, 0], "the stepper row: two equal choices");
			assert_eq!(t.len(), 6 * (1 + GSECS[sec].len()));
		}
		let t = gfx_table(&rows, 0);
		assert_eq!(&t[6..12], &rows[0]);
		assert_eq!(&t[t.len() - 6..], &[ID_HUD, HUDB, ID_SHARP, ID_ORIGINAL, 0, 0], "HUD closes the Picture page");
		assert_eq!(&gfx_table(&rows, 1)[6..12], &rows[4], "Effects starts with Water");
		let s = gfx_table(&rows, 2);
		assert_eq!(&s[s.len() - 6..], &[ID_GLYPHS, crate::retail::pcext::GLYPHS, ID_PLAYSTATION, ID_XBOX, ID_SWITCH, 0], "Button glyphs closes the Screen page");
	}

	/// The Graphics table is rewritten for the section; a shorter section leaves no stale record behind its zero record.
	#[test]
	fn the_graphics_table_follows_the_section_and_the_hud_byte_survives() {
		let mut mem = Mem::new();
		let rows = fake_rows();
		let gtab = BASE + 0xaa0;
		assert_eq!(hud_sharp(&mut mem), None, "not built yet");
		build_gfx(&mut mem, gtab, &rows, true);
		assert_eq!((hud_sharp(&mut mem), mem.rd8(G_SEL)), (Some(true), 0));
		let rec = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..6).map(|k| mem.rd32(gtab + 24 * i + 4 * k)).collect() };
		assert_eq!(rec(&mut mem, 0), [ID_PAGE, G_SEL, ID_SELG, ID_SELG, 0, 0]);
		assert_eq!(rec(&mut mem, 6), [ID_HUD, HUDB, ID_SHARP, ID_ORIGINAL, 0, 0]);
		assert_eq!(rec(&mut mem, 7), [0; 6]);
		test_gfx_section(&mut mem, 1, &rows, gtab);
		assert_eq!(rec(&mut mem, 1), rows[4]);
		assert_eq!(rec(&mut mem, 4), rows[8]);
		assert_eq!((rec(&mut mem, 5), rec(&mut mem, 6), rec(&mut mem, 7)), (vec![0; 6], vec![0; 6], vec![0; 6]), "no record of the Picture page is left");
		// The player picks Original; a rebuild (level load) keeps it, a fresh RAM takes the file's value.
		mem.wr8(HUDB, 1);
		assert_eq!(hud_sharp(&mut mem), Some(false));
		build_gfx(&mut mem, gtab, &rows, true);
		assert_eq!(hud_sharp(&mut mem), Some(false));
		assert_eq!(rec(&mut mem, 1), rows[4], "the table is the section's the player left");
		let mut fresh = Mem::new();
		build_gfx(&mut fresh, gtab, &rows, false);
		assert_eq!(hud_sharp(&mut fresh), Some(false), "hud=original in the file");
		test_gfx_section(&mut mem, 0, &rows, gtab);
	}

	/// The stepper: Cross (the byte toggles) steps on and wraps, Left / Right step back / on and wrap, only with the cursor on the
	/// row, and the first look is only a baseline.
	#[test]
	fn the_pager_steps_and_wraps() {
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
		binds::init(None, None);
		let mut mem = page_mem();
		mem.wr8(CBASE + 1, 1); // Cross row of section 0
		sync(&mut mem);
		assert!(binds::capturing());
		assert_eq!(mem.rd8(CBASE + 1), 1, "the row reads 'Press a key' while it waits");
		binds::capture_poll(&[K::KeyK], 0);
		assert!(!binds::capturing());
		sync(&mut mem);
		assert_eq!(mem.rd8(CBASE + 1), 0);
		assert_eq!(text(ID_VALUE + 1, 0, 0, 0, false), b"K");
		assert_eq!(binds::kb().0[binds::CROSS][0], Some(K::KeyK));
		assert_eq!(binds::take_changes().map(|c| c.0), Some(Some("cross:KeyK".to_string())));
		// Esc ends a capture without a binding.
		mem.wr8(CBASE + 2, 1);
		sync(&mut mem);
		binds::capture_poll(&[K::Escape], 0);
		sync(&mut mem);
		assert_eq!((mem.rd8(CBASE + 2), binds::take_changes()), (0, None));
		// A controller row waits for a button (section 5: Controller buttons 1/2).
		go(&mut mem, 5);
		mem.wr8(CBASE + 1, 1);
		sync(&mut mem);
		binds::capture_poll(&[K::KeyZ], 0);
		assert!(binds::capturing(), "a key does not bind a controller function");
		binds::capture_poll(&[], 0x2000);
		sync(&mut mem);
		assert_eq!(text(ID_VALUE + 1, 0, 5, 0, false), b"B");
		assert_eq!(text(ID_VALUE + 1, 0, 5, 0, true), b"Circle");
		assert_eq!(text(ID_VALUE + 4, 0, 5, 0, false), b"A", "Circle's row: the button Cross gave up (a swap)");
		assert_eq!(binds::pad_map()[binds::CROSS], 1);
		binds::init(None, None);
		go(&mut mem, 0);
	}

	/// A section change cancels a capture and rewrites the table; a press on a blank row is ignored; the reset rows restore the
	/// defaults and read Done.
	#[test]
	fn sections_reset_and_blank_rows() {
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
		go(&mut mem, 8);
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

	/// Before `build_page` ran on this RAM nothing is read or written (the title screen, other overlays).
	#[test]
	fn sync_is_inert_without_the_page() {
		let mut mem = Mem::new();
		mem.wr8(CBASE + 1, 1);
		sync(&mut mem);
		assert!(!binds::capturing());
		assert_eq!(mem.rd8(CBASE + 1), 1);
		field(&mut mem, 4, 0x2000, &fake_rows(), BASE + 0xaa0);
		assert_eq!(csec(), 0);
	}
}
