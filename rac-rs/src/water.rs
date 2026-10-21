//! Ripple-heightfield water (decomp/notes/water.md, decomp/notes/novalis_environment.md "Water surface
//! rendering"): class 751's 21 patches of 16x16 cells, a two-step wave simulation, region activation by
//! cuboid, and the two-pass (scrolling water + sphere-map) draw as world quads.
//!
//! The patch descriptors are read from the level's overlay image (`Level::sections[0]`); the addresses in
//! `Layout` are Novalis's. Other planets' overlays are not mapped [P].

use crate::particles::{EffectMoby, WorldQuad};

/// NOV-20.3: the falling water drop (class 787, FUN_002ff6f0) and the splash ring (class 775, FUN_002ff140).
pub const DROP_CLASS: u32 = 787;
pub const RING_CLASS: u32 = 775;

/// A class-787 drop: per-tick vz (pvar+8) and accel (+0x10), angles +0x40 / +0x48, rot rate (+0x1c), counter
/// (+0x14), splashed flag (+0x18) and the scale factor over the class template.
#[derive(Clone, Debug)]
pub struct WaterDrop {
	pub pos: [f32; 3],
	pub angle_x: f32,
	pub angle_z: f32,
	pub scale: f32,
}

/// A class-775 splash ring: scale factor and alpha byte +0x23.
#[derive(Clone, Debug)]
pub struct WaterRing {
	pub pos: [f32; 3],
	pub scale: f32,
	pub alpha: u8,
	/// Rotation about z (+0x48), the one `Rng::angle` draw of the constructor.
	pub yaw: f32,
}

/// Where a planet's overlay keeps the water data (virtual addresses).
#[derive(Clone, Copy, Debug)]
pub struct Layout {
	pub patches: usize,
	pub count: usize,
	pub stride: usize,
	/// Visible-subblock masks: one u16 every 4 bytes, per patch.
	pub masks: usize,
	/// The class-751 instance's class id.
	pub class: u32,
	/// (first patch, patch count, random chance) of each activation region; region i is gated by pvar i.
	pub regions: &'static [(usize, usize, u32)],
}

pub const NOVALIS: Layout = Layout {
	patches: 0x1e33c0,
	count: 21,
	stride: 0x1190,
	masks: 0x1fa490,
	class: 751,
	regions: &[(0, 4, 200), (4, 2, 2000), (6, 6, 2000), (12, 1, 2000), (13, 0, 0), (13, 4, 200), (17, 1, 2000)],
};

/// Cells per patch side; vertices per side is one more.
const N: usize = 16;
/// Padded grid: rows/cols -1..=17 (halo for neighbour publish and the normal's 5-point cross).
const P: usize = N + 3;
/// Patch lighting: |L| per axis, flat-water colour constants (RippleVertex FUN_00261990).
const LIGHT_AXIS: f32 = 0.57735;




/// Water height under (x, y, z) from the displayed buffer (FUN_002b8240 via FUN_0026e8d0): the first patch
/// whose 16x16 box (x1024 integer bounds, z within -1..+1 of the patch z) holds the point and whose
/// subblock bit is shown; bilinear between the four cell heights plus the patch z. None = no patch (the
/// caller falls back to the flat plane / its own surface).
fn surface_query<'a>(patches: impl Iterator<Item = ([f32; 3], u16, &'a [f32])>, x: f32, y: f32, z: f32) -> Option<f32> {
	let (ix, iy, iz) = ((x * 1024.0) as i32, (y * 1024.0) as i32, (z * 1024.0) as i32);
	for (pos, mask, buf) in patches {
		let (x0, y0, z0) = (((pos[0] - 8.0) * 1024.0) as i32, ((pos[1] - 8.0) * 1024.0) as i32, ((pos[2] - 1.0) * 1024.0) as i32);
		let w = (N as f32 * 1024.0) as i32;
		if !(x0 <= ix && y0 <= iy && z0 <= iz && ix < x0 + w && iy < y0 + w && iz < z0 + 0x800) {
			continue;
		}
		let (lx, ly) = (x - (pos[0] - 8.0), y - (pos[1] - 8.0));
		let (cx, cy) = ((lx as isize).clamp(0, N as isize - 1), (ly as isize).clamp(0, N as isize - 1));
		if mask & (1 << ((cx >> 2 & 3) | (cy & 0xc))) == 0 {
			continue;
		}
		// Column/row 16 reads the neighbour halo (published edge cells).
		let h = |r: isize, c: isize| buf[at(r, c)];
		let (fx, fy) = (lx - cx as f32, ly - cy as f32);
		let (h00, h10, h01, h11) = (h(cy, cx), h(cy, cx + 1), h(cy + 1, cx), h(cy + 1, cx + 1));
		let a = h00 + (h10 - h00) * fx;
		let b = h01 + (h11 - h01) * fx;
		return Some(a + (b - a) * fy + pos[2]);
	}
	None
}

/// One patch's static descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct Patch {
	pub pos: [f32; 3],
	/// Neighbour patch ids: (0,-16) (-16,0) (0,+16) (+16,0) (-16,-16) (+16,-16) (-16,+16) (+16,+16); 0xff = none.
	pub nb: [u8; 8],
	/// fx texture of the sphere-map pass and of the scrolling pass.
	pub tex_sphere: usize,
	pub tex_water: usize,
	/// ALPHA_1 FIX bytes of the sphere-map and water passes (blend = lerp(Cd, Cs, FIX/128)).
	pub fix_sphere: u8,
	pub fix_water: u8,
	pub mask: u16,
	/// UV accumulators, speeds, wobble angles/rates/amplitude, edge-pin flags.
	pub scroll: [f32; 2],
	pub speed: [f32; 2],
	pub ang: [f32; 2],
	pub ang_rate: [f32; 2],
	pub amp: f32,
	pub pin: u8,
}

/// The level's overlay section is the overlay's loaded image minus its first 0x38 bytes: virtual
/// address `va` is at offset `va - IMAGE_BASE` [C: NOV-18.1 byte comparison with the extracted overlay.elf].
const IMAGE_BASE: usize = 0x15ef38;

fn elf_at(img: &[u8], va: usize, len: usize) -> Option<&[u8]> {
	// USA: the layout is Novalis's (level 1).
	if crate::retail::disc::usa() {
		return crate::retail::disc::ov_bytes(img, crate::retail::disc::va_level(1, va as u32) as usize, len);
	}
	img.get(va.checked_sub(IMAGE_BASE)?..)?.get(..len)
}

/// Read the patch table and masks from a level overlay image.
pub fn parse_patches(elf: &[u8], lay: &Layout) -> Option<Vec<Patch>> {
	let table = elf_at(elf, lay.patches, lay.stride * lay.count)?;
	let masks = elf_at(elf, lay.masks, lay.count * 4)?;
	let f = |b: &[u8], o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
	let i = |b: &[u8], o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
	Some(
		(0..lay.count)
			.map(|k| {
				let b = &table[k * lay.stride..(k + 1) * lay.stride];
				let mut p = Patch {
					pos: [f(b, 0), f(b, 4), f(b, 8)],
					nb: b[12..20].try_into().unwrap(),
					tex_sphere: i(b, 0x14).max(0) as usize,
					tex_water: i(b, 0x18).max(0) as usize,
					fix_sphere: b[0x1c],
					fix_water: b[0x1d],
					mask: 0,
					// FUN_002b7378 defaults.
					scroll: [0.0; 2],
					speed: [-0.0016; 2],
					ang: [std::f32::consts::FRAC_PI_2, -std::f32::consts::FRAC_PI_2],
					ang_rate: [0.02, -0.031],
					amp: 0.03,
					pin: 0,
				};
				// Patches 13..16 scroll in u only; patch 15 pins vertex row 16 (bit 8).
				if (13..=16).contains(&k) {
					p.speed[1] = 0.0;
				}
				if k == 15 {
					p.pin = 8;
				}
				p
			})
			.collect(),
	)
	.map(|mut v: Vec<Patch>| {
		for (k, p) in v.iter_mut().enumerate() {
			p.mask = u16::from_le_bytes([masks[k * 4], masks[k * 4 + 1]]);
		}
		v
	})
}

fn at(r: isize, c: isize) -> usize {
	((r + 1) as usize) * P + (c + 1) as usize
}

/// NOV-28: the fog while the camera is underwater: near 0 m, far 32 m, GS intensity 255 at near and 48 at far.
pub const UNDERWATER_FOG: [f32; 4] = [0.0, 32.0, 255.0, 48.0];

/// NOV-28.2: pool-particle far cap while the camera is dry (int 0x1f4000, 12-bit fixed, FUN_00218c70).
pub const PART_FAR: f32 = 500.0;
/// NOV-28.2: pool-particle far cap while the camera is underwater (int 0x40000).
pub const PART_FAR_UNDER: f32 = 64.0;

/// NOV-28.2: a pool particle's far distance in m: byte +9 high nibble * 32 m, capped by the live clamp
/// (`pminw((b9 & 0xf0) << 13, clamp)`, FUN_0027c190).
pub fn pool_far(b9: u8, clamp: f32) -> f32 {
	((b9 >> 4) as f32 * 32.0).min(clamp)
}

/// NOV-28: the tint sprite blends by its alpha byte over 128 (GS ALPHA 0x44, As/128).
pub fn tint_alpha(a: u8) -> f32 {
	a as f32 / 128.0
}

/// NOV-28.3: a GS colour for the gamma-space Unorm target: the raw byte / 255, as the GS blends and fogs it.
pub fn gs_rgb(c: [u8; 3]) -> [f32; 3] {
	[c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]
}

/// NOV-28.3: the submerged tint rect colour: `gs_rgb` of the rgb bytes, alpha byte / 128 (`tint_alpha`).
pub fn tint_colour(t: [u8; 4]) -> [f32; 4] {
	let c = gs_rgb([t[0], t[1], t[2]]);
	[c[0], c[1], c[2], tint_alpha(t[3])]
}

pub struct Water {
	patches: Vec<Patch>,
	/// Three padded height grids per patch.
	bufs: Vec<[Vec<f32>; 3]>,
	/// Output buffer: 2 between steps, `prev` on a step tick.
	out: usize,
	// NOV-20.4: highest containing region, including regions with no patches.
	/// NOV-28: full-screen tint (r, g, b, alpha /128) and underwater fog colour, set by the active region and
	/// sticky while no region contains the eye.
	tint: [u8; 4],
	under_fog: [u8; 3],
	/// Light direction used by the vertex colour (towards the surface), None = no light.
	light: [f32; 3],
	pub drops: Vec<WaterDrop>,
	pub rings: Vec<WaterRing>,
	pub sounds: Vec<[f32; 3]>,
}


/// Vertex colour grey in byte / 128 units: `m` is the used (down-pointing) normal, `l` the light travel direction,
/// q = l.m, t = clamp((q - 0.5729) * 3.25 + 0.5729, 0, 1), byte = 0x40 + floor(128 t) (water.md, FUN_00261990).
fn colour_grey(l: [f32; 3], m: [f32; 3]) -> f32 {
	let q = l[0] * m[0] + l[1] * m[1] + l[2] * m[2];
	let t = ((q - 0.5729) * 3.25 + 0.5729).clamp(0.0, 1.0);
	(64.0 + (128.0 * t).floor()) / 128.0
}

/// NOV-21.1: I = -unit(eye - vertex); reflect when the dot sign bit is clear.
/// UV = R.xy * 0.9 * 0.5 + 0.5; the negative-sign branch uses R = -I.
fn sphere_uv(pos: [f32; 3], eye: [f32; 3], m: [f32; 3]) -> [f32; 2] {
	let e = [eye[0] - pos[0], eye[1] - pos[1], eye[2] - pos[2]];
	let q = 1.0 / (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt();
	let u = [e[0] * q, e[1] * q, e[2] * q];
	let iv = [0.0 - u[0], 0.0 - u[1], 0.0 - u[2]];
	let d = iv[0] * m[0] + iv[1] * m[1] + iv[2] * m[2];
	let r = if d.is_sign_negative() { [u[0], u[1]] } else {
		let v = [m[0] * d - iv[0], m[1] * d - iv[1]];
		[v[0] * 2.0 + iv[0], v[1] * 2.0 + iv[1]]
	};
	[r[0] * 0.9 * 0.5 + 0.5, r[1] * 0.9 * 0.5 + 0.5]
}

impl Water {
	pub fn new(elf: &[u8], lay: Layout, pvars: &[u8], to_light: Option<[f32; 3]>) -> Option<Water> {
		let patches = parse_patches(elf, &lay)?;
		let full: Vec<u16> = patches.iter().map(|p| p.mask).collect();
		let region_ids: Vec<i32> = (0..lay.regions.len()).map(|i| pvars.get(4 * i..4 * i + 4).map_or(-1, |b| i32::from_le_bytes(b.try_into().unwrap()))).collect();
		// FUN_002fca18: L = (cos t, sin t) * 0.57735, -0.57735 with t the to-light azimuth.
		let light = to_light.map_or([0.0; 3], |l| {
			let t = l[1].atan2(l[0]);
			[t.cos() * LIGHT_AXIS, t.sin() * LIGHT_AXIS, -LIGHT_AXIS]
		});
		let bufs = patches.iter().map(|_| std::array::from_fn(|_| vec![0.0; P * P])).collect();
		let mut w = Water { patches,  bufs,   out: 2,      tint: [0x18, 0x30, 0x90, 0x40], under_fog: [0x18, 0x30, 0x90], light,   drops: Vec::new(), rings: Vec::new(), sounds: Vec::new() };
		for p in &mut w.patches {
			p.mask = 0;
		}
		Some(w)
	}








	/// NOV-28: the submerged full-screen tint (rgb, alpha with 0x80 = 1) and the underwater fog colour.
	pub fn submerged(&self) -> ([u8; 4], [u8; 3]) {
		(self.tint, self.under_fog)
	}





	/// The drops and rings as effect mobies (scale relative to the class template; ring tint FUN_00264c68(0x28,
	/// 0x28, 0x46) OV:158260).
	pub fn effect_mobys(&self) -> Vec<EffectMoby> {
		let drops = self.drops.iter().map(|d| EffectMoby { class: DROP_CLASS, pos: d.pos, rot: [d.angle_x, 0.0, d.angle_z], scale: d.scale, alpha: 128.0, tint: [1.0; 3], draw: None });
		let rings = self.rings.iter().map(|r| EffectMoby { class: RING_CLASS, pos: r.pos, rot: [0.0, 0.0, r.yaw], scale: r.scale, alpha: r.alpha as f32, tint: [0x28 as f32 / 128.0, 0x28 as f32 / 128.0, 0x46 as f32 / 128.0], draw: None });
		drops.chain(rings).collect()
	}



	/// Water height under (x, y, z) (`surface_query`).
	pub fn surface_z(&self, x: f32, y: f32, z: f32) -> Option<f32> {
		// NOV-10.1: one query shared with `WaterSurface` so the two cannot diverge.
		surface_query(self.patches.iter().enumerate().map(|(k, p)| (p.pos, p.mask, &self.bufs[k][self.out][..])), x, y, z)
	}



	/// Emit both passes of every shown patch's visible 4x4 subblocks as world quads.
	pub fn quads(&self, eye: [f32; 3], out: &mut Vec<WorldQuad>) {
		emit_quads(&self.patches, &|k| &self.bufs[k][self.out][..], self.light, eye, out);
	}
}

/// One patch read from retail RAM (retail mode): the descriptor and its displayed height grid in the port's padded layout.
#[derive(Clone, Debug)]
pub struct RamPatch {
	pub patch: Patch,
	pub grid: Vec<f32>,
}

/// The padded grid (`at(r, c)`) from the 368-float retail height buffer (layout decoded from the DMA FUN_002617a8, Novalis
/// numbering; the five-point normal reads only the cells listed here).
pub fn grid_from_buffer(b: &[f32]) -> Vec<f32> {
	let mut g = vec![0.0; P * P];
	let n = N as isize;
	for r in 0..n {
		for c in 0..n {
			g[at(r, c)] = b[(16 * r + c) as usize];
		}
	}
	for k in 0..n {
		g[at(-1, k)] = b[256 + k as usize];
		g[at(n, k)] = b[288 + k as usize];
		g[at(n + 1, k)] = b[320 + k as usize];
		g[at(k, -1)] = b[272 + k as usize];
		g[at(k, n)] = b[304 + k as usize];
		g[at(k, n + 1)] = b[336 + k as usize];
	}
	g[at(-1, -1)] = b[352];
	g[at(-1, n)] = b[353];
	g[at(n, -1)] = b[354];
	g[at(n, n)] = b[355];
	g[at(n, n + 1)] = b[356];
	g[at(n + 1, n)] = b[357];
	g
}

/// Both passes of every shown patch (`p.mask != 0`) as world quads; `grid(k)` is patch k's displayed padded height grid.
pub fn emit_quads<'a>(patches: &[Patch], grid: &dyn Fn(usize) -> &'a [f32], light: [f32; 3], eye: [f32; 3], out: &mut Vec<WorldQuad>) {
	{
		// Wobble selector of strip row r (rows r and r+1 of a subblock) at column c (0x1cb0b0 laid out per strip).
		const SEL: [[u8; 5]; 4] = [[0, 1, 1, 0, 0], [1, 0, 0, 1, 0], [1, 0, 0, 1, 1], [0, 1, 1, 0, 0]];
		for (k, p) in patches.iter().enumerate() {
			if p.mask == 0 {
				continue;
			}
			let g = grid(k);
			let h = |r: isize, c: isize| g[at(r, c)];
			// Vertex (i, j), 0..=16: position, colour grey, sphere-map uv.
			let vert = |i: isize, j: isize| {
				let (hc, hl, hr, hu, hd) = (h(j, i), h(j, i - 1), h(j, i + 1), h(j - 1, i), h(j + 1, i));
				let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
				let (v10, v11, v12, v13) = ([0.0, -1.0, hc - hd], [-1.0, 1.0, hd - hr], [0.0, 1.0, hc - hu], [1.0, -1.0, hu - hl]);
				let (a, b) = (cross(v11, v10), cross(v13, v12));
				let s = [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
				let l = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt().max(1e-9);
				let nrm = [-s[0] / l, -s[1] / l, -s[2] / l];
				let mut z = hc;
				if (p.pin & 1 != 0 && j == 0) || (p.pin & 2 != 0 && i == N as isize) || (p.pin & 4 != 0 && j == N as isize) || (p.pin & 8 != 0 && i == 0) {
					z = 0.0;
				}
				let pos = [p.pos[0] - 8.0 + i as f32, p.pos[1] - 8.0 + j as f32, p.pos[2] + z];
				let uv = sphere_uv(pos, eye, nrm);
				(pos, colour_grey(light, nrm), uv)
			};
			let grid: Vec<_> = (0..=N as isize).flat_map(|j| (0..=N as isize).map(move |i| (i, j))).map(|(i, j)| vert(i, j)).collect();
			let vx = |i: usize, j: usize| &grid[j * (N + 1) + i];
			let wob = |sel: u8| {
				let a = p.ang[sel as usize];
				[p.scroll[0] + p.amp * a.cos(), p.scroll[1] + p.amp * a.sin()]
			};
			for by in 0..4usize {
				for bx in 0..4usize {
					if p.mask >> (by * 4 + bx) & 1 == 0 {
						continue;
					}
					for cy in 0..4usize {
						for cx in 0..4usize {
							let (i, j) = (bx * 4 + cx, by * 4 + cy);
							let corners = [(0usize, 0usize), (1, 0), (1, 1), (0, 1)];
							let vs: Vec<_> = corners.iter().map(|&(a, b)| vx(i + a, j + b)).collect();
							// Pass 1: scrolling water, UV = subblock-local grid + wobble of the strip's column selector.
							let mut uv1 = [[0.0f32; 2]; 4];
							for (m, &(a, b)) in corners.iter().enumerate() {
								let w = wob(SEL[cy][cx + a]);
								uv1[m] = [(cx + a) as f32 / 4.0 + w[0], (cy + b) as f32 / 4.0 + w[1]];
							}
							let grey: Vec<f32> = vs.iter().map(|v| v.1).collect();
							let colour = |fix: u8| std::array::from_fn(|m| [grey[m], grey[m], grey[m], fix as f32 / 128.0]);
							let pos = std::array::from_fn(|m| vs[m].0);
							out.push(WorldQuad { pos, uv: uv1, colour: colour(p.fix_water), tex: p.tex_water, additive: false });
							out.push(WorldQuad { pos, uv: std::array::from_fn(|m| vs[m].2), colour: colour(p.fix_sphere), tex: p.tex_sphere, additive: false });
						}
					}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{iso, level::Level, toc};
	use std::path::Path;

	const ISO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");

	/// A synthetic overlay image holding a 2-patch table and mask table.
	fn fake_elf(lay: &Layout) -> Vec<u8> {
		let mut e = vec![0u8; lay.masks + 16 - IMAGE_BASE];
		let put = |e: &mut Vec<u8>, va: usize, b: &[u8]| e[va - IMAGE_BASE..va - IMAGE_BASE + b.len()].copy_from_slice(b);
		for k in 0..lay.count {
			let a = lay.patches + k * lay.stride;
			put(&mut e, a, &(10.0 * k as f32).to_le_bytes());
			put(&mut e, a + 12, &[0xff; 8]);
			put(&mut e, a + 0x14, &40i32.to_le_bytes());
			put(&mut e, a + 0x18, &41i32.to_le_bytes());
			put(&mut e, a + 0x1c, &[0x1c, 0x44]);
			put(&mut e, lay.masks + k * 4, &0xeee0u16.to_le_bytes());
		}
		e
	}

	fn tiny() -> Layout {
		Layout { patches: 0x160000, count: 2, stride: 0x1190, masks: 0x163000, class: 751, regions: &[(0, 2, 0)] }
	}

	#[test]
	fn gs_colours_are_raw_bytes() {
		assert_eq!(gs_rgb([0x00, 0x28, 0x30]), [0.0, 0.15686275, 0.1882353]);
		assert_eq!(gs_rgb([5, 5, 15]), [0.019607844, 0.019607844, 0.05882353]);
		let t = tint_colour([0x00, 0x28, 0x30, 0x40]);
		assert_eq!(t, [0.0, 0.15686275, 0.1882353, 0.5]);
		// GS blend (Cs - Cd) * As + Cd of grey 128 under the tint.
		let out: Vec<f32> = (0..3).map(|i| (128.0 / 255.0 * (1.0 - t[3]) + t[i] * t[3]) * 255.0).collect();
		assert_eq!(out, [64.0, 84.0, 88.0]);
	}

	#[test]
	fn pool_far_clamps_underwater() {
		assert_eq!(PART_FAR, 0x1f4000 as f32 / 4096.0);
		assert_eq!(PART_FAR_UNDER, 0x40000 as f32 / 4096.0);
		assert_eq!(pool_far(0x74, PART_FAR_UNDER), 64.0);
		assert_eq!(pool_far(0x74, PART_FAR), 224.0);
		assert_eq!(pool_far(0xf0, PART_FAR), 480.0);
		assert_eq!(pool_far(0x00, PART_FAR_UNDER), 0.0);
		// The gpu.rs set_particles cull/fade with the clamped far (b9 0x74: near 1.0).
		let fade = |vz: f32, clamp: f32| {
			let (near, far) = ((0x74u8 & 15) as f32 / 4.0, pool_far(0x74, clamp));
			(vz > near && vz < far).then(|| (vz - near).min((far - vz) / 16.0).clamp(0.0, 1.0))
		};
		assert_eq!(fade(60.0, PART_FAR_UNDER), Some(0.25));
		assert_eq!(fade(100.0, PART_FAR_UNDER), None);
		assert_eq!(fade(100.0, PART_FAR), Some(1.0));
	}

	// NOV-20.3 tests 1, 5, 6.
	// NOV-20.3 test 2: a drop 0.01 above the water face ripples once, flags, 1 ring + 16 drops, one sound.
	// NOV-20.3 tests 3, 4.
	#[test]
	fn flat_colour_is_139() {
		let l = [0.57735 * 0.3, 0.57735 * 0.9, -0.57735];
		assert_eq!(colour_grey(l, [0.0, 0.0, -1.0]) * 128.0, 139.0);
		// Faces turned from the light clamp to the 0x40 floor, facing ones to 0xc0.
		assert_eq!(colour_grey(l, [0.0, 0.0, 1.0]) * 128.0, 64.0);
		assert_eq!(colour_grey([0.0, 0.0, -1.0], [0.0, 0.0, -1.0]) * 128.0, 192.0);
	}

	#[test]
	fn sphere_uv_reflects_only_facing() {
		// Eye straight above a flat surface (m down): xy of R is 0, the centre of the map.
		assert_eq!(sphere_uv([0.0; 3], [0.0, 0.0, 10.0], [0.0, 0.0, -1.0]), [0.5, 0.5]);
		// Oblique view: I = (1,0,-1)/sqrt2 reflects to R.x = I.x.. -> -(I.x) after 2(I.m)m - I.
		let uv = sphere_uv([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]);
		assert!((uv[0] - (0.5 - 0.45 * 0.70710678)).abs() < 1e-5 && (uv[1] - 0.5).abs() < 1e-6);
		// I.m < 0 (seen from below): no reflection.
		let uv = sphere_uv([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 0.0, -1.0]);
		assert!((uv[0] - (0.5 - 0.45 * 0.70710678)).abs() < 1e-5);
	}

	#[test]
	fn overlay_header_is_not_image_data() {
		// NOV-18.1: the byte before the loaded-image base cannot be addressed.
		assert!(elf_at(&[0; 64], 0x15ef37, 1).is_none());
		assert_eq!(elf_at(&[7; 64], IMAGE_BASE, 1), Some(&[7][..]));
	}

	#[test]
	fn parses_descriptors() {
		let lay = tiny();
		let p = parse_patches(&fake_elf(&lay), &lay).unwrap();
		assert_eq!(p.len(), 2);
		assert_eq!((p[1].pos[0], p[1].tex_sphere, p[1].tex_water, p[1].fix_sphere, p[1].fix_water, p[1].mask), (10.0, 40, 41, 0x1c, 0x44, 0xeee0));
		assert_eq!(p[0].nb, [0xff; 8]);
		assert!(parse_patches(&fake_elf(&lay)[..0x800], &lay).is_none());
	}

	#[test]
	fn surface_bilinear_and_mask() {
		let lay = tiny();
		let e = fake_elf(&lay);
		let mut w = Water::new(&e, lay, &[], None).unwrap();
		// Patch 0 at (0, 0, z 0): its box is x, y in [-8, 8), z in [-1, 1).
		w.out = 2;
		w.bufs[0][2][at(8, 8)] = 1.0;
		w.bufs[0][2][at(8, 9)] = 3.0;
		w.bufs[0][2][at(9, 8)] = 5.0;
		w.bufs[0][2][at(9, 9)] = 7.0;
		// Hidden subblock (mask 0): no surface.
		assert_eq!(w.surface_z(0.5, 0.5, 0.0), None);
		w.patches[0].mask = 0xffff;
		// Cell (8, 8) is x, y in [0, 1): weights 0.25 / 0.5.
		let z = w.surface_z(0.25, 0.5, 0.0).unwrap();
		assert!((z - (1.0 + 2.0 * 0.25 + 4.0 * 0.5 + 0.0 * 0.25 * 0.5 + 0.0)).abs() < 1e-5, "{z}");
		// Outside the z slab or the box: None.
		assert_eq!(w.surface_z(0.25, 0.5, 1.5), None);
		assert_eq!(w.surface_z(8.5, 0.5, 0.0), None);
	}

	// NOV-10.1
}
