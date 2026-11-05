//! The local-memory span a GPU-captured prim samples (moved out of `gpu_raster` in gpu.rs to keep that fn under its size cap).

use super::super::Gs;
use super::St;

/// The pair of local-memory blocks a prim's texture can reach (`Gs::span` of the texel box `wrapc` in gpu.wgsl can give), or None for an
/// untextured prim. `memo` keeps the last (tex0, clamp) -> span, because consecutive prims repeat the same texture.
pub(super) fn sampled_span(memo: &mut Option<((u64, u64), (u32, u32))>, st: &St) -> Option<(u32, u32)> {
    if st.at & 0x10 == 0 {
        return None;
    }
    let tex0 = st.tex0;
    if let Some((k, v)) = *memo {
        if k == (tex0, st.clamp) {
            return Some(v);
        }
    }
    let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
    let cl = st.clamp;
    // The texel coordinates `wrapc` (gpu.wgsl) can give, first and last: region clamp keeps them in
    // [min(MIN, MAX), MAX], region repeat in [MAX, MIN | MAX] (a 1024-wide guess ran past the end of local memory
    // and claimed all of it, so the map's icons near the top of memory went serial).
    let reach = |m: u64, size: u32, lo: u32, hi: u32| match m & 3 {
        2 => (lo.min(hi), hi),
        3 => (hi, lo | hi),
        _ => (0, size - 1),
    };
    let (x0, x1) = reach(cl, tw, ((cl >> 4) & 0x3ff) as u32, ((cl >> 14) & 0x3ff) as u32);
    let (y0, y1) = reach(cl >> 2, th, ((cl >> 24) & 0x3ff) as u32, ((cl >> 34) & 0x3ff) as u32);
    let v = Gs::span(((tex0 >> 20) & 0x3f) as u32, (tex0 & 0x3fff) as u32, ((tex0 >> 14) & 0x3f) as u32, x0, y0, x1 - x0 + 1, y1 - y0 + 1);
    *memo = Some(((tex0, st.clamp), v));
    Some(v)
}
