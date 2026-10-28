//! [PC] Run-time names for the GPU capture's textured prims and the replacement word of their records (lane v041_hud, B2a-3).
//!
//! With a textures mod enabled (`hudrepl::enabled()`; otherwise every hook below returns at once and nothing is allocated) the capture
//! names the texture each overlay prim samples the way PCSX2 would (`gsname::name_for_draw`), asks the pack for its image
//! (`hudrepl::lookup`, decoded off the render thread), puts the image into the GPU atlas (`Gpu::repl_id`) and writes the atlas id and
//! the origin of the named rectangle into word 1 of the prim's record (`gpu_repl::pack_word`); gpu.wgsl then samples the image instead
//! of the GS texture. Word 1 stays 0 (the original texture) for everything the rules below refuse.
//!
//! Local memory lives on the GPU in this mode, so names are read from a CPU copy: `Gs::vram`, exact at the moment the GPU capture takes
//! over (`gpu_attach`) and kept up to date here by what reaches the GPU (host -> local transfers that are written, local copies).
//! Draws into other buffers change local memory too; their blocks are marked `dirty` (never named) until an upload overwrites them.
//! Each block also has a version (`ver`, the `stamp` of its last change), so a cached name stays valid only while no block of the
//! texture (and of its CLUT) changed after the name was made.
//!
//! Rules (word 1 = 0): textures of formats PCSX2 does not hash, CLAMP mode 3 (region repeat), a texture inside the main frame buffer
//! (render target) or on a dirty block, full-screen grabs (CT32 of 512 x 256 texels or more), prims that are not overlay prims.

use super::draw::cpu_repl::CpuSel;
use super::draw::gpu_repl::{pack_word, Miss, MAX_IDS};
use super::Gs;
use crate::texpack::gsname::{block_size, has_palette, name_for_draw};
use crate::texpack::hudrepl::{self, Count, State};
use crate::texpack::name::TexName;
use std::collections::HashMap;
use std::sync::Arc;

/// Blocks of local memory (256 bytes each).
const BLOCKS: usize = 16384;

/// The registers a name depends on: TEX0 without TCC / TFX / CLD, CLAMP, TEXA, TEXCLUT.
type Key = (u64, u64, u64, u64);

/// What is known about one texture (one `Key`).
#[derive(Default)]
struct Entry {
    /// `ReplState::stamp` when the name was made, and when it was last found still valid.
    built: u64,
    checked: u64,
    /// Block ranges of the texture and of its CLUT (empty: none).
    span: (u32, u32),
    cspan: (u32, u32),
    /// The name, the named rectangle (x, y, w, h in GS texels) and the pack's slot.
    named: Option<(TexName, [u32; 4])>,
    slot: Option<Arc<hudrepl::Slot>>,
    /// Record word 1 (0 = original), once the image is in the atlas.
    word: u32,
    /// The CPU capture's table of the named rectangle (texrepl_cpu.rs), once the image is boiled down.
    cpu: Option<CpuSel>,
    /// Nothing to show for this texture until its blocks change (or an eviction made room): no file, refused, atlas full, a render target.
    done: bool,
    /// `done` because the atlas / the tables had no room: the entry asks again after an eviction (other `done` entries do not).
    room: bool,
}

/// An image the GPU atlas refused for room, kept until the field end (one per name): what the eviction must make room for, and what
/// places it afterwards (`texrepl_evict.rs`).
struct Need {
    /// The image's size in texels.
    size: (u32, u32),
    slot: Arc<hudrepl::Slot>,
    /// The named rectangle in GS texels (`Gpu::repl_id`).
    rw: u32,
    rh: u32,
}

/// A refused image / table across the field ends: GPU = (name, 0, 0), CPU = (name, rectangle w, h).
type RKey = (TexName, u32, u32);

/// The per-capture state of the replacements (empty until the first hook with a pack enabled).
#[derive(Default)]
pub(super) struct ReplState {
    ver: Vec<u64>,
    dirty: Vec<u64>,
    /// 64 bits: a version never wraps in practice, so `max_ver(span) > built` keeps telling a changed texture from an unchanged one.
    stamp: u64,
    /// Fields ended so far (`repl_frame_end`), and the field number (`frame + 1`) of the last field in which a replacement was drawn
    /// (0 = none yet): what that field drew is never evicted, however many fields without a replacement come after it.
    frame: u64,
    drew: u64,
    /// The room refused since the last eviction, one per image: the GPU capture's images (by name) and the CPU capture's tables (name and
    /// rectangle size, with their bytes). Never grows past 256 images.
    need_imgs: HashMap<TexName, Need>,
    need_tabs: HashMap<RKey, u64>,
    /// Images / tables the last eviction could not make room for: no new need is recorded for them before the field in the value, so one
    /// image that cannot fit does not hold up the rest. `reopen`: the field at which the refused entries ask again (0 = nothing waits).
    retry: HashMap<RKey, u64>,
    reopen: u64,
    /// Tables made so far (`CpuRepl::serial`), and per serial the field number in which the table was last drawn.
    serial: u64,
    tab_use: HashMap<u64, u64>,
    cache: HashMap<Key, Entry>,
    /// The entry of the last prim (consecutive prims repeat their texture): out of `cache` while it is in use.
    cur: Option<(Key, Entry)>,
    /// The CPU capture (texrepl_cpu.rs): the table the current prim samples (`View` points into it), the tables made so far by name and
    /// rectangle size, and the bytes they hold; the last range `repl_drawn_once` marked.
    pub(super) cpu_cur: Option<CpuSel>,
    tabs: HashMap<(TexName, u32, u32), Arc<super::draw::cpu_repl::CpuRepl>>,
    tab_bytes: usize,
    last_drawn: (u32, u32, u64),
}

impl ReplState {
    fn ensure(&mut self) {
        if self.ver.is_empty() {
            self.ver = vec![0; BLOCKS];
            self.dirty = vec![0; BLOCKS / 64];
        }
    }

    fn range(r: (u32, u32)) -> std::ops::Range<usize> {
        (r.0 as usize).min(BLOCKS)..(r.1 as usize).min(BLOCKS)
    }

    /// Blocks `r` changed: new version, and dirty (a draw) or clean (an upload or copy).
    fn touch(&mut self, r: (u32, u32), dirty: bool) {
        self.ensure();
        self.stamp += 1;
        let s = self.stamp;
        for b in Self::range(r) {
            self.ver[b] = s;
            if dirty {
                self.dirty[b >> 6] |= 1 << (b & 63);
            } else {
                self.dirty[b >> 6] &= !(1 << (b & 63));
            }
        }
    }

    fn max_ver(&self, r: (u32, u32)) -> u64 {
        self.ver.get(Self::range(r)).and_then(|v| v.iter().copied().max()).unwrap_or(0)
    }

    fn any_dirty(&self, r: (u32, u32)) -> bool {
        Self::range(r).any(|b| self.dirty[b >> 6] & (1 << (b & 63)) != 0)
    }

    /// Entries held (tests, exit line).
    #[cfg(test)]
    fn len(&self) -> usize {
        self.cache.len() + usize::from(self.cur.is_some())
    }
}

/// The 256-byte blocks a rectangle of a buffer covers, one call each (not the range between them): `Gs::span_calc`'s walk.
fn for_blocks(psm: u32, bp: u32, bw: u32, x: u32, y: u32, w: u32, h: u32, mut f: impl FnMut(u32)) {
    type Addr = fn(u32, u32, u32, u32) -> usize;
    let (sw, sh, addr): (u32, u32, Addr) = match psm {
        0x00 | 0x01 | 0x1b | 0x24 | 0x2c => (8, 8, Gs::a32),
        0x02 | 0x0a => (16, 8, Gs::a16),
        0x13 => (16, 16, Gs::a8),
        0x14 => (32, 16, |b, w, x, y| Gs::a4(b, w, x, y) / 2),
        _ => return,
    };
    if w == 0 || h == 0 {
        return;
    }
    for xx in (x..x + w).step_by(sw as usize).chain(std::iter::once(x + w - 1)) {
        for yy in (y..y + h).step_by(sh as usize).chain(std::iter::once(y + h - 1)) {
            f((addr(bp, bw, xx, yy) / 256) as u32);
        }
    }
}

/// `(bp, bw, psm, x, y, w, h)` of the destination of BITBLTBUF / TRXPOS / TRXREG.
fn dest(bb: u64, pos: u64, rr: u64) -> (u32, u32, u32, u32, u32, u32, u32) {
    (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32, ((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32, (rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32)
}

impl Gs {
    /// Whether the hooks work: a textures mod is enabled (the GPU capture or the CPU capture draws).
    fn repl_on(&self) -> bool {
        hudrepl::enabled()
    }

    /// A host -> local transfer starts (`trx_start`): its blocks change from now on (new version, no longer dirty). Marking the blocks of
    /// the rectangle one by one is exact; a block range could mark a render target's block clean.
    pub(super) fn repl_upload(&mut self, bb: u64, pos: u64, rr: u64) {
        if self.repl_on() {
            let (bp, bw, psm, x, y, w, h) = dest(bb, pos, rr);
            self.repl_mark(psm, bp, bw, x, y, w, h, false);
        }
    }

    /// The blocks of a rectangle changed: clean (their content is in `vram`) or dirty (it is not).
    fn repl_mark(&mut self, psm: u32, bp: u32, bw: u32, x: u32, y: u32, w: u32, h: u32, dirty: bool) {
        self.repl.ensure();
        self.repl.stamp += 1;
        let (s, st) = (self.repl.stamp, &mut self.repl);
        for_blocks(psm, bp, bw, x, y, w, h, |b| {
            let b = b as usize % BLOCKS;
            st.ver[b] = s;
            if dirty {
                st.dirty[b >> 6] |= 1 << (b & 63);
            } else {
                st.dirty[b >> 6] &= !(1 << (b & 63));
            }
        });
    }

    /// Qwords `q0..` of a host -> local transfer reach the GPU: write them into `vram` as well. Transfers into the main buffer (the
    /// picture, never a texture) and the 24-bit straddle (not modelled, as in `put`) are not copied.
    pub(super) fn repl_shadow(&mut self, bb: u64, pos: u64, rr: u64, q0: u32, data: &[u128]) {
        if !self.repl_on() {
            return;
        }
        let (bp, bw, psm, x, y, w, h) = dest(bb, pos, rr);
        let bpp = Self::bpp(psm);
        if w == 0 || h == 0 || bpp == 0 {
            return;
        }
        let (lo, hi) = Self::span(psm, bp, bw, x, y, w, h);
        if let Some(f) = self.main_fbp {
            if lo < self.main_end && f * 32 < hi {
                return;
            }
        }
        let (ppq, total) = (128 / bpp, w * h);
        let mask = if bpp >= 32 { u128::from(u32::MAX) } else { (1u128 << bpp) - 1 };
        let mut i = q0 * ppq;
        for &q in data {
            for k in 0..ppq {
                if i >= total {
                    return;
                }
                self.wr_px(psm, bp, bw, x + i % w, y + i / w, ((q >> (k * bpp)) & mask) as u32);
                i += 1;
            }
        }
    }

    /// A local -> local copy (`TRXDIR` 2) ran on the GPU. A copy between ordinary buffers is repeated on `vram` when it is small and its
    /// source is clean; every other copy leaves its destination dirty (a screen grab, a render target, a big move).
    pub(super) fn repl_copy(&mut self) {
        if !self.repl_on() {
            return;
        }
        let bb = self.r[0x50];
        let (sbp, sbw, spsm) = ((bb & 0x3fff) as u32, ((bb >> 16) & 0x3f) as u32, ((bb >> 24) & 0x3f) as u32);
        let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
        let pos = self.r[0x51];
        let (sx, sy, dx, dy) = ((pos & 0x7ff) as u32, ((pos >> 16) & 0x7ff) as u32, ((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
        let rr = self.r[0x52];
        let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
        if rw == 0 || rh == 0 {
            return;
        }
        self.repl.ensure();
        let mut src_dirty = rw * rh > 256 * 256;
        for_blocks(spsm, sbp, sbw, sx, sy, rw, rh, |b| src_dirty |= self.repl.dirty[(b as usize % BLOCKS) >> 6] & (1 << (b & 63)) != 0);
        let main_src = self.main_fbp.is_some_and(|f| {
            let (lo, hi) = Self::span(spsm, sbp, sbw, sx, sy, rw, rh);
            lo < self.main_end && f * 32 < hi
        });
        if src_dirty || main_src || !matches!(spsm, 0x00 | 0x01 | 0x02 | 0x0a | 0x13 | 0x14) {
            self.repl_mark(dpsm, dbp, dbw, dx, dy, rw, rh, true);
            return;
        }
        // The CPU capture has done the copy itself: `vram` is exact, only the versions change.
        if self.gpu.is_some() {
            self.flush(0, u32::MAX);
            for y in 0..rh {
                for x in 0..rw {
                    let v = self.rd_px(spsm, sbp, sbw, sx + x, sy + y);
                    self.wr_px(dpsm, dbp, dbw, dx + x, dy + y, v);
                }
            }
        }
        self.repl_mark(dpsm, dbp, dbw, dx, dy, rw, rh, false);
    }

    /// A prim drew into other buffers (`gpu_raster`'s written block ranges): local memory there is no longer what `vram` holds.
    pub(super) fn repl_drawn(&mut self, written: &[(u32, u32, u64)]) {
        if !written.is_empty() && self.repl_on() {
            for &(lo, hi, _) in written {
                self.repl_drawn_once(lo, hi);
            }
        }
    }

    /// Record word 1 of a prim: the atlas id of its texture's replacement with the origin of the named rectangle, or 0. Called for every
    /// prim of the GPU capture (`gpu_raster`), so the first line is the whole cost without a pack.
    pub(super) fn repl_word(&mut self, ov: bool, at: u64, tex0: u64, clamp: u64) -> u32 {
        let Some(key) = self.repl_key(ov, at, tex0, clamp) else { return 0 };
        let mut e = self.repl_entry(key);
        hudrepl::count(Count::Prims);
        let w = self.repl_resolve(&mut e);
        self.repl.cur = Some((key, e));
        w
    }

    /// The cache key of a prim's texture, or None for a prim the rules refuse (see the module doc).
    fn repl_key(&self, ov: bool, at: u64, tex0: u64, clamp: u64) -> Option<Key> {
        if !ov || at & 0x10 == 0 || !self.repl_on() {
            return None;
        }
        let psm = ((tex0 >> 20) & 0x3f) as u32;
        let (tbp, tw, th) = ((tex0 & 0x3fff) as u32, 1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
        // Region repeat (CLAMP mode 3) needs bit operations on the coordinates; a texture in the main buffer is a render target.
        let main = self.main_fbp.is_some_and(|f| tbp >= f * 32 && tbp < self.main_end.max(f * 32 + 1));
        if block_size(psm).0 == 0 || clamp & 3 == 3 || (clamp >> 2) & 3 == 3 || main || (psm == 0 && tw * th >= 512 * 256) {
            return None;
        }
        Some((tex0 & !(0x7 << 34 | 0x7 << 61), clamp, self.r[0x3b], self.r[0x1c]))
    }

    /// The entry of `key` (the last one used, one of the cache, or new), valid for the texture as it is now.
    fn repl_entry(&mut self, key: Key) -> Entry {
        self.repl.ensure();
        let mut e = match self.repl.cur.take() {
            Some((k, e)) if k == key => e,
            old => {
                if let Some((k, e)) = old {
                    self.repl.cache.insert(k, e);
                }
                self.repl.cache.remove(&key).unwrap_or(Entry { built: u64::MAX, ..Entry::default() })
            }
        };
        if e.built != u64::MAX && e.checked == self.repl.stamp {
            return e;
        }
        if e.built == u64::MAX || self.repl.max_ver(e.span) > e.built || self.repl.max_ver(e.cspan) > e.built {
            e = self.repl_build(key);
        } else {
            e.checked = self.repl.stamp;
        }
        e
    }

    /// Names the texture of `key` as local memory holds it now and asks the pack for it.
    fn repl_build(&mut self, (tex0, clamp, texa, texclut): Key) -> Entry {
        let (psm, tbp, tbw) = (((tex0 >> 20) & 0x3f) as u32, (tex0 & 0x3fff) as u32, ((tex0 >> 14) & 0x3f) as u32);
        let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
        let span = Self::span(psm, tbp, tbw, 0, 0, tw, th);
        let cbp = ((tex0 >> 37) & 0x3fff) as u32;
        let cspan = if has_palette(psm) { (cbp, cbp + 4) } else { (0, 0) };
        // The uploads this texture waits for are written first, as for any draw that reads local memory.
        self.flush(span.0, span.1);
        if cspan.1 != 0 {
            self.flush(cspan.0, cspan.1);
        }
        let stamp = self.repl.stamp;
        let mut e = Entry { built: stamp, checked: stamp, span, cspan, ..Entry::default() };
        if self.repl.any_dirty(span) || self.repl.any_dirty(cspan) {
            e.done = true;
            return e;
        }
        let Some(n) = name_for_draw(&self.vram, tex0, clamp, texa, texclut, false) else {
            e.done = true;
            return e;
        };
        if n.x >= 1024 || n.y >= 1024 {
            e.done = true;
            return e;
        }
        e.slot = Some(hudrepl::lookup(&n.name));
        e.named = Some((n.name, [n.x, n.y, n.w, n.h]));
        e
    }

    /// Record word 1 for `e`: asks the pack's slot again while it is still loading, and the atlas once for a ready image.
    fn repl_resolve(&mut self, e: &mut Entry) -> u32 {
        if e.word != 0 {
            hudrepl::count(Count::Replaced);
            let field = self.repl.drawn();
            if let Some(g) = self.gpu.as_mut() {
                g.repl_touch(e.word, field);
            }
            return e.word;
        }
        if e.done {
            return 0;
        }
        let (Some((name, [x, y, w, h])), Some(slot)) = (e.named.as_ref(), e.slot.as_ref()) else { return 0 };
        match slot.state() {
            State::Pending => {
                hudrepl::count(Count::Pending);
                0
            }
            State::OverBudget => {
                // The decode budget was full: decode again once it has room (`Slot::rearm`).
                if slot.rearm() {
                    hudrepl::count(Count::Redecoded);
                }
                hudrepl::count(Count::Pending);
                0
            }
            State::Ready | State::Released | State::Dropped => {
                let Some(g) = self.gpu.as_mut() else { return 0 };
                match g.repl_id(name, slot, *w, *h) {
                    Ok(id) => {
                        debug_assert!(id < MAX_IDS);
                        e.word = pack_word(id, *x, *y);
                        g.repl_touch(e.word, self.repl.drawn());
                        hudrepl::count(Count::Replaced);
                        e.word
                    }
                    Err(Miss::NoImage) => {
                        // The slot gave its copy to the atlas and the atlas lost the image (an eviction): decode it again.
                        slot.rearm();
                        hudrepl::count(Count::Redecoded);
                        hudrepl::count(Count::Pending);
                        0
                    }
                    Err(miss) => {
                        // No room: the original stays until the texture changes or the next field boundary has made room.
                        if let Miss::Full(iw, ih) = miss {
                            self.repl.refuse_img(*name, Need { size: (iw, ih), slot: slot.clone(), rw: *w, rh: *h });
                            e.room = true;
                            hudrepl::count(Count::Full);
                        } else {
                            hudrepl::count(Count::Never);
                        }
                        hudrepl::count(Count::Refused);
                        e.done = true;
                        0
                    }
                }
            }
            _ => {
                hudrepl::count(Count::Missing);
                e.done = true;
                0
            }
        }
    }
}

#[path = "texrepl_cpu.rs"]
mod cpu;
#[path = "texrepl_evict.rs"]
mod evict;

#[cfg(test)]
#[path = "texrepl_tests.rs"]
mod tests;
