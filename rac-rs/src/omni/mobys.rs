//! Moby queries for mods (ABI 1.3 `moby_query` / `moby_valid`; `omni.mobys{..}`, `omni.moby_valid(addr)`): which objects of the level
//! are alive, filtered by class, distance and "is an enemy", nearest first. Only reads game RAM, runs no game code.
//!
//! Driven ONLY by the binding's `mobys` section (binding.rs `MobysDef`): no address or offset of the game is in this file. The pool cells and the
//! target registry are overlay-window addresses (window.rs), translated to the level loaded now. The walk (research PERK_SYSTEM_A.md section 2):
//! read `start` and `end` of the pool from the cells, step by `stride`, stop at the slot whose state byte is `tail` (the game's own loop stops
//! there; the slots behind it are zero memory), skip slots whose state is `dead_from` or more (deleted). With the registry flag the zero-terminated
//! pointer array of the target registry is walked instead (what the scheduler admitted this frame: near and not culled), each entry checked the same way.
//!
//! A result is valid until the end of the tick (the game reuses a deleted slot 2 ticks after the delete); `moby_valid` re-checks one address.

use super::binding::MobysDef;
use super::cabi::OmniMobyQuery;
use super::host::{cpu, state};
use std::ffi::c_void;

/// `OmniMobyQuery.flags`
pub const Q_NEAR: u32 = 1;
pub const Q_ENEMY: u32 = 2;
pub const Q_REGISTRY: u32 = 4;
/// `max` 0 means this many; the hard cap and the class list cap of the ABI.
const DEFAULT_MAX: usize = 32;
const HARD_MAX: usize = 256;
const MAX_CLASSES: usize = 16;

/// Why there is no answer: the level's data is not mapped (title screen, loading) or the pool cells do not describe a pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoPool {
	NoLevel,
	Unusable,
}

/// The pool of the level loaded now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pool {
	pub start: u32,
	pub pool_start: u32,
	pub end: u32,
	/// The address of the first slot whose state is the tail marker, or `end`: nothing at or behind it exists.
	pub tail: u32,
}

/// Translates a binding (window) address with `len` bytes behind it to a real RAM address.
pub type Translate<'a> = &'a dyn Fn(u32, u32) -> Option<u32>;

fn u8_at(ram: &[u8], a: u32) -> Option<u8> {
	ram.get(a as usize).copied()
}
fn u16_at(ram: &[u8], a: u32) -> Option<u16> {
	let a = a as usize;
	Some(u16::from_le_bytes(ram.get(a..a.checked_add(2)?)?.try_into().ok()?))
}
fn u32_at(ram: &[u8], a: u32) -> Option<u32> {
	let a = a as usize;
	Some(u32::from_le_bytes(ram.get(a..a.checked_add(4)?)?.try_into().ok()?))
}
fn f32_at(ram: &[u8], a: u32) -> Option<f32> {
	u32_at(ram, a).map(f32::from_bits)
}

/// Reads the pool cells and finds the tail marker.
pub fn pool(def: &MobysDef, ram: &[u8], tr: Translate) -> Result<Pool, NoPool> {
	let len = def.start_word.max(def.pool_start_word).max(def.end_word) + 4;
	let cells = tr(def.cells, len).ok_or(NoPool::NoLevel)?;
	let word = |w: u32| u32_at(ram, cells.checked_add(w)?);
	let (Some(start), Some(pool_start), Some(end)) = (word(def.start_word), word(def.pool_start_word), word(def.end_word)) else { return Err(NoPool::Unusable) };
	let bytes = end.wrapping_sub(start);
	if start >= end || bytes % def.stride != 0 || bytes / def.stride > def.max_slots || end as usize > ram.len() || start % 4 != 0 {
		return Err(NoPool::Unusable);
	}
	let mut tail = end;
	let mut a = start;
	while a < end {
		if u8_at(ram, a + def.state) == Some(def.tail as u8) {
			tail = a;
			break;
		}
		a += def.stride;
	}
	Ok(Pool { start, pool_start, end, tail })
}

impl Pool {
	/// A live moby: a slot start inside the pool before the tail marker whose state is not a deleted one.
	pub fn is_live(&self, def: &MobysDef, ram: &[u8], a: u32) -> bool {
		a >= self.start && a < self.tail && (a - self.start) % def.stride == 0 && u8_at(ram, a + def.state).is_some_and(|s| u32::from(s) < def.dead_from)
	}
}

/// The retail "is a living enemy" test: in the target registry (flag), of the enemy class type, with a target block whose health is above 0.
fn is_enemy(def: &MobysDef, ram: &[u8], a: u32) -> bool {
	let Some(flags) = u16_at(ram, a + def.flags).map(u32::from) else { return false };
	if flags & def.registered_flag == 0 || flags & def.has_component_flag == 0 {
		return false;
	}
	let Some(class_ptr) = u32_at(ram, a + def.class_ptr) else { return false };
	if u16_at(ram, class_ptr.wrapping_add(def.class_type)).map(|t| i32::from(t as i16)) != Some(def.enemy_type) {
		return false;
	}
	let comp = u32_at(ram, a + def.pvar).and_then(|pvar| u32_at(ram, pvar)).unwrap_or(0);
	comp != 0 && f32_at(ram, comp).is_some_and(|h| h > 0.0)
}

/// Whether `a` passes the query's filters; the squared distance to `near` when there is one. Cheapest test first: class, distance, enemy.
fn passes(def: &MobysDef, ram: &[u8], q: &OmniMobyQuery, classes: &[i32], a: u32) -> Option<f32> {
	if !classes.is_empty() {
		let class = i32::from(u16_at(ram, a + def.class)? as i16);
		if !classes.contains(&class) {
			return None;
		}
	}
	let mut d2 = 0.0f32;
	if q.flags & Q_NEAR != 0 {
		let p = [f32_at(ram, a + def.pos)?, f32_at(ram, a + def.pos + 4)?, f32_at(ram, a + def.pos + 8)?];
		if !p.iter().all(|v| v.is_finite()) {
			return None;
		}
		d2 = (0..3).map(|i| (p[i] - q.near[i]) * (p[i] - q.near[i])).sum();
		if q.radius > 0.0 && d2 > q.radius * q.radius {
			return None;
		}
	}
	if q.flags & Q_ENEMY != 0 && !is_enemy(def, ram, a) {
		return None;
	}
	Some(d2)
}

/// The addresses a query asks for: the whole pool in slot order, or the registry in its order, as far as the pool is alive.
fn candidates(def: &MobysDef, ram: &[u8], pool: &Pool, tr: Translate, registry: bool) -> Result<Vec<u32>, NoPool> {
	if !registry {
		let mut v = Vec::new();
		let mut a = pool.start;
		while a < pool.tail {
			if u8_at(ram, a + def.state).is_some_and(|s| u32::from(s) < def.dead_from) {
				v.push(a);
			}
			a += def.stride;
		}
		return Ok(v);
	}
	let at = tr(def.registry, 4).ok_or(NoPool::NoLevel)?;
	let mut v = Vec::new();
	for i in 0..def.max_slots {
		match u32_at(ram, at.wrapping_add(4 * i)) {
			Some(a) if a != 0 => {
				if pool.is_live(def, ram, a) {
					v.push(a);
				}
			}
			_ => break,
		}
	}
	Ok(v)
}

/// The live mobies matching `q`: nearest first when `Q_NEAR`, else in walk order; at most `q.max` (0 = 32, cap 256).
pub fn query(def: &MobysDef, ram: &[u8], tr: Translate, q: &OmniMobyQuery) -> Result<Vec<u32>, NoPool> {
	let pool = pool(def, ram, tr)?;
	let max = match q.max as usize {
		0 => DEFAULT_MAX,
		m => m.min(HARD_MAX),
	};
	let classes = &q.classes[..(q.n_classes as usize).min(MAX_CLASSES)];
	let near = q.flags & Q_NEAR != 0;
	let mut hits: Vec<(f32, u32)> = Vec::new();
	for a in candidates(def, ram, &pool, tr, q.flags & Q_REGISTRY != 0)? {
		if let Some(d2) = passes(def, ram, q, classes, a) {
			hits.push((d2, a));
			if !near && hits.len() >= max {
				break;
			}
		}
	}
	if near {
		let key = |x: &(f32, u32), y: &(f32, u32)| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1));
		if hits.len() > max {
			hits.select_nth_unstable_by(max, key);
			hits.truncate(max);
		}
		hits.sort_unstable_by(key);
	}
	Ok(hits.into_iter().map(|h| h.1).collect())
}

/// `moby_query` on a RAM image: addresses written to `out` (as many as fit), the count; -1 when no level is loaded.
pub fn run_query(def: &MobysDef, ram: &[u8], tr: Translate, q: &OmniMobyQuery, out: &mut [u32]) -> i32 {
	match query(def, ram, tr, q) {
		Ok(v) => {
			let n = v.len().min(out.len());
			out[..n].copy_from_slice(&v[..n]);
			n as i32
		}
		Err(NoPool::NoLevel) => -1,
		Err(NoPool::Unusable) => 0,
	}
}

/// `moby_valid` on a RAM image: 1 for a live moby of the pool, else 0.
pub fn run_valid(def: &MobysDef, ram: &[u8], tr: Translate, addr: u32) -> i32 {
	i32::from(pool(def, ram, tr).is_ok_and(|p| p.is_live(def, ram, addr)))
}

fn translator(user: *mut c_void, ram: &[u8]) -> impl Fn(u32, u32) -> Option<u32> {
	let level_word = ram.get(LEVEL_WORD..LEVEL_WORD + 4).map_or(-1, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
	let usa = state(user).ntsc;
	move |addr, len| super::window::translate(usa, level_word, addr, len)
}

/// A lookup `addr -> class id` for the live mobys of the pool now (None for anything else, or without a pool): for stores that must notice a slot
/// being reused (pose_follow.rs). Walks the pool once to build, then each lookup is O(1).
pub(super) fn class_lookup<'a>(st: &'a super::host::HostState, ram: &'a [u8]) -> impl Fn(u32) -> Option<u16> + 'a {
	let level_word = ram.get(LEVEL_WORD..LEVEL_WORD + 4).map_or(-1, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
	let def = st.binding.mobys.as_ref();
	let pool = def.and_then(|d| pool(d, ram, &|a, l| super::window::translate(st.ntsc, level_word, a, l)).ok());
	move |a| {
		let (d, p) = (def?, pool.as_ref()?);
		if p.is_live(d, ram, a) {
			u16_at(ram, a + d.class)
		} else {
			None
		}
	}
}

/// The level number word of the machine (`oracle LEVEL`), the same as native_host.rs reads.
pub const LEVEL_WORD: usize = 0x15_ED84;

/// `OmniHost::moby_query`.
pub unsafe extern "C" fn moby_query_cb(user: *mut c_void, q: *const OmniMobyQuery, out: *mut u32, cap: u32) -> i32 {
	let (Some(c), Some(def)) = (cpu(), state(user).binding.mobys.as_ref()) else { return -1 };
	if q.is_null() || out.is_null() || cap == 0 {
		return 0;
	}
	// SAFETY: the loader passes one OmniMobyQuery and room for `cap` addresses.
	let (q, out) = unsafe { (*q, std::slice::from_raw_parts_mut(out, cap as usize)) };
	run_query(def, &c.mem.ram, &translator(user, &c.mem.ram), &q, out)
}

/// `OmniHost::moby_valid`.
pub unsafe extern "C" fn moby_valid_cb(user: *mut c_void, addr: u32) -> i32 {
	let (Some(c), Some(def)) = (cpu(), state(user).binding.mobys.as_ref()) else { return 0 };
	run_valid(def, &c.mem.ram, &translator(user, &c.mem.ram), addr)
}

/// Fills the two ABI 1.3 entries when the binding has a `mobys` section (NULL = not supported).
pub fn fill(h: &mut super::cabi::Host, binding: &super::binding::Binding) {
	if binding.mobys.is_some() {
		h.moby_query = Some(moby_query_cb);
		h.moby_valid = Some(moby_valid_cb);
	}
}
