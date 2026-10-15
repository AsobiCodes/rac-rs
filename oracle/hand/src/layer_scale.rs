//! Layer-scale update (0x227c78) and the HUD slot request tail call (0x227f40).
use crate::actor;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Address of the word at gp - GP_SCALE_OFF holds the layer scale (a float).
const GP_SCALE_OFF: u32 = 0x7dec;
/// gp - MODE_GP_OFF holds the mode byte the layer update reads before attaching.
const MODE_GP_OFF: u32 = 0x7e7c;
/// Scale cap used in mode 6 (0.66).
const SCALE_CAP: F = F(0x3faa_3d71);
/// Mode word (lui 0x16, offset -0xa5c): checked for the cap.
const CAP_MODE_ADDR: u32 = 0x0016_0000 - 0x0a5c;
/// Mode word (lui 0x16, offset -0x127c): read again after the attach.
const MODE_ADDR: u32 = 0x0016_0000 - 0x127c;
/// The layer record (lui 0x18, offset -0x3bc0): flag byte at +1, scale fields at +0x20/+0x24/+0x28.
const LAYER_REC: u32 = 0x0018_0000 - 0x3bc0;
/// Layer attach kind and size arguments.
const LAYER_KIND: u32 = 0x18;
/// Object list head (lui 0x17, offset -0x36a0): count halfword at +0x44, pointers from +0x178.
const LIST_HEAD: u32 = 0x0017_0000 - 0x36a0;
const LIST_COUNT_OFF: u32 = 0x44;
const LIST_PTRS_OFF: u32 = 0x178;
/// Object kinds (halfword at +0xa6) that get the scale.
const SCALED_KINDS: [i32; 3] = [0x1b1, 0x50a, 0x509];
/// Modes that walk the object list.
const WALK_MODES: [u32; 4] = [2, 9, 0xb, 0xd];
/// Mode that caps the scale.
const CAP_MODE: u32 = 6;
/// Retail callee run after the update (owner in a0).
const FINISH_FN: u32 = 0x227b90;
/// Retail HUD slot request run by the tail call.
const HUD_SLOT_REQUEST: u32 = 0x228840;
/// Frame size of the update.
const UPDATE_FRAME: u32 = 0x40;

/// 0x227c78 layer_scale_update(a0=owner) -> none: scales the layer record and the listed objects' slots by the gp
/// scale (capped in mode 6), attaching the layer first when its flag byte is clear, then runs the finish callee.
pub fn layer_scale_update(c: &mut Cpu) -> bool {
    let owner = c.arg(0);
    let gp = c.g(28) as u32;
    let orig_f20 = F(c.f[20] as u32);
    let mut scale = c.rf(gp.wrapping_sub(GP_SCALE_OFF));
    let saves = [(16, 0x00, h::SQ), (17, 0x10, h::SQ), (31, 0x20, h::SQ)];
    h::frame(c, UPDATE_FRAME, &saves, |c, sp| {
        c.wf(sp + 0x30, orig_f20);
        if c.r32(CAP_MODE_ADDR) == CAP_MODE && SCALE_CAP < scale {
            scale = SCALE_CAP;
        }
        let mut mode = c.r32(gp.wrapping_sub(MODE_GP_OFF));
        if c.r8(LAYER_REC + 1) == 0 {
            h::run(c, actor::layer_attach, &[owner, LAYER_KIND, LAYER_REC], &[]);
            c.wf(LAYER_REC + 0x28, scale);
            c.wf(LAYER_REC + 0x20, scale);
            c.wf(LAYER_REC + 0x24, scale);
            mode = c.r32(MODE_ADDR);
        }
        if WALK_MODES.contains(&mode) {
            scale_listed_objects(c, scale);
        }
        h::call(c, FINISH_FN, &[owner], &[]);
    });
    true
}

/// Walks the object list and multiplies the float at each listed slot's object (+0x24) by `scale` into the slot
/// (+0x2c), for the object kinds in SCALED_KINDS. The count is re-read every step, as retail does.
fn scale_listed_objects(c: &mut Cpu, scale: F) {
    let mut i: u32 = 0;
    while (i as i32) < c.r16(LIST_HEAD + LIST_COUNT_OFF) as i16 as i32 {
        let slot = c.r32(LIST_HEAD + LIST_PTRS_OFF + 4 * i);
        if slot != 0 {
            let kind = c.r16(slot + 0xa6) as i16 as i32;
            if SCALED_KINDS.contains(&kind) {
                let obj = c.r32(slot + 0x24);
                let v = c.rf(obj + 0x24) * scale;
                c.wf(slot + 0x2c, v);
            }
        }
        i += 1;
    }
}

/// 0x227f40 hud_slot_request_tail() -> v0 from hud_slot_request: requests HUD slot 1 with table arguments and
/// returns its result (the retail callee runs with its scratch registers left as retail leaves them).
pub fn hud_slot_request_tail(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x10, &[(31, 0x00, h::SQ)], |c, _sp| {
        h::call_keep(c, HUD_SLOT_REQUEST, &[1, 0x7535, 0x22_C6F0, 0x22_C770, 0x22_C950, 0x14_15F8, 8], &[]);
    });
    true
}
