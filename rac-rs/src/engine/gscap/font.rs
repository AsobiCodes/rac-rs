//! Button glyphs option (R&C1 rac1/retail/glyphs.rs, through the pack's `font_patch`) in the GS capture: the font sheet's ten button cells are rewritten in the sheet the retail
//! code uploads (`patch_font`, called by `flush_work` before a waiting transfer is written). Retail re-sends the three font sheets
//! every frame (fx textures are uploaded lazily on first use in a frame, `sky_frame_begin` resets the upload cursor), so a style or
//! rebind change shows with the next upload and nothing is ever written into GS memory from outside an upload: no transfer is
//! started from inside a GPU-capture job (DeSynkro's freeze guard, b1dfb689). Default style + default bindings: the function returns
//! after three integer compares and `glyphs::wanted()` (one uncontended lock of the live bindings), no copy.

use super::*;

/// The game's button-glyph patcher (R&C1: `rac1/retail/glyphs.rs`, the pack's `font_patch`). Given the sheet's
/// 256x128 PSMT8 qwords it puts the cells of the chosen style into them when the sheet is a font sheet and the style is not retail's
/// own; it returns the sheet as retail sent it when it patched and `want_orig` is set, else `None`.
pub type FontPatch = fn(data: &mut [u128], want_orig: bool) -> Option<Vec<u128>>;

impl Gs {
    /// A waiting transfer is about to be written: if it is one of the level's three font sheets (256x128 PSMT8, recognised by its
    /// texels) and the cells are not retail's own, put the cells of the style into it. Returns the sheet as retail sent it when a
    /// textures mod is on (`flush_work` names the texture for a pack from the unpatched texels, so a pack made from the retail
    /// sheet still matches; a pack that replaces the sheet shows its own picture and the cells are not drawn).
    pub(super) fn patch_font(&mut self, p: &mut Pend) -> Option<Vec<u128>> {
        let (rw, rh) = ((p.rr & 0xfff) as u32, ((p.rr >> 32) & 0xfff) as u32);
        let dpsm = ((p.bb >> 56) & 0x3f) as u32;
        if dpsm != 0x13 || !matches!((rw, rh), (256, 128) | (512, 64)) || p.data.len() < 256 * 128 / 16 {
            return None;
        }
        let hook = crate::pack::get().font_patch?;
        hook(&mut p.data, crate::texpack::hudrepl::enabled())
    }
}
