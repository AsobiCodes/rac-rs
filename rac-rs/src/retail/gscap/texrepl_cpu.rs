//! [PC] The CPU capture's side of the texture-pack replacements (lane v041_hudcpu): the same naming, entries and rules as the GPU
//! capture (`texrepl.rs`), but a ready image becomes a `CpuRepl` table (`draw/cpu_repl.rs`) instead of an atlas id.
//!
//! In the CPU capture `Gs::vram` is exact at all times, so no shadow copy is made (`repl_shadow` is only called with a GPU); the hooks that
//! keep the versions current are `repl_upload` (`trx_start`), `repl_copy` (`local_copy`) and `draw/cpu_repl.rs repl_cpu_prim` (prims that
//! draw into local memory mark their blocks dirty, like the GPU capture's `repl_drawn`).

use super::super::draw::cpu_repl::{box_table, CpuRepl, CpuSel};
use super::*;

/// Bytes the boiled-down tables may hold together (4 bytes a texel): 96 MB, `RAC_TEXPACK_TAB_MB` changes it (read once; test aid for the
/// eviction of the CPU capture's tables).
pub(super) fn tab_budget() -> usize {
    static MB: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *MB.get_or_init(|| std::env::var("RAC_TEXPACK_TAB_MB").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(96)) << 20
}
/// Largest named rectangle that gets a table, in texels.
const MAX_TEXELS: u32 = 1 << 20;

impl Gs {
    /// The table the CPU capture's prim samples, or None (original texture): the prim's entry as in `repl_word`, resolved to a table.
    pub(in crate::retail::gscap) fn repl_cpu_for(&mut self, ov: bool, at: u64, tex0: u64, clamp: u64) -> Option<CpuSel> {
        let key = self.repl_key(ov, at, tex0, clamp)?;
        let mut e = self.repl_entry(key);
        hudrepl::count(Count::Prims);
        let t = self.repl_resolve_cpu(&mut e);
        self.repl.cur = Some((key, e));
        t
    }

    /// Marks the blocks `lo..hi` as drawn into by a prim (a render target, never named) unless the last call did exactly that and nothing
    /// changed since (a page's thousands of prims into one target are one mark).
    pub(in crate::retail::gscap) fn repl_drawn_once(&mut self, lo: u32, hi: u32) {
        let (l, h, s) = self.repl.last_drawn;
        if (l, h) == (lo, hi) && s == self.repl.stamp {
            return;
        }
        self.repl.touch((lo, hi), true);
        self.repl.last_drawn = (lo, hi, self.repl.stamp);
    }

    /// Like `repl_resolve`: the table of `e` once the pack's image is there (made once per name and rectangle size, then the pack's CPU
    /// copy of the image is released).
    fn repl_resolve_cpu(&mut self, e: &mut Entry) -> Option<CpuSel> {
        if let Some(c) = &e.cpu {
            hudrepl::count(Count::Replaced);
            self.repl.use_tab(c.tab.serial);
            return Some(c.clone());
        }
        if e.done {
            return None;
        }
        let (Some((name, [x, y, w, h])), Some(slot)) = (e.named, e.slot.clone()) else { return None };
        let tab = if let Some(t) = self.repl.tabs.get(&(name, w, h)) {
            Ok(t.clone())
        } else {
            match slot.state() {
                State::Pending => {
                    hudrepl::count(Count::Pending);
                    return None;
                }
                State::Ready => {
                    let t = slot.image().map_or(Err(false), |img| self.repl_make_table(name, &img, w, h));
                    if let Ok(t) = &t {
                        self.repl.tabs.insert((name, w, h), t.clone());
                        slot.release();
                    }
                    t
                }
                State::Released | State::Dropped | State::OverBudget => {
                    // Another size of the name (or an evicted table) needs the image again: decode it anew (also after a decode refused
                    // for the memory budget, once the budget has room).
                    if slot.rearm() {
                        hudrepl::count(Count::Redecoded);
                    }
                    hudrepl::count(Count::Pending);
                    return None;
                }
                _ => {
                    hudrepl::count(Count::Missing);
                    e.done = true;
                    return None;
                }
            }
        };
        let tab = match tab {
            Ok(t) => t,
            Err(room) => {
                hudrepl::count(Count::Refused);
                e.done = true;
                e.room = room;
                return None;
            }
        };
        self.repl.use_tab(tab.serial);
        let sel = CpuSel { tab, x, y };
        e.cpu = Some(sel.clone());
        hudrepl::count(Count::Replaced);
        Some(sel)
    }

    /// The image boiled down to a `w` x `h` table, or Err (a wrong image or a rectangle too big: Err(false); the budget is spent:
    /// Err(true), and the bytes are remembered per table in `need_tabs`, so the next field boundary evicts the least recently used
    /// tables to make room).
    fn repl_make_table(&mut self, name: TexName, img: &hudrepl::HudImg, w: u32, h: u32) -> Result<Arc<CpuRepl>, bool> {
        let bytes = w as usize * h as usize * 4;
        if w == 0 || h == 0 || w * h > MAX_TEXELS || img.w == 0 || img.h == 0 || img.rgba.len() as u64 != img.w as u64 * img.h as u64 * 4 {
            return Err(false);
        }
        if self.repl.tab_bytes + bytes > tab_budget() {
            self.repl.refuse_tab((name, w, h), bytes as u64);
            return Err(true);
        }
        self.repl.tab_bytes += bytes;
        self.repl.serial += 1;
        self.repl.tab_use.insert(self.repl.serial, 0);
        Ok(Arc::new(CpuRepl { w, h, serial: self.repl.serial, px: box_table(&img.rgba, img.w, img.h, w, h) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole CPU path on a `Gs`: tables fill the budget, the next one is refused (`repl_make_table` -> `refuse_tab`), the field boundary
    /// (`field_end`, as `repl_frame_end` calls it for the CPU capture) drops an unused table, and the refused one is made at the next try.
    /// `tab_use` holds only the tables that are alive.
    #[test]
    fn a_refused_table_is_made_after_the_field_end_frees_an_unused_one() {
        let mut g = Gs::new(vec![0; 4 << 20]);
        let nm = |n: u64| TexName::palette(n, n + 1, 0x1dd3);
        let img = hudrepl::HudImg { w: 4, h: 4, rgba: vec![9; 64] };
        // 1024 x 1024 texels = 4 MB a table: the budget (96 MB unless RAC_TEXPACK_TAB_MB says otherwise) holds budget / 4 MB of them.
        let fit = tab_budget() / (4 << 20);
        if fit < 2 {
            return; // RAC_TEXPACK_TAB_MB < 8 in the test environment: the test needs room for two tables
        }
        let mut serials = Vec::new();
        for k in 0..fit as u64 {
            let t = g.repl_make_table(nm(k), &img, 1024, 1024).unwrap_or_else(|_| panic!("table {k} refused"));
            serials.push(t.serial);
            g.repl.tabs.insert((nm(k), 1024, 1024), t);
        }
        g.repl.tab_bytes = fit * (4 << 20);
        assert_eq!(g.repl.tab_use.len(), fit);
        let last = (nm(fit as u64), 1024, 1024);
        assert!(matches!(g.repl_make_table(last.0, &img, 1024, 1024), Err(true)), "the budget is spent");
        assert_eq!(g.repl.need_tabs.len(), 1);
        // The two newest tables are drawn in this field and stay; the least recently used of the others goes (the lowest key on a tie).
        g.repl.use_tab(serials[fit - 1]);
        g.repl.use_tab(serials[fit - 2]);
        g.repl.field_end(None);
        assert_eq!(g.repl.tabs.len(), fit - 1);
        assert!(!g.repl.tabs.contains_key(&(nm(0), 1024, 1024)) && g.repl.tabs.contains_key(&(nm(fit as u64 - 1), 1024, 1024)));
        assert_eq!(g.repl.tab_use.len(), fit - 1, "the evicted table's use is forgotten");
        assert!(!g.repl.tab_use.contains_key(&serials[0]) && g.repl.tab_use[&serials[fit - 1]] > 0);
        assert!(g.repl.need_tabs.is_empty());
        assert_eq!(g.repl.tab_bytes, (fit - 1) * (4 << 20));
        assert!(g.repl_make_table(last.0, &img, 1024, 1024).is_ok(), "the refused table fits now");
    }
}
