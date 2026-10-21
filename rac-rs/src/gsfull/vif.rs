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
    pub mpg_words: u64,
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

/// A queued UNPACK of shadow mode (`Vif1::lazy`): the command and the VIF registers it reads at its turn (CYCLE, MODE,
/// MASK, ROW, COL; MODE 2 is never queued, so ROW does not change), and where its data is. `src` points into the
/// word slice of the feed call that queued it: valid until the queue is flushed (see `Vif1::hold`).
struct Pend {
    cmd: u32,
    imm: u32,
    num: u32,
    base: usize,
    cl: u32,
    wl: u32,
    mode: u32,
    mask: u32,
    row: [u32; 4],
    col: [u32; 4],
    src: *const u32,
    words: usize,
}

// SAFETY: the pointer is only dereferenced by `Vif1::flush`, on the thread that fed the slice it points into.
unsafe impl Send for Pend {}

#[derive(Debug)]
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
    /// Lazy shadow feed (default on; RAC_VIF_LAZY=0 or the check model turns it off): in shadow mode a plain UNPACK
    /// (MODE 0, no mask, no fill) is not copied through `buf` and not applied at once; `flush` applies the batch at the
    /// end of the feed call (or before a non-plain UNPACK), skipping ops whose whole destination a later op of the batch
    /// overwrites in all four elements. Nothing reads VU1 data memory between the ops, so memory and every register
    /// are bit-identical to the eager model at each `feed` return.
    pub lazy: bool,
    /// While true the queue is not flushed at the end of a feed call, only before a call that needs VU1 data memory
    /// (non-shadow) or a non-queueable UNPACK, and by `flush_now`: the caller keeps every fed slice alive and unchanged
    /// until it calls `flush_now` (`VuModel::run`).
    pub hold: bool,
    pend: Vec<Pend>,
    dead: Vec<bool>,
    /// Profiling: (ops queued, ops skipped, words queued, words skipped, marks seen, flushes).
    pub cnt: [u64; 12],
}

impl Default for Vif1 {
    fn default() -> Self {
        Vif1::new()
    }
}

impl Vif1 {
    /// RAC_VIF_FEED_CHECK: true when every VIF register and the partial command state equal `o`'s (the buffered words
    /// only count while a command is collecting).
    pub fn state_eq(&self, o: &Vif1) -> bool {
        let regs = (self.cl, self.wl, self.mode, self.mask, self.row, self.col, self.base, self.offset, self.tops, self.itops) == (o.cl, o.wl, o.mode, o.mask, o.row, o.col, o.base, o.offset, o.tops, o.itops)
            && (self.dbf, self.qn, self.qw, self.bad, self.cur_tag2, self.unpack_from_tag, self.shadow) == (o.dbf, o.qn, o.qw, o.bad, o.cur_tag2, o.unpack_from_tag, o.shadow);
        regs && format!("{:?}", self.st) == format!("{:?}", o.st) && (matches!(self.st, St::Idle | St::Pad(_) | St::Direct { .. }) || self.buf == o.buf)
    }

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
            lazy: !std::env::var("RAC_VIF_LAZY").is_ok_and(|v| v == "0"),
            hold: false,
            pend: Vec::new(),
            dead: Vec::new(),
            cnt: [0; 12],
        }
    }

    /// Runs one `Job::Vif1`: `w` the word stream, `marks` the (index, phase) of each DMA block start.
    pub fn feed(&mut self, w: &[u32], marks: &[(usize, u32)], vu: &mut Vu1, out: &mut Out) {
        out.stats.vif_words += w.len() as u64;
        let lazy = self.shadow && self.lazy;
        if !lazy && !self.pend.is_empty() {
            self.flush(vu);
        }
        if self.bad {
            self.bad = false;
            self.st = St::Idle;
        }
        self.cnt[4] += marks.len() as u64;
        self.cnt[5] += 1;
        for (k, &(at, ph)) in marks.iter().enumerate() {
            let end = marks.get(k + 1).map(|m| m.0).unwrap_or(w.len()).min(w.len());
            let mut i = at;
            while i < end {
                if lazy && self.buf.is_empty() {
                    if let St::Unpack { cmd, imm, num, left } = self.st {
                        if left <= end - i && self.defer(cmd, imm, num, w[i..].as_ptr(), left) {
                            let last = i + left - 1;
                            i += left;
                            self.st = St::Idle;
                            let rem = end - i;
                            let pad = 3 - ((ph as usize + (last - at)) & 3);
                            if self.unpack_from_tag && rem < 4 && rem == pad && rem > 0 {
                                self.st = St::Pad(rem);
                            }
                            continue;
                        }
                    }
                }
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
                            if was_unpack && !self.pend.is_empty() {
                                self.flush(vu);
                            }
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
        if !self.hold && !self.pend.is_empty() {
            self.flush(vu);
        }
    }

    /// Applies the queued UNPACKs (the VU1 data memory is exact after this).
    pub fn flush_now(&mut self, vu: &mut Vu1) {
        if !self.pend.is_empty() {
            self.flush(vu);
        }
    }

    /// Queues the UNPACK `cmd` / `imm` / `num` whose `words` data words start at `src`; false (MODE 2: it changes ROW):
    /// the caller runs it the eager way after a flush.
    fn defer(&mut self, cmd: u32, imm: u32, num: u32, src: *const u32, words: usize) -> bool {
        if self.mode == 2 {
            return false;
        }
        let flg = imm & 0x8000 != 0;
        let base = ((imm & 0x3ff) + if flg { self.tops } else { 0 }) as usize;
        self.pend.push(Pend { cmd, imm, num, base, cl: self.cl, wl: self.wl, mode: self.mode, mask: self.mask, row: self.row, col: self.col, src, words });
        true
    }

    /// Applies the queued ops in order, minus those a later op of the queue overwrites completely (no op reads VU1 data
    /// memory in between, so the memory ends bit-identical to the eager model).
    fn flush(&mut self, vu: &mut Vu1) {
        let n = self.pend.len();
        let mut dead = std::mem::take(&mut self.dead);
        dead.clear();
        dead.resize(n, false);
        // covered: bit per VU1 data qword that a later op writes in all four elements
        // (cov4: all four elements; cov3: at least elements 0-2, which is all a V3 op writes)
        let mut cov = [0u64; 16];
        let mut cov3 = [0u64; 16];
        for (k, p) in self.pend.iter().enumerate().rev() {
            // contiguous and CYCLE-skipping ops take part; fill ops (WL > CL) are always applied
            let (cl, wl) = (p.cl.max(1), p.wl.max(1));
            if wl > cl {
                // fill vectors: always applied
                self.cnt[9] += 1;
                continue;
            }
            let (lo, len) = (p.base & 0x3ff, p.num as usize);
            let (vn, vl) = ((p.cmd >> 2) & 3, p.cmd & 3);
            let v3 = vn == 2 && vl != 3;
            if cl != wl {
                // CYCLE skipping: vector k goes to base + (k / wl) * cl + k % wl
                let (cl, wl) = (cl as usize, wl as usize);
                let at = |k: usize| (p.base + (k / wl) * cl + k % wl) & 0x3ff;
                let tc = if v3 { &cov3 } else { &cov };
                if (0..len).all(|k| tc[at(k) >> 6] >> (at(k) & 63) & 1 != 0) {
                    dead[k] = true;
                    continue;
                }
                self.cnt[9] += 1;
                if p.cmd & 0x10 == 0 {
                    for q in 0..len {
                        let a = at(q);
                        cov3[a >> 6] |= 1 << (a & 63);
                        if !v3 {
                            cov[a >> 6] |= 1 << (a & 63);
                        }
                    }
                }
                continue;
            }
            if range_all(if v3 { &cov3 } else { &cov }, lo, len) {
                dead[k] = true;
                continue;
            }
            if p.cmd & 0x10 == 0 {
                range_set(&mut cov3, lo, len);
                if !v3 {
                    range_set(&mut cov, lo, len);
                }
            }
            // survivor classes (profiling)
            self.cnt[6] += v3 as u64;
            self.cnt[7] += (p.cmd & 0x10 != 0) as u64;
            self.cnt[8] += (p.mode == 1) as u64;
        }
        for (k, p) in self.pend.iter().enumerate() {
            self.cnt[0] += 1;
            self.cnt[2] += p.words as u64;
            if dead[k] {
                self.cnt[1] += 1;
                self.cnt[3] += p.words as u64;
                continue;
            }
            // SAFETY: `src` / `words` describe a slice of a feed call's input; the caller keeps it alive until the flush.
            let src = unsafe { std::slice::from_raw_parts(p.src, p.words) };
            let (vn, vl) = ((p.cmd >> 2) & 3, p.cmd & 3);
            let (cl, wl) = (p.cl.max(1), p.wl.max(1));
            if vn == 3 && vl == 0 && p.cmd & 0x10 == 0 && p.mode == 0 && cl == wl && src.len() >= p.num as usize * 4 {
                copy_v4(src, p.base, p.num as usize, &mut vu.data);
            } else {
                let mut row = p.row;
                unpack_core(src, p.cmd, p.imm, p.num, p.base, p.cl, p.wl, p.mode, p.mask, &mut row, &p.col, &mut vu.data);
            }
        }
        self.pend.clear();
        self.dead = dead;
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
                out.stats.mpg_words += self.buf.len() as u64;
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
        let flg = imm & 0x8000 != 0;
        let base = ((imm & 0x3ff) + if flg { self.tops } else { 0 }) as usize;
        let src = std::mem::take(&mut self.buf);
        let fills = unpack_core(&src, cmd, imm, num, base, self.cl, self.wl, self.mode, self.mask, &mut self.row, &self.col, &mut vu.data);
        self.buf = src;
        out.stats.unpack_fill += fills;
    }
}

/// True when the `len` data qwords from `lo` (wrapping at 1024) are all set in `cov`.
fn range_all(cov: &[u64; 16], lo: usize, len: usize) -> bool {
    let mut ok = true;
    range_words(lo, len, |w, m| ok &= cov[w] & m == m);
    ok
}

fn range_set(cov: &mut [u64; 16], lo: usize, len: usize) {
    range_words(lo, len, |w, m| cov[w] |= m);
}

/// Calls `f(word, mask)` for the 64-bit words of the bit range [lo, lo + len) of a 1024-bit set (wraps; len <= 1024).
fn range_words(lo: usize, len: usize, mut f: impl FnMut(usize, u64)) {
    let mut a = lo & 0x3ff;
    let mut left = len.min(1024);
    while left > 0 {
        let take = left.min(1024 - a).min(64 - (a & 63));
        let m = if take == 64 { u64::MAX } else { ((1u64 << take) - 1) << (a & 63) };
        f(a >> 6, m);
        a = (a + take) & 0x3ff;
        left -= take;
    }
}

/// V4-32 UNPACK with contiguous addresses: straight copy of `num` qwords from `base` (wrapping at 1024).
fn copy_v4(src: &[u32], base: usize, num: usize, data: &mut [[u32; 4]]) {
    let mut a = base & 0x3ff;
    let mut k = 0usize;
    while k < num {
        let run = (num - k).min(1024 - a);
        for (d, s) in data[a..a + run].iter_mut().zip(src[k * 4..(k + run) * 4].chunks_exact(4)) {
            d.copy_from_slice(s);
        }
        k += run;
        a = (a + run) & 0x3ff;
    }
}

/// The UNPACK of `Vif1::unpack`, with the registers it reads passed in (`row` is written in MODE 2). Returns the
/// number of fill vectors.
#[allow(clippy::too_many_arguments)]
fn unpack_core(src: &[u32], cmd: u32, imm: u32, num: u32, base: usize, cl: u32, wl: u32, mode: u32, mask: u32, row: &mut [u32; 4], col: &[u32; 4], mem: &mut [[u32; 4]]) -> u64 {
    let vn = (cmd >> 2) & 3;
    let vl = cmd & 3;
    let masked = cmd & 0x10 != 0;
    let usn = imm & 0x4000 != 0;
    let elems = vn as usize + 1;
    let (cl, wl) = (cl.max(1), wl.max(1));
    let mut fills = 0u64;
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
        let mut dv = [0u32; 4];
        if !fill {
            if vl == 3 {
                let w = src.get(sk / 2).copied().unwrap_or(0);
                let v = (w >> ((sk & 1) * 16)) & 0xffff;
                dv = [(v & 0x1f) << 3, ((v >> 5) & 0x1f) << 3, ((v >> 10) & 0x1f) << 3, ((v >> 15) & 1) << 7];
            } else {
                for e in 0..elems {
                    dv[e] = rd(sk, e);
                }
                match vn {
                    0 => dv = [dv[0]; 4],
                    1 => dv = [dv[0], dv[1], dv[0], dv[1]],
                    _ => {}
                }
            }
            sk += 1;
        } else {
            fills += 1;
        }
        let dst = &mut mem[addr];
        for e in 0..4usize {
            if vn == 2 && e == 3 && vl != 3 && !fill {
                continue;
            }
            let m = if masked || fill { (mask >> (cyc.min(3) * 8 + e as u32 * 2)) & 3 } else { 0 };
            let m = if fill && !masked { 1 } else { m };
            let v = match m {
                0 => match mode {
                    1 => dv[e].wrapping_add(row[e]),
                    2 => {
                        let s = dv[e].wrapping_add(row[e]);
                        row[e] = s;
                        s
                    }
                    _ => dv[e],
                },
                1 => row[e],
                2 => col[cyc.min(3) as usize],
                _ => continue,
            };
            dst[e] = v;
        }
    }

    fills
}
