//! [PC] FxHash-style hasher for lookup-only maps on hot paths (the interpolator's pairing maps, the props pose reuse).

use std::collections::HashMap;

/// [PC] FxHash-style hasher for the per-field pairing maps (`pair_ordered`, `still_kinds`): with std's SipHash, Pokitaru's pool
/// and waterfall quads cost ~2.3 ms of the main thread a field. Only lookups use it, never an iteration order.
#[derive(Default, Clone, Copy)]
pub struct FxHasher(u64);

impl FxHasher {
	#[inline]
	fn add(&mut self, x: u64) {
		self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x517c_c1b7_2722_0a95);
	}
}

impl std::hash::Hasher for FxHasher {
	fn finish(&self) -> u64 {
		self.0
	}
	fn write(&mut self, b: &[u8]) {
		for &x in b {
			self.add(x as u64);
		}
	}
	fn write_u8(&mut self, x: u8) {
		self.add(x as u64);
	}
	fn write_u16(&mut self, x: u16) {
		self.add(x as u64);
	}
	fn write_u32(&mut self, x: u32) {
		self.add(x as u64);
	}
	fn write_i32(&mut self, x: i32) {
		self.add(x as u32 as u64);
	}
	fn write_u64(&mut self, x: u64) {
		self.add(x);
	}
	fn write_usize(&mut self, x: usize) {
		self.add(x as u64);
	}
}

pub type FxMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<FxHasher>>;
