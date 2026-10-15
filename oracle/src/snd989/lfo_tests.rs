use super::*;
use crate::snd989::bank::{Bank, Grain, Signature, Sound};
use crate::snd989::handler::{Driver, Handler, PlayArgs, RecVoices, VoiceEvent};
use crate::snd989::irx::{Irx, MODULE_989SND};
use crate::snd989::pitch::Note2PitchTables;
use crate::snd989::testdata;
use crate::snd989::voices::{PanTable, Reg, SpuVoices};
use std::sync::Arc;

#[path = "lfo_golden.rs"]
mod golden;
use golden::{Cfg, Scenario};

type L = Driver<RecVoices, Lfos>;

fn fnv16(vals: impl Iterator<Item = i32>) -> u32 {
    let mut h = 0x811c_9dc5u32;
    for v in vals {
        for b in (v as u16).to_le_bytes() {
            h = (h ^ b as u32).wrapping_mul(0x0100_0193);
        }
    }
    h
}

fn disc_irx() -> Option<Vec<u8>> {
    let blob = testdata::read("globals/irx.bin")?;
    Irx::find_in_blob(&blob, MODULE_989SND).map(|b| b.to_vec())
}
fn disc_rng() -> Option<Rng250> {
    let b = disc_irx()?;
    Rng250::from_irx(&Irx::parse(&b).ok()?).ok()
}

fn lfo_grain(c: &Cfg) -> Grain {
    let mut p = [0u8; 32];
    p[0] = c.slot;
    p[1] = c.target;
    p[2] = c.link;
    p[3] = c.wave as u8;
    p[4..6].copy_from_slice(&c.width.to_le_bytes());
    p[6..8].copy_from_slice(&c.depth.to_le_bytes());
    p[8] = c.flags;
    p[0xc..0x10].copy_from_slice(&c.rate.to_le_bytes());
    p[0x12..0x14].copy_from_slice(&c.phase.to_le_bytes());
    Grain { kind: 4, delay: 0, p }
}
fn plain(kind: u32, delay: u16) -> Grain {
    Grain { kind, delay, p: [0; 32] }
}
fn tone_grain(delay: u16) -> Grain {
    let mut p = [0u8; 32];
    p[1] = 100;
    p[4..6].copy_from_slice(&180i16.to_le_bytes());
    Grain { kind: 1, delay, p }
}
fn bank_with(sound_volume: i8, grains: Vec<Grain>) -> Arc<Bank> {
    let s = Sound { volume: sound_volume, group: 0, pan: 90, pad5: 0, flags: 0, grain_offset: 0, grains };
    Arc::new(Bank { signature: Signature { id: 0, sounds: 1, grains: 0, vags: 0 }, sounds: vec![s], data: vec![] })
}
fn args() -> PlayArgs {
    PlayArgs::cid17(0x7fff_ffff, -1, 0, 0)
}

fn test_rng() -> Rng250 {
    let mut t = [0u16; 250];
    for (i, x) in t.iter_mut().enumerate() {
        *x = (i as u16).wrapping_mul(40503).wrapping_add(7);
    }
    Rng250::from_state(t, 0).unwrap()
}

fn offsets(h: &Handler) -> [i32; 4] {
    [h.vol_off as i32, h.pan_off as i32, h.pmod_off as i32, h.bend_off as i32]
}

#[test]
fn sine_formula_equals_golden_and_disc() {
    let s = sine_formula();
    assert_eq!(s.len(), 2048);
    assert_eq!(fnv16(s.iter().map(|&v| v as i32)), golden::SINE_FNV);
    for (i, &p) in golden::PHASES.iter().enumerate() {
        assert_eq!(s[p as usize] as i32, golden::SINE_AT[i]);
    }
    assert_eq!((s[0], s[512], s[1024], s[1536]), (0x7fff, 0, -0x7fff, 0), "the table is a cosine: phase 0 is the peak");
    if let Some(b) = disc_irx() {
        let irx = Irx::parse(&b).unwrap();
        assert_eq!(sine_from_irx(&irx).unwrap(), s, "formula == the module's table");
    } else {
        eprintln!("skip disc part: extracted files absent");
    }
}

#[test]
fn deterministic_waves_match_the_replay() {
    let mut rng = test_rng();
    let sine = sine_formula();
    let mk = |wave: u16, aux: u32| LfoSlot { wave, aux, ..LfoSlot::default() };
    let tri: Vec<i32> = (0..2048).map(|i| triangle(i)).collect();
    let saw_v: Vec<i32> = (0..2048).map(|i| saw(i)).collect();
    assert_eq!(fnv16(tri.iter().copied()), golden::TRI_FNV);
    assert_eq!(tri.iter().map(|&v| v as i64).sum::<i64>(), golden::TRI_SUM);
    assert_eq!(fnv16(saw_v.iter().copied()), golden::SAW_FNV);
    assert_eq!(saw_v.iter().map(|&v| v as i64).sum::<i64>(), golden::SAW_SUM);
    let sq: Vec<i32> = (0..2048).map(|i| wave_value(&mut mk(2, 700), i, &sine, &mut rng)).collect();
    assert_eq!(sq.iter().map(|&v| v as i64).sum::<i64>(), golden::SQUARE_700_SUM);
    for (n, &p) in golden::PHASES.iter().enumerate() {
        let p = p as i32;
        assert_eq!(wave_value(&mut mk(3, 0), p, &sine, &mut rng), golden::TRI_AT[n], "triangle {p}");
        assert_eq!(wave_value(&mut mk(4, 0), p, &sine, &mut rng), golden::SAW_AT[n], "saw {p}");
        assert_eq!(wave_value(&mut mk(2, 700), p, &sine, &mut rng), golden::SQUARE_700_AT[n], "square {p}");
        assert_eq!(wave_value(&mut mk(1, 0), p, &sine, &mut rng), golden::SINE_AT[n], "sine {p}");
        assert_eq!(wave_value(&mut mk(0, 0), p, &sine, &mut rng), 0);
    }
    // shapes the note describes: saw jumps from +0x7fff (0x3ff) to -0x7fff (0x400); triangle peaks at 0x200 and 0x600
    assert_eq!((saw(0x3ff), saw(0x400)), (0x7fff, -0x7fff));
    assert_eq!((triangle(0x200), triangle(0x600)), (0x7fff, -0x7fff));
    // the saw's magic multiply is a truncating division by 1023 for every input
    for i in 0..0x400 {
        assert_eq!(saw(i), ((i << 15) - i) / 1023);
    }
    assert_eq!(rng.state().1, 0, "deterministic waves draw nothing");
}

#[test]
fn random_wave_replays_the_draws() {
    let Some(mut rng) = disc_rng() else { return eprintln!("skip: extracted files absent") };
    let sine = sine_formula();
    // 0x20bc: two draws at start
    let (r1, r2) = (rng.next() as i32, rng.next() as i32);
    let mut sl = LfoSlot { wave: 5, state_1c: 1, aux: (-(r1 & 0x7fff)).wrapping_mul(r2 & 1) as u32, ..LfoSlot::default() };
    for &(a1, v, f1c, d) in golden::RANDOM_SEQ {
        let w = wave_value(&mut sl, a1 as i32, &sine, &mut rng);
        assert_eq!((w, sl.state_1c), (v, f1c), "phase {a1:#x}");
        assert_eq!(rng.state().1 as u32, d % 250, "draw count after phase {a1:#x}");
    }
}

#[test]
fn amplitude_table() {
    for &(t, d, sv, a) in golden::AMPS {
        let mut sl = LfoSlot { target: t, depth: d, state_24: 7, ..LfoSlot::default() };
        amplitude(&mut sl, sv);
        assert_eq!((sl.amp, sl.state_24), (a, 0), "target {t} depth {d} sound volume {sv}");
    }
    // targets 0, 5, 6: amplitude unchanged, +0x24 cleared
    for t in [0u8, 5, 6, 9] {
        let mut sl = LfoSlot { target: t, depth: 500, amp: 1234, state_24: 7, ..LfoSlot::default() };
        amplitude(&mut sl, 100);
        assert_eq!((sl.amp, sl.state_24), (1234, 0));
    }
}

/// Plays the scenario's grain 4 list through the real interpreter, then steps the list by hand (to read the return codes).
fn run_scenario(sc: &Scenario, rng: Rng250, by_update: bool) -> (L, usize, Vec<[i32; 5]>) {
    let mut grains: Vec<Grain> = sc.grains.iter().map(lfo_grain).collect();
    grains.push(plain(35, 30000));
    let bank = bank_with(sc.sound_vol, grains);
    let mut d: L = Driver::new(rng, RecVoices::new(), Lfos::new());
    let h = d.play(1, &bank, 0, &args());
    assert_ne!(h, 0, "{}", sc.name);
    let s = d.core.find(h).unwrap();
    let o = offsets(d.core.slot(s));
    let mut trace = vec![[o[0], o[1], o[2], o[3], 0]];
    for _ in 0..sc.steps {
        let mut code = 0i32;
        if by_update {
            d.lfo.update(&mut d.core, &mut d.voices);
        } else {
            for (i, (hh, k)) in d.lfo.list.clone().into_iter().enumerate() {
                code |= (d.lfo.step(&mut d.core, hh, k) as i32) << (2 * i);
            }
        }
        let o = offsets(d.core.slot(s));
        trace.push([o[0], o[1], o[2], o[3], code]);
    }
    (d, s, trace)
}

#[test]
fn scenarios_match_the_replay() {
    if disc_rng().is_none() {
        return eprintln!("skip: extracted files absent");
    }
    for sc in golden::SCENARIOS {
        let (d, s, trace) = run_scenario(sc, disc_rng().unwrap(), false);
        assert_eq!(trace.len(), sc.trace.len(), "{}", sc.name);
        for (i, (got, want)) in trace.iter().zip(sc.trace.iter()).enumerate() {
            assert_eq!(got, want, "{} step {}", sc.name, i);
        }
        for k in 0..4 {
            let l = &d.core.slot(s).lfo[k];
            let got = [l.wave as i64, l.target as i64, l.in_list as i64, l.depth as i64, l.phase as i64, l.rate as i64, l.aux as i32 as i64, l.state_1c as i64, l.amp as i64];
            assert_eq!(got, sc.slots[k], "{} slot {}", sc.name, k);
        }
        assert_eq!(d.core.rng.state().1 as u32, sc.draws % 250, "{} R250 draws", sc.name);
        assert_eq!(d.lfo.running() as u32, sc.list_len, "{} list", sc.name);
        // the Lfo::update entry (offsets only) walks the same path
        let (_, _, t2) = run_scenario(sc, disc_rng().unwrap(), true);
        for (i, (got, want)) in t2.iter().zip(sc.trace.iter()).enumerate() {
            assert_eq!(got[..4], want[..4], "{} update() step {}", sc.name, i);
        }
    }
}

#[test]
fn list_holds_ten_keeps_start_order_and_releases_on_stop() {
    let four: Vec<Grain> = (0..4u8)
        .map(|k| lfo_grain(&Cfg { slot: k, target: 2, wave: 1, depth: 100, rate: 0x1000, flags: 0, phase: 0, width: 0, link: 0 }))
        .chain([plain(35, 30000)])
        .collect();
    let bank = bank_with(100, four);
    let mut d: L = Driver::new(test_rng(), RecVoices::new(), Lfos::new());
    let hs: Vec<u32> = (0..3).map(|_| d.play(1, &bank, 0, &args())).collect();
    assert!(hs.iter().all(|&h| h != 0));
    assert_eq!(d.lfo.running(), MAX_LFOS, "12 starts, 10 run");
    let s2 = d.core.find(hs[2]).unwrap();
    let flags: Vec<bool> = (0..4).map(|k| d.core.slot(s2).lfo[k].in_list).collect();
    assert_eq!(flags, vec![true, true, false, false], "the 11th and 12th slot were refused");
    assert_eq!(d.core.slot(s2).lfo[2].amp, (100 * 45) >> 8, "the amplitude is computed before the list test");
    let order: Vec<(usize, usize)> = d.lfo.list.clone();
    let s0 = d.core.find(hs[0]).unwrap();
    assert_eq!(order[0], (s0, 0));
    assert_eq!(order[3], (s0, 3));
    // stopping the first handler removes its 4 slots; the others keep their order
    assert!(d.stop(hs[0]));
    d.ticks(2);
    assert_eq!(d.lfo.running(), 6);
    assert_eq!(d.lfo.list[..], order[4..]);
    // a new handler takes the freed capacity (all 4 slots fit again)
    let h4 = d.play(1, &bank, 0, &args());
    assert_ne!(h4, 0);
    assert_eq!(d.lfo.running(), 10);
    // grain 4 with target 0 removes a single slot
    let off = bank_with(100, vec![lfo_grain(&Cfg { slot: 1, target: 2, wave: 1, depth: 100, rate: 0x1000, flags: 0, phase: 0, width: 0, link: 0 }), {
        let mut g = plain(4, 0);
        g.p[0] = 1;
        g
    }, plain(35, 30000)]);
    let mut d2: L = Driver::new(test_rng(), RecVoices::new(), Lfos::new());
    let h = d2.play(1, &off, 0, &args());
    let s = d2.core.find(h).unwrap();
    assert_eq!(d2.lfo.running(), 0);
    assert_eq!((d2.core.slot(s).lfo[1].wave, d2.core.slot(s).lfo[1].in_list), (0, false));
}

#[test]
fn updates_run_on_even_ticks_only_and_reapply_volume_and_pitch() {
    let g = vec![
        lfo_grain(&Cfg { slot: 0, target: 1, wave: 1, depth: 900, rate: 0x40000, flags: 0, phase: 0, width: 0, link: 0 }),
        lfo_grain(&Cfg { slot: 1, target: 4, wave: 3, depth: 600, rate: 0x30000, flags: 0, phase: 0, width: 0, link: 0 }),
        lfo_grain(&Cfg { slot: 2, target: 2, wave: 1, depth: 700, rate: 0x28000, flags: 0, phase: 0, width: 0, link: 0 }),
        tone_grain(0),
        plain(35, 30000),
    ];
    let bank = bank_with(100, g);
    let mut d: L = Driver::new(test_rng(), RecVoices::new(), Lfos::new());
    let h = d.play(1, &bank, 0, &args());
    let s = d.core.find(h).unwrap();
    let mut changes = 0;
    let mut prev = offsets(d.core.slot(s));
    for t in 1..=40u32 {
        d.tick();
        let now = offsets(d.core.slot(s));
        if now != prev {
            assert_eq!(t % 2, 0, "offsets change on even ticks only (tick {t})");
            changes += 1;
        }
        prev = now;
    }
    assert!(changes >= 15, "{changes} changes in 20 updates");
    let revol: Vec<u32> = d.voices.log.iter().filter(|e| matches!(e.1, VoiceEvent::Volume { .. })).map(|e| e.0).collect();
    let repit: Vec<u32> = d.voices.log.iter().filter(|e| matches!(e.1, VoiceEvent::Pitch { .. })).map(|e| e.0).collect();
    assert!(!revol.is_empty() && !repit.is_empty());
    assert!(revol.iter().chain(repit.iter()).all(|t| t % 2 == 0), "re-applies come from the even-tick update only");
    // target 1 only lowers the volume (offset <= 0); volume stays within 0..=0x7f
    assert!(d.core.slot(s).vol_off <= 0);
    assert!((0..=0x7f).contains(&d.core.slot(s).volume));
    assert!(d.core.errors.is_empty());
}

#[test]
fn start_runs_one_step_but_does_not_reapply_the_voices() {
    // a pan LFO that starts on the sine peak: pan offset non-zero at once, no Volume event until the next update changes it
    let g = vec![
        tone_grain(0),
        lfo_grain(&Cfg { slot: 0, target: 2, wave: 1, depth: 700, rate: 0x10, flags: 0, phase: 0, width: 0, link: 0 }),
        plain(35, 30000),
    ];
    let bank = bank_with(100, g);
    let mut d: L = Driver::new(test_rng(), RecVoices::new(), Lfos::new());
    let h = d.play(1, &bank, 0, &args());
    let s = d.core.find(h).unwrap();
    assert_ne!(d.core.slot(s).pan_off, 0, "the first step ran inside the grain");
    assert!(!d.voices.log.iter().any(|e| matches!(e.1, VoiceEvent::Volume { .. })), "start does not re-apply");
    d.ticks(2);
    // the rate is tiny, so the offset is unchanged at tick 2: the step returns 0 and still nothing is re-applied
    assert!(!d.voices.log.iter().any(|e| matches!(e.1, VoiceEvent::Volume { .. })));
}

#[test]
fn rate_and_depth_modulation_targets_5_and_6() {
    let sc = golden::SCENARIOS.iter().find(|s| s.name == "rate_depth_modulation").unwrap();
    let Some(r) = disc_rng() else { return eprintln!("skip: extracted files absent") };
    let (d, s, _) = run_scenario(sc, r, false);
    let l = &d.core.slot(s).lfo;
    assert_ne!(l[0].rate, l[0].rate_base, "slot 1 (target 5) modulates the rate of slot 0");
    assert_ne!(l[0].depth, l[0].depth_wide as u16, "slot 2 (target 6) modulates the depth of slot 0");
    assert_eq!(l[0].depth_wide, 1024, "+8 keeps the base depth");
}

#[test]
fn census_lfo_grains_in_all_banks_run_clean() {
    let banks = testdata::all_banks();
    if banks.is_empty() {
        return eprintln!("skip: extracted files absent");
    }
    let (mut n, mut sounds_with, mut max_rate, mut max_depth) = (0, 0, 0u32, 0u16);
    let mut targets = [0u32; 8];
    let mut waves = [0u32; 8];
    let mut ticks_run = 0u64;
    for (_, raw) in &banks {
        let bank = Arc::new(Bank::parse(raw).unwrap());
        for (si, snd) in bank.sounds.iter().enumerate() {
            let lfos: Vec<&Grain> = snd.grains.iter().filter(|g| g.kind == 4).collect();
            if lfos.is_empty() {
                continue;
            }
            sounds_with += 1;
            for g in &lfos {
                n += 1;
                let (slot, target, wave) = (g.u8_at(0), g.u8_at(1), g.u8_at(3));
                assert!(slot < 4, "slot {slot}");
                assert!(target <= 6, "target {target}");
                targets[target as usize] += 1;
                if target != 0 {
                    assert!(wave <= 5, "wave {wave}");
                    waves[wave as usize] += 1;
                    max_rate = max_rate.max(g.u32_at(0xc));
                    max_depth = max_depth.max(g.u16_at(6));
                    assert!((g.u32_at(0xc) as u64) * 2 < 0x800_0000, "phase step stays inside the table");
                    if target >= 5 {
                        assert!(g.u8_at(2) < 4, "link");
                    }
                }
            }
            let mut d: L = Driver::new(test_rng(), RecVoices::new(), Lfos::new());
            let h = d.play(1, &bank, si as i32, &PlayArgs::cid17(0x7fff_ffff, -1, 0, 0));
            for _ in 0..600 {
                d.tick();
                assert!(d.lfo.running() <= MAX_LFOS);
            }
            ticks_run += 600;
            assert!(d.core.errors.is_empty(), "bank sound {si}: {:?}", d.core.errors);
            d.stop_all();
            d.ticks(300);
            assert_eq!(d.lfo.running(), 0, "stop releases the LFOs (handle {h:#x})");
            assert_eq!(d.core.active_count(), 0);
        }
    }
    eprintln!("lfo census: {n} grains in {sounds_with} sounds, targets {targets:?}, waves {waves:?}, max rate {max_rate:#x}, max depth {max_depth}, {ticks_run} ticks");
    assert!(n > 0);
}

#[test]
fn title_sound_3_through_spu_voices_with_the_real_lfo() {
    let (Some(raw), Some(snd)) = (testdata::read("globals/sound_bank.bin"), disc_irx()) else { return eprintln!("skip: extracted files absent") };
    let irx = Irx::parse(&snd).unwrap();
    let bank = Arc::new(Bank::parse(&raw).unwrap());
    assert_eq!(bank.sounds[3].volume, 95);
    let mut v = SpuVoices::new(PanTable::from_irx(&irx).unwrap(), Note2PitchTables::from_formula());
    v.apply_boot_config();
    let mut d = Driver::new(Rng250::from_irx(&irx).unwrap(), v, Lfos::from_irx(&irx).unwrap());
    let h = d.play(1, &bank, 3, &args());
    assert_ne!(h, 0);
    let s = d.core.find(h).unwrap();
    assert_eq!(d.lfo.running(), 3, "volume, pan and pitch LFOs");
    // the replay: sound 3 of the python scenario `title_sound_3` (same three random start phases, same R250 start)
    let sc = golden::SCENARIOS.iter().find(|s| s.name == "title_sound_3").unwrap();
    let o = offsets(d.core.slot(s));
    assert_eq!(o[..], sc.trace[0][..4]);
    for i in 1..=sc.steps as usize {
        d.ticks(2);
        let o = offsets(d.core.slot(s));
        assert_eq!(o[..], sc.trace[i][..4], "after update {i} (tick {})", 2 * i);
    }
    // 3 cycles: KON/KOFF stay a function of the grains (sine waves draw nothing), the LFO restarts every cycle
    d.ticks(3 * 6000);
    let ev = d.voices.take_events();
    let kons: Vec<u32> = ev.iter().filter(|e| e.reg == Reg::Kon).map(|e| e.tick).collect();
    assert!(kons.len() >= 2, "{kons:?}");
    // volume / pitch rewrites caused by the LFO happen on even ticks; at odd ticks only a key-on writes them
    for e in ev.iter().filter(|e| matches!(e.reg, Reg::VolL | Reg::VolR | Reg::Pitch)) {
        if e.tick % 2 == 1 {
            assert!(kons.contains(&e.tick), "odd-tick register write {:?} without a key-on", e);
        }
    }
    let pitch_writes: Vec<u32> = ev.iter().filter(|e| e.reg == Reg::Pitch).map(|e| e.value).collect();
    let distinct = {
        let mut p = pitch_writes.clone();
        p.sort();
        p.dedup();
        p.len()
    };
    assert!(distinct > 20, "the bend LFO moves the pitch ({distinct} distinct values)");
    assert!(d.core.errors.is_empty() && d.voices.errors.is_empty());
    eprintln!("title+lfo: {} kon, {} pitch writes ({} distinct), {} events", kons.len(), pitch_writes.len(), distinct, ev.len());
}
