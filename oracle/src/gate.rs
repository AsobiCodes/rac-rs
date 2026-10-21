//! The savestate gate: run retail R&C1 EE code from a PCSX2 savestate.
//!
//!   ee-oracle run STATE_A STATE_B [--spin N] [--div near|chop]
//!
//! Resumes A, runs the game until its frame counter equals B's at the vblank
//! wait, then diffs EE RAM / scratchpad / mobys against B.

use crate::{fpu, load, load_raw, load_stubs};
use std::collections::HashMap;
use std::time::Instant;

const FRAME: u32 = 0x15f5ac;
const LEVEL: u32 = 0x15ed84;
/// Vblank count, +1 in the game's VBLANK_S handler 0x12f1c8 (boot).
const VBCOUNT: u32 = 0x15ed48;
const HERO: u32 = 0x1413d0;

fn r32(m: &[u8], a: u32) -> u32 {
    let i = a as usize;
    u32::from_le_bytes(m[i..i + 4].try_into().unwrap())
}
fn rf(m: &[u8], a: u32) -> f32 {
    f32::from_bits(r32(m, a))
}

fn top<K: std::fmt::LowerHex + Copy>(h: &HashMap<K, u64>, n: usize) -> Vec<(K, u64)> {
    let mut v: Vec<(K, u64)> = h.iter().map(|(k, c)| (*k, *c)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.truncate(n);
    v
}

fn diff_region(name: &str, a: &[u8], o: &[u8], b: &[u8], lo: usize, hi: usize, bucket: usize) {
    let (mut changed, mut repro, mut missed, mut spur) = (0u64, 0u64, 0u64, 0u64);
    let mut rows = Vec::new();
    let mut k = lo;
    while k < hi {
        let e = (k + bucket).min(hi);
        let (mut c, mut m, mut s) = (0u64, 0u64, 0u64);
        for i in k..e {
            if a[i] != b[i] {
                c += 1;
                if o[i] == b[i] {
                    repro += 1
                } else {
                    m += 1
                }
            } else if o[i] != b[i] {
                s += 1
            }
        }
        changed += c;
        missed += m;
        spur += s;
        if m + s > 0 {
            rows.push((k, c, m, s));
        }
        k = e;
    }
    println!(
        "{name}: real changes {changed}, reproduced {repro} ({:.1}%), missed {missed}, spurious {spur}",
        if changed > 0 { 100.0 * repro as f64 / changed as f64 } else { 100.0 }
    );
    rows.sort_by(|x, y| (y.2 + y.3).cmp(&(x.2 + x.3)));
    for (k, c, m, s) in rows.iter().take(25) {
        println!("  {k:08x}+{bucket:x}: changed {c} missed {m} spurious {s}");
    }
}

fn mobys(a: &[u8], o: &[u8], b: &[u8], lvl: u8) {
    let (sa, ea) = if lvl == 0 { (0x15ff98, 0x15ffa0) } else { (0x15ff58, 0x15ff60) };
    let (base, end) = (r32(b, sa), r32(b, ea));
    if base == 0 || end <= base || end as usize > b.len() {
        println!("mobys: bad range {base:08x}..{end:08x}");
        return;
    }
    let (mut live, mut moved, mut exact, mut close, mut differ, mut statediff) = (0, 0, 0, 0, 0, 0);
    let mut worst: Vec<(f32, u32, u16)> = Vec::new();
    let mut p = base;
    while p < end {
        let st = b[p as usize + 0x20];
        if st < 0xfd {
            live += 1;
            let pb = [rf(b, p + 0x10), rf(b, p + 0x14), rf(b, p + 0x18)];
            let po = [rf(o, p + 0x10), rf(o, p + 0x14), rf(o, p + 0x18)];
            let pa = [rf(a, p + 0x10), rf(a, p + 0x14), rf(a, p + 0x18)];
            if pa != pb {
                moved += 1;
            }
            let d = ((pb[0] - po[0]).powi(2) + (pb[1] - po[1]).powi(2) + (pb[2] - po[2]).powi(2)).sqrt();
            if (0..12).all(|i| b[(p + 0x10) as usize + i] == o[(p + 0x10) as usize + i]) {
                exact += 1;
            } else if d < 1e-3 {
                close += 1;
            } else {
                differ += 1;
                let cls = u16::from_le_bytes([b[p as usize + 0xa6], b[p as usize + 0xa7]]);
                worst.push((if d.is_nan() { f32::MAX } else { d }, p, cls));
            }
            if o[p as usize + 0x20] != st {
                statediff += 1;
            }
        }
        p += 0x100;
    }
    println!(
        "mobys {base:08x}..{end:08x}: live {live}, moved A->B {moved}, pos exact {exact}, <1mm {close}, differ {differ}, state byte differs {statediff}"
    );
    worst.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap());
    for (d, p, c) in worst.iter().take(10) {
        let i = (p - base) / 0x100;
        println!("  slot {i} @{p:08x} class {c}: off by {d:.4}");
    }
}

pub fn main() {
    // The access and call counts in the report below.
    crate::mem::STATS.store(true, std::sync::atomic::Ordering::Relaxed);
    // Recompiled code nests a Rust frame per retail call.
    std::thread::Builder::new().stack_size(256 << 20).spawn(run).unwrap().join().unwrap();
}

fn run() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 || args[1] != "run" {
        eprintln!("usage: ee-oracle run STATE_A STATE_B [--spin N] [--div near|chop]");
        std::process::exit(2);
    }
    let mut spin = 200_000_000u64;
    let mut div_chop = false;
    let mut i = 4;
    while i < args.len() {
        match args[i].as_str() {
            "--spin" => {
                spin = args[i + 1].parse().unwrap();
                i += 1;
            }
            "--div" => {
                div_chop = args[i + 1] == "chop";
                i += 1;
            }
            x => panic!("unknown arg {x}"),
        }
        i += 1;
    }
    let mut cpu = load(&args[2]);
    // B is only compared against: retail's RAM as saved (no in-flight RPC completion).
    let bstate = load_raw(&args[3]);
    if div_chop {
        cpu.div_csr = fpu::CSR_CHOP;
    }
    load_stubs(&mut cpu, "stubs.txt");
    let a_ram = cpu.mem.ram.clone();
    let a_spr = cpu.mem.spr.clone();
    let b = &bstate.mem;
    let target = r32(&b.ram, FRAME);
    let start = r32(&a_ram, FRAME);
    let lvl = b.ram[LEVEL as usize];
    let target_vb = r32(&b.ram, VBCOUNT);
    let start_vb = r32(&a_ram, VBCOUNT);
    println!("A frame {start} pc {:08x} | B frame {target} pc {:08x} | level {lvl}", cpu.pc, bstate.pc);

    let t0 = Instant::now();
    let mut hist: HashMap<u32, u64> = HashMap::new();
    // A's own SetVSyncFlag registration (if any) is kernel state in RAM, written by the BIOS INTC handler (Machine::load).
    cpu.vsync = None;
    if let Ok(n) = std::env::var("ORACLE_TRACE") {
        for _ in 0..n.parse::<u32>().unwrap() {
            let op = cpu.mem.fetch(cpu.pc);
            println!("{:08x} {op:08x} v0 {:x} v1 {:x} a0 {:x} a1 {:x} k0 {:x} at {:x}", cpu.pc, cpu.g(2), cpu.g(3), cpu.g(4), cpu.g(5), cpu.g(26), cpu.g(1));
            cpu.step();
            if cpu.stop.is_some() { break; }
        }
        return;
    }
    if std::env::var("ORACLE_REC").is_ok_and(|v| v == "1") {
        let t = crate::rec::Table::build(&cpu.mem, cpu.mem.ram[LEVEL as usize] as u32);
        println!("rec: {} functions", t.len());
        cpu.rec = Some(Box::new(t));
    }
    // Each armed vblank wait: log it, stop at B's vblank count (before raising it), else raise the vblank and go on.
    #[derive(Default)]
    struct Waits {
        n: u64,
        log: Vec<(u32, u64)>,
        done: bool,
        last: u64,
    }
    struct Quit;
    let st = std::sync::Arc::new(std::sync::Mutex::new(Waits { last: cpu.icount, ..Default::default() }));
    let cb_st = st.clone();
    cpu.vblank_cb = Some(Box::new(move |cpu: &mut crate::cpu::Cpu| {
        let mut w = cb_st.lock().unwrap();
        let f = cpu.mem.rd32(FRAME);
        let vb = cpu.mem.rd32(VBCOUNT);
        w.n += 1;
        if w.n % 50 == 0 {
            eprintln!("vblank {vb} frame {f} insns {}M", cpu.icount / 1_000_000);
        }
        w.log.push((f, cpu.icount));
        w.last = cpu.icount;
        if std::env::var("ORACLE_DBG").is_ok() {
            let wh: Vec<String> = cpu.watch_hits.iter().map(|(a, n)| format!("{a:x}:{n}")).collect();
            let pk: Vec<String> = std::env::var("ORACLE_PEEK").unwrap_or_default().split(',').filter(|x| !x.is_empty()).map(|x| {
                let a = u32::from_str_radix(x.trim_start_matches("0x"), 16).unwrap();
                format!("{a:x}={:x}", cpu.mem.rd32(a))
            }).collect();
            eprintln!("wait vb {vb} frame {f} insns {} watch {} peek {}", cpu.icount, wh.join(" "), pk.join(" "));
            cpu.watch_hits.clear();
        }
        if vb == target_vb {
            w.done = true;
            drop(w);
            std::panic::resume_unwind(Box::new(Quit));
        }
        if vb.wrapping_sub(start_vb) > target_vb.wrapping_sub(start_vb) {
            println!("vblank count passed target: {vb}");
            drop(w);
            std::panic::resume_unwind(Box::new(Quit));
        }
        drop(w);
        cpu.vblank();
    }));
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| loop {
        cpu.run_rec(4096);
        hist.entry(cpu.pc).and_modify(|c| *c += 1).or_insert(1);
        if let Some(s) = &cpu.stop {
            println!("STOP: {s}");
            break;
        }
        if let Some(f) = &cpu.mem.fault {
            println!("FAULT: {f} (pc {:08x})", cpu.pc);
            break;
        }
        let since = cpu.icount - st.lock().unwrap().last;
        if since > spin {
            println!("SPIN: {since} insns without reaching the vblank wait; top PCs:");
            for (pc, n) in top(&hist, 20) {
                println!("  {pc:08x} {n}");
            }
            break;
        }
        if since < 4096 {
            hist.clear();
        }
    }));
    cpu.vblank_cb = None;
    let (waits, frames_log, done) = {
        let w = st.lock().unwrap();
        (w.n, w.log.clone(), w.done)
    };
    let dt = t0.elapsed().as_secs_f64();
    println!(
        "{} : {} insns in {:.2}s ({:.1} MIPS), {} vblank waits, frame {}",
        if done { "REACHED B" } else { "NOT REACHED" },
        cpu.icount,
        dt,
        cpu.icount as f64 / dt / 1e6,
        waits,
        cpu.mem.rd32(FRAME)
    );
    if !done {
        let ring: Vec<String> = (0..64).map(|k| format!("{:08x}", cpu.ring[(cpu.ri + k) & 63])).collect();
        println!("last pcs: {}", ring.join(" "));
        println!("ra {:08x} sp {:08x}", cpu.g(31) as u32, cpu.g(29) as u32);
    }
    let steps: Vec<String> = frames_log
        .windows(2)
        .take(12)
        .map(|w| format!("{}:{}k", w[1].0, (w[1].1 - w[0].1) / 1000))
        .collect();
    println!("frames (counter:insns): {}", steps.join(" "));
    let mut fr: Vec<u32> = frames_log.windows(2).map(|w| w[1].0.wrapping_sub(w[0].0)).collect();
    fr.sort();
    fr.dedup();
    println!("counter steps per wait: {fr:?}");
    let mut sc: Vec<(i32, u64)> = cpu.syscalls.iter().map(|(k, v)| (*k, *v)).collect();
    sc.sort();
    println!("syscalls: {}", sc.iter().map(|(k, v)| format!("{k:#x}:{v}")).collect::<Vec<_>>().join(" "));
    println!("interrupts: intc {} dmac {} | vif1 stops {}", cpu.irqs[0], cpu.irqs[1], cpu.mem.vif1_stops);
    println!(
        "vu0 micro calls: {}",
        top(&cpu.vu_calls, 12).iter().map(|(a, n)| format!("{a:04x}:{n}")).collect::<Vec<_>>().join(" ")
    );
    for (pc, n) in top(&cpu.stub_hits, 20) {
        println!("stub {pc:08x} {} x{n}", cpu.stubs[&pc].name);
    }
    println!(
        "hw reads: {}",
        top(&cpu.mem.hw_reads, 12).iter().map(|(a, n)| format!("{a:08x}:{n}")).collect::<Vec<_>>().join(" ")
    );
    println!(
        "hw writes: {}",
        top(&cpu.mem.hw_writes, 12).iter().map(|(a, n)| format!("{a:08x}:{n}")).collect::<Vec<_>>().join(" ")
    );
    if std::env::var("ORACLE_DBG").is_ok() {
        let mut k: Vec<_> = cpu.mem.kick_pcs.iter().filter(|((c, _), _)| *c != 0x1000_d000 && *c != 0x1000_d400).collect();
        k.sort();
        for ((c, pc), n) in k {
            println!("kick {c:08x} from {pc:08x} x{n}");
        }
    }
    println!("vif0: mpg {} unpack {}", cpu.mem.vif0.mpg_uploads, cpu.mem.vif0.unpacks);
    println!(
        "dma kicks: {}",
        top(&cpu.mem.dma_kicks, 12).iter().map(|(a, n)| format!("{a:08x}:{n}")).collect::<Vec<_>>().join(" ")
    );

    if let Ok(d) = std::env::var("ORACLE_DUMP") {
        std::fs::write(format!("{d}.ram"), &cpu.mem.ram).unwrap();
        std::fs::write(format!("{d}.spr"), &cpu.mem.spr).unwrap();
    }
    let o = &cpu.mem.ram;
    diff_region("EE RAM 0x100000-0x2000000", &a_ram, o, &b.ram, 0x10_0000, 0x200_0000, 0x10000);
    diff_region("scratchpad", &a_spr, &cpu.mem.spr, &b.spr, 0, 0x4000, 0x400);
    mobys(&a_ram, o, &b.ram, lvl);
    let hp = r32(&b.ram, HERO);
    if hp != 0 && (hp as usize) < o.len() {
        let pos = |m: &[u8]| [rf(m, hp + 0x10), rf(m, hp + 0x14), rf(m, hp + 0x18)];
        println!("hero A {:?} B {:?} ours {:?}", pos(&a_ram), pos(&b.ram), pos(o));
    }
    let gd: Vec<String> = (1..32)
        .filter(|&k| cpu.r[k] != bstate.r[k])
        .map(|k| format!("r{k}"))
        .collect();
    println!("GPRs differing from B: {}", gd.join(" "));
}
