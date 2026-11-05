//! The games' C library rand() (engine split stage E2, 2026-10-26: moved out of rac1/particles.rs so the formats and the
//! engine need not reach into the R&C1 pack; `particles::{Rng, RAND_SEED}` still work, re-exported there).

/// The game's rand() (newlib FUN_001160d8, decomp/notes/title_assets.md): s = s * 1103515245 + 12345, returns
/// s & 0x7fffffff; the game seeds it 12345 once at boot and every subsystem draws from that one sequence. `Rng` is
/// a handle on that shared state, one per thread (the game thread's is the game's; each test starts from the
/// seed). Draws match the game's order only where every consumer of the tick is ported [I].
#[derive(Clone, Copy, Debug, Default)]
pub struct Rng;

/// The game's rand() seed.
pub const RAND_SEED: u32 = 12345;

thread_local! {
	static RAND: std::cell::Cell<u32> = const { std::cell::Cell::new(RAND_SEED) };
}

impl Rng {
	/// The shared generator.
	pub fn shared() -> Rng {
		Rng
	}
	/// FUN_001160c8: sets the state word. Level entry calls it with `LEVEL_SEED` (decomp/notes/particles.md RNG).
	pub fn reseed(&mut self, seed: u32) {
		RAND.with(|s| s.set(seed));
	}
	/// rand(): 0..0x7fffffff.
	pub fn next(&mut self) -> u32 {
		RAND.with(|s| {
			let v = s.get().wrapping_mul(1_103_515_245).wrapping_add(12345);
			s.set(v);
			v & 0x7fff_ffff
		})
	}
	/// The state word without a draw (PC debug aid: the viewer bench's determinism hash).
	pub fn state(&self) -> u32 {
		RAND.with(|s| s.get())
	}
	/// High-word draw h = (r >> 16) & 0x7fff, one rand().
	fn high(&mut self) -> u32 {
		(self.next() >> 16) & 0x7fff
	}
	/// FUN_00257760(n) / FUN_00212d58: h % n; the draw is consumed even for n == 0.
	pub fn small(&mut self, n: u32) -> u32 {
		let h = self.high();
		if n == 0 { 0 } else { h % n }
	}
	/// FUN_002577f8(a, b): a + float(h) * (b - a) * (1/32768), one draw.
	pub fn float(&mut self, a: f32, b: f32) -> f32 {
		let h = self.high() as f32;
		a + h * (b - a) * (1.0 / 32768.0)
	}
	/// FUN_002577a0(a, b): a + h % (b - a + 1), one draw even for an empty range.
	pub fn int(&mut self, a: i32, b: i32) -> i32 {
		let h = self.high();
		let span = b.wrapping_sub(a).wrapping_add(1);
		if span == 0 { a } else { a.wrapping_add((h as i32).wrapping_rem(span)) }
	}
	/// FUN_00257848(lo, hi): one draw; magnitude lo + float(q) * (hi - lo) / 4096 from the high word's low 12 bits,
	/// negated when high-word bit 0 is set (magnitude and sign are correlated).
	pub fn signed(&mut self, lo: f32, hi: f32) -> f32 {
		let r = self.next() >> 16;
		let a = lo + (r & 4095) as f32 * (hi - lo) / 4096.0;
		if r & 1 != 0 { -a } else { a }
	}
	/// FUN_00257858(0, m): symmetric in (-m, m), one draw.
	pub fn sym(&mut self, m: f32) -> f32 {
		self.signed(0.0, m)
	}
	/// FUN_0026c628: float(q - 2048) * pi / 2048 for q = high-word low 12 bits, one draw.
	pub fn angle(&mut self) -> f32 {
		let q = ((self.next() >> 16) & 4095) as i32;
		(q - 2048) as f32 * std::f32::consts::PI / 2048.0
	}
	/// FUN_0026c678(lo, hi): exactly three draws, angle, angle, float(lo, hi); a point at that radius.
	pub fn unit_vec_len(&mut self, lo: f32, hi: f32) -> [f32; 3] {
		let a = self.angle();
		let b = self.angle();
		let r = self.float(lo, hi);
		[a.cos() * r * b.cos(), a.sin() * r * b.cos(), b.sin() * r]
	}
}
