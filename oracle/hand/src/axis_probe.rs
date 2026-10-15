//! Hand versions of two level-4 EE helpers: an axis-index packer and a probe-scaled vector writer.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Float 1.0 (0x3f800000).
const ONE: F = F(0x3f80_0000);
/// Float 127.0 (0x42fe0000), the bias added to each packed lane.
const BIAS: F = F(0x42fe_0000);
/// Float 1e-4 (0x38d1b717), the per-index scale step.
const STEP: F = F(0x38d1_b717);
/// Float 10000.0 (0x461c4000).
const SCALE_NUM: F = F(0x461c_4000);
/// Float 63.0 (0x427c0000).
const SCALE_DEN: F = F(0x427c_0000);
/// Stack frame size of both retail functions (`addiu sp,sp,-0x50`).
const FRAME: u32 = 0x50;
/// Largest index the packer keeps (`ori 0xff`).
const INDEX_MAX: i32 = 0xff;
/// Retail vector-op helper at 0x1f1840 (no hand version yet).
const VEC_OP: u32 = 0x1f_1840;

/// Sets general register `n` to `v` sign-extended, as a 32-bit MIPS op leaves it.
fn set_gpr(c: &mut Cpu, n: usize, v: i32) {
    c.s(n, v as i64 as u64);
}

/// `|x|` through the motion::fabs hand, keeping the callee's scratch registers as retail's `jal` does.
fn abs(c: &mut Cpu, x: F) -> F {
    h::run_keep(c, crate::motion::fabs, &[], &[x]).1
}

/// 0x253d28 axis_index_store(v, out) -> void: index = clamp(int(max(|x|,|y|,|z|) * 10000 / 63), 1, 255);
/// *out = the low bytes of (x, y, z, index) after x,y,z -> 127 + v / (index * 1e-4).
pub fn axis_index_store(c: &mut Cpu) -> bool {
    let (v, out) = (c.arg(0), c.arg(1));
    let (x, y, z) = (c.rf(v), c.rf(v + 4), c.rf(v + 8));
    let (f20, f21) = (F(c.f[20]), F(c.f[21]));
    // Retail compares keep the operand order of the original code (ties and NaN included).
    let ax = abs(c, x);
    let mut m = abs(c, y);
    if m < ax {
        m = ax;
    }
    let az = abs(c, z);
    let big = if az < m { m } else { az };
    let q = big * SCALE_NUM / SCALE_DEN;
    let raw = h::run_keep(c, crate::mathx::float_to_int, &[], &[q]).0;
    // The retail converter leaves its integer result in f12 (cvt.w.s in place).
    c.f[12] = raw;
    let r = raw as i32;
    let r = if r < 256 { r } else { INDEX_MAX };
    let index = if r > 0 { r } else { 1 };
    let k = F::from_int(index);
    let scale = k * STEP;
    let inv = ONE / scale;
    let (x2, y2, z2) = (x * inv + BIAS, y * inv + BIAS, z * inv + BIAS);
    h::frame(c, FRAME, &[(16, 0x10, h::SQ), (17, 0x20, h::SQ), (31, 0x30, h::SQ)], |c, sp| {
        c.wf(sp + 0x40, f20);
        c.wf(sp + 0x48, f21);
        c.wf(sp, x2);
        c.wf(sp + 4, y2);
        c.wf(sp + 8, z2);
        c.wf(sp + 0xc, k);
        set_gpr(c, 5, 1);
        set_gpr(c, 3, index);
        set_gpr(c, 1, 0x42fe_0000);
        c.f[0] = inv.0;
        c.f[1] = BIAS.0;
        c.f[2] = y2.0;
        c.f[3] = z2.0;
        c.f[4] = x2.0;
        c.f[5] = scale.0;
        c.f[6] = k.0;
        let packed = h::run_keep(c, crate::vu_blend_pack::pack_vec4_bytes, &[sp], &[]).0;
        c.w32(out, packed);
    });
    true
}

/// 0x253e50 probe_scale_vec(out, key) -> void: packs (127, 127, 127, 0) with *key through
/// vu_probes::pack_bytes_f32, then out = vec_scale(out, buf3 * 1e-4) of the vector 0x1f1840 makes from it.
pub fn probe_scale_vec(c: &mut Cpu) -> bool {
    let (out, key) = (c.arg(0), c.arg(1));
    let key_val = c.r32(key);
    let f20 = F(c.f[20]);
    h::frame(c, FRAME, &[(31, 0x30, h::SQ), (16, 0x20, h::SQ)], |c, sp| {
        c.mem.wr128(sp + 0x10, 0);
        c.wf(sp + 0x10, BIAS);
        c.wf(sp + 0x14, BIAS);
        c.wf(sp + 0x18, BIAS);
        c.wf(sp + 0x40, f20);
        c.f[0] = BIAS.0;
        h::run_keep(c, crate::vu_probes::pack_bytes_f32, &[sp, key_val], &[]);
        let s = c.rf(sp + 0xc) * STEP;
        c.f[0] = STEP.0;
        set_gpr(c, 1, 0x38d1_b717);
        h::call_keep(c, VEC_OP, &[sp, sp, sp + 0x10], &[]);
        h::run_keep(c, crate::vecmath::vec_scale, &[out, sp], &[s]);
    });
    true
}
