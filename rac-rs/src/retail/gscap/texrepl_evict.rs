//! [PC] Eviction of the texture-pack replacements at a field boundary (lane v041_fx1, review R1 F1 / F2): when the GPU atlas or the CPU
//! tables had no room for an image during the field (`ReplState::need_imgs` / `need_tabs`), the least recently used replacements are
//! dropped, so a long session with a big pack keeps showing the pack's pictures for what is on screen now instead of filling up for good.
//!
//! Rules:
//!  * Nothing happens unless room was refused (two map checks per field otherwise).
//!  * "Recently used" is recorded per image, not per texture entry: the GPU atlas keeps the field number of each id's last draw
//!    (`Book::touch`), the CPU capture one per table (`ReplState::tab_use`). A texture rebuilt under another name in the same field
//!    cannot lose the use of the name it had.
//!  * What was drawn in the last field that drew a replacement (`ReplState::drew`) is never evicted: it is on screen, and evicting it
//!    only moves the refusal to another image. Fields that draw no replacement (a lag field, a load screen) do not move that mark, so
//!    a retry that lands on one protects the same pictures.
//!  * One need per image: two textures that name the same image count once. The GPU atlas is judged by a dry run of the shelf packer
//!    (`Book::plan_keys`; free texels say nothing about a free run of rows tall enough) and the refused images that fit are placed
//!    right away in the dry run's order (tallest first), so what the plan promised is what the atlas does, whatever order the game
//!    would have drawn them in.
//!  * An image no eviction can make room for is given up for `RETRY_FIELDS` fields (`ReplState::retry`): it records no new need in that
//!    time, so it cannot hold up the other images, and the need of a screen that is gone is not carried along. Its entry asks again
//!    when the time is over (`reopen_backed_off`).
//!  * An evicted name or table is forgotten by every entry that held it (it asks the pack again: the slot is `Released`, `Slot::rearm`
//!    decodes the file anew); entries refused for room get another try. The eviction runs between fields, so no prim is in flight.

use super::*;
use std::cmp::Reverse;
use std::collections::HashSet;

/// Fields an image that no eviction could make room for waits before it is looked at again.
const RETRY_FIELDS: u64 = 64;

/// The GPU atlas as the field end uses it (a trait so that tests drive the real `field_end` with a bare `Book`).
pub(super) trait Atlas {
    /// Frees shelves for the refused images (see `Book::plan_keys`): the names evicted and the names given up.
    fn evict(&mut self, needs: &[(TexName, (u32, u32))], keep_from: u64) -> (Vec<TexName>, Vec<TexName>);
    /// Places a refused image that the plan made room for.
    fn place(&mut self, name: &TexName, need: &Need);
}

/// The capture's atlas.
struct GpuAtlas<'a>(&'a mut super::super::draw::gpu::Gpu);

impl Atlas for GpuAtlas<'_> {
    fn evict(&mut self, needs: &[(TexName, (u32, u32))], keep_from: u64) -> (Vec<TexName>, Vec<TexName>) {
        self.0.repl_evict(needs, keep_from)
    }

    fn place(&mut self, name: &TexName, need: &Need) {
        // A slot that lost its image meanwhile answers `NoImage`: its entry decodes it again when it is drawn.
        let _ = self.0.repl_id(name, &need.slot, need.rw, need.rh);
    }
}

impl Gs {
    /// A field ended (`field_end`): evicts when room was refused during it.
    pub(in crate::retail::gscap) fn repl_frame_end(&mut self) {
        if !self.repl_on() {
            return;
        }
        match self.gpu.as_mut() {
            Some(g) => self.repl.field_end(Some(&mut GpuAtlas(g))),
            None => self.repl.field_end(None),
        }
    }
}

impl ReplState {
    /// The entries: the cache and the one in use.
    fn entries_mut(&mut self) -> impl Iterator<Item = &mut Entry> {
        self.cache.values_mut().chain(self.cur.as_mut().map(|(_, e)| e))
    }

    /// A replacement is drawn now: the number of the current field (the value `Book::touch` / `use_tab` store).
    pub(super) fn drawn(&mut self) -> u64 {
        self.drew = self.frame + 1;
        self.drew
    }

    /// The CPU table `serial` was drawn now.
    pub(super) fn use_tab(&mut self, serial: u64) {
        let field = self.drawn();
        if let Some(u) = self.tab_use.get_mut(&serial) {
            *u = field;
        }
    }

    /// Whether the last eviction gave up on `key` and the time to look at it again has not come.
    fn backing_off(&self, key: &RKey) -> bool {
        self.retry.get(key).is_some_and(|&t| self.frame < t)
    }

    /// The GPU atlas refused `name`'s image for room: one need per name, at most 256 names, none while the name is given up.
    pub(super) fn refuse_img(&mut self, name: TexName, need: Need) {
        if !self.backing_off(&(name, 0, 0)) && (self.need_imgs.len() < 256 || self.need_imgs.contains_key(&name)) {
            self.need_imgs.insert(name, need);
        }
    }

    /// The CPU tables refused the table `key` for room (`bytes` big): one need per table, the same rules.
    pub(super) fn refuse_tab(&mut self, key: RKey, bytes: u64) {
        if !self.backing_off(&key) && (self.need_tabs.len() < 256 || self.need_tabs.contains_key(&key)) {
            self.need_tabs.insert(key, bytes);
        }
    }

    /// A field ended: counts it, and frees room for what was refused during it (the module doc has the rules). `atlas` is the GPU capture's
    /// atlas, None for the CPU capture.
    pub(super) fn field_end(&mut self, atlas: Option<&mut dyn Atlas>) {
        self.frame += 1;
        if atlas.is_some() && (!self.tabs.is_empty() || !self.need_tabs.is_empty()) {
            // The GPU capture took over: the CPU capture's tables, and what they refused, are never read again.
            self.drop_cpu_tables();
        }
        if self.reopen != 0 && self.frame >= self.reopen {
            self.reopen_backed_off();
        }
        if self.need_imgs.is_empty() && self.need_tabs.is_empty() {
            return;
        }
        let keep_from = self.drew.max(1);
        let gave_up = match atlas {
            Some(a) => self.evict_atlas(a, keep_from),
            None => self.evict_cpu(keep_from),
        };
        let until = self.frame + RETRY_FIELDS;
        for key in gave_up {
            self.retry.insert(key, until);
        }
        if !self.retry.is_empty() && self.reopen == 0 {
            self.reopen = until;
        }
    }

    /// The GPU atlas: frees the shelves the refused images need and places the ones that fit. The keys given up.
    fn evict_atlas(&mut self, atlas: &mut dyn Atlas, keep_from: u64) -> Vec<RKey> {
        let needs: Vec<(TexName, (u32, u32))> = self.need_imgs.iter().map(|(n, d)| (*n, d.size)).collect();
        let (gone, gave_up) = atlas.evict(&needs, keep_from);
        self.count_evicted(gone.len());
        let mut place: Vec<(TexName, Need)> = self.need_imgs.drain().filter(|(n, _)| !gave_up.contains(n)).collect();
        place.sort_by_key(|(n, d)| (Reverse((d.size.1, d.size.0)), *n));
        for (name, need) in &place {
            atlas.place(name, need);
        }
        let gone: HashSet<TexName> = gone.into_iter().collect();
        self.forget(|e| e.word != 0 && e.named.as_ref().is_some_and(|(n, _)| gone.contains(n)));
        gave_up.into_iter().map(|n| (n, 0, 0)).collect()
    }

    /// The CPU tables: drops the least recently used unprotected tables so that the smallest refused tables fit the budget. The keys given up.
    fn evict_cpu(&mut self, keep_from: u64) -> Vec<RKey> {
        let mut needs: Vec<(RKey, u64)> = self.need_tabs.drain().collect();
        needs.sort_by_key(|&(k, b)| (b, k));
        let protected = self.protected_tab_bytes(keep_from);
        let (mut want, mut gave_up) = (0u64, Vec::new());
        for (key, bytes) in needs {
            if protected + want + bytes <= cpu::tab_budget() as u64 {
                want += bytes;
            } else {
                gave_up.push(key);
            }
        }
        let gone = if want > 0 { self.evict_tables(want, keep_from).unwrap_or_default() } else { HashSet::new() };
        self.count_evicted(gone.len());
        if !gone.is_empty() {
            self.cpu_cur = None;
        }
        self.forget(|e| e.cpu.as_ref().is_some_and(|c| gone.contains(&c.tab.serial)));
        gave_up
    }

    /// Drops every CPU table and what they refused (the entries that held one, or were refused for room, ask the atlas instead).
    fn drop_cpu_tables(&mut self) {
        self.cpu_cur = None;
        self.tabs.clear();
        self.tab_use.clear();
        self.tab_bytes = 0;
        self.need_tabs.clear();
        self.retry.retain(|k, _| k.1 == 0 && k.2 == 0);
        for e in self.entries_mut() {
            e.cpu = None;
        }
        self.forget(|_| false);
    }

    /// The time of the given-up images is over: the ones that are due ask again, and every entry refused for room is looked at once more
    /// (what is still given up is refused without a need).
    fn reopen_backed_off(&mut self) {
        let frame = self.frame;
        self.retry.retain(|_, t| *t > frame);
        self.reopen = self.retry.values().copied().min().unwrap_or(0);
        self.forget(|_| false);
    }

    /// An eviction freed `n` images / tables (maybe none: there was room already).
    fn count_evicted(&self, n: usize) {
        hudrepl::count_n(Count::Evicted, n as u64);
        if n > 0 {
            hudrepl::count(Count::Evictions);
        }
    }

    /// Bytes of the tables drawn in field `keep_from` or later.
    fn protected_tab_bytes(&self, keep_from: u64) -> u64 {
        self.tabs.values().filter(|t| self.tab_use.get(&t.serial).copied().unwrap_or(0) >= keep_from).map(|t| t.w as u64 * t.h as u64 * 4).sum()
    }

    /// The CPU tables, least recently used first, dropped until `need` more bytes fit the budget; tables drawn in field `keep_from` or later
    /// stay. None when the budget would still be exceeded (nothing is dropped then). The serials of the dropped tables.
    fn evict_tables(&mut self, need: u64, keep_from: u64) -> Option<HashSet<u64>> {
        let mut order: Vec<(RKey, u64, usize)> =
            self.tabs.iter().map(|(k, t)| (*k, self.tab_use.get(&t.serial).copied().unwrap_or(0), t.w as usize * t.h as usize * 4)).collect();
        order.sort_unstable_by_key(|&(k, used, _)| (used, k));
        let (mut bytes, mut take) = (self.tab_bytes as u64, Vec::new());
        for (key, used, b) in order {
            if bytes + need <= cpu::tab_budget() as u64 {
                break;
            }
            if used < keep_from {
                bytes -= b as u64;
                take.push((key, b));
            }
        }
        if bytes + need > cpu::tab_budget() as u64 {
            return None;
        }
        let mut gone = HashSet::new();
        for (key, b) in take {
            if let Some(t) = self.tabs.remove(&key) {
                gone.insert(t.serial);
                self.tab_use.remove(&t.serial);
                self.tab_bytes -= b;
            }
        }
        Some(gone)
    }

    /// Entries for which `evicted` holds lose their replacement (and are asked again); entries refused for room get another try. Other
    /// `done` entries (no file, a file refused, a render target) stay done.
    fn forget(&mut self, evicted: impl Fn(&Entry) -> bool) {
        for e in self.entries_mut() {
            let lost = evicted(e);
            if lost {
                e.word = 0;
                e.cpu = None;
            }
            if lost || e.room {
                e.done = false;
                e.room = false;
            }
        }
    }
}

#[cfg(test)]
#[path = "texrepl_evict_tests.rs"]
mod tests;
