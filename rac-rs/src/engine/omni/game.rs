//! What the mod host asks the game pack (engine split stage E2, 2026-10-26): the binding and the spec files built into the exe, the
//! game's RAM words the host reads (reads only), its names for modes, pause pages and HUD icons, the tables of its PC features a mod
//! can replace (clash.rs), and the few readers that are game code (disc map, camera, pause page, memory card).
//!
//! R&C1's is `rac1/omni_bind.rs` (`RAC1`). The pack hands it over with `set` before anything of the host runs (`Retail::load`, again
//! in `omni::init`); nothing in this folder holds an address or a name of one game. Reads go through `get`, a plain static load.

use super::clash::{Feature, Span};
use super::events::Event;
use ee_oracle::cpu::Cpu;
use std::ffi::CStr;
use std::sync::OnceLock;

/// One game's part of the mod host. Addresses are RAM addresses of the PAL disc's layout; the ones marked "through `va`" go through
/// the game's disc map, so the USA disc works too.
pub struct Game {
	/// The game's id in the mod format (`"rac1"`): the host's `game` string; the files next to the exe that override the copies below
	/// are `omni/<id>.json`, `omni/<id>_funcs.txt`, `omni/<id>_reloc.txt`.
	pub id: &'static CStr,
	/// The game binding (mod interface 1.3; binding.rs).
	pub binding: &'static str,
	/// Game functions by number for native mods (funcs.rs).
	pub funcs: &'static str,
	/// Hook point specs (hooks.rs).
	pub reloc: &'static str,
	/// The per-level map of the level programs' global data (data_map.rs).
	pub data_map: &'static str,
	/// Levels are `0..levels`: a level word outside is no level (title screen, a menu, a load).
	pub levels: i32,
	/// The level number word (the same address on every disc).
	pub level_word: u32,
	/// The level-loop exit word: nonzero from the moment the level loop is told to end until the next level starts (through `va`).
	pub exit_word: u32,
	/// The game mode word (through `va`), its values' `omni.mode()` names (one must be "pause": its pages are reported), the modes in
	/// which the hero's pose ops are dropped (movies, cutscenes, shop scenes), and the mode in which the 3D view is a frozen picture.
	pub mode_word: u32,
	pub modes: &'static [(u32, &'static str)],
	pub scene_modes: &'static [u32],
	pub menu_mode: u32,
	/// Pause page id -> the API's page name (others are `page_<hex>`).
	pub pages: &'static [(u32, &'static str)],
	/// The executable's globals end here; the level program's data above it moves per level (`relocate`, data_map.rs).
	pub exe_data_end: u32,
	/// The vblank wait of the boot code (the oracle's `VBLANK_WAIT`): where an unwound retail call is left (frame_wait.rs).
	pub vblank_wait: u32,
	/// The hero cell (the hero moby's pointer), the hero's state word, its death states, its position (draw demo only).
	pub hero_cell: u32,
	pub hero_state: u32,
	pub death_states: &'static [u32],
	pub hero_pos: u32,
	/// The hero attach matrices the game writes every tick (four rows each, stride 0x40) and the attach points that fill them, in order
	/// (pose_read.rs).
	pub attach_matrices: u32,
	pub attach_list: &'static [u32],
	/// The game's pad object: held buttons, buttons newly pressed this field (RAC_OMNI_TRACE only; PAL disc).
	pub pad_words: [u32; 2],
	/// New Game from the title: the level a new game starts in. New Game+: the spec of the function that starts one (newgame.rs).
	pub new_game_level: i32,
	pub ng_plus: &'static str,
	/// The game's segment query for `host.raycast`: its spec, and its hit record (a reference-level data address, mapped per level).
	pub seg_collide: &'static str,
	pub seg_record: u32,
	/// The clash switch (clash.rs): our PC features a mod can replace (the pack's feature ids index this table), the game words each
	/// hook point's mod handler decides, and the index of the keyboard table (replaced while a mod holds the keyboard).
	pub features: &'static [Feature],
	pub hook_writes: &'static [(&'static str, &'static [Span])],
	pub keyboard_feature: usize,
	/// HUD icons by item id (draw_art.rs): (id, name, is a weapon) for the named ones, and the first and last id that has a sheet.
	pub icons: &'static [(u8, &'static str, bool)],
	pub icon_ids: (u8, u8),
	/// The USA disc runs.
	pub usa: fn() -> bool,
	/// The disc map: the address of a PAL address on the disc that runs.
	pub va: fn(u32) -> u32,
	/// The open pause page's id from RAM (`ram` = all 32 MB), given the level word and the mode; None when no page is open.
	pub pause_page: fn(ram: &[u8], level_word: i32, mode: u32) -> Option<u32>,
	/// The render camera in RAM: eye, forward, up and the tan scalar the level publishes; None outside a level.
	pub camera: fn(ram: &[u8], level: u32) -> Option<([f32; 3], [f32; 3], [f32; 3], f32)>,
	/// This field's memory-card events (`saving`, `save_loaded`), read once per field after the EE ran it; `tick` for RAC_OMNI_TRACE.
	pub card: fn(c: &Cpu, tick: u64) -> Vec<Event>,
}

impl Game {
	/// `id` as text (`rac1`): the prefix of the spec files next to the exe.
	pub fn id(&self) -> &'static str {
		self.id.to_str().unwrap_or("game")
	}
}

static GAME: OnceLock<&'static Game> = OnceLock::new();

/// The pack hands its part over (once per process; later calls keep the first).
pub fn set(g: &'static Game) {
	if GAME.set(g).is_ok() {
		super::clash::set_keyboard_feature(g.keyboard_feature);
	}
}

/// The game's part. Set by the pack before the host runs; a call before that is a bug in the pack's start order.
#[inline]
pub fn get() -> &'static Game {
	GAME.get().expect("omni: the game pack did not hand over its part (omni::game::set) before the mod host ran")
}
