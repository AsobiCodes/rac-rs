//! Fixed boot-RAM addresses (level byte, held moby, moby table) and the game-mode predicates built on the
//! mode words (frozen view, scene view, vendor).

use super::*;

pub(super) const LEVEL: u32 = 0x15ed84;

/// The held moby pointer (boot BSS, every level): [C] aridia_gadgets.md:143 (FUN_002c0a58 reads `*0x1403e0` as the held
/// Trespasser), kerwan_enemies.md:573.
pub const HELD_MOBY: u32 = 0x1403e0;

/// Moby table start / end pointers: Veldin, Rilgar, Oltanis and Gemlik keep them 0x40 higher (each overlay's code
/// windows referencing them paired with Novalis's, as for `camera_globals`).
pub fn table(level: u32) -> (u32, u32) {
	if matches!(level, 0 | 5 | 14 | 16) { (va(0x15ff98), va(0x15ffa0)) } else { (va(0x15ff58), va(0x15ff60)) }
}

/// Game modes where the main thread keeps the last frame: the moby table, hero and camera are not mirrored. The main
/// loop's mode switch (Eudora OV:54316) draws the world (FUN_001eae20) in modes 0 (gameplay), 2 (in-engine scene:
/// FUN_002897b0 / draw FUN_001eb6f8), 5 (vendor shop) and 6 (ship: draw FUN_00282520 by substate, `frozen_view`). Mode 1
/// (movie: the FMV is drawn over the frame), 3 (pause and galactic-map menus: FUN_00269f00, drawn by the GS capture
/// overlay), 4 (FUN_001f5b48 menu, world only while 0x173154 == 0 [C], not seen in Eudora play) and 7 (FUN_001eb7e0: a
/// sprite screen, no world) stay frozen.
pub fn frozen_mode(mode: u32) -> bool {
	!matches!(mode, 0 | 2 | 5 | 6)
}

/// Mode 6 (ship) substate word 0x13e050 (boot RAM): FUN_00282520 draws the world in 0 (takeoff stream) and 8 (landing
/// stream) as a scene, 3 (flight away) and 7; 4 is the arrival warp (its own star / ribbon pass, not the level) and
/// 1, 2, 5, 6 draw nothing (level_transition.md).
pub(super) const SHIP_STATE: u32 = 0x13e050;

/// The main loop's no-present word 0x15f5b8 (Eudora OV:54400 [C]): while it is nonzero the loop skips the whole frame
/// submit (FUN_001e5f60, the vsync / flip func_0x00122298) and only syncs (FUN_002911f8(1), FUN_00291018), and the
/// mode draws FUN_001eb698 / FUN_001eb6f8 return at once, so the screen keeps the last presented picture. Seen in
/// play_0040_0041 [C, RAC_SCENE_LOG]: f1849-1870 (gameplay -> scene cut, then the 0x15f3fc fade-in 1.0 / 0.66 / 0.32),
/// f2626-2639 (scene end) and f2647-2663 (after the movie: the movie's last picture stays up).
pub(super) const NO_PRESENT: u32 = 0x15f5b8;

/// `frozen_mode` with the mode-6 substate: true when retail draws no level world this field.
pub fn frozen_view(mode: u32, ship: u32) -> bool {
	frozen_mode(mode) || (mode == 6 && !matches!(ship, 0 | 3 | 7 | 8))
}

/// Game mode of the vendor (`0x15f5a4`): the shop and the gadget cinematic that follows a purchase (`FUN_002ae128`).
pub(super) const VENDOR_MODE: u32 = 5;

/// A scene plays its actors (scene_chunk_parse, Veldin FUN_00244e58; cutscenes.md "Scene actors"): mode 2, or mode 6 in
/// the takeoff / landing streams (substates 0 and 8, which run the same chunk player).
pub(super) fn scene_view(mode: u32, ship: u32) -> bool {
	mode == 2 || (mode == 6 && matches!(ship, 0 | 8))
}
