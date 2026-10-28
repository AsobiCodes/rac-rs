//! [PC] The GPU side of the texture-pack replacements for the capture's HUD / menu / text prims (lane tex3, B2a-2): one RGBA8 atlas of
//! the replacement images and a table that says where each one sits (gpu.wgsl bindings 9 and 10, `repl_img` / `repl_tab`).
//!
//! Idle until `Gpu::repl_id` is called (`texrepl.rs`): with no pack the two bindings are 1 x 1 stand-ins and the
//! prim records keep word 1 at 0, which is exactly today's path. The atlas and the table are made at the first image.
//!
//! Rules:
//!  * An image goes in once per `TexName` (`Book::ids`), so a new capture that asks again gets the same id back.
//!  * Placement is a shelf packer (rows of near-equal height, 1 px gutter) and writes only to regions nobody has used, so kernels that
//!    are queued or running (the queue orders `write_texture` after the earlier submits anyway) never see a region change.
//!  * A full atlas or table refuses the image (`Refuse::Full`): the caller keeps drawing the original texture and, at the next frame
//!    boundary, asks `Book::plan_keys` for room: the least recently used shelves that were not drawn in the last field that drew
//!    replacements are freed whole (their ids go back to the pool) until a dry run of the packer places every refused image (free texels
//!    are not enough: an image needs one free shelf or run of rows tall enough). Use is recorded per atlas id (`Book::touch`, one store
//!    per prim), so an image counts as drawn whatever texture entries held it. The caller forgets the evicted names' ids (their slots
//!    decode again on demand) and a freed region is reused only by uploads made after every recorded step was submitted, so a kernel
//!    never sees a region change.
//!  * An image that can never fit (wider or taller than the atlas) is `Refuse::Never`: no eviction would help.
//!
//! Record word 1 of a prim (gpu.rs `STRIDE` record): `id` (12 bits, 0 = original texture) | `x0` << 12 | `y0` << 22 (10 bits each, the
//! origin of the named rectangle inside the GS texture). Table texel `id` (Rgba32Uint): x = ax | ay << 16, y = aw | ah << 16 (the
//! image's place in the atlas), z = rw | rh << 16 (the named rectangle's size in GS texels), w = 0.

use crate::texpack::hudrepl::{HudImg, Slot};
use crate::texpack::name::TexName;
use std::collections::HashMap;

/// Atlas width in texels (the default `max_texture_dimension_2d`).
pub(super) const ATLAS_W: u32 = 8192;
/// Table size: ids 1 .. MAX_IDS - 1 fit the 12 bits of record word 1.
pub(in super::super) const MAX_IDS: u32 = 4096;
/// Free texels around an image in the atlas.
const GUTTER: u32 = 1;
/// Default atlas size in MB (`RAC_TEXPACK_HUD_MB`, 1 ..= 256).
const DEFAULT_MB: u32 = 256;

/// Record word 1 for the replacement `id` of the rectangle at (`x0`, `y0`).
pub(in super::super) fn pack_word(id: u32, x0: u32, y0: u32) -> u32 {
    debug_assert!(id < MAX_IDS && x0 < 1024 && y0 < 1024);
    (id & 0xfff) | (x0 & 0x3ff) << 12 | (y0 & 0x3ff) << 22
}

/// `(id, x0, y0)` of a record word 1.
pub(super) fn unpack_word(w: u32) -> (u32, u32, u32) {
    (w & 0xfff, (w >> 12) & 0x3ff, w >> 22)
}

/// The table texel of an image placed at (`ax`, `ay`) of size `aw` x `ah`, standing for a rectangle of `rw` x `rh` GS texels.
pub(super) fn table_texel(ax: u32, ay: u32, aw: u32, ah: u32, rw: u32, rh: u32) -> [u32; 4] {
    [ax | ay << 16, aw | ah << 16, rw | rh << 16, 0]
}

/// The atlas height in rows for `mb` MB (`None`: the default), 8192 texels wide, at most `ATLAS_W` rows.
pub(super) fn atlas_rows(mb: Option<u32>) -> u32 {
    (mb.unwrap_or(DEFAULT_MB).clamp(1, DEFAULT_MB) * (1 << 20) / (ATLAS_W * 4)).min(ATLAS_W)
}

/// One shelf: a row of images of near-equal height.
#[derive(Clone)]
struct Shelf {
    y: u32,
    h: u32,
    x: u32,
    /// No live image: the whole width is free again (`Shelves::release`).
    free: bool,
}

/// The shelf packer: places `w` x `h` boxes (plus the gutter) into a `w_all` x `h_all` area, never moving a placed one.
#[derive(Clone)]
pub(super) struct Shelves {
    w: u32,
    h: u32,
    rows: Vec<Shelf>,
    y: u32,
}

impl Shelves {
    pub(super) fn new(w: u32, h: u32) -> Shelves {
        Shelves { w, h, rows: Vec::new(), y: 0 }
    }

    /// Where a `w` x `h` image goes (its top left), or None when it does not fit. A shelf takes the image when it is tall enough and
    /// not more than twice as tall as the image plus gutter (the least wasteful such shelf wins); else a new shelf opens.
    pub(super) fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (bw, bh) = (w.checked_add(GUTTER)?, h.checked_add(GUTTER)?);
        if w == 0 || h == 0 || bw > self.w {
            return None;
        }
        let best = self.rows.iter_mut().filter(|s| !s.free && s.h >= bh && s.h <= bh * 2 && self.w - s.x >= bw).min_by_key(|s| s.h);
        if let Some(s) = best {
            let at = (s.x, s.y);
            s.x += bw;
            return Some(at);
        }
        // A freed shelf (the smallest that is tall enough) is reused, cut down to the image when it is more than twice as tall.
        if let Some(i) = (0..self.rows.len()).filter(|&i| self.rows[i].free && self.rows[i].h >= bh).min_by_key(|&i| self.rows[i].h) {
            let Shelf { y, h, .. } = self.rows[i];
            let used = if h > bh * 2 { bh } else { h };
            self.rows[i] = Shelf { y, h: used, x: bw, free: false };
            if used < h {
                self.rows.insert(i + 1, Shelf { y: y + used, h: h - used, x: 0, free: true });
            }
            return Some((0, y));
        }
        if self.y.checked_add(bh)? > self.h {
            return None;
        }
        self.rows.push(Shelf { y: self.y, h: bh, x: bw, free: false });
        self.y += bh;
        Some((0, self.y - bh))
    }

    /// Whether an image of this size could ever be placed (an empty atlas).
    fn can_ever_fit(&self, w: u32, h: u32) -> bool {
        w != 0 && h != 0 && w.saturating_add(GUTTER) <= self.w && h.saturating_add(GUTTER) <= self.h
    }

    /// Frees the shelf that starts at row `y`: its images are gone. Free neighbours merge; a free shelf at the end gives its rows back.
    fn release(&mut self, y: u32) {
        let Some(mut i) = self.rows.iter().position(|s| s.y == y) else { return };
        self.rows[i].free = true;
        self.rows[i].x = 0;
        if i + 1 < self.rows.len() && self.rows[i + 1].free {
            self.rows[i].h += self.rows[i + 1].h;
            self.rows.remove(i + 1);
        }
        if i > 0 && self.rows[i - 1].free {
            self.rows[i - 1].h += self.rows[i].h;
            self.rows.remove(i);
            i -= 1;
        }
        if i + 1 == self.rows.len() {
            self.y = self.rows[i].y;
            self.rows.pop();
        }
    }

    /// Whether every image of `imgs` (w, h) could be placed one after the other, tallest first (a dry run on a copy). Free TEXELS are not
    /// enough: an image needs one free shelf or run of end rows that is tall enough.
    fn fits_all(&self, imgs: &[(u32, u32)]) -> bool {
        let mut sim = self.clone();
        let mut v = imgs.to_vec();
        v.sort_unstable_by_key(|&(w, h)| std::cmp::Reverse((h, w)));
        v.iter().all(|&(w, h)| sim.alloc(w, h).is_some())
    }
}

/// Why an image was not taken.
#[derive(Debug, PartialEq, Eq)]
pub(in super::super) enum Refuse {
    /// No room in the atlas (or no id left).
    Full,
    /// The pixel count does not match the size.
    Bad,
    /// The image is larger than the whole atlas: evicting would not help.
    Never,
}

/// Why `ReplGpu::get_or_add` gave no id.
#[derive(Debug, PartialEq, Eq)]
pub(in super::super) enum Miss {
    /// The slot has no decoded image (still loading, or released while the atlas lost the copy).
    NoImage,
    /// No room: the size of the image (w, h in texels). Evicting may help.
    Full(u32, u32),
    /// Never: a wrong image or one larger than the atlas.
    Never,
}

/// Where an image went.
#[derive(Debug, PartialEq, Eq)]
pub(in super::super) struct Placed {
    pub id: u32,
    pub x: u32,
    pub y: u32,
}

/// The bookkeeping of the atlas without any GPU object (so it is testable): ids per name and the packer.
pub(in super::super) struct Book {
    shelves: Shelves,
    ids: HashMap<TexName, u32>,
    /// Per live id: the name and the row of its shelf (what `evict` frees together).
    at: HashMap<u32, (TexName, u32)>,
    /// Ids of evicted images, handed out again before `next` grows.
    spare: Vec<u32>,
    next: u32,
    /// Per id: the field number (`touch`) in which the image was last drawn, 0 = never.
    last: Vec<u64>,
}

impl Book {
    pub(in super::super) fn new(rows: u32) -> Book {
        Book { shelves: Shelves::new(ATLAS_W, rows), ids: HashMap::new(), at: HashMap::new(), spare: Vec::new(), next: 1, last: vec![0; MAX_IDS as usize] }
    }

    pub(in super::super) fn id_of(&self, name: &TexName) -> Option<u32> {
        self.ids.get(name).copied()
    }

    /// The place and id for a `w` x `h` image of `name` (an id it already has is not given again).
    pub(in super::super) fn place(&mut self, name: TexName, w: u32, h: u32) -> Result<Placed, Refuse> {
        if !self.shelves.can_ever_fit(w, h) {
            return Err(Refuse::Never);
        }
        let id = match self.spare.last() {
            Some(&id) => id,
            None if self.next < MAX_IDS => self.next,
            None => return Err(Refuse::Full),
        };
        let (x, y) = self.shelves.alloc(w, h).ok_or(Refuse::Full)?;
        if self.spare.pop().is_none() {
            self.next += 1;
        }
        self.ids.insert(name, id);
        self.at.insert(id, (name, y)); // `alloc`'s y is the top row of the image's shelf
        self.last[id as usize] = 0;
        Ok(Placed { id, x, y })
    }

    /// The image `id` was drawn in field number `field` (a store per prim: the eviction ranks and protects by it).
    pub(in super::super) fn touch(&mut self, id: u32, field: u64) {
        if let Some(l) = self.last.get_mut(id as usize) {
            *l = field;
        }
    }

    /// Which shelves to free (their rows, least recently used first) so that the refused images `needs` (key, (w, h)) can be placed and an
    /// id is spare for each, by a dry run on a copy of the packer. Shelves with an image drawn in field `keep_from` or later (`touch`) are
    /// never freed (they are on screen: freeing them only trades one refusal for another). Returns the rows to free and the keys of the
    /// images given up (they do not fit even with every unprotected shelf freed; the tallest are given up first). No rows and no keys:
    /// there is room already.
    ///
    /// Cost bound: N = images in `needs` (the caller keeps one per name, at most 256), S = shelves. The give-up count and the number of
    /// shelves freed are both found by binary search, so there are at most about log2(N) + log2(S) <= 17 probes, each a dry run of at most
    /// N allocations over S shelves plus S releases: O((log N + log S) * (N * S + S^2)), under a millisecond for 256 images and 200
    /// shelves. (The searches assume that freeing more shelves or asking for fewer images never fits worse; where that is not exact a
    /// probe only costs a few more freed shelves, never an image that does not fit, because the answer is always a probed state.)
    pub(in super::super) fn plan_keys<K: Clone>(&self, needs: &[(K, (u32, u32))], keep_from: u64) -> (Vec<u32>, Vec<K>) {
        if needs.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let mut shelf_use: HashMap<u32, (u64, usize)> = HashMap::new();
        for (id, (_, row)) in &self.at {
            let e = shelf_use.entry(*row).or_default();
            e.0 = e.0.max(self.last[*id as usize]);
            e.1 += 1;
        }
        let mut order: Vec<(u64, u32, usize)> = shelf_use.into_iter().filter(|(_, (t, _))| *t < keep_from).map(|(row, (t, n))| (t, row, n)).collect();
        order.sort_unstable();
        let mut want: Vec<(K, (u32, u32))> = needs.to_vec();
        want.sort_by_key(|&(_, (w, h))| std::cmp::Reverse((h, w)));
        let sizes: Vec<(u32, u32)> = want.iter().map(|&(_, s)| s).collect();
        let spare_ids = self.spare.len() + (MAX_IDS - self.next.min(MAX_IDS)) as usize;
        // Whether `sizes[d..]` all fit with the first `k` shelves of `order` freed.
        let feasible = |d: usize, k: usize| -> bool {
            let imgs = &sizes[d..];
            if imgs.is_empty() {
                return true;
            }
            if spare_ids + order[..k].iter().map(|o| o.2).sum::<usize>() < imgs.len() {
                return false;
            }
            let mut sim = self.shelves.clone();
            for o in &order[..k] {
                sim.release(o.1);
            }
            sim.fits_all(imgs)
        };
        let all = order.len();
        // The fewest tallest images to give up when every unprotected shelf is freed, then the fewest shelves for the rest.
        let first = |n: usize, ok: &dyn Fn(usize) -> bool| -> usize {
            let (mut lo, mut hi) = (0, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                if ok(mid) {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            lo
        };
        let d = first(sizes.len(), &|d| feasible(d, all));
        let k = first(all, &|k| feasible(d, k));
        (order[..k].iter().map(|o| o.1).collect(), want[..d].iter().map(|(key, _)| key.clone()).collect())
    }

    /// `plan_keys` for sizes alone (tests): None when not even one image fits.
    #[cfg(test)]
    pub(super) fn plan(&self, needs: &[(u32, u32)], keep_from: u64) -> Option<Vec<u32>> {
        let keyed: Vec<((), (u32, u32))> = needs.iter().map(|&s| ((), s)).collect();
        let (rows, gave_up) = self.plan_keys(&keyed, keep_from);
        (gave_up.len() < needs.len() || needs.is_empty()).then_some(rows)
    }

    /// Frees the shelves `rows` (from `plan`): their images lose their ids (free for new images). The names evicted.
    pub(in super::super) fn free_rows(&mut self, rows: &[u32]) -> Vec<TexName> {
        let mut gone = Vec::new();
        for &row in rows {
            let ids: Vec<u32> = self.at.iter().filter(|(_, (_, r))| *r == row).map(|(id, _)| *id).collect();
            for id in ids {
                if let Some((name, _)) = self.at.remove(&id) {
                    self.ids.remove(&name);
                    self.spare.push(id);
                    gone.push(name);
                }
            }
            self.shelves.release(row);
        }
        gone
    }

    /// `plan` and `free_rows` in one step (tests): the names evicted.
    #[cfg(test)]
    pub(super) fn evict(&mut self, needs: &[(u32, u32)], keep_from: u64) -> Vec<TexName> {
        let rows = self.plan(needs, keep_from).unwrap_or_default();
        self.free_rows(&rows)
    }

    /// `touch` by name (tests).
    #[cfg(test)]
    pub(in super::super) fn touch_name(&mut self, name: &TexName, field: u64) {
        if let Some(id) = self.id_of(name) {
            self.touch(id, field);
        }
    }

    pub(in super::super) fn len(&self) -> usize {
        self.ids.len()
    }
}

/// The atlas and table textures with their bookkeeping. Created with 1 x 1 stand-ins; the real textures at the first image.
pub(super) struct ReplGpu {
    atlas: wgpu::TextureView,
    tab: wgpu::TextureView,
    tex: Option<(wgpu::Texture, wgpu::Texture)>,
    book: Option<Book>,
    rows: u32,
}

fn make_tex(dev: &wgpu::Device, label: &str, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    dev.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn view(t: &wgpu::Texture) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor::default())
}

impl ReplGpu {
    /// The idle state: two 1 x 1 stand-in textures (zero filled), no book. `RAC_TEXPACK_HUD_MB`: the atlas size once it is made.
    pub(super) fn new(dev: &wgpu::Device) -> ReplGpu {
        let mb = std::env::var("RAC_TEXPACK_HUD_MB").ok().and_then(|v| v.trim().parse::<u32>().ok());
        ReplGpu {
            atlas: view(&make_tex(dev, "gs repl atlas stand-in", 1, 1, wgpu::TextureFormat::Rgba8Unorm)),
            tab: view(&make_tex(dev, "gs repl table stand-in", 1, 1, wgpu::TextureFormat::Rgba32Uint)),
            tex: None,
            book: None,
            rows: atlas_rows(mb),
        }
    }

    /// The atlas id of `name`'s image if it was uploaded.
    pub(super) fn id_of(&self, name: &TexName) -> Option<u32> {
        self.book.as_ref()?.id_of(name)
    }

    /// The views for bindings 9 (atlas) and 10 (table).
    pub(super) fn views(&self) -> (&wgpu::TextureView, &wgpu::TextureView) {
        (&self.atlas, &self.tab)
    }

    /// Whether the real textures exist.
    pub(super) fn active(&self) -> bool {
        self.tex.is_some()
    }

    /// Uploads `img` for `name` (standing for a rectangle of `rw` x `rh` GS texels: the size of the hashed rectangle, `gsname::Name::w/h`;
    /// `name.region_w/h` is 0 for a texture without a region). Ok((id, true)): the textures were made by this call, so the bind group
    /// must be built again.
    pub(super) fn add(&mut self, dev: &wgpu::Device, q: &wgpu::Queue, name: TexName, img: &HudImg, rw: u32, rh: u32) -> Result<(u32, bool), Refuse> {
        if img.rgba.len() as u64 != img.w as u64 * img.h as u64 * 4 {
            return Err(Refuse::Bad);
        }
        let rows = self.rows;
        let book = self.book.get_or_insert_with(|| Book::new(rows));
        if let Some(id) = book.id_of(&name) {
            return Ok((id, false));
        }
        let p = book.place(name, img.w, img.h)?;
        let made = self.tex.is_none();
        if made {
            let a = make_tex(dev, "gs repl atlas", ATLAS_W, self.rows, wgpu::TextureFormat::Rgba8Unorm);
            let t = make_tex(dev, "gs repl table", MAX_IDS, 1, wgpu::TextureFormat::Rgba32Uint);
            (self.atlas, self.tab) = (view(&a), view(&t));
            self.tex = Some((a, t));
        }
        let (a, t) = self.tex.as_ref().unwrap();
        let at = |tex, x, y| wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All };
        let ext = |w, h| wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        q.write_texture(at(a, p.x, p.y), &img.rgba, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(img.w * 4), rows_per_image: Some(img.h) }, ext(img.w, img.h));
        let texel = table_texel(p.x, p.y, img.w, img.h, rw, rh);
        q.write_texture(at(t, p.id, 0), bytemuck::cast_slice(&texel), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(16), rows_per_image: Some(1) }, ext(1, 1));
        Ok((p.id, made))
    }

    /// The id of `name`'s image: the uploaded one, else `slot`'s image uploaded now (and the slot's CPU copy released). The flag is
    /// `add`'s. The `Miss` says why there is no id (see there).
    pub(super) fn get_or_add(&mut self, dev: &wgpu::Device, q: &wgpu::Queue, name: &TexName, slot: &Slot, rw: u32, rh: u32) -> Result<(u32, bool), Miss> {
        if let Some(id) = self.id_of(name) {
            return Ok((id, false));
        }
        let img = slot.image().ok_or(Miss::NoImage)?;
        let r = self.add(dev, q, *name, &img, rw, rh).map_err(|e| match e {
            Refuse::Full => Miss::Full(img.w, img.h),
            Refuse::Bad | Refuse::Never => Miss::Never,
        })?;
        slot.release();
        Ok(r)
    }

    /// The shelves to free for `needs` and the keys given up (see `Book::plan_keys`); with no book nothing can be freed and all are given up.
    pub(super) fn plan_evict<K: Clone>(&self, needs: &[(K, (u32, u32))], keep_from: u64) -> (Vec<u32>, Vec<K>) {
        match &self.book {
            Some(b) => b.plan_keys(needs, keep_from),
            None => (Vec::new(), needs.iter().map(|(k, _)| k.clone()).collect()),
        }
    }

    /// The image `id` was drawn in field number `field` (`Book::touch`).
    pub(super) fn touch(&mut self, id: u32, field: u64) {
        if let Some(b) = self.book.as_mut() {
            b.touch(id, field);
        }
    }

    /// Frees the shelves `rows` (see `Book::free_rows`); the names whose ids are gone.
    pub(super) fn free_rows(&mut self, rows: &[u32]) -> Vec<TexName> {
        self.book.as_mut().map_or_else(Vec::new, |b| b.free_rows(rows))
    }

    /// Images in the atlas.
    pub(super) fn images(&self) -> usize {
        self.book.as_ref().map_or(0, Book::len)
    }
}

/// The bind group layout of the capture kernels: bindings 0 (params, dynamic offset), 1 - 8 (the storage buffers) and the replacement
/// atlas and table (9, 10). Moved out of `Gpu::open`.
pub(super) fn layout_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let buf = |binding: u32, ty: wgpu::BufferBindingType, dynamic: bool| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: dynamic, min_binding_size: if dynamic { wgpu::BufferSize::new(256) } else { None } },
        count: None,
    };
    let tex = |binding: u32, sample_type| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
        count: None,
    };
    let (rw, ro) = (wgpu::BufferBindingType::Storage { read_only: false }, wgpu::BufferBindingType::Storage { read_only: true });
    let kinds = [wgpu::BufferBindingType::Uniform, rw, rw, rw, ro, ro, ro, ro, rw];
    let mut e: Vec<_> = kinds.iter().enumerate().map(|(i, &k)| buf(i as u32, k, i == 0)).collect();
    e.push(tex(9, wgpu::TextureSampleType::Float { filterable: false }));
    e.push(tex(10, wgpu::TextureSampleType::Uint));
    e
}

#[cfg(test)]
#[path = "gpu_repl_tests.rs"]
mod tests;
