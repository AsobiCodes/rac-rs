use super::*;
use crate::snd989::bank::{Signature, Sound};
use crate::snd989::irx::{Irx, MODULE_989SND};
use crate::snd989::testdata;

type D = Driver<RecVoices, NullLfo>;

fn gr(kind: u32, delay: u16, f: impl FnOnce(&mut [u8; 32])) -> Grain {
    let mut p = [0u8; 32];
    f(&mut p);
    Grain { kind, delay, p }
}
fn w16(p: &mut [u8; 32], o: usize, v: i16) {
    p[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
fn tone(delay: u16, vol: i8) -> Grain {
    gr(1, delay, |p| {
        p[1] = vol as u8;
        w16(p, 4, 180);
    })
}
fn reg(kind: u32, r: i16, a: i16, b: i16) -> Grain {
    gr(kind, 0, |p| {
        w16(p, 0, r);
        w16(p, 2, a);
        w16(p, 4, b);
    })
}
fn p0(kind: u32, v: i16) -> Grain {
    gr(kind, 0, move |p| w16(p, 0, v))
}
fn mk(grains: Vec<Grain>) -> Arc<Bank> {
    let s = Sound { volume: 100, group: 0, pan: 90, pad5: 0, flags: 0, grain_offset: 0, grains };
    Arc::new(Bank { signature: Signature { id: 0, sounds: 1, grains: 0, vags: 0 }, sounds: vec![s], data: vec![] })
}
fn rng() -> Rng250 {
    let mut t = [0u16; 250];
    for (i, x) in t.iter_mut().enumerate() {
        *x = (i as u16).wrapping_mul(40503).wrapping_add(7);
    }
    Rng250::from_state(t, 0).unwrap()
}
fn drv() -> D {
    Driver::new(rng(), RecVoices::new(), NullLfo::default())
}
fn keyons(d: &D) -> Vec<(u32, i8)> {
    d.voices.log.iter().filter_map(|(t, e)| if let VoiceEvent::KeyOn { tone_volume, .. } = e { Some((*t, *tone_volume)) } else { None }).collect()
}
fn offs(d: &D) -> Vec<u32> {
    d.voices.log.iter().filter_map(|(t, e)| if let VoiceEvent::KeyOff { .. } = e { Some(*t) } else { None }).collect()
}
fn nop(kind: u32, delay: u16) -> Grain {
    gr(kind, delay, |_| {})
}
const A: PlayArgs = PlayArgs { scale: 0x400, pan: -1, pitch_mod: 0, bend: 0, regs: None };

#[test]
fn handle_rule_lowest_slot_serial_plus_one() {
    let b = mk(vec![tone(0, 100), nop(21, 1000), nop(22, 0)]);
    let mut d = drv();
    let (h0, h1) = (d.play(0, &b, 0, &A), d.play(0, &b, 0, &A));
    assert_eq!((h0, h1), (0x8500_0001, 0x8501_0001));
    assert!(d.stop(h0));
    assert_eq!(d.playing(h0), 0);
    assert_eq!(d.playing(h1), h1);
    assert_eq!((d.playing(0), d.playing(0xffff_ffff)), (0, 0xffff_ffff));
    assert_eq!(d.play(0, &b, 0, &A), 0x8500_0002, "lowest free slot, serial +1");
    assert!(!d.stop(h0), "old handle no longer matches the slot word");
    let dead = mk(vec![nop(24, 0)]);
    assert_eq!(d.play(0, &dead, 0, &A), 0);
    assert_eq!(d.core.slot(2).word & 0xffff, 1, "a failed play still consumed a serial");
    assert_eq!(d.play(0, &b, 5, &A), 0, "no such sound");
    let mut n = 2;
    while d.play(0, &b, 0, &A) != 0 {
        n += 1;
    }
    assert_eq!(n, 64, "64 slots");
}

#[test]
fn tone_wait_keyoff_stop_and_free_timing() {
    let b = mk(vec![tone(0, 100), nop(41, 50), nop(24, 160)]);
    let mut d = drv();
    let h = d.play(0, &b, 0, &A);
    assert_eq!(keyons(&d), vec![(0, 100)], "zero-delay tone keys on inside the play call");
    d.ticks(209);
    assert_eq!(d.playing(h), h);
    assert_eq!(offs(&d), vec![50]);
    d.tick();
    assert_eq!(d.playing(h), 0, "grain 24 at tick 210 frees the handler");
    assert_eq!(d.core.active_count(), 0);
    let e = d.voices.log.iter().find_map(|(_, e)| if let VoiceEvent::KeyOn { volume, pan, .. } = e { Some((*volume, *pan)) } else { None });
    assert_eq!(e, Some((100, 90)), "volume = 0x400 * 100 >> 10, pan = the sound default");
}

#[test]
fn counted_loop_registers_and_conditions() {
    let b = mk(vec![reg(30, 0, 3, 0), nop(21, 0), tone(10, 7), reg(33, 0, 0, 0), reg(34, 0, 2, 0), nop(22, 0), nop(24, 0)]);
    let mut d = drv();
    let h = d.play(0, &b, 0, &A);
    d.ticks(100);
    assert_eq!(keyons(&d).iter().map(|k| k.0).collect::<Vec<_>>(), vec![10, 20, 30]);
    assert_eq!(d.playing(h), 0);
    let mut c = Core::new(rng());
    let (mut v, mut l) = (RecVoices::new(), NullLfo::default());
    let br = mk(vec![reg(30, 1, 127, 0), reg(32, 1, 0, 0), reg(40, 3, -100, 3), reg(40, 3, -100, 3), tone(5, -2)]);
    let _ = br.sounds[0].grains[2].s16_at(0);
    // grain 40 operands: p0 = amount, p2 = register
    let br = mk(vec![reg(30, 1, 127, 0), reg(32, 1, 0, 0), gr(40, 0, |p| { w16(p, 0, -100); w16(p, 2, 3); }), gr(40, 0, |p| { w16(p, 0, -100); w16(p, 2, 3); }), tone(5, -2)]);
    assert_ne!(c.play(&mut v, &mut l, 0, &br, 0, &A), 0);
    assert_eq!(c.slot(0).regs, [0, 127, 0, -128], "inc caps at 127, add clamps at -128");
}

#[test]
fn block_select_never_repeats_and_runs_one_block() {
    let mut g = vec![gr(25, 0, |p| {
        w16(p, 0, 3);
        w16(p, 2, 2);
    })];
    for k in 1..=3i8 {
        g.push(tone(0, k));
        g.push(nop(20, 0));
    }
    g.push(nop(24, 0));
    let b = mk(g);
    let mut d = drv();
    let mut shadow = d.core.rng.clone();
    let (mut last, mut want) = (0i32, Vec::new());
    for _ in 0..40 {
        let mut a = shadow.next() as i32 % 3;
        if a == last {
            a = (a + 1) % 3;
        }
        last = a;
        want.push(a as i8 + 1);
        d.play(0, &b, 0, &A);
        d.tick();
    }
    let got: Vec<i8> = keyons(&d).iter().map(|k| k.1).collect();
    assert_eq!(got, want, "exact R250 replay, one block per play, last pick persists across handlers");
    assert!(got.windows(2).all(|w| w[0] != w[1]));
    assert!(d.core.errors.is_empty());
}

#[test]
fn stop_section_runs_at_once_and_ignores_delays() {
    let b = mk(vec![tone(0, 1), nop(21, 5), nop(22, 5), nop(43, 0), reg(30, 1, 9, 0), tone(1000, 2), nop(41, 2000)]);
    let mut d = drv();
    let h = d.play(0, &b, 0, &A);
    d.ticks(100);
    assert_eq!(d.playing(h), h, "21/22 loop runs forever");
    let t = d.core.tick;
    assert!(d.stop(h));
    assert_eq!(d.playing(h), 0);
    assert_eq!(keyons(&d), vec![(0, 1), (t, 2)], "the section's tone keys on at the stop tick, not 1000 ticks later");
    assert_eq!(d.voices.log.iter().filter(|e| matches!(e.1, VoiceEvent::KeyOff { mask, .. } if mask != [0, 0])).count(), 2, "section start keys off tone 1, its own grain 41 keys off tone 2");
}

#[test]
fn program_end_with_voices_waits_then_frees() {
    let b = mk(vec![tone(0, 3)]);
    let mut d = drv();
    d.voices.natural_end = Some(30);
    let h = d.play(0, &b, 0, &A);
    assert_eq!(d.core.slot(0).idx, -1);
    d.ticks(30);
    assert_eq!(d.playing(h), h);
    d.ticks(2);
    assert_eq!(d.playing(h), 0, "last voice ended: freed, cid 25 answers 0");
    assert_eq!(d.core.active_count(), 0);
}

#[test]
fn grain_38_waits_for_voices() {
    let b = mk(vec![tone(0, 1), nop(38, 0), tone(0, 2), nop(24, 0)]);
    let mut d = drv();
    d.voices.natural_end = Some(20);
    let h = d.play(0, &b, 0, &A);
    d.ticks(40);
    let k = keyons(&d);
    assert_eq!(k.len(), 2);
    assert!(k[1].0 >= 20, "second tone only after the first voice ended: {k:?}");
    assert_eq!(d.playing(h), 0);
}

#[test]
fn bend_grains_exact() {
    let mut c = Core::new(rng());
    let (mut v, mut l) = (RecVoices::new(), NullLfo::default());
    c.play(&mut v, &mut l, 0, &mk(vec![p0(28, 127), p0(29, -127), p0(28, -127), p0(29, 1000), nop(24, 0)]), 0, &A);
    assert_eq!(c.slot(0).bend_base, 0x7fff, "28(127)=0x7fff, 29(-127)=0, 28(-127)=-0x8000, 29(+1000) clamps at 0x7fff");
}

#[test]
fn pause_continue_and_running_update() {
    let b = mk(vec![tone(0, 50), nop(21, 1), nop(22, 0)]);
    let mut d = drv();
    let h = d.play(0, &b, 0, &A);
    d.pause_groups(1);
    assert!(d.core.slot(0).paused());
    let w = d.core.slot(0).wait;
    d.ticks(5);
    assert_eq!(d.core.slot(0).wait, w, "a paused handler is skipped by the tick");
    d.resume_groups(1);
    assert!(!d.core.slot(0).paused());
    assert_eq!(d.voices.log.iter().filter(|e| matches!(e.1, VoiceEvent::Pause { .. } | VoiceEvent::Resume { .. })).count(), 2);
    let n = d.voices.log.len();
    assert_eq!(d.update(h, 1, 0x400, 0, 0, 0), h);
    assert_eq!(d.voices.log.len(), n, "unchanged volume and pan touch no voice");
    d.update(h, 1, 0x200, 0, 0, 0);
    assert!(matches!(d.voices.log.last(), Some((_, VoiceEvent::Volume { volume: 50, paused: false, .. }))));
    d.update(h, 8 | 0x10, 0, 0, 77, 1234);
    assert!(matches!(d.voices.log.last(), Some((_, VoiceEvent::Pitch { bend: 1234, pitch_mod: 77, .. }))));
    assert_eq!(d.update(0x8500_0063, 1, 0, 0, 0, 0), 0);
    d.stop_all();
    assert_eq!(d.core.active_count(), 0);
}

#[test]
fn tick_order_and_lfo_on_even_ticks() {
    let b = mk(vec![tone(0, 1), nop(41, 2), tone(0, 2), nop(21, 1000), nop(22, 0)]);
    let mut d = drv();
    d.play(0, &b, 0, &A);
    d.ticks(4);
    assert_eq!(d.lfo.updates, 2, "LFO update on tick 2 and 4");
    let i_off = d.voices.log.iter().position(|e| matches!(e.1, VoiceEvent::KeyOff { .. })).unwrap();
    let i_app = d.voices.log.iter().position(|e| matches!(e.1, VoiceEvent::KeyOffsApplied)).unwrap();
    assert!(i_off < i_app);
    assert_eq!((d.voices.log[i_off].0, d.voices.log[i_app].0), (2, 2), "pending key-offs applied at the end of the same tick");
}

fn disc_rng() -> Option<Rng250> {
    let blob = testdata::read("globals/irx.bin")?;
    let irx = Irx::find_in_blob(&blob, MODULE_989SND)?;
    Rng250::from_irx(&Irx::parse(irx).ok()?).ok()
}

#[test]
fn title_sound_3_global_bank_replays_the_rng() {
    let (Some(raw), Some(r)) = (testdata::read("globals/sound_bank.bin"), disc_rng()) else {
        eprintln!("skip: extracted files absent");
        return;
    };
    let bank = Arc::new(Bank::parse(&raw).unwrap());
    let g = &bank.sounds[3].grains;
    assert_eq!(g.iter().map(|x| x.kind).collect::<Vec<_>>(), vec![35, 4, 4, 4, 1, 26, 41, 26, 36]);
    let mut shadow = r.clone();
    let mut d = Driver::new(r, RecVoices::new(), NullLfo::default());
    let h = d.play(1, &bank, 3, &PlayArgs::cid17(0x7fff_ffff, -1, 0, 0));
    assert_ne!(h, 0);
    let (mut t, mut want_on, mut want_off) = (0i64, vec![], vec![]);
    for _ in 0..3 {
        let mut ret = 0i64;
        for x in g.iter() {
            t += x.delay as i64 + ret;
            ret = 0;
            match x.kind {
                4 if x.p[1] != 0 && x.p[8] & 2 != 0 => {
                    shadow.next();
                }
                1 => want_on.push(t),
                41 => want_off.push(t),
                26 => ret = (shadow.next() as i64) % (x.s16_at(0) as i64 + 1),
                _ => {}
            }
        }
    }
    d.ticks(want_off.last().copied().unwrap() as u32 + 1);
    let on: Vec<i64> = keyons(&d).iter().map(|k| k.0 as i64).collect();
    let off: Vec<i64> = offs(&d).iter().map(|&x| x as i64).collect();
    assert_eq!(on[..], want_on[..on.len()]);
    assert_eq!(off, want_off);
    assert_eq!(d.playing(h), h, "endless program");
    assert!(d.core.errors.is_empty());
}

#[test]
fn census_all_banks_runs_without_panic_and_obeys_loop_structure() {
    let all = testdata::all_banks();
    let Some(r) = disc_rng() else {
        eprintln!("skip: extracted files absent");
        return;
    };
    let (mut rng, mut ended, mut alive, mut errors) = (r, 0usize, 0usize, 0usize);
    for (name, raw) in &all {
        let bank = Arc::new(Bank::parse(raw).unwrap());
        for (si, snd) in bank.sounds.iter().enumerate() {
            let kinds: Vec<u32> = snd.grains.iter().map(|g| g.kind).collect();
            let has = |k: u32| kinds.contains(&k);
            let mut voices = RecVoices::new();
            voices.natural_end = Some(150);
            let mut d = Driver::new(rng.clone(), voices, NullLfo::default());
            let h = d.play(1, &bank, si as i32, &A);
            let mut ticks = 0u32;
            while h != 0 && d.playing(h) != 0 && ticks < 12_000 {
                d.tick();
                ticks += 1;
            }
            let is_alive = h != 0 && d.playing(h) != 0;
            if d.core.errors.is_empty() {
                let forward_only = !has(22) && !has(36) && !has(37) && !has(23);
                let eternal = (has(22) || has(36)) && !has(24) && !has(34) && !has(43) && !has(23) && !has(37) && !has(25);
                assert!(!(forward_only && is_alive), "{name} sound {si}: forward-only program must end ({kinds:?})");
                assert!(!(eternal && !is_alive), "{name} sound {si}: unconditional loop must run on ({kinds:?})");
            } else {
                errors += 1;
                eprintln!("{name} sound {si}: errors {:?}", d.core.errors);
            }
            if is_alive {
                alive += 1;
            } else {
                ended += 1;
            }
            rng = d.core.rng.clone();
        }
    }
    eprintln!("census: {ended} ended, {alive} still running at 12000 ticks, {errors} with errors");
    assert_eq!(errors, 0);
}
