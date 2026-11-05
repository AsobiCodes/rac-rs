//! Whether two pixels a capture step writes can meet in local memory (the same word): such a step runs on one thread or in the
//! CPU's order (`Gpu::prim` serial, `k_xfer`'s serial branch), any other on many threads at once. Page layout as gsfull gs_mem.rs.

/// Page width and height in pixels and the buffer width in pages of a format `wr_px` writes; None: one it does not write.
fn page(psm: u32, bw: u32) -> Option<(u32, u32, u32)> {
    Some(match psm {
        0x00 | 0x01 | 0x1b | 0x24 | 0x2c => (64, 32, bw),
        0x02 => (64, 64, bw),
        0x13 => (128, 64, (bw / 2).max(1)),
        0x14 => (128, 128, (bw / 2).max(1)),
        _ => return None,
    })
}

/// Whether distinct pixels of a rectangle ending at (`x1`, `y1`) (exclusive) of a buffer never share memory: the rows
/// stay inside the buffer width (wider x wraps into another page row) and the buffer does not wrap at the end of local
/// memory. Formats `wr_px` does not write count as safe.
pub(super) fn alias_free(psm: u32, bp: u32, bw: u32, x1: u32, y1: u32) -> bool {
    let Some((pw, ph, bw)) = page(psm, bw) else { return true };
    bw > 0 && x1 <= pw * bw && bp as u64 * 256 + ((y1.max(1) - 1) / ph + 1) as u64 * bw as u64 * 8192 <= 0x40_0000
}

/// Whether no two pixels of a host->local transfer of `rw` x `rh` pixels at (`dx`, `dy`) land on the same memory (coordinates
/// wrap at 2048, gs_mem.rs `trx_xy`). The rectangle is cut into cells, one per page column and page row of the buffer; the
/// pixels of one cell sit at distinct places of its page, so two cells only meet when they fall on the same page (page numbers
/// wrap with local memory: 512 pages) with overlapping in-page boxes. Exact, so a transfer whose rows run past the buffer
/// width or wrap at 2048 (R&C2 keeps a stray TRXPOS for a while: every upload lands at x 1508, y 2047) still runs in parallel.
pub(super) fn trx_alias_free(psm: u32, bw: u32, dx: u32, dy: u32, rw: u32, rh: u32) -> bool {
    let Some((pw, ph, bwp)) = page(psm, bw) else { return true };
    if bwp == 0 || rw > 2048 || rh > 2048 {
        return false;
    }
    // The runs [a, b) of one axis, cut where the coordinate wraps.
    let runs = |s: u32, n: u32| if s + n <= 2048 { [(s, s + n), (0, 0)] } else { [(s, 2048), (0, s + n - 2048)] };
    let mut cells: Vec<(u32, [u32; 4])> = Vec::new();
    for (xa, xb) in runs(dx, rw).into_iter().filter(|r| r.0 < r.1) {
        for c in xa / pw..=(xb - 1) / pw {
            let (x0, x1) = (xa.max(c * pw) - c * pw, xb.min(c * pw + pw) - c * pw);
            for (ya, yb) in runs(dy, rh).into_iter().filter(|r| r.0 < r.1) {
                for b in ya / ph..=(yb - 1) / ph {
                    cells.push(((b * bwp + c) % 512, [x0, x1, ya.max(b * ph) - b * ph, yb.min(b * ph + ph) - b * ph]));
                }
            }
        }
    }
    cells.sort_unstable_by_key(|c| c.0);
    let apart = |a: &[u32; 4], b: &[u32; 4]| a[1] <= b[0] || b[1] <= a[0] || a[3] <= b[2] || b[3] <= a[2];
    cells.chunk_by(|a, b| a.0 == b.0).all(|g| g.iter().enumerate().all(|(i, a)| g[i + 1..].iter().all(|b| apart(&a.1, &b.1))))
}

#[cfg(test)]
mod tests {
    use super::trx_alias_free;
    use crate::gsfull::gs_mem::{a16, a32, a4, a8, luts, trx_xy};

    /// Whether two pixels of the transfer land on the same memory, as the software GS writes them (nibble addresses).
    fn meets(psm: u32, bw: u32, dx: u32, dy: u32, rw: u32, rh: u32) -> bool {
        let l = luts();
        let mut seen = std::collections::HashSet::new();
        (0..rh).any(|ty| {
            (0..rw).any(|tx| {
                let (x, y) = trx_xy(dx, dy, tx, ty);
                let a = match psm {
                    0x02 => a16(l, 0x123, bw, x, y) * 2,
                    0x13 => a8(l, 0x123, bw, x, y) * 2,
                    0x14 => a4(l, 0x123, bw, x, y),
                    _ => a32(l, 0x123, bw, x, y) * 2,
                };
                !seen.insert(a)
            })
        })
    }

    #[test]
    fn exact_against_the_addresses() {
        for psm in [0x00, 0x02, 0x13, 0x14] {
            for bw in [1, 3, 4, 10] {
                for (dx, dy) in [(0, 0), (1508, 2047), (2000, 2040), (2047, 0)] {
                    for rw in [1, 70, 300] {
                        for rh in [1, 70, 300] {
                            assert_eq!(trx_alias_free(psm, bw, dx, dy, rw, rh), !meets(psm, bw, dx, dy, rw, rh), "psm {psm:x} bw {bw} at {dx},{dy} {rw}x{rh}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rc2_stray_trxpos_upload_is_parallel() {
        assert!(trx_alias_free(0x13, 4, 1508, 2047, 256, 256));
        assert!(!trx_alias_free(0x00, 1, 0, 0, 128, 64));
    }
}
