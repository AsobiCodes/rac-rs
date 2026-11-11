//! Unit tests of the pure parts of fgen (mode, queue plan, swapchain sizes, DXGI present arguments).

use super::queues::{plan, wanted, Family};
use super::{mode_of, Mode};

#[test]
fn mode_of_values() {
	assert_eq!(mode_of(None), Mode::Auto);
	assert_eq!(mode_of(Some("")), Mode::Auto);
	assert_eq!(mode_of(Some("0")), Mode::Off);
	assert_eq!(mode_of(Some(" OFF ")), Mode::Off);
	assert_eq!(mode_of(Some("null")), Mode::Null);
	assert_eq!(mode_of(Some("Null")), Mode::Null);
	assert_eq!(mode_of(Some("fsr")), Mode::Fsr);
	assert_eq!(mode_of(Some("none")), Mode::Auto);
	assert_eq!((mode_of(Some("dlss2")), mode_of(Some(" DLSS3 ")), mode_of(Some("dlss4"))), (Mode::Dlss(1), Mode::Dlss(2), Mode::Dlss(3)));
	assert_eq!((mode_of(Some("dlss5")), mode_of(Some("dlss"))), (Mode::Auto, Mode::Auto), "no 5x, no bare dlss");
}

#[test]
fn wanted_only_with_fsr_or_the_test_switch() {
	// B1: framegen off / RAC_FG unset = today's device, whatever DLLs sit next to the exe.
	for (mode, forced, want) in [(Mode::Auto, false, false), (Mode::Auto, true, true), (Mode::Off, false, false), (Mode::Off, true, false), (Mode::Null, false, false), (Mode::Null, true, true), (Mode::Fsr, false, true), (Mode::Fsr, true, true)] {
		assert_eq!(wanted(mode, forced), want, "{mode:?} forced {forced}");
	}
	assert!(!wanted(super::mode_with(None, Some("off")), false), "framegen=off in the options file");
	assert!(!wanted(super::mode_with(None, None), false), "no framegen key");
	assert!(wanted(super::mode_with(None, Some("fsr")), false), "framegen=fsr");
}

fn fam(graphics: bool, compute: bool, present: bool, count: u32) -> Family {
	Family { graphics, compute, present, count }
}

/// No role is ever the game queue (family `main`, queue 0).
fn no_game_queue(p: &super::queues::Plan, main: u32) {
	for r in [p.roles.compute, p.roles.present, p.roles.acquire] {
		assert_ne!(r, (main, 0), "{p:?}");
	}
}

#[test]
fn plan_with_a_compute_only_family_nvidia_like() {
	let p = plan(&[fam(true, true, true, 16), fam(false, false, false, 2), fam(false, true, true, 8)], 0).unwrap();
	assert_eq!((p.roles.compute, p.roles.present, p.roles.acquire), ((2, 0), (0, 1), (2, 1)));
	assert_eq!(p.counts, vec![(0, 2), (2, 2)]);
	no_game_queue(&p, 0);
}

#[test]
fn plan_without_a_compute_family() {
	let p = plan(&[fam(true, true, true, 16)], 0).unwrap();
	assert_eq!((p.roles.compute, p.roles.present, p.roles.acquire), ((0, 1), (0, 2), (0, 3)));
	assert_eq!(p.counts, vec![(0, 4)]);
	let p = plan(&[fam(true, true, true, 2)], 0).unwrap();
	assert_eq!((p.roles.compute, p.roles.present, p.roles.acquire), ((0, 1), (0, 1), (0, 1)), "roles share the second queue");
	no_game_queue(&p, 0);
}

#[test]
fn plan_on_one_graphics_queue_presents_on_a_compute_family_amd_like() {
	// AMD on Windows / RADV (Steam Deck): one graphics queue, compute queues that present.
	let p = plan(&[fam(true, true, true, 1), fam(false, true, true, 2), fam(false, false, false, 2)], 0).unwrap();
	assert_eq!((p.roles.compute, p.roles.present, p.roles.acquire), ((1, 0), (1, 1), (1, 1)));
	assert_eq!(p.counts, vec![(0, 1), (1, 2)]);
	no_game_queue(&p, 0);
	let p = plan(&[fam(true, true, true, 1), fam(false, true, true, 4)], 0).unwrap();
	assert_eq!((p.roles.compute, p.roles.present, p.roles.acquire), ((1, 0), (1, 1), (1, 2)));
	// A second graphics queue that cannot present: present goes to the compute family.
	let p = plan(&[fam(true, true, false, 2), fam(false, true, true, 2)], 0).unwrap();
	assert_eq!(p.roles.present, (1, 1));
}

#[test]
fn plan_refuses_when_only_the_game_queue_could_present() {
	assert!(plan(&[fam(true, true, true, 1)], 0).is_none(), "one queue in all");
	assert!(plan(&[fam(true, true, true, 1), fam(false, true, false, 2)], 0).is_none(), "the compute family cannot present");
	assert!(plan(&[fam(true, true, true, 1), fam(false, false, true, 1)], 0).is_none(), "no async-compute queue besides the game queue");
	assert!(plan(&[fam(false, true, true, 4)], 0).is_none());
	assert!(plan(&[fam(true, true, true, 4)], 3).is_none());
}

#[cfg(windows)]
#[test]
fn one_frame_id_per_presented_frame_and_no_stale_prepare() {
	use super::ffx_fg::FgFrame;
	use super::present::pick;
	let none = || crate::viewer::upscale::ffx_abi::Resource::none();
	let f = |id, prepared| Some(FgFrame { id, prepared, hudless: none(), dx: None });
	let a = pick(f(7, true), 7);
	assert_eq!((a.id, a.prepared), (7, true), "the frame's own inputs");
	let b = pick(None, 8);
	assert_eq!((b.id, b.prepared), (8, false), "no inputs: no generation, still its own id");
	let c = pick(f(8, true), 9);
	assert_eq!((c.id, c.prepared), (9, false), "inputs of another id never carry prepared over");
	// Ids grow by exactly one per presented frame: what take_pending hands out over a run of frames.
	let ids: Vec<u64> = (1..=5).map(|id| pick(if id % 2 == 0 { f(id, true) } else { None }, id).id).collect();
	assert_eq!(ids, vec![1, 2, 3, 4, 5]);
}

#[cfg(windows)]
#[test]
fn vk_image_count_and_extent() {
	use super::swap_vk::{extent_of, image_count};
	assert_eq!(image_count(2, 2, 8), 3);
	assert_eq!(image_count(1, 3, 8), 3);
	assert_eq!(image_count(3, 2, 3), 3);
	assert_eq!(image_count(5, 2, 0), 6);
	assert_eq!(extent_of((1280, 720), (1920, 1080), (1, 1), (4096, 4096)), (1280, 720));
	assert_eq!(extent_of((u32::MAX, u32::MAX), (1920, 1080), (1, 1), (1600, 4096)), (1600, 1080));
	assert_eq!(extent_of((u32::MAX, u32::MAX), (0, 0), (1, 1), (4096, 4096)), (1, 1));
}

#[cfg(windows)]
#[test]
fn dx12_present_args_follow_wgpu() {
	use super::swap_dx12::present_args;
	use windows::Win32::Graphics::Dxgi::{DXGI_PRESENT, DXGI_PRESENT_ALLOW_TEARING};
	assert_eq!(present_args(wgpu::PresentMode::Immediate, true), (0, DXGI_PRESENT_ALLOW_TEARING));
	assert_eq!(present_args(wgpu::PresentMode::Immediate, false), (0, DXGI_PRESENT::default()));
	assert_eq!(present_args(wgpu::PresentMode::Mailbox, true), (0, DXGI_PRESENT::default()));
	assert_eq!(present_args(wgpu::PresentMode::Fifo, true), (1, DXGI_PRESENT::default()));
}

/// [S2] The FidelityFX frame generation structs match the SDK 2.x headers (x64 sizes and offsets).
#[cfg(windows)]
#[test]
fn ffx_fg_abi_layouts() {
	use super::ffx_fg_abi as a;
	use std::mem::{offset_of, size_of};
	assert_eq!(size_of::<a::CreateFg>(), 40);
	assert_eq!(size_of::<a::HeaderU32>(), 24);
	assert_eq!(size_of::<a::CallbackPresent>(), 192);
	assert_eq!(offset_of!(a::CallbackPresent, is_generated_frame), 176);
	assert_eq!(offset_of!(a::CallbackPresent, frame_id), 184);
	assert_eq!(size_of::<a::ConfigureFg>(), 144);
	assert_eq!(offset_of!(a::ConfigureFg, enabled), 56);
	assert_eq!(offset_of!(a::ConfigureFg, hudless_color), 64);
	assert_eq!(offset_of!(a::ConfigureFg, flags), 112);
	assert_eq!(offset_of!(a::ConfigureFg, only_present_generated), 116);
	assert_eq!(offset_of!(a::ConfigureFg, generation_rect), 120);
	assert_eq!(offset_of!(a::ConfigureFg, frame_id), 136);
	assert_eq!(size_of::<a::PrepareV2>(), 232);
	assert_eq!(offset_of!(a::PrepareV2, command_list), 32);
	assert_eq!(offset_of!(a::PrepareV2, frame_time_delta), 64);
	assert_eq!(offset_of!(a::PrepareV2, reset), 68);
	assert_eq!(offset_of!(a::PrepareV2, depth), 88);
	assert_eq!(offset_of!(a::PrepareV2, motion_vectors), 136);
	assert_eq!(offset_of!(a::PrepareV2, camera_position), 184);
	assert_eq!(size_of::<a::QueueInfo>(), 24);
	assert_eq!(size_of::<a::CreateSwapchainVk>(), 248);
	assert_eq!(offset_of!(a::CreateSwapchainVk, create_info), 48);
	assert_eq!(offset_of!(a::CreateSwapchainVk, game_queue), 152);
	assert_eq!(size_of::<a::SwapchainFunctionsVk>(), 72);
	assert_eq!(size_of::<a::CreateSwapchainForHwndDx12>(), 64);
}

/// S4: a set env wins over the options file; an empty env is unset; the file's `fsr` turns frame generation on.
#[test]
fn the_env_overrides_the_options_file() {
	use super::mode_with;
	assert_eq!(mode_with(None, None), Mode::Auto);
	assert_eq!(mode_with(None, Some("fsr")), Mode::Fsr);
	assert_eq!(mode_with(Some(""), Some("fsr")), Mode::Fsr);
	assert_eq!(mode_with(Some("0"), Some("fsr")), Mode::Off);
	assert_eq!(mode_with(Some("null"), None), Mode::Null);
	assert_eq!(mode_with(Some("fsr"), Some("off")), Mode::Fsr);
	assert_eq!(mode_with(None, Some("off")), Mode::Auto);
}

/// S4: the options file key round trip, unknown values = Off, withdrawn values read Off, the restart flag.
#[test]
fn the_framegen_choice_parses_saves_and_withdraws() {
	use super::choice::*;
	use crate::pcopts::PcOptions;
	reset_for_test();
	assert_eq!(KEY, "framegen");
	assert_eq!(KINDS, ["off", "fsr", "dlss2", "dlss3", "dlss4"]);
	assert_eq!(DLSS_VALUES, [2, 3, 4]);
	// Every DLSS value maps to the mode of its generated frames (value - 1).
	for v in DLSS_VALUES {
		assert_eq!(super::mode_with(None, Some(KINDS[v as usize])), super::Mode::Dlss(v as u32 - 1));
	}
	assert_eq!((index(None), index(Some(" FSR ")), index(Some("dlss9")), index(Some("off"))), (0, 1, 0, 0));
	let mut o = PcOptions::new();
	o.extra_set(KEY, Some("fsr".into()));
	init(&o);
	set_runs(FSR_VALUE, true); // init withdrew it: no DLLs next to the test exe
	assert_eq!((chosen(), started(), restart_pending(), available()), (1, 1, false, true));
	assert!(set(&mut o, 0), "a change writes the key");
	assert_eq!(o.extra_get(KEY), Some("off"));
	assert!(!set(&mut o, 0), "the same value writes nothing");
	assert!(restart_pending(), "off differs from the fsr this run started with");
	assert!(set(&mut o, 7) && o.extra_get(KEY) == Some("dlss4"), "clamped to the last value");
	assert!(set(&mut o, 1));
	// Saved and loaded again (the options file text round trip).
	let again = PcOptions::parse(&o.to_text());
	assert_eq!(index(again.extra_get(KEY)), 1);
	set_runs(FSR_VALUE, false);
	assert_eq!((offered(0), offered(FSR_VALUE), offered(2), offered(5), chosen(), available()), (true, false, false, false, 0, false), "no DLSS DLLs next to the test exe");
	for v in DLSS_VALUES {
		set_runs(v, true);
	}
	assert!(available() && offered(4));
	refuse_dlss(1); // a card that makes one generated frame (RTX 40): 2x only
	assert_eq!((offered(2), offered(3), offered(4)), (true, false, false));
	refuse_dlss(0); // refused at start: none
	assert!(!offered(2) && !available());
	assert!(!crate::ui::live::offered(KEY, FSR_VALUE), "the modern row skips it too");
	reset_for_test();
}

/// S3: Streamline's repr(C) layouts (x64 sizes and offsets from the SDK 2.14.16 headers).
#[cfg(windows)]
#[test]
fn sl_abi_layouts() {
	use super::sl_abi::*;
	use std::mem::{offset_of, size_of};
	assert_eq!(size_of::<Header>(), 32);
	assert_eq!((size_of::<Preferences>(), offset_of!(Preferences, flags), offset_of!(Preferences, render_api)), (144, 88, 136));
	assert_eq!((size_of::<Resource>(), offset_of!(Resource, state), offset_of!(Resource, gpu_virtual_address), offset_of!(Resource, internal_flags)), (112, 64, 88, 104));
	assert_eq!((size_of::<ResourceTag>(), offset_of!(ResourceTag, extent)), (64, 48));
	assert_eq!((size_of::<Viewport>(), size_of::<AdapterInfo>()), (40, 56));
	assert_eq!(size_of::<Constants>(), 456);
	assert_eq!((offset_of!(Constants, jitter_offset), offset_of!(Constants, camera_pos), offset_of!(Constants, camera_near), offset_of!(Constants, depth_inverted)), (352, 376, 424, 444));
	assert_eq!(offset_of!(Constants, min_relative_linear_depth_object_separation), 452);
	assert_eq!((size_of::<DlssgOptions>(), offset_of!(DlssgOptions, on_error_callback), offset_of!(DlssgOptions, queue_parallelism_mode), offset_of!(DlssgOptions, dynamic_target_frame_rate)), (120, 96, 108, 116));
	assert_eq!((size_of::<DlssgState>(), offset_of!(DlssgState, num_frames_to_generate_max), offset_of!(DlssgState, inputs_processing_completion_fence), offset_of!(DlssgState, b_is_dynamic_mfg_supported)), (88, 52, 64, 80));
	assert_eq!(size_of::<ReflexOptions>(), 48);
	assert_eq!(SDK_VERSION >> 16, (2 << 32) | (14 << 16) | 16);
}

/// S3: generated frames for a wanted multiplier and the card's maximum (never above 4x nor the card's maximum, at least 2x).
#[cfg(windows)]
#[test]
fn dlss_offer_frames() {
	use super::sl::offer_frames;
	assert_eq!((offer_frames(3, 1), offer_frames(2, 3), offer_frames(3, 3), offer_frames(0, 3), offer_frames(5, 5), offer_frames(1, 0)), (1, 2, 3, 1, 3, 1));
}

/// S3: Streamline's camera maths: inverse * m = identity, a still camera = identity clipToPrevClip, a moved one is not; a cut
/// resets it.
#[cfg(windows)]
#[test]
fn sl_constants_maths() {
	use super::sl_frame::sl_math::*;
	let close = |a: &super::sl_abi::Mat, b: &super::sl_abi::Mat| a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| (x - y).abs() < 1e-4);
	let p = perspective(1.0, 16.0 / 9.0, 0.1, 1000.0);
	assert!(close(&mul(&inverse(&p), &p), &IDENTITY));
	let v = view([3.0, -2.0, 5.0], [0.6, 0.8, 0.0], [0.0, 0.0, 1.0]);
	assert!(close(&mul(&v, &inverse(&v)), &IDENTITY));
	// The eye maps to the view origin; a point straight ahead to +z (left-handed, z forward), depth 0..1 between near and far.
	let mulv = |p: [f32; 4], m: &super::sl_abi::Mat| -> [f32; 4] { std::array::from_fn(|j| (0..4).map(|k| p[k] * m[k][j]).sum()) };
	let o = mulv([3.0, -2.0, 5.0, 1.0], &v);
	assert!(o.iter().take(3).all(|x| x.abs() < 1e-5));
	let a = mulv([3.6, -1.2, 5.0, 1.0], &v);
	assert!((a[2] - 1.0).abs() < 1e-5 && a[0].abs() < 1e-5);
	let near = mulv([0.0, 0.0, 0.1, 1.0], &p);
	let far = mulv([0.0, 0.0, 1000.0, 1.0], &p);
	assert!((near[2] / near[3]).abs() < 1e-5 && (far[2] / far[3] - 1.0).abs() < 1e-4);
	let cam = |eye: [f32; 3], reset: bool| Cam { eye, fwd: [0.6, 0.8, 0.0], up: [0.0, 0.0, 1.0], fov_y: 1.0, near: 0.1, far: 1000.0, aspect: 16.0 / 9.0, jitter: [0.25, -0.25], reset };
	let (c0, v0) = constants(&cam([3.0, -2.0, 5.0], false), None);
	assert!(close(&c0.clip_to_prev_clip, &IDENTITY), "no previous view");
	let (c1, _) = constants(&cam([3.0, -2.0, 5.0], false), Some(v0));
	assert!(close(&c1.clip_to_prev_clip, &IDENTITY), "a still camera");
	let (c2, _) = constants(&cam([4.0, -2.0, 5.0], false), Some(v0));
	assert!(!close(&c2.clip_to_prev_clip, &IDENTITY) && close(&mul(&c2.clip_to_prev_clip, &c2.prev_clip_to_clip), &IDENTITY));
	let (c3, _) = constants(&cam([4.0, -2.0, 5.0], true), Some(v0));
	assert!(close(&c3.clip_to_prev_clip, &IDENTITY) && c3.reset == 1, "a cut");
	assert_eq!((c2.mvec_scale, c2.jitter_offset, c2.depth_inverted, c2.camera_motion_included), ([1.0, 1.0], [0.25, -0.25], 0, 1));
	// A slow zoom (still camera, fov 1.0 -> 1.02): clipToPrevClip maps a clip point through the PREVIOUS projection.
	let zoom = Cam { fov_y: 1.02, ..cam([3.0, -2.0, 5.0], false) };
	let (c4, _) = constants(&zoom, Some(v0));
	assert_eq!(c4.reset, 0, "a slow zoom is no cut");
	let want = mul(&inverse(&perspective(1.02, 16.0 / 9.0, 0.1, 1000.0)), &perspective(1.0, 16.0 / 9.0, 0.1, 1000.0));
	assert!(close(&c4.clip_to_prev_clip, &want) && !close(&c4.clip_to_prev_clip, &IDENTITY), "previous projection used");
	let (c5, p5) = constants(&Cam { fov_y: 1.0 + FOV_CUT * 2.0, ..cam([3.0, -2.0, 5.0], false) }, Some(v0));
	assert!(close(&c5.clip_to_prev_clip, &IDENTITY) && c5.reset == 1, "a fov jump is a cut");
	assert_eq!(p5.fov_y, 1.0 + FOV_CUT * 2.0);
	// The up hint: a rolled camera (up = +x for a camera looking along +y) gives a camera_up of +x, not world Z.
	let rolled = Cam { fwd: [0.0, 1.0, 0.0], up: [1.0, 0.0, 0.0], ..cam([0.0; 3], false) };
	let (c6, _) = constants(&rolled, None);
	assert!((c6.camera_up[0] - 1.0).abs() < 1e-5 && c6.camera_up[2].abs() < 1e-5, "up {:?}", c6.camera_up);
}

/// S3 (agent 3): the card memory. Only card answers are remembered (not driver / OS / HAGS); the file names the card; the DLSS
/// values fit the remembered maximum; a run-time row note replaces the static one and goes again.
#[cfg(windows)]
#[test]
fn dlss_card_memory() {
	use super::choice::{dlss_fits, format_card, parse_card, CardMemory};
	use super::sl::{card_refusal, remembered_max};
	assert!(card_refusal(6) && card_refusal(7) && card_refusal(32));
	assert!(!card_refusal(0) && !card_refusal(2) && !card_refusal(4) && !card_refusal(15));
	let m = CardMemory { name: "NVIDIA GeForce RTX 3060".into(), max: 0, driver: "32.0.15.8157".into() };
	assert_eq!(format_card(&m), "NVIDIA GeForce RTX 3060\t0\t32.0.15.8157\n");
	assert_eq!(parse_card(&format_card(&m)).as_ref(), Some(&m), "round trip");
	assert!(m.is("NVIDIA GeForce RTX 3060", "32.0.15.8157"));
	assert!(!m.is("NVIDIA GeForce RTX 5070 Ti", "32.0.15.8157"), "another card");
	assert!(!m.is("NVIDIA GeForce RTX 3060", "32.0.15.9000"), "another driver: the memory goes (a driver update may lift it)");
	// The first file format (no driver) reads with an empty driver: it matches no real driver and is forgotten.
	assert_eq!(parse_card("X\t9"), Some(CardMemory { name: "X".into(), max: 3, driver: String::new() }), "clamped to 4x");
	assert_eq!((parse_card("garbage"), parse_card("X\tz"), parse_card("\t1\td")), (None, None, None));
	// slDLSSGGetState: only a successful answer is remembered (a failure says nothing about the card).
	assert_eq!((remembered_max(0, 1), remembered_max(0, 5), remembered_max(0, 0)), (Some(1), Some(3), Some(1)));
	assert_eq!((remembered_max(2, 3), remembered_max(15, 1)), (None, None));
	assert_eq!([2, 3, 4].map(|v| dlss_fits(v, 0)), [false, false, false]);
	assert_eq!([2, 3, 4].map(|v| dlss_fits(v, 1)), [true, false, false]);
	assert_eq!([2, 3, 4].map(|v| dlss_fits(v, 3)), [true, true, true]);
	crate::ui::live::set_note("fgen test row", Some(&super::choice::NO_CARD_NOTE));
	assert_eq!(crate::ui::live::note("fgen test row").map(|t| t[0]), Some("Not supported on this graphics card"));
	crate::ui::live::set_note("fgen test row", None);
	assert!(crate::ui::live::note("fgen test row").is_none());
}

/// [S3] sl_vk.rs: Streamline's Vulkan resource fields (raw VkFormat / VkImageUsageFlags / VkImageLayout values of the spec).
#[cfg(windows)]
#[test]
fn sl_vk_resource_fields() {
	use super::sl_vk::{vk_format, vk_layout, vk_usage};
	use wgpu::TextureFormat as F;
	use wgpu::TextureUsages as U;
	assert_eq!(vk_format(F::Rgba8Unorm), Some(37));
	assert_eq!(vk_format(F::Bgra8Unorm), Some(44));
	assert_eq!(vk_format(F::Rgb10a2Unorm), Some(64));
	assert_eq!(vk_format(F::Rg16Float), Some(83));
	assert_eq!(vk_format(F::Rgba16Float), Some(97));
	assert_eq!(vk_format(F::Depth32Float), Some(126));
	assert_eq!(vk_format(F::R8Uint), None);
	assert_eq!(vk_usage(U::TEXTURE_BINDING | U::RENDER_ATTACHMENT, false), 0x4 | 0x10);
	assert_eq!(vk_usage(U::TEXTURE_BINDING | U::RENDER_ATTACHMENT, true), 0x4 | 0x20);
	assert_eq!(vk_usage(U::COPY_SRC | U::COPY_DST | U::STORAGE_BINDING, false), 0x1 | 0x2 | 0x8);
	assert_eq!(vk_usage(U::empty(), false), 0);
	assert_eq!(vk_layout(false), 5);
	assert_eq!(vk_layout(true), 4);
}

/// [S3] review fix 1/6: the Vulkan instance goes through Streamline only for an NVIDIA card, and a remembered refusal only stops
/// it for the same card on the same driver (sl_card.rs `vk_gate`); DXGI's driver version text.
#[cfg(windows)]
#[test]
fn dlss_vulkan_card_gate() {
	use super::choice::CardMemory;
	use super::sl_card::{driver_string, vk_gate, DxCard, NVIDIA};
	let nv = DxCard { name: "NVIDIA GeForce RTX 5090".into(), vendor: NVIDIA, driver: "32.0.15.8157".into() };
	let amd = DxCard { name: "AMD Radeon RX 7900 XTX".into(), vendor: 0x1002, driver: "31.0.24027.1012".into() };
	let intel = DxCard { name: "Intel(R) Arc(TM) A770 Graphics".into(), vendor: 0x8086, driver: "32.0.101.6078".into() };
	assert!(vk_gate(Some(&nv), None).is_ok());
	assert!(vk_gate(Some(&amd), None).is_err() && vk_gate(Some(&intel), None).is_err(), "not NVIDIA: today's instance");
	assert!(vk_gate(None, None).is_err(), "no adapter to check");
	let refused = CardMemory { name: nv.name.clone(), max: 0, driver: nv.driver.clone() };
	assert!(vk_gate(Some(&nv), Some(&refused)).is_err(), "refused on this driver");
	assert!(vk_gate(Some(&nv), Some(&CardMemory { driver: "32.0.15.9000".into(), ..refused.clone() })).is_ok(), "new driver: asked again");
	assert!(vk_gate(Some(&nv), Some(&CardMemory { name: "NVIDIA GeForce RTX 3060".into(), ..refused.clone() })).is_ok(), "another card's memory is ignored");
	assert!(vk_gate(Some(&nv), Some(&CardMemory { max: 1, ..refused })).is_ok(), "a 2x card");
	assert_eq!(driver_string(0x0020_0000_0F00_1FDD), "32.0.3840.8157");
}

/// [S3] review fix 3: what the row shows for a DLSS multiplier above the card's maximum is the one that runs (sl.rs clamps it),
/// for the chosen and the started value alike (no false "- restart").
#[test]
fn dlss_shown_value_follows_the_clamp() {
	use super::choice::shown;
	let rtx40 = |v: i32| v <= 2; // Off, FSR, DLSS 2x
	assert_eq!([0, 1, 2, 3, 4].map(|v| shown(v, rtx40)), [0, 1, 2, 2, 2]);
	let none = |v: i32| v <= 1; // no DLSS value offered
	assert_eq!([2, 3, 4].map(|v| shown(v, none)), [0, 0, 0]);
	let no_fsr = |v: i32| v != 1 && v <= 3; // FSR withdrawn, DLSS up to 3x
	assert_eq!([1, 4].map(|v| shown(v, no_fsr)), [0, 3], "FSR never maps; 4x shows 3x");
}

#[test]
fn the_missing_files_note_yields_to_the_card_note() {
	use super::choice::{note_for, MISSING_NOTE, NO_CARD_NOTE};
	assert!(note_for(true, true).is_some_and(|n| *n == NO_CARD_NOTE));
	assert!(note_for(false, true).is_some_and(|n| *n == MISSING_NOTE));
	assert!(note_for(false, false).is_none());
}
