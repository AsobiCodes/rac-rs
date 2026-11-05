//! Insomniac's "WAD" LZ compression, implemented from the packet table in Wrench's
//! docs/file_loading.md. Header: "WAD", u32 compressed size (including the 16-byte header), 9 bytes
//! padding, then a stream of packets selected by their first (flag) byte.

use std::fmt;

pub const HEADER_SIZE: usize = 0x10;

#[derive(Debug)]
pub enum WadError {
	BadMagic,
	Truncated { src_pos: usize },
	BadOffset { src_pos: usize, dest_pos: usize, distance: usize },
	DoubleLiteral { src_pos: usize, flag: u8 },
}

impl fmt::Display for WadError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{:?}", self)
	}
}

impl std::error::Error for WadError {}

pub fn is_wad(data: &[u8]) -> bool {
	data.len() >= HEADER_SIZE && &data[0..3] == b"WAD"
}

pub fn compressed_size(data: &[u8]) -> usize {
	u32::from_le_bytes([data[3], data[4], data[5], data[6]]) as usize
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>, WadError> {
	if !is_wad(data) {
		return Err(WadError::BadMagic);
	}
	let end = compressed_size(data).min(data.len());
	let mut out: Vec<u8> = Vec::with_capacity(end * 4);
	let mut src = HEADER_SIZE;
	// The stream starts with a literal, which is only legal after a match.
	let mut last_flag: u8 = 0xff;

	let byte = |pos: usize| -> Result<u8, WadError> {
		if pos < end { Ok(data[pos]) } else { Err(WadError::Truncated { src_pos: pos }) }
	};

	// A first byte above 0x11 is an initial literal run of (byte - 0x11) bytes.
	if src < end && data[src] > 0x11 {
		let len = data[src] as usize - 0x11;
		if src + 1 + len > end {
			return Err(WadError::Truncated { src_pos: src });
		}
		out.extend_from_slice(&data[src + 1..src + 1 + len]);
		src += 1 + len;
		last_flag = 0;
	}

	while src < end {
		let flag = byte(src)?;
		if flag < 0x10 {
			if last_flag < 0x10 {
				return Err(WadError::DoubleLiteral { src_pos: src, flag });
			}
			// Big literal (flag 0): N[8], N+18 bytes. Medium literal: N = flag, N+3 bytes.
			let (len, start) = if flag == 0 {
				(byte(src + 1)? as usize + 18, src + 2)
			} else {
				(flag as usize + 3, src + 1)
			};
			if start + len > end {
				return Err(WadError::Truncated { src_pos: src });
			}
			out.extend_from_slice(&data[start..start + len]);
			src = start + len;
		} else {
			let (copy_len, distance, tail_n, next);
			if flag >= 0x40 {
				// Little match: M[3] A[3] N[2] | B[8]; M+1 bytes from dest-B*8-A-1.
				let b = byte(src + 1)? as usize;
				let m = (flag >> 5) as usize;
				let a = ((flag >> 2) & 7) as usize;
				copy_len = m + 1;
				distance = b * 8 + a + 1;
				tail_n = (flag & 3) as usize;
				next = src + 2;
			} else if flag >= 0x20 {
				// Medium match 001 M[5] | A[6] N[2] | B[8]: M+2 bytes; big match 0x20 | M[8] | ...: M+33.
				let (m_len, p) = if flag == 0x20 {
					(byte(src + 1)? as usize + 33, src + 2)
				} else {
					((flag & 0x1f) as usize + 2, src + 1)
				};
				let an = byte(p)?;
				let b = byte(p + 1)? as usize;
				copy_len = m_len;
				distance = (an >> 2) as usize + b * 0x40 + 1;
				tail_n = (an & 3) as usize;
				next = p + 2;
			} else {
				// Far matches, flag 0x10..0x1f: 0001 A[1] M[3] | (big: M[8]) | B[6] N[2] | C[8].
				let a = ((flag >> 3) & 1) as usize;
				let m3 = (flag & 7) as usize;
				let (m_len, p) = if m3 == 0 {
					(byte(src + 1)? as usize + 9, src + 2)
				} else {
					(m3 + 2, src + 1)
				};
				let bn = byte(p)?;
				let c = byte(p + 1)? as usize;
				let b = (bn >> 2) as usize;
				tail_n = (bn & 3) as usize;
				next = p + 2;
				if a == 0 && b == 0 && c == 0 {
					// Zero distance: "11 00 00" (length 1) only carries its tail literals; any
					// other length skips to the next 0x2000 block, which the game refills by DMA.
					let mut s = next;
					if m3 != 1 {
						while s < end && (s % 0x2000) != 0x10 {
							s += 1;
						}
						src = s;
						last_flag = flag;
						continue;
					}
					copy_len = 0;
					distance = 0;
				} else {
					copy_len = m_len;
					// A selects the upper 16 KB window: (flag & 8) * 2048 = A * 0x4000.
					distance = 0x4000 + a * 0x4000 + b + c * 0x40;
				}
			}
			if copy_len > 0 {
				if distance > out.len() {
					return Err(WadError::BadOffset { src_pos: src, dest_pos: out.len(), distance });
				}
				let from = out.len() - distance;
				for i in 0..copy_len {
					// Byte by byte: the source range may overlap what is being written.
					let v = out[from + i];
					out.push(v);
				}
			}
			if next + tail_n > end {
				return Err(WadError::Truncated { src_pos: src });
			}
			out.extend_from_slice(&data[next..next + tail_n]);
			src = next + tail_n;
		}
		last_flag = flag;
	}
	Ok(out)
}
