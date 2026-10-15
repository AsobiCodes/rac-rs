//! Retail mode: draws the arrival warp retail runs (mode 6 substate 4) from retail's own warp state
//! (`retail::warp::WarpRam`): the stream type, segment, tick, drift, fade and ribbon history come from RAM every field,
//! the camera is retail's render camera; the ported SCN-4.1c pose and SCN-4.2 draw pass (`Warp::draw_frame`, warpfx.rs,
//! the BRIDGE-35 actor batch) turn them into the picture. Nothing here runs outside retail mode.

use super::*;
use crate::game::transit::{Warp, WarpTick, FAN_BASE, FAN_DIR, SHRINK, SHRINK_WINDOW, WARP_FLOOR};
use crate::retail::warp::WarpRam;

/// The warp bundle and what was built from it, kept while retail stays in the warp.
struct MirrorWarp {
	bundle: Vec<u8>,
	assets: scene::WarpAssets,
	anims: HashMap<u32, anim::Animated>,
	/// (segment, type) of `stream`.
	key: (u32, usize),
	stream: Option<scene::Scene>,
	/// (segment, type, tick) last drawn.
	drawn: Option<(u32, usize, u32)>,
	/// The level's directional lights, replaced by retail's RAM table while the warp draws.
	saved_lights: Option<Vec<crate::gameplay::DirLight>>,
}

thread_local! {
	static MIRROR: std::cell::RefCell<Option<MirrorWarp>> = const { std::cell::RefCell::new(None) };
}

/// SCN-4.1 steps 4, 5 and 7 for retail's state (the tick already ran; the drift is RAM's).
fn tick_of(w: &WarpRam) -> WarpTick {
	let quiet = w.to == 0 || (w.to == 1 && !w.kerwan);
	let k = if w.ty == 4 && w.end > 0 { w.end.saturating_sub(w.tick) as f32 / w.end as f32 } else { 1.0 };
	let base = (w.tick as f32 - 100.0) * 20.0 * k;
	let mut t = WarpTick { proj_floor: WARP_FLOOR[w.ty], phase: FAN_DIR[w.ty].map(|d| base * d), fan_base: FAN_BASE[w.ty], ..Default::default() };
	if w.ty == 4 {
		t.hidden = quiet;
		if w.tick + SHRINK_WINDOW > w.end {
			let left = w.end.saturating_sub(w.tick);
			t.shrink = Some(left as f32 * SHRINK);
			t.ribbon_alpha = left as u8;
		} else {
			t.ribbon_alpha = SHRINK_WINDOW as u8;
		}
	}
	t
}

/// Once per field after `Retail::sync`: loads the warp on its first field, poses and uploads each new retail tick,
/// and drops it when retail leaves substate 4.
pub(in crate::viewer) fn field(r: &crate::retail::Retail, gpu: Option<&mut Gpu>, iso: &str) {
	let Some(g) = gpu else { return };
	MIRROR.with(|c| {
		let mut c = c.borrow_mut();
		let Some(w) = r.warp_ram() else {
			if let Some(mw) = c.take() {
				g.clear_warp_actors();
				g.clear_warp();
				if let (Some(l), Some(lt)) = (mw.saved_lights, g.lighting.as_mut()) {
					lt.lights = l;
				}
			}
			return;
		};
		if c.is_none() {
			let loaded = (|| -> Result<MirrorWarp, String> {
				let mut d = iso::Disc::open(std::path::Path::new(iso)).map_err(|e| e.to_string())?;
				let bundle = scene::warp_bundle(&mut d)?;
				let assets = scene::warp_assets(&bundle)?;
				let (big, banner) = scene::warp_pair_textures(&bundle, w.from, w.to)?;
				let mut anims = HashMap::new();
				for (id, _, block) in &assets.classes {
					if let Some(a) = anim::animation_of_class_block(block, *id)? {
						anims.insert(*id, a);
					}
				}
				g.load_warp_actors(&assets);
				g.load_warp(&assets, &big, banner.as_ref());
				Ok(MirrorWarp { bundle, assets, anims, key: (u32::MAX, 0), stream: None, drawn: None, saved_lights: g.lighting.as_ref().map(|l| l.lights.clone()) })
			})();
			match loaded {
				Ok(m) => *c = Some(m),
				Err(e) => {
					eprintln!("retail: warp mirror: {e}");
					return;
				}
			}
		}
		let Some(mw) = c.as_mut() else { return };
		if mw.drawn == Some((w.seg, w.ty, w.tick)) {
			return;
		}
		mw.drawn = Some((w.seg, w.ty, w.tick));
		if mw.key != (w.seg, w.ty) {
			mw.key = (w.seg, w.ty);
			mw.stream = scene::warp_stream_from(&mw.bundle, w.ty).map_err(|e| eprintln!("retail: warp stream {}: {e}", w.ty)).ok();
		}
		let t = tick_of(w);
		// SCN-4.1c pose: the last iteration's key (tick + 0.5), the slot-0 ship of mark `mk`, the type-4 shrink.
		let factor = t.shrink.unwrap_or(1.0);
		let (mut posed, mut lights) = (Vec::new(), Vec::new());
		if let Some(s) = &mw.stream {
			for (ai, actor) in s.actors.iter().enumerate() {
				let cl = if ai == 0 && actor.class_id == 0x215 { crate::game::transit::warp_ship_class(w.mk) } else { actor.class_id };
				let (Some(a), Some((_, class, _))) = (mw.anims.get(&cl), mw.assets.classes.iter().find(|k| k.0 == cl)) else { continue };
				let tk = w.tick as f32 + 0.5;
				let (Some((ka, kb, f)), Some(pos)) = (actor.pair(tk), actor.pos_at(tk)) else { continue };
				// FUN_002a2ce0 (Novalis OV:102641-102665) [C RAM 0165]: the actor's clock is set to sequence 2 frame 0 of
				// its own class for both blend slots (+0x52/+0x53 = 2, +0x50/+0x51 = 0, FUN_002632b0), and only the first
				// 0x20 bytes of the frame body (joints 0..3 rotations) are copied from the stream frames k and k + 1 into that
				// one shared frame, so the k + 1 copy wins. Every other joint (gear, canopy, fins) keeps the class's own
				// flight pose: the 38-joint 0x215 stream frames do not fit the 27/28-joint ship marks 0x213/0x214.
				let own = a.sequences.get(2).and_then(|q| q.frames.first()).map(|f0| {
					let mut fr = f0.clone();
					for (j, r) in kb.frame.rotation.iter().take(4).enumerate() {
						if let Some(d) = fr.rotation.get_mut(j) {
							*d = *r;
						}
					}
					fr
				});
				let mats = match &own {
					Some(fr) => a.pose_frames(fr, fr, 0.0),
					None => a.pose_frames(&ka.frame, &kb.frame, f),
				};
				let (mut ps, ns) = anim::skin(class, &mats);
				for p in &mut ps {
					*p = p.map(|x| x * factor);
				}
				if std::env::var_os("RAC_SCENE_LOG").is_some() {
					eprintln!("retail: warp t {} ty {} actor {ai} class {cl:#x} port pos {pos:?} ram {:?} gloss quads {} ram basis {:?} port joint0 {:?} load {:?} cam rows {:?} cam {:?}", w.tick, w.ty, w.actors.get(ai), w.gloss.len(), w.basis.get(ai), mats.first(), w.load, w.cam_rows, w.cam);
						let pj = |j: usize| a.attach_points.get(j).copied().flatten().and_then(|j| mats.get(j)).map(|m| [pos[0] + m[3], pos[1] + m[7], pos[2] + m[11]]);
						if let Some(Some((ck, b))) = w.seqs.get(ai) {
							if let Ok(q) = anim::parse_sequence_frames(b, b, a.skeleton.parents.len()) {
								let (fa, fb) = (q.frames.get(ck.cur as usize), q.frames.get(ck.next as usize));
								if let (Some(fa), Some(fb)) = (fa, fb) {
									let rm = a.pose_frames(fa, fb, ck.p);
									let rj = |j: usize| a.attach_points.get(j).copied().flatten().and_then(|j| rm.get(j)).map(|m| [pos[0] + m[3], pos[1] + m[7], pos[2] + m[11]]);
									eprintln!("retail: warp ram seq frames {} clock {} {} {} {} {} -> p1 {:?} p2 {:?}", q.frames.len(), ck.cur, ck.next, ck.old_seq, ck.new_seq, ck.p, rj(1), rj(2));
								}
							}
						}
						let gc = w.gloss.iter().fold(([0.0f32; 3], 0.0f32), |(c, n), q| { let mut c = c; for p in &q.pos { for k in 0..3 { c[k] += p[k]; } } (c, n + 4.0) });
						eprintln!("retail: warp gloss centroid {:?} n {} ship pos {pos:?}", gc.0.map(|x| x / gc.1.max(1.0)), w.gloss.len());
						eprintln!("retail: warp joints port p1 {:?} p2 {:?} ram {:?} class scale {} unit {} attach {:?}", pj(1), pj(2), w.hist[w.head], class.scale, a.skeleton.unit, &a.attach_points[..a.attach_points.len().min(4)]);
				}
				posed.push((cl, (ps, ns), pos));
				lights.push(w.actors.get(ai).map(|x| x.3));
			}
		}
		// The moby pass lights the stream actors with retail's light table of this field (word +0x38 indexes it).
		if let Some(lt) = g.lighting.as_mut() {
			lt.lights = w.lights.iter().map(|r| crate::gameplay::DirLight {
				colour: [[r[0][0], r[0][1], r[0][2]], [r[2][0], r[2][1], r[2][2]]],
				dir: [[r[1][0], r[1][1], r[1][2]], [r[3][0], r[3][1], r[3][2]]],
				wrap: [r[0][3], r[2][3]],
			}).collect();
		}
		g.set_warp_actors(&posed, t.hidden, &lights);
		let mut warp = Warp::mirror(w.to, w.kerwan, w.hist);
		(warp.ty, warp.seg, warp.tick, warp.cc98, warp.end, warp.fade, warp.drift, warp.head, warp.count) = (w.ty, w.seg, w.tick, w.cc98, w.end, w.fade, w.drift, w.head, w.count);
		let tan = w.cam.map_or(WARP_FLOOR[w.ty], |c| c.3);
		let mut f = warp.draw_frame(&t, tan, mw.assets.sky.meshes.len(), w.mk);
		// Retail draws the planet banner as GS path 2/3 sprites [C: the capture overlay shows it, stage 6 grid16/17]:
		// with the capture (or GS-full) on, it comes from there, so the native SCN-4.2 banner would draw it twice.
		if r.gs {
			f.banner = None;
		}
		g.set_warp_frame(&f, &w.gloss);
	});
}

/// The render camera while the warp mirror draws: retail's (stream space, no pad placement).
pub(in crate::viewer) fn camera(r: &crate::retail::Retail) -> Option<crate::retail::Cam> {
	MIRROR.with(|c| c.borrow().is_some()).then(|| r.warp_ram().and_then(|w| w.cam)).flatten()
}
