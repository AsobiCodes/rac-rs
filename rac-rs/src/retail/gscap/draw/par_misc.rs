//! Pool helper of the CPU rasteriser: the RGBA8 conversion of a published picture, its rows converted on the pool (the same
//! bytes as the one-thread conversion; vendor 0.77 -> 0.44 ms/field on 8 threads).

use super::{on_pool, px8, Rows, BLANK};
use rayon::prelude::*;

/// Pixels from which `to_rgba` converts its rows on the pool.
const RGBA_PAR_PX: usize = 1 << 16;

/// The first `rows` rows of a picture as RGBA8; the rows that are not dirty are blank and become the constant.
pub(super) fn to_rgba(pic: &[[f32; 4]], w: u32, rows: u32, dirty: &Rows) -> Vec<[u8; 4]> {
    let w = w as usize;
    let blank = px8(&BLANK);
    if w > 0 && pic.len() >= w * rows as usize && w * rows as usize >= RGBA_PAR_PX {
        let mut out = vec![blank; w * rows as usize];
        on_pool(|| {
            out.par_chunks_mut(w).zip(pic.par_chunks(w)).enumerate().for_each(|(y, (o, r))| {
                if dirty.has(y as i32) {
                    for (d, s) in o.iter_mut().zip(r) {
                        *d = px8(s);
                    }
                }
            })
        });
        return out;
    }
    to_rgba_serial(pic, w, rows, dirty)
}

/// The one-thread conversion (the reference of the tests).
pub(super) fn to_rgba_serial(pic: &[[f32; 4]], w: usize, rows: u32, dirty: &Rows) -> Vec<[u8; 4]> {
    let blank = px8(&BLANK);
    let mut out = Vec::with_capacity(w * rows as usize);
    for (y, r) in pic.chunks(w).take(rows as usize).enumerate() {
        if dirty.has(y as i32) {
            out.extend(r.iter().map(px8));
        } else {
            out.resize(out.len() + w, blank);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pool conversion gives the serial conversion's bytes, dirty and blank rows mixed.
    #[test]
    fn pool_rgba_matches_serial() {
        let (w, h) = (640usize, 200usize);
        let mut s = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let pic: Vec<[f32; 4]> = (0..w * (h + 7)).map(|_| std::array::from_fn(|k| (next() % 600) as f32 - 40.0 + if k == 3 { 0.37 } else { 0.0 })).collect();
        let mut dirty = Rows::default();
        for y in 0..h as i32 {
            if y % 3 != 1 {
                dirty.mark(y, y);
            }
        }
        for rows in [h as u32, 150, 1] {
            assert_eq!(to_rgba(&pic, w as u32, rows, &dirty), to_rgba_serial(&pic, w, rows, &dirty), "rows {rows}");
        }
    }
}
