//! Hero front-probe helpers: the two spring-gain stores and the forward-reach segment probe.
use crate::h;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Base address of the hero-block gain table (lui 0x18 + addiu -0x5880).
const GAIN_BASE: u32 = 0x0017_A780;
/// Hero state block (s2 in the probe): holds the feet radius at +0x234 and the matrix at +0x80.
const HERO_BLOCK: u32 = 0x0013_F350;
const HERO_FEET_RADIUS: u32 = 0x234;
const HERO_OFFSET: u32 = 0x80;
/// Reference point for the horizontal distance (lui 0x17 + addiu 0x3f60).
const DIST_ORIGIN: u32 = 0x0017_3F60;
/// Retail polar-to-world helper (hero_space::polar_to_world); called as retail so its scratch leftovers match.
const POLAR_TO_WORLD: u32 = 0x0022_6708;
/// Retail segment-hit probe: (segment start, segment end, mask, ...).
const SEGMENT_PROBE: u32 = 0x001e_24d8;
/// Radius inset (0.02) subtracted from the feet radius for the probe start.
const FEET_INSET: F = F(0x3ca3_d70a);

/// 0x1fd2b8 hero_block2_gain(k, c): stores the block 2 spring constants (f12 at +0x204, f13 at +0x208).
pub fn hero_block2_gain(c: &mut Cpu) -> bool {
    let (k, damp) = (c.farg(0), c.farg(1));
    c.wf(GAIN_BASE + 0x204, k);
    c.wf(GAIN_BASE + 0x208, damp);
    c.ret(GAIN_BASE);
    true
}

/// 0x1fd2d0 hero_block3_gain(k, c): stores the block 3 spring constants (f12 at +0x2b4, f13 at +0x2b8).
pub fn hero_block3_gain(c: &mut Cpu) -> bool {
    let (k, damp) = (c.farg(0), c.farg(1));
    c.wf(GAIN_BASE + 0x2b4, k);
    c.wf(GAIN_BASE + 0x2b8, damp);
    c.ret(GAIN_BASE);
    true
}

/// 0x1fd2e8 hero_front_probe(out_dist*, reach, z): probes the segment from the feet to `reach` at height z; on a hit
/// writes the horizontal distance to the hit point to *out (when out is non-null). Returns 1 on hit, else 0.
pub fn hero_front_probe(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let reach = c.farg(0);
    let z = c.farg(1);
    // Retail frame: the start and end points live in the frame itself (a0 = sp, a1 = sp + 0x10).
    let saves = [(16, 0x20, h::SQ), (17, 0x30, h::SQ), (18, 0x40, h::SQ), (31, 0x50, h::SQ)];
    let (hit, f0) = h::frame(c, 0x80, &saves, |c, sp| {
        let start = sp;
        let end = sp + 0x10;
        // swc1 f20/f21/f22 into the frame, as the retail prologue does.
        for (reg, off) in [(20, 0x60), (21, 0x68), (22, 0x70)] {
            c.w32(sp + off, c.f[reg]);
        }
        let start_r = c.rf(HERO_BLOCK + HERO_FEET_RADIUS) - FEET_INSET;
        h::call_keep(c, POLAR_TO_WORLD, &[start], &[start_r, F(0), reach]);
        h::call_keep(c, POLAR_TO_WORLD, &[end], &[z, F(0), reach]);
        let (hit, probe_f0) = h::call_keep(c, SEGMENT_PROBE, &[start, end, 2, 0, 0], &[]);
        if hit == 0 {
            return (0, probe_f0);
        }
        if out != 0 {
            let dist = h::run_keep(c, vecmath::dist_xy, &[HERO_BLOCK + HERO_OFFSET, DIST_ORIGIN], &[]).1;
            c.wf(out, dist);
            return (1, dist);
        }
        (1, probe_f0)
    });
    c.ret(hit);
    c.fret(f0);
    true
}
