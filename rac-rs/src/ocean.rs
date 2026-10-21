//! Level oceans (Aridia: decomp/notes/aridia_presentation.md ARI-49a/49b; Kerwan: decomp/notes/kerwan_environment.md
//! KER-30): groups of flat patch descriptors from the level overlay, each drawn as 7x7-cell patches (an 8x8 vertex
//! grid, one 124-vertex triangle strip) at the descriptor's height, textured with a cross-faded palette animation from
//! the level fx table and modulated by per-patch vertex colours. No wave field, no RNG. `OceanDef` holds what differs
//! per level.

use crate::gameplay::Cuboid;
use crate::level::Level;
use crate::particles::WorldStrip;

/// Aridia's image below the zero-filled gap [0x162320, 0x166300) is 0x90 bytes shorter than the ELF (byte comparison of
/// the image against extracted/levels/2_aridia/overlay.elf, 2026-10-07: base 0x15eef0 below the gap, the image base
/// above it). The two small descriptors' masks (0x161e30, 0x161e38) live below it; reading them at the image base gave
/// zero masks, so the pit's two descriptors drew nothing (the quicksand channel rendered as a black void).
const GAP: usize = 0x164000;

/// Overlay virtual address `va` in the level image (`Level::sections[0]`): the def's low base below `GAP` when it has
/// one, its image base otherwise.
fn elf_at<'a>(img: &'a [u8], va: usize, len: usize, def: &OceanDef) -> Option<&'a [u8]> {
	if crate::retail::disc::usa() {
		return crate::retail::disc::ov_bytes(img, va, len);
	}
	let base = match def.low_base {
		Some(low) if va < GAP => low,
		_ => def.image_base,
	};
	img.get(va.checked_sub(base)?..)?.get(..len)
}

/// The Aridia ocean moby class (one placement; update FUN_002e9ac8, draw callback FUN_002e9978).
pub const CLASS: u32 = 854;

/// Per-level ocean constants.
pub struct OceanDef {
	/// Descriptor overlay addresses per draw group (Aridia: one group per class-854 pvar word, FUN_002e9978 order).
	pub groups: [&'static [usize]; 3],
	/// Per-strip-vertex ST table (248 f32) and its scale (the ocean_st_scale argument).
	pub st_table: usize,
	pub st_scale: f32,
	/// Strip order: 124 s16 byte offsets into the 8x8 grid (/ 8 = grid index i + 8j).
	pub strip_index: usize,
	/// Texture animation: first fx record and frame count.
	pub first_fx: usize,
	pub frame_count: usize,
	/// GS ALPHA FIX byte (descriptor +0x3f): 0x80 = opaque, below = `(Cs - Cd) * fix / 128 + Cd`, no Z write.
	pub fix: u8,
	/// Groups drawn only while the eye is inside their pvar region cuboid (Aridia); Kerwan draws its one
	/// descriptor unconditionally.
	pub gated: bool,
	/// Overlay virtual address of `Level::sections[0]` byte 0, and the base below `GAP` where the image is shifted.
	pub image_base: usize,
	pub low_base: Option<usize>,
	/// Ocean fog words: near/far distance (world units = words / 1024), intensity near/far, FOGCOL. The values are
	/// baked into the fragment shaders (`fs_ocean` Aridia, `fs_ocean_k` Kerwan in viewer.rs; tested against them).
	pub fog: ([f32; 2], [f32; 2], [u8; 3]),
	/// First `Game.frames` value on which the ocean is drawn (Kerwan: update call 1 only initialises, call 2 queues
	/// the first draw).
	pub first_frame: u32,
}

/// Aridia (class 854): three region-gated groups, opaque, Z-writing.
pub const ARIDIA: OceanDef = OceanDef {
	groups: [&[0x1f3b00], &[0x1f3bd0, 0x1f3c30, 0x1f3c90, 0x1f3cd0, 0x1f3d10], &[0x1f3b60]],
	st_table: 0x1cb410,
	st_scale: 0.16666667,
	strip_index: 0x1cb300,
	first_fx: 40,
	frame_count: 64,
	fix: 0x80,
	gated: true,
	// Novalis 0x15ef38, Veldin 0x15ef70: byte search of descriptor 0x1f3b00 in `Level::sections[0]`, 2026-10-06.
	image_base: 0x15ef80,
	low_base: Some(0x15eef0),
	fog: ([0.0, 260080.0 / 1024.0], [255.0, 0.0], [15, 15, 25]),
	first_frame: 0,
};

/// Kerwan (class 994, update 0x2dc2e0, draw 0x291248): one ungated descriptor, FIX 0x20 blend without Z write.
pub const KERWAN: OceanDef = OceanDef {
	groups: [&[0x1dc0a0], &[], &[]],
	st_table: 0x1caf10,
	st_scale: 0.6666667,
	strip_index: 0x1cae00,
	first_fx: 40,
	frame_count: 4,
	fix: 0x20,
	gated: false,
	image_base: 0x15ef98,
	low_base: None,
	fog: ([10000.0 / 1024.0, 260080.0 / 1024.0], [255.0, 0.0], [96, 96, 96]),
	first_frame: 2,
};

pub const STRIP_LEN: usize = 124;
/// Cells per patch side.
const CELLS: u16 = 7;
/// Texture animation: game frames per step (descriptor +0x33, 30 on Aridia and Kerwan).
const STEP: i32 = 30;
/// One animation frame: 64x64 PSMT8.
const TEXELS: usize = 4096;
/// Colour block per patch slot: a 16-byte VIF header then 124 RGBA words.
const COLOUR_STRIDE: usize = 0x200;
const COLOUR_SKIP: usize = 0x10;

/// The GPU blend constant of a FIX-blended ocean: FIX / 128.
pub fn blend_constant(fix: u8) -> f64 {
	fix as f64 / 128.0
}

/// One overlay patch descriptor (0x40 bytes).
#[derive(Clone, Debug)]
pub struct Desc {
	pub origin: [f32; 3],
	pub spacing: [f32; 2],
	pub cells: [u16; 2],
	/// +0xc: squared patch-centre draw distance (160000.0 = radius 400).
	pub radius2: f32,
	/// One byte per patch, row-major (0 = no patch).
	pub mask: Vec<u8>,
	/// 124 colours per patch slot (R, G, B, 0), slot-major.
	pub colours: Vec<[u8; 4]>,
}

impl Desc {
	fn grid(&self) -> (usize, usize) {
		(self.cells[0].div_ceil(CELLS) as usize, self.cells[1].div_ceil(CELLS) as usize)
	}

	fn read(ov: &[u8], va: usize, def: &OceanDef) -> Option<Desc> {
		let b = elf_at(ov, va, 0x40, def)?;
		let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
		let w = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
		let h = |o: usize| u16::from_le_bytes(b[o..o + 2].try_into().unwrap());
		let cells = [h(0x18), h(0x1a)];
		let slots = cells[0].div_ceil(CELLS) as usize * cells[1].div_ceil(CELLS) as usize;
		let mask = elf_at(ov, w(0x38), slots, def)?.to_vec();
		let raw = elf_at(ov, w(0x34), slots * COLOUR_STRIDE, def)?;
		let colours = (0..slots).flat_map(|s| (0..STRIP_LEN).map(move |k| s * COLOUR_STRIDE + COLOUR_SKIP + k * 4)).map(|o| raw[o..o + 4].try_into().unwrap()).collect();
		Some(Desc { origin: [f(0), f(4), f(8)], spacing: [f(0x10), f(0x14)], cells, radius2: f(0xc), mask, colours })
	}
}

/// The level's ocean: descriptors, strip tables and the 64 animation frames (raw palette, pixels).
#[derive(Debug)]
pub struct Ocean {
	pub groups: [Vec<Desc>; 3],
	/// Class-854 pvar words 0..2: region cuboid ids (-1 = never, unless the global override is on).
	pub cuboid: [i32; 3],
	pub st: [[f32; 2]; STRIP_LEN],
	pub idx: [u8; STRIP_LEN],
	pub frames: Vec<([[u8; 4]; 256], Vec<u8>)>,
	/// GS ALPHA FIX byte carried into every strip (`OceanDef::fix`).
	pub fix: u8,
	/// Region-cuboid gate of the groups (`OceanDef::gated`).
	pub gated: bool,
	/// `OceanDef::first_frame`.
	pub first_frame: u32,
	/// Retail mode (queue_q1::ocean, built from RAM): the descriptor's FOGCOL (+0x30), drawn by `fs_ocean_f` with the
	/// Kerwan fog words. None = the shader-baked fog of Aridia / Kerwan.
	pub fog: Option<[u8; 3]>,
}

impl Ocean {
	/// `ov` = the level overlay image (`Level::sections[0]`), `pvars` = the ocean instance's pvars (Aridia's region
	/// cuboid ids; unused when the def is not gated). None when the overlay or the def's fx records are missing.
	pub fn new(ov: &[u8], level: &Level, pvars: &[u8], def: &OceanDef) -> Option<Ocean> {
		crate::viewer::gs_state::guard_path("ocean");
		let mut groups: [Vec<Desc>; 3] = Default::default();
		for (g, list) in def.groups.iter().enumerate() {
			for &va in *list {
				groups[g].push(Desc::read(ov, crate::retail::disc::va_level(level.id, va as u32) as usize, def)?);
			}
		}
		let word = |k: usize| pvars.get(k * 4..k * 4 + 4).map_or(-1, |b| i32::from_le_bytes(b.try_into().unwrap()));
		let cuboid = [word(0), word(1), word(2)];
		let raw = elf_at(ov, crate::retail::disc::va_level(level.id, def.st_table as u32) as usize, STRIP_LEN * 8, def)?;
		let f = |o: usize| f32::from_le_bytes(raw[o..o + 4].try_into().unwrap());
		let st = std::array::from_fn(|k| [f(k * 8) * def.st_scale, f(k * 8 + 4) * def.st_scale]);
		let raw = elf_at(ov, crate::retail::disc::va_level(level.id, def.strip_index as u32) as usize, STRIP_LEN * 2, def)?;
		let idx = std::array::from_fn(|k| (i16::from_le_bytes([raw[k * 2], raw[k * 2 + 1]]) / 8) as u8);
		let frames = (0..def.frame_count).map(|k| fx_frame(level, def.first_fx + k)).collect::<Option<Vec<_>>>()?;
		Some(Ocean { groups, cuboid, st, idx, frames, fix: def.fix, gated: def.gated, first_frame: def.first_frame, fog: None })
	}

	/// Whether the ocean is drawn at `Game.frames` = `frames` completed ticks (Kerwan: nothing on the moby's init tick).
	pub fn shown(&self, frames: u32) -> bool {
		frames >= self.first_frame
	}

	/// FUN_0024f838: the 64x64 texture of game frame `frames` (`Game.frames`, one step per 50 Hz tick),
	/// records A and B cross-faded by the fraction of the 30-frame step, truncated per channel.
	pub fn texture(&self, frames: u32) -> Vec<[u8; 4]> {
		let (n, t) = blend_at(frames, self.frames.len());
		let (a, b) = (&self.frames[n.0], &self.frames[n.1]);
		(0..TEXELS)
			.map(|i| {
				let (pa, pb) = (a.0[a.1[i] as usize], b.0[b.1[i] as usize]);
				std::array::from_fn(|c| mix(pa[c], pb[c], t))
			})
			.collect()
	}

	/// FUN_002e9978 / 24f2e0 / 24f618: one 124-vertex strip per drawn patch, groups 0, 1, 2 in order, descriptors
	/// in order, patches row-major. `all` = the global draw-everything mode (0x15f5e8).
	pub fn patches(&self, eye: [f32; 3], cuboids: &[Cuboid], all: bool) -> Vec<WorldStrip> {
		let mut out = Vec::new();
		for (g, descs) in self.groups.iter().enumerate() {
			let id = self.cuboid[g];
			if self.gated && !(all || (id != -1 && cuboids.get(id as usize).is_some_and(|c| c.contains(eye)))) {
				continue;
			}
			for d in descs {
				self.desc_patches(d, eye, &mut out);
			}
		}
		out
	}

	fn desc_patches(&self, d: &Desc, eye: [f32; 3], out: &mut Vec<WorldStrip>) {
		let (cols, rows) = d.grid();
		let [ox, oy, oz] = d.origin;
		let [sx, sy] = d.spacing;
		let (step_x, step_y) = (sx * CELLS as f32, sy * CELLS as f32);
		let mut py = oy;
		for r in 0..rows {
			let mut px = ox;
			for c in 0..cols {
				let slot = r * cols + c;
				if d.mask[slot] != 0 && patch_near(px, py, d, eye) {
					out.push(self.strip(d, slot, px, py));
				}
				px += step_x;
			}
			py += step_y;
		}
	}

	fn strip(&self, d: &Desc, slot: usize, px: f32, py: f32) -> WorldStrip {
		let xs = grid_line(px, d.spacing[0]);
		let ys = grid_line(py, d.spacing[1]);
		let pos = self.idx.iter().map(|&g| [xs[g as usize % 8], ys[g as usize / 8], d.origin[2]]).collect();
		let colour = d.colours[slot * STRIP_LEN..(slot + 1) * STRIP_LEN].iter().map(|c| [c[0] as f32 / 128.0, c[1] as f32 / 128.0, c[2] as f32 / 128.0, 0.0]).collect();
		WorldStrip { pos, uv: self.st.to_vec(), colour, tex: 0, fix: self.fix, add: false }
	}
}

/// fx record `k` (level index 0x58 count, 0x5c table, 0x68 base; record = palette offset +0, pixel offset +4):
/// the raw 256-entry palette (entry v at byte v * 4, no CSM1 swizzle) and the 4096 pixel bytes.
fn fx_frame(level: &Level, k: usize) -> Option<([[u8; 4]; 256], Vec<u8>)> {
	let u = |d: &[u8], o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
	let (count, table, base) = (u(&level.index, 0x58)?, u(&level.index, 0x5c)?, u(&level.index, 0x68)?);
	if k >= count {
		return None;
	}
	let (pal, pix) = (u(&level.index, table + k * 16)?, u(&level.index, table + k * 16 + 4)?);
	let p = level.core.get(base + pal..base + pal + 1024)?;
	let palette = std::array::from_fn(|v| p[v * 4..v * 4 + 4].try_into().unwrap());
	Some((palette, level.core.get(base + pix..base + pix + TEXELS)?.to_vec()))
}

/// ((record A, record B) as frame indices, blend fraction t) at game frame `frames` of a `count`-frame animation.
fn blend_at(frames: u32, count: usize) -> ((usize, usize), f32) {
	let n = frames as i32;
	let q = n / STEP;
	let t = n as f32 / STEP as f32 - q as f32;
	let count = count as i32;
	(((q % count) as usize, ((q + 1) % count) as usize), t)
}

pub(crate) fn mix(a: u8, b: u8, t: f32) -> u8 {
	(a as f32 * (1.0 - t) + b as f32 * t) as i32 as u8
}

/// The 8 grid coordinates of a patch row/column: `start` plus `step` added i times (f32 adds).
fn grid_line(start: f32, step: f32) -> [f32; 8] {
	let mut v = [start; 8];
	for i in 1..8 {
		v[i] = v[i - 1] + step;
	}
	v
}

/// FUN_0024f2e0 distance test: patch centre (px + 3.5 sx, py + 3.5 sy, oz), strictly inside radius2.
fn patch_near(px: f32, py: f32, d: &Desc, eye: [f32; 3]) -> bool {
	let c = [px + d.spacing[0] * CELLS as f32 * 0.5, py + d.spacing[1] * CELLS as f32 * 0.5, d.origin[2]];
	let (dx, dy, dz) = (c[0] - eye[0], c[1] - eye[1], c[2] - eye[2]);
	dx * dx + dy * dy + dz * dz - d.radius2 < 0.0
}

/// Retail mode (retail/oldsim.rs `RamOcean`, the oceans of Batalia, Gaspar, Oltanis, Rilgar and Umbris): the patch strips of
/// one descriptor whose strip tables, ALPHA FIX and colours were read from retail RAM. Never gated by a region cuboid.
pub fn ram_strips(d: &Desc, st: &[[f32; 2]; STRIP_LEN], idx: &[u8; STRIP_LEN], fix: u8, eye: [f32; 3]) -> Vec<WorldStrip> {
	crate::viewer::gs_state::guard_path("ocean");
	let o = Ocean { groups: Default::default(), cuboid: [-1; 3], st: *st, idx: *idx, frames: Vec::new(), fix, gated: false, first_frame: 0, fog: None };
	let mut out = Vec::new();
	o.desc_patches(d, eye, &mut out);
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{iso, toc};

	const ISO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");

	fn aridia() -> (Level, Ocean) {
		let mut disc = iso::Disc::open(std::path::Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[2]).unwrap();
		let pv: Vec<u8> = [46i32, 48, 49, -1].iter().flat_map(|v| v.to_le_bytes()).collect();
		let ov = l.sections[0].as_ref().unwrap();
		let o = Ocean::new(ov, &l, &pv, &ARIDIA).unwrap();
		(l, o)
	}

	fn kerwan() -> (Level, Ocean) {
		let mut disc = iso::Disc::open(std::path::Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[3]).unwrap();
		let ov = l.sections[0].as_ref().unwrap();
		// KER-30 (7): fx records 40..43 exist, so the Kerwan ocean builds.
		let o = Ocean::new(ov, &l, &[], &KERWAN).unwrap();
		(l, o)
	}

	/// KER-30: the Kerwan descriptor 0x1dc0a0 read at image base 0x15ef98, its ST scale and 4-frame animation.
	#[test]
	fn kerwan_ocean_tables() {
		let (l, o) = kerwan();
		let ov = l.sections[0].as_ref().unwrap();
		let b = elf_at(ov, 0x1dc0a0, 12, &KERWAN).unwrap();
		let f = |k: usize| f32::from_le_bytes(b[k * 4..k * 4 + 4].try_into().unwrap());
		assert_eq!([f(0), f(1), f(2)], [-260.0, -220.0, -24.0]);
		// (1) st[0] = -3 * 0.6666667 = -2.0 (f32 bits 0x3f2aaaab).
		assert_eq!(KERWAN.st_scale.to_bits(), 0x3f2aaaab);
		assert_eq!(ARIDIA.st_scale.to_bits(), 0x3e2aaaab);
		assert_eq!(f32::from_bits(0xc0400000) * KERWAN.st_scale, f32::from_bits(0xc0000000));
		assert_eq!(o.st[0][0], -2.0);
		// (3) one descriptor, 42 x 35 cells (word 0x0023002a = 2293802) -> 6 x 5 = 30 patches, every mask byte 1.
		assert_eq!(o.groups[0].len(), 1);
		assert!(o.groups[1].is_empty() && o.groups[2].is_empty());
		let d = &o.groups[0][0];
		assert_eq!(d.origin, [-260.0, -220.0, -24.0]);
		assert_eq!((d.spacing, d.cells, d.radius2), ([40.0, 40.0], [42, 35], 160000.0));
		assert_eq!(d.grid(), (6, 5));
		assert_eq!(d.mask, vec![1; 30]);
		assert_eq!(d.colours[0], [0x79, 0x79, 0x79, 0]);
		// (4) four animation frames.
		assert_eq!(o.frames.len(), 4);
		// (6) FIX 0x20 = 25 % source, Aridia opaque.
		assert_eq!((o.fix, o.gated), (0x20, false));
		assert_eq!(blend_constant(o.fix), 0.25);
	}

	/// KER-30 (3): no cuboid gate; patch (0, 0) is centred at (-120, -80, -24), so an eye there draws it first.
	#[test]
	fn kerwan_patches_ungated() {
		let (_, o) = kerwan();
		let eye = [-120.0, -80.0, -24.0];
		let s = o.patches(eye, &[], false);
		assert!(!s.is_empty());
		assert!(s.iter().all(|s| s.pos.len() == STRIP_LEN && s.fix == 0x20));
		// First vertex of patch (0, 0) is its grid corner (-260, -220).
		assert_eq!(s[0].pos.iter().fold([f32::MAX; 2], |m, p| [m[0].min(p[0]), m[1].min(p[1])]), [-260.0, -220.0]);
		// The radius-400 distance test still applies: an eye far from every patch centre draws nothing.
		assert!(o.patches([-10000.0, -10000.0, -24.0], &[], false).is_empty());
	}

	/// KER-30 (2): update call 1 only initialises (state 0 -> 1), call 2 queues the first draw: drawn from
	/// `Game.frames` 2. Aridia keeps drawing from frame 0.
	#[test]
	fn kerwan_one_tick_gate() {
		let (_, o) = kerwan();
		assert!(!o.shown(0) && !o.shown(1) && o.shown(2));
		assert_eq!(ARIDIA.first_frame, 0);
	}

	/// KER-30: the shader fog constants (viewer.rs fs_ocean / fs_ocean_k) are the defs' fog words: Aridia slope over
	/// 253.98438 from 0, Kerwan over 244.21875 from 9.765625; restore (test 5) is trivial, the level fog is never written.
	#[test]
	fn ocean_fog_words() {
		let ([n, f], [i0, i1], col) = ARIDIA.fog;
		assert_eq!((n, f - n, i0, i1, col), (0.0, 253.98438, 255.0, 0.0, [15, 15, 25]));
		let ([n, f], [i0, i1], col) = KERWAN.fog;
		assert_eq!((n, f - n, i0, i1, col), (9.765625, 244.21875, 255.0, 0.0, [96, 96, 96]));
		let src = include_str!("viewer.rs");
		assert!(src.contains("(d - 9.765625) * (0.0 - 255.0) / 244.21875") && src.contains("d * (0.0 - 255.0) / 253.98438"));
	}

	/// KER-30 (4): the 4-frame animation cycles records 40..43, pairs (0,1), (1,2), (2,3), (3,0).
	#[test]
	fn kerwan_texture_cycle() {
		assert_eq!(blend_at(0, 4).0, (0, 1));
		assert_eq!(blend_at(30, 4).0, (1, 2));
		assert_eq!(blend_at(60, 4).0, (2, 3));
		assert_eq!(blend_at(90, 4).0, (3, 0));
		assert_eq!(blend_at(120, 4).0, (0, 1));
	}

	#[test]
	fn aridia_ocean_tables() {
		let (_, o) = aridia();
		// (2) ST table: 2-decimal ELF values / 6; st[1].t is -0.35666668 in f32 (1 ulp off the EE's truncation).
		assert_eq!(o.st[0], [-0.5, -0.5]);
		assert_eq!(o.st[1][0], -0.5);
		assert!((o.st[1][1] - -0.35666665f32).abs() <= f32::EPSILON * 0.5, "{}", o.st[1][1]);
		assert_eq!(o.st[123], [0.5, 0.5]);
		// (3) strip order.
		assert_eq!(o.idx[0..4], [0, 8, 1, 9]);
		assert_eq!(o.idx[16..18], [15, 8]);
		// Descriptor 0x1f3b00.
		let d = &o.groups[0][0];
		assert_eq!(d.origin, [132.600006, 133.460007, 24.77]);
		assert_eq!((d.spacing, d.cells, d.radius2), ([2.62, 2.62], [84, 70], 160000.0));
		assert_eq!(d.grid(), (12, 10));
		// (5) slot 0 colour 0.
		assert_eq!(d.colours[0], [0x1b, 0x1b, 0x1a, 0]);
		assert_eq!(o.cuboid, [46, 48, 49]);
		assert_eq!(o.frames.len(), 64);
	}

	/// The pit descriptors 0x1f3cd0 / 0x1f3d10 keep their masks at 0x161e30 / 0x161e38 (below the image's 0x90-byte
	/// shift): both patches are drawn (the quicksand channel was a black void with zero masks).
	#[test]
	fn pit_descriptor_masks() {
		let (_, o) = aridia();
		assert_eq!(o.groups[1][3].mask, vec![1, 1]);
		assert_eq!(o.groups[1][4].mask, vec![1]);
		assert_eq!(o.groups[1][3].colours.len(), 2 * STRIP_LEN);
	}

	#[test]
	fn patch_column_steps() {
		// (1) 0x1f3b00 column 4: px by f32 adds of 7 * 2.62, then grid point 7.
		let step = 2.62f32 * 7.0;
		let mut px = 132.600006f32;
		for _ in 0..4 {
			px += step;
		}
		assert_eq!(px, 205.95999);
		assert_eq!(grid_line(px, 2.62)[7], 224.29996);
	}

	#[test]
	fn texture_blend_steps() {
		// (4) frames 45: A = 41, B = 42 (indices 1, 2), t = 0.5; frames 50: t = 0.6666666, 200 / 100 -> 133.
		assert_eq!(blend_at(45, 64), ((1, 2), 0.5));
		let ((a, b), t) = blend_at(50, 64);
		assert_eq!(((a, b), t), ((1, 2), 0.6666666));
		assert_eq!(mix(200, 100, t), 133);
		// Wraps: step 63 blends record 103 into record 40.
		assert_eq!(blend_at(63 * 30, 64).0, (63, 0));
		// Spec 49b (3): the record pair changes at frame 30, frames 29 and 30 differ only through t / the pair switch.
		assert_eq!(blend_at(29, 64).0, (0, 1));
		assert_eq!(blend_at(30, 64), ((1, 2), 0.0));
	}

	#[test]
	fn aridia_patches_by_region() {
		let (l, o) = aridia();
		let cuboids = crate::gameplay::cuboids(&l).unwrap();
		// (6) camera outside all three cuboids, mode 0: nothing.
		let far = [-10000.0, -10000.0, -10000.0];
		assert!(o.cuboid.iter().all(|&id| !cuboids[id as usize].contains(far)));
		assert!(o.patches(far, &cuboids, false).is_empty());
		// Inside cuboid 46 at a 0x1f3b00 patch: 124-vertex strips (49b test 2).
		let eye = cuboids[46].pos;
		let s = o.patches(eye, &cuboids, false);
		assert!(!s.is_empty());
		assert!(s.iter().all(|s| s.pos.len() == STRIP_LEN && s.uv.len() == STRIP_LEN && s.colour.len() == STRIP_LEN && s.fix == 0x80));
		// The override draws every group near the eye.
		assert!(o.patches(eye, &cuboids, true).len() >= s.len());
		// Texture: 4096 texels.
		assert_eq!(o.texture(0).len(), TEXELS);
		assert_ne!(o.texture(0), o.texture(15));
	}
}
