//! Ratchet's armor files of R&C2 (G/ARMOR.WAD on the disc; from lane rc2's `rc2/world/rc2.rs`).

use crate::bytes::{span, u32_at, R};
use crate::level::Texture;

/// Ratchet's armor file: u32 header size, u32 0, then per armor (model sector, sectors, texture sector, sectors) [C: 15
/// armors; the model block starts with the four class header bytes +4..+7, u32 packet table offset (0x10), u32 size].
pub const ARMOR_WAD: &str = "G/ARMOR.WAD";
/// The hero's class (0): the level's class block keeps the header, skeleton and scale but its packets come from the worn
/// armor [C: Aranos RAM class header points at packets of an ARMOR.WAD model block loaded into RAM].
pub const HERO_CLASS: u32 = 0;
/// Moby remap entries given to the armor textures (`texture_key(Moby, ARMOR_TEXTURE_BASE + i)`); above the level's own.
pub const ARMOR_TEXTURE_BASE: u8 = 0xf0;

/// The hero class block with armor `index`'s packets appended (packet table entries relocated), its remap and its
/// textures (8-bit PIF2 images: magic, size, w, h, psm 0x13, ..., 0x20-byte header, CSM1 palette, linear indices).
pub fn armor(wad: &[u8], index: usize, class: &[u8]) -> R<(Vec<u8>, [u8; 16], Vec<Texture>)> {
	let e = span(wad, 8 + index * 16, 16)?;
	let sec = |i: usize| -> R<&[u8]> { span(wad, u32_at(e, i * 8)? as usize * 0x800, u32_at(e, i * 8 + 4)? as usize * 0x800) };
	let (model, tex) = (sec(0)?, sec(1)?);
	let mut b = class.to_vec();
	b.resize(b.len().div_ceil(16) * 16, 0);
	let x = b.len();
	let size = u32_at(model, 8)? as usize;
	b.extend_from_slice(span(model, 0, size)?);
	let table = x + u32_at(model, 4)? as usize;
	b[0..4].copy_from_slice(&(table as u32).to_le_bytes());
	b[4..8].copy_from_slice(&model[0..4]);
	// High, low and metal packets.
	let n = model[0] as usize + model[1] as usize + model[2] as usize;
	for k in 0..n {
		let p = table + k * 16;
		for f in [0, 8] {
			let v = u32_at(&b, p + f)? as usize + x;
			b[p + f..p + f + 4].copy_from_slice(&(v as u32).to_le_bytes());
		}
	}
	let count = u32_at(tex, 0)? as usize;
	let mut textures = Vec::new();
	for i in 0..count.min(16) {
		let o = u32_at(tex, 4 + i * 4)? as usize;
		let h = span(tex, o, 0x20)?;
		if &h[0..4] != b"2FIP" || u32_at(h, 16)? != 0x13 {
			return Err(format!("armor texture {i}: not an 8-bit PIF2 image"));
		}
		let (w, ht) = (u32_at(h, 8)? as usize, u32_at(h, 12)? as usize);
		textures.push(Texture::new(w, ht, span(tex, o + 0x420, w * ht)?, span(tex, o + 0x20, 0x400)?)?);
	}
	let mut remap = [0xffu8; 16];
	for (i, r) in remap.iter_mut().enumerate().take(textures.len()) {
		*r = ARMOR_TEXTURE_BASE + i as u8;
	}
	Ok((b, remap, textures))
}
