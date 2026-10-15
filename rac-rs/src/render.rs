//! A dependency-free software rasteriser for checking decoded geometry: perspective camera,
//! depth buffer, flat Lambert shading, a stable colour per texture index. Writes PNG.

use crate::level::Texture;
use crate::mesh::Mesh;

pub struct Image {
	pub w: usize,
	pub h: usize,
	pub rgb: Vec<u8>,
}

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: V3, b: V3) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: V3) -> V3 {
	let l = dot(a, a).sqrt().max(1e-9);
	[a[0] / l, a[1] / l, a[2] / l]
}

fn tex_colour(t: u32) -> V3 {
	let h = t.wrapping_mul(2_654_435_761);
	let c = |s: u32| 0.45 + ((h >> s) & 0xff) as f32 / 255.0 * 0.5;
	[c(0), c(8), c(16)]
}

/// Render meshes (Z up) from above at `pitch_deg` looking at the scene centre, yaw `yaw_deg`.
pub fn render(meshes: &[Mesh], textures: &std::collections::HashMap<u32, Texture>, w: usize, h: usize, yaw_deg: f32, pitch_deg: f32) -> Image {
	let mut lo = [f32::MAX; 3];
	let mut hi = [f32::MIN; 3];
	for m in meshes {
		for p in &m.positions {
			for j in 0..3 {
				lo[j] = lo[j].min(p[j]);
				hi[j] = hi[j].max(p[j]);
			}
		}
	}
	let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
	let radius = dot(sub(hi, lo), sub(hi, lo)).sqrt() / 2.0;
	let (yaw, pitch) = (yaw_deg.to_radians(), pitch_deg.to_radians());
	let fov = 50f32.to_radians();
	let dist = radius / (fov / 2.0).tan() * 0.8;
	let dir = [yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin()];
	let eye = [centre[0] + dir[0] * dist, centre[1] + dir[1] * dist, centre[2] + dir[2] * dist];
	let fwd = norm(sub(centre, eye));
	let right = norm(cross(fwd, [0.0, 0.0, 1.0]));
	let up = cross(right, fwd);
	let f = (h as f32 / 2.0) / (fov / 2.0).tan();
	let light = norm([0.4, 0.3, 1.0]);

	let mut depth = vec![f32::MAX; w * h];
	let mut rgb = vec![0u8; w * h * 3];
	for (i, px) in rgb.chunks_exact_mut(3).enumerate() {
		let t = (i / w) as f32 / h as f32;
		px.copy_from_slice(&[(40.0 + 60.0 * t) as u8, (50.0 + 70.0 * t) as u8, (80.0 + 90.0 * t) as u8]);
	}
	for m in meshes {
		let proj: Vec<Option<[f32; 3]>> = m
			.positions
			.iter()
			.map(|&p| {
				let d = sub(p, eye);
				let z = dot(d, fwd);
				(z > 0.1).then(|| [w as f32 / 2.0 + dot(d, right) * f / z, h as f32 / 2.0 - dot(d, up) * f / z, z])
			})
			.collect();
		for &(tex, [a, b, c]) in &m.faces {
			let tex = tex & crate::mobies::KEY_MASK;
			let (Some(pa), Some(pb), Some(pc)) = (proj[a as usize], proj[b as usize], proj[c as usize]) else { continue };
			let n = norm(cross(sub(m.positions[b as usize], m.positions[a as usize]), sub(m.positions[c as usize], m.positions[a as usize])));
			let shade = 0.45 + 0.55 * dot(n, light).abs();
			let flat = tex_colour(tex).map(|x| (x * shade * 255.0).min(255.0) as u8);
			let texture = textures.get(&tex);
			let (ua, ub, uc) = (m.uvs[a as usize], m.uvs[b as usize], m.uvs[c as usize]);
			let minx = pa[0].min(pb[0]).min(pc[0]).floor().max(0.0) as usize;
			let maxx = pa[0].max(pb[0]).max(pc[0]).ceil().min(w as f32 - 1.0);
			let miny = pa[1].min(pb[1]).min(pc[1]).floor().max(0.0) as usize;
			let maxy = pa[1].max(pb[1]).max(pc[1]).ceil().min(h as f32 - 1.0);
			if maxx < 0.0 || maxy < 0.0 {
				continue;
			}
			let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0]);
			if area.abs() < 1e-6 {
				continue;
			}
			for y in miny..=maxy as usize {
				for x in minx..=maxx as usize {
					let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
					let w0 = ((pb[0] - px) * (pc[1] - py) - (pb[1] - py) * (pc[0] - px)) / area;
					let w1 = ((pc[0] - px) * (pa[1] - py) - (pc[1] - py) * (pa[0] - px)) / area;
					let w2 = 1.0 - w0 - w1;
					if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
						continue;
					}
					// Perspective-correct: interpolate 1/z and attribute/z in screen space.
					let (ia, ib, ic) = (w0 / pa[2], w1 / pb[2], w2 / pc[2]);
					let iz = ia + ib + ic;
					let z = 1.0 / iz;
					let k = y * w + x;
					if z >= depth[k] {
						continue;
					}
					let col = match texture {
						Some(t) => {
							let u = (ia * ua[0] + ib * ub[0] + ic * uc[0]) * z;
							let v = (ia * ua[1] + ib * ub[1] + ic * uc[1]) * z;
							let s = t.sample(u, v);
							if s[3] < 0x40 {
								continue; // cutout
							}
							[s[0], s[1], s[2]].map(|x| (x as f32 * shade * 1.6).min(255.0) as u8)
						}
						None => flat,
					};
					depth[k] = z;
					rgb[k * 3..k * 3 + 3].copy_from_slice(&col);
				}
			}
		}
	}
	Image { w, h, rgb }
}

fn crc32(data: &[u8]) -> u32 {
	let mut c = !0u32;
	for &b in data {
		c ^= b as u32;
		for _ in 0..8 {
			c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
		}
	}
	!c
}

/// PNG with uncompressed (stored) deflate blocks: larger files, no dependencies.
pub fn png(img: &Image) -> Vec<u8> {
	let mut raw = Vec::with_capacity((img.w * 3 + 1) * img.h);
	for row in img.rgb.chunks_exact(img.w * 3) {
		raw.push(0);
		raw.extend_from_slice(row);
	}
	let mut z = vec![0x78, 0x01];
	let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
	for (i, b) in blocks.iter().enumerate() {
		z.push((i + 1 == blocks.len()) as u8);
		z.extend_from_slice(&(b.len() as u16).to_le_bytes());
		z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
		z.extend_from_slice(b);
	}
	let (mut s1, mut s2) = (1u32, 0u32);
	for &b in &raw {
		s1 = (s1 + b as u32) % 65521;
		s2 = (s2 + s1) % 65521;
	}
	z.extend_from_slice(&((s2 << 16) | s1).to_be_bytes());
	let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
	let mut chunk = |kind: &[u8], payload: &[u8]| {
		out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
		let mut body = kind.to_vec();
		body.extend_from_slice(payload);
		out.extend_from_slice(&body);
		out.extend_from_slice(&crc32(&body).to_be_bytes());
	};
	let mut ihdr = Vec::new();
	ihdr.extend_from_slice(&(img.w as u32).to_be_bytes());
	ihdr.extend_from_slice(&(img.h as u32).to_be_bytes());
	ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
	chunk(b"IHDR", &ihdr);
	chunk(b"IDAT", &z);
	chunk(b"IEND", &[]);
	out
}
