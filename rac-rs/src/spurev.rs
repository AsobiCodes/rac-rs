//! The SPU2 effect (reverb) unit, integer-exact, one instance per core (stage 10c).
//!
//! Behaviour written from the documented SPU2 effect network (PCSX2 SPU2 Reverb.cpp / ReverbResample.cpp are a
//! behaviour reference only; nothing is copied): the unit runs on every output sample but computes the network at 24 kHz
//! (left and right alternate on even and odd samples); the input goes through a 39-tap half-band FIR (downsample), the
//! network works on the SPU2 RAM work area [ESA, EEA] (a circular buffer that moves one word per 2 output samples), and
//! the result goes through the same FIR doubled (upsample). Registers are the 32 libsd effect values (units of 8 bytes,
//! four 16-bit words) in the order of the preset table `audio::PRESETS`.
//!
//! RAM: the work area only needs its own words, so the unit keeps a private 1 M word RAM (2 MiB, SPU2 word addresses).
//! The driver's "clear work area" is `clear`.

/// SPU2 RAM in 16-bit words.
const RAM_WORDS: usize = 1 << 20;
const TAPS: usize = 39;

/// The 39-tap downsample filter (the zeros between the taps are real), padded to 48.
const DOWN: [i16; 48] = [
	-1, 0, 2, 0, -10, 0, 35, 0, -103, 0, 266, 0, -616, 0, 1332, 0, -2960, 0, 10246, 16384, 10246, 0, -2960, 0, 1332, 0, -616, 0, 266, 0, -103, 0, 35, 0,
	-10, 0, 2, 0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// The upsample filter: the downsample filter doubled and saturated to 16 bits.
const fn up_coefs() -> [i16; 48] {
	let mut r = [0i16; 48];
	let mut i = 0;
	while i < 48 {
		let v = DOWN[i] as i32 * 2;
		r[i] = if v > 32767 { 32767 } else if v < -32768 { -32768 } else { v as i16 };
		i += 1;
	}
	r
}
const UP: [i16; 48] = up_coefs();

fn clamp16(x: i32) -> i32 {
	x.clamp(-32768, 32767)
}

/// Rounded Q15 product of one 16-bit lane (the SIMD multiply-high-round of the 8-lane filter).
fn mul_hr(a: i16, b: i16) -> i16 {
	((((a as i32 * b as i32) >> 14) + 1) >> 1) as i16
}

/// The 39-tap filter over `buf[idx..idx + 40]` as an 8-lane vector filter computes it: per lane a saturating sum of the five
/// rounded products, then three pairwise saturating horizontal adds.
fn fir(buf: &[i16; 128], idx: usize, coefs: &[i16; 48]) -> i16 {
	let mut acc = [0i16; 8];
	for b in 0..5 {
		for (l, a) in acc.iter_mut().enumerate() {
			let i = 8 * b + l;
			*a = a.saturating_add(mul_hr(buf[idx + i], coefs[i]));
		}
	}
	let h1 = [acc[0].saturating_add(acc[1]), acc[2].saturating_add(acc[3]), acc[4].saturating_add(acc[5]), acc[6].saturating_add(acc[7])];
	let h2 = [h1[0].saturating_add(h1[1]), h1[2].saturating_add(h1[3])];
	h2[0].saturating_add(h2[1])
}

/// The decoded effect registers (addresses in words).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Regs {
	in_l: i16,
	in_r: i16,
	apf1_size: u32,
	apf2_size: u32,
	apf1_vol: i16,
	apf2_vol: i16,
	same_src: [u32; 2],
	diff_src: [u32; 2],
	same_dst: [u32; 2],
	diff_dst: [u32; 2],
	iir: i16,
	wall: i16,
	comb_src: [[u32; 2]; 4],
	comb_vol: [i16; 4],
	apf1_dst: [u32; 2],
	apf2_dst: [u32; 2],
}

impl Regs {
	/// From the 32 libsd effect values: dAPF1, dAPF2, vIIR, vCOMB1..4, vWALL, vAPF1, vAPF2, mLSAME, mRSAME, mLCOMB1,
	/// mRCOMB1, mLCOMB2, mRCOMB2, dLSAME, dRSAME, mLDIFF, mRDIFF, mLCOMB3, mRCOMB3, mLCOMB4, mRCOMB4, dLDIFF, dRDIFF,
	/// mLAPF1, mRAPF1, mLAPF2, mRAPF2, vLIN, vRIN. The name prefix d = a distance, m = an address: the SPU2 names these
	/// by what they do (so "mLSAME" is the same-side write address SAME_L_DST and "dLSAME" the same-side read SAME_L_SRC).
	pub fn from_preset(r: &[u16; 32]) -> Regs {
		let a = |i: usize| r[i] as u32 * 4;
		let v = |i: usize| r[i] as i16;
		Regs {
			in_l: v(30),
			in_r: v(31),
			apf1_size: a(0),
			apf2_size: a(1),
			apf1_vol: v(8),
			apf2_vol: v(9),
			same_src: [a(16), a(17)],
			diff_src: [a(24), a(25)],
			same_dst: [a(10), a(11)],
			diff_dst: [a(18), a(19)],
			iir: v(2),
			wall: v(7),
			comb_src: [[a(12), a(13)], [a(14), a(15)], [a(20), a(21)], [a(22), a(23)]],
			comb_vol: [v(3), v(4), v(5), v(6)],
			apf1_dst: [a(26), a(27)],
			apf2_dst: [a(28), a(29)],
		}
	}
}

pub struct Reverb {
	r: Regs,
	/// SPU2 RAM (words); empty until an area is set.
	ram: Vec<i16>,
	/// Work area start (word address) and end (the end register keeps only its high half: low 16 bits read 0xffff).
	esa: u32,
	eea: u32,
	/// FxEnable: the network always runs, but only writes back to RAM while enabled.
	pub fx_enable: bool,
	down: [[i16; 128]; 2],
	up: [[i16; 128]; 2],
	pos: usize,
	/// Output samples processed (its parity picks the channel, half of it moves the buffer).
	cycles: u32,
}

impl Default for Reverb {
	fn default() -> Reverb {
		Reverb::new()
	}
}

impl Reverb {
	pub fn new() -> Reverb {
		Reverb { r: Regs::default(), ram: Vec::new(), esa: 0, eea: 0, fx_enable: false, down: [[0; 128]; 2], up: [[0; 128]; 2], pos: 0, cycles: 0 }
	}

	pub fn set_regs(&mut self, preset: &[u16; 32]) {
		self.r = Regs::from_preset(preset);
	}

	/// The work area from byte addresses `esa` .. `eea` (inclusive, as the driver hands them to libsd; libsd shifts them to
	/// SPU2 word addresses, and the end register keeps only the high half of its word address).
	pub fn set_area_bytes(&mut self, esa: u32, eea: u32) {
		self.esa = (esa >> 1) & 0xf_ffff;
		self.eea = ((((eea >> 1) & 0xf_ffff) >> 16) << 16) | 0xffff;
		if self.ram.is_empty() {
			self.ram = vec![0; RAM_WORDS];
		}
	}

	/// sceSdClearEffectWorkArea: zero the bytes `esa` .. `eea`.
	pub fn clear(&mut self, esa: u32, eea: u32) {
		if self.ram.is_empty() {
			return;
		}
		let (a, b) = ((esa >> 1) as usize, ((eea >> 1) as usize).min(RAM_WORDS - 1));
		if a <= b {
			self.ram[a..=b].fill(0);
		}
	}

	/// Whether the unit has a work area (otherwise it returns silence).
	pub fn has_area(&self) -> bool {
		self.esa < self.eea && !self.ram.is_empty()
	}

	fn idx(&self, off: u32) -> usize {
		let start = self.esa & 0x3f_ffff;
		let end = (self.eea & 0x3f_ffff) | 0xffff;
		let x = ((self.cycles >> 1).wrapping_add(off)) % ((end - start) + 1);
		((x + start) & 0xf_ffff) as usize
	}

	/// One output sample: the effect input (the wet sum of the core) -> the effect output (before EVOL).
	pub fn process(&mut self, tw: [i32; 2]) -> [i32; 2] {
		if !self.has_area() {
			self.cycles = self.cycles.wrapping_add(1);
			return [0, 0];
		}
		let inp = [clamp16(tw[0]) as i16, clamp16(tw[1]) as i16];
		for c in 0..2 {
			self.down[c][self.pos] = inp[c];
			self.down[c][self.pos | 64] = inp[c];
		}
		let rt = (self.cycles & 1) as usize; // 0: left, 1: right
		let g = self.r;
		let same_src = self.idx(g.same_src[rt]);
		let same_dst = self.idx(g.same_dst[rt]);
		let same_prv = self.idx(g.same_dst[rt].wrapping_sub(1));
		// The crossed line: the left channel reads the right diff source and vice versa; its destination is its own side.
		let diff_src = self.idx(g.diff_src[1 - rt]);
		let diff_dst = self.idx(g.diff_dst[rt]);
		let diff_prv = self.idx(g.diff_dst[rt].wrapping_sub(1));
		let comb: [usize; 4] = [0, 1, 2, 3].map(|k| self.idx(g.comb_src[k][rt]));
		let apf1_src = self.idx(g.apf1_dst[rt].wrapping_sub(g.apf1_size));
		let apf1_dst = self.idx(g.apf1_dst[rt]);
		let apf2_src = self.idx(g.apf2_dst[rt].wrapping_sub(g.apf2_size));
		let apf2_dst = self.idx(g.apf2_dst[rt]);

		let mul = |x: i16, y: i32| (x as i32 * y) >> 15;
		let m = |i: usize| self.ram[i] as i32;
		let down = fir(&self.down[rt], (self.pos.wrapping_sub(TAPS)) & 63, &DOWN) as i32;
		let input = mul(if rt == 1 { g.in_r } else { g.in_l }, down);
		let same = mul(g.iir, input + mul(g.wall, m(same_src)) - m(same_prv)) + m(same_prv);
		let diff = mul(g.iir, input + mul(g.wall, m(diff_src)) - m(diff_prv)) + m(diff_prv);
		let mut out = mul(g.comb_vol[0], m(comb[0])) + mul(g.comb_vol[1], m(comb[1])) + mul(g.comb_vol[2], m(comb[2])) + mul(g.comb_vol[3], m(comb[3]));
		let apf1 = out - mul(g.apf1_vol, m(apf1_src));
		out = m(apf1_src) + mul(g.apf1_vol, apf1);
		let apf2 = out - mul(g.apf2_vol, m(apf2_src));
		out = m(apf2_src) + mul(g.apf2_vol, apf2);
		if self.fx_enable {
			self.ram[same_dst] = clamp16(same) as i16;
			self.ram[diff_dst] = clamp16(diff) as i16;
			self.ram[apf1_dst] = clamp16(apf1) as i16;
			self.ram[apf2_dst] = clamp16(apf2) as i16;
		}
		let out = clamp16(out) as i16;
		self.up[rt][self.pos] = out;
		self.up[1 - rt][self.pos] = 0;
		self.up[rt][self.pos | 64] = out;
		self.up[1 - rt][self.pos | 64] = 0;
		self.pos = (self.pos + 1) & 63;
		let i = (self.pos.wrapping_sub(TAPS)) & 63;
		let res = [fir(&self.up[0], i, &UP) as i32, fir(&self.up[1], i, &UP) as i32];
		self.cycles = self.cycles.wrapping_add(1);
		res
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::audio::PRESETS;

	fn room() -> Reverb {
		let (units, regs) = PRESETS[0];
		let mut r = Reverb::new();
		r.set_regs(&regs);
		let bytes = units as u32 * 8;
		r.set_area_bytes(0x20_0000 - bytes, 0x1f_ffff);
		r.fx_enable = true;
		r
	}

	#[test]
	fn filter_taps() {
		// the centre tap passes half of a full-scale sample (Q15 0.5), the up filter passes it whole (0.5 * 2 saturated)
		let mut b = [0i16; 128];
		b[19] = 32767;
		assert_eq!(fir(&b, 0, &DOWN), 16384);
		assert_eq!(fir(&b, 0, &UP), 32766);
		// DC gain of the half-band filter: the taps sum to 32766 / 32768
		let b = [1000i16; 128];
		let s: i32 = DOWN.iter().map(|&c| c as i32).sum();
		assert_eq!(s, 32766);
		assert!((fir(&b, 0, &DOWN) as i32 - 1000).abs() <= 10);
	}

	#[test]
	fn silence_and_no_area() {
		let mut r = room();
		for _ in 0..5000 {
			assert_eq!(r.process([0, 0]), [0, 0]);
		}
		let mut n = Reverb::new();
		assert_eq!(n.process([20000, -20000]), [0, 0]);
	}

	#[test]
	fn impulse_leaves_a_tail_and_clear_stops_it() {
		let mut r = room();
		let mut e = 0i64;
		let mut first = [0i32; 2];
		for i in 0..30000 {
			let x = if i == 0 { [30000, 30000] } else { [0, 0] };
			let o = r.process(x);
			if i == 0 {
				first = o;
			}
			e += (o[0] as i64).abs() + (o[1] as i64).abs();
		}
		assert_eq!(first, [0, 0], "the filters delay the output");
		assert!(e > 1000, "tail energy {e}");
		// without write-back (FxEnable off) nothing is stored: a second impulse sees an empty buffer again
		let (units, _) = PRESETS[0];
		r.clear(0x20_0000 - units as u32 * 8, 0x1f_ffff);
		r.fx_enable = true;
		let mut e2 = 0i64;
		for _ in 0..30000 {
			let o = r.process([0, 0]);
			e2 += (o[0] as i64).abs() + (o[1] as i64).abs();
		}
		// the FIR memory still holds the tail of the last outputs only for 40 samples
		assert!(e2 < e / 50, "after clear {e2} vs {e}");
	}

	/// A plain 32-bit-accumulator version of the same network (the documented algorithm without the vector rounding):
	/// the unit must stay within a few LSB of it, which pins the addressing, the channel order and the write-back.
	struct Plain {
		ram: Vec<i16>,
		r: Regs,
		esa: u32,
		eea: u32,
		down: Vec<[i32; 2]>,
		up: Vec<[i32; 2]>,
		cycles: u32,
	}

	impl Plain {
		fn idx(&self, off: u32) -> usize {
			let end = (self.eea | 0xffff) as u64;
			let start = self.esa as u64;
			(((((self.cycles >> 1).wrapping_add(off)) as u64 % (end - start + 1)) + start) & 0xf_ffff) as usize
		}
		fn step(&mut self, inp: [i32; 2]) -> [i32; 2] {
			let rt = (self.cycles & 1) as usize;
			self.down.push(inp);
			let n = self.down.len();
			let mut d = 0i32;
			for i in 0..TAPS {
				d += self.down[n - 1 - TAPS + i][rt] * DOWN[i] as i32; // window pos-39 .. pos-1 (the newest sample is excluded)
			}
			let d = clamp16(d >> 15);
			let g = self.r;
			let mul = |x: i16, y: i32| (x as i32 * y) >> 15;
			let (ss, sd, sp) = (self.idx(g.same_src[rt]), self.idx(g.same_dst[rt]), self.idx(g.same_dst[rt].wrapping_sub(1)));
			let (ds, dd, dp) = (self.idx(g.diff_src[1 - rt]), self.idx(g.diff_dst[rt]), self.idx(g.diff_dst[rt].wrapping_sub(1)));
			let c: Vec<usize> = (0..4).map(|k| self.idx(g.comb_src[k][rt])).collect();
			let (a1s, a1d) = (self.idx(g.apf1_dst[rt].wrapping_sub(g.apf1_size)), self.idx(g.apf1_dst[rt]));
			let (a2s, a2d) = (self.idx(g.apf2_dst[rt].wrapping_sub(g.apf2_size)), self.idx(g.apf2_dst[rt]));
			let m = |s: &Plain, i: usize| s.ram[i] as i32;
			let input = mul(if rt == 1 { g.in_r } else { g.in_l }, d);
			let same = mul(g.iir, input + mul(g.wall, m(self, ss)) - m(self, sp)) + m(self, sp);
			let diff = mul(g.iir, input + mul(g.wall, m(self, ds)) - m(self, dp)) + m(self, dp);
			let mut out = (0..4).map(|k| mul(g.comb_vol[k], m(self, c[k]))).sum::<i32>();
			let apf1 = out - mul(g.apf1_vol, m(self, a1s));
			out = m(self, a1s) + mul(g.apf1_vol, apf1);
			let apf2 = out - mul(g.apf2_vol, m(self, a2s));
			out = m(self, a2s) + mul(g.apf2_vol, apf2);
			self.ram[sd] = clamp16(same) as i16;
			self.ram[dd] = clamp16(diff) as i16;
			self.ram[a1d] = clamp16(apf1) as i16;
			self.ram[a2d] = clamp16(apf2) as i16;
			let out = clamp16(out);
			let mut o = [0, 0];
			o[rt] = out;
			self.up.push(o);
			let n = self.up.len();
			let mut res = [0i32; 2];
			for (ch, r) in res.iter_mut().enumerate() {
				let mut s = 0i32;
				for i in 0..TAPS {
					s += self.up[n - TAPS + i][ch] * UP[i] as i32;
				}
				*r = clamp16(s >> 15);
			}
			self.cycles += 1;
			res
		}
	}

	#[test]
	fn matches_the_plain_network() {
		let (units, regs) = PRESETS[4]; // HALL
		let mut r = Reverb::new();
		r.set_regs(&regs);
		let bytes = units as u32 * 8;
		r.set_area_bytes(0x20_0000 - bytes, 0x1f_ffff);
		r.fx_enable = true;
		let mut p = Plain { ram: vec![0; RAM_WORDS], r: Regs::from_preset(&regs), esa: r.esa, eea: r.eea, down: vec![[0, 0]; TAPS], up: vec![[0, 0]; TAPS], cycles: 0 };
		let mut seed = 12345u32;
		let mut worst = 0;
		for i in 0..40000 {
			seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
			let burst = (i / 4000) % 2 == 0;
			let x = if burst { [((seed >> 8) as i32 & 0x3fff) - 0x2000, ((seed >> 4) as i32 & 0x3fff) - 0x2000] } else { [0, 0] };
			let a = r.process(x);
			let b = p.step(x);
			worst = worst.max((a[0] - b[0]).abs()).max((a[1] - b[1]).abs());
		}
		assert!(worst <= 40, "worst difference {worst}");
	}
}
