//! Tests of engine/omni/events.rs (engine split stage E2, 2026-10-26): they run with R&C1's part of the mod host (rac1/omni_bind.rs: binding,
//! spec files, RAM words), so they live in the R&C1 pack; moved from the engine file's test modules, each test hands that part over first.
#![allow(unused_imports)]
mod tests {
	use crate::omni::events::*;
	use crate::omni::game;
	use crate::omni::lock;
	use crate::omni::native_host;
	use crate::omni::Omni;
	use ee_oracle::cpu::Cpu;
	use std::sync::atomic::AtomicBool;
	use std::sync::atomic::Ordering;
	use std::sync::Mutex;
	use crate::toc;

	fn snap(front: bool, level: i32, exit: bool, mode: u32, page: Option<u32>) -> Snapshot {
		Snapshot { front, level_word: level, exit, mode, page_id: page }
	}

	#[test]
	fn mode_names_follow_the_game_mode_word_and_the_front_end_flag() {
		crate::omni_bind::for_tests();
		let n = |m| mode_name(&snap(false, 1, false, m, None));
		assert_eq!((n(0), n(1), n(2), n(3), n(5), n(6)), ("gameplay", "movie", "cutscene", "pause", "vendor", "ship"));
		assert_eq!((n(4), n(7), n(9)), ("dialog", "screen", "unknown"));
		assert_eq!(mode_name(&snap(true, 1, false, 0, None)), "title");
		assert_eq!(mode_name(&snap(false, 1, true, 0, None)), "loading");
		assert_eq!(mode_name(&snap(false, 99, false, 0, None)), "loading");
		assert_eq!(mode_name(&snap(false, -1, false, 3, None)), "loading");
	}

	#[test]
	fn page_ids_map_to_the_api_names_else_page_hex() {
		crate::omni_bind::for_tests();
		assert_eq!(page_name(3), "weapons");
		assert_eq!(page_name(0x24), "skill_points");
		assert_eq!(page_name(0x1e), "save");
		assert_eq!(page_name(0x1f), "load");
		assert_eq!(page_name(0xd), "quit");
		assert_eq!(page_name(0x20), "page_20");
		assert_eq!(page_name(0x1d), "page_1d");
	}

	#[test]
	fn events_come_only_on_change_in_a_fixed_order() {
		crate::omni_bind::for_tests();
		let mut w = Watcher::default();
		// Title first: only the mode.
		assert_eq!(w.step(&snap(true, 0, false, 0, None)), vec![Event::Mode("title")]);
		assert!(w.step(&snap(true, 0, false, 0, None)).is_empty());
		// Into level 3: loaded, then mode.
		assert_eq!(w.step(&snap(false, 3, false, 0, None)), vec![Event::LevelLoaded(3), Event::Mode("gameplay")]);
		assert!(w.step(&snap(false, 3, false, 0, None)).is_empty());
		// Pause, weapons page, other page, close.
		assert_eq!(w.step(&snap(false, 3, false, 3, Some(2))), vec![Event::Mode("pause"), Event::PausePage("root".into())]);
		assert_eq!(w.step(&snap(false, 3, false, 3, Some(3))), vec![Event::PausePage("weapons".into())]);
		assert!(w.step(&snap(false, 3, false, 3, Some(3))).is_empty());
		assert_eq!(w.step(&snap(false, 3, false, 0, None)), vec![Event::Mode("gameplay"), Event::PausePage(String::new())]);
		// The level loop ends: unloading, loading mode.
		assert_eq!(w.step(&snap(false, 3, true, 0, None)), vec![Event::LevelUnloading(3), Event::Mode("loading")]);
		assert!(w.step(&snap(false, 3, true, 0, None)).is_empty());
		// Straight to another level without a field of exit: unloading then loaded.
		assert_eq!(w.step(&snap(false, 5, false, 0, None)), vec![Event::LevelLoaded(5), Event::Mode("gameplay")]);
		assert_eq!(w.step(&snap(false, 6, false, 0, None)), vec![Event::LevelUnloading(5), Event::LevelLoaded(6)]);
		// Quit to the title.
		assert_eq!(w.step(&snap(true, 6, false, 0, None)), vec![Event::LevelUnloading(6), Event::Mode("title")]);
	}

	#[test]
	fn a_page_id_outside_pause_is_not_reported() {
		crate::omni_bind::for_tests();
		let mut w = Watcher::default();
		let first = w.step(&snap(false, 1, false, 0, Some(2)));
		assert_eq!(first, vec![Event::LevelLoaded(1), Event::Mode("gameplay")]);
	}

	#[test]
	fn event_parts_are_the_omni_event_arguments() {
		crate::omni_bind::for_tests();
		assert_eq!(Event::LevelLoaded(4).parts(), ("level_loaded", 4, ""));
		assert_eq!(Event::LevelUnloading(4).parts(), ("level_unloading", 4, ""));
		assert_eq!(Event::Mode("pause").parts(), ("mode", 0, "pause"));
		assert_eq!(Event::PausePage("map".into()).parts(), ("pause_page", 0, "map"));
		assert_eq!(Event::NewGame(false).parts(), ("new_game", 0, ""));
		assert_eq!(Event::NewGame(true).parts(), ("new_game", 1, ""));
		assert_eq!(Event::Saving(2).parts(), ("saving", 2, ""));
		assert_eq!(Event::SaveLoaded(4).parts(), ("save_loaded", 4, ""));
	}

	fn put(ram: &mut [u8], a: u32, v: u32) {
		ram[a as usize..a as usize + 4].copy_from_slice(&v.to_le_bytes());
	}

	#[test]
	fn the_snapshot_reads_level_exit_mode_and_the_open_page_from_ram() {
		crate::omni_bind::for_tests();
		let mut ram = vec![0u8; 0x40_0000];
		put(&mut ram, crate::omni_bind::RAC1.level_word, 0);
		put(&mut ram, 0x15_F5A4, 3);
		// Level 0's current-page pointer (mouse_menu PAGE_PTR[0] = 0x1b9cf4) -> a page struct whose id word (+0x3c) is 6 (items).
		put(&mut ram, 0x1b_9cf4, 0x30_0000);
		put(&mut ram, 0x30_003c, 6);
		let s = snapshot(&ram, false);
		assert_eq!((s.level_word, s.exit, s.mode, s.page_id, s.front), (0, false, 3, Some(6), false));
		assert_eq!(mode_name(&s), "pause");
		assert_eq!(Watcher::default().step(&s), vec![Event::LevelLoaded(0), Event::Mode("pause"), Event::PausePage("items".into())]);
		// Exit word set: the level is leaving; outside mode 3 there is no page.
		put(&mut ram, 0x15_F550, 1);
		put(&mut ram, 0x15_F5A4, 0);
		let s = snapshot(&ram, false);
		assert_eq!((s.exit, s.page_id, level_of(&s)), (true, None, -1));
	}

	#[test]
	fn events_need_a_loaded_mod_and_a_dll_with_omni_event() {
		crate::omni_bind::for_tests();
		assert!(should_send(1, true));
		assert!(!should_send(0, true));
		assert!(!should_send(2, false));
		assert!(!should_send(0, false));
	}

	#[test]
	fn the_front_flag_is_shared_through_note_front() {
		crate::omni_bind::for_tests();
		note_front(true);
		assert!(FRONT.load(Ordering::Relaxed));
		note_front(false);
		assert!(!FRONT.load(Ordering::Relaxed));
	}
}
