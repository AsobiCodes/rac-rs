//! What the Modern camera's line tests (`seg_raw`, retail's segment test `seg_collide`) leave behind on the mobys, and the test
//! switch that measures it.
//!
//! [C] seg_collide stamps every moby that passes its broadphase with the query id (`moby + 0x9c`, the id is `++DAT_00173e50`) and
//! tests a moby only when the old stamp differs from the id (overlay.elf.c seg_collide: `*(int *)(moby + 0x9c) = id` next to the
//! `old != id` test). `seg_raw` puts the result block back (it holds the id counter) but until now not the stamps: after a
//! camera probe the counter is back at N while the probed mobys hold N + 1, so retail's NEXT query takes id N + 1 and silently
//! skips every moby the probe touched. The Shoulder probes (`shoulder::probes`, five segments a field right beside Ratchet) use
//! `seg_clean`, which puts the stamps back too: retail then sees exactly what it would without them.
//!
//! RAC_BOOM_STAMP_FIX (TEST ONLY, temporary, default 0 = every other camera test as before, i.e. today's default RAM): bit 0 puts
//! the stamps back after EVERY `seg_mask` line test of the camera (boom, ceiling, drop / open-space rays, crosshair cast: the
//! effect on retail is what is measured); bit 1 (value 2 or 3) compares all 32 MB of RAM before and after each of the first
//! `DIAG_CALLS` line tests and prints one `stamp-diff` line to stderr for each one that left a trace.
use super::*;
use std::sync::{atomic::AtomicU32, OnceLock};

/// Line tests the diagnostic (bit 1 of the switch, value 2) checks: a 32 MB copy and compare each.
const DIAG_CALLS: u32 = 4000;

/// A moby's touch index (+0xa4) and query stamp (+0x9c) before a line test.
pub(super) type Saved = Vec<(u8, u32)>;

/// The switch's bits from the env text (anything not a number is 0).
fn bits(v: Option<&str>) -> u32 {
	v.and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

fn switch() -> u32 {
	static S: OnceLock<u32> = OnceLock::new();
	*S.get_or_init(|| bits(std::env::var("RAC_BOOM_STAMP_FIX").ok().as_deref()))
}

pub(super) fn save(c: &mut Cpu, ms: &[u32]) -> Saved {
	ms.iter().map(|&m| (c.mem.rd8(m + 0xa4), c.mem.rd32(m + 0x9c))).collect()
}

/// Puts the touch indexes back, and the query stamps when `stamps`.
pub(super) fn restore(c: &mut Cpu, ms: &[u32], saved: &Saved, stamps: bool) {
	for (&m, &(touch, stamp)) in ms.iter().zip(saved) {
		c.mem.wr8(m + 0xa4, touch);
		if stamps {
			c.mem.wr32(m + 0x9c, stamp);
		}
	}
}

/// The camera's line test with the line-test mask as a parameter; the stamps stay as the test left them unless the test switch
/// (bit 0) says to put them back.
pub(super) fn seg_mask(c: &mut Cpu, lv: &Lv, a: [f32; 3], b: [f32; 3], mask: u32) -> Option<[f32; 3]> {
	run(c, lv, a, b, mask, switch() & 1 != 0)
}

/// The camera's line test (mask 0xb4) that leaves no trace on the mobys at all (the Shoulder probes).
pub(super) fn seg_clean(c: &mut Cpu, lv: &Lv, a: [f32; 3], b: [f32; 3]) -> Option<[f32; 3]> {
	run(c, lv, a, b, 0xb4, true)
}

fn run(c: &mut Cpu, lv: &Lv, a: [f32; 3], b: [f32; 3], mask: u32, keep: bool) -> Option<[f32; 3]> {
	static CALLS: AtomicU32 = AtomicU32::new(0);
	let n = CALLS.load(Ordering::Relaxed);
	if switch() & 2 == 0 || n >= DIAG_CALLS {
		return seg_raw(c, lv, a, b, mask, keep);
	}
	CALLS.store(n + 1, Ordering::Relaxed);
	let before = c.mem.ram.to_vec();
	let hit = seg_raw(c, lv, a, b, mask, keep);
	report(n, mask, keep, &before, &c.mem.ram);
	hit
}

/// Up to `RANGES` differing byte ranges (start, length; bytes closer than 8 are one range) and the total of differing bytes.
fn diff(before: &[u8], after: &[u8]) -> (u32, Vec<(usize, usize)>) {
	const RANGES: usize = 6;
	let (mut total, mut ranges) = (0u32, Vec::<(usize, usize)>::new());
	for (i, (x, y)) in before.chunks(0x1000).zip(after.chunks(0x1000)).enumerate() {
		if x == y {
			continue;
		}
		for j in x.iter().zip(y).enumerate().filter(|(_, (p, q))| p != q).map(|(j, _)| j) {
			let at = i * 0x1000 + j;
			total += 1;
			if ranges.last().is_some_and(|r| at < r.0 + r.1 + 8) {
				let r = ranges.last_mut().unwrap();
				r.1 = at + 1 - r.0;
			} else if ranges.len() < RANGES {
				ranges.push((at, 1));
			}
		}
	}
	(total, ranges)
}

fn report(n: u32, mask: u32, keep: bool, before: &[u8], after: &[u8]) {
	let (total, ranges) = diff(before, after);
	if total > 0 {
		let r: Vec<String> = ranges.iter().map(|&(s, l)| format!("{s:#x}+{l}")).collect();
		eprintln!("stamp-diff call={n} mask={mask:#x} kept={} bytes={total} ranges={}", keep as u8, r.join(","));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_switch_reads_plain_numbers_only() {
		assert_eq!([bits(None), bits(Some("")), bits(Some("x")), bits(Some("1")), bits(Some(" 3 "))], [0, 0, 0, 1, 3]);
	}

	#[test]
	fn the_diff_lists_ranges_and_counts_bytes() {
		let mut a = vec![0u8; 0x3000];
		let b = a.clone();
		assert_eq!(diff(&a, &b), (0, vec![]));
		a[0x10] = 1;
		a[0x1004] = 7;
		a[0x1006] = 7;
		a[0x2fff] = 9;
		let (n, r) = diff(&a, &b);
		assert_eq!((n, r), (4, vec![(0x10, 1), (0x1004, 3), (0x2fff, 1)]));
	}
}
