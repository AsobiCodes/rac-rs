//! Machine state to bytes and back (fndiff's checkpoint cache). `Cpu::save_dyn` writes everything a run changes except
//! RAM and IOP RAM (the caller deals with those, page by page); `Cpu::load_dyn` puts it back into a Cpu that was loaded
//! from the same savestate (`Machine::load`: stubs, ISO handle and the recompiled table come from there).
use crate::cpu::Cpu;
use std::collections::HashMap;

/// The sources an interpreted run (`Cpu::step` and everything it reaches) is made of. fndiff's checkpoint-cache key
/// hashes them; reading them here ties the hash to the compilation of this crate (a build that races an edit of the
/// sources cannot pair old code with a new hash). Not part of it: gs.rs (only used by the GS capture, off in fndiff),
/// rec/ (the recompiled table is never built there), gate.rs, main.rs.
pub const SRC: [&str; 29] = [
    include_str!("cpu.rs"),
    include_str!("../core/src/fpu.rs"),
    include_str!("hle.rs"),
    include_str!("lib.rs"),
    include_str!("mem.rs"),
    include_str!("vif.rs"),
    include_str!("vu.rs"),
    include_str!("snapfile.rs"),
    include_str!("mc.rs"),
    include_str!("vu0aot.rs"),
    include_str!("vu0_prog0.rs"),
    include_str!("vu0_prog1.rs"),
    include_str!("snd989/driver.rs"),
    include_str!("snd989/handler.rs"),
    include_str!("snd989/voices.rs"),
    include_str!("snd989/streams.rs"),
    include_str!("snd989/lfo.rs"),
    include_str!("snd989/bank.rs"),
    include_str!("spu2.rs"),
    include_str!("snd989/persist.rs"),
    include_str!("snd989/coreinit.rs"),
    include_str!("snd989/irx.rs"),
    include_str!("snd989/pitch.rs"),
    include_str!("snd989/rng.rs"),
    include_str!("snd989/adopt.rs"),
    include_str!("../core/src/cpu.rs"),
    include_str!("../core/src/mem.rs"),
    include_str!("../core/src/vu.rs"),
    include_str!("../core/src/rec.rs"),
];

/// Every field of the structs `save_dyn` / `load_dyn` have to cover: saved ones, ones rebuilt by `Machine::load`, and
/// ones that cannot be set in an fndiff run (`save_dyn` asserts that). `fields_covered` compares these lists with the
/// struct definitions in the source text, so a field added to one of them switches the cache off until this file is
/// updated (a stale cache would otherwise replay a run without that field).
const CPU_FIELDS: &[&str] = &[
    "r", "hi", "lo", "sa", "pc", "npc", "cp0", "f", "facc", "fcr31", "vf", "vacc", "vi", "mem", "icount", "stop",
    "stub_bits", "stubs", "stub_hits", "syscalls", "sema_next", "div_csr", "ring", "ri", "vu_calls", "vsync", "irqs",
    "vsync_armed", "watch", "watch_hits", "trace_call", "trace_left", "trace_pend", "snd", "pad", "iop",
    "iso", "rec", "vblank_cb", "rec_unwind", "rec_flush", "rec_catch", "vu0_stats", "vu0_trace", "mc", "vu0_aot", "skinff",
];
const MEM_FIELDS: &[&str] = &[
    "ram", "spr", "vu0m", "vu0m_gen", "vu0d", "vu1m", "vu1d", "hw", "gs", "rcnt1", "hw_reads", "hw_writes", "dma_kicks",
    "pc", "kick_pcs", "fault", "gs_field", "now", "irq_dirty", "due", "dma_delay", "vif1_pend", "vif1_stops", "vif0",
    "gsc", "gsq",
];
const SND_FIELDS: &[&str] = &["inner", "record"];
const INNER_FIELDS: &[&str] = &["banks", "drv", "streams", "spu_mem", "bank_mem", "stream_mem", "core", "rev", "vags", "record", "tick_off", "vb", "tick_off_vb", "out", "notes"];
const VIF_FIELDS: &[&str] = &["cl", "wl", "mode", "mask", "row", "col", "mpg_uploads", "unpacks"];
/// `gen` is a change counter, not machine state (`load_dyn` gives it a new number).
const MC_FIELDS: &[&str] = &["loaded", "cards", "fds", "list", "list_pos", "gen"];
const CARD_FIELDS: &[&str] = &["root", "free", "cluster", "formatted"];
const ENT_FIELDS: &[&str] = &["name", "mode", "len", "created", "modified", "attr", "data", "kids"];
const FD_FIELDS: &[&str] = &["port", "path", "pos"];

/// The field names of `[pub] struct <name> {` in `src`.
fn struct_fields(src: &str, name: &str) -> Vec<String> {
    let head = format!("struct {name} {{");
    let head_pub = format!("pub struct {name} {{");
    let mut out = Vec::new();
    let mut inside = false;
    for line in src.lines() {
        if !inside {
            inside = line.starts_with(&head) || line.starts_with(&head_pub);
            continue;
        }
        if line.starts_with('}') {
            break;
        }
        let t = line.trim_start();
        if t.is_empty() || t.starts_with("//") || t.starts_with('#') {
            continue;
        }
        let t = t.strip_prefix("pub(crate) ").or_else(|| t.strip_prefix("pub ")).unwrap_or(t);
        if let Some((n, _)) = t.split_once(':') {
            if !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                out.push(n.to_string());
            }
        }
    }
    out
}

/// Ok when the struct definitions in SRC list exactly the fields `save_dyn` was written for.
pub fn fields_covered() -> Result<(), String> {
    for (src, name, want) in [
        (SRC[0], "Cpu", CPU_FIELDS),
        (SRC[4], "Mem", MEM_FIELDS),
        (SRC[11], "Snd989", SND_FIELDS),
        (SRC[11], "Inner", INNER_FIELDS),
        (SRC[5], "Vif", VIF_FIELDS),
        (SRC[8], "Mc", MC_FIELDS),
        (SRC[8], "Card", CARD_FIELDS),
        (SRC[8], "Ent", ENT_FIELDS),
        (SRC[8], "Fd", FD_FIELDS),
    ] {
        let mut have = struct_fields(src, name);
        let mut want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        have.sort();
        want.sort();
        if have != want {
            let new: Vec<&String> = have.iter().filter(|x| !want.contains(x)).collect();
            let gone: Vec<&String> = want.iter().filter(|x| !have.contains(x)).collect();
            return Err(format!("{name} fields changed (new {new:?}, gone {gone:?}): update snapfile.rs save_dyn/load_dyn and its field lists"));
        }
    }
    Ok(())
}

pub struct Wr(pub Vec<u8>, pub Vec<(usize, &'static str)>);

impl Wr {
    pub fn new() -> Wr {
        Wr(Vec::new(), Vec::new())
    }
    /// Names the section that starts here (diagnostics; sections named `stat_*` hold counters and the PC ring that
    /// no emulated behaviour reads, which a state comparison leaves out).
    pub fn mark(&mut self, name: &'static str) {
        self.1.push((self.0.len(), name));
    }
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.0.push(v as u8);
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u128(&mut self, v: u128) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.u64(b.len() as u64);
        self.0.extend_from_slice(b);
    }
    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    pub fn opt_str(&mut self, s: &Option<String>) {
        match s {
            Some(s) => {
                self.bool(true);
                self.str(s);
            }
            None => self.bool(false),
        }
    }
    pub fn u32s(&mut self, v: &[u32]) {
        self.u64(v.len() as u64);
        for &x in v {
            self.u32(x);
        }
    }
    /// A map sorted by key (the file does not depend on the hash order).
    pub fn map<K: Copy + Ord, V: Copy>(&mut self, m: &HashMap<K, V>, key: impl Fn(&mut Wr, K), val: impl Fn(&mut Wr, V)) {
        let mut v: Vec<(K, V)> = m.iter().map(|(k, v)| (*k, *v)).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        self.u64(v.len() as u64);
        for (k, x) in v {
            key(self, k);
            val(self, x);
        }
    }
}

pub struct Rd<'a> {
    pub b: &'a [u8],
    pub p: usize,
}

impl<'a> Rd<'a> {
    pub fn new(b: &'a [u8]) -> Rd<'a> {
        Rd { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> &'a [u8] {
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        s
    }
    pub fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }
    pub fn bool(&mut self) -> bool {
        self.u8() != 0
    }
    pub fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    pub fn i32(&mut self) -> i32 {
        i32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    pub fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take(8).try_into().unwrap())
    }
    pub fn u128(&mut self) -> u128 {
        u128::from_le_bytes(self.take(16).try_into().unwrap())
    }
    pub fn f64(&mut self) -> f64 {
        f64::from_bits(self.u64())
    }
    pub fn bytes(&mut self) -> &'a [u8] {
        let n = self.u64() as usize;
        self.take(n)
    }
    pub fn str(&mut self) -> String {
        String::from_utf8(self.bytes().to_vec()).unwrap()
    }
    pub fn opt_str(&mut self) -> Option<String> {
        if self.bool() { Some(self.str()) } else { None }
    }
    pub fn u32s(&mut self) -> Vec<u32> {
        let n = self.u64() as usize;
        (0..n).map(|_| self.u32()).collect()
    }
    pub fn map<K: std::hash::Hash + Eq, V>(&mut self, key: impl Fn(&mut Rd) -> K, val: impl Fn(&mut Rd) -> V) -> HashMap<K, V> {
        let n = self.u64() as usize;
        let mut m = HashMap::new();
        for _ in 0..n {
            let k = key(self);
            let v = val(self);
            m.insert(k, v);
        }
        m
    }
    pub fn done(&self) -> bool {
        self.p == self.b.len()
    }
}

impl Cpu {
    /// Everything but RAM and IOP RAM. Panics for state the files cannot hold (a stop message, the sound event
    /// recorder, a GS capture): none of those exists in an fndiff run.
    pub fn save_dyn(&self, w: &mut Wr) {
        assert!(self.stop.is_none() && !self.snd.record && self.mem.gsc.is_none() && self.mem.gsq.is_none(), "state not saveable");
        assert!(self.rec.is_none() && self.vblank_cb.is_none(), "recompiled-table state not saveable");
        assert!(self.vu0_trace.is_none(), "VU0 trace not saveable");
        w.mark("regs");
        for &x in &self.r {
            w.u128(x);
        }
        w.u128(self.hi);
        w.u128(self.lo);
        for v in [self.sa, self.pc, self.npc] {
            w.u32(v);
        }
        for &x in &self.cp0 {
            w.u32(x);
        }
        for &x in &self.f {
            w.u32(x);
        }
        w.u32(self.facc);
        w.u32(self.fcr31);
        for v in &self.vf {
            for &x in v {
                w.u32(x);
            }
        }
        for &x in &self.vacc {
            w.u32(x);
        }
        for &x in &self.vi {
            w.u32(x);
        }
        w.mark("icount");
        w.u64(self.icount);
        w.mark("stat_stub_hits");
        w.map(&self.stub_hits, |w, k| w.u32(k), |w, v| w.u64(v));
        w.mark("stat_syscalls");
        w.map(&self.syscalls, |w, k| w.i32(k), |w, v| w.u64(v));
        w.mark("sema");
        w.u32(self.sema_next);
        w.u32(self.div_csr);
        w.mark("stat_ring");
        for &x in &self.ring {
            w.u32(x);
        }
        w.u64(self.ri as u64);
        w.mark("ring_end");
        w.mark("stat_vu_calls");
        w.map(&self.vu_calls, |w, k| w.u32(k), |w, v| w.u64(v));
        w.mark("stat_vu0");
        for &x in &self.vu0_stats {
            w.u64(x);
        }
        w.mark("vsync");
        match self.vsync {
            Some((a, b)) => {
                w.bool(true);
                w.u32(a);
                w.u32(b);
            }
            None => w.bool(false),
        }
        for &x in &self.irqs {
            w.u64(x);
        }
        w.bool(self.vsync_armed);
        w.mark("stat_watch_hits");
        w.u64(self.watch_hits.len() as u64);
        for (&k, &v) in &self.watch_hits {
            w.u32(k);
            w.u64(v);
        }
        w.u32(self.trace_left);
        w.mark("snd");
        w.bytes(&self.snd.save());
        w.mark("mc");
        self.mc.save(w);
        w.mark("pad");
        match self.pad {
            Some(p) => {
                w.bool(true);
                w.0.extend_from_slice(&p);
            }
            None => w.bool(false),
        }
        // Bookkeeping of the recompiled-code dispatcher: nothing fndiff's interpreter loop does reads it (no table is
        // built there), but a hand's nested call (`call_dyn` -> `step_rec` -> `rec_flushed`) clears rec_flush where the
        // unsampled walk leaves it set, so a state comparison must not count it.
        w.mark("stat_rec");
        w.bool(self.rec_unwind);
        w.bool(self.rec_flush);
        w.bool(self.rec_catch);
        w.mark("membufs");
        let m = &self.mem;
        for b in [&m.spr, &m.vu0m, &m.vu0d, &m.vu1m, &m.vu1d, &m.hw, &m.gs] {
            w.bytes(b);
        }
        w.mark("rcnt1");
        w.u32(m.rcnt1);
        w.mark("stat_hw_reads");
        w.map(&m.hw_reads, |w, k| w.u32(k), |w, v| w.u64(v));
        w.mark("stat_hw_writes");
        w.map(&m.hw_writes, |w, k| w.u32(k), |w, v| w.u64(v));
        w.mark("stat_dma_kicks");
        w.map(&m.dma_kicks, |w, k| w.u32(k), |w, v| w.u64(v));
        w.mark("mem.pc");
        w.u32(m.pc);
        w.mark("stat_kick_pcs");
        w.map(&m.kick_pcs, |w, k| { w.u32(k.0); w.u32(k.1) }, |w, v| w.u64(v));
        w.mark("fault");
        w.opt_str(&m.fault);
        w.u32(m.gs_field);
        w.mark("now");
        w.u64(m.now);
        w.bool(m.irq_dirty);
        w.u64(m.due);
        w.u64(m.dma_delay);
        match m.vif1_pend {
            Some(p) => {
                w.bool(true);
                for x in p {
                    w.u32(x);
                }
            }
            None => w.bool(false),
        }
        w.u64(m.vif1_stops);
        w.mark("vif0");
        m.vif0.save(w);
    }

    /// Inverse of `save_dyn`.
    pub fn load_dyn(&mut self, r: &mut Rd) {
        for x in self.r.iter_mut() {
            *x = r.u128();
        }
        self.hi = r.u128();
        self.lo = r.u128();
        self.sa = r.u32();
        self.pc = r.u32();
        self.npc = r.u32();
        for x in self.cp0.iter_mut() {
            *x = r.u32();
        }
        for x in self.f.iter_mut() {
            *x = r.u32();
        }
        self.facc = r.u32();
        self.fcr31 = r.u32();
        for v in self.vf.iter_mut() {
            for x in v.iter_mut() {
                *x = r.u32();
            }
        }
        for x in self.vacc.iter_mut() {
            *x = r.u32();
        }
        for x in self.vi.iter_mut() {
            *x = r.u32();
        }
        self.icount = r.u64();
        self.stub_hits = r.map(|r| r.u32(), |r| r.u64());
        self.syscalls = r.map(|r| r.i32(), |r| r.u64());
        self.sema_next = r.u32();
        self.div_csr = r.u32();
        for x in self.ring.iter_mut() {
            *x = r.u32();
        }
        self.ri = r.u64() as usize;
        self.vu_calls = r.map(|r| r.u32(), |r| r.u64());
        for x in self.vu0_stats.iter_mut() {
            *x = r.u64();
        }
        self.vsync = if r.bool() { Some((r.u32(), r.u32())) } else { None };
        for x in self.irqs.iter_mut() {
            *x = r.u64();
        }
        self.vsync_armed = r.bool();
        self.watch_hits.clear();
        for _ in 0..r.u64() {
            let k = r.u32();
            let v = r.u64();
            self.watch_hits.insert(k, v);
        }
        self.trace_left = r.u32();
        self.snd.load(r.bytes());
        self.mc.load_dyn(r);
        self.pad = if r.bool() {
            let mut p = [0u8; 18];
            p.copy_from_slice(r.take(18));
            Some(p)
        } else {
            None
        };
        self.rec_unwind = r.bool();
        self.rec_flush = r.bool();
        self.rec_catch = r.bool();
        let m = &mut self.mem;
        for b in [&mut m.spr, &mut m.vu0m, &mut m.vu0d, &mut m.vu1m, &mut m.vu1d, &mut m.hw, &mut m.gs] {
            let s = r.bytes();
            assert_eq!(s.len(), b.len());
            b.copy_from_slice(s);
        }
        // Micro memory was replaced: the native prog0 verifies it again (`vu0_aot` is not saved, its verdict is
        // tied to this counter; `vu0m_gen` itself is not saved either).
        m.vu0m_changed();
        m.rcnt1 = r.u32();
        m.hw_reads = r.map(|r| r.u32(), |r| r.u64());
        m.hw_writes = r.map(|r| r.u32(), |r| r.u64());
        m.dma_kicks = r.map(|r| r.u32(), |r| r.u64());
        m.pc = r.u32();
        m.kick_pcs = r.map(|r| (r.u32(), r.u32()), |r| r.u64());
        m.fault = r.opt_str();
        m.gs_field = r.u32();
        m.now = r.u64();
        m.irq_dirty = r.bool();
        m.due = r.u64();
        m.limit = if m.irq_dirty { 0 } else { m.due };
        m.dma_delay = r.u64();
        m.vif1_pend = if r.bool() { Some([r.u32(), r.u32(), r.u32(), r.u32(), r.u32()]) } else { None };
        m.vif1_stops = r.u64();
        m.vif0.load(r);
        self.stop = None;
    }
}
