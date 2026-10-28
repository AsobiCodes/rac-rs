//! The ABI 1.1 host entries for native C/C++ mods: `ram_ptr`, `call_game`, `relocate`, `relocate_level`, `loaded_level`, `stack_push`,
//! `stack_pop`. A native mod reads and writes the game's RAM through the pointer (no callback per access), calls game functions by number
//! (funcs.rs), and asks for room on the game's stack for the arguments of such a call.
//!
//! RAM: the pointer is the first machine's RAM buffer (set by `set_ram` before the mods load). A later `Retail::load` builds a NEW machine with a
//! new buffer; native mods keep the old pointer, so after such a load `machine_check` switches the mods off (`stale`): ALL of them, script
//! mods too, because they share the one loader tick: no `on_frame`, no events, no hooks, no calls; the game runs on as without them until it
//! is restarted.
//!
//! call_game runs a retail function from inside a callback of the DLL, on the machine in scope (host.rs `scope`): inside a hook it is like
//! `call_original`; between two fields (`tick`, `pad`) it runs on the machine as it stands at the field boundary. It never leaves a trace in
//! the registers: they are saved first and restored after; only the instruction count stays advanced. See `do_call` for the refusals.

use super::cabi::{Host, Value};
use super::counts::{self, Counter};
use super::host::{cpu, state};
use ee_oracle::cpu::Cpu;
use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use std::sync::Mutex;

/// The return address the oracle uses for calls out of hands (`rec::CALL_RET`): no code lives there.
const CALL_RET: u32 = 0x10;
/// Nested call_game depth (a game function can reach a hook whose mod calls call_game again).
const MAX_DEPTH: u32 = 8;
/// The executable's data ends here; from 0x15EF00 the overlay's data starts (it differs per level).
const EXE_DATA_END: u32 = 0x15_EF00;
/// The game stack never goes below the executable's start.
const STACK_FLOOR: u32 = 0x10_0000;
/// The level number word (`oracle LEVEL`).
pub(super) const LEVEL_WORD: usize = 0x15_ED84;
/// `OMNI_LEVEL_UNSUPPORTED`.
const LEVEL_UNSUPPORTED: i32 = i32::MIN;

static RAM_PTR: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static STALE: AtomicBool = AtomicBool::new(false);
static DEPTH: AtomicU32 = AtomicU32::new(0);

/// A line in the run log at most once per key (a mod that fails on every tick must not flood it).
pub fn say_once(key: &str, msg: &str) -> bool {
	static SAID: Mutex<Option<HashSet<String>>> = Mutex::new(None);
	let mut g = SAID.lock().unwrap_or_else(|e| e.into_inner());
	let first = g.get_or_insert_with(HashSet::new).insert(key.to_string());
	if first {
		super::logq::line(&format!("omni: {msg}"));
	}
	first
}

/// The mods were switched off by a second machine (see the module doc).
pub fn stale() -> bool {
	STALE.load(Ordering::Relaxed)
}

/// Records the RAM of the machine the mods load on (before `omni_load`).
pub fn set_ram(c: &mut Cpu) {
	RAM_PTR.store(c.mem.ram.as_mut_ptr(), Ordering::SeqCst);
}

/// Every `Retail::load` after the mods started: a machine with another RAM buffer invalidates the pointer native mods hold.
pub fn machine_check(c: &mut Cpu, native: bool) {
	if native && c.mem.ram.as_mut_ptr() != RAM_PTR.load(Ordering::SeqCst) && !STALE.swap(true, Ordering::SeqCst) {
		eprintln!("omni: a second game load built a new machine; native mods hold the old game memory, so all mods are switched off until the game is restarted");
	}
}

/// Room on the stack: (new sp, token = old sp) for `size` bytes below `sp`, 16-byte aligned; None for size 0 or no room.
pub fn stack_alloc(sp: u64, size: u32) -> Option<(u32, u32)> {
	let sp = sp as u32;
	let new = sp.checked_sub(size).filter(|_| size != 0)? & !15;
	(new >= STACK_FLOOR && sp < 0x0200_0000).then_some((new, sp))
}

/// Where call_game puts its arguments: integers in a0..a7 (r4..r11), floats in f12..f19, each class in order.
#[derive(Debug, PartialEq)]
pub struct Placement {
	pub ints: Vec<u64>,
	pub floats: Vec<u32>,
}

/// The register contents for `args`; None when there are more than 8 of either class. Integers that are 32-bit values go in sign-extended
/// (the EE's convention for int / pointer arguments), wider ones keep their bit pattern; floats are rounded to f32.
pub fn place_args(args: &[Value]) -> Option<Placement> {
	let mut p = Placement { ints: Vec::new(), floats: Vec::new() };
	for a in args {
		if a.is_float != 0 {
			p.floats.push((a.f as f32).to_bits());
		} else {
			let v = a.i;
			p.ints.push(if v as i32 as i64 == v || v as u32 as i64 == v { v as i32 as i64 as u64 } else { v as u64 });
		}
	}
	(p.ints.len() <= 8 && p.floats.len() <= 8).then_some(p)
}

/// `loaded_level` from the level word: 0..=18 is a level, anything else (the title screen, a menu, a load) is -1, the executable's.
pub fn loaded_level_of(word: i32) -> i32 {
	if (0..=18).contains(&word) {
		word
	} else {
		-1
	}
}

/// `relocate` / `relocate_level`: the address of the global the mod names (a reference address of the format's numbering: executable globals and the
/// NTSC-U level-0 program) in the program of `level` (-1: none loaded) on the disc that runs. The executable's globals are where they are on this
/// disc; the overlay's (from 0x15EF00) go through the per-level map (data_map.rs), None when it has no run for the address.
pub fn relocate_in(usa: bool, level: i32, addr: u32) -> Option<u32> {
	if addr < EXE_DATA_END {
		return Some(addr);
	}
	super::data_map::builtin().lookup(usa, level, addr)
}

/// The level word read through the RAM pointer the mods hold (also before the machine is in scope: the mods ask while they load).
fn level_word_now() -> i32 {
	let p = RAM_PTR.load(Ordering::SeqCst);
	if p.is_null() {
		return -1;
	}
	// SAFETY: the pointer is the first machine's RAM (32 MB) and LEVEL_WORD + 4 lies inside it; the buffer lives for the process (set_ram).
	i32::from_le_bytes(unsafe { std::ptr::read_unaligned(p.add(LEVEL_WORD) as *const [u8; 4]) })
}

pub(super) fn level_word(c: &Cpu) -> i32 {
	level_word_in(&c.mem.ram)
}

/// The level word of a RAM image (-1 when the image is too short).
pub(super) fn level_word_in(ram: &[u8]) -> i32 {
	ram.get(LEVEL_WORD..LEVEL_WORD + 4).map_or(-1, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// The CPU state a call_game must give back (everything a function can change except memory and the instruction count).
struct Saved {
	r: [u128; 32],
	hi: u128,
	lo: u128,
	sa: u32,
	pc: u32,
	npc: u32,
	f: [u32; 32],
	facc: u32,
	fcr31: u32,
	vf: [[u32; 4]; 32],
	vacc: [u32; 4],
	vi: [u32; 32],
	tick: u64,
}

impl Saved {
	fn take(c: &Cpu) -> Saved {
		Saved { r: c.r, hi: c.hi, lo: c.lo, sa: c.sa, pc: c.pc, npc: c.npc, f: c.f, facc: c.facc, fcr31: c.fcr31, vf: c.vf, vacc: c.vacc, vi: c.vi, tick: c.mem.tick }
	}

	fn put(self, c: &mut Cpu) {
		(c.r, c.hi, c.lo, c.sa, c.pc, c.npc) = (self.r, self.hi, self.lo, self.sa, self.pc, self.npc);
		(c.f, c.facc, c.fcr31, c.vf, c.vacc, c.vi) = (self.f, self.facc, self.fcr31, self.vf, self.vacc, self.vi);
		c.mem.tick = self.tick;
		c.mem.now = c.icount;
	}
}

fn put_args(c: &mut Cpu, p: &Placement) {
	for (i, &v) in p.ints.iter().enumerate() {
		c.s(4 + i, v);
	}
	for (i, &b) in p.floats.iter().enumerate() {
		c.f[12 + i] = b;
	}
}

/// Runs game function `number` with `args`. Refused (Err with the reason) when: the mods are still loading (`omni::get` is None), no machine is
/// in scope, the machine stopped, native mods are stale, the depth is 8, the number is unknown or not in the loaded code, or there are more than
/// 8 arguments of a class. Between two fields the cold-boot field timer is held off for the call, so it cannot fire inside it.
fn do_call(user: *mut c_void, number: u32, args: &[Value], want_float: bool) -> Result<(Value, &'static super::funcs::FuncSpec), String> {
	if super::get().is_none() {
		return Err("game functions cannot be called while the mods are loading".into());
	}
	let c = cpu().ok_or("no game machine in scope")?;
	if c.stop.is_some() {
		return Err("the game machine has stopped".into());
	}
	if stale() {
		return Err("native mods are switched off (a second game load)".into());
	}
	depth_check()?;
	let ntsc = state(user).disc == "ntsc";
	let (entry, spec) = super::funcs::resolve(c, number, ntsc, level_word(c))?;
	let place = place_args(args).ok_or("more than 8 integer or 8 float arguments")?;
	run_at(c, entry, |c| put_args(c, &place), |c| if want_float { Value::float(f32::from_bits(c.f[0]) as f64) } else { Value::int(c.g(2) as i32 as i64) })
		.map(|v| (v, spec))
		.map_err(|why| format!("game function 0x{number:x} {why}"))
}

/// A `run_at` is live on the stack (`DEPTH`): it will catch what a callee unwinds with (bridge.rs `park_panic`).
pub(super) fn calls_live() -> u32 {
	DEPTH.load(Ordering::SeqCst)
}

/// Refused when game calls are already nested `MAX_DEPTH` deep.
pub(super) fn depth_check() -> Result<(), String> {
	if DEPTH.load(Ordering::SeqCst) >= MAX_DEPTH {
		return Err(format!("game calls nested deeper than {MAX_DEPTH}"));
	}
	Ok(())
}

/// Runs the retail code at `entry` as a call from the host: `set` puts the arguments into the registers, `read` reads the result, and the CPU
/// state comes back as it was (memory and the instruction count excepted). Between fields the cold-boot field timer is held off for the call.
/// Err: the function did not return, or panicked.
pub(super) fn run_at<T>(c: &mut Cpu, entry: u32, set: impl FnOnce(&mut Cpu), read: impl FnOnce(&Cpu) -> T) -> Result<T, &'static str> {
	if super::bridge::unwound() {
		return Err("was refused: the hooked function already left at a frame wait");
	}
	if super::bridge::panic_parked() {
		return Err("was refused: an earlier game call was cut short by a panic that is still unwinding");
	}
	let saved = Saved::take(c);
	let held = !super::bridge::in_hook();
	if held {
		c.mem.tick = u64::MAX;
	}
	set(c);
	c.s(31, CALL_RET as u64);
	DEPTH.fetch_add(1, Ordering::SeqCst);
	let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| super::frame_wait::call(c, entry, CALL_RET)));
	DEPTH.fetch_sub(1, Ordering::SeqCst);
	let result = match ran {
		Ok(true) => Ok(read(c)),
		Ok(false) => Err("did not return (it waits for the next frame)"),
		// The oracle unwinds the game thread with panics that carry no text (`RecRebuilt`: the recompiled table was rebuilt under compiled
		// frames; `Quit`): on the game thread they must reach its loop, so they are parked and the DLL call that made this one returns first; between fields
		// (the worker) nothing could catch them: the call answers an error and the game thread is told through `rec_unwind` (bridge.rs).
		Err(p) if super::bridge::is_unwind_signal(&p) => {
			super::bridge::park_panic(c, p);
			Err("was cut short (the game thread is being unwound)")
		}
		Err(_) => Err("panicked"),
	};
	saved.put(c);
	if held {
		restore_due(c);
	}
	result
}

/// The forced-vblank deadline of the cold-boot machine for the timer `Saved::put` gave back: a VIF1 stop that matured while the timer was held
/// off (`tick` = MAX) left `due` at MAX (oracle `Mem::mature`), and the field would come late. Same rule as the oracle's `rearm_tick`.
fn restore_due(c: &mut Cpu) {
	c.mem.set_due(if c.mem.vif1_pend.is_some() { c.mem.dma_due.min(c.mem.tick) } else { c.mem.tick });
}

/// The game's RAM. Writing DATA through it is supported; writing game CODE (0x100000..0x400000 inside a recompiled function) is not: the recompiled
/// functions never see it and the next table build drops their whole set to the interpreter (codeguard.rs refuses it for `write_mem`; a raw
/// pointer cannot be checked). Use a hook point to change what a game function does.
unsafe extern "C" fn ram_ptr_cb(_user: *mut c_void) -> *mut u8 {
	RAM_PTR.load(Ordering::SeqCst)
}

unsafe extern "C" fn call_game_cb(user: *mut c_void, addr: u32, args: *const Value, nargs: u32, ret: *mut Value) -> i32 {
	// SAFETY: the loader passes `nargs` values; `ret` is one OmniValue (or NULL).
	let given: &[Value] = if args.is_null() { &[] } else { unsafe { std::slice::from_raw_parts(args, nargs as usize) } };
	let want_float = !ret.is_null() && unsafe { (*ret).is_float != 0 };
	let result = do_call(user, addr, given, want_float);
	// An answered call came with the spec it found (`resolve`): counted on it, no second lookup.
	if let Ok((_, spec)) = &result {
		spec.calls.fetch_add(1, Ordering::Relaxed);
	} else {
		counts::bump(Counter::CallGameRefused);
	}
	match result {
		Ok((v, _)) => {
			if !ret.is_null() {
				// SAFETY: as above.
				unsafe { *ret = v };
			}
			1
		}
		Err(why) => {
			say_once(&format!("call_game {addr:x} {why}"), &format!("call_game(0x{addr:x}): {why}"));
			0
		}
	}
}

unsafe extern "C" fn relocate_cb(user: *mut c_void, addr: u32, out: *mut u32) -> i32 {
	relocate_answer(state(user).ntsc, loaded_level_of(level_word_now()), addr, out)
}

unsafe extern "C" fn relocate_level_cb(user: *mut c_void, level: i32, addr: u32, out: *mut u32) -> i32 {
	relocate_answer(state(user).ntsc, level, addr, out)
}

/// Writes the answer to `out` and counts it (`counts`): 1, or 0 with one log line per address and level when it is unmapped.
fn relocate_answer(usa: bool, level: i32, addr: u32, out: *mut u32) -> i32 {
	match relocate_in(usa, level, addr) {
		Some(a) => {
			counts::bump(Counter::Relocate);
			// SAFETY: `out` is one u32 the loader gave.
			unsafe { *out = a };
			1
		}
		None => {
			counts::bump(Counter::RelocateUnmapped);
			say_once(&format!("relocate {addr:x} {level}"), &format!("relocate(0x{addr:x}) in level {level}: not in the overlay data map, the mod gets 0 (its feature that reads it does not work)"));
			0
		}
	}
}

unsafe extern "C" fn loaded_level_cb(_user: *mut c_void) -> i32 {
	cpu().map_or(LEVEL_UNSUPPORTED, |c| loaded_level_of(level_word(c)))
}

unsafe extern "C" fn stack_push_cb(_user: *mut c_void, size: u32, saved: *mut u32) -> u32 {
	let Some(c) = cpu() else { return 0 };
	match stack_alloc(c.g(29), size) {
		Some((new, old)) => {
			c.s(29, new as u64);
			// SAFETY: `saved` is one u32 the loader gave.
			unsafe { *saved = old };
			new
		}
		None => {
			say_once("stack_push", "stack_push: no room on the game stack (or size 0)");
			0
		}
	}
}

unsafe extern "C" fn stack_pop_cb(_user: *mut c_void, saved: u32) {
	if let Some(c) = cpu() {
		c.s(29, saved as u64);
	}
}

/// The stack pointer to compare with after a mod ran (`sp_check`).
pub fn sp_before(c: &Cpu) -> u64 {
	c.g(29)
}

/// A mod that pushed stack room and did not pop it leaves the game's stack lower: warn once and put it back (written only when it changed).
pub fn sp_check(c: &mut Cpu, before: u64, what: &str) {
	if c.g(29) != before {
		say_once(&format!("sp {what}"), &format!("a mod left the game stack pointer changed in {what} (stack_push without stack_pop); restored"));
		c.s(29, before);
	}
}

/// Fills the ABI 1.1 entries of the host.
pub fn fill(h: &mut Host) {
	h.ram_ptr = Some(ram_ptr_cb);
	h.call_game = Some(call_game_cb);
	h.relocate = Some(relocate_cb);
	h.relocate_level = Some(relocate_level_cb);
	h.loaded_level = Some(loaded_level_cb);
	h.stack_push = Some(stack_push_cb);
	h.stack_pop = Some(stack_pop_cb);
	super::raycast::fill(h);
}
