//! Camera per-frame tick (the 0x1df910 area): frame counter, mode hooks, follow-copy into the camera offset block,
//! shake, clip check, and the flat-zone colour blend.
use crate::h;
use crate::ps2::*;
use crate::{camera_frame, camera_pre, camobj, geom, spheres, vecmath};
use ee_oracle::cpu::Cpu;

/// Camera globals block (s0 in retail).
const CAM: u32 = 0x166d80;
/// Camera record pointers and mode (halfword at CAM + 0x270).
const CAM_RECORD: u32 = CAM + 0x180;
const CAM_RECORD_ALT: u32 = CAM + 0x184;
const CAM_MODE: u32 = CAM + 0x270;
/// Frame counter (u32 at CAM + 0x398).
const CAM_FRAME_COUNT: u32 = CAM + 0x398;
/// Game mode word; the mode reset below runs only in this mode.
const GAME_MODE: u32 = 0x15f5a4;
const GAME_MODE_RESET: u32 = 5;
/// Non-zero here skips the mode reset.
const RESET_SKIP_FLAG: u32 = 0x1ca580;
/// Gate block whose word at +0x14 must be zero for the follow copy and the matrix step.
const GATE_FLAG: u32 = 0x16c158 + 0x14;
/// Byte flag that enables the cross product at the end.
const CROSS_ENABLE: u32 = 0x15edb4;
/// Camera offset vector (CAM + 0x140) and shake records.
const CAM_OFFSET: u32 = 0x166ec0;
const SHAKE_MAIN: u32 = 0x166ee0;
const SHAKE_ALT: u32 = 0x166ef0;
/// Matrix (CAM + 0x350 .. + 0x380) filled from the record, then turned into Euler angles at EULER_OUT.
const MATRIX: u32 = 0x1670d0;
const EULER_OUT: u32 = 0x166ed0;
const CROSS_OUT: u32 = 0x1670e0;
const CROSS_IN: u32 = 0x1670f0;
/// Flat-zone colour blend outputs: three colour bytes, then four floats.
const BLEND_RGB: u32 = 0x15f444;
const BLEND_F_A: u32 = 0x15f448;
const BLEND_F_B: u32 = 0x15f44c;
const BLEND_F_C: u32 = 0x15f450;
const BLEND_F_D: u32 = 0x15f454;
/// Zone table (0x41 x 0x41 grid of pointers to entry lists) and its base pointer.
const GRID_PTR: u32 = 0x15ef80;
const GRID_SIZE: i32 = 0x41;
const GRID_ENTRY: u32 = 0x30;
/// Flat zones: 128-byte records at this base.
const ZONE_TABLE: u32 = 0x17efc0;
/// Debug/error routine called for an out-of-grid position, and its string argument.
const GRID_ERROR_FN: u32 = 0x1dd878;
const GRID_ERROR_MSG: u32 = 0x1d95a0;

/// Copies 16 bytes (one quadword) from `src` to `dst`.
fn copy16(c: &mut Cpu, dst: u32, src: u32) {
    let v = c.rv(src);
    c.wv(dst, v);
}

/// Position of the zone-blend parameter `t` mixed between two zone floats: zone[a] * t + zone[b] * (1 - t).
fn mix(c: &mut Cpu, zone: u32, a: u32, b: u32, t: F, u: F) -> F {
    c.rf(zone + a) * t + c.rf(zone + b) * u
}

/// 0x1df910 camera_update(): once per frame. Bumps the frame counter, runs the camera hooks, copies the current
/// record's transform into the camera block (when the gate is clear), applies the two shakes and the clip check,
/// and, when enabled, stores the cross product of the camera basis. Returns v0 as retail leaves it.
pub fn camera_update(c: &mut Cpu) -> bool {
    // Declined: camera_pre_update, mat4_to_euler, cross3 and zone_blend's flat_zone_hit leave VU register residue
    // that the Rust hands do not reproduce (fndiff vf BAD), so this whole function stays with retail.
    if true {
        return false;
    }
    h::frame(c, 0x70, &[(31, 0x60, h::SQ), (17, 0x50, h::SQ), (16, 0x40, h::SQ)], |c, sp| {
        if c.r32(GAME_MODE) == GAME_MODE_RESET {
            if c.r32(RESET_SKIP_FLAG) != 0 {
                c.ret(0x1d0000);
                return;
            }
            c.w16(CAM_MODE, 0);
            c.w8(CAM_MODE + 2, 0);
        }
        let frames = c.r32(CAM_FRAME_COUNT).wrapping_add(1);
        c.w32(CAM_FRAME_COUNT, frames);
        h::run(c, camera_frame::camera_boost_tick, &[], &[]);
        h::run(c, camera_frame::camera_flags_init, &[], &[]);
        h::run(c, camera_pre::camera_pre_update, &[], &[]);
        h::run(c, camobj::camera_select, &[], &[]);

        let record = c.r32(CAM_RECORD);
        if matches!(c.r16(CAM_MODE), 1 | 2) {
            let alt = c.r32(CAM_RECORD_ALT);
            h::call(c, 0x1de708, &[alt], &[]);
        }
        if c.r16(CAM_MODE) as i16 == 3 {
            h::call(c, 0x1df118, &[record], &[]);
        } else if c.r32(GATE_FLAG) == 0 {
            copy16(c, CAM_OFFSET, record + 0x30);
            copy16(c, CAM + 0x350, record);
            copy16(c, CAM + 0x360, record + 0x10);
            copy16(c, CAM + 0x370, record + 0x20);
        }
        if c.r32(GATE_FLAG) == 0 {
            h::run(c, vecmath::mat3_copy_w1, &[sp, MATRIX], &[]);
            h::run(c, geom::mat4_to_euler, &[sp, EULER_OUT], &[]);
        }

        h::run(c, camera_frame::camera_shake_step, &[SHAKE_MAIN, 0], &[]);
        h::run(c, camera_frame::camera_shake_step, &[SHAKE_ALT, 1], &[]);
        h::run(c, camera_frame::camera_clip_check, &[], &[]);
        h::run(c, zone_blend, &[CAM_OFFSET], &[]);

        if c.r8(CROSS_ENABLE) != 0 {
            let v0 = h::run(c, vecmath::cross3, &[CROSS_OUT, CROSS_IN, MATRIX], &[]).0;
            c.ret(v0);
        } else {
            c.ret(0);
        }
    });
    true
}

/// 0x1e0a18 grid_find_entry(pos, radius) -> entry: takes pos scaled by 1/16 as the grid cell (x from pos.x, y from
/// pos.y, both must be 0..=64, else the error routine runs and 0 is returned), walks that cell's entry list and
/// returns the first entry whose radius (entry +0xc) plus `radius` reaches pos and that passes 0x1e0b80. 0 if none.
pub fn grid_find_entry(c: &mut Cpu) -> bool {
    let (pos, radius) = (c.arg(0), c.farg(0));
    // Non-empty cell: the entry walk calls vecmath::length_xy, whose VU residue the Rust hand does not reproduce
    // (vf BAD), so retail runs those calls. Decided before any state is touched.
    {
        let cell = F(0x3d80_0000);
        let gx = (c.rf(pos) * cell).to_int();
        let gy = (c.rf(pos + 4) * cell).to_int();
        if (0..GRID_SIZE).contains(&gx) && (0..GRID_SIZE).contains(&gy) {
            let grid = c.r32(GRID_PTR);
            let slot = c.r32(grid.wrapping_add((gy as u32 * 64 + gx as u32) * 4));
            if slot != 0 && (c.r32(slot.wrapping_add(grid)) as i32) > 0 {
                return false;
            }
        }
    }
    let saves = [(31, 0x70, h::SQ), (20, 0x60, h::SQ), (19, 0x50, h::SQ), (18, 0x40, h::SQ), (17, 0x30, h::SQ),
        (16, 0x20, h::SQ)];
    h::frame(c, 0xa0, &saves, |c, sp| {
        c.wf(sp + 0x80, F(c.f[20]));
        c.wf(sp + 0x88, F(c.f[21]));
        c.wf(sp + 0x90, F(c.f[22]));
        let v = c.rv(pos);
        c.wv(sp, v);
        let cell = F(0x3d80_0000);
        let gx = (c.rf(sp) * cell).to_int();
        let gy = (c.rf(sp + 4) * cell).to_int();
        if !(0..GRID_SIZE).contains(&gx) || !(0..GRID_SIZE).contains(&gy) {
            h::call(c, GRID_ERROR_FN, &[GRID_ERROR_MSG], &[]);
            c.ret(0);
            return;
        }
        let grid = c.r32(GRID_PTR);
        let slot = c.r32(grid.wrapping_add((gy as u32 * 64 + gx as u32) * 4));
        if slot == 0 {
            c.ret(0);
            return;
        }
        let list = slot.wrapping_add(grid);
        if (c.r32(list) as i32) <= 0 {
            c.ret(0);
            return;
        }
        let mut entry = list + 0x10;
        let mut i = 0i32;
        loop {
            h::call(c, 0x1f1840, &[sp + 0x10, entry, sp], &[]);
            c.wf(sp + 0x1c, F(0x3f80_0000));
            let entry_radius = c.rf(entry + 0xc);
            let dist = h::run(c, vecmath::length_xy, &[sp + 0x10], &[]).1;
            if dist < entry_radius + radius {
                let hit = h::call(c, 0x1e0b80, &[sp, entry], &[radius]).0;
                if hit != 0 {
                    c.ret(entry);
                    return;
                }
            }
            i += 1;
            if i >= c.r32(list) as i32 {
                break;
            }
            entry += GRID_ENTRY;
        }
        c.ret(0);
    });
    true
}

/// 0x1e0f20 zone_blend(pos): spheres::flat_zone_hit finds the flat zone at pos (t = weight from the hit, zone index in
/// the second word). When that zone's flag word has bit 1, writes the blended colour bytes (0x15f444..) and the four
/// blend floats (0x15f448..0x15f454) from its two colours and float tables; returns int(t * 255), else 0.
pub fn zone_blend(c: &mut Cpu) -> bool {
    // Declined: spheres::flat_zone_hit (the only callee) leaves VU residue the Rust hand does not reproduce (vf BAD).
    if true {
        return false;
    }
    let pos = c.arg(0);
    h::frame(c, 0x40, &[(31, 0x20, h::SQ), (16, 0x10, h::SQ)], |c, sp| {
        c.wf(sp + 0x30, F(c.f[20]));
        let hit = h::run(c, spheres::flat_zone_hit, &[pos, sp, sp + 4], &[]).0;
        c.ret(hit);
        if hit == 0 {
            return;
        }
        let zone = ZONE_TABLE.wrapping_add(c.r32(sp + 4) << 7);
        if c.r32(zone + 0x50) & 2 == 0 {
            c.ret(0);
            return;
        }
        let t = c.rf(sp);
        let k255 = F(0x437f_0000);
        let k1024 = F(0x4480_0000);
        let u = F(0x3f80_0000) - t;
        let ti = (t * k255).to_int();
        let t1 = 255 - ti;

        let f2 = k255 - mix(c, zone, 0x78, 0x68, t, u) * k255;
        let f3 = mix(c, zone, 0x74, 0x64, t, u) * k1024;
        let f6 = mix(c, zone, 0x6c, 0x5c, t, u) * k1024;
        let f5 = mix(c, zone, 0x70, 0x60, t, u) * k255;

        let colour_a = c.r32(zone + 0x58);
        let colour_b = c.r32(zone + 0x54);
        for k in 0..3u32 {
            let ca = ((colour_a >> (8 * k)) & 0xff) as i32;
            let cb = ((colour_b >> (8 * k)) & 0xff) as i32;
            c.w8(BLEND_RGB + k, ((ca * ti + cb * t1) >> 8) as u8);
        }
        c.wf(BLEND_F_A, f6);
        c.wf(BLEND_F_B, f3);
        c.wf(BLEND_F_C, k255 - f5);
        c.wf(BLEND_F_D, f2);
        c.ret(ti as u32);
    });
    true
}
