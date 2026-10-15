//! Stage 5: the LFO layer of 989snd (R&C1 PAL). Verified against the disassembly (scratch_grain/mdis.py):
//! waves 0x1f10 (none) 0x1f18 (table 0x17d60) 0x1f30 (square) 0x1f50 (triangle) 0x1fcc (saw) 0x2024 (random),
//! 0x20bc start, 0x2240 / 0x21a4 list add / remove, 0x22ec one step (targets 1-6), 0x27e8 amplitude, 0x2748 the loop.
//! A literal python replay is scratch_grain/lfo_sim.py; `lfo_golden.rs` is generated from it.
//!
//! What the layer does: up to [`MAX_LFOS`] = 10 slots (4 per handler, h+0x54 + 0x30*k) run in one global list in start
//! order. On every EVEN tick counter (120 Hz) each slot advances `phase += rate * 2` (27-bit, one wrap) and computes its
//! wave at the phase BEFORE the step (`phase >> 16` = 0..2047). Targets: 1 volume offset h+0x30, 2 pan offset h+0x34,
//! 3 pitch-mod offset h+0x3c, 4 bend offset h+0x38, 5 rate of another slot, 6 depth of another slot. A changed offset
//! of target 1 / 2 re-applies volume and pan to all of the handler's voices (0x5aac with the children flag set: no
//! children), of 3 / 4 the pitch (0x5ffc).
use super::handler::{Core, Lfo, LfoSlot, Voices};
use super::irx::Irx;
use super::rng::Rng250;

/// The running list holds at most 10 slots (0x2240).
pub const MAX_LFOS: usize = 10;
/// Module address of the wave table (2048 x s16).
pub const SINE_ADDR: u32 = 0x17d60;
pub const SINE_LEN: usize = 2048;

/// The "sine" table of the module is a cosine, `trunc(0x7fff * cos(2 pi i / 2048))`: i = 0 is the peak (checked equal to
/// the disc's table by `sine_formula_equals_golden_and_disc`).
pub fn sine_formula() -> Vec<i16> {
    (0..SINE_LEN).map(|i| (32767.0 * (std::f64::consts::TAU * i as f64 / SINE_LEN as f64).cos()) as i16).collect()
}

/// The table as the module holds it (source of truth when the disc is present).
pub fn sine_from_irx(irx: &Irx) -> Option<Vec<i16>> {
    let b = irx.bytes(SINE_ADDR, SINE_LEN * 2)?;
    Some(b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
}

/// 0x27e8: amplitude A (+0x20) of a slot from its depth and, for target 1, the sound's own volume byte (`lb`, signed).
/// Targets 0, 5, 6 and above leave A unchanged. +0x24 is always cleared.
pub fn amplitude(sl: &mut LfoSlot, sound_volume: i8) {
    let d = sl.depth as i32;
    match sl.target {
        1 => sl.amp = (sound_volume as i32).wrapping_mul(d) >> 10,
        2 => sl.amp = (d * 45) >> 8,
        3 => sl.amp = (d * 381) >> 6,
        4 => sl.amp = (d.wrapping_shl(15).wrapping_sub(d)) >> 10,
        _ => {}
    }
    sl.state_24 = 0;
}

/// The wave functions. `idx` = phase >> 16 (0..2047); an index outside the table (impossible for the banks, a rate would
/// have to exceed 0x4000000) reads 0 here where retail would read the module's memory past the table.
/// Wave 5 (random) mutates the slot (+0x18 held value, +0x1c flag) and draws from the shared R250 stream.
pub fn wave_value(sl: &mut LfoSlot, idx: i32, sine: &[i16], rng: &mut Rng250) -> i32 {
    match sl.wave {
        0 => 0,
        1 => sine.get(idx as usize).map_or(0, |&v| v as i32),
        2 => {
            if idx < sl.aux as i32 {
                0x7fff
            } else {
                -0x7fff
            }
        }
        3 => triangle(idx),
        4 => saw(idx),
        5 => {
            if idx < 0x400 {
                if sl.state_1c == 0 {
                    sl.state_1c = 1;
                    let r1 = rng.next() as i32;
                    let r2 = rng.next() as i32;
                    sl.aux = (-(r1 & 0x7fff)).wrapping_mul(r2 & 1) as u32;
                }
            } else if sl.state_1c == 1 {
                sl.state_1c = 0;
                let r = rng.next() as i32;
                sl.aux = (((r & 0x7fff) - 0x3fff) << 1) as u32;
            }
            sl.aux as i32
        }
        // the wave table continues with grain handlers (0x463c, 0x4644): not reachable from a bank (grain operand p3 is 0..5)
        _ => 0,
    }
}

/// 0x1f50. Divisions by 512 / 1024 round toward zero like the MIPS code (the numerators are non-negative here).
pub fn triangle(idx: i32) -> i32 {
    if idx < 0x200 {
        ((idx << 15) - idx) >> 9
    } else if idx < 0x600 {
        let v1 = idx - 0x200;
        0x7fff - ((((v1 << 15) - v1) << 1) >> 10)
    } else {
        let v1 = idx - 0x600;
        (((v1 << 15) - v1) >> 9) - 0x7fff
    }
}

/// 0x1fcc. Below 0x400: `idx * 0x7fff / 0x3ff` (the code multiplies by the signed reciprocal 0x80200803, which equals a
/// truncating division by 1023 for every input), from 0x400 on `((idx - 0x400) * 0x7fff >> 10) - 0x7fff`.
pub fn saw(idx: i32) -> i32 {
    if idx < 0x400 {
        let n = (idx << 15) - idx;
        let hi = ((n as i64 * 0x8020_0803u32 as i32 as i64) >> 32) as i32;
        ((hi.wrapping_add(n)) >> 9) - (n >> 31)
    } else {
        let v1 = idx - 0x400;
        (((v1 << 15) - v1) >> 10) - 0x7fff
    }
}

/// The running list and the wave table. `Driver<V, Lfos>` is the full layer; `Core::iteration` calls [`Lfo::update`] on
/// every even tick counter.
#[derive(Clone, Debug)]
pub struct Lfos {
    /// (handler slot, LFO index 0..3) in list order = start order (list head 0x18d60, count 0x18d64).
    pub list: Vec<(usize, usize)>,
    sine: Vec<i16>,
    /// Count of 0x22ec steps run (diagnostics).
    pub steps: u64,
}

impl Default for Lfos {
    fn default() -> Self {
        Lfos::new()
    }
}

impl Lfos {
    /// Wave table from the formula.
    pub fn new() -> Lfos {
        Lfos { list: Vec::new(), sine: sine_formula(), steps: 0 }
    }
    /// Wave table read from the module (the disc's 989snd IRX).
    pub fn from_irx(irx: &Irx) -> Result<Lfos, String> {
        let sine = sine_from_irx(irx).ok_or("wave table 0x17d60 not in the module")?;
        Ok(Lfos { list: Vec::new(), sine, steps: 0 })
    }
    /// Savestate adoption: the running list rebuilt from the IOP's list (head 0x18d60, links at slot+0x2c), already
    /// mapped to (handler, index) pairs by the caller. The slots' own fields come with the handlers.
    pub fn with_list(mut self, list: Vec<(usize, usize)>) -> Lfos {
        self.list = list;
        self
    }

    pub fn running(&self) -> usize {
        self.list.len()
    }

    /// 0x2240: true = in the list afterwards (also when it already was), false = the list is full (10): no update.
    fn add(&mut self, handler: usize, index: usize, sl: &mut LfoSlot) -> bool {
        if sl.in_list {
            return true;
        }
        if self.list.len() == MAX_LFOS {
            return false;
        }
        sl.in_list = true;
        self.list.push((handler, index));
        true
    }

    /// 0x22ec: one step of one slot. Returns 0 (offset unchanged), 1 (volume / pan offset changed), 2 (pitch offset changed).
    /// Targets 5 / 6 rewrite another slot of the same handler and return 0.
    pub fn step(&mut self, core: &mut Core, handler: usize, index: usize) -> u8 {
        self.steps += 1;
        let (h, rng) = core.lfo_access(handler);
        let target = h.lfo[index].target;
        if !(1..=6).contains(&target) {
            return 0;
        }
        let sound_volume = h.sound_ref().map_or(0, |s| s.volume);
        // phase step (the same 12 instructions in every target), the wave sees the phase before the step
        let sl = &mut h.lfo[index];
        let old = sl.phase as i32;
        let mut new = old.wrapping_add((sl.rate as i32).wrapping_shl(1));
        let idx = old >> 16;
        if 0x7ff_ffff < new {
            new = new.wrapping_add(0xf800_0000u32 as i32);
        }
        sl.phase = new as u32;
        let mut w = wave_value(sl, idx, &self.sine, rng);
        if sl.flags & 1 != 0 {
            w = w.wrapping_neg();
        }
        let amp = sl.amp;
        let depth = sl.depth;
        let link = sl.link as usize;
        match target {
            1 => {
                let v = amp.wrapping_mul(w.wrapping_sub(0x7fff)) >> 16;
                if v == h.vol_off as i32 {
                    return 0;
                }
                h.vol_off = v as i16;
                1
            }
            2 => {
                let v = amp.wrapping_mul(w) >> 15;
                if v == h.pan_off as i32 {
                    return 0;
                }
                h.pan_off = v as i16;
                1
            }
            3 => {
                let v = amp.wrapping_mul(w) >> 15;
                if v == h.pmod_off as i32 {
                    return 0;
                }
                h.pmod_off = v as i16;
                2
            }
            4 => {
                let v = amp.wrapping_mul(w) >> 15;
                if v == h.bend_off as i32 {
                    return 0;
                }
                h.bend_off = v as i16;
                2
            }
            5 | 6 => {
                if link >= 4 {
                    // retail would write past the 4 slots of the handler; no bank does
                    return 0;
                }
                let o = &mut h.lfo[link];
                if target == 5 {
                    let v = ((o.rate_base >> 8).wrapping_mul(depth as u32)) >> 10;
                    let v = (v as i32).wrapping_mul(w) >> 15;
                    o.rate = o.rate_base.wrapping_add((v as u32).wrapping_shl(8));
                } else {
                    let v = (o.depth_wide.wrapping_mul(depth as u32)) >> 10;
                    let v = (v as i32).wrapping_mul(w) >> 15;
                    o.depth = (o.depth_wide as u16).wrapping_add(v as u16);
                    amplitude(o, sound_volume);
                }
                0
            }
            _ => 0,
        }
    }
}

impl Lfo for Lfos {
    /// 0x20bc: random wave = two draws now (value and sign), amplitude, list add, and ONE step at once (its result is
    /// ignored: voices are not re-applied until a later step changes the offset).
    fn start(&mut self, core: &mut Core, handler: usize, index: usize) {
        let (h, rng) = core.lfo_access(handler);
        let sound_volume = h.sound_ref().map_or(0, |s| s.volume);
        let mut sl = std::mem::take(&mut h.lfo[index]);
        if sl.wave == 5 {
            let r1 = rng.next() as i32;
            let r2 = rng.next() as i32;
            sl.state_1c = 1;
            sl.aux = (-(r1 & 0x7fff)).wrapping_mul(r2 & 1) as u32;
        }
        amplitude(&mut sl, sound_volume);
        let listed = self.add(handler, index, &mut sl);
        h.lfo[index] = sl;
        if listed {
            self.step(core, handler, index);
        }
    }

    /// 0x21a4: clear the in-list bit and unlink (list order of the others is kept).
    fn stop(&mut self, handler: usize, index: usize, slot: &mut LfoSlot) {
        slot.in_list = false;
        if let Some(p) = self.list.iter().position(|&e| e == (handler, index)) {
            self.list.remove(p);
        }
    }

    /// 0x2748 (called on even tick counters only): every listed slot steps; a changed volume / pan offset re-applies
    /// volume and pan (0x5aac(handle, keep, keep, children = 1)), a changed pitch offset the pitch (0x5ffc).
    fn update(&mut self, core: &mut Core, voices: &mut dyn Voices) {
        let mut i = 0;
        while i < self.list.len() {
            let (h, k) = self.list[i];
            let r = self.step(core, h, k);
            if r & 1 != 0 {
                core.update_volume(h, 0x7fff_ffff, -2, voices);
            }
            if r & 2 != 0 {
                core.reapply_pitch(h, voices);
            }
            i += 1;
        }
    }
}

#[cfg(test)]
#[path = "lfo_tests.rs"]
mod tests;

// ---------------------------------------------------------------- savestate (oracle snapfile section)
use super::persist::{In, Out};

impl Lfos {
    /// The running list and the step counter (the wave table is rebuilt by the constructor).
    pub fn save(&self, o: &mut Out) {
        o.len(self.list.len());
        for (s, k) in &self.list {
            o.len(*s);
            o.len(*k);
        }
        o.u64(self.steps);
    }
    pub fn load(&mut self, i: &mut In) {
        let n = i.len();
        self.list = (0..n).map(|_| (i.len(), i.len())).collect();
        self.steps = i.u64();
    }
}
