//! The posed hero marker model (set_marker) and its clipped polygons.

use super::*;
use super::marker_clip;
use super::marker_ctx::{MarkerCtx, MarkerIn, Own, Pose};

/// One `set_marker` call that ran the compute build (gpuhero.rs): what the rest of the call needs to know.
struct GpuFrame {
	/// The clipper has nothing to do for any pose inside the hero's sphere (`gpuskin::sphere_skips`).
	skip: bool,
	/// The GPU vertex buffer is the one drawn (no CPU build, no upload).
	taken: bool,
	/// The blend factor the GPU build used (1.0 without a blend source).
	t: f32,
	/// The CPU build runs too and its clip result is compared with the selective clipper's (check modes).
	check_clip: bool,
	self_kind: Option<usize>,
	check_bytes: Option<Vec<u8>>,
}

impl GpuFrame {
	/// The selective clipper can answer this frame (only then is its result compared in the check modes): the chunk spheres bound
	/// the lerp of the two poses for a factor in 0..=1 only, and a skip frame has nothing to clip.
	fn selective_ok(&self) -> bool {
		!self.skip && (0.0..=1.0).contains(&self.t)
	}

	/// The CPU build of this frame is compared with the GPU's (a blocking check mode or an Auto self-check frame): it must be rebuilt.
	fn cpu_compare(&self) -> bool {
		self.check_bytes.is_some() || self.self_kind.is_some()
	}
}

/// [PC] perf (lane R2, gpuhero.rs): a compute pass builds the vertices from the model and the two step poses (RAC_HERO_GPU=1).
/// Since lane A (v0.4.2) the build also serves the frames where the hero is not clearly inside / outside the clip guard band: the
/// clipper then looks only at the faces that may cross it (`marker_clip.rs`). RAC_HERO_GPU_CHECK=1 runs both and compares the bytes.
fn gpu_build(queue: &GatedQueue, device: &wgpu::Device, scene: &mut Scene, inp: &MarkerIn, cur: &Posed, blend: Option<(&Posed, f32)>, gens: (u64, u64)) -> Option<GpuFrame> {
	if scene.marker_gpu.is_none() || !super::gpuhero::gpu_hero_on() {
		return None;
	}
	let n_model = scene.marker_model.len();
	let hg = scene.marker_gpu.as_mut()?;
	// A check asked by an earlier call of this frame that no encoder has recorded yet is dropped (check.rs).
	hg.drop_unrecorded();
	let _p = super::profile::span("sm_poses");
	let can_blend = hg.set_poses(queue, blend.map(|b| (b.0, gens.1)), (cur, gens.0));
	drop(_p);
	let t = if can_blend { blend.map_or(1.0, |b| b.1) } else { 1.0 };
	let skip = scene.clip_view.as_ref().is_none_or(|v| super::gpuskin::sphere_skips(v, inp.feet, hg.radius));
	// The first frames of each kind run the GPU build and the CPU build and compare them (the CPU result is drawn).
	let extras = inp.ghosts.iter().any(Option::is_some) || inp.bomb.is_some() || inp.shift.is_some() || inp.clank.is_some();
	let kind = super::gpuhero::kind(can_blend && t != 1.0, inp.rows.is_some(), extras, inp.own.iter().any(Option::is_some));
	let mut hero_check = super::gpuhero::gpu_hero_check();
	let mut self_kind = None;
	// Auto self-check frame: the GPU build is read back without a wait (gpuhero/check.rs); one check in flight at a time. A kind
	// not verified yet draws the CPU build also while another frame's check is in flight (`taken` below).
	let unverified = !hero_check && super::gpuhero::self_check_wanted(kind);
	if unverified && !hg.check_busy() {
		self_kind = Some(kind);
	} else if !hero_check && !skip && super::gpucache::marker_check_on() {
		// RAC_MARKER_CHECK=1: the guard-band frames also run the legacy CPU build and compare the clipper's result with it.
		hero_check = true;
	}
	let _q = super::profile::span("sm_queue");
	hg.queue_build(queue, inp.hero_params(hg.parity(), t, can_blend, n_model));
	drop(_q);
	// The selective clipper's chunk spheres bound the lerp of the two poses only for a factor in 0..=1.
	// After a selective-clip mismatch (check frames) the guard-band frames take the full CPU path for good, as a failed self-check does.
	let taken = !hero_check && !unverified && (skip || ((0.0..=1.0).contains(&t) && !marker_clip::failed()));
	let check_bytes = hero_check.then(|| hg.build_and_read(device, queue, &scene.marker.vbuf));
	if let Some(k) = self_kind {
		let _s = super::profile::span("sm_start");
		hg.start_check(device, queue, &scene.marker.vbuf, k);
	}
	Some(GpuFrame { skip, taken, t, check_clip: hero_check || self_kind.is_some(), self_kind, check_bytes })
}

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
		let loose = std::mem::take(&mut self.marker_loose);
		let bomb = self.marker_bomb.take();
		let tilt = std::mem::take(&mut self.marker_tilt);
		let clank_light = self.marker_light.take();
		let Some(scene) = &mut self.scene else { return };
		// An Auto self-check in flight (gpuhero/check.rs): collected here without blocking.
		if let Some(hg) = scene.marker_gpu.as_mut() {
			let _p = super::profile::span("sm_poll");
			hg.poll_check(&self.device);
		}
		// NOV-5.9d: the clip queue is rebuilt every frame, marker polygons first.
		scene.clip_queue.clear();
		let Some((feet, yaw)) = at else {
			scene.marker_shown = false;
			scene.marker_alpha = false;
			return;
		};
		// The skinned pose replaces the model's positions and normals (PC perf: per vertex, into a reused buffer).
		let skin0 = skinned.filter(|(ps, _)| ps.len() == scene.marker_model.len());
		let (c, sn) = (yaw.cos(), yaw.sin());
		let rows = (tilt != [0.0; 2]).then(|| crate::particles::euler_m([tilt[0], tilt[1], yaw]));
		let _l = super::profile::span("sm_light");
		let (light, tint) = Self::hero_light_tint(&self.lighting, feet, self.hero_placed, &mut self.hero_env);
		self.hero_tint_out = Some(tint);
		let id = dyn_id(&self.table, &mut self.moby_dyn, feet);
		// A loose wrench is lit like the moby it is: its own light and the third light where it stands (props_set.rs does the same).
		let own = loose.map(|r| r.map(|r| Own { a: r.a, b: r.b, light: Self::moby_light(r.light), id: dyn_id(&self.table, &mut self.moby_dyn, r.pos) }));
		drop(_l);
		let shift = self.marker_v.take();
		let inp = MarkerIn { feet, c, sn, rows, tint: scene.marker_tint, hero_len: scene.marker_hero_len, clank: clank_light, ghosts, bomb, shift, id, light, own };
		let alpha = ghosts.iter().any(Option::is_some);
		let n_model = scene.marker_model.len();
		let blend = blend.filter(|(b, _)| skin0.is_some() && b.0.len() == n_model && b.1.len() == n_model);
		let gens = std::mem::take(&mut self.hero_gens);
		let gf = skin0.and_then(|cur| gpu_build(&self.queue, &self.device, scene, &inp, cur, blend, gens));
		if let Some(gf) = gf.as_ref().filter(|g| g.taken) {
			// The GPU wrote the vertex buffer. The clipper's verdict is "no face to take" when the hero is wholly inside / outside the
			// guard band (unmask what an earlier frame masked); else it takes the faces the selective clipper finds.
			let (taken, polys, _) = if gf.skip { Default::default() } else { marker_clip::select(scene, &inp, gf.t) };
			marker_clip::finish(&self.queue, scene, taken, polys);
			scene.marker_verts.clear();
			scene.marker_key = None;
			scene.marker_shown = true;
			scene.marker_alpha = alpha;
			return;
		}
		if let Some(hg) = scene.marker_gpu.as_mut() {
			hg.cancel();
		}
		let sel = gf.as_ref().filter(|g| g.check_clip && g.selective_ok()).map(|g| (&inp, g.t));
		// The CPU build: the blend the GPU would have done.
		let blended_cpu = blend.and_then(|(b, t)| skin0.and_then(|s| super::smooth::blend_posed(b, s, t)));
		let skin = blended_cpu.as_ref().or(skin0);
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
			own: own.map(|o| o.map(|o| (o.a, o.b, o.light.map(f32::to_bits), o.id.to_bits()))),
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
		if same_inputs && !check && gf.as_ref().is_none_or(|g| !g.cpu_compare()) {
			let verts = std::mem::take(&mut scene.marker_verts);
			clip_marker(&self.queue, scene, &verts, None);
			scene.marker_verts = verts;
			scene.marker_shown = true;
			scene.marker_alpha = alpha;
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
		let why = if check { scene.marker_key.as_ref().map_or(0x3fff, |k| k.diff(&key)) | (!skin_same as u32) << 13 } else { 0 };
		scene.marker_key = Some(key);
		let mut verts = std::mem::take(&mut scene.marker_verts);
		let _cpu = super::profile::span("sm_cpu");
		verts.clear();
		let ctx = MarkerCtx { inp, model: &scene.marker_model, pose: skin.map_or(Pose::Model, Pose::Plain) };
		verts.extend((0..n_model.min(skin.map_or(usize::MAX, |(_, ns)| ns.len()))).map(|k| ctx.vertex(k)));
		let _w = super::profile::span("sm_write");
		self.queue.write_buffer(&scene.marker.vbuf, 0, bytemuck::cast_slice(&verts));
		drop(_w);
		if let Some(gb) = gf.as_ref().and_then(|g| g.check_bytes.as_ref()) {
			super::gpuhero::note_check(gb, bytemuck::cast_slice(&verts));
		}
		if let (true, Some(hg)) = (gf.as_ref().is_some_and(|g| g.self_kind.is_some()), scene.marker_gpu.as_mut()) {
			hg.set_check_cpu(bytemuck::cast_slice(&verts));
		}
		if check {
			super::gpucache::marker_note(same_inputs, kept.as_ref().is_none_or(|k| bytemuck::cast_slice::<_, u8>(&k[..]) == bytemuck::cast_slice::<_, u8>(&verts[..])), why);
		}
		drop(_cpu);
		clip_marker(&self.queue, scene, &verts, sel);
		scene.marker_verts = verts;
		scene.marker_shown = true;
		scene.marker_alpha = alpha;
	}
}

/// NOV-5.9d path 2: the hero marker (body and held gadgets, world-space `verts`) takes the same per-face clip; masked faces
/// are zeroed in `marker.ibuf` with the compare-and-write rule of `clip_props`. `sel` (check modes) = the inputs of the selective
/// clipper (its poses are the ones the GPU draws, `HeroGpu::kept_pose`), whose result must equal this full one.
fn clip_marker(queue: &GatedQueue, scene: &mut Scene, verts: &[Vertex], sel: Option<(&MarkerIn, f32)>) {
	let _c = super::profile::span("sm_clip");
	let (taken, polys) = marker_clip::full(scene.clip_view, &scene.marker_faces, verts);
	if let Some((inp, t)) = sel {
		marker_clip::check(scene, inp, t, &taken, &polys);
	}
	marker_clip::finish(queue, scene, taken, polys);
}
