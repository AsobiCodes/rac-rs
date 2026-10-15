//! HUD ammo-suppress counter (gp-0x72c0) and the HUD packet emitter at 0x22f8f8.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Ammo-HUD suppress counter, a gp-relative word.
const SUPPRESS_OFF: i32 = -0x72c0;
/// Packet buffer pointer (lui 0x16 + 0x1140).
const PACKET_PTR: u32 = 0x0016_1140;
/// Packet write pointer, a gp-relative word.
const PACKET_CUR_OFF: i32 = -0x5ac0;
/// Retail clears the HUD flag words at these addresses (lui 0x16 + -0x6b8 / -0x6bc).
const HUD_FLAG_A: u32 = 0x0015_f948;
const HUD_FLAG_B: u32 = 0x0015_f944;
/// Retail's packet tag words.
const TAG_HEAD: u32 = 0x1000_0006;
const TAG_END_A: u32 = 0x5000_0006;
const TAG_CTRL: u32 = 0x3000_0000;
const TAG_END_B: u32 = 0x5000_0000;
/// Callees: HUD slot clear, a packet helper, and the draw/emit helper.
const HUD_CLEAR_SLOTS: u32 = 0x0022_89b8;
const PACKET_SETUP: u32 = 0x0012_2330;
const PACKET_FLUSH: u32 = 0x0011_8a80;
const PACKET_EMIT: u32 = 0x0012_2658;

/// 0x22e040 hud_reset_suppress(): clears every HUD slot and sets the ammo-HUD suppress counter to 1.
pub fn hud_reset_suppress(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x10, &[(31, 0, h::SQ)], |c, _sp| {
        h::call_keep(c, HUD_CLEAR_SLOTS, &[], &[]);
        let gp = c.g(28) as u32;
        c.w32(gp.wrapping_add(SUPPRESS_OFF as u32), 1);
        c.w32(HUD_FLAG_A, u32::MAX);
        c.w32(HUD_FLAG_B, 0);
        c.ret(1);
        c.s(3, u64::MAX);
        c.s(1, 0x0016_0000);
    });
    true
}

/// 0x22e078 hud_unsuppress(): decrements the ammo-HUD suppress counter if nonzero; returns the new value.
pub fn hud_unsuppress(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    let addr = gp.wrapping_add(SUPPRESS_OFF as u32);
    let v = c.r32(addr);
    let next = v.wrapping_sub(1);
    if v != 0 {
        c.w32(addr, next);
    }
    // The decrement sits in the beqz delay slot (0x22e080), so retail returns v-1 even when v == 0.
    c.ret(next);
    true
}

/// Sign-extends the low 16 bits of `v` (sll 16 / sra 16).
fn sext16(v: u32) -> u32 {
    v as u16 as i16 as i32 as u32
}

/// 0x22f8f8 hud_packet_emit(s3 = a0, a1, a2, a3, t0 = shift, t1 = direct flag): builds a HUD packet in the packet
/// buffer (direct: at the packet pointer, else on the stack), calls the setup helper, then flushes or emits it.
pub fn hud_packet_emit(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let (x, a1, a2, a3, t0, t1) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3), c.arg(4), c.arg(5));
    let saves = [(16, 0x60, h::SQ), (17, 0x70, h::SQ), (18, 0x80, h::SQ), (19, 0x90, h::SQ), (31, 0xa0, h::SQ)];
    h::frame(c, 0xb0, &saves, |c, sp| {
        let direct = t1 == 0;
        let t5 = a3;
        let v1 = 1u32.wrapping_shl(t5) as i32;
        let t4 = match v1 >> 6 {
            n if n > 0 => n as u32,
            _ => 1,
        };
        let s2 = 1u32.wrapping_shl(t5.wrapping_add(t0).wrapping_sub(4));
        let gp = c.g(28) as u32;
        let cur_addr = gp.wrapping_add(PACKET_CUR_OFF as u32);
        let s0 = if direct {
            let p = c.r32(PACKET_PTR);
            c.w32(p, TAG_HEAD);
            c.w32(p.wrapping_add(4), 0);
            c.w32(p.wrapping_add(8), 0);
            c.w32(p.wrapping_add(0xc), TAG_END_A);
            c.w32(cur_addr, p.wrapping_add(0x70));
            p.wrapping_add(0x10)
        } else {
            sp
        };
        let args = [
            s0,
            sext16(a1),
            sext16(t4),
            sext16(a2),
            0,
            0,
            sext16(1u32.wrapping_shl(t5)),
            sext16(1u32.wrapping_shl(t0)),
        ];
        h::call_keep(c, PACKET_SETUP, &args, &[]);
        if direct {
            let cur = c.r32(cur_addr);
            c.w32(cur, TAG_CTRL | s2);
            let p = c.r32(PACKET_PTR);
            c.w32(p.wrapping_add(4), x);
            c.w32(p.wrapping_add(8), 0);
            c.w32(p.wrapping_add(0xc), s2 | TAG_END_B);
            c.w32(cur_addr, p.wrapping_add(0x10));
            c.ret(p.wrapping_add(0x10));
            // Retail's last loads leave v1 = a0 = packet pointer and a1 = the control word.
            c.s(3, p as u64);
            c.s(4, p as u64);
            c.s(5, (s2 | TAG_END_B) as u64);
        } else {
            h::call_keep(c, PACKET_FLUSH, &[0], &[]);
            h::call_keep(c, PACKET_EMIT, &[s0, x], &[]);
        }
    });
    true
}
