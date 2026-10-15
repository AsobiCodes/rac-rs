//! SPU2 voice model: per-sample ADSR envelope, sample cursor (block flags, loop/stop), ENVX.
//! Rules: decomp/notes/sound_grains.md "SPU2 ADSR" (psx-spx register fields; PCSX2 V_ADSR::Calculate / UpdateCache for
//! the exact integer form). Reference model and numbers: scratch_grain/adsr_sim.py, adsr_golden.py.
//! Pure Rust, no oracle types: moves unchanged to oracle/src/spu2.rs. All rules run once per 48 kHz output sample.

/// Envelope phase, in the order the hardware walks them (`Stopped` = envelope 0, voice silent).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    Attack = 0,
    Decay = 1,
    Sustain = 2,
    Release = 3,
    Stopped = 4,
}

impl Phase {
    fn from_index(i: u8) -> Phase {
        match i {
            0 => Phase::Attack,
            1 => Phase::Decay,
            2 => Phase::Sustain,
            3 => Phase::Release,
            _ => Phase::Stopped,
        }
    }
}

/// One phase as PCSX2 `UpdateCache` builds it from ADSR1/ADSR2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhaseCfg {
    pub decr: bool,
    pub exp: bool,
    pub shift: u32,
    pub step: i32,
    pub target: i32,
}

/// ADSR1/ADSR2 (raw register values, written by the driver from tone p10 / p12) -> [attack, decay, sustain, release].
pub fn decode(adsr1: u16, adsr2: u16) -> [PhaseCfg; 4] {
    let (a1, a2) = (adsr1 as i32, adsr2 as i32);
    let sl = a1 & 15;
    let attack = PhaseCfg { decr: false, exp: (a1 >> 15) & 1 != 0, shift: ((a1 >> 10) & 31) as u32, step: 7 - ((a1 >> 8) & 3), target: 0x7fff };
    let decay = PhaseCfg { decr: true, exp: true, shift: ((a1 >> 4) & 15) as u32, step: -8, target: (sl + 1) << 11 };
    let sdecr = (a2 >> 14) & 1 != 0;
    let mut sstep = 7 - ((a2 >> 6) & 3);
    if sdecr {
        sstep = !sstep; // -8..-5
    }
    let sustain = PhaseCfg { decr: sdecr, exp: (a2 >> 15) & 1 != 0, shift: ((a2 >> 8) & 31) as u32, step: sstep, target: 0 };
    let release = PhaseCfg { decr: true, exp: (a2 >> 5) & 1 != 0, shift: (a2 & 31) as u32, step: -8, target: 0 };
    [attack, decay, sustain, release]
}

/// One voice's envelope. `value` is ENVX (0..=0x7fff); the driver only tests it for 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Env {
    cfg: [PhaseCfg; 4],
    phase: Phase,
    counter: u32,
    value: i32,
}

impl Env {
    /// A stopped envelope with the given ADSR registers.
    pub fn new(adsr1: u16, adsr2: u16) -> Env {
        Env { cfg: decode(adsr1, adsr2), phase: Phase::Stopped, counter: 0, value: 0 }
    }

    /// Adopt a running envelope (savestate: PCSX2 phase / counter / value).
    pub fn from_state(adsr1: u16, adsr2: u16, phase: Phase, counter: u32, value: i32) -> Env {
        Env { cfg: decode(adsr1, adsr2), phase, counter, value }
    }

    /// Register write while the voice runs: the phase parameters change, phase / counter / level are kept.
    pub fn set_adsr(&mut self, adsr1: u16, adsr2: u16) {
        self.cfg = decode(adsr1, adsr2);
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn counter(&self) -> u32 {
        self.counter
    }

    /// ENVX.
    pub fn envx(&self) -> u16 {
        self.value as u16
    }

    pub fn cfg(&self) -> &[PhaseCfg; 4] {
        &self.cfg
    }

    /// KON: attack from level 0 (also when the voice was playing; the stored ENVX is dropped).
    pub fn key_on(&mut self) {
        self.phase = Phase::Attack;
        self.counter = 0;
        self.value = 0;
    }

    /// KOFF: release from the current level. KOFF is applied BEFORE KON within one sample.
    pub fn key_off(&mut self) {
        if self.phase != Phase::Stopped {
            self.phase = Phase::Release;
            self.counter = 0;
        }
    }

    /// V_Voice::Stop: sample loop-end without repeat, or the envelope ran out.
    pub fn stop(&mut self) {
        self.value = 0;
        self.phase = Phase::Stopped;
    }

    /// One 48 kHz sample (PCSX2 V_ADSR::Calculate). Returns false when the voice has stopped.
    pub fn tick(&mut self) -> bool {
        if self.phase == Phase::Stopped {
            self.value = 0;
            return false;
        }
        let p = self.cfg[self.phase as usize];
        let mut counter_inc: u32 = 0x8000u32 >> p.shift.saturating_sub(11);
        let mut level_inc: i32 = p.step << 11u32.saturating_sub(p.shift);
        if p.exp {
            if !p.decr && self.value > 0x6000 {
                counter_inc >>= 2;
            }
            if p.decr {
                level_inc = ((level_inc * self.value) >> 15) as i16 as i32; // arithmetic shift, 16-bit wrap
            }
        }
        counter_inc = counter_inc.max(1);
        self.counter += counter_inc;
        if self.counter >= 0x8000 {
            self.counter = 0;
            self.value = (self.value + level_inc).clamp(0, 0x7fff);
        }
        if self.phase == Phase::Sustain {
            // no target test in sustain; a sustain that reaches 0 stops the voice
            if self.value == 0 {
                self.stop();
            }
            return self.value != 0;
        }
        // the target is tested EVERY sample, not only on level updates
        if (!p.decr && self.value >= p.target) || (p.decr && self.value <= p.target) {
            self.phase = Phase::from_index(self.phase as u8 + 1);
        }
        if self.phase > Phase::Release {
            self.stop();
            return false;
        }
        true
    }
}

pub const BLOCK_BYTES: u32 = 16;
pub const BLOCK_SAMPLES: u32 = 28;
/// Block flag byte (+1 of each 16-byte ADPCM block).
pub const FLAG_END: u8 = 1;
pub const FLAG_REPEAT: u8 = 2;
pub const FLAG_START: u8 = 4;
/// The SPU2 caps PITCH at 0x3fff (4 source samples per output sample at most is 0x4000).
pub const MAX_PITCH: u32 = 0x3fff;

/// What a cursor step did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Run,
    /// The sample ended without repeat (or ran off the data): V_Voice::Stop, the envelope goes to 0.
    Stop,
}

/// PS-ADPCM prediction filters (block header high nibble; 5..15 have no filter, (0, 0)).
pub const FILTERS: [(i32, i32); 5] = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)];

/// Decodes one 16-byte block (header byte: shift low nibble, filter high nibble; 14 data bytes, low nibble first) with the
/// running history `prev1` (newest) / `prev2`. Each sample: the signed nibble at the top of a 32-bit word, arithmetic right
/// shift by 16 + shift, plus `(f0 * prev1 + f1 * prev2 + 32) >> 6`, clamped to 16 bits [C: PCSX2 Mixer.cpp XA_decode_block].
/// Shifts 13..15 therefore act as a plain shift (no block on the disc uses them: decomp/notes/sound_grains.md stage 10a).
pub fn decode_block(blk: &[u8], prev1: &mut i32, prev2: &mut i32) -> [i16; 28] {
    let shift = 16 + (blk[0] & 15) as u32;
    let (f0, f1) = FILTERS.get((blk[0] >> 4) as usize).copied().unwrap_or((0, 0));
    let mut out = [0i16; 28];
    for (k, o) in out.iter_mut().enumerate() {
        let b = blk[2 + k / 2] as u32;
        let nib = if k & 1 == 0 { b & 15 } else { b >> 4 };
        let s = (((nib << 28) as i32) >> shift) + ((f0 * *prev1 + f1 * *prev2 + 32) >> 6);
        let s = s.clamp(-0x8000, 0x7fff);
        *prev2 = *prev1;
        *prev1 = s;
        *o = s as i16;
    }
    out
}

/// The voice's sample engine as the SPU2 runs it (stage 10a, [C] PCSX2 master Mixer.cpp DecodeSamples / ConsumeSamples,
/// spu2sys.cpp V_Voice::Start): a decoder that runs ahead of the play position through a 32-entry FIFO of decoded samples.
/// Every output sample: the flags of NextA's block are read (loop-start flag: LoopStartA := that block); when fewer than 13
/// decoded samples are waiting, one group of 4 is decoded and NextA advances one halfword; when NextA leaves a block, its
/// loop-end flag sets ENDX and jumps to LoopStartA, and without the repeat flag the voice STOPS THERE (envelope 0), i.e.
/// while the last 12..16 decoded samples are still unplayed. The voice reads `fifo[DecPosRead + 0..=3]` (Gaussian window)
/// and the pitch step advances DecPosRead.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Cursor {
    /// NextA's block (byte offset into the sample data).
    pub block: u32,
    /// NextA & 7: the halfword inside the block (1..=7 = the next 4-sample group to decode is group - 1).
    pub group: u32,
    /// LoopStartA (byte offset): the last block NextA entered with the loop-start flag (KON: StartA [I], see `start`).
    pub loop_start: u32,
    /// LoopFlags: the flag byte of NextA's block, read every sample.
    pub flags: u8,
    /// SP low 12 bits: the interpolation phase (Gaussian table index = (frac & 0xff0) >> 4).
    pub frac: u32,
    /// DecPosWrite / DecPosRead: running sample counters since KON (FIFO index = counter & 31).
    pub w: u32,
    pub r: u32,
    pub fifo: [i16; 32],
    /// ADPCM history after the last decoded block.
    pub prev1: i32,
    pub prev2: i32,
    /// SBuffer: NextA's block decoded (None until its first group is needed).
    pub sbuf: Option<[i16; 28]>,
    /// ENDX bit of this voice: set at every loop-end block, cleared by KON.
    pub endx: bool,
}

impl Cursor {
    /// KON (V_Voice::Start): SP = 0, NextA = StartA + 1 halfword, history and FIFO counters 0, LoopFlags 0. The FIFO contents
    /// are not cleared (never read before they are written). LoopStartA := StartA [I: the SPU2 keeps the LSA register; the
    /// driver never writes LSA, and every looping sample of the banks starts its loop with the flag].
    pub fn start(offset: u32, _data: &[u8]) -> Cursor {
        Cursor { block: offset, group: 1, loop_start: offset, ..Cursor::default() }
    }

    /// Interpolation table index for the current phase.
    pub fn gauss_index(&self) -> usize {
        ((self.frac & 0xff0) >> 4) as usize
    }

    /// The four samples of the interpolation window, oldest first: `fifo[DecPosRead + 0..=3]`.
    pub fn window(&self) -> [i32; 4] {
        std::array::from_fn(|k| self.fifo[(self.r.wrapping_add(k as u32) & 31) as usize] as i32)
    }

    /// Samples consumed since KON.
    pub fn played(&self) -> u32 {
        self.r
    }

    /// The decoder part of one output sample (before the voice value). `Step::Stop` = the voice stops this sample (end block
    /// without repeat, or NextA ran off the sample data).
    pub fn decode(&mut self, data: &[u8]) -> Step {
        let Some(blk) = data.get(self.block as usize..self.block as usize + 16) else { return Step::Stop };
        self.flags = blk[1];
        if self.flags & FLAG_START != 0 {
            self.loop_start = self.block;
        }
        if self.w.wrapping_sub(self.r) as i32 > 12 {
            return Step::Run;
        }
        let (p1, p2) = (&mut self.prev1, &mut self.prev2);
        let buf = self.sbuf.get_or_insert_with(|| decode_block(blk, p1, p2));
        let at = (self.group - 1) as usize * 4;
        for k in 0..4 {
            self.fifo[(self.w.wrapping_add(k as u32) & 31) as usize] = buf[at + k];
        }
        self.w = self.w.wrapping_add(4);
        self.group += 1;
        if self.group < 8 {
            return Step::Run;
        }
        // NextA left the block: its flags apply now.
        self.group = 1;
        self.sbuf = None;
        if self.flags & FLAG_END != 0 {
            self.endx = true;
            self.block = self.loop_start;
            if self.flags & FLAG_REPEAT == 0 {
                return Step::Stop;
            }
        } else {
            self.block += BLOCK_BYTES;
        }
        Step::Run
    }

    /// UpdatePitch + ConsumeSamples: SP += min(pitch, 0x3fff); DecPosRead += SP >> 12; SP &= 0xfff.
    pub fn consume(&mut self, pitch: u32) {
        self.frac += pitch.min(MAX_PITCH);
        self.r = self.r.wrapping_add(self.frac >> 12);
        self.frac &= 0xfff;
    }
}

/// One of the 48 SPU2 voices as the driver programs it (registers VOLL/VOLR/PITCH/SSA/ADSR1/ADSR2, reverb mask bit).
#[derive(Clone, Debug)]
pub struct Spu2Voice {
    pub env: Env,
    pub cur: Cursor,
    /// VOLL / VOLR register values (fixed mode, 15 bit, bit 14 = phase invert): the driver writes `(result & 0xffff) >> 1`.
    pub vol_l: u16,
    pub vol_r: u16,
    pub pitch: u16,
    pub adsr1: u16,
    pub adsr2: u16,
    /// SSA: byte offset of the sample inside its bank's sample data.
    pub ssa: u32,
    pub reverb: bool,
}

impl Default for Spu2Voice {
    fn default() -> Self {
        Spu2Voice { env: Env::new(0, 0), cur: Cursor::default(), vol_l: 0, vol_r: 0, pitch: 0, adsr1: 0, adsr2: 0, ssa: 0, reverb: false }
    }
}

/// VOLL / VOLR register (fixed volume mode) -> signed volume, a multiplier of 1/0x8000 (bit 14 = phase invert).
pub fn volume_of(reg: u16) -> i32 {
    ((reg << 1) as i16) as i32
}

impl Spu2Voice {
    /// KON with the registers as currently written (ADSR, SSA, ...) over `data` = the bank's sample data.
    pub fn key_on(&mut self, data: &[u8]) {
        self.env = Env::new(self.adsr1, self.adsr2);
        self.env.key_on();
        self.cur = Cursor::start(self.ssa, data);
    }

    pub fn key_off(&mut self) {
        self.env.key_off();
    }

    pub fn envx(&self) -> u16 {
        self.env.envx()
    }

    /// Voice still produces sound (envelope not stopped).
    pub fn active(&self) -> bool {
        self.env.phase() != Phase::Stopped
    }

    /// One output sample in PCSX2 MixVoice order: decoder (may stop the voice: end block without repeat, V_Voice::Stop sets
    /// ENVX = 0), envelope, pitch step. False once the voice has stopped.
    pub fn tick(&mut self, data: &[u8]) -> bool {
        if self.cur.decode(data) == Step::Stop {
            self.env.stop();
            return false;
        }
        if !self.env.tick() {
            return false;
        }
        self.cur.consume(self.pitch as u32);
        true
    }

    /// `n` output samples; returns how many samples the voice was alive for.
    pub fn step_samples(&mut self, data: &[u8], n: u32) -> u32 {
        let mut alive = 0;
        for _ in 0..n {
            if self.tick(data) {
                alive += 1;
            }
        }
        alive
    }

    /// Mixer output for one interpolated sample `interp` (16 bit): `(ENVX * s) >> 15`, then `(VOL * s) >> 15` per side.
    pub fn mix(&self, interp: i32) -> (i32, i32) {
        let s = (self.env.envx() as i32 * interp) >> 15;
        ((volume_of(self.vol_l) * s) >> 15, (volume_of(self.vol_r) * s) >> 15)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// adsr_sim.run: key on, attack end, decay end, level after `hold` sustain samples, release samples from there.
    fn run(a1: u16, a2: u16, hold: u32) -> (u32, i32, u32, i32, i32, u32) {
        let mut e = Env::new(a1, a2);
        e.key_on();
        let mut n = 0;
        while e.phase() == Phase::Attack {
            e.tick();
            n += 1;
        }
        let att = n;
        let peak = e.value;
        while e.phase() == Phase::Decay {
            e.tick();
            n += 1;
        }
        let dec = n - att;
        let lvl_sus = e.value;
        for _ in 0..hold {
            e.tick();
        }
        let lvl_hold = e.value;
        e.key_off();
        let mut r = 0u32;
        while e.tick() {
            r += 1;
            assert!(r < 5_000_000, "release does not end");
        }
        (att, peak, dec, lvl_sus, lvl_hold, r + 1)
    }

    fn release_from(a1: u16, a2: u16, level: i32) -> u32 {
        let mut e = Env::from_state(a1, a2, Phase::Release, 0, level);
        let mut n = 0;
        while e.tick() {
            n += 1;
        }
        n + 1
    }

    #[test]
    fn common_pair_0x1eff_0xdfcc() {
        let (a1, a2) = (0x1EFF, 0xDFCC);
        let ph = decode(a1, a2);
        // attack: linear, shift 7, step 7-2 = 5, +5<<4 = +80 per sample
        assert_eq!(ph[0], PhaseCfg { decr: false, exp: false, shift: 7, step: 5, target: 0x7fff });
        // decay: SL 15 -> target 0x8000 above the maximum level
        assert_eq!(ph[1].target, 0x8000);
        // sustain: exp decrease, shift 31, step -5
        assert_eq!(ph[2], PhaseCfg { decr: true, exp: true, shift: 31, step: -5, target: 0 });
        // release: linear, shift 12, step -8
        assert_eq!(ph[3], PhaseCfg { decr: true, exp: false, shift: 12, step: -8, target: 0 });
        let (att, peak, dec, lvl_sus, lvl_hold, rel) = run(a1, a2, 48000);
        assert_eq!(att, 410); // 0x7fff / 80 = 409.6
        assert_eq!(peak, 0x7fff);
        assert_eq!(dec, 1);
        assert_eq!(lvl_sus, 0x7fff);
        assert_eq!(lvl_hold, 0x7fff - 5); // one sustain update (-5) per 32768 samples
        assert_eq!(rel, 8192); // 2 samples per -8 from 0x7fff: 4096 updates
        assert_eq!(release_from(a1, a2, 0x7fff), 8192);
        // ENVX at the end of attack is exactly 0x7fff and stays there through the decay sample
        let mut e = Env::new(a1, a2);
        e.key_on();
        for _ in 0..409 {
            assert!(e.tick());
        }
        assert_eq!(e.envx(), 409 * 80);
        assert_eq!(e.phase(), Phase::Attack);
        e.tick();
        assert_eq!((e.phase(), e.envx()), (Phase::Decay, 0x7fff));
        e.tick();
        assert_eq!((e.phase(), e.envx()), (Phase::Sustain, 0x7fff));
    }

    // (adsr1, adsr2, attack samples, peak, decay samples, level after decay, level after 48000 sustain samples, release samples)
    // from scratch_grain/adsr_golden.py (adsr_sim.py, PCSX2 form): the 8 top pairs of the banks plus 40 random pairs.
    // (adsr1, adsr2, attack samples, peak, decay samples, level after decay, level after 48000 sustain samples, release samples)
    const ADSR_GOLDEN: &[(u16, u16, u32, i32, u32, i32, i32, u32)] = &[
        (0x1EFF, 0xDFCC, 410, 32767, 1, 32767, 32762, 8192),
        (0x80FF, 0x9FC0, 6, 32767, 1, 32767, 32767, 2),
        (0xA5FF, 0xCA80, 2389, 32767, 1, 32767, 0, 1),
        (0xAEFF, 0x5089, 11468, 32767, 1, 32767, 23767, 743),
        (0x809F, 0xCA00, 6, 32767, 1, 32735, 0, 1),
        (0x80FF, 0x9FCA, 6, 32767, 1, 32767, 32767, 2048),
        (0x80B8, 0x9FC0, 6, 32767, 2190, 18428, 18432, 2),
        (0xAEFF, 0x9FCC, 11468, 32767, 1, 32767, 32767, 8192),
        (0x17E7, 0x4747, 128, 32767, 20792, 16383, 0, 1),
        (0x97D8, 0x140A, 221, 32767, 8760, 18428, 19079, 1193),
        (0x8E10, 0x6F83, 44, 32767, 10, 1844, 0, 1),
        (0xB775, 0x8920, 57332, 32767, 249, 12284, 32767, 15),
        (0xA7D4, 0x64A7, 3581, 32767, 17224, 10238, 0, 1),
        (0x8F88, 0xE6EC, 53, 32767, 292, 18403, 0, 1),
        (0xB67E, 0x46ED, 45872, 32767, 17, 30648, 0, 1),
        (0x2990, 0xC347, 2731, 32767, 2619, 2047, 0, 1),
        (0x22D3, 0x692C, 820, 32767, 19952, 8192, 0, 1),
        (0x1624, 0xC245, 103, 32767, 9, 9849, 0, 1),
        (0x0958, 0xEA08, 11, 32767, 37, 18281, 0, 1),
        (0x928B, 0x7FC0, 91, 32767, 146, 24567, 24562, 2),
        (0x34D4, 0xD203, 18724, 32767, 17224, 10238, 9113, 5),
        (0x0F1A, 0x9E88, 32, 32767, 2, 18431, 18436, 289),
        (0x15C6, 0x0D69, 86, 32767, 6222, 14335, 32767, 4154),
        (0x982D, 0xEF43, 258, 32767, 1, 28671, 13953, 7),
        (0xA730, 0x3FEE, 3581, 32767, 43, 2037, 2041, 16328),
        (0x1618, 0x984D, 103, 32767, 2, 18431, 18461, 9232),
        (0xB216, 0x6382, 22936, 32767, 3, 13823, 0, 1),
        (0xA282, 0x8B0C, 1435, 32767, 840, 6133, 32767, 8192),
        (0x079A, 0x1363, 8, 32767, 377, 22513, 23635, 123),
        (0x9F65, 0x8FCE, 893, 32767, 125, 12250, 24250, 24256),
        (0x0B8B, 0x4CE1, 16, 32767, 146, 24567, 0, 1),
        (0x2F09, 0x558A, 8192, 32767, 1, 16383, 16107, 1007),
        (0xB740, 0xB3C0, 57332, 32767, 88, 1990, 2738, 1),
        (0x94CC, 0xE58D, 131, 32767, 1610, 26620, 0, 1),
        (0xA29B, 0x86E8, 1435, 32767, 290, 24556, 32767, 2427),
        (0x9F22, 0xFE09, 893, 32767, 13, 5771, 5769, 181),
        (0x0BF5, 0x04E7, 16, 32767, 57968, 12287, 32767, 1389),
        (0x0719, 0xA5A4, 8, 32767, 2, 18431, 32767, 237),
        (0x1BED, 0xCE4C, 256, 32767, 4096, 28671, 6422, 1606),
        (0xAB4F, 0x35E1, 7165, 32767, 1, 31743, 31927, 34),
        (0x0A64, 0xC24E, 13, 32767, 148, 10216, 0, 1),
        (0x84CE, 0xB405, 8, 32767, 512, 30719, 30880, 61),
        (0x177B, 0xCB2E, 128, 32767, 74, 24495, 0, 1),
        (0x2666, 0x964E, 1639, 32767, 106, 14229, 14367, 14368),
        (0xAFA2, 0x24A0, 14333, 32767, 3169, 6142, 32767, 15),
        (0xA213, 0x862D, 1435, 32767, 5, 7775, 32767, 44528),
        (0x24B0, 0x9503, 1171, 32767, 9084, 2048, 2370, 2),
        (0x8D63, 0x10E4, 37, 32767, 176, 8189, 14189, 211),
    ];


    #[test]
    fn adsr_golden_pairs() {
        for &(a1, a2, att, peak, dec, lvl_sus, lvl_hold, rel) in ADSR_GOLDEN {
            let got = run(a1, a2, 48000);
            assert_eq!(got, (att, peak, dec, lvl_sus, lvl_hold, rel), "adsr1 {a1:#06x} adsr2 {a2:#06x}");
        }
    }

    #[test]
    fn sustain_reaching_zero_stops_the_voice() {
        // 0xA5FF/0xCA80: sustain exp decrease reaches 0 within 1 s and a sustain at 0 stops the voice (ENVX 0)
        let mut e = Env::new(0xA5FF, 0xCA80);
        e.key_on();
        let mut stopped_at = None;
        for i in 0..48000u32 {
            if !e.tick() {
                stopped_at = Some(i);
                break;
            }
        }
        assert!(stopped_at.is_some(), "voice should stop within 1 s");
        assert_eq!((e.phase(), e.envx()), (Phase::Stopped, 0));
        // key-off of a stopped voice does nothing
        e.key_off();
        assert_eq!(e.phase(), Phase::Stopped);
    }

    #[test]
    fn exp_attack_slows_down_above_0x6000() {
        // 0xA5FF: exp attack sh9 step +6: 2389 samples in total (note table)
        let (att, ..) = run(0xA5FF, 0xCA80, 0);
        assert_eq!(att, 2389);
    }

    /// 3 blocks of 16 bytes; flags per block.
    fn blocks(flags: &[u8]) -> Vec<u8> {
        let mut d = Vec::new();
        for &f in flags {
            let mut b = [0u8; 16];
            b[1] = f;
            d.extend_from_slice(&b);
        }
        d
    }

    fn voice(a1: u16, a2: u16, pitch: u16, ssa: u32) -> Spu2Voice {
        Spu2Voice { adsr1: a1, adsr2: a2, pitch, ssa, vol_l: 0x2000, vol_r: 0x2000, ..Default::default() }
    }

    #[test]
    fn one_shot_block_ends_with_env_zero() {
        // 0x80FF/0x9FC0: attack 6 samples to full level, then frozen at 0x7fff
        let data = blocks(&[0, 0, FLAG_END]); // end flag, no repeat
        let mut v = voice(0x80FF, 0x9FC0, 0x1000, 0);
        v.key_on(&data);
        // decode ahead at pitch 0x1000: group k (1-based, 4 samples) is decoded at output sample 4k - 15 once the FIFO is
        // primed (k >= 5); the 21st and last group of the end block at sample 69, which stops the voice there with 16
        // decoded samples unplayed (PCSX2 DecodeSamples)
        for i in 1..=68 {
            assert!(v.tick(&data), "sample {i}");
        }
        assert_eq!(v.envx(), 0x7fff);
        assert!(!v.cur.endx);
        assert_eq!((v.cur.played(), v.cur.w), (68, 80));
        assert!(!v.tick(&data));
        assert_eq!(v.cur.w, 84);
        assert!(v.cur.endx);
        assert_eq!((v.env.phase(), v.envx()), (Phase::Stopped, 0));
        assert!(!v.active());
        assert!(!v.tick(&data));
    }

    #[test]
    fn looping_block_never_ends() {
        // block 0 loop start, block 2 end + repeat: loops back to block 0 forever
        let data = blocks(&[FLAG_START, 0, FLAG_END | FLAG_REPEAT]);
        let mut v = voice(0x80FF, 0x9FC0, 0x1000, 0);
        v.key_on(&data);
        for _ in 0..(84 * 50 + 13) {
            assert!(v.tick(&data));
        }
        assert!(v.cur.endx);
        assert_eq!(v.envx(), 0x7fff);
        assert_eq!(v.cur.played(), 84 * 50 + 13);
        assert_eq!(v.cur.loop_start, 0);
        // the window is the sample stream in play order: an all-zero bank decodes to zeros
        assert_eq!(v.cur.window(), [0; 4]);
    }

    #[test]
    fn loop_returns_to_the_start_flagged_block() {
        // block 1 carries the loop-start flag: after the end block the sample resumes at block 1, not block 0
        let data = blocks(&[0, FLAG_START, FLAG_END | FLAG_REPEAT]);
        let mut v = voice(0x80FF, 0x9FC0, 0x1000, 0);
        v.key_on(&data);
        assert_eq!(v.cur.loop_start, 0);
        // the decoder leaves block 0 at sample 13 (its 7th group) and reads block 1's flags at sample 14
        for _ in 0..13 {
            v.tick(&data);
        }
        assert_eq!((v.cur.block, v.cur.loop_start), (16, 0));
        v.tick(&data);
        assert_eq!((v.cur.block, v.cur.loop_start), (16, 16));
        for _ in 14..68 {
            v.tick(&data);
        }
        assert_eq!((v.cur.block, v.cur.endx), (32, false));
        v.tick(&data);
        assert_eq!((v.cur.block, v.cur.group, v.cur.endx), (16, 1, true));
    }

    #[test]
    fn pitch_is_capped_at_0x3fff() {
        let data = blocks(&[0; 400]);
        let mut a = voice(0x80FF, 0x9FC0, 0xffff, 0);
        let mut b = voice(0x80FF, 0x9FC0, 0x3fff, 0);
        a.key_on(&data);
        b.key_on(&data);
        for _ in 0..1000 {
            a.tick(&data);
            b.tick(&data);
        }
        assert_eq!(a.cur, b.cur);
        // 1000 samples at 0x3fff: (1000 * 0x3fff) >> 12 = 3999 source samples
        assert_eq!(a.cur.played(), 3999);
        // pitch 0x1000 = one source sample per output sample, pitch 0x2000 = two
        let mut c = voice(0x80FF, 0x9FC0, 0x2000, 0);
        c.key_on(&data);
        c.step_samples(&data, 30);
        assert_eq!(c.cur.played(), 60);
    }

    #[test]
    fn koff_applies_before_kon_in_one_sample() {
        let data = blocks(&[FLAG_START, 0, FLAG_END | FLAG_REPEAT]);
        let mut v = voice(0x1EFF, 0xDFCC, 0x1000, 0);
        v.key_on(&data);
        for _ in 0..600 {
            v.tick(&data);
        }
        assert_eq!(v.env.phase(), Phase::Sustain);
        // driver order inside one tick: KOFF first, then KON of the same voice: the voice restarts its attack, no release
        v.key_off();
        assert_eq!(v.env.phase(), Phase::Release);
        v.key_on(&data);
        assert_eq!((v.env.phase(), v.envx()), (Phase::Attack, 0));
        assert!(v.tick(&data));
        assert_eq!(v.envx(), 80);
        assert!(!v.cur.endx);
        // KON alone on a stopped voice, and KOFF alone: release from the current level
        let mut w = voice(0x1EFF, 0xDFCC, 0x1000, 0);
        w.key_on(&data);
        for _ in 0..100 {
            w.tick(&data);
        }
        let level = w.envx();
        w.key_off();
        w.tick(&data);
        assert_eq!(w.env.phase(), Phase::Release);
        assert!(w.envx() <= level);
    }

    #[test]
    fn adpcm_block_decode() {
        // shift 2, filter 1 (60/64): nibbles 1, -1 (0xf), 7, -8; history carried
        let mut blk = [0u8; 16];
        blk[0] = 0x12;
        blk[2] = 0xf1;
        blk[3] = 0x87;
        let (mut p1, mut p2) = (100, 0);
        let s = decode_block(&blk, &mut p1, &mut p2);
        // 1 << 12 >> 2 = 1024, + (60*100 + 32) >> 6 = 94 -> 1118; then -1024 + (60*1118 + 32) >> 6 = 1048 -> 24
        assert_eq!(&s[..2], &[1118, 24]);
        assert_eq!(s[2], ((7 << 12) >> 2) + ((60 * 24 + 32) >> 6));
        assert_eq!((p1, p2), (s[27] as i32, s[26] as i32));
        // shift 13: a plain arithmetic shift of the top-aligned nibble (-8 << 12 >> 13 = -4), no filter
        let mut b13 = [0u8; 16];
        b13[0] = 0x0d;
        b13[2] = 0x08;
        let s = decode_block(&b13, &mut 0, &mut 0);
        assert_eq!(s[0], -4);
        // clamp
        let mut bc = [0u8; 16];
        bc[0] = 0x40;
        bc[2] = 0x77;
        let (mut p1, mut p2) = (32767, 32767);
        assert_eq!(decode_block(&bc, &mut p1, &mut p2)[0], 32767);
    }

    #[test]
    fn fifo_window_reads_the_decoded_stream() {
        let mut data = blocks(&[0, FLAG_END]);
        for (k, x) in data.iter_mut().enumerate() {
            if k % 16 >= 2 {
                *x = (k as u8).wrapping_mul(37);
            }
        }
        data[0] = 0x14;
        data[16] = 0x23;
        let (mut p1, mut p2) = (0, 0);
        let mut want: Vec<i32> = decode_block(&data[..16], &mut p1, &mut p2).iter().map(|&s| s as i32).collect();
        want.extend(decode_block(&data[16..], &mut p1, &mut p2).iter().map(|&s| s as i32));
        let mut v = voice(0x80FF, 0x9FC0, 0x1000, 0);
        v.key_on(&data);
        for n in 0..40 {
            assert_eq!(v.cur.decode(&data), Step::Run);
            assert_eq!(v.cur.window().to_vec(), want[n..n + 4].to_vec(), "sample {n}");
            v.cur.consume(0x1000);
        }
        // the 41st sample decodes the last group of the end block: the voice stops with 16 samples unplayed
        assert_eq!(v.cur.decode(&data), Step::Stop);
        assert_eq!(v.cur.w - v.cur.r, 16);
    }

    #[test]
    fn mix_and_phase_invert() {
        let mut v = voice(0x80FF, 0x9FC0, 0x1000, 0);
        let data = blocks(&[FLAG_START, FLAG_END | FLAG_REPEAT]);
        v.key_on(&data);
        v.step_samples(&data, 10);
        assert_eq!(v.envx(), 0x7fff);
        // register 0x2000 = +0x4000 (half scale); the driver's (result & 0xffff) >> 1 puts the invert bit at bit 14
        assert_eq!(volume_of(0x2000), 0x4000);
        let (l, r) = v.mix(10000);
        assert_eq!((l, r), (4999, 4999)); // (0x7fff * 10000) >> 15 = 9999; (0x4000 * 9999) >> 15 = 4999
        // negative result -0x2000 -> (0xe000) >> 1 = 0x7000: bit 14 set = phase inverted
        v.vol_l = ((-0x2000i32 as u16) >> 1) as u16;
        assert_eq!(v.vol_l, 0x7000);
        assert_eq!(volume_of(v.vol_l), -0x2000);
        let (l, r) = v.mix(10000);
        assert_eq!((l, r), (-2500, 4999));
    }
}

// ---------------------------------------------------------------- savestate (oracle snapfile section)
use crate::snd989::persist::{In, Out};

impl Env {
    pub fn save(&self, o: &mut Out) {
        for c in &self.cfg {
            o.bool(c.decr);
            o.bool(c.exp);
            o.u32(c.shift);
            o.i32(c.step);
            o.i32(c.target);
        }
        o.u8(self.phase as u8);
        o.u32(self.counter);
        o.i32(self.value);
    }
    pub fn load(i: &mut In) -> Env {
        let cfg = std::array::from_fn(|_| PhaseCfg { decr: i.bool(), exp: i.bool(), shift: i.u32(), step: i.i32(), target: i.i32() });
        Env { cfg, phase: Phase::from_index(i.u8()), counter: i.u32(), value: i.i32() }
    }
}

impl Cursor {
    pub fn save(&self, o: &mut Out) {
        o.u32(self.block);
        o.u32(self.group);
        o.u32(self.loop_start);
        o.u8(self.flags);
        o.u32(self.frac);
        o.u32(self.w);
        o.u32(self.r);
        for s in self.fifo {
            o.u16(s as u16);
        }
        o.i32(self.prev1);
        o.i32(self.prev2);
        o.bool(self.sbuf.is_some());
        for s in self.sbuf.unwrap_or([0; 28]) {
            o.u16(s as u16);
        }
        o.bool(self.endx);
    }
    pub fn load(i: &mut In) -> Cursor {
        let (block, group, loop_start, flags, frac, w, r) = (i.u32(), i.u32(), i.u32(), i.u8(), i.u32(), i.u32(), i.u32());
        let fifo = std::array::from_fn(|_| i.u16() as i16);
        let (prev1, prev2, has) = (i.i32(), i.i32(), i.bool());
        let buf: [i16; 28] = std::array::from_fn(|_| i.u16() as i16);
        Cursor { block, group, loop_start, flags, frac, w, r, fifo, prev1, prev2, sbuf: has.then_some(buf), endx: i.bool() }
    }
}

impl Spu2Voice {
    pub fn save(&self, o: &mut Out) {
        self.env.save(o);
        self.cur.save(o);
        o.u16(self.vol_l);
        o.u16(self.vol_r);
        o.u16(self.pitch);
        o.u16(self.adsr1);
        o.u16(self.adsr2);
        o.u32(self.ssa);
        o.bool(self.reverb);
    }
    pub fn load(i: &mut In) -> Spu2Voice {
        let env = Env::load(i);
        let cur = Cursor::load(i);
        Spu2Voice { env, cur, vol_l: i.u16(), vol_r: i.u16(), pitch: i.u16(), adsr1: i.u16(), adsr2: i.u16(), ssa: i.u32(), reverb: i.bool() }
    }
}
