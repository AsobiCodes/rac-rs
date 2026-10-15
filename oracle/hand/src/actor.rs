//! Actor (moby) table, animation-layer and effect-slot helpers of the Eudora overlay.
use crate::cells::refile_cells;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// First entry and end of the 0x100-byte moby table.
const MOBY_BASE: u32 = 0x15ff5c;
const MOBY_END: u32 = 0x15ff60;
/// Per-moby 0x80-byte scratch area base (indexed by moby number).
const MOBY_SCRATCH: u32 = 0x15ff68;
/// Frame counter.
const FRAME: u32 = 0x15f5ac;
/// Count of moby slots that may be reclaimed.
const FREE_COUNT: u32 = 0x15ff3c;
/// gp-relative scratch counter (reset by the reclaim scan).
const GP_COUNTER: i32 = -0x6cc4;
/// Moby state byte: 0xff free, 0xfe / 0xfd released (reusable once the frame stamp at +0x38 has passed).
const STATE: u32 = 0x20;
const FREE: u8 = 0xff;
const RELEASED: u8 = 0xfe;
/// Rectangle key meaning "in no grid cell" (see cells::refile_cells).
const NO_CELLS: u32 = 0x8080_7f7f;
/// Active-moby table (16 pointers).
const ACTIVE: u32 = 0x196dc0;
/// Effect slots: 7 records of 0x40 bytes (the search stops at index 6 when 0..5 are used).
const FX_SLOTS: u32 = 0x18ec40;
const FX_DATA: u32 = 0x186e40;
const FX_DATA_STRIDE: u32 = 0x1500;
/// Animation track table of the default (no animation) state: 0x800 bytes per entry.
const DEFAULT_ANIM: u32 = 0x18edc0;
/// Voice slot table: 0x70 bytes per entry.
const VOICES: u32 = 0x13e550;
/// 1/16, 16.0, 1/1024 style constants.
const ONE_SIXTEENTH: F = F(0x3d80_0000);
const SIXTEEN: F = F(0x4180_0000);
const PIVOT_SCALE: F = F(0x3a80_0000);

fn gp(c: &Cpu, off: i32) -> u32 {
    (c.g(28) as u32).wrapping_add(off as u32)
}

/// Runs `f` with `size` bytes of EE stack reserved below sp (pointer passed in).
fn with_stack<R>(c: &mut Cpu, size: u32, f: impl FnOnce(&mut Cpu, u32) -> R) -> R {
    let sp = c.g(29) as u32;
    let buf = sp - size;
    c.s(29, buf as i32 as i64 as u64);
    let r = f(c, buf);
    c.s(29, sp as i32 as i64 as u64);
    r
}

fn call(c: &mut Cpu, addr: u32, args: &[u32]) {
    for (i, &a) in args.iter().enumerate() {
        c.set_arg(i, a);
    }
    c.call(addr);
}

fn callf(c: &mut Cpu, addr: u32, args: &[u32], f12: F) {
    c.set_farg(0, f12);
    call(c, addr, args);
}

fn fill(c: &mut Cpu, addr: u32, n: u32) {
    for i in 0..n {
        c.w8(addr + i, 0);
    }
}

/// The 64-bit frame stamp at moby + 0x38.
fn stamp(c: &mut Cpu, m: u32) -> u64 {
    c.r32(m + 0x38) as u64 | (c.r32(m + 0x3c) as u64) << 32
}

/// 0x240d28 count_reclaimable_mobys(): sets the reclaim counter to 1 once any slot after a free one is reclaimable.
pub fn count_reclaimable_mobys(c: &mut Cpu) -> bool {
    let counter = gp(c, GP_COUNTER);
    let (mut m, end) = (c.r32(MOBY_BASE), c.r32(MOBY_END));
    c.w32(counter, 0);
    let frame = c.r32(FRAME) as u64;
    let mut seen_free = false;
    while m < end {
        let state = c.r8(m + STATE);
        if seen_free || (state >= RELEASED && frame >= stamp(c, m)) {
            let n = c.r32(counter).wrapping_add(1);
            c.w32(FREE_COUNT, n);
            seen_free |= state == FREE;
        }
        m += 0x100;
    }
    true
}

/// 0x240db8 alloc_moby(arg) -> moby: first free or expired moby slot, initialised through 0x240eb0.
pub fn alloc_moby(c: &mut Cpu) -> bool {
    let arg = c.arg(0);
    let (mut m, end) = (c.r32(MOBY_BASE), c.r32(MOBY_END));
    let frame = c.r32(FRAME);
    while m < end {
        let state = c.r8(m + STATE);
        if state >= RELEASED && frame as u64 >= stamp(c, m) {
            if state == FREE {
                c.w8(m + 0x120, FREE);
            }
            call(c, 0x240eb0, &[m, arg]);
            let index = (m.wrapping_sub(c.r32(MOBY_BASE)) as i32) >> 8;
            let scratch = c.r32(MOBY_SCRATCH).wrapping_add((index << 7) as u32);
            c.w32(m + 0x78, scratch);
            call(c, 0x1f15d8, &[scratch, 0, 0x80]);
            let n = c.r32(FREE_COUNT);
            if n != 0 {
                c.w32(FREE_COUNT, n.wrapping_sub(1));
                c.s(1, 0x16_0000); // retail's lui $at, 0x16 survives
            }
            c.s(3, scratch as i32 as i64 as u64);
            c.ret(m);
            return true;
        }
        m += 0x100;
    }
    call(c, 0x1dd878, &[0x1dab10, frame]);
    c.ret(0);
    true
}

/// 0x2410e8 free_moby(m): marks a moby released (0xfd below the table start, else 0xfe), expiring in 2 frames, and
/// removes it from the grid cells.
pub fn free_moby(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let state = if m < c.r32(MOBY_BASE) { 0xfd } else { RELEASED };
    c.w8(m + STATE, state);
    let due = c.r32(FRAME).wrapping_add(2);
    c.w32(m + 0x38, due);
    c.w32(m + 0x3c, ((due as i32) >> 31) as u32);
    refile_cells(c, m, NO_CELLS);
    true
}

/// 0x241140 anim_load_tracks(m): fetches the animation data pointers (+0x68, +0x6c) and flags (+0x7c, +0x7e) for the
/// moby's current (+0x50/+0x52) and next (+0x51/+0x53) animation.
pub fn anim_load_tracks(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let (cur, next) = (c.r8(m + 0x50) as u32, c.r8(m + 0x51) as u32);
    let (cur_set, next_set) = (c.r8(m + 0x52), c.r8(m + 0x53) as u32);
    let tab = c.r32(m + 0x24).wrapping_add(0x48);
    if cur_set == 0xff {
        c.w8(m + 0x7c, 0xff);
        c.w8(m + 0x7e, 0);
        c.w32(m + 0x68, (cur << 11).wrapping_add(DEFAULT_ANIM));
    } else {
        let set = c.r32(tab + 4 * cur_set as u32);
        let entry = c.r32(set.wrapping_add(4 * cur).wrapping_add(0x1c));
        c.w32(m + 0x68, entry);
        let v = c.r8(set + 0x12);
        c.w8(m + 0x7e, v);
        let v = c.r8(set + 0x11);
        c.w8(m + 0x7c, v);
    }
    let set = c.r32(tab + 4 * next_set);
    let entry = c.r32(set.wrapping_add(4 * next).wrapping_add(0x1c));
    c.w32(m + 0x6c, entry);
    true
}

/// 0x241200 actor_voice_sync(m): keeps the voice slot (+0x7d) in step with the animation sound id (+0x7c).
pub fn actor_voice_sync(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let (voice, sound) = (c.r8(m + 0x7d), c.r8(m + 0x7c));
    if voice == 0xff {
        if sound != 0xff {
            call(c, 0x27e470, &[sound as u32, 4, m]);
            let v = c.result() as u8;
            c.w8(m + 0x7d, v);
        }
        return true;
    }
    let rec = VOICES + 0x70 * voice as u32;
    if c.r32(rec + 0x88) == m {
        if c.r16(rec + 0x7e) as i16 as u32 != sound as u32 {
            call(c, 0x27e1a0, &[voice as u32]);
            c.w8(m + 0x7d, 0xff);
        }
    } else {
        c.w8(m + 0x7d, 0xff);
    }
    true
}

/// 0x241298 anim_find_key(m, angle) -> index of the key (+0x1c table, count +0x10) nearest below angle * 16, or -1.
pub fn anim_find_key(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let x = c.farg(0) * SIXTEEN;
    callf(c, 0x1f27e8, &[m], x);
    let t = c.result() as i16 as i32;
    let n = c.r8(m + 0x10) as i32;
    let key = |c: &mut Cpu, i: i32| {
        let k = c.r32(m + 0x1c + 4 * i as u32);
        c.r16(k + 4) as i16 as i32
    };
    for i in 0..n {
        let k = key(c, i);
        if t < k {
            let best = if i > 0 && t - key(c, i - 1) < k - t { i - 1 } else { i };
            c.ret(best as u32);
            return true;
        }
    }
    c.ret(-1i32 as u32);
    true
}

/// 0x241348 anim_blend_value(m) -> value: the current key's value (key +4, via 0x1f27d8) / 16, mixed with the next
/// key's by the blend factor at +0x54.
pub fn anim_blend_value(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let (cur_set, next_set) = (c.r8(m + 0x52), c.r8(m + 0x53));
    let key = if cur_set == 0xff { c.r32(m + 0x6c) } else { c.r32(m + 0x68) };
    let t = c.rf(m + 0x54);
    let value = |c: &mut Cpu, key: u32| -> F {
        let a = c.r16(key + 4) as i16 as i32 as u32;
        call(c, 0x1f27d8, &[a]);
        c.fresult()
    };
    let r = if t == F::ZERO {
        value(c, key) * ONE_SIXTEENTH
    } else if cur_set != next_set || c.r8(m + 0x51) < c.r8(m + 0x50) {
        let v = value(c, key) * ONE_SIXTEENTH;
        v + c.rf(m + 0x54)
    } else {
        let next = c.r32(m + 0x6c);
        let a = value(c, key) * (F::ONE - t);
        let b = value(c, next) * c.rf(m + 0x54);
        (a + b) * ONE_SIXTEENTH
    };
    c.fret(r);
    true
}

/// 0x2417a0 link_node_set(m, i, v, f12 = a, f13 = b): node i (32 bytes at m[0x94] + 16) gets the vector
/// (v - m.pos) * m.rot * k and scalars b * k at +4, a * k at +0x1c, with k = 1024 / m[0x2c].
pub fn link_node_set(c: &mut Cpu) -> bool {
    let (m, i, v) = (c.arg(0), c.arg(1), c.arg(2));
    let (a, b) = (c.farg(0), c.farg(1));
    let node = c.r32(m + 0x94).wrapping_add((i << 5) + 0x10);
    let k = F(0x4480_0000) / c.rf(m + 0x2c);
    with_stack(c, 0xc0, |c, buf| {
        let d = buf + 0x40;
        call(c, 0x1f2290, &[buf, m + 0xc0]);
        call(c, 0x1f1840, &[d, v, m + 0x10]);
        call(c, 0x1f1c90, &[d, d, buf]);
        callf(c, 0x1f1898, &[d, d], k);
        let r = c.rv(d);
        c.wv(node + 0x10, r);
    });
    c.wf(node + 0x1c, a * k);
    c.wf(node + 4, b * k);
    true
}

/// First free effect slot index (6 when the six regular ones are used).
fn fx_free_index(c: &mut Cpu) -> u32 {
    (0..6).find(|&i| c.r8(FX_SLOTS + 0x40 * i + 4) == 0).unwrap_or(6)
}

/// 0x2418f0 fx_slot_alloc(m, id) -> slot: claims an effect slot for animation `id` of the moby and links it into the
/// moby's slot list (+0x60).
pub fn fx_slot_alloc(c: &mut Cpu) -> bool {
    let (m, id) = (c.arg(0), c.arg(1));
    let i = fx_free_index(c);
    let slot = FX_SLOTS + 0x40 * i;
    c.w8(slot + 4, 1);
    c.w16(slot, id as u16);
    c.w32(slot + 0x10, FX_DATA + FX_DATA_STRIDE * i);
    let model = c.r32(m + 0x24);
    c.w32(slot + 0x14, model);
    let anim = c.r32(model + 0x1c).wrapping_add(((id as i16 as i32) << 2) as u32);
    let data = c.r32(anim + 4);
    let b = c.r8(data);
    let v = c.r8(data + 2) as u16;
    c.w16(slot + 2, v);
    c.w32(slot + 0xc, data + b as u32 + 4);
    let head = c.r32(m + 0x60);
    c.w32(slot + 0x1c, head);
    c.w32(m + 0x60, slot);
    c.ret(slot);
    true
}

/// Unlinks `rec` from the singly linked list whose head pointer lives at `head_at` and whose next links are at
/// `next` (no change when `rec` is not on the list), then clears the 0x40-byte record.
fn unlink_clear(c: &mut Cpu, head_at: u32, next: u32, rec: u32) {
    let head = c.r32(head_at);
    if head == rec {
        let n = c.r32(rec + next);
        c.w32(head_at, n);
    } else {
        let mut p = head;
        loop {
            let n = c.r32(p + next);
            if n == 0 {
                break;
            }
            if n == rec {
                let after = c.r32(rec + next);
                c.w32(p + next, after);
                break;
            }
            p = n;
        }
    }
    fill(c, rec, 0x40);
}

/// 0x2419b0 fx_slot_free(m, slot_ptr): releases the slot stored at *slot_ptr from the moby's list and clears it.
pub fn fx_slot_free(c: &mut Cpu) -> bool {
    let (m, at) = (c.arg(0), c.arg(1));
    let rec = c.r32(at);
    if rec == 0 {
        return true;
    }
    unlink_clear(c, m + 0x60, 0x1c, rec);
    c.w32(at, 0);
    true
}

/// 0x241ca8 fx_slot_start(m, slot, a2, a3, t0 = steps, t1 = update): sets the slot's from/to animation bytes
/// (+0x20..+0x23), per-step rate 1 / steps (+0x2c), progress 0 / 1.0, and optionally runs the first update.
pub fn fx_slot_start(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let (m, slot, a2, a3, steps, update) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3), c.arg(4), c.arg(5));
    let rate = F::ONE / F::from_int(steps as i32);
    let (b53, b51) = (c.r8(m + 0x53), c.r8(m + 0x51));
    c.w8(slot + 0x22, b53);
    c.w8(slot + 0x23, a2 as u8);
    c.w8(slot + 0x20, b51);
    c.w8(slot + 0x21, a3 as u8);
    c.wf(slot + 0x2c, rate);
    c.w32(slot + 0x24, 0);
    c.wf(slot + 0x28, F::ONE);
    if update != 0 {
        call(c, 0x241a58, &[m, slot]);
    }
    true
}

/// 0x241e58 layer_attach(m, id, layer): fills an idle layer record (+1 clear) and pushes it on the moby's list
/// (+0x64).
pub fn layer_attach(c: &mut Cpu) -> bool {
    let (m, id, layer) = (c.arg(0), c.arg(1), c.arg(2));
    if c.r8(layer + 1) != 0 {
        return true;
    }
    c.w8(layer, id as u8);
    c.w8(layer + 1, 1);
    for off in [0x28, 0x1c, 0x20, 0x24] {
        c.wf(layer + off, F::ONE);
    }
    let model = c.r32(m + 0x24);
    let anim = c.r32(model + 0x1c).wrapping_add(4 * (id & 0xff));
    let data = c.r32(anim + 4);
    let at = data + c.r8(data) as u32;
    let kind = c.r8(at + 4) as u32;
    c.w32(layer + 4, (kind << 6).wrapping_add(0x7000_0000));
    let head = c.r32(m + 0x64);
    c.w32(layer + 8, head);
    c.w32(m + 0x64, layer);
    true
}

/// 0x241ed0 layer_detach(m, layer): unlinks a layer record from the moby's list (+0x64) and clears it.
pub fn layer_detach(c: &mut Cpu) -> bool {
    let (m, layer) = (c.arg(0), c.arg(1));
    if layer != 0 {
        unlink_clear(c, m + 0x64, 8, layer);
    }
    true
}

/// 0x241f60 active_add(m) -> index: stores the moby in the first free (or already holding it) of 16 active slots,
/// -1 when full.
pub fn active_add(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    for i in 0..16 {
        let v = c.r32(ACTIVE + 4 * i);
        if v == 0 || v == m {
            c.w32(ACTIVE + 4 * i, m);
            c.ret(i);
            return true;
        }
    }
    c.ret(-1i32 as u32);
    true
}

/// 0x241fa8 active_prune(): drops active-table mobys whose state has bit 0x80 or whose current animation set (+0x52) is not 0xff.
pub fn active_prune(c: &mut Cpu) -> bool {
    for i in 0..16 {
        let m = c.r32(ACTIVE + 4 * i);
        if m != 0 && (c.r8(m + STATE) & 0x80 != 0 || c.r8(m + 0x52) != 0xff) {
            c.w32(ACTIVE + 4 * i, 0);
        }
    }
    true
}

/// 0x241ff0 pose_node_world(m, a1, out): out matrix = (node matrix from 0x246868) * m's rotation, translation scaled
/// by m[0x2c] / 1024 and moved by m.pos.
pub fn pose_node_world(c: &mut Cpu) -> bool {
    let (m, a1, out) = (c.arg(0), c.arg(1), c.arg(2));
    let k = c.rf(m + 0x2c) * PIVOT_SCALE;
    with_stack(c, 0xa0, |c, buf| {
        let t = buf + 0x40;
        c.w32(t, a1);
        call(c, 0x246868, &[m, 1, t, out]);
        callf(c, 0x1f1898, &[out + 0x30, out + 0x30], k);
        call(c, 0x1f2250, &[buf, m + 0xc0]);
        call(c, 0x1f2370, &[out, buf, out]);
        call(c, 0x1f1810, &[out + 0x30, out + 0x30, m + 0x10]);
    });
    true
}

/// 0x242090 pose_node_pos(m, a1, out): out = (translation from 0x246868) * scale * rotation + m.pos.
pub fn pose_node_pos(c: &mut Cpu) -> bool {
    let (m, a1, out) = (c.arg(0), c.arg(1), c.arg(2));
    let k = c.rf(m + 0x2c) * PIVOT_SCALE;
    with_stack(c, 0x90, |c, buf| {
        let t = buf + 0x40;
        c.w32(t, a1);
        call(c, 0x246868, &[m, 1, t, buf]);
        callf(c, 0x1f1898, &[out, buf + 0x30], k);
        call(c, 0x1f1c68, &[out, out, m + 0xc0]);
        call(c, 0x1f1810, &[out, out, m + 0x10]);
    });
    true
}

/// 0x242118 pose_chain_pos(m, n, a2, out): n consecutive positions (0x246710 fills `out`), each scaled, rotated by
/// m's matrix and moved by m.pos.
pub fn pose_chain_pos(c: &mut Cpu) -> bool {
    let (m, n, a2, out) = (c.arg(0), c.arg(1) as i32, c.arg(2), c.arg(3));
    let f12 = c.farg(0);
    let k = c.rf(m + 0x2c) * PIVOT_SCALE;
    callf(c, 0x246710, &[m, n as u32, a2, out], f12);
    let mut p = out;
    for _ in 0..n.max(0) {
        callf(c, 0x1f1898, &[p, p], k);
        call(c, 0x1f1c68, &[p, p, m + 0xc0]);
        call(c, 0x1f1810, &[p, p, m + 0x10]);
        p += 0x10;
    }
    if n > 0 {
        c.set_arg(0, p);
    }
    true
}

/// 0x2421b8 pose_chain_world(m, n, a2, out): n node matrices (0x246868), each multiplied by m's rotation, with the
/// translation scaled and moved by m.pos.
pub fn pose_chain_world(c: &mut Cpu) -> bool {
    let (m, n, a2, out) = (c.arg(0), c.arg(1) as i32, c.arg(2), c.arg(3));
    let k = c.rf(m + 0x2c) * PIVOT_SCALE;
    with_stack(c, 0xb0, |c, buf| {
        call(c, 0x1f2250, &[buf, m + 0xc0]);
        call(c, 0x246868, &[m, n as u32, a2, out]);
        let mut p = out;
        for _ in 0..n.max(0) {
            call(c, 0x1f2370, &[p, buf, p]);
            callf(c, 0x1f1898, &[p + 0x30, p + 0x30], k);
            call(c, 0x1f1810, &[p + 0x30, p + 0x30, m + 0x10]);
            p += 0x40;
            c.set_arg(2, p);
        }
    });
    true
}

/// Table of 64-byte pivot entries blended by 0x242c18 (the vectors sit at +0x10).
const PIVOT_TABLE: u32 = 0x17ffc0;

/// The pivot vector of a moby: entry (byte 0 of the stamp at +0x38) blended toward entry (byte 1) by byte 2 / 256.
/// Mirrors the VU0 registers retail leaves: vf1 (from), vf2 (to), vf3 (weights), ACC.xyz and vf1 (blend).
pub fn pivot_blend(c: &mut Cpu, m: u32) -> V {
    let stamp = c.r32(m + 0x38);
    let (b0, b1, b2) = (stamp & 0xff, (stamp >> 8) & 0xff, (stamp >> 16) & 0xff);
    let from = c.rv(PIVOT_TABLE + (b0 << 6) + 0x10);
    c.set_vf(1, from);
    if b2 == 0 {
        return from;
    }
    let to = c.rv(PIVOT_TABLE + (b1 << 6) + 0x10);
    c.set_vf(2, to);
    let x = V([b2 << 4, 0, 0, 0]).itof(12).x();
    let w = F::ONE.vu_sub(x);
    c.set_vf(3, V([x.0, 0, 0, w.0]));
    let acc = c.acc().with(from * w, XYZ);
    c.set_acc(acc);
    let v = from.with(V::madd(acc, to, V::splat(x)), XYZ);
    c.set_vf(1, v);
    v
}

/// 0x242c18 pivot_blend_hand(m, out): out = pivot_blend(m).
pub fn pivot_blend_hand(c: &mut Cpu) -> bool {
    let (m, out) = (c.arg(0), c.arg(1));
    let v = pivot_blend(c, m);
    c.wv(out, v);
    true
}

/// Light direction scale and the constant z component written by 0x242470.
const LIGHT_SCALE: F = F(0x3e0f_5c29);
const LIGHT_Z: F = F(0xbf7d_70a4);
const LIGHT_DIR: u32 = 0x1aec80;
const LIGHT_MOBY: u32 = 0x1413d0;

/// 0x242470 light_dir_update(): from the pivot of the moby at *0x1413d0, direction (sin a, cos a, -0.99) with
/// a = atan2(pivot.x, pivot.y) and the x and y parts scaled by 0.1407.
pub fn light_dir_update(c: &mut Cpu) -> bool {
    let m = c.r32(LIGHT_MOBY);
    let v = pivot_blend(c, m);
    with_stack(c, 0x40, |c, _| {
        c.set_farg(1, v.y());
        callf(c, 0x1f1e48, &[], v.x());
        let a = c.fresult();
        callf(c, 0x1f1d80, &[], a);
        let x = c.fresult() * LIGHT_SCALE;
        c.wf(LIGHT_DIR, x);
        callf(c, 0x1f1d98, &[], a);
        let y = c.fresult() * LIGHT_SCALE;
        c.wf(LIGHT_DIR + 8, LIGHT_Z);
        c.wf(LIGHT_DIR + 4, y);
        c.fret(y);
        c.f[1] = LIGHT_Z.0; // retail leaves the z constant in f1 / $at
        c.s(1, LIGHT_Z.0 as i32 as i64 as u64);
    });
    true
}

/// Draw-record lists: indices (negative-terminated) at 0x19a700, moby table at 0x197400, patch table at 0x199f80.
const PATCH_LIST: u32 = 0x19a700;
const PATCH_MOBYS: u32 = 0x197400;
const PATCH_TABLE: u32 = 0x199f80;

fn patch_word(c: &mut Cpu, at: u32, bits: u32) {
    let w = c.r32(at);
    c.w32(at, (w & 0xffff_c000) | bits);
}

/// 0x242398 patch_draw_records(): for every listed moby, ORs the two 16-bit patch values of each byte code (patch
/// table, 4 bytes per code) into words +0x30 / +0x40 of the successive 0x40-byte draw records.
pub fn patch_draw_records(c: &mut Cpu) -> bool {
    let mut list = PATCH_LIST;
    if (c.r32(list) as i32) < 0 {
        return true;
    }
    loop {
        let idx = c.r32(list);
        list += 4;
        let moby = c.r32(PATCH_MOBYS + 4 * idx);
        let mut group = c.r32(moby + 0x20);
        loop {
            let mut rec = c.r32(group + 0xc) & 0x7fff_ffff;
            let mut code = group;
            if c.r8(code) != 0xff {
                loop {
                    let e = PATCH_TABLE + 4 * c.r8(code) as u32;
                    let (p0, p1) = (c.r16(e) as i16 as i32 as u32, c.r16(e + 2) as i16 as i32 as u32);
                    if p0 != 0 {
                        patch_word(c, rec + 0x30, p0);
                    }
                    code += 1;
                    if p1 != 0 {
                        patch_word(c, rec + 0x40, p1);
                    }
                    if c.r8(code) == 0xff {
                        break;
                    }
                    rec += 0x40;
                }
            }
            if (c.r32(group + 0xc) as i32) < 0 {
                break;
            }
            group += 0x10;
        }
        if (c.r32(list) as i32) < 0 {
            return true;
        }
    }
}
