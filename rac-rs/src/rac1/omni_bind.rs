//! R&C1's part of the mod host (engine split stage E2, 2026-10-26; moved out of engine/omni): the binding and spec files built into
//! the exe (`omni_bind/`), the RAM words the host reads, R&C1's mode / pause page / HUD icon names, our PC features a mod can replace
//! (the clash switch's table), and the readers that are R&C1 code. `RAC1` is handed to the host (`omni::game::set`) by `Retail::load`.
//!
//! Words (PAL addresses; the 0x15f5xx ones go through `disc::va` so the USA disc works too):
//! - level word 0x15ed84: 0..=18 while a level is loaded; anything else is no level.
//! - level-loop exit 0x15f550: nonzero from the moment the level loop is told to end (travel, quit to title, load game) until the next level
//!   starts. Seen as "unloading"; the level word changing while a level is loaded counts as unloading too.
//! - game mode 0x15f5a4: 0 gameplay, 1 movie, 2 in-engine cutscene, 3 page menu (pause), 4 dialog box, 5 vendor, 6 ship, 7 sprite screen.
//!   Mode names for `omni.mode()`: `gameplay`, `movie`, `cutscene`, `pause` (3), `dialog` (4), `vendor`, `ship`, `screen` (7, full-screen
//!   sprite screens) [I]. Pause pages are the page ids of the page-menu engine (`mouse_menu`, id at page+0x3c).
//! - hero cell 0x1413d0 (the hero moby's pointer, boot data, the same on both discs) and its state word 0x1413d4; the death states are
//!   death_respawn.md section 2 (the same list as modcam.rs GATE_STATES' death part). gHeroPos 0x13F3D0 = rac1.json's hero base 0x13F350 + 0x80.
//! - the nine hero attach matrices `HeroComputeAttachMatrices` -> `MobyEvalAttachLists` writes every tick at 0x13fe10 (stride 0x40); the attach
//!   points (indices into the class attach table `class+0x1c`) whose matrices are stored, in order, are `ATTACH_LIST`.
//! - the pad object (boot data, PAL): held buttons +0x1a0 = 0x13cae0, newly pressed +0x1a4 = 0x13cae4 (decomp/notes/hero_crouch_ledge.md).
//! - the vblank wait of the PAL boot (oracle `VBLANK_WAIT`) 0x118cb0; the executable's data ends at 0x15EF00 (the overlay's starts there).
//!
//! Specs found by code like a mod hook point (tools/make_spec.py): the segment query `seg_collide` (Veldin overlay 0x1efbf0, v1.00 PAL, 4336
//! bytes with padding; `--after func_L00_001EFF50:2 --len 4336`, all 19 levels give one masked hash; its hit record is Veldin's 0x173e40,
//! mapped by the data map), and New Game+ (challenge mode, Veldin 0x2842e8, 764 bytes; `--after func_00227B00:3 --len 764`). A new game
//! from the title starts in Veldin (level 0).

mod card;

use crate::omni::clash::{span, Feature, Span};
use crate::omni::game::Game;

/// The binding built into the exe (mod interface 1.3; THIRD_PARTY.md), used when `omni/rac1.json` is not next to the exe.
pub const BINDING: &str = include_str!("omni_bind/rac1.json");
/// Game functions by number for native mods, over which `omni/rac1_funcs.txt` next to the exe wins.
pub const FUNCS: &str = include_str!("omni_bind/rac1_funcs.txt");
/// Hook point specs, over which `omni/rac1_reloc.txt` next to the exe wins.
pub const RELOC: &str = include_str!("omni_bind/rac1_reloc.txt");
/// The per-level data map (tools/make_data_map.py).
pub const DATA_MAP: &str = include_str!("omni_bind/rac1_data.txt");

/// The feature indices for `omni::clash::off`. Order of `FEATURES`.
pub const MODERN_CAMERA: usize = 0;
pub const STRAFE_AIM: usize = 1;
pub const LOCKON: usize = 2;
pub const KBM: usize = 3;

/// hero + 0x98, the way he faces (retail/modcam.rs, strafe.rs, aim.rs HERO_YAW).
pub const HERO_FACING: Span = span("hero facing 0x13F3E8", 0x13F3E8, 0x13F3EC);
/// hero + 0x180, where the stick moves him (HERO_WISH).
pub const HERO_MOVE_YAW: Span = span("hero move yaw 0x13F4D0", 0x13F4D0, 0x13F4D4);

/// Order = the index constants above. [C from retail/modcam.rs, strafe.rs, aim.rs: what each patches.]
pub static FEATURES: [Feature; 4] = [
	// Follow-camera hand (camera record rows), the hero-attack hand, and in aim mode the hero facing, move yaw, manual-aim override and swing yaw.
	Feature {
		id: "rac-rs.modern-camera",
		label: "rac-rs modern camera",
		hooks: &["rac1.camera.follow.apply_overrides"],
		writes: &[HERO_FACING, HERO_MOVE_YAW, span("manual aim 0x140600", 0x140600, 0x140610), span("swing yaw 0x13FDA8", 0x13FDA8, 0x13FDB0)],
		needs: &[],
	},
	// The RMB strafe mode: jump-chooser flip hands (hero facing, flip state) and the draw-only leg twist; it reads the modern camera's yaw.
	Feature {
		id: "rac-rs.strafe-aim",
		label: "rac-rs strafe aim",
		hooks: &[],
		writes: &[HERO_FACING, span("flip state 0x13F760", 0x13F760, 0x13F7B0)],
		needs: &["rac-rs.modern-camera"],
	},
	// The wide Blaster lock-on: the lock target word (and the crosshair icon).
	Feature { id: "rac-rs.lockon", label: "rac-rs wide lock-on", hooks: &[], writes: &[span("lock target 0x13FDA0", 0x13FDA0, 0x13FDA4)], needs: &[] },
	// The keyboard table (retail/input.rs tick_keys): off while a mod has the keyboard.
	Feature { id: "rac-rs.kbm", label: "rac-rs keyboard table", hooks: &[], writes: &[], needs: &[] },
];

/// Hook point -> the game words its mod handler decides. Only the ones a feature also writes matter; a hook missing here decides no word of ours
/// (pad.process_report: the pad report; advance_animation: animation; set_planar_velocity: velocity; item/vendor/weapon/moby hooks: other state;
/// camera.follow.apply_overrides is listed in `Feature::hooks` of the modern camera instead).
pub static HOOK_WRITES: [(&str, &[Span]); 2] = [
	// turn_to_heading turns hero+0x98 toward hero+0x180.
	("rac1.player.turn_to_heading", &[HERO_FACING]),
	// update_motion_input writes hero+0x180 and the stick-derived motion.
	("rac1.player.update_motion_input", &[HERO_MOVE_YAW]),
];

/// HUD icons with a known name (decomp/notes/omni_weapons.md): (item id, name, is a weapon). Items 2..=36 have a sheet
/// (inventory::GADGETS = 37 items; 0 and 1 are no items).
pub const ICONS: [(u8, &str, bool); 19] = [
	(8, "omniwrench", false),
	(9, "suck_cannon", true),
	(10, "bomb_glove", true),
	(11, "devastator", true),
	(12, "swingshot", false),
	(13, "visibomb", true),
	(14, "taunter", true),
	(15, "blaster", true),
	(16, "pyrocitor", true),
	(17, "mine_glove", true),
	(18, "walloper", true),
	(19, "tesla_claw", true),
	(20, "glove_of_doom", true),
	(21, "morph_o_ray", true),
	(22, "hydrodisplacer", false),
	(23, "ryno", true),
	(24, "drone_device", false),
	(25, "decoy_glove", true),
	(26, "trespasser", false),
];
pub const ICON_IDS: (u8, u8) = (2, 36);

/// The attach points whose matrices the game stores, in order.
pub const ATTACH_LIST: [u32; 9] = [0, 1, 2, 3, 4, 5, 6, 0x1d, 0x1e];

const SEG_COLLIDE: &str = "reloc omni_segcollide entry 0x1e94d8 len 0x10f0
mhash 0xffe7afb7b1d10960
mask 0x18 0xffff
mask 0x108 0x3ffffff
mask 0x348 0x3ffffff
mask 0x760 0xffff
mask 0x7e8 0xffff
mask 0x8b0 0xffff
mask 0x968 0x3ffffff
mask 0xfe0 0xffff
mask 0x1050 0xffff
mask 0x10c4 0xffff
jabs 0x188
jabs 0x210
jabs 0x230
jabs 0x248
jabs 0x2a0
jabs 0x6c8
jabs 0x720
jabs 0x7c0
jabs 0xaf8
jabs 0xd60
jabs 0xe58
jabs 0xe90
jabs 0xfa8
jabs 0x10e8\n";

const NG_PLUS: &str = "reloc omni_ngplus entry 0x272ee0 len 0x2fc\nmhash 0x40e1fbca3f867b95\nmask 0x50 0x3ffffff\nmask 0x6c 0x3ffffff\n\
mask 0x80 0x3ffffff\nmask 0x94 0x3ffffff\nmask 0xb0 0x3ffffff\nmask 0xc8 0x3ffffff\nmask 0xdc 0x3ffffff\nmask 0x110 0x3ffffff\nmask 0x120 0x3ffffff\n\
mask 0x130 0xffff\nmask 0x138 0xffff\nmask 0x174 0x3ffffff\nmask 0x190 0x3ffffff\nmask 0x1e4 0x3ffffff\nmask 0x1fc 0x3ffffff\nmask 0x298 0xffff\n\
mask 0x2a4 0x3ffffff\n";

const MASK: u32 = 0x01ff_ffff;

fn rd(ram: &[u8], a: u32) -> u32 {
	let i = (a & MASK) as usize;
	ram.get(i..i + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn usa() -> bool {
	crate::retail::disc::usa()
}

fn va(pal: u32) -> u32 {
	crate::retail::disc::va(pal)
}

/// The open pause page: the page-menu engine's current page id (`mouse_menu`), in the page-menu mode inside a level.
fn pause_page(ram: &[u8], level_word: i32, mode: u32) -> Option<u32> {
	use crate::retail::mouse_menu;
	if mode == mouse_menu::MODE && (0..=18).contains(&level_word) {
		mouse_menu::read(&|a| rd(ram, a), level_word as u32, mode).map(|m| m.id)
	} else {
		None
	}
}

fn camera(ram: &[u8], level: u32) -> Option<([f32; 3], [f32; 3], [f32; 3], f32)> {
	crate::retail::camera::view_in_ram(ram, level)
}

pub static RAC1: Game = Game {
	id: c"rac1",
	binding: BINDING,
	funcs: FUNCS,
	reloc: RELOC,
	data_map: DATA_MAP,
	levels: 19,
	level_word: 0x15_ED84,
	exit_word: 0x15_F550,
	mode_word: 0x15_F5A4,
	modes: &[(0, "gameplay"), (1, "movie"), (2, "cutscene"), (3, "pause"), (4, "dialog"), (5, "vendor"), (7, "screen"), (6, "ship")],
	scene_modes: &[1, 2, 5],
	menu_mode: 3,
	pages: &[
		(2, "root"),
		(3, "weapons"),
		(4, "gadgets"),
		(5, "quick_select"),
		(6, "items"),
		(7, "help"),
		(8, "options"),
		(9, "goodies"),
		(0xb, "map"),
		(0xd, "quit"),
		(0x15, "goals"),
		(0x17, "moves"),
		(0x1e, "save"),
		(0x1f, "load"),
		(0x24, "skill_points"),
	],
	exe_data_end: 0x15_EF00,
	vblank_wait: 0x118cb0,
	hero_cell: 0x14_13d0,
	hero_state: 0x14_13d4,
	death_states: &[0x3d, 0x47, 0x77, 0x7a, 0x7b, 0x7c, 0x7d, 0x7f, 0x80, 0x82],
	hero_pos: 0x13_F3D0,
	attach_matrices: 0x13_fe10,
	attach_list: &ATTACH_LIST,
	pad_words: [0x13cae0, 0x13cae4],
	new_game_level: 0,
	ng_plus: NG_PLUS,
	seg_collide: SEG_COLLIDE,
	seg_record: 0x17_3e40,
	features: &FEATURES,
	hook_writes: &HOOK_WRITES,
	keyboard_feature: KBM,
	icons: &ICONS,
	icon_ids: ICON_IDS,
	usa,
	va,
	pause_page,
	camera,
	card: card::events,
};

/// Tests of the host that run with R&C1's part (rac1/engine_tests/omni_*.rs).
#[cfg(test)]
pub fn for_tests() {
	crate::omni::game::set(&RAC1);
}
