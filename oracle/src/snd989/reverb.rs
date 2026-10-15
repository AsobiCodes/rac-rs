//! 989snd reverb driver (stage 10e): the per-core effect state of the IOP module and its commands. Everything here is the
//! module's own logic, decoded from its disassembly ([C], module addresses in the comments); the register effects are
//! returned as [`FxOp`]s that the host logs as SPU2 events (`Reg::EvolL` ... `Reg::FxClear`) and applies to its core image.
//! Layout (module base + offset): mode[c] 0x196f0 (init -1), area[c] 0x196f8, fixed[c] 0x19700 (init -1), depth L / R u16
//! 0x19708 / 0x1970c + 2c, channel word 0x19710 + 4c, pending 0x19718 + 4c, size table 0x19720, fade records 0x19a24 + 56c.
//! Written from the module's behaviour and the public SPU2 / libsd register semantics; no emulator code is used.
use super::driver::Spu2Alloc;
use super::persist::{In, Out};
use super::voices::Reg;

/// The effect work-area sizes in bytes, by libsd mode 0..=9 (OFF, ROOM, STUDIO_A, STUDIO_B, STUDIO_C, HALL, SPACE, ECHO, DELAY,
/// PIPE) = the libsd preset units * 8. The module keeps them at 0x19720; `adopt` compares them with the live module's table.
pub const SIZES: [u32; 10] = [0x80, 0x26c0, 0x1f40, 0x4840, 0x6fe0, 0xade0, 0xf6c0, 0x18040, 0x18040, 0x3c00];
/// Driver ticks the work-area clear takes before the effect is switched on (0xc198 sets the pending word to this).
pub const PENDING_TICKS: u32 = 10;

/// The driver's error codes (diagnostics): fixed area allocation, dynamic allocation, area too small.
pub const ERR_FIXED_ALLOC: u32 = 0x55;
pub const ERR_ALLOC: u32 = 0x56;
pub const ERR_TOO_SMALL: u32 = 0x57;

/// One register effect, in driver order (same fields as `SpuWrite` without the tick).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxOp {
    pub reg: Reg,
    pub index: u8,
    pub value: u32,
    pub bank: u32,
}

fn op(reg: Reg, core: usize, value: u32) -> FxOp {
    FxOp { reg, index: core as u8, value, bank: 0 }
}

/// One fade record (0x19a24 / 0x19a40 + 56c: step; 0x19a28 / 0x19a44: target).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fade {
    pub step: i16,
    pub target: i16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reverb {
    /// Current libsd mode of each core (-1 = never set).
    pub mode: [i32; 2],
    /// Byte address of the allocated effect block.
    pub area: [u32; 2],
    /// Mode of the fixed-size block cid 81 reserved (-1 / 0 = none: the block follows the mode).
    pub fixed: [i32; 2],
    /// Stored depth halves [core][left, right].
    pub depth: [[u16; 2]; 2],
    /// DMA channel + 1 of the running work-area clear (0 = none).
    pub chan: [u32; 2],
    pub pending: [u32; 2],
    pub fade: [[Fade; 2]; 2],
    /// delay / feedback of the last SetEffectAttr (modes 7 / 8; not kept in the module's variables).
    pub delay: [u32; 2],
    pub feedback: [u32; 2],
    pub sizes: [u32; 10],
    /// Last driver error (diagnostics, not saved).
    pub last_error: u32,
}

impl Default for Reverb {
    fn default() -> Self {
        Reverb::new()
    }
}

impl Reverb {
    pub fn new() -> Reverb {
        Reverb {
            mode: [-1; 2],
            area: [0; 2],
            fixed: [-1; 2],
            depth: [[0; 2]; 2],
            chan: [0; 2],
            pending: [0; 2],
            fade: [[Fade::default(); 2]; 2],
            delay: [0; 2],
            feedback: [0; 2],
            sizes: SIZES,
            last_error: 0,
        }
    }

    fn size(&self, mode: i32) -> Option<u32> {
        usize::try_from(mode).ok().and_then(|m| self.sizes.get(m).copied())
    }

    /// The block of core `c` the allocator holds (address, size): the fixed block, else the mode's.
    pub fn owned(&self, c: usize) -> Option<(u32, u32)> {
        let m = if self.fixed[c] > 0 {
            self.fixed[c]
        } else if self.mode[c] > 0 {
            self.mode[c]
        } else {
            return None;
        };
        Some((self.area[c], self.size(m)?))
    }

    /// cid 81 (0xbee0): the fixed effect block of the cores in `mask`.
    pub fn cid81(&mut self, mem: &mut Spu2Alloc, mask: u32, m: i32, out: &mut Vec<FxOp>) {
        for c in 0..2 {
            if mask & (1 << c) != 0 {
                self.fixed_block(mem, c, m, out);
            }
        }
    }

    /// 0xbf30(c, m).
    fn fixed_block(&mut self, mem: &mut Spu2Alloc, c: usize, m: i32, out: &mut Vec<FxOp>) {
        if self.mode[c] > 0 {
            self.change(mem, c, 0, 0, 0, out);
        }
        if self.fixed[c] > 0 {
            if let Some(n) = self.size(self.fixed[c]) {
                mem.release(self.area[c], n);
            }
        }
        self.fixed[c] = m;
        if m > 0 {
            match self.size(m).and_then(|n| mem.alloc_effect(n)) {
                Some(a) => self.area[c] = a,
                None => {
                    self.mode[c] = -1;
                    self.fixed[c] = -1;
                    self.last_error = ERR_FIXED_ALLOC;
                }
            }
        }
    }

    /// cid 80 (0xc094): effect type and depth of the cores in `mask`.
    pub fn cid80(&mut self, mem: &mut Spu2Alloc, mask: u32, mode: i32, depth: u32, delay: u32, feedback: u32, out: &mut Vec<FxOp>) {
        for c in 0..2 {
            if mask & (1 << c) != 0 {
                self.change(mem, c, mode, delay, feedback, out);
                self.depth_set(c, depth as u16, depth as u16, out);
            }
        }
    }

    /// 0xc560: store the depth halves; the EVOL registers follow unless the work area is being cleared.
    pub fn depth_set(&mut self, c: usize, l: u16, r: u16, out: &mut Vec<FxOp>) {
        self.depth[c] = [l, r];
        if self.chan[c] == 0 {
            out.push(op(Reg::EvolL, c, l as u32));
            out.push(op(Reg::EvolR, c, r as u32));
        }
    }

    /// sceSdSetEffectAttr: programs the preset (and writes EVOL = 0).
    fn set_attr(&mut self, c: usize, mode: i32, delay: u32, feedback: u32, out: &mut Vec<FxOp>) {
        self.delay[c] = delay;
        self.feedback[c] = feedback;
        out.push(FxOp { reg: Reg::FxAttr, index: c as u8, value: mode as u32, bank: ((delay & 0xffff) << 16) | (feedback & 0xffff) });
        out.push(op(Reg::EvolL, c, 0));
        out.push(op(Reg::EvolR, c, 0));
    }

    /// 0xc198(c, mode, delay, feedback): returns 0, or the driver's negative error (-13 area too small, -11 no memory).
    pub fn change(&mut self, mem: &mut Spu2Alloc, c: usize, mode: i32, delay: u32, feedback: u32, out: &mut Vec<FxOp>) -> i32 {
        let Some(size) = self.size(mode) else { return -1 };
        if mode == self.mode[c] {
            self.set_attr(c, mode, delay, feedback, out);
            return 0;
        }
        if self.fixed[c] > 0 && self.size(self.fixed[c]).is_some_and(|f| f < size) {
            self.last_error = ERR_TOO_SMALL;
            return -13;
        }
        if self.mode[c] > 0 {
            out.push(op(Reg::FxOn, c, 0));
            out.push(op(Reg::EvolL, c, 0));
            out.push(op(Reg::EvolR, c, 0));
        }
        if self.fixed[c] <= 0 {
            if self.mode[c] > 0 {
                if let Some(n) = self.size(self.mode[c]) {
                    mem.release(self.area[c], n);
                }
            }
            if mode != 0 {
                match mem.alloc_effect(size) {
                    Some(a) => self.area[c] = a,
                    None => {
                        self.area[c] = 0;
                        self.mode[c] = -1;
                        self.last_error = ERR_ALLOC;
                        return -11;
                    }
                }
            }
        }
        self.fade[c] = [Fade::default(); 2];
        if mode != 0 {
            out.push(op(Reg::Eea, c, self.area[c].wrapping_sub(1).wrapping_add(size)));
        }
        self.set_attr(c, mode, delay, feedback, out);
        if mode != 0 {
            self.chan[c] = 1;
            self.pending[c] = PENDING_TICKS;
            out.push(op(Reg::FxClear, c, mode as u32));
        }
        self.mode[c] = mode;
        0
    }

    /// cid 16 (wrapper 0x5f0 -> 0x34e0): fade the depth of the cores in `mask` to `target`, `rate` steps; `side` bit 0 = left,
    /// bit 1 = right (the game sends 3).
    pub fn cid16(&mut self, mask: u32, target: u32, rate: u32, side: u32, out: &mut Vec<FxOp>) {
        for c in 0..2 {
            if mask & (1 << c) != 0 {
                for s in 0..2 {
                    if side & (1 << s) != 0 {
                        self.fade_start(c, s, target as i16, rate as i32, out);
                    }
                }
            }
        }
    }

    /// 0x3550 for one side (0 = left, 1 = right).
    fn fade_start(&mut self, c: usize, s: usize, target: i16, rate: i32, out: &mut Vec<FxOp>) {
        self.fade[c][s].step = 0;
        if rate == 0 {
            let [l, r] = self.depth[c];
            if s == 0 {
                self.depth_set(c, target as u16, r, out);
            } else {
                self.depth_set(c, l, target as u16, out);
            }
            return;
        }
        let delta = target as i32 - self.depth[c][s] as i16 as i32;
        if delta == 0 {
            return;
        }
        self.fade[c][s].target = target;
        self.fade[c][s].step = if delta.abs() < rate { delta.signum() as i16 } else { (delta / rate) as i16 };
    }

    /// The driver tick hook, after the key flush (0xc5e0, then the fade pass 0x37a8).
    pub fn tick(&mut self, out: &mut Vec<FxOp>) {
        for c in 0..2 {
            if self.pending[c] != 0 {
                self.pending[c] -= 1;
                if self.pending[c] == 0 {
                    out.push(op(Reg::EvolL, c, self.depth[c][0] as u32));
                    out.push(op(Reg::EvolR, c, self.depth[c][1] as u32));
                    out.push(op(Reg::FxOn, c, 1));
                    self.chan[c] = 0;
                }
            }
        }
        for c in 0..2 {
            for s in 0..2 {
                let f = self.fade[c][s];
                if f.step == 0 {
                    continue;
                }
                let cur = self.depth[c][s] as i16 as i32;
                let next = cur + f.step as i32;
                let next = if f.step > 0 { next.min(f.target as i32) } else { next.max(f.target as i32) };
                let [l, r] = self.depth[c];
                if s == 0 {
                    self.depth_set(c, next as u16, r, out);
                } else {
                    self.depth_set(c, l, next as u16, out);
                }
                if next == f.target as i32 {
                    self.fade[c][s].step = 0;
                }
            }
        }
    }

    /// The register image the host needs to continue from this state (adopted savestates): EEA, the preset, EVOL, EFFECT_ENABLE.
    /// The work area content itself is not known (starts silent).
    pub fn snapshot(&self) -> Vec<FxOp> {
        let mut v = Vec::new();
        for c in 0..2 {
            if self.mode[c] > 0 {
                if let Some(n) = self.size(self.mode[c]) {
                    v.push(op(Reg::Eea, c, self.area[c].wrapping_sub(1).wrapping_add(n)));
                    v.push(FxOp {
                        reg: Reg::FxAttr,
                        index: c as u8,
                        value: self.mode[c] as u32,
                        bank: ((self.delay[c] & 0xffff) << 16) | (self.feedback[c] & 0xffff),
                    });
                }
            }
            let evol = if self.chan[c] == 0 { self.depth[c] } else { [0, 0] };
            v.push(op(Reg::EvolL, c, evol[0] as u32));
            v.push(op(Reg::EvolR, c, evol[1] as u32));
            v.push(op(Reg::FxOn, c, (self.mode[c] > 0 && self.chan[c] == 0 && self.pending[c] == 0) as u32));
        }
        v
    }

    pub fn save(&self, o: &mut Out) {
        for c in 0..2 {
            o.i32(self.mode[c]);
            o.u32(self.area[c]);
            o.i32(self.fixed[c]);
            o.u16(self.depth[c][0]);
            o.u16(self.depth[c][1]);
            o.u32(self.chan[c]);
            o.u32(self.pending[c]);
            for f in &self.fade[c] {
                o.i16(f.step);
                o.i16(f.target);
            }
            o.u32(self.delay[c]);
            o.u32(self.feedback[c]);
        }
        for s in self.sizes {
            o.u32(s);
        }
    }

    pub fn load(i: &mut In) -> Reverb {
        let mut r = Reverb::new();
        for c in 0..2 {
            r.mode[c] = i.i32();
            r.area[c] = i.u32();
            r.fixed[c] = i.i32();
            r.depth[c] = [i.u16(), i.u16()];
            r.chan[c] = i.u32();
            r.pending[c] = i.u32();
            for f in &mut r.fade[c] {
                *f = Fade { step: i.i16(), target: i.i16() };
            }
            r.delay[c] = i.u32();
            r.feedback[c] = i.u32();
        }
        for s in &mut r.sizes {
            *s = i.u32();
        }
        r
    }
}

#[cfg(test)]
#[path = "reverb_tests.rs"]
mod tests;
