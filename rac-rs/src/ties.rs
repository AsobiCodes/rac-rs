//! Ties: static instanced meshes (rocks, buildings, props). LOD-0 class meshes, and the PAL
//! placements from gameplay data.
//!
//! A tie packet does not store triangles: it stores the GS writes VU1 would make. We rebuild LOD 0
//! by replaying them in GS address order (a material costs 6 qwords, a strip GIF tag 1, a vertex 3;
//! a vertex may be written to two addresses). Missing or conflicting writes are errors.
//! Layout: rac1-decomp tools/extract/ties.py (Wrench's tie reader; 80-byte material stride
//! confirmed there against the game's tie renderer).

use crate::bytes::{f32_at, i16_at, i32_at, span, u16_at, u32_at, R};
use crate::gameplay::{baked_raw, pext5, select_light, DirLight, TIE_CAP};
use crate::level::{texture_key, Level, TextureGroup};
use crate::mesh::{Lit, LitKind, Mesh};
use std::collections::{BTreeMap, HashMap};

pub fn tie_mesh(data: &[u8], remap: &[u8], name: String) -> R<Mesh> {
	tie_mesh_indexed(data, remap, name, 0).map(|(m, ..)| m)
}

/// A tie vertex's colour slots: the three slot indices into an instance's 64 colours (x, y, z) and whether the vertex is a
/// fat (morphing) one. A regular vertex has the one slot in all three lanes (decomp/notes/novalis_enemies.md
/// REN-CULL-TIE-MCOL).
pub type TieSlot = ([usize; 3], bool);

/// A tie face key: the texture key with the material's GS TEX1 K in the shrub K bits (`shrubs::key_k`), so the draw knows its
/// mip level choice, and its GS CLAMP wrap in the shrub wrap bits (`shrubs::WRAP_SHIFT`: bit 26 WMS clamp, bit 27 WMT clamp).
/// Tie texture indices are below 256 in every level; a larger one (or no usable K) keeps the plain key and the wrap.
fn tie_key(texture: u32, k: Option<u32>, wrap: u32) -> u32 {
	let key = texture_key(TextureGroup::Tie, texture) | (wrap & 3) << crate::shrubs::WRAP_SHIFT;
	match k {
		Some(k) if texture < 256 => key | (k & 0xff) << crate::shrubs::K_LO_SHIFT | (k >> 8) << crate::shrubs::K_HI_SHIFT,
		_ => key,
	}
}

/// The GS CLAMP wrap of a tie material record (80 bytes): the fourth A+D qword is +0x30 WMS, +0x34 WMT, +0x38 register 8
/// (CLAMP_1), as the shrub record's +0x10 / +0x14 source words (runtime CLAMP_1 = WMS | WMT << 2). 0 = REPEAT both, bit 0 = u
/// clamped, bit 1 = v clamped. A REPEAT sample at the edge of a clamped tie material (tree canopies, shoal mounds) bled the
/// opposite texture edge in as a thin dark line along the polygon border. Region modes (2, 3) and other registers keep REPEAT.
fn tie_wrap(record: &[u8]) -> u32 {
	match (u32_at(record, 0x30), u32_at(record, 0x34), u32_at(record, 0x38)) {
		(Ok(wms @ 0..=1), Ok(wmt @ 0..=1), Ok(8)) => wms | wmt << 1,
		_ => 0,
	}
}

/// The class mesh plus each vertex's colour slots, read from the packet's index stream (VU1 prog7: regular vertex k takes
/// stream byte k, fat vertex j the bytes x, y, z of stream word ceil(R / 4) + j). `lod` picks the packet
/// table (class u32 +4 x lod, packet count byte +0x20 + lod); the third result is each vertex's geomorph delta in class
/// units (the fat vertex's first i16 triple x scale / 1024, zero for regular vertices): a morphing vertex sits at
/// position + Q x delta (decomp/notes/novalis_enemies.md REN-CULL-TIE).
fn tie_mesh_indexed(data: &[u8], remap: &[u8], name: String, lod: usize) -> R<(Mesh, Vec<TieSlot>, Vec<[f32; 3]>)> {
	let mut colour_index = Vec::new();
	let mut morph = Vec::new();
	let header = span(data, 0, 0x70)?;
	let packet_table = u32_at(header, lod * 4)? as usize;
	let scale = f32_at(header, 0x40)?;
	if !scale.is_finite() || scale <= 0.0 {
		return Err("invalid tie scale".into());
	}
	let material_count = header[0x23] as usize;
	if !(1..=16).contains(&material_count) {
		return Err("invalid tie material count".into());
	}
	let textures = &remap[..material_count];
	if textures.contains(&255) {
		return Err("tie material has no texture".into());
	}
	// Material record (80 bytes, five A+D qwords): TEX0 slot (reg 6), TEX1 source (reg 0x14: +0x10 K, +0x14 MMIN, +0x18 the
	// register), MIPTBP1 (0x34), CLAMP (8), MIPTBP2 (0x36). The class setup builds the GS TEX1 from it like a shrub material
	// (vertex_colours.md): (texture mips - 1) << 2 (MXL) | MMIN (4 = LINEAR_MIPMAP_NEAREST) << 6 | 0x20 | K << 32 [C: Kalebo,
	// all 1305 records have MMIN 4, K 0xf92 or 0xf72; the GSPIX TEX1 of the mountain ties is 0xff92_0000_012c, level 3 of 3].
	let materials = span(data, u32_at(header, 0x2c)? as usize, material_count * 80)?;
	let material_k: Vec<Option<u32>> = materials
		.chunks_exact(80)
		.map(|r| Some(u32_at(r, 0x10).ok()? & 0xfff).filter(|_| u32_at(r, 0x14).ok() == Some(4) && u32_at(r, 0x18).ok() == Some(0x14)))
		.collect();
	let material_wrap: Vec<u32> = materials.chunks_exact(80).map(tie_wrap).collect();
	if crate::viewer::gs_state::enabled() {
		for r in materials.chunks_exact(80) {
			// The record's TEX1 / CLAMP source words as the GS registers (runtime CLAMP = WMS | WMT << 2); a record that is not the
			// registers the decode above expects (+0x18 != 0x14, +0x38 != 8) logs as a state the path ignores.
			let w = |o| u32_at(r, o).unwrap_or(0) as u64;
			let (tex1, clamp) = (w(0x10) << 32 | w(0x14) << 6 | 0x20, w(0x30) | w(0x34) << 2);
			if w(0x18) != 0x14 || w(0x38) != 8 {
				crate::viewer::gs_state::note("tie", "record.registers", (w(0x18) << 8 | w(0x38)) as u32);
			}
			crate::viewer::gs_state::guard_world("tie", crate::viewer::gs_state::TEST_TIE, tex1, clamp);
		}
	}
	let table = span(data, packet_table, header[0x20 + lod] as usize * 16)?;
	let mut mesh = Mesh::new(name);
	for (packet_id, entry) in table.chunks_exact(16).enumerate() {
		let start = packet_table + u32_at(entry, 0)? as usize;
		let control = span(data, start, entry[8] as usize * 16)?;
		span(control, 0, 0x2c)?;
		let dest: Vec<i32> = (0..4).map(|i| i32_at(control, i * 4)).collect::<R<_>>()?; // GS addresses of materials 1..3
		let src: Vec<i32> = (0..4).map(|i| i32_at(control, 16 + i * 4)).collect::<R<_>>()?; // their 80-byte AD GIF offsets
		let strips: Vec<[u8; 4]> = span(control, 0x2c, control[0x23] as usize * 4)?.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
		let packed = span(data, start + entry[8] as usize * 16, entry[9] as usize * 16)?;
		let dinky = control[0x28] as usize; // regular-vertex qwords plus four
		if dinky < 4 || dinky % 2 != 0 {
			return Err("invalid tie vertex encoding".into());
		}
		let regular = span(packed, 0, (dinky - 4) * 8)?;
		let fat = &packed[regular.len()..];
		if fat.len() % 24 != 0 && fat.len() % 24 != 8 {
			return Err("misaligned tie extended vertices".into());
		}
		// (x, y, z, first address, s, t, q, second address)
		let mut raw = Vec::new();
		for v in regular.chunks_exact(16) {
			raw.push([i16_at(v, 0)?, i16_at(v, 2)?, i16_at(v, 4)?, u16_at(v, 6)? as i16, i16_at(v, 8)?, i16_at(v, 10)?, i16_at(v, 12)?, u16_at(v, 14)? as i16, 0, 0, 0]);
		}
		let mut at = 0;
		while at + 24 <= fat.len() {
			raw.push([i16_at(fat, at + 8)?, i16_at(fat, at + 10)?, i16_at(fat, at + 12)?, u16_at(fat, at + 6)? as i16, i16_at(fat, at + 16)?, i16_at(fat, at + 18)?, i16_at(fat, at + 20)?, u16_at(fat, at + 22)? as i16, i16_at(fat, at)?, i16_at(fat, at + 2)?, i16_at(fat, at + 4)?]);
			at += 24;
		}
		let mut writes: BTreeMap<u16, [i16; 9]> = BTreeMap::new();
		// The index stream: n VU qwords of one byte per lane, `entry[10]` qwords after the packet start.
		let stream = span(data, start + entry[10] as usize * 16, entry[11] as usize * 4)?;
		let regular_count = regular.len() / 16;
		let packet_slots = packet_slots(stream, regular_count, raw.len() - regular_count).map_err(|x| format!("tie packet {packet_id}: {x}"))?;
		let mut slot_of: BTreeMap<u16, TieSlot> = BTreeMap::new();
		for (k, r) in raw.iter().enumerate() {
			let v = [r[0], r[1], r[2], r[4], r[5], r[6], r[8], r[9], r[10]];
			let slot = packet_slots[k];
			for a in [r[3] as u16, r[7] as u16] {
				slot_of.insert(a, slot);
				if a != 0 && *writes.entry(a).or_insert(v) != v {
					return Err("conflicting tie vertex writes".into());
				}
			}
		}
		let check_material = |s: i32| -> R<usize> {
			if s < 0 || s % 80 != 0 || s as usize / 80 >= material_count {
				return Err("invalid tie material source".into());
			}
			Ok(s as usize / 80)
		};
		let (mut address, mut shader, mut strip_index) = (6u16, 1usize, 0usize);
		let mut material = check_material(src[0])?;
		let (mut primitive, mut expected, mut winding): (Vec<u32>, usize, u8) = (Vec::new(), 0, 0);
		let finish = |mesh: &mut Mesh, primitive: &[u32], expected: usize, winding: u8, material: usize| -> R<()> {
			if primitive.len() != expected {
				return Err("tie strip vertex count mismatch".into());
			}
			for i in 2..primitive.len() {
				let f = [primitive[i - 2], primitive[i - 1], primitive[i]];
				let f = if i % 2 == winding as usize { f } else { [f[2], f[1], f[0]] };
				mesh.faces.push((tie_key(textures[material] as u32, material_k[material], material_wrap[material]), f));
			}
			Ok(())
		};
		while !writes.is_empty() || strip_index < strips.len() {
			if strip_index < strips.len() && strips[strip_index][2] as u16 == address {
				if expected > 0 {
					finish(&mut mesh, &primitive, expected, winding, material)?;
				}
				let [e, pad, _, w] = strips[strip_index];
				if e < 3 || pad != 0 || w > 1 {
					return Err("unsupported tie strip descriptor".into());
				}
				(expected, winding) = (e as usize, w);
				primitive.clear();
				strip_index += 1;
				address += 1;
			} else if let Some(v) = writes.remove(&address) {
				if strip_index == 0 {
					return Err("tie vertex precedes its strip".into());
				}
				if v[5] != 4096 {
					return Err("unsupported tie perspective texture coordinate".into());
				}
				primitive.push(mesh.positions.len() as u32);
				colour_index.push(slot_of[&address]);
				mesh.positions.push([v[0], v[1], v[2]].map(|c| c as f32 * scale / 1024.0));
					morph.push([v[6], v[7], v[8]].map(|c| c as f32 * scale / 1024.0));
				mesh.uvs.push([v[3] as f32 / 4096.0, v[4] as f32 / 4096.0]);
				address += 3;
			} else if shader < 4 && dest[shader - 1] == address as i32 {
				if expected > 0 {
					finish(&mut mesh, &primitive, expected, winding, material)?;
					primitive.clear();
					expected = 0;
				}
				material = check_material(src[shader])?;
				shader += 1;
				address += 6;
			} else {
				return Err(format!("tie packet {packet_id}: missing GS event at {address}"));
			}
		}
		if expected > 0 {
			finish(&mut mesh, &primitive, expected, winding, material)?;
		}
	}
	Ok((mesh, colour_index, morph))
}

/// Class ID -> mesh, from the core index's 32-byte tie class entries.
pub fn tie_classes(level: &Level) -> R<HashMap<u32, Mesh>> {
	let mut out = HashMap::new();
	for e in level.table(0x20, 32)? {
		let (start, class_id) = (u32_at(e, 0)? as usize, u32_at(e, 4)?);
		if start == 0 || out.contains_key(&class_id) {
			return Err(format!("duplicate or empty tie class {class_id}"));
		}
		let m = tie_mesh(level.block(start)?, &e[16..32], format!("tie_{class_id}")).map_err(|x| format!("tie class {class_id}: {x}"))?;
		out.insert(class_id, m);
	}
	Ok(out)
}

pub struct Instance {
	pub class_id: u32,
}

/// A column-major affine matrix at +0x10 of a placement record.
pub fn placement(record: &[u8], what: &str) -> R<[f32; 16]> {
	let mut m = [0f32; 16];
	for (i, v) in m.iter_mut().enumerate() {
		*v = f32_at(record, 0x10 + i * 4)?;
	}
	if m.iter().any(|x| !x.is_finite()) || m[3] != 0.0 || m[7] != 0.0 || m[11] != 0.0 {
		return Err(format!("invalid {what} matrix"));
	}
	m[15] = 1.0; // the game stores 0.01 here
	Ok(m)
}

/// PAL tie instances: gameplay +0x34 points to a count, then 0xe0-byte records.
pub fn tie_instances(level: &Level, classes: &HashMap<u32, Mesh>) -> R<Vec<Instance>> {
	let g = &level.gameplay;
	let start = u32_at(g, 0x34)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	let table = span(g, start + 16, count * 0xe0)?;
	table
		.chunks_exact(0xe0)
		.enumerate()
		.map(|(i, r)| {
			let class_id = u32_at(r, 0)?;
			if !classes.contains_key(&class_id) {
				return Err(format!("tie instance {i}: unknown class {class_id}"));
			}
			Ok(Instance { class_id, })
		})
		.collect()
}

/// A tie class's 64 per-colour-slot normals: class +0xc points at 64 x (s16 x, y, z, pad) / 32768
/// (LightTies prog1 input; decomp/notes/vertex_colours.md "Tie normals").
fn tie_normals(data: &[u8]) -> R<Vec<[f32; 3]>> {
	let at = u32_at(data, 0xc)? as usize;
	span(data, at, 64 * 8)?.chunks_exact(8).map(|c| Ok(std::array::from_fn(|j| i16::from_le_bytes([c[j * 2], c[j * 2 + 1]]) as f32 / 32768.0))).collect()
}

/// The world bounding sphere of a tie or shrub instance (loader at the level-load routine, Novalis ~45829 / 45960):
/// the class sphere's centre goes through the placement matrix, times the class scale, plus the translation; the
/// radius is the class radius times the longest matrix axis. The test below calibrates the units on Veldin and
/// Novalis: centre and radius both carry the class scale (the loader's radius product has no separate scale term
/// in the decompile, so the matrix rows it measures presumably hold it [I]).
pub fn instance_sphere(m: &[f32; 16], centre: [f32; 3], radius: f32, scale: f32) -> [f32; 4] {
	let c = centre.map(|x| x * scale);
	let at: [f32; 3] = std::array::from_fn(|r| m[r] * c[0] + m[4 + r] * c[1] + m[8 + r] * c[2] + m[12 + r]);
	let axis = |k: usize| (m[k * 4].powi(2) + m[k * 4 + 1].powi(2) + m[k * 4 + 2].powi(2)).sqrt();
	[at[0], at[1], at[2], radius * scale * axis(0).max(axis(1)).max(axis(2))]
}

pub fn unit(v: [f32; 3]) -> [f32; 3] {
	let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
	if l > 1e-12 { v.map(|c| c / l) } else { [0.0, 0.0, 1.0] }
}

/// Cap on a tie's draw distance (metres): the record's own distance is clamped to it (REN-CULL-TIE; retail 720.0 at level
/// init, the debug and pause toggles that change it are not modelled).
pub const TIE_DRAW_CAP: f32 = 720.0;

/// What `Gpu::cull_ties` needs per tie instance: the world sphere, the record's draw distance and the class's LOD distances
/// (class f32 +0x10, +0x14, +0x18).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TieCull {
	pub centre: [f32; 3],
	pub radius: f32,
	pub dist: f32,
	pub lod: [f32; 3],
	/// The occlusion (mask byte, index byte) pair of the record (REN-CULL-OCC); `occlusion::UNMAPPED` passes every mask.
	pub occ: (u8, u8),
}

/// One placed tie: its three LOD meshes (world space, baked colours, each with its own `Lit`) and, per LOD, every vertex's
/// world-space geomorph delta.
pub struct LitTie {
	pub lods: [Mesh; 3],
	pub morph: [Vec<[f32; 3]>; 3],
	/// Per LOD, every vertex's colour slots (BRIDGE-32).
	pub slots: [Vec<TieSlot>; 3],
	/// The instance's 64 capped slot colours as built (the viewer recolours the drawn LOD from them).
	pub cols: Vec<[f32; 4]>,
	pub cull: TieCull,
}

/// The per-frame tie choice for camera depth `pz` (the view-space z of the sphere centre): None when the tie is past its draw
/// distance, else the LOD to draw and the geomorph factor Q (REN-CULL-TIE Order 2 and 4; no hysteresis, nothing is kept
/// between frames). Distances compare exactly as the retail f32 expressions do.
pub fn tie_pick(c: &TieCull, pz: f32) -> Option<(usize, f32)> {
	let d = c.dist.min(TIE_DRAW_CAP);
	if (d - c.radius) - (pz - c.radius) < 0.0 {
		return None;
	}
	let pzc = pz.max(0.0);
	let [l0, l1, l2] = c.lod;
	Some(if l0 - pzc >= 0.0 {
		(0, 0.0)
	} else if l2 - pzc < 0.0 {
		(2, 0.0)
	} else if l1 - pzc >= 0.0 {
		(0, (pzc - l0) / (l1 - l0))
	} else {
		(1, (pzc - l1) / (l2 - l1))
	})
}

/// The colour slots of a packet's vertices in raw order (regular, then fat) from its index stream: regular vertex k is
/// stream byte k; fat vertex j is bytes x, y, z of stream word ceil(R / 4) + j (VU1 prog7 `MTIR` reads).
fn packet_slots(stream: &[u8], regular: usize, fat: usize) -> R<Vec<TieSlot>> {
	let lane = |i: usize| stream.get(i).map(|&b| b as usize).ok_or_else(|| format!("index stream ends at {i}"));
	let mut out = Vec::with_capacity(regular + fat);
	for k in 0..regular {
		let s = lane(k)?;
		out.push(([s, s, s], false));
	}
	for j in 0..fat {
		let at = 4 * (regular.div_ceil(4) + j);
		out.push(([lane(at)?, lane(at + 1)?, lane(at + 2)?], true));
	}
	Ok(out)
}

/// A tie vertex's colour from the instance's capped slot colours at geomorph factor `q` (VU1 prog7): a regular vertex takes
/// its slot's whole-byte colour; a fat vertex blends slots y and z against x as `256 (1 - q) c_x + 256 q (c_y + c_z) / 2`
/// with the VU's truncating float arithmetic, keeping the low byte of the result (the GIF takes bits 0-7 of each lane).
pub fn tie_colour(cap: &[[f32; 4]], slot: TieSlot, q: f32) -> [f32; 4] {
	use crate::physics::{vadd1, vmul, vsub1};
	let ([x, y, z], fat) = slot;
	// The slot float 65536 + c / 128 (bits 0x47800000 + c) keeps whole bytes only.
	let whole = |c: f32| c.floor();
	let f = |c: f32| f32::from_bits(0x4780_0000 + whole(c) as u32);
	let (cx, cy, cz) = (cap[x], cap[y], cap[z]);
	if !fat {
		return [whole(cx[0]), whole(cx[1]), whole(cx[2]), cx[3]];
	}
	let w = vmul(256.0, q);
	let zz = vsub1(256.0, w);
	let lane = |k: usize| {
		let m = vadd1(vmul(f(cy[k]), 0.5), vmul(f(cz[k]), 0.5));
		let out = vadd1(vmul(m, w), vmul(f(cx[k]), zz));
		(out.to_bits() & 0xff) as f32
	};
	[lane(0), lane(1), lane(2), cx[3]]
}

/// [PC] perf: `tie_colour` for every slot triple of one instance at one geomorph factor, with the per-slot products computed
/// once (the same VU operations on the same values, so the same bytes).
pub struct TieColours<'a> {
	cap: &'a [[f32; 4]],
	w: f32,
	/// Per slot and lane: VMUL(f(c), 0.5) and VMUL(f(c), 256 - w).
	half: Vec<[f32; 3]>,
	low: Vec<[f32; 3]>,
}

impl<'a> TieColours<'a> {
	pub fn new(cap: &'a [[f32; 4]], q: f32) -> Self {
		use crate::physics::{vmul, vsub1};
		let f = |c: f32| f32::from_bits(0x4780_0000 + c.floor() as u32);
		let w = vmul(256.0, q);
		let zz = vsub1(256.0, w);
		let half = cap.iter().map(|c| [0, 1, 2].map(|k| vmul(f(c[k]), 0.5))).collect();
		let low = cap.iter().map(|c| [0, 1, 2].map(|k| vmul(f(c[k]), zz))).collect();
		TieColours { cap, w, half, low }
	}

	pub fn colour(&self, slot: TieSlot) -> [f32; 4] {
		use crate::physics::{vadd1, vmul};
		let ([x, y, z], fat) = slot;
		let cx = self.cap[x];
		if !fat {
			return [cx[0].floor(), cx[1].floor(), cx[2].floor(), cx[3]];
		}
		let lane = |k: usize| {
			let m = vadd1(self.half[y][k], self.half[z][k]);
			(vadd1(vmul(m, self.w), self.low[x][k]).to_bits() & 0xff) as f32
		};
		[lane(0), lane(1), lane(2), cx[3]]
	}
}

/// A morphing vertex at geomorph factor `q`: its base position plus q times its delta.
pub fn morphed(base: [f32; 3], delta: [f32; 3], q: f32) -> [f32; 3] {
	std::array::from_fn(|k| base[k] + q * delta[k])
}

/// World-space tie instances with baked vertex colours (decomp/notes/vertex_colours.md): each
/// instance's 64 RGBA5551 colours at record +0x50 (PEXT5) plus the directional pair picked by the
/// selector at +0xd0, `q' = max(q, q * w)`, capped at 243. All three LODs of every class are built.
pub fn lit_instances(level: &Level, lights: &[DirLight]) -> R<Vec<LitTie>> {
	type Lod = (Mesh, Vec<TieSlot>, Vec<[f32; 3]>);
	let mut classes: HashMap<u32, ([Lod; 3], Vec<[f32; 3]>, [f32; 5], [f32; 3])> = HashMap::new();
	for e in level.table(0x20, 32)? {
		let (start, class_id) = (u32_at(e, 0)? as usize, u32_at(e, 4)?);
		let data = level.block(start)?;
		let lod = |k: usize| tie_mesh_indexed(data, &e[16..32], format!("tie_{class_id}"), k).map_err(|x| format!("tie class {class_id} lod {k}: {x}"));
		let meshes = [lod(0)?, lod(1)?, lod(2)?];
		let sphere = [f32_at(data, 0x30)?, f32_at(data, 0x34)?, f32_at(data, 0x38)?, f32_at(data, 0x3c)?, f32_at(data, 0x40)?];
		let dist = [f32_at(data, 0x10)?, f32_at(data, 0x14)?, f32_at(data, 0x18)?];
		classes.insert(class_id, (meshes, tie_normals(data).map_err(|x| format!("tie class {class_id} normals: {x}"))?, sphere, dist));
	}
	let g = &level.gameplay;
	let start = u32_at(g, 0x34)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	let records = span(g, start + 16, count * 0xe0)?;
	// Occlusion ids: record u32 +0xc; no mapping block leaves every tie unmapped (they pass the all-ones mask).
	let ids = records.chunks_exact(0xe0).map(|r| u32_at(r, 0xc)).collect::<R<Vec<u32>>>()?;
	let occ = crate::occlusion::OccMap::new(g).map_or_else(|| vec![crate::occlusion::UNMAPPED; ids.len()], |m| crate::occlusion::tie_bytes(&m, &ids));
	records
		.chunks_exact(0xe0)
		.enumerate()
		.map(|(i, r)| {
			let class_id = u32_at(r, 0)?;
			let (class_lods, slot_normals, cs, lod_dist) = classes.get(&class_id).ok_or(format!("tie instance {i}: unknown class {class_id}"))?;
			let mat = placement(r, "tie instance")?;
			let light = select_light(lights, u32_at(r, 0xd0)?);
			// Lighting is per colour slot with the class's slot normal; prog1 takes the lights to local space
			// with -R^T, the same as rotating the normal into the world (rows normalised).
			let world = |n: [f32; 3]| unit(std::array::from_fn(|r| (0..3).map(|c| unit([mat[c * 4], mat[c * 4 + 1], mat[c * 4 + 2]])[r] * n[c]).sum()));
			let wn: Vec<[f32; 3]> = (0..64).map(|k| world(slot_normals[k])).collect();
			let slots: Vec<[f32; 4]> = (0..64).map(|k| Ok(baked_raw(&light, pext5(u16_at(r, 0x50 + k * 2)?), wn[k]))).collect::<R<_>>()?;
			let capped: Vec<[f32; 4]> = slots.iter().map(|c| [c[0].clamp(0.0, TIE_CAP), c[1].clamp(0.0, TIE_CAP), c[2].clamp(0.0, TIE_CAP), c[3]]).collect();
			let sphere = instance_sphere(&mat, [cs[0], cs[1], cs[2]], cs[3], cs[4]);
			let build = |k: usize| -> R<(Mesh, Vec<[f32; 3]>, Vec<TieSlot>)> {
				let (class, index, delta) = &class_lods[k];
				let mut m = transformed(class, &mat, format!("tie_instance_{i}"));
				m.colours = index.iter().map(|&s| tie_colour(&capped, s, 0.0)).collect();
				// One 64-slot `Lit` on LOD 0 only (BRIDGE-32): the viewer recolours every LOD vertex from the slot colours.
				if k == 0 {
					m.lit = Some(Lit { kind: LitKind::Tie, sphere, origin: [0.0; 3], cap: TIE_CAP, normals: wn.clone(), raw: slots.clone() });
				}
				// The delta is a direction: the matrix's linear part only.
				let morph = delta.iter().map(|d| std::array::from_fn(|r| mat[r] * d[0] + mat[4 + r] * d[1] + mat[8 + r] * d[2])).collect();
				Ok((m, morph, index.clone()))
			};
			let ((m0, d0, s0), (m1, d1, s1), (m2, d2, s2)) = (build(0)?, build(1)?, build(2)?);
			let dist = i32_at(r, 4)? as f32;
			Ok(LitTie { lods: [m0, m1, m2], morph: [d0, d1, d2], slots: [s0, s1, s2], cols: capped, cull: TieCull { centre: [sphere[0], sphere[1], sphere[2]], radius: sphere[3], dist, lod: *lod_dist, occ: occ[i] } })
		})
		.collect()
}

/// A copy of `mesh` with every position transformed by a column-major affine matrix.
pub fn transformed(mesh: &Mesh, m: &[f32; 16], name: String) -> Mesh {
	let mut out = Mesh::new(name);
	out.positions = mesh
		.positions
		.iter()
		.map(|p| std::array::from_fn(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r]))
		.collect();
	out.uvs = mesh.uvs.clone();
	out.faces = mesh.faces.clone();
	out.colours = mesh.colours.clone();
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{iso, toc};

	/// vertex_colours.md "Tie normals": class +0xc holds 64 unit-length i15 normals per class.
	#[test]
	fn veldin_tie_normals() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let mut lens = Vec::new();
		for e in level.table(0x20, 32).unwrap() {
			let n = tie_normals(level.block(u32_at(e, 0).unwrap() as usize).unwrap()).unwrap();
			lens.extend(n.iter().map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()));
		}
		let near = lens.iter().filter(|&&l| (l - 1.0).abs() < 0.01).count();
		println!("{} normals, {near} unit, first {:?}", lens.len(), &lens[..4]);
		assert!(near * 10 >= lens.len() * 9);
		let lights = crate::gameplay::lighting(&level).unwrap().lights;
		assert!(!lit_instances(&level, &lights).unwrap().is_empty());
	}

	/// `tie_wrap`: the CLAMP source qword of every tie material on Veldin, Novalis and Pokitaru is register 8 with WMS / WMT 0
	/// or 1 (no region mode), and Veldin classes 2020 / 2027 give the vertex_colours.md sources (u32 lo = WMS, hi = WMT).
	#[test]
	fn tie_material_clamp_wraps() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for info in levels.iter().filter(|l| [0, 1, 11].contains(&l.id)) {
			let level = Level::load(&mut disc, info).unwrap();
			let mut count = [0usize; 4];
			for e in level.table(0x20, 32).unwrap() {
				let data = level.block(u32_at(e, 0).unwrap() as usize).unwrap();
				let n = data[0x23] as usize;
				let records = &data[u32_at(data, 0x2c).unwrap() as usize..][..n * 80];
				let wraps: Vec<u32> = records.chunks_exact(80).map(tie_wrap).collect();
				for r in records.chunks_exact(80) {
					assert_eq!(u32_at(r, 0x38).unwrap(), 8, "class {} CLAMP register", u32_at(e, 4).unwrap());
					assert!(u32_at(r, 0x30).unwrap() <= 1 && u32_at(r, 0x34).unwrap() <= 1, "class {} region mode", u32_at(e, 4).unwrap());
				}
				wraps.iter().for_each(|&w| count[w as usize] += 1);
				match (info.id, u32_at(e, 4).unwrap()) {
					(0, 2020) => assert_eq!(wraps, vec![2, 2, 0, 2]),
					(0, 2027) => assert_eq!(wraps, vec![3, 2, 2, 0, 3, 0]),
					_ => {}
				}
			}
			println!("level {}: tie materials repeat / u clamp / v clamp / both {count:?}", info.id);
			assert!(count[1] + count[2] + count[3] > 0);
		}
	}

	/// The instance bounding sphere (class sphere x placement) encloses the instance's vertices on Veldin and Novalis.
	#[test]
	fn instance_spheres_enclose_the_mesh() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for info in &levels[..2] {
			let level = Level::load(&mut disc, info).unwrap();
			let lights = crate::gameplay::lighting(&level).unwrap().lights;
			let (mut bad, mut n) = (0, 0);
			for t in lit_instances(&level, &lights).unwrap() {
				let m = &t.lods[0];
				let s = m.lit.as_ref().unwrap().sphere;
				let far = m.positions.iter().map(|p| ((p[0] - s[0]).powi(2) + (p[1] - s[1]).powi(2) + (p[2] - s[2]).powi(2)).sqrt()).fold(0.0f32, f32::max);
				n += 1;
				bad += (far > s[3] * 1.01 + 0.1) as usize;
			}
			println!("level {}: {bad} of {n} tie spheres do not enclose their vertices", info.id);
			assert_eq!(bad, 0);
		}
	}

	fn cull(lod: [f32; 3], dist: f32, radius: f32) -> TieCull {
		TieCull { centre: [0.0; 3], radius, dist, lod, occ: crate::occlusion::UNMAPPED }
	}

	/// REN-CULL-TIE tests 1-3 and 5: LOD bands and Q, the draw-distance edge, the 720 m cap, the geomorph position.
	#[test]
	fn tie_pick_bands_distance_and_morph() {
		let c = cull([20.0, 40.0, 80.0], 1000.0, 1.5);
		assert_eq!(tie_pick(&c, 20.0), Some((0, 0.0)));
		assert_eq!(tie_pick(&c, 25.0), Some((0, 0.25)));
		assert_eq!(tie_pick(&c, 40.0), Some((0, 1.0)));
		assert_eq!(tie_pick(&c, 50.0), Some((1, 0.25)));
		assert_eq!(tie_pick(&c, 80.0), Some((1, 1.0)));
		assert_eq!(tie_pick(&c, 80.5), Some((2, 0.0)));
		assert_eq!(tie_pick(&c, -3.0), Some((0, 0.0)), "behind the camera the band depth clamps to 0");
		let d = cull([20.0, 40.0, 80.0], 100.0, 1.5);
		assert!(tie_pick(&d, 100.0).is_some());
		assert!(tie_pick(&d, 100.001).is_none());
		assert!(tie_pick(&c, 720.0).is_some(), "record distance 1000 caps at 720");
		assert!(tie_pick(&c, 720.01).is_none());
		assert_eq!(morphed([0.0; 3], [1024.0 / 1024.0, 0.0, 0.0], 0.25), [0.25, 0.0, 0.0]);
	}

	/// All three LODs of every class build on Veldin and Novalis; the fat vertices carry deltas, regular ones none, and
	/// each LOD mesh has one delta and one baked colour per vertex.
	#[test]
	fn three_lods_build_with_morph_deltas() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for info in &levels[..2] {
			let level = Level::load(&mut disc, info).unwrap();
			let lights = crate::gameplay::lighting(&level).unwrap().lights;
			let ties = lit_instances(&level, &lights).unwrap();
			let (mut fat, mut empty) = ([0usize; 3], [0usize; 3]);
			for t in &ties {
				for k in 0..3 {
					assert_eq!(t.morph[k].len(), t.lods[k].positions.len());
					assert_eq!(t.lods[k].colours.len(), t.lods[k].positions.len());
					fat[k] += t.morph[k].iter().any(|d| d.iter().any(|&c| c != 0.0)) as usize;
					empty[k] += t.lods[k].faces.is_empty() as usize;
				}
				for k in 0..3 {
					assert!(t.slots[k].iter().all(|(s, _)| s.iter().all(|&i| i < 64)) && t.slots[k].len() == t.lods[k].positions.len());
				}
				assert!(t.cull.dist > 0.0 && t.cull.lod[0] <= t.cull.lod[1] && t.cull.lod[1] <= t.cull.lod[2], "{:?}", t.cull);
			}
			println!("level {}: {} ties, with fat vertices per LOD {:?}, empty per LOD {:?}", info.id, ties.len(), fat, empty);
			assert!(!ties.is_empty() && empty[0] == 0);
		}
	}

	/// REN-CULL-TIE-MCOL tests 1-2: the index streams of Veldin class 2020 LOD 0, packets 0 and 1.
	#[test]
	fn index_stream_slots_of_class_2020() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let e = level.table(0x20, 32).unwrap().into_iter().find(|e| u32_at(e, 4).unwrap() == 2020).unwrap();
		let data = level.block(u32_at(e, 0).unwrap() as usize).unwrap();
		let table = u32_at(data, 0).unwrap() as usize;
		let slots = |packet: usize, regular: usize, fat: usize| {
			let entry = &data[table + packet * 16..table + packet * 16 + 16];
			assert_eq!((entry[10], entry[11]), if packet == 0 { (0x3e, 0x20) } else { (entry[10], 34) });
			let start = table + u32_at(entry, 0).unwrap() as usize;
			let stream = &data[start + entry[10] as usize * 16..][..entry[11] as usize * 4];
			packet_slots(stream, regular, fat).unwrap()
		};
		let p0 = slots(0, 16, 28);
		let reg: Vec<usize> = p0[..16].iter().map(|s| s.0[0]).collect();
		assert_eq!(reg, vec![4, 1, 5, 1, 8, 10, 9, 9, 4, 1, 5, 1, 33, 5, 61, 13]);
		assert_eq!((p0[16], p0[17]), (([6, 4, 1], true), ([7, 5, 1], true)));
		let p1 = slots(1, 15, 30);
		let reg: Vec<usize> = p1[..15].iter().map(|s| s.0[0]).collect();
		assert_eq!(reg, vec![0, 1, 2, 1, 22, 14, 23, 14, 22, 23, 25, 15, 34, 14, 14]);
		assert_eq!((p1[15], p1[16]), (([3, 0, 1], true), ([3, 2, 1], true)));
	}

	/// [PC] perf: the per-instance colour table gives `tie_colour`'s bytes.
	#[test]
	fn tie_colours_match_tie_colour() {
		let cap: Vec<[f32; 4]> = (0..64).map(|i| [(i * 37 % 256) as f32 + 0.25, (i * 91 % 256) as f32, (i * 13 % 244) as f32 + 0.75, 128.0]).collect();
		for q in [0.0, 0.1, 0.33, 0.5, 0.77, 0.999, 1.0] {
			let tc = TieColours::new(&cap, q);
			for s in [([0, 1, 2], true), ([5, 63, 17], true), ([40, 40, 40], true), ([7, 0, 0], false), ([63, 1, 2], false)] {
				assert_eq!(tc.colour(s).map(f32::to_bits), tie_colour(&cap, s, q).map(f32::to_bits), "{s:?} {q}");
			}
		}
	}

	/// REN-CULL-TIE-MCOL tests 3-5: the fat-vertex colour blend of the VU1 morph path (whole-byte slot floats, truncating
	/// arithmetic) and the regular vertex's whole-byte colour.
	#[test]
	fn tie_colour_blend_bytes() {
		let blend = |cx: f32, cy: f32, cz: f32, q: f32| {
			let cap = [[cx, cx, cx, 128.0], [cy, cy, cy, 128.0], [cz, cz, cz, 128.0]];
			tie_colour(&cap, ([0, 1, 2], true), q)[0]
		};
		for (c, q, want) in [((40.0, 100.0, 61.0), 0.5, 60.0), ((40.0, 100.0, 61.0), 0.3, 51.0), ((40.0, 100.0, 61.0), 0.0, 40.0), ((40.0, 100.0, 61.0), 1.0, 80.0), ((200.0, 10.0, 20.0), 0.75, 61.0), ((243.0, 243.0, 243.0), 0.37, 242.0), ((128.0, 128.0, 128.0), 0.3, 127.0), ((0.0, 0.0, 0.0), 0.1, 255.0), ((1.0, 0.0, 0.0), 0.1, 0.0)] {
			assert_eq!(blend(c.0, c.1, c.2, q), want, "{c:?} q {q}");
		}
		let cap = [[40.7, 12.3, 243.0, 128.0]];
		for q in [0.0, 0.4, 1.0] {
			assert_eq!(tie_colour(&cap, ([0, 0, 0], false), q), [40.0, 12.0, 243.0, 128.0]);
		}
	}
}
