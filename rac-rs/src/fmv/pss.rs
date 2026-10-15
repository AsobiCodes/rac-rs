//! MPEG-2 program stream (PSS) demuxer, streaming over any reader (decomp/notes/cutscenes.md).
//! Video is PES stream 0xE0; audio is private stream 1 (0xBD) with a 4 byte Sony header
//! `FF A1 00 <track>`; the five languages are separate tracks interleaved packet by packet.

use crate::iso::{Disc, SECTOR_SIZE};
use std::io::{self, Read};

#[derive(Debug, Clone)]
pub enum Packet {
	Video { pts: Option<u64>, dts: Option<u64>, data: Vec<u8> },
	/// `data` excludes the 4 byte FF A1 00 track header. The first packet of a track starts with
	/// the SShd/SSbd headers (see adpcm_stream).
	Audio { track: u8, pts: Option<u64>, data: Vec<u8> },
}

/// Track index in the movie for a front-end language (0 EN, 1 FR, 2 DE, 3 ES, 4 IT; settings.rs `lang`).
/// Track 1 is unused on this disc; the same mapping as the help-voice banks (sound.rs).
pub fn track_for_language(lang: u8) -> u8 {
	[0u8, 2, 3, 4, 5][lang.min(4) as usize]
}

pub struct PssDemuxer<R: Read> {
	r: R,
	pub pack_count: u32,
}

fn pts_at(b: &[u8]) -> u64 {
	(((b[0] as u64 >> 1) & 7) << 30) | ((b[1] as u64) << 22) | (((b[2] as u64) >> 1) << 15) | ((b[3] as u64) << 7) | (b[4] as u64 >> 1)
}

impl<R: Read> PssDemuxer<R> {
	pub fn new(r: R) -> Self {
		PssDemuxer { r, pack_count: 0 }
	}

	fn read_exact_opt(&mut self, buf: &mut [u8]) -> io::Result<bool> {
		match self.r.read_exact(buf) {
			Ok(()) => Ok(true),
			Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
			Err(e) => Err(e),
		}
	}

	/// Next video or audio packet; None at the end of the stream.
	pub fn next_packet(&mut self) -> io::Result<Option<Packet>> {
		loop {
			let mut h = [0u8; 4];
			if !self.read_exact_opt(&mut h)? {
				return Ok(None);
			}
			// Resynchronise on a start code prefix.
			while h[..3] != [0, 0, 1] {
				h.copy_within(1.., 0);
				if !self.read_exact_opt(&mut h[3..])? {
					return Ok(None);
				}
			}
			match h[3] {
				0xba => {
					let mut p = [0u8; 10];
					if !self.read_exact_opt(&mut p)? {
						return Ok(None);
					}
					self.pack_count += 1;
					let stuff = (p[9] & 7) as usize;
					let mut s = [0u8; 7];
					if !self.read_exact_opt(&mut s[..stuff])? {
						return Ok(None);
					}
				}
				0xb9 => return Ok(None),
				id => {
					let mut l = [0u8; 2];
					if !self.read_exact_opt(&mut l)? {
						return Ok(None);
					}
					let mut body = vec![0u8; u16::from_be_bytes(l) as usize];
					if !self.read_exact_opt(&mut body)? {
						return Ok(None);
					}
					if id != 0xe0 && id != 0xbd {
						continue; // system header, padding, ...
					}
					if body.len() < 3 {
						continue;
					}
					let flags = body[1];
					let hl = body[2] as usize;
					if body.len() < 3 + hl {
						continue;
					}
					let (mut pts, mut dts) = (None, None);
					if flags & 0x80 != 0 && hl >= 5 {
						pts = Some(pts_at(&body[3..8]));
					}
					if flags & 0x40 != 0 && hl >= 10 {
						dts = Some(pts_at(&body[8..13]));
					}
					let pl = &body[3 + hl..];
					if id == 0xe0 {
						return Ok(Some(Packet::Video { pts, dts, data: pl.to_vec() }));
					}
					if pl.len() < 4 || pl[0] != 0xff {
						continue;
					}
					return Ok(Some(Packet::Audio { track: pl[3], pts, data: pl[4..].to_vec() }));
				}
			}
		}
	}
}

/// Sequential reader over `len` bytes of a disc starting at `lba`.
pub struct DiscReader<'a> {
	disc: &'a mut Disc,
	next: u64,
	left: u64,
	buf: Vec<u8>,
	pos: usize,
}

impl<'a> DiscReader<'a> {
	pub fn new(disc: &'a mut Disc, lba: u32, len: u64) -> Self {
		DiscReader { disc, next: lba as u64, left: len, buf: Vec::new(), pos: 0 }
	}
}

impl Read for DiscReader<'_> {
	fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
		if self.pos >= self.buf.len() {
			if self.left == 0 {
				return Ok(0);
			}
			let sectors = (self.left.div_ceil(SECTOR_SIZE)).min(64);
			let mut b = self.disc.read_sectors(self.next, sectors)?;
			b.truncate(self.left.min(sectors * SECTOR_SIZE) as usize);
			self.next += sectors;
			self.left -= b.len() as u64;
			self.buf = b;
			self.pos = 0;
		}
		let n = out.len().min(self.buf.len() - self.pos);
		out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
		self.pos += n;
		Ok(n)
	}
}
