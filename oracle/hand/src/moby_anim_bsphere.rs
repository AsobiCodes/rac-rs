//! Moby animation setters and the stored-rows entry of the moby bounding-sphere refresh.
use crate::actor::anim_load_tracks;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail part of the bounding sphere refresh, entered by the stored-rows jump.
const REFRESH_TAIL: u32 = 0x2436f8;
/// Animation pointer table in the class record (+0x24), one word per sequence from +0x48.
const CLASS_ANIMS: u32 = 0x48;
/// Frame rate (float) of the moby.
const FRAME_RATE: u32 = 0x5c;
const ANIM_FRAMES: u32 = 0x10;
const SEQ: u32 = 0x52;
const SEQ_NEXT: u32 = 0x53;
const FRAME_LO: u32 = 0x50;
const FRAME_HI: u32 = 0x51;
const TRACKS_PTR: u32 = 0x68;
const FLAGS_70: u32 = 0x70;
const FLAG_70_BIT2: u8 = 0x02;

/// 0x24a090 set_moby_animation(moby, seq, frame): stores the sequence (+0x52 and +0x53) and a frame clamped to the
/// sequence's frame count (+0x50, +0x51, the last frame is kept when the frame overruns), loads the tracks of the
/// moby through anim_load_tracks, takes the frame rate from the track pointer's first float into +0x5c and clears
/// bit 2 of +0x70. Exit: v0 = the track pointer, v1 = the new +0x70 byte.
pub fn set_moby_animation(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let seq = c.arg(1);
    let frame = c.arg(2) as i32;
    let class = c.r32(m + 0x24);
    let slot = class + CLASS_ANIMS + 4 * seq;
    let anim = c.r32(slot);
    let count = c.r8(anim + ANIM_FRAMES) as i32;
    c.w8(m + SEQ, seq as u8);
    // The frame is clamped to the last frame index: min(frame, count - 1) as a signed compare.
    let clamped = if frame < count { frame } else { count - 1 };
    c.w8(m + FRAME_LO, clamped as u8);
    c.w8(m + FRAME_HI, (clamped + 1) as u8);
    // A frame index past the sequence's count pulls the stored frame back to count - 1.
    if count - 1 < ((clamped + 1) & 0xff) {
        c.w8(m + FRAME_HI, (count - 1) as u8);
    }
    c.w8(m + SEQ_NEXT, seq as u8);
    // The stored frame must be below the new sequence's count, else it restarts at 0.
    if (c.r8(m + FRAME_HI) as i32) >= count {
        c.w8(m + FRAME_HI, 0);
    }
    crate::h::run_keep(c, anim_load_tracks, &[m], &[]);
    let tracks = c.r32(m + TRACKS_PTR);
    let flags = c.r8(m + FLAGS_70) & !FLAG_70_BIT2;
    c.w8(m + FLAGS_70, flags);
    let rate = c.r32(tracks);
    c.f[0] = rate;
    c.w32(m + FRAME_RATE, rate);
    c.ret(tracks);
    c.s(3, flags as u64);
    true
}

/// 0x243860 bounding sphere refresh entry: the rotation rows are already stored at +0xc0..+0xe0, so they are loaded
/// into vf20-vf22 (vf23 = vf0) and the retail refresh from 0x2436f8 runs with the registers it expects (v0 = class,
/// v1 = scale bits, t9 = moby, a0/a1 = frames, a3 = flag 0x8000). Nothing is written when +0x20 is negative.
pub fn bsphere_stored_rows(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    if (c.r8(m + 0x20) as i8) < 0 {
        return true;
    }
    let flags = c.r16(m + 0x34);
    let class = c.r32(m + 0x24);
    let frame = c.r8(m + SEQ) as u32;
    let next = c.r8(m + SEQ_NEXT) as u32;
    let scale = c.r32(m + 0x2c);
    for i in 0..3 {
        let row = c.rv(m + 0xc0 + 0x10 * i as u32);
        c.set_vf(20 + i, row);
    }
    c.set_vf(23, c.vf(0));
    c.s(2, class as i32 as i64 as u64);
    c.s(3, scale as i32 as i64 as u64);
    c.s(25, m as i32 as i64 as u64);
    crate::h::call_keep(c, REFRESH_TAIL, &[frame, next, c.arg(2), (flags & 0x8000) as u32], &[]);
    true
}
