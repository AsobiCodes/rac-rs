//! [PC] Water Enhanced: the planar mirror (no retail counterpart; Water Classic never runs it). Flat water shows a true
//! mirror image instead of the screen-space reflection, which can only reflect what is already on screen and smears at its
//! edges: once a frame the level is drawn a second time, mirrored about the height of the nearest flat water plane below
//! the eye, at half the window size, into the water pass's mirror target (`waterfx::WaterFx::mirror_target`). Drawn: the
//! sky, the world, ties, tfrags, the hero and the mobies (the shrubs, particles and effects are left out to keep it cheap).
//! An oblique near plane (Lengyel) clips everything below the water, so nothing under the surface shows in the mirror. The
//! water pass (`fs_water`) reads it on the pixels lying on that plane and keeps the screen-space reflection everywhere else.
//! The same drawing, unmirrored, makes the water bed (`Gpu::draw_bed`): the level without its water, so the water pass can
//! show what lies under the surface.

use super::*;

/// A water draw's flat plane (`plane_of`), as [height, x0, y0, x1, y1].
pub(super) type Plane = Option<[f32; 5]>;

/// The flat plane of a water mesh: the height most of its vertices share (to 0.25) and the XY box of the vertices on it,
/// as [height, x0, y0, x1, y1]; none when no height holds a quarter of the vertices (a river running downhill, waves).
pub(super) fn plane_of(pos: impl Iterator<Item = [f32; 3]> + Clone) -> Plane {
	let mut bins: HashMap<i32, usize> = HashMap::new();
	let mut n = 0;
	for p in pos.clone() {
		*bins.entry((p[2] * 4.0).round() as i32).or_default() += 1;
		n += 1;
	}
	let (&k, &c) = bins.iter().max_by_key(|(k, c)| (**c, -**k))?;
	// Neighbouring bins count too (a height on a bin edge).
	let c = c + bins.get(&(k - 1)).copied().unwrap_or(0) + bins.get(&(k + 1)).copied().unwrap_or(0);
	if c * 4 < n || n < 3 {
		return None;
	}
	let h = k as f32 / 4.0;
	let mut b = [h, f32::MAX, f32::MAX, f32::MIN, f32::MIN];
	for p in pos.filter(|p| (p[2] - h).abs() <= 0.5) {
		b[1] = b[1].min(p[0]);
		b[2] = b[2].min(p[1]);
		b[3] = b[3].max(p[0]);
		b[4] = b[4].max(p[1]);
	}
	Some(b)
}

/// The plane to mirror this frame: of the flat water planes below the eye, the nearest (horizontal distance to its box,
/// plus a quarter of the height above it).
pub(super) fn pick(draws: &[super::waterfx::WaterDraw], eye: [f32; 3]) -> Option<f32> {
	draws
		.iter()
		.filter_map(|d| d.plane)
		.filter(|b| eye[2] > b[0] + 0.3)
		.map(|b| {
			let dx = (b[1] - eye[0]).max(eye[0] - b[3]).max(0.0);
			let dy = (b[2] - eye[1]).max(eye[1] - b[4]).max(0.0);
			((dx * dx + dy * dy).sqrt() + 0.25 * (eye[2] - b[0]), b[0])
		})
		.min_by(|a, b| a.0.total_cmp(&b.0))
		.map(|(_, h)| h)
}

/// The mirror's own uniforms (the level's, with the mirrored camera) and their bind group (the level's layout).
pub(super) struct Mirror {
	ubuf: wgpu::Buffer,
	bind: wgpu::BindGroup,
}

impl Mirror {
	pub(super) fn new(device: &wgpu::Device, ulayout: &wgpu::BindGroupLayout, sun: &super::sunshadow::SunShadow) -> Mirror {
		let ubuf = buffer(device, "mirror uniforms", bytemuck::bytes_of(&Uniforms::zeroed()), wgpu::BufferUsages::UNIFORM);
		let bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("mirror uniforms"), layout: ulayout, entries: &sun.entries(&ubuf) });
		Mirror { ubuf, bind }
	}
}

/// Column-major a * b.
fn mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
	let mut o = [[0.0; 4]; 4];
	for c in 0..4 {
		for r in 0..4 {
			o[c][r] = (0..4).map(|k| a[k][r] * b[c][k]).sum();
		}
	}
	o
}

/// The inverse of a column-major 4x4 (cofactors), none when singular.
fn inverse(m: &[[f32; 4]; 4]) -> Option<[[f32; 4]; 4]> {
	// Row-major copy: a[r][c].
	let a: [[f64; 4]; 4] = std::array::from_fn(|r| std::array::from_fn(|c| m[c][r] as f64));
	let minor = |r: usize, c: usize| {
		let rows: Vec<usize> = (0..4).filter(|&i| i != r).collect();
		let cols: Vec<usize> = (0..4).filter(|&i| i != c).collect();
		let e = |i: usize, j: usize| a[rows[i]][cols[j]];
		e(0, 0) * (e(1, 1) * e(2, 2) - e(1, 2) * e(2, 1)) - e(0, 1) * (e(1, 0) * e(2, 2) - e(1, 2) * e(2, 0)) + e(0, 2) * (e(1, 0) * e(2, 1) - e(1, 1) * e(2, 0))
	};
	let cof: [[f64; 4]; 4] = std::array::from_fn(|r| std::array::from_fn(|c| if (r + c) % 2 == 0 { minor(r, c) } else { -minor(r, c) }));
	let det: f64 = (0..4).map(|c| a[0][c] * cof[0][c]).sum();
	if det.abs() < 1e-20 {
		return None;
	}
	// inverse[r][c] = cof[c][r] / det; column-major out[c][r].
	Some(std::array::from_fn(|c| std::array::from_fn(|r| (cof[c][r] / det) as f32)))
}

/// The level's view-projection `vp` mirrored about the plane z = `h`, with its near plane replaced by the water plane
/// (Lengyel's oblique clip, depth 0..1): everything below the water falls in front of the near plane and is clipped.
pub(super) fn mirrored(vp: &[[f32; 4]; 4], h: f32) -> [[f32; 4]; 4] {
	let m = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, -1.0, 0.0], [0.0, 0.0, 2.0 * h, 1.0]];
	let mut a = mul(vp, &m);
	let Some(inv) = inverse(&a) else { return a };
	// The plane (z - h >= 0 kept) in clip space: c_j = sum_k C_k inv[k][j] (row-major inv), column-major inv[j][k].
	let plane = [0.0, 0.0, 1.0, -h];
	let cc: [f32; 4] = std::array::from_fn(|j| (0..4).map(|k| plane[k] * inv[j][k]).sum());
	// The clip-space far corner opposite the plane.
	let q = [cc[0].signum(), cc[1].signum(), 1.0, 1.0];
	let d: f32 = (0..4).map(|k| cc[k] * q[k]).sum();
	if d.abs() < 1e-12 {
		return a;
	}
	for c in 0..4 {
		a[c][2] = plane[c] / d;
	}
	a
}

impl Gpu {
	/// The mirrored level into `target` (MSAA colour, its resolve, MSAA depth), mirrored about z = `h`; `u` is this frame's
	/// level uniforms, `ties` this frame's indirect tie runs (`bindless::push_args`), if any.
	pub(super) fn draw_mirror(&self, enc: &mut wgpu::CommandEncoder, scene: &Scene, u: &Uniforms, h: f32, target: (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView), ties: Option<&super::bindless::Runs>) {
		let mut mu = *u;
		mu.view_proj = mirrored(&u.view_proj, h);
		for r in 0..4 {
			mu.sky_view_proj[2][r] = -u.sky_view_proj[2][r];
		}
		mu.eye[2] = 2.0 * h - u.eye[2];
		self.draw_copy(enc, scene, &self.mirror, &mu, target, ties, true);
	}

	/// The water bed: the level as the camera sees it, without its water (sky, world, ties, tfrags, hero, mobies), into
	/// `target` (MSAA colour, its resolve, MSAA depth, kept for the water pass).
	pub(super) fn draw_bed(&self, enc: &mut wgpu::CommandEncoder, scene: &Scene, u: &Uniforms, target: (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView), ties: Option<&super::bindless::Runs>) {
		self.draw_copy(enc, scene, &self.bed, u, target, ties, false);
	}

	/// The level drawn again with the uniforms `mu` (through `with`'s buffer); `mirror`: the view is mirrored (winding flips).
	#[allow(clippy::too_many_arguments)]
	fn draw_copy(&self, enc: &mut wgpu::CommandEncoder, scene: &Scene, with: &Mirror, mu: &Uniforms, target: (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView), ties: Option<&super::bindless::Runs>, mirror: bool) {
		self.queue.write_buffer(&with.ubuf, 0, bytemuck::bytes_of(mu));
		let (colour, resolve, depth) = target;
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some(if mirror { "water mirror" } else { "water bed" }),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: colour, resolve_target: Some(resolve), depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(scene.clear), store: wgpu::StoreOp::Discard } })],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
				view: depth,
				depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: if mirror { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store } }),
				stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(super::shadowvol::STENCIL_CLEAR), store: wgpu::StoreOp::Discard }),
			}),
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_bind_group(0, &with.bind, &[]);
		// The mirror flips the winding: front-face culling becomes back-face culling and back.
		let flip = |c: u8| if mirror { 2 - c as usize } else { c as usize - 1 };
		if let Some(sky) = scene.sky.as_ref() {
			let split = scene.sky_shells.as_ref().map_or(sky.draws.len(), |s| s.split.min(sky.draws.len()));
			let shells = |pass: &mut wgpu::RenderPass, draws: &[Draw]| {
				pass.set_pipeline(&self.sky_pipeline);
				pass.set_vertex_buffer(0, sky.vbuf.slice(..));
				pass.set_index_buffer(sky.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				for d in draws {
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
				}
			};
			shells(&mut pass, &sky.draws[..split]);
			if let Some(sp) = scene.sky_sprites.as_ref().filter(|sp| sp.counts.iter().any(|&c| c > 0)) {
				pass.set_pipeline(&self.sky_add_pipeline);
				pass.set_vertex_buffer(0, sp.vbuf.slice(..));
				pass.set_index_buffer(sp.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				let mut first = 0;
				for (k, &n) in sp.counts.iter().enumerate() {
					if n > 0 {
						pass.set_bind_group(1, &scene.bind_groups[sp.bind[k]], &[]);
						pass.draw_indexed(first * 6..(first + n) * 6, 0, 0..1);
					}
					first += n;
				}
			}
			shells(&mut pass, &sky.draws[split..]);
		}
		let marker = scene.marker_shown.then_some(&scene.marker);
		let props = scene.props_shown.then_some(&scene.props);
		let skinned = self.props_gpu.as_ref().zip(scene.props_sj.as_ref());
		let array = self.bindless.as_ref().zip(scene.tex_array.as_ref());
		for (pipeline, batch) in [(&self.pipeline, Some(&scene.world)), (&self.pipeline, Some(&scene.ties)), (&self.tfrag_pipeline, Some(&scene.tfrags)), (&self.pipeline, marker), (&self.pipeline, props), (&self.tfrag_pipeline_lt, Some(&scene.tfrags))] {
			let Some(batch) = batch else { continue };
			if let Some((bl, tex_array)) = array.filter(|_| std::ptr::eq(batch, &scene.ties)) {
				pass.set_bind_group(1, tex_array, &[]);
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				// One indirect draw per cull run, as the level pass (thousands of ties otherwise cost milliseconds to encode).
				if let (Some(runs), Some((buf, _))) = (ties, bl.indirect.as_ref()) {
					for &(cull, off, n) in runs {
						pass.set_pipeline(if cull != 0 { &bl.pipeline_cull[flip(cull)] } else { &bl.pipeline });
						pass.multi_draw_indexed_indirect(buf, off, n);
					}
					continue;
				}
				for d in &batch.draws {
					pass.set_pipeline(if d.cull != 0 { &bl.pipeline_cull[flip(d.cull)] } else { &bl.pipeline });
					pass.draw_indexed(d.start..d.start + d.count, 0, d.texture as u32..d.texture as u32 + 1);
				}
				continue;
			}
			let (pipeline, cull) = match skinned.filter(|_| std::ptr::eq(batch, &scene.props)) {
				Some((pg, sj)) => {
					pass.set_bind_group(2, &pg.bind, &[]);
					pass.set_vertex_buffer(1, sj.slice(..));
					(&pg.pipeline, &pg.pipeline_cull)
				}
				None => (pipeline, &self.pipeline_cull),
			};
			pass.set_vertex_buffer(0, batch.vbuf.slice(..));
			pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
			for d in &batch.draws {
				pass.set_pipeline(if d.cull != 0 { &cull[flip(d.cull)] } else { pipeline });
				pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
				pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn apply(m: &[[f32; 4]; 4], p: [f32; 4]) -> [f32; 4] {
		std::array::from_fn(|r| (0..4).map(|c| m[c][r] * p[c]).sum())
	}

	/// A point above the plane lands where its mirror image does in the plain mirrored view (x, y, w), in front of the
	/// near plane; one below it is clipped (z < 0).
	#[test]
	fn mirror_clips_below_the_water() {
		let cam = Camera { pos: [0.0, -20.0, 10.0], yaw: std::f32::consts::FRAC_PI_2, pitch: -0.3, tan_half: 0.6, up: [0.0, 0.0, 1.0] };
		let vp = super::super::view_proj(&cam, 1.6, 0.5, 2000.0);
		let h = 2.0;
		let a = mirrored(&vp, h);
		let above = apply(&a, [1.0, 30.0, 6.0, 1.0]);
		let image = apply(&vp, [1.0, 30.0, 2.0 * h - 6.0, 1.0]);
		assert!((above[0] - image[0]).abs() < 1e-3 && (above[1] - image[1]).abs() < 1e-3 && (above[3] - image[3]).abs() < 1e-3);
		assert!(above[2] > 0.0 && above[2] < above[3]);
		let below = apply(&a, [1.0, 30.0, 1.0, 1.0]);
		assert!(below[2] < 0.0);
	}

	#[test]
	fn plane_of_finds_the_shared_height() {
		let pts = [[0.0, 0.0, 5.0], [10.0, 0.0, 5.0], [10.0, 8.0, 5.0], [0.0, 8.0, 5.1], [3.0, 3.0, 20.0]];
		let b = plane_of(pts.iter().copied()).unwrap();
		assert_eq!(b, [5.0, 0.0, 0.0, 10.0, 8.0]);
		assert!(plane_of([[0.0, 0.0, 0.0], [1.0, 0.0, 3.0], [2.0, 0.0, 6.0], [3.0, 0.0, 9.0], [4.0, 0.0, 12.0]].iter().copied()).is_none());
	}
}
