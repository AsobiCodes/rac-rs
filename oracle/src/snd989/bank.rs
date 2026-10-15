//! Sound banks: the 'SBlk' container (12-byte sound entries, 0x28-byte grains, SPU2 sample data), banks keyed by the
//! handle the game holds, and the two load paths (global bank = RPC fno 3 from the disc, level bank = fno 0x57 from
//! EE RAM). Layout: decomp/notes/sound_grains.md "Handler struct" / "Grain type table" / "Census".
//!
//! Container (little endian): u32 3, u32 2 chunks, u32 meta offset (0x18), u32 meta size, u32 data offset, u32 data size;
//! total length = data offset + data size. Meta chunk = 'SBlk' block: +0 'SBlk', +4 u32 version 1, +8 u32 (4 on disc),
//! +0xc u32 name/id, +0x16 u16 sounds, +0x18 u16 grains, +0x1a u16 VAGs, +0x1c u32 sound table offset,
//! +0x20 u32 grain table offset. Sound entry (12 bytes): +0 s8 volume, +1 u8 volume group, +2 u16 pan degrees,
//! +4 u8 grain count, +5 pad, +6 u16 flags, +8 u32 byte offset of its grains inside the grain table.
//! Grain (0x28 bytes): u32 type, u32 delay (the driver reads the u16 at +4), 32 operand bytes ("p" = grain+8).

use std::collections::HashMap;
use std::sync::Arc;

pub const SECTOR: usize = 0x800;
pub const GRAIN_BYTES: usize = 0x28;
pub const SOUND_BYTES: usize = 12;

/// Grain type 1: key a tone on.
pub const GRAIN_TONE: u32 = 1;

fn u16le(b: &[u8], o: usize) -> Result<u16, String> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| format!("bank: read at {o:#x} outside the data"))
}

fn u32le(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| format!("bank: read at {o:#x} outside the data"))
}

fn span(b: &[u8], o: usize, len: usize) -> Result<&[u8], String> {
    o.checked_add(len).and_then(|e| b.get(o..e)).ok_or_else(|| format!("bank: range {o:#x}+{len:#x} outside the data"))
}

/// One grain: type, delay (ticks, 240 Hz) and its 32 operand bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grain {
    pub kind: u32,
    /// The u16 the driver reads at +4 (the high half is 0 in every retail bank).
    pub delay: u16,
    /// grain+8 .. grain+0x28.
    pub p: [u8; 32],
}

impl Grain {
    pub fn u8_at(&self, o: usize) -> u8 {
        self.p[o]
    }

    pub fn s8_at(&self, o: usize) -> i8 {
        self.p[o] as i8
    }

    pub fn u16_at(&self, o: usize) -> u16 {
        u16::from_le_bytes([self.p[o], self.p[o + 1]])
    }

    pub fn s16_at(&self, o: usize) -> i16 {
        self.u16_at(o) as i16
    }

    pub fn u32_at(&self, o: usize) -> u32 {
        u32::from_le_bytes([self.p[o], self.p[o + 1], self.p[o + 2], self.p[o + 3]])
    }

    /// The tone operands of a type-1 grain.
    pub fn tone(&self) -> Option<Tone> {
        if self.kind != GRAIN_TONE {
            return None;
        }
        Some(Tone {
            priority: self.s8_at(0),
            volume: self.s8_at(1),
            centre_note: self.s8_at(2),
            centre_fine: self.s8_at(3),
            pan: self.s16_at(4),
            map_lo: self.u8_at(6),
            map_hi: self.u8_at(7),
            bend_down: self.s8_at(8),
            bend_up: self.s8_at(9),
            adsr1: self.u16_at(10),
            adsr2: self.u16_at(12),
            flags: self.u16_at(14),
            sample: self.u32_at(16),
        })
    }
}

/// Operands of a tone grain (type 1), p = grain+8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tone {
    /// p0 s8 voice priority.
    pub priority: i8,
    /// p1 s8 tone volume (negative: -1..-4 = register r0..r3, below -4 = random; see the grain table).
    pub volume: i8,
    /// p2 s8 centre note; the driver uses |p2| and treats a non-negative byte as a 44.1 kHz sample.
    pub centre_note: i8,
    /// p3 s8 centre fine, ADDED to the fine pitch (0..125 in the retail banks).
    pub centre_fine: i8,
    /// p4 s16 pan degrees (negative: -1..-4 = register, below -4 = random).
    pub pan: i16,
    pub map_lo: u8,
    pub map_hi: u8,
    /// p8 / p9 s8 pitch-bend range in semitones for bend below / above zero.
    pub bend_down: i8,
    pub bend_up: i8,
    /// p10 / p12 raw ADSR1 / ADSR2.
    pub adsr1: u16,
    pub adsr2: u16,
    /// p14: bit 0 reverb send, bits 1-2 priority fade mode.
    pub flags: u16,
    /// p16 byte offset of the sample (VAG) inside the bank's sample data.
    pub sample: u32,
}

impl Tone {
    pub fn reverb(&self) -> bool {
        self.flags & 1 != 0
    }

    pub fn fade_mode(&self) -> u16 {
        (self.flags >> 1) & 3
    }
}

/// A sound entry plus its grains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sound {
    pub volume: i8,
    pub group: u8,
    pub pan: u16,
    pub pad5: u8,
    /// Entry flags (+6): bit 1 = single instance (the play call first stops other copies, FUN_0000d184), bit 2 = pan
    /// scaling by the volume scale (0x635c).
    pub flags: u16,
    pub grain_offset: u32,
    pub grains: Vec<Grain>,
}

impl Sound {
    pub fn tones(&self) -> impl Iterator<Item = Tone> + '_ {
        self.grains.iter().filter_map(Grain::tone)
    }
}

/// What identifies a bank besides its handle: the SBlk header counts and id word. A savestate's IOP RAM keeps these
/// (the loader only relocates pointers), so an adopted handle can be matched to the disc bank it was loaded from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signature {
    pub id: u32,
    pub sounds: u16,
    pub grains: u16,
    pub vags: u16,
}

impl Signature {
    /// From the bytes of an 'SBlk' block (container meta chunk, or the relocated copy in IOP RAM at the bank handle).
    pub fn from_sblk(sblk: &[u8]) -> Result<Signature, String> {
        if span(sblk, 0, 4)? != b"SBlk" {
            return Err("not an SBlk block".into());
        }
        Ok(Signature { id: u32le(sblk, 0xc)?, sounds: u16le(sblk, 0x16)?, grains: u16le(sblk, 0x18)?, vags: u16le(sblk, 0x1a)? })
    }
}

/// A parsed bank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bank {
    pub signature: Signature,
    pub sounds: Vec<Sound>,
    /// The SPU2 sample data (ADPCM blocks); tone `sample` offsets index into it.
    pub data: Vec<u8>,
}

impl Bank {
    /// Total length of a container from its first 0x18 bytes (data offset + data size).
    pub fn container_len(header: &[u8]) -> Result<usize, String> {
        if u32le(header, 4)? != 2 {
            return Err("sound bank container: expected two chunks".into());
        }
        Ok(u32le(header, 16)? as usize + u32le(header, 20)? as usize)
    }

    /// Parse a container (the bank file / disc section; trailing sector padding is ignored).
    pub fn parse(container: &[u8]) -> Result<Bank, String> {
        let len = Bank::container_len(container)?;
        let container = span(container, 0, len)?;
        let meta = span(container, u32le(container, 8)? as usize, u32le(container, 12)? as usize)?;
        let data = span(container, u32le(container, 16)? as usize, u32le(container, 20)? as usize)?;
        if span(meta, 0, 4)? != b"SBlk" || u32le(meta, 4)? != 1 {
            return Err("sound bank: not an SBlk version 1 block".into());
        }
        let signature = Signature::from_sblk(meta)?;
        let sound_table = u32le(meta, 0x1c)? as usize;
        let grain_table = u32le(meta, 0x20)? as usize;
        let grains = span(meta, grain_table, signature.grains as usize * GRAIN_BYTES)?;
        let mut sounds = Vec::with_capacity(signature.sounds as usize);
        for i in 0..signature.sounds as usize {
            let e = span(meta, sound_table + i * SOUND_BYTES, SOUND_BYTES)?;
            let (count, grain_offset) = (e[4] as usize, u32le(e, 8)?);
            let mut list = Vec::with_capacity(count);
            for g in 0..count {
                let b = span(grains, grain_offset as usize + g * GRAIN_BYTES, GRAIN_BYTES)
                    .map_err(|err| format!("sound {i} grain {g}: {err}"))?;
                let mut p = [0u8; 32];
                p.copy_from_slice(&b[8..]);
                list.push(Grain { kind: u32le(b, 0)?, delay: u16le(b, 4)?, p });
            }
            sounds.push(Sound { volume: e[0] as i8, group: e[1], pan: u16le(e, 2)?, pad5: e[5], flags: u16le(e, 6)?, grain_offset, grains: list });
        }
        Ok(Bank { signature, sounds, data: data.to_vec() })
    }

    /// The sound a play call (cid 17) starts: None when the index is out of range or the entry has no grains
    /// (FUN_00003df0 returns 0 then, no handle).
    pub fn playable(&self, index: i32) -> Option<&Sound> {
        let s = self.sounds.get(usize::try_from(index).ok()?)?;
        if s.grains.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

/// Which load path brought a bank in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BankKind {
    /// RPC fno 3: snd_BankLoadByLoc(lba, 0), read from the disc (boot FUN_0012df20).
    Global,
    /// RPC fno 0x57: asynchronous load from an EE RAM address (boot FUN_0012e088); the level's own bank.
    Level,
}

pub const RPC_LOAD_GLOBAL: u32 = 3;
pub const RPC_LOAD_LEVEL: u32 = 0x57;

impl BankKind {
    pub fn of_rpc(fno: u32) -> Option<BankKind> {
        match fno {
            RPC_LOAD_GLOBAL => Some(BankKind::Global),
            RPC_LOAD_LEVEL => Some(BankKind::Level),
            _ => None,
        }
    }
}

/// Sector reader (the ISO): `count` 2048-byte sectors from `lba`.
pub trait SectorRead {
    fn read_sectors(&mut self, lba: u32, count: u32) -> Option<Vec<u8>>;
}

/// EE RAM (or any flat memory image): `len` bytes at `addr`.
pub trait RamRead {
    fn read(&self, addr: u32, len: usize) -> Option<Vec<u8>>;
}

impl RamRead for [u8] {
    fn read(&self, addr: u32, len: usize) -> Option<Vec<u8>> {
        let a = addr as usize;
        self.get(a..a.checked_add(len)?).map(|s| s.to_vec())
    }
}

impl RamRead for Vec<u8> {
    fn read(&self, addr: u32, len: usize) -> Option<Vec<u8>> {
        self.as_slice().read(addr, len)
    }
}

/// An in-memory disc image (sector 0 at `base_lba`), for tests and tools.
pub struct ImageDisc<'a> {
    pub image: &'a [u8],
    pub base_lba: u32,
}

impl SectorRead for ImageDisc<'_> {
    fn read_sectors(&mut self, lba: u32, count: u32) -> Option<Vec<u8>> {
        let o = lba.checked_sub(self.base_lba)? as usize * SECTOR;
        self.image.get(o..o + count as usize * SECTOR).map(|s| s.to_vec())
    }
}

struct Loaded {
    kind: BankKind,
    bank: Arc<Bank>,
}

/// Loaded banks keyed by the handle the game stores (an IOP address) and sends back in every play call (cid 17 d[0]).
pub struct Banks {
    by_handle: HashMap<u32, Loaded>,
    next_fresh: u32,
}

/// First fresh handle (the address both loads used in the old HLE; savestates hold it in EE RAM).
pub const FRESH_HANDLE_BASE: u32 = 0x0019_ce00;
pub const FRESH_HANDLE_STEP: u32 = 0x1000;

impl Default for Banks {
    fn default() -> Self {
        Banks::new()
    }
}

impl Banks {
    pub fn new() -> Banks {
        Banks { by_handle: HashMap::new(), next_fresh: FRESH_HANDLE_BASE }
    }

    fn fresh_handle(&mut self) -> u32 {
        loop {
            let h = self.next_fresh;
            self.next_fresh += FRESH_HANDLE_STEP;
            if !self.by_handle.contains_key(&h) {
                return h;
            }
        }
    }

    /// Insert a parsed bank under a given handle (savestate adoption) or a fresh one.
    pub fn insert(&mut self, kind: BankKind, bank: Bank, handle: Option<u32>) -> u32 {
        let h = handle.unwrap_or_else(|| self.fresh_handle());
        self.by_handle.insert(h, Loaded { kind, bank: Arc::new(bank) });
        h
    }

    /// RPC fno 3: read the global bank from the disc at `lba` (the first sector's container header gives the length)
    /// and return its handle.
    pub fn load_global(&mut self, disc: &mut dyn SectorRead, lba: u32) -> Result<u32, String> {
        let first = disc.read_sectors(lba, 1).ok_or("global bank: disc read failed")?;
        let sectors = Bank::container_len(&first)?.div_ceil(SECTOR) as u32;
        let all = disc.read_sectors(lba, sectors).ok_or("global bank: disc read failed")?;
        Ok(self.insert(BankKind::Global, Bank::parse(&all)?, None))
    }

    /// RPC fno 0x57: copy the bank from EE RAM at `addr` (the exact bytes the game passes) and return its handle.
    pub fn load_level(&mut self, ee: &dyn RamRead, addr: u32) -> Result<u32, String> {
        let header = ee.read(addr, 0x18).ok_or("level bank: EE RAM read failed")?;
        let len = Bank::container_len(&header)?;
        let all = ee.read(addr, len).ok_or("level bank: EE RAM read failed")?;
        Ok(self.insert(BankKind::Level, Bank::parse(&all)?, None))
    }

    /// cid 6: unload. False for an unknown handle.
    pub fn unload(&mut self, handle: u32) -> bool {
        self.by_handle.remove(&handle).is_some()
    }

    pub fn get(&self, handle: u32) -> Option<&Arc<Bank>> {
        self.by_handle.get(&handle).map(|l| &l.bank)
    }

    pub fn kind(&self, handle: u32) -> Option<BankKind> {
        self.by_handle.get(&handle).map(|l| l.kind)
    }

    /// The handle of the most recently loaded bank of a kind (highest handle among that kind).
    pub fn handle_of_kind(&self, kind: BankKind) -> Option<u32> {
        self.by_handle.iter().filter(|(_, l)| l.kind == kind).map(|(h, _)| *h).max()
    }

    pub fn len(&self) -> usize {
        self.by_handle.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_handle.is_empty()
    }

    /// The sound a play call (handle, sound index) starts, with the bank it lives in.
    pub fn playable(&self, handle: u32, index: i32) -> Option<(&Arc<Bank>, &Sound)> {
        let bank = self.get(handle)?;
        let sound = bank.playable(index)?;
        Some((bank, sound))
    }

    /// Savestate adoption for a handle the loads did not produce: `sblk` = the bytes at the handle in IOP RAM (the
    /// relocated 'SBlk' block); matched by signature against the candidate banks (disc banks the caller can load).
    /// Returns the candidate's index. None when absent or ambiguous.
    pub fn match_signature(sblk: &[u8], candidates: &[&Bank]) -> Option<usize> {
        let sig = Signature::from_sblk(sblk).ok()?;
        let mut hit = None;
        for (i, b) in candidates.iter().enumerate() {
            if b.signature == sig {
                if hit.is_some() {
                    return None;
                }
                hit = Some(i);
            }
        }
        hit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snd989::testdata;
    use std::collections::BTreeMap;

    fn banks() -> Vec<(String, Vec<u8>)> {
        let b = testdata::all_banks();
        if b.is_empty() {
            eprintln!("skip: extracted sound banks absent");
        }
        b
    }

    #[test]
    fn global_bank_parses() {
        let Some(raw) = testdata::read("globals/sound_bank.bin") else {
            eprintln!("skip: extracted/globals/sound_bank.bin absent");
            return;
        };
        let b = Bank::parse(&raw).unwrap();
        assert_eq!((b.signature.sounds, b.signature.grains, b.signature.vags), (10, 124, 18));
        assert_eq!(b.sounds.len(), 10);
        // trailing sector padding is not part of the container
        assert!(raw.len() >= Bank::container_len(&raw).unwrap());
        assert_eq!(b.data.len(), 0x240e0);
        // title sound 3: "35:0; 4,4,4 (LFO); 1 tone; 26 wait; 41 key-off; 26 wait; 36 goto" (note, Census)
        let kinds: Vec<u32> = b.sounds[3].grains.iter().map(|g| g.kind).collect();
        assert_eq!(kinds, [35, 4, 4, 4, 1, 26, 41, 26, 36]);
        let t = b.sounds[3].grains[4].tone().unwrap();
        assert!((t.sample as usize) < b.data.len());
        // "26 wait 2400 + rand%481; 41 key-off; 26 wait 3600 + rand%2401": the delay is before the grain, p0 the random range
        assert_eq!((b.sounds[3].grains[5].delay, b.sounds[3].grains[5].s16_at(0)), (2400, 480));
        assert_eq!((b.sounds[3].grains[7].delay, b.sounds[3].grains[7].s16_at(0)), (3600, 2400));
        assert!(b.playable(3).is_some());
        assert!(b.playable(10).is_none() && b.playable(-1).is_none());
    }

    /// The census of the note ("Census (global bank + 19 level banks, 10590 grains)") and the field statistics
    /// quoted in "Voice path" / "SPU2 ADSR", reproduced from the parsed banks.
    #[test]
    fn census_of_all_twenty_banks() {
        let all = banks();
        if all.is_empty() {
            return;
        }
        assert_eq!(all.len(), 20, "global bank + 19 level banks");
        let mut kinds: BTreeMap<u32, usize> = BTreeMap::new();
        let mut sounds = 0usize;
        let mut groups: BTreeMap<u8, usize> = BTreeMap::new();
        let mut sflags: BTreeMap<u16, usize> = BTreeMap::new();
        let mut adsr: BTreeMap<(u16, u16), usize> = BTreeMap::new();
        let mut prio: BTreeMap<i8, usize> = BTreeMap::new();
        let mut tone_flags: BTreeMap<u16, usize> = BTreeMap::new();
        let (mut tones, mut fine_nonzero, mut fine_max, mut fine_min) = (0usize, 0usize, 0i8, 0i8);
        for (name, raw) in &all {
            let b = Bank::parse(raw).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(b.sounds.len(), b.signature.sounds as usize);
            let mut ngrains = 0;
            for s in &b.sounds {
                sounds += 1;
                *groups.entry(s.group).or_default() += 1;
                *sflags.entry(s.flags).or_default() += 1;
                assert_eq!(s.pad5, 0, "{name}");
                assert!(!s.grains.is_empty(), "{name}: every sound has grains");
                for g in &s.grains {
                    ngrains += 1;
                    *kinds.entry(g.kind).or_default() += 1;
                    assert!(g.kind <= 43, "{name}: grain type {}", g.kind);
                    if let Some(t) = g.tone() {
                        tones += 1;
                        *adsr.entry((t.adsr1, t.adsr2)).or_default() += 1;
                        *prio.entry(t.priority).or_default() += 1;
                        *tone_flags.entry(t.flags).or_default() += 1;
                        fine_nonzero += (t.centre_fine != 0) as usize;
                        fine_max = fine_max.max(t.centre_fine);
                        fine_min = fine_min.min(t.centre_fine);
                        assert!((t.sample as usize) < b.data.len(), "{name}: tone sample offset");
                    }
                }
            }
            // every grain in the table belongs to exactly one sound (no sharing between entries)
            assert_eq!(ngrains, b.signature.grains as usize, "{name}: grains reachable through sounds");
        }
        // note "Census": per type grain counts, total 10590
        let census: [(u32, usize); 24] = [
            (1, 6996), (4, 105), (20, 89), (21, 202), (22, 204), (24, 259), (25, 192), (26, 117), (27, 813), (28, 108),
            (29, 138), (30, 152), (31, 219), (32, 53), (33, 128), (34, 276), (35, 129), (36, 102), (37, 11), (38, 25),
            (40, 14), (41, 212), (42, 10), (43, 36),
        ];
        assert_eq!(kinds.len(), census.len(), "grain types present: {kinds:?}");
        for (k, n) in census {
            assert_eq!(kinds.get(&k), Some(&n), "grain type {k}");
        }
        assert_eq!(kinds.values().sum::<usize>(), 10590);
        // "Voice path": groups 0 (5088 sounds) and 4 (304); "Port plan": 5392 sounds
        assert_eq!(sounds, 5392);
        assert_eq!(groups, BTreeMap::from([(0u8, 5088usize), (4, 304)]));
        // sound entry flags: 0 (4803), 1 (578), 0x8000 (10), 5 (1 sound: bit 2 = pan scaling)
        assert_eq!(sflags, BTreeMap::from([(0u16, 4803usize), (1, 578), (0x8000, 10), (5, 1)]));
        // tones: priorities 0, 7, 90, 100; flags only 0 / 1; centre fine 0..=125 (about 3900 non-zero)
        assert_eq!(tones, 6996);
        assert_eq!(prio.keys().copied().collect::<Vec<_>>(), [0, 7, 90, 100]);
        assert_eq!(tone_flags.keys().copied().collect::<Vec<_>>(), [0, 1]);
        assert!(fine_min >= 0 && fine_max == 125, "fine {fine_min}..{fine_max}");
        assert!((3800..4000).contains(&fine_nonzero), "non-zero centre fine: {fine_nonzero}");
        // ADSR pairs (decoded pair table in the note): 4529 / 1544 / 72 / 60 / 57 tones
        for (pair, n) in [((0x1EFF, 0xDFCC), 4529), ((0x80FF, 0x9FC0), 1544), ((0xA5FF, 0xCA80), 72), ((0xAEFF, 0x5089), 60), ((0x809F, 0xCA00), 57)] {
            assert_eq!(adsr.get(&pair), Some(&n), "adsr {pair:x?}");
        }
    }

    #[test]
    fn grain_delays_fit_in_u16() {
        // "Header: u32 type, u32 delay (high 16 bits are 0 in all banks)"
        for (name, raw) in banks() {
            let b = Bank::parse(&raw).unwrap();
            let meta_off = u32::from_le_bytes(raw[8..12].try_into().unwrap()) as usize;
            let meta = &raw[meta_off..];
            let gt = u32::from_le_bytes(meta[0x20..0x24].try_into().unwrap()) as usize;
            for g in 0..b.signature.grains as usize {
                let hi = u16::from_le_bytes(meta[gt + g * GRAIN_BYTES + 6..][..2].try_into().unwrap());
                assert_eq!(hi, 0, "{name}: grain {g} delay high half");
            }
        }
    }

    #[test]
    fn handles_and_the_two_load_paths() {
        let (Some(global_raw), Some(level_raw)) = (testdata::read("globals/sound_bank.bin"), testdata::read("levels/4_eudora/sound_bank.bin"))
        else {
            eprintln!("skip: extracted sound banks absent");
            return;
        };
        let mut banks = Banks::new();
        // fno 3: the global bank read from a disc image (sector 0 of the image = lba 1000)
        let mut disc = ImageDisc { image: &global_raw, base_lba: 1000 };
        let g = banks.load_global(&mut disc, 1000).unwrap();
        // fno 0x57: the level bank copied out of EE RAM at an address
        let mut ee = vec![0u8; 0x400000];
        ee[0x100000..0x100000 + level_raw.len()].copy_from_slice(&level_raw);
        let l = banks.load_level(&ee, 0x100000).unwrap();
        assert_ne!(g, l);
        assert_eq!(banks.len(), 2);
        assert_eq!((banks.kind(g), banks.kind(l)), (Some(BankKind::Global), Some(BankKind::Level)));
        assert_eq!(BankKind::of_rpc(3), Some(BankKind::Global));
        assert_eq!(BankKind::of_rpc(0x57), Some(BankKind::Level));
        assert_eq!(BankKind::of_rpc(4), None);
        // the same sound index means different sounds in the two banks (the diagnosis of the port plan)
        let (gb, gs) = banks.playable(g, 3).unwrap();
        let (lb, ls) = banks.playable(l, 3).unwrap();
        assert_eq!(gb.signature.sounds, 10);
        assert_ne!(gs.grains, ls.grains);
        assert!(lb.signature.sounds > 100);
        assert_eq!(banks.handle_of_kind(BankKind::Level), Some(l));
        // unknown handle, out-of-range sound
        assert!(banks.playable(0x1234, 0).is_none());
        assert!(banks.playable(g, 10).is_none());
        // cid 6 drops the bank; adoption keeps the savestate's handle
        assert!(banks.unload(l));
        assert!(!banks.unload(l));
        assert!(banks.get(l).is_none());
        let adopted = banks.insert(BankKind::Level, Bank::parse(&level_raw).unwrap(), Some(0x0019_ce00));
        assert_eq!(adopted, 0x0019_ce00);
        assert!(banks.playable(0x0019_ce00, 3).is_some());
        // fresh handles never collide with an adopted one
        let again = banks.insert(BankKind::Level, Bank::parse(&level_raw).unwrap(), None);
        assert_ne!(again, 0x0019_ce00);
    }

    #[test]
    fn savestate_handle_matches_the_disc_bank() {
        let (Some(iop), Some(level_raw), Some(global_raw)) =
            (testdata::read_state("s07/iopMemory.bin"), testdata::read("levels/4_eudora/sound_bank.bin"), testdata::read("globals/sound_bank.bin"))
        else {
            eprintln!("skip: s07 savestate or extracted banks absent");
            return;
        };
        // EE word 0x15ed5c of s07 holds 0x0019ce00: the relocated 'SBlk' block of the level bank sits at that IOP address
        let level = Bank::parse(&level_raw).unwrap();
        let global = Bank::parse(&global_raw).unwrap();
        let at = &iop[0x0019_ce00..];
        assert_eq!(&at[..4], b"SBlk");
        let sig = Signature::from_sblk(at).unwrap();
        assert_eq!(sig, level.signature);
        assert_eq!(Banks::match_signature(at, &[&global, &level]), Some(1));
        // ambiguity / absence
        assert_eq!(Banks::match_signature(at, &[&global]), None);
        assert_eq!(Banks::match_signature(at, &[&level, &level]), None);
    }
}

// ---------------------------------------------------------------- savestate / adoption helpers
use super::persist::{In, Out};

impl Tone {
    pub fn save(&self, o: &mut Out) {
        o.i8(self.priority);
        o.i8(self.volume);
        o.i8(self.centre_note);
        o.i8(self.centre_fine);
        o.i16(self.pan);
        o.u8(self.map_lo);
        o.u8(self.map_hi);
        o.i8(self.bend_down);
        o.i8(self.bend_up);
        o.u16(self.adsr1);
        o.u16(self.adsr2);
        o.u16(self.flags);
        o.u32(self.sample);
    }
    pub fn load(i: &mut In) -> Tone {
        Tone {
            priority: i.i8(),
            volume: i.i8(),
            centre_note: i.i8(),
            centre_fine: i.i8(),
            pan: i.i16(),
            map_lo: i.u8(),
            map_hi: i.u8(),
            bend_down: i.i8(),
            bend_up: i.i8(),
            adsr1: i.u16(),
            adsr2: i.u16(),
            flags: i.u16(),
            sample: i.u32(),
        }
    }
}

impl Bank {
    /// The whole parsed bank, for a savestate holding a bank that is on no disc section (the vendor bank the game loads
    /// from its own file through fno 0x57).
    pub fn save(&self, o: &mut Out) {
        self.signature.save(o);
        o.len(self.sounds.len());
        for s in &self.sounds {
            o.i8(s.volume);
            o.u8(s.group);
            o.u16(s.pan);
            o.u8(s.pad5);
            o.u16(s.flags);
            o.u32(s.grain_offset);
            o.len(s.grains.len());
            for g in &s.grains {
                o.u32(g.kind);
                o.u16(g.delay);
                o.bytes(&g.p);
            }
        }
        o.bytes(&self.data);
    }
    pub fn load(i: &mut In) -> Bank {
        let signature = Signature::load(i);
        let n = i.len();
        let mut sounds = Vec::with_capacity(n);
        for _ in 0..n {
            let (volume, group, pan, pad5, flags, grain_offset) = (i.i8(), i.u8(), i.u16(), i.u8(), i.u16(), i.u32());
            let m = i.len();
            let mut grains = Vec::with_capacity(m);
            for _ in 0..m {
                let (kind, delay) = (i.u32(), i.u16());
                let mut p = [0u8; 32];
                p.copy_from_slice(i.bytes());
                grains.push(Grain { kind, delay, p });
            }
            sounds.push(Sound { volume, group, pan, pad5, flags, grain_offset, grains });
        }
        Bank { signature, sounds, data: i.bytes().to_vec() }
    }
}

impl Signature {
    pub fn save(&self, o: &mut Out) {
        o.u32(self.id);
        o.u16(self.sounds);
        o.u16(self.grains);
        o.u16(self.vags);
    }
    pub fn load(i: &mut In) -> Signature {
        Signature { id: i.u32(), sounds: i.u16(), grains: i.u16(), vags: i.u16() }
    }
}

impl Banks {
    /// (handle, kind, signature) of every loaded bank, sorted by handle.
    pub fn entries(&self) -> Vec<(u32, BankKind, Signature)> {
        let mut v: Vec<_> = self.by_handle.iter().map(|(h, l)| (*h, l.kind, l.bank.signature)).collect();
        v.sort_by_key(|e| e.0);
        v
    }
    pub fn next_fresh(&self) -> u32 {
        self.next_fresh
    }
    pub fn set_next_fresh(&mut self, v: u32) {
        self.next_fresh = v;
    }
    /// Bank content hash for the savestate check (FNV over the sample data and the sound table sizes).
    pub fn content_hash(bank: &Bank) -> u32 {
        let mut h: u32 = 0x811c_9dc5;
        for b in bank.data.iter().step_by(61) {
            h = (h ^ *b as u32).wrapping_mul(0x0100_0193);
        }
        h ^ bank.data.len() as u32
    }
}
