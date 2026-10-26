//! Particle sprites, ribbons and lines (set_particles) and their view tests.

use super::*;

impl Gpu {
	/// Camera-facing quads for this frame's particles.
	/// `fx`: retail mode's effect pool (retail.rs `RetailFx`): its lines (mode 2) and ribbons (mode 3) join the pool
	/// sprites in one depth-sorted walk (queue_b.rs `pool_walk`); `lines` / `ribbons` are rac-rs's own.
	pub(in crate::viewer) fn set_particles(&mut self, sprites: &[particles::Sprite], ribbons: &[particles::Ribbon], lines: &[particles::Line], mobys: &[particles::EffectMoby], cam: &Camera, fx: Option<&crate::retail::RetailFx>) {
		let _s = super::profile::span("set_particles");
		let f = [cam.pitch.cos() * cam.yaw.cos(), cam.pitch.cos() * cam.yaw.sin(), cam.pitch.sin()];
		let (slots, pool_ribbons, pool_lines, line_slots) = fx.map_or((&[][..], &[][..], &[][..], &[][..]), |x| (&x.sprite_slots[..], &x.ribbons[..], &x.lines[..], &x.line_slots[..]));
		// [PC] Water Enhanced contact rings and wakes (waterfx.rs `Rings`, `Wakes`): the hero, the pool particles (a line by its
		// lower end) and the drawn mobys.
		if let Some(x) = fx {
			let parts = sprites.iter().filter(|sp| sp.pool.is_some()).map(|sp| sp.pos).chain(pool_lines.iter().map(|l| if l.a[2] < l.b[2] { l.a } else { l.b }));
			self.waterfx.contacts(x.frames, x.hero, parts, &x.movers);
		}
		let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
		let m = view_proj(cam, aspect, RETAIL_NEAR, self.world_far());
		// KER-92: pool lines, kept when both points pass the clip test and the midpoint view depth is inside (near, far).
		self.line_verts.clear();
		if !lines.is_empty() {
			for l in lines.iter().filter(|l| line_shown(&m, cam.pos, f, l, self.part_far)) {
				self.line_verts.push(Vertex { pos: l.a, uv: [0.0; 2], normal: [0.0; 3], colour: l.col_a, light: [0.0; 4] });
				self.line_verts.push(Vertex { pos: l.b, uv: [0.0; 2], normal: [0.0; 3], colour: l.col_b, light: [0.0; 4] });
			}
		}
		// BRIDGE-56: billboards face the rolled view.
		let right = normalise(cross(f, cam.up));
		let up = cross(right, f);
		self.part_verts.clear();
		let view = self.scene.as_ref().and_then(|s| s.clip_view.as_ref()).map(|v| (v.e, v.f));
		let defs = &self.part_defs;
		// Effect type -> texture key; FX_TEX + n is an fx texture, an unmapped type falls back to the disc.
		let key = |t: u32| if t >= particles::FX_TEX { t - particles::FX_TEX } else { defs.get(t as usize).map_or(u32::MAX, |&n| PART_TEX + n as u32) };
		let at = |p: [f32; 3], a: [f32; 3], s: f32, b: [f32; 3], t: f32| [p[0] + a[0] * s + b[0] * t, p[1] + a[1] * s + b[1] * t, p[2] + a[2] * s + b[2] * t];
		// Retail pool draw order (retail/queue_b.rs `pool_walk`): pool sprites, ribbons and lines draw back to front in
		// runs of one (blend, texture); every other sprite keeps run 0, grouped by (blend, texture) as before.
		let mut run: (u32, Option<(bool, u32)>) = (0, None);
		let mut next_run = |additive: bool, k: u32| {
			if run.1 != Some((additive, k)) {
				run = (run.0 + 1, Some((additive, k)));
			}
			run.0
		};
		use crate::retail::queue_b::PoolItem;
		for item in crate::retail::queue_b::pool_walk(sprites, slots, pool_ribbons, pool_lines, line_slots, cam.pos, f) {
			let sp = match item {
				PoolItem::Sprite(i) => &sprites[i],
				// Mode 3 (FUN_00259e18 sort, overlay.elf.c 81947-81985): both points clip-tested, midpoint key strict in
				// (near, far); the build fades both alphas by the sprite ramp (retail.rs `pool_ribbon_quad`) [C].
				PoolItem::Ribbon(i) => {
					let r = &pool_ribbons[i];
					let vz = (0..3).map(|k| ((r.head[k] + r.tail[k]) * 0.5 - cam.pos[k]) * f[k]).sum::<f32>();
					let (near, far) = ((r.b9 & 15) as f32 / 4.0, crate::water::pool_far(r.b9, self.part_far));
					if vz - near <= 0.0 || vz - far >= 0.0 || !clip_inside(&m, r.head) || !clip_inside(&m, r.tail) {
						continue;
					}
					let q = crate::retail::pool_ribbon_quad(r, cam.pos, (vz - near).min((far - vz) / 16.0).clamp(0.0, 1.0));
					let out = self.part_verts.entry((next_run(q.additive, q.tex as u32), q.additive, q.tex as u32)).or_default();
					for k in 0..4 {
						out.push(Vertex { pos: q.pos[k], uv: q.uv[k], normal: [0.0; 3], colour: q.colour[k], light: [0.0; 4] });
					}
					continue;
				}
				// Mode 2 (KER-92 cull, `line_shown`): no fade.
				PoolItem::Line(i) => {
					let l = &pool_lines[i];
					if line_shown(&m, cam.pos, f, l, self.part_far) {
						let out = self.part_verts.entry((next_run(false, LINE_KEY), false, LINE_KEY)).or_default();
						out.push(Vertex { pos: l.a, uv: [0.0; 2], normal: [0.0; 3], colour: l.col_a, light: [0.0; 4] });
						out.push(Vertex { pos: l.b, uv: [0.0; 2], normal: [0.0; 3], colour: l.col_b, light: [0.0; 4] });
					}
					continue;
				}
			};
			let run = match sp.pool {
				Some(_) => next_run(sp.additive, key(sp.tex)),
				None => 0,
			};
			// Pool sprites (decomp/notes/particles.md "Draw size"): size * vz/(vz+0.5), alpha * the byte +9 near/far
			// ramp (culled outside it), texture (0,0) corner at screen angle rot clockwise, i.e. this quad's
			// rot + 135 deg counter-clockwise = -rot. PAL's 0.918 vertical squash is dropped (modern display).
			let (size, rot, colour) = match sp.pool {
				Some(b9) => {
					let vz = (sp.pos[0] - cam.pos[0]) * f[0] + (sp.pos[1] - cam.pos[1]) * f[1] + (sp.pos[2] - cam.pos[2]) * f[2];
					let (near, far) = ((b9 & 15) as f32 / 4.0, crate::water::pool_far(b9, self.part_far));
					if vz <= near || vz >= far {
						continue;
					}
					let fade = (vz - near).min((far - vz) / 16.0).clamp(0.0, 1.0);
					let mut c = sp.colour;
					c[3] *= fade;
					// NOV-22.12 mode 1: same cull and fade, then a world-XY quad (no perspective term, no 135 deg turn).
					if sp.flat {
						let out = self.part_verts.entry((run, sp.additive, key(sp.tex))).or_default();
						for (p, uv) in flat_corners(sp.pos, sp.size, sp.rot).into_iter().zip([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]) {
							out.push(Vertex { pos: p, uv, normal: [0.0; 3], colour: c, light: [0.0; 4] });
						}
						continue;
					}
					(sp.size * vz / (vz + 0.5), -sp.rot - std::f32::consts::FRAC_PI_4 * 3.0, c)
				}
				None => (sp.size, sp.rot, sp.colour),
			};
			let h = size / 2.0;
			let (c, sn) = (rot.cos() * h, rot.sin() * h);
			let r = [right[0] * c + up[0] * sn, right[1] * c + up[1] * sn, right[2] * c + up[2] * sn];
			let u = [up[0] * c - right[0] * sn, up[1] * c - right[1] * sn, up[2] * c - right[2] * sn];
			let out = self.part_verts.entry((run, sp.additive, key(sp.tex))).or_default();
			for (s, t, uv) in [(-1.0, -1.0, [0.0, 1.0]), (1.0, -1.0, [1.0, 1.0]), (1.0, 1.0, [1.0, 0.0]), (-1.0, 1.0, [0.0, 0.0])] {
				out.push(Vertex { pos: at(sp.pos, r, s, u, t), uv, normal: [0.0; 3], colour, light: [0.0; 4] });
			}
		}
		// Blast mobys (decomp/notes/particles.md "Ship crash"): triangles as degenerate quads, alpha-over [I].
		for m in mobys {
			let Some(mesh) = self.fx_meshes.get(&m.class) else { continue };
			let Some(fade) = effect_fade(view, m.draw, m.pos) else { continue };
			let (sx, cx) = m.rot[0].sin_cos();
			let (sy, cy) = m.rot[1].sin_cos();
			let (sz, cz) = m.rot[2].sin_cos();
			let colour = [m.tint[0], m.tint[1], m.tint[2], moby_draw_alpha(fade, m.alpha as u8) as f32 / 128.0];
			for (tex, f) in &mesh.tris {
				let out = self.part_verts.entry((0, false, *tex as u32)).or_default();
				let v = |i: u32| {
					let p = mesh.pos[i as usize];
					let (x, y, z) = (p[0] * m.scale, p[1] * m.scale, p[2] * m.scale);
					let (y, z) = (y * cx - z * sx, y * sx + z * cx);
					let (x, z) = (x * cy + z * sy, -x * sy + z * cy);
					let (x, y) = (x * cz - y * sz, x * sz + y * cz);
					Vertex { pos: [m.pos[0] + x, m.pos[1] + y, m.pos[2] + z], uv: mesh.uv.get(i as usize).copied().unwrap_or([0.0; 2]), normal: [0.0; 3], colour, light: [0.0; 4] }
				};
				out.extend([v(f[0]), v(f[1]), v(f[2]), v(f[2])]);
			}
		}
		for rb in ribbons {
			let n = rb.points.len();
			let out = self.part_verts.entry((0, rb.additive, key(rb.tex))).or_default();
			for k in (if rb.arc { 2 } else { 1 })..n {
				let (a, b) = (rb.points[k - 1], rb.points[k]);
				let side = normalise(cross([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [cam.pos[0] - a[0], cam.pos[1] - a[1], cam.pos[2] - a[2]]));
				let edge = |p: [f32; 3], j: usize, u: f32| {
					let t = j as f32 / (n - 1).max(1) as f32;
					let w = if rb.arc {
						let t = j as f32 / 20.0;
						rb.width0 + (rb.width1 - rb.width0) * (-3.0 * t * t * t + 5.0 * t * t - t) // Hermite(-1, 0, 1, 0)
					} else {
						rb.width0 + (rb.width1 - rb.width0) * t
					};
					let c: [f32; 4] = std::array::from_fn(|i| rb.colour[i] + (rb.tip_colour[i] - rb.colour[i]) * t);
					let u = if rb.arc { u } else { 0.5 };
					[Vertex { pos: at(p, side, -w, side, 0.0), uv: [u, 1.0], normal: [0.0; 3], colour: c, light: [0.0; 4] }, Vertex { pos: at(p, side, w, side, 0.0), uv: [u, 0.0], normal: [0.0; 3], colour: c, light: [0.0; 4] }]
				};
				let ([a0, a1], [b0, b1]) = (edge(a, k - 1, rb.uv_scroll), edge(b, k, rb.uv_scroll + 1.0));
				out.extend([a0, b0, b1, a1]);
			}
		}
	}
}

pub(super) fn line_shown(m: &[[f32; 4]; 4], cam: [f32; 3], f: [f32; 3], l: &particles::Line, part_far: f32) -> bool {
	let mid: [f32; 3] = std::array::from_fn(|k| (l.a[k] + l.b[k]) * 0.5 - cam[k]);
	let key = mid[0] * f[0] + mid[1] * f[1] + mid[2] * f[2];
	let far = l.far.min(part_far);
	if key - l.near <= 0.0 || key - far >= 0.0 {
		return false;
	}
	clip_inside(m, l.a) && clip_inside(m, l.b)
}

/// The pool's two-point clip test (`vclip` of each point, no flag set): inside the view frustum.
fn clip_inside(m: &[[f32; 4]; 4], p: [f32; 3]) -> bool {
	let c: [f32; 4] = std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r]);
	c[3] > 0.0 && c[0].abs() <= c[3] && c[1].abs() <= c[3] && (0.0..=c[3]).contains(&c[2])
}

/// `part_verts` texture key of a retail pool line run (drawn with `line_pipeline`, 2 vertices per line).
pub(super) const LINE_KEY: u32 = u32::MAX - 1;

/// NOV-22.12 (BRIDGE-44): the mode-1 flat pool sprite corners (VU1 prog2 0x218-0x240) in quad-cycle order P0, P1,
/// P3, P2 = pos + (a, b), (-b, a), (-a, -b), (b, -a), all at pos.z; half-diagonal R = raw size / 420000 =
/// `size * sqrt2 / 4` for the billboard size convention, a = R cos(rot), b = R sin(rot).
pub(super) fn flat_corners(pos: [f32; 3], size: f32, rot: f32) -> [[f32; 3]; 4] {
	let r = size * (std::f32::consts::SQRT_2 / 4.0);
	let (a, b) = (r * rot.cos(), r * rot.sin());
	[[pos[0] + a, pos[1] + b, pos[2]], [pos[0] - b, pos[1] + a, pos[2]], [pos[0] - a, pos[1] - b, pos[2]], [pos[0] + b, pos[1] - a, pos[2]]]
}
