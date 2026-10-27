//! ee-oracle: retail R&C1 EE code run from a PCSX2 savestate (interpreter).
//! The `ee-oracle` binary diffs a run against a second savestate; `Machine`
//! is the per-frame driver rac-rs uses to play from a savestate.

pub mod isolvl;
#[allow(dead_code)]
mod wad;
pub mod clock;
pub mod cold;
pub mod ver;
pub mod cpu;
pub use ee_core::fpu;
pub mod gate;
pub mod gs;
pub mod hle;
pub mod mc;
pub mod mem;
pub mod snapfile;
pub mod snd989;
pub mod spu2;
pub mod rec;
pub mod vif;
mod vu;
mod vu0_prog0;
mod vu0_prog1;
mod vu0_prog9hi;
pub mod skinff;
pub mod vu0aot;

use cpu::Cpu;
use mem::Mem;

const LEVEL: u32 = 0x15ed84;
/// The vblank poll loop the game waits in (boot).
pub(crate) const VBLANK_WAIT: u32 = 0x118cb0;
/// sceGsSyncV's INTC_STAT poll while no vsync callback is registered (boot FUN_00118bd0 +0x40; `Cpu::poll_ff`).
pub(crate) const POLL_HEAD: u32 = 0x118c10;
/// Pad read (boot FUN_00124bd8): copies the pad DMA reply into a1, returns its length.
pub const PAD_READ: u32 = 0x124bd8;

/// A savestate (p2s.py output) to run: `load_raw`, then the RPC calls in flight in it are completed
/// (`Cpu::finish_rpcs_in_flight`).
pub fn load(dir: &str) -> Cpu {
    let mut c = load_raw(dir);
    c.finish_rpcs_in_flight();
    c
}

/// A savestate exactly as retail left it (RAM untouched): for states that are only compared against, never run.
pub fn load_raw(dir: &str) -> Cpu {
    let rd = |n: &str| std::fs::read(format!("{dir}/{n}")).unwrap_or_else(|e| panic!("{dir}/{n}: {e}"));
    let mut m = Mem::new();
    let cp = |dst: &mut Vec<u8>, src: Vec<u8>| {
        let n = dst.len().min(src.len());
        dst[..n].copy_from_slice(&src[..n]);
    };
    cp(&mut m.ram, rd("eeMemory.bin"));
    cp(&mut m.spr, rd("Scratchpad.bin"));
    cp(&mut m.vu0d, rd("vu0Memory.bin"));
    cp(&mut m.vu0m, rd("vu0MicroMem.bin"));
    m.vu0m_changed();
    cp(&mut m.vu1d, rd("vu1Memory.bin"));
    cp(&mut m.vu1m, rd("vu1MicroMem.bin"));
    cp(&mut m.hw, rd("eeHwRegs.bin"));
    let r = rd("regs.bin");
    let u32a = |o: usize| u32::from_le_bytes(r[o..o + 4].try_into().unwrap());
    let u128a = |o: usize| u128::from_le_bytes(r[o..o + 16].try_into().unwrap());
    let mut c = Cpu::new(m);
    for i in 0..32 {
        c.r[i] = u128a(i * 16);
    }
    c.hi = u128a(512);
    c.lo = u128a(528);
    for i in 0..32 {
        c.cp0[i] = u32a(544 + i * 4);
    }
    c.sa = u32a(672);
    c.pc = u32a(676);
    c.npc = c.pc.wrapping_add(4);
    assert_eq!(u32a(680), 0, "state paused in a delay slot");
    for i in 0..32 {
        c.f[i] = u32a(684 + i * 4);
    }
    c.fcr31 = u32a(812);
    c.facc = u32a(816);
    for i in 0..32 {
        for k in 0..4 {
            c.vf[i][k] = u32a(820 + i * 16 + k * 4);
        }
    }
    for k in 0..4 {
        c.vacc[k] = u32a(1332 + k * 4);
    }
    for i in 0..32 {
        c.vi[i] = u32a(1348 + i * 4);
    }
    // VPU_STAT (vi29) is the oracle's parked-microprogram state (see vu.rs). The hardware's busy bit and micro address
    // in a PCSX2 state taken while VU0 ran have nothing to resume here: the program starts again with its next VCALLMS.
    c.vi[29] &= 0xfffe;
    // IOP RAM: the game parks level data there (pause-menu WADs) and copies it back over RPC 0x1c13a8.
    if let Ok(iop) = std::fs::read(format!("{dir}/iopMemory.bin")) {
        c.iop = iop;
    }
    // The SPU2 voices running at save time (oracle/p2s.py `spu_state`): next to the state, else under $ORACLE_SPUV_DIR/<kind>/<name>.
    let p = std::path::Path::new(dir);
    let alt = std::env::var("ORACLE_SPUV_DIR").ok().and_then(|d| Some(format!("{d}/{}/{}/spu_voices.bin", p.parent()?.file_name()?.to_str()?, p.file_name()?.to_str()?)));
    if let Some(b) = std::fs::read(format!("{dir}/spu_voices.bin")).ok().or_else(|| alt.and_then(|a| std::fs::read(a).ok())) {
        c.snd.spuv = b;
    }
    let iso = std::env::var("ORACLE_ISO").unwrap_or_else(|_| DEFAULT_ISO.into());
    // The disc and its video mode (USA: NTSC timing; PAL: unchanged).
    ver::from_iso(&iso);
    c.iso = std::fs::File::open(iso).ok();
    c
}

/// The disc image read when ORACLE_ISO is not set: the retail file name next to this crate's folder (the dev tree root).
pub const DEFAULT_ISO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");

/// Four words at `addr` of level `n`'s overlay, read from the disc image (isolvl::overlay_image).
fn overlay_words(n: u32, addr: u32) -> Option<[u32; 4]> {
    let (va, b) = crate::isolvl::overlay_image(n)?;
    if addr < *va || addr as usize + 16 > *va as usize + b.len() {
        return None;
    }
    let o = (addr - *va) as usize;
    let u = |k: usize| u32::from_le_bytes(b[o + k..o + k + 4].try_into().unwrap());
    Some([u(0), u(4), u(8), u(12)])
}

pub fn load_stubs(c: &mut Cpu, path: &str) {
    if let Ok(w) = std::env::var("ORACLE_WATCH") {
        for a in w.split(',') {
            c.add_watch(u32::from_str_radix(a.trim_start_matches("0x"), 16).unwrap());
        }
    }
    if let Ok(w) = std::env::var("ORACLE_CALL") {
        let a = u32::from_str_radix(w.trim_start_matches("0x"), 16).unwrap();
        c.trace_call = Some(a);
        c.trace_left = std::env::var("ORACLE_CALLN").ok().and_then(|n| n.parse().ok()).unwrap_or(40);
        c.add_watch(a);
    }
    let Ok(t) = std::fs::read_to_string(path) else { return };
    let lvl = c.mem.ram[LEVEL as usize] as u32;
    for l in t.lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let mut it = l.split_whitespace().peekable();
        // "@N addr ret name": only while level N's overlay is loaded (Cpu::stub checks the code words).
        let mut level = None;
        if let Some(n) = it.peek().and_then(|w| w.strip_prefix('@')) {
            level = Some(n.parse::<u32>().unwrap());
            it.next();
        }
        let a = u32::from_str_radix(it.next().unwrap().trim_start_matches("0x"), 16).unwrap();
        let v = it.next().unwrap();
        let ret = if let Some(h) = v.strip_prefix("0x") {
            u64::from_str_radix(h, 16).unwrap()
        } else {
            v.parse::<i64>().unwrap() as u64
        };
        let name: Vec<&str> = it.collect();
        if c.stubs.contains_key(&a) {
            eprintln!("stubs.txt: {a:x} listed twice; keeping the first");
            continue;
        }
        c.add_stub(a, ret, &name.join(" "));
        c.stubs.get_mut(&a).unwrap().sig = match level {
            Some(n) => match overlay_words(n, a) {
                Some(s) => Some(s),
                None if n == lvl => Some(std::array::from_fn(|k| c.mem.rd32(a + 4 * k as u32))),
                None => continue,
            },
            None => None,
        };

    }
}

/// A retail game resumed from a savestate, stepped one vblank at a time. The game runs on its own thread, which
/// parks at each vblank wait (`Cpu::vblank_cb`) until the next `frame`; recompiled code nests a Rust frame per
/// retail call and stays on that thread's stack across vblanks. Between frames the thread is parked and `cpu` is
/// the host's.
pub struct Machine {
    pub cpu: std::mem::ManuallyDrop<Box<Cpu>>,
    game: Option<Game>,
}

struct Game {
    go: std::sync::mpsc::Sender<()>,
    done: std::sync::mpsc::Receiver<Result<(), String>>,
    join: Option<std::thread::JoinHandle<()>>,
    /// The thread did not reach a vblank in time and may still be running on `cpu` (leaked at drop).
    lost: bool,
    /// The game stopped (stop, fault, panic); later frames fail.
    over: Option<String>,
}

/// Unwinds the game thread out of the retail code (the host dropped the machine).
struct Quit;

/// Unwinds the game thread's compiled frames after a recompiled-table rebuild (`Cpu::rec_unwind`); it goes on
/// interpreting from the vblank wait.
pub(crate) struct RecRebuilt;

struct CpuPtr(*mut Cpu);
unsafe impl Send for CpuPtr {}
impl CpuPtr {
    fn get(self) -> *mut Cpu {
        self.0
    }
}

/// How long `frame` waits for a vblank before it reports a spin.
const FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

fn halted(c: &Cpu) -> Option<String> {
    if let Some(s) = &c.stop {
        return Some(format!("stop: {s}"));
    }
    c.mem.fault.as_ref().map(|f| format!("fault: {f} (pc {:08x})", c.pc))
}

impl Machine {
    /// An empty machine that is never run: the target of `mirror_to` (a view of RAM, scratchpad and GS registers for a
    /// reader thread; rac-rs `retail/snap_pipe.rs`). No game thread, no write watch.
    pub fn ghost() -> Machine {
        Machine { cpu: std::mem::ManuallyDrop::new(Box::new(Cpu::new(Mem::new()))), game: None }
    }

    /// Copies what the picture path reads into `ghost` (`Mem::mirror_to`); call between frames. Returns the RAM bytes copied.
    pub fn mirror_to(&self, ghost: &mut Machine, fresh: &mut bool) -> usize {
        self.cpu.mem.mirror_to(&mut ghost.cpu.mem, fresh)
    }

    /// True when `ghost` holds the same RAM, scratchpad and GS registers as this machine (RAC_SNAP_CHECK).
    pub fn same_view(&self, ghost: &Machine) -> bool {
        let (a, b) = (&self.cpu.mem, &ghost.cpu.mem);
        a.ram == b.ram && a.spr == b.spr && a.gs == b.gs
    }

    /// Loads `dir` (p2s.py output) and the stubs file `stubs.txt` from `dir` or its parents.
    pub fn load(dir: &str) -> Machine {
        let mut cpu = load(dir);
        let mut d = std::path::PathBuf::from(dir);
        for _ in 0..3 {
            let p = d.join("stubs.txt");
            if p.exists() {
                load_stubs(&mut cpu, &p.to_string_lossy());
                break;
            }
            if !d.pop() {
                break;
            }
        }
        // No SetVSyncFlag registration of the oracle's own yet. One the game made before the savestate is kernel
        // state in RAM (BIOS SetVSyncFlag 0x80001588 stores flag/csr at 0x80019444/0x80019448); the BIOS INTC handler
        // the oracle runs from RAM (0x80001660) writes it at the next VBLANK_S and clears it. The state may be anywhere
        // (a load's SIF poll, not only the vblank poll), so the outputs are never guessed from the stack pointer.
        cpu.vsync = None;
        cpu.add_stub(PAD_READ, 18, "pad_read");
        // Debug: hook rules (stopif / tracer / stop / trace ...) on a savestate machine, env COLD_HOOKS=file; the BIOS in
        // RAM stays in charge (cold::Kernel::hooks_only).
        let hooks = std::env::var("COLD_HOOKS").ok().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| {
            let mut k = cold::Kernel::hooks_only(&t);
            if std::env::var("COLD_PROF").is_ok() {
                k.prof = Some(Default::default());
            }
            Box::new(k)
        });
        if hooks.is_some() {
            cpu.cold = hooks;
            cold::install(&mut cpu, false);
        }
        let mut m = Machine { cpu: std::mem::ManuallyDrop::new(Box::new(cpu)), game: None };
        m.set_pad(0, [0x80; 4]);
        m
    }

    /// Cold boot of the retail game from the disc image: SCES_509.16 from the ISO into zeroed RAM, no savestate, no
    /// BIOS (see `cold`). `stubs` is oracle/stubs.txt, `hooks` an optional extra rule file (env COLD_HOOKS).
    pub fn cold(iso: &str, stubs: &str) -> Result<Machine, String> {
        let hooks = std::env::var("COLD_HOOKS").ok();
        let (mut cpu, mut k) = cold::boot(iso, stubs, hooks.as_deref())?;
        if std::env::var("COLD_PROF").is_ok() {
            k.prof = Some(Default::default());
        }
        cpu.cold = Some(Box::new(k));
        cold::install(&mut cpu, true);
        // A cold boot starts like a PS2 with a freshly formatted memory card, never a project card (see `Mc::base_cards`).
        cpu.mc.blank_default = true;
        let mut m = Machine { cpu: std::mem::ManuallyDrop::new(Box::new(cpu)), game: None };
        m.set_pad(0, [0x80; 4]);
        Ok(m)
    }

    /// The cold kernel's counters (None for savestate machines). Only valid between frames.
    pub fn cold_kernel(&self) -> Option<&cold::Kernel> {
        self.cpu.cold.as_deref()
    }

    /// Same, mutable (between frames only).
    pub fn cold_kernel_mut(&mut self) -> Option<&mut cold::Kernel> {
        self.cpu.cold.as_deref_mut()
    }

    /// The pad report the game reads next: buttons (bit set = pressed; 0x10 Triangle, 0x20 Circle, 0x40 Cross,
    /// 0x80 Square, 1 L2, 2 R2, 4 L1, 8 R1, 0x100 Select, 0x800 Start, 0x1000 Up, 0x2000 Right, 0x4000 Down,
    /// 0x8000 Left) and sticks (rx, ry, lx, ly; 0x80 centre, 0 = left / up).
    pub fn set_pad(&mut self, buttons: u16, sticks: [u8; 4]) {
        let b = !buttons;
        let mut r = [0u8; 18];
        r[0] = (b >> 8) as u8;
        r[1] = b as u8;
        r[2..6].copy_from_slice(&sticks);
        // Pressure bytes (right, left, up, down, triangle, circle, cross, square, L1, R1, L2, R2).
        let order = [0x2000u16, 0x8000, 0x1000, 0x4000, 0x10, 0x20, 0x40, 0x80, 4, 8, 1, 2];
        for (k, m) in order.iter().enumerate() {
            r[6 + k] = if buttons & m != 0 { 0xff } else { 0 };
        }
        self.cpu.pad = Some(r);
    }

    /// Runs the game to its next vblank wait and raises the vblank (one PAL field).
    pub fn frame(&mut self) -> Result<(), String> {
        if self.game.is_none() {
            self.start();
        }
        let g = self.game.as_mut().unwrap();
        if let Some(e) = &g.over {
            return Err(e.clone());
        }
        if g.lost {
            return Err("spin: no vblank wait reached".into());
        }
        let r = match g.go.send(()).map(|_| g.done.recv_timeout(FRAME_TIMEOUT)) {
            Ok(Ok(r)) => r,
            Ok(Err(std::sync::mpsc::RecvTimeoutError::Timeout)) => {
                g.lost = true;
                return Err(format!("spin: no vblank wait within {}s", FRAME_TIMEOUT.as_secs()));
            }
            _ => Err("game thread ended".into()),
        };
        if let Err(e) = &r {
            g.over = Some(e.clone());
        }
        r
    }

    /// Starts the game thread; it waits for the first `frame`.
    fn start(&mut self) {
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let p = CpuPtr(&mut **self.cpu as *mut Cpu);
        let join = std::thread::Builder::new()
            .name("ee".into())
            .stack_size(256 << 20)
            .spawn(move || {
                if go_rx.recv().is_err() {
                    return;
                }
                // Safety: the host touches `cpu` only while this thread is parked (between a done and the next go)
                // or after it ended, and the box is not freed while it may run (Machine::drop).
                let c = unsafe { &mut *p.get() };
                let cb_tx = done_tx.clone();
                c.vblank_cb = Some(Box::new(move |c: &mut Cpu| {
                    c.vblank();
                    // A new level's overlay: its recompiled functions (none while the loader replaces the overlay).
                    let lvl = c.mem.ram[LEVEL as usize] as u32;
                    let ld = u16::from_le_bytes([c.mem.ram[rec::LOADER as usize], c.mem.ram[rec::LOADER as usize + 1]]);
                    if c.rec.as_ref().is_some_and(|t| t.level != lvl || t.loader != ld) {
                        c.rec = Some(Box::new(rec::Table::build(&c.mem, lvl)));
                        c.rec_unwind = true;
                    }
                    let _ = cb_tx.send(halted(c).map_or(Ok(()), Err));
                    if go_rx.recv().is_err() {
                        std::panic::resume_unwind(Box::new(Quit));
                    }
                }));
                c.rec_catch = true;
                let r = loop {
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| loop {
                        c.run_rec(4096);
                        if let Some(e) = halted(c) {
                            return e;
                        }
                    }));
                    match r {
                        Err(x) if x.is::<RecRebuilt>() => continue,
                        r => break r,
                    }
                };
                c.vblank_cb = None;
                c.rec_catch = false;
                let e = match r {
                    Ok(e) => e,
                    Err(x) if x.is::<Quit>() => return,
                    Err(x) => format!(
                        "panic: {}",
                        x.downcast_ref::<String>().cloned().or(x.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
                    ),
                };
                let _ = done_tx.send(Err(e));
            })
            .expect("game thread");
        self.game = Some(Game { go: go_tx, done: done_rx, join: Some(join), lost: false, over: None });
    }

    /// Starts the GS capture (VRAM from `dir`/GS.bin); returns false without it.
    pub fn capture_gs(&mut self, dir: &str) -> bool {
        let Some(v) = gs::Gs::load_vram(&format!("{dir}/GS.bin")) else { return false };
        self.cpu.mem.gsc = Some(Box::new(gs::Gs::new(v)));
        true
    }

    /// The GS capture on a thread of its own: each overlay it finishes replaces the one in the returned slot.
    pub fn capture_gs_async(&mut self, dir: &str) -> Option<std::sync::Arc<std::sync::Mutex<Option<gs::Overlay>>>> {
        let v = gs::Gs::load_vram(&format!("{dir}/GS.bin"))?;
        let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let out = slot.clone();
        let (tx, rx) = std::sync::mpsc::channel::<gs::Job>();
        std::thread::Builder::new()
            .name("gs".into())
            .spawn(move || {
                let mut g = gs::Gs::new(v);
                while let Ok(j) = rx.recv() {
                    g.run(j);
                    if let Some(o) = g.done.take() {
                        *out.lock().unwrap() = Some(o);
                    }
                }
            })
            .ok()?;
        self.cpu.mem.gsq = Some(tx);
        Some(slot)
    }

    /// Runs the recompiled functions (rec/) whose code matches RAM; returns how many. ORACLE_REC=0 keeps the
    /// interpreter alone.
    pub fn enable_rec(&mut self) -> usize {
        if std::env::var("ORACLE_REC").is_ok_and(|v| v == "0") {
            return 0;
        }
        let t = rec::Table::build(&self.cpu.mem, self.level());
        let n = t.len();
        self.cpu.rec = Some(Box::new(t));
        n
    }

    /// Starts recording the SPU2 writes and stream events of the sound driver (off by default).
    pub fn record_sound(&mut self) {
        self.cpu.snd.set_record();
    }

    /// The SPU2 voice writes the exact 989snd driver made since the last call (needs `record_sound`).
    pub fn take_spu_events(&mut self) -> Vec<snd989::voices::SpuWrite> {
        let vb = self.cpu.mem.rd32(0x15ed48);
        let c = &mut *self.cpu;
        let inner = c.snd.ensure(&c.iop, &c.mem.ram, c.cold.is_some(), vb);
        inner.advance_to(inner.vb);
        std::mem::take(&mut inner.out.writes)
    }

    /// The stream (VAG) events since the last call (needs `record_sound`).
    pub fn take_stream_events(&mut self) -> Vec<snd989::streams::StreamEv> {
        let vb = self.cpu.mem.rd32(0x15ed48);
        let c = &mut *self.cpu;
        let inner = c.snd.ensure(&c.iop, &c.mem.ram, c.cold.is_some(), vb);
        inner.advance_to(inner.vb);
        std::mem::take(&mut inner.out.streams)
    }

    pub fn level(&self) -> u32 {
        self.cpu.mem.ram[LEVEL as usize] as u32
    }
    pub fn rd8(&self, a: u32) -> u8 {
        self.cpu.mem.ram[(a & 0x01ff_ffff) as usize]
    }
    pub fn rd16(&self, a: u32) -> u16 {
        let i = (a & 0x01ff_ffff) as usize;
        u16::from_le_bytes([self.cpu.mem.ram[i], self.cpu.mem.ram[i + 1]])
    }
    pub fn rd32(&self, a: u32) -> u32 {
        let i = (a & 0x01ff_ffff) as usize;
        u32::from_le_bytes(self.cpu.mem.ram[i..i + 4].try_into().unwrap())
    }
    pub fn rdf(&self, a: u32) -> f32 {
        f32::from_bits(self.rd32(a))
    }
    pub fn rdv3(&self, a: u32) -> [f32; 3] {
        [self.rdf(a), self.rdf(a + 4), self.rdf(a + 8)]
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        if let Some(mut g) = self.game.take() {
            if g.lost {
                // Still running on `cpu`: leaked.
                return;
            }
            // A parked thread unwinds out of the retail code (Quit) when its go channel closes.
            drop(g.go);
            if let Some(j) = g.join.take() {
                let _ = j.join();
            }
        }
        // Safety: no thread uses it any more.
        unsafe { std::mem::ManuallyDrop::drop(&mut self.cpu) }
    }
}
