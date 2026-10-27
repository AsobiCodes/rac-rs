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

pub const DEATHS: u32 = 1005;
/// Per-level record ids (table B at 0x1848c0).
pub const VISITED: u32 = 3001;

#[derive(Clone, Debug, PartialEq)]
pub struct Block(pub Vec<(u32, Vec<u8>)>);

impl Block {

	pub fn get(&self, id: u32) -> Option<&[u8]> {
		self.0.iter().find(|r| r.0 == id).map(|r| r.1.as_slice())
	}

	pub fn word(&self, id: u32) -> Option<u32> {
		self.get(id).and_then(|d| u32_at(d, 0).ok())
	}

}

#[derive(Clone, Debug, PartialEq)]
pub struct Save {
	pub global: Block,
	pub levels: Vec<Block>,
}

impl Save {

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
pub const A1008: u32 = 1008;
pub const A1009: u32 = 1009;

impl Default for Story {
	fn default() -> Story {
		Story { unlock: [0; 20], order: [0; 20], a5: [0; 128], npc: vec![0; 1936], mission: [[0; 16]; LEVELS], crate_nibbles: [[0; 16]; LEVELS], visited: [0; LEVELS], help: vec![0; 1184], counters: vec![0; 288], inv: crate::inventory::Inventory::new(), new_flags: [0; 37], vendor: [0; 12], gold_upgrades: [0; 40], skill: [0; 32], gold: [[0; 4]; LEVELS], deaths: 0, level_deaths: [0; LEVELS], uids: [[0; 256]; LEVELS], remembered: [[0; 256]; LEVELS], fallback: [0, 1, 0, 0], hits: 0, a1003: 0, a1008: 0, a1009: 0, level_hits: [0; LEVELS], level_bolts: [0; LEVELS], planet: [PlanetStat::default(); LEVELS], death_slot: [0; 16] }
	}
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

	/// B3002 of `level` decoded to the 512x512-bit mask (zero blob = nothing explored).
	pub fn explored(&self, level: usize, spans: &[u8]) -> Vec<u8> {
		explore_decode(self.levels.get(level).and_then(|b| b.get(EXPLORED)).unwrap_or(&[]), spans)
	}

}

const MASK_BYTES: usize = 0x8000;
const MASK_PIXELS: usize = 0x40000;
const BLOB: usize = 0x800;


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
	/// Month, day, year bytes (BCD) of the clock record, printed as day/month/year.
	pub date: [u8; 3],
}

impl Summary {
	/// The date as day, month and year, two digits each.
	pub fn date_text(&self) -> String {
		format!("{:02x}/{:02x}/{:02x}", self.date[1], self.date[0], self.date[2])
	}
}

fn bcd(v: u32) -> u8 {
	(((v / 10) << 4) | (v % 10)) as u8
}

/// The clock record bytes {0, sec, min, hour, 0, day, month, year % 100}, each BCD.
pub fn clock_bytes(year: u32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> [u8; 8] {
	[0, bcd(sec), bcd(min), bcd(hour), 0, bcd(day), bcd(month), bcd(year % 100)]
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

impl Where {

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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn explore_nibble_popcount_table() {
		let table = [0u32, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4];
		for (n, &t) in table.iter().enumerate() {
			assert_eq!((n as u8).count_ones(), t);
		}
	}

	#[test]
	fn remembered_slots_claim_from_the_top() {
		let mut t = [0u8; 256];
		assert_eq!(find_or_claim(&mut t, 5), 63);
		assert_eq!(u16::from_le_bytes([t[252], t[253]]), 6);
		assert_eq!(find_or_claim(&mut t, 5), 63);
		assert_eq!(find_or_claim(&mut t, 9), 62);
		assert_eq!(find_or_claim(&mut t, -1), -1);
	}

	/// The slot row prints the date the way the local player writes it: day, then month, then year.
	#[test]
	fn the_slot_row_shows_day_month_year() {
		let s = Summary { level: 0, bolts: 0, play_frames: 0, date: [0x10, 0x23, 0x26] };
		assert_eq!(s.date_text(), "23/10/26");
	}
}
