//! R&C2: textures the R&C2 pack asks the software GS for: classes decoded from RAM (rac2 `scene::Scene::ram_classes`) carry the
//! GS TEX0 words the game wrote into their packets instead of level texture slots, so their texels are read from VRAM.
//! The renderer asks (`want`), the GS stage reads when it first draws with that TEX0 (`at_draw`; textures the game uploads
//! just before a draw and overwrites later in the field, like Ratchet's head at TBP 0) or, for one not drawn within a few
//! fields, after a field (`serve`); the renderer picks them up (`get`).
use super::gs::Gs;
use super::gs_mem::{clut_read, luts, rd_px, unpack_col};
use crate::level::Texture;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Wanted TEX0 words and the fields they have waited.
static WANT: Mutex<Vec<(u64, u32)>> = Mutex::new(Vec::new());
static ANY: AtomicBool = AtomicBool::new(false);
static HAVE: Mutex<Option<HashMap<u64, Arc<Texture>>>> = Mutex::new(None);

/// Ask for the texture of a TEX0 word (read at the end of the next field the GS draws).
pub fn want(tex0: u64) {
    if get(tex0).is_some() {
        return;
    }
    if let Ok(mut w) = WANT.lock() {
        if !w.iter().any(|e| e.0 == tex0) {
            w.push((tex0, 0));
            if std::env::var_os("RAC2_TEXLOG").is_some() {
                eprintln!("texreq: want {tex0:x}");
            }
            ANY.store(true, Ordering::Relaxed);
        }
    }
}

/// Is this TEX0 wanted (CLD ignored)?
pub fn wanted(tex0: u64) -> bool {
    pending() && WANT.lock().is_ok_and(|w| w.iter().any(|e| (e.0 ^ tex0) & !(7u64 << 61) == 0))
}

/// A skipped 3D draw (HUD overlay mode, vif Out::texmarks) would draw with this TEX0 here: the uploads so far are applied
/// and a wanted texture is read as at a draw.
pub fn mark(g: &mut Gs, tex0: u64) {
    if wanted(tex0) {
        g.flush_all();
        at_draw(g, [tex0, tex0]);
    }
}

pub fn get(tex0: u64) -> Option<Arc<Texture>> {
    HAVE.lock().ok()?.as_ref()?.get(&tex0).cloned()
}

/// Is any texture wanted (the GS's per-draw check)?
#[inline]
pub fn pending() -> bool {
    ANY.load(Ordering::Relaxed)
}

/// The GS is about to draw with these TEX0 words (TEX0_1, TEX0_2) and its uploads for them are applied: read any wanted
/// one now. CLD (bits 61..63) is ignored when matching.
pub fn at_draw(g: &mut Gs, regs: [u64; 2]) {
    let hit: Vec<u64> = match WANT.lock() {
        Ok(mut w) => {
            let mut hit = Vec::new();
            w.retain(|e| {
                let h = regs.iter().any(|&r| (r ^ e.0) & !(7u64 << 61) == 0);
                if h {
                    hit.push(e.0);
                }
                !h
            });
            ANY.store(!w.is_empty(), Ordering::Relaxed);
            hit
        }
        Err(_) => return,
    };
    if hit.is_empty() {
        return;
    }
    if std::env::var_os("RAC2_TEXLOG").is_some() {
        eprintln!("texreq: at draw {hit:x?} path {} prog {} hud {}", g.path, g.prog, g.hud);
    }
    let vram = g.vram().as_ptr();
    // SAFETY: as in `serve`.
    let out: Vec<_> = hit.into_iter().map(|t| (t, Arc::new(unsafe { read(vram, t) }))).collect();
    if let Ok(d) = std::env::var("RAC2_TEXDUMP") {
        for (t, x) in &out {
            let rgb = x.rgba.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
            let _ = std::fs::write(format!("{d}/tex_{t:016x}.png"), crate::render::png(&crate::render::Image { w: x.w, h: x.h, rgb }));
            let rgb = x.rgba.iter().flat_map(|p| [p[3], p[3], p[3]]).collect();
            let _ = std::fs::write(format!("{d}/alpha_{t:016x}.png"), crate::render::png(&crate::render::Image { w: x.w, h: x.h, rgb }));
        }
    }
    if let Ok(mut h) = HAVE.lock() {
        h.get_or_insert_with(HashMap::new).extend(out);
    }
}

/// Read wanted textures that no draw used for 3 fields from VRAM as it is after a field (GS stage thread).
pub fn serve(g: &mut Gs) {
    let want: Vec<u64> = match WANT.lock() {
        Ok(mut w) if !w.is_empty() => {
            w.iter_mut().for_each(|e| e.1 += 1);
            let old = w.iter().filter(|e| e.1 >= 3).map(|e| e.0).collect::<Vec<_>>();
            w.retain(|e| e.1 < 3);
            ANY.store(!w.is_empty(), Ordering::Relaxed);
            old
        }
        _ => return,
    };
    if want.is_empty() {
        return;
    }
    if std::env::var_os("RAC2_TEXLOG").is_some() {
        eprintln!("texreq: undrawn, read after the field {want:x?}");
    }
    let vram = g.vram().as_ptr();
    let mut out = Vec::new();
    for t in want {
        // SAFETY: `vram` is the GS's VRAM_SIZE bytes; `Gs::vram` finished every pending write.
        out.push((t, Arc::new(unsafe { read(vram, t) })));
    }
    if let Ok(mut h) = HAVE.lock() {
        h.get_or_insert_with(HashMap::new).extend(out);
    }
}

/// RGBA of a TEX0 (PSMCT32, PSMT8 / T8H and PSMT4 / T4HL / T4HH with a CT32 / CT16 CLUT; other formats white), PS2 alpha 0x80 = 255. TCC 0
/// (texture alpha unused) reads opaque.
unsafe fn read(m: *const u8, t: u64) -> Texture {
    let f = |sh: u32, bits: u32| ((t >> sh) & ((1 << bits) - 1)) as u32;
    let (tbp, tbw, psm) = (f(0, 14), f(14, 6), f(20, 6));
    let (w, h) = (1u32 << f(26, 4).min(10), 1u32 << f(30, 4).min(10));
    let (tcc, cbp, cpsm, csa) = (f(34, 1), f(37, 14), f(51, 4), f(56, 5));
    let t4 = matches!(psm, 0x14 | 0x24 | 0x2c);
    let mut clut = [[0u8; 4]; 512];
    let texa = 0x80 | 0x80 << 32;
    if matches!(psm, 0x13 | 0x14 | 0x1b | 0x24 | 0x2c) {
        clut_read(m, &mut clut, cbp, if cpsm & 2 != 0 { 0x02 } else { 0x00 }, t4, csa as usize * 16, texa);
    }
    let l = luts();
    let mut rgba = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let c = match psm {
                0x00 => unpack_col(0, rd_px(m, l, 0, tbp, tbw, x, y), texa),
                0x13 | 0x1b => clut[rd_px(m, l, psm, tbp, tbw, x, y) as usize & 0xff],
                0x14 | 0x24 | 0x2c => clut[csa as usize * 16 + (rd_px(m, l, psm, tbp, tbw, x, y) as usize & 0xf)],
                _ => [255, 255, 255, 0x80],
            };
            let a = if tcc == 0 { 255 } else { (c[3] as u32 * 2).min(255) as u8 };
            rgba.push([c[0], c[1], c[2], a]);
        }
    }
    // Debug: RAC2_TEXSOLID=TEX0hex:RRGGBB,.. paints those textures one colour (which faces use which texture).
    if let Ok(v) = std::env::var("RAC2_TEXSOLID") {
        for e in v.split(',') {
            if let Some((k, c)) = e.split_once(':') {
                if u64::from_str_radix(k, 16).ok() == Some(t) {
                    let c = u32::from_str_radix(c, 16).unwrap_or(0xff0000);
                    rgba.iter_mut().for_each(|p| *p = [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
                }
            }
        }
    }
    Texture { w: w as usize, h: h as usize, rgba }
}
