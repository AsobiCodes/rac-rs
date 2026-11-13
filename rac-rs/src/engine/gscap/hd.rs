//! The sharp HUD's size: the window-resolution overlay's scale and rows (`set_hd_target`, read by the capture's GPU path
//! `draw/gpu.rs` and R&C2's `rc2.rs`); moved out of gscap.rs whole (size limit, dev s138 update).

/// The sharp HUD's scale, set by the main thread every frame (`set_hd_target`): the GS pixel is `kx / 256` by `ky / 256` window
/// pixels, and the frame has `rows` rows; packed `kx | ky << 16 | rows << 32`; 0 = off (window too small, `hud=original`,
/// RAC_PC_EXT=0).
static HD_TARGET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Rows of the last published frame (the main buffer's scissor height: 448 in play on both discs, other in some screens): the
/// next frame's sharp scale assumes them (a frame with other rows has no sharp picture, then the scale follows).
static HD_SEEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The highest window height the sharp HUD draws at (the planes' size, `draw/gpu.rs` `HPS`); taller windows scale it up.
pub const HD_MAX_H: u32 = 1536;

/// (kx, ky, rows) of the sharp HUD, 0 = off.
pub(crate) fn hd_target() -> (u32, u32, u32) {
    let t = HD_TARGET.load(std::sync::atomic::Ordering::Relaxed);
    ((t & 0xffff) as u32, ((t >> 16) & 0xffff) as u32, (t >> 32) as u32)
}

/// A frame of `rows` rows was published.
pub(crate) fn hd_note_rows(rows: u32) {
    HD_SEEN.store(rows, std::sync::atomic::Ordering::Relaxed);
}

/// R&C2: the next frame's HUD buffer has `rows` rows (the last overlay field's scissor): the scale `set_hd_target` sets follows them.
pub fn hd_expect_rows(rows: u32) {
    hd_note_rows(rows);
}

/// Rows of the game's frame until the first one is published: 448 on the USA disc, 512 on PAL (`draw.rs disp_rows`).
fn hd_rows() -> u32 {
    if ee_oracle::ver::ntsc() {
        448
    } else {
        512
    }
}

/// The fewest output (window) pixels per GS row at which the sharp HUD is drawn; a smaller window gets the 512 picture as before.
/// Below 2.0 the second picture adds little (a 1280 x 800 Steam Deck window has 1.79 per row of the 448-row frame, 1080p has 2.41),
/// and its fill cost (the pause menu and the ring, 0.5 - 2.2 ms on a fast desktop GPU) is not worth it on a slow one.
pub const HD_MIN_PX_PER_ROW: f32 = 2.0;

/// The sharp HUD for a window `window_h` pixels high (`on`: the option and the extension allow it). Off below `HD_MIN_PX_PER_ROW`
/// window pixels per GS row; above `HD_MAX_H` the picture is drawn there and scaled up.
pub fn set_hd_target(window_h: u32, on: bool) {
    set_hd_target_with_min(window_h, on, HD_MIN_PX_PER_ROW);
}

/// Localized title cards also benefit at smaller window sizes.
pub fn set_hd_caption_target(window_h: u32, on: bool) {
    set_hd_target_with_min(window_h, on, 1.0);
}

fn set_hd_target_with_min(window_h: u32, on: bool, min_pixels_per_row: f32) {
    let seen = HD_SEEN.load(std::sync::atomic::Ordering::Relaxed);
    let rows = if (64..=512).contains(&seen) { seen } else { hd_rows() };
    let t = if on && window_h as f32 >= rows as f32 * min_pixels_per_row {
        let h = window_h.min(HD_MAX_H);
        (h * 2 / 3) as u64 | ((h * 256 / rows) as u64) << 16 | (rows as u64) << 32
    } else {
        0
    };
    HD_TARGET.store(t, std::sync::atomic::Ordering::Relaxed);
}

/// The size of the sharp HUD planes for a scale: width 512 GS pixels, `rows` rows.
pub(crate) fn hd_dims(kx: u32, ky: u32, rows: u32) -> (u32, u32) {
    (2 * kx, (rows * ky).div_ceil(256))
}
