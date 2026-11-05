//! The effect pool's draw order and its ribbon quads (engine split stage E2 batch 4b, 2026-10-26: moved out of
//! rac1/retail/fx_queue.rs and retail/queue_b.rs; the old paths re-export): `pool_walk` = the frame's particles in the order the
//! retail pool draw sends them (back to front by depth bucket, then falling slot index, every mode in one list),
//! `pool_ribbon_quad` = a mode-3 slot as a world quad.

/// A mode-3 pool slot: head +0x10, tail +0x20, half-width +0x1c, tail scale +0x2c, colours +4 (head) / +0xc (tail),
/// particle texture +2, additive unless ALPHA +3 == 0x44, fade/cull byte +9, and its pool slot index.
#[derive(Clone, Copy, Debug)]
pub struct PoolRibbon {
	pub head: [f32; 3],
	pub tail: [f32; 3],
	pub half: f32,
	pub scale: f32,
	pub c_head: u32,
	pub c_tail: u32,
	pub tex: u8,
	pub additive: bool,
	pub b9: u8,
	pub slot: u16,
}

/// A pool colour word (R low byte, alpha on top) as 0..1 channels (0x80 = 1.0).
pub fn pool_rgba(w: u32) -> [f32; 4] {
	[(w & 0xff) as f32 / 128.0, (w >> 8 & 0xff) as f32 / 128.0, (w >> 16 & 0xff) as f32 / 128.0, (w >> 24) as f32 / 128.0]
}

/// A mode-3 pool slot's quad (FUN_00259e18 mode 3, particles.md "Draw size"): corners head +- off, tail +- off * scale,
/// off = the unit cross(midpoint - eye, tail - head) * half-width; U across, V along (head 0, tail 1). Fan order.
/// `fade` scales both alphas: the packet build multiplies bytes +7 / +0xf by min(key - near, (far - key) / 16, 1) of the
/// midpoint depth key, as the sprite modes (overlay.elf.c 82318-82380: pminw of (far - key) >> 4, key - near * 0x400 and
/// 0x1000, pmultuw, >> 12) [C].
pub fn pool_ribbon_quad(r: &PoolRibbon, eye: [f32; 3], fade: f32) -> crate::prims::WorldQuad {
	let (a, b) = (r.head, r.tail);
	let d: [f32; 3] = std::array::from_fn(|i| b[i] - a[i]);
	let v: [f32; 3] = std::array::from_fn(|i| (a[i] + b[i]) * 0.5 - eye[i]);
	let x = [v[1] * d[2] - v[2] * d[1], v[2] * d[0] - v[0] * d[2], v[0] * d[1] - v[1] * d[0]];
	let l = (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
	let off: [f32; 3] = if l > 0.0 { std::array::from_fn(|i| x[i] / l * r.half) } else { [0.0; 3] };
	let p = |o: [f32; 3], s: f32| -> [f32; 3] { std::array::from_fn(|i| o[i] + off[i] * s) };
	let (mut c0, mut c1) = (pool_rgba(r.c_head), pool_rgba(r.c_tail));
	c0[3] *= fade;
	c1[3] *= fade;
	crate::prims::WorldQuad {
		pos: [p(a, -1.0), p(a, 1.0), p(b, r.scale), p(b, -r.scale)],
		uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
		colour: [c0, c0, c1, c1],
		tex: (crate::viewer::PART_TEX + r.tex as u32) as usize,
		additive: r.additive,
	}
}

/// One entry of the frame's particle walk (`pool_walk`): an index into the sprites, the pool ribbons (mode 3) or the
/// pool lines (mode 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolItem {
	Sprite(usize),
	Ribbon(usize),
	Line(usize),
}

/// Bucket of a view depth: `min(ftoi12(vz) >> 10, 0x3ff)` = floor(4 vz) (negative depths are culled before).
fn bucket(vz: f32) -> u32 {
	((vz * 4096.0) as i32 >> 10).clamp(0, 0x3ff) as u32
}

/// Draw order of the frame's particles: every non-pool sprite first, in list order (the queue-A glows run before the
/// pool, weapon_fx.md root cause 2), then every pool slot in retail's order, whatever its mode. Retail pool draw Eudora
/// FUN_00259e18 (= Veldin FUN_00267628) [C: overlay.elf.c 81836-81985 sort, 81992-82005 drain, 82087-82380 build]:
/// the sort pass walks the slots by rising index; modes 0/1 (flags +1 bit 1 clear) key on the slot position's view
/// depth, modes 2/3 (bit 1 set) on the depth of the midpoint of +0x10 and +0x20 (vf25.z = 0.5 on both points); BOTH
/// branches push the slot onto the head of the same bucket list `min(ftoi12(key) >> 10, 0x3ff)` at SPR 0x3000. The
/// drain reads the heads from bucket 0x3ff down to 0 into ONE index list, and the packet build walks that list once,
/// dispatching each slot by `flags & 3` (sprite, flat, line, ribbon) into one GIF stream. So the pool is drawn back to
/// front and, within one bucket, by falling slot index, with lines and ribbons interleaved among the sprites (stage 9;
/// stage 4 left modes 2/3 in separate passes). `sprite_slots` maps the n-th pool sprite to its slot (retail.rs
/// `queue_fx`); without it (or past its end) the list index stands in. Blended and additive slots interleave in that
/// order; drawing them grouped by blend put the Pyrocitor's additive glows over its blended flame frames (stage 4).
pub fn pool_walk(
	sprites: &[crate::prims::Sprite],
	sprite_slots: &[u16],
	ribbons: &[PoolRibbon],
	lines: &[crate::prims::Line],
	line_slots: &[u16],
	eye: [f32; 3],
	f: [f32; 3],
) -> Vec<PoolItem> {
	let mut order: Vec<PoolItem> = (0..sprites.len()).filter(|&i| sprites[i].pool.is_none()).map(PoolItem::Sprite).collect();
	let mid = |a: [f32; 3], b: [f32; 3]| dot(sub([(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, (a[2] + b[2]) * 0.5], eye), f);
	let mut pool: Vec<(u32, u32, PoolItem)> = Vec::new();
	for (n, i) in (0..sprites.len()).filter(|&i| sprites[i].pool.is_some()).enumerate() {
		let slot = sprite_slots.get(n).map_or(0x800 + i as u32, |&s| s as u32);
		pool.push((bucket(dot(sub(sprites[i].pos, eye), f)), slot, PoolItem::Sprite(i)));
	}
	for (i, r) in ribbons.iter().enumerate() {
		pool.push((bucket(mid(r.head, r.tail)), r.slot as u32, PoolItem::Ribbon(i)));
	}
	for (i, l) in lines.iter().enumerate() {
		let slot = line_slots.get(i).map_or(0x800 + i as u32, |&s| s as u32);
		pool.push((bucket(mid(l.a, l.b)), slot, PoolItem::Line(i)));
	}
	pool.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
	order.extend(pool.into_iter().map(|(_, _, it)| it));
	order
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// [PC] Fire: an explosion the pack reports for a field that no blast-type pool sprite shows (R&C1: a thrown bomb that went off,
/// rac1/retail/bomb_blast.rs). `id` stays the same while the explosion lasts (the ground fire lights once per id), `size` is in
/// the scale of a blast sprite's size (viewer/firefx_blast.rs `land`: 3 = a fire bed of the default radius).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlastPoint {
	pub id: u32,
	pub pos: [f32; 3],
	pub size: f32,
}

/// One field's effect pool as the particle pass reads it (viewer/gpu/particle_fx.rs `set_particles`; R&C1 builds it from its
/// `RetailFx`, retail/fx_queue.rs `pool_fx`): the pool slot of each pool sprite (the sprites are in the frame's sprite list),
/// the mode-3 ribbons and mode-2 lines with their slots, and for the Water Enhanced contact rings the level frame counter, the
/// hero's position and the moving mobys (class, position, scale).
#[derive(Clone, Copy)]
pub struct PoolFx<'a> {
	pub sprite_slots: &'a [u16],
	/// The effect type (slot byte +0) and particle texture (byte +2) of each pool sprite, parallel to `sprite_slots` (volumetric smoke, `Pack::smoke`); may be empty.
	pub sprite_types: &'a [u8],
	pub sprite_texs: &'a [u8],
	pub ribbons: &'a [PoolRibbon],
	pub lines: &'a [crate::prims::Line],
	pub line_slots: &'a [u16],
	pub frames: u32,
	pub hero: Option<[f32; 3]>,
	pub movers: &'a [(u32, [f32; 3], f32)],
	/// [PC] Weapon lights: the pack's light points of this field (engine/shot_rows.rs `ShotPoint`, viewer/shotlight.rs); may be empty.
	pub shots: &'a [crate::shot_rows::ShotPoint],
	/// [PC] Fire: the pack's explosions of this field that are no pool sprites (`BlastPoint`); may be empty.
	pub blasts: &'a [BlastPoint],
	/// [PC] With frame smoothing: the shown field's own pool sprites and hero position, before the smoothing blended them with the
	/// field before (parallel to `sprite_slots`; empty / None = the sprites as given). The Pyrocitor's jets are built from these and
	/// blended once, node by node (viewer/firefx_jet_lerp.rs): built from the blended sprites and blended again, the stream stood
	/// still on the first display frame of each field and then caught up (lane nfire, agent 6: "the pyrocitor fire is still stuttery").
	pub tick: &'a [crate::prims::Sprite],
	pub tick_hero: Option<[f32; 3]>,
}
