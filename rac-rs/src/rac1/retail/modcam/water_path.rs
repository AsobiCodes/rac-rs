//! Retail's water probe along the Modern camera's path (GitHub #39: the underwater look and sound stayed on above the water).
//!
//! [C] The camera water flag (Novalis 0x167414 + the level shift; it drives the retail fog swap, the blue tint sprite and the 989
//! sound driver's underwater drop) is written by one probe per level (Novalis FUN_0020e8f0, found below by its code words), once a
//! tick, after the camera main published the eye into the camera globals (0x1671c0): it casts a vertical segment from eye z + 0.75
//! down to eye z - 0.75 (seg_collide, mask 0x12) and, when the first hit is an id-0 water face, sets the flag to 1 for an eye below
//! face + 0.04 and to 0 above it. With no water face in that 1.5 m window the flag is left as it was (sticky). The retail camera
//! moves in small steps, so it always crosses the 0.71 m band above the face with a probe inside it. The Modern camera can lift the
//! eye by more than that in one field (a mouse flick): the band is skipped, the flag stays 1 and the look and sound stay under
//! water with the camera metres above it.
//!
//! The fix feeds retail's own probe what a continuous camera gives it: when the Modern camera moved the eye by more than the band
//! in one field (`follow` hand, after `step`), the probe runs at interim eye positions on the straight path from the eye the last
//! tick published to the new eye (at most `STEP` apart), in path order. Retail's own write of the flag is the only persistent
//! effect; everything else the probe touches is put back exactly: the camera position words, seg_collide's result block, the moby
//! touch ring and query stamps, seg_collide's moby collision-mesh cache, the stack below the caller, every register, the instruction
//! clock and the event state, and, because a cache fill runs retail's joint pose (FUN_00267b58), the machine state its SPR DMA kicks
//! change: [C] each kick of channel 9 / 8 writes the D9 / D8 registers and sets that channel's D_STAT bit (CIS), marks the interrupt
//! lines dirty (oracle mem.rs hw_wr), copies through the scratchpad, and an interrupt taken in the walk would also set the Cause IP
//! bits and run a handler. So the walk runs with Status.IE / EIE off (no interrupt is taken inside it), and the hardware register
//! page, Cause and the 16 KB scratchpad are saved before and put back after it. Not undone: debug counters (hw_reads / dma_kicks).
//! The Classic camera gets here only where the hand itself moves the eye: with the strafe aim held (RMB / R2: `classic::why` sets
//! BORROW and `step` writes the eye) and while the blend fields ease the eye back after the aim is released or a hand-back; a
//! Classic camera that is left alone does not move the eye, and with no move there is no call.
use super::*;
use std::sync::{Mutex, OnceLock};

/// A vertical eye move above this (m in one field) can skip the band in which the probe updates the flag (0.71 m above the face,
/// 0.79 m on the way down).
const TRIGGER: f32 = 0.7;
/// Largest distance between two probe positions (smaller than the 0.71 m band: each band holds a probe).
const STEP: f32 = 0.65;
/// At most this many probe steps per field (so |dz| up to 10.4 m); a bigger move is a cut or a respawn, not a camera path.
const MAX_STEPS: usize = 16;
/// Horizontal distance above which two eyes are no camera path either (a respawn or a scene cut: the interim points would lie over
/// some other water).
const MAX_DXY: f32 = 16.0;
/// Stack bytes saved and put back below the probe's caller frame (the probe, seg_collide and its helpers write there).
const STACK_BELOW: u32 = 0x800;

/// The eye positions between `prev` (the eye the last probe saw) and `new` at which the probe has to run so that no flag band is
/// skipped: empty unless the eye moved more than `TRIGGER` vertically on a believable camera path; else the points
/// `prev + (new - prev) * i / n` for `i` in `1..n`, `n = ceil(|dz| / STEP)`, in path order (the ends are probed by retail itself).
pub(super) fn path_points(prev: [f32; 3], new: [f32; 3]) -> Vec<[f32; 3]> {
	let dz = new[2] - prev[2];
	let (dx, dy) = (new[0] - prev[0], new[1] - prev[1]);
	if !(dz.abs() > TRIGGER) || !(dx * dx + dy * dy <= MAX_DXY * MAX_DXY) {
		return Vec::new();
	}
	let n = (dz.abs() / STEP).ceil();
	if n > MAX_STEPS as f32 {
		return Vec::new();
	}
	let n = n as usize;
	(1..n)
		.map(|i| {
			let t = i as f32 / n as f32;
			[prev[0] + (new[0] - prev[0]) * t, prev[1] + (new[1] - prev[1]) * t, prev[2] + (new[2] - prev[2]) * t]
		})
		.collect()
}

/// The first words of the probe that do not depend on the level's addresses: (word index, word) from FUN_0020e8f0 (stack frame, the
/// saved registers, `li a0, 6` and `lh v1, 0x86(v0)` = camera class 6, the 0.75 constant and its add / sub).
const SIG: [(usize, u32); 15] = [
	(0, 0x27bd_ffa0),
	(2, 0x7fb2_0040),
	(4, 0x7fbf_0050),
	(5, 0x0060_902d),
	(6, 0x7fb1_0030),
	(7, 0x2404_0006),
	(8, 0x7fb0_0020),
	(10, 0x8443_0086),
	(11, 0x1064_0003),
	(13, 0x1040_0005),
	(25, 0x0000_802d),
	(26, 0x3c01_3f40),
	(27, 0x4481_1000),
	(29, 0x4602_0840),
	(30, 0x4602_0001),
];

/// The word index in `words` at which the probe starts, when exactly one place carries the signature.
fn find_sig(words: &[u32]) -> Option<usize> {
	let last = SIG[SIG.len() - 1].0;
	let mut found = None;
	for at in 0..words.len().saturating_sub(last) {
		if SIG.iter().all(|&(i, w)| words[at + i] == w) {
			if found.is_some() {
				return None;
			}
			found = Some(at);
		}
	}
	found
}

/// The probe of the level whose segment test sits at `lv.seg` (PAL and USA: the probe lies 0x2e80..0x2e90 bytes below it); cached per
/// (level, seg). None = not found: nothing is done on that level.
fn probe_addr(c: &mut Cpu, lv: &Lv) -> Option<u32> {
	static CACHE: Mutex<Vec<(u32, u32, Option<u32>)>> = Mutex::new(Vec::new());
	let mut cache = CACHE.lock().ok()?;
	if let Some(&(_, _, p)) = cache.iter().find(|e| e.0 == lv.level && e.1 == lv.seg) {
		return p;
	}
	let lo = lv.seg - 0x8000;
	let words: Vec<u32> = (lo..lv.seg).step_by(4).map(|a| c.mem.rd32(a)).collect();
	let p = find_sig(&words).map(|i| lo + 4 * i as u32);
	if path_log() {
		eprintln!("UNDER_PATH probe level {} seg {:#x} -> {:?}", lv.level, lv.seg, p.map(|p| format!("{p:#x}")));
	}
	cache.push((lv.level, lv.seg, p));
	p
}

/// [PC] Dev aid: RAC_UNDER_LOG=1 prints one UNDER_PATH line per field the walk ran (and checks that nothing but the flag changed).
fn path_log() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var_os("RAC_UNDER_LOG").is_some())
}

/// Called by the `follow` hand in place of its final `regs.restore(c)`: `prev` is the camera position words (0x1671c0) as the last
/// probe saw them, `e0` the eye retail's follow update left in the record, before `step` replaced it with the Modern pose.
pub(super) fn walk(c: &mut Cpu, lv: &Lv, rec: u32, prev: [f32; 3], e0: [f32; 3], regs: Regs) {
	let new = rv(c, rec + 0x30);
	let pts = if new == e0 { Vec::new() } else { path_points(prev, new) };
	if let Some(probe) = (!pts.is_empty()).then(|| probe_addr(c, lv)).flatten() {
		run(c, lv, probe, &pts, (prev, new));
	}
	regs.restore(c);
}

/// Bytes of seg_collide's moby collision-mesh cache: 8 round-robin slots, each a key (8 bytes: the actor's +0xa8 stamp and a flag) in the
/// table at `*(res + 8)` and a joint-prefix result (0x800 bytes, FUN_00267b58) in the table at `*(res + 4)`; `*(res + 0xc)` is the next
/// slot (`& 7`, inside the result block, which `run` puts back too).
const CACHE_KEYS: u32 = 8 * 8;
const CACHE_DATA: u32 = 8 * 0x800;

/// The cache slots a probe may fill (a moby query with a mask that reaches mobys: FUN_002b8240 / FUN_00267b58): saved with their
/// addresses so a later retail query finds the slots and the index it would have found without the walk.
struct MeshCache {
	keys: (u32, Vec<u8>),
	data: (u32, Vec<u8>),
}

impl MeshCache {
	/// None when the two table pointers do not lie in RAM (the level has not set the cache up): the walk then does not run.
	fn save(c: &mut Cpu, lv: &Lv) -> Option<MeshCache> {
		let (data, keys) = (c.mem.rd32(lv.res + 4), c.mem.rd32(lv.res + 8));
		let ram_len = c.mem.ram.len();
		let fits = |a: u32, n: u32| a >= 0x10_0000 && a as usize + n as usize <= ram_len; // plain RAM, above the kernel's page
		(fits(keys, CACHE_KEYS) && fits(data, CACHE_DATA)).then(|| MeshCache { keys: (keys, ram(c, keys, CACHE_KEYS)), data: (data, ram(c, data, CACHE_DATA)) })
	}
	fn put_back(&self, c: &mut Cpu) {
		put(c, self.keys.0, &self.keys.1);
		put(c, self.data.0, &self.data.1);
	}
	/// Whether the probe filled a slot (dev log).
	fn changed(&self, c: &Cpu) -> bool {
		ram(c, self.keys.0, CACHE_KEYS) != self.keys.1 || ram(c, self.data.0, CACHE_DATA) != self.data.1
	}
}

/// Status.IE and Status.EIE: an interrupt is taken only with both set (oracle cpu.rs `irq_taken`).
const IRQ_ENABLE: u32 = 0x1_0001;

/// The machine's instruction clock and event state while the probe runs. The walk is not part of the game's timeline: its
/// instructions are not counted (`icount` / `mem.now` are put back), and nothing that is due by the clock may fire inside it, neither
/// the cold-boot field timer, a matured VIF1 stop nor an interrupt: the event state is held off for the walk and Status.IE / EIE are
/// cleared, so a line that was pending before the walk, or one a DMA kick of the probe raises, is not taken inside it. Everything is
/// given back unchanged (a line that was pending stays pending and is taken at the next block entry, as without the walk).
struct Clock {
	icount: u64,
	now: u64,
	limit: u64,
	due: u64,
	tick: u64,
	dirty: bool,
	status: u32,
}

impl Clock {
	fn hold(c: &mut Cpu) -> Clock {
		let m = &mut c.mem;
		let held = Clock { icount: c.icount, now: m.now, limit: m.limit, due: m.due, tick: m.tick, dirty: m.irq_dirty, status: c.cp0[12] };
		(m.tick, m.due, m.limit, m.irq_dirty) = (u64::MAX, u64::MAX, u64::MAX, false);
		c.cp0[12] &= !IRQ_ENABLE;
		held
	}
	fn release(self, c: &mut Cpu) {
		c.icount = self.icount;
		c.cp0[12] = self.status;
		let m = &mut c.mem;
		(m.now, m.limit, m.due, m.tick, m.irq_dirty) = (self.now, self.limit, self.due, self.tick, self.dirty);
	}
}

/// The hardware state a slot fill leaves behind (see the module doc): the Cause register (the IP bits `try_irq` rewrites), the
/// whole hardware register page (D_STAT, the D8 / D9 channel registers, INTC) and the scratchpad (SPR DMA target; the probe and
/// joint pose spill their registers and lists there). Saved on the fields that walk only.
struct Hw {
	cause: u32,
	regs: Vec<u8>,
	spr: Vec<u8>,
}

impl Hw {
	fn save(c: &Cpu) -> Hw {
		Hw { cause: c.cp0[13], regs: c.mem.hw.clone(), spr: c.mem.spr.clone() }
	}
	fn put_back(&self, c: &mut Cpu) {
		c.cp0[13] = self.cause;
		c.mem.hw.clone_from(&self.regs);
		c.mem.spr.clone_from(&self.spr);
	}
	/// (hardware register bytes, scratchpad bytes) that differ from the saved copy (dev log).
	fn changed(&self, c: &Cpu) -> (usize, usize) {
		let diff = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|p| p.0 != p.1).count();
		(diff(&self.regs, &c.mem.hw), diff(&self.spr, &c.mem.spr))
	}
}

/// Runs the probe at each point and puts everything back but the flag the probe itself writes. Nothing runs when the mesh cache
/// cannot be located.
fn run(c: &mut Cpu, lv: &Lv, probe: u32, pts: &[[f32; 3]], ends: ([f32; 3], [f32; 3])) {
	let Some(cache) = MeshCache::save(c, lv) else { return };
	let (pos, flag) = (lv.cam_pos(), lv.water_flag());
	let flag_before = c.mem.rd32(flag);
	let whole = path_log().then(|| c.mem.ram.to_vec());
	let saved_pos = ram(c, pos, 0x10);
	let res = ram(c, lv.res, 0x80);
	let ring = ram(c, lv.ring, 0x1000);
	let ms = mobys(c, lv.level);
	let touch = stamps::save(c, &ms);
	let sp = c.g(29) as u32;
	let top = sp.wrapping_sub(0x200) & !0xf;
	let stack = ram(c, top.wrapping_sub(STACK_BELOW), STACK_BELOW + 0x200);
	let irqs = c.irqs;
	let hw = Hw::save(c);
	let clock = Clock::hold(c);
	let spent = c.icount;
	for p in pts {
		wv(c, pos, *p);
		c.s(29, top.wrapping_sub(0x40) as i32 as i64 as u64);
		c.call(probe);
	}
	let (spent, filled, taken) = (c.icount.wrapping_sub(spent), path_log() && cache.changed(c), c.irqs != irqs);
	let hw_diff = path_log().then(|| hw.changed(c));
	cache.put_back(c);
	put(c, pos, &saved_pos);
	put(c, lv.res, &res);
	put(c, lv.ring, &ring);
	stamps::restore(c, &ms, &touch, true);
	put(c, top.wrapping_sub(STACK_BELOW), &stack);
	c.s(29, sp as i32 as i64 as u64);
	hw.put_back(c);
	clock.release(c);
	if let Some(mut before) = whole {
		let at = (flag & 0x01ff_ffff) as usize;
		before[at..at + 4].copy_from_slice(&c.mem.ram[at..at + 4]);
		let (total, ranges) = stamps::diff(&before, &c.mem.ram);
		let r: Vec<String> = ranges.iter().map(|&(s, l)| format!("{s:#x}+{l}")).collect();
		eprintln!(
			"UNDER_PATH level {} prev_z {} new_z {} points {} flag {:#x}->{:#x} other_ram_diff_bytes {} ranges {} insns {} mesh_cache_filled {} irq_taken {} hw_bytes_changed {:?}",
			lv.level,
			ends.0[2],
			ends.1[2],
			pts.len(),
			flag_before,
			c.mem.rd32(flag),
			total,
			r.join(","),
			spent,
			filled,
			taken,
			hw_diff.unwrap_or_default()
		);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn gap(a: [f32; 3], b: [f32; 3]) -> f32 {
		((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
	}

	#[test]
	fn small_moves_get_no_interim_points() {
		assert!(path_points([0.0, 0.0, 65.0], [0.0, 0.0, 65.7]).is_empty());
		assert!(path_points([0.0, 0.0, 65.0], [3.0, 2.0, 65.69]).is_empty());
		assert!(path_points([0.0, 0.0, 65.0], [0.0, 0.0, 65.0]).is_empty());
		assert!(path_points([0.0, 0.0, 65.0], [0.0, 0.0, 64.3]).is_empty());
	}

	#[test]
	fn a_flick_up_gets_points_in_path_order_at_most_a_step_apart() {
		let (a, b) = ([1.0, 2.0, 65.37], [2.0, 4.0, 66.80]); // the repro: +1.43 m in one field
		let p = path_points(a, b);
		assert_eq!(p.len(), 2); // n = ceil(1.43 / 0.65) = 3
		let mut chain = vec![a];
		chain.extend(p.iter().copied());
		chain.push(b);
		for w in chain.windows(2) {
			assert!(w[1][2] > w[0][2]); // path order: rising
			assert!((w[1][2] - w[0][2]).abs() <= STEP + 1e-5);
		}
		assert!((p[0][0] - (1.0 + 1.0 / 3.0)).abs() < 1e-5 && (p[1][1] - (2.0 + 4.0 / 3.0)).abs() < 1e-5); // x / y interpolate too
		assert!(p.iter().all(|q| q[2] > a[2] && q[2] < b[2])); // the ends are not repeated
	}

	#[test]
	fn a_drop_gets_points_going_down() {
		let (a, b) = ([0.0, 0.0, 70.0], [0.0, 0.0, 66.0]);
		let p = path_points(a, b);
		assert_eq!(p.len(), 6); // n = ceil(4 / 0.65) = 7
		assert!(p.windows(2).all(|w| w[1][2] < w[0][2]) && p[0][2] < a[2] && p[5][2] > b[2]);
		let mut chain = vec![a];
		chain.extend(p.iter().copied());
		chain.push(b);
		assert!(chain.windows(2).all(|w| gap(w[0], w[1]) <= STEP + 1e-5));
	}

	#[test]
	fn the_trigger_edge_and_the_caps() {
		assert_eq!(path_points([0.0; 3], [0.0, 0.0, 0.71]).len(), 1); // just over the trigger: one midpoint
		assert!(path_points([0.0; 3], [0.0, 0.0, 10.4]).len() <= MAX_STEPS - 1);
		assert!(!path_points([0.0; 3], [0.0, 0.0, 10.3]).is_empty());
		assert!(path_points([0.0; 3], [0.0, 0.0, 10.5]).is_empty()); // a cut, not a path
		assert!(path_points([0.0; 3], [0.0, 0.0, 1e9]).is_empty());
		assert!(path_points([0.0; 3], [20.0, 0.0, 2.0]).is_empty()); // too far across: a respawn
		assert!(path_points([0.0; 3], [16.0, 0.0, 2.0]).len() == 3);
	}

	#[test]
	fn nan_and_infinite_positions_do_nothing() {
		for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
			assert!(path_points([0.0; 3], [0.0, 0.0, bad]).is_empty());
			assert!(path_points([0.0, bad, 0.0], [0.0, 0.0, 3.0]).is_empty());
			assert!(path_points([bad, 0.0, 0.0], [0.0, 0.0, 3.0]).is_empty());
			assert!(path_points([0.0, 0.0, bad], [0.0, 0.0, 3.0]).is_empty());
		}
	}

	#[test]
	fn the_clock_and_the_event_state_are_held_and_given_back() {
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		c.icount = 1000;
		(c.mem.now, c.mem.limit, c.mem.due, c.mem.tick, c.mem.irq_dirty) = (1000, 0, 1200, 1500, true);
		let held = Clock::hold(&mut c);
		// Nothing can fall due, nothing is pending: a block entry (`icount + n <= limit`) and the interpreter's checks pass.
		assert_eq!((c.mem.limit, c.mem.due, c.mem.tick, c.mem.irq_dirty), (u64::MAX, u64::MAX, u64::MAX, false));
		c.icount += 5_000_000; // the walk's instructions, far past the old deadlines
		c.mem.now = c.icount;
		held.release(&mut c);
		assert_eq!((c.icount, c.mem.now, c.mem.limit, c.mem.due, c.mem.tick, c.mem.irq_dirty), (1000, 1000, 0, 1200, 1500, true));
		// An idle machine (limit follows due) comes back the same way.
		(c.mem.limit, c.mem.due, c.mem.tick, c.mem.irq_dirty) = (900, 900, u64::MAX, false);
		let held = Clock::hold(&mut c);
		c.icount += 77;
		held.release(&mut c);
		assert_eq!((c.icount, c.mem.limit, c.mem.due, c.mem.tick, c.mem.irq_dirty), (1000, 900, 900, u64::MAX, false));
	}

	const D_STAT: u32 = 0x1000_e010;
	const D9: u32 = 0x1000_d400;

	/// A machine with a pending DMAC interrupt (INT1: CIS9 up, its mask bit on) that an enabled Status takes at the next step, nops at pc.
	fn machine_with_a_pending_interrupt() -> Cpu {
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		(c.pc, c.npc, c.cp0[12]) = (0x10_0000, 0x10_0004, 0x0001_0801); // IE, EIE, IM3 (INT1)
		c.mem.wr32(D_STAT, 1 << 25); // write-1-to-toggle: channel 9's mask bit on
		c.mem.set_hw32(D_STAT, c.mem.hw32(D_STAT) | 1 << 9); // CIS9
		c.mem.mark_dirty();
		assert_eq!(c.mem.irq_lines(), (false, true));
		c
	}

	#[test]
	fn a_walk_that_fills_a_slot_takes_no_interrupt_and_leaves_the_machine_as_it_was() {
		let lv = &LVS[1];
		let mut c = machine_with_a_pending_interrupt();
		let (keys, data) = (lv.res + 0x80, lv.res + 0xc0);
		c.mem.wr32(lv.res + 4, data);
		c.mem.wr32(lv.res + 8, keys);
		c.mem.wr32(0x20_0000, 0x1234_5678); // the joint-prefix source the kick copies into the scratchpad
		c.mem.spr.iter_mut().enumerate().for_each(|(i, b)| *b = (i % 253) as u8);
		let (status, cause) = (c.cp0[12], c.cp0[13]);
		let hw_before = c.mem.hw.clone();
		let (spr_before, icount) = (c.mem.spr.clone(), c.icount);
		let cache = MeshCache::save(&mut c, lv).expect("pointers in RAM");
		let (hw, clock) = (Hw::save(&c), Clock::hold(&mut c));
		assert_eq!(c.cp0[12] & IRQ_ENABLE, 0);
		// The probe fills slot 1 (key, data, index) and kicks SPR DMA channel 9 as retail's joint pose does.
		c.mem.wr32(keys + 8, 0xdead_beef);
		c.mem.wr8(data + 0x800, 0xaa);
		c.mem.wr32(D9 + 0x10, 0x20_0000);
		c.mem.wr32(D9 + 0x20, 1);
		c.mem.wr32(D9 + 0x80, 0x100);
		c.mem.wr32(D9, 0x100);
		assert_eq!(c.mem.spr[0x100..0x104], 0x1234_5678u32.to_le_bytes()); // the kick really moved data
		assert!(c.mem.irq_dirty && c.mem.limit == 0); // and marked the lines dirty, with a line up
		for _ in 0..4 {
			c.step(); // block entries inside the walk: the pending line must not be taken
		}
		assert_eq!((c.pc, c.irqs), (0x10_0010, [0, 0]));
		assert!(cache.changed(&c) && hw.changed(&c).0 > 0 && hw.changed(&c).1 > 0);
		cache.put_back(&mut c);
		hw.put_back(&mut c);
		clock.release(&mut c);
		assert!(!cache.changed(&c));
		assert_eq!((c.cp0[12], c.cp0[13], c.icount), (status, cause, icount)); // the 4 steps of the walk are not counted
		assert!(c.mem.hw == hw_before && c.mem.spr == spr_before);
		assert_eq!(hw.changed(&c), (0, 0));
		assert!(c.mem.irq_dirty && c.mem.limit_ok());
		// Without the walk the pending line is taken at the next step, now that Status allows it again.
		c.pc = 0x10_0000;
		c.npc = 0x10_0004;
		c.step();
		assert_eq!((c.irqs, c.cp0[14], c.pc & !0xff), ([0, 1], 0x10_0000, 0x8000_0200)); // INT1 taken from pc 0x100000 into the vector
	}

	#[test]
	fn the_mesh_cache_is_saved_and_put_back_slot_for_slot() {
		let lv = &LVS[1]; // Novalis: result block 0x174240, keys 0x1742c0, data 0x174300
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		let (keys, data) = (lv.res + 0x80, lv.res + 0xc0);
		c.mem.wr32(lv.res + 4, data);
		c.mem.wr32(lv.res + 8, keys);
		for i in 0..CACHE_KEYS / 4 {
			c.mem.wr32(keys + 4 * i, 0x1111_0000 + i);
		}
		for i in [0u32, 1, 0x7ff, 0x800, 0xfff, 0x2000, 0x3fff] {
			c.mem.wr8(data + i, (i % 251) as u8 + 1);
		}
		let cache = MeshCache::save(&mut c, lv).expect("pointers in RAM");
		assert!(!cache.changed(&c));
		c.mem.wr32(keys + 8, 0xdead_beef); // a probe fills a slot: key and data
		c.mem.wr8(data + 0x800, 0xaa);
		c.mem.wr8(data + 0x3fff, 0xbb);
		assert!(cache.changed(&c));
		cache.put_back(&mut c);
		assert!(!cache.changed(&c));
		assert_eq!(c.mem.rd32(keys + 8), 0x1111_0002);
		assert_eq!((c.mem.rd8(data + 0x800), c.mem.rd8(data + 0x3fff)), ((0x800 % 251) as u8 + 1, (0x3fff % 251) as u8 + 1));
	}

	#[test]
	fn a_cache_that_is_not_set_up_stops_the_walk() {
		let lv = &LVS[1];
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		assert!(MeshCache::save(&mut c, lv).is_none()); // both pointers 0 (before the level's init)
		c.mem.wr32(lv.res + 4, 0x174300);
		c.mem.wr32(lv.res + 8, 0x7000_1000); // keys in the scratchpad, not RAM
		assert!(MeshCache::save(&mut c, lv).is_none());
		c.mem.wr32(lv.res + 8, 0x01ff_fff0); // keys table running over the end
		assert!(MeshCache::save(&mut c, lv).is_none());
		c.mem.wr32(lv.res + 8, 0x1742c0);
		assert!(MeshCache::save(&mut c, lv).is_some());
	}

	#[test]
	fn the_probe_signature_is_found_once_or_not_at_all() {
		let mut words = vec![0x1234_5678u32; 200];
		assert_eq!(find_sig(&words), None);
		for &(i, w) in SIG.iter() {
			words[37 + i] = w;
		}
		assert_eq!(find_sig(&words), Some(37));
		let mut twice = words.clone();
		for &(i, w) in SIG.iter() {
			twice[120 + i] = w;
		}
		assert_eq!(find_sig(&twice), None); // ambiguous: do nothing
		words[37 + 29] ^= 1;
		assert_eq!(find_sig(&words), None);
		assert_eq!(find_sig(&[]), None);
		assert_eq!(find_sig(&words[..20]), None);
	}

	#[test]
	fn the_signature_sits_at_the_probe_start() {
		// Fixed words of FUN_0020e8f0 (index 0 = `addiu sp, sp, -0x60`, 7 = `li a0, 6`, 10 = `lh v1, 0x86(v0)`, 26 = `lui at, 0x3f40` = 0.75).
		let at = |i: usize| SIG.iter().find(|s| s.0 == i).map(|s| s.1);
		assert_eq!(at(0), Some(0x27bd_ffa0));
		assert_eq!(at(7), Some(0x2404_0006));
		assert_eq!(at(10), Some(0x8443_0086));
		assert_eq!(at(26), Some(0x3c01_3f40));
		assert!(SIG.windows(2).all(|w| w[0].0 < w[1].0));
	}
}
