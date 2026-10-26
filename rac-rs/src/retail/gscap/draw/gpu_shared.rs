//! Which GPU device the GS capture (`gpu.rs`) draws on. By default the WINDOW'S OWN device, Vulkan or DirectX 12: the window publishes
//! it (`window_ready`, viewer/gpu/setup.rs, right after its `request_device`) and `acquire` hands out clones of it, so the capture never
//! opens a second instance and device. That second Vulkan device, opened on a thread while the window's device started presenting,
//! froze the window white before its first frame on an RX 9070 XT (GitHub #12; v0.2.2, which had no GPU capture, ran), which is why
//! v0.3.1 limited the GPU capture to NVIDIA (v0.3.5: graphics cards of every vendor, not built-in graphics, `default_on`).
//! On DirectX 12 the kernels need Microsoft's DXC compiler (`dxcompiler.dll` next to the exe, `dx12_dxc`): the old FXC rejects five of
//! them ("unable to unroll loop") and needs minutes for k_hd. A DX12 window without DXC keeps the CPU capture (`default_on`), and a
//! DX12 window NEVER opens a Vulkan instance (DX12 is the backup when a Vulkan driver or layer misbehaves): there is no second device
//! on DX12 at all. `RAC_GSCAP_GPU=own` opens the old own Vulkan device for A/B on a Vulkan window. Without a window (`rac-rs warm-gpu`,
//! unit tests) nothing is ever published and `acquire` opens its own device, of the API the player chose (`own`).
//!
//! Order at a start: `expect_window` (handler.rs `resumed`), `Gpu::new` calls `window_ready` (publishes the device, or declines when the
//! window is neither Vulkan nor DX12), then the game loads and `Gpu::prestart` opens the capture on a background thread: `acquire` waits
//! (Condvar, no busy loop) only while a window is expected and has not answered yet.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// The window's device, as the capture borrows it (clones: wgpu devices and queues are shared handles).
#[derive(Clone)]
pub(crate) struct Shared {
    pub dev: wgpu::Device,
    pub q: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
}

/// What `acquire` returns: a device with what `Gpu::open` needs to know about it.
pub(super) struct Dev {
    pub dev: wgpu::Device,
    pub q: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    /// TIMESTAMP_QUERY is on and RAC_GSGPU_TIME asked for the per-kernel timings.
    pub timing: bool,
    /// PIPELINE_CACHE is on (the kernels keep their `PipeCache` file).
    pub has_cache: bool,
    /// The window's device (not one of its own).
    pub shared: bool,
}

enum Slot<T> {
    /// No window announced itself (warm-gpu, tests): nothing to wait for.
    NoWindow,
    /// A window is starting; its `Gpu::new` has not answered yet.
    Pending,
    Ready(T),
    Declined,
}

/// The window's answer, with the wait for it.
pub(crate) struct Board<T> {
    slot: Mutex<Slot<T>>,
    cv: Condvar,
}

impl<T: Clone> Board<T> {
    pub(crate) const fn new() -> Self {
        Board { slot: Mutex::new(Slot::NoWindow), cv: Condvar::new() }
    }

    /// A window is starting: `wait` blocks until `publish` or `decline` (an answer already given stays).
    pub(crate) fn expect(&self) {
        if let Ok(mut s) = self.slot.lock() {
            if matches!(*s, Slot::NoWindow) {
                *s = Slot::Pending;
            }
        }
    }

    pub(crate) fn publish(&self, v: T) {
        if let Ok(mut s) = self.slot.lock() {
            *s = Slot::Ready(v);
        }
        self.cv.notify_all();
    }

    /// The window has no device the capture can use (not Vulkan).
    pub(crate) fn decline(&self) {
        if let Ok(mut s) = self.slot.lock() {
            *s = Slot::Declined;
        }
        self.cv.notify_all();
    }

    /// The published value, waiting at most `limit` while a window is still starting; None when none is (or will be) published.
    pub(crate) fn wait(&self, limit: Duration) -> Option<T> {
        let end = Instant::now() + limit;
        let mut s = self.slot.lock().ok()?;
        while matches!(*s, Slot::Pending) {
            let left = end.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())?;
            s = self.cv.wait_timeout(s, left).ok()?.0;
        }
        match &*s {
            Slot::Ready(v) => Some(v.clone()),
            _ => None,
        }
    }
}

static BOARD: Board<Shared> = Board::new();

/// Longest wait for a window that said it is starting (its `Gpu::new` takes a second or two; a hung one must not hold the game).
const WAIT: Duration = Duration::from_secs(30);

static SUBMIT: Mutex<()> = Mutex::new(());

/// Held by the capture around each `Queue::submit` and by the window around `Surface::configure` (viewer/gpu/setup.rs
/// `configure_surface`). On the shared device wgpu's configure waits for the GPU to go idle and fails with "Failed to wait for GPU to
/// come idle before reconfiguring the Surface" (a panic) when another thread submitted meanwhile: seen 2026-10-23 on the AMD iGPU
/// (the first window resize during a run). One mutex keeps the capture's submit out of that wait; it is held for microseconds.
pub(crate) fn submit_lock() -> std::sync::MutexGuard<'static, ()> {
    SUBMIT.lock().unwrap_or_else(|e| e.into_inner())
}

/// `Queue::submit` under `submit_lock` (the capture's only way to submit).
pub(super) fn submit(q: &wgpu::Queue, cmds: impl IntoIterator<Item = wgpu::CommandBuffer>) {
    let _lock = submit_lock();
    q.submit(cmds);
}

/// handler.rs `resumed`, before the window's `Gpu::new`.
pub(crate) fn expect_window() {
    crate::gscap_guard::activate();
    BOARD.expect();
}

/// The window's API is DirectX 12 (set by `window_ready` before the answer is published): `acquire` then never opens a Vulkan device.
static DX12_WINDOW: AtomicBool = AtomicBool::new(false);

/// `dxcompiler.dll` next to the exe, when it is there (viewer/gpu/renderer.rs `dxc_next_to_exe` looks at the same file).
fn dxc_dll() -> Option<String> {
    let dll = std::env::current_exe().ok()?.parent()?.join("dxcompiler.dll");
    dll.is_file().then(|| dll.to_string_lossy().into_owned())
}

/// Whether a DX12 instance made by `viewer/gpu/renderer.rs` compiles with DXC: `WGPU_DX12_COMPILER` (fxc / dxc / staticdxc; `auto` counts
/// as FXC) when set, else the DLL next to the exe. The capture's kernels need it (module doc).
pub(crate) fn dx12_dxc() -> bool {
    match wgpu::Dx12Compiler::from_env() {
        Some(wgpu::Dx12Compiler::Fxc | wgpu::Dx12Compiler::Auto) => false,
        Some(_) => true,
        None => dxc_dll().is_some(),
    }
}

/// Whether the GPU capture is on without RAC_GSCAP_GPU: on a Vulkan or DX12 window it shares the window's device, on a DX12 window only
/// when the kernels can be compiled (`dxc`: DXC, module doc). Built-in (integrated) graphics keep the CPU capture whatever the vendor
/// or API (user 2026-10-23: the sharp HUD halves their frame rate; the Original HUD is the default there). This is the ONE place for
/// the default; `pc_options.txt gscap_gpu=auto` (gscap_guard.rs) means this rule, RAC_GSCAP_GPU=1 forces the GPU.
pub(crate) fn default_on(backend: wgpu::Backend, ty: wgpu::DeviceType, dxc: bool) -> bool {
    ty != wgpu::DeviceType::IntegratedGpu
        && match backend {
            wgpu::Backend::Vulkan => true,
            wgpu::Backend::Dx12 => dxc,
            _ => false,
        }
}

/// The window's `Gpu::new`, after `request_device`: publishes the device when the window is Vulkan or DX12, else declines, and sets the
/// capture's default (`gpu::set_default`).
pub(crate) fn window_ready(dev: &wgpu::Device, q: &wgpu::Queue, info: &wgpu::AdapterInfo) {
    crate::gscap_guard::note_window(info);
    DX12_WINDOW.store(info.backend == wgpu::Backend::Dx12, Ordering::SeqCst);
    if matches!(info.backend, wgpu::Backend::Vulkan | wgpu::Backend::Dx12) {
        BOARD.publish(Shared { dev: dev.clone(), q: q.clone(), info: info.clone() });
    } else {
        BOARD.decline();
    }
    super::gpu::set_default(default_on(info.backend, info.device_type, dx12_dxc()));
}

/// The features the window's device asks for on top of its own, for the capture to use it: PIPELINE_CACHE (the kernels' cache file;
/// not with RAC_GSGPU_NOCACHE) and, for RAC_GSGPU_TIME, TIMESTAMP_QUERY. Vulkan only.
pub(crate) fn window_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    if adapter.get_info().backend != wgpu::Backend::Vulkan {
        return wgpu::Features::empty();
    }
    let have = adapter.features();
    let mut f = wgpu::Features::empty();
    if std::env::var_os("RAC_GSGPU_NOCACHE").is_none() {
        f |= wgpu::Features::PIPELINE_CACHE & have;
    }
    if std::env::var_os("RAC_GSGPU_TIME").is_some() {
        f |= wgpu::Features::TIMESTAMP_QUERY & have;
    }
    f
}

/// RAC_GSCAP_GPU's value for the capture: None = CPU. `cpu_default`: nothing is set and the default is the CPU (`set_default`), or the
/// start-up guard switched the GPU capture off (gscap_guard.rs; RAC_GSCAP_GPU set always wins).
pub(crate) fn resolve(env: Option<String>, cpu_default: bool) -> Option<String> {
    let w = match env.filter(|w| !w.is_empty()) {
        Some(w) => w,
        None if cpu_default || crate::gscap_guard::off() => return None,
        None => "1".into(),
    };
    (w != "0").then_some(w)
}

/// Whether `want` (a `resolve` result) takes the window's device: "1" does, "own" and "low" never, an adapter name part when the
/// window's adapter has it in its name.
pub(crate) fn uses_window(want: &str, window_adapter: &str) -> bool {
    match want {
        "1" => true,
        "own" | "low" => false,
        n => window_adapter.to_lowercase().contains(&n.to_lowercase()),
    }
}

/// The capture's device for `want`: the window's when one is published and `want` takes it, else one of its own. This is the step
/// the start-up guard watches (gscap_guard.rs: the marker is written first; no device: it is removed again).
pub(super) fn acquire(want: &str) -> Option<Dev> {
    crate::gscap_guard::begin();
    let dev = pick(want);
    match &dev {
        Some(d) => crate::gscap_guard::note_adapter(&d.info),
        None => crate::gscap_guard::cancel(),
    }
    dev
}

fn pick(want: &str) -> Option<Dev> {
    let dx12 = DX12_WINDOW.load(Ordering::SeqCst);
    // On a DX12 window "own" and "low" (a Vulkan device of its own) have no meaning: it takes the window's device.
    let w = if dx12 && matches!(want, "own" | "low") { "1" } else { want };
    if w != "own" {
        if let Some(s) = BOARD.wait(WAIT).filter(|s| uses_window(w, &s.info.name)) {
            let f = s.dev.features();
            let timing = std::env::var_os("RAC_GSGPU_TIME").is_some() && f.contains(wgpu::Features::TIMESTAMP_QUERY);
            return Some(Dev { has_cache: f.contains(wgpu::Features::PIPELINE_CACHE), dev: s.dev, q: s.q, info: s.info, timing, shared: true });
        }
    }
    if dx12 {
        eprintln!("gscap gpu: the window is DirectX 12 and RAC_GSCAP_GPU={want} names no other adapter: no Vulkan device is opened, the capture draws on the CPU");
        return None;
    }
    own(if want == "own" { "1" } else { want })
}

/// The API of a device of the capture's own (no window to share, or `own`): DirectX 12 when WGPU_BACKEND says so or, without that, the
/// player's `renderer=dx12` (so `rac-rs warm-gpu` of a DX12 player never loads Vulkan), else Vulkan.
fn own_is_dx12() -> bool {
    match wgpu::Backends::from_env() {
        Some(b) => b == wgpu::Backends::DX12,
        None => crate::retail::pcopts::PcOptions::load(&crate::retail::pcopts::file()).renderer_dx12(),
    }
}

/// An instance, adapter and device of the capture's own, of one API only: Vulkan, or DirectX 12 with DXC (`own_is_dx12`; without
/// DXC the capture draws on the CPU). Never both: a DX12 player's process loads no Vulkan driver.
fn own(want: &str) -> Option<Dev> {
    let dx12 = own_is_dx12();
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    let backends = if dx12 { wgpu::Backends::DX12 } else { wgpu::Backends::VULKAN };
    desc.backends = backends;
    if dx12 {
        if !dx12_dxc() {
            eprintln!("gscap gpu: DirectX 12 without dxcompiler.dll next to the exe: no GPU capture, it draws on the CPU");
            return None;
        }
        desc.backend_options.dx12.shader_compiler = dxc_dll().map_or(wgpu::Dx12Compiler::Fxc, |dxc_path| wgpu::Dx12Compiler::DynamicDxc { dxc_path });
    }
    let inst = wgpu::Instance::new(desc);
    let adapter = if want == "1" || want == "low" {
        let pp = if want == "low" { wgpu::PowerPreference::LowPower } else { wgpu::PowerPreference::HighPerformance };
        pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions { power_preference: pp, compatible_surface: None, force_fallback_adapter: false, apply_limit_buckets: false })).ok()?
    } else {
        let w = want.to_lowercase();
        pollster::block_on(inst.enumerate_adapters(backends)).into_iter().find(|a| a.get_info().name.to_lowercase().contains(&w))?
    };
    let info = adapter.get_info();
    let timing = std::env::var_os("RAC_GSGPU_TIME").is_some() && adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
    let mut features = if timing { wgpu::Features::TIMESTAMP_QUERY } else { wgpu::Features::empty() };
    // Vulkan keeps the compiled kernels in a pipeline cache file of the player folder (`PipeCache`): later starts skip the driver's build
    // even when the driver's own cache has lost them.
    let has_cache = adapter.features().contains(wgpu::Features::PIPELINE_CACHE) && std::env::var_os("RAC_GSGPU_NOCACHE").is_none();
    if has_cache {
        features |= wgpu::Features::PIPELINE_CACHE;
    }
    let (dev, q) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("gs capture"), required_features: features, ..Default::default() })).ok()?;
    dev.set_device_lost_callback(|reason, msg| {
        if reason != wgpu::DeviceLostReason::Destroyed {
            eprintln!("gscap gpu: the capture device was lost ({reason:?}): {msg}");
        }
    });
    Some(Dev { dev, q, info, timing, has_cache, shared: false })
}

/// [PC] (speed) Waits until the `want` map callbacks counted in `done` have run (the lazy readback, `Gpu::rb_finish`). The window
/// shares this device: a blocking `Device::poll(wait_indefinitely)` holds wgpu's snatch lock (read) for the whole GPU wait, so the
/// render thread's buffer drops, `write_buffer`s and submits queue up behind it and the GPU idles. This polls without blocking and
/// yields / sleeps 50 us in between. `RAC_GSCAP_WAIT=1` = the old blocking poll (A/B), which is also the fallback after `POLL_LIMIT`.
pub(super) fn wait_maps(dev: &wgpu::Device, done: &std::sync::atomic::AtomicU32, want: u32, block: bool) {
    if !block {
        dev.poll(wgpu::PollType::Poll).ok();
        return;
    }
    static OLD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *OLD.get_or_init(|| std::env::var_os("RAC_GSCAP_WAIT").is_some_and(|v| v != "0")) {
        dev.poll(wgpu::PollType::wait_indefinitely()).ok();
        return;
    }
    const POLL_LIMIT: Duration = Duration::from_secs(5);
    let t = Instant::now();
    let mut n = 0u32;
    loop {
        dev.poll(wgpu::PollType::Poll).ok();
        if done.load(Ordering::SeqCst) >= want {
            return;
        }
        if t.elapsed() > POLL_LIMIT {
            dev.poll(wgpu::PollType::wait_indefinitely()).ok();
            return;
        }
        n += 1;
        if n < 16 {
            std::thread::yield_now();
        } else {
            std::thread::sleep(Duration::from_micros(50));
        }
    }
}
