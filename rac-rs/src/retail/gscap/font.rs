//! Button glyphs option (retail/glyphs.rs) in the GS capture: the font sheet's ten button cells are rewritten in the sheet the retail
//! code uploads (`patch_font`, called by `flush_work` before a waiting transfer is written). Retail re-sends the three font sheets
//! every frame (fx textures are uploaded lazily on first use in a frame, `sky_frame_begin` resets the upload cursor), so a style or
//! rebind change shows with the next upload and nothing is ever written into GS memory from outside an upload: no transfer is
//! started from inside a GPU-capture job (DeSynkro's freeze guard, b1dfb689). Default style + default bindings: the function returns
//! after three integer compares and `glyphs::wanted()` (one uncontended lock of the live bindings), no copy.

use super::*;
use crate::retail::glyphs as gl;

impl Gs {
    /// A waiting transfer is about to be written: if it is one of the level's three font sheets (256x128 PSMT8, recognised by its
    /// texels) and the cells are not retail's own, put the cells of the style into it. Returns the sheet as retail sent it when a
    /// textures mod is on (`flush_work` names the texture for a pack from the unpatched texels, so a pack made from the retail
    /// sheet still matches; a pack that replaces the sheet shows its own picture and the cells are not drawn).
    pub(super) fn patch_font(&mut self, p: &mut Pend) -> Option<Vec<u128>> {
        let (rw, rh) = ((p.rr & 0xfff) as u32, ((p.rr >> 32) & 0xfff) as u32);
        let dpsm = ((p.bb >> 56) & 0x3f) as u32;
        if dpsm != 0x13 || rw != 256 || rh != 128 || p.data.len() < 256 * 128 / 16 {
            return None;
        }
        let want = gl::wanted();
        if gl::is_retail(&want) {
            return None;
        }
        let patch = gl::patch()?;
        let which = patch.find(&p.data)?;
        let cells = gl::cells_for(&patch, which, &want);
        let orig = crate::texpack::hudrepl::enabled().then(|| p.data.clone());
        gl::patch_qwords(&mut p.data, &cells);
        orig
    }
}
