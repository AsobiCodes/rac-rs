//! Tests of retail mode (the savestate probes are `#[ignore]`).

use super::*;

mod tests {
	use super::*;

	/// The skip rule on the keyboard, as the gameplay path and the front-end mirror both poll it (`movie_poll`): the logos
	/// never skip, the attract movie ends on any new key, the New Game intro and the story movies on a new Enter (Start) or
	/// on Esc / Space (the player's skip keys, which feed Start), and past Veldin Start alone does nothing: retail wants
	/// Start with C, Left Ctrl, V and B (L1 R1 L2 R2) held, which Esc / Space supply together.
	#[test]
	fn movie_skip_rule_keyboard() {
		use fmv_bridge::Skip;
		use winit::keyboard::KeyCode as K;
		let poll = |rule: Skip, prev: u16, keys: &[K]| movie_poll(rule, prev, |k| keys.contains(&k), 0, false);
		// Boot logos (mode -1): nothing skips.
		assert!(!poll(Skip::Never, 0, &[K::Escape, K::Enter, K::Space]).0);
		// Attract movie (mode 2): any new button, but not one already down at the last poll.
		assert!(poll(Skip::Any, 0, &[K::Space]).0);
		assert!(poll(Skip::Any, 0, &[K::KeyF]).0);
		assert!(!poll(Skip::Any, 0x40, &[K::Space]).0, "held since before");
		// Everything else (level word 0: New Game intro; Start rule): a new Esc / Enter / Numpad Enter / Space.
		for start in [K::Escape, K::Enter, K::NumpadEnter] {
			assert_eq!(poll(Skip::Start, 0, &[start]), (true, 0x800), "{start:?} = Start");
		}
		assert!(poll(Skip::Start, 0, &[K::Space]).0, "Space is a skip key");
		assert!(!poll(Skip::Start, 0, &[K::KeyF]).0, "other keys are no skip");
		assert!(!poll(Skip::Start, 0x800, &[K::Escape]).0, "Start held from before the movie");
		assert!(!poll(Skip::Start, 0x840, &[K::Space]).0, "Space held from before the movie");
		// A press polled twice skips once: the second poll sees it already down.
		let (hit, prev) = poll(Skip::Start, 0, &[K::Escape]);
		assert!(hit && !poll(Skip::Start, prev, &[K::Escape]).0);
		// Past Veldin in a normal run (combo rule): Start alone (Enter) does nothing, with all four shoulder keys it skips.
		let four = [K::KeyC, K::ControlLeft, K::KeyV, K::KeyB];
		assert_eq!(pad_from_keys(|k| four.contains(&k), false, false).0, 0xf, "L1 R1 L2 R2");
		assert!(!poll(Skip::Combo, 0, &[K::Enter]).0);
		for missing in 0..4 {
			let mut keys: Vec<K> = four.iter().enumerate().filter(|(i, _)| *i != missing).map(|(_, k)| *k).collect();
			keys.push(K::Enter);
			assert!(!poll(Skip::Combo, 0, &keys).0, "only three shoulder keys");
		}
		let mut keys = four.to_vec();
		keys.push(K::Enter);
		assert!(poll(Skip::Combo, 0, &keys).0);
		assert!(!poll(Skip::Combo, 0xf | 0x800, &keys).0, "Start held from before");
		// The shoulder keys pressed after Start do not skip, the Start press that completes the combo does.
		let (hit, prev) = poll(Skip::Combo, 0, &[K::Enter]);
		assert!(!hit && !poll(Skip::Combo, prev, &keys).0, "Start was already down when the shoulders came");
		// Esc / Space feed the whole combo, once per press.
		for k in [K::Escape, K::Space] {
			let (hit, prev) = poll(Skip::Combo, 0, &[k]);
			assert!(hit && !poll(Skip::Combo, prev, &[k]).0, "{k:?} skips a combo movie once");
			assert!(!poll(Skip::Combo, 0x80f, &[k]).0, "{k:?} held from before");
		}
	}

	/// Esc / Space during a scene (mode 2) press Start with the four shoulder buttons (retail's scene skip test), during a
	/// ship takeoff or landing (mode 6, substate 0 / 8) Cross (its skip test is a new Triangle or Cross); nothing elsewhere.
	#[test]
	fn skip_keys_feed_scene_and_flight() {
		assert_eq!(skip_feed(2, 0, true), 0x80f);
		assert_eq!(skip_feed(2, 0, false), 0);
		assert_eq!(skip_feed(6, 0, true), 0x40);
		assert_eq!(skip_feed(6, 8, true), 0x40);
		assert_eq!(skip_feed(6, 3, true), 0, "flight away: no skip");
		for mode in [0, 1, 3, 4, 5, 7] {
			assert_eq!(skip_feed(mode, 0, true), 0, "mode {mode}");
		}
	}

	/// GitHub #34 / #23: with the Start skips cutscenes row On a plain Start (Enter, or the controller's Start bit) ends a story movie
	/// whose retail rule wants the four shoulder buttons too; Off is retail's rule; a controller skips movies at all (it was never read).
	#[test]
	fn plain_start_skips_every_movie_when_the_row_is_on() {
		use fmv_bridge::Skip;
		use winit::keyboard::KeyCode as K;
		let none = |_: K| false;
		let enter = |k: K| k == K::Enter;
		// On: Enter ends a Combo movie once (a press held through the next poll skips nothing more); Off: nothing.
		let (hit, prev) = movie_poll(Skip::Combo, 0, enter, 0, true);
		assert!(hit && !movie_poll(Skip::Combo, prev, enter, 0, true).0, "a plain Start skips a combo movie once");
		assert!(!movie_poll(Skip::Combo, 0, enter, 0, false).0, "Off: retail's rule, Start alone does nothing");
		assert!(!movie_poll(Skip::Combo, 0x800, enter, 0, true).0, "Start held from before the movie skips nothing");
		// The controller's Start (0x800), alone and with a stick or button: the same; its shoulder buttons make the retail combo.
		assert!(movie_poll(Skip::Combo, 0, none, 0x800, true).0, "controller Start, row On");
		assert!(!movie_poll(Skip::Combo, 0, none, 0x800, false).0, "controller Start, row Off: the combo is missing");
		assert!(movie_poll(Skip::Combo, 0, none, 0x80f, false).0, "controller Start + L1 R1 L2 R2: retail's own combo");
		assert!(movie_poll(Skip::Start, 0, none, 0x800, false).0, "a Veldin / intro movie: controller Start, row Off");
		assert!(!movie_poll(Skip::Start, 0x800, none, 0x800, false).0, "controller Start held from before");
		assert!(movie_poll(Skip::Any, 0, none, 0x40, false).0, "the attract movie: any controller button");
		// The logos stay unskippable either way (the Skip intro row ends them).
		assert!(!movie_poll(Skip::Never, 0, enter, 0x800, true).0);
		// A button that is not Start feeds nothing, row On or not.
		assert!(!movie_poll(Skip::Combo, 0, none, 0x40, true).0 && !movie_poll(Skip::Combo, 0, |k| k == K::KeyF, 0, true).0);
		// Scenes and flights: `skip_feed` turns an already decided skip key into the retail buttons (`pcskip::start_pressed` decides it, row On
		// and Start in the pad bits: tested in pcskip.rs, it reads the row's global).
		assert_eq!(skip_feed(2, 0, true), 0x80f);
		assert_eq!(skip_feed(2, 0, false), 0);
	}

	/// Mouse in retail play: left = Circle (0x20), middle = Triangle (0x10), neither Square (0x80); the keyboard keeps F =
	/// Square, G = Circle, T = Triangle; a click latched by `press_mouse` outlives the release for `TAP_FIELDS` fields.
	#[test]
	fn mouse_buttons_are_circle_and_triangle() {
		use winit::keyboard::KeyCode as K;
		let pad = |k: Option<K>, c: bool, t: bool| pad_from_keys(|x| Some(x) == k, c, t).0;
		assert_eq!(pad(None, true, false), 0x20, "LMB = Circle");
		assert_eq!(pad(None, false, true), 0x10, "MMB = Triangle");
		assert_eq!(pad(None, true, true), 0x30);
		assert_eq!(pad(None, false, false), 0);
		assert_eq!(pad(Some(K::KeyF), false, false), 0x80, "F = Square");
		assert_eq!(pad(Some(K::KeyG), false, false), 0x20, "G = Circle");
		assert_eq!(pad(Some(K::KeyT), false, false), 0x10, "T = Triangle");
		assert_eq!(pad(Some(K::KeyG), true, false), 0x20);
		assert_eq!(pad(Some(K::Space), true, true), 0x70, "Space = Cross");
		// A click down and up between two samples: held for the next fields (tick_keys reads the latch, then ticks).
		let mut m = MouseTaps::default();
		m.press(0);
		m.press(1);
		for f in 0..TAP_FIELDS {
			assert!(m.0[0] > 0 && m.0[1] > 0, "tap lost at field {f}");
			let (b, _) = pad_from_keys(|_| false, m.0[0] > 0, m.0[1] > 0);
			assert_eq!(b, 0x30);
			m.tick();
		}
		assert_eq!(m.0, [0, 0]);
		m.press(2);
		assert_eq!(m.0, [0, 0], "no third button");
	}

	/// Stage I2 bisect (manual): the acceptance oracle runner's loop on RAC_TEST_STATE with the Retail::load extras
	/// switched on one by one (RAC_BIS = letters: f fmv_bridge::register, r record_sound, p set_pad
	/// neutral before the feed, c memcard Saver).
	#[test]
	#[ignore]
	fn retail_bisect() {
		let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/play/eudora1_0022").into());
		let n: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
		let bis = std::env::var("RAC_BIS").unwrap_or_default();
		let mut m = ee_oracle::Machine::load(&dir);
		if bis.contains('r') {
			m.record_sound();
		}
		ee_game::register();
		if bis.contains('f') {
			fmv_bridge::reset();
			fmv_bridge::register();
		}
		m.enable_rec();
		let feed = playfeed::open(Some(playfeed::MARK), false, &dir, 0, "ACC_ORACLE_NO_TRAIL", playfeed::code_hash(include_str!("../../../oracle/pcsx2rec/playfeed.rs"))).expect("feed");
		let mut card = memcard::Saver::new(&m);
		for f in 0..n {
			if bis.contains('p') {
				m.set_pad(0, [0x80; 4]);
			}
			feed.apply(&mut m.cpu, f);
			if let Err(e) = m.frame() {
				panic!("bisect {bis:?}: stop at field {f}: {e}");
			}
			if bis.contains('c') {
				card.after_field(&m);
			}
			let _ = take_sound_of(&mut m);
		}
		eprintln!("bisect {bis:?}: {n} fields ok");
	}

	/// Stage I2 (manual): from RAC_TEST_STATE (default fg1/4_0159) request travel to RAC_TEST_TRAVEL (default 5, Rilgar)
	/// the way the level-end helper does, service any movie (ended at once), wait for the new level's gameplay, run
	/// RAC_TEST_FIELDS (600) more fields and report the memory-card writes (RAC_PLAYER_DIR must be a scratch dir).
	#[test]
	#[ignore]
	fn retail_travel_save() {
		let pdir = std::env::var("RAC_PLAYER_DIR").expect("set RAC_PLAYER_DIR to a scratch dir");
		assert!(pdir.contains("scratch"), "RAC_PLAYER_DIR must be a scratch dir");
		let to: u32 = std::env::var("RAC_TEST_TRAVEL").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
		unsafe {
			std::env::set_var("RAC_RETAIL_HUD", "0");
			std::env::set_var("RAC_RETAIL_TRAVEL", format!("{to}:10"));
		}
		let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/fg1/4_0159").into());
		let after: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
		enable(&dir).unwrap();
		let mut r = Retail::load().unwrap();
		fmv_bridge::set_serviced(true);
		let (mut arrived, mut modes, mut movies) = (None, Vec::<(u32, u32, u32)>::new(), Vec::new());
		while !r.failed && r.fields < 6000 && arrived.is_none_or(|a| r.fields < a + after) {
			r.tick(0, [0x80; 4]);
			if let Some(q) = r.take_movie() {
				movies.push((r.fields, q));
				r.end_movie();
			}
			if modes.last().map(|x| (x.1, x.2)) != Some((r.snap.level, r.snap.mode)) {
				modes.push((r.fields, r.snap.level, r.snap.mode));
			}
			if let Some((l, pools)) = r.switch() {
				eprintln!("travel_save: field {} switch to level {l}, {} pools, hero {:?}", r.fields, pools.len(), r.snap.hero.map(|h| h.0));
				arrived = Some(r.fields);
			}
		}
		let card = std::path::Path::new(&pdir).join("card.bin");
		eprintln!("travel_save: fields {} (field, level, mode) changes {modes:?}; movies {movies:?}; stop {:?}; card {} {:?}", r.fields, r.stop, card.display(), std::fs::metadata(&card).map(|m| m.len()).ok());
		assert!(r.stop.is_none() && !r.failed, "oracle stopped: {:?}", r.stop);
		let a = arrived.expect("no travel");
		assert_eq!(r.snap.level, to);
		assert!(r.fields >= a + after);
	}

	/// Stage I2 (manual): a fresh `Retail::load` restores RAC_PLAYER_DIR/card.bin; prints the card-0 tree of the base
	/// image and of the restored card (files that differ are the player's saves) and runs RAC_TEST_FIELDS fields.
	#[test]
	#[ignore]
	fn retail_card_restore() {
		let pdir = std::env::var("RAC_PLAYER_DIR").expect("set RAC_PLAYER_DIR to a scratch dir");
		assert!(pdir.contains("scratch"), "RAC_PLAYER_DIR must be a scratch dir");
		unsafe { std::env::set_var("RAC_RETAIL_HUD", "0") };
		let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/fg1/4_0159").into());
		let bytes = std::fs::read(std::path::Path::new(&pdir).join("card.bin")).expect("no card.bin");
		fn walk(e: &ee_oracle::mc::Ent, path: &str, out: &mut Vec<(String, u32, [u8; 8], u64)>) {
			for k in &e.kids {
				let p = format!("{path}/{}", k.name);
				let h = k.data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100_0000_01b3));
				out.push((p.clone(), k.len, k.modified, h));
				walk(k, &p, out);
			}
		}
		let base = ee_oracle::Machine::load(&dir);
		let mut fresh = ee_oracle::Machine::load(&dir);
		assert!(memcard::restore_bytes(&mut fresh, &bytes));
		let (mut b, mut f) = (Vec::new(), Vec::new());
		// The state's card is read only at the first card call: compare with the base image itself then.
		let img = ee_oracle::mc::image_path(0).and_then(|f| std::fs::read(f).ok()).and_then(|i| ee_oracle::mc::Card::parse(&i).ok());
		if let Some(c) = base.cpu.mc.cards[0].as_ref().or(img.as_ref()) {
			walk(&c.root, "", &mut b);
		}
		if let Some(c) = &fresh.cpu.mc.cards[0] {
			walk(&c.root, "", &mut f);
		}
		for e in &f {
			let tag = match b.iter().find(|x| x.0 == e.0) {
				None => "NEW",
				Some(x) if x.3 != e.3 || x.2 != e.2 => "CHANGED",
				Some(_) => "same",
			};
			eprintln!("card_restore: {tag:7} {} len {} modified {:02x?} data {:016x}", e.0, e.1, e.2, e.3);
		}
		eprintln!("card_restore: base loaded {} files {}, restored loaded {} files {}", base.cpu.mc.loaded, b.len(), fresh.cpu.mc.loaded, f.len());
		drop((base, fresh));
		enable(&dir).unwrap();
		let mut r = Retail::load().unwrap();
		let n: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
		while r.fields < n && !r.failed {
			r.tick(0, [0x80; 4]);
		}
		eprintln!("card_restore: Retail ran {} fields after the restore, stop {:?}, level {} mode {}", r.fields, r.stop, r.snap.level, r.snap.mode);
		assert!(r.stop.is_none() && !r.failed);
	}

	/// Stage I2 (manual): from fg1/4_0159, keyboard keys through `tick_keys` (the viewer's path, no input injection):
	/// RAC_TEST_IDLE (960) + 30 idle fields, then W (stick forward) for 90, D for 60, Space (jump) taps; the hero must move.
	#[test]
	#[ignore]
	fn retail_keyboard_moves() {
		use winit::keyboard::KeyCode as K;
		unsafe {
			std::env::set_var("RAC_RETAIL_HUD", "0");
			std::env::set_var("RAC_PLAYER_CARD", "0");
		}
		let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/fg1/4_0159").into());
		enable(&dir).unwrap();
		let mut r = Retail::load().unwrap();
		// In this save the hero stands locked (hero state 114) until field ~897 in the PCSX2 recording too (fullgame1
		// ends.tsv: hstate 114 -> 0 at vb 253442 with only stick input before it); keys start after RAC_TEST_IDLE fields.
		let idle: u32 = std::env::var("RAC_TEST_IDLE").ok().and_then(|v| v.parse().ok()).unwrap_or(960);
		let mut track = Vec::new();
		for k in 0..240u32 + idle {
			let key = match k.saturating_sub(idle) {
				_ if k < idle => None,
				30..120 => Some(K::KeyW),
				120..180 => Some(K::KeyD),
				190..194 => Some(K::Space),
				_ => None,
			};
			r.tick_keys(|c| Some(c) == key, false, false);
			if k >= idle && (k - idle) % 30 == 29 {
				track.push((k + 1, r.snap.mode, r.snap.pad[0], r.snap.hero.map(|h| h.0)));
			}
		}
		for t in &track {
			eprintln!("keyboard: field {} mode {} game pad {:05x} hero {:?}", t.0, t.1, t.2, t.3);
		}
		assert!(r.stop.is_none() && !r.failed);
		let p = |i: usize| track[i].3.unwrap();
		let d = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
		eprintln!("keyboard: idle drift {:.2}, W moved {:.2}, D moved {:.2}", d(p(0), p(1)), d(p(0), p(3)), d(p(3), p(5)));
		assert!(d(p(0), p(3)) > 1.0, "W did not move the hero");
		assert!(d(p(3), p(5)) > 1.0, "D did not move the hero");
	}

	/// Manual (one game run): fg1/1_0018 (Novalis). Start opens the pause menu; the mouse hovers every root button through
	/// the layer (the game's own selection must follow), clicks into each page, hovers inside it, and right-clicks back.
	/// Everything goes through `mouse_*` + `tick_keys` (the viewer's path); the pages visited are printed for the handoff.
	#[test]
	#[ignore]
	fn retail_mouse_menus() {
		use super::mouse_menu::{Kind, MenuSnap, Target};
		use winit::keyboard::KeyCode as K;
		unsafe {
			std::env::set_var("RAC_RETAIL_HUD", "0");
			std::env::set_var("RAC_PLAYER_CARD", "0");
		}
		let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/fg1/1_0018").into());
		enable(&dir).unwrap();
		let mut r = Retail::load().unwrap();
		r.aspect = 16.0 / 9.0;
		let idle: u32 = std::env::var("RAC_TEST_IDLE").ok().and_then(|v| v.parse().ok()).unwrap_or(200);
		for _ in 0..idle {
			r.tick_keys(|_| false, false, false);
		}
		eprintln!("mouse: level {} mode {} after {idle} idle fields", r.snap.level, r.snap.mode);
		fn pump(r: &mut Retail, n: u32) {
			for _ in 0..n {
				r.tick_keys(|_| false, false, false);
			}
		}
		fn until(r: &mut Retail, max: u32, f: impl Fn(&MenuSnap) -> bool) -> bool {
			for _ in 0..max {
				r.tick_keys(|_| false, false, false);
				if r.snap.menu.as_ref().is_some_and(|m| !m.busy() && f(m)) {
					return true;
				}
			}
			false
		}
		// Window fractions of an overlay pixel (the inverse of `to_ps2` at 16:9).
		let k = r.aspect / (4.0 / 3.0);
		let frac = move |x: f32, y: f32| ((x / 512.0 - 0.5) / k + 0.5, y / 448.0);
		let centre = |it: &mouse_menu::Item, sub: u32| -> (f32, f32) {
			let [x, y, w, h] = it.rect.map(|v| v as f32);
			match &it.kind {
				Kind::Label { rows, .. } => (x + w * 0.5, y + rows.get(sub as usize).map_or(h * 0.5, |r| r.0)),
				Kind::Value { n, .. } => (x + w * 0.5, y + (sub as f32 + 1.0) * h / (*n as f32 + 1.0)),
				Kind::Grid { rows, cols, .. } => (x + ((sub % cols) as f32 + 0.5) * w / *cols as f32, y + ((sub / cols) as f32 + 0.5) * h / *rows as f32),
				Kind::Ring { .. } => mouse_menu::ring_slot(it.rect, sub),
				_ => (x + w * 0.5, y + h * 0.5),
			}
		};
		// Start (Enter) opens the pause menu.
		for _ in 0..4 {
			r.tick_keys(|k| k == K::Enter, false, false);
		}
		assert!(until(&mut r, 600, |m| m.id == 2), "pause menu did not open: mode {} menu {:?}", r.snap.mode, r.snap.menu);
		if let Some((w, rows, _, _)) = &r.snap.ov {
			eprintln!("mouse: overlay {w}x{rows}");
		}
		pump(&mut r, 30);
		let root = r.snap.menu.clone().unwrap();
		eprintln!("mouse: root page {:?}", root);
		assert!(root.items.len() >= 5, "root items: {}", root.items.len());
		let mut report = Vec::new();
		// Hover every root button: the game's selection must reach it.
		let mut items: Vec<_> = root.items.iter().filter(|i| matches!(i.kind, Kind::Label { .. })).cloned().collect();
		items.sort_by_key(|i| i.rect[1]);
		for it in &items {
			let (x, y) = centre(it, 0);
			let (fx, fy) = frac(x, y);
			r.mouse_cursor(fx, fy);
			let from = r.snap.menu.as_ref().map_or(0, |m| m.cur);
			let mut n = 0;
			let ok = loop {
				n += 1;
				r.tick_keys(|_| false, false, false);
				if r.snap.menu.as_ref().is_some_and(|m| !m.busy() && m.cur == it.addr) {
					break true;
				}
				if n >= 150 {
					break false;
				}
			};
			// Fields until the game's selection (as the snap reads it, 2 fields behind) is the hovered item.
			eprintln!("mouse: hover {:#x} at ({x:.0},{y:.0}) from {from:#x} -> cur {:#x} ok {ok} in {n} fields", it.addr, r.snap.menu.as_ref().map_or(0, |m| m.cur));
			assert!(ok, "hover {:#x} did not select it", it.addr);
		}
		// A long jump: from the last root button straight to the first (several rows in one hover), and back.
		for it in [items.first().unwrap(), items.last().unwrap()] {
			let (x, y) = centre(it, 0);
			let (fx, fy) = frac(x, y);
			r.mouse_cursor(fx, fy);
			let mut n = 0;
			while n < 200 && !r.snap.menu.as_ref().is_some_and(|m| !m.busy() && m.cur == it.addr) {
				r.tick_keys(|_| false, false, false);
				n += 1;
			}
			// The selection must stay there (no overshoot past the target).
			pump(&mut r, 10);
			let cur = r.snap.menu.as_ref().map_or(0, |m| m.cur);
			eprintln!("mouse: jump to {:#x}: {n} fields, {} rows, stays {}", it.addr, items.len() - 1, cur == it.addr);
			assert!(n < 200 && cur == it.addr, "jump to {:#x}: {n} fields, now {cur:#x}", it.addr);
		}
		// Click into every page, look around, right-click back.
		for it in &items {
			let (x, y) = centre(it, 0);
			let (fx, fy) = frac(x, y);
			r.mouse_cursor(fx, fy);
			r.mouse_click();
			let mut ok = false;
			for _ in 0..250 {
				r.tick_keys(|_| false, false, false);
				if r.snap.menu.as_ref().is_some_and(|m| m.id != 2) {
					ok = true;
					break;
				}
			}
			let pid = r.snap.menu.as_ref().map_or(0, |m| m.id);
			if !ok {
				report.push(format!("click {:#x}: no new page (still {pid:#x})", it.addr));
				continue;
			}
			pump(&mut r, 60);
			if r.snap.menu.as_ref().is_none_or(|m| m.busy()) {
				report.push(format!("click {:#x}: page {pid:#x} never went idle: {:?}", it.addr, r.snap.menu));
			}
			let pg = r.snap.menu.clone().unwrap();
			eprintln!("mouse: clicked {:#x} -> page {:#x}: {:?}", it.addr, pg.id, pg);
			// Hover each selectable target of the page (rows / cells / slots) and see whether the selection follows.
			let mut seen = 0;
			for sub_it in pg.items.iter().filter(|i| !matches!(i.kind, Kind::Hint(_) | Kind::Other)).cloned().collect::<Vec<_>>() {
				let subs: Vec<u32> = match &sub_it.kind {
					Kind::Label { rows, .. } => (0..rows.len() as u32).filter(|&i| rows[i as usize].1).collect(),
					Kind::Value { n, .. } => (0..*n).collect(),
					Kind::Grid { rows, cols, .. } => vec![0, rows * cols - 1],
					Kind::Ring { .. } => vec![3, 6],
					_ => vec![],
				};
				for sub in subs.into_iter().take(4) {
					let (x, y) = centre(&sub_it, sub);
					let (fx, fy) = frac(x, y);
					let t = r.snap.menu.as_ref().and_then(|m| m.hit(x, y));
					r.mouse_cursor(fx, fy);
					let want = Target::Sub { item: sub_it.addr, sub };
					let ok = until(&mut r, 150, |m| m.selected(want));
					seen += 1;
					if !ok || t != Some(want) {
						report.push(format!("page {:#x}: hover {:?} hit {:?} followed {ok}", pg.id, want, t));
					}
				}
			}
			eprintln!("mouse: page {:#x}: {seen} targets hovered", pg.id);
			// Right click = back to the root.
			r.mouse_back();
			let back = until(&mut r, 250, |m| m.id == 2);
			if !back {
				report.push(format!("page {:#x}: right click did not return to the root (now {:#x})", pg.id, r.snap.menu.as_ref().map_or(0, |m| m.id)));
				// try the keyboard so the next page can be tried
				for _ in 0..3 {
					r.tick_keys(|k| k == K::Enter, false, false);
				}
				until(&mut r, 250, |m| m.id == 2);
			}
			pump(&mut r, 20);
		}
		for l in &report {
			eprintln!("mouse REPORT: {l}");
		}
		assert!(r.stop.is_none() && !r.failed);
		assert!(report.is_empty(), "{} mouse problems (see REPORT lines)", report.len());
	}

	/// A field draws with the previous field's swap words; the first field with its own.
	#[test]
	fn swap_words_lag_one_field() {
		let w = |far: f32| SwapWords { fog_rgb: [0x72, 0xbe, 0xc8], fog: [50.0, far, 249.9, 63.75], part_far: 500.0, under: false, surf: None };
		let fx = |far: f32| RetailFx { swap: Some(w(far)), ..RetailFx::default() };
		let mut lag = None;
		let (mut a, mut b, mut c) = (fx(250.0), fx(128.0), fx(250.0));
		lag_swap(&mut a, &mut lag);
		lag_swap(&mut b, &mut lag);
		lag_swap(&mut c, &mut lag);
		assert_eq!((a.swap, b.swap, c.swap, lag), (Some(w(250.0)), Some(w(250.0)), Some(w(128.0)), Some(w(250.0))));
	}

	/// Manual check (needs the Eudora savestate): stick forward for 100 fields moves the hero.
	#[test]
	#[ignore]
	fn retail_stick_moves_hero() {
		enable(concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07")).unwrap();
		let mut r = Retail::load().unwrap();
		let p0 = r.snap.hero.unwrap().0;
		let (mut ms, mut worst) = (0.0, 0f32);
		for k in 0..600 {
			r.tick(if k == 100 { 0x40 } else { 0 }, [0x80, 0x80, 0x80, if k < 100 { 0 } else { 0x80 }]);
			ms += r.ms;
			worst = worst.max(r.ms);
		}
		r.tick(0, [0x80; 4]);
		let p1 = r.snap.hero.unwrap().0;
		eprintln!("hero {p0:?} -> {p1:?}, {:.1} ms/field (worst {worst:.1}), cam {:?}", ms / 600.0, r.cam);
		assert!(!r.failed);
		assert!((p1[0] - p0[0]).abs() + (p1[1] - p0[1]).abs() > 1.0);
	}

	/// Manual: Eudora -> Rilgar by the game's own travel sequence and loader; the new level's pools and camera come up.
	#[test]
	#[ignore]
	fn retail_travel() {
		enable(concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07")).unwrap();
		// SAFETY: the only test that sets it, before the interpreter thread starts.
		unsafe { std::env::set_var("RAC_RETAIL_TRAVEL", "5:10") };
		let mut r = Retail::load().unwrap();
		let mut got = None;
		for k in 0..2400 {
			r.tick(0, [0x80; 4]);
			if let Some(s) = r.switch() {
				eprintln!("field {k}: level {} pools {} classes, hero {:?}, cam {:?}", s.0, s.1.len(), r.snap.hero.map(|h| h.0), r.snap.cam);
				got = Some(s);
				break;
			}
		}
		assert!(!r.failed);
		let (l, pools) = got.expect("no travel");
		assert_eq!(l, 5);
		assert!(!pools.is_empty() && r.snap.hero.is_some() && r.snap.cam.is_some());
	}

	/// Manual (--wide across a level change): GS-full + wide from the Eudora state, travel to Rilgar, 200 more fields there; the
	/// wrapping hand must be used on both levels, VU1 errors stay 0, and every picture is 512x448 or 512x512.
	#[test]
	#[ignore]
	fn retail_wide_travel_probe() {
		enable(concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07")).unwrap();
		enable_gsfull();
		crate::gsfull::wide::enable();
		// SAFETY: the only test that sets it, before the interpreter thread starts.
		unsafe { std::env::set_var("RAC_RETAIL_TRAVEL", "5:10") };
		let mut r = Retail::load().unwrap();
		let gs = r.gsfull.clone().expect("GS-full threads");
		let (mut switched, mut after, mut shown, mut sizes) = (None, 0u32, 0u32, std::collections::BTreeSet::new());
		for k in 0..4000u32 {
			r.tick(0, [0x80; 4]);
			if let Some(f) = r.take_gsfull_frame() {
				shown += 1;
				sizes.insert((f.w, f.h));
			}
			if switched.is_none() {
				if let Some(s) = r.switch() {
					eprintln!("field {k}: switched to level {}, wide calls so far {:?}", s.0, crate::gsfull::wide::by_level());
					switched = Some(k);
				}
			} else {
				after += 1;
				if after >= 200 {
					break;
				}
			}
		}
		let by = crate::gsfull::wide::by_level();
		eprintln!("wide calls per level (eudora, rilgar) {by:?}; pictures {shown}, sizes {sizes:?}; {}", gs.report());
		assert!(!r.failed && switched.is_some());
		assert!(by[0] > 0 && by[1] > 0, "the hand must run on both levels");
	}

	/// Manual (needs the Eudora savestate): the 4:3 tests reproduce retail's `+0x31` on every moby of every field, and while the
	/// camera turns the mobys retail culled only at its 4:3 side plane that rac-rs's 16:9 window shows are all outside the 4:3
	/// band, inside the wide one, unhidden and within the draw distance.
	#[test]
	#[ignore]
	fn retail_wide_cull() {
		enable(concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07")).unwrap();
		let mut r = Retail::load().unwrap();
		let (w, h) = (1282.0f32, 720.0f32);
		let aspect = w / h;
		let (mut fields, mut checked, mut wrong) = (0, 0, Vec::new());
		let mut added: std::collections::BTreeMap<u32, (u32, u32, u32, u32)> = Default::default();
		let mut min_margin = f32::MAX;
		let (mut bad, mut centre_in_43, mut centre_out_wide) = (0, 0, 0);
		let mut shown_fields = 0;
			let mut caps: std::collections::BTreeSet<i32> = Default::default();
		let mut sample = Vec::new();
		let mut crates: Vec<String> = Vec::new();
		let script = |k: u32| -> u8 {
			match k {
				0..=149 => 0x80,
				150..=399 => 0xff,
				400..=649 => 0x00,
				650..=899 => 0xff,
				_ => 0x80,
			}
		};
		for k in 0..1000u32 {
			r.tick(0, [script(k), 0x80, 0x80, 0x80]);
			if k < 3 || r.snap.mobys.is_empty() {
				continue;
			}
			fields += 1;
			let mut any = false;
			for mb in &r.snap.mobys {
				let Some(c) = mb.culls else { continue };
				checked += 1;
				if mb.shown == c.any() {
					wrong.push((k, mb.addr, mb.class, mb.shown, c));
				}
				let Some(s) = mb.side_only else { continue };
				if !s.inside(aspect) {
					continue;
				}
				any = true;
				let tx43 = s.tan_v * 4.0 / 3.0;
				let txw = s.tan_v * aspect;
				let x43 = w / 2.0 + s.vx / (s.vz * tx43) * 480.0;
				let xw = w / 2.0 + s.vx / (s.vz * txw) * (w / 2.0);
				let e = added.entry(mb.addr).or_insert((0, k, 0, mb.class));
				e.0 += 1;
				e.2 = k;
				// The independent recheck: not hidden, inside the draw distance, outside the 4:3 band, nothing the rule skipped.
				let (flags, draw, cap) = mb.raw;
					caps.insert(cap);
				let dist_ok = (draw as i32).min(cap) << 10;
				if flags & 0x81 != 0 || s.vz - s.w > dist_ok as f32 || mb.shown || c.any() && !c.side_only() {
					bad += 1;
				}
				// Pixels the centre lies outside the 4:3 band (negative = inside it, only possible for a small far sphere: rac-rs's
				// 4:3 window is ty * 4/3 = 0.635 wide against retail's 0.63).
				min_margin = min_margin.min((x43 - w / 2.0).abs() - 480.0);
				if s.vx.abs() <= tx43 * s.vz {
					centre_in_43 += 1;
				}
				if xw < 0.0 || xw > w {
					centre_out_wide += 1;
				}
				if (500..=511).contains(&mb.class) && k % 10 == 0 && k < 340 {
					crates.push(format!("field {k} {:06x} class {} x {xw:.0} z {:.1} m", mb.addr, mb.class, s.vz / 1024.0));
				}
				if sample.len() < 12 && (sample.is_empty() || k % 25 == 0) {
					sample.push(format!("field {k} {:06x} class {} x43 {x43:.0} x16:9 {xw:.0} (4:3 band 161..1121) z {:.1} m r {:.2} m", mb.addr, mb.class, s.vz / 1024.0, s.w / 1024.0));
				}
			}
			if any {
				shown_fields += 1;
			}
		}
		eprintln!("fields {fields}, mobys checked {checked}, +0x31 mismatches {}", wrong.len());
		for x in wrong.iter().take(10) {
			eprintln!("  mismatch field {} {:06x} class {} shown {} culls {:?}", x.0, x.1, x.2, x.3, x.4);
		}
		eprintln!("fields where the rule adds a moby: {shown_fields}; distinct mobys {}; rule violations {bad}; centre inside 4:3 band {centre_in_43}; centre off the 16:9 window {centre_out_wide}", added.len());
		eprintln!("draw-distance caps seen with a moby added: {caps:?}");
		eprintln!("least pixels a centre lies outside the 4:3 band: {min_margin:.1}");
		for (a, (n, first, last, class)) in &added {
			eprintln!("  moby {a:06x} class {class}: added in {n} fields ({first}..{last})");
		}
		for l in &crates {
			eprintln!("  crate {l}");
		}
		for l in &sample {
			eprintln!("  {l}");
		}
		assert!(!r.failed);
		assert!(wrong.is_empty(), "the 4:3 tests disagree with +0x31");
		assert_eq!(bad, 0);
		assert!(!added.is_empty(), "the camera turn showed no wide-only moby");
	}

	/// Manual (input track): RAC_TEST_STATE (default s13, the pause menu) with RAC_TEST_PAD `from-to:buttons;...` (hex
	/// buttons, fields decimal; default d-pad Down at 10-14 and 25-29) for RAC_TEST_FIELDS fields through `Retail::tick`:
	/// one line per field with the game mode and the overlay hash (does the drawn menu follow the presses?).
	#[test]
	#[ignore]
	fn retail_pad_probe() {
		let state = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s13").into());
		let pad = std::env::var("RAC_TEST_PAD").unwrap_or_else(|_| "10-15:4000;25-30:4000".into());
		let fields: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
		// `from-to:buttons[:lx,ly[,rx,ry]]` (sticks hex, 80 centred).
		let script: Vec<(u32, u32, u16, Option<[u8; 4]>)> = pad
			.split(';')
			.filter_map(|e| {
				let mut f = e.split(':');
				let (r, b) = (f.next()?, f.next()?);
				let (a, z) = r.split_once('-')?;
				let st = f.next().and_then(|s| {
					let v: Vec<u8> = s.split(',').map(|x| u8::from_str_radix(x, 16).ok()).collect::<Option<_>>()?;
					Some([v[0], v[1], v.get(2).copied().unwrap_or(0x80), v.get(3).copied().unwrap_or(0x80)])
				});
				Some((a.parse().ok()?, z.parse().ok()?, u16::from_str_radix(b, 16).ok()?, st))
			})
			.collect();
		enable(&state).unwrap();
		let mut r = Retail::load().unwrap();
		let mut last = 0u64;
		for k in 0..fields {
			let b = script.iter().filter(|s| (s.0..s.1).contains(&k)).fold(0u16, |b, s| b | s.2);
			let st = script.iter().filter(|s| (s.0..s.1).contains(&k)).find_map(|s| s.3).unwrap_or([0x80; 4]);
			r.tick(b, [st[2], st[3], st[0], st[1]]);
			let ov = r.take_overlay().map(|(_, _, px, _)| px.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| (h ^ u32::from_le_bytes(*p) as u64).wrapping_mul(0x0100_0000_01b3)));
			let note = match ov {
				Some(h) if h != last => {
					last = h;
					format!("overlay {h:016x} CHANGED")
				}
				Some(h) => format!("overlay {h:016x}"),
				None => "no overlay".into(),
			};
			eprintln!("field {k} pad {b:04x} game held {:05x} new {:05x} raw {:05x} {:05x} mode {} equipped {} ammo {} last {} {:.1} ms cam yaw {:.1} pitch {:.1} hero {:?} {note}", r.snap.pad[0], r.snap.pad[1], r.snap.pad[2], r.snap.pad[3], r.snap.mode, r.snap.equipped, r.snap.ammo.get(r.snap.equipped as usize).copied().unwrap_or(0), r.snap.last, r.ms, r.cam.1.to_degrees(), r.cam.2.to_degrees(), r.snap.hero.map(|h| h.0));
		}
		assert!(!r.failed);
	}

	/// Manual (GS-full proof, run with RAC_GSFULL_LOG=1 [RAC_GSFULL_DUMP=DIR]): RAC_TEST_STATE (default s07) for RAC_TEST_FIELDS
	/// idle fields (default 20) through `Retail::tick` with the GS-full display on. The GS-full thread logs one line per field
	/// (FNV hash of the presented 512x448 RGBA picture, ms); this test waits for it to draw them all and prints the EE thread's
	/// and the GS-full thread's mean ms per field. <dev>/scratch_play_gsint_src (gsint) compares those pictures with the
	/// prototype's.
	#[test]
	#[ignore]
	fn retail_gsfull_probe() {
		let state = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07").into());
		let fields: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
		enable(&state).unwrap();
		enable_gsfull();
		let mut r = Retail::load().unwrap();
		let gs = r.gsfull.clone().expect("GS-full thread (needs GS.bin, vu1Memory.bin, vu1MicroMem.bin)");
		let (t0, mut ee, mut shown, mut first) = (std::time::Instant::now(), 0f32, 0u32, None);
		for _ in 0..fields + 1 {
			r.tick(0, [0x80; 4]);
			ee += r.ms;
			if let Some(f) = r.take_gsfull_frame() {
				shown += 1;
				first.get_or_insert((f.field, crate::gsfull::pipe::hash(&f.rgba)));
			}
		}
		let t_ee = t0.elapsed();
		let t = std::time::Instant::now();
		while gs.fields() < fields as u64 && !gs.dead.load(Ordering::SeqCst) && t.elapsed().as_secs() < 120 {
			std::thread::sleep(std::time::Duration::from_millis(20));
		}
		eprintln!("retail_gsfull: {fields} fields, EE thread {:.2} ms/field (ticks took {:.2} ms/field wall), GS-full: {}, {} pictures taken by the main thread, first {first:x?}", ee / (fields + 1) as f32, t_ee.as_secs_f64() * 1000.0 / (fields + 1) as f64, gs.report(), shown);
		assert!(!r.failed && !gs.dead.load(Ordering::SeqCst));
		assert!(gs.fields() >= fields as u64, "GS-full thread drew only {} fields", gs.fields());
	}

	/// Manual (--wide evidence): the same state and pad script run plain and with the wide-FOV hands (gsfull/wide.rs); every
	/// 50 fields the hero, the moby table and the RAM are compared. RAC_TEST_STATE (default s07), RAC_TEST_FIELDS (default 300).
	#[test]
	#[ignore]
	fn retail_wide_gameplay_probe() {
		let state = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07").into());
		let n: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
		ee_game::register();
		crate::gsfull::wide::register();
		let mut runs: Vec<Vec<(u32, Snap, Vec<u8>)>> = vec![];
		for wide in [false, true] {
			crate::gsfull::wide::set_active(wide);
			let mut m = ee_oracle::Machine::load(&state);
			m.enable_rec();
			let mut cps = vec![];
			for k in 0..n {
				// Idle for 20 fields, then the left stick forward, with a jump (Cross) every 90 fields.
				let (b, st) = if k < 20 { (0, [0x80u8; 4]) } else { (if k % 90 < 6 { 0x40 } else { 0 }, [0x80, 0x80, 0x80, 0x00]) };
				m.set_pad(b, st);
				m.frame().unwrap();
				if (k + 1) % 50 == 0 || k + 1 == n {
					cps.push((k + 1, snap(&m), m.cpu.mem.ram.to_vec()));
				}
			}
			runs.push(cps);
		}
		crate::gsfull::wide::set_active(true);
		let (calls, rewide, seen) = crate::gsfull::wide::stats();
		eprintln!("wide hand: {calls} calls, {rewide} with an already-wide input, game inputs seen {:x?}", seen);
		for (a, b) in runs[0].iter().zip(&runs[1]) {
			let (sa, sb) = (&a.1, &b.1);
			let ha = sa.hero.as_ref().map(|h| h.0);
			let hb = sb.hero.as_ref().map(|h| h.0);
			let bm: std::collections::HashMap<u32, &Moby> = sb.mobys.iter().map(|m| (m.addr, m)).collect();
			let (mut moved, mut shown, mut gone) = (0, 0, 0);
			for m in &sa.mobys {
				match bm.get(&m.addr) {
					Some(o) => {
						moved += (m.pos != o.pos || m.class != o.class) as u32;
						shown += (m.shown != o.shown) as u32;
					}
					None => gone += 1,
				}
			}
			let words = a.2.chunks_exact(4).zip(b.2.chunks_exact(4)).filter(|(x, y)| x != y).count();
			eprintln!(
				"field {}: hero {:?} vs {:?} same={}; health {}/{} bolts {}/{}; mobys {}/{}: moved {moved}, shown flag differs {shown}, only in plain {gone}; RAM words differing {words}",
				a.0, ha, hb, ha == hb, sa.health, sb.health, sa.bolts, sb.bolts, sa.mobys.len(), sb.mobys.len()
			);
		}
		// The mobys that differ at the last checkpoint: class, positions, the game's shown flag (+0x31) in both runs.
		{
			let (sa, sb) = (&runs[0].last().unwrap().1, &runs[1].last().unwrap().1);
			let bm: std::collections::HashMap<u32, &Moby> = sb.mobys.iter().map(|m| (m.addr, m)).collect();
			let mut by_class: std::collections::BTreeMap<u32, (u32, u32, u32)> = Default::default();
			let mut rows = vec![];
			for m in &sa.mobys {
				let e = by_class.entry(m.class).or_default();
				e.0 += 1;
				if let Some(o) = bm.get(&m.addr) {
					let mv = m.pos != o.pos || m.class != o.class;
					e.1 += mv as u32;
					e.2 += (m.shown != o.shown) as u32;
					if mv && rows.len() < 14 {
						let d = ((m.pos[0] - o.pos[0]).powi(2) + (m.pos[1] - o.pos[1]).powi(2) + (m.pos[2] - o.pos[2]).powi(2)).sqrt();
						rows.push(format!("class {} @{:x} plain {:?} wide {:?} dist {d:.2} shown {}/{}", m.class, m.addr, m.pos, o.pos, m.shown, o.shown));
					}
				}
			}
			eprintln!("per class (count, moved, shown differs), classes with a difference: {:?}", by_class.iter().filter(|(_, v)| v.1 + v.2 > 0).collect::<Vec<_>>());
			for r in rows {
				eprintln!("  {r}");
			}
		}
		// Where the RAM differs at the last checkpoint (1 MB buckets).
		let (x, y) = (&runs[0].last().unwrap().2, &runs[1].last().unwrap().2);
		let mut bucket = std::collections::BTreeMap::new();
		for (i, (p, q)) in x.chunks_exact(4).zip(y.chunks_exact(4)).enumerate() {
			if p != q {
				*bucket.entry(i * 4 / 0x1000).or_insert(0u32) += 1;
			}
		}
		eprintln!("differing 4 KB pages (page address: words): {:x?}", bucket.iter().map(|(k, v)| (k * 0x1000, *v)).collect::<Vec<_>>());
	}

	/// Manual (EE speed): `Machine::frame` ms per field of RAC_TEST_STATE (default s07) over RAC_TEST_FIELDS fields (default 40) with
	/// the pieces of `Retail::load` switched on one by one by RAC_TEST_EE=snd,sound,gsq (comma list).
	#[test]
	#[ignore]
	fn retail_ee_probe() {
		let state = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07").into());
		let fields: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
		let opts = std::env::var("RAC_TEST_EE").unwrap_or_default();
		let has = |k: &str| opts.split(',').any(|o| o == k);
		if has("regfirst") {
			ee_game::register();
		}
		let mut m = ee_oracle::Machine::load(&state);
		if has("sound") {
			m.record_sound();
		}
		if !has("regfirst") {
			ee_game::register();
		}
		let nrec = m.enable_rec();
		eprintln!("retail_ee: {nrec} recompiled functions");
		let (tx, rx) = std::sync::mpsc::channel::<ee_oracle::gs::Job>();
		if has("gsq") {
			m.cpu.mem.gsq = Some(tx);
		}
		let worker = has("thread");
		let mut run = move || {
			let mut ms = Vec::new();
			for _ in 0..fields {
				m.set_pad(0, [0x80; 4]);
				let t = std::time::Instant::now();
				m.frame().unwrap();
				ms.push(t.elapsed().as_secs_f64() * 1000.0);
				let _ = rx.try_iter().count();
			}
			ms
		};
		let ms = if worker { std::thread::Builder::new().stack_size(256 << 20).spawn(run).unwrap().join().unwrap() } else { run() };
		let skip = 3.min(ms.len());
		eprintln!("retail_ee [{opts}]: {:.2} ms/field (first {skip} skipped), min {:.2}", ms[skip..].iter().sum::<f64>() / (ms.len() - skip) as f64, ms[skip..].iter().cloned().fold(f64::MAX, f64::min));
	}

	/// The movie hook dispatches from the oracle (stage I1). eudora1 window 0041's recorded pad (RAC_RETAIL_PLAY=@play)
	/// enters an in-game movie after field 1133 (the oracle used to spin there, pc 0x25939c, game mode 1): through the
	/// real Retail path the retail player call reaches fmv_bridge's hook, `tick` hands the request out instead of
	/// blocking, the game thread stays parked (no spin, no field) until `end_movie`, then the game leaves movie mode.
	/// Run: cargo test --release retail_movie_hook -- --ignored --nocapture
	#[test]
	#[ignore]
	fn retail_movie_hook() {
		unsafe {
			std::env::set_var("RAC_RETAIL_HUD", "0");
			std::env::set_var("RAC_RETAIL_PLAY", "@play");
			std::env::set_var("RAC_PLAYER_CARD", "0");
			std::env::set_var("RAC_PLAYER_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/target/player_test"));
		}
		let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/play/eudora1_0041").into());
		let max: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(1450);
		enable(&dir).unwrap();
		let mut r = Retail::load().unwrap();
		fmv_bridge::set_serviced(true);
		let t0 = std::time::Instant::now();
		let mut got = None;
		while r.fields < max && !r.failed {
			r.tick(0, [0x80; 4]);
			if let Some(q) = r.take_movie() {
				got = Some(q);
				break;
			}
		}
		let q = got.unwrap_or_else(|| panic!("no movie request in {} fields (stop {:?}, mode {})", r.fields, r.stop, r.snap.mode));
		let at = r.fields;
		eprintln!("movie_hook: request {q:?} after field {at} ({:.1} s), last finished field mode {} level {}", t0.elapsed().as_secs_f32(), r.snap.mode, r.snap.level);
		assert!(fmv_bridge::waiting());
		// Parked, not spinning: no field completes while the movie "plays" (1 s), and tick returns at once.
		let mut tick_ms = 0.0f64;
		for _ in 0..50 {
			let t1 = std::time::Instant::now();
			r.tick(0, [0x80; 4]);
			tick_ms = tick_ms.max(t1.elapsed().as_secs_f64() * 1000.0);
			std::thread::sleep(std::time::Duration::from_millis(20));
		}
		assert_eq!(r.fields, at, "a field ran while the game thread should wait in the hook");
		assert!(r.take_movie().is_none() && fmv_bridge::waiting() && !r.failed);
		eprintln!("movie_hook: parked 1 s: fields still {at}, longest tick {tick_ms:.3} ms");
		r.end_movie();
		let mut modes = Vec::new();
		let mut more = 0;
		// After the movie the game may run an in-engine scene (mode 2); wait for gameplay (mode 0), at most 3000 fields.
		for _ in 0..3000 {
			r.tick(0, [0x80; 4]);
			if r.take_movie().is_some() {
				more += 1;
				r.end_movie();
			}
			if modes.last().map(|x: &(u32, u32)| x.1) != Some(r.snap.mode) {
				modes.push((r.fields, r.snap.mode));
			}
			if r.failed || (r.snap.mode == 0 && r.fields > at + 20) {
				break;
			}
		}
		let (played, skipped) = fmv_bridge::counts();
		eprintln!("movie_hook: after end_movie: modes (field, mode) {modes:?}; requests played {played} skipped {skipped} more {more}; stop {:?}; fields {}", r.stop, r.fields);
		assert!(r.stop.is_none() && !r.failed, "oracle stopped: {:?}", r.stop);
		assert_eq!((played, skipped, more), (1, 0, 0));
		assert_eq!(r.snap.mode, 0, "game mode after the movie");
	}

	/// Manual: the mobys near the camera after 150 idle fields.
	#[test]
	#[ignore]
	fn retail_near_camera() {
		enable(concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07")).unwrap();
		let mut r = Retail::load().unwrap();
		for _ in 0..151 {
			r.tick(0, [0x80; 4]);
		}
		let c = r.cam.0;
		for mb in &r.snap.mobys {
			let d = ((mb.pos[0] - c[0]).powi(2) + (mb.pos[1] - c[1]).powi(2) + (mb.pos[2] - c[2]).powi(2)).sqrt();
			if d < 12.0 {
				eprintln!("{:06x} class {} d {d:.1} pos {:?} scale {} shown {}", mb.addr, mb.class, mb.pos, mb.scale, mb.shown);
			}
		}
	}
}
