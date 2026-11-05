//! Disc tests of formats/sound.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::sound::*;
	use crate::{iso, toc};
	use std::path::{Path, PathBuf};
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::iso::Disc;
	use crate::level::Level;
	use crate::rand::Rng;
	use crate::physics::add;
	use crate::physics::length;
	use crate::physics::scale;
	use crate::physics::sub;
	use crate::physics::Hit;
	use crate::physics::MobySolid;
	use crate::physics::Scene;
	use crate::physics::World;
	use crate::physics::V3;
	use std::cell::RefCell;
	use std::collections::HashMap;
	use std::sync::Arc;
	use crate::cues::*;
	use crate::toc::help_voice;

	const ISO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");

	const OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/test_out/sounds");

	fn write_wav(path: &Path, s: &Sample) {
		let mut b = Vec::new();
		let n = (s.pcm.len() * 2) as u32;
		b.extend(b"RIFF");
		b.extend((36 + n).to_le_bytes());
		b.extend(b"WAVEfmt ");
		b.extend(16u32.to_le_bytes());
		b.extend(1u16.to_le_bytes());
		b.extend(1u16.to_le_bytes());
		b.extend(s.rate.to_le_bytes());
		b.extend((s.rate * 2).to_le_bytes());
		b.extend(2u16.to_le_bytes());
		b.extend(16u16.to_le_bytes());
		b.extend(b"data");
		b.extend(n.to_le_bytes());
		for v in &s.pcm {
			b.extend(v.to_le_bytes());
		}
		std::fs::write(path, b).unwrap();
	}

	/// The HelpDesk voice stream decodes for English and is absent past the table (ingame_menu_leftovers.md).
	#[test]
	fn help_voice_decodes() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let (rate, pcm) = (0..150).find_map(|v| help_voice(&mut disc, 0, v).unwrap()).expect("an English help voice");
		assert!((8000..=48000).contains(&rate) && pcm.len() > rate as usize / 2, "rate {rate}, {} samples", pcm.len());
		assert!(help_voice(&mut disc, 0, 150).unwrap().is_none());
	}

	/// AUD-11a.fan: only the skeleton-less fan has a static loop sound.
	#[test]
	fn novalis_fan_static_loop() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let nov = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&nov).unwrap();
		assert_eq!(ev.static_loops.get(&705), Some(&0));
		assert_eq!(ev.enemy_cs[&(705, 0)].sound, 227);
		for c in [459, 577, 660] {
			assert!(!ev.static_loops.contains_key(&c));
		}
		assert_eq!(ev.static_loops.len(), 1);
		assert_eq!(static_loop_sound(&nov, 705).unwrap(), Some(0));
		assert_eq!(static_loop_sound(&nov, 459).unwrap(), None);
		let vel = Level::load(&mut disc, &levels[0]).unwrap();
		let ev = events(&vel).unwrap();
		assert!(ev.static_loops.keys().all(|c| crate::game::CUE_CLASSES.contains(c)));
	}

	/// Aridia enemies (shark 580, constructorbot 612, nest 668) resolve their sequence-trigger and event cues through
	/// `enemy_cs`; before they were in CUE_CLASSES the level was silent for them.
	#[test]
	fn aridia_enemy_cue_classes() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[2]).unwrap();
		let ev = events(&l).unwrap();
		for (c, n) in [(580u32, 7usize), (612, 10), (668, 5)] {
			assert!(crate::game::CUE_CLASSES.contains(&c), "{c}");
			assert!((0..n).any(|i| ev.enemy_cs.contains_key(&(c, i))), "{c}");
		}
		assert!(ev.enemy_cs.contains_key(&(580, 4)));
	}

	/// AUD-11a: Novalis animation cue classes resolve through `enemy_cs`; triggers and loop bytes parse.
	#[test]
	fn novalis_cue_classes() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&l).unwrap();
		for (k, snd) in [((459, 4), 165), ((577, 4), 193), ((572, 0), 183), ((865, 4), 258), ((866, 0), 261), ((705, 0), 227), ((604, 0), 201), ((790, 0), 244)] {
			assert_eq!(ev.enemy_cs[&k].sound, snd, "{k:?}");
		}
		let cues = |class: u32, seq: usize| {
			let a = crate::anim::animation(&l, class).unwrap().unwrap();
			let s = &a.sequences[seq];
			(s.triggers.iter().map(|w| ((w & 0xffff) as usize, (w >> 16) as usize)).collect::<Vec<_>>(), s.loop_sound)
		};
		assert_eq!(cues(459, 1).0, vec![(2, 24), (1, 136)]);
		assert_eq!(cues(459, 8).1, 6);
		assert_eq!(cues(577, 5).0, vec![(6, 56), (6, 128), (1, 176), (0, 184)]);
		assert_eq!(cues(572, 1).1, 4);
	}

	/// Parsed trigger words and loop bytes match the cues read from the decomp (sound.md "Animation sound cues").
	#[test]
	fn enemy_animation_cues() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[0]).unwrap();
		let cues = |class: u32, seq: usize| {
			let a = crate::anim::animation(&l, class).unwrap().unwrap();
			let s = &a.sequences[seq];
			(s.triggers.iter().map(|w| ((w & 0xffff) as usize, (w >> 16) as usize)).collect::<Vec<_>>(), s.loop_sound)
		};
		assert_eq!(cues(749, 1).0, vec![(8, 40)]);
		assert_eq!(cues(749, 5).0, vec![(6, 64), (0, 160)]);
		assert_eq!(cues(749, 6).0, vec![(2, 8), (4, 192), (1, 288)]);
		assert_eq!(cues(1440, 2).0, vec![(0, 1776)]);
		assert_eq!(cues(1440, 7).0, vec![(6, 8), (8, 304)]);
		assert_eq!(cues(1440, 10).0, vec![(1, 8), (2, 256), (3, 624)]);
		assert_eq!(cues(1440, 6).1, 5);
		assert_eq!(cues(1440, 9).1, 10);
	}

	#[test]
	fn veldin_events() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[0]).unwrap();
		let ev = events(&l).unwrap();
		println!("{ev:#?}");
		assert_eq!(ev.jump.unwrap().sound, 46);
		assert_eq!(ev.hurt.unwrap().sound, 53);
		assert_eq!(ev.swing.iter().map(|c| c.sound).collect::<Vec<_>>(), vec![47, 72]);
		assert_eq!(ev.bolt.get(&13).unwrap().sound, 91);
		assert_eq!(ev.crate_break[&500].sound, 179);
		assert_eq!(ev.crate_break[&501].sound, 180);
		assert_eq!(ev.crate_break[&511].sound, 182);
		assert_eq!(ev.nanotech.unwrap().sound, 181);
		assert!(ev.slam.iter().all(|c| c.is_some()), "{:?}", ev.slam);
		assert_eq!(ev.footsteps.len(), 16);
		assert!(ev.footsteps.iter().all(|c| c.is_some()));
		assert!(ev.ambient.map_or(false, |c| c.looped));
		assert_eq!(ev.enemy[&(749, "idle")].sound, 205);
		assert_eq!(ev.enemy[&(1440, "death")].sound, 224);
		for i in 0..9 {
			assert_eq!(ev.enemy_cs[&(749, i)].sound, 197 + i);
		}
		for i in 0..11 {
			assert_eq!(ev.enemy_cs[&(1440, i)].sound, 223 + i);
		}
		for c in ev.footsteps.iter().flatten().chain(ev.jump.iter()) {
			assert!(c.volume > 0.0 && c.volume <= 1.0, "{c:?}");
		}
		// Ground sound classes under the level's collision (class 3 plays as 0).
		let mut hist = [0usize; 4];
		for f in crate::collision::collision(&l).unwrap() {
			hist[((f.kind >> 5) & 3) as usize] += 1;
		}
		println!("collision sound-class histogram {hist:?}");
		// Write a few decoded events for listening / duration checks.
		let samples = level_sounds(&l).unwrap();
		let dir = PathBuf::from(OUT).parent().unwrap().join("sfx");
		std::fs::create_dir_all(&dir).unwrap();
		for (name, c) in [("jump", ev.jump), ("hurt", ev.hurt), ("bolt", ev.bolt.get(&13).copied()), ("crate", ev.crate_break.get(&500).copied()), ("nanotech", ev.nanotech), ("step0", ev.footsteps[0])] {
			let c = c.unwrap();
			let sm = &samples[c.sound];
			println!("{name}: sound {} {:.2}s", c.sound, sm.pcm.len() as f32 / sm.rate as f32);
			write_wav(&dir.join(format!("{name}.wav")), sm);
		}
	}

	/// Layout-B tones store the centre note as a negative signed byte: 989snd negates it
	/// (IOP FUN_0001600c) before sceSdNote2Pitch. Sound 19 (Veldin ambient loop) is centre 86.
	#[test]
	fn negative_centre_note() {
		let mut p = [0u8; 32];
		p[2] = 0xaa;
		p[3] = 0x42;
		let t = Grain { kind: GRAIN_TONE, delay: 0, params: p }.tone().unwrap();
		assert_eq!((t.centre_note, t.centre_fine), (86, 0x42));
		let r = t.rate();
		assert_eq!(r, 11016, "pitch 940 (centre 86 fine 66 ADDED)");
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[0]).unwrap();
		let bank = level_bank(&l).unwrap();
		let s = bank.sound_sample(19).unwrap().unwrap();
		println!("sound 19: rate {} len {} samples = {:.2}s loop {:?}", s.rate, s.pcm.len(), s.pcm.len() as f32 / s.rate as f32, s.loop_start);
		assert_eq!(s.rate, 11016);
	}

	/// Every gameplay cue is a class sound with its own def (no bank-number fallbacks): the gadget classes take
	/// their defs from the gadget table, and an unmapped sound plays nothing (Novalis hero cs9 = 0xffff).
	#[test]
	fn cues_come_from_defs() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for id in [0usize, 1] {
			let l = Level::load(&mut disc, &levels[id]).unwrap();
			let ev = events(&l).unwrap();
			let g = &ev.glove;
			for (name, c) in [("throw", &g.throw), ("empty", &g.empty), ("draw", &g.draw), ("pickup", &g.pickup), ("small0", &g.small[0]), ("small1", &g.small[1]), ("big", &g.big), ("wthrow", &ev.wrench_throw), ("wcatch", &ev.wrench_catch), ("imp0", &ev.wrench_impact[0]), ("imp1", &ev.wrench_impact[1]), ("imp2", &ev.wrench_impact[2])] {
				let c = c.as_ref().unwrap_or_else(|| panic!("level {id} cue {name} missing"));
				assert!(c.fall.max_range > 0.0, "level {id} cue {name} has an empty def");
			}
			// The glove classes' defs come from the gadget table, not a core class block.
			assert_eq!(ev.death.is_some(), id == 0, "level {id} death cue");
		}
	}

	// AUD-11d test 2: Novalis hero cs 26 / 30 / 33 are bank 42 / 44 / 46.
	#[test]
	fn novalis_hero_trigger_sounds() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&l).unwrap();
		assert_eq!((ev.hero_cs[&26].sound, ev.hero_cs[&30].sound, ev.hero_cs[&33].sound), (42, 44, 46));
	}

	// AUD-11g test 1: the footstep def base per level, and the Novalis def values.
	#[test]
	fn footstep_defs_per_level() {
		assert_eq!((footstep_base(0), footstep_base(1)), (2, 4));
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let nov = events(&Level::load(&mut disc, &levels[1]).unwrap()).unwrap();
		let f = |i: usize| nov.footsteps[i].unwrap();
		assert_eq!(f(0).sound, 4);
		assert!((f(0).volume - 655.0 / 1024.0).abs() < 1e-6);
		assert_eq!(f(12).sound, 16);
		assert_eq!(f(12).volume, 1.0);
		let vel = events(&Level::load(&mut disc, &levels[0]).unwrap()).unwrap();
		assert_eq!(vel.footsteps[0].unwrap().sound, 2);
		// ARI-2 (decomp/notes/aridia_environment.md): Aridia table byte 6, defs 8..23.
		assert_eq!(footstep_base(2), 8);
		let ari = events(&Level::load(&mut disc, &levels[2]).unwrap()).unwrap();
		assert!((0..16).all(|i| ari.footsteps[i].unwrap().sound == 8 + i));
	}

	// AUD-8b test 1: the wrench impact class 71 cs2 resolves to a level-bank sound on both levels.
	#[test]
	fn wrench_impact_uses_level_bank() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for i in [0, 1] {
			let l = Level::load(&mut disc, &levels[i]).unwrap();
			let bank = level_bank(&l).unwrap();
			let c = class_cue(&l, 71, 2).unwrap().unwrap();
			assert!(bank.sounds.get(c.sound).is_some(), "level {i}");
		}
	}

	// Pause-menu cues: class 0x472 cs0..7 through each level's remap (Veldin, Novalis, Aridia), and the fixed
	// pitch of cue 3 (+0xfe) and cue 4 (+0x1fc) through the tone bend range 127 (1/128 semitones).
	#[test]
	fn menu_cue_remap_per_level() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for (i, first) in [(0, 213), (1, 272), (2, 262)] {
			let l = Level::load(&mut disc, &levels[i]).unwrap();
			let ev = events(&l).unwrap();
			let got: Vec<usize> = ev.menu.iter().map(|c| c.as_ref().unwrap().sound).collect();
			assert_eq!(got, (first..first + 8).collect::<Vec<_>>(), "level {i}");
			let shift = |k: usize| ev.menu[k].as_ref().unwrap().random_shift(|_| unreachable!());
			assert_eq!((shift(0), shift(1), shift(3), shift(4)), (0, 0, 126, 252), "level {i}");
		}
	}

	// AUD-11e test 1: Novalis hero remap for the direct cues.
	#[test]
	fn novalis_hero_direct_cue_sounds() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&l).unwrap();
		for (cs, bank) in [(3, 25), (6, 28), (7, 29), (8, 30), (0xc, 32), (0x13, 37), (0x17, 39)] {
			assert_eq!(ev.hero_cs[&cs].sound, bank, "cs {cs}");
		}
	}

	// AUD-11c test 1: the Suck Cannon capture sound (class 849 cs5) is bank 252 on Novalis.
	#[test]
	fn suck_cannon_capture_sound() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&l).unwrap();
		assert_eq!(ev.class[&(849, 5)].sound, 252);
	}

	// WPN-6.9d4 test (3): every held rig class with sequence triggers resolves its cues (gadget block counts, Novalis banks).
	#[test]
	fn held_class_sounds() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&l).unwrap();
		let want: [(u32, &[(usize, usize)]); 8] = [
			(175, &[(6, 114), (7, 115)]),
			(168, &[(2, 102), (3, 103), (4, 104)]),
			(157, &[(0, 93), (1, 94), (2, 95), (3, 96)]),
			(163, &[(0, 97), (1, 98), (2, 99)]),
			(180, &[(2, 127)]),
			(190, &[(1, 138)]),
			(229, &[(0, 151), (1, 152)]),
			(185, &[(1, 129), (2, 130), (3, 131)]),
		];
		for (class, cues) in want {
			assert!(crate::game::SOUND_CLASSES.contains(&class), "{class}");
			for &(cs, bank) in cues {
				assert_eq!(ev.class[&(class, cs)].sound, bank, "{class} cs {cs}");
			}
		}
	}

	// NOV-13.2 test 1: all eight vendor defs are registered (class block +0x0d), cs i -> Novalis bank 68 + i.
	#[test]
	fn vendor_class_sounds_all_eight() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&l).unwrap();
		assert_eq!(ev.class[&(11, 7)].sound, 75);
		assert_eq!(ev.class[&(11, 4)].sound, 72);
		for i in 0..8 {
			assert_eq!(ev.class[&(11, i)].sound, 68 + i);
			assert_eq!(ev.class[&(11, i)].volume, 1.0);
		}
	}
}
