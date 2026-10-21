//! EE address space: 32 MB RAM (+ uncached mirrors and kseg0/1), scratchpad,
//! VU memories, hardware registers and GS privileged registers.
//! Most DMA channels finish as soon as they start, and only scratchpad DMA
//! and VIF0 (see vif.rs) move data (game logic depends on them, rendering
//! does not). VIF1 walks its
//! tag chain without moving data and completes `dma_delay` instructions after
//! the kick, so the game's DMAC handler sees the real stop points (IRQ tags).

use std::collections::HashMap;

/// [PC] (speed) Whether the report counters count: `Mem::hw_reads` / `hw_writes` / `dma_kicks` / `kick_pcs` and
/// `Cpu::stub_hits` / `vu_calls`. Only reports read them (the gate tool, examples coldboot / gsstat / travel, which switch it
/// on); in the game the hash-map update on every hardware access, stub call and VU0 micro call was pure overhead.
pub static STATS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[inline(always)]
pub fn stats() -> bool {
    STATS.load(std::sync::atomic::Ordering::Relaxed)
}

pub use ee_core::mem::RAM;

/// T1_COUNT counts in one PAL field (BUSCLK / 256 / 50).
pub const T1_FIELD: u32 = 11520;

/// `#[repr(C)]`: the first fields are ee-core's `Mem` (the generated code's memory fast path).
#[repr(C)]
pub struct Mem {
    pub ram: Vec<u8>,
    pub spr: Vec<u8>,
    pub vu0d: Vec<u8>,
    /// Debug: log CPU writes into VU0 data memory too (env COLD_WATCH_VU0D); `vu0d_cpu_writes` counts them (all of
    /// them while the log is on; otherwise recompiled code stores there directly, see rec/mod.rs `fbuf`).
    pub watch_vu: bool,
    /// An interrupt line may be asserted: the CPU re-tests INTC/DMAC.
    pub irq_dirty: bool,
    /// When the pending VIF1 stop becomes visible (u64::MAX: none).
    pub due: u64,
    /// Poll limit: 0 while `irq_dirty`, else `due` (the generated code's block-entry test is `icount + n <= limit`).
    /// Written only through `mark_dirty`, `clear_dirty`, `set_due`.
    pub limit: u64,
    pub vu0m: Vec<u8>,
    /// Changes (`vu0m_changed`) with every write to VU0 micro memory: VIF0 MPG, EE stores, state loads and restores;
    /// code that writes `vu0m` directly calls `vu0m_changed` itself. Not machine state (never saved).
    pub vu0m_gen: u32,
    pub vu1m: Vec<u8>,
    pub vu1d: Vec<u8>,
    pub hw: Vec<u8>,
    pub gs: Vec<u8>,
    /// Value returned for T1_COUNT (frame-time meter, long-frame catch-up test): 0x1000 after a write (the time
    /// within a field), plus `T1_FIELD` for every vblank since.
    pub rcnt1: u32,
    pub hw_reads: HashMap<u32, u64>,
    pub hw_writes: HashMap<u32, u64>,
    pub dma_kicks: HashMap<u32, u64>,
    /// PC of the instruction being executed (set by the CPU).
    pub pc: u32,
    pub kick_pcs: HashMap<(u32, u32), u64>,
    pub fault: Option<String>,
    /// Debug: scratchpad byte range (lo, hi) whose writes (CPU and SPR DMA) are logged into `watch_log` (last 64).
    pub watch: Option<(usize, usize)>,
    pub watch_log: std::collections::VecDeque<String>,
    pub vu0d_cpu_writes: u64,
    pub gs_field: u32,
    /// Instruction count (set by the CPU every step).
    pub now: u64,
    /// Cold boot field timer (EE instruction count of the next forced vblank; u64::MAX: none, savestate runs).
    pub tick: u64,
    /// When the pending VIF1 stop (`vif1_pend`) is due; `due` is the earlier of this and `tick`.
    pub dma_due: u64,
    pub dma_delay: u64,
    /// The transfer behind `due` (set together with it, taken when it matures).
    pub vif1_pend: Option<[u32; 5]>,
    pub vif1_stops: u64,
    pub vif0: crate::vif::Vif,
    /// GS capture (VIF1 DIRECT and GIF DMA packets), off unless enabled.
    pub gsc: Option<Box<crate::gs::Gs>>,
    /// The GS capture on its own thread (`Machine::capture_gs_async`): the packets go there instead.
    pub gsq: Option<std::sync::mpsc::Sender<crate::gs::Job>>,
    /// [PC] (speed) Words the last captured VIF1 chain copied: the next one's buffer starts that large instead of
    /// growing (and recopying) from empty. Only a capacity, never state.
    pub vif1_words: usize,
}

pub const INTC_STAT: u32 = 0x1000_f000;
pub const INTC_MASK: u32 = 0x1000_f010;
pub const D_STAT: u32 = 0x1000_e010;
const VIF1: u32 = 0x1000_9000;

enum Loc {
    Ram(usize),
    Spr(usize),
    Hw(u32),
    Vu(u8, usize),
    Gs(usize),
    Rom,
    Bad,
}

#[inline(always)]
/// [PC] (speed) The scratchpad offset of an `n`-byte access at `a` that lies wholly inside it (`map`'s `Loc::Spr`).
#[inline(always)]
fn spr_at(a: u32, n: usize) -> Option<usize> {
    let i = (a & 0x3fff) as usize;
    (a & 0xffff_c000 == 0x7000_0000 && i + n <= 0x4000).then_some(i)
}

fn map(a: u32) -> Loc {
    if a & 0xf000_0000 == 0x7000_0000 {
        return if a & 0x0fff_ffff < 0x4000 { Loc::Spr((a & 0x3fff) as usize) } else { Loc::Bad };
    }
    let p = if (0x8000_0000..0xc000_0000).contains(&a) { a & 0x1fff_ffff } else { a };
    match p {
        0..=0x01ff_ffff => Loc::Ram(p as usize),
        0x2000_0000..=0x21ff_ffff | 0x3000_0000..=0x31ff_ffff => Loc::Ram((p & 0x01ff_ffff) as usize),
        0x1000_0000..=0x1000_ffff => Loc::Hw(p),
        0x1100_0000..=0x1100_ffff => {
            let o = (p & 0xffff) as usize;
            match o >> 14 {
                0 => Loc::Vu(0, o & 0xfff),
                1 => Loc::Vu(1, o & 0xfff),
                2 => Loc::Vu(2, o & 0x3fff),
                _ => Loc::Vu(3, o & 0x3fff),
            }
        }
        0x1200_0000..=0x1200_1fff => Loc::Gs((p & 0x1fff) as usize),
        0x1fc0_0000..=0x1fff_ffff => Loc::Rom,
        _ => Loc::Bad,
    }
}

/// Write-watched RAM for the picture thread's mirror (speed lane A, rac-rs `retail/snap_pipe.rs`).
///
/// `watch_next()` makes the next `Mem::new` allocate its RAM with `VirtualAlloc(MEM_WRITE_WATCH)`; the OS then records which
/// 4 KB pages any write touched (CPU stores of the recompiled code, `memcpy`, file reads into the buffer), and `Mem::mirror_to`
/// copies exactly those pages into a second machine. The Vec is built from the VirtualAlloc pointer: it must never be dropped
/// or grown (the live machine's `Cpu` is leaked in a `ManuallyDrop`, see `Machine`). Without Windows, or when the flag is not
/// set, RAM is an ordinary Vec and `mirror_to` copies it whole.
pub mod watch {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    static NEXT: AtomicBool = AtomicBool::new(false);
    /// Start address of the write-watched RAM (0: none).
    pub static BASE: AtomicUsize = AtomicUsize::new(0);

    /// The next `Mem::new` gets write-watched RAM (consumed by it).
    pub fn watch_next() {
        NEXT.store(true, Ordering::SeqCst);
    }

    #[cfg(windows)]
    mod sys {
        extern "system" {
            pub fn VirtualAlloc(addr: *mut u8, size: usize, ty: u32, prot: u32) -> *mut u8;
            pub fn GetWriteWatch(flags: u32, base: *mut u8, size: usize, addrs: *mut *mut u8, count: *mut usize, gran: *mut u32) -> u32;
            pub fn GetModuleHandleA(name: *const u8) -> *mut u8;
            pub fn GetProcAddress(module: *mut u8, name: *const u8) -> *mut u8;
        }
    }

    /// Running under Wine / Proton (the Windows exe on Linux, the Steam Deck): ntdll exports `wine_get_version` there.
    /// Wine emulates MEM_WRITE_WATCH with page protection and a fault on the first write to each watched page, so on Wine the
    /// write watch costs the EE thread about one fault per dirty page and field: not worth it, see `available`.
    pub fn is_wine() -> bool {
        #[cfg(windows)]
        unsafe {
            let h = sys::GetModuleHandleA(b"ntdll.dll\0".as_ptr());
            return !h.is_null() && !sys::GetProcAddress(h, b"wine_get_version\0".as_ptr()).is_null();
        }
        #[allow(unreachable_code)]
        false
    }

    /// The OS write watch is cheap here: native Windows only (not Wine, not Linux/macOS builds).
    pub fn available() -> bool {
        cfg!(windows) && !is_wine()
    }

    pub(super) fn new_ram() -> Vec<u8> {
        #[cfg(windows)]
        if NEXT.swap(false, Ordering::SeqCst) {
            // MEM_COMMIT | MEM_RESERVE | MEM_WRITE_WATCH, PAGE_READWRITE: zeroed, like `vec![0; RAM]`.
            let p = unsafe { sys::VirtualAlloc(std::ptr::null_mut(), super::RAM, 0x1000 | 0x2000 | 0x20_0000, 4) };
            if !p.is_null() {
                BASE.store(p as usize, Ordering::SeqCst);
                return unsafe { Vec::from_raw_parts(p, super::RAM, super::RAM) };
            }
        }
        vec![0; super::RAM]
    }

    /// The 4 KB page indexes of `ram` written since the last call (or since `new_ram`), and the write watch is reset.
    /// None: this RAM is not write-watched (or the call failed): the caller copies everything.
    #[allow(unused_variables)]
    pub fn dirty_pages(ram: &[u8]) -> Option<Vec<u32>> {
        #[cfg(windows)]
        {
            if ram.as_ptr() as usize != BASE.load(Ordering::SeqCst) || ram.len() != super::RAM {
                return None;
            }
            let mut addrs = vec![std::ptr::null_mut::<u8>(); super::RAM / 4096];
            let mut n = addrs.len();
            let mut gran = 0u32;
            // WRITE_WATCH_FLAG_RESET = 1
            let r = unsafe { sys::GetWriteWatch(1, ram.as_ptr() as *mut u8, ram.len(), addrs.as_mut_ptr(), &mut n, &mut gran) };
            if r != 0 || gran != 4096 || n > addrs.len() {
                return None;
            }
            let b = ram.as_ptr() as usize;
            return Some(addrs[..n].iter().map(|&a| ((a as usize - b) / 4096) as u32).collect());
        }
        #[allow(unreachable_code)]
        None
    }
}

impl Mem {
    /// Brings the mirror `dst` to this memory's RAM, scratchpad and GS registers (the only memories the picture path reads).
    /// `fresh`: the first call (or a mirror that was not kept in step): all of RAM is copied, and the write watch is reset
    /// first so the next call starts from this state. Else only the pages written since the previous call are copied
    /// (the whole RAM when the write watch is not available). Returns the RAM bytes copied.
    /// The host must have this machine parked (between `Machine::frame` calls).
    pub fn mirror_to(&self, dst: &mut Mem, fresh: &mut bool) -> usize {
        let pages = watch::dirty_pages(&self.ram);
        dst.spr.copy_from_slice(&self.spr);
        dst.gs.copy_from_slice(&self.gs);
        match pages {
            Some(p) if !*fresh => {
                for &pg in &p {
                    let o = pg as usize * 4096;
                    dst.ram[o..o + 4096].copy_from_slice(&self.ram[o..o + 4096]);
                }
                p.len() * 4096
            }
            _ => {
                dst.ram.copy_from_slice(&self.ram);
                *fresh = false;
                self.ram.len()
            }
        }
    }
}

impl Mem {
    /// Call after writing `vu0m` directly (the native prog0's "this is prog0" verdict is dropped, see vu0aot.rs).
    pub fn vu0m_changed(&mut self) {
        self.vu0m_gen = self.vu0m_gen.wrapping_add(1);
    }

    pub fn new() -> Mem {
        Mem {
            ram: watch::new_ram(),
            spr: vec![0; 0x4000],
            vu0m: vec![0; 0x1000],
            vu0m_gen: 0,
            vu0d: vec![0; 0x1000],
            vu1m: vec![0; 0x4000],
            vu1d: vec![0; 0x4000],
            hw: vec![0; 0x10000],
            gs: vec![0; 0x2000],
            rcnt1: 0x1000,
            hw_reads: HashMap::new(),
            hw_writes: HashMap::new(),
            dma_kicks: HashMap::new(),
            pc: 0,
            kick_pcs: HashMap::new(),
            fault: None,
            watch: None,
            watch_log: Default::default(),
            watch_vu: false,
            vu0d_cpu_writes: 0,
            now: 0,
            irq_dirty: true,
            due: u64::MAX,
            limit: 0,
            tick: u64::MAX,
            dma_due: 0,
            dma_delay: 20_000,
            vif1_pend: None,
            vif1_stops: 0,
            vif0: Default::default(),
            gsc: None,
            gsq: None,
            gs_field: 0,
            vif1_words: 0,
        }
    }

    #[inline(always)]
    pub fn fetch(&mut self, a: u32) -> u32 {
        if a < RAM as u32 {
            let i = a as usize;
            return u32::from_le_bytes(self.ram[i..i + 4].try_into().unwrap());
        }
        self.rd(a, 4) as u32
    }

    #[inline(always)]
    pub fn rd8(&mut self, a: u32) -> u8 {
        if a < RAM as u32 { self.ram[a as usize] } else { self.rd(a, 1) as u8 }
    }
    #[inline(always)]
    pub fn rd16(&mut self, a: u32) -> u16 {
        if a < RAM as u32 - 1 {
            let i = a as usize;
            u16::from_le_bytes(self.ram[i..i + 2].try_into().unwrap())
        } else {
            match spr_at(a, 2) {
                Some(i) => u16::from_le_bytes(self.spr[i..i + 2].try_into().unwrap()),
                None => self.rd(a, 2) as u16,
            }
        }
    }
    #[inline(always)]
    pub fn rd32(&mut self, a: u32) -> u32 {
        if a < RAM as u32 - 3 {
            let i = a as usize;
            u32::from_le_bytes(self.ram[i..i + 4].try_into().unwrap())
        } else {
            match spr_at(a, 4) {
                Some(i) => u32::from_le_bytes(self.spr[i..i + 4].try_into().unwrap()),
                None => self.rd(a, 4) as u32,
            }
        }
    }
    #[inline(always)]
    pub fn rd64(&mut self, a: u32) -> u64 {
        if a < RAM as u32 - 7 {
            let i = a as usize;
            u64::from_le_bytes(self.ram[i..i + 8].try_into().unwrap())
        } else {
            match spr_at(a, 8) {
                Some(i) => u64::from_le_bytes(self.spr[i..i + 8].try_into().unwrap()),
                None => self.rd(a, 8),
            }
        }
    }
    pub fn rd128(&mut self, a: u32) -> u128 {
        let lo = self.rd64(a) as u128;
        let hi = self.rd64(a.wrapping_add(8)) as u128;
        lo | (hi << 64)
    }

    #[inline(always)]
    pub fn wr8(&mut self, a: u32, v: u8) {
        if a < RAM as u32 { self.ram[a as usize] = v } else { self.wr(a, v as u64, 1) }
    }
    #[inline(always)]
    pub fn wr16(&mut self, a: u32, v: u16) {
        if a < RAM as u32 - 1 {
            let i = a as usize;
            self.ram[i..i + 2].copy_from_slice(&v.to_le_bytes());
        } else if let (Some(i), None) = (spr_at(a, 2), self.watch) {
            self.spr[i..i + 2].copy_from_slice(&v.to_le_bytes());
        } else {
            self.wr(a, v as u64, 2)
        }
    }
    #[inline(always)]
    pub fn wr32(&mut self, a: u32, v: u32) {
        if a < RAM as u32 - 3 {
            let i = a as usize;
            self.ram[i..i + 4].copy_from_slice(&v.to_le_bytes());
        } else if let (Some(i), None) = (spr_at(a, 4), self.watch) {
            self.spr[i..i + 4].copy_from_slice(&v.to_le_bytes());
        } else {
            self.wr(a, v as u64, 4)
        }
    }
    #[inline(always)]
    pub fn wr64(&mut self, a: u32, v: u64) {
        if a < RAM as u32 - 7 {
            let i = a as usize;
            self.ram[i..i + 8].copy_from_slice(&v.to_le_bytes());
        } else if let (Some(i), None) = (spr_at(a, 8), self.watch) {
            self.spr[i..i + 8].copy_from_slice(&v.to_le_bytes());
        } else {
            self.wr(a, v, 8)
        }
    }
    pub fn wr128(&mut self, a: u32, v: u128) {
        self.wr64(a, v as u64);
        self.wr64(a.wrapping_add(8), (v >> 64) as u64);
    }

    fn slice(&mut self, loc: &Loc) -> Option<(&mut Vec<u8>, usize)> {
        match *loc {
            Loc::Ram(i) => Some((&mut self.ram, i)),
            Loc::Spr(i) => Some((&mut self.spr, i)),
            Loc::Vu(0, i) => Some((&mut self.vu0m, i)),
            Loc::Vu(1, i) => Some((&mut self.vu0d, i)),
            Loc::Vu(2, i) => Some((&mut self.vu1m, i)),
            Loc::Vu(_, i) => Some((&mut self.vu1d, i)),
            Loc::Gs(i) => Some((&mut self.gs, i)),
            _ => None,
        }
    }

    fn rd(&mut self, a: u32, n: usize) -> u64 {
        // [PC] (speed) Scratchpad first: the same bytes as the `Loc::Spr` arm below, without the address decode.
        if a & 0xffff_c000 == 0x7000_0000 {
            let i = (a & 0x3fff) as usize;
            let mut b = [0u8; 8];
            let e = (i + n).min(self.spr.len());
            b[..e - i].copy_from_slice(&self.spr[i..e]);
            return u64::from_le_bytes(b);
        }
        let loc = map(a);
        match loc {
            Loc::Hw(p) => self.hw_rd(p, n),
            Loc::Gs(o) if (o & !7) == 0x1000 => {
                // CSR: FINISH and VSINT set, FIELD toggles per read.
                self.gs_field ^= 0x2000;
                (0x0000_000a | self.gs_field) as u64
            }
            Loc::Rom => 0,
            Loc::Bad => {
                if self.fault.is_none() {
                    self.fault = Some(format!("read{} from unmapped {:08x}", n * 8, a));
                }
                0
            }
            _ => {
                let (buf, i) = self.slice(&loc).unwrap();
                let mut b = [0u8; 8];
                let e = (i + n).min(buf.len());
                b[..e - i].copy_from_slice(&buf[i..e]);
                u64::from_le_bytes(b)
            }
        }
    }

    fn wr(&mut self, a: u32, v: u64, n: usize) {
        if a & 0xffff_c000 == 0x7000_0000 && self.watch.is_none() {
            let i = (a & 0x3fff) as usize;
            let e = (i + n).min(self.spr.len());
            self.spr[i..e].copy_from_slice(&v.to_le_bytes()[..e - i]);
            return;
        }
        let loc = map(a);
        match loc {
            Loc::Hw(p) => self.hw_wr(p, v, n),
            Loc::Rom => {}
            Loc::Bad => {
                if self.fault.is_none() {
                    self.fault = Some(format!("write{} to unmapped {:08x}", n * 8, a));
                }
            }
            _ => {
                if let Loc::Vu(0, _) = loc {
                    self.vu0m_changed();
                }
                if let Loc::Vu(1, i) = loc {
                    self.vu0d_cpu_writes += 1;
                    if self.watch_vu {
                        self.watch_note(format!("cpu pc {:08x} vu0d {:03x} n {} val {:x} @{}", self.pc, i, n, v, self.now));
                    }
                }
                if let (Loc::Spr(i), Some((lo, hi))) = (&loc, self.watch) {
                    if *i < hi && *i + n > lo {
                        self.watch_note(format!("cpu pc {:08x} spr {:04x} n {} val {:x} @{}", self.pc, i, n, v, self.now));
                    }
                }
                let (buf, i) = self.slice(&loc).unwrap();
                let e = (i + n).min(buf.len());
                buf[i..e].copy_from_slice(&v.to_le_bytes()[..e - i]);
            }
        }
    }

    pub fn hw32(&self, p: u32) -> u32 {
        let o = (p & 0xfffc) as usize;
        u32::from_le_bytes(self.hw[o..o + 4].try_into().unwrap())
    }
    pub fn set_hw32(&mut self, p: u32, v: u32) {
        let o = (p & 0xfffc) as usize;
        self.hw[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn hw_rd(&mut self, p: u32, n: usize) -> u64 {
        if stats() {
            *self.hw_reads.entry(p).or_insert(0) += 1;
        }
        let v = match p & !3 {
            0x1000_0800 => self.rcnt1,
            0x1000_0000 | 0x1000_1000 | 0x1000_1800 => (self.now as u32 >> 1) & 0xffff,
            0x1000_3020 | 0x1000_3800 | 0x1000_3c00 => 0,
            0x1000_8000 | 0x1000_a000 | 0x1000_b000 | 0x1000_b400 | 0x1000_c000 | 0x1000_c400
            | 0x1000_c800 | 0x1000_d000 | 0x1000_d400 => self.hw32(p) & !0x100,
            _ => {
                let o = (p & 0xffff) as usize;
                let mut b = [0u8; 8];
                let e = (o + n).min(0x10000);
                b[..e - o].copy_from_slice(&self.hw[o..e]);
                return u64::from_le_bytes(b);
            }
        };
        v as u64
    }

    fn hw_wr(&mut self, p: u32, v: u64, n: usize) {
        if stats() {
            *self.hw_writes.entry(p).or_insert(0) += 1;
        }
        match p & !3 {
            0x1000_0800 => self.rcnt1 = 0x1000,
            0x1000_e010 => {
                let s = self.hw32(p);
                let v = v as u32;
                self.set_hw32(p, (s & !(v & 0xffff)) ^ (v & 0xffff_0000));
                self.mark_dirty();
            }
            0x1000_f000 => {
                let s = self.hw32(p);
                self.set_hw32(p, s & !(v as u32));
                self.mark_dirty();
            }
            0x1000_f010 => {
                let s = self.hw32(p);
                self.set_hw32(p, s ^ (v as u32));
                self.mark_dirty();
            }
            0x1000_8000 | 0x1000_9000 | 0x1000_a000 | 0x1000_b000 | 0x1000_b400 | 0x1000_c000
            | 0x1000_c400 | 0x1000_c800 | 0x1000_d000 | 0x1000_d400 => {
                self.set_hw32(p, v as u32);
                if v & 0x100 != 0 {
                    if stats() {
                        *self.dma_kicks.entry(p).or_insert(0) += 1;
                        *self.kick_pcs.entry((p, self.pc)).or_insert(0) += 1;
                    }
                    let ch = match p {
                        0x1000_8000 => 0,
                        0x1000_9000 => 1,
                        0x1000_a000 => 2,
                        0x1000_b000 => 3,
                        0x1000_b400 => 4,
                        0x1000_c000 => 5,
                        0x1000_c400 => 6,
                        0x1000_c800 => 7,
                        0x1000_d000 => 8,
                        _ => 9,
                    };
                    match ch {
                        0 => self.vif0_kick(),
                        1 => return self.vif1_kick(),
                        2 => self.gif_kick(),
                        8 => self.spr_dma(false),
                        9 => self.spr_dma(true),
                        _ => {}
                    }
                    let c = self.hw32(p);
                    self.set_hw32(p, c & !0x100);
                    // SIF0/SIF2 are fed by the IOP, which is not emulated.
                    if ch != 5 && ch != 7 {
                        let s = self.hw32(D_STAT);
                        self.set_hw32(D_STAT, s | 1 << ch);
                        self.mark_dirty();
                    }
                }
            }
            _ => {
                let o = (p & 0xffff) as usize;
                let e = (o + n).min(0x10000);
                self.hw[o..e].copy_from_slice(&v.to_le_bytes()[..e - o]);
            }
        }
    }

    pub fn watch_note(&mut self, line: String) {
        static CAP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        let cap = *CAP.get_or_init(|| std::env::var("COLD_WATCH_N").ok().and_then(|v| v.parse().ok()).unwrap_or(64));
        if self.watch_log.len() >= cap {
            self.watch_log.pop_front();
        }
        self.watch_log.push_back(line);
    }

    /// Debug (env COLD_SPRLOG=FROM:CAP): log SPR DMA transfers from icount FROM, at most CAP lines.
    // [PC] (speed) The line is built only when it prints (it was formatted for every transfer).
    fn sprlog(&self, line: impl FnOnce() -> String) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CFG: std::sync::OnceLock<(u64, u64)> = std::sync::OnceLock::new();
        static N: AtomicU64 = AtomicU64::new(0);
        let (from, cap) = *CFG.get_or_init(|| {
            std::env::var("COLD_SPRLOG")
                .ok()
                .and_then(|v| v.split_once(':').map(|(a, b)| (a.parse().unwrap_or(u64::MAX), b.parse().unwrap_or(0))))
                .unwrap_or((u64::MAX, 0))
        });
        if self.now >= from && N.fetch_add(1, Ordering::Relaxed) < cap {
            eprintln!("sprdma {} pc {:08x} @{}", line(), self.pc, self.now);
        }
    }

    /// Scratchpad DMA (SPR_TO: RAM -> scratchpad, SPR_FROM: scratchpad -> RAM).
    /// Normal mode, plus source chain mode for SPR_TO.
    fn spr_dma(&mut self, to: bool) {
        let base = if to { 0x1000_d400 } else { 0x1000_d000 };
        let chcr = self.hw32(base);
        let mode = (chcr >> 2) & 3;
        let mut sadr = self.hw32(base + 0x80) & 0x3ff0;
        let copy = |m: &mut Mem, madr: u32, sadr: u32, qwc: u32| {
            // [PC] (speed) Without a watch: the same quadwords as the loop below, copied in runs up to the scratchpad's
            // wrap (in order, so a transfer above 16 KB overwrites the same way). The game moves ~1500 blocks a field.
            if m.watch.is_none() {
                let base = (madr & 0x01ff_fff0) as usize;
                let mut left = (qwc as usize).min(RAM.saturating_sub(base) / 16);
                let (mut ra, mut sa) = (base, (sadr & 0x3ff0) as usize);
                while left > 0 {
                    let k = left.min((0x4000 - sa) / 16);
                    if to {
                        m.spr[sa..sa + k * 16].copy_from_slice(&m.ram[ra..ra + k * 16]);
                    } else {
                        m.ram[ra..ra + k * 16].copy_from_slice(&m.spr[sa..sa + k * 16]);
                    }
                    (ra, sa, left) = (ra + k * 16, 0, left - k);
                }
                return;
            }
            for q in 0..qwc {
                let ra = (madr & 0x01ff_fff0).wrapping_add(q * 16) as usize;
                let sa = ((sadr + q * 16) & 0x3ff0) as usize;
                if ra + 16 > RAM {
                    break;
                }
                if let Some((lo, hi)) = m.watch {
                    if to && sa < hi && sa + 16 > lo {
                        m.watch_note(format!("dma-to-spr spr {:04x} from ram {:08x} {:08x?} (pc {:08x}) @{}", sa, ra, &m.ram[ra..ra + 16], m.pc, m.now));
                    }
                }
                if to {
                    let (src, dst) = (&m.ram[ra..ra + 16], sa);
                    let tmp: [u8; 16] = src.try_into().unwrap();
                    m.spr[dst..dst + 16].copy_from_slice(&tmp);
                } else {
                    let tmp: [u8; 16] = m.spr[sa..sa + 16].try_into().unwrap();
                    m.ram[ra..ra + 16].copy_from_slice(&tmp);
                }
            }
        };
        if mode == 0 || !to {
            if mode != 0 && self.fault.is_none() {
                self.fault = Some(format!("SPR_FROM chain mode chcr {:08x}", chcr));
            }
            let madr = self.hw32(base + 0x10);
            let qwc = self.hw32(base + 0x20) & 0xffff;
            self.sprlog(|| format!("{} normal chcr {chcr:08x} madr {madr:08x} sadr {sadr:04x} qwc {qwc}", if to { "to" } else { "from" }));
            copy(self, madr, sadr, qwc);
            self.set_hw32(base + 0x10, madr.wrapping_add(qwc * 16));
            self.set_hw32(base + 0x80, (sadr + qwc * 16) & 0x3ff0);
            self.set_hw32(base + 0x20, 0);
            return;
        }
        // Source chain (SPR_TO).
        let tte = chcr & 0x40 != 0;
        let mut tadr = self.hw32(base + 0x30);
        let mut asr = [0u32; 2];
        let mut asp = 0usize;
        for _ in 0..4096 {
            let tag = self.rd64(tadr & 0x01ff_fff0);
            let qwc = (tag & 0xffff) as u32;
            let id = ((tag >> 28) & 7) as u32;
            let addr = ((tag >> 32) as u32) & 0x7fff_fff0;
            if tte {
                let t = self.rd128(tadr & 0x01ff_fff0);
                let sa = sadr as usize & 0x3ff0;
                self.spr[sa..sa + 16].copy_from_slice(&t.to_le_bytes());
                sadr = (sadr + 16) & 0x3ff0;
            }
            let (data, next, end) = match id {
                0 => (addr, 0, true),
                1 => (tadr + 16, tadr + 16 + qwc * 16, false),
                2 => (tadr + 16, addr, false),
                3 | 4 => (addr, tadr + 16, false),
                5 => {
                    if asp < 2 {
                        asr[asp] = tadr + 16 + qwc * 16;
                        asp += 1;
                    }
                    (tadr + 16, addr, false)
                }
                6 => {
                    if asp > 0 {
                        asp -= 1;
                        (tadr + 16, asr[asp], false)
                    } else {
                        (tadr + 16, 0, true)
                    }
                }
                _ => (tadr + 16, 0, true),
            };
            self.sprlog(|| format!("to chain chcr {chcr:08x} tadr {tadr:08x} tag {tag:016x} data {data:08x} sadr {sadr:04x} qwc {qwc}"));
            copy(self, data, sadr, qwc);
            sadr = (sadr + qwc * 16) & 0x3ff0;
            if end || (tag >> 31) & 1 != 0 && chcr & 0x80 != 0 {
                break;
            }
            tadr = next;
        }
        self.set_hw32(base + 0x30, tadr);
        self.set_hw32(base + 0x80, sadr);
        self.set_hw32(base + 0x20, 0);
    }

    /// VIF1 kick: walk the source chain (no data moves) to the next stop
    /// (end tag, or an IRQ tag with TIE) and schedule it.
    fn vif1_kick(&mut self) {
        let chcr = self.hw32(VIF1);
        let mode = (chcr >> 2) & 3;
        let qwc = self.hw32(VIF1 + 0x20) & 0xffff;
        let mut madr = self.hw32(VIF1 + 0x10).wrapping_add(qwc * 16);
        let mut tadr = self.hw32(VIF1 + 0x30);
        let mut asr = [self.hw32(VIF1 + 0x40), self.hw32(VIF1 + 0x50)];
        let mut asp = ((chcr >> 4) & 3) as usize;
        let mut tag_hi = chcr & 0xffff_0000;
        // DIR = 0 is a GS -> memory download (the pause menu grabs the frame), not a VIF code stream.
        let cap = (self.gsc.is_some() || self.gsq.is_some()) && chcr & 1 != 0;
        let madr0 = self.hw32(VIF1 + 0x10);
        if let Some(g) = self.gsc.as_mut().filter(|g| g.log && chcr & 1 == 0) {
            *g.stats.entry(format!("vif1 download qwc {qwc:x} madr {madr0:x} mode {mode}")).or_insert(0) += 1;
            if std::env::var("ORACLE_GSSEQ").is_ok() {
                eprintln!("seq download qwc {qwc:x} madr {madr0:x}");
            }
        }
        let mut words = if cap { crate::gs::take_words(self.vif1_words) } else { Vec::new() };
        let mut marks = vec![(0usize, 0u32)];
        if cap {
            self.dma_words(self.hw32(VIF1 + 0x10), qwc, &mut words);
            if mode == 0 {
                self.gsc_vif1(std::mem::take(&mut words), std::mem::take(&mut marks));
            }
        }
        if mode == 1 {
            let mut n = 0;
            loop {
                n += 1;
                if n > 1_000_000 {
                    self.fault = Some(format!("VIF1 chain does not end (tadr {tadr:08x})"));
                    break;
                }
                let tag = self.rd64(tadr & 0x01ff_fff0);
                let q = (tag & 0xffff) as u32;
                let id = ((tag >> 28) & 7) as u32;
                let addr = ((tag >> 32) as u32) & 0x7fff_fff0;
                tag_hi = tag as u32 & 0xffff_0000;
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
                if cap {
                    if chcr & 0x40 != 0 {
                        marks.push((words.len(), 2));
                        let t = self.dma_qw(tadr);
                        words.push((t >> 64) as u32);
                        words.push((t >> 96) as u32);
                    }
                    // REF data may sit in scratchpad (address bit 31), which the walk above masks off.
                    let src = if id == 3 || id == 4 { (tag >> 32) as u32 & 0xffff_fff0 } else { data };
                    if let Some(g) = self.gsc.as_mut() {
                        if g.log {
                            g.blocks.push((words.len(), src, q, id));
                        }
                    }
                    marks.push((words.len(), 0));
                    self.dma_words(src, q, &mut words);
                }
                madr = data.wrapping_add(q * 16);
                tadr = next;
                if end || (tag >> 31) & 1 != 0 && chcr & 0x80 != 0 {
                    break;
                }
            }
        }
        if cap && mode == 1 {
            self.vif1_words = words.len();
            self.gsc_vif1(words, marks);
        }
        let fin = (chcr & 0xffcf) | (asp as u32) << 4 | tag_hi;
        self.vif1_pend = Some([fin, madr, tadr, asr[0], asr[1]]);
        self.dma_due = self.now + self.dma_delay;
        self.set_due(self.dma_due.min(self.tick));
    }

    /// A DMA qword (RAM, or scratchpad when bit 31 is set); no side effects.
    fn dma_qw(&self, a: u32) -> u128 {
        let b: &[u8] = if a & 0x8000_0000 != 0 {
            let o = (a & 0x3ff0) as usize;
            &self.spr[o..o + 16]
        } else {
            let o = (a & 0x01ff_fff0) as usize;
            if o + 16 > RAM {
                return 0;
            }
            &self.ram[o..o + 16]
        };
        u128::from_le_bytes(b.try_into().unwrap())
    }

    fn dma_words(&self, a: u32, q: u32, out: &mut Vec<u32>) {
        // [PC] (speed) RAM data that does not wrap: the same words in one pass (the game sends ~800K words a field).
        let o = (a & 0x01ff_fff0) as usize;
        if a & 0x8000_0000 == 0 && o + q as usize * 16 <= RAM {
            let b = &self.ram[o..o + q as usize * 16];
            // [PC] (speed) little-endian host: one block copy of the RAM bytes into the word Vec (same words as the from_le_bytes loop).
            #[cfg(target_endian = "little")]
            {
                let n = b.len() / 4;
                out.reserve(n);
                // SAFETY: `reserve` made room for n more u32 (4n bytes) behind len; the source is a disjoint &[u8] of exactly 4n bytes
                // (q*16); every bit pattern is a valid u32; set_len follows the copy.
                unsafe {
                    let len = out.len();
                    std::ptr::copy_nonoverlapping(b.as_ptr(), out.as_mut_ptr().add(len) as *mut u8, n * 4);
                    out.set_len(len + n);
                }
            }
            #[cfg(not(target_endian = "little"))]
            out.extend(b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())));
            return;
        }
        for k in 0..q {
            let v = self.dma_qw(a.wrapping_add(k * 16));
            out.extend([v as u32, (v >> 32) as u32, (v >> 64) as u32, (v >> 96) as u32]);
        }
    }

    /// DISPFB of the displayed read circuit (PMODE EN2 only: circuit 2).
    fn dispfb(&self) -> u64 {
        let r = |o: usize| u64::from_le_bytes(self.gs[o..o + 8].try_into().unwrap());
        if r(0) & 3 == 2 { r(0x90) } else { r(0x70) }
    }

    fn gsc_vif1(&mut self, w: Vec<u32>, marks: Vec<(usize, u32)>) {
        let job = crate::gs::Job::Vif1(w, marks, self.dispfb());
        self.gs_job(job);
    }

    fn gs_job(&mut self, job: crate::gs::Job) {
        if let Some(q) = &self.gsq {
            if q.send(job).is_err() {
                self.gsq = None;
            }
        } else if let Some(g) = self.gsc.as_mut() {
            g.run(job);
        }
    }

    /// GIF DMA (path 3) kick: only feeds the GS capture; the channel finishes at once.
    fn gif_kick(&mut self) {
        if self.gsc.is_none() && self.gsq.is_none() {
            return;
        }
        let mut qs = Vec::new();
        const GIF: u32 = 0x1000_a000;
        let chcr = self.hw32(GIF);
        let qwc = self.hw32(GIF + 0x20) & 0xffff;
        let madr = self.hw32(GIF + 0x10);
        for k in 0..qwc {
            qs.push(self.dma_qw(madr.wrapping_add(k * 16)));
        }
        if (chcr >> 2) & 3 == 1 {
            let mut tadr = self.hw32(GIF + 0x30);
            let mut asr = [0u32; 2];
            let mut asp = 0usize;
            for _ in 0..100_000 {
                let tag = self.dma_qw(tadr) as u64;
                let q = (tag & 0xffff) as u32;
                let id = ((tag >> 28) & 7) as u32;
                let addr = ((tag >> 32) as u32) & 0x7fff_fff0 | ((tag >> 32) as u32) & 0x8000_0000;
                let (data, next, end) = match id {
                    0 => (addr, 0, true),
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
                            (tadr + 16, 0, true)
                        }
                    }
                    _ => (tadr + 16, 0, true),
                };
                for k in 0..q {
                    qs.push(self.dma_qw(data.wrapping_add(k * 16)));
                }
                if end || id == 7 {
                    break;
                }
                tadr = next;
            }
        }
        let job = crate::gs::Job::Gif(qs, self.dispfb());
        self.gs_job(job);
    }

    /// An interrupt line may be asserted (the CPU re-tests INTC/DMAC); the poll limit drops to 0.
    #[inline(always)]
    pub fn mark_dirty(&mut self) {
        self.irq_dirty = true;
        self.limit = 0;
    }

    /// The lines were re-tested and are low.
    #[inline(always)]
    pub fn clear_dirty(&mut self) {
        self.irq_dirty = false;
        self.limit = self.due;
    }

    #[inline(always)]
    pub fn set_due(&mut self, d: u64) {
        self.due = d;
        self.limit = if self.irq_dirty { 0 } else { d };
    }

    /// `limit` follows `irq_dirty` and `due` (test: the writers above keep it).
    pub fn limit_ok(&self) -> bool {
        self.limit == if self.irq_dirty { 0 } else { self.due }
    }

    /// The scheduled VIF1 stop becomes visible: registers, CIS1, interrupt.
    pub fn mature(&mut self, ic: u64) {
        if self.vif1_pend.is_some() && ic < self.dma_due {
            self.set_due(self.dma_due.min(self.tick));
            return;
        }
        self.set_due(self.tick);
        let Some([chcr, madr, tadr, a0, a1]) = self.vif1_pend.take() else { return };
        self.vif1_stops += 1;
        self.set_hw32(VIF1, chcr & !0x100);
        self.set_hw32(VIF1 + 0x10, madr);
        self.set_hw32(VIF1 + 0x20, 0);
        self.set_hw32(VIF1 + 0x30, tadr);
        self.set_hw32(VIF1 + 0x40, a0);
        self.set_hw32(VIF1 + 0x50, a1);
        let s = self.hw32(D_STAT);
        self.set_hw32(D_STAT, s | 2);
        self.mark_dirty();
    }

    /// Cold boot: the next forced vblank is one field of instructions after `now`.
    pub fn rearm_tick(&mut self, now: u64) {
        self.tick = now + crate::ver::period();
        self.set_due(if self.vif1_pend.is_some() { self.dma_due.min(self.tick) } else { self.tick });
    }

    /// Interrupt lines: (INT0 = INTC, INT1 = DMAC).
    pub fn irq_lines(&self) -> (bool, bool) {
        let intc = self.hw32(INTC_STAT) & self.hw32(INTC_MASK) & 0x7fff != 0;
        let d = self.hw32(D_STAT);
        let dmac = (d & 0x3ff) & (d >> 16) & 0x3ff != 0
            || (d >> 13) & (d >> 29) & 3 != 0
            || d & 0x8000 != 0;
        (intc, dmac)
    }
}
