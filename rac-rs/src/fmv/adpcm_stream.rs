//! PSS stream audio: SShd/SSbd headers and stateful PS2 ADPCM decode (decomp/notes/cutscenes.md).
//! First packet of a track: `SShd`, size 0x18, codec 0x10, rate, channels, interleave, 8x FF; then
//! `SSbd`, data size; then ADPCM. Data is channel-interleaved in `interleave` byte chunks (L, R, L, ...).

use super::pss::{Packet, PssDemuxer};
use crate::sound::FILTERS;
use std::io::Read;

#[derive(Debug, Clone, Copy)]
pub struct AudioHeader {
	pub codec: u32,
	pub rate: u32,
	pub channels: u32,
	pub interleave: usize,
	pub data_size: u32,
}

/// Parse the SShd/SSbd headers at the start of a track's first packet; returns the header and
/// the number of header bytes.
pub fn parse_header(d: &[u8]) -> Option<(AudioHeader, usize)> {
	let u = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
	if d.len() < 40 || &d[0..4] != b"SShd" || &d[32..36] != b"SSbd" {
		return None;
	}
	let h = AudioHeader { codec: u(8), rate: u(12), channels: u(16), interleave: u(20) as usize, data_size: u(36) };
	Some((h, 40))
}

/// Decode one 16 byte frame (28 samples) with filter history `h`.
fn decode_frame(f: &[u8], h: &mut (i32, i32), out: &mut Vec<i16>) {
	let (shift, filter) = ((f[0] & 15) as u32, ((f[0] >> 4) as usize).min(4));
	let (c1, c2) = FILTERS[filter];
	let shift = if shift > 12 { 9 } else { shift };
	for i in 0..28 {
		let b = f[2 + i / 2];
		let nib = if i & 1 == 0 { b & 15 } else { b >> 4 } as i32;
		let mut s = (((nib << 12) as i16) as i32) >> shift;
		s += (h.0 * c1 + h.1 * c2 + 32) >> 6;
		let s = s.clamp(-32768, 32767);
		h.1 = h.0;
		h.0 = s;
		out.push(s as i16);
	}
}

/// Stateful decoder: feed track bytes (header stripped) in any split, get interleaved i16.
pub struct AdpcmDecoder {
	pub channels: usize,
	pub interleave: usize,
	hist: Vec<(i32, i32)>,
	pending: Vec<u8>,
	/// Per channel: an END-flagged frame (bit 0) has been decoded; the rest of that channel is silence.
	ended: Vec<bool>,
}

impl AdpcmDecoder {
	pub fn new(channels: usize, interleave: usize) -> Self {
		AdpcmDecoder { channels, interleave, hist: vec![(0, 0); channels], pending: Vec::new(), ended: vec![false; channels] }
	}

	pub fn push(&mut self, data: &[u8], out: &mut Vec<i16>) {
		self.pending.extend_from_slice(data);
		let block = self.channels * self.interleave;
		let mut pos = 0;
		let mut per: Vec<Vec<i16>> = vec![Vec::new(); self.channels];
		while self.pending.len() - pos >= block {
			for c in 0..self.channels {
				per[c].clear();
				let chunk = &self.pending[pos + c * self.interleave..pos + (c + 1) * self.interleave];
				for f in chunk.chunks_exact(16) {
					// [PC] The SPU stops a voice after the block flagged END (bit 0). The PSS audio tracks end with such a block, then a 00 07 77 77.. terminator
					// frame: decoded as data that is a constant 0x7000 (+0.875 full scale) for 28 samples, then silence, a loud click at every movie end.
					if self.ended[c] {
						per[c].extend(std::iter::repeat(0i16).take(28));
						continue;
					}
					decode_frame(f, &mut self.hist[c], &mut per[c]);
					if f[1] & 1 != 0 {
						self.ended[c] = true;
					}
				}
			}
			for i in 0..per[0].len() {
				for c in 0..self.channels {
					out.push(per[c][i]);
				}
			}
			pos += block;
		}
		self.pending.drain(..pos);
	}
}

/// Decode a whole track (e.g. `track_for_language(0)`) of a movie; returns (header, interleaved pcm).
#[cfg(test)]
pub fn decode_track<R: Read>(dm: &mut PssDemuxer<R>, track: u8) -> std::io::Result<Option<(AudioHeader, Vec<i16>)>> {
	let mut dec: Option<(AudioHeader, AdpcmDecoder)> = None;
	let mut pcm = Vec::new();
	while let Some(p) = dm.next_packet()? {
		if let Packet::Audio { track: t, data, .. } = p {
			if t != track {
				continue;
			}
			let mut body = &data[..];
			if dec.is_none() {
				let Some((h, n)) = parse_header(body) else { continue };
				body = &body[n..];
				dec = Some((h, AdpcmDecoder::new(h.channels as usize, h.interleave)));
			}
			dec.as_mut().unwrap().1.push(body, &mut pcm);
		}
	}
	Ok(dec.map(|(h, _)| (h, pcm)))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::fmv::pss::{track_for_language, DiscReader};
	use crate::iso::Disc;
	use std::collections::HashMap;
	use std::path::{Path, PathBuf};

	fn open() -> Option<Disc> {
		let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		path.exists().then(|| Disc::open(&path).unwrap())
	}

	fn write_wav(path: &Path, rate: u32, ch: u16, pcm: &[i16]) {
		let n = (pcm.len() * 2) as u32;
		let mut b = Vec::new();
		b.extend(b"RIFF");
		b.extend((36 + n).to_le_bytes());
		b.extend(b"WAVEfmt ");
		b.extend(16u32.to_le_bytes());
		b.extend(1u16.to_le_bytes());
		b.extend(ch.to_le_bytes());
		b.extend(rate.to_le_bytes());
		b.extend((rate * 2 * ch as u32).to_le_bytes());
		b.extend((2 * ch).to_le_bytes());
		b.extend(16u16.to_le_bytes());
		b.extend(b"data");
		b.extend(n.to_le_bytes());
		for v in pcm {
			b.extend(v.to_le_bytes());
		}
		std::fs::write(path, b).unwrap();
	}

	#[test]
	fn decode_english_track() {
		let Some(mut disc) = open() else { return };
		let m = crate::scene::movie(&mut disc, 1).unwrap();
		let mut dm = PssDemuxer::new(DiscReader::new(&mut disc, m.lba, m.bytes));
		let (h, pcm) = decode_track(&mut dm, track_for_language(0)).unwrap().unwrap();
		assert_eq!((h.codec, h.rate, h.channels, h.interleave), (0x10, 48000, 2, 32));
		let secs = pcm.len() as f32 / 2.0 / h.rate as f32;
		eprintln!("english: {secs} s");
		assert!((secs - 65.0).abs() < 1.5, "{secs}");
		let dir = std::env::temp_dir();
		write_wav(&dir.join("movie22_en_10s.wav"), h.rate, 2, &pcm[..(h.rate as usize * 2 * 10)]);
	}
}
