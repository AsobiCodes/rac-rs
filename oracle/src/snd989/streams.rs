//! VAG streams of the 989snd stream engine (music, speech, stinger): handles, voices, key-on registers, play position.
//! Sources, all [C] unless marked (IOP module addresses, disassembly `scratch_grain/full.dis`, handoff_streams.md):
//! - cid 44 0xb18 -> 0xeacc; voices 0xe8a4 (chan voices, 0xe948 / 0xe9d8) and 0xede8 (handler voice); handle 0x14140.
//! - key-on 0x131ec (VOL via 0x16780 + 0x16dcc, VMIX dry, PITCH chan+8, SSA, ADSR 0xf / 0x1fc0, pending KON 0x198d8).
//! - cid 25 0xb278, cid 79 0x108f8, cid 50 0x107e0 (+ 0x1070c elapsed, 0x10860 total), cid 33 stream branch 0x14354.
//! - lookup 0xcadc (type 4 = stream, pool [0x19780] x [0x19784], stride 0x50).
//! The PCM is NOT here: the caller decodes the VAG keyed by lba (events [`StreamEv`]); this module owns the driver side.
//!
//! What a stream holds (verified): a mono stream = 1 voice, plus a second voice when flags & 0x20 (the same ring buffer,
//! L/R swapped, both hard-panned: a mono source on both speakers); a stereo stream (lba_r != 0) = 2 voices, one per
//! channel. All are state 4, priority 0x7f (never stolen), allocated in the stream's group range (boot cid 78 puts groups
//! 1, 2, 4 on {0x18, 0x2f}: the first free voice from 0x18 up), not in the active list (no end poll).
//!
//! Timing model (stage 6c, HANDOFF_6c.md), three parts.
//! - Position (exact): after key-on the SPU2 consumes `pitch * 200 / 4096` samples per driver tick (200 samples of 48 kHz
//!   per tick), so `pos` advances with the PITCH register value (0 while held / paused). Total (chan +0x94) =
//!   (data bytes >> 4) * 28 samples. The disc read rate does not move it as long as the read keeps ahead.
//! - END RULE (0xfe74 -> 0x101b8) [C]: there is no compare of chan +0x98 with chan +0x94 anywhere. Once chan flag 0x40 is
//!   set (the last data is queued: 0x13100 / 0x13194), the stream pass reads ENVX (GetParam 0x500 | core | vox << 1,
//!   0x10550..0x105a4); ENVX == 0 = natural end, noticed in the first stream pass after it (chain hand-off, else
//!   0xfc48(h, 1)). The model keeps `pos >= total` as "the SPU2 has consumed the end-flagged block" (envelope 0 right
//!   after it); the end is only noticed at a stream pass, and the first pass after key-on does nothing (chan flags 6 -> 8
//!   at 0x105d0).
//! - ELAPSED (chan +0x98, cid 50 via 0x1070c) [C]: NAX deltas. NAX = GetAddr(0x2240 | core | vox << 1) (3 reads with a vote,
//!   0x10208..0x102e8), E += NAX' - prev with prev initialised to SSA = chan +0x18 + chan +0x88 + 0x30 at key-on and a
//!   half-ring correction on a half change (0x1063c). No rounding in the IRX: the granularity is the steps NAX takes (the
//!   model: one 16-byte block per 28 samples [I: NAX phase is a property of the SPU2 emulation, OPEN]). E is updated
//!   once per stream pass, frozen while paused (chan flag 0x10), and the first pass after key-on is skipped.
//! - START (no tick count exists in the IRX): an event chain of CD reads and SPU2 transfers ([`start_chain`]); the disc
//!   model [`DiscTiming`] gives the ticks per event, default INSTANT (the oracle completes reads at once inside the RPC).
//!   A held stream (flags & 1) is keyed at that moment with VOL = PITCH = 0 and plays when cid 46 arrives.
//! Handle: alive word = `0x84000000 | idx << 16 | serial`: 0x14140 writes `0x04000000 | idx << 16 | (serial + 1)` and the
//! end of cid 44 (0xc8a4, called at 0xf04c) ORs bit 31 and links the handler into the tick list; 0xc924 clears bit 31 on kill.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

use super::voices::{stream_lr, volume_reg, Reg, SpuVoices, StreamData};

/// Handle type 4 in bits 24..28 plus the alive bit 31 (`0x84000000 | idx << 16 | serial`: 0x14140 writes the type, serial
/// and index, 0xc8a4 at the end of cid 44 sets bit 31; 0xc924 clears it).
pub const HANDLE_TYPE: u32 = 0x8400_0000;
pub const ADSR1: u32 = 0x000f;
pub const ADSR2: u32 = 0x1fc0;
/// chan +6 of the left channel of a stereo stream (header parse 0x11d24) and of the first channel when the handler holds a
/// second voice (0x13328): rotates the pan by 270 degrees.
pub const EXTRA_PAN: i32 = 0x10e;
/// chan +6 of the right channel of a stereo stream (0x11d44): 90 degrees.
pub const RIGHT_PAN: i32 = 0x5a;
/// chan +4 and the handler group bytes 0x34..0x43 (0x11d4c / 0x11d1c; 0xe51c, 0xeebc).
pub const CHAN_VOL: i32 = 0x7f;
/// cid 42 default of the oracle's EE call (4, 0xf000, 0, 1): ring size `fp` of one chan pair of halves.
pub const DEFAULT_RING_BYTES: u32 = 0xf000;
/// Samples the SPU2 reads per tick = 48000 / 240.
pub const SAMPLES_PER_TICK: u64 = 200;

/// What the VAG header gives (+0xc data size, +0x10 sample rate; both big endian in the file).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VagInfo {
    pub rate: u32,
    pub data_bytes: u32,
}

/// Header lookup by disc sector (the file's first sector).
pub trait VagSource {
    fn vag(&self, lba: u32) -> Option<VagInfo>;
}
impl VagSource for HashMap<u32, VagInfo> {
    fn vag(&self, lba: u32) -> Option<VagInfo> {
        self.get(&lba).copied()
    }
}

/// cid 44 arguments as 0xeacc unpacks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayVag {
    pub lba: u32,
    /// 0 = mono.
    pub lba_r: u32,
    /// d2 low 16 / d3 low 16: chan +0x88 of the left / right channel = byte offset of the VAG header inside the first
    /// disc sector (0xf330 at 0xef48 / 0xefc8; used at 0x11b10, 0x11444, 0x12464); SSA = ring + this + 0x30.
    pub ring: [u16; 2],
    /// d2 >> 16: volume 0..1024 (stored as `v * 127 >> 10`).
    pub volume: u32,
    /// d3 >> 16 stored as s16.
    pub pan: i16,
    pub group: i8,
    /// Handle of the stream to queue behind (0 = start a new stream).
    pub after: u32,
    /// 1 start held, 2 reverb, 4 loop, 8 wait for cid 46 before the queue advances [?], 0x10 fail if `after` is unknown,
    /// 0x20 a mono source plays on two voices.
    pub flags: u32,
}

impl PlayVag {
    /// The seven words the wrapper 0xb18 reads from the RPC buffer.
    pub fn cid44(d: &[u32; 7]) -> PlayVag {
        PlayVag { lba: d[0], lba_r: d[1], ring: [d[2] as u16, d[3] as u16], volume: d[2] >> 16, pan: (d[3] >> 16) as u16 as i16, group: d[4] as i8, after: d[5], flags: d[6] }
    }
}

/// Events for the PCM side (the register writes go to `SpuVoices.events`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamEv {
    /// Stream voices keyed: play the VAG at `lba` from sample 0 on `voice` (and `extra` with L/R swapped; 0 = held).
    Start { tick: u32, voice: usize, extra: Option<usize>, lba: u32, held: bool },
    /// The queued part starts (gapless): the VAG at `lba` replaces the playing one on the same voices.
    Next { tick: u32, voice: usize, lba: u32 },
    /// The stream was stopped or ended.
    End { tick: u32, voice: usize, natural: bool },
}

/// Disc model of the start chain: ticks per CD read and per SPU2 transfer. `INSTANT` (0 / 0) is the oracle's model: its
/// CDVD completes reads at once inside the RPC handler (`iso_read`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiscTiming {
    pub cd_read_ticks: u32,
    pub spu_dma_ticks: u32,
}

impl DiscTiming {
    pub const INSTANT: DiscTiming = DiscTiming { cd_read_ticks: 0, spu_dma_ticks: 0 };
}

/// The event chain from cid 44 to the first key-on [C] (threads 0x10938 / 0x116e8 / 0x12d60 / 0x1295c, parse 0x11b00):
/// `t = data bytes + chan +0x88 + 0x30`, `half` = one half ring. Returns (CD reads, SPU2 transfers) that must complete:
/// short (t <= half): header + data read, 1 transfer; mid (half < t <= 2 half): 3 reads, 2 transfers; long: header +
/// 3 half-ring reads, transfer 0 done (the key-on comes from the CD completion at read 3).
pub fn start_chain(data_bytes: u32, ring_off: u16, half: u32) -> (u32, u32) {
    let t = (data_bytes as u64) + ring_off as u64 + 0x30;
    let half = half as u64;
    if t <= half {
        (2, 1)
    } else if t <= 2 * half {
        (3, 2)
    } else {
        (4, 2)
    }
}

#[derive(Clone, Copy, Debug)]
struct Seg {
    lba: u32,
    lba_r: u32,
    flags: u32,
}

/// One stream: the 0x50-byte handler entry plus its channels.
#[derive(Clone, Debug)]
pub struct Stream {
    pub handle: u32,
    pub group: i8,
    /// Flags of the playing part (queued parts replace it).
    pub flags: u32,
    /// +0x10 volume (0..127), +0x12 pan.
    pub vol: i16,
    pub pan: i16,
    /// +0x16 bit 1 / bit 3.
    pub held: bool,
    pub buffered: bool,
    /// chan +0x78 of the left and right channel; +0x30 of the handler.
    pub voices: [Option<usize>; 2],
    pub extra: Option<usize>,
    pub lba: [u32; 2],
    pub ring: [u16; 2],
    /// Chan pool index of the left / right channel (0x1472c: lowest free). SSA = spu_base + chan * half + ring + 0x30.
    pub chan: [usize; 2],
    /// The ADPCM data of each channel's ring (stage 11): (stream id, data), made at the key-on from `Streams::payload`.
    pub bufs: [Option<(u32, StreamData)>; 2],
    /// chan +6 of both channels, set at the key-on (header parse).
    pub pan_off: [i32; 2],
    /// Driver tick of cid 44.
    created: u32,
    /// Driver tick of the first key-on, computed at the first tick that sees the stream (`created` + chain).
    key_at: Option<u32>,
    pub keyed: bool,
    /// Handler flag 4 (cid 21): muted, waiting for the tick pass that frees it (the handle stays alive until then).
    pub stopping: bool,
    /// The first stream pass after key-on is skipped (chan flags 6 -> 8).
    first_pass: bool,
    /// chan +0x98 as sampled at the last stream pass.
    elapsed: u32,
    /// chan +0x90 style state: the current part's header.
    pub info: Option<VagInfo>,
    /// PITCH register value of the part (chan +8) and the live value (0 while held / paused).
    pub pitch: u16,
    pub pitch_live: u16,
    /// Samples read by the SPU2 in 1/4096 sample.
    pos: u64,
    queue: VecDeque<Seg>,
}

impl Stream {
    pub fn stereo(&self) -> bool {
        self.lba[1] != 0
    }
    /// chan flags & 0x400 (flags & 4): endless, cid 50 answers 0.
    pub fn looping(&self) -> bool {
        self.flags & 4 != 0
    }
    /// Samples read so far (position in the current part).
    pub fn samples(&self) -> u64 {
        self.pos >> 12
    }
    /// chan +0x94: total samples of the part.
    pub fn total_samples(&self) -> u32 {
        self.info.map_or(0, |i| (i.data_bytes >> 4) * 28)
    }
    /// chan +0x98 as of the last stream pass: NAX - SSA, one 16-byte block per 28 samples.
    pub fn elapsed_bytes(&self) -> u32 {
        self.elapsed
    }
    fn sample_elapsed(&self) -> u32 {
        ((self.samples() / 28) as u32).wrapping_mul(16)
    }
    /// Pause / continue order: chan 0, the extra voice, chan 1.
    fn all_voices(&self) -> Vec<usize> {
        let mut o: Vec<usize> = self.voices[0].into_iter().collect();
        o.extend(self.extra);
        o.extend(self.voices[1]);
        o
    }
    /// Stop path order (0xfc48): the extra voice first, then the channels.
    fn release_order(&self) -> Vec<usize> {
        let mut o: Vec<usize> = self.extra.into_iter().collect();
        o.extend(self.voices.iter().flatten().copied());
        o
    }
}

/// 0x1070c / 0x10860: `samples / rate` in 1/1024 s: `(s / r) << 10 + ((s % r) << 10) / r`.
pub fn fixed_time(samples: u32, rate: u32) -> u32 {
    if rate == 0 {
        return 0;
    }
    ((samples / rate) << 10).wrapping_add(((samples % rate) << 10) / rate)
}

/// 0x1070c: bytes -> samples (28 per 16-byte block, the remainder above 4 bytes adds 2 per byte).
pub fn bytes_to_samples(bytes: u32) -> u32 {
    let mut s = (bytes >> 4) * 28;
    let rem = bytes & 0xf;
    if rem >= 5 {
        s += (rem - 4) * 2;
    }
    s
}

/// chan +8: `((rate << 12) * 0x057619f1) >> 42` (0x11b7c), the PITCH register of a rate at 48 kHz.
pub fn pitch_of_rate(rate: u32) -> u16 {
    ((((rate << 12) as u64) * 0x0576_19f1) >> 42) as u16
}

pub struct Streams {
    /// [0x197d4]: cid 42 has run.
    pub inited: bool,
    slots: Vec<Option<Stream>>,
    serial: Vec<u16>,
    chan_used: Vec<bool>,
    /// One half ring in bytes = [0x19790] = fp / 2.
    pub half: u32,
    pub disc: DiscTiming,
    /// First SPU2 RAM address of the ring block (return of the SPU2 allocator 0xd640(fp * N) in cid 42, 0xe6cc / 0xe6d4):
    /// runtime dependent, a parameter until the allocator is modelled (STAGES.md, integration item).
    pub spu_base: u32,
    pub events: Vec<StreamEv>,
    /// Driver error codes seen (0x2f: cid 44 before init, 0x25: second cid 42).
    pub errors: Vec<u32>,
    /// The VAG payloads (ADPCM blocks after the 0x30-byte header) of the files cid 44 named, by lba of the file's first
    /// sector, read from the disc by the host (stage 11). Streams without a payload run silent (register writes only).
    pub payload: HashMap<u32, Arc<Vec<u8>>>,
    /// Next stream id for `Reg::StreamSrc`.
    next_id: u32,
}

/// ADPCM block flags (byte 1): 1 loop end, 2 repeat, 4 loop start.
const BLOCK: usize = 16;

/// A part's data as the stream ring presents it to the SPU2 (stage 11). The files carry [.., 1, 7] at their end (loop end
/// without repeat, then loop end + repeat + start), so a part played alone ends by itself after its next-to-last block.
/// A looping part (cid 44 flag 4: the IOP re-reads the file) starts with a loop-start block and ends with loop end +
/// repeat, jumping back to its first block. [I: the IOP's ring patch 0x1383c sets these flags per ring half; the audible
/// result is the file's blocks in sequence.]
pub fn new_part_buf(raw: &[u8], looping: bool) -> Vec<u8> {
    let mut b = raw[..raw.len() & !(BLOCK - 1)].to_vec();
    if looping && b.len() >= BLOCK {
        b[1] |= 4;
        let n = b.len();
        if n >= 2 * BLOCK {
            b[n - 2 * BLOCK + 1] = 0;
        }
        b[n - BLOCK + 1] = 3;
    }
    b
}

/// A part queued behind the data already in `buf` (gapless: the SPU2 reads straight on). The previous part's end flags
/// are cleared (the ring continues with the next file's blocks), then the new part is added as [`new_part_buf`].
pub fn append_part(buf: &mut Vec<u8>, raw: &[u8], looping: bool) {
    let n = buf.len();
    for k in 1..=2 {
        if n >= k * BLOCK {
            buf[n - k * BLOCK + 1] = 0;
        }
    }
    buf.extend_from_slice(&new_part_buf(raw, looping));
}

impl Streams {
    /// Before cid 42 (0xe1f0): cid 44 fails with error 0x2f.
    pub fn new() -> Streams {
        Streams {
            inited: false,
            slots: Vec::new(),
            serial: Vec::new(),
            chan_used: Vec::new(),
            half: DEFAULT_RING_BYTES / 2,
            disc: DiscTiming::INSTANT,
            spu_base: 0x5040,
            events: Vec::new(),
            errors: Vec::new(),
            payload: HashMap::new(),
            next_id: 1,
        }
    }

    /// Stage 11: the ADPCM data of a stream's channels (left, right) at its key-on, queued parts included.
    fn make_bufs(&mut self, s: &mut Stream) {
        for ch in 0..2 {
            let lba = s.lba[ch];
            let Some(raw) = (lba != 0).then(|| self.payload.get(&lba).cloned()).flatten() else { continue };
            let mut b = new_part_buf(&raw, s.looping());
            for seg in &s.queue {
                let l = if ch == 0 { seg.lba } else { seg.lba_r };
                if let Some(r) = self.payload.get(&l) {
                    append_part(&mut b, r, seg.flags & 4 != 0);
                }
            }
            let id = self.next_id;
            self.next_id += 1;
            s.bufs[ch] = Some((id, Arc::new(RwLock::new(b))));
        }
    }

    /// cid 42 with the oracle's EE call (4, 0xf000, 0, 1).
    pub fn init(&mut self, count: usize) {
        self.init_cfg(count, DEFAULT_RING_BYTES);
    }

    /// cid 42 (0xe1f0): `count` handler entries and chans (a0 = [0x19784]); `buf_bytes` (a1) rounded up to 4 KB pages, at
    /// least 2 pages (0xe274..0xe294): fp = pages << 12, half ring = fp / 2. A second call is error 0x25 ([0x197d4] is the
    /// chan list head). Every serial starts at 1 (the pool word is `idx << 16 | 0x400001`, 0xe478..0xe540).
    pub fn init_cfg(&mut self, count: usize, buf_bytes: u32) -> bool {
        if self.inited {
            self.errors.push(0x25);
            return false;
        }
        self.inited = true;
        let pages = ((buf_bytes as u64 + 0xfff) >> 12).max(2) as u32;
        self.half = (pages << 12) / 2;
        self.slots = vec![None; count];
        self.serial = vec![1; count];
        self.chan_used = vec![false; count];
        true
    }

    /// Lowest free chan (0x1472c, first-fit over the free list).
    fn alloc_chan(&mut self) -> Option<usize> {
        let c = self.chan_used.iter().position(|u| !*u)?;
        self.chan_used[c] = true;
        Some(c)
    }

    /// SPU2 RAM address of a chan's ring (chan +0x18, half A): `spu_base + chan * half`.
    pub fn ring_base(&self, chan: usize) -> u32 {
        self.spu_base.wrapping_add((chan as u32).wrapping_mul(self.half))
    }

    /// 0xcadc type 4: the slot of a live handle (the word must match exactly, so a reused slot rejects the old handle).
    pub fn find(&self, handle: u32) -> Option<usize> {
        if (handle >> 24) & 0x1f != 4 {
            return None;
        }
        let idx = ((handle >> 16) & 0xff) as usize;
        match self.slots.get(idx) {
            Some(Some(s)) if s.handle == handle => Some(idx),
            _ => None,
        }
    }

    pub fn stream(&self, handle: u32) -> Option<&Stream> {
        self.find(handle).and_then(|i| self.slots[i].as_ref())
    }

    /// cid 44. Returns the reply word: the handle for a new stream, 1 for a part queued behind `after`, 0 on failure.
    pub fn play(&mut self, now: u32, v: &mut SpuVoices, a: &PlayVag) -> u32 {
        if !self.inited {
            self.errors.push(0x2f);
            return 0;
        }
        let mut a = *a;
        if a.after != 0 {
            match self.find(a.after) {
                Some(i) => {
                    // a part behind the playing one: no new voice, no new handle (0xec68 / 0xebbc)
                    let s = self.slots[i].as_mut().unwrap();
                    if s.stereo() == (a.lba_r != 0) {
                        s.queue.push_back(Seg { lba: a.lba, lba_r: a.lba_r, flags: a.flags });
                    } else {
                        // mono part behind a stereo stream or the reverse: one segment on the left chain only
                        s.queue.push_back(Seg { lba: a.lba, lba_r: if s.stereo() { a.lba_r } else { 0 }, flags: a.flags });
                    }
                    // a playing stream: the ring goes on with the new part's blocks
                    for ch in 0..2 {
                        let l = if ch == 0 { a.lba } else { a.lba_r };
                        if let (Some((_, d)), Some(raw)) = (&s.bufs[ch], self.payload.get(&l)) {
                            append_part(&mut d.write().unwrap(), raw, a.flags & 4 != 0);
                        }
                    }
                    if a.flags & 8 != 0 {
                        // 0xeb94: +0x4c := 0 (the queue waits)
                    }
                    return 1;
                }
                None if a.flags & 0x10 != 0 => return 0,
                None => a.after = 0,
            }
        }
        let Some(idx) = self.slots.iter().position(|s| s.is_none()) else {
            self.errors.push(0x30);
            return 0;
        };
        // chans first (1 per stream, 2 when stereo); no free chan fails the call [? error code of 0x1472c unread]
        let stereo = a.lba_r != 0;
        let need = if stereo { 2 } else { 1 };
        if self.chan_used.iter().filter(|u| !**u).count() < need {
            return 0;
        }
        // 0xe8a4: chan voices (left, then right when stereo); failure frees the left one and fails the call
        let Some(vl) = v.alloc_stream(a.group) else { return 0 };
        let vr = if stereo {
            match v.alloc_stream(a.group) {
                Some(x) => Some(x),
                None => {
                    v.free_stream_voice(vl);
                    return 0;
                }
            }
        } else {
            None
        };
        // 0xede8: a mono source with flags & 0x20 holds a second voice (a failed alloc is ignored)
        let extra = if a.flags & 0x20 != 0 && !stereo { v.alloc_stream(a.group) } else { None };
        let serial = self.serial[idx].wrapping_add(1);
        self.serial[idx] = serial;
        let handle = HANDLE_TYPE | (idx as u32) << 16 | serial as u32;
        let c0 = self.alloc_chan().unwrap();
        let c1 = if stereo { self.alloc_chan().unwrap() } else { c0 };
        let s = Stream {
            handle,
            group: a.group,
            flags: a.flags,
            vol: ((a.volume.wrapping_mul(127) as i32) >> 10) as i16,
            pan: a.pan,
            held: a.flags & 1 != 0,
            buffered: false,
            voices: [Some(vl), vr],
            extra,
            lba: [a.lba, a.lba_r],
            ring: a.ring,
            chan: [c0, c1],
            bufs: [None, None],
            pan_off: [0, 0],
            created: now,
            key_at: None,
            keyed: false,
            stopping: false,
            first_pass: false,
            elapsed: 0,
            info: None,
            pitch: 0,
            pitch_live: 0,
            pos: 0,
            queue: VecDeque::new(),
        };
        self.slots[idx] = Some(s);
        handle
    }

    /// cid 25 (0xb278): the handle while the stream exists, else 0.
    pub fn playing(&self, handle: u32) -> u32 {
        if handle == 0 || handle == u32::MAX {
            return 0;
        }
        if self.find(handle).is_some() {
            handle
        } else {
            0
        }
    }

    /// cid 79 (0x108f8): -1 unknown handle, else handler +0x16 bit 3.
    pub fn buffered(&self, handle: u32) -> i32 {
        match self.stream(handle) {
            None => -1,
            Some(s) => s.buffered as i32,
        }
    }

    /// cid 50 (0x107e0): remaining time in 1/1024 s = total - elapsed; 0 for an unknown handle or a looping part.
    pub fn remaining(&self, handle: u32) -> u32 {
        let Some(s) = self.stream(handle) else { return 0 };
        let Some(info) = s.info else { return 0 };
        if s.looping() || info.rate == 0 {
            return 0;
        }
        let elapsed = fixed_time(bytes_to_samples(s.elapsed_bytes()), info.rate);
        fixed_time(s.total_samples(), info.rate).wrapping_sub(elapsed)
    }

    /// cid 45 (0xf5bc, under lock 0xde60): a complete no-op when the held bit (handler +0x16 bit 1) is already set. Else
    /// the bit goes up first, then every chan gets hold flag 0x10 and VOLL, VOLR, PITCH := 0 in the order chan 0, the extra
    /// voice, chan 1. No test of the chan state: a not-yet-keyed chan is written too.
    pub fn pause(&mut self, v: &mut SpuVoices, handle: u32) {
        let Some(i) = self.find(handle) else { return };
        let s = self.slots[i].as_mut().unwrap();
        if s.held {
            return;
        }
        s.held = true;
        s.pitch_live = 0;
        for x in s.all_voices() {
            v.stream_write(Reg::VolL, x, 0);
            v.stream_write(Reg::VolR, x, 0);
            v.stream_write(Reg::Pitch, x, 0);
        }
    }

    /// cid 46 (0xf7ec): a complete no-op when the held bit is clear (even if a chan carries hold flag 0x10) or the handle is
    /// unknown. Else the bit is cleared first, then phase 1 writes PITCH := chan +8 of every keyed chan (chan 0, the extra
    /// voice with chan 0's pitch, chan 1), phase 2 the hold flags are cleared and every keyed chan gets VOLL, VOLR from its
    /// record L / R through the group master (chan 0, extra voice with its own record, chan 1). A chan that is not keyed
    /// gets no write: its later key-on programs the real values. Also starts a stream created held.
    pub fn resume(&mut self, v: &mut SpuVoices, handle: u32) {
        let Some(i) = self.find(handle) else { return };
        let s = self.slots[i].as_mut().unwrap();
        if !s.held {
            return;
        }
        s.held = false;
        s.flags &= !1;
        if s.keyed {
            s.pitch_live = s.pitch;
            let order = s.all_voices();
            for &x in &order {
                v.stream_write(Reg::Pitch, x, s.pitch as u32);
            }
            for &x in &order {
                let (l, r, g) = (v.rec[x].l, v.rec[x].r, v.rec[x].group as i32);
                v.stream_write(Reg::VolL, x, volume_reg(l, g, &v.group_vol) as u32);
                v.stream_write(Reg::VolR, x, volume_reg(r, g, &v.group_vol) as u32);
            }
        }
    }

    /// cid 33 on a stream handle (0x14354): see [`update_stream`]. False for an unknown handle.
    pub fn update(&mut self, v: &mut SpuVoices, handle: u32, scale: i32, pan: i32) -> bool {
        let Some(i) = self.find(handle) else { return false };
        let s = self.slots[i].as_mut().unwrap();
        update_stream(v, s, scale, pan);
        true
    }

    /// cid 21 (0xb230 -> 0xcfe8 -> 0xd098 -> 0xfb30), on the EE RPC thread: the mute 0x14354(handle, 0, 0) (volume 0, pan
    /// 0, L = R = 0, VOLL = VOLR = 0 written unless held) and handler flag 4. No key-off, no free: the handle stays alive
    /// (cid 25 keeps answering) until the next stream pass, [`Streams::tick`].
    pub fn stop(&mut self, v: &mut SpuVoices, handle: u32) -> bool {
        match self.find(handle) {
            Some(i) => {
                let s = self.slots[i].as_mut().unwrap();
                mute(v, s);
                true
            }
            None => false,
        }
    }

    /// cid 52 (0xf3c8): every alive stream is stopped as by cid 21, then the first stream pass frees them (the retail call
    /// blocks the EE up to 0x1e0 waits until the tick thread has freed all of them).
    pub fn stop_all(&mut self, now: u32, v: &mut SpuVoices) {
        for i in 0..self.slots.len() {
            if let Some(s) = self.slots[i].as_mut() {
                mute(v, s);
            }
        }
        for i in 0..self.slots.len() {
            self.pass_stopping(now, v, i);
        }
    }

    /// The flag-4 branch of the stream pass (0xfe74 -> 0xfc48): voices released (extra first, then the chans), the chans
    /// returned, the handle dead. The CD / SPU2 request-in-flight retry (0xfbe0 / 0xfb94) never holds it up in the disc
    /// model. Returns true when the slot was freed.
    fn pass_stopping(&mut self, now: u32, v: &mut SpuVoices, i: usize) -> bool {
        if !self.slots[i].as_ref().is_some_and(|s| s.stopping) {
            return false;
        }
        let s = self.slots[i].take().unwrap();
        self.release(now, v, &s, false);
        true
    }

    /// 0xfc48: per voice 0x16154 (state 1, priority cleared by rec +0x1a := 0) and 0x156f8 (pending key-on dropped, pending
    /// key-off set). The record is freed by the normal ENVX poll. The chans go back to the pool.
    fn release(&mut self, now: u32, v: &mut SpuVoices, s: &Stream, natural: bool) {
        for x in s.release_order() {
            v.stream_release(x);
            if s.keyed {
                self.events.push(StreamEv::End { tick: now, voice: x, natural });
            }
        }
        self.chan_used[s.chan[0]] = false;
        self.chan_used[s.chan[1]] = false;
    }

    /// Per driver tick, before the handler pass (so the pending key-on is applied by the same iteration). One stream pass
    /// (0xfe74) per handler: the stop branch first, then the first key-on, then position / elapsed / end.
    pub fn tick(&mut self, now: u32, v: &mut SpuVoices, vag: &dyn VagSource) {
        for i in 0..self.slots.len() {
            if self.pass_stopping(now, v, i) {
                continue;
            }
            let Some(mut s) = self.slots[i].take() else { continue };
            if !s.keyed {
                let Some(info) = vag.vag(s.lba[0]) else {
                    // unknown VAG header: the stream is dropped (the driver's error code was not read [?])
                    self.release(now, v, &s, false);
                    continue;
                };
                let key_at = match s.key_at {
                    Some(k) => k,
                    None => {
                        let (reads, transfers) = start_chain(info.data_bytes, s.ring[0], self.half);
                        let k = s.created + reads * self.disc.cd_read_ticks + transfers * self.disc.spu_dma_ticks;
                        s.key_at = Some(k);
                        k
                    }
                };
                if now < key_at {
                    self.slots[i] = Some(s);
                    continue;
                }
                s.info = Some(info);
                s.pitch = pitch_of_rate(info.rate);
                s.pitch_live = if s.held { 0 } else { s.pitch };
                s.buffered = true;
                s.keyed = true;
                s.first_pass = true;
                s.elapsed = 0;
                self.make_bufs(&mut s);
                key_on(v, &mut s, self.spu_base, self.half);
                self.events.push(StreamEv::Start { tick: now, voice: s.voices[0].unwrap(), extra: s.extra, lba: s.lba[0], held: s.held });
                self.slots[i] = Some(s);
                continue;
            }
            s.pos += s.pitch_live as u64 * SAMPLES_PER_TICK;
            if s.first_pass {
                // chan flags 6 -> 8 (0x105d0): no elapsed update, no end test
                s.first_pass = false;
                self.slots[i] = Some(s);
                continue;
            }
            let total = s.total_samples() as u64;
            if total != 0 && s.samples() >= total {
                if let Some(seg) = s.queue.pop_front() {
                    // gapless: the next part continues with the carry-over of the position
                    s.pos -= total << 12;
                    s.lba = [seg.lba, seg.lba_r];
                    s.flags = seg.flags;
                    if let Some(info) = vag.vag(seg.lba) {
                        s.info = Some(info);
                        let p = pitch_of_rate(info.rate);
                        if p != s.pitch {
                            s.pitch = p;
                            if !s.held {
                                s.pitch_live = p;
                            }
                            for x in s.all_voices() {
                                v.stream_write(Reg::Pitch, x, s.pitch_live as u32);
                            }
                        }
                    }
                    self.events.push(StreamEv::Next { tick: now, voice: s.voices[0].unwrap(), lba: seg.lba });
                } else if s.looping() {
                    s.pos -= total << 12;
                } else {
                    // ENVX == 0 after the end-flagged block (0x101b8): natural end
                    self.release(now, v, &s, true);
                    continue;
                }
            }
            if !s.held {
                s.elapsed = s.sample_elapsed();
            }
            self.slots[i] = Some(s);
        }
    }

    pub fn take_events(&mut self) -> Vec<StreamEv> {
        std::mem::take(&mut self.events)
    }

    /// Number of live streams.
    pub fn count(&self) -> usize {
        self.slots.iter().flatten().count()
    }
}

/// 0x14354 tail, stream branch: volume `s16(((scale << 7) - scale) >> 10)` with 32-bit wrapping arithmetic (negative scale
/// = `-scale` as s16, 0x7fffffff keeps), clamp to 0x7f at >= 0x80 (no lower clamp); pan -1 = 0, -2 keeps, else the low 16
/// bits unchecked. Then, for a keyed stream, per channel entry (the counter counts skipped entries too): L / R =
/// `stream_lr` with the chan pan offset, stored in the voice record; chan 0 with an extra voice copies them swapped into
/// the extra record (raw, even when held); unless held, both volumes are computed first, then the writes go: chan VOLL,
/// VOLR; for chan 0 with an extra voice, extra VOLR := Lv, extra VOLL := Rv. An unkeyed stream has no chan +4/5/6 yet:
/// only the handler fields change.
fn update_stream(v: &mut SpuVoices, s: &mut Stream, scale: i32, pan: i32) {
    if scale != 0x7fff_ffff {
        s.vol = if scale < 0 { scale.wrapping_neg() as i16 } else { (scale.wrapping_shl(7).wrapping_sub(scale) >> 10) as i16 };
    }
    if s.vol >= 0x80 {
        s.vol = 0x7f;
    }
    match pan {
        -1 => s.pan = 0,
        -2 => {}
        p => s.pan = p as i16,
    }
    if !s.keyed {
        return;
    }
    let mode = v.out_mode;
    for ch in 0..2 {
        let Some(x) = s.voices[ch] else { continue };
        let mut lr = [0i16; 2];
        stream_lr(&v.pan, mode, s.vol as i32, s.pan as i32, CHAN_VOL, CHAN_VOL, s.pan_off[ch], &mut lr);
        v.rec[x].l = lr[0];
        v.rec[x].r = lr[1];
        let extra = if ch == 0 { s.extra } else { None };
        if let Some(e) = extra {
            v.rec[e].r = lr[0];
            v.rec[e].l = lr[1];
        }
        if s.held {
            continue;
        }
        let g = v.rec[x].group as i32;
        let lv = volume_reg(lr[0], g, &v.group_vol) as u32;
        let rv = volume_reg(lr[1], g, &v.group_vol) as u32;
        v.stream_write(Reg::VolL, x, lv);
        v.stream_write(Reg::VolR, x, rv);
        if let Some(e) = extra {
            v.stream_write(Reg::VolR, e, lv);
            v.stream_write(Reg::VolL, e, rv);
        }
    }
}

/// 0xfb30: 0x14354(handle, 0, 0), then handler flag 4.
fn mute(v: &mut SpuVoices, s: &mut Stream) {
    update_stream(v, s, 0, 0);
    s.stopping = true;
}

/// 0x131ec for one stream: the extra voice first (when the handler holds one), then the channel voices in order.
/// Per voice: VOLL, VOLR, VMIXL, VMIXR (dry), PITCH, SSA, ADSR1, ADSR2; then chan flags |= 4 and the pending key-on and
/// reverb bit. VOL / PITCH are 0 while the stream is held. The chan pans are those of the header parse (0x11d24, 0x11d44)
/// and of 0x13328: left channel 0x10e when stereo or when an extra voice exists, right channel 0x5a when stereo.
fn key_on(v: &mut SpuVoices, s: &mut Stream, spu_base: u32, half: u32) {
    let mode = v.out_mode;
    let held = s.held;
    let stereo = s.voices[1].is_some();
    s.pan_off = [if stereo || s.extra.is_some() { EXTRA_PAN } else { 0 }, if stereo { RIGHT_PAN } else { 0 }];
    let mut plan: Vec<(usize, usize, bool)> = Vec::new(); // (voice, channel, swapped)
    if let Some(x) = s.extra {
        plan.push((x, 0, true));
    }
    for ch in 0..2 {
        if let Some(x) = s.voices[ch] {
            plan.push((x, ch, false));
        }
    }
    // L / R of the channel voice first (the extra voice copies them swapped, 0x133cc)
    let mut lr: [[i16; 2]; 2] = [[0; 2]; 2];
    for ch in 0..2 {
        if s.voices[ch].is_some() {
            stream_lr(&v.pan, mode, s.vol as i32, s.pan as i32, CHAN_VOL, CHAN_VOL, s.pan_off[ch], &mut lr[ch]);
        }
    }
    for (x, ch, swapped) in plan {
        let (l, r) = if swapped { (lr[ch][1], lr[ch][0]) } else { (lr[ch][0], lr[ch][1]) };
        v.rec[x].l = l;
        v.rec[x].r = r;
        let (core, bit) = (x / 24, 1u32 << (x % 24));
        let g = v.rec[x].group as i32;
        let (wl, wr) = if held { (0, 0) } else { (volume_reg(l, g, &v.group_vol) as u32, volume_reg(r, g, &v.group_vol) as u32) };
        v.stream_write(Reg::VolL, x, wl);
        v.stream_write(Reg::VolR, x, wr);
        v.vmix_l[core] |= bit;
        let m = v.vmix_l[core];
        v.stream_write(Reg::VmixL, core, m);
        v.vmix_r[core] |= bit;
        let m = v.vmix_r[core];
        v.stream_write(Reg::VmixR, core, m);
        v.stream_write(Reg::Pitch, x, if held { 0 } else { s.pitch as u32 });
        let base = spu_base.wrapping_add((s.chan[ch] as u32).wrapping_mul(half));
        if let Some((id, d)) = &s.bufs[ch] {
            v.attach_stream(x, *id, d.clone());
        }
        v.stream_write(Reg::Ssa, x, base.wrapping_add(s.ring[ch] as u32).wrapping_add(0x30));
        v.stream_write(Reg::Adsr1, x, ADSR1);
        v.stream_write(Reg::Adsr2, x, ADSR2);
        if s.flags & 2 != 0 {
            v.reverb[core] |= bit;
        } else {
            v.reverb[core] &= !bit;
        }
        v.pending_kon[core] |= bit;
    }
}

#[cfg(test)]
#[path = "streams_tests.rs"]
mod tests;

// ---------------------------------------------------------------- savestate (oracle snapfile section)
use super::persist::{In, Out};

impl VagInfo {
    pub fn save(&self, o: &mut Out) {
        o.u32(self.rate);
        o.u32(self.data_bytes);
    }
    pub fn load(i: &mut In) -> VagInfo {
        VagInfo { rate: i.u32(), data_bytes: i.u32() }
    }
}

impl StreamEv {
    pub fn save(&self, o: &mut Out) {
        match *self {
            StreamEv::Start { tick, voice, extra, lba, held } => {
                o.u8(0);
                o.u32(tick);
                o.len(voice);
                o.opt_usize(extra);
                o.u32(lba);
                o.bool(held);
            }
            StreamEv::Next { tick, voice, lba } => {
                o.u8(1);
                o.u32(tick);
                o.len(voice);
                o.u32(lba);
            }
            StreamEv::End { tick, voice, natural } => {
                o.u8(2);
                o.u32(tick);
                o.len(voice);
                o.bool(natural);
            }
        }
    }
    pub fn load(i: &mut In) -> StreamEv {
        match i.u8() {
            0 => StreamEv::Start { tick: i.u32(), voice: i.len(), extra: i.opt_usize(), lba: i.u32(), held: i.bool() },
            1 => StreamEv::Next { tick: i.u32(), voice: i.len(), lba: i.u32() },
            _ => StreamEv::End { tick: i.u32(), voice: i.len(), natural: i.bool() },
        }
    }
}

impl Stream {
    pub fn save(&self, o: &mut Out) {
        o.u32(self.handle);
        o.i8(self.group);
        o.u32(self.flags);
        o.i16(self.vol);
        o.i16(self.pan);
        o.bool(self.held);
        o.bool(self.buffered);
        for v in self.voices {
            o.opt_usize(v);
        }
        o.opt_usize(self.extra);
        for v in self.lba {
            o.u32(v);
        }
        for v in self.ring {
            o.u16(v);
        }
        for v in self.chan {
            o.len(v);
        }
        for v in self.pan_off {
            o.i32(v);
        }
        o.u32(self.created);
        o.opt_u32(self.key_at);
        o.bool(self.keyed);
        o.bool(self.stopping);
        o.bool(self.first_pass);
        o.u32(self.elapsed);
        o.bool(self.info.is_some());
        if let Some(x) = &self.info {
            x.save(o);
        }
        o.u16(self.pitch);
        o.u16(self.pitch_live);
        o.u64(self.pos);
        o.len(self.queue.len());
        for s in &self.queue {
            o.u32(s.lba);
            o.u32(s.lba_r);
            o.u32(s.flags);
        }
    }
    pub fn load(i: &mut In) -> Stream {
        let handle = i.u32();
        let group = i.i8();
        let flags = i.u32();
        let (vol, pan, held, buffered) = (i.i16(), i.i16(), i.bool(), i.bool());
        let voices = [i.opt_usize(), i.opt_usize()];
        let extra = i.opt_usize();
        let lba = [i.u32(), i.u32()];
        let ring = [i.u16(), i.u16()];
        let chan = [i.len(), i.len()];
        let pan_off = [i.i32(), i.i32()];
        let created = i.u32();
        let key_at = i.opt_u32();
        let (keyed, stopping, first_pass, elapsed) = (i.bool(), i.bool(), i.bool(), i.u32());
        let info = if i.bool() { Some(VagInfo::load(i)) } else { None };
        let (pitch, pitch_live, pos) = (i.u16(), i.u16(), i.u64());
        let n = i.len();
        let queue = (0..n).map(|_| Seg { lba: i.u32(), lba_r: i.u32(), flags: i.u32() }).collect();
        Stream { handle, group, flags, vol, pan, held, buffered, voices, extra, lba, ring, chan, bufs: [None, None], pan_off, created, key_at, keyed, stopping, first_pass, elapsed, info, pitch, pitch_live, pos, queue }
    }
}

impl Streams {
    pub fn save(&self, o: &mut Out) {
        o.bool(self.inited);
        o.len(self.slots.len());
        for s in &self.slots {
            o.bool(s.is_some());
            if let Some(s) = s {
                s.save(o);
            }
        }
        o.len(self.serial.len());
        for s in &self.serial {
            o.u16(*s);
        }
        o.len(self.chan_used.len());
        for s in &self.chan_used {
            o.bool(*s);
        }
        o.u32(self.half);
        o.u32(self.disc.cd_read_ticks);
        o.u32(self.disc.spu_dma_ticks);
        o.u32(self.spu_base);
        o.len(self.events.len());
        for e in &self.events {
            e.save(o);
        }
        o.len(self.errors.len());
        for e in &self.errors {
            o.u32(*e);
        }
    }
    pub fn load(&mut self, i: &mut In) {
        self.inited = i.bool();
        let n = i.len();
        self.slots = (0..n).map(|_| if i.bool() { Some(Stream::load(i)) } else { None }).collect();
        let n = i.len();
        self.serial = (0..n).map(|_| i.u16()).collect();
        let n = i.len();
        self.chan_used = (0..n).map(|_| i.bool()).collect();
        self.half = i.u32();
        self.disc = DiscTiming { cd_read_ticks: i.u32(), spu_dma_ticks: i.u32() };
        self.spu_base = i.u32();
        let n = i.len();
        self.events = (0..n).map(|_| StreamEv::load(i)).collect();
        let n = i.len();
        self.errors = (0..n).map(|_| i.u32()).collect();
    }

    /// Stage 11 savestate adoption of a stream whose file is known (the music channel: track -> level header lba): a fresh
    /// mono stream under the EE's handle, keyed at the next tick from position 0 (the IOP's play position is not in the
    /// savestate [I]); `flags` as cid 44 (0x20 two voices, 4 loop), `held` = paused. False when the handle does not fit.
    pub fn adopt_playing(&mut self, now: u32, v: &mut SpuVoices, handle: u32, lba: u32, flags: u32, volume: u32, held: bool) -> bool {
        if !self.inited || (handle >> 24) & 0x1f != 4 || lba == 0 {
            return false;
        }
        let idx = ((handle >> 16) & 0xff) as usize;
        if idx >= self.slots.len() || self.slots[idx].is_some() {
            return false;
        }
        if self.chan_used.iter().all(|u| *u) {
            return false;
        }
        let Some(vl) = v.alloc_stream(1) else { return false };
        let extra = if flags & 0x20 != 0 { v.alloc_stream(1) } else { None };
        let c0 = self.alloc_chan().unwrap();
        self.serial[idx] = handle as u16;
        self.slots[idx] = Some(Stream {
            handle,
            group: 1,
            flags,
            vol: ((volume.wrapping_mul(127) as i32) >> 10) as i16,
            pan: 0,
            held,
            buffered: false,
            voices: [Some(vl), None],
            extra,
            lba: [lba, 0],
            ring: [0, 0],
            chan: [c0, c0],
            bufs: [None, None],
            pan_off: [0, 0],
            created: now,
            key_at: None,
            keyed: false,
            stopping: false,
            first_pass: false,
            elapsed: 0,
            info: None,
            pitch: 0,
            pitch_live: 0,
            pos: 0,
            queue: VecDeque::new(),
        });
        true
    }

    /// Savestate adoption (stage 7d): a stream the EE channel struct holds a handle for, rebuilt at position 0 with that
    /// handle (slot = bits 16..23, serial = low 16 bits). The voice comes from `alloc_stream`, `keyed` and `buffered` are
    /// set, no register is written (the host SPU2 already plays). False when the handle does not fit the pool.
    pub fn adopt(&mut self, now: u32, v: &mut SpuVoices, handle: u32, lba: u32, info: Option<VagInfo>) -> bool {
        if !self.inited || (handle >> 24) & 0x1f != 4 {
            return false;
        }
        let idx = ((handle >> 16) & 0xff) as usize;
        if idx >= self.slots.len() || self.slots[idx].is_some() {
            return false;
        }
        let Some(c) = self.chan_used.iter().position(|u| !*u) else { return false };
        let Some(vl) = v.alloc_stream(1) else { return false };
        self.chan_used[c] = true;
        self.serial[idx] = handle as u16;
        let s = Stream {
            handle,
            group: 1,
            flags: 0,
            vol: CHAN_VOL as i16,
            pan: 0,
            held: false,
            buffered: true,
            voices: [Some(vl), None],
            extra: None,
            lba: [lba, 0],
            ring: [0, 0],
            chan: [c, c],
            bufs: [None, None],
            pan_off: [0, 0],
            created: now,
            key_at: Some(now),
            keyed: true,
            stopping: false,
            first_pass: false,
            elapsed: 0,
            info,
            pitch: info.map_or(0, |x| pitch_of_rate(x.rate)),
            pitch_live: info.map_or(0, |x| pitch_of_rate(x.rate)),
            pos: 0,
            queue: VecDeque::new(),
        };
        self.slots[idx] = Some(s);
        true
    }
}
