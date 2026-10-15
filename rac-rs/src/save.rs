//! Save games in the game's own 59,912-byte image format (decomp/notes/savegame.md), stored as
//! plain files `saves/slot0..4.bin` next to the executable instead of on a memory card.
//!
//! Image: u32 sizeA, u32 sizeB, the global block, then one block per level (20). Block: u32
//! payload length, u32 CRC of the payload, then records {id, len, data padded to 4} and a
//! {-1, 0} terminator. Records map onto game globals by id; ids rac-rs does not model are kept
//! as opaque bytes from the default (new game) image.

use crate::bytes::{span, u32_at, R};
use crate::iso::Disc;
use crate::settings::{Settings, VOLUME_MAX};
use crate::toc;
use std::path::{Path, PathBuf};

pub const SLOTS: usize = 5;
pub const LEVELS: usize = 20;
/// ToC entry of save_game.bin; its header word 4/5 = default image offset/size.
const SAVE_GAME: usize = 0x10;

/// Global record ids (table A at 0x1845c0) that rac-rs reads or writes.
pub const CURRENT_LEVEL: u32 = 0;
/// Play time in 60 Hz frames (the slot list shows it as hours:minutes) [I].
pub const PLAY_TIME: u32 = 3;
/// gBolts, then a second word (lifetime total?) [I].
pub const BOLTS: u32 = 1;
/// The 8-byte BCD wall-clock record shown as the slot date.
pub const RTC: u32 = 4;
pub const DEATHS: u32 = 1005;
/// Per-level record ids (table B at 0x1848c0).
pub const VISITED: u32 = 3001;

/// memcard_Checksum (0x24cb88): MSB-first CRC-16, poly 0x1f45, init 0x8320; 0 above 0x1800 bytes.
pub fn checksum(data: &[u8]) -> u32 {
	if data.len() > 0x1800 {
		return 0;
	}
	let mut c: u16 = 0x8320;
	for &b in data {
		c ^= (b as u16) << 8;
		for _ in 0..8 {
			c = if c & 0x8000 != 0 { (c << 1) ^ 0x1f45 } else { c << 1 };
		}
	}
	c as u32
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block(pub Vec<(u32, Vec<u8>)>);

impl Block {
	fn parse(b: &[u8]) -> R<Block> {
		let len = u32_at(b, 0)? as usize;
		let payload = span(b, 8, len)?;
		if checksum(payload) != u32_at(b, 4)? {
			return Err("save block checksum mismatch".into());
		}
		let mut out = Vec::new();
		let mut at = 8;
		loop {
			let id = u32_at(b, at)?;
			if id == u32::MAX {
				return Ok(Block(out));
			}
			let n = u32_at(b, at + 4)? as usize;
			out.push((id, span(b, at + 8, n)?.to_vec()));
			at += 8 + n.next_multiple_of(4);
		}
	}

	/// GetDataSize: header, records padded to 4, terminator.
	fn size(&self) -> usize {
		8 + self.0.iter().map(|(_, d)| 8 + d.len().next_multiple_of(4)).sum::<usize>() + 8
	}

	fn write(&self, out: &mut Vec<u8>) {
		let start = out.len();
		out.extend([0; 8]);
		for (id, d) in &self.0 {
			out.extend(id.to_le_bytes());
			out.extend((d.len() as u32).to_le_bytes());
			out.extend(d);
			out.resize(out.len().next_multiple_of(4), 0);
		}
		out.extend(u32::MAX.to_le_bytes());
		out.extend(0u32.to_le_bytes());
		let len = (out.len() - start - 8) as u32;
		let crc = checksum(&out[start + 8..]);
		out[start..start + 4].copy_from_slice(&len.to_le_bytes());
		out[start + 4..start + 8].copy_from_slice(&crc.to_le_bytes());
	}

	pub fn get(&self, id: u32) -> Option<&[u8]> {
		self.0.iter().find(|r| r.0 == id).map(|r| r.1.as_slice())
	}

	pub fn word(&self, id: u32) -> Option<u32> {
		self.get(id).and_then(|d| u32_at(d, 0).ok())
	}

	/// Overwrites the leading bytes of a record (keeps its length).
	pub fn set_bytes(&mut self, id: u32, v: &[u8]) {
		if let Some(r) = self.0.iter_mut().find(|r| r.0 == id) {
			let n = r.1.len().min(v.len());
			r.1[..n].copy_from_slice(&v[..n]);
		}
	}

	/// Sets the first word of a record (keeps its length).
	pub fn set_word(&mut self, id: u32, v: u32) {
		if let Some(r) = self.0.iter_mut().find(|r| r.0 == id).filter(|r| r.1.len() >= 4) {
			r.1[..4].copy_from_slice(&v.to_le_bytes());
		}
	}
}

#[derive(Clone, Debug, PartialEq)]
pub struct Save {
	pub global: Block,
	pub levels: Vec<Block>,
}

impl Save {
	pub fn parse(b: &[u8]) -> R<Save> {
		let (a, s) = (u32_at(b, 0)? as usize, u32_at(b, 4)? as usize);
		let global = Block::parse(span(b, 8, a)?)?;
		let levels = (0..LEVELS).map(|l| Block::parse(span(b, 8 + a + l * s, s)?)).collect::<R<Vec<_>>>()?;
		Ok(Save { global, levels })
	}

	/// MakeWholeSave. Block sizes come from the records, so an image built from the default one
	/// keeps the game's own sizes.
	pub fn bytes(&self) -> Vec<u8> {
		let mut out = Vec::new();
		out.extend((self.global.size() as u32).to_le_bytes());
		out.extend((self.levels.first().map_or(16, Block::size) as u32).to_le_bytes());
		self.global.write(&mut out);
		for l in &self.levels {
			l.write(&mut out);
		}
		out
	}

	/// The new-game image from save_game.bin (loaded by 0x24b210 before a new game).
	pub fn default_image(disc: &mut Disc) -> R<Save> {
		let t = toc::read_toc(disc).map_err(|e| e.to_string())?;
		let f = disc.read_sectors(u32_at(&t, SAVE_GAME)? as u64, u32_at(&t, SAVE_GAME + 4)? as u64).map_err(|e| e.to_string())?;
		Save::parse(span(&f, u32_at(&f, 16)? as usize, u32_at(&f, 20)? as usize)?)
	}

	/// The level to load, None for a fresh game (-1 in the default image).
	pub fn level(&self) -> Option<usize> {
		self.global.word(CURRENT_LEVEL).filter(|&l| (l as usize) < LEVELS).map(|l| l as usize)
	}

	pub fn bolts(&self) -> u32 {
		self.global.word(BOLTS).unwrap_or(0)
	}

	/// Autosave state on arriving at `level` (0x24d040): current level, bolts, visited byte.
	pub fn record(&mut self, level: usize, bolts: u32, play_frames: u32) {
		self.global.set_word(CURRENT_LEVEL, level as u32);
		self.global.set_word(BOLTS, bolts);
		self.global.set_word(PLAY_TIME, play_frames);
		self.global.set_bytes(RTC, &local_clock());
		if let Some(r) = self.levels.get_mut(level).and_then(|b| b.0.iter_mut().find(|r| r.0 == VISITED)) {
			if r.1.first() == Some(&0) {
				r.1[0] = 1;
			}
		}
	}
}

/// Global state that outlives a level (decomp/notes/level_transition.md "Unlock persistence"):
/// A14 unlock[20], A20 order[20], A5 story flags, A15 NPC dialogue records, B3004 mission bytes
/// and B3001 visited bytes. Carried across level changes and written to the save card.
#[derive(Clone, Debug, PartialEq)]
pub struct Story {
	pub unlock: [u8; 20],
	pub order: [u32; 20],
	pub a5: [u8; 128],
	pub npc: Vec<u8>,
	pub mission: [[u8; 16]; LEVELS],
	/// B3008 marker crate counts per level (NOV-14.2).
	pub crate_nibbles: [[u8; 16]; LEVELS],
	pub visited: [u8; LEVELS],
	/// A16 help-message slots (148 x 8 bytes) and A17 move counters (36 x 8), game/hints.rs.
	pub help: Vec<u8>,
	pub counters: Vec<u8>,
	/// Inventory records: A9 ammo, A10 owned, A13 quick-select, A32 word 0 = last-chosen weapon
	/// (decomp/notes/savegame.md). `equipped` and `worn` are not saved.
	pub inv: crate::inventory::Inventory,
	/// A11 new/unseen flags (37), A12 vendor stock (12), A30 gold-upgrade bytes (40), A8 skill-point flags (32).
	pub new_flags: [u8; 37],
	pub vendor: [u8; 12],
	pub gold_upgrades: [u8; 40],
	pub skill: [u8; 32],
	/// B3003 gold-bolt collection bytes per level (retail 0x14bec0 + level*4 + n).
	pub gold: [[u8; 4]; LEVELS],
	/// A1005 global deaths and B4002 per-level deaths.
	pub deaths: u32,
	pub level_deaths: [u32; LEVELS],
	/// B3005 persistent UID bitmap per level (retail UID semantics, not placement indices).
	pub uids: [[u8; 256]; LEVELS],
	/// B3006 remembered bolts per level: 64 x {u16 uid+1, u16 bolts} (FUN_0029a480 table, 0x100 bytes).
	pub remembered: [[u8; 256]; LEVELS],
	/// A21..A24 (0x15ed8c..9c): previous main item, wrench-in-hand preference, back-item-3 preference, unknown.
	/// A22/A23 are derived from the live inventory at every save (`prefs`); A21/A24 stay as loaded.
	pub fallback: [u32; 4],
	/// A1004 hero-hit events (global) and B4001 per level, B4000 bolts picked up per level.
	pub hits: u32,
	/// A1003 play ticks (+1 per controller tick, FUN_002ab290; savegame.md SAV-2).
	pub a1003: u32,
	/// A1008 / A1009: the adaptive reward counters (consumer 0x275520, NOV-9).
	pub a1008: i32,
	pub a1009: i32,
	pub level_hits: [u32; LEVELS],
	pub level_bolts: [u32; LEVELS],
	/// B3007 per-planet entry statistics.
	pub planet: [PlanetStat; LEVELS],
	/// Hero deaths per placed-actor mission slot (retail 0x14ee90, 16 words): session only, never saved or cleared by a
	/// level load; read by the class-501 crate gate (NOV-14.7c-gate).
	pub death_slot: [u32; 16],
}

/// B3007 (8 bytes per level): {s16 entry count (-1 = counting off), u16 longest play time / 600, u32 level mask | bit 31}
/// (savegame.md; Novalis OV:47916-47932 at level entry, OV:48090 each frame for the bucket).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanetStat {
	pub count: i16,
	pub minutes: u16,
	pub mask: u32,
}

impl PlanetStat {
	fn from_bytes(d: &[u8]) -> PlanetStat {
		match d {
			[a, b, c, e, f, g, h, i, ..] => PlanetStat { count: i16::from_le_bytes([*a, *b]), minutes: u16::from_le_bytes([*c, *e]), mask: u32::from_le_bytes([*f, *g, *h, *i]) },
			_ => PlanetStat::default(),
		}
	}

	fn to_bytes(self) -> [u8; 8] {
		let mut o = [0; 8];
		o[..2].copy_from_slice(&self.count.to_le_bytes());
		o[2..4].copy_from_slice(&self.minutes.to_le_bytes());
		o[4..].copy_from_slice(&self.mask.to_le_bytes());
		o
	}

	/// The play-time bucket only ever grows (retail maximises every frame).
	pub fn touch(&mut self, frames: u32) {
		self.minutes = self.minutes.max((frames / 600).min(0xffff) as u16);
	}

	/// Arriving on `level`: count + 1 unless -1, bucket, level bit and bit 31.
	pub fn enter(&mut self, level: usize, frames: u32) {
		if self.count != -1 {
			self.count = self.count.wrapping_add(1);
		}
		self.touch(frames);
		self.mask |= 1 << (level & 31) | 0x8000_0000;
	}
}

pub const UNLOCKED: u32 = 14;
pub const UNLOCK_ORDER: u32 = 20;
pub const STORY_FLAGS: u32 = 5;
pub const NPC_RECORDS: u32 = 15;
pub const MISSION_BYTES: u32 = 3004;
/// NOV-14.2 marker crate counts: 16 bytes per level, 32 packed nibbles (retail 0x14bf10 + level * 16).
pub const CRATE_NIBBLES: u32 = 3008;
pub const HELP_SLOTS: u32 = 16;
pub const MOVE_COUNTERS: u32 = 17;
pub const SKILL_FLAGS: u32 = 8;
pub const AMMO: u32 = 9;
pub const OWNED: u32 = 10;
pub const NEW_FLAGS: u32 = 11;
pub const VENDOR_STOCK: u32 = 12;
pub const QUICK_SELECT: u32 = 13;
pub const GOLD_UPGRADES: u32 = 30;
pub const LAST_WEAPONS: u32 = 32;
pub const GOLD_SLOTS: u32 = 3003;
pub const EXPLORED: u32 = 3002;
pub const UID_BITS: u32 = 3005;
pub const REMEMBERED: u32 = 3006;
pub const LEVEL_DEATHS: u32 = 4002;
pub const LEVEL_BOLTS: u32 = 4000;
pub const LEVEL_HITS: u32 = 4001;
pub const PLANET_STATS: u32 = 3007;
/// A21..A24 are consecutive records (previous main item, wrench pref, back-item pref, A24 unknown).
pub const FALLBACK_FIRST: u32 = 21;
pub const AMMO_STATS: [u32; 3] = [1000, 1001, 1002];
pub const PLAY_TICKS: u32 = 1003;
pub const HITS: u32 = 1004;
pub const HELP_LOG_BYTES: u32 = 1010;
pub const HELP_LOG_COUNT: u32 = 1011;
pub const A1008: u32 = 1008;
pub const A1009: u32 = 1009;
/// Options and cheats (table A): camera reversal, camera speed, HelpDesk, subtitles, stereo, volumes.
pub const CHEATS: u32 = 7;
pub const CAMERA_LR: u32 = 25;
pub const CAMERA_UD: u32 = 26;
pub const CAMERA_SPEED: u32 = 27;
pub const HELP_VOICE: u32 = 28;
pub const HELP_TEXT: u32 = 29;
pub const SUBTITLES: u32 = 33;
pub const STEREO: u32 = 34;
pub const MUSIC_VOLUME: u32 = 35;
pub const SFX_VOLUME: u32 = 36;
pub const CHEAT_UNLOCKS: u32 = 37;

impl Default for Story {
	fn default() -> Story {
		Story { unlock: [0; 20], order: [0; 20], a5: [0; 128], npc: vec![0; 1936], mission: [[0; 16]; LEVELS], crate_nibbles: [[0; 16]; LEVELS], visited: [0; LEVELS], help: vec![0; 1184], counters: vec![0; 288], inv: crate::inventory::Inventory::new(), new_flags: [0; 37], vendor: [0; 12], gold_upgrades: [0; 40], skill: [0; 32], gold: [[0; 4]; LEVELS], deaths: 0, level_deaths: [0; LEVELS], uids: [[0; 256]; LEVELS], remembered: [[0; 256]; LEVELS], fallback: [0, 1, 0, 0], hits: 0, a1003: 0, a1008: 0, a1009: 0, level_hits: [0; LEVELS], level_bolts: [0; LEVELS], planet: [PlanetStat::default(); LEVELS], death_slot: [0; 16] }
	}
}

fn fill(r: &mut Vec<u8>, bytes: &[u8]) {
	let n = r.len().min(bytes.len());
	r[..n].copy_from_slice(&bytes[..n]);
}

impl Save {
	/// Reads the persistent story state (a missing record stays default).
	pub fn story(&self) -> Story {
		let mut s = Story::default();
		let g = &self.global;
		if let Some(d) = g.get(UNLOCKED) {
			let n = d.len().min(20);
			s.unlock[..n].copy_from_slice(&d[..n]);
		}
		if let Some(d) = g.get(UNLOCK_ORDER) {
			for (i, w) in d.chunks_exact(4).take(20).enumerate() {
				s.order[i] = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
			}
		}
		if let Some(d) = g.get(STORY_FLAGS) {
			let n = d.len().min(128);
			s.a5[..n].copy_from_slice(&d[..n]);
		}
		if let Some(d) = g.get(NPC_RECORDS) {
			let n = d.len().min(s.npc.len());
			s.npc[..n].copy_from_slice(&d[..n]);
		}
		let words = |id: u32| g.get(id).map(|d| d.chunks_exact(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect::<Vec<u32>>());
		if let Some(w) = words(AMMO) {
			for (a, v) in s.inv.ammo.iter_mut().zip(w) {
				*a = v.min(u16::MAX as u32) as u16;
			}
		}
		if let Some(d) = g.get(OWNED) {
			for (h, v) in s.inv.have.iter_mut().zip(d) {
				*h = *v != 0;
			}
		}
		// Quick-select ids; an id outside the item range (an empty slot) reads as 0 [P: retail sentinel].
		if let Some(w) = words(QUICK_SELECT) {
			for (q, v) in s.inv.quick.iter_mut().zip(w) {
				*q = if (v as usize) < crate::inventory::GADGETS { v as u8 } else { 0 };
			}
		}
		// A32: per equipment slot the last-chosen item (0 none): weapon, feet, head, back; words 4..6 as saved.
		if let Some(w) = words(LAST_WEAPONS) {
			let item = |v: u32| if (v as usize) < crate::inventory::GADGETS { v as u8 } else { 0 };
			s.inv.last = w.first().copied().map_or(0, item);
			for k in 1..4 {
				s.inv.worn[k] = w.get(k).copied().map_or(0, item);
			}
			for k in 0..3 {
				s.inv.sel_tail[k] = w.get(4 + k).copied().unwrap_or(0);
			}
		}
		for (k, id) in AMMO_STATS.iter().enumerate() {
			if let Some(w) = words(*id) {
				for (a, v) in s.inv.stats[k].iter_mut().zip(w) {
					*a = v;
				}
			}
		}
		for (k, f) in s.fallback.iter_mut().enumerate() {
			if let Some(v) = g.word(FALLBACK_FIRST + k as u32) {
				*f = v;
			}
		}
		s.hits = g.word(HITS).unwrap_or(0);
		s.a1003 = g.word(PLAY_TICKS).unwrap_or(0);
		s.a1008 = g.word(A1008).unwrap_or(0) as i32;
		s.a1009 = g.word(A1009).unwrap_or(0) as i32;
		for (dst, id) in [(&mut s.new_flags[..], NEW_FLAGS), (&mut s.vendor[..], VENDOR_STOCK), (&mut s.gold_upgrades[..], GOLD_UPGRADES), (&mut s.skill[..], SKILL_FLAGS)] {
			if let Some(d) = g.get(id) {
				let n = d.len().min(dst.len());
				dst[..n].copy_from_slice(&d[..n]);
			}
		}
		s.deaths = g.word(DEATHS).unwrap_or(0);
		for (dst, id) in [(&mut s.help, HELP_SLOTS), (&mut s.counters, MOVE_COUNTERS)] {
			if let Some(d) = g.get(id) {
				let n = d.len().min(dst.len());
				dst[..n].copy_from_slice(&d[..n]);
			}
		}
		for (l, b) in self.levels.iter().enumerate().take(LEVELS) {
			if let Some(d) = b.get(MISSION_BYTES) {
				let n = d.len().min(16);
				s.mission[l][..n].copy_from_slice(&d[..n]);
			}
			if let Some(d) = b.get(CRATE_NIBBLES) {
				let n = d.len().min(16);
				s.crate_nibbles[l][..n].copy_from_slice(&d[..n]);
			}
			s.visited[l] = b.get(VISITED).and_then(|d| d.first().copied()).unwrap_or(0);
			if let Some(d) = b.get(GOLD_SLOTS) {
				let n = d.len().min(4);
				s.gold[l][..n].copy_from_slice(&d[..n]);
			}
			if let Some(d) = b.get(UID_BITS) {
				let n = d.len().min(256);
				s.uids[l][..n].copy_from_slice(&d[..n]);
			}
			if let Some(d) = b.get(REMEMBERED) {
				let n = d.len().min(256);
				s.remembered[l][..n].copy_from_slice(&d[..n]);
			}
			s.level_deaths[l] = b.word(LEVEL_DEATHS).unwrap_or(0);
			s.level_hits[l] = b.word(LEVEL_HITS).unwrap_or(0);
			s.level_bolts[l] = b.word(LEVEL_BOLTS).unwrap_or(0);
			s.planet[l] = b.get(PLANET_STATS).map_or_else(PlanetStat::default, PlanetStat::from_bytes);
		}
		s
	}

	/// Writes the story state into the records the image already has (same lengths).
	pub fn set_story(&mut self, s: &Story) {
		let order: Vec<u8> = s.order.iter().flat_map(|w| w.to_le_bytes()).collect();
		for (id, bytes) in [(UNLOCKED, &s.unlock[..]), (UNLOCK_ORDER, &order[..]), (STORY_FLAGS, &s.a5[..]), (NPC_RECORDS, &s.npc[..]), (HELP_SLOTS, &s.help[..]), (MOVE_COUNTERS, &s.counters[..])] {
			if let Some(r) = self.global.0.iter_mut().find(|r| r.0 == id) {
				fill(&mut r.1, bytes);
			}
		}
		let le = |it: &mut dyn Iterator<Item = u32>| it.flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>();
		let ammo = le(&mut s.inv.ammo.iter().map(|&a| a as u32));
		let owned: Vec<u8> = s.inv.have.iter().map(|&h| h as u8).collect();
		let quick = le(&mut s.inv.quick.iter().map(|&q| q as u32));
		// A32: last-chosen weapon, worn feet/head/back, then the three words kept as loaded.
		let last = le(&mut [s.inv.last as u32, s.inv.worn[1] as u32, s.inv.worn[2] as u32, s.inv.worn[3] as u32].into_iter().chain(s.inv.sel_tail));
		let fallback = s.prefs();
		let stats: Vec<Vec<u8>> = s.inv.stats.iter().map(|st| le(&mut st.iter().copied())).collect();
		for (id, bytes) in [(AMMO, &ammo[..]), (OWNED, &owned[..]), (QUICK_SELECT, &quick[..]), (LAST_WEAPONS, &last[..]), (NEW_FLAGS, &s.new_flags[..]), (VENDOR_STOCK, &s.vendor[..]), (GOLD_UPGRADES, &s.gold_upgrades[..]), (SKILL_FLAGS, &s.skill[..])] {
			if let Some(r) = self.global.0.iter_mut().find(|r| r.0 == id) {
				fill(&mut r.1, bytes);
			}
		}
		for (k, id) in AMMO_STATS.iter().enumerate() {
			self.global.set_bytes(*id, &stats[k]);
		}
		for (k, v) in fallback.iter().enumerate() {
			self.global.set_word(FALLBACK_FIRST + k as u32, *v);
		}
		self.global.set_word(HITS, s.hits);
		self.global.set_word(PLAY_TICKS, s.a1003);
		self.global.set_word(A1008, s.a1008 as u32);
		self.global.set_word(A1009, s.a1009 as u32);
		self.global.set_word(DEATHS, s.deaths);
		for (l, b) in self.levels.iter_mut().enumerate().take(LEVELS) {
			b.set_word(LEVEL_DEATHS, s.level_deaths[l]);
			b.set_word(LEVEL_HITS, s.level_hits[l]);
			b.set_word(LEVEL_BOLTS, s.level_bolts[l]);
			b.set_bytes(PLANET_STATS, &s.planet[l].to_bytes());
			for (id, bytes) in [(MISSION_BYTES, &s.mission[l][..]), (CRATE_NIBBLES, &s.crate_nibbles[l][..]), (VISITED, &s.visited[l..l + 1]), (GOLD_SLOTS, &s.gold[l][..]), (UID_BITS, &s.uids[l][..]), (REMEMBERED, &s.remembered[l][..])] {
				if let Some(r) = b.0.iter_mut().find(|r| r.0 == id) {
					fill(&mut r.1, bytes);
				}
			}
		}
	}
}

impl Story {
	/// A21..A24 as a save writes them: A22 = the wrench was the last weapon selection (the controller sets it
	/// when the wrench is chosen and clears it for any other weapon, OV:25947-25952), A23 = back item 3 is
	/// the chosen back item (OV:25957-25962; kept while nothing is worn), A21/A24 as loaded.
	pub fn prefs(&self) -> [u32; 4] {
		let mut f = self.fallback;
		f[1] = (self.inv.equipped == crate::inventory::WRENCH) as u32;
		if self.inv.worn[3] != 0 {
			f[2] = (self.inv.worn[3] == 3) as u32;
		}
		f
	}

	/// Brings A22/A23 up to the live inventory (called when the state is snapshotted).
	pub fn refresh_prefs(&mut self) {
		self.fallback = self.prefs();
	}

	/// Level entry (OV:47916-47932): B3007 count, bucket and level bit.
	pub fn enter_level(&mut self, level: usize, play_frames: u32) {
		if let Some(p) = self.planet.get_mut(level) {
			p.enter(level, play_frames);
		}
	}
}

impl Save {
	/// The one encoder behind manual save, autosave and level-transition snapshots: current level,
	/// bolts, play time, the story/inventory records and the level's exploration mask (if a map is up).
	pub fn capture(&mut self, level: usize, bolts: u32, play_frames: u32, story: &Story, explored: Option<(&[u8], &[u8])>) {
		self.record(level, bolts, play_frames);
		self.set_story(story);
		// The play-time bucket of the current planet is maximised each frame in the game; fold it in now.
		if let Some(b) = self.levels.get_mut(level) {
			let mut p = b.get(PLANET_STATS).map_or_else(PlanetStat::default, PlanetStat::from_bytes);
			p.touch(play_frames);
			b.set_bytes(PLANET_STATS, &p.to_bytes());
		}
		if let Some((m, spans)) = explored {
			self.set_explored(level, m, spans);
		}
	}

	/// The saved options and cheat records over `s` (a missing record keeps `s`); RestoreData writes them into
	/// the live globals on every slot load (savegame.md A7, A25..A29, A33..A37). Out-of-range values are
	/// clamped to what the menus can show.
	/// Help Log history from records 1010/1011 (row indices into the help table; savegame.md SAV-2).
	pub fn help_log(&self) -> crate::help::HelpLog {
		let g = &self.global;
		let Some(count) = g.word(HELP_LOG_COUNT) else { return crate::help::HelpLog::default() };
		let bytes = g.get(HELP_LOG_BYTES).unwrap_or(&[]);
		let n = (count as usize).min(150).min(bytes.len());
		crate::help::HelpLog(bytes[..n].iter().filter_map(|&b| crate::help::help_id(b as usize)).collect())
	}

	/// Writes the Help Log into records 1010/1011.
	pub fn set_help_log(&mut self, log: &crate::help::HelpLog) {
		let rows: Vec<u8> = log.0.iter().filter_map(|&h| crate::help::row(h).map(|r| r as u8)).collect();
		let mut buf = [0u8; 150];
		buf[..rows.len()].copy_from_slice(&rows);
		self.global.set_bytes(HELP_LOG_BYTES, &buf);
		self.global.set_word(HELP_LOG_COUNT, rows.len() as u32);
	}

	pub fn options(&self, mut s: Settings) -> Settings {
		let g = &self.global;
		let byte = |id| g.get(id).and_then(|d| d.first().copied());
		if let Some(v) = g.word(CAMERA_LR) {
			s.cam_lr = v != 0;
		}
		if let Some(v) = g.word(CAMERA_UD) {
			s.cam_ud = v != 0;
		}
		if let Some(v) = g.word(CAMERA_SPEED) {
			s.cam_speed = v.min(2) as u8;
		}
		if let Some(v) = byte(HELP_VOICE) {
			s.help_voice = v != 0;
		}
		if let Some(v) = byte(HELP_TEXT) {
			s.help_text = v != 0;
		}
		if let Some(v) = byte(SUBTITLES) {
			s.subtitles = v != 0;
		}
		if let Some(v) = g.word(STEREO) {
			s.stereo = v != 0;
		}
		if let Some(v) = g.word(MUSIC_VOLUME) {
			s.music = v.min(VOLUME_MAX as u32) as u16;
		}
		if let Some(v) = g.word(SFX_VOLUME) {
			s.sfx = v.min(VOLUME_MAX as u32) as u16;
		}
		for (dst, id) in [(&mut s.cheats, CHEATS), (&mut s.cheat_unlocks, CHEAT_UNLOCKS)] {
			if let Some(d) = g.get(id) {
				let n = d.len().min(12);
				dst[..n].copy_from_slice(&d[..n]);
			}
		}
		s
	}

	pub fn set_options(&mut self, s: &Settings) {
		let g = &mut self.global;
		for (id, v) in [(CAMERA_LR, s.cam_lr as u32), (CAMERA_UD, s.cam_ud as u32), (CAMERA_SPEED, s.cam_speed as u32), (STEREO, s.stereo as u32), (MUSIC_VOLUME, s.music as u32), (SFX_VOLUME, s.sfx as u32)] {
			g.set_word(id, v);
		}
		for (id, v) in [(HELP_VOICE, s.help_voice), (HELP_TEXT, s.help_text), (SUBTITLES, s.subtitles)] {
			g.set_bytes(id, &[v as u8]);
		}
		g.set_bytes(CHEATS, &s.cheats);
		g.set_bytes(CHEAT_UNLOCKS, &s.cheat_unlocks);
	}

	/// B3002 of `level` decoded to the 512x512-bit mask (zero blob = nothing explored).
	pub fn explored(&self, level: usize, spans: &[u8]) -> Vec<u8> {
		explore_decode(self.levels.get(level).and_then(|b| b.get(EXPLORED)).unwrap_or(&[]), spans)
	}

	pub fn set_explored(&mut self, level: usize, mask: &[u8], spans: &[u8]) {
		let blob = explore_encode(mask, spans);
		if let Some(r) = self.levels.get_mut(level).and_then(|b| b.0.iter_mut().find(|r| r.0 == EXPLORED)) {
			fill(&mut r.1, &blob);
		}
	}
}

const MASK_BYTES: usize = 0x8000;
const MASK_PIXELS: usize = 0x40000;
const BLOB: usize = 0x800;


/// FUN_00221e58: bit runs over the span pixels; byte 0 = first run (of 1s, may be 0) * 2 + 1, then
/// alternating runs <= 255 (a 0 run pads a longer one). None = the stream would not fit in 0x800 bytes.
fn explore_rle(mask: &[u8], spans: &[u8]) -> Option<Vec<u8>> {
	let mut out: Vec<u8> = Vec::new();
	let (mut run, mut max, mut pos, mut left, mut bit) = (0u32, 0x7fu32, 0usize, 0u32, 1u8);
	let mut sp = spans.chunks_exact(2);
	loop {
		while left > 0 {
			let b = mask.get(pos >> 3).map_or(0, |v| v >> (pos & 7) & 1);
			pos += 1;
			left -= 1;
			if b == bit {
				if run == max {
					out.push(run as u8);
					if out.len() + 1 >= BLOB {
						return None;
					}
					out.push(0);
					run = 1;
					max = 0xff;
				} else {
					run += 1;
				}
			} else {
				out.push(run as u8);
				max = 0xff;
				if out.len() >= BLOB {
					return None;
				}
				run = 1;
				bit = b;
			}
		}
		let p = sp.next()?;
		pos += p[0] as usize;
		left = p[1] as u32;
		if pos == MASK_PIXELS {
			break;
		}
	}
	if run != 0 {
		out.push(run as u8);
	}
	let first = *out.first()?;
	out[0] = first.wrapping_mul(2) + 1;
	(out.len() <= BLOB).then_some(out)
}

/// FUN_0025df50: 4x4 reduce, a block is set when >= 8 of its 16 pixels are (nibble popcount table
/// DAT_00209a90 [C]); output bit c of row r*16 + c/8, byte 0 = (byte 0 & 0xfe) | 2.
fn explore_reduce(mask: &[u8]) -> Vec<u8> {
	let mut out = vec![0u8; BLOB];
	for r in 0..128 {
		for c in 0..128 {
			let n: u32 = (0..4).map(|dy| ((mask.get((r * 4 + dy) * 64 + c / 2).copied().unwrap_or(0) >> ((c & 1) * 4)) & 0xf).count_ones()).sum();
			if n >= 8 {
				out[r * 16 + c / 8] |= 1 << (c % 8);
			}
		}
	}
	out[0] = out[0] & 0xfe | 2;
	out
}

/// FUN_0025da28: the RLE stream, else the 4x4 fallback; always 0x800 bytes.
pub fn explore_encode(mask: &[u8], spans: &[u8]) -> Vec<u8> {
	let mut b = explore_rle(mask, spans).unwrap_or_else(|| explore_reduce(mask));
	b.resize(BLOB, 0);
	b
}

/// FUN_0025dad0: low bit of byte 0 set = RLE (FUN_0025db48), clear = 4x expansion (FUN_0025dd78).
pub fn explore_decode(blob: &[u8], spans: &[u8]) -> Vec<u8> {
	let mut mask = vec![0u8; MASK_BYTES];
	if blob.len() < BLOB {
		return mask;
	}
	if blob[0] & 1 == 0 {
		for r in 0..128 {
			for c in 0..128 {
				if blob[r * 16 + c / 8] >> (c % 8) & 1 != 0 {
					for dy in 0..4 {
						mask[(r * 4 + dy) * 64 + c / 2] |= 0xf << ((c & 1) * 4);
					}
				}
			}
		}
		return mask;
	}
	let (mut at, mut left, mut bit, mut pos) = (1usize, (blob[0] >> 1) as u32, true, 0usize);
	for p in spans.chunks_exact(2) {
		pos += p[0] as usize;
		if pos >= MASK_PIXELS {
			break;
		}
		for _ in 0..p[1] {
			while left == 0 {
				if at >= BLOB {
					(left, bit) = (u32::MAX, false);
					break;
				}
				left = blob[at] as u32;
				bit = !bit;
				at += 1;
			}
			if bit && pos < MASK_PIXELS {
				mask[pos >> 3] |= 1 << (pos & 7);
			}
			left -= 1;
			pos += 1;
		}
	}
	mask
}

/// (class, placement index, UID bit) of the instances whose retail UID can set a bit (0..0x800;
/// negative UIDs never do). `Actor.uid` holds the placement index, the bitmap the raw UID.
pub fn uid_map(inst: &[crate::gameplay::MobyInstance]) -> Vec<(u32, u32, usize)> {
	inst.iter().filter_map(|m| m.uid_bit().map(|b| (m.class_id, m.index, b))).collect()
}

/// FUN_0029a480: the B3006 slot of `uid` in one level's table (64 x {u16 uid + 1, u16 bolts}). The scan runs from
/// entry 63 down; the first entry holding `uid + 1` or empty is taken (the remembered half is kept). -1 when the UID
/// is negative or the table is full.
pub fn find_or_claim(table: &mut [u8; 256], uid: i32) -> i8 {
	if uid < 0 {
		return -1;
	}
	let key = (uid + 1) as u16;
	for k in (0..64).rev() {
		let cur = u16::from_le_bytes([table[4 * k], table[4 * k + 1]]);
		if cur == key || cur == 0 {
			table[4 * k..4 * k + 2].copy_from_slice(&key.to_le_bytes());
			return k as i8;
		}
	}
	-1
}


/// Every placement whose UID bit is set (UIDs repeat, so both placements of a UID come back).
pub fn uid_saved(bits: &[u8; 256], map: &[(u32, u32, usize)]) -> std::collections::HashSet<(u32, u32)> {
	map.iter().filter(|m| bits[m.2 >> 3] >> (m.2 & 7) & 1 != 0).map(|m| (m.0, m.1)).collect()
}

/// The slot list row: level, bolts and play time (0x13d2b0 + slot*0x1c).
#[derive(Clone, Copy, Debug)]
pub struct Summary {
	pub level: usize,
	pub bolts: u32,
	pub play_frames: u32,
	/// Month, day, year bytes (BCD) of the clock record, printed as "%02x/%02x/%02x".
	pub date: [u8; 3],
}

impl Summary {
	pub fn date_text(&self) -> String {
		format!("{:02x}/{:02x}/{:02x}", self.date[0], self.date[1], self.date[2])
	}
}

/// One frame of the play timer A3 (counted in 1/60 s units with the PAL correction): +1, and +11
/// instead when the incremented value is a multiple of 50.
pub fn tick_play(a3: u32) -> u32 {
	let n = a3.wrapping_add(1) as i32;
	if n % 50 == 0 {
		a3.wrapping_add(11)
	} else {
		a3.wrapping_add(1)
	}
}

fn bcd(v: u32) -> u8 {
	(((v / 10) << 4) | (v % 10)) as u8
}

/// The clock record bytes {0, sec, min, hour, 0, day, month, year % 100}, each BCD.
pub fn clock_bytes(year: u32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> [u8; 8] {
	[0, bcd(sec), bcd(min), bcd(hour), 0, bcd(day), bcd(month), bcd(year % 100)]
}

#[cfg(windows)]
fn local_clock() -> [u8; 8] {
	#[repr(C)]
	#[derive(Default)]
	struct SystemTime {
		year: u16,
		month: u16,
		weekday: u16,
		day: u16,
		hour: u16,
		minute: u16,
		second: u16,
		millis: u16,
	}
	unsafe extern "system" {
		fn GetLocalTime(t: *mut SystemTime);
	}
	let mut t = SystemTime::default();
	// SAFETY: GetLocalTime fills the caller's SYSTEMTIME-shaped struct and has no other effect.
	unsafe { GetLocalTime(&mut t) };
	clock_bytes(t.year as u32, t.month as u32, t.day as u32, t.hour as u32, t.minute as u32, t.second as u32)
}

#[cfg(not(windows))]
fn local_clock() -> [u8; 8] {
	let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
	let (days, rem) = ((secs / 86400) as i64, (secs % 86400) as u32);
	// Days since 1970-01-01 to a civil date.
	let z = days + 719468;
	let era = z.div_euclid(146097);
	let doe = z.rem_euclid(146097);
	let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
	let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
	let year = (yoe + era * 400 + i64::from(month <= 2)) as u32;
	clock_bytes(year, month, day, rem / 3600, rem / 60 % 60, rem % 60)
}

pub fn slot_path(slot: usize) -> PathBuf {
	let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
	dir.join("saves").join(format!("slot{slot}.bin"))
}

pub fn load_slot(slot: usize) -> Option<Save> {
	Save::parse(&std::fs::read(slot_path(slot)).ok()?).ok()
}

pub fn write_slot(slot: usize, save: &Save) -> std::io::Result<()> {
	write_whole(&slot_path(slot), save)
}

/// Manual save / new game (command 0x13): the whole image.
pub fn write_whole(p: &Path, save: &Save) -> std::io::Result<()> {
	if let Some(d) = p.parent() {
		std::fs::create_dir_all(d)?;
	}
	std::fs::write(p, save.bytes())
}

/// Autosave into the active slot (command 0xf, state 0x10; savegame.md): global A is rewritten in place at
/// byte 8, then only the source planet's B block, with no temporary file or rollback. Every other block stays as
/// the card holds it. A missing or differently sized file gets the whole image instead.
pub fn write_autosave(slot: usize, save: &Save, source: usize) -> std::io::Result<()> {
	write_autosave_to(&slot_path(slot), save, source)
}

pub fn write_autosave_to(p: &Path, save: &Save, source: usize) -> std::io::Result<()> {
	use std::io::{Read, Seek, SeekFrom, Write};
	let img = save.bytes();
	let (a, b) = (u32_at(&img, 0).unwrap_or(0) as usize, u32_at(&img, 4).unwrap_or(0) as usize);
	let Ok(mut f) = std::fs::OpenOptions::new().read(true).write(true).open(p) else { return write_whole(p, save) };
	let mut head = [0u8; 8];
	if f.metadata()?.len() as usize != img.len() || f.read_exact(&mut head).is_err() || head != img[..8] || source >= LEVELS {
		drop(f);
		return write_whole(p, save);
	}
	f.seek(SeekFrom::Start(8))?;
	f.write_all(&img[8..8 + a])?;
	let at = 8 + a + source * b;
	f.seek(SeekFrom::Start(at as u64))?;
	f.write_all(&img[at..at + b])?;
	f.flush()
}

/// Save-anywhere (a rac-rs extension [PC] port choice: the game stores no position and a load starts at the level's
/// start, ingame_save_quit.md s4): where the hero stood at a manual save, kept beside `slotN.bin`.
#[derive(Clone, Debug, PartialEq)]
pub struct Where {
	pub level: usize,
	pub pos: [f32; 3],
	pub yaw: f32,
	pub health: i32,
	pub moves: u32,
	/// The cutscene controller's next record (-1 = finished, None = no controller), the help slots
	/// seen (flow.rs) and the Help Log ids (help.rs `HelpLog`).
	pub record: Option<i32>,
	pub seen: Vec<i16>,
	pub requested: Vec<u32>,
	pub inv: crate::inventory::Inventory,
}

fn where_path(slot: usize) -> PathBuf {
	slot_path(slot).with_extension("where")
}

impl Where {
	pub fn to_text(&self) -> String {
		let list = |v: &mut dyn Iterator<Item = String>| v.collect::<Vec<_>>().join(" ");
		let i = &self.inv;
		[
			format!("level {}", self.level),
			format!("pos {} {} {} {}", self.pos[0], self.pos[1], self.pos[2], self.yaw),
			format!("health {}", self.health),
			format!("moves {}", self.moves),
			format!("record {}", self.record.map_or("none".to_string(), |r| r.to_string())),
			format!("seen {}", list(&mut self.seen.iter().map(|x| x.to_string()))),
			format!("requested {}", list(&mut self.requested.iter().map(|x| x.to_string()))),
			format!("have {}", list(&mut i.have.iter().map(|&b| (b as u8).to_string()))),
			format!("ammo {}", list(&mut i.ammo.iter().map(|x| x.to_string()))),
			format!("quick {}", list(&mut i.quick.iter().map(|x| x.to_string()))),
			format!("hand {} {}", i.equipped, i.last),
			format!("worn {}", list(&mut i.worn.iter().map(|x| x.to_string()))),
		]
		.join("
")
			+ "
"
	}

	pub fn parse(t: &str) -> Option<Where> {
		let mut w = Where { level: 0, pos: [0.0; 3], yaw: 0.0, health: 0, moves: 0, record: None, seen: Vec::new(), requested: Vec::new(), inv: crate::inventory::Inventory::new() };
		let mut got = 0;
		for line in t.lines() {
			let mut it = line.split_whitespace();
			let key = it.next()?;
			let v: Vec<&str> = it.collect();
			let nums = |v: &[&str]| v.iter().map(|x| x.parse::<f64>().ok()).collect::<Option<Vec<f64>>>();
			match key {
				"level" => w.level = v.first()?.parse().ok()?,
				"pos" => {
					let n = nums(&v)?;
					(w.pos, w.yaw) = ([*n.first()? as f32, *n.get(1)? as f32, *n.get(2)? as f32], *n.get(3)? as f32);
					got |= 1;
				}
				"health" => w.health = v.first()?.parse().ok()?,
				"moves" => w.moves = v.first()?.parse().ok()?,
				"record" => w.record = v.first().and_then(|x| x.parse().ok()),
				"seen" => w.seen = v.iter().filter_map(|x| x.parse().ok()).collect(),
				"requested" => w.requested = v.iter().filter_map(|x| x.parse().ok()).collect(),
				"have" => v.iter().zip(w.inv.have.iter_mut()).for_each(|(x, h)| *h = *x == "1"),
				"ammo" => v.iter().zip(w.inv.ammo.iter_mut()).for_each(|(x, a)| *a = x.parse().unwrap_or(0)),
				"worn" => v.iter().zip(w.inv.worn.iter_mut()).for_each(|(x, q)| *q = x.parse().unwrap_or(0)),
				"quick" => v.iter().zip(w.inv.quick.iter_mut()).for_each(|(x, q)| *q = x.parse().unwrap_or(0)),
				"hand" => {
					w.inv.equipped = v.first()?.parse().ok()?;
					w.inv.last = v.get(1)?.parse().ok()?;
				}
				_ => {}
			}
		}
		(got == 1).then_some(w)
	}
}

pub fn load_where(slot: usize) -> Option<Where> {
	Where::parse(&std::fs::read_to_string(where_path(slot)).ok()?)
}

/// Written by a manual save; None (an autosave) removes it so a load matches the slot image.
pub fn write_where(slot: usize, w: Option<&Where>) -> std::io::Result<()> {
	match w {
		Some(w) => std::fs::write(where_path(slot), w.to_text()),
		None => match std::fs::remove_file(where_path(slot)) {
			Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
			_ => Ok(()),
		},
	}
}

/// The five rows of the slot list; None = empty.
pub fn summaries() -> [Option<Summary>; SLOTS] {
	std::array::from_fn(|i| {
		let s = load_slot(i)?;
		let rtc = s.global.get(RTC).filter(|r| r.len() >= 8).map_or([0; 3], |r| [r[6], r[5], r[7]]);
		Some(Summary { level: s.level().unwrap_or(0), bolts: s.bolts(), play_frames: s.global.word(PLAY_TIME).unwrap_or(0), date: rtc })
	})
}

/// What can ask for an autosave (decomp/notes/ingame_save_quit.md s6, FUN_0024d040 callers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
	LevelChange,
	DeathRetry,
	/// A boss fight or cutscene ending.
	SceneEnd,
	/// No caller: the game never saves on pause or on quit.
	Pause,
	Quit,
	/// Game::save_request (FUN_00260fe0(0,-1), e.g. a gold bolt pickup).
	Request,
}

/// FUN_0024d040's gate: a save trigger, an active slot and autosave enabled ([0x13d384]). The
/// scene/boss callers are class 750 and 1290 hooks at specific points, neither placed on Veldin
/// (ingame_save_quit.md s6), so a cutscene ending does not save here.
pub fn autosave_due(t: Trigger, slot: Option<usize>, enabled: bool) -> bool {
	matches!(t, Trigger::LevelChange | Trigger::DeathRetry | Trigger::Request) && slot.is_some() && enabled
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn autosave_table() {
		use Trigger::*;
		for (t, due) in [(LevelChange, true), (DeathRetry, true), (SceneEnd, false), (Pause, false), (Quit, false), (Request, true)] {
			assert_eq!(autosave_due(t, Some(2), true), due, "{t:?}");
			assert!(!autosave_due(t, None, true));
			assert!(!autosave_due(t, Some(2), false));
		}
	}

	#[test]
	fn default_image_round_trips() {
		let mut disc = Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let t = toc::read_toc(&mut disc).unwrap();
		let f = disc.read_sectors(u32_at(&t, SAVE_GAME).unwrap() as u64, u32_at(&t, SAVE_GAME + 4).unwrap() as u64).unwrap();
		let raw = span(&f, 0x8534, 0xea08).unwrap();
		let s = Save::default_image(&mut disc).unwrap();
		assert_eq!((s.global.0.len(), s.levels.len(), s.global.0[0].0), (47, 20, 0));
		assert_eq!(s.level(), None);
		assert_eq!(s.bytes(), raw);
		let mut m = s.clone();
		m.record(0, 1234, 600);
		let back = Save::parse(&m.bytes()).unwrap();
		assert_eq!((back.level(), back.bolts(), back.levels[0].get(VISITED).map(|v| v[0])), (Some(0), 1234, Some(1)));
		assert_eq!(m.bytes().len(), 59912);
	}

	#[test]
	fn help_log_and_play_ticks_round_trip() {
		let mut sv = Save { global: Block(vec![(PLAY_TICKS, vec![0; 4]), (HELP_LOG_BYTES, vec![0; 150]), (HELP_LOG_COUNT, vec![0; 4])]), levels: vec![] };
		let mut st = Story::default();
		st.a1003 = 12345;
		sv.set_story(&st);
		assert_eq!(sv.story().a1003, 12345);
		let log = crate::help::HelpLog(vec![3, 2000, 20011]);
		sv.set_help_log(&log);
		assert_eq!(&sv.global.get(HELP_LOG_BYTES).unwrap()[..4], &[0, 10, 117, 0]);
		assert_eq!(sv.global.word(HELP_LOG_COUNT), Some(3));
		assert_eq!(sv.help_log(), log);
	}

	#[test]
	fn story_round_trips() {
		let mk = |id: u32, n: usize| (id, vec![0u8; n]);
		let blk = |ids: &[(u32, usize)]| Block(ids.iter().map(|&(i, n)| mk(i, n)).collect());
		let mut sv = Save { global: blk(&[(UNLOCKED, 20), (UNLOCK_ORDER, 80), (STORY_FLAGS, 128), (NPC_RECORDS, 1936)]), levels: (0..LEVELS).map(|_| blk(&[(VISITED, 1), (MISSION_BYTES, 16)])).collect() };
		let mut s = Story::default();
		s.unlock[1] = 1;
		s.unlock[3] = 1;
		s.order[0] = 1;
		s.order[1] = 3;
		s.a5[0xF] = 1;
		s.npc[0xC + 48] = 1;
		s.mission[1][0] = 0xff;
		s.visited[1] = 1;
		sv.set_story(&s);
		let back = Save::parse(&sv.bytes()).unwrap();
		assert_eq!(back.story(), s);
	}

	fn disc_image() -> Save {
		let mut disc = Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		Save::default_image(&mut disc).unwrap()
	}

	/// A span table over pixel ranges (start, end), skips split at 255.
	fn region(ranges: &[(usize, usize)]) -> Vec<u8> {
		let (mut v, mut at) = (Vec::new(), 0usize);
		for &(s, e) in ranges {
			let mut skip = s - at;
			while skip > 255 {
				v.extend([255, 0]);
				skip -= 255;
			}
			let mut p = s;
			let mut first = true;
			while p < e {
				let n = (e - p).min(255);
				v.extend([if first { skip as u8 } else { 0 }, n as u8]);
				first = false;
				p += n;
			}
			at = e;
		}
		let mut skip = MASK_PIXELS - at;
		while skip > 255 {
			v.extend([255, 0]);
			skip -= 255;
		}
		v.extend([skip as u8, 0]);
		v
	}

	fn disc(m: &mut [u8], cx: usize, cy: usize) {
		for y in cy.saturating_sub(16)..(cy + 16).min(512) {
			for x in cx.saturating_sub(16)..(cx + 16).min(512) {
				if (x as i32 - cx as i32).pow(2) + (y as i32 - cy as i32).pow(2) <= 256 {
					m[(y * 512 + x) >> 3] |= 1 << (x & 7);
				}
			}
		}
	}

	#[test]
	fn bolts_live_in_a1_and_clock_in_a4() {
		let mut sv = disc_image();
		sv.record(2, 1234, 0);
		assert_eq!(sv.global.word(1), Some(1234));
		assert_ne!(sv.global.word(4), Some(1234));
		assert_eq!(clock_bytes(2026, 10, 5, 14, 7, 9), [0, 0x09, 0x07, 0x14, 0, 0x05, 0x10, 0x26]);
		let c = clock_bytes(2026, 10, 5, 14, 7, 9);
		let sm = Summary { level: 0, bolts: 0, play_frames: 0, date: [c[6], c[5], c[7]] };
		assert_eq!(sm.date_text(), "10/05/26");
	}

	#[test]
	fn play_timer_steps_with_pal_jump() {
		let run = |n: u32| (0..n).fold(0, |a, _| tick_play(a));
		assert_eq!((run(49), run(50), run(89), run(90), run(130)), (49, 60, 99, 110, 160));
	}

	#[test]
	fn reward_counters_round_trip() {
		let mut sv = disc_image();
		let mut s = sv.story();
		assert_eq!((s.a1008, s.a1009), (0, 0));
		s.a1008 = 7;
		s.a1009 = 300;
		sv.set_story(&s);
		let b = sv.story();
		assert_eq!((b.a1008, b.a1009), (7, 300));
	}

	#[test]
	fn explore_nibble_popcount_table() {
		let table = [0u32, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4];
		for (n, &t) in table.iter().enumerate() {
			assert_eq!((n as u8).count_ones(), t);
		}
	}

	#[test]
	fn all_records_round_trip_on_default_image() {
		let mut sv = disc_image();
		let before = sv.bytes();
		let mut s = sv.story();
		s.inv.have[10] = true;
		s.inv.have[3] = true;
		s.inv.ammo[10] = 33;
		s.inv.quick = [10, 3, 0, 0, 0, 0, 0, 0];
		s.inv.last = 10;
		s.new_flags[3] = 1;
		s.vendor[2] = 7;
		s.gold_upgrades[10] = 1;
		s.skill[0] = 1;
		s.skill[31] = 1;
		s.gold[1][2] = 1;
		s.deaths = 9;
		s.level_deaths[1] = 4;
		s.uids[1][5] = 0x81;
		s.mission[1][3] = 0xff;
		s.refresh_prefs();
		s.planet[1].minutes = 1; // capture folds the play-time bucket
		let mut mask = vec![0u8; MASK_BYTES];
		mask[100] = 0xf0;
		let reg = region(&[(0, 64 * 512)]);
		sv.capture(1, 777, 600, &s, Some((&mask, &reg)));
		let bytes = sv.bytes();
		assert_eq!(bytes.len(), 59912);
		let back = Save::parse(&bytes).unwrap();
		assert_eq!(back.story(), s);
		assert_eq!(back.explored(1, &reg), mask);
		assert_eq!(back.bolts(), 777);
		// Records rac-rs does not model are untouched: only capture's own records differ.
		let old = Save::parse(&before).unwrap();
		for (a, b) in old.global.0.iter().zip(&back.global.0) {
			if ![0, 1, 3, 4, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 20, 21, 22, 23, 24, 30, 32, 1000, 1001, 1002, 1004, 1005].contains(&a.0) {
				assert_eq!(a, b, "A{}", a.0);
			}
		}
		assert_eq!(old.global.get(32).unwrap()[4..], back.global.get(32).unwrap()[4..]);
		assert_eq!(old.levels[1].get(3006), back.levels[1].get(3006));
		assert_eq!(old.levels[1].0.len(), back.levels[1].0.len());
	}

	#[test]
	fn options_and_cheats_round_trip() {
		let mut sv = disc_image();
		let d = sv.options(Settings::default());
		// The new-game image is the settings default (camera normal, medium, HelpDesk on, music 716, SFX 1024).
		assert_eq!(d, Settings::default());
		let s = Settings { sfx: 17, music: 900, stereo: false, cam_lr: false, cam_ud: false, cam_speed: 2, subtitles: true, help_voice: false, help_text: false, cheats: [1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 9], cheat_unlocks: [5; 12], ..Settings::default() };
		sv.set_options(&s);
		let back = Save::parse(&sv.bytes()).unwrap();
		// Port-only extras are not in the image: the base settings keep them.
		let got = back.options(Settings { lang: 3, hz60: true, cam_modern: true, ..Settings::default() });
		assert_eq!(got, Settings { lang: 3, hz60: true, cam_modern: true, ..s });
		// Record widths stay as the game has them (1 byte for HelpDesk/subtitles, 4 for the rest).
		assert_eq!([HELP_VOICE, HELP_TEXT, SUBTITLES, STEREO, SFX_VOLUME].map(|id| back.global.get(id).unwrap().len()), [1, 1, 1, 4, 4]);
		// A new game keeps only music and SFX around the defaults.
		let n = got.new_game();
		assert_eq!((n.sfx, n.music, n.cam_lr, n.cheats, n.lang), (17, 900, true, [0; 12], 3));
	}

	#[test]
	fn autosave_rewrites_global_and_source_block_only() {
		let mut sv = disc_image();
		let p = std::env::temp_dir().join(format!("rac_autosave_{}.bin", std::process::id()));
		write_whole(&p, &sv).unwrap();
		let first = std::fs::read(&p).unwrap();
		let (a, b) = (u32_at(&first, 0).unwrap() as usize, u32_at(&first, 4).unwrap() as usize);
		let mut s = sv.story();
		s.deaths = 4;
		s.level_deaths[1] = 2;
		s.level_deaths[2] = 6;
		sv.capture(2, 55, 600, &s, None);
		// Arrival at 2 from 1: global A (current level 2) and B[1] change; B[2] stays as the card had it.
		write_autosave_to(&p, &sv, 1).unwrap();
		let got = std::fs::read(&p).unwrap();
		let new = sv.bytes();
		assert_eq!(got.len(), first.len());
		assert_eq!(got[..8 + a], new[..8 + a]);
		let blk = |v: &[u8], l: usize| v[8 + a + l * b..8 + a + (l + 1) * b].to_vec();
		assert_eq!(blk(&got, 1), blk(&new, 1));
		assert_eq!(blk(&got, 2), blk(&first, 2));
		assert_ne!(blk(&got, 2), blk(&new, 2));
		let back = Save::parse(&got).unwrap();
		assert_eq!((back.level(), back.bolts(), back.story().level_deaths), (Some(2), 55, {
			let mut d = [0; LEVELS];
			d[1] = 2;
			d
		}));
		// A missing or resized file is replaced by the whole image.
		std::fs::write(&p, [0u8; 10]).unwrap();
		write_autosave_to(&p, &sv, 1).unwrap();
		assert_eq!(std::fs::read(&p).unwrap(), new);
		std::fs::remove_file(&p).unwrap();
		write_autosave_to(&p, &sv, 0).unwrap();
		assert_eq!(std::fs::read(&p).unwrap(), new);
		let _ = std::fs::remove_file(&p);
	}

	#[test]
	fn restore_before_construction() {
		let mut sv = disc_image();
		let mut s = sv.story();
		s.inv.unlock(10, true);
		s.inv.ammo[10] = 25;
		s.inv.quick[1] = 10;
		s.inv.last = 10;
		s.mission[0][2] = 0xff;
		s.skill[0] = 1;
		s.gold[0][1] = 1;
		sv.capture(0, 5, 0, &s, None);
		let fresh = Save::parse(&sv.bytes()).unwrap().story();
		let g = crate::game::Game::with_story(&[], &std::collections::HashMap::new(), ([0.0; 3], 0.0), -100.0, fresh, 0);
		assert!(g.inv.have[10] && g.inv.ammo[10] == 25 && g.inv.quick[1] == 10 && g.inv.equipped == 10);
		assert_eq!(g.story.mission[0][2], 0xff);
		assert!(g.skill_points[0] == 1 && g.gold_slots[0][1] == 1);
		assert_eq!(g.persist().inv.ammo[10], 25);
	}

	#[test]
	fn remembered_slots_claim_from_the_top_and_round_trip() {
		let mut t = [0u8; 256];
		assert_eq!(find_or_claim(&mut t, 5), 63);
		assert_eq!(u16::from_le_bytes([t[252], t[253]]), 6);
		assert_eq!(find_or_claim(&mut t, 5), 63);
		assert_eq!(find_or_claim(&mut t, 9), 62);
		assert_eq!(find_or_claim(&mut t, -1), -1);
		let mut s = Story::default();
		s.remembered[1][3] = 7;
		let mut sv = disc_image();
		sv.capture(1, 10, 6000, &s, None);
		assert_eq!(Save::parse(&sv.bytes()).unwrap().story().remembered[1][3], 7);
	}

	/// ARI-9 (decomp/notes/savegame.md): Aridia's records round-trip through the shared tables, other levels untouched.
	#[test]
	fn aridia_records_round_trip() {
		let mut s = Story::default();
		s.skill[1] = 1;
		s.skill[2] = 1;
		s.gold[2] = [1, 0, 1, 0];
		s.a5[0x14] = 1;
		s.a5[0x15] = 2;
		s.a5[0] = 1;
		s.crate_nibbles[2][3] = 0x21;
		s.new_flags[26] = 1;
		s.remembered[2][..4].copy_from_slice(&[0x01, 0x01, 25, 0]);
		let mut sv = disc_image();
		sv.capture(2, 10, 6000, &s, None);
		let back = Save::parse(&sv.bytes()).unwrap().story();
		assert_eq!((back.skill, back.a5, back.new_flags), (s.skill, s.a5, s.new_flags));
		assert_eq!((back.gold, back.crate_nibbles, back.remembered), (s.gold, s.crate_nibbles, s.remembered));
		assert_eq!(u16::from_le_bytes([back.remembered[2][0], back.remembered[2][1]]), 0x0101);
		assert_eq!(back.remembered[2][2], 25);
		let fresh = Story::default();
		assert!((0..LEVELS).filter(|&k| k != 2).all(|k| back.gold[k] == fresh.gold[k] && back.crate_nibbles[k] == fresh.crate_nibbles[k] && back.remembered[k] == fresh.remembered[k]));
	}

	#[test]
	fn where_round_trips() {
		let mut inv = crate::inventory::Inventory::new();
		inv.have[3] = true;
		inv.ammo[3] = 17;
		inv.quick[1] = 3;
		inv.equipped = 3;
		inv.worn[2] = 7;
		let w = Where { level: 0, pos: [1.5, -2.25, 30.0], yaw: 0.5, health: 3, moves: 5, record: Some(-1), seen: vec![4, 7], requested: vec![20000, 2], inv };
		assert_eq!(Where::parse(&w.to_text()), Some(w));
	}
}
