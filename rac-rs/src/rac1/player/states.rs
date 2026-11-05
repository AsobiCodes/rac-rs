//! State history, the SetState refusal rules, the ground-ray reactions and the lava hurt state
//! (decomp/notes/hero_movement.md "State fields", hero_surfaces.md, hero_air_states.md).
use super::*;
use crate::gameplay::pal_ticks;

/// fead0(n): 50 Hz ticks scaled for the PAL clock, (int)(n * crate::gameplay::PAL_SCALE + 0.5).
fn fead(n: f32) -> f32 {
	crate::gameplay::pal_ticks(n as i32) as f32
}


/// The retail (state, group) of a port state. Ids marked [?] are not mapped in the notes and only need to be
/// distinct from the mapped ones.
pub(super) fn retail_state(s: &State) -> (u8, u8) {
	match *s {
		State::Ground { id: Gnd::Stand } => (0, 0),
		State::Ground { id: Gnd::Walk } => (2, 1),
		State::Ground { id: Gnd::Skid } => (3, 1),
		State::Look { fp: true, .. } => (0x1e, 0), // WPN-6.1e2
		State::Look { .. } => (1, 0),
		State::Crouch { .. } => (4, 0xc), // NOV-6.2
		State::Jump { kind, .. } => match kind {
			JumpKind::Stand => (7, 4),
			JumpKind::Run => (9, 4),
			JumpKind::High(_) => (0xb, 4),
			JumpKind::PullUp => (0x1c, 4),
			JumpKind::Double => (0xe, 4),
			JumpKind::Wall => (0x11, 4),
			JumpKind::Mud { .. } => (0x69, 0x19),
		},
		State::Fall { .. } => (6, 2),
		State::Swing => (0x2c, 0xe),     // ARI-47a
		State::SwingFall { .. } => (0x2d, 2), // ARI-47b
		State::Pull { kind, .. } => (
			match kind {
				PullKind::Aim => 0x24,
				PullKind::Reel => 0x25,
				PullKind::Orbit => 0x26,
			},
			0xd,
		), // ARI-46
		State::Grab { .. } => (0x18, 3),
		State::Hang { .. } => (0x19, 3),
		State::Shimmy { left, .. } => (if left { 0x1a } else { 0x1b }, 3),
		State::Tumble { .. } => (0x77, 2),
		State::Slip { .. } => (0x79, 2),
		State::Swim { mode, .. } => match mode {
			Swim::Under => (0x33, 0x11),
			Swim::Tread => (0x34, 0x11),
			Swim::Fast => (0x35, 0x11),
			Swim::Surface => (0x36, 0x12),
			Swim::Idle => (0x37, 0x12),
			Swim::Drown => (0x6a, 0x14),
		},
		State::Slide { .. } => (0x31, 0x10),
		State::Hurt { .. } => (0x3c, 4),
		State::Hit { .. } => (0x16, 7),
		State::SwimHit { top } => (if top { 0x75 } else { 0x76 }, 7),
		State::Wallop => (0x20, 6),
		State::Grind => (0x74, 0x1a), // KER-61a
		State::Remote { .. } => (0x1d, 9),
		State::Mud { death: false, .. } => (0x68, 0x19),
		State::Mud { death: true, .. } => (0x7b, 0x19),
	}
}

impl Player {
	/// The retail (state, group) ids of the current state.
	pub fn retail_ids(&self) -> (u8, u8) {
		retail_state(&self.state)
	}




























}

/// Hero state 0x22 landing-strike shake writer (decomp/notes/camera.md "NOV-5.5 port spec"). `descending`
/// is the 0x13f808 flag (set once the smoothed local vertical velocity goes negative after tick
/// fead0(0x21)); `land_count` is 0x13f814 (0 on entry). The first landed tick with the flag set writes shake
/// record A (amplitude 0.2, fead0(40) ticks); later landed ticks only count.
/// [P] the state 0x22 entry (air dispatcher), its movement body, velocity smoothing and the feet strike
/// spheres are not ported, so nothing calls this yet.
#[derive(Clone, Copy, Default, Debug)]
pub struct StrikeLanding {
	pub descending: bool,
	pub land_count: u32,
}

impl StrikeLanding {
	/// One state-0x22 tick: `tick` the state tick, `vz` the smoothed local vertical velocity, `airborne` the
	/// air counter != 0. Returns the shake write (amplitude, seconds) on the landing tick.
	pub fn tick(&mut self, tick: f32, vz: f32, airborne: bool) -> Option<(f32, f32)> {
		if tick > fead(33.0) && vz < 0.0 {
			self.descending = true;
		}
		if !self.descending || airborne {
			return None;
		}
		let shake = (self.land_count == 0).then(|| (0.2, crate::gameplay::pal_ticks(40) as f32 * 0.02));
		self.land_count += 1;
		shake
	}
}

#[cfg(test)]
mod strike_tests {
	use super::*;

	// NOV-5.5 tests: no descent -> no shake; landing with the flag -> one shake; airborne -> none.
	#[test]
	fn strike_landing_shake() {
		let mut s = StrikeLanding::default();
		assert_eq!(s.tick(5.0, -1.0, false), None);
		assert_eq!(s.tick(30.0, 1.0, false), None);
		let mut s = StrikeLanding::default();
		assert_eq!(s.tick(30.0, -1.0, true), None);
		assert!(s.descending);
		assert_eq!(s.tick(31.0, -1.0, true), None);
		let w = s.tick(32.0, 0.0, false).unwrap();
		assert_eq!(w.0, 0.2);
		assert!((w.1 - 0.66).abs() < 1e-6);
		assert_eq!(s.tick(33.0, 0.0, false), None);
		assert_eq!(s.land_count, 2);
	}
}
