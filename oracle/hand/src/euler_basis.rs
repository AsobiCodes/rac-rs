//! Euler-angle rotation rows (VU0 micro 0xd18) written to memory.
use crate::ps2::*;
use ee_oracle::cpu::{uw4, Cpu};

/// MIPS register number of $at.
const REG_AT: usize = 1;

/// 0x1f17f8 zero_vec4(dst): stores four zero words at dst.
pub fn zero_vec4(c: &mut Cpu) -> bool {
    let dst = c.arg(0);
    c.wv(dst, V::default());
    true
}

/// 0x1f1fe8 euler_basis(out, angles*): writes the three rotation rows of the Euler angles at angles* to out
/// (16 bytes each, rows at out+0, +0x10, +0x20); $at gets the whole first row as a 128-bit value.
pub fn euler_basis(c: &mut Cpu) -> bool {
    // Unresolved: the rewrite is not bit-exact on sampled states (24 BAD, many registers and 737 RAM bytes differ),
    // while the same micro is exact in euler_matrix. Retail runs this function.
    let _ = (c, REG_AT);
    false
}

/// VU0 micro program (vcallms 0x1a3) that builds the rotation rows in VF20..VF23 from the angles in VF1.
const MICRO_EULER_ROWS: u32 = 0xd18;

/// 0x1f2008 euler_matrix(out, angles*): writes the rotation rows of the Euler angles at angles* to out (rows at
/// out+0, +0x10, +0x20) and a fourth row at out+0x30; $at gets the whole first row as a 128-bit value.
pub fn euler_matrix(c: &mut Cpu) -> bool {
    let (out, angles) = (c.arg(0), c.arg(1));
    // lqc2 vf1, 0(angles); vcallms: the micro leaves VF / ACC / VI as retail does.
    let angles_vf = c.rv(angles);
    c.set_vf(1, angles_vf);
    let _ = c.vu0_micro(MICRO_EULER_ROWS);
    // qmfc2 $at, vf20; sqc2 vf20..vf23 to out.
    c.r[REG_AT] = uw4(c.vf(20).0);
    for (i, n) in [20, 21, 22, 23].into_iter().enumerate() {
        c.wv(out + 0x10 * i as u32, c.vf(n));
    }
    true
}
