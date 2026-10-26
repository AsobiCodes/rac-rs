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

#[cfg(test)]
mod tests {
	#[test]
	fn smooth_and_classic_shaders_validate_for_plain_and_bindless() {
		let plain = super::super::volfog::soft::level_source(super::super::SHADER, false);
		for src in [plain.to_owned(), super::super::bindless::shader_source(&plain)] {
			let m = naga::front::wgsl::parse_str(&src).expect("WGSL parse");
			naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("WGSL validation");
		}
	}
}
