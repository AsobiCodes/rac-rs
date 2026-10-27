//! PC options in the game's own pause menu (Pause -> Options -> Camera Options, retail page id 0x1d): two extra rows in the
//! retail look, remembered between sessions (`pcopts.rs`, `<player dir>/pc_options.txt`).
//!
//! - `Camera: Classic / Static / Smooth`: Static and Smooth = the mouse-driven modern camera (modcam.rs; Smooth is its dynamic layer,
//!   Static the plain Modern camera of before), Classic = retail's camera and controls. A new player starts with Smooth.
//! - `Strafe aiming: On / Off`: the right mouse button's strafe mode (modcam's aim mode, strafe.rs). It only matters while the
//!   camera is Modern; with Classic strafe aiming is off whatever the row says. A new player starts with On.
//!
//! How (design: scratch_race/OPTIONS_DESIGN.md): the retail multi-choice item (update 0x27e898 / draw 0x27ea40 on every level)
//! reads its rows from a table of 6-word records `{label id, byte*, choice0..3}` ended by a zero word, so a row is data:
//! - `patch_rows` (worker, each field): on the running level, when the retail table sits where the signature says, a copy
//!   with five records (the three retail ones + Camera + Strafe) is built in reserved RAM and the item's table pointer (+0x34) is
//!   repointed to it. Overlay data is initialised by every level load, so the poll patches again after a reload.
//! - The rows' values are bytes in reserved RAM (`BASE`), outside the save struct, so the memory card stays 1:1. The
//!   retail update toggles them itself; `pre_field` reads them back every field, applies them (modcam's camera, the strafe gate)
//!   and writes the file when one changed. F7 flips the camera byte.
//! - Three texts are new (ids 0x7f01..0x7f03: Classic, Modern, the strafe label). A wrapping hand on the retail text lookup
//!   (`FUN_00203620` twin, 16 instructions, per level) answers them from a pool in reserved RAM in the game's current language
//!   and declines every other id, so retail's own lookup runs for those. Labels and On/Off reuse retail's text ids
//!   (0x4f23 Camera, 0x4f5a On, 0x4f5b Off), so only three strings have wording of their own.
//! - Language: Options -> Language (title) already exists; the host watches the game's language word and remembers it
//!   (`watch_language`), and a cold boot starts in that language (`init`). The retail card is untouched.
//! - Graphics page (Pause -> Options -> Graphics, `patch_graphics`): the Options list (button/list item 0x2781d0 twin) reads its
//!   rows from 12-byte records `{text id, type, arg}` (type 3 = go to the page at arg), its description item (flags 0x90) and
//!   its picture item (flags 0x20) index their own tables by the list row, so a row is data too: copies with a Graphics row
//!   before Quit Game go to reserved RAM and the three pointers are repointed. The row opens a copy of the Camera Options
//!   page (same page id 0x1d, layout, header and multi-choice item; the header shows "Graphics Options" and the item reads
//!   the Graphics rows), so it is drawn, steered and sounds exactly like the retail value pages. Rows: Lighting
//!   Classic / Enhanced, Ambient occlusion and Dynamic shadows Off / High / Medium / Low (the menu bytes 0..3 keep today's 0 Off, 1 On = High; `swap_level` maps them to the levels), Display mode Windowed / Fullscreen,
//!   Water and Particles Classic / High / Medium / Low, Sun rays Off / High / Medium / Low (`graphics`, viewer/post.rs; High is
//!   the full Enhanced picture, Low and Medium cost the GPU less: `Graphics::ao_q` ..), Frame smoothing Off / On (viewer/interp.rs),
//!   Vegetation Off / Moderate / Lush, Culling On / Off, Global illumination Off / High / Medium / Low (viewer/post.rs `fs_gi`), Waves On / Off (viewer/waterfx.rs `geo_look`), Draw distance Medium / High / Low (viewer/fardraw.rs, with either Lighting;
//!   the byte order puts the default Medium at 0 and still cycles Low -> Medium -> High); a new player sees the retail lighting
//!   (Original) with today's Medium draw distance. The picture of the row is help_ss 5 (the lit planet), from the same ToC.
//! - Paged Graphics / Controls (pcctl.rs): in a level the pause pages show at most 7 rows, row 0 is a "Page" selector (Left / Right
//!   or Cross steps and wraps): Graphics has 3 sections (Picture: Lighting, Ambient occlusion, Dynamic shadows, Global
//!   illumination, Sun rays, HUD; Effects: Water, Waves, Particles, Vegetation; Screen: Display mode, Frame smoothing, Draw
//!   distance, Culling, Button glyphs), Controls 9 (pcctl.rs). `HUD: Sharp / Original` is a byte of the pcctl block that `sync` applies to
//!   `hud_sharp_now()` and `hud=` in the file. `Button glyphs: PlayStation / Xbox / Switch` is a byte of pcext's own block
//!   (`GLYPHS`) that `sync` applies to `glyph_style_now()` and `glyphs=` in the file: it says how a gamepad's buttons are named in
//!   the prompts (prompts.rs). The title's Graphics page keeps the 13 retail rows and has no HUD row, but it has the Button
//!   glyphs row.
//! - Title screen: Options -> Graphics (`title_options`, a hand on the front end's Options list set-up) shows the same rows on
//!   a copy of the Video Options page, with the Button glyphs row after them; `front_field` applies and remembers them, the mirror
//!   draws them (viewer/front_mirror.rs, frontend.rs `Page::Graphics`). The Options rows themselves are retail's of the disc's
//!   region (USA: no Video row).
//!
//! - Debug mode (Controls -> Debug, pcctl.rs; `debug=on` in the file, default Off): `sync` hands the row's byte to `devtools::set_menu`, so the
//!   dev tools (F1 planet list / warp, F2 free camera) and the debug keys (F3 draw distance A/B, F5 immortality, F6 infinite ammo, F8
//!   +10000 bolts) work without the environment variables (those still force them on); switching it On draws a notice with the keys
//!   and a warning. Off, nothing is taken, drawn or written.
//!
//! `RAC_PC_EXT=0` registers nothing and writes nothing (today's bit-exact path); the oracle's gates never link this file.
use super::pcopts::{self, Lang, PcOptions};
use super::{modcam, FieldIn};
use ee_oracle::cpu::Cpu;
use ee_oracle::mem::Mem;
use ee_oracle::rec::{register_hand, Hand};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

/// Reserved RAM: all zero in every savestate scanned, below the ELF image (0x100000) and outside the rec/hand range.
pub const BASE: u32 = 0xB0000;
/// "PCX1": the region has been set up (fresh RAM after a start, a savestate load or a cold boot reads 0 here).
const MAGIC: u32 = 0x3158_4350;
/// Camera style: 0 Classic, 1 Modern (the byte the retail row toggles).
pub const CAM: u32 = BASE + 4;
/// Strafe aiming: 0 Off, 1 On.
pub const STRAFE: u32 = BASE + 5;
/// Buy all ammo at a vendor: 0 Off, 1 On.
pub const BUYALL: u32 = BASE + 6;
/// Graphics -> Lighting: 0 Classic (the retail picture), 1 Enhanced.
pub const LIGHT: u32 = BASE + 8;
/// Graphics -> Ambient occlusion: 0 Off, 1 Low, 2 Medium, 3 High.
pub const AO: u32 = BASE + 9;
/// Graphics -> Dynamic shadows: 0 Off, 1 Low, 2 Medium, 3 High.
pub const SHADOWS: u32 = BASE + 10;
/// Graphics -> Display mode: 0 Windowed, 1 Fullscreen (borderless, viewer/handler.rs `apply_display`).
pub const DISPLAY: u32 = BASE + 11;
/// Graphics -> Water: 0 Classic, 1 Low, 2 Medium, 3 High (viewer/waterfx.rs; with Lighting Enhanced).
pub const WATER: u32 = BASE + 12;
/// Graphics -> Particles: 0 Classic, 1 Low, 2 Medium, 3 High (with Lighting Enhanced).
pub const PARTICLES: u32 = BASE + 13;
/// Graphics -> Sun rays: 0 Off, 1 Low, 2 Medium, 3 High (with Lighting Enhanced).
pub const SUNRAYS: u32 = BASE + 14;
/// Graphics -> Frame smoothing: 0 Off, 1 On (viewer/interp.rs: the picture between two fields).
pub const SMOOTH: u32 = BASE + 15;
/// Graphics -> Vegetation: 0 Off, 1 Moderate, 2 Lush (viewer/vegfx.rs; with Lighting Enhanced).
pub const VEG: u32 = BASE + 16;
/// Graphics -> Culling: 0 On, 1 Off (the renderer draws what it would leave out as hidden or out of view; viewer/gpu.rs
/// `culling`). 0 = On so a new player's byte is the zero RAM held there before the row existed.
pub const CULL: u32 = BASE + 17;
/// Graphics -> Global illumination: 0 Off, 1 High, 2 Medium, 3 Low (the effect rows' bytes, `swap_level`; viewer/post.rs `fs_gi`;
/// with Lighting Enhanced). 0 = Off, also the zero RAM held there before the row existed.
pub const GI: u32 = BASE + 18;
/// Graphics -> Waves: 0 On, 1 Off (the geometric sea waves of Water Medium / High, viewer/waterfx.rs `geo_look`; Off keeps the
/// flat sea with its shaded ripples). 0 = On so a new player's byte is the zero RAM held there before the row existed.
pub const WAVES: u32 = BASE + 19;
/// Graphics -> Draw distance: 0 Medium, 1 High, 2 Low (`pcopts::draw_level`; viewer/fardraw.rs, with either Lighting). 0 = Medium
/// so a new player's byte is the zero RAM held there before the row existed (today's Enhanced distances).
pub const DRAW: u32 = BASE + 20;
/// Graphics -> Button glyphs: 0 PlayStation (the retail's own glyphs), 1 Xbox, 2 Switch. Its byte is here, with the other Graphics
/// rows' bytes, so the title screen's Graphics page and the pause menu's share it.
pub const GLYPHS: u32 = BASE + 21;
/// The Graphics page's row bytes in row order.
const GROWS: [u32; 13] = [LIGHT, AO, SHADOWS, DISPLAY, WATER, PARTICLES, SUNRAYS, SMOOTH, VEG, CULL, GI, WAVES, DRAW];
/// The copy of the row table (5 records + the zero record).
const TAB: u32 = BASE + 0x40;
/// String pool: one 0x40-byte slot per new text (`NEW_IDS` order; 28 slots end at BASE + 0x800, the rest from `POOL2`).
const POOL: u32 = BASE + 0x100;
const POOL_SLOTS: usize = 28;
/// The string pool's second part: after the title's Graphics rows (they end at BASE + 0xef8), before prompts.rs (4 slots).
const POOL2: u32 = BASE + 0x1800;
/// Graphics page (`patch_graphics`): the Options list records (8 + the zero record, 12 bytes each), its row descriptions (8 + 0),
/// its row pictures (8 ToC records {sector, sectors}), the page (a copy of Camera Options; 0x100 bytes, zero after its 0x60:
/// the menu engine reads item slots past the page's zero item, retail's own pages are followed by zeros there), its header
/// item, its multi-choice item and that item's rows (13 + the zero record, 24 bytes each, ending at BASE + 0xbf0; prompts.rs starts at
/// BASE + 0x1000).
const OPT_RECS: u32 = BASE + 0x800;
const OPT_DESC: u32 = BASE + 0x880;
/// The Options pictures (9 records now: Graphics and Controls) live in the Controls block (`pcctl.rs`): 0x48 bytes here would reach GPAGE.
const OPT_PICS: u32 = super::pcctl::PICS;
pub const GPAGE: u32 = BASE + 0x900;
const GHDR: u32 = BASE + 0xa00;
pub const GVAL: u32 = BASE + 0xa60;
const GTAB: u32 = BASE + 0xaa0;
/// The title's Graphics page (`title_options`): the Options list records (up to 4 + the zero record, 12 bytes each), the page
/// (a copy of Video Options; 0x100 bytes, zero after its 0x88), its header item, its value item and that item's rows (13 + the
/// zero record, 20 bytes each: the front end's value item reads 5-word rows), ending at BASE + 0xef8.
const TRECS: u32 = BASE + 0xc00;
pub const TGPAGE: u32 = BASE + 0xc40;
const TGHDR: u32 = BASE + 0xd40;
/// The title's row table (15 records of 20 bytes: the 13 Graphics rows, the Button glyphs row and the zero record) and the
/// value item (64 bytes) share the space the two had before, with the row table first: the Button glyphs row needs one more
/// record and the value item does not fill its old place.
const TGTAB: u32 = BASE + 0xdd8;
pub const TGVAL: u32 = BASE + 0xd98;
/// The Graphics rows of both pages: label, row byte, the value texts for 0, 1, 2 and 3 (0: no such value; the retail item's
/// record holds four choices). The pause page's multi-choice item cycles a row through its values; the title's value item only
/// flips a byte, so `sync` steps the rows with more than two values on.
#[rustfmt::skip]
const ROWS: [[u32; 6]; 13] = [
	[ID_LIGHTING, LIGHT, ID_LIGHT_CLASSIC, ID_ENHANCED, 0, 0],
	[ID_AO, AO, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_SHADOWS, SHADOWS, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_DISPLAY, DISPLAY, ID_WINDOWED, ID_FULLSCREEN, 0, 0],
	[ID_WATER, WATER, ID_CLASSIC, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_PARTICLES, PARTICLES, ID_PART_CLASSIC, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_SUNRAYS, SUNRAYS, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_SMOOTHING, SMOOTH, ID_OFF, ID_ON, 0, 0],
	[ID_VEGETATION, VEG, ID_OFF, ID_VEG_MODERATE, ID_VEG_LUSH, 0],
	[ID_CULLING, CULL, ID_ON, ID_OFF, 0, 0],
	[ID_GI, GI, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_WAVES, WAVES, ID_ON, ID_OFF, 0, 0],
	[ID_DRAW, DRAW, ID_MEDIUM, ID_HIGH, ID_LOW, 0],
];

/// How many values row `i` of `ROWS` has (2, 3 or 4).
fn row_values(i: usize) -> u8 {
	ROWS[i][2..].iter().take_while(|&&v| v != 0).count() as u8
}
/// Front end texts (frontbin): the Options rows Video and Sound, the Video Options header and its one row.
const FRONT_VIDEO: u32 = 0x4f21;
const FRONT_SOUND: u32 = 0x4f22;
const FRONT_VIDEO_HDR: u32 = 0x4f3a;
const FRONT_VIDEO_ROW: u32 = 0x4f3b;
/// The Graphics row of the Options list: before Quit Game (the 7th retail row).
const GRAPHICS_ROW: usize = 6;
/// The retail Options rows (text ids, list item records at +0x34): HelpDesk, Save, Load, Sound, Camera, Subtitles, Quit Game.
const OPTIONS_ROWS: [u32; 7] = [0x4f1e, 0x4f1f, 0x4f20, 0x4f22, 0x4f23, 0x4f24, 0x4f26];
/// The Camera Options header text (its header item's +0x34).
const ID_CAMERA_OPTIONS: u32 = 0x4f42;
/// The game's language word.
const LANG_WORD: u32 = 0x15ed88;

const ID_CAMERA: u32 = 0x4f23;
const ID_ON: u32 = 0x4f5a;
const ID_OFF: u32 = 0x4f5b;
pub const ID_CLASSIC: u32 = 0x7f01;
pub const ID_MODERN: u32 = 0x7f02;
pub const ID_STRAFE: u32 = 0x7f03;
pub const ID_BUYALL: u32 = 0x7f04;
/// The Buy plate text with the new line: retail's "[Cross] Buy", a newline, "[Square] label" (vendor.rs draws it on the plate).
pub const ID_BUYALL_PROMPT: u32 = 0x7f05;
/// Graphics page texts: the Options row, the page header, the row's description, the three row labels and two values.
pub const ID_GRAPHICS: u32 = 0x7f06;
pub const ID_GRAPHICS_HDR: u32 = 0x7f07;
pub const ID_GRAPHICS_DESC: u32 = 0x7f08;
pub const ID_LIGHTING: u32 = 0x7f09;
pub const ID_LIGHT_CLASSIC: u32 = 0x7f0a;
pub const ID_ENHANCED: u32 = 0x7f0b;
pub const ID_AO: u32 = 0x7f0c;
pub const ID_SHADOWS: u32 = 0x7f0d;
pub const ID_DISPLAY: u32 = 0x7f0e;
pub const ID_WINDOWED: u32 = 0x7f0f;
pub const ID_FULLSCREEN: u32 = 0x7f10;
pub const ID_WATER: u32 = 0x7f11;
pub const ID_PARTICLES: u32 = 0x7f12;
pub const ID_SUNRAYS: u32 = 0x7f13;
/// Particles' Classic (plural: les particules, los efectos, le particelle). (0x7f14 and 0x7f16, the old Enhanced values of Water
/// and Particles, are free: High replaced them.)
pub const ID_PART_CLASSIC: u32 = 0x7f15;
pub const ID_SMOOTHING: u32 = 0x7f17;
/// Vegetation and its values (feminine: la vegetation, la vegetacion, la vegetazione).
pub const ID_VEGETATION: u32 = 0x7f18;
pub const ID_VEG_MODERATE: u32 = 0x7f19;
pub const ID_VEG_LUSH: u32 = 0x7f1a;
/// The quality levels of the five effect rows (Ambient occlusion, Dynamic shadows, Water, Particles, Sun rays).
pub const ID_LOW: u32 = 0x7f1b;
pub const ID_MEDIUM: u32 = 0x7f1c;
pub const ID_HIGH: u32 = 0x7f1d;
pub const ID_CULLING: u32 = 0x7f1e;
pub const ID_GI: u32 = 0x7f1f;
pub const ID_WAVES: u32 = 0x7f20;
pub const ID_DRAW: u32 = 0x7f21;
const NEW_IDS: [u32; 31] = [
	ID_CLASSIC,
	ID_MODERN,
	ID_STRAFE,
	ID_BUYALL,
	ID_BUYALL_PROMPT,
	ID_GRAPHICS,
	ID_GRAPHICS_HDR,
	ID_GRAPHICS_DESC,
	ID_LIGHTING,
	ID_LIGHT_CLASSIC,
	ID_ENHANCED,
	ID_AO,
	ID_SHADOWS,
	ID_DISPLAY,
	ID_WINDOWED,
	ID_FULLSCREEN,
	ID_WATER,
	ID_PARTICLES,
	ID_SUNRAYS,
	ID_PART_CLASSIC,
	ID_SMOOTHING,
	ID_VEGETATION,
	ID_VEG_MODERATE,
	ID_VEG_LUSH,
	ID_LOW,
	ID_MEDIUM,
	ID_HIGH,
	ID_CULLING,
	ID_GI,
	ID_WAVES,
	ID_DRAW,
];

/// New texts per language (En, Fr, De, Es, It) in the game's font bytes: a-acute 0x84, e-acute 0x8c, e-grave 0x8d, a-umlaut 0x87,
/// o-acute 0x9c, e-circumflex 0x8e. Short like the retail labels (the German ones are abbreviated to fit). [I] wording, native review wanted. The
/// Graphics header and description follow the retail Camera Options ones ("Options cam\x8cra", "Adjust camera control settings").
static TEXTS: [[&[u8]; 5]; 31] = [
	[b"Classic", b"Classique", b"Klassisch", b"Cl\x84sica", b"Classica"],
	[b"Modern", b"Moderne", b"Modern", b"Moderna", b"Moderna"],
	[b"Strafe aiming", b"Vis\x8ce lat\x8crale", b"Strafe-Zielen", b"Apuntado lateral", b"Mira laterale"],
	[b"Refill all ammo", b"Tout remplir", b"Alles f\xa7llen", b"Llenar todo", b"Ricarica tutto"],
	// The Buy plate: retail's own "[Cross] Buy" (id 0x5234), a newline (byte 1), the Square glyph (0x13) and a one-word label that fits
	// the plate on one line (vendor.rs test `plate_text_is_two_lines`; the plate holds about 7 letters after a glyph).
	[b"\x10 Buy\x01\x13 Refill", b"\x10 Acheter\x01\x13 Remplir", b"\x10 Kaufen\x01\x13 F\xa7llen", b"\x10 Comprar\x01\x13 Llenar", b"\x10 Compra\x01\x13 Ricarica"],
	[b"Graphics", b"Graphismes", b"Grafik", b"Gr\x84ficos", b"Grafica"],
	[b"Graphics Options", b"Options graphismes", b"Grafik-Optionen", b"Opciones de gr\x84ficos", b"Opzioni grafica"],
	[b"Adjust lighting and shadow settings", b"R\x8cgler l'\x8cclairage et les ombres", b"Beleuchtung und Schatten \x87ndern", b"Ajusta la iluminaci\x9cn y las sombras", b"Regola illuminazione e ombre"],
	[b"Lighting", b"Lumi\x8dre", b"Beleuchtung", b"Iluminaci\x9cn", b"Illuminazione"],
	// Lighting values (masculine: "le rendu", "el modo"; the Camera row's Classic text 0x7f01 is feminine in Es / It).
	[b"Classic", b"Classique", b"Klassisch", b"Cl\x84sico", b"Classico"],
	[b"Enhanced", b"Am\x8clior\x8c", b"Verbessert", b"Mejorado", b"Migliorato"],
	[b"Ambient occlusion", b"Occlusion ambiante", b"Umgebungsverdeckung", b"Oclusi\x9cn ambiental", b"Occlusione ambientale"],
	[b"Dynamic shadows", b"Ombres dynamiques", b"Dynamische Schatten", b"Sombras din\x84micas", b"Ombre dinamiche"],
	[b"Display mode", b"Affichage", b"Anzeige", b"Pantalla", b"Schermo"],
	[b"Windowed", b"Fen\x8etre", b"Fenster", b"Ventana", b"Finestra"],
	[b"Fullscreen", b"Plein \x8ccran", b"Vollbild", b"Completa", b"Intero"],
	[b"Water", b"Eau", b"Wasser", b"Agua", b"Acqua"],
	// Spanish "Efectos": "Part\xedculas" needs an i-acute the font table notes do not list.
	[b"Particles", b"Particules", b"Partikel", b"Efectos", b"Particelle"],
	[b"Sun rays", b"Rayons de soleil", b"Sonnenstrahlen", b"Rayos de sol", b"Raggi di sole"],
	[b"Classic", b"Classiques", b"Klassisch", b"Cl\x84sicos", b"Classiche"],
	[b"Frame smoothing", b"Lissage d'image", b"Bildgl\x87ttung", b"Suavizado de imagen", b"Movimento fluido"],
	[b"Vegetation", b"V\x8cg\x8ctation", b"Vegetation", b"Vegetaci\x9cn", b"Vegetazione"],
	[b"Moderate", b"Mod\x8cr\x8ce", b"Mittel", b"Moderada", b"Moderata"],
	// German "Dicht": "Uppig" needs a capital U-umlaut the font table notes do not list.
	[b"Lush", b"Luxuriante", b"Dicht", b"Exuberante", b"Rigogliosa"],
	// Quality levels. French High is "Haut": the font table notes list no capital E-acute for "\xc9lev\xe9".
	[b"Low", b"Bas", b"Niedrig", b"Bajo", b"Basso"],
	[b"Medium", b"Moyen", b"Mittel", b"Medio", b"Medio"],
	[b"High", b"Haut", b"Hoch", b"Alto", b"Alto"],
	// [PC] Culling: hiding what is out of view or behind walls.
	[b"Culling", b"Masquage", b"Culling", b"Ocultaci\x9cn", b"Occultamento"],
	// [PC] Global illumination: light bounced off nearby surfaces (viewer/post.rs `fs_gi`).
	[b"Global illumination", b"Illumination globale", b"Globale Beleuchtung", b"Iluminaci\x9cn global", b"Illuminazione globale"],
	// [PC] Waves: the sea's rolling waves (viewer/waterfx.rs `geo_look`).
	[b"Waves", b"Vagues", b"Wellen", b"Olas", b"Onde"],
	// [PC] Draw distance: how far the world is drawn (viewer/fardraw.rs).
	[b"Draw distance", b"Distance d'affichage", b"Sichtweite", b"Distancia de dibujado", b"Distanza visiva"],
];

/// The retail Camera Options row table per level (index = level byte 0x15ed84): `{0x4fa2, 0x15ede0, 0x4fa5, 0x4fa4, 0, 0}` is its
/// first record; the item (+0x34 = table pointer) follows at +0x60, so the pointer to patch is at +0x94. Computed from
/// extracted/levels/N_*/overlay.elf (test `tables_match_the_overlays`).
pub static TABLES: [u32; 19] = [
	0x1b5b40, 0x1b5f40, 0x1b60c0, 0x1b5bc0, 0x1b5c40, 0x1b5fc0, 0x1b6240, 0x1b5b40, 0x1b62c0, 0x1b5c40, 0x1b5f40, 0x1b64c0, 0x1b5f40, 0x1b5dc0, 0x1b6240, 0x1b60c0, 0x1b5f40, 0x1b63c0, 0x1b64c0,
];
/// `TABLES` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
pub static TABLES_USA: [u32; 19] = [
	0x1b5b40, 0x1b5fc0, 0x1b6140, 0x1b5c40, 0x1b5cc0, 0x1b5fc0, 0x1b62c0, 0x1b5bc0, 0x1b6340, 0x1b5cc0, 0x1b5fc0, 0x1b6540, 0x1b5fc0, 0x1b5e40, 0x1b6240, 0x1b6140, 0x1b5fc0, 0x1b6440, 0x1b6540,
];
const PTR_OFF: u32 = 0x94;
const SIG: [u32; 2] = [0x4fa2, 0x15ede0];

/// The text lookup (`FUN_00203620` twin) of every level: entry, end = entry + 0x40, FNV hash of the code (jal targets differ per
/// level). est 13 = the retail function's own length.
static HANDS: [Hand; 21] = [
	Hand { entry: 0x203620, end: 0x203660, hash: 0x3629dc5b6b49d307, est: 13, f: text_hook }, // 0 veldin_tutorial
	Hand { entry: 0x2254b0, end: 0x2254f0, hash: 0x7e7b1f7e1623414b, est: 13, f: text_hook }, // 1 novalis
	Hand { entry: 0x214608, end: 0x214648, hash: 0x85b7ba437d360729, est: 13, f: text_hook }, // 2 aridia
	Hand { entry: 0x1fcf38, end: 0x1fcf78, hash: 0xeb24b940634c9005, est: 13, f: text_hook }, // 3 kerwan
	Hand { entry: 0x1f6b80, end: 0x1f6bc0, hash: 0x6da79bf77928f66f, est: 13, f: text_hook }, // 4 eudora
	Hand { entry: 0x232308, end: 0x232348, hash: 0x3f708f5518982169, est: 13, f: text_hook }, // 5 rilgar
	Hand { entry: 0x21b640, end: 0x21b680, hash: 0xa3eb0366abb23fbf, est: 13, f: text_hook }, // 6 nebula_g34
	Hand { entry: 0x22b408, end: 0x22b448, hash: 0x15c56ecb291b9929, est: 13, f: text_hook }, // 7 umbris
	Hand { entry: 0x216120, end: 0x216160, hash: 0xda3f93931e3c60c7, est: 13, f: text_hook }, // 8 batalia
	Hand { entry: 0x223280, end: 0x2232c0, hash: 0x61e854250e8f17af, est: 13, f: text_hook }, // 9 gaspar
	Hand { entry: 0x1f6c08, end: 0x1f6c48, hash: 0xbcc287a94ed20b29, est: 13, f: text_hook }, // 10 orxon
	Hand { entry: 0x234f08, end: 0x234f48, hash: 0x77db3b94ebeea8e9, est: 13, f: text_hook }, // 11 pokitaru
	Hand { entry: 0x2264b0, end: 0x2264f0, hash: 0x41a7c7eb53740d4b, est: 13, f: text_hook }, // 12 hoven
	Hand { entry: 0x20f088, end: 0x20f0c8, hash: 0x7a1c9242706d3909, est: 13, f: text_hook }, // 13 oltanis_orbit
	Hand { entry: 0x216c30, end: 0x216c70, hash: 0x8b9c875f10f60eeb, est: 13, f: text_hook }, // 14 oltanis
	Hand { entry: 0x1fcd18, end: 0x1fcd58, hash: 0x9ad2134b19b2bbed, est: 13, f: text_hook }, // 15 quartu
	Hand { entry: 0x201e08, end: 0x201e48, hash: 0x6f83b2093c7fd729, est: 13, f: text_hook }, // 16 kalebo_iii
	Hand { entry: 0x200648, end: 0x200688, hash: 0xe641001c37a5fe99, est: 13, f: text_hook }, // 17 veldin_orbit
	Hand { entry: 0x20c388, end: 0x20c3c8, hash: 0xee17fd3646c79949, est: 13, f: text_hook }, // 18 veldin_finale
	// The same function in the boot ELF and in the front end (frontbin: title, New Game, memory card), for the keyboard prompts.
	Hand { entry: 0x1fd7d8, end: 0x1fd818, hash: 0xa54b5b2195d3c76f, est: 13, f: text_hook }, // boot ELF
	Hand { entry: 0x1fd518, end: 0x1fd558, hash: 0x3c354b39385afa9f, est: 13, f: text_hook }, // frontbin
];

// ---- switch and gates -----------------------------------------------------------------------------------------------

/// The extension is on unless `RAC_PC_EXT=0` (read once). Off: nothing is registered, patched or written.
pub fn enabled() -> bool {
	// Off on the USA disc with RAC_USA_EXTRAS=0 (retail/disc.rs).
	if super::disc::extras_off() {
		return false;
	}
	static E: OnceLock<bool> = OnceLock::new();
	*E.get_or_init(|| !std::env::var("RAC_PC_EXT").is_ok_and(|v| v == "0"))
}

/// The camera row as the worker last applied it (true = Modern). The defaults hold until `init` / the first field.
static CAM_MODERN: AtomicBool = AtomicBool::new(true);
/// The strafe row as last applied (true = On).
static STRAFE_ON: AtomicBool = AtomicBool::new(true);
/// The buy-all-ammo row as last applied (true = On).
static BUYALL_ON: AtomicBool = AtomicBool::new(true);
/// The HUD row as last applied (true = Sharp, false = Original); the Graphics page's HUD row (pcctl.rs) writes it.
static HUD_SHARP: AtomicBool = AtomicBool::new(true);

/// The HUD style the Graphics row asks for (true = Sharp, false = Original): `Retail::set_hud_target` reads it every frame.
pub fn hud_sharp_now() -> bool {
	HUD_SHARP.load(Ordering::Relaxed)
}

/// The button glyphs style as last applied (0 PlayStation, 1 Xbox, 2 Switch); the Graphics page's Button glyphs row (pcctl.rs)
/// writes it.
static GLYPH_STYLE: AtomicU8 = AtomicU8::new(1);

/// The button glyphs style the Graphics row asks for: 0 PlayStation (the retail's own glyphs), 1 Xbox, 2 Switch. The prompts
/// (prompts.rs) read it: with the extension off the retail's own glyphs.
pub fn glyph_style_now() -> u8 {
	if !enabled() {
		return super::prompts::STYLE_PS;
	}
	GLYPH_STYLE.load(Ordering::Relaxed).min(2)
}

/// The Graphics rows as last applied (`pack_gfx`; default Classic, the rest On / Enhanced for when the player picks Enhanced).
static GFX: AtomicU32 = AtomicU32::new(pack_gfx(DEFAULT_ROWS));
/// The row bytes of a new player (`PcOptions::graphics_rows` of no file): Classic lighting, the effect rows at High, Windowed,
/// frame smoothing On, Moderate vegetation, culling On, global illumination Off (a new look and GPU cost the player opts into),
/// waves On, draw distance Medium (byte 0).
const DEFAULT_ROWS: [u8; 13] = [0, 1, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0];
/// The Display mode row as last applied (true = Fullscreen).
static FULLSCREEN: AtomicBool = AtomicBool::new(false);

/// The Display mode the window should have (viewer/handler.rs, every frame): false with the extension off.
pub fn fullscreen() -> bool {
	enabled() && FULLSCREEN.load(Ordering::Relaxed)
}

/// The vendor's buy-all-ammo option (vendor.rs): the menu row; false with the extension off (no row, no text, retail exact).
pub fn buyall_on() -> bool {
	enabled() && BUYALL_ON.load(Ordering::Relaxed)
}

/// What the Graphics page asks of the renderer (viewer/post.rs). `lighting` 0 = Classic: the retail picture, nothing added; the
/// other rows only act with Lighting Enhanced, except frame smoothing, culling and the draw distance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Graphics {
	/// 0 Classic (the retail picture), 1 Enhanced.
	pub lighting: u8,
	/// Ambient occlusion on (`ao_q` > 0).
	pub ao: bool,
	pub shadows: bool,
	/// Water Enhanced (viewer/waterfx.rs; `water_q` > 0).
	pub water: bool,
	/// Particles Enhanced (`particles_q` > 0).
	pub particles: bool,
	pub sun_rays: bool,
	/// Frame smoothing On (viewer/interp.rs; works with Lighting Classic too).
	pub smoothing: bool,
	/// Vegetation: 0 Off, 1 Moderate, 2 Lush (viewer/vegfx.rs).
	pub veg: u8,
	/// Culling On (the default): the renderer leaves out what is hidden or out of view. Off draws it (viewer/gpu.rs `culling`).
	pub culling: bool,
	/// The quality level of each effect row: 0 Off (Classic), 1 Low, 2 Medium, 3 High. High is the full Enhanced picture; the
	/// renderer reads these to pick the cheaper settings of Low and Medium.
	pub ao_q: u8,
	pub shadows_q: u8,
	pub water_q: u8,
	pub particles_q: u8,
	pub sun_q: u8,
	/// Global illumination (viewer/post.rs `fs_gi`): 0 Off, 1 Low, 2 Medium, 3 High.
	pub gi_q: u8,
	/// Waves On (the default): the sea of Water Medium / High rolls in geometric waves (viewer/waterfx.rs `geo_look`).
	pub waves: bool,
	/// Draw distance (viewer/fardraw.rs, with either Lighting): 0 Low (retail's distances and PS2 far LODs), 1 Medium (the
	/// default), 2 High (`draw_q`).
	pub draw: u8,
}

impl Graphics {
	pub const CLASSIC: Graphics = Graphics { lighting: 0, ao: false, shadows: false, water: false, particles: false, sun_rays: false, smoothing: false, veg: 0, culling: true, ao_q: 0, shadows_q: 0, water_q: 0, particles_q: 0, sun_q: 0, gi_q: 0, waves: true, draw: 0 };

	/// Enhanced: the post-processing runs.
	pub fn enhanced(self) -> bool {
		self.lighting != 0
	}

	/// The draw distance level: 0 Low (retail), 1 Medium, 2 High.
	pub fn draw_q(self) -> u8 {
		self.draw.min(2)
	}
}

/// The menu byte of an effect row (ambient occlusion, shadows, water, particles, sun rays) and the level it stands for are the
/// same table both ways (an involution): the bytes keep today's values so the retail menu RAM does not change by default (0 Off or
/// Classic, 1 High = today's On / Enhanced, 2 Medium, 3 Low; the menu cycles Off, High, Medium, Low); the levels count up for
/// the renderer and the file (0 Off, 1 Low, 2 Medium, 3 High). The ONE place the two meet.
const fn swap_level(v: u8) -> u8 {
	match v {
		1 => 3,
		2 => 2,
		3 => 1,
		_ => 0,
	}
}

/// `g` (`GROWS` order, bytes or levels) with the six effect rows through `swap_level`.
const fn swap_rows(mut g: [u8; 13]) -> [u8; 13] {
	g[1] = swap_level(g[1]);
	g[2] = swap_level(g[2]);
	g[4] = swap_level(g[4]);
	g[5] = swap_level(g[5]);
	g[6] = swap_level(g[6]);
	g[10] = swap_level(g[10]);
	g
}

/// The row bytes (`GROWS` order) as one word: lighting in bits 0..8, then one "on" bit per effect row (ambient occlusion, shadows,
/// water, particles, sun rays: level > 0; frame smoothing; display mode excluded: `FULLSCREEN`), vegetation in bits 14..16,
/// the five effect levels in two bits each from bit 16 (ambient occlusion, shadows, water, particles, sun rays), culling Off
/// in bit 26, the global illumination level in bits 27..28 (no "on" bit: 0 = Off), waves Off in bit 29 and the draw distance
/// byte (0 Medium, 1 High, 2 Low) in bits 30..31. A word with an "on" bit and a zero level (the packing before the levels
/// existed) reads as High (`unpack_gfx`).
const fn pack_gfx(g: [u8; 13]) -> u32 {
	let veg = if g[8] > 2 { 2 } else { g[8] } as u32;
	let draw = if g[12] > 2 { 2 } else { g[12] } as u32;
	let g = swap_rows(g);
	let (ao, sh, wa, pa, su) = (g[1] as u32, g[2] as u32, g[4] as u32, g[5] as u32, g[6] as u32);
	g[0] as u32
		| ((ao != 0) as u32) << 8
		| ((sh != 0) as u32) << 9
		| ((wa != 0) as u32) << 10
		| ((pa != 0) as u32) << 11
		| ((su != 0) as u32) << 12
		| ((g[7] == 1) as u32) << 13
		| veg << 14
		| ao << 16
		| sh << 18
		| wa << 20
		| pa << 22
		| su << 24
		| ((g[9] == 1) as u32) << 26
		| ((g[10] & 3) as u32) << 27
		| ((g[11] == 1) as u32) << 29
		| draw << 30
}

/// The effect level (0..3) the word holds for the row whose "on" bit is `bit` and whose level sits at `shift`.
fn gfx_level(g: u32, bit: u32, shift: u32) -> u8 {
	match (g >> bit & 1 != 0, (g >> shift & 3) as u8) {
		(false, _) => 0,
		(true, 0) => 3,
		(true, l) => l,
	}
}

/// `pack_gfx`'s inverse: the row bytes (display mode not in the word: 0).
fn unpack_gfx(g: u32) -> [u8; 13] {
	swap_rows([(g & 0xff).min(1) as u8, gfx_level(g, 8, 16), gfx_level(g, 9, 18), 0, gfx_level(g, 10, 20), gfx_level(g, 11, 22), gfx_level(g, 12, 24), (g >> 13 & 1) as u8, (g >> 14 & 3).min(2) as u8, (g >> 26 & 1) as u8, (g >> 27 & 3) as u8, (g >> 29 & 1) as u8, (g >> 30 & 3).min(2) as u8])
}

/// The Graphics page's verdict (render thread, every frame). Classic with the extension off (`RAC_PC_EXT=0`, the smoke path).
pub fn graphics() -> Graphics {
	if !enabled() {
		return Graphics::CLASSIC;
	}
	let r = swap_rows(unpack_gfx(GFX.load(Ordering::Relaxed)));
	Graphics { lighting: r[0], ao: r[1] != 0, shadows: r[2] != 0, water: r[4] != 0, particles: r[5] != 0, sun_rays: r[6] != 0, smoothing: r[7] != 0, veg: r[8], culling: r[9] == 0, ao_q: r[1], shadows_q: r[2], water_q: r[4], particles_q: r[5], sun_q: r[6], gi_q: r[10], waves: r[11] == 0, draw: pcopts::draw_level(r[12]) }
}

/// The Vegetation row for a level load (viewer/vegfx.rs `plan_veg`): the first level is built before `init` reads the file.
pub fn veg_for_load() -> u8 {
	if !enabled() {
		return 0;
	}
	if ST.lock().is_ok_and(|s| s.path.is_some()) {
		return graphics().veg;
	}
	PcOptions::load(&pcopts::file()).graphics_rows()[8]
}

/// The one gate of the right-mouse-button strafe mode (modcam's aim mode and strafe.rs): the menu's verdict. With the extension
/// on it is true only while Camera is Modern and Strafe aiming is On. With `RAC_PC_EXT=0` there is no menu, the old switches
/// (`--modern-cam` / RAC_MODERN_CAM, RAC_MOVE_CLASSIC) decide as before and this answers true.
pub fn strafe_on() -> bool {
	gate(enabled(), CAM_MODERN.load(Ordering::Relaxed), STRAFE_ON.load(Ordering::Relaxed))
}

fn gate(ext: bool, camera_modern: bool, strafe: bool) -> bool {
	!ext || (camera_modern && strafe)
}

/// Registers the text hands (Retail::load, after modcam's, so a hand of the swarm on the same entry is replaced).
pub fn register() {
	if enabled() {
		register_hand(crate::retail::disc::hands(&HANDS));
		register_hand(crate::retail::disc::hands(&super::prompts::GLYPH_HANDS));
	}
}

// ---- state ----------------------------------------------------------------------------------------------------------

struct St {
	opts: PcOptions,
	path: Option<PathBuf>,
	/// The row bytes last seen in RAM (None: the region was not set up yet).
	last: Option<(u8, u8, u8)>,
	/// The Graphics row bytes last seen in RAM (`GROWS` order).
	glast: Option<[u8; 13]>,
	/// The language word last seen by the watcher.
	lang_seen: Option<u32>,
	/// A failed write was reported.
	warned: bool,
}

static ST: Mutex<St> = Mutex::new(St { opts: PcOptions::new(), path: None, last: None, glast: None, lang_seen: None, warned: false });

impl St {
	fn save(&mut self) {
		let Some(path) = &self.path else { return };
		if let Err(e) = self.opts.store(path) {
			if !self.warned {
				self.warned = true;
				eprintln!("retail: PC options: cannot write {}: {e}", path.display());
			}
		}
	}
}

/// Retail::load, before the machine exists: reads the options file, applies the camera style to modcam and, on a cold boot,
/// starts the console in the remembered language (the dev override COLD_LANG wins).
pub fn init(cold: bool) {
	if !enabled() {
		// The extension is off: the options file is not read, so the bindings are the defaults (the file was read before the
		// threads started, for the prompts' sake).
		super::binds::init(None, None);
		return;
	}
	let path = pcopts::file();
	let opts = PcOptions::load(&path);
	eprintln!(
		"retail: PC options {}: camera {}, strafe aiming {}, language {}, lighting {}, ambient occlusion {}, dynamic shadows {}",
		path.display(),
		["Classic", "Static", "Smooth"][opts.camera_byte() as usize],
		if opts.strafe_on() { "On" } else { "Off" },
		opts.lang.map_or("(not chosen)", |l| l.code()),
		pcopts::LIGHTING[opts.lighting_or_default() as usize],
		pcopts::LEVELS[opts.ao_level() as usize],
		pcopts::LEVELS[opts.shadows_level() as usize]
	);
	if cold && std::env::var_os("COLD_LANG").is_none() {
		if let Some(l) = opts.lang {
			ee_oracle::cold::set_language(l.osd());
			eprintln!("retail: PC options: cold boot in the remembered language ({})", l.code());
		}
	}
	super::binds::init(opts.extra_get("keys"), opts.extra_get("pad"));
	CAM_MODERN.store(opts.camera_modern(), Ordering::Relaxed);
	STRAFE_ON.store(opts.strafe_on(), Ordering::Relaxed);
	BUYALL_ON.store(opts.buyall_on(), Ordering::Relaxed);
	HUD_SHARP.store(opts.hud_sharp(), Ordering::Relaxed);
	GLYPH_STYLE.store(opts.glyph_style(), Ordering::Relaxed);
	GFX.store(pack_gfx(swap_rows(opts.graphics_rows())), Ordering::Relaxed);
	FULLSCREEN.store(opts.fullscreen_on(), Ordering::Relaxed);
	modcam::set_active(opts.camera_modern());
	modcam::set_smooth(opts.camfeel_smooth());
	// Debug mode from the file (no notice: the player switched it on in an earlier session).
	super::devtools::set_menu(opts.debug_on(), false);
	camopts_init(&opts);
	if let Ok(mut s) = ST.lock() {
		*s = St { opts, path: Some(path), last: None, glast: None, lang_seen: None, warned: false };
	}
}

// ---- worker ---------------------------------------------------------------------------------------------------------

/// Worker, before each field of a level (not in the front end): sets up the reserved region on fresh RAM, applies F7 (flips the
/// camera byte), reads the row bytes back and applies / remembers a change, and patches the Camera Options table of the running
/// level. Returns the field's input with the F7 press taken out (it is consumed here).
pub fn pre_field(m: &mut ee_oracle::Machine, fin: &FieldIn) -> FieldIn {
	let mut out = *fin;
	if !enabled() {
		return out;
	}
	let level = m.level();
	if let Ok(mut s) = ST.lock() {
		if sync(&mut m.cpu.mem, level, fin.mc.toggle, &mut s) {
			out.mc.toggle = false;
		}
		// The Lock-on and Camera speed rows (camopts section).
		camopts_field(&mut m.cpu.mem, level, fin.buttons, &mut s);
	}
	super::pcctl::field(&mut m.cpu.mem, level, fin.buttons, &ROWS, GTAB);
	out
}

/// Worker, before each field of the front end: once the title's Options page has set the region up (`title_options`), the
/// Graphics rows changed there are applied and remembered as in a level. Until then it writes nothing (a cold boot that
/// never opens Options keeps today's RAM).
pub fn front_field(m: &mut ee_oracle::Machine) {
	if !enabled() || m.rd32(BASE) != MAGIC {
		return;
	}
	if let Ok(mut s) = ST.lock() {
		sync(&mut m.cpu.mem, u32::MAX, false, &mut s);
	}
}

/// Fresh RAM (start, savestate load, cold boot): the options file's values go in.
fn setup(mem: &mut Mem, s: &mut St) {
	if mem.rd32(BASE) == MAGIC {
		return;
	}
	mem.wr32(BASE, MAGIC);
	let (c, t, b) = (s.opts.camera_byte(), s.opts.strafe_on() as u8, s.opts.buyall_on() as u8);
	mem.wr8(CAM, c);
	mem.wr8(STRAFE, t);
	mem.wr8(BUYALL, b);
	s.last = Some((c, t, b));
	apply(s, c, t, b, false);
	let g = swap_rows(s.opts.graphics_rows());
	for (a, v) in GROWS.iter().zip(g) {
		mem.wr8(*a, v);
	}
	mem.wr8(GLYPHS, s.opts.glyph_style());
	s.glast = Some(g);
	apply_graphics(s, g, false);
	camopts_setup(mem, s);
}

/// `pre_field` on the memory: true when the F7 press was consumed.
fn sync(mem: &mut Mem, level: u32, toggle: bool, s: &mut St) -> bool {
	setup(mem, s);
	if toggle {
		// Classic <-> the remembered Modern feel (Static or Smooth).
		let c = mem.rd8(CAM);
		let to = if c != 0 { 0 } else if s.opts.camfeel_smooth() { 2 } else { 1 };
		mem.wr8(CAM, to);
		eprintln!("retail: F7: camera {}", ["Classic", "Static", "Smooth"][to as usize]);
	}
	let (c, t, b) = (mem.rd8(CAM).min(2), mem.rd8(STRAFE), mem.rd8(BUYALL));
	if s.last != Some((c, t, b)) {
		s.last = Some((c, t, b));
		apply(s, c, t, b, true);
	}
	let mut g = GROWS.map(|a| mem.rd8(a));
	g[0] = g[0].min(1);
	// Ambient occlusion, shadows, water, particles, sun rays and global illumination have four values, vegetation and draw
	// distance three: the pause page cycles a row (+1 mod values), the title's value item only flips the byte; any change is one
	// press, so the row steps on from its last value on both pages.
	for i in [1, 2, 4, 5, 6, 8, 10, 12] {
		let n = row_values(i);
		if let Some(last) = s.glast.filter(|l| l[i] != g[i]) {
			g[i] = (last[i] + 1) % n;
			mem.wr8(GROWS[i], g[i]);
		}
		g[i] = g[i].min(n - 1);
	}
	if s.glast != Some(g) {
		s.glast = Some(g);
		apply_graphics(s, g, true);
	}
	patch_rows(mem, level);
	patch_graphics(mem, level);
	// The Controls page (pcctl.rs): the rows the player pressed, and a changed binding goes into the file.
	super::pcctl::sync(mem);
	if let Some(sharp) = super::pcctl::hud_sharp(mem) {
		if HUD_SHARP.swap(sharp, Ordering::Relaxed) != sharp {
			eprintln!("retail: PC options: HUD {}", if sharp { "Sharp" } else { "Original" });
			s.opts.hud = Some(sharp);
			s.save();
		}
	}
	// The Button glyphs row (Graphics -> Screen, and the title screen's Graphics page): the prompts name a gamepad's buttons the
	// way the row says. It has three values: the pause page cycles a row (+1 mod values), the title's value item only flips the
	// byte; any change is one press, so the row steps on from its last value on both pages.
	let style = mem.rd8(GLYPHS);
	if style != GLYPH_STYLE.load(Ordering::Relaxed) {
		let stepped = (GLYPH_STYLE.load(Ordering::Relaxed) + 1) % 3;
		mem.wr8(GLYPHS, stepped);
		GLYPH_STYLE.store(stepped, Ordering::Relaxed);
		eprintln!("retail: PC options: Button glyphs {}", pcopts::GLYPHS[stepped as usize]);
		s.opts.glyphs = Some(stepped);
		s.save();
	}
	// Debug mode (Controls -> Debug): the row's byte opens / closes the dev tools and debug keys at once and is remembered (`debug=on`).
	if let Some(on) = super::pcctl::debug_on(mem) {
		if super::devtools::set_menu(on, true) {
			eprintln!("retail: PC options: Debug mode {}", if on { "On" } else { "Off" });
			s.opts.debug = on.then_some(true);
			s.save();
		}
	}
	if let Some((keys, pad)) = super::binds::take_changes() {
		eprintln!("retail: PC options: controls keys {} pad {}", keys.as_deref().unwrap_or("(default)"), pad.as_deref().unwrap_or("(default)"));
		s.opts.extra_set("keys", keys);
		s.opts.extra_set("pad", pad);
		s.save();
	}
	toggle
}

/// A Graphics byte changed (or the region was set up): the renderer follows (`graphics`); a change is remembered.
fn apply_graphics(s: &mut St, g: [u8; 13], changed: bool) {
	GFX.store(pack_gfx(g), Ordering::Relaxed);
	FULLSCREEN.store(g[3] == 1, Ordering::Relaxed);
	if changed {
		let on = |v: u8| if v == 1 { "On" } else { "Off" };
		let lv = |v: u8| pcopts::LEVELS[swap_level(v) as usize];
		let lvc = |v: u8| pcopts::LEVELS_CLASSIC[swap_level(v) as usize];
		eprintln!(
			"retail: PC options: lighting {}, ambient occlusion {}, dynamic shadows {}, display {}, water {}, particles {}, sun rays {}, frame smoothing {}, vegetation {}, culling {}, global illumination {}, waves {}, draw distance {}",
			pcopts::LIGHTING[g[0] as usize],
			lv(g[1]),
			lv(g[2]),
			if g[3] == 1 { "Fullscreen" } else { "Windowed" },
			lvc(g[4]),
			lvc(g[5]),
			lv(g[6]),
			on(g[7]),
			["Off", "Moderate", "Lush"][g[8].min(2) as usize],
			if g[9] == 1 { "Off" } else { "On" },
			lv(g[10]),
			if g[11] == 1 { "Off" } else { "On" },
			pcopts::DRAW[pcopts::draw_level(g[12]) as usize]
		);
		s.opts.set_graphics_rows(swap_rows(g));
		s.save();
	}
}

/// A row byte changed (or the region was set up): the camera hands and the strafe gate follow; a change is remembered.
fn apply(s: &mut St, cam: u8, strafe: u8, buyall: u8, changed: bool) {
	// The Camera row: 0 Classic, 1 Static, 2 Smooth. Static and Smooth are both the Modern camera (every other gate asks only
	// "not Classic"); the feel is modcam's dynamic layer.
	let cam = cam.min(2);
	let (modern, strafe, buyall) = (cam != 0, strafe == 1, buyall == 1);
	BUYALL_ON.store(buyall, Ordering::Relaxed);
	CAM_MODERN.store(modern, Ordering::Relaxed);
	STRAFE_ON.store(strafe, Ordering::Relaxed);
	modcam::set_active(modern);
	modcam::set_smooth(cam != 1);
	if changed {
		eprintln!("retail: PC options: camera {}, strafe aiming {}", ["Classic", "Static", "Smooth"][cam as usize], if strafe { "On" } else { "Off" });
		s.opts.set_camera_byte(cam);
		s.opts.strafe = Some(strafe);
		s.opts.buyall = Some(buyall);
		s.save();
	}
}

/// The Camera Options table of `level`, when the retail one is there: builds the seven-row copy and repoints the item to it.
/// Idempotent: a second look finds the copy's address, not the table's. Guards: the level has a row, the item points at the
/// table and the table starts with the retail signature, so another overlay (loading, other level) is never touched.
fn patch_rows(mem: &mut Mem, level: u32) -> bool {
	let Some(&t) = (*crate::retail::disc::pick(&TABLES, &TABLES_USA)).get(level as usize) else { return false };
	patch_rows_at(mem, t)
}

/// `patch_rows` for the retail table at `t` (a `TABLES` / `TABLES_USA` entry).
fn patch_rows_at(mem: &mut Mem, t: u32) -> bool {
	if mem.rd32(t + PTR_OFF) != t || mem.rd32(t) != SIG[0] || mem.rd32(t + 4) != SIG[1] {
		return false;
	}
	// The retail records Left / Right and Up / Down, then the Camera speed row (the retail Slow / Medium / Fast record is replaced
	// by the percent row, camopts section), Camera, Strafe, Buy all and Lock-on, then the zero record: 7 rows + 1 = the 8 records
	// that fit between `TAB` and `POOL`.
	let mut w: Vec<u32> = (0..12).map(|i| mem.rd32(t + 4 * i)).collect();
	w.extend([mem.rd32(t + 48), CAMSEL, ID_PCT, ID_PCT, 0, 0]);
	w.extend([ID_CAMERA, CAM, ID_CLASSIC, ID_STATIC, ID_SMOOTH, 0]);
	w.extend([ID_STRAFE, STRAFE, ID_OFF, ID_ON, 0, 0]);
	w.extend([ID_BUYALL, BUYALL, ID_OFF, ID_ON, 0, 0]);
	w.extend([ID_LOCKON, LOCKON, ID_AUTO, ID_ON, ID_OFF, 0]);
	w.extend([0; 6]);
	for (i, v) in w.iter().enumerate() {
		mem.wr32(TAB + 4 * i as u32, *v);
	}
	mem.wr32(t + PTR_OFF, TAB);
	true
}

/// Whether `a` holds the retail Options list records: the seven rows (`OPTIONS_ROWS`), then the zero record.
fn is_options_list(mem: &mut Mem, a: u32) -> bool {
	(0x10_0000..0x200_0000).contains(&a) && OPTIONS_ROWS.iter().enumerate().all(|(i, &id)| mem.rd32(a + 12 * i as u32) & 0xffff == id) && mem.rd32(a + 12 * 7) == 0
}

/// The Graphics page of `level` (module doc): when the Options list of the running level still reads its retail records, the
/// Graphics row, its description and picture, the page, its header and value items and the row table are (re)built in reserved
/// RAM and the three Options pointers repointed, last, so the game never sees half a page. Every address is found through the
/// Camera Options page of `TABLES` and checked against the retail signatures (Camera Options page id 0x1d whose parent is the
/// Options page id 8; the list, description and picture items by their data), so another overlay is never touched. Idempotent:
/// a patched list points at `OPT_RECS`. Overlay data is initialised by every level load, so the poll patches again after one.
fn patch_graphics(mem: &mut Mem, level: u32) -> bool {
	let Some(&t) = (*crate::retail::disc::pick(&TABLES, &TABLES_USA)).get(level as usize) else { return false };
	let (cp, val) = (t - 0xe0, t + 0x60);
	if mem.rd32(t) != SIG[0] || mem.rd32(t + 4) != SIG[1] || mem.rd32(cp + 0x3c) != 0x1d {
		return false;
	}
	let opt = mem.rd32(cp + 0x38);
	if !(0x10_0000..0x200_0000).contains(&opt) || mem.rd32(opt + 0x3c) != 8 {
		return false;
	}
	let items: Vec<u32> = (0..14).map(|k| mem.rd32(opt + 0x44 + 4 * k)).take_while(|&a| (0x10_0000..0x200_0000).contains(&a)).collect();
	let Some(list) = items.iter().copied().find(|&it| {
		let a = mem.rd32(it + 0x34);
		is_options_list(mem, a)
	}) else {
		return false;
	};
	let text = items.iter().copied().find(|&it| {
		let d = mem.rd32(it + 0x34);
		mem.rd32(it + 0x30) == 0x90 && mem.rd32(d + 4 * GRAPHICS_ROW as u32) == 0x4f39
	});
	let pic = items.iter().copied().find(|&it| mem.rd32(it + 0x34) == 0x20 && mem.rd32(it + 0x38) == 0x93 && mem.rd32(it + 0x3c) == 0x85);
	let cam_items: Vec<u32> = (0..14).map(|k| mem.rd32(cp + 0x44 + 4 * k)).take_while(|&a| (0x10_0000..0x200_0000).contains(&a)).collect();
	let hdr = cam_items.into_iter().find(|&it| mem.rd32(it + 0x34) == ID_CAMERA_OPTIONS);
	let (Some(text), Some(pic), Some(hdr)) = (text, pic, hdr) else { return false };
	// The pictures: the seven Options ones and, for Graphics, help_ss 5 (the ToC's help_ss records end where options_ss start).
	let src = mem.rd32(pic + 0x30);
	let mut pics: Vec<[u32; 2]> = (0..7).map(|i| [mem.rd32(src.wrapping_add(8 * i)), mem.rd32(src.wrapping_add(8 * i + 4))]).collect();
	let planet = [mem.rd32(src.wrapping_sub(0x10)), mem.rd32(src.wrapping_sub(0xc))];
	if !(0x10_0000..0x200_0000).contains(&src) || pics.iter().chain([&planet]).any(|p| p[1] == 0) {
		return false;
	}
	pics.insert(GRAPHICS_ROW, planet);
	// Controls (pcctl.rs) follows Graphics and wears the Camera row's picture.
	pics.insert(GRAPHICS_ROW + 1, pics[4]);
	// The list rows and their descriptions with Graphics before Quit Game.
	let (recs, descs) = (mem.rd32(list + 0x34), mem.rd32(text + 0x34));
	let mut rows: Vec<[u32; 3]> = (0..7).map(|i| [mem.rd32(recs + 12 * i), mem.rd32(recs + 12 * i + 4), mem.rd32(recs + 12 * i + 8)]).collect();
	rows.insert(GRAPHICS_ROW, [ID_GRAPHICS | 3 << 16, GPAGE, 0]);
	rows.insert(GRAPHICS_ROW + 1, [super::pcctl::ID_CONTROLS | 3 << 16, super::pcctl::PAGE_AT, 0]);
	rows.push([0; 3]);
	let mut desc: Vec<u32> = (0..7).map(|i| mem.rd32(descs + 4 * i)).collect();
	desc.insert(GRAPHICS_ROW, ID_GRAPHICS_DESC);
	desc.insert(GRAPHICS_ROW + 1, super::pcctl::ID_DESC);
	desc.push(0);
	for (i, r) in rows.iter().enumerate() {
		for (k, w) in r.iter().enumerate() {
			mem.wr32(OPT_RECS + 12 * i as u32 + 4 * k as u32, *w);
		}
	}
	for (i, d) in desc.iter().enumerate() {
		mem.wr32(OPT_DESC + 4 * i as u32, *d);
	}
	for (i, p) in pics.iter().enumerate() {
		mem.wr32(OPT_PICS + 8 * i as u32, p[0]);
		mem.wr32(OPT_PICS + 8 * i as u32 + 4, p[1]);
	}
	// The page: Camera Options' own (14 panel sequences, parent = Options, id 0x1d, items) with our header and value items.
	let copy = |mem: &mut Mem, from: u32, to: u32, len: u32| {
		for o in (0..len).step_by(4) {
			let w = mem.rd32(from + o);
			mem.wr32(to + o, w);
		}
	};
	copy(mem, cp, GPAGE, 0x60);
	for o in (0x60..0x100).step_by(4) {
		mem.wr32(GPAGE + o, 0);
	}
	// The value panel (slot 3) plays the Sound page's slot-3 sequence: its rect (53,127,406,164) holds the 13 rows at a wider
	// pitch than Camera Options' (69,176,381,91), since the item spaces rows at height / (rows + 1) (title_assets.md
	// "Page rect table"). The Sound page is the Options row with text 0x4f22 (type 3, arg = page, parent = Options).
	let mut sound_seq = None;
	if let Some(sound) = rows.iter().find(|r| r[0] == 0x4f22 | 3 << 16).map(|r| r[1]).filter(|&p| (0x10_0000..0x200_0000).contains(&p) && mem.rd32(p + 0x38) == opt) {
		let seq = mem.rd32(sound + 0xc);
		mem.wr32(GPAGE + 0xc, seq);
		// Camera Options now has 7 rows (Camera speed, Lock-on): its own value panel (69,176,381,91) is too short for them (rows
		// 11 px apart overlap), so its value panel plays the Sound page's taller one too (rows 20 px apart).
		mem.wr32(cp + 0xc, seq);
		sound_seq = Some(seq);
	}
	for o in (0x40..0x60).step_by(4) {
		match mem.rd32(GPAGE + o) {
			a if a == hdr => mem.wr32(GPAGE + o, GHDR),
			a if a == val => mem.wr32(GPAGE + o, GVAL),
			_ => {}
		}
	}
	copy(mem, hdr, GHDR, 0x58);
	mem.wr32(GHDR + 0x34, ID_GRAPHICS_HDR);
	copy(mem, val, GVAL, 0x40);
	mem.wr32(GVAL + 0x34, GTAB);
	mem.wr32(GVAL + 0x38, 0);
	mem.wr32(GVAL + 0x3c, 0);
	// 6-word rows {label, byte, value 0, value 1, value 2 or 0, value 3 or 0} and the zero record.
	super::pcctl::build_gfx(mem, GTAB, &ROWS, HUD_SHARP.load(Ordering::Relaxed));
	// The Controls page, from the same Camera Options items.
	super::pcctl::build_page(mem, cp, hdr, val, sound_seq);
	mem.wr32(text + 0x34, OPT_DESC);
	mem.wr32(pic + 0x30, OPT_PICS);
	mem.wr32(list + 0x34, OPT_RECS);
	true
}

fn copy_words(mem: &mut Mem, from: u32, to: u32, len: u32) {
	for o in (0..len).step_by(4) {
		let w = mem.rd32(from + o);
		mem.wr32(to + o, w);
	}
}

/// The front end's Options list set-up (11 instructions, boot ELF 0x219ca8 on the PAL title, also in frontbin and on the USA
/// disc): `item+0x34` = the PAL rows Language, Video, Sound, or the NTSC rows Language, Sound, by the region word 0x15ee90.
/// The words that hold addresses (the region word, the two lists) are masked.
const TITLE_CODE: [(u32, u32); 11] = [
	(0x3c02_0000, 0xffff_0000), // lui v0, hi(region)
	(0x8c42_0000, 0xffff_0000), // lw v0, lo(region)(v0)
	(0x5040_0004, !0),          // beql v0, zero, +4
	(0x3c02_0000, 0xffff_0000), // lui v0, hi(lists)
	(0x3c02_0000, 0xffff_0000), // lui v0, hi(lists)
	(0x1000_0002, !0),          // b +2
	(0x2442_0000, 0xffff_0000), // addiu v0, v0, lo(PAL list)
	(0x2442_0000, 0xffff_0000), // addiu v0, v0, lo(NTSC list)
	(0xac82_0034, !0),          // sw v0, 0x34(a0)
	(0x03e0_0008, !0),          // jr ra
	(0x0000_102d, !0),          // move v0, zero
];

/// The front end runs interpreted (no recompiled code: title_boot.rs `hook_front_movies`), so the Options hook is an oracle host
/// stub on every copy of `TITLE_CODE` in the code range, applied only while RAM still holds that function. Called when the
/// front end starts (cold boot title in the boot ELF, frontbin after Quit Game).
pub fn hook_front(m: &mut ee_oracle::Machine) {
	if !enabled() {
		return;
	}
	let ram = &m.cpu.mem.ram;
	let rd = |a: usize| u32::from_le_bytes(ram[a..a + 4].try_into().unwrap());
	let found: Vec<u32> = (0x10_0000..0x40_0000 - 4 * TITLE_CODE.len()).step_by(4).filter(|&a| TITLE_CODE.iter().enumerate().all(|(k, &(v, mask))| rd(a + 4 * k) & mask == v)).map(|a| a as u32).collect();
	for a in found {
		let sig = [0, 1, 2, 3].map(|k| m.rd32(a + 4 * k));
		m.cpu.add_host_stub(a, "title Options list (pcext)", sig, title_options);
		eprintln!("retail: PC options: title Options hook at {a:#x}");
	}
}

/// The title's Options page (host stub on its list set-up, `hook_front`): the retail rows of the disc's region and a Graphics row
/// that opens a copy of the Video Options page reading the Graphics rows (`ROWS`: the bytes of the pause menu's Graphics
/// page), all in reserved RAM. The region is set up first (`setup`), so the rows show the remembered values. The instruction
/// count advances as retail's would (9 instructions with the PAL rows, 8 with the NTSC ones).
fn title_options(c: &mut Cpu) -> bool {
	let item = c.g(4) as u32;
	let Ok(mut s) = ST.lock() else { return false };
	let Some(n) = build_title(&mut c.mem, c.pc, item, &mut s) else { return false };
	c.s(2, 0);
	c.icount += n;
	c.mem.now = c.icount;
	true
}

/// `title_options` on the memory; Some(the instructions retail runs). Every address comes from the function's own code at `pc`
/// (the region word, both row lists) and the Video page the PAL list names, checked against the retail texts, so the same body
/// runs on both discs. None when a check fails: the retail function then runs (no Graphics row). The page's slots, items and
/// row table are rebuilt on every entry to Options, the list records only when the rows changed (their third word is the
/// menu's own).
fn build_title(mem: &mut Mem, pc: u32, item: u32, s: &mut St) -> Option<u64> {
	let ok = |a: u32| (0x10_0000..0x200_0000).contains(&a);
	let imm = |w: u32| w as u16 as i16 as i32 as u32;
	// lui v0,hi; lw v0,lo(v0) (the region word); ...; lui v0,hi; ...; addiu v0,v0,pal; addiu v0,v0,ntsc.
	let (w0, w1, w3, w6, w7) = (mem.rd32(pc), mem.rd32(pc + 4), mem.rd32(pc + 0xc), mem.rd32(pc + 0x18), mem.rd32(pc + 0x1c));
	let region = ((w0 & 0xffff) << 16).wrapping_add(imm(w1));
	let (pal, ntsc) = (((w3 & 0xffff) << 16).wrapping_add(imm(w6)), ((w3 & 0xffff) << 16).wrapping_add(imm(w7)));
	if !ok(item) || !ok(region) || !ok(pal) || !ok(ntsc) {
		return None;
	}
	let rows = |mem: &mut Mem, a: u32| -> Vec<[u32; 3]> { (0..5).map(|i| [mem.rd32(a + 12 * i), mem.rd32(a + 12 * i + 4), mem.rd32(a + 12 * i + 8)]).take_while(|r| r[0] != 0).collect() };
	let page = |rs: &[[u32; 3]], id: u32| rs.iter().find(|r| r[0] == id | 3 << 16).map(|r| r[1]).filter(|&p| ok(p));
	let is_pal = mem.rd32(region) != 0;
	let shown = if is_pal { pal } else { ntsc };
	let mut recs = rows(mem, shown);
	let (Some(video), Some(sound)) = (page(&rows(mem, pal), FRONT_VIDEO), page(&recs, FRONT_SOUND)) else { return None };
	if recs.len() > 3 || recs.iter().any(|r| r[0] >> 16 != 3) || mem.rd32(video + 0x38) != mem.rd32(sound + 0x38) {
		return None;
	}
	let items: Vec<u32> = (0..14).map(|k| mem.rd32(video + 0x44 + 4 * k)).filter(|&a| ok(a)).collect();
	let hdr = items.iter().copied().find(|&it| mem.rd32(it + 0x34) == FRONT_VIDEO_HDR);
	let val = items.iter().copied().find(|&it| {
		let t = mem.rd32(it + 0x34);
		ok(t) && mem.rd32(t) == FRONT_VIDEO_ROW
	});
	let (Some(hdr), Some(val)) = (hdr, val) else { return None };
	setup(mem, s);
	recs.push([ID_GRAPHICS | 3 << 16, TGPAGE, 0]);
	let same = recs.iter().enumerate().all(|(i, r)| mem.rd32(TRECS + 12 * i as u32) == r[0] && mem.rd32(TRECS + 12 * i as u32 + 4) == r[1]) && mem.rd32(TRECS + 12 * recs.len() as u32) == 0;
	if !same {
		for (i, r) in recs.iter().chain([&[0u32; 3]]).enumerate() {
			for (k, w) in r.iter().enumerate() {
				mem.wr32(TRECS + 12 * i as u32 + 4 * k as u32, *w);
			}
		}
	}
	// The page: Video Options' own (slot sequences, parent = Options, id, items) with our header and value items. The value
	// panel (slot 3) plays the Sound page's slot-3 sequence: its rect (47,125,419,164) holds the 13 rows, Video's holds one.
	copy_words(mem, video, TGPAGE, 0x88);
	for o in (0x88..0x100).step_by(4) {
		mem.wr32(TGPAGE + o, 0);
	}
	let seq = mem.rd32(sound + 0xc);
	mem.wr32(TGPAGE + 0xc, seq);
	for o in (0x40..0x80).step_by(4) {
		match mem.rd32(TGPAGE + o) {
			a if a == hdr => mem.wr32(TGPAGE + o, TGHDR),
			a if a == val => mem.wr32(TGPAGE + o, TGVAL),
			_ => {}
		}
	}
	copy_words(mem, hdr, TGHDR, 0x58);
	mem.wr32(TGHDR + 0x34, ID_GRAPHICS_HDR);
	copy_words(mem, val, TGVAL, 0x40);
	mem.wr32(TGVAL + 0x34, TGTAB);
	mem.wr32(TGVAL + 0x38, 0);
	mem.wr32(TGVAL + 0x3c, 0);
	// 5-word rows {label, byte, value 0, value 1, flags 0 (Cross flips the byte; Video's 1 = the keep-mode prompt)}, the zero record.
	// The Button glyphs row is ours (three values; the record holds the first two, the mirror draws the third).
	let tab: Vec<u32> = ROWS
		.iter()
		.flat_map(|r| [r[0], r[1], r[2], r[3], 0])
		.chain([crate::retail::pcctl::ID_GLYPHS, GLYPHS, crate::retail::pcctl::ID_PLAYSTATION, crate::retail::pcctl::ID_XBOX, 0])
		.chain([0; 5])
		.collect();
	for (i, w) in tab.iter().enumerate() {
		mem.wr32(TGTAB + 4 * i as u32, *w);
	}
	mem.wr32(item + 0x34, TRECS);
	Some(if is_pal { 9 } else { 8 })
}

/// The title's Graphics page as the mirror draws it (viewer/front_mirror.rs): each row's label and value text ids, for the
/// values last applied.
pub fn title_rows() -> Vec<(u32, u32)> {
	let mut v = unpack_gfx(GFX.load(Ordering::Relaxed));
	v[3] = FULLSCREEN.load(Ordering::Relaxed) as u8;
	let mut rows: Vec<(u32, u32)> = ROWS.iter().zip(v).map(|(r, v)| (r[0], r[2 + v as usize])).collect();
	// The Button glyphs row: the style the prompts use.
	let style = GLYPH_STYLE.load(Ordering::Relaxed).min(2) as usize;
	rows.push((crate::retail::pcctl::ID_GLYPHS, [crate::retail::pcctl::ID_PLAYSTATION, crate::retail::pcctl::ID_XBOX, crate::retail::pcctl::ID_SWITCH][style]));
	rows
}

/// The new texts in `lang` (0 En .. 4 It) as the mirror's strings (font bytes as chars, like the disc's texts).
pub fn texts(lang: usize) -> impl Iterator<Item = (u32, String)> {
	NEW_IDS.iter().enumerate().map(move |(k, &id)| (id, TEXTS[k][lang.min(4)].iter().map(|&b| b as char).collect()))
}

/// The text lookup hand: the three new ids get their string for the game's language; every other id goes to `prompts::lookup`
/// (keyboard prompts; it declines with a gamepad, so retail runs).
fn text_hook(c: &mut Cpu) -> bool {
	let id = c.g(4) as u32;
	// The Lock-on / Camera speed rows' texts (camopts section below): own pool, one slot per id.
	if (ID_LOCKON..=ID_SMOOTH).contains(&id) {
		return camopt_text(c, id);
	}
	// The Controls page's texts (pcctl.rs): they follow the bindings at every look; the Debug section's ids have a pool of their own.
	if (0x7f40..super::pcctl::ID_LAST).contains(&id) || (super::pcctl::ID_DBG0..super::pcctl::ID_DBG_END).contains(&id) {
		let lang = Lang::from_word(c.mem.rd32(LANG_WORD)).map_or(0, |l| l as usize);
		return super::pcctl::text_hook(c, id, lang);
	}
	// Not one of ours: the keyboard's button prompts (prompts.rs) or retail's own lookup.
	let Some(k) = NEW_IDS.iter().position(|&n| n == id) else { return super::prompts::lookup(c) };
	let lang = Lang::from_word(c.mem.rd32(LANG_WORD)).map_or(0, |l| l as usize);
	let at = pool_slot(k);
	let s = text_for(k, lang);
	// Keyboard: key names for the glyphs (slot is 0x40 bytes with the terminator).
	let s = super::prompts::for_device(&s).filter(|t| t.len() < 0x40).unwrap_or(s);
	for (i, b) in s.iter().enumerate() {
		c.mem.wr8(at + i as u32, *b);
	}
	c.mem.wr8(at + s.len() as u32, 0);
	c.s(2, at as u64);
	true
}

/// The string pool slot of new text `k` (`NEW_IDS` order): `POOL`, then `POOL2`.
fn pool_slot(k: usize) -> u32 {
	match k < POOL_SLOTS {
		true => POOL + 0x40 * k as u32,
		false => POOL2 + 0x40 * (k - POOL_SLOTS) as u32,
	}
}

fn text_for(k: usize, lang: usize) -> Vec<u8> {
	TEXTS[k][lang].to_vec()
}

/// Cold boots only: remembers Options -> Language. Called each field while the front end runs: the game sets its language word
/// from the console language during the first fields, so the baseline is taken from field 4; a later change is the
/// player's choice.
pub fn watch_language(m: &ee_oracle::Machine, field: u32, in_front: bool) {
	if !enabled() || field < 4 || !in_front {
		return;
	}
	let w = m.rd32(LANG_WORD);
	let Ok(mut s) = ST.lock() else { return };
	match s.lang_seen {
		None => s.lang_seen = Some(w),
		Some(p) if p != w => {
			s.lang_seen = Some(w);
			if let Some(l) = Lang::from_word(w) {
				eprintln!("retail: PC options: language {} remembered (field {field})", l.code());
				s.opts.lang = Some(l);
				s.save();
			}
		}
		_ => {}
	}
}

// ---- Camera Options: Lock-on and Camera speed rows ---------------------------------------------------------------------------
//
// Two rows of the Camera Options page (see `patch_rows_at`; the page then has 7 rows):
//
// - Camera speed (row 2, replaces retail's Slow / Medium / Fast row): a value row in percent, 10..200 in steps of 10, shown as
//   `< 50 >` in retail's font (no percent sign: the font's code 0x25 is the Cross button glyph). Retail has no slider item the page could reuse (the only bars are the Sound page's own item,
//   per-level code), so it is a multi-choice row whose two choices are the same dynamic text (`ID_PCT`): retail's update toggles
//   the row's byte `CAMSEL` on Cross, which the host reads as "+10" (200 wraps to 10); Left / Right (pad bits of the field, also
//   the mouse wheel) are read by the host while this row is the page's cursor row: -10 / +10 with the menu's auto-repeat.
//   50 % is retail's Fast exactly. The percent is the player's own key `camspeed` (pcopts.rs); without the key nothing is
//   written: retail's row decides (Slow 30 / Medium 40 / Fast 50 is only what the row shows). With the key, all three entries of
//   retail's follow-camera rate table (1.0 / 1.3 / 1.6 degrees a tick, found by value in the overlay's data, per level) are
//   written to Fast * percent / 50 every field and retail's own byte 0x15ede4 gets the nearest preset (so the save stays valid);
//   the Modern camera (modcam.rs) scales its mouse and stick turn by percent / 50.
// - Lock-on (row 6): Auto / On / Off for the strafe aim's lock-on (aim.rs `blaster_search_any`: with it off the green crosshair
//   stays centred and the shot flies to the aim point). Auto = on with a pad, off with keyboard and mouse (`prompts::keyboard_now`).
//
// Reserved RAM: the row bytes `LOCKON` / `CAMSEL`, the three text slots from `CO_POOL` (BASE + 0x1000..0x1800 is this section's,
// prompts.rs' canary skips it) and the texts' ids 0x7f80..0x7f82.

/// Lock-on row: 0 Auto, 1 On, 2 Off (the retail update cycles it). Zero RAM is Auto.
pub const LOCKON: u32 = BASE + 0x38;
/// The camera speed row's byte: retail toggles it 0 <-> 1 on every Cross; the host reads each change as one +10 step.
const CAMSEL: u32 = BASE + 0x39;
pub const ID_LOCKON: u32 = 0x7f80;
pub const ID_AUTO: u32 = 0x7f81;
/// The camera speed row's value text, "< 50 >" (built from the current percent on every lookup).
pub const ID_PCT: u32 = 0x7f82;
/// The Camera row's two feels (Static = the plain Modern camera, Smooth = the dynamic one; the third value is `ID_CLASSIC`).
pub const ID_STATIC: u32 = 0x7f83;
pub const ID_SMOOTH: u32 = 0x7f84;
/// This section's string pool: one 0x40-byte slot per id (`ID_LOCKON`..=`ID_SMOOTH`); the range is skipped by prompts.rs' canary.
const CO_POOL: u32 = BASE + 0x1000;
pub const CO_POOL_END: u32 = BASE + 0x1900;
/// Retail's camera speed byte (save struct): 0 Slow, 1 Medium, 2 Fast.
const RETAIL_SPEED: u32 = 0x15ede4;
/// The page-menu mode word (mouse_menu::MODE while a pause page is open) and the row index of the camera speed row.
const MENU_MODE: u32 = 0x15f5a4;
const SPEED_ROW: u32 = 2;
/// The Camera Options page id (the Graphics page is a copy with the same id: told apart by the page address).
const CAMERA_PAGE: u32 = 0x1d;
/// Retail's follow-camera turn rates (radians a tick: 1.0, 1.3, 1.6 degrees) indexed by `RETAIL_SPEED`; the Fast bits are 50 %.
const RATE_RETAIL: [u32; 3] = [0x3c8e_fa35, 0x3cb9_dede, 0x3ce4_c388];
/// Where the overlay's data holds that table on every level of both discs (PAL 0x161d70..0x1626e8).
const RATE_SCAN: std::ops::Range<u32> = 0x15_ef00..0x17_0000;

/// Lock-on texts per language (En, Fr, De, Es, It) and the Auto value.
#[rustfmt::skip]
static CO_TEXTS: [[&[u8]; 5]; 4] = [
	[b"Lock-on", b"Verrouillage", b"Zielerfassung", b"Fijar objetivo", b"Aggancio"],
	[b"Auto", b"Auto", b"Auto", b"Auto", b"Auto"],
	// The Camera row's feels (Es: a-acute 0x84). [I] wording, native review wanted.
	[b"Static", b"Statique", b"Statisch", b"Est\x84tica", b"Statica"],
	[b"Smooth", b"Fluide", b"Weich", b"Suave", b"Fluida"],
];

/// The Lock-on row as last applied (0 Auto, 1 On, 2 Off).
static LOCKON_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// The player's camera speed in percent (`camspeed` key); 0 = never set, retail's row decides and nothing is written.
static CAMSPEED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// The percent the row shows now (the key, else retail's byte as 30 / 40 / 50).
static CAMSHOW: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(50);
/// `CAMSEL` as last seen (0xff: not known yet, the next look is the baseline).
static CAMSEL_SEEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0xff);
/// Where the rate table was found (0: not yet) and the bits the host last wrote into it (0: nothing).
static RATE_AT: AtomicU32 = AtomicU32::new(0);
static RATE_SET: AtomicU32 = AtomicU32::new(0);
/// Fields Left / Right have been held (auto-repeat).
static HELD: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];

/// The Lock-on decision of the strafe aim (aim.rs): On, Off, or Auto = on while the last real input came from a pad.
pub fn lockon_on() -> bool {
	lockon_decide(LOCKON_MODE.load(Ordering::Relaxed), super::prompts::keyboard_now())
}

fn lockon_decide(mode: u8, keyboard: bool) -> bool {
	match mode {
		1 => true,
		2 => false,
		_ => !keyboard,
	}
}

/// The Modern camera's turn scale (modcam.rs `step`): the camera speed row's percent / 50, exactly 1.0 without the key.
pub fn camspeed_scale() -> f32 {
	camspeed_factor(CAMSPEED.load(Ordering::Relaxed))
}

fn camspeed_factor(pct: u8) -> f32 {
	if pct == 0 {
		1.0
	} else {
		pct as f32 / 50.0
	}
}

/// The turn rate (radians a tick) for `pct` percent: Fast * pct / 50, so 50 is retail's Fast bits exactly.
pub fn camspeed_rate_bits(pct: u8) -> u32 {
	(f32::from_bits(RATE_RETAIL[2]) * (pct as f32 / 50.0)).to_bits()
}

/// What retail's byte shows as a percent (Slow, Medium, Fast).
fn preset_pct(byte: u8) -> u8 {
	[30, 40, 50][byte.min(2) as usize]
}

/// Retail's byte for the nearest preset of `pct` (the save keeps a valid Slow / Medium / Fast).
fn preset_byte(pct: u8) -> u8 {
	match pct {
		..=35 => 0,
		36..=45 => 1,
		_ => 2,
	}
}

/// A button of the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
	Left,
	Right,
	Cross,
}

/// The row's value after one `step` from `cur`: Left -10 and Right +10 stop at the ends, Cross +10 wraps from 200 to 10.
fn camspeed_step(cur: u8, step: Step) -> u8 {
	use pcopts::{CAMSPEED_MAX as MAX, CAMSPEED_MIN as MIN, CAMSPEED_STEP as STEP};
	let cur = pcopts::snap_camspeed(cur);
	match step {
		Step::Left => cur.saturating_sub(STEP).max(MIN),
		Step::Right => (cur + STEP).min(MAX),
		Step::Cross => {
			if cur >= MAX {
				MIN
			} else {
				cur + STEP
			}
		}
	}
}

/// Whether a held direction steps in this field: on the press, then after 12 fields every 4 (the menu's own auto-repeat feel).
fn repeat_due(held: u32) -> bool {
	held == 1 || (held >= 13 && (held - 13) % 4 == 0)
}

/// The row's text: "< 50 >" in retail's font (`<` and `>` are glyphs of all three fonts; a `%` is not: its code 0x25 draws the
/// Cross button, so the unit is left out).
fn pct_text(pct: u8) -> Vec<u8> {
	format!("< {pct} >").into_bytes()
}

/// The string of new text `id` in language `lang` (0 En, 1 Fr, 2 De, 3 Es, 4 It); the percent row uses `pct`.
fn camopt_string(id: u32, lang: usize, pct: u8) -> Option<Vec<u8>> {
	match id {
		ID_LOCKON => Some(CO_TEXTS[0][lang.min(4)].to_vec()),
		ID_AUTO => Some(CO_TEXTS[1][lang.min(4)].to_vec()),
		ID_PCT => Some(pct_text(pct)),
		ID_STATIC => Some(CO_TEXTS[2][lang.min(4)].to_vec()),
		ID_SMOOTH => Some(CO_TEXTS[3][lang.min(4)].to_vec()),
		_ => None,
	}
}

/// Text hook, ids `ID_LOCKON..=ID_SMOOTH`: the string goes to its pool slot and the lookup returns it.
fn camopt_text(c: &mut Cpu, id: u32) -> bool {
	let lang = Lang::from_word(c.mem.rd32(LANG_WORD)).map_or(0, |l| l as usize);
	let Some(s) = camopt_string(id, lang, CAMSHOW.load(Ordering::Relaxed)) else { return super::prompts::lookup(c) };
	let at = CO_POOL + 0x40 * (id - ID_LOCKON);
	for (i, b) in s.iter().enumerate() {
		c.mem.wr8(at + i as u32, *b);
	}
	c.mem.wr8(at + s.len() as u32, 0);
	c.s(2, at as u64);
	true
}

/// `init`: the options file's rows are known before the first field.
fn camopts_init(opts: &PcOptions) {
	LOCKON_MODE.store(opts.lockon_mode(), Ordering::Relaxed);
	CAMSPEED.store(opts.camspeed.map_or(0, pcopts::snap_camspeed), Ordering::Relaxed);
	CAMSEL_SEEN.store(0xff, Ordering::Relaxed);
	RATE_AT.store(0, Ordering::Relaxed);
	RATE_SET.store(0, Ordering::Relaxed);
	eprintln!(
		"retail: PC options: lock-on {}, camera speed {}",
		pcopts::LOCKON[opts.lockon_mode() as usize],
		opts.camspeed.map_or("(retail row)".to_string(), |p| format!("{}%", pcopts::snap_camspeed(p)))
	);
}

/// Fresh RAM (`setup`): the row bytes take the file's values; the rate table is looked for again.
fn camopts_setup(mem: &mut Mem, s: &St) {
	let l = s.opts.lockon_mode();
	mem.wr8(LOCKON, l);
	mem.wr8(CAMSEL, 0);
	LOCKON_MODE.store(l, Ordering::Relaxed);
	CAMSPEED.store(s.opts.camspeed.map_or(0, pcopts::snap_camspeed), Ordering::Relaxed);
	CAMSEL_SEEN.store(0xff, Ordering::Relaxed);
	RATE_AT.store(0, Ordering::Relaxed);
	RATE_SET.store(0, Ordering::Relaxed);
	for h in &HELD {
		h.store(0, Ordering::Relaxed);
	}
}

/// Whether the camera speed row is the cursor row of the open Camera Options page (not the Graphics copy) of `level`, idle.
fn on_speed_row(mem: &mut Mem, level: u32) -> bool {
	if mem.rd32(crate::retail::disc::va(MENU_MODE)) != super::mouse_menu::MODE {
		return false;
	}
	let Some(&t) = (*crate::retail::disc::pick(&TABLES, &TABLES_USA)).get(level as usize) else { return false };
	let (cp, item) = (t - 0xe0, t + 0x60);
	let ram = &mem.ram;
	let rd = |a: u32| -> u32 {
		let i = (a & 0x01ff_ffff) as usize;
		ram.get(i..i + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
	};
	if rd(item + 0x34) != TAB {
		return false;
	}
	match super::mouse_menu::read(&rd, level, super::mouse_menu::MODE) {
		Some(snap) => snap.page == cp && snap.id == CAMERA_PAGE && snap.state == CAMERA_PAGE && snap.cur == item && rd(item + 0x38) == SPEED_ROW,
		None => false,
	}
}

/// Finds retail's rate table (the three turn rates in a row) in the overlay's data.
fn find_rate_table(mem: &mut Mem) -> Option<u32> {
	RATE_SCAN.step_by(4).find(|&a| RATE_RETAIL.iter().enumerate().all(|(k, &w)| mem.rd32(a + 4 * k as u32) == w))
}

/// Writes `bits` into all three entries of the rate table (found on first use, looked for again when a level load put other data
/// there). Only entries that differ are written.
fn set_rate_table(mem: &mut Mem, bits: u32) -> bool {
	let applied = RATE_SET.load(Ordering::Relaxed);
	let mut at = RATE_AT.load(Ordering::Relaxed);
	let words = |mem: &mut Mem, a: u32| [mem.rd32(a), mem.rd32(a + 4), mem.rd32(a + 8)];
	if at == 0 || !(words(mem, at) == RATE_RETAIL || (applied != 0 && words(mem, at) == [applied; 3])) {
		let Some(found) = find_rate_table(mem) else {
			RATE_AT.store(0, Ordering::Relaxed);
			return false;
		};
		at = found;
		RATE_AT.store(at, Ordering::Relaxed);
	}
	for k in 0..3 {
		if mem.rd32(at + 4 * k) != bits {
			mem.wr32(at + 4 * k, bits);
		}
	}
	RATE_SET.store(bits, Ordering::Relaxed);
	true
}

/// The player set the camera speed to `pct`: remembered, retail's byte follows with the nearest preset.
fn set_camspeed(mem: &mut Mem, s: &mut St, pct: u8) {
	CAMSPEED.store(pct, Ordering::Relaxed);
	CAMSHOW.store(pct, Ordering::Relaxed);
	mem.wr8(RETAIL_SPEED, preset_byte(pct));
	s.opts.camspeed = Some(pct);
	s.save();
	eprintln!("retail: PC options: camera speed {pct}%");
}

/// Worker, each field of a level, after `sync`: the Lock-on byte, the camera speed row's steps and the rate table.
fn camopts_field(mem: &mut Mem, level: u32, buttons: u16, s: &mut St) {
	// Lock-on: the retail update cycles the byte; a change is applied and remembered.
	let l = mem.rd8(LOCKON).min(2);
	if LOCKON_MODE.swap(l, Ordering::Relaxed) != l {
		eprintln!("retail: PC options: lock-on {}", pcopts::LOCKON[l as usize]);
		s.opts.lockon = Some(l);
		s.save();
	}
	// Camera speed: the percent the row shows, then the steps (Cross: retail toggled CAMSEL; Left / Right: the pad bits).
	let key = CAMSPEED.load(Ordering::Relaxed);
	let mut shown = if key != 0 { key } else { preset_pct(mem.rd8(RETAIL_SPEED)) };
	let sel = mem.rd8(CAMSEL);
	let seen = CAMSEL_SEEN.swap(sel, Ordering::Relaxed);
	let cross = seen != 0xff && seen != sel;
	let mut dir = [false; 2];
	for (i, bit) in [super::mouse_menu::LEFT, super::mouse_menu::RIGHT].into_iter().enumerate() {
		let held = if buttons & bit != 0 { HELD[i].load(Ordering::Relaxed).saturating_add(1) } else { 0 };
		HELD[i].store(held, Ordering::Relaxed);
		dir[i] = repeat_due(held);
	}
	if (cross || dir[0] != dir[1]) && on_speed_row(mem, level) {
		let step = if cross {
			Step::Cross
		} else if dir[0] {
			Step::Left
		} else {
			Step::Right
		};
		let next = camspeed_step(shown, step);
		if next != shown {
			set_camspeed(mem, s, next);
			shown = next;
		}
	}
	CAMSHOW.store(shown, Ordering::Relaxed);
	let key = CAMSPEED.load(Ordering::Relaxed);
	if key != 0 {
		set_rate_table(mem, camspeed_rate_bits(key));
	}
}

#[cfg(test)]
mod camopts_tests {
	use super::*;

	/// The tests below set process-wide state (CAMSPEED, CAMSHOW ..): they take turns.
	static TURN: Mutex<()> = Mutex::new(());

	fn turn() -> std::sync::MutexGuard<'static, ()> {
		TURN.lock().unwrap_or_else(|e| e.into_inner())
	}

	fn reset_statics() {
		CAMSPEED.store(0, Ordering::Relaxed);
		CAMSHOW.store(50, Ordering::Relaxed);
		LOCKON_MODE.store(0, Ordering::Relaxed);
		CAMSEL_SEEN.store(0xff, Ordering::Relaxed);
		RATE_AT.store(0, Ordering::Relaxed);
		RATE_SET.store(0, Ordering::Relaxed);
	}

	/// The retail Camera Options records at the table: Left / Right, Up / Down, Slow / Medium / Fast.
	const RETAIL_RECS: [[u32; 6]; 3] = [[0x4fa2, 0x15ede0, 0x4fa5, 0x4fa4, 0, 0], [0x4fa3, 0x15eddc, 0x4fa5, 0x4fa4, 0, 0], [0x521b, 0x15ede4, 0x521a, 0x5219, 0x5218, 0]];

	fn rec(mem: &mut Mem, i: u32) -> [u32; 6] {
		let mut r = [0u32; 6];
		for (k, w) in r.iter_mut().enumerate() {
			*w = mem.rd32(TAB + 24 * i + 4 * k as u32);
		}
		r
	}

	/// The row table of every level of both discs: the retail records, the Camera speed row in place of Slow / Medium / Fast, the
	/// three PC rows, Lock-on, and the zero record; patched once; 8 records fit between TAB and POOL.
	#[test]
	fn camera_rows_of_every_level_pal_and_usa() {
		assert_eq!(POOL - TAB, 8 * 24, "7 rows and the zero record fill the copy exactly");
		for (name, tables) in [("PAL", &TABLES), ("USA", &TABLES_USA)] {
			for (lv, &t) in tables.iter().enumerate() {
				let mut mem = Mem::new();
				for (i, r) in RETAIL_RECS.iter().enumerate() {
					for (k, w) in r.iter().enumerate() {
						mem.wr32(t + 24 * i as u32 + 4 * k as u32, *w);
					}
				}
				assert!(!patch_rows_at(&mut mem, t), "{name} level {lv}: no item pointer yet");
				mem.wr32(t + PTR_OFF, t);
				assert!(patch_rows_at(&mut mem, t), "{name} level {lv}");
				assert_eq!(mem.rd32(t + PTR_OFF), TAB, "{name} level {lv}: item repointed");
				assert_eq!(rec(&mut mem, 0), RETAIL_RECS[0], "{name} level {lv}: Left / Right as retail");
				assert_eq!(rec(&mut mem, 1), RETAIL_RECS[1], "{name} level {lv}: Up / Down as retail");
				assert_eq!(rec(&mut mem, 2), [0x521b, CAMSEL, ID_PCT, ID_PCT, 0, 0], "{name} level {lv}: camera speed row (retail's label)");
				assert_eq!(rec(&mut mem, 3), [ID_CAMERA, CAM, ID_CLASSIC, ID_STATIC, ID_SMOOTH, 0], "{name} level {lv}");
				assert_eq!(rec(&mut mem, 4), [ID_STRAFE, STRAFE, ID_OFF, ID_ON, 0, 0], "{name} level {lv}");
				assert_eq!(rec(&mut mem, 5), [ID_BUYALL, BUYALL, ID_OFF, ID_ON, 0, 0], "{name} level {lv}");
				assert_eq!(rec(&mut mem, 6), [ID_LOCKON, LOCKON, ID_AUTO, ID_ON, ID_OFF, 0], "{name} level {lv}: lock-on row");
				assert_eq!(rec(&mut mem, 7), [0; 6], "{name} level {lv}: zero record");
				// The retail table itself is left alone, and a patched item is not patched again.
				assert_eq!(mem.rd32(t + 4), 0x15ede0);
				assert!(!patch_rows_at(&mut mem, t), "{name} level {lv}: idempotent");
			}
		}
		// The bytes and the text ids do not collide with the other rows or the pools.
		let rows = [CAM, STRAFE, BUYALL, LOCKON, CAMSEL].into_iter().chain(GROWS);
		let mut v: Vec<u32> = rows.collect();
		v.sort();
		v.dedup();
		assert_eq!(v.len(), 5 + GROWS.len(), "row bytes are distinct");
		assert!((ID_LOCKON..=ID_SMOOTH).all(|id| !NEW_IDS.contains(&id)));
		assert!(CO_POOL + 0x40 * (ID_SMOOTH - ID_LOCKON + 1) <= CO_POOL_END && CO_POOL_END >= POOL2 + 0x100);
	}

	#[test]
	fn camera_speed_steps_and_ends() {
		use Step::*;
		assert_eq!(camspeed_step(50, Right), 60);
		assert_eq!(camspeed_step(50, Left), 40);
		assert_eq!(camspeed_step(10, Left), 10, "Left stops at 10");
		assert_eq!(camspeed_step(200, Right), 200, "Right stops at 200");
		assert_eq!(camspeed_step(190, Right), 200);
		assert_eq!(camspeed_step(50, Cross), 60);
		assert_eq!(camspeed_step(200, Cross), 10, "Cross wraps");
		assert_eq!(camspeed_step(10, Cross), 20);
		// 20 presses of Cross visit every value once and come back.
		let mut v = 10;
		let mut seen = Vec::new();
		for _ in 0..20 {
			seen.push(v);
			v = camspeed_step(v, Cross);
		}
		assert_eq!((v, seen.len(), seen.first(), seen.last()), (10, 20, Some(&10), Some(&200)));
		// Retail's three presets show as 30 / 40 / 50 and a percent maps back to the nearest one.
		assert_eq!([0u8, 1, 2, 7].map(preset_pct), [30, 40, 50, 50]);
		assert_eq!([10u8, 30, 35, 36, 40, 45, 46, 50, 200].map(preset_byte), [0, 0, 0, 1, 1, 1, 2, 2, 2]);
		for b in 0..3u8 {
			assert_eq!(preset_byte(preset_pct(b)), b, "the shown value of a preset maps back to it");
		}
		// Auto-repeat: the press, then from the 13th field every 4th.
		let due: Vec<u32> = (0..30).filter(|&h| repeat_due(h)).collect();
		assert_eq!(due, [1, 13, 17, 21, 25, 29]);
	}

	#[test]
	fn camera_speed_rate_maths() {
		assert_eq!(camspeed_rate_bits(50), 0x3ce4_c388, "50 % is retail's Fast exactly");
		assert_eq!(f32::from_bits(camspeed_rate_bits(100)), 2.0 * f32::from_bits(0x3ce4_c388), "100 % doubles the turn per tick");
		assert_eq!(f32::from_bits(camspeed_rate_bits(10)), f32::from_bits(0x3ce4_c388) * (10.0 / 50.0));
		let rates: Vec<f32> = (1..=20).map(|k| f32::from_bits(camspeed_rate_bits(10 * k))).collect();
		assert!(rates.windows(2).all(|w| w[1] > w[0]), "monotonic");
		assert_eq!(camspeed_factor(0), 1.0, "no key: the Modern camera is untouched");
		assert_eq!(camspeed_factor(50), 1.0);
		assert_eq!(camspeed_factor(100), 2.0);
		assert_eq!(camspeed_factor(10), 0.2);
		// Retail's three rates are 1.0, 1.3 and 1.6 degrees.
		let deg: Vec<f32> = RATE_RETAIL.iter().map(|&b| f32::from_bits(b).to_degrees()).collect();
		assert!((deg[0] - 1.0).abs() < 1e-3 && (deg[1] - 1.3).abs() < 1e-3 && (deg[2] - 1.6).abs() < 1e-3, "{deg:?}");
	}

	#[test]
	fn lockon_truth_table() {
		// Auto follows the device (pad: on, keyboard: off); On and Off do not care.
		assert!(lockon_decide(0, false));
		assert!(!lockon_decide(0, true));
		assert!(lockon_decide(1, true) && lockon_decide(1, false));
		assert!(!lockon_decide(2, true) && !lockon_decide(2, false));
		assert!(lockon_decide(9, false) && !lockon_decide(9, true), "a stray byte acts as Auto");
	}

	#[test]
	fn camera_option_texts() {
		for lang in 0..5 {
			for id in ID_LOCKON..=ID_SMOOTH {
				let s = camopt_string(id, lang, 70).unwrap();
				assert!(!s.is_empty() && s.len() < 0x3f);
				assert!(s.iter().all(|&b| (0x20..0x7f).contains(&b) || b == 0x84), "printable ASCII (and the a-acute 0x84) in the font: {s:?}");
			}
		}
		assert_eq!(camopt_string(ID_LOCKON, 0, 0).unwrap(), b"Lock-on");
		assert_eq!(camopt_string(ID_LOCKON, 2, 0).unwrap(), b"Zielerfassung");
		assert_eq!(camopt_string(ID_AUTO, 3, 0).unwrap(), b"Auto");
		assert_eq!(camopt_string(ID_PCT, 1, 50).unwrap(), b"< 50 >");
		assert_eq!(camopt_string(ID_PCT, 4, 200).unwrap(), b"< 200 >");
		assert_eq!(camopt_string(ID_STATIC, 0, 0).unwrap(), b"Static");
		assert_eq!(camopt_string(ID_SMOOTH, 0, 0).unwrap(), b"Smooth");
		assert_eq!(camopt_string(ID_STATIC, 1, 0).unwrap(), b"Statique");
		assert_eq!(camopt_string(ID_SMOOTH, 1, 0).unwrap(), b"Fluide");
		assert_eq!(camopt_string(ID_STATIC, 2, 0).unwrap(), b"Statisch");
		assert_eq!(camopt_string(ID_SMOOTH, 2, 0).unwrap(), b"Weich");
		assert_eq!(camopt_string(ID_STATIC, 3, 0).unwrap(), b"Est\x84tica");
		assert_eq!(camopt_string(ID_SMOOTH, 3, 0).unwrap(), b"Suave");
		assert_eq!(camopt_string(ID_STATIC, 4, 0).unwrap(), b"Statica");
		assert_eq!(camopt_string(ID_SMOOTH, 4, 0).unwrap(), b"Fluida");
		assert_eq!(camopt_string(ID_SMOOTH + 1, 0, 50), None);
		assert_eq!(camopt_string(ID_LOCKON - 1, 0, 50), None);
	}

	fn state_dir(name: &str) -> Option<String> {
		[concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/")].iter().map(|b| format!("{b}{name}")).find(|p| std::path::Path::new(p).join("eeMemory.bin").exists())
	}

	/// On the loaded Eudora state: the rate table is found by value; without the key nothing at all is written; with the key all
	/// three entries take the rate, again after a level reload; the texts come out of the new pool through the real lookup.
	#[test]
	fn eudora_rate_table_texts_and_no_key_no_write() {
		let _t = turn();
		let Some(dir) = state_dir("fg1/4_0159") else { return };
		let mut m = ee_oracle::Machine::load(&dir);
		ee_game::register();
		register_hand(crate::retail::disc::hands(&HANDS));
		m.enable_rec();
		assert_eq!(m.level(), 4);
		reset_statics();
		assert_eq!(find_rate_table(&mut m.cpu.mem), Some(0x161ea0), "the Eudora overlay's table, found by its three values");
		let mut guard = ST.lock().unwrap_or_else(|e| e.into_inner());
		let s: &mut St = &mut guard;
		let file = std::env::temp_dir().join(format!("pcext_camopts_{}.txt", std::process::id()));
		let _ = std::fs::remove_file(&file);
		s.opts = PcOptions::default();
		s.path = Some(file.clone());
		// No key: setup, then a field with nothing pressed changes no byte of RAM at all.
		camopts_setup(&mut m.cpu.mem, s);
		let before = m.cpu.mem.ram.clone();
		camopts_field(&mut m.cpu.mem, 4, 0, s);
		camopts_field(&mut m.cpu.mem, 4, 0x2000, s);
		assert!(m.cpu.mem.ram == before, "no camspeed key: the field writes nothing");
		assert!(!file.exists(), "nothing is remembered until the player changes a row");
		assert_eq!(CAMSHOW.load(Ordering::Relaxed), preset_pct(m.cpu.mem.rd8(RETAIL_SPEED)), "the row shows retail's own preset");
		// Lock-on: the retail update cycles the byte, the host applies and remembers it.
		m.cpu.mem.wr8(LOCKON, 2);
		camopts_field(&mut m.cpu.mem, 4, 0, s);
		assert_eq!(LOCKON_MODE.load(Ordering::Relaxed), 2);
		assert!(!lockon_decide(LOCKON_MODE.load(Ordering::Relaxed), false));
		assert_eq!(PcOptions::load(&file).lockon, Some(2));
		// Key 100: all three entries take the doubled Fast rate; an unchanged table is not rewritten; a level reload (retail data
		// again) is written again; the retail byte is not touched by the key alone.
		let at = 0x161ea0;
		let byte = m.cpu.mem.rd8(RETAIL_SPEED);
		CAMSPEED.store(100, Ordering::Relaxed);
		camopts_field(&mut m.cpu.mem, 4, 0, s);
		let w = |m: &mut ee_oracle::Machine| [m.cpu.mem.rd32(at), m.cpu.mem.rd32(at + 4), m.cpu.mem.rd32(at + 8)];
		assert_eq!(w(&mut m), [camspeed_rate_bits(100); 3]);
		assert_eq!(f32::from_bits(w(&mut m)[0]), 2.0 * f32::from_bits(0x3ce4_c388));
		assert_eq!(m.cpu.mem.rd8(RETAIL_SPEED), byte);
		camopts_field(&mut m.cpu.mem, 4, 0, s);
		assert_eq!(w(&mut m), [camspeed_rate_bits(100); 3]);
		for (k, v) in RATE_RETAIL.iter().enumerate() {
			m.cpu.mem.wr32(at + 4 * k as u32, *v);
		}
		camopts_field(&mut m.cpu.mem, 4, 0, s);
		assert_eq!(w(&mut m), [camspeed_rate_bits(100); 3], "rewritten after a level reload");
		// A new value: the doubled rate gives way to the new one.
		CAMSPEED.store(50, Ordering::Relaxed);
		camopts_field(&mut m.cpu.mem, 4, 0, s);
		assert_eq!(w(&mut m), [0x3ce4_c388; 3], "50 % writes retail's Fast into all three entries");
		// The row's step: the player sets 120: remembered, retail's byte follows with the nearest preset (Fast).
		set_camspeed(&mut m.cpu.mem, s, 120);
		assert_eq!((CAMSPEED.load(Ordering::Relaxed), CAMSHOW.load(Ordering::Relaxed), m.cpu.mem.rd8(RETAIL_SPEED)), (120, 120, 2));
		assert_eq!(PcOptions::load(&file).camspeed, Some(120));
		set_camspeed(&mut m.cpu.mem, s, 30);
		assert_eq!(m.cpu.mem.rd8(RETAIL_SPEED), 0);
		// The texts through the real lookup: the language word decides, the percent row shows the current value.
		let look = |m: &mut ee_oracle::Machine, id: u32, word: u32| -> (u32, String) {
			m.cpu.mem.wr32(LANG_WORD, word);
			m.cpu.s(4, id as u64);
			m.cpu.call(0x1f6b80);
			let p = m.cpu.g(2) as u32;
			let mut out = Vec::new();
			let mut a = p;
			while m.cpu.mem.rd8(a) != 0 {
				out.push(m.cpu.mem.rd8(a));
				a += 1;
			}
			(p, out.iter().map(|&b| b as char).collect())
		};
		let (p, text) = look(&mut m, ID_LOCKON, 2);
		assert_eq!((p, text.as_str()), (CO_POOL, "Verrouillage"));
		let (p, text) = look(&mut m, ID_LOCKON, 3);
		assert_eq!((p, text.as_str()), (CO_POOL, "Zielerfassung"));
		let (p, text) = look(&mut m, ID_AUTO, 5);
		assert_eq!((p, text.as_str()), (CO_POOL + 0x40, "Auto"));
		CAMSHOW.store(170, Ordering::Relaxed);
		let (p, text) = look(&mut m, ID_PCT, 0);
		assert_eq!((p, text.as_str()), (CO_POOL + 0x80, "< 170 >"));
		// A retail id still declines to retail's lookup (German state: 'Kamera').
		assert_eq!(look(&mut m, ID_CAMERA, 3).1, "Kamera");
		let _ = std::fs::remove_file(&file);
		s.path = None;
		s.opts = PcOptions::default();
		reset_statics();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn state_dir(name: &str) -> Option<String> {
		[concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/")].iter().map(|b| format!("{b}{name}")).find(|p| std::path::Path::new(p).join("eeMemory.bin").exists())
	}

	fn scratch(name: &str) -> PathBuf {
		let d = std::env::temp_dir().join(format!("pcext_test_{}_{name}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		d
	}

	static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

	#[test]
	fn strafe_gate_truth_table() {
		// Extension on: only Modern + On.
		assert!(gate(true, true, true));
		assert!(!gate(true, true, false));
		assert!(!gate(true, false, true), "Classic camera: strafe is off whatever the row says");
		assert!(!gate(true, false, false));
		// Extension off: the old switches decide, the menu does not restrict.
		assert!(gate(false, false, false));
	}

	#[test]
	fn texts_fit_the_font_and_are_nonempty() {
		for row in TEXTS {
			for s in row {
				assert!(!s.is_empty() && s.len() < 0x3f);
				// Printable ASCII, the game's accent bytes (0x80..0xb0, see TEXTS) or the plate's newline (1) and button glyphs (0x10, 0x13).
				assert!(s.iter().all(|&b| (0x20..0x7f).contains(&b) || (0x80..=0xb0).contains(&b) || [1, 0x10, 0x13].contains(&b)));
			}
		}
	}

	/// The hashes and table addresses against the extracted overlays: the text lookup of every level and the Camera Options row table.
	#[test]
	fn tables_match_the_overlays() {
		let dir = [concat!(env!("CARGO_MANIFEST_DIR"), "/../extracted/levels")].into_iter().find(|d| std::path::Path::new(d).is_dir());
		let Some(dir) = dir else { return };
		let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().join("overlay.elf").exists()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
		names.sort_by_key(|n| n.split('_').next().and_then(|x| x.parse::<u32>().ok()).unwrap_or(99));
		assert_eq!(names.len(), 19);
		for (lv, name) in names.iter().enumerate() {
			let elf = std::fs::read(format!("{dir}/{name}/overlay.elf")).unwrap();
			let word = |a: u32| -> Option<u32> {
				let u = |o: usize| u32::from_le_bytes(elf[o..o + 4].try_into().unwrap()) as usize;
				let (phoff, phn) = (u(0x1c), u16::from_le_bytes([elf[0x2c], elf[0x2d]]) as usize);
				(0..phn).find_map(|i| {
					let p = phoff + 32 * i;
					let (ty, off, va, fs) = (u(p), u(p + 4), u(p + 8) as u32, u(p + 16));
					(ty == 1 && a >= va && (a - va) as usize + 4 <= fs).then(|| u(off + (a - va) as usize) as u32)
				})
			};
			let h = &HANDS[lv];
			let mut hash = 0xcbf2_9ce4_8422_2325u64;
			for a in (h.entry..h.end).step_by(4) {
				hash = (hash ^ word(a).unwrap() as u64).wrapping_mul(0x100_0000_01b3);
			}
			assert_eq!(hash, h.hash, "text lookup hash, level {lv} {name}");
			assert_eq!(h.end - h.entry, 0x40);
			// The row table: signature, and the item (table + 0x60) points back at it.
			let t = TABLES[lv];
			assert_eq!((word(t), word(t + 4), word(t + 8), word(t + 12), word(t + 16), word(t + 20)), (Some(0x4fa2), Some(0x15ede0), Some(0x4fa5), Some(0x4fa4), Some(0), Some(0)), "table signature, level {lv}");
			assert_eq!(word(t + PTR_OFF), Some(t), "item pointer, level {lv}");
		}
	}

	/// No other hand sits on one of the text lookups (the PC hand would replace it; its decline path runs the retail code).
	#[test]
	fn no_other_hand_on_a_text_lookup() {
		ee_game::register();
		let n: Vec<_> = ee_oracle::rec::hands().into_iter().filter(|h| HANDS.iter().any(|p| p.entry == h.entry && !std::ptr::eq(*h, p))).map(|h| h.entry).collect();
		eprintln!("hands on a text lookup entry besides ours: {n:x?}");
		assert!(n.is_empty(), "another hand sits on a text lookup: {n:x?}");
	}

	/// On a loaded Eudora state: the lookup answers the new ids in all five languages, declines the retail ones, the row table is
	/// patched once, and a byte change is applied and remembered.
	#[test]
	fn eudora_text_rows_and_persistence() {
		// The renderer flags (GFX, FULLSCREEN) are global: the tests that sync take turns.
		let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		let Some(dir) = state_dir("fg1/4_0159") else { return };
		let mut m = ee_oracle::Machine::load(&dir);
		ee_game::register();
		register_hand(crate::retail::disc::hands(&HANDS));
		m.enable_rec();
		let level = m.level();
		assert_eq!(level, 4);
		// Text lookup: the new ids in every language, via the real call path.
		let pool = |m: &mut ee_oracle::Machine, id: u32, word: u32| -> String {
			m.cpu.mem.wr32(LANG_WORD, word);
			m.cpu.s(4, id as u64);
			m.cpu.call(0x1f6b80);
			let p = m.cpu.g(2) as u32;
			let mut out = Vec::new();
			let mut a = p;
			while m.cpu.mem.rd8(a) != 0 {
				out.push(m.cpu.mem.rd8(a));
				a += 1;
			}
			assert!((POOL..POOL + 0x140).contains(&p) || (POOL2..POOL2 + 0x100).contains(&p) || id < 0x7f00, "pool pointer for {id:#x}: {p:#x}");
			out.iter().map(|&b| b as char).collect()
		};
		for (word, expect) in [(0u32, "Modern"), (2, "Moderne"), (3, "Modern"), (4, "Moderna"), (5, "Moderna")] {
			assert_eq!(pool(&mut m, ID_MODERN, word), expect, "language word {word}");
		}
		assert_eq!(pool(&mut m, ID_STRAFE, 3), "Strafe-Zielen");
		assert_eq!(pool(&mut m, ID_CLASSIC, 0), "Classic");
		// A retail id declines: the retail lookup runs and finds the German 'Kamera' (the state is German).
		m.cpu.mem.wr32(LANG_WORD, 3);
		assert_eq!(pool(&mut m, ID_CAMERA, 3), "Kamera");
		assert_eq!(pool(&mut m, ID_ON, 3), "ein");
		assert_eq!(pool(&mut m, ID_OFF, 3), "aus");
		// Rows: the retail table is patched once, then idempotently.
		let t = TABLES[4];
		assert_eq!(m.cpu.mem.rd32(t + PTR_OFF), t);
		let dirp = scratch("eudora");
		let mut s = St { opts: PcOptions::default(), path: Some(dirp.join("pc_options.txt")), last: None, glast: None, lang_seen: None, warned: false };
		assert!(!sync(&mut m.cpu.mem, 4, false, &mut s));
		assert_eq!(m.cpu.mem.rd32(t + PTR_OFF), TAB);
		assert_eq!((m.cpu.mem.rd8(CAM), m.cpu.mem.rd8(STRAFE), m.cpu.mem.rd8(BUYALL)), (2, 1, 1), "new player: Modern Smooth, strafe On, buy all On");
		let rec = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..6).map(|k| mem.rd32(TAB + 24 * i + 4 * k)).collect() };
		assert_eq!(rec(&mut m.cpu.mem, 0), [0x4fa2, 0x15ede0, 0x4fa5, 0x4fa4, 0, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 3), [ID_CAMERA, CAM, ID_CLASSIC, ID_STATIC, ID_SMOOTH, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 4), [ID_STRAFE, STRAFE, ID_OFF, ID_ON, 0, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 2), [0x521b, CAMSEL, ID_PCT, ID_PCT, 0, 0], "camera speed row in place of Slow / Medium / Fast");
		assert_eq!(rec(&mut m.cpu.mem, 5), [ID_BUYALL, BUYALL, ID_OFF, ID_ON, 0, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 6), [ID_LOCKON, LOCKON, ID_AUTO, ID_ON, ID_OFF, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 7), [0; 6]);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		assert_eq!(m.cpu.mem.rd32(t + PTR_OFF), TAB);
		assert!(!dirp.join("pc_options.txt").exists(), "nothing is written until the player changes something");
		// The player flips Camera to Classic in the menu (the retail update writes the byte): applied, remembered, strafe follows.
		m.cpu.mem.wr8(CAM, 0);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		assert!(!CAM_MODERN.load(Ordering::Relaxed));
		assert_eq!(PcOptions::load(&dirp.join("pc_options.txt")).camera, Some(false));
		// F7 flips it back (to the remembered feel, Smooth for a new player) and is consumed.
		assert!(sync(&mut m.cpu.mem, 4, true, &mut s));
		assert_eq!(m.cpu.mem.rd8(CAM), 2);
		assert!(CAM_MODERN.load(Ordering::Relaxed));
		// Static in the menu (the retail update cycles Classic -> Static -> Smooth): still Modern for every other gate, remembered.
		m.cpu.mem.wr8(CAM, 1);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		assert!(CAM_MODERN.load(Ordering::Relaxed) && strafe_on());
		let o = PcOptions::load(&dirp.join("pc_options.txt"));
		assert_eq!((o.camera, o.camfeel), (Some(true), Some(false)));
		assert!(std::fs::read_to_string(dirp.join("pc_options.txt")).unwrap().contains("camfeel=static"));
		// Classic, then F7 returns to the remembered Static, not to Smooth.
		m.cpu.mem.wr8(CAM, 0);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		assert!(!CAM_MODERN.load(Ordering::Relaxed));
		assert!(sync(&mut m.cpu.mem, 4, true, &mut s));
		assert_eq!(m.cpu.mem.rd8(CAM), 1, "F7 goes back to Static");
		// Smooth again: the file has no feel key (the default), Modern.
		m.cpu.mem.wr8(CAM, 2);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		let o = PcOptions::load(&dirp.join("pc_options.txt"));
		assert_eq!((o.camera, o.camfeel), (Some(true), None));
		assert!(!std::fs::read_to_string(dirp.join("pc_options.txt")).unwrap().contains("camfeel"));
		// Strafe Off in the menu.
		m.cpu.mem.wr8(STRAFE, 0);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		assert!(!STRAFE_ON.load(Ordering::Relaxed));
		assert_eq!(PcOptions::load(&dirp.join("pc_options.txt")).strafe, Some(false));
		// A level reload re-initialises the overlay data: the next look patches again, the bytes stay.
		m.cpu.mem.wr32(t + PTR_OFF, t);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		assert_eq!(m.cpu.mem.rd32(t + PTR_OFF), TAB);
		assert_eq!(m.cpu.mem.rd8(STRAFE), 0);
		// Another level's guard: never touches a wrong overlay.
		assert!(!patch_rows(&mut m.cpu.mem, 5));
		assert!(!patch_rows(&mut m.cpu.mem, 99));
		let _ = std::fs::remove_dir_all(&dirp);
		// Leave the process-wide state as a fresh run has it.
		CAM_MODERN.store(true, Ordering::Relaxed);
		STRAFE_ON.store(true, Ordering::Relaxed);
		modcam::set_active(true);
		modcam::set_smooth(true);
	}

	/// Debug mode (Controls -> Debug) on the Eudora state: the row's byte opens the dev tools and the debug keys live (the menu, or the
	/// environment variables), the file keeps `debug=on` only while it is On, the notice goes up only for the player's own switch, Off
	/// takes every key and the drawing away again, and a file that says On starts On without the notice.
	#[test]
	fn eudora_debug_mode_row() {
		use super::super::{debugkeys, devtools, pcctl};
		use winit::keyboard::KeyCode as K;
		let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		let Some(dir) = state_dir("fg1/4_0159") else { return };
		let env = ["RAC_DEV_TOOLS", "RAC_DEBUG_KEYS"].iter().any(|n| std::env::var(n).is_ok_and(|v| v == "1"));
		let quads = || {
			let mut q = Vec::new();
			devtools::draw(&mut q, 1920.0, 1080.0, 4, [0.0; 3]);
			q.len()
		};
		devtools::set_menu(false, false);
		let mut m = ee_oracle::Machine::load(&dir);
		let mem = &mut m.cpu.mem;
		let dirp = scratch("debug");
		let file = dirp.join("pc_options.txt");
		let mut s = St { opts: PcOptions::default(), path: Some(file.clone()), last: None, glast: None, lang_seen: None, warned: false };
		sync(mem, 4, false, &mut s);
		assert_eq!(pcctl::debug_on(mem), Some(false), "a new player: Off");
		assert_eq!((devtools::menu(), s.opts.debug), (false, None));
		let text = |f: &std::path::Path| std::fs::read_to_string(f).unwrap_or_default();
		assert!(!text(&file).contains("debug"), "Off writes no key");
		if !env {
			assert!(!devtools::enabled() && !debugkeys::enabled());
			assert!(!devtools::ui_key(K::F1, true, false) && !devtools::ui_key(K::F2, true, false), "Off: no key is taken");
			assert_eq!((debugkeys::title_suffix(), devtools::title_suffix(4, [0.0; 3])), (String::new(), String::new()));
			assert_eq!(quads(), 0, "Off: nothing is drawn");
			assert!(!devtools::free_on() && devtools::take_warp(true, 4).is_none() && devtools::pad_mask() == 0);
		}
		// The player switches the row On.
		mem.wr8(pcctl::DBGB, 1);
		sync(mem, 4, false, &mut s);
		assert!(devtools::menu() && devtools::enabled() && debugkeys::enabled());
		assert_eq!(s.opts.debug, Some(true));
		assert!(text(&file).contains("debug=on\n"), "{}", text(&file));
		assert!(quads() > 100, "the notice (keys and warning) is drawn");
		assert!(devtools::NOTICE.iter().any(|l| l.contains("WARNING") && l.contains("STORY")) && devtools::NOTICE.iter().any(|l| l.contains("SAVE")));
		assert!(devtools::ui_key(K::F1, true, false), "F1 opens the planet list");
		assert!(devtools::ui_key(K::Escape, true, false), "Esc closes it");
		assert!(devtools::ui_key(K::F2, true, false) && devtools::free_on(), "F2: free camera");
		sync(mem, 4, false, &mut s);
		assert!(devtools::menu(), "an unchanged byte changes nothing");
		// Off again: the free camera and the tools are put away, the key leaves the file.
		mem.wr8(pcctl::DBGB, 0);
		sync(mem, 4, false, &mut s);
		assert_eq!((devtools::menu(), s.opts.debug), (false, None));
		assert!(!text(&file).contains("debug"), "{}", text(&file));
		if !env {
			assert!(!devtools::enabled() && !debugkeys::enabled() && !devtools::free_on());
			assert!(!devtools::ui_key(K::F1, true, false) && !devtools::ui_key(K::F2, true, false));
			assert_eq!(quads(), 0);
		}
		// On again later: the free camera does not come back by itself.
		mem.wr8(pcctl::DBGB, 1);
		sync(mem, 4, false, &mut s);
		assert!(devtools::menu() && !devtools::free_on());
		// A file that says On (`init`: set_menu(true, false)): fresh RAM shows On, nothing is rewritten, no notice.
		assert!(devtools::set_menu(false, false));
		assert!(devtools::set_menu(true, false));
		let mut m2 = ee_oracle::Machine::load(&dir);
		let mem2 = &mut m2.cpu.mem;
		let mut s2 = St { opts: PcOptions::parse("debug=on\n"), path: Some(dirp.join("pc_options2.txt")), last: None, glast: None, lang_seen: None, warned: false };
		sync(mem2, 4, false, &mut s2);
		assert_eq!((pcctl::debug_on(mem2), s2.opts.debug, devtools::menu()), (Some(true), Some(true), true));
		assert!(!dirp.join("pc_options2.txt").exists(), "no change, no write");
		assert_eq!(quads(), 0, "no notice for a file's On");
		devtools::set_menu(false, false);
	}

	/// On a loaded Eudora state: the Options list gains Graphics before Quit Game (record, description, picture), the row opens a
	/// copy of Camera Options that reads the Graphics rows, the patch is idempotent and redone after a level reload, and a value byte
	/// change reaches the renderer's verdict and the file.
	#[test]
	fn eudora_graphics_page() {
		// The renderer flags (GFX, FULLSCREEN) are global: the tests that sync take turns.
		let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		let Some(dir) = state_dir("fg1/4_0159") else { return };
		let mut m = ee_oracle::Machine::load(&dir);
		let mem = &mut m.cpu.mem;
		let t = TABLES[4];
		let (cp, opt) = (t - 0xe0, mem.rd32(t - 0xe0 + 0x38));
		let list = opt + 0x1a8; // Eudora 0x1b4bf8: the page's list item
		assert_eq!((opt, mem.rd32(list + 0x34)), (0x1b4a50, 0x1b4b98), "Eudora Options page and its list records");
		let (text, pic) = (0x1b4c68, 0x1b4b30);
		let (descs, pics) = (mem.rd32(text + 0x34), mem.rd32(pic + 0x30));
		let dirp = scratch("graphics");
		let mut s = St { opts: PcOptions::default(), path: Some(dirp.join("pc_options.txt")), last: None, glast: None, lang_seen: None, warned: false };
		sync(mem, 4, false, &mut s);
		assert_eq!(mem.rd32(list + 0x34), OPT_RECS);
		assert_eq!(mem.rd32(text + 0x34), OPT_DESC);
		assert_eq!(mem.rd32(pic + 0x30), OPT_PICS);
		let ids: Vec<u32> = (0..10).map(|i| mem.rd32(OPT_RECS + 12 * i) & 0xffff).collect();
		assert_eq!(ids, [0x4f1e, 0x4f1f, 0x4f20, 0x4f22, 0x4f23, 0x4f24, ID_GRAPHICS, super::super::pcctl::ID_CONTROLS, 0x4f26, 0]);
		assert_eq!((mem.rd32(OPT_RECS + 84) >> 16, mem.rd32(OPT_RECS + 88)), (3, super::super::pcctl::PAGE_AT), "type 3: go to the Controls page");
		assert_eq!((mem.rd32(OPT_RECS + 72) >> 16, mem.rd32(OPT_RECS + 76)), (3, GPAGE), "type 3: go to the Graphics page");
		let desc: Vec<u32> = (0..10).map(|i| mem.rd32(OPT_DESC + 4 * i)).collect();
		assert_eq!(desc, [0x4f31, 0x4f32, 0x4f33, 0x4f35, 0x4f36, 0x4f37, ID_GRAPHICS_DESC, super::super::pcctl::ID_DESC, 0x4f39, 0]);
		// Pictures: the retail seven around help_ss 5 (the record just below options_ss 0 is help_ss 6) and the Camera row's picture for Controls.
		assert_eq!(mem.rd32(OPT_PICS + 8 * 8), mem.rd32(0x138040 + 8 * 6));
		assert_eq!((mem.rd32(OPT_PICS + 8 * 7), mem.rd32(OPT_PICS + 8 * 7 + 4)), (mem.rd32(OPT_PICS + 8 * 4), mem.rd32(OPT_PICS + 8 * 4 + 4)));
		assert_eq!(mem.rd32(OPT_PICS + 8 * 6), mem.rd32(0x138040 - 0x10));
		assert_ne!(mem.rd32(OPT_PICS + 8 * 6 + 4), 0);
		// The page: id 0x1d, parent Options, current item = our value item, items = ours where Camera Options has its own.
		assert_eq!((mem.rd32(GPAGE + 0x38), mem.rd32(GPAGE + 0x3c), mem.rd32(GPAGE + 0x40)), (opt, 0x1d, GVAL));
		// Panel sequences: Camera Options' except the value panel, which plays the Sound page's (Options row 3) taller one.
		let sound = mem.rd32(OPT_RECS + 12 * 3 + 4);
		assert_eq!((mem.rd32(OPT_RECS + 12 * 3), mem.rd32(sound + 0x38)), (0x4f22 | 3 << 16, opt));
		let seqs = |mem: &mut Mem, p: u32| -> Vec<u32> { (0..14).map(|k| mem.rd32(p + 4 * k)).collect() };
		let (mut want, got) = (seqs(mem, cp), seqs(mem, GPAGE));
		want[3] = mem.rd32(sound + 0xc);
		assert_eq!(mem.rd32(cp + 0xc), want[3], "Camera Options (7 rows) plays the taller panel too");
		assert_eq!(got, want);
		let items: Vec<u32> = (0..6).map(|k| mem.rd32(GPAGE + 0x44 + 4 * k)).collect();
		let cam: Vec<u32> = (0..6).map(|k| mem.rd32(cp + 0x44 + 4 * k)).collect();
		assert_eq!(items, [GHDR, cam[1], cam[2], GVAL, cam[4], 0]);
		// The Controls page (pcctl.rs): the same shape with its own header and value items and the 13-row table.
		let pc = super::super::pcctl::PAGE_AT;
		assert_eq!((mem.rd32(pc + 0x38), mem.rd32(pc + 0x3c)), (opt, 0x1d));
		let citems: Vec<u32> = (0..6).map(|k| mem.rd32(pc + 0x44 + 4 * k)).collect();
		assert_eq!((citems[1], citems[2], citems[4], citems[5]), (cam[1], cam[2], cam[4], 0));
		assert!(citems[0] != cam[0] && citems[0] != GHDR && citems[3] != cam[3] && citems[3] != GVAL);
		let ctab = mem.rd32(citems[3] + 0x34);
		let crow = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..6).map(|k| mem.rd32(ctab + 24 * i + 4 * k)).collect() };
		let cb = super::super::pcctl::CBASE;
		assert_eq!(crow(mem, 0), [0x7f73, cb, 0x7f74, 0x7f74, 0, 0]);
		assert_eq!(crow(mem, 1), [0x7f41, cb + 1, 0x7f51, 0x7f61, 0, 0]);
		assert_eq!(crow(mem, 6), [0x7f46, cb + 6, 0x7f56, 0x7f66, 0, 0]);
		assert_eq!(crow(mem, 7), [0; 6], "at most 7 rows per page");
		assert_eq!(mem.rd32(citems[0] + 0x34), 0x7f71);
		assert_eq!((mem.rd32(GHDR + 0x34), mem.rd32(GVAL + 0x34), mem.rd32(GVAL)), (ID_GRAPHICS_HDR, GTAB, mem.rd32(t + 0x60)));
		let row = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..6).map(|k| mem.rd32(GTAB + 24 * i + 4 * k)).collect() };
		// Section 0 (Picture): the selector, Lighting, Ambient occlusion, Dynamic shadows, Global illumination, Sun rays, HUD.
		let gb = cb + 0x10;
		assert_eq!(row(mem, 0), [0x7f73, gb, 0x7f75, 0x7f75, 0, 0]);
		assert_eq!(row(mem, 1), [ID_LIGHTING, LIGHT, ID_LIGHT_CLASSIC, ID_ENHANCED, 0, 0]);
		assert_eq!(row(mem, 2), [ID_AO, AO, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW]);
		assert_eq!(row(mem, 3), [ID_SHADOWS, SHADOWS, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW]);
		assert_eq!(row(mem, 4), [ID_GI, GI, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW]);
		assert_eq!(row(mem, 5), [ID_SUNRAYS, SUNRAYS, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW]);
		assert_eq!(row(mem, 6), [0x7f76, gb + 1, 0x7f77, 0x7f78, 0, 0]);
		assert_eq!(row(mem, 7), [0; 6]);
		// Section 1 (Effects): Water, Waves, Particles, Vegetation; section 2 (Screen): Display mode, Frame smoothing, Draw distance, Culling.
		super::super::pcctl::test_gfx_section(mem, 1, &ROWS, GTAB);
		assert_eq!(row(mem, 0), [0x7f73, gb, 0x7f75, 0x7f75, 0, 0]);
		assert_eq!(row(mem, 1), [ID_WATER, WATER, ID_CLASSIC, ID_HIGH, ID_MEDIUM, ID_LOW]);
		assert_eq!(row(mem, 2), [ID_WAVES, WAVES, ID_ON, ID_OFF, 0, 0]);
		assert_eq!(row(mem, 3), [ID_PARTICLES, PARTICLES, ID_PART_CLASSIC, ID_HIGH, ID_MEDIUM, ID_LOW]);
		assert_eq!(row(mem, 4), [ID_VEGETATION, VEG, ID_OFF, ID_VEG_MODERATE, ID_VEG_LUSH, 0]);
		assert_eq!(row(mem, 5), [0; 6]);
		super::super::pcctl::test_gfx_section(mem, 2, &ROWS, GTAB);
		assert_eq!(row(mem, 1), [ID_DISPLAY, DISPLAY, ID_WINDOWED, ID_FULLSCREEN, 0, 0]);
		assert_eq!(row(mem, 2), [ID_SMOOTHING, SMOOTH, ID_OFF, ID_ON, 0, 0]);
		assert_eq!(row(mem, 3), [ID_DRAW, DRAW, ID_MEDIUM, ID_HIGH, ID_LOW, 0]);
		assert_eq!(row(mem, 4), [ID_CULLING, CULL, ID_ON, ID_OFF, 0, 0]);
		assert_eq!(row(mem, 5), [0; 6]);
		super::super::pcctl::test_gfx_section(mem, 0, &ROWS, GTAB);
		assert_eq!(GROWS.map(|a| mem.rd8(a)), DEFAULT_ROWS, "new player: Classic, the effect rows at High, Windowed, smoothing On, Moderate, culling On, GI Off, waves On, draw distance Medium");
		assert_eq!(DEFAULT_ROWS, [0, 1, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0], "today's menu bytes (culling On = 0, global illumination Off = 0, waves On = 0, draw distance Medium = 0)");
		assert!(!fullscreen());
		assert_eq!(
			graphics(),
			Graphics { lighting: 0, ao: true, shadows: true, water: true, particles: true, sun_rays: true, smoothing: true, veg: 1, culling: true, ao_q: 3, shadows_q: 3, water_q: 3, particles_q: 3, sun_q: 3, gi_q: 0, waves: true, draw: 1 }
		);
		// Idempotent; a level reload (fresh overlay data) patches again.
		assert!(!patch_graphics(mem, 4));
		mem.wr32(list + 0x34, 0x1b4b98);
		mem.wr32(text + 0x34, descs);
		mem.wr32(pic + 0x30, pics);
		assert!(patch_graphics(mem, 4));
		assert!(!patch_graphics(mem, 5), "another level's guard");
		// The player picks Enhanced (the retail update cycles the byte): the renderer follows and the file remembers it.
		mem.wr8(LIGHT, 1);
		sync(mem, 4, false, &mut s);
		assert!(graphics().enhanced());
		let o = PcOptions::load(&dirp.join("pc_options.txt"));
		assert_eq!((o.lighting, o.ao, o.shadows), (Some(1), Some(3), Some(3)));
		assert_eq!(o.camera, None, "a Graphics change writes the Graphics keys only");
		// The HUD row (pcctl.rs byte): Original -> the flag and the file follow; Sharp again.
		assert!(hud_sharp_now());
		mem.wr8(gb + 1, 1);
		sync(mem, 4, false, &mut s);
		assert!(!hud_sharp_now());
		assert_eq!(s.opts.hud, Some(false));
		assert!(std::fs::read_to_string(dirp.join("pc_options.txt")).unwrap().contains("hud=original
"));
		mem.wr8(gb + 1, 0);
		sync(mem, 4, false, &mut s);
		assert!(hud_sharp_now());
		assert_eq!(s.opts.hud, Some(true));
		// Water Classic, Sun rays Off: the renderer and the file follow.
		// (High -> Medium -> Low -> Off is three presses; the byte is written 0 each time, the row steps on from its last value.)
		for _ in 0..3 {
			mem.wr8(WATER, 0);
			mem.wr8(SUNRAYS, 0);
			sync(mem, 4, false, &mut s);
		}
		let g = graphics();
		assert!(!g.water && g.particles && !g.sun_rays);
		assert_eq!((g.water_q, g.particles_q, g.sun_q), (0, 3, 0));
		let o = PcOptions::load(&dirp.join("pc_options.txt"));
		assert_eq!((o.water, o.particles, o.sun_rays), (Some(0), Some(3), Some(0)));
		// The four-value rows: the pause page cycles a row +1 mod 4 (Off -> Low -> Medium -> High -> Off); the title's flip (any other
		// byte) steps the row on from its last value too. The renderer's levels and the file follow every press.
		// A press always steps +1 from the last value, whatever the byte now says (a title flip may write 0 from 2).
		for (write, want) in [(2u8, 2u8), (3, 3), (0, 0), (1, 1), (2, 2), (3, 3), (0, 0), (1, 1), (0, 2), (0, 3), (0, 0)] {
			mem.wr8(AO, write);
			sync(mem, 4, false, &mut s);
			assert_eq!((mem.rd8(AO), graphics().ao_q, graphics().ao), (want, swap_level(want), want != 0), "AO byte {write}");
		}
		assert_eq!(PcOptions::load(&dirp.join("pc_options.txt")).ao, Some(0));
		for (row, seq) in [(SHADOWS, [(2u8, 2u8), (2, 2), (2, 2)]), (WATER, [(1, 1), (2, 2), (3, 3)]), (PARTICLES, [(2, 2), (2, 2), (2, 2)]), (SUNRAYS, [(1, 1), (2, 2), (1, 3)])] {
			for (write, want) in seq {
				mem.wr8(row, write);
				sync(mem, 4, false, &mut s);
				assert_eq!(mem.rd8(row), want);
			}
		}
		mem.wr8(WATER, 0);
		sync(mem, 4, false, &mut s);
		assert_eq!(mem.rd8(WATER), 0);
		mem.wr8(WATER, 1);
		sync(mem, 4, false, &mut s);
		let (g, o) = (graphics(), PcOptions::load(&dirp.join("pc_options.txt")));
		assert_eq!((g.shadows_q, g.water_q, g.particles_q, g.sun_q), (2, 3, 2, 1), "byte 1 = High, 2 = Medium, 3 = Low");
		assert_eq!((o.shadows, o.water, o.particles, o.sun_rays), (Some(2), Some(3), Some(2), Some(1)));
		assert!(std::fs::read_to_string(dirp.join("pc_options.txt")).unwrap().contains("shadows=medium\ndisplay=windowed\nwater=high\nparticles=medium\nsunrays=low\n"));
		// The title's rows show the level text of each row.
		let tr: Vec<(u32, u32)> = title_rows();
		assert_eq!(tr[2], (ID_SHADOWS, ID_MEDIUM));
		assert_eq!(tr[4], (ID_WATER, ID_HIGH));
		assert_eq!(tr[6], (ID_SUNRAYS, ID_LOW));
		assert_eq!(tr[1], (ID_AO, ID_OFF));
		// Vegetation: the pause page cycles Moderate -> Lush -> Off -> Moderate; the title's flip (2 -> 0 or 3, 1 -> 0) steps on too.
		for (write, want) in [(2u8, 2u8), (0, 0), (1, 1), (0, 2), (3, 0)] {
			mem.wr8(VEG, write);
			sync(mem, 4, false, &mut s);
			assert_eq!((mem.rd8(VEG), graphics().veg), (want, want), "veg byte {write}");
		}
		assert_eq!(PcOptions::load(&dirp.join("pc_options.txt")).vegetation, Some(0));
		// Culling: byte 0 On (the default), 1 Off; the renderer follows and the file remembers it.
		assert!(graphics().culling);
		mem.wr8(CULL, 1);
		sync(mem, 4, false, &mut s);
		assert!(!graphics().culling);
		assert_eq!(PcOptions::load(&dirp.join("pc_options.txt")).culling, Some(false));
		assert!(std::fs::read_to_string(dirp.join("pc_options.txt")).unwrap().contains("culling=off\n"));
		mem.wr8(CULL, 0);
		sync(mem, 4, false, &mut s);
		assert!(graphics().culling);
		// Global illumination: Off -> High -> Medium -> Low -> Off (bytes 0, 1, 2, 3 = levels 0, 3, 2, 1); the file remembers it.
		assert_eq!(graphics().gi_q, 0);
		for (write, want) in [(1u8, 3u8), (2, 2), (3, 1), (0, 0), (1, 3)] {
			mem.wr8(GI, write);
			sync(mem, 4, false, &mut s);
			assert_eq!((mem.rd8(GI), graphics().gi_q), (write, want), "GI byte {write}");
		}
		assert_eq!(PcOptions::load(&dirp.join("pc_options.txt")).gi, Some(3));
		assert!(std::fs::read_to_string(dirp.join("pc_options.txt")).unwrap().contains("gi=high\n"));
		assert_eq!(title_rows()[10], (ID_GI, ID_HIGH));
		// Back to Off: every change is one press (High -> Medium -> Low -> Off).
		for b in [2u8, 3, 0] {
			mem.wr8(GI, b);
			sync(mem, 4, false, &mut s);
		}
		assert_eq!(graphics().gi_q, 0);
		// Waves: On (byte 0) by default, Off (1) is remembered.
		assert!(graphics().waves);
		mem.wr8(WAVES, 1);
		sync(mem, 4, false, &mut s);
		assert!(!graphics().waves);
		assert!(std::fs::read_to_string(dirp.join("pc_options.txt")).unwrap().contains("waves=off\n"));
		mem.wr8(WAVES, 0);
		sync(mem, 4, false, &mut s);
		assert!(graphics().waves);
		// Draw distance: Medium (byte 0) by default; the pause page cycles Medium -> High -> Low -> Medium (bytes 0, 1, 2 = levels
		// 1, 2, 0), every change is one press, with either Lighting; the file names the level.
		assert_eq!(graphics().draw_q(), 1);
		for (write, want, level, name) in [(1u8, 1u8, 2u8, "high"), (2, 2, 0, "low"), (0, 0, 1, "medium"), (1, 1, 2, "high")] {
			mem.wr8(DRAW, write);
			sync(mem, 4, false, &mut s);
			assert_eq!((mem.rd8(DRAW), graphics().draw_q()), (want, level), "draw byte {write}");
			assert_eq!(PcOptions::load(&dirp.join("pc_options.txt")).draw, Some(level));
			assert!(std::fs::read_to_string(dirp.join("pc_options.txt")).unwrap().contains(&format!("draw={name}\n")));
		}
		assert_eq!(title_rows()[12], (ID_DRAW, ID_HIGH));
		// The title's flip (High 1 -> 0, Low 2 -> 3 or 0) steps the row on too: High -> Low -> Medium.
		for (write, want) in [(0u8, 2u8), (3, 0)] {
			mem.wr8(DRAW, write);
			sync(mem, 4, false, &mut s);
			assert_eq!(mem.rd8(DRAW), want, "title flip to {write}");
		}
		assert_eq!(graphics().draw_q(), 1);
		// A fresh start reads the remembered level back (bytes 0..2 through `pcopts::draw_byte`).
		assert_eq!(swap_rows(PcOptions::load(&dirp.join("pc_options.txt")).graphics_rows())[12], 0);
		// Display mode Fullscreen: the window follows (`fullscreen`) and the file remembers it; a fresh start reads it back.
		mem.wr8(DISPLAY, 1);
		sync(mem, 4, false, &mut s);
		assert!(fullscreen());
		let o = PcOptions::load(&dirp.join("pc_options.txt"));
		assert_eq!(o.fullscreen, Some(true));
		assert!(std::fs::read_to_string(dirp.join("pc_options.txt")).unwrap().contains("display=fullscreen\n"));
		mem.wr32(BASE, 0);
		FULLSCREEN.store(false, Ordering::Relaxed);
		sync(mem, 4, false, &mut s);
		assert_eq!(mem.rd8(DISPLAY), 1, "fresh RAM takes the remembered display mode");
		assert!(fullscreen());
		let _ = std::fs::remove_dir_all(&dirp);
		GFX.store(pack_gfx(DEFAULT_ROWS), Ordering::Relaxed);
		FULLSCREEN.store(false, Ordering::Relaxed);
	}

	/// The Button glyphs row: its byte is the style, a change steps it on (the pause page cycles it, the title's flip only marks
	/// a change), the file names it and the title screen's row list shows the style.
	#[test]
	fn the_glyphs_row_steps_and_is_remembered() {
		let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		let dirp = scratch("glyphs");
		let mut mem = Mem::new();
		let mut s = St { opts: PcOptions::default(), path: Some(dirp.join("pc_options.txt")), last: None, glast: None, lang_seen: None, warned: false };
		// A new player gets the Xbox letters.
		sync(&mut mem, 4, false, &mut s);
		assert_eq!(mem.rd8(GLYPHS), 1, "Xbox by default");
		assert_eq!(title_rows()[13], (crate::retail::pcctl::ID_GLYPHS, crate::retail::pcctl::ID_XBOX));
		// The row steps on: Xbox -> Switch -> PlayStation -> Xbox.
		for want in [2u8, 0, 1] {
			let next = (mem.rd8(GLYPHS) + 1) % 3;
			mem.wr8(GLYPHS, next);
			sync(&mut mem, 4, false, &mut s);
			assert_eq!(mem.rd8(GLYPHS), want, "the row steps on");
			assert_eq!(title_rows()[13].1, [crate::retail::pcctl::ID_PLAYSTATION, crate::retail::pcctl::ID_XBOX, crate::retail::pcctl::ID_SWITCH][want as usize]);
		}
		// The file names the style.
		assert!(std::fs::read_to_string(dirp.join("pc_options.txt")).unwrap().contains("glyphs=xbox\n"));
		// A fresh RAM takes the remembered style back.
		let o = PcOptions::load(&dirp.join("pc_options.txt"));
		assert_eq!(o.glyphs, Some(1));
		GLYPH_STYLE.store(1, Ordering::Relaxed);
	}

	/// The packed Graphics word: every level of every effect row round-trips, a word from before the levels existed (an "on" bit
	/// and no level) reads as High, and the effect rows have the right number of values.
	#[test]
	fn gfx_word_packs_levels_and_reads_the_old_packing() {
		let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		for lvl in 0..4u8 {
			for row in [1usize, 2, 4, 5, 6, 10] {
				let mut g = DEFAULT_ROWS;
				g[row] = lvl;
				g[3] = 0;
				assert_eq!(unpack_gfx(pack_gfx(g)), g, "row {row} level {lvl}");
			}
		}
		let all = [1, 2, 3, 0, 1, 2, 3, 1, 2, 1, 2, 1, 2];
		assert_eq!(unpack_gfx(pack_gfx(all)), all);
		for b in 0..3u8 {
			let mut g = DEFAULT_ROWS;
			g[12] = b;
			assert_eq!(unpack_gfx(pack_gfx(g)), g, "draw distance byte {b}");
			assert_eq!(pack_gfx(g) >> 30, b as u32);
		}
		// The old word: lighting 1, "on" bits for AO, shadows, water, particles, sun rays and smoothing, vegetation Lush, no levels.
		let old = 1 | 0x3f << 8 | 2 << 14;
		assert_eq!(unpack_gfx(old), [1, 1, 1, 0, 1, 1, 1, 1, 2, 0, 0, 0, 0], "old on = High = byte 1");
		// An old word with some rows off.
		assert_eq!(unpack_gfx(1 | 1 << 8 | 1 << 11 | 1 << 14), [1, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0]);
		// The new default packs to the same flags as the old default plus the High levels.
		assert_eq!(pack_gfx([0, 1, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0]) & 0xffff, pack_gfx(DEFAULT_ROWS) & 0xffff, "flag bits are unchanged");
		assert_eq!(pack_gfx(DEFAULT_ROWS) >> 16, 3 | 3 << 2 | 3 << 4 | 3 << 6 | 3 << 8, "the word holds levels: bytes of 1 are High = 3");
		assert_eq!([0u8, 1, 2, 3].map(swap_level), [0, 3, 2, 1]);
		let counts: Vec<u8> = (0..13).map(row_values).collect();
		assert_eq!(counts, [2, 4, 4, 2, 4, 4, 4, 2, 3, 2, 4, 2, 3]);
		// Every value of every row has a text, and the new level texts are in the pool.
		for r in ROWS {
			assert!(r[2..].iter().filter(|&&v| v != 0).all(|v| *v < 0x7f00 || NEW_IDS.contains(v)), "row {:#x}", r[0]);
		}
		let slots: std::collections::HashSet<u32> = NEW_IDS.iter().copied().collect();
		assert_eq!(slots.len(), NEW_IDS.len(), "no id twice");
		assert_eq!(NEW_IDS.len(), TEXTS.len());
		assert!(POOL + 0x40 * POOL_SLOTS as u32 <= OPT_RECS, "the string pool ends before the Graphics page copies");
		assert!(TGVAL + 0x40 <= TGTAB && TGTAB + 20 * (ROWS.len() as u32 + 2) <= POOL2 && GTAB + 24 * (ROWS.len() as u32 + 1) <= TRECS, "the row tables end before the next copies");
		assert!(pool_slot(NEW_IDS.len() - 1) + 0x40 <= crate::retail::prompts::POOL, "the second pool ends before prompts.rs");
	}

	/// The title's Options list set-up as the PAL boot ELF holds it (0x219ca8, region word 0x15ee90, lists 0x1d4790 / 0x1d47c0) with
	/// its Options, Video and Sound pages: the hook pattern matches it, PAL gains Graphics after Language, Video, Sound and USA after
	/// Language, Sound; the Graphics page is Video Options' with our items and Sound's value panel; the menu's word survives a
	/// re-entry; a page that is not the retail one leaves retail's list.
	#[test]
	fn title_graphics_page() {
		let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		let (pc, region, opt, video, sound, hdr, val, vtab) = (0x219ca8, 0x15ee90, 0x1d4708, 0x1d4a38, 0x1d4838, 0x1d4ac0, 0x1d4b40, 0x1d4b18);
		let fresh = |pal: bool| -> Mem {
			let mut m = Mem::new();
			let code = [0x3c02_0016, 0x8c42_ee90, 0x5040_0004, 0x3c02_001d, 0x3c02_001d, 0x1000_0002, 0x2442_4790, 0x2442_47c0, 0xac82_0034, 0x03e0_0008, 0x0000_102d];
			for (k, w) in code.iter().enumerate() {
				m.wr32(pc + 4 * k as u32, *w);
			}
			m.wr32(region, pal as u32);
			for (list, recs) in [(0x1d4790, &[(0x4f2b, 0x1d48c0), (FRONT_VIDEO, video), (FRONT_SOUND, sound)][..]), (0x1d47c0, &[(0x4f2b, 0x1d48c0), (FRONT_SOUND, sound)][..])] {
				for (i, &(id, page)) in recs.iter().enumerate() {
					m.wr32(list + 12 * i as u32, id | 3 << 16);
					m.wr32(list + 12 * i as u32 + 4, page);
				}
			}
			for (k, s) in [0, 1, 0xca, 0xcb, 0xc3].iter().enumerate() {
				m.wr32(video + 4 * k as u32, *s);
			}
			m.wr32(sound + 0xc, 0xd2);
			for (page, id) in [(video, 0x31), (sound, 0x32)] {
				m.wr32(page + 0x38, opt);
				m.wr32(page + 0x3c, id);
			}
			m.wr32(video + 0x40, val);
			for (k, it) in [hdr, 0x1d4a00, 0x1d4a10, val].iter().enumerate() {
				m.wr32(video + 0x44 + 4 * k as u32, *it);
			}
			m.wr32(hdr + 0x34, FRONT_VIDEO_HDR);
			m.wr32(val, 0x1234);
			m.wr32(val + 0x34, vtab);
			m.wr32(val + 0x38, 1);
			m.wr32(vtab, FRONT_VIDEO_ROW);
			m
		};
		let code_at = |m: &mut Mem| TITLE_CODE.iter().enumerate().all(|(k, &(v, mask))| m.rd32(pc + 4 * k as u32) & mask == v);
		let ids = |m: &mut Mem| -> Vec<u32> { (0..5).map(|i| m.rd32(TRECS + 12 * i)).collect() };
		let dirp = scratch("title");
		let mut s = St { opts: PcOptions::default(), path: Some(dirp.join("pc_options.txt")), last: None, glast: None, lang_seen: None, warned: false };
		let item = 0x1d47e8;

		let mut m = fresh(true);
		assert!(code_at(&mut m));
		assert_eq!(build_title(&mut m, pc, item, &mut s), Some(9));
		assert_eq!((m.rd32(item + 0x34), m.rd32(BASE)), (TRECS, MAGIC));
		assert_eq!(ids(&mut m), [0x34f2b, 0x34f21, 0x34f22, ID_GRAPHICS | 3 << 16, 0]);
		assert_eq!((m.rd32(TRECS + 4), m.rd32(TRECS + 16), m.rd32(TRECS + 28), m.rd32(TRECS + 40)), (0x1d48c0, video, sound, TGPAGE));
		// The page: Video's sequences with Sound's value panel, parent Options, id, current item and items ours.
		let seqs: Vec<u32> = (0..6).map(|k| m.rd32(TGPAGE + 4 * k)).collect();
		assert_eq!(seqs, [0, 1, 0xca, 0xd2, 0xc3, 0]);
		assert_eq!((m.rd32(TGPAGE + 0x38), m.rd32(TGPAGE + 0x3c), m.rd32(TGPAGE + 0x40)), (opt, 0x31, TGVAL));
		let items: Vec<u32> = (0..5).map(|k| m.rd32(TGPAGE + 0x44 + 4 * k)).collect();
		assert_eq!(items, [TGHDR, 0x1d4a00, 0x1d4a10, TGVAL, 0]);
		assert_eq!((m.rd32(TGHDR + 0x34), m.rd32(TGVAL), m.rd32(TGVAL + 0x34), m.rd32(TGVAL + 0x38)), (ID_GRAPHICS_HDR, 0x1234, TGTAB, 0));
		let row = |m: &mut Mem, i: u32| -> Vec<u32> { (0..5).map(|k| m.rd32(TGTAB + 20 * i + 4 * k)).collect() };
		assert_eq!(row(&mut m, 0), [ID_LIGHTING, LIGHT, ID_LIGHT_CLASSIC, ID_ENHANCED, 0]);
		for (i, r) in ROWS.iter().enumerate() {
			assert_eq!(row(&mut m, i as u32), [r[0], r[1], r[2], r[3], 0], "the title's value item flips between the first two values");
		}
		assert_eq!(row(&mut m, 12), [ID_DRAW, DRAW, ID_MEDIUM, ID_HIGH, 0]);
		assert_eq!(row(&mut m, 13), [crate::retail::pcctl::ID_GLYPHS, GLYPHS, crate::retail::pcctl::ID_PLAYSTATION, crate::retail::pcctl::ID_XBOX, 0], "the Button glyphs row closes the page");
		assert_eq!(row(&mut m, 14), [0; 5]);
		// Re-entry: the list's runtime word stays, the page is rebuilt (the cursor back on the first row).
		m.wr32(TRECS + 8, 0x77);
		m.wr32(TGVAL + 0x38, 3);
		assert_eq!(build_title(&mut m, pc, item, &mut s), Some(9));
		assert_eq!((m.rd32(TRECS + 8), m.rd32(TGVAL + 0x38)), (0x77, 0));

		// USA (region word 0): Language, Sound, Graphics.
		let mut m = fresh(false);
		assert_eq!(build_title(&mut m, pc, item, &mut s), Some(8));
		assert_eq!(ids(&mut m), [0x34f2b, 0x34f22, ID_GRAPHICS | 3 << 16, 0, 0]);
		assert_eq!(m.rd32(TGPAGE + 0xc), 0xd2);

		// Not the retail Video page (header text), a fourth row, or a row that is not a page: retail's list stays.
		for bad in 0..3 {
			let mut m = fresh(true);
			match bad {
				0 => m.wr32(hdr + 0x34, 0x4f3b),
				1 => m.wr32(0x1d4790 + 36, 0x34f2c),
				_ => m.wr32(0x1d4790, 0x14f2b),
			}
			m.wr32(item + 0x34, 0x1d4790);
			assert_eq!(build_title(&mut m, pc, item, &mut s), None, "case {bad}");
			assert_eq!((m.rd32(item + 0x34), m.rd32(TRECS)), (0x1d4790, 0), "case {bad}");
		}
		let _ = std::fs::remove_dir_all(&dirp);
		GFX.store(pack_gfx(DEFAULT_ROWS), Ordering::Relaxed);
		FULLSCREEN.store(false, Ordering::Relaxed);
	}
}
