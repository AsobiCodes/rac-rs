//! Disc tests of engine/fmv/adpcm_stream.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through the R&C1 pack, so
//! they live in it; moved unchanged from the engine file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::fmv::adpcm_stream::*;
	use crate::fmv::pss::{track_for_language, DiscReader};
	use crate::iso::Disc;
	use std::collections::HashMap;
	use std::path::{Path, PathBuf};
	use crate::fmv::pss::Packet;
	use crate::fmv::pss::PssDemuxer;
	use crate::sound::FILTERS;
	use std::io::Read;
	use crate::toc;

	fn open() -> Option<Disc> {
		let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
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
