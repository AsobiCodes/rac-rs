//! Frame uniforms and fog, dynamic light rows, relight of the scene vertices, and the moby / hero light words.

use super::*;

/// Levels whose Enhanced sun follows the painted sky sun (`Gpu::sun_rows`): Veldin, Gemlik (user 2026-10-25: the big orange star
/// stood behind Ratchet and the shadows fell to his right; retail's key light comes from -x, 40 degrees beside the star).
const SKY_KEY: [usize; 2] = [0, 13];
/// Of those, the levels whose Enhanced shading (mobies, the hero) takes light row 0's turned key light too, so a lit side faces
/// the star the shadows point away from. Veldin keeps retail's shading (approved 2026-10-24).
const SKY_KEY_LIT: [usize; 1] = [13];

/// The painted sun's compass direction at the higher of its own and the retail key light's height above the horizon:
/// Veldin's dusk sun sits 8.5 degrees up, and a key light that low left the whole canyon floor in shadow.
fn sky_key(sky: [f32; 3], key: [f32; 3]) -> [f32; 3] {
	let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
	let up = (sky[2] / len(sky)).max(key[2] / len(key)).clamp(-1.0, 1.0);
	let flat = (sky[0] * sky[0] + sky[1] * sky[1]).sqrt().max(1e-6);
	let h = (1.0 - up * up).sqrt() / flat;
	[sky[0] * h, sky[1] * h, up]
}

impl Gpu {
	/// One frame of the dynamic lights on the world: follow the light table, then rewrite the colours of the visible
	/// ties, shrubs and tfrags that hold a light or just lost one (`dynlight::LitWorld`, lighting_fog.md 5b).
	pub(in crate::viewer) fn relight(&mut self, view: Option<&dynlight::View>) {
		let _s = super::profile::span("relight");
		let Some(scene) = &mut self.scene else { return };
		for r in scene.lit.frame(&self.table, view) {
			if r.kind == crate::mesh::LitKind::Tie {
				// BRIDGE-32: a tie returns its 64 slot colours; the vertices of whichever LOD is drawn are recoloured from
				// them in `cull_ties` (its Q picks the blend).
				scene.tie_cols[r.at] = r.colours;
				scene.tie_stale[r.at] = [true; 3];
				if let Some(g) = &mut scene.tie_gpu {
					g.caps_dirty.push(r.at);
				}
				continue;
			}
			if r.kind == crate::mesh::LitKind::Tfrag {
				// REN-CULL-TFRAG: per position record colours; `cull_tfrags` recolours the drawn LOD from them (after the morph).
				scene.tfrag_cols[r.at] = r.colours;
				super::tfrag_memo::forget(&mut scene.tfrag_memo, r.at); // [PC] perf (ncull): the fragment's ranges are posed again
				continue;
			}
			let (space, first) = relight_space(r.first, scene.tie_first, scene.shrub_first);
			let (verts, buf) = match space {
				VertexSpace::Shrub => (&mut scene.shrub_verts, &scene.shrubs.vbuf),
				VertexSpace::Tie => (&mut scene.tie_verts, &scene.ties.vbuf),
				VertexSpace::World => (&mut scene.world_verts, &scene.world.vbuf),
			};
			for (k, c) in r.colours.iter().enumerate() {
				// A shrub keeps the alpha carrier of its current fade (colour w), world and tie vertices stay -2.
				verts[first + k].colour = [c[0] / 128.0, c[1] / 128.0, c[2] / 128.0, verts[first + k].colour[3]];
			}
			let range = first..first + r.colours.len();
			self.queue.write_buffer(buf, (first * std::mem::size_of::<Vertex>()) as u64, bytemuck::cast_slice(&verts[range]));
		}
	}

	/// Camera, level lights and fog (decomp/notes/lighting_fog.md); a neutral rig without them. The fog is the
	/// live one (`fog_live`: camera zone or underwater, NOV-28) once the handler has set it.
	pub(in crate::viewer) fn uniforms(&self, cam: &Camera, eye: &Camera, aspect: f32, range: (f32, f32)) -> Uniforms {
		self.uniforms_fog(cam, eye, aspect, range, true)
	}

	/// `uniforms`, with `live` false keeping the static level fog (the pause item panel).
	pub(super) fn uniforms_fog(&self, cam: &Camera, eye: &Camera, aspect: f32, range: (f32, f32), live: bool) -> Uniforms {
		let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
		let (lights, ambient, fog_colour, fog) = match &self.lighting {
			Some(lt) if !lt.lights.is_empty() => {
				let lights = self.light_rows();
				let spawn = v4(gameplay::SPAWN_AMBIENT.map(|c| c as f32 / 128.0));
				let (colour, fog) = match self.fog_live.filter(|_| live) {
					Some((colour, fog)) => (colour, fog),
					None => (lt.fog_colour, lt.fog),
				};
				let c = crate::water::gs_rgb(colour); // the target is gamma-space Unorm: raw GS bytes
				(lights, [spawn; 4], [c[0], c[1], c[2], 1.0], fog)
			}
			_ => (self.light_rows(), [[0.4, 0.4, 0.4, 0.0]; 4], [0.0, 0.0, 0.0, 1.0], [0.0, 1.0e9, 255.0, 255.0]),
		};
		// [PC] Enhanced on a `SKY_KEY_LIT` level: the shading's key light (slot 0) points at the painted sun, as the shadows do.
		let mut lights = lights;
		if SKY_KEY_LIT.contains(&self.level) && crate::retail::splitview::fx(self.modern_ok && crate::retail::pcext::graphics().enhanced() && !super::super::pipe_guard::off(super::super::pipe_guard::Effect::Post)) { // [PC] pipe_guard.rs: a refused Post leaves the plain Classic light
			lights[0] = self.sun_rows()[0];
		}
		// The arrival warp sets its own fog (retail FOGCOL 0,0,0x10; range 0..0x80000) over the departure level's.
		let (fog_colour, fog) = if self.scene.as_ref().is_some_and(|s| s.warp_shown) { ([0.0, 0.0, 0x10 as f32 / 255.0, 1.0], [0.0, 524288.0, 255.0, 128.0]) } else { (fog_colour, fog) };
		let mut dynl = [[0.0; 4]; 2 * MAX_DYN];
		for (k, (c, d)) in self.moby_dyn.iter().enumerate() {
			dynl[2 * k] = [c[0], c[1], c[2], 0.0];
			dynl[2 * k + 1] = [d[0], d[1], d[2], 0.0];
		}
		Uniforms {
			view_proj: view_proj(cam, aspect, range.0, range.1),
			sky_view_proj: view_proj(eye, aspect, range.0, range.1),
			lights,
			ambient,
			fog_colour,
			fog,
			eye: [cam.pos[0], cam.pos[1], cam.pos[2], super::icefx::flag(self.level)],
			points: self.points,
			dynl,
			sun_vp: [[[0.0; 4]; 4]; 3],
			sun: [0.0; 4],
			sun_bias: [0.0; 4],
			sun_texel: [0.0; 4],
			sun_q: [0.0; 4],
			keep_tail: [0.0; 4],
		}
	}

	/// The uniform light slots (`Uniforms::lights`): per level light (colour A + wrap, to-light A, colour B + wrap, to-light B), zero past
	/// the level's lights; without level lights one default slot 0.
	pub(in crate::viewer) fn light_rows(&self) -> [[[f32; 4]; 4]; 16] {
		let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
		let pair = |l: &gameplay::DirLight| {
			let c = |k: usize| [l.colour[k][0], l.colour[k][1], l.colour[k][2], l.wrap[k]];
			[c(0), v4(l.dir[0].map(|x| -x)), c(1), v4(l.dir[1].map(|x| -x))]
		};
		match &self.lighting {
			Some(lt) if !lt.lights.is_empty() => std::array::from_fn(|k| match self.reserved_lights {
				Some(r) if k >= 12 => r[k - 12],
				_ => lt.lights.get(k).map_or([[0.0; 4]; 4], pair),
			}),
			_ => {
				let mut lights = [[[0.0; 4]; 4]; 16];
				lights[0] = [[0.8, 0.8, 0.8, 0.0], [0.3, 0.3, 0.9, 0.0], [0.0; 4], [0.0; 4]];
				lights
			}
		}
	}

	/// [PC] The Enhanced sun (dynamic shadows, sky map, fog and clouds): `light_rows` with slot 0's key light turned towards the
	/// sun the level's sky paints (`post::sky_sun`) on the levels in `SKY_KEY`. Veldin (user 2026-10-24): retail's key light came
	/// from about 65 degrees beside the painted dusk sun, so shadows and the sky disagreed. Classic shading keeps `light_rows`.
	pub(in crate::viewer) fn sun_rows(&self) -> [[[f32; 4]; 4]; 16] {
		let mut rows = self.light_rows();
		if let Some(to) = SKY_KEY.contains(&self.level).then(|| crate::viewer::post::sky_sun(self.level)).flatten() {
			for k in [1, 3] {
				if rows[0][k][..3].iter().any(|v| *v != 0.0) {
					let d = sky_key(to, [rows[0][k][0], rows[0][k][1], rows[0][k][2]]);
					rows[0][k] = [d[0], d[1], d[2], rows[0][k][3]];
				}
			}
		}
		rows
	}

	/// A moby's `Vertex::light` (decomp/notes/lighting_fog.md "Moby lighting"): a placed moby's own colour and
	/// light word (instance +0x64.. / +0x70), else the spawn default (64, 64, 64) and word 0. Zones and point
	/// lights are the hero's only (`hero_light`).
	pub(in crate::viewer) fn moby_light(placed: Option<([u8; 3], u32)>) -> [f32; 4] {
		let (col, word) = placed.unwrap_or((gameplay::SPAWN_AMBIENT, 0));
		let (a, b, t) = gameplay::light_word(word);
		Self::pack_light(col.map(|c| c as f32 / 128.0), a, b, t)
	}

	/// The hero's `Vertex::light`: HeroEnvLighting (zones, else its placed colour and word, plus point lights).
	pub(in crate::viewer) fn hero_light(lighting: &Option<gameplay::Lighting>, p: [f32; 3], placed: ([u8; 3], u32), env: &mut (usize, usize, f32, [f32; 3])) -> [f32; 4] {
		Self::hero_light_tint(lighting, p, placed, env).0
	}

	/// `hero_light` and BRIDGE-57's copy of the hero's +0x3c ambient bytes / +0x38 light word (FUN_00271c10 reads them;
	/// decomp/notes/port_bridges.md BRIDGE-57). [I] a blended zone ambient quantises by truncation.
	pub(in crate::viewer) fn hero_light_tint(lighting: &Option<gameplay::Lighting>, p: [f32; 3], placed: ([u8; 3], u32), env: &mut (usize, usize, f32, [f32; 3])) -> ([f32; 4], ([u8; 3], u32)) {
		match lighting {
			Some(lt) => {
				let (a, b, t, amb) = lt.hero(p, env);
				let bytes = amb.map(|c| (c * 128.0).min(255.0) as u8);
				(Self::pack_light(amb, a, b, t), (bytes, a as u32 | (b as u32) << 8 | ((t * 256.0) as u32) << 16))
			}
			None => (Self::moby_light(Some(placed)), placed),
		}
	}

	/// Slots past 15 read as slot 15 (zero unless the level has 16 lights).
	pub(in crate::viewer) fn pack_light(amb: [f32; 3], a: usize, b: usize, t: f32) -> [f32; 4] {
		[amb[0], amb[1], amb[2], (a.min(15) * 16 + b.min(15)) as f32 + t.clamp(0.0, 0.999)]
	}
}

/// Which vertex buffer a `LitWorld` vertex address falls in, and the offset inside it: world below `tie_first`, tie LOD vertices
/// from there, shrubs from `shrub_first` on (>=, the boundaries belong to the later buffer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VertexSpace {
	World,
	Tie,
	Shrub,
}

pub(super) fn relight_space(first: usize, tie_first: usize, shrub_first: usize) -> (VertexSpace, usize) {
	if first >= shrub_first {
		(VertexSpace::Shrub, first - shrub_first)
	} else if first >= tie_first {
		(VertexSpace::Tie, first - tie_first)
	} else {
		(VertexSpace::World, first)
	}
}
