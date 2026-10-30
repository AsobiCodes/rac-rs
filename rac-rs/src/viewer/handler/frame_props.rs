//! RedrawRequested: props phase (prop poses).

use super::*;

impl App {
	/// Which props are posed this frame, and which of those are blended: a shown prop always; a camera-culled moby (`Prop::cast`) is
	/// used only by the sun shadow pass (Enhanced lighting, Dynamic shadows on: its pose is the shadow's shape) and, with Culling Off,
	/// drawn by the main pass, so it is posed in exactly those cases (Classic and the default Culling pose nothing more). The shadow
	/// pass only takes the ones in a cascade box (`SunGate`); the ones within `HISTORY_MARGIN` of a box are posed for their pose
	/// history, the ones in a box are blended too.
	fn posing(&self, game: &Game, g: crate::retail::pcext::Graphics) -> (Vec<bool>, Vec<bool>) {
		let sun = SunGate::of(self, g);
		game.props.iter().map(|p| {
			if p.shown || (p.cast && !g.culling) {
				return (true, true);
			}
			let sphere = || self.prop_anims.get(&p.class_id).map(|c| c.0.bsphere);
			match &sun {
				Some(s) if p.cast => (s.takes(p, sphere(), HISTORY_MARGIN), s.takes(p, sphere(), 0.0)),
				_ => (false, false),
			}
		}).unzip()
	}

	pub(super) fn frame_props(&mut self, f: &mut Frame) {
		let flight_pose = f.flight_pose;
		let g = crate::retail::pcext::graphics();
		let (pose, blend) = match &self.game {
			Some(game) => self.posing(game, g),
			None => (Vec::new(), Vec::new()),
		};
		if let (Some(game), Some(w)) = (&mut self.game, &self.world) {
			let p = &game.player;
			let mut posed: HashMap<PoseKey, Posed> = HashMap::new();
			// Vendor and orb: blend / joint-spin poses (vendor.rs vendor_fx), one skinning per prop.
			let mut fx_posed: HashMap<usize, std::sync::Arc<Posed>> = HashMap::new();
			// The skinning of the props with joint records is done on the worker pool below (`fx_jobs`).
			let mut fx_jobs: Vec<(usize, &mobies::MobyClass, Vec<[f32; 12]>)> = Vec::new();
			for (pi, pr) in game.props.iter().enumerate().filter(|(pi, _)| pose[*pi]) {
				// SCN-4.3c: the flying ship blends sequence 1 frame 0 into sequence 2 by the flight's +0x54.
				if let (Some(fp), Some((class, a))) = (flight_pose.filter(|fp| fp.prop == pi), self.prop_anims.get(&pr.class_id)) {
					if a.sequences.len() > 2 && !a.sequences[1].frames.is_empty() && !a.sequences[2].frames.is_empty() {
						let (f0, f1) = game::transit::ship_blend_frames(a.sequences[1].frames.len());
						fx_posed.insert(pi, std::sync::Arc::new(anim::skin_all(class, &a.pose_frames(&a.sequences[1].frames[f0], &a.sequences[2].frames[f1.min(a.sequences[2].frames.len() - 1)], fp.blend))));
						continue;
					}
				}
				// Vendor/orb spins, and any prop fading from a snapshot (its pose is its own).
				let spins = game.vendor_fx.poses.get(&pi).map(|fx| fx.spins.as_slice());
				if let Some((class, a)) = self.prop_anims.get(&pr.class_id) {
					// NOV-14.8e: the Plumber's head-look joint overrides.
					// KER-1a: the Kerwan walker's blocks drive the attach slot 2 / 3 joints.
					let mut fx: Vec<anim::JointFx> = Vec::new();
					// NOV-30c: the shop vendor's icon bar corner records.
					fx.extend(game.shop_bar_fx(pi, &a.slot_map));
					// M3a, retail mode: the moby's own +0x64 records (e.g. Clank's blink), as the evaluator FUN_00268a80 applies them.
					if let Some(r) = &self.retail {
						fx.extend(r.prop_fx(pi).iter().cloned());
					}
					if spins.is_some() || pr.anim.fading() || !fx.is_empty() {
						fx_jobs.push((pi, class, pr.anim.pose_fx_full(a, spins.unwrap_or(&[]), &fx)));
					}
				}
			}
			// [PC] perf: a prop whose class and joint matrices are last frame's keeps its skinning (fx_skin.rs).
			{ let _s = profile::span("prop_skin_fx"); self.smooth.fx_skins.skin(fx_jobs, &mut fx_posed); }
			// PC perf: no game tick since the last frame's poses (same stamp): the anim states are unchanged, so each
			// pose key's skinning is moved over from that frame instead of recomputed.
			let stamp = (game.frames, game.props.as_ptr() as usize, game.props.len());
			let mut last = if self.smooth.ticks == 0 && self.pose_stamp == Some(stamp) { self.smooth.take_posed() } else { HashMap::new() };
			self.pose_stamp = Some(stamp);
			// [PC] perf: the joint matrices per new key here, the skinning on the frame worker pool; the keys go into
			// `posed` in first-seen order as before. A key skinned from last frame's class and matrices takes last frame's pose
			// (`pose_born.rs`, which also gives each key pose its token).
			let mut todo: Vec<(PoseKey, usize, pose_born::KeyJob)> = Vec::new();
			for pr in game.props.iter().enumerate().filter(|(pi, _)| pose[*pi]).map(|(_, p)| p) {
				if let Some((class, a)) = self.prop_anims.get(&pr.class_id) {
					let key = pose_key(pr);
					if !posed.contains_key(&key) {
						let job = match last.remove(&key) {
							Some(p) => pose_born::KeyJob::Moved(p),
							None => self.smooth.fx_skins.keys.job(key, class, pr.anim.pose(a, &[]), self.smooth.posed()),
						};
						todo.push((key, class as *const mobies::MobyClass as usize, job));
						posed.insert(key, Default::default());
					}
				}
			}
			drop(last);
			{
				use rayon::prelude::*;
				use pose_born::KeyJob;
				let _s = profile::span("prop_skin");
				let done: Vec<Posed> = todo.par_iter_mut().map(|(_, _, job)| match job {
					KeyJob::Moved(p) | KeyJob::Kept(p) => std::mem::take(p),
					KeyJob::Skin(class, mats) => anim::skin_all(class, mats),
				}).collect();
				for ((key, _, _), p) in todo.iter().zip(done) {
					posed.insert(*key, p);
				}
				let fs = &mut self.smooth.fx_skins;
				fs.keys.settle(todo, &mut fs.born.keys);
			}
			// PC-FPS-1: blended poses join `fx_posed`, blended placements go to `smooth::PROP_XF`.
			{ let _s = profile::span("sm_props"); self.smooth.props(&game.props, posed, &mut fx_posed, &blend); }
			profile::lap(profile::Phase::Props);
			f.fx_posed = fx_posed;
		}
		self.apply_follow(f.at, f.tilt);
	}
}
