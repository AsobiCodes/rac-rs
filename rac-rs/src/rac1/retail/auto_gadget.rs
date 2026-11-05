//! Auto-equip gadgets (GitHub #76; modern menu Game -> Gameplay -> "Auto-equip gadgets", options file key `autogadget` = on / off).
//! Off (default) = retail: nothing runs, nothing is written (the item-tick hand declines at once, the pad filter returns the pad).
//!
//! On, the Ratchet & Clank 3 way: Circle (fire) where a gadget is needed takes the gadget out and uses it, without firing the item in
//! hand; once Ratchet stands or runs again (and is off the gadget's pad) the weapon comes back. Retail facts [C]
//! decomp/notes/auto_equip_gadgets.md:
//! - Where: the Swingshot (item 12) when retail's own Versa-Target selectors find a target (swing Aridia FUN_0022ad78, pull
//!   FUN_0022aff0; the fire dispatcher runs them only while the Swingshot is in hand); the Trespasser (26) on an unsolved lock pad
//!   (class 615, state byte 1) Ratchet stands on; the Hydrodisplacer (22) on its pump pad (class 0x155, retail's pad test
//!   FUN_002de7a8 true) in hero group 0 / 1. Only
//!   items Ratchet owns.
//! - How: retail's request word 0x141408, the word the Quick Select ring's release writes; the item tick's request handler
//!   (hero_item_request_apply, Veldin FUN_0020f9e8) swaps the item in that field exactly as for a ring pick. The return asks for the
//!   weapon of before the same way.
//! - The press: retail's fire dispatcher runs before that handler in the tick, so the field's Circle is held back from the game
//!   until the gadget is in hand (slot 0 id = the gadget, state 2); then Circle reaches the game as a fresh press while the player
//!   still holds it (swing, pull, pump, lock), or as one press when he tapped it meanwhile.
//! - The Versa-Target test: a relocatable hand on hero_item_slots_tick (Veldin FUN_00210290, the same code on all 19 levels of both
//!   discs) calls retail's two selectors in the tick while the row is On, the Swingshot is owned and not in hand and the hero may
//!   fire; then the machine is put back as it was (every register, the clock and event state, scratchpad, VU0 data, hardware
//!   page, the selectors' caches, the stack and the traces of their line-of-sight test; `Keep`; RAC_AG_LOG=1 compares all of RAM)
//!   and the hand declines (retail's tick runs as called). The answer decides the next field's Circle. Addresses: lane nfeat1 scratchpad ag_scan.py (both ISOs, va(PAL) == USA on every level);
//!   the code at an address is checked against its spec before every call. No mod hook runs inside the probe (engine
//!   omni/bridge.rs `quiet`); a probe that goes wrong (`bad`: a panic, no return, a fault or stop, over 100000 instructions) is
//!   turned off for that level. `reset` (a new machine, the front end, the row Off) forgets every decision.
use super::pcopts::PcOptions;
use crate::retail::disc::{pick, va};
use ee_oracle::cpu::Cpu;
use ee_oracle::hs_reloc::{Reloc, RelocHand};
use ee_oracle::mem::Mem;
use std::cell::RefCell;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

/// Options file key; values `on` / `off`.
const KEY: &str = "autogadget";
static ON: AtomicBool = AtomicBool::new(false);
/// The newest probe (item-tick hand) found a Versa-Target the Swingshot would take.
static TARGET: AtomicBool = AtomicBool::new(false);

const GAME_MODE: u32 = 0x15f5a4;
// Core words (below the overlay, the same on both discs) [C] decomp/notes/weapons_framework.md section 1, auto_equip_gadgets.md.
const LEVEL_BYTE: u32 = 0x15ed84;
const OWNED: u32 = 0x13d4c0;
const HELD: u32 = 0x140408;
const ITEM_STATE: u32 = 0x140404;
const ITEM_MOBY: u32 = 0x1403e0;
const REQUEST: u32 = 0x141408;
const LAST: u32 = 0x141660;
const HERO_STATE: u32 = 0x1413d4;
const HERO_GROUP: u32 = 0x1413dc;
/// Bytes that stop item use: the hero update off (0x1413f4), items stowed (0x1413fc), the wrench lock (0x1413f7).
const HERO_OFF: u32 = 0x1413f4;
const STOWED: u32 = 0x1413fc;
const WRENCH_LOCK: u32 = 0x1413f7;
/// The moby the hero stands on, and the u16 that is non-zero while he is not on the ground.
const SUPPORT: u32 = 0x13f64c;
const AIR: u32 = 0x13f65e;
/// The selectors' results: swing ok (u16), pull ok (word), and the u16 that blocks both in the dispatcher.
const SWING_OK: u32 = 0x13fcee;
const PULL_OK: u32 = 0x13fcb8;
const NO_SWING: u32 = 0x13f536;
/// The selectors' cache words 0x13fcb4..0x13fcf0 (put back after a probe).
const CACHE: u32 = 0x13fcb4;
const CACHE_WORDS: u32 = 15;

const SWINGSHOT: u32 = 12;
const HYDRO: u32 = 22;
const TRESPASSER: u32 = 26;
const WRENCH: u32 = 8;
const LOCK_PAD: u16 = 615;
const HYDRO_PAD: u16 = 0x155;
/// The byte that picks the pumped moby's flag bit in the pad test (Kerwan FUN_002de7a8: 0 -> bit 1, else bit 0).
const PUMP_DIR: u32 = 0x141400;
/// Circle in the game's button word (`Machine::set_pad` input, active high).
const CIRCLE: u16 = 0x20;
/// Item slot state "ready" (the fire dispatcher's gate).
const READY: u32 = 2;
/// A gadget not in hand after this many fields is given up (and our request taken back if it still waits).
const WAIT_MAX: u32 = 150;
/// Fields Ratchet stands or runs (Circle up, off the pad) before the weapon comes back.
const BACK_FIELDS: u32 = 15;
/// Hero states that count as standing or running: idle 0, run 2, walk 0x73 (hero_movement.md).
const REST_STATES: [u32; 3] = [0, 2, 0x73];

/// The selectors per level 0..18 (0 = none: Novalis has no Versa-Target code), PAL and USA (ag_scan.py).
static SWING: [u32; 19] = [
	0x221ac0, 0, 0x22ad78, 0x215c98, 0x215020, 0x24c5b8, 0x234ce8, 0x2498a8, 0x230188, 0x241720, 0x2150a8, 0x24d1a0, 0x23f720, 0x22d528, 0x22f628,
	0x216288, 0x21d9e8, 0x21db78, 0x227420,
];
static SWING_USA: [u32; 19] = [
	0x221ee0, 0, 0x22b210, 0x216130, 0x2154c0, 0x24c9d0, 0x235188, 0x249d48, 0x230620, 0x241bc0, 0x215548, 0x24d638, 0x23fbb8, 0x22d9c8, 0x22fa40,
	0x216720, 0x21de80, 0x21e018, 0x2278b8,
];
static PULL: [u32; 19] = [
	0x221d38, 0, 0x22aff0, 0x215f10, 0x215298, 0x24c830, 0x234f60, 0x249b20, 0x230400, 0x241998, 0x215320, 0x24d418, 0x23f998, 0x22d7a0, 0x22f8a0,
	0x216500, 0x21dc60, 0x21ddf0, 0x227698,
];
static PULL_USA: [u32; 19] = [
	0x222158, 0, 0x22b488, 0x2163a8, 0x215738, 0x24cc48, 0x235400, 0x249fc0, 0x230898, 0x241e38, 0x2157c0, 0x24d8b0, 0x23fe30, 0x22dc40, 0x22fcb8,
	0x216998, 0x21e0f8, 0x21e290, 0x227b30,
];

/// Code specs (ag_scan.py: the bits that differ between the 37 copies masked, `jal` targets always).
const SWING_SPEC: &str = "reloc ag_swing entry 0x22ad78 len 0x278\nmhash 0xd47d5124fe0a70f1\nmask 0x3c 0xffff\nmask 0xcc 0x3ffffff\nmask 0x178 0x3ffffff\nmask 0x1b0 0x3ffffff\nmask 0x22c 0x3ffffff\n";
const PULL_SPEC: &str = "reloc ag_pull entry 0x22aff0 len 0x2a0\nmhash 0x114454c0fe39db6c\nmask 0x44 0xffff\nmask 0xbc 0x3ffffff\nmask 0x154 0x3ffffff\nmask 0x170 0x3ffffff\nmask 0x1b8 0x3ffffff\nmask 0x24c 0x3ffffff\n";
const ITEMS_SPEC: &str = "reloc ag_items entry 0x210290 len 0x1e0\nmhash 0xf7c11ac3da8be802\nmask 0x88 0x3ffffff\nmask 0xc0 0x3ffffff\nmask 0xfc 0x3ffffff\nmask 0x14c 0x3ffffff\nmask 0x194 0x3ffffff\n";

fn spec(k: usize) -> &'static Reloc {
	static R: OnceLock<[Reloc; 3]> = OnceLock::new();
	&R.get_or_init(|| [SWING_SPEC, PULL_SPEC, ITEMS_SPEC].map(|s| Reloc::parse(s).expect("auto_gadget spec parses")))[k]
}

fn items_spec() -> &'static Reloc {
	spec(2)
}

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
	eprintln!("retail: PC options: Auto-equip gadgets {}", if on { "On" } else { "Off" });
	true
}

/// Registers the item-tick hand (worker/machine.rs, before the recompiled table is built). It checks the row per call.
pub fn register() {
	static HAND: RelocHand = RelocHand { name: "auto_gadget", reloc: items_spec, est: ee_oracle::hs::EXACT, f: items_tick };
	ee_oracle::rec::register_reloc_hand(&HAND);
}

/// The pad the hero stands on, as far as a gadget goes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Pad {
	#[default]
	None,
	/// A Trespasser lock pad waiting for the Trespasser (state 1).
	Lock,
	/// A Hydrodisplacer pump pad.
	Hydro,
}

/// What a field-start decision reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Seen {
	mode: u32,
	/// Slot 0: the item in hand, it is ready (state 2, a moby), the request word, the last chosen weapon.
	held: u32,
	ready: bool,
	request: u32,
	last: u32,
	/// Items may be used (hero update on, not stowed, no wrench lock).
	play: bool,
	hero: u32,
	group: u32,
	air: bool,
	pad: Pad,
	/// The newest probe found a Versa-Target.
	target: bool,
	/// Owned items, bit = item id.
	owned: u64,
}

impl Seen {
	fn owns(&self, id: u32) -> bool {
		id < 64 && self.owned >> id & 1 != 0
	}
}

/// The decision state, kept between fields (worker thread).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct St {
	level: u32,
	/// Circle held in the player's pad of the last field.
	circle: bool,
	/// The gadget asked for and not in hand yet (0 = none), fields waited, and whether Circle went up meanwhile.
	pending: u32,
	wait: u32,
	tap: bool,
	/// The gadget we put in hand (0 = none), the weapon to bring back, fields the return condition held.
	auto: u32,
	back: u32,
	rest: u32,
}

/// What the field does: Circle forced (Some) or as the pad has it (None), and a value for the request word.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Out {
	circle: Option<bool>,
	request: Option<u32>,
}

/// Pure: the gadget this place wants (pads first: standing on one is explicit).
fn wanted(s: &Seen) -> Option<u32> {
	match s.pad {
		Pad::Lock => Some(TRESPASSER),
		Pad::Hydro if s.group <= 1 => Some(HYDRO),
		_ if s.target => Some(SWINGSHOT),
		_ => None,
	}
}

/// Pure: one field of the decision (`circle` = the player holds Circle in this field's pad).
fn step(st: &mut St, s: &Seen, circle: bool) -> Out {
	let edge = circle && !st.circle;
	st.circle = circle;
	let usable = s.mode == 0 && s.play;
	if st.pending != 0 {
		st.wait += 1;
		if s.held == st.pending && s.ready {
			(st.auto, st.pending, st.rest) = (st.pending, 0, 0);
			let tap = std::mem::take(&mut st.tap);
			return Out { circle: Some(circle || tap), request: None };
		}
		if !usable || st.wait > WAIT_MAX {
			let mine = s.request == st.pending;
			(st.pending, st.tap, st.back) = (0, false, 0);
			return Out { circle: None, request: mine.then_some(0) };
		}
		st.tap |= !circle;
		return Out { circle: Some(false), request: None };
	}
	if edge && usable && s.request == 0 {
		if let Some(id) = wanted(s).filter(|&id| s.held != id && s.owns(id)) {
			st.back = if st.auto != 0 && s.held == st.auto {
				st.back
			} else if s.held == WRENCH {
				s.last
			} else {
				s.held
			};
			(st.pending, st.wait, st.tap, st.auto, st.rest) = (id, 0, false, 0, 0);
			return Out { circle: Some(false), request: Some(id) };
		}
	}
	if st.auto != 0 {
		if s.held != st.auto || s.request != 0 {
			(st.auto, st.back, st.rest) = (0, 0, 0);
		} else if !circle && usable && !s.air && REST_STATES.contains(&s.hero) && wanted(s) != Some(st.auto) {
			st.rest += 1;
			if st.rest >= BACK_FIELDS {
				let back = st.back;
				(st.auto, st.back, st.rest) = (0, 0, 0);
				if back != 0 && back != s.held && s.owns(back) {
					return Out { circle: None, request: Some(back) };
				}
			}
		} else {
			st.rest = 0;
		}
	}
	Out::default()
}

/// [C] Kerwan FUN_002de7a8, the Hydrodisplacer's pad test (decomp/notes/auto_equip_gadgets.md): the pad's pvar word 0 indexes the
/// moby it pumps (moby table + idx * 0x100); that moby's +0xbc bit 1 (byte PUMP_DIR == 0) or bit 0 (otherwise) lets the pump run.
/// Only a moby slot of `span` (ram_globals.rs `moby_span`) is read.
fn pump_ok(mem: &mut Mem, pad: u32, span: (u32, u32)) -> bool {
	let pvar = mem.rd32(pad + 0x78) & 0x01ff_fffc;
	if pvar == 0 {
		return false;
	}
	let moby = span.0.wrapping_add(mem.rd32(pvar).wrapping_mul(0x100)) & 0x01ff_ffff;
	let bit = if mem.rd8(PUMP_DIR) == 0 { 2 } else { 1 };
	super::ram_globals::is_moby(span, moby) && mem.rd8(moby + 0xbc) & bit != 0
}

/// The decision's view of RAM at the field start. The support word is read as a moby only when it is a slot of the moby table.
fn seen(mem: &mut Mem) -> Seen {
	let level = mem.rd8(LEVEL_BYTE) as u32;
	let span = super::ram_globals::moby_span(mem, level);
	let sup = mem.rd32(SUPPORT) & 0x01ff_ffff;
	let air = mem.rd16(AIR) != 0;
	let pad = match span {
		Some(span) if !air && super::ram_globals::is_moby(span, sup) => match mem.rd16(sup + 0xa6) {
			LOCK_PAD if mem.rd8(sup + 0x20) == 1 => Pad::Lock,
			HYDRO_PAD if pump_ok(mem, sup, span) => Pad::Hydro,
			_ => Pad::None,
		},
		_ => Pad::None,
	};
	let owned = (0..37u32).filter(|&id| mem.rd8(OWNED + id) != 0).fold(0u64, |o, id| o | 1 << id);
	Seen {
		mode: mem.rd32(va(GAME_MODE)),
		held: mem.rd32(HELD),
		ready: mem.rd32(ITEM_STATE) == READY && mem.rd32(ITEM_MOBY) != 0,
		request: mem.rd32(REQUEST),
		last: mem.rd32(LAST),
		play: mem.rd8(HERO_OFF) == 0 && mem.rd8(STOWED) == 0 && mem.rd8(WRENCH_LOCK) == 0,
		hero: mem.rd32(HERO_STATE),
		group: mem.rd32(HERO_GROUP),
		air,
		pad,
		target: TARGET.load(Ordering::Relaxed),
		owned,
	}
}

/// The decision state of no level: the first field of a level only starts it.
const FRESH: St = St { level: u32::MAX, circle: false, pending: 0, wait: 0, tap: false, auto: 0, back: 0, rest: 0 };
static ST: Mutex<St> = Mutex::new(FRESH);
/// The decision ran since the last `reset` (there is something to forget).
static LIVE: AtomicBool = AtomicBool::new(false);
/// The level whose Versa-Target probe went wrong (`bad`; u32::MAX = none): no probe there until the level changes or `reset`.
static PROBE_OFF: AtomicU32 = AtomicU32::new(u32::MAX);

/// Forgets every decision and probe answer: a new machine (worker/machine.rs `register_hooks`), the front end (`filter`: after a
/// return to the title a load may come back to the same level, where an old request or return must not go on), the row Off.
/// A death respawn is NOT a reset: the world rebuild also zeroes the level frame counter, but the gadget stays in hand and its
/// return is still due [C] (Eudora FUN_00233e98 OV:53214 `DAT_0015f5ac = 0`, run by the rebuild FUN_00236668 OV:53727 on a load and
/// on a respawn; death_respawn.md: respawn = the Veldin twin FUN_00243cf8(0, 1)).
pub fn reset() {
	*ST.lock().unwrap_or_else(|e| e.into_inner()) = FRESH;
	TARGET.store(false, Ordering::Relaxed);
	PROBE_OFF.store(u32::MAX, Ordering::Relaxed);
	LIVE.store(false, Ordering::Relaxed);
}

/// Worker, before `set_pad` of a level field: the pad the game gets (Circle held back / handed over) and the request word written.
/// Off: one load (and nothing written) unless the decision ran since the last reset.
pub fn filter(m: &mut ee_oracle::Machine, front: bool, field: u32, buttons: u16) -> u16 {
	if front || !on() {
		if LIVE.load(Ordering::Relaxed) {
			reset();
		}
		return buttons;
	}
	LIVE.store(true, Ordering::Relaxed);
	let level = m.level();
	decide(&mut m.cpu.mem, level, field, buttons)
}

/// One level field of the decision on RAM as it is (`filter` without the machine; tests).
fn decide(mem: &mut Mem, level: u32, field: u32, buttons: u16) -> u16 {
	let Ok(mut st) = ST.lock() else { return buttons };
	if st.level != level {
		*st = St { level, circle: buttons & CIRCLE != 0, ..FRESH };
		TARGET.store(false, Ordering::Relaxed);
		PROBE_OFF.store(u32::MAX, Ordering::Relaxed);
		return buttons;
	}
	let s = seen(mem);
	let was = *st;
	let out = step(&mut st, &s, buttons & CIRCLE != 0);
	if ag_log() && (*st != was || out != Out::default()) {
		eprintln!("AUTO_GADGET step field {field} {s:?} {was:?} -> {:?} {out:?}", *st);
	}
	if let Some(id) = out.request {
		mem.wr32(REQUEST, id);
		eprintln!("retail: Auto-equip gadgets: request item {id} (field {field}, level {level}, held {})", s.held);
	}
	match out.circle {
		Some(true) => buttons | CIRCLE,
		Some(false) => buttons & !CIRCLE,
		None => buttons,
	}
}

/// May the Swingshot fire now if it were in hand (the dispatcher's case 0xc group gate and the item-use bytes)?
fn may_probe(c: &mut Cpu) -> bool {
	let (group, state) = (c.mem.rd32(HERO_GROUP), c.mem.rd32(HERO_STATE));
	c.mem.rd32(va(GAME_MODE)) == 0
		&& c.mem.rd8(OWNED + SWINGSHOT) != 0
		&& c.mem.rd32(HELD) != SWINGSHOT
		&& c.mem.rd8(HERO_OFF) == 0
		&& c.mem.rd8(STOWED) == 0
		&& (matches!(group, 0 | 1 | 2 | 4 | 5 | 0xf) || state == 0x1e)
}

/// Status.IE and Status.EIE (oracle cpu.rs `irq_taken`): cleared for the probe, so no interrupt is taken inside it.
const IRQ_ENABLE: u32 = 0x1_0001;
/// Stack bytes below the stack pointer put back after the probe (the selectors' and seg_collide's frames; the RAM check
/// `RAC_AG_LOG=1` lists any byte the probe leaves changed).
const STACK_BELOW: u32 = 0x1000;

/// Retail instructions one probe may spend (both selectors; measured at most 10821, lane nfeat1 HANDOFF part 8): more = a bad probe.
const PROBE_INSNS: u64 = 100_000;
/// `Cpu::call`'s return address (oracle rec/mod.rs `CALL_RET`, private there): the PC once the called function returned.
const CALL_RET: u32 = 0x10;

/// The machine as the probe found it: every CPU register, the clock and event state, the scratchpad, VU0 data memory and the
/// hardware page, the selectors' cache words, the stack below the stack pointer and seg_collide's side effects (the selectors'
/// line of sight; modcam/stamps.rs `SegSide`). `take` also holds the clock: nothing falls due and no interrupt is taken inside
/// the probe, whose instructions are not counted (as modcam/water_path.rs `Clock`). One record (`KEEP`) is refilled probe after
/// probe: its buffers are kept.
#[derive(Default)]
struct Keep {
	r: [u128; 32],
	hilo: (u128, u128, u32),
	pc: (u32, u32),
	f: ([u32; 32], u32, u32),
	vu: ([[u32; 4]; 32], [u32; 4], [u32; 32]),
	div_csr: u32,
	cp0: [u32; 32],
	clock: (u64, u64, u64, u64, u64, bool),
	spr: Vec<u8>,
	vu0d: Vec<u8>,
	hw: Vec<u8>,
	cache: Vec<u8>,
	stack: (usize, Vec<u8>),
	seg: super::modcam::stamps::SegSide,
	seg_ok: bool,
}

thread_local! {
	/// The probe's record (the hand runs on the game thread).
	static KEEP: RefCell<Keep> = RefCell::new(Keep::default());
}

/// `dst` = `src` (buffer kept).
fn refill(dst: &mut Vec<u8>, src: &[u8]) {
	dst.clear();
	dst.extend_from_slice(src);
}

impl Keep {
	fn take(&mut self, c: &mut Cpu, level: u32) {
		let at = |a: u32| (a & 0x01ff_ffff) as usize;
		let (cache, sp) = (at(CACHE), at(c.r[29] as u32));
		let low = sp.saturating_sub(STACK_BELOW as usize);
		self.seg_ok = self.seg.save(c, level);
		(self.r, self.hilo, self.pc, self.f) = (c.r, (c.hi, c.lo, c.sa), (c.pc, c.npc), (c.f, c.facc, c.fcr31));
		(self.vu, self.div_csr, self.cp0) = ((c.vf, c.vacc, c.vi), c.div_csr, c.cp0);
		let m = &mut c.mem;
		self.clock = (c.icount, m.now, m.limit, m.due, m.tick, m.irq_dirty);
		refill(&mut self.spr, &m.spr);
		refill(&mut self.vu0d, &m.vu0d);
		refill(&mut self.hw, &m.hw);
		refill(&mut self.cache, &m.ram[cache..cache + 4 * CACHE_WORDS as usize]);
		self.stack.0 = low;
		refill(&mut self.stack.1, &m.ram[low..sp]);
		(m.tick, m.due, m.limit, m.irq_dirty) = (u64::MAX, u64::MAX, u64::MAX, false);
		c.cp0[12] &= !IRQ_ENABLE;
	}

	fn put_back(&self, c: &mut Cpu) {
		if self.seg_ok {
			self.seg.put_back(c);
		}
		let cache = (CACHE & 0x01ff_ffff) as usize;
		let m = &mut c.mem;
		m.ram[cache..cache + self.cache.len()].copy_from_slice(&self.cache);
		m.ram[self.stack.0..self.stack.0 + self.stack.1.len()].copy_from_slice(&self.stack.1);
		m.spr.copy_from_slice(&self.spr);
		m.vu0d.copy_from_slice(&self.vu0d);
		m.hw.copy_from_slice(&self.hw);
		(c.icount, m.now, m.limit, m.due, m.tick, m.irq_dirty) = self.clock;
		c.r = self.r;
		(c.hi, c.lo, c.sa) = self.hilo;
		(c.pc, c.npc) = self.pc;
		(c.f, c.facc, c.fcr31) = self.f;
		(c.vf, c.vacc, c.vi) = self.vu;
		(c.div_csr, c.cp0) = (self.div_csr, self.cp0);
	}
}

/// [PC] Dev aid: RAC_AG_LOG=1 compares all of RAM, the scratchpad and the hardware page before and after each probe (32 MB copy
/// per probe) and prints one AUTO_GADGET line per probe: the answer, its time (ns, without the compare) and the bytes the probe
/// left changed (must be 0); `filter` adds an AUTO_GADGET step line for each field whose decision state or output changes.
fn ag_log() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var_os("RAC_AG_LOG").is_some())
}

/// Pure: what went wrong in a probe, None = nothing. `returned` = both selectors came back to the call's return address with the
/// stack pointer as before (Some(false): one did not; None: a panic); `spent` = retail instructions of both; `stop` / `fault` = the
/// CPU's stop or a memory fault appeared during the probe.
fn bad(returned: Option<bool>, spent: u64, stop: bool, fault: bool) -> Option<&'static str> {
	match returned {
		None => Some("a panic in the selectors"),
		_ if fault => Some("a memory fault"),
		_ if stop => Some("the CPU stopped"),
		Some(false) => Some("a selector did not return"),
		_ if spent > PROBE_INSNS => Some("over the instruction budget"),
		_ => None,
	}
}

/// Retail's two Versa-Target selectors of this level (code checked against the specs), with the whole machine put back. Not on a
/// level whose probe went wrong (`PROBE_OFF`), nor while the CPU has a stop or a memory fault pending.
fn probe(c: &mut Cpu) -> bool {
	let level = c.mem.rd8(LEVEL_BYTE) as u32;
	if PROBE_OFF.load(Ordering::Relaxed) == level || c.stop.is_some() || c.mem.fault.is_some() {
		return false;
	}
	let (Some(&sw), Some(&pl)) = (pick(&SWING, &SWING_USA).get(level as usize), pick(&PULL, &PULL_USA).get(level as usize)) else { return false };
	if sw == 0 || !spec(0).matches(&c.mem, sw) || !spec(1).matches(&c.mem, pl) {
		return false;
	}
	KEEP.with(|k| k.try_borrow_mut().is_ok_and(|mut keep| probe_with(c, level, [sw, pl], &mut keep)))
}

/// `probe` once its selectors are known: the two calls between `Keep::take` and `put_back`, inside a quiet scope (no mod chain runs
/// for these calls the game never made: engine omni/bridge.rs `quiet`). A bad probe (`bad`) turns the probe off for the level, with
/// one line; a panic goes on unwinding once the machine is put back.
fn probe_with(c: &mut Cpu, level: u32, sel: [u32; 2], keep: &mut Keep) -> bool {
	let before = ag_log().then(|| (c.mem.ram.to_vec(), c.mem.spr.clone(), c.mem.hw.clone(), c.r, c.icount));
	let t0 = before.is_some().then(std::time::Instant::now);
	keep.take(c, level);
	// [C] Aridia FUN_0022aff0 (overlay.elf.c:31550) writes the pull answer only on some paths: no older answer may count.
	c.mem.wr32(PULL_OK, 0);
	let (start, sp) = (c.icount, c.r[29] as u32);
	let quiet = crate::omni::bridge::quiet();
	let run = std::panic::catch_unwind(AssertUnwindSafe(|| {
		sel.into_iter().all(|f| {
			c.call(f);
			c.pc == CALL_RET && c.r[29] as u32 == sp
		})
	}));
	drop(quiet);
	let spent = c.icount.wrapping_sub(start);
	let (swing, pull, no) = (c.mem.rd16(SWING_OK), c.mem.rd32(PULL_OK), c.mem.rd16(NO_SWING));
	let why = bad(run.as_ref().ok().copied(), spent, c.stop.is_some(), c.mem.fault.is_some());
	keep.put_back(c);
	// Both were None when the probe began (`probe`).
	(c.stop, c.mem.fault) = (None, None);
	let hit = why.is_none() && no == 0 && (swing != 0 || pull != 0);
	let ns = t0.map_or(0, |t| t.elapsed().as_nanos());
	if let Some((ram, spr, hw, r, ic)) = before {
		let (n, ranges) = super::modcam::stamps::diff(&ram, &c.mem.ram);
		let other = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|p| p.0 != p.1).count();
		let rs: Vec<String> = ranges.iter().map(|&(s, l)| format!("{s:#x}+{l}")).collect();
		eprintln!(
			"AUTO_GADGET probe level {level} hit {} swing {swing} pull {pull} insns {spent} ns {ns} ram_diff {n} ranges [{}] spr_diff {} hw_diff {} regs_same {} icount_same {}",
			hit as u8,
			rs.join(","),
			other(&spr, &c.mem.spr),
			other(&hw, &c.mem.hw),
			r == c.r,
			ic == c.icount
		);
	}
	if let Some(why) = why {
		PROBE_OFF.store(level, Ordering::Relaxed);
		eprintln!("retail: Auto-equip gadgets: Versa-Target probe off on level {level} ({why})");
	}
	if let Err(p) = run {
		std::panic::resume_unwind(p);
	}
	hit
}

/// The hand at hero_item_slots_tick's entry: the probe while On, then it declines (retail's tick runs as called).
fn items_tick(c: &mut Cpu) -> bool {
	if on() {
		let hit = may_probe(c) && probe(c);
		TARGET.store(hit, Ordering::Relaxed);
	}
	false
}

#[cfg(test)]
#[path = "auto_gadget_tests.rs"]
mod tests;
