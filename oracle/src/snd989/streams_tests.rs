use super::*;
use crate::snd989::handler::Voices;
use crate::snd989::voices::{SpuWrite, VState};

const INTRO: u32 = 100;
const LOOP: u32 = 200;
const READY: u32 = 0;
const BLOCKS: u32 = 441; // 441 * 28 = 12348 samples = 0.28 s at 44.1 kHz

fn vags() -> HashMap<u32, VagInfo> {
    let mut m = HashMap::new();
    m.insert(INTRO, VagInfo { rate: 44100, data_bytes: BLOCKS * 16 });
    m.insert(LOOP, VagInfo { rate: 44100, data_bytes: BLOCKS * 16 });
    m.insert(300, VagInfo { rate: 22050, data_bytes: 16 * 100 });
    m.insert(400, VagInfo { rate: 44100, data_bytes: 40000 });
    m.insert(500, VagInfo { rate: 44100, data_bytes: 100000 });
    m
}

fn setup() -> (SpuVoices, Streams) {
    let mut v = SpuVoices::with_formula();
    v.apply_boot_config();
    let mut s = Streams::new();
    s.init(4);
    (v, s)
}

fn args(lba: u32, lba_r: u32, group: i8, after: u32, flags: u32) -> PlayVag {
    PlayVag { lba, lba_r, ring: [0x100, 0x200], volume: 1024, pan: 0, group, after, flags }
}

/// One driver iteration: stream tick, then the key-on application of the voice layer.
fn step(v: &mut SpuVoices, s: &mut Streams, now: u32) {
    v.tick_begin(now);
    s.tick(now, v, &vags());
    v.apply_key_offs(now);
    v.apply_key_ons(now);
}

fn writes(ev: &[SpuWrite], reg: Reg, voice: usize) -> Vec<u32> {
    ev.iter().filter(|e| e.reg == reg && e.index as usize == voice).map(|e| e.value).collect()
}

#[test]
fn helpers_pitch_time_and_bytes() {
    assert_eq!(pitch_of_rate(44100), 3763);
    assert_eq!(pitch_of_rate(48000), 4096);
    assert_eq!(pitch_of_rate(22050), 1881);
    assert_eq!(fixed_time(44100, 44100), 1024);
    assert_eq!(fixed_time(22050, 44100), 512);
    assert_eq!(fixed_time(1, 44100), 0);
    assert_eq!(bytes_to_samples(16), 28);
    assert_eq!(bytes_to_samples(16 * 3 + 4), 84);
    assert_eq!(bytes_to_samples(16 * 3 + 5), 86);
    assert_eq!(bytes_to_samples(16 * 3 + 15), 84 + 22);
}

#[test]
fn handle_words_and_replies() {
    let (mut v, mut s) = setup();
    let mut cold = Streams::new();
    assert_eq!(cold.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20)), 0, "before cid 42: error 0x2f, reply 0");
    assert_eq!(cold.errors, vec![0x2f]);
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20));
    let b = s.play(0, &mut v, &args(LOOP, 0, 2, 0, 0));
    assert_eq!(a, 0x8400_0002, "slot 0, serial 1 + 1");
    assert_eq!(b, 0x8401_0002, "slot 1");
    assert_eq!(s.playing(a), a);
    assert_eq!(s.playing(0), 0);
    assert_eq!(s.playing(u32::MAX), 0);
    assert_eq!(s.playing(0x8500_0001), 0, "a sound handle is not a stream");
    assert_eq!(s.buffered(0x8402_0002), -1, "unknown handle: -1");
    assert_eq!(s.buffered(a), 0, "not buffered before the first buffers are in the ring");
    assert_eq!(s.remaining(a), 0, "no header read yet");
    step(&mut v, &mut s, READY);
    assert_eq!((s.buffered(a), s.buffered(b)), (1, 1));
    // free + reuse: the handle dies at the next stream pass; the slot's serial counts on, the old handle is rejected
    assert!(s.stop(&mut v, a));
    assert_eq!(s.buffered(a), 1, "alive until the stream pass");
    step(&mut v, &mut s, 60);
    assert_eq!(s.buffered(a), -1);
    let c = s.play(60, &mut v, &args(INTRO, 0, 1, 0, 0));
    assert_eq!(c, 0x8400_0003);
    assert_eq!(s.playing(a), 0);
    // `after` unknown: with flag 0x10 the call fails, without it a new stream starts
    assert_eq!(s.play(60, &mut v, &args(INTRO, 0, 1, 0x8409_0002, 0x30)), 0);
    let d = s.play(60, &mut v, &args(INTRO, 0, 1, 0x8409_0002, 0x20));
    assert_eq!(d >> 24, 0x84);
    assert_ne!(d, 1);
}

#[test]
fn stream_voices_occupy_the_pool_and_are_never_stolen() {
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20)); // mono + 0x20 -> two voices
    let st = s.stream(a).unwrap();
    assert_eq!((st.voices, st.extra), ([Some(0x18), None], Some(0x19)), "first free voices from 0x18 up (boot cid 78)");
    for x in [0x18, 0x19] {
        assert_eq!(v.rec[x].state, VState::Stream);
        assert_eq!(v.rec[x].priority, 0x7f);
    }
    // 60 allocations of group 1 at the highest tone priority (100): the other 22 voices are taken, then stolen over and
    // over; the two stream voices never come back
    let mut seen = std::collections::HashSet::new();
    for t in 0..60u32 {
        let g = v.alloc(t, 1, 100).expect("SFX voice");
        seen.insert(g.voice);
        assert!(g.voice != 0x18 && g.voice != 0x19);
    }
    assert_eq!(seen.len(), 22);
    // a new stream (priority 0x7f) beats any tone: it steals an SFX voice (reported), never a stream voice
    let w = v.alloc_stream(1).expect("stream steals a tone voice");
    assert!(w != 0x18 && w != 0x19);
    assert!(w >= 0x1a, "an SFX voice: {w:#x}");
    // stereo: two voices, no handler voice, even with flags & 0x20
    let (mut v2, mut s2) = setup();
    let b = s2.play(0, &mut v2, &args(INTRO, LOOP, 2, 0, 0x20));
    let st = s2.stream(b).unwrap();
    assert_eq!((st.voices, st.extra), ([Some(0x18), Some(0x19)], None));
    // a mono stream without 0x20: one voice; group 0 scans from voice 0
    let c = s2.play(0, &mut v2, &args(INTRO, 0, 0, 0, 0));
    assert_eq!(s2.stream(c).unwrap().voices, [Some(0), None]);
    // a failed second alloc frees the first and fails the call
    let (mut v3, mut s3) = setup();
    for _ in 0..23 {
        v3.alloc_stream(1).unwrap();
    }
    assert_eq!(s3.play(0, &mut v3, &args(INTRO, LOOP, 1, 0, 0)), 0);
    assert_eq!(v3.rec.iter().filter(|r| r.state == VState::Stream).count(), 23, "the left voice was given back");
}

#[test]
fn key_on_register_set_mono_with_extra_voice() {
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20 | 2));
    for t in 0..=READY {
        step(&mut v, &mut s, t);
    }
    let ev = v.take_events();
    let regs: Vec<(Reg, u8)> = ev.iter().map(|e| (e.reg, e.index)).collect();
    // the extra voice (0x19) first, then the channel voice (0x18); both on core 1 (voice 0x18 = core 1 voice 0)
    let one = |x: u8| [(Reg::VolL, x), (Reg::VolR, x), (Reg::VmixL, 1), (Reg::VmixR, 1), (Reg::Pitch, x), (Reg::Ssa, x), (Reg::Adsr1, x), (Reg::Adsr2, x)];
    let mut want: Vec<(Reg, u8)> = one(0x19).to_vec();
    want.extend(one(0x18));
    assert_eq!(&regs[..16], &want[..]);
    // then the application of the pending key-on: reverb send (flag 2) and KON
    assert_eq!(regs[16..], [(Reg::VmixEL, 0), (Reg::VmixER, 0), (Reg::VmixEL, 1), (Reg::VmixER, 1), (Reg::Kon, 0), (Reg::Kon, 1)]);
    let amp = 127 * 0x102; // volume 1024 -> 127; centre
    let l = volume_reg(amp as i16, 1, &v.group_vol) as u32;
    // channel voice: pan rotated by 270 -> hard left; the extra voice is the same data swapped -> hard right
    assert_eq!(writes(&ev, Reg::VolL, 0x18), vec![l]);
    assert_eq!(writes(&ev, Reg::VolR, 0x18), vec![0]);
    assert_eq!(writes(&ev, Reg::VolL, 0x19), vec![0]);
    assert_eq!(writes(&ev, Reg::VolR, 0x19), vec![l]);
    for x in [0x18, 0x19] {
        assert_eq!(writes(&ev, Reg::Pitch, x), vec![3763]);
        assert_eq!(writes(&ev, Reg::Ssa, x), vec![0x5040 + 0x100 + 0x30]);
        assert_eq!(writes(&ev, Reg::Adsr1, x), vec![0x000f]);
        assert_eq!(writes(&ev, Reg::Adsr2, x), vec![0x1fc0]);
    }
    // VMIX: dry only, the voice bits accumulate; the wet switches follow the reverb flag
    assert_eq!(writes(&ev, Reg::VmixL, 1), vec![0x2, 0x3]);
    assert_eq!(writes(&ev, Reg::VmixR, 1), vec![0x2, 0x3]);
    assert_eq!(writes(&ev, Reg::Kon, 1), vec![0x3]);
    assert_eq!(writes(&ev, Reg::VmixEL, 1), vec![0x3]);
    assert!(s.stream(a).unwrap().keyed);
    assert_eq!(s.take_events(), vec![StreamEv::Start { tick: READY, voice: 0x18, extra: Some(0x19), lba: INTRO, held: false }]);
}

#[test]
fn key_on_stereo_uses_each_channels_ring_and_no_reverb_without_flag() {
    let (mut v, mut s) = setup();
    s.spu_base = 0x5000;
    let h = s.play(0, &mut v, &args(INTRO, LOOP, 2, 0, 0));
    for t in 0..=READY {
        step(&mut v, &mut s, t);
    }
    let ev = v.take_events();
    // ring_base = spu_base + chan * half (half = 0x7800 for cid 42 (4, 0xf000)); chans 0 and 1
    assert_eq!(writes(&ev, Reg::Ssa, 0x18), vec![0x5000 + 0x100 + 0x30]);
    assert_eq!(writes(&ev, Reg::Ssa, 0x19), vec![0x5000 + 0x7800 + 0x200 + 0x30]);
    // stereo pan offsets (header parse 0x11d24 / 0x11d44): left channel 0x10e = hard left, right channel 0x5a = hard right
    let amp = 127 * 0x102;
    let l = volume_reg(amp as i16, 2, &v.group_vol) as u32;
    assert_eq!((writes(&ev, Reg::VolL, 0x18), writes(&ev, Reg::VolR, 0x18)), (vec![l], vec![0]));
    assert_eq!((writes(&ev, Reg::VolL, 0x19), writes(&ev, Reg::VolR, 0x19)), (vec![0], vec![l]));
    assert_eq!(s.stream(h).unwrap().pan_off, [0x10e, 0x5a]);
    assert_eq!(writes(&ev, Reg::VmixEL, 1), vec![0], "flags & 2 clear: no reverb send");
    assert_eq!(writes(&ev, Reg::Kon, 1), vec![3]);
}

#[test]
fn queued_intro_then_loop_is_gapless_and_takes_no_voice() {
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20));
    assert_eq!(s.play(0, &mut v, &args(LOOP, 0, 1, a, 0x24)), 1, "queued: reply 1, not a handle");
    assert_eq!(v.rec.iter().filter(|r| r.state == VState::Stream).count(), 2, "no new voice");
    assert_eq!(s.count(), 1);
    let mut next = None;
    let mut t = 0;
    while next.is_none() && t < 500 {
        step(&mut v, &mut s, t);
        next = s.take_events().into_iter().find_map(|e| if let StreamEv::Next { tick, lba, .. } = e { Some((tick, lba)) } else { None });
        t += 1;
    }
    let (tick, lba) = next.expect("the loop part starts");
    assert_eq!(lba, LOOP);
    // 12348 samples at 3763 * 200 / 4096 per tick, key-on at ready_ticks
    let per_tick = 3763u64 * 200;
    let end_tick = READY + 1 + ((12348u64 << 12) / per_tick) as u32;
    assert!((tick as i64 - end_tick as i64).abs() <= 1, "tick {tick} vs {end_tick}");
    assert_eq!(s.count(), 1, "the stream goes on");
    assert_eq!(s.remaining(a), 0, "a looping part has no remaining time (chan flag 0x400)");
    for t2 in t..t + 400 {
        step(&mut v, &mut s, t2);
    }
    assert_eq!(s.playing(a), a, "the loop never ends");
    assert_eq!(s.buffered(a), 1);
}

#[test]
fn position_remaining_and_natural_end() {
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20));
    for t in 0..=READY {
        step(&mut v, &mut s, t);
    }
    let total = fixed_time(BLOCKS * 28, 44100);
    assert_eq!(s.remaining(a), total, "just keyed: the whole length");
    let t0 = READY + 1;
    for t in t0..t0 + 20 {
        step(&mut v, &mut s, t);
    }
    let st = s.stream(a).unwrap();
    let played = (20u64 * 3763 * 200) >> 12;
    assert_eq!(st.samples(), played);
    assert_eq!(st.elapsed_bytes() as u64, played / 28 * 16);
    let elapsed = fixed_time(bytes_to_samples(st.elapsed_bytes()), 44100);
    assert_eq!(s.remaining(a), total - elapsed);
    // run to the end: handle gone, voices free, key-offs queued
    let mut t = t0 + 20;
    while s.playing(a) != 0 && t < 400 {
        step(&mut v, &mut s, t);
        t += 1;
    }
    assert_eq!(s.playing(a), 0);
    assert_eq!(s.buffered(a), -1);
    assert_eq!(s.remaining(a), 0);
    assert!(v.rec.iter().all(|r| r.state != VState::Stream));
    let evs = s.take_events();
    assert_eq!(evs.iter().filter(|e| matches!(e, StreamEv::End { natural: true, .. })).count(), 2);
    assert!((t as i64 - (t0 as i64 + 68)).abs() <= 2, "ends after 0.28 s = 67.2 ticks, ended at {t}");
}

#[test]
fn pause_and_continue_write_vol_and_pitch_and_hold_the_position() {
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20));
    for t in 0..=READY + 10 {
        step(&mut v, &mut s, t);
    }
    v.take_events();
    let pos = s.stream(a).unwrap().samples();
    s.pause(&mut v, a);
    let ev = v.take_events();
    for x in [0x18, 0x19] {
        assert_eq!(writes(&ev, Reg::VolL, x), vec![0]);
        assert_eq!(writes(&ev, Reg::VolR, x), vec![0]);
        assert_eq!(writes(&ev, Reg::Pitch, x), vec![0]);
    }
    s.pause(&mut v, a);
    assert!(v.take_events().is_empty(), "pause while held is a complete no-op");
    let t0 = READY + 11;
    let rem = s.remaining(a);
    for t in t0..t0 + 30 {
        step(&mut v, &mut s, t);
    }
    assert_eq!(s.stream(a).unwrap().samples(), pos, "pitch 0: the SPU2 reads nothing");
    assert_eq!(s.remaining(a), rem);
    // cid 9 while paused rewrites the volume of state-4 voices too (0x16e74)
    v.take_events();
    v.set_group_volume(1, 0x200);
    assert!(!writes(&v.take_events(), Reg::VolL, 0x18).is_empty());
    s.resume(&mut v, a);
    let ev = v.take_events();
    assert_eq!(writes(&ev, Reg::Pitch, 0x18), vec![3763]);
    assert_eq!(writes(&ev, Reg::Pitch, 0x19), vec![3763]);
    let g = v.rec[0x18].group as i32;
    assert_eq!(writes(&ev, Reg::VolL, 0x18), vec![volume_reg(v.rec[0x18].l, g, &v.group_vol) as u32]);
    for t in t0 + 30..t0 + 40 {
        step(&mut v, &mut s, t);
    }
    assert!(s.stream(a).unwrap().samples() > pos, "playing again");
}

#[test]
fn held_start_keys_silent_and_cid46_starts_it() {
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20 | 1));
    for t in 0..=READY + 5 {
        step(&mut v, &mut s, t);
    }
    let ev = v.take_events();
    assert_eq!(writes(&ev, Reg::Pitch, 0x18), vec![0], "held: PITCH 0 at key-on");
    assert_eq!(writes(&ev, Reg::VolL, 0x18), vec![0]);
    assert_eq!(writes(&ev, Reg::Kon, 1), vec![3], "the voices are keyed, silent");
    assert_eq!(s.buffered(a), 1, "cid 79 sees the buffers while held: the EE then sends cid 46");
    assert_eq!(s.stream(a).unwrap().samples(), 0);
    s.resume(&mut v, a);
    let ev = v.take_events();
    assert_eq!(writes(&ev, Reg::Pitch, 0x18), vec![3763]);
    assert!(writes(&ev, Reg::VolL, 0x18)[0] != 0);
    let t = READY + 6;
    step(&mut v, &mut s, t);
    assert!(s.stream(a).unwrap().samples() > 0);
}

#[test]
fn start_chain_counts_and_disc_timing() {
    assert_eq!(start_chain(1600, 0x100, 0x7800), (2, 1));
    assert_eq!(start_chain(0x7800 - 0x130, 0x100, 0x7800), (2, 1), "t == half is still short");
    assert_eq!(start_chain(0x7800 - 0x12f, 0x100, 0x7800), (3, 2));
    assert_eq!(start_chain(40000, 0x100, 0x7800), (3, 2));
    assert_eq!(start_chain(100000, 0x100, 0x7800), (4, 2));
    // DiscTiming { 10, 1 }: 2 * 10 + 1 / 3 * 10 + 2 / 4 * 10 + 2 ticks after cid 44
    for (lba, want) in [(300u32, 21u32), (400, 32), (500, 42)] {
        let (mut v, mut s) = setup();
        s.disc = DiscTiming { cd_read_ticks: 10, spu_dma_ticks: 1 };
        let h = s.play(5, &mut v, &args(lba, 0, 1, 0, 0));
        for t in 5..5 + want {
            step(&mut v, &mut s, t);
            assert_eq!(s.buffered(h), 0, "lba {lba} tick {t}");
        }
        step(&mut v, &mut s, 5 + want);
        assert_eq!(s.buffered(h), 1, "lba {lba}");
        assert_eq!(s.take_events(), vec![StreamEv::Start { tick: 5 + want, voice: 0x18, extra: None, lba, held: false }]);
    }
    // INSTANT keys at the first tick, however long the file is
    let (mut v, mut s) = setup();
    assert_eq!(s.disc, DiscTiming::INSTANT);
    let h = s.play(7, &mut v, &args(500, 0, 1, 0, 0));
    step(&mut v, &mut s, 7);
    assert_eq!(s.buffered(h), 1);
}

#[test]
fn init_cfg_pages_half_ring_and_second_call() {
    let mut s = Streams::new();
    assert!(s.init_cfg(4, 0xf000));
    assert_eq!(s.half, 0x7800);
    assert!(!s.init_cfg(8, 0x2000), "a second cid 42 is error 0x25");
    assert_eq!(s.errors, vec![0x25]);
    assert_eq!(s.half, 0x7800, "and changes nothing");
    let mut t = Streams::new();
    t.init_cfg(2, 100);
    assert_eq!(t.half, 0x1000, "min 2 pages of 4 KB: fp 0x2000");
    let mut u = Streams::new();
    u.init_cfg(2, 0x5001);
    assert_eq!(u.half, 0x3000, "rounded up to 6 pages: fp 0x6000");
    u.spu_base = 0x5040;
    assert_eq!((u.ring_base(0), u.ring_base(3)), (0x5040, 0x5040 + 3 * 0x3000));
}

#[test]
fn pan_offsets_mono_extra_and_stereo() {
    let (mut v, mut s) = setup();
    let m = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0));
    let e = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20));
    let st = s.play(0, &mut v, &args(INTRO, LOOP, 2, 0, 0));
    step(&mut v, &mut s, 0);
    assert_eq!(s.stream(m).unwrap().pan_off, [0, 0]);
    assert_eq!(s.stream(e).unwrap().pan_off, [0x10e, 0]);
    assert_eq!(s.stream(st).unwrap().pan_off, [0x10e, 0x5a]);
    // chans: lowest free first
    assert_eq!((s.stream(m).unwrap().chan, s.stream(e).unwrap().chan, s.stream(st).unwrap().chan), ([0, 0], [1, 1], [2, 3]));
}

#[test]
fn stop_keeps_the_handle_until_the_stream_pass_then_queues_key_off_and_frees_after_envx() {
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20));
    for t in 0..4 {
        step(&mut v, &mut s, t);
    }
    v.take_events();
    let _ = s.take_events();
    assert!(s.stop(&mut v, a));
    // at the call: mute only (VOLL VOLR 0 on the chan voice, then the extra voice VOLR VOLL), no key-off, handle alive
    let ev = v.take_events();
    let seq: Vec<(Reg, u8)> = ev.iter().map(|e| (e.reg, e.index)).collect();
    assert_eq!(seq, [(Reg::VolL, 0x18), (Reg::VolR, 0x18), (Reg::VolR, 0x19), (Reg::VolL, 0x19)]);
    assert!(ev.iter().all(|e| e.value == 0));
    assert_eq!(v.pending_koff, [0, 0]);
    assert_eq!((v.rec[0x18].l, v.rec[0x18].r, v.rec[0x19].l, v.rec[0x19].r), (0, 0, 0, 0));
    assert_eq!((s.playing(a), s.buffered(a)), (a, 1), "cid 25 / 79 still answer");
    assert_eq!(s.stream(a).unwrap().vol, 0);
    assert!(s.stop(&mut v, a), "a second cid 21 is accepted too");
    // the next stream pass: voices become state 1 with priority 0, KOFF queued, handle dead, chans free
    s.tick(4, &mut v, &vags());
    assert_eq!((s.playing(a), s.buffered(a), s.count()), (0, -1, 0));
    assert_eq!(v.pending_koff[1], 0b11);
    for x in [0x18, 0x19] {
        assert_eq!((v.rec[x].state, v.rec[x].priority), (VState::Keyed, 0));
    }
    let evs = s.take_events();
    let ends: Vec<_> = evs.iter().filter_map(|e| if let StreamEv::End { voice, natural, .. } = e { Some((*voice, *natural)) } else { None }).collect();
    assert_eq!(ends, [(0x19, false), (0x18, false)], "extra first");
    // the KOFF flush, then the records are freed only by the ENVX poll
    v.apply_key_offs(4);
    assert!(v.take_events().iter().any(|e| e.reg == Reg::Koff));
    assert!(v.rec[0x18].state != VState::Free, "not freed without the ENVX poll");
    for _ in 0..200 {
        v.poll_end(5);
        if v.rec[0x18].state == VState::Free && v.rec[0x19].state == VState::Free {
            break;
        }
    }
    assert!(v.rec[0x18].state == VState::Free && v.rec[0x19].state == VState::Free, "freed after ENVX 0");
    let d = s.play(6, &mut v, &args(INTRO, 0, 1, 0, 0));
    assert_eq!((s.stream(d).unwrap().voices[0], s.stream(d).unwrap().chan[0]), (Some(0x18), 0));
}

#[test]
fn stop_all_marks_every_stream_and_frees_them_in_the_first_pass() {
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20));
    let b = s.play(0, &mut v, &args(LOOP, LOOP + 1, 2, 0, 0));
    let c = s.play(0, &mut v, &args(300, 0, 4, 0, 0));
    assert!(!s.stop(&mut v, 0x8408_0002), "unknown handle");
    for t in 0..3 {
        step(&mut v, &mut s, t);
    }
    assert_eq!(v.rec.iter().filter(|r| r.state == VState::Stream).count(), 2 + 2 + 1);
    v.take_events();
    s.stop_all(10, &mut v);
    assert_eq!(s.count(), 0);
    assert!(v.rec.iter().all(|r| r.state != VState::Stream));
    for h in [a, b, c] {
        assert_eq!((s.playing(h), s.buffered(h)), (0, -1));
    }
    assert_eq!(v.pending_koff[1] & 0b1_1111, 0b1_1111, "0x18..0x1c queued for key-off");
    assert!(v.take_events().iter().any(|e| e.reg == Reg::VolL && e.value == 0), "muted first");
    // the voices come back (after the ENVX poll frees them): the next stream takes the first free voices again
    v.apply_key_offs(10);
    for _ in 0..200 {
        v.poll_end(11);
    }
    let d = s.play(12, &mut v, &args(INTRO, 0, 1, 0, 0));
    assert_eq!(s.stream(d).unwrap().voices[0], Some(0x18));
}

#[test]
fn cid33_update_scale_pan_clamp_and_write_order() {
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0));
    assert_eq!(s.stream(a).unwrap().vol, 127, "1024 * 127 >> 10");
    // not keyed yet: only the handler fields change, no register write
    assert!(s.update(&mut v, a, 512, -2));
    assert_eq!(s.stream(a).unwrap().vol, 63);
    assert!(v.take_events().is_empty());
    assert!(s.update(&mut v, a, 0x7fff_ffff, 90));
    assert_eq!((s.stream(a).unwrap().vol, s.stream(a).unwrap().pan), (63, 90));
    assert!(s.update(&mut v, a, -200, -1));
    assert_eq!((s.stream(a).unwrap().vol, s.stream(a).unwrap().pan), (127, 0), "-200 -> 200, clamped to 0x7f");
    assert!(s.update(&mut v, a, i32::MIN, 5));
    assert_eq!(s.stream(a).unwrap().vol, 0, "-i32::MIN wraps to 0 as s16: no lower clamp");
    assert!(s.update(&mut v, a, 0x7fff_fff0, 5), "32-bit wrapping scale: no overflow panic");
    assert!(!s.update(&mut v, 0x8409_0001, 1, 1));
    let p = PlayVag::cid44(&[7, 8, 0x0400_0123, 0xfffe_0456, 3, 0x8400_0002, 0x24]);
    assert_eq!((p.lba, p.lba_r, p.ring, p.volume, p.pan, p.group, p.after, p.flags), (7, 8, [0x123, 0x456], 0x400, -2, 3, 0x8400_0002, 0x24));
    // wrap value: ((s << 7) - s) >> 10 in 32 bits
    let s2 = 0x0200_0001i32;
    let want = (s2.wrapping_shl(7).wrapping_sub(s2) >> 10) as i16;
    s.update(&mut v, a, s2, 0);
    assert_eq!(s.stream(a).unwrap().vol, if want >= 0x80 { 0x7f } else { want });

    // keyed mono + extra: write order chan VOLL, VOLR, then extra VOLR := Lv, VOLL := Rv; the records swap
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20));
    step(&mut v, &mut s, 0);
    v.take_events();
    assert!(s.update(&mut v, a, 512, 0));
    let ev = v.take_events();
    let seq: Vec<(Reg, u8)> = ev.iter().map(|e| (e.reg, e.index)).collect();
    assert_eq!(seq, [(Reg::VolL, 0x18), (Reg::VolR, 0x18), (Reg::VolR, 0x19), (Reg::VolL, 0x19)]);
    let (lv, rv) = (ev[0].value, ev[1].value);
    assert_eq!((ev[2].value, ev[3].value), (lv, rv));
    assert_eq!((v.rec[0x19].l, v.rec[0x19].r), (v.rec[0x18].r, v.rec[0x18].l));
    let g = v.rec[0x18].group as i32;
    assert_eq!(lv, volume_reg(v.rec[0x18].l, g, &v.group_vol) as u32);
    // stereo: each chan writes only its own VOLL, VOLR, with its own pan offset
    let (mut v, mut s) = setup();
    let b = s.play(0, &mut v, &args(INTRO, LOOP, 2, 0, 0));
    step(&mut v, &mut s, 0);
    v.take_events();
    s.update(&mut v, b, 512, -2);
    let ev = v.take_events();
    let seq: Vec<(Reg, u8)> = ev.iter().map(|e| (e.reg, e.index)).collect();
    assert_eq!(seq, [(Reg::VolL, 0x18), (Reg::VolR, 0x18), (Reg::VolL, 0x19), (Reg::VolR, 0x19)]);
    assert_eq!((ev[0].value != 0, ev[1].value), (true, 0), "left channel hard left");
    assert_eq!((ev[2].value, ev[3].value != 0), (0, true), "right channel hard right");
    // held: the records are updated, no register write
    let (mut v, mut s) = setup();
    let h = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20 | 1));
    step(&mut v, &mut s, 0);
    v.take_events();
    s.update(&mut v, h, 512, 0);
    assert!(v.take_events().is_empty());
    assert_ne!(v.rec[0x18].l, 0);
}

#[test]
fn pause_and_continue_write_order() {
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20));
    step(&mut v, &mut s, 0);
    v.take_events();
    // pause: chan 0 (VOLL VOLR PITCH), extra (VOLL VOLR PITCH)  [mono: no chan 1]
    s.pause(&mut v, a);
    let seq: Vec<(Reg, u8)> = v.take_events().iter().map(|e| (e.reg, e.index)).collect();
    assert_eq!(seq, [(Reg::VolL, 0x18), (Reg::VolR, 0x18), (Reg::Pitch, 0x18), (Reg::VolL, 0x19), (Reg::VolR, 0x19), (Reg::Pitch, 0x19)]);
    // continue: PITCH phase first, then the VOL phase, each in the order chan 0, extra
    s.resume(&mut v, a);
    let seq: Vec<(Reg, u8)> = v.take_events().iter().map(|e| (e.reg, e.index)).collect();
    assert_eq!(seq, [(Reg::Pitch, 0x18), (Reg::Pitch, 0x19), (Reg::VolL, 0x18), (Reg::VolR, 0x18), (Reg::VolL, 0x19), (Reg::VolR, 0x19)]);
    s.resume(&mut v, a);
    assert!(v.take_events().is_empty(), "continue without the held bit is a no-op");
    // stereo: chan 0, chan 1
    let (mut v, mut s) = setup();
    let b = s.play(0, &mut v, &args(INTRO, LOOP, 2, 0, 0));
    step(&mut v, &mut s, 0);
    v.take_events();
    s.pause(&mut v, b);
    let seq: Vec<(Reg, u8)> = v.take_events().iter().map(|e| (e.reg, e.index)).collect();
    assert_eq!(seq, [(Reg::VolL, 0x18), (Reg::VolR, 0x18), (Reg::Pitch, 0x18), (Reg::VolL, 0x19), (Reg::VolR, 0x19), (Reg::Pitch, 0x19)]);
    s.resume(&mut v, b);
    let seq: Vec<(Reg, u8)> = v.take_events().iter().map(|e| (e.reg, e.index)).collect();
    assert_eq!(seq, [(Reg::Pitch, 0x18), (Reg::Pitch, 0x19), (Reg::VolL, 0x18), (Reg::VolR, 0x18), (Reg::VolL, 0x19), (Reg::VolR, 0x19)]);
    // pause before the key-on writes even the not-yet-keyed chan; continue writes nothing for it, only clears the bit
    let (mut v, mut s) = setup();
    s.disc = DiscTiming { cd_read_ticks: 5, spu_dma_ticks: 0 };
    let c = s.play(0, &mut v, &args(300, 0, 1, 0, 0));
    s.pause(&mut v, c);
    assert_eq!(v.take_events().len(), 3, "VOLL VOLR PITCH of the unkeyed chan");
    s.resume(&mut v, c);
    assert!(v.take_events().is_empty());
}

#[test]
fn elapsed_is_sampled_per_pass_skips_the_first_and_freezes_while_paused() {
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0));
    step(&mut v, &mut s, 0); // key-on
    assert_eq!(s.stream(a).unwrap().elapsed_bytes(), 0);
    step(&mut v, &mut s, 1); // first pass: the position moves, the elapsed counter does not
    assert!(s.stream(a).unwrap().samples() > 0);
    assert_eq!(s.stream(a).unwrap().elapsed_bytes(), 0, "first pass after key-on skipped");
    for t in 2..12 {
        step(&mut v, &mut s, t);
    }
    let st = s.stream(a).unwrap();
    assert_eq!(st.elapsed_bytes(), (st.samples() / 28) as u32 * 16);
    assert!(st.elapsed_bytes() > 0);
    s.pause(&mut v, a);
    let e = s.stream(a).unwrap().elapsed_bytes();
    for t in 12..20 {
        step(&mut v, &mut s, t);
    }
    assert_eq!(s.stream(a).unwrap().elapsed_bytes(), e, "frozen while paused");
}

#[test]
fn natural_end_is_noticed_at_a_stream_pass_after_the_end_block_and_frees_through_the_poll() {
    let (mut v, mut s) = setup();
    let a = s.play(0, &mut v, &args(300, 0, 1, 0, 0));
    let mut t = 0;
    while s.playing(a) != 0 && t < 400 {
        step(&mut v, &mut s, t);
        t += 1;
    }
    assert_eq!(s.playing(a), 0);
    // 1600 bytes at 22.05 kHz = 2800 samples at pitch 1881: 2800 * 4096 / (1881 * 200) = 30.5 ticks
    assert!((29..=34).contains(&t), "ended at {t}");
    assert!(v.rec[0x18].state != VState::Stream, "key-off queued, the record waits for the ENVX poll");
}

// ---------------------------------------------------------------- stage 11: the PCM of the stream voices

/// A VAG payload of `n` blocks the way the disc files end: [.., 0, 1, 7] flags; audible data (predictor 0, nibbles 7).
fn raw_payload(n: u32) -> Arc<Vec<u8>> {
    let mut b = vec![0u8; n as usize * 16];
    for k in 0..n as usize {
        b[k * 16 + 2..k * 16 + 16].fill(0x77);
    }
    b[(n as usize - 2) * 16 + 1] = 1;
    b[(n as usize - 1) * 16 + 1] = 7;
    Arc::new(b)
}

fn flag(buf: &[u8], block: usize) -> u8 {
    buf[block * 16 + 1]
}

/// One driver iteration with the SPU2 running 200 samples (the order of `tick_once`).
fn step_spu(v: &mut SpuVoices, s: &mut Streams, now: u32) {
    v.step_spu(200);
    step(v, s, now);
}

#[test]
fn part_buffers_follow_the_ring_rules() {
    let raw = raw_payload(10);
    let one = new_part_buf(&raw, false);
    assert_eq!(one.len(), 160);
    assert_eq!((flag(&one, 8), flag(&one, 9)), (1, 7), "the file's own end flags stay");
    let lp = new_part_buf(&raw, true);
    assert_eq!((flag(&lp, 0), flag(&lp, 8), flag(&lp, 9)), (4, 0, 3), "loop start on the first block, loop end + repeat on the last");
    // a queued loop behind an intro: the intro's end flags are cleared, the loop part is a loop
    let mut b = new_part_buf(&raw, false);
    append_part(&mut b, &raw, true);
    assert_eq!(b.len(), 320);
    assert_eq!((flag(&b, 8), flag(&b, 9)), (0, 0), "gapless: no end flags between the parts");
    assert_eq!((flag(&b, 10), flag(&b, 19)), (4, 3));
    // a trailing partial block is dropped
    let mut odd = (*raw).clone();
    odd.extend_from_slice(&[1, 2, 3]);
    assert_eq!(new_part_buf(&odd, false).len(), 160);
}

#[test]
fn stream_voices_play_the_payload_and_end_where_the_driver_model_does() {
    let (mut v, mut s) = setup();
    s.payload.insert(INTRO, raw_payload(BLOCKS));
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20));
    let (mut stop_tick, mut t) = (None, 0u32);
    let mut ev_all: Vec<SpuWrite> = Vec::new();
    while s.playing(a) != 0 && t < 400 {
        step_spu(&mut v, &mut s, t);
        ev_all.extend(v.take_events());
        if stop_tick.is_none() && t > 3 && !v.spu[0x18].active() && v.spu[0x18].cur.played() > 0 {
            stop_tick = Some(t);
        }
        t += 1;
    }
    // StreamSrc comes before the voice's SSA, one id for the chan voice and the extra voice
    let src: Vec<_> = ev_all.iter().enumerate().filter(|(_, e)| e.reg == Reg::StreamSrc).collect();
    assert_eq!(src.len(), 2);
    assert_eq!(src[0].1.value, src[1].1.value, "the extra voice shares the stream's data");
    for (i, e) in &src {
        let ssa = ev_all.iter().position(|w| w.reg == Reg::Ssa && w.index == e.index).unwrap();
        assert!(*i < ssa, "StreamSrc before SSA on voice {}", e.index);
    }
    assert!(v.stream_by_id(src[0].1.value).is_some());
    // the oracle voice decoded the real blocks: it stopped itself at the file's end flag (block 439 of 441), within 2 ticks
    // of the model's natural end (t - 1 is the tick the stream pass noticed it)
    let stop = stop_tick.expect("the voice stopped by itself");
    assert!((stop as i64 - (t as i64 - 1)).abs() <= 2, "voice stopped at tick {stop}, model ended at {}", t - 1);
    assert!(v.stream_by_id(src[0].1.value).is_some());
}

#[test]
fn queued_loop_part_is_appended_and_the_voice_never_ends() {
    let (mut v, mut s) = setup();
    s.payload.insert(INTRO, raw_payload(BLOCKS));
    s.payload.insert(LOOP, raw_payload(BLOCKS));
    let a = s.play(0, &mut v, &args(INTRO, 0, 1, 0, 0x20));
    assert_eq!(s.play(0, &mut v, &args(LOOP, 0, 1, a, 0x24)), 1);
    for t in 0..700 {
        step_spu(&mut v, &mut s, t);
        if t == 3 {
            // keyed: the buffer holds both parts, the junction has no end flags
            let d = v.stream_of(0x18).expect("stream data on the chan voice").read().unwrap().clone();
            assert_eq!(d.len(), 2 * BLOCKS as usize * 16);
            let j = BLOCKS as usize;
            assert_eq!((flag(&d, j - 2), flag(&d, j - 1), flag(&d, j), flag(&d, 2 * j - 1)), (0, 0, 4, 3));
        }
        if t > 5 {
            assert!(v.spu[0x18].active() && v.spu[0x19].active(), "voice {t} playing through the junction and the loop");
        }
    }
    assert_eq!(s.playing(a), a);
    assert!(v.spu[0x18].cur.endx, "the loop end was passed at least once");
}

#[test]
fn adopted_music_stream_keys_on_from_the_start() {
    let (mut v, mut s) = setup();
    s.payload.insert(LOOP, raw_payload(BLOCKS));
    assert!(s.adopt_playing(0, &mut v, 0x8401_0017, LOOP, 0x24, 1024, false));
    assert!(!s.adopt_playing(0, &mut v, 0x8401_0017, LOOP, 0x24, 1024, false), "slot taken");
    for t in 0..5 {
        step_spu(&mut v, &mut s, t);
    }
    assert_eq!(s.playing(0x8401_0017), 0x8401_0017);
    assert!(v.spu[0x18].active() && v.spu[0x19].active(), "two voices (mono source on both speakers)");
    let held = {
        let (mut v2, mut s2) = setup();
        s2.payload.insert(LOOP, raw_payload(BLOCKS));
        assert!(s2.adopt_playing(0, &mut v2, 0x8400_0001, LOOP, 0x24, 1024, true));
        for t in 0..5 {
            step_spu(&mut v2, &mut s2, t);
        }
        v2.spu[0x18].pitch
    };
    assert_eq!(held, 0, "a paused channel keys on held (PITCH 0)");
}
