//! Position-keyed sphere zones that tint a moby: a flat list of influence spheres (with per-sphere colour matrices)
//! and a 64-wide grid of per-cell sphere lists. `zone_tint` blends the zone colours into the moby's two colour words.
use crate::ps2::*;
use crate::rcall::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Count of flat zones (word), the zones themselves (16 bytes: centre xyz, w = radius) and their 0x80-byte
/// records (matrix rows 0..3, colours at +0x40 / +0x44, mix words +0x48 / +0x4c, flags at +0x50).
const FLAT_COUNT: u32 = 0x15_fb44;
const FLAT_ZONES: u32 = 0x17_edc0;
const FLAT_RECORDS: u32 = 0x17_efc0;
/// Grid: pointer to the cell offset table (64 columns, word per cell) and pointer to the grid zones (32 bytes each,
/// radius at +0xc, colour word at +0x10).
const GRID_TABLE: u32 = 0x15_fb40;
const GRID_ZONES: u32 = 0x15_fb00;

/// 0x2496a8 flat_zone_hit(pos, t_out*, index_out*) -> 0/1: first flat zone (in list order, including one extra
/// entry past the count) whose xy distance is within its radius; stores the zone index and
/// t = 0.5 * (x of pos transformed by the zone matrix + 1).
pub fn flat_zone_hit(c: &mut Cpu) -> bool {
    let (pos, t_out, idx_out) = (c.arg(0), c.arg(1), c.arg(2));
    let hit = zone_hit(c, pos, t_out, idx_out);
    c.ret(hit as u32);
    true
}

fn zone_hit(c: &mut Cpu, pos: u32, t_out: u32, idx_out: u32) -> bool {
    let count = c.r32(FLAT_COUNT);
    // vaddw.xyz vf1, vf0, vf0w: VF1.xyz = 0 + 1, w kept.
    let old1 = c.vf(1);
    c.set_vf(1, V([F::ONE.0, F::ONE.0, F::ONE.0, old1.0[3]]));
    // lqc2 vf10 <- pos; vaddx.w vf10, vf0, vf0x: w = 1 + 0.
    let p = c.rv(pos);
    let vf10 = V([p.0[0], p.0[1], p.0[2], F::ONE.vu_add(F::ZERO).0]);
    let mut vf11 = c.vf(11);
    let mut vf12 = c.vf(12);
    let mut vf13 = c.vf(13);
    let mut vf14 = c.vf(14);
    let mut vf15 = c.vf(15);
    let mut hit_at = None;
    for i in 0..=count {
        // lqc2 vf11 <- zone
        let zone = c.rv(FLAT_ZONES + 16 * i);
        vf11 = zone;
        // vsub.xyzw vf12, vf10, vf11
        vf12 = V(std::array::from_fn(|l| F(vf10.0[l]).vu_sub(F(zone.0[l])).0));
        // vaddw.xyz vf14, vf0, vf11w: xyz = 0 + zone.w, w kept
        let zw = F::ZERO.vu_add(F(zone.0[3])).0;
        vf14 = V([zw, zw, zw, vf14.0[3]]);
        // vmul.xyz vf12, vf12, vf12
        let sq: [u32; 3] = std::array::from_fn(|l| F(vf12.0[l]).vu_mul(F(vf12.0[l])).0);
        vf12 = V([sq[0], sq[1], sq[2], vf12.0[3]]);
        // vaddy.x vf13, vf12, vf12y
        vf13 = V([F(vf12.0[0]).vu_add(F(vf12.0[1])).0, vf13.0[1], vf13.0[2], vf13.0[3]]);
        // vsub.x vf15, vf13, vf14
        vf15 = V([F(vf13.0[0]).vu_sub(F(vf14.0[0])).0, vf15.0[1], vf15.0[2], vf15.0[3]]);
        // qmfc2 + srl 31: the sign of vf15.x selects the hit path
        if vf15.0[0] >> 31 != 0 {
            hit_at = Some(i);
            break;
        }
    }
    c.set_vf(10, vf10);
    c.set_vf(11, vf11);
    c.set_vf(12, vf12);
    c.set_vf(13, vf13);
    c.set_vf(14, vf14);
    c.set_vf(15, vf15);
    let i = match hit_at {
        Some(i) => i,
        None => return false,
    };
    // Hit path: t4 is FLAT_RECORDS for every i (retail advances and undoes t4 in the loop), so the record is fixed.
    let rec = FLAT_RECORDS;
    let (r1, r2, r3, r4) = (c.rv(rec), c.rv(rec + 0x10), c.rv(rec + 0x20), c.rv(rec + 0x30));
    c.set_vf(1, r1);
    c.set_vf(2, r2);
    c.set_vf(3, r3);
    c.set_vf(4, r4);
    // vmulax ACC = vf1 * vf10.x; vmadday ACC += vf2 * vf10.y; vmaddaz ACC += vf3 * vf10.z; vmaddw vf6 = ACC + vf4 * vf10.w
    let acc = r1 * F(vf10.0[0]);
    let acc = V::madd(acc, r2, V::splat(F(vf10.0[1])));
    let acc = V::madd(acc, r3, V::splat(F(vf10.0[2])));
    let vf6 = V::madd(acc, r4, V::splat(F(vf10.0[3])));
    c.set_acc(acc);
    c.set_vf(6, vf6);
    // vclipw.xyz vf0, vf6; cfc2 clip flags
    if vecmath::clip_bits(vf6) & 0x3f != 0 {
        return false;
    }
    // qmtc2 vf2 <- 0x3f000000 (upper words zero); vaddw.x vf6 = vf6.x + 1; vmul.x vf6 = vf2.x * vf6.x
    c.set_vf(2, V([0x3f00_0000, 0, 0, 0]));
    let x = F(0x3f00_0000).vu_mul(F(vf6.0[0]).vu_add(F::ONE));
    c.set_vf(6, V([x.0, vf6.0[1], vf6.0[2], vf6.0[3]]));
    c.w32(idx_out, i);
    c.wf(t_out, x);
    true
}

/// 0x2497dc grid_zone_lookup(pos, d2_out*) -> zone pointer or 0: finds the grid cell of pos (16-unit cells, 64
/// per row) and returns the first listed zone whose xy distance covers pos, storing dx^2 + dy^2.
pub fn grid_zone_lookup(c: &mut Cpu) -> bool {
    let (pos, out) = (c.arg(0), c.arg(1));
    match grid_lookup(c, pos, out) {
        Some(zone) => c.ret(zone),
        None => c.ret(0),
    }
    true
}

fn grid_lookup(c: &mut Cpu, pos: u32, out: u32) -> Option<u32> {
    let p = c.rv(pos);
    let (xi, yi) = ((p.x().to_int() as u32) >> 4, (p.y().to_int() as u32) >> 4);
    let table = c.r32(GRID_TABLE);
    let cell = table.wrapping_add((yi << 6).wrapping_add(xi) << 2);
    let off = c.r32(cell);
    if off == 0 {
        return None;
    }
    let list = off.wrapping_add(table);
    let count = c.r32(list);
    let zones = c.r32(GRID_ZONES);
    // lqc2 vf10 <- pos: the raw quadword (retail does not reset w here).
    let vf10 = p;
    let mut vf11 = c.vf(11);
    let mut vf12 = c.vf(12);
    let mut vf13 = c.vf(13);
    let mut vf14 = c.vf(14);
    let mut vf15 = c.vf(15);
    let mut found = None;
    for k in 0..count {
        let zone_ptr = zones.wrapping_add(c.r32(list + 4 + 4 * k) << 5);
        // lqc2 vf11 <- zone
        let zone = c.rv(zone_ptr);
        vf11 = zone;
        // vsub.xyzw vf12, vf10, vf11
        let diff = V(std::array::from_fn(|l| F(vf10.0[l]).vu_sub(F(zone.0[l])).0));
        // vmul.xyz vf12, vf12, vf12 (w kept)
        let sq: [u32; 3] = std::array::from_fn(|l| F(diff.0[l]).vu_mul(F(diff.0[l])).0);
        vf12 = V([sq[0], sq[1], sq[2], diff.0[3]]);
        // vaddy.x vf13, vf12, vf12y
        vf13 = V([F(vf12.0[0]).vu_add(F(vf12.0[1])).0, vf13.0[1], vf13.0[2], vf13.0[3]]);
        // vaddw.xyz vf14, vf0, vf11w: xyz = 0 + zone.w, w kept
        let zw = F::ZERO.vu_add(F(zone.0[3])).0;
        vf14 = V([zw, zw, zw, vf14.0[3]]);
        // vsub.x vf15, vf13, vf14
        vf15 = V([F(vf13.0[0]).vu_sub(F(vf14.0[0])).0, vf15.0[1], vf15.0[2], vf15.0[3]]);
        if vf15.0[0] >> 31 != 0 {
            found = Some((zone_ptr, F(vf13.0[0])));
            break;
        }
    }
    c.set_vf(10, vf10);
    c.set_vf(11, vf11);
    c.set_vf(12, vf12);
    c.set_vf(13, vf13);
    c.set_vf(14, vf14);
    c.set_vf(15, vf15);
    let (zone_ptr, d2) = found?;
    c.wf(out, d2);
    Some(zone_ptr)
}

/// Per-byte blend (a * (255 - mix) + b * mix) >> 8 with retail's halfword products and saturating byte add.
fn blend_bytes(a: u32, b: u32, mix: i32) -> u32 {
    let (ma, mb) = ((255 - mix) as i16 as i32, mix as i16 as i32);
    let mut out = 0;
    for i in 0..4 {
        let (ca, cb) = ((a >> (8 * i)) & 0xff, (b >> (8 * i)) & 0xff);
        let pa = (ma * ca as i32) as u32;
        let pb = (mb * cb as i32) as u32;
        let hi = ((pa >> 8) & 0xff) + ((pb >> 8) & 0xff);
        out |= hi.min(255) << (8 * i);
    }
    out
}

/// 0x2498ec zone_tint(moby): looks up the flat zone under moby+0x10; when its flag bit 0 is set blends the zone's two
/// colours by t into moby+0x80 and packs the zone's mix words into moby+0x38. Then a grid zone sets moby+0x3c
/// to its colour darkened by distance added to moby+0x80; without one moby+0x3c = moby+0x80.
pub fn zone_tint(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let (sp, f) = push_frame(c, 0x20);
    if zone_hit(c, m + 0x10, f, f + 4) {
        let rec = FLAT_RECORDS + (c.r32(f + 4) << 7);
        if c.r32(rec + 0x50) & 1 != 0 {
            let t = c.rf(f);
            let mix = (F(0x437f_0000) * t).to_int();
            let (a, b) = (c.r32(rec + 0x40), c.r32(rec + 0x44));
            c.w32(m + 0x80, blend_bytes(a, b, mix));
            let packed = c.r32(rec + 0x48) | (c.r32(rec + 0x4c) << 8) | ((mix as u32) << 16);
            c.w32(m + 0x38, packed);
        }
    }
    let found = grid_lookup(c, m + 0x10, f);
    match found {
        None => {
            let base = c.r32(m + 0x80);
            c.w32(m + 0x3c, base);
            pop_frame(c, &sp);
            c.ret(0);
        }
        Some(zone) => {
            let ratio = c.rf(f) / c.rf(zone + 0xc);
            let mix = (ratio * F(0x437f_0000)).to_int();
            let m255 = (255 - mix) as i16 as i32;
            let colour = c.r32(zone + 0x10);
            let mut dark = 0u32;
            for i in 0..4 {
                let p = (m255 * ((colour >> (8 * i)) & 0xff) as i32) as u32;
                dark |= ((p >> 8) & 0xff) << (8 * i);
            }
            let base = c.r32(m + 0x80);
            let mut sum = 0u32;
            for i in 0..4 {
                let s = ((dark >> (8 * i)) & 0xff) + ((base >> (8 * i)) & 0xff);
                sum |= s.min(255) << (8 * i);
            }
            c.w32(m + 0x3c, sum);
            pop_frame(c, &sp);
            c.ret(zone);
        }
    }
    true
}
