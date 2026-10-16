//! PC options in the game's own pause menu (Pause -> Options -> Camera Options, retail page id 0x1d): two extra rows in the
//! retail look, remembered between sessions (`pcopts.rs`, `<player dir>/pc_options.txt`).
//!
//! - `Camera: Classic / Modern`: Modern = the mouse-driven modern camera (modcam.rs), Classic = retail's camera and controls.
//!   A new player starts with Modern.
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
//!   Classic / Enhanced, Ambient occlusion and Dynamic shadows On / Off, Display mode Windowed / Fullscreen, Water and Particles
//!   Classic / Enhanced, Sun rays Off / On (`graphics`, viewer/post.rs); a new
//!   player sees the retail picture (Original). The picture of the row is help_ss 5 (the lit planet), from the same ToC.
//!
//! `RAC_PC_EXT=0` registers nothing and writes nothing (today's bit-exact path); the oracle's gates never link this file.
use super::pcopts::{self, Lang, PcOptions};
use super::{modcam, FieldIn};
use ee_oracle::cpu::Cpu;
use ee_oracle::mem::Mem;
use ee_oracle::rec::{register_hand, Hand};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
/// Graphics -> Ambient occlusion: 0 Off, 1 On.
pub const AO: u32 = BASE + 9;
/// Graphics -> Dynamic shadows: 0 Off, 1 On.
pub const SHADOWS: u32 = BASE + 10;
/// Graphics -> Display mode: 0 Windowed, 1 Fullscreen (borderless, viewer/handler.rs `apply_display`).
pub const DISPLAY: u32 = BASE + 11;
/// Graphics -> Water: 0 Classic, 1 Enhanced (viewer/waterfx.rs; with Lighting Enhanced).
pub const WATER: u32 = BASE + 12;
/// Graphics -> Particles: 0 Classic, 1 Enhanced (with Lighting Enhanced).
pub const PARTICLES: u32 = BASE + 13;
/// Graphics -> Sun rays: 0 Off, 1 On (with Lighting Enhanced).
pub const SUNRAYS: u32 = BASE + 14;
/// The Graphics page's row bytes in row order.
const GROWS: [u32; 7] = [LIGHT, AO, SHADOWS, DISPLAY, WATER, PARTICLES, SUNRAYS];
/// The copy of the row table (5 records + the zero record).
const TAB: u32 = BASE + 0x40;
/// String pool: one 0x40-byte slot per new text (`NEW_IDS` order; 22 slots end at BASE + 0x680).
const POOL: u32 = BASE + 0x100;
/// Graphics page (`patch_graphics`): the Options list records (8 + the zero record, 12 bytes each), its row descriptions (8 + 0),
/// its row pictures (8 ToC records {sector, sectors}), the page (a copy of Camera Options; 0x100 bytes, zero after its 0x60:
/// the menu engine reads item slots past the page's zero item, retail's own pages are followed by zeros there), its header
/// item, its multi-choice item and that item's rows (7 + the zero record, 24 bytes each, ending at BASE + 0xb60; prompts.rs starts at
/// BASE + 0x1000).
const OPT_RECS: u32 = BASE + 0x800;
const OPT_DESC: u32 = BASE + 0x880;
const OPT_PICS: u32 = BASE + 0x8c0;
pub const GPAGE: u32 = BASE + 0x900;
const GHDR: u32 = BASE + 0xa00;
pub const GVAL: u32 = BASE + 0xa60;
const GTAB: u32 = BASE + 0xaa0;
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
/// Enhanced for a feminine row (Water: l'eau, el agua, l'acqua; Classic is the Camera row's feminine 0x7f01).
pub const ID_ENHANCED_F: u32 = 0x7f14;
/// Particles' values (plural: les particules, los efectos, le particelle).
pub const ID_PART_CLASSIC: u32 = 0x7f15;
pub const ID_PART_ENHANCED: u32 = 0x7f16;
const NEW_IDS: [u32; 22] = [
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
	ID_ENHANCED_F,
	ID_PART_CLASSIC,
	ID_PART_ENHANCED,
];

/// New texts per language (En, Fr, De, Es, It) in the game's font bytes: a-acute 0x84, e-acute 0x8c, e-grave 0x8d, a-umlaut 0x87,
/// o-acute 0x9c, e-circumflex 0x8e. Short like the retail labels (the German ones are abbreviated to fit). [I] wording, native review wanted. The
/// Graphics header and description follow the retail Camera Options ones ("Options cam\x8cra", "Adjust camera control settings").
static TEXTS: [[&[u8]; 5]; 22] = [
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
	[b"Enhanced", b"Am\x8clior\x8ce", b"Verbessert", b"Mejorada", b"Migliorata"],
	[b"Classic", b"Classiques", b"Klassisch", b"Cl\x84sicos", b"Classiche"],
	[b"Enhanced", b"Am\x8clior\x8ces", b"Verbessert", b"Mejorados", b"Migliorate"],
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

/// The Graphics rows as last applied (`pack_gfx`; default Classic, the rest On / Enhanced for when the player picks Enhanced).
static GFX: AtomicU32 = AtomicU32::new(pack_gfx([0, 1, 1, 0, 1, 1, 1]));
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
/// other rows only act with Lighting Enhanced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Graphics {
	/// 0 Classic (the retail picture), 1 Enhanced.
	pub lighting: u8,
	pub ao: bool,
	pub shadows: bool,
	/// Water Enhanced (viewer/waterfx.rs).
	pub water: bool,
	/// Particles Enhanced.
	pub particles: bool,
	pub sun_rays: bool,
}

impl Graphics {
	pub const CLASSIC: Graphics = Graphics { lighting: 0, ao: false, shadows: false, water: false, particles: false, sun_rays: false };

	/// Enhanced: the post-processing runs.
	pub fn enhanced(self) -> bool {
		self.lighting != 0
	}
}

/// The row bytes (`GROWS` order) as one word: lighting in bits 0..8, then one bit per On / Enhanced row (display mode excluded:
/// `FULLSCREEN`).
const fn pack_gfx(g: [u8; 7]) -> u32 {
	g[0] as u32 | ((g[1] == 1) as u32) << 8 | ((g[2] == 1) as u32) << 9 | ((g[4] == 1) as u32) << 10 | ((g[5] == 1) as u32) << 11 | ((g[6] == 1) as u32) << 12
}

/// The Graphics page's verdict (render thread, every frame). Classic with the extension off (`RAC_PC_EXT=0`, the smoke path).
pub fn graphics() -> Graphics {
	if !enabled() {
		return Graphics::CLASSIC;
	}
	let g = GFX.load(Ordering::Relaxed);
	Graphics { lighting: (g & 0xff).min(1) as u8, ao: g >> 8 & 1 != 0, shadows: g >> 9 & 1 != 0, water: g >> 10 & 1 != 0, particles: g >> 11 & 1 != 0, sun_rays: g >> 12 & 1 != 0 }
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
	glast: Option<[u8; 7]>,
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
		return;
	}
	let path = pcopts::file();
	let opts = PcOptions::load(&path);
	eprintln!(
		"retail: PC options {}: camera {}, strafe aiming {}, language {}, lighting {}, ambient occlusion {}, dynamic shadows {}",
		path.display(),
		if opts.camera_modern() { "Modern" } else { "Classic" },
		if opts.strafe_on() { "On" } else { "Off" },
		opts.lang.map_or("(not chosen)", |l| l.code()),
		pcopts::LIGHTING[opts.lighting_or_default() as usize],
		if opts.ao_on() { "On" } else { "Off" },
		if opts.shadows_on() { "On" } else { "Off" }
	);
	if cold && std::env::var_os("COLD_LANG").is_none() {
		if let Some(l) = opts.lang {
			ee_oracle::cold::set_language(l.osd());
			eprintln!("retail: PC options: cold boot in the remembered language ({})", l.code());
		}
	}
	CAM_MODERN.store(opts.camera_modern(), Ordering::Relaxed);
	STRAFE_ON.store(opts.strafe_on(), Ordering::Relaxed);
	BUYALL_ON.store(opts.buyall_on(), Ordering::Relaxed);
	GFX.store(pack_gfx(opts.graphics_rows()), Ordering::Relaxed);
	FULLSCREEN.store(opts.fullscreen_on(), Ordering::Relaxed);
	modcam::set_active(opts.camera_modern());
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
	}
	out
}

/// `pre_field` on the memory: true when the F7 press was consumed.
fn sync(mem: &mut Mem, level: u32, toggle: bool, s: &mut St) -> bool {
	if mem.rd32(BASE) != MAGIC {
		// Fresh RAM (start, savestate load, cold boot): the options file's values go in.
		mem.wr32(BASE, MAGIC);
		let (c, t, b) = (s.opts.camera_modern() as u8, s.opts.strafe_on() as u8, s.opts.buyall_on() as u8);
		mem.wr8(CAM, c);
		mem.wr8(STRAFE, t);
		mem.wr8(BUYALL, b);
		s.last = Some((c, t, b));
		apply(s, c, t, b, false);
		let g = s.opts.graphics_rows();
		for (a, v) in GROWS.iter().zip(g) {
			mem.wr8(*a, v);
		}
		s.glast = Some(g);
		apply_graphics(s, g, false);
	}
	if toggle {
		let c = mem.rd8(CAM);
		mem.wr8(CAM, (c == 0) as u8);
		eprintln!("retail: F7: camera {}", if c == 0 { "Modern" } else { "Classic" });
	}
	let (c, t, b) = (mem.rd8(CAM), mem.rd8(STRAFE), mem.rd8(BUYALL));
	if s.last != Some((c, t, b)) {
		s.last = Some((c, t, b));
		apply(s, c, t, b, true);
	}
	let mut g = GROWS.map(|a| mem.rd8(a));
	g[0] = g[0].min(1);
	if s.glast != Some(g) {
		s.glast = Some(g);
		apply_graphics(s, g, true);
	}
	patch_rows(mem, level);
	patch_graphics(mem, level);
	toggle
}

/// A Graphics byte changed (or the region was set up): the renderer follows (`graphics`); a change is remembered.
fn apply_graphics(s: &mut St, g: [u8; 7], changed: bool) {
	GFX.store(pack_gfx(g), Ordering::Relaxed);
	FULLSCREEN.store(g[3] == 1, Ordering::Relaxed);
	if changed {
		let on = |v: u8| if v == 1 { "On" } else { "Off" };
		let enh = |v: u8| if v == 1 { "Enhanced" } else { "Classic" };
		eprintln!(
			"retail: PC options: lighting {}, ambient occlusion {}, dynamic shadows {}, display {}, water {}, particles {}, sun rays {}",
			pcopts::LIGHTING[g[0] as usize],
			on(g[1]),
			on(g[2]),
			if g[3] == 1 { "Fullscreen" } else { "Windowed" },
			enh(g[4]),
			enh(g[5]),
			on(g[6])
		);
		s.opts.set_graphics_rows(g);
		s.save();
	}
}

/// A row byte changed (or the region was set up): the camera hands and the strafe gate follow; a change is remembered.
fn apply(s: &mut St, cam: u8, strafe: u8, buyall: u8, changed: bool) {
	let (cam, strafe, buyall) = (cam == 1, strafe == 1, buyall == 1);
	BUYALL_ON.store(buyall, Ordering::Relaxed);
	CAM_MODERN.store(cam, Ordering::Relaxed);
	STRAFE_ON.store(strafe, Ordering::Relaxed);
	modcam::set_active(cam);
	if changed {
		eprintln!("retail: PC options: camera {}, strafe aiming {}", if cam { "Modern" } else { "Classic" }, if strafe { "On" } else { "Off" });
		s.opts.camera = Some(cam);
		s.opts.strafe = Some(strafe);
		s.opts.buyall = Some(buyall);
		s.save();
	}
}

/// The Camera Options table of `level`, when the retail one is there: builds the five-record copy and repoints the item to it.
/// Idempotent: a second look finds the copy's address, not the table's. Guards: the level has a row, the item points at the
/// table and the table starts with the retail signature, so another overlay (loading, other level) is never touched.
fn patch_rows(mem: &mut Mem, level: u32) -> bool {
	let Some(&t) = (*crate::retail::disc::pick(&TABLES, &TABLES_USA)).get(level as usize) else { return false };
	if mem.rd32(t + PTR_OFF) != t || mem.rd32(t) != SIG[0] || mem.rd32(t + 4) != SIG[1] {
		return false;
	}
	// The three retail records, then Camera and Strafe, then the zero record.
	let mut w: Vec<u32> = (0..18).map(|i| mem.rd32(t + 4 * i)).collect();
	w.extend([ID_CAMERA, CAM, ID_CLASSIC, ID_MODERN, 0, 0]);
	w.extend([ID_STRAFE, STRAFE, ID_OFF, ID_ON, 0, 0]);
	w.extend([ID_BUYALL, BUYALL, ID_OFF, ID_ON, 0, 0]);
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
	// The list rows and their descriptions with Graphics before Quit Game.
	let (recs, descs) = (mem.rd32(list + 0x34), mem.rd32(text + 0x34));
	let mut rows: Vec<[u32; 3]> = (0..7).map(|i| [mem.rd32(recs + 12 * i), mem.rd32(recs + 12 * i + 4), mem.rd32(recs + 12 * i + 8)]).collect();
	rows.insert(GRAPHICS_ROW, [ID_GRAPHICS | 3 << 16, GPAGE, 0]);
	rows.push([0; 3]);
	let mut desc: Vec<u32> = (0..7).map(|i| mem.rd32(descs + 4 * i)).collect();
	desc.insert(GRAPHICS_ROW, ID_GRAPHICS_DESC);
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
	// The value panel (slot 3) plays the Sound page's slot-3 sequence: its rect (53,127,406,164) holds the 7 rows at a wider
	// pitch than Camera Options' (69,176,381,91), since the item spaces rows at height / (rows + 1) (title_assets.md
	// "Page rect table"). The Sound page is the Options row with text 0x4f22 (type 3, arg = page, parent = Options).
	if let Some(sound) = rows.iter().find(|r| r[0] == 0x4f22 | 3 << 16).map(|r| r[1]).filter(|&p| (0x10_0000..0x200_0000).contains(&p) && mem.rd32(p + 0x38) == opt) {
		let seq = mem.rd32(sound + 0xc);
		mem.wr32(GPAGE + 0xc, seq);
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
	#[rustfmt::skip]
	let tab = [
		ID_LIGHTING, LIGHT, ID_LIGHT_CLASSIC, ID_ENHANCED, 0, 0,
		ID_AO, AO, ID_OFF, ID_ON, 0, 0,
		ID_SHADOWS, SHADOWS, ID_OFF, ID_ON, 0, 0,
		ID_DISPLAY, DISPLAY, ID_WINDOWED, ID_FULLSCREEN, 0, 0,
		ID_WATER, WATER, ID_CLASSIC, ID_ENHANCED_F, 0, 0,
		ID_PARTICLES, PARTICLES, ID_PART_CLASSIC, ID_PART_ENHANCED, 0, 0,
		ID_SUNRAYS, SUNRAYS, ID_OFF, ID_ON, 0, 0,
		0, 0, 0, 0, 0, 0,
	];
	for (i, w) in tab.iter().enumerate() {
		mem.wr32(GTAB + 4 * i as u32, *w);
	}
	mem.wr32(text + 0x34, OPT_DESC);
	mem.wr32(pic + 0x30, OPT_PICS);
	mem.wr32(list + 0x34, OPT_RECS);
	true
}

/// The text lookup hand: the three new ids get their string for the game's language; every other id goes to `prompts::lookup`
/// (keyboard prompts; it declines with a gamepad, so retail runs).
fn text_hook(c: &mut Cpu) -> bool {
	let id = c.g(4) as u32;
	// Not one of ours: the keyboard's button prompts (prompts.rs) or retail's own lookup.
	let Some(k) = NEW_IDS.iter().position(|&n| n == id) else { return super::prompts::lookup(c) };
	let lang = Lang::from_word(c.mem.rd32(LANG_WORD)).map_or(0, |l| l as usize);
	let at = POOL + 0x40 * k as u32;
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

	/// No swarm hand sits on one of the text lookups (the PC hand would replace it; its decline path runs the retail code).
	#[test]
	fn no_hand_of_the_swarm_on_a_text_lookup() {
		ee_game::register();
		let n: Vec<_> = ee_oracle::rec::hands().into_iter().filter(|h| HANDS.iter().any(|p| p.entry == h.entry && !std::ptr::eq(*h, p))).map(|h| h.entry).collect();
		eprintln!("hands on a text lookup entry besides ours: {n:x?}");
		assert!(n.is_empty(), "a hand of ee-hand sits on a text lookup: {n:x?}");
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
			assert!((POOL..POOL + 0x140).contains(&p) || id < 0x7f00, "pool pointer for {id:#x}: {p:#x}");
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
		assert_eq!((m.cpu.mem.rd8(CAM), m.cpu.mem.rd8(STRAFE), m.cpu.mem.rd8(BUYALL)), (1, 1, 1), "new player: Modern, strafe On, buy all On");
		let rec = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..6).map(|k| mem.rd32(TAB + 24 * i + 4 * k)).collect() };
		assert_eq!(rec(&mut m.cpu.mem, 0), [0x4fa2, 0x15ede0, 0x4fa5, 0x4fa4, 0, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 3), [ID_CAMERA, CAM, ID_CLASSIC, ID_MODERN, 0, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 4), [ID_STRAFE, STRAFE, ID_OFF, ID_ON, 0, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 5), [ID_BUYALL, BUYALL, ID_OFF, ID_ON, 0, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 6), [0; 6]);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		assert_eq!(m.cpu.mem.rd32(t + PTR_OFF), TAB);
		assert!(!dirp.join("pc_options.txt").exists(), "nothing is written until the player changes something");
		// The player flips Camera to Classic in the menu (the retail update writes the byte): applied, remembered, strafe follows.
		m.cpu.mem.wr8(CAM, 0);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		assert!(!CAM_MODERN.load(Ordering::Relaxed));
		assert_eq!(PcOptions::load(&dirp.join("pc_options.txt")).camera, Some(false));
		// F7 flips it back and is consumed.
		assert!(sync(&mut m.cpu.mem, 4, true, &mut s));
		assert_eq!(m.cpu.mem.rd8(CAM), 1);
		assert!(CAM_MODERN.load(Ordering::Relaxed));
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
		let ids: Vec<u32> = (0..9).map(|i| mem.rd32(OPT_RECS + 12 * i) & 0xffff).collect();
		assert_eq!(ids, [0x4f1e, 0x4f1f, 0x4f20, 0x4f22, 0x4f23, 0x4f24, ID_GRAPHICS, 0x4f26, 0]);
		assert_eq!((mem.rd32(OPT_RECS + 72) >> 16, mem.rd32(OPT_RECS + 76)), (3, GPAGE), "type 3: go to the Graphics page");
		let desc: Vec<u32> = (0..9).map(|i| mem.rd32(OPT_DESC + 4 * i)).collect();
		assert_eq!(desc, [0x4f31, 0x4f32, 0x4f33, 0x4f35, 0x4f36, 0x4f37, ID_GRAPHICS_DESC, 0x4f39, 0]);
		// Pictures: the retail seven around help_ss 5 (the record just below options_ss 0 is help_ss 6).
		assert_eq!(mem.rd32(OPT_PICS + 8 * 7), mem.rd32(0x138040 + 8 * 6));
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
		assert_ne!(want[3], mem.rd32(cp + 0xc));
		assert_eq!(got, want);
		let items: Vec<u32> = (0..6).map(|k| mem.rd32(GPAGE + 0x44 + 4 * k)).collect();
		let cam: Vec<u32> = (0..6).map(|k| mem.rd32(cp + 0x44 + 4 * k)).collect();
		assert_eq!(items, [GHDR, cam[1], cam[2], GVAL, cam[4], 0]);
		assert_eq!((mem.rd32(GHDR + 0x34), mem.rd32(GVAL + 0x34), mem.rd32(GVAL)), (ID_GRAPHICS_HDR, GTAB, mem.rd32(t + 0x60)));
		let row = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..6).map(|k| mem.rd32(GTAB + 24 * i + 4 * k)).collect() };
		assert_eq!(row(mem, 0), [ID_LIGHTING, LIGHT, ID_LIGHT_CLASSIC, ID_ENHANCED, 0, 0]);
		assert_eq!(row(mem, 2), [ID_SHADOWS, SHADOWS, ID_OFF, ID_ON, 0, 0]);
		assert_eq!(row(mem, 3), [ID_DISPLAY, DISPLAY, ID_WINDOWED, ID_FULLSCREEN, 0, 0]);
		assert_eq!(row(mem, 4), [ID_WATER, WATER, ID_CLASSIC, ID_ENHANCED_F, 0, 0]);
		assert_eq!(row(mem, 5), [ID_PARTICLES, PARTICLES, ID_PART_CLASSIC, ID_PART_ENHANCED, 0, 0]);
		assert_eq!(row(mem, 6), [ID_SUNRAYS, SUNRAYS, ID_OFF, ID_ON, 0, 0]);
		assert_eq!(row(mem, 7), [0; 6]);
		assert_eq!(GROWS.map(|a| mem.rd8(a)), [0, 1, 1, 0, 1, 1, 1], "new player: Classic, AO / shadows On, Windowed, the rest Enhanced / On");
		assert!(!fullscreen());
		assert_eq!(graphics(), Graphics { lighting: 0, ao: true, shadows: true, water: true, particles: true, sun_rays: true });
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
		assert_eq!((o.lighting, o.ao, o.shadows), (Some(1), Some(true), Some(true)));
		assert_eq!(o.camera, None, "a Graphics change writes the Graphics keys only");
		// Water Classic, Sun rays Off: the renderer and the file follow.
		mem.wr8(WATER, 0);
		mem.wr8(SUNRAYS, 0);
		sync(mem, 4, false, &mut s);
		let g = graphics();
		assert!(!g.water && g.particles && !g.sun_rays);
		let o = PcOptions::load(&dirp.join("pc_options.txt"));
		assert_eq!((o.water, o.particles, o.sun_rays), (Some(false), Some(true), Some(false)));
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
		GFX.store(pack_gfx([0, 1, 1, 0, 1, 1, 1]), Ordering::Relaxed);
		FULLSCREEN.store(false, Ordering::Relaxed);
	}
}
