//! The posed hero marker model (set_marker) and its clipped polygons.

use super::*;

impl Gpu {
	/// Vertex count of the marker model (Ratchet with the items appended): a posed hero must cover all of it (`set_marker`).
	pub(in crate::viewer) fn marker_len(&self) -> Option<usize> {
		self.scene.as_ref().map(|s| s.marker_model.len())
	}

	/// `blend` = (the pose before the newest tick, factor): the GPU hero's blend source (see `gpuhero.rs`); the CPU path blends it here.
	pub(in crate::viewer) fn set_marker(&mut self, at: Option<([f32; 3], f32)>, skinned: Option<&Posed>, blend: Option<(&Posed, f32)>) {
		let _s = super::profile::span("set_marker");
		// The first posing call of a frame: the posed mobies' third lights start over.
		self.moby_dyn.clear();
		// The afterimage ranges are for this call only, whether or not the marker shows (BRIDGE-28).
		let ghosts = std::mem::take(&mut self.marker_ghosts);
		let bomb = self.marker_bomb.take();
		let tilt = std::mem::take(&mut self.marker_tilt);
		let clank_light = self.marker_light.take();
		let Some(scene) = &mut self.scene else { return };
		// NOV-5.9d: the clip queue is rebuilt every frame, marker polygons first.
		scene.clip_queue.clear();
		let Some((feet, yaw)) = at else {
			scene.marker_shown = false;
			scene.marker_alpha = false;
			return;
		};
		// The skinned pose replaces the model's positions and normals (PC perf: per vertex, into a reused buffer).
		let skin = skinned.filter(|(ps, _)| ps.len() == scene.marker_model.len());
		let (c, sn) = (yaw.cos(), yaw.sin());
		// BRIDGE-75: a tilted hero uses the full euler basis (rows of euler_m); untilted keeps the yaw-only turn.
		let rows = (tilt != [0.0; 2]).then(|| crate::particles::euler_m([tilt[0], tilt[1], yaw]));
		let turn = |v: [f32; 3]| match &rows {
			Some(r) => [
				v[0] * r[0][0] + v[1] * r[1][0] + v[2] * r[2][0],
				v[0] * r[0][1] + v[1] * r[1][1] + v[2] * r[2][1],
				v[0] * r[0][2] + v[1] * r[1][2] + v[2] * r[2][2],
			],
			None => [v[0] * c - v[1] * sn, v[0] * sn + v[1] * c, v[2]],
		};
		let _l = super::profile::span("sm_light");
		let (light, tint) = Self::hero_light_tint(&self.lighting, feet, self.hero_placed, &mut self.hero_env);
		self.hero_tint_out = Some(tint);
		let id = dyn_id(&self.table, &mut self.moby_dyn, feet);
		drop(_l);
		let shift = self.marker_v.take();
		// [PC] perf (lane R2, gpuhero.rs): a compute pass builds the vertices from the model and the two step poses (RAC_HERO_GPU=1).
		// Falls back to the CPU build below when the hero is not clearly inside / outside the clip guard band (the clipper needs the
		// world vertices then); RAC_HERO_GPU_CHECK=1 runs both and compares the bytes.
		let n_model = scene.marker_model.len();
		let blend = blend.filter(|(b, _)| skin.is_some() && b.0.len() == n_model && b.1.len() == n_model);
		let (gen_now, gen_before) = std::mem::take(&mut self.hero_gens);
		let mut check_bytes: Option<Vec<u8>> = None;
		let mut gpu_taken = false;
		// Self-check kind (gpuhero.rs): set when this frame is compared with the CPU build although the GPU build is the one in use.
		let mut self_kind: Option<usize> = None;
		let mut hero_check = super::gpuhero::gpu_hero_check();
		if let (Some(cur), true, true) = (skin, scene.marker_gpu.is_some(), super::gpuhero::gpu_hero_on()) {
			let hg = scene.marker_gpu.as_mut().expect("checked");
			let _p = super::profile::span("sm_poses");
			let can_blend = hg.set_poses(&self.queue, blend.map(|b| (b.0, gen_before)), (cur, gen_now));
			drop(_p);
			let t = if can_blend { blend.map_or(1.0, |b| b.1) } else { 1.0 };
			let r = rows.unwrap_or([[0.0; 3]; 3]);
			let row4 = |i: usize| [r[i][0], r[i][1], r[i][2], 0.0];
			let mut gh = [[0.0f32; 4]; 5];
			let mut gr = [[0u32; 4]; 5];
			for (k, g) in ghosts.iter().enumerate() {
				if let Some(&(a, b, al)) = g.as_ref() {
					gh[k][0] = moby_alpha_encoding(id, al).0;
					gr[k] = [a as u32, b as u32, 1, 0];
				}
			}
			let tint3 = scene.marker_tint.map(|(t, s)| (t, s as u32));
			let hp = super::gpuhero::HeroParams {
				feet: [feet[0], feet[1], feet[2], 1.0],
				cs: [c, sn, t, id],
				r0: { let mut r = row4(0); r[3] = hg.parity(); r },
				r1: row4(1),
				r2: row4(2),
				light,
				tint: tint3.map_or([0.0; 4], |(t, _)| [t[0], t[1], t[2], 0.0]),
				clank: clank_light.map_or([0.0; 4], |(_, _, t)| [t[0], t[1], t[2], 0.0]),
				bomb: [bomb.map_or(0.0, |b| b.2[0]), bomb.map_or(0.0, |b| b.2[1]), bomb.map_or(0.0, |b| b.2[2]), shift.map_or(0.0, |s| s.2)],
				gh,
				gr,
				u0: [n_model as u32, can_blend as u32, rows.is_some() as u32, tint3.is_some() as u32],
				u1: [tint3.map_or(0, |t| t.1), scene.marker_hero_len as u32, clank_light.map_or(0, |l| l.0 as u32), clank_light.map_or(0, |l| l.1 as u32)],
				u2: [clank_light.is_some() as u32, bomb.is_some() as u32, shift.is_some() as u32, 0],
				u3: [bomb.map_or(0, |b| b.0 as u32), bomb.map_or(0, |b| b.1 as u32), shift.map_or(0, |s| s.0 as u32), shift.map_or(0, |s| s.1 as u32)],
			};
			let skip = scene.clip_view.as_ref().is_none_or(|v| super::gpuskin::sphere_skips(v, feet, hg.radius));
			// The first frames of each kind run the GPU build and the CPU build and compare them (the CPU result is drawn).
			let kind = super::gpuhero::kind(can_blend && t != 1.0, rows.is_some(), ghosts.iter().any(Option::is_some) || bomb.is_some() || shift.is_some() || clank_light.is_some());
			if skip && !hero_check && super::gpuhero::self_check_wanted(kind) {
				self_kind = Some(kind);
				hero_check = true;
			}
			if skip || hero_check {
				hg.queue_build(&self.queue, hp);
			}
			gpu_taken = skip && !hero_check;
			if hero_check {
				check_bytes = Some(hg.build_and_read(&self.device, &self.queue, &scene.marker.vbuf));
			}
		}
		if gpu_taken {
			// The clipper's verdict is "no face to take" for any pose in the sphere: unmask what an earlier frame masked.
			clip_marker(&self.queue, scene, &[]);
			scene.marker_verts.clear();
			scene.marker_key = None;
			scene.marker_shown = true;
			scene.marker_alpha = ghosts.iter().any(Option::is_some);
			return;
		}
		if let Some(hg) = scene.marker_gpu.as_mut() {
			hg.cancel();
		}
		// The CPU build: the blend the GPU would have done.
		let blended_cpu = blend.and_then(|(b, t)| skin.and_then(|s| super::smooth::blend_posed(b, s, t)));
		let skin = blended_cpu.as_ref().or(skin);
		// [PC] perf (lane A, B2): the same inputs and skinned pose as the call that filled `marker_verts` and the vertex buffer give the
		// same vertices: no rebuild and no upload (the clip pass below still runs against this frame's view). RAC_MARKER_CHECK=1
		// rebuilds anyway and compares the bytes with the kept ones.
		let b3 = super::gpucache::bits3;
		let key = super::gpucache::MarkerKey {
			skin: skin.map(|(ps, _)| ps.len()),
			feet: b3(feet),
			yaw: yaw.to_bits(),
			tilt: tilt.map(f32::to_bits),
			light: light.map(f32::to_bits),
			id: id.to_bits(),
			tint: scene.marker_tint.map(|(t, s)| (b3(t), s)),
			hero_len: scene.marker_hero_len,
			clank: clank_light.map(|(a, b, t)| (a, b, b3(t))),
			ghosts,
			bomb: bomb.map(|(a, b, t)| (a, b, b3(t))),
			shift: shift.map(|(a, b, d)| (a, b, d.to_bits())),
		};
		let skin_same = match skin {
			Some((ps, ns)) => {
				let eq = |a: &[[f32; 3]], b: &[[f32; 3]]| a.len() == b.len() && bytemuck::cast_slice::<_, u8>(a) == bytemuck::cast_slice::<_, u8>(b);
				eq(&scene.marker_skin.0, ps) && eq(&scene.marker_skin.1, ns)
			}
			None => true,
		};
		let same_inputs = skin_same && scene.marker_key.as_ref() == Some(&key) && !scene.marker_verts.is_empty();
		let check = super::gpucache::marker_check_on();
		if same_inputs && !check && check_bytes.is_none() {
			let verts = std::mem::take(&mut scene.marker_verts);
			clip_marker(&self.queue, scene, &verts);
			scene.marker_verts = verts;
			scene.marker_shown = true;
			scene.marker_alpha = ghosts.iter().any(Option::is_some);
			return;
		}
		if !skin_same {
			if let Some((ps, ns)) = skin {
				scene.marker_skin.0.clear();
				scene.marker_skin.0.extend_from_slice(ps);
				scene.marker_skin.1.clear();
				scene.marker_skin.1.extend_from_slice(ns);
			}
		}
		let kept = (same_inputs && check).then(|| scene.marker_verts.clone());
		let why = if check { scene.marker_key.as_ref().map_or(0x1fff, |k| k.diff(&key)) | (!skin_same as u32) << 12 } else { 0 };
		scene.marker_key = Some(key);
		let mut verts = std::mem::take(&mut scene.marker_verts);
		verts.clear();
		verts.extend(scene.marker_model
			.iter()
			.enumerate()
			.take(skin.map_or(usize::MAX, |(_, ns)| ns.len()))
			.map(|(k, v)| {
				let v = &match skin {
					Some((ps, ns)) => Vertex { pos: ps[k], normal: ns[k], ..*v },
					None => *v,
				};
				let p = turn(v.pos);
				let colour = match scene.marker_tint.filter(|&(_, s)| k >= s && k < scene.marker_hero_len) {
					Some((t, _)) => [t[0], t[1], t[2], -2.0],
					None => match clank_light.filter(|l| (l.0..l.1).contains(&k)) {
						Some((_, _, t)) => [t[0], t[1], t[2], -2.0],
						None => match ghosts.iter().flatten().find(|g| (g.0..g.1).contains(&k)) {
							Some(&(_, _, a)) => [v.colour[0], v.colour[1], v.colour[2], moby_alpha_encoding(id, a).0],
							None => [v.colour[0], v.colour[1], v.colour[2], id],
						},
					},
				};
				// WPN-3.2: the held bomb's colour scale rides the same per-range vertex colour path.
				let colour = match bomb.filter(|b| (b.0..b.1).contains(&k)) {
					Some((_, _, t)) => [colour[0] * t[0], colour[1] * t[1], colour[2] * t[2], colour[3]],
					None => colour,
				};
				let uv = match shift {
					Some((a, b, dv)) if (a..b).contains(&k) => [v.uv[0], v.uv[1] + dv],
					_ => v.uv,
				};
				Vertex { pos: [p[0] + feet[0], p[1] + feet[1], p[2] + feet[2]], uv, normal: turn(v.normal), colour, light, ..*v }
			}));
		self.queue.write_buffer(&scene.marker.vbuf, 0, bytemuck::cast_slice(&verts));
		if let Some(gb) = &check_bytes {
			match self_kind {
				Some(k) => super::gpuhero::self_check_result(k, gb, bytemuck::cast_slice(&verts)),
				None => super::gpuhero::note_check(gb, bytemuck::cast_slice(&verts)),
			}
		}
		if check {
			super::gpucache::marker_note(same_inputs, kept.as_ref().is_none_or(|k| bytemuck::cast_slice::<_, u8>(&k[..]) == bytemuck::cast_slice::<_, u8>(&verts[..])), why);
		}
		clip_marker(&self.queue, scene, &verts);
		scene.marker_verts = verts;
		scene.marker_shown = true;
		scene.marker_alpha = ghosts.iter().any(Option::is_some);
	}
}

/// NOV-5.9d path 2: the hero marker (body and held gadgets, world-space `verts`) takes the same per-face clip; masked faces
/// are zeroed in `marker.ibuf` with the compare-and-write rule of `clip_props`.
fn clip_marker(queue: &wgpu::Queue, scene: &mut Scene, verts: &[Vertex]) {
	let mut taken: Vec<u32> = Vec::new();
	if let Some(view) = scene.clip_view {
		let (band, port) = (view.band(), view.viewport());
		let (mut inside, mut all_out) = (true, 0x3fu8);
		for v in verts {
			let p = view.view(v.pos);
			inside &= band.flags(p) == 0;
			all_out &= port.flags(p);
		}
		if !inside && all_out == 0 {
			for (fi, f) in scene.marker_faces.iter().enumerate() {
				if let Some(poly) = super::clip::clip_face(&view, f.idx.map(|i| verts[i as usize]), f.phase, f.tex, f.cull) {
					taken.push(fi as u32);
					scene.clip_queue.push(poly);
				}
			}
		}
	}
	if taken != scene.marker_masked {
		let (old, new): (HashSet<u32>, HashSet<u32>) = (scene.marker_masked.iter().copied().collect(), taken.iter().copied().collect());
		for &k in old.difference(&new) {
			let f = &scene.marker_faces[k as usize];
			queue.write_buffer(&scene.marker.ibuf, f.slot as u64 * 4, bytemuck::cast_slice(&f.idx));
		}
		for &k in new.difference(&old) {
			queue.write_buffer(&scene.marker.ibuf, scene.marker_faces[k as usize].slot as u64 * 4, bytemuck::cast_slice(&[0u32; 3]));
		}
		scene.marker_masked = taken;
	}
}
