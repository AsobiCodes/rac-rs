//! Wrench throw (modern menu: Game -> Gameplay -> Wrench throw; options file key `wrenchthrow`). Classic (default) = retail: nothing
//! runs, nothing is written. Moving = the Ratchet & Clank 3 throw: Ratchet may move once the wrench has left his hand, and the throw
//! flies where he aims while strafing, never along a backwards stick (R&C2's flaw).
//!
//! Retail [C] decomp/notes/wrench.md section 10: the throw is hero state 0x15; its motion code sets the target speed 0 and calls
//! hero_speed_slew (Novalis FUN_002321b0), so Ratchet stands until the anim ends, long after the release (anim clock 33).
//! 1. Move: a relocatable hand on hero_speed_slew (found in every level overlay and on the USA disc by its 20 code words, the two
//!    `jal` targets masked: `SPEC`). Moving, state 0x15, the wrench out (wrench moby byte +0x20 != 0), the anim clock past the
//!    release and the stick pushed or Cross held: retail's own SetState(0, 0) (the exit the throw takes at its end), then what the
//!    state transitions' epilogue does on that exit (group sound loops stopped, 0x13f4f0 / 0x13f4f4 cleared), all registers put
//!    back, and the hand declines: retail's slew runs as called. The wrench flies on and homes to the hand (retail's return path).
//! 2. Aim: modcam.rs `attack` (wrapping hand on hero_attack_lock) runs it through `aimed_call`: in state 0x15 while RMB aim is in
//!    force and unlocked, the target search runs from the aim yaw with no stick (retail would use the stick wish), so a backwards
//!    strafe still throws forward or at a target in front.
//! No RAM byte: the option lives in the options file only (the Kerwan fight RAM hash is unchanged while Classic).
use super::pcopts::PcOptions;
use ee_oracle::cpu::Cpu;
use ee_oracle::hs_reloc::{Reloc, RelocHand};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// Options file key; values `moving` / `classic`.
const KEY: &str = "wrenchthrow";

/// Moving (true) or Classic (false, retail).
static MOVING: AtomicBool = AtomicBool::new(false);

// Core addresses (below the overlay, the same on both discs) [C] decomp/notes/wrench.md section 10.
const HERO_STATE: u32 = 0x1413d4;
const HERO_GROUP: u32 = 0x1413dc;
const STATE_TIMER: u32 = 0x1413d8;
const HERO_MOBY: u32 = 0x1413d0;
const LEVEL_BYTE: u32 = 0x15ed84;
const WRENCH_MOBY: u32 = 0x1403e0;
const ANIM_T: u32 = 0x13fdf8;
const STICK_AMT: u32 = 0x1415ec;
const PAD_HELD: u32 = 0x13cae0;
const HERO_YAW: u32 = 0x13f3e8;
const ATK_STICK: u32 = 0x13fda8;
const EPI_TIMER: u32 = 0x13f4f0;
const EPI_SEQ: u32 = 0x13f4f4;
/// The throw state (Comet-Strike).
const THROW: u32 = 0x15;
/// Cross in the held-buttons word.
const CROSS: u32 = 0x40;
/// The anim clock at which Ratchet may move: the release is at 33, a short follow-through after it.
const MOVE_T: f32 = 37.0;
/// Stick amount that counts as "pushed" (retail's own move threshold in state 0 is lower; this ignores stick noise).
const MOVE_STICK: f32 = 0.2;

/// hero_speed_slew of Novalis (0x2321b0, 0x50 bytes; binary twins on all 19 levels and the USA disc): the two `jal` targets masked.
const SPEC: &str = "reloc wrench_slew entry 0x2321b0 len 0x50\nmhash 0x06a660860ca42aaa\nmask 0x2c 0x3ffffff\nmask 0x3c 0x3ffffff\n";

/// The group-change step of the transitions' epilogue (Novalis FUN_00227e70: stops the hero group sound loops), per level.
static GROUP_STOP: [u32; 19] = [
	0x206750, 0x227e70, 0x216fc8, 0x1ff8f8, 0x1f9cb0, 0x234cc8, 0x21e538, 0x22e538, 0x218ae0, 0x2263b0, 0x1f9d38, 0x2378c8, 0x228f58, 0x2121b8, 0x2196d8, 0x1ff828,
	0x2047c8, 0x203628, 0x20ee98,
];
static GROUP_STOP_USA: [u32; 19] = [
	0x206c08, 0x2283a8, 0x217500, 0x1ffe30, 0x1fa1e8, 0x235180, 0x21ea70, 0x22ea70, 0x219018, 0x2268e8, 0x1fa270, 0x237e00, 0x229490, 0x2126f0, 0x219b90, 0x1ffd60,
	0x204d00, 0x203b60, 0x20f3d0,
];

pub fn init(opts: &PcOptions) {
	MOVING.store(opts.extra_get(KEY).is_some_and(|v| v.eq_ignore_ascii_case("moving")), Ordering::Relaxed);
}

/// Moving (the R&C3 throw) is chosen.
pub fn moving() -> bool {
	MOVING.load(Ordering::Relaxed)
}

/// The modern screen set the row (`pcext/ui_bridge_more.rs`): applied at once and remembered; true = the options file changed.
pub fn set_from_ui(opts: &mut PcOptions, on: bool) -> bool {
	if MOVING.swap(on, Ordering::Relaxed) == on {
		return false;
	}
	opts.extra_set(KEY, Some(if on { "moving" } else { "classic" }.to_string()));
	eprintln!("retail: PC options: wrench throw {}", if on { "Moving" } else { "Classic" });
	true
}

fn reloc() -> &'static Reloc {
	static R: OnceLock<Reloc> = OnceLock::new();
	R.get_or_init(|| Reloc::parse(SPEC).expect("the hero_speed_slew spec parses"))
}

/// Registers the hand (before the recompiled table is built: worker/machine.rs, next to `prompts::init`). It checks the option per
/// call, so the row works without a restart.
pub fn register() {
	static HAND: RelocHand = RelocHand { name: "wrench_throw", reloc, est: ee_oracle::hs::EXACT, f: slew };
	ee_oracle::rec::register_reloc_hand(&HAND);
}

fn f32_at(c: &mut Cpu, a: u32) -> f32 {
	f32::from_bits(c.mem.rd32(a))
}

/// The wrench is out of the hand (thrown or on its way back): its moby's byte +0x20 [C].
fn wrench_out(c: &mut Cpu) -> bool {
	let w = c.mem.rd32(WRENCH_MOBY) & 0x01ff_ffff;
	w != 0 && c.mem.rd8(w + 0x20) != 0
}

/// May the throw end now? State 0x15, wrench out, the anim past the release, and the player wants to move or jump.
fn exit_wanted(c: &mut Cpu) -> bool {
	c.mem.rd32(HERO_STATE) == THROW
		&& wrench_out(c)
		&& f32_at(c, ANIM_T) >= MOVE_T
		&& (f32_at(c, STICK_AMT) > MOVE_STICK || c.mem.rd32(PAD_HELD) & CROSS != 0)
}

/// The hand at hero_speed_slew's entry: always declines (retail's slew runs with its own arguments).
fn slew(c: &mut Cpu) -> bool {
	if log_on() {
		log(c);
	}
	if moving() && exit_wanted(c) {
		end_throw(c);
		if log_on() {
			eprintln!("wrench_throw: Moving ended the throw -> state {:#x}", c.mem.rd32(HERO_STATE));
		}
	}
	false
}

/// RAC_WRENCH_LOG (dev proof): one stderr line per new anim clock while the throw state runs and for 120 lines after it.
fn log_on() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_WRENCH_LOG").is_ok_and(|v| v == "1"))
}

fn log(c: &mut Cpu) {
	use std::sync::atomic::AtomicU32;
	static LAST: AtomicU32 = AtomicU32::new(u32::MAX);
	static AFTER: AtomicU32 = AtomicU32::new(0);
	let (state, t) = (c.mem.rd32(HERO_STATE), c.mem.rd32(ANIM_T));
	if state == THROW {
		AFTER.store(120, Ordering::Relaxed);
	} else if AFTER.load(Ordering::Relaxed) == 0 {
		return;
	}
	let pos = [f32_at(c, 0x13f3d0), f32_at(c, 0x13f3d4), f32_at(c, 0x13f3d8)];
	let key = t ^ state.rotate_left(24) ^ pos[0].to_bits();
	if LAST.swap(key, Ordering::Relaxed) == key {
		return;
	}
	if state != THROW {
		AFTER.fetch_sub(1, Ordering::Relaxed);
	}
	let out = wrench_out(c);
	let w = c.mem.rd32(WRENCH_MOBY) & 0x01ff_ffff;
	let wp = if w != 0 { [f32_at(c, w + 0x10), f32_at(c, w + 0x14)] } else { [0.0; 2] };
	eprintln!(
		"wrench_throw: state {state:#x} T {:.1} pos {:.2} {:.2} {:.2} wrench {:.2} {:.2} yaw {:.3} out {out} stick {:.2} held {:#06x} moving {}",
		f32::from_bits(t),
		pos[0],
		pos[1],
		pos[2],
		wp[0],
		wp[1],
		f32_at(c, HERO_YAW),
		f32_at(c, STICK_AMT),
		c.mem.rd32(PAD_HELD) & 0xffff,
		moving()
	);
}

/// SetState(0, 0) of the running level and the transitions' epilogue for that change; every register put back afterwards.
fn end_throw(c: &mut Cpu) {
	let level = c.mem.rd8(LEVEL_BYTE) as usize;
	let (Some(set_state), Some(&stop)) = (super::strafe::set_state_of(level), (*super::disc::pick(&GROUP_STOP, &GROUP_STOP_USA)).get(level)) else { return };
	let regs = (c.r, c.f, c.hi, c.lo, c.fcr31, c.pc, c.npc);
	let hero = c.mem.rd32(HERO_MOBY) & 0x01ff_ffff;
	let seq = |c: &mut Cpu| if hero != 0 { c.mem.rd8(hero + 0x53) } else { 0 };
	let (group, timer, seq0) = (c.mem.rd32(HERO_GROUP), c.mem.rd32(STATE_TIMER), seq(c));
	c.s(4, 0);
	c.s(5, 0);
	c.call(set_state);
	if c.mem.rd32(HERO_STATE) != THROW {
		if c.mem.rd32(HERO_GROUP) != group {
			c.call(stop);
		}
		if c.mem.rd32(STATE_TIMER) != timer {
			c.mem.wr32(EPI_TIMER, 0);
		}
		if seq(c) != seq0 {
			c.mem.wr32(EPI_SEQ, 0);
		}
	}
	(c.r, c.f, c.hi, c.lo, c.fcr31, c.pc, c.npc) = regs;
}

/// modcam.rs `attack`: calls hero_attack_lock at `t`. Moving, in the throw, unlocked and with RMB aim in force (`aim`), the call
/// sees the aim as the hero yaw and no stick, so its target search and its heading start from the aim; both words are put back.
pub(crate) fn aimed_call(c: &mut Cpu, t: u32, aim: fn() -> Option<f32>) {
	let face = if moving() && c.mem.rd32(HERO_STATE) == THROW && c.mem.rd32(ATK_STICK) == 0 { aim() } else { None };
	let Some(yaw) = face else {
		c.call(t);
		return;
	};
	let saved = (c.mem.rd32(STICK_AMT), c.mem.rd32(HERO_YAW));
	if log_on() {
		eprintln!("wrench_throw: aimed search from yaw {yaw:.3} (hero yaw {:.3}, stick {:.2})", f32::from_bits(saved.1), f32::from_bits(saved.0));
	}
	c.mem.wr32(STICK_AMT, 0f32.to_bits());
	c.mem.wr32(HERO_YAW, yaw.to_bits());
	c.call(t);
	c.mem.wr32(STICK_AMT, saved.0);
	c.mem.wr32(HERO_YAW, saved.1);
}

#[cfg(test)]
#[path = "wrench_throw_tests.rs"]
mod tests;
