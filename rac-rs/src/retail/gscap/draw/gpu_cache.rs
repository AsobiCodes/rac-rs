//! The Vulkan pipeline cache of the GS capture kernels (moved out of gpu.rs to keep that file under its size cap).

/// The Vulkan pipeline cache of the kernels, kept in the player folder (`gpu_kernels_<adapter, driver and shader hash>.bin`; a file
/// that fails its check or that the driver refuses is ignored, the kernels are then built and the file is written again).
pub(crate) struct PipeCache {
    pub(super) cache: wgpu::PipelineCache,
    path: std::path::PathBuf,
    loaded: usize,
}

impl PipeCache {
    pub(super) fn open(dev: &wgpu::Device, info: &wgpu::AdapterInfo, shader: &str) -> PipeCache {
        let fnv = |h: u64, b: &[u8]| b.iter().fold(h, |h, &x| (h ^ x as u64).wrapping_mul(0x0100_0000_01b3));
        let mut key = fnv(0xcbf2_9ce4_8422_2325, info.name.as_bytes());
        key = fnv(key, info.driver.as_bytes());
        key = fnv(key, info.driver_info.as_bytes());
        key = fnv(key, &info.vendor.to_le_bytes());
        key = fnv(key, &info.device.to_le_bytes());
        key = fnv(key, shader.as_bytes());
        let dir = std::env::var_os("RAC_GSGPU_CACHEDIR").map(std::path::PathBuf::from).unwrap_or_else(crate::retail::memcard::player_dir);
        let path = dir.join(format!("gpu_kernels_{key:016x}.bin"));
        // File: 8 bytes check (FNV of the rest), then the driver's data.
        let data = std::fs::read(&path).ok().filter(|d| d.len() > 8 && fnv(0xcbf2_9ce4_8422_2325, &d[8..]).to_le_bytes() == d[..8]);
        let loaded = data.as_ref().map_or(0, |d| d.len() - 8);
        // SAFETY: the data is either None or what `PipelineCache::get_data` returned for this adapter and shader (checked above; the
        // key names the adapter, driver and shader); `fallback` lets wgpu start an empty cache when the driver refuses it anyway.
        let cache = unsafe { dev.create_pipeline_cache(&wgpu::PipelineCacheDescriptor { label: Some("gs kernels"), data: data.as_deref().map(|d| &d[8..]), fallback: true }) };
        PipeCache { cache, path, loaded }
    }

    pub(super) fn save(&self) {
        let Some(d) = self.cache.get_data() else { return };
        if d.len() == self.loaded {
            return;
        }
        let fnv = d.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x0100_0000_01b3));
        let mut f = fnv.to_le_bytes().to_vec();
        f.extend_from_slice(&d);
        if let Some(p) = self.path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, &f).is_ok() && std::fs::rename(&tmp, &self.path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}
