//! The fur noise: a small tiling texture of strand coverage, made once on the CPU, with its whole mip chain, so the strands are
//! filtered like any texture (trilinear + anisotropic): fine strands up close, a smooth plush surface from afar, no shimmer.
//!
//! RGBA = the fraction of the texel covered by strands at the heights 1/8, 3/8, 5/8 and 7/8 of the fur length (fs_fur blends the
//! two channels around a shell's height). Strands stand on a jittered grid, each with its own length and root radius, and taper
//! towards the tip; coverage is supersampled 4 x 4 per texel.

use std::sync::OnceLock;

/// Texels per side of the top level.
pub(super) const SIZE: usize = 256;
/// Strand grid pitch in texels (~7200 strands per tile).
const PITCH: f32 = 3.0;
const SS: usize = 4;
const HEIGHTS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];

/// Every mip level, the largest first, as tightly packed RGBA8 rows.
pub(super) fn levels() -> &'static [Vec<u8>] {
	static L: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
	L.get_or_init(build)
}

fn build() -> Vec<Vec<u8>> {
	let n = SIZE * SS;
	let mut cov = vec![0u8; n * n];
	let mut seed = 0x2545_f491u32;
	let mut rnd = || {
		seed ^= seed << 13;
		seed ^= seed >> 17;
		seed ^= seed << 5;
		seed as f32 / u32::MAX as f32
	};
	let cells = (SIZE as f32 / PITCH) as usize;
	for gy in 0..cells {
		for gx in 0..cells {
			let cx = (gx as f32 + 0.15 + 0.7 * rnd()) * PITCH * SS as f32;
			let cy = (gy as f32 + 0.15 + 0.7 * rnd()) * PITCH * SS as f32;
			let len = 0.45 + 0.55 * rnd();
			// Root discs of 0.65..0.95 pitch radius overlap their neighbours: the coat is closed at the root (no skin between strands).
			let r0 = (0.65 + 0.3 * rnd()) * PITCH * SS as f32;
			let reach = r0.ceil() as isize + 1;
			// The strand's radius at each height (0 = the strand is shorter: nothing is covered, d < 0 never holds), once per strand
			// instead of per sample; the same f32 values the per-sample form computed, so the texture is byte-identical.
			let rks = HEIGHTS.map(|h| if h < len { r0 * (1.0 - h / len).powf(0.75) } else { 0.0 });
			for dy in -reach..=reach {
				for dx in -reach..=reach {
					let (px, py) = ((cx as isize + dx).rem_euclid(n as isize) as usize, (cy as isize + dy).rem_euclid(n as isize) as usize);
					let (ox, oy) = ((cx as isize + dx) as f32 + 0.5 - cx, (cy as isize + dy) as f32 + 0.5 - cy);
					let d = (ox * ox + oy * oy).sqrt();
					for (k, &rk) in rks.iter().enumerate() {
						if d < rk {
							cov[py * n + px] |= 1 << k;
						}
					}
				}
			}
		}
	}
	let mut top = vec![0u8; SIZE * SIZE * 4];
	for y in 0..SIZE {
		for x in 0..SIZE {
			for k in 0..4 {
				let hits: usize = (0..SS * SS).filter(|s| cov[(y * SS + s / SS) * n + x * SS + s % SS] >> k & 1 != 0).count();
				top[(y * SIZE + x) * 4 + k] = (hits * 255 / (SS * SS)) as u8;
			}
		}
	}
	let mut out = vec![top];
	let mut size = SIZE;
	while size > 1 {
		let prev = out.last().unwrap();
		let half = size / 2;
		let mut next = vec![0u8; half * half * 4];
		for y in 0..half {
			for x in 0..half {
				for c in 0..4 {
					let at = |xx: usize, yy: usize| prev[(yy * size + xx) * 4 + c] as u32;
					next[(y * half + x) * 4 + c] = ((at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1) + 2) / 4) as u8;
				}
			}
		}
		out.push(next);
		size = half;
	}
	out
}

/// The noise as a GPU texture with all its mips.
pub(super) fn texture(device: &wgpu::Device, queue: &super::super::presenter::GatedQueue) -> wgpu::Texture {
	let lv = levels();
	let tex = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("fur noise"),
		size: wgpu::Extent3d { width: SIZE as u32, height: SIZE as u32, depth_or_array_layers: 1 },
		mip_level_count: lv.len() as u32,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: wgpu::TextureFormat::Rgba8Unorm,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	});
	for (m, data) in lv.iter().enumerate() {
		let s = (SIZE >> m) as u32;
		let dst = wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: m as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All };
		queue.write_texture(dst, data, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * s), rows_per_image: Some(s) }, wgpu::Extent3d { width: s, height: s, depth_or_array_layers: 1 });
	}
	tex
}

/// The per-sample form this file used before the radii were hoisted (kept to prove the texture is byte-identical).
#[cfg(test)]
fn build_reference() -> Vec<Vec<u8>> {
	let n = SIZE * SS;
	let mut cov = vec![0u8; n * n];
	let mut seed = 0x2545_f491u32;
	let mut rnd = || {
		seed ^= seed << 13;
		seed ^= seed >> 17;
		seed ^= seed << 5;
		seed as f32 / u32::MAX as f32
	};
	let cells = (SIZE as f32 / PITCH) as usize;
	for gy in 0..cells {
		for gx in 0..cells {
			let cx = (gx as f32 + 0.15 + 0.7 * rnd()) * PITCH * SS as f32;
			let cy = (gy as f32 + 0.15 + 0.7 * rnd()) * PITCH * SS as f32;
			let len = 0.45 + 0.55 * rnd();
			// Root discs of 0.65..0.95 pitch radius overlap their neighbours: the coat is closed at the root (no skin between strands).
			let r0 = (0.65 + 0.3 * rnd()) * PITCH * SS as f32;
			let reach = r0.ceil() as isize + 1;
			for dy in -reach..=reach {
				for dx in -reach..=reach {
					let (px, py) = ((cx as isize + dx).rem_euclid(n as isize) as usize, (cy as isize + dy).rem_euclid(n as isize) as usize);
					let (ox, oy) = ((cx as isize + dx) as f32 + 0.5 - cx, (cy as isize + dy) as f32 + 0.5 - cy);
					let d = (ox * ox + oy * oy).sqrt();
					for (k, &h) in HEIGHTS.iter().enumerate() {
						if h < len && d < r0 * (1.0 - h / len).powf(0.75) {
							cov[py * n + px] |= 1 << k;
						}
					}
				}
			}
		}
	}
	let mut top = vec![0u8; SIZE * SIZE * 4];
	for y in 0..SIZE {
		for x in 0..SIZE {
			for k in 0..4 {
				let hits: usize = (0..SS * SS).filter(|s| cov[(y * SS + s / SS) * n + x * SS + s % SS] >> k & 1 != 0).count();
				top[(y * SIZE + x) * 4 + k] = (hits * 255 / (SS * SS)) as u8;
			}
		}
	}
	let mut out = vec![top];
	let mut size = SIZE;
	while size > 1 {
		let prev = out.last().unwrap();
		let half = size / 2;
		let mut next = vec![0u8; half * half * 4];
		for y in 0..half {
			for x in 0..half {
				for c in 0..4 {
					let at = |xx: usize, yy: usize| prev[(yy * size + xx) * 4 + c] as u32;
					next[(y * half + x) * 4 + c] = ((at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1) + 2) / 4) as u8;
				}
			}
		}
		out.push(next);
		size = half;
	}
	out
}


#[cfg(test)]
mod tests {
	fn fnv(l: &[Vec<u8>]) -> u64 {
		let mut h = 0xcbf2_9ce4_8422_2325u64;
		for b in l.iter().flatten() {
			h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
		}
		h
	}

	#[test]
	fn the_hoisted_radii_give_the_same_bytes_as_the_per_sample_form() {
		let new = super::build();
		let old = super::build_reference();
		println!("fur noise fnv new {:016x} old {:016x}", fnv(&new), fnv(&old));
		assert!(new == old, "fur noise differs");
	}
}
