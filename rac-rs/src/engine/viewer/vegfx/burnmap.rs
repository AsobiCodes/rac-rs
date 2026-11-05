//! [PC] Vegetation burns: the burn MAP (lane ngrass; the user: "burning the grass seems to have a delay and a limit"). Replaces the
//! list of at most 16 round burnt areas: a world-space top-down grid of `N` x `N` texels of `TEXEL` m (128 m square) around the
//! camera, re-centred by whole texels once the camera is more than `RECENTRE` m from its centre (the CPU copy is shifted, a burn stays
//! at its world spot, the whole texture is uploaded again). Each texel holds (burn start on the fire's GAME clock, relative to the map's
//! `epoch`; 0 = unburnt, ground height), so the burn follows the flame's path exactly, with no cap.
//!
//! Painting (CPU, every frame): the Pyrocitor's ground touches (firefx_jet_grass.rs, `touch`) heat the texels under the flame (a disc
//! of 1.28 x the flame's half width + `veg_burn_reach`); a texel held `HEAT_IGNITE` s at strength `HEAT_FULL` or more (a gap over `GAP` s starts again)
//! lights. A ground fire patch (firefx_blast.rs) lights its disc at once (`ignite`). A texel still burning is left alone; a burnt or
//! regrowing one touched again turns black at once (no new flames). Only the dirty rectangle is uploaded (`BurnTex::upload`).
//! The shaders (vegfx/shader.rs `veg_burn`) read the texel under each blade / plant root (a little jitter for a ragged edge), each
//! plant lighting up to `LAG_MAX` s late (its own hash), then follow vegfx/burn.rs `timeline`. Nothing live (`is_empty`): count 0.

use super::burn::{BurnU, Times};

/// Texels per side.
pub(in crate::viewer) const N: usize = 512;
/// Texel size (m).
pub(in crate::viewer) const TEXEL: f32 = 0.25;
/// The map re-centres when the camera is this far (m, along x or y) from its centre.
const RECENTRE: f32 = 32.0;
/// Seconds at full strength a texel must be in the flame before it lights.
const HEAT_IGNITE: f32 = 0.15;
/// Touch strength (vegfx stamps, 1 - smoothstep(0.2, 1.0, gap)) from which a texel heats at the full rate, and under which it does
/// not heat: a stream at hip height (~0.7 m between its underside and the ground: strength ~0.32) that visibly bends the grass also
/// lights it (user 2026-11-03, the old spot rule IGNITE_MIN 0.3; a ramp from 0.35 left such a stream at a fifth of the rate, ~1 s).
const HEAT_FULL: f32 = 0.3;
const HEAT_MIN: f32 = 0.25;
/// A gap in the touches longer than this (s) starts the heating again.
const GAP: f32 = 0.2;
/// The longest a plant lights after its texel (its own hash, in the shader).
pub(in crate::viewer) const LAG_MAX: f32 = 0.3;
/// Height difference (m) under which a touch belongs to the same burn (a ledge above is another place).
const SAME_Z: f32 = 1.5;

fn tune(name: &str, d: f32) -> f32 {
	crate::devtune::f32(name, d)
}

/// The CPU half: the grid, its heat and what changed since the last upload.
#[derive(Default)]
pub(in crate::viewer) struct BurnMap {
	/// World texel index of texel (0, 0); None until placed.
	org: Option<[i32; 2]>,
	/// Fire clock of relative time 0 (set at the first paint after a clear, 1 s before it: a lit texel is never 0).
	epoch: Option<f64>,
	/// Per texel (born, ground z), row-major; empty until the first paint.
	born: Vec<[f32; 2]>,
	/// Per texel (exposure s, last touch), same layout.
	heat: Vec<[f32; 2]>,
	/// The newest burn start (relative), and whether a burn may still show.
	last: f32,
	live: bool,
	/// Anything non-zero in the grid (a clear must zero it and re-upload).
	touched: bool,
	/// Dirty texel rectangle (x0, y0, x1, y1), ends exclusive.
	dirty: Option<[usize; 4]>,
}

impl BurnMap {
	fn rel(&mut self, now: f64) -> f32 {
		let e = *self.epoch.get_or_insert(now - 1.0);
		(now - e) as f32
	}

	/// Forgets every burn (level change, Fire Low / Off, Classic, Vegetation Off): the texture is zeroed at the next upload.
	pub(in crate::viewer) fn clear(&mut self) {
		if self.touched {
			self.born.iter_mut().for_each(|t| *t = [0.0; 2]);
			self.heat.iter_mut().for_each(|t| *t = [0.0; 2]);
			self.touched = false;
			self.dirty = Some([0, 0, N, N]);
		}
		(self.org, self.epoch, self.last, self.live) = (None, None, 0.0, false);
	}

	/// Nothing burnt that still shows.
	pub(in crate::viewer) fn is_empty(&self) -> bool {
		!self.live
	}

	/// Keeps the map around `eye`: placed at the first call, then shifted by whole texels when `eye` is over `RECENTRE` m from the
	/// centre (the burns keep their world spots; what leaves the square is forgotten).
	pub(in crate::viewer) fn follow(&mut self, eye: [f32; 3]) {
		let want = [(eye[0] / TEXEL).floor() as i32 - (N / 2) as i32, (eye[1] / TEXEL).floor() as i32 - (N / 2) as i32];
		let Some(org) = self.org else {
			self.org = Some(want);
			return;
		};
		let far = (RECENTRE / TEXEL) as i32;
		if (want[0] - org[0]).abs() > far || (want[1] - org[1]).abs() > far {
			self.shift(want[0] - org[0], want[1] - org[1]);
		}
	}

	fn shift(&mut self, dx: i32, dy: i32) {
		let org = self.org.unwrap_or([0, 0]);
		self.org = Some([org[0] + dx, org[1] + dy]);
		if !self.touched {
			return;
		}
		let n = N as i32;
		let mut nb = vec![[0.0f32; 2]; N * N];
		let mut nh = vec![[0.0f32; 2]; N * N];
		for y in 0..n {
			let sy = y + dy;
			if !(0..n).contains(&sy) {
				continue;
			}
			for x in 0..n {
				let sx = x + dx;
				if (0..n).contains(&sx) {
					let (d, s) = ((y * n + x) as usize, (sy * n + sx) as usize);
					(nb[d], nh[d]) = (self.born[s], self.heat[s]);
				}
			}
		}
		(self.born, self.heat) = (nb, nh);
		self.dirty = Some([0, 0, N, N]);
	}

	/// The texels whose centre lies within `r` of `pos` (row-major indices), placing the map around `pos` if it has no place yet.
	fn disc(&mut self, pos: [f32; 3], r: f32) -> Vec<usize> {
		if self.org.is_none() {
			self.follow(pos);
		}
		if self.born.is_empty() {
			(self.born, self.heat) = (vec![[0.0; 2]; N * N], vec![[0.0; 2]; N * N]);
		}
		let org = self.org.unwrap_or([0, 0]);
		let (cx, cy) = (pos[0] / TEXEL - org[0] as f32, pos[1] / TEXEL - org[1] as f32);
		let rt = r / TEXEL;
		let (x0, x1) = (((cx - rt).floor().max(0.0)) as usize, ((cx + rt).ceil().min(N as f32)).max(0.0) as usize);
		let (y0, y1) = (((cy - rt).floor().max(0.0)) as usize, ((cy + rt).ceil().min(N as f32)).max(0.0) as usize);
		let mut out = Vec::new();
		for y in y0..y1 {
			for x in x0..x1 {
				let (ex, ey) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
				if ex * ex + ey * ey <= rt * rt {
					out.push(y * N + x);
				}
			}
		}
		out
	}

	fn mark_dirty(&mut self, i: usize) {
		let (x, y) = (i % N, i / N);
		self.dirty = Some(match self.dirty {
			Some(d) => [d[0].min(x), d[1].min(y), d[2].max(x + 1), d[3].max(y + 1)],
			None => [x, y, x + 1, y + 1],
		});
	}

	fn light(&mut self, i: usize, born: f32, z: f32) {
		self.born[i] = [born, z];
		self.heat[i] = [0.0; 2];
		(self.touched, self.live, self.last) = (true, true, self.last.max(born));
		self.mark_dirty(i);
	}

	/// The flame on texel `i` at relative time `t`: true when it is handled (still burning: nothing; burnt or regrowing: black again).
	fn on_burnt(&mut self, i: usize, z: f32, t: f32, times: &Times) -> bool {
		let [b, bz] = self.born[i];
		let age = t - b;
		if b <= 0.0 || (bz - z).abs() > SAME_Z || age > times.total() {
			return false;
		}
		if age > times.burn {
			self.light(i, t - times.burn, z);
		}
		true
	}

	/// The Pyrocitor's flame touching the ground at `pos` (disc radius `r`) with strength 0..1 for `dt` s at fire clock `now`.
	pub(in crate::viewer) fn touch(&mut self, pos: [f32; 3], r: f32, strength: f32, dt: f32, now: f64, times: &Times) {
		let rate = ((strength - HEAT_MIN) / (HEAT_FULL - HEAT_MIN)).clamp(0.0, 1.0);
		if rate <= 0.0 {
			return;
		}
		let t = self.rel(now);
		for i in self.disc(pos, r) {
			if self.on_burnt(i, pos[2], t, times) {
				continue;
			}
			let [h, ht] = self.heat[i];
			let h = if ht <= 0.0 || t - ht > GAP || t < ht { 0.0 } else { h } + rate * dt;
			if h >= HEAT_IGNITE {
				self.light(i, t, pos[2]);
			} else {
				self.heat[i] = [h, t];
				self.touched = true;
			}
		}
	}

	/// A ground fire patch at `pos` (radius `r`) lights its disc at once.
	pub(in crate::viewer) fn ignite(&mut self, pos: [f32; 3], r: f32, now: f64, times: &Times) {
		let t = self.rel(now);
		for i in self.disc(pos, r) {
			if !self.on_burnt(i, pos[2], t, times) {
				self.light(i, t, pos[2]);
			}
		}
	}

	/// Marks the map empty once its newest burn has fully regrown. Every texel is regrown then, which draws as unburnt, so the grid is
	/// zeroed (one upload) and `touched` goes down: a camera shift no longer walks and re-uploads the 2 MB map for the rest of the level.
	pub(in crate::viewer) fn expire(&mut self, now: f64, times: &Times) {
		if self.live && self.epoch.is_some_and(|e| (now - e) as f32 - self.last > times.total()) {
			self.live = false;
			if self.touched {
				self.born.iter_mut().for_each(|t| *t = [0.0; 2]);
				self.heat.iter_mut().for_each(|t| *t = [0.0; 2]);
				self.touched = false;
				self.dirty = Some([0, 0, N, N]);
			}
		}
	}

	/// The uniform for this frame: head = (1, burn s, burnt s, regrow s), the look, map[0] = (world x, y of texel (0, 0), 1 / texel,
	/// N), map[1] = (now relative to the epoch, 0, 0, 0). Nothing live: all zero (count 0, the shader skips the map).
	pub(in crate::viewer) fn pack(&self, now: f64, times: &Times) -> BurnU {
		let mut u = BurnU::none();
		let (Some(org), Some(e)) = (self.org, self.epoch) else { return u };
		if !self.live || (now - e) as f32 - self.last > times.total() {
			return u;
		}
		u.head = [1.0, times.burn, times.burnt, times.regrow];
		u.look = [tune("veg_char_h", 0.95).clamp(0.2, 1.0), tune("veg_tip_flame", 0.12).max(0.0), tune("veg_burnt_h", 0.35).clamp(0.05, 1.0), tune("veg_ember", 0.35).max(0.0)];
		u.map[0] = [org[0] as f32 * TEXEL, org[1] as f32 * TEXEL, 1.0 / TEXEL, N as f32];
		u.map[1] = [(now - e) as f32, 0.0, 0.0, 0.0];
		u
	}

	/// The dirty rectangle (x, y, w, h) and its texels packed tight, forgetting it.
	fn take_dirty(&mut self) -> Option<([u32; 4], Vec<[f32; 2]>)> {
		let [x0, y0, x1, y1] = self.dirty.take()?;
		let mut data = Vec::with_capacity((x1 - x0) * (y1 - y0));
		for y in y0..y1 {
			match self.born.get(y * N + x0..y * N + x1) {
				Some(row) => data.extend_from_slice(row),
				None => data.extend(std::iter::repeat_n([0.0f32; 2], x1 - x0)),
			}
		}
		Some(([x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32], data))
	}
}

/// The GPU half: the Rg32Float texture the vegetation shaders read (`veg_burnmap`, group 3 binding 1; zero at creation).
pub(in crate::viewer) struct BurnTex {
	tex: wgpu::Texture,
	pub view: wgpu::TextureView,
}

impl BurnTex {
	pub(in crate::viewer) fn new(device: &wgpu::Device) -> BurnTex {
		let tex = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("veg burn map"),
			size: wgpu::Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rg32Float,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
		BurnTex { tex, view }
	}

	/// The bind group layout entry (vertex stage, unfilterable float: read with textureLoad).
	pub(in crate::viewer) fn layout_entry() -> wgpu::BindGroupLayoutEntry {
		let ty = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
		wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty, count: None }
	}

	/// Uploads what changed since the last call (nothing: no call to the queue).
	pub(in crate::viewer) fn upload(&self, queue: &crate::viewer::presenter::GatedQueue, map: &mut BurnMap) {
		let Some(([x, y, w, h], data)) = map.take_dirty() else { return };
		let texture = wgpu::TexelCopyTextureInfo { texture: &self.tex, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All };
		let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 8), rows_per_image: Some(h) };
		queue.write_texture(texture, bytemuck::cast_slice(&data), layout, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const T: Times = Times { burn: 3.0, burnt: 60.0, regrow: 15.0 };

	/// The texel under a world point: (born relative, z), or None outside the map.
	fn at(m: &BurnMap, x: f32, y: f32) -> Option<[f32; 2]> {
		let o = m.org?;
		let (tx, ty) = ((x / TEXEL).floor() as i32 - o[0], (y / TEXEL).floor() as i32 - o[1]);
		((0..N as i32).contains(&tx) && (0..N as i32).contains(&ty)).then(|| m.born.get((ty as usize) * N + tx as usize).copied().unwrap_or([0.0; 2]))
	}

	fn lit(m: &BurnMap, x: f32, y: f32) -> bool {
		at(m, x, y).is_some_and(|b| b[0] > 0.0)
	}

	#[test]
	fn a_touch_lights_after_a_few_frames_and_a_flick_does_not() {
		let mut m = BurnMap::default();
		m.follow([0.0; 3]);
		// Full strength, 60 fps: not at once, lit within 0.15-0.2 s.
		let mut when = None;
		for k in 0..30 {
			let now = 10.0 + k as f64 / 60.0;
			m.touch([1.0, 1.0, 0.0], 0.5, 1.0, 1.0 / 60.0, now, &T);
			if when.is_none() && lit(&m, 1.0, 1.0) {
				when = Some(k);
			}
		}
		let k = when.expect("lit");
		assert!((5..=12).contains(&k), "lit after {k} frames");
		assert!(!m.is_empty() && m.pack(11.0, &T).head[0] == 1.0);
		// A flick (3 frames) lights nothing; neither does a touch with long gaps.
		let mut f = BurnMap::default();
		for k in 0..3 {
			f.touch([5.0, 5.0, 0.0], 0.5, 1.0, 1.0 / 60.0, k as f64 / 60.0, &T);
		}
		for k in 0..6 {
			f.touch([5.0, 5.0, 0.0], 0.5, 1.0, 1.0 / 60.0, 1.0 + k as f64 * 0.5, &T);
		}
		assert!(!lit(&f, 5.0, 5.0) && f.is_empty() && f.pack(5.0, &T).head == [0.0; 4]);
		// A stream at hip height (strength 0.32, ~0.7 m over the ground) lights as fast as one on the grass.
		let mut hip = BurnMap::default();
		for k in 0..12 {
			hip.touch([0.0; 3], 0.5, 0.32, 1.0 / 60.0, k as f64 / 60.0, &T);
		}
		assert!(lit(&hip, 0.0, 0.0), "hip-height stream lit within 0.2 s");
		// A very weak touch (a jet well above the grass) never lights.
		let mut w = BurnMap::default();
		for k in 0..60 {
			w.touch([0.0; 3], 0.5, 0.2, 1.0 / 60.0, k as f64 / 60.0, &T);
		}
		assert!(w.is_empty());
	}

	#[test]
	fn a_sweep_burns_its_whole_path_with_no_cap() {
		let mut m = BurnMap::default();
		m.follow([0.0; 3]);
		// The stream end crosses 40 m slowly enough to heat each spot: every metre along it is lit (the old list kept 16 circles).
		for k in 0..1200 {
			let x = -20.0 + k as f32 * (40.0 / 1200.0);
			m.touch([x, 3.0, 0.0], 0.6, 1.0, 1.0 / 60.0, k as f64 / 60.0, &T);
		}
		for x in -19..20 {
			assert!(lit(&m, x as f32, 3.0), "burnt at x {x}");
		}
		assert!(!lit(&m, 0.0, 4.0), "nothing beyond the flame's width");
	}

	#[test]
	fn a_patch_lights_at_once_and_burnt_grass_turns_black_again() {
		let mut m = BurnMap::default();
		m.ignite([2.0, 2.0, 1.0], 1.5, 50.0, &T);
		assert!(lit(&m, 2.0, 2.0) && lit(&m, 3.2, 2.0) && !lit(&m, 4.0, 2.0));
		let b0 = at(&m, 2.0, 2.0).unwrap();
		assert_eq!(b0[1], 1.0, "the ground height is kept");
		// Still burning: nothing changes; burnt: black again from the end of the flames.
		m.ignite([2.0, 2.0, 1.0], 1.5, 51.0, &T);
		assert_eq!(at(&m, 2.0, 2.0).unwrap(), b0);
		m.ignite([2.0, 2.0, 1.0], 1.5, 80.0, &T);
		assert!((at(&m, 2.0, 2.0).unwrap()[0] - (b0[0] + 30.0 - 3.0)).abs() < 1e-4);
		// A ledge 3 m higher is another place: it lights fresh.
		m.ignite([2.0, 2.0, 4.0], 0.5, 81.0, &T);
		assert_eq!(at(&m, 2.0, 2.0).unwrap(), [b0[0] + 31.0, 4.0]);
	}

	#[test]
	fn a_burn_keeps_its_world_spot_when_the_map_recentres() {
		let mut m = BurnMap::default();
		m.follow([0.0; 3]);
		m.ignite([10.0, -5.0, 0.0], 1.0, 5.0, &T);
		let (o0, b0) = (m.org.unwrap(), at(&m, 10.0, -5.0).unwrap());
		m.take_dirty();
		m.follow([30.0, 0.0, 0.0]);
		assert_eq!(m.org.unwrap(), o0, "inside 32 m: no shift");
		m.follow([45.0, -40.0, 0.0]);
		let o1 = m.org.unwrap();
		assert!(o1 != o0);
		assert_eq!(at(&m, 10.0, -5.0).unwrap(), b0, "same burn at the same world point");
		assert!(!lit(&m, 10.0 + 3.0, -5.0));
		assert_eq!(m.take_dirty().unwrap().0, [0, 0, N as u32, N as u32], "the whole map is uploaded after a shift");
		// The uniform names the new corner.
		let u = m.pack(6.0, &T);
		assert_eq!(u.map[0], [o1[0] as f32 * TEXEL, o1[1] as f32 * TEXEL, 4.0, N as f32]);
		// Far away (over 64 m): the burn left the square and is gone.
		m.follow([200.0, 0.0, 0.0]);
		assert!(at(&m, 10.0, -5.0).is_none());
	}

	#[test]
	fn a_regrown_map_no_longer_walks_or_uploads_on_a_camera_shift() {
		let mut m = BurnMap::default();
		m.ignite([0.0; 3], 1.0, 100.0, &T);
		m.take_dirty();
		m.expire(100.0 + 3.0 + 60.0 + 15.0 + 0.4, &T);
		assert!(m.is_empty() && !m.touched, "nothing burnt is left");
		assert!(m.take_dirty().is_some(), "the zeroed grid goes up once");
		m.shift(200, 0);
		assert!(m.take_dirty().is_none(), "a shift of a regrown map uploads nothing");
		m.ignite([60.0, 0.0, 0.0], 1.0, 400.0, &T);
		assert!(m.touched && lit(&m, 60.0, 0.0), "a new burn works as before");
	}

	#[test]
	fn the_map_expires_after_the_regrowth_and_clears_on_a_level_change() {
		let mut m = BurnMap::default();
		m.ignite([0.0; 3], 1.0, 100.0, &T);
		let u = m.pack(102.0, &T);
		assert_eq!(u.head, [1.0, 3.0, 60.0, 15.0]);
		assert!((u.map[1][0] - 3.0).abs() < 1e-5, "now relative to the epoch (1 s before the first paint)");
		m.expire(100.0 + 3.0 + 60.0 + 15.0 + 0.2, &T);
		assert!(!m.is_empty(), "the slowest plant is still regrowing");
		m.expire(100.0 + 3.0 + 60.0 + 15.0 + 0.4, &T);
		assert!(m.is_empty() && m.pack(200.0, &T).head == [0.0; 4]);
		// A level change: zeroed, the whole texture re-uploaded once, then nothing more to upload.
		m.ignite([0.0; 3], 1.0, 300.0, &T);
		m.take_dirty();
		m.clear();
		assert!(m.is_empty() && !lit(&m, 0.0, 0.0));
		let (r, data) = m.take_dirty().unwrap();
		assert!(r == [0, 0, N as u32, N as u32] && data.iter().all(|t| *t == [0.0; 2]));
		m.clear();
		assert!(m.take_dirty().is_none(), "clearing an empty map uploads nothing");
	}

	#[test]
	fn only_the_dirty_rectangle_is_uploaded() {
		let mut m = BurnMap::default();
		m.follow([0.0; 3]);
		assert!(m.take_dirty().is_none());
		m.ignite([0.0, 0.0, 0.0], 0.5, 1.0, &T);
		let (r, data) = m.take_dirty().unwrap();
		assert!(r[2] <= 4 && r[3] <= 4 && data.len() == (r[2] * r[3]) as usize, "{r:?}");
		assert!(data.iter().any(|t| t[0] > 0.0));
		assert!(m.take_dirty().is_none());
	}
}
