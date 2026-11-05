//! Widescreen (16:9) for the GS-full display: `--wide` / RAC_WIDE=1.
//!
//! The game draws a 4:3 view into its 512x448 buffer and the display stretches it. For 16:9 the picture is drawn with a
//! wider horizontal field of view in the same buffer (squeezed 0.75 horizontally; the window stretches it back), the
//! vertical view stays as it was, and the 2D HUD is squeezed back by the GS-level rule (`apply_hud`).
//!
//! The field of view itself is the game's: R&C1 wraps its FOV function in a hand (`rac1/retail/wide_fov.rs`, stage E2: moved
//! out of this file), registered by the retail worker when 16:9 is on.
use super::gs::Gs;
use std::sync::atomic::{AtomicBool, Ordering};

/// Wide view requested (`--wide`; RAC_WIDE=1 does the same).
static WIDE: AtomicBool = AtomicBool::new(false);

pub fn enable() {
    WIDE.store(true, Ordering::SeqCst);
}

/// Whether the 16:9 view is on (default off).
pub fn enabled() -> bool {
    WIDE.load(Ordering::SeqCst) || std::env::var("RAC_WIDE").is_ok_and(|v| v != "0" && !v.is_empty())
}

/// The picture's display aspect.
pub const ASPECT: f32 = 16.0 / 9.0;

/// The GS-level HUD rule: prims drawn after the last PATH1 (VU1) draw of a field are squeezed horizontally by 0.75
/// toward the nearer edge, except prims that read the frame buffer itself.
pub fn apply_hud(g: &mut Gs) {
    g.hud_squeeze = 0.75;
    g.hud_mask = 0;
}
