//! Cold boot of R&C1 from the ISO (RACE.md step 4): the retail boot ELF (SCES_509.16) runs in a zeroed machine with no
//! PCSX2 savestate and no BIOS image. `boot` builds the `Cpu` and the `Kernel` (the small HLE of what the BIOS would
//! provide: syscalls by number, INTC/DMAC handler lists and interrupt dispatch, SIF registers, a few rules for
//! libkernel/SIF code that probes hardware); `step` is the cold replacement of `Cpu::step_rec`. Nothing here is used
//! by the savestate paths (`Machine::load`); `Machine::cold` is the entry point.
//! Console language (what the BIOS would report through GetOsdConfigParam): env COLD_LANG = en fr de es it (or the OSD
//! number 1..5; default English) or `set_language` before `Machine::cold`; see `language`.
//! Hook rules (text, one per line; `HOOKS` plus an optional extra file, env COLD_HOOKS):
//!   ret PC VAL | w32 PC ADDR VAL | w32r PC REG OFF VAL | jmp PC TARGET | trace PC N | stop PC
use crate::cpu::Cpu;
use crate::mem::{Mem, D_STAT, INTC_MASK, INTC_STAT};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU32, Ordering};

pub const TRAMP: u32 = 0x8000_0300;
/// One PAL field in EE instructions (294.912 MHz / 50) when nothing waits at the vblank poll. The cold kernel uses
/// `ver::period()` (the same in PAL mode, shorter for NTSC).
pub const PERIOD: u64 = 5_900_000;
/// Power-on register set applied by `boot` (bits of env COLD_PWR, see there).
pub const PWR_DEFAULT: u32 = 0xf;

/// Console language the cold kernel reports to the game (GetOsdConfigParam, syscall 0x4b), as the PS2 OSD language number:
/// 1 English, 2 French, 3 Spanish, 4 German, 5 Italian are the five languages of the PAL disc (0 Japanese and 6+ are not on it:
/// the game prints a debug line and falls back to English). The game maps the number to its own language word 0x15ed88
/// (FUN_0023d3xx: 1 -> 0, 2 -> 2, 3 -> 4, 4 -> 3, 5 -> 5) and loads the front end's language pack for it.
pub const OSD_ENGLISH: u32 = 1;
static LANGUAGE: AtomicU32 = AtomicU32::new(u32::MAX);

/// Chooses the language of the next `Machine::cold` (call before it; overrides env COLD_LANG). `osd` as `parse_language`.
pub fn set_language(osd: u32) {
    LANGUAGE.store(osd & 0x1f, Ordering::Relaxed);
}

/// OSD language number of a language name or number: en/eng/english, fr/fra/fre/french, de/deu/ger/german/deutsch,
/// es/spa/spanish/espanol, it/ita/italian/italiano (case does not matter), or the number itself (0..31).
pub fn parse_language(s: &str) -> Option<u32> {
    let t = s.trim().to_ascii_lowercase();
    Some(match t.as_str() {
        "en" | "eng" | "english" => 1,
        "fr" | "fra" | "fre" | "french" | "francais" => 2,
        "es" | "spa" | "spanish" | "espanol" => 3,
        "de" | "deu" | "ger" | "german" | "deutsch" => 4,
        "it" | "ita" | "italian" | "italiano" => 5,
        _ => return t.parse::<u32>().ok().filter(|&n| n < 32),
    })
}

/// The OSD language number a cold boot reports: `set_language`, else env COLD_LANG, else English. A COLD_LANG value that
/// `parse_language` cannot read is reported on stderr and gives English.
pub fn language() -> u32 {
    let set = LANGUAGE.load(Ordering::Relaxed);
    if set != u32::MAX {
        return set;
    }
    match std::env::var("COLD_LANG") {
        Ok(v) => parse_language(&v).unwrap_or_else(|| {
            eprintln!("cold: COLD_LANG={v:?} is not a language (en fr de es it, or the OSD number 1..5): English");
            OSD_ENGLISH
        }),
        Err(_) => OSD_ENGLISH,
    }
}

/// The console's OSD settings word (GetOsdConfigParam): bit 0 spdif, bits 1-2 screen type (0 4:3, 1 full, 2 16:9), bit 3 video
/// output, bit 4 japLanguage, bits 5-12 ps1drv, bits 13-15 version (2), bits 16-20 language (`language()`), bits 21-31 timezone
/// offset. Defaults: English, 4:3 (env COLD_ASPECT 0..3).
fn osd_config_word() -> u32 {
    let aspect = std::env::var("COLD_ASPECT").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    2 << 13 | (language() & 0x1f) << 16 | (aspect & 3) << 1
}

pub struct Hnd {
    pub id: u32,
    pub cause: u32,
    pub func: u32,
    gp: u32,
    arg: u32,
}

pub struct Saved {
    r: [u128; 32],
    hi: u128,
    lo: u128,
    sa: u32,
    f: [u32; 32],
    facc: u32,
    fcr31: u32,
    epc: u32,
    queue: Vec<(u32, u32, u32)>,
    cause: u32,
    pub dmac: bool,
}

#[derive(Default)]
pub struct Kernel {
    pub intc: Vec<Hnd>,
    pub dmac: Vec<Hnd>,
    pub next_id: u32,
    pub stack: Vec<Saved>,
    pub irq_cnt: HashMap<(bool, u32), u64>,
    pub sif_reg: HashMap<u32, u32>,
    pub sys_seen: HashMap<i32, u64>,
    pub unhandled: HashMap<i32, u64>,
    pub threads: Vec<(u32, u32, u32)>,
    pub heap_end: u32,
    pub rules_ret: HashMap<u32, u64>,
    pub rules_w32: HashMap<u32, Vec<(u32, u32)>>,
    pub rules_jmp: HashMap<u32, u32>,
    pub frames: u64,
    pub sema: HashMap<u32, (i32, i32)>,
    pub sema_next: u32,
    pub sema_blocked: HashMap<u32, u64>,
    pub trace: HashMap<u32, u32>,
    pub stops: HashSet<u32>,
    pub rules_w32r: HashMap<u32, Vec<(usize, u32, u32)>>,
    pub pads: Vec<(u64, u64, u16)>,
    pub hooked: Vec<bool>,
    pub next_vb: u64,
    pub prof: Option<HashMap<u32, u32>>,
    pub fault_info: Option<String>,
    /// Savestate machine with hook rules only (env COLD_HOOKS on `Machine::load`): no kernel HLE, no field timer.
    pub hooks_only: bool,
    pub stopif: HashMap<u32, Vec<(usize, u32)>>,
    pub tracer: HashMap<u32, u32>,
    /// pctrace PC SKIP COUNT: record the executed pc ranges of COUNT calls (after SKIP) from PC until it returns
    pub pctrace: HashMap<u32, (u32, u32, Option<(usize, u32)>)>,
    /// memat PC REG OFF NW N: print NW words at (REG + OFF) at the next N executions of PC
    pub memat: HashMap<u32, Vec<(usize, u32, u32, u32)>>,
    pub pcact: Option<PcAct>,
    /// What GetOsdConfigParam (syscall 0x4b) stores: fixed at `boot` (`osd_config_word`).
    pub osd_word: u32,
}

pub struct PcAct {
    pub entry: u32,
    pub ra: u32,
    pub sp: u32,
    pub ranges: Vec<(u32, u32)>,
    pub regs: String,
    pub icount: u64,
}

pub fn iso_find(f: &mut std::fs::File, want: &str) -> Option<(u32, u32)> {
    let mut rd = |lba: u32, n: usize| {
        let mut b = vec![0u8; n * 2048];
        f.seek(SeekFrom::Start(lba as u64 * 2048)).ok()?;
        f.read_exact(&mut b).ok()?;
        Some(b)
    };
    let pvd = rd(16, 1)?;
    if &pvd[1..6] != b"CD001" {
        return None;
    }
    let u32at = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let (lba, size) = (u32at(&pvd, 156 + 2), u32at(&pvd, 156 + 10));
    let d = rd(lba, (size as usize).div_ceil(2048))?;
    let mut o = 0;
    while o < size as usize {
        let l = d[o] as usize;
        if l == 0 {
            o = (o / 2048 + 1) * 2048;
            continue;
        }
        let nl = d[o + 32] as usize;
        let name = String::from_utf8_lossy(&d[o + 33..o + 33 + nl]).to_string();
        if name == want {
            return Some((u32at(&d, o + 2), u32at(&d, o + 10)));
        }
        o += l;
    }
    None
}

pub fn read_hooks(t: &str, k: &mut Kernel, hooked: &mut [bool]) {
    let h = |s: &str| u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap();
    for l in t.lines() {
        let mut w: Vec<&str> = l.split_whitespace().collect();
        if w.is_empty() || w[0].starts_with('#') {
            continue;
        }
        // A rule for one disc only: "pal RULE.." / "usa RULE.." (addresses above 0x15ef00 differ between the discs).
        if w[0] == "pal" || w[0] == "usa" {
            if w[0] != crate::ver::disc().tag() {
                continue;
            }
            w.remove(0);
        }
        let pc = h(w[1]);
        hooked[(pc >> 2) as usize & 0x7f_ffff] = true;
        match w[0] {
            "ret" => {
                k.rules_ret.insert(pc, h(w[2]) as u64);
            }
            "w32" => k.rules_w32.entry(pc).or_default().push((h(w[2]), h(w[3]))),
            "jmp" => {
                k.rules_jmp.insert(pc, h(w[2]));
            }
            // w32r PC REGNO OFF VAL: store VAL at (register REGNO + OFF)
            "w32r" => k.rules_w32r.entry(pc).or_default().push((w[2].parse().unwrap(), h(w[3]), h(w[4]))),
            "trace" => {
                k.trace.insert(pc, w[2].parse().unwrap());
            }
            "stop" => {
                k.stops.insert(pc);
            }
            // stopif PC REGNO VAL: stop when the low 32 bits of the register equal VAL
            "stopif" => k.stopif.entry(pc).or_default().push((w[2].parse().unwrap(), h(w[3]))),
            // tracer PC N: print all GPRs (low 32 bits) at the next N executions
            "tracer" => {
                k.tracer.insert(pc, w[2].parse().unwrap());
            }
            "pctrace" => {
                let f = if w.len() >= 6 { Some((w[4].parse().unwrap(), h(w[5]))) } else { None };
                k.pctrace.insert(pc, (w[2].parse().unwrap(), w[3].parse().unwrap(), f));
            }
            "memat" => k.memat.entry(pc).or_default().push((w[2].parse().unwrap(), h(w[3]), w[4].parse().unwrap(), w[5].parse().unwrap())),
            _ => eprintln!("hooks: unknown rule {l}"),
        }
    }
}

fn ret(c: &mut Cpu, v: u64) {
    c.s(2, v);
    let ra = c.g(31) as u32;
    c.pc = ra;
    c.npc = ra.wrapping_add(4);
}

fn save(c: &Cpu, queue: Vec<(u32, u32, u32)>, cause: u32, dmac: bool) -> Saved {
    Saved { r: c.r, hi: c.hi, lo: c.lo, sa: c.sa, f: c.f, facc: c.facc, fcr31: c.fcr31, epc: c.cp0[14], queue, cause, dmac }
}

/// The CPU took INT0/INT1 (vector 0x80000200): run the registered handlers for the lowest pending cause, as the BIOS
/// dispatcher would, each on the interrupted stack; the trampoline address is their return address.
fn dispatch(c: &mut Cpu, k: &mut Kernel) {
    let (i0, i1) = c.mem.irq_lines();
    let (mut queue, mut cause, mut dmac) = (Vec::new(), 0, false);
    if i0 {
        let st = c.mem.hw32(INTC_STAT) & c.mem.hw32(INTC_MASK) & 0x7fff;
        cause = st.trailing_zeros();
        queue = k.intc.iter().filter(|h| h.cause == cause).map(|h| (h.func, h.gp, h.arg)).collect();
    } else if i1 {
        dmac = true;
        let d = c.mem.hw32(D_STAT);
        let st = (d & 0x3ff) & (d >> 16) & 0x3ff;
        if st != 0 {
            cause = st.trailing_zeros();
            queue = k.dmac.iter().filter(|h| h.cause == cause || h.cause == u32::MAX).map(|h| (h.func, h.gp, h.arg)).collect();
        } else {
            cause = 31; // MFIFO empty / bus error style bits: acknowledged below
        }
    }
    *k.irq_cnt.entry((dmac, cause)).or_insert(0) += 1;
    let s = save(c, queue, cause, dmac);
    k.stack.push(s);
    next_handler(c, k);
}

fn ack(c: &mut Cpu, dmac: bool, cause: u32) {
    if dmac {
        let v = if cause < 10 { 1 << cause } else { 0x6000 | 0x8000 };
        c.mem.wr32(D_STAT, v);
    } else if cause < 16 {
        c.mem.wr32(INTC_STAT, 1 << cause);
    }
    c.mem.mark_dirty();
}

fn next_handler(c: &mut Cpu, k: &mut Kernel) {
    let top = k.stack.last_mut().unwrap();
    if let Some((func, gp, arg)) = (!top.queue.is_empty()).then(|| top.queue.remove(0)) {
        let sp = (top.r[29] as u32 & !0xf).wrapping_sub(0x200);
        let cause = top.cause;
        c.s(4, cause as u64);
        c.s(5, arg as u64);
        c.s(6, 0);
        c.s(28, gp as u64);
        c.s(29, sp as u64);
        c.s(31, TRAMP as u64);
        c.pc = func;
        c.npc = func.wrapping_add(4);
        return;
    }
    let s = k.stack.pop().unwrap();
    ack(c, s.dmac, s.cause);
    c.r = s.r;
    c.hi = s.hi;
    c.lo = s.lo;
    c.sa = s.sa;
    c.f = s.f;
    c.facc = s.facc;
    c.fcr31 = s.fcr31;
    c.cp0[12] &= !2;
    c.pc = s.epc;
    c.npc = s.epc.wrapping_add(4);
    c.mem.mark_dirty();
}

/// The cold-boot kernel: syscalls the oracle's own HLE (hle.rs) cannot do without the BIOS in RAM. Returns false to
/// let hle.rs handle the number (it returns 0 for most).
pub fn syscall(c: &mut Cpu, k: &mut Kernel, pc: u32) -> bool {
    let n = c.g(3) as u32 as i32;
    let (a0, a1, a2, a3) = (c.g(4) as u32, c.g(5) as u32, c.g(6) as u32, c.g(7) as u32);
    *k.sys_seen.entry(n).or_insert(0) += 1;
    let v: i64 = match n.unsigned_abs() {
        // SetupThread(gp, stack, size, args, root): the stack top the crt0 runs on; args block = argc, argv, strings.
        0x3c => {
            let top = if a1 == u32::MAX { 0x0200_0000 } else { a1.wrapping_add(a2) };
            if a3 != 0 {
                c.mem.wr32(a3, 1);
                c.mem.wr32(a3 + 4, a3 + 0x10);
                let arg0 = format!("cdrom0:\\{};1\0", crate::ver::disc().boot_file());
                for (i, b) in arg0.bytes().enumerate() {
                    c.mem.wr8(a3 + 0x10 + i as u32, b);
                }
            }
            c.s(28, a0 as u64);
            top.wrapping_sub(0x2a0) as i64
        }
        // SetupHeap(base, size) returns the heap start; EndOfHeap its end.
        0x3d => {
            k.heap_end = a0.wrapping_add(a1);
            a0 as i64
        }
        0x3e => k.heap_end as i64,
        // AddIntcHandler(cause, fn, next, arg) / AddDmacHandler(channel, fn, next, arg): the handler list the BIOS keeps.
        0x10 | 0x12 => {
            k.next_id += 1;
            let h = Hnd { id: k.next_id, cause: a0, func: a1, gp: c.g(28) as u32, arg: a3 };
            let id = h.id;
            if n.unsigned_abs() == 0x10 { k.intc.push(h) } else { k.dmac.push(h) }
            id as i64
        }
        0x11 | 0x13 => {
            let l = if n.unsigned_abs() == 0x11 { &mut k.intc } else { &mut k.dmac };
            let before = l.len();
            l.retain(|h| !(h.cause == a0 && h.id == a1));
            (l.len() != before) as i64 - 1
        }
        // EnableIntc/DisableIntc(cause): INTC_MASK is write-1-to-toggle.
        0x14 | 0x15 | 0x1c | 0x1d => {
            let en = matches!(n.unsigned_abs(), 0x14 | 0x1c);
            let m = c.mem.hw32(INTC_MASK);
            let cur = m >> a0 & 1 != 0;
            if cur != en {
                c.mem.wr32(INTC_MASK, 1 << a0);
            }
            c.mem.mark_dirty();
            (cur != en) as i64
        }
        // EnableDmac/DisableDmac(channel): D_STAT mask bits 16.. toggle on write.
        0x16 | 0x17 | 0x1e | 0x1f => {
            let en = matches!(n.unsigned_abs(), 0x16 | 0x1e);
            let d = c.mem.hw32(D_STAT);
            let cur = d >> (16 + a0) & 1 != 0;
            if cur != en {
                c.mem.wr32(D_STAT, 1 << (16 + a0));
            }
            c.mem.mark_dirty();
            (cur != en) as i64
        }
        // Semaphores with real counts: ee_sema {count, max_count, init_count, wait_threads, attr, option}.
        0x40 => {
            k.sema_next += 1;
            let id = 0x200 + k.sema_next;
            k.sema.insert(id, (c.mem.rd32(a0 + 8) as i32, c.mem.rd32(a0 + 4) as i32));
            id as i64
        }
        0x41 => {
            k.sema.remove(&a0);
            a0 as i64
        }
        0x42 | 0x43 => {
            if let Some(s) = k.sema.get_mut(&a0) {
                if s.0 < s.1 {
                    s.0 += 1;
                }
            }
            a0 as i64
        }
        // WaitSema: no scheduler, so a wait on a zero count cannot be served; counted and let through.
        0x44 => {
            if let Some(s) = k.sema.get_mut(&a0) {
                if s.0 > 0 {
                    s.0 -= 1;
                } else {
                    *k.sema_blocked.entry(pc).or_insert(0) += 1;
                }
            }
            a0 as i64
        }
        0x45 | 0x46 => match k.sema.get_mut(&a0) {
            Some(s) if s.0 > 0 => {
                s.0 -= 1;
                a0 as i64
            }
            _ => -419,
        },
        0x20 => {
            // ThreadParam: attr, entry, stack, stack_size, gp, prio (read at a0)
            let (entry, stack, size) = (c.mem.rd32(a0 + 4), c.mem.rd32(a0 + 8), c.mem.rd32(a0 + 12));
            k.threads.push((entry, stack, size));
            0x100 + k.threads.len() as i64
        }
        0x22 => {
            eprintln!("StartThread id {a0:#x} arg {a1:#x} (not run: no scheduler) entry {:08x}", k.threads.get((a0 as usize).wrapping_sub(0x101)).map_or(0, |t| t.0));
            0
        }
        // 0x76 sceSifDmaStat is left to hle.rs (-1: finished). 0x78 sceSifSetDChain: nothing to chain.
        0x78 => 0,
        // GetOsdConfigParam(u32 *p): the console's OSD settings word (`osd_config_word`, language from `language()`).
        0x4b => {
            if a0 != 0 {
                c.mem.wr32(a0, k.osd_word);
            }
            0
        }
        // sceSifSetReg(reg, val) / sceSifGetReg(reg): reg 4 is SMFLG; the IOP's boot-end/cmd-init bits come up at once.
        0x79 => {
            k.sif_reg.insert(a0, a1);
            0
        }
        0x7a => {
            let v = k.sif_reg.get(&a0).copied().unwrap_or(0);
            (if a0 == 4 { v | 0x7_0000 } else { v }) as i64
        }
        0x04 | 0x06 | 0x07 => {
            c.stop = Some(format!("game called Exit/LoadExecPS2/ExecPS2 ({n:#x}) at {pc:08x} a0 {a0:08x} a1 {a1:08x} a2 {a2:08x}"));
            0
        }
        0x23 | 0x24 => {
            c.stop = Some(format!("ExitThread at {pc:08x}"));
            0
        }
        _ => {
            *k.unhandled.entry(n).or_insert(0) += 1;
            return false;
        }
    };
    c.s(2, v as u64);
    true
}

/// Built-in rules (boot ELF addresses, PAL v1.00); see coldboot.md "Run 2" for the why of each.
pub const HOOKS: &str = include_str!("cold_hooks.txt");

/// A cold-boot machine: the ELF from the ISO in zeroed RAM, Status as at an ELF entry, pad idle.
pub fn boot(iso_path: &str, stubs_path: &str, extra_hooks: Option<&str>) -> Result<(Cpu, Kernel), String> {
    let mut iso = std::fs::File::open(iso_path).map_err(|e| format!("iso {iso_path}: {e}"))?;
    // The disc (PAL SCES_509.16 or USA SCUS_971.99) and the video mode the game will pick from SYSTEM.CNF.
    crate::ver::from_iso(iso_path);
    let boot = crate::ver::disc().boot_file();
    let (lba, size) = iso_find(&mut iso, &format!("{boot};1")).ok_or_else(|| format!("neither SCES_509.16 nor SCUS_971.99 in the ISO {iso_path}"))?;
    let mut elf = vec![0u8; size as usize];
    iso.seek(SeekFrom::Start(lba as u64 * 2048)).map_err(|e| e.to_string())?;
    iso.read_exact(&mut elf).map_err(|e| e.to_string())?;
    let u32at = |o: usize| u32::from_le_bytes(elf[o..o + 4].try_into().unwrap());
    let entry = u32at(24);
    let (ph, phn) = (u32at(28) as usize, u16::from_le_bytes(elf[44..46].try_into().unwrap()) as usize);
    let mut m = Mem::new();
    for i in 0..phn {
        let o = ph + 32 * i;
        let (ty, off, va, fsz) = (u32at(o), u32at(o + 4) as usize, u32at(o + 8) as usize, u32at(o + 16) as usize);
        if ty == 1 {
            m.ram[va..va + fsz].copy_from_slice(&elf[off..off + fsz]);
        }
    }
    let mut c = Cpu::new(m);
    // Debug (env COLD_VU0D=file): preload VU0 data memory (e.g. a savestate's vu0Memory.bin) to test whether the game relies on it.
    if let Some(b) = std::env::var("COLD_VU0D").ok().and_then(|p| std::fs::read(p).ok()) {
        let n = b.len().min(c.mem.vu0d.len());
        c.mem.vu0d[..n].copy_from_slice(&b[..n]);
    }
    c.pc = entry;
    c.npc = entry + 4;
    // Status at an ELF's entry: CU0..3 usable, IM bits for INT0/INT1 set, EDI set; EIE clear until the game's `ei`.
    // VU0 power-on state (a savestate restores it from regs.bin): vf0 = (0,0,0,1.0) is a hardware constant the VU0 macro code
    // uses (vmulw/vdiv/vclip against vf0.w); R register in the 1.x range.
    c.vf[0] = [0, 0, 0, 0x3f80_0000];
    c.vi[20] = 0x3f80_0000;
    c.cp0[12] = 0x7002_0c01;
    c.cp0[15] = 0x0000_2e20; // PRId (R5900)
    // Debug (env COLD_PWR=hex bitmask): the other power-on values every savestate has (Run 6 audit): 1 VU0 I (vi21), 2 fcr31, 4 COP0 Compare, 8 COP0 Config.
    let pwr = std::env::var("COLD_PWR").ok().and_then(|v| u32::from_str_radix(&v, 16).ok()).unwrap_or(PWR_DEFAULT);
    if pwr & 1 != 0 {
        c.vi[21] = 0x362e_9c14;
    }
    if pwr & 2 != 0 {
        c.fcr31 = 0x0100_0001;
    }
    if pwr & 4 != 0 {
        c.cp0[11] = 1;
    }
    if pwr & 8 != 0 {
        c.cp0[16] = 0x73443;
    }
    c.iso = std::fs::File::open(iso_path).ok();
    crate::load_stubs(&mut c, stubs_path);
    c.add_stub(crate::PAD_READ, 18, "pad_read");
    let mut pad = [0u8; 18];
    pad[0] = 0xff;
    pad[1] = 0xff;
    pad[2..6].copy_from_slice(&[0x80; 4]);
    c.pad = Some(pad);
    if let Ok(w) = std::env::var("COLD_WATCH") {
        let (lo, hi) = w.split_once('-').expect("COLD_WATCH=lo-hi (hex scratchpad offsets)");
        c.mem.watch = Some((usize::from_str_radix(lo, 16).unwrap(), usize::from_str_radix(hi, 16).unwrap()));
    }
    let mut k = Kernel::default();
    let mut hk = vec![false; 1 << 23];
    read_hooks(HOOKS, &mut k, &mut hk);
    if let Some(p) = extra_hooks {
        if let Ok(t) = std::fs::read_to_string(p) {
            read_hooks(&t, &mut k, &mut hk);
        }
    }
    k.hooked = hk;
    k.next_vb = crate::ver::period();
    k.osd_word = osd_config_word();
    Ok((c, k))
}

/// Kernel space and hook rules at one pc (called from `Cpu::stub`, i.e. only for pcs `install` marked in `stub_bits`,
/// from the interpreter and from the dispatcher in front of compiled code). True: the pc was handled (no instruction runs).
pub fn stub(c: &mut Cpu, pc: u32) -> bool {
    let Some(mut k) = c.cold.take() else { return false };
    let r = stub_k(c, &mut k, pc);
    c.cold = Some(k);
    r
}

fn stub_k(c: &mut Cpu, k: &mut Kernel, pc: u32) -> bool {
    if pc >= 0x8000_0000 {
        if k.hooks_only {
            return false;
        }
        if pc == 0x8000_0200 {
            // The vector's first word is a nop in the zeroed kernel RAM: it counts as one instruction (as the run 5 driver did).
            c.pc = 0x8000_0204;
            c.npc = 0x8000_0208;
            c.icount += 1;
            c.mem.now = c.icount;
        } else if pc == 0x8000_0204 {
            dispatch(c, k);
        } else if pc == TRAMP {
            next_handler(c, k);
        } else {
            return false;
        }
        return true;
    }
    if k.hooked[(pc >> 2) as usize & 0x7f_ffff] {
        let tr_from: u64 = {
            static F: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
            *F.get_or_init(|| std::env::var("COLD_TR_FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(0))
        };
        if let Some(ms) = k.memat.get_mut(&pc) {
            for m in ms.iter_mut() {
                if m.3 > 0 && c.icount >= tr_from {
                    m.3 -= 1;
                    let base = (c.g(m.0) as u32).wrapping_add(m.1);
                    let ws: Vec<String> = (0..m.2).map(|i| format!("{:08x}", c.mem.rd32(base.wrapping_add(4 * i)))).collect();
                    eprintln!("memat {pc:08x} @{} [r{}+{:x}={base:08x}] {}", c.icount, m.0, m.1, ws.join(" "));
                }
            }
        }
        if let Some(n) = k.trace.get_mut(&pc) {
            if *n > 0 {
                *n -= 1;
                eprintln!(
                    "trace {pc:08x} a0 {:08x} a1 {:08x} a2 {:08x} a3 {:08x} v0 {:08x} ra {:08x} sp {:08x} @{}",
                    c.g(4) as u32, c.g(5) as u32, c.g(6) as u32, c.g(7) as u32, c.g(2) as u32, c.g(31) as u32, c.g(29) as u32, c.icount
                );
            }
        }
        if let Some(n) = k.tracer.get_mut(&pc) {
            if *n > 0 && c.icount >= tr_from {
                *n -= 1;
                eprintln!("tracer {pc:08x} @{} {}", c.icount, (0..32).map(|i| format!("{i}={:x}", c.g(i) as u32)).collect::<Vec<_>>().join(" "));
            }
        }
        let hit = k.stops.contains(&pc) || k.stopif.get(&pc).is_some_and(|v| v.iter().all(|&(r, x)| c.g(r) as u32 == x));
        if hit {
            c.stop = Some(format!("hook stop at {pc:08x}"));
            if k.fault_info.is_none() {
                k.fault_info = Some(fault_info(c, pc));
            }
            return true;
        }
        if let Some(ws) = k.rules_w32r.get(&pc) {
            for &(r, off, v) in ws.clone().iter() {
                let base = c.g(r) as u32;
                c.mem.wr32(base.wrapping_add(off), v);
            }
        }
        if let Some(&v) = k.rules_ret.get(&pc) {
            ret(c, v);
            return true;
        }
        if let Some(ws) = k.rules_w32.get(&pc) {
            for &(ad, v) in ws.clone().iter() {
                c.mem.wr32(ad, v);
            }
        }
        if let Some(&t) = k.rules_jmp.get(&pc) {
            c.pc = t;
            c.npc = t + 4;
            return true;
        }
    }
    false
}

/// The field timer expired (`Cpu::step` / `poll_slow`): the same vblank, raised while the game computes.
pub fn tick(c: &mut Cpu) {
    c.mem.rearm_tick(c.icount);
    c.vsync_armed = true;
    if !c.vblank_hook() {
        c.vblank();
    }
}

/// `syscall` instruction at `pc` (hle.rs `Cpu::syscall`, post-fetch: the PC is already past it). True: handled here.
pub fn syscall_hook(c: &mut Cpu, pc: u32) -> bool {
    if c.pc != pc.wrapping_add(4) {
        return false; // in a delay slot: left to hle.rs, as before
    }
    let Some(mut k) = c.cold.take() else { return false };
    let r = !k.hooks_only && syscall(c, &mut k, pc);
    c.cold = Some(k);
    // Debug (env COLD_SYSFREE=1, interpreter only): a syscall handled here costs no instruction, as in the run 5 driver (equivalence test).
    static FREE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if r && *FREE.get_or_init(|| std::env::var("COLD_SYSFREE").is_ok()) {
        c.icount -= 1;
        c.mem.now = c.icount;
    }
    r
}

/// Marks the pcs `stub` must see in `stub_bits` and (full cold boot) starts the field timer. Call after the kernel is in `c.cold`.
pub fn install(c: &mut Cpu, full: bool) {
    let mut idx: Vec<usize> = Vec::new();
    {
        let k = c.cold.as_ref().expect("install: no kernel");
        idx.extend(k.hooked.iter().enumerate().filter(|(_, &b)| b).map(|(i, _)| i));
    }
    if full {
        // POLL_HEAD: the interpreter's idle skip of sceGsSyncV's poll (`Cpu::poll_ff`, level loads after the 60 Hz switch).
        idx.extend([0x8000_0200u32, 0x8000_0204, TRAMP, crate::POLL_HEAD].iter().map(|&p| ((p & 0x01ff_ffff) >> 2) as usize));
        c.mem.rearm_tick(0);
    }
    for i in idx {
        c.stub_bits[i >> 6] |= 1 << (i & 63);
    }
}

/// Where a bus fault happened: the instruction `pc` just executed (or the DMA engine if the pc did not touch memory),
/// the registers and the DMA channel registers.
fn fault_info(c: &mut Cpu, pc: u32) -> String {
    let mut s = format!("fault-time pc {pc:08x} op {:08x} icount {}
", c.mem.fetch(pc), c.icount);
    s += &format!("gpr: {}
", (0..32).map(|i| format!("{i}={:x}", c.g(i) as u64)).collect::<Vec<_>>().join(" "));
    s += &format!("fpr: {}
", (0..32).map(|i| format!("{i}={:x}", c.f[i])).collect::<Vec<_>>().join(" "));
    s += &format!("vu0_stats {:?} vu0m_gen {}
", c.vu0_stats, c.mem.vu0m_gen);
    for l in c.mem.watch_log.iter() {
        s += &format!("  watch: {l}
");
    }
    s += &format!("last pcs: {}
", (0..32).map(|i| format!("{:x}", c.ring[(c.ri + 32 + i) & 63])).collect::<Vec<_>>().join(" "));
    for (n, b) in [("VIF0", 0x1000_8000u32), ("VIF1", 0x1000_9000), ("GIF", 0x1000_a000), ("SPR_FROM", 0x1000_d000), ("SPR_TO", 0x1000_d400)] {
        s += &format!(
            "{n}: chcr {:08x} madr {:08x} qwc {:08x} tadr {:08x} asr0 {:08x} asr1 {:08x} sadr {:08x}
",
            c.mem.hw32(b), c.mem.hw32(b + 0x10), c.mem.hw32(b + 0x20), c.mem.hw32(b + 0x30), c.mem.hw32(b + 0x40), c.mem.hw32(b + 0x50), c.mem.hw32(b + 0x80)
        );
    }
    s
}

impl Kernel {
    /// Hook rules only, for a savestate machine (the BIOS in RAM stays in charge): `text` is a rule file's content.
    pub fn hooks_only(text: &str) -> Kernel {
        let mut k = Kernel { hooks_only: true, ..Default::default() };
        let mut hk = vec![false; 1 << 23];
        read_hooks(text, &mut k, &mut hk);
        k.hooked = hk;
        k
    }

    /// Text of the counters a run collects (syscalls seen, interrupts delivered, handler lists).
    pub fn report(&self) -> String {
        let mut s: Vec<_> = self.sys_seen.iter().collect();
        s.sort();
        let mut u: Vec<_> = self.unhandled.iter().collect();
        u.sort();
        let mut q: Vec<_> = self.irq_cnt.iter().collect();
        q.sort();
        format!(
            "{}syscalls seen: {}\nleft to hle.rs (return 0): {}\nirqs (dmac?, cause): {:?}\nintc handlers: {:?}\ndmac handlers: {:?}",
            self.fault_info.clone().unwrap_or_default(),
            s.iter().map(|(n, v)| format!("{:#x}:{v}", n.unsigned_abs())).collect::<Vec<_>>().join(" "),
            u.iter().map(|(n, v)| format!("{:#x}:{v}", n.unsigned_abs())).collect::<Vec<_>>().join(" "),
            q,
            self.intc.iter().map(|h| (h.cause, format!("{:x}", h.func))).collect::<Vec<_>>(),
            self.dmac.iter().map(|h| (h.cause, format!("{:x}", h.func))).collect::<Vec<_>>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::parse_language;

    #[test]
    fn language_names_and_numbers() {
        for (s, osd) in [("en", 1), ("English", 1), ("fr", 2), (" FRENCH ", 2), ("es", 3), ("de", 4), ("Deutsch", 4), ("it", 5), ("italiano", 5)] {
            assert_eq!(parse_language(s), Some(osd), "{s}");
        }
        for n in 0..32u32 {
            assert_eq!(parse_language(&n.to_string()), Some(n));
        }
        for s in ["", "xx", "32", "-1", "1.5", "english2"] {
            assert_eq!(parse_language(s), None, "{s}");
        }
    }
}
