//! VU1: the PS2 vector unit that turns the game's model data into GS triangle packets (PATH1, XGKICK).
//!
//! Two engines with one semantics:
//! - the interpreter below (reference; port of the GS-full prototype `scratch_gsfull/gsfull/src/vu1.rs`, itself a port
//!   of ee-oracle's VU0 micro mode): instruction pairs, VF write latency 4 cycles (per register), Q after 7 / 13
//!   cycles, MAC/status/clip flags visible 4 cycles after issue, branch delay slots, the E-bit slot, the branch hazard
//!   (a branch reads an integer register one cycle early), XGKICK handed to the sink at once;
//! - ahead-of-time translations of the retail microprograms (`vu1aot.rs`, generated `vu1_progN.rs`), used when the
//!   micro memory holds exactly a known program. They reproduce the interpreter bit for bit (VF/VI/ACC, data memory,
//!   flags, Q, cycle and pair counts, XGKICK order) and hand any state they do not cover back to the interpreter
//!   mid-run ("bail"), so the interpreter stays the fallback for unknown microcode and odd paths.
//!
//! Floats run on host SSE with MXCSR set once per run (DAZ, FTZ, round toward zero); operands are clamped like the
//! oracle's fpu.rs (exponent 255 -> +-FLT_MAX), so no inf/NaN enters or leaves.
//!
//! Env `VU1AOT`: unset/`1` native when available, `0` interpreter only, `diff` runs both on every run and compares
//! everything (slow; prints the first difference, keeps the interpreter's result).
use std::collections::VecDeque;

pub(crate) const ACC: usize = 32;
const NONE: usize = 99;
const MAXF: u32 = 0x7f7f_ffff;
/// Interpreter runaway cap (pairs per run).
pub(crate) const GUARD: u32 = 3_000_000;

const ADD: u8 = 0;
const SUB: u8 = 1;
const MADD: u8 = 2;
const MSUB: u8 = 3;
const MAX: u8 = 4;
const MIN: u8 = 5;
const MUL: u8 = 6;
const BC_KIND: [u8; 7] = [ADD, SUB, MADD, MSUB, MAX, MIN, MUL];
const QI: [(u8, bool); 12] = [
    (MUL, false), (MAX, true), (MUL, true), (MIN, true), (ADD, false), (MADD, false),
    (ADD, true), (MADD, true), (SUB, false), (MSUB, false), (SUB, true), (MSUB, true),
];
const FULL: [u8; 8] = [ADD, MADD, MUL, MAX, SUB, MSUB, 255, MIN];

#[inline(always)]
pub(crate) fn fin(b: u32) -> f32 {
    if b & 0x7f80_0000 == 0x7f80_0000 { f32::from_bits((b & 0x8000_0000) | MAXF) } else { f32::from_bits(b) }
}
#[inline(always)]
pub(crate) fn fout(r: f32) -> u32 {
    let b = r.to_bits();
    if b & 0x7f80_0000 == 0x7f80_0000 { (b & 0x8000_0000) | MAXF } else { b }
}
#[inline(always)]
fn fz(b: u32) -> f32 {
    if b & 0x7f80_0000 == 0 { f32::from_bits(b & 0x8000_0000) } else { fin(b) }
}
#[inline(always)]
fn vmul(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    [fout(fin(a[0]) * fin(b[0])), fout(fin(a[1]) * fin(b[1])), fout(fin(a[2]) * fin(b[2])), fout(fin(a[3]) * fin(b[3]))]
}
#[inline(always)]
fn vadd(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    [fout(fin(a[0]) + fin(b[0])), fout(fin(a[1]) + fin(b[1])), fout(fin(a[2]) + fin(b[2])), fout(fin(a[3]) + fin(b[3]))]
}
#[inline(always)]
fn vsub(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    [fout(fin(a[0]) - fin(b[0])), fout(fin(a[1]) - fin(b[1])), fout(fin(a[2]) - fin(b[2])), fout(fin(a[3]) - fin(b[3]))]
}
#[inline(always)]
fn fmax(a: u32, b: u32) -> u32 {
    if fz(a) > fz(b) { a } else { b }
}
#[inline(always)]
fn fmin(a: u32, b: u32) -> u32 {
    if fz(a) < fz(b) { a } else { b }
}
fn cvt_w(b: u32) -> u32 {
    if b & 0x7f80_0000 <= 0x4e80_0000 {
        (fin(b) as i32) as u32
    } else if b & 0x8000_0000 != 0 {
        0x8000_0000
    } else {
        0x7fff_ffff
    }
}
/// VU DIV: zero/denormal divisor gives +-FLT_MAX with the xor sign.
pub(crate) fn fdiv(a: u32, b: u32) -> u32 {
    if b & 0x7f80_0000 == 0 {
        return ((a ^ b) & 0x8000_0000) | MAXF;
    }
    fout(fin(a) / fin(b))
}
/// VU SQRT of |t|.
pub(crate) fn fsqrt(t: u32) -> u32 {
    let b = t & 0x7fff_ffff;
    if b & 0x7f80_0000 == 0 { 0 } else { fout(fin(b).sqrt()) }
}

/// MAC flag of a result: bit (3-lane) zero, bit (7-lane) sign, for the lanes in `dest` (bit 3 = x).
pub(crate) fn mac_of(v: [u32; 4], dest: u32) -> u32 {
    let mut m = 0;
    for l in 0..4 {
        if dest & (8 >> l) != 0 {
            let b = 3 - l;
            if v[l] & 0x7fff_ffff == 0 {
                m |= 1 << b;
            }
            if v[l] >> 31 != 0 {
                m |= 1 << (b + 4);
            }
        }
    }
    m
}
/// Status low bits (Z, S) of a MAC flag.
#[inline(always)]
pub(crate) fn st_of(m: u32) -> u32 {
    ((m & 0xf != 0) as u32) | (((m & 0xf0 != 0) as u32) << 1)
}
#[inline(always)]
fn lane(l: usize) -> u32 {
    8 >> l
}

struct Up {
    dst: usize,
    val: [u32; 4],
    mask: u32,
    mac: Option<u32>,
    clip: Option<u32>,
    reads: [usize; 3],
}

/// Where XGKICK packets go: `data` is VU1 data memory at the moment of the kick, `addr` the qword address.
pub trait Path1 {
    fn xgkick(&mut self, data: &[[u32; 4]], addr: usize);
}

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct VuStats {
    pub runs: u64,
    pub pairs: u64,
    pub cycles: u64,
    pub xgkicks: u64,
    pub errors: u64,
}

/// Counters of the native path (not part of the compared VU state).
#[derive(Default, Clone, Debug)]
pub struct AotStats {
    /// Runs started natively.
    pub native: u64,
    /// Native runs handed to the interpreter mid-run.
    pub bails: u64,
    /// Runs on the interpreter because the code or the entry is not covered.
    pub interp: u64,
    /// Diff mode: runs compared / runs that differed.
    pub compared: u64,
    pub differed: u64,
    /// Bails by (native program id, pc where the interpreter took over).
    pub bail_at: std::collections::BTreeMap<(u8, usize), u64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AotMode {
    Off,
    On,
    Diff,
}

/// The interpreter's in-flight state of one run (timing pipeline, pending branch, hazard). A native run that bails
/// builds one of these, so the interpreter continues exactly where the native code stopped.
#[derive(Clone, Debug)]
pub(crate) struct Pipe {
    pub pc: usize,
    pub cyc: u64,
    pub ready: [u64; 33],
    pub q_pend: Option<(u64, u32)>,
    /// (ready cycle, kind 0 = MAC (value, status) / 1 = clip (value), value, status bits)
    pub flags: VecDeque<(u64, u8, u32, u32)>,
    pub clip_latest: u32,
    pub taken: Option<usize>,
    pub end_next: bool,
    pub pairs: u64,
    pub prev_old: [u32; 16],
    pub prev_mask: u32,
}

impl Pipe {
    pub fn fresh(pc: usize, clip: u32) -> Pipe {
        Pipe {
            pc: pc & 0x3ff8,
            cyc: 0,
            ready: [0; 33],
            q_pend: None,
            flags: VecDeque::new(),
            clip_latest: clip,
            taken: None,
            end_next: false,
            pairs: 0,
            prev_old: [0; 16],
            prev_mask: 0,
        }
    }
}

pub struct Vu1 {
    pub vf: [[u32; 4]; 32],
    pub acc: [u32; 4],
    pub vi: [u32; 32],
    /// 1024 qwords (16 KB).
    pub data: Vec<[u32; 4]>,
    /// 2048 instruction pairs (16 KB); write through `mpg` so the native-code check sees the change.
    micro: Vec<u64>,
    micro_gen: u64,
    pub tpc: usize,
    pub st: VuStats,
    pub last_err: Option<String>,
    /// The branch hazard (on by default; env GSFULL_NOBRHAZ turns it off, which also turns the native path off).
    pub br_hazard: bool,
    pub aot_mode: AotMode,
    pub aot: AotStats,
    pub(crate) aot_cache: crate::gsfull::vu1aot::Cache,
    /// Diff mode: first differences (text), at most a few.
    pub diff_log: Vec<String>,
}

impl Vu1 {
    /// `data` / `micro`: VU1 data and micro memory images (e.g. a savestate's vu1Memory.bin / vu1MicroMem.bin).
    pub fn new(data: &[u8], micro: &[u8]) -> Vu1 {
        let mut d = vec![[0u32; 4]; 1024];
        for (i, q) in d.iter_mut().enumerate() {
            for k in 0..4 {
                let o = i * 16 + k * 4;
                if o + 4 <= data.len() {
                    q[k] = u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
                }
            }
        }
        let mut m = vec![0u64; 2048];
        for (i, w) in m.iter_mut().enumerate() {
            let o = i * 8;
            if o + 8 <= micro.len() {
                *w = u64::from_le_bytes(micro[o..o + 8].try_into().unwrap());
            }
        }
        let br_hazard = std::env::var("GSFULL_NOBRHAZ").is_err();
        let aot_mode = match std::env::var("VU1AOT").as_deref() {
            Ok("0") | Ok("off") => AotMode::Off,
            Ok("diff") => AotMode::Diff,
            _ => AotMode::On,
        };
        Vu1 {
            vf: {
                let mut v = [[0u32; 4]; 32];
                v[0] = [0, 0, 0, 0x3f80_0000];
                v
            },
            acc: [0; 4],
            vi: [0; 32],
            data: d,
            micro: m,
            micro_gen: 1,
            tpc: 0,
            st: VuStats::default(),
            last_err: None,
            br_hazard,
            aot_mode,
            aot: AotStats::default(),
            aot_cache: Default::default(),
            diff_log: Vec::new(),
        }
    }

    /// Micro memory (2048 pairs, lower word in the low half).
    pub fn micro(&self) -> &[u64] {
        &self.micro
    }

    /// MPG: writes instruction pairs starting at pair index `at` (wraps at 2048).
    pub fn mpg(&mut self, at: usize, pairs: &[u64]) {
        for (k, &p) in pairs.iter().enumerate() {
            self.micro[(at + k) & 0x7ff] = p;
        }
        self.micro_gen += 1;
    }

    /// The micro memory image (2048 pairs).
    pub fn micro_image(&self) -> &[u64] {
        &self.micro
    }

    pub(crate) fn micro_gen(&self) -> u64 {
        self.micro_gen
    }

    fn set_vi(&mut self, i: usize, v: u32) {
        if i != 0 {
            self.vi[i] = if i < 16 { v & 0xffff } else { v };
        }
    }

    fn fmac(&self, kind: u8, s: [u32; 4], t: [u32; 4]) -> [u32; 4] {
        match kind {
            ADD => vadd(s, t),
            SUB => vsub(s, t),
            MUL => vmul(s, t),
            MADD => vadd(self.acc, vmul(s, t)),
            MSUB => vsub(self.acc, vmul(s, t)),
            MAX => [fmax(s[0], t[0]), fmax(s[1], t[1]), fmax(s[2], t[2]), fmax(s[3], t[3])],
            _ => [fmin(s[0], t[0]), fmin(s[1], t[1]), fmin(s[2], t[2]), fmin(s[3], t[3])],
        }
    }

    fn upper(&self, op: u32, clip_prev: u32) -> Result<Up, String> {
        let dest = (op >> 21) & 15;
        let ft = ((op >> 16) & 31) as usize;
        let fs = ((op >> 11) & 31) as usize;
        let fd = ((op >> 6) & 31) as usize;
        let s = self.vf[fs];
        let t = self.vf[ft];
        let bc = (op & 3) as usize;
        let (q, i) = (self.vi[22], self.vi[21]);
        let mut u = Up { dst: NONE, val: [0; 4], mask: dest, mac: None, clip: None, reads: [NONE; 3] };
        let f = (op & 0x3f) as usize;
        let acc_read = |k: u8| if k == MADD || k == MSUB { ACC } else { NONE };
        if f < 0x3c {
            let (k, tv, tr) = match f {
                0..=27 => (BC_KIND[f >> 2], [t[bc]; 4], ft),
                28..=39 => {
                    let (k, isi) = QI[f - 28];
                    (k, [if isi { i } else { q }; 4], NONE)
                }
                46 => {
                    let p = vmul([s[1], s[2], s[0], 0], [t[2], t[0], t[1], 0]);
                    u.val = vsub(self.acc, p);
                    u.mask = dest & 0xe;
                    u.dst = fd;
                    u.mac = Some(mac_of(u.val, u.mask));
                    u.reads = [fs, ft, ACC];
                    return Ok(u);
                }
                40..=47 => (FULL[f - 40], t, ft),
                _ => return Err(format!("unknown VU upper {op:08x}")),
            };
            u.val = self.fmac(k, s, tv);
            u.dst = fd;
            u.reads = [fs, tr, acc_read(k)];
            if k != MAX && k != MIN {
                u.mac = Some(mac_of(u.val, dest));
            }
            return Ok(u);
        }
        let x = ((op & 3) | ((op >> 4) & 0x7c)) as usize;
        match x {
            0..=15 | 24..=27 => {
                let k = BC_KIND[x >> 2];
                u.val = self.fmac(k, s, [t[bc]; 4]);
                u.dst = ACC;
                u.reads = [fs, ft, acc_read(k)];
                u.mac = Some(mac_of(u.val, dest));
            }
            16..=19 => {
                let sc = [1.0f32, 0.0625, 1.0 / 4096.0, 1.0 / 32768.0][x - 16];
                for l in 0..4 {
                    let v = s[l] as i32 as f32;
                    u.val[l] = fout(v * sc);
                }
                u.dst = ft;
                u.reads[0] = fs;
            }
            20..=23 => {
                let sc = [1.0f32, 16.0, 4096.0, 32768.0][x - 20];
                for l in 0..4 {
                    let v = fout(fin(s[l]) * sc);
                    u.val[l] = cvt_w(v);
                }
                u.dst = ft;
                u.reads[0] = fs;
            }
            29 => {
                u.val = [s[0] & 0x7fff_ffff, s[1] & 0x7fff_ffff, s[2] & 0x7fff_ffff, s[3] & 0x7fff_ffff];
                u.dst = ft;
                u.reads[0] = fs;
            }
            31 => {
                let w = fz(t[3]).abs();
                let mut c = (clip_prev << 6) & 0xff_ffff;
                for l in 0..3 {
                    let v = fz(s[l]);
                    if v > w {
                        c |= 1 << (2 * l);
                    }
                    if v < -w {
                        c |= 2 << (2 * l);
                    }
                }
                u.clip = Some(c);
                u.reads = [fs, ft, NONE];
            }
            28 | 30 | 32..=39 => {
                let (k, isi) = QI[x - 28];
                u.val = self.fmac(k, s, [if isi { i } else { q }; 4]);
                u.dst = ACC;
                u.reads = [fs, NONE, acc_read(k)];
                u.mac = Some(mac_of(u.val, dest));
            }
            40..=42 | 44 | 45 => {
                let k = FULL[x - 40];
                u.val = self.fmac(k, s, t);
                u.dst = ACC;
                u.reads = [fs, ft, acc_read(k)];
                u.mac = Some(mac_of(u.val, dest));
            }
            46 => {
                u.val = vmul([s[1], s[2], s[0], 0], [t[2], t[0], t[1], 0]);
                u.mask = dest & 0xe;
                u.dst = ACC;
                u.reads = [fs, ft, NONE];
                u.mac = Some(mac_of(u.val, u.mask));
            }
            47 => {}
            _ => return Err(format!("unknown VU upper {op:08x}")),
        }
        Ok(u)
    }

    /// Runs from `start` (byte address) to the E bit. `top` / `itop` are VIF1's TOP and ITOP at the start.
    /// Returns the run's VU1 cycle count.
    pub fn run<S: Path1>(&mut self, start: usize, top: u32, itop: u32, sink: &mut S) -> Result<u64, String> {
        #[allow(deprecated)]
        let saved = unsafe { std::arch::x86_64::_mm_getcsr() };
        #[allow(deprecated)]
        unsafe {
            std::arch::x86_64::_mm_setcsr(0x1f80 | 0x0040 | 0x8000 | 0x6000)
        };
        self.st.runs += 1;
        let r = match self.aot_mode {
            AotMode::Diff if self.br_hazard => self.run_diff(start, top, itop, sink),
            AotMode::On if self.br_hazard => self.run_native(start, top, itop, sink),
            _ => {
                let mut p = Pipe::fresh(start, self.vi[18]);
                self.interp(&mut p, top, itop, sink)
            }
        };
        #[allow(deprecated)]
        unsafe {
            std::arch::x86_64::_mm_setcsr(saved)
        };
        if let Err(e) = &r {
            self.st.errors += 1;
            self.last_err = Some(e.clone());
        }
        r
    }

    /// Native code when the micro memory and the entry are covered, else (or from a bail point) the interpreter.
    /// Runs under the run's MXCSR.
    pub(crate) fn run_native(&mut self, start: usize, top: u32, itop: u32, sink: &mut dyn Path1) -> Result<u64, String> {
        match crate::gsfull::vu1aot::exec(self, start, top, itop, sink) {
            crate::gsfull::vu1aot::Exec::Done(c) => {
                self.aot.native += 1;
                Ok(c)
            }
            crate::gsfull::vu1aot::Exec::Bail(mut p) => {
                self.aot.native += 1;
                self.aot.bails += 1;
                let id = crate::gsfull::vu1aot::native_prog(self).unwrap_or(255);
                *self.aot.bail_at.entry((id, p.pc)).or_default() += 1;
                self.interp(&mut p, top, itop, sink)
            }
            crate::gsfull::vu1aot::Exec::NotCovered => {
                self.aot.interp += 1;
                let mut p = Pipe::fresh(start, self.vi[18]);
                self.interp(&mut p, top, itop, sink)
            }
        }
    }

    fn snapshot(&self) -> Snap {
        Snap { vf: self.vf, acc: self.acc, vi: self.vi, data: self.data.clone(), tpc: self.tpc, st: self.st.clone() }
    }
    fn restore(&mut self, s: &Snap) {
        self.vf = s.vf;
        self.acc = s.acc;
        self.vi = s.vi;
        self.data.copy_from_slice(&s.data);
        self.tpc = s.tpc;
        self.st = s.st.clone();
    }

    /// Diff mode: interpreter (feeds the real sink) and native code (recorded) on the same input; compares the whole
    /// VU state, the XGKICK packets and the result; keeps the interpreter's state.
    fn run_diff<S: Path1>(&mut self, start: usize, top: u32, itop: u32, sink: &mut S) -> Result<u64, String> {
        let before = self.snapshot();
        let mut tee = Tee { inner: Some(sink), rec: Vec::new() };
        let mut p = Pipe::fresh(start, self.vi[18]);
        let ri = self.interp(&mut p, top, itop, &mut tee);
        let after_i = self.snapshot();
        let rec_i = std::mem::take(&mut tee.rec);
        self.restore(&before);
        let mut rec = Tee::<S> { inner: None, rec: Vec::new() };
        let rn = self.run_native(start, top, itop, &mut rec);
        let after_n = self.snapshot();
        self.aot.compared += 1;
        let mut why = Vec::new();
        if ri != rn {
            why.push(format!("result {ri:?} vs native {rn:?}"));
        }
        diff_snap(&after_i, &after_n, &mut why);
        if rec_i != rec.rec {
            let k = rec_i.iter().zip(rec.rec.iter()).position(|(a, b)| a != b).unwrap_or(rec_i.len().min(rec.rec.len()));
            why.push(format!("xgkick packets differ: {} vs {} packets, first difference at packet {k}", rec_i.len(), rec.rec.len()));
        }
        if !why.is_empty() {
            self.aot.differed += 1;
            if self.diff_log.len() < 8 {
                self.diff_log.push(format!("VU1AOT diff (entry {start:04x} top {top:x} itop {itop:x}): {}", why.join("; ")));
            }
        }
        self.restore(&after_i);
        ri
    }

    /// The interpreter, from any in-flight state `p` (a fresh one at a run's start).
    pub(crate) fn interp(&mut self, p: &mut Pipe, top: u32, itop: u32, sink: &mut dyn Path1) -> Result<u64, String> {
        let mut pc = p.pc;
        let mut guard = p.pairs as u32;
        loop {
            guard += 1;
            if guard > GUARD {
                return Err(format!("VU1 run does not end (pc {pc:04x})"));
            }
            let w = self.micro[pc >> 3];
            let lo = w as u32;
            let hi = (w >> 32) as u32;
            let ibit = hi & 0x8000_0000 != 0;
            let up = self.upper(hi & 0x07ff_ffff, p.clip_latest).map_err(|e| format!("{e} at vu1 {pc:04x}"))?;
            let lr = if ibit { LowerInfo::default() } else { lower_info(lo) };
            // Stalls.
            let mut need = p.cyc;
            for &r in up.reads.iter().chain(lr.reads.iter()) {
                if r != NONE {
                    need = need.max(p.ready[r]);
                }
            }
            if lr.waitq || lr.div {
                if let Some((r, _)) = p.q_pend {
                    need = need.max(r);
                }
            }
            p.cyc = need;
            let cyc = p.cyc;
            if let Some((r, v)) = p.q_pend {
                if cyc >= r {
                    self.vi[22] = v;
                    p.q_pend = None;
                }
            }
            while let Some(&(r, k, v, st)) = p.flags.front() {
                if r > cyc {
                    break;
                }
                if k == 0 {
                    self.vi[17] = v;
                    self.vi[16] = (self.vi[16] & !0xf) | st | (st << 6);
                } else {
                    self.vi[18] = v;
                }
                p.flags.pop_front();
            }
            // Recompute on the post-stall state (Q / flags may have landed).
            let up = self.upper(hi & 0x07ff_ffff, p.clip_latest).map_err(|e| format!("{e} at vu1 {pc:04x}"))?;
            let mut branch = None;
            let before: [u32; 16] = self.vi[..16].try_into().unwrap();
            if !ibit {
                branch = self.lower(lo, pc, p, top, itop, sink).map_err(|e| format!("{e} at vu1 {pc:04x}"))?;
            }
            {
                let mut m = 0u32;
                for i in 1..16 {
                    if self.vi[i] != before[i] {
                        m |= 1 << i;
                    }
                }
                // Flag reads (FSAND / FCAND / FMAND / FCGET / ...) do not delay a following branch: it sees the new value
                // (PCSX2 microVU analyzeBranchVI: no VI backup when the writer reads flags).
                if !ibit && (0x10..=0x1c).contains(&(lo >> 25)) {
                    m = 0;
                }
                p.prev_old = before;
                p.prev_mask = m;
            }
            // Commit upper.
            if up.dst != NONE && up.dst != 0 {
                for l in 0..4 {
                    if up.mask & lane(l) != 0 {
                        if up.dst == ACC {
                            self.acc[l] = up.val[l];
                        } else {
                            self.vf[up.dst][l] = up.val[l];
                        }
                    }
                }
                p.ready[up.dst] = cyc + 4;
            }
            if let Some(m) = up.mac {
                p.flags.push_back((cyc + 4, 0, m, st_of(m)));
            }
            if let Some(c) = up.clip {
                p.clip_latest = c;
                p.flags.push_back((cyc + 4, 1, c, 0));
            }
            if ibit {
                self.vi[21] = lo;
            }
            p.cyc += 1;
            p.pairs += 1;
            if p.end_next {
                self.tpc = (pc + 8) & 0x3ff8;
                break;
            }
            if hi & 0x4000_0000 != 0 {
                p.end_next = true;
            }
            let next = p.taken.take().unwrap_or(pc + 8);
            p.taken = branch;
            pc = next & 0x3ff8;
            p.pc = pc;
        }
        if let Some((_, v)) = p.q_pend.take() {
            self.vi[22] = v;
        }
        while let Some((_, k, v, st)) = p.flags.pop_front() {
            if k == 0 {
                self.vi[17] = v;
                self.vi[16] = (self.vi[16] & !0xf) | st | (st << 6);
            } else {
                self.vi[18] = v;
            }
        }
        self.st.pairs += p.pairs;
        self.st.cycles += p.cyc;
        Ok(p.cyc)
    }

    fn lower(&mut self, op: u32, pc: usize, p: &mut Pipe, top: u32, itop: u32, sink: &mut dyn Path1) -> Result<Option<usize>, String> {
        let cyc = p.cyc;
        let dest = (op >> 21) & 15;
        let it = ((op >> 16) & 31) as usize;
        let is = ((op >> 11) & 31) as usize;
        let id = ((op >> 6) & 31) as usize;
        let imm11 = (((op & 0x7ff) as i32) << 21 >> 21) as u32;
        let target = || ((pc as i64 + 8 + (imm11 as i32 as i64) * 8) as usize) & 0x3ff8;
        let vi = |c: &Vu1, r: usize| c.vi[r] & 0xffff;
        // A branch reads an integer register one cycle early: if the instruction right before wrote it, the old value is seen.
        let (hz, pmask, pold) = (self.br_hazard, p.prev_mask, p.prev_old);
        let bvi = |c: &Vu1, r: usize| if hz && r < 16 && (pmask >> r) & 1 != 0 { pold[r] & 0xffff } else { c.vi[r] & 0xffff };
        let qaddr = |c: &Vu1, r: usize, off: u32| (c.vi[r].wrapping_add(off) & 0x3ff) as usize;
        let opc = op >> 25;
        let ready = &mut p.ready;
        let wvf = |c: &mut Vu1, ready: &mut [u64; 33], r: usize, v: [u32; 4], m: u32| {
            if r == 0 {
                return;
            }
            for l in 0..4 {
                if m & lane(l) != 0 {
                    c.vf[r][l] = v[l];
                }
            }
            ready[r] = cyc + 4;
        };
        let flag = |c: &Vu1, k: usize| c.vi[k];
        match opc {
            0x00 => {
                let a = qaddr(self, is, imm11);
                let v = self.data[a];
                wvf(self, ready, it, v, dest);
            }
            0x01 => {
                let a = qaddr(self, it, imm11);
                let s = self.vf[is];
                for l in 0..4 {
                    if dest & lane(l) != 0 {
                        self.data[a][l] = s[l];
                    }
                }
            }
            0x04 => {
                let a = qaddr(self, is, imm11);
                let l = (0..4).find(|&l| dest & lane(l) != 0).unwrap_or(0);
                let v = self.data[a][l] & 0xffff;
                self.set_vi(it, v);
            }
            0x05 => {
                let a = qaddr(self, is, imm11);
                let v = vi(self, it);
                for l in 0..4 {
                    if dest & lane(l) != 0 {
                        self.data[a][l] = v;
                    }
                }
            }
            0x08 | 0x09 => {
                let im = ((op >> 10) & 0x7800) | (op & 0x7ff);
                let v = if opc == 8 { self.vi[is].wrapping_add(im) } else { self.vi[is].wrapping_sub(im) };
                self.set_vi(it, v);
            }
            0x10..=0x13 => {
                let im = op & 0xff_ffff;
                let c = flag(self, 18) & 0xff_ffff;
                match opc {
                    0x10 => self.vi[1] = (c == im) as u32,
                    0x11 => {
                        // FCSET writes a new clip flag instance like CLIP does (PCSX2 microVU): the next CLIP shifts this
                        // value, not the chain from before it, and FCAND / FCGET / ... see it once it lands (4 cycles).
                        p.clip_latest = im;
                        p.flags.push_back((cyc + 4, 1, im, 0));
                    }
                    0x12 => self.vi[1] = (c & im != 0) as u32,
                    _ => self.vi[1] = ((c | im) & 0xff_ffff == 0xff_ffff) as u32,
                }
            }
            0x14..=0x17 => {
                let im = ((op >> 10) & 0x800) | (op & 0x7ff);
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
            0x24 => return Ok(Some(((bvi(self, is) as usize) * 8) & 0x3ff8)),
            0x25 => {
                let t = ((bvi(self, is) as usize) * 8) & 0x3ff8;
                self.set_vi(it, ((pc + 16) / 8) as u32);
                return Ok(Some(t));
            }
            0x28..=0x2f => {
                let a = bvi(self, is) as u16 as i16;
                let b = bvi(self, it) as u16 as i16;
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
                                let v = self.data[a];
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
                                        self.data[a][l] = s[l];
                                    }
                                }
                                if x == 0x35 {
                                    self.set_vi(it, self.vi[it].wrapping_add(1));
                                }
                            }
                            0x38..=0x3a => {
                                let t = self.vf[ft][ftf];
                                let (v, lat) = match x {
                                    0x38 => (fdiv(s[fsf], t), 7),
                                    0x39 => (fsqrt(t), 7),
                                    _ => (fdiv(s[fsf], fsqrt(t)), 13),
                                };
                                p.q_pend = Some((cyc + lat, v));
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
                                let v = self.data[a][l] & 0xffff;
                                self.set_vi(it, v);
                            }
                            0x3f => {
                                let a = qaddr(self, is, 0);
                                let v = vi(self, it);
                                for l in 0..4 {
                                    if dest & lane(l) != 0 {
                                        self.data[a][l] = v;
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
                            0x68 => self.set_vi(it, top),
                            0x69 => self.set_vi(it, itop),
                            0x6c => {
                                self.st.xgkicks += 1;
                                let a = (self.vi[is] & 0x3ff) as usize;
                                sink.xgkick(&self.data, a);
                            }
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
    reads: [usize; 2],
    waitq: bool,
    div: bool,
}

fn lower_info(op: u32) -> LowerInfo {
    let mut li = LowerInfo { reads: [NONE; 2], waitq: false, div: false };
    let ft = ((op >> 16) & 31) as usize;
    let fs = ((op >> 11) & 31) as usize;
    match op >> 25 {
        0x01 => li.reads[0] = fs,
        0x40 if op & 0x3c == 0x3c => {
            let x = (op & 3) | ((op >> 4) & 0x7c);
            match x {
                0x30 | 0x31 | 0x35 | 0x37 => li.reads[0] = fs,
                0x38 | 0x3a => {
                    li.reads = [fs, ft];
                    li.div = true;
                }
                0x39 => {
                    li.reads[0] = ft;
                    li.div = true;
                }
                0x3b => li.waitq = true,
                0x3c | 0x42 | 0x43 => li.reads[0] = fs,
                _ => {}
            }
        }
        _ => {}
    }
    li
}

struct Snap {
    vf: [[u32; 4]; 32],
    acc: [u32; 4],
    vi: [u32; 32],
    data: Vec<[u32; 4]>,
    tpc: usize,
    st: VuStats,
}

fn diff_snap(a: &Snap, b: &Snap, why: &mut Vec<String>) {
    for r in 0..32 {
        if a.vf[r] != b.vf[r] {
            why.push(format!("vf{r:02} {:08x?} vs {:08x?}", a.vf[r], b.vf[r]));
        }
    }
    if a.acc != b.acc {
        why.push(format!("acc {:08x?} vs {:08x?}", a.acc, b.acc));
    }
    for r in 0..32 {
        if a.vi[r] != b.vi[r] {
            why.push(format!("vi{r:02} {:08x} vs {:08x}", a.vi[r], b.vi[r]));
        }
    }
    if let Some(k) = (0..a.data.len()).find(|&k| a.data[k] != b.data[k]) {
        let n = (0..a.data.len()).filter(|&k| a.data[k] != b.data[k]).count();
        why.push(format!("data: {n} qwords differ, first {k}: {:08x?} vs {:08x?}", a.data[k], b.data[k]));
    }
    if a.tpc != b.tpc {
        why.push(format!("tpc {:04x} vs {:04x}", a.tpc, b.tpc));
    }
    if a.st != b.st {
        why.push(format!("stats {:?} vs {:?}", a.st, b.st));
    }
}

/// One XGKICK as the GS reads it: the qword address and the packet's qwords up to and including the EOP tag's data
/// (GIF tag lengths: PACKED nloop*nreg, REGLIST (nloop*nreg+1)/2, IMAGE nloop; at most 1024 qwords).
pub fn gif_packet(data: &[[u32; 4]], addr: usize) -> Vec<[u32; 4]> {
    let mut out = Vec::new();
    let mut a = addr & 0x3ff;
    for _ in 0..4096 {
        let t = data[a];
        let nloop = (t[0] & 0x7fff) as usize;
        let eop = (t[0] >> 15) & 1 != 0;
        let flg = (t[1] >> 26) & 3;
        let nreg = ((t[1] >> 28) & 15) as usize;
        let nreg = if nreg == 0 { 16 } else { nreg };
        let len = match flg {
            0 => nloop * nreg,
            1 => (nloop * nreg).div_ceil(2),
            _ => nloop,
        };
        if len + 1 > 1024 {
            out.push([0xdead_beef; 4]);
            break;
        }
        for k in 0..=len {
            out.push(data[(a + k) & 0x3ff]);
        }
        a = (a + len + 1) & 0x3ff;
        if eop {
            break;
        }
    }
    out
}

/// Records every XGKICK packet and optionally forwards it.
struct Tee<'a, S: Path1> {
    inner: Option<&'a mut S>,
    rec: Vec<(usize, Vec<[u32; 4]>)>,
}

impl<S: Path1> Path1 for Tee<'_, S> {
    fn xgkick(&mut self, data: &[[u32; 4]], addr: usize) {
        self.rec.push((addr, gif_packet(data, addr)));
        if let Some(s) = self.inner.as_mut() {
            s.xgkick(data, addr);
        }
    }
}
