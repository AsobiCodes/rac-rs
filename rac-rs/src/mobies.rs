//! Mobys: animated model classes. LOD-0 bind-pose meshes plus per-vertex skin data.
//!
//! A moby packet is one VU1 batch: an ST unpack (texture coordinates), an index unpack (strip
//! indices with texture switch markers) and a vertex table (16-byte vertices, a duplicate list that
//! refers to a per-class vertex cache, and a skinning matrix cache replayed for the joints).
//! Layout: decomp/notes/moby_format.md (from Wrench's moby reader, verified on our disc).

use crate::bytes::{f32_at, i16_at, span, u16_at, u32_at, R};
use crate::level::{texture_key, Level, TextureGroup};
use crate::mesh::Mesh;
use crate::wad;
use std::collections::HashMap;

/// Up to three (joint, weight) pairs; unused pairs have weight 0. Weights are w/256 (no renormalisation): they sum to 1 when the bytes sum to 256.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skin {
	pub joints: [u8; 3],
	pub weights: [f32; 3],
}

impl Skin {
	fn single(joint: u8) -> Skin {
		Skin { joints: [joint, 0, 0], weights: [1.0, 0.0, 0.0] }
	}

	fn blend(parts: &[(Skin, f32)]) -> R<Skin> {
		let mut pairs: Vec<(u8, f32)> = Vec::new();
		for (skin, w) in parts {
			for i in 0..3 {
				if skin.weights[i] == 0.0 {
					continue;
				}
				match pairs.iter_mut().find(|p| p.0 == skin.joints[i]) {
					Some(p) => p.1 += skin.weights[i] * w,
					None => pairs.push((skin.joints[i], skin.weights[i] * w)),
				}
			}
		}
		if pairs.len() > 3 {
			return Err("skin blend has more than three joints".into());
		}
		let mut out = Skin { joints: [0; 3], weights: [0.0; 3] };
		for (i, p) in pairs.into_iter().enumerate() {
			(out.joints[i], out.weights[i]) = p;
		}
		Ok(out)
	}
}

#[derive(Clone)]
pub struct MobyClass {
	/// LOD 0 in game units, Z up. Faces carry `texture_key(TextureGroup::Moby, ..)`.
	pub mesh: Mesh,
	pub scale: f32,
	/// Joints and weights per output vertex, parallel to `mesh.positions`.
	pub skin: Vec<Skin>,
	/// Unit normals, parallel to `mesh.positions` (joint space for skinned vertices).
	pub normals: Vec<[f32; 3]>,
	/// Those untextured triangles as indices into `mesh.positions` (the class-122 shard fragments).
	pub flat: Vec<[u32; 3]>,
	/// Bounding sphere (centre xyz, radius) in game units: header +0x30 raw × scale/1024
	/// (decomp/notes/moby_format.md §2; robot debris spins about it, effects_lighting_mapping.md §2.4).
	pub bsphere: [f32; 4],
	/// First vertex of packet `header[0x0a]`: the draw loop FUN_002555d0 (moby flag +0x34 & 0x10) queues
	/// packets [byte_a, LOD end) for FUN_00255480, which overwrites their per-vertex lit colours with
	/// the moby's +0x90 RGBA. `mesh.positions.len()` when byte_a >= high_lod_count (no override).
	pub colour_start: usize,
	/// Class header +0x40: the default tail colour word (R = low byte, 0x80 = 1.0 per channel; 0 = none), copied to the moby's
	/// +0x90 at spawn (FUN_00263020 / FUN_0024e540); see `glow_rgb`.
	pub glow: u32,
	/// The collision block (class +0x10, decomp/notes/moby_format.md §7b), in game units at class scale.
	pub collision: Option<Collision>,
	/// The metal (chrome / glass) packets, table entries `header[7] ..+ header[6]` (moby_format.md 4.1, NOV-35b).
	pub metal: Vec<MetalPacket>,
	/// Class header +0x46 s16 type (u8 +0x46 with u8 +0x47 read as one short; moby_format.md row 0x46): 5 = enemy,
	/// 9 = crate (the Walloper spark burst FUN_00236820, weapons_framework.md WPN-6.6b1).
	pub kind: u16,
	/// BRIDGE-64a: the shadow-volume primitives (header byte 0xf rows of 16 bytes before the skeleton; empty when 0).
	pub shadow_prims: Vec<ShadowPrim>,
}

/// One shadow-volume primitive of a class (decomp/notes/shadows.md "Moby shadow volumes", record build FUN_0029b278).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShadowPrim {
	/// Type 0 (0x20 bytes): joint (+4), ring segment count (+8), centre in the joint frame (+0x10) and radius (+0x1c), in game
	/// units at class scale.
	Sphere { joint: usize, n: u32, at: [f32; 3], r: f32 },
	/// Type 1 (0x30 bytes): the capsule; its ring FUN_0029ba08 is BRIDGE-64b (not mapped), so it casts nothing yet.
	Capsule,
}

/// BRIDGE-64a: walk the class primitive block `header[0xf] * 16` bytes before the skeleton (+0x14): type byte 0 is a sphere
/// (0x20 bytes), any other a capsule (0x30 bytes); the walk ends after the primitive whose byte 1 is nonzero. `s` = scale / 1024.
fn shadow_prims(block: &[u8], header: &[u8], s: f32) -> R<Vec<ShadowPrim>> {
	let rows = header[0xf] as usize;
	if rows == 0 {
		return Ok(Vec::new());
	}
	let end = u32_at(header, 0x14)? as usize;
	let mut at = end.checked_sub(rows * 16).ok_or("shadow primitives before the class block")?;
	let mut out = Vec::new();
	loop {
		let head = span(block, at, 2)?;
		let size = if head[0] == 0 { 0x20 } else { 0x30 };
		let p = span(block, at, size)?;
		if head[0] == 0 {
			let at3 = [f32_at(p, 0x10)?, f32_at(p, 0x14)?, f32_at(p, 0x18)?];
			out.push(ShadowPrim::Sphere { joint: crate::bytes::i32_at(p, 4)?.max(0) as usize, n: u32_at(p, 8)?, at: at3.map(|v| v * s), r: f32_at(p, 0x1c)? * s });
		} else {
			out.push(ShadowPrim::Capsule);
		}
		at += size;
		if head[1] != 0 || at >= end {
			return Ok(out);
		}
	}
}

/// One metal packet (moby_format.md 4.1): its 16-byte AoS vertices (positions in game units at class scale, unit normals,
/// direct joint skin) and its strip triangles with the AD texture slot each draws with (-3 glass, other values < -1 chrome).
/// `cull` is the index unpack's VU1 cull mode byte (the same VU1 body as the LOD packets).
#[derive(Clone, Debug, Default)]
pub struct MetalPacket {
	pub positions: Vec<[f32; 3]>,
	pub normals: Vec<[f32; 3]>,
	pub skin: Vec<Skin>,
	pub faces: Vec<(i32, [u32; 3])>,
	pub cull: u8,
}

/// One collision primitive (moby_format.md §7b): `sets` bit 0 = set A, bit 1 = set B.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Prim {
	/// Type 1: sphere at a local centre.
	Sphere { centre: [f32; 3], r: f32 },
	/// Type 2: sphere at a joint plus an offset.
	JointSphere { joint: usize, offset: [f32; 3], r: f32 },
	/// Type 3: vertical capsule, axis (cx, cy, z .. z + h).
	Upright { base: [f32; 3], h: f32, r: f32 },
	/// Type 4 (and unknown types): capsule between two joints.
	JointCapsule { a: usize, b: usize, r: f32 },
}

/// A class collision block: primitives with their set bits, then the mesh (local vertices and one-sided faces
/// with their attr byte, outward normal (v2 - v0) x (v1 - v0)).
#[derive(Clone, Debug, Default)]
pub struct Collision {
	pub prims: Vec<(Prim, u8)>,
	pub verts: Vec<[f32; 3]>,
	pub faces: Vec<([u16; 3], u8)>,
	/// The raw 32-byte primitive rows (class order, through the row with flag 0x8000) and the unit (class scale / 1024).
	pub raw: Vec<[u8; 32]>,
	pub unit: f32,
}

/// Decodes the collision block at class offset `at`; `s` = class scale / 1024 (units per raw unit).
fn collision_block(block: &[u8], at: usize, s: f32) -> R<Collision> {
	let head = span(block, at, 0x10)?;
	let (prim_bytes, face_bytes, vert_bytes) = (u32_at(head, 4)? as usize, u32_at(head, 8)? as usize, u32_at(head, 0xc)? as usize);
	let prims = span(block, at + 0x10, prim_bytes)?;
	let verts = span(block, at + 0x10 + prim_bytes, vert_bytes)?;
	let faces = span(block, at + 0x10 + prim_bytes + vert_bytes, face_bytes)?;
	let mut out = Collision::default();
	out.unit = s;
	for p in prims.chunks_exact(32) {
		let v = |o: usize| -> R<[f32; 3]> { Ok([f32_at(p, o)? * s, f32_at(p, o + 4)? * s, f32_at(p, o + 8)? * s]) };
		let sets = (u16_at(p, 2)? & 3) as u8;
		let prim = match p[0] as i8 {
			1 => Prim::Sphere { centre: v(0x10)?, r: f32_at(p, 0x1c)? * s },
			2 => Prim::JointSphere { joint: u32_at(p, 4)? as usize, offset: v(0x10)?, r: f32_at(p, 0xc)? * s },
			3 => Prim::Upright { base: v(0x10)?, h: f32_at(p, 4)? * s, r: f32_at(p, 0x1c)? * s },
			_ => Prim::JointCapsule { a: u16_at(p, 4)? as usize, b: u16_at(p, 6)? as usize, r: f32_at(p, 0xc)? * s },
		};
		out.prims.push((prim, sets));
		out.raw.push(p.try_into().unwrap());
		if u16_at(p, 2)? & 0x8000 != 0 {
			break;
		}
	}
	for v in verts.chunks_exact(8) {
		out.verts.push([i16_at(v, 0)? as f32 * s, i16_at(v, 2)? as f32 * s, i16_at(v, 4)? as f32 * s]);
	}
	for f in faces.chunks_exact(4) {
		out.faces.push(([f[0] as u16, f[1] as u16, f[2] as u16], f[3]));
	}
	Ok(out)
}

/// Triangle winding. Strip triangle i is (s[i], s[i+1], s[i+2]) for even i and (s[i+1], s[i], s[i+2])
/// for odd i; this flips them all. Settled on the joint-less classes (no joints, so stored normals
/// are in model space; `winding` test, 1523 class instances over 19 levels): with the unflipped
/// parity (b - a) x (c - a) agrees with the vertex normals for 37% of faces, with the flip for 63%;
/// faces point away from the class centroid 59% of the time with the flip and 41% without, and the
/// stored normals themselves point away 74%. So faces are clockwise seen from outside as strips
/// are read, and the flip makes them counter-clockwise. The agreement is only partial because
/// the strippers do not preserve orientation from strip to strip (each strip is uniform): render
/// double-sided or accept ~1/3 reversed patches. Within a strip winding is consistent; shared
/// edges of Ratchet's faces run opposite in 9145 cases and the same way in 1078 (`edge_consistency`).
const FLIP_WINDING: bool = true;

#[derive(Clone, Copy)]
struct CachedVertex {
	position: [f32; 3],
	normal: [f32; 3],
	skin: Skin,
}

/// Unpacks of one packet's VIF list: (command, VU address, payload).
fn vif_unpacks(list: &[u8]) -> R<Vec<(u8, usize, &[u8])>> {
	let mut out = Vec::new();
	let mut at = 0;
	while at + 4 <= list.len() {
		let word = u32_at(list, at)?;
		let (imm, num, cmd) = ((word & 0xffff) as usize, (word >> 16 & 0xff) as usize, (word >> 24 & 0x7f) as u8);
		if cmd & 0x60 == 0x60 {
			let vnvl = (cmd & 0xf) as usize;
			let bits = [32, 16, 8, 5][vnvl & 3];
			let size = (num * ((vnvl >> 2) + 1) * bits + 31) / 32 * 4;
			out.push((cmd, imm & 0x3ff, span(list, at + 4, size)?));
			at += 4 + size;
		} else {
			at += 4;
		}
	}
	Ok(out)
}

/// Triangles of one strip (degenerates dropped), indices local to the packet's `count` vertices.
/// Each triangle carries its strip position `i % 2` (the loop index keeps counting across skipped degenerates).
fn strip_triangles(s: &[i32], count: usize) -> R<Vec<([u32; 3], u8)>> {
	let mut out = Vec::new();
	for i in 0..s.len().saturating_sub(2) {
		let (a, b, c) = (s[i], s[i + 1], s[i + 2]);
		if a == b || b == c || a == c {
			continue;
		}
		if [a, b, c].iter().any(|&x| x < 0 || x as usize >= count) {
			return Err("strip index out of range".into());
		}
		let f = if i % 2 == 0 { [a, b, c] } else { [b, a, c] };
		let f = if FLIP_WINDING { [f[2], f[1], f[0]] } else { f };
		out.push((f.map(|x| x as u32), (i % 2) as u8));
	}
	Ok(out)
}

/// Strips of one packet's index unpack (byte 2 the first texture-switch code, strip bytes from byte 4) with the texture slot
/// each strip draws with. The AD unpack `ad` supplies the switch codes (byte 0xc of each qword k) and slots (qword 2 of each
/// 0x40 block); `texture` carries the slot across packets (a packet without an AD unpack keeps the previous one).
fn packet_strips(index: &[u8], ad: Option<&[u8]>, texture: &mut i32) -> R<Vec<(i32, Vec<i32>)>> {
	let mut secret = vec![index[2] as i8];
	let mut textures = Vec::new();
	if let Some(ad) = ad {
		if ad.len() % 0x40 != 0 {
			return Err("misaligned AD unpack".into());
		}
		for k in 0..ad.len() / 0x40 {
			secret.push(ad[k * 0x10 + 0xc] as i8);
			textures.push(u32_at(ad, k * 0x40 + 0x20)? as i32);
		}
	}
	let indices: Vec<i8> = index[4..].iter().map(|&b| b as i8).collect();
	let mut strips: Vec<(i32, Vec<i32>)> = Vec::new();
	let mut ad = 0;
	for (j, &b) in indices.iter().enumerate() {
		let mut b = b;
		if b == 0 {
			let s = *secret.get(ad).ok_or("index list outruns its texture switches")?;
			if s == 0 {
				if let Some(cur) = strips.last_mut() {
					let keep = cur.1.len().saturating_sub(3);
					cur.1.truncate(keep);
				}
				break;
			}
			b = ((s & 0x7f) | -128i8) as i8;
			*texture = *textures.get(ad).ok_or("texture switch without AD block")?;
			ad += 1;
		}
		// A flagged vertex (no GS kick) followed by another one starts a strip; a lone one just
		// enters the vertex queue, so the strip continues with its natural parity.
		if b <= 0 && indices.get(j + 1).map_or(false, |&next| next <= 0) {
			strips.push((*texture, Vec::new()));
		}
		let cur = strips.last_mut().ok_or("vertex before the first strip")?;
		cur.1.push((b as i32 & 0x7f) - 1);
	}
	Ok(strips)
}

/// The metal packets of a class (moby_format.md 4.1): table entries `header[7] ..+ header[6]`; each VIF list is an index unpack
/// plus an optional AD unpack (no ST unpack), each vertex table a qword header `{n, ..}` and n 16-byte vertices (s16 x, y, z;
/// u8 azimuth, elevation; u8 joints 0..2, joint count 1..3, u8 weights / 256). `texture` continues from the LOD packets.
fn metal_packets(block: &[u8], header: &[u8], packet_table: usize, scale: f32, texture: &mut i32) -> R<Vec<MetalPacket>> {
	let mut out = Vec::new();
	for i in 0..header[6] as usize {
		let wrap = |e: String| format!("metal packet {i}: {e}");
		let entry = span(block, packet_table + (header[7] as usize + i) * 16, 16).map_err(wrap)?;
		let list = span(block, u32_at(entry, 0)? as usize, u16_at(entry, 4)? as usize * 16).map_err(wrap)?;
		let ups = vif_unpacks(list).map_err(wrap)?;
		if !(1..=2).contains(&ups.len()) || ups[0].0 & 0xf != 0xe || ups.get(1).map_or(false, |u| u.0 & 0xf != 0xc) {
			return Err(wrap(format!("unexpected VIF unpacks {:x?}", ups.iter().map(|u| (u.0, u.1, u.2.len())).collect::<Vec<_>>())));
		}
		let index = ups[0].2;
		if index.len() < 4 {
			return Err(wrap("short index unpack".into()));
		}
		let strips = packet_strips(index, ups.get(1).map(|u| u.2), texture).map_err(wrap)?;
		let vtx = u32_at(entry, 8)? as usize;
		let n = u32_at(span(block, vtx, 16).map_err(wrap)?, 0)? as usize;
		if n != entry[15] as usize || entry[12] as usize != n + 1 {
			return Err(wrap("inconsistent metal vertex table header".into()));
		}
		let mut p = MetalPacket { cull: index[0], ..Default::default() };
		for v in span(block, vtx + 16, n * 16).map_err(wrap)?.chunks_exact(16) {
			p.positions.push([i16_at(v, 0)?, i16_at(v, 2)?, i16_at(v, 4)?].map(|c| c as f32 * scale));
			p.normals.push(normal(v[6], v[7]));
			let w = |k: usize| v[12 + k] as f32 / 256.0;
			p.skin.push(match v[11] {
				// One joint: S_j0 unweighted (CMSAR 0x12d).
				1 => Skin::single(v[8]),
				2 => Skin { joints: [v[8], v[9], 0], weights: [w(0), w(1), 0.0] },
				3 => Skin { joints: [v[8], v[9], v[10]], weights: [w(0), w(1), w(2)] },
				// A class without joints stores count 0 (and joint / weight bytes 0): the vertex stays in model space (`skin_metal`).
				0 if header[8] == 0 => Skin { joints: [0; 3], weights: [0.0; 3] },
				c => return Err(wrap(format!("metal vertex joint count {c}"))),
			});
		}
		for (tex, s) in strips {
			for (f, _) in strip_triangles(&s, n).map_err(wrap)? {
				p.faces.push((tex, f));
			}
		}
		out.push(p);
	}
	Ok(out)
}

/// NOV-35b: the env matrix E (columns) of one moby's metal packets (FUN_0026a338 raw 0x26b4f8-0x26b610). `cam_pos` is the
/// camera position (game units), `cam_rot` the rows of the world-to-view rotation (camera 0x167080 +0x00/+0x10/+0x20),
/// `sphere` the moby's draw sphere centre (1/1024 units, moby +0x00) and `moby_cols` its three model axes in world space
/// (moby +0xc0/+0xd0/+0xe0). d = normalize(sphere - 1024 camera), dv = V d, C_k = R2 R1 V col_k.
pub fn metal_env(cam_pos: [f32; 3], cam_rot: [[f32; 3]; 3], sphere: [f32; 3], moby_cols: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
	let d: [f32; 3] = std::array::from_fn(|k| sphere[k] - cam_pos[k] * 1024.0);
	let l = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
	// VU RSQRT of 0 gives the largest float, which times the zero vector stays zero.
	let q = if l > 0.0 { 1.0 / l.sqrt() } else { 0.0 };
	let d = d.map(|x| x * q);
	let v = |x: [f32; 3]| -> [f32; 3] { std::array::from_fn(|i| cam_rot[i][0] * x[0] + cam_rot[i][1] * x[1] + cam_rot[i][2] * x[2]) };
	let dv = v(d);
	moby_cols.map(|col| {
		let c = v(col);
		let c = [c[0], dv[2] * c[1] - dv[1] * c[2], dv[1] * c[1] + dv[2] * c[2]];
		[dv[2] * c[0] - dv[0] * c[2], c[1], dv[0] * c[0] + dv[2] * c[2]]
	})
}

/// NOV-35b: the sphere-map UV of a metal vertex with skinned model-space normal `n` (not renormalised): per axis
/// st = FTOI12(E n + 1.0) (prog0 0x0a10 / 0x0a38), stored as (st >> 1) as s16 (psraw 1) and read like LOD UVs (s16 / 4096).
pub fn metal_uv(e: [[f32; 3]; 3], n: [f32; 3]) -> [f32; 2] {
	std::array::from_fn(|a| {
		let x = e[0][a] * n[0] + e[1][a] * n[1] + e[2][a] * n[2] + 1.0;
		(((x * 4096.0) as i32 >> 1) as i16) as f32 / 4096.0
	})
}

fn normal(azimuth: u8, elevation: u8) -> [f32; 3] {
	let (a, e) = (azimuth as f32 * std::f32::consts::PI / 128.0, elevation as f32 * std::f32::consts::PI / 128.0);
	// Wrench's sin/cos swap for azimuth is wrong: on joint-less classes this form lines up with the
	// face normals (mean |cos| 0.96 against 0.61 for the swapped form).
	[a.cos() * e.cos(), a.sin() * e.cos(), e.sin()]
}

pub fn moby_class(block: &[u8], remap: &[u8], id: u32) -> R<MobyClass> {
	decode_class(block, Some(remap), id)
}

/// Fallback slot base for a gadget class with no remap in the moby class table (the game always has one,
/// so faces normally carry `texture_key(Moby, remap[slot])`, a texture of the level's moby group).
pub const GADGET_TEXTURE_BASE: u32 = 0x8000;

/// Texture slot of untextured (AD slot -1) faces: retail draws them with an 8 x 8 grey texel 0x80808080
/// (decomp/notes/novalis_environment.md NOV-33.1); the viewer inserts that texture under this key.
pub const FLAT_SLOT: u32 = 0xffff;

/// `remap` is None for gadget classes (see `GADGET_TEXTURE_BASE`).
fn decode_class(block: &[u8], remap: Option<&[u8]>, id: u32) -> R<MobyClass> {
	let header = span(block, 0, 0x48)?;
	let packet_table = u32_at(header, 0)? as usize;
	let (high, joint_count) = (header[4] as usize, header[8] as usize);
	let scale = f32_at(header, 0x24)?;
	if !scale.is_finite() || scale <= 0.0 {
		return Err("invalid moby scale".into());
	}
	if remap.map_or(false, |r| r.len() != 16) {
		return Err("moby texture remap is not 16 bytes".into());
	}
	let scale = scale / 1024.0;
	let table = span(block, packet_table, if high == 0 { 0 } else { (high + header[5] as usize) * 16 })?;
	let mut mesh = Mesh::new(format!("moby_{id}"));
	let mut skin = Vec::new();
	let mut normals = Vec::new();
	let mut cache: HashMap<u16, CachedVertex> = HashMap::new();
	// VU0 matrix cache: 64 slots, persistent across packets.
	let mut slots: [Option<Skin>; 64] = [None; 64];
	let mut texture = 0i32;
	let mut untextured = 0;
	let mut flat: Vec<[u32; 3]> = Vec::new();
	let mut colour_start = usize::MAX;
	for (packet_id, entry) in table.chunks_exact(16).take(high).enumerate() {
		let wrap = |e: String| format!("packet {packet_id}: {e}");
		if packet_id == header[0x0a] as usize {
			colour_start = mesh.positions.len();
		}
		let list = span(block, u32_at(entry, 0)? as usize, u16_at(entry, 4)? as usize * 16).map_err(wrap)?;
		let ups = vif_unpacks(list).map_err(wrap)?;
		if !(2..=3).contains(&ups.len()) || ups[0].0 & 0xf != 5 || ups[1].0 & 0xf != 0xe || ups.get(2).map_or(false, |u| u.0 & 0xf != 0xc) {
			return Err(wrap(format!("unexpected VIF unpacks {:x?}", ups.iter().map(|u| (u.0, u.1, u.2.len())).collect::<Vec<_>>())));
		}
		let st = ups[0].2;
		let index = ups[1].2;
		if index.len() < 4 {
			return Err(wrap("short index unpack".into()));
		}
		let ad = ups.get(2).map(|u| u.2);

		// Vertex table.
		let vtx = u32_at(entry, 8)? as usize;
		let head = span(block, vtx, 0x20)?;
		let field = |i: usize| u32_at(head, i * 4).map(|v| v as usize);
		let (transfers, two, three, main, dupes, transfer, array, trailer) = (field(0)?, field(1)?, field(2)?, field(3)?, field(4)?, field(5)?, field(6)?, field(7)?);
		let n = two + three + main;
		if transfer != n + dupes || entry[15] as usize != transfer || array % 16 != 0 {
			return Err(wrap("inconsistent vertex table header".into()));
		}
		let mut p = vtx + 0x20;
		let preloop = span(block, p, transfers * 2)?;
		p += transfers * 2;
		if p % 4 != 0 {
			p += 2;
		}
		if p % 8 != 0 {
			p += 4;
		}
		let duplicates: Vec<u16> = (0..dupes).map(|k| u16_at(block, p + k * 2).map(|v| v >> 7)).collect::<R<_>>().map_err(wrap)?;
		let raw = span(block, vtx + array, n * 16).map_err(wrap)?;
		let verts: Vec<&[u8]> = raw.chunks_exact(16).collect();
		let low: Vec<u16> = verts.iter().map(|v| u16::from_le_bytes([v[0], v[1]])).collect();

		// Cache IDs: vertex k stores the ID of vertex k - 7; the last seven live in the epilogue.
		let mut ids = vec![0u16; n];
		for k in 7..n {
			ids[k - 7] = low[k] & 0x1ff;
		}
		let epilogue = trailer.checked_sub(array).ok_or_else(|| wrap("bad vertex trailer".into()))? / 16;
		let epilogue = epilogue.checked_sub(n).ok_or_else(|| wrap("bad vertex trailer".into()))?;
		let start = 7usize.saturating_sub(n);
		let mut q = vtx + array + n * 16 + start * 16;
		for i in start..epilogue {
			let di = n + i - 7;
			let v = span(block, q, 16).map_err(wrap)?;
			q += 16;
			if di < n {
				ids[di] = u16::from_le_bytes([v[0], v[1]]) & 0x1ff;
			}
		}
		let last = span(block, q - 16, 16).map_err(wrap)?;
		for i in 7usize.saturating_sub(n + epilogue)..6 {
			let di = n + epilogue + i - 7;
			if di < n {
				ids[di] = u16_at(last, 4 + 2 * i)? & 0x1ff;
			}
		}

		// Skinning replay.
		for t in preloop.chunks_exact(2) {
			let slot = t[1] as usize / 4;
			if slot >= 64 || t[0] as usize >= joint_count.max(1) {
				return Err(wrap("matrix transfer out of range".into()));
			}
			slots[slot] = Some(Skin::single(t[0]));
		}
		let mut packet_vertices: Vec<CachedVertex> = Vec::new();
		for (k, v) in verts.iter().enumerate() {
			let joint = (low[k] >> 9) as u8;
			let load = |slots: &[Option<Skin>; 64], addr: u8| -> R<Skin> {
				slots.get(addr as usize / 4).copied().flatten().ok_or_else(|| format!("vertex {k} reads an empty matrix slot {addr:#x}"))
			};
			let store = |slots: &mut [Option<Skin>; 64], addr: u8, s: Skin| -> R<()> {
				*slots.get_mut(addr as usize / 4).ok_or("matrix store out of range")? = Some(s);
				Ok(())
			};
			let skinned = if k < two {
				let (a, b) = (load(&slots, v[2]).map_err(wrap)?, load(&slots, v[3]).map_err(wrap)?);
				let (w1, w2) = (v[4] as f32 / 256.0, v[5] as f32 / 256.0);
				let s = Skin::blend(&[(a, w1), (b, w2)])?;
				store(&mut slots, v[6], Skin::single(joint))?;
				store(&mut slots, v[7], s)?;
				s
			} else if k < two + three {
				let (a, b, c) = (load(&slots, v[2]).map_err(wrap)?, load(&slots, v[3]).map_err(wrap)?, load(&slots, joint.wrapping_mul(2)).map_err(wrap)?);
				let s = Skin::blend(&[(a, v[4] as f32 / 256.0), (b, v[5] as f32 / 256.0), (c, v[6] as f32 / 256.0)])?;
				store(&mut slots, v[7], s)?;
				s
			} else {
				let s = load(&slots, v[2]).map_err(wrap)?;
				store(&mut slots, v[3], Skin::single(joint))?;
				s
			};
			packet_vertices.push(CachedVertex {
				position: [i16_at(v, 10)?, i16_at(v, 12)?, i16_at(v, 14)?].map(|c| c as f32 * scale),
				normal: normal(v[8], v[9]),
				skin: skinned,
			});
		}
		let uv = |k: usize| -> R<[f32; 2]> { Ok([i16_at(st, k * 4)? as f32 / 4096.0, i16_at(st, k * 4 + 2)? as f32 / 4096.0]) };
		let mut out_uvs = Vec::new();
		for k in 0..n {
			cache.insert(ids[k], packet_vertices[k]);
			out_uvs.push(uv(k).map_err(wrap)?);
		}
		for (d, id) in duplicates.iter().enumerate() {
			let v = *cache.get(id).ok_or_else(|| wrap(format!("duplicate refers to unknown vertex {id}")))?;
			packet_vertices.push(v);
			out_uvs.push(uv(n + d).map_err(wrap)?);
		}

		// Strips.
		let base = mesh.positions.len() as u32;
		let count = packet_vertices.len();
		let strips = packet_strips(index, ad, &mut texture).map_err(wrap)?;
		for (tex, s) in &strips {
			// -1 is GS state without a texture (flat geometry); chrome and glass (-2, -3) live in metal packets.
			if *tex == -1 {
				let tris = strip_triangles(s, count).map_err(wrap)?;
				untextured += tris.len();
				for &(f, phase) in &tris {
					mesh.strip_phase.push(phase);
					mesh.faces.push((texture_key(TextureGroup::Moby, FLAT_SLOT) | cull_bits(index[0]), f.map(|x| x + base)));
				}
				flat.extend(tris.into_iter().map(|(f, _)| f.map(|x| x + base)));
				continue;
			}
			if !(0..16).contains(tex) {
				return Err(wrap(format!("unsupported texture slot {tex}")));
			}
			let global = match remap {
				Some(r) if r[*tex as usize] == 0xff => return Err(wrap(format!("texture slot {tex} has no texture"))),
				Some(r) => r[*tex as usize] as u32,
				None => GADGET_TEXTURE_BASE + *tex as u32,
			};
			for (f, phase) in strip_triangles(s, count).map_err(wrap)? {
				mesh.strip_phase.push(phase);
				mesh.faces.push((texture_key(TextureGroup::Moby, global) | cull_bits(index[0]), f.map(|x| x + base)));
			}
		}
		for (v, t) in packet_vertices.iter().zip(out_uvs) {
			mesh.positions.push(v.position);
			mesh.uvs.push(t);
			normals.push(v.normal);
			skin.push(v.skin);
		}
	}
	let colour_start = colour_start.min(mesh.positions.len());
	let mut bsphere = [0.0; 4];
	for (k, b) in bsphere.iter_mut().enumerate() {
		*b = f32_at(header, 0x30 + k * 4)? * scale;
	}
	let collision = match u32_at(header, 0x10)? as usize {
		0 => None,
		at => Some(collision_block(block, at, scale)?),
	};
	let glow = u32_at(header, 0x40)?;
	let metal = metal_packets(block, header, packet_table, scale, &mut texture)?;
	let kind = class_kind(header)?;
	let shadow_prims = shadow_prims(block, header, scale).unwrap_or_default();
	mesh.normals = normals.clone();
	Ok(MobyClass {  mesh,  scale: scale * 1024.0, skin, normals,  flat, colour_start, glow, bsphere, collision, metal, kind, shadow_prims })
}

/// WPN-6.6b1: the class header +0x46 type short (5 = enemy, 9 = crate).
pub(crate) fn class_kind(header: &[u8]) -> R<u16> {
	u16_at(header, 0x46)
}

/// The class tail colour as shader RGB (byte / 128), or None when the class word is 0 (the moby stays lit).
pub fn glow_rgb(glow: u32) -> Option<[f32; 3]> {
	if glow == 0 {
		return None;
	}
	Some([(glow & 0xff) as f32 / 128.0, (glow >> 8 & 0xff) as f32 / 128.0, (glow >> 16 & 0xff) as f32 / 128.0])
}

/// Face key bit for a packet whose VU1 cull mode (index unpack byte 0) is 2: back faces are dropped (NOV-5.9a).
pub const CULL_BACK: u32 = 1 << 24;
/// Face key bit for cull mode 0: front faces are dropped.
pub const CULL_FRONT: u32 = 1 << 25;
/// Mask that strips the cull bits from a face texture key.
pub const KEY_MASK: u32 = 0x00ff_ffff;

/// The face-key cull bits of a packet's cull mode byte (254 and every other value cull nothing).
pub fn cull_bits(mode: u8) -> u32 {
	match mode {
		0 => CULL_FRONT,
		2 => CULL_BACK,
		_ => 0,
	}
}

/// The retail moby clipper's guard band in view space (prog11 L_1b58): x and y planes at `k * vz`, z planes at near / far.
#[derive(Clone, Copy, Debug)]
pub struct GuardBand {
	pub near: f32,
	pub far: f32,
	pub kx: f32,
	pub ky: f32,
}

impl GuardBand {
	/// The retail band for the vertical tangent `ty` and horizontal `tx`: 4 times the tangents, 0.03125 m .. 728.1632 m.
	pub fn retail(tx: f32, ty: f32) -> GuardBand {
		GuardBand { near: 0.03125, far: 728.1632, kx: 4.0 * tx, ky: 4.0 * ty }
	}

	/// The six plane flags of a view-space point (bit 5 -z, 4 +z, 3 -y, 2 +y, 1 -x, 0 +x), classified with |vz| as VU CLIP does.
	pub fn flags(&self, p: [f32; 3]) -> u8 {
		let w = p[2];
		let (ax, ay) = (self.kx * w.abs(), self.ky * w.abs());
		(p[2] < self.near) as u8 * 0x20 | (p[2] > self.far) as u8 * 0x10 | (p[1] < -ay) as u8 * 0x08 | (p[1] > ay) as u8 * 0x04 | (p[0] < -ax) as u8 * 0x02 | (p[0] > ax) as u8
	}

	/// Signed inside distance of `p` to plane `bit` (negative = outside), the quantity the clip parameter t uses.
	fn inside(&self, bit: u8, p: [f32; 3]) -> f32 {
		match bit {
			0x20 => p[2] - self.near,
			0x10 => self.far - p[2],
			0x08 => self.ky * p[2] + p[1],
			0x04 => self.ky * p[2] - p[1],
			0x02 => self.kx * p[2] + p[0],
			_ => self.kx * p[2] - p[0],
		}
	}
}


/// `guard_clip` that also returns each polygon vertex's barycentric weights over the three original vertices (the same
/// interpolation parameters t applied to unit weights), so the caller can lerp any per-vertex attribute with the same t.
pub fn guard_clip_weights(tri: [[f32; 3]; 3], k: GuardBand) -> Vec<(u8, [f32; 3], [f32; 3])> {
	let active = tri.iter().fold(0u8, |a, &p| a | k.flags(p));
	let mut list: Vec<(u8, [f32; 3], [f32; 3])> = tri.iter().enumerate().map(|(i, &p)| (i as u8, p, std::array::from_fn(|j| (i == j) as u8 as f32))).collect();
	let lerp = |a: [f32; 3], b: [f32; 3], t: f32| [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]), a[2] + t * (b[2] - a[2])];
	for bit in [0x20u8, 0x10, 0x08, 0x04, 0x02, 0x01] {
		if active & bit == 0 {
			continue;
		}
		let mut next = Vec::with_capacity(list.len() + 1);
		for i in 0..list.len() {
			let (p, q) = (list[i], list[(i + 1) % list.len()]);
			let (po, qo) = (k.flags(p.1) & bit != 0, k.flags(q.1) & bit != 0);
			let (sp, sq) = (k.inside(bit, p.1), k.inside(bit, q.1));
			match (po, qo) {
				(false, false) => next.push(q),
				(false, true) => {
					let t = (sq / (sq - sp)).clamp(0.0, 1.0);
					next.push((3, lerp(q.1, p.1, t), lerp(q.2, p.2, t)));
				}
				(true, false) => {
					let t = (sp / (sp - sq)).clamp(0.0, 1.0);
					next.push((3, lerp(p.1, q.1, t), lerp(p.2, q.2, t)));
					next.push(q);
				}
				(true, true) => {}
			}
		}
		list = next;
	}
	list
}

/// The strip triangles `(k, k+1, k+2)` of an emitted polygon of `n` vertices.
pub fn clip_strip(n: usize) -> Vec<[usize; 3]> {
	(0..n.saturating_sub(2)).map(|k| [k, k + 1, k + 2]).collect()
}

/// Class ID -> LOD-0 model, from the core index's 32-byte moby class entries.
pub fn moby_classes(level: &Level) -> R<HashMap<u32, MobyClass>> {
	let mut out = HashMap::new();
	for e in level.table(0x18, 32)? {
		let (start, class_id) = (u32_at(e, 0)? as usize, u32_at(e, 4)?);
		if start == 0 {
			continue;
		}
		if out.contains_key(&class_id) {
			return Err(format!("duplicate moby class {class_id}"));
		}
		let c = moby_class(level.block(start)?, &e[0x10..0x20], class_id).map_err(|x| format!("moby class {class_id}: {x}"))?;
		out.insert(class_id, c);
	}
	Ok(out)
}

/// Class ID -> 16-byte texture remap from every entry of the moby class table (including the model-less
/// entries that exist only to carry the remap of a gadget class).
pub fn moby_remaps(level: &Level) -> R<HashMap<u32, [u8; 16]>> {
	let mut out = HashMap::new();
	for e in level.table(0x18, 32)? {
		let mut r = [0u8; 16];
		r.copy_from_slice(span(e, 0x10, 16)?);
		out.insert(u32_at(e, 4)?, r);
	}
	Ok(out)
}

/// The decompressed class block of one gadget class (None when the level has no such gadget). Each block
/// is WAD-compressed in core data.
pub fn gadget_block(level: &Level, class_id: u32) -> R<Option<Vec<u8>>> {
	for e in level.table(0x80, 16)? {
		let start = u32_at(e, 0)? as usize;
		if start == 0 || u32_at(e, 4)? != class_id {
			continue;
		}
		let block = level.block(start)?;
		let out = if wad::is_wad(block) { wad::decompress(block).map_err(|e| format!("gadget class {class_id}: WAD: {e}"))? } else { block.to_vec() };
		return Ok(Some(out));
	}
	Ok(None)
}

/// Gadget (hero weapon / item) class ID -> model, from the core index's 16-byte gadget table (offset 0x80:
/// core offset, class ID, offset of the gadget's GS texture list in the index, 0). The blocks use the moby
/// class layout (joints, bind pose), with no sequences.
pub fn gadget_classes(level: &Level) -> R<HashMap<u32, MobyClass>> {
	let mut out = HashMap::new();
	let remaps = moby_remaps(level)?;
	for e in level.table(0x80, 16)? {
		let (start, class_id) = (u32_at(e, 0)? as usize, u32_at(e, 4)?);
		if start == 0 {
			continue;
		}
		let wrap = |x: String| format!("gadget class {class_id}: {x}");
		let block = gadget_block(level, class_id)?.ok_or_else(|| wrap("vanished".into()))?;
		// The gadget entry has no remap of its own: the level loader copies it from the moby class table
		// entry (start 0, no model) with the same class ID.
		let remap = remaps.get(&class_id);
		let c = decode_class(&block, remap.map(|r| &r[..]), class_id).map_err(wrap)?;
		if out.insert(class_id, c).is_some() {
			return Err(format!("duplicate gadget class {class_id}"));
		}
	}
	Ok(out)
}

/// Class ID of the Omniwrench model (the held item with ID 8 in the hero item table).
pub const WRENCH_CLASS: u32 = 71;

#[cfg(test)]
mod tests {
	// WPN-6.6b1 test (1): header bytes 0x46..0x48 read as one little-endian short.
	#[test]
	fn class_kind_short() {
		let mut h = vec![0u8; 0x48];
		for (b, k) in [(5u8, 5u16), (9, 9), (1, 1)] {
			h[0x46] = b;
			assert_eq!(super::class_kind(&h).unwrap(), k);
		}
	}

	// CLASS-tint test 1.
	#[test]
	fn glow_rgb_channels() {
		assert_eq!(super::glow_rgb(0x808c8c8c), Some([1.09375; 3]));
		assert_eq!(super::glow_rgb(0x80c8c8c8), Some([1.5625; 3]));
		assert_eq!(super::glow_rgb(0x8084e643), Some([0.5234375, 1.796875, 1.03125]));
		assert_eq!(super::glow_rgb(0), None);
	}

	// NOV-5.9c test 4: the phase is the loop index parity, counted across skipped degenerates.
	#[test]
	fn strip_triangles_carry_their_strip_phase() {
		let t = super::strip_triangles(&[0, 1, 2, 3, 3, 4, 5], 6).unwrap();
		assert_eq!(t, vec![([2, 1, 0], 0), ([3, 1, 2], 1), ([5, 4, 3], 0)]);
	}

	use super::*;
	use crate::{iso, toc};
	use std::path::PathBuf;

	fn open() -> (iso::Disc, Vec<toc::Level>) {
		let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = iso::Disc::open(&path).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		(disc, levels)
	}

	#[test]
	fn cull_bits_from_the_packet_mode() {
		assert_eq!(cull_bits(254), 0);
		assert_eq!(cull_bits(0), CULL_FRONT);
		assert_eq!(CULL_FRONT, 0x0200_0000);
		assert_eq!(cull_bits(2), CULL_BACK);
		assert_eq!(CULL_BACK, 0x0100_0000);
		assert_eq!((cull_bits(1), cull_bits(3)), (0, 0));
		let key = texture_key(TextureGroup::Moby, 5) | CULL_BACK;
		assert_eq!(key, 0x0101_0005);
		assert_eq!(key & KEY_MASK, 0x0001_0005);
		assert_eq!(key & 0xffff, 5);
		assert_eq!((3071.0f32 + 254.0) * 16.0, 53200.0);
		assert_eq!((3071.0f32 + 0.0) * 16.0, 49136.0);
	}

	/// Packets with cull mode 2 give class 705 (fan) faces the back-cull bit; jets 660/688 are mode 254.
	#[test]
	fn cull_bits_reach_the_faces() {
		let (mut disc, levels) = open();
		let (mut fan, mut jets) = (0, 0);
		for l in levels.iter().take(20) {
			let Ok(level) = Level::load(&mut disc, l) else { continue };
			let Ok(classes) = moby_classes(&level) else { continue };
			if let Some(c) = classes.get(&705) {
				assert!(c.mesh.faces.iter().all(|f| f.0 & CULL_BACK != 0 && f.0 & CULL_FRONT == 0));
				fan += 1;
			}
			for id in [660, 688] {
				if let Some(c) = classes.get(&id) {
					assert!(c.mesh.faces.iter().all(|f| f.0 & !KEY_MASK == 0));
					jets += 1;
				}
			}
		}
		println!("fan levels {fan}, jet classes {jets}");
	}

	// BRIDGE-64a: the shadow primitive block before the skeleton (class header byte 0xf), walked to the last-flagged entry.
	#[test]
	fn shadow_prims_from_disc() {
		let (mut disc, levels) = open();
		let aridia = Level::load(&mut disc, &levels[2]).unwrap();
		let classes = moby_classes(&aridia).unwrap();
		// Constructor bot 612: one sphere on joint 7 (12 ring sides, r 0.36 at class scale) ahead of 10 capsules.
		let bot = &classes[&612].shadow_prims;
		assert_eq!(bot.len(), 11);
		match bot[0] {
			ShadowPrim::Sphere { joint, n, at, r } => {
				assert_eq!((joint, n), (7, 12));
				assert_eq!(at, [0.0; 3]);
				assert!((r - 0.36).abs() < 0.01, "{r}");
			}
			ShadowPrim::Capsule => panic!("612 starts with a sphere"),
		}
		assert!(bot[1..].iter().all(|p| matches!(p, ShadowPrim::Capsule)));
		// Shark 580: capsules only (BRIDGE-64b).
		let shark = &classes[&580].shadow_prims;
		assert_eq!(shark.len(), 6);
		assert!(shark.iter().all(|p| matches!(p, ShadowPrim::Capsule)));
		// Every placed class: the allocator's 0x400 flag exactly when the class has a shadow block.
		for li in 0..3 {
			let level = Level::load(&mut disc, &levels[li]).unwrap();
			for (id, c) in &moby_classes(&level).unwrap() {
				let f = crate::game::sched::placed_flags(*id);
				if f != 0 {
					assert_eq!(f & 0x400 != 0, !c.shadow_prims.is_empty(), "level {li} class {id} flags {f:#x}");
				}
			}
		}
	}

	// NOV-33.1 test 1: untextured (slot -1) faces join the mesh under FLAT_SLOT, `flat` stays for the fx path.
	#[test]
	fn untextured_faces_draw_with_the_flat_slot() {
		let (mut disc, levels) = open();
		let novalis = Level::load(&mut disc, &levels[1]).unwrap();
		let classes = moby_classes(&novalis).unwrap();
		let rocket = &classes[&686];
		assert_eq!(rocket.mesh.positions.len(), 50);
		assert_eq!(rocket.mesh.faces.len(), 64);
		assert_eq!(rocket.flat.len(), 64);
		assert_eq!(rocket.mesh.strip_phase.len(), 64);
		assert!(rocket.mesh.faces.iter().all(|f| f.0 == 0x1ffff && f.0 == texture_key(TextureGroup::Moby, FLAT_SLOT)));
		let veldin = Level::load(&mut disc, &levels[0]).unwrap();
		let classes = moby_classes(&veldin).unwrap();
		assert_eq!(classes[&1137].mesh.faces.len(), 64);
		assert_eq!(classes[&10].mesh.faces.len(), 4502);
	}

	/// Collision blocks against collision_and_gameplay.md "Moby collision shapes": the crate cube, the robot's
	/// two upright capsules, the toad's capsule.
	#[test]
	fn collision_blocks() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let classes = moby_classes(&level).unwrap();
		let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
		let c = classes[&500].collision.as_ref().unwrap();
		assert_eq!((c.verts.len(), c.faces.len()), (8, 12));
		assert!(matches!(c.prims[..], [(Prim::Sphere { centre, r }, 2)] if near(centre[2], 0.5) && near(r, 0.5)));
		let (lo, hi) = c.verts.iter().fold(([9.0f32; 3], [-9.0f32; 3]), |(l, h), v| (std::array::from_fn(|i| l[i].min(v[i])), std::array::from_fn(|i| h[i].max(v[i]))));
		assert!(near(lo[0], -0.5) && near(hi[0], 0.5) && near(lo[2], 0.0) && near(hi[2], 1.0), "{lo:?} {hi:?}");
		let r = classes[&1440].collision.as_ref().unwrap();
		assert!(matches!(r.prims[..], [(Prim::Upright { base: a, h: ha, r: ra }, 1), (Prim::Upright { base: b, h: hb, r: rb }, 2)]
			if near(a[2], 0.5) && near(ha, 0.5) && near(ra, 0.5) && near(b[2], 0.5) && near(hb, 0.75) && near(rb, 0.75)), "{:?}", r.prims);
		assert!(r.faces.is_empty());
		let t = classes[&749].collision.as_ref().unwrap();
		assert!(matches!(t.prims[..], [(Prim::Upright { base, h, r }, 3)] if near(base[2], 0.5) && near(h, 0.2) && near(r, 0.5)), "{:?}", t.prims);
	}

	#[test]
	fn all_levels() {
		let (mut disc, levels) = open();
		let mut failed = 0;
		for info in &levels {
			let result = Level::load(&mut disc, info).and_then(|l| moby_classes(&l));
			match result {
				Ok(c) => println!("level {}: {} classes, {} triangles", info.id, c.len(), c.values().map(|m| m.mesh.faces.len()).sum::<usize>()),
				Err(e) => {
					println!("level {}: error {e}", info.id);
					failed += 1;
				}
			}
		}
		println!("{} levels, {failed} failed", levels.len());
		assert_eq!(failed, 0);
	}


	/// Shared edges of adjacent faces should run in opposite directions when winding is consistent.
	#[test]
	fn edge_consistency() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let c = &moby_classes(&level).unwrap()[&0];
		let mut ids: HashMap<[i64; 3], u32> = HashMap::new();
		let key: Vec<u32> = c
			.mesh
			.positions
			.iter()
			.map(|p| {
				let n = ids.len() as u32;
				*ids.entry(p.map(|x| (x * 10000.0).round() as i64)).or_insert(n)
			})
			.collect();
		let mut edges: HashMap<(u32, u32), usize> = HashMap::new();
		for (_, f) in &c.mesh.faces {
			let k = f.map(|i| key[i as usize]);
			for e in 0..3 {
				*edges.entry((k[e], k[(e + 1) % 3])).or_insert(0) += 1;
			}
		}
		let opposite: usize = edges.iter().filter(|((a, b), _)| a < b).map(|(&(a, b), &n)| n.min(edges.get(&(b, a)).copied().unwrap_or(0))).sum();
		let same: usize = edges.iter().filter(|((a, b), _)| a != b).map(|(_, &n)| n - 1).sum();
		println!("ratchet shared edges: {opposite} opposite, {same} same direction");
		assert!(opposite > 5 * same);
	}

	/// NOV-35b tests 1, 2 and 4: Novalis class 13 metal packets, the metal normal and the sphere-map UV; every metal face on the disc
	/// draws with a metal slot (< -1).
	#[test]
	fn metal_packets_decode() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[1]).unwrap();
		let classes = moby_classes(&level).unwrap();
		let c = &classes[&13];
		assert_eq!(c.metal.len(), 2);
		assert_eq!((c.metal[0].positions.len(), c.metal[1].positions.len()), (96, 60));
		let s = c.scale / 1024.0;
		assert_eq!(c.metal[0].positions[0], [-5944.0 * s, 3789.0 * s, 2196.0 * s]);
		assert_eq!(c.metal[0].skin[0], Skin::single(0));
		let n = normal(0x7d, 2);
		for (x, e) in n.iter().zip([-0.9960892f32, 0.07347595, 0.04906768]) {
			assert!((x - e).abs() < 1e-6, "{n:?}");
		}
		assert_eq!(c.metal[0].normals[0], n);
		let id = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
		assert_eq!(metal_uv(id, n), [0.001953125, 0.5366211]);
		assert!(c.metal.iter().all(|p| !p.faces.is_empty() && p.faces.iter().all(|f| f.0 == -3)));
		let mut bad = Vec::new();
		for info in &levels {
			let Ok(l) = Level::load(&mut disc, info) else { continue };
			for (id, k) in moby_classes(&l).unwrap() {
				if k.metal.iter().any(|p| p.faces.iter().any(|f| f.0 >= -1)) {
					bad.push((info.id, id));
				}
			}
		}
		assert!(bad.is_empty(), "{bad:?}");
	}

	/// NOV-35b: with the camera straight down the view axis (d = (0, 0, 1) in view space) R1 and R2 are the identity, so E = V cols.
	#[test]
	fn metal_env_rotations() {
		let v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
		let cols = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
		assert_eq!(metal_env([0.0; 3], v, [0.0, 0.0, 2048.0], cols), cols);
		// d = (1, 0, 0): R1 (cos = dv.z = 0, sin = dv.y = 0) keeps only x, R2 moves it to z.
		let e = metal_env([0.0; 3], v, [1024.0, 0.0, 0.0], v);
		assert_eq!(e, [[0.0, 0.0, 1.0], [0.0; 3], [0.0; 3]]);
		// Camera at the sphere centre: d = 0, E = 0, uv = 0.5.
		let z = metal_env([1.0, 2.0, 3.0], v, [1024.0, 2048.0, 3072.0], v);
		assert_eq!(metal_uv(z, [0.3, 0.4, 0.5]), [0.5, 0.5]);
	}
}
