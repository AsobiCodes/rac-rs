//! [PC] R&C1's achievements = the 35 trophies of the PS3 "Ratchet & Clank" HD list (sources and the evidence for every detector:
//! `decomp/notes/achievements.md`). The engine half is `engine/achievements.rs`; this file is the list (`DEFS`) and the
//! detector (`read`, once per field from the worker, retail RAM read only). All addresses are below 0x15ef00, the same on the PAL
//! and the USA disc (no `disc::va` mapping needed): save-table A/B words of decomp/notes/savegame.md.
use crate::achievements::{AchPack, Def, Grade, Reading};

pub static PACK: AchPack = AchPack { game: "rac1", defs: &DEFS, read, icons: Some(&crate::trophy_icons::SET) };

const fn t(id: &'static str, name: &'static str, desc: &'static str, grade: Grade, hidden: bool, max: u32) -> Def {
	Def { id, name, desc, grade, hidden, max, platinum: false }
}

use Grade::{Bronze as B, Gold as G, Silver as S};

/// How a trophy is detected.
#[derive(Clone, Copy)]
enum Kind {
	/// Owned byte 0x13d4c0 + item id.
	Have(usize),
	/// Nanotech doctor flag byte 0x13d38c + n (0 = Premium, 1 = Ultra): set by the doctor's purchase (Orxon class 1326, update
	/// 0x2e7c80: Premium 4000 bolts -> 0x13d38c = 1, max 5; Ultra 30000 bolts -> 0x13d38d = 1, max 8) and by the NG+ restore.
	Nano(usize),
	/// Qwark's starfighter (class 388, Oltanis orbit) defeated = mission byte 0x14c050 + 13 * 16 + 3 is 0xff.
	Qwark,
	/// Lifetime bolts picked up: the 20 per-planet words 0x13df38 summed (see `PLANET_BOLTS`), carried across New Game+ cycles.
	Bolts,
	Weapons,
	Skills(u32),
	GoldBolts,
	/// Chairman Drek defeated (see `drek_defeated`), or the post-game page's flags.
	Drek,
	GoldWeapons,
	/// Skill-point byte 0x13d408 + index.
	Skill(usize),
	Platinum,
}
use Kind::*;

pub static DEFS: [Def; 35] = [
	t("hit_me_more", "Hit Me More!", "Obtain the Premium Nanotech upgrade", B, false, 0),
	t("lombax_magnetic", "Lombax Magnetic", "Obtain the Magneboots", B, false, 0),
	t("not_the_droids", "These are not the droids you're looking for...", "Obtain the Persuader", B, false, 0),
	t("very_sucky", "Very Sucky", "Obtain the Suck Cannon", B, false, 0),
	t("all_qwarkd_up", "All Qwark'd Up", "Defeat Captain Qwark's Ship", S, false, 0),
	t("bolt_collector", "Bolt Collector", "Collect 1,000,000 Bolts", S, false, 1_000_000),
	t("gadgeteer", "Gadgeteer", "Obtain all regular weapon gadgets", S, false, 14),
	t("nano_this", "Nano This!", "Upgrade to Ultra Nanotech", S, false, 0),
	t("ultimate_animal", "Ultimate Animal", "Obtain the R.Y.N.O.", S, false, 0),
	t("unlock_sketchbook", "Unlock Sketchbook", "Attain 15 Skill Points to Unlock Sketchbook in Menu", S, false, 15),
	t("bolt_commando", "Bolt Commando", "Gather All 40 Golden Bolts throughout the game", G, false, 40),
	t("drek_this", "Drek This!", "Defeat Chairman Drek", G, false, 0),
	t("king_midas", "Ratchet, King Midas", "Obtain all ten gold weapons", G, false, 10),
	t("skillful", "Skillful", "Master all 30 skill points", G, false, 30),
	t("speedy", "Speedy", "Beat the hoverboard challenge in less than 1:35 on Rilgar", G, false, 0),
	Def { id: "gadget_master", name: "Gadget Master", desc: "Obtain All Ratchet & Clank Trophies", grade: Grade::Platinum, hidden: false, max: 0, platinum: true },
	t("take_aim", "Take Aim", "Destroy a fighter or bomber on Novalis", B, true, 0),
	t("transported", "Transported", "Destroyed three flying transports on Aridia", B, true, 0),
	t("destroyed", "Destroyed", "On Gaspar, blow up all Blarg destroyers", B, true, 0),
	t("sniper", "Sniper", "Shoot a Scream through the sniper hole on Orxon", B, true, 0),
	t("buried_treasure", "Buried Treasure", "Find and destroy all the crates in the ocean on Pokitaru", B, true, 0),
	t("magician", "Magician", "Score 4500+ on a hoverboard race on Kalebo III", B, true, 0),
	t("pest_control", "Pest Control", "Kill all Anklebiters on Planet Hoven", B, true, 0),
	t("hey_over_here", "Hey, Over Here!", "Lure an enemy into a forcefield on Orxon", B, true, 0),
	t("sitting_ducks", "Sitting Ducks", "Destroy all cargo ships in the base on Orbital Oltanis", B, true, 0),
	t("careful_cruise", "Careful Cruise", "Traverse the water region without losing any health on Veldin's Orbital Station", B, true, 0),
	t("strike_a_pose", "Strike a Pose!", "Stand between Al's Roboshack Statue's legs on Kerwan", B, true, 0),
	t("going_commando", "Going Commando", "Kill 10 elite guards with only the wrench on Veldin", B, true, 0),
	t("vehicle_bombardier", "Vehicle Bombardier", "Blow Up 10 Vehicles on Eudora", B, true, 0),
	t("cluck_cluck", "Cluck, Cluck", "Turn a tank into a chicken on Rilgar", B, true, 0),
	t("qwarktastic", "Qwarktastic", "Destroy Captain Qwark training robot on Kerwan", B, true, 0),
	t("girl_trouble", "Girl Trouble", "Defeat Blarg Station Alien Queen using only the wrench!", S, true, 0),
	t("twisted_hoverboard", "Twisted Hoverboard", "Do a \"Twisty McMarx\" in the hoverboard race", S, true, 0),
	t("vandal", "Vandal", "Destroy all streetlights in Oltanis", S, true, 0),
	t("eat_lead", "Eat Lead", "Use a Sand Mouse to destroy all the tanks on Batalia", S, true, 0),
];

/// Parallel to `DEFS` (a test holds the ids together). Skill indices = the position in the retail Skill Points list (texts
/// 20444.. = 0x4fdc..).
static KINDS: [(&str, Kind); 35] = [
	("hit_me_more", Nano(0)),
	("lombax_magnetic", Have(28)),
	("not_the_droids", Have(35)),
	("very_sucky", Have(9)),
	("all_qwarkd_up", Qwark),
	("bolt_collector", Bolts),
	("gadgeteer", Weapons),
	("nano_this", Nano(1)),
	("ultimate_animal", Have(23)),
	("unlock_sketchbook", Skills(15)),
	("bolt_commando", GoldBolts),
	("drek_this", Drek),
	("king_midas", GoldWeapons),
	("skillful", Skills(30)),
	("speedy", Skill(9)),
	("gadget_master", Platinum),
	("take_aim", Skill(0)),
	("transported", Skill(2)),
	("destroyed", Skill(14)),
	("sniper", Skill(16)),
	("buried_treasure", Skill(19)),
	("magician", Skill(26)),
	("pest_control", Skill(20)),
	("hey_over_here", Skill(17)),
	("sitting_ducks", Skill(22)),
	("careful_cruise", Skill(28)),
	("strike_a_pose", Skill(3)),
	("going_commando", Skill(29)),
	("vehicle_bombardier", Skill(6)),
	("cluck_cluck", Skill(8)),
	("qwarktastic", Skill(5)),
	("girl_trouble", Skill(10)),
	("twisted_hoverboard", Skill(7)),
	("vandal", Skill(23)),
	("eat_lead", Skill(13)),
];

/// The ten weapons with a gold version (price column of weapons_framework.md "Novalis revisit item inventory"): gold byte 0x13e520 + id.
const GOLD_IDS: [usize; 10] = [9, 10, 11, 15, 16, 17, 19, 20, 21, 25];
/// The 14 weapons other than the R.Y.N.O. (23) and the wrench.
const WEAPON_IDS: [usize; 14] = [9, 10, 11, 13, 14, 15, 16, 17, 18, 19, 20, 21, 24, 25];

const LEVEL: usize = 0x15ed84;
const NANO_FLAGS: usize = 0x13d38c;
/// Per-level mission bytes {16 per level} (B3004); 0xff = that mission slot is done.
const MISSIONS: usize = 0x14c050;
/// Qwark's ship: level 13 (Oltanis orbit), slot 3 (the class 388 moby and the player ship moby both carry +0xb0 = 3; the ship's
/// win exit, FUN_002ba990 case 8, calls FUN_0025bab8(3) which stores 0xff).
const QWARK_SLOT: usize = 13 * 16 + 3;
/// Challenge-mode (NG+) counter: with the endgame flag the retail "game completed" test (Goodies, cheats).
const NG_PLUS: usize = 0x15ee20;
const BOLTS: usize = 0x15ed98;
const HAVE: usize = 0x13d4c0;
const GOLD_UP: usize = 0x13e520;
const SKILLS: usize = 0x13d408;
const GOLD_BOLTS: usize = 0x14bec0;
/// 20 words, one per level: bolts picked up on that level. Only writer: the bolt pickup FUN_002a63d8 (class-independent, every
/// overlay has a copy; `*(0x13df38 + 4 * level) += value`, next to `0x15ed98 += value`; the one other bolt gain, a pickup that could
/// not spawn its moby (Novalis overlay line 117973), skips it). Spending never lowers it. Not in the New Game+ restore list, so
/// the new cycle starts at 0 (ngplus state 0000: sum 0, held bolts 7124, 0x15ee20 = 1; fg2 0283: sum 94262, held 7124).
const PLANET_BOLTS: usize = 0x13df38;
/// The Drek boss moby pointer: written by Drek's update (class 1422, Veldin finale FUN_002f2520, only there) every update once
/// its phase counter is past 1; the camera code reads it as the boss target.
const BOSS: usize = 0x13f928;
const DREK_CLASS: u16 = 1422;
/// Drek's moby state after the killing blow (FUN_002f2520 case 0x1b: starts the death scene FUN_00297d18(3), then the credits, the
/// ad movie and the post-game page, which deletes the moby). fg2 0283 / 0284 / 0285 (scene, credits, movie): state 27.
const DREK_DEAD: u8 = 0x1b;
const ENDGAME: usize = 0x15eea0;

/// Every retail RAM range this file reads (base, length), all below 0x15ef00: `disc::tests::trophy_addresses_are_core` proves each
/// maps to itself on the USA disc.
pub const ADDRS: [(usize, usize); 13] = [
	(LEVEL, 4),
	(NANO_FLAGS, 2),
	(MISSIONS, 16 * 19),
	(NG_PLUS, 4),
	(BOLTS, 4),
	(HAVE, 40),
	(GOLD_UP, 40),
	(SKILLS, 30),
	(GOLD_BOLTS, 76),
	(PLANET_BOLTS, 80),
	(ENDGAME, 4),
	(BOSS, 4),
	(MISSIONS + QWARK_SLOT, 1),
];

/// The detector. None while RAM is not a game (too short, level word out of range).
pub fn read(ram: &[u8]) -> Option<Vec<Reading>> {
	if ram.len() <= ENDGAME + 4 {
		return None;
	}
	let w = |a: usize| i32::from_le_bytes([ram[a], ram[a + 1], ram[a + 2], ram[a + 3]]);
	if !(0..19).contains(&w(LEVEL)) {
		return None;
	}
	let nz = |a: usize, n: usize| ram[a..a + n].iter().filter(|&&b| b != 0).count() as u32;
	let skills = nz(SKILLS, 30);
	let gold_bolts = nz(GOLD_BOLTS, 76).min(40);
	let weapons = WEAPON_IDS.iter().filter(|&&i| ram[HAVE + i] != 0).count() as u32;
	let golds = GOLD_IDS.iter().filter(|&&i| ram[GOLD_UP + i] != 0).count() as u32;
	let planets: i64 = (0..20).map(|i| i64::from(w(PLANET_BOLTS + 4 * i).max(0))).sum();
	let bolts = planets.min(1_000_000) as u32;
	let mut out: Vec<Reading> = KINDS
		.iter()
		.map(|(_, k)| match *k {
			Have(i) => cond(ram[HAVE + i] != 0),
			Nano(n) => cond(ram[NANO_FLAGS + n] != 0),
			Qwark => cond(ram[MISSIONS + QWARK_SLOT] == 0xff),
			// Lifetime total: the planet words restart at New Game+ (challenge counter 0x15ee20), the store carries the old cycles.
			Bolts => Reading { epoch: Some(w(NG_PLUS).max(0) as u32), ..count(bolts, 1_000_000) },
			Weapons => count(weapons, 14),
			Skills(n) => Reading { met: skills >= n, progress: skills, epoch: None },
			GoldBolts => count(gold_bolts, 40),
			// First route: Drek's moby in its death state (the kill, before the death scene). Second route: the post-game page (after
			// the credits) sets 0x15eea0 = 1 (Cross) or 0x15ee20 += 1 (Circle, NG+): retail's "game completed" test, also for loads.
			Drek => cond(drek_defeated(ram, w(LEVEL)) || w(ENDGAME) != 0 || w(NG_PLUS) != 0),
			GoldWeapons => count(golds, 10),
			Skill(i) => cond(ram[SKILLS + i] != 0),
			Platinum => Reading::default(),
		})
		.collect();
	// The Gadgeteer / King Midas / Bolt Commando / Skillful / Sketchbook counters carry their own goal in `Reading::progress`.
	if let Some(i) = KINDS.iter().position(|(id, _)| *id == "skillful") {
		out[i].met = skills >= 30;
	}
	Some(out)
}

/// Veldin finale (level 18), boss pointer 0x13f928 -> a class 1422 moby in state 0x1b.
fn drek_defeated(ram: &[u8], level: i32) -> bool {
	let p = u32::from_le_bytes([ram[BOSS], ram[BOSS + 1], ram[BOSS + 2], ram[BOSS + 3]]) as usize;
	level == 18 && p % 4 == 0 && (0x100000..ram.len().saturating_sub(0x100)).contains(&p) && ram[p + 0x20] == DREK_DEAD && u16::from_le_bytes([ram[p + 0xa6], ram[p + 0xa7]]) == DREK_CLASS
}

fn cond(met: bool) -> Reading {
	Reading { met, ..Reading::default() }
}

fn count(n: u32, goal: u32) -> Reading {
	Reading { met: n >= goal, progress: n, epoch: None }
}

#[cfg(test)]
mod tests {
	use super::*;

	fn ram() -> Vec<u8> {
		vec![0; 0x160000]
	}
	fn set32(r: &mut [u8], a: usize, v: i32) {
		r[a..a + 4].copy_from_slice(&v.to_le_bytes());
	}
	fn get(r: &[u8], id: &str) -> Reading {
		let i = DEFS.iter().position(|d| d.id == id).unwrap();
		read(r).unwrap()[i]
	}

	#[test]
	fn the_list_is_the_ps3_list() {
		assert_eq!(DEFS.len(), 35);
		let n = |g: Grade| DEFS.iter().filter(|d| d.grade == g).count();
		assert_eq!((n(Grade::Bronze), n(Grade::Silver), n(Grade::Gold), n(Grade::Platinum)), (19, 10, 5, 1));
		assert_eq!(DEFS.iter().filter(|d| d.hidden).count(), 19);
		assert_eq!(DEFS.iter().filter(|d| d.platinum).count(), 1);
		for (d, (id, _)) in DEFS.iter().zip(&KINDS) {
			assert_eq!(d.id, *id);
		}
		let mut ids: Vec<_> = DEFS.iter().map(|d| d.id).collect();
		ids.sort();
		ids.dedup();
		assert_eq!(ids.len(), 35, "ids are unique");
	}

	#[test]
	fn nothing_in_a_fresh_game_and_no_game_no_reading() {
		let mut r = ram();
		assert!(read(&r[..1000]).is_none());
		set32(&mut r, LEVEL, 99);
		assert!(read(&r).is_none());
		set32(&mut r, LEVEL, 0);
		assert!(read(&r).unwrap().iter().all(|x| !x.met));
	}

	#[test]
	fn items_nanotech_and_story() {
		let mut r = ram();
		set32(&mut r, LEVEL, 3);
		r[NANO_FLAGS] = 1;
		assert!(get(&r, "hit_me_more").met && !get(&r, "nano_this").met);
		r[NANO_FLAGS + 1] = 1;
		assert!(get(&r, "nano_this").met);
		for (id, item) in [("lombax_magnetic", 28), ("not_the_droids", 35), ("very_sucky", 9), ("ultimate_animal", 23)] {
			assert!(!get(&r, id).met);
			r[HAVE + item] = 1;
			assert!(get(&r, id).met, "{id}");
		}
		assert!(!get(&r, "all_qwarkd_up").met && !get(&r, "drek_this").met);
		r[MISSIONS + 17 * 16 + 3] = 0xff;
		r[MISSIONS + 13 * 16 + 2] = 0xff;
		assert!(!get(&r, "all_qwarkd_up").met, "only level 13 slot 3");
		r[MISSIONS + QWARK_SLOT] = 0xff;
		assert!(get(&r, "all_qwarkd_up").met);
		set32(&mut r, NG_PLUS, 1);
		assert!(get(&r, "drek_this").met, "NG+ counter");
		set32(&mut r, NG_PLUS, 0);
		assert!(!get(&r, "drek_this").met);
		set32(&mut r, ENDGAME, 1);
		assert!(get(&r, "drek_this").met);
	}

	#[test]
	fn counters() {
		let mut r = ram();
		set32(&mut r, LEVEL, 1);
		for (n, i) in WEAPON_IDS.iter().enumerate() {
			r[HAVE + i] = 1;
			assert_eq!(get(&r, "gadgeteer").progress, n as u32 + 1);
		}
		assert!(get(&r, "gadgeteer").met);
		for (n, i) in GOLD_IDS.iter().enumerate() {
			assert!(!get(&r, "king_midas").met);
			r[GOLD_UP + i] = 1;
			assert_eq!(get(&r, "king_midas").progress, n as u32 + 1);
		}
		assert!(get(&r, "king_midas").met);
		for i in 0..76 {
			r[GOLD_BOLTS + i] = 1;
		}
		assert_eq!(get(&r, "bolt_commando").progress, 40, "capped at 40");
		set32(&mut r, BOLTS, 5_000_000);
		assert!(!get(&r, "bolt_collector").met, "held bolts are not a lifetime total");
		set32(&mut r, PLANET_BOLTS + 8, 600_000);
		set32(&mut r, PLANET_BOLTS + 12, 399_999);
		assert!(!get(&r, "bolt_collector").met);
		assert_eq!((get(&r, "bolt_collector").progress, get(&r, "bolt_collector").epoch), (999_999, Some(0)));
		set32(&mut r, PLANET_BOLTS + 12, 400_000);
		assert!(get(&r, "bolt_collector").met, "lifetime gained bolts count");
		set32(&mut r, NG_PLUS, 2);
		assert_eq!(get(&r, "bolt_collector").epoch, Some(2), "the cycle is the challenge counter");
	}

	#[test]
	fn drek_dies_before_the_credits() {
		let mut r = ram();
		set32(&mut r, LEVEL, 18);
		let p = 0x1eee640;
		set32(&mut r, BOSS, p as i32);
		r.resize(0x2000000, 0);
		r[p + 0x20] = 0x0e;
		r[p + 0xa6..p + 0xa8].copy_from_slice(&DREK_CLASS.to_le_bytes());
		assert!(!get(&r, "drek_this").met, "fighting");
		r[p + 0x20] = DREK_DEAD;
		assert!(get(&r, "drek_this").met, "killing blow, no post-game flag yet");
		r[p + 0xa6] = 0;
		assert!(!get(&r, "drek_this").met, "another class");
		r[p + 0xa6] = 0x8e;
		set32(&mut r, LEVEL, 17);
		assert!(!get(&r, "drek_this").met, "other level, stale pointer");
		set32(&mut r, LEVEL, 18);
		set32(&mut r, BOSS, 0x1ffff00);
		assert!(!get(&r, "drek_this").met, "pointer past the end");
		set32(&mut r, BOSS, 0);
		assert!(!get(&r, "drek_this").met);
	}

	#[test]
	fn skills_map_to_their_trophies() {
		let mut r = ram();
		set32(&mut r, LEVEL, 2);
		let map = [("take_aim", 0), ("transported", 2), ("strike_a_pose", 3), ("qwarktastic", 5), ("twisted_hoverboard", 7), ("cluck_cluck", 8), ("speedy", 9), ("girl_trouble", 10), ("eat_lead", 13), ("destroyed", 14), ("sniper", 16), ("hey_over_here", 17), ("buried_treasure", 19), ("pest_control", 20), ("sitting_ducks", 22), ("vandal", 23), ("vehicle_bombardier", 6), ("magician", 26), ("careful_cruise", 28), ("going_commando", 29)];
		for (id, idx) in map {
			let before = read(&r).unwrap();
			r[SKILLS + idx] = 1;
			let after = read(&r).unwrap();
			let changed: Vec<_> = (0..35).filter(|&i| before[i].met != after[i].met).map(|i| DEFS[i].id).filter(|id| *id != "unlock_sketchbook").collect();
			assert_eq!(changed, vec![id], "skill {idx}");
		}
		// 20 set so far; Sketchbook at 15, Skillful at 30.
		assert!(get(&r, "unlock_sketchbook").met && !get(&r, "skillful").met);
		for i in 0..30 {
			r[SKILLS + i] = 1;
		}
		assert!(get(&r, "skillful").met && get(&r, "skillful").progress == 30);
	}
}
