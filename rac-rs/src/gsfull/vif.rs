//! VIF1: the word stream of a DMA chain (UNPACK into VU1 data memory, MPG into VU1 micro memory, MSCAL / MSCNT, DIRECT)
//! and the GIF output it produces. VU1 runs synchronously at MSCAL / MSCNT; its XGKICK packets (PATH1) and the DIRECT
//! data (PATH2) land in `Out` in program order.
//!
//! Port of the GS-full prototype (<dev>/scratch_gsfull/gsfull/src/vif.rs) without its debug hooks, driving
//! `super::vu1::Vu1` (AOT dispatch with interpreter fallback). Kept from the prototype on purpose:
//! - UNPACK padding: an UNPACK that was issued from a DMA tag's VIF code (second word of a phase-2 block) and whose data
//!   ends inside the block's last qword leaves 1-3 padding words, which are skipped. The oracle's own `gs.rs` `vif_word`
//!   skips padding at the wrong time; do not copy that.
//! - the branch hazard lives in `Vu1` (a conditional branch reads its integer register one cycle early).
use super::vu1::{Path1, Vu1};

/// The GS-bound qwords of one VIF1 job with where each run of them came from.
#[derive(Default)]
pub struct Out {
    pub qs: Vec<u128>,
    /// (first qword, path 1/2/3, VU1 program id)
    pub segs: Vec<(u32, u8, u8)>,
    /// The VU1 program the next run belongs to (`identify` on the chunk MPG-uploaded at address 0; 15 = unknown).
    pub prog: u8,
    pub stats: Stats,
}

#[derive(Default, Clone)]
pub struct Stats {
    pub vif_words: u64,
    pub mscal: u64,
    pub mscnt: u64,
    pub mpg: u64,
    pub unpack: u64,
    pub unpack_words: u64,
    pub unpack_fill: u64,
    pub direct_qw: u64,
    pub path1_qw: u64,
    pub path1_pkts: u64,
    pub bad_codes: u64,
    pub bad_xgkick: u64,
    pub run_errs: u64,
    pub codes: u64,
    pub prog_runs: [u64; 16],
    pub prog_pairs: [u64; 16],
    pub prog_cycles: [u64; 16],
    pub prog_qw: [u64; 16],
}

/// (FNV-1a of the first 16 words of a program's first chunk, program id): retail VU1 programs 0..11 (decomp/vu1/progN.bin).
/// Programs 3 and 4 start with the same 16 words; as in the prototype the first entry wins, so both read as 3.
const PROGS: [(u64, u8); 12] = [
    (0xac94c7d56d963410, 0),
    (0x64b9278f738ff6bb, 1),
    (0xbe7f3096b7a8caed, 2),
    (0xefdae0d82adfbcd6, 3),
    (0xefdae0d82adfbcd6, 4),
    (0x4e329bc8a744a9c9, 5),
    (0xd28121151fdf0150, 6),
    (0x5908a5930badba6e, 7),
    (0xed92117ff6e7476a, 8),
    (0x5fd86134df75989d, 9),
    (0xcfcb763ca14d3b55, 10),
    (0x68a07c118d24d2ca, 11),
];

/// Program id of a chunk uploaded at address 0 (15 = unknown). Only tags the output (`Out::segs`, `Stats`); VU1 itself
/// identifies the microprogram by content (`vu1aot`).
pub fn identify(w: &[u32]) -> u8 {
    let h = w.iter().take(16).fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x0100_0000_01b3));
    PROGS.iter().find(|p| p.0 == h).map(|p| p.1).unwrap_or(15)
}

impl Out {
    pub fn seg(&mut self, path: u8) {
        let n = self.qs.len() as u32;
        match self.segs.last() {
            Some(&(s, p, g)) if p == path && g == self.prog && s <= n => {}
            _ => self.segs.push((n, path, self.prog)),
        }
    }
}

impl Path1 for Out {
    fn xgkick(&mut self, data: &[[u32; 4]], addr: usize) {
        let mut a = addr & 0x3ff;
        self.seg(1);
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 4096 {
                break;
            }
            let t = data[a];
            let nloop = (t[0] & 0x7fff) as usize;
            let eop = t[0] >> 15 & 1 != 0;
            let flg = (t[1] >> 26) & 3;
            let nreg = ((t[1] >> 28) & 15) as usize;
            let nreg = if nreg == 0 { 16 } else { nreg };
            let len = match flg {
                0 => nloop * nreg,
                1 => (nloop * nreg + 1) / 2,
                _ => nloop,
            };
            if len + 1 > 1024 {
                self.stats.bad_xgkick += 1;
                break;
            }
            for k in 0..=len {
                let q = data[(a + k) & 0x3ff];
                self.qs.push(q[0] as u128 | (q[1] as u128) << 32 | (q[2] as u128) << 64 | (q[3] as u128) << 96);
            }
            self.stats.path1_qw += len as u64 + 1;
            self.stats.prog_qw[(self.prog & 15) as usize] += len as u64 + 1;
            a = (a + len + 1) & 0x3ff;
            if eop {
                break;
            }
        }
        self.stats.path1_pkts += 1;
    }
}

enum St {
    Idle,
    /// Collecting `left` more words for STMASK / STROW / STCOL (what = code).
    Collect { what: u32, left: usize },
    Mpg { addr: usize, left: usize },
    Unpack { cmd: u32, imm: u32, num: u32, left: usize },
    Direct { left: usize },
    Pad(usize),
}

pub struct Vif1 {
    cl: u32,
    wl: u32,
    mode: u32,
    mask: u32,
    row: [u32; 4],
    col: [u32; 4],
    base: u32,
    offset: u32,
    tops: u32,
    itops: u32,
    dbf: bool,
    st: St,
    buf: Vec<u32>,
    qn: usize,
    qw: [u32; 4],
    bad: bool,
    cur_tag2: bool,
    unpack_from_tag: bool,
    /// Shadow mode (the effects layer's VU1 model, retail/efx_vu.rs): MPG / UNPACK / VIF registers are applied, but
    /// MSCAL / MSCALF / MSCNT do not run VU1 (only TOPS / DBF advance) and DIRECT data is not collected.
    pub shadow: bool,
}

impl Default for Vif1 {
    fn default() -> Self {
        Vif1::new()
    }
}

impl Vif1 {
    pub fn new() -> Vif1 {
        Vif1 {
            cl: 1,
            wl: 1,
            mode: 0,
            mask: 0,
            row: [0; 4],
            col: [0; 4],
            base: 0,
            offset: 0,
            tops: 0,
            itops: 0,
            dbf: false,
            st: St::Idle,
            buf: Vec::new(),
            qn: 0,
            qw: [0; 4],
            bad: false,
            cur_tag2: false,
            unpack_from_tag: false,
            shadow: false,
        }
    }

    /// Runs one `Job::Vif1`: `w` the word stream, `marks` the (index, phase) of each DMA block start.
    pub fn feed(&mut self, w: &[u32], marks: &[(usize, u32)], vu: &mut Vu1, out: &mut Out) {
        out.stats.vif_words += w.len() as u64;
        if self.bad {
            self.bad = false;
            self.st = St::Idle;
        }
        for (k, &(at, ph)) in marks.iter().enumerate() {
            let end = marks.get(k + 1).map(|m| m.0).unwrap_or(w.len()).min(w.len());
            let mut i = at;
            while i < end {
                match &mut self.st {
                    St::Idle => {
                        let code = w[i];
                        self.cur_tag2 = ph == 2 && i == at + 1;
                        i += 1;
                        self.code(code, vu, out);
                    }
                    St::Pad(n) => {
                        let take = (*n).min(end - i);
                        *n -= take;
                        i += take;
                        if *n == 0 {
                            self.st = St::Idle;
                        }
                    }
                    St::Direct { left } => {
                        let take = (*left).min(end - i);
                        *left -= take;
                        let done = *left == 0;
                        if self.shadow {
                            i += take;
                            if done {
                                self.st = St::Idle;
                            }
                            continue;
                        }
                        out.seg(2);
                        for &x in &w[i..i + take] {
                            self.qw[self.qn] = x;
                            self.qn += 1;
                            if self.qn == 4 {
                                self.qn = 0;
                                let q = self.qw;
                                out.qs.push(q[0] as u128 | (q[1] as u128) << 32 | (q[2] as u128) << 64 | (q[3] as u128) << 96);
                                out.stats.direct_qw += 1;
                            }
                        }
                        i += take;
                        if done {
                            self.st = St::Idle;
                        }
                    }
                    St::Collect { left, .. } | St::Mpg { left, .. } | St::Unpack { left, .. } => {
                        let take = (*left).min(end - i);
                        *left -= take;
                        self.buf.extend_from_slice(&w[i..i + take]);
                        let last = i + take - 1;
                        i += take;
                        let done = matches!(&self.st, St::Collect { left: 0, .. } | St::Mpg { left: 0, .. } | St::Unpack { left: 0, .. });
                        if done {
                            let st = std::mem::replace(&mut self.st, St::Idle);
                            let rem = end - i;
                            let pad = 3 - ((ph as usize + (last - at)) & 3);
                            let was_unpack = matches!(st, St::Unpack { .. });
                            self.finish(st, vu, out);
                            // Unpack data that ends inside the block's last qword: the rest is padding (tag UNPACKs only).
                            if was_unpack && self.unpack_from_tag && rem < 4 && rem == pad && rem > 0 {
                                self.st = St::Pad(rem);
                            }
                        }
                    }
                }
            }
        }
    }

    fn code(&mut self, code: u32, vu: &mut Vu1, out: &mut Out) {
        out.stats.codes += 1;
        let cmd = (code >> 24) & 0x7f;
        let imm = code & 0xffff;
        let num = (code >> 16) & 0xff;
        match cmd {
            0x00 | 0x06 | 0x07 | 0x10 | 0x11 | 0x13 => {}
            0x01 => {
                self.cl = imm & 0xff;
                self.wl = (imm >> 8) & 0xff;
            }
            0x02 => {
                self.offset = imm & 0x3ff;
                self.dbf = false;
                self.tops = self.base;
            }
            0x03 => self.base = imm & 0x3ff,
            0x04 => self.itops = imm & 0x3ff,
            0x05 => self.mode = imm & 3,
            0x14 | 0x15 => {
                out.stats.mscal += 1;
                self.exec(imm as usize * 8, vu, out);
            }
            0x17 => {
                out.stats.mscnt += 1;
                let a = vu.tpc;
                self.exec(a, vu, out);
            }
            0x20 => {
                self.st = St::Collect { what: 0x20, left: 1 };
                self.buf.clear();
            }
            0x30 | 0x31 => {
                self.st = St::Collect { what: cmd, left: 4 };
                self.buf.clear();
            }
            0x4a => {
                let n = if num == 0 { 256 } else { num as usize };
                self.st = St::Mpg { addr: imm as usize * 8, left: n * 2 };
                self.buf.clear();
            }
            0x50 | 0x51 => {
                let n = if imm == 0 { 65536 } else { imm as usize };
                self.st = St::Direct { left: n * 4 };
                self.qn = 0;
            }
            0x60..=0x7f => {
                let vn = (cmd >> 2) & 3;
                let vl = cmd & 3;
                let n = if num == 0 { 256 } else { num };
                let (cl, wl) = (self.cl, self.wl);
                let items = if wl <= cl || wl == 0 { n } else { (n / wl) * cl + (n % wl).min(cl) };
                let bits = if vl == 3 { 16 } else { (vn + 1) * (32 >> vl) };
                let words = (items * bits).div_ceil(32) as usize;
                if words > 0 {
                    self.unpack_from_tag = self.cur_tag2;
                    self.st = St::Unpack { cmd, imm, num: n, left: words };
                    self.buf.clear();
                }
                out.stats.unpack += 1;
                out.stats.unpack_words += words as u64;
            }
            _ => {
                out.stats.bad_codes += 1;
                self.bad = true;
                self.st = St::Idle;
            }
        }
    }

    fn exec(&mut self, addr: usize, vu: &mut Vu1, out: &mut Out) {
        let top = self.tops;
        let itop = self.itops;
        if self.dbf {
            self.tops = self.base;
        } else {
            self.tops = (self.base + self.offset) & 0x3ff;
        }
        self.dbf = !self.dbf;
        if self.shadow {
            return;
        }
        let (p0, c0) = (vu.st.pairs, vu.st.cycles);
        let id = out.prog;
        // (a run that errors leaves the packets it already sent; the field goes on)
        if vu.run(addr, top, itop, out).is_err() {
            out.stats.run_errs += 1;
        }
        let k = (id & 15) as usize;
        out.stats.prog_runs[k] += 1;
        out.stats.prog_pairs[k] += vu.st.pairs - p0;
        out.stats.prog_cycles[k] += vu.st.cycles - c0;
    }

    fn finish(&mut self, st: St, vu: &mut Vu1, out: &mut Out) {
        match st {
            St::Collect { what, .. } => match what {
                0x20 => self.mask = self.buf[0],
                0x30 => self.row.copy_from_slice(&self.buf[..4]),
                _ => self.col.copy_from_slice(&self.buf[..4]),
            },
            St::Mpg { addr, .. } => {
                out.stats.mpg += 1;
                if addr == 0 {
                    out.prog = identify(&self.buf);
                }
                let pairs: Vec<u64> = self.buf.chunks_exact(2).map(|p| p[0] as u64 | (p[1] as u64) << 32).collect();
                vu.mpg(addr >> 3, &pairs);
            }
            St::Unpack { cmd, imm, num, .. } => self.unpack(cmd, imm, num, vu, out),
            _ => {}
        }
    }

    fn unpack(&mut self, cmd: u32, imm: u32, num: u32, vu: &mut Vu1, out: &mut Out) {
        let vn = (cmd >> 2) & 3;
        let vl = cmd & 3;
        let masked = cmd & 0x10 != 0;
        let usn = imm & 0x4000 != 0;
        let flg = imm & 0x8000 != 0;
        let elems = vn as usize + 1;
        let (cl, wl) = (self.cl.max(1), self.wl.max(1));
        let base = ((imm & 0x3ff) + if flg { self.tops } else { 0 }) as usize;
        let src = &self.buf;
        let rd = |k: usize, e: usize| -> u32 {
            // element e of source vector k
            match vl {
                0 => src.get(k * elems + e).copied().unwrap_or(0),
                1 => {
                    let i = k * elems + e;
                    let w = src.get(i / 2).copied().unwrap_or(0);
                    let v = (w >> ((i & 1) * 16)) & 0xffff;
                    if usn { v } else { v as u16 as i16 as i32 as u32 }
                }
                2 => {
                    let i = k * elems + e;
                    let w = src.get(i / 4).copied().unwrap_or(0);
                    let v = (w >> ((i & 3) * 8)) & 0xff;
                    if usn { v } else { v as u8 as i8 as i32 as u32 }
                }
                _ => 0,
            }
        };
        let mut sk = 0usize; // source vector index
        for k in 0..num {
            let cyc = k % wl;
            let addr = (base + ((k / wl) * cl + cyc) as usize) & 0x3ff;
            let fill = wl > cl && cyc >= cl;
            let mut data = [0u32; 4];
            if !fill {
                if vl == 3 {
                    let w = src.get(sk / 2).copied().unwrap_or(0);
                    let v = (w >> ((sk & 1) * 16)) & 0xffff;
                    data = [(v & 0x1f) << 3, ((v >> 5) & 0x1f) << 3, ((v >> 10) & 0x1f) << 3, ((v >> 15) & 1) << 7];
                } else {
                    for e in 0..elems {
                        data[e] = rd(sk, e);
                    }
                    match vn {
                        0 => data = [data[0]; 4],
                        1 => data = [data[0], data[1], data[0], data[1]],
                        _ => {}
                    }
                }
                sk += 1;
            } else {
                out.stats.unpack_fill += 1;
            }
            let dst = &mut vu.data[addr];
            for e in 0..4usize {
                if vn == 2 && e == 3 && vl != 3 && !fill {
                    continue;
                }
                let m = if masked || fill { (self.mask >> (cyc.min(3) * 8 + e as u32 * 2)) & 3 } else { 0 };
                let m = if fill && !masked { 1 } else { m };
                let v = match m {
                    0 => match self.mode {
                        1 => data[e].wrapping_add(self.row[e]),
                        2 => {
                            let s = data[e].wrapping_add(self.row[e]);
                            self.row[e] = s;
                            s
                        }
                        _ => data[e],
                    },
                    1 => self.row[e],
                    2 => self.col[cyc.min(3) as usize],
                    _ => continue,
                };
                dst[e] = v;
            }
        }
    }
}
