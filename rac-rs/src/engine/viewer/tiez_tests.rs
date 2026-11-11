//! Tests of tiez.rs (lane tpre): the pre-pass module variants validate and keep the main text as their prefix, the pipeline states
//! the exactness argument rests on, the alpha split, the gate.

use naga::valid::Capabilities as C;

/// The main bindless level texts the pre-pass is built from: (label, text, naga capabilities): plain, fire without and with the
/// quad sharing (flitquad.rs `fire_text`; the subgroup twin needs the subgroup capabilities), both forced-early-depth choices,
/// Vulkan and DX12 text (texfilter.rs `frag_w_for`).
fn mains() -> Vec<(String, String, C)> {
	let mut v = Vec::new();
	for early in [false, true] {
		for dx in [false, true] {
			let level = crate::viewer::texfilter::frag_w_for(crate::viewer::metalfx::with_override(crate::viewer::volfog::soft::level_source(crate::viewer::SHADER, early)), dx);
			let bl = crate::viewer::bindless::shader_source(&level);
			let tag = format!("early={early} dx12={dx}");
			let fire = |q: bool| crate::viewer::flitquad::fire_text(&bl, q).unwrap_or_else(|| panic!("fire edits {tag}")).0;
			v.push((format!("plain {tag}"), bl.clone(), C::all().difference(C::SUBGROUP).difference(C::SUBGROUP_BARRIER).difference(C::SUBGROUP_VERTEX_STAGE)));
			v.push((format!("fire {tag}"), fire(false), C::all().difference(C::SUBGROUP).difference(C::SUBGROUP_BARRIER).difference(C::SUBGROUP_VERTEX_STAGE)));
			v.push((format!("fire quads {tag}"), fire(true), C::all().difference(C::SUBGROUP_BARRIER).difference(C::SUBGROUP_VERTEX_STAGE)));
		}
	}
	v
}

#[test]
fn every_prepass_variant_validates_and_keeps_the_main_text() {
	for (label, main, caps) in mains() {
		let t = super::text(main.clone());
		// Review finding 2: the pre-pass `vs` is the main pipelines' function text (the main text unchanged, entries after it).
		assert!(t.starts_with(&main) && &t[main.len()..] == super::WGSL, "{label}");
		let m = naga::front::wgsl::parse_str(&t).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&t)));
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), caps).validate(&m).unwrap_or_else(|e| panic!("{label}: {e:?}"));
		for e in ["vs", "fs_tiez_mark", "fs_tiez_z", "vs_tiez_full", "fs_tiez_none"] {
			assert!(m.entry_points.iter().any(|p| p.name == e), "{label}: {e}");
		}
	}
}

#[test]
fn the_main_level_text_carries_nothing_of_the_lane() {
	// Review finding 3: the main modules are built from the pre-lane text (setup.rs, bindless.rs); the entries live only in `text`.
	for (label, main, _) in mains() {
		assert!(!main.contains("fs_tiez_") && !main.contains("vs_tiez_full"), "{label}");
	}
}

#[test]
fn pipeline_states_hold_the_exactness_argument() {
	use wgpu::{CompareFunction as Cf, StencilOperation as S};
	let le = Cf::LessEqual;
	let [mark, z, restore] = super::states(le, super::Z_BIAS);
	// Mark: the tie test, no Z write, every passing sample stencilled (Always / Replace, all bits), unbiased.
	assert_eq!((mark.vs, mark.fs, mark.depth_write, mark.depth_compare), ("vs", "fs_tiez_mark", false, le));
	assert_eq!((mark.stencil_compare, mark.stencil_pass, mark.stencil_write_mask, mark.bias, mark.buffers), (Cf::Always, S::Replace, 0xff, 0, true));
	// Z: the tie test, Z write only where the stencil equals the reference (STENCIL_CLEAR, unmarked), stencil never written,
	// biased FARTHER (positive constant) so the nearest opaque fragment is never rejected by its own pre-pass Z.
	assert_eq!((z.vs, z.fs, z.depth_write, z.depth_compare), ("vs", "fs_tiez_z", true, le));
	assert_eq!((z.stencil_compare, z.stencil_pass, z.stencil_write_mask, z.buffers), (Cf::Equal, S::Keep, 0, true));
	assert!(z.bias > 0);
	assert_eq!(super::Z_BIAS, 16);
	// Restore: full-screen triangle, no depth test or write, stencil Replace on every sample.
	assert_eq!((restore.vs, restore.fs, restore.depth_write, restore.depth_compare), ("vs_tiez_full", "fs_tiez_none", false, Cf::Always));
	assert_eq!((restore.stencil_compare, restore.stencil_pass, restore.stencil_write_mask, restore.bias, restore.buffers), (Cf::Always, S::Replace, 0xff, 0, false));
	// The references: mark = one above the shadow-volume start value, Z pass and restore at it.
	assert_eq!(super::MARK, super::super::shadowvol::STENCIL_CLEAR + 1);
	assert_eq!(super::MARK, 129);
	// The tie test the states are built with is the main tie pipelines' (metalpipe.rs `ties`, gs_state.rs `ztest`).
	assert_eq!(crate::gs_state::ztest(), le);
}

#[test]
fn mark_and_z_split_fs_tie_at_its_z_write_and_full_alpha() {
	// fs_tie draws alpha >= 0x40 with Z; the mark takes 0x40..0x7f, the Z pass only 0x80 (world_alpha 1.0).
	assert!(super::WGSL.contains("if (ra * 128.0 < 64.0 || ra >= 1.0) { discard; }"));
	assert!(super::WGSL.contains("if (world_alpha(world_texel(i)) < 1.0) { discard; }"));
	assert!(crate::viewer::SHADER.contains("if (ra * 128.0 < 64.0) { discard; }\n\treturn vec4<f32>(tie_rgb(i, c), ra);"), "fs_tie's Z-writing split moved: update tiez.wgsl");
}

#[test]
fn auto_takes_fire_frames_at_1440p_native_only() {
	assert!(super::auto(true, 2560 * 1440));
	assert!(!super::auto(true, 1707 * 960));
	assert!(!super::auto(false, 2560 * 1440));
	assert!(!super::auto(false, 3840 * 2160));
}

#[test]
fn rac_tiez_values_other_than_0_and_1_mean_auto() {
	assert_eq!(super::parse_mode(Some("0")), 0);
	assert_eq!(super::parse_mode(Some("1")), 1);
	for v in [None, Some("on"), Some("true"), Some(" 1"), Some("")] {
		assert_eq!(super::parse_mode(v), 2, "{v:?}");
	}
}

#[test]
fn only_nvidia_vulkan_qualifies() {
	assert_eq!(super::NVIDIA, 0x10de);
}
