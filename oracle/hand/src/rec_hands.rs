//! Hand rewrites of three R&C1 EE helpers: the 64-byte record list walk (0x259d88), the blend object step
//! (0x265bd8) and the GS bucket emit (0x2781e0).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// MIPS register numbers the hands write as retail leaves them (scratch registers the caller may read).
const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 12;
const T1: usize = 13;

/// Sets a GPR the way a 32-bit MIPS op leaves it (sign-extended over the whole register).
fn put(c: &mut Cpu, r: usize, v: u32) {
    c.r[r] = v as i32 as i128 as u128;
}

/// Record size of the walked list (0x40 bytes per record).
const RECORD_SIZE: u32 = 0x40;
/// Callback table indexed by record byte 0 (one word per entry).
const CALLBACK_TABLE: u32 = 0x001b_1f80;
/// List state in gp-relative slots: head pointer (-0x6b14), record count (-0x6b0c), cursor (-0x6af0),
/// end pointer (-0x6aec) and the slot where the return address is saved (-0x6af8).
const GP_HEAD: u32 = 0x6b14;
const GP_COUNT: u32 = 0x6b0c;
const GP_CURSOR: u32 = 0x6af0;
const GP_END: u32 = 0x6aec;
const GP_RA_SAVE: u32 = 0x6af8;

/// Blend object step: the record at obj + 0x20 holds a mode in its halfword at +2 (1 does nothing, 2 adds the
/// parent's translation to obj's position), and the limit halfword at +0x20.
const PARENT_PTR_OFF: u32 = 0x4;
const LIMIT_OFF: u32 = 0x20;
/// Retail callback run when the time byte of obj + 4 is not above the limit.
const STEP_CALLBACK: u32 = 0x0025_9b88;

/// GS bucket emit: pointer to the packet write cursor, record table base (gp-relative index table at -0x6860),
/// and the GIF tag words the retail code writes.
const GS_CURSOR_SLOT: u32 = 0x0016_1140;
const GS_RECORD_BASE: u32 = 0x001b_bf90;
const GS_INDEX_TABLE_GP: u32 = 0x6860;
const GS_SAVED_PTR_GP: u32 = 0x5ac0;
const GS_CALLEE: u32 = 0x0027_9638;
const GS_TAG_HEAD: u32 = 0x3000_0003;
const GS_TAG_REG: u32 = 0x1300_0000;
const GS_TAG_TAIL: u32 = 0x5000_0003;
const GS_RECORD_STRIDE: u32 = 0x30;

/// 0x259d88 walk_records(): calls the callback of every record between the list head and the end, selected by
/// record byte 0; records whose byte 1 is negative are skipped. The return address is kept in the gp slot.
// Declined: the callbacks it calls leave VU register vf3 different from retail (38/300 s07 calls), and the
// callee chain is outside this module, so retail runs the whole function.
#[allow(unreachable_code)]
pub fn walk_records(c: &mut Cpu) -> bool {
    return false;
    let gp = c.g(28) as u32;
    let ra_save = gp.wrapping_sub(GP_RA_SAVE);
    let cursor_slot = gp.wrapping_sub(GP_CURSOR);
    let end_slot = gp.wrapping_sub(GP_END);
    let ra = c.r[31] as u64;
    c.w32(ra_save, ra as u32);
    c.w32(ra_save.wrapping_add(4), (ra >> 32) as u32);
    let count = c.r32(gp.wrapping_sub(GP_COUNT));
    let head = c.r32(gp.wrapping_sub(GP_HEAD));
    let end = head.wrapping_add(count.wrapping_add(1) << 6);
    c.w32(end_slot, end);
    let mut cur = head;
    loop {
        // Loop head: a3 = end, at = byte 1 of the record (the delay slot of beq), a2 = cursor.
        let end = c.r32(end_slot);
        put(c, A3, end);
        let flag = c.r8(cur.wrapping_add(1)) as i8;
        put(c, AT, flag as i32 as u32);
        put(c, A2, cur);
        if cur == end {
            break;
        }
        let kind = c.r8(cur) as i8;
        put(c, V0, kind as i32 as u32);
        cur = cur.wrapping_add(RECORD_SIZE);
        put(c, A2, cur);
        if flag < 0 {
            continue;
        }
        put(c, V1, CALLBACK_TABLE);
        let callback = c.r32(CALLBACK_TABLE.wrapping_add((kind as i32 as u32) << 2));
        put(c, V0, callback);
        c.w32(cursor_slot, cur);
        h::call_keep(c, callback, &[cur.wrapping_sub(RECORD_SIZE)], &[]);
        cur = c.r32(cursor_slot);
        put(c, A2, cur);
    }
    let lo = c.r32(ra_save) as u64;
    let hi = c.r32(ra_save.wrapping_add(4)) as u64;
    c.r[31] = (c.r[31] & !(u64::MAX as u128)) | (lo | (hi << 32)) as u128;
    true
}

/// 0x265bd8 blend_obj_step(obj): for mode 2 adds the parent's translation (x, y, z at +0x10..+0x18) to obj's
/// position; then the time byte (top byte of the word at +4) becomes (time - limit) when that is positive, or
/// the step callback 0x259b88 runs otherwise.
pub fn blend_obj_step(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let rec = obj.wrapping_add(LIMIT_OFF);
    h::frame(c, 0x10, &[(31, 0, h::SQ)], |c, _sp| {
        c.ret(2);
        let mode = c.r16(rec + 2) as i16;
        if mode == 1 {
            return;
        }
        if mode == 2 {
            let parent = c.r32(rec + PARENT_PTR_OFF);
            for off in [0x10u32, 0x14, 0x18] {
                let sum = c.rf(parent + off) + c.rf(rec + off);
                c.wf(obj + off, sum);
            }
        }
        let word = c.r32(obj + PARENT_PTR_OFF);
        let limit = c.r16(rec) as i16 as i32;
        c.ret(limit as u32);
        let t = ((word as i32) >> 24).wrapping_sub(limit);
        if t > 0 {
            let v = ((t as u32) << 24) | (word & 0x00ff_ffff);
            c.w32(obj + PARENT_PTR_OFF, v);
            c.ret(v);
        } else {
            c.ret(0x00ff_0000);
            let (v0, _) = h::call(c, STEP_CALLBACK, &[obj, word], &[]);
            c.ret(v0);
        }
    });
    true
}

/// 0x2781e0 emit_gs_bucket(a0, idx, rec): writes the GIF tag, the record address (rec * 0x30 + 0x1bbf90) and
/// the tag words at the GS write cursor, then calls 0x279638 with the bucket word at index idx.
pub fn emit_gs_bucket(c: &mut Cpu) -> bool {
    let a0 = c.arg(0);
    let idx = c.arg(1);
    let rec = c.arg(2);
    h::frame(c, 0x10, &[(31, 0, h::SQ)], |c, _sp| {
        let ptr = c.r32(GS_CURSOR_SLOT);
        let rec_addr = rec.wrapping_mul(GS_RECORD_STRIDE).wrapping_add(GS_RECORD_BASE);
        c.w32(ptr, GS_TAG_HEAD);
        c.w32(ptr.wrapping_add(4), rec_addr);
        c.w32(ptr.wrapping_add(8), GS_TAG_REG);
        c.w32(ptr.wrapping_add(0xc), GS_TAG_TAIL);
        let gp = c.g(28) as u32;
        let slot = gp.wrapping_sub(GS_INDEX_TABLE_GP).wrapping_add(idx << 2);
        let bucket = c.r32(slot);
        c.w32(gp.wrapping_sub(GS_SAVED_PTR_GP), ptr.wrapping_add(0x10));
        put(c, V0, ptr.wrapping_add(0x10));
        put(c, V1, ptr);
        put(c, T0, ptr);
        put(c, T1, GS_TAG_REG);
        let (v0, _) = h::call_keep(c, GS_CALLEE, &[a0, bucket, rec_addr, GS_TAG_TAIL], &[]);
        c.ret(v0);
    });
    true
}
