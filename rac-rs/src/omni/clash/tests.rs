use super::*;
use crate::omni::binding::Binding;

fn m(id: &str, hooks: &[&str]) -> ModUse {
	ModUse { id: id.to_string(), hooks: hooks.iter().map(|h| h.to_string()).collect(), ..ModUse::default() }
}

fn names(r: &[Replaced]) -> Vec<&'static str> {
	r.iter().map(|r| FEATURES[r.feature].id).collect()
}

const LOCK_STRAFE: [&str; 6] = [
	"rac1.pad.process_report",
	"rac1.player.update_motion_input",
	"rac1.player.turn_to_heading",
	"rac1.player.set_planar_velocity",
	"rac1.camera.follow.apply_overrides",
	"rac1.player.advance_animation",
];

#[test]
fn lock_strafe_replaces_the_camera_and_the_strafe_aim_only() {
	let r = decide(&[m("lock-strafe", &LOCK_STRAFE)]);
	assert_eq!(names(&r), ["rac-rs.modern-camera", "rac-rs.strafe-aim"]);
	assert_eq!(log_line(&r[0]), "omni: lock-strafe replaces rac-rs modern camera (hook rac1.camera.follow.apply_overrides)");
	assert_eq!(log_line(&r[1]), "omni: lock-strafe replaces rac-rs strafe aim (hook rac1.player.turn_to_heading decides the same word, hero facing 0x13F3E8)");
	// The wide lock-on writes a word no hook of lock-strafe decides; the keyboard table is untouched.
	assert!(!names(&r).contains(&"rac-rs.lockon") && !names(&r).contains(&"rac-rs.kbm"));
}

#[test]
fn no_mods_and_unrelated_mods_replace_nothing() {
	assert!(decide(&[]).is_empty());
	let r = decide(&[m("difficulty", &["rac1.moby.nanotech_ball.update", "rac1.player.take_damage"]), m("pad-only", &["rac1.pad.process_report", "rac1.player.advance_animation", "rac1.player.set_planar_velocity"])]);
	assert!(r.is_empty(), "{r:?}");
}

#[test]
fn a_camera_only_mod_takes_the_strafe_aim_with_it_because_that_needs_the_camera() {
	let r = decide(&[m("cam", &["rac1.camera.follow.apply_overrides"])]);
	assert_eq!(names(&r), ["rac-rs.modern-camera", "rac-rs.strafe-aim"]);
	assert_eq!(r[1].why, "it needs rac-rs.modern-camera, which was replaced");
}

#[test]
fn a_mod_that_decides_the_move_yaw_replaces_the_camera_by_the_shared_word() {
	let r = decide(&[m("motion", &["rac1.player.update_motion_input"])]);
	assert_eq!(names(&r)[0], "rac-rs.modern-camera");
	assert_eq!(r[0].why, "hook rac1.player.update_motion_input decides the same word, hero move yaw 0x13F4D0");
}

#[test]
fn conflicts_and_the_keyboard_name_the_features() {
	let mut a = m("a", &[]);
	a.conflicts = vec!["rac-rs.lockon".to_string(), "someone.else".to_string()];
	let mut b = m("b", &[]);
	b.keyboard = true;
	let r = decide(&[a, b]);
	assert_eq!(names(&r), ["rac-rs.lockon", "rac-rs.kbm"]);
	assert_eq!((r[0].by.as_str(), r[1].by.as_str()), ("a", "b"));
	assert_eq!(r[0].why, "its manifest conflicts names it");
}

#[test]
fn a_feature_is_listed_once_for_the_first_mod() {
	let r = decide(&[m("one", &["rac1.camera.follow.apply_overrides"]), m("two", &["rac1.camera.follow.apply_overrides"])]);
	assert_eq!(r.iter().filter(|r| r.feature == MODERN_CAMERA).count(), 1);
	assert_eq!(r[0].by, "one");
}

#[test]
fn the_tables_agree_with_the_binding_and_the_index_constants() {
	let b = Binding::parse(include_str!("../rac1.json")).unwrap();
	for (i, id) in [(MODERN_CAMERA, "rac-rs.modern-camera"), (STRAFE_AIM, "rac-rs.strafe-aim"), (LOCKON, "rac-rs.lockon"), (KBM, "rac-rs.kbm")] {
		assert_eq!(FEATURES[i].id, id);
	}
	for f in &FEATURES {
		assert!(f.hooks.iter().all(|h| b.hook(h).is_some()), "{}", f.id);
		assert!(f.needs.iter().all(|n| FEATURES.iter().any(|g| g.id == *n)), "{}", f.id);
	}
	assert!(HOOK_WRITES.iter().all(|(h, _)| b.hook(h).is_some()));
	assert!(FEATURES.iter().flat_map(|f| f.writes).chain(HOOK_WRITES.iter().flat_map(|(_, w)| w.iter())).all(|s| s.lo < s.hi));
	// The words hero.facing / hero.move_yaw of the binding (hero base 0x13F350 on the NTSC layout) are the spans the table names.
	assert_eq!(0x13F350 + 0x98, HERO_FACING.lo);
	assert_eq!(0x13F350 + 0x180, HERO_MOVE_YAW.lo);
}

#[test]
fn status_json_gets_a_replaces_list_per_mod() {
	let st = json::parse(r#"{"mods":[{"id":"lock-strafe","state":"loaded","hooks":["rac1.camera.follow.apply_overrides"]},{"id":"x","state":"refused"}],"api":"1.3"}"#).unwrap();
	let mods = loaded_mods(&st);
	assert_eq!(mods.len(), 1);
	assert_eq!(mods[0].hooks, ["rac1.camera.follow.apply_overrides"]);
	let r = decide(&mods);
	let out = json::parse(&status_with_replaces(st, &r)).unwrap();
	let list = |i: usize| -> Vec<String> { out.get("mods").unwrap().arr()[i].get("replaces").unwrap().arr().iter().map(|s| s.str().unwrap().to_string()).collect() };
	assert_eq!(list(0), ["rac-rs.modern-camera", "rac-rs.strafe-aim"]);
	assert!(list(1).is_empty());
	assert_eq!(out.get("api").and_then(Json::str), Some("1.3"));
}

#[test]
fn the_switch_bits_set_and_the_keyboard_follows_its_holder() {
	// The keyboard bit is shared with the input tests' publish(); the lock-on bit is touched by no other test (and stays set: nothing reads it).
	assert!(!off(LOCKON) && !off(STRAFE_AIM));
	set_off(LOCKON);
	assert!(off(LOCKON) && !off(MODERN_CAMERA) && !off(STRAFE_AIM));
	OFF.fetch_and(!(1 << LOCKON), Ordering::Relaxed);
	assert!(!off(LOCKON));
}
