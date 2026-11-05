//! Disc tests of engine/fmv/mpeg2.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through the R&C1 pack, so
//! they live in it; moved unchanged from the engine file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::fmv::mpeg2::*;
	use crate::iso::Disc;
	use std::path::PathBuf;
	use std::collections::VecDeque;
	use std::rc::Rc;
	use crate::toc;

	/// Video PES payloads (stream 0xE0) of a program stream, with PTS.
	fn video_es(ps: &[u8]) -> Vec<(Vec<u8>, Option<u64>)> {
		let mut out = Vec::new();
		let mut i = 0;
		while let Some(p) = find_sc(ps, i) {
			let code = ps[p + 3];
			i = p + 4;
			match code {
				0xba => i = p + 14 + (ps[p + 13] & 7) as usize,
				0xb9 => break,
				0xbb | 0xbc | 0xbd | 0xbe | 0xbf | 0xc0..=0xdf | 0xe0..=0xef => {
					let len = ((ps[p + 4] as usize) << 8) | ps[p + 5] as usize;
					if code == 0xe0 {
						let flags = ps[p + 7];
						let hl = ps[p + 8] as usize;
						let pts = if flags & 0x80 != 0 {
							let b = &ps[p + 9..p + 14];
							Some((((b[0] >> 1) & 7) as u64) << 30 | (b[1] as u64) << 22 | ((b[2] >> 1) as u64) << 15 | (b[3] as u64) << 7 | (b[4] >> 1) as u64)
						} else {
							None
						};
						out.push((ps[p + 9 + hl..p + 6 + len].to_vec(), pts));
					}
					i = p + 6 + len;
				}
				_ => {}
			}
		}
		out
	}

	fn to_png(f: &Frame) -> Vec<u8> {
		let (w, h) = (f.width as usize, f.height as usize);
		let cw = f.chroma_width();
		let mut rgb = Vec::with_capacity(w * h * 3);
		for y in 0..h {
			for x in 0..w {
				let yy = (f.y[y * w + x] as f32 - 16.0) * 1.164;
				let cb = f.cb[(y / 2) * cw + x / 2] as f32 - 128.0;
				let cr = f.cr[(y / 2) * cw + x / 2] as f32 - 128.0;
				rgb.push((yy + 1.596 * cr).clamp(0.0, 255.0) as u8);
				rgb.push((yy - 0.392 * cb - 0.813 * cr).clamp(0.0, 255.0) as u8);
				rgb.push((yy + 2.017 * cb).clamp(0.0, 255.0) as u8);
			}
		}
		crate::render::png(&crate::render::Image { w, h, rgb })
	}

	#[test]
	fn decode_movie_one() {
		let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = Disc::open(&path).unwrap();
		let m = crate::scene::movie(&mut disc, 0x4001).unwrap();
		let bytes = disc.read_bytes(m.lba as u64, (m.bytes as usize).min(12 << 20)).unwrap();
		let pes = video_es(&bytes);
		let mut dec = Decoder::new();
		let mut frames = Vec::new();
		let t0 = std::time::Instant::now();
		'o: for (es, pts) in &pes {
			dec.push(es, *pts);
			while let Some(f) = dec.next_frame() {
				frames.push(f);
				if frames.len() >= 100 {
					break 'o;
				}
			}
		}
		let dt = t0.elapsed().as_secs_f64();
		println!("{} frames in {dt:.2}s = {:.1} fps, errors {}", frames.len(), frames.len() as f64 / dt, dec.errors);
		assert!(frames.len() >= 100);
		let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/test_out");
		std::fs::create_dir_all(dir).unwrap();
		for n in [0, 12, 50, 99] {
			let f = &frames[n];
			println!("frame {n}: {}x{} type {} struct {} tff {} prog {} pts {:?}", f.width, f.height, f.coding_type, f.picture_structure, f.top_field_first, f.progressive, f.pts);
			std::fs::write(format!("{dir}/mpeg_{n}.png"), to_png(f)).unwrap();
		}
	}

	/// REN-8: the horizontal banding of the first picture of 22.pss is in the bitstream (period 4 rows, locked to the 8-row
	/// block grid), so retail shows it too; this pins the decoder to that pattern.
	#[test]
	fn source_banding_profile() {
		let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = Disc::open(&path).unwrap();
		let m = crate::scene::movie(&mut disc, 0x4001).unwrap();
		let bytes = disc.read_bytes(m.lba as u64, (m.bytes as usize).min(2 << 20)).unwrap();
		let mut dec = Decoder::new();
		let mut first = None;
		for (es, pts) in &video_es(&bytes) {
			dec.push(es, *pts);
			if let Some(f) = dec.next_frame() {
				first = Some(f);
				break;
			}
		}
		let f = first.expect("a decoded picture");
		let w = f.width as usize;
		assert_eq!((f.width, f.height), (512, 416));
		let r: Vec<f64> = (0..f.height as usize).map(|y| f.y[y * w..(y + 1) * w].iter().map(|&v| v as f64).sum::<f64>() / w as f64).collect();
		let (mut sum, mut n) = ([0.0f64; 8], [0u32; 8]);
		for y in 8..408 {
			let mean = r[y - 4..=y + 4].iter().sum::<f64>() / 9.0;
			sum[y % 8] += r[y] - mean;
			n[y % 8] += 1;
		}
		let bin: Vec<f64> = (0..8).map(|k| sum[k] / n[k] as f64).collect();
		for k in 0..8 {
			if matches!(k, 0 | 1 | 4 | 5) {
				assert!(bin[k] < -0.5, "bin {k} = {}", bin[k]);
			} else {
				assert!(bin[k] > 0.5, "bin {k} = {}", bin[k]);
			}
		}
	}
}
