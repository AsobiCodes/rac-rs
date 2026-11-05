//! `host.raycast` (ABI 1.2): the game's own collision line test for the mods.
//!
//! Retail function: the segment query `seg_collide(start*, end*, mask, self, attack*)` (decomp/notes/collision_and_gameplay.md "Collision queries"):
//! Veldin overlay 0x1efbf0 (v1.00 PAL; 4336 bytes with padding), a per-level copy in every level's overlay (Novalis 0x211770, ...), found by its code
//! like a mod hook point (spec made with tools/make_spec.py: `--after func_L00_001EFF50:2 --len 4336`, all 19 levels give one masked hash).
//! It is NOT in `rac1_funcs.txt` (stage B's file, a merge conflict magnet): the spec is the game's own (`Game::seg_collide`, R&C1's in
//! rac1/omni_bind.rs), so only this file calls it.
//!
//! Arguments: `a0` / `a1` = pointers to 16-byte vectors (x, y, z floats in the game's metre units, w = 1.0), `a2` = mask, `a3` = the moby to skip,
//! `t0` = attack record (0 = no hit is queued for any moby). Mask 2 = terrain + moby meshes, no moby primitives: the mask of the hero's ground ray
//! (FUN_00232888 / OV:28100), so crates, bridges and doors are hit by their meshes; the skipped moby is the hero (`0x1413d0`). Returns nonzero on a hit.
//! Result record (level-relative data address, Veldin 0x173e40, mapped by `data_map` to the level that runs): +0x20 hit point (x, y, z), +0x40 normal
//! (unnormalised; normalised here). The call also advances the record counter at +0x10 (the stamp mobys keep so one query tests them once): that is
//! left advanced on purpose, because putting it back would let a stale stamp make the game's next query skip a moby. Everything else in the
//! 128-byte record block is put back, so a query between two fields leaves the game's own last hit as it was.
//! With no level (title screen, level loop ending), no machine in scope, native mods stale or a function not found: -1.

use super::{events, funcs, native_host};
use ee_oracle::cpu::Cpu;
use ee_oracle::hs_reloc::Reloc;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

/// Bytes of the record block that are saved and put back.
const RECORD_BYTES: usize = 0x80;
/// Offset of the query counter inside the record block (not put back).
const COUNTER: usize = 0x10;
/// The mask: terrain and moby meshes.
const MASK: u64 = 2;

pub fn reloc() -> &'static Reloc {
	static R: OnceLock<Reloc> = OnceLock::new();
	R.get_or_init(|| Reloc::parse(super::game::get().seg_collide).expect("the seg_collide spec parses"))
}

/// The entry found for a level word.
static FOUND: Mutex<Option<(i32, Option<u32>)>> = Mutex::new(None);

/// The entry of the line test in the code loaded now (searched once per level word, verified again on every use).
fn entry(c: &Cpu, level_word: i32) -> Option<u32> {
	let mut g = FOUND.lock().unwrap_or_else(|e| e.into_inner());
	if let Some((l, e)) = *g {
		if l == level_word && e.is_none_or(|e| reloc().matches(&c.mem, e)) {
			return e;
		}
	}
	let e = funcs::find_in(&funcs::code_words(c), funcs::CODE_LO, reloc());
	*g = Some((level_word, e));
	e
}

/// Records the answer of the level-load prescan (prescan.rs).
pub(super) fn seed(level_word: i32, entry: Option<u32>) {
	*FOUND.lock().unwrap_or_else(|e| e.into_inner()) = Some((level_word, entry));
}

/// Stack room for the two vectors: (new sp, start address, end address); the vectors lie above the callee's frames.
pub fn plan(sp: u64) -> Option<(u32, u32, u32)> {
	let (new, _) = native_host::stack_alloc(sp, 32)?;
	Some((new, new, new + 16))
}

/// A vector for the game: x, y, z, w = 1.0, little endian.
pub fn vec_bytes(p: [f32; 3]) -> [u8; 16] {
	let mut b = [0u8; 16];
	for (i, v) in [p[0], p[1], p[2], 1.0].iter().enumerate() {
		b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
	}
	b
}

/// The six floats handed to the mod: hit point, then the unit normal (a zero normal stays zero).
pub fn out6(hit: [f32; 3], n: [f32; 3]) -> [f32; 6] {
	let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
	let k = if len > 0.0 { 1.0 / len } else { 0.0 };
	[hit[0], hit[1], hit[2], n[0] * k, n[1] * k, n[2] * k]
}

pub fn f3(ram: &[u8], a: usize) -> [f32; 3] {
	let f = |o: usize| ram.get(a + o..a + o + 4).map_or(0.0, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
	[f(0), f(4), f(8)]
}

/// The line test on the machine: Some(None) no hit, Some(Some(six floats)) a hit, None cannot test.
pub fn cast(c: &mut Cpu, usa: bool, from: [f32; 3], to: [f32; 3]) -> Option<Option<[f32; 6]>> {
	let level = events::live_level(c)?;
	let entry = entry(c, native_host::level_word(c))?;
	let g = super::game::get();
	// The hit record: a reference-level data address of the game, mapped to the level that runs.
	let record = native_host::relocate_in(usa, level, g.seg_record)? as usize;
	let (new_sp, start, end) = plan(c.g(29))?;
	if record + RECORD_BYTES > c.mem.ram.len() {
		return None;
	}
	let saved: Vec<u8> = c.mem.ram[record..record + RECORD_BYTES].to_vec();
	c.mem.ram[start as usize..start as usize + 16].copy_from_slice(&vec_bytes(from));
	c.mem.ram[end as usize..end as usize + 16].copy_from_slice(&vec_bytes(to));
	// The hero moby (the hero cell's pointer): the moby the ray does not hit.
	let cell = g.hero_cell as usize;
	let hero = u32::from_le_bytes(c.mem.ram[cell..cell + 4].try_into().ok()?) as u64;
	let hit = native_host::run_at(
		c,
		entry,
		|c| {
			c.s(29, new_sp as u64);
			c.s(4, start as u64);
			c.s(5, end as u64);
			c.s(6, MASK);
			c.s(7, hero);
			c.s(8, 0);
		},
		|c| c.g(2) != 0,
	)
	.ok();
	let result = hit.map(|h| h.then(|| out6(f3(&c.mem.ram, record + 0x20), f3(&c.mem.ram, record + 0x40))));
	// Put the record back except the counter word.
	for i in (0..RECORD_BYTES).filter(|i| !(COUNTER..COUNTER + 4).contains(i)) {
		c.mem.ram[record + i] = saved[i];
	}
	result
}

unsafe extern "C" fn raycast_cb(user: *mut c_void, from3: *const f32, to3: *const f32, out6: *mut f32) -> i32 {
	if from3.is_null() || to3.is_null() || out6.is_null() || super::get().is_none() {
		return -1;
	}
	let Some(c) = super::host::cpu() else { return -1 };
	if c.stop.is_some() || native_host::stale() || native_host::depth_check().is_err() {
		return -1;
	}
	// SAFETY: the loader passes three floats for each point and room for six.
	let (from, to) = unsafe { ([*from3, *from3.add(1), *from3.add(2)], [*to3, *to3.add(1), *to3.add(2)]) };
	let r = cast(c, super::host::state(user).ntsc, from, to);
	super::counts::bump(match r {
		Some(Some(_)) => super::counts::Counter::RaycastHit,
		Some(None) => super::counts::Counter::RaycastMiss,
		None => super::counts::Counter::RaycastRefused,
	});
	match r {
		Some(Some(six)) => {
			// SAFETY: `out6` has room for six floats.
			unsafe { std::ptr::copy_nonoverlapping(six.as_ptr(), out6, 6) };
			1
		}
		Some(None) => 0,
		None => {
			native_host::say_once("raycast", "raycast: cannot test now (no level, or the line test is not in the code loaded)");
			-1
		}
	}
}

/// Fills the `raycast` entry of the host.
pub fn fill(h: &mut super::cabi::Host) {
	h.raycast = Some(raycast_cb);
}

