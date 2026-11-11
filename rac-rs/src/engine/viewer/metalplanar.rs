//! [PC] Metal On: the planar reflection of flat metal floors (a child of metalenv.rs; nothing here runs with Metal Off, Lighting
//! Classic or a pack without metal textures). The screen-space walk (metalfx.wgsl `metal_ssr`) can only reflect what is already on
//! screen: on a metal floor it tore the hero's reflection into blocky pieces, drew posts as long streaks and flickered while
//! walking (user 2026-11-03). So, as the water mirror (mirror.rs), the level is drawn a second time, mirrored about the height of
//! the flat metal floor most of the view looks at, at a divisor of the render size (metal_detail.rs: half at Full, the engine default;
//! the R&C1 row's High / Medium / Low a quarter, a sixth, an eighth), before the level pass; its colour gets a short mip chain (glossy blur by roughness), and the metal pixels lying on that plane read it instead of
//! the screen walk (metalfx.wgsl `metal_planar`). No such floor in view = no pass.
//!
//! - `Floors` (per level, built with the scene): the heights of the level's flat (normal within 5 degrees of up) metal faces
//!   (world, ties, terrain; the textures the pack marks as metal) in 0.25 bins, each with the 2 x 2 cells it covers.
//! - `Floors::pick`: of a 9 x 9 grid of rays through the screen, the height whose cells the most rays meet within `REACH`;
//!   last frame's height stays unless another meets more than `KEEP_X` times its rays plus `KEEP` (a room of two floors a little
//!   apart in height, Blarg's hangar, flipped between them while the camera turned: agent 15). After a change the planar weight
//!   fades in over `FADE` seconds (`fade`), so a change never pops.

use super::*;
use std::collections::HashSet;

/// Floor cell size (world units).
const CELL: f32 = 2.0;
/// Up-facing: the face normal's z at least cos 5 degrees.
const COS_UP: f32 = 0.9962;
/// Height bins per unit.
const BINS: f32 = 4.0;
/// Least metal floor area of a height (a small plate is no floor).
const MIN_AREA: f32 = 6.0;
/// Rays meeting a floor further than this (world units) do not count.
const REACH: f32 = 70.0;
/// Rays a new height needs over `KEEP_X` times the kept one's.
const KEEP: u32 = 3;
const KEEP_X: u32 = 2;
/// Seconds the planar weight takes to fade in after a floor change.
const FADE: f32 = 0.2;
/// Mip levels of the planar colour (glossy blur).
pub(super) const PL_MIPS: u32 = 4;

/// How a batch's face keys name their texture (scene_build.rs: `batch`, `batch_instances`, the terrain rule).
#[derive(Clone, Copy)]
pub(crate) enum Keys {
	World,
	Instance,
	Terrain,
}

/// The level's flat metal floors: (height, cells covered) per height bin.
#[derive(Default)]
pub(crate) struct Floors {
	levels: Vec<(f32, HashSet<(i32, i32)>)>,
}

fn tex_of(key: u32, k: Keys) -> u32 {
	match k {
		Keys::World => key & crate::mobies::KEY_MASK,
		Keys::Instance if crate::shrubs::key_k(key).is_some() => key & crate::mobies::KEY_MASK & !crate::shrubs::MATERIAL_BITS,
		Keys::Instance => key & crate::mobies::KEY_MASK,
		Keys::Terrain => key & 0xff,
	}
}

impl Floors {
	/// The flat metal faces of `groups` (meshes with their key kind); `textures` gives the metal verdict (metalfx.rs `texture_flag`).
	pub(crate) fn build(groups: &[(&[&Mesh], Keys)], textures: &HashMap<u32, Texture>) -> Floors {
		let mut flags: HashMap<u32, bool> = HashMap::new();
		let tris = groups.iter().flat_map(|(ms, k)| ms.iter().flat_map(move |m| m.faces.iter().map(move |f| (m, f, *k))));
		Floors::from_faces(tris.filter_map(|(m, &(key, f), k)| {
			let tex = tex_of(key, k);
			let metal = *flags.entry(tex).or_insert_with(|| super::metalfx::texture_flag(textures.get(&tex)) != 0);
			let p = f.map(|i| m.positions.get(i as usize).copied());
			(metal && p.iter().all(|q| q.is_some())).then(|| p.map(|q| q.unwrap_or_default()))
		}))
	}

	/// The floors of the triangles given (metal ones only).
	pub(crate) fn from_faces(tris: impl Iterator<Item = [[f32; 3]; 3]>) -> Floors {
		let mut bins: HashMap<i32, (f64, f64, HashSet<(i32, i32)>)> = HashMap::new();
		for [a, b, c] in tris {
			let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
			let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
			let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
			if len < 1e-6 || n[2].abs() < COS_UP * len {
				continue;
			}
			let (area, z) = (0.5 * len, (a[2] + b[2] + c[2]) / 3.0);
			let e = bins.entry((z * BINS).round() as i32).or_default();
			e.0 += (area * z) as f64;
			e.1 += area as f64;
			let cell = |x: f32| (x / CELL).floor() as i32;
			let (x0, x1) = (cell(a[0].min(b[0]).min(c[0])), cell(a[0].max(b[0]).max(c[0])));
			let (y0, y1) = (cell(a[1].min(b[1]).min(c[1])), cell(a[1].max(b[1]).max(c[1])));
			if (x1 - x0) * (y1 - y0) > 4096 {
				continue;
			}
			for x in x0..=x1 {
				for y in y0..=y1 {
					e.2.insert((x, y));
				}
			}
		}
		let mut levels: Vec<(f32, HashSet<(i32, i32)>)> = bins.into_values().filter(|b| b.1 >= MIN_AREA as f64).map(|b| ((b.0 / b.1) as f32, b.2)).collect();
		levels.sort_by(|a, b| a.0.total_cmp(&b.0));
		Floors { levels }
	}

	/// The floor height to mirror this frame (see the module doc): `eye` the camera, `ivp` the inverse view-projection (column-major).
	pub(crate) fn pick(&self, eye: [f32; 3], ivp: &[[f32; 4]; 4], prev: Option<f32>) -> Option<f32> {
		const GRID: usize = 9;
		if self.levels.is_empty() {
			return None;
		}
		let rays: Vec<[f32; 3]> = (0..GRID * GRID)
			.filter_map(|i| {
				let step = |k: usize| -0.95 + 1.9 * k as f32 / (GRID - 1) as f32;
				let c = [step(i % GRID), step(i / GRID), 0.5, 1.0];
				let p: [f32; 4] = std::array::from_fn(|r| (0..4).map(|k| ivp[k][r] * c[k]).sum());
				(p[3].abs() > 1e-6).then(|| [p[0] / p[3] - eye[0], p[1] / p[3] - eye[1], p[2] / p[3] - eye[2]])
			})
			.filter(|d| d[2] < 0.0)
			.collect();
		let counts: Vec<(u32, f32)> = self
			.levels
			.iter()
			.filter(|l| eye[2] > l.0 + 0.3)
			.map(|(h, cells)| {
				let on = rays
					.iter()
					.filter(|d| {
						let t = (h - eye[2]) / d[2];
						let (x, y) = (eye[0] + d[0] * t, eye[1] + d[1] * t);
						t * (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() <= REACH && cells.contains(&((x / CELL).floor() as i32, (y / CELL).floor() as i32))
					})
					.count() as u32;
				(on, *h)
			})
			.collect();
		let best = counts.iter().filter(|c| c.0 >= 2).max_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)))?;
		let kept = prev.and_then(|h| counts.iter().find(|c| (c.1 - h).abs() < 0.01 && c.0 >= 2));
		Some(match kept {
			Some(k) if k.0 * KEEP_X + KEEP > best.0 => k.1,
			_ => best.1,
		})
	}
}

/// The planar targets at the Metal reflection detail's size (metal_detail.rs `planar_size`): MSAA colour and depth, the resolved colour (all mips bound to the level
/// shader at binding 9, level 0 the resolve target) and per level 1..: (level below, target level).
pub(super) struct Planar {
	ms: wgpu::TextureView,
	depth: wgpu::TextureView,
	pub(super) all: wgpu::TextureView,
	l0: wgpu::TextureView,
	mips: Vec<(wgpu::BindGroup, wgpu::TextureView)>,
	/// The contact weights into level 0's alpha (metalcontact.rs), from `depth`.
	contact: super::contact::Contact,
	/// The reflection source size it was made for and its Metal reflection detail (metal_detail.rs).
	made_for: ((u32, u32), crate::viewer::metal_detail::Detail),
}

impl Planar {
	/// The targets for the reflection source size `src` at the Metal reflection detail `d` (metal_detail.rs `planar_size`: Full =
	/// `src` with `PL_MIPS` levels, the v0.5 targets; the others smaller with fewer levels). `real`: the mip and contact pipelines.
	pub(super) fn new(device: &wgpu::Device, real: &Real, samp: &wgpu::Sampler, (format, depth_format): (wgpu::TextureFormat, wgpu::TextureFormat), src: (u32, u32), d: crate::viewer::metal_detail::Detail) -> Planar {
		let ((w, h), pl_mips) = crate::viewer::metal_detail::planar_size(src, PL_MIPS, d);
		let ra = wgpu::TextureUsages::RENDER_ATTACHMENT;
		let ms = make_tex(device, "metal planar msaa", { let (w, h) = crate::viewer::scene_msaa::ms_size(w, h); (w, h, 1) }, 1, crate::viewer::targets::msaa(), format, ra | crate::viewer::scene_msaa::copy_usage()).create_view(&Default::default());
		let dt = make_tex(device, "metal planar depth", (w, h, 1), 1, crate::viewer::targets::msaa(), depth_format, ra | wgpu::TextureUsages::TEXTURE_BINDING);
		let (depth, contact) = (dt.create_view(&Default::default()), super::contact::Contact::new(device, &dt, &real.contact));
		let t = make_tex(device, "metal planar", (w, h, 1), pl_mips, 1, format, ra | wgpu::TextureUsages::TEXTURE_BINDING | crate::viewer::scene_msaa::copy_usage());
		let mips = (1..pl_mips).map(|m| mip_step(device, &real.mip_layout, samp, &t, 0, m)).collect();
		Planar { ms, depth, all: t.create_view(&Default::default()), l0: layer(&t, 0, 0), mips, contact, made_for: (src, d) }
	}

	/// The shader's mip pick goes down this many levels (u.metal[13].w, metalfx.wgsl `metal_planar`): 0 at Full, the v0.5 uniforms.
	pub(super) fn lod_bias(&self) -> f32 {
		self.made_for.1.lod_bias()
	}
}

impl MetalEnv {
	/// From `alloc`, after the reflection source exists: (re)make the planar targets when there are none, the source changed size or
	/// the Metal reflection detail row changed (metal_detail.rs; read each frame, so the row applies on the next frame). True = made:
	/// the level pass's group (binding 9) must be rebound, and the mirror pass's group is remade on its next pass.
	pub(super) fn alloc_planar(&mut self) -> bool {
		let (Some((src, _)), Some(real)) = (self.ssr.as_ref(), self.real.get()) else { return false };
		let want = (*src, crate::viewer::metal_detail::get());
		if self.planar.as_ref().is_some_and(|p| p.made_for == want) {
			return false;
		}
		self.planar = Some(Planar::new(&self.device, real, &self.samp, (self.format, self.depth_format), want.0, want.1));
		self.planar_mirror.get_mut().take();
		true
	}
}

/// RAC_TUNE `metal_planar` (1 = on, 0 = the screen walk alone, as v8).
fn planar_on() -> bool {
	knob("metal_planar", 1.0) > 0.0
}

impl MetalEnv {
	/// The planar floor mirror's height of the frame (None without one): gpu/mirror_cull.rs culls the tfrags it can show.
	pub(in crate::viewer) fn planar_height(&self) -> Option<f32> {
		self.plane.get()
	}

	/// After `write_uniforms`: this frame's floor (`Floors::pick` from the camera `eye` and the level's view-projection), kept for
	/// `Gpu::metal_planar`, and u.metal[12] = (on, height, planar gloss share, blur in mips per roughness). Off, no targets or no
	/// floor in view: zero (the shader keeps the screen walk) and no pass.
	pub(in crate::viewer) fn plan_planar(&self, u: &mut Uniforms, floors: &Floors, eye: [f32; 3], warp: bool) {
		let on = u.keep_tail[1] > 0.0 && !warp && planar_on() && self.planar.is_some();
		let prev = self.plane.get();
		let h = on.then(|| super::mirror::inverse(&u.view_proj).and_then(|ivp| floors.pick(eye, &ivp, prev))).flatten();
		self.plane.set(h);
		let now = std::time::Instant::now();
		let (w0, t0) = self.planar_fade.get();
		let w = fade(prev, h, w0, t0.map_or(0.0, |t| now.duration_since(t).as_secs_f32()));
		self.planar_fade.set((w, Some(now)));
		u.metal[12] = match h {
			Some(h) => [w, h, knob("metal_planar_pop", 0.6), knob("metal_planar_blur", 9.0)],
			None => [0.0; 4],
		};
		// The smaller targets' mip pick (metal_detail.rs); u.metal[13].w is 0 otherwise (metalcontact.rs `knobs`).
		if h.is_some() {
			u.metal[13][3] = self.planar.as_ref().map_or(0.0, Planar::lod_bias);
		}
	}
}

/// The planar weight (u.metal[12].x) this frame: 0 without a floor, from 0 again when the floor changed since last frame (`prev`),
/// else last frame's `w` plus `dt / FADE` (dt capped at 0.1 s: a hitch is no instant pop), at most 1.
pub(super) fn fade(prev: Option<f32>, h: Option<f32>, w: f32, dt: f32) -> f32 {
	match (prev, h) {
		(_, None) => 0.0,
		(Some(p), Some(h)) if (p - h).abs() < 0.01 => (w + dt.clamp(0.0, 0.1) / FADE).min(1.0),
		_ => 0.0,
	}
}

impl Gpu {
	/// Before the level pass: the level mirrored about this frame's floor (`MetalEnv::plan_planar`) into the planar target, then its
	/// mip chain. Nothing without a floor. `ties`: this frame's indirect tie runs.
	pub(in crate::viewer) fn metal_planar(&self, enc: &mut wgpu::CommandEncoder, scene: &Scene, u: &Uniforms, ties: Option<&super::bindless::Runs>) {
		let me = &self.metalenv;
		let (Some(h), Some(pl), Some(re)) = (me.plane.get(), me.planar.as_ref(), me.real.get()) else { return };
		let mut slot = me.planar_mirror.borrow_mut();
		let with = slot.get_or_insert_with(|| super::mirror::Mirror::new(&self.device, self.volfog.soft.layout(), &self.sun, me));
		// The clip plane sits `metal_planar_clip` above the mirror plane: the floor's own faces a little above the picked height (the
		// height is a bin's mean) were mirrored just under it and covered the whole reflection (black floor, no hero; agent 14).
		let clip = h + knob("metal_planar_clip", 0.15);
		self.draw_mirror_with(enc, scene, u, (h, clip), with, (&pl.ms, &pl.l0, &pl.depth), ties, super::mirror::METAL_PLANAR);
		// Contact reflections: each mirrored point's weight by its reflection distance, into the colour's alpha (mipped with it below).
		pl.contact.draw(&self.queue, enc, &pl.l0, &super::mirror::mirrored_clip(&u.view_proj, h, clip), h, [u.metal[13][0], u.metal[13][1], 0.0, 0.0]);
		for (from, target) in &pl.mips {
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("metal planar mip"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: None,
				timestamp_writes: crate::viewer::passtime::rw("metal planar mip"),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_pipeline(&re.mip);
			pass.set_bind_group(0, from, &[]);
			pass.draw(0..3, 0..1);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn quad(z: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> [[[f32; 3]; 3]; 2] {
		[[[x0, y0, z], [x1, y0, z], [x1, y1, z]], [[x0, y0, z], [x1, y1, z], [x0, y1, z]]]
	}

	fn look_down(eye: [f32; 3]) -> [[f32; 4]; 4] {
		let cam = Camera { pos: eye, yaw: std::f32::consts::FRAC_PI_2, pitch: -0.6, tan_half: 0.6, up: [0.0, 0.0, 1.0] };
		let vp = super::super::view_proj(&cam, 1.6, 0.5, 2000.0);
		super::super::mirror::inverse(&vp).unwrap()
	}

	/// A flat metal floor ahead and below is picked; a wall (vertical faces) and a small plate are not floors.
	#[test]
	fn planar_picks_the_floor_in_view() {
		let wall = [[[0.0, 10.0, 0.0], [10.0, 10.0, 0.0], [10.0, 10.0, 8.0]]];
		let plate = quad(3.0, 0.0, 0.0, 1.0, 1.0);
		let floor = quad(1.0, -30.0, -10.0, 30.0, 60.0);
		let f = Floors::from_faces(floor.into_iter().chain(plate).chain(wall));
		assert_eq!(f.levels.len(), 1);
		let ivp = look_down([0.0, 0.0, 6.0]);
		assert_eq!(f.pick([0.0, 0.0, 6.0], &ivp, None), Some(1.0));
		// Below the floor, or no floor: nothing.
		assert_eq!(f.pick([0.0, 0.0, 0.5], &look_down([0.0, 0.0, 0.5]), None), None);
		assert_eq!(Floors::default().pick([0.0, 0.0, 6.0], &ivp, None), None);
	}

	/// After a floor change the weight starts at 0 and reaches 1 in `FADE` seconds; no floor = 0; a hitch adds at most 0.1 s.
	#[test]
	fn planar_fades_in_after_a_change() {
		assert_eq!(fade(None, Some(1.0), 1.0, 0.016), 0.0);
		assert_eq!(fade(Some(2.0), Some(1.0), 1.0, 0.016), 0.0);
		assert_eq!(fade(Some(1.0), None, 1.0, 0.016), 0.0);
		let w = fade(Some(1.0), Some(1.0), 0.0, 0.05);
		assert!((w - 0.25).abs() < 1e-5);
		assert_eq!(fade(Some(1.0), Some(1.0), 0.0, 5.0), 0.5);
		assert_eq!(fade(Some(1.0), Some(1.0), 0.9, 0.05), 1.0);
	}

	/// Two floors in view: the kept one stays while the other wins by fewer than `KEEP` rays.
	#[test]
	fn planar_keeps_last_frames_floor() {
		let f = Floors::from_faces(quad(1.0, -30.0, 0.0, 30.0, 60.0).into_iter().chain(quad(2.0, -30.0, -10.0, 30.0, 0.5)));
		let eye = [0.0, 0.0, 6.0];
		let ivp = look_down(eye);
		let free = f.pick(eye, &ivp, None).unwrap();
		let other = if free == 1.0 { 2.0 } else { 1.0 };
		let held = f.pick(eye, &ivp, Some(other)).unwrap();
		assert!(held == other || held == free);
	}
}
