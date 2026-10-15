//! Per-frame tail hooks of the level 4 overlay: the frame tail (0x2864a0), the empty hook (0x288648) and the
//! weapon/targeting latch clear (0x288650).

use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const RA: usize = 31;
const S0: usize = 16;
const SP: usize = 29;
const GP: usize = 28;
const V0: usize = 2;
const AT: usize = 1;

/// Loader-side pointer word (lw/sw at 0x161140). Its value is stored to the save slot and bumped by 0x10.
const PTR_WORD: u32 = 0x161140;
/// Shadow word copied to 0x15ee74 on every frame.
const SHADOW_SRC: u32 = 0x15ee78;
const SHADOW_DST: u32 = 0x15ee74;
/// Save slot for the old pointer value, and the first hook's argument block.
const SAVE_SLOT: u32 = 0x160f58;
const HOOK_BLOCK_A: u32 = 0x160f20;
/// Second hook's argument block (s0 = 0x160000 + 0xf30).
const HOOK_BLOCK_B: u32 = 0x160f30;
/// s0 holds the hook block base (0x160000) from the lui in the delay slot until the second hook.
const HOOK_S0: u64 = 0x160000;
/// Loader flag word: when nonzero the GS packet and the cache flush run.
const LOADER_FLAG: u32 = 0x16a108;
const FLUSH_CACHE: u32 = 0x118a80;
const PRE_FLUSH_RETAIL: u32 = 0x2867c0;
const POST_RETAIL: u32 = 0x286218;
const GS_PACKET: u32 = 0x1c7380;
const GS_PACKET_LEN: u32 = 0x3600;
const GS_PACKET_TAG: u32 = 0x40;
const COPY_SRC: u32 = 0x1c3230;
const COPY_BYTES: u32 = 0x20;
/// Return addresses of the retail jal instructions (jal + 8): the callees store ra (0x2867c0 saves it to scratchpad).
const FLUSH_RET: u32 = 0x286500;
const PRE_FLUSH_RET: u32 = 0x286508;
const POST_RET: u32 = 0x286534;

/// Latch words cleared by 0x288650, in store order (the last one is gp-relative).
const LATCH_WORDS: [u32; 7] = [0x15f424, 0x17e604, 0x15f42c, 0x15f430, 0x15f428, 0x15f434, 0x15f2f0];
const LATCH_GP_OFS: u32 = 0x790c;

/// Calls retail code at `addr` as a retail `jal` does: ra = `ret`, and the callee's scratch registers stay as it
/// leaves them. s0-s7, gp, sp, fp, ra and f20-f31 are put back.
fn call_retail_at(c: &mut Cpu, addr: u32, ret: u32, args: &[u32]) {
    let (r, f) = (c.r, c.f);
    for (i, &a) in args.iter().enumerate() {
        c.set_arg(i, a);
    }
    c.s(RA, ret as u64);
    c.call_dyn(addr, ret);
    for i in (16..24).chain(28..32) {
        c.r[i] = r[i];
    }
    for i in 20..32 {
        c.f[i] = f[i];
    }
}

/// A 128-bit register as the four words of a quadword (lane 0 = low word).
fn quad(v: u128) -> V {
    V([v as u32, (v >> 32) as u32, (v >> 64) as u32, (v >> 96) as u32])
}

/// 0x2864a0 frame_tail(): saves the loader pointer word, runs the empty hook, and when the loader flag is set
/// flushes the cache and writes the GS packet. Then it runs the retail post-frame step, copies two quads and
/// runs the empty hook again.
pub fn frame_tail(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let entry_sp = c.g(SP);
    let entry_s0 = c.g(S0);
    let sp = (entry_sp as u32).wrapping_sub(0x20);
    c.s(SP, sp as i32 as i64 as u64);
    c.wv(sp.wrapping_add(0x10), quad(c.r[RA]));
    c.wv(sp, quad(c.r[S0]));

    let ptr = c.r32(PTR_WORD);
    let shadow = c.r32(SHADOW_SRC);
    c.w32(SAVE_SLOT, ptr);
    c.w32(SHADOW_DST, shadow);
    c.w32(PTR_WORD, ptr.wrapping_add(0x10));
    crate::h::run_keep(c, crate::collision::empty_hook, &[HOOK_BLOCK_A, 1], &[]);

    c.s(S0, HOOK_S0);
    if c.r32(LOADER_FLAG) != 0 {
        call_retail_at(c, FLUSH_CACHE, FLUSH_RET, &[0]);
        call_retail_at(c, PRE_FLUSH_RETAIL, PRE_FLUSH_RET, &[]);
        crate::h::run_keep(c, crate::gifpkt::gs_regs_write, &[GS_PACKET, GS_PACKET_LEN, GS_PACKET_TAG], &[]);
    }

    c.s(S0, HOOK_BLOCK_B as u64);
    crate::h::run_keep(c, crate::collision::empty_hook, &[HOOK_BLOCK_B, 5], &[]);
    call_retail_at(c, POST_RETAIL, POST_RET, &[HOOK_BLOCK_B, 5]);

    let dst = c.r32(PTR_WORD);
    crate::h::run_keep(c, crate::mathx::copy_qwords, &[dst, COPY_SRC, COPY_BYTES], &[]);
    crate::h::run_keep(c, crate::collision::empty_hook, &[HOOK_BLOCK_B, 5], &[]);
    c.s(S0, entry_s0);
    c.s(SP, entry_sp);
    true
}

/// 0x288648 noop_hook(): empty function (returns at once).
pub fn noop_hook(_c: &mut Cpu) -> bool {
    true
}

/// 0x288650 frame_latch_clear(): zeroes the weapon/targeting latch words and returns.
pub fn frame_latch_clear(c: &mut Cpu) -> bool {
    let gp = c.g(GP) as u32;
    for addr in LATCH_WORDS {
        c.w32(addr, 0);
    }
    c.w32(gp.wrapping_sub(LATCH_GP_OFS), 0);
    c.s(V0, 0x180000);
    c.s(AT, 0x160000);
    true
}
