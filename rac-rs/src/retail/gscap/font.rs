//! The font sheet's ten button cells: the Xbox/Switch artwork is written into the sheet the retail game uploads
//! (`patch_font`) and into the sheet already in the graphics memory when the style or the bindings change (`repatch_font`).
//! `use super::*` reaches the capture's private items.

use super::*;

impl Gs {
    /// The font sheet's button cells get the Xbox artwork: the transfer that carries the font is recognised by
    /// its format, its size and its content, and the artwork's indices replace the ten cells in it. The transfer's
    /// data is in the order the texels come (row by row over the rectangle), so a texel of the sheet is at
    /// `(y - dy) * rw + (x - dx)` in it.
    pub(super) fn patch_font(&mut self, p: &mut Pend) {
        let (dx, dy) = (((p.pos >> 32) & 0x7ff) as u32, ((p.pos >> 48) & 0x7ff) as u32);
        let (rw, rh) = ((p.rr & 0xfff) as u32, (p.rr >> 32) as u32);
        let (dbp, dbw, dpsm) = (((p.bb >> 32) & 0x3fff) as u32, ((p.bb >> 48) & 0x3f) as u32, ((p.bb >> 56) & 0x3f) as u32);
        if self.font_log < 500 && Self::glyph_debug() {
            self.font_log += 1;
            eprintln!("glyph: upload psm={dpsm:#x} {rw}x{rh} block={dbp} at {dx},{dy}");
        }
        let Some(font) = crate::retail::xglyphs::patch() else { return };
        if font.cells.len() < 3 {
            return;
        }
        if dpsm != 0x13 || rw != 256 || rh != 128 {
            return;
        }
        // The transfer's texels, in the order they come.
        let mut bytes = Vec::with_capacity(p.data.len() * 16);
        for &q in &p.data {
            for k in 0..16 {
                bytes.push(((q >> (8 * k)) & 0xff) as u8);
            }
        }
        // The sheet is one of the game's three fonts: it is recognised by its own texels.
        let Some(which) = font.sheets.iter().position(|s| s.len() == bytes.len().min(256 * 128) && bytes[..s.len()] == s[..]) else {
            if Self::glyph_debug() {
                eprintln!("glyph: not a font sheet ({} texels, {} known)", bytes.len(), font.sheets.len());
            }
            return;
        };
        // The sheet is in the graphics memory now: remember where, so a change of style can put the other
        // artwork in it.
        self.font_at = Some((dbp, dbw, dx, dy, which));
        let style = crate::retail::xglyphs::style().min(2) as usize;
        let map = crate::retail::binds::pad_map();
        self.font_style = style as u8;
        self.font_map = map;
        let cells = font.cells_for(which, style, &map);
        Self::patch_cells(p, &cells, dx, dy, rw);
        crate::retail::xglyphs::set_artwork_in_place(style != 0);
        if !self.font_said {
            self.font_said = true;
            eprintln!(
                "retail: the font sheet was found in the graphics memory{}",
                if style == 0 { "" } else { ", the button artwork is in it" }
            );
        }
    }

    /// RAC_GLYPH_DEBUG=1 (debug): a line per upload, so a run that does not find the font sheet says why.
    fn glyph_debug() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var("RAC_GLYPH_DEBUG").is_ok_and(|v| v == "1"))
    }

    /// The artwork's indices in the ten cells of the sheet, in the transfer's data.
    fn patch_cells(p: &mut Pend, cells: &[Vec<u8>], dx: u32, dy: u32, rw: u32) {
        for (k, cell) in cells.iter().enumerate() {
            let u = crate::retail::xglyphs::CELLS[k].1 as u32;
            for y in 0..crate::retail::xglyphs::CELL_H as u32 {
                for x in 0..crate::retail::xglyphs::CELL_W as u32 {
                    let i = ((96 + y - dy) * rw + (u + x - dx)) as usize;
                    let (q, k8) = (i / 16, i % 16);
                    if q >= p.data.len() || cell.len() <= (y * crate::retail::xglyphs::CELL_W as u32 + x) as usize {
                        continue;
                    }
                    let v = cell[(y * crate::retail::xglyphs::CELL_W as u32 + x) as usize] as u128;
                    p.data[q] = (p.data[q] & !(0xffu128 << (8 * k8))) | (v << (8 * k8));
                }
            }
        }
    }

    /// The style or the bindings changed: the sheet in the graphics memory gets the other artwork. The sheet's place is
    /// known from its upload (`font_at`).
    pub fn repatch_font(&mut self) {
        let Some((dbp, dbw, dx, dy, which)) = self.font_at else { return };
        let style = crate::retail::xglyphs::style().min(2) as usize;
        let map = crate::retail::binds::pad_map();
        if style as u8 == self.font_style && map == self.font_map {
            return;
        }
        let Some(font) = crate::retail::xglyphs::patch() else { return };
        if font.cells.len() < 3 {
            return;
        }
        self.font_style = style as u8;
        self.font_map = map;
        let cells = font.cells_for(which, style, &map);
        // [PC] The GPU capture builds its transfer stream per job (0.4.0's lazy readback): a transfer written from inside a
        // job breaks it and the overlay stops updating (the pause menu stays on the screen). The artwork the sheet was
        // uploaded with stays; the viewer's own copy of the font sheet is re-blitted instead (viewer/handler.rs).
        if self.gpu.is_some() {
            return;
        }
        // The sheet's texels, read back, with the cells of the style in it (the PlayStation style's cells are the sheet's own).
        let mut sheet = vec![0u8; 256 * 128];
        for y in 0..128u32 {
            for x in 0..256u32 {
                let v = self.rd_px(0x13, dbp, dbw, dx + x, dy + y);
                sheet[(y * 256 + x) as usize] = v as u8;
            }
        }
        for (k, cell) in cells.iter().enumerate() {
            let u = crate::retail::xglyphs::CELLS[k].1 as u32;
            for y in 0..crate::retail::xglyphs::CELL_H as u32 {
                for x in 0..crate::retail::xglyphs::CELL_W as u32 {
                    sheet[((96 + y) * 256 + u + x) as usize] =
                        cell[(y * crate::retail::xglyphs::CELL_W as u32 + x) as usize];
                }
            }
        }
        for y in 0..128u32 {
            for x in 0..256u32 {
                self.wr_px(0x13, dbp, dbw, dx + x, dy + y, sheet[(y * 256 + x) as usize] as u32);
            }
        }
        // The GPU capture holds its own copy of local memory.
        if self.gpu.is_some() {
            let mut data = Vec::with_capacity(sheet.len() / 16);
            for q in sheet.chunks(16) {
                let mut w = 0u128;
                for (k, b) in q.iter().enumerate() {
                    w |= (*b as u128) << (8 * k);
                }
                data.push(w);
            }
            let bb = ((dbp as u64) << 32) | ((dbw as u64) << 48) | (0x13u64 << 56);
            let pos = ((dx as u64) << 32) | ((dy as u64) << 48);
            let rr = 256u64 | (128u64 << 32);
            self.gpu_xfer(bb, pos, rr, 0, &data);
        }
        crate::retail::xglyphs::set_artwork_in_place(style != 0);
    }

}
