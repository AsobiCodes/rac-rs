//! Per-frame display-list setup of the Eudora overlay: DMA chain tags (qword: tag word, next address, 0, 0) appended
//! at the list cursor, and the scratchpad resets around them.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Cursor of the packet list (next free 16-byte tag).
const LIST: u32 = 0x161140;
/// Pointer to the current head tag of the main chain / of the second chain.
const HEAD: u32 = 0x15ff4c;
const HEAD2: u32 = 0x15ff80;
/// DMA tag id bits: 2 = next (continue at the address in the tag), 1 = cnt.
const TAG_NEXT: u32 = 0x2000_0000;
const TAG_CNT: u32 = 0x1000_0000;
/// Frame-state words.
const LIST_LIMIT: u32 = 0x161148;
const SECOND_CHAIN_ON: u32 = 0x15ff7c;
const PREFETCH_ON: u32 = 0x15ff78;
const MODE_A: u32 = 0x16a114;
const MODE_B: u32 = 0x16a118;
const SPR_TAGS: u32 = 0x70003800;
const SPR_CLEAR: u32 = 0x70003a00;
/// Where the skin pass 0x2481a8 spills the caller's ra (DAT_0019b8d8, last of its 12 saved registers).
const SKIN_SPILL_RA: u32 = 0x19b8d8;

/// MIPS registers the retail code leaves set (lw / lui / addiu sign-extend; the upper 64 bits are kept).
const AT: usize = 1;
const V0: usize = 2;
const S0: usize = 16;
const S1: usize = 17;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
/// `lui at, 0x16`: the upper half of the 0x16xxxx frame-state addresses.
const AT_HI: u32 = 0x16_0000;
/// 0x16a0f0 (`addiu a3, v0(0x170000), -0x5f10`): base of the mode words MODE_A (+0x24) / MODE_B (+0x28).
const MODE_BASE: u32 = 0x16a0f0;

/// Sets register `r` the way LW / LUI / ADDIU do: sign-extended to 64 bits, upper 64 bits kept.
fn set(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

fn gp(c: &Cpu, off: i32) -> u32 {
    (c.g(28) as u32).wrapping_add(off as u32)
}

fn call(c: &mut Cpu, addr: u32, args: &[u32]) {
    for (i, &a) in args.iter().enumerate() {
        c.set_arg(i, a);
    }
    c.call(addr);
}

/// Writes tag fields (+4 = next pointer, +8 and +0xc cleared) of the tag at `tag`.
fn set_next(c: &mut Cpu, tag: u32, next: u32) {
    c.w32(tag + 4, next);
    c.w32(tag + 8, 0);
    c.w32(tag + 0xc, 0);
}

/// 0x242290 frame_list_begin(): links the head tag to a new cursor tag, then appends tags at the cursor (two
/// qwords), and sets the third tag (the gp-held pointer, or the cursor after 0x247e88 / 0x2915b0 in both-mode-flags state) to next.
/// Registers as retail leaves them: at 0x160000, a1 TAG_NEXT, a0 = v1 = the last tag, v0 = the cursor after it; a2
/// TAG_NEXT and a3 MODE_BASE unless the two calls ran (then the callees' values).
pub fn frame_list_begin(c: &mut Cpu) -> bool {
    set(c, A2, TAG_NEXT);
    set(c, A3, MODE_BASE);
    set(c, AT, AT_HI);
    let head = c.r32(HEAD);
    let s0 = c.r32(LIST);
    c.w32(LIST, s0.wrapping_add(0x10));
    c.w32(head, TAG_NEXT);
    let (head, cur) = (c.r32(HEAD), c.r32(LIST));
    set_next(c, head, cur);
    let hold = gp(c, -0x5ac0);
    let mut target = c.r32(hold);
    let mode_a = c.r32(MODE_A);
    if c.r32(MODE_B) != 0 && mode_a != 0 {
        let a0 = c.r32(gp(c, -0x7d8c));
        set(c, V0, mode_a);
        set(c, V1, target);
        call(c, 0x247e88, &[a0, TAG_NEXT]);
        call(c, 0x2915b0, &[]);
        target = c.r32(LIST);
    }
    c.w32(target, TAG_NEXT);
    let (list, head) = (c.r32(LIST), c.r32(HEAD));
    c.w32(list + 4, head.wrapping_add(0x10));
    c.w32(list + 8, 0);
    c.w32(list + 0xc, 0);
    let next = c.r32(LIST).wrapping_add(0x10);
    c.w32(LIST, next);
    c.w32(s0, TAG_NEXT);
    let cur = c.r32(LIST);
    c.w32(s0 + 0xc, 0);
    c.w32(s0 + 4, cur);
    c.w32(s0 + 8, 0);
    set(c, AT, AT_HI);
    set(c, A1, TAG_NEXT);
    set(c, A0, list);
    set(c, V1, list);
    set(c, V0, cur);
    true
}

/// 0x242500 frame_list_second(): second chain head: closed with TAG_CNT when the second chain is off; otherwise
/// two linked tags are appended after 0x2787e0 / 0x2785e8 (scratch state) and 0x1e9600 runs on the 0x15ff00 table.
/// Registers as retail leaves them: off: a0 = v0 = v1 = the head tag; on: what 0x1e9600 leaves of a0 0x15ff00,
/// a1 8, a2 = v1 = the last tag, v0 = the cursor after it, at 0x160000.
pub fn frame_list_second(c: &mut Cpu) -> bool {
    // Retail's 0x30-byte frame saves s0, s1 and ra with sq and reloads them with lq (all 128 bits); the retail callees
    // below leave garbage in the upper halves of the s registers, which only that reload heals.
    crate::h::frame(c, 0x30, &[(16, 0, crate::h::SQ), (17, 0x10, crate::h::SQ), (31, 0x20, crate::h::SQ)], |c, _| {
        frame_list_second_body(c)
    });
    true
}

fn frame_list_second_body(c: &mut Cpu) {
    if c.r32(SECOND_CHAIN_ON) == 0 {
        // retail reloads the head pointer before each store
        let a0 = c.r32(HEAD2);
        c.w32(a0, TAG_CNT);
        let v1 = c.r32(HEAD2);
        c.w32(v1 + 4, 0);
        let v0 = c.r32(HEAD2);
        c.w32(v0 + 8, 0);
        let v1 = c.r32(HEAD2);
        c.w32(v1 + 0xc, 0);
        set(c, A0, a0);
        set(c, V0, v0);
        set(c, V1, v1);
        return;
    }
    let s0 = c.r32(LIST);
    set(c, S0, s0);
    set(c, S1, TAG_NEXT);
    c.w32(LIST, s0.wrapping_add(0x10));
    let head = c.r32(HEAD2);
    c.w32(head, TAG_NEXT);
    let (head, cur) = (c.r32(HEAD2), c.r32(LIST));
    c.w32(head + 4, cur);
    let head = c.r32(HEAD2);
    c.w32(head + 8, 0);
    let head = c.r32(HEAD2);
    c.w32(head + 0xc, 0);
    set(c, AT, AT_HI);
    set(c, A0, head);
    set(c, V0, head);
    set(c, V1, head);
    call(c, 0x2787e0, &[]);
    call(c, 0x2785e8, &[0x1ac880]);
    let list = c.r32(LIST);
    c.w32(list, TAG_NEXT);
    let head = c.r32(HEAD2).wrapping_add(0x10);
    let list = c.r32(LIST);
    c.w32(list + 4, head);
    let list = c.r32(LIST);
    set_next_tail(c, list);
    let next = c.r32(LIST).wrapping_add(0x10);
    c.w32(LIST, next);
    c.w32(s0, TAG_NEXT);
    let cur = c.r32(LIST);
    c.w32(s0 + 0xc, 0);
    c.w32(s0 + 8, 0);
    c.w32(s0 + 4, cur);
    set(c, AT, AT_HI);
    set(c, A2, list);
    set(c, V1, list);
    set(c, V0, cur);
    call(c, 0x1e9600, &[0x15ff00, 8]);
}

fn set_next_tail(c: &mut Cpu, tag: u32) {
    c.w32(tag + 8, 0);
    c.w32(tag + 0xc, 0);
}

/// 0x242648 spr_reload(): resets the scratchpad tag area (0x118a80, copy 0x800 bytes from 0x166180 to 0x70003800)
/// and re-arms the DMA with 0x2481a8.
pub fn spr_reload(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    // Retail runs in a 0x10-byte frame holding ra (sq / lq). The skin pass 0x2481a8 spills the caller's s0-s7, gp, sp,
    // s8 and ra into 0x19b880..0x19b8dc (dead after the call), so it must see retail's sp and, in DAT_0019b8d8, ra.
    crate::h::frame(c, 0x10, &[(31, 0, crate::h::SQ)], |c, _| {
        call(c, 0x118a80, &[0]);
        call(c, 0x1f16c0, &[SPR_TAGS, 0x166180, 0x800]);
        let a = c.r32(0x15f5d8);
        let b = c.r32(gp(c, -0x7624));
        let spilled = c.mem.rd64(SKIN_SPILL_RA);
        call(c, 0x2481a8, &[a, b]);
        // `Cpu::call` hands the callee its own return marker as ra; the retail `jal` at 0x242678 left 0x242680 (when
        // 0x2481a8 is stubbed it spills nothing and the word stays).
        if c.mem.rd64(SKIN_SPILL_RA) != spilled {
            c.mem.wr64(SKIN_SPILL_RA, 0x242680);
        }
    });
    true
}

/// 0x242690 spr_clear(): fills 0x380 bytes of the scratchpad at 0x70003a00 with 0x40000000.
pub fn spr_clear(c: &mut Cpu) -> bool {
    call(c, 0x1f15d8, &[SPR_CLEAR, 0x4000_0000, 0x380]);
    true
}

/// 0x242718 frame_state_init(): sets up the mode registers (0x291278 / 0x290f20 / 0x2913c8), the list heads and the
/// list limit (0x15f5dc - 0x10000).
pub fn frame_state_init(c: &mut Cpu) -> bool {
    let a1 = c.r16(0x10fa90) as u32;
    call(c, 0x291278, &[0x10faa0, a1]);
    c.w32(0x15f5c0, 6);
    call(c, 0x290f20, &[0x100080]);
    call(c, 0x2913c8, &[0x47, 0x5360b]);
    let list = c.r32(LIST);
    let v1 = c.r32(0x15ee78);
    c.w32(HEAD, list);
    c.w32(0x15ee74, v1);
    let g = gp(c, -0x5ac0);
    c.w32(g, list.wrapping_add(0x10));
    call(c, 0x1e9608, &[0x15ff10, 1]);
    let limit = c.r32(0x15f5dc).wrapping_sub(0x1_0000);
    let a0 = c.r32(0x15f5d8);
    c.w32(HEAD2, 0);
    c.w32(LIST_LIMIT, limit);
    c.w32(0x15ff54, a0);
    // Retail's leftover registers: v0 = limit, v1 = -0x10000, a0 = the loaded pointer.
    c.ret(limit);
    c.s(3, 0xffff_0000u32 as i32 as i64 as u64);
    c.set_arg(0, a0);
    true
}

/// 0x242850 frame_lists_build(): builds the list heads (0x1e9608 / 0x242290 / 0x1e9600), the scratchpad reload when
/// mode B is on and the second chain when its flag is set.
pub fn frame_lists_build(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    // Retail runs in a 0x20-byte frame (sq s0 at 0, sq ra at 0x10) and keeps s0 = the list base 0x15ff20, then 0x170000
    // (the page of the mode words) while it calls 0x242648. The skin pass inside spills the caller's s0, sp and ra, and
    // reloads the s registers with 64-bit ld, which leaves their upper halves as the skin code made them; the lq of s0
    // at the end puts all 128 bits back.
    crate::h::frame(c, 0x20, &[(16, 0, crate::h::SQ), (31, 0x10, crate::h::SQ)], |c, _| {
        set(c, S0, 0x15ff20);
        call(c, 0x1e9608, &[0x15ff20, 3]);
        c.call(0x242290);
        call(c, 0x1e9600, &[0x15ff20, 5]);
        set(c, S0, 0x170000);
        if c.r32(MODE_B) != 0 {
            c.call(0x242648);
            if c.r32(PREFETCH_ON) != 0 {
                c.call(0x248138);
            }
        }
        call(c, 0x1e9600, &[0x15ff30, 3]);
        // Retail's leftover registers: v1 = MODE_B (lw at 0x2428d0) and v0 = the word at gp-0x6c80, which the delay slot of
        // the `beqz v1` loads on both paths; the second-chain call below, when it runs, leaves its own.
        let mode_b = c.r32(MODE_B);
        let second = c.r32(gp(c, -0x6c80));
        set(c, V1, mode_b);
        set(c, V0, second);
        if mode_b != 0 && second != 0 {
            c.call(0x242500);
        }
    });
    true
}

/// 0x242900 frame_setup(): init, optional scratch clear with the packet DMA (0x248288), the list-overflow report and
/// the list build.
pub fn frame_setup(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    // Retail's 0x10-byte frame (sq ra / lq ra): the skin pass below spills sp (DAT_0019b8c8), so the callee chain
    // must see retail's stack depth.
    crate::h::frame(c, 0x10, &[(31, 0, crate::h::SQ)], |c, _| {
        c.call(0x242718);
        if c.r32(0x16a118) != 0 {
            c.call(0x242690);
            let (a, b) = (c.r32(0x15ff58), c.r32(0x15ff54));
            call(c, 0x248288, &[a, b, u32::MAX, 1]);
            let r = c.result();
            c.w32(gp(c, -0x6cac), r);
            if (c.r32(LIST_LIMIT) as i32) < c.r32(LIST) as i32 {
                call(c, 0x1dd878, &[0x1dab50]);
            }
        }
        c.call(0x242850);
    });
    true
}
