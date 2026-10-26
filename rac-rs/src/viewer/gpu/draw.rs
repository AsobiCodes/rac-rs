//! The per-frame draw: Gpu::draw, strip drawing, debug switches, water mirror choice.
//! - `prepare`: the steps before the passes (uniforms and sun, water flags, surface image, HUD quads, indirect arguments).
//! - `passes`:  the render passes (level, particles, post, HUD).
//! - `finish`:  movie overlay, readback, submit / present.

use super::*;

mod prepare;
mod passes;
mod pass_world;
mod pass_water;
mod pass_particles;
mod pass_hud;
mod pass_omni;
mod pass_shrubs;
mod pass_kofi;
mod finish;
mod split;

/// Values of the frame that the first step computes (camera, uniforms, graphics flags, sun) and the later steps read.
struct DrawFrame {
	aspect: f32,
	eye: Camera,
	wfar: f32,
	u: Uniforms,
	gfx: crate::retail::pcext::Graphics,
	post_on: bool,
	sun: Option<super::sunshadow::SunFrame>,
}

/// Water, river and particle flags of the frame and the water draws the level pass records for the water pass.
struct DrawFx {
	water_on: bool,
	wrec: Vec<super::waterfx::WaterDraw>,
	strips_water: bool,
	strip_flows: Vec<u32>,
	river: Vec<usize>,
	river_groups: Vec<(u32, Vec<usize>)>,
	part_on: bool,
	rays: Option<([f32; 2], f32)>,
	water_frame: Option<super::waterfx::WaterFrame>,
}

/// The acquired surface image, its view, the command encoder with its cut points, and the GI level of the frame.
struct DrawTarget {
	/// None in the Classic pass of a split frame (split.rs): it draws into the offscreen target and presents nothing.
	frame: Option<wgpu::SurfaceTexture>,
	view: wgpu::TextureView,
	enc: wgpu::CommandEncoder,
	cuts: super::presenter::Cuts,
	gi_level: u8,
	/// [PC] ReShade add-on (reshade.rs): this frame renders the effects before the HUD.
	rs: bool,
}

/// HUD quads as draw groups and buffers, the tie / shrub indirect arguments, the overlay size.
struct DrawBufs {
	sw: f32,
	sh: f32,
	hd: Vec<(Draw, bool)>,
	split_group: Option<usize>,
	multi: Option<(super::bindless::Runs, super::bindless::Runs, super::bindless::Runs)>,
	sharp_pipes: Option<&'static [wgpu::RenderPipeline; 2]>,
	hud_buffers: Option<(wgpu::Buffer, wgpu::Buffer)>,
}


impl Gpu {
	pub(in crate::viewer) fn draw(&mut self, cam: &Camera, capture: bool) -> Option<crate::render::Image> {
		super::passtime::frame_begin(&self.device); // [PC] RAC_GPU_PASSES (passtime.rs)
		self.swap_repl(); // [PC] texture packs
		let classic_ready = split::take_ready(); // [PC] split view (split.rs): the Classic half was drawn offscreen first (handler/frame_split.rs)
		let dc = self.frame_uniforms(cam)?;
		let mut dfx = self.frame_effects(cam, &dc)?;
		let (mut dt, mut warp_hud) = self.frame_target(cam, &dc)?;
		self.split_extras(&mut warp_hud, classic_ready);
		let db = self.frame_buffers(warp_hud)?;
		self.draw_passes(cam, &dc, &mut dfx, &mut dt, &db)?;
		self.frame_finish(dt, db.sw, db.sh, capture)
	}
}

/// KER-92: pool draw mode 2 cull. The depth key is the view z of the segment midpoint (`f` = view forward); the line is
/// drawn iff `key - near > 0` and `key - far < 0` with far = min(line far, the pool clamp `part_far`), and both points lie
/// inside the view frustum (`m` = view-projection, depth 0..1).
/// BRIDGE-23: FIX-blend strips, triangles (i, i+1, i+2), one draw per strip; the blend constant is FIX / 128.
/// [PC] Dynamic shadows (sunshadow.rs): the texture slots whose texels mostly blend (PS2 alpha below 0x60; the texture holds
/// alpha x 2): a mesh drawn with them is a see-through shell and casts no sun shadow.
pub(super) fn clear_slots(slots: &HashMap<u32, usize>, n: usize, textures: &HashMap<u32, Texture>) -> Vec<bool> {
	let mut clear = vec![false; n];
	for (&key, &slot) in slots {
		if let Some(t) = textures.get(&(key & crate::mobies::KEY_MASK)).filter(|t| !t.rgba.is_empty()) {
			clear[slot] = t.rgba.iter().filter(|c| c[3] < 0xc0).count() * 5 >= t.rgba.len() * 3;
		}
	}
	clear
}

/// `rec`: [PC] Water Enhanced records each strip's buffers (waterfx.rs), with its flow slot from the list (by strip index; 0 past it);
/// only the strips with one of the listed fx textures, if any are listed.
#[allow(clippy::too_many_arguments)]
fn draw_strips(device: &wgpu::Device, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, fx_tex: &HashMap<usize, wgpu::BindGroup>, strips: &[particles::WorldStrip], pipeline: &wgpu::RenderPipeline, add_pipeline: &wgpu::RenderPipeline, mut rec: Option<(&mut Vec<super::waterfx::WaterDraw>, &[u32], &[usize])>) {
	for (i, s) in strips.iter().enumerate() {
		let Some(bind) = fx_tex.get(&crate::retail::efx_draw::plain_tex(s.tex)) else { continue };
		if s.pos.len() < 3 {
			continue;
		}
		// An effects-layer strip with GS fog carries the fog byte F in its vertex alpha (retail/efx_draw.rs `fog_tex`).
		let fogged = s.tex & crate::retail::efx_draw::FOG_ON != 0;
		let verts: Vec<Vertex> = (0..s.pos.len()).map(|j| Vertex { pos: s.pos[j], uv: s.uv[j], normal: [0.0; 3], colour: if fogged { [s.colour[j][0], s.colour[j][1], s.colour[j][2], 1.0] } else { s.colour[j] }, light: crate::retail::efx_draw::fog_light(s.tex, 0, if fogged { s.colour[j][3] } else { -1.0 }) }).collect();
		let idx: Vec<u32> = (0..s.pos.len() as u32 - 2).flat_map(|t| [t, t + 1, t + 2]).collect();
		let vb = buffer(device, "fx strip vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
		queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
		let ib = buffer(device, "fx strip indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
		queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
		pass.set_pipeline(if s.add { add_pipeline } else { pipeline });
		let k = s.fix as f64 / 128.0;
		pass.set_blend_constant(wgpu::Color { r: k, g: k, b: k, a: 1.0 });
		pass.set_vertex_buffer(0, vb.slice(..));
		pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
		pass.set_bind_group(1, bind, &[]);
		pass.draw_indexed(0..idx.len() as u32, 0, 0..1);
		if let Some((r, flows, _)) = rec.as_mut().filter(|(_, _, only)| only.is_empty() || only.contains(&crate::retail::efx_draw::plain_tex(s.tex))) {
			if water_log() {
				eprintln!("water strip {i}: tex {} add {} fix {} verts {} z {:.2}", s.tex, s.add, s.fix, s.pos.len(), s.pos.iter().map(|p| p[2]).sum::<f32>() / s.pos.len() as f32);
			}
			let mut d = super::waterfx::WaterDraw::new(vb, ib, idx.len() as u32, s.pos.iter().copied()).kind("strip");
			d.flow = flows.get(i).copied().unwrap_or(0);
			r.push(d);
		}
	}
}

/// [probe] RAC_WATER_LOG=1: the water draws the water pass gets, per frame.
pub(super) fn water_log() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_WATER_LOG").is_ok())
}

/// [probe] RAC_DRAWSKIP=a,b,.. leaves those main-pass draw groups out (fx lane run 16): world ties tfrags marker props clipped
/// debris warp sky metal pools shrub bb vol hlate plate abe ocean oceang quads strips.
fn dskip(name: &str) -> bool {
	static S: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
	S.get_or_init(|| std::env::var("RAC_DRAWSKIP").map(|v| v.split(',').map(str::to_owned).collect()).unwrap_or_default()).iter().any(|s| s == name)
}

/// Silent probes (env-gated): RAC_TFRAG_SKIP=i skips fragment i, RAC_TFRAG_SKIPBIT=b skips every fragment whose index has bit b set
/// (bisect which fragment draws a pixel), RAC_TFRAG_INFO=i prints that fragment's cull facts each frame.
pub(super) fn tfrag_probe_skip(i: usize) -> bool {
	static P: std::sync::OnceLock<(Option<usize>, Option<u32>)> = std::sync::OnceLock::new();
	let p = P.get_or_init(|| (std::env::var("RAC_TFRAG_SKIP").ok().and_then(|v| v.parse().ok()), std::env::var("RAC_TFRAG_SKIPBIT").ok().and_then(|v| v.parse().ok())));
	p.0 == Some(i) || p.1.is_some_and(|b| (i >> b) & 1 == 1)
}

pub(super) fn tfrag_probe_info(i: usize) -> bool {
	static P: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
	*P.get_or_init(|| std::env::var("RAC_TFRAG_INFO").ok().and_then(|v| v.parse().ok())) == Some(i)
}

/// [PC] How many planes the water pass mirrors per frame: Water High (`water_q` 3) `mirror::MAX_MIRRORS`, Medium 1, Low none.
/// RAC_WATER_MIRRORS=0..3 overrides it (probe: time 0 / 1 / 2 / 3 mirrors in one exe).
fn mirror_count(water_q: u8) -> usize {
	static S: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
	let o = *S.get_or_init(|| std::env::var("RAC_WATER_MIRRORS").ok().and_then(|v| v.trim().parse::<usize>().ok()));
	o.unwrap_or(match water_q {
		0 | 1 => 0,
		2 => 1,
		_ => super::mirror::MAX_MIRRORS,
	})
	.min(super::mirror::MAX_MIRRORS)
}

/// [PC] The planes to mirror this frame, at most `n` heights (`mirror::pick` for one, `mirror::pick_many` for more, given last
/// frame's planes of this level). A plane the estimate misses for a frame (the culled patch set shows no flat height that frame)
/// stays for `MIRROR_HOLD` frames while a slot is free, instead of dropping to the screen-space reflection.
fn mirror_pick(level: usize, draws: &[super::waterfx::WaterDraw], eye: [f32; 3], vp: &[[f32; 4]; 4], n: usize) -> Vec<f32> {
	const MIRROR_HOLD: u32 = 12;
	thread_local! {
		static LAST: std::cell::RefCell<(usize, Vec<(f32, u32)>)> = const { std::cell::RefCell::new((usize::MAX, Vec::new())) };
	}
	if n == 0 {
		return Vec::new();
	}
	LAST.with(|l| {
		let mut l = l.borrow_mut();
		if l.0 != level {
			*l = (level, Vec::new());
		}
		let prev: Vec<f32> = l.1.iter().map(|p| p.0).collect();
		let ivp = super::mirror::inverse(vp);
		let fresh: Vec<f32> = if n == 1 { super::mirror::pick(draws, eye, ivp.as_ref(), prev.first().copied()).into_iter().collect() } else { super::mirror::pick_many(draws, eye, ivp.as_ref(), &prev, n) };
		let mut now: Vec<(f32, u32)> = fresh.into_iter().map(|h| (h, 0)).collect();
		for &(h, age) in &l.1 {
			if now.len() < n && age < MIRROR_HOLD && now.iter().all(|p| (p.0 - h).abs() >= 0.5) {
				now.push((h, age + 1));
			}
		}
		l.1 = now;
		l.1.iter().map(|p| p.0).collect()
	})
}