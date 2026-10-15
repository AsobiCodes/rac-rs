//! Shadow-blob drawing (Eudora overlay): a rotated ring of points kept in the scratchpad, clipped, and emitted as
//! GIF packets into the packet buffer whose write pointer lives at `GIF_PTR`.
use crate::ps2::*;
use crate::h::{call_keep, with_stack};
use ee_oracle::cpu::Cpu;

/// Scratchpad base: ring points, two quads (inner, outer) per point.
const SPR: u32 = 0x7000_0000;
/// Scratchpad: per-point signed flag bytes of the first pass (compared with the pass argument).
const SPR_FLAGS_A: u32 = 0x7000_3000;
/// Scratchpad: per-point flag byte pairs of the second and third pass.
const SPR_FLAGS_B: u32 = 0x7000_3800;
/// Packet-buffer write pointer.
const GIF_PTR: u32 = 0x16_1140;
/// Number of ring points in the scratchpad.
const POINT_COUNT: u32 = 0x16_03e0;
/// Plane values for the second / third pass.
const PLANE_LO: u32 = 0x16_03e8;
const PLANE_HI: u32 = 0x16_03ec;
/// Ring axis (normalised), perpendicular start vector and scaled axis.
const AXIS: u32 = 0x16_03b0;
const START: u32 = 0x16_03c0;
const CENTRE: u32 = 0x16_03d0;
/// gp-relative globals.
const GP_SCRATCH: i64 = -0x6840;
const GP_TABLE: i64 = -0x6860;
const GP_PTR_SAVE: i64 = -0x5ac0;

fn gp(c: &Cpu, off: i64) -> u32 {
    (c.g(28) as i64).wrapping_add(off) as u32
}

/// 0x2795c0 shadow_clip_z(lo, hi): clips every ring point pair in the scratchpad to the z range [lo.z, hi.z].
pub fn shadow_clip_z(c: &mut Cpu) -> bool {
    let (lo, hi) = (c.rv(c.arg(0)), c.rv(c.arg(1)));
    clip_z(c, lo, hi);
    true
}

/// Masked VU write of x, y, z (lane i = `f(i)`); the w lane keeps `base`'s value.
fn with_xyz(base: V, f: impl Fn(usize) -> F) -> V {
    let mut out = base.0;
    for (i, lane) in out.iter_mut().enumerate().take(3) {
        *lane = f(i).0;
    }
    V(out)
}

/// Sign-extends a 32-bit value into a 64-bit GPR (as mfc1 does).
fn sx32(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// A 128-bit GPR as the four VU lanes (lane 0 lowest), and back.
fn vf_from_quad(q: u128) -> V {
    V(std::array::from_fn(|i| (q >> (32 * i)) as u32))
}

fn quad_of(v: V) -> u128 {
    v.0.iter().enumerate().fold(0u128, |q, (i, &l)| q | (l as u128) << (32 * i))
}

/// Retail `mfc1 $at, $f0; qmtc2 $at, vf1; vcallms entry; qmfc2 $at, vf1` on the angle: runs the VU0 micro program at
/// `entry` (0xc90 sin, 0xc80 cos) and returns its vf1.x. VF / ACC are left as the micro program leaves them.
fn vu_micro_angle(c: &mut Cpu, angle: F, entry: u32) -> F {
    c.s(1, sx32(angle.0));
    c.set_vf(1, vf_from_quad(c.r[1]));
    let _ = c.vu0_micro(entry);
    c.r[1] = quad_of(c.vf(1));
    c.vf(1).lane(0)
}

/// 0x2795c0 clip body: per point pair, clips the segment to the z planes `lo` (vf10) and `hi` (vf11). Leaves
/// vf1..vf5, vf10, vf11 and vf20 as the retail loop does (vf20.xyz = 1, w kept; vf3.w / vf4.w / vf5.w unchanged).
fn clip_z(c: &mut Cpu, lo: V, hi: V) {
    let vf0 = c.vf(0);
    let one = vf0.w();
    c.set_vf(20, with_xyz(c.vf(20), |i| vf0.lane(i).vu_add(one)));
    c.set_vf(10, lo);
    c.set_vf(11, hi);
    let mut left = c.r32(POINT_COUNT);
    let mut p = SPR;
    loop {
        let (a, b) = (c.rv(p), c.rv(p + 0x10));
        c.set_vf(1, a);
        c.set_vf(2, b);
        let d = with_xyz(c.vf(3), |i| b.lane(i).vu_sub(a.lane(i)));
        c.set_vf(3, d);
        // vdiv Q, vf0w, vf3z; vsub.z offsets; vmulz.xyz by the offset; vmulq.xyz by Q
        let q = one.q_div(d.z());
        let (za, zb) = (lo.z().vu_sub(a.z()), hi.z().vu_sub(b.z()));
        let da = [0, 1, 2].map(|i| d.lane(i).vu_mul(za).vu_mul(q));
        let db = [0, 1, 2].map(|i| d.lane(i).vu_mul(zb).vu_mul(q));
        c.set_vf(4, with_xyz(c.vf(4), |i| da[i]));
        c.set_vf(5, with_xyz(c.vf(5), |i| db[i]));
        let n1 = with_xyz(a, |i| a.lane(i).vu_add(da[i]));
        let n2 = with_xyz(b, |i| b.lane(i).vu_add(db[i]));
        c.set_vf(1, n1);
        c.set_vf(2, n2);
        c.wv(p, n1);
        c.wv(p + 0x10, n2);
        left = left.wrapping_sub(1);
        p += 0x20;
        if left == 0 {
            return;
        }
    }
}

/// One GIF packet of the single-quad/// One GIF packet of the single-quad points copied from the scratchpad: ring entries whose flag byte (two bytes per
/// point, starting at `flags`) is zero; `first` is the quad offset inside each ring entry.
fn emit_points(c: &mut Cpu, tex: u32, first: u32, flags: u32) -> Option<u128> {
    let count = c.r32(POINT_COUNT);
    let base = c.r32(GIF_PTR);
    let mut out = base + 0x40;
    let mut n = 0u32;
    let mut last = None;
    for i in 0..count {
        if c.r8(flags + 2 * i) as i8 == 0 {
            let q = c.rv(SPR + first + 0x20 * i);
            last = Some(upper_half(q));
            c.wv(out, q);
            n += 1;
            out += 0x10;
        }
    }
    if n == 0 {
        return None;
    }
    let words = [
        0x1000_0000 | (n + 3), 0, 0, 0x5000_0000 | (n + 3),
        1, 0x2400_0000, 1, 0,
        tex, 0x3f80_0000, 0x55, 0,
        0x8000 | n, 0x2400_0000, 0x52, 0,
    ];
    for (i, w) in words.into_iter().enumerate() {
        c.w32(base + 4 * i as u32, w);
    }
    c.w32(GIF_PTR, base + (n + 4) * 16);
    last
}

/// A quad's z / w lanes as the upper doubleword of a 128-bit GPR (the retail code copies quads through `$at`, and the
/// stale upper half of `$at` later ends up in memory via VU0 transfers).
fn upper_half(q: V) -> u128 {
    (q.0[3] as u128) << 96 | (q.0[2] as u128) << 64
}

/// The pass sign test: +-1 (by `side`) times the plane value, as the VU multiplies it.
fn plane_sign(c: &mut Cpu, plane: u32, side: u32) -> F {
    let sign = if side == 2 { F::ZERO.vu_sub(F::ONE) } else { F::ZERO.vu_add(F::ONE) };
    sign.vu_mul(c.rf(plane))
}

/// 0x279638 shadow_ring_emit(side, tex): emits the ring entries whose flag byte equals `side` (4 quads each), then
/// the single-quad points of the second and third pass. Returns what the retail code leaves in the upper half of `$at`.
pub fn shadow_ring_emit(c: &mut Cpu, side: u32, tex: u32) -> Option<u128> {
    let mut left = c.r32(POINT_COUNT).wrapping_sub(1);
    if left == 0 {
        return None;
    }
    let base = c.r32(GIF_PTR);
    let mut out = base + 0x40;
    let mut n = 0u32;
    let mut p = SPR;
    let mut flag = SPR_FLAGS_A;
    while left != 0 {
        let f = c.r8(flag) as i8 as i32;
        left -= 1;
        flag += 1;
        if f == side as i32 {
            c.w32(out, 0);
            c.w32(out + 4, 0);
            c.w32(out + 8, 0x54);
            c.w32(out + 12, 0);
            for k in 0..4 {
                let q = c.rv(p + 0x10 * k);
                c.wv(out + 0x10 + 0x10 * k, q);
            }
            out += 0x50;
            n += 1;
        }
        p += 0x20;
    }
    let tag = ((out - base) >> 4).wrapping_sub(1);
    let words = [
        0x1000_0000 | tag, 0, 0, 0x5000_0000 | tag,
        1, 0x1400_0000, 0xf1, 0,
        tex, 0x3f80_0000, 0, 0,
        0x8000 | n, 0xa400_0000, 0x5252_520f, 0x52,
    ];
    for (i, w) in words.into_iter().enumerate() {
        c.w32(base + 4 * i as u32, w);
    }
    c.w32(GIF_PTR, out);
    if !plane_sign(c, PLANE_LO, side).neg_bit() {
        emit_points(c, tex, 0, SPR_FLAGS_B);
    }
    let hi = plane_sign(c, PLANE_HI, side);
    // $at ends as vf1 (qmfc2 after each plane test) unless the last pass copied quads through it.
    let mut upper = upper_half(V(c.vf[1]).with(V([0; 4]), X | Y));
    if hi.0 as i32 <= 0 {
        if let Some(q) = emit_points(c, tex, 0x10, SPR_FLAGS_B + 1) {
            upper = q;
        }
    }
    Some(upper)
}

/// 0x2781e0 shadow_draw(side, table_idx, entry): writes a DMA ref tag for entry `entry`, then emits the ring.
pub fn shadow_draw(c: &mut Cpu) -> bool {
    let (side, idx, entry) = (c.arg(0), c.arg(1), c.arg(2));
    draw_entry(c, side, idx, entry);
    true
}

fn draw_entry(c: &mut Cpu, side: u32, idx: u32, entry: u32) -> Option<u128> {
    let base = c.r32(GIF_PTR);
    c.w32(base, 0x3000_0003);
    c.w32(base + 4, entry.wrapping_mul(0x30).wrapping_add(0x1b_bf90));
    c.w32(base + 8, 0x1300_0000);
    c.w32(base + 12, 0x5000_0003);
    let tex = c.r32(gp(c, GP_TABLE).wrapping_add(idx << 2));
    let save = gp(c, GP_PTR_SAVE);
    c.w32(save, base + 0x10);
    shadow_ring_emit(c, side, tex)
}

/// 0x278058 shadow_axis_setup(v, scale): normalises v into AXIS, builds the perpendicular start vector in START and
/// the scaled axis in CENTRE.
pub fn shadow_axis_setup(c: &mut Cpu) -> bool {
    let v = c.rv(c.arg(0));
    let scale = c.farg(0);
    axis_setup(c, v, scale);
    true
}

fn axis_setup(c: &mut Cpu, v: V, scale: F) {
    // The retail frame (0x30 bytes) holds v for the retail set_length; call_keep leaves its VU0 state as retail does.
    with_stack(c, 0x30, |c, buf| {
        c.wv(buf, v);
        call_keep(c, 0x1f1a98, &[AXIS, buf], &[F::ONE]);
        axis_body(c, v, scale);
    });
}

fn axis_body(c: &mut Cpu, v: V, scale: F) {
    for i in 0..3 {
        let a = c.rf(AXIS + 4 * i);
        c.wf(CENTRE + 4 * i, a * scale);
    }
    let (x, y, z) = (v.x(), v.y(), v.z());
    let gp_scratch = gp(c, GP_SCRATCH);
    // START is a permutation of v's components, picked by their order (the middle one goes to gp_scratch when the
    // permutation leaves START.x unwritten).
    if x < y {
        if x < z {
            c.wf(START, x);
            c.wf(START + 4, z);
            c.wf(START + 8, y);
        } else {
            c.wf(gp_scratch, y);
            c.wf(START + 4, x);
            c.wf(START + 8, z);
        }
    } else if y < z {
        c.wf(START, z);
        c.wf(START + 4, y);
        c.wf(START + 8, x);
    } else {
        c.wf(gp_scratch, y);
        c.wf(START + 4, x);
        c.wf(START + 8, z);
    }
    // START = START x AXIS (retail cross3), then normalised in place (retail set_length).
    call_keep(c, 0x1f1958, &[START, START, AXIS], &[]);
    call_keep(c, 0x1f1a98, &[START, START], &[F::ONE]);
}

/// 0x278e20 shadow_ring_build(desc): rotates START about AXIS in n equal steps (desc[1] = n, desc+0x10 = centre
/// offset) and writes inner / outer ring points to the scratchpad; stores n + 1 in POINT_COUNT.
pub fn shadow_ring_build(c: &mut Cpu) -> bool {
    let d = c.arg(0);
    ring_build(c, d)
}

fn ring_build(c: &mut Cpu, d: u32) -> bool {
    let n = c.r32(d + 4) as i32;
    if n <= 0 {
        return false;
    }
    let centre = c.rv(d + 0x10);
    let outer = c.rv(CENTRE);
    let axis = c.rv(AXIS);
    let start = c.rv(START);
    c.set_vf(27, centre);
    c.set_vf(28, outer);
    // vsub.xyz vf30, vf0, vf30: the negated axis (w kept)
    let vf0 = c.vf(0);
    c.set_vf(30, with_xyz(axis, |i| vf0.lane(i).vu_sub(axis.lane(i))));
    // f0 = 0x40c8f5c3 / n (the angle); sin and cos come from the VU0 micro programs, into vf31.x and vf31.y
    let angle = F(0x40c8_f5c3) / F::from_int(n);
    c.fret(angle);
    let sin = vu_micro_angle(c, angle, 0xc90);
    let mut s = c.vf(31);
    s.0[0] = c.vf(0).lane(0).vu_add(sin).0;
    c.set_vf(31, s);
    let cos = vu_micro_angle(c, angle, 0xc80);
    let mut s = c.vf(31);
    s.0[1] = c.vf(0).lane(1).vu_add(cos).0;
    s.0[3] = c.vf(0).lane(3).vu_sub(s.lane(1)).0;
    c.set_vf(31, s);
    // vf29 = vf30 * sin (xyz); vf1..vf3 = the rotation rows, (A_i * A_j) * (1 - cos) plus the cos / sin terms
    let s = c.vf(31);
    let a = c.vf(30);
    let (sin_f, cos_f, w_f) = (s.x(), s.y(), s.w());
    let b = with_xyz(c.vf(29), |i| a.lane(i).vu_mul(sin_f));
    c.set_vf(29, b);
    let (ax, ay, az) = (a.x(), a.y(), a.z());
    let (bx, by, bz) = (b.x(), b.y(), b.z());
    let k = |p: F, q: F| p.vu_mul(q).vu_mul(w_f);
    let row1 = [k(ax, ax).vu_add(cos_f), k(ay, ax).vu_sub(bz), k(az, ax).vu_add(by)];
    let row2 = [k(ax, ay).vu_add(bz), k(ay, ay).vu_add(cos_f), k(az, ay).vu_sub(bx)];
    let row3 = [k(ax, az).vu_sub(by), k(ay, az).vu_add(bx), k(az, az).vu_add(cos_f)];
    c.set_vf(1, with_xyz(c.vf(1), |i| row1[i]));
    c.set_vf(2, with_xyz(c.vf(2), |i| row2[i]));
    c.set_vf(3, with_xyz(c.vf(3), |i| row3[i]));
    // the ring loop: vf26 = point, ACC = vf1 * x + vf2 * y (ACC.w kept), vf26 = ACC + vf3 * z
    let (r1, r2, r3) = (c.vf(1), c.vf(2), c.vf(3));
    let acc_base = c.acc();
    let mut partial = acc_base;
    let mut p = start.with(start * centre.w(), XYZ);
    let mut v5 = c.vf(5);
    let mut t2 = SPR;
    for _ in 0..n {
        let acc = V::madd(r1 * p.x(), r2, V::splat(p.y()));
        partial = acc;
        p = p.with(V::madd(acc, r3, V::splat(p.z())), XYZ);
        v5 = v5.with(p + centre, XYZ);
        c.wv(t2, v5);
        v5 = v5.with(v5 + outer, XYZ);
        c.wv(t2 + 0x10, v5);
        t2 += 0x20;
    }
    c.set_vf(26, p);
    c.set_acc(with_xyz(acc_base, |i| partial.lane(i)));
    let (q0, q1) = (c.rv(SPR), c.rv(SPR + 0x10));
    c.wv(t2, q0);
    c.wv(t2 + 0x10, q1);
    c.set_vf(5, q1);
    c.r[1] = c.r[1] & !(u64::MAX as u128) | POINT_COUNT as u128;
    c.w32(POINT_COUNT, n as u32 + 1);
    true
}

/// 0x278188 shadow_build(desc) -> next desc: dispatches on desc[0] (0 ring, 1 other shape); returns the pointer past
/// the descriptor.
pub fn shadow_build(c: &mut Cpu) -> bool {
    let d = c.arg(0);
    // Kind 0 (shadow_ring_build) is not reproduced here: retail runs the whole call, so it leaves no host-side RAM write.
    if c.r16(d) as i16 == 0 {
        return false;
    }
    let s0 = c.r[16];
    match build_entry(c, d) {
        Some(next) => {
            // `move $v0, $s0` copies the whole register; s0 (saved and restored by retail) keeps its upper half.
            c.r[2] = s0 & !(u64::MAX as u128) | next as i32 as i64 as u64 as u128;
            true
        }
        None => false,
    }
}

fn build_entry(c: &mut Cpu, d: u32) -> Option<u32> {
    let kind = c.r16(d) as i16;
    let next = match kind {
        0 => {
            call_keep(c, 0x278e20, &[d], &[]);
            d + 0x20
        }
        1 => {
            call_keep(c, 0x278f70, &[d], &[]);
            // The retail callee may copy its return address (0x2781c4) into $t8; a host call links the sentinel 0x10
            // instead, so put the retail value back where that happened.
            if c.r[24] as u32 == 0x10 {
                c.r[24] = c.r[24] & !(u64::MAX as u128) | 0x2781c4;
            }
            d + 0x30
        }
        _ => d,
    };
    Some(next)
}

/// Screen-space size (x, y) halfwords of the shadow rectangle, in the overlay's globals.
const RECT_SIZE: u32 = 0x15_1780 + 0x150;
const PRIM_BITS: u64 = 0x9000 << 46;

fn w64(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a + 4, (v >> 32) as u32);
}

/// 0x2913c8 gs_reg_write(reg, value): appends an A+D GIF packet that sets one GS register.
pub fn gs_reg_write(c: &mut Cpu, reg: u32, value: u64) {
    let p = c.r32(GIF_PTR);
    c.w32(p, 0x1000_0002);
    c.w32(p + 4, 0);
    c.w32(p + 8, 0);
    c.w32(p + 12, 0x5000_0002);
    c.w32(p + 0x10, 0x8001);
    c.w32(p + 0x14, 0x1000_0000);
    c.w32(p + 0x18, 0xe);
    c.w32(p + 0x1c, 0);
    w64(c, p + 0x20, value);
    c.w32(p + 0x28, reg);
    c.w32(p + 0x2c, 0);
    c.w32(GIF_PTR, p + 0x30);
}

/// Writes the vertex pairs of a rectangle packet: `rows` (top, bottom) doublewords at `p + 0x60`.
fn rect_rows(c: &mut Cpu, p: u32, rows: i32, x: i32, y: i32) {
    let top = ((0x8000 - y * 8) as i64 as u64) << 16;
    let bottom_y = ((y * 8 + 0x7ff0) as i64 as u64) << 16;
    let left = -(x * 8);
    let mut a = left.wrapping_add(0x8000);
    let mut b = left.wrapping_add(0x8200);
    let mut out = p + 0x60;
    for _ in 0..rows {
        w64(c, out, a as i64 as u64 | top);
        w64(c, out + 8, b as i64 as u64 | bottom_y);
        out += 16;
        b = b.wrapping_add(0x200);
        a = a.wrapping_add(0x200);
    }
}

/// The rectangle size, and its row count (size.x / 32, rounded toward zero).
fn rect_size(c: &mut Cpu) -> (i32, i32, i32) {
    let x = c.r16(RECT_SIZE) as i16 as i32;
    let y = c.r16(RECT_SIZE + 2) as i16 as i32;
    (x, y, (if x > -1 { x } else { x + 0x1f }) >> 5)
}

/// The DMA / VIF tag and the GIF block shared by both rectangle packets, written at the buffer pointer; returns the
/// block start (`base + 0x10`). `tail` is the doubleword at block + 0x38 (the argument / constant), `last` the one at
/// + 0x40, `gif` the one at + 0x10.
fn rect_block(c: &mut Cpu, rows: i32, gif: u64, tail: u64, last: u64) -> u32 {
    let base = c.r32(GIF_PTR);
    let qwc = (rows + 5) as u32;
    c.w32(base, qwc | 0x1000_0000);
    c.w32(base + 4, 0);
    c.w32(base + 8, 0);
    c.w32(base + 12, qwc | 0x5000_0000);
    let b = base + 0x10;
    c.w32(GIF_PTR, b);
    w64(c, b, 0x1000_0000_0000_0001);
    w64(c, b + 0x48, 0x44);
    w64(c, b + 8, 0xe);
    w64(c, b + 0x10, gif);
    w64(c, b + 0x18, 0x47);
    w64(c, b + 0x38, tail);
    w64(c, b + 0x40, last);
    b
}

/// 0x278270 shadow_rect_draw(): emits the shadow rectangle packet (header, GIF block and `size.x / 32` rows).
pub fn shadow_rect_draw(c: &mut Cpu) -> bool {
    rect_draw(c);
    true
}

fn rect_draw(c: &mut Cpu) {
    gs_reg_write(c, 0x42, 0x64);
    let (x, y, rows) = rect_size(c);
    let b = rect_block(c, rows, 0x3_1001, 0x7f80_8080, PRIM_BITS | rows as u64 | 0x8000);
    w64(c, b + 0x20, PRIM_BITS | 0x8001);
    w64(c, b + 0x28, 0x10);
    w64(c, b + 0x30, 0x146);
    if rows > 0 {
        rect_rows(c, b - 0x10, rows, x, y);
    }
    let end = c.r32(GIF_PTR) + (rows as u32) * 16 + 0x50;
    c.w32(GIF_PTR, end);
}

/// 0x278418 shadow_rect_end(a0): like `shadow_rect_draw` with a0 as the packet's constant and a closing empty
/// packet.
pub fn shadow_rect_end(c: &mut Cpu) -> bool {
    let a0 = c.arg(0) as i32 as i64 as u64;
    rect_end(c, a0);
    true
}

fn rect_end(c: &mut Cpu, a0: u64) {
    let (x, y, rows) = rect_size(c);
    let b = rect_block(c, rows, 0x3_d801, a0, PRIM_BITS | rows as u64 | 0x8000);
    w64(c, b + 0x20, PRIM_BITS | 1);
    w64(c, b + 0x28, 0x10);
    w64(c, b + 0x30, 0x146);
    if rows > 0 {
        rect_rows(c, b - 0x10, rows, x, y);
    }
    let end = c.r32(GIF_PTR) + (rows as u32) * 16 + 0x50;
    c.w32(GIF_PTR, end);
    c.w32(end, 0x1000_0000);
    c.w32(end + 4, 0);
    c.w32(end + 8, 0x1300_0000);
    c.w32(end + 12, 0);
    c.w32(GIF_PTR, end + 0x10);
}

/// Global table of model lists (indexed by the entry values of `MODEL_LIST`).
const MODEL_TABLE: u32 = 0x1c_0000 - 0x3ed0;
/// Signed word list of model indices, terminated by a negative entry.
const MODEL_LIST: u32 = 0x1c_0000 - 0x3050;
/// Per-material (u16 low-bits, u16 high-bits) pairs, indexed by the vertex-group id byte.
const MATERIAL_BITS: u32 = 0x1c_0000 - 0x3550;

/// 0x279ad0 shadow_patch_models(): for every model in `MODEL_LIST`, ORs the material bits into the two tag words of
/// each of its vertex groups (keeping the top 18 bits of each word).
pub fn shadow_patch_models(c: &mut Cpu) -> bool {
    const KEEP: u32 = 0xffff_c000;
    let mut list = MODEL_LIST;
    if (c.r32(list) as i32) < 0 {
        return true;
    }
    while (c.r32(list) as i32) >= 0 {
        let entry = c.r32(list);
        let model = c.r32(MODEL_TABLE + entry * 4);
        list += 4;
        let mesh_count = c.r16(model + 0x28) as i16 as i32;
        let mut i = 0;
        while i < mesh_count || i == 0 && mesh_count > 0 {
            let mesh = c.r32(model + 0x40 + i as u32 * 8);
            let head = mesh + 0x10;
            let n = c.r32(head) as i32;
            if n > 0 {
                let mut group = head + (c.r32(head + 4) << 4) + 0x10;
                let mut k = 0;
                loop {
                    let id = c.r8(group + 0x13) as u32;
                    let bits = MATERIAL_BITS + id * 4;
                    let lo = c.r16(bits) as i16 as i32 as u32;
                    if lo != 0 {
                        let w = c.r32(group + 0x30);
                        c.w32(group + 0x30, w & KEEP | lo);
                    }
                    let hi = c.r16(bits + 2) as i16 as i32 as u32;
                    if hi != 0 {
                        let w = c.r32(group + 0x20);
                        c.w32(group + 0x20, w & KEEP | hi);
                    }
                    k += 1;
                    group += 0x40;
                    if k >= c.r32(head) as i32 {
                        break;
                    }
                }
            }
            i += 1;
            if i >= c.r16(model + 0x28) as i16 as i32 {
                break;
            }
        }
    }
    true
}

/// 0x279990 shadow_end_chain(): terminates the DMA chain (end tag, ref back to the chain head) and records the chain
/// size when the stats flag is on.
pub fn shadow_end_chain(c: &mut Cpu) -> bool {
    const END_TAG: u32 = 0x2000_0000;
    const STATS: u32 = 0x17_0000 - 0x5f10;
    const CHAIN_HEAD: u32 = 0x16_0430;
    let regs = c.r;
    let s1 = c.r32(GIF_PTR);
    c.w32(GIF_PTR, s1 + 0x10);
    let head = c.r32(CHAIN_HEAD);
    c.w32(head, END_TAG);
    let p = c.r32(GIF_PTR);
    c.w32(head + 4, p);
    c.w32(head + 8, 0);
    c.w32(head + 12, 0);
    if c.r32(STATS + 0x20) != 0 && c.r32(STATS + 0x1c) != 0 {
        let arg = gp(c, -0x7d8c);
        let arg = c.r32(arg);
        c.set_arg(0, arg);
        c.set_arg(1, END_TAG);
        c.set_arg(2, END_TAG);
        c.set_arg(3, STATS);
        c.call(0x27b3d0);
        let s0 = c.result() as i32;
        c.call(0x2915b0);
        if 0x40_0000 < s0 {
            c.set_arg(0, 0x1e_0000 - 0x4ff0);
            c.call(0x1dd878);
        }
        if (c.r32(0x16_0438) as i32) < s0 {
            c.w32(gp(c, -0x67c8), s0 as u32);
        }
    }
    let p = c.r32(GIF_PTR);
    c.w32(p, END_TAG);
    let head = c.r32(CHAIN_HEAD);
    c.w32(p + 4, head + 0x10);
    c.w32(p + 8, 0);
    c.w32(p + 12, 0);
    c.w32(GIF_PTR, p + 0x10);
    c.w32(s1, END_TAG);
    c.w32(s1 + 4, p + 0x10);
    c.w32(s1 + 8, 0);
    c.w32(s1 + 12, 0);
    c.r = regs;
    true
}

/// 0x2785e8 shadow_draw_all(desc): draws every shadow descriptor in the list starting at `desc` (terminated by a zero
/// first word), then closes the rectangle packet and restores the GS state registers.
pub fn shadow_draw_all(c: &mut Cpu) -> bool {
    const CLIP_FLAG: u32 = 0x16_0000 - 0x1280;
    let (regs, fregs) = (c.r, c.f);
    let mut d = c.arg(0);
    rect_draw(c);
    let p = c.r32(GIF_PTR);
    c.w32(p, 0x3000_0007);
    let model = if c.r32(CLIP_FLAG) != 0 { 0x1c_0000 - 0x3f40 } else { 0x1c_0000 - 0x3fb0 };
    c.w32(p + 4, model);
    c.w32(p + 8, 0x1300_0000);
    c.w32(p + 12, 0x5000_0007);
    c.w32(GIF_PTR, p + 0x10);
    while c.r32(d) != 0 {
        let head = d;
        let count = c.r32(head) as i32;
        let v = c.rv(head + 0x10);
        d += 0x20;
        axis_setup(c, v, F(0x447a_0000));
        let (z_lo, z_hi) = (c.rf(head + 0xc), c.rf(head + 8));
        let mut done = 0;
        while count > 0 {
            d = match build_entry(c, d) {
                Some(next) => next,
                None => return false,
            };
            done += 1;
            clip_z(c, V([0, 0, z_lo.0, 0]), V([0, 0, z_hi.0, 0]));
            c.set_arg(0, SPR);
            let (n, kind) = (c.r32(POINT_COUNT), c.r32(head + 4));
            c.set_arg(1, n);
            c.set_arg(2, kind);
            c.call(0x278aa8);
            let mut upper = None;
            for (side, idx) in [(2, 1), (1, 0)] {
                upper = draw_entry(c, side, idx, 2).or(upper);
            }
            if let Some(u) = upper {
                c.r[1] = c.r[1] & u64::MAX as u128 | u;
            }
            if done >= c.r32(head) as i32 {
                break;
            }
        }
    }
    let mode = if c.r32(CLIP_FLAG) != 0 { 0x8_0080 } else { 0x8_0070 };
    gs_reg_write(c, 0x4c, mode);
    gs_reg_write(c, 0x42, (0x8000 << 22) | 0x64);
    rect_end(c, 0);
    gs_reg_write(c, 0x47, 0x5_360b);
    gs_reg_write(c, 0x42, (0x8000 << 24) | 0x44);
    c.r = regs;
    c.f = fregs;
    true
}
