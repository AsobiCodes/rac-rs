//! Tests of driver.rs (stage 7b): allocator, tick, save/load, adoption of the s07 savestate, cid answers.
//! Tests that need the disc or the savestates print a note and return when they are absent.
#![allow(unused_imports)]
use super::*;
use crate::snd989::testdata;

fn rng0() -> Rng250 {
    rng_from_iop(&[]).unwrap_or_else(|| Rng250::from_state([0; LEN], 0).unwrap())
}

fn s07() -> Option<(Vec<u8>, Vec<u8>)> {
    let iop = testdata::read_state("s07/iopMemory.bin")?;
    let ee = testdata::read_state("s07/eeMemory.bin")?;
    Some((iop, ee))
}

#[test]
fn allocator_first_fit_and_merge() {
    let mut a = Spu2Alloc::default();
    let x = a.alloc(0x40).unwrap();
    assert_eq!(x, 0x5040);
    let y = a.alloc(0x41).unwrap();
    assert_eq!(y, 0x5080);
    let z = a.alloc(0x10).unwrap();
    assert_eq!(z, 0x5100);
    a.release(x, 0x40);
    assert_eq!(a.alloc(0x40), Some(0x5040), "first fit reuses the hole");
    a.release(0x5040, 0x40);
    a.release(y, 0x41);
    a.release(z, 0x10);
    assert_eq!(a.regions(), &[(0x5040, 0x20_0000)], "neighbours merged back into one region");
    assert_eq!(Spu2Alloc::round(0x41), 0x80);
}

#[test]
fn cid42_is_the_first_allocation() {
    let mut i = Inner::blank(rng0());
    let r = i.command(0, 42, &[STREAM_COUNT as u32, STREAM_BUFFER, 0, 0, 0, 0, 0], &mut None);
    assert_eq!(r, Some(u32::MAX));
    assert_eq!(i.streams.spu_base, 0x5040);
    let (a, n) = i.stream_mem.unwrap();
    assert_eq!(a, 0x5040);
    assert!(n >= 0x2000);
    // A bank loaded after it lands behind the ring block.
    let next = i.spu_mem.alloc(0x40).unwrap();
    assert_eq!(next, 0x5040 + n);
}

#[test]
fn tick_of_is_240_hz_over_50() {
    assert_eq!(tick_of(0), 0);
    assert_eq!(tick_of(5), 24);
    assert_eq!(tick_of(50), 240);
    assert_eq!(tick_of(1), 4);
    assert_eq!(tick_of(2), 9);
    assert_eq!(tick_of(1000), 4800);
}

fn global_with_sound() -> Option<(Bank, i32)> {
    let c = disc_banks().iter().find(|c| c.kind == BankKind::Global)?;
    let k = (0..400).find(|&k| c.bank.playable(k).is_some())?;
    Some((c.bank.clone(), k))
}

#[test]
fn unknown_handles_cid25_cid79_and_bad_bank_play() {
    let mut i = Inner::blank(rng0());
    let z = [0u32; 7];
    let mut unk = z;
    unk[0] = 0x8512_3456;
    assert_eq!(i.command(0, 25, &unk, &mut None), Some(0), "cid 25 on an unknown sound handle");
    assert_eq!(i.command(0, 79, &unk, &mut None), Some(u32::MAX), "cid 79 on an unknown stream handle");
    let mut st = z;
    st[0] = 0x8412_0001;
    assert_eq!(i.command(0, 79, &st, &mut None), Some(u32::MAX));
    // cid 17 on a bank handle that is not loaded replies 0.
    let mut p = z;
    p[0] = 0x0019_ce00;
    p[1] = 3;
    assert_eq!(i.command(0, 17, &p, &mut None), Some(0));
    assert_eq!(i.command(0, 55, &z, &mut None), None, "disc reads stay with the HLE");
}

#[test]
fn play_answers_handle_until_freed() {
    let Some((bank, k)) = global_with_sound() else {
        eprintln!("skip: no disc image");
        return;
    };
    let mut i = Inner::blank(rng0());
    let h = i.add_bank(BankKind::Global, bank, Some(FIRST_LEVEL_HANDLE));
    let mut p = [0u32; 7];
    p[0] = h;
    p[1] = k as u32;
    p[2] = 0x400;
    let sh = i.command(1, 17, &p, &mut None).unwrap();
    assert_ne!(sh, 0);
    assert_eq!(sh >> 24, 0x85);
    let mut q = [0u32; 7];
    q[0] = sh;
    assert_eq!(i.command(1, 25, &q, &mut None), Some(sh));
    assert_eq!(i.command(1, 21, &q, &mut None), Some(u32::MAX));
    assert_eq!(i.command(2, 25, &q, &mut None), Some(0), "freed sounds answer 0");
}

fn run_a_bit(i: &mut Inner, from: u32, to: u32) {
    for vb in from..to {
        i.advance_to(vb);
        let mut u = [0u32; 7];
        u[0] = i.drv.core.tick; // harmless: unknown handle update
        let _ = i.command(vb, 33, &u, &mut None);
    }
}

#[test]
fn save_load_round_trip() {
    let Some((bank, k)) = global_with_sound() else {
        eprintln!("skip: no disc image");
        return;
    };
    let mut s = Snd989::default();
    s.record = true;
    let mut inner = Inner::blank(rng0());
    inner.record = true;
    let h = inner.add_bank(BankKind::Global, bank, Some(FIRST_LEVEL_HANDLE));
    let mut p = [0u32; 7];
    p[0] = h;
    p[1] = k as u32;
    p[2] = 0x400;
    let sh = inner.command(3, 17, &p, &mut None).unwrap();
    assert_ne!(sh, 0);
    run_a_bit(&mut inner, 3, 20);
    s.inner = Some(Box::new(inner));
    let bytes = s.save();
    let mut t = Snd989::default();
    t.load(&bytes);
    assert_eq!(t.save(), bytes, "save -> load -> save is byte equal");
    // A loaded copy continues exactly like the original.
    let (mut a, mut b) = (s, t);
    {
        let (x, y) = (a.inner.as_mut().unwrap(), b.inner.as_mut().unwrap());
        x.out.writes.clear();
        y.out.writes.clear();
        run_a_bit(x, 20, 20 + 125);
        run_a_bit(y, 20, 20 + 125);
        assert_eq!(x.out.writes, y.out.writes);
        assert_eq!(x.tick(), y.tick());
    }
    assert_eq!(a.save(), b.save());
}

#[test]
fn s07_adoption() {
    let Some((iop, ee)) = s07() else {
        eprintln!("skip: no s07 state");
        return;
    };
    if disc_banks().is_empty() {
        eprintln!("skip: no disc image");
        return;
    }
    let vb = u32::from_le_bytes(ee[0x15ed48..0x15ed4c].try_into().unwrap());
    let i = Inner::adopt(&iop, &ee, false, vb);
    let e = i.banks.entries();
    let lvl: Vec<_> = e.iter().filter(|(_, k, _)| *k == BankKind::Level).collect();
    assert!(!lvl.is_empty(), "no level bank adopted: {:?}", i.notes);
    assert!(lvl.iter().any(|(h, _, _)| *h == FIRST_LEVEL_HANDLE), "level bank handle: {:x?}", e.iter().map(|x| x.0).collect::<Vec<_>>());
    // The global bank (10 sounds) is not resident in s07 and the SBlk at 0x1a8200 (id MN, 5 sounds) is on no disc section
    // (sound_grains.md, 2026-10-13): only the level bank is adopted.
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(i.streams.spu_base, 0x5040);
    assert_eq!(i.tick(), tick_of(vb).wrapping_add(i.tick_off));
    // s07 has no SPU2 dump: the running driver is not adopted (stage 8c rule), no handler is in use.
    assert!((0..64).all(|s| !i.drv.core.slot(s).in_use()));
    // cid 25 / 79 on unknown handles.
    let mut me = i;
    let mut u = [0u32; 7];
    u[0] = 0x8500_0000 | 0x0777;
    assert_eq!(me.command(vb, 25, &u, &mut None), Some(0));
    assert_eq!(me.command(vb, 79, &u, &mut None), Some(u32::MAX));
}

fn vb_of(ee: &[u8]) -> u32 {
    u32::from_le_bytes(ee[0x15ed48..0x15ed4c].try_into().unwrap())
}

/// BUG 1 (7c): a bank the game loads at run time that is on no disc section (the vendor bank, id MN, 5 sounds, loaded from the
/// game's own file through fno 0x57 into a fresh handle) made `load` panic. It is now saved whole: adopt a state, add such a
/// bank, save -> load -> save is byte equal and the bank survives intact.
#[test]
fn non_disc_bank_survives_save_load() {
    let Some(ee) = testdata::read_state("cov/00290_s07_travel13_287/eeMemory.bin") else {
        eprintln!("skip: no cov state");
        return;
    };
    let iop = testdata::read_state("cov/00290_s07_travel13_287/iopMemory.bin").unwrap();
    if disc_banks().is_empty() {
        eprintln!("skip: no disc image");
        return;
    }
    let mut i = Inner::adopt(&iop, &ee, false, vb_of(&ee));
    let before = i.banks.entries().len();
    // A level bank of the disc with another id word, one sample byte changed: on no disc section.
    let mut vendor = disc_banks().iter().find(|c| c.kind == BankKind::Level).unwrap().bank.clone();
    vendor.signature.id = 0x0100_4e4d;
    vendor.data[3] ^= 0x55;
    let h = i.add_bank(BankKind::Level, vendor.clone(), None);
    assert_eq!(i.banks.entries().len(), before + 1);
    let mut s = Snd989::default();
    s.inner = Some(Box::new(i));
    let bytes = s.save();
    let mut t = Snd989::default();
    t.load(&bytes);
    assert_eq!(t.save(), bytes, "adopt -> save -> load -> save is byte equal");
    let back = t.inner.as_ref().unwrap().banks.get(h).expect("the non-disc bank is back");
    assert_eq!(**back, vendor);
    assert_eq!(t.inner.as_ref().unwrap().banks.entries().len(), before + 1);
}

/// 7d: a state without an IOP image adopts the level bank from the disc by the EE level number, under the EE's handle word.
#[test]
fn iop_less_state_adopts_the_level_bank_by_level_number() {
    if disc_banks().is_empty() {
        eprintln!("skip: no disc image");
        return;
    }
    for (state, level, handle) in [("s08", 4u32, 0x19ce00u32), ("s03", 0, 0x19cc00), ("s05", 1, 0x19b300)] {
        let Some(ee) = testdata::read_state(&format!("{state}/eeMemory.bin")) else {
            eprintln!("skip: no {state}");
            continue;
        };
        assert_eq!(ee[0x15ed84] as u32, level, "{state}");
        let i = Inner::adopt(&[], &ee, false, vb_of(&ee));
        let e = i.banks.entries();
        assert_eq!(e.len(), 1, "{state}: {e:?} {:?}", i.notes);
        assert_eq!((e[0].0, e[0].1), (handle, BankKind::Level), "{state}");
        let want = disc_banks().iter().find(|c| c.level == Some(level)).unwrap();
        assert_eq!(e[0].2, want.sig);
        // cid 17 on that handle now starts a sound (index 0 of the level bank has grains), like retail.
        let mut me = i;
        let vb = vb_of(&ee);
        let k = (0..want.bank.sounds.len() as i32).find(|&k| want.bank.playable(k).is_some()).unwrap();
        let mut p = [0u32; 7];
        p[0] = handle;
        p[1] = k as u32;
        p[2] = 0x400;
        assert_ne!(me.command(vb, 17, &p, &mut None), Some(0), "{state}: cid 17 answers");
    }
}


/// Stage 8: a state with sounds running (fg1/10_0504 + its spu_voices.bin) keeps them: handlers, voices and their SPU2 state
/// are adopted, answer cid 25 with the EE's own handle words, and survive save -> load -> save.
#[test]
fn running_voices_survive_adopt_save_load() {
    let st = concat!(env!("CARGO_MANIFEST_DIR"), "/states/fg1/10_0504");
    let sv = std::env::var("ORACLE_SPUV_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../scratch_oracle_snd/spuv").into());
    let (Ok(iop), Ok(ee), Ok(spuv)) = (std::fs::read(format!("{st}/iopMemory.bin")), std::fs::read(format!("{st}/eeMemory.bin")), std::fs::read(format!("{sv}/fg1/10_0504/spu_voices.bin"))) else {
        eprintln!("skip: no fg1/10_0504 state or spu_voices.bin");
        return;
    };
    if disc_banks().is_empty() {
        eprintln!("skip: no disc image");
        return;
    }
    let vb = u32::from_le_bytes(ee[0x15ed48..0x15ed4c].try_into().unwrap());
    let i = Inner::adopt_with(&iop, &ee, false, vb, &spuv);
    let busy = (0..64).filter(|&s| i.drv.core.slot(s).in_use()).count();
    let live = i.drv.voices.spu.iter().filter(|v| v.active()).count();
    assert!(busy >= 15 && live >= 15, "busy {busy} live {live}: {:?}", i.notes);
    // the EE's own handle word of slot 3 answers as playing
    let mut me = i;
    let mut u = [0u32; 7];
    u[0] = 0x8503_1d34;
    assert_ne!(me.command(vb, 25, &u, &mut None), Some(0), "pre-state sound dropped");
    let a = Snd989 { inner: Some(Box::new(me)), record: false, spuv: Vec::new() }.save();
    let mut b = Snd989::default();
    b.load(&a);
    assert_eq!(b.save(), a);
}

/// PCSX2 ADPCM decode of one block (SPU2 filter table), `hist` = (prev1, prev2); returns the 28 PCM samples.
fn adpcm_block(b: &[u8], hist: &mut (i32, i32)) -> [i32; 28] {
    const F: [(i32, i32); 5] = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)];
    let (shift, filt) = ((b[0] & 15) as i32, ((b[0] >> 4) & 7) as usize);
    let f = F[filt.min(4)];
    let mut out = [0i32; 28];
    for n in 0..28 {
        let nib = ((b[2 + n / 2] >> (4 * (n & 1))) & 15) as i32;
        let nib = if nib >= 8 { nib - 16 } else { nib };
        let v = ((nib << 12) >> shift) + ((hist.0 * f.0 + hist.1 * f.1 + 32) >> 6);
        let v = v.clamp(-32768, 32767);
        *hist = (v, hist.0);
        out[n] = v;
    }
    out
}

/// Stage 10a: the SPU2 cursor's decoder (spu2::decode_block) equals this independent decode for every shift the disc uses
/// (0..12) and all five filters, history carried across blocks.
#[test]
fn spu2_decode_block_equals_reference() {
    let mut x = 0x1234_5678u32;
    let (mut h, mut p1, mut p2) = ((0, 0), 0, 0);
    for n in 0..2000 {
        let mut blk = [0u8; 16];
        for v in blk.iter_mut() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *v = x as u8;
        }
        blk[0] = ((n % 5) << 4) as u8 | (n / 5 % 13) as u8;
        let want = adpcm_block(&blk, &mut h);
        let got = crate::spu2::decode_block(&blk, &mut p1, &mut p2);
        assert_eq!(got.map(|s| s as i32), want, "block {n}");
        assert_eq!((p1, p2), h);
    }
}

/// Stage 10a (supersedes the 8b cursor check): an adopted running voice carries the SPU2 decoder as it was (NextA, the FIFO
/// and its counters, Prev1/Prev2); when NextA's block was already partly decoded, `adopt_cursor` rebuilds SBuffer from the
/// FIFO and the bank data and must reproduce both the decoded FIFO samples and the dumped history exactly (no note). The
/// window at the read position is the sample the PCSX2 voice reads next.
#[test]
fn adopted_cursor_is_the_sample_the_voice_plays() {
    let sv_dir = std::env::var("ORACLE_SPUV_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../scratch_oracle_snd/spuv").into());
    if disc_banks().is_empty() {
        eprintln!("skip: no disc image");
        return;
    }
    let (mut seen, mut rebuilt, mut notes) = (0, 0, Vec::new());
    let mut names: Vec<String> = ["fg1/10_0504", "fg1/10_0382", "fg1/4_0159", "fg1/1_0018", "fg1/0_0001", "fg1/0_0002", "fg1/0_0007"].map(String::from).to_vec();
    // ORACLE_SPUV_ALL=1: every state of the spuv directory
    if std::env::var("ORACLE_SPUV_ALL").is_ok() {
        names.clear();
        for k in std::fs::read_dir(&sv_dir).into_iter().flatten().flatten() {
            for s in std::fs::read_dir(k.path()).into_iter().flatten().flatten() {
                names.push(format!("{}/{}", k.file_name().to_string_lossy(), s.file_name().to_string_lossy()));
            }
        }
    }
    for name in &names {
        let st = format!("{}/states/{name}", env!("CARGO_MANIFEST_DIR"));
        let (Ok(iop), Ok(ee), Ok(spuv)) = (std::fs::read(format!("{st}/iopMemory.bin")), std::fs::read(format!("{st}/eeMemory.bin")), std::fs::read(format!("{sv_dir}/{name}/spu_voices.bin"))) else {
            eprintln!("skip {name}: no state or spu_voices.bin");
            continue;
        };
        let vb = u32::from_le_bytes(ee[0x15ed48..0x15ed4c].try_into().unwrap());
        let i = Inner::adopt_with(&iop, &ee, false, vb, &spuv);
        let recs = crate::snd989::adopt::parse_spuv(&spuv).expect("spu_voices.bin version 3");
        notes.extend(i.notes.iter().filter(|n| n.contains("SBuffer") || n.contains("NextA")).map(|n| format!("{name}: {n}")));
        for v in 0..48 {
            let sp = &i.drv.voices.spu[v];
            if !sp.active() {
                continue;
            }
            seen += 1;
            rebuilt += sp.cur.sbuf.is_some() as u32;
            assert_eq!(sp.cur.window()[0], recs[v].next_sample, "{name} voice {v}");
            assert_eq!((sp.cur.w, sp.cur.r), (recs[v].fifo_w, recs[v].fifo_r));
        }
    }
    eprintln!("cursor check: {seen} live voices, {rebuilt} SBuffer rebuilds, notes {notes:?}");
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn play_position_rule() {
    use crate::snd989::adopt::play_position;
    // NextA 0x252ab (2 groups of block 0x252a8 decoded), write 0x139c, read 0x138c: 16 samples behind = block 0x252a0 sample 20
    assert_eq!(play_position(0x252ab, 0x139c, 0x138c), Some((0x252a0, 20)));
    // NextA at a block start (previous block fully decoded): 28 - 16 + 0
    assert_eq!(play_position(0x252a8, 0x139c, 0x138c), Some((0x252a0, 12)));
    assert_eq!(play_position(0x252ab, 10, 40), None);
}

/// Stage 8c: a state without SPU2 voice state (s07: only voices.txt, which is not read) gets no driver adoption and no invented
/// voices (volume and loop position would be estimates): stage 7d behaviour. With the SPU2 state (fg1/10_0504) the driver is
/// adopted and the R250 state is the IOP image's.
#[test]
fn driver_adoption_needs_spu2_voice_state() {
    let (Some((iop, ee)), Some(txt)) = (s07(), testdata::read_state("s07/voices.txt")) else {
        eprintln!("skip: no s07 state");
        return;
    };
    if disc_banks().is_empty() {
        eprintln!("skip: no disc image");
        return;
    }
    let vb = vb_of(&ee);
    let i = Inner::adopt_with(&iop, &ee, false, vb, &txt);
    assert!((0..48).all(|v| !i.drv.voices.spu[v].active()), "{:?}", i.notes);
    assert!((0..64).all(|s| !i.drv.core.slot(s).in_use()));
    let st = concat!(env!("CARGO_MANIFEST_DIR"), "/states/fg1/10_0504");
    let sv = std::env::var("ORACLE_SPUV_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../scratch_oracle_snd/spuv").into());
    let (Ok(iop), Ok(ee), Ok(spuv)) = (std::fs::read(format!("{st}/iopMemory.bin")), std::fs::read(format!("{st}/eeMemory.bin")), std::fs::read(format!("{sv}/fg1/10_0504/spu_voices.bin"))) else {
        eprintln!("skip: no fg1/10_0504 spu_voices.bin");
        return;
    };
    let j = Inner::adopt_with(&iop, &ee, false, vb_of(&ee), &spuv);
    let want = crate::snd989::rng::Rng250::from_iop(&iop, crate::snd989::adopt::live_base(&iop).expect("live module") as u32).unwrap();
    assert_eq!(j.drv.core.rng.state(), want.state());
}
