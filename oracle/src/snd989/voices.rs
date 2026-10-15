//! 989snd voice layer (stage 6 of the exact port): the 48-voice records, the pool (0x14d60), the L/R volume math
//! (0x1642c, 0x635c, group master 0x16dcc), the key-on register image (0x14fc0), pending KON/KOFF (0x15388, 0x1547c,
//! 0x157f8, 0x15968, 0x162b0), the end-of-voice poll (0xdec8 with 0x1556c / 0x156f8), pause / continue (0x15bb8,
//! 0x15cac), the running volume / pitch updates (the per-voice halves of 0x5aac / 0x5ffc) and the group-volume
//! rewrite (cid 9, 0x16300 / 0x16e74). Note: decomp/notes/sound_grains.md "Voice path"; the disassembly wins
//! (scratch_grain/mdis.py). Every function cites its IOP module address.
//!
//! Output: a register-write event stream [`SpuWrite`] (what the driver writes to the SPU2: voice, register, value) and
//! the SPU2 state itself, one [`Spu2Voice`] per voice. [`SpuVoices::step_spu`] runs the 48 kHz envelope / sample cursor
//! (200 samples per 240 Hz tick) so the end poll can read ENVX exactly like the driver does; the oracle answers ENVX
//! from it and rac-rs can mix the very same voices. Tables come from the player's disc (pan table: the 989snd IRX at
//! 0x19144; pitch tables: libsd), or from the formulas they follow (tested equal to the disc).
//!
//! Not modelled: stream voices (state 4, priority 0x7f, alloc at 0x6182 and cid 9's stream half 0xa6d8), the voice
//! reservation state 3 (0x14cdc), the core mix / effect registers written at init (~0x14b08), and a negative group index
//! (unreachable: banks only hold groups 0 and 4).

use super::bank::{Bank, Tone};
use super::handler::{mask_has, mask_voices, tone_priority, Granted, KeyOn, PitchCtx, VoiceEnd, VoiceMask, Voices, VolCtx, VOICES};
use super::irx::Irx;
use super::pitch::{voice_pitch, Note2PitchTables};
use crate::spu2::Spu2Voice;
use std::sync::Arc;

/// Voice records: base 0x1ff80, stride 0x38.
pub const RECORD_BASE: u32 = 0x1ff80;
pub const RECORD_STRIDE: u32 = 0x38;
/// Pan table: 180 pairs of s16 {l, r} in the 989snd data section.
pub const PAN_TABLE_ADDR: u32 = 0x19144;
pub const PAN_ENTRIES: usize = 180;
/// SPU2 output samples per 240 Hz driver tick (48000 / 240).
pub const SAMPLES_PER_TICK: u32 = 200;
/// Priority of stream voices: never stolen, 0x1556c / 0x15968 do not free them.
pub const STREAM_PRIORITY: u8 = 0x7f;
/// A victim with a priority at or above this is never stolen (0x14eac).
pub const NO_STEAL_PRIORITY: u8 = 0x7e;
/// Polls with ENVX 0 before a keyed voice counts as ended (0x14f4c / 0xe03c: the countdown starts at 4).
pub const END_COUNTDOWN: i8 = 4;

// ---------------------------------------------------------------- pan table

/// The 180 {l, r} pairs the driver indexes with (pan + 90) / 2 degrees... as stored: pair i = floor(0x3fff * cos(i/2 deg)),
/// floor(0x3fff * sin(i/2 deg)), except entry 1 whose r is 182 in the retail data (the formula gives 142).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanTable {
    pub pairs: [(i16, i16); PAN_ENTRIES],
}

impl PanTable {
    /// Built from the formula (+ the retail entry 1). Tested equal to the disc data.
    pub fn from_formula() -> PanTable {
        let mut pairs = [(0i16, 0i16); PAN_ENTRIES];
        for (i, p) in pairs.iter_mut().enumerate() {
            let a = (i as f64) * 0.5_f64.to_radians();
            *p = ((16383.0 * a.cos()).floor() as i16, (16383.0 * a.sin()).floor() as i16);
        }
        pairs[1].1 = 182;
        PanTable { pairs }
    }

    /// Read from the player's 989snd IRX (module address 0x19144).
    pub fn from_irx(irx: &Irx) -> Result<PanTable, String> {
        let b = irx.bytes(PAN_TABLE_ADDR, PAN_ENTRIES * 4).ok_or("pan table outside the module image")?;
        let mut pairs = [(0i16, 0i16); PAN_ENTRIES];
        for (i, p) in pairs.iter_mut().enumerate() {
            *p = (i16::from_le_bytes([b[i * 4], b[i * 4 + 1]]), i16::from_le_bytes([b[i * 4 + 2], b[i * 4 + 3]]));
        }
        Ok(PanTable { pairs })
    }
}

// ---------------------------------------------------------------- volume math

/// 0x635c(pan, scale): the pan scaling of sounds with flag bit 2 (truncating divisions).
pub fn pan_scale(pan: i32, scale: i32) -> i32 {
    if scale >= 0x356 {
        pan
    } else if scale <= 0xaa {
        0
    } else if pan < 0xb5 {
        pan * (scale - 0xaa) / 0x2ac
    } else {
        0x168 - (0x168 - pan) * (scale - 0xaa) / 0x2ac
    }
}

/// 0x1642c(0x7f, 0, vol, pan, tone volume, tone pan, out): the voice's L/R volume. `out` is the voice's previous
/// L/R; the key-on call passes zeros, the running update (0x5aac) the stored values, and the code tests their signs
/// (a surround rear voice keeps its inverted channel). `mode` = output mode word 0x198b0 (1 mono, 2 no phase invert).
pub fn lr_volume(table: &PanTable, mode: i16, vol: i32, pan: i32, tone_volume: i32, tone_pan: i32, out: &mut [i16; 2]) {
    // A = ((0x7f * 0x102 * tv) / 0x7f * vol) / 0x7f (both divisions truncate toward zero)
    let t0 = (0x7f * 0x102i32).wrapping_mul(tone_volume) / 0x7f;
    let a = t0.wrapping_mul(vol) / 0x7f;
    lr_from_amplitude(table, mode, a, tone_pan + pan, out);
}

/// 0x16780 (the stream voice L/R, called from the stream key-on 0x131ec): the same pan stage as 0x1642c, but the
/// amplitude is `((vol * 0x102 * 0x7f / 0x7f) * chan_vol / 0x7f) * byte_vol / 0x7f` (all divisions truncate toward
/// zero) and the angle is `pan + 0 + chan_pan`. `vol` = stream handler +0x10, `pan` = handler +0x12, `chan_vol` = chan
/// +4, `byte_vol` = handler byte 0x34 + chan +5, `chan_pan` = chan +6.
pub fn stream_lr(table: &PanTable, mode: i16, vol: i32, pan: i32, chan_vol: i32, byte_vol: i32, chan_pan: i32, out: &mut [i16; 2]) {
    let s1 = vol.wrapping_mul(0x102).wrapping_mul(0x7f) / 0x7f;
    let s2 = s1.wrapping_mul(chan_vol) / 0x7f;
    let a = s2.wrapping_mul(byte_vol) / 0x7f;
    lr_from_amplitude(table, mode, a, pan + chan_pan, out);
}

/// The pan stage shared by 0x1642c and 0x16780: amplitude `a`, angle = pan + tone pan (not yet folded).
pub fn lr_from_amplitude(table: &PanTable, mode: i16, a: i32, angle: i32, out: &mut [i16; 2]) {
    if a == 0 {
        *out = [0, 0];
        return;
    }
    if mode == 1 {
        *out = [a as i16, a as i16];
        return;
    }
    // idx = (tone pan + pan + 90) folded into 0..359 (0x164c4-0x16524)
    let mut idx = angle.rem_euclid(0x168);
    idx = if idx < 0x10e { idx + 0x5a } else { idx - 0x10e };
    let (o0, o1) = (out[0] as i32, out[1] as i32);
    if idx < 0xb4 {
        let (tl, tr) = table.pairs[idx as usize];
        let mut l = a.wrapping_mul(tl as i32) / 0x3fff;
        let mut r = a.wrapping_mul(tr as i32) / 0x3fff;
        // sign state of the previous L/R (0x165a0-0x16690)
        if o0 >= 0 && o1 >= 0 {
        } else if o0 < 0 && o1 < 0 {
            l = -l;
            r = -r;
        } else {
            let (a1, a0) = (o0.abs(), o1.abs());
            let f = if a0 < a1 { o0 / a1 } else { o1 / a0 };
            l = l.wrapping_mul(f);
            r = r.wrapping_mul(f);
        }
        *out = [l as i16, r as i16];
    } else {
        // behind: the channels swap, then (mode != 2) one channel is phase inverted (0x16698-0x16774)
        let (tl, tr) = table.pairs[(idx - 0xb4) as usize];
        let mut r = a.wrapping_mul(tl as i32) / 0x3fff;
        let mut l = a.wrapping_mul(tr as i32) / 0x3fff;
        if mode != 2 {
            if o0 >= 0 {
                if o1 >= 0 {
                    if r < l {
                        r = -r;
                    } else {
                        l = -l;
                    }
                } else {
                    r = -r;
                }
            } else if o1 >= 0 {
                l = -l;
            } else if l < r {
                r = -r;
            } else {
                l = -l;
            }
        }
        *out = [l as i16, r as i16];
    }
}

/// 0x16dcc(v16, group): the group master. G = 0x19870[group] (0..0x400). Groups >= 7 pass the value through.
pub fn group_master(v: i16, group: i32, g: &[i32; 9]) -> i16 {
    if group >= 7 || group < 0 {
        return v;
    }
    let v = (v as i32).min(0x7ffe);
    let x = (v.wrapping_mul(g[group as usize])) / 1024;
    let sign = if x >= 0 { 1 } else { -1 };
    let q = x.wrapping_mul(x) / 0x7ffe;
    (sign * q) as i16
}

/// The SPU2 VOLL / VOLR value the driver writes: `(group_master(v) & 0xffff) >> 1` (fixed mode, bit 14 = phase invert).
pub fn volume_reg(v: i16, group: i32, g: &[i32; 9]) -> u16 {
    (group_master(v, group, g) as u16) >> 1
}

// ---------------------------------------------------------------- events

/// SPU2 registers the voice layer writes. Per-voice registers carry the voice (0..47) as index, switch registers the
/// core (0, 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reg {
    /// 0x000 | core | voice-in-core * 2
    VolL,
    /// 0x100
    VolR,
    /// 0x200
    Pitch,
    /// 0x300
    Adsr1,
    /// 0x400
    Adsr2,
    /// 0x2040: sample start; value = tone.sample, the byte offset inside the bank's sample data (see `SpuWrite.bank`).
    Ssa,
    /// 0x1800 | core: voices mixed to the left dry output (24 bit mask).
    VmixL,
    /// 0x1a00 | core
    VmixR,
    /// 0x1900 | core: reverb send to the left.
    VmixEL,
    /// 0x1b00 | core
    VmixER,
    /// 0x1500 | core
    Kon,
    /// 0x1600 | core
    Koff,
    /// 0x981 / 0xa81: core 1 master volume (cid 9 group 8).
    MvolL,
    MvolR,
    /// 0xb80 / 0xc80 | core: reverb return volume (EVOLL / EVOLR); value = the 16 bit register.
    EvolL,
    EvolR,
    /// SetCoreAttr(EFFECT_ENABLE | core): value 0 / 1.
    FxOn,
    /// SetAddr(EEA | core): value = the byte address the driver passes (libsd keeps its 128 KiB page only).
    Eea,
    /// sceSdSetEffectAttr(core, {mode, delay, feedback}): value = mode, `SpuWrite.bank` = delay << 16 | feedback. Loads the
    /// preset (modes 7 / 8 patched by delay / feedback) and puts ESA = EEA page end - size + 1.
    FxAttr,
    /// sceSdClearEffectWorkArea(core, channel, mode): the effect work area is zeroed.
    FxClear,
    /// Host event (stage 11, not an SPU2 register): voice `index` plays the ADPCM data of stream `value` (see
    /// [`SpuVoices::attach_stream`]) instead of a bank. Written just before the stream key-on's SSA / KON.
    StreamSrc,
}

/// The ADPCM blocks a stream voice plays: the VAG payload read from the disc (flags prepared by `streams::part_flags`),
/// shared by the oracle's voice and the host mixer. The oracle appends a queued part to it while the stream plays.
pub type StreamData = std::sync::Arc<std::sync::RwLock<Vec<u8>>>;

impl Reg {
    pub fn per_voice(self) -> bool {
        matches!(self, Reg::VolL | Reg::VolR | Reg::Pitch | Reg::Adsr1 | Reg::Adsr2 | Reg::Ssa)
    }
}

/// One register write, in the order the driver issues them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpuWrite {
    /// Driver tick counter (0x19770) when the write happened.
    pub tick: u32,
    pub reg: Reg,
    /// Voice (per-voice registers) or core (switch / master registers).
    pub index: u8,
    pub value: u32,
    /// Bank handle of the sample, for `Reg::Ssa` only (else 0).
    pub bank: u32,
}

// ---------------------------------------------------------------- voice records

/// Voice record state (+0x08).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VState {
    Free = 0,
    /// Keyed: in the active list 0x19868.
    Keyed = 1,
    /// Claimed by the pool, key-on not issued yet.
    Claimed = 2,
    /// Reserved by 0x14cdc (not modelled).
    Reserved = 3,
    /// Stream voice (not modelled).
    Stream = 4,
}

/// The fields of the 0x38-byte voice record the driver uses (offsets in the comments).
#[derive(Clone, Debug)]
pub struct VoiceRec {
    pub state: VState,
    /// +0x0c owner handler (slot); stays set until the voice is freed (0x161e4).
    pub owner: Option<usize>,
    /// +0x10 callback 0x61b4 attached (0 = detached).
    pub callback: bool,
    /// +0x14 tone.
    pub tone: Option<Tone>,
    /// +0x18 / +0x19 pitch centre (60) and fine (0) of the handler at key-on.
    pub centre: i8,
    pub centre_fine: i8,
    /// +0x1a priority (u8), +0x1b group.
    pub priority: u8,
    pub group: i8,
    /// +0x1c stamp = tick counter at alloc / key-on.
    pub stamp: u32,
    /// +0x20 / +0x22 current L / R volume (s16, before the group master).
    pub l: i16,
    pub r: i16,
    /// +0x24 bend, +0x26 pitch mod.
    pub bend: i16,
    pub pmod: i16,
    /// +0x2c tone volume byte, +0x2e tone pan.
    pub tone_volume: i8,
    pub tone_pan: i16,
    /// 0x1fb50[v]: polls with ENVX 0 left before the natural end.
    pub countdown: i8,
}

impl Default for VoiceRec {
    fn default() -> Self {
        VoiceRec {
            state: VState::Free,
            owner: None,
            callback: false,
            tone: None,
            centre: 60,
            centre_fine: 0,
            priority: 0,
            group: 0,
            stamp: 0,
            l: 0,
            r: 0,
            bend: 0,
            pmod: 0,
            tone_volume: 0,
            tone_pan: 0,
            countdown: 0,
        }
    }
}

// ---------------------------------------------------------------- the voice layer

/// The voice layer: the 48 records, the pending masks, the SPU2 voices and the event log.
pub struct SpuVoices {
    pub rec: [VoiceRec; VOICES],
    /// The SPU2 side of every voice (registers as the driver wrote them, envelope, sample cursor).
    pub spu: [Spu2Voice; VOICES],
    /// Sample data owner of the keyed tone (bank handle, bank) per voice.
    banks: [Option<(u32, Arc<Bank>)>; VOICES],
    /// The stream a stream voice plays (stage 11): the SPU2 ring buffer's audio, kept as the whole payload (the voice's
    /// SSA is the offset 0 in it).
    stream: [Option<StreamData>; VOICES],
    /// The newest 32 attached streams by id (the host looks the `Reg::StreamSrc` events up here).
    stream_ids: std::collections::VecDeque<(u32, StreamData)>,
    /// 0x19848: {first, last} voice per group (init {0, 0x2f}).
    pub ranges: [(i16, i16); 8],
    /// 0x19870: group master volumes (0..0x400, init 0x400); index 8 = the master volume written by cid 9.
    pub group_vol: [i32; 9],
    /// 0x198b0: output mode (cid 11): 1 = mono, 2 = surround without phase inversion, else surround.
    pub out_mode: i16,
    pub pan: PanTable,
    pub pitch: Note2PitchTables,
    /// Pending key-on 0x198d8, pending key-off 0x198e0, "on" 0x198e8, "off" 0x198f0 masks.
    pub pending_kon: VoiceMask,
    pub pending_koff: VoiceMask,
    pub on_mask: VoiceMask,
    pub off_mask: VoiceMask,
    /// Reverb send mask per core (0x198d0 / 0x198d4).
    pub reverb: VoiceMask,
    /// Shadow of VMIXL / VMIXR per core (read-modify-write at key-on, 0x17510 / 0x17508).
    pub vmix_l: VoiceMask,
    pub vmix_r: VoiceMask,
    /// The active list 0x19868 in link order (append at key-on).
    pub active: Vec<usize>,
    /// Tick counter [0x19770], set by `tick_begin`.
    pub tick: u32,
    /// Run the SPU2 for `samples_per_tick` samples at the start of every `poll_end` (the time since the last wake).
    pub auto_step: bool,
    pub samples_per_tick: u32,
    /// Register-write log; drain with [`SpuVoices::take_events`].
    pub events: Vec<SpuWrite>,
    pub log_events: bool,
    /// Driver errors (0x52: voice already in the list, 0x53: cid 9 with group 7).
    pub errors: Vec<u32>,
    /// Sound voices a stream allocation stole (priority 0x7f beats any tone): the caller feeds them to the handler core's
    /// `voice_end` like the `Granted.stolen` of a tone alloc. Drain with [`SpuVoices::take_stream_stolen`].
    pub stream_stolen: Vec<VoiceEnd>,
}

impl SpuVoices {
    pub fn new(pan: PanTable, pitch: Note2PitchTables) -> SpuVoices {
        SpuVoices {
            rec: std::array::from_fn(|_| VoiceRec::default()),
            spu: std::array::from_fn(|_| Spu2Voice::default()),
            banks: std::array::from_fn(|_| None),
            stream: std::array::from_fn(|_| None),
            stream_ids: std::collections::VecDeque::new(),
            ranges: [(0, 0x2f); 8],
            group_vol: [0x400; 9],
            out_mode: 0,
            pan,
            pitch,
            pending_kon: [0; 2],
            pending_koff: [0; 2],
            on_mask: [0; 2],
            off_mask: [0; 2],
            reverb: [0; 2],
            vmix_l: [0; 2],
            vmix_r: [0; 2],
            active: Vec::new(),
            tick: 0,
            auto_step: true,
            samples_per_tick: SAMPLES_PER_TICK,
            events: Vec::new(),
            log_events: true,
            errors: Vec::new(),
            stream_stolen: Vec::new(),
        }
    }

    /// Both tables from their formulas (equal to the disc data, see the tests).
    pub fn with_formula() -> SpuVoices {
        SpuVoices::new(PanTable::from_formula(), Note2PitchTables::from_formula())
    }

    /// Both tables from the player's disc: the 989snd IRX (pan table) and libsd (pitch tables).
    pub fn from_irx(snd: &Irx, libsd: &Irx) -> Result<SpuVoices, String> {
        Ok(SpuVoices::new(PanTable::from_irx(snd)?, Note2PitchTables::from_libsd(libsd)?))
    }

    /// The group configuration the game's boot code leaves behind: voice mode 0 (all groups {0, 0x2f}), then cid 78
    /// for groups 1, 2 and 4 = {0x18, 0x2f} (the upper 24 voices).
    pub fn apply_boot_config(&mut self) {
        self.set_voice_mode(0);
        for g in [1, 2, 4] {
            self.set_group_range(g, 0x18, 0x2f);
        }
    }

    /// cid 13 -> init 0x14a38: mode 1 splits the voices (all groups {0x18, 0x2f}, group 1 {0, 0x17}), any other mode
    /// gives every group all 48 voices. (corrected 2026-10-13: the note said mode 1 sets {0x18, 0x2f} for all groups;
    /// the code leaves group 1 at {0, 0x17}.)
    pub fn set_voice_mode(&mut self, mode: i32) {
        if mode == 1 {
            self.ranges = [(0x18, 0x2f); 8];
            self.ranges[1] = (0, 0x17);
        } else {
            self.ranges = [(0, 0x2f); 8];
        }
    }

    /// cid 78 -> 0x14ad4(group, first, last): first < 0 becomes 0, last above 0x2f becomes 0x2f.
    pub fn set_group_range(&mut self, group: usize, first: i32, last: i32) {
        if group < 8 {
            self.ranges[group] = (first.max(0) as i16, if last >= 0x30 { 0x2f } else { last } as i16);
        }
    }

    /// cid 11 -> 0x16410: the output mode word.
    pub fn set_out_mode(&mut self, mode: i16) {
        self.out_mode = mode;
    }

    /// cid 9 -> 0x16300(group, volume): clamp the volume to 0..0x400, store G[group]; groups 0-6 rewrite the L/R
    /// volume of every running voice of the group (0x16e74, no pause test: a paused handler's voices become audible
    /// again), group 8 writes the master volume `volume * 0x3fff >> 10` to both cores' 0x981 / 0xa81 (core 1 side).
    /// Group 7 is an error (0x53, false). The stream half (0xa6d8) is not modelled.
    pub fn set_group_volume(&mut self, group: i32, volume: i32) -> bool {
        let vol = volume.min(0x400).max(0);
        if group == 7 {
            self.errors.push(0x53);
            return false;
        }
        if (0..=8).contains(&group) {
            self.group_vol[group as usize] = vol;
        }
        if group != 8 {
            self.rewrite_group_volumes(group);
        } else {
            let m = ((vol * 0x3fff) / 1024) as u32 & 0xffff;
            self.write(Reg::MvolL, 1, m);
            self.write(Reg::MvolR, 1, m);
        }
        true
    }

    /// 0x16e74: VOLL / VOLR of every voice in state 1 or 4 whose record group is `group` (groups 0-6 only).
    pub fn rewrite_group_volumes(&mut self, group: i32) {
        if !(0..7).contains(&group) {
            return;
        }
        for v in 0..VOICES {
            if matches!(self.rec[v].state, VState::Keyed | VState::Stream) && self.rec[v].group as i32 == group {
                self.write_volumes(v);
            }
        }
    }

    /// 0x14d60(group, 0x7f) then state := 4 (stored at 0xe9a0 / 0xea6c / 0xee18): a stream voice. Priority 0x7f is never
    /// stolen; the record is not in the active list (no end poll). None = no free voice in the group's range.
    pub fn alloc_stream(&mut self, group: i8) -> Option<usize> {
        let g = Voices::alloc(self, self.tick, group, STREAM_PRIORITY as i8)?;
        let v = g.voice;
        self.stream_stolen.extend(g.stolen);
        self.rec[v].state = VState::Stream;
        self.rec[v].owner = None;
        self.banks[v] = None;
        self.stream[v] = None;
        Some(v)
    }

    /// Stage 11: voice `v` (a stream voice, state 4) plays `data`; logs `Reg::StreamSrc` (value = `id`). Call before the
    /// stream's key-on writes.
    pub fn attach_stream(&mut self, v: usize, id: u32, data: StreamData) {
        self.stream[v] = Some(data.clone());
        if !self.stream_ids.iter().any(|(i, _)| *i == id) {
            self.stream_ids.push_back((id, data));
            if self.stream_ids.len() > 32 {
                self.stream_ids.pop_front();
            }
        }
        self.write(Reg::StreamSrc, v, id);
    }

    /// The data of an attached stream (host side of `Reg::StreamSrc`).
    pub fn stream_by_id(&self, id: u32) -> Option<StreamData> {
        self.stream_ids.iter().find(|(i, _)| *i == id).map(|(_, d)| d.clone())
    }

    /// The stream a voice plays (tests, host attach).
    pub fn stream_of(&self, v: usize) -> Option<&StreamData> {
        self.stream[v].as_ref()
    }

    /// Sound voices stolen by stream allocations since the last call.
    /// Savestate adoption (stage 8): voice `v` as the live driver and the SPU2 hold it.
    /// The bank a voice plays from (tests).
    pub fn bank_of(&self, v: usize) -> Option<&Arc<Bank>> {
        self.banks[v].as_ref().map(|b| &b.1)
    }

    pub fn adopt_voice(&mut self, v: usize, rec: VoiceRec, spu: Spu2Voice, bank: Option<(u32, Arc<Bank>)>) {
        self.rec[v] = rec;
        self.spu[v] = spu;
        self.banks[v] = bank;
        self.stream[v] = None;
    }

    pub fn take_stream_stolen(&mut self) -> Vec<VoiceEnd> {
        std::mem::take(&mut self.stream_stolen)
    }

    /// 0x161e4 for a stream voice: state 0, owner 0.
    pub fn free_stream_voice(&mut self, v: usize) {
        self.list_remove(v);
    }

    /// 0xfc48 for one stream voice: 0x16154 (state 1, appended to the active list, priority rec +0x1a := 0, no callback)
    /// and 0x156f8 (pending key-on dropped, pending key-off set). The normal end poll frees the record once ENVX is 0
    /// and the key-off was issued (0x1556c); a voice is never freed without key-off on this path.
    pub fn stream_release(&mut self, v: usize) {
        self.list_add(v);
        self.rec[v].priority = 0;
        self.rec[v].callback = false;
        let (core, bit) = (v / 24, 1u32 << (v % 24));
        self.pending_kon[core] &= !bit;
        self.pending_koff[core] |= bit;
    }

    /// One SPU2 register write of the stream key-on code (0x174f8 / 0x17508 / 0x17518), logged like every other.
    pub fn stream_write(&mut self, reg: Reg, index: usize, value: u32) {
        self.write(reg, index, value);
    }

    /// Queue a key-off for stream voices (the 0x1547c path at the next tick).
    pub fn stream_key_off(&mut self, v: usize) {
        let (core, bit) = (v / 24, 1u32 << (v % 24));
        self.pending_kon[core] &= !bit;
        self.pending_koff[core] |= bit;
    }

    /// Take the logged register writes.
    pub fn take_events(&mut self) -> Vec<SpuWrite> {
        std::mem::take(&mut self.events)
    }

    /// ENVX of a voice (what the end poll reads).
    pub fn envx(&self, voice: usize) -> u16 {
        self.spu[voice].envx()
    }

    /// Voices that are not free.
    pub fn busy(&self) -> usize {
        self.rec.iter().filter(|r| r.state != VState::Free).count()
    }

    /// Run the SPU2 for `n` output samples (48 kHz): envelope and sample cursor of every active voice.
    pub fn step_spu(&mut self, n: u32) {
        for v in 0..VOICES {
            if self.spu[v].active() {
                if let Some(s) = &self.stream[v] {
                    let g = s.read().unwrap();
                    self.spu[v].step_samples(&g[..], n);
                    continue;
                }
                let data: &[u8] = self.banks[v].as_ref().map_or(&[], |(_, b)| &b.data[..]);
                self.spu[v].step_samples(data, n);
            }
        }
    }

    // ------------------------------------------------ register writes

    fn write(&mut self, reg: Reg, index: usize, value: u32) {
        self.write_bank(reg, index, value, 0);
    }

    /// A non-voice register event from outside this layer (the reverb driver `reverb.rs`): logged like the driver's own writes.
    pub fn push_event(&mut self, reg: Reg, index: usize, value: u32, bank: u32) {
        self.write_bank(reg, index, value, bank);
    }

    fn write_bank(&mut self, reg: Reg, index: usize, value: u32, bank: u32) {
        if self.log_events {
            self.events.push(SpuWrite { tick: self.tick, reg, index: index as u8, value, bank });
        }
        if reg.per_voice() {
            let s = &mut self.spu[index];
            match reg {
                Reg::VolL => s.vol_l = value as u16,
                Reg::VolR => s.vol_r = value as u16,
                Reg::Pitch => s.pitch = value as u16,
                Reg::Adsr1 => {
                    s.adsr1 = value as u16;
                    s.env.set_adsr(s.adsr1, s.adsr2);
                }
                Reg::Adsr2 => {
                    s.adsr2 = value as u16;
                    s.env.set_adsr(s.adsr1, s.adsr2);
                }
                // a stream voice reads its data from offset 0 (the event keeps the SPU2 ring address)
                Reg::Ssa => s.ssa = if self.stream[index].is_some() { 0 } else { value },
                _ => {}
            }
        }
    }

    /// VOLL then VOLR from the record's L / R through the group master.
    fn write_volumes(&mut self, v: usize) {
        let (l, r, g) = (self.rec[v].l, self.rec[v].r, self.rec[v].group as i32);
        let (wl, wr) = (volume_reg(l, g, &self.group_vol), volume_reg(r, g, &self.group_vol));
        self.write(Reg::VolL, v, wl as u32);
        self.write(Reg::VolR, v, wr as u32);
    }

    /// PITCH from the record (tone, bend, mod, centre, fine); `centre`/`fine` override the record's (0x5ffc passes the
    /// handler's current values, 0x15cac and 0x14fc0 the record's).
    fn pitch_of(&self, v: usize, bend: i16, pmod: i16, centre: i8, fine: i8) -> Option<u16> {
        let t = self.rec[v].tone.as_ref()?;
        Some(voice_pitch(&self.pitch, t.centre_note, t.centre_fine, t.bend_down, t.bend_up, bend, pmod, centre as u8, fine as u8))
    }

    // ------------------------------------------------ list, free, end

    /// 0x16154: state 1 and append to the active list.
    fn list_add(&mut self, v: usize) {
        self.rec[v].state = VState::Keyed;
        if self.active.contains(&v) {
            self.errors.push(0x52);
        } else {
            self.active.push(v);
        }
    }

    /// 0x161e4: state 0, owner 0, unlink.
    fn list_remove(&mut self, v: usize) {
        self.rec[v].state = VState::Free;
        self.rec[v].owner = None;
        self.active.retain(|&x| x != v);
    }

    fn callback_end(&self, v: usize) -> Option<VoiceEnd> {
        match (self.rec[v].callback, self.rec[v].owner) {
            (true, Some(slot)) => Some(VoiceEnd { voice: v, slot }),
            _ => None,
        }
    }

    /// 0x1556c: free a released voice (stream priority 0x7f stays), callback (reason 2) if still attached, clear the
    /// "off" bit.
    fn free_voice(&mut self, v: usize, ends: &mut Vec<VoiceEnd>) {
        if self.rec[v].priority != STREAM_PRIORITY {
            self.list_remove(v);
        }
        if let Some(e) = self.callback_end(v) {
            ends.push(e);
        }
        self.rec[v].callback = false;
        self.off_mask[v / 24] &= !(1 << (v % 24));
    }

    /// 0x156f8: natural end of a keyed voice: callback (reason 3), detach, drop the pending key-on, queue the key-off.
    fn natural_end(&mut self, v: usize, ends: &mut Vec<VoiceEnd>) {
        if self.rec[v].state == VState::Keyed {
            if let Some(e) = self.callback_end(v) {
                ends.push(e);
            }
        }
        self.rec[v].callback = false;
        let bit = 1u32 << (v % 24);
        self.pending_kon[v / 24] &= !bit;
        self.pending_koff[v / 24] |= bit;
    }
}

impl Voices for SpuVoices {
    fn tick_begin(&mut self, tick: u32) {
        self.tick = tick;
    }

    /// 0x14d60(group, priority).
    fn alloc(&mut self, tick: u32, group: i8, priority: i8) -> Option<Granted> {
        // group >= 8 becomes 7 (0x14d74); a negative group would index before the table (unreachable)
        let g = (group as i32).clamp(0, 7) as usize;
        let (first, last) = self.ranges[g];
        let (lo, hi) = (first as i32, last as i32 + 1);
        let claim = |s: &mut SpuVoices, v: usize| {
            let r = &mut s.rec[v];
            r.state = VState::Claimed;
            r.stamp = tick;
            r.priority = priority as u8;
            r.group = g as i8;
            r.countdown = END_COUNTDOWN;
        };
        // first free voice of the group's range
        for v in lo.max(0)..hi.min(VOICES as i32) {
            if self.rec[v as usize].state == VState::Free {
                claim(self, v as usize);
                return Some(Granted { voice: v as usize, stolen: None });
            }
        }
        // victim: lowest unsigned priority, ties -> the oldest stamp (strictly smaller wins, scan from the first voice)
        if !(0..VOICES as i32).contains(&lo) {
            return None;
        }
        let mut best = lo as usize;
        for v in (lo + 1)..hi.min(VOICES as i32) {
            let (c, b) = (&self.rec[v as usize], &self.rec[best]);
            if c.priority < b.priority || (c.priority == b.priority && c.stamp < b.stamp) {
                best = v as usize;
            }
        }
        let vp = self.rec[best].priority;
        if (priority as i32) < vp as i32 || vp >= NO_STEAL_PRIORITY {
            return None;
        }
        // steal: the owner's callback runs with reason 1, the voice leaves the list and is re-keyed by the caller
        let stolen = self.callback_end(best);
        self.list_remove(best);
        claim(self, best);
        self.rec[best].callback = false;
        Some(Granted { voice: best, stolen })
    }

    /// 0x1642c + 0x14fc0.
    fn key_on(&mut self, v: usize, req: &KeyOn) {
        let tpan = if req.sound_flags & 4 != 0 { pan_scale(req.tone_pan as i32, req.scale as i32) } else { req.tone_pan as i32 };
        let mut out = [0i16; 2];
        lr_volume(&self.pan, self.out_mode, req.volume as i32, req.pan as i32, req.tone_volume as i32, tpan, &mut out);
        {
            let r = &mut self.rec[v];
            r.owner = Some(req.slot);
            r.callback = true;
            r.tone = Some(req.tone.clone());
            r.tone_volume = req.tone_volume;
            r.tone_pan = req.tone_pan;
            r.centre = req.centre;
            r.centre_fine = req.centre_fine;
            r.bend = req.bend;
            r.pmod = req.pitch_mod;
            r.group = req.group;
            r.l = out[0];
            r.r = out[1];
            r.stamp = req.tick;
        }
        self.banks[v] = Some((req.bank_handle, req.bank.clone()));
        self.stream[v] = None;
        let (core, bit) = (v / 24, 1u32 << (v % 24));
        // register order: VOLL, VOLR, VMIXL, VMIXR, PITCH, SSA, ADSR1, ADSR2, reverb mask
        self.write_volumes(v);
        self.vmix_l[core] |= bit;
        self.write(Reg::VmixL, core, self.vmix_l[core]);
        self.vmix_r[core] |= bit;
        self.write(Reg::VmixR, core, self.vmix_r[core]);
        let pitch = self.pitch_of(v, req.bend, req.pitch_mod, req.centre, req.centre_fine).unwrap_or(0);
        self.write(Reg::Pitch, v, pitch as u32);
        self.write_bank(Reg::Ssa, v, req.tone.sample, req.bank_handle);
        self.write(Reg::Adsr1, v, req.tone.adsr1 as u32);
        self.write(Reg::Adsr2, v, req.tone.adsr2 as u32);
        if req.tone.reverb() {
            self.reverb[core] |= bit;
        } else {
            self.reverb[core] &= !bit;
        }
        self.pending_kon[core] |= bit;
        self.list_add(v);
    }

    /// 0x157f8(mask, notify).
    fn key_off(&mut self, mask: VoiceMask, notify: bool) -> Vec<VoiceEnd> {
        let mut ends = Vec::new();
        for v in mask_voices(&mask).collect::<Vec<_>>() {
            if self.rec[v].state != VState::Keyed {
                continue;
            }
            let bit = 1u32 << (v % 24);
            self.pending_koff[v / 24] |= bit;
            if notify {
                if let Some(e) = self.callback_end(v) {
                    ends.push(e);
                }
            }
            self.rec[v].callback = false;
            self.pending_kon[v / 24] &= !bit;
        }
        ends
    }

    /// 0x15968(mask, notify): every voice in the mask (any state) loses its callback and its volume at once.
    fn kill(&mut self, mask: VoiceMask, notify: bool) -> Vec<VoiceEnd> {
        let mut ends = Vec::new();
        for v in mask_voices(&mask).collect::<Vec<_>>() {
            if notify && self.rec[v].state == VState::Keyed {
                if let Some(e) = self.callback_end(v) {
                    ends.push(e);
                }
            }
            self.rec[v].callback = false;
            if self.rec[v].priority != STREAM_PRIORITY {
                self.list_remove(v);
            }
            self.write(Reg::VolL, v, 0);
            self.write(Reg::VolR, v, 0);
            // 0x162b0: drop the pending key-on, queue the key-off
            let bit = 1u32 << (v % 24);
            self.pending_kon[v / 24] &= !bit;
            self.pending_koff[v / 24] |= bit;
        }
        ends
    }

    /// 0x15bb8(handler): VOLL = VOLR = PITCH = 0 for every non-free voice whose owner is `slot` (including voices that
    /// were keyed off and no longer sit in the handler's masks). `mask` is not used by the driver.
    fn pause(&mut self, slot: usize, _mask: VoiceMask) {
        for v in 0..VOICES {
            if self.rec[v].state != VState::Free && self.rec[v].owner == Some(slot) {
                self.write(Reg::VolL, v, 0);
                self.write(Reg::VolR, v, 0);
                self.write(Reg::Pitch, v, 0);
            }
        }
    }

    /// 0x15cac(mask): VOLL / VOLR from the stored L / R through the group master, PITCH from the record's tone, bend,
    /// mod, centre and fine. No state test.
    fn resume(&mut self, mask: VoiceMask) {
        for v in 0..VOICES {
            if !mask_has(&mask, v) {
                continue;
            }
            self.write_volumes(v);
            let r = &self.rec[v];
            if let Some(p) = self.pitch_of(v, r.bend, r.pmod, r.centre, r.centre_fine) {
                self.write(Reg::Pitch, v, p as u32);
            }
        }
    }

    /// 0x5aac, per voice.
    fn revolume(&mut self, v: usize, ctx: &VolCtx) {
        let r = &self.rec[v];
        let Some(tone) = r.tone.as_ref() else { return };
        let tpan = if ctx.sound_flags & 4 != 0 { pan_scale(r.tone_pan as i32, ctx.scale as i32) } else { r.tone_pan as i32 };
        let mut out = [r.l, r.r];
        lr_volume(&self.pan, self.out_mode, ctx.volume as i32, ctx.pan as i32, r.tone_volume as i32, tpan, &mut out);
        let prio = tone_priority(ctx.scale, tone) as u8;
        self.rec[v].l = out[0];
        self.rec[v].r = out[1];
        if !ctx.paused {
            self.write_volumes(v);
        }
        self.rec[v].priority = prio;
    }

    /// 0x5ffc, per voice: the record keeps bend and mod; PITCH is written unless the handler is paused.
    fn repitch(&mut self, v: usize, ctx: &PitchCtx) {
        self.rec[v].bend = ctx.bend;
        self.rec[v].pmod = ctx.pitch_mod;
        if !ctx.paused {
            if let Some(p) = self.pitch_of(v, ctx.bend, ctx.pitch_mod, ctx.centre, ctx.centre_fine) {
                self.write(Reg::Pitch, v, p as u32);
            }
        }
    }

    /// 0x1547c.
    fn apply_key_offs(&mut self, _tick: u32) {
        if self.pending_koff == [0, 0] {
            return;
        }
        let koff = self.pending_koff;
        self.write(Reg::Koff, 0, koff[0]);
        self.write(Reg::Koff, 1, koff[1]);
        self.pending_koff = [0, 0];
        for v in mask_voices(&koff).collect::<Vec<_>>() {
            self.spu[v].key_off();
        }
        for c in 0..2 {
            self.on_mask[c] &= !koff[c];
            self.off_mask[c] |= koff[c];
        }
    }

    /// 0x15388.
    fn apply_key_ons(&mut self, _tick: u32) {
        if self.pending_kon == [0, 0] {
            return;
        }
        let kon = self.pending_kon;
        let rev = self.reverb;
        self.write(Reg::VmixEL, 0, rev[0]);
        self.write(Reg::VmixER, 0, rev[0]);
        self.write(Reg::VmixEL, 1, rev[1]);
        self.write(Reg::VmixER, 1, rev[1]);
        self.write(Reg::Kon, 0, kon[0]);
        self.write(Reg::Kon, 1, kon[1]);
        self.pending_kon = [0, 0];
        for v in 0..VOICES {
            self.spu[v].reverb = rev[v / 24] & (1 << (v % 24)) != 0;
        }
        for v in mask_voices(&kon).collect::<Vec<_>>() {
            if let Some(s) = &self.stream[v] {
                let g = s.read().unwrap();
                self.spu[v].key_on(&g[..]);
                continue;
            }
            let data: &[u8] = self.banks[v].as_ref().map_or(&[], |(_, b)| &b.data[..]);
            self.spu[v].key_on(data);
        }
        for c in 0..2 {
            self.off_mask[c] &= !kon[c];
            self.on_mask[c] |= kon[c];
        }
    }

    /// 0xdec8: the SPU2 has run for one tick since the last wake; then, for every voice of the active list that has no
    /// pending key-on: ENVX != 0 reloads the countdown; ENVX 0 frees a released voice (0x1556c) or counts a keyed
    /// voice down (4 polls in a row end it, 0x156f8).
    fn poll_end(&mut self, _tick: u32) -> Vec<VoiceEnd> {
        if self.auto_step {
            self.step_spu(self.samples_per_tick);
        }
        let mut ends = Vec::new();
        for v in self.active.clone() {
            let (core, bit) = (v / 24, 1u32 << (v % 24));
            if self.pending_kon[core] & bit != 0 {
                continue;
            }
            if self.spu[v].envx() != 0 {
                self.rec[v].countdown = END_COUNTDOWN;
            } else if self.off_mask[core] & bit != 0 {
                self.free_voice(v, &mut ends);
            } else if self.on_mask[core] & bit != 0 {
                let c = self.rec[v].countdown.wrapping_sub(1);
                self.rec[v].countdown = c;
                if c == 0 {
                    self.natural_end(v, &mut ends);
                } else if c < 0 {
                    self.rec[v].countdown = END_COUNTDOWN;
                }
            }
        }
        ends
    }
}

#[cfg(test)]
#[path = "voices_tests.rs"]
mod tests;

// ---------------------------------------------------------------- savestate (oracle snapfile section)
use super::bank::Banks;
use super::persist::{In, Out};

const REGS: [Reg; 21] = [
    Reg::VolL,
    Reg::VolR,
    Reg::Pitch,
    Reg::Adsr1,
    Reg::Adsr2,
    Reg::Ssa,
    Reg::VmixL,
    Reg::VmixR,
    Reg::VmixEL,
    Reg::VmixER,
    Reg::Kon,
    Reg::Koff,
    Reg::MvolL,
    Reg::MvolR,
    Reg::EvolL,
    Reg::EvolR,
    Reg::FxOn,
    Reg::Eea,
    Reg::FxAttr,
    Reg::FxClear,
    Reg::StreamSrc,
];

impl SpuWrite {
    pub fn save(&self, o: &mut Out) {
        o.u32(self.tick);
        o.u8(REGS.iter().position(|r| *r == self.reg).unwrap() as u8);
        o.u8(self.index);
        o.u32(self.value);
        o.u32(self.bank);
    }
    pub fn load(i: &mut In) -> SpuWrite {
        SpuWrite { tick: i.u32(), reg: REGS[i.u8() as usize], index: i.u8(), value: i.u32(), bank: i.u32() }
    }
}

impl VoiceRec {
    pub fn save(&self, o: &mut Out) {
        o.u8(self.state as u8);
        o.opt_usize(self.owner);
        o.bool(self.callback);
        o.bool(self.tone.is_some());
        if let Some(t) = &self.tone {
            t.save(o);
        }
        o.i8(self.centre);
        o.i8(self.centre_fine);
        o.u8(self.priority);
        o.i8(self.group);
        o.u32(self.stamp);
        o.i16(self.l);
        o.i16(self.r);
        o.i16(self.bend);
        o.i16(self.pmod);
        o.i8(self.tone_volume);
        o.i16(self.tone_pan);
        o.i8(self.countdown);
    }
    pub fn load(i: &mut In) -> VoiceRec {
        let state = match i.u8() {
            0 => VState::Free,
            1 => VState::Keyed,
            2 => VState::Claimed,
            3 => VState::Reserved,
            _ => VState::Stream,
        };
        let owner = i.opt_usize();
        let callback = i.bool();
        let tone = if i.bool() { Some(Tone::load(i)) } else { None };
        VoiceRec {
            state,
            owner,
            callback,
            tone,
            centre: i.i8(),
            centre_fine: i.i8(),
            priority: i.u8(),
            group: i.i8(),
            stamp: i.u32(),
            l: i.i16(),
            r: i.i16(),
            bend: i.i16(),
            pmod: i.i16(),
            tone_volume: i.i8(),
            tone_pan: i.i16(),
            countdown: i.i8(),
        }
    }
}

impl SpuVoices {
    pub fn save(&self, o: &mut Out) {
        for r in &self.rec {
            r.save(o);
        }
        for s in &self.spu {
            s.save(o);
        }
        for b in &self.banks {
            o.opt_u32(b.as_ref().map(|x| x.0));
        }
        for (a, b) in self.ranges {
            o.i16(a);
            o.i16(b);
        }
        for g in self.group_vol {
            o.i32(g);
        }
        o.i16(self.out_mode);
        for m in [&self.pending_kon, &self.pending_koff, &self.on_mask, &self.off_mask, &self.reverb, &self.vmix_l, &self.vmix_r] {
            o.mask(m);
        }
        o.len(self.active.len());
        for a in &self.active {
            o.len(*a);
        }
        o.u32(self.tick);
        o.bool(self.auto_step);
        o.u32(self.samples_per_tick);
        o.len(self.events.len());
        for e in &self.events {
            e.save(o);
        }
        o.bool(self.log_events);
        o.len(self.errors.len());
        for e in &self.errors {
            o.u32(*e);
        }
        o.len(self.stream_stolen.len());
        for e in &self.stream_stolen {
            o.len(e.voice);
            o.len(e.slot);
        }
    }
    pub fn load(&mut self, i: &mut In, banks: &Banks) {
        for r in &mut self.rec {
            *r = VoiceRec::load(i);
        }
        for s in &mut self.spu {
            *s = Spu2Voice::load(i);
        }
        for b in &mut self.banks {
            *b = i.opt_u32().map(|h| (h, banks.get(h).unwrap_or_else(|| panic!("savestate: voice bank {h:#x} not loaded")).clone()));
        }
        for r in &mut self.ranges {
            *r = (i.i16(), i.i16());
        }
        for g in &mut self.group_vol {
            *g = i.i32();
        }
        self.out_mode = i.i16();
        self.pending_kon = i.mask();
        self.pending_koff = i.mask();
        self.on_mask = i.mask();
        self.off_mask = i.mask();
        self.reverb = i.mask();
        self.vmix_l = i.mask();
        self.vmix_r = i.mask();
        let n = i.len();
        self.active = (0..n).map(|_| i.len()).collect();
        self.tick = i.u32();
        self.auto_step = i.bool();
        self.samples_per_tick = i.u32();
        let n = i.len();
        self.events = (0..n).map(|_| SpuWrite::load(i)).collect();
        self.log_events = i.bool();
        let n = i.len();
        self.errors = (0..n).map(|_| i.u32()).collect();
        let n = i.len();
        self.stream_stolen = (0..n).map(|_| VoiceEnd { voice: i.len(), slot: i.len() }).collect();
    }
}
