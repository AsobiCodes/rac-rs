//! Warp (travel) actors: load, pose and clear.

use super::*;

impl Gpu {
	/// BRIDGE-35: registers the warp bundle's ship classes. Their faces use moby texture keys 0..2, which the level's
	/// own textures also use, so they get their own texture map and fresh bind slots appended to the scene's list.
	pub(in crate::viewer) fn load_warp_actors(&mut self, assets: &crate::scene::WarpAssets) {
		let Some(mut scene) = self.scene.take() else { return };
		let textures: HashMap<u32, Texture> = assets
			.moby_textures
			.iter()
			.enumerate()
			.filter_map(|(k, t)| t.as_ref().map(|t| (crate::level::texture_key(crate::level::TextureGroup::Moby, k as u32), Texture { w: t.w, h: t.h, rgba: t.rgba.clone() })))
			.collect();
		let mut model = HashMap::new();
		let (mut max_v, mut max_i) = (0, 0);
		for (id, class, _) in &assets.classes {
			let mut slots = HashMap::new();
			let (_, _, _, verts) = self.batch(&[&class.mesh], None, false, &textures, &mut scene.bind_groups, &mut slots);
			let mut by_tex: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
			let mut phases: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
			let phased = class.mesh.strip_phase.len() == class.mesh.faces.len();
			for (fi, &(tex, f)) in class.mesh.faces.iter().enumerate() {
				let slot = slots[&(tex & crate::mobies::SLOT_MASK)];
				by_tex.entry(slot).or_default().extend(f);
				phases.entry(slot).or_default().push(if phased { class.mesh.strip_phase[fi] } else { crate::mesh::NO_PHASE });
			}
			max_v = max_v.max(verts.len());
			max_i = max_i.max(class.mesh.faces.len() * 3);
			model.insert(*id, (verts, by_tex.into_iter().collect::<Vec<_>>(), phases.into_iter().collect::<Vec<_>>()));
		}
		// One actor per stream and class, a few spare for the segment change.
		let n = 4;
		scene.warp_batch = Some(Batch {
			vbuf: buffer(&self.device, "warp vertices", &vec![0u8; max_v * n * std::mem::size_of::<Vertex>()], wgpu::BufferUsages::VERTEX),
			ibuf: buffer(&self.device, "warp indices", &vec![0u8; max_i * n * 4], wgpu::BufferUsages::INDEX),
			draws: Vec::new(),
		});
		scene.warp_model = model;
		self.scene = Some(scene);
	}

	/// BRIDGE-35: this frame's posed warp actors (class, skinned positions and normals, actor position); `hidden`
	/// draws nothing (the quiet warp hides the ship). Turns the level draw off until `clear_warp_actors`.
	/// `lights[i]`: actor i's (ambient bytes, light word) from retail RAM (retail mirror); missing = the spawn ambient.
	pub(in crate::viewer) fn set_warp_actors(&mut self, actors: &[(u32, Posed, [f32; 3])], hidden: bool, lights: &[Option<([u8; 3], u32)>]) {
		let Some(mut scene) = self.scene.take() else { return };
		scene.warp_shown = true;
		if let Some(batch) = &mut scene.warp_batch {
			let (mut verts, mut by_tex): (Vec<Vertex>, BTreeMap<usize, Vec<u32>>) = (Vec::new(), BTreeMap::new());
			for (ai, (class, (ps, ns), pos)) in actors.iter().enumerate().filter(|_| !hidden) {
				let ram = lights.get(ai).copied().flatten();
				let light = Self::moby_light(ram);
				let Some((model, faces, _)) = scene.warp_model.get(class) else { continue };
				// Retail mirror: the ship is a lit moby (REN-5c: ambient + light pair, vertex colour bytes 0x80), so the
				// vertices take the moby code (id 0 = no dynamic light, full alpha), not the mesh's baked -2 colours.
				let lit_moby = ram.is_some();
				let base = verts.len() as u32;
				if std::env::var_os("RAC_SCENE_LOG").is_some() {
					eprintln!("retail: warp actor {ai} class {class:#x} model verts {} skinned {} normals {}", model.len(), ps.len(), ns.len());
				}
				for (i, v) in model.iter().enumerate() {
					let mut v = *v;
					if let (Some(p), Some(n)) = (ps.get(i), ns.get(i)) {
						v.pos = [p[0] + pos[0], p[1] + pos[1], p[2] + pos[2]];
						v.normal = *n;
					}
					v.light = light;
					if lit_moby {
						v.colour = [1.0, 1.0, 1.0, moby_alpha_encoding(0.0, 0x80).0];
					}
					verts.push(v);
				}
				for (t, idx) in faces {
					by_tex.entry(*t).or_default().extend(idx.iter().map(|i| i + base));
				}
			}
			let (mut indices, mut draws) = (Vec::new(), Vec::new());
			for (texture, idx) in by_tex {
				draws.push(Draw { inst: 0, cull: 0, texture, start: indices.len() as u32, count: idx.len() as u32 });
				indices.extend(idx);
			}
			if !verts.is_empty() {
				self.queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(&verts));
				self.queue.write_buffer(&batch.ibuf, 0, bytemuck::cast_slice(&indices));
			}
			batch.draws = draws;
		}
		self.scene = Some(scene);
	}

	/// BRIDGE-35: the warp is over; the level draws again.
	pub(in crate::viewer) fn clear_warp_actors(&mut self) {
		if let Some(scene) = &mut self.scene {
			scene.warp_shown = false;
			scene.warp_batch = None;
			scene.warp_model.clear();
		}
	}
}
