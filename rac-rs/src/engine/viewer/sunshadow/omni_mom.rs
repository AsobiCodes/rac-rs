//! [PC] Fire light v2 stage 4 (lane dlit3, FIRE_LIGHT_V2.md 3.3): the fire shadow maps' prefiltered MOMENT cubes. After the six
//! faces of each drawn fire map are in the atlas (omni.rs `encode_faces`), one compute pass (omni_mom.wgsl) turns them into a cube
//! array of 4-moment texels (`MAX_LIGHTS` cubes, `MOM` texels a face side, `MIPS` levels by 2 x 2 box means): per texel the mean of
//! (z, z^2, z^3, z^4) over its `sub(tile)`^2 sub-directions, z = the radial distance from the map's origin over [near, far], stored
//! through Peters & Klein 2015's quantisation transform (`optimized`; `unoptimized` undoes it). The level / bindless / vegetation /
//! fur shaders see the cube array at group 0 binding `BINDING` (`LAYOUT_ENTRY`, sunshadow.rs `entries_soft`; flitshadow.wgsl
//! `fls_mom`, read with the metal look's linear sampler). Since stage 5 the receivers' fire shadow lookup reads it (flitshadow.wgsl
//! `flit_shadow`; flit_debug=5 shows it, `flit_debug_mom`). The pass runs only on frames that draw a fire map (omni.rs `light_shadows`: nothing at GI Low / Off,
//! Classic, fire-free frames); RAC_TUNE flsh_mom=0 skips it.

use super::omni::{FACES, MAX_LIGHTS};

/// Moment texels a cube face side (mip 0).
pub(in crate::viewer) const MOM: u32 = 128;
/// Mip levels: 128, 64, 32, 16, 8 (the coarsest texel ~11 degrees; with trilinear between them, cones of tens of degrees).
pub(in crate::viewer) const MIPS: u32 = 5;
/// Array layers: six faces a map (cube k = layers 6 k ..).
pub(in crate::viewer) const LAYERS: u32 = (MAX_LIGHTS * FACES) as u32;
/// The moment texel format (3.3: Rgba16Float first; Rgba32Float the fallback if it leaks).
pub(in crate::viewer) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Group 0 binding of the cube array in the level shaders (after metalenv.rs's 4..9).
pub(in crate::viewer) const BINDING: u32 = 10;
const WG: u32 = 8;

pub(in crate::viewer) const LAYOUT_ENTRY: wgpu::BindGroupLayoutEntry = wgpu::BindGroupLayoutEntry {
	binding: BINDING,
	visibility: wgpu::ShaderStages::FRAGMENT,
	ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::CubeArray, multisampled: false },
	count: None,
};

const WGSL: &str = include_str!("omni_mom.wgsl");

/// Peters & Klein 2015 (moment shadow mapping, optimised quantisation): optimized[j] = sum_i b[i] F[i][j] + (OFS, 0, 0, 0).
const F: [[f32; 4]; 4] = [
	[-2.07224649, 13.7948857237, 0.105877704, 9.7924062118],
	[32.23703778, -59.4683975703, -1.9077466311, -33.7652110555],
	[-68.571074599, 82.0359750338, 9.3496555107, 47.9456096605],
	[39.3703274134, -35.364903257, -6.6543490496, -23.9728048165],
];
const OFS: f32 = 0.035955884801;
/// The inverse transform (applied after removing `OFS`).
const G: [[f32; 4]; 4] = [
	[0.2227744146, 0.1549679261, 0.1451988946, 0.163127443],
	[0.0771972861, 0.1394629426, 0.2120202157, 0.2591432266],
	[0.7926986636, 0.7963415838, 0.7258694464, 0.6539092497],
	[0.0319417555, -0.1722823173, -0.2758014811, -0.3376131734],
];

fn mul(b: [f32; 4], m: &[[f32; 4]; 4]) -> [f32; 4] {
	std::array::from_fn(|j| (0..4).map(|i| b[i] * m[i][j]).sum())
}

/// The stored texel of depth z (twin of omni_mom.wgsl `cs_mom`'s store).
pub(in crate::viewer) fn optimized(z: f32) -> [f32; 4] {
	let z2 = z * z;
	let mut o = mul([z, z2, z2 * z, z2 * z2], &F);
	o[0] += OFS;
	o
}

/// The 4 moments (z, z^2, z^3, z^4) of a stored texel (twin of flitshadow.wgsl `fls_moments`).
pub(in crate::viewer) fn unoptimized(o: [f32; 4]) -> [f32; 4] {
	mul([o[0] - OFS, o[1], o[2], o[3]], &G)
}

/// Depth texels a moment texel side reads (sub-directions per side): the face tile over `MOM`, rounded up, 2..4 (256 -> 2, 408 / 512
/// -> 4), so every depth texel under a moment texel is seen at least once.
pub(in crate::viewer) const fn sub(tile: u32) -> u32 {
	let s = tile.div_ceil(MOM);
	if s < 2 { 2 } else if s > 4 { 4 } else { s }
}

/// Texels a side of mip `m`.
pub(in crate::viewer) const fn mip_size(m: u32) -> u32 {
	MOM >> m
}

/// Twin of omni_mom.wgsl `mom_dir`: cube face `f`'s direction at (s, t) in [-1, 1].
pub(in crate::viewer) fn cube_dir(f: usize, s: f32, t: f32) -> [f32; 3] {
	match f {
		0 => [1.0, -t, -s],
		1 => [-1.0, -t, s],
		2 => [s, 1.0, t],
		3 => [s, -1.0, -t],
		4 => [s, -t, 1.0],
		_ => [-s, -t, -1.0],
	}
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MomU {
	q: [f32; 4],
	m: [[f32; 4]; MAX_LIGHTS],
}

struct Passes {
	conv: wgpu::ComputePipeline,
	down: wgpu::ComputePipeline,
	/// `conv`'s group (uniform, the atlas, mip 0), then `down`'s per mip 1.. (mip m - 1 in, mip m out).
	binds: Vec<wgpu::BindGroup>,
}

pub(in crate::viewer) struct OmniMom {
	_tex: wgpu::Texture,
	/// The whole cube array, every mip (group 0 binding `BINDING`).
	pub(in crate::viewer) cube: wgpu::TextureView,
	ubuf: wgpu::Buffer,
	/// None when the compute pipelines failed (a validation error, e.g. DX12 without dxcompiler.dll and FXC refusing them): the cube
	/// stays unwritten, so no fire map is drawn at all (`ok`, omni.rs `fire_maps`; review S5).
	passes: Option<Passes>,
}

impl OmniMom {
	/// The cube array and its passes; `atlas` = the fire maps' depth atlas layer (omni.rs, the sun map's layer CASCADES).
	pub(in crate::viewer) fn new(device: &wgpu::Device, atlas: &wgpu::TextureView) -> OmniMom {
		let tex = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("flit moments"),
			size: wgpu::Extent3d { width: MOM, height: MOM, depth_or_array_layers: LAYERS },
			mip_level_count: MIPS,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: FORMAT,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
			view_formats: &[],
		});
		let cube = tex.create_view(&wgpu::TextureViewDescriptor { label: Some("flit moments cube"), dimension: Some(wgpu::TextureViewDimension::CubeArray), ..Default::default() });
		let mip = |m: u32| tex.create_view(&wgpu::TextureViewDescriptor { label: Some("flit moments mip"), dimension: Some(wgpu::TextureViewDimension::D2Array), base_mip_level: m, mip_level_count: Some(1), ..Default::default() });
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("flit moments"), size: std::mem::size_of::<MomU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("omni_mom"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("omni_mom", WGSL.into())) });
		let pipe = |entry: &str| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("flit moments"), layout: None, module: &module, entry_point: Some(entry), compilation_options: Default::default(), cache: None });
		let (conv, down) = (pipe("cs_mom"), pipe("cs_down"));
		let views: Vec<wgpu::TextureView> = (0..MIPS).map(mip).collect();
		let tv = wgpu::BindingResource::TextureView;
		let mut binds = vec![device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("flit moments"),
			layout: &conv.get_bind_group_layout(0),
			entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: tv(atlas) }, wgpu::BindGroupEntry { binding: 2, resource: tv(&views[0]) }],
		})];
		let dl = down.get_bind_group_layout(0);
		binds.extend((1..MIPS as usize).map(|m| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("flit moments mip"), layout: &dl, entries: &[wgpu::BindGroupEntry { binding: 2, resource: tv(&views[m]) }, wgpu::BindGroupEntry { binding: 3, resource: tv(&views[m - 1]) }] })));
		let passes = match pollster::block_on(scope.pop()) {
			None => Some(Passes { conv, down, binds }),
			Some(e) => {
				eprintln!("flit moments: off ({e})");
				None
			}
		};
		OmniMom { _tex: tex, cube, ubuf, passes }
	}

	/// The moment passes built: the receivers may read the cube (false: omni.rs `fire_maps` draws no fire map).
	pub(in crate::viewer) fn ok(&self) -> bool {
		self.passes.is_some()
	}

	/// The moment cubes of this frame's drawn maps: `maps` = (near, far) of map 0.., faces `(tile, side)` (omni.rs `layout`).
	pub(in crate::viewer) fn encode(&self, queue: &super::GatedQueue, enc: &mut wgpu::CommandEncoder, maps: &[(f32, f32)], (tile, side): (u32, u32)) {
		let Some(p) = &self.passes else { return };
		if maps.is_empty() || crate::devtune::f32("flsh_mom", 1.0) <= 0.0 {
			return;
		}
		let _s = super::super::profile::span("flit_moments");
		let mut u = MomU { q: [tile as f32, side as f32, sub(tile) as f32, 0.0], m: [[0.0; 4]; MAX_LIGHTS] };
		for (row, &(near, far)) in u.m.iter_mut().zip(maps) {
			*row = [near, far, 0.0, 0.0];
		}
		queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));
		let layers = (maps.len().min(MAX_LIGHTS) * FACES) as u32;
		let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("flit moments"), timestamp_writes: crate::viewer::passtime::cw("flit moments") });
		pass.set_pipeline(&p.conv);
		pass.set_bind_group(0, &p.binds[0], &[]);
		pass.dispatch_workgroups(MOM.div_ceil(WG), MOM.div_ceil(WG), layers);
		pass.set_pipeline(&p.down);
		for m in 1..MIPS {
			let n = mip_size(m).div_ceil(WG);
			pass.set_bind_group(0, &p.binds[m as usize], &[]);
			pass.dispatch_workgroups(n, n, layers);
		}
	}
}

/// Twin of omni_mom.wgsl `cs_mom`'s drawn-face projection: the NDC of direction `d` on the drawn face `f` (omni.rs `face`).
#[cfg(test)]
fn drawn_ndc(f: usize, d: [f32; 3]) -> [f32; 2] {
	let ax = f / 2;
	let z = d[ax].abs();
	let s = super::omni::SCALE;
	[d[(ax + 1) % 3] * s / z, d[(ax + 2) % 3] * s / z]
}

#[cfg(test)]
#[path = "omni_mom_tests.rs"]
mod tests;
