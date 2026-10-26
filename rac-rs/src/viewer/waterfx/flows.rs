//! River and strip flows: the flow slot of each strip or river quad from the drift of its texture
//! (`WaterFx::strip_flows`, `quad_flows`).

use super::*;

/// River quad flows (`WaterFx::quad_flows`): directions and speed steps (world units per second) the quads share slots by.
/// How clear a river of effect quads is over its bed (flows z; under 0.5: no bed path): retail paints its light pattern
/// on the surface, which stays.
pub(super) const RIVER_CLEAR: f32 = 0.25;
const RIVER_DIRS: f32 = 12.0;
const RIVER_SPEED_STEP: f32 = 0.5;

/// A world strip's texture drift: the mean of its uv last frame and the smoothed uv step per game tick.
#[derive(Clone, Copy, Default)]
pub(super) struct Drift {
	mean: [f32; 2],
	step: [f32; 2],
	pub(super) seen: bool,
}

impl WaterFx {
	/// Before the level pass, on a level whose strips are water: the flow slot of each strip (0 = still), from the drift of
	/// its texture. Retail scrolls a river strip's uv a fixed step per game tick (FUN_002b9010, strips.rs `strip_scroll_step`);
	/// the step is measured on the strip's mean uv (frames where it does not move fall between two ticks, a jump past a third
	/// of the texture is the scroll's wrap) and turned into world units per second through the strip's uv-to-world map (a
	/// least-squares fit over its vertices). The ripples then drift the way the texture does, at its speed (only with
	/// `STRIP_FLOW`, off since 2026-10-20: the per-piece flows tore seams and jumped). The step also gives a waterfall's
	/// streak direction, so the drift is kept per strip identity, not per list index.
	pub(in crate::viewer) fn strip_flows(&mut self, strips: &[crate::particles::WorldStrip]) -> Vec<u32> {
		self.flows.truncate(1);
		self.phased = false;
		self.falls.clear();
		self.falls_fresh = true;
		let mut slots = Vec::with_capacity(strips.len());
		// [PC] A strip's drift is kept by what the strip is (its first vertex's x / y, its vertex count and its rank among strips
		// with the same ones: a record's two uv sets share positions), not by its place in this frame's list. The list changes
		// from frame to frame (a class not drawn this tick, strips from other callbacks), and by index a strip was compared with
		// another one's uv: garbage steps, rivers and waterfalls that turned and sped off (user 2026-10-20, Novalis).
		if self.drift.len() > 512 {
			self.drift.clear();
		}
		let mut rank: std::collections::HashMap<(u32, u32, u32), u32> = std::collections::HashMap::new();
		for s in strips {
			let at = s.pos.first().map_or([0; 2], |p| [p[0].to_bits(), p[1].to_bits()]);
			let r = rank.entry((at[0], at[1], s.pos.len() as u32)).or_insert(0);
			let d = self.drift.entry((at[0], at[1], s.pos.len() as u32, *r)).or_default();
			*r += 1;
			let n = s.uv.len().max(1) as f32;
			let mean = s.uv.iter().fold([0.0f32; 2], |a, u| [a[0] + u[0] / n, a[1] + u[1] / n]);
			let step = [mean[0] - d.mean[0], mean[1] - d.mean[1]];
			let size = step[0].abs().max(step[1].abs());
			if d.seen && size > 1e-6 && size < 0.3 {
				d.step = if d.step == [0.0; 2] { step } else { [d.step[0] * 0.8 + step[0] * 0.2, d.step[1] * 0.8 + step[1] * 0.2] };
			}
			d.mean = mean;
			d.seen = true;
			// [PC] Steep triangles are a waterfall (Novalis: the river's drop into the lower pool, z 60 -> 39): they are drawn
			// again with falling streaks (`fs_fall`) along the way retail scrolls them, and the foot of the steepest churns the
			// water (`Wakes`).
			let along_u = d.step[0].abs() > d.step[1].abs();
			let sign = if along_u { d.step[0] } else { d.step[1] }.signum();
			for k in 2..s.pos.len().min(s.uv.len()) {
				let t = [s.pos[k - 2], s.pos[k - 1], s.pos[k], s.pos[k]];
				let (a, b) = ([t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]], [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]]);
				let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
				let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
				if nl >= 1e-9 && n[2].abs() < FALL_STEEP * nl {
					if n[2].abs() < 0.5 * nl {
						push_foot(&mut self.falls, &t);
					}
					let uv = [s.uv[k - 2], s.uv[k - 1], s.uv[k], s.uv[k]];
					self.fall_verts.extend((0..4).map(|j| Vertex { pos: t[j], uv: uv[j], normal: [0.0, sign, along_u as u32 as f32], colour: [0.5, 0.5, 0.5, RIVER_FALL_ALPHA], light: [0.0; 4] }));
				}
			}
			// [PC] No ripple flow on strips (STRIP_FLOW): each piece got its own flow, so neighbouring pieces drifted their ripples
			// apart (seams), and a piece's flow jumped in direction and speed with the measured step.
			let flow = STRIP_FLOW.then(|| uv_map(&s.pos, &s.uv)).flatten().map(|inv| {
				// A texture feature at uv u sits at x = inv (u - c): it moves by -inv (uv step) per tick, 50 ticks a second.
				let v = [-(inv[0][0] * d.step[0] + inv[0][1] * d.step[1]) * 50.0, -(inv[1][0] * d.step[0] + inv[1][1] * d.step[1]) * 50.0];
				let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
				if l > 6.0 { [v[0] * 6.0 / l, v[1] * 6.0 / l] } else { v }
			});
			match flow.filter(|v| v[0].abs() + v[1].abs() > 1e-3 && self.flows.len() < LAST_FLOW as usize) {
				Some(v) => {
					slots.push(self.flows.len() as u32);
					self.flows.push(v);
				}
				None => slots.push(0),
			}
		}
		keep_lowest(&mut self.falls);
		slots
	}

	/// Before the level pass, on a level with a river of effect quads (`river_fx_tex`): the river quads grouped by flow slot
	/// (slot, indices into `quads`). The texture scrolls the same uv step per tick on every quad, so the step is the median uv
	/// change of the quads drawn at the same place last frame (no change: between two ticks; a change past 0.3 is the wrap),
	/// smoothed as `strip_flows` does. Each quad's flow is then that step through its own uv-to-world map, rounded to
	/// `RIVER_DIRS` directions and `RIVER_SPEED_STEP` speeds so the quads share a few slots. Steep quads (the waterfall, normal
	/// z under 0.5) are left out and keep their retail look.
	pub(in crate::viewer) fn quad_flows(&mut self, quads: &[([[f32; 3]; 4], [[f32; 2]; 4])]) -> Vec<(u32, Vec<usize>)> {
		self.flows.truncate(1);
		self.phased = true;
		self.falls.clear();
		self.falls_fresh = true;
		let key = |p: [f32; 3]| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
		let (mut du, mut dv) = (Vec::new(), Vec::new());
		for (pos, uv) in quads {
			if let Some(o) = self.river_uv.get(&key(pos[0])) {
				let d = [uv[0][0] - o[0], uv[0][1] - o[1]];
				if d[0].abs() < 0.3 && d[1].abs() < 0.3 {
					du.push(d[0]);
					dv.push(d[1]);
				}
			}
		}
		if !du.is_empty() {
			du.sort_by(f32::total_cmp);
			dv.sort_by(f32::total_cmp);
			let step = [du[du.len() / 2], dv[dv.len() / 2]];
			if step[0].abs().max(step[1].abs()) > 1e-6 {
				let d = self.river_step;
				self.river_step = if d == [0.0; 2] { step } else { [d[0] * 0.8 + step[0] * 0.2, d[1] * 0.8 + step[1] * 0.2] };
			}
		}
		self.river_uv.clear();
		self.river_uv.extend(quads.iter().map(|(pos, uv)| (key(pos[0]), uv[0])));
		let st = self.river_step;
		let mut keys: std::collections::BTreeMap<(i32, i32), u32> = std::collections::BTreeMap::new();
		let mut groups: Vec<(u32, Vec<usize>)> = Vec::new();
		for (i, (pos, uv)) in quads.iter().enumerate() {
			let (a, b) = ([pos[1][0] - pos[0][0], pos[1][1] - pos[0][1], pos[1][2] - pos[0][2]], [pos[3][0] - pos[0][0], pos[3][1] - pos[0][1], pos[3][2] - pos[0][2]]);
			let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
			let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
			if nl < 1e-9 || n[2].abs() < 0.5 * nl {
				// [PC] A steep quad's bottom edge is a waterfall's foot (`Wakes` churn), one per CHURN_CELL, and the quad is drawn
				// again with falling streaks (`fs_fall`) along the way retail scrolls it.
				if nl >= 1e-9 {
					push_foot(&mut self.falls, pos);
					let along_u = st[0].abs() > st[1].abs();
					let sign = if along_u { st[0] } else { st[1] }.signum();
					self.fall_verts.extend((0..4).map(|k| Vertex { pos: pos[k], uv: uv[k], normal: [0.0, sign, along_u as u32 as f32], colour: [0.5, 0.5, 0.5, RIVER_FALL_ALPHA], light: [0.0; 4] }));
				}
				continue;
			}
			let flow = uv_map(&pos[..], &uv[..]).map(|inv| {
				let v = [-(inv[0][0] * st[0] + inv[0][1] * st[1]) * 50.0, -(inv[1][0] * st[0] + inv[1][1] * st[1]) * 50.0];
				let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
				if l > 6.0 { [v[0] * 6.0 / l, v[1] * 6.0 / l] } else { v }
			});
			let slot = match flow.map(|v| (v[1].atan2(v[0]), (v[0] * v[0] + v[1] * v[1]).sqrt())).filter(|f| f.1 >= RIVER_SPEED_STEP * 0.5) {
				Some((ang, sp)) => {
					let k = ((ang / std::f32::consts::TAU * RIVER_DIRS).round() as i32).rem_euclid(RIVER_DIRS as i32);
					let q = (sp / RIVER_SPEED_STEP).round() as i32;
					match keys.get(&(k, q)) {
						Some(&s) => s,
						None if self.flows.len() < LAST_FLOW as usize => {
							let (ang, sp) = (k as f32 * std::f32::consts::TAU / RIVER_DIRS, q as f32 * RIVER_SPEED_STEP);
							let s = self.flows.len() as u32;
							self.flows.push([ang.cos() * sp, ang.sin() * sp]);
							keys.insert((k, q), s);
							s
						}
						None => 0,
					}
				}
				None => 0,
			};
			match groups.iter_mut().find(|g| g.0 == slot) {
				Some(g) => g.1.push(i),
				None => groups.push((slot, vec![i])),
			}
		}
		keep_lowest(&mut self.falls);
		groups
	}
}
