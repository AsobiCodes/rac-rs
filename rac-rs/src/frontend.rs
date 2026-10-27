//! The front end ("level -1"): title scene, logo, Press Start and the page menu
//! (decomp/notes/frontend_flow.md, title_assets.md).

use crate::bytes::{span, u32_at, R};
use crate::iso::Disc;
use crate::level::{Level, Texture};
use crate::overlay::Quad;
use crate::settings::Settings;
use crate::{hud, toc, wad};
use std::collections::HashMap;

/// ToC entry of the global string table (all_text): per-language blocks.
const ALL_TEXT: usize = 0x1528;
/// Block per language slot: 0 English, 1 unused, 2 French, 3 German, 4 Spanish, 5 Italian.
const TEXT_SLOTS: [usize; 5] = [0, 2, 3, 4, 5];

/// Strings by id for one language. Block: (count, size) then 16-byte entries (text offset, id, ...).
#[derive(Default)]
pub struct Text(HashMap<u32, String>);

impl Text {
	pub fn load(disc: &mut Disc, lang: usize) -> R<Text> {
		let toc = toc::read_toc(disc).map_err(|e| e.to_string())?;
		let raw = disc.read_sectors(u32_at(&toc, ALL_TEXT)? as u64, u32_at(&toc, ALL_TEXT + 4)? as u64).map_err(|e| e.to_string())?;
		let file = if wad::is_wad(&raw) { wad::decompress(&raw).map_err(|e| format!("WAD: {e}"))? } else { raw };
		Text::parse(&file, lang)
	}

	pub fn parse(file: &[u8], lang: usize) -> R<Text> {
		let start = u32_at(file, TEXT_SLOTS.get(lang).copied().ok_or("no such language")? * 4)? as usize;
		let count = u32_at(file, start)? as usize;
		let size = u32_at(file, start + 4)? as usize;
		let block = span(file, start, size)?;
		let mut out = HashMap::new();
		for e in span(block, 8, count * 16)?.chunks_exact(16) {
			let raw = block.get(u32_at(e, 0)? as usize..).ok_or("text outside its block")?;
			let raw = &raw[..raw.iter().position(|&b| b == 0).unwrap_or(raw.len())];
			out.insert(u32_at(e, 4)?, raw.iter().map(|&b| b as char).collect());
		}
		Ok(Text(out))
	}

	pub fn get(&self, id: u32) -> &str {
		self.0.get(&id).map_or("", String::as_str)
	}

	/// Adds or replaces a string (the PC options' own texts, retail/pcext.rs).
	pub fn insert(&mut self, id: u32, s: String) {
		self.0.insert(id, s);
	}
}

/// Fx texture record `k` of a level's index table 0x58 (16 B: palette, pixels, w, h), PSMT8.
/// Number of records in the level's fx texture table (index +0x58).
pub fn fx_count(level: &Level) -> usize {
	u32_at(&level.index, 0x58).map_or(0, |n| n as usize)
}

pub fn fx_texture(level: &Level, k: usize) -> R<Texture> {
	let count = u32_at(&level.index, 0x58)? as usize;
	let table = u32_at(&level.index, 0x5c)? as usize;
	let base = u32_at(&level.index, 0x68)? as usize;
	if k >= count {
		return Err(format!("no fx texture {k}"));
	}
	let r = span(&level.index, table + k * 16, 16)?;
	let (pal, pix, w, h) = (u32_at(r, 0)? as usize, u32_at(r, 4)? as usize, u32_at(r, 8)? as usize, u32_at(r, 12)? as usize);
	let colours = hud::palette_rgba(span(&level.core, base + pal, 1024)?);
	let px = span(&level.core, base + pix, w * h)?;
	Ok(Texture { w, h, rgba: px.iter().map(|&v| colours[v as usize]).collect() })
}

/// The title logo: 256x128 PSMCT32 at core + index[0x84] (alpha 0x80 = opaque).
pub fn logo(level: &Level) -> R<Texture> {
	let at = u32_at(&level.index, 0x84)? as usize;
	let px = span(&level.core, at, 256 * 128 * 4)?;
	Ok(Texture { w: 256, h: 128, rgba: px.chunks_exact(4).map(|c| [c[0], c[1], c[2], (c[3] as u16 * 2).min(255) as u8]).collect() })
}

/// Press Start, baked per language: fx records 4..8 (EN, FR, DE, ES, IT).
pub fn press_start(level: &Level, lang: usize) -> R<Texture> {
	fx_texture(level, 4 + lang.min(4))
}

/// Front-end screen coordinates are the PS2's 512x448.
const SCREEN: [f32; 2] = [512.0, 448.0];
/// Menu panel mapping from the class-0x472 joints (depth 6, y right, z up) to screen pixels;
/// the game projects them with its menu camera (FUN_00237588; decomp/notes/ingame_pause_menu.md "Menu camera").
/// Root page button rects (x, y, w, h) on the PAL 512x448 screen: class 0x472 seqs 0x10..0x12 at rest
/// (frame 0), joints projected by FUN_00237588 with the fixed menu view (cd30 0.63, PAL aspect 0.756,
/// depth 6; notes/title_assets.md "Panel projection").
const ROOT_PANELS: [[f32; 4]; 3] = [[155.0, 153.0, 203.0, 39.0], [155.0, 207.0, 203.0, 39.0], [155.0, 262.0, 203.0, 39.0]];
/// Text ids (all_text.bin).
const NEW_GAME: u32 = 0x4f2a;
const LOAD_GAME: u32 = 0x4f28;
const OPTIONS: u32 = 0x4ee7;
const LANGUAGE: u32 = 0x4f2b;
const VIDEO: u32 = 0x4f21;
/// Video page (0x1b8bb8, decomp/notes/ingame_pause_menu.md): title "Video Options", one toggle row
/// "Video mode (50Hz or 60Hz)" showing "50Hz" / "60Hz".
const VIDEO_OPTIONS: u32 = 0x4f3a;
const VIDEO_MODE: u32 = 0x4f3b;
const HZ50: u32 = 0x4f3c;
const HZ60: u32 = 0x4f3d;
const SOUND: u32 = 0x4f22;
const LANGUAGES: u32 = 0x4f2c;
const SELECT_FILE: u32 = 0x4f27;
const NO_SAVE: u32 = 0x5232;
const EMPTY_SLOT: u32 = 0x5217;
/// Colours (PS2 ABGR, 0x80 alpha = opaque): selectable text 0x80ffa888, selected 0x8020ffff.
const UNSELECTED: [f32; 4] = [0x88 as f32 / 255.0, 0xa8 as f32 / 255.0, 1.0, 1.0];
const SELECTED: [f32; 4] = [1.0, 1.0, 0x20 as f32 / 255.0, 1.0];
/// Panel fill 0x80100808. The game frames it with sprites fx 0x19..0x1c; `draw_frame` instead
/// paints a capture-matched gold band and fill [I], the real sprite frame is not used [P].
/// Frame underlay slot meaning "no HUD sprite" (title level): a flat PANEL fill instead [P].
pub const NO_UNDERLAY: usize = usize::MAX;
const PANEL: [f32; 4] = [8.0 / 255.0, 8.0 / 255.0, 16.0 / 255.0, 0.8];
/// FUN_00223928 OV 0x223af8: the 0x1c scanline sprite's vertex colour 0x50606060 (RGB 0x60, A 0x50, 0x80 = 1).
pub const SCANLINE_TINT: [f32; 4] = [0.75, 0.75, 0.75, 0.625];
/// Panel colours sampled from retail PCSX2 captures (gamma space) [I].
const PANEL_FILL: [f32; 4] = [18.0 / 255.0, 18.0 / 255.0, 26.0 / 255.0, 1.0];
const GOLD_DARK: [f32; 4] = [60.0 / 255.0, 58.0 / 255.0, 42.0 / 255.0, 1.0];
const GOLD: [f32; 4] = [110.0 / 255.0, 108.0 / 255.0, 82.0 / 255.0, 1.0];
const GOLD_HI: [f32; 4] = [155.0 / 255.0, 155.0 / 255.0, 120.0 / 255.0, 1.0];
/// Font texels are 0x80 (sampled as 128/255) and the frontend colours are bytes / 255, so the GS
/// modulate (`texel * vertex >> 7`) needs 255 / 128 to make each byte act as byte / 128.
const TEXT_GAIN: f32 = 255.0 / 128.0;
/// Highlight colour fade (gp9574 = 10 ticks).
const FADE_FRAMES: f32 = 10.0;

/// In-game pause pages (decomp/notes/ingame_pause_menu.md s1, ingame_menu_leftovers.md): text ids.
const WEAPONS: u32 = 0x4ee2;
const GADGETS: u32 = 0x4ee3;
const QUICK_SELECT: u32 = 0x4ee4;
const ITEMS: u32 = 0x4ee5;
const HELP: u32 = 0x4ee6;
const GOODIES: u32 = 0x4fc2;
const GADGET_LISTS: [u32; 4] = [0x4ee8, 0x4ee9, 0x4eea, 0x4eeb];
const EQUIP: u32 = 0x4eec;
/// Button legends, labels and stats of the pause pages (all_text; layout from retail PCSX2 captures [I]).
const EXIT: u32 = 0x4ee0;
const SELECT: u32 = 0x4f05;
const VIEW_MISSIONS: u32 = 0x4eee;
const PLAY_INFOBOT: u32 = 0x4eef;
const PREV_MAP: u32 = 0x4efa;
const NEXT_MAP: u32 = 0x4efb;
const TRACK_ZOOM: u32 = 0x4efc;
/// The Items grid (V 0x1b5e00, 10-byte cells {icon, frame, kind, id, 0}; labels 0x1b6008, descriptions
/// 0x1b5f88), row by row: (icon, frame, kind, id, label, description). Kind 0 = owned item (0x13d4c0), 1 =
/// collectible byte 0x13d388 + id (decomp/notes/ingame_pause_menu.md).
pub const ITEM_GRID: [(u16, u8, u8, u8, u32, u32); 9] = [
	(0xea7e, 0, 0, 0x1e, 0x4e69, 0x4f44),
	(0xeac9, 0, 0, 0x23, 0x4e73, 0x4f45),
	(0xeac9, 2, 0, 0x22, 0x4e78, 0x4f46),
	(0xeac9, 3, 1, 4, 0x4e71, 0x4f47),
	(0xeac9, 4, 1, 5, 0x4e72, 0x4f48),
	(0xeac9, 1, 0, 0x21, 0x4e70, 0x4f49),
	(0xeac8, 0, 1, 0, 0x4e74, 0x4f4a),
	(0xeac8, 1, 1, 1, 0x4e75, 0x4f4b),
	(0xeac8, 2, 1, 2, 0x4e77, 0x4f4c),
];
const GOLD_BOLTS: u32 = 0x4f4e;
/// Found:, Used:, Remain:.
const GOLD_STATS: [u32; 3] = [0x4f4f, 0x4f50, 0x4f51];
const NO_AMMO: u32 = 0x4f4d;
/// The Help and in-game Options pages' description of the selected row (left panel).
const HELP_INFO: [u32; 5] = [0x4fd6, 0x4efd, 0x4efe, 0x4eff, 0x4f00];
const GAME_OPTION_INFO: [u32; 7] = [0x4f31, 0x4f32, 0x4f33, 0x4f35, 0x4f36, 0x4f37, 0x4f39];
/// Icon grid cells of the Weapons / Quick Select / Items lists, 5 per row (retail capture) [I].
const CELL: [f32; 2] = [41.0, 38.0];
/// Quick-select slot squares and selection outline: retail capture colours [I].
const SEL_EDGE: [f32; 4] = [190.0 / 255.0, 175.0 / 255.0, 80.0 / 255.0, 1.0];
/// Capture-tuned (no retail byte mapped) [I]: 0.76, 0.49, 0.27 under the old gain 1.6, times 1.6 * 128 / 255.
const GOLD_TEXT: [f32; 4] = [0.6103843, 0.39353728, 0.21684706, 1.0];
const HELP_MENU: u32 = 0x4f5e;
const HELP_LOG: u32 = 0x4fd5;
const CONTROLS: u32 = 0x4ef1;
const MOVES: u32 = 0x4ef2;
const WEAPONS_HELP: u32 = 0x4f99;
const GADGETS_HELP: u32 = 0x4f9a;
/// The Moves page list (13 entries).
const MOVE_LIST: [u32; 13] = [0x4e8c, 0x4e85, 0x4e88, 0x4f06, 0x4f07, 0x4f08, 0x4f09, 0x4f0a, 0x4f0b, 0x4f0c, 0x4e87, 0x4e86, 0x4e8d];
const GAME_OPTIONS: u32 = 0x4f1d;
/// In-game Options entries: HelpDesk, Save, Load, Sound, Camera, Subtitles, Quit Game.
const GAME_OPTION_LIST: [u32; 7] = [0x4f1e, 0x4f1f, 0x4f20, 0x4f22, 0x4f23, 0x4f24, 0x4f26];
const HELPDESK_OPTIONS: u32 = 0x4f40;
const VOICE: u32 = 0x4f9f;
const TEXT_ON: u32 = 0x4fa0;
const ON: u32 = 0x4f5a;
const OFF: u32 = 0x4f5b;
const SAVE_GAME: u32 = 0x4f29;
const OVERWRITE: u32 = 0x4fb3;
const SOUND_OPTIONS: u32 = 0x4f43;
const SOUND_ROWS: [u32; 3] = [0x5212, 0x5213, 0x5214];
const MONO: u32 = 0x5215;
const STEREO: u32 = 0x5216;
const CAMERA_OPTIONS: u32 = 0x4f42;
const CAMERA_ROWS: [u32; 3] = [0x4fa2, 0x4fa3, 0x521b];
const NORMAL: u32 = 0x4fa4;
const REVERSED: u32 = 0x4fa5;
/// Slow, Medium, Fast.
const SPEEDS: [u32; 3] = [0x521a, 0x5219, 0x5218];
const SUBTITLES_OPTIONS: u32 = 0x4f6f;
const SUBTITLES: u32 = 0x4f24;
const QUIT_TITLE: u32 = 0x4f3f;
const QUIT_BODY: u32 = 0x4f6d;
const NO: u32 = 0x524b;
const YES: u32 = 0x524f;
const PLANET: u32 = 0x4ecc;
/// Galactic Map: title, location texts (+ planet id), cancel legend.
const GALACTIC_MAP: u32 = 0x4ef0;
const LOCATION: u32 = 0x4eb9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
	Title,
	Root,
	NewGame,
	LoadGame,
	Options,
	Language,
	Video,
	/// [PC] Graphics Options on the title (retail/pcext.rs `title_options`): the pause menu's Graphics rows.
	Graphics,
	/// Sound Options: shared by the title and the pause menu (same item set).
	Sound,
	/// The pause root (Start in play) and its pages.
	Pause,
	Weapons,
	Gadgets,
	QuickSelect,
	Items,
	Goodies,
	Help,
	HelpLog,
	Controls,
	Moves,
	WeaponsHelp,
	GadgetsHelp,
	GameOptions,
	HelpDesk,
	SaveGame,
	/// The in-game Load page (the title's is `LoadGame`).
	LoadSaved,
	Overwrite,
	Camera,
	Subtitles,
	Quit,
	/// The Galactic Map (Select); the viewer draws the map itself.
	Map,
	/// Ship travel: the Galactic Map page 0xf and its details page 0x10 (level_transition.md).
	Galactic,
	GalacticInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
	Up,
	Down,
	Left,
	Right,
	/// Cross.
	Confirm,
	/// Triangle.
	Back,
	Circle,
	/// Start (and Select/R3 on the map): closes the pause menu from any page.
	Start,
}

/// What the mouse pointer is over on a page (port extra, `Menu::point`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Point {
	Row(usize),
	/// Quit / Overwrite: Yes (Circle) and No (Triangle).
	Yes,
	No,
	/// Sound page volume bar (row, fraction).
	Volume(usize, f32),
}

/// Menu sounds (class 0x472 cues): 0 confirm, 1 navigate, 2 type-2 item, 3 menu open, 4 page switch.
/// The page controller (FUN_00276560) plays cue 3 when the target page is the current one (only the open sets
/// both) and cue 4 on every other switch, forward and back alike (decomp/notes/ingame_pause_menu.md s2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
	Confirm = 0,
	Navigate = 1,
	/// Cross on an empty or unowned grid cell (FUN_00278e30).
	Deny = 2,
	Open = 3,
	Switch = 4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
	/// Start Veldin from the default image, saving to the slot (None = continue without saving).
	NewGame(Option<usize>),
	Load(usize),
	Language(usize),
	/// Pause menu: write the slot, close the menu, quit to the title, equip a weapon.
	Save(usize),
	Resume,
	Quit,
	Equip(u8),
	/// Gadgets page: wear `id` (0 = none) in category `cat` (inventory::category).
	Wear(usize, u8),
	/// Ship travel: depart for this planet (the current one = landing only).
	Depart(usize),
}

/// What the pause pages show, filled by the viewer when the menu opens.
#[derive(Clone, Debug, Default)]
pub struct InGame {
	/// Goodies is listed only once the game is completed (0x15eea0 or the NG+ counter 0x15ee20).
	pub goodies: bool,
	/// Help Log rows newest first: (title, help text).
	pub help_log: Vec<(String, String)>,
	/// Owned weapons: (item id, name, ammo and cap), and the one in hand.
	pub weapons: Vec<(u8, String, Option<(u16, u16)>)>,
	pub equipped: u8,
	/// Quick-select slot names ("" = empty).
	pub quick: Vec<String>,
	pub gadgets: Vec<String>,
	/// Gadgets Help entries: (owned item id, cell index 0..13 in GADGET_GRID_IDS order).
	pub gadget_help: Vec<(u8, usize)>,
	/// Gadgets page lists Hand, Back, Head, Foot: the owned items of that category (id, name) and the
	/// one worn (0 = none).
	pub wear: [(Vec<(u8, String)>, u8); 4],
	/// The Items grid cells in ITEM_GRID order: (name, description) when owned.
	pub items: Vec<Option<(String, String)>>,
	/// Quick-select slot item ids (0 = empty).
	pub quick_ids: Vec<u8>,
	/// Item ids whose gold byte 0x13e520 + id is set.
	pub golds: Vec<u8>,
	/// Gold bolts found / used (FUN_00264198 / FUN_00264208 * 4).
	pub gold: (u32, u32),
	pub planet: String,
	/// The map's right title: the hero's area name.
	pub area: String,
	/// Controls page rows (the game shows a controller picture; rac-rs lists its keys) [PC] port choice.
	pub controls: Vec<String>,
	/// The ship's Galactic Map pages (None in the pause menu).
	pub travel: Option<Travel>,
}

/// What the Galactic Map lists: the unlock order and the unlock / visited bytes (save.rs `Story`).
#[derive(Clone, Debug, Default)]
pub struct Travel {
	pub current: usize,
	pub planets: Vec<usize>,
	pub unlock: [u8; 20],
	pub visited: [u8; 20],
	/// Per planet: the mission checklist rows (text id, complete) and 'all complete' (missions.rs).
	pub missions: Vec<(Vec<(u32, bool)>, bool)>,
	/// Per planet: gold bolts found (nonzero bytes of 0x14bec0 + planet*4).
	pub gold: Vec<u32>,
}

/// Textures the front end draws, as HUD texture slots.
pub struct Art {
	pub font: (usize, Vec<hud::Glyph>),
	/// Font 2 (the Gadgets Help description text); None falls back to font 1.
	pub font2: Option<(usize, Vec<hud::Glyph>)>,
	pub logo: Option<usize>,
	/// Press Start per language.
	pub press: Vec<usize>,
	/// Sound page volume fill, HUD icon 0xe99e frames 8 (sfx) and 9 (music): (slot, w, h).
	pub vol: [Option<(usize, f32, f32)>; 2],
	/// Panel art (decomp/notes/title_assets.md, FUN_00223928): underlay 0xe99e frame 7, then menu
	/// atlas sprites 0x1a (noise), 0x1c (scanlines), 0x19 (bevel), here the fx records of those ids
	/// [I: the atlas ids index the fx table; sizes 32, 16 and 64 match the source rects]: (slot, w, h).
	pub frame: Option<[(usize, f32, f32); 4]>,
	/// Item icons 0xea60 + id frame 0, by item id: (slot, w, h).
	pub icons: HashMap<u8, (usize, f32, f32)>,
	/// Pause-menu pictures (Help / Options portraits, L1/R1 arrows, map globe).
	pub pics: Pics,
}

/// Pause-menu picture slots (HUD texture slots), decomp/notes/ingame_pause_menu.md "Pause presentation".
#[derive(Clone, Default)]
pub struct Pics {
	/// help_ss 0..4 (the Help page's picture item 0x27d3a8, indexed by the selected row).
	pub help: Vec<Option<usize>>,
	/// options_ss 0..6 (same item, selected Options row).
	pub options: Vec<Option<usize>>,
	/// help_gadgets 0..13 (the Gadgets Help picture item, indexed by the selected entry's grid cell).
	pub gadget_help: Vec<Option<usize>>,
	/// HUD icon 0xe99e frame 0: the Quick Select L1/R1 arrow (FUN_0027e100).
	pub arrow: Option<(usize, f32, f32)>,
	/// Globe layers (map bundle sections 4..6) and the planet's inset in 1/16 px of 256.
	pub globe: Option<[usize; 3]>,
	pub globe_inset: i32,
	/// The globe's planet id (its scroll offset and the station variants).
	pub globe_planet: usize,
	/// HUD icon 0xe99a frames 0xc..0xf (marker, ring, backdrop, stars) of the Galactic Map: (slot, w, h).
	pub galaxy: [Option<(usize, f32, f32)>; 4],
	/// The Items grid icons in ITEM_GRID order: (slot, w, h).
	pub items_grid: [Option<(usize, f32, f32)>; 9],
	/// Planet pictures 0..18 (512x128, the top left 306x108 is shown).
	pub planets: Vec<Option<usize>>,
	/// Globe layers of every planet (the details page shows the selected one's).
	pub globes: Vec<Option<[usize; 3]>>,
	/// The checklist tick, HUD icon 0xe99e frame 1: (slot, w, h).
	pub tick: Option<(usize, f32, f32)>,
	/// Class 0x472, the 14 panel slot mobies whose joints 2 and 3 give each panel rect (the fly-in).
	pub panels: Option<std::sync::Arc<crate::anim::Animated>>,
	/// Item icons 0xea60 + id frame 1: the worn state in the Gadgets grids (FUN_0027af20).
	pub worn: HashMap<u8, (usize, f32, f32)>,
	/// Item icons 0xea60 + id frame 4: shown when the gold byte 0x13e520 + id is set (quick ring, Gadgets grids).
	pub gold: HashMap<u8, (usize, f32, f32)>,
}

/// ToC bases of the 7-picture global sets (8-byte {sector, sectors} slots).
pub const HELP_SS: usize = 0x488;
pub const OPTIONS_SS: usize = 0x4c0;
/// ToC base of the 14 Gadgets Help pictures (8-byte slots, same layout as HELP_SS).
pub const HELP_GADGETS: usize = 0x418;

/// Gadgets Help text ids per item id: (item id, description +0x40, related move +0x44).
pub const GADGET_HELP_TEXT: [(u8, u32, u32); 14] = [
	(26, 0x4f83, 0x4e7d), (22, 0x4f82, 0x4e7f), (12, 0x4f81, 0x4e81), (32, 0x4f86, 0x5314), (27, 0x4f85, 0x5315), (31, 0x4f84, 0x5316),
	(2, 0x4f8d, 0x4e87), (3, 0x4f8c, 0x4e86), (4, 0x4f8e, 0x4e6f), (6, 0x4f8a, 0x4ec3), (5, 0x4f89, 0x5317), (7, 0x4f8b, 0x5318),
	(29, 0x4f88, 0x5319), (28, 0x4f87, 0x4e83),
];

/// One global 256x256 picture (PIF2: 0x20 header, 256x4 CLUT, 8bpp pixels at +0x420; the ToC slot is
/// WAD-compressed). The visible picture is the top left 147x133.
pub fn ss_picture(disc: &mut Disc, toc: &[u8], base: usize, i: usize) -> R<Texture> {
	let at = base + i * 8;
	let raw = disc.read_sectors(u32_at(toc, at)? as u64, u32_at(toc, at + 4)? as u64).map_err(|e| e.to_string())?;
	let d = if wad::is_wad(&raw) { wad::decompress(&raw).map_err(|e| format!("WAD: {e}"))? } else { raw };
	if span(&d, 0, 4)? != b"2FIP" {
		return Err("global picture is not a PIF2 image".into());
	}
	let pal = hud::palette_rgba(span(&d, 0x20, 0x400)?);
	let px = span(&d, 0x420, 256 * 256)?;
	Ok(Texture { w: 256, h: 256, rgba: px.iter().map(|&v| { let c = pal[v as usize]; [c[0], c[1], c[2], 255] }).collect() })
}

/// ToC bases: the 19 planet pictures (PIF2 512x128) and the 6 per-language title-card plate sets.
pub const PLANET_PICS: usize = 0x788;

fn pif2(d: &[u8], at: usize, keep_alpha: bool) -> R<Texture> {
	let e = d.get(at..).ok_or("PIF2 outside its file")?;
	if span(e, 0, 4)? != b"2FIP" {
		return Err("not a PIF2 image".into());
	}
	let (w, h) = (u32_at(e, 8)? as usize, u32_at(e, 12)? as usize);
	let pal = hud::palette_rgba(span(e, 0x20, 0x400)?);
	let px = span(e, 0x420, w * h)?;
	Ok(Texture { w, h, rgba: px.iter().map(|&v| { let c = pal[v as usize]; [c[0], c[1], c[2], if keep_alpha { c[3] } else { 255 }] }).collect() })
}

fn maybe_wad(raw: Vec<u8>) -> R<Vec<u8>> {
	if wad::is_wad(&raw) { wad::decompress(&raw).map_err(|e| format!("WAD: {e}")) } else { Ok(raw) }
}

/// Planet picture `i` of the Galactic Map (disc table 0x138308 = ToC 0x788; level_transition.md).
pub fn planet_picture(disc: &mut Disc, toc: &[u8], i: usize) -> R<Texture> {
	let at = PLANET_PICS + i * 8;
	let raw = disc.read_sectors(u32_at(toc, at)? as u64, u32_at(toc, at + 4)? as u64).map_err(|e| e.to_string())?;
	pif2(&maybe_wad(raw)?, 0, false)
}

pub struct Menu {
	pub page: Page,
	stack: Vec<(Page, usize)>,
	pub sel: usize,
	/// Seconds on the front end and on this page.
	time: f32,
	page_time: f32,
	/// Per item highlight, 0..1 over FADE_FRAMES.
	glow: Vec<f32>,
	/// The previous page's panel rects; this page's panels fly from them (decomp/notes/title_assets.md).
	from: Vec<[f32; 4]>,
	/// The page left for this one, and whether that was a step back (its slot seqs then play reversed).
	left: Option<(Page, bool)>,
	/// Gadgets page: the cursor cell of each grid (item +0x3c), kept while the menu lives.
	cell: [usize; 4],
	/// Per panel noise pulse (FUN_00223928): counter while active, and this tick's 0x1a source origin.
	pulse: [(Option<u32>, [f32; 2]); 16],
	/// Ticks not yet run.
	ticks: f32,
	/// Help Log body cross-fade: the shown selection and the counter (-1 until the first tick).
	help_fade: (usize, i32),
	pub lang: usize,
	/// The Options values (settings.rs); the viewer applies and stores them when they change.
	pub settings: Settings,
	logo_alpha: f32,
	pub slots: [Option<crate::save::Summary>; crate::save::SLOTS],
	pub ingame: InGame,
	/// The slot an overwrite confirm is for.
	pending: usize,
	/// A message box over the page (text id, seconds left); keys wait while it is up.
	notice: Option<(u32, f32)>,
	/// Held Left/Right time not yet turned into volume steps (50 per second).
	hold_t: f32,
	/// Retail mode (viewer/front_mirror.rs): the front end's own logo and Press Start alphas (PS2 alpha, 0x80 opaque) read
	/// from its RAM; the menu then only draws what retail's state says (`follow`).
	pub ram_alpha: Option<(f32, f32)>,
	/// Retail mode: the Press Start caption index (captions run EN, FR, DE, ES, IT; `lang` is the Language page's row order).
	pub press_lang: usize,
	/// Retail mode: the text ids of the Options rows as the game lists them (viewer/front_mirror.rs); empty = the PAL rows.
	pub front_rows: Vec<u32>,
	/// Retail mode: the text ids of the Language page's five languages, as the game lists them (viewer/front_mirror.rs); empty =
	/// the disc's own order. The game lists the languages in the console's order and the disc's ids are in the game's, which puts
	/// German and Spanish the other way round, so the rows follow the game.
	pub lang_rows: Vec<u32>,
}

/// Where a page's panels, header and rows go (PAL 512x448): the rest rects of the class-0x472 slot
/// mobies (decomp/notes/title_assets.md "Page rect table"). Which slot holds the header and the list
/// is not traced per page: slot 0 as header, the list panel picked by shape [I].
struct Layout {
	panels: Vec<[f32; 4]>,
	title: Option<[f32; 4]>,
	rows: Rows,
}

enum Rows {
	/// One panel per item (root pages, the Gadgets lists).
	Each(Vec<[f32; 4]>),
	/// Rows spread over one panel at h/(n+1) (FUN_0021acc0 row spacing).
	Column([f32; 4]),
	/// The save-slot strip: rows 0x38 high every 0x54 from y+4 (FUN_002234e0); New Game's extra
	/// 'Continue without saving' row goes in the second panel [I].
	Slots([f32; 4], Option<[f32; 4]>),
	None,
}

const TITLE_BACK: [f32; 4] = [193.0, 345.0, 127.0, 78.0];
const GAME_BACK: [f32; 4] = [191.0, 346.0, 131.0, 78.0];
const OPTION_TITLE: [f32; 4] = [22.0, 23.0, 281.0, 47.0];
const SLOT_STRIP: [f32; 4] = [20.0, 25.0, 126.0, 398.0];
const SLOT_TITLE: [f32; 4] = [208.0, 23.0, 241.0, 47.0];
/// The slot page's detail panel: the selected slot's summary.
const SLOT_INFO: [f32; 4] = [168.0, 116.0, 322.0, 120.0];
/// Pause root buttons (frame 0 of seqs e..14): x 155, 203x39; the Goodies slot stays reserved.
/// Page move-in length in ticks: d04 = 12 (FUN_00276560, decomp/notes/title_assets.md).
const MOVE_TICKS: f32 = 12.0;
const PAUSE_Y: [f32; 7] = [43.0, 97.0, 153.0, 207.0, 262.0, 316.0, 372.0];
/// The Galactic Map's map panel (slot 2 of page 0x1b3518).
pub const MAP_PANEL: [f32; 4] = [214.0, 80.0, 276.0, 341.0];

fn layout(page: Page, n: usize, in_game: bool) -> Layout {
	let l = |panels: &[[f32; 4]], rows: Rows| Layout { panels: panels.to_vec(), title: Some(panels[0]), rows };
	match page {
		Page::Root => Layout { panels: ROOT_PANELS.to_vec(), title: None, rows: Rows::Each(ROOT_PANELS.to_vec()) },
		Page::Pause => {
			let b: Vec<[f32; 4]> = PAUSE_Y.iter().take(n).map(|&y| [155.0, y, 203.0, 39.0]).collect();
			Layout { panels: b.clone(), title: None, rows: Rows::Each(b) }
		}
		Page::Options => l(&[[193.0, 23.0, 127.0, 47.0], [193.0, 119.0, 127.0, 176.0], TITLE_BACK], Rows::Column([193.0, 119.0, 127.0, 176.0])),
		Page::Language => l(&[[175.0, 23.0, 163.0, 47.0], [181.0, 119.0, 151.0, 176.0], TITLE_BACK], Rows::Column([181.0, 119.0, 151.0, 176.0])),
		Page::Video => l(&[[155.0, 23.0, 203.0, 47.0], [58.0, 170.0, 396.0, 62.0], TITLE_BACK], Rows::Column([58.0, 170.0, 396.0, 62.0])),
		// Video's header and back panels, the Sound page's value panel (pcext.rs `title_options` copies its slot-3 sequence).
		Page::Graphics => l(&[[155.0, 23.0, 203.0, 47.0], [47.0, 125.0, 419.0, 164.0], TITLE_BACK], Rows::Column([47.0, 125.0, 419.0, 164.0])),
		Page::Sound if in_game => l(&[OPTION_TITLE, [53.0, 127.0, 406.0, 164.0], GAME_BACK], Rows::Column([53.0, 127.0, 406.0, 164.0])),
		Page::Sound => l(&[[155.0, 23.0, 203.0, 47.0], [47.0, 125.0, 419.0, 164.0], [191.0, 346.0, 131.0, 78.0]], Rows::Column([47.0, 125.0, 419.0, 164.0])),
		Page::NewGame | Page::LoadGame => {
			let more = [168.0, 254.0, 322.0, 48.0];
			l(&[SLOT_TITLE, SLOT_STRIP, SLOT_INFO, more, [172.0, 345.0, 316.0, 80.0]], Rows::Slots(SLOT_STRIP, (page == Page::NewGame).then_some(more)))
		}
		Page::SaveGame | Page::LoadSaved => l(&[SLOT_TITLE, SLOT_STRIP, SLOT_INFO, [168.0, 254.0, 322.0, 48.0], [265.0, 358.0, 131.0, 67.0]], Rows::Slots(SLOT_STRIP, None)),
		Page::Overwrite => l(&[SLOT_TITLE, SLOT_STRIP, [188.0, 94.0, 287.0, 238.0], [265.0, 358.0, 131.0, 67.0]], Rows::None),
		// Only slot 3 is on screen; header and buttons sit inside it [I].
		Page::Quit => Layout { panels: vec![[173.0, 125.0, 166.0, 168.0]], title: Some([173.0, 129.0, 166.0, 30.0]), rows: Rows::None },
		Page::GameOptions => l(&[[136.0, 23.0, 241.0, 47.0], [23.0, 134.0, 148.0, 150.0], [341.0, 134.0, 148.0, 150.0], [191.0, 91.0, 131.0, 235.0], GAME_BACK], Rows::Column([191.0, 91.0, 131.0, 235.0])),
		Page::Help => l(&[[152.0, 24.0, 209.0, 47.0], [21.0, 134.0, 155.0, 148.0], [336.0, 134.0, 155.0, 148.0], [195.0, 91.0, 123.0, 235.0], [195.0, 345.0, 123.0, 78.0]], Rows::Column([195.0, 91.0, 123.0, 235.0])),
		Page::HelpLog => l(&[[23.0, 24.0, 209.0, 47.0], [23.0, 92.0, 209.0, 329.0], HELP_BODY, [326.0, 382.0, 92.0, 42.0]], Rows::Column([23.0, 92.0, 209.0, 329.0])),
		Page::Controls => l(&[[24.0, 24.0, 148.0, 47.0], [21.0, 92.0, 471.0, 266.0], [381.0, 381.0, 111.0, 43.0]], Rows::Column([21.0, 92.0, 471.0, 266.0])),
		Page::Moves => l(&[[22.0, 24.0, 178.0, 47.0], [22.0, 82.0, 178.0, 286.0], [225.0, 278.0, 266.0, 144.0], [225.0, 23.0, 266.0, 235.0], [54.0, 385.0, 113.0, 40.0]], Rows::Column([22.0, 82.0, 178.0, 286.0])),
		Page::WeaponsHelp | Page::GadgetsHelp => {
			let mut p = vec![[24.0, 24.0, 209.0, 47.0], [23.0, 90.0, 58.0, 211.0]];
			if page == Page::WeaponsHelp {
				p.extend([[101.0, 90.0, 134.0, 97.0], [101.0, 204.0, 134.0, 97.0], [253.0, 24.0, 238.0, 235.0]]);
			} else {
				p.extend([[101.0, 90.0, 134.0, 211.0], [253.0, 25.0, 238.0, 175.0], [253.0, 220.0, 238.0, 39.0]]);
			}
			p.extend([[23.0, 322.0, 212.0, 40.0], [64.0, 381.0, 134.0, 41.0], [253.0, 278.0, 238.0, 144.0]]);
			l(&p, Rows::Column(if page == Page::GadgetsHelp { [23.0, 90.0, 58.0, 211.0] } else { [101.0, 90.0, 134.0, 211.0] }))
		}
		Page::Weapons => l(&[[24.0, 23.0, 212.0, 35.0], [263.0, 25.0, 224.0, 236.0], [24.0, 90.0, 212.0, 147.0], [24.0, 264.0, 212.0, 100.0], [263.0, 282.0, 227.0, 37.0], [264.0, 342.0, 90.0, 82.0], EQUIP_PANEL, [378.0, 342.0, 113.0, 82.0]], Rows::None),
		Page::Gadgets => {
			let lists = GADGET_PANELS.map(|g| g.1);
			let mut p = vec![[24.0, 23.0, 212.0, 35.0], [263.0, 25.0, 224.0, 236.0], [263.0, 282.0, 227.0, 37.0], [264.0, 342.0, 90.0, 82.0], [378.0, 342.0, 113.0, 82.0]];
			p.extend(GADGET_PANELS.map(|g| g.0));
			p.extend(lists);
			l(&p, Rows::Each(lists.to_vec()))
		}
		Page::QuickSelect => l(&[[24.0, 23.0, 212.0, 35.0], [263.0, 25.0, 224.0, 236.0], [23.0, 85.0, 215.0, 148.0], [61.0, 258.0, 138.0, 105.0], [263.0, 282.0, 227.0, 37.0], [264.0, 342.0, 90.0, 82.0], [21.0, 390.0, 219.0, 35.0], [378.0, 342.0, 113.0, 82.0]], Rows::None),
		Page::Items => l(&[[22.0, 23.0, 189.0, 47.0], [22.0, 89.0, 189.0, 148.0], [238.0, 258.0, 253.0, 103.0], [237.0, 24.0, 253.0, 213.0], [22.0, 315.0, 189.0, 106.0], [391.0, 383.0, 100.0, 39.0], [22.0, 258.0, 189.0, 34.0]], Rows::None),
		Page::Goodies => l(&[[21.0, 25.0, 192.0, 42.0], [21.0, 93.0, 192.0, 228.0], [239.0, 25.0, 251.0, 297.0], [56.0, 343.0, 121.0, 81.0], [239.0, 343.0, 251.0, 81.0]], Rows::Column([21.0, 93.0, 192.0, 228.0])),
		Page::HelpDesk | Page::Subtitles => l(&[OPTION_TITLE, [166.0, 175.0, 172.0, 67.0], GAME_BACK], Rows::Column([166.0, 175.0, 172.0, 67.0])),
		Page::Camera => l(&[OPTION_TITLE, [69.0, 176.0, 381.0, 91.0], GAME_BACK], Rows::Column([69.0, 176.0, 381.0, 91.0])),
		// Galactic Map (frame 0): header s0, planet title s1, planet list s3; the viewer draws the map in s2.
		// The planet name goes in s1 (left) and the area name in s0 (retail capture) [I].
		Page::Map => Layout { title: Some([23.0, 27.0, 172.0, 35.0]), ..l(&[[214.0, 27.0, 276.0, 35.0], [23.0, 27.0, 172.0, 35.0], MAP_PANEL, [23.0, 81.0, 172.0, 170.0], [23.0, 277.0, 172.0, 145.0]], Rows::None) },
		// Page 0xf (level_transition.md, Veldin 0x1b6088): title s0, list s1, map s2, legend s3 and planet
		// picture s4 (corrected 2026-10-04: s3/s4 were guessed at (20,350) / (183,292)).
		Page::Galactic => l(&[[21.0, 23.0, 470.0, 42.0], [20.0, 86.0, 144.0, 251.0], [183.0, 86.0, 307.0, 197.0], [22.0, 358.0, 144.0, 66.0], [183.0, 304.0, 307.0, 120.0]], Rows::None),
		// Page 0x10 (0x1b63f8): s0 area name, s1 planet name, s2 checklist, s3 globe, s4 gold bolts, s5 legend
		// (items 0x1b3868, 0x1b35f8, 0x1b64d0, 0x1b6550, 0x1b65b8, 0x1b6480).
		Page::GalacticInfo => Layout { title: None, ..l(&[[268.0, 23.0, 223.0, 42.0], [22.0, 23.0, 223.0, 42.0], [268.0, 88.0, 223.0, 242.0], [22.0, 88.0, 223.0, 242.0], [268.0, 351.0, 223.0, 72.0], [55.0, 351.0, 155.0, 72.0]], Rows::None) },
		Page::Title => Layout { panels: Vec::new(), title: None, rows: Rows::None },
	}
}

/// The 14 class-0x472 slot seqs of a page (page struct +0x00; boot VA for the title tree, Veldin overlay
/// VA in game) and whether it is a root page (parent 0, resting on frame 0), decomp/notes/title_assets.md
/// "Page rect table". Slots 5..0xd mostly park the 1-frame off-screen seqs.
fn slot_seqs(page: Page, in_game: bool) -> Option<([u8; 14], bool)> {
	let park = |head: &[u8]| -> [u8; 14] { std::array::from_fn(|i| head.get(i).copied().unwrap_or(i as u8)) };
	Some(match page {
		Page::Root => (park(&[0, 1, 0x10, 0x11, 0x12]), true),
		Page::Options => (park(&[0, 1, 0xbe, 0xbf, 0xc0]), false),
		Page::Language => (park(&[0, 1, 0xc1, 0xc2, 0xc3]), false),
		Page::Video => (park(&[0, 1, 0xca, 0xcb, 0xc3]), false),
		Page::Graphics => (park(&[0, 1, 0xca, 0xd2, 0xc3]), false),
		Page::Sound if in_game => (park(&[0x76, 0xd4, 0xd5, 0xd6, 0xd7]), false),
		Page::Sound => (park(&[0, 1, 0xca, 0xd2, 0xd7]), false),
		Page::LoadGame => (park(&[0xc4, 0xc5, 0xc6, 0xc7, 0xc8]), false),
		Page::NewGame => (park(&[0xc4, 0xc5, 0xc6, 0xc7, 0xc9]), false),
		Page::Pause => (park(&[0xe, 0xf, 0x10, 0x11, 0x12, 0x13, 0x14]), true),
		Page::Weapons => (park(&[0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x14, 0x20, 0x21]), false),
		Page::Gadgets => ([0x1a, 0x1b, 0x22, 0x23, 0x1e, 0x1f, 0x14, 0x24, 0x21, 0x25, 0x26, 0x27, 0x28, 0x29], false),
		Page::QuickSelect => (park(&[0x1a, 0x1b, 0x2a, 0x2b, 0x1e, 0x1f, 0x14, 0x2c, 0x21]), false),
		Page::Items => (park(&[0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x14, 0x5a]), false),
		Page::Help => (park(&[0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x14]), false),
		Page::HelpLog => (park(&[0xb9, 0xba, 0xbb, 0xbc, 0xbd]), false),
		Page::Controls => (park(&[0x39, 0x3a, 0x3b, 0x3c, 0x3d]), false),
		Page::Moves => (park(&[0x3e, 0x3f, 0x40, 0x41, 0x42]), false),
		Page::WeaponsHelp => (park(&[0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60, 6, 0x61, 0x62]), false),
		Page::GadgetsHelp => (park(&[0x6e, 0x6f, 0x70, 0x71, 0x72, 0x73, 6, 0x74, 0x75]), false),
		Page::GameOptions => (park(&[0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x14]), false),
		Page::HelpDesk | Page::Subtitles => (park(&[0x49, 0x4a, 0x4b, 0x4c, 0x4d]), false),
		Page::Camera => (park(&[0x76, 0x77, 0x78, 0x79, 0x7a]), false),
		Page::SaveGame | Page::LoadSaved => (park(&[0x7c, 0x7d, 0x7e, 0x7f, 0x80]), false),
		// The front end's copy (page 0x1d4db8): slot 4 is the Yes / No label panel.
		Page::Overwrite if !in_game => (park(&[0x81, 0x82, 0x83, 0x84, 0xd1]), false),
		Page::Overwrite => (park(&[0x81, 0x82, 0x83, 0x84, 0x85]), false),
		Page::Quit => (park(&[0x4e, 0x4f, 0x50, 0x51, 0x52]), false),
		Page::Goodies => (park(&[0x8e, 0x8f, 0x90, 0x91, 0x92, 0x13, 0x14]), false),
		Page::Map => (park(&[0x15, 0x16, 0x17, 0x18, 0x19]), true),
		Page::Galactic => (park(&[0x63, 0x64, 0x65, 0x66, 0x67]), true),
		Page::GalacticInfo => (park(&[0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d]), false),
		Page::Title => return None,
	})
}

/// Index of a slot seq's last frame (0 for the parked 1-frame seqs and missing ones).
fn last_frame(a: &crate::anim::Animated, seq: u8) -> f32 {
	a.sequences.get(seq as usize).map_or(0, |s| s.frames.len().saturating_sub(1)) as f32
}

/// Panel rect of slot seq `seq` at frame position `at` (pose lerped between frames, FUN_00253d18):
/// joints 2 (top left) and 3 (bottom right), camera-relative (depth, y, z), projected by FUN_00237588 /
/// FUN_002998e8 with tan-half-FOV 0.63 and the PAL 0.756 vertical factor, truncated, plus (1, 1)
/// (decomp/notes/title_assets.md "Fly-in", ingame_pause_menu.md "Menu camera").
fn slot_rect(a: &crate::anim::Animated, seq: u8, at: f32) -> Option<[f32; 4]> {
	let [p0, p3] = slot_corners(a, seq, at)?;
	let px = |w: [f32; 3]| (256.0 - 256.0 * w[1] / (0.63 * w[0]), 224.0 - 224.0 * w[2] / (0.63 * 0.756 * w[0]));
	let ((x0, y0), (x3, y3)) = (px(p0), px(p3));
	Some([x0.trunc() + 1.0, y0.trunc() + 1.0, (x3 - x0).trunc(), (y3 - y0).trunc()])
}

/// Camera-relative (depth, y, z) of a slot's top-left and bottom-right corners (joints 2 and 3).
fn slot_corners(a: &crate::anim::Animated, seq: u8, at: f32) -> Option<[[f32; 3]; 2]> {
	let s = a.sequences.get(seq as usize).filter(|s| !s.frames.is_empty())?;
	let at = at.clamp(0.0, (s.frames.len() - 1) as f32);
	let f = at.floor() as usize;
	let m = a.pose_frames(&s.frames[f], &s.frames[(f + 1).min(s.frames.len() - 1)], at - f as f32);
	Some([2, 3].map(|j| {
		let w = a.joint_world(&m, j);
		[w[3], w[7], w[11]]
	}))
}

/// Gadgets page grids in list order Hand, Back, Head, Foot (items s2, s3, s9, s10 of page 0x1b28e8):
/// (rows +0x40, columns +0x44, cell item ids from the 10-byte cells at +0x48, whether the worn item
/// shows icon frame 1; the hand grid has flag 0x20, which suppresses it). Up / down chain the grids
/// Hand -> Back -> Head -> Foot -> Hand (+0x4c / +0x50); none has left / right neighbours.
/// Every item id in the Gadgets grids (their worn-frame icons are loaded).
pub const GADGET_GRID_IDS: [u8; 14] = [26, 22, 12, 32, 27, 31, 2, 3, 4, 6, 5, 7, 29, 28];
const GADGET_GRIDS: [(usize, usize, &[u8], bool); 4] = [(2, 3, &[26, 22, 12, 32, 27, 31], false), (1, 3, &[2, 3, 4], true), (1, 3, &[6, 5, 7], true), (1, 2, &[29, 28], true)];

/// One Gadgets grid (FUN_0027af20): cells of 0.45 x 0.45 world units, 0.1 margins (+0x34 / +0x38), spread
/// over the panel's world size (slot joint lengths, moby vars +0x40 / +0x44) and scaled to pixels by
/// `s`. Owned items draw icon 0xea60 + id (frame 1 when worn); the cursor cell gets a pulsing square
/// (colour (2v, 2v, v), v = 0x40 + |(tick & 0x3f) - 0x20| [I: FUN_001fedb0 taken as abs]) 3 px out and
/// a 0x80100808 square 1 px out. Frame 4 when the byte 0x13e520 + id is set, else frame 0.
#[allow(clippy::too_many_arguments)]
fn gadget_grid(out: &mut Vec<Quad>, art: &Art, s: f32, (rows, cols): (usize, usize), cells: &[u8], golds: &[u8], owned: impl Fn(u8) -> bool, worn: impl Fn(u8) -> bool, cur: Option<usize>, tick: i32, r: [f32; 4], k: f32, ox: f32) {
	const CELL_W: f32 = 0.45;
	let (w, h) = (r[2] / s, r[3] / s);
	let spread = |n: usize, size: f32| if n < 2 { ((size - CELL_W) * 0.5, 0.0) } else { (0.1, CELL_W + (size - 0.2 - CELL_W * n as f32) / (n - 1) as f32) };
	let ((x0, dx), (y0, dy)) = (spread(cols, w), spread(rows, h));
	let c = (s * CELL_W).trunc();
	for (i, &id) in cells.iter().enumerate().take(rows * cols) {
		let (x, y) = (r[0] + (s * (x0 + (i % cols) as f32 * dx)).trunc(), r[1] + (s * (y0 + (i / cols) as f32 * dy)).trunc());
		if cur == Some(i) {
			let v = (0x40 + ((tick & 0x3f) - 0x20).abs()) as f32 / 128.0;
			crate::overlay::rect(out, [ox + (x - 3.0) * k, (y - 3.0) * k, (c + 6.0) * k, (c + 6.0) * k], [(2.0 * v).min(1.0), (2.0 * v).min(1.0), v, 1.0]);
			crate::overlay::rect(out, [ox + (x - 1.0) * k, (y - 1.0) * k, (c + 2.0) * k, (c + 2.0) * k], [8.0 / 128.0, 8.0 / 128.0, 16.0 / 128.0, 1.0]);
		}
		if !owned(id) {
			continue;
		}
		let icon = if worn(id) { art.pics.worn.get(&id) } else if golds.contains(&id) { art.pics.gold.get(&id) } else { art.icons.get(&id) };
			let icon = icon.or(art.icons.get(&id));
		if let Some(&(slot, _, _)) = icon {
			out.push(Quad { tex: slot, rect: [ox + x * k, y * k, c * k, c * k], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
		}
	}
}

/// The Help Log's text panel (slot 2 of page 0x1b3c60).
const HELP_BODY: [f32; 4] = [253.0, 113.0, 237.0, 222.0];
/// The Weapons page 'Equip' panel (slot 7).
const EQUIP_PANEL: [f32; 4] = [24.0, 389.0, 212.0, 35.0];
/// Gadgets page: (label panel, list panel) for Hand, Back, Head and Foot items.
const GADGET_PANELS: [([f32; 4], [f32; 4]); 4] = [
	([20.0, 108.0, 64.0, 43.0], [100.0, 77.0, 138.0, 105.0]),
	([20.0, 210.0, 64.0, 43.0], [100.0, 200.0, 138.0, 62.0]),
	([20.0, 290.0, 64.0, 43.0], [100.0, 281.0, 138.0, 62.0]),
	([20.0, 369.0, 64.0, 43.0], [101.0, 361.0, 96.0, 62.0]),
];

/// Row rects for `n` items in a column panel, scrolled to keep `sel` visible: (first item, rects).
fn column(r: [f32; 4], n: usize, sel: usize) -> (usize, Vec<[f32; 4]>) {
	let max = (((r[3] - 8.0) / 16.0).floor() as usize).max(1);
	let shown = n.min(max);
	let first = sel.saturating_sub(shown.saturating_sub(1)).min(n - shown);
	let step = r[3] / (shown + 1) as f32;
	let rh = step.min(22.0);
	(first, (0..shown).map(|i| [r[0] + 4.0, r[1] + step * (i + 1) as f32 - rh / 2.0, r[2] - 8.0, rh]).collect())
}

/// Save-slot rows in the strip: y + 4 + i*0x54, 0x38 high (selected rect from y-4).
fn slot_rects(r: [f32; 4], n: usize) -> Vec<[f32; 4]> {
	(0..n.min(crate::save::SLOTS)).map(|i| [r[0], r[1] + i as f32 * 84.0, r[2], 56.0]).collect()
}

impl Menu {
	pub fn new(lang: usize) -> Menu {
		// FUN_0022a980 resets the shared rand() to 12345 before generating its effect slots.
		crate::particles::Rng::shared().reseed(crate::particles::RAND_SEED);
		Menu {
			page: Page::Title,
			stack: Vec::new(),
			sel: 0,
			time: 0.0,
			page_time: 0.0,
			glow: Vec::new(),
			from: Vec::new(),
			left: None,
			cell: [0; 4],
			pulse: [(None, [0.0; 2]); 16],
			ticks: 0.0,
			help_fade: (0, -1),
			lang,
			settings: Settings::default(),
			logo_alpha: 0.0,
			slots: Default::default(),
			ingame: InGame::default(),
			pending: 0,
			notice: None,
			hold_t: 0.0,
			ram_alpha: None,
			press_lang: 0,
			front_rows: Vec::new(),
			lang_rows: Vec::new(),
		}
	}

	/// Retail mode: take the page, selection and slot list from the running front end's RAM (viewer/front_mirror.rs). The
	/// menu's own key handling and clocks decide nothing; this only starts the page fly-in when retail changed page.
	pub fn follow(&mut self, page: Page, sel: usize, slots: [Option<crate::save::Summary>; crate::save::SLOTS]) {
		if page != self.page {
			if page == Page::Title || matches!(page, Page::Root) && !self.stack.iter().any(|s| s.0 == Page::Root) && self.page == Page::Title {
				// The title and its first menu page: the root page flies in over the title.
				self.stack.clear();
				self.from = self.panels_now();
				self.left = Some((self.page, false));
				self.page = page;
				self.page_time = 0.0;
				self.glow.clear();
			} else if self.stack.last().is_some_and(|s| s.0 == page) {
				self.back();
			} else {
				self.enter(page, sel);
			}
		}
		self.sel = sel;
		self.slots = slots;
	}

	/// The planet the Galactic Map cursor is on.
	fn selected_planet(&self) -> Option<usize> {
		self.ingame.travel.as_ref().and_then(|t| t.planets.get(self.sel).copied())
	}

	fn slot_rows(&self, text: &Text, empty_ok: bool) -> Vec<(String, bool)> {
		self.slots
			.iter()
			.map(|s| match s {
				Some(s) => {
					let name = crate::toc::LEVEL_NAMES.get(s.level).copied().unwrap_or("?");
					(format!("{name}  {:02}:{:02}  {}  {}", (s.play_frames / 216000).min(99), s.play_frames / 3600 % 60, s.bolts, s.date_text()), true)
				}
				None => (text.get(EMPTY_SLOT).to_string(), empty_ok),
			})
			.collect()
	}

	/// The Weapons grid: the owned weapons without the wrench, which retail does not list [I: capture].
	fn grid_weapons(&self) -> Vec<&(u8, String, Option<(u16, u16)>)> {
		self.ingame.weapons.iter().filter(|w| w.0 != crate::inventory::WRENCH).collect()
	}

	/// The 3D item preview of the page (item id, panel rect in 512x448 screen space): the selected weapon
	/// in panel 3 of Weapons / Quick Select (FUN_00281948 spawns the item's moby there; the per-item view
	/// table is not transcribed [P]).
	/// Also the panel item's angle: pi at page entry, +0.01 rad per tick, wrapped (FUN_0027b808).
	pub fn preview(&self) -> Option<(u8, [f32; 4], f32)> {
		let id = match self.page {
			Page::Weapons => self.grid_weapons().get(self.sel)?.0,
			Page::QuickSelect => *self.ingame.quick_ids.get(self.sel).filter(|&&q| q != 0)?,
			// The Gadgets panel model follows the grid cursor (i = sel.min(3), FUN_0027b808).
			Page::Gadgets => {
				let i = self.sel.min(3);
				GADGET_GRIDS[i].2[self.cell[i].min(GADGET_GRIDS[i].2.len() - 1)]
			}
			Page::GadgetsHelp => self.gadget_entry()?.0,
			_ => return None,
		};
		let t = (std::f32::consts::TAU + 0.01 * (self.page_time * 50.0).floor()).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
		(self.page_time * 50.0 >= MOVE_TICKS).then(|| (id, layout(self.page, 0, self.in_game()).panels[if self.page == Page::GadgetsHelp { 2 } else { 3 }], t))
	}

	/// The hero in the big panel of Weapons / Gadgets / Quick Select (decomp/notes/ingame_pause_menu.md "Item
	/// previews"): (item id whose model is attached, 0 = none; panel 1 rect in 512x448 screen space).
	/// The Gadgets Help list entry under the cursor: (item id, grid cell index).
	pub fn gadget_entry(&self) -> Option<(u8, usize)> {
		self.ingame.gadget_help.get(self.sel).copied()
	}

	/// Whether item `id` is in one of the Gadgets page's owned lists (the panel model is drawn only then).
	pub fn gadget_owned(&self, id: u8) -> bool {
		self.page == Page::GadgetsHelp && self.gadget_entry().is_some_and(|e| e.0 == id)
			|| self.ingame.wear.iter().any(|w| w.0.iter().any(|(i, _)| *i == id))
	}

	/// The details page's gold bolt (FUN_0027c550 spawns class 1134 at menu camera + (8, 1.3, -0.1)): the panel-4
	/// rect it is drawn in, a square at the panel's left, free of the right-aligned count [I: placement; turntable [P]].
	pub fn gold_preview(&self) -> Option<[f32; 4]> {
		if self.page != Page::GalacticInfo || self.page_time * 50.0 < MOVE_TICKS {
			return None;
		}
		let r = layout(self.page, 0, self.in_game()).panels[4];
		Some([r[0] + 4.0, r[1] + 4.0, r[3] - 8.0, r[3] - 8.0])
	}

	pub fn hero_preview(&self) -> Option<(u8, [f32; 4])> {
		let id = match self.page {
			Page::Weapons => self.grid_weapons().get(self.sel).map_or(0, |w| w.0),
			Page::Gadgets => self.ingame.wear[self.sel.min(3)].1,
			Page::QuickSelect => 0,
			_ => return None,
		};
		(self.page_time * 50.0 >= MOVE_TICKS).then(|| (id, layout(self.page, 0, self.in_game()).panels[1]))
	}

	/// Pixels per world unit of the Gadgets grid in panel `r`: max(pixel w, h) / max(world w, h) of the
	/// slot whose rest rect is `r` (FUN_0027af20, item +0x20 / +0x24 taken as the pixel size [I]).
	/// Without the slot class the panels sit at depth 6 (title_assets.md) [I].
	fn grid_scale(&self, art: &Art, r: [f32; 4]) -> f32 {
		let found = art.pics.panels.as_deref().zip(slot_seqs(self.page, self.in_game())).and_then(|(a, (s, root))| {
			(0..14).find_map(|j| {
				let at = if root { 0.0 } else { last_frame(a, s[j]) };
				let q = slot_rect(a, s[j], at)?;
				if !(0..4).all(|c| (q[c] - r[c]).abs() <= 1.0) {
					return None;
				}
				let [p0, p3] = slot_corners(a, s[j], at)?;
				Some(r[2].max(r[3]) / (p3[1] - p0[1]).abs().max((p3[2] - p0[2]).abs()))
			})
		});
		found.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(256.0 / (0.63 * 6.0))
	}

	fn items(&self, text: &Text) -> Vec<(String, bool)> {
		let t = |id| (crate::retail::prompts::for_text(text.get(id)), true);
		let onoff = |b: bool| text.get(if b { ON } else { OFF });
		let s = &self.settings;
		match self.page {
			Page::Title | Page::Quit | Page::Overwrite => Vec::new(),
			Page::Map => vec![(self.ingame.planet.clone(), true)],
			Page::Galactic | Page::GalacticInfo => self.ingame.travel.iter().flat_map(|t| t.planets.iter()).map(|&p| (text.get(PLANET + p as u32).to_string(), true)).collect(),
			Page::Root => vec![t(NEW_GAME), t(LOAD_GAME), t(OPTIONS)],
			Page::NewGame => {
				let mut v = self.slot_rows(text, true);
				v.push(t(NO_SAVE));
				v
			}
			Page::LoadGame | Page::LoadSaved => self.slot_rows(text, false),
			Page::SaveGame => self.slot_rows(text, true),
			// Retail mode: the rows the game lists (the Language page's five languages in the game's own order, USA: no Video row;
			// the PC Graphics row, retail/pcext.rs).
			Page::Options if !self.front_rows.is_empty() => self.front_rows.iter().map(|&id| t(id)).collect(),
			Page::Options if crate::retail::disc::usa() => vec![t(LANGUAGE), t(SOUND)],
			Page::Options => vec![t(LANGUAGE), t(VIDEO), t(SOUND)],
			Page::Language if !self.lang_rows.is_empty() => self.lang_rows.iter().map(|&id| t(id)).collect(),
			Page::Language => (0..5).map(|i| t(LANGUAGES + i)).collect(),
			Page::Video => vec![(format!("{}  {}", text.get(VIDEO_MODE), text.get(if s.hz60 { HZ60 } else { HZ50 })), true)],
			Page::Graphics => crate::retail::pcext::title_rows().into_iter().map(|(label, value)| (format!("{}  {}", text.get(label), text.get(value)), true)).collect(),
			Page::Sound => {
				let mut v: Vec<(String, bool)> = SOUND_ROWS.iter().map(|&id| t(id)).collect();
				v[2].0 = format!("{}  {}", v[2].0, text.get(if s.stereo { STEREO } else { MONO }));
				v
			}
			Page::Pause => {
				let mut v: Vec<(String, bool)> = [WEAPONS, GADGETS, QUICK_SELECT, ITEMS, HELP, OPTIONS].iter().map(|&id| t(id)).collect();
				if self.ingame.goodies {
					v.push(t(GOODIES));
				}
				v
			}
			Page::Weapons => self.grid_weapons().into_iter().map(|(id, name, ammo)| {
				let mark = if *id == self.ingame.equipped { "* " } else { "" };
				(match ammo { Some((n, m)) => format!("{mark}{name}  {n}/{m}"), None => format!("{mark}{name}") }, true)
			}).collect(),
			// The four grids are the rows; their cells are drawn by `gadget_grid`, not as labels.
			Page::Gadgets => (0..4).map(|_| (String::new(), true)).collect(),
			Page::QuickSelect => self.ingame.quick.iter().enumerate().map(|(i, q)| (format!("{}  {}", i + 1, if q.is_empty() { "-" } else { q }), !q.is_empty())).collect(),
			Page::Items => (0..9).map(|i| (self.ingame.items.get(i).cloned().flatten().map_or(String::new(), |c| c.0), true)).collect(),
			// Skill Points, Credits, Cheats, movies...: not reachable before the game is beaten (hook only).
			Page::Goodies => Vec::new(),
			Page::Help => vec![t(HELP_LOG), t(CONTROLS), t(MOVES), t(WEAPONS), t(GADGETS)],
			Page::HelpLog => self.ingame.help_log.iter().map(|(title, _)| (title.clone(), true)).collect(),
			Page::Controls => self.ingame.controls.iter().map(|c| (c.clone(), false)).collect(),
			Page::Moves => MOVE_LIST.iter().map(|&id| t(id)).collect(),
			Page::WeaponsHelp => self.ingame.weapons.iter().map(|w| (w.1.clone(), true)).collect(),
			Page::GadgetsHelp => self.ingame.gadgets.iter().map(|g| (g.clone(), true)).collect(),
			Page::GameOptions => GAME_OPTION_LIST.iter().map(|&id| t(id)).collect(),
			Page::HelpDesk => vec![(format!("{}  {}", text.get(VOICE), onoff(s.help_voice)), true), (format!("{}  {}", text.get(TEXT_ON), onoff(s.help_text)), true)],
			Page::Camera => vec![
				(format!("{}  {}", text.get(CAMERA_ROWS[0]), text.get(if s.cam_lr { NORMAL } else { REVERSED })), true),
				(format!("{}  {}", text.get(CAMERA_ROWS[1]), text.get(if s.cam_ud { NORMAL } else { REVERSED })), true),
				(format!("{}  {}", text.get(CAMERA_ROWS[2]), text.get(SPEEDS[(s.cam_speed as usize).min(2)])), true),
				// Port extra (not in the game's text table, so English only).
				(format!("Camera  {}", if s.cam_modern { "Modern" } else { "Classic" }), true),
				// PC-FPS-1 port extra: render interpolation between the 50 Hz ticks.
				(format!("Smooth motion  {}", if s.smooth { "On" } else { "Off" }), true),
			],
			Page::Subtitles => vec![(format!("{}  {}", text.get(SUBTITLES), onoff(s.subtitles)), true)],
		}
	}

	/// The current page's panels, for the next page to fly from.
	fn panels_now(&self) -> Vec<[f32; 4]> {
		layout(self.page, PAUSE_Y.len(), self.in_game()).panels
	}

	fn enter(&mut self, page: Page, sel: usize) {
		self.from = self.panels_now();
		self.left = Some((self.page, false));
		self.stack.push((self.page, self.sel));
		self.page = page;
		self.sel = sel;
		self.page_time = 0.0;
		self.glow.clear();
	}

	fn back(&mut self) -> bool {
		let Some((p, s)) = self.stack.pop() else { return false };
		self.from = self.panels_now();
		self.left = Some((self.page, true));
		self.page = p;
		self.sel = s;
		self.page_time = 0.0;
		self.glow.clear();
		true
	}

	/// True on pages of the pause menu (Start closes them; Triangle at the root closes).
	pub fn in_game(&self) -> bool {
		// The overwrite confirm entered from New Game is the front end's (retail page 0x1d4db8, viewer/front_mirror.rs).
		let front_overwrite = self.page == Page::Overwrite && self.stack.last().is_some_and(|s| s.0 == Page::NewGame);
		!front_overwrite && !matches!(self.page, Page::Title | Page::Root | Page::NewGame | Page::LoadGame | Page::Options | Page::Language | Page::Video | Page::Graphics) && (self.page != Page::Sound || self.stack.first().is_some_and(|s| s.0 == Page::Pause))
	}

	pub fn update(&mut self, dt: f32) {
		self.time += dt;
		self.page_time += dt;
		// Frame noise pulse (FUN_00223928, decomp/notes/title_assets.md): an idle panel starts one with
		// chance 1/2000 per PAL tick, the counter then climbs 2 per tick and ends above 255 (a capped triangle,
		// not a sine); the 0x1a source origin is re-rolled each tick. Panels are indexed by layout slot, not by
		// moby [P].
		self.ticks = (self.ticks + dt * 50.0).min(8.0);
		while self.ticks >= 1.0 {
			self.ticks -= 1.0;
			self.help_fade = if matches!(self.page, Page::HelpLog | Page::GadgetsHelp) { help_fade_step(self.help_fade, self.sel) } else { (0, -1) };
			for i in 0..self.pulse.len() {
				let c = match self.pulse[i].0 {
					Some(c) => Some(c + 2).filter(|&c| c <= 255),
					None => (self.rand(2000) == 0).then_some(0),
				};
				let o = [self.rand(200) as f32, self.rand(200) as f32];
				self.pulse[i] = (c, o);
			}
		}
		if let Some((_, t)) = &mut self.notice {
			*t -= dt;
			if *t <= 0.0 {
				self.notice = None;
			}
		}
		let f = dt * 50.0;
		// Logo: alpha +1 per PAL tick after the scaled 0x3c-frame delay (50 ticks), up to 0x40, on the title; -0x10
		// per tick once the menu is up. The highlight below also steps per tick (10 ticks).
		self.logo_alpha = if self.page == Page::Title {
			if self.time * 50.0 > 50.0 { (self.logo_alpha + f).min(64.0) } else { 0.0 }
		} else {
			(self.logo_alpha - 16.0 * f).max(0.0)
		};
		let sel = self.sel;
		self.glow.resize(sel.max(8) + 1, 0.0);
		for (i, g) in self.glow.iter_mut().enumerate() {
			*g = if i == sel { (*g + f / FADE_FRAMES).min(1.0) } else { (*g - f / FADE_FRAMES).max(0.0) };
		}
	}

	fn header(&self, text: &Text) -> Option<String> {
		let id = match self.page {
			Page::NewGame | Page::LoadGame => SELECT_FILE,
			// The front end's confirm keeps New Game's title item (0x1d4c90).
			Page::Overwrite if !self.in_game() => SELECT_FILE,
			Page::Options => OPTIONS,
			Page::Language => LANGUAGE,
			Page::Video => VIDEO_OPTIONS,
			Page::Graphics => crate::retail::pcext::ID_GRAPHICS_HDR,
			Page::Sound => SOUND_OPTIONS,
			Page::Weapons => WEAPONS,
			Page::Gadgets => GADGETS,
			Page::QuickSelect => QUICK_SELECT,
			Page::Items => ITEMS,
			Page::Goodies => GOODIES,
			Page::Help => HELP_MENU,
			Page::HelpLog => HELP_LOG,
			Page::Controls => CONTROLS,
			Page::Moves => MOVES,
			Page::WeaponsHelp => WEAPONS_HELP,
			Page::GadgetsHelp => GADGETS_HELP,
			Page::GameOptions => GAME_OPTIONS,
			Page::HelpDesk => HELPDESK_OPTIONS,
			Page::SaveGame | Page::Overwrite => SAVE_GAME,
			Page::LoadSaved => LOAD_GAME,
			Page::Camera => CAMERA_OPTIONS,
			Page::Subtitles => SUBTITLES_OPTIONS,
			Page::Quit => QUIT_TITLE,
			Page::Map => return Some(format!("{} {}", text.get(PLANET), self.ingame.planet)),
			Page::Galactic | Page::GalacticInfo => GALACTIC_MAP,
			Page::Title | Page::Root | Page::Pause => return None,
		};
		Some(text.get(id).to_string())
	}

	/// FUN_00212d58(n) on the game's shared rand() stream (crate::particles::Rng; draws interleave with every
	/// other ported consumer, order exact only where all of them are ported [I]).
	fn rand(&mut self, n: u32) -> u32 {
		crate::particles::Rng::shared().small(n)
	}

	/// The overlay, in window pixels.
	/// One panel's art stretched to its rect r (PAL pixels), FUN_00223928 (title_assets.md): the
	/// underlay, 0x1a from a random source origin (FUN_00212d58(2000), (200)) at one texel per pixel,
	/// 0x1c from (0, 0) over (w, 1.5 h), and 0x19 from (1, 1, 62, 62) to (x+1, y+1, w+d, h+d) with d
	/// -2 up to 75 px, -1 up to 150, else 0. Sprite colour taken as 0x80 grey (= 1.0) [I].
	fn draw_frame(&self, out: &mut Vec<Quad>, f: &[(usize, f32, f32); 4], r: [f32; 4], k: f32, ox: f32, slot: usize) {
		let [x, y, w, h] = r;
		let dest = |x: f32, y: f32, w: f32, h: f32| [ox + x * k, y * k, w.max(0.0) * k, h.max(0.0) * k];
		let white = [1.0, 1.0, 1.0, 1.0];
		// Retail look from PCSX2 captures [I]: near-black navy fill, faint scanlines, a gold edge band
		// (dark-bright-dark) straddling the rect; the fx 0x19/0x1c sprites decode with zero alpha, so the
		// band stands in for them [P] (decomp/notes/title_assets.md Port status).
		crate::overlay::rect(out, dest(x, y, w, h), PANEL_FILL);
		// The underlay 0xe99e frame 7 (black RGB, alpha vignette) stretched over the rect, alpha 0x80.
		if f[0].0 != NO_UNDERLAY {
			out.push(Quad { tex: f[0].0, rect: dest(x, y, w, h), uv: [0.0, 0.0, 1.0, 1.0], colour: white, angle: 0.0, corners: None });
		}
		// 0x1a (noise) only during the random pulse: additive, FIX = min(0x80, (0x80 - |c - 0x80|) * 2),
		// one texel per pixel from the rolled origin (FUN_00223928, decomp/notes/title_assets.md).
		if let Some((Some(c), o)) = self.pulse.get(slot) {
			let a = (0x80 - (*c as i32 - 0x80).abs()).saturating_mul(2).min(0x80) as f32 / 128.0;
			let (tw, th) = (f[1].1, f[1].2);
			out.push(Quad { tex: f[1].0, rect: dest(x, y, w, h), uv: [o[0] / tw, o[1] / th, (o[0] + w) / tw, (o[1] + h) / th], colour: [1.0, 1.0, 1.0, -a], angle: 0.0, corners: None });
		}
		out.push(Quad { tex: f[2].0, rect: dest(x, y, w, h), uv: [0.0, 0.0, w / f[2].1, 1.5 * h / f[2].2], colour: SCANLINE_TINT, angle: 0.0, corners: None });
		for (inset, t, c) in [(-2.0, 5.0, GOLD_DARK), (-1.0, 3.0, GOLD), (-0.5, 1.0, GOLD_HI)] {
			let mut e = |x0: f32, y0: f32, ww: f32, hh: f32| crate::overlay::rect(out, dest(x0, y0, ww, hh), c);
			e(x + inset, y + inset, w - 2.0 * inset, t);
			e(x + inset, y + h - inset - t, w - 2.0 * inset, t);
			e(x + inset, y + inset + t, t, h - 2.0 * inset - 2.0 * t);
			e(x + w - inset - t, y + inset + t, t, h - 2.0 * inset - 2.0 * t);
		}
		let d = |n: f32| if n <= 75.0 { -2.0 } else if n <= 150.0 { -1.0 } else { 0.0 };
		let (bw, bh) = (f[3].1, f[3].2);
		out.push(Quad { tex: f[3].0, rect: dest(x + 1.0, y + 1.0, w + d(w), h + d(h)), uv: [1.0 / bw, 1.0 / bh, 63.0 / bw, 63.0 / bh], colour: white, angle: 0.0, corners: None });
	}

	pub fn draw(&self, out: &mut Vec<Quad>, w: f32, h: f32, art: &Art, text: &Text) {
		let k = h / SCREEN[1];
		let ox = (w - SCREEN[0] * k) / 2.0;
		let logo_alpha = self.ram_alpha.map_or(self.logo_alpha, |a| a.0);
		if let Some(t) = art.logo.filter(|_| logo_alpha > 0.0) {
			// Modulated by 0x808080 with alpha 0x40 at most: half opacity (PS2 0x80 = 1.0).
			out.push(Quad { tex: t, rect: [ox + 236.0 * k, 16.0 * k, 256.0 * k, 128.0 * k], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0, 1.0, 1.0, logo_alpha / 128.0], angle: 0.0, corners: None });
		}
		if let (Some((_, p)), Page::Title) = (self.ram_alpha, self.page) {
			// Retail's own Press Start alpha DAT_0015ef54 (FUN_001eafa8 computes it; 0 = not drawn).
			if let Some(&t) = art.press.get(self.press_lang).filter(|_| p > 0.0) {
				out.push(Quad { tex: t, rect: [ox + 160.0 * k, (SCREEN[1] - 80.0) * k, 192.0 * k, 96.0 * k], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0, 1.0, 1.0, p / 128.0], angle: 0.0, corners: None });
			}
			return;
		}
		if self.page == Page::Title {
			let frames = self.time * 60.0;
			if let Some(&t) = art.press.get(self.lang).filter(|_| frames > 120.0) {
				let a = (((frames - 120.0) % 60.0) * 0.104_719_76 - std::f32::consts::PI).sin() * 32.0 + 96.0;
				out.push(Quad { tex: t, rect: [ox + 160.0 * k, (SCREEN[1] - 80.0) * k, 192.0 * k, 96.0 * k], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0, 1.0, 1.0, a / 128.0], angle: 0.0, corners: None });
			}
			return;
		}
		// Glyphs ('\x10 Select file') name the keys while the keyboard is the device (retail/prompts.rs).
		let header = self.header(text).map(|h| crate::retail::prompts::for_text(&h));
		// Capture-tuned white kept at its old on-screen value (1.6 * 128 / 255) [I].
		let white = [0.80313724, 0.80313724, 0.80313724, 1.0];
		let items = self.items(text);
		let lay = layout(self.page, items.len(), self.in_game());
		// Fly-in (FUN_00276560, decomp/notes/title_assets.md "Seq semantics"): each panel is the class
		// 0x472 slot whose rest rect it is, posed at 0.5 frame per tick for 12 ticks. Entering a page plays
		// its slot seq forward from frame 0 (the parent's rect); a root page (no parent) plays its intro from
		// the last frame back to its resting frame 0; going back plays the left child's seq reversed. The
			// page state switches at once; retail switches after the 12 ticks but exits the old items when the move starts
			// and draws none until then (FUN_00276560), and input is blocked meanwhile, so the picture is the same. Every
			// panel of every page has a slot seq (test `panel_slots`); the lerp / grow arms only cover missing art.
		let ticks = (self.page_time * 50.0).min(MOVE_TICKS);
		let grow = ticks / MOVE_TICKS;
		let seqs = slot_seqs(self.page, self.in_game());
		for (i, r) in lay.panels.iter().enumerate() {
			let flown = art.pics.panels.as_deref().zip(seqs).and_then(|(a, (s, root))| {
				let rest = |seq: u8| if root { 0.0 } else { last_frame(a, seq) };
				let slot = (0..14).find(|&j| slot_rect(a, s[j], rest(s[j])).is_some_and(|q| (0..4).all(|c| (q[c] - r[c]).abs() <= 1.0)))?;
				let back = self.left.filter(|l| l.1).and_then(|l| slot_seqs(l.0, self.in_game()));
				let (seq, at) = match back {
					Some((c, _)) => (c[slot], last_frame(a, c[slot]) - ticks * 0.5),
					None if root => (s[slot], last_frame(a, s[slot]) - ticks * 0.5),
					None => (s[slot], ticks * 0.5),
				};
				if last_frame(a, seq) == 0.0 {
					return Some(*r);
				}
				slot_rect(a, seq, at)
			});
			let [cx, cy, cw, ch] = match (flown, self.from.get(i)) {
				(Some(q), _) => q,
				(None, Some(f)) => std::array::from_fn(|j| f[j] + (r[j] - f[j]) * grow),
				(None, None) => {
					let (cw, ch) = (r[2] * grow, r[3] * grow);
					[r[0] + (r[2] - cw) / 2.0, r[1] + (r[3] - ch) / 2.0, cw, ch]
				}
			};
			// Shadow (x+2, y+2)-(x+w-1, y+h-1) in 0x80100808, then the panel.
			crate::overlay::rect(out, [ox + (cx + 2.0) * k, (cy + 2.0) * k, (cw - 3.0).max(0.0) * k, (ch - 3.0).max(0.0) * k], [16.0 / 255.0, 8.0 / 255.0, 8.0 / 255.0, 1.0]);
			match art.frame {
				Some(f) => self.draw_frame(out, &f, [cx, cy, cw, ch], k, ox, i),
				None => crate::overlay::rect(out, [ox + cx * k, cy * k, (cw - 1.0).max(0.0) * k, (ch - 1.0).max(0.0) * k], PANEL),
			}
		}
		if grow < 1.0 {
			return self.draw_notice(out, art, text, k, ox);
		}
		if let (Some(hd), Some(r)) = (&header, lay.title) {
			// Headers are lavender like unselected rows (retail capture) [I].
			fit_text(out, &art.font, hd, [r[0] + 6.0, r[1] + 4.0, r[2] - 12.0, r[3] - 8.0], k, ox, UNSELECTED);
		}
		let colour = |i: usize, enabled: bool| -> [f32; 4] {
			if !enabled {
				// Type 0 (not selectable) text: 0x80303030, 0x80006060 while focused.
				return if i == self.sel { [0x60 as f32 / 255.0, 0x60 as f32 / 255.0, 0.0, 1.0] } else { [0x30 as f32 / 255.0, 0x30 as f32 / 255.0, 0x30 as f32 / 255.0, 1.0] };
			}
			let g = self.glow.get(i).copied().unwrap_or(0.0);
			std::array::from_fn(|j| UNSELECTED[j] + (SELECTED[j] - UNSELECTED[j]) * g)
		};
		match &lay.rows {
			Rows::Each(rects) => {
				for (i, ((label, enabled), r)) in items.iter().zip(rects).enumerate() {
					fit_text(out, &art.font, label, [r[0] + 4.0, r[1] + 2.0, r[2] - 8.0, r[3] - 3.0], k, ox, colour(i, *enabled));
				}
			}
			Rows::Column(r) if !matches!(self.page, Page::Sound | Page::GadgetsHelp) => {
				let (first, rects) = column(*r, items.len(), self.sel);
				for (j, rr) in rects.iter().enumerate() {
					let i = first + j;
					fit_text(out, &art.font, &items[i].0, *rr, k, ox, colour(i, items[i].1));
				}
			}
			Rows::Slots(strip, more) => {
				for (i, rr) in slot_rects(*strip, items.len()).iter().enumerate() {
					if i == self.sel {
						let g = |c: u8| c as f32 / 255.0;
						crate::overlay::rect(out, [ox + rr[0] * k, (rr[1] + 2.0) * k, rr[2] * k, rr[3] * k], [g(0x20), g(0xff), g(0xff), 0.35]);
					}
					// "name  time  bolts": one line each.
					let parts: Vec<&str> = items[i].0.split("  ").collect();
					let lh = (rr[3] - 8.0) / parts.len() as f32;
					for (j, part) in parts.iter().enumerate() {
						fit_text(out, &art.font, part, [rr[0] + 4.0, rr[1] + 4.0 + j as f32 * lh, rr[2] - 8.0, lh.min(16.0)], k, ox, colour(i, items[i].1));
					}
				}
				if let (Some(m), Some((label, en))) = (more, items.get(crate::save::SLOTS)) {
					fit_text(out, &art.font, label, [m[0] + 6.0, m[1] + 4.0, m[2] - 12.0, m[3] - 8.0], k, ox, colour(crate::save::SLOTS, *en));
				}
				// The selected slot's summary in the detail panel.
				if let Some((label, true)) = items.get(self.sel).filter(|_| self.sel < crate::save::SLOTS && self.slots[self.sel].is_some()) {
					for (j, part) in label.split("  ").enumerate() {
						fit_text(out, &art.font, part, [SLOT_INFO[0] + 8.0, SLOT_INFO[1] + 12.0 + j as f32 * 32.0, SLOT_INFO[2] - 16.0, 24.0], k, ox, white);
					}
				}
			}
			_ => {}
		}
		match self.page {
			// Sound page 0x27a3d8, relative to the list panel (w, h): rows at h/4, h/2, 3h/4, label at
			// (w/2-8, y-8) [I: right-aligned there], bar frame 0x80696969 (w/2+7, y-8)-(w-0x3f, y+8), inner
			// 0x80383838 (w/2+9, y-6)-(w-0x41, y+6), fill = icon 0xe99e frame 8/9 from w/2+9 to
			// w/2+8+(w-(w/2+0x4a))*vol/1024, y-6..y+5 (31x10 px of the frame, stretched; the bar sits in
			// rows 11..21 of the 32x32 frame, so v starts at 11 [I]).
			Page::Sound => {
				let Rows::Column(p) = lay.rows else { return };
				let (pw, ph) = (p[2].floor(), p[3].floor());
				let hw = (pw / 2.0).floor();
				for (i, (label, en)) in items.iter().enumerate().take(3) {
					let y = p[1] + (ph / 4.0).floor() * (i + 1) as f32;
					let tw = (crate::help::width(&art.font, label)).min(hw - 16.0);
					fit_text(out, &art.font, label, [p[0] + hw - 8.0 - tw, y - 8.0, tw, 16.0], k, ox, colour(i, *en));
					if i == 2 {
						continue;
					}
					let v = if i == 0 { self.settings.sfx } else { self.settings.music };
					let g = |c: u8| c as f32 / 255.0;
					let px = |x0: f32, y0: f32, x1: f32, y1: f32| [ox + (p[0] + x0) * k, y0 * k, (x1 - x0).max(0.0) * k, (y1 - y0).max(0.0) * k];
					crate::overlay::rect(out, px(hw + 7.0, y - 8.0, pw - 63.0, y + 8.0), [g(0x69), g(0x69), g(0x69), 1.0]);
					crate::overlay::rect(out, px(hw + 9.0, y - 6.0, pw - 65.0, y + 6.0), [g(0x38), g(0x38), g(0x38), 1.0]);
					let x1 = hw + 8.0 + ((pw - (hw + 74.0)) * v as f32 / 1024.0).floor();
					match art.vol[i] {
						Some((slot, tw, th)) => out.push(Quad { tex: slot, rect: px(hw + 9.0, y - 6.0, x1, y + 5.0), uv: [0.0, 11.0 / th, (31.0 / tw).min(1.0), (21.0 / th).min(1.0)], colour: [1.0, 1.0, 1.0, 1.0], angle: 0.0, corners: None }),
						None => crate::overlay::rect(out, px(hw + 9.0, y - 6.0, x1, y + 5.0), [UNSELECTED[0], UNSELECTED[1], UNSELECTED[2], 1.0]),
					}
				}
			}
			// The selected entry's help text in panel 2 (item 0x1b44a8; the selection link is inferred) [I].
			Page::HelpLog => {
				if let Some((_, body)) = self.ingame.help_log.get(self.sel) {
					let r = HELP_BODY;
					let lines = crate::help::wrap(&art.font, body, r[2] - 16.0);
					let lh = ((r[3] - 16.0) / lines.len().max(1) as f32).min(16.0);
					let c = help_body_colour(self.help_fade.1);
					for (i, l) in lines.iter().enumerate() {
						fit_text(out, &art.font, l, [r[0] + 8.0, r[1] + 8.0 + i as f32 * lh, r[2] - 16.0, lh], k, ox, c);
					}
				}
			}
			// Weapons / Quick Select (retail captures) [I]: owned icons in a grid with the cursor square,
			// the picked weapon's name and ammo, the button legend. Quick Select adds the 8-slot ring in
			// the big panel with the L1 / R1 glyphs; its name and ammo follow the weapon in the ring slot
			// `sel` [I]. The weapon preview is `Menu::preview`, the hero in panel 1 is
			// drawn by the viewer (`hero_preview_clip`); ammo models are not drawn [P].
			Page::Weapons | Page::QuickSelect => {
				let ws = self.grid_weapons();
				let quick = self.page == Page::QuickSelect;
				let ids: Vec<u8> = ws.iter().map(|w| w.0).collect();
				// Quick Select: the grid cursor is on the weapon assigned to the ring slot `sel` (none if empty) [I].
				let cur = if quick { self.ingame.quick_ids.get(self.sel).and_then(|q| ids.iter().position(|i| i == q)) } else { Some(self.sel) };
				icon_grid(out, art, &ids, cur, lay.panels[2], k, ox);
				if let Some((_, name, ammo)) = cur.and_then(|c| ws.get(c)) {
					let ammo = ammo.map_or_else(|| text.get(NO_AMMO).to_string(), |(n, m)| format!("{n}/{m}"));
					for (s, r) in [(name.as_str(), lay.panels[4]), (ammo.as_str(), lay.panels[6])] {
						fit_text(out, &art.font, s, [r[0] + 8.0, r[1] + 4.0, r[2] - 16.0, r[3] - 8.0], k, ox, UNSELECTED);
					}
				}
				legend(out, &art.font, &[text.get(EQUIP), text.get(EXIT)], lay.panels[7], k, ox);
				if quick {
					quick_ring(out, art, &self.ingame.quick_ids, &self.ingame.golds, self.sel, (self.time * 50.0) as i32, lay.panels[1], k, ox);
				}
			}
			Page::Gadgets => {
				for (&id, (r, _)) in GADGET_LISTS.iter().zip(GADGET_PANELS) {
					fit_text(out, &art.font, text.get(id), [r[0] + 3.0, r[1] + 4.0, r[2] - 6.0, r[3] - 8.0], k, ox, UNSELECTED);
				}
				for (i, &(rows, cols, cells, worn_frame)) in GADGET_GRIDS.iter().enumerate() {
					let r = GADGET_PANELS[i].1;
					let (opts, on) = &self.ingame.wear[i];
					let cur = (self.sel.min(3) == i).then_some(self.cell[i]);
					let pulse = (self.time * 50.0) as i32;
					gadget_grid(out, art, self.grid_scale(art, r), (rows, cols), cells, &self.ingame.golds, |id| opts.iter().any(|o| o.0 == id), |id| worn_frame && *on == id, cur, pulse, r, k, ox);
				}
				// Item text s4 (+0x34 = the 0x4c-byte item records): the cursor cell's name when owned [I].
				let i = self.sel.min(3);
				let id = GADGET_GRIDS[i].2[self.cell[i].min(GADGET_GRIDS[i].2.len() - 1)];
				if let Some((_, name)) = self.ingame.wear[i].0.iter().find(|o| o.0 == id) {
					let r = lay.panels[2];
					fit_text(out, &art.font, name, [r[0] + 8.0, r[1] + 4.0, r[2] - 16.0, r[3] - 8.0], k, ox, UNSELECTED);
				}
				legend(out, &art.font, &[text.get(EQUIP), text.get(EXIT)], lay.panels[4], k, ox);
			}
			// Items: Gold Bolts in orange, Found / Used / Remain right-aligned with their counts and a
			// rule over Remain (retail capture) [I].
			Page::Items => {
				// The 3x3 grid (FUN_0027af20 over V 0x1b5e00) in the big panel: owned cells show their icon, the
				// cursor cell its name and description. Cell spacing and the panel choice follow the capture [I].
				let g = lay.panels[3];
				let (cw, ch) = (g[2] / 3.0, g[3] / 3.0);
				for i in 0..9 {
					let (x, y) = (g[0] + (i % 3) as f32 * cw, g[1] + (i / 3) as f32 * ch);
					let s = 48.0f32.min(cw - 8.0).min(ch - 8.0);
					let (ix, iy) = (x + (cw - s) / 2.0, y + (ch - s) / 2.0);
					if let (Some(Some(_)), Some(&(slot, _, _))) = (self.ingame.items.get(i), art.pics.items_grid[i].as_ref()) {
						out.push(Quad { tex: slot, rect: [ox + ix * k, iy * k, s * k, s * k], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
					}
					if i == self.sel.min(8) {
						outline(out, [ix - 3.0, iy - 3.0, s + 6.0, s + 6.0], 2.0, SEL_EDGE, k, ox);
					}
				}
				if let Some(Some((name, desc))) = self.ingame.items.get(self.sel.min(8)) {
					let r = lay.panels[6];
					fit_text(out, &art.font, name, [r[0] + 6.0, r[1] + 4.0, r[2] - 12.0, r[3] - 8.0], k, ox, UNSELECTED);
					let r = lay.panels[4];
					for (j, l) in crate::help::wrap(&art.font, desc, r[2] - 16.0).iter().enumerate().take(((r[3] - 8.0) / 16.0) as usize) {
						left_text(out, &art.font, l, r[0] + 8.0, r[1] + 4.0 + j as f32 * 16.0, 16.0, r[2] - 16.0, UNSELECTED, k, ox);
					}
				}
				let r = lay.panels[2];
				let gold = plain(text.get(GOLD_BOLTS));
				let lines = crate::help::wrap(&art.font, &gold, 80.0);
				for (i, l) in lines.iter().enumerate() {
					let y = r[1] + r[3] / 2.0 - 8.0 * lines.len() as f32 + 16.0 * i as f32;
					fit_text(out, &art.font, l, [r[0] + 14.0, y, 80.0, 16.0], k, ox, GOLD_TEXT);
				}
				let (found, used) = self.ingame.gold;
				for (i, (&id, n)) in GOLD_STATS.iter().zip([found, used, found.saturating_sub(used)]).enumerate() {
					let y = r[1] + 14.0 + i as f32 * 27.0;
					let label = text.get(id);
					let tw = crate::help::width(&art.font, label).min(110.0);
					fit_text(out, &art.font, label, [r[0] + 205.0 - tw, y, tw, 16.0], k, ox, UNSELECTED);
					fit_text(out, &art.font, &n.to_string(), [r[0] + 212.0, y, 30.0, 16.0], k, ox, UNSELECTED);
				}
				let c = [92.0 / 255.0, 108.0 / 255.0, 170.0 / 255.0, 1.0];
				crate::overlay::rect(out, [ox + (r[0] + 210.0) * k, (r[1] + 63.0) * k, 34.0 * k, 2.0 * k], c);
				legend(out, &art.font, &[text.get(EXIT)], lay.panels[5], k, ox);
			}
			// Help / Options: the selected row's description in the left panel, Select / Exit in the
			// bottom one, the row's help_ss / options_ss picture in the right one.
			Page::Help | Page::GameOptions => {
				let info = if self.page == Page::Help { HELP_INFO.get(self.sel) } else { GAME_OPTION_INFO.get(self.sel) };
				if let Some(&id) = info {
					let r = lay.panels[1];
					for (i, l) in crate::help::wrap(&art.font, &plain(text.get(id)), r[2] - 12.0).iter().enumerate().take(8) {
						let tw = crate::help::width(&art.font, l);
						fit_text(out, &art.font, l, [r[0] + 6.0, r[1] + 6.0 + i as f32 * 17.0, tw, 16.0], k, ox, UNSELECTED);
					}
				}
				legend(out, &art.font, &[text.get(SELECT), text.get(EXIT)], lay.panels[4], k, ox);
				// The picture item 0x27d3a8 (flags 0x20, 147x133) shows help_ss / options_ss [selected row]
				// in the right panel: the top left 147x133 of the 256x256 texture at 1:1 [I centring].
				let pics = if self.page == Page::Help { &art.pics.help } else { &art.pics.options };
				if let Some(&Some(slot)) = pics.get(self.sel) {
					let r = lay.panels[2];
					let (x, y) = (r[0] + (r[2] - 147.0) / 2.0, r[1] + (r[3] - 133.0) / 2.0);
					out.push(Quad { tex: slot, rect: [ox + x * k, y * k, 147.0 * k, 133.0 * k], uv: [0.0, 0.0, 147.0 / 256.0, 133.0 / 256.0], colour: [1.0; 4], angle: 0.0, corners: None });
				}
			}
			// Gadgets Help: the owned gadgets as an icon column (slot 1), the entry's picture (slot 2), its move
			// name centred (slot 4) and description from (4, 4) (slot 8); both texts follow the fade counter.
			// Slot rects come from the panel mobies, else the layout rects.
			Page::GadgetsHelp => {
				let slot = |seq: u8, fallback: usize| {
					art.pics.panels.as_deref().and_then(|a| slot_rect(a, seq, last_frame(a, seq))).unwrap_or(lay.panels[fallback])
				};
				let list = slot(0x6f, 1);
				for (i, &(id, _)) in self.ingame.gadget_help.iter().enumerate() {
					let (cx, cy) = (list[0] + 8.0, list[1] + 4.0 + i as f32 * CELL[1]);
					if cy + CELL[1] > list[1] + list[3] {
						break;
					}
					if let Some(&(tex, _, _)) = art.icons.get(&id) {
						out.push(Quad { tex, rect: [ox + (cx + 3.0) * k, (cy + 3.0) * k, (CELL[0] - 6.0) * k, (CELL[1] - 6.0) * k], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
					}
					if i == self.sel {
						outline(out, [cx + 1.0, cy + 1.0, CELL[0] - 2.0, CELL[1] - 2.0], 2.0, SEL_EDGE, k, ox);
					}
				}
				if let Some((_, cell)) = self.gadget_entry() {
					if let Some(&Some(tex)) = art.pics.gadget_help.get(cell) {
						let r = slot(0x70, 3);
						out.push(Quad { tex, rect: [ox + r[0] * k, r[1] * k, 238.0 * k, 159.0 * k], uv: [0.0, 0.0, 238.0 / 256.0, 159.0 / 256.0], colour: [1.0; 4], angle: 0.0, corners: None });
					}
				}
				// The texts show the entry that finished fading in; no text before the first fade tick.
				if self.help_fade.1 >= 0 {
					if let Some(&(id, _)) = self.ingame.gadget_help.get(self.help_fade.0) {
						if let Some(&(_, desc, mv)) = GADGET_HELP_TEXT.iter().find(|e| e.0 == id) {
							let c = help_body_colour(self.help_fade.1);
							fit_text(out, &art.font, text.get(mv), slot(0x72, 4), k, ox, c);
							let r = slot(0x75, 7);
							let font = art.font2.as_ref().unwrap_or(&art.font);
							for (i, l) in crate::help::wrap(font, &plain(text.get(desc)), r[2] - 8.0).iter().enumerate() {
								let tw = crate::help::width(font, l);
								fit_text(out, font, l, [r[0] + 4.0, r[1] + 4.0 + i as f32 * 16.0, tw, 16.0], k, ox, c);
							}
						}
					}
				}
			}
			// Planet map: area name in the right title, the spinning globe (name text without the bundle),
			// the button legend (retail capture) [I].
			Page::Map => {
				let r = lay.panels[0];
				fit_text(out, &art.font, &self.ingame.area, [r[0] + 8.0, r[1] + 4.0, r[2] - 16.0, r[3] - 8.0], k, ox, UNSELECTED);
				let g = lay.panels[3];
				match art.pics.globe {
					Some(layers) => globe(out, layers, art.pics.globe_inset, art.pics.globe_planet, g, (self.page_time * 50.0) as u32, k, ox),
					None => fit_text(out, &art.font, &self.ingame.planet, [g[0] + 10.0, g[1] + g[3] / 2.0 - 10.0, g[2] - 20.0, 20.0], k, ox, SELECTED),
				}
				let rows: Vec<String> = [VIEW_MISSIONS, PLAY_INFOBOT, PREV_MAP, NEXT_MAP, TRACK_ZOOM, EXIT].iter().map(|&id| plain(text.get(id))).collect();
				let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
				legend(out, &art.font, &rows, lay.panels[4], k, ox);
			}
			Page::Galactic | Page::GalacticInfo => galactic(self, out, art, text, &lay, k, ox),
			// The front end's overwrite confirm (page 0x1d4db8): FUN_00221650 draws text 0x4fb3 centred in the body panel (colour
			// 0x80ffa888), the label item lists Circle Yes 0x524f / Triangle No 0x524b, New Game's slot list stays up with its cursor.
			Page::Overwrite if !self.in_game() => {
				let r = lay.panels[2];
				let lines = crate::help::wrap(&art.font, text.get(OVERWRITE), r[2] - 16.0);
				let top = r[1] + ((r[3] - lines.len() as f32 * 16.0) / 2.0).max(4.0);
				for (i, l) in lines.iter().enumerate() {
					let tw = crate::help::width(&art.font, l).min(r[2] - 16.0);
					fit_text(out, &art.font, l, [r[0] + (r[2] - tw) / 2.0, top + i as f32 * 16.0, tw, 16.0], k, ox, UNSELECTED);
				}
				legend(out, &art.font, &[text.get(YES), text.get(NO)], lay.panels[3], k, ox);
				let rows = self.slot_rows(text, true);
				for (i, rr) in slot_rects(SLOT_STRIP, crate::save::SLOTS).iter().enumerate() {
					if i == self.sel {
						let g = |c: u8| c as f32 / 255.0;
						crate::overlay::rect(out, [ox + rr[0] * k, (rr[1] + 2.0) * k, rr[2] * k, rr[3] * k], [g(0x20), g(0xff), g(0xff), 0.35]);
					}
					let parts: Vec<&str> = rows[i].0.split("  ").collect();
					let lh = (rr[3] - 8.0) / parts.len() as f32;
					for (j, part) in parts.iter().enumerate() {
						fit_text(out, &art.font, part, [rr[0] + 4.0, rr[1] + 4.0 + j as f32 * lh, rr[2] - 8.0, lh.min(16.0)], k, ox, colour(i, true));
					}
				}
			}
			Page::Quit | Page::Overwrite => {
				let body = text.get(if self.page == Page::Quit { QUIT_BODY } else { OVERWRITE });
				let r = lay.panels[if self.page == Page::Quit { 0 } else { 2 }];
				let top = r[1] + if self.page == Page::Quit { 36.0 } else { 12.0 };
				let lines = crate::help::wrap(&art.font, body, r[2] - 16.0);
				let lh = ((r[1] + r[3] - 40.0 - top) / lines.len().max(1) as f32).min(16.0);
				for (i, l) in lines.iter().enumerate() {
					fit_text(out, &art.font, l, [r[0] + 8.0, top + i as f32 * lh, r[2] - 16.0, lh], k, ox, UNSELECTED);
				}
				// Circle = Yes, Triangle = No (0x27bda0 / 0x27f098); the overwrite page draws no Yes/No
				// itself (ingame_menu_leftovers.md P1-e), shown here like Quit [P].
				let y = r[1] + r[3] - 30.0;
				let named = |id| crate::retail::prompts::for_text(text.get(id));
				fit_text(out, &art.font, &named(NO), [r[0] + 8.0, y, r[2] / 2.0 - 12.0, 20.0], k, ox, UNSELECTED);
				fit_text(out, &art.font, &named(YES), [r[0] + r[2] / 2.0 + 4.0, y, r[2] / 2.0 - 12.0, 20.0], k, ox, UNSELECTED);
				if self.page == Page::Overwrite {
					for (i, rr) in slot_rects(SLOT_STRIP, crate::save::SLOTS).iter().enumerate() {
						let label = self.slot_rows(text, true)[i].0.clone();
						for (j, part) in label.split("  ").enumerate() {
							fit_text(out, &art.font, part, [rr[0] + 4.0, rr[1] + 4.0 + j as f32 * 16.0, rr[2] - 8.0, 16.0], k, ox, colour(i, i == self.pending));
						}
					}
				}
			}
			_ => {}
		}
		self.draw_notice(out, art, text, k, ox);
	}

	/// The busy / failure message box (FUN_002012e0 style), first paragraph for the card texts.
	/// The whole card text: byte 0x01 is a newline, a doubled one the paragraph gap (ingame_save_quit.md).
	fn draw_notice(&self, out: &mut Vec<Quad>, art: &Art, text: &Text, k: f32, ox: f32) {
		let Some((id, _)) = self.notice else { return };
		let body = text.get(id);
		let lines = crate::help::wrap(&art.font, body, 360.0);
		let hgt = lines.len() as f32 * 16.0 + 24.0;
		let top = 224.0 - hgt / 2.0;
		crate::overlay::rect(out, [ox + 66.0 * k, top * k, 380.0 * k, hgt * k], [PANEL[0], PANEL[1], PANEL[2], 0.95]);
		for (i, l) in lines.iter().enumerate() {
			fit_text(out, &art.font, l, [76.0, top + 12.0 + i as f32 * 16.0, 360.0, 16.0], k, ox, [0.80313724, 0.80313724, 0.80313724, 1.0]);
		}
	}
}

/// What retail's message box shows for its words (FUN_001fb780 tail): text ids of the body (a second one follows after a blank
/// line), the Yes / No hints at x = 202 / 309 and the single hint at x = 256 (0 = none), and whether the box is drawn at all.
#[derive(Debug, PartialEq, Eq)]
struct DialogText {
	body: [u32; 2],
	yes: u32,
	no: u32,
	hint: u32,
}

/// The keys of the front end's message box (front mode 4) from the hints it shows (`menu_keys::front_dialog`); None when it
/// shows none (worker, retail.rs `front_snap`).
pub fn dialog_keys(d: &crate::retail::FrontDialog) -> Option<crate::retail::menu_keys::Screen> {
	let s = dialog_text(d)?;
	crate::retail::menu_keys::front_dialog(s.yes, s.no, s.hint)
}

/// The memory card states' messages (FUN_001fb780 kind 3 by the card state DAT_0015eeb0, flags DAT_0015eeb4 `b`, DAT_0015f5c8 `p`),
/// the 60 Hz confirm (kind 6, by DAT_0019329c) and nothing for the in-game kinds. A state without a case draws an empty box.
fn dialog_text(d: &crate::retail::FrontDialog) -> Option<DialogText> {
	let (b, p) = (d.flags, d.card != 0);
	let t = |body: [u32; 2], yes: u32, no: u32, hint: u32| Some(DialogText { body, yes, no, hint });
	match d.kind {
		3 => match d.state {
			2 => t([0x4fa6, 0], 0, 0, if d.fade_in != 0 { 0 } else { 0x524a }),
			3 | 4 if p && b & 2 != 0 => t([0x4fab, 0], 0x524e, 0x524b, 0),
			3 | 4 => t([if b & 4 != 0 { 0x4fa7 } else { 0x4fac }, 0], 0, 0, 0x4fa8),
			5 => t([0x4fb0, 0x4fa7], 0, 0, 0x4fa8),
			6 => t([0x4faf, 0], 0x524f, 0x524b, 0),
			7 | 8 => t([0x4fb7, 0], 0, 0, 0),
			0xa | 0xb => t([0x4fc1, 0], 0, 0, 0),
			0xc if b & 2 == 0 => t([0x4fa7, 0], 0, 0, 0x4fa8),
			0xc | 0xd => t([0x4fad, 0], 0x524f, 0x524b, 0),
			0xe | 0xf => t([0x4fb8, 0], 0, 0, 0),
			0x11 => t([0x4fba, 0], 0, 0, 0x524a),
			0x12 => t([0x4fbc, 0], 0, 0, 0x524a),
			0x13 if b & 4 != 0 => t([0x4fa7, 0], 0, 0, 0x4fa8),
			0x13 if p => t([0x4fa9, 0x4faa], 0x524e, 0x524b, 0),
			0x13 => t([0x4fa9, 0], 0, 0, 0x4fa8),
			0x14 => t([0x4fbb, 0], 0, 0, 0x524a),
			0x15 => t([0x4fbd, 0], 0, 0, 0x524a),
			0x17 => t([0x4fb1, 0], 0x524e, 0x524b, 0),
			0x18 => t([0x4fae, 0x4faa], 0x524e, 0x524b, 0),
			_ => t([0, 0], 0, 0, 0),
		},
		6 => match d.step as i32 {
			0 | 1 => t([0x522a, 0], 0x524e, 0x524b, 0),
			2 => t([0x522b, 0], 0x524e, 0x524b, 0),
			3 => t([0x522c, 0], 0, 0, 0x524a),
			_ => t([0, 0], 0, 0, 0),
		},
		_ => None,
	}
}

/// Retail's word wrap of the message box (FUN_001f6bc0, centred, `full` = 320 px): lines break at a space or a control byte once
/// the width reaches the limit; the limit then shrinks by 16 until the last line is at least a third of it, unless that makes a
/// line more (then the full width is used). The text ends at the end of the string; byte 0x01 starts a new line.
fn dialog_wrap(font: &(usize, Vec<hud::Glyph>), s: &str, full: i32) -> Vec<String> {
	let ch: Vec<u32> = s.chars().map(|c| c as u32).collect();
	let c = |i: isize| if i < 0 { 0 } else { ch.get(i as usize).copied().unwrap_or(0) };
	let adv = |c: u32| font.1.get(c as usize).map_or(0, |g| g.adv as i32);
	// The lines at width limit `w`: (first, last) char index inclusive, and the width of the last line.
	let lines_at = |w: i32| -> (Vec<(isize, isize)>, i32) {
		let mut out = Vec::new();
		let (mut pos, mut last_w) = (0isize, 0);
		if c(0) != 0 && out.len() < 32 {
			loop {
				let start = pos;
				let (mut cand, mut sum, mut p) = (pos, 0, pos);
				let (mut s9, mut s11);
				loop {
					s9 = p;
					if c(p) == 0x20 || c(p) < 0x10 {
						cand = p;
					}
					s11 = cand;
					if c(p) < 2 {
						break;
					}
					sum += adv(c(p));
					p += 1;
					if sum >= w {
						s9 = p;
						s11 = cand;
						break;
					}
				}
				let mut end = if s11 == start { s9 } else { s11 };
				let e = end;
				if c(e) == 0x20 || c(e) < 0x10 {
					end -= 1;
				}
				out.push((start, end));
				last_w = sum;
				if c(e) == 0 || out.len() >= 32 {
					break;
				}
				pos = e + 1;
				if c(pos) == 0 {
					break;
				}
			}
		}
		(out, last_w)
	};
	let (mut w, mut first, mut full_pass) = (full, 0usize, false);
	let lines = loop {
		let (l, last_w) = lines_at(w);
		if full_pass || l.len() < 2 {
			break l;
		}
		if first == 0 {
			first = l.len();
		}
		if first < l.len() {
			full_pass = true;
			w = full;
			continue;
		}
		let third = w / 3;
		w -= 16;
		if third <= last_w || w <= 0 {
			break l;
		}
	};
	lines.iter().map(|&(a, b)| (a..=b).map(|i| char::from_u32(c(i)).unwrap_or(' ')).collect()).collect()
}

impl Menu {
	/// The front end's modal message box (mode 4: FUN_001fb780 draws it, FUN_001fc8f0 runs it, FUN_001fb5e8 opens it) from retail's
	/// own words (`FrontDialog`): the panel (96..416, centred on the screen height DAT_0013e504, alpha 80 of 128 times the fade-in
	/// DAT_00193284 / scale(30) = 25 ticks) with the wrapped text at its top and the hints 20 px above its bottom, which fade in
	/// by DAT_001932a4 [C]. 512x448 space like `draw`.
	pub fn draw_dialog(&self, out: &mut Vec<Quad>, w: f32, h: f32, art: &Art, text: &Text, d: &crate::retail::FrontDialog) {
		let (k, Some(s)) = (h / SCREEN[1], dialog_text(d)) else { return };
		let ox = (w - SCREEN[0] * k) / 2.0;
		const FADE: f32 = 25.0;
		let fade_in = (1.0 - d.fade_in as f32 / FADE).clamp(0.0, 1.0);
		// Glyphs spelled out as key names while the keyboard is the device (prompts.rs; the hints name the box's keys,
		// `dialog_keys`).
		let body = [s.body[0], s.body[1]].iter().filter(|&&id| id != 0).map(|&id| crate::retail::prompts::for_text(text.get(id))).collect::<Vec<_>>().join("\u{1}\u{1}");
		let lines = dialog_wrap(&art.font, &body, 320);
		// Kind 6 has its own panel; kind 3 centres a box 40 px taller than the text on the screen height.
		let (top, bottom, text_y) = if d.kind == 6 {
			(100.0, 300.0, 104.0)
		} else {
			let screen_h = if d.height == 0 { SCREEN[1] } else { d.height as f32 };
			let top = ((screen_h - (lines.len() as f32 * 16.0 + 40.0)) / 2.0).floor();
			(top, top + lines.len() as f32 * 16.0 + 40.0, top + 4.0)
		};
		let hint_y = if d.kind == 6 { 280.0 } else { bottom - 20.0 };
		// FUN_001f5e18: the body and three stepped columns either side (alpha int(fade * 80) of 128, colour 0x040404).
		let a = (fade_in * 80.0).floor() / 128.0;
		let c = [4.0 / 128.0, 4.0 / 128.0, 4.0 / 128.0, a];
		let (x0, x1) = (96.0, 416.0);
		for (rx0, rx1, inset) in [(x0, x1, 0.0), (x0 - 2.0, x0, 1.0), (x0 - 3.0, x0 - 2.0, 2.0), (x0 - 4.0, x0 - 3.0, 4.0), (x1, x1 + 2.0, 1.0), (x1 + 2.0, x1 + 3.0, 2.0), (x1 + 3.0, x1 + 4.0, 4.0)] {
			crate::overlay::rect(out, [ox + rx0 * k, (top + inset) * k, (rx1 - rx0) * k, (bottom - top - 2.0 * inset) * k], c);
		}
		let line = |out: &mut Vec<Quad>, s: &str, cx: f32, y: f32, colour: [f32; 4]| {
			let tw = crate::help::width(&art.font, s);
			fit_text(out, &art.font, s, [cx - tw / 2.0, y, tw, 16.0], k, ox, colour);
		};
		let tc = [UNSELECTED[0], UNSELECTED[1], UNSELECTED[2], fade_in];
		for (i, l) in lines.iter().enumerate() {
			line(out, l, 256.0, text_y + i as f32 * 16.0, tc);
		}
		// Kind 3's hints fade in on their own timer after the box's first 30 ticks; kind 6's with the panel.
		let hint_a = if d.kind == 6 { fade_in } else { (1.0 - d.hint_t as f32 / FADE).clamp(0.0, 1.0) };
		let hc = [SELECTED[0], SELECTED[1], SELECTED[2], hint_a];
		for (id, x) in [(s.yes, 202.0), (s.no, 309.0), (s.hint, 256.0)] {
			if id != 0 {
				line(out, &crate::retail::prompts::for_text(text.get(id)), x, hint_y, hc);
			}
		}
	}
}

/// Planet markers of the galaxy map (x, y, label dx, label dy) for planets 1..18, in map pixels
/// (level_transition.md "Galaxy map").
const GALAXY: [[f32; 4]; 18] = [
	[80.0, 95.0, 20.0, -20.0], [85.0, 130.0, -16.0, -8.0], [90.0, 65.0, -16.0, -16.0], [108.0, 110.0, -16.0, 40.0],
	[160.0, 110.0, 10.0, -16.0], [120.0, 85.0, 16.0, 18.0], [150.0, 150.0, -16.0, 16.0], [180.0, 130.0, 32.0, -12.0],
	[200.0, 110.0, 16.0, 16.0], [220.0, 140.0, 16.0, 16.0], [280.0, 100.0, -16.0, 3.0], [230.0, 80.0, 16.0, -16.0],
	[210.0, 40.0, -16.0, -16.0], [200.0, 50.0, -16.0, -16.0], [160.0, 40.0, -16.0, -16.0], [130.0, 50.0, -16.0, -16.0],
	[45.0, 90.0, 32.0, -60.0], [35.0, 80.0, 32.0, -50.0],
];

/// Left-aligned text with its top at y, `h` px high (shrunk to fit `maxw`).
fn left_text(out: &mut Vec<Quad>, font: &(usize, Vec<hud::Glyph>), s: &str, x: f32, y: f32, h: f32, maxw: f32, colour: [f32; 4], k: f32, ox: f32) {
	let w = crate::help::width(font, s);
	if w <= 0.0 {
		return;
	}
	let scale = (h / 16.0).min(1.5).min(maxw / w);
	fit_text(out, font, s, [x, y, w * scale, 16.0 * scale], k, ox, colour);
}

/// The Galactic Map (page 0xf) and its details page (0x10): planet list, galaxy map with blinking
/// markers, planet picture, legend; the details page has the names, mission checklist, spinning
/// planet, gold-bolt count and its own legend (decomp/notes/level_transition.md "Ship travel and
/// Galactic Map"). The gold bolt model beside the count is drawn by the viewer in `gold_preview`'s rect [P: turntable].
fn galactic(m: &Menu, out: &mut Vec<Quad>, art: &Art, text: &Text, lay: &Layout, k: f32, ox: f32) {
	let Some(t) = &m.ingame.travel else { return };
	let Some(planet) = m.selected_planet() else { return };
	let info = m.page == Page::GalacticInfo;
	let white = [1.0; 4];
	let frames = m.time * 60.0;
	let l = lay.panels[1];
	if !info {
		// List: entries 34 px high, location line then the planet name indented by 16 (FUN_002907c8).
		let rows = ((l[3] / 34.0).floor() as usize).max(1);
		let first = m.sel.saturating_sub(rows - 1).min(t.planets.len().saturating_sub(rows));
		for (j, &p) in t.planets.iter().enumerate().skip(first).take(rows) {
			let y = l[1] + 4.0 + (j - first) as f32 * 34.0;
			let g = if j == m.sel { 0.5 + 0.5 * (frames * 0.1).sin() } else { 0.0 };
			let c: [f32; 4] = std::array::from_fn(|i| UNSELECTED[i] + (SELECTED[i] - UNSELECTED[i]) * g);
			left_text(out, &art.font, &plain(text.get(LOCATION + p as u32)), l[0] + 4.0, y, 13.0, l[2] - 8.0, c, k, ox);
			left_text(out, &art.font, &plain(text.get(PLANET + p as u32)), l[0] + 20.0, y + 15.0, 13.0, l[2] - 24.0, c, k, ox);
		}
		// Galaxy map (FUN_00224d98): black, backdrop, scrolling stars, markers, ring and label.
		let r = lay.panels[2];
		crate::overlay::rect(out, [ox + r[0] * k, r[1] * k, r[2] * k, r[3] * k], [0.0, 0.0, 0.0, 1.0]);
		let full = |tex: usize, u0: f32, u1: f32| Quad { tex, rect: [ox + r[0] * k, r[1] * k, r[2] * k, r[3] * k], uv: [u0, 0.0, u1, 1.0], colour: white, angle: 0.0, corners: None };
		if let Some((s, _, _)) = art.pics.galaxy[2] {
			out.push(full(s, 0.0, 1.0));
		}
		if let Some((s, w, _)) = art.pics.galaxy[3] {
			// U = (tick & 0xfff) in 1/16 texels: 1/16 texel a tick, wrapping every 4096 ticks.
			let f = ((((m.time * 50.0) as u32 & 0xfff) as f32 / 16.0) / w.max(1.0)).fract();
			let split = r[2] * (1.0 - f);
			for (x0, x1, u0, u1) in [(0.0, split, f, 1.0), (split, r[2], 0.0, f)] {
				if x1 > x0 {
					out.push(Quad { tex: s, rect: [ox + (r[0] + x0) * k, r[1] * k, (x1 - x0) * k, r[3] * k], uv: [u0, 0.0, u1, 1.0], colour: white, angle: 0.0, corners: None });
				}
			}
		}
		// PAL: y scaled by 0x1c0 / 0x1a0.
		let ys = 448.0 / 416.0;
		let on = (m.time * 50.0) % 25.0 < 18.0;
		let at = |i: usize| (r[0] + GALAXY[i - 1][0], r[1] + GALAXY[i - 1][1] * ys);
		for i in 1..=18usize {
			let (x, y) = at(i);
			let shown = t.visited.get(i).is_some_and(|&v| v != 0) || (t.unlock.get(i).is_some_and(|&u| u != 0) && on);
			if let (Some((s, _, _)), true) = (art.pics.galaxy[0], shown) {
				out.push(Quad { tex: s, rect: [ox + (x - 5.0) * k, (y - 5.0) * k, 10.0 * k, 10.0 * k], uv: [0.0, 0.0, 1.0, 1.0], colour: white, angle: 0.0, corners: None });
			}
		}
		if (1..=18).contains(&planet) {
			let (x, y) = at(planet);
			let [_, _, dx, dy] = GALAXY[planet - 1];
			if let Some((s, _, _)) = art.pics.galaxy[1] {
				out.push(Quad { tex: s, rect: [ox + (x - 10.0) * k, (y - 10.0) * k, 20.0 * k, 20.0 * k], uv: [0.0, 0.0, 1.0, 1.0], colour: white, angle: 0.0, corners: None });
			}
			// Leader line: black shadow at +1, then 0x80f0f0f0.
			let (ex, ey) = (x + dx, y + dy);
			let n = (dx.abs().max(dy.abs())).ceil().max(1.0) as usize;
			for (off, c) in [(1.0, [0.0, 0.0, 0.0, 1.0]), (0.0, [0.94, 0.94, 0.94, 1.0])] {
				for s in 0..=n {
					let f = s as f32 / n as f32;
					crate::overlay::rect(out, [ox + (x + dx * f + off) * k, (y + dy * f + off) * k, k, k], c);
				}
			}
			let name = plain(text.get(PLANET + planet as u32));
			let w = crate::help::width(&art.font, &name).min(120.0);
			left_text(out, &art.font, &name, if dx < 0.0 { ex - w } else { ex }, ey - 8.0, 16.0, 120.0, UNSELECTED, k, ox);
		}
	} else {
		// Title items (flags 0x4b / 0x24b): the area name right, the planet name left, header colour.
		for (r, base) in [(lay.panels[0], LOCATION), (lay.panels[1], PLANET)] {
			fit_text(out, &art.font, &plain(text.get(base + planet as u32)), [r[0] + 6.0, r[1] + 4.0, r[2] - 12.0, r[3] - 8.0], k, ox, UNSELECTED);
		}
		if let Some(c) = t.missions.get(planet) {
			checklist(out, art, text, c, lay.panels[2], k, ox);
		}
		if let Some(&Some(layers)) = art.pics.globes.get(planet) {
			let inset = crate::map::GLOBE_INSET.get(planet).copied().unwrap_or(0);
			globe(out, layers, inset, planet, lay.panels[3], (m.time * 50.0) as u32, k, ox);
		}
		// Gold bolts (FUN_0027c550): "%s %d %s %d" of 0x4f4f, the planet's nonzero bytes at 0x14bec0 +
		// planet*4, 0x4f53 and the planet total (0x1c4908), right-aligned at w - 16 in the item's pow2
		// target and centred vertically, black shadow at (+1, +1) [I: target-to-panel crop].
		let r = lay.panels[4];
		let found = t.gold.get(planet).copied().unwrap_or(0);
		let s = format!("{} {found} {} {}", plain(text.get(FOUND)), plain(text.get(OF)), GOLD_TOTALS.get(planet).copied().unwrap_or(0));
		let w = crate::help::width(&art.font, &s).min(r[2] - 16.0);
		left_text(out, &art.font, &s, r[0] + r[2] - 16.0 - w, r[1] + (r[3] - 16.0) / 2.0, 16.0, r[2] - 16.0, UNSELECTED, k, ox);
		let rows = [EXIT, INFOBOT, BLAST_OFF].map(|id| plain(text.get(id)));
		let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
		return legend(out, &art.font, &rows, lay.panels[5], k, ox);
	}
	// Planet picture: the top left 306x108 of the 512x128 texture at 1:1.
	let r = lay.panels[4];
	if let Some(&Some(slot)) = art.pics.planets.get(planet) {
		out.push(Quad { tex: slot, rect: [ox + (r[0] + 0.5) * k, (r[1] + 6.0) * k, 306.0 * k, 108.0 * k], uv: [0.0, 0.0, 306.0 / 512.0, 108.0 / 128.0], colour: white, angle: 0.0, corners: None });
	}
	let rows = [plain(text.get(SELECT)), plain(text.get(EXIT))];
	let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
	legend(out, &art.font, &rows, lay.panels[3], k, ox);
}

/// Gold bolts per planet (0x1c4908).
const GOLD_TOTALS: [u32; 19] = [0, 3, 4, 3, 1, 2, 2, 2, 2, 2, 2, 1, 2, 1, 4, 2, 2, 2, 3];
const FOUND: u32 = 0x4f4f;
const OF: u32 = 0x4f53;
/// Details-page legend (labels 0x1b2508): Play Infobot, Blast Off.
const INFOBOT: u32 = 0x4eef;
const BLAST_OFF: u32 = 0x4f71;
/// Checklist heading "Missions" and 'All Missions Complete'.
const MISSIONS_HEAD: u32 = 0x4f59;
const ALL_DONE: u32 = 0x523d;

/// The mission checklist item (FUN_0027cc60; N FUN_002929a8): heading at (4, 4) in 0x80ffa888; rows from
/// y 24, text at x 16 wrapped to the panel, 16 px per line, the first row focused (0x8020ffff; the
/// per-planet cursor +0x30 is never moved) [I]; a checkbox at x 9 centred on the row (y + 16 for two
/// lines, else y + 10): 11x11 frame in 0x80ffa888, 9x9 fill, and the tick 0xe99e frame 1 (30x30 at
/// (-13, -18)) when complete; 'All Missions Complete' centred 8 px below the rows (missions.rs).
fn checklist(out: &mut Vec<Quad>, art: &Art, text: &Text, (rows, all): &(Vec<(u32, bool)>, bool), r: [f32; 4], k: f32, ox: f32) {
	left_text(out, &art.font, &plain(text.get(MISSIONS_HEAD)), r[0] + 4.0, r[1] + 4.0, 16.0, r[2] - 8.0, UNSELECTED, k, ox);
	let mut y = 24.0;
	for (i, &(id, done)) in rows.iter().enumerate() {
		let lines = crate::help::wrap(&art.font, &plain(text.get(id)), r[2] - 20.0);
		if y + 16.0 * lines.len() as f32 > r[3] {
			break;
		}
		let c = if i == 0 { SELECTED } else { UNSELECTED };
		for (j, s) in lines.iter().enumerate() {
			left_text(out, &art.font, s, r[0] + 16.0, r[1] + y + 16.0 * j as f32, 16.0, r[2] - 20.0, c, k, ox);
		}
		let cy = r[1] + y + if lines.len() == 2 { 16.0 } else { 10.0 };
		crate::overlay::rect(out, [ox + (r[0] + 4.0) * k, (cy - 5.0) * k, 11.0 * k, 11.0 * k], UNSELECTED);
		crate::overlay::rect(out, [ox + (r[0] + 5.0) * k, (cy - 4.0) * k, 9.0 * k, 9.0 * k], [0.0, 0.0, 0.0, 1.0]);
		if let (true, Some((s, _, _))) = (done, art.pics.tick) {
			out.push(Quad { tex: s, rect: [ox + (r[0] - 4.0) * k, (cy - 18.0) * k, 30.0 * k, 30.0 * k], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
		}
		y += 16.0 * lines.len() as f32;
	}
	if *all {
		fit_text(out, &art.font, &plain(text.get(ALL_DONE)), [r[0] + 4.0, r[1] + y + 8.0, r[2] - 8.0, 16.0], k, ox, SELECTED);
	}
}

/// Text without the highlight codes (0xc on, 0x8 off), for labels drawn in one colour.
fn plain(s: &str) -> String {
	s.replace(['\u{c}', '\u{8}'], "")
}

/// Button legend rows ("✕ Equip", "△ Exit"...) stacked in panel r, left-aligned at one shared scale
/// and centred as a block (retail captures) [I].
fn legend(out: &mut Vec<Quad>, font: &(usize, Vec<hud::Glyph>), rows: &[&str], r: [f32; 4], k: f32, ox: f32) {
	let named: Vec<String> = rows.iter().map(|s| crate::retail::prompts::for_text(s)).collect();
	let rows: Vec<&str> = named.iter().map(String::as_str).collect();
	let rows = &rows[..];
	let widths: Vec<f32> = rows.iter().map(|s| crate::help::width(font, s)).collect();
	let wmax = widths.iter().copied().fold(1.0, f32::max);
	let lh = 20.0f32.min((r[3] - 10.0) / rows.len().max(1) as f32);
	let scale = ((r[2] - 16.0) / wmax).min(lh / 16.0).min(1.25);
	let x = r[0] + (r[2] - wmax * scale) / 2.0;
	let top = r[1] + (r[3] - lh * rows.len() as f32) / 2.0;
	for (i, (s, w)) in rows.iter().zip(widths).enumerate() {
		fit_text(out, font, s, [x, top + i as f32 * lh + (lh - 16.0 * scale) / 2.0, w * scale, 16.0 * scale], k, ox, UNSELECTED);
	}
}

/// Item icons (frame 0 of 0xea60 + id) in CELL-sized cells, 5 per row from the panel's top left, with
/// the gold cursor square on `sel` (retail capture) [I].
fn icon_grid(out: &mut Vec<Quad>, art: &Art, ids: &[u8], sel: Option<usize>, r: [f32; 4], k: f32, ox: f32) {
	for (i, id) in ids.iter().enumerate() {
		let (cx, cy) = (r[0] + 5.0 + (i % 5) as f32 * CELL[0], r[1] + 5.0 + (i / 5) as f32 * CELL[1]);
		if cy + CELL[1] > r[1] + r[3] {
			break;
		}
		if let Some(&(slot, _, _)) = art.icons.get(id) {
			out.push(Quad { tex: slot, rect: [ox + (cx + 3.0) * k, (cy + 3.0) * k, (CELL[0] - 6.0) * k, (CELL[1] - 6.0) * k], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
		}
		if sel == Some(i) {
			outline(out, [cx + 1.0, cy + 1.0, CELL[0] - 2.0, CELL[1] - 2.0], 2.0, SEL_EDGE, k, ox);
		}
	}
}

fn outline(out: &mut Vec<Quad>, r: [f32; 4], t: f32, c: [f32; 4], k: f32, ox: f32) {
	let [x, y, w, h] = r;
	for e in [[x, y, w, t], [x, y + h - t, w, t], [x, y + t, t, h - 2.0 * t], [x + w - t, y + t, t, h - 2.0 * t]] {
		crate::overlay::rect(out, [ox + e[0] * k, e[1] * k, e[2] * k, e[3] * k], c);
	}
}

/// The Quick Select page's ring in panel r: 8 grey squares, slot i at angle i*45 deg from the top
/// clockwise (as the in-play wheel), the slot's icon, the cursor square on `sel`, and the L1 / R1
/// glyphs at the top corners (retail capture; the yellow arrow sprites are not found) [I].
fn quick_ring(out: &mut Vec<Quad>, art: &Art, ids: &[u8], golds: &[u8], sel: usize, tick: i32, r: [f32; 4], k: f32, ox: f32) {
	// FUN_0027e100: centre = item rect centre, radius min(w, h) / 2 - 40, slot angle i*pi/4 - pi/2.
	let (cx, cy, rad) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2].min(r[3]) / 2.0 - 40.0);
	let square = |out: &mut Vec<Quad>, x: f32, y: f32, half: f32, c: [f32; 4]| crate::overlay::rect(out, [ox + (x - half) * k, (y - half) * k, 2.0 * half * k, 2.0 * half * k], c);
	for i in 0..8 {
		let ang = i as f32 * std::f32::consts::FRAC_PI_4 - std::f32::consts::FRAC_PI_2;
		let (x, y) = (cx + rad * ang.cos(), cy + rad * ang.sin());
		let id = ids.get(i).copied().unwrap_or(0);
		// Selected: a 38 px pulsing square (2v, 2v, v), v = 0x40 + |(tick & 0x3f) - 0x20|, then a 36 px 0x80100808 square.
		if i == sel {
			let v = (0x40 + ((tick & 0x3f) - 0x20).abs()) as f32 / 128.0;
			square(out, x, y, 19.0, [(2.0 * v).min(1.0), (2.0 * v).min(1.0), v, 1.0]);
			square(out, x, y, 18.0, [8.0 / 128.0, 8.0 / 128.0, 16.0 / 128.0, 1.0]);
		}
		// Icon frame 4 when the gold byte 0x13e520 + id is set, else frame 0; 32 px at (x - 0x11, y - 0x11), alpha 0x80.
		let icon = if golds.contains(&id) { art.pics.gold.get(&id).or(art.icons.get(&id)) } else { art.icons.get(&id) };
		match icon.filter(|_| id != 0) {
			Some(&(slot, _, _)) => out.push(Quad { tex: slot, rect: [ox + (x - 17.0) * k, (y - 17.0) * k, 32.0 * k, 32.0 * k], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None }),
			// Empty slot: 30 px square, 0x40404040.
			None => square(out, x, y, 15.0, [0.5; 4]),
		}
	}
	// Arrow sprites (icon 0xe99e frame 0): 32x32 at (8, 7), v flipped (height -32); the right one at
	// x = w-10 with width -32, so mirrored too. The glyph text sits inside them at (40, 15) and (w-60, 15).
	if let Some((slot, _, _)) = art.pics.arrow {
		out.push(Quad { tex: slot, rect: [ox + (r[0] + 8.0) * k, (r[1] + 7.0) * k, 32.0 * k, 32.0 * k], uv: [0.0, 1.0, 1.0, 0.0], colour: [1.0; 4], angle: 0.0, corners: None });
		out.push(Quad { tex: slot, rect: [ox + (r[0] + r[2] - 42.0) * k, (r[1] + 7.0) * k, 32.0 * k, 32.0 * k], uv: [1.0, 1.0, 0.0, 0.0], colour: [1.0; 4], angle: 0.0, corners: None });
	}
	// L1/R1 glyph text 0x80ffa888 (FUN_0027e100).
	fit_text(out, &art.font, "\u{14}", [r[0] + 40.0, r[1] + 15.0, 20.0, 18.0], k, ox, UNSELECTED);
	fit_text(out, &art.font, "\u{15}", [r[0] + r[2] - 60.0, r[1] + 15.0, 20.0, 18.0], k, ox, UNSELECTED);
}

/// The Galactic Map globe (FUN_0027de30 -> FUN_00250880) in panel g: a 256x256 target stretched square into the
/// panel [I]; layer 0 inset by the planet's table value with base U = (tick + planet * 0x2ab) % 0x800 in 1/16
/// texels (repeat wrap, so two quads), layers 1 and 2 on top, static, alpha tested. Stations (ids 6, 13, 17)
/// keep the base still and show layer 1 while tick % 60 < 40, layer 2 while tick % 150 < 90.
fn globe(out: &mut Vec<Quad>, layers: [usize; 3], inset: i32, planet: usize, g: [f32; 4], tick: u32, k: f32, ox: f32) {
	let side = g[2].min(g[3]);
	let s = side / 256.0;
	let (gx, gy) = (g[0] + (g[2] - side) / 2.0, g[1] + (g[3] - side) / 2.0);
	let i = inset as f32 / 16.0;
	let (x, y, w) = (gx + i * s, gy + i * s, (256.0 - 2.0 * i) * s);
	let station = matches!(planet, 6 | 13 | 17);
	let f = if station { 0.0 } else { ((tick + planet as u32 * 0x2ab) % 0x800) as f32 / 16.0 / 128.0 };
	let white = [1.0; 4];
	let part = |x0: f32, x1: f32, u0: f32, u1: f32| Quad { tex: layers[0], rect: [ox + x0 * k, y * k, (x1 - x0) * k, w * k], uv: [u0, 0.0, u1, 1.0], colour: white, angle: 0.0, corners: None };
	let split = x + w * (1.0 - f);
	out.push(part(x, split, f, 1.0));
	if f > 0.0 {
		out.push(part(split, x + w, 0.0, f));
	}
	for (j, &t) in layers[1..].iter().enumerate() {
		if station && !(if j == 0 { tick % 60 < 40 } else { tick % 150 < 90 }) {
			continue;
		}
		out.push(Quad { tex: t, rect: [ox + x * k, y * k, w * k, w * k], uv: [0.0, 0.0, 1.0, 1.0], colour: white, angle: 0.0, corners: None });
	}
}

fn fit_text(out: &mut Vec<Quad>,font: &(usize, Vec<hud::Glyph>), s: &str, r: [f32; 4], k: f32, ox: f32, colour: [f32; 4]) {
	let glyph = |c: char| font.1.get(c as usize).copied().filter(|g| g.adv != 0);
	let width: f32 = s.chars().filter_map(glyph).map(|g| g.adv as f32).sum();
	if width <= 0.0 {
		return;
	}
	let scale = (r[2] / width).min(r[3] / 16.0).min(1.5);
	let x0 = r[0] + (r[2] - width * scale) / 2.0;
	let y0 = r[1] + (r[3] - 16.0 * scale) / 2.0;
	let colour = [colour[0] * TEXT_GAIN, colour[1] * TEXT_GAIN, colour[2] * TEXT_GAIN, colour[3]];
	for (d, c) in [(2.0, [0.0, 0.0, 0.0, colour[3]]), (0.0, colour)] {
		let mut x = x0;
		for ch in s.chars() {
			let Some(g) = glyph(ch) else { continue };
			if ch != ' ' {
				let (u, v) = (g.u as f32, g.v as f32);
				out.push(Quad {
					tex: font.0,
					rect: [ox + (x + d * scale) * k, (y0 + (g.dy as f32 + d) * scale) * k, 16.0 * scale * k, 16.0 * scale * k],
					uv: [u / 256.0, v / 128.0, (u + 16.0) / 256.0, (v + 16.0) / 128.0],
					colour: c,
					angle: 0.0,
					corners: None,
				});
			}
			x += g.adv as f32 * scale;
		}
	}
}

/// One menu tick of the Help Log body fade (label FUN_00277900): counters above scale(10) = 8 clamp first;
/// the first tick sets 8 on the current selection; the shown selection fades in by 3 per tick, a changed
/// selection fades the old body out by 3 and switches at 0.
fn help_fade_step((shown, counter): (usize, i32), sel: usize) -> (usize, i32) {
	if counter == -1 {
		return (sel, 8);
	}
	let counter = counter.min(8);
	if sel == shown {
		(shown, counter + 3)
	} else if counter - 3 <= 0 {
		(sel, 0)
	} else {
		(shown, counter - 3)
	}
}

/// Help Log body colour: mix(f, M, B) per RGBA byte (FUN_001ffe48 = x(1 - f) + y f, truncated), with B the
/// usual light blue 0x80ffa888, M = mix(0.5, A, B) of A = 0x80100808 and f = 1 above counter 8, else 1 - (8 - c) / 8.
fn help_body_colour(counter: i32) -> [f32; 4] {
	let mix = |f: f32, x: [u8; 3], y: [u8; 3]| -> [u8; 3] { std::array::from_fn(|i| (x[i] as f32 * (1.0 - f) + y[i] as f32 * f) as u8) };
	let (a, b) = ([0x08, 0x08, 0x10], [0x88, 0xa8, 0xff]);
	let m = mix(0.5, a, b);
	let f = if counter > 8 { 1.0 } else { 1.0 - (8 - counter) as f32 / 8.0 };
	let c = mix(f, m, b);
	[c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]
}

/// What the title idle clock asks the viewer to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AttractEvent {
	/// Start the attract movie with this `scene::movie` id (63 + slot index).
	Movie(u32),
	/// The black ticks after a movie are over: restart the title.
	Title,
}

/// The title attract loop (decomp/notes/frontend_flow.md UI-11c): 1250 idle PAL ticks on the title page, 10 black
/// ticks, one of the four movies (slots 84-87, the first picked by rand() & 3, then in turn), 4 black ticks.
#[derive(Clone, Debug)]
pub struct Attract {
	idle: u32,
	next: u32,
	pub black: u32,
	then_movie: bool,
	pub playing: bool,
}

impl Attract {

	pub fn new(first: u32) -> Attract {
		Attract { idle: 0, next: first & 3, black: 0, then_movie: false, playing: false }
	}

}

#[cfg(test)]
mod tests {
	// UI-TEXT-1 tests (3) and (6): the coloured pass of `fit_text` is the byte / 128; capture-tuned gold kept.
	#[test]
	fn fit_text_colour_is_bytes_over_128() {
		let glyphs: Vec<crate::hud::Glyph> = (0..128).map(|_| crate::hud::Glyph { u: 0, v: 0, dy: 0, adv: 8 }).collect();
		let font = (3usize, glyphs);
		let mut out = Vec::new();
		super::fit_text(&mut out, &font, "AB", [0.0, 0.0, 40.0, 16.0], 1.0, 0.0, super::UNSELECTED);
		assert_eq!(out.len(), 4);
		assert_eq!(out[0].colour, [0.0, 0.0, 0.0, 1.0]);
		let want = [0.53333336 * 1.9921875, 0.65882355 * 1.9921875, 1.9921875f32, 1.0];
		for q in &out[2..] {
			assert!(q.colour.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-6), "{:?}", q.colour);
			assert!(q.colour.iter().zip([1.0625, 1.3125, 1.9921875, 1.0]).all(|(a, b)| (a - b).abs() < 1e-6));
		}
		assert!((super::GOLD_TEXT[0] * super::TEXT_GAIN - 1.216).abs() < 1e-6);
	}

	
	#[test]
	fn help_log_body_fade_counters_and_colours() {
		let mut st = (0, -1);
		let mut seen = vec![];
		st = help_fade_step(st, 0);
		seen.push(st.1);
		for _ in 0..3 {
			st = help_fade_step(st, 1);
			seen.push(st.1);
		}
		assert_eq!((seen.clone(), st.0), (vec![8, 5, 2, 0], 1));
		for _ in 0..3 {
			st = help_fade_step(st, 1);
			seen.push(st.1);
		}
		assert_eq!(seen[4..], [3, 6, 9]);
		assert_eq!(help_fade_step(st, 1).1, 11);
		let bytes = |c: i32| help_body_colour(c).map(|v| (v * 255.0).round() as u32);
		assert_eq!(bytes(8), [0x88, 0xa8, 0xff, 255]);
		assert_eq!(bytes(5), [0x70, 0x8a, 0xd2, 255]);
		assert_eq!(bytes(2), [0x58, 0x6c, 0xa5, 255]);
		assert_eq!(bytes(0), [0x48, 0x58, 0x87, 255]);
		assert_eq!(bytes(3), [0x60, 0x76, 0xb4, 255]);
		assert_eq!(bytes(6), [0x78, 0x94, 0xe1, 255]);
		assert_eq!(bytes(9), [0x88, 0xa8, 0xff, 255]);
	}

	use super::*;

	/// Lists the text table (id, string) for one language: `RAC_LANG=n cargo test dump_text -- --ignored --nocapture`.
	#[test]
	#[ignore]
	fn dump_text() {
		let mut disc = Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let lang = std::env::var("RAC_LANG").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
		let t = Text::load(&mut disc, lang).unwrap();
		let mut ids: Vec<_> = t.0.iter().collect();
		ids.sort();
		for (id, s) in ids {
			println!("{id:#x}	{s:?}");
		}
	}

	#[test]
	fn panel_slots() {
		let mut disc = Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let l = Level::title(&mut disc).unwrap();
		let a = crate::anim::animation(&l, 0x472).unwrap().unwrap();
		assert_eq!(slot_rect(&a, 0x10, 0.0), Some([155.0, 153.0, 203.0, 39.0]));
		assert_eq!(slot_rect(&a, 0x1b, 6.0), Some([263.0, 25.0, 224.0, 236.0]));
		assert_eq!(slot_rect(&a, 0x1b, 4.0), Some([340.0, 111.0, 80.0, 34.0]));
		let mut misses = Vec::new();
		for (page, in_game) in [(Page::Root, false), (Page::Options, false), (Page::Language, false), (Page::Video, false), (Page::Sound, false), (Page::NewGame, false), (Page::LoadGame, false)]
			.into_iter()
			.chain([Page::Pause, Page::Weapons, Page::Gadgets, Page::QuickSelect, Page::Items, Page::Help, Page::HelpLog, Page::Controls, Page::Moves, Page::WeaponsHelp, Page::GadgetsHelp, Page::GameOptions, Page::HelpDesk, Page::Subtitles, Page::Sound, Page::Camera, Page::SaveGame, Page::LoadSaved, Page::Overwrite, Page::Quit, Page::Goodies, Page::Map, Page::Galactic, Page::GalacticInfo].map(|p| (p, true)))
		{
			let (s, root) = slot_seqs(page, in_game).unwrap();
			for r in layout(page, 6, in_game).panels {
				let hit = (0..14).any(|j| slot_rect(&a, s[j], if root { 0.0 } else { last_frame(&a, s[j]) }).is_some_and(|q| (0..4).all(|c| (q[c] - r[c]).abs() <= 1.0)));
				if !hit {
					misses.push((page, r));
				}
			}
		}
		assert!(misses.is_empty(), "{misses:?}");
	}

	/// REN-1: the title frame sprites' texture alpha and the scanline tint (FUN_00223928).
	#[test]
	fn frame_sprite_alpha() {
		let iso = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		if !iso.exists() {
			return;
		}
		let mut disc = Disc::open(iso).unwrap();
		let l = Level::title(&mut disc).unwrap();
		let lit = |k: usize| fx_texture(&l, k).unwrap().rgba.iter().filter(|p| p[3] > 0).count();
		assert_eq!(lit(0x1c), 128);
		assert_eq!(lit(0x19), 141);
		assert_eq!(SCANLINE_TINT, [0x60 as f32 / 128.0, 0x60 as f32 / 128.0, 0x60 as f32 / 128.0, 0x50 as f32 / 128.0]);
	}

	#[test]
	fn title_assets() {
		let mut disc = Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let t = Text::load(&mut disc, 0).unwrap();
		assert_eq!((t.get(0x4f2a), t.get(0x4f28), t.get(0x4ee7)), ("New Game", "Load Game", "Options"));
		assert_eq!(t.get(EMPTY_SLOT), "EMPTY", "{:?}", t.get(EMPTY_SLOT));
		let l = Level::title(&mut disc).unwrap();
		let logo = logo(&l).unwrap();
		assert!(logo.rgba.iter().any(|p| p[3] == 255) && logo.rgba.iter().any(|p| p[3] == 0));
		assert_eq!((press_start(&l, 0).unwrap().w, press_start(&l, 4).unwrap().h), (256, 128));
	}

}
