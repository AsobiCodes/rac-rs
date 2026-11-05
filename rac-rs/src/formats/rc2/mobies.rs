//! R&C2 moby classes the game built in EE RAM outside the level file (engine split stage E5, from lane rc2's `rc2/world/mobies.rs`).

use crate::bytes::{span, u16_at, u32_at, R};
use crate::gamelayout::Game;
use crate::mobies::{decode_class, vif_unpacks, MobyClass};

/// R&C2: a class header the game built in EE RAM outside the level file (a class in the boot data, or one loaded at run
/// time into the buffers at 0x1f00000+) [C: Aranos RAM 0x178590, 0x1f0c000; their pointers are absolute RAM addresses].
/// Decoded from a copy of RAM from the header on with the high and metal packet pointers made relative to it; collision
/// and shadow data are left out; metal packets get chrome slot -2. The game wrote GS TEX0 words over the AD texture slots (AD block +0x20):
/// faces get slot keys `GADGET_TEXTURE_BASE + i` and the returned list holds TEX0 i (at most 16 per class).
pub fn moby_class_ram(ram: &[u8], hdr: usize, id: u32) -> R<(MobyClass, Vec<u64>)> {
	let mut b = span(ram, hdr, ram.len().saturating_sub(hdr))?.to_vec();
	let rel = |v: u32| -> R<u32> {
		let p = (v & 0x01ff_ffff) as usize;
		p.checked_sub(hdr).map(|x| x as u32).ok_or_else(|| format!("class pointer {p:#x} below the header {hdr:#x}"))
	};
	let put = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
	let table = rel(u32_at(&b, 0)?)? as usize;
	put(&mut b, 0, table as u32);
	put(&mut b, 0x10, 0);
	b[0xf] = 0;
	let (high, metal, first_metal) = (b[4] as usize, b[6] as usize, b[7] as usize);
	// Metal packets (index + AD unpacks): pointers made relative, their TEX0 (the environment map) back to slot -2.
	let mut metal_ok = true;
	for k in first_metal..first_metal + metal {
		let mut one = || -> R<()> {
			for f in [0, 8] {
				let at = table + k * 16 + f;
				let v = rel(u32_at(&b, at)?)?;
				put(&mut b, at, v);
			}
			let (list, qw) = (u32_at(&b, table + k * 16)? as usize, u16_at(&b, table + k * 16 + 4)? as usize);
			if let Some(u) = vif_unpacks(span(&b, list, qw * 16)?)?.get(1) {
				let (o, n) = (u.2.as_ptr() as usize - b.as_ptr() as usize, u.2.len());
				for j in 0..n / 0x40 {
					let at = o + j * 0x40 + 0x20;
					if (u32_at(&b, at)? as i32) >= -1 || u32_at(&b, at + 4)? != 0xffff_ffff {
						put(&mut b, at, -2i32 as u32);
						put(&mut b, at + 4, 0xffff_ffff);
					}
				}
			}
			Ok(())
		};
		if one().is_err() {
			metal_ok = false;
			break;
		}
	}
	if !metal_ok {
		b[6] = 0;
	}
	let mut tex0: Vec<u64> = Vec::new();
	for k in 0..high {
		for f in [0, 8] {
			let at = table + k * 16 + f;
			let v = rel(u32_at(&b, at)?)?;
			put(&mut b, at, v);
		}
		let (list, qw) = (u32_at(&b, table + k * 16)? as usize, u16_at(&b, table + k * 16 + 4)? as usize);
		let ad = match vif_unpacks(span(&b, list, qw * 16)?)?.get(2) {
			Some(u) => (u.2.as_ptr() as usize - b.as_ptr() as usize, u.2.len()),
			None => continue,
		};
		for j in 0..ad.1 / 0x40 {
			let at = ad.0 + j * 0x40 + 0x20;
			// Untextured (slot -1) stays as it is.
			if u32_at(&b, at)? as i32 == -1 {
				continue;
			}
			let t = u32_at(&b, at)? as u64 | (u32_at(&b, at + 4)? as u64) << 32;
			let i = match tex0.iter().position(|&x| x == t) {
				Some(i) => i,
				None => {
					tex0.push(t);
					tex0.len() - 1
				}
			};
			if i >= 16 {
				return Err("more than 16 textures".into());
			}
			put(&mut b, at, i as u32);
		}
	}
	// A class whose metal packets do not decode is kept without them.
	match decode_class(Game::Rc2, &b, None, id) {
		Ok(c) => Ok((c, tex0)),
		Err(_) if b[6] != 0 => {
			b[6] = 0;
			Ok((decode_class(Game::Rc2, &b, None, id)?, tex0))
		}
		Err(e) => Err(e),
	}
}
