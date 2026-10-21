// GS capture on the GPU (gpu.rs): the capture's pixel work (draw.rs raster, gscap.rs transfers / local copies / CLUT loads,
// the overlay planes) as compute kernels. PC speed-up of the capture, no retail counterpart. Every value is the one the CPU
// path computes: the CPU's f32 operations are IEEE single precision, one rounding each and no fused multiply-add, which GPU
// float hardware does not promise (fusion, reassociation, flushed denormals, inexact division), so each f32 operation of
// the pixel pipeline (`fadd` ...) takes the hardware only for operands where it is exact, and otherwise integer arithmetic
// (the `sf_` functions, round to nearest even with subnormals, x86 NaN rules) on the operands' bit patterns.

struct Params { w: array<vec4<u32>, 16> }

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read_write> vram: array<atomic<u32>>;
@group(0) @binding(2) var<storage, read_write> ovl: array<u32>;
@group(0) @binding(3) var<storage, read_write> clut: array<u32>;
@group(0) @binding(4) var<storage, read> lut: array<u32>;
@group(0) @binding(5) var<storage, read> prims: array<u32>;
@group(0) @binding(6) var<storage, read> tiles: array<u32>;
@group(0) @binding(7) var<storage, read> dat: array<u32>;
// The sharp HUD's planes (lane G, gpu.rs `hd_*`, the `k_hd` block at the end of this file): plane p pixel i is two words, f16 (r, g) and
// (b, a); the stage plane (plane 6) is RGBA8, one word.
@group(0) @binding(8) var<storage, read_write> hdp: array<u32>;

fn pw(i: u32) -> u32 {
    return P.w[i >> 2u][i & 3u];
}

// ---- soft f32 (bit patterns) ----

const ONE: u32 = 0x3f800000u;
const HALF: u32 = 0x3f000000u;
const F128: u32 = 0x43000000u;
const F255: u32 = 0x437f0000u;
const NAN_DEF: u32 = 0xffc00000u;

fn is_nan(a: u32) -> bool {
    return (a & 0x7fffffffu) > 0x7f800000u;
}

fn nan2(a: u32, b: u32) -> u32 {
    if (is_nan(a)) {
        return a | 0x400000u;
    }
    return b | 0x400000u;
}

fn jam(a: u32, d: u32) -> u32 {
    if (d == 0u) {
        return a;
    }
    if (d < 31u) {
        return (a >> d) | select(0u, 1u, (a << (32u - d)) != 0u);
    }
    return select(0u, 1u, a != 0u);
}

fn pack(s: u32, e: i32, sig: u32) -> u32 {
    return (s << 31u) + (bitcast<u32>(e) << 23u) + sig;
}

fn round_pack(s: u32, e0: i32, sig0: u32) -> u32 {
    var e = e0;
    var sig = sig0;
    var rb = sig & 0x7fu;
    if (bitcast<u32>(e) >= 0xfdu) {
        if (e < 0) {
            sig = jam(sig, bitcast<u32>(-e));
            e = 0;
            rb = sig & 0x7fu;
        } else if (e > 0xfd || sig + 0x40u >= 0x80000000u) {
            return pack(s, 0xff, 0u);
        }
    }
    sig = (sig + 0x40u) >> 7u;
    if (rb == 0x40u) {
        sig = sig & ~1u;
    }
    if (sig == 0u) {
        e = 0;
    }
    return pack(s, e, sig);
}

// a + b: the larger magnitude first, the smaller aligned with a sticky bit, six extra bits, one rounding. (NaNs: draw.rs / x86
// rules: the first NaN operand quieted, else the second; inf - inf the default NaN.)
fn sf_add(a: u32, b: u32) -> u32 {
    let aa = a & 0x7fffffffu;
    let ab = b & 0x7fffffffu;
    if (aa > 0x7f800000u || ab > 0x7f800000u) {
        return nan2(a, b);
    }
    if (aa >= 0x7f800000u || ab >= 0x7f800000u) {
        if (aa >= 0x7f800000u && ab >= 0x7f800000u) {
            return select(NAN_DEF, a, a == b);
        }
        return select(b, a, aa >= 0x7f800000u);
    }
    var x = a;
    var y = b;
    if (ab > aa) {
        x = b;
        y = a;
    }
    let s = x >> 31u;
    let sub = ((x ^ y) >> 31u) != 0u;
    var ex = (x >> 23u) & 0xffu;
    var ey = (y >> 23u) & 0xffu;
    var fx = x & 0x7fffffu;
    var fy = y & 0x7fffffu;
    if (ex != 0u) {
        fx = fx | 0x800000u;
    } else {
        ex = 1u;
    }
    if (ey != 0u) {
        fy = fy | 0x800000u;
    } else {
        ey = 1u;
    }
    let xx = fx << 6u;
    let yy = jam(fy << 6u, ex - ey);
    let sm = select(xx + yy, xx - yy, sub);
    if (sm == 0u) {
        return select(s << 31u, 0u, sub);
    }
    let p = 31u - countLeadingZeros(sm);
    return round_pack(s, i32(ex + p) - 30, sm << (30u - p));
}

// a - b is a + (-b) (a NaN b keeps its sign).
fn sf_sub(a: u32, b: u32) -> u32 {
    return sf_add(a, select(b ^ 0x80000000u, b, is_nan(b)));
}

// (lo, hi) of a 32 x 32 bit product.
fn mul64(a: u32, b: u32) -> vec2<u32> {
    let a0 = a & 0xffffu;
    let a1 = a >> 16u;
    let b0 = b & 0xffffu;
    let b1 = b >> 16u;
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 16u) + (p01 & 0xffffu) + (p10 & 0xffffu);
    return vec2<u32>((p00 & 0xffffu) | (mid << 16u), p11 + (p01 >> 16u) + (p10 >> 16u) + (mid >> 16u));
}

// (exponent, significand) of a subnormal significand, normalised.
fn norm_sub(sig: u32) -> vec2<u32> {
    let sh = countLeadingZeros(sig) - 8u;
    return vec2<u32>(bitcast<u32>(1 - i32(sh)), sig << sh);
}

fn sf_mul(a: u32, b: u32) -> u32 {
    let s = (a ^ b) >> 31u;
    var ea = i32((a >> 23u) & 0xffu);
    var sa = a & 0x7fffffu;
    var eb = i32((b >> 23u) & 0xffu);
    var sb = b & 0x7fffffu;
    if (ea == 0xff) {
        if (sa != 0u || (eb == 0xff && sb != 0u)) {
            return nan2(a, b);
        }
        if ((u32(eb) | sb) == 0u) {
            return NAN_DEF;
        }
        return pack(s, 0xff, 0u);
    }
    if (eb == 0xff) {
        if (sb != 0u) {
            return nan2(a, b);
        }
        if ((u32(ea) | sa) == 0u) {
            return NAN_DEF;
        }
        return pack(s, 0xff, 0u);
    }
    if (ea == 0) {
        if (sa == 0u) {
            return s << 31u;
        }
        let n = norm_sub(sa);
        ea = bitcast<i32>(n.x);
        sa = n.y;
    }
    if (eb == 0) {
        if (sb == 0u) {
            return s << 31u;
        }
        let n = norm_sub(sb);
        eb = bitcast<i32>(n.x);
        sb = n.y;
    }
    var ez = ea + eb - 0x7f;
    let p = mul64((sa | 0x00800000u) << 7u, (sb | 0x00800000u) << 8u);
    var sz = p.y | select(0u, 1u, p.x != 0u);
    if (sz < 0x40000000u) {
        ez = ez - 1;
        sz = sz << 1u;
    }
    return round_pack(s, ez, sz);
}

fn sf_div(a: u32, b: u32) -> u32 {
    let s = (a ^ b) >> 31u;
    var ea = i32((a >> 23u) & 0xffu);
    var sa = a & 0x7fffffu;
    var eb = i32((b >> 23u) & 0xffu);
    var sb = b & 0x7fffffu;
    if (ea == 0xff) {
        if (sa != 0u) {
            return nan2(a, b);
        }
        if (eb == 0xff) {
            if (sb != 0u) {
                return nan2(a, b);
            }
            return NAN_DEF;
        }
        return pack(s, 0xff, 0u);
    }
    if (eb == 0xff) {
        if (sb != 0u) {
            return nan2(a, b);
        }
        return s << 31u;
    }
    if (eb == 0) {
        if (sb == 0u) {
            if ((u32(ea) | sa) == 0u) {
                return NAN_DEF;
            }
            return pack(s, 0xff, 0u);
        }
        let n = norm_sub(sb);
        eb = bitcast<i32>(n.x);
        sb = n.y;
    }
    if (ea == 0) {
        if (sa == 0u) {
            return s << 31u;
        }
        let n = norm_sub(sa);
        ea = bitcast<i32>(n.x);
        sa = n.y;
    }
    var ez = ea - eb + 0x7e;
    sa = sa | 0x00800000u;
    sb = sb | 0x00800000u;
    var k = 30u;
    if (sa < sb) {
        ez = ez - 1;
        k = 31u;
    }
    // q = floor(sa * 2^k / sb), long division.
    var q = 0u;
    var r = sa;
    if (r >= sb) {
        q = 1u;
        r = r - sb;
    }
    for (var i = 0u; i < k; i++) {
        r = r << 1u;
        q = q << 1u;
        if (r >= sb) {
            r = r - sb;
            q = q | 1u;
        }
    }
    q = q | select(0u, 1u, r != 0u);
    return round_pack(s, ez, q);
}

// a < b, false when either is NaN (IEEE).
fn sf_lt(a: u32, b: u32) -> bool {
    if (is_nan(a) || is_nan(b)) {
        return false;
    }
    if (((a | b) << 1u) == 0u) {
        return false;
    }
    let sa = a >> 31u;
    let sb = b >> 31u;
    if (sa != sb) {
        return sa == 1u;
    }
    if (sa == 0u) {
        return a < b;
    }
    return a > b;
}

fn sf_eq(a: u32, b: u32) -> bool {
    if (is_nan(a) || is_nan(b)) {
        return false;
    }
    return a == b || ((a | b) << 1u) == 0u;
}

fn sf_le(a: u32, b: u32) -> bool {
    return sf_lt(a, b) || sf_eq(a, b);
}

// Rust f32::min(x, c) / max(x, c) with a constant c: c when x is NaN.
fn sf_min(x: u32, c: u32) -> u32 {
    return select(c, x, sf_lt(x, c));
}

fn sf_max(x: u32, c: u32) -> u32 {
    return select(c, x, sf_lt(c, x));
}

// Rust f32::clamp(lo, hi).
fn sf_clamp(x: u32, lo: u32, hi: u32) -> u32 {
    var v = x;
    if (sf_lt(v, lo)) {
        v = lo;
    }
    if (sf_lt(hi, v)) {
        v = hi;
    }
    return v;
}

// Rust `x as i32` (truncation, saturating, NaN 0).
fn sf_to_i32(a: u32) -> i32 {
    if (is_nan(a)) {
        return 0;
    }
    let x = i32((a >> 23u) & 0xffu) - 127;
    let neg = (a >> 31u) != 0u;
    if (x < 0) {
        return 0;
    }
    if (x >= 31) {
        return select(2147483647, -2147483647 - 1, neg);
    }
    let m = (a & 0x7fffffu) | 0x800000u;
    var v: u32;
    if (x >= 23) {
        v = m << u32(x - 23);
    } else {
        v = m >> u32(23 - x);
    }
    return select(i32(v), -i32(v), neg);
}

// Rust `x as u32` (truncation, saturating, NaN and negatives 0).
fn sf_to_u32(a: u32) -> u32 {
    if (is_nan(a) || (a >> 31u) != 0u) {
        return 0u;
    }
    let x = i32((a >> 23u) & 0xffu) - 127;
    if (x < 0) {
        return 0u;
    }
    if (x >= 32) {
        return 0xffffffffu;
    }
    let m = (a & 0x7fffffu) | 0x800000u;
    if (x >= 23) {
        return m << u32(x - 23);
    }
    return m >> u32(23 - x);
}

// Rust `x.round() as i32` (half away from zero).
fn sf_round_i32(a: u32) -> i32 {
    if (is_nan(a)) {
        return 0;
    }
    let x = i32((a >> 23u) & 0xffu) - 127;
    let neg = (a >> 31u) != 0u;
    if (x < -1) {
        return 0;
    }
    if (x == -1) {
        return select(1, -1, neg);
    }
    if (x >= 31) {
        return select(2147483647, -2147483647 - 1, neg);
    }
    let m = (a & 0x7fffffu) | 0x800000u;
    var v: u32;
    if (x >= 23) {
        v = m << u32(x - 23);
    } else {
        let fb = u32(23 - x);
        v = (m + (1u << (fb - 1u))) >> fb;
    }
    return select(i32(v), -i32(v), neg);
}

// draw.rs `ffloor`.
fn sf_ffloor(a: u32) -> u32 {
    if (((a >> 23u) & 0xffu) >= 150u) {
        return a;
    }
    let t = bitcast<u32>(f32(sf_to_i32(a)));
    if (sf_lt(a, t)) {
        return sf_sub(t, ONE);
    }
    return t;
}

// ---- f32 operations on the GPU's own float unit where that is exact ----
//
// Vulkan and D3D12 give correctly rounded (round to nearest even) add, subtract and multiply; what they do not promise
// is denormals (inputs and results may be flushed to zero) and the rounding count (a product fed to an add may be
// fused into one multiply-add). An operand that is zero or has an exponent field in [64, 254] keeps every input and
// result of an add, subtract or multiply normal or exactly zero (a product of two such is at least 2^-126; a sum or
// difference is a multiple of the smaller operand's unit, at least 2^-86, or zero), so flushing cannot show; the OR
// with a zero word the compiler cannot see through (`zr`) keeps each result rounded on its own. Other operands
// (denormals, infinities, NaNs) take the integer path. Division is the hardware quotient corrected on its exact
// integer remainder. `SOFT` (gpu.rs: when the start-up self test finds a difference) forces the integer path.

override SOFT: bool = false;

// The k_raster pipeline: an operand the hardware path cannot take marks the pixel (`bad`) instead of running the integer
// path, which then is not in the kernel at all (its registers and code made every pixel about 2.5 times slower); k_redo
// draws such a tile again from that prim on with the integer path.
override FAST: bool = false;

var<private> bad: bool;

fn zr() -> u32 {
    return P.w[15].w;
}

// A loop bound `n` the driver cannot see through (n + the zero word) in the kernels with the integer path: a loop of a few
// iterations over float operations is not unrolled there, so each operation's integer path is built once instead of n
// times. The kernels with FAST keep the constant bound (unrolled, as before).
fn lb(n: u32) -> u32 {
    if (FAST) {
        return n;
    }
    return n + zr();
}

fn okf(x: u32) -> bool {
    return !SOFT && ((((x >> 23u) & 0xffu) - 64u) < 191u || (x << 1u) == 0u);
}

fn fadd(a: u32, b: u32) -> u32 {
    if (okf(a) && okf(b)) {
        return bitcast<u32>(bitcast<f32>(a) + bitcast<f32>(b)) | zr();
    }
    if (FAST) {
        bad = true;
        return 0u;
    }
    return sf_add(a, b);
}

fn fsub(a: u32, b: u32) -> u32 {
    if (okf(a) && okf(b)) {
        return bitcast<u32>(bitcast<f32>(a) - bitcast<f32>(b)) | zr();
    }
    if (FAST) {
        bad = true;
        return 0u;
    }
    return sf_sub(a, b);
}

fn fmul(a: u32, b: u32) -> u32 {
    if (okf(a) && okf(b)) {
        return bitcast<u32>(bitcast<f32>(a) * bitcast<f32>(b)) | zr();
    }
    if (FAST) {
        bad = true;
        return 0u;
    }
    return sf_mul(a, b);
}

fn fdiv(a: u32, b: u32) -> u32 {
    // x / 128 is x * 2^-7 bit for bit (both round the same exact value).
    if (b == F128) {
        return fmul(a, 0x3c000000u);
    }
    let ea = (a >> 23u) & 0xffu;
    let eb = (b >> 23u) & 0xffu;
    // 0 / a finite non-zero: a zero with the sign of the quotient.
    if (!SOFT && (a << 1u) == 0u && eb - 1u < 254u) {
        return (a ^ b) & 0x80000000u;
    }
    if (!SOFT && ea - 1u < 254u && eb - 1u < 254u) {
        let ma = (a & 0x7fffffu) | 0x800000u;
        let mb = (b & 0x7fffffu) | 0x800000u;
        var e = i32(ea) - i32(eb) + 127;
        var sh = 23u;
        if (ma < mb) {
            e = e - 1;
            sh = 24u;
        }
        if (e >= 1 && e <= 253) {
            // q = floor(ma * 2^sh / mb), in [2^23, 2^24): the hardware quotient is a few units off at most.
            var q = u32((f32(ma) / f32(mb)) * bitcast<f32>((127u + sh) << 23u));
            let xl = ma << sh;
            let xh = ma >> (32u - sh);
            let p = mul64(q, mb);
            let rh = xh - p.y - select(0u, 1u, xl < p.x);
            var r = bitcast<i32>(xl - p.x);
            // The remainder fits an i32 when the high word is the sign extension of the low one.
            if (rh == select(0u, 0xffffffffu, r < 0)) {
                for (var n = 0u; n < 8u; n++) {
                    if (r < 0) {
                        q = q - 1u;
                        r = r + i32(mb);
                    } else if (r >= i32(mb)) {
                        q = q + 1u;
                        r = r - i32(mb);
                    } else {
                        break;
                    }
                }
                if (r >= 0 && r < i32(mb) && q >= 0x800000u && q < 0x1000000u) {
                    let r2 = u32(r) * 2u;
                    if (r2 > mb || (r2 == mb && (q & 1u) != 0u)) {
                        q = q + 1u;
                    }
                    if (q == 0x1000000u) {
                        q = 0x800000u;
                        e = e + 1;
                    }
                    return ((a ^ b) & 0x80000000u) | (u32(e) << 23u) | (q & 0x7fffffu);
                }
            }
        }
    }
    if (FAST) {
        bad = true;
        return 0u;
    }
    return sf_div(a, b);
}

// draw.rs `ffloor` on the bit pattern.
fn fffloor(a: u32) -> u32 {
    let e = (a >> 23u) & 0xffu;
    if (e >= 150u) {
        return a;
    }
    let neg = (a >> 31u) != 0u;
    if (e < 127u) {
        return select(0u, 0xbf800000u, neg && (a << 1u) != 0u);
    }
    let t = a & (0xffffffffu << (150u - e));
    if (neg && t != a) {
        return fsub(t, ONE);
    }
    return t;
}

// An integer (|i| < 2^24) as f32 bits.
fn fi(i: i32) -> u32 {
    return bitcast<u32>(f32(i));
}

fn fu(i: u32) -> u32 {
    return bitcast<u32>(f32(i));
}

// ---- local memory ----

fn page_w(bw: u32) -> u32 {
    return max(bw / 2u, 1u);
}

fn a32(bp: u32, bw: u32, x: u32, y: u32) -> u32 {
    let page = (y / 32u) * bw + x / 64u;
    return (bp * 256u + page * 8192u + lut[(y % 32u) * 64u + x % 64u]) & 0x3fffffu;
}

fn a16(bp: u32, bw: u32, x: u32, y: u32) -> u32 {
    let page = (y / 64u) * bw + x / 64u;
    return (bp * 256u + page * 8192u + lut[2048u + (y % 64u) * 64u + x % 64u]) & 0x3fffffu;
}

fn a8(bp: u32, bw: u32, x: u32, y: u32) -> u32 {
    let page = (y / 64u) * page_w(bw) + x / 128u;
    return (bp * 256u + page * 8192u + lut[6144u + (y % 64u) * 128u + x % 128u]) & 0x3fffffu;
}

// Nibble address.
fn a4(bp: u32, bw: u32, x: u32, y: u32) -> u32 {
    let page = (y / 128u) * page_w(bw) + x / 128u;
    return (bp * 512u + page * 16384u + lut[14336u + (y % 128u) * 128u + x % 128u]) & 0x7fffffu;
}

fn zaddr(bp: u32, bw: u32, x: u32, y: u32) -> u32 {
    let page = (y / 32u) * bw + x / 64u;
    return (bp * 256u + page * 8192u + lut[30720u + (y % 32u) * 64u + x % 64u]) & 0x3fffffu;
}

fn rd32(a: u32) -> u32 {
    return atomicLoad(&vram[a >> 2u]);
}

fn rd8(a: u32) -> u32 {
    return (atomicLoad(&vram[a >> 2u]) >> ((a & 3u) * 8u)) & 0xffu;
}

fn rd16(a: u32) -> u32 {
    return (atomicLoad(&vram[a >> 2u]) >> ((a & 3u) * 8u)) & 0xffffu;
}

// Writes the bits `m << sh` of word `w` (only this pixel owns them: other pixels' bits of the word are untouched).
fn wr_bits(w: u32, m: u32, sh: u32, v: u32) {
    atomicAnd(&vram[w], ~(m << sh));
    atomicOr(&vram[w], (v & m) << sh);
}

fn rd_px(psm: u32, bp: u32, bw: u32, x: u32, y: u32) -> u32 {
    switch psm {
        case 0x00u: {
            return rd32(a32(bp, bw, x, y));
        }
        case 0x01u: {
            return rd32(a32(bp, bw, x, y)) & 0xffffffu;
        }
        case 0x02u: {
            return rd16(a16(bp, bw, x, y));
        }
        case 0x13u: {
            return rd8(a8(bp, bw, x, y));
        }
        case 0x14u: {
            let n = a4(bp, bw, x, y);
            return (rd8(n >> 1u) >> ((n & 1u) * 4u)) & 0xfu;
        }
        case 0x1bu: {
            return rd8(a32(bp, bw, x, y) + 3u);
        }
        case 0x24u: {
            return rd8(a32(bp, bw, x, y) + 3u) & 0xfu;
        }
        case 0x2cu: {
            return rd8(a32(bp, bw, x, y) + 3u) >> 4u;
        }
        default: {
            return 0u;
        }
    }
}

fn wr_px(psm: u32, bp: u32, bw: u32, x: u32, y: u32, v: u32) {
    switch psm {
        case 0x00u: {
            atomicStore(&vram[a32(bp, bw, x, y) >> 2u], v);
        }
        case 0x01u: {
            wr_bits(a32(bp, bw, x, y) >> 2u, 0xffffffu, 0u, v);
        }
        case 0x02u: {
            let a = a16(bp, bw, x, y);
            wr_bits(a >> 2u, 0xffffu, (a & 3u) * 8u, v);
        }
        case 0x13u: {
            let a = a8(bp, bw, x, y);
            wr_bits(a >> 2u, 0xffu, (a & 3u) * 8u, v);
        }
        case 0x14u: {
            let n = a4(bp, bw, x, y);
            let a = n >> 1u;
            wr_bits(a >> 2u, 0xfu, (a & 3u) * 8u + (n & 1u) * 4u, v);
        }
        case 0x1bu: {
            wr_bits(a32(bp, bw, x, y) >> 2u, 0xffu, 24u, v);
        }
        case 0x24u: {
            wr_bits(a32(bp, bw, x, y) >> 2u, 0xfu, 24u, v);
        }
        case 0x2cu: {
            wr_bits(a32(bp, bw, x, y) >> 2u, 0xfu, 28u, v);
        }
        default: {}
    }
}

// gscap.rs `unpack_col` (integer channels).
fn unpack_col(psm: u32, v: u32, texa_lo: u32, texa_hi: u32) -> vec4<u32> {
    if (psm == 0x02u || psm == 0x0au) {
        let rgb0 = (v & 0x7fffu) == 0u;
        var a = texa_lo & 0xffu;
        if ((v & 0x8000u) != 0u) {
            a = texa_hi & 0xffu;
        } else if ((texa_lo & 0x8000u) != 0u && rgb0) {
            a = 0u;
        }
        return vec4<u32>((v & 0x1fu) << 3u, ((v >> 5u) & 0x1fu) << 3u, ((v >> 10u) & 0x1fu) << 3u, a);
    }
    if (psm == 0x01u) {
        var a = texa_lo & 0xffu;
        if ((texa_lo & 0x8000u) != 0u && (v & 0xffffffu) == 0u) {
            a = 0u;
        }
        return vec4<u32>(v & 0xffu, (v >> 8u) & 0xffu, (v >> 16u) & 0xffu, a);
    }
    return vec4<u32>(v & 0xffu, (v >> 8u) & 0xffu, (v >> 16u) & 0xffu, v >> 24u);
}

fn f4(c: vec4<u32>) -> vec4<u32> {
    return vec4<u32>(fu(c.x), fu(c.y), fu(c.z), fu(c.w));
}

fn clut_rd(slot: u32, i: u32) -> vec4<u32> {
    let v = clut[slot * 512u + i];
    return vec4<u32>(v & 0xffu, (v >> 8u) & 0xffu, (v >> 16u) & 0xffu, v >> 24u);
}

// ---- prims (records written by gpu.rs `Prim`) ----

const STRIDE: u32 = 80u;
const R_KIND: u32 = 0u;
const R_PLANE: u32 = 2u;
const R_SLOT: u32 = 3u;
const R_BB: u32 = 4u;
const R_AT: u32 = 8u;
const R_TEX0: u32 = 9u;
const R_TEX1: u32 = 11u;
const R_CLAMP: u32 = 12u;
const R_ALPHA: u32 = 14u;
const R_TEST: u32 = 16u;
const R_FRAME: u32 = 17u;
const R_FBA: u32 = 19u;
const R_TEXA: u32 = 20u;
const R_FOGCOL: u32 = 22u;
const R_COLCLAMP: u32 = 23u;
const R_PABE: u32 = 24u;
const R_CLUTOFF: u32 = 25u;
const R_OV: u32 = 26u;
const R_ZB: u32 = 27u;
const R_G: u32 = 33u;
const R_PX: u32 = 42u;

struct Px {
    rgba: vec4<u32>,
    s: u32,
    t: u32,
    q: u32,
    u: u32,
    v: u32,
    f: u32,
    z: u32,
}

// Channel `m` of a vertex value: 0-3 colour, 4 s, 5 t, 6 q, 7 u, 8 v, 9 fog, 10 z.
fn px_get(p: Px, m: u32) -> u32 {
    switch m {
        case 0u, 1u, 2u, 3u: {
            return p.rgba[m];
        }
        case 4u: {
            return p.s;
        }
        case 5u: {
            return p.t;
        }
        case 6u: {
            return p.q;
        }
        case 7u: {
            return p.u;
        }
        case 8u: {
            return p.v;
        }
        case 9u: {
            return p.f;
        }
        default: {
            return p.z;
        }
    }
}

fn px_with(p: Px, m: u32, v: u32) -> Px {
    var o = p;
    switch m {
        case 0u, 1u, 2u, 3u: {
            o.rgba[m] = v;
        }
        case 4u: {
            o.s = v;
        }
        case 5u: {
            o.t = v;
        }
        case 6u: {
            o.q = v;
        }
        case 7u: {
            o.u = v;
        }
        case 8u: {
            o.v = v;
        }
        case 9u: {
            o.f = v;
        }
        default: {
            o.z = v;
        }
    }
    return o;
}

fn rd_vtx(pb: u32, k: u32) -> Px {
    let b = pb + R_PX + k * 11u;
    return Px(vec4<u32>(prims[b], prims[b + 1u], prims[b + 2u], prims[b + 3u]), prims[b + 4u], prims[b + 5u], prims[b + 6u], prims[b + 7u], prims[b + 8u], prims[b + 9u], prims[b + 10u]);
}

fn wrapc(m: u32, c: i32, size: u32, lo: u32, hi: u32) -> u32 {
    switch m & 3u {
        case 0u: {
            return bitcast<u32>(c) & (size - 1u);
        }
        case 1u: {
            return u32(clamp(c, 0, i32(size) - 1));
        }
        case 2u: {
            return u32(max(min(max(c, i32(lo)), i32(hi)), 0));
        }
        default: {
            return (bitcast<u32>(c) & lo) | hi;
        }
    }
}

fn tex_w(pb: u32) -> u32 {
    return 1u << min((prims[pb + R_TEX0] >> 26u) & 0xfu, 10u);
}

fn tex_h(pb: u32) -> u32 {
    return 1u << min(((prims[pb + R_TEX0] >> 30u) | (prims[pb + R_TEX0 + 1u] << 2u)) & 0xfu, 10u);
}

// draw.rs `View::texel_slow` (channels as f32 bits).
fn texel(pb: u32, u: i32, v: i32) -> vec4<u32> {
    let t0 = prims[pb + R_TEX0];
    let cl = prims[pb + R_CLAMP];
    let clh = prims[pb + R_CLAMP + 1u];
    let x = wrapc(cl, u, tex_w(pb), (cl >> 4u) & 0x3ffu, (cl >> 14u) & 0x3ffu);
    let y = wrapc(cl >> 2u, v, tex_h(pb), ((cl >> 24u) | (clh << 8u)) & 0x3ffu, (clh >> 2u) & 0x3ffu);
    let psm = (t0 >> 20u) & 0x3fu;
    let raw = rd_px(psm, t0 & 0x3fffu, (t0 >> 14u) & 0x3fu, x, y);
    if (psm == 0x13u || psm == 0x1bu) {
        return f4(clut_rd(prims[pb + R_SLOT], raw & 0xffu));
    }
    if (psm == 0x14u || psm == 0x24u || psm == 0x2cu) {
        return f4(clut_rd(prims[pb + R_SLOT], (raw & 0xfu) + prims[pb + R_CLUTOFF]));
    }
    return f4(unpack_col(psm, raw, prims[pb + R_TEXA], prims[pb + R_TEXA + 1u]));
}

fn lerp(a: u32, b: u32, f: u32) -> u32 {
    return fadd(a, fmul(fsub(b, a), f));
}

// The texture value at (u, v): the nearest texel, or the bilinear blend of four (draw.rs `View::shade`). The four fetches
// are one loop and the 12 lerps one lerp in a loop, so the kernels build one texel read and one lerp, not four and twelve.
fn fetch(pb: u32, u0: u32, v0: u32) -> vec4<u32> {
    let bil = (prims[pb + R_TEX1] & 0x20u) != 0u;
    var u = u0;
    var v = v0;
    if (bil) {
        u = fsub(u0, HALF);
        v = fsub(v0, HALF);
    }
    let fx0 = fffloor(u);
    let fy0 = fffloor(v);
    let x0 = sf_to_i32(fx0);
    let y0 = sf_to_i32(fy0);
    var t00: vec4<u32>;
    var t10: vec4<u32>;
    var t01: vec4<u32>;
    var t11: vec4<u32>;
    for (var j = 0u; j < select(1u, 4u, bil); j++) {
        let tt = texel(pb, x0 + i32(j & 1u), y0 + i32(j >> 1u));
        if (j == 0u) {
            t00 = tt;
        } else if (j == 1u) {
            t10 = tt;
        } else if (j == 2u) {
            t01 = tt;
        } else {
            t11 = tt;
        }
    }
    if (!bil) {
        return t00;
    }
    let fx = fsub(u, fx0);
    let fy = fsub(v, fy0);
    var o: vec4<u32>;
    for (var k = 0u; k < lb(4u); k++) {
        var a = 0u;
        var b = 0u;
        for (var s = 0u; s < lb(3u); s++) {
            var l0 = a;
            var l1 = b;
            var f = fy;
            if (s == 0u) {
                l0 = t00[k];
                l1 = t10[k];
                f = fx;
            } else if (s == 1u) {
                l0 = t01[k];
                l1 = t11[k];
                f = fx;
            }
            let r = lerp(l0, l1, f);
            if (s == 0u) {
                a = r;
            } else if (s == 1u) {
                b = r;
            } else {
                o[k] = r;
            }
        }
    }
    return o;
}

struct Sh {
    ok: bool,
    c: vec4<u32>,
    wa: bool,
}

// draw.rs `View::shade`.
fn shade(pb: u32, p: Px) -> Sh {
    let at = prims[pb + R_AT];
    var c = p.rgba;
    if ((at & 0x10u) != 0u) {
        let t1 = prims[pb + R_TEX0 + 1u];
        var u: u32;
        var v: u32;
        if ((at & 0x100u) != 0u) {
            u = p.u;
            v = p.v;
        } else {
            let q = select(p.q, ONE, ((p.q << 1u) == 0u));
            u = fmul(fdiv(p.s, q), fu(tex_w(pb)));
            v = fmul(fdiv(p.t, q), fu(tex_h(pb)));
        }
        let t = fetch(pb, u, v);
        let tcc = (t1 & 4u) != 0u;
        let tfx = (t1 >> 3u) & 3u;
        if (tfx == 1u) {
            c[0] = t[0];
            c[1] = t[1];
            c[2] = t[2];
            if (tcc) {
                c[3] = t[3];
            }
        } else {
            // 0 modulate (alpha too with TCC), 2 / 3 highlight (3: highlight 2, texture alpha as is).
            let a = c[3];
            for (var k = 0u; k < lb(select(3u, 4u, tfx == 0u && tcc)); k++) {
                var m = fdiv(fmul(t[k], c[k]), F128);
                if (tfx != 0u) {
                    m = fadd(m, a);
                }
                c[k] = sf_min(m, F255);
            }
            if (tfx != 0u) {
                if (tcc) {
                    c[3] = select(t[3], sf_min(fadd(t[3], a), F255), tfx == 2u);
                } else {
                    c[3] = a;
                }
            }
        }
    }
    if ((at & 0x20u) != 0u) {
        let fc = prims[pb + R_FOGCOL];
        let f = fdiv(p.f, F255);
        let omf = fsub(ONE, f);
        for (var k = 0u; k < lb(3u); k++) {
            c[k] = fadd(fmul(c[k], f), fmul(fu((fc >> (8u * k)) & 0xffu), omf));
        }
    }
    let test = prims[pb + R_TEST];
    var wa = true;
    if ((test & 1u) != 0u) {
        let aref = fu((test >> 4u) & 0xffu);
        let a = fffloor(c[3]);
        var ok = false;
        switch (test >> 1u) & 7u {
            case 0u: {
                ok = false;
            }
            case 1u: {
                ok = true;
            }
            case 2u: {
                ok = sf_lt(a, aref);
            }
            case 3u: {
                ok = sf_le(a, aref);
            }
            case 4u: {
                ok = sf_eq(a, aref);
            }
            case 5u: {
                ok = sf_le(aref, a);
            }
            case 6u: {
                ok = sf_lt(aref, a);
            }
            default: {
                ok = !sf_eq(a, aref);
            }
        }
        if (!ok) {
            let fl = (test >> 12u) & 3u;
            if (fl == 0u || fl == 2u) {
                return Sh(false, c, wa);
            }
            if (fl == 3u) {
                wa = false;
            }
        }
    }
    return Sh(true, c, wa);
}

fn sel_cd(s: u32, cs: u32, cd: u32) -> u32 {
    switch s & 3u {
        case 0u: {
            return cs;
        }
        case 1u: {
            return cd;
        }
        default: {
            return 0u;
        }
    }
}

// A pixel's writes (`commit`): k bit 0 overlay plane words `o..o + 4` = `c`, bit 1 the frame pixel, bit 2 the Z value.
struct Wr {
    k: u32,
    o: u32,
    c: vec4<u32>,
    psm: u32,
    bp: u32,
    bw: u32,
    x: u32,
    y: u32,
    v: u32,
    za: u32,
    zi: u32,
    z24: bool,
}

fn commit(w: Wr) {
    if ((w.k & 1u) != 0u) {
        ovl[w.o] = w.c[0];
        ovl[w.o + 1u] = w.c[1];
        ovl[w.o + 2u] = w.c[2];
        ovl[w.o + 3u] = w.c[3];
    }
    if ((w.k & 2u) != 0u) {
        wr_px(w.psm, w.bp, w.bw, w.x, w.y, w.v);
    }
    if ((w.k & 4u) != 0u) {
        if (w.z24) {
            wr_bits(w.za >> 2u, 0xffffffu, 0u, w.zi);
        } else {
            atomicStore(&vram[w.za >> 2u], w.zi);
        }
    }
}

// draw.rs `View::put` (its writes; nothing is written here).
fn put(pb: u32, x: i32, y: i32, c: vec4<u32>, wa: bool, z: u32) -> Wr {
    var w: Wr;
    let at = prims[pb + R_AT];
    let al = prims[pb + R_ALPHA];
    let fix = fu(prims[pb + R_ALPHA + 1u] & 0xffu);
    let pabe = (prims[pb + R_PABE] & 1u) != 0u && sf_lt(c[3], F128);
    let blend = (at & 0x40u) != 0u && !pabe;
    if (prims[pb + R_OV] != 0u) {
        if (x >= 512 || y >= 512) {
            return w;
        }
        let o = ((prims[pb + R_PLANE] * 262144u) + u32(y) * 512u + u32(x)) * 4u;
        w.k = 1u;
        w.o = o;
        if (!blend) {
            w.c = vec4<u32>(c[0], c[1], c[2], 0u);
            return w;
        }
        let a = al & 3u;
        let b = (al >> 2u) & 3u;
        let d = (al >> 6u) & 3u;
        var cf: u32;
        switch (al >> 4u) & 3u {
            case 0u: {
                cf = fdiv(c[3], F128);
            }
            case 1u: {
                cf = ONE;
            }
            default: {
                cf = fdiv(fix, F128);
            }
        }
        let ks = fadd(fsub(select(0u, cf, a == 0u), select(0u, cf, b == 0u)), select(0u, ONE, d == 0u));
        let kd = fadd(fsub(select(0u, cf, a == 1u), select(0u, cf, b == 1u)), select(0u, ONE, d == 1u));
        if (sf_lt(ks, 0u)) {
            w.k = 0u;
            return w;
        }
        for (var k = 0u; k < lb(3u); k++) {
            w.c[k] = fadd(fmul(ks, c[k]), fmul(kd, ovl[o + k]));
        }
        w.c[3] = fmul(ovl[o + 3u], kd);
        return w;
    }
    let ux = u32(x);
    let uy = u32(y);
    var zw = false;
    var za = 0u;
    var zi = 0u;
    var z24 = false;
    if ((prims[pb + R_ZB + 5u]) != 0u) {
        let zbp = prims[pb + R_ZB];
        let zbw = prims[pb + R_ZB + 1u];
        z24 = prims[pb + R_ZB + 2u] != 0u;
        let zmsk = prims[pb + R_ZB + 3u] != 0u;
        let tst = prims[pb + R_ZB + 4u];
        zi = min(sf_to_u32(sf_max(z, 0u)), select(0xffffffffu, 0xffffffu, z24));
        za = zaddr(zbp, zbw, ux, uy);
        let old = rd32(za) & select(0xffffffffu, 0xffffffu, z24);
        var zpass: bool;
        switch tst {
            case 0u: {
                zpass = false;
            }
            case 1u: {
                zpass = true;
            }
            case 2u: {
                zpass = zi >= old;
            }
            default: {
                zpass = zi > old;
            }
        }
        if (!zpass) {
            return w;
        }
        if (!zmsk && wa) {
            zw = true;
        }
    }
    let fr = prims[pb + R_FRAME];
    let msk = prims[pb + R_FRAME + 1u];
    let fbp = (fr & 0x1ffu) * 32u;
    let fbw = (fr >> 16u) & 0x3fu;
    let psm = (fr >> 24u) & 0x3fu;
    let dst = rd_px(psm, fbp, fbw, ux, uy);
    let dc = f4(unpack_col(psm, dst, prims[pb + R_TEXA], prims[pb + R_TEXA + 1u]));
    var out = c;
    if (blend) {
        var cf: u32;
        switch (al >> 4u) & 3u {
            case 0u: {
                cf = c[3];
            }
            case 1u: {
                cf = dc[3];
            }
            default: {
                cf = fix;
            }
        }
        for (var k = 0u; k < lb(3u); k++) {
            let v = fadd(fdiv(fmul(fsub(sel_cd(al, c[k], dc[k]), sel_cd(al >> 2u, c[k], dc[k])), cf), F128), sel_cd(al >> 6u, c[k], dc[k]));
            if ((prims[pb + R_COLCLAMP] & 1u) != 0u) {
                out[k] = sf_clamp(v, 0u, F255);
            } else {
                out[k] = fi(sf_to_i32(v) & 0xff);
            }
        }
    }
    var a = select(dc[3], out[3], wa);
    if ((prims[pb + R_FBA] & 1u) != 0u) {
        a = sf_max(a, F128);
    }
    var v: u32;
    let b0 = sf_to_u32(sf_clamp(out[0], 0u, F255));
    let b1 = sf_to_u32(sf_clamp(out[1], 0u, F255));
    let b2 = sf_to_u32(sf_clamp(out[2], 0u, F255));
    if (psm == 0x02u || psm == 0x0au) {
        v = (b0 >> 3u) | ((b1 >> 3u) << 5u) | ((b2 >> 3u) << 10u) | (select(0u, 1u, sf_le(F128, a)) << 15u);
    } else {
        let b3 = sf_to_u32(sf_clamp(a, 0u, F255));
        v = b0 | (b1 << 8u) | (b2 << 16u) | (b3 << 24u);
    }
    v = (v & ~msk) | (dst & msk);
    w.k = 2u | select(0u, 4u, zw);
    w.psm = psm;
    w.bp = fbp;
    w.bw = fbw;
    w.x = ux;
    w.y = uy;
    w.v = v;
    w.za = za;
    w.zi = zi;
    w.z24 = z24;
    return w;
}

fn pixel(pb: u32, x: i32, y: i32, p: Px) -> Wr {
    var w: Wr;
    if (x < 0 || y < 0) {
        return w;
    }
    let s = shade(pb, p);
    if (s.ok) {
        w = put(pb, x, y, s.c, s.wa, p.z);
    }
    return w;
}

fn mix3(w0: u32, w1: u32, w2: u32, a: u32, b: u32, c: u32) -> u32 {
    return fadd(fadd(fmul(w0, a), fmul(w1, b)), fmul(w2, c));
}

// A pixel's shaded value (`prim_sv`): k 0 nothing to draw, 1 a write, 2 a write that leaves alpha (test fail mode 3).
struct Sv {
    k: u32,
    c: vec4<u32>,
    z: u32,
}

fn sv_px(pb: u32, x: i32, y: i32, p: Px) -> Sv {
    var v: Sv;
    if (x < 0 || y < 0) {
        return v;
    }
    let s = shade(pb, p);
    if (s.ok) {
        v = Sv(select(2u, 1u, s.wa), s.c, p.z);
    }
    return v;
}

// The pixel's writes for its shaded value (`View::put`).
fn wr_of(pb: u32, x: i32, y: i32, v: Sv) -> Wr {
    if (v.k == 0u) {
        var w: Wr;
        return w;
    }
    return put(pb, x, y, v.c, v.k == 1u, v.z);
}

// The vertex values of prim `pb` at (x, y), inside its bounding box (which the caller checked): `go` is false when the prim
// leaves the pixel alone. Not for lines. Every kind ends in the one `shade` call of the caller (shade is the big function:
// built once, not once per kind).
struct Pp {
    go: bool,
    p: Px,
}

fn prim_p(pb: u32, x: i32, y: i32) -> Pp {
    let kind = prims[pb + R_KIND];
    let at = prims[pb + R_AT];
    let g = pb + R_G;
    var w: Pp;
    switch kind {
        // Point: the pixel is the box.
        case 0u: {
            w.go = true;
            w.p = rd_vtx(pb, 0u);
        }
        // Line (prim_px draws them).
        case 1u: {
            bad = true;
        }
        // Sprite: the texture coordinates interpolate across the box (one loop over x, y).
        case 2u: {
            let pbv = rd_vtx(pb, 1u);
            w.go = true;
            w.p = pbv;
            if ((at & 0x10u) == 0u) {
                return w;
            }
            let pa = rd_vtx(pb, 0u);
            let uv = (at & 0x100u) != 0u;
            for (var a = 0u; a < lb(2u); a++) {
                let c0 = prims[g + a];
                let c1 = prims[g + 2u + a];
                var t = 0u;
                if (!sf_eq(c1, c0)) {
                    t = fdiv(fsub(fi(select(x, y, a == 1u)), c0), fsub(c1, c0));
                }
                let m = select(4u, 7u, uv) + a;
                w.p = px_with(w.p, m, lerp(px_get(pa, m), px_get(pbv, m), t));
            }
        }
        // Triangle.
        default: {
            let p0x = prims[g];
            let p0y = prims[g + 1u];
            let p1x = prims[g + 2u];
            let p1y = prims[g + 3u];
            let p2x = prims[g + 4u];
            let p2y = prims[g + 5u];
            let area = prims[pb + R_G + 6u];
            let neg = (area >> 31u) != 0u;
            let fx = fi(x);
            let fy = fi(y);
            // The edge functions of edges 1-2 and 2-0, then the weights; each in one loop.
            var wt = array<u32, 3>(0u, 0u, 0u);
            for (var e = 0u; e < lb(2u); e++) {
                let ax = select(p1x, p2x, e == 1u);
                let ay = select(p1y, p2y, e == 1u);
                let bx = select(p2x, p0x, e == 1u);
                let by = select(p2y, p0y, e == 1u);
                let n = fsub(fmul(fsub(ax, fx), fsub(by, fy)), fmul(fsub(bx, fx), fsub(ay, fy)));
                if ((n << 1u) != 0u && ((n >> 31u) != 0u) != neg) {
                    return w;
                }
                wt[e] = n;
            }
            for (var e = 0u; e < lb(2u); e++) {
                wt[e] = fdiv(wt[e], area);
            }
            wt[2] = fsub(fsub(ONE, wt[0]), wt[1]);
            for (var e = 0u; e < lb(3u); e++) {
                if (sf_lt(wt[e], 0u)) {
                    return w;
                }
            }
            let v0 = rd_vtx(pb, 0u);
            let v1 = rd_vtx(pb, 1u);
            let v2 = rd_vtx(pb, 2u);
            var q = v2;
            // Channels: 0-3 colour, 4-6 s t q, 7-8 u v, 9 fog, 10 z (px_get).
            for (var m = 0u; m < lb(11u); m++) {
                var en = false;
                if (m < 4u) {
                    en = (at & 8u) != 0u;
                } else if (m < 7u) {
                    en = (at & 0x100u) == 0u;
                } else if (m < 9u) {
                    en = (at & 0x100u) != 0u;
                } else if (m == 9u) {
                    en = (at & 0x20u) != 0u;
                } else {
                    en = prims[pb + R_ZB + 5u] != 0u;
                }
                if (en) {
                    q = px_with(q, m, mix3(wt[0], wt[1], wt[2], px_get(v0, m), px_get(v1, m), px_get(v2, m)));
                }
            }
            w.go = true;
            w.p = q;
        }
    }
    return w;
}

// The shaded value of prim `pb` at (x, y) (k_shade).
fn prim_sv(pb: u32, x: i32, y: i32) -> Sv {
    let pp = prim_p(pb, x, y);
    var v: Sv;
    if (pp.go) {
        v = sv_px(pb, x, y, pp.p);
    }
    return v;
}

// Shades and blends the pixel (x, y) from vertex values `p` and writes it (nothing, when the integer path is needed and the
// kernel has none: `bad`).
fn shade_put(pb: u32, x: i32, y: i32, p: Px) {
    if (x < 0 || y < 0) {
        return;
    }
    let s = shade(pb, p);
    if (s.ok) {
        let w = put(pb, x, y, s.c, s.wa, p.z);
        if (!bad) {
            commit(w);
        }
    }
}

// One pixel of prim `pb` (inside its bounding box, which the caller checked), written. Lines: every step that lands on
// (x, y), in order. One shade_put call serves points, sprites, triangles and lines.
fn prim_draw(pb: u32, x: i32, y: i32) {
    let line = prims[pb + R_KIND] == 1u;
    var cnt = 1u;
    let g = pb + R_G;
    let at = prims[pb + R_AT];
    var x0 = 0u;
    var y0 = 0u;
    var dx = 0u;
    var dy = 0u;
    var pa: Px;
    var pbv: Px;
    if (line) {
        if (FAST) {
            bad = true;
            return;
        }
        x0 = prims[g];
        y0 = prims[g + 1u];
        dx = fsub(prims[g + 2u], x0);
        dy = fsub(prims[g + 3u], y0);
        cnt = prims[g + 4u];
        pa = rd_vtx(pb, 0u);
        pbv = rd_vtx(pb, 1u);
    }
    for (var i = 0u; i < cnt; i++) {
        var go = false;
        var p: Px;
        if (line) {
            let t = fdiv(fu(i), fu(cnt));
            let px = sf_round_i32(fadd(x0, fmul(dx, t)));
            let py = sf_round_i32(fadd(y0, fmul(dy, t)));
            if (px == x && py == y) {
                go = true;
                p = pbv;
                if ((at & 8u) != 0u) {
                    for (var k = 0u; k < lb(4u); k++) {
                        p.rgba[k] = fadd(pa.rgba[k], fmul(fsub(pbv.rgba[k], pa.rgba[k]), t));
                    }
                }
            }
        } else {
            let pp = prim_p(pb, x, y);
            go = pp.go;
            p = pp.p;
        }
        if (go) {
            shade_put(pb, x, y, p);
        }
    }
}

fn in_box(pb: u32, x: i32, y: i32) -> bool {
    let b = pb + R_BB;
    return x >= bitcast<i32>(prims[b]) && x < bitcast<i32>(prims[b + 1u]) && y >= bitcast<i32>(prims[b + 2u]) && y < bitcast<i32>(prims[b + 3u]);
}

// Words of `ovl` after the four planes: per pixel of the last raster step (256 per tile, at most `RQ_TILES` tiles) the
// prim k_redo starts at (NONE: the pixel is done).
const RQ: u32 = 4194304u;
const NONE: u32 = 0xffffffffu;

// A batch of prims, one workgroup per 16 x 16 tile: each thread owns one pixel and runs the tile's prims on it in order.
// A pixel that needs the integer path stops before that prim's writes; k_redo draws it from there on. The pixels of a
// batch do not depend on each other (gpu.rs `prim`), so each stops on its own. Params: 0 tile header base (word index into
// `tiles`), 1 tiles.
@compute @workgroup_size(16, 16)
fn k_raster(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) li: vec3<u32>, @builtin(local_invocation_index) lx: u32) {
    let ti = wg.x + wg.y * 65535u;
    if (ti >= pw(1u)) {
        return;
    }
    let h = pw(0u) + ti * 4u;
    let x = i32(tiles[h] * 16u + li.x);
    let y = i32(tiles[h + 1u] * 16u + li.y);
    let start = tiles[h + 2u];
    let n = tiles[h + 3u];
    var r = NONE;
    for (var i = 0u; i < n; i++) {
        let pb = tiles[start + i] * STRIDE;
        if (in_box(pb, x, y)) {
            bad = false;
            prim_draw(pb, x, y);
            if (bad) {
                r = i;
                break;
            }
        }
    }
    ovl[RQ + ti * 256u + lx] = r;
}

// Words of `ovl` after the redo words: shaded values of a batch's (tile, prim) pairs, 256 pixels of 6 words each
// (gpu.rs `SV_PAIRS` pairs at most).
const SVB: u32 = 8388608u;

// A batch with few tiles and long tile lists (gpu.rs `close`): every (tile, prim) pair is one workgroup that shades its
// 256 pixels (texture, colour, fog, alpha test: nothing that depends on the frame), and k_put then blends each pixel's
// values in prim order. Params: 0 tile header base, 1 tiles, 2 pairs (the tile lists' entries, in order); after the lists
// `tiles` holds each entry's tile.
@compute @workgroup_size(16, 16)
fn k_shade(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) li: vec3<u32>, @builtin(local_invocation_index) lx: u32) {
    let j = wg.x + wg.y * 65535u;
    if (j >= pw(2u)) {
        return;
    }
    let lb = pw(0u) + pw(1u) * 4u;
    let ti = tiles[lb + pw(2u) + j];
    let h = pw(0u) + ti * 4u;
    let x = i32(tiles[h] * 16u + li.x);
    let y = i32(tiles[h + 1u] * 16u + li.y);
    let pb = tiles[lb + j] * STRIDE;
    var v: Sv;
    if (in_box(pb, x, y)) {
        bad = false;
        v = prim_sv(pb, x, y);
        if (bad) {
            v.k = 3u;
        }
    }
    let o = SVB + (j * 256u + lx) * 6u;
    ovl[o] = v.k;
    ovl[o + 1u] = v.c[0];
    ovl[o + 2u] = v.c[1];
    ovl[o + 3u] = v.c[2];
    ovl[o + 4u] = v.c[3];
    ovl[o + 5u] = v.z;
}

// The blending half of k_raster over k_shade's values (the same params); a pixel that needs the integer path stops for
// k_redo as in k_raster.
@compute @workgroup_size(16, 16)
fn k_put(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) li: vec3<u32>, @builtin(local_invocation_index) lx: u32) {
    let ti = wg.x + wg.y * 65535u;
    if (ti >= pw(1u)) {
        return;
    }
    let lb = pw(0u) + pw(1u) * 4u;
    let h = pw(0u) + ti * 4u;
    let x = i32(tiles[h] * 16u + li.x);
    let y = i32(tiles[h + 1u] * 16u + li.y);
    let start = tiles[h + 2u];
    let n = tiles[h + 3u];
    var r = NONE;
    for (var i = 0u; i < n; i++) {
        let o = SVB + ((start + i - lb) * 256u + lx) * 6u;
        let k = ovl[o];
        if (k == 3u) {
            r = i;
            break;
        }
        if (k != 0u) {
            bad = false;
            let w = put(tiles[start + i] * STRIDE, x, y, vec4<u32>(ovl[o + 1u], ovl[o + 2u], ovl[o + 3u], ovl[o + 4u]), k == 1u, ovl[o + 5u]);
            if (bad) {
                r = i;
                break;
            }
            commit(w);
        }
    }
    ovl[RQ + ti * 256u + lx] = r;
}

// The pixels k_raster left (the same params), from the prim each stopped at, with the integer path where needed.
@compute @workgroup_size(16, 16)
fn k_redo(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) li: vec3<u32>, @builtin(local_invocation_index) lx: u32) {
    let ti = wg.x + wg.y * 65535u;
    if (ti >= pw(1u)) {
        return;
    }
    let r = ovl[RQ + ti * 256u + lx];
    if (r == NONE) {
        return;
    }
    let h = pw(0u) + ti * 4u;
    let x = i32(tiles[h] * 16u + li.x);
    let y = i32(tiles[h + 1u] * 16u + li.y);
    let start = tiles[h + 2u];
    let n = tiles[h + 3u];
    for (var i = r; i < n; i++) {
        let pb = tiles[start + i] * STRIDE;
        if (in_box(pb, x, y)) {
            prim_draw(pb, x, y);
        }
    }
}

// One prim whose pixels depend on each other (they share memory or it samples its own target), on one thread in the CPU's
// order: rows top to bottom, each left to right; a line step by step. Params: 0 prim index.
@compute @workgroup_size(1)
fn k_serial() {
    let pb = pw(0u) * STRIDE;
    let b = pb + R_BB;
    let line = prims[pb + R_KIND] == 1u;
    let g = pb + R_G;
    let at = prims[pb + R_AT];
    let bx = bitcast<i32>(prims[b]);
    let by = bitcast<i32>(prims[b + 2u]);
    let wd = u32(max(bitcast<i32>(prims[b + 1u]) - bx, 0));
    let ht = u32(max(bitcast<i32>(prims[b + 3u]) - by, 0));
    var total = wd * ht;
    var x0 = 0u;
    var y0 = 0u;
    var dx = 0u;
    var dy = 0u;
    var pa: Px;
    var pbv: Px;
    if (line) {
        x0 = prims[g];
        y0 = prims[g + 1u];
        dx = fsub(prims[g + 2u], x0);
        dy = fsub(prims[g + 3u], y0);
        total = prims[g + 4u];
        pa = rd_vtx(pb, 0u);
        pbv = rd_vtx(pb, 1u);
    }
    // One loop over the lines' steps or the box's pixels (rows top to bottom, each left to right), one shade_put call.
    for (var i = 0u; i < total; i++) {
        var x = 0;
        var y = 0;
        var go = false;
        var p: Px;
        if (line) {
            let t = fdiv(fu(i), fu(total));
            let px = sf_round_i32(fadd(x0, fmul(dx, t)));
            let py = sf_round_i32(fadd(y0, fmul(dy, t)));
            if (in_box(pb, px, py)) {
                x = px;
                y = py;
                go = true;
                p = pbv;
                if ((at & 8u) != 0u) {
                    for (var k = 0u; k < lb(4u); k++) {
                        p.rgba[k] = fadd(pa.rgba[k], fmul(fsub(pbv.rgba[k], pa.rgba[k]), t));
                    }
                }
            }
        } else {
            x = bx + i32(i % wd);
            y = by + i32(i / wd);
            let pp = prim_p(pb, x, y);
            go = pp.go;
            p = pp.p;
        }
        if (go) {
            shade_put(pb, x, y, p);
        }
    }
}

// ---- transfers ----

// HWREG data into local memory (gscap.rs `put`), pixels `first..first + n` of the transfer, their qwords from `dat`.
// Params: 0 dbp, 1 dbw, 2 dpsm, 3 dx, 4 dy, 5 rw, 6 rh, 7 first pixel, 8 pixels, 9 data word offset (the qword holding pixel
// `first`), 10 serial, 11 main buffer known, 12 its first block, 13 its end block, 14 its width (`upload_main`).
fn xfer_px(i: u32) {
    let dpsm = pw(2u);
    var bpp = 4u;
    switch dpsm {
        case 0x00u: {
            bpp = 32u;
        }
        case 0x01u: {
            bpp = 24u;
        }
        case 0x02u, 0x0au: {
            bpp = 16u;
        }
        case 0x13u, 0x1bu: {
            bpp = 8u;
        }
        default: {}
    }
    let ppq = 128u / bpp;
    let first = pw(7u);
    let g = first + i;
    let rw = pw(5u);
    let tx = g % rw;
    let ty = g / rw;
    if (ty >= pw(6u)) {
        return;
    }
    let qi = (g / ppq) - (first / ppq);
    let bit = (g % ppq) * bpp;
    let w = pw(9u) + qi * 4u + bit / 32u;
    let sh = bit % 32u;
    var v = dat[w] >> sh;
    if (sh + bpp > 32u) {
        v = v | (dat[w + 1u] << (32u - sh));
    }
    if (bpp < 32u) {
        v = v & ((1u << bpp) - 1u);
    }
    let x = pw(3u) + tx;
    let y = pw(4u) + ty;
    let dbp = pw(0u);
    let dbw = pw(1u);
    wr_px(dpsm, dbp, dbw, x, y, v);
    // gscap.rs `upload_main`: a pixel landing in the main buffer clears the overlay there.
    if (dpsm == 0u && pw(11u) != 0u) {
        let a = a32(dbp, dbw, x, y);
        let blk = a / 256u;
        let fb32 = pw(12u);
        if (blk >= fb32 && blk < pw(13u)) {
            let word = a / 4u - fb32 * 64u;
            let page = word / 2048u;
            let b = (word / 64u) % 32u;
            let ww = word % 64u;
            let inv = lut[32768u + b * 64u + ww];
            let ix = inv & 0xffu;
            let iy = inv >> 8u;
            let bw = max(pw(14u), 1u);
            let ox = (page % bw) * 64u + (ix / 8u) * 8u + ix % 8u;
            let oy = (page / bw) * 32u + (iy / 8u) * 8u + iy % 8u;
            if (ox < 512u && oy < 512u) {
                let o = (oy * 512u + ox) * 4u;
                ovl[o] = 0u;
                ovl[o + 1u] = 0u;
                ovl[o + 2u] = 0u;
                ovl[o + 3u] = ONE;
                if (pw(15u) != 0u) {
                    hd_clear(ox, oy);
                }
            }
        }
    }
}

@compute @workgroup_size(64)
fn k_xfer(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = pw(8u);
    if (pw(10u) != 0u) {
        if (gid.x == 0u) {
            for (var i = 0u; i < n; i++) {
                xfer_px(i);
            }
        }
        return;
    }
    let i = gid.x + gid.y * 65535u * 64u;
    if (i < n) {
        xfer_px(i);
    }
}

// TRXDIR 2 (gscap.rs `local_copy`). Params: 0 sbp, 1 sbw, 2 spsm, 3 dbp, 4 dbw, 5 dpsm, 6 sx, 7 sy, 8 dx, 9 dy, 10 rw,
// 11 rh, 12 serial.
fn copy_px(i: u32) {
    let rw = pw(10u);
    let x = i % rw;
    let y = i / rw;
    let v = rd_px(pw(2u), pw(0u), pw(1u), pw(6u) + x, pw(7u) + y);
    wr_px(pw(5u), pw(3u), pw(4u), pw(8u) + x, pw(9u) + y, v);
}

@compute @workgroup_size(64)
fn k_copy(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = pw(10u) * pw(11u);
    if (pw(12u) != 0u) {
        if (gid.x == 0u) {
            for (var i = 0u; i < n; i++) {
                copy_px(i);
            }
        }
        return;
    }
    let i = gid.x + gid.y * 65535u * 64u;
    if (i < n) {
        copy_px(i);
    }
}

// CLUT loads (draw.rs `clut_load`), in order: each copies its previous slot and loads its entries from local memory.
// Params: 0 first load (word index into `dat`, 8 words a load), 1 loads. A load: prev slot, new slot, cbp, cfmt, t4, offset,
// texa lo, texa hi.
@compute @workgroup_size(256)
fn k_clut(@builtin(local_invocation_id) li: vec3<u32>) {
    let n = pw(1u);
    let t = li.x;
    for (var j = 0u; j < n; j++) {
        let l = pw(0u) + j * 8u;
        let prev = dat[l];
        let slot = dat[l + 1u];
        let cbp = dat[l + 2u];
        let cfmt = dat[l + 3u];
        let t4 = dat[l + 4u] != 0u;
        let off = dat[l + 5u];
        clut[slot * 512u + t] = clut[prev * 512u + t];
        clut[slot * 512u + 256u + t] = clut[prev * 512u + 256u + t];
        storageBarrier();
        workgroupBarrier();
        var i = 512u;
        var raw = 0u;
        if (t4) {
            if (t < 16u) {
                raw = rd_px(cfmt, cbp, 1u, t % 8u, t / 8u);
                i = off + t;
            }
        } else {
            let p = (t & 0xe7u) | ((t & 8u) << 1u) | ((t & 0x10u) >> 1u);
            raw = rd_px(cfmt, cbp, 1u, p % 16u, p / 16u);
            i = t;
        }
        if (i < 512u) {
            let c = unpack_col(cfmt, raw, dat[l + 6u], dat[l + 7u]);
            clut[slot * 512u + i] = c.x | (c.y << 8u) | (c.z << 16u) | (c.w << 24u);
        }
        storageBarrier();
        workgroupBarrier();
    }
}

// ---- overlay planes ----

// Params: 0 op (0 fill with (0, 0, 0, 1), 1 copy, 2 normalise: rgb / 255 clamped to 0..1, alpha copied), 1 source plane,
// 2 destination plane.
@compute @workgroup_size(256)
fn k_ov(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * 65535u * 256u;
    if (i >= 262144u) {
        return;
    }
    let s = (pw(1u) * 262144u + i) * 4u;
    let d = (pw(2u) * 262144u + i) * 4u;
    switch pw(0u) {
        case 0u: {
            ovl[d] = 0u;
            ovl[d + 1u] = 0u;
            ovl[d + 2u] = 0u;
            ovl[d + 3u] = ONE;
        }
        case 1u: {
            for (var k = 0u; k < 4u; k++) {
                ovl[d + k] = ovl[s + k];
            }
        }
        default: {
            for (var k = 0u; k < 3u; k++) {
                ovl[d + k] = sf_clamp(fdiv(ovl[s + k], F255), 0u, ONE);
            }
            ovl[d + 3u] = ovl[s + 3u];
        }
    }
}

// Soft-float self test (gpu.rs `selftest`): dat holds (op, a, b) triples, the results go to `ovl`. Ops 0-8 the integer
// path, 9-14 the fast one (14: a * b + a, which a fused multiply-add would round once).
@compute @workgroup_size(64)
fn k_sftest(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * 65535u * 64u;
    if (i >= pw(0u)) {
        return;
    }
    let op = dat[i * 3u];
    let a = dat[i * 3u + 1u];
    let b = dat[i * 3u + 2u];
    var r = 0u;
    switch op {
        case 0u: {
            r = sf_add(a, b);
        }
        case 1u: {
            r = sf_sub(a, b);
        }
        case 2u: {
            r = sf_mul(a, b);
        }
        case 3u: {
            r = sf_div(a, b);
        }
        case 4u: {
            r = bitcast<u32>(sf_to_i32(a));
        }
        case 5u: {
            r = sf_to_u32(a);
        }
        case 6u: {
            r = bitcast<u32>(sf_round_i32(a));
        }
        case 7u: {
            r = sf_ffloor(a);
        }
        case 8u: {
            r = select(0u, 1u, sf_lt(a, b)) | (select(0u, 2u, sf_eq(a, b)));
        }
        case 9u: {
            r = fadd(a, b);
        }
        case 10u: {
            r = fsub(a, b);
        }
        case 11u: {
            r = fmul(a, b);
        }
        case 12u: {
            r = fdiv(a, b);
        }
        case 13u: {
            r = fffloor(a);
        }
        default: {
            r = fadd(fmul(a, b), a);
        }
    }
    ovl[i] = r;
}

// ---- sharp HUD (lane G; gpu.rs `hd_*`) ----
// The overlay prims drawn a second time at the window's resolution, in hardware float (no PS2 bit-exactness: this picture is a PC
// extra, the 512-wide picture next to it is the exact one). The HD plane is Wp x Hp pixels (Wp = 2 * KX, Hp = ceil(rows * KY / 256);
// KX, KY: sharp pixels per GS pixel, x256). A sharp pixel's centre is e = ((X + .5) / kx, (Y + .5) / ky) in GS pixel edge
// coordinates (GS pixel i spans [i, i + 1)), the same convention the 512 path has for whole-pixel prims: the sprite
// [lx, hx) covers e iff lx <= e < hx. Tunables are consts: try them with RAC_GSGPU_WGSL=<file>, no rebuild.

const HPS: u32 = 3145728u;
// Plane 4 (gpu.rs `TWIN`): the sharp twin of a small offscreen target (the shop's 512 x 128 scratch, stage 3), SPS pixels of f16 rgba after
// the stage plane, Wp pixels a row like the others; it holds GS pixel values (rgb 0..255, alpha 0..255 as written), not an overlay.
const SPS: u32 = 1572864u;
const HD_TLEN: f32 = 2.0;
const HD_TWIN: f32 = 2.2;
const HD_BC: f32 = 0.4;
const HD_B0: f32 = 0.12;
const HD_B1: f32 = 0.25;
const HD_C0: f32 = 0.25;
const HD_C1: f32 = 0.6;
const HD_NDEL: f32 = 0.5;
const HD_AAW: f32 = 1.0;
const HD_E0: f32 = 0.2;
const HD_E1: f32 = 0.5;
const HD_T0: f32 = 0.002;
const HD_T1: f32 = 0.02;
const HD_FPMIN: f32 = 0.02;
const HD_LOWC: f32 = 0.04;

fn hd_o(p: u32, i: u32) -> u32 {
    return select((p * HPS + i) * 2u, 7u * HPS + i * 2u, p == 4u);
}

fn hd_ld(p: u32, i: u32) -> vec4<f32> {
    let o = hd_o(p, i);
    let a = unpack2x16float(hdp[o]);
    let b = unpack2x16float(hdp[o + 1u]);
    return vec4<f32>(a.x, a.y, b.x, b.y);
}

fn hd_st(p: u32, i: u32, c: vec4<f32>) {
    let o = hd_o(p, i);
    hdp[o] = pack2x16float(c.xy);
    hdp[o + 1u] = pack2x16float(c.zw);
}

// Params: 0 op (0 fill (0, 0, 0, 1), 1 copy, 2 normalise into the stage plane: rgb / 255 and alpha clamped to 0..1, RGBA8 as the
// CPU rounds it), 1 source plane, 2 destination plane, 3 pixels (Wp * Hp).
@compute @workgroup_size(256)
fn k_hov(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * 65535u * 256u;
    if (i >= pw(3u)) {
        return;
    }
    switch pw(0u) {
        case 0u: {
            hd_st(pw(2u), i, vec4<f32>(0.0, 0.0, 0.0, 1.0));
        }
        case 1u: {
            let s = (pw(1u) * HPS + i) * 2u;
            let d = (pw(2u) * HPS + i) * 2u;
            hdp[d] = hdp[s];
            hdp[d + 1u] = hdp[s + 1u];
        }
        default: {
            let c = hd_ld(pw(1u), i);
            let n = vec4<f32>(clamp(c.rgb / 255.0, vec3<f32>(0.0), vec3<f32>(1.0)), clamp(c.a, 0.0, 1.0));
            let b = vec4<u32>(n * 255.0 + vec4<f32>(0.5));
            hdp[6u * HPS + i] = b.x | (b.y << 8u) | (b.z << 16u) | (b.w << 24u);
        }
    }
}

// xfer_px: the sharp pixels whose centre lies in GS pixel (ox, oy) are cleared with it. Params 15: KX | KY << 16.
fn hd_clear(ox: u32, oy: u32) {
    let kx = pw(15u) & 0xffffu;
    let ky = pw(15u) >> 16u;
    let wp = 2u * kx;
    let x0 = (ox * kx + 127u) / 256u;
    let x1 = ((ox + 1u) * kx + 127u) / 256u;
    let y0 = (oy * ky + 127u) / 256u;
    let y1 = ((oy + 1u) * ky + 127u) / 256u;
    for (var y = y0; y < y1; y++) {
        for (var x = x0; x < x1; x++) {
            if (y * wp + x < HPS) {
                hd_st(0u, y * wp + x, vec4<f32>(0.0, 0.0, 0.0, 1.0));
            }
        }
    }
}

fn hf(i: u32) -> f32 {
    return bitcast<f32>(prims[i]);
}

// A vertex or an interpolated point: c colour, a = (s, t, q, fog), b = (u, v).
struct HV {
    c: vec4<f32>,
    a: vec4<f32>,
    b: vec2<f32>,
}

fn hv(pb: u32, k: u32) -> HV {
    let b = pb + R_PX + k * 11u;
    return HV(vec4<f32>(hf(b), hf(b + 1u), hf(b + 2u), hf(b + 3u)), vec4<f32>(hf(b + 4u), hf(b + 5u), hf(b + 6u), hf(b + 9u)), vec2<f32>(hf(b + 7u), hf(b + 8u)));
}

var<private> hgo: bool;

// The prim's vertex values at e (like prim_p, in float); `hgo` says whether the prim covers e. Values are also given
// outside the prim (the texel footprint is taken from neighbours).
fn hd_geom(pb: u32, e: vec2<f32>) -> HV {
    let kind = prims[pb + R_KIND];
    let at = prims[pb + R_AT];
    let g = pb + R_G;
    hgo = false;
    var o = hv(pb, select(select(1u, 2u, kind >= 3u), 0u, kind == 0u));
    if (kind == 0u) {
        let b = pb + R_BB;
        hgo = e.x >= f32(bitcast<i32>(prims[b])) && e.x < f32(bitcast<i32>(prims[b + 1u])) && e.y >= f32(bitcast<i32>(prims[b + 2u])) && e.y < f32(bitcast<i32>(prims[b + 3u]));
    } else if (kind == 1u) {
        // One sharp-pixel-thin line: within half a GS pixel of the segment along its minor axis, the steps `cnt` of prim_draw
        // reach from the first pixel's start to the last pixel's end.
        let a = vec2<f32>(hf(g), hf(g + 1u)) + vec2<f32>(0.5);
        let d = vec2<f32>(hf(g + 2u), hf(g + 3u)) + vec2<f32>(0.5) - a;
        let cnt = max(f32(prims[g + 4u]), 1.0);
        var s = 0.0;
        if (d.x == 0.0 && d.y == 0.0) {
            hgo = all(abs(e - a) <= vec2<f32>(0.5));
        } else {
            var off: f32;
            if (abs(d.x) >= abs(d.y)) {
                s = (e.x - a.x) / d.x;
                off = e.y - (a.y + s * d.y);
            } else {
                s = (e.y - a.y) / d.y;
                off = e.x - (a.x + s * d.x);
            }
            hgo = abs(off) <= 0.5 && s * cnt >= -0.5 && s * cnt <= cnt - 0.5;
        }
        if ((at & 8u) != 0u) {
            o.c = mix(hv(pb, 0u).c, o.c, clamp(s, 0.0, 1.0));
        }
    } else if (kind == 2u) {
        let c0 = vec2<f32>(hf(g), hf(g + 1u));
        let c1 = vec2<f32>(hf(g + 2u), hf(g + 3u));
        hgo = all(e >= min(c0, c1)) && all(e < max(c0, c1));
        if ((at & 0x10u) != 0u) {
            let v0 = hv(pb, 0u);
            let dd = c1 - c0;
            let t = vec2<f32>(select(0.0, (e.x - c0.x) / dd.x, dd.x != 0.0), select(0.0, (e.y - c0.y) / dd.y, dd.y != 0.0));
            if ((at & 0x100u) != 0u) {
                o.b = mix(v0.b, o.b, t);
            } else {
                o.a = vec4<f32>(mix(v0.a.xy, o.a.xy, t), o.a.zw);
            }
        }
    } else {
        let p0 = vec2<f32>(hf(g), hf(g + 1u));
        let p1 = vec2<f32>(hf(g + 2u), hf(g + 3u));
        let p2 = vec2<f32>(hf(g + 4u), hf(g + 5u));
        let area = hf(g + 6u);
        // A nudge off the pixel-aligned lattice: a shared edge never belongs to both triangles.
        let q = e + vec2<f32>(1.3 / 4096.0, 1.7 / 4096.0);
        let w0 = ((p1.x - q.x) * (p2.y - q.y) - (p1.y - q.y) * (p2.x - q.x)) / area;
        let w1 = ((p2.x - q.x) * (p0.y - q.y) - (p2.y - q.y) * (p0.x - q.x)) / area;
        let w2 = 1.0 - w0 - w1;
        hgo = area != 0.0 && w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0;
        let v0 = hv(pb, 0u);
        let v1 = hv(pb, 1u);
        if ((at & 8u) != 0u) {
            o.c = w0 * v0.c + w1 * v1.c + w2 * o.c;
        }
        if ((at & 0x100u) == 0u) {
            o.a = vec4<f32>(w0 * v0.a.xyz + w1 * v1.a.xyz + w2 * o.a.xyz, o.a.w);
        } else {
            o.b = w0 * v0.b + w1 * v1.b + w2 * o.b;
        }
        if ((at & 0x20u) != 0u) {
            o.a.w = w0 * v0.a.w + w1 * v1.a.w + w2 * o.a.w;
        }
    }
    return o;
}

// Texel coordinates of a point (shade's u, v).
fn hd_uv(pb: u32, v: HV) -> vec2<f32> {
    if ((prims[pb + R_AT] & 0x100u) != 0u) {
        return v.b;
    }
    let q = select(v.a.z, 1.0, v.a.z == 0.0);
    return vec2<f32>(v.a.x / q * f32(tex_w(pb)), v.a.y / q * f32(tex_h(pb)));
}

// The texel rectangle the prim's own vertices span (x0, y0, x1, y1 inclusive); the sampler keeps its cells inside it, so a
// sprite's edge never takes a neighbour in the atlas. Unbounded when that rectangle leaves the texture (wrapping prims).
fn hd_rect(pb: u32) -> vec4<i32> {
    let kind = prims[pb + R_KIND];
    let big = vec4<i32>(-1048576, -1048576, 1048576, 1048576);
    if (kind < 2u) {
        return big;
    }
    var lo = hd_uv(pb, hv(pb, 0u));
    var hi = lo;
    let u1 = hd_uv(pb, hv(pb, 1u));
    lo = min(lo, u1);
    hi = max(hi, u1);
    if (kind >= 3u) {
        let u2 = hd_uv(pb, hv(pb, 2u));
        lo = min(lo, u2);
        hi = max(hi, u2);
    }
    if (lo.x >= 0.0 && lo.y >= 0.0 && hi.x <= f32(tex_w(pb)) && hi.y <= f32(tex_h(pb))) {
        let a = floor(lo + vec2<f32>(0.001));
        let b = max(ceil(hi - vec2<f32>(0.001)) - vec2<f32>(1.0), a);
        return vec4<i32>(vec2<i32>(a), vec2<i32>(b));
    }
    return big;
}

// Magnified sampling (a texel spans more than one sharp pixel). The inner 4 x 4 of the 6 x 6 texels around the point is fetched first, as
// alpha-premultiplied colour (rgb / 255 * a, a) with a = alpha / 128 (1 when the prim ignores the texture's alpha): a flat 4 x 4 returns
// its texel, a 4 x 4 below HD_LOWC contrast is the plain bicubic (the tensor would be under its noise floor), only the rest fetches the
// outer ring. `usea` means the prim reads the TEXTURE's alpha (TCC set and a blend with As or PABE, or an alpha test that can decide; see
// `hd_shade`); otherwise (including FIX blend + ATST always) the colour is the texel's whatever its alpha, as in the 512 path. With TCC clear the texel alpha
// is not part of the picture and must not weight the colour. Then:
//  1. a windowed structure tensor of (alpha, luma) tells the edge direction (tangent t, normal n) and how line-like it is;
//  2. the field is reconstructed bilinearly but averaged ALONG the tangent (length HD_TLEN * coherence): stair-steps of
//     diagonals and curves are smoothed away, the profile across the edge is kept;
//  3. where the neighbourhood has a strong, line-like contrast, each channel is re-projected onto an edge of HD_AAW sharp
//     pixels width: the signed distance to the mid-level contour is estimated from the slope of the smoothed field along n
//     and turned into smooth coverage, limited to the neighbourhood's own min..max (no overshoot);
//  4. smooth shaded art (no strong line-like contrast) is left as the smooth bilinear field.
const HN: u32 = 6u;
const HC: f32 = 2.0;

// The k_hd pipelines (gpu.rs `K_HD`, `K_HDF`): the second takes a step without a textured prim, without the sampler (its registers
// made every pixel of an untextured step, the menus' full-screen tints, about twice as slow).
override HDT: bool = true;

// The texels the magnified pixels of a `k_hd` tile read, fetched once for the whole workgroup (each pixel's 6 x 6 in private memory made
// the kernel spill, and every prim of the tile paid for it): `hbs` holds the hd_q values of a block of cells, rows of `hbw`, cell (x, y)
// the texel (x, y) clamped into the prim's rectangle as the sampler takes it; the pixel's 6 x 6 starts at `hb0`.
const HBN: u32 = 1000u;
var<workgroup> hbs: array<vec4<f32>, HBN>;
// The pending pixels' windows (x0, y0, x1, y1; the sane ones), the lowest pending pixel, whether one is not sane.
var<workgroup> hbk: array<atomic<i32>, 6>;
// A pass's block: origin x, y, width, height; height 0: no pixel left, negative: the last pass.
var<workgroup> hbx: vec4<i32>;
var<private> hb0: u32;
var<private> hbw: u32;

fn hd_q(t: vec4<f32>, usea: bool) -> vec4<f32> {
    if (usea) {
        let an = t.a / 128.0;
        return vec4<f32>(t.rgb * (an / 255.0), an);
    }
    return vec4<f32>(t.rgb / 255.0, 1.0);
}

// Texel (x, y) of the 6 x 6 around the point (index space 0..5), from the block.
fn hqa(x: u32, y: u32) -> vec4<f32> {
    return hbs[hb0 + y * hbw + x];
}

fn hbk_reset() {
    atomicStore(&hbk[0], 0x7fffffff);
    atomicStore(&hbk[1], 0x7fffffff);
    atomicStore(&hbk[2], -0x7fffffff);
    atomicStore(&hbk[3], -0x7fffffff);
    atomicStore(&hbk[4], 256);
    atomicStore(&hbk[5], 0);
    hbx = vec4<i32>(0);
}

// Bilinear in the 6 x 6 (index space 0..5; the point lies in 2..3).
fn hq_bil(x: vec2<f32>) -> vec4<f32> {
    let p = clamp(x, vec2<f32>(0.0), vec2<f32>(4.999));
    let fl = floor(p);
    let f = p - fl;
    let ix = u32(fl.x);
    let iy = u32(fl.y);
    return mix(mix(hqa(ix, iy), hqa(ix + 1u, iy), f.x), mix(hqa(ix, iy + 1u), hqa(ix + 1u, iy + 1u), f.x), f.y);
}

// The smooth field at x averaged along the tangent t over +-l (tent of 7 taps).
fn hq_tan(x: vec2<f32>, t: vec2<f32>, l: f32) -> vec4<f32> {
    let h = t * (l / 3.0);
    return (hq_bil(x - 3.0 * h) + 2.0 * hq_bil(x - 2.0 * h) + 3.0 * hq_bil(x - h) + 4.0 * hq_bil(x) + 3.0 * hq_bil(x + h) + 2.0 * hq_bil(x + 2.0 * h) + hq_bil(x + 3.0 * h)) / 16.0;
}

// Catmull-Rom weights at t.
fn hq_cr(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(-0.5 * t3 + t2 - 0.5 * t, 1.5 * t3 - 2.5 * t2 + 1.0, -1.5 * t3 + 2.0 * t2 + 0.5 * t, 0.5 * t3 - 0.5 * t2);
}

// Bicubic (Catmull-Rom) of the 4 x 4 around the point (the 6 x 6's inner block), limited to the min..max of the 2 x 2 cell it is in
// so edges do not ring.
fn hq_bicubic(f: vec2<f32>) -> vec4<f32> {
    let wx = hq_cr(f.x);
    let wy = hq_cr(f.y);
    var r = vec4<f32>(0.0);
    for (var j = 0u; j < 4u; j++) {
        let row = hqa(1u, 1u + j) * wx.x + hqa(2u, 1u + j) * wx.y + hqa(3u, 1u + j) * wx.z + hqa(4u, 1u + j) * wx.w;
        r += row * select(select(select(wy.x, wy.y, j == 1u), wy.z, j == 2u), wy.w, j == 3u);
    }
    let a = hqa(2u, 2u);
    let b = hqa(3u, 2u);
    let d = hqa(2u, 3u);
    let e = hqa(3u, 3u);
    return clamp(r, min(min(a, b), min(d, e)), max(max(a, b), max(d, e)));
}

// p: the point in the 6 x 6's index space (`k_hd`: the cell floor(uv - 0.5) is its texel (2, 2)); the 6 x 6 is in the block at `hb0`.
fn hd_tex_mag(p: vec2<f32>, fp: vec2<f32>, usea: bool) -> vec4<f32> {
    // The inner 4 x 4 first (all that bicubic needs); the outer ring only where the 4 x 4 has contrast.
    var lo4 = hqa(1u, 1u);
    var hi4 = lo4;
    for (var k = 1u; k < lb(16u); k++) {
        let c = hqa(1u + (k & 3u), 1u + (k >> 2u));
        lo4 = min(lo4, c);
        hi4 = max(hi4, c);
    }
    let rg4 = hi4 - lo4;
    let rmax = max(max(rg4.x, rg4.y), max(rg4.z, rg4.w));
    if (rmax == 0.0) {
        let tx = hqa(2u, 2u);
        if (usea) {
            return vec4<f32>(select(tx.rgb * 255.0 / tx.a, vec3<f32>(0.0), tx.a < 0.0001), tx.a * 128.0);
        }
        return vec4<f32>(tx.rgb * 255.0, 128.0);
    }
    if (rmax < HD_LOWC) {
        // no edge in reach: the structure tensor would be below its noise floor, the result is the plain bicubic
        let o = hq_bicubic(p - vec2<f32>(HC));
        if (!usea) {
            return vec4<f32>(clamp(o.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0, 128.0);
        }
        return vec4<f32>(clamp(o.rgb / max(o.a, 0.04), vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0, o.a * 128.0);
    }
    // 1. structure tensor of (alpha, luma): quad (qx, qy) spans texels (qx, qy)..(qx + 1, qy + 1)
    var jxx = 0.0;
    var jxy = 0.0;
    var jyy = 0.0;
    var wsum = 0.0;
    for (var qy = 0u; qy < HN - 1u; qy++) {
        for (var qx = 0u; qx < HN - 1u; qx++) {
            let a = hqa(qx, qy);
            let b = hqa(qx + 1u, qy);
            let c = hqa(qx, qy + 1u);
            let d = hqa(qx + 1u, qy + 1u);
            let gx = ((b - a) + (d - c)) * 0.5;
            let gy = ((c - a) + (d - b)) * 0.5;
            let lx = dot(gx.rgb, vec3<f32>(0.299, 0.587, 0.114));
            let ly = dot(gy.rgb, vec3<f32>(0.299, 0.587, 0.114));
            let w = max(0.0, HD_TWIN - length(p - vec2<f32>(f32(qx) + 0.5, f32(qy) + 0.5)));
            jxx += w * (gx.a * gx.a + lx * lx);
            jxy += w * (gx.a * gy.a + lx * ly);
            jyy += w * (gy.a * gy.a + ly * ly);
            wsum += w;
        }
    }
    jxx /= wsum;
    jxy /= wsum;
    jyy /= wsum;
    let en = jxx + jyy;
    let dm = sqrt((jxx - jyy) * (jxx - jyy) + 4.0 * jxy * jxy);
    let coh = dm / max(en, 1.0e-6) * smoothstep(HD_T0, HD_T1, en);
    var nv = vec2<f32>(jxx - jyy + dm, 2.0 * jxy);
    let nv2 = vec2<f32>(2.0 * jxy, jyy - jxx + dm);
    if (dot(nv2, nv2) > dot(nv, nv)) {
        nv = nv2;
    }
    nv = select(vec2<f32>(1.0, 0.0), normalize(nv), dot(nv, nv) > 1.0e-12);
    let tv = vec2<f32>(-nv.y, nv.x);
    // 2. smooth field along the tangent: at p and one step either side along the normal
    let l = HD_TLEN * coh;
    let v0 = hq_tan(p, tv, l);
    let vm = hq_tan(p - nv * HD_NDEL, tv, l);
    let vp = hq_tan(p + nv * HD_NDEL, tv, l);
    // 3. edge re-projection between the extremes of the 4 x 4 around p. Alpha is one channel of its own. Colour moves along the
    // line between the colours of the darkest and the brightest texel with ONE shared coverage (per-channel contours made hue noise).
    var lo = hqa(1u, 1u);
    var hi = lo;
    var cylo = lo.rgb;
    var cyhi = lo.rgb;
    var ylo = 1.0e9;
    var yhi = -1.0e9;
    for (var j = 0u; j < 16u; j++) {
        let c = hqa(1u + (j & 3u), 1u + (j >> 2u));
        lo = min(lo, c);
        hi = max(hi, c);
        let y = dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
        if (y < ylo) {
            ylo = y;
            cylo = c.rgb;
        }
        if (y > yhi) {
            yhi = y;
            cyhi = c.rgb;
        }
    }
    // two-levelness of the 4 x 4: mean distance of a texel from the nearer extreme, relative to the range (0 = a pure edge between
    // two flat levels: glyph, outline; about 0.25 = shaded art with detail at texel size, which stays smooth)
    var my = 0.0;
    var ma = 0.0;
    for (var j = 0u; j < 16u; j++) {
        let c = hqa(1u + (j & 3u), 1u + (j >> 2u));
        let y = dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
        my += min(y - ylo, yhi - y);
        ma += min(c.a - lo.a, hi.a - c.a);
    }
    let by = 1.0 - smoothstep(HD_B0, HD_B1, my / 16.0 / max(yhi - ylo, 1.0e-4));
    let ba = 1.0 - smoothstep(HD_B0, HD_B1, ma / 16.0 / max(hi.a - lo.a, 1.0e-4));
    let ch = smoothstep(HD_C0, HD_C1, coh);
    let ra = select(0.0, hi.a - lo.a, usea);
    let ea = smoothstep(HD_E0, HD_E1, ra) * ch * ba;
    // colour is re-projected only where alpha is uniform; across an alpha edge it stays the alpha-weighted smooth field (no hue
    // shifts from the colour of transparent texels)
    let ec = smoothstep(HD_E0, HD_E1, yhi - ylo) * ch * by * (1.0 - smoothstep(0.1, 0.35, ra));
    let fpn = length(fp * nv);
    // away from edges: bicubic (sharper than bilinear, same smoothness); along edges: the tangent-smoothed field
    let wb = smoothstep(0.0, HD_BC, coh);
    var o = mix(hq_bicubic(p - vec2<f32>(HC)), v0, wb);
    // colour (premultiplied) along the line between the extreme colours
    var pcol = o.rgb;
    {
        let y0 = dot(v0.rgb, vec3<f32>(0.299, 0.587, 0.114));
        let g = abs(dot(vp.rgb - vm.rgb, vec3<f32>(0.299, 0.587, 0.114))) / (2.0 * HD_NDEL) * fpn;
        let ym = 0.5 * (ylo + yhi);
        var d = select(-8.0, 8.0, y0 > ym);
        if (g >= 0.0005) {
            d = clamp((y0 - ym) / g, -8.0, 8.0);
        }
        let t = smoothstep(0.0, 1.0, clamp(d / HD_AAW + 0.5, 0.0, 1.0));
        let ts = clamp((y0 - ylo) / max(yhi - ylo, 1.0e-4), 0.0, 1.0);
        pcol = max(pcol + ec * (t - ts) * (cyhi - cylo), vec3<f32>(0.0));
    }
    if (!usea) {
        return vec4<f32>(clamp(pcol, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0, 128.0);
    }
    // alpha: smooth coverage on an edge of HD_AAW sharp pixels; the colour stays the straight colour of the smooth field (taken
    // before the alpha moves, so no division by the re-projected alpha)
    let scol = clamp(pcol / max(o.a, 0.04), vec3<f32>(0.0), vec3<f32>(1.0));
    var aout = o.a;
    {
        let g = abs(vp.a - vm.a) / (2.0 * HD_NDEL) * fpn;
        let am = 0.5 * (lo.a + hi.a);
        var d = select(-8.0, 8.0, v0.a > am);
        if (g >= 0.0005) {
            d = clamp((v0.a - am) / g, -8.0, 8.0);
        }
        let t = smoothstep(0.0, 1.0, clamp(d / HD_AAW + 0.5, 0.0, 1.0));
        aout = mix(o.a, lo.a + (hi.a - lo.a) * t, ea);
    }
    return vec4<f32>(scol * 255.0, aout * 128.0);
}

// The texture colour at texel coordinates `uv`, cells kept in `rc`, of a pixel that is not magnified (a sharp pixel spans at least a
// texel): the texel as the 512 path takes it, or bilinear when the prim asks. Magnified pixels are always filtered (`hd_tex_mag`, in
// the texel block passes of `k_hd`).
fn hd_tex(pb: u32, uv: vec2<f32>, rc: vec4<i32>, usea: bool) -> vec4<f32> {
    let bil = (prims[pb + R_TEX1] & 0x20u) != 0u;
    let pos = select(uv, uv - vec2<f32>(0.5), bil);
    let fl = floor(pos);
    let f = pos - fl;
    let i0 = vec2<i32>(fl);
    var t: array<vec4<f32>, 4>;
    for (var j = 0u; j < lb(select(1u, 4u, bil)); j++) {
        let cx = clamp(i0.x + i32(j & 1u), rc.x, rc.z);
        let cy = clamp(i0.y + i32(j >> 1u), rc.y, rc.w);
        t[j] = bitcast<vec4<f32>>(texel(pb, cx, cy));
    }
    if (!bil) {
        return t[0];
    }
    var wrgb = vec3<f32>(0.0);
    var plain = vec3<f32>(0.0);
    var asum = 0.0;
    for (var k = 0u; k < 4u; k++) {
        let wx = select(1.0 - f.x, f.x, (k & 1u) != 0u);
        let wy = select(1.0 - f.y, f.y, (k >> 1u) != 0u);
        let w = wx * wy;
        plain += w * t[k].rgb;
        wrgb += w * t[k].a * t[k].rgb;
        asum += w * t[k].a;
    }
    return vec4<f32>(select(plain, wrgb / asum, usea && asum > 0.5), asum);
}

var<private> hd_wp: u32;

// The texture colour of a prim that textures from the twin target (record word 79 set; the 512 scratch is not read): the twin plane
// sampled bilinearly at the texel position uv (texels = the target's GS pixels), the four taps kept inside the texel rectangle `rc` the
// prim spans (in sharp pixels) and the plane. rgb and alpha are interpolated as stored: the target's own pixel values, as the 512 path
// reads them. A scratch composited at a whole-GS-pixel offset lands within a fraction of a sharp pixel of its own grid (weights near 0 / 1).
fn hd_tex_twin(uv: vec2<f32>, rc: vec4<i32>, kx: f32, ky: f32) -> vec4<f32> {
    let k = vec2<f32>(kx, ky);
    let lo = max(ceil(vec2<f32>(f32(rc.x), f32(rc.y)) * k - vec2<f32>(0.5)), vec2<f32>(0.0));
    let hi0 = ceil(vec2<f32>(f32(rc.z + 1), f32(rc.w + 1)) * k - vec2<f32>(0.5)) - vec2<f32>(1.0);
    let hi = max(min(hi0, vec2<f32>(f32(hd_wp) - 1.0, f32(SPS / hd_wp) - 1.0)), lo);
    let p = uv * k - vec2<f32>(0.5);
    let fl = floor(p);
    let f = p - fl;
    let x0 = u32(clamp(fl.x, lo.x, hi.x));
    let x1 = u32(clamp(fl.x + 1.0, lo.x, hi.x));
    let y0 = u32(clamp(fl.y, lo.y, hi.y));
    let y1 = u32(clamp(fl.y + 1.0, lo.y, hi.y));
    let a = mix(hd_ld(4u, y0 * hd_wp + x0), hd_ld(4u, y0 * hd_wp + x1), f.x);
    let b = mix(hd_ld(4u, y1 * hd_wp + x0), hd_ld(4u, y1 * hd_wp + x1), f.x);
    return mix(a, b, f.y);
}

struct HS {
    ok: bool,
    c: vec4<f32>,
}

// Whether a textured prim reads the TEXTURE's alpha. The texture's alpha is part of the picture only where something reads it: a blend
// with As as the coefficient (or PABE, which tests it) or an alpha test that can fail or pass by it. A plate drawn with a FIX coefficient
// and ATST always (the select scene's text) writes the texel's colour whatever its alpha, so its colour must not be weighted by (or lost
// with) the alpha.
fn hd_usea(pb: u32) -> bool {
    let at = prims[pb + R_AT];
    let test = prims[pb + R_TEST];
    let al = prims[pb + R_ALPHA];
    let atst = (test >> 1u) & 7u;
    let ablend = (at & 0x40u) != 0u && (((al >> 4u) & 3u) == 0u || (prims[pb + R_PABE] & 1u) != 0u);
    let atest = (test & 1u) != 0u && atst != 0u && atst != 1u;
    return (prims[pb + R_TEX0 + 1u] & 4u) != 0u && (ablend || atest);
}

// draw.rs `View::shade` at sharp pixel centre e, in float, from the prim's values there `o` (`hd_geom`) and its texture colour `t` (a
// textured prim): texture function, fog, alpha test.
fn hd_shade(pb: u32, o: HV, t: vec4<f32>) -> HS {
    let at = prims[pb + R_AT];
    var c = o.c;
    let test = prims[pb + R_TEST];
    if (HDT && (at & 0x10u) != 0u) {
        let t1 = prims[pb + R_TEX0 + 1u];
        let tcc = (t1 & 4u) != 0u;
        let tfx = (t1 >> 3u) & 3u;
        if (tfx == 1u) {
            c = vec4<f32>(t.rgb, select(c.a, t.a, tcc));
        } else {
            // 0 modulate (alpha too with TCC), 2 / 3 highlight (3: highlight 2, texture alpha as is).
            let a = c.a;
            var m = t * c / 128.0;
            if (tfx != 0u) {
                m = m + vec4<f32>(a);
            }
            m = min(m, vec4<f32>(255.0));
            var na = a;
            if (tfx == 0u) {
                na = select(a, m.a, tcc);
            } else if (tcc) {
                na = select(t.a, min(t.a + a, 255.0), tfx == 2u);
            }
            c = vec4<f32>(m.rgb, na);
        }
    }
    if ((at & 0x20u) != 0u) {
        let fc = prims[pb + R_FOGCOL];
        let f = o.a.w / 255.0;
        let fcv = vec3<f32>(f32(fc & 0xffu), f32((fc >> 8u) & 0xffu), f32((fc >> 16u) & 0xffu));
        c = vec4<f32>(c.rgb * f + fcv * (1.0 - f), c.a);
    }
    if ((test & 1u) != 0u) {
        let aref = f32((test >> 4u) & 0xffu);
        let a = floor(c.a);
        var ok = false;
        switch (test >> 1u) & 7u {
            case 0u: {
                ok = false;
            }
            case 1u: {
                ok = true;
            }
            case 2u: {
                ok = a < aref;
            }
            case 3u: {
                ok = a <= aref;
            }
            case 4u: {
                ok = a == aref;
            }
            case 5u: {
                ok = a >= aref;
            }
            case 6u: {
                ok = a > aref;
            }
            default: {
                ok = a != aref;
            }
        }
        if (!ok) {
            let fl = (test >> 12u) & 3u;
            if (fl == 0u || fl == 2u) {
                return HS(false, c);
            }
        }
    }
    return HS(true, c);
}

// One 16 x 16 tile of sharp pixels: each thread runs the tile's prims on its pixel in order, the picture of the prim's plane in
// registers. Params: 0 tile header base (word index into `tiles`), 1 tiles, 2 KX, 3 KY, 4 Wp, 5 Hp, 6 the 512 picture's rows.
// The prim loop is the same for the whole workgroup (no `continue`): a textured prim's magnified pixels fetch their texels together.
@compute @workgroup_size(16, 16)
fn k_hd(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) li: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    let ti = wg.x + wg.y * 65535u;
    if (ti >= pw(1u)) {
        return;
    }
    if (lid == 0u) {
        hbk_reset();
    }
    workgroupBarrier();
    let h = pw(0u) + ti * 4u;
    let xx = tiles[h] * 16u + li.x;
    let yy = tiles[h + 1u] * 16u + li.y;
    let wp = pw(4u);
    // a thread past the plane's edge only helps fetch texels
    let act = xx < wp && yy < pw(5u);
    let kx = f32(pw(2u)) / 256.0;
    let ky = f32(pw(3u)) / 256.0;
    // The picture's border cells (the first and the last 512 column and row) take the sample position of the 512 picture's own border
    // pixel (the GS tests pixel i at the integer position i): a full-screen quad of the retail fade or tint (x -0.5 .. 511.5) then covers
    // the whole last cell, as the 512 picture shows it, and the side strips (the clamped edge columns of this picture) carry it.
    var e = vec2<f32>((f32(xx) + 0.5) / kx, (f32(yy) + 0.5) / ky);
    let last = vec2<f32>(511.0, f32(pw(6u)) - 1.0);
    e = select(select(e, vec2<f32>(0.0), e < vec2<f32>(1.0)), last, e >= last);
    let idx = yy * wp + xx;
    hd_wp = wp;
    let start = tiles[h + 2u];
    let n = tiles[h + 3u];
    var pl = 99u;
    var acc = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    for (var i = 0u; i < n; i++) {
        let pb = tiles[start + i] * STRIDE;
        let sc = pb + 75u;
        let np = prims[pb + R_PLANE];
        let at = prims[pb + R_AT];
        let tex = HDT && (at & 0x10u) != 0u;
        let twin = prims[pb + 79u] != 0u;
        var on = act && !(e.x < hf(sc) || e.x >= hf(sc + 1u) || e.y < hf(sc + 2u) || e.y >= hf(sc + 3u)) && !(np == 4u && idx >= SPS);
        var o: HV;
        if (on) {
            o = hd_geom(pb, e);
            on = hgo;
        }
        // The texture: twin and not magnified pixels here, magnified ones in the block passes below.
        var t = vec4<f32>(0.0);
        var fp = vec2<f32>(0.0);
        var rc = vec4<i32>(0);
        var usea = false;
        var pend = false;
        var mp = vec2<f32>(0.0);
        var mi0 = vec2<i32>(0);
        if (tex) {
            rc = hd_rect(pb);
            usea = hd_usea(pb);
        }
        if (on && tex) {
            let uv = hd_uv(pb, o);
            let ux = hd_uv(pb, hd_geom(pb, e + vec2<f32>(1.0 / kx, 0.0)));
            let uy = hd_uv(pb, hd_geom(pb, e + vec2<f32>(0.0, 1.0 / ky)));
            fp = max(max(abs(ux - uv), abs(uy - uv)), vec2<f32>(HD_FPMIN));
            if (twin) {
                t = hd_tex_twin(uv, rc, kx, ky);
            } else if (max(fp.x, fp.y) < 1.0) {
                pend = true;
                let pos = uv - vec2<f32>(0.5);
                let fl = floor(pos);
                mp = pos - fl + vec2<f32>(HC);
                mi0 = vec2<i32>(fl);
            } else {
                t = hd_tex(pb, uv, rc, usea);
            }
        }
        if (tex && !twin) {
            // A pass: the pending pixels report their 6 x 6 windows, the lowest pending pixel picks the block: all the windows when they
            // fit (the normal case, one pass), else as many as fit around its own; the workgroup fetches the block and each pixel whose
            // window lies in it samples. A cell beyond +-2^28 (no block arithmetic there) is fetched alone.
            let sane = all(mi0 > vec2<i32>(-268435456)) && all(mi0 < vec2<i32>(268435456));
            let wa = vec4<i32>(mi0 - vec2<i32>(2), mi0 + vec2<i32>(3));
            loop {
                if (pend) {
                    if (sane) {
                        atomicMin(&hbk[0], wa.x);
                        atomicMin(&hbk[1], wa.y);
                        atomicMax(&hbk[2], wa.z);
                        atomicMax(&hbk[3], wa.w);
                    } else {
                        atomicMax(&hbk[5], 1);
                    }
                    atomicMin(&hbk[4], i32(lid));
                }
                workgroupBarrier();
                let ld = u32(atomicLoad(&hbk[4]));
                if (lid == ld) {
                    if (!sane) {
                        hbx = vec4<i32>(wa.xy, 6, 6);
                    } else {
                        let b = vec4<i32>(atomicLoad(&hbk[0]), atomicLoad(&hbk[1]), atomicLoad(&hbk[2]), atomicLoad(&hbk[3]));
                        let bw = b.z - b.x + 1;
                        let bh = b.w - b.y + 1;
                        if (bw <= i32(HBN) && bh <= i32(HBN) && bw * bh <= i32(HBN)) {
                            hbx = vec4<i32>(b.x, b.y, bw, select(bh, -bh, atomicLoad(&hbk[5]) == 0));
                        } else {
                            let w = min(bw, 32);
                            let hh = min(bh, i32(HBN) / w);
                            hbx = vec4<i32>(max(b.x, wa.z - w + 1), max(b.y, wa.w - hh + 1), w, hh);
                        }
                    }
                }
                let u = workgroupUniformLoad(&hbx);
                if (u.w == 0) {
                    break;
                }
                if (lid == 0u) {
                    hbk_reset();
                }
                // (wrapping i32 sums: an insane window's cells are the sampler's own, i0 + x - 2)
                let bw = u32(u.z);
                let bh = abs(u.w);
                for (var k = lid; k < bw * u32(bh); k += 256u) {
                    let cx = clamp(u.x + i32(k % bw), rc.x, rc.z);
                    let cy = clamp(u.y + i32(k / bw), rc.y, rc.w);
                    hbs[k] = hd_q(bitcast<vec4<f32>>(texel(pb, cx, cy)), usea);
                }
                workgroupBarrier();
                let mine = select(lid == ld, all(wa.xy >= u.xy) && wa.z < u.x + u.z && wa.w < u.y + bh, sane);
                if (pend && mine) {
                    hb0 = u32((wa.y - u.y) * u.z + wa.x - u.x);
                    hbw = bw;
                    t = hd_tex_mag(mp, fp, usea);
                    pend = false;
                }
                if (u.w < 0) {
                    break;
                }
            }
        }
        if (on) {
            let s = hd_shade(pb, o, t);
            if (s.ok) {
                if (np != pl) {
                    if (pl != 99u) {
                        hd_st(pl, idx, acc);
                    }
                    acc = hd_ld(np, idx);
                    pl = np;
                }
                let c = s.c;
                if (np == 4u) {
                    // The twin target holds GS pixels (put in the 512 path: blend of the colour with the target's colour, the alpha written is
                    // the prim's, FBA sets its top bit).
                    var o = vec4<f32>(c.rgb, c.a);
                    if ((at & 0x40u) != 0u && !((prims[pb + R_PABE] & 1u) != 0u && c.a < 128.0)) {
                        let al = prims[pb + R_ALPHA];
                        let sa = al & 3u;
                        let sb = (al >> 2u) & 3u;
                        let sd = (al >> 6u) & 3u;
                        var cf = f32(prims[pb + R_ALPHA + 1u] & 0xffu) / 128.0;
                        switch (al >> 4u) & 3u {
                            case 0u: {
                                cf = c.a / 128.0;
                            }
                            case 1u: {
                                cf = acc.a / 128.0;
                            }
                            default: {}
                        }
                        let ks = cf * f32(sa == 0u) - cf * f32(sb == 0u) + f32(sd == 0u);
                        let kd = cf * f32(sa == 1u) - cf * f32(sb == 1u) + f32(sd == 1u);
                        o = vec4<f32>(ks * c.rgb + kd * acc.rgb, c.a);
                    }
                    o = vec4<f32>(clamp(o.rgb, vec3<f32>(0.0), vec3<f32>(255.0)), clamp(o.a, 0.0, 255.0));
                    if ((prims[pb + R_FBA] & 1u) != 0u) {
                        o.a = max(o.a, 128.0);
                    }
                    acc = o;
                } else if ((at & 0x40u) == 0u || ((prims[pb + R_PABE] & 1u) != 0u && c.a < 128.0)) {
                    acc = vec4<f32>(c.rgb, 0.0);
                } else {
                    let al = prims[pb + R_ALPHA];
                    let sa = al & 3u;
                    let sb = (al >> 2u) & 3u;
                    let sd = (al >> 6u) & 3u;
                    var cf = f32(prims[pb + R_ALPHA + 1u] & 0xffu) / 128.0;
                    switch (al >> 4u) & 3u {
                        case 0u: {
                            cf = c.a / 128.0;
                        }
                        case 1u: {
                            cf = 1.0;
                        }
                        default: {}
                    }
                    let ks = cf * f32(sa == 0u) - cf * f32(sb == 0u) + f32(sd == 0u);
                    let kd = cf * f32(sa == 1u) - cf * f32(sb == 1u) + f32(sd == 1u);
                    if (!(ks < 0.0)) {
                        acc = vec4<f32>(ks * c.rgb + kd * acc.rgb, acc.a * kd);
                    }
                }
            }
        }
    }
    if (pl != 99u) {
        hd_st(pl, idx, acc);
    }
}
