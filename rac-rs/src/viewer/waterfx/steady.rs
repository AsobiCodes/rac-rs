//! [PC] The sea's height, steadied (Split view: inside the water pass, `DrawFx.water_on`):retail's water mesh moves in 50 Hz steps (Pokitaru's pool bobs 0.01..0.03 a step and jitters
//! between steps), but the Enhanced surface (the geometric grid, the planar mirror) is cast onto that height every rendered
//! frame: at 100+ fps it stair-stepped, and the whole surface (every crest, the reflection) jumped with each step. `Steady`
//! follows the height with a short low-pass on the render clock, so the heave is smooth at any frame rate (render-only).

/// The low-pass time constant (seconds): long enough to swallow a 50 Hz step, short against a real tide.
const TAU: f32 = 0.12;

/// A height further than this from every known one starts a new track (another sea, a level change, a teleport).
const NEW_TRACK: f32 = 0.6;

/// The two height tracks: the geometric sea planes and the planar mirrors (each called once a frame, at the same clock).
#[derive(Default)]
pub(super) struct Heave {
	pub(super) geo: Steady,
	pub(super) mir: Steady,
	/// The shore map's tide-free height reference (`shore_ref.rs`).
	pub(super) shore: super::shore_ref::ShoreRef,
}

impl Heave {
	/// The geometric sea planes' heights (`[height, ..]` each) steadied in place: the grid is cast onto them every frame.
	pub(super) fn steady_planes(&mut self, now: f32, planes: &mut [[f32; 4]]) {
		let raw: Vec<f32> = planes.iter().map(|p| p[0]).collect();
		for (p, z) in planes.iter_mut().zip(self.geo.filter(now, &raw)) {
			p[0] = z;
		}
	}
}

/// The smoothed heights of the planes seen last call, and the clock of that call.
#[derive(Default)]
pub(super) struct Steady {
	prev: Vec<f32>,
	t: f32,
}

impl Steady {
	/// `zs` (this frame's raw heights) followed at render time `now`: each takes the nearest earlier track and moves towards it.
	pub(super) fn filter(&mut self, now: f32, zs: &[f32]) -> Vec<f32> {
		let dt = (now - self.t).clamp(0.0, 0.5);
		let a = 1.0 - (-dt / TAU).exp();
		let out: Vec<f32> = zs
			.iter()
			.map(|&z| {
				let near = self.prev.iter().copied().min_by(|p, q| (p - z).abs().total_cmp(&(q - z).abs()));
				match near {
					Some(p) if (p - z).abs() < NEW_TRACK => p + (z - p) * a,
					_ => z,
				}
			})
			.collect();
		// (Called again at the same time: dt 0 keeps the tracks as they were.)
		if dt > 0.0 || self.prev.is_empty() || now < self.t {
			self.prev = out.clone();
			self.t = now;
		}
		out
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A level that steps every 20 ms (retail) read at 130 fps comes out with frame-to-frame changes far below the step.
	#[test]
	fn steps_become_a_ramp() {
		let mut s = Steady::default();
		let (mut worst_raw, mut worst, mut last): (f32, f32, Option<(f32, f32)>) = (0.0, 0.0, None);
		for f in 0..400 {
			let now = f as f32 / 130.0;
			let raw = 223.0 + 0.02 * (now * 50.0).floor() * 0.5 + if (now * 50.0) as i32 % 2 == 0 { 0.012 } else { -0.012 };
			let out = s.filter(now, &[raw])[0];
			if let Some((lr, lo)) = last {
				worst_raw = worst_raw.max((raw - lr).abs());
				worst = worst.max((out - lo).abs());
			}
			last = Some((raw, out));
		}
		assert!(worst < worst_raw * 0.4, "{worst} vs {worst_raw}");
	}

	#[test]
	fn a_new_height_starts_a_new_track() {
		let mut s = Steady::default();
		s.filter(0.0, &[10.0]);
		assert_eq!(s.filter(0.1, &[50.0])[0], 50.0);
	}
}
