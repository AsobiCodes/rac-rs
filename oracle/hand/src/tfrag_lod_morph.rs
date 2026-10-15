//! TfragProc LOD thresholds and the morph table (retail 0x284720).

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Scale applied to the raw LOD thresholds before truncation (float 1024.0).
const SCALE: F = F(0x4480_0000);
/// Float 1.0.
const ONE: F = F(0x3f80_0000);
/// Float 0.5.
const HALF: F = F(0x3f00_0000);
/// Float -0.5.
const NEG_HALF: F = F(0xbf00_0000);
/// Three raw LOD threshold floats.
const THRESH: u32 = 0x160ee0;
/// Three truncated (int) LOD thresholds, written as words.
const THRESH_OUT: u32 = 0x160ef0;
/// Scale word read as lwc1 -0x32a0(v1) with v1 = 0x170000.
const SCALE_WORD: u32 = 0x16cd60;
/// Morph table (16 words, zeroed first).
const MORPH: u32 = 0x1c29f0;
/// Frame size of the retail prologue.
const FRAME: u32 = 0x90;

/// 0x284720 lod_thresholds_morph(): stores int(threshold * 1024) for the three LOD thresholds at 0x160ef0, then
/// fills the 16-word morph table at 0x1c29f0 from the thresholds' reciprocal spans (v0 = the last threshold int).
pub fn lod_thresholds_morph(c: &mut Cpu) -> bool {
    let saves = [
        (16, 0x00, h::SQ),
        (17, 0x10, h::SQ),
        (18, 0x20, h::SQ),
        (19, 0x30, h::SQ),
        (20, 0x40, h::SQ),
        (31, 0x50, h::SQ),
    ];
    h::frame(c, FRAME, &saves, |c, sp| {
        // retail's own spills of the callee-saved float registers
        for (k, reg) in (20..=24).enumerate() {
            c.w32(sp + 0x60 + 8 * k as u32, c.f[reg]);
        }
        fill(c);
    });
    true
}

fn fill(c: &mut Cpu) {
    let raw = [c.rf(THRESH), c.rf(THRESH + 4), c.rf(THRESH + 8)];
    let mut last = 0u32;
    for (i, x) in raw.iter().enumerate() {
        last = (*x * SCALE).to_int() as u32;
        c.w32(THRESH_OUT + 4 * i as u32, last);
    }
    c.ret(last);

    let k = c.rf(SCALE_WORD);
    let (p0, p1, p2) = (raw[0] * k, raw[1] * k, raw[2] * k);
    let inv_d = ONE / (p0 - p1);
    let inv_e = ONE / (p1 - p2);

    for i in 0..16u32 {
        c.w32(MORPH + 4 * i, 0);
    }
    c.wf(MORPH, inv_d * HALF);
    c.wf(MORPH + 0x4, -inv_d);
    c.wf(MORPH + 0xc, p0);
    c.wf(MORPH + 0x10, inv_e * HALF);
    c.wf(MORPH + 0x14, -inv_e);
    c.wf(MORPH + 0x20, (p1 * inv_d) * NEG_HALF);
    c.wf(MORPH + 0x1c, p1);
    c.wf(MORPH + 0x24, p0 * inv_d);
    c.wf(MORPH + 0x30, (p2 * inv_e) * NEG_HALF);
    c.wf(MORPH + 0x34, p1 * inv_e);

    // scratch registers as retail leaves them
    c.f[0] = ((p1 * inv_d) * NEG_HALF).0;
    c.f[1] = (inv_d * HALF).0;
    c.f[2] = (inv_e * HALF).0;
    c.f[3] = NEG_HALF.0;
    c.f[12] = last;
    c.s(1, 0x3f00_0000);
    c.s(3, 0x0017_0000);
    c.s(4, (MORPH + 0x40) as u64);
    c.s(5, 0);
}
