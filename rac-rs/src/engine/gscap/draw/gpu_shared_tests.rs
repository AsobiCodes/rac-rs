//! Unit tests of the GS capture's device choice (`gpu_shared.rs`): the default per vendor and backend, RAC_GSCAP_GPU's values, and the
//! window's slot (publish, take, no deadlock without a window).
use super::gpu_shared::{default_on, resolve, second_device_wanted, uses_window, Board};
use std::time::{Duration, Instant};
use wgpu::Backend::{Dx12, Vulkan};
use wgpu::DeviceType::{DiscreteGpu as Discrete, IntegratedGpu as Integrated};

#[test]
fn default_is_on_on_a_vulkan_window_for_a_graphics_card_of_any_kind() {
    for ty in [Discrete, wgpu::DeviceType::VirtualGpu, wgpu::DeviceType::Other, wgpu::DeviceType::Cpu] {
        // DXC does not matter for Vulkan.
        assert!(default_on(Vulkan, ty, false) && default_on(Vulkan, ty, true), "{ty:?} on Vulkan");
    }
}

/// A DX12 window is like a Vulkan one for every vendor (the vendor is no input any more), but only with the DXC compiler: FXC rejects
/// five kernels and needs minutes for k_hd.
#[test]
fn default_on_a_dx12_window_needs_dxc() {
    for ty in [Discrete, wgpu::DeviceType::VirtualGpu, wgpu::DeviceType::Other] {
        assert!(default_on(Dx12, ty, true), "{ty:?} on DX12 with DXC");
        assert!(!default_on(Dx12, ty, false), "{ty:?} on DX12 without DXC");
    }
    assert!(!default_on(wgpu::Backend::Gl, Discrete, true));
    assert!(!default_on(wgpu::Backend::Metal, Discrete, true));
}

/// Built-in graphics are off whatever the API or compiler.
#[test]
fn default_is_off_on_built_in_graphics() {
    for b in [Vulkan, Dx12] {
        for dxc in [false, true] {
            assert!(!default_on(b, Integrated, dxc), "{b:?} built-in, dxc {dxc}");
        }
    }
}

#[test]
fn rac_gscap_gpu_values() {
    let r = |e: Option<&str>, cpu| resolve(e.map(String::from), cpu);
    // Not set: the default decides.
    assert_eq!(r(None, false).as_deref(), Some("1"));
    assert_eq!(r(None, true), None);
    assert_eq!(r(Some(""), false).as_deref(), Some("1"));
    assert_eq!(r(Some(""), true), None);
    // 0 is the CPU whatever the default; anything else asks for the GPU even where the default is the CPU.
    assert_eq!(r(Some("0"), false), None);
    assert_eq!(r(Some("0"), true), None);
    assert_eq!(r(Some("1"), true).as_deref(), Some("1"));
    assert_eq!(r(Some("own"), false).as_deref(), Some("own"));
    assert_eq!(r(Some("own"), true).as_deref(), Some("own"));
    assert_eq!(r(Some("low"), false).as_deref(), Some("low"));
    assert_eq!(r(Some("radeon"), true).as_deref(), Some("radeon"));
}

#[test]
fn which_values_take_the_windows_device() {
    let amd = "AMD Radeon(TM) Graphics";
    assert!(uses_window("1", amd));
    assert!(!uses_window("own", amd));
    assert!(!uses_window("low", amd));
    assert!(uses_window("radeon", amd));
    assert!(uses_window("RADEON", amd));
    assert!(!uses_window("nvidia", amd));
}

#[test]
fn without_a_window_nothing_waits() {
    let b: Board<u32> = Board::new();
    let t = Instant::now();
    assert_eq!(b.wait(Duration::from_secs(30)), None);
    assert!(t.elapsed() < Duration::from_secs(1), "warm-gpu must not wait for a window that never comes");
}

#[test]
fn a_published_value_is_taken_again_and_again() {
    let b: Board<u32> = Board::new();
    b.expect();
    b.publish(7);
    assert_eq!(b.wait(Duration::from_secs(1)), Some(7));
    assert_eq!(b.wait(Duration::from_secs(1)), Some(7));
    // A late `expect` does not take the answer back.
    b.expect();
    assert_eq!(b.wait(Duration::from_secs(1)), Some(7));
}

#[test]
fn a_declined_window_gives_none_at_once() {
    let b: Board<u32> = Board::new();
    b.expect();
    b.decline();
    let t = Instant::now();
    assert_eq!(b.wait(Duration::from_secs(30)), None);
    assert!(t.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_waiter_wakes_when_the_window_publishes() {
    static B: Board<u32> = Board::new();
    B.expect();
    let h = std::thread::spawn(|| B.wait(Duration::from_secs(20)));
    std::thread::sleep(Duration::from_millis(50));
    B.publish(42);
    assert_eq!(h.join().unwrap(), Some(42));
}

#[test]
fn a_window_that_never_answers_is_given_up_after_the_limit() {
    let b: Board<u32> = Board::new();
    b.expect();
    let t = Instant::now();
    assert_eq!(b.wait(Duration::from_millis(80)), None);
    let e = t.elapsed();
    assert!(e >= Duration::from_millis(70) && e < Duration::from_secs(5), "{e:?}");
}

#[test]
fn the_submit_lock_excludes_a_second_holder_and_survives_a_poisoned_holder() {
    let g = super::gpu_shared::submit_lock();
    // The holder's 80 ms start after the second thread has taken its start time (a late thread start on a loaded PC cannot shorten it).
    let (ready, started) = std::sync::mpsc::channel();
    let h = std::thread::spawn(move || {
        let t = Instant::now();
        ready.send(()).unwrap();
        let _g = super::gpu_shared::submit_lock();
        t.elapsed()
    });
    started.recv().unwrap();
    std::thread::sleep(Duration::from_millis(80));
    drop(g);
    assert!(h.join().unwrap() >= Duration::from_millis(80));
    let _ = std::thread::spawn(|| {
        let _g = super::gpu_shared::submit_lock();
        panic!("poison the lock");
    })
    .join();
    drop(super::gpu_shared::submit_lock());
}

/// v0.4.1: NVIDIA on Vulkan gets a second device on the window's adapter; AMD / Intel / DX12 and every explicit RAC_GSCAP_GPU keep the
/// window's device, RAC_GSCAP_DEV overrides both ways.
#[test]
fn second_device_only_on_nvidia_vulkan_without_overrides() {
    let (nv, amd, intel) = (0x10de, 0x1002, 0x8086);
    assert!(second_device_wanted("1", None, None, Vulkan, nv));
    assert!(second_device_wanted("1", Some(""), Some(""), Vulkan, nv));
    assert!(!second_device_wanted("1", None, None, Vulkan, amd));
    assert!(!second_device_wanted("1", None, None, Vulkan, intel));
    assert!(!second_device_wanted("1", None, None, Dx12, nv), "DX12 never");
    assert!(!second_device_wanted("1", Some("1"), None, Vulkan, nv), "RAC_GSCAP_GPU=1 is the window's device");
    assert!(!second_device_wanted("radeon", None, None, Vulkan, nv), "only the plain GPU capture");
    assert!(!second_device_wanted("1", None, Some("window"), Vulkan, nv));
    assert!(second_device_wanted("1", None, Some("second"), Vulkan, amd));
    assert!(second_device_wanted("1", Some("1"), Some("second"), Vulkan, nv));
    assert!(!second_device_wanted("1", None, Some("second"), Dx12, nv));
}

/// Review R2 F6: the lazy readback waits with a blocking poll on a device of the capture's own (`shared` false) and with the
/// non-blocking poll loop on the window's device; both return once the map callback has run. Skipped without a Vulkan adapter.
#[test]
fn wait_maps_returns_after_the_map_callback_on_both_kinds_of_device() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    desc.backends = wgpu::Backends::VULKAN;
    let inst = wgpu::Instance::new(desc);
    let Ok(adapter) = pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("no Vulkan adapter: skipped");
        return;
    };
    let Ok((dev, q)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
        eprintln!("no Vulkan device: skipped");
        return;
    };
    for shared in [false, true] {
        let buf = |usage| dev.create_buffer(&wgpu::BufferDescriptor { label: None, size: 4096, usage, mapped_at_creation: false });
        let (src, dst) = (buf(wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST), buf(wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ));
        let aux = buf(wgpu::BufferUsages::COPY_DST);
        let mut enc = dev.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&src, 0, &dst, 0, 4096);
        let sub = q.submit([enc.finish()]);
        // Later work on the same queue (the capture keeps submitting after a lazy readback); the wait is for `sub` only.
        let mut later = dev.create_command_encoder(&Default::default());
        later.copy_buffer_to_buffer(&src, 0, &aux, 0, 1024);
        q.submit([later.finish()]);
        let done = Arc::new(AtomicU32::new(0));
        let d = done.clone();
        dst.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            r.unwrap();
            d.fetch_add(1, Ordering::SeqCst);
        });
        super::gpu_shared::wait_maps_on(&dev, &done, 1, true, shared, Some(sub));
        assert_eq!(done.load(Ordering::SeqCst), 1, "shared {shared}");
        dst.unmap();
    }
}

/// Review N2: which kind of device the capture draws on is the exact `Dev::shared` of `acquire`, never a comparison of wgpu device
/// ids (numbered per Instance: a device of a second Instance can equal the window's). The flag follows each `acquire` result.
#[test]
fn the_capture_device_kind_is_the_exact_flag() {
    use super::gpu_shared::{note_device, on_window_device};
    note_device(true);
    assert!(on_window_device());
    note_device(false);
    assert!(!on_window_device());
}
