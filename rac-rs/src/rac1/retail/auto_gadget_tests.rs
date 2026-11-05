//! Tests of auto_gadget.rs: the specs and tables, the pure field decision `step`, the machine put back by `Keep`, Off, and the probe
//! on real savestates (all of RAM, the scratchpad, the hardware page and every register the same after it).
use super::*;
use ee_oracle::mem::Mem;

/// Every item owned.
const ALL: u64 = u64::MAX;
const BLASTER: u32 = 10;

/// In play, `held` ready in hand, nothing asked for, on the ground, idle, owning `owned`.
fn seen(held: u32, owned: u64) -> Seen {
	Seen { mode: 0, held, ready: true, request: 0, last: held, play: true, hero: 0, group: 0, air: false, pad: Pad::None, target: false, owned }
}

fn at_target(held: u32) -> Seen {
	Seen { target: true, ..seen(held, ALL) }
}

/// One field with Circle `c`, the decision state `st`.
fn field(st: &mut St, s: &Seen, c: bool) -> Out {
	step(st, s, c)
}

#[test]
fn specs_parse_with_their_lengths() {
	assert_eq!([spec(0).len, spec(1).len, spec(2).len], [0x278, 0x2a0, 0x1e0]);
	assert_eq!([spec(0).entry, spec(1).entry, spec(2).entry], [0x22ad78, 0x22aff0, 0x210290]);
}

#[test]
fn selector_tables_are_empty_only_on_novalis() {
	for t in [&SWING, &SWING_USA, &PULL, &PULL_USA] {
		for (l, &a) in t.iter().enumerate() {
			assert_eq!(a == 0, l == 1, "level {l}: {a:#x}");
		}
	}
	// The Aridia entries are the spec entries (the specs were cut there).
	assert_eq!((SWING[2], PULL[2]), (spec(0).entry, spec(1).entry));
}

#[test]
fn a_press_at_a_target_asks_for_the_swingshot_and_holds_circle_back() {
	let mut st = St::default();
	assert_eq!(field(&mut st, &at_target(BLASTER), false), Out::default());
	let o = field(&mut st, &at_target(BLASTER), true);
	assert_eq!(o, Out { circle: Some(false), request: Some(SWINGSHOT) });
	assert_eq!((st.pending, st.back), (SWINGSHOT, BLASTER));
	// The swap runs: still the Blaster, then the Swingshot not ready yet: Circle stays held back, nothing asked again.
	let mut s = Seen { request: SWINGSHOT, ..at_target(BLASTER) };
	assert_eq!(field(&mut st, &s, true), Out { circle: Some(false), request: None });
	s = Seen { held: SWINGSHOT, ready: false, request: 0, ..s };
	assert_eq!(field(&mut st, &s, true), Out { circle: Some(false), request: None });
	// Ready while the player still holds Circle: it reaches the game (a fresh press after the held-back fields).
	s.ready = true;
	assert_eq!(field(&mut st, &s, true), Out { circle: Some(true), request: None });
	assert_eq!((st.pending, st.auto), (0, SWINGSHOT));
	// From then on the pad goes through as it is.
	assert_eq!(field(&mut st, &s, true), Out::default());
}

#[test]
fn a_tap_during_the_swap_becomes_one_press() {
	let mut st = St::default();
	field(&mut st, &at_target(BLASTER), false);
	assert_eq!(field(&mut st, &at_target(BLASTER), true).request, Some(SWINGSHOT));
	let swap = Seen { held: SWINGSHOT, ready: false, ..at_target(BLASTER) };
	field(&mut st, &swap, false); // released meanwhile
	assert_eq!(field(&mut st, &swap, false).circle, Some(false));
	let ready = Seen { ready: true, ..swap };
	assert_eq!(field(&mut st, &ready, false).circle, Some(true), "one press for the tap");
	assert_eq!(field(&mut st, &ready, false), Out::default(), "only one");
}

#[test]
fn nothing_happens_without_a_reason() {
	let cases = [
		("no target", seen(BLASTER, ALL)),
		("swingshot not owned", Seen { owned: ALL & !(1 << SWINGSHOT), ..at_target(BLASTER) }),
		("not in play", Seen { mode: 3, ..at_target(BLASTER) }),
		("items stowed / no use", Seen { play: false, ..at_target(BLASTER) }),
		("a request already waits", Seen { request: 5, ..at_target(BLASTER) }),
		("already in hand", at_target(SWINGSHOT)),
		("hydro pad outside group 0 / 1", Seen { pad: Pad::Hydro, group: 2, ..seen(BLASTER, ALL) }),
	];
	for (what, s) in cases {
		let mut st = St::default();
		assert_eq!(field(&mut st, &s, false), Out::default(), "{what}");
		assert_eq!(field(&mut st, &s, true), Out::default(), "{what}");
		assert_eq!((st.pending, st.auto), (0, 0), "{what}");
	}
	// Circle already held when the target shows up: no edge, no request.
	let mut st = St::default();
	field(&mut st, &seen(BLASTER, ALL), true);
	assert_eq!(field(&mut st, &at_target(BLASTER), true), Out::default());
}

#[test]
fn pads_pick_their_gadget_first() {
	let lock = Seen { pad: Pad::Lock, target: true, ..seen(BLASTER, ALL) };
	assert_eq!(wanted(&lock), Some(TRESPASSER));
	assert_eq!(wanted(&Seen { pad: Pad::Hydro, group: 1, ..seen(BLASTER, ALL) }), Some(HYDRO));
	assert_eq!(wanted(&Seen { pad: Pad::Hydro, group: 2, ..seen(BLASTER, ALL) }), None);
	let mut st = St::default();
	field(&mut st, &lock, false);
	assert_eq!(field(&mut st, &lock, true).request, Some(TRESPASSER));
	let mut st = St::default();
	let hydro = Seen { pad: Pad::Hydro, ..seen(BLASTER, ALL) };
	field(&mut st, &hydro, false);
	assert_eq!(field(&mut st, &hydro, true).request, Some(HYDRO));
	// Not owned: nothing.
	let mut st = St::default();
	let no = Seen { owned: ALL & !(1 << HYDRO), ..hydro };
	field(&mut st, &no, false);
	assert_eq!(field(&mut st, &no, true), Out::default());
}

/// The pump pad counts only when retail's pad test (Kerwan FUN_002de7a8) would let the pump run: the pumped moby's flag bit 1
/// (direction byte 0) or bit 0 (otherwise).
#[test]
fn a_hydro_pad_counts_only_when_retails_pad_test_lets_the_pump_run() {
	let mut m = Mem::new();
	let (pvar, base) = (0x30_1000u32, 0x40_0000u32);
	let (pad, table) = (base + 5 * 0x100, crate::retail::ram_globals::table(7));
	m.wr8(LEVEL_BYTE, 7);
	m.wr32(table.0, base);
	m.wr32(table.1, base + 9 * 0x100);
	m.wr32(SUPPORT, pad);
	m.wr16(pad + 0xa6, HYDRO_PAD);
	m.wr32(pad + 0x78, pvar);
	m.wr32(pvar, 3);
	let flags = base + 3 * 0x100 + 0xbc;
	for (dir, f, want) in [(0, 0, Pad::None), (0, 2, Pad::Hydro), (0, 1, Pad::None), (1, 1, Pad::Hydro), (1, 2, Pad::None)] {
		m.wr8(PUMP_DIR, dir);
		m.wr8(flags, f);
		assert_eq!(super::seen(&mut m).pad, want, "direction {dir} flags {f}");
	}
	m.wr8(PUMP_DIR, 0);
	m.wr8(flags, 2);
	assert_eq!(super::seen(&mut m).pad, Pad::Hydro);
	// Only slots of the moby table are read: a pumped index past the last slot, a support word off a slot start or past the table,
	// a table whose words are no span.
	m.wr32(pvar, 10);
	m.wr8(base + 10 * 0x100 + 0xbc, 2);
	assert_eq!(super::seen(&mut m).pad, Pad::None, "pumped index past the last slot");
	m.wr32(pvar, 3);
	for sup in [pad + 4, base + 10 * 0x100] {
		m.wr16(sup + 0xa6, HYDRO_PAD);
		m.wr32(sup + 0x78, pvar);
	}
	for sup in [pad + 4, base + 10 * 0x100, 0x01ff_fff0] {
		m.wr32(SUPPORT, sup);
		assert_eq!(super::seen(&mut m).pad, Pad::None, "support {sup:#x}");
	}
	m.wr32(SUPPORT, pad);
	assert_eq!(super::seen(&mut m).pad, Pad::Hydro);
	m.wr32(table.1, base - 0x100);
	assert_eq!(super::seen(&mut m).pad, Pad::None, "end below start");
	m.wr32(table.1, base + 9 * 0x100);
	m.wr32(pad + 0x78, 0);
	assert_eq!(super::seen(&mut m).pad, Pad::None, "no pvar");
}

/// Puts the Swingshot in hand through the decision (Blaster before unless `from` says otherwise); returns the state after.
fn swung(from: Seen) -> St {
	let mut st = St::default();
	field(&mut st, &from, false);
	assert_eq!(field(&mut st, &from, true).request, Some(SWINGSHOT));
	let ready = Seen { held: SWINGSHOT, request: 0, ..from };
	assert_eq!(field(&mut st, &ready, true).circle, Some(true));
	st
}

#[test]
fn the_weapon_comes_back_after_fifteen_rest_fields() {
	let mut st = swung(at_target(BLASTER));
	let air = Seen { held: SWINGSHOT, air: true, hero: 0x2c, ..seen(SWINGSHOT, ALL) };
	for _ in 0..40 {
		assert_eq!(field(&mut st, &air, false), Out::default(), "not in the air");
	}
	let rest = seen(SWINGSHOT, ALL);
	for i in 1..BACK_FIELDS {
		assert_eq!(field(&mut st, &rest, false), Out::default(), "rest field {i}");
	}
	// Circle held resets the count.
	field(&mut st, &rest, true);
	for _ in 1..BACK_FIELDS {
		assert_eq!(field(&mut st, &rest, false), Out::default());
	}
	assert_eq!(field(&mut st, &rest, false), Out { circle: None, request: Some(BLASTER) });
	assert_eq!((st.pending, st.auto, st.back, st.rest), (0, 0, 0, 0));
}

#[test]
fn no_return_while_the_place_still_wants_the_gadget_or_the_player_changed_item() {
	// Still at the target: the Swingshot stays.
	let mut st = swung(at_target(BLASTER));
	let here = at_target(SWINGSHOT);
	for _ in 0..3 * BACK_FIELDS {
		assert_eq!(field(&mut st, &here, false), Out::default());
	}
	// The player picks another item himself: we forget ours.
	let mut st = swung(at_target(BLASTER));
	field(&mut st, &seen(5, ALL), false);
	assert_eq!(st.auto, 0);
	for _ in 0..3 * BACK_FIELDS {
		assert_eq!(field(&mut st, &seen(5, ALL), false), Out::default());
	}
}

#[test]
fn from_the_wrench_the_last_chosen_weapon_comes_back() {
	let from = Seen { last: BLASTER, ..at_target(WRENCH) };
	let mut st = swung(from);
	assert_eq!(st.back, BLASTER);
	let rest = Seen { last: SWINGSHOT, ..seen(SWINGSHOT, ALL) };
	let out: Vec<Out> = (0..BACK_FIELDS).map(|_| field(&mut st, &rest, false)).collect();
	assert_eq!(out.last(), Some(&Out { circle: None, request: Some(BLASTER) }));
	// A weapon not owned any more is not asked for.
	let mut st = swung(at_target(BLASTER));
	let rest = seen(SWINGSHOT, ALL & !(1 << BLASTER));
	assert!((0..2 * BACK_FIELDS).all(|_| field(&mut st, &rest, false) == Out::default()));
}

#[test]
fn a_swap_that_never_ends_is_given_up_and_our_request_taken_back() {
	let mut st = St::default();
	field(&mut st, &at_target(BLASTER), false);
	field(&mut st, &at_target(BLASTER), true);
	let stuck = Seen { request: SWINGSHOT, ..at_target(BLASTER) };
	for _ in 0..WAIT_MAX {
		assert_eq!(field(&mut st, &stuck, true).circle, Some(false));
	}
	assert_eq!(field(&mut st, &stuck, true), Out { circle: None, request: Some(0) });
	assert_eq!((st.pending, st.back), (0, 0));
	// Someone else's request in the word is left alone; leaving play gives up at once.
	let mut st = St::default();
	field(&mut st, &at_target(BLASTER), false);
	field(&mut st, &at_target(BLASTER), true);
	assert_eq!(field(&mut st, &Seen { mode: 3, request: 7, ..at_target(BLASTER) }, true), Out { circle: None, request: None });
	assert_eq!(st.pending, 0);
}

/// Off (the default) and every gate closed: the hand declines and writes nothing, the probe does not run.
#[test]
fn off_runs_nothing() {
	let _t = crate::pcopts::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let was = ON.swap(false, Ordering::Relaxed);
	let mut c = Cpu::new(Mem::new());
	c.mem.wr8(OWNED + SWINGSHOT, 1);
	let (ram, r, ic) = (c.mem.ram.clone(), c.r, c.icount);
	TARGET.store(true, Ordering::Relaxed);
	assert!(!items_tick(&mut c));
	assert!(TARGET.load(Ordering::Relaxed), "Off: the hand does not even clear its own flag");
	assert!(c.mem.ram == ram && c.r == r && c.icount == ic);
	// On, but the level has no code at the table address (blank RAM): no call, no write, no target.
	ON.store(true, Ordering::Relaxed);
	assert!(!items_tick(&mut c));
	assert!(!TARGET.load(Ordering::Relaxed));
	assert!(c.mem.ram == ram && c.r == r && c.icount == ic);
	// Novalis (level 1) has no selectors: nothing either.
	c.mem.wr8(LEVEL_BYTE, 1);
	let ram = c.mem.ram.clone();
	assert!(!probe(&mut c));
	assert!(c.mem.ram == ram);
	ON.store(was, Ordering::Relaxed);
	TARGET.store(false, Ordering::Relaxed);
}

#[test]
fn keep_puts_every_register_the_clock_and_the_memory_back() {
	let mut c = Cpu::new(Mem::new());
	c.r[29] = 0x1f_0000;
	c.r[4] = 0x1234_5678_9abc_def0_1122_3344_5566_7788;
	(c.hi, c.lo, c.sa, c.pc, c.npc, c.facc, c.fcr31, c.div_csr) = (1, 2, 3, 0x210290, 0x210294, 4, 5, 6);
	(c.f[3], c.vf[7][2], c.vacc[1], c.vi[9], c.cp0[12], c.cp0[13]) = (7, 8, 9, 10, 0x1_0001, 0x400);
	(c.icount, c.mem.now, c.mem.limit, c.mem.due, c.mem.tick, c.mem.irq_dirty) = (1000, 1000, 0, 1200, 1500, true);
	c.mem.wr32(CACHE + 8, 0xdead);
	c.mem.wr32(0x1f_0000 - 0x100, 0xbeef);
	c.mem.spr[5] = 1;
	c.mem.hw[9] = 2;
	c.mem.vu0d[3] = 4;
	let before = (c.mem.ram.clone(), c.mem.spr.clone(), c.mem.hw.clone(), c.mem.vu0d.clone());
	let regs = ((c.r, c.hi, c.lo, c.sa, c.pc, c.npc, c.f), (c.facc, c.fcr31, c.vf, c.vacc, c.vi, c.div_csr, c.cp0));
	// A record used before (other buffers, other sizes) is refilled.
	let mut keep = Keep::default();
	keep.take(&mut Cpu::new(Mem::new()), 2);
	keep.stack.1.resize(0x3000, 5);
	keep.take(&mut c, 2);
	// Held: nothing can fall due, no interrupt can be taken.
	assert_eq!((c.mem.limit, c.mem.due, c.mem.tick, c.mem.irq_dirty, c.cp0[12] & IRQ_ENABLE), (u64::MAX, u64::MAX, u64::MAX, false, 0));
	// What a probe does: registers, clock, caches, stack, scratchpad, hardware page, VU0 data.
	c.r = [7; 32];
	(c.hi, c.lo, c.sa, c.pc, c.npc, c.facc, c.fcr31, c.div_csr) = (0, 0, 0, 0xffff_fff0, 0, 0, 0, 0);
	(c.f, c.vf, c.vacc, c.vi) = ([1; 32], [[2; 4]; 32], [3; 4], [4; 32]);
	c.cp0[13] = 0;
	c.icount += 50_000;
	c.mem.now = c.icount;
	for i in 0..CACHE_WORDS {
		c.mem.wr32(CACHE + 4 * i, i + 1);
	}
	c.mem.wr32(0x1f_0000 - STACK_BELOW, 1);
	c.mem.wr32(0x1f_0000 - 4, 2);
	c.mem.spr.fill(9);
	c.mem.hw.fill(9);
	c.mem.vu0d.fill(9);
	keep.put_back(&mut c);
	assert!((c.mem.ram.clone(), c.mem.spr.clone(), c.mem.hw.clone(), c.mem.vu0d.clone()) == before);
	assert!(((c.r, c.hi, c.lo, c.sa, c.pc, c.npc, c.f), (c.facc, c.fcr31, c.vf, c.vacc, c.vi, c.div_csr, c.cp0)) == regs);
	assert_eq!((c.icount, c.mem.now, c.mem.limit, c.mem.due, c.mem.tick, c.mem.irq_dirty), (1000, 1000, 0, 1200, 1500, true));
}

/// RAM of a level field in play: `held` ready in hand, Swingshot and Blaster owned (what `decide` reads through `seen`).
fn play_ram(held: u32) -> Mem {
	let mut m = Mem::new();
	m.wr8(LEVEL_BYTE, 4);
	m.wr32(HELD, held);
	m.wr32(ITEM_STATE, READY);
	m.wr32(ITEM_MOBY, 0x30_0000);
	m.wr8(OWNED + SWINGSHOT, 1);
	m.wr8(OWNED + BLASTER, 1);
	m
}

/// A Circle at a target asks for the Swingshot (field 2 of the level; field 1 only starts the decision).
fn ask(m: &mut Mem, level: u32) {
	m.wr32(REQUEST, 0);
	assert_eq!(decide(m, level, 1, 0), 0);
	TARGET.store(true, Ordering::Relaxed);
	assert_eq!(decide(m, level, 2, CIRCLE), 0, "Circle held back");
	assert_eq!((m.rd32(REQUEST), ST.lock().unwrap().pending), (SWINGSHOT, SWINGSHOT));
}

/// B1 (review R10): a new machine or the front end (`reset`) and a new level forget a pending request, the return to come and the
/// probe's answers; a load on the same level then starts fresh (Circle as the pad has it, nothing asked for).
#[test]
fn a_reset_or_another_level_forgets_the_decision() {
	let _t = crate::pcopts::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	reset();
	let mut m = play_ram(BLASTER);
	ask(&mut m, 4);
	LIVE.store(true, Ordering::Relaxed);
	PROBE_OFF.store(4, Ordering::Relaxed);
	reset();
	let st = *ST.lock().unwrap();
	assert_eq!((st, TARGET.load(Ordering::Relaxed), PROBE_OFF.load(Ordering::Relaxed), LIVE.load(Ordering::Relaxed)), (FRESH, false, u32::MAX, false));
	m.wr32(REQUEST, 0);
	for f in 3..6 {
		assert_eq!(decide(&mut m, 4, f, CIRCLE), CIRCLE, "field {f}: Circle passes");
	}
	assert_eq!(m.rd32(REQUEST), 0);
	// Pending again, then the next field is another level's: forgotten too, and that level's probe is on.
	reset();
	ask(&mut m, 4);
	PROBE_OFF.store(5, Ordering::Relaxed);
	m.wr32(REQUEST, 0);
	assert_eq!(decide(&mut m, 5, 3, CIRCLE), CIRCLE);
	let st = *ST.lock().unwrap();
	assert_eq!((st.level, st.pending, TARGET.load(Ordering::Relaxed), PROBE_OFF.load(Ordering::Relaxed)), (5, 0, false, u32::MAX));
	assert_eq!(m.rd32(REQUEST), 0);
	reset();
}

/// B2 (review R10): what turns the probe off for a level.
#[test]
fn a_probe_is_bad_on_a_panic_no_return_a_fault_a_stop_or_too_many_instructions() {
	assert_eq!(bad(Some(true), 10_821, false, false), None);
	assert_eq!(bad(Some(true), PROBE_INSNS, false, false), None);
	for (returned, spent, stop, fault) in [(Some(true), PROBE_INSNS + 1, false, false), (Some(false), 0, false, false), (None, 0, false, false), (Some(true), 0, true, false), (Some(true), 0, false, true)] {
		assert!(bad(returned, spent, stop, fault).is_some(), "{returned:?} {spent} {stop} {fault}");
	}
}

fn state_dir(name: &str) -> Option<String> {
	[concat!(env!("CARGO_MANIFEST_DIR"), "/../../../oracle/states/"), concat!(env!("CARGO_MANIFEST_DIR"), "/../../../../oracle/states/")]
		.iter()
		.map(|b| format!("{b}{name}"))
		.find(|p| std::path::Path::new(p).join("eeMemory.bin").exists())
}

/// The probe on real savestates (Umbris near a Versa-Target and a Hydrodisplacer pad, Eudora near swing / pull targets and on a
/// lock pad, Oltanis): the answer is printed, and all of RAM, the scratchpad, the hardware page, VU0 data, every register and the
/// clock are the same after.
#[test]
fn the_probe_leaves_the_machine_as_it_found_it() {
	let names = ["fg1/7_0305", "fg1/7_0306", "fg1/7_0309", "fg1/13_0650", "fg1/14_0693", "fg2/15_0146"]
		.into_iter()
		.chain(["play/eudora1_0039", "play/eudora1_0040", "play/eudora1_0041"]);
	let _t = crate::pcopts::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	PROBE_OFF.store(u32::MAX, Ordering::Relaxed);
	let mut seen_hit = Vec::new();
	ee_game::register();
	for name in names {
		let dir = state_dir(name).unwrap_or_else(|| panic!("{name}: savestate missing (oracle/states of the lane or of the project root)"));
		let mut m = ee_oracle::Machine::load(&dir);
		m.enable_rec();
		let c = &mut m.cpu;
		// The probe must really call retail's selectors here: this level has both and the code at them is the specs' code.
		let level = c.mem.rd8(LEVEL_BYTE) as usize;
		let (sw, pl) = (pick(&SWING, &SWING_USA)[level], pick(&PULL, &PULL_USA)[level]);
		assert!(sw != 0 && spec(0).matches(&c.mem, sw) && spec(1).matches(&c.mem, pl), "{name}: level {level} selectors not found");
		let before = (c.mem.ram.clone(), c.mem.spr.clone(), c.mem.hw.clone(), c.mem.vu0d.clone());
		let regs = ((c.r, c.hi, c.lo, c.sa, c.pc, c.npc, c.f), (c.facc, c.fcr31, c.vf, c.vacc, c.vi, c.div_csr, c.cp0));
		let clock = (c.icount, c.mem.now, c.mem.limit, c.mem.due, c.mem.tick, c.mem.irq_dirty);
		let hit = probe(c);
		seen_hit.push((name, c.mem.rd8(LEVEL_BYTE), hit));
		assert!((c.mem.ram.clone(), c.mem.spr.clone(), c.mem.hw.clone(), c.mem.vu0d.clone()) == before, "{name}: memory changed");
		assert!(((c.r, c.hi, c.lo, c.sa, c.pc, c.npc, c.f), (c.facc, c.fcr31, c.vf, c.vacc, c.vi, c.div_csr, c.cp0)) == regs, "{name}: registers changed");
		assert_eq!((c.icount, c.mem.now, c.mem.limit, c.mem.due, c.mem.tick, c.mem.irq_dirty), clock, "{name}: clock changed");
		assert_eq!(PROBE_OFF.load(Ordering::Relaxed), u32::MAX, "{name}: the probe went wrong");
		assert!(c.stop.is_none() && c.mem.fault.is_none(), "{name}: stop / fault left");
		// No probe on a level whose probe went wrong, nor with a stop pending: no answer where there is one.
		PROBE_OFF.store(level as u32, Ordering::Relaxed);
		assert!(!probe(c) || !hit);
		PROBE_OFF.store(u32::MAX, Ordering::Relaxed);
		c.stop = Some("test".into());
		assert!(!probe(c) || !hit);
		c.stop = None;
		assert_eq!(probe(c), hit, "{name}: the same answer again (the record is refilled)");
	}
	eprintln!("auto_gadget probe on savestates (name, level, hit): {seen_hit:?}");
}
