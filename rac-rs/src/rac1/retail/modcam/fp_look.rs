//! [PC] Mouse look in retail's first-person view (lane nfp, 2026-11-02; user: "also add mouse support to the first person mode").
//! First person is retail's free-look camera, class 4 (decomp/notes/camera.md 2b, CAM-7.1): L1 / L2 held on the ground (hero
//! state 1, key C) or the weapon-ready look (hero state 0x1e); the eye sits at the hero's head and the sticks / D-pad turn it
//! through two springs (1.5 deg a tick at most). The mouse reached the game only through the Modern camera's hand on the FOLLOW
//! class (modcam.rs `step`), which does not run in class 4, so it did nothing there.
//!
//! Here, before each field (worker `field_run.rs`, after `modcam::pre_field`): when the field brought mouse counts and the current
//! camera record is the free look in gameplay, its forward row is turned the way retail's class-4 update turns it for the stick
//! (Kerwan FUN_002e8590): yaw about the hero's up (moby +0xe0, unit), pitch about up x forward with retail's pitch word (class
//! state +0x10) stopped at retail's hard limit of 84 deg and held still under retail's own pitch gate (camera switch blend with
//! previous hero state 2); rows 1 / 2 rebuilt as the update rebuilds them. Retail's update then runs on that pose in the same field
//! (its stick springs, eye back-off, publish), so the pad and the keys add to the mouse as before. Gains and directions are the
//! Modern camera mouse's (`modcam_turn::turn`: 0.003 rad a count x RAC_MODCAM_SENS x Mouse speed row, Reversed per axis from the
//! Camera options); counts are whole per field (`take_ui` keeps the rest), so the turn does not depend on the frame rate and a
//! still mouse writes nothing: a run without mouse counts (every pad / keyboard replay) is bit-exact. With Camera Classic the
//! viewer grabs the mouse while first person is up (`grab_wanted`), as it does for the Classic strafe aim. RAC_FP_MOUSE=0: off.

use super::*;

/// Retail's hard pitch limit of the free look (f32 0x3fbba866, 84 deg; the stick's soft zone fades out there too).
const PITCH_MAX: f32 = 1.466_076_6;
/// The hero moby cell (boot data, the same on both discs) and the moby's up vector (retail's yaw axis in the free look).
const HERO_CELL: u32 = 0x1413d0;
const HERO_UP: u32 = 0xe0;
/// The previous hero state (state history; retail's stick pitch gate reads it).
const HERO_PREV_STATE: u32 = 0x1413e0;
/// Free-look record word +0x70: its class state; state +0x10: the pitch word (rad, + = looking down, 0 = level at entry).
const REC_STATE: u32 = 0x70;
const STATE_PITCH: u32 = 0x10;

/// First person was the camera after the last field (worker), for the viewer's mouse grab with Camera Classic.
static LIVE: AtomicBool = AtomicBool::new(false);

/// RAC_FP_MOUSE=0 turns the mouse look in first person off (read once).
fn switch() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_FP_MOUSE").map_or(true, |v| v.trim() != "0"))
}

/// The viewer grabs the mouse while first person is up (needed with Camera Classic; the Modern camera grabs it in gameplay anyway).
pub fn grab_wanted() -> bool {
	switch() && enabled() && LIVE.load(Ordering::Relaxed)
}

/// The current camera record and the level's row when it is retail's free look in gameplay.
fn free_look(m: &ee_oracle::Machine) -> Option<(u32, &'static Lv)> {
	if m.rd32(va(GAME_MODE)) != 0 {
		return None;
	}
	let lv = lv_of(m.level())?;
	let rec = m.rd32(lv.cur()) & 0x01ff_ffff;
	(rec != 0 && m.rd16(rec + 0x86) as i16 == sticks::FREE_LOOK_CLASS).then_some((rec, lv))
}

/// Worker, after each field (`field_gs.rs` `field_post`): whether first person is up, for `grab_wanted`.
pub fn post_field(m: &ee_oracle::Machine) {
	LIVE.store(switch() && enabled() && free_look(m).is_some(), Ordering::Relaxed);
}

/// [PC] Worker, after `post_field` (late_look.rs): the free look's pitch room (up, down; rad) before the 84 deg stop and whether
/// retail's own pitch gate holds the pitch still; None outside first person (or with the first-person mouse off).
pub(super) fn pitch_room(m: &ee_oracle::Machine) -> Option<(f32, f32, bool)> {
	if !(switch() && enabled()) {
		return None;
	}
	let (rec, lv) = free_look(m)?;
	let state = m.rd32(rec + REC_STATE) & 0x01ff_ffff;
	if state == 0 {
		return None;
	}
	// The pitch word is + = looking down.
	let p = f32::from_bits(m.rd32(state + STATE_PITCH));
	Some(((p + PITCH_MAX).max(0.0), (PITCH_MAX - p).max(0.0), m.rd32(lv.blend()) != 0 && m.rd32(HERO_PREV_STATE) == 2))
}

/// Worker, before each field: the field's mouse counts turn the free look's view (nothing without counts or outside it).
pub fn pre_field(m: &mut ee_oracle::Machine, fin: &crate::retail::FieldIn) {
	let look = fin.mc.look;
	if look == [0, 0] || !switch() || !enabled() {
		return;
	}
	let Some((rec, lv)) = free_look(m) else { return };
	let state = m.rd32(rec + REC_STATE) & 0x01ff_ffff;
	let hero = m.rd32(HERO_CELL) & 0x01ff_ffff;
	if state == 0 || hero == 0 {
		return;
	}
	let (lr, ud) = (if m.rd8(OPT_LR) == 0 { -1.0 } else { 1.0 }, if m.rd8(OPT_UD) == 0 { -1.0 } else { 1.0 });
	let g = crate::retail::modcam_turn::Gains { mouse: MOUSE, sens: sens(), mouse_k: crate::retail::pcext::mousespeed_scale(), rot: 0.0, tilt: 0.0, stick_k: 0.0, lr, ud };
	let (dyaw, dpitch) = crate::retail::modcam_turn::turn(look, [0.0; 2], &g);
	// Retail's own gate of the stick pitch (FUN_002e8590): a camera switch blend running and the previous hero state 2.
	let dpitch = if m.rd32(lv.blend()) != 0 && m.rd32(HERO_PREV_STATE) == 2 { 0.0 } else { dpitch };
	let rf = |a: u32| f32::from_bits(m.rd32(a));
	let (fwd, up, pitch) = ([rf(rec), rf(rec + 4), rf(rec + 8)], [rf(hero + HERO_UP), rf(hero + HERO_UP + 4), rf(hero + HERO_UP + 8)], rf(state + STATE_PITCH));
	let Some((rows, pitch)) = turn_view(fwd, up, pitch, dyaw, dpitch) else { return };
	for (i, row) in rows.iter().enumerate() {
		for (j, x) in row.iter().enumerate() {
			m.cpu.mem.wr32(rec + 0x10 * i as u32 + 4 * j as u32, x.to_bits());
		}
	}
	m.cpu.mem.wr32(state + STATE_PITCH, pitch.to_bits());
}

/// The free look's rows and pitch word after a mouse turn. `fwd` = the record's row 0, `up` = the hero's up (any length), `pitch` =
/// the pitch word; yaw about up (+ = left), then pitch about up x forward (+ = down; the word follows and stops at +-84 deg, a word
/// already past it is not pulled back). Rows 1 / 2 as retail's update rebuilds them: unit(up x fwd), then fwd x row 1. None for a
/// degenerate up / forward (nothing is written).
pub(crate) fn turn_view(fwd: [f32; 3], up: [f32; 3], pitch: f32, dyaw: f32, dpitch: f32) -> Option<([[f32; 3]; 3], f32)> {
	let u = unit(up)?;
	let f = rotate(unit(fwd)?, u, dyaw);
	let p = if dpitch > 0.0 { (pitch + dpitch).min(PITCH_MAX.max(pitch)) } else { (pitch + dpitch).max((-PITCH_MAX).min(pitch)) };
	let f = if p != pitch { unit(rotate(f, unit(cross3(u, f))?, p - pitch))? } else { unit(f)? };
	let left = unit(cross3(u, f))?;
	Some(([f, left, cross3(f, left)], p))
}

/// `v` turned about the unit axis `k` by `a` radians, right hand (Rodrigues; plain multiplies and adds, no fused ones).
fn rotate(v: [f32; 3], k: [f32; 3], a: f32) -> [f32; 3] {
	if a == 0.0 {
		return v;
	}
	let (s, c) = a.fsin_cos();
	let kv = cross3(k, v);
	let d = dot3(k, v) * (1.0 - c);
	[v[0] * c + kv[0] * s + k[0] * d, v[1] * c + kv[1] * s + k[1] * d, v[2] * c + kv[2] * s + k[2] * d]
}

fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
	let l = len3(v);
	(l.is_finite() && l > 1e-6).then(|| [v[0] / l, v[1] / l, v[2] / l])
}

#[cfg(test)]
#[path = "fp_look_tests.rs"]
mod tests;
