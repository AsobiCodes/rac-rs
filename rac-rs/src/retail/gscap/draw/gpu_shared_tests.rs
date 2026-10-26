//! Unit tests of the GS capture's device choice (`gpu_shared.rs`): the default per vendor and backend, RAC_GSCAP_GPU's values, and the
//! window's slot (publish, take, no deadlock without a window).
use super::gpu_shared::{default_on, resolve, uses_window, Board};
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
    let h = std::thread::spawn(|| {
        let t = Instant::now();
        let _g = super::gpu_shared::submit_lock();
        t.elapsed()
    });
    std::thread::sleep(Duration::from_millis(80));
    drop(g);
    assert!(h.join().unwrap() >= Duration::from_millis(60));
    let _ = std::thread::spawn(|| {
        let _g = super::gpu_shared::submit_lock();
        panic!("poison the lock");
    })
    .join();
    drop(super::gpu_shared::submit_lock());
}
