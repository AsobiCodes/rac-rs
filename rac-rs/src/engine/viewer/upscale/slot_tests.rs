//! Tests of the upscaler slot (slot.rs) and its own passes' constants and shader (slot_gpu.rs, slot_gpu.wgsl).

use super::super::{slot_gpu, Prep};
use super::*;

#[test]
fn the_env_value() {
	for off in [None, Some(""), Some("0"), Some("off"), Some("fsr:10"), Some("fsr:150"), Some("fsr:nope"), Some("easu")] {
		assert_eq!(parse(off), None, "{off:?}");
	}
	assert_eq!(parse(Some("fsr")), Some(Setting { kind: Kind::Fsr, ratio: 1.5, sharp: SHARP }));
	assert_eq!(parse(Some("fsr1")), parse(Some("fsr")), "FSR 1 is gone (lane nfsr1): its old name = FSR");
	assert_eq!(parse(Some("fsr:performance")), Some(Setting { kind: Kind::Fsr, ratio: 2.0, sharp: SHARP }));
	assert_eq!(parse(Some("dlss")), Some(Setting { kind: Kind::Dlss, ratio: 1.5, sharp: SHARP }));
	assert_eq!(parse(Some("dlss:native")).map(|s| (s.kind, s.ratio)), Some((Kind::Dlss, 1.0)));
	assert_eq!(parse(Some(" FSR:Performance ")).map(|s| s.ratio), Some(2.0));
	assert_eq!(parse(Some("fsr:balanced")).map(|s| s.ratio), Some(1.7));
	assert_eq!(parse(Some("fsr:ultra")).map(|s| s.ratio), Some(3.0));
	assert_eq!(parse(Some("fsr:50%")).map(|s| s.ratio), Some(2.0));
	assert_eq!(parse(Some("fsr:native:0.5")), Some(Setting { kind: Kind::Fsr, ratio: 1.0, sharp: 0.5 }));
	assert_eq!(parse(Some("fsr::9")).map(|s| (s.ratio, s.sharp)), Some((DEFAULT_RATIO, 2.0)));
	assert_eq!(parse(Some("fsr:quality:x")).map(|s| s.sharp), Some(SHARP));
}

#[test]
fn render_sizes() {
	let s = |r: f32| Setting { kind: Kind::Fsr, ratio: r, sharp: SHARP };
	assert_eq!(s(1.5).render_size((1280, 720), 8192), (853, 480));
	assert_eq!(s(2.0).render_size((1280, 800), 8192), (640, 400));
	assert_eq!(s(3.0).render_size((3840, 2160), 8192), (1280, 720));
	assert_eq!(s(1.0).render_size((1920, 1080), 8192), (1920, 1080));
	assert_eq!(s(1.0).render_size((1920, 1080), 1000), (1000, 1000));
	assert_eq!(s(3.0).render_size((1, 1), 8192), (1, 1));
}

#[test]
fn hud_rects_scale_round_and_stay_inside() {
	let r = [10.0, 20.0, 300.0, 200.0];
	assert_eq!(scale_rect(r, [1.0, 1.0], (1280, 720)), r);
	assert_eq!(scale_rect(r, [1.5, 1.5], (1920, 1080)), [15.0, 30.0, 450.0, 300.0]);
	assert_eq!(scale_rect([0.0, 0.0, 853.0, 480.0], [1280.0 / 853.0, 1.5], (1280, 720)), [0.0, 0.0, 1280.0, 720.0]);
	assert_eq!(scale_rect([800.0, 400.0, 100.0, 100.0], [2.0, 2.0], (1280, 720)), [1280.0, 720.0, 0.0, 0.0]);
	// The slot off (no frame begun): every rectangle as it was.
	assert_eq!(Prep::default().hud_rect(r), r);
}

#[test]
fn slot_constants() {
	assert_eq!(slot_gpu::uniforms((853, 480), (1280, 720)).size, [853.0, 480.0, 1280.0, 720.0]);
	assert_eq!(slot_gpu::uniforms((0, 0), (0, 0)).size, [1.0; 4], "never a zero size in the shader");
	assert_eq!(std::mem::size_of::<slot_gpu::SlotUniforms>(), 16);
}

fn module() -> (naga::Module, naga::valid::ModuleInfo) {
	let m = naga::front::wgsl::parse_str(slot_gpu::SHADER).expect("wgsl parse");
	let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	(m, info)
}

#[test]
fn slot_shader_validates() {
	let (m, _) = module();
	for e in ["vs_full", "fs_scale", "fs_copy"] {
		assert!(m.entry_points.iter().any(|p| p.name == e), "{e}");
	}
}

/// DX12 parity: the shader also goes through naga's HLSL writer (FXC / DXC run on a DX12 run).
#[test]
fn slot_shader_writes_hlsl() {
	let (m, info) = module();
	let (options, pipeline) = (naga::back::hlsl::Options::default(), naga::back::hlsl::PipelineOptions::default());
	let mut out = String::new();
	naga::back::hlsl::Writer::new(&mut out, &options, &pipeline).write(&m, &info, None).expect("HLSL writes");
	assert!(out.contains("fs_scale") && out.contains("fs_copy"));
}
