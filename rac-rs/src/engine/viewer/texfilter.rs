//! Smooth mip transitions with Lighting Enhanced. Split view: pcext::texfilter::enabled calls splitview::fx at frame_uniforms.
//! The billboard sampler's Linear-mip twin keeps ClampToEdge on both axes. Classic still requests integer levels, so only
//! Smooth mixes neighbours. HUD, metal and other users of the original Nearest-mip clamp sampler remain unchanged.

pub(super) fn clamp_sampler(device: &wgpu::Device) -> wgpu::Sampler {
	device.create_sampler(&wgpu::SamplerDescriptor {
		label: Some("smooth billboard clamp"),
		address_mode_u: wgpu::AddressMode::ClampToEdge,
		address_mode_v: wgpu::AddressMode::ClampToEdge,
		mag_filter: wgpu::FilterMode::Linear,
		min_filter: wgpu::FilterMode::Linear,
		mipmap_filter: wgpu::MipmapFilterMode::Linear,
		..Default::default()
	})
}

/// `gs_level`'s depth term as written (texfilter.wgsl): on Vulkan the fragment `@builtin(position).w` is 1/w (SPIR-V FragCoord).
pub(in crate::viewer) const VK_LOD: &str = "log2(32.0 / i.pos.w)";
/// The same term for DirectX 12: naga's HLSL backend hands SV_Position.w through as w (no reciprocal), so `32 / w` there was
/// log2(32 vz) and clamped every texel to mip 0 (distant ties, shrubs, plants and fur over-sharp vs Vulkan and PCSX2).
pub(in crate::viewer) const DX_LOD: &str = "log2(32.0 * i.pos.w)";

/// [PC] The level-shader text for the window's API (gpu/renderer.rs `note_running`): unchanged on Vulkan (byte-identical), `DX_LOD`
/// on DirectX 12. Every module built from the level shader goes through it before `devtune::wgsl` (level, level bindless, veg, fur),
/// so a RAC_WGSL_DIR dump on DX12 holds the DX12 text and a file loaded from there is used as is.
pub(in crate::viewer) fn frag_w_fix(src: String) -> String {
	frag_w_for(src, crate::viewer::gpu::renderer::RUNNING_DX12.load(std::sync::atomic::Ordering::Relaxed))
}

/// `frag_w_fix` for a given API. Logs when a module with `gs_level` did not get exactly one patch per `gs_level`.
pub(in crate::viewer) fn frag_w_for(src: String, dx12: bool) -> String {
	if !dx12 {
		return src;
	}
	let (fns, hits) = (src.matches("fn gs_level(").count(), src.matches(VK_LOD).count());
	if hits != fns {
		eprintln!("texfilter: DX12 LOD patch expected {fns} `{VK_LOD}`, found {hits}");
	}
	src.replace(VK_LOD, DX_LOD)
}

#[cfg(test)]
mod tests {
	/// The DX12 text of every module built from the level shader swaps the LOD term once and validates; the Vulkan text is today's.
	#[test]
	fn dx12_lod_term_is_patched_and_vulkan_text_is_unchanged() {
		use super::super::{bindless, furfx, vegfx, volfog};
		for early in [false, true] {
			let level = super::super::metalfx::with_override(volfog::soft::level_source(super::super::SHADER, early));
			for src in [level.clone(), bindless::shader_source(&level), vegfx::pipes::source_with(early), furfx::pipes::source_with(early)] {
				assert_eq!(src.matches(super::VK_LOD).count(), 1);
				assert_eq!(super::frag_w_for(src.clone(), false), src, "Vulkan byte-identical");
				let dx = super::frag_w_for(src, true);
				assert!(dx.contains("log2(32.0 * i.pos.w) + i.kmx.x") && !dx.contains("32.0 / i.pos.w"));
				let m = naga::front::wgsl::parse_str(&dx).expect("WGSL parse");
				naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("WGSL validation");
				// Review W7 item 1: the fire module (flitquad.rs, built from the fixed source) keeps the DX12 term once, with and without
				// the quad twin, and validates (the twin with the caps a SUBGROUP device gives naga).
				let sub = naga::valid::Capabilities::all().difference(naga::valid::Capabilities::SUBGROUP_BARRIER).difference(naga::valid::Capabilities::SUBGROUP_VERTEX_STAGE);
				for q in [false, true] {
					let (f, _, _) = super::super::flitquad::fire_text(&dx, q).expect("fire anchors on the DX12 text");
					assert_eq!(f.matches(super::DX_LOD).count(), 1, "fire text quads={q}");
					let m = naga::front::wgsl::parse_str(&f).expect("WGSL parse");
					naga::valid::Validator::new(naga::valid::ValidationFlags::all(), sub).validate(&m).expect("WGSL validation");
				}
			}
		}
	}

	#[test]
	fn smooth_and_classic_shaders_validate_for_plain_and_bindless() {
		let plain = super::super::volfog::soft::level_source(super::super::SHADER, false);
		for src in [plain.to_owned(), super::super::bindless::shader_source(&plain)] {
			let m = naga::front::wgsl::parse_str(&src).expect("WGSL parse");
			naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("WGSL validation");
		}
	}
}
