//! [PC] Sun shadow pass (lane nsun, 2026-11-03): mobys cast by texel alpha, where the main pass writes their depth.
//!
//! The main pass writes a moby's depth only where texel alpha x draw alpha reaches 96/128 (viewer.rs `fs_moby_a` with `moby_a`; the
//! opaque `fs` draws only the texels at 128), so a palm frond, a glow or ring card or a decal shows only its painted shape. The
//! shadow pass drew every moby depth-only (`vs_sh_props`), so such a card cast its whole quad (corrected 2026-11-03: the stray
//! plaza shapes this was first blamed for were real casters, off-screen or lit by the old key light). Now a moby draw whose texture goes below the cut casts through `fs_sh_props_cut` (the same cut); a texture that never
//! does (`Scene::sun_alpha` >= `MOBY_OPAQUE`) keeps the depth-only pipeline (same depth, no texture fetch). The clipped props and
//! the debris (world space) take the terrain cut (`fs_sh_tfrag`, the same 95.5/128). A slot drawn in the additive pass casts
//! nothing (props.rs `draws`). RAC_SUN_PROPCUT=0 (gfxq.rs) gives the shadows of before.

use super::{Draw, Vertex, SUN_SHADER};

/// The smallest texel alpha (texture value 0..255) that passes the moby cut at every draw alpha a slot casts with (96..128,
/// `sh_place`): only 255 (ta 128: floor(128 x ma / 128) = ma >= 96). 191 passed only at full draw alpha: a slot faded to 112 dropped
/// texel 200 (floor(100 x 112 / 128) = 87), so a half-faded card cast its whole quad (review R26 m2).
pub(super) const MOBY_OPAQUE: u8 = 255;

/// The props shader of the alpha-tested moby casts: `SUN_SHADER` (its placement `sh_place`, the kind rule and the faded-moby rule
/// stay the one copy) plus the texture array in group 2 and the cut.
pub(super) fn shader() -> String {
	format!(
		r#"enable wgpu_binding_array;
{SUN_SHADER}
@group(2) @binding(0) var texs: binding_array<texture_2d<f32>, {cap}>;
@group(2) @binding(1) var samp: sampler;
struct POut {{ @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) @interpolate(flat) ti: u32, @location(2) @interpolate(flat) ma: f32 }};

// `vs_sh_props` with the texel's uv (a placed prop's class scroll added as `vs_props` adds it), the texture slot (the low 16 bits
// of the first instance, `bindless::first_instance`) and the draw alpha.
@vertex fn vs_sh_props_cut(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(5) sj: vec2<u32>, @builtin(instance_index) inst: u32) -> POut {{
	var o: POut;
	let s = insts[sj.x];
	o.pos = sh_place(p, n, sj);
	o.uv = select(uv, uv + s.a.wz, (s.k.x & 0xffu) != 2u);
	o.ti = inst & 0xffffu;
	o.ma = sh_alpha(s);
	return o;
}}

// `moby_a` of the main pass below 96/128: no depth there, no shadow.
@fragment fn fs_sh_props_cut(i: POut) {{
	let ta = floor(textureSampleLevel(texs[i.ti], samp, i.uv, 0.0).a * 128.0 + 0.5);
	if (floor(ta * i.ma / 128.0) < 96.0) {{ discard; }}
}}
"#,
		cap = super::super::bindless::TEX_CAP
	)
}

/// The alpha-tested props pipeline: groups (cascade uniform, posed props, texture array), the batch's position, uv and normal (`sh_grow`), the slot attribute.
pub(super) fn pipeline(device: &wgpu::Device, ulayout: &wgpu::BindGroupLayout, skin: &wgpu::BindGroupLayout, array: &wgpu::BindGroupLayout) -> wgpu::RenderPipeline {
	let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sun shadow moby cut"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("sun shadow moby cut", shader().into())) });
	let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sun shadow moby cut"), bind_group_layouts: &[Some(ulayout), Some(skin), Some(array)], immediate_size: 0 });
	let vu = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3] };
	let sj = wgpu::VertexBufferLayout { array_stride: 8, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![5 => Uint32x2] };
	device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
		label: Some("fs_sh_props_cut"),
		layout: Some(&pl),
		vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_sh_props_cut"), compilation_options: Default::default(), buffers: &[Some(vu), Some(sj)] },
		fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs_sh_props_cut"), compilation_options: Default::default(), targets: &[] }),
		primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
		depth_stencil: Some(wgpu::DepthStencilState {
			format: wgpu::TextureFormat::Depth32Float,
			depth_write_enabled: Some(true),
			depth_compare: Some(wgpu::CompareFunction::Less),
			stencil: Default::default(),
			bias: wgpu::DepthBiasState { constant: 0, slope_scale: 1.5, clamp: 0.0 },
		}),
		multisample: Default::default(),
		multiview_mask: None,
		cache: None,
	})
}

/// Whether a moby draw casts through the cut: `cut` (the moby cut runs this frame) and not a texture known never to go below it
/// (`min` = `Scene::sun_alpha` of its slot; 0 = unknown) while depth-only casts are allowed (`plain_ok`).
pub(super) fn alpha_tested(cut: bool, plain_ok: bool, min: Option<u8>) -> bool {
	cut && !(plain_ok && min.is_some_and(|a| a >= MOBY_OPAQUE))
}

/// Sorts the props runs of `props::draws` (`(first index, index count, draw index)`) into the alpha-tested list (with the slot word,
/// `bindless::first_instance`) and the depth-only list.
pub(super) fn split(draws: &[Draw], sun_alpha: &[u8], list: &[(u32, u32, u32)], cut: bool, plain_ok: bool, cut_l: &mut Vec<(u32, u32, u32)>, plain_l: &mut Vec<(u32, u32, u32)>) {
	cut_l.clear();
	plain_l.clear();
	for &(start, count, di) in list {
		let Some(d) = draws.get(di as usize) else { continue };
		if alpha_tested(cut, plain_ok, sun_alpha.get(d.texture).copied()) {
			cut_l.push((start, count, super::super::bindless::first_instance(d)));
		} else {
			plain_l.push((start, count, 0));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_textures_that_go_below_the_cut_are_alpha_tested() {
		// `fs_sh_props_cut` on a texel value at a draw alpha: kept (depth written) or discarded.
		let kept = |texel: u8, ma: f32| ((texel as f32 / 255.0 * 128.0 + 0.5).floor() * ma / 128.0).floor() >= 96.0;
		assert!((96..=128).all(|ma| kept(MOBY_OPAQUE, ma as f32)), "the depth-only texel passes at every draw alpha a slot casts with");
		assert!(!kept(MOBY_OPAQUE - 1, 96.0) && !kept(200, 112.0), "anything less is cut at some draw alpha");
		assert!(!alpha_tested(true, true, Some(MOBY_OPAQUE)));
		assert!(alpha_tested(true, true, Some(MOBY_OPAQUE - 1)) && alpha_tested(true, true, Some(0)) && alpha_tested(true, true, None));
		assert!(alpha_tested(true, false, Some(255)), "RAC_SUN_NOCULL: every draw alpha-tested");
		assert!(!alpha_tested(false, true, Some(0)) && !alpha_tested(false, false, None), "RAC_SUN_PROPCUT=0: depth-only");
	}

	#[test]
	fn the_split_keeps_runs_and_carries_the_slot() {
		let d = |texture: usize| Draw { inst: 0, cull: 0, texture, start: 0, count: 0 };
		let draws = [d(3), d(7)];
		let alpha = [0u8, 0, 0, 255, 0, 0, 0, 10];
		let (mut c, mut p) = (Vec::new(), Vec::new());
		split(&draws, &alpha, &[(0, 6, 0), (6, 9, 1), (30, 3, 1), (40, 3, 5)], true, true, &mut c, &mut p);
		assert_eq!(p, [(0, 6, 0)], "the opaque texture casts depth-only");
		let w = super::super::super::bindless::first_instance(&draws[1]);
		assert_eq!(w & 0xffff, 7);
		assert_eq!(c, [(6, 9, w), (30, 3, w)], "the cut draws carry their slot; a run past the draws is dropped");
		split(&draws, &alpha, &[(0, 6, 0), (6, 9, 1)], false, true, &mut c, &mut p);
		assert!(c.is_empty() && p == [(0, 6, 0), (6, 9, 0)]);
	}

	/// The cut shader is `SUN_SHADER` plus the cut: the placement and the kind rule are not copied.
	#[test]
	fn the_cut_shader_reuses_the_placement() {
		let src = shader();
		assert!(src.starts_with("enable wgpu_binding_array;\n"));
		assert!(src.contains(SUN_SHADER));
		assert_eq!(src.matches("kind == ").count(), 3, "the keep rule only once (in SUN_SHADER's `sh_place`)");
		assert!(src.contains("o.pos = sh_place(p, n, sj);") && src.contains("o.ma = sh_alpha(s);"));
		assert!(src.contains("if (floor(ta * i.ma / 128.0) < 96.0) { discard; }"), "the cut of `fs_moby_a`: moby_a x 128 < 96");
		assert!(src.contains("@group(2) @binding(0) var texs: binding_array<texture_2d<f32>, 4096>;"));
	}

	/// Both props shaders (the depth-only `SUN_SHADER` with its placement moved into `sh_place`, and the cut) parse and validate.
	#[test]
	fn the_shadow_shaders_validate() {
		for (name, src) in [("sun shadow", SUN_SHADER.to_string()), ("moby cut", shader())] {
			let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{name}: {e}"));
			naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{name}: {e:?}"));
		}
	}
}
