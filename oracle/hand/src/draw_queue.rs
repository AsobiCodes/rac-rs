//! Per-frame draw callback queues (0x1ebc48..0x1ebe50): two 64-slot lists of (callback, moby) pairs. The main
//! queue is run once per frame by `draw_queue_run`, which calls each callback with its moby.
use crate::h::*;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// MIPS register numbers that the retail code writes as scratch: $at and $v1.
const AT: usize = 1;
const V1: usize = 3;
/// High half of the page that holds the queue words (`lui $at, 0x16`).
const QUEUE_PAGE: u32 = 0x16_0000;

/// Writes a 32-bit result to a register, sign-extended into its low 64 bits (upper half kept).
fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Maximum number of entries in each queue.
const QUEUE_MAX: i32 = 0x40;

/// One queue: an entry count word in RAM plus two parallel 64-entry word arrays.
struct Queue {
    count_at: u32,
    callbacks: u32,
    mobies: u32,
}

/// Main draw queue (count at 0x15f424, callbacks at 0x16d980, mobies at 0x16da80).
const MAIN: Queue = Queue { count_at: 0x15_f424, callbacks: 0x16_d980, mobies: 0x16_da80 };
/// Secondary draw queue (count at 0x15f428, callbacks at 0x16db80, mobies at 0x16dc80).
const ALT: Queue = Queue { count_at: 0x15_f428, callbacks: 0x16_db80, mobies: 0x16_dc80 };

/// Appends (callback, moby) to `q`. Returns the callback slot address in v0, or 0 when the queue is full.
fn enqueue(c: &mut Cpu, q: &Queue) -> bool {
    let (callback, moby) = (c.arg(0), c.arg(1));
    let count = c.r32(q.count_at);
    c.set_arg(2, count);
    c.set_arg(3, callback);
    c.set_arg(4, moby);
    if (count as i32) < QUEUE_MAX {
        let offset = count.wrapping_mul(4);
        c.set_arg(0, q.callbacks);
        c.set_arg(1, count.wrapping_add(1));
        c.w32(q.callbacks.wrapping_add(offset), callback);
        c.w32(q.mobies.wrapping_add(offset), moby);
        c.w32(q.count_at, count.wrapping_add(1));
        set32(c, V1, q.mobies.wrapping_add(offset));
        set32(c, AT, QUEUE_PAGE);
        c.ret(q.callbacks.wrapping_add(offset));
    } else {
        c.ret(0);
    }
    true
}

/// 0x1ebc48 draw_queue_add(callback, moby) -> slot: appends a draw callback to the main queue (max 64); v0 is the
/// callback slot address, or 0 if the queue is full.
pub fn draw_queue_add(c: &mut Cpu) -> bool {
    enqueue(c, &MAIN)
}

/// 0x1ebc98 draw_queue_run(): calls every main-queue callback in registration order as callback(moby). The count
/// is re-read after each call, so callbacks may queue more work. v0 ends as the initial count if the queue was
/// empty, otherwise 0.
pub fn draw_queue_run(c: &mut Cpu) -> bool {
    let initial = c.r32(MAIN.count_at);
    if (initial as i32) <= 0 {
        c.ret(initial);
        return true;
    }
    let mut i: u32 = 0;
    while (i as i32) < (c.r32(MAIN.count_at) as i32) {
        let offset = i.wrapping_mul(4);
        let callback = c.r32(MAIN.callbacks.wrapping_add(offset));
        let moby = c.r32(MAIN.mobies.wrapping_add(offset));
        c.set_arg(0, moby);
        c.call(callback);
        i = i.wrapping_add(1);
    }
    c.ret(0);
    true
}

/// 0x1ebe00 draw_queue_add_alt(callback, moby) -> slot: appends to the secondary queue (max 64); v0 is the callback
/// slot address, or 0 if the queue is full.
pub fn draw_queue_add_alt(c: &mut Cpu) -> bool {
    enqueue(c, &ALT)
}
