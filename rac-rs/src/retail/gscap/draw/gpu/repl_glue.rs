//! [PC] The capture's side of the texture-pack atlas (`gpu_repl.rs`): the calls `texrepl.rs` makes on the `Gpu` (moved out of gpu.rs).

use super::super::gpu_repl::{unpack_word, Miss};
use super::Gpu;
use crate::texpack::name::TexName;

impl Gpu {
    /// The atlas id of `name`'s replacement (record word 1, `gpu_repl::pack_word`), uploaded on first use for a named rectangle of `rw` x `rh`
    /// GS texels; `Miss` says why there is none.
    pub(in crate::retail::gscap) fn repl_id(&mut self, name: &TexName, slot: &crate::texpack::hudrepl::Slot, rw: u32, rh: u32) -> Result<u32, Miss> {
        let (id, made) = self.repl.get_or_add(&self.dev, &self.q, name, slot, rw, rh)?;
        if made {
            self.bg = Self::bind(&self.dev, &self.bgl, &self.bufs, &self.repl);
        }
        Ok(id)
    }

    /// The image of record word 1 `word` was drawn in field number `field` (what the eviction protects and ranks by).
    pub(in crate::retail::gscap) fn repl_touch(&mut self, word: u32, field: u64) {
        self.repl.touch(unpack_word(word).0, field);
    }

    /// Frees atlas room for the refused images `needs` ((key, (w, h))), least recently used first and never what was drawn in field
    /// `keep_from` or later (`Book::plan_keys`). Returns the names whose ids are gone (empty when there was room already) and the keys of
    /// the images given up (no eviction can make room for them). Everything recorded so far is submitted before a shelf is freed, so no
    /// step that still has to run reads a region that a later upload reuses.
    pub(in crate::retail::gscap) fn repl_evict<K: Clone>(&mut self, needs: &[(K, (u32, u32))], keep_from: u64) -> (Vec<TexName>, Vec<K>) {
        let (rows, gave_up) = self.repl.plan_evict(needs, keep_from);
        if rows.is_empty() {
            return (Vec::new(), gave_up);
        }
        self.submit(None);
        (self.repl.free_rows(&rows), gave_up)
    }
}
