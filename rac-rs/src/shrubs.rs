//! Shrubs: small static foliage meshes. Class meshes from their VIF packet programs, and the PAL
//! placements from gameplay data.
//!
//! Like ties, a shrub packet stores the GS writes VU1 would make, not triangles: we replay them in
//! GS address order (a GIF tag costs 1 qword, a material 5, a vertex 3; short packets repeat their
//! last vertex as padding). Winding is not stored, so each face faces the average of its vertices'
//! stored normals. The far-LOD billboard block is read by `lit_instances` (REN-CULL-SHRUB-BB). Baked colours: `lit_instances`.
//! Layout: rac1-decomp tools/extract/shrubs.py (Wrench's shrub reader).

use crate::bytes::{f32_at, i16_at, i32_at, span, u32_at, R};
use crate::level::{texture_key, Level, TextureGroup};
use crate::mesh::{Lit, LitKind, Mesh};
use crate::gameplay::{baked_raw, select_light, DirLight, TIE_CAP};
use crate::ties::{instance_sphere, placement, transformed, unit};
use std::collections::HashMap;

const STCYCL: u8 = 0x01;
const STMOD: u8 = 0x05;
const GIF_REGS: u32 = 0x412; // ST, RGBAQ, XYZ2 per vertex.
/// Face texture keys carry the material's GS CLAMP wrap in bits 26..28 (bit 26 WMS clamp, bit 27 WMT clamp; 0 = repeat).
/// Material record +0x10 / +0x14 = WMS / WMT source words, runtime CLAMP_1 = WMS | WMT << 2 (vertex_colours.md) [C].
/// Eudora: classes 42, 57 (flat dirt decals), 78-81 clamp both, class 58 one material WMT only [C].
pub const WRAP_SHIFT: u32 = 26;
/// Shrub face keys also carry the material's GS TEX1 K (record +0x00, signed 12 bits, 1/16 mip): low 8 bits in key bits
/// 8..16, high 4 bits in bits 28..32 (shrub texture indices are below 256). Shrub class setup (Veldin FUN_00240a68 twin,
/// OV:51666) builds TEX1 = (texture entry +8 mips - 1) << 2 (MXL) | record +0x04 (MMIN, 4 on every level) << 6 | 0x20
/// (MMAG linear) | K << 32 [C]; MXL comes from the texture's mip count (`Level::mip_chains`).
pub const K_LO_SHIFT: u32 = 8;
pub const K_HI_SHIFT: u32 = 28;
/// Every material bit of a shrub face key (wrap and K): stripping them leaves the texture key.
pub const MATERIAL_BITS: u32 = 3 << WRAP_SHIFT | 0xff << K_LO_SHIFT | 0xf << K_HI_SHIFT;

/// The raw 12-bit TEX1 K of a shrub or tie face key (None for other groups; tie keys carry it the same way).
pub fn key_k(key: u32) -> Option<u32> {
	matches!((key & crate::mobies::KEY_MASK) >> 16, 2 | 3).then(|| (key >> K_LO_SHIFT & 0xff) | (key >> K_HI_SHIFT & 0xf) << 8)
}

struct Packet<'a> {
	cmd: u8,
	imm: u16,
	count: usize,
	data: &'a [u8],
}

/// Frame a VIF command list (UNPACK V4-32 and V4-16 only), keeping UNPACK payloads packed.
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
			0x6c => 16 * count,
			0x6d => 8 * count,
			0 | STCYCL | STMOD => 0,
			c => return Err(format!("unsupported shrub VIF command {c:#x}")),
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

fn quad(data: &[u8], i: usize) -> R<[i16; 4]> {
	Ok([i16_at(data, i * 8)?, i16_at(data, i * 8 + 2)?, i16_at(data, i * 8 + 4)?, i16_at(data, i * 8 + 6)?])
}

fn finish(mesh: &mut Mesh, normals: &[[i16; 3]], primitive: &[u32], expected: usize, kind: u64, texture: Option<u32>) -> R<()> {
	if primitive.len() != expected {
		return Err("shrub GIF vertex count mismatch".into());
	}
	if primitive.is_empty() {
		return Ok(());
	}
	if kind == 3 && primitive.len() % 3 != 0 {
		return Err("incomplete shrub triangle list".into());
	}
	let texture = texture.ok_or("shrub face without material")?;
	let step = if kind == 3 { 3 } else { 1 };
	for i in (0..primitive.len() - 2).step_by(step) {
		let [a, b, c] = [primitive[i], primitive[i + 1], primitive[i + 2]];
		let [pa, pb, pc] = [a, b, c].map(|v| mesh.positions[v as usize].map(|x| x as f64));
		let ab: [f64; 3] = std::array::from_fn(|j| pb[j] - pa[j]);
		let ac: [f64; 3] = std::array::from_fn(|j| pc[j] - pa[j]);
		let cross = [ab[1] * ac[2] - ab[2] * ac[1], ab[2] * ac[0] - ab[0] * ac[2], ab[0] * ac[1] - ab[1] * ac[0]];
		if cross.iter().map(|x| x * x).sum::<f64>() < 1e-24 {
			continue; // Zero-area faces are strip joins.
		}
		let dot: f64 = (0..3).map(|j| cross[j] * (normals[a as usize][j] as f64 + normals[b as usize][j] as f64 + normals[c as usize][j] as f64)).sum();
		mesh.faces.push((texture, if dot < 0.0 { [c, b, a] } else { [a, b, c] }));
	}
	Ok(())
}


/// The class mesh and each vertex's class normal (one of the 24 signed i16 normals at +0x2c).
fn shrub_mesh_normals(data: &[u8], remap: &[u8], name: String) -> R<(Mesh, Vec<[i16; 3]>)> {
	let header = span(data, 0, 64)?;
	let scale = f32_at(header, 0x20)?;
	let count = i16_at(header, 0x28)?;
	if !scale.is_finite() || scale <= 0.0 || count < 0 {
		return Err("invalid shrub scale or packet count".into());
	}
	let table = span(data, 64, count as usize * 8)?;
	let normal_table = span(data, u32_at(header, 0x2c)? as usize, 24 * 8)?;
	let normal_table: Vec<[i16; 4]> = (0..24).map(|i| quad(normal_table, i)).collect::<R<_>>()?;
	let mut mesh = Mesh::new(name);
	let mut normals: Vec<[i16; 3]> = Vec::new();
	for (packet_id, entry) in table.chunks_exact(8).enumerate() {
		let (offset, size) = (i32_at(entry, 0)?, i32_at(entry, 4)?);
		if offset < 0 || size < 0 {
			return Err("invalid shrub packet range".into());
		}
		let p = program(span(data, offset as usize, size as usize)?, &[STCYCL, STMOD, 0x6c, STMOD, 0x6d, STMOD, 0x6d])?;
		if (p[0].imm, p[1].imm, p[3].imm, p[5].imm) != (0x404, 0, 0, 0) {
			return Err("unsupported shrub VIF cycle or mode".into());
		}
		let (control, xyz, st) = (&p[2], &p[4], &p[6]);
		let (materials, tag_count, vertex_count, vertex_offset) = (i32_at(control.data, 0)?, i32_at(control.data, 4)?, i32_at(control.data, 8)?, i32_at(control.data, 12)?);
		if materials.min(tag_count).min(vertex_count) <= 0 {
			return Err("invalid shrub packet counts".into());
		}
		let (materials, tag_count, vertex_count) = (materials as usize, tag_count as usize, vertex_count as usize);
		if control.data.len() != 16 + tag_count * 16 + materials * 64
			|| vertex_offset as usize != control.data.len() / 16
			|| xyz.count != vertex_count
			|| st.count != vertex_count
			|| (control.imm as usize, xyz.imm as usize, st.imm as usize) != (0x8000, 0x8000 + vertex_offset as usize, 0x8000 + vertex_offset as usize + vertex_count)
			|| vertex_offset as usize + 2 * vertex_count > 1024
		{
			return Err("shrub VIF counts and addresses disagree".into());
		}
		// (tag, registers, GS address)
		let tags: Vec<(u64, u32, i64)> = (0..tag_count)
			.map(|i| Ok((u32_at(control.data, 16 + i * 16)? as u64 | (u32_at(control.data, 20 + i * 16)? as u64) << 32, u32_at(control.data, 24 + i * 16)?, u32_at(control.data, 28 + i * 16)? as i64)))
			.collect::<R<_>>()?;
		let records: Vec<&[u8]> = (0..materials).map(|i| span(control.data, 16 + tag_count * 16 + i * 64, 64)).collect::<R<_>>()?;
		let positions: Vec<[i16; 4]> = (0..vertex_count).map(|i| quad(xyz.data, i)).collect::<R<_>>()?;
		let coords: Vec<[i16; 4]> = (0..vertex_count).map(|i| quad(st.data, i)).collect::<R<_>>()?;
		let (mut ti, mut mi, mut vi, mut address) = (0, 0, 0, 0i64);
		let (mut texture, mut primitive, mut expected, mut kind): (Option<u32>, Vec<u32>, usize, u64) = (None, Vec::new(), 0, 0);
		while ti < tag_count || mi < materials || vi < vertex_count {
			if ti < tag_count && tags[ti].2 == address {
				finish(&mut mesh, &normals, &primitive, expected, kind, texture)?;
				let (tag, regs, _) = tags[ti];
				kind = (tag >> 47) & 7; // GS PRIM: 3 triangles, 4 triangle strip.
				if ![3, 4].contains(&kind) || (tag >> 58) & 3 != 0 || tag >> 60 != 3 || regs != GIF_REGS {
					return Err("unsupported shrub GIF primitive".into());
				}
				expected = (tag & 0x7fff) as usize;
				if expected < 3 {
					return Err("short shrub GIF primitive".into());
				}
				primitive.clear();
				ti += 1;
				address += 1;
			} else if mi < materials && i32_at(records[mi], 12)? as i64 == address {
				finish(&mut mesh, &normals, &primitive, expected, kind, texture)?;
				primitive.clear();
				expected = 0;
				let slot = u32_at(records[mi], 48)? as usize; // TEX0 data, patched at load time.
				if slot >= remap.len() || remap[slot] == 255 {
					return Err("shrub material has no texture".into());
				}
				let (wms, wmt) = (u32_at(records[mi], 16)?, u32_at(records[mi], 20)?);
				if crate::viewer::gs_state::enabled() {
					let (k, mmin) = (u32_at(records[mi], 0)? as u64, u32_at(records[mi], 4)? as u64);
					crate::viewer::gs_state::guard_world("shrub", crate::viewer::gs_state::TEST_SHRUB, (k & 0xfff) << 32 | mmin << 6 | 0x20, wms as u64 | (wmt as u64) << 2);
				}
				if wms > 1 || wmt > 1 {
					return Err("unsupported shrub CLAMP region mode".into());
				}
				// TEX1 source: +0x00 K, +0x04 MMIN (LINEAR_MIPMAP_NEAREST), +0x08 register 0x14 [C, every level].
				let (k, mmin) = (u32_at(records[mi], 0)? & 0xfff, u32_at(records[mi], 4)?);
				if mmin != 4 || u32_at(records[mi], 8)? != 0x14 {
					return Err("unsupported shrub TEX1 source".into());
				}
				texture = Some(texture_key(TextureGroup::Shrub, remap[slot] as u32) | (wms | wmt << 1) << WRAP_SHIFT | (k & 0xff) << K_LO_SHIFT | (k >> 8) << K_HI_SHIFT);
				mi += 1;
				address += 5;
			} else if vi < vertex_count && positions[vi][3] as i64 == address {
				if expected == 0 || texture.is_none() {
					return Err("shrub vertex precedes its GIF tag or material".into());
				}
				let [x, y, z, _] = positions[vi];
				let [s, t, q, n] = coords[vi];
				let n = (n as u16 & 0x7fff) as usize;
				if n >= 24 || q != 4096 {
					return Err("invalid shrub normal or texture Q".into());
				}
				primitive.push(mesh.positions.len() as u32);
				mesh.positions.push([x, y, z].map(|c| c as f32 * scale / 1024.0));
				mesh.uvs.push([s as f32 / 4096.0, t as f32 / 4096.0]);
				normals.push([normal_table[n][0], normal_table[n][1], normal_table[n][2]]);
				vi += 1;
				address += 3;
			} else if vi > 0
				&& ti == tag_count
				&& mi == materials
				&& (vi..vertex_count).all(|k| positions[k] == positions[vi - 1] && coords[k][..3] == coords[vi - 1][..3] && coords[k][3] as u16 & 0x7fff == coords[vi - 1][3] as u16 & 0x7fff)
			{
				vi = vertex_count; // Padding: copies of the last vertex.
			} else {
				return Err(format!("shrub packet {packet_id}: missing GS event at {address}"));
			}
		}
		finish(&mut mesh, &normals, &primitive, expected, kind, texture)?;
	}
	Ok((mesh, normals))
}



/// The 24 signed class normals at class block +0x2c (xyz of each 8-byte entry): what the shrub's light words are computed from.
pub fn class_normals(data: &[u8]) -> R<Vec<[i16; 3]>> {
	let table = span(data, u32_at(data, 0x2c)? as usize, 24 * 8)?;
	(0..24).map(|i| quad(table, i).map(|q| [q[0], q[1], q[2]])).collect()
}

/// Level-init global cap on every shrub draw distance (m).
pub const SHRUB_CAP: f32 = 500.0;

/// What the per-frame shrub cull needs of one instance (REN-CULL-SHRUB): bounding sphere, the authored draw distance and the
/// billboard switch distance of its class (byte B, None = no billboard).
#[derive(Clone, Copy, Debug)]
pub struct ShrubCull {
	pub centre: [f32; 3],
	pub radius: f32,
	pub dist: f32,
	pub bb: Option<u8>,
}

/// One shrub's verdict for a frame: `mesh` None = the mesh is not drawn, Some(0x80) opaque, Some(a < 0x80) faded with alpha
/// a / 128; `bb` the billboard alpha (0 = none, 0x80 = opaque, in between blended).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShrubState {
	pub mesh: Option<u8>,
	pub bb: u8,
}

impl ShrubCull {

	/// `d_eff` under the live cap word (retail mode reads it from RAM: 144 during the Visibomb flight).
	pub fn d_eff_cap(&self, cap: f32) -> f32 {
		let mut d = if self.dist < 16.0 { 16.0 } else { self.dist };
		if let Some(b) = self.bb {
			d = d.max(b as f32 + 24.0);
		}
		d.min(cap)
	}


	/// `state` under the live cap word.
	pub fn state_cap(&self, pz: f32, cap: f32) -> Option<ShrubState> {
		let (d, r) = (self.d_eff_cap(cap), self.radius);
		let y = (d - r) - (pz - r);
		if y < 0.0 || y.is_nan() {
			return None;
		}
		let v = (y * 4096.0) as i32;
		let mut s = ShrubState { mesh: Some(if v >= 0x8000 { 0x80 } else { (v >> 8) as u8 }), bb: 0 };
		if let Some(b) = self.bb {
			let from = b as i32 * 4096;
			let z = (pz.max(0.0) * 4096.0) as i32;
			if z >= from {
				let a = ((z - from).min(0x8000) >> 8) as u8;
				if a == 0x80 {
					s.mesh = None;
					s.bb = ((v >> 1).min(0x8000) >> 8) as u8;
				} else {
					s.bb = a;
				}
			}
		}
		Some(s)
	}
}

/// The camera-facing quad of a far-LOD shrub (REN-CULL-SHRUB-BB): corner k = origin + u (y_k 2 half_w) + up (z_k h + z0), with
/// (y, z) = (-0.5, 1), (0.5, 1), (-0.5, 0), (0.5, 0).
#[derive(Clone, Copy, Debug)]
pub struct ShrubBillboard {
	pub origin: [f32; 3],
	pub half_w: f32,
	pub h: f32,
	pub z0: f32,
	pub colour: [u8; 3],
	pub class_id: u32,
	/// GS mip bias of the class texture (TEX1 K, mip levels).
	pub lod_k: f32,
}

impl ShrubBillboard {
	/// Corners c0..c3 for a camera at `cam`: the axis is the horizontal turn of the unit vector to the camera, not renormalised.
	pub fn corners(&self, cam: [f32; 3]) -> [[f32; 3]; 4] {
		let d = [self.origin[0] - cam[0], self.origin[1] - cam[1], self.origin[2] - cam[2]];
		let l2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
		let inv = if l2 > 0.0 { 1.0 / l2.sqrt() } else { 0.0 };
		let n = [d[0] * inv, d[1] * inv];
		let u = [n[1], -n[0]];
		[(-0.5, 1.0), (0.5, 1.0), (-0.5, 0.0), (0.5, 0.0)].map(|(y, z): (f32, f32)| {
			let side = y * 2.0 * self.half_w;
			[self.origin[0] + u[0] * side, self.origin[1] + u[1] * side, self.origin[2] + z * self.h + self.z0]
		})
	}
}

/// The packed scale word of a shrub's matrix (low half hs, high half vs, both /4096): hs = mean of the first two row lengths x 4096
/// capped at 0x10000, vs = third row length x 4096 or 0 above 0x10000; a hs of 0x10000 carries into the high half like the
/// retail integer add. Returns (hs, vs) as the halves.
pub fn scale_halves(l0: f32, l1: f32, l2: f32) -> (u32, u32) {
	let hs = (((l0 + l1) * 0.5 * 4096.0) as i32).min(0x10000) as u32;
	let mut vs = (l2 * 4096.0) as i32 as u32;
	if vs > 0x10000 {
		vs = 0;
	}
	let word = hs.wrapping_add(vs << 16);
	(word & 0xffff, word >> 16)
}

/// Per-channel mean (integer division) of the 24 class light colours.
pub fn average_colour(cols: &[[u8; 3]]) -> [u8; 3] {
	let mut sum = [0u32; 3];
	for c in cols {
		for k in 0..3 {
			sum[k] += c[k] as u32;
		}
	}
	sum.map(|s| (s / 24) as u8)
}

/// The class's billboard block (class +0x1c): switch distance byte, quad size terms and the GS mip bias.
struct ClassBillboard {
	b: u8,
	y: f32,
	z: f32,
	w: f32,
	lod_k: f32,
}

/// A shrub instance: its world mesh, cull data, billboard and the class's slot in the load table (retail walks classes in slot order).
pub struct ShrubInstance {
	pub mesh: Mesh,
	pub cull: ShrubCull,
	pub bb: Option<ShrubBillboard>,
	pub class_slot: u32,
	/// [PC] vegfx preview: the shrub class id.
	pub class_id: u32,
}

/// World-space shrub instances with baked vertex colours (decomp/notes/vertex_colours.md): base
/// `(col.r, col.g, col.b, 0x80)` from record +0x50, the class normal turned by the instance axes,
/// and the directional pair picked by the selector at +0x60.
pub fn lit_instances(level: &Level, lights: &[DirLight]) -> R<Vec<ShrubInstance>> {
	let mut classes = HashMap::new();
	for (slot, e) in level.table(0x28, 48)?.into_iter().enumerate() {
		let (start, class_id) = (u32_at(e, 0)? as usize, u32_at(e, 4)?);
		let data = level.block(start)?;
		let (mesh, normals) = shrub_mesh_normals(data, &e[16..32], format!("shrub_{class_id}")).map_err(|x| format!("shrub class {class_id}: {x}"))?;
		let sphere = [f32_at(data, 0)?, f32_at(data, 4)?, f32_at(data, 8)?, f32_at(data, 0xc)?, f32_at(data, 0x20)?];
		let bb = match u32_at(data, 0x1c)? as usize {
			0 => None,
			off => {
				let blk = data.get(off..).ok_or(format!("shrub class {class_id}: billboard block out of range"))?;
				let k = (u32_at(blk, 0x10)? & 0xfff) as i32;
				let k = if k >= 0x800 { k - 0x1000 } else { k };
				Some(ClassBillboard { b: f32_at(blk, 0)? as i32 as u8, y: f32_at(blk, 4)?, z: f32_at(blk, 8)?, w: f32_at(blk, 0xc)?, lod_k: k as f32 / 16.0 })
			}
		};
		classes.insert(class_id, (mesh, normals, sphere, class_normals(data)?, bb, slot as u32));
	}
	let g = &level.gameplay;
	let start = u32_at(g, 0x3c)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	span(g, start + 16, count * 0x70)?
		.chunks_exact(0x70)
		.enumerate()
		.map(|(i, r)| {
			let class_id = u32_at(r, 0)?;
			let (class, normals, cs, table, cbb, class_slot) = classes.get(&class_id).ok_or(format!("shrub instance {i}: unknown class {class_id}"))?;
			let matrix = placement(r, "shrub instance")?;
			let mut m = transformed(class, &matrix, format!("shrub_instance_{i}"));
			let light = select_light(lights, u32_at(r, 0x60)?);
			let base = [u32_at(r, 0x50)? as u8, u32_at(r, 0x54)? as u8, u32_at(r, 0x58)? as u8, 0x80].map(|x| x as f32);
			let turn = |n: [i16; 3]| {
				let n = n.map(|c| c as f32 / 32767.0);
				unit(std::array::from_fn(|r| matrix[r] * n[0] + matrix[4 + r] * n[1] + matrix[8 + r] * n[2]))
			};
			let wn: Vec<[f32; 3]> = normals.iter().map(|&n| turn(n)).collect();
			let raw: Vec<[f32; 4]> = wn.iter().map(|&n| baked_raw(&light, base, n)).collect();
			m.colours = raw.iter().map(|c| [c[0].clamp(0.0, TIE_CAP), c[1].clamp(0.0, TIE_CAP), c[2].clamp(0.0, TIE_CAP), c[3]]).collect();
			let sphere = instance_sphere(&matrix, [cs[0], cs[1], cs[2]], cs[3], cs[4]);
			m.lit = Some(Lit { kind: LitKind::Shrub, sphere, origin: [matrix[12], matrix[13], matrix[14]], cap: TIE_CAP, normals: wn, raw });
			let cull = ShrubCull { centre: [sphere[0], sphere[1], sphere[2]], radius: sphere[3], dist: f32_at(r, 4)?, bb: cbb.as_ref().map(|b| b.b) };
			let bb = cbb.as_ref().map(|b| {
				let len = |o: usize| (matrix[o] * matrix[o] + matrix[o + 1] * matrix[o + 1] + matrix[o + 2] * matrix[o + 2]).sqrt();
				let (hs, vs) = scale_halves(len(0), len(4), len(8));
				let (hs, vs) = (hs as f32 / 4096.0, vs as f32 / 4096.0);
				let s = cs[4];
				// The 24 light words of the class normals under this instance's light, per channel mean.
				let cols: Vec<[u8; 3]> = table
					.iter()
					.map(|&n| {
						let c = baked_raw(&light, base, turn(n));
						[c[0].clamp(0.0, TIE_CAP) as u8, c[1].clamp(0.0, TIE_CAP) as u8, c[2].clamp(0.0, TIE_CAP) as u8]
					})
					.collect();
				ShrubBillboard { origin: [matrix[12], matrix[13], matrix[14]], half_w: 0.5 * b.y * hs * s / 1024.0, h: b.z * vs * s / 1024.0, z0: b.w * vs * s / 1024.0, colour: average_colour(&cols), class_id, lod_k: b.lod_k }
			});
			Ok(ShrubInstance { mesh: m, cull, bb, class_slot: *class_slot, class_id })
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{iso, toc};
	use std::path::PathBuf;

	/// The instance bounding sphere (class sphere x placement) encloses the instance's vertices on Veldin and Novalis.
	#[test]
	fn instance_spheres_enclose_the_mesh() {
		let mut disc = iso::Disc::open(&PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for info in &levels[..2] {
			let level = Level::load(&mut disc, info).unwrap();
			let lights = crate::gameplay::lighting(&level).unwrap().lights;
			let (mut bad, mut n) = (0, 0);
			for m in lit_instances(&level, &lights).unwrap().into_iter().map(|s| s.mesh) {
				let s = m.lit.as_ref().unwrap().sphere;
				let far = m.positions.iter().map(|p| ((p[0] - s[0]).powi(2) + (p[1] - s[1]).powi(2) + (p[2] - s[2]).powi(2)).sqrt()).fold(0.0f32, f32::max);
				n += 1;
				bad += (far > s[3] * 1.01 + 0.1) as usize;
			}
			println!("level {}: {bad} of {n} shrub spheres do not enclose their vertices", info.id);
			assert_eq!(bad, 0);
		}
	}

	#[test]
	fn veldin_shrub_colour() {
		let mut disc = iso::Disc::open(&PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let lights = crate::gameplay::lighting(&level).unwrap().lights;
		let shrubs = lit_instances(&level, &lights).unwrap();
		// vertex_colours.md "Veldin checks": shrub 0, first vertex (39.20, 36.13, 30.95, 128).
		let c = shrubs[0].mesh.colours[0];
		println!("shrub 0 vertex 0 {c:?}");
		assert!((c[0] - 39.20).abs() < 0.5 && (c[1] - 36.13).abs() < 0.5 && (c[2] - 30.95).abs() < 0.5 && c[3] == 128.0);
	}
	fn cull(dist: f32, bb: Option<u8>) -> ShrubCull {
		ShrubCull { centre: [0.0; 3], radius: 1.5, dist, bb }
	}

	/// REN-CULL-SHRUB-BB tests (1)-(3).
	#[test]
	fn billboard_corners_scale_and_colour() {
		let b = ShrubBillboard { origin: [0.0; 3], half_w: 1.0, h: 3.0, z0: 0.0, colour: [0; 3], class_id: 0, lod_k: 0.0 };
		let c = b.corners([10.0, 0.0, 2.0]);
		let want = [[0.0, -0.9805807, 3.0], [0.0, 0.9805807, 3.0], [0.0, -0.9805807, 0.0], [0.0, 0.9805807, 0.0]];
		for k in 0..4 {
			for j in 0..3 {
				assert!((c[k][j] - want[k][j]).abs() < 1e-6, "corner {k} axis {j}: {}", c[k][j]);
			}
		}
		assert_eq!(scale_halves(1.5, 1.5, 2.0), (6144, 8192));
		// hs 0x10000 overflows into vs; a vertical scale above 16 gives 0.
		assert_eq!(scale_halves(16.0, 16.0, 1.0), (0, 4097));
		assert_eq!(scale_halves(1.0, 1.0, 17.0), (4096, 0));
		let cols: Vec<[u8; 3]> = (0..24).map(|i| [i, 0, 0]).collect();
		assert_eq!(average_colour(&cols), [11, 0, 0]);
	}

	/// Retail walks one run per class from the first record whose class changes, so records are grouped by class: the class
	/// load slots never decrease in record order (BRIDGE-27 C362).
	#[test]
	fn class_slots_are_grouped_on_both_levels() {
		let mut disc = iso::Disc::open(&PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for info in &levels[..2] {
			let level = Level::load(&mut disc, info).unwrap();
			let lights = crate::gameplay::lighting(&level).unwrap().lights;
			let shrubs = lit_instances(&level, &lights).unwrap();
			let slots: Vec<u32> = shrubs.iter().map(|s| s.class_slot).collect();
			let grouped = slots.windows(2).all(|w| w[0] <= w[1]);
			let with_bb = shrubs.iter().filter(|s| s.bb.is_some()).count();
			let zero_b = shrubs.iter().filter(|s| s.cull.bb == Some(0)).count();
			println!("level {}: {} shrubs, slots non-decreasing {grouped}, {with_bb} with a billboard, {zero_b} with B 0", info.id, shrubs.len());
			assert!(grouped);
			for s in shrubs.iter().filter_map(|s| s.bb) {
				assert!(s.half_w.is_finite() && s.h.is_finite() && s.z0.is_finite() && s.h >= 0.0);
			}
		}
	}
}
