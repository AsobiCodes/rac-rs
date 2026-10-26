//! Sky spin and sprites, ocean surfaces (set_ocean, set_ocean_ram) and the sea shading mesh.

use super::*;

impl Gpu {
	/// ARI-49b (BRIDGE-63, decomp/notes/aridia_presentation.md): this frame's Aridia ocean patches (pure: camera, region
	/// cuboids, the 0x15f5e8 override `all`) and, when the game frame `frames` changed, the cross-faded 64x64 texture.
	/// `level_fog` = Visibomb live (0x15f458 == 1): the level fog words and colour stay instead of the ocean's.
	pub(in crate::viewer) fn set_ocean(&mut self, ocean: Option<&crate::ocean::Ocean>, eye: [f32; 3], cuboids: &[gameplay::Cuboid], all: bool, frames: u32, level_fog: bool) {
		let Some(ocean) = ocean else {
			self.ocean_draws.clear();
			self.ocean_frame = None;
			return;
		};
		self.ocean_draws = ocean.patches(eye, cuboids, all);
		self.ocean_level_fog = level_fog;
		self.ocean_fog = ocean.fog;
		if self.ocean_frame == Some(frames) && self.ocean_tex.is_some() {
			return;
		}
		self.ocean_frame = Some(frames);
		self.ocean_rgba = ocean.texture(frames);
		let size = wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 };
		if self.ocean_tex.is_none() {
			let tex = self.device.create_texture(&wgpu::TextureDescriptor {
				label: Some("ocean"),
				size,
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: wgpu::TextureFormat::Rgba8Unorm,
				usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
				view_formats: &[],
			});
			let view = tex.create_view(&Default::default());
			// [I] repeat + bilinear: the packet leaves CLAMP_1/TEX1_1 at the frame's values; ST spans -0.5..0.5 per patch.
			let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("ocean"),
				layout: &self.tex_layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
					wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
				],
			});
			self.ocean_tex = Some((tex, bind));
		}
		let Some((tex, _)) = &self.ocean_tex else { return };
		self.queue.write_texture(
			tex.as_image_copy(),
			bytemuck::cast_slice(&self.ocean_rgba),
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(64 * 4), rows_per_image: Some(64) },
			size,
		);
	}

	/// Retail mode: the retail oceans of Rilgar, Umbris and Gaspar (retail/oldsim.rs `RamOcean`): this field's
	/// patch strips (camera distance cull), the texture and the fog packed per vertex for `fs_ocean_g`.
	pub(in crate::viewer) fn set_ocean_ram(&mut self, oceans: &[crate::retail::oldsim::RamOcean], eye: [f32; 3]) {
		self.ocean_ram.truncate(oceans.len());
		for (k, o) in oceans.iter().enumerate() {
			let strips = crate::ocean::ram_strips(&o.desc, &o.st, &o.idx, o.fix, eye);
			let n = [o.fog[0], o.fog[1], 0.0];
			let l = [o.fog[2], o.fog[3], o.fogcol as f32, o.level_fog as u8 as f32];
			let (mut verts, mut idx) = (Vec::new(), Vec::new());
			for s in strips.iter().filter(|s| s.pos.len() >= 3) {
				let base = verts.len() as u32;
				verts.extend((0..s.pos.len()).map(|j| Vertex { pos: s.pos[j], uv: s.uv[j], normal: n, colour: s.colour[j], light: l }));
				idx.extend((0..s.pos.len() as u32 - 2).flat_map(|t| [base + t, base + t + 1, base + t + 2]));
			}
			let (mut pverts, mut pidx) = (Vec::new(), Vec::new());
			for p in o.patches.iter().filter(|p| p.pos.len() >= 3) {
				let base = pverts.len() as u32;
				pverts.extend((0..p.pos.len()).map(|j| Vertex { pos: p.pos[j], uv: p.uv[j], normal: n, colour: o.patch_colour, light: l }));
				pidx.extend((0..p.pos.len() as u32 - 2).flat_map(|t| [base + t, base + t + 1, base + t + 2]));
			}
			let size = wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 };
			if self.ocean_ram.len() <= k {
				let tex = self.device.create_texture(&wgpu::TextureDescriptor {
					label: Some("ocean ram"),
					size,
					mip_level_count: 1,
					sample_count: 1,
					dimension: wgpu::TextureDimension::D2,
					format: wgpu::TextureFormat::Rgba8Unorm,
					usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
					view_formats: &[],
				});
				let view = tex.create_view(&Default::default());
				let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
					label: Some("ocean ram"),
					layout: &self.tex_layout,
					entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) }],
				});
				self.ocean_ram.push(OceanBatch { tex, bind, verts: Vec::new(), idx: Vec::new(), pverts: Vec::new(), pidx: Vec::new(), fix: o.fix });
			}
			let b = &mut self.ocean_ram[k];
			(b.verts, b.idx, b.pverts, b.pidx, b.fix) = (verts, idx, pverts, pidx, o.fix);
			self.queue.write_texture(b.tex.as_image_copy(), bytemuck::cast_slice(&o.rgba), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(64 * 4), rows_per_image: Some(64) }, size);
		}
	}

	/// NOV-31: poses the sky shells for game frame `frames` on level `level` (the loaded level index): a shell with a
	/// `sky::shell_angle` turns about z by it, every other shell keeps its loaded positions. Rewrites the sky vertices
	/// only when the pose changes (once per game tick at most).
	pub(in crate::viewer) fn set_sky_spin(&mut self, level: usize, frames: u32) {
		let Some(scene) = &mut self.scene else { return };
		let (Some(batch), Some(s)) = (&scene.sky, &mut scene.sky_shells) else { return };
		if s.posed == Some((level, frames)) {
			return;
		}
		s.posed = Some((level, frames));
		let mut changed = false;
		for (k, (range, base)) in s.ranges.iter().zip(&s.base).enumerate() {
			let angle = sky::shell_angle(level, k, frames);
			let pos = match angle {
				Some(a) => sky::fan_positions(base, particles::euler_m([0.0, 0.0, a]), [0.0; 3]),
				None if s.turned[k] => base.clone(),
				None => continue,
			};
			s.turned[k] = angle.is_some();
			for (v, p) in s.verts[range.clone()].iter_mut().zip(pos) {
				v.pos = p;
			}
			changed = true;
		}
		if changed {
			self.queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(&s.verts));
		}
	}

	/// ARI-4: this frame's sky sprites from the held records (pure, no rand()): `view` rows are the camera right, up and forward
	/// axes, `tan_x` the port camera's horizontal half-angle tangent ([PC] widescreen x cull; retail 0.63 at 4:3).
	pub(in crate::viewer) fn set_sky_sprites(&mut self, sprites: &[sky::SkySprite], view: [[f32; 3]; 3], right: [f32; 3], up: [f32; 3], tan_x: f32) {
		let Some(scene) = &mut self.scene else { return };
		let Some(d) = &mut scene.sky_sprites else { return };
		let draws = sky::sprite_draws(sprites, view, right, up, tan_x, (1.0 + tan_x * tan_x).sqrt());
		let mut verts = Vec::with_capacity(draws.len() * 4);
		d.counts = [0; 2];
		// Grouped by texture (additive blending commutes).
		for (k, t) in [2u8, 3].into_iter().enumerate() {
			for (_, quad, colour) in draws.iter().filter(|q| q.0 == t) {
				verts.extend(quad.iter().map(|&(pos, uv)| Vertex { pos, uv, normal: [0.0; 3], colour: *colour, light: [0.0; 4] }));
				d.counts[k] += 1;
			}
		}
		if !verts.is_empty() {
			self.queue.write_buffer(&d.vbuf, 0, bytemuck::cast_slice(&verts));
		}
	}
}

/// [PC] Water Enhanced: the sea's shading mesh (the `WaterDraw` copy of the effect-quad sea; the visible quads keep retail z).
/// Retail draws Pokitaru's sea as long overlapping triangles of the base texture (22) at slightly different heights (a few
/// hundredths) plus two more layers (40 / 41); the position pass (depth test Less) writes whichever surface is nearest, so the
/// shaded surface creased along the triangles' intersections. The shading mesh keeps only the base texture's quads, each vertex
/// slid along its view ray onto the sea's plane height (the same screen position, one height).
pub(super) fn sea_shading_mesh(sea: &mut Vec<Vertex>, tex: &[usize], base: usize, eye: [f32; 3]) {
	if base != 22 {
		return;
	}
	let keep: Vec<Vertex> = sea.chunks(4).zip(tex).filter(|(_, t)| **t == base).flat_map(|(c, _)| c.iter().copied()).collect();
	if !keep.is_empty() {
		*sea = keep;
	}
	if let Some(pl) = super::mirror::plane_of(sea.iter().map(|v| v.pos)).filter(|pl| eye[2] > pl[0] + 0.05) {
		for v in sea.iter_mut() {
			let dz = v.pos[2] - eye[2];
			if dz < -0.01 {
				let k = (pl[0] - eye[2]) / dz;
				if k > 0.0 {
					for a in 0..2 {
						v.pos[a] = eye[a] + (v.pos[a] - eye[a]) * k;
					}
					v.pos[2] = pl[0];
				}
			}
		}
	}
}
