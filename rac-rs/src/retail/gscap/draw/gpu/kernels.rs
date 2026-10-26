//! The GS capture's compute kernels: building the 13 pipelines (`Gpu::pipes`) and what happens when a driver or shader compiler refuses
//! one. A refused kernel (DX12 with FXC: "unable to unroll loop"; a driver bug on any API) is caught in a per-thread error scope, so it
//! never reaches wgpu's default handler (a panic); `Gpu::open` then gives the capture up with a log line and the game draws the HUD
//! on the CPU, as it does without a GPU capture.
use super::{Gpu, PipeCache, IDLE, KERNELS, K_HDF};

impl Gpu {
    /// The kernels; `soft`: every f32 operation on the integer path. The driver's first build of a kernel costs seconds of one core and
    /// a few hundred MB (gpu.wgsl is written to keep that small), so a few workers take the kernels in turn, the costly ones first,
    /// and the two with the whole pixel pipeline never together: the game's own threads keep their cores while this runs.
    /// Err: what the compiler said about the kernels it refused (all kernels are tried, so the log names every failing one).
    pub(super) fn pipes(dev: &wgpu::Device, pl: &wgpu::PipelineLayout, sm: &wgpu::ShaderModule, soft: bool, cache: Option<&PipeCache>, dx12: bool) -> Result<Vec<wgpu::ComputePipeline>, String> {
        // `KERNELS` indices, costly first.
        const ORDER: [usize; 13] = [7, 1, 0, 8, 9, 10, 12, 6, 2, 3, 4, 5, 11];
        let workers = std::env::var("RAC_GSGPU_BUILDERS").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or_else(|| (std::thread::available_parallelism().map_or(4, |n| n.get()) / 4).clamp(1, if dx12 { 4 } else { 3 }));
        let log = std::env::var_os("RAC_GSGPU_BUILDLOG").is_some();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let big = std::sync::Mutex::new(());
        let out: std::sync::Mutex<Vec<Option<wgpu::ComputePipeline>>> = std::sync::Mutex::new((0..KERNELS.len()).map(|_| None).collect());
        let failed: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|sc| {
            for _ in 0..workers {
                sc.spawn(|| {
                    if let Some(f) = IDLE.get() {
                        f();
                    }
                    loop {
                        let n = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let Some(&ki) = ORDER.get(n) else { break };
                        let k = KERNELS[ki];
                        let c = [
                            ("SOFT", if soft { 1.0 } else { 0.0 }),
                            ("FAST", if matches!(k, "k_raster" | "k_shade" | "k_put") { 1.0 } else { 0.0 }),
                            ("HDT", if ki == K_HDF as usize { 0.0 } else { 1.0 }),
                        ];
                        // DX12 (DXC) takes the two together: its compiler runs in its own instance per call and the start waits for the sum otherwise.
                        let _one = (!dx12 && matches!(k, "k_redo" | "k_serial")).then(|| big.lock());
                        let t = std::time::Instant::now();
                        // Error scopes are per thread: a refusal here is this kernel's, and the window's own errors stay with its handler.
                        let (sv, si) = (dev.push_error_scope(wgpu::ErrorFilter::Validation), dev.push_error_scope(wgpu::ErrorFilter::Internal));
                        let p = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some(k),
                            layout: Some(pl),
                            module: sm,
                            entry_point: Some(k),
                            compilation_options: wgpu::PipelineCompilationOptions { constants: &c, ..Default::default() },
                            cache: cache.map(|c| &c.cache),
                        });
                        let err = pollster::block_on(si.pop()).or_else(|| pollster::block_on(sv.pop()));
                        if log || t.elapsed().as_millis() > 1000 {
                            eprintln!("gscap: kernel {k} took {} ms to build", t.elapsed().as_millis());
                        }
                        match err {
                            Some(e) => failed.lock().unwrap_or_else(|e| e.into_inner()).push(format!("{k}: {e}")),
                            None => out.lock().unwrap_or_else(|e| e.into_inner())[ki] = Some(p),
                        }
                    }
                });
            }
        });
        let failed = failed.into_inner().unwrap_or_default();
        if !failed.is_empty() {
            return Err(failed.join(" | "));
        }
        if let Some(c) = cache {
            c.save();
        }
        Ok(out.into_inner().unwrap_or_default().into_iter().map(|p| p.expect("every kernel was built")).collect())
    }

    /// RAC_TEST_GSCAP_WAIT (tests): the game waits for the GPU capture to finish opening instead of drawing on the CPU meanwhile, so the
    /// takeover is at the first field on every API (DX12 builds its kernels at every start) and a gate picture shows the sharp HUD.
    pub(super) fn test_wait() -> bool {
        std::env::var_os("RAC_TEST_GSCAP_WAIT").is_some()
    }

    /// `Gpu::open` found the kernels refused (`pipes` Err): the capture is given up. The start-up marker goes (the start did not freeze,
    /// it just has no GPU capture), and the CPU capture takes over.
    pub(super) fn kernels_failed(adapter: &str, why: &str) -> Option<Box<Gpu>> {
        let why: String = why.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).take(1200).collect();
        eprintln!("gscap gpu: {adapter}: the compute kernels did not build ({why}); the HUD capture draws on the CPU");
        crate::gscap_guard::cancel();
        None
    }
}
