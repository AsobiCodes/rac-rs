//! [PC] (speed, lane nwater, 2026-10-29) The "bed minimum" for the geometric sea's shore fade. `shore_fade` (shore_fade.rs) reads 16
//! texels of the half-size bed depth per water pixel (4 wide reads x 4 bilinear taps, each tap clamped to the texture and turned into
//! a water depth `wl_d`) and fades to 0 only where the water over the bed is under SHORE_FADE (0.6) deep: on open sea (almost all of
//! the pixels) every tap is deeper and the fade is exactly 1.0. The taps of a pixel with base texel `fl` = floor(uv x size - 0.5)
//! lie in the 4 x 4 texels `fl-1 ..= fl+2` (wide reads at +-0.75 texel: floor(t - 0.75) is fl-1 or fl, floor(t + 0.75) is fl or fl+1,
//! the taps add 0 and 1; the diagonal pair of the near switch is a subset). A compute pass (`cs_bedmin`, once per frame before the
//! water shade pass) writes the minimum of the raw depth over the 4 x 4 window starting at each texel (coordinates clamped like
//! `bed_tap`'s; `linear_depth` is monotone in it). `bed_all_deep` asks it: when the window's nearest ground z is at least zw x (1 + 0.65 / K) (K = the camera's height
//! over the water, zw the water pixel's view z), every tap's `wl_d` is at least 0.65 (the monotone K x (z / zw - 1), or 40 for no
//! ground and for the swimming hero, which only ever raise a tap), so the bilinear mix, the 0.25 sum and `smoothstep(0, 0.6, .)` give
//! exactly 1.0 and `shore_fade_lod` returns the same `mix(1.0, 1.0, lod)` as the full path, without the 16 reads. A window that
//! reaches past the texture edge is only larger there (clamped taps repeat the edge texel), so the test can only fail more often,
//! never pass wrongly. The picture is byte for byte the same; RAC_WATER_BED_MIN=0 turns the pass and the shortcut off (A/B switch).

use super::*;

/// `w.lod.w` bit: the pass ran this frame (a geometric sea is drawn) and `bedmin` holds this frame's windows.
const FLAG: f32 = 16.0;

/// The near-identical switch mask of `far_lod::params` with the bed-min bit when it applies (`sea`: a geometric sea is drawn).
pub(super) fn lod_flags(mut lod: [f32; 4], sea: bool) -> [f32; 4] {
	if sea && enabled() {
		lod[3] += FLAG;
	}
	lod
}

/// RAC_WATER_BED_MIN=0 turns the pass off (default on); read once. Off too when the driver refused `cs_bedmin` (pipe_guard.rs).
pub(super) fn enabled() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_WATER_BED_MIN").ok().as_deref() != Some("0")) && !crate::viewer::pipe_guard::off(crate::viewer::pipe_guard::Effect::WaterBedMin)
}

/// WGSL appended to the water shader. Group 0 binding 14 is the pass's output as `fs_water` reads it; group 1 binding 0 is the
/// storage view the pass writes (a pipeline uses one of the two).
pub(super) const BED_MIN_WGSL: &str = r#"
@group(0) @binding(14) var bedmin: texture_2d<f32>;
@group(1) @binding(0) var bedmin_out: texture_storage_2d<r32float, write>;
const BM_T: i32 = 16; // the output tile (one workgroup of 16 x 16 threads); its input tile is (BM_T + 3)^2 texels
const BM_I: u32 = 19u;
var<workgroup> bm_tile: array<f32, 361>;

// The minimum of the bed's raw depth over the 4 x 4 texels starting at each texel (clamped coordinates), sample 0 like `bed_tap`.
// Raw, not `linear_depth`: the fragment stage turns the minimum into a view z itself (`bed_all_deep`), with the same code as the
// taps' own, so no compute-vs-fragment rounding difference can creep in (`linear_depth` is monotone in the raw depth).
@compute @workgroup_size(16, 16, 1)
fn cs_bedmin(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) li: vec3<u32>, @builtin(local_invocation_index) lidx: u32) {
	let bd = vec2<i32>(textureDimensions(bed_depth));
	let o = vec2<i32>(wg.xy) * BM_T;
	for (var k = lidx; k < BM_I * BM_I; k += 256u) {
		let q = o + vec2<i32>(i32(k % BM_I), i32(k / BM_I));
		bm_tile[k] = textureLoad(bed_depth, clamp(q, vec2<i32>(0), bd - vec2<i32>(1)), 0);
	}
	workgroupBarrier();
	let p = o + vec2<i32>(li.xy);
	if (any(p >= bd)) { return; }
	var m = bm_tile[li.y * BM_I + li.x];
	for (var j = 0u; j < 4u; j++) {
		for (var i = 0u; i < 4u; i++) { m = min(m, bm_tile[(li.y + j) * BM_I + li.x + i]); }
	}
	textureStore(bedmin_out, p, vec4<f32>(m, 0.0, 0.0, 0.0));
}

// Is every tap of `shore_fade` at the water pixel pf (view z zw) at least 0.65 deep? (Then its value is exactly 1.0; see the file docs.)
fn bed_all_deep(pf: vec3<f32>, zw: f32) -> bool {
	let k = w.eye.z - pf.z;
	if ((u32(w.lod.w) & 16u) == 0u || !(k > 0.0 && zw > 0.0)) { return false; }
	let bd = vec2<i32>(textureDimensions(bed_depth));
	let fl = vec2<i32>(floor(project(pf).xy * vec2<f32>(bd) - 0.5));
	return linear_depth(textureLoad(bedmin, clamp(fl - vec2<i32>(1), vec2<i32>(0), bd - vec2<i32>(1)), 0).r) >= zw * (1.0 + 0.65 / k);
}

// `mix(shore_fade(..), 1.0, lod)` of the geometric sea; where all taps are deep the fade is the same 1.0 as a runtime value.
fn shore_fade_lod(pf: vec3<f32>, zw: f32, lod: f32) -> f32 {
	var sh = 1.0;
	if (!bed_all_deep(pf, zw)) { sh = shore_fade(project(pf).xy, pf, zw); }
	return mix(sh, 1.0, lod);
}
"#;

/// The compute pipeline and the layouts around it (made once with the water pass).
pub(super) struct BedMin {
	c0: wgpu::BindGroupLayout,
	out_l: wgpu::BindGroupLayout,
	pipe: wgpu::ComputePipeline,
}

/// The windows' texture (as big as the bed depth) and the groups on it, made with the targets.
pub(super) struct BedMinGroups {
	g_c0: wgpu::BindGroup,
	g_out: wgpu::BindGroup,
	/// What `fs_water` reads as `bedmin` (group 0 binding 14).
	pub(super) view: wgpu::TextureView,
	size: (u32, u32),
}

impl BedMin {
	/// Built under its own guard: a driver that refuses `cs_bedmin` turns only this shortcut off (`enabled`), not the water.
	pub(super) fn new(device: &wgpu::Device, shader: &wgpu::ShaderModule) -> BedMin {
		crate::viewer::pipe_guard::build(device, crate::viewer::pipe_guard::Effect::WaterBedMin, || Self::build(device, shader))
	}

	fn build(device: &wgpu::Device, shader: &wgpu::ShaderModule) -> BedMin {
		let c = wgpu::ShaderStages::COMPUTE;
		let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: c, ty, count: None };
		let uniform = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
		let depth = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: true };
		let store = wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::R32Float, view_dimension: wgpu::TextureViewDimension::D2 };
		let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries });
		let c0 = layout("water bed min scene", &[entry(0, uniform), entry(9, depth)]);
		let out_l = layout("water bed min out", &[entry(0, store)]);
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water bed min"), bind_group_layouts: &[Some(&c0), Some(&out_l)], immediate_size: 0 });
		let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("water bed min"), layout: Some(&pl), module: shader, entry_point: Some("cs_bedmin"), compilation_options: Default::default(), cache: None });
		BedMin { c0, out_l, pipe }
	}

	/// The texture for the bed depth `bed_depth` (depth aspect view, MSAA) and the groups on it; `ubuf` = the water uniforms.
	pub(super) fn groups(&self, device: &wgpu::Device, ubuf: &wgpu::Buffer, bed_depth: &wgpu::TextureView) -> BedMinGroups {
		let size = (bed_depth.texture().width().max(1), bed_depth.texture().height().max(1));
		let tex = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("water bed min"),
			size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::R32Float,
			usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		});
		let view = tex.create_view(&Default::default());
		let group = |label, layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry]| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout, entries });
		BedMinGroups {
			g_c0: group("water bed min scene", &self.c0, &[
				wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(bed_depth) },
			]),
			g_out: group("water bed min out", &self.out_l, &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }]),
			view,
			size,
		}
	}

	/// The windows of this frame's bed depth (after the bed pass, before the water shade pass).
	pub(super) fn dispatch(&self, enc: &mut wgpu::CommandEncoder, g: &BedMinGroups) {
		let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("water bed min"), timestamp_writes: crate::viewer::passtime::cw("water bed min") });
		cp.set_pipeline(&self.pipe);
		cp.set_bind_group(0, &g.g_c0, &[]);
		cp.set_bind_group(1, &g.g_out, &[]);
		cp.dispatch_workgroups(g.size.0.div_ceil(16), g.size.1.div_ceil(16), 1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_flag_is_one_bit_above_the_near_switches() {
		assert_eq!(FLAG as u32 & 15, 0);
		let lod = super::super::far_lod::params();
		assert_eq!(lod_flags(lod, false), lod);
		let f = lod_flags([25.0, 50.0, 0.0, 5.0], true);
		assert_eq!(f[3] as u32 & 15, 5);
		assert_eq!(f[3] as u32 & 16, if enabled() { 16 } else { 0 });
	}

	/// The window the shader reads (`fl-1 ..= fl+2`) holds every texel a tap can reach: floor(t +- 0.75 + 0/1) for t in fl..fl+1.
	#[test]
	fn the_window_holds_every_tap() {
		for step in 0..4000 {
			let t = 100.0f32 + step as f32 * 0.00025;
			let fl = t.floor() as i32;
			for dx in [-0.75f32, 0.75] {
				let base = (t + dx).floor() as i32;
				for tap in [base, base + 1] {
					assert!(tap >= fl - 1 && tap <= fl + 2, "t {t} dx {dx} tap {tap}");
				}
			}
		}
	}

	/// Where every tap is at least 0.65 deep, the fade is exactly 1.0 (bilinear mix of values >= 0.65, the 0.25 sum, smoothstep).
	#[test]
	fn deep_taps_give_a_fade_of_exactly_one() {
		let smooth = |x: f32| {
			let t = (x / 0.6).clamp(0.0, 1.0);
			t * t * (3.0 - 2.0 * t)
		};
		for d in [0.65f32, 0.6500001, 0.7, 1.0, 5.0, 40.0] {
			for f in [0.0f32, 0.1, 0.37, 0.5, 0.99, 1.0] {
				let mix = |a: f32, b: f32, t: f32| a * (1.0 - t) + b * t;
				let m = mix(mix(d, d, f), mix(d, d, f), f);
				let sum = 0.25 * (m + m + m + m);
				assert_eq!(smooth(sum), 1.0, "d {d} f {f}");
			}
		}
	}
}
