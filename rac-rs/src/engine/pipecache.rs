//! The Vulkan pipeline cache files of the player folder (shared by the GS capture kernels, gscap/draw/gpu_cache.rs, and the level
//! pipelines, viewer/pipewarm.rs). A file is `<prefix>_<adapter, driver and shader hash>.bin`: 8 bytes check (FNV of the rest), then
//! the driver's data; a file that fails its check or that the driver refuses is ignored, the pipelines are then built and the file is
//! written again (tmp + rename).

/// One cache file and the driver cache loaded from it.
pub(crate) struct PipeCache {
    pub(crate) cache: wgpu::PipelineCache,
    path: std::path::PathBuf,
    /// The data size last loaded or written (`save` skips a write while it is unchanged).
    loaded: std::sync::atomic::AtomicUsize,
}

fn fnv(h: u64, b: &[u8]) -> u64 {
    b.iter().fold(h, |h, &x| (h ^ x as u64).wrapping_mul(0x0100_0000_01b3))
}

impl PipeCache {
    /// The GS capture kernels' cache (`gpu_kernels_<key>.bin`).
    pub(crate) fn open(dev: &wgpu::Device, info: &wgpu::AdapterInfo, shader: &str) -> PipeCache {
        PipeCache::open_named(dev, info, &[shader], "gpu_kernels", "gs kernels")
    }

    /// The cache `<prefix>_<key>.bin`; the key names the adapter, the driver and every text of `shader`.
    pub(crate) fn open_named(dev: &wgpu::Device, info: &wgpu::AdapterInfo, shader: &[&str], prefix: &str, label: &str) -> PipeCache {
        let mut key = fnv(0xcbf2_9ce4_8422_2325, info.name.as_bytes());
        key = fnv(key, info.driver.as_bytes());
        key = fnv(key, info.driver_info.as_bytes());
        key = fnv(key, &info.vendor.to_le_bytes());
        key = fnv(key, &info.device.to_le_bytes());
        for s in shader {
            key = fnv(key, s.as_bytes());
        }
        let dir = std::env::var_os("RAC_GSGPU_CACHEDIR").map(std::path::PathBuf::from).unwrap_or_else(crate::pcopts::player_dir);
        let path = dir.join(format!("{prefix}_{key:016x}.bin"));
        // File: 8 bytes check (FNV of the rest), then the driver's data.
        let data = std::fs::read(&path).ok().filter(|d| d.len() > 8 && fnv(0xcbf2_9ce4_8422_2325, &d[8..]).to_le_bytes() == d[..8]);
        let loaded = data.as_ref().map_or(0, |d| d.len() - 8);
        // SAFETY: the data is either None or what `PipelineCache::get_data` returned for this adapter and shader (checked above; the
        // key names the adapter, driver and shader); `fallback` lets wgpu start an empty cache when the driver refuses it anyway.
        let cache = unsafe { dev.create_pipeline_cache(&wgpu::PipelineCacheDescriptor { label: Some(label), data: data.as_deref().map(|d| &d[8..]), fallback: true }) };
        PipeCache { cache, path, loaded: loaded.into() }
    }

    /// Writes the file when the driver's data changed size since it was loaded or last written. The temporary file is unique per
    /// process and call (test build 7 review: two saves at once, the pool's and the periodic one, never share it).
    pub(crate) fn save(&self) {
        use std::sync::atomic::Ordering;
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let Some(d) = self.cache.get_data() else { return };
        if d.len() == self.loaded.load(Ordering::Relaxed) {
            return;
        }
        let mut f = fnv(0xcbf2_9ce4_8422_2325, &d).to_le_bytes().to_vec();
        f.extend_from_slice(&d);
        if let Some(p) = self.path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let tmp = self.path.with_extension(format!("{}.{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        if std::fs::write(&tmp, &f).is_err() {
            let _ = std::fs::remove_file(&tmp);
        } else if std::fs::rename(&tmp, &self.path).is_ok() {
            self.loaded.store(d.len(), Ordering::Relaxed);
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}
