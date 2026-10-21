//! `Snd989`: the exact 989snd driver as the oracle's sound HLE (stage 7b/7d). Replaces the old `hle::Snd` guesses.
//!
//! Time: the IOP timer ticks 240 times a second free of the EE vblank (50 Hz), so tick(vb) = floor(vb * 240 / 50). Every
//! command runs at the tick boundary of its field: `advance_to(vb)` first, then the command (4.8 ticks per field
//! accumulate, nothing is rounded per field). `vb` is the driver's own field clock `Inner::vb` (stage 10a): the machine's
//! fields (`Snd989::field`, called by every `Cpu::vblank`), started at the game's VBCOUNT (EE 0x15ed48) when a savestate is
//! adopted (so tick and parity of stage 8 are unchanged) and at 0 on a cold boot (the driver's tick counter [0x19770] starts
//! at 0 with the module). On hardware the tick is an IOP hard timer (989snd 0x6b0c: AllocHardTimer(SYSCLK, 32 bit, 1),
//! compare = USec2SysClock(1 s) / 240 = 153600 IOP cycles, handler 0x6df0 wakes the tick thread), independent of EE code,
//! so the driver ticks on the title screen, in menus and in movies.
//!
//! Savestates: the IOP RAM of the states holds the 989snd module NEVER initialised (STAGES.md 7d), so adoption builds the
//! boot state of the module in code (`Inner::boot`: the cids the EE's boot code sends, which are not resent after a
//! load), takes the bank images by signature (the handle IS the IOP address of the relocated SBlk), drops sounds that
//! played at save time and rebuilds the three EE stream channels at position 0.
use super::bank::{Bank, BankKind, Banks, RamRead, SectorRead, Signature, SECTOR};
use super::coreinit::{CoreMix, Core as CoreRegs};
use super::handler::{Driver, PlayArgs, Voices};
use super::lfo::Lfos;
use super::persist::{In, Out};
use super::reverb::{FxOp, Reverb};
use super::rng::{Rng250, INDEX_ADDR, LEN, TABLE_ADDR};
use super::streams::{PlayVag, StreamEv, Streams, VagInfo};
use super::voices::{Reg, SpuVoices, SpuWrite};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, OnceLock};

/// Where the module sits in the IOP RAM of every state seen (s07, fg1/4_0159, fg2/15_0055).
pub const MODULE_BASE: u32 = 0x99fa0;
/// First level-bank handle of a fresh machine (the EE keeps it in 0x15ed5c).
pub const FIRST_LEVEL_HANDLE: u32 = 0x0019_ce00;
/// EE word holding the level bank's handle, and the EE byte holding the level number.
pub const LEVEL_HANDLE_WORD: u32 = 0x15ed5c;
pub const LEVEL_NUMBER_BYTE: u32 = 0x15ed84;
/// The EE's stream channel structs: music, voice, stinger (word 0 = handle: 0 none, -1 reply pending).
pub const STREAM_CHANNELS: [u32; 3] = [0x151704, 0x151720, 0x15173c];
/// boot FUN_0022c3e0: the group volumes the EE computed (groups 0..5), and the option word that selects mono.
pub const GROUP_VOLUMES: u32 = 0x13e598;
pub const OPTION_WORD: u32 = 0x15ede8;
/// cid 42 as the EE sends it (boot FUN_00214e88 -> FUN_0012eb20(4, 0xf000, 0, 1)).
pub const STREAM_COUNT: usize = 4;
/// The level header's music lbas as the EE keeps them (slot n = 0x13a628 + 4 * n, music.md).
pub const SLOT_LBAS: u32 = 0x13a628;
pub const STREAM_BUFFER: u32 = 0xf000;

/// tick(vb) = floor(vb * 240 / 50); NTSC (the USA disc, or the PAL disc in its 60 Hz mode: 59.94 fields a second):
/// floor(vb * 240 * 1001 / 60000).
pub fn tick_of(vb: u32) -> u32 {
    tick_in(vb, crate::ver::ntsc())
}

/// `tick_of` in the given video mode.
fn tick_in(vb: u32, ntsc: bool) -> u32 {
    if ntsc {
        return (vb as u64 * 240 * 1001 / 60000) as u32;
    }
    (vb as u64 * 240 / 50) as u32
}

// ---------------------------------------------------------------- SPU2 RAM allocator (0xd5f0 / 0xd640)

/// The IRX's SPU2 RAM allocator: init = one free region 0x5040..0x200000, sizes rounded up to 0x40, first fit. Callers: the
/// bank loader (sample data) and cid 42 (stream rings: fp * N). `free` returns a block to the list [I: the free path is not
/// read; the model merges neighbours].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spu2Alloc {
    /// Sorted (start, end) of the free regions.
    free: Vec<(u32, u32)>,
}

impl Default for Spu2Alloc {
    fn default() -> Self {
        Spu2Alloc { free: vec![(0x5040, 0x20_0000)] }
    }
}

impl Spu2Alloc {
    pub fn round(size: u32) -> u32 {
        size.wrapping_add(0x3f) & !0x3f
    }
    pub fn alloc(&mut self, size: u32) -> Option<u32> {
        let n = Self::round(size).max(0x40);
        let i = self.free.iter().position(|(a, b)| b - a >= n)?;
        let a = self.free[i].0;
        if self.free[i].1 - a == n {
            self.free.remove(i);
        } else {
            self.free[i].0 += n;
        }
        Some(a)
    }
    pub fn release(&mut self, addr: u32, size: u32) {
        let n = Self::round(size).max(0x40);
        let at = self.free.partition_point(|r| r.0 < addr);
        self.free.insert(at, (addr, addr + n));
        let mut k = 0;
        while k + 1 < self.free.len() {
            if self.free[k].1 == self.free[k + 1].0 {
                self.free[k].1 = self.free[k + 1].1;
                self.free.remove(k + 1);
            } else {
                k += 1;
            }
        }
    }
    pub fn regions(&self) -> &[(u32, u32)] {
        &self.free
    }

    /// The effect-block allocator (0xd71c): the size is rounded up to 0x40; a free region qualifies when the block fits below
    /// the 128 KiB boundary under the region's end (`end & !0x1ffff`, the SPU2 reverb end page); the smallest qualifying region
    /// (the first on a tie) is cut at that boundary, the remainders stay free. Returns the block address, None when nothing fits.
    pub fn alloc_effect(&mut self, size: u32) -> Option<u32> {
        let n = Self::round(size).max(0x40);
        let mut best: Option<(usize, u32)> = None;
        for (i, &(a, b)) in self.free.iter().enumerate() {
            let aligned = b & !0x1ffff;
            if b - a >= n && a < aligned && aligned - a >= n && best.map_or(true, |(j, _)| b - a < self.free[j].1 - self.free[j].0) {
                best = Some((i, aligned));
            }
        }
        let (_, aligned) = best?;
        let at = aligned - n;
        self.carve(at, n).then_some(at)
    }

    /// Takes the exact range [addr, addr + size rounded to 0x40) out of the free list (false when it is not entirely free).
    pub fn carve(&mut self, addr: u32, size: u32) -> bool {
        let n = Self::round(size).max(0x40);
        let Some(i) = self.free.iter().position(|&(a, b)| a <= addr && addr + n <= b) else { return false };
        let (a, b) = self.free[i];
        self.free.remove(i);
        if addr + n < b {
            self.free.insert(i, (addr + n, b));
        }
        if a < addr {
            self.free.insert(i, (a, addr));
        }
        true
    }
}

// ---------------------------------------------------------------- disc access

/// The disc image as a sector source.
pub struct IsoSectors<'a>(pub &'a mut std::fs::File);

impl SectorRead for IsoSectors<'_> {
    fn read_sectors(&mut self, lba: u32, count: u32) -> Option<Vec<u8>> {
        self.0.seek(SeekFrom::Start(lba as u64 * SECTOR as u64)).ok()?;
        let mut v = vec![0u8; count as usize * SECTOR];
        self.0.read_exact(&mut v).ok()?;
        Some(v)
    }
}

/// EE RAM with the address masked like the CPU does.
pub struct EeRam<'a>(pub &'a [u8]);

impl RamRead for EeRam<'_> {
    fn read(&self, addr: u32, len: usize) -> Option<Vec<u8>> {
        let a = (addr & 0x01ff_ffff) as usize;
        self.0.get(a..a.checked_add(len)?).map(|s| s.to_vec())
    }
}

/// A bank of the player's disc: kind, signature and parsed content.
pub struct Cand {
    pub kind: BankKind,
    pub sig: Signature,
    pub bank: Bank,
    /// The level number (0..19) of a level bank (`isolvl` index, EE byte 0x15ed84), None for the global bank.
    pub level: Option<u32>,
}

/// The global bank (ToC + 0x14e0: sector, sector count) and the 19 level banks (section 1 of every level) of the disc image
/// (ORACLE_ISO, else DEFAULT_ISO). Read once; empty without a disc.
pub fn disc_banks() -> &'static [Cand] {
    static C: OnceLock<Vec<Cand>> = OnceLock::new();
    C.get_or_init(|| {
        let mut out = Vec::new();
        let iso = std::env::var("ORACLE_ISO").unwrap_or_else(|_| crate::DEFAULT_ISO.into());
        if let Ok(mut f) = std::fs::File::open(iso) {
            let mut d = IsoSectors(&mut f);
            if let Some(toc) = d.read_sectors(1500, 6) {
                let w = |o: usize| u32::from_le_bytes(toc[o..o + 4].try_into().unwrap());
                let (lba, n) = (w(0x14e0), w(0x14e4));
                if let Some(b) = d.read_sectors(lba, n).and_then(|raw| Bank::parse(&raw).ok()) {
                    out.push(Cand { kind: BankKind::Global, sig: b.signature, bank: b, level: None });
                }
            }
        }
        for lvl in 0..19 {
            if let Some(b) = crate::isolvl::level_section(lvl, 1).and_then(|raw| Bank::parse(&raw).ok()) {
                out.push(Cand { kind: BankKind::Level, sig: b.signature, bank: b, level: Some(lvl) });
            }
        }
        out
    })
}

/// The VAG header of a stream's first sector: (rate, data bytes). The header starts `ring` bytes into the sector (chan +0x88).
pub fn read_vag(iso: &mut Option<std::fs::File>, lba: u32, ring: u16) -> Option<VagInfo> {
    let f = iso.as_mut()?;
    let mut sec = vec![0u8; SECTOR];
    f.seek(SeekFrom::Start(lba as u64 * SECTOR as u64)).ok()?;
    f.read_exact(&mut sec).ok()?;
    let be = |h: &[u8], o: usize| u32::from_be_bytes(h[o..o + 4].try_into().unwrap());
    for off in [ring as usize, 0] {
        if let Some(h) = sec.get(off..off + 0x30) {
            if &h[..4] == b"VAGp" && be(h, 0x10) != 0 {
                return Some(VagInfo { rate: be(h, 0x10), data_bytes: be(h, 0xc) });
            }
        }
    }
    None
}

/// The player's disc image (ORACLE_ISO, else DEFAULT_ISO).
pub fn open_iso() -> Option<std::fs::File> {
    std::fs::File::open(std::env::var("ORACLE_ISO").unwrap_or_else(|_| crate::DEFAULT_ISO.into())).ok()
}

/// The VAG file of a stream: its header and its payload (the ADPCM blocks after the 0x30-byte header). The payload is read
/// from the disc image at run time, never bundled (stage 11).
pub fn read_vag_payload(iso: &mut Option<std::fs::File>, lba: u32, ring: u16) -> Option<(VagInfo, Vec<u8>)> {
    let info = read_vag(iso, lba, ring)?;
    let f = iso.as_mut()?;
    let mut sec = vec![0u8; SECTOR];
    f.seek(SeekFrom::Start(lba as u64 * SECTOR as u64)).ok()?;
    f.read_exact(&mut sec).ok()?;
    let off = [ring as usize, 0].into_iter().find(|&o| sec.get(o..o + 4) == Some(b"VAGp".as_slice()))?;
    if info.data_bytes > 0x0400_0000 {
        return None;
    }
    let mut data = vec![0u8; info.data_bytes as usize];
    f.seek(SeekFrom::Start(lba as u64 * SECTOR as u64 + off as u64 + 0x30)).ok()?;
    f.read_exact(&mut data).ok()?;
    Some((info, data))
}

// ---------------------------------------------------------------- the driver

/// What a drained advance produced for the PCM side.
#[derive(Default, Debug)]
pub struct SndOut {
    pub writes: Vec<SpuWrite>,
    pub streams: Vec<StreamEv>,
}

pub struct Inner {
    pub banks: Banks,
    pub drv: Driver<SpuVoices, Lfos>,
    pub streams: Streams,
    pub spu_mem: Spu2Alloc,
    /// bank handle -> (SPU2 address, bytes) of its sample data.
    pub bank_mem: BTreeMap<u32, (u32, u32)>,
    /// The stream ring block (cid 42).
    pub stream_mem: Option<(u32, u32)>,
    pub core: CoreMix,
    /// The reverb driver state (stage 10e): cids 16 / 80 / 81, the tick hook.
    pub rev: Reverb,
    /// VAG headers by first sector, read at cid 44.
    pub vags: HashMap<u32, VagInfo>,
    /// Keep the SPU2 writes / stream events (a host drains them); off = they are dropped after every advance.
    pub record: bool,
    /// Added to the EE-derived tick: the live driver of an adopted state keeps its own tick counter (stage 8).
    pub tick_off: u32,
    /// The driver's field clock (see the module doc): advanced by `Snd989::field`, read by every `advance_to`.
    pub vb: u32,
    /// Debug only (ORACLE_SNDCLOCK): the last seen clock - VBCOUNT difference. Not saved.
    pub tick_off_vb: u32,
    pub out: SndOut,
    /// Load problems and unknown requests (diagnostics, not saved).
    pub notes: Vec<String>,
}

impl Inner {
    /// A driver with no banks and no boot configuration (what the EE's own boot cids start from).
    pub fn blank(rng: Rng250) -> Inner {
        let mut voices = SpuVoices::with_formula();
        voices.log_events = true;
        Inner {
            banks: Banks::new(),
            drv: Driver::new(rng, voices, Lfos::new()),
            streams: Streams::new(),
            spu_mem: Spu2Alloc::default(),
            bank_mem: BTreeMap::new(),
            stream_mem: None,
            core: CoreMix::after_sdinit(),
            rev: Reverb::new(),
            vags: HashMap::new(),
            record: false,
            tick_off: 0,
            vb: 0,
            tick_off_vb: 0,
            out: SndOut::default(),
            notes: Vec::new(),
        }
    }

    pub fn tick(&self) -> u32 {
        self.drv.core.tick
    }

    /// cid 42 (4 handlers, 0xf000 bytes): the ring block comes from the SPU2 allocator, in call order.
    pub fn stream_init(&mut self, count: usize, buf_bytes: u32) {
        if !self.streams.inited {
            let fp = ((buf_bytes as u64 + 0xfff) >> 12).max(2) as u32 * 0x1000;
            if let Some(a) = self.spu_mem.alloc(fp.wrapping_mul(count as u32)) {
                self.streams.spu_base = a;
                self.stream_mem = Some((a, fp.wrapping_mul(count as u32)));
            } else {
                self.notes.push("cid 42: SPU2 RAM exhausted (error 0x2c)".into());
            }
        }
        self.streams.init_cfg(count, buf_bytes);
    }

    /// The boot calls the EE sends once (boot FUN_0022c3e0), applied by hand after a savestate load: cid 0 (init), 11
    /// (out mode from the option word), 13 {0, 1}, 81 (reverb, no effect), 78 for groups 1, 2, 4, 42 (streams), 9 for
    /// groups 0..5 (the volumes the EE computed, read from its RAM: 0x13e598 + 4 * g).
    pub fn boot(&mut self, ee: &[u8]) {
        let rd = |a: u32| u32::from_le_bytes(ee[a as usize..a as usize + 4].try_into().unwrap());
        let (core, _calls) = CoreMix::boot_init();
        self.core = core;
        let mono = (rd(OPTION_WORD) == 0) as i16;
        self.drv.voices.set_out_mode(mono);
        self.core.set_out_mode(mono);
        self.drv.voices.apply_boot_config();
        self.core.set_voice_mode(1);
        self.rev = Reverb::new();
        self.spu_mem = Spu2Alloc::default();
        self.cid81(2, 4);
        self.stream_init(STREAM_COUNT, STREAM_BUFFER);
        for g in 0..6 {
            self.drv.voices.set_group_volume(g, rd(GROUP_VOLUMES + 4 * g as u32) as i32);
        }
        self.drv.voices.take_events();
    }

    /// Init 0x6918, the game's sync call fno 0 on the command client (boot FUN_0012da28 <- FUN_0022c3e0; guard
    /// [0x18e50] == -1, so it runs once): the core registers after sceSdInit + `INIT`, output mode 0, group volumes 0-6
    /// and 8 = 0x400 (group 8 = core 1 MVOL, logged as an SPU2 event). A savestate's driver ran it before the save
    /// (`boot` applies it by hand); a cold boot sends it (hle.rs `rpc_reply`). The guard: only the untouched
    /// after-sceSdInit image takes it.
    pub fn module_init(&mut self) {
        if self.core != CoreMix::after_sdinit() {
            return;
        }
        let (core, _calls) = CoreMix::boot_init();
        self.core = core;
        self.drv.voices.set_out_mode(0);
        for g in [0, 1, 2, 3, 4, 5, 6, 8] {
            self.drv.voices.set_group_volume(g, 0x400);
        }
    }

    /// Logs the reverb driver's register effects as SPU2 events and applies them to the core register image.
    fn apply_fx(&mut self, ops: Vec<FxOp>) {
        for o in ops {
            self.drv.voices.push_event(o.reg, o.index as usize, o.value, o.bank);
            let k = &mut self.core.core[(o.index & 1) as usize];
            match o.reg {
                Reg::EvolL => k.evol[0] = o.value as u16,
                Reg::EvolR => k.evol[1] = o.value as u16,
                Reg::FxOn => k.effect_enable = o.value != 0,
                _ => {}
            }
        }
    }

    /// cid 81 (reverb block reservation).
    pub fn cid81(&mut self, mask: u32, m: i32) {
        let mut ops = Vec::new();
        self.rev.cid81(&mut self.spu_mem, mask, m, &mut ops);
        self.apply_fx(ops);
    }

    /// Adopt a savestate (7d). `iop` = IOP RAM image, `ee` = EE RAM, `cold` = a cold boot (the EE sends the boot cids itself).
    pub fn adopt(iop: &[u8], ee: &[u8], cold: bool, vb: u32) -> Inner {
        Inner::adopt_with(iop, ee, cold, vb, &[])
    }

    /// `spuv` = the state's spu_voices.bin (empty = none: the voices playing at save time have no SPU2 state and end as silence).
    pub fn adopt_with(iop: &[u8], ee: &[u8], cold: bool, vb: u32, spuv: &[u8]) -> Inner {
        let rng = rng_from_iop(iop).unwrap_or_else(|| Rng250::from_state([0; LEN], 0).unwrap());
        let mut me = Inner::blank(rng);
        me.vb = vb;
        me.drv.core.tick = tick_of(vb);
        me.drv.voices.tick = me.drv.core.tick;
        if cold || ee.len() < 0x15ee00 {
            return me;
        }
        me.boot(ee);
        // The ring block is the first allocation (boot FUN_0022c3e0 runs before the global bank load FUN_0022d218), then the banks
        // in IOP address order.
        let cands = disc_banks();
        let mut used = vec![false; cands.len()];
        let mut at = 0usize;
        while at + 0x30 <= iop.len() {
            if &iop[at..at + 4] == b"SBlk" {
                if let Ok(sig) = Signature::from_sblk(&iop[at..]) {
                    if let Some(k) = (0..cands.len()).find(|&k| !used[k] && cands[k].sig == sig) {
                        used[k] = true;
                        me.add_bank(cands[k].kind, cands[k].bank.clone(), Some(at as u32));
                    } else {
                        me.notes.push(format!("SBlk at IOP {at:#x} ({sig:?}) matches no disc bank: not adopted"));
                    }
                }
            }
            at += 4;
        }
        // A state without an IOP image (or with none of the level bank in it) has no relocated SBlk to match: the EE still holds
        // the level bank's handle (EE word 0x15ed5c), and the boot path loaded that bank from the level's own disc section, so take
        // it from the disc by the EE level number (EE byte 0x15ed84, the isolvl index) under that handle.
        if !me.banks.entries().iter().any(|e| e.1 == BankKind::Level) {
            let h = u32::from_le_bytes(ee[LEVEL_HANDLE_WORD as usize..LEVEL_HANDLE_WORD as usize + 4].try_into().unwrap());
            let lvl = ee[LEVEL_NUMBER_BYTE as usize] as u32;
            if (0x0010_0000..0x0020_0000).contains(&h) && me.banks.get(h).is_none() {
                if let Some(c) = cands.iter().find(|c| c.level == Some(lvl)) {
                    me.add_bank(BankKind::Level, c.bank.clone(), Some(h));
                } else {
                    me.notes.push(format!("level {lvl}: no disc level bank for handle {h:#x}"));
                }
            }
        }
        // The running driver (handlers, voices, lists, tick) of the live module; the staged copy has none.
        let sv = super::adopt::parse_spuv(spuv);
        me.adopt_driver(iop, vb, sv.as_deref());
        // Streams (position 0, EE handle kept); the first sound lba is not in the struct, so the length is unknown.
        let now = me.tick();
        for ch in STREAM_CHANNELS {
            let h = u32::from_le_bytes(ee[ch as usize..ch as usize + 4].try_into().unwrap());
            if h != 0 && h != u32::MAX {
                if ch == STREAM_CHANNELS[0] && me.adopt_music(ee, h, now) {
                    continue;
                }
                let Inner { streams, drv, .. } = &mut me;
                streams.adopt(now, &mut drv.voices, h, 0, None);
            }
        }
        me.drain();
        me
    }

    /// Stage 11: the music channel's stream from before the savestate plays on from the start of its file: the track's loop
    /// part (level header slot track + 1, else the track's own slot; looping when the channel's loop flag is set; Eudora
    /// FUN_00257620), held when the channel's pause bit (0x8000 of +0xa) is set [I: the IOP's play position is not in the
    /// savestate]. False when the file or the disc is not available (the stream then only holds its voice).
    fn adopt_music(&mut self, ee: &[u8], h: u32, now: u32) -> bool {
        let w16 = |o: u32| u16::from_le_bytes(ee[o as usize..o as usize + 2].try_into().unwrap());
        let ch = STREAM_CHANNELS[0];
        let (track, vol, flags, state) = (w16(ch + 4) as i16, w16(ch + 6) as i32, w16(ch + 8), w16(ch + 0xa));
        let lba_of = |slot: usize| u32::from_le_bytes(ee[SLOT_LBAS as usize + 4 * slot..SLOT_LBAS as usize + 4 * slot + 4].try_into().unwrap());
        let slot = (track.max(0) as usize + 1).min(14);
        let lba = Some(lba_of(slot)).filter(|&l| l != 0).unwrap_or_else(|| lba_of(track.max(0) as usize));
        if lba == 0 {
            return false;
        }
        let mut iso = open_iso();
        let Some((info, data)) = read_vag_payload(&mut iso, lba, 0) else { return false };
        self.vags.insert(lba, info);
        self.streams.payload.insert(lba, std::sync::Arc::new(data));
        let Inner { streams, drv, .. } = self;
        streams.adopt_playing(now, &mut drv.voices, h, lba, 0x20 | if flags & 1 != 0 { 4 } else { 0 }, vol.max(0) as u32, state & 0x8000 != 0)
    }

    /// Put a parsed bank under `handle` (or a fresh one) and allocate its SPU2 RAM.
    pub fn add_bank(&mut self, kind: BankKind, bank: Bank, handle: Option<u32>) -> u32 {
        let len = bank.data.len() as u32;
        let h = self.banks.insert(kind, bank, handle);
        if let Some(a) = self.spu_mem.alloc(len) {
            self.bank_mem.insert(h, (a, len));
        } else {
            self.notes.push(format!("bank {h:#x}: SPU2 RAM exhausted"));
        }
        h
    }

    /// RPC fno 3 (global bank from the disc sector `arg`) and fno 0x57 (level bank from EE RAM `arg`); the reply is the handle.
    /// The first level bank of a machine gets 0x0019ce00 (the EE keeps that word in savestates).
    pub fn load_bank(&mut self, fno: u32, arg: u32, ee: &[u8], iso: &mut Option<std::fs::File>) -> u32 {
        let res: Result<(BankKind, Bank), String> = if fno == super::bank::RPC_LOAD_GLOBAL {
            match iso.as_mut() {
                None => Err("no disc image (ORACLE_ISO)".into()),
                Some(f) => {
                    let mut d = IsoSectors(f);
                    (|| {
                        let first = d.read_sectors(arg, 1).ok_or("global bank: disc read failed")?;
                        let n = Bank::container_len(&first)?.div_ceil(SECTOR) as u32;
                        let all = d.read_sectors(arg, n).ok_or("global bank: disc read failed")?;
                        Ok((BankKind::Global, Bank::parse(&all)?))
                    })()
                }
            }
        } else {
            let ram = EeRam(ee);
            (|| {
                let header = ram.read(arg, 0x18).ok_or("level bank: EE RAM read failed")?;
                let all = ram.read(arg, Bank::container_len(&header)?).ok_or("level bank: EE RAM read failed")?;
                Ok((BankKind::Level, Bank::parse(&all)?))
            })()
        };
        match res {
            Ok((kind, bank)) => {
                let want = (kind == BankKind::Level && self.banks.get(FIRST_LEVEL_HANDLE).is_none()).then_some(FIRST_LEVEL_HANDLE);
                self.add_bank(kind, bank, want)
            }
            Err(e) => {
                self.notes.push(format!("bank load fno {fno:#x} arg {arg:#x}: {e}"));
                0
            }
        }
    }

    pub fn unload_bank(&mut self, handle: u32) {
        if self.banks.unload(handle) {
            self.drv.core.forget_bank(handle);
            if let Some((a, n)) = self.bank_mem.remove(&handle) {
                self.spu_mem.release(a, n);
            }
        }
    }

    /// Run the driver up to the tick of vblank `vb` (never backwards).
    pub fn advance_to(&mut self, vb: u32) {
        let target = tick_of(vb).wrapping_add(self.tick_off);
        while self.drv.core.tick < target {
            self.tick_once();
        }
        self.drain();
    }

    /// One timer wake: voice poll (callbacks first), then the loop iteration with the stream pass inside the handler loop.
    fn tick_once(&mut self) {
        let ends = self.drv.voices.poll_end(self.drv.core.tick);
        self.drv.core.voice_ends(ends, &mut self.drv.lfo);
        let Inner { drv, streams, vags, .. } = self;
        let vags = &*vags;
        drv.core.iteration_with(&mut drv.voices, &mut drv.lfo, |v, now| streams.tick(now, v, vags));
        // 0xc5e0 (after the key flush) and the depth fade 0x37a8
        let mut ops = Vec::new();
        self.rev.tick(&mut ops);
        self.apply_fx(ops);
        if self.record {
            self.drain();
        }
    }

    /// Move the events out of the layers (kept when `record`, else dropped).
    fn drain(&mut self) {
        let w = self.drv.voices.take_events();
        let s = self.streams.take_events();
        if self.record {
            self.out.writes.extend(w);
            self.out.streams.extend(s);
        }
    }

    fn stolen(&mut self) {
        let st = self.drv.voices.take_stream_stolen();
        self.drv.core.voice_ends(st, &mut self.drv.lfo);
    }

    /// One sound command at the tick boundary of field `vb`. `d` = the six data words (cid 44 reads seven). None = not a
    /// command of the driver model (reverb, plain disc reads and anything unknown stay with the caller).
    pub fn command(&mut self, vb: u32, cid: i16, d: &[u32; 7], iso: &mut Option<std::fs::File>) -> Option<u32> {
        self.advance_to(vb);
        let stream = |h: u32| (h >> 24) & 0x1f == 4;
        let r = match cid {
            17 => match self.banks.playable(d[0], d[1] as i32).map(|(b, _)| b.clone()) {
                Some(bank) => self.drv.play(d[0], &bank, d[1] as i32, &PlayArgs::cid17(d[2] as i32, d[3] as i32, d[4] as i32, d[5] as i32)),
                None => 0,
            },
            21 => {
                if stream(d[0]) {
                    let Inner { streams, drv, .. } = self;
                    streams.stop(&mut drv.voices, d[0]);
                } else {
                    self.drv.stop(d[0]);
                }
                u32::MAX
            }
            25 => {
                if stream(d[0]) {
                    self.streams.playing(d[0])
                } else {
                    self.drv.playing(d[0])
                }
            }
            33 => {
                if stream(d[0]) {
                    let Inner { streams, drv, .. } = self;
                    if streams.update(&mut drv.voices, d[0], d[2] as i32, d[3] as i32) {
                        d[0]
                    } else {
                        0
                    }
                } else {
                    self.drv.update(d[0], d[1], d[2] as i32, d[3] as i32, d[4] as i16, d[5] as i16)
                }
            }
            22 => {
                self.drv.pause_groups(d[0]);
                u32::MAX
            }
            23 => {
                self.drv.resume_groups(d[0]);
                u32::MAX
            }
            24 => {
                self.drv.stop_all();
                u32::MAX
            }
            9 => {
                self.drv.voices.set_group_volume(d[0] as i32, d[1] as i32);
                u32::MAX
            }
            11 => {
                self.drv.voices.set_out_mode(d[0] as i16);
                self.core.set_out_mode(d[0] as i16);
                u32::MAX
            }
            13 => {
                self.drv.voices.set_voice_mode(d[0] as i32);
                self.core.set_voice_mode(d[1]);
                u32::MAX
            }
            78 => {
                self.drv.voices.set_group_range(d[0] as usize, d[1] as i32, d[2] as i32);
                u32::MAX
            }
            6 => {
                self.unload_bank(d[0]);
                u32::MAX
            }
            8 => u32::MAX,
            16 => {
                let mut ops = Vec::new();
                self.rev.cid16(d[0], d[1], d[2], d[3], &mut ops);
                self.apply_fx(ops);
                u32::MAX
            }
            80 => {
                let mut ops = Vec::new();
                self.rev.cid80(&mut self.spu_mem, d[0], d[1] as i32, d[2], d[3], d[4], &mut ops);
                self.apply_fx(ops);
                u32::MAX
            }
            81 => {
                self.cid81(d[0], d[1] as i32);
                u32::MAX
            }
            42 => {
                self.stream_init(d[0] as usize, d[1]);
                u32::MAX
            }
            44 => {
                if !self.streams.inited {
                    self.stream_init(STREAM_COUNT, STREAM_BUFFER);
                }
                let a = PlayVag::cid44(d);
                for (lba, ring) in [(a.lba, a.ring[0]), (a.lba_r, a.ring[1])] {
                    if lba != 0 {
                        if !self.streams.payload.contains_key(&lba) {
                            if let Some((i, data)) = read_vag_payload(iso, lba, ring) {
                                self.vags.insert(lba, i);
                                if self.streams.payload.len() >= 6 {
                                    self.streams.payload.clear();
                                }
                                self.streams.payload.insert(lba, std::sync::Arc::new(data));
                            }
                        } else if let Some(i) = read_vag(iso, lba, ring) {
                            self.vags.insert(lba, i);
                        }
                    }
                }
                let now = self.tick();
                let Inner { streams, drv, .. } = self;
                let r = streams.play(now, &mut drv.voices, &a);
                self.stolen();
                r
            }
            45 => {
                let Inner { streams, drv, .. } = self;
                streams.pause(&mut drv.voices, d[0]);
                u32::MAX
            }
            46 => {
                let Inner { streams, drv, .. } = self;
                streams.resume(&mut drv.voices, d[0]);
                u32::MAX
            }
            50 => self.streams.remaining(d[0]),
            52 => {
                let now = self.tick();
                let Inner { streams, drv, .. } = self;
                streams.stop_all(now, &mut drv.voices);
                u32::MAX
            }
            79 => self.streams.buffered(d[0]) as u32,
            _ => return None,
        };
        self.drain();
        Some(r)
    }
}

/// The R250 state of the module in an IOP RAM image: the table and index word at `MODULE_BASE` (verified against the
/// module's cosine table), else found through that table anywhere in the image.
pub fn rng_from_iop(iop: &[u8]) -> Option<Rng250> {
    let cos: Vec<u8> = (0..64).flat_map(|i| (((0x7fff as f64) * (2.0 * std::f64::consts::PI * i as f64 / 2048.0).cos()) as i16).to_le_bytes()).collect();
    let ok = |base: usize| iop.get(base + 0x17d60..base + 0x17d60 + cos.len()) == Some(&cos[..]) && base + INDEX_ADDR as usize + 4 <= iop.len() && base + TABLE_ADDR as usize + 2 * LEN <= iop.len();
    let base = if ok(MODULE_BASE as usize) {
        MODULE_BASE as usize
    } else {
        let p = iop.windows(cos.len()).position(|w| w == &cos[..])?;
        let b = p.checked_sub(0x17d60)?;
        if !ok(b) {
            return None;
        }
        b
    };
    Rng250::from_iop(iop, base as u32).ok()
}

// ---------------------------------------------------------------- Snd989 (the field of Cpu)

#[derive(Default)]
pub struct Snd989 {
    pub inner: Option<Box<Inner>>,
    /// A host wants the SPU2 writes (`Machine::record_sound`).
    pub record: bool,
    /// The loaded state's spu_voices.bin (stage 8), empty = none.
    pub spuv: Vec<u8>,
}

impl Snd989 {
    /// First use: adopt the state the machine was loaded from.
    /// `vb` = the game's VBCOUNT, used only when this call adopts (a cold boot starts the clock at 0).
    pub fn ensure(&mut self, iop: &[u8], ee: &[u8], cold: bool, vb: u32) -> &mut Inner {
        let vb = if cold { 0 } else { vb };
        let record = self.record;
        let spuv = &self.spuv;
        self.inner.get_or_insert_with(|| {
            let mut i = Box::new(Inner::adopt_with(iop, ee, cold, vb, spuv));
            i.record = record;
            i
        })
    }

    /// One machine field (vblank start, `Cpu::vblank`): the IOP timer runs on whatever the EE does.
    pub fn field(&mut self) {
        if let Some(i) = &mut self.inner {
            i.vb = i.vb.wrapping_add(1);
        }
    }

    /// The game switches its video mode (`ver::set_ntsc`, called after this): the IOP timer keeps its 240 Hz, so the tick
    /// goes on from where it is at the new number of ticks per field.
    pub fn video_mode(&mut self, ntsc: bool) {
        let old = crate::ver::ntsc();
        if let Some(i) = &mut self.inner {
            i.tick_off = i.tick_off.wrapping_add(tick_in(i.vb, old)).wrapping_sub(tick_in(i.vb, ntsc));
        }
    }

    /// The driver's field clock (None before the first use adopts the machine).
    pub fn clock(&self) -> Option<u32> {
        self.inner.as_ref().map(|i| i.vb)
    }

    pub fn set_record(&mut self) {
        self.record = true;
        if let Some(i) = &mut self.inner {
            i.record = true;
        }
    }

    // ---- snapfile section (written by `Cpu::save_dyn`)

    pub fn save(&self) -> Vec<u8> {
        let mut o = Out::new();
        o.bool(self.record);
        let Some(i) = &self.inner else {
            o.bool(false);
            return o.0;
        };
        o.bool(true);
        let es = i.banks.entries();
        o.len(es.len());
        for (h, kind, sig) in &es {
            o.u32(*h);
            o.u8(*kind as u8);
            sig.save(&mut o);
            let bank = i.banks.get(*h).unwrap();
            // A bank of the disc is stored by its hash; one that is on no disc section (the vendor bank, loaded from the game's
            // own file through fno 0x57) is stored whole.
            let hash = Banks::content_hash(bank);
            let on_disc = disc_banks().iter().any(|c| c.sig == *sig && c.kind == *kind && Banks::content_hash(&c.bank) == hash);
            o.bool(on_disc);
            if on_disc {
                o.u32(hash);
            } else {
                bank.save(&mut o);
            }
        }
        o.u32(i.banks.next_fresh());
        o.len(i.bank_mem.len());
        for (h, (a, n)) in &i.bank_mem {
            o.u32(*h);
            o.u32(*a);
            o.u32(*n);
        }
        o.len(i.spu_mem.regions().len());
        for (a, b) in i.spu_mem.regions() {
            o.u32(*a);
            o.u32(*b);
        }
        o.bool(i.stream_mem.is_some());
        let (a, n) = i.stream_mem.unwrap_or((0, 0));
        o.u32(a);
        o.u32(n);
        let c = &i.core;
        o.i16(c.out_mode);
        for k in &c.core {
            o.u16(k.mmix);
            for v in [k.mvol, k.evol, k.avol, k.bvol] {
                o.u16(v[0]);
                o.u16(v[1]);
            }
            o.bool(k.effect_enable);
        }
        let mut vs: Vec<_> = i.vags.iter().collect();
        vs.sort_by_key(|(l, _)| **l);
        o.len(vs.len());
        for (l, v) in vs {
            o.u32(*l);
            v.save(&mut o);
        }
        o.len(i.out.writes.len());
        for w in &i.out.writes {
            w.save(&mut o);
        }
        o.len(i.out.streams.len());
        for e in &i.out.streams {
            e.save(&mut o);
        }
        i.drv.core.save(&mut o);
        i.drv.voices.save(&mut o);
        i.drv.lfo.save(&mut o);
        i.streams.save(&mut o);
        o.u32(i.tick_off);
        o.u32(i.vb);
        i.rev.save(&mut o);
        o.0
    }

    pub fn load(&mut self, bytes: &[u8]) {
        let mut r = In::new(bytes);
        self.record = r.bool();
        if !r.bool() {
            self.inner = None;
            return;
        }
        let mut i = Box::new(Inner::blank(Rng250::from_state([0; LEN], 0).unwrap()));
        i.record = self.record;
        let cands = disc_banks();
        for _ in 0..r.len() {
            let h = r.u32();
            let kind = if r.u8() == 0 { BankKind::Global } else { BankKind::Level };
            let sig = Signature::load(&mut r);
            if !r.bool() {
                i.banks.insert(kind, Bank::load(&mut r), Some(h));
                continue;
            }
            let hash = r.u32();
            let c = cands
                .iter()
                .find(|c| c.sig == sig && c.kind == kind && Banks::content_hash(&c.bank) == hash)
                .or_else(|| cands.iter().find(|c| c.sig == sig && Banks::content_hash(&c.bank) == hash))
                .unwrap_or_else(|| panic!("savestate: sound bank {h:#x} ({sig:?}) is not on the disc image: sound bank files changed"));
            i.banks.insert(kind, c.bank.clone(), Some(h));
        }
        let nf = r.u32();
        i.banks.set_next_fresh(nf);
        for _ in 0..r.len() {
            let (h, a, n) = (r.u32(), r.u32(), r.u32());
            i.bank_mem.insert(h, (a, n));
        }
        let n = r.len();
        i.spu_mem = Spu2Alloc { free: (0..n).map(|_| (r.u32(), r.u32())).collect() };
        let some = r.bool();
        let (a, n) = (r.u32(), r.u32());
        i.stream_mem = some.then_some((a, n));
        i.core.out_mode = r.i16();
        for k in &mut i.core.core {
            *k = CoreRegs { mmix: r.u16(), mvol: [r.u16(), r.u16()], evol: [r.u16(), r.u16()], avol: [r.u16(), r.u16()], bvol: [r.u16(), r.u16()], effect_enable: r.bool() };
        }
        let n = r.len();
        i.vags = (0..n).map(|_| (r.u32(), VagInfo::load(&mut r))).collect();
        let n = r.len();
        i.out.writes = (0..n).map(|_| SpuWrite::load(&mut r)).collect();
        let n = r.len();
        i.out.streams = (0..n).map(|_| StreamEv::load(&mut r)).collect();
        i.drv.core.load(&mut r, &i.banks);
        i.drv.voices.load(&mut r, &i.banks);
        i.drv.lfo.load(&mut r);
        i.streams.load(&mut r);
        i.tick_off = r.u32();
        i.vb = r.u32();
        if !r.done() {
            i.rev = Reverb::load(&mut r);
        }
        assert!(r.done(), "savestate: sound section has trailing bytes");
        self.inner = Some(i);
    }
}

/// `Arc` is only named for the doc links above.
#[allow(dead_code)]
type _Unused = Arc<()>;

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
