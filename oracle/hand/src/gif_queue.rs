//! Queue-driven draw helpers (Eudora overlay): a callback-table dispatcher, the shadow-quad queue flush that builds a
//! GIF block on the stack, and the per-frame GIF emitter that appends a 0x50-byte block to the packet buffer.
use crate::h;
use crate::ps2::*;
use crate::vecmath::set_length;
use ee_oracle::cpu::Cpu;

/// Packet-buffer write pointer.
const GIF_PTR: u32 = 0x16_1140;
/// Draw callback entry count (the callback table itself is run by retail).
const CB_COUNT: u32 = 0x15_F428;
/// Shadow record count and the table of 4 template quads used by the flush.
const SHADOW_COUNT: u32 = 0x15_F434;
const SHADOW_TEMPLATE: u32 = 0x16_C8E0;
/// Shadow records, 32 bytes apart; each record's vector sits 16 bytes in.
const SHADOW_RECORDS: u32 = 0x16_E180;
/// Retail helpers reached by the flush and the emitter.
const SUB_BLOCK: u32 = 0x1e_bb00;
const EMIT_BLOCK: u32 = 0x1e_f7c0;
/// Emitter flags: the enable word, the counter, the gp-relative timer and the saved packet pointer.
const EMIT_ENABLE: u32 = 0x15_F404;
const EMIT_COUNTER: u32 = 0x15_F408;
const GP_TIMER: i64 = -0x77f8;
const GP_PTR_SAVE: i64 = -0x5ac0;
/// Constant quads copied into the packet buffer by the emitter, and the data table of attribute words.
const EMIT_HEADER_A: u32 = 0x16_0860;
const EMIT_HEADER_B: u32 = 0x16_0870;
const EMIT_TABLE: u32 = 0x13_E500;

/// IEEE single-precision 1.0.
const ONE: u32 = 0x3f80_0000;

/// Sign-extends a 32-bit word to the 64-bit register value.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Stores a 64-bit doubleword little-endian.
fn put64(c: &mut Cpu, addr: u32, v: u64) {
    c.w32(addr, v as u32);
    c.w32(addr + 4, (v >> 32) as u32);
}

fn gp(c: &Cpu, off: i64) -> u32 {
    (c.g(28) as i64).wrapping_add(off) as u32
}

/// 0x1ebe50 draw_queue_run(): calls each registered draw callback fn(arg) in registration order.
pub fn draw_queue_run(c: &mut Cpu) -> bool {
    let count = c.r32(CB_COUNT) as i32;
    if count > 0 {
        // The callback loop leaves callee-clobbered v1/a0/f-regs that h::call would restore, so retail runs it.
        return false;
    }
    c.s(2, sx(count as u32));
    true
}

/// 0x1ebec8 shadow_queue_flush(): builds the GIF header block on the stack, transforms each shadow record's ring
/// vector against the template quads, and passes the block to the emitter for every record.
pub fn shadow_queue_flush(c: &mut Cpu) -> bool {
    // Retail's first instruction is addiu sp,sp,-0x110: the frame is below the caller's sp.
    let sp = (c.g(29) as u32).wrapping_sub(0x110);
    let (v0, _) = h::call(c, SUB_BLOCK, &[0], &[]);
    put64(c, sp + 0x78, sx(v0));
    put64(c, sp + 0x80, (0xff90u64 << 32) | 0x260);
    put64(c, sp + 0x70, 5);
    put64(c, sp + 0x88, (0x8000u64 << 24) | 0x44);
    for k in 0..4u32 {
        c.w32(sp + 0x40 + 4 * k, 0x4080_8080);
        let f0 = c.rf(SHADOW_TEMPLATE + 16 * k + 8);
        let f1 = c.rf(SHADOW_TEMPLATE + 16 * k + 12);
        c.wf(sp + 0x50 + 8 * k, f0);
        c.wf(sp + 0x54 + 8 * k, f1);
    }
    let cnt = c.r32(SHADOW_COUNT) as i32;
    if cnt > 0 {
        let mut i = 0u32;
        loop {
            let ring = SHADOW_RECORDS + 32 * i;
            let d = set_length(c.rv(ring + 0x10), F(ONE));
            c.wv(sp + 0x90, d);
            let f8 = c.rf(ring + 0xc);
            for j in 0..4u32 {
                let t = SHADOW_TEMPLATE + 16 * j;
                let dst = sp + 16 * j;
                let dx = c.rf(sp + 0x90);
                let dy = c.rf(sp + 0x94);
                let mut f1 = c.rf(t);
                let mut f2 = c.rf(t + 4);
                let f4 = dx;
                let dot = f1 * f4 + f2 * dy;
                let q = c.rv(ring);
                c.wv(dst, q);
                let mut f3 = c.rf(dst);
                let mut f6 = c.rf(dst + 4);
                let mut f7 = c.rf(dst + 8);
                f1 = f1 - f4 * dot;
                f3 = f3 + f1 * f8;
                c.wf(dst, f3);
                f2 = f2 - dy * dot;
                f6 = f6 + f2 * f8;
                c.wf(dst + 4, f6);
                let dz = c.rf(sp + 0x98);
                f7 = f7 - dz * dot * f8;
                c.wf(dst + 8, f7);
            }
            h::call(c, EMIT_BLOCK, &[sp, 0, 0], &[]);
            i += 1;
            if (i as i32) >= c.r32(SHADOW_COUNT) as i32 {
                break;
            }
        }
    }
    // Retail's v0 is the loop's slt result (0) or the count itself when the loop is skipped.
    c.s(2, if cnt > 0 { 0 } else { sx(cnt as u32) });
    true
}

/// 0x1ec440 queue_emit(): advances the emit timer and, while the counter is non-zero, appends one 0x50-byte GIF block
/// to the packet buffer, scaled by the counter.
pub fn queue_emit(c: &mut Cpu) -> bool {
    if c.r32(EMIT_ENABLE) != 0 {
        let t = c.r32(gp(c, GP_TIMER)) as i32;
        // Retail computes t + 1 into v0 on this path whether or not the timer is stored.
        c.s(2, sx((t + 1) as u32));
        if t < 0x18 {
            c.w32(gp(c, GP_TIMER), (t + 1) as u32);
        }
    } else {
        let n = c.r32(EMIT_COUNTER);
        if n == 0 {
            c.s(2, u64::MAX);
            return true;
        }
        c.w32(EMIT_COUNTER, n - 1);
        c.s(2, sx(n - 1));
    }
    let n = c.r32(EMIT_COUNTER);
    if n == 0 {
        return true;
    }
    emit_block(c, n << 4);
    true
}

fn emit_block(c: &mut Cpu, t1: u32) {
    let save = gp(c, GP_PTR_SAVE);
    c.w32(save, 0x1000_0007);
    let g = c.r32(GIF_PTR);
    c.w32(g + 4, 0);
    c.w32(g + 8, 0);
    c.w32(g + 0xc, 0x5000_0007);
    let p = c.r32(GIF_PTR) + 0x10;
    c.w32(GIF_PTR, p);
    let header = c.rv(EMIT_HEADER_A);
    c.wv(p, header);
    c.w16(p, 0x8001);

    let r = c.r32(GIF_PTR);
    let s = r + 0x10;
    c.w32(GIF_PTR, s);
    put64(c, s, 0x104);
    put64(c, s + 8, 0x8000u64 << 16);

    let q = c.r32(GIF_PTR);
    let p2 = q + 0x10;
    c.w32(GIF_PTR, p2);
    let header_b = c.rv(EMIT_HEADER_B);
    c.wv(p2, header_b);
    c.w16(p2, 0x8008);

    let a3 = ((0xffffu64 << 16) | 0xf300) << 24;
    let rec = c.r32(GIF_PTR);
    let a2 = rec + 0x10;
    c.w32(GIF_PTR, a2);
    let pack = |lo: u32, hi: u32| sx(lo) | (sx(hi) << 16) | a3;
    let hi_t1 = |hi: u32| hi.wrapping_add(t1);
    let hi_m1 = |hi: u32| hi.wrapping_sub(t1);
    let (w10, w14, w18, w1c) = (c.r32(EMIT_TABLE + 0x10), c.r32(EMIT_TABLE + 0x14), c.r32(EMIT_TABLE + 0x18), c.r32(EMIT_TABLE + 0x1c));
    put64(c, rec + 0x10, pack(w10, w14));
    put64(c, rec + 0x18, pack(w10, hi_t1(w14)));
    put64(c, rec + 0x20, pack(w18, w14));
    put64(c, rec + 0x28, pack(w18, hi_t1(w14)));
    put64(c, rec + 0x30, pack(w18, w1c));
    put64(c, rec + 0x38, pack(w18, hi_m1(w1c)));
    put64(c, rec + 0x40, pack(w10, w1c));
    put64(c, rec + 0x48, pack(w10, hi_m1(w1c)));
    c.w32(GIF_PTR, a2 + 0x40);
    // Retail's v0 at the end is the new packet pointer.
    c.s(2, sx(a2 + 0x40));
}
