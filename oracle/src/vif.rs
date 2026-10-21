//! VIF0: the DMA chain is moved at the kick and its VIF codes are applied to
//! VU0 memory at once (MPG microcode uploads, UNPACK data). A code that would
//! start a VU0 microprogram (MSCAL/MSCNT) is reported as a fault.

use crate::mem::Mem;

#[derive(Default, Clone)]
pub struct Vif {
    cl: u32,
    wl: u32,
    mode: u32,
    mask: u32,
    row: [u32; 4],
    col: [u32; 4],
    pub mpg_uploads: u64,
    pub unpacks: u64,
}

const VIF0: u32 = 0x1000_8000;

impl Mem {
    /// VIF0 kick: gather the transfer (normal or source chain, TTE words
    /// included) and run the VIF codes in it.
    pub fn vif0_kick(&mut self) {
        let chcr = self.hw32(VIF0);
        let mode = (chcr >> 2) & 3;
        let tte = chcr & 0x40 != 0;
        let mut words: Vec<u32> = Vec::new();
        let push = |m: &mut Mem, w: &mut Vec<u32>, addr: u32, qwc: u32| {
            // [PC] (speed) Inside RAM, `rd32` of each word is a plain read of those bytes: copy them at once
            // (the per-word path and the byte copy in `vif0_unpack` took ~17% of the EE thread with skinning).
            let base = (addr & 0x01ff_fff0) as usize;
            let n = qwc as usize * 16;
            if base + n <= crate::mem::RAM {
                w.extend(m.ram[base..base + n].chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())));
                return;
            }
            for k in 0..qwc * 4 {
                w.push(m.rd32((addr & 0x01ff_fff0).wrapping_add(k * 4)));
            }
        };
        let qwc = self.hw32(VIF0 + 0x20) & 0xffff;
        let madr = self.hw32(VIF0 + 0x10);
        push(self, &mut words, madr, qwc);
        self.set_hw32(VIF0 + 0x10, madr.wrapping_add(qwc * 16));
        self.set_hw32(VIF0 + 0x20, 0);
        if mode == 1 {
            let mut tadr = self.hw32(VIF0 + 0x30);
            let mut asr = [self.hw32(VIF0 + 0x40), self.hw32(VIF0 + 0x50)];
            let mut asp = ((chcr >> 4) & 3) as usize;
            for n in 0.. {
                if n > 100_000 {
                    self.fault = Some(format!("VIF0 chain does not end (tadr {tadr:08x})"));
                    break;
                }
                let tag = self.rd64(tadr & 0x01ff_fff0);
                let q = (tag & 0xffff) as u32;
                let id = ((tag >> 28) & 7) as u32;
                let addr = ((tag >> 32) as u32) & 0x7fff_fff0;
                if tte {
                    let hi = self.rd64((tadr & 0x01ff_fff0) + 8);
                    words.push(hi as u32);
                    words.push((hi >> 32) as u32);
                }
                let (data, next, end) = match id {
                    0 => (addr, tadr + 16, true),
                    1 => (tadr + 16, tadr + 16 + q * 16, false),
                    2 => (tadr + 16, addr, false),
                    3 | 4 => (addr, tadr + 16, false),
                    5 => {
                        if asp < 2 {
                            asr[asp] = tadr + 16 + q * 16;
                            asp += 1;
                        }
                        (tadr + 16, addr, false)
                    }
                    6 => {
                        if asp > 0 {
                            asp -= 1;
                            (tadr + 16, asr[asp], false)
                        } else {
                            (tadr + 16, tadr + 16 + q * 16, true)
                        }
                    }
                    _ => (tadr + 16, tadr + 16 + q * 16, true),
                };
                push(self, &mut words, data, q);
                self.set_hw32(VIF0 + 0x10, data.wrapping_add(q * 16));
                tadr = next;
                if end || (tag >> 31) & 1 != 0 && chcr & 0x80 != 0 {
                    break;
                }
            }
            self.set_hw32(VIF0 + 0x30, tadr);
            self.set_hw32(VIF0 + 0x40, asr[0]);
            self.set_hw32(VIF0 + 0x50, asr[1]);
            self.set_hw32(VIF0, (self.hw32(VIF0) & !0x30) | (asp as u32) << 4);
        }
        self.vif0_run(&words);
    }

    fn vif0_run(&mut self, w: &[u32]) {
        let mut i = 0;
        while i < w.len() {
            let code = w[i];
            let cmd = (code >> 24) & 0x7f;
            let imm = code & 0xffff;
            let num = (code >> 16) & 0xff;
            i += 1;
            match cmd {
                0x00 | 0x07 | 0x10 | 0x11 | 0x13 => {}
                0x01 => {
                    self.vif0.cl = imm & 0xff;
                    self.vif0.wl = (imm >> 8) & 0xff;
                }
                0x04 => {} // ITOP: only read back by VU code through XITOP
                0x05 => self.vif0.mode = imm & 3,
                0x20 => {
                    self.vif0.mask = w.get(i).copied().unwrap_or(0);
                    i += 1;
                }
                0x30 | 0x31 => {
                    let mut v = [0u32; 4];
                    for (k, x) in v.iter_mut().enumerate() {
                        *x = w.get(i + k).copied().unwrap_or(0);
                    }
                    if cmd == 0x30 {
                        self.vif0.row = v;
                    } else {
                        self.vif0.col = v;
                    }
                    i += 4;
                }
                0x4a => {
                    let n = if num == 0 { 256 } else { num } as usize;
                    // [PC] (speed) The game uploads the same code again and again (Blarg: the prog9 piece at 0xc80
                    // before its runs); only a write that changes a byte drops the native code's verdict.
                    let mut changed = false;
                    for k in 0..n * 2 {
                        let a = (((imm as usize) * 8 + k * 4) & 0xfff) as usize;
                        let v = w.get(i + k).copied().unwrap_or(0).to_le_bytes();
                        changed |= self.vu0m[a..a + 4] != v;
                        self.vu0m[a..a + 4].copy_from_slice(&v);
                    }
                    self.vif0.mpg_uploads += 1;
                    if changed {
                        self.vu0m_changed();
                    }
                    i += n * 2;
                }
                0x60..=0x7f => i += self.vif0_unpack(cmd, imm, num, &w[i..]),
                _ => {
                    if self.fault.is_none() {
                        self.fault = Some(format!("VIF0 code {code:08x} not handled"));
                    }
                    return;
                }
            }
        }
    }

    /// UNPACK into VU0 data memory; returns the words consumed.
    fn vif0_unpack(&mut self, cmd: u32, imm: u32, num: u32, w: &[u32]) -> usize {
        let vn = (cmd >> 2) & 3;
        let vl = cmd & 3;
        let masked = cmd & 0x10 != 0;
        let usn = imm & 0x4000 != 0;
        let n = if num == 0 { 256 } else { num };
        let elems = vn + 1;
        let bits = if vl == 3 { 16 / elems } else { 32 >> vl };
        let bytes = (n * elems * bits).div_ceil(8);
        let words = bytes.div_ceil(4) as usize;
        // The packed input is the first `words` words (as little-endian bytes); bytes past the end read as zero.
        let wn = words.min(w.len());
        let (cl, wl) = (self.vif0.cl.max(1), self.vif0.wl.max(1));
        if wl > cl && self.fault.is_none() {
            self.fault = Some(format!("VIF0 UNPACK fill mode cl {cl} wl {wl}"));
        }
        self.vif0.unpacks += 1;
        let base = imm & 0x3ff;
        // `off` is a multiple of the element size (4, 2 or 1 bytes), so an element never spans two words.
        let rd = |off: usize, size: u32| -> u32 {
            let x = if off >> 2 < wn { w[off >> 2] } else { 0 };
            match size {
                32 => x,
                16 => {
                    let v = (x >> ((off & 2) * 8)) & 0xffff;
                    if usn { v } else { v as u16 as i16 as i32 as u32 }
                }
                _ => {
                    let v = (x >> ((off & 3) * 8)) & 0xff;
                    if usn { v } else { v as u8 as i8 as i32 as u32 }
                }
            }
        };
        let esize = (32 >> vl) as usize / 8;
        for k in 0..n {
            let mut data = [0u32; 4];
            if vl == 3 {
                let v = rd(k as usize * 2, 16) & 0xffff;
                data = [(v & 0x1f) << 3, ((v >> 5) & 0x1f) << 3, ((v >> 10) & 0x1f) << 3, ((v >> 15) & 1) << 7];
            } else {
                let off = (k * elems) as usize * esize;
                for e in 0..elems as usize {
                    data[e] = rd(off + e * esize, 32 >> vl);
                }
                match vn {
                    0 => data = [data[0]; 4],
                    1 => data = [data[0], data[1], data[0], data[1]],
                    _ => {}
                }
            }
            let cyc = k % wl;
            let addr = (base + (k / wl) * cl + cyc) & 0xff;
            let a = addr as usize * 16;
            for e in 0..4usize {
                if vn == 2 && e == 3 && vl != 3 {
                    continue;
                }
                let m = if masked { (self.vif0.mask >> (cyc.min(3) * 8 + e as u32 * 2)) & 3 } else { 0 };
                let v = match m {
                    0 => match self.vif0.mode {
                        1 => data[e].wrapping_add(self.vif0.row[e]),
                        2 => {
                            let s = data[e].wrapping_add(self.vif0.row[e]);
                            self.vif0.row[e] = s;
                            s
                        }
                        _ => data[e],
                    },
                    1 => self.vif0.row[e],
                    2 => self.vif0.col[cyc.min(3) as usize],
                    _ => continue,
                };
                self.vu0d[a + e * 4..a + e * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        words
    }
}

impl Vif {
    /// fndiff checkpoint files.
    pub fn save(&self, w: &mut crate::snapfile::Wr) {
        for v in [self.cl, self.wl, self.mode, self.mask] {
            w.u32(v);
        }
        for v in self.row.iter().chain(self.col.iter()) {
            w.u32(*v);
        }
        w.u64(self.mpg_uploads);
        w.u64(self.unpacks);
    }

    pub fn load(&mut self, r: &mut crate::snapfile::Rd) {
        (self.cl, self.wl, self.mode, self.mask) = (r.u32(), r.u32(), r.u32(), r.u32());
        for v in self.row.iter_mut().chain(self.col.iter_mut()) {
            *v = r.u32();
        }
        self.mpg_uploads = r.u64();
        self.unpacks = r.u64();
    }
}
