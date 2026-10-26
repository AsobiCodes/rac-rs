//! A zlib / deflate decoder (RFC 1950 / 1951, canonical Huffman as in zlib's `puff.c`) for the PNG replacement textures. Checksums
//! are not verified (the PNG chunk CRCs are not either): a damaged file gives a wrong picture or an error, never a crash.

struct Bits<'a> {
	data: &'a [u8],
	pos: usize,
	buf: u32,
	cnt: u32,
}

impl Bits<'_> {
	fn need(&mut self, n: u32) -> Result<u32, String> {
		while self.cnt < n {
			let b = *self.data.get(self.pos).ok_or("deflate: data ends early")?;
			self.pos += 1;
			self.buf |= (b as u32) << self.cnt;
			self.cnt += 8;
		}
		let v = self.buf & ((1u32 << n) - 1);
		self.buf = if n >= 32 { 0 } else { self.buf >> n };
		self.cnt -= n;
		Ok(v)
	}
}

/// A canonical Huffman code: how many codes of each length and the symbols in code order.
struct Huff {
	count: [u16; 16],
	symbol: Vec<u16>,
}

impl Huff {
	fn new(lengths: &[u8]) -> Huff {
		let mut count = [0u16; 16];
		for &l in lengths {
			count[l as usize] += 1;
		}
		let mut offs = [0u16; 16];
		for l in 1..15 {
			offs[l + 1] = offs[l] + count[l];
		}
		let mut symbol = vec![0u16; lengths.len()];
		for (s, &l) in lengths.iter().enumerate() {
			if l != 0 {
				symbol[offs[l as usize] as usize] = s as u16;
				offs[l as usize] += 1;
			}
		}
		Huff { count, symbol }
	}

	fn decode(&self, b: &mut Bits) -> Result<u16, String> {
		let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
		for len in 1..16 {
			code |= b.need(1)? as i32;
			let count = self.count[len] as i32;
			if code - count < first {
				return self.symbol.get((index + (code - first)) as usize).copied().ok_or_else(|| "deflate: bad code".to_string());
			}
			index += count;
			first += count;
			first <<= 1;
			code <<= 1;
		}
		Err("deflate: bad code".into())
	}
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

fn codes(b: &mut Bits, out: &mut Vec<u8>, lit: &Huff, dist: &Huff, limit: usize) -> Result<(), String> {
	loop {
		let s = lit.decode(b)? as usize;
		match s {
			0..=255 => out.push(s as u8),
			256 => return Ok(()),
			_ => {
				let s = s - 257;
				if s >= 29 {
					return Err("deflate: bad length".into());
				}
				let len = LEN_BASE[s] as usize + b.need(LEN_EXTRA[s] as u32)? as usize;
				let d = dist.decode(b)? as usize;
				if d >= 30 {
					return Err("deflate: bad distance".into());
				}
				let dist = DIST_BASE[d] as usize + b.need(DIST_EXTRA[d] as u32)? as usize;
				if dist > out.len() {
					return Err("deflate: distance too far back".into());
				}
				let from = out.len() - dist;
				for i in 0..len {
					out.push(out[from + i]);
				}
			}
		}
		if out.len() > limit {
			return Err("deflate: output larger than expected".into());
		}
	}
}

fn dynamic(b: &mut Bits) -> Result<(Huff, Huff), String> {
	const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
	let (nlen, ndist, ncode) = (b.need(5)? as usize + 257, b.need(5)? as usize + 1, b.need(4)? as usize + 4);
	if nlen > 286 || ndist > 30 {
		return Err("deflate: bad table sizes".into());
	}
	let mut lengths = [0u8; 320];
	for &o in ORDER.iter().take(ncode) {
		lengths[o] = b.need(3)? as u8;
	}
	let cl = Huff::new(&lengths[..19]);
	let mut lens = vec![0u8; nlen + ndist];
	let mut i = 0;
	while i < nlen + ndist {
		let sym = cl.decode(b)?;
		if sym < 16 {
			lens[i] = sym as u8;
			i += 1;
			continue;
		}
		let (prev, rep) = match sym {
			16 => (*lens.get(i.wrapping_sub(1)).ok_or("deflate: repeat without a length")?, 3 + b.need(2)? as usize),
			17 => (0, 3 + b.need(3)? as usize),
			_ => (0, 11 + b.need(7)? as usize),
		};
		if i + rep > nlen + ndist {
			return Err("deflate: too many lengths".into());
		}
		lens[i..i + rep].fill(prev);
		i += rep;
	}
	Ok((Huff::new(&lens[..nlen]), Huff::new(&lens[nlen..])))
}

fn fixed() -> (Huff, Huff) {
	let mut l = [8u8; 288];
	l[144..256].fill(9);
	l[256..280].fill(7);
	(Huff::new(&l), Huff::new(&[5u8; 30]))
}

/// Raw deflate data; `limit` bounds the output (a PNG knows its size).
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
	let mut b = Bits { data, pos: 0, buf: 0, cnt: 0 };
	let mut out = Vec::with_capacity(limit.min(1 << 26));
	loop {
		let last = b.need(1)?;
		match b.need(2)? {
			0 => {
				b.buf = 0;
				b.cnt = 0;
				let h = data.get(b.pos..b.pos + 4).ok_or("deflate: stored block ends early")?;
				let len = u16::from_le_bytes([h[0], h[1]]) as usize;
				let body = data.get(b.pos + 4..b.pos + 4 + len).ok_or("deflate: stored block ends early")?;
				out.extend_from_slice(body);
				b.pos += 4 + len;
			}
			1 => {
				let (l, d) = fixed();
				codes(&mut b, &mut out, &l, &d, limit)?;
			}
			2 => {
				let (l, d) = dynamic(&mut b)?;
				codes(&mut b, &mut out, &l, &d, limit)?;
			}
			_ => return Err("deflate: bad block type".into()),
		}
		if last == 1 {
			return Ok(out);
		}
		if out.len() > limit {
			return Err("deflate: output larger than expected".into());
		}
	}
}

/// A zlib stream (2 byte header, deflate data, adler32 ignored).
pub fn zlib(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
	if data.len() < 2 || data[0] & 0x0f != 8 || (u16::from_be_bytes([data[0], data[1]]) % 31) != 0 || data[1] & 0x20 != 0 {
		return Err("zlib: bad header".into());
	}
	inflate(&data[2..], limit)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn stored_and_fixed_blocks() {
		// zlib.compress(b"hello hello hello hello", 0) and level 9 (fixed Huffman with a back reference).
		let stored = [0x78, 0x01, 0x01, 0x05, 0x00, 0xfa, 0xff, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x06, 0x2c, 0x02, 0x15];
		assert_eq!(zlib(&stored, 100).unwrap(), b"hello");
		let fixed = [0x78, 0xda, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01, 0x68, 0x03, 0x08, 0xb1];
		assert_eq!(zlib(&fixed, 100).unwrap(), b"hello hello hello hello");
	}

	#[test]
	fn dynamic_block() {
		// zlib.compress(skewed pseudo-random text of 3000 bytes, 9): a dynamic Huffman block.
		let data = include_bytes!("inflate_dyn.bin");
		let want = include_bytes!("inflate_dyn.want");
		let out = zlib(data, 5000).unwrap();
		assert_eq!(out, &want[..]);
	}

	#[test]
	fn damaged_data_is_an_error() {
		assert!(zlib(&[0x78, 0x9c, 0xff, 0xff], 100).is_err());
		assert!(zlib(&[0, 0], 100).is_err());
	}
}
