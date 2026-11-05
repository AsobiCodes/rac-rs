//! [PC] Volumetric smoke (Pause -> Options -> Graphics -> Smoke Low / Medium / High with Lighting Enhanced; `smoke_option.rs`; no
//! retail counterpart). Off (the default) draws retail's smoke sprites exactly as before and nothing here runs.
//!
//! The pool's smoke sprites (the pack's rows, `Pack::smoke`) are not drawn as flat quads any more: each becomes a volume (a soft
//! sphere of eroded, billowing smoke; `plan.rs`, `volumes.rs`), a crowd of hundreds is merged into at most the quality's cap. A ray
//! march at a fraction of the picture (the fog's machinery: `volfog/scatter.rs` helpers, `temporal.rs` accumulation) integrates
//! the volumes in front of the scene into scattered light and transmittance: the sun (phase function, the cascaded shadow map, a
//! short march toward the sun for self shadowing), multi-scatter, a grey-blue ambient, and the warm glow of the fire lights near
//! (`partfx.rs` `FxLights`). The result is applied depth-aware over the picture before the fog (`shader.rs`).
//!
//! Quality (`quality`): Low = 10 steps, grid 1/3, one shadow tap, no sun shadow map, 24 volumes; Medium = 16 steps, grid 1/2, two
//! taps, 40 volumes; High = 24 steps, grid 1/2, three taps, 64 volumes. Look knobs: RAC_TUNE `smoke_*` (ENV.md), the shader text
//! RAC_SMOKE_WGSL=<file>.
//! Split view: `ready` gates with `splitview::fx`, so the Classic pass keeps retail's smoke sprites.
//! If the volumes cannot be drawn in a frame (`settle`), the held sprite quads are given back: the sprites show as in retail. Near
//! the range (`smoke_range`) a volume fades out while its puffs' sprites fade back in (`volumes::Reduced::band`).
//! A level fire's smoke (a burning building's grey columns and volume sheets) is not in the pool: the game draws it as alpha world
//! quads. The pack names them (`Pack::smoke_quads`); `columns.rs` takes them (the quad pass leaves them out while the volumes draw,
//! `took`) and `plume.rs` turns the quads of each emitter into one plume: puffs born at the fire that rise, widen, lean with the wind
//! and thin out at the top. Smoke Off: retail's quads.

use super::volfog::{self, frame, noise};
use super::*;

#[cfg(test)]
mod billow_tests;
pub(in crate::viewer) mod columns;
mod fire_cut;
mod flow;
pub(in crate::viewer) mod plan;
mod plume;
mod plume_track;
mod shader;
#[cfg(test)]
mod tests;
mod volumes;

pub(in crate::viewer) use plan::Plan;
use volumes::{Light, Raw, Track, Vol, MAX_VOLS};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The march's jitter counter wraps here like the fog's (volfog.rs `JITTER_PERIOD`).
const JITTER_PERIOD: u32 = 1000;

/// What a Smoke level costs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Quality {
	pub steps: f32,
	/// The grid's divisor of the half-size linear depth grid.
	pub div: u32,
	/// Taps of the short march toward the sun (self shadowing).
	pub taps: u32,
	/// Whether the sun's shadow map is read.
	pub sun_map: bool,
	/// Most volumes drawn.
	pub max: usize,
}

/// The settings of a Smoke level (0 Off, 1 Low, 2 Medium, anything else High); None = off.
pub(super) const fn quality(level: u8) -> Option<Quality> {
	match level {
		0 => None,
		1 => Some(Quality { steps: 10.0, div: 3, taps: 1, sun_map: false, max: 24 }),
		2 => Some(Quality { steps: 16.0, div: 2, taps: 2, sun_map: true, max: 40 }),
		_ => Some(Quality { steps: 24.0, div: 2, taps: 3, sun_map: true, max: MAX_VOLS }),
	}
}

/// The extra uniform block (`G` in the WGSL), field for field.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct Extra {
	pub tune: [f32; 4],
	pub tune2: [f32; 4],
	pub tune3: [f32; 4],
	pub amb: [f32; 4],
	/// Fire cores after the volumes (count), the shell round a core (times the core), the detail noise fade, the cores' occluder
	/// density (fire_cut.rs, shader.rs).
	pub cut: [f32; 4],
	/// The billow noise's offset (flow.rs) wrapped by `flow::WRAP`, and wrapped by the detail tile.
	pub flow: [f32; 4],
	pub flowd: [f32; 4],
}

struct Pipes {
	march: wgpu::RenderPipeline,
	resolve: wgpu::RenderPipeline,
	apply: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	noise: wgpu::TextureView,
	nsamp: wgpu::Sampler,
	lsamp: wgpu::Sampler,
	dummy: wgpu::TextureView,
}

struct Targets {
	key: (u32, u32, u64),
	cur: wgpu::TextureView,
	hist: [wgpu::TextureView; 2],
}

/// The inputs of one frame (the fog's, without the retail fog / floor / cover).
pub(super) struct Input<'a> {
	pub cam: &'a Camera,
	pub aspect: f32,
	pub scene: &'a wgpu::TextureView,
	pub lin: &'a wgpu::TextureView,
	pub generation: u64,
	pub size: (u32, u32),
	pub far: f32,
	pub sun_map: &'a wgpu::TextureView,
	pub sun_cmp: &'a wgpu::Sampler,
	pub sun: Option<&'a super::sunshadow::SunFrame>,
	pub fog_col: [f32; 3],
	pub light: [f32; 3],
	/// Seconds (f64: the flow's per-frame step stays exact after hours of play, review R29 m1).
	pub time: f64,
}

impl<'a> Input<'a> {
	/// The frame's inputs, or None when the post targets are not up. `at` = (level, picture width, height); `view` = (camera,
	/// aspect, far plane); `rows`: the level light rows.
	pub(super) fn new(post: &'a super::post::Post, sun_shadow: &'a super::sunshadow::SunShadow, sun: Option<&'a super::sunshadow::SunFrame>, u: &Uniforms, at: (usize, u32, u32), view: (&'a Camera, f32, f32), rows: &[[[f32; 4]; 4]; 16]) -> Option<Input<'a>> {
		let (scene, lin, _, generation) = post.water_inputs()?;
		Some(Input {
			cam: view.0,
			aspect: view.1,
			scene,
			lin,
			generation,
			size: (at.1, at.2),
			far: view.2,
			sun_map: &sun_shadow.array,
			sun_cmp: &sun_shadow.cmp,
			sun,
			fog_col: [u.fog_colour[0], u.fog_colour[1], u.fog_colour[2]],
			light: frame::key_colour(rows),
			time: frame::clock64(),
		})
	}
}

/// The pool's puffs and the smoke quads' volumes as one list (the puffs themselves when there are no quad volumes).
fn joined<'a>(pool: &'a [Raw], quads: &[Raw]) -> std::borrow::Cow<'a, [Raw]> {
	match quads.is_empty() {
		true => std::borrow::Cow::Borrowed(pool),
		false => std::borrow::Cow::Owned([pool, quads].concat()),
	}
}

pub(super) struct VolSmoke {
	format: wgpu::TextureFormat,
	pipes: Option<Pipes>,
	ubuf: Option<wgpu::Buffer>,
	gbuf: Option<wgpu::Buffer>,
	vbuf: Option<wgpu::Buffer>,
	t: Option<Targets>,
	/// The previous frame's camera rows, the frames since the history was valid, which history texture is the newer.
	prev: Option<[[f32; 4]; 4]>,
	frames: u32,
	flip: usize,
	/// The last `set_particles`' smoke volumes and the sprite quads held back for them.
	raw: Vec<Raw>,
	held: Vec<plan::Held>,
	/// The quads the last `settle` appended to `part_verts` (a second pass of the frame removes them before it decides again).
	given: Vec<(plan::Key, [Vertex; 4])>,
	/// The volumes of this frame's smoke quads (`columns::gather`), which effect quad each came from, and which quads they replace.
	qraw: Vec<Raw>,
	qsrc: Vec<u32>,
	qgrp: Vec<u32>,
	qgroups: Vec<Vec<u32>>,
	took: Vec<bool>,
	/// This pass's volumes (`settle`: the pool's and the quads' puffs reduced to the cap), the largest radius among them, what the
	/// reduction remembers between frames.
	vols: Vec<Raw>,
	reach: f32,
	track: Track,
	/// The billow noise's motion (one offset per frame, review R18 B2).
	flow: flow::Flow,
	/// The level fires' plumes followed from frame to frame (`columns::gather`).
	plumes: plume_track::PlumeTrack,
}

impl VolSmoke {
	pub(super) fn new(format: wgpu::TextureFormat) -> VolSmoke {
		VolSmoke { format, pipes: None, ubuf: None, gbuf: None, vbuf: None, t: None, prev: None, frames: 0, flip: 0, raw: Vec::new(), held: Vec::new(), given: Vec::new(), qraw: Vec::new(), qsrc: Vec::new(), qgrp: Vec::new(), qgroups: Vec::new(), took: Vec::new(), vols: Vec::new(), reach: 0.0, track: Track::default(), flow: flow::Flow::default(), plumes: plume_track::PlumeTrack::default() }
	}

	/// Takes over a frame's plan (`set_particles`, `part_verts` was just cleared): its volumes and held quads replace the last frame's.
	pub(in crate::viewer) fn finish(&mut self, plan: Plan) {
		(self.raw, self.held) = (plan.raw, plan.held);
		self.given.clear();
	}

	/// Takes the frame's smoke quads over (`rows` = the pack's, `quads` = the effect quads, `eye` = the camera; `on` as
	/// `columns::gather`): their plumes, followed from last frame. Called before `ready` / `settle`.
	pub(in crate::viewer) fn gather_quads(&mut self, rows: &[crate::smoke_rows::QuadRow], quads: &[crate::prims::WorldQuad], eye: [f32; 3], on: bool) {
		let g = columns::gather(rows, quads, eye, on, &mut self.plumes);
		self.set_quads(g);
	}

	/// Takes over the frame's smoke quads (`columns::gather`, called before `ready` / `settle`).
	pub(in crate::viewer) fn set_quads(&mut self, g: columns::Gathered) {
		(self.qraw, self.qsrc, self.took, self.qgrp, self.qgroups) = (g.raw, g.src, g.took, g.grp, g.groups);
	}

	/// Whether effect quad `i` became a volume this frame (the quad pass leaves it out; false whenever the volumes do not draw).
	pub(in crate::viewer) fn took(&self, i: usize) -> bool {
		self.took.get(i).copied().unwrap_or(false)
	}

	/// Whether this frame's smoke can be drawn as volumes: some are held or a plume fades out, the level is not Off, the shader was
	/// not refused.
	pub(super) fn ready(&self) -> bool {
		crate::splitview::fx(!self.raw.is_empty() || !self.took.is_empty() || !self.qraw.is_empty()) && crate::smoke_option::level() > 0 && !super::pipe_guard::off(super::pipe_guard::Effect::VolSmoke)
	}

	/// Start of `draw_passes`, once per pass (the split view runs two on one `finish`: the held list stays whole for the later pass; what
	/// this call gave back is removed again first). `on`: the volumes draw. The puffs are reduced to the quality's cap (`cam`: the eye);
	/// the sprites / quads of the puffs the cap cut go back to the particle pass / the quad pass, so a cut smoke shows as in retail
	/// (A22). Otherwise (`!on`) every held quad goes back (the sprites show as in retail; the order inside one run may differ) and
	/// the volumes are dropped.
	pub(super) fn settle(&mut self, on: bool, cam: &Camera, verts: &mut std::collections::BTreeMap<plan::Key, Vec<Vertex>>) {
		self.settle_cap(on, cam, Self::cap(), verts);
	}

	/// The most volumes this frame (the quality's, or the RAC_TUNE `smoke_max`); 0 = Smoke off.
	fn cap() -> usize {
		let Some(q) = quality(crate::smoke_option::level()) else { return 0 };
		match crate::devtune::f32("smoke_max", 0.0) as usize {
			0 => q.max,
			m => m.min(MAX_VOLS),
		}
	}

	/// `settle` with the cap given (the tests).
	fn settle_cap(&mut self, on: bool, cam: &Camera, cap: usize, verts: &mut std::collections::BTreeMap<plan::Key, Vec<Vertex>>) {
		for (key, quad) in self.given.drain(..) {
			if let Some(v) = verts.get_mut(&key) {
				super::firefx::remove_quad(v, &quad);
				if v.is_empty() {
					verts.remove(&key);
				}
			}
		}
		if on {
			let (cut, band) = self.reduce_frame(cam, cap);
			self.give_back(&cut, &band, verts);
			return;
		}
		self.vols.clear();
		for h in &self.held {
			verts.entry(h.key).or_default().extend(h.quad);
			self.given.push((h.key, h.quad));
		}
		(self.qraw, self.qsrc, self.took, self.qgrp, self.qgroups) = Default::default();
		if !crate::splitview::classic() {
			// (The Classic half of a split frame leaves the Enhanced half's history and its volumes' tracking alone, review R7-4.)
			self.track.forget();
			self.prev = None;
		}
	}

	/// The sprites and quads of the puffs `cut` (indices into the pool's puffs followed by the quads') go back to the passes; so do
	/// those of `band` (volumes fading out at the range with share w, review R7-2): a sprite at alpha x (1 - w), so the smoke hands over
	/// without a gap; a level quad whole (the quad pass has no fade: the fading volume over it reveals it).
	fn give_back(&mut self, cut: &[usize], band: &[(usize, f32)], verts: &mut std::collections::BTreeMap<plan::Key, Vec<Vertex>>) {
		let n = self.raw.len();
		// Per pool puff: the share of its sprite that comes back (0 = it stays out).
		let mut back = vec![0.0f32; n];
		// (A puff in the trail band keeps that share of its sprite whatever the volumes do: plan.rs `trail_share`.)
		self.held.iter().filter_map(|h| h.vol.map(|v| (v as usize, h.keep))).for_each(|(v, k)| back[v] = back[v].max(k));
		// A cut puff gets its whole sprite back; one fading out at the range (its volume drawn at share x w) the rest of it, so the
		// sprite and the volume still add up to one: keep + (1 - keep)(1 - w) (review R18 m1). The two lists never share a puff.
		for (c, w) in cut.iter().map(|&c| (c, 0.0)).chain(band.iter().copied()) {
			if c < n {
				back[c] += (1.0 - back[c]) * (1.0 - w);
			} else {
				// The whole plume's quads come back (the puff stands for all of them), not only the one its puff maps to.
				let all = self.qgrp.get(c - n).and_then(|&g| self.qgroups.get(g as usize));
				for &qi in all.map(Vec::as_slice).unwrap_or(&[]).iter().chain(self.qsrc.get(c - n)) {
					if let Some(t) = self.took.get_mut(qi as usize) {
						*t = false;
					}
				}
			}
		}
		for h in &self.held {
			let Some(k) = h.vol.and_then(|v| back.get(v as usize).copied()).filter(|&k| k > 0.0) else { continue };
			let mut quad = h.quad;
			if k < 1.0 {
				quad.iter_mut().for_each(|v| v.colour[3] *= k);
			}
			verts.entry(h.key).or_default().extend(quad);
			self.given.push((h.key, quad));
		}
	}

	/// The pass's volumes: the held puffs culled, merged to the quality's cap (`Quality::max`). Returns the puffs no volume shows and
	/// those of the volumes fading out at the range (`volumes::Reduced`).
	fn reduce_frame(&mut self, cam: &Camera, cap: usize) -> (Vec<usize>, Vec<(usize, f32)>) {
		let t = crate::devtune::f32;
		if cap == 0 {
			self.vols.clear();
			return (Vec::new(), Vec::new());
		}
		let all = joined(&self.raw, &self.qraw);
		let r = volumes::reduce(&all, (cam.pos, forward(cam), t("smoke_range", 150.0)), cap, t("smoke_cell_hold", 90.0) as u32, &mut self.track);
		self.reach = r.vols.iter().map(volumes::bound).fold(0.0, f32::max);
		self.vols = r.vols;
		(r.cut, r.band)
	}

	/// Warm-up (review R7-3): makes the pipelines and the noise on the first frame Smoke could draw (Low..High with the post targets),
	/// not when the first smoke comes into view mid-game. A driver that refuses the shader turns the smoke off (pipe_guard.rs).
	pub(super) fn warm(&mut self, device: &wgpu::Device, queue: &GatedQueue) {
		if self.pipes.is_some() || quality(crate::smoke_option::level()).is_none() {
			return;
		}
		self.pipes = super::pipe_guard::guard(device, super::pipe_guard::Effect::VolSmoke, || self.pipelines(device, queue));
	}

	/// The whole WGSL of the pass.
	fn pipelines(&self, device: &wgpu::Device, queue: &GatedQueue) -> Pipes {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("volsmoke"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("volsmoke", shader::source().into())) });
		let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None };
		let tex2 = |filterable: bool| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
		let uniform = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("volsmoke"),
			entries: &[
				entry(0, uniform),
				entry(1, tex2(false)),
				entry(2, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
				entry(3, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
				entry(4, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }),
				entry(5, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)),
				entry(6, tex2(true)),
				entry(7, tex2(true)),
				entry(8, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
				entry(9, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }),
				entry(10, uniform),
			],
		});
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("volsmoke"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		let make = |fs: &'static str, target: wgpu::TextureFormat, blend: Option<wgpu::BlendState>| {
			let mask = if fs == "fs_apply" { wgpu::ColorWrites::COLOR } else { wgpu::ColorWrites::ALL };
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(fs),
				layout: Some(&pl),
				vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
				fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: target, blend, write_mask: mask })] }),
				primitive: Default::default(),
				depth_stencil: None,
				multisample: Default::default(),
				multiview_mask: None,
				cache: None,
			})
		};
		// The picture times the transmittance (the fragment's alpha) plus the scattered light.
		let over = wgpu::BlendState {
			color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::SrcAlpha, operation: wgpu::BlendOperation::Add },
			alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::Zero, operation: wgpu::BlendOperation::Add },
		};
		let (noise, nsamp) = noise::upload(device, queue);
		let lsamp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("volsmoke linear"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
		let dummy = device
			.create_texture(&wgpu::TextureDescriptor {
				label: Some("volsmoke dummy"),
				size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: FORMAT,
				usage: wgpu::TextureUsages::TEXTURE_BINDING,
				view_formats: &[],
			})
			.create_view(&Default::default());
		Pipes { march: make("fs_smoke", FORMAT, None), resolve: make("fs_resolve", FORMAT, None), apply: make("fs_apply", self.format, Some(over)), layout, noise, nsamp, lsamp, dummy }
	}

	fn targets(device: &wgpu::Device, inp: &Input, div: u32) -> Targets {
		let (fw, fh) = (inp.size.0.div_ceil(2).div_ceil(div).max(1), inp.size.1.div_ceil(2).div_ceil(div).max(1));
		let make = |label| {
			device
				.create_texture(&wgpu::TextureDescriptor {
					label: Some(label),
					size: wgpu::Extent3d { width: fw, height: fh, depth_or_array_layers: 1 },
					mip_level_count: 1,
					sample_count: 1,
					dimension: wgpu::TextureDimension::D2,
					format: FORMAT,
					usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
					view_formats: &[],
				})
				.create_view(&Default::default())
		};
		Targets { key: (fw, fh, inp.generation), cur: make("volsmoke now"), hist: [make("volsmoke history 0"), make("volsmoke history 1")] }
	}

	/// The pass's volumes (`settle`) lit by the fire lights.
	fn volumes(&self, lights: &super::partfx::FxLights) -> Vec<Vol> {
		let ls: Vec<Light> = (0..lights.n.min(lights.l.len())).map(|k| ([lights.l[k][0][0], lights.l[k][0][1], lights.l[k][0][2]], lights.l[k][0][3], [lights.l[k][1][0], lights.l[k][1][1], lights.l[k][1][2]])).collect();
		volumes::to_vols(&self.vols, &ls)
	}

	/// The fog's uniform struct filled for the smoke (cam / prev rows, the sun's cascades, the march).
	fn uniform(&self, inp: &Input, q: &Quality, fresh: bool, fsize: (u32, u32)) -> volfog::Uniform {
		let t = crate::devtune::f32;
		let cam = volfog::cam_rows(inp.cam, inp.aspect);
		let mut u: volfog::Uniform = Zeroable::zeroed();
		u.cam = cam;
		u.prev = self.prev.unwrap_or(cam);
		let mut su: Uniforms = Zeroable::zeroed();
		super::sunshadow::write_uniforms(&mut su, inp.sun);
		(u.sun_vp, u.sun, u.sun_bias, u.sun_texel, u.sun_q) = (su.sun_vp, su.sun, su.sun_bias, su.sun_texel, su.sun_q);
		u.sun_q[3] = if q.sun_map && inp.sun.is_some() { 1.0 } else { 0.0 };
		if inp.sun.is_none() {
			u.sun = [-0.4, -0.3, -0.85, 0.0]; // no key light this frame: a fixed direction, no shadows
		}
		u.phase = [u.sun[0], u.sun[1], u.sun[2], 0.0];
		let lc = inp.light;
		u.fog_col = [inp.fog_col[0], inp.fog_col[1], inp.fog_col[2], 1.0];
		u.sun_col = [lc[0], lc[1], lc[2], 0.55];
		// u.wind stays 0: the smoke's noise moves by flow.rs (g.flow), not by the fog's wind.
		u.size = [fsize.0 as f32, fsize.1 as f32, 1.0 / fsize.0 as f32, 1.0 / fsize.1 as f32];
		let blend = t("smoke_blend", 0.30).clamp(0.05, 1.0);
		u.march = [q.steps, (t("smoke_range", 150.0) + self.reach).min(inp.far),(self.frames % JITTER_PERIOD) as f32, if fresh { 1.0 } else { blend }];
		u.proj = [inp.cam.tan_half * inp.aspect, inp.cam.tan_half, RETAIL_NEAR, inp.far];
		u.lin = [inp.size.0.div_ceil(2) as f32, inp.size.1.div_ceil(2) as f32, q.div as f32, 6.0];
		u
	}

	/// The smoke's own uniform block.
	fn extra(inp: &Input, q: &Quality, count: usize, cuts: usize, flow: ([f32; 4], [f32; 4])) -> Extra {
		let t = crate::devtune::f32;
		let luma = (0.2126 * inp.light[0] + 0.7152 * inp.light[1] + 0.0722 * inp.light[2]).clamp(0.0, 1.0);
		let k = 0.5 + 0.5 * luma;
		let amb = [(inp.fog_col[0] * 0.45 + 0.10) * k, (inp.fog_col[1] * 0.45 + 0.12) * k, (inp.fog_col[2] * 0.45 + 0.16) * k];
		Extra {
			tune: [t("smoke_k", 0.5), t("smoke_scale", 3.2).max(0.05), t("smoke_detail", 1.1).max(0.05), 0.0],
			tune2: [t("smoke_amb", 1.0), t("smoke_sun", 1.0), t("smoke_glow", 0.6), t("smoke_shadow_len", 2.5)],
			tune3: [0.0, count as f32, q.taps as f32, t("smoke_grey", 0.85)],
			amb: [amb[0], amb[1], amb[2], t("smoke_erode", 0.8)],
			cut: [cuts as f32, t("smoke_hollow", 2.4).max(1.01), t("smoke_detail_lod", 0.12).max(0.001), t("smoke_occ", 4.0).max(0.0)],
			flow: flow.0,
			flowd: flow.1,
		}
	}

	/// Marches, accumulates and applies the smoke into `inp.scene`. False (history dropped, nothing drawn, no GPU work) when no
	/// volume is in view. `lights`: the frame's effect lights (the fire's glow on the smoke); `cores`: the frame's flames and fireballs
	/// (firefx_vol.rs `core`; fire_cut.rs).
	pub(super) fn run(&mut self, device: &wgpu::Device, queue: &GatedQueue, enc: &mut wgpu::CommandEncoder, inp: &Input, lights: &super::partfx::FxLights, cores: &[([f32; 3], f32, f32)]) -> bool {
		let (Some(mut q), true) = (quality(crate::smoke_option::level()), inp.size.0 > 0) else { return false };
		let t = crate::devtune::f32;
		let (div, steps) = (t("smoke_div", 0.0) as u32, t("smoke_steps", 0.0));
		(q.div, q.steps) = (if div > 0 { div } else { q.div }, if steps >= 1.0 { steps } else { q.steps });
		let vols = self.volumes(lights);
		if vols.is_empty() {
			self.prev = None;
			return false;
		}
		let _s = super::profile::span("volsmoke");
		let fl = self.flow.advance(inp.time, &self.vols, flow::Knobs::tuned());
		if self.pipes.is_none() {
			// [PC] pipe_guard.rs: a driver that refuses the shader turns the smoke off (the sprites come back), it does not crash.
			let Some(p) = super::pipe_guard::guard(device, super::pipe_guard::Effect::VolSmoke, || self.pipelines(device, queue)) else { return false };
			self.pipes = Some(p);
		}
		let key_ok = self.t.as_ref().is_some_and(|t| t.key.2 == inp.generation && t.key.0 == inp.size.0.div_ceil(2).div_ceil(q.div) && t.key.1 == inp.size.1.div_ceil(2).div_ceil(q.div));
		if !key_ok {
			self.t = Some(Self::targets(device, inp, q.div));
			self.prev = None;
		}
		let Some(fsize) = self.t.as_ref().map(|t| (t.key.0, t.key.1)) else { return false };
		let fresh = self.prev.is_none();
		let buf = |slot: &mut Option<wgpu::Buffer>, label, size: u64, usage| slot.get_or_insert_with(|| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })).clone();
		let ubuf = buf(&mut self.ubuf, "volsmoke", std::mem::size_of::<volfog::Uniform>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
		let gbuf = buf(&mut self.gbuf, "volsmoke extra", std::mem::size_of::<Extra>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
		let vbuf = buf(&mut self.vbuf, "volsmoke volumes", ((MAX_VOLS + fire_cut::MAX_CUTS) * std::mem::size_of::<Vol>()) as u64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
		let u = self.uniform(inp, &q, fresh, fsize);
		let cuts = fire_cut::cuts(cores.iter().copied(), u.cam[0][..3].try_into().unwrap_or([0.0; 3]), u.cam[1][..3].try_into().unwrap_or([0.0; 3]), u.march[1], t("smoke_cut", 1.3));
		queue.write_buffer(&ubuf, 0, bytemuck::bytes_of(&u));
		queue.write_buffer(&gbuf, 0, bytemuck::bytes_of(&Self::extra(inp, &q, vols.len(), cuts.len(), fl)));
		let mut all = vols;
		all.extend_from_slice(&cuts);
		queue.write_buffer(&vbuf, 0, bytemuck::cast_slice(&all));
		let (Some(pipes), Some(tg)) = (&self.pipes, &self.t) else { return false };
		let (read, write) = (self.flip, 1 - self.flip);
		let group = |cur: &wgpu::TextureView, hist: &wgpu::TextureView| {
			device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("volsmoke"),
				layout: &pipes.layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(inp.lin) },
					wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&pipes.noise) },
					wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&pipes.nsamp) },
					wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(inp.sun_map) },
					wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(inp.sun_cmp) },
					wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(cur) },
					wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(hist) },
					wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&pipes.lsamp) },
					wgpu::BindGroupEntry { binding: 9, resource: vbuf.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 10, resource: gbuf.as_entire_binding() },
				],
			})
		};
		let groups = [group(&pipes.dummy, &pipes.dummy), group(&tg.cur, &tg.hist[read]), group(&tg.hist[write], &pipes.dummy)];
		let pass = |enc: &mut wgpu::CommandEncoder, label: &str, target: &wgpu::TextureView, pipeline: &wgpu::RenderPipeline, g: &wgpu::BindGroup, load: wgpu::LoadOp<wgpu::Color>| {
			let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some(label),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: None,
				timestamp_writes: crate::viewer::passtime::rw(label),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			p.set_pipeline(pipeline);
			p.set_bind_group(0, g, &[]);
			p.draw(0..3, 0..1);
		};
		let clear = super::post::overwrite();
		pass(enc, "volsmoke march", &tg.cur, &pipes.march, &groups[0], clear);
		pass(enc, "volsmoke accumulate", &tg.hist[write], &pipes.resolve, &groups[1], clear);
		pass(enc, "volsmoke apply", inp.scene, &pipes.apply, &groups[2], wgpu::LoadOp::Load);
		self.prev = Some(volfog::cam_rows(inp.cam, inp.aspect));
		self.flip = write;
		self.frames = self.frames.wrapping_add(1);
		true
	}
}
