use super::*;

/// The rows' ratios map to NGX's modes (UltraQuality never: NGX offers no size for it); a percentage gets the nearest mode.
#[test]
fn ratios_map_to_ngx_modes() {
	let q: Vec<u32> = [1.0, 1.5, 1.7, 2.0, 3.0].iter().map(|&r| ngx_quality(r)).collect();
	assert_eq!(q, vec![abi::QUALITY_DLAA, abi::QUALITY_MAX_QUALITY, abi::QUALITY_BALANCED, abi::QUALITY_MAX_PERF, abi::QUALITY_ULTRA_PERFORMANCE]);
	assert_eq!(ngx_quality(100.0 / 60.0), abi::QUALITY_BALANCED);
	assert_eq!(ngx_quality(1.1), abi::QUALITY_DLAA);
	assert!(super::super::slot::QUALITY.iter().all(|&(_, r)| ngx_quality(r) != abi::QUALITY_ULTRA_QUALITY));
}

/// Preset M / L = DLSS 4.5 in every mode (the create desc asks for it), the label follows NGX's report.
#[test]
fn presets_and_labels() {
	for q in [abi::QUALITY_DLAA, abi::QUALITY_MAX_QUALITY, abi::QUALITY_BALANCED, abi::QUALITY_MAX_PERF] {
		assert_eq!(create_desc(((1707, 960), (2560, 1440)), q).preset, abi::PRESET_M);
	}
	let c = create_desc(((853, 480), (2560, 1440)), abi::QUALITY_ULTRA_PERFORMANCE);
	assert_eq!((c.preset, c.render_w, c.out_h, c.flags), (abi::PRESET_L, 853, 1440, abi::FLAG_MV_LOW_RES | abi::FLAG_AUTO_EXPOSURE));
	assert_eq!(label_of('M' as u32), "DLSS 4.5");
	assert_eq!(label_of('L' as u32), "DLSS 4.5");
	assert_eq!(label_of('K' as u32), "DLSS 4");
	assert_eq!(label_of(0), "DLSS");
}

/// The evaluate: jitter in render pixels with the sign switch, UV vectors times the render size, reset flag.
#[test]
fn eval_values() {
	let e = eval_desc((1280, 720), [0.25, -0.125], 1.0, true, 16.0);
	assert_eq!((e.jitter_x, e.jitter_y, e.mv_scale_x, e.mv_scale_y, e.reset, e.render_w), (0.25, -0.125, 1280.0, 720.0, 1, 1280));
	let e = eval_desc((1280, 720), [0.25, -0.125], jitter_sign(Some("-1")), false, 16.0);
	assert_eq!((e.jitter_x, e.jitter_y, e.reset), (-0.25, 0.125, 0));
	assert_eq!(jitter_sign(None), 1.0);
	assert!(switched_off(Some("0")) && !switched_off(Some("1")) && !switched_off(None));
}

/// Every format the slot hands to DLSS has a VkFormat (Bgra8 / Rgba8 colour, R32 depth copy, Rg16 vectors).
#[test]
fn slot_formats_have_vk_formats() {
	use wgpu::TextureFormat as F;
	for f in [F::Bgra8Unorm, F::Rgba8Unorm, F::R32Float, F::Rg16Float, super::super::slot_gpu::MID_FORMAT] {
		assert_ne!(vk_format(f), 0, "{f:?}");
	}
	assert_eq!(vk_format(F::Depth32Float), 0);
}

/// The fixed menu texts are set once per quality-row label generation (review R21 m5), again after a change or another text.
#[test]
fn fixed_labels_repeat_only_after_a_change() {
	use super::super::dlss_frame::repeat;
	let last = std::sync::atomic::AtomicU64::new(0);
	assert!(!repeat(&last, 3, 1), "first time: set");
	assert!(repeat(&last, 3, 1), "same text, same generation: skipped");
	assert!(!repeat(&last, 4, 1), "the quality row was relabelled: set again");
	assert!(!repeat(&last, 4, 2), "another fixed text: set");
	last.store(0, std::sync::atomic::Ordering::Relaxed); // a reported model
	assert!(!repeat(&last, 4, 2));
	assert!(!repeat(&last, 0, 1), "generation 0 never matches the reset value");
}
