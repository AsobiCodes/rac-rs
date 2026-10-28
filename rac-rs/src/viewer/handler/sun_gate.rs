//! Which camera-culled mobys (`Prop::cast`) the sun shadow pass can take this frame (props phase, `frame_props.rs`). Posing a
//! moby costs CPU (joint matrices, skinning, the display-frame blend), and one the cascades never take shows nothing, so only the
//! ones a cascade can take are posed.

use super::*;

/// How far outside the cascade boxes (metres) a camera-culled moby is still posed, so that it has its pose of the tick before when
/// the camera or the light moves it into a box: the display-frame blend of its pose needs both ticks. Only the posing is done in the
/// margin; the blend, the draw and the shadow are for the ones the boxes take (`SunGate::takes` with margin 0).
pub(super) const HISTORY_MARGIN: f32 = 8.0;

/// Slack on the camera of the reach fallback (metres): the camera may move a little (shake, a follow offset) between the props phase
/// and the draw.
const CAM_SLACK: f32 = 25.0;

/// The sun shadow pass of this frame, as far as it is known before the draw.
pub(super) struct SunGate {
	cam: [f32; 3],
	q: sunshadow::Quality,
	tan_half: f32,
	aspect: f32,
	/// The cascades this frame's draw will fit, when the light holds still (`SunShadow::settled_cascades`): the boxes the maps cover,
	/// exactly. Else None: the reach of any cascade of any light direction (`sunshadow::cast_reach`).
	boxes: Option<[sunshadow::Cascade; sunshadow::CASCADES]>,
}

impl SunGate {
	/// The sun shadow pass of this frame, or None when it cannot run: the same gate as `Gpu::prepare` (`sunshadow::casts`: Enhanced
	/// lighting over a retail level drawn by rac-rs, Dynamic shadows on, no warp, no ship fight), from what is known before the draw
	/// (the warp and the ship fight are set in the ticks; `modern_ok` is the draw's own test, `frame_render.rs`; the split view's
	/// Classic half is not known yet, which only makes the gate wider).
	pub(super) fn of(app: &App, g: crate::retail::pcext::Graphics) -> Option<SunGate> {
		let modern_ok = app.retail.as_ref().is_some_and(|r| r.gs && r.front_ram().is_none() && r.gsfull_view().is_none());
		let gpu = app.gpu.as_ref()?;
		let warp = gpu.scene.as_ref().is_some_and(|s| s.warp_shown);
		let ship = app.retail.as_ref().is_some_and(|r| r.ship_fight());
		sunshadow::casts(g.enhanced() && modern_ok && !crate::viewer::pipe_guard::off(crate::viewer::pipe_guard::Effect::Post), g.shadows, warp, ship).then(|| {
			let (q, aspect) = (sunshadow::quality(g.shadows_q), gpu.config.width as f32 / gpu.config.height.max(1) as f32);
			let boxes = gpu.scene.as_ref().and_then(|s| gpu.sun.settled_cascades(&app.cam, aspect, &gpu.sun_rows(), s.world_verts.len() ^ (s.radius.to_bits() as usize) << 20, q));
			SunGate { cam: app.cam.pos, q, tan_half: app.cam.tan_half, aspect, boxes }
		})
	}

	/// Whether the shadow pass can take the camera-culled moby `p` (`class_sphere` = its class's bounding sphere: centre xyz, radius),
	/// `margin` metres outside the boxes: the sphere around its placement, widened by the class sphere with the largest scale, 5 m and a
	/// tick's blend (`smooth::SNAP_DIST`), touches a cascade box (`Cascade::touches`: what the maps clip to, so a moby outside every box
	/// is not in any map). Without boxes: within the reach of the cascades of any light.
	pub(super) fn takes(&self, p: &Prop, class_sphere: Option<[f32; 4]>, margin: f32) -> bool {
		let rho = class_sphere.map_or(f32::INFINITY, |s| (s[0].hypot(s[1]).hypot(s[2]) + s[3].abs()) * p.scale.abs().max(1.0) + 5.0);
		if !rho.is_finite() || !p.pos.iter().all(|v| v.is_finite()) {
			return true;
		}
		let rho = rho + smooth::SNAP_DIST + margin;
		match &self.boxes {
			Some(boxes) => boxes[..self.q.cascades].iter().any(|c| c.touches(p.pos, rho)),
			None => {
				let d = (0..3).map(|k| (p.pos[k] - self.cam[k]) * (p.pos[k] - self.cam[k])).sum::<f32>().sqrt();
				d <= sunshadow::cast_reach(self.q, self.tan_half * 1.1 + 0.02, self.aspect, rho) + CAM_SLACK
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The Enhanced sun test of `Gpu::prepare` and of the props phase is the one `sunshadow::casts`: its truth table.
	#[test]
	fn the_sun_gate_is_the_drawers() {
		for bits in 0..16u8 {
			let (post_on, shadows, warp, ship) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0);
			assert_eq!(sunshadow::casts(post_on, shadows, warp, ship), post_on && shadows && !warp && !ship);
		}
	}

	fn gate(boxes: bool) -> SunGate {
		let (cam, tan_half, aspect, q) = ([0.0f32, 0.0, 0.0], 0.6, 16.0 / 9.0, sunshadow::quality(3));
		let boxes = boxes.then(|| sunshadow::fit_all(&Camera { pos: cam, yaw: 0.0, pitch: 0.0, tan_half, up: [0.0, 0.0, 1.0] }, aspect, [0.3, 0.2, -0.93], q));
		SunGate { cam, q, tan_half, aspect, boxes }
	}

	fn moby_at(pos: [f32; 3]) -> Prop {
		let mut p = Prop::retail(1, 0);
		p.pos = pos;
		p
	}

	/// A moby inside a box is taken; one far off every box is not, and the margin widens the set.
	#[test]
	fn the_boxes_take_what_the_maps_cover() {
		let g = gate(true);
		let sphere = Some([0.0, 0.0, 0.0, 1.0]);
		assert!(g.takes(&moby_at([30.0, 0.0, 0.0]), sphere, 0.0), "in front of the camera");
		assert!(!g.takes(&moby_at([30.0, 4000.0, 0.0]), sphere, 0.0), "far to the side");
		assert!(g.takes(&moby_at([30.0, 0.0, 0.0]), None, 0.0), "no class sphere: always");
		// The edge of a box: out at margin 0, in at a big margin.
		let edge = (0..4000).map(|i| [30.0, i as f32, 0.0]).find(|p| !g.takes(&moby_at(*p), sphere, 0.0)).unwrap();
		assert!(g.takes(&moby_at(edge), sphere, HISTORY_MARGIN + 100.0));
		assert!(!g.takes(&moby_at(edge), sphere, 0.0), "out at margin 0");
		// A wider margin never drops a moby a narrower one takes.
		let taken: Vec<bool> = [0.0, 1.0, 4.0, HISTORY_MARGIN, 16.0, 64.0].iter().map(|m| g.takes(&moby_at(edge), sphere, *m)).collect();
		assert!(taken.windows(2).all(|w| !w[0] || w[1]), "margins are monotonic: {taken:?}");
		// The reach fallback takes the boxes' set and more.
		let r = gate(false);
		assert!(r.takes(&moby_at([30.0, 0.0, 0.0]), sphere, 0.0));
		assert!(!r.takes(&moby_at([0.0, 9000.0, 0.0]), sphere, 0.0));
	}
}
