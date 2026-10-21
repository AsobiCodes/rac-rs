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

/// The flat plane of a water mesh: the height most of its vertices share and the XY box of the vertices on it, as
/// [height, x0, y0, x1, y1]; none when no height holds a quarter of the vertices (a river running downhill, waves). The
/// height is the 0.25 bin most vertices fall in, then moved to the mean of the vertices around it (`settle`), so a sea that
/// bobs (Pokitaru's) gives a height that follows it smoothly: snapped to the bin, the mirror jumped a quarter unit every
/// second or so and the whole reflection with it.
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
	let h = settle(pos.clone(), k as f32 / 4.0);
	let mut b = [h, f32::MAX, f32::MAX, f32::MIN, f32::MIN];
	for p in pos.filter(|p| (p[2] - h).abs() <= 0.5) {
		b[1] = b[1].min(p[0]);
		b[2] = b[2].min(p[1]);
		b[3] = b[3].max(p[0]);
		b[4] = b[4].max(p[1]);
	}
	Some(b)
}

/// From `h`, the mean height of the vertices near it (weights falling to 0 at 1 unit away), three times over: it settles on the
/// heights' local centre whatever bin it starts from, so it moves smoothly when they do.
fn settle(pos: impl Iterator<Item = [f32; 3]> + Clone, mut h: f32) -> f32 {
	for _ in 0..3 {
		let (mut sw, mut sz) = (0.0f64, 0.0f64);
		for p in pos.clone() {
			let w = (1.0 - (p[2] - h).abs()).max(0.0) as f64;
			sw += w;
			sz += w * p[2] as f64;
		}
		if sw <= 0.0 {
			break;
		}
		h = (sz / sw) as f32;
	}
	h
}

/// Planes mirrored at most per frame (Water High; `Gpu::draw_mirror` once each, into its own layer of the water pass's mirror target).
pub(super) const MAX_MIRRORS: usize = 3;

/// The flat water planes below the eye as (rays on it, distance, height): of `grid` x `grid` rays through the screen (NDC -`span`
/// .. `span`, `ivp` = the inverse view-projection), those meeting the plane inside its box; the distance is the horizontal one to
/// its box plus a quarter of the height above it.
fn view_planes(draws: &[super::waterfx::WaterDraw], eye: [f32; 3], ivp: Option<&[[f32; 4]; 4]>, grid: usize, span: f32) -> Vec<(u32, f32, f32)> {
	// Column-major: ray directions through the grid, from the eye to the unprojected point (NDC depth 0.5 lies in front).
	let rays: Vec<[f32; 3]> = ivp
		.map(|m| {
			(0..grid * grid)
				.filter_map(|i| {
					let step = |k: usize| -span + 2.0 * span * k as f32 / (grid - 1) as f32;
					let c = [step(i % grid), step(i / grid), 0.5, 1.0];
					let p: [f32; 4] = std::array::from_fn(|r| (0..4).map(|k| m[k][r] * c[k]).sum());
					(p[3].abs() > 1e-6).then(|| [p[0] / p[3] - eye[0], p[1] / p[3] - eye[1], p[2] / p[3] - eye[2]])
				})
				.collect()
		})
		.unwrap_or_default();
	draws
		.iter()
		.filter_map(|d| d.plane)
		.filter(|b| eye[2] > b[0] + 0.3)
		.map(|b| {
			let on = rays
				.iter()
				.filter(|d| {
					let t = (b[0] - eye[2]) / d[2];
					let (x, y) = (eye[0] + d[0] * t, eye[1] + d[1] * t);
					d[2] < 0.0 && x >= b[1] - 1.0 && x <= b[3] + 1.0 && y >= b[2] - 1.0 && y <= b[4] + 1.0
				})
				.count() as u32;
			let dx = (b[1] - eye[0]).max(eye[0] - b[3]).max(0.0);
			let dy = (b[2] - eye[1]).max(eye[1] - b[4]).max(0.0);
			(on, (dx * dx + dy * dy).sqrt() + 0.25 * (eye[2] - b[0]), b[0])
		})
		.collect()
}

/// The plane to mirror this frame (Water Medium): of the flat water planes below the eye, the one most of the view looks at (of
/// `VIEW_GRID` x `VIEW_GRID` rays through the screen, `ivp` = the inverse view-projection, the most meeting the plane inside its
/// box), then the nearest (horizontal distance to its box, plus a quarter of the height above it). Last frame's plane (`prev`) stays
/// unless another wins by `VIEW_KEEP` rays, so it does not flip while two planes take about as much of the view; with no ray
/// on any water (looking up) it stays too. (Before 2026-10-20 the nearest always won: on Rilgar a 16 x 16 pool by the ship pad
/// [C: RAC_WATER_LOG, fogged quads at 61.2, box 328..344 x 102..118], 1.6 units above the sea, took the mirror whenever the
/// camera came near it, and the sea in view fell back to the screen-space reflection: no mirror image, no ripples bending it,
/// flipping as the camera moved; user 2026-10-20.)
pub(super) fn pick(draws: &[super::waterfx::WaterDraw], eye: [f32; 3], ivp: Option<&[[f32; 4]; 4]>, prev: Option<f32>) -> Option<f32> {
	const VIEW_GRID: usize = 5;
	const VIEW_KEEP: u32 = 3;
	let planes = view_planes(draws, eye, ivp, VIEW_GRID, 0.8);
	let best = planes.iter().max_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)))?;
	let kept = prev.and_then(|h| planes.iter().filter(|p| (p.2 - h).abs() < 0.5).min_by(|a, b| (a.2 - h).abs().total_cmp(&(b.2 - h).abs())));
	Some(match kept {
		Some(k) if k.0 + VIEW_KEEP > best.0 => k.2,
		_ => best.2,
	})
}

/// The planes to mirror this frame (Water High): up to `n` heights, the planes most of the view looks at (a finer ray grid than
/// `pick`'s, so a small pool at the edge of the view still counts), at least 0.8 apart (one mirror serves every height within 0.4
/// of its plane). Last frame's planes (`prev`) stay while they hold a ray; a free slot takes the best plane with a ray; with all
/// slots taken a plane replaces the weakest only when it has `keep` rays more, so the set does not flip while planes take about
/// as much of the view. With no ray on any water (looking up) the nearest plane alone, as `pick`.
pub(super) fn pick_many(draws: &[super::waterfx::WaterDraw], eye: [f32; 3], ivp: Option<&[[f32; 4]; 4]>, prev: &[f32], n: usize) -> Vec<f32> {
	const GRID: usize = 9;
	let keep = (3 * (GRID * GRID) as u32 / 25).max(1);
	let planes = view_planes(draws, eye, ivp, GRID, 0.95);
	let mut order: Vec<&(u32, f32, f32)> = planes.iter().collect();
	order.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.total_cmp(&b.1)));
	let apart = |c: &[(u32, f32, f32)], h: f32| c.iter().all(|p| (p.2 - h).abs() >= 0.8);
	let mut chosen: Vec<(u32, f32, f32)> = Vec::new();
	for &ph in prev {
		let k = planes.iter().filter(|p| p.0 > 0 && (p.2 - ph).abs() < 0.5 && apart(&chosen, p.2)).min_by(|a, b| (a.2 - ph).abs().total_cmp(&(b.2 - ph).abs()));
		if let (Some(&k), true) = (k, chosen.len() < n) {
			chosen.push(k);
		}
	}
	for _ in 0..2 * MAX_MIRRORS {
		let Some(&&c) = order.iter().find(|p| p.0 > 0 && apart(&chosen, p.2)) else { break };
		if chosen.len() < n {
			chosen.push(c);
		} else if let Some((w, weak)) = chosen.iter().enumerate().min_by_key(|(_, p)| p.0) {
			if c.0 < weak.0 + keep {
				break;
			}
			chosen[w] = c;
		}
	}
	if chosen.is_empty() {
		chosen.extend(order.first().map(|p| **p));
	}
	chosen.into_iter().map(|p| p.2).take(n).collect()
}

/// RAC_WATER_LOG: (rays of a 9 x 9 grid, height) of every flat plane below the eye.
pub(super) fn view_stats(draws: &[super::waterfx::WaterDraw], eye: [f32; 3], ivp: Option<&[[f32; 4]; 4]>) -> Vec<(u32, f32)> {
	view_planes(draws, eye, ivp, 9, 0.95).into_iter().map(|p| (p.0, p.2)).collect()
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
pub(super) fn inverse(m: &[[f32; 4]; 4]) -> Option<[[f32; 4]; 4]> {
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
	/// level uniforms, `ties` this frame's indirect tie runs (`bindless::push_args`), if any. `k`: the mirror pass of this frame
	/// (0..`MAX_MIRRORS`); each has its own uniform buffer, as all passes of a frame go in one submit.
	#[allow(clippy::too_many_arguments)]
	pub(super) fn draw_mirror(&self, enc: &mut wgpu::CommandEncoder, scene: &Scene, u: &Uniforms, h: f32, k: usize, target: (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView), ties: Option<&super::bindless::Runs>) {
		let mut mu = *u;
		mu.view_proj = mirrored(&u.view_proj, h);
		for r in 0..4 {
			mu.sky_view_proj[2][r] = -u.sky_view_proj[2][r];
		}
		mu.eye[2] = 2.0 * h - u.eye[2];
		self.draw_copy(enc, scene, &self.mirror[k], &mu, target, ties, true, Part::All);
	}

	/// The water bed: the level as the camera sees it, without its water (sky, world, ties, tfrags, hero, mobies), into
	/// `target` (MSAA colour, its resolve, MSAA depth, kept for the water pass); then the geometric sea waves (waterfx.rs
	/// `sea_waves`: the shore map from the bed's depth, the sea's grid). [PC] The moving mobys (waterfx.rs `track_props`) draw
	/// after the shore map, so it holds the still ground only (a fish bobbing at the surface made surf that came and went
	/// every frame); the bed itself is the same.
	pub(super) fn draw_bed(&self, enc: &mut wgpu::CommandEncoder, scene: &Scene, u: &Uniforms, target: (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView), ties: Option<&super::bindless::Runs>) {
		let movers = self.waterfx.shore_movers();
		let split = (scene.props_shown && !movers.is_empty()).then(|| super::shadowvol::split_late(&scene.props.draws, &scene.slot_runs, movers)).filter(|(_, moving)| !moving.is_empty());
		match &split {
			Some((still, moving)) => {
				self.draw_copy(enc, scene, &self.bed, u, target, ties, false, Part::Still(still));
				self.waterfx.sea_waves(enc);
				self.draw_copy(enc, scene, &self.bed, u, target, ties, false, Part::Moving(moving));
			}
			None => {
				self.draw_copy(enc, scene, &self.bed, u, target, ties, false, Part::All);
				self.waterfx.sea_waves(enc);
			}
		}
	}

	/// The level drawn again with the uniforms `mu` (through `with`'s buffer); `mirror`: the view is mirrored (winding flips);
	/// `part`: all of it, or the bed in two passes (`draw_bed`).
	#[allow(clippy::too_many_arguments)]
	fn draw_copy(&self, enc: &mut wgpu::CommandEncoder, scene: &Scene, with: &Mirror, mu: &Uniforms, target: (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView), ties: Option<&super::bindless::Runs>, mirror: bool, part: Part) {
		self.queue.write_buffer(&with.ubuf, 0, bytemuck::bytes_of(mu));
		let (colour, resolve, depth) = target;
		// The second pass of a split bed goes on with the first's colour, depth and stencil.
		let (first, last) = (!matches!(part, Part::Moving(_)), !matches!(part, Part::Still(_)));
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some(if mirror { "water mirror" } else { "water bed" }),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: colour, resolve_target: last.then_some(resolve), depth_slice: None, ops: wgpu::Operations { load: if first { wgpu::LoadOp::Clear(scene.clear) } else { wgpu::LoadOp::Load }, store: if last { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store } } })],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
				view: depth,
				depth_ops: Some(wgpu::Operations { load: if first { wgpu::LoadOp::Clear(1.0) } else { wgpu::LoadOp::Load }, store: if mirror { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store } }),
				stencil_ops: Some(wgpu::Operations { load: if first { wgpu::LoadOp::Clear(super::shadowvol::STENCIL_CLEAR) } else { wgpu::LoadOp::Load }, store: if last { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store } }),
			}),
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_bind_group(0, &with.bind, &[]);
		// The mirror flips the winding: front-face culling becomes back-face culling and back.
		let flip = |c: u8| if mirror { 2 - c as usize } else { c as usize - 1 };
		if let Some(sky) = scene.sky.as_ref().filter(|_| first) {
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
		let skinned = scene.props_sj.as_ref().map(|sj| (&self.props_gpu, sj));
		let array = self.bindless.as_ref().zip(scene.tex_array.as_ref());
		// (Which pass draws it: 1 the first, 2 the second, 3 both: the props split into the still and the moving.)
		for (pipeline, batch, when) in [(&self.pipeline, Some(&scene.world), 1), (&self.pipeline, Some(&scene.ties), 1), (&self.tfrag_pipeline, Some(&scene.tfrags), 1), (&self.pipeline, marker, 1), (&self.pipeline, props, 3), (&self.tfrag_pipeline_lt, Some(&scene.tfrags), 2)] {
			let Some(batch) = batch else { continue };
			if !((first || when & 2 != 0) && (last || when & 1 != 0)) {
				continue;
			}
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
			let draws: &[Draw] = match part {
				Part::Still(d) | Part::Moving(d) if std::ptr::eq(batch, &scene.props) => d,
				_ => &batch.draws,
			};
			pass.set_vertex_buffer(0, batch.vbuf.slice(..));
			pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
			for d in draws {
				pass.set_pipeline(if d.cull != 0 { &cull[flip(d.cull)] } else { pipeline });
				pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
				pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
			}
		}
	}
}

/// Which of the level `Gpu::draw_copy` draws: all of it, or the bed's first pass (all but the moving props' `Draw`s and the
/// translucent tfrags: the still props' draws given) or its second (the moving props' draws given, the translucent tfrags).
#[derive(Clone, Copy)]
enum Part<'a> {
	All,
	Still(&'a [Draw]),
	Moving(&'a [Draw]),
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
		assert!((b[0] - 5.02).abs() < 0.01 && b[1..] == [0.0, 0.0, 10.0, 8.0], "{b:?}");
		assert!(plane_of([[0.0, 0.0, 0.0], [1.0, 0.0, 3.0], [2.0, 0.0, 6.0], [3.0, 0.0, 9.0], [4.0, 0.0, 12.0]].iter().copied()).is_none());
	}

	/// A bobbing sea (waves of 0.3 around a level that rises 0.01 a step) gives a height that rises with it, never a jump.
	#[test]
	fn plane_of_follows_a_bobbing_sea() {
		let mut last: Option<f32> = None;
		for s in 0..60 {
			let lift = 223.0 + 0.01 * s as f32;
			let pts: Vec<[f32; 3]> = (0..400).map(|i| [i as f32, (i / 20) as f32, lift + 0.3 * (i as f32 * 0.7 + s as f32 * 0.2).sin()]).collect();
			let h = plane_of(pts.iter().copied()).unwrap()[0];
			assert!((h - lift).abs() < 0.05, "{s}: {h} vs {lift}");
			if let Some(l) = last {
				assert!((h - l).abs() < 0.03, "{s}: jump {l} -> {h}");
			}
			last = Some(h);
		}
	}
}
