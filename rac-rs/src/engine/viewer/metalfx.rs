//! Optional stylised metal: per-game texture identities and a separate chrome/glass verdict.
//! Split view: `active` is gated by the Enhanced pass at frame_uniforms, so Classic remains unchanged.

use super::{Draw, Texture};

/// Material bits above the retail mip/wrap word (16..19: flag, subdued, 2 kind bits); array draws relocate all four to spare
/// texture-slot bits 12..15 (`bindless::first_instance`).
pub(super) const DRAW_FLAG: u32 = 1 << 16;
pub(super) const SUBDUED_FLAG: u32 = 1 << 17;
/// The pack's metal kind (`pack::METAL_*`, 2 bits; metalfx.wgsl `(i.metal >> 2) & 3`).
pub(super) const KIND_SHIFT: u32 = 18;
pub(super) const DRAW_FLAGS: u32 = 0xf << 16;

/// The metal shading (included in the level `SHADER`); RAC_METAL_WGSL=<file> replaces it at start-up (`with_override`).
pub(super) const METAL_WGSL: &str = include_str!("metalfx.wgsl");

/// The level shader with the metal part read from RAC_METAL_WGSL (dev tuning without a rebuild); unset or unreadable = built in.
pub(super) fn with_override(src: String) -> String {
	let Some(path) = std::env::var_os("RAC_METAL_WGSL") else { return src };
	match std::fs::read_to_string(&path) {
		Ok(text) if src.contains(METAL_WGSL) => {
			eprintln!("metalfx: metal shader from {}", std::path::Path::new(&path).display());
			src.replace(METAL_WGSL, &text)
		}
		_ => {
			eprintln!("metalfx: RAC_METAL_WGSL {} not used (unreadable or shader changed)", std::path::Path::new(&path).display());
			src
		}
	}
}

/// Enabled by the player setting when this game supplies a material list.
pub(super) fn active() -> bool {
	crate::metal_option::enabled() && !crate::pack::get().metal_textures.is_empty()
}

/// FNV-1a over dimensions (little endian) and RGB, excluding alpha so the identity is independent of the PS2 alpha scaling.
fn texture_hash(t: &Texture) -> u64 {
	let mut h = 0xcbf29ce484222325u64;
	for b in (t.w as u32).to_le_bytes().into_iter().chain((t.h as u32).to_le_bytes()).chain(t.rgba.iter().flat_map(|p| p[..3].iter().copied())) {
		h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
	}
	h
}

/// Look up the original decoded texture before a texture pack replaces its image.
pub(super) fn texture_flag(t: Option<&Texture>) -> u32 {
	if crate::pack::get().metal_textures.is_empty() { return 0; }
	let Some(t) = t else { return 0; };
	if t.w.checked_mul(t.h) != Some(t.rgba.len()) { return 0; }
	let id = texture_hash(t);
	let Ok(row) = crate::pack::get().metal_textures.binary_search_by_key(&id, |r| r.0) else { return 0; };
	let kind = u32::from(crate::pack::get().metal_textures[row].1 & 3) << KIND_SHIFT;
	DRAW_FLAG | kind | if crate::pack::get().metal_subdued_textures.binary_search(&id).is_ok() { SUBDUED_FLAG } else { 0 }
}

/// Mark each static world draw by its original texture slot; mip and wrap bits are preserved.
pub(super) fn mark(draws: &mut [Draw], words: &[u32]) {
	for d in draws { d.inst |= words.get(d.texture).copied().unwrap_or(0) & DRAW_FLAGS; }
}

/// Retail slot 0 is chrome; slot 1 is glass and must keep its ordinary look.
pub(super) fn chrome_instance(d: &Draw) -> u32 {
	if d.texture == 0 { DRAW_FLAG } else { 0 }
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn hash_uses_dimensions_and_rgb_but_not_alpha() {
		let a = Texture { w: 1, h: 1, rgba: vec![[10, 20, 30, 0]] };
		let b = Texture { w: 1, h: 1, rgba: vec![[10, 20, 30, 255]] };
		let c = Texture { w: 1, h: 1, rgba: vec![[10, 20, 31, 0]] };
		let d = Texture { w: 2, h: 1, rgba: vec![[10, 20, 30, 0]] };
		assert_eq!(texture_hash(&a), 0xb3e6666e6575109f); // independently generated from the RGB bytes
		assert_eq!(texture_hash(&a), texture_hash(&b));
		assert_ne!(texture_hash(&a), texture_hash(&c));
		assert_ne!(texture_hash(&a), texture_hash(&d));
	}
	#[test]
	fn material_bits_preserve_mips_and_distinguish_glass() {
		let mut draws = [Draw { inst: 0x9437, cull: 0, texture: 1, start: 0, count: 3 }];
		mark(&mut draws, &[0, DRAW_FLAG | SUBDUED_FLAG]);
		draws[0].texture = 4095;
		let packed = super::super::bindless::first_instance(&draws[0]);
		assert_eq!(packed & 0xfff, 4095);
		assert_eq!((packed >> 12) & 15, 3);
		assert_eq!(packed >> 16, 0x9437);
		draws[0].texture = 1;
		assert_eq!(draws[0].inst, 0x9437 | DRAW_FLAG | SUBDUED_FLAG);
		assert_eq!(chrome_instance(&draws[0]), 0);
		draws[0].texture = 0;
		assert_eq!(chrome_instance(&draws[0]), DRAW_FLAG);
	}
	#[test]
	fn kind_bits_reach_both_paths() {
		let mut draws = [Draw { inst: 0x1234, cull: 0, texture: 7, start: 0, count: 3 }];
		mark(&mut draws, &[0, 0, 0, 0, 0, 0, 0, DRAW_FLAG | (3 << KIND_SHIFT)]);
		assert_eq!(draws[0].inst >> 16, 0b1101);
		let packed = super::super::bindless::first_instance(&draws[0]);
		assert_eq!(packed & 0xfff, 7);
		assert_eq!((packed >> 12) & 15, 0b1101);
		assert_eq!(packed >> 16, 0x1234);
	}
	#[test]
	fn override_needs_the_switch() {
		let src = format!("head {METAL_WGSL} tail");
		if std::env::var_os("RAC_METAL_WGSL").is_none() { assert_eq!(with_override(src.clone()), src); }
		assert!(super::super::SHADER.contains(METAL_WGSL));
	}
	#[test]
	fn shaders_validate_plain_and_array_with_and_without_early_depth() {
		for early in [false, true] {
			let plain = super::super::volfog::soft::level_source(super::super::SHADER, early);
			for src in [plain.clone(), super::super::bindless::shader_source(&plain)] {
				let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
				naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("metal WGSL validates");
			}
		}
	}

	// CPU twins of metalfx.wgsl `metal_ssr_bias`, `metal_ssr_slope`, `metal_ssr` (`march_hiz`: the walk through the nearest-depth
	// chain) and `metal_shade`, plus previews v1 and v2's marches for comparison; `wgsl_keeps_the_twins_lines` pins the shader lines
	// they copy.
	type V = [f32; 3];
	fn sub(a: V, b: V) -> V { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
	fn add(a: V, b: V) -> V { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
	fn mul(a: V, s: f32) -> V { a.map(|x| x * s) }
	fn dot(a: V, b: V) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
	fn norm(a: V) -> V { mul(a, 1.0 / dot(a, a).sqrt()) }
	fn cross(a: V, b: V) -> V { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
	fn lum(c: V) -> f32 { 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2] }
	fn ssr_bias(z: f32) -> f32 { 0.02 + 0.01 * z }
	fn ssr_hit(w0: f32, w: f32, z: f32, thick: f32) -> bool {
		let (dz, e) = (w - z, ssr_bias(z));
		dz > e && dz <= thick + e + (w - w0).abs()
	}
	type Boxes = [(V, V)];
	fn box_hit(b: &(V, V), o: V, d: V) -> Option<f32> {
		let (mut t0, mut t1) = (f32::MIN, f32::MAX);
		for i in 0..3 {
			if d[i].abs() < 1e-12 {
				if o[i] < b.0[i] || o[i] > b.1[i] { return None; }
				continue;
			}
			let (a, c) = ((b.0[i] - o[i]) / d[i], (b.1[i] - o[i]) / d[i]);
			(t0, t1) = (t0.max(a.min(c)), t1.min(a.max(c)));
		}
		(t1 >= t0.max(0.0) && t0 > 0.0).then_some(t0)
	}
	fn first_hit(boxes: &Boxes, o: V, d: V) -> Option<f32> {
		boxes.iter().filter_map(|b| box_hit(b, o, d)).reduce(f32::min)
	}
	/// The reflection source's size (metalenv.rs `capture_ssr`).
	const DW: f32 = 640.0;
	const DH: f32 = 400.0;
	/// A pinhole camera over the floor z = 0 (tan of the half view angle 0.6); `w` = view depth.
	struct Cam { eye: V, f: V, r: V, u: V }
	impl Cam {
		fn new(eye: V, at: V) -> Cam {
			let f = norm(sub(at, eye));
			let r = norm(cross(f, [0.0, 0.0, 1.0]));
			Cam { eye, f, r, u: cross(r, f) }
		}
		fn proj(&self, q: V) -> (f32, f32, f32) {
			let d = sub(q, self.eye);
			let w = dot(d, self.f);
			(dot(d, self.r) / w / 0.6, dot(d, self.u) / w / 0.6, w)
		}
		/// The view ray through screen (x, y), scaled so its length along it is the view depth.
		fn ray(&self, x: f32, y: f32) -> V { add(self.f, add(mul(self.r, x * 0.6), mul(self.u, y * 0.6))) }
		/// The view depth of the floor or the nearest box seen at screen (x, y).
		fn depth(&self, boxes: &Boxes, x: f32, y: f32) -> f32 {
			let d = self.ray(x, y);
			let floor = if d[2] < 0.0 { -self.eye[2] / d[2] } else { 1e9 };
			first_hit(boxes, self.eye, d).map_or(floor, |t| t.min(floor))
		}
		/// The depth source point-sampled at (x, y): its texel holds the nearest of 2x2 sub-samples (post.rs `fs_lin`), so a floor
		/// seen at a slant reads nearer than it is.
		fn depth_tex(&self, boxes: &Boxes, x: f32, y: f32) -> f32 {
			let (ix, iy) = (((x * 0.5 + 0.5) * DW).floor(), ((0.5 - y * 0.5) * DH).floor());
			let mut best = f32::MAX;
			for sx in [0.25, 0.75] {
				for sy in [0.25, 0.75] { best = best.min(self.depth(boxes, ((ix + sx) / DW - 0.5) * 2.0, (0.5 - (iy + sy) / DH) * 2.0)); }
			}
			best
		}
		/// `metal_ssr_slope`: the depth step to the next texel where the depth runs one way across it (the larger axis).
		fn slope(&self, boxes: &Boxes, x: f32, y: f32, z: f32) -> f32 {
			let step = |dx: f32, dy: f32| {
				let (a, b) = (z - self.depth_tex(boxes, x - dx, y - dy), self.depth_tex(boxes, x + dx, y + dy) - z);
				if a * b > 0.0 { a.abs().min(b.abs()) } else { 0.0 }
			};
			step(2.0 / DW, 0.0).max(step(0.0, 2.0 / DH))
		}
		fn mirror(&self, p: V) -> V {
			let v = norm(sub(self.eye, p));
			[-v[0], -v[1], v[2]]
		}
	}
	/// Metal preview v2's march (reach 14, thickness 0.4, up to `steps` samples) of floor point p's mirror ray r with per-pixel
	/// jitter `jit`: the world point the reflection shows (the seen surface at the hit), or None.
	fn march(c: &Cam, boxes: &Boxes, p: V, r: V, jit: f32, steps: f32) -> Option<V> {
		let (reach, thick) = (14.0, 0.4);
		let (ax, ay, aw) = c.proj(p);
		if aw <= 0.1 { return None; }
		let we = c.proj(add(p, mul(r, reach))).2;
		let tr = if we < 0.1 { reach * (aw - 0.1) / (aw - we).max(1e-4) } else { reach };
		let (ex, ey, ew) = c.proj(add(p, mul(r, tr)));
		let n = (((ex - ax) * DW * 0.5).hypot((ey - ay) * DH * 0.5) / 3.0).ceil().clamp(4.0, steps);
		let at = |s: f32| (ax + (ex - ax) * s, ay + (ey - ay) * s, 1.0 / (1.0 / aw + (1.0 / ew - 1.0 / aw) * s));
		let (mut s0, mut w0) = (0.0, aw);
		for k in 0..48 {
			if k as f32 >= n { break; }
			let s = ((k as f32 + jit) / n).powf(1.5);
			let q = at(s);
			if q.0.abs() >= 1.0 || q.1.abs() >= 1.0 { break; }
			let zq = c.depth_tex(boxes, q.0, q.1);
			if ssr_hit(w0, q.2, zq, thick) {
				let g = c.slope(boxes, q.0, q.1, zq);
				let (mut lo, mut hi, mut h, mut wl) = (s0, s, q, w0);
				for _ in 0..5 {
					let m = 0.5 * (lo + hi);
					let b = at(m);
					let z = c.depth_tex(boxes, b.0, b.1);
					if b.2 - z > ssr_bias(z) + g { (hi, h) = (m, b) } else { (lo, wl) = (m, b.2) }
				}
				let z = c.depth_tex(boxes, h.0, h.1);
				let e = ssr_bias(z) + g;
				if h.2 - z > e && h.2 - z <= 0.5 * thick + e + (h.2 - wl).abs() { return Some(add(c.eye, mul(c.ray(h.0, h.1), z))); }
			}
			(s0, w0) = (s, q.2);
		}
		None
	}
	/// Metal preview v1's march: 12 world-space steps over 14 units (quadratic, jitter 0.5), a hit iff 0 < dz <= 0.4 plus the step's
	/// depth travel, a 4-step bisection and no contact check.
	fn march_v1(c: &Cam, boxes: &Boxes, p: V, r: V) -> Option<V> {
		let at = |t: f32| c.proj(add(p, mul(r, t)));
		let (mut t0, mut w0) = (0.0, c.proj(p).2);
		for k in 0..12 {
			let f = (k as f32 + 0.5) / 12.0;
			let t = 0.08 + 14.0 * f * f;
			let (x, y, w) = at(t);
			if w <= 0.05 || x.abs() >= 1.0 || y.abs() >= 1.0 { return None; }
			let z = c.depth_tex(boxes, x, y);
			if w - z > 0.0 && w - z <= 0.4 + (w - w0).abs() {
				let (mut lo, mut hi, mut h) = (t0, t, (x, y, z));
				for _ in 0..4 {
					let m = 0.5 * (lo + hi);
					let (x, y, w) = at(m);
					let z = c.depth_tex(boxes, x, y);
					if w - z > 0.0 { (hi, h) = (m, (x, y, z)) } else { lo = m }
				}
				return Some(add(c.eye, mul(c.ray(h.0, h.1), h.2)));
			}
			(t0, w0) = (t, w);
		}
		None
	}
	/// The reflection depth of one view (`Cam::depth_tex` per texel) and its nearest-depth chain (metalenv.rs `HIZ` levels, `fs_hiz`).
	struct Tex { lv: Vec<(usize, usize, Vec<f32>)> }
	impl Tex {
		fn new(c: &Cam, boxes: &Boxes) -> Tex {
			let (w, h) = (DW as usize, DH as usize);
			let l0: Vec<f32> = (0..w * h).map(|i| c.depth_tex(boxes, (((i % w) as f32 + 0.5) / DW - 0.5) * 2.0, (0.5 - ((i / w) as f32 + 0.5) / DH) * 2.0)).collect();
			let mut lv = vec![(w, h, l0)];
			for _ in 1..super::super::metalenv::HIZ {
				let (pw, p) = { let (pw, _, p) = lv.last().unwrap(); (*pw, p) };
				let (w, h) = (pw / 2, lv.last().unwrap().1 / 2);
				let at = |x: usize, y: usize| p[y * pw + x];
				let v: Vec<f32> = (0..w * h).map(|i| { let (x, y) = (i % w * 2, i / w * 2); at(x, y).min(at(x + 1, y)).min(at(x, y + 1)).min(at(x + 1, y + 1)) }).collect();
				lv.push((w, h, v));
			}
			Tex { lv }
		}
		/// `textureLoad` (level-0 reads clamp to the edge as `metal_ssr_depth`).
		fn at(&self, l: usize, x: i32, y: i32) -> f32 {
			let (w, h, v) = &self.lv[l];
			v[y.clamp(0, *h as i32 - 1) as usize * w + x.clamp(0, *w as i32 - 1) as usize]
		}
		/// `metal_ssr_slope` at level-0 texel (x, y).
		fn slope(&self, x: i32, y: i32, z: f32) -> f32 {
			let step = |dx: i32, dy: i32| {
				let (a, b) = (z - self.at(0, x - dx, y - dy), self.at(0, x + dx, y + dy) - z);
				if a * b > 0.0 { a.abs().min(b.abs()) } else { 0.0 }
			};
			step(1, 0).max(step(0, 1))
		}
	}
	/// `metal_ssr`: the walk through every texel the ray crosses (cells it stays in front of skipped whole), at most `iters` cells.
	/// The side-entry window (an eighth of the thickness) was chosen on this scene: a half let rays passing just behind the pole and
	/// the lamp head count as hits (wrong 109/70/224 of good 995/738/582), none dropped the sides of objects (grain 535/104/230);
	/// an eighth: good 950/715/631, wrong 33/12/44, grain 13/21/18 (v2: good 856/658/479, wrong 75/39/68, grain 109/121/87).
	fn march_hiz(c: &Cam, t: &Tex, p: V, r: V, iters: usize) -> Option<V> {
		let (reach, thick, top) = (14.0f32, 0.4f32, super::super::metalenv::HIZ as i32 - 1);
		let (ax, ay, aw) = c.proj(p);
		if aw <= 0.1 { return None; }
		let we = c.proj(add(p, mul(r, reach))).2;
		let tr = if we < 0.1 { reach * (aw - 0.1) / (aw - we).max(1e-4) } else { reach };
		let (ex, ey, ew) = c.proj(add(p, mul(r, tr)));
		let tx = |x: f32, y: f32| [(x * 0.5 + 0.5) * DW, (0.5 - y * 0.5) * DH];
		let (p0, p1) = (tx(ax, ay), tx(ex, ey));
		let d = [p1[0] - p0[0], p1[1] - p0[1]];
		let len = d[0].abs().max(d[1].abs());
		if len < 1.0 { return None; }
		let dd = d.map(|v| if v.abs() < 1e-6 { 1e-6 } else { v });
		let exit = |q: [f32; 2], l: i32| {
			let cs = (1 << l) as f32;
			let e = |k: usize| (((q[k] / cs).floor() + if dd[k] > 0.0 { 1.0 } else { 0.0 }) * cs - p0[k]) / dd[k];
			e(0).min(e(1))
		};
		let (eps, ia, ib) = (0.01 / len, 1.0 / aw, 1.0 / ew);
		let (mut lvl, mut s) = (0, exit(p0, 0) + eps);
		for _ in 0..iters.min(128) {
			if s >= 1.0 { break; }
			let q = [p0[0] + d[0] * s, p0[1] + d[1] * s];
			if q[0] < 0.0 || q[1] < 0.0 || q[0] >= DW || q[1] >= DH { break; }
			let se = exit(q, lvl).min(1.0);
			let cs = (1 << lvl) as f32;
			let cell = [(q[0] / cs).floor() as i32, (q[1] / cs).floor() as i32];
			let (w0, w1) = (1.0 / (ia + (ib - ia) * s), 1.0 / (ia + (ib - ia) * se));
			let z = t.at(lvl as usize, cell[0], cell[1]);
			if w0.max(w1) <= z + ssr_bias(z) {
				(s, lvl) = (se + eps, (lvl + 1).min(top));
				continue;
			}
			if lvl > 0 {
				lvl -= 1;
				continue;
			}
			let e = ssr_bias(z) + t.slope(cell[0], cell[1], z);
			let sh = if w0 > z + e { (w0 - z <= 0.125 * thick + e).then_some(s) } else if w1 > z + e { Some(((1.0 / (z + e) - ia) / (ib - ia)).clamp(s, se)) } else { None };
			if let Some(sh) = sh {
				let (hx, hy) = (ax + (ex - ax) * sh, ay + (ey - ay) * sh);
				return Some(add(c.eye, mul(c.ray(hx, hy), z)));
			}
			s = se + eps;
		}
		None
	}
	/// Grain: floor pixels whose verdict (hit or not) differs from both horizontal or both vertical neighbours (the dithered
	/// speckle of a per-pixel jitter), over a 200x125 view.
	fn grain(c: &Cam, boxes: &Boxes, m: impl Fn(V, V, f32) -> Option<V>) -> u32 {
		let mut hit = vec![None; 200 * 125];
		for py in 0..125 {
			for px in 0..200 {
				let (x, y) = ((px as f32 + 0.5) / 100.0 - 1.0, 1.0 - (py as f32 + 0.5) / 62.5);
				let d = c.ray(x, y);
				if d[2] >= 0.0 { continue; }
				let fl = -c.eye[2] / d[2];
				if first_hit(boxes, c.eye, d).is_some_and(|t| t < fl) { continue; }
				let p = add(c.eye, mul(d, fl));
				let jit = (52.9829189 * ((px as f32 + 0.5) * 0.06711056 + (py as f32 + 0.5) * 0.00583715).fract()).fract();
				hit[py * 200 + px] = Some(m(p, c.mirror(p), jit).is_some());
			}
		}
		let at = |x: usize, y: usize| hit[y * 200 + x];
		let mut n = 0;
		for y in 1..124 {
			for x in 1..199 {
				let Some(h) = at(x, y) else { continue };
				let odd = |a: Option<bool>, b: Option<bool>| a.is_some_and(|a| a != h) && b.is_some_and(|b| b != h);
				n += (odd(at(x - 1, y), at(x + 1, y)) || odd(at(x, y - 1), at(x, y + 1))) as u32;
			}
		}
		n
	}
	/// The march's verdicts over a 200x125 view's floor pixels against the true mirror hit: right (within 0.25), wrong (of these, the
	/// floor mirroring itself: a hit on the floor, which an upward mirror ray never meets) and missed.
	#[derive(Default, Debug)]
	struct Score { good: u32, wrong: u32, floor: u32, miss: u32 }
	fn score(c: &Cam, boxes: &Boxes, m: impl Fn(V, V, f32) -> Option<V>) -> Score {
		let mut s = Score::default();
		for py in 0..125 {
			for px in 0..200 {
				let (x, y) = ((px as f32 + 0.5) / 100.0 - 1.0, 1.0 - (py as f32 + 0.5) / 62.5);
				let d = c.ray(x, y);
				if d[2] >= 0.0 { continue; }
				let fl = -c.eye[2] / d[2];
				if first_hit(boxes, c.eye, d).is_some_and(|t| t < fl) { continue; }
				let p = add(c.eye, mul(d, fl));
				let r = c.mirror(p);
				let truth = first_hit(boxes, p, r).filter(|&t| t < 14.0).map(|t| add(p, mul(r, t)));
				let jit = (52.9829189 * ((px as f32 + 0.5) * 0.06711056 + (py as f32 + 0.5) * 0.00583715).fract()).fract();
				match (m(p, r, jit), truth) {
					(None, t) => s.miss += t.is_some() as u32,
					(Some(h), Some(t)) if dot(sub(h, t), sub(h, t)) < 0.25 * 0.25 => s.good += 1,
					(Some(h), _) => (s.wrong, s.floor) = (s.wrong + 1, s.floor + (h[2] < 0.1) as u32),
				}
			}
		}
		s
	}
	/// The hero, a held gun (an overhang), a lamp post with its flat head (another) and a bed.
	const SCENE: [(V, V); 5] = [
		([-0.3, -0.3, 0.0], [0.3, 0.3, 1.8]),
		([0.3, -0.12, 0.55], [1.1, 0.12, 0.75]),
		([2.0, 1.0, 0.0], [2.1, 1.1, 3.0]),
		([1.6, 0.6, 3.0], [2.5, 1.5, 3.15]),
		([-2.5, 2.0, 0.0], [-1.0, 3.0, 0.8]),
	];
	/// Preview v1's smears: a ray passing UNDER the held gun or a lamp head (behind its silhouette) took its colour (a stretched
	/// copy of the gun and legs under the hero), and the slanted floor's depth read nearer than the floor, so the floor mirrored
	/// itself a few pixels up (streaks dragged down the screen). Preview v2 fixed both with a contact check past the depth error, but
	/// its jittered samples still stepped over thin parts (grain, `ssr_has_no_grain`). Now the walk meets the first surface the ray
	/// touches, texel by texel.
	#[test]
	fn ssr_mirrors_what_the_ray_meets() {
		let c = Cam::new([0.0, -6.0, 4.0], [0.0, 0.0, 0.8]);
		let t = Tex::new(&c, &SCENE[..1]);
		for p in [[0.0, -0.6, 0.0], [0.0, -1.0, 0.0]] {
			let r = c.mirror(p);
			let truth = add(p, mul(r, first_hit(&SCENE[..1], p, r).expect("the mirror ray meets the hero")));
			let h = march_hiz(&c, &t, p, r, 64).unwrap_or_else(|| panic!("contact reflection at {p:?}"));
			assert!(dot(sub(h, truth), sub(h, truth)) < 0.25 * 0.25, "contact reflection at {p:?}: {h:?} vs {truth:?}");
		}
		let (mut good, mut wrong) = (0, 0);
		for (eye, at) in [([1.5, -4.5, 3.0], [0.6, 0.3, 0.6]), ([0.5, -7.0, 2.5], [0.6, 0.3, 0.6]), ([3.0, -5.0, 4.0], [0.5, 0.5, 0.8])] {
			let c = Cam::new(eye, at);
			let t = Tex::new(&c, &SCENE);
			let v1 = score(&c, &SCENE, |p, r, _| march_v1(&c, &SCENE, p, r));
			let v2 = score(&c, &SCENE, |p, r, j| march(&c, &SCENE, p, r, j, 24.0));
			let v3 = score(&c, &SCENE, |p, r, _| march_hiz(&c, &t, p, r, 64));
			eprintln!("{eye:?}: v1 {v1:?} v2 {v2:?} v3 {v3:?}");
			assert!(v1.floor >= 200 && v1.wrong >= v1.good / 4, "the scene shows v1's smears from {eye:?}: {v1:?}");
			assert!(v3.wrong * 100 <= v3.good * 15, "wrong hits from {eye:?}: {v3:?}");
			assert!(v3.floor * 100 <= v3.good, "the floor mirrors itself from {eye:?}: {v3:?}");
			assert!(v3.good >= v2.good, "right hits from {eye:?}: {v2:?} -> {v3:?}");
			(good, wrong) = (good + v3.good, wrong + v3.wrong);
		}
		assert!(wrong * 10 <= good, "wrong hits overall: {wrong} of {good}");
	}
	/// With nothing above it, the floor reflects nothing: no texel of an upward mirror ray may count the floor as met.
	#[test]
	fn ssr_floor_never_mirrors_itself() {
		for (eye, at) in [([1.5, -4.5, 3.0], [0.6, 0.3, 0.6]), ([0.5, -7.0, 1.2], [0.0, 3.0, 0.0]), ([4.0, -3.0, 2.0], [0.0, 2.0, 0.5])] {
			let c = Cam::new(eye, at);
			let t = Tex::new(&c, &[]);
			let v1 = score(&c, &[], |p, r, _| march_v1(&c, &[], p, r));
			let v3 = score(&c, &[], |p, r, _| march_hiz(&c, &t, p, r, 64));
			assert!(v1.wrong >= 1000, "v1 mirrored the floor from {eye:?}: {v1:?}");
			assert!(v3.wrong == 0, "the floor mirrors itself from {eye:?}: {v3:?} (v1 {v1:?})");
		}
	}
	/// Preview v2's per-pixel jitter made neighbouring pixels sample the ray at different points, so thin parts (ears, the gun, a
	/// pole) were met by every other pixel: a dithered mirror image (user, 2026-11-03). The walk has no jitter and steps over
	/// nothing: what neighbours see differs only where the scene does.
	#[test]
	fn ssr_has_no_grain() {
		let (mut g2, mut g3) = (0, 0);
		for (eye, at) in [([1.5, -4.5, 3.0], [0.6, 0.3, 0.6]), ([0.5, -7.0, 2.5], [0.6, 0.3, 0.6]), ([3.0, -5.0, 4.0], [0.5, 0.5, 0.8])] {
			let c = Cam::new(eye, at);
			let t = Tex::new(&c, &SCENE);
			let (a, b) = (grain(&c, &SCENE, |p, r, j| march(&c, &SCENE, p, r, j, 24.0)), grain(&c, &SCENE, |p, r, _| march_hiz(&c, &t, p, r, 64)));
			eprintln!("grain from {eye:?}: v2 {a} v3 {b}");
			(g2, g3) = (g2 + a, g3 + b);
		}
		assert!(g3 * 5 <= g2 && g3 <= 60, "isolated hit/miss pixels: v2 {g2} -> v3 {g3}");
	}

	fn shade(base: V, env: V, around: V, ndv: f32, rough: f32, metalness: f32, k: [f32; 4], g: [f32; 4]) -> V {
		let s = 1.0 - rough.clamp(0.0, 1.0);
		let le = lum(env);
		let rl = (le / lum(around).max(0.01)).clamp(0.0, 2.0);
		let tint = g[2].clamp(0.0, 1.0);
		let graze = (1.0 - ndv.clamp(0.0, 1.0)).powf(5.0);
		let fw = (g[0] + (g[1] - g[0]) * graze).clamp(0.0, 1.0);
		let (hw, wr, wg) = ((k[2] * s * s * (1.0 - tint)).clamp(0.0, 1.0), (k[0] * s * fw).clamp(0.0, 1.0), (graze * s * s * k[0] * g[1]).clamp(0.0, 1.0));
		let f = k[1] + (s.max(k[1]) - k[1]) * graze;
		let lb = lum(base).max(1e-3);
		std::array::from_fn(|i| {
			let hue = 1.0 + (env[i] / le.max(1e-4) - 1.0) * hw;
			let rel = base[i] * (1.0 + (rl * hue - 1.0) * wr);
			let own = (1.0 + ((base[i] / lb).clamp(0.0, 2.0) - 1.0) * tint).min(1.0);
			let metal = rel + (env[i] * own - rel) * wg;
			let plain = base[i] + (env[i] - base[i]) * f;
			plain + (metal - plain) * metalness.clamp(0.0, 1.0)
		})
	}
	/// The knob defaults (metalenv.rs `knobs` m[0]: reflection, dielectric F0, reflected-colour share, highlight; m[10]: strength
	/// looking down, strength at grazing angles, own-colour tint, texture break-up).
	const K: [f32; 4] = [0.75, 0.04, 0.35, 0.8];
	const G: [f32; 4] = [0.45, 0.85, 0.6, 0.5];
	/// v3 as a twin: full strength at every angle, no tint (the "mirror" look the user asked to tone down).
	const G_V3: [f32; 4] = [1.0, 1.0, 0.0, 0.0];
	/// Test build 9 lifted metal toward white (a grey plate under a bright sky: ~0.39 linear for a retail 0.11). Now the metal keeps
	/// its retail brightness and shows the reflection relative to the light it already has.
	#[test]
	fn metal_keeps_its_retail_brightness() {
		let (base, sky) = ([0.12, 0.11, 0.10], [0.55, 0.62, 0.70]);
		let near = |a: V, b: V| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-6);
		for ndv in [0.1, 0.5, 1.0] {
			assert!(near(shade(base, sky, [0.2; 3], ndv, 1.0, 1.0, K, G), base), "rough metal is the retail colour");
		}
		for rough in [0.05, 0.22, 0.45] {
			assert!(near(shade(base, [0.6; 3], [0.6; 3], 1.0, rough, 1.0, K, G), base), "plain surroundings face-on: the retail colour");
		}
		let mean = [0.3f32, 0.9].iter().map(|&e| lum(shade(base, [e; 3], [0.6; 3], 1.0, 0.22, 1.0, K, G))).sum::<f32>() / 2.0;
		assert!((mean - lum(base)).abs() < 1e-5, "reflections average out to the retail brightness: {mean}");
		let o = shade(base, sky, [0.5, 0.56, 0.64], 0.7, 0.22, 1.0, K, G);
		assert!(lum(o) <= 2.0 * lum(base) && lum(o) < 0.15, "a bright sky in a polished plate: {o:?}");
		let dark = lum(shade(base, [0.02; 3], sky, 0.7, 0.22, 1.0, K, G)) / lum(base);
		assert!(dark < 0.85 && dark > 0.4, "a dark object reflected darkens it, glossy: not a mirror's full drop ({dark})");
		assert!((lum(shade(base, sky, sky, 1.0, 0.38, 0.3, K, G)) - lum(base)).abs() < 0.03, "painted: the retail colour and a thin coat");
		assert!(lum(shade(base, sky, sky, 0.05, 0.05, 1.0, K, G)) > lum(base) * 2.0, "grazing on mirror chrome: the surroundings");
	}
	/// User 2026-11-03 on v3: "metal is reflective ofc but it isnt a mirror so we need to somehow tone down the reflections".
	/// Looking down, a reflection moves the metal less far than v3 did; at grazing angles it stays strong; a coloured metal tints
	/// what it reflects.
	#[test]
	fn metal_is_glossy_not_a_mirror() {
		let base = [0.12, 0.11, 0.10];
		let bright = [0.9; 3];
		for env in [bright, [0.02; 3]] {
			let (v5, v3) = (shade(base, env, [0.3; 3], 1.0, 0.05, 1.0, K, G), shade(base, env, [0.3; 3], 1.0, 0.05, 1.0, K, G_V3));
			let (d5, d3) = ((lum(v5) - lum(base)).abs(), (lum(v3) - lum(base)).abs());
			assert!(d5 < 0.7 * d3, "looking down: v5 moves {d5}, v3 {d3}");
		}
		let up = lum(shade(base, bright, [0.3; 3], 1.0, 0.05, 1.0, K, G)) / lum(base);
		assert!(up <= 1.65, "a bright reflection looking down lifts a plate by at most 65 % ({up})");
		let graze = shade(base, bright, [0.3; 3], 0.05, 0.05, 1.0, K, G);
		assert!(lum(graze) > 2.0 * lum(base), "grazing: still a strong reflection {graze:?}");
		let (gold, blue) = ([0.30, 0.20, 0.05], [0.2, 0.4, 0.9]);
		let blueness = |c: V| c[2] / lum(c);
		let (t5, t0) = (shade(gold, blue, [0.3; 3], 0.2, 0.05, 1.0, K, G), shade(gold, blue, [0.3; 3], 0.2, 0.05, 1.0, K, [G[0], G[1], 0.0, G[3]]));
		assert!(blueness(t5) < 0.8 * blueness(t0), "gold tints a blue sky: {t5:?} vs untinted {t0:?}");
	}
	/// The tests' knobs are the shipped defaults (review R24 m4: G was v5's 0.3 looking down).
	#[test]
	fn test_knobs_are_the_defaults() {
		let m = super::super::metalenv::knobs(false, None);
		// metalenv `knobs`: metal_refl 1.0 x the Metal reflections default 75 % = K[0] 0.75, the approved look's gain (lane nmet: knob 0.75, no slider).
		assert_eq!((m[0], m[10]), (K, G), "the reflection gain is the knob (1.0) times the default 75 % = K");
	}
	/// CPU twin of metalfx.wgsl `metal_env_w`: the share of `env` in `shade`.
	fn env_w(ndv: f32, rough: f32, metalness: f32, k: [f32; 4], g: [f32; 4]) -> f32 {
		let s = 1.0 - rough.clamp(0.0, 1.0);
		let graze = (1.0 - ndv.clamp(0.0, 1.0)).powf(5.0);
		let (wm, wp) = ((graze * s * s * k[0] * g[1]).clamp(0.0, 1.0), k[1] + (s.max(k[1]) - k[1]) * graze);
		wp + (wm - wp) * metalness.clamp(0.0, 1.0)
	}
	/// One frame of a grey metal pixel (retail 0.10, surroundings 0.05, open sky blocked) whose screen hit is its own last frame `b`
	/// (metal facing metal; worst case: the miss reads it too): CPU twin of `metal_look`'s shade plus the MK_POP gloss.
	fn self_frame(b: f32, ndv: f32, rough: f32, metalness: f32) -> f32 {
		let (base, around, ha, vis) = (0.10f32, 0.05f32, 1.0 - rough / 0.5, 0.0f32);
		let lin = shade([base; 3], [b; 3], [around; 3], ndv, rough, metalness, K, G)[1];
		let fw = (G[0] + (G[1] - G[0]) * (1.0 - ndv).powf(5.0)).clamp(0.0, 1.0);
		let pw = (1.2 * metalness * (K[0] * (1.0 - rough) * fw).clamp(0.0, 1.0)).min((0.7 - env_w(ndv, rough, metalness, K, G)).max(0.0) / (ha * (1.0 - vis)).max(1e-4));
		lin + b * ha * (1.0 - vis) * pw
	}
	/// Review R24 M4: the hit is the next frame's source, so metal facing metal fed its own light back at a gain over 1 toward grazing
	/// angles (1.2 at ndv 0.1: a runaway glow). The hit's own light now counts at most MK_HIT_MAX = 0.7: every kind converges.
	#[test]
	fn metal_facing_metal_converges() {
		for (rough, m) in [(0.05, 1.0), (0.22, 1.0), (0.45, 1.0), (0.38, 0.3)] {
			for ndv in [0.02, 0.1, 0.3, 0.6, 1.0] {
				let mut b = [0.10f32; 61];
				for n in 1..61 {
					b[n] = self_frame(b[n - 1], ndv, rough, m);
				}
				assert!((b[60] - b[59]).abs() < 1e-3 * b[60], "converged: rough {rough} ndv {ndv}: {} -> {}", b[59], b[60]);
				let gain = self_frame(11.0, ndv, rough, m) - self_frame(10.0, ndv, rough, m);
				assert!(gain <= 0.7 + 1e-3, "gain {gain} at rough {rough} ndv {ndv}");
				assert!(b[60] <= 5.0 * 0.10, "bounded: {} at rough {rough} ndv {ndv}", b[60]);
			}
		}
	}
	/// CPU twin of skyvis.wgsl `skyvis_sun` with the gap hook `gap(q) -> (behind, covered)`.
	fn sun_open(p: V, r: V, gap: impl Fn(V) -> (f32, bool)) -> (f32, bool) {
		fn sm(e0: f32, e1: f32, x: f32) -> f32 {
			let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
			t * t * (3.0 - 2.0 * t)
		}
		let (mut prev, mut have, mut all_behind, mut covered, mut blocked, mut t0) = (0.0f32, false, 1.0f32, false, 0.0f32, 0.0f32);
		for k in 0..6 {
			let x = k as f32 / 5.0;
			let t = 0.4 + 12.0 * x * x;
			let (g, cov) = gap(add(p, mul(r, t)));
			let tol = 1.5 + (t - t0);
			t0 = t;
			if !cov {
				(all_behind, have) = (0.0, false);
				continue;
			}
			covered = true;
			let behind = sm(-0.25, 0.25, g);
			all_behind = all_behind.min(behind);
			if have {
				let through = sm(0.0, 0.25, prev) * (1.0 - sm(0.6 * tol, tol, prev)) * (1.0 - sm(0.6 * tol, tol, -g));
				blocked = blocked.max(through * (1.0 - behind));
			}
			(prev, have) = (g, true);
		}
		if covered {
			blocked = blocked.max(all_behind);
		}
		(1.0 - blocked, covered)
	}
	/// User 2026-11-03: "some reflections are visible THROUGH walls ... the planets reflection even tho its behind a wall". The
	/// shadow-map test: a roof seen from below blocks the sky, open ground does not, a tall window does not, a closed hall does.
	#[test]
	fn sky_is_blocked_by_a_roof_not_by_a_window() {
		// A light straight from above; `top(q)` = the height of the first surface over q's column (the ground at 0).
		fn gap(top: impl Fn(V) -> f32) -> impl Fn(V) -> (f32, bool) {
			move |q: V| (top(q) - q[2], true)
		}
		fn roof(h: f32, half: f32) -> impl Fn(V) -> f32 {
			move |q: V| if q[0].abs() < half && q[1].abs() < half { h } else { 0.0 }
		}
		let up = norm([0.2, 0.1, 1.0]);
		assert!(sun_open([0.0, 0.0, 0.1], up, gap(roof(5.0, 50.0))).0 < 0.05, "a roof 5 above, from below");
		assert!(sun_open([0.0, 0.0, 0.1], up, gap(|_| 0.0)).0 > 0.95, "open ground");
		assert_eq!(sun_open([0.0, 0.0, 0.1], up, |_| (0.0, false)), (1.0, false), "no map: open");
		assert!(sun_open([0.0, 0.0, 0.1], up, gap(roof(40.0, 50.0))).0 < 0.05, "a closed hall: every point behind");
		let out = norm([1.0, 0.0, 0.15]);
		assert!(sun_open([3.0, 0.0, 0.5], out, gap(roof(12.0, 5.0))).0 > 0.95, "out through a tall window beside the hall");
		assert!(sun_open([-30.0, 0.0, 0.5], up, gap(roof(5.0, 5.0))).0 > 0.95, "a shed elsewhere does not block");
	}
	/// CPU twin of skyvis.wgsl `skyvis_screen`'s path end: clip coords `a` of the point and `c` of the ray's direction -> (end in ndc,
	/// trust); None = off the picture.
	fn screen_end(a: [f32; 4], c: [f32; 4]) -> Option<([f32; 2], f32)> {
		if a[3] <= 0.1 {
			return None;
		}
		let na = [a[0] / a[3], a[1] / a[3]];
		if na[0].abs().max(na[1].abs()) >= 1.0 {
			return None;
		}
		let mut e = [c[0] / c[3].max(1e-4), c[1] / c[3].max(1e-4)];
		let out = e[0].abs().max(e[1].abs()) - 1.0;
		let ss = |x: f32| {
			let t = (x / 0.6).clamp(0.0, 1.0);
			t * t * (3.0 - 2.0 * t)
		};
		let trust = if c[3] > 1e-4 { 1.0 - ss(out) } else { 0.0 };
		if c[3] <= 1e-4 || out > 0.0 {
			let dir = [c[0] - na[0] * c[3], c[1] - na[1] * c[3]];
			let dd = dir.map(|v| if v.abs() < 1e-6 { 1e-6 } else { v });
			let t = [(dd[0].signum() - na[0]) / dd[0], (dd[1].signum() - na[1]) / dd[1]];
			let te = t[0].min(t[1]).max(0.0);
			e = [(na[0] + dd[0] * te).clamp(-1.0, 1.0), (na[1] + dd[1] * te).clamp(-1.0, 1.0)];
		}
		Some((e, trust))
	}
	/// CPU twin of metalfx.wgsl `skyvis_cone` over a reflection source `dim` texels whose texel (x, y) saw the sky where `sky(x, y)`
	/// (metalenv.rs `fs_ssr` alpha; level l of its mip chain = the mean over 2^l x 2^l texels; trilinear, clamp to edge).
	fn cone(dim: [u32; 2], sky: &dyn Fn(i32, i32) -> bool, uv: [f32; 2], rad: f32) -> f32 {
		let level = |l: u32, x: i32, y: i32| {
			let (w, h) = ((dim[0] >> l) as i32, (dim[1] >> l) as i32);
			let (x, y, b) = (x.clamp(0, w - 1), y.clamp(0, h - 1), 1i32 << l);
			let n = (0..b * b).filter(|k| sky(x * b + k % b, y * b + k / b)).count();
			n as f32 / (b * b) as f32
		};
		let bilin = |l: u32, q: [f32; 2]| {
			let f = [q[0] * (dim[0] >> l) as f32 - 0.5, q[1] * (dim[1] >> l) as f32 - 0.5];
			let (b, fr) = ([f[0].floor(), f[1].floor()], [f[0] - f[0].floor(), f[1] - f[1].floor()]);
			let (x, y) = (b[0] as i32, b[1] as i32);
			let top = level(l, x, y) * (1.0 - fr[0]) + level(l, x + 1, y) * fr[0];
			let bot = level(l, x, y + 1) * (1.0 - fr[0]) + level(l, x + 1, y + 1) * fr[0];
			top * (1.0 - fr[1]) + bot * fr[1]
		};
		let lod = (rad * dim[1] as f32).max(1.0).log2().clamp(0.0, 5.0);
		let (l0, t) = (lod.floor() as u32, lod - lod.floor());
		let tri = |q: [f32; 2]| {
			let q = [q[0].clamp(0.0, 1.0), q[1].clamp(0.0, 1.0)];
			bilin(l0, q) * (1.0 - t) + if t > 0.0 { bilin(l0 + 1, q) * t } else { 0.0 }
		};
		let o = [rad * dim[1] as f32 / dim[0] as f32 * 0.5, rad * 0.5];
		let s = tri([uv[0] + o[0], uv[1] + o[1]]) + tri([uv[0] - o[0], uv[1] + o[1]]) + tri([uv[0] + o[0], uv[1] - o[1]]) + tri([uv[0] - o[0], uv[1] - o[1]]);
		let x = ((s * 0.25 - 0.25) / 0.5).clamp(0.0, 1.0);
		x * x * (3.0 - 2.0 * x)
	}
	/// Agent 17: the screen sky test reads a footprint (a cone around the ray), not one point: a thin far post at the vanishing point
	/// (the dark band down Blarg's hangar floor) leaves the sky open, a wall or ceiling closes it; the result slides smoothly as a
	/// skyline crosses it; a footprint reaching past the screen edge has no step. Painted metal 0.38, the Deck's 640 x 400 source.
	#[test]
	fn sky_cone_ignores_thin_posts() {
		assert!(super::super::metalenv::BLIT.contains("z >= 700.0") && METAL_WGSL.contains("const MK_SKY_Z: f32 = 700.0;"), "fs_ssr marks the sky as MK_SKY_Z does");
		let (dim, rad) = ([640u32, 400u32], 0.03 + 0.25 * 0.38);
		let post = |x: i32, _y: i32| !(320..328).contains(&x);
		assert!(cone(dim, &post, [0.51, 0.4], rad) > 0.99, "an 8-texel post at the vanishing point: open");
		assert!(cone(dim, &post, [0.51, 0.4], 0.03) > 0.5, "mirror-smooth metal (the smallest footprint): mostly open");
		let wall = |x: i32, _y: i32| x < 160;
		assert!(cone(dim, &wall, [0.6, 0.4], rad) < 0.01, "a wall: closed");
		assert!(cone(dim, &|_x: i32, y: i32| y > 200, [0.5, 0.2], rad) < 0.01, "a ceiling: closed");
		let mut last = cone(dim, &wall, [0.1, 0.4], rad);
		assert!(last > 0.99, "open sky beside the wall");
		for k in 1..=200 {
			let v = cone(dim, &wall, [0.1 + k as f32 * 0.001, 0.4], rad);
			assert!((v - last).abs() < 0.03 && v <= last + 1e-5, "a skyline slides in smoothly ({k}: {last} -> {v})");
			last = v;
		}
		assert!(last < 0.01, "behind the wall's edge: closed");
		let edge = |x: i32, _y: i32| x < 600;
		let mut last = cone(dim, &edge, [0.9, 0.5], rad);
		for k in 1..=50 {
			let v = cone(dim, &edge, [0.9 + k as f32 * 0.002, 0.5], rad);
			assert!(v.is_finite() && (v - last).abs() < 0.1, "no step at the screen edge ({k}: {last} -> {v})");
			last = v;
		}
	}
	/// Where the screen test looks: the vanishing point when it is on screen (trust 1), else the edge the ray's screen path runs into,
	/// with the trust falling smoothly (no line on the floor where the vanishing point leaves the screen); behind the camera trust 0.
	#[test]
	fn screen_test_reads_the_vanishing_point_or_the_edge() {
		// Camera at the origin looking along +y, x right, z up, 90 degrees: clip = (x, z, _, y) for points and directions alike.
		let clip = |v: V| [v[0], v[2], 0.0, v[1]];
		let p = [0.5, 4.0, -1.0];
		let (e, t) = screen_end(clip(p), clip(norm([0.2, 1.0, 0.5]))).expect("on screen");
		assert!((e[0] - 0.2).abs() < 1e-5 && (e[1] - 0.5).abs() < 1e-5 && t == 1.0, "vanishing point {e:?} {t}");
		let (e, t) = screen_end(clip(p), clip(norm([0.0, 1.0, 3.0]))).expect("off the top");
		assert!((e[1] - 1.0).abs() < 1e-5 && t == 0.0 && e[0].abs() < 1.0, "steep up: the top edge, no trust ({e:?} {t})");
		let (e, t) = screen_end(clip(p), clip(norm([0.3, -1.0, 0.4]))).expect("back past the camera");
		assert!(t == 0.0 && e[0].abs().max(e[1].abs()) > 1.0 - 1e-5, "behind: an edge, no trust ({e:?})");
		let mut last = 1.0f32;
		for k in 0..=40 {
			let up = 0.8 + 0.02 * k as f32;
			let t = screen_end(clip(p), clip(norm([0.0, 1.0, up]))).expect("on").1;
			assert!(t <= last && last - t < 0.1, "trust falls smoothly: {last} -> {t} at {up}");
			last = t;
		}
		assert!(screen_end(clip([5.0, 2.0, 0.0]), clip(norm([0.0, 1.0, 0.0]))).is_none(), "a point off the picture");
	}
	/// CPU twin of metalfx.wgsl `metal_sky`'s face choice and seam weight: (largest axis, second, weight of the second face).
	fn seam(d: V) -> (usize, usize, f32) {
		let a = d.map(f32::abs);
		let (i, j) = if a[0] >= a[1] && a[0] >= a[2] {
			(0, if a[1] >= a[2] { 1 } else { 2 })
		} else if a[1] >= a[2] {
			(1, if a[0] >= a[2] { 0 } else { 2 })
		} else {
			(2, if a[0] >= a[1] { 0 } else { 1 })
		};
		let x = ((a[i] - a[j]) / a[i].max(1e-4) / 0.15).clamp(0.0, 1.0);
		(i, j, 0.5 - 0.5 * x * x * (3.0 - 2.0 * x))
	}
	/// The seam line on the floor (v3 p0459 x~905): each sky face is filtered on its own; blended near the boundary the sky is
	/// continuous across |x| = |y| (two faces that differ by a whole step meet at their mean from both sides).
	#[test]
	fn sky_faces_blend_across_the_seam() {
		let face = |k: usize, d: V| if d[k] > 0.0 { 2 * k } else { 2 * k + 1 };
		let col = |f: usize| f as f32 * 0.1;
		let sky = |d: V| {
			let (i, j, w) = seam(d);
			col(face(i, d)) * (1.0 - w) + col(face(j, d)) * w
		};
		for e in [1e-3f32, 1e-2] {
			let (a, b) = (sky(norm([1.0, 1.0 - e, 0.3])), sky(norm([1.0 - e, 1.0, 0.3])));
			assert!((a - b).abs() < 0.02, "across |x| = |y|: {a} vs {b}");
		}
		assert_eq!(seam(norm([1.0, 0.5, 0.2])).2, 0.0, "away from the seam: one face");
		assert!((seam(norm([1.0, 1.0, 0.2])).2 - 0.5).abs() < 1e-6, "on the seam: half each");
	}
	/// CPU twin of metalfx.wgsl `metal_lobe_axes`: the two offsets of the glossy lobe's four directions.
	fn lobe_axes(r: V, rough: f32) -> (V, V) {
		let cross = |a: V, b: V| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
		let h = 0.5 * (0.1 + 0.6 * rough);
		let ax = if r[2].abs() > 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
		let a = norm(cross(r, ax)).map(|c| c * h);
		(a, cross(r, a))
	}
	/// Agent 18 (Blarg's hangar): a far window edge (sky on one side of an azimuth, a wall on the other) seen in a painted floor is a
	/// gradient across the four lobe directions, not a line: the open share steps through 1/4, 1/2, 3/4 over the lobe's width.
	#[test]
	fn sky_lobe_softens_a_far_edge() {
		for r in [norm([1.0, 0.2, 0.3]), norm([0.1, 0.1, 1.0]), norm([-0.3, 1.0, -0.2])] {
			let (a, b) = lobe_axes(r, 0.38);
			let dot = |x: V, y: V| x[0] * y[0] + x[1] * y[1] + x[2] * y[2];
			assert!(dot(a, r).abs() < 1e-5 && dot(b, r).abs() < 1e-5 && dot(a, b).abs() < 1e-5, "perpendicular at {r:?}");
			assert!((dot(a, a).sqrt() - 0.164).abs() < 1e-3 && (dot(b, b).sqrt() - 0.164).abs() < 1e-3, "half the lobe angle");
		}
		// Open where the azimuth is past `edge`; the floor's reflected rays sweep across it.
		let open = |d: V, edge: f32| if d[1].atan2(d[0]) > edge { 1.0f32 } else { 0.0 };
		let lobe = |r: V| {
			let (a, b) = lobe_axes(r, 0.38);
			let add = |s: f32, t: V| norm([r[0] + s * t[0], r[1] + s * t[1], r[2] + s * t[2]]);
			[add(1.0, a), add(-1.0, a), add(1.0, b), add(-1.0, b)].iter().map(|&d| open(d, 0.0)).sum::<f32>() / 4.0
		};
		let mut seen = Vec::new();
		for k in -40..=40 {
			let az = k as f32 * 0.01;
			let v = lobe(norm([az.cos(), az.sin(), 0.3]));
			if v > 0.0 && v < 1.0 { seen.push(v); }
		}
		assert!(seen.len() >= 20, "the edge spreads over >= 0.2 rad of azimuth: {}", seen.len());
	}
	#[test]
	fn wgsl_keeps_the_twins_lines() {
		for line in [
			"const MK_RMAX: f32 = 2.0;",
			"return 0.02 + 0.01 * z;",
			"if (we < 0.1) { tr = reach * (a.z - 0.1) / max(a.z - we, 1e-4); }",
			"return max(select(0.0, min(abs(ax), abs(bx)), ax * bx > 0.0), select(0.0, min(abs(ay), abs(by)), ay * by > 0.0));",
			"let t = ((floor(q / cs) + side) * cs - p0) * inv;",
			"var s = metal_ssr_exit(p0, inv, side, p0, 0) + eps;",
			"if (max(w0, w1) <= z + metal_ssr_bias(z)) { s = se + eps; lvl = min(lvl + 1, MK_HIZ_TOP); continue; }",
			"if (lvl > 0) { lvl -= 1; continue; }",
			"let e = metal_ssr_bias(z) + metal_ssr_slope((vec2<f32>(cell) + 0.5) / dim, dim, z);",
			"let lim = 0.125 * thick + e;",
			"if (w0 > z + e) { if (w0 - z <= lim) { sh = s; } } else if (w1 > z + e) { sh = clamp((1.0 / (z + e) - ia) / (ib - ia), s, se); }",
			"if (wp < 0.5) { hit = metal_ssr(i.w, r, rough); hit.a *= 1.0 - 2.0 * wp; }",
			"return vec4<f32>(c * c, fade * smoothstep(0.5, 1.0, cov));",
			"let lift = mix(max(1.0, MK_INDOOR_LIFT * metal_lum(miss) / max(metal_lum(oa), 1e-4)), 1.0, ha);",
			"let around = mix(oa * lift, metal_world(n, 1.0, 0.0), vis);",
			"pop = hit.rgb * (ha * (1.0 - vis) * popw);",
			"const MK_HIT_MAX: f32 = 0.7;",
			"let pw = min(MK_POP * metalness * clamp(k.x * (1.0 - rough) * fw, 0.0, 1.0), max(MK_HIT_MAX - metal_env_w(ndv, rough, metalness, k, u.metal[10]), 0.0) / max(ha * (1.0 - vis), 1e-4));",
			"lin += pop * pw;",
			"let w = (vec2<f32>(2.0) - abs(o - 1.0 - fr)) * 0.25;",
			"let rl = clamp(le / max(metal_lum(around), 0.01), 0.0, MK_RMAX);",
			"let hue = mix(vec3<f32>(1.0), env / max(le, 1e-4), clamp(k.z * s * s * (1.0 - tint), 0.0, 1.0));",
			"let fw = clamp(g.x + (g.y - g.x) * graze, 0.0, 1.0);",
			"let rel = base * mix(vec3<f32>(1.0), rl * hue, clamp(k.x * s * fw, 0.0, 1.0));",
			"let own = min(mix(vec3<f32>(1.0), clamp(base / max(metal_lum(base), 1e-3), vec3<f32>(0.0), vec3<f32>(2.0)), tint), vec3<f32>(1.0));",
			"let metal = mix(rel, env * own, ew.x);",
			"let plain = mix(base, env, ew.y);",
			"return vec2<f32>(clamp(graze * s * s * k.x * g.y, 0.0, 1.0), k.y + (max(s, k.y) - k.y) * graze);",
			"return mix(w.y, w.x, clamp(metalness, 0.0, 1.0));",
			"var lin = metal_shade(base, env, around, ndv, rough, metalness, k, u.metal[10]);",
			"let w = 0.5 - 0.5 * smoothstep(0.0, 0.15, (ai - aj) / max(ai, 1e-4));",
			"const MK_SKY_LOBE: vec2<f32> = vec2<f32>(0.1, 0.6);",
			"let h = 0.5 * (MK_SKY_LOBE.x + MK_SKY_LOBE.y * rough);",
			"let ax = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(r.z) > 0.9);",
			"return mat2x3<f32>(a, cross(r, a));",
		] {
			assert!(METAL_WGSL.contains(line), "{line}");
		}
		let skyvis = include_str!("skyvis.wgsl");
		for line in [
			"const SKYVIS_REACH: f32 = 12.0;",
			"let t = 0.4 + SKYVIS_REACH * x * x;",
			"let tol = 1.5 + (t - t0);",
			"if (g.y < 0.5) { all_behind = 0.0; have = false; continue; }",
			"let behind = smoothstep(-0.25, 0.25, g.x);",
			"let through = smoothstep(0.0, 0.25, prev) * (1.0 - smoothstep(0.6 * tol, tol, prev)) * (1.0 - smoothstep(0.6 * tol, tol, -g.x));",
			"blocked = max(blocked, through * (1.0 - behind));",
			"if (covered) { blocked = max(blocked, all_behind); }",
			"if (max(abs(na.x), abs(na.y)) >= 1.0) { return vec2<f32>(1.0, 0.0); }",
			"if (c.w > 1e-4) { trust = 1.0 - smoothstep(0.0, 0.6, max(abs(e.x), abs(e.y)) - 1.0); }",
			"let dir = c.xy - na * c.w;",
			"e = clamp(na + dd * max(min(t.x, t.y), 0.0), vec2<f32>(-1.0), vec2<f32>(1.0));",
			"return sun.x * mix(1.0, scr.x, max(scr.y, 1.0 - sun.y));",
		] {
			assert!(skyvis.contains(line), "{line}");
		}
		assert!(super::super::SHADER.contains(skyvis), "the level shader holds skyvis.wgsl");
		let top = format!("const MK_HIZ_TOP: i32 = {};", super::super::metalenv::HIZ - 1);
		assert!(METAL_WGSL.contains(&top), "{top}");
	}
}
