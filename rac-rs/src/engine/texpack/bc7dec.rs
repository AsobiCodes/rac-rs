//! BC7 (BPTC) block decoder, written from the format description (Direct3D 11 functional spec "BC7 Format", Khronos Data Format
//! Specification chapter "BPTC", Microsoft Learn "BC7 Format"); no library code was copied. The partition and anchor tables are the
//! ones the specs print. Checked bit for bit against `texconv` (DirectXTex 2026.5.8.1, decode to R8G8B8A8_UNORM): see the
//! `bc7dec_tests.rs` fixtures and the lane's HANDOFF.md for the totals.
//!
//! Format in short: a 4x4 block is 128 bits. The mode is the number of zero bits before the first set bit (0..7); a first byte of 0 is
//! a reserved block and decodes to (0, 0, 0, 0) for all 16 texels. Modes 0, 1, 2, 3 and 7 split the block into 2 or 3 subsets by one of
//! 64 (16 for mode 0) partitions, each subset has two endpoints and its own index range; modes 4, 5 have one subset, a channel rotation
//! (R, G or B swapped with A) and, mode 4 only, a choice of which index set (2 bit or 3 bit) the colour uses; mode 6 has one subset and
//! 4 bit indices. Endpoints are stored with 4..8 bits per channel, an optional p-bit (modes 0, 1 per subset, 3, 6, 7 per endpoint) is
//! appended as the lowest bit, and the value is widened to 8 bits by repeating its top bits. Texels are
//! `((64 - w) * e0 + w * e1 + 32) >> 6` with the weight `w` from the 2, 3 or 4 bit table. The first texel of every subset (the anchor)
//! stores its index without the top bit. Modes 0..3 have no alpha (alpha 255).
//!
//! PCSX2's DDS loader (`GSTextureReplacements`) takes a DXGI BC7 file as it is and ignores the DX10 alpha mode; `dds::load` follows that
//! (`bc7alpha`): an opaque file stays BC7 for the GPU, anything with alpha is decoded here and its PS2-scale alpha is doubled. This
//! decoder also serves a device without BC compression (`ReplTex::to_rgba8`) and the grass colour reader.

/// Interpolation weights for 2, 3 and 4 bit indices.
const W2: [u32; 4] = [0, 21, 43, 64];
const W3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const W4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

/// Partition of each texel (row by row) for the 64 two-subset partitions.
#[rustfmt::skip]
const PART2: [[u8; 16]; 64] = [
	[0,0,1,1,0,0,1,1,0,0,1,1,0,0,1,1], [0,0,0,1,0,0,0,1,0,0,0,1,0,0,0,1], [0,1,1,1,0,1,1,1,0,1,1,1,0,1,1,1], [0,0,0,1,0,0,1,1,0,0,1,1,0,1,1,1],
	[0,0,0,0,0,0,0,1,0,0,0,1,0,0,1,1], [0,0,1,1,0,1,1,1,0,1,1,1,1,1,1,1], [0,0,0,1,0,0,1,1,0,1,1,1,1,1,1,1], [0,0,0,0,0,0,0,1,0,0,1,1,0,1,1,1],
	[0,0,0,0,0,0,0,0,0,0,0,1,0,0,1,1], [0,0,1,1,0,1,1,1,1,1,1,1,1,1,1,1], [0,0,0,0,0,0,0,1,0,1,1,1,1,1,1,1], [0,0,0,0,0,0,0,0,0,0,0,1,0,1,1,1],
	[0,0,0,1,0,1,1,1,1,1,1,1,1,1,1,1], [0,0,0,0,0,0,0,0,1,1,1,1,1,1,1,1], [0,0,0,0,1,1,1,1,1,1,1,1,1,1,1,1], [0,0,0,0,0,0,0,0,0,0,0,0,1,1,1,1],
	[0,0,0,0,1,0,0,0,1,1,1,0,1,1,1,1], [0,1,1,1,0,0,0,1,0,0,0,0,0,0,0,0], [0,0,0,0,0,0,0,0,1,0,0,0,1,1,1,0], [0,1,1,1,0,0,1,1,0,0,0,1,0,0,0,0],
	[0,0,1,1,0,0,0,1,0,0,0,0,0,0,0,0], [0,0,0,0,1,0,0,0,1,1,0,0,1,1,1,0], [0,0,0,0,0,0,0,0,1,0,0,0,1,1,0,0], [0,1,1,1,0,0,1,1,0,0,1,1,0,0,0,1],
	[0,0,1,1,0,0,0,1,0,0,0,1,0,0,0,0], [0,0,0,0,1,0,0,0,1,0,0,0,1,1,0,0], [0,1,1,0,0,1,1,0,0,1,1,0,0,1,1,0], [0,0,1,1,0,1,1,0,0,1,1,0,1,1,0,0],
	[0,0,0,1,0,1,1,1,1,1,1,0,1,0,0,0], [0,0,0,0,1,1,1,1,1,1,1,1,0,0,0,0], [0,1,1,1,0,0,0,1,1,0,0,0,1,1,1,0], [0,0,1,1,1,0,0,1,1,0,0,1,1,1,0,0],
	[0,1,0,1,0,1,0,1,0,1,0,1,0,1,0,1], [0,0,0,0,1,1,1,1,0,0,0,0,1,1,1,1], [0,1,0,1,1,0,1,0,0,1,0,1,1,0,1,0], [0,0,1,1,0,0,1,1,1,1,0,0,1,1,0,0],
	[0,0,1,1,1,1,0,0,0,0,1,1,1,1,0,0], [0,1,0,1,0,1,0,1,1,0,1,0,1,0,1,0], [0,1,1,0,1,0,0,1,0,1,1,0,1,0,0,1], [0,1,0,1,1,0,1,0,1,0,1,0,0,1,0,1],
	[0,1,1,1,0,0,1,1,1,1,0,0,1,1,1,0], [0,0,0,1,0,0,1,1,1,1,0,0,1,0,0,0], [0,0,1,1,0,0,1,0,0,1,0,0,1,1,0,0], [0,0,1,1,1,0,1,1,1,1,0,1,1,1,0,0],
	[0,1,1,0,1,0,0,1,1,0,0,1,0,1,1,0], [0,0,1,1,1,1,0,0,1,1,0,0,0,0,1,1], [0,1,1,0,0,1,1,0,1,0,0,1,1,0,0,1], [0,0,0,0,0,1,1,0,0,1,1,0,0,0,0,0],
	[0,1,0,0,1,1,1,0,0,1,0,0,0,0,0,0], [0,0,1,0,0,1,1,1,0,0,1,0,0,0,0,0], [0,0,0,0,0,0,1,0,0,1,1,1,0,0,1,0], [0,0,0,0,0,1,0,0,1,1,1,0,0,1,0,0],
	[0,1,1,0,1,1,0,0,1,0,0,1,0,0,1,1], [0,0,1,1,0,1,1,0,1,1,0,0,1,0,0,1], [0,1,1,0,0,0,1,1,1,0,0,1,1,1,0,0], [0,0,1,1,1,0,0,1,1,1,0,0,0,1,1,0],
	[0,1,1,0,1,1,0,0,1,1,0,0,1,0,0,1], [0,1,1,0,0,0,1,1,0,0,1,1,1,0,0,1], [0,1,1,1,1,1,1,0,1,0,0,0,0,0,0,1], [0,0,0,1,1,0,0,0,1,1,1,0,0,1,1,1],
	[0,0,0,0,1,1,1,1,0,0,1,1,0,0,1,1], [0,0,1,1,0,0,1,1,1,1,1,1,0,0,0,0], [0,0,1,0,0,0,1,0,1,1,1,0,1,1,1,0], [0,1,0,0,0,1,0,0,0,1,1,1,0,1,1,1],
];

/// Partition of each texel for the 64 three-subset partitions (mode 0 uses the first 16, mode 2 all 64).
#[rustfmt::skip]
const PART3: [[u8; 16]; 64] = [
	[0,0,1,1,0,0,1,1,0,2,2,1,2,2,2,2], [0,0,0,1,0,0,1,1,2,2,1,1,2,2,2,1], [0,0,0,0,2,0,0,1,2,2,1,1,2,2,1,1], [0,2,2,2,0,0,2,2,0,0,1,1,0,1,1,1],
	[0,0,0,0,0,0,0,0,1,1,2,2,1,1,2,2], [0,0,1,1,0,0,1,1,0,0,2,2,0,0,2,2], [0,0,2,2,0,0,2,2,1,1,1,1,1,1,1,1], [0,0,1,1,0,0,1,1,2,2,1,1,2,2,1,1],
	[0,0,0,0,0,0,0,0,1,1,1,1,2,2,2,2], [0,0,0,0,1,1,1,1,1,1,1,1,2,2,2,2], [0,0,0,0,1,1,1,1,2,2,2,2,2,2,2,2], [0,0,1,2,0,0,1,2,0,0,1,2,0,0,1,2],
	[0,1,1,2,0,1,1,2,0,1,1,2,0,1,1,2], [0,1,2,2,0,1,2,2,0,1,2,2,0,1,2,2], [0,0,1,1,0,1,1,2,1,1,2,2,1,2,2,2], [0,0,1,1,2,0,0,1,2,2,0,0,2,2,2,0],
	[0,0,0,1,0,0,1,1,0,1,1,2,1,1,2,2], [0,1,1,1,0,0,1,1,2,0,0,1,2,2,0,0], [0,0,0,0,1,1,2,2,1,1,2,2,1,1,2,2], [0,0,2,2,0,0,2,2,0,0,2,2,1,1,1,1],
	[0,1,1,1,0,1,1,1,0,2,2,2,0,2,2,2], [0,0,0,1,0,0,0,1,2,2,2,1,2,2,2,1], [0,0,0,0,0,0,1,1,0,1,2,2,0,1,2,2], [0,0,0,0,1,1,0,0,2,2,1,0,2,2,1,0],
	[0,1,2,2,0,1,2,2,0,0,1,1,0,0,0,0], [0,0,1,2,0,0,1,2,1,1,2,2,2,2,2,2], [0,1,1,0,1,2,2,1,1,2,2,1,0,1,1,0], [0,0,0,0,0,1,1,0,1,2,2,1,1,2,2,1],
	[0,0,2,2,1,1,0,2,1,1,0,2,0,0,2,2], [0,1,1,0,0,1,1,0,2,0,0,2,2,2,2,2], [0,0,1,1,0,1,2,2,0,1,2,2,0,0,1,1], [0,0,0,0,2,0,0,0,2,2,1,1,2,2,2,1],
	[0,0,0,0,0,0,0,2,1,1,2,2,1,2,2,2], [0,2,2,2,0,0,2,2,0,0,1,2,0,0,1,1], [0,0,1,1,0,0,1,2,0,0,2,2,0,2,2,2], [0,1,2,0,0,1,2,0,0,1,2,0,0,1,2,0],
	[0,0,0,0,1,1,1,1,2,2,2,2,0,0,0,0], [0,1,2,0,1,2,0,1,2,0,1,2,0,1,2,0], [0,1,2,0,2,0,1,2,1,2,0,1,0,1,2,0], [0,0,1,1,2,2,0,0,1,1,2,2,0,0,1,1],
	[0,0,1,1,1,1,2,2,2,2,0,0,0,0,1,1], [0,1,0,1,0,1,0,1,2,2,2,2,2,2,2,2], [0,0,0,0,0,0,0,0,2,1,2,1,2,1,2,1], [0,0,2,2,1,1,2,2,0,0,2,2,1,1,2,2],
	[0,0,2,2,0,0,1,1,0,0,2,2,0,0,1,1], [0,2,2,0,1,2,2,1,0,2,2,0,1,2,2,1], [0,1,0,1,2,2,2,2,2,2,2,2,0,1,0,1], [0,0,0,0,2,1,2,1,2,1,2,1,2,1,2,1],
	[0,1,0,1,0,1,0,1,0,1,0,1,2,2,2,2], [0,2,2,2,0,1,1,1,0,2,2,2,0,1,1,1], [0,0,0,2,1,1,1,2,0,0,0,2,1,1,1,2], [0,0,0,0,2,1,1,2,2,1,1,2,2,1,1,2],
	[0,2,2,2,0,1,1,1,0,1,1,1,0,2,2,2], [0,0,0,2,1,1,1,2,1,1,1,2,0,0,0,2], [0,1,1,0,0,1,1,0,0,1,1,0,2,2,2,2], [0,0,0,0,0,0,0,0,2,1,1,2,2,1,1,2],
	[0,1,1,0,0,1,1,0,2,2,2,2,2,2,2,2], [0,0,2,2,0,0,1,1,0,0,1,1,0,0,2,2], [0,0,2,2,1,1,2,2,1,1,2,2,0,0,2,2], [0,0,0,0,0,0,0,0,0,0,0,0,2,1,1,2],
	[0,0,0,2,0,0,0,1,0,0,0,2,0,0,0,1], [0,2,2,2,1,2,2,2,0,2,2,2,1,2,2,2], [0,1,0,1,2,2,2,2,2,2,2,2,2,2,2,2], [0,1,1,1,2,0,1,1,2,2,0,1,2,2,2,0],
];

/// Anchor texel of subset 1 (two-subset partitions).
#[rustfmt::skip]
const ANCHOR2: [u8; 64] = [
	15,15,15,15,15,15,15,15, 15,15,15,15,15,15,15,15, 15, 2, 8, 2, 2, 8, 8,15, 2, 8, 2, 2, 8, 8, 2, 2,
	15,15, 6, 8, 2, 8,15,15,  2, 8, 2, 2, 2,15,15, 6,  6, 2, 6, 8,15,15, 2, 2, 15,15,15,15,15, 2, 2,15,
];
/// Anchor texel of subset 1 and subset 2 (three-subset partitions).
#[rustfmt::skip]
const ANCHOR3_1: [u8; 64] = [
	 3, 3,15,15, 8, 3,15,15, 8, 8, 6, 6, 6, 5, 3, 3,  3, 3, 8,15, 3, 3, 6,10, 5, 8, 8, 6, 8, 5,15,15,
	 8,15, 3, 5, 6,10, 8,15, 15, 3,15, 5,15,15,15,15,  3,15, 5, 5, 5, 8, 5,10, 5,10, 8,13,15,12, 3, 3,
];
#[rustfmt::skip]
const ANCHOR3_2: [u8; 64] = [
	15, 8, 8, 3,15,15, 3, 8, 15,15,15,15,15,15,15, 8, 15, 8,15, 3,15, 8,15, 8, 3,15, 6,10,15,15,10, 8,
	15, 3,15,10,10, 8, 9,10,  6,15, 8,15, 3, 6, 6, 8, 15, 3,15,15,15,15,15,15, 15,15,15,15, 3,15,15, 8,
];

/// Layout of one mode.
struct Mode {
	/// Subsets (1, 2, 3) and partition-number bits.
	subsets: usize,
	part_bits: u32,
	/// Bits of a colour channel and of alpha per endpoint, before the p-bit (alpha 0 = no alpha channel).
	cb: u32,
	ab: u32,
	/// 0 no p-bits, 1 one per endpoint, 2 one per subset (shared by its two endpoints).
	pbits: u8,
	/// Bits of the colour index and (modes 4 and 5, which have a second index set) of the alpha index.
	ib: u32,
	ib2: u32,
}

const MODES: [Mode; 8] = [
	Mode { subsets: 3, part_bits: 4, cb: 4, ab: 0, pbits: 1, ib: 3, ib2: 0 },
	Mode { subsets: 2, part_bits: 6, cb: 6, ab: 0, pbits: 2, ib: 3, ib2: 0 },
	Mode { subsets: 3, part_bits: 6, cb: 5, ab: 0, pbits: 0, ib: 2, ib2: 0 },
	Mode { subsets: 2, part_bits: 6, cb: 7, ab: 0, pbits: 1, ib: 2, ib2: 0 },
	Mode { subsets: 1, part_bits: 0, cb: 5, ab: 6, pbits: 0, ib: 2, ib2: 3 },
	Mode { subsets: 1, part_bits: 0, cb: 7, ab: 8, pbits: 0, ib: 2, ib2: 2 },
	Mode { subsets: 1, part_bits: 0, cb: 7, ab: 7, pbits: 1, ib: 4, ib2: 0 },
	Mode { subsets: 2, part_bits: 6, cb: 5, ab: 5, pbits: 1, ib: 2, ib2: 0 },
];

/// Reads fields of a block from the low bit up.
struct Bits {
	v: u128,
	at: u32,
}

impl Bits {
	#[inline(always)]
	fn take(&mut self, n: u32) -> u32 {
		let r = ((self.v >> self.at) & ((1u128 << n) - 1)) as u32;
		self.at += n;
		r
	}
}

/// An `n` bit value (with its p-bit already appended) widened to 8 bits by repeating its top bits.
#[inline(always)]
fn widen(v: u32, n: u32) -> u32 {
	if n >= 8 { v } else { (v << (8 - n)) | (v >> (2 * n - 8)) }
}

/// A block's header fields and unquantised endpoints (endpoint `e` is end `e % 2` of subset `e / 2`); `r` stands at the first index bit.
struct Header {
	m: &'static Mode,
	part: usize,
	rot: u32,
	sel: u32,
	ep: [[u32; 4]; 6],
	r: Bits,
}

/// Parses everything before the indices; `mode` is the block's mode (0..7).
fn header(blk: &[u8; 16], mode: usize) -> Header {
	let m = &MODES[mode];
	let mut r = Bits { v: u128::from_le_bytes(*blk), at: mode as u32 + 1 };
	let part = r.take(m.part_bits) as usize;
	let rot = if mode == 4 || mode == 5 { r.take(2) } else { 0 };
	let sel = if mode == 4 { r.take(1) } else { 0 };
	// Endpoints: all red, then green, blue, alpha.
	let ne = m.subsets * 2;
	let mut ep = [[0u32; 4]; 6];
	for ch in 0..3 {
		for e in ep.iter_mut().take(ne) {
			e[ch] = r.take(m.cb);
		}
	}
	if m.ab > 0 {
		for e in ep.iter_mut().take(ne) {
			e[3] = r.take(m.ab);
		}
	}
	let mut p = [0u32; 6];
	match m.pbits {
		1 => p.iter_mut().take(ne).for_each(|x| *x = r.take(1)),
		2 => {
			for s in 0..m.subsets {
				let b = r.take(1);
				(p[2 * s], p[2 * s + 1]) = (b, b);
			}
		}
		_ => {}
	}
	let extra = (m.pbits != 0) as u32;
	for (e, pb) in ep.iter_mut().zip(p).take(ne) {
		for c in e.iter_mut().take(3) {
			*c = widen((*c << extra) | pb * extra, m.cb + extra);
		}
		e[3] = if m.ab > 0 { widen((e[3] << extra) | pb * extra, m.ab + extra) } else { 255 };
	}
	Header { m, part, rot, sel, ep, r }
}

fn weights(bits: u32) -> &'static [u32] {
	match bits {
		2 => &W2,
		3 => &W3,
		_ => &W4,
	}
}

/// Decodes one 16 byte block into 16 RGBA8 texels (row by row, 64 bytes).
pub fn decode_block(blk: &[u8; 16], out: &mut [u8; 64]) {
	let mode = blk[0].trailing_zeros() as usize;
	if mode >= 8 {
		*out = [0; 64];
		return;
	}
	let Header { m, part, rot, sel, ep, mut r } = header(blk, mode);
	// Indices, texel by texel; the anchor texel of each subset has its top bit left out.
	let (part_tab, a1, a2) = match m.subsets {
		2 => (&PART2[part], ANCHOR2[part] as usize, 16),
		3 => (&PART3[part], ANCHOR3_1[part] as usize, ANCHOR3_2[part] as usize),
		_ => (&PART2[0], 16, 16),
	};
	let one = m.subsets == 1;
	let read_idx = |r: &mut Bits, i: usize, bits: u32| -> u32 {
		let s = if one { 0 } else { part_tab[i] as usize };
		let anchor = i == 0 || (s == 1 && i == a1) || (s == 2 && i == a2);
		r.take(bits - anchor as u32)
	};
	let mut ci = [0u32; 16];
	let mut ai = [0u32; 16];
	for (i, x) in ci.iter_mut().enumerate() {
		*x = read_idx(&mut r, i, m.ib);
	}
	if m.ib2 > 0 {
		for (i, x) in ai.iter_mut().enumerate() {
			*x = read_idx(&mut r, i, m.ib2);
		}
	}
	// Mode 4: index set 0 (read first) is the 2 bit one and `sel` swaps which set the colour and which the alpha uses.
	// Modes 4 and 5 have two sets (`ci`, `ai`); every other mode uses `ci` for both.
	let two = m.ib2 > 0;
	let swap = two && sel == 1;
	let (wtc, wta) = if !two { (weights(m.ib), weights(m.ib)) } else if swap { (weights(m.ib2), weights(m.ib)) } else { (weights(m.ib), weights(m.ib2)) };
	for i in 0..16 {
		let s = if one { 0 } else { part_tab[i] as usize };
		let (e0, e1) = (&ep[2 * s], &ep[2 * s + 1]);
		let (ic, ia) = if !two { (ci[i], ci[i]) } else if swap { (ai[i], ci[i]) } else { (ci[i], ai[i]) };
		let (wc, wa) = (wtc[ic as usize], wta[ia as usize]);
		let mut px = [0u8; 4];
		for c in 0..3 {
			px[c] = (((64 - wc) * e0[c] + wc * e1[c] + 32) >> 6) as u8;
		}
		px[3] = (((64 - wa) * e0[3] + wa * e1[3] + 32) >> 6) as u8;
		if rot != 0 {
			px.swap(3, rot as usize - 1);
		}
		out[4 * i..4 * i + 4].copy_from_slice(&px);
	}
}

/// Whether all 16 texels of the block have alpha 255 (visible or not). Modes 0..3 and blocks whose alpha endpoints are all 255 (without
/// rotation, which would move a colour into alpha) answer from the header; the rest are decoded.
pub fn block_is_opaque(blk: &[u8; 16]) -> bool {
	let mode = blk[0].trailing_zeros() as usize;
	if mode >= 8 {
		return false;
	}
	if mode < 4 {
		return true;
	}
	let h = header(blk, mode);
	if h.rot == 0 && h.ep.iter().take(h.m.subsets * 2).all(|e| e[3] == 255) {
		return true;
	}
	let mut px = [0u8; 64];
	decode_block(blk, &mut px);
	px.chunks_exact(4).all(|p| p[3] == 255)
}

fn blocks_of(w: u32, h: u32) -> (usize, usize) {
	(w.div_ceil(4) as usize, h.div_ceil(4) as usize)
}

/// Decodes a whole `w` x `h` level to RGBA8 (rows top to bottom; blocks on the right / bottom edge are cut to the size). The data must
/// hold every block, else Err.
pub fn decode_level(w: u32, h: u32, data: &[u8]) -> Result<Vec<u8>, String> {
	let (bw, bh) = blocks_of(w, h);
	if data.len() < bw * bh * 16 {
		return Err(format!("BC7 level {w}x{h} needs {} bytes, the file has {}", bw * bh * 16, data.len()));
	}
	let stride = w as usize * 4;
	let mut out = vec![0u8; stride * h as usize];
	let mut px = [0u8; 64];
	for (n, b) in data.chunks_exact(16).take(bw * bh).enumerate() {
		decode_block(b.try_into().unwrap(), &mut px);
		let (x0, y0) = ((n % bw) * 4, (n / bw) * 4);
		let cols = (w as usize - x0).min(4) * 4;
		for row in 0..(h as usize - y0).min(4) {
			out[(y0 + row) * stride + x0 * 4..][..cols].copy_from_slice(&px[row * 16..][..cols]);
		}
	}
	Ok(out)
}

/// Whether every visible texel of a `w` x `h` level decodes to alpha 255 (`block_is_opaque` answers most blocks without decoding them).
/// Stops at the first texel that is not opaque; Err if the data is short.
pub fn level_is_opaque(w: u32, h: u32, data: &[u8]) -> Result<bool, String> {
	let (bw, bh) = blocks_of(w, h);
	if data.len() < bw * bh * 16 {
		return Err(format!("BC7 level {w}x{h} needs {} bytes, the file has {}", bw * bh * 16, data.len()));
	}
	let mut px = [0u8; 64];
	for (n, b) in data.chunks_exact(16).take(bw * bh).enumerate() {
		let b: &[u8; 16] = b.try_into().unwrap();
		if block_is_opaque(b) {
			continue;
		}
		decode_block(b, &mut px);
		let (x0, y0) = ((n % bw) * 4, (n / bw) * 4);
		let (cols, rows) = ((w as usize - x0).min(4), (h as usize - y0).min(4));
		if (0..rows).any(|y| (0..cols).any(|x| px[(y * 4 + x) * 4 + 3] != 255)) {
			return Ok(false);
		}
	}
	Ok(true)
}

#[cfg(test)]
#[path = "bc7dec_tests.rs"]
mod tests;
