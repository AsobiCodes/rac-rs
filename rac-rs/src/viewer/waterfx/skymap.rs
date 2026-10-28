//! [PC] Water Enhanced / Water Low..High (Split view: inside the water pass, `DrawFx.water_on`): where the sun can reach the water. The sunlight effects of the water (light caustics on
//! the bed, the underwater caustics and light shafts, the sun glint) must not show where a roof, a bridge or a cliff keeps the
//! sun off (user 2026-10-24, YouTuber video: "underwater lightshafts appear in places which are completely shaded").
//!
//! A top-down height map of the level (a "sky map"): the static world (terrain, ties, tfrags) drawn once from straight above
//! with an orthographic view into a depth target 768 x 768 texels (`HALF` = 144 world units either side of the window centre),
//! both faces (a ceiling modelled as one face seen from below still counts). Its depth is the height of the highest surface at
//! each texel (a solid column: everything below it is covered). It is redrawn only when the camera has left the middle of the
//! window, the level changed or the height range no longer fits (every few dozen units of swimming), never per frame.
//! The shaders (`WGSL`, `sky_vis`) march a ray from a point on the water surface towards the level's sun (the key light of row 0
//! raised to at least 60 degrees, `steep`: only a roof, a bridge or a close wall counts, not a far cliff) through the map: the share of the sun that gets past is 0..1. With no
//! map yet, no sun, or a point outside the window the answer is 1 (today's look).

use super::*;
use std::cell::Cell;

/// Texels across the sky map and its half width in world units.
pub(in crate::viewer) const N: u32 = 768;
const HALF: f32 = 144.0;
/// The window centre snaps to this grid and is moved when the camera is this far from it.
const SNAP: f32 = 32.0;
const MOVE: f32 = 40.0;
/// Heights: the window reaches `UP` above the camera and `DOWN` below it.
const UP: f32 = 250.0;
const DOWN: f32 = 200.0;

/// The shaders' part: the map's depth (SKYD) and window (SKYU) bindings and `sky_vis`. `a` = (centre x, y, half width, built),
/// `b` = (top of the height range, range, texels across), `c` = (to the sun xyz, 1 with a sun).
const WGSL: &str = r#"
struct SkyU { a: vec4<f32>, b: vec4<f32>, c: vec4<f32> };
@group(0) @binding(SKYD) var sky_depth: texture_depth_multisampled_2d;
@group(0) @binding(SKYU) var<uniform> sky: SkyU;
// The share (0..1) of the sun that reaches the world point `p` (a point of the water surface) along the sky map's sun ray; `jit` 0..1
// moves the sample positions (a different value per pixel hides the steps).
fn sky_vis(p: vec3<f32>, jit: f32) -> f32 {
	let l = sky.c.xyz;
	if (sky.a.w < 0.5 || sky.c.w < 0.5 || l.z < 0.08) { return 1.0; }
	let n = sky.b.z;
	let hz = max(length(l.xy), 1e-3);
	let len = min((sky.b.x - p.z) / l.z, 90.0 / hz);
	if (len < 0.7) { return 1.0; }
	var occ = 0.0;
	for (var i = 0; i < 10; i++) {
		let f = (f32(i) + jit) / 10.0;
		let q = p + l * (0.5 + (len - 0.5) * f * f);
		let ux = ((q.x - sky.a.x) / sky.a.z + 1.0) * 0.5 * n;
		let uy = (1.0 - (q.y - sky.a.y) / sky.a.z) * 0.5 * n;
		if (ux < 0.0 || uy < 0.0 || ux >= n || uy >= n) { continue; }
		let d = textureLoad(sky_depth, vec2<i32>(i32(ux), i32(uy)), 0);
		if (d >= 1.0) { continue; }
		occ = max(occ, smoothstep(0.3, 1.3, (sky.b.x - d * sky.b.y) - q.z));
	}
	return 1.0 - occ;
}
"#;

/// `WGSL` for a shader that has the map's depth at binding `d` and its window uniform at binding `u` of group 0.
pub(in crate::viewer) fn wgsl(d: u32, u: u32) -> String {
	WGSL.replace("SKYD", &d.to_string()).replace("SKYU", &u.to_string())
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SkyUniform {
	a: [f32; 4],
	b: [f32; 4],
	c: [f32; 4],
}

/// The map's MSAA targets (about 28 MB with a 32-bit depth and stencil): made on the first frame water is in view, not for everyone.
pub(super) struct SkyTargets {
	colour: wgpu::TextureView,
	depth: wgpu::TextureView,
	depth_only: wgpu::TextureView,
}

pub(in crate::viewer) struct SkyMap {
	/// A 1x1 stand-in depth (same layout) for the water groups made before the real targets: they are rebuilt when `made` turns true
	/// (waterfx `Targets::key`), and the map is not read before then (its uniform is still zero).
	stub: wgpu::TextureView,
	formats: (wgpu::TextureFormat, wgpu::TextureFormat),
	real: std::cell::OnceCell<SkyTargets>,
	ubuf: wgpu::Buffer,
	/// The top-down pass's level uniforms (buffer and group 0).
	level: std::cell::OnceCell<(wgpu::Buffer, wgpu::BindGroup)>,
	/// The window as built: (level, world verts), centre x, y, camera height.
	built: Cell<Option<((usize, usize), f32, f32, f32)>>,
}

impl SkyMap {
	pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> SkyMap {
		let stub = make_target(device, "sky map stand-in", depth_format, 1, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT);
		let stub = stub.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("sky map window"), size: std::mem::size_of::<SkyUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		SkyMap { stub, formats: (format, depth_format), real: std::cell::OnceCell::new(), ubuf, level: std::cell::OnceCell::new(), built: Cell::new(None) }
	}

	/// The real targets, made on the first call.
	pub(super) fn ensure(&self, device: &wgpu::Device) -> &SkyTargets {
		self.real.get_or_init(|| {
			let colour = make_target(device, "sky map colour", self.formats.0, N, wgpu::TextureUsages::RENDER_ATTACHMENT);
			let depth = make_target(device, "sky map depth", self.formats.1, N, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
			let depth_only = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
			SkyTargets { colour, depth, depth_only }
		})
	}

	/// The real targets exist (the water groups hold their view from then on).
	pub(super) fn made(&self) -> bool {
		self.real.get().is_some()
	}

	/// The map's depth as the shaders read it (binding of `wgsl`'s SKYD) and its window uniform (SKYU).
	pub(in crate::viewer) fn depth_view(&self) -> &wgpu::TextureView {
		self.real.get().map_or(&self.stub, |r| &r.depth_only)
	}

	pub(in crate::viewer) fn uniform(&self) -> &wgpu::Buffer {
		&self.ubuf
	}
}

/// One `side` x `side` MSAA target of the sky map (`N` for the real ones, 1 for the stand-in).
fn make_target(device: &wgpu::Device, label: &str, format: wgpu::TextureFormat, side: u32, usage: wgpu::TextureUsages) -> wgpu::TextureView {
	device
		.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: MSAA, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
		.create_view(&Default::default())
}

impl WaterFx {
	pub(in crate::viewer) fn sky(&self) -> &SkyMap {
		&self.sky
	}
}

/// The sun for the water's light effects: no lower than `RISE` above the horizon. A low sun would shade a whole canyon river behind
/// its cliff; the effects only need to know about a roof, a bridge or a close wall.
const RISE: f32 = 0.87;
fn steep(d: [f32; 3]) -> [f32; 3] {
	let h = (d[0] * d[0] + d[1] * d[1]).sqrt();
	if -d[2] >= RISE || h < 1e-6 {
		return d;
	}
	let k = (1.0 - RISE * RISE).sqrt() / h;
	[d[0] * k, d[1] * k, -RISE]
}

/// Column-major orthographic view from straight above: x, y of the window to -1..1, height `zmax` to depth 0 and `zmax - range` to 1.
fn ortho(cx: f32, cy: f32, zmax: f32, range: f32) -> [[f32; 4]; 4] {
	[[1.0 / HALF, 0.0, 0.0, 0.0], [0.0, 1.0 / HALF, 0.0, 0.0], [0.0, 0.0, -1.0 / range, 0.0], [-cx / HALF, -cy / HALF, zmax / range, 1.0]]
}

impl Gpu {
	/// Keeps the sky map for the camera at `pos`: redraws it when needed (see the module doc) and writes the window and the sun
	/// into its uniform. Call each frame a water look is on screen (`on`), before the water pass and the underwater pass.
	pub(in crate::viewer) fn sky_build(&self, enc: &mut wgpu::CommandEncoder, scene: &Scene, u: &Uniforms, pos: [f32; 3], on: bool) {
		if !on {
			return;
		}
		let sm = self.waterfx.sky();
		let key = (self.level, scene.world_verts.len());
		let fits = sm.built.get().is_some_and(|(k, cx, cy, cz)| k == key && (pos[0] - cx).abs() < MOVE && (pos[1] - cy).abs() < MOVE && (pos[2] - cz).abs() < 100.0);
		if !fits {
			let (cx, cy) = ((pos[0] / SNAP).round() * SNAP, (pos[1] / SNAP).round() * SNAP);
			sm.built.set(Some((key, cx, cy, pos[2])));
			self.sky_draw(enc, scene, u, cx, cy, pos[2] + UP);
		}
		let Some((_, cx, cy, cz)) = sm.built.get() else { return };
		// The sun: the level's own light (row 0), raised as the dynamic shadows raise it; towards the sun = against its travel.
		let sun = super::sunshadow::key_light(&self.sun_rows(), 0, 0, 0.0).filter(|k| k.0[2] < -0.05).map(|k| steep(super::sunshadow::raise(k.0)));
		let to = sun.map_or([0.0, 0.0, 1.0], |d| [-d[0], -d[1], -d[2]]);
		let w = SkyUniform { a: [cx, cy, HALF, 1.0], b: [cz + UP, UP + DOWN, N as f32, 0.0], c: [to[0], to[1], to[2], sun.is_some() as u32 as f32] };
		self.queue.write_buffer(sm.uniform(), 0, bytemuck::bytes_of(&w));
	}

	/// The top-down pass: world, ties and tfrags, both faces, into the sky map's depth.
	fn sky_draw(&self, enc: &mut wgpu::CommandEncoder, scene: &Scene, u: &Uniforms, cx: f32, cy: f32, zmax: f32) {
		let sm = self.waterfx.sky();
		let real = sm.ensure(&self.device);
		let (ubuf, bind) = sm.level.get_or_init(|| {
			let ubuf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("sky map level uniforms"), size: std::mem::size_of::<Uniforms>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
			let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("sky map level uniforms"), layout: &self.pipeline.get_bind_group_layout(0), entries: &self.sun.entries(&ubuf) });
			(ubuf, bind)
		});
		let mut mu = *u;
		mu.view_proj = ortho(cx, cy, zmax, UP + DOWN);
		mu.sun = [0.0; 4];
		self.queue.write_buffer(ubuf, 0, bytemuck::bytes_of(&mu));
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("sky map"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &real.colour, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(scene.clear), store: wgpu::StoreOp::Discard } })],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
				view: &real.depth,
				depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
				stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(super::shadowvol::STENCIL_CLEAR), store: wgpu::StoreOp::Discard }),
			}),
			timestamp_writes: crate::viewer::passtime::rw("sky map"),
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_bind_group(0, bind, &[]);
		let array = self.bindless.as_ref().zip(scene.tex_array.as_ref());
		// Everything static inside the window, not what this frame's culling kept: the roof of a cave behind the camera or above
		// its view is no less a roof (`Scene::ties.draws` / `tfrags.draws` hold only the fragments in the camera's view).
		let near = |c: [f32; 3], r: f32| (c[0] - cx).abs() < HALF + r && (c[1] - cy).abs() < HALF + r;
		let mut tie_all: Vec<&Draw> = Vec::new();
		for (i, c) in scene.tie_cull.iter().enumerate().filter(|(_, c)| near(c.centre, c.radius)) {
			tie_all.extend(&scene.tie_draws[scene.tie_ranges[i][0].start as usize..scene.tie_ranges[i][0].end as usize]);
		}
		let mut tf_all: Vec<&Draw> = Vec::new();
		for (i, t) in scene.tfrag_set.iter().enumerate().filter(|(_, t)| near(t.cull.centre, t.cull.radius)) {
			tf_all.extend(&scene.tfrag_draws[scene.tfrag_ranges[i][0].start as usize..scene.tfrag_ranges[i][0].end as usize]);
		}
		let world_all: Vec<&Draw> = scene.world.draws.iter().collect();
		for (pipeline, batch, draws) in [(&self.pipeline, &scene.world, world_all), (&self.pipeline, &scene.ties, tie_all), (&self.tfrag_pipeline, &scene.tfrags, tf_all)] {
			pass.set_vertex_buffer(0, batch.vbuf.slice(..));
			pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
			if let Some((bl, tex_array)) = array.filter(|_| std::ptr::eq(batch, &scene.ties)) {
				pass.set_bind_group(1, tex_array, &[]);
				for d in draws.iter().copied() {
					let inst = d.texture as u32..d.texture as u32 + 1;
					if d.cull == 0 {
						pass.set_pipeline(&bl.pipeline);
						pass.draw_indexed(d.start..d.start + d.count, 0, inst);
					} else {
						for p in &bl.pipeline_cull {
							pass.set_pipeline(p);
							pass.draw_indexed(d.start..d.start + d.count, 0, inst.clone());
						}
					}
				}
				continue;
			}
			for d in draws.iter().copied() {
				pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
				if d.cull == 0 {
					pass.set_pipeline(pipeline);
					pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
				} else {
					for p in &self.pipeline_cull {
						pass.set_pipeline(p);
						pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
					}
				}
			}
		}
	}
}
