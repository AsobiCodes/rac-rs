//! Runtime asset mutation feed: the class texture-coordinate scroll retail does to a class block in RAM.
//!
//! Retail scrolls a moby class's UVs by rewriting the class's ST unpack data in place: `FUN_00263928(class, du, dv)` (Novalis;
//! Veldin 0x24ebc0, twins on all 19 levels, `decomp/notes/runtime_asset_writes.md`) walks the class packet table and adds
//! (du, dv) to every s16 (s, t) pair of every packet. The callers are the class updaters of the Pyrocitor nozzle / Visibomb
//! aux (class 179, every level) and, on Oltanis orbit, three more scrolling classes. The mirror builds its class meshes
//! once from the WAD, so these scrolls stood still in retail mode.
//!
//! The feed compares, per live class, the first ST pair of each packet in RAM with the same pair of the WAD class block
//! (`load`, once per level). Retail adds the same delta to every pair, so one (du, dv) per class is exact; the viewer adds it
//! to the UV of every instance of the class (`Gpu::class_uv`). A class whose pairs equal the WAD's is not listed (inert).
//! The mirror never writes RAM.

use ee_oracle::Machine;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// Class id -> first ST pair (s, t) of every packet (LOD 0 then LOD 1 packet order) of the WAD class block.
static BASE: Mutex<Option<HashMap<u32, Vec<Option<[u16; 2]>>>>> = Mutex::new(None);

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
	b.get(o..o + 4).map(|x| u32::from_le_bytes(x.try_into().unwrap()))
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
	b.get(o..o + 2).map(|x| u16::from_le_bytes(x.try_into().unwrap()))
}

/// Whether a VIF code word is an UNPACK V2_16 (cmd 0x65, with or without the mask bit) of at least one element.
fn is_st_unpack(code: u32) -> bool {
	(code >> 24) & 0x6f == 0x65 && (code >> 16) & 0xff != 0
}

/// The first ST pair of each packet of a class block (class header +0 packet table, +4 / +5 packet counts of the two LODs,
/// 16-byte entries whose word 0 is the VIF list; its first code is the ST unpack, data from +4) [C: RAM class 11 Umbris,
/// moby_format.md 3.1]. A packet without an ST unpack gives None.
pub fn st_pairs(block: &[u8]) -> Option<Vec<Option<[u16; 2]>>> {
	let pt = u32_at(block, 0)? as usize;
	let n = *block.get(4)? as usize + *block.get(5)? as usize;
	if n == 0 || n > 96 {
		return None;
	}
	(0..n)
		.map(|i| {
			let v = u32_at(block, pt + 16 * i)? as usize;
			Some(is_st_unpack(u32_at(block, v)?).then(|| Some([u16_at(block, v + 4)?, u16_at(block, v + 6)?])).flatten())
		})
		.collect()
}

/// Level load (retail mode): the WAD baseline of every moby and gadget class of the level.
pub fn load(level: &crate::level::Level) {
	let mut base = HashMap::new();
	if let Ok(t) = level.table(0x18, 32) {
		for e in t {
			let (Some(start), Some(id)) = (u32_at(e, 0), u32_at(e, 4)) else { continue };
			if start == 0 {
				continue;
			}
			if let Some(p) = level.block(start as usize).ok().and_then(st_pairs) {
				base.insert(id, p);
			}
		}
	}
	if let Ok(t) = level.table(0x80, 16) {
		for e in t {
			let Some(id) = u32_at(e, 4) else { continue };
			if let Some(p) = crate::mobies::gadget_block(level, id).ok().flatten().and_then(|b| st_pairs(&b)) {
				base.entry(id).or_insert(p);
			}
		}
	}
	if super::deferring() {
		super::defer(Box::new(move || *BASE.lock().unwrap() = Some(base)));
		return;
	}
	*BASE.lock().unwrap() = Some(base);
}

/// The class UV deltas retail's scrolls hold in RAM now: (class id, (du, dv) in texture units). Walks the live moby table
/// once per field (one entry per distinct class, the class struct = moby +0x24); an entry exists only for a class whose
/// first ST pairs differ from the WAD's. A class whose packets disagree on the delta, or whose packet count differs from
/// the WAD block's, is skipped (the deltas are uniform in retail; a mismatch means the class is not the WAD's).
pub fn class_uv(m: &Machine, level: u32) -> Vec<(u32, [f32; 2])> {
	let guard = BASE.lock().unwrap();
	let Some(base) = guard.as_ref() else { return Vec::new() };
	let log = log_on();
	let (s, e) = super::table(level);
	let (s, e) = (m.rd32(s) & 0x01ff_ffff, m.rd32(e) & 0x01ff_ffff);
	let mut seen: HashSet<u32> = HashSet::new();
	let mut out = Vec::new();
	let mut a = s;
	while a < e && a < 0x0200_0000 - 0x100 {
		let (class, cp) = (m.rd16(a + 0xa6) as u32, m.rd32(a + 0x24) & 0x01ff_ffff);
		a += 0x100;
		if class == 0 || cp == 0 || !seen.insert(class) {
			continue;
		}
		let Some(b) = base.get(&class) else { continue };
		let pt = m.rd32(cp) & 0x01ff_ffff;
		let n = m.rd8(cp + 4) as usize + m.rd8(cp + 5) as usize;
		if n != b.len() || pt == 0 || pt >= 0x0200_0000 - 16 * n as u32 {
			if log {
				eprintln!("asset_feed: class {class} packet count RAM {n} WAD {} (skipped)", b.len());
			}
			continue;
		}
		let mut delta: Option<[i16; 2]> = None;
		let mut uniform = true;
		for (i, w) in b.iter().enumerate() {
			let Some(w) = w else { continue };
			let v = m.rd32(pt + 16 * i as u32) & 0x01ff_ffff;
			if v == 0 || v >= 0x0200_0000 - 8 || !is_st_unpack(m.rd32(v)) {
				uniform = false;
				break;
			}
			let d = [m.rd16(v + 4).wrapping_sub(w[0]) as i16, m.rd16(v + 6).wrapping_sub(w[1]) as i16];
			match delta {
				None => delta = Some(d),
				Some(x) if x != d => uniform = false,
				_ => {}
			}
		}
		if !uniform {
			if log {
				eprintln!("asset_feed: class {class} packets disagree on the ST delta (skipped)");
			}
			continue;
		}
		if let Some(d) = delta.filter(|d| *d != [0, 0]) {
			if log {
				eprintln!("asset_feed: class {class} ST delta ({}, {}) / 4096 at field {}", d[0], d[1], m.rd32(0x15f5ac));
			}
			out.push((class, [d[0] as f32 / 4096.0, d[1] as f32 / 4096.0]));
		}
	}
	out
}

/// RAC_ASSET_LOG=1: log the deltas and the skipped classes (debug, read once).
fn log_on() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_ASSET_LOG").is_ok_and(|v| v == "1"))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn block() -> Vec<u8> {
		// packet table at 0x20: two packets (LOD 0 count 2, LOD 1 count 0); VIF lists at 0x60 and 0x80: ST unpack, then pairs.
		let mut b = vec![0u8; 0xa0];
		b[0..4].copy_from_slice(&0x20u32.to_le_bytes());
		b[4] = 2;
		for (i, (v, s, t)) in [(0x60u32, 100u16, 200u16), (0x80, 5, 6)].into_iter().enumerate() {
			b[0x20 + 16 * i..0x24 + 16 * i].copy_from_slice(&v.to_le_bytes());
			b[v as usize..v as usize + 4].copy_from_slice(&0x7561_80c2u32.to_le_bytes());
			b[v as usize + 4..v as usize + 6].copy_from_slice(&s.to_le_bytes());
			b[v as usize + 6..v as usize + 8].copy_from_slice(&t.to_le_bytes());
		}
		b
	}

	#[test]
	fn st_pairs_reads_the_first_pair_of_each_packet() {
		assert_eq!(st_pairs(&block()), Some(vec![Some([100, 200]), Some([5, 6])]));
	}

	#[test]
	fn packet_without_st_unpack_is_none_and_bad_tables_fail() {
		let mut b = block();
		b[0x80..0x84].copy_from_slice(&0u32.to_le_bytes());
		assert_eq!(st_pairs(&b), Some(vec![Some([100, 200]), None]));
		assert_eq!(st_pairs(&[0; 8]), None);
		assert!(is_st_unpack(0x7561_80c2) && is_st_unpack(0x6561_80c2) && !is_st_unpack(0x6c61_80c2) && !is_st_unpack(0x7500_80c2));
	}
}
