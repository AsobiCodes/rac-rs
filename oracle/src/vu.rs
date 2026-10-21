//! VU0 micro mode (VCALLMS / VCALLMSR). Runs the program at call time to its E bit, or to a wait loop that only the
//! EE can end (then it is parked and goes on at the EE's next CTC2 to a VI register; see `vu0_exec`).
//! Pipeline model (needed by the game's code, e.g. FMOR reading the MAC flag
//! of the op 4 pairs back): VF results ready 4 cycles after issue with
//! per-lane stalls, Q after 7 (DIV/SQRT) / 13 (RSQRT) cycles, MAC/status/clip
//! flags visible 4 cycles after issue. Upper and lower read the pre-pair state;
//! upper wins a same-register write; LOI's I is seen from the next pair.

use crate::cpu::Cpu;
use ee_core::vu::{lane, Up, ACC, NONE};
use crate::fpu::{self, CSR_CHOP};


/// A VU0 microprogram that waits for the EE (a wait loop on VI registers, see `vu0_exec`) is parked in VPU_STAT
/// (vi29), as on the hardware: bit 0 is the busy flag, bits 16..27 the micro address it waits at. A parked program
/// has let all its pending results land (time passes while it waits), so nothing else needs saving, and every copy
/// of the VI registers (`Snap`, fndiff's register snapshots) carries it with them. A program that runs to its E bit
/// leaves VPU_STAT at 0.
const VPU_BUSY: u32 = 1;

impl Cpu {
    // The upper ops live in ee-core (vu.rs there), where the generated code inlines them.
    #[inline(always)] pub fn vu_upper(&self, op: u32, clip_prev: u32) -> Result<Up, String> { self.core().vu_upper(op, clip_prev) }
    #[inline(always)] pub fn vu_upper_g<const FAST: bool>(&self, op: u32, clip_prev: u32) -> Result<Up, String> { self.core().vu_upper_g::<FAST>(op, clip_prev) }


    /// VCALLMS / VCALLMSR: starts the microprogram at `start` and runs it to its E bit, or to the point where it
    /// waits for the EE (a wait loop on VI registers; see `vu0_exec`). In the second case the program stays
    /// parked in VPU_STAT and goes on when the EE changes a VI register (`vu0_resume`, from CTC2).
    pub fn vu0_micro(&mut self, start: u32) -> Result<u64, String> {
        if self.vu0_parked() {
            // COP2 macro ops wait for a running microprogram: it gets its chance to end first.
            self.vu0_resume()?;
            if self.vu0_parked() {
                return Err("VU0 microprogram started while the previous one still waits for the EE".into());
            }
        }
        self.vu0_stats[0] += 1;
        if let Some(t) = self.vu0_trace.as_mut() {
            t.push((2, start, self.vi[3] | self.vi[4] << 16, self.vi[5]));
        }
        // Is prog0 in micro memory? (then the native translation runs it, see vu0aot.rs)
        self.vu0_aot_check();
        self.vu0_exec((start & 0xff8) as usize)
    }

    /// True while a microprogram waits for the EE (VPU_STAT busy).
    pub fn vu0_parked(&self) -> bool {
        self.vi[29] & VPU_BUSY != 0
    }

    /// The EE changed a VI register (CTC2): a suspended microprogram re-tests the condition it waits on.
    pub fn vu0_resume(&mut self) -> Result<u64, String> {
        if !self.vu0_parked() {
            return Ok(0);
        }
        let pc = ((self.vi[29] >> 16) & 0xff8) as usize;
        self.vi[29] = 0;
        self.vu0_stats[2] += 1;
        self.vu0_exec(pc)
    }

    /// True when the pairs t ..= pc + 8 are a pure wait loop: upper NOP, lower NOP or branch (B, IBEQ, ..), no
    /// other side effect. `pc` holds the branch that jumps back to `t`.
    fn vu0_is_spin(&self, t: usize, pc: usize) -> bool {
        let end = pc + 8;
        if end - t > 64 {
            return false;
        }
        let rdm = |a: usize| u32::from_le_bytes(self.mem.vu0m[a..a + 4].try_into().unwrap());
        let mut a = t;
        while a <= end {
            let (lo, hi) = (rdm(a), rdm(a + 4));
            if hi & 0xf800_0000 != 0 || hi & 0x07ff_ffff != 0x0000_02ff {
                return false;
            }
            let op = lo >> 25;
            if !(lo == 0x8000_033c || op == 0x20 || (0x28..=0x2f).contains(&op)) {
                return false;
            }
            a += 8;
        }
        true
    }

    /// Runs `run` until the E bit (the run ends) or a wait loop that only the EE can end (the run is parked in
    /// VPU_STAT with its pc at the loop head). Returns the cycles run.
    pub(crate) fn vu0_exec_interp(&mut self, mut pc: usize) -> Result<u64, String> {
        // Starting or resuming: nothing is in flight (a parked program let everything land), the clip flag
        // register holds the newest clip result.
        let (mut cyc, mut clip_latest) = (0u64, self.vi[18]);
        // MXCSR stays at round-toward-zero / DAZ / FTZ for the whole run instead of switching for every float op.
        let _chop = fpu::ChopGuard::new();
        let mut ready = [[0u64; 4]; 33];
        let mut q_pend: Option<(u64, u32)> = None;
        // Results in flight, (ready cycle, kind 0 = MAC+status / 1 = clip, value, status bits), oldest first. One op
        // issues per cycle and a result lands 4 cycles later, so 8 slots never overflow.
        let mut fl = [(0u64, 0u8, 0u32, 0u32); 8];
        let (mut fh, mut fnn) = (0usize, 0usize);
        let mut taken: Option<usize> = None;
        let mut end_next = false;
        let mut parked = false;
        let mut capped = true;
        let mut pairs = 0u64;
        let rdm = |m: &Vec<u8>, a: usize| u32::from_le_bytes(m[a..a + 4].try_into().unwrap());
        for _ in 0..5_000_000u32 {
            pairs += 1;
            let lo = rdm(&self.mem.vu0m, pc);
            let hi = rdm(&self.mem.vu0m, pc + 4);
            let ibit = hi & 0x8000_0000 != 0;
            let mut up = self.vu_upper_g::<true>(hi & 0x07ff_ffff, clip_latest).map_err(|e| format!("{e} at vu0 {pc:04x}"))?;
            let lr = if ibit { LowerInfo::default() } else { lower_info(lo) };
            // Stalls.
            let mut need = cyc;
            for &(r, m) in up.reads.iter().chain(lr.reads.iter()) {
                if r != NONE {
                    let rr = &ready[r];
                    // lane(l) = 8 >> l: bit 3 is x.
                    if m & 8 != 0 {
                        need = need.max(rr[0]);
                    }
                    if m & 4 != 0 {
                        need = need.max(rr[1]);
                    }
                    if m & 2 != 0 {
                        need = need.max(rr[2]);
                    }
                    if m & 1 != 0 {
                        need = need.max(rr[3]);
                    }
                }
            }
            if lr.waitq || lr.div {
                if let Some((r, _)) = q_pend {
                    need = need.max(r);
                }
            }
            cyc = need;
            if let Some((r, v)) = q_pend {
                if cyc >= r {
                    self.vi[22] = v;
                    q_pend = None;
                    // Q landed: the upper op reads the new Q (flags landing change nothing it reads).
                    up = self.vu_upper_g::<true>(hi & 0x07ff_ffff, clip_latest).map_err(|e| format!("{e} at vu0 {pc:04x}"))?;
                }
            }
            while fnn > 0 {
                let (r, k, v, st) = fl[fh & 7];
                if r > cyc {
                    break;
                }
                self.vu0_flag_lands(k, v, st);
                fh += 1;
                fnn -= 1;
            }
            let mut branch = None;
            // (The common NOP lower, MOVE vf0, vf0, has no effect.)
            if !ibit && lo != 0x8000_033c {
                branch = self.vu_lower(lo, pc, cyc, &mut ready, &mut q_pend).map_err(|e| format!("{e} at vu0 {pc:04x}"))?;
            }
            // A taken backward branch over a pure wait loop: only the EE (CTC2 to a VI register) can end it.
            // Time passes, the pending results land, and the program waits at the loop head.
            if let Some(t) = branch {
                if t <= pc && taken.is_none() && !end_next && self.vu0_is_spin(t, pc) {
                    pc = t;
                    parked = true;
                    capped = false;
                    break;
                }
            }
            // Commit upper.
            if up.dst != NONE && (up.dst != 0) {
                for l in 0..4 {
                    if up.mask & lane(l) != 0 {
                        if up.dst == ACC {
                            self.vacc[l] = up.val[l];
                        } else {
                            self.vf[up.dst][l] = up.val[l];
                        }
                        ready[up.dst][l] = cyc + 4;
                    }
                }
            }
            let mut push = None;
            if let Some(m) = up.mac {
                let st = ((m & 0xf != 0) as u32) | (((m & 0xf0 != 0) as u32) << 1);
                push = Some((cyc + 4, 0u8, m, st));
            }
            if let Some(c) = up.clip {
                clip_latest = c;
                push = Some((cyc + 4, 1u8, c, 0));
            }
            if let Some(e) = push {
                if fnn == 8 {
                    let (_, k, v, st) = fl[fh & 7];
                    self.vu0_flag_lands(k, v, st);
                    fh += 1;
                    fnn -= 1;
                }
                fl[(fh + fnn) & 7] = e;
                fnn += 1;
            }
            if ibit {
                self.vi[21] = lo;
            }
            cyc += 1;
            if end_next {
                capped = false;
                break;
            }
            if hi & 0x4000_0000 != 0 {
                end_next = true;
            }
            let next = taken.take().unwrap_or(pc + 8);
            taken = branch;
            pc = next & 0xff8;
        }
        self.vu0_stats[3] += pairs;
        self.vu0_stats[4] = pc as u64;
        if let Some(t) = self.vu0_trace.as_mut() {
            t.push((if parked { 3 } else { 4 }, pc as u32, pairs as u32, cyc as u32));
        }
        self.vu0_stats[5] = if parked { 1 } else if capped { 2 } else { 0 };
        if capped {
            return Err(format!("VU0 microprogram ran 5000000 pairs without ending or waiting (pc {pc:04x})"));
        }
        // Whatever is still in flight lands (the program ended, or waits for the EE and time passes).
        if let Some((_, v)) = q_pend {
            self.vi[22] = v;
        }
        while fnn > 0 {
            let (_, k, v, st) = fl[fh & 7];
            self.vu0_flag_lands(k, v, st);
            fh += 1;
            fnn -= 1;
        }
        if parked {
            self.vu0_stats[1] += 1;
            self.vi[29] = VPU_BUSY | (pc as u32) << 16;
        }
        Ok(cyc)
    }

    /// A MAC+status (kind 0) or clip (kind 1) result reaches its register.
    #[inline(always)]
    fn vu0_flag_lands(&mut self, k: u8, v: u32, st: u32) {
        if k == 0 {
            self.vi[17] = v;
            self.vi[16] = (self.vi[16] & !0xf) | st | (st << 6);
        } else {
            self.vi[18] = v;
        }
    }

    fn vu_lower(
        &mut self,
        op: u32,
        pc: usize,
        cyc: u64,
        ready: &mut [[u64; 4]; 33],
        q_pend: &mut Option<(u64, u32)>,
    ) -> Result<Option<usize>, String> {
        let dest = (op >> 21) & 15;
        let it = ((op >> 16) & 31) as usize;
        let is = ((op >> 11) & 31) as usize;
        let id = ((op >> 6) & 31) as usize;
        let imm11 = (((op & 0x7ff) as i32) << 21 >> 21) as u32;
        let target = || ((pc as i64 + 8 + (imm11 as i32 as i64) * 8) as usize) & 0xff8;
        let vi = |c: &Cpu, r: usize| c.vi[r] & 0xffff;
        let qaddr = |c: &Cpu, r: usize, off: u32| ((c.vi[r].wrapping_add(off) & 0xff) * 16) as usize;
        let wvf = |c: &mut Cpu, ready: &mut [[u64; 4]; 33], r: usize, v: [u32; 4], m: u32| {
            if r == 0 {
                return;
            }
            for l in 0..4 {
                if m & lane(l) != 0 {
                    c.vf[r][l] = v[l];
                    ready[r][l] = cyc + 4;
                }
            }
        };
        let rdq = |c: &Cpu, a: usize| {
            let m = &c.mem.vu0d;
            [0, 4, 8, 12].map(|o| u32::from_le_bytes(m[a + o..a + o + 4].try_into().unwrap()))
        };
        let wrw = |c: &mut Cpu, a: usize, v: u32| c.mem.vu0d[a..a + 4].copy_from_slice(&v.to_le_bytes());
        let flag = |c: &Cpu, k: usize| c.vi[k];
        let opc = op >> 25;
        match opc {
            0x00 => {
                let a = qaddr(self, is, imm11);
                let v = rdq(self, a);
                wvf(self, ready, it, v, dest);
            }
            0x01 => {
                let a = qaddr(self, it, imm11);
                let s = self.vf[is];
                for l in 0..4 {
                    if dest & lane(l) != 0 {
                        wrw(self, a + l * 4, s[l]);
                    }
                }
            }
            0x04 => {
                let a = qaddr(self, is, imm11);
                let l = (0..4).find(|&l| dest & lane(l) != 0).unwrap_or(0);
                let v = rdq(self, a)[l] & 0xffff;
                self.set_vi(it, v);
            }
            0x05 => {
                let a = qaddr(self, is, imm11);
                let v = vi(self, it);
                for l in 0..4 {
                    if dest & lane(l) != 0 {
                        wrw(self, a + l * 4, v);
                    }
                }
            }
            0x08 | 0x09 => {
                let im = (((op >> 10) & 0x7800) | (op & 0x7ff)) as u32;
                let v = if opc == 8 { self.vi[is].wrapping_add(im) } else { self.vi[is].wrapping_sub(im) };
                self.set_vi(it, v);
            }
            0x10..=0x13 => {
                let im = op & 0xff_ffff;
                let c = flag(self, 18) & 0xff_ffff;
                match opc {
                    0x10 => self.vi[1] = (c == im) as u32,
                    0x11 => {
                        self.vi[18] = im;
                    }
                    0x12 => self.vi[1] = (c & im != 0) as u32,
                    _ => self.vi[1] = ((c | im) & 0xff_ffff == 0xff_ffff) as u32,
                }
            }
            0x14..=0x17 => {
                let im = (((op >> 10) & 0x800) | (op & 0x7ff)) as u32;
                let s = flag(self, 16) & 0xfff;
                match opc {
                    0x14 => self.set_vi(it, (s == im) as u32),
                    0x15 => self.vi[16] = (self.vi[16] & 0x3f) | (im & 0xfc0),
                    0x16 => self.set_vi(it, s & im),
                    _ => self.set_vi(it, ((s | im) == 0xfff) as u32),
                }
            }
            0x18 => self.set_vi(it, ((flag(self, 17) & 0xffff) == vi(self, is)) as u32),
            0x1a => self.set_vi(it, flag(self, 17) & vi(self, is)),
            0x1b => self.set_vi(it, flag(self, 17) | vi(self, is)),
            0x1c => self.set_vi(it, flag(self, 18) & 0xfff),
            0x20 => return Ok(Some(target())),
            0x21 => {
                self.set_vi(it, ((pc + 16) / 8) as u32);
                return Ok(Some(target()));
            }
            0x24 => return Ok(Some(((vi(self, is) as usize) * 8) & 0xff8)),
            0x25 => {
                let t = ((vi(self, is) as usize) * 8) & 0xff8;
                self.set_vi(it, ((pc + 16) / 8) as u32);
                return Ok(Some(t));
            }
            0x28..=0x2f => {
                let a = vi(self, is) as u16 as i16;
                let b = vi(self, it) as u16 as i16;
                let c = match opc {
                    0x28 => a == b,
                    0x29 => a != b,
                    0x2c => a < 0,
                    0x2d => a > 0,
                    0x2e => a <= 0,
                    0x2f => a >= 0,
                    _ => return Err(format!("unknown VU lower {op:08x}")),
                };
                return Ok(if c { Some(target()) } else { None });
            }
            0x40 => {
                let f = op & 0x3f;
                match f {
                    0x30 => self.set_vi(id, self.vi[is].wrapping_add(self.vi[it])),
                    0x31 => self.set_vi(id, self.vi[is].wrapping_sub(self.vi[it])),
                    0x32 => {
                        let im = ((((op >> 6) & 31) as i32) << 27 >> 27) as u32;
                        self.set_vi(it, self.vi[is].wrapping_add(im));
                    }
                    0x34 => self.set_vi(id, self.vi[is] & self.vi[it]),
                    0x35 => self.set_vi(id, self.vi[is] | self.vi[it]),
                    0x3c..=0x3f => {
                        let x = (op & 3) | ((op >> 4) & 0x7c);
                        let fs = is;
                        let ft = it;
                        let s = self.vf[fs];
                        let fsf = ((op >> 21) & 3) as usize;
                        let ftf = ((op >> 23) & 3) as usize;
                        match x {
                            0x30 => wvf(self, ready, ft, s, dest),
                            0x31 => wvf(self, ready, ft, [s[1], s[2], s[3], s[0]], dest),
                            0x34 | 0x36 => {
                                if x == 0x36 {
                                    self.set_vi(is, self.vi[is].wrapping_sub(1));
                                }
                                let a = qaddr(self, is, 0);
                                let v = rdq(self, a);
                                wvf(self, ready, ft, v, dest);
                                if x == 0x34 {
                                    self.set_vi(is, self.vi[is].wrapping_add(1));
                                }
                            }
                            0x35 | 0x37 => {
                                if x == 0x37 {
                                    self.set_vi(it, self.vi[it].wrapping_sub(1));
                                }
                                let a = qaddr(self, it, 0);
                                for l in 0..4 {
                                    if dest & lane(l) != 0 {
                                        wrw(self, a + l * 4, s[l]);
                                    }
                                }
                                if x == 0x35 {
                                    self.set_vi(it, self.vi[it].wrapping_add(1));
                                }
                            }
                            0x38..=0x3a => {
                                let t = self.vf[ft][ftf];
                                let (v, lat) = match x {
                                    0x38 => (fpu::div(s[fsf], t, CSR_CHOP), 7),
                                    0x39 => (fpu::sqrt(t, CSR_CHOP), 7),
                                    _ => (fpu::rsqrt(s[fsf], t, CSR_CHOP), 13),
                                };
                                *q_pend = Some((cyc + lat, v));
                            }
                            0x3b => {}
                            0x3c => self.set_vi(ft, s[fsf] & 0xffff),
                            0x3d => {
                                let v = self.vi[is] as u16 as i16 as i32 as u32;
                                wvf(self, ready, ft, [v; 4], dest);
                            }
                            0x3e => {
                                let a = qaddr(self, is, 0);
                                let l = (0..4).find(|&l| dest & lane(l) != 0).unwrap_or(0);
                                let v = rdq(self, a)[l] & 0xffff;
                                self.set_vi(it, v);
                            }
                            0x3f => {
                                let a = qaddr(self, is, 0);
                                let v = vi(self, it);
                                for l in 0..4 {
                                    if dest & lane(l) != 0 {
                                        wrw(self, a + l * 4, v);
                                    }
                                }
                            }
                            0x40 | 0x41 => {
                                if x == 0x40 {
                                    let r = self.vi[20];
                                    let b = ((r >> 4) & 1) ^ ((r >> 22) & 1);
                                    self.vi[20] = (((r << 1) ^ b) & 0x7f_ffff) | 0x3f80_0000;
                                }
                                let r = self.vi[20];
                                wvf(self, ready, ft, [r; 4], dest);
                            }
                            0x42 => self.vi[20] = 0x3f80_0000 | (s[fsf] & 0x7f_ffff),
                            0x43 => self.vi[20] = 0x3f80_0000 | ((self.vi[20] ^ s[fsf]) & 0x7f_ffff),
                            _ => return Err(format!("unknown VU lower {op:08x}")),
                        }
                    }
                    _ => return Err(format!("unknown VU lower {op:08x}")),
                }
            }
            _ => return Err(format!("unknown VU lower {op:08x}")),
        }
        Ok(None)
    }
}

#[derive(Default)]
struct LowerInfo {
    reads: [(usize, u32); 2],
    waitq: bool,
    div: bool,
}

/// Source VF fields of a lower op, for stall timing.
fn lower_info(op: u32) -> LowerInfo {
    let mut li = LowerInfo { reads: [(NONE, 0); 2], waitq: false, div: false };
    let dest = (op >> 21) & 15;
    let ft = ((op >> 16) & 31) as usize;
    let fs = ((op >> 11) & 31) as usize;
    let fsf = ((op >> 21) & 3) as usize;
    let ftf = ((op >> 23) & 3) as usize;
    match op >> 25 {
        0x01 => li.reads[0] = (fs, dest),
        0x40 if op & 0x3c == 0x3c => {
            let x = (op & 3) | ((op >> 4) & 0x7c);
            match x {
                0x30 | 0x35 | 0x37 => li.reads[0] = (fs, dest),
                0x31 => {
                    let mut m = 0;
                    for l in 0..4 {
                        if dest & lane(l) != 0 {
                            m |= lane((l + 1) & 3);
                        }
                    }
                    li.reads[0] = (fs, m);
                }
                0x38 | 0x3a => {
                    li.reads = [(fs, lane(fsf)), (ft, lane(ftf))];
                    li.div = true;
                }
                0x39 => {
                    li.reads[0] = (ft, lane(ftf));
                    li.div = true;
                }
                0x3b => li.waitq = true,
                0x3c | 0x42 | 0x43 => li.reads[0] = (fs, lane(fsf)),
                _ => {}
            }
        }
        _ => {}
    }
    li
}
