//! Quick Select pauses the game (GitHub #46 / #67; modern menu Game -> Menus -> "Quick Select pauses the game", options file key
//! `qspause` = on / off). Off (default) = retail: nothing runs, nothing is written.
//!
//! Retail [C] decomp/notes/ingame_other_screens.md section 2: Triangle makes the hero state function request HUD element 3, the
//! wheel (update Veldin FUN_00237780), which the HUD update runs every tick: the stick picks, the release equips and fades it out.
//! The world keeps updating all the while.
//!
//! On: while the ring is up the world stands still through retail's own update mask, the word the gameplay tick (world_update,
//! Kerwan FUN_002841f0) tests before each world step [C Kerwan OV:108911-108952]: bit 1 the hero update, bit 2 the moby update
//! loop with the moby slots, the voice queue, the ambience and the frame counter, bit 4 the particles, bit 8 the camera. The level
//! start sets it to 0xf (Kerwan OV:49043); only retail's debug menu writes other values. For a field that starts with the ring up
//! and Triangle held the word is 0, and after the field it is 0xf again. That tick still reads the pad, opens the pause menu or the
//! map, runs the HUD (the ring opens, turns, picks), the help boxes, sound and the lights (both follow the frozen hero and camera);
//! no moby (enemies, projectiles, platforms), particle, the hero, the camera or the frame counter moves. The field in which Triangle
//! is let go runs as retail's (the equip, the fade-out with the world running), and everything goes on from where it stood.
//! In a frozen field the mouse look is dropped (`worker/field_run.rs`): the camera does not turn and nothing piles up for the close.
//!
//! The word's address: base + 8 of the block world_update loads in its prologue (`lui v0; addiu v1, v0`), one copy per level
//! overlay, found on all 19 PAL overlays by its prologue words; = Novalis 0x16c460 shifted like modcam's camera globals. USA through
//! `va`, proven on the USA disc (2026-11-03): world_update's prologue base + 8 in all 19 USA overlays = `va` of this table (aligned
//! runs of disc_map.rs), e.g. Kerwan PAL 0x16c0e0 -> USA 0x16c160. It is written only while it reads 0xf and the camera word beside it (+0xc, retail's camera choice, 0 in play) reads 0, so a
//! debug state or a wrong address is never written.
use super::pcopts::PcOptions;
use crate::retail::disc::va;
use ee_oracle::mem::Mem;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Options file key; values `on` / `off`.
const KEY: &str = "qspause";
static ON: AtomicBool = AtomicBool::new(false);
/// Worker: the mask word set to 0 for the field that runs now (0 = none), put back by `post_field`.
static HELD: AtomicU32 = AtomicU32::new(0);
/// Worker: the level of that field (a level that changed meanwhile is not written).
static HELD_LV: AtomicU32 = AtomicU32::new(0);
/// The newest field ran frozen (the display's late mouse turn stays off: `camera.rs`).
static FROZEN: AtomicBool = AtomicBool::new(false);

const GAME_MODE: u32 = 0x15f5a4;
/// The mask in play (level start): hero 1, mobys + frame counter 2, particles 4, camera 8.
const PLAY: u32 = 0xf;
/// Triangle in the game's button word (`Machine::set_pad` stores it inverted).
const TRIANGLE: u16 = 0x10;

/// world_update's update mask per level 0..18 on the PAL disc (prologue base + 8; maskscan of the 19 overlay.elf files, 2026-11-02).
const MASK: [u32; 19] = [
	0x16c060, 0x16c460, 0x16c5e0, 0x16c0e0, 0x16c160, 0x16c460, 0x16c760, 0x16c060, 0x16c7e0, 0x16c160, 0x16c460, 0x16c9e0, 0x16c460, 0x16c2e0, 0x16c760,
	0x16c5e0, 0x16c3e0, 0x16c8e0, 0x16c9e0,
];

pub fn init(opts: &PcOptions) {
	ON.store(opts.extra_get(KEY).is_some_and(|v| v.eq_ignore_ascii_case("on")), Ordering::Relaxed);
}

/// The row is On.
pub fn on() -> bool {
	ON.load(Ordering::Relaxed)
}

/// The modern screen set the row (`pcext/ui_bridge_more.rs`): applied from the next field and remembered; true = the options file changed.
pub fn set_from_ui(opts: &mut PcOptions, on: bool) -> bool {
	if ON.swap(on, Ordering::Relaxed) == on {
		return false;
	}
	opts.extra_set(KEY, Some(if on { "on" } else { "off" }.to_string()));
	eprintln!("retail: PC options: Quick Select pauses the game {}", if on { "On" } else { "Off" });
	true
}

/// The newest field ran with the world stopped.
pub fn frozen() -> bool {
	FROZEN.load(Ordering::Relaxed)
}

/// What the decision reads at the start of a field.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Seen {
	on: bool,
	mode: u32,
	mask: u32,
	cam: u32,
	ring: bool,
	triangle: bool,
}

/// Pure: does this field run with the world stopped? The row On, gameplay (mode 0), the mask and the camera word as in play, the ring
/// up (its fade byte) and Triangle still held in this field's pad (so the release field runs as retail's).
fn wants(s: &Seen) -> bool {
	s.on && s.mode == 0 && s.mask == PLAY && s.cam == 0 && s.ring && s.triangle
}

/// Pure: the mask word after a frozen field: put back to the play value when it still holds our 0, else the game wrote it (kept).
fn put_back(now: u32) -> Option<u32> {
	(now == 0).then_some(PLAY)
}

/// Triangle held in the pad the field reads (`Machine::set_pad`: bytes 0 / 1 = the inverted button word, high byte first).
fn triangle(pad: Option<[u8; 18]>) -> bool {
	pad.is_some_and(|r| !(u16::from(r[0]) << 8 | u16::from(r[1])) & TRIANGLE != 0)
}

/// Before a level field: Some(mask address) when the field runs frozen (the word is 0 now).
fn start(mem: &mut Mem, level: u32, on: bool, pad: Option<[u8; 18]>) -> Option<u32> {
	let a = va(*MASK.get(level as usize)?);
	// The camera word sits in the same aligned run on both discs (+0xc there too: lane nfeat1 usa_mask.py, all 19 levels).
	let (mode, mask, cam) = (mem.rd32(va(GAME_MODE)), mem.rd32(a), mem.rd32(a + 0xc));
	let ram = &mem.ram;
	let ring = super::wheel_mouse::open_in(level, |x| ram.get((x & 0x01ff_ffff) as usize).copied().unwrap_or(0));
	let s = Seen { on, mode, mask, cam, ring, triangle: triangle(pad) };
	wants(&s).then(|| {
		mem.wr32(a, 0);
		a
	})
}

/// After the field: the mask word back (when the game did not write it meanwhile).
fn end(mem: &mut Mem, a: u32) {
	if let Some(v) = put_back(mem.rd32(a)) {
		mem.wr32(a, v);
	}
}

/// Worker, before a level field runs (the pad set): true = the field runs frozen (drop its mouse look). Logs each stop / go on.
pub fn pre_field(m: &mut ee_oracle::Machine, field: u32) -> bool {
	let (level, pad) = (m.level(), m.cpu.pad);
	let held = if on() { start(&mut m.cpu.mem, level, true, pad) } else { None };
	HELD.store(held.unwrap_or(0), Ordering::Relaxed);
	HELD_LV.store(level, Ordering::Relaxed);
	let frozen = held.is_some();
	if FROZEN.swap(frozen, Ordering::Relaxed) != frozen {
		eprintln!("retail: Quick Select pause: world {} (field {field}, level {level})", if frozen { "stopped" } else { "goes on" });
	}
	frozen
}

/// Worker, after every field: the mask word of a frozen field back to the play value (same level only).
pub fn post_field(m: &mut ee_oracle::Machine) {
	let a = HELD.swap(0, Ordering::Relaxed);
	if a != 0 && m.level() == HELD_LV.load(Ordering::Relaxed) {
		end(&mut m.cpu.mem, a);
	}
}

#[cfg(test)]
#[path = "qs_pause_tests.rs"]
mod tests;
