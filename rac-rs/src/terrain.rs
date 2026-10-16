//! Static terrain ("tfrags") at LOD 0 or 2, decoded from the VIF packet programs that the game
//! uploads to VU1, without running VIF or VU code.
//!
//! Each fragment's packets are checked against the one program shape seen on the disc; anything
//! else is an error rather than a guess. Layout follows rac1-decomp's tools/extract/terrain.py
//! (itself from Wrench's tfrag reader; stream selection confirmed there by func_002352C8).

use crate::bytes::{f32_at, i32_at, span, u16_at, u32_at, R};
use crate::gameplay::{baked_colour, pext5, select_light, DirLight, TERRAIN_CAP};
use crate::mesh::{Lit, LitKind, Mesh};
use std::collections::BTreeSet;

const FRAGMENT_SIZE: usize = 0x40;
const STROW: u8 = 0x30;
const STCYCL: u8 = 0x01;
const STMOD: u8 = 0x05;
/// STROW for vertex infos: float 2048.0 in the first two lanes.
const INFO_ROW: i32 = 0x4500_0000;

/// VIF UNPACK element sizes: V3-16, V4-32, V4-16, V4-8.
fn unpack_size(cmd: u8) -> Option<usize> {
	match cmd {
		0x69 => Some(6),
		0x6c => Some(16),
		0x6d => Some(8),
		0x6e => Some(4),
		_ => None,
	}
}

#[derive(Clone, Copy)]
struct Packet<'a> {
	cmd: u8,
	imm: u16,
	count: usize,
	data: &'a [u8],
}

/// Frame a VIF command list, keeping UNPACK payloads packed.
fn packets(data: &[u8]) -> R<Vec<Packet<'_>>> {
	let mut out = Vec::new();
	let mut pos = 0;
	while pos < data.len() {
		let word = u32_at(data, pos)?;
		let cmd = (word >> 24) as u8;
		let count = match (word >> 16) & 0xff {
			0 => 256,
			n => n as usize,
		};
		let size = match cmd {
			c if unpack_size(c).is_some() => unpack_size(c).unwrap() * count,
			STROW => 16,
			0 | STCYCL | STMOD => 0,
			c => return Err(format!("unsupported terrain VIF command {c:#x}")),
		};
		let padded = (size + 3) & !3;
		let payload = span(data, pos + 4, padded)?;
		out.push(Packet { cmd, imm: word as u16, count, data: &payload[..size] });
		pos += 4 + padded;
	}
	Ok(out)
}

/// The non-NOP packets of a command list, which must be exactly `cmds`.
fn program<'a>(data: &'a [u8], cmds: &[u8]) -> R<Vec<Packet<'a>>> {
	let p: Vec<_> = packets(data)?.into_iter().filter(|p| p.cmd != 0).collect();
	if p.iter().map(|p| p.cmd).ne(cmds.iter().copied()) {
		return Err("unsupported VIF program".into());
	}
	Ok(p)
}

fn row(p: &Packet) -> R<[i32; 4]> {
	Ok([i32_at(p.data, 0)?, i32_at(p.data, 4)?, i32_at(p.data, 8)?, i32_at(p.data, 12)?])
}

fn i16x<const N: usize>(data: &[u8]) -> Vec<[i16; N]> {
	data.chunks_exact(N * 2)
		.map(|c| std::array::from_fn(|j| i16::from_le_bytes([c[j * 2], c[j * 2 + 1]])))
		.collect()
}

/// Byte offset of `part` (a sub-slice of `data`) from `start`.
fn offset_in(data: &[u8], start: usize, part: &[u8]) -> usize {
	part.as_ptr() as usize - data.as_ptr() as usize - start
}

/// Per-position baked colours from the fragment's 8-byte light records (FUN_00294cf8,
/// decomp/notes/vertex_colours.md): u16 position offset, signed azimuth and elevation bytes, RGBA5551
/// base, u16 selector. Header +0x34 (as i8 [I]) < 0 takes each record's selector, else that one
/// light index for the fragment. Point lights (+0x36) are unused on Veldin (all 0xffff) [P].
fn fragment_colours(data: &[u8], start: usize, header: &[u8], pos_offs: &[usize], lights: &[DirLight]) -> R<(Vec<[f32; 4]>, Vec<[f32; 3]>)> {
	let (from, to) = (u16_at(header, 0x30)? as usize + 0x10, u16_at(header, 0x32)? as usize);
	let one = header[0x34] as i8;
	let mut out = vec![[64.0, 64.0, 64.0, 128.0]; pos_offs.len()];
	let mut normals = vec![[0.0, 0.0, 1.0]; pos_offs.len()];
	let mut found = 0;
	for r in span(data, start + from, to.saturating_sub(from))?.chunks_exact(8) {
		let off = u16_at(r, 0)? as usize;
		let Some(i) = pos_offs.iter().position(|&o| o == off) else { continue };
		// The angle bytes are full-circle (256 = 360 degrees): az turns in the ground plane (0 = +x), el lifts out of it (64 =
		// straight up, every flat floor). Fitted 2026-10-13 against the RGBA8 retail RAM holds after FUN_00294cf8 (light table
		// read from RAM): 94-99 % of the records of levels 1, 4, 10-18 reproduce it within one byte; the earlier pi / 256
		// decode matched 3-10 % (every flat floor took a 45 degree tilt, E1 of the effects census) [C].
		let (az, el) = (r[2] as i8 as f32 * std::f32::consts::PI / 128.0, r[3] as i8 as f32 * std::f32::consts::PI / 128.0);
		let n = [el.cos() * az.cos(), el.cos() * az.sin(), el.sin()];
		let selector = if one < 0 { u16_at(r, 6)? as u32 } else { one as u32 };
		out[i] = baked_colour(&select_light(lights, selector), pext5(u16_at(r, 4)?), n, TERRAIN_CAP);
		normals[i] = n;
		found += 1;
	}
	if found < pos_offs.len() {
		return Err(format!("terrain light records cover {found} of {} positions", pos_offs.len()));
	}
	Ok((out, normals))
}

/// Faces and material slots from the strip descriptors (Wrench's recover_faces).
/// Each descriptor is (count, packet end, material offset, pad) as signed bytes; count 0 ends the
/// list and a negative count switches material. Strips longer than 3 are even and split as quads.
fn strip_faces(indices: &[u8], strips: &[u8], materials: usize) -> R<Vec<(usize, [u8; 3])>> {
	let mut out = Vec::new();
	let mut cursor = 0;
	let mut material: Option<usize> = None;
	let mut terminated = false;
	for d in strips.chunks_exact(4) {
		let encoded = d[0] as i8;
		let ad_offset = d[2] as i8;
		if encoded == 0 {
			terminated = true;
			break;
		}
		let count = (d[0] & 0x7f) as usize;
		if count < 3 || (count > 3 && count % 2 == 1) {
			return Err(format!("unsupported terrain strip length {count}"));
		}
		if encoded < 0 && ad_offset >= 0 {
			if ad_offset % 5 != 0 {
				return Err("terrain material offset is not a five-qword record".into());
			}
			material = Some(ad_offset as usize / 5);
		}
		let m = material.filter(|&m| m < materials).ok_or("terrain strip has no valid material")?;
		let s = span(indices, cursor, count)?;
		cursor += count;
		if count == 3 {
			out.push((m, [s[0], s[1], s[2]]));
		} else {
			for i in 0..count - 2 {
				let (a, b, c) = (s[i], s[i + 1], s[i + 2]);
				out.push((m, if i % 2 == 0 { [c, b, a] } else { [b, c, a] }));
			}
		}
	}
	if !terminated {
		return Err("terrain strip list lacks a terminator".into());
	}
	if indices.len() - cursor > 3 {
		return Err("terrain strip list leaves more than alignment padding".into());
	}
	Ok(out)
}

struct Cursor<'a, 'b> {
	p: &'b [Packet<'a>],
	at: usize,
}

impl<'a, 'b> Cursor<'a, 'b> {
	fn take(&mut self, cmd: u8, imm: Option<u16>, expect_row: Option<[i32; 4]>) -> R<Packet<'a>> {
		let packet = *self.p.get(self.at).ok_or("truncated terrain refinement program")?;
		self.at += 1;
		if packet.cmd != cmd || imm.is_some_and(|i| i != packet.imm) {
			return Err("unsupported terrain refinement command or address".into());
		}
		if let Some(r) = expect_row {
			if row(&packet)? != r {
				return Err("unsupported terrain refinement origin or base".into());
			}
		}
		Ok(packet)
	}
}

/// What `refinement` found beyond the finest strips (REN-CULL-TFRAG): per stage the parent info per refinement vertex, the seam
/// migration bytes (parent info per seam entry) and the position / info bases and counts.
struct Refined<'a> {
	indices: &'a [u8],
	strips: &'a [u8],
	parents: [&'a [u8]; 2],
	migration: [&'a [u8]; 2],
	count: [usize; 2],
	position_base: [usize; 2],
	info_base: [usize; 2],
	info_count: [usize; 2],
}

/// Add the LOD-1 and LOD-0 stages' vertices; return the finest (indices, strips).
/// Each stage adds absolute positions with two earlier parents each, plus vertex infos for them
/// and for texture seams. Migration arrays (LOD morphing) are checked for placement only.
#[allow(clippy::too_many_arguments)]
fn refinement<'a>(
	data: &'a [u8],
	start: usize,
	header: &[u8],
	vu: &[u16; 20],
	origin: [i32; 3],
	positions: &mut Vec<[i16; 3]>,
	pos_offs: &mut Vec<usize>,
	infos: &mut Vec<[i16; 4]>,
) -> R<Refined<'a>> {
	let shared = u16_at(header, 0x16)? as usize;
	let high = u16_at(header, 0x18)? as usize;
	let begin = u16_at(header, 0x1a)? as usize;
	let end = u16_at(header, 0x1e)? as usize;
	let middle = shared + header[0x22] as usize * 16;
	if !(high <= begin && begin < middle && middle < end) || begin + header[0x23] as usize * 16 != end {
		return Err("invalid terrain refinement boundaries".into());
	}
	let vu = |i: usize| vu[i] as usize;
	let index_row = [vu(7) as i32; 4];
	let info_row = [INFO_ROW, INFO_ROW, vu(6) as i32, vu(6) as i32];
	let (mut indices, mut strips) = (None, None);
	let mut out = Refined { indices: &[], strips: &[], parents: [&[]; 2], migration: [&[]; 2], count: [0; 2], position_base: [0; 2], info_base: [0; 2], info_count: [0; 2] };
	let raws = [span(data, start + begin, middle - begin)?, span(data, start + middle, end - middle)?];
	for (stage, raw) in raws.into_iter().enumerate() {
		let p: Vec<_> = packets(raw)?.into_iter().filter(|p| p.cmd != 0).collect();
		let mut c = Cursor { p: &p, at: 0 };
		let count = vu(2 + stage * 2);
		let position_base = positions.len();
		let info_base = infos.len();
		let pos_imm = (0x8000 | (vu(6) + position_base * 2)) as u16;
		let (mut pos, mut info, mut parents, mut migration) = (None, None, None, &[][..]);
		out.position_base[stage] = position_base;
		out.info_base[stage] = info_base;
		if stage == 1 {
			if count > 0 {
				pos = Some(c.take(0x69, Some(pos_imm), None)?);
			}
			c.take(STMOD, Some(0), None)?;
			c.take(STCYCL, Some(0x404), None)?;
			strips = Some(c.take(0x6e, Some((0x8000 | vu(18)) as u16), None)?.data);
			c.take(STROW, None, Some(index_row))?;
			c.take(STMOD, Some(1), None)?;
			indices = Some(c.take(0x6e, Some((0xc000 | vu(13)) as u16), None)?.data);
		} else if count > 0 {
			c.take(STROW, None, Some(index_row))?;
			c.take(STMOD, Some(1), None)?;
		}
		if count > 0 {
			let par = c.take(0x6e, Some((0xc000 | vu(14 + stage * 2)) as u16), None)?.data;
			if !(par.len() >= count && par.len() - count < 4) {
				return Err("terrain parent array count mismatch".into());
			}
			parents = Some(par);
			if c.p.get(c.at).is_some_and(|x| x.cmd == 0x6e) {
				migration = c.take(0x6e, Some((0xc000 | vu(15 + stage * 2)) as u16), None)?.data; // migration data
			}
			c.take(STROW, None, Some(info_row))?;
			info = Some(c.take(0x6d, Some((0x8000 | vu(9 + stage * 2)) as u16), None)?);
			if vu(9 + stage * 2) != vu(7) + info_base {
				return Err("non-contiguous terrain vertex-info array".into());
			}
		}
		if stage == 0 {
			c.take(STROW, None, Some([origin[0], origin[1], origin[2], 0]))?;
			c.take(STCYCL, Some(0x102), None)?;
			if count > 0 {
				pos = Some(c.take(0x69, Some(pos_imm), None)?);
			}
		} else {
			c.take(STMOD, Some(0), None)?;
		}
		if c.at != p.len() {
			return Err("unexpected terrain refinement packets".into());
		}
		if count == 0 {
			if vu(3 + stage * 2) != 0 {
				return Err("terrain seam entries without a refinement stage".into());
			}
			continue;
		}
		let (pos, info, parents) = (pos.unwrap(), info.unwrap(), parents.unwrap());
		out.parents[stage] = parents;
		out.migration[stage] = migration;
		out.count[stage] = count;
		out.info_count[stage] = info.count;
		if pos.count != count || info.count < count {
			return Err("terrain refinement vertex count mismatch".into());
		}
		positions.extend(i16x::<3>(pos.data));
		pos_offs.extend((0..count).map(|i| offset_in(data, start, pos.data) + i * 6));
		let added = i16x::<4>(info.data);
		let covered: BTreeSet<i32> = added[..count].iter().map(|e| e[3] as i32).collect();
		let expected: BTreeSet<i32> = (position_base * 2..positions.len() * 2).step_by(2).map(|x| x as i32).collect();
		if covered != expected {
			return Err("terrain refinement position coverage mismatch".into());
		}
		infos.extend(added.iter().copied());
		for (i, e) in added.iter().enumerate() {
			let parent = e[2] as i32;
			if parent < 0 || parent % 2 != 0 || parent as usize / 2 >= positions.len() {
				return Err("invalid terrain refinement parent position".into());
			}
			if i < count {
				let pi = parents[i] as usize;
				if pi >= info_base || (infos[pi][3] as i32 / 2) as usize >= position_base || parent as usize / 2 >= position_base {
					return Err("invalid terrain refinement parent relationship".into());
				}
			}
		}
	}
	match (indices, strips) {
		(Some(i), Some(s)) => {
			(out.indices, out.strips) = (i, s);
			Ok(out)
		}
		_ => Err("terrain refinement has no final strips".into()),
	}
}

/// One tfrag: common vertices, then LOD-2 strips or both refinements.
fn fragment(data: &[u8], table_offset: usize, header: &[u8], lod: u8, lights: Option<&[DirLight]>, name: String) -> R<Mesh> {
	fragment_all(data, table_offset, header, lod, lights, name, false).map(|r| r.0)
}

/// `fragment`, and with `want_lods` (LOD 0 only) the far / mid / near meshes and the morph tables as well.
fn fragment_all(data: &[u8], table_offset: usize, header: &[u8], lod: u8, lights: Option<&[DirLight]>, name: String, want_lods: bool) -> R<(Mesh, Option<TfragLods>)> {
	let start = table_offset + u32_at(header, 0x10)? as usize;
	let low = u16_at(header, 0x14)? as usize;
	let shared = u16_at(header, 0x16)? as usize;
	let high = u16_at(header, 0x18)? as usize;
	if !(low < shared && shared < high) {
		return Err("invalid terrain program boundaries".into());
	}
	let coarse = program(span(data, start + low, shared - low)?, &[STROW, STMOD, 0x6e, STMOD, 0x6e])?;
	let common = program(
		span(data, start + shared, high - shared)?,
		&[0x6d, 0x6c, STROW, STMOD, 0x6d, STROW, STCYCL, 0x69, STCYCL, STMOD],
	)?;
	if (coarse[1].imm, coarse[3].imm, common[3].imm, common[6].imm, common[8].imm, common[9].imm) != (1, 0, 1, 0x102, 0x404, 0) {
		return Err("unsupported terrain VIF cycle or mode".into());
	}
	for (p, unsigned) in [(&coarse[2], true), (&coarse[4], false), (&common[0], true), (&common[1], false), (&common[4], false), (&common[7], false)] {
		if p.imm & 0x3c00 != 0 || (p.imm & 0x4000 != 0) != unsigned {
			return Err("unsupported terrain UNPACK flags".into());
		}
		if p.imm & 0x8000 == 0 {
			return Err("terrain UNPACK is not TOPS-relative".into());
		}
	}
	if common[0].data.len() != 40 {
		return Err("unexpected terrain VU header size".into());
	}
	// VU memory map (Wrench's TfragHeaderUnpack).
	let mut vu = [0u16; 20];
	for (i, v) in vu.iter_mut().enumerate() {
		*v = u16_at(common[0].data, i * 2)?;
	}
	if common[7].count != vu[0] as usize {
		return Err("terrain common position count mismatch".into());
	}
	if (common[7].imm & 0x3ff, common[4].imm & 0x3ff, coarse[2].imm & 0x3ff, coarse[4].imm & 0x3ff) != (vu[6], vu[7], vu[13], vu[18]) {
		return Err("terrain VU addresses disagree with the header".into());
	}
	if row(&coarse[0])? != [vu[7] as i32; 4] {
		return Err("unsupported terrain index base".into());
	}
	if row(&common[2])? != [INFO_ROW, INFO_ROW, 0, vu[6] as i32] {
		return Err("unsupported terrain vertex-info base".into());
	}
	let materials = header[0x28] as usize;
	if common[1].data.len() != materials * 80 {
		return Err("terrain texture primitive count mismatch".into());
	}
	// Five-qword GS primitives; the first word is the texture index until the game patches it at
	// load time (func_00204340 in rac1-decomp's v2.00 naming).
	let textures: Vec<u32> = (0..materials).map(|i| u32_at(common[1].data, i * 80)).collect::<R<_>>()?;
	let material_states: Vec<TfragMaterial> = (0..materials)
		.map(|i| {
			let k = (u32_at(common[1].data, i * 80 + 0x10)? & 0xfff) as i32;
			let k = if k >= 0x800 { k - 0x1000 } else { k };
			// GS CLAMP_1 source (qword 2): word +0x20 WMS, word +0x24 WMT (as the shrub records); retail RAM holds the built
			// CLAMP_1 qword (register 0x08) there.
			let (wms, wmt) = (u32_at(common[1].data, i * 80 + 0x20)?, u32_at(common[1].data, i * 80 + 0x24)?);
			if crate::viewer::gs_state::enabled() {
				let mmin = u32_at(common[1].data, i * 80 + 0x14)? as u64;
				crate::viewer::gs_state::guard_world("tfrag", crate::viewer::gs_state::TEST_TFRAG, (k as u32 as u64 & 0xfff) << 32 | mmin << 6 | 0x20, wms as u64 | (wmt as u64) << 2);
			}
			if wms > 1 || wmt > 1 {
				return Err("unsupported terrain CLAMP region mode".into());
			}
			Ok(TfragMaterial { texture: textures[i], k: k as f32 / 16.0, mmin: u32_at(common[1].data, i * 80 + 0x14)?, wrap: wms | wmt << 1 })
		})
		.collect::<R<_>>()?;
	let mut positions = i16x::<3>(common[7].data);
	let mut pos_offs: Vec<usize> = (0..positions.len()).map(|i| offset_in(data, start, common[7].data) + i * 6).collect();
	let o = row(&common[5])?;
	let origin = [o[0], o[1], o[2]];
	let mut infos = i16x::<4>(common[4].data);
	if infos.iter().any(|e| e[2] != 0x1000 || e[3] < 0 || e[3] % 2 != 0 || e[3] as usize / 2 >= positions.len()) {
		return Err("invalid terrain common vertex info".into());
	}
	let n_common = infos.len();
	let refined = if lod == 0 { Some(refinement(data, start, header, &vu, origin, &mut positions, &mut pos_offs, &mut infos)?) } else { None };
	let (indices, strips) = match &refined {
		Some(r) => (r.indices, r.strips),
		None => (coarse[2].data, coarse[4].data),
	};
	let sphere = [f32_at(header, 0)?, f32_at(header, 4)?, f32_at(header, 8)?, f32_at(header, 12)?];
	if sphere.iter().any(|x| !x.is_finite()) || sphere[3] < 0.0 {
		return Err("invalid terrain bounding sphere".into());
	}
	let colours = lights.map(|l| fragment_colours(data, start, header, &pos_offs, l)).transpose()?;
	let asm = Assemble { origin, positions: &positions, colours: colours.as_ref(), textures: &textures, materials, sphere };
	let (mesh, near_mat) = assemble_mats(&asm, name.clone(), &infos, indices, strips)?;
	let Some(r) = refined.filter(|_| want_lods) else { return Ok((mesh, None)) };
	// The LOD tables (REN-CULL-TFRAG): far = common vertices on the coarse strips, mid = common + stage 0 on the LOD 1 strips
	// of [+0x18, +0x1a), near = everything.
	let (far, far_mat) = assemble_mats(&asm, format!("{name}_far"), &infos[..n_common], coarse[2].data, coarse[4].data)?;
	let n_mid = n_common + r.info_count[0];
	let (high, begin) = (u16_at(header, 0x18)? as usize, u16_at(header, 0x1a)? as usize);
	let region = packets(span(data, start + high, begin - high)?)?.into_iter().filter(|p| p.cmd != 0).collect::<Vec<_>>();
	let mut c = Cursor { p: &region, at: 0 };
	let mid_strips = c.take(0x6e, Some((0x8000 | vu[18]) as u16), None)?.data;
	c.take(STROW, None, Some([vu[7] as i32; 4]))?;
	c.take(STMOD, Some(1), None)?;
	let mid_indices = c.take(0x6e, Some((0xc000 | vu[13]) as u16), None)?.data;
	if c.at != region.len() {
		return Err("unexpected terrain LOD 1 packets".into());
	}
	let (mid, mid_mat) = assemble_mats(&asm, format!("{name}_mid"), &infos[..n_mid], mid_indices, mid_strips)?;
	let pos_of = |v: usize| infos[v][3] as usize / 2;
	let (mut morph, mut snap): ([Vec<[u32; 3]>; 2], [Vec<[u32; 3]>; 2]) = Default::default();
	for st in 0..2 {
		let (base, count) = (r.info_base[st], r.count[st]);
		for i in 0..count {
			let (v, b) = (base + i, r.parents[st][i] as usize);
			morph[st].push([pos_of(v) as u32, (infos[v][2] as usize / 2) as u32, pos_of(b) as u32]);
			snap[st].push([v as u32, (infos[v][2] as usize / 2) as u32, b as u32]);
		}
		let seam = vu[3 + st * 2] as usize;
		if seam > 0 {
			let first = (vu[10 + st * 2] as usize).checked_sub(vu[7] as usize).ok_or("terrain seam infos before the index base")?;
			if r.migration[st].len() < seam || r.migration[st].len() - seam >= 4 || first + seam > infos.len() {
				return Err("terrain seam migration data mismatch".into());
			}
			for j in 0..seam {
				let (v, b) = (first + j, r.migration[st][j] as usize);
				if b >= infos.len() {
					return Err("terrain seam parent outside the vertex infos".into());
				}
				snap[st].push([v as u32, (infos[v][2] as usize / 2) as u32, b as u32]);
			}
		}
	}
	let pos_world: Vec<[f32; 3]> = positions.iter().map(|p| std::array::from_fn(|j| (origin[j] + p[j] as i32) as f32 / 1024.0)).collect();
	let vert_pos: Vec<u32> = infos.iter().map(|e| (e[3] as usize / 2) as u32).collect();
	let corners_at = start + u16_at(header, 0x38)? as usize;
	let raw = span(data, corners_at, 64)?;
	let corners: [[f32; 3]; 8] = std::array::from_fn(|k| std::array::from_fn(|j| i16::from_le_bytes([raw[k * 8 + j * 2], raw[k * 8 + j * 2 + 1]]) as f32 * 64.0 / 1024.0));
	let m = sphere.map(|x| x / 1024.0);
	let cull = TfragCull { centre: [m[0], m[1], m[2]], radius: m[3], corners, force_far: header[0x27] != 0, occ: crate::occlusion::UNMAPPED, id: header[0x3d] };
	// One vertex per position carries the relight state (the `LitWorld` object); the three meshes read its colours.
	let mut lit = Mesh::new(format!("{name}_positions"));
	lit.positions = pos_world.clone();
	if let Some((c, n)) = &colours {
		lit.colours = c.clone();
		lit.lit = Some(Lit { kind: LitKind::Tfrag, sphere: m, origin: [0.0; 3], cap: TERRAIN_CAP, normals: n.clone(), raw: c.clone() });
	}
	let lods = TfragLods { materials: material_states, face_mat: [far_mat, mid_mat, near_mat], far, mid, near: mesh.clone(), pos_world, vert_pos, lit, morph, snap, cull };
	Ok((mesh, Some(lods)))
}

struct Assemble<'a> {
	origin: [i32; 3],
	positions: &'a [[i16; 3]],
	colours: Option<&'a (Vec<[f32; 4]>, Vec<[f32; 3]>)>,
	textures: &'a [u32],
	materials: usize,
	sphere: [f32; 4],
}


/// `assemble`, plus each face's material record index (the face key only carries the texture).
fn assemble_mats(a: &Assemble, name: String, infos: &[[i16; 4]], indices: &[u8], strips: &[u8]) -> R<(Mesh, Vec<u8>)> {
	let mut mesh = Mesh::new(name);
	let mut normals = Vec::new();
	for &[s, t, _parent, position] in infos {
		if position < 0 || position % 2 != 0 || position as usize / 2 >= a.positions.len() {
			return Err("invalid terrain vertex reference".into());
		}
		let p = a.positions[position as usize / 2];
		mesh.positions.push(std::array::from_fn(|j| (a.origin[j] + p[j] as i32) as f32 / 1024.0));
		if let Some((c, n)) = a.colours {
			mesh.colours.push(c[position as usize / 2]);
			normals.push(n[position as usize / 2]);
		}
		// Negative coordinates are halved, as in Wrench; not yet traced in VU code.
		let uv = |x: i16| x as f32 / if x < 0 { 8192.0 } else { 4096.0 };
		mesh.uvs.push([uv(s), uv(t)]);
	}
	let mut face_mat = Vec::new();
	for (m, face) in strip_faces(indices, strips, a.materials)? {
		if face.iter().any(|&i| i as usize >= mesh.positions.len()) {
			return Err("terrain face index outside the vertex infos".into());
		}
		mesh.faces.push((a.textures[m], face.map(u32::from)));
		face_mat.push(m as u8);
	}
	if a.colours.is_some() {
		// The header sphere is in 1/1024 units like the vertices (the retail test scales the light by 1024).
		let sphere = a.sphere.map(|x| x / 1024.0);
		mesh.lit = Some(Lit { kind: LitKind::Tfrag, sphere, origin: [0.0; 3], cap: TERRAIN_CAP, normals, raw: mesh.colours.clone() });
	}
	Ok((mesh, face_mat))
}

/// Every fragment of a tfrag block at one static LOD.
pub fn terrain(data: &[u8], lod: u8) -> R<Vec<Mesh>> {
	terrain_lit(data, lod, None)
}

/// `terrain`, plus baked vertex colours under the level's directional lights when given.
pub fn terrain_lit(data: &[u8], lod: u8, lights: Option<&[DirLight]>) -> R<Vec<Mesh>> {
	if lod != 0 && lod != 2 {
		return Err("terrain LOD must be 0 or 2".into());
	}
	let table_offset = u32_at(data, 0)? as usize;
	let count = u32_at(data, 4)? as usize;
	let table = span(data, table_offset, count * FRAGMENT_SIZE)?;
	table
		.chunks_exact(FRAGMENT_SIZE)
		.enumerate()
		.map(|(i, h)| fragment(data, table_offset, h, lod, lights, format!("Terrain_{i:03}")).map_err(|e| format!("terrain fragment {i}: {e}")))
		.collect()
}


/// Per-fragment cull data of the LOD walk (metres): the header sphere, the 8 box corners, byte +0x27 (always the far mesh), the
/// occlusion pair (REN-CULL-OCC) and the fragment's mapping id (header byte +0x3d).
#[derive(Clone, Debug)]
pub struct TfragCull {
	pub centre: [f32; 3],
	pub radius: f32,
	pub corners: [[f32; 3]; 8],
	pub force_far: bool,
	pub occ: (u8, u8),
	pub id: u8,
}

/// One fragment at its three LODs (REN-CULL-TFRAG, BRIDGE-31). The meshes are prefixes of one vertex-info list: `far` the common
/// vertices, `mid` those plus stage 0, `near` all of them, so a vertex index means the same vertex in every mesh. `vert_pos[v]` is
/// vertex v's position record (common, then stage 0, then stage 1), `pos_world` every position record in metres, and `lit` one
/// vertex per position record carrying the relight state. `morph[stage]` lists [position, parent A, parent B] per refinement
/// vertex; `snap[stage]` lists [vertex, parent A position, parent B vertex] in the retail order (list pass, then seam pass).
pub struct TfragLods {
	/// Per material record: texture index, GS mip bias K and MMIN (REN-TEX-RESIDENCY).
	pub materials: Vec<TfragMaterial>,
	/// Per face of far, mid and near: the material record index of its strip.
	pub face_mat: [Vec<u8>; 3],
	pub far: Mesh,
	pub mid: Mesh,
	pub near: Mesh,
	pub pos_world: Vec<[f32; 3]>,
	pub vert_pos: Vec<u32>,
	pub lit: Mesh,
	pub morph: [Vec<[u32; 3]>; 2],
	pub snap: [Vec<[u32; 3]>; 2],
	pub cull: TfragCull,
}

/// The mip state of one terrain material record (80 bytes, five A+D qwords; word +0x10 holds K in its low 12 bits as a signed
/// value in 1/16 mip, word +0x14 the MMIN mode, 4 = nearest mip; qword 2 = GS CLAMP_1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TfragMaterial {
	pub texture: u32,
	pub k: f32,
	pub mmin: u32,
	/// CLAMP_1 wrap: bit 0 = WMS CLAMP (u), bit 1 = WMT CLAMP (v); clear = REPEAT (the `wrap_samplers` index + 1, as shrubs).
	pub wrap: u32,
}

/// Every fragment of a tfrag block with its LOD meshes and tables, and the LOD scale B (f32 at block +0x8); `map` gives each
/// fragment its occlusion bytes.
pub fn terrain_lods(data: &[u8], lights: Option<&[DirLight]>, map: Option<&crate::occlusion::OccMap>) -> R<(f32, Vec<TfragLods>)> {
	let table_offset = u32_at(data, 0)? as usize;
	let count = u32_at(data, 4)? as usize;
	let table = span(data, table_offset, count * FRAGMENT_SIZE)?;
	let mut out = Vec::with_capacity(count);
	for (i, h) in table.chunks_exact(FRAGMENT_SIZE).enumerate() {
		let (_, lods) = fragment_all(data, table_offset, h, 0, lights, format!("Terrain_{i:03}"), true).map_err(|e| format!("terrain fragment {i}: {e}"))?;
		out.push(lods.ok_or("terrain fragment without LOD tables")?);
	}
	if let Some(m) = map {
		let ids: Vec<u8> = out.iter().map(|l| l.cull.id).collect();
		for (l, b) in out.iter_mut().zip(crate::occlusion::tfrag_bytes(m, &ids)) {
			l.cull.occ = b;
		}
	}
	Ok((f32_at(data, 8)?, out))
}

/// The LOD distances in metres (6 B, 4 B, 2 B) and as the retail integer band (trunc of the distance in 1/1024 m).
pub fn lod_distances(b: f32) -> ([f32; 3], [i32; 3]) {
	let t = [6.0 * b, 4.0 * b, 2.0 * b];
	(t, t.map(|x| (x * 1024.0) as i32))
}

/// The 500 m draw cap on a sphere at view depth `pz` with radius `r` (1/1024 m): culled when `512000 - (pz - r) < 0`.
pub fn beyond_draw_cap(pz: f32, r: f32) -> bool {
	512000.0 - (pz - r) < 0.0
}

/// The TfragProc packet code from the sphere's near and far depth ints `n` and `f` (1/1024 m) against the band ints `t`: 6 far,
/// 8 mid with straddle collapse, 0xa mid, 0xe near with straddle collapse, 0x10 near with stage-1 morph, 0x14 near.
pub fn lod_code(force_far: bool, n: i32, f: i32, t: [i32; 3]) -> u8 {
	if force_far || t[0] - n <= 0 {
		6
	} else if t[0] - f <= 0 {
		8
	} else if t[1] - n <= 0 {
		0xa
	} else if t[1] - f <= 0 {
		0xe
	} else if t[2] - n <= 0 || t[2] - f <= 0 {
		0x10
	} else {
		0x14
	}
}

/// The morph weights (x, y) of a vertex at view depth `pz` in the band `lo`..`hi` (x = 0.5 t, y = 1 - t).
pub fn morph_weights(pz: f32, lo: f32, hi: f32) -> (f32, f32) {
	let t = ((pz - lo) / (hi - lo)).clamp(0.0, 1.0);
	(0.5 * t, 1.0 - t)
}

/// The vertices of the picked mesh after the morph and the straddle collapse: per vertex position, RGBA colour and texture
/// coordinates.
pub struct Posed {
	pub pos: Vec<[f32; 3]>,
	pub col: Vec<[f32; 4]>,
	pub uv: Vec<[f32; 2]>,
}

/// Poses fragment `l` for packet `code` (see `lod_code`), `t` the band distances in metres (T0, T1, T2), `cc60` the clip-w depth
/// scale, `cols` the current colour of every position record and `z` the view depth of a world point (metres). Stage 0
/// morphs in codes 8, 0xa and 0xe (band T1..T0), stage 1 in 0x10 and 0xe (band T2..T1) from the already morphed stage-0
/// parents; the codes 8 and 0xe then collapse a vertex whose two parents are both past the band edge onto parent B (REN-CULL-TFRAG-SNAP).
pub fn pose(l: &TfragLods, code: u8, t: [f32; 3], cc60: f32, cols: &[[f32; 4]], z: &dyn Fn([f32; 3]) -> f32) -> Posed {
	let nv = match code {
		6 => l.far.positions.len(),
		8 | 0xa => l.mid.positions.len(),
		_ => l.near.positions.len(),
	};
	let (mut pos, mut col) = (l.pos_world.clone(), cols.to_vec());
	let bands = [(matches!(code, 8 | 0xa | 0xe), t[1], t[0]), (matches!(code, 0xe | 0x10), t[2], t[1])];
	for (st, (on, lo, hi)) in bands.into_iter().enumerate() {
		if !on {
			continue;
		}
		for &[k, a, b] in &l.morph[st] {
			let (k, a, b) = (k as usize, a as usize, b as usize);
			let (x, y) = morph_weights(z(l.pos_world[k]), lo, hi);
			pos[k] = std::array::from_fn(|j| x * (pos[a][j] + pos[b][j]) + y * l.pos_world[k][j]);
			col[k] = std::array::from_fn(|j| x * (col[a][j] + col[b][j]) + y * cols[k][j]);
		}
	}
	let mut out = Posed {
		pos: (0..nv).map(|v| pos[l.vert_pos[v] as usize]).collect(),
		col: (0..nv).map(|v| col[l.vert_pos[v] as usize]).collect(),
		uv: l.near.uvs[..nv].to_vec(),
	};
	let snap = match code {
		8 => Some((0, t[0])),
		0xe => Some((1, t[1])),
		_ => None,
	};
	if let Some((st, edge)) = snap {
		let tp = edge * cc60;
		let mut cur: Vec<u32> = l.vert_pos[..nv].to_vec();
		let past = |d: f32| d < 0.0 && d.is_normal();
		for &[i, a, b] in &l.snap[st] {
			let (i, b) = (i as usize, b as usize);
			if i >= nv || b >= nv {
				continue;
			}
			let (wa, wb) = (cc60 * z(pos[a as usize]), cc60 * z(pos[cur[b] as usize]));
			if past(wa - tp) && past(wb - tp) {
				out.pos[i] = out.pos[b];
				out.col[i] = out.col[b];
				out.uv[i] = out.uv[b];
				cur[i] = cur[b];
			}
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use crate::{gameplay, iso, level::Level, toc};

	use super::*;

	/// REN-CULL-TFRAG tests 1 and 2: the band ints and the packet codes.
	#[test]
	fn lod_band_and_codes() {
		let (t, ints) = lod_distances(10.0);
		assert_eq!((t, ints), ([60.0, 40.0, 20.0], [61440, 40960, 20480]));
		let code = |pz: i32, r: i32| lod_code(false, (pz - r) * 1024, (pz + r) * 1024, ints);
		assert_eq!([code(50, 5), code(58, 5), code(70, 5), code(38, 5), code(25, 3), code(10, 3)], [0xa, 8, 6, 0xe, 0x10, 0x14]);
		// Byte +0x27 forces the far mesh at any distance.
		assert_eq!(lod_code(true, 0, 2048, ints), 6);
		// N == T0 is far (compares are `<= 0`).
		assert_eq!(lod_code(false, 61440, 70000, ints), 6);
	}

	/// REN-CULL-TFRAG test 5: the 500 m cap is exact at r = 5 m and culls at r = 4 m.
	#[test]
	fn draw_cap_is_strict() {
		assert!(beyond_draw_cap(505.0 * 1024.0, 4.0 * 1024.0));
		assert!(!beyond_draw_cap(505.0 * 1024.0, 5.0 * 1024.0));
	}

	/// REN-CULL-TFRAG test 3: weights over both bands (B = 10).
	#[test]
	fn morph_weight_bands() {
		for (pz, w) in [(40.0, (0.0, 1.0)), (50.0, (0.25, 0.5)), (55.0, (0.375, 0.25)), (60.0, (0.5, 0.0)), (61.0, (0.5, 0.0))] {
			assert_eq!(morph_weights(pz, 40.0, 60.0), w);
		}
		for (pz, w) in [(20.0, (0.0, 1.0)), (30.0, (0.25, 0.5)), (40.0, (0.5, 0.0))] {
			assert_eq!(morph_weights(pz, 20.0, 40.0), w);
		}
	}

	/// A two-parent fragment: positions 0 (A), 1 (B) common, 2 a stage-0 vertex over them; vertices 0, 1, 2 one per position.
	fn tiny(snap: Vec<[u32; 3]>) -> TfragLods {
		let mut mesh = Mesh::new("t".into());
		mesh.positions = vec![[0.0; 3]; 3];
		mesh.uvs = vec![[0.0, 0.0], [0.25, 0.5], [0.75, 0.75]];
		TfragLods {
			materials: Vec::new(),
			face_mat: Default::default(),
			far: Mesh::new("f".into()),
			mid: mesh.clone(),
			near: mesh,
			pos_world: vec![[0.0; 3], [4.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
			vert_pos: vec![0, 1, 2],
			lit: Mesh::new("l".into()),
			morph: [vec![[2, 0, 1]], vec![]],
			snap: [snap, vec![]],
			cull: TfragCull { centre: [0.0; 3], radius: 1.0, corners: [[0.0; 3]; 8], force_far: false, occ: (0x80, 0x7f), id: 0 },
		}
	}

	/// REN-CULL-TFRAG tests 4 and 7: the world-space blend and the colour blend (code 0xa, x = 0.25, y = 0.5 at pz 50).
	#[test]
	fn morph_blends_position_and_colour() {
		let l = tiny(vec![]);
		let cols = [[0.0; 4], [40.0, 0.0, 200.0, 128.0], [100.0, 50.0, 0.0, 128.0]];
		let p = pose(&l, 0xa, [60.0, 40.0, 20.0], -0.857234, &cols, &|_| 50.0);
		assert_eq!(p.pos[2], [1.5, 0.0, 0.0]);
		assert_eq!(p.col[2], [60.0, 25.0, 50.0, 96.0]);
		// Plain codes leave the vertex alone.
		let p = pose(&l, 0x14, [60.0, 40.0, 20.0], -0.857234, &cols, &|_| 50.0);
		assert_eq!(p.pos[2], [1.0, 0.0, 0.0]);
	}

	/// REN-CULL-TFRAG-SNAP tests 1 to 3: both parents past T0 collapse the vertex onto parent B, uv and colour included.
	#[test]
	fn straddle_collapse_copies_parent_b() {
		let l = tiny(vec![[2, 0, 1]]);
		let cols = [[1.0; 4], [0.0; 4], [100.0, 50.0, 0.0, 128.0]];
		let z = |a: f32, b: f32| move |p: [f32; 3]| if p[0] == 0.0 { a } else { b };
		let t = [60.0, 40.0, 20.0];
		let p = pose(&l, 8, t, -0.857234, &cols, &z(60.001, 70.0));
		assert_eq!((p.pos[2], p.col[2], p.uv[2]), (p.pos[1], [0.0; 4], [0.25, 0.5]));
		// pz(A) = 60.0: no snap; 59.999: no snap.
		for a in [60.0, 59.999] {
			let p = pose(&l, 8, t, -0.857234, &cols, &z(a, 70.0));
			assert_ne!(p.uv[2], [0.25, 0.5]);
		}
		// Code 0xa never collapses.
		let p = pose(&l, 0xa, t, -0.857234, &cols, &z(70.0, 70.0));
		assert_eq!(p.uv[2], [0.75, 0.75]);
	}

	/// REN-CULL-TFRAG-SNAP test 5: a chain reads the already snapped vertex.
	#[test]
	fn collapse_chain_reads_snapped_values() {
		let mut l = tiny(vec![[1, 0, 0], [2, 0, 1]]);
		l.morph = [vec![], vec![]];
		let cols = [[7.0; 4], [1.0; 4], [2.0; 4]];
		let p = pose(&l, 8, [60.0, 40.0, 20.0], -0.857234, &cols, &|_| 70.0);
		assert_eq!(p.col, vec![[7.0; 4]; 3]);
	}

	/// Every fragment of both levels: the three meshes are prefixes of one info list, the lists index real vertices and the
	/// box corners enclose the sphere centre; the LOD scale is the block's f32.
	#[test]
	fn lods_of_veldin_and_novalis() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for (k, count, b) in [(0, 460, 12.0), (1, 1004, 15.0)] {
			let l = Level::load(&mut disc, &levels[k]).unwrap();
			let lights = gameplay::lighting(&l).unwrap().lights;
			let map = crate::occlusion::OccMap::new(&l.gameplay);
			let (got, lods) = terrain_lods(l.terrain_block().unwrap(), Some(&lights), map.as_ref()).unwrap();
			assert_eq!((lods.len(), got), (count, b));
			let near_meshes = terrain_lit(l.terrain_block().unwrap(), 0, Some(&lights)).unwrap();
			let mut off = 0;
			for (f, m) in lods.iter().zip(&near_meshes) {
				assert_eq!(f.near.positions, m.positions);
				assert!(f.far.positions.len() <= f.mid.positions.len() && f.mid.positions.len() <= f.near.positions.len());
				assert_eq!(f.far.positions[..], f.near.positions[..f.far.positions.len()]);
				assert_eq!(f.mid.positions[..], f.near.positions[..f.mid.positions.len()]);
				assert_eq!(f.vert_pos.len(), f.near.positions.len());
				assert_eq!(f.lit.positions.len(), f.pos_world.len());
				for st in 0..2 {
					assert!(f.morph[st].iter().all(|e| e.iter().all(|&x| (x as usize) < f.pos_world.len())));
					assert!(f.snap[st].iter().all(|e| (e[0] as usize) < f.near.positions.len() && (e[2] as usize) < f.near.positions.len()));
				}
				let c = f.cull.centre;
				let lo: Vec<f32> = (0..3).map(|j| f.cull.corners.iter().map(|p| p[j]).fold(f32::MAX, f32::min)).collect();
				let hi: Vec<f32> = (0..3).map(|j| f.cull.corners.iter().map(|p| p[j]).fold(f32::MIN, f32::max)).collect();
				assert!((0..3).all(|j| lo[j] <= c[j] + 1.0 && c[j] <= hi[j] + 1.0), "corners {lo:?} {hi:?} centre {c:?}");
				off += 1;
			}
			assert_eq!(off, count);
			println!("level {k}: far/mid/near {} / {} / {} vertices in total, snaps {}", lods.iter().map(|f| f.far.positions.len()).sum::<usize>(), lods.iter().map(|f| f.mid.positions.len()).sum::<usize>(), lods.iter().map(|f| f.near.positions.len()).sum::<usize>(), lods.iter().map(|f| f.snap[0].len() + f.snap[1].len()).sum::<usize>());
		}
	}

	#[test]
	fn veldin_baked_colours() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[0]).unwrap();
		let lights = gameplay::lighting(&l).unwrap().lights;
		let meshes = super::terrain_lit(l.terrain_block().unwrap(), 0, Some(&lights)).unwrap();
		assert_eq!(meshes.len(), 460);
		// The dynamic-light sphere (header / 1024) encloses the vertices of its fragment (the units check), and normals are unit.
		for m in &meshes {
			let l = m.lit.as_ref().unwrap();
			assert_eq!(l.normals.len(), m.positions.len());
			for p in &m.positions {
				let d = ((p[0] - l.sphere[0]).powi(2) + (p[1] - l.sphere[1]).powi(2) + (p[2] - l.sphere[2]).powi(2)).sqrt();
				// A few LOD-0 refinement vertices poke out of the sphere by under 10%.
				assert!(d <= l.sphere[3] * 1.1 + 0.01, "{} outside sphere by {}", m.name, d - l.sphere[3]);
			}
		}
		for m in &meshes {
			assert_eq!(m.colours.len(), m.positions.len());
			assert!(m.colours.iter().all(|c| c[3] == 128.0 && c[..3].iter().all(|&x| (0.0..=255.0).contains(&x))));
		}
		let avg: Vec<f32> = (0..3).map(|j| meshes.iter().flat_map(|m| &m.colours).map(|c| c[j]).sum::<f32>() / meshes.iter().map(|m| m.colours.len()).sum::<usize>() as f32).collect();
		println!("average terrain colour {avg:?}");
	}
	/// Eudora (level 4) terrain CLAMP_1: the per-mode record counts equal the retail RAM records of state play/eudora1_0039
	/// (A+D TEX0 / TEX1 / CLAMP / MIPTBP1 / MIPTBP2 qwords: 649 repeat, 71 clamp u, 762 clamp v, 116 both; fx_census_late.md run 21).
	#[test]
	fn eudora_material_wrap() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[4]).unwrap();
		let (_, frags) = terrain_lods(l.terrain_block().unwrap(), None, None).unwrap();
		let all: Vec<TfragMaterial> = frags.iter().flat_map(|f| f.materials.iter().copied()).collect();
		let n = |w: u32| all.iter().filter(|m| m.wrap == w).count();
		assert_eq!((all.len(), n(0), n(1), n(2), n(3)), (1598, 649, 71, 762, 116));
	}
	/// REN-TEX-RESIDENCY tests (2) (3): the material records of Novalis (level 1).
	#[test]
	fn novalis_material_mip_state() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let (_, frags) = terrain_lods(l.terrain_block().unwrap(), None, None).unwrap();
		let all: Vec<TfragMaterial> = frags.iter().flat_map(|f| f.materials.iter().copied()).collect();
		assert_eq!(all.len(), 2167);
		assert!(all.iter().all(|m| m.mmin == 4));
		assert_eq!(frags[0].materials[0].k, -7.8125);
		let common = all.iter().filter(|m| m.k == -8.5625).count();
		assert_eq!(common, 736);
		let chains = l.terrain_mips().unwrap();
		let size = |m: &TfragMaterial| {
			let c = chains[m.texture as usize].as_ref().unwrap();
			(c[0].w, c.len())
		};
		assert_eq!(all.iter().filter(|m| size(m) == (128, 4)).count(), 2093);
		assert_eq!(all.iter().filter(|m| size(m) == (64, 4)).count(), 72);
		assert_eq!(all.iter().filter(|m| size(m) == (32, 3)).count(), 2);
		// Is K a function of the texture alone?
		let mut by_tex: std::collections::BTreeMap<u32, std::collections::BTreeSet<i32>> = Default::default();
		for m in &all {
			by_tex.entry(m.texture).or_default().insert((m.k * 16.0) as i32);
		}
		let multi = by_tex.values().filter(|s| s.len() > 1).count();
		println!("{} textures, {} with more than one K, {} distinct (texture, K)", by_tex.len(), multi, by_tex.values().map(|s| s.len()).sum::<usize>());
	}
	/// REN-TEX-RESIDENCY test (4): fragment 0's first texture sphere (the residency request list entry the GS mip state depends
	/// on) and its two distances. The port keeps no residency state, so the numbers are checked on the authored data only.
	#[test]
	fn novalis_first_texture_sphere() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let data = l.terrain_block().unwrap();
		let table_offset = u32_at(data, 0).unwrap() as usize;
		let header = &data[table_offset..table_offset + FRAGMENT_SIZE];
		let start = table_offset + u32_at(header, 0x10).unwrap() as usize;
		let at = start + u16_at(header, 0x2e).unwrap() as usize;
		let c: Vec<f32> = (0..3).map(|k| f32_at(data, at + k * 4).unwrap() / 1024.0).collect();
		let (r, b, tex) = (u16_at(data, at + 0xc).unwrap() as f32 / 1024.0, data[at + 0xe] as f32, data[at + 0xf]);
		assert_eq!(c, vec![292.62503, 142.31332, 82.875]);
		assert_eq!((r, b as u32, tex), (1.2548828, 156, 13));
		// Request below 128 b, full at 64 b (vf22.x = 0.5), both in 1/1024 m.
		assert_eq!(128.0 * b, 19968.0);
		assert_eq!(128.0 * b * 0.5, 9984.0);
	}
}
