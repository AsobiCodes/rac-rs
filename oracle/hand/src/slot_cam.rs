//! Weapon slot state step (retail 0x2ddad8) and camera class-6 re-seat (retail 0x2ee9b0).
//!
//! Retail callees are run as retail code (`h::call_keep`), so the scratch registers they leave behind match the
//! retail jal exactly. Only the register values the retail body itself writes are set here.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const S0: usize = 16;
const S1: usize = 17;
const S2: usize = 18;
const RA: usize = 31;
const V1: usize = 3;
const A0: usize = 4;
/// Both retail bodies open a 0x40-byte frame: s0 at +0, s1 at +0x10, s2 at +0x20, ra at +0x30.
const FRAME_SIZE: u32 = 0x40;

/// Retail callees.
const COMP_LOOKUP_FN: u32 = 0x2dd548;
const FLOAT_TO_INT_FN: u32 = 0x1f14b8;
const ANIM_START_FN: u32 = 0x24a148;
const CAM_PARAMS_COPY_FN: u32 = 0x1dd8c8;
const CAM_PARAMS_ZERO_FN: u32 = 0x1dd8b0;
const RIG_TAIL_FN: u32 = 0x1f17e8;
const CAM_RESET_FN: u32 = 0x2ee908;

/// Owner byte of the object: 0xfe and 0xfd mean the object takes no slot.
const OBJ_KIND_OFF: u32 = 0x20;
const KIND_INACTIVE: u8 = 0xfe;
const KIND_SKIPPED: u8 = 0xfd;
/// Object byte holding the animation index the slot should show.
const OBJ_ANIM_OFF: u32 = 0x53;
/// Slot record: state halfword, slot index halfword, pointer to the animation block.
const REC_STATE_OFF: u32 = 0x68;
const REC_SLOT_OFF: u32 = 0x6a;
const REC_ANIM_PTR_OFF: u32 = 0x70;
/// Byte inside the animation block that holds its current animation index.
const ANIM_INDEX_OFF: u32 = 7;
/// State the record is left in after a step, and the state that is skipped outright.
const STATE_DONE: u32 = 7;
const STATE_SKIP: i32 = 8;
const STATE_COUNT: i32 = 10;
/// Retail jump table of the states (ten words at 0x1dc550) and its targets, as read from the image.
const STATE_TABLE: u32 = 0x1dc550;
const STATE_TARGETS: [u32; 10] = [
    0x2ddbc0, 0x2ddb78, 0x2ddb78, 0x2ddb48, 0x2ddbb8, 0x2ddbc0, 0x2ddbc0, 0x2ddbc0, 0x2ddbc0, 0x2ddbc0,
];
const STATE_RELEASE: i32 = 3;
const STATE_SHOWN: i32 = 4;
/// Retail address of the slot owner table (one word per slot) and of the active-slot counter word.
const SLOT_OWNER_TABLE: u32 = 0x1413a0;
const SLOT_COUNT_WORD: u32 = 0x1413cc;
const NO_SLOT: i16 = -1;
/// Scale argument of float_to_int_scaled used for the animation speed.
const SPEED_SCALE: u32 = 10;

/// Camera rig: pointer at +0x70 of the camera, target block at +0xb0, copy block at +0x80.
const CAM_RIG_OFF: u32 = 0x70;
const CAM_PARAMS_OFF: u32 = 0x50;
const RIG_TARGET_OFF: u32 = 0xb0;
const RIG_COPY_OFF: u32 = 0x80;
const RIG_YAW_OFF: u32 = 0x18;
const RIG_PARAMS_OFF: u32 = 0x14;
const RIG_TAIL_OFF: u32 = 0x90;

fn set_v1(c: &mut Cpu, v: u32) {
    c.r[V1] = v as i32 as i128 as u128;
}

fn set_a0(c: &mut Cpu, v: u32) {
    c.r[A0] = v as i32 as i128 as u128;
}

/// 0x2ddad8 slot_state_step(obj) -> v0: finds the slot record of obj (retail comp_lookup) and steps its state; state 3
/// releases its slot, states 1-3 reload the animation when the index changed, state 4 returns 1, all others 0.
pub fn slot_state_step(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    let frame = &[(S0, 0x0, h::SQ), (S1, 0x10, h::SQ), (S2, 0x20, h::SQ), (RA, 0x30, h::SQ)];
    h::frame(c, FRAME_SIZE, frame, |c, _sp| {
        let (rec, _) = h::call_keep(c, COMP_LOOKUP_FN, &[obj], &[]);
        let v0 = slot_state(c, obj, rec);
        c.ret(v0);
    });
    true
}

/// The body of slot_state_step once the record is known; returns the v0 value and leaves v1 / a0 as retail does.
fn slot_state(c: &mut Cpu, obj: u32, rec: u32) -> u32 {
    if rec == 0 || obj == 0 {
        return 0;
    }
    let kind = c.r8(obj + OBJ_KIND_OFF);
    set_v1(c, kind as u32);
    if kind == KIND_INACTIVE || kind == KIND_SKIPPED {
        return 0;
    }
    let state = c.r16(rec + REC_STATE_OFF) as i16 as i32;
    set_v1(c, state as u32);
    if state == STATE_SKIP || !(0..STATE_COUNT).contains(&state) {
        return 0;
    }
    let index = state as usize;
    set_v1(c, STATE_TABLE + 4 * index as u32);
    set_a0(c, STATE_TARGETS[index]);
    match state {
        0 | 5..=9 => return 0,
        STATE_SHOWN => return 1,
        STATE_RELEASE => release_slot(c, rec),
        _ => {}
    }
    let anim = c.r32(rec + REC_ANIM_PTR_OFF);
    set_a0(c, anim);
    let wanted = c.r8(obj + OBJ_ANIM_OFF);
    let current = c.r8(anim + ANIM_INDEX_OFF);
    if wanted != current {
        let (scaled, _) = h::call_keep(c, FLOAT_TO_INT_FN, &[SPEED_SCALE], &[]);
        h::call_keep(c, ANIM_START_FN, &[obj, current as u32, 0, scaled], &[]);
    }
    c.w16(rec + REC_STATE_OFF, STATE_DONE as u16);
    set_v1(c, STATE_DONE);
    0
}

/// Clears the record's slot in the owner table and decrements the active-slot counter when it is above zero.
fn release_slot(c: &mut Cpu, rec: u32) {
    let slot = c.r16(rec + REC_SLOT_OFF) as i16;
    if slot == NO_SLOT {
        return;
    }
    let entry = SLOT_OWNER_TABLE.wrapping_add((slot as i32 as u32).wrapping_mul(4));
    c.w32(entry, 0);
    let count = c.r32(SLOT_COUNT_WORD) as i32;
    if count > 0 {
        c.w32(SLOT_COUNT_WORD, (count - 1) as u32);
    }
}

/// 0x2ee9b0 cam6_reseat(cam, src, quad, yaw): class-6 re-seat: copies the quad into the rig's two blocks, stores yaw,
/// copies src into two parameter blocks, clears the first, then runs the retail tail (0x1f17e8, 0x2ee908).
pub fn cam6_reseat(c: &mut Cpu) -> bool {
    let cam = c.arg(0);
    let src = c.arg(1);
    let quad = c.arg(2);
    let yaw = c.farg(0);
    let frame = &[(S0, 0x0, h::SQ), (S1, 0x10, h::SQ), (S2, 0x20, h::SQ), (RA, 0x30, h::SQ)];
    h::frame(c, FRAME_SIZE, frame, |c, _sp| {
        let rig = c.r32(cam + CAM_RIG_OFF);
        let first = c.rv(quad);
        c.wv(rig + RIG_TARGET_OFF, first);
        let second = c.rv(quad);
        c.wv(rig + RIG_COPY_OFF, second);
        c.wf(rig + RIG_TARGET_OFF + RIG_YAW_OFF, yaw);
        h::call_keep(c, CAM_PARAMS_COPY_FN, &[rig + RIG_PARAMS_OFF, src], &[yaw]);
        h::call_keep(c, CAM_PARAMS_COPY_FN, &[cam + CAM_PARAMS_OFF, src], &[F(0)]);
        h::call_keep(c, CAM_PARAMS_ZERO_FN, &[rig], &[]);
        h::call_keep(c, RIG_TAIL_FN, &[rig + RIG_TAIL_OFF], &[]);
        h::call_keep(c, CAM_RESET_FN, &[cam], &[]);
    });
    true
}
