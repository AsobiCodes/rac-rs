//! The follow-pose store: cap, replace, remove and the auto-clear rules.

use super::*;

fn rec(item: u32, source: u32) -> FollowRec {
	FollowRec { item, source, from: 56, to: 76, mirror: true }
}

fn all_live(_: u32) -> Option<u16> {
	Some(7)
}

#[test]
fn a_record_per_item_replace_remove_and_the_cap() {
	let mut s = Store::new();
	assert!(s.set(rec(0x100, 0x200), 7, 7).is_ok());
	assert!(s.set(rec(0x100, 0x300), 7, 7).is_ok(), "same item = replace");
	assert_eq!((s.len(), s.records()[0].source), (1, 0x300));
	for i in 1..MAX_RECORDS as u32 {
		assert!(s.set(rec(0x1000 + i, 0x200), 7, 7).is_ok());
	}
	assert_eq!(s.len(), MAX_RECORDS);
	assert_eq!(s.set(rec(0x9000, 0x200), 7, 7), Err(ERR_CAP));
	assert!(s.set(rec(0x1001, 0x201), 7, 7).is_ok(), "replacing still works when full");
	assert!(s.remove(0x100) && !s.remove(0x100));
	assert_eq!(s.len(), MAX_RECORDS - 1);
}

#[test]
fn the_rules_clear_on_level_exit_scene_death_and_a_changed_hero() {
	let mk = || {
		let mut s = Store::new();
		s.run_rules(5, false, false, 0xaa00, false, all_live);
		s.set(rec(0x100, 0x200), 7, 7).unwrap();
		s
	};
	let mut s = mk();
	s.run_rules(5, false, false, 0xaa00, false, all_live);
	assert_eq!(s.len(), 1, "nothing changed: kept");
	for (name, lvl, exit, scene, hero, dead) in [("level", 6, false, false, 0xaa00, false), ("exit", 5, true, false, 0xaa00, false), ("scene", 5, false, true, 0xaa00, false), ("hero", 5, false, false, 0xbb00, false), ("death", 5, false, false, 0xaa00, true)] {
		let mut s = mk();
		s.run_rules(lvl, exit, scene, hero, dead, all_live);
		assert!(s.is_empty(), "{name} clears");
	}
}

#[test]
fn a_record_is_dropped_when_its_slot_holds_another_moby() {
	let mut s = Store::new();
	s.run_rules(1, false, false, 1, false, all_live);
	s.set(rec(0x100, 0x200), 7, 8).unwrap();
	s.run_rules(1, false, false, 1, false, |a| Some(if a == 0x100 { 7 } else { 8 }));
	assert_eq!(s.len(), 1);
	s.run_rules(1, false, false, 1, false, |a| Some(if a == 0x100 { 9 } else { 8 }));
	assert!(s.is_empty(), "item slot reused by another class");
	s.set(rec(0x100, 0x200), 7, 8).unwrap();
	s.run_rules(1, false, false, 1, false, |a| (a == 0x100).then_some(7));
	assert!(s.is_empty(), "source slot no longer live");
}

#[test]
fn the_snapshot_is_empty_in_scenes_and_without_a_posable_hero() {
	let _g = pose::TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
	clear_all();
	lock().set(rec(0x100, 0x200), 7, 7).unwrap();
	assert_eq!(snapshot_for(true, false).len(), 1);
	assert!(snapshot_for(true, true).is_empty() && snapshot_for(false, false).is_empty());
	clear_all();
	assert!(snapshot_for(true, false).is_empty());
}
