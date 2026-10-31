//! Profile only (RAC_GSCAP_PROF=2): the draw's time under a coarse class key, so the prof dump can show ns per pixel per class.
//! Off with plain RAC_GSCAP_PROF=1, so the per-draw key formatting does not distort before / after timings.

use super::super::{Gs, Vtx};
use std::sync::atomic::{AtomicU8, Ordering};

/// 0 = not read yet, 1 = off, 2 = on (RAC_GSCAP_PROF=2).
static COARSE: AtomicU8 = AtomicU8::new(0);

fn coarse_on() -> bool {
    match COARSE.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var("RAC_GSCAP_PROF").is_ok_and(|v| v == "2");
            COARSE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        v => v == 2,
    }
}

impl Gs {
    /// The draw's time under a coarse key (type, target, format, texture / blend, size class), with the pixels of its bounding box
    /// under `P <key>`.
    pub(super) fn prof_coarse(&mut self, ty: u32, vs: &[Vtx], ns: u64) {
        if !coarse_on() {
            return;
        }
        let at = self.attrs();
        let c = ((at >> 9) & 1) as usize;
        let fbp = (self.r[0x4c + c] & 0x1ff) as u32;
        let fpsm = (self.r[0x4c + c] >> 24) & 0x3f;
        let tgt = if self.to_disp { "dsp" } else if Some(fbp) == self.main_fbp { "ov" } else { "vram" };
        let ext = |f: fn(&Vtx) -> u16| u32::from(vs.iter().map(f).max().unwrap_or(0).saturating_sub(vs.iter().map(f).min().unwrap_or(0))) / 16;
        let area = if ty == 6 || (3..=5).contains(&ty) { ext(|v| v.x) as u64 * ext(|v| v.y) as u64 } else { 1 };
        let size = match area {
            0..=255 => "tiny",
            256..=4095 => "small",
            4096..=65535 => "mid",
            _ => "big",
        };
        let key = format!("C ty{ty} {tgt} psm{fpsm:x} t{} a{} {size}", (at >> 4) & 1, (at >> 6) & 1);
        self.prof_add_ns(&format!("P {key}"), area);
        self.prof_add_ns(&key, ns);
    }

    /// The `gscap cat` line of the prof dump: the class keys of `prof_coarse` with their ms, draws and pixels per field.
    pub(in crate::retail::gscap) fn prof_cat_line(v: &[(String, (u64, u64))]) -> String {
        let mut c = String::from("gscap cat (ms/field, draws/field, Mpx/field, ns/px):");
        if !coarse_on() {
            return c + " off (RAC_GSCAP_PROF=2 turns it on)";
        }
        for (k, (n, ns)) in v.iter().filter(|(k, _)| k.starts_with("C ")) {
            let px = v.iter().find(|(q, _)| q.strip_prefix("P ") == Some(k.as_str())).map_or(0, |(_, (_, a))| *a);
            c += &format!(" [{}: {:.2} ms x{:.1} {:.3} Mpx {:.1}]", &k[2..], *ns as f64 / 1e8, *n as f64 / 100.0, px as f64 / 1e8, *ns as f64 / px.max(1) as f64);
        }
        c
    }
}
