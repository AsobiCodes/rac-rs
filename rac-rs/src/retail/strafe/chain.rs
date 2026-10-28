//! Strafe flip chaining (part of strafe.rs's flips): Cross pressed shortly before a strafe flip lands starts the next strafe
//! flip on the touchdown field, like R&C2's. Retail answers that press itself there, without the jump chooser that strafe.rs
//! `jump` wraps (Eudora OV:40338-40352: an air state with the landing count set and a Cross press in the last scale(8) fields
//! goes to the running jump 9, or the plain jump 7, OV:39924-39937), so this hand wraps SetState: that jump becomes the next
//! strafe flip while RMB aims with the stick sideways or back. Everything else is the retail function.

use crate::fmath::Fm;
use super::*;

/// SetState(state, flag) of each level (`Row::set_state`, the chooser's last call). Hashes: rec::hash over its first 16
/// instructions in each overlay (the same words on both discs; the levels have 6 builds of it).
static HANDS: [Hand; 19] = [
	Hand { entry: 0x221fd8, end: 0x222018, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 0
	Hand { entry: 0x23cb00, end: 0x23cb40, hash: 0x7f2ff29258ce02e1, est: 0, f: set_state_any }, // 1
	Hand { entry: 0x22b290, end: 0x22b2d0, hash: 0x7f2ff29258ce02e1, est: 0, f: set_state_any }, // 2
	Hand { entry: 0x2161b0, end: 0x2161f0, hash: 0x1b32d75a1bbad501, est: 0, f: set_state_any }, // 3
	Hand { entry: 0x215538, end: 0x215578, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 4
	Hand { entry: 0x24cad0, end: 0x24cb10, hash: 0x8336466e8e718b01, est: 0, f: set_state_any }, // 5
	Hand { entry: 0x235200, end: 0x235240, hash: 0x6dc581b5e0d67be1, est: 0, f: set_state_any }, // 6
	Hand { entry: 0x249dc0, end: 0x249e00, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 7
	Hand { entry: 0x2306a0, end: 0x2306e0, hash: 0x1b32d75a1bbad501, est: 0, f: set_state_any }, // 8
	Hand { entry: 0x241c38, end: 0x241c78, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 9
	Hand { entry: 0x2155c0, end: 0x215600, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 10
	Hand { entry: 0x24d6b8, end: 0x24d6f8, hash: 0x7f2ff29258ce02e1, est: 0, f: set_state_any }, // 11
	Hand { entry: 0x23fc38, end: 0x23fc78, hash: 0x7f2ff29258ce02e1, est: 0, f: set_state_any }, // 12
	Hand { entry: 0x22da40, end: 0x22da80, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 13
	Hand { entry: 0x22fb40, end: 0x22fb80, hash: 0x1b32d75a1bbad501, est: 0, f: set_state_any }, // 14
	Hand { entry: 0x2167a0, end: 0x2167e0, hash: 0x7f2ff29258ce02e1, est: 0, f: set_state_any }, // 15
	Hand { entry: 0x21df00, end: 0x21df40, hash: 0xb81eea84b0f444c1, est: 0, f: set_state_any }, // 16
	Hand { entry: 0x21e090, end: 0x21e0d0, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 17
	Hand { entry: 0x227938, end: 0x227978, hash: 0x1b32d75a1bbad501, est: 0, f: set_state_any }, // 18
];

/// `HANDS` on the USA disc (`ROWS_USA`'s SetState).
static HANDS_USA: [Hand; 19] = [
	Hand { entry: 0x2223f8, end: 0x222438, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 0
	Hand { entry: 0x23cf98, end: 0x23cfd8, hash: 0x7f2ff29258ce02e1, est: 0, f: set_state_any }, // 1
	Hand { entry: 0x22b728, end: 0x22b768, hash: 0x7f2ff29258ce02e1, est: 0, f: set_state_any }, // 2
	Hand { entry: 0x216648, end: 0x216688, hash: 0x1b32d75a1bbad501, est: 0, f: set_state_any }, // 3
	Hand { entry: 0x2159d8, end: 0x215a18, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 4
	Hand { entry: 0x24cee8, end: 0x24cf28, hash: 0x8336466e8e718b01, est: 0, f: set_state_any }, // 5
	Hand { entry: 0x2356a0, end: 0x2356e0, hash: 0x6dc581b5e0d67be1, est: 0, f: set_state_any }, // 6
	Hand { entry: 0x24a260, end: 0x24a2a0, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 7
	Hand { entry: 0x230b38, end: 0x230b78, hash: 0x1b32d75a1bbad501, est: 0, f: set_state_any }, // 8
	Hand { entry: 0x2420d8, end: 0x242118, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 9
	Hand { entry: 0x215a60, end: 0x215aa0, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 10
	Hand { entry: 0x24db50, end: 0x24db90, hash: 0x7f2ff29258ce02e1, est: 0, f: set_state_any }, // 11
	Hand { entry: 0x2400d0, end: 0x240110, hash: 0x7f2ff29258ce02e1, est: 0, f: set_state_any }, // 12
	Hand { entry: 0x22dee0, end: 0x22df20, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 13
	Hand { entry: 0x22ff58, end: 0x22ff98, hash: 0x1b32d75a1bbad501, est: 0, f: set_state_any }, // 14
	Hand { entry: 0x216c38, end: 0x216c78, hash: 0x7f2ff29258ce02e1, est: 0, f: set_state_any }, // 15
	Hand { entry: 0x21e398, end: 0x21e3d8, hash: 0xb81eea84b0f444c1, est: 0, f: set_state_any }, // 16
	Hand { entry: 0x21e530, end: 0x21e570, hash: 0x6cc579b5df236e49, est: 0, f: set_state_any }, // 17
	Hand { entry: 0x227dd0, end: 0x227e10, hash: 0x1b32d75a1bbad501, est: 0, f: set_state_any }, // 18
];

/// The SetState hands of the disc that runs (strafe.rs `register`).
pub(super) fn hands() -> &'static [Hand] {
	crate::retail::disc::pick(&HANDS, &HANDS_USA)
}

fn set_state_any(c: &mut Cpu) -> bool {
	match (*crate::retail::disc::pick(&ROWS, &ROWS_USA)).iter().find(|r| r.set_state == c.pc) {
		Some(r) => land_jump(c, r),
		None => false,
	}
}

/// SetState(7 or 9) on a strafe flip's touchdown (state 0xb, landing count set): the next strafe flip instead, toward the
/// stick against the aim. The wish yaw is not the stick there (state 0xb sets it to the flip's carry heading every tick), so
/// the heading is the stick's own angle from the aim, as retail's wish is from the camera yaw (= the aim while aiming).
fn land_jump(c: &mut Cpu, r: &'static Row) -> bool {
	let to = c.g(4) as u32;
	if !matches!(to, 7 | 9) || c.mem.rd32(HERO_STATE) != 0xb || c.mem.rd32(FLIP_LANDED) == 0 || !enabled() {
		return false;
	}
	let Some(aim) = modcam::aim_yaw() else {
		return false;
	};
	if c.mem.rd8(BODY_FORM) != 0 || c.mem.rd32(va(GAME_MODE)) != 0 || !ST.lock().is_ok_and(|s| s.feel.flipping()) {
		return false;
	}
	let (x, y) = (f32::from_bits(c.mem.rd32(STICK_X)), f32::from_bits(c.mem.rd32(STICK_Y)));
	let d = stick_heading(x, y);
	let class = stick_class(d);
	if class == 0 || (x * x + y * y).sqrt() <= 0.5 || IN.swap(true, Ordering::SeqCst) {
		return false;
	}
	let _g = Guard;
	let wish = wrap(aim + d);
	c.mem.wr32(HERO_WISH, wish.to_bits());
	start_flip(c, r, wish, aim, d, class);
	if let Ok(mut s) = ST.lock() {
		let line = format!("flip_chain field {} jump {to} stick {x:.2} {y:.2} d {:.1} class {class}", s.field, d.to_degrees());
		log(&mut s, &line);
		s.flips += 1;
	}
	true
}

/// The stick's heading from the camera's forward (radians, positive = left): x right, y down, as the hero's wish yaw turns.
fn stick_heading(x: f32, y: f32) -> f32 {
	(-x).fatan2(-y)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn stick_heading_matches_the_wish() {
		// Measured wish - aim (jump_request lines): left +90, right -90, back 180, forward 0.
		let deg = |x: f32, y: f32| stick_heading(x, y).to_degrees();
		assert!((deg(-1.0, 0.0) - 90.0).abs() < 1e-4);
		assert!((deg(1.0, 0.0) + 90.0).abs() < 1e-4);
		assert!((deg(0.0, 1.0).abs() - 180.0).abs() < 1e-4);
		assert!(deg(0.0, -1.0).abs() < 1e-4);
		assert_eq!((stick_class(stick_heading(-1.0, 0.0)), stick_class(stick_heading(0.0, 1.0))), (1, 2));
	}

	#[test]
	fn pal_and_usa_rows_line_up() {
		for (i, (p, u)) in HANDS.iter().zip(HANDS_USA.iter()).enumerate() {
			assert_eq!((p.entry, p.hash), (ROWS[i].set_state, u.hash), "level {i}");
			assert_eq!(u.entry, ROWS_USA[i].set_state, "level {i}");
			assert_eq!((p.end - p.entry, u.end - u.entry), (64, 64), "level {i}");
		}
	}
}
