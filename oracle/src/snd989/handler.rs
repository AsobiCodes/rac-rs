//! 989snd grain interpreter and handler lifecycle (stage 4 of the exact port; note: decomp/notes/sound_grains.md
//! "Handler struct", "Timing", "Grain type table", "Interpreter", "Stop path"; every function here cites its IOP module
//! address). A *handler* is one playing sound: 64 slots at module 0x1a2d0 (stride 0x114), a singly linked active list
//! (head 0x19750, tail 0x19754, link at +0x20), handle word `0x85000000 | slot << 16 | serial`.
//!
//! Layering: this file owns the handlers, the RNG draws and the tick order. Everything below it is a trait so the later
//! stages plug in without touching the interpreter: [`Voices`] (stage 6: the 48-voice pool, key-on math, SPU2 register
//! image; key-on and key-off leave the handler as calls = events) and [`Lfo`] (stage 5: the 4 LFO slots per handler).
//! [`RecVoices`] / [`NullLfo`] are the recording stand-ins used by the tests and by the stage 7 plumbing tests.
//!
//! Not modelled (no bank uses them, kept as errors/no-ops with a comment at the place): child handlers (grains 5-7,
//! +0x24 parent, +0x28 child), the fade list at +0x08 (0x2960 / 0xd358, cid 33 flag 4), 0xc5e0, streams (handle type 4).

use super::bank::{Bank, Grain, Tone};
use super::rng::Rng250;
use std::collections::HashMap;
use std::sync::Arc;

pub const SLOTS: usize = 64;
pub const VOICES: usize = 48;
/// Handle word of a linked handler: bit 31 = linked (0xc8a4), bits 24-28 = type 5, bits 16-23 = slot, low 16 = serial.
pub const HANDLE_LINKED: u32 = 0x8000_0000;
pub const HANDLE_TYPE: u32 = 0x0500_0000;
/// Guard against an endless zero-delay loop inside one tick (retail would hang the IOP thread).
pub const MAX_GRAINS_PER_PASS: u32 = 200_000;

/// Error codes passed to FUN_00001e64 by the grain handlers.
pub mod err {
    pub const LOOP_END_WITHOUT_START: u32 = 8; // grain 22, 0x4b84
    pub const BREAK_WITHOUT_LOOP_END: u32 = 9; // grain 23, 0x4c1c
    pub const BLOCK_NESTED: u32 = 10; // grain 25, 0x4d0c
    pub const GOTO_MISSING: u32 = 11; // grain 36, 0x5224
    pub const RANDOM_GOTO_MISSING: u32 = 12; // grain 37, 0x52cc
    pub const SEQ_BLOCK_NESTED: u32 = 13; // grain 39, 0x5420
    pub const BAD_GRAIN: u32 = 0x5f; // types 8-19, 5-7 (children, not modelled), >= 44 (null handler)
    pub const DIVIDE_TRAP: u32 = 0x1c00; // `trap(0x1c00)` in a % by zero
    pub const RUNAWAY: u32 = 0xffff_0001; // MAX_GRAINS_PER_PASS exceeded (model guard, not a retail code)
    pub const BAD_INDEX: u32 = 0xffff_0002; // idx outside the grain table (model guard)
}

pub type VoiceMask = [u32; 2];

pub fn mask_has(m: &VoiceMask, v: usize) -> bool {
    v < VOICES && m[v / 24] & (1 << (v % 24)) != 0
}
pub fn mask_set(m: &mut VoiceMask, v: usize) {
    m[v / 24] |= 1 << (v % 24);
}
pub fn mask_clear(m: &mut VoiceMask, v: usize) {
    m[v / 24] &= !(1 << (v % 24));
}
pub fn mask_empty(m: &VoiceMask) -> bool {
    m[0] == 0 && m[1] == 0
}
/// Voice indices in a mask, ascending (0xd1f0 scans from 0 up).
pub fn mask_voices(m: &VoiceMask) -> impl Iterator<Item = usize> + '_ {
    (0..VOICES).filter(move |&v| mask_has(m, v))
}

// ---------------------------------------------------------------- traits

/// A voice granted by the pool (0x14d60). `stolen` = a lower-priority voice that was taken over: its owner's callback
/// (0x61b4, reason 1) must run before the new voice is attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Granted {
    pub voice: usize,
    pub stolen: Option<VoiceEnd>,
}

/// A voice-callback notification: `voice` ended (or was stolen) and its owner handler is `slot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceEnd {
    pub voice: usize,
    pub slot: usize,
}

/// Everything the tone grain 0x4644 hands to key-on (0x1642c L/R math + 0x14fc0 register image).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyOn {
    pub tick: u32,
    pub slot: usize,
    pub handle: u32,
    pub bank_handle: u32,
    /// The bank itself: its sample data is what the SPU2 voice plays.
    pub bank: Arc<Bank>,
    pub sound: usize,
    pub grain: usize,
    pub tone: Tone,
    pub group: i8,
    pub priority: i8,
    /// The voice's tone volume byte after the register / random operand rules (s8).
    pub tone_volume: i8,
    /// The tone pan argument after the register / random rules (degrees, s16).
    pub tone_pan: i16,
    /// Handler volume h+0x10 and pan h+0x12 (0..0x7f, 0..359).
    pub volume: i16,
    pub pan: i16,
    /// h+0x32 volume scale (0x400 = 1.0): the pan scaling 0x635c (sound flag bit 2) and the priority use it.
    pub scale: i16,
    pub centre: i8,
    pub centre_fine: i8,
    /// h+0x2e bend sum and h+0x14 pitch mod.
    pub bend: i16,
    pub pitch_mod: i16,
    pub sound_flags: u16,
}

/// What the running update 0x5aac needs per voice (the voice record holds the tone and the previous L/R).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolCtx {
    pub volume: i16,
    pub pan: i16,
    pub scale: i16,
    pub group: i8,
    pub sound_flags: u16,
    /// Handler paused (h+0x16 bit 1): the voice record changes but VOLL/VOLR are not written.
    pub paused: bool,
}

/// What 0x5ffc needs per voice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PitchCtx {
    pub bend: i16,
    pub pitch_mod: i16,
    pub centre: i8,
    pub centre_fine: i8,
    pub paused: bool,
}

/// The voice layer (stage 6). Calls are the events the SPU2 sees; a real implementation also owns the pending
/// key-on/key-off masks (0x198d8.., applied by `apply_key_offs` then `apply_key_ons` at the end of every tick).
pub trait Voices {
    /// Called at the start of every 240 Hz iteration with the new tick counter [0x19770].
    fn tick_begin(&mut self, _tick: u32) {}
    /// 0x14d60: take a voice for (group, priority). None = the tone is dropped silently.
    fn alloc(&mut self, tick: u32, group: i8, priority: i8) -> Option<Granted>;
    /// 0x1642c + 0x14fc0: set up and queue the key-on of an allocated voice (owner = `req.slot`).
    fn key_on(&mut self, voice: usize, req: &KeyOn);
    /// 0x157f8: queue KOFF for the voices; they detach from their handler (callback := 0). With `notify` the callback
    /// runs once (reason 3): the returned list is applied to the handlers by the caller.
    fn key_off(&mut self, mask: VoiceMask, notify: bool) -> Vec<VoiceEnd>;
    /// 0x15968: zero the volume and free the voices at once.
    fn kill(&mut self, mask: VoiceMask, notify: bool) -> Vec<VoiceEnd>;
    /// 0x15bb8(handler): pause = VOLL = VOLR = PITCH = 0 for every non-free voice owned by `slot` (envelope and sample keep running).
    fn pause(&mut self, slot: usize, mask: VoiceMask);
    /// 0x15e40: continue = rewrite VOLL/VOLR (stored L/R through the group master) and PITCH.
    fn resume(&mut self, mask: VoiceMask);
    /// 0x5aac per voice: recompute L/R (0x1642c, out[] NOT zeroed), write VOLL/VOLR unless paused, priority := 0x640c.
    fn revolume(&mut self, voice: usize, ctx: &VolCtx);
    /// 0x5ffc per voice: store bend/mod, recompute and write PITCH unless paused.
    fn repitch(&mut self, voice: usize, ctx: &PitchCtx);
    /// 0x1547c: write the pending KOFF bits.
    fn apply_key_offs(&mut self, tick: u32);
    /// 0x15388: write VMIXEL/VMIXER and the pending KON bits.
    fn apply_key_ons(&mut self, tick: u32);
    /// 0xdec8, once per timer wake before the tick iterations: end-of-voice polling; the callbacks it fires.
    fn poll_end(&mut self, tick: u32) -> Vec<VoiceEnd>;
}

/// One LFO slot of a handler (h+0x54 + 0x30*k). Offsets are the retail ones; grain 4 (0x4a5c) fills the fields below,
/// the running state (+0x1c.. +0x2c) is stage 5's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LfoSlot {
    /// +0x00 wave (0 none, 1 sine, 2 square, 3 triangle, 4 saw, 5 random).
    pub wave: u16,
    /// +0x02 target (0 = off).
    pub target: u8,
    /// +0x03 p2: the other LFO slot for targets 5 / 6.
    pub link: u8,
    /// +0x04 flags (bit 0 invert, bit 1 random start phase).
    pub flags: u8,
    /// +0x05 bit 0: in the running LFO list (set by 0x20bc, cleared by 0x21a4).
    pub in_list: bool,
    /// +0x06 depth and its zero-extended copy at +0x08.
    pub depth: u16,
    pub depth_wide: u32,
    /// +0x0c phase = start << 16.
    pub phase: u32,
    /// +0x10 / +0x14 rate (u32 p12) and its base copy.
    pub rate: u32,
    pub rate_base: u32,
    /// +0x18 raw 32 bits: square width (u16 p4, wave 2), the held random value (s32, wave 5), else 0.
    pub aux: u32,
    /// +0x1c random wave: 0 = the first-half value is due, 1 = held (a grain writes 0).
    pub state_1c: u32,
    /// +0x20 amplitude A (0x27e8), s32.
    pub amp: i32,
    pub state_24: u32,
}

/// The LFO layer (stage 5).
pub trait Lfo {
    /// 0x20bc: register a freshly written slot (`core.slot(handler).lfo[index]`) in the running list; the real one also
    /// draws the random wave's first value, computes the amplitude and runs the first step (it needs the R250 stream and
    /// the handler, hence `core`).
    fn start(&mut self, core: &mut Core, handler: usize, index: usize);
    /// 0x21a4: remove one slot from the running list.
    fn stop(&mut self, handler: usize, index: usize, slot: &mut LfoSlot);
    /// 0x2154: remove all 4 slots of a handler (stop path, handler free).
    fn release(&mut self, handler: usize, slots: &mut [LfoSlot; 4]) {
        for (i, s) in slots.iter_mut().enumerate() {
            self.stop(handler, i, s);
        }
    }
    /// 0x2748 -> 0x22ec, 120 Hz: called from the tick only when the NEW tick counter is even.
    fn update(&mut self, _core: &mut Core, _voices: &mut dyn Voices) {}
}

// ---------------------------------------------------------------- handler

/// One handler slot (0x114 bytes in the IOP). Field comments give the retail offsets.
#[derive(Clone, Debug, Default)]
pub struct Handler {
    /// +0x00 handle word.
    pub word: u32,
    /// +0x04 sound block (None = slot free) and the sound's place in its bank (+0x50 bank).
    pub bank: Option<Arc<Bank>>,
    pub bank_handle: u32,
    pub sound: usize,
    /// +0x20 list link.
    pub next: Option<usize>,
    pub base_volume: i16,    // +0x0c
    pub pan_arg: u16,        // +0x0e
    pub volume: i16,         // +0x10
    pub pan: i16,            // +0x12
    pub pitch_mod: i16,      // +0x14
    pub flags: u8,           // +0x16: bit 1 paused, bit 2 stopping
    pub group: i8,           // +0x17
    pub masks: VoiceMask,    // +0x18 / +0x1c
    pub centre: i8,          // +0x2c (60)
    pub centre_fine: i8,     // +0x2d (0)
    pub bend: i16,           // +0x2e sum
    pub vol_off: i16,        // +0x30 (LFO target 1)
    pub scale: i16,          // +0x32
    pub pan_off: i16,        // +0x34
    pub pan_base: i16,       // +0x36
    pub bend_off: i16,       // +0x38
    pub bend_base: i16,      // +0x3a
    pub pmod_off: i16,       // +0x3c
    pub pmod_base: i16,      // +0x3e
    pub wait: i16,           // +0x40
    pub idx: i16,            // +0x42, -1 = program finished
    pub regs: [i8; 4],       // +0x44
    pub block_active: bool,  // +0x48
    pub block_count: u16,    // +0x4a
    pub block_skip: i16,     // +0x4c
    pub lfo: [LfoSlot; 4],   // +0x54
}

impl Handler {
    pub fn in_use(&self) -> bool {
        self.bank.is_some()
    }
    pub fn paused(&self) -> bool {
        self.flags & 2 != 0
    }
    pub fn sound_ref(&self) -> Option<&super::bank::Sound> {
        self.bank.as_ref().and_then(|b| b.sounds.get(self.sound))
    }
}

/// Arguments of a cid 17 play call (41a0 -> 3df0; the base volume and the sound-default pan are fixed by 0x3db0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayArgs {
    /// d[2] volume scale (0x400 = 1.0, 0x7fffffff = 0x400).
    pub scale: i32,
    /// d[3] pan degrees (-1 / -2 = the sound's own pan).
    pub pan: i32,
    /// d[4] pitch mod (1/128 semitone) and d[5] bend.
    pub pitch_mod: i16,
    pub bend: i16,
    /// Registers r0..r3 when the options carry them (flag 0x40; 41a0 never sets it).
    pub regs: Option<[i8; 4]>,
}

impl PlayArgs {
    pub fn cid17(d2: i32, d3: i32, d4: i32, d5: i32) -> PlayArgs {
        PlayArgs { scale: d2, pan: d3, pitch_mod: d4 as i16, bend: d5 as i16, regs: None }
    }
}

impl Default for PlayArgs {
    fn default() -> Self {
        PlayArgs { scale: 0x400, pan: -1, pitch_mod: 0, bend: 0, regs: None }
    }
}

/// Error log entry (FUN_00001e64 calls).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SndError {
    pub tick: u32,
    pub code: u32,
    pub slot: usize,
    pub idx: i16,
}

/// 0x640c: tone priority, faded by the handler's volume scale (flags bits 1-2 = mode; no bank uses a mode).
pub fn tone_priority(scale: i16, tone: &Tone) -> i8 {
    let p0 = tone.priority as i32;
    if tone.flags & 6 == 0 {
        return tone.priority;
    }
    let d = match (tone.flags >> 1) & 3 {
        1 => p0 / 5,
        2 => p0 / 2,
        _ => p0,
    };
    if d == 0 || scale as i32 > 0x3b6 {
        return tone.priority;
    }
    (p0 - (d - d * scale as i32 / 0x3b6)) as i8
}

/// `r % n` as the IOP computes it (signed `%`; n == 0 is a break exception, handled by the caller).
fn c_mod(r: i32, n: i32) -> Option<i32> {
    if n == 0 {
        None
    } else if n == -1 {
        Some(0)
    } else {
        Some(r % n)
    }
}

/// The handler table plus everything the interpreter shares: tick counter, R250 stream, persistent grain state.
pub struct Core {
    slots: Vec<Handler>,
    head: Option<usize>,
    tail: Option<usize>,
    /// [0x19770].
    pub tick: u32,
    /// The one shared random stream (0xad14).
    pub rng: Rng250,
    pub errors: Vec<SndError>,
    /// Grains 25 / 39 write their state back into the grain (p4): the table is shared by all handlers of a bank, so it
    /// is kept here, keyed (bank handle, sound, grain). Missing = the value in the bank (0 in every retail bank).
    picks: HashMap<(u32, usize, usize), i16>,
}

impl Core {
    pub fn new(rng: Rng250) -> Core {
        Core { slots: vec![Handler::default(); SLOTS], head: None, tail: None, tick: 0, rng, errors: Vec::new(), picks: HashMap::new() }
    }

    /// Savestate adoption (stage 8): slot `i` as the live driver holds it.
    pub fn adopt_slot(&mut self, i: usize, h: Handler) {
        self.slots[i] = h;
    }

    /// Savestate adoption: the active list in order (links, head, tail).
    pub fn adopt_list(&mut self, order: &[usize]) {
        for s in &mut self.slots {
            s.next = None;
        }
        for w in order.windows(2) {
            self.slots[w[0]].next = Some(w[1]);
        }
        self.head = order.first().copied();
        self.tail = order.last().copied();
    }

    pub fn slot(&self, i: usize) -> &Handler {
        &self.slots[i]
    }
    pub fn slot_mut(&mut self, i: usize) -> &mut Handler {
        &mut self.slots[i]
    }
    /// One handler and the shared R250 stream at once (the LFO layer draws random values while it edits the slots).
    pub fn lfo_access(&mut self, i: usize) -> (&mut Handler, &mut Rng250) {
        (&mut self.slots[i], &mut self.rng)
    }
    /// Active list in run order (head to tail).
    pub fn list(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = self.head;
        while let Some(s) = cur {
            out.push(s);
            cur = self.slots[s].next;
            if out.len() > SLOTS {
                break;
            }
        }
        out
    }
    pub fn active_count(&self) -> usize {
        self.list().len()
    }

    /// Forget the persistent grain state of a bank (cid 6 unload).
    pub fn forget_bank(&mut self, bank_handle: u32) {
        self.picks.retain(|k, _| k.0 != bank_handle);
    }

    fn error(&mut self, slot: usize, code: u32) {
        let idx = self.slots[slot].idx;
        self.errors.push(SndError { tick: self.tick, code, slot, idx });
    }

    fn pick(&self, s: usize, gi: usize, g: &Grain) -> i16 {
        let h = &self.slots[s];
        self.picks.get(&(h.bank_handle, h.sound, gi)).copied().unwrap_or_else(|| g.s16_at(4))
    }
    fn set_pick(&mut self, s: usize, gi: usize, v: i16) {
        let h = &self.slots[s];
        self.picks.insert((h.bank_handle, h.sound, gi), v);
    }

    // ------------------------------------------------ handle table (0xc794, 0xc8a4, 0xc924, 0xcadc)

    /// 0xcadc: the handler a handle word names (slot word must equal the handle, so a freed or reused slot misses).
    pub fn find(&self, handle: u32) -> Option<usize> {
        if (handle >> 24) & 0x1f != 5 {
            return None;
        }
        let slot = ((handle >> 16) & 0xff) as usize;
        if slot >= SLOTS {
            return None;
        }
        (self.slots[slot].word == handle).then_some(slot)
    }

    /// cid 25 (0xb278) reply: 0 for 0, -1 for -1, the handle while it is alive, else 0 (ENDED).
    pub fn playing(&self, handle: u32) -> u32 {
        if handle == 0 {
            0
        } else if handle == 0xffff_ffff {
            0xffff_ffff
        } else if self.find(handle).is_some() {
            handle
        } else {
            0
        }
    }

    /// 0xc794: the lowest free slot gets word = ((old low 16 bits + 1) & 0xffff) | slot << 16 | 5 << 24 and is cleared.
    fn alloc_slot(&mut self) -> Option<usize> {
        let s = (0..SLOTS).find(|&i| !self.slots[i].in_use())?;
        let h = &mut self.slots[s];
        h.word = ((h.word & 0xffff).wrapping_add(1) & 0xffff) | ((s as u32) << 16) | HANDLE_TYPE;
        h.flags = 0;
        h.next = None;
        Some(s)
    }

    /// 0xc8a4: set bit 31 and append to the active list.
    fn link(&mut self, s: usize) {
        self.slots[s].word |= HANDLE_LINKED;
        self.slots[s].next = None;
        match self.tail {
            None => {
                self.head = Some(s);
                self.tail = Some(s);
            }
            Some(t) if t != s => {
                self.slots[t].next = Some(s);
                self.tail = Some(s);
            }
            Some(_) => {}
        }
    }

    /// 0xc924: sb := 0, bit 31 cleared, unlinked (the slot's own `next` is left alone: a running tick pass still follows it).
    fn free(&mut self, s: usize) {
        self.slots[s].bank = None;
        self.slots[s].word &= !HANDLE_LINKED;
        let (mut prev, mut cur) = (None, self.head);
        while let Some(c) = cur {
            if c == s {
                break;
            }
            prev = cur;
            cur = self.slots[c].next;
        }
        if cur.is_some() {
            let after = self.slots[s].next;
            match prev {
                None => self.head = after,
                Some(p) => self.slots[p].next = after,
            }
            if after.is_none() {
                self.tail = prev;
            }
        }
    }

    // ------------------------------------------------ voice callback and ends

    /// 0x61b4 (voice callback, any reason): clear the voice's bit; with both masks empty and the program finished
    /// (idx == -1) the LFOs are released and the handler freed. (Parent/child links are not modelled.)
    pub fn voice_end(&mut self, end: VoiceEnd, lfo: &mut dyn Lfo) {
        let s = end.slot;
        if s >= SLOTS {
            return;
        }
        mask_clear(&mut self.slots[s].masks, end.voice);
        let h = &self.slots[s];
        if mask_empty(&h.masks) && h.idx == -1 {
            lfo.release(s, &mut self.slots[s].lfo);
            self.free(s);
        }
    }

    pub fn voice_ends(&mut self, ends: Vec<VoiceEnd>, lfo: &mut dyn Lfo) {
        for e in ends {
            self.voice_end(e, lfo);
        }
    }

    // ------------------------------------------------ the interpreter (0x595c)

    /// One grain. true = the handler is finished (0x595c returned 1: STOP, error, or program end with no voice).
    fn run_grain(&mut self, s: usize, v: &mut dyn Voices, lfo: &mut dyn Lfo) -> bool {
        let Some(bank) = self.slots[s].bank.clone() else { return true };
        let sound = self.slots[s].sound;
        let grains = &bank.sounds[sound].grains;
        let count = grains.len() as u8 as i8 as i32; // sb+4 is read with `lb`
        let idx = self.slots[s].idx;
        if idx < 0 || idx as usize >= grains.len() {
            self.error(s, err::BAD_INDEX);
            return true;
        }
        let ret = self.exec(s, &bank, idx as usize, v, lfo);
        if ret < 0 {
            return true;
        }
        let h = &mut self.slots[s];
        if h.block_active {
            h.block_count = h.block_count.wrapping_sub(1);
            if h.block_count == 0 {
                h.block_active = false;
                h.idx = h.idx.wrapping_add(h.block_skip);
            }
        }
        h.idx = h.idx.wrapping_add(1);
        if count - 1 < h.idx as i32 {
            if mask_empty(&h.masks) {
                return true;
            }
            h.idx = -1;
        } else {
            let delay = usize::try_from(h.idx).ok().and_then(|i| grains.get(i)).map_or(0, |g| g.delay);
            h.wait = (delay as i32 + ret) as i16;
        }
        false
    }

    fn reg(&self, s: usize, i: i16) -> i8 {
        self.slots[s].regs.get(i as usize).copied().unwrap_or(0)
    }
    fn set_reg(&mut self, s: usize, i: i16, val: i8) {
        if let Some(r) = self.slots[s].regs.get_mut(i as usize) {
            *r = val;
        }
    }

    /// Search `grains` for the first type-35 marker with id `id`.
    fn find_marker(grains: &[Grain], id: i32) -> Option<usize> {
        grains.iter().position(|g| g.kind == 35 && g.s16_at(0) as i32 == id)
    }

    /// The grain handler table 0x18d80. Returns the extra wait for the next grain (>= 0) or -1 = stop.
    fn exec(&mut self, s: usize, bank: &Arc<Bank>, gi: usize, v: &mut dyn Voices, lfo: &mut dyn Lfo) -> i32 {
        let sound = self.slots[s].sound;
        let grains = &bank.sounds[sound].grains;
        let g = &grains[gi];
        let count = grains.len() as u8 as i8 as i32;
        match g.kind {
            0 | 2 | 3 | 20 | 21 | 35 => 0,
            1 => self.grain_tone(s, bank, gi, v, lfo),
            4 => self.grain_lfo(s, g, lfo),
            22 => {
                // 0x4b84: idx := (nearest type-21 before this grain) - 1
                let mut j = self.slots[s].idx as i32 - 1;
                while j >= 0 {
                    if grains[j as usize].kind == 21 {
                        self.slots[s].idx = (j - 1) as i16;
                        return 0;
                    }
                    j -= 1;
                }
                self.error(s, err::LOOP_END_WITHOUT_START);
                -1
            }
            23 => {
                // 0x4c1c: idx := next type-22 after this grain (the grain after it runs next)
                let mut j = self.slots[s].idx as i32 + 1;
                while j < count {
                    if grains[j as usize].kind == 22 {
                        self.slots[s].idx = j as i16;
                        return 0;
                    }
                    j += 1;
                }
                self.error(s, err::BREAK_WITHOUT_LOOP_END);
                -1
            }
            24 => -1,
            25 => {
                // 0x4d0c random block select: N blocks (p0), L grains each (p2), last pick (p4, written back)
                if self.slots[s].block_active {
                    self.error(s, err::BLOCK_NESTED);
                    return -1;
                }
                let r = self.rng.next() as i32;
                let (n, l) = (g.s16_at(0) as i32, g.s16_at(2));
                let Some(mut a) = c_mod(r, n) else {
                    self.error(s, err::DIVIDE_TRAP);
                    return -1;
                };
                if a == self.pick(s, gi, g) as i32 {
                    a += 1;
                    if a == n {
                        a = 0;
                    }
                }
                self.block_start(s, gi, a as i16, n as i16, l);
                0
            }
            26 => {
                // 0x4e28 random wait: ret = rand % (p0 + 1)
                let r = self.rng.next() as i32;
                match c_mod(r, g.s16_at(0) as i32 + 1) {
                    Some(x) => x,
                    None => {
                        self.error(s, err::DIVIDE_TRAP);
                        -1
                    }
                }
            }
            27 => {
                // 0x4e80 random bend
                let r = self.rng.next() as u32 % 0x7fff;
                let y = (r.wrapping_mul(0xffff) as i32) / 0x7fff - 0x8000;
                let b = (y.wrapping_mul(g.s16_at(0) as i32) / 100) as i16;
                self.set_bend(s, b, v);
                0
            }
            28 => {
                // 0x4f50 set bend
                let x = g.s16_at(0) as i32;
                let b = if x < 0 { ((x.wrapping_mul(-0x8000) / 0x7f) as i16).wrapping_neg() } else { (x.wrapping_mul(0x7fff) / 0x7f) as i16 };
                self.set_bend(s, b, v);
                0
            }
            29 => {
                // 0x4fd4 add to bend (h+0x2e is the current sum)
                let x = self.slots[s].bend as i32 + (g.s16_at(0) as i32).wrapping_mul(0x7fff) / 0x7f;
                self.set_bend(s, x.clamp(-0x8000, 0x7fff) as i16, v);
                0
            }
            30 => {
                self.set_reg(s, g.s16_at(0), g.u8_at(2) as i8);
                0
            }
            31 => {
                // 0x5070: r[p0] = (char)(rand % (p4 - p2 + 1)) + (char)p2
                let n = g.s16_at(4) as i32 - g.s16_at(2) as i32 + 1;
                let r = self.rng.next() as i32;
                let Some(m) = c_mod(r, n) else {
                    self.error(s, err::DIVIDE_TRAP);
                    return -1;
                };
                self.set_reg(s, g.s16_at(0), (m as i8).wrapping_add(g.s16_at(2) as i8));
                0
            }
            32 => {
                let r = (self.reg(s, g.s16_at(0)) as i32 + 1).min(0x7f);
                self.set_reg(s, g.s16_at(0), r as i8);
                0
            }
            33 => {
                let r = (self.reg(s, g.s16_at(0)) as i32 - 1).max(-0x80);
                self.set_reg(s, g.s16_at(0), r as i8);
                0
            }
            34 => {
                // 0x5164 if register: op p2 (0 <, 1 ==, 2 >) against p4; false = the NEXT grain is skipped (its delay lost)
                let (r, val) = (self.reg(s, g.s16_at(0)) as i32, g.s16_at(4) as i32);
                let ok = match g.s16_at(2) {
                    0 => r < val,
                    1 => r == val,
                    2 => r > val,
                    _ => true,
                };
                if !ok {
                    self.slots[s].idx = self.slots[s].idx.wrapping_add(1);
                }
                0
            }
            36 => match Self::find_marker(grains, g.s16_at(0) as i32) {
                Some(i) => {
                    self.slots[s].idx = (i as i32 - 1) as i16;
                    0
                }
                None => {
                    self.error(s, err::GOTO_MISSING);
                    -1
                }
            },
            37 => {
                // 0x52cc random goto: id = rand % (p2 - p0 + 1) + p0
                let n = g.s16_at(2) as i32 - g.s16_at(0) as i32 + 1;
                let r = self.rng.next() as i32;
                let Some(m) = c_mod(r, n) else {
                    self.error(s, err::DIVIDE_TRAP);
                    return -1;
                };
                match Self::find_marker(grains, m + g.s16_at(0) as i32) {
                    Some(i) => {
                        self.slots[s].idx = (i as i32 - 1) as i16;
                        0
                    }
                    None => {
                        self.error(s, err::RANDOM_GOTO_MISSING);
                        -1
                    }
                }
            }
            38 => {
                // 0x53e4 wait for voices: this grain runs again after its delay + 1
                if !mask_empty(&self.slots[s].masks) {
                    self.slots[s].idx = self.slots[s].idx.wrapping_sub(1);
                    1
                } else {
                    0
                }
            }
            39 => {
                // 0x5420 sequential block select: state p4 counts 0..N-1 (written back)
                if self.slots[s].block_active {
                    self.error(s, err::SEQ_BLOCK_NESTED);
                    return -1;
                }
                let n = g.s16_at(0);
                let a = self.pick(s, gi, g);
                let mut next = a.wrapping_add(1);
                if next == n {
                    next = 0;
                }
                self.set_pick(s, gi, next);
                let l = g.s16_at(2);
                let h = &mut self.slots[s];
                h.idx = h.idx.wrapping_add((a as i32 * l as i32) as i16);
                h.block_count = l.wrapping_add(1) as u16;
                h.block_active = true;
                h.block_skip = (((n as i32 - 1) - a as i32) * l as i32) as i16;
                0
            }
            40 => {
                let r = (self.reg(s, g.s16_at(2)) as i32 + g.s16_at(0) as i32).clamp(-0x80, 0x7f);
                self.set_reg(s, g.s16_at(2), r as i8);
                0
            }
            41 => {
                // 0x5528 key off every voice of the handler, detach them
                let m = self.slots[s].masks;
                let ends = v.key_off(m, false);
                self.slots[s].masks = [0, 0];
                self.voice_ends(ends, lfo);
                0
            }
            42 => {
                let m = self.slots[s].masks;
                let ends = v.kill(m, false);
                self.slots[s].masks = [0, 0];
                self.voice_ends(ends, lfo);
                0
            }
            43 => {
                // 0x5598: jump to the end of the program (the label of the stop section)
                self.slots[s].idx = (count - 1) as i16;
                0
            }
            _ => {
                // 5-7 (child sounds, not in any bank), 8-19, >= 44 (null table entry)
                self.error(s, err::BAD_GRAIN);
                -1
            }
        }
    }

    /// Shared tail of grain 25: chosen block `a` of `n`, `l` grains each; the pick is written back into the grain.
    fn block_start(&mut self, s: usize, gi: usize, a: i16, n: i16, l: i16) {
        self.set_pick(s, gi, a);
        let h = &mut self.slots[s];
        h.idx = h.idx.wrapping_add((a as i32 * l as i32) as i16);
        h.block_count = l.wrapping_add(1) as u16;
        h.block_active = true;
        h.block_skip = (((n as i32 - 1) - a as i32) * l as i32) as i16;
    }

    /// 0x4a5c: LFO set. Fills the slot; target 0 stops it (0x21a4) and zeroes the wave.
    fn grain_lfo(&mut self, s: usize, g: &Grain, lfo: &mut dyn Lfo) -> i32 {
        let k = (g.u8_at(0) as usize).min(3);
        let target = g.u8_at(1);
        if g.u8_at(0) > 3 {
            // retail writes outside the 4 slots; no bank does. Model guard: ignore.
            self.error(s, err::BAD_INDEX);
            return 0;
        }
        self.slots[s].lfo[k].target = target;
        if target == 0 {
            lfo.stop(s, k, &mut self.slots[s].lfo[k]);
            self.slots[s].lfo[k].wave = 0;
        } else {
            let phase = if g.u8_at(8) & 2 == 0 { g.u16_at(0x12) as u32 } else { (self.rng.next() & 0x7ff) as u32 };
            let sl = &mut self.slots[s].lfo[k];
            sl.wave = g.u8_at(3) as u16;
            sl.link = g.u8_at(2);
            sl.flags = g.u8_at(8);
            sl.depth = g.u16_at(6);
            sl.depth_wide = g.u16_at(6) as u32;
            sl.rate = g.u32_at(0xc);
            sl.rate_base = g.u32_at(0xc);
            sl.state_1c = 0;
            sl.state_24 = 0;
            sl.aux = if sl.wave == 2 { g.u16_at(4) as u32 } else { 0 };
            sl.phase = phase << 16;
            lfo.start(self, s, k);
        }
        0
    }

    /// 0x4644: tone grain = allocate a voice and queue its key-on. Returns 0 even when the tone is dropped.
    fn grain_tone(&mut self, s: usize, bank: &Arc<Bank>, gi: usize, v: &mut dyn Voices, lfo: &mut dyn Lfo) -> i32 {
        let sb = &bank.sounds[self.slots[s].sound];
        let tone = sb.grains[gi].tone().expect("grain type 1");
        let h = &mut self.slots[s];
        // h+0x10 = clamp(s16(u16(h+0x30) + (scale * base >> 10)), 0, 0x7f)
        let sum = h.vol_off as u16 as i32 + ((h.scale as i32 * h.base_volume as i32) >> 10);
        h.volume = (sum as i16).clamp(0, 0x7f);
        // h+0x12 = (pan base + pan offset) folded into 0..359
        let mut p = h.pan_base.wrapping_add(h.pan_off);
        while p > 0x167 {
            p = p.wrapping_sub(0x168);
        }
        while p < 0 {
            p = p.wrapping_add(0x168);
        }
        h.pan = p;
        let priority = tone_priority(h.scale, &tone);
        let Some(granted) = v.alloc(self.tick, sb.group as i8, priority) else { return 0 };
        if let Some(st) = granted.stolen {
            self.voice_end(st, lfo);
        }
        // voice bit, then the random / register operands (volume first, then pan)
        let tv: i8 = if tone.volume < 0 {
            if tone.volume < -4 {
                (self.rng.next() & 0x7f) as u8 as i8
            } else {
                self.reg(s, !(tone.volume as i16) )
            }
        } else {
            tone.volume
        };
        let tp: i16 = if tone.pan < 0 {
            if tone.pan < -4 {
                (self.rng.next() as i32 % 0x168) as i16
            } else {
                ((self.reg(s, !tone.pan) as i32 * 0x168) / 0x7f) as i16
            }
        } else {
            tone.pan
        };
        let h = &mut self.slots[s];
        mask_set(&mut h.masks, granted.voice);
        let req = KeyOn {
            tick: self.tick,
            slot: s,
            handle: h.word,
            bank_handle: h.bank_handle,
            bank: bank.clone(),
            sound: h.sound,
            grain: gi,
            tone,
            group: sb.group as i8,
            priority,
            tone_volume: tv,
            tone_pan: tp,
            volume: h.volume,
            pan: h.pan,
            scale: h.scale,
            centre: h.centre,
            centre_fine: h.centre_fine,
            bend: h.bend,
            pitch_mod: h.pitch_mod,
            sound_flags: sb.flags,
        };
        v.key_on(granted.voice, &req);
        0
    }

    // ------------------------------------------------ bend / pitch / volume updates (0x5ecc, 0x5f64, 0x5ffc, 0x5aac)

    /// 0x5ecc: h+0x3a := value, then re-apply the pitch.
    fn set_bend(&mut self, s: usize, value: i16, v: &mut dyn Voices) {
        self.slots[s].bend_base = value;
        self.reapply_pitch(s, v);
    }

    /// 0x5f64: h+0x3e := value, then re-apply the pitch.
    fn set_pmod(&mut self, s: usize, value: i16, v: &mut dyn Voices) {
        self.slots[s].pmod_base = value;
        self.reapply_pitch(s, v);
    }

    /// 0x5ffc: h+0x2e = clamp16(h+0x3a + h+0x38), h+0x14 = h+0x3e + h+0x3c, then every voice of the handler. Also the
    /// entry the LFO stage uses after it changed an offset (targets 3 and 4).
    pub fn reapply_pitch(&mut self, s: usize, v: &mut dyn Voices) {
        let h = &mut self.slots[s];
        h.bend = (h.bend_base as i32 + h.bend_off as i32).clamp(-0x8000, 0x7fff) as i16;
        h.pitch_mod = h.pmod_base.wrapping_add(h.pmod_off);
        let ctx = PitchCtx { bend: h.bend, pitch_mod: h.pitch_mod, centre: h.centre, centre_fine: h.centre_fine, paused: h.paused() };
        for voice in mask_voices(&h.masks) {
            v.repitch(voice, &ctx);
        }
    }

    /// 0x5aac: running volume/pan update. `scale` 0x7fffffff keeps, negative = |x| * 0x400 / 0x7f; `pan` -2 keeps, -1 =
    /// the sound's own pan. Voices are touched only when the handler's volume or pan actually changed. Also the entry
    /// the LFO stage uses after it changed an offset (targets 1 and 2): `update_volume(s, 0x7fffffff, -2, v)`.
    pub fn update_volume(&mut self, s: usize, scale: i32, pan: i32, v: &mut dyn Voices) {
        let sound_pan = self.slots[s].sound_ref().map_or(0, |x| x.pan as i16);
        let h = &mut self.slots[s];
        if scale < 0 {
            h.scale = (scale.wrapping_mul(-0x400) / 0x7f) as i16;
        } else if scale != 0x7fff_ffff {
            h.scale = scale as i16;
        }
        if pan == -1 {
            h.pan_base = sound_pan;
        } else if pan != -2 {
            h.pan_base = pan as i16;
        }
        let vol = (((h.scale as i32 * h.base_volume as i32) >> 10) + h.vol_off as i32).clamp(0, 0x7f);
        let mut p = h.pan_base as i32 + h.pan_off as i32;
        while p > 0x167 {
            p -= 0x168;
        }
        while p < 0 {
            p += 0x168;
        }
        if vol != h.volume as i32 || p != h.pan as i32 {
            h.volume = vol as i16;
            h.pan = p as i16;
            let ctx = VolCtx {
                volume: h.volume,
                pan: h.pan,
                scale: h.scale,
                group: h.group,
                sound_flags: h.sound_ref_flags(),
                paused: h.paused(),
            };
            for voice in mask_voices(&h.masks) {
                v.revolume(voice, &ctx);
            }
        }
    }

    // ------------------------------------------------ stop path (0x451c, 0xcfe8)

    /// 0x45c8 + 0x451c: key off every voice, run the grains after the first type-43 grain at once (delays ignored).
    /// true = the handler must stay (voices left in the masks).
    fn stop_section(&mut self, s: usize, v: &mut dyn Voices, lfo: &mut dyn Lfo) -> bool {
        let bank = self.slots[s].bank.clone().expect("stop_section on a free slot");
        let grains = &bank.sounds[self.slots[s].sound].grains;
        let n = grains.len();
        let label = grains.iter().position(|g| g.kind == 43).filter(|&k| k as i32 != (n as u8 as i8 as i32) - 1);
        if let Some(k) = label {
            let m = self.slots[s].masks;
            let ends = v.key_off(m, false);
            self.slots[s].masks = [0, 0];
            self.voice_ends(ends, lfo);
            self.slots[s].idx = (k + 1) as i16;
            let mut guard = 0;
            loop {
                if self.slots[s].idx == -1 {
                    break;
                }
                guard += 1;
                if guard > MAX_GRAINS_PER_PASS {
                    self.error(s, err::RUNAWAY);
                    break;
                }
                if self.run_grain(s, v, lfo) {
                    break;
                }
            }
            if !mask_empty(&self.slots[s].masks) {
                return true;
            }
        }
        lfo.release(s, &mut self.slots[s].lfo);
        false
    }

    /// 0xcfe8(h, recurse, kill) for a bank handler: mark stopping, run the stop section, then (unless voices keep it
    /// alive) key off every voice still attached (kill when `kill` or paused) and free the handler.
    pub fn stop_path(&mut self, s: usize, kill: bool, v: &mut dyn Voices, lfo: &mut dyn Lfo) {
        if !self.slots[s].in_use() {
            return;
        }
        self.slots[s].flags |= 4;
        if self.stop_section(s, v, lfo) {
            return;
        }
        let m = self.slots[s].masks;
        let ends = if !kill && !self.slots[s].paused() { v.key_off(m, true) } else { v.kill(m, true) };
        // the callbacks name this very handler; it is freed right below, so only other owners could matter (none)
        let others: Vec<VoiceEnd> = ends.into_iter().filter(|e| e.slot != s).collect();
        self.free(s);
        self.voice_ends(others, lfo);
    }

    // ------------------------------------------------ public commands

    /// cid 17 -> 0x3df0: start a sound. Returns the handle, or 0 (no such sound, no free slot, or the program ended
    /// at once with no voice). Zero-delay grains run inside this call.
    pub fn play(&mut self, v: &mut dyn Voices, lfo: &mut dyn Lfo, bank_handle: u32, bank: &Arc<Bank>, sound: i32, a: &PlayArgs) -> u32 {
        let Some(sb) = bank.playable(sound) else { return 0 };
        let sound = sound as usize;
        let base = sb.volume as i16;
        let scale = if a.scale == 0x7fff_ffff { 0x400 } else { a.scale };
        let mut pan = a.pan;
        if (pan.wrapping_add(2) as u32) < 2 {
            pan = sb.pan as i16 as i32;
        }
        let vol0 = (scale.wrapping_mul(base as i32) >> 10).min(0x7f);
        if sb.flags & 2 != 0 {
            // 0xd184: single instance, stop the other copies first (recurse = 1, kill = 0)
            let mut cur = self.head;
            while let Some(c) = cur {
                if self.slots[c].bank_handle == bank_handle && self.slots[c].sound == sound && self.slots[c].in_use() {
                    self.stop_path(c, false, v, lfo);
                }
                cur = self.slots[c].next;
            }
        }
        let Some(s) = self.alloc_slot() else { return 0 };
        let h = &mut self.slots[s];
        let word = h.word;
        *h = Handler {
            word,
            bank: Some(bank.clone()),
            bank_handle,
            sound,
            next: None,
            base_volume: base,
            pan_arg: sb.pan,
            volume: vol0 as i16,
            pan: pan as i16,
            pitch_mod: a.pitch_mod,
            flags: 0,
            group: sb.group as i8,
            masks: [0, 0],
            centre: 0x3c,
            centre_fine: 0,
            bend: a.bend,
            vol_off: 0,
            scale: scale as i16,
            pan_off: 0,
            pan_base: pan as i16,
            bend_off: 0,
            bend_base: a.bend,
            pmod_off: 0,
            pmod_base: a.pitch_mod,
            wait: sb.grains[0].delay as i16,
            idx: 0,
            regs: a.regs.unwrap_or([0; 4]),
            block_active: false,
            block_count: 0,
            block_skip: 0,
            lfo: Default::default(),
        };
        let mut finished = false;
        if self.slots[s].idx != -1 {
            let mut guard = 0;
            while self.slots[s].wait == 0 {
                guard += 1;
                if guard > MAX_GRAINS_PER_PASS {
                    self.error(s, err::RUNAWAY);
                    finished = true;
                    break;
                }
                if self.run_grain(s, v, lfo) {
                    finished = true;
                    break;
                }
                if self.slots[s].idx == -1 {
                    break;
                }
            }
        }
        if finished || (self.slots[s].wait == 0 && mask_empty(&self.slots[s].masks)) {
            self.stop_path(s, false, v, lfo);
            self.slots[s].bank = None;
            return 0;
        }
        self.link(s);
        self.slots[s].word
    }

    /// cid 21 -> 0xb230: stop a handler by handle (stop path, recurse 1, kill 0). false = no such handler.
    pub fn stop(&mut self, v: &mut dyn Voices, lfo: &mut dyn Lfo, handle: u32) -> bool {
        match self.find(handle) {
            Some(s) => {
                self.stop_path(s, false, v, lfo);
                true
            }
            None => false,
        }
    }

    /// cid 24 -> 0xcf3c: stop every parentless handler with kill, then kill all voices.
    pub fn stop_all(&mut self, v: &mut dyn Voices, lfo: &mut dyn Lfo) {
        let mut cur = self.head;
        while let Some(c) = cur {
            let next = self.slots[c].next;
            if self.slots[c].in_use() {
                self.stop_path(c, true, v, lfo);
            }
            cur = next;
        }
        let ends = v.kill([0x00ff_ffff, 0x00ff_ffff], true);
        self.voice_ends(ends, lfo);
    }

    /// cid 33 -> 0xb428: running update. Returns the reply (the handle, or 0 when it is gone).
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, v: &mut dyn Voices, handle: u32, flags: u32, scale: i32, pan: i32, pitch_mod: i16, bend: i16) -> u32 {
        let Some(s) = self.find(handle) else { return 0 };
        let mut flags = flags;
        if flags & 6 == 6 {
            flags &= !2;
        }
        let scale = if flags & 1 == 0 { 0x7fff_ffff } else { scale };
        let pan = if flags & 2 == 0 { -2 } else { pan };
        if flags & 3 != 0 {
            self.update_volume(s, scale, pan, v);
        }
        // flag 4 = 0x2960 fade: not modelled (no bank / game path measured)
        if flags & 8 != 0 {
            self.set_pmod(s, pitch_mod, v);
        }
        if flags & 0x10 != 0 {
            self.set_bend(s, bend, v);
        }
        handle
    }

    /// cid 32 -> 0xb79c.
    pub fn set_pitch_mod(&mut self, v: &mut dyn Voices, handle: u32, value: i16) {
        if let Some(s) = self.find(handle) {
            self.set_pmod(s, value, v);
        }
    }

    /// 0xb73c.
    pub fn set_bend_by_handle(&mut self, v: &mut dyn Voices, handle: u32, value: i16) {
        if let Some(s) = self.find(handle) {
            self.set_bend(s, value, v);
        }
    }

    /// cid 22 -> 0xcd2c: pause every running handler whose volume group bit is set in `mask`.
    pub fn pause_groups(&mut self, v: &mut dyn Voices, mask: u32) {
        let mut cur = self.head;
        while let Some(c) = cur {
            let h = &mut self.slots[c];
            if mask & (1u32 << ((h.group as i32) & 0x1f)) != 0 && !h.paused() {
                h.flags |= 2;
                v.pause(c, h.masks);
            }
            cur = self.slots[c].next;
        }
    }

    /// cid 23 -> 0xce74: continue every paused handler of the groups in `mask`.
    pub fn resume_groups(&mut self, v: &mut dyn Voices, mask: u32) {
        let mut cur = self.head;
        while let Some(c) = cur {
            let h = &mut self.slots[c];
            if mask & (1u32 << ((h.group as i32) & 0x1f)) != 0 && h.paused() {
                v.resume(h.masks);
                h.flags &= !2;
            }
            cur = self.slots[c].next;
        }
    }

    // ------------------------------------------------ the 240 Hz tick

    /// 0x4494: one handler for one tick. true = finished (run the stop path).
    fn run_handler(&mut self, s: usize, v: &mut dyn Voices, lfo: &mut dyn Lfo) -> bool {
        if self.slots[s].idx == -1 {
            return false;
        }
        self.slots[s].wait = self.slots[s].wait.wrapping_sub(1);
        let mut guard = 0;
        while self.slots[s].wait < 1 {
            guard += 1;
            if guard > MAX_GRAINS_PER_PASS {
                self.error(s, err::RUNAWAY);
                return true;
            }
            if self.run_grain(s, v, lfo) {
                return true;
            }
            if self.slots[s].idx == -1 {
                return false;
            }
        }
        false
    }

    /// One iteration of the timer loop at 0xe0d8: tick counter++, every handler in list order (paused ones skipped),
    /// pending key-offs, pending key-ons, [0xc5e0 not modelled], LFO update on even tick counters. The end-of-voice
    /// poll (0xdec8) runs once per timer wake before the iteration(s): see [`Driver::tick`].
    pub fn iteration(&mut self, v: &mut dyn Voices, lfo: &mut dyn Lfo) {
        self.tick = self.tick.wrapping_add(1);
        v.tick_begin(self.tick);
        let mut cur = self.head;
        while let Some(s) = cur {
            if self.slots[s].in_use() && !self.slots[s].paused() && self.run_handler(s, v, lfo) {
                self.stop_path(s, false, v, lfo);
            }
            cur = self.slots[s].next;
        }
        v.apply_key_offs(self.tick);
        v.apply_key_ons(self.tick);
        if self.tick % 2 == 0 {
            lfo.update(self, v);
        }
    }

    /// [`Core::iteration`] with a hook between the handler loop and the key-off flush: retail runs the stream pass 0xfe74
    /// from the handler loop (0xd2d4), so a stream's key-on lands in the same iteration's KON flush. `mid(voices, tick)`.
    pub fn iteration_with<V: Voices>(&mut self, v: &mut V, lfo: &mut dyn Lfo, mid: impl FnOnce(&mut V, u32)) {
        self.tick = self.tick.wrapping_add(1);
        v.tick_begin(self.tick);
        let mut cur = self.head;
        while let Some(s) = cur {
            if self.slots[s].in_use() && !self.slots[s].paused() && self.run_handler(s, v, lfo) {
                self.stop_path(s, false, v, lfo);
            }
            cur = self.slots[s].next;
        }
        mid(v, self.tick);
        v.apply_key_offs(self.tick);
        v.apply_key_ons(self.tick);
        if self.tick % 2 == 0 {
            lfo.update(self, v);
        }
    }
}

impl Handler {
    fn sound_ref_flags(&self) -> u16 {
        self.sound_ref().map_or(0, |s| s.flags)
    }
}

// ---------------------------------------------------------------- driver wrapper

/// Handlers + the two layers under them. `tick()` is one timer wake; the cid commands are thin wrappers.
pub struct Driver<V: Voices, L: Lfo> {
    pub core: Core,
    pub voices: V,
    pub lfo: L,
}

impl<V: Voices, L: Lfo> Driver<V, L> {
    pub fn new(rng: Rng250, voices: V, lfo: L) -> Self {
        Driver { core: Core::new(rng), voices, lfo }
    }

    /// One timer wake with one loop iteration: end-of-voice poll (callbacks first), then the iteration.
    pub fn tick(&mut self) {
        let ends = self.voices.poll_end(self.core.tick);
        self.core.voice_ends(ends, &mut self.lfo);
        self.core.iteration(&mut self.voices, &mut self.lfo);
    }

    pub fn ticks(&mut self, n: u32) {
        for _ in 0..n {
            self.tick();
        }
    }

    pub fn play(&mut self, bank_handle: u32, bank: &Arc<Bank>, sound: i32, a: &PlayArgs) -> u32 {
        self.core.play(&mut self.voices, &mut self.lfo, bank_handle, bank, sound, a)
    }
    pub fn stop(&mut self, handle: u32) -> bool {
        self.core.stop(&mut self.voices, &mut self.lfo, handle)
    }
    pub fn stop_all(&mut self) {
        self.core.stop_all(&mut self.voices, &mut self.lfo)
    }
    pub fn playing(&self, handle: u32) -> u32 {
        self.core.playing(handle)
    }
    pub fn update(&mut self, handle: u32, flags: u32, scale: i32, pan: i32, pitch_mod: i16, bend: i16) -> u32 {
        self.core.update(&mut self.voices, handle, flags, scale, pan, pitch_mod, bend)
    }
    pub fn pause_groups(&mut self, mask: u32) {
        self.core.pause_groups(&mut self.voices, mask)
    }
    pub fn resume_groups(&mut self, mask: u32) {
        self.core.resume_groups(&mut self.voices, mask)
    }
}

// ---------------------------------------------------------------- recording stand-ins

/// What [`RecVoices`] logs: the SPU2-facing events of the voice layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceEvent {
    /// Handler-time key-on request (the SPU2 sees it at `KeyOnsApplied`).
    KeyOn { voice: usize, slot: usize, sound: usize, grain: usize, tone_volume: i8, tone_pan: i16, volume: i16, pan: i16, priority: i8 },
    KeyOff { mask: VoiceMask, notify: bool },
    Kill { mask: VoiceMask, notify: bool },
    Pause { mask: VoiceMask },
    Resume { mask: VoiceMask },
    Volume { voice: usize, volume: i16, pan: i16, paused: bool },
    Pitch { voice: usize, bend: i16, pitch_mod: i16, paused: bool },
    KeyOffsApplied,
    KeyOnsApplied,
    Ended { voice: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Vs {
    Free,
    Keyed(u32),
    Releasing(u32),
}

/// Minimal voice pool for tests: first free voice, no steal, optional one-shot length and release time.
pub struct RecVoices {
    pub log: Vec<(u32, VoiceEvent)>,
    pub now: u32,
    /// Ticks until a keyed voice ends by itself (a one-shot sample); None = loops forever.
    pub natural_end: Option<u32>,
    /// Ticks from the applied KOFF to the voice being free again.
    pub release_ticks: u32,
    owner: [Option<usize>; VOICES],
    state: [Vs; VOICES],
    pending_off: Vec<usize>,
    pub keyons: u32,
}

impl RecVoices {
    pub fn new() -> RecVoices {
        RecVoices { log: Vec::new(), now: 0, natural_end: None, release_ticks: 8, owner: [None; VOICES], state: [Vs::Free; VOICES], pending_off: Vec::new(), keyons: 0 }
    }
    pub fn busy(&self) -> usize {
        self.state.iter().filter(|s| **s != Vs::Free).count()
    }
    pub fn events(&self) -> impl Iterator<Item = &(u32, VoiceEvent)> {
        self.log.iter()
    }
    fn push(&mut self, e: VoiceEvent) {
        self.log.push((self.now, e));
    }
    fn detach(&mut self, mask: VoiceMask, notify: bool, kill: bool) -> Vec<VoiceEnd> {
        let mut ends = Vec::new();
        for v in mask_voices(&mask) {
            if let Vs::Keyed(_) = self.state[v] {
                if let Some(slot) = self.owner[v].take() {
                    if notify {
                        ends.push(VoiceEnd { voice: v, slot });
                    }
                }
                if kill {
                    self.state[v] = Vs::Free;
                } else {
                    self.pending_off.push(v);
                }
            }
        }
        ends
    }
}

impl Default for RecVoices {
    fn default() -> Self {
        RecVoices::new()
    }
}

impl Voices for RecVoices {
    fn tick_begin(&mut self, tick: u32) {
        self.now = tick;
    }
    fn alloc(&mut self, tick: u32, _group: i8, _priority: i8) -> Option<Granted> {
        self.now = self.now.max(tick);
        let voice = (0..VOICES).find(|&i| self.state[i] == Vs::Free)?;
        self.state[voice] = Vs::Keyed(self.now);
        Some(Granted { voice, stolen: None })
    }
    fn key_on(&mut self, voice: usize, req: &KeyOn) {
        self.owner[voice] = Some(req.slot);
        self.keyons += 1;
        self.push(VoiceEvent::KeyOn {
            voice,
            slot: req.slot,
            sound: req.sound,
            grain: req.grain,
            tone_volume: req.tone_volume,
            tone_pan: req.tone_pan,
            volume: req.volume,
            pan: req.pan,
            priority: req.priority,
        });
    }
    fn key_off(&mut self, mask: VoiceMask, notify: bool) -> Vec<VoiceEnd> {
        self.push(VoiceEvent::KeyOff { mask, notify });
        self.detach(mask, notify, false)
    }
    fn kill(&mut self, mask: VoiceMask, notify: bool) -> Vec<VoiceEnd> {
        self.push(VoiceEvent::Kill { mask, notify });
        self.detach(mask, notify, true)
    }
    fn pause(&mut self, _slot: usize, mask: VoiceMask) {
        self.push(VoiceEvent::Pause { mask });
    }
    fn resume(&mut self, mask: VoiceMask) {
        self.push(VoiceEvent::Resume { mask });
    }
    fn revolume(&mut self, voice: usize, ctx: &VolCtx) {
        self.push(VoiceEvent::Volume { voice, volume: ctx.volume, pan: ctx.pan, paused: ctx.paused });
    }
    fn repitch(&mut self, voice: usize, ctx: &PitchCtx) {
        self.push(VoiceEvent::Pitch { voice, bend: ctx.bend, pitch_mod: ctx.pitch_mod, paused: ctx.paused });
    }
    fn apply_key_offs(&mut self, tick: u32) {
        if !self.pending_off.is_empty() {
            for v in std::mem::take(&mut self.pending_off) {
                if let Vs::Keyed(_) = self.state[v] {
                    self.state[v] = Vs::Releasing(tick + self.release_ticks);
                }
            }
            self.push(VoiceEvent::KeyOffsApplied);
        }
    }
    fn apply_key_ons(&mut self, _tick: u32) {
        if self.keyons > 0 {
            self.keyons = 0;
            self.push(VoiceEvent::KeyOnsApplied);
        }
    }
    fn poll_end(&mut self, tick: u32) -> Vec<VoiceEnd> {
        let mut ends = Vec::new();
        for v in 0..VOICES {
            match self.state[v] {
                Vs::Keyed(since) => {
                    if let Some(n) = self.natural_end {
                        if tick.saturating_sub(since) >= n {
                            self.state[v] = Vs::Free;
                            if let Some(slot) = self.owner[v].take() {
                                ends.push(VoiceEnd { voice: v, slot });
                            }
                            self.log.push((tick, VoiceEvent::Ended { voice: v }));
                        }
                    }
                }
                Vs::Releasing(until) => {
                    if tick >= until {
                        self.state[v] = Vs::Free;
                        self.log.push((tick, VoiceEvent::Ended { voice: v }));
                    }
                }
                Vs::Free => {}
            }
        }
        ends
    }
}

/// LFO stand-in: tracks the running list only, no update.
#[derive(Default)]
pub struct NullLfo {
    pub started: Vec<(usize, usize)>,
    pub stopped: Vec<(usize, usize)>,
    pub updates: u32,
}

impl Lfo for NullLfo {
    fn start(&mut self, core: &mut Core, handler: usize, index: usize) {
        core.slot_mut(handler).lfo[index].in_list = true;
        self.started.push((handler, index));
    }
    fn stop(&mut self, handler: usize, index: usize, slot: &mut LfoSlot) {
        if slot.in_list {
            self.stopped.push((handler, index));
        }
        slot.in_list = false;
    }
    fn update(&mut self, _core: &mut Core, _voices: &mut dyn Voices) {
        self.updates += 1;
    }
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;

// ---------------------------------------------------------------- savestate (oracle snapfile section)
use super::bank::Banks;
use super::persist::{In, Out};

impl LfoSlot {
    pub fn save(&self, o: &mut Out) {
        o.u16(self.wave);
        o.u8(self.target);
        o.u8(self.link);
        o.u8(self.flags);
        o.bool(self.in_list);
        o.u16(self.depth);
        o.u32(self.depth_wide);
        o.u32(self.phase);
        o.u32(self.rate);
        o.u32(self.rate_base);
        o.u32(self.aux);
        o.u32(self.state_1c);
        o.i32(self.amp);
        o.u32(self.state_24);
    }
    pub fn load(i: &mut In) -> LfoSlot {
        LfoSlot {
            wave: i.u16(),
            target: i.u8(),
            link: i.u8(),
            flags: i.u8(),
            in_list: i.bool(),
            depth: i.u16(),
            depth_wide: i.u32(),
            phase: i.u32(),
            rate: i.u32(),
            rate_base: i.u32(),
            aux: i.u32(),
            state_1c: i.u32(),
            amp: i.i32(),
            state_24: i.u32(),
        }
    }
}

impl Handler {
    pub fn save(&self, o: &mut Out) {
        o.u32(self.word);
        o.bool(self.bank.is_some());
        o.u32(self.bank_handle);
        o.len(self.sound);
        o.opt_usize(self.next);
        o.i16(self.base_volume);
        o.u16(self.pan_arg);
        o.i16(self.volume);
        o.i16(self.pan);
        o.i16(self.pitch_mod);
        o.u8(self.flags);
        o.i8(self.group);
        o.mask(&self.masks);
        o.i8(self.centre);
        o.i8(self.centre_fine);
        for v in [self.bend, self.vol_off, self.scale, self.pan_off, self.pan_base, self.bend_off, self.bend_base, self.pmod_off, self.pmod_base, self.wait, self.idx] {
            o.i16(v);
        }
        for r in self.regs {
            o.i8(r);
        }
        o.bool(self.block_active);
        o.u16(self.block_count);
        o.i16(self.block_skip);
        for l in &self.lfo {
            l.save(o);
        }
    }
    pub fn load(i: &mut In, banks: &Banks) -> Handler {
        let word = i.u32();
        let has_bank = i.bool();
        let bank_handle = i.u32();
        let bank = if has_bank { Some(banks.get(bank_handle).unwrap_or_else(|| panic!("savestate: handler bank {bank_handle:#x} not loaded")).clone()) } else { None };
        let sound = i.len();
        let next = i.opt_usize();
        let (base_volume, pan_arg, volume, pan, pitch_mod) = (i.i16(), i.u16(), i.i16(), i.i16(), i.i16());
        let (flags, group, masks, centre, centre_fine) = (i.u8(), i.i8(), i.mask(), i.i8(), i.i8());
        let mut w = [0i16; 11];
        for x in &mut w {
            *x = i.i16();
        }
        let regs = [i.i8(), i.i8(), i.i8(), i.i8()];
        let (block_active, block_count, block_skip) = (i.bool(), i.u16(), i.i16());
        let lfo = std::array::from_fn(|_| LfoSlot::load(i));
        Handler {
            word,
            bank,
            bank_handle,
            sound,
            next,
            base_volume,
            pan_arg,
            volume,
            pan,
            pitch_mod,
            flags,
            group,
            masks,
            centre,
            centre_fine,
            bend: w[0],
            vol_off: w[1],
            scale: w[2],
            pan_off: w[3],
            pan_base: w[4],
            bend_off: w[5],
            bend_base: w[6],
            pmod_off: w[7],
            pmod_base: w[8],
            wait: w[9],
            idx: w[10],
            regs,
            block_active,
            block_count,
            block_skip,
            lfo,
        }
    }
}

impl Core {
    pub fn save(&self, o: &mut Out) {
        o.len(self.slots.len());
        for h in &self.slots {
            h.save(o);
        }
        o.opt_usize(self.head);
        o.opt_usize(self.tail);
        o.u32(self.tick);
        let (t, ix) = self.rng.state();
        for v in t {
            o.u16(*v);
        }
        o.len(ix);
        o.len(self.errors.len());
        for e in &self.errors {
            o.u32(e.tick);
            o.u32(e.code);
            o.len(e.slot);
            o.i16(e.idx);
        }
        let mut p: Vec<_> = self.picks.iter().collect();
        p.sort();
        o.len(p.len());
        for ((bh, snd, g), v) in p {
            o.u32(*bh);
            o.len(*snd);
            o.len(*g);
            o.i16(*v);
        }
    }
    pub fn load(&mut self, i: &mut In, banks: &Banks) {
        let n = i.len();
        self.slots = (0..n).map(|_| Handler::load(i, banks)).collect();
        self.head = i.opt_usize();
        self.tail = i.opt_usize();
        self.tick = i.u32();
        let mut t = [0u16; super::rng::LEN];
        for v in &mut t {
            *v = i.u16();
        }
        let ix = i.len();
        self.rng = Rng250::from_state(t, ix).expect("savestate: R250 index");
        let n = i.len();
        self.errors = (0..n).map(|_| SndError { tick: i.u32(), code: i.u32(), slot: i.len(), idx: i.i16() }).collect();
        let n = i.len();
        self.picks = (0..n).map(|_| ((i.u32(), i.len(), i.len()), i.i16())).collect();
    }
}
