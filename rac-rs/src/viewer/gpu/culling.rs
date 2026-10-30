//! Per-frame culling of ties, shrubs and tfrags, the far draw level, and the ice tag.

use super::*;

impl Gpu {
	/// [PC] Draw distance (fardraw.rs): the Graphics row's level (with either Lighting) over a retail level drawn by rac-rs;
	/// None = Low, retail's distances.
	pub(in crate::viewer) fn far_draw(&self) -> Option<super::fardraw::Level> {
		super::fardraw::on(crate::retail::pcext::graphics().draw_q(), self.modern_ok)
	}

	/// [PC] The world far plane (m) of the frame's draw distance (fardraw.rs `far_plane`): `RETAIL_FAR` but with High. Every world
	/// draw and every pass that reads their depth back uses it.
	pub(in crate::viewer) fn world_far(&self) -> f32 {
		super::fardraw::far_plane(self.far_draw())
	}

	/// [PC] Graphics -> Culling (On by default): Off draws the ties, tfrags and props this renderer leaves out as hidden (the
	/// occlusion mask) or out of view (the tfrag viewport test), the mobys retail's draw left out for the game camera or its draw
	/// distance (`Prop::cast`), and drops every draw distance: ties, tfrags (the cap), shrubs, mobys (and their fade), debris.
	/// Retail's own verdicts (props_drawn) stay as they are; the world far plane (Draw distance) still clips. Never drawn even so:
	/// what no camera cell of the level sees (`Occlusion::ever_visible`) and the mobys with no draw distance (`Culls::never`).
	pub(in crate::viewer) fn culling(&self) -> bool {
		crate::retail::pcext::graphics().culling
	}

	/// BRIDGE-29 / REN-CULL-TIE, once per drawn frame before the scene is borrowed: per tie instance the LOD and geomorph
	/// factor from the view depth of its sphere centre (`ties::tie_pick`; with no clip view every tie draws at LOD 0 unmorphed
	/// [I]), the picked LOD's draws rebuilt into `scene.ties.draws`, and the picked range's positions rewritten when its Q
	/// changed. A tie whose occlusion bytes fail the frame's mask (REN-CULL-OCC, skipped with no clip view) is not drawn.
	pub(in crate::viewer) fn cull_ties(&mut self) {
		let _s = super::profile::span("cull_ties");
		let far = self.far_draw();
		let cull = self.culling();
		let Some(scene) = &mut self.scene else { return };
		if scene.tie_cull.is_empty() {
			return;
		}
		if let Some(g) = &scene.tie_gpu {
			g.flush_pending(&self.device, &self.queue);
		}
		// [PC] perf (lane A, B3): with the same clip view, occlusion mask and draw-distance mode as the last pass and no stale tie
		// range, every pick, Q and draw list is the same as last time and no range is rewritten (`tie_q` already holds each
		// drawn range's Q), so the pass is skipped. RAC_CULL_CHECK=1 runs it anyway and compares.
		// Culling Off passes every tie some camera cell can see (`Occlusion::ever_visible`; one no cell sees is hidden by design), so the mask still keys the pass.
		let key = super::gpucache::TiePassKey { view: scene.clip_view.as_ref().map(super::gpucache::view_bits), mask: scene.occ.mask, far, cull };
		let same_inputs = scene.tie_pass.as_ref() == Some(&key) && scene.tie_lod.len() == scene.tie_cull.len() && !scene.tie_stale.iter().any(|s| s.iter().any(|&b| b));
		let check = super::gpucache::cull_check_on();
		if same_inputs && !check {
			return;
		}
		let before = (same_inputs && check).then(|| (scene.ties.draws.iter().map(|d| (d.inst, d.cull, d.texture, d.start, d.count)).collect::<Vec<_>>(), scene.tie_lod.clone()));
		scene.tie_lod.clear();
		scene.tie_lod.resize(scene.tie_cull.len(), u8::MAX);
		// The draw list and the job list keep their capacity from frame to frame.
		let mut draws = std::mem::take(&mut scene.ties.draws);
		draws.clear();
		// [PC] perf: the vertex rewrites (tie, LOD, Q, moved, all) run on the frame worker pool after the pick loop.
		let mut jobs: Vec<super::tiegpu::Job> = std::mem::take(&mut scene.tie_jobs);
		let gcheck = super::tiegpu::check_on();
		jobs.clear();
		let sp_pick = super::profile::span("ct_pick");
		for i in 0..scene.tie_cull.len() {
			let c = scene.tie_cull[i];
			let pick = match (&scene.clip_view, far) {
				// [PC] Draw distance Medium / High (fardraw.rs): LOD 0, drawn farther. Culling Off: no draw distance at all.
				(Some(v), Some(l)) => super::fardraw::tie(l, &c, v.view(c.centre)[2]).or(Some((0, 0.0)).filter(|_| !cull)),
				(Some(v), None) => if cull { ties::tie_pick(&c, v.view(c.centre)[2]) } else { Some(ties::tie_pick_open(&c, v.view(c.centre)[2])) },
				(None, _) => Some((0, 0.0)),
			};
			let Some((lod, q)) = pick else { continue };
			if scene.clip_view.is_some() && !scene.occ.passes(c.occ) && (cull || !scene.occ.ever_visible(c.occ)) {
				continue;
			}
			draws.extend_from_slice(&scene.tie_draws[scene.tie_ranges[i][lod].start as usize..scene.tie_ranges[i][lod].end as usize]);
			scene.tie_lod[i] = lod as u8;
			// Positions follow Q for fat vertices; colours follow Q too, and a relight leaves every range stale (BRIDGE-32).
			let moved = scene.tie_fat[i][lod] && q != scene.tie_q[i][lod];
			if moved || scene.tie_stale[i][lod] {
				let all = scene.tie_stale[i][lod] || !scene.tie_full[i][lod];
				// [PC] perf (lane D5b): a job inside the proven domain goes to the GPU rewrite (tiegpu.rs).
				let mut job = (i, lod, q, moved, all, false);
				if let Some(g) = scene.tie_gpu.as_mut() {
					job = g.route(job, scene.tie_fat[i][lod], gcheck);
				}
				jobs.push(job);
				scene.tie_q[i][lod] = q;
				scene.tie_stale[i][lod] = false;
				scene.tie_full[i][lod] = true;
			}
		}
		drop(sp_pick);
		if gcheck {
			if let Some(g) = scene.tie_gpu.as_mut() {
				for j in g.stress_jobs(&scene.tie_lod, &scene.tie_fat, 6) {
					jobs.push(j);
					scene.tie_q[j.0][j.1] = f32::NAN;
					scene.tie_full[j.0][j.1] = false;
					scene.tie_stale[j.0][j.1] = true;
				}
			}
		}
		if let Some(g) = scene.tie_gpu.as_mut() {
			g.flush_caps(&self.queue, &scene.tie_cols);
		}
		{
			let (base, morph, slots, cols, qdep, tri, tris) = (&scene.tie_base, &scene.tie_morph, &scene.tie_slots, &scene.tie_cols, &scene.tie_qdep, &scene.tie_tri, &scene.tie_tris);
			let rewrite = |verts: &mut [Vertex], at: usize, (i, lod, q, moved, all): (usize, usize, f32, bool, bool)| {
				// [PC] perf round 3: the slot products once per piece (`ties::TieColours`, same bytes as `tie_colour`), and each
				// distinct fat-vertex slot triple of the LOD blended once (`tie_triples`).
				let tc = ties::TieColours::new(&cols[i], q);
				let fat: Vec<[f32; 4]> = tris[i][lod].iter().map(|&t| tc.colour((t, true))).collect();
				for (d, v) in verts.iter_mut().enumerate() {
					let k = at + d;
					if !all && !qdep[k] {
						continue;
					}
					if moved {
						v.pos = ties::morphed(base[k], morph[k], q);
					}
					let c = match slots[k].1 {
						true => fat[tri[k] as usize],
						false => tc.colour(slots[k]),
					};
					v.colour = [c[0] / 128.0, c[1] / 128.0, c[2] / 128.0, -2.0];
				}
			};
			// The CPU rewrites every job the GPU does not take (all of them in check mode, which keeps the mirror exact to compare with).
			let sel: Vec<usize> = (0..jobs.len()).filter(|&k| gcheck || !jobs[k].5).collect();
			let ranges: Vec<std::ops::Range<usize>> = sel.iter().map(|&k| scene.tie_vranges[jobs[k].0][jobs[k].1].clone()).collect();
			let sp_rw = super::profile::span("ct_rewrite");
			par_chunked(&mut scene.tie_verts, &ranges, |k, off, verts| {
				let j = jobs[sel[k]];
				rewrite(verts, ranges[k].start + off, (j.0, j.1, j.2, j.3, j.4))
			});
			drop(sp_rw);
			let ranges: Vec<std::ops::Range<usize>> = sel.iter().zip(ranges).filter(|(k, _)| !jobs[**k].5).map(|(_, r)| r).collect();
			let sp_wr = super::profile::span("ct_write");
			// Adjacent ranges go up as one write (same bytes at the same offsets).
			let raw = ranges.len();
			let mut writes = 0usize;
			let mut covered = 0usize;
			let mut cur: Option<std::ops::Range<usize>> = None;
			let mut flush = |r: std::ops::Range<usize>| {
				covered += r.len();
				writes += 1;
				self.queue.write_buffer(&scene.ties.vbuf, (r.start * std::mem::size_of::<Vertex>()) as u64, bytemuck::cast_slice(&scene.tie_verts[r]));
			};
			for vr in ranges.iter().cloned() {
				match cur.take() {
					Some(c) if c.end == vr.start => cur = Some(c.start..vr.end),
					Some(c) => {
						flush(c);
						cur = Some(vr);
					}
					None => cur = Some(vr),
				}
			}
			if let Some(c) = cur {
				flush(c);
			}
			drop(sp_wr);
			let sp_gp = super::profile::span("ct_gpu");
			let mut tasks = 0;
			if let Some(g) = scene.tie_gpu.as_mut() {
				if jobs.iter().any(|j| j.5) {
					tasks = g.dispatch(&self.device, &self.queue, &jobs, &scene.tie_vranges, &scene.tie_verts, gcheck);
				}
			}
			drop(sp_gp);
			if let Some(g) = &scene.tie_gpu {
				g.note_pass(scene.tie_cull.len(), draws.len(), &jobs, &scene.tie_vranges, tasks);
			}
			if check {
				let total: usize = ranges.iter().map(|r| r.len()).sum();
				let same_out = before.as_ref().is_none_or(|(d, l)| {
					jobs.is_empty() && *l == scene.tie_lod && d.len() == draws.len() && d.iter().zip(&draws).all(|(a, b)| *a == (b.inst, b.cull, b.texture, b.start, b.count))
				}) && covered == total;
				super::gpucache::cull_note(same_inputs, same_out, raw, writes);
			}
		}
		scene.ties.draws = draws;
		scene.tie_jobs = jobs;
		scene.tie_pass = Some(key);
	}

	/// BRIDGE-27 / REN-CULL-SHRUB, first thing in `draw`: per shrub instance (class load-slot order) the view depth of its sphere
	/// centre gives culled / opaque / faded (`ShrubCull::state`); a changed fade alpha is rewritten into the instance's vertex
	/// colours, the draw lists are rebuilt and the far-LOD billboards of the frame are queued (pass 1 before pass 2).
	pub(in crate::viewer) fn cull_shrubs(&mut self, cam: &Camera) {
		let _s = super::profile::span("cull_shrubs");
		let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
		let cap = self.shrub_cap;
		let far = self.far_draw();
		let cull = self.culling();
		let Some(scene) = &mut self.scene else { return };
		scene.shrub_opaque.clear();
		scene.shrub_faded.clear();
		scene.shrub_bb_quads.clear();
		if scene.shrub_cull.is_empty() {
			return;
		}
		let cv = super::clip::ClipView::new(cam, aspect);
		let (mut pass1, mut pass2) = (Vec::new(), Vec::new());
		for k in 0..scene.shrub_order.len() {
			let i = scene.shrub_order[k];
			let c = scene.shrub_cull[i];
			// [PC] Draw distance Medium / High (fardraw.rs): the mesh and its billboard switch farther out.
			let (pz, cap) = match far {
				Some(l) => super::fardraw::shrub(l, cv.view(c.centre)[2], cap),
				None => (cv.view(c.centre)[2], cap),
			};
			// [PC] Culling Off: no draw distance (`ShrubCull::state_open`).
			let Some(st) = (if cull { c.state_cap(pz, cap) } else { Some(c.state_open(pz)) }) else { continue };
			if let Some(a) = st.mesh {
				if let Some(w) = shrub_rewrite(scene.shrub_a[i], a) {
					let vr = scene.shrub_ranges[i].1.clone();
					for v in &mut scene.shrub_verts[vr.clone()] {
						v.colour[3] = w;
					}
					self.queue.write_buffer(&scene.shrubs.vbuf, (vr.start * std::mem::size_of::<Vertex>()) as u64, bytemuck::cast_slice(&scene.shrub_verts[vr]));
					scene.shrub_a[i] = a;
				}
				// Unfaded meshes blend their texel alpha too: retail's partial-alpha grass patches are soft and faint
				// (PCSX2 Eudora capture s07, 2026-10-07) [I], so every mesh takes the alpha path with A = 0x80.
				scene.shrub_faded.extend(scene.shrub_ranges[i].0.clone());
			}
			if st.bb == 0 {
				continue;
			}
			let Some(b) = &scene.shrub_bb[i] else { continue };
			let Some(&(_, mxl)) = scene.shrub_bb_tex.get(&b.class_id) else { continue };
			let q = b.corners(cam.pos);
			let colour = [b.colour[0] as f32 / 128.0, b.colour[1] as f32 / 128.0, b.colour[2] as f32 / 128.0, shrub_carrier(st.bb)];
			let quad = ShrubQuad { pos: [q[0], q[1], q[3], q[2]], colour, class_id: b.class_id, lod_k: b.lod_k, mxl: mxl as f32, pass1: st.bb & 0x80 != 0 };
			if quad.pass1 {
				pass1.push(quad);
			} else {
				pass2.push(quad);
			}
		}
		scene.shrub_bb_quads = pass1;
		scene.shrub_bb_quads.extend(pass2);
	}

	/// [PC] icefx.rs, first thing in `draw`: once per level the ice marks on the tie and shrub vertices of the level's ice textures.
	pub(in crate::viewer) fn tag_ice(&mut self) {
		let level = self.level;
		let Some(scene) = &mut self.scene else { return };
		if scene.ice_level == level {
			return;
		}
		scene.ice_level = level;
		let (tk, sk) = super::icefx::ice_keys(level);
		// [PC] perf (lane D5b): after GPU rewrites the mirror is stale, so only the ice marks (light w) of the changed vertices go up.
		let before = scene.tie_gpu.as_ref().is_some_and(|g| g.dirty_mirror()).then(|| scene.tie_verts.iter().map(|v| v.light[3]).collect::<Vec<f32>>());
		if super::icefx::retag(&mut scene.tie_verts, &scene.tie_vkey, tk, &[], -3.0) {
			match before {
				None => self.queue.write_buffer(&scene.ties.vbuf, 0, bytemuck::cast_slice(&scene.tie_verts)),
				Some(b) => {
					let off = std::mem::offset_of!(Vertex, light) + 12;
					for (k, (v, w)) in scene.tie_verts.iter().zip(&b).enumerate() {
						if v.light[3] != *w {
							self.queue.write_buffer(&scene.ties.vbuf, (k * std::mem::size_of::<Vertex>() + off) as u64, bytemuck::bytes_of(&v.light[3]));
						}
					}
				}
			}
		}
		if super::icefx::retag(&mut scene.shrub_verts, &scene.shrub_vkey, sk, &scene.shrub_soft, -2.0) {
			self.queue.write_buffer(&scene.shrubs.vbuf, 0, bytemuck::cast_slice(&scene.shrub_verts));
		}
	}

	/// REN-CULL-SHRUB-BBTEX: the billboard texture chain of each shrub class (key = class id), uploaded level by level with the
	/// clamping sampler; the mip count - 1 goes to the shader as the top level.
	pub(in crate::viewer) fn set_shrub_bb_textures(&mut self, chains: &HashMap<u32, Vec<Texture>>) {
		let smooth_clamp = super::texfilter::clamp_sampler(&self.device);
		let Some(mut scene) = self.scene.take() else { return };
		for (&class, chain) in chains.iter().filter(|(_, c)| !c.is_empty()) {
			scene.shrub_bb_tex.insert(class, (self.upload_chain(chain, &smooth_clamp), chain.len() as u32 - 1));
		}
		self.scene = Some(scene);
	}

	/// BRIDGE-31 / REN-CULL-TFRAG, once per drawn frame after `relight` and `set_clip_view`: per terrain fragment the occlusion
	/// mask, the 500 m cap, the box test (a fragment partly outside the guard band draws its near mesh unmorphed), the LOD
	/// code, and the picked mesh's morphed / collapsed vertices (`terrain::pose`), rewritten into `scene.tfrags.vbuf` only
	/// where they changed. With no clip view every fragment draws its plain near mesh [I].
	pub(in crate::viewer) fn cull_tfrags(&mut self) {
		let _s = super::profile::span("cull_tfrags");
		let fog = self.lighting.as_ref().map_or([0.0, 524288.0, 255.0, 0.0], |l| l.fog);
		let far = self.far_draw();
		let cull = self.culling();
		let Some(scene) = &mut self.scene else { return };
		if scene.tfrag_set.is_empty() {
			return;
		}
		// FUN_001f7900: the clip-w depth scale (the fog slope per 1024 world units).
		let cc60 = (fog[3] - fog[2]) / ((fog[1] - fog[0]).max(0.001) / 1024.0);
		let (t, ints) = terrain::lod_distances(scene.tfrag_b);
		scene.tfrag_lod.clear();
		scene.tfrag_lod.resize(scene.tfrag_set.len(), u8::MAX);
		let mut draws = Vec::new();
		let mut st = [0u32; 4];
		// [PC] perf: (fragment, LOD code, LOD) of the drawn fragments; the posing and the vertex compare run on the frame worker pool.
		// [PC] perf (ncull, tfrag_memo.rs): the 4th field marks a pose the memo says can be skipped (posed anyway in check mode).
		let mut jobs: Vec<(usize, u8, usize, bool)> = Vec::new();
		let (memo_on, check) = (super::tfrag_memo::on(), super::gpucache::cull_check_on());
		if scene.tfrag_memo.len() != scene.tfrag_set.len() {
			scene.tfrag_memo = vec![[0; 3]; scene.tfrag_set.len()];
		}
		let sp_pick = super::profile::span("ctf_pick");
		for i in 0..scene.tfrag_set.len() {
			let l = &scene.tfrag_set[i];
			let c = &l.cull;
			if tfrag_probe_skip(i) {
				continue;
			}
			if tfrag_probe_info(i) {
				if let Some(v) = &scene.clip_view {
					let p = v.view(c.centre).map(|x| x * 1024.0);
					let r = c.radius * 1024.0;
					let flags: Vec<u8> = c.corners.iter().map(|&k| v.viewport().flags(v.view(k))).collect();
					let bands: Vec<u8> = c.corners.iter().map(|&k| v.band().flags(v.view(k))).collect();
					eprintln!(
						"TFINFO i={i} centre={:?} r={r} view_p={p:?} cap={} near={} occ={} inside={} flags={flags:?} band={bands:?} force_far={} lod_code={:#x} side_out_x={} side_out_y={}",
						c.centre,
						terrain::beyond_draw_cap(p[2], r),
						32.0 - (p[2] + r) >= 0.0,
						scene.occ.passes(c.occ),
						v.sphere_wholly_inside(p, r),
						c.force_far,
						terrain::lod_code(c.force_far, (p[2] - r) as i32, (p[2] + r) as i32, ints),
						0.63 * p[2] - (p[0].abs() - r * 1.1819053) < 0.0,
						0.47628 * p[2] - (p[1].abs() - r * 1.1076293) < 0.0
					);
				}
			}
			let code = match &scene.clip_view {
				None => 0x14,
				Some(v) => {
					let p = v.view(c.centre).map(|x| x * 1024.0);
					let r = c.radius * 1024.0;
					// [PC] Draw distance Medium / High (fardraw.rs): the cap moves out to the level's cap.
					// [PC] Culling Off: no cap (the world far plane still clips).
					if cull && match far { Some(l) => super::fardraw::tfrag_beyond(l, p[2], r), None => terrain::beyond_draw_cap(p[2], r) } { st[0] += 1; continue; }
					if 32.0 - (p[2] + r) >= 0.0 { st[1] += 1; continue; }
					if !scene.occ.passes(c.occ) && (cull || !scene.occ.ever_visible(c.occ)) { st[2] += 1; continue; }
					let mut near_only = false;
					if !v.sphere_wholly_inside(p, r) {
						// [PC] perf (ncull): no flag vectors; the same ANDed / ORed corner flags.
						let (vp, bd) = (v.viewport(), v.band());
						if cull && c.corners.iter().fold(0x3f, |a, &k| a & vp.flags(v.view(k))) != 0 {
							st[3] += 1;
							continue;
						}
						near_only = c.corners.iter().any(|&k| bd.flags(v.view(k)) != 0);
					}
					// [PC] Draw distance Medium / High: the full-detail near mesh unmorphed (a forced-far fragment keeps its far mesh).
					if near_only || (far.is_some() && !c.force_far) {
						0x14
					} else {
						terrain::lod_code(c.force_far, (p[2] - r) as i32, (p[2] + r) as i32, ints)
					}
				}
			};
			let lod = match code {
				6 => 0,
				8 | 0xa => 1,
				_ => 2,
			};
			draws.extend_from_slice(&scene.tfrag_draws[scene.tfrag_ranges[i][lod].start as usize..scene.tfrag_ranges[i][lod].end as usize]);
			scene.tfrag_lod[i] = lod as u8;
			let hit = memo_on && super::tfrag_memo::skip(&scene.tfrag_memo, i, lod, code);
			if !hit || check {
				jobs.push((i, code, lod, hit));
			}
		}
		if std::env::var_os("RAC_TFRAG_STATS").is_some() { eprintln!("TFSTATS total={} cap={} near={} occ={} frustum={} drawn={} occ_enabled={}", scene.tfrag_set.len(), st[0], st[1], st[2], st[3], scene.tfrag_lod.iter().filter(|&&l| l != u8::MAX).count(), scene.occ.enabled); }
		drop(sp_pick);
		let ranges: Vec<std::ops::Range<usize>> = jobs.iter().map(|&(i, _, lod, _)| scene.tfrag_vranges[i][lod].clone()).collect();
		let sp_pose = super::profile::span("ctf_pose");
		let dirty = {
			let (set, cols, view) = (&scene.tfrag_set, &scene.tfrag_cols, scene.clip_view);
			// [PC] perf (ncull): a few small jobs run here, not through the frame worker pool's fork and join (on a loaded CPU that
			// wait is far longer than the work).
			let serial = ranges.iter().map(|r| r.len()).sum::<usize>() < SERIAL_VERTS;
			let run = |j: usize, verts: &mut [Vertex]| {
				let (i, code, _, _) = jobs[j];
				let posed = match &view {
					Some(v) => terrain::pose(&set[i], code, t, cc60, &cols[i], &|p| v.view(p)[2]),
					None => terrain::pose(&set[i], code, t, cc60, &cols[i], &|_| 0.0),
				};
				let coloured = !cols[i].is_empty();
				let mut changed = false;
				for (k, v) in verts.iter_mut().enumerate() {
					let colour = if coloured { [posed.col[k][0] / 128.0, posed.col[k][1] / 128.0, posed.col[k][2] / 128.0, -2.0] } else { v.colour };
					if v.pos != posed.pos[k] || v.uv != posed.uv[k] || v.colour != colour {
						(v.pos, v.uv, v.colour) = (posed.pos[k], posed.uv[k], colour);
						changed = true;
					}
				}
				changed
			};
			if serial {
				ranges.iter().enumerate().map(|(j, r)| run(j, &mut scene.tfrag_verts[r.clone()])).collect::<Vec<bool>>()
			} else {
				par_ranges(&mut scene.tfrag_verts, &ranges, run)
			}
		};
		if check {
			let bad = jobs.iter().zip(&dirty).filter(|(j, d)| j.3 && **d).count();
			super::profile::span_add("ctf_memo_bad", bad as f32 * 1e-3);
		}
		for &(i, code, lod, _) in &jobs {
			super::tfrag_memo::record(&mut scene.tfrag_memo, i, lod, code);
		}
		drop(sp_pose);
		let sp_wr = super::profile::span("ctf_write");
		let (mut n_w, mut n_v) = (0usize, 0usize);
		for (vr, _) in ranges.into_iter().zip(&dirty).filter(|(_, d)| **d) {
			n_w += 1;
			n_v += vr.len();
			self.queue.write_buffer(&scene.tfrags.vbuf, (vr.start * std::mem::size_of::<Vertex>()) as u64, bytemuck::cast_slice(&scene.tfrag_verts[vr]));
		}
		drop(sp_wr);
		// [PC] profile only (ncull, 2026-10-29): writes and vertices written this frame, as counts shown in "us" (1 us = 1 write / 1 vertex).
		super::profile::span_add("ctf_n_writes", n_w as f32 * 1e-3);
		super::profile::span_add("ctf_n_verts", n_v as f32 * 1e-3);
		scene.tfrags.draws = draws;
	}
}

/// [PC] perf (ncull): below this many vertices of pose jobs `cull_tfrags` poses on the render thread.
const SERIAL_VERTS: usize = 3000;

/// The vertex colour alpha to write when a shrub's fade state changes from `cached` to `a` (0x80 = opaque), None when unchanged.
pub(super) fn shrub_rewrite(cached: u8, a: u8) -> Option<f32> {
	(a != cached).then(|| shrub_carrier(a))
}

/// Instance indices in retail's walk order: classes by load slot, records in gameplay order inside a class (stable sort).
pub(super) fn shrub_order(set: &[shrubs::ShrubInstance]) -> Vec<usize> {
	let mut order: Vec<usize> = (0..set.len()).collect();
	order.sort_by_key(|&i| set[i].class_slot);
	order
}
