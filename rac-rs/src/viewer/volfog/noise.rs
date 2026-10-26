//! Shared by the volumetric passes (fog now, clouds later): the tiling 3D noise volume the shaders read. Generated on the CPU
//! once (64^3, four channels, no file), uploaded as an `Rgba8Unorm` 3D texture with a repeating, trilinear sampler.
//!
//! Channels: R = value-noise fbm (3 octaves, the big soft billows), G = inverted cellular (Worley) noise (puffy cell
//! shapes, cloud material), B = a finer value-noise fbm (wisps and erosion detail), A = very low frequency value noise (coverage
//! maps). Every channel tiles at the volume edge (lattice periods divide the size).

/// Voxels per side.
pub const SIZE: u32 = 64;

fn hash(x: i32, y: i32, z: i32, seed: u32) -> u32 {
	let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f) ^ seed.wrapping_mul(0x9e37_79b9);
	h ^= h >> 15;
	h = h.wrapping_mul(0x2c1b_3c6d);
	h ^= h >> 12;
	h = h.wrapping_mul(0x297a_2d39);
	h ^ (h >> 15)
}

fn unit(h: u32) -> f32 {
	(h >> 8) as f32 / (1u32 << 24) as f32
}

fn wrap(i: i32, period: i32) -> i32 {
	i.rem_euclid(period)
}

/// Value noise on a lattice of `period` cells per volume side, at volume coordinate `p` in 0..1 (tiles).
pub fn value(p: [f32; 3], period: i32, seed: u32) -> f32 {
	let q = p.map(|c| c * period as f32);
	let i = q.map(|c| c.floor());
	let f = [q[0] - i[0], q[1] - i[1], q[2] - i[2]].map(|t| t * t * (3.0 - 2.0 * t));
	let at = |dx: i32, dy: i32, dz: i32| unit(hash(wrap(i[0] as i32 + dx, period), wrap(i[1] as i32 + dy, period), wrap(i[2] as i32 + dz, period), seed));
	let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
	let x = |dy, dz| lerp(at(0, dy, dz), at(1, dy, dz), f[0]);
	let y = |dz| lerp(x(0, dz), x(1, dz), f[1]);
	lerp(y(0), y(1), f[2])
}

/// Fractal value noise: `octaves` doublings from `period`, each half as strong, normalised to 0..1.
pub fn fbm(p: [f32; 3], period: i32, octaves: u32, seed: u32) -> f32 {
	let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
	for o in 0..octaves {
		sum += amp * value(p, period << o, seed + o);
		norm += amp;
		amp *= 0.5;
	}
	sum / norm
}

/// Cellular noise: distance to the nearest of one jittered point per cell, `period` cells per side (tiles), inverted so cell
/// centres are bright. 0..1.
pub fn cellular(p: [f32; 3], period: i32, seed: u32) -> f32 {
	let q = p.map(|c| c * period as f32);
	let i = q.map(|c| c.floor() as i32);
	let mut best = 1.0e9f32;
	for dz in -1..=1 {
		for dy in -1..=1 {
			for dx in -1..=1 {
				let c = [i[0] + dx, i[1] + dy, i[2] + dz];
				let h = hash(wrap(c[0], period), wrap(c[1], period), wrap(c[2], period), seed);
				let o = [unit(h), unit(h.wrapping_mul(0x9e37_79b1)), unit(h.rotate_left(13).wrapping_mul(0x85eb_ca6b))];
				let d = [0, 1, 2].map(|k| c[k] as f32 + o[k] - q[k]);
				best = best.min(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
			}
		}
	}
	1.0 - best.sqrt().min(1.0)
}

/// The volume, x fastest then y then z, four bytes per voxel.
pub fn generate(size: u32) -> Vec<u8> {
	let n = size as usize;
	let mut out = Vec::with_capacity(n * n * n * 4);
	let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
	for z in 0..n {
		for y in 0..n {
			for x in 0..n {
				let p = [x as f32 / n as f32, y as f32 / n as f32, z as f32 / n as f32];
				out.extend([to_u8(fbm(p, 4, 3, 1)), to_u8(cellular(p, 6, 7)), to_u8(fbm(p, 8, 3, 11)), to_u8(value(p, 2, 21))]);
			}
		}
	}
	out
}

/// The volume as a 3D texture and its repeating trilinear sampler.
pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue) -> (wgpu::TextureView, wgpu::Sampler) {
	let data = generate(SIZE);
	let tex = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("volume noise"),
		size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: SIZE },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D3,
		format: wgpu::TextureFormat::Rgba8Unorm,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	});
	queue.write_texture(
		wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
		&data,
		wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(SIZE * 4), rows_per_image: Some(SIZE) },
		wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: SIZE },
	);
	let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
		label: Some("volume noise sampler"),
		address_mode_u: wgpu::AddressMode::Repeat,
		address_mode_v: wgpu::AddressMode::Repeat,
		address_mode_w: wgpu::AddressMode::Repeat,
		mag_filter: wgpu::FilterMode::Linear,
		min_filter: wgpu::FilterMode::Linear,
		..Default::default()
	});
	(tex.create_view(&Default::default()), sampler)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn channels_tile_and_stay_in_range() {
		for (a, b) in [([0.0, 0.3, 0.7], [1.0, 0.3, 0.7]), ([0.2, 0.0, 0.5], [0.2, 1.0, 0.5]), ([0.9, 0.1, 0.0], [0.9, 0.1, 1.0])] {
			assert!((fbm(a, 4, 3, 1) - fbm(b, 4, 3, 1)).abs() < 1e-4, "fbm tiles");
			assert!((cellular(a, 6, 7) - cellular(b, 6, 7)).abs() < 1e-4, "cellular tiles");
			assert!((value(a, 2, 21) - value(b, 2, 21)).abs() < 1e-4, "value tiles");
		}
		let data = generate(8);
		assert_eq!(data.len(), 8 * 8 * 8 * 4);
		let mean = data.iter().step_by(4).map(|&b| b as f32).sum::<f32>() / 512.0;
		assert!((60.0..200.0).contains(&mean), "fbm channel is mid-range, mean {mean}");
	}
}
