//! [PC] Fire: the GAME clock of the ground fire, the burnt grass, the Pyrocitor's jet memory (fix round, review m3 / m4) and of every
//! fire shader and the High lights (review R7-7).
//!
//! Their ages (a patch's flames and scorch, the burnt grass' char and regrow, the places a jet stood at) must not run on the wall clock:
//! in the pause menu, the Quick Select wheel or a travel the game stops while the wall goes on, and a fire would die behind a frozen
//! picture. The game's own time is the level frame counter of the effects pool (`PoolFx.frames`, +1 per ticking frame, +2 on a slow
//! one, standing in modes 3 / 4 and while the game is not ticking), but it only moves in steps (50 Hz) and the fire is drawn on every
//! display frame (frame smoothing: 144 Hz), so a clock made of the counter alone would stand still between two steps and jump on the
//! next: patches would stutter. So the clock is the WALL time (continuous, smooth on step and interpolated frames alike) that never
//! runs ahead of the last step the game made: `t` advances with the wall (a wall that steps back stands), is capped one field behind
//! `g` (the game time of the newest step) and is pulled up to `g - 2 windows` when the game runs faster than the wall. In a steady game
//! that is plain wall time (every step lifts the cap by about what the wall moved); in a pause `t` stops within two fields and stays;
//! in a slowdown it follows the steps. `update` runs where `set_particles` runs (every display frame). The DRAW clock `shown(wall)` is
//! a second wall-following clock moved at each draw: capped at `g` itself (so it has the field `t` lags by as room), never below
//! `t`, never backwards. It moves on every display frame, also on draws without an update of their own and when the drawing changes
//! from one frame per field to several (where `t` stands at its cap), and stands within one field of a pause. The fire shaders (flames,
//! fireballs, jets, embers, the ground fire's flames, the mesh flames: `Clock::game`, firefx_clock.rs) and the High lights' flicker
//! run on `shown`, the ages (patches, scorch, burnt grass, jet memory) on `now`, so a pause freezes the whole fire like retail's
//! frozen sprites and quads.
//!
//! The counter also tells when the world was swapped under the fire: a respawn (back to 0), a savestate load (back or a long jump) or a
//! gap with no pool (`frames` None) - `update` returns true once and the caller forgets what it kept (`FireFx::forget`).

/// The frames a counter may go forward in one display frame and still count as the game running on (slow frames, a stall); more is a jump.
const MAX_FORWARD: u32 = 90;
/// The frames it may go back and still count as the interpolated copy of the pool (frame smoothing shows the newest field's counter
/// on some frames and the older one on others); more is a respawn or a savestate load.
const MAX_BACK: u32 = 8;

/// The fire's game time in seconds (arbitrary zero; only differences count).
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct GameClock {
	/// The time handed out for ages (`now`).
	t: f64,
	/// The wall clock of the last `update`.
	wall: Option<f64>,
	/// The highest counter seen since the last gap or jump.
	top: Option<u32>,
	/// The game time of the newest step: `t` stays one `step` behind it, `shown` never passes it.
	g: f64,
	/// How far `t` may lag behind `g` (the seconds of the newest step, at most three fields).
	window: f64,
	/// One field's time (the last `update`'s `field`): how far `t` stays behind `g`.
	step: f64,
	/// The draw clock (`shown`) and its wall clock at the last draw (cells: the draws take `&self`).
	shown: std::cell::Cell<f64>,
	shown_wall: std::cell::Cell<Option<f64>>,
}

/// A wall clock read `wall` after the reference `last`: (the new reference, the seconds it moved, at most 0.25). A wall that steps back
/// a little stands (the test clock reads the old field's time on the frame between two fields, RAC_TEST_SUB: taking it as the new
/// reference would count the same stretch of wall twice and push the clock against its cap); a long way back is a clock that started
/// over: the new reference, nothing moved.
fn forward(last: Option<f64>, wall: f64) -> (f64, f64) {
	match last {
		Some(w) if wall < w && w - wall <= 0.25 => (w, 0.0),
		Some(w) => (wall, (wall - w).clamp(0.0, 0.25)),
		None => (wall, 0.0),
	}
}

impl GameClock {
	/// The fire clock of this frame. `wall`: the wall clock in seconds (any origin); `frames`: the pool's counter, None when there is no pool
	/// for this level; `field`: one field's time. True = the game was swapped under the fire (respawn, savestate, a gap with no pool):
	/// the caller forgets what it kept.
	pub(super) fn update(&mut self, wall: f64, frames: Option<u32>, field: f64) -> bool {
		let (wall, dt) = forward(self.wall, wall);
		self.wall = Some(wall);
		self.step = field;
		let Some(f) = frames else {
			// No pool: the game does not run for us; the time stands, and the gap is reported once.
			return self.top.take().is_some();
		};
		let mut jump = false;
		match self.top {
			None => self.restart(f, field),
			Some(top) if f > top => {
				if f - top <= MAX_FORWARD {
					let n = (f - top) as f64;
					self.g += n * field;
					self.window = n.min(3.0) * field;
					self.top = Some(f);
				} else {
					self.restart(f, field);
					jump = true;
				}
			}
			Some(top) if top - f <= MAX_BACK => {} // the older copy of the pool (smoothing), or a pause: hold
			Some(_) => {
				self.restart(f, field);
				jump = true;
			}
		}
		// One field behind the newest step: the room `shown` runs in.
		let hi = self.g - self.step;
		self.t = (self.t + dt).clamp((self.g - 2.0 * self.window).min(hi), hi);
		jump
	}

	/// The counter starts over at `f`: the time handed out so far is the game time one field before this step.
	fn restart(&mut self, f: u32, field: f64) {
		self.top = Some(f);
		self.g = self.t + field;
		self.window = field;
	}

	/// The fire is not active (`firefx::active()` false: Fire Off, a refused shader, Classic lighting, particles off, the split view's
	/// Classic half): the clock keeps its time and starts over at the next `update`.
	pub(super) fn rest(&mut self) {
		self.wall = None;
		self.top = None;
	}

	/// Seconds of game time, for ages (patches, scorch, burnt grass, jets): moves at each `update`, one field behind the newest step.
	pub(super) fn now(&self) -> f64 {
		self.t
	}

	/// The draw clock at wall time `wall` (the clock `update` gets, `firefx_clock::age`): moves with the wall since the last draw, never
	/// past the newest step, never below `now`, never back (a world swap that moved the step back is the one exception: then it is held
	/// at the new step). Smooth on every display frame of a steady game, frozen within one field of a pause.
	pub(super) fn shown(&self, wall: f64) -> f64 {
		let (w, dt) = forward(self.shown_wall.get(), wall);
		self.shown_wall.set(Some(w));
		let d = (self.shown.get() + dt).max(self.t).min(self.g);
		self.shown.set(d);
		d
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const FIELD: f64 = 0.02;

	/// A 144 Hz display over a 50 Hz game: (wall, counter) of each display frame for `secs` seconds, the counter rising on every 50 Hz step.
	fn frames(secs: f64, from: u32) -> Vec<(f64, u32)> {
		let mut out = Vec::new();
		let mut k = 0u32;
		while (k as f64) / 144.0 < secs {
			let wall = 10.0 + k as f64 / 144.0;
			out.push((wall, from + ((wall - 10.0) / FIELD).floor() as u32));
			k += 1;
		}
		out
	}

	#[test]
	fn a_steady_game_runs_smooth_on_step_and_interpolated_frames() {
		let mut c = GameClock::default();
		let mut last = None;
		let mut worst: f64 = 0.0;
		let mut moved = 0.0;
		let mut drawn = 0.0;
		for (wall, f) in frames(4.0, 100) {
			assert!(!c.update(wall, Some(f), FIELD));
			let t = c.now();
			let d = c.shown(wall);
			assert!(d >= t && d <= c.g && d >= drawn && d - drawn <= 1.0 / 144.0 + 1e-9, "draw clock {drawn} -> {d} (age {t})");
			drawn = d;
			if let Some(l) = last {
				assert!(t >= l, "never backwards: {l} -> {t}");
				worst = worst.max(t - l);
				moved += t - l;
			}
			last = Some(t);
		}
		assert!(worst <= 1.5 / 144.0 + 1e-9, "no frame advances more than 1.5 display frames: {worst}");
		assert!(moved > 3.8, "the clock keeps up with the wall: {moved} of 4 s");
	}

	#[test]
	fn a_pause_freezes_the_time_and_the_resume_goes_on_without_a_jump() {
		let mut c = GameClock::default();
		for (wall, f) in frames(1.0, 100) {
			c.update(wall, Some(f), FIELD);
		}
		let top = 100 + 50;
		let wall0 = 11.0;
		let mut before = c.now();
		// 3 s with the counter standing.
		for k in 0..432 {
			assert!(!c.update(wall0 + k as f64 / 144.0, Some(top), FIELD));
			if k == 10 {
				before = c.now(); // frozen within two fields of the pause
			}
		}
		let frozen = c.now();
		assert!(frozen - before < 1e-9, "stands in the pause: {before} -> {frozen}");
		// Resume: the first steps move on from the frozen time.
		let mut last = frozen;
		for k in 0..36 {
			c.update(wall0 + 3.0 + k as f64 / 144.0, Some(top + 1 + (k as u32 * 50 / 144)), FIELD);
			assert!(c.now() >= last && c.now() - last < 0.0105, "no jump: {} -> {}", last, c.now());
			last = c.now();
		}
		assert!(last > frozen, "and it runs again");
	}

	#[test]
	fn a_respawn_a_savestate_and_a_gap_are_reported_once() {
		let mut c = GameClock::default();
		for (wall, f) in frames(0.5, 500) {
			assert!(!c.update(wall, Some(f), FIELD));
		}
		// The counter starts over (death and restart).
		assert!(c.update(11.0, Some(0), FIELD), "back to 0");
		assert!(!c.update(11.007, Some(0), FIELD) && !c.update(11.014, Some(1), FIELD));
		// A savestate far ahead.
		assert!(c.update(11.5, Some(5000), FIELD), "forward 5000");
		assert!(!c.update(11.507, Some(5000), FIELD));
		// No pool for a while: reported once, at the first frame of the gap; then the new pool primes.
		assert!(c.update(12.0, None, FIELD));
		assert!(!c.update(12.007, None, FIELD) && !c.update(12.5, None, FIELD));
		assert!(!c.update(13.0, Some(77), FIELD), "the first call after a gap only primes");
		// A gap that was never primed reports nothing.
		let mut d = GameClock::default();
		assert!(!d.update(1.0, None, FIELD));
	}

	#[test]
	fn the_older_copy_of_the_pool_is_no_jump() {
		let mut c = GameClock::default();
		let mut wall = 10.0;
		c.update(wall, Some(100), FIELD);
		for k in 0..200u32 {
			wall += 1.0 / 144.0;
			let f = 100 + k / 3;
			// Smoothing: every other display frame shows the older field's counter.
			let shown = if k % 2 == 0 { f } else { f.saturating_sub(1).max(100) };
			assert!(!c.update(wall, Some(shown), FIELD), "frame {k}");
		}
		assert!(c.now() > 1.0, "time went on: {}", c.now());
	}

	#[test]
	fn a_slow_game_is_followed() {
		let mut c = GameClock::default();
		let mut wall = 10.0;
		let mut f = 100;
		c.update(wall, Some(f), FIELD);
		// One step (3 fields) every 60 ms for 3 s: the game runs at the wall's speed (50 Hz of 3-field steps = 1 s/s).
		let mut last = c.now();
		for k in 0..432 {
			wall += 1.0 / 144.0;
			if k % 9 == 8 {
				f += 3;
			}
			c.update(wall, Some(f), FIELD);
			assert!(c.now() >= last);
			last = c.now();
		}
		// (48 steps of 3 fields = 2.88 s of game time in 3 s of wall: the clock is at the newest step or up to two windows behind it.)
		assert!(last <= 2.88 + 1e-9 && last >= 2.88 - 0.13, "follows the steps: {last}");
		// A game at half speed (a field every 40 ms): t follows g, not the wall.
		let mut d = GameClock::default();
		let mut wall = 10.0;
		let mut f = 100;
		d.update(wall, Some(f), FIELD);
		for k in 0..432 {
			wall += 1.0 / 144.0;
			if k % 6 == 5 {
				f += 1;
			}
			d.update(wall, Some(f), FIELD);
		}
		assert!((d.now() - 1.44).abs() < 0.1, "half speed: {}", d.now());
	}

	/// One update per field (`set_particles` runs per uploaded field), `per` draws per field at even wall steps (RAC_TEST_SUB): the
	/// (wall, counter, update?) of each display frame, the counter rising by one at each field's first frame.
	fn fields(n: u32, per: u32, wall0: f64, from: u32) -> Vec<(f64, u32, bool)> {
		(0..n * per).map(|k| (wall0 + k as f64 * FIELD / per as f64, from + k / per, k % per == 0)).collect()
	}

	/// Draws the frames (updating where asked) and returns each frame's `shown`, checking it never goes back and never passes `g`.
	fn draw(c: &mut GameClock, frames: &[(f64, u32, bool)], mut last: f64) -> Vec<f64> {
		let mut out = Vec::new();
		for &(wall, f, up) in frames {
			if up {
				assert!(!c.update(wall, Some(f), FIELD));
			}
			let d = c.shown(wall);
			assert!(d >= last - 1e-12, "never backwards: {last} -> {d}");
			assert!(d <= c.g + 1e-12, "never past the newest step: {d} > {}", c.g);
			assert!(d >= c.now() - 1e-12, "never behind the age clock");
			out.push(d);
			last = d;
		}
		out
	}

	#[test]
	fn the_draw_clock_runs_evenly_between_per_field_updates() {
		let mut c = GameClock::default();
		let shown = draw(&mut c, &fields(100, 3, 10.0, 100), 0.0);
		// After the first field (the clock primes there), every display frame moves a third of a field.
		for (k, w) in shown.windows(2).enumerate().skip(3) {
			assert!((w[1] - w[0] - FIELD / 3.0).abs() < 1e-9, "frame {k}: {} -> {}", w[0], w[1]);
		}
		// 144 Hz over a 50 Hz game, an upload on the display frame after each step (two or three frames apart): as smooth as the wall.
		let mut d = GameClock::default();
		let ups: Vec<(f64, u32, bool)> = frames(4.0, 100).windows(2).map(|w| (w[1].0, w[1].1, w[1].1 != w[0].1)).collect();
		let shown = draw(&mut d, &ups, 0.0);
		let warm = 144 / 4;
		for (k, w) in shown.windows(2).enumerate().skip(warm) {
			assert!((w[1] - w[0] - 1.0 / 144.0).abs() < 1e-9, "frame {k}: {} -> {}", w[0], w[1]);
		}
		assert!(shown[shown.len() - 1] - shown[0] > 3.9, "keeps up with the wall: {}", shown[shown.len() - 1] - shown[0]);
	}

	/// A smoothing shot run (RAC_TEST_CLOCK, RAC_TEST_SUB=3, RAC_SHOT_STEP): per field the tick frame and a few waiting frames at the
	/// field's time, the three captured frames at +0, +1/3, +2/3 field, then one frame back at the field's time (the place wrapped before
	/// the next tick); `update` on every frame. Before the sequence the run draws one frame per field (`t` stands at its cap there).
	/// The captured frames' draw clock moves a third of a field each from the first one on, never once per field.
	#[test]
	fn a_wall_that_steps_back_adds_no_time() {
		let mut c = GameClock::default();
		let mut caps = Vec::new();
		for n in 0..60u32 {
			let a = 10.0 + n as f64 * FIELD;
			let mut run = vec![(a, n < 20)];
			if n >= 20 {
				run.extend(vec![(a, false); 5]);
				run.extend([(a, true), (a + FIELD / 3.0, true), (a + 2.0 * FIELD / 3.0, true), (a, false)]);
			}
			for (wall, captured) in run {
				assert!(!c.update(wall, Some(500 + n), FIELD));
				let d = c.shown(wall);
				if captured {
					caps.push(d);
				}
			}
		}
		for (k, w) in caps.windows(2).enumerate().skip(2) {
			let want = if k < 20 { FIELD } else { FIELD / 3.0 };
			assert!((w[1] - w[0] - want).abs() < 1e-9, "captured frame {k}: {} -> {}", w[0], w[1]);
		}
		// A clock that starts over far back is taken as the new reference: the time goes on from there.
		let t = c.now();
		c.update(2.0, Some(540), FIELD);
		c.update(2.0 + FIELD, Some(541), FIELD);
		assert!(c.now() > t, "runs on after the wall started over: {t} -> {}", c.now());
	}

	#[test]
	fn the_draw_clock_stands_in_a_pause_and_when_the_game_holds() {
		let mut c = GameClock::default();
		let run = fields(50, 3, 10.0, 100);
		let mut last = *draw(&mut c, &run, 0.0).last().unwrap();
		// The pause menu: updates go on with the counter standing; within one field the draw clock stands.
		let wall0 = 10.0 + 50.0 * FIELD;
		let paused: Vec<(f64, u32, bool)> = fields(100, 3, wall0, 0).into_iter().map(|(w, _, up)| (w, 149, up)).collect();
		let shown = draw(&mut c, &paused, last);
		assert!(shown[3..].iter().all(|&d| d == shown[3]), "frozen within one field: {:?}", &shown[..6]);
		last = *shown.last().unwrap();
		// The game held (no upload at all: a held test run, a stall): the wall goes on; the draw clock runs out the rest of its field
		// (at most one field after the last update), then stands at the newest step.
		let held0 = paused.last().unwrap().0;
		for k in 1..=300 {
			let d = c.shown(held0 + k as f64 / 144.0);
			assert!(d >= last && d - last <= 1.0 / 144.0 + 1e-9 && d <= c.g, "held frame {k}: {last} -> {d}");
			if k >= 3 {
				assert_eq!(d, c.g, "held frame {k}");
			}
			last = d;
		}
		// Resume: on from the frozen time, a third of a field per frame, no jump.
		let resume = fields(20, 3, held0 + 300.0 / 144.0 + FIELD / 3.0, 150);
		let shown = draw(&mut c, &resume, last);
		assert!(shown[0] - last <= FIELD / 3.0 + 1e-9 && shown[shown.len() - 1] > last + 0.3, "resumes: {last} -> {:?}", &shown[..4]);
		for w in shown.windows(2) {
			assert!(w[1] - w[0] <= FIELD / 3.0 + 1e-9, "no jump: {} -> {}", w[0], w[1]);
		}
	}

	#[test]
	fn resting_keeps_the_time_and_primes_again() {
		let mut c = GameClock::default();
		for (wall, f) in frames(1.0, 100) {
			c.update(wall, Some(f), FIELD);
		}
		let t = c.now();
		c.rest();
		assert_eq!(c.now(), t);
		assert!(!c.update(50.0, Some(9999), FIELD), "the first call after a rest primes, whatever the counter is");
		assert!((c.now() - t).abs() < 1e-9, "and the wall's gap is not added");
	}
}
