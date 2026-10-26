//! [PC] Volumetric fog: the depth of the blended texels that write no Z. Retail fades the bottoms of cliffs, pillars, mesas and
//! buildings by texture alpha: terrain texels below alpha 0x60, tie texels below 0x40 and the shrub fades blend over what lies
//! behind without a Z write (gpu/setup.rs `tfrag_pipeline_lt`, `tie_pipelines`, `shrub_pipelines`). The fog found no depth there,
//! took those pixels for void and painted the far haze and the fog floor over the faded skirt, so the bottoms ended in a hard
//! stair-stepped cut (user 2026-10-25, Veldin / Eudora / Kerwan). With Lighting Enhanced those fragments also store their distance
//! and alpha in a per-pixel buffer (group 0 binding 3, the nearest one wins); `fs_apply` (volfog.rs) fogs such a pixel at that
//! distance, weighted by the alpha, as retail's GS fog fades the texel itself. The level pass's colour and depth stay untouched;
//! Classic writes nothing (`u.sun_texel.w` = 0), the mirrors and previews bind a buffer of size 0 (`off_buffer`).
//! Split view: the Classic pass gets the Classic lighting uniforms (`u.sun_texel.w` = 0), so it writes nothing, and the fog
//! apply that reads the buffer only runs for the Enhanced pass.

/// Group 0 binding 3 of the level shader: the per-pixel buffer, read-write from the fragment stage.
pub(in crate::viewer) const LAYOUT_ENTRY: wgpu::BindGroupLayoutEntry = wgpu::BindGroupLayoutEntry {
	binding: 3,
	visibility: wgpu::ShaderStages::FRAGMENT,
	ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
	count: None,
};

/// Header words: width, height (0 = no buffer), two spare; the pixels follow, one u32 each: (2^24 - 1 - distance * 256) << 8 |
/// alpha * 255, so the largest value is the nearest texel and 0 is none.
const HEADER: u64 = 16;

/// The level shader's side (appended by `level_source`): `soft_mark` is called by the no-Z fragment entry points.
const WORLD: &str = r#"
@group(0) @binding(3) var<storage, read_write> fog_soft: array<atomic<u32>>;
// [PC] Volumetric fog (volfog/soft.rs): this blended no-Z texel's distance and alpha for the fog, the nearest one per pixel.
fn soft_mark(i: VOut, a: f32) {
	if (u.sun_texel.w <= 0.0) { return; }
	let w = atomicLoad(&fog_soft[0]);
	let p = vec2<u32>(i.pos.xy);
	if (p.x >= w || p.y >= atomicLoad(&fog_soft[1])) { return; }
	let q = u32(clamp(distance(i.w, u.eye.xyz) * 256.0, 0.0, 16777215.0));
	atomicMax(&fog_soft[4u + p.y * w + p.x], ((16777215u - q) << 8u) | u32(clamp(a * 255.0 + 0.5, 1.0, 255.0)));
}
"#;

/// The volfog shader's side (bindings 11 and 12 of its group): the nearest no-Z texel at a pixel, the level's MSAA depth.
pub(super) const READ: &str = r#"
@group(0) @binding(11) var<storage, read> soft_px: array<u32>;
@group(0) @binding(12) var depth_ms: texture_depth_multisampled_2d;
// The nearest blended no-Z texel at full-size pixel `pos` (soft.rs): (distance from the eye, alpha); alpha 0 = none.
fn soft_surface(pos: vec2<f32>) -> vec2<f32> {
	let w = soft_px[0];
	let p = vec2<u32>(pos);
	if (p.x >= w || p.y >= soft_px[1]) { return vec2<f32>(0.0); }
	let s = soft_px[4u + p.y * w + p.x];
	if (s == 0u) { return vec2<f32>(0.0); }
	return vec2<f32>(f32(16777215u - (s >> 8u)) / 256.0, f32(s & 255u) / 255.0);
}
"#;

/// The no-Z fragment entry points that store their texel (`soft_mark`).
const STORERS: [&str; 4] = ["fs_tfrag_lt96", "fs_tie_lt64", "fs_shrub_lt12", "fs_shrub_lt96"];

/// The device feature `level_source` uses where the adapter has it (Vulkan; DirectX 12 has none in wgpu).
pub(in crate::viewer) fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
	wgpu::Features::SHADER_EARLY_DEPTH_TEST & adapter.features()
}

/// The level shader with the fog's no-Z texel store (gpu/setup.rs builds the plain and the bindless module from it). `early`
/// (the device has SHADER_EARLY_DEPTH_TEST; RAC_TUNE fog_early=0 turns it off for A/B): the storing entry points force the depth
/// test before the fragment shader. A storage write otherwise moves it after the shader, so every hidden blended texel (the second
/// terrain pass, shrubs, Lush vegetation) ran the whole shader: Eudora at 2000x1088 fell from ~170 to ~110 fps with any Fog level
/// (user 2026-10-25). These pipelines write no depth, so testing first only skips texels the test drops anyway; the one change is that a
/// hidden texel no longer leaves its mark (772 of 2.2M pixels at that view, single pixels along far plant edges).
pub(in crate::viewer) fn level_source(src: &str, early: bool) -> String {
	let mut s = format!("{src}{WORLD}");
	if early && crate::devtune::f32("fog_early", 1.0) >= 0.5 {
		for fs in STORERS {
			s = s.replace(&format!("@fragment fn {fs}("), &format!("@fragment @early_depth_test(force) fn {fs}("));
		}
	}
	s
}

/// Whether `device` can force the early depth test (`level_source`).
pub(in crate::viewer) fn early(device: &wgpu::Device) -> bool {
	device.features().contains(wgpu::Features::SHADER_EARLY_DEPTH_TEST)
}

/// The buffer the mirrors, the previews and the bed bind (sunshadow.rs `entries`): width 0, nothing is written.
pub(in crate::viewer) fn off_buffer(device: &wgpu::Device) -> wgpu::Buffer {
	device.create_buffer(&wgpu::BufferDescriptor { label: Some("volfog soft off"), size: HEADER, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false })
}

/// The level pass's buffer at the picture size, with the group-0 layout to rebind it.
pub(in crate::viewer) struct Soft {
	layout: wgpu::BindGroupLayout,
	buf: Option<(wgpu::Buffer, (u32, u32))>,
}

impl Soft {
	pub(in crate::viewer) fn new(layout: &wgpu::BindGroupLayout) -> Soft {
		Soft { layout: layout.clone(), buf: None }
	}

	pub(in crate::viewer) fn layout(&self) -> &wgpu::BindGroupLayout {
		&self.layout
	}

	/// This frame's buffer (None before the first Enhanced frame).
	pub(in crate::viewer) fn buffer(&self) -> Option<&wgpu::Buffer> {
		self.buf.as_ref().map(|b| &b.0)
	}

	/// Every Enhanced frame before the level pass: the buffer for the picture size, emptied. True = a new buffer (the caller rebinds
	/// group 0 with it: sunshadow.rs `entries_soft`).
	pub(in crate::viewer) fn prepare(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, size: (u32, u32)) -> bool {
		let size = (size.0.max(1), size.1.max(1));
		if let Some((b, s)) = &self.buf
			&& *s == size
		{
			enc.clear_buffer(b, HEADER, None);
			return false;
		}
		let b = device.create_buffer(&wgpu::BufferDescriptor { label: Some("volfog soft"), size: HEADER + size.0 as u64 * size.1 as u64 * 4, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: true });
		if let Ok(mut head) = b.slice(..HEADER).get_mapped_range_mut() {
			head.copy_from_slice(bytemuck::cast_slice(&[size.0, size.1, 0, 0]));
		}
		b.unmap();
		self.buf = Some((b, size));
		true
	}
}

#[cfg(test)]
mod tests {
	/// The level shader with the store parses and validates, plain and bindless, and every no-Z entry point stores its texel.
	#[test]
	fn level_source_validates() {
		let src = super::level_source(crate::viewer::SHADER, false);
		let early = super::level_source(crate::viewer::SHADER, true);
		for s in [src.clone(), crate::viewer::bindless::shader_source(&src), early.clone(), crate::viewer::bindless::shader_source(&early)] {
			let m = naga::front::wgsl::parse_str(&s).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&s)));
			naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("level wgsl validate");
		}
		for fs in super::STORERS {
			let body = &src[src.find(&format!("fn {fs}(")).expect(fs)..];
			assert!(body[..body.find("\n}").unwrap()].contains("soft_mark(i, r.a);"), "{fs} stores its texel");
			assert!(early.contains(&format!("@fragment @early_depth_test(force) fn {fs}(")), "{fs} tests depth first");
		}
		// Every entry point that stores is in STORERS (a new one would run its whole shader for hidden texels again).
		assert_eq!(src.matches("soft_mark(i, ").count(), super::STORERS.len());
	}
}
