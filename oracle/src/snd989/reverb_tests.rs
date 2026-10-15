//! Tests of reverb.rs (stage 10e): allocator, fixed block, pending countdown, depth fade, errors, save / load, census states.
use super::*;
use crate::snd989::adopt::live_base;
use crate::snd989::driver::{rng_from_iop, Inner, Snd989};
use crate::snd989::rng::{Rng250, LEN};
use crate::snd989::testdata;

fn ops_of(f: impl FnOnce(&mut Vec<FxOp>)) -> Vec<FxOp> {
    let mut v = Vec::new();
    f(&mut v);
    v
}

fn regs(ops: &[FxOp], r: Reg, core: u8) -> Vec<u32> {
    ops.iter().filter(|o| o.reg == r && o.index == core).map(|o| o.value).collect()
}

/// The ESA libsd derives from the EEA value the driver passes: the register keeps the 128 KiB page, ESA = page end - size + 1.
fn esa_of(eea_arg: u32, size: u32) -> u32 {
    (((eea_arg >> 17) << 17) | 0x1ffff) - size + 1
}

fn boot() -> (Reverb, Spu2Alloc) {
    let (mut r, mut m) = (Reverb::new(), Spu2Alloc::default());
    r.cid81(&mut m, 2, 4, &mut Vec::new());
    (r, m)
}

#[test]
fn effect_allocator_cuts_the_top_page() {
    let mut a = Spu2Alloc::default();
    assert_eq!(a.alloc_effect(0x6fe0), Some(0x1f9000), "0x6fe0 rounds to 0x7000");
    assert_eq!(a.regions(), &[(0x5040, 0x1f9000)]);
    a.release(0x1f9000, 0x6fe0);
    assert_eq!(a.regions(), &[(0x5040, 0x20_0000)], "free merges back");
    // best fit: the smaller region that qualifies wins
    let mut a = Spu2Alloc::default();
    assert!(a.carve(0x50000, 0x30000));
    assert_eq!(a.regions(), &[(0x5040, 0x50000), (0x80000, 0x20_0000)]);
    // the first region ends at 0x50000: boundary 0x40000, 0x3afc0 bytes below it; the second ends at the top
    assert_eq!(a.alloc_effect(0x18040), Some(0x40000 - 0x18040), "the smaller region qualifies and is chosen");
    // what is left of region one, [0x5040, 0x27fc0), still has 0x1afc0 bytes below its boundary 0x20000 and is the smallest
    assert_eq!(a.alloc_effect(0x1_0000), Some(0x1_0000));
    // a block that does not fit below the boundary of the only region
    let mut b = Spu2Alloc::default();
    assert!(b.carve(0x5040, 0x1f_0000 - 0x5040));
    assert_eq!(b.alloc_effect(0x2_0000), None, "region [0x1f0000, 0x200000) has 64 KiB below the boundary");
}

#[test]
fn cid81_reserves_the_fixed_block_of_core_one() {
    let (mut r, mut m) = boot();
    assert_eq!((r.fixed, r.area[1], r.mode), ([-1, 4], 0x1f9000, [-1, -1]));
    assert_eq!(m.regions(), &[(0x5040, 0x1f9000)]);
    assert_eq!(r.owned(1), Some((0x1f9000, 0x6fe0)));
    // replacing it frees the old block first
    r.cid81(&mut m, 2, 3, &mut Vec::new());
    assert_eq!((r.fixed[1], r.area[1]), (3, 0x20_0000 - 0x4840));
    assert_eq!(m.regions(), &[(0x5040, 0x20_0000 - 0x4840)]);
}

#[test]
fn cid80_in_the_fixed_block_gives_the_retail_esa() {
    // PROVED on SPU2 RAM dumps: fg1/1_0038 (mode 3, 6000) first nonzero word at 0x1fb7c0; fg1/11_0421 (mode 4) at 0x1f9020
    for (mode, esa) in [(3, 0x1fb7c0), (4, 0x1f9020)] {
        let (mut r, mut m) = boot();
        let ops = ops_of(|o| r.cid80(&mut m, 2, mode, 6000, 0, 0, o));
        let eea = regs(&ops, Reg::Eea, 1);
        assert_eq!(eea.len(), 1);
        assert_eq!(esa_of(eea[0], r.sizes[mode as usize]), esa, "mode {mode}");
        assert_eq!(r.area[1], 0x1f9000, "the fixed block stays");
        assert_eq!(regs(&ops, Reg::FxClear, 1), vec![mode as u32]);
        // driver order: EEA, SetEffectAttr (EVOL 0), clear; the depth write is held back by the pending clear
        let order: Vec<Reg> = ops.iter().map(|o| o.reg).collect();
        assert_eq!(order, [Reg::Eea, Reg::FxAttr, Reg::EvolL, Reg::EvolR, Reg::FxClear]);
        assert_eq!(r.depth[1], [6000, 6000]);
    }
}

#[test]
fn pending_clear_lasts_ten_ticks() {
    let (mut r, mut m) = boot();
    r.cid80(&mut m, 2, 3, 6000, 0, 0, &mut Vec::new());
    for t in 1..=9 {
        let ops = ops_of(|o| r.tick(o));
        assert!(ops.is_empty(), "tick {t}");
    }
    let ops = ops_of(|o| r.tick(o));
    assert_eq!((regs(&ops, Reg::EvolL, 1), regs(&ops, Reg::EvolR, 1), regs(&ops, Reg::FxOn, 1)), (vec![6000], vec![6000], vec![1]));
    assert_eq!(r.chan[1], 0);
    // same type again: SetEffectAttr only (EVOL 0), then the depth at once
    let ops = ops_of(|o| r.cid80(&mut m, 2, 3, 3000, 0, 0, o));
    let order: Vec<(Reg, u32)> = ops.iter().map(|o| (o.reg, o.value)).collect();
    assert_eq!(order, [(Reg::FxAttr, 3), (Reg::EvolL, 0), (Reg::EvolR, 0), (Reg::EvolL, 3000), (Reg::EvolR, 3000)]);
}

#[test]
fn type_change_switches_the_effect_off_first() {
    let (mut r, mut m) = boot();
    r.cid80(&mut m, 2, 3, 6000, 0, 0, &mut Vec::new());
    for _ in 0..10 {
        r.tick(&mut Vec::new());
    }
    let ops = ops_of(|o| r.cid80(&mut m, 2, 4, 5000, 0, 0, o));
    let order: Vec<Reg> = ops.iter().map(|o| o.reg).collect();
    assert_eq!(order, [Reg::FxOn, Reg::EvolL, Reg::EvolR, Reg::Eea, Reg::FxAttr, Reg::EvolL, Reg::EvolR, Reg::FxClear]);
    assert_eq!(ops[0].value, 0);
}

#[test]
fn depth_fades_by_delta_over_rate_per_tick() {
    let (mut r, mut m) = boot();
    r.cid80(&mut m, 2, 3, 0, 0, 0, &mut Vec::new());
    for _ in 0..10 {
        r.tick(&mut Vec::new());
    }
    // the game's {2, depth, 12, 3}: 6000 / 12 = 500 per tick on both sides
    r.cid16(2, 6000, 12, 3, &mut Vec::new());
    assert_eq!((r.fade[1][0].step, r.fade[1][1].step, r.fade[1][0].target), (500, 500, 6000));
    let mut seen = Vec::new();
    for _ in 0..13 {
        let ops = ops_of(|o| r.tick(o));
        seen.push(regs(&ops, Reg::EvolL, 1).first().copied());
    }
    let want: Vec<Option<u32>> = (1..=12).map(|k| Some(500 * k)).chain([None]).collect();
    assert_eq!(seen, want);
    assert_eq!((r.fade[1][0].step, r.depth[1]), (0, [6000, 6000]));
    // fade down: delta -6000 / 12 = -500, both sides; side 1 alone starts only the left
    r.cid16(2, 0, 12, 3, &mut Vec::new());
    assert_eq!((r.fade[1][0].step, r.fade[1][1].step), (-500, -500));
    r.cid16(2, 5, 12, 1, &mut Vec::new());
    assert_eq!((r.fade[1][0].step, r.fade[1][0].target, r.fade[1][1].target), (-499, 5, 0));
    // |delta| < rate: one unit per tick
    let (mut r2, _) = boot();
    r2.depth[1] = [10, 10];
    r2.cid16(2, 4, 12, 3, &mut Vec::new());
    assert_eq!((r2.fade[1][0].step, r2.fade[1][1].step), (-1, -1));
    // delta 7000 / 12 = 583; the 13th step is clamped to the target
    let (mut r3, _) = boot();
    r3.cid16(2, 7000, 12, 3, &mut Vec::new());
    assert_eq!(r3.fade[1][0].step, 583);
    for _ in 0..12 {
        r3.tick(&mut Vec::new());
    }
    assert_eq!(r3.depth[1], [6996, 6996], "12 steps of 583");
    r3.tick(&mut Vec::new());
    assert_eq!((r3.depth[1], r3.fade[1][0].step), ([7000, 7000], 0), "the 13th step clamps to the target and ends the fade");
    // rate 0: immediate (left, then right keeps the new left); no change: nothing
    let ops = ops_of(|o| r3.cid16(2, 100, 0, 3, o));
    assert_eq!((regs(&ops, Reg::EvolL, 1), regs(&ops, Reg::EvolR, 1)), (vec![100, 100], vec![7000, 100]));
    let ops = ops_of(|o| r3.cid16(2, 100, 12, 3, o));
    assert!(ops.is_empty() && r3.fade[1][0].step == 0);
}

#[test]
fn errors_leave_the_state_as_the_driver_does() {
    // fixed block 3 (0x4840) cannot hold mode 4 (0x6fe0): error 0x57, -13, nothing changed
    let (mut r, mut m) = (Reverb::new(), Spu2Alloc::default());
    r.cid81(&mut m, 2, 3, &mut Vec::new());
    r.cid80(&mut m, 2, 3, 100, 0, 0, &mut Vec::new());
    let before = (r.clone(), m.clone());
    let mut ops = Vec::new();
    assert_eq!(r.change(&mut m, 1, 4, 0, 0, &mut ops), -13);
    assert!(ops.is_empty());
    assert_eq!((r.last_error, r.mode[1], r.area[1]), (ERR_TOO_SMALL, 3, before.0.area[1]));
    assert_eq!(m, before.1);
    // dynamic core 0 with no memory: error 0x56, -11, mode -1, area 0
    let (mut r, mut m) = (Reverb::new(), Spu2Alloc::default());
    assert!(m.carve(0x5040, 0x20_0000 - 0x5040));
    assert_eq!(r.change(&mut m, 0, 4, 0, 0, &mut Vec::new()), -11);
    assert_eq!((r.last_error, r.mode[0], r.area[0]), (ERR_ALLOC, -1, 0));
    // a fixed block that cannot be had: mode = fixed = -1
    r.cid81(&mut m, 1, 4, &mut Vec::new());
    assert_eq!((r.last_error, r.mode[0], r.fixed[0]), (ERR_FIXED_ALLOC, -1, -1));
}

#[test]
fn dynamic_block_follows_the_mode() {
    let (mut r, mut m) = (Reverb::new(), Spu2Alloc::default());
    assert_eq!(r.change(&mut m, 0, 4, 0, 0, &mut Vec::new()), 0);
    assert_eq!(r.area[0], 0x1f9000);
    assert_eq!(r.change(&mut m, 0, 5, 0, 0, &mut Vec::new()), 0);
    assert_eq!(r.area[0], 0x20_0000 - 0xae00, "0xade0 rounds to 0xae00");
    assert_eq!(r.change(&mut m, 0, 0, 0, 0, &mut Vec::new()), 0);
    assert_eq!(m.regions(), &[(0x5040, 0x20_0000)], "mode 0 frees the block");
}

#[test]
fn save_and_load_round_trip() {
    let (mut r, mut m) = boot();
    r.cid80(&mut m, 2, 4, 1234, 5, 6, &mut Vec::new());
    r.cid16(2, 4000, 12, 3, &mut Vec::new());
    r.tick(&mut Vec::new());
    let mut o = Out::new();
    r.save(&mut o);
    let mut i = In::new(&o.0);
    let back = Reverb::load(&mut i);
    assert!(i.done());
    assert_eq!(back, r);
}

fn rng0() -> Rng250 {
    rng_from_iop(&[]).unwrap_or_else(|| Rng250::from_state([0; LEN], 0).unwrap())
}

#[test]
fn commands_reach_the_events_and_the_core_image() {
    let mut i = Inner::blank(rng0());
    i.record = true;
    assert_eq!(i.command(0, 81, &[2, 4, 0, 0, 0, 0, 0], &mut None), Some(u32::MAX));
    assert_eq!(i.command(0, 80, &[2, 3, 6000, 0, 0, 0, 0], &mut None), Some(u32::MAX));
    assert!(!i.core.core[1].effect_enable);
    let order: Vec<Reg> = i.out.writes.iter().map(|w| w.reg).collect();
    assert_eq!(order, [Reg::Eea, Reg::FxAttr, Reg::EvolL, Reg::EvolR, Reg::FxClear]);
    // field 3 = tick 14: the 10 pending ticks are over
    i.advance_to(3);
    assert!(i.core.core[1].effect_enable);
    assert_eq!(i.core.core[1].evol, [6000, 6000]);
    let on: Vec<u32> = i.out.writes.iter().filter(|w| w.reg == Reg::FxOn).map(|w| w.tick).collect();
    assert_eq!(on, [10], "enabled at the 10th driver tick after the command");
    assert_eq!(i.command(3, 16, &[2, 0, 12, 3, 0, 0, 0], &mut None), Some(u32::MAX));
    assert_eq!(i.rev.fade[1][0].step, -500);
    // the reverb state survives the savestate section
    let want = i.rev.clone();
    let s = Snd989 { inner: Some(Box::new(i)), record: true, spuv: Vec::new() };
    let bytes = s.save();
    let mut t = Snd989 { inner: None, record: false, spuv: Vec::new() };
    t.load(&bytes);
    assert_eq!(t.inner.as_ref().unwrap().rev, want);
}

/// Census of the savestates: the live module's variables (IOP RAM) give the same state the boot sequence builds.
#[test]
fn live_module_variables_of_the_savestates() {
    for (name, mode, depth) in [("fg1/1_0038", 3, 6000u16), ("fg1/11_0421", 4, 5000)] {
        let (Some(iop), Some(ee)) = (testdata::read_state(&format!("{name}/iopMemory.bin")), testdata::read_state(&format!("{name}/eeMemory.bin"))) else {
            println!("note: state {name} absent, skipped");
            continue;
        };
        let Some(base) = live_base(&iop) else { panic!("{name}: no live module") };
        let mut i = Inner::blank(rng0());
        i.boot(&ee);
        i.adopt_reverb(&iop, base);
        let r = &i.rev;
        println!("{name}: mode {:?} depth {:?} chan {:?} pending {:?} fades {:?}", r.mode, r.depth, r.chan, r.pending, r.fade);
        assert_eq!(r.sizes, SIZES, "{name}: the module's size table");
        assert_eq!((r.fixed, r.area[1]), ([-1, 4], 0x1f9000), "{name}");
        assert_eq!((r.mode[1], r.depth[1]), (mode, [depth, depth]), "{name}");
        assert_eq!(i.spu_mem.regions().last().map(|r| r.1), Some(0x1f9000), "{name}: allocator holds the fixed block");
        assert_eq!(i.core.core[1].evol, [depth, depth]);
        assert!(i.core.core[1].effect_enable);
    }
}

/// Every savestate with a live module (fg1, fg2, ...): core 1 holds the fixed mode-4 block at 0x1f9000, core 0 is untouched, only
/// the modes 0 / 3 / 4 occur, the pending clear and the fades are idle or consistent, and the allocator rebuilt from the module
/// ends at the block.
#[test]
fn census_of_all_savestates() {
    let Some(dir) = testdata::states() else { return };
    let (mut seen, mut modes) = (0, std::collections::BTreeSet::new());
    let mut dirs: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    dirs.push(dir.clone());
    for sub in dirs {
        let Ok(rd) = std::fs::read_dir(&sub) else { continue };
        for e in rd.flatten() {
            let Ok(iop) = std::fs::read(e.path().join("iopMemory.bin")) else { continue };
            let Some(base) = live_base(&iop) else { continue };
            let mut i = Inner::blank(rng0());
            i.boot(&vec![0u8; 0x15ee00]);
            i.adopt_reverb(&iop, base);
            let (r, n) = (&i.rev, e.file_name());
            assert_eq!((r.fixed, r.area[1], r.mode[0]), ([-1, 4], 0x1f9000, -1), "{n:?}");
            assert!(r.pending[1] <= 10 && r.sizes == SIZES, "{n:?}");
            assert_eq!(i.spu_mem.regions().last().map(|x| x.1), Some(0x1f9000));
            modes.insert(r.mode[1]);
            seen += 1;
        }
    }
    println!("census: {seen} states, core-1 modes {modes:?}");
    assert!(seen == 0 || seen >= 30);
}
