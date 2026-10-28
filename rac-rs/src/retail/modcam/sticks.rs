//! The right stick the game gets while the Modern camera is on (v0.4.1 lane cam, 2026-10-27; player reports: "the right stick looks
//! inverted again after a cutscene / in free look", "strafe aim and normal play turn opposite ways"). One convention for every camera:
//! with the Modern camera, stick up looks up and stick right looks right (like the mouse) whichever camera runs, modcam's own or
//! retail's (cutscene hand-back, slide, free look, the blend-out after them). Retail's cameras read the stick with retail's
//! sign (follow: up looks down, right turns left; free look: up looks down, right turns right), so the worker flips the controller's
//! share of the bytes before `set_pad` (`modcam_turn::camera_sticks`, the maths and tests) and modcam reads the same bytes.
//! The keyboard keys (Q/E Z/X) and the mouse keep their directions; with the Classic camera nothing changes (retail 1:1, also
//! the RAM hash of the replays); the Classic strafe aim that borrows modcam then turns the way retail's follow camera does.

use super::*;

/// Worker, field_begin, before `set_pad`: the pad's right stick bytes [rx, ry, lx, ly] as the game should get them. Unchanged unless
/// retail's camera runs in this game mode (`modcam_turn::camera_reads_stick`: 0 and 6; the pause map zooms on the same stick bytes),
/// the Modern camera is on (`st.active`) and a camera record is up; `keys` = `Input::keys`.
pub fn camera_sticks(m: &ee_oracle::Machine, sticks: [u8; 4], keys: [i8; 2]) -> [u8; 4] {
	if !enabled() {
		return sticks;
	}
	if !crate::retail::modcam_turn::camera_reads_stick(m.rd32(va(GAME_MODE))) {
		return sticks;
	}
	let Ok(st) = ST.lock() else { return sticks };
	let Some(lv) = lv_of(m.level()).filter(|_| st.active) else { return sticks };
	let rec = m.rd32(lv.cur()) & 0x01ff_ffff;
	if rec == 0 {
		return sticks;
	}
	let class4 = m.rd16(rec + 0x86) as i16 == FREE_LOOK_CLASS;
	let [rx, ry] = crate::retail::modcam_turn::camera_sticks([sticks[0], sticks[1]], keys, class4, m.rd8(OPT_LR) == 0);
	[rx, ry, sticks[2], sticks[3]]
}

/// Retail's free look camera class (L1 + L2 "look").
const FREE_LOOK_CLASS: i16 = 4;
