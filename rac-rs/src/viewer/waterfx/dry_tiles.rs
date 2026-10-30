//! [PC] (speed) "Dry tiles" for the geometric sea grid. `vs_grid` calms each vertex's wave against level geometry standing in the
//! water (`edge_calm`: up to 56 depth + sea-mask loads per vertex, 90 % of the grid pass on an open sea). `edge_calm` returns exactly
//! 1.0 unless one of its samples (at most 42 px away) finds a "bank": a non-sea pixel of the level nearer than the sea plane. A
//! compute pass (`cs_dry`, once per frame before the grid) marks every 32 px tile that has such a pixel (one bit per sea plane,
//! looser test than `edge_calm`'s and without its hero exception, so a superset); `dry_near` asks the 43 px neighbourhood of a vertex
//! and `calm_edge` skips `edge_calm` when no tile in it is marked. Also marks where `fs_grid` can keep a fragment (the sea mask,
//! the hero): a grid vertex with no such tile near (`sea_near`) feeds no kept fragment and `vs_grid` skips its waves altogether. Same
//! picture, the skipped work was a no-op. [PC] (speed) The heights are made once per vertex by `cs_wave` (grid_waves.rs).

use super::*;

/// Tile size in px (`DRY_T` in the WGSL: one workgroup of 16 x 16 threads x 2 x 2 px).
const TILE: u32 = 32;

/// WGSL appended to the water shader. Group 1: `dry_in` (`cs_wave`) or `dry_out` (`cs_dry`), never both in one pipeline.
pub(super) const DRY_WGSL: &str = r#"
@group(1) @binding(0) var dry_in: texture_2d<u32>;
@group(1) @binding(1) var dry_out: texture_storage_2d<r32uint, write>;
const DRY_T: i32 = 32;
var<workgroup> dry_acc: atomic<u32>;

// One workgroup = one 32 px tile (16 x 16 threads, 2 x 2 px each: [PC] (speed) was 8 x 8 x 4 x 4, the same pixels and test; more
// threads hide the loads' latency on a small GPU). Bits 0..3 (plane k): a bank pixel (`edge_calm`'s test, looser, no
// hero exception); bits 4..7: `fs_grid` could keep a fragment whose flat point is a pixel of this tile for plane k (sea mask of
// the plane, or the hero's body in the water).
@compute @workgroup_size(16, 16, 1)
fn cs_dry(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) li: vec3<u32>, @builtin(local_invocation_index) lidx: u32) {
	if (lidx == 0u) { atomicStore(&dry_acc, 0u); }
	workgroupBarrier();
	let dim = vec2<i32>(textureDimensions(depth_ms));
	let base = vec2<i32>(wg.xy) * DRY_T + vec2<i32>(li.xy) * 2;
	var bits = 0u;
	for (var j = 0; j < 2; j++) {
		for (var i = 0; i < 2; i++) {
			let q = base + vec2<i32>(i, j);
			if (any(q >= dim)) { continue; }
			let m = textureLoad(seamask, q, 0).r;
			if (m != m) { bits |= 0xF0u; }
			if (m > -1e8) {
				for (var k = 0; k < 4; k++) {
					if (w.geo[k].y < 0.5) { break; } // (the planes are packed first: no plane k, none after it, `positions`; no reader asks its bits)
					if (abs(m - w.geo[k].x) <= GEO_MATCH) { bits |= 16u << u32(k); }
				}
				continue;
			}
			let zs = linear_depth(textureLoad(depth_ms, q, 0));
			if (w.hero.w >= 0.5) {
				for (var k = 0; k < 4; k++) {
					if (w.geo[k].y < 0.5) { break; }
					if (hero_pixel(q, zs, w.geo[k].x)) { bits |= 16u << u32(k); }
				}
			}
			if (zs >= w.proj.w * 0.98) { continue; }
			for (var k = 0; k < 4; k++) {
				if (w.geo[k].y < 0.5) { break; }
				if ((bits & (1u << u32(k))) != 0u) { continue; }
				let zp = plane_depth(vec2<f32>(q) + 0.5, w.geo[k].x);
				if (zs < zp * 0.9905 - 0.04) { bits |= 1u << u32(k); }
			}
		}
	}
	atomicOr(&dry_acc, bits);
	workgroupBarrier();
	if (lidx == 0u) { textureStore(dry_out, vec2<i32>(wg.xy), vec4<u32>(atomicLoad(&dry_acc), 0u, 0u, 0u)); }
}

// Does a tile within m px of px have bit `bit`? `edge`: a window past the screen reads the border tiles (`fs_grid` clamps the flat
// point's pixel), else it reads nothing (`edge_calm` skips samples off the screen).
fn dry_has(px: vec2<f32>, m: f32, bit: u32, edge: bool) -> bool {
	let nt = vec2<i32>(textureDimensions(dry_in));
	var lo = vec2<i32>(floor((px - m) / f32(DRY_T)));
	var hi = vec2<i32>(floor((px + m) / f32(DRY_T)));
	if (edge) {
		lo = clamp(lo, vec2<i32>(0), nt - vec2<i32>(1));
		hi = clamp(hi, vec2<i32>(0), nt - vec2<i32>(1));
	} else {
		lo = max(lo, vec2<i32>(0));
		hi = min(hi, nt - vec2<i32>(1));
	}
	for (var j = lo.y; j <= hi.y; j++) {
		for (var i = lo.x; i <= hi.x; i++) {
			if (((textureLoad(dry_in, vec2<i32>(i, j), 0).r >> bit) & 1u) != 0u) { return true; }
		}
	}
	return false;
}
// A sample of `edge_calm` (at most 42 px from px, 43 with the int cast) can find a bank of plane gi.
fn dry_near(px: vec2<f32>, gi: u32) -> bool { return dry_has(px, 43.0, gi, false); }
// A grid vertex at px can belong to a kept fragment of plane gi: `fs_grid` keeps only fragments whose flat point (a convex mix of
// the triangle's vertices, at most a cell diagonal from each, then truncated to a pixel) lies on the plane's sea mask or the hero.
// Else the vertex is dead: no kept fragment depends on its height.
fn sea_near(px: vec2<f32>, gi: u32) -> bool { return dry_has(px, w.ggrid.z * 2.0 + 2.0, gi + 4u, true); }
// The wave height of grid vertex (flat point f, ndc, plane z0 = instance gi); 0 for a dead vertex (see `sea_near`).
fn grid_wave(f: vec3<f32>, ndc: vec2<f32>, z0: f32, gi: u32, fpg: f32) -> f32 {
	if (!sea_near((ndc * vec2<f32>(0.5, -0.5) + 0.5) * w.size.xy, gi)) { return 0.0; }
	let am = calm_edge(f, ndc, z0, gi);
	if (am <= 0.001) { return 0.0; }
	return clamp(sea_w(f.xy, w.eye.w, fpg, fpg, geo_an(f - w.eye.xyz), i32(w.wave.x)).hg * am, -GEO_HMAX, GEO_HMAX);
}
"#;

/// The compute pipeline and the layouts around it (made once with the water pass).
pub(super) struct DryTiles {
	c0: wgpu::BindGroupLayout,
	out_l: wgpu::BindGroupLayout,
	pipe: wgpu::ComputePipeline,
	/// The grid's wave heights (`cs_wave`, which reads the tiles) and the grid pass's pipeline layout.
	pub(super) waves: GridWaves,
}

/// The tile texture's groups, made with the targets (they follow the window size).
pub(super) struct DryGroups {
	g_c0: wgpu::BindGroup,
	g_out: wgpu::BindGroup,
	pub(super) waves: GridGroups,
	tiles: (u32, u32),
}

impl DryTiles {
	pub(super) fn new(device: &wgpu::Device, shader: &wgpu::ShaderModule, main: &wgpu::BindGroupLayout) -> DryTiles {
		let entry = |binding, ty, visibility| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
		let c = wgpu::ShaderStages::COMPUTE;
		let tex = |sample_type, multisampled| wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled };
		let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries });
		let uniform = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
		let c0 = layout("water dry tiles scene", &[
			entry(0, uniform, c),
			entry(1, tex(wgpu::TextureSampleType::Depth, true), c),
			entry(10, tex(wgpu::TextureSampleType::Float { filterable: false }, false), c),
		]);
		let out_l = layout("water dry tiles out", &[entry(1, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::R32Uint, view_dimension: wgpu::TextureViewDimension::D2 }, c)]);
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water dry tiles"), bind_group_layouts: &[Some(&c0), Some(&out_l)], immediate_size: 0 });
		let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("water dry tiles"), layout: Some(&pl), module: shader, entry_point: Some("cs_dry"), compilation_options: Default::default(), cache: None });
		DryTiles { c0, out_l, pipe, waves: GridWaves::new(device, shader, main) }
	}

	/// The tile texture for a (w x h) scene and the groups on it: `ubuf` = the water uniforms, `depth` = the scene depth (depth aspect),
	/// `seamask` = the sea mask; plus the grid heights' (`shore` = the shore map).
	pub(super) fn groups(&self, device: &wgpu::Device, (w, h): (u32, u32), (ubuf, shore): (&wgpu::Buffer, &wgpu::Buffer), depth: &wgpu::TextureView, seamask: &wgpu::TextureView) -> DryGroups {
		let tiles = (w.div_ceil(TILE).max(1), h.div_ceil(TILE).max(1));
		let tex = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("water dry tiles"),
			size: wgpu::Extent3d { width: tiles.0, height: tiles.1, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::R32Uint,
			usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		});
		let view = tex.create_view(&Default::default());
		let group = |label, layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry]| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout, entries });
		DryGroups {
			g_c0: group("water dry tiles scene", &self.c0, &[
				wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
				wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(seamask) },
			]),
			g_out: group("water dry tiles out", &self.out_l, &[wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) }]),
			waves: self.waves.groups(device, (w, h), (ubuf, shore), depth, seamask, &view),
			tiles,
		}
	}

	/// Marks the tiles (after the sea mask, before the grid pass).
	pub(super) fn dispatch(&self, enc: &mut wgpu::CommandEncoder, g: &DryGroups) {
		let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("water dry tiles"), timestamp_writes: crate::viewer::passtime::cw("water dry tiles") });
		cp.set_pipeline(&self.pipe);
		cp.set_bind_group(0, &g.g_c0, &[]);
		cp.set_bind_group(1, &g.g_out, &[]);
		cp.dispatch_workgroups(g.tiles.0, g.tiles.1, 1);
	}
}
