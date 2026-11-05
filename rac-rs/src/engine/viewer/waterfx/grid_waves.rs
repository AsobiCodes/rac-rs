//! [PC] (speed) The geometric sea grid's vertices. `vs_grid` works out `grid_wave` (the sea_near tile test, the shore calm,
//! `edge_calm`, the wave sum: 60 % of the grid pass on Pokitaru) for every vertex it shades, and the GPU shades a vertex of an
//! indexed grid once per vertex-reuse batch it falls into. Default: the old row by row index order. Opt-in RAC_TUNE=grid_strip=4
//! (same picture): `grid_indices` walks the grid in strips 4 quads wide (meant to put a block of the grid in one batch instead
//! of a 2-row sliver; same triangles, only their order within a plane changes), but it MEASURED SLOWER (Pokitaru 1080p sea grid
//! 0.103 -> 0.217 ms, Batalia 0.154 -> 0.169), so it stays off and is kept only for tests. RAC_TUNE=grid_cs=1 (not exact, default off): a compute pass
//! (`cs_wave`, after the dry tiles) makes each vertex's height once into a buffer and `vs_grid_cs` reads it; the compute stage's
//! float rounding differs from the vertex stage's in the far waves (Pokitaru 1080p: 267 px of 2.07 M differ, channel sum <= 53).

use super::*;

/// Quads per strip of the index buffer (`grid_indices`; RAC_TUNE=grid_strip=N, 0 = the old row by row order).
fn strip() -> u32 {
	static V: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
	*V.get_or_init(|| crate::devtune::f32("grid_strip", 0.0).max(0.0) as u32)
}

/// RAC_TUNE=grid_cs=1: the heights from the compute pass (see the module doc).
fn compute_on() -> bool {
	static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*V.get_or_init(|| crate::devtune::f32("grid_cs", 0.0) >= 0.5)
}

/// The grid's index buffer: per quad (i, j) the triangles (a, a+1, a+vc) and (a+1, a+vc+1, a+vc), a = j vc + i, quads in
/// strips of `strip()` columns, each strip top to bottom, row by row within it.
pub(super) fn grid_indices(cols: u32, rows: u32) -> Vec<u32> {
	let vc = cols + 1;
	let sw = if strip() == 0 { cols } else { strip() };
	let mut idx = Vec::with_capacity((cols * rows * 6) as usize);
	for s0 in (0..cols).step_by(sw.max(1) as usize) {
		for j in 0..rows {
			for i in s0..(s0 + sw).min(cols) {
				let a = j * vc + i;
				idx.extend_from_slice(&[a, a + 1, a + vc, a + 1, a + vc + 1, a + vc]);
			}
		}
	}
	idx
}

/// The smallest grid cell (px) of any look (`geo_look`: 4 / 6; the fallback 8): the buffer is sized for it.
const MIN_CELL: f32 = 4.0;

/// The grid (columns, rows) for a (w x h) px scene and a cell of `cell` px (`positions`; vertices = (columns + 1) (rows + 1)).
pub(super) fn grid_dims(w: f32, h: f32, cell: f32) -> (u32, u32) {
	let [mx, mb, mt] = GRID_MARGIN;
	(((w * (1.0 + mx) / cell).ceil() as u32).max(1), ((h * (1.0 + 0.5 * (mb + mt)) / cell).ceil() as u32).max(1))
}

/// WGSL appended to the water shader. Group 1 binding 2: the heights `cs_wave` writes; binding 3: the same buffer read by `vs_grid`.
pub(super) const GRID_WAVES_WGSL: &str = r#"
@group(1) @binding(2) var<storage, read_write> gh_out: array<f32>;
@group(1) @binding(3) var<storage, read> gh_in: array<f32>;
// Grid vertices per plane.
fn grid_n() -> u32 { return (u32(w.ggrid.x) + 1u) * (u32(w.ggrid.y) + 1u); }
// Grid vertex vi of plane gi: its view ray cast onto the plane w.geo[gi].x (a ray missing it: far out along its heading), its NDC
// and the world size of a band px there (`grid_wave`'s band limit).
struct GF { f: vec3<f32>, ndc: vec2<f32>, fpg: f32 };
fn grid_flat(vi: u32, gi: u32) -> GF {
	let vc = u32(w.ggrid.x) + 1u;
	let ndc = w.gridb.xy + vec2<f32>(f32(vi % vc), f32(vi / vc)) * w.gridb.zw;
	let z0 = w.geo[gi].x;
	let eye = w.eye.xyz;
	let a = w.ivp * vec4<f32>(ndc, 0.5, 1.0);
	let dir = normalize(a.xyz / a.w - eye);
	let down = z0 - eye.z;
	var f = vec3<f32>(eye.xy + normalize(dir.xy + vec2<f32>(1e-6, 0.0)) * GEO_FAR_T, z0);
	if (dir.z * down > 0.0) {
		let t = down / dir.z;
		if (t < GEO_FAR_T) { f = vec3<f32>(eye.xy + dir.xy * t, z0); }
	}
	let dist = max(length(f - eye), 1e-3);
	return GF(f, ndc, dist * 2.0 * w.proj.y * w.size.w * w.ggrid.w);
}
// RAC_TUNE=grid_cs=1: the vertex with its height from `cs_wave`.
@vertex fn vs_grid_cs(@builtin(vertex_index) vi: u32, @builtin(instance_index) gi: u32) -> GO {
	return grid_out(grid_flat(vi, gi), gh_in[gi * grid_n() + vi], gi);
}
// One thread per grid vertex (x) of plane y: its wave height (`grid_wave`, 0 for a dead vertex).
@compute @workgroup_size(64, 1, 1)
fn cs_wave(@builtin(global_invocation_id) id: vec3<u32>) {
	let n = grid_n();
	if (id.x >= n) { return; }
	let g = grid_flat(id.x, id.y);
	gh_out[id.y * n + id.x] = grid_wave(g.f, g.ndc, w.geo[id.y].x, id.y, g.fpg);
}
"#;

/// The compute pipeline, its layouts and the grid pass's pipeline layout (made once with the water pass).
pub(super) struct GridWaves {
	c0: wgpu::BindGroupLayout,
	out_l: wgpu::BindGroupLayout,
	in_l: wgpu::BindGroupLayout,
	pipe: wgpu::ComputePipeline,
	/// The grid pass's pipeline layout: the water group plus the dry tiles and the heights (group 1, bindings 0 and 3).
	pub(super) grid_pl: wgpu::PipelineLayout,
}

/// The heights buffer and its groups, made with the targets (sized for the window at the smallest cell).
pub(super) struct GridGroups {
	g_c0: wgpu::BindGroup,
	g_out: wgpu::BindGroup,
	pub(super) g_in: wgpu::BindGroup,
	/// Vertices per plane the buffer holds.
	cap: u32,
}

impl GridWaves {
	pub(super) fn new(device: &wgpu::Device, shader: &wgpu::ShaderModule, main: &wgpu::BindGroupLayout) -> GridWaves {
		let entry = |binding, ty, visibility| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
		let c = wgpu::ShaderStages::COMPUTE;
		let tex = |sample_type, multisampled| wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled };
		let store = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
		let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries });
		let uniform = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
		let c0 = layout("water sea waves scene", &[
			entry(0, uniform, c),
			entry(1, tex(wgpu::TextureSampleType::Depth, true), c),
			entry(10, tex(wgpu::TextureSampleType::Float { filterable: false }, false), c),
			entry(11, store(true), c),
		]);
		let out_l = layout("water sea waves out", &[entry(0, tex(wgpu::TextureSampleType::Uint, false), c), entry(2, store(false), c)]);
		let v = wgpu::ShaderStages::VERTEX;
		let in_l = layout("water sea waves in", &[entry(0, tex(wgpu::TextureSampleType::Uint, false), v), entry(3, store(true), v)]);
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water sea waves"), bind_group_layouts: &[Some(&c0), Some(&out_l)], immediate_size: 0 });
		let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("water sea waves"), layout: Some(&pl), module: shader, entry_point: Some("cs_wave"), compilation_options: Default::default(), cache: None });
		let grid_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water sea grid"), bind_group_layouts: &[Some(main), Some(&in_l)], immediate_size: 0 });
		GridWaves { c0, out_l, in_l, pipe, grid_pl }
	}

	/// The heights buffer for a (w x h) scene and the groups on it: `ubuf` = the water uniforms, `depth` = the scene depth (depth
	/// aspect), `seamask` = the sea mask, `shore` = the shore map, `tiles` = the dry tiles.
	pub(super) fn groups(&self, device: &wgpu::Device, (w, h): (u32, u32), (ubuf, shore): (&wgpu::Buffer, &wgpu::Buffer), depth: &wgpu::TextureView, seamask: &wgpu::TextureView, tiles: &wgpu::TextureView) -> GridGroups {
		let (cols, rows) = grid_dims(w as f32, h as f32, MIN_CELL);
		let cap = (cols + 1) * (rows + 1);
		let size = if compute_on() { (cap as u64) * (GEO_PLANES as u64) * 4 } else { 4 };
		let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("water sea waves"), size, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
		let group = |label, layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry]| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout, entries });
		GridGroups {
			g_c0: group("water sea waves scene", &self.c0, &[
				wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
				wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(seamask) },
				wgpu::BindGroupEntry { binding: 11, resource: shore.as_entire_binding() },
			]),
			g_out: group("water sea waves out", &self.out_l, &[
				wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(tiles) },
				wgpu::BindGroupEntry { binding: 2, resource: buf.as_entire_binding() },
			]),
			g_in: group("water sea waves in", &self.in_l, &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(tiles) }, wgpu::BindGroupEntry { binding: 3, resource: buf.as_entire_binding() }]),
			cap,
		}
	}

	/// The grid pipeline's vertex entry point.
	pub(super) fn vs_entry(&self) -> &'static str {
		if compute_on() { "vs_grid_cs" } else { "vs_grid" }
	}

	/// RAC_TUNE=grid_cs=1: the heights of `planes` planes of a (cols x rows) grid (after the dry tiles, before the grid pass).
	pub(super) fn dispatch(&self, enc: &mut wgpu::CommandEncoder, g: &GridGroups, (cols, rows): (u32, u32), planes: u32) {
		if !compute_on() {
			return;
		}
		let n = (cols + 1) * (rows + 1);
		debug_assert!(n <= g.cap && planes as usize <= GEO_PLANES);
		let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("water sea waves"), timestamp_writes: crate::viewer::passtime::cw("water sea waves") });
		cp.set_pipeline(&self.pipe);
		cp.set_bind_group(0, &g.g_c0, &[]);
		cp.set_bind_group(1, &g.g_out, &[]);
		cp.dispatch_workgroups(n.min(g.cap).div_ceil(64), planes.min(GEO_PLANES as u32), 1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Every quad's two triangles, each once, whatever the strip width.
	#[test]
	fn strips_keep_every_triangle() {
		let (cols, rows) = (13, 5);
		let mut got: Vec<[u32; 3]> = grid_indices(cols, rows).chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
		let vc = cols + 1;
		let mut want: Vec<[u32; 3]> = (0..rows).flat_map(|j| (0..cols).flat_map(move |i| {
			let a = j * vc + i;
			[[a, a + 1, a + vc], [a + 1, a + vc + 1, a + vc]]
		})).collect();
		got.sort();
		want.sort();
		assert_eq!(got, want);
	}

	/// The heights buffer is sized for MIN_CELL: every look's grid fits in it (`grid_dims` grows as the cell shrinks).
	#[test]
	fn every_look_fits_the_heights_buffer() {
		for level in 0..=u8::MAX {
			let cell = geo_look(level).map_or(8.0, |g| g.cell);
			assert!(cell >= MIN_CELL, "level {level}: cell {cell}");
			for (w, h) in [(1280.0, 720.0), (1920.0, 1080.0), (2560.0, 1440.0), (3840.0, 2160.0), (1281.0, 799.0)] {
				let (c, r) = grid_dims(w, h, cell);
				let (cm, rm) = grid_dims(w, h, MIN_CELL);
				assert!((c + 1) * (r + 1) <= (cm + 1) * (rm + 1));
			}
		}
	}
}
