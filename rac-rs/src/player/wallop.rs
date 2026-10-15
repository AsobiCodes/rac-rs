//! WPN-6.6a: the Walloper attack, hero state 0x20 (group 6): the input gate (FUN_00240a40 case 0x12), the entry
//! (SetState case 0x20), the attack lock FUN_00234d38 with its registry pick FUN_0022dd00 / score FUN_0022dab8, the
//! motion (FUN_00236c20 case 0x20), the edge guard FUN_002365d0, the hero afterimage trail 0x1409c0
//! (FUN_00276f98 / FUN_00276fc0 / FUN_002770a0, BRIDGE-40 HERO-GHOST) and the exit (FUN_00242498 case 0x20)
//! (decomp/notes/weapons_framework.md "WPN-6.6a port spec"). The damage spheres run in `Game::wallop_hits`.
use super::*;
use crate::gameplay::pal_ticks;






/// One afterimage copy (FUN_00276fc0): its alpha byte, its lag in trail entries and where it was last placed (None
/// until the trail holds `lag` entries).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ghost {
	pub alpha: u8,
	pub lag: u8,
	pub at: Option<(V3, f32)>,
}

/// The hero afterimage trail record 0x1409c0 (FUN_00276f98): a ring of the last 8 hero positions and yaws (+0x00 /
/// +0x80), its head (+0x130) and fill count (+0x132, at most 8), and up to four copies (+0x100 / +0x110 / +0x120).
#[derive(Clone, Debug, PartialEq)]
pub struct Trail {
	ring: [(V3, f32); 8],
	head: u8,
	filled: u8,
	pub copies: Vec<Ghost>,
}


impl Player {
	/// The afterimage copies to draw (BRIDGE-40): world position, yaw and alpha byte of each placed copy.
	pub fn hero_ghosts(&self) -> [Option<(V3, f32, u8)>; 3] {
		let mut out = [None; 3];
		if let Some(t) = &self.wallop_trail {
			for (o, g) in out.iter_mut().zip(&t.copies) {
				*o = g.at.filter(|_| g.alpha != 0).map(|(p, y)| (p, y, g.alpha));
			}
		}
		out
	}








}

#[cfg(test)]
mod tests {
	use super::*;

	fn near(a: f32, b: f32) {
		assert!((a - b).abs() < 1e-6, "{a} != {b}");
	}

	fn flat_world() -> World {
		World::new([[[-50.0, -50.0, 0.0], [50.0, -50.0, 0.0], [50.0, 50.0, 0.0]], [[-50.0, -50.0, 0.0], [50.0, 50.0, 0.0], [-50.0, 50.0, 0.0]]], 4.0)
	}

	// WPN-6.6a test (1): the PAL tick scale of the windows.
	#[test]
	fn window_ticks() {
		assert_eq!((pal_ticks(4), pal_ticks(8), pal_ticks(10)), (3, 7, 8));
		assert_eq!((pal_ticks(10) + pal_ticks(8), pal_ticks(10) + pal_ticks(8) + pal_ticks(4)), (15, 18));
	}

	// WPN-6.6a test (2): the speed slew from rest at ticks 9 and 10.
	// WPN-6.6a test (8): the lock score, the crate bonus, the range / gap rejects and the tie rule.
	// Entry: state 0x20 group 6, the lock aims at the pick (no stick: the move heading stays the hero yaw), hero cue 0.
	// A stick past 0.5 sets the stick lock: the motion turns toward its yaw.
	// WPN-6.6a test (6): moving off a ledge (no ground under the probe) stops the hero; on ground it keeps moving.
	// WPN-6.6a test (7): the exit needs the anim past frame 20 (strict) with the blend done.
	// BRIDGE-40: the trail starts at tick 7 with three copies, places each at its lag, fades 5 a tick after tick 15 and
	// is freed by any state change.
	// WPN-6.6b3 test (1): the 0x13f544 countdown at the top of the hero update stops at 0.
	// WPN-6.6b3 tests (2) and (3): the Walloper passes the gate through mode 6 with `platform_ride` 0; a mode-0 hero
	// with the stick at 1.0 passes only while `platform_ride` != 0.
	// WPN-6.1e1 tests (1) and (6): a press in group 0 -> SetAnim(8, 56) only; the latch blocks a second call; the
	// release in state 0 on seq 56 blends to the idle (-2) and clears 0x1413fa.
	// WPN-6.1e1 test (2): crouch (group 0xc) -> the layer on seq 62; group 0 with the aim blocked -> seq 57.
	// WPN-6.1e1 test (3): Circle held (not pressed) latches only past scale(5) = 4 draw ticks (strict).
	// WPN-6.1e1 test (4): no ammo, Circle held -> the click unless its voice still plays; no input -> nothing.
	// WPN-6.1e1 test (5): group 3 (ledge hang) and group 7 (hit) refuse; no Blaster -> nothing.
}
