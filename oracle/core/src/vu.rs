//! VU0 upper ops in macro mode (moved verbatim from ee-oracle vu.rs, where the micro mode stays).

use crate::cpu::{Cpu, BC_KIND, FULL, MAX, MIN, MADD, MSUB, QI};
use crate::fpu;

pub const ACC: usize = 32;
pub const NONE: usize = 99;

pub struct Up {
    pub dst: usize,
    pub val: [u32; 4],
    pub mask: u32,
    pub mac: Option<u32>,
    pub clip: Option<u32>,
    pub reads: [(usize, u32); 3],
}

/// Lane-ordered 4-bit mask (bit l = lane l) <-> the MAC / dest layout (lane l at bit 3 - l).
const REV4: [u32; 16] = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];

/// MAC flags of a result under `dest`: zero (|v| bits 0) at bit 3 - l, sign at bit 7 - l. [PC] (speed) on whole vectors (the
/// per-lane loop, `mac_of_ref`, was a measurable part of every recompiled COP2 macro op).
#[inline(always)]
pub fn mac_of(v: [u32; 4], dest: u32) -> u32 {
    use std::arch::x86_64::*;
    unsafe {
        let x = _mm_loadu_si128(v.as_ptr() as *const __m128i);
        let z = _mm_movemask_ps(_mm_castsi128_ps(_mm_cmpeq_epi32(_mm_and_si128(x, _mm_set1_epi32(0x7fff_ffff)), _mm_setzero_si128()))) as u32;
        let s = _mm_movemask_ps(_mm_castsi128_ps(x)) as u32;
        let lm = REV4[(dest & 15) as usize];
        REV4[(z & lm) as usize] | REV4[(s & lm) as usize] << 4
    }
}

/// The status bits 0-1 a result sets (zero, sign over the dest lanes), for the sticky bits 6-7 alone.
#[inline(always)]
pub fn sticky_of(v: [u32; 4], dest: u32) -> u32 {
    use std::arch::x86_64::*;
    unsafe {
        let x = _mm_loadu_si128(v.as_ptr() as *const __m128i);
        let z = _mm_movemask_ps(_mm_castsi128_ps(_mm_cmpeq_epi32(_mm_and_si128(x, _mm_set1_epi32(0x7fff_ffff)), _mm_setzero_si128()))) as u32;
        let s = _mm_movemask_ps(_mm_castsi128_ps(x)) as u32;
        let lm = REV4[(dest & 15) as usize];
        ((z & lm != 0) as u32) | (((s & lm != 0) as u32) << 1)
    }
}

#[cfg(test)]
pub fn mac_of_ref(v: [u32; 4], dest: u32) -> u32 {
    let mut m = 0;
    for l in 0..4 {
        if dest & (8 >> l) != 0 {
            let b = 3 - l;
            if v[l] & 0x7fff_ffff == 0 {
                m |= 1 << b;
            }
            if v[l] >> 31 != 0 {
                m |= 1 << (b + 4);
            }
        }
    }
    m
}
pub fn lane(l: usize) -> u32 {
    8 >> l
}

impl Cpu {
    /// Compute an upper instruction against the current state, no writes. Inlined: in recompiled code `op` is a
    /// constant and the decode folds away.
    #[inline(always)]
    pub fn vu_upper(&self, op: u32, clip_prev: u32) -> Result<Up, String> {
        self.vu_upper_g::<false>(op, clip_prev)
    }

    /// `vu_upper`; FAST = true skips the MXCSR switching of every float op (a `fpu::ChopGuard` must be alive).
    #[inline(always)]
    pub fn vu_upper_g<const FAST: bool>(&self, op: u32, clip_prev: u32) -> Result<Up, String> {
        let dest = (op >> 21) & 15;
        let ft = ((op >> 16) & 31) as usize;
        let fs = ((op >> 11) & 31) as usize;
        let fd = ((op >> 6) & 31) as usize;
        let s = self.vf[fs];
        let t = self.vf[ft];
        let bc = (op & 3) as usize;
        let (q, i) = (self.vi[22], self.vi[21]);
        let mut u = Up { dst: NONE, val: [0; 4], mask: dest, mac: None, clip: None, reads: [(NONE, 0); 3] };
        let f = (op & 0x3f) as usize;
        let acc_read = |k: u8| if k == MADD || k == MSUB { (ACC, dest) } else { (NONE, 0) };
        if f < 0x3c {
            let (k, tv, tr) = match f {
                0..=27 => (BC_KIND[f >> 2], [t[bc]; 4], (ft, lane(bc))),
                28..=39 => {
                    let (k, isi) = QI[f - 28];
                    (k, [if isi { i } else { q }; 4], (NONE, 0))
                }
                46 => {
                    let p = fpu::vmul_g::<FAST>([s[1], s[2], s[0], 0], [t[2], t[0], t[1], 0]);
                    u.val = fpu::vsub_g::<FAST>(self.vacc, p);
                    u.mask = dest & 0xe;
                    u.dst = fd;
                    u.mac = Some(mac_of(u.val, u.mask));
                    u.reads = [(fs, 0xe), (ft, 0xe), (ACC, 0xe)];
                    return Ok(u);
                }
                40..=47 => (FULL[f - 40], t, (ft, dest)),
                _ => return Err(format!("unknown VU upper {op:08x}")),
            };
            u.val = self.fmac_g::<FAST>(k, s, tv);
            u.dst = fd;
            u.reads = [(fs, dest), tr, acc_read(k)];
            if k != MAX && k != MIN {
                u.mac = Some(mac_of(u.val, dest));
            }
            return Ok(u);
        }
        let x = ((op & 3) | ((op >> 4) & 0x7c)) as usize;
        match x {
            0..=15 | 24..=27 => {
                let k = BC_KIND[x >> 2];
                u.val = self.fmac_g::<FAST>(k, s, [t[bc]; 4]);
                u.dst = ACC;
                u.reads = [(fs, dest), (ft, lane(bc)), acc_read(k)];
                u.mac = Some(mac_of(u.val, dest));
            }
            16..=19 => {
                let sc = [1.0f32, 0.0625, 1.0 / 4096.0, 1.0 / 32768.0][x - 16];
                if FAST {
                    for l in 0..4 {
                        u.val[l] = fpu::fout(fpu::mulss_nc(fpu::cvtsi2ss_nc(s[l] as i32), sc));
                    }
                } else {
                    u.val = fpu::vitof(s, sc);
                }
                u.dst = ft;
                u.reads[0] = (fs, dest);
            }
            20..=23 => {
                let sc = [1.0f32, 16.0, 4096.0, 32768.0][x - 20];
                if FAST {
                    for l in 0..4 {
                        u.val[l] = fpu::cvt_w(fpu::fout(fpu::mulss_nc(fpu::fin(s[l]), sc)));
                    }
                } else {
                    u.val = fpu::vftoi(s, sc);
                }
                u.dst = ft;
                u.reads[0] = (fs, dest);
            }
            29 => {
                u.val = [s[0] & 0x7fff_ffff, s[1] & 0x7fff_ffff, s[2] & 0x7fff_ffff, s[3] & 0x7fff_ffff];
                u.dst = ft;
                u.reads[0] = (fs, dest);
            }
            31 => {
                let w = fpu::fz(t[3]).abs();
                let mut c = (clip_prev << 6) & 0xff_ffff;
                for l in 0..3 {
                    let v = fpu::fz(s[l]);
                    if v > w {
                        c |= 1 << (2 * l);
                    }
                    if v < -w {
                        c |= 2 << (2 * l);
                    }
                }
                u.clip = Some(c);
                u.reads = [(fs, 0xe), (ft, 1), (NONE, 0)];
            }
            28 | 30 | 32..=39 => {
                let (k, isi) = QI[x - 28];
                u.val = self.fmac_g::<FAST>(k, s, [if isi { i } else { q }; 4]);
                u.dst = ACC;
                u.reads = [(fs, dest), (NONE, 0), acc_read(k)];
                u.mac = Some(mac_of(u.val, dest));
            }
            40..=42 | 44 | 45 => {
                let k = FULL[x - 40];
                u.val = self.fmac_g::<FAST>(k, s, t);
                u.dst = ACC;
                u.reads = [(fs, dest), (ft, dest), acc_read(k)];
                u.mac = Some(mac_of(u.val, dest));
            }
            46 => {
                u.val = fpu::vmul_g::<FAST>([s[1], s[2], s[0], 0], [t[2], t[0], t[1], 0]);
                u.mask = dest & 0xe;
                u.dst = ACC;
                u.reads = [(fs, 0xe), (ft, 0xe), (NONE, 0)];
                u.mac = Some(mac_of(u.val, u.mask));
            }
            47 => {}
            _ => return Err(format!("unknown VU upper {op:08x}")),
        }
        Ok(u)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpu::{cvt_w, cvtsi2ss, fin, fout, mulss, vftoi, vitof, CSR_CHOP};

    fn rnd(s: &mut u64) -> u32 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        (*s >> 16) as u32
    }
    /// Random words biased to exponent edges, denormals, inf / NaN, integers near 2^24 and the i32 limits.
    fn word(s: &mut u64) -> u32 {
        let r = rnd(s);
        match rnd(s) % 8 {
            0 => r,
            1 => (r & 0x807f_ffff) | [0, 0x7f80_0000, 0x0080_0000, 0x7f00_0000, 0x4e80_0000, 0x4f00_0000, 0x3f80_0000, 0x4b80_0000][(r % 8) as usize],
            2 => ((1u32 << (r % 32)) as i32).wrapping_add((rnd(s) % 512) as i32 - 256) as u32,
            3 => r & 0x8000_007f,
            4 => [0, 1, 0x7fff_ffff, 0x8000_0000, 0x8000_0001, 0x00ff_ffff, 0x0100_0001, 0xff00_0001, 0xffff_ffff][(r % 9) as usize],
            5 => r % 0x2000_0000,
            6 => (r & 0x80ff_ffff) | 0x4e00_0000,
            _ => r | 0x7f80_0000,
        }
    }

    #[test]
    fn itof_ftoi_mac_match_mxcsr_reference() {
        let mut s = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..1_000_000 {
            let v = [word(&mut s), word(&mut s), word(&mut s), word(&mut s)];
            for (k, &sc) in [1.0f32, 0.0625, 1.0 / 4096.0, 1.0 / 32768.0].iter().enumerate() {
                let r: [u32; 4] = std::array::from_fn(|l| fout(mulss(cvtsi2ss(v[l] as i32, CSR_CHOP), sc, CSR_CHOP)));
                assert_eq!(vitof(v, sc), r, "ITOF{k} {v:08x?}");
            }
            for (k, &sc) in [1.0f32, 16.0, 4096.0, 32768.0].iter().enumerate() {
                let r: [u32; 4] = std::array::from_fn(|l| cvt_w(fout(mulss(fin(v[l]), sc, CSR_CHOP))));
                assert_eq!(vftoi(v, sc), r, "FTOI{k} {v:08x?}");
            }
            let d = rnd(&mut s) & 15;
            assert_eq!(mac_of(v, d), mac_of_ref(v, d), "mac {v:08x?} dest {d:x}");
        }
    }
}
