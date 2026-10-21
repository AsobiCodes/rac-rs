//! The planet-to-planet transit pieces of mode 6 (decomp/notes/level_transition.md SCN-4.1 / SCN-4.3): the arrival warp
//! tick (substate 4, `FUN_002a2ce0`) and the departure flight (substate 3, `FUN_002a39b0` case 3). State machines only:
//! the flight is driven by viewer/travel.rs `Step::Flight` (SCN-4.3a); the warp draw pass (SCN-4.2), its viewer wiring
//! and the pose history feed are not ported yet.

use super::mouse::wrap_angle;
use super::pyro::approach;
use crate::gameplay::{PAL_DT, PAL_DT2};
use crate::particles::Rng;
use crate::physics::V3;
use crate::shadows::fast_atan2;

/// Projection scalar floor per stream type (f32 table 0x1be048).
pub const WARP_FLOOR: [f32; 5] = [0.3, 0.3, 0.3, 0.3, 0.63];
/// Star-fan angle base per type (0x1be030).
pub const FAN_BASE: [f32; 5] = [0.0, 0.0, 0.0, 0.0, 3.0];
/// Star-fan phase direction per type (0x1bdfe0).
pub const FAN_DIR: [[f32; 4]; 5] = [[0.6, 0.15, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0], [0.9, -0.4, 0.0, 1.0], [-0.3, -0.9, 0.0, 1.0], [0.4, 0.4, 0.0, 1.0]];
/// The drift vector a segment starts with (0x160520).
pub const DRIFT0: [f32; 4] = [-370.0, -280.0, 280.0, 0.0];
/// Drift step per tick for type 4, times k (0x160540).
pub const DRIFT_STEP: [f32; 4] = [0.3, 0.5, 0.0, 0.0];
/// Fade decrement per tick (OV:102529).
pub const WARP_FADE_STEP: f32 = 0.25;
/// Ticks per stream chunk (`0x5f < cc98`).
pub const WARP_CHUNK_TICKS: u32 = 96;
/// The type-4 actor shrink window (0x38 ticks) and factor (1/56).
pub const SHRINK_WINDOW: u32 = 0x38;
pub const SHRINK: f32 = f32::from_bits(0x3c92_4925);

/// What one warp tick asks of the caller.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WarpTick {
	/// The sequence is over (a segment ended with `seg > 1`): no draw this tick, the pending level change proceeds.
	pub done: bool,
	/// Play warp sound index `ty` (the first tick of a real destination).
	pub sound: Option<usize>,
	/// `cc98` passed 0x5f: parse the next chunk (the global tick keeps counting).
	pub next_chunk: bool,
	/// A segment ended and a new stream (`ty`) was loaded this tick.
	pub new_segment: bool,
	/// Floor of the camera key scalar (the key's own scalar is raised to at least this).
	pub proj_floor: f32,
	/// Star-fan phase vector (raw sky units) and the fan angle base.
	pub phase: [f32; 4],
	pub fan_base: f32,
	/// Type 4 only: the ship actor is hidden (level 0 / level 1 before Kerwan).
	pub hidden: bool,
	/// Type 4 only: the actor scale multiplier while shrinking (None outside the last 0x38 ticks).
	pub shrink: Option<f32>,
	/// Type 4 only: the three ribbon colour alpha bytes.
	pub ribbon_alpha: u8,
}

/// The arrival warp (SCN-4.1), per game tick at 50 Hz.
#[derive(Clone, Debug)]
pub struct Warp {
	pub ty: usize,
	pub seg: u32,
	pub tick: u32,
	pub cc98: u32,
	pub end: u32,
	pub fade: f32,
	pub drift: [f32; 4],
	hist: [(V3, V3); 32],
	pub head: usize,
	pub count: usize,
	to: usize,
	kerwan: bool,
}

impl Warp {
	/// The loader `FUN_002a5198`: `load(ty)` returns the tick count of the stream loaded for that type.
	pub fn new(to: usize, kerwan: bool, mut load: impl FnMut(usize) -> u32) -> Warp {
		let mut ty = Rng::shared().small(4) as usize;
		let mut seg = 0;
		if to == 0 || (to == 1 && !kerwan) {
			seg = 2;
			ty = 4;
		}
		Warp { ty, seg, tick: 0, cc98: 0, end: load(ty), fade: 1.0, drift: DRIFT0, hist: [([0.0; 3], [0.0; 3]); 32], head: 0, count: 0, to, kerwan }
	}

	/// Retail mode's warp mirror (viewer/travel/mirror_warp.rs): a warp whose state and history are retail's RAM; the
	/// caller sets the public fields.
	pub fn mirror(to: usize, kerwan: bool, hist: [(V3, V3); 32]) -> Warp {
		Warp { ty: 0, seg: 0, tick: 0, cc98: 0, end: 0, fade: 0.0, drift: DRIFT0, hist, head: 0, count: 0, to, kerwan }
	}

	/// Level 0, or level 1 before Kerwan is unlocked: the short silent warp.
	fn quiet(&self) -> bool {
		self.to == 0 || (self.to == 1 && !self.kerwan)
	}

	/// One tick (`FUN_002a2ce0`, steps 1-5 and 7; step 6's pose and history go through `push_history`).
	pub fn tick(&mut self, mut load: impl FnMut(usize) -> u32) -> WarpTick {
		let mut out = WarpTick::default();
		// (1)
		self.fade = (self.fade - WARP_FADE_STEP).max(0.0);
		self.tick += 1;
		self.cc98 += 1;
		// (2)
		if self.tick == 1 && !self.quiet() {
			out.sound = Some(self.ty);
		}
		// (3)
		if self.tick < self.end {
			if self.cc98 > 0x5f {
				self.cc98 = 0;
				out.next_chunk = true;
			}
		} else {
			// The port loads synchronously, so the load is always ready at a segment end.
			if self.seg > 1 {
				out.done = true;
				return out;
			}
			self.ty = if self.seg == 0 { (self.ty + Rng::shared().small(3) as usize + 1) & 3 } else { 4 };
			self.seg += 1;
			self.head = 0;
			self.count = 0;
			self.drift = DRIFT0;
			self.tick = 0;
			self.cc98 = 0;
			self.end = load(self.ty);
			out.new_segment = true;
		}
		// (4)
		out.proj_floor = WARP_FLOOR[self.ty];
		// (5)
		let k = if self.ty == 4 {
			let k = (self.end - self.tick) as f32 / self.end as f32;
			for (d, s) in self.drift.iter_mut().zip(DRIFT_STEP) {
				*d += k * s;
			}
			k
		} else {
			1.0
		};
		let base = (self.tick as f32 - 100.0) * 20.0 * k;
		out.phase = FAN_DIR[self.ty].map(|d| base * d);
		out.fan_base = FAN_BASE[self.ty];
		// (7)
		if self.ty == 4 {
			if self.quiet() {
				out.hidden = true;
				self.count = 0;
			}
			if self.tick + SHRINK_WINDOW > self.end {
				let left = (self.end - self.tick) as f32;
				out.shrink = Some(left * SHRINK);
				out.ribbon_alpha = (self.end - self.tick) as u8;
			} else {
				out.ribbon_alpha = SHRINK_WINDOW as u8;
			}
		}
		out
	}

	/// Step 6: push the pose's joint 1 and 2 world positions into the history ring (once per iteration, two per tick).
	pub fn push_history(&mut self, p1: V3, p2: V3) {
		self.head = (self.head + 1) & 31;
		self.hist[self.head] = (p1, p2);
		self.count = (self.count + 1).min(32);
	}

}

/// The fan's per-shell (euler y, euler z offset, scale) (OV:99533-99558); shells 0 and 1 are identical.
pub const FAN_SHELLS: [(f32, f32, f32); 6] = [(0.0, 0.0, 1.0), (0.0, 0.0, 1.0), (-0.075, -0.15, 1.25), (0.05, 0.125, 1.5), (0.1, -0.05, 1.75), (-0.15, 0.1, 2.0)];
/// Fan projection scalar floor (the fan only).
pub const FAN_TAN_FLOOR: f32 = 0.63;
/// Big-quad size per destination (0x1bdf90) and the quad's corner offsets (0x1bdf50).
pub const BIG_SCALE: [f32; 19] = [0.8, 1.0, 0.9, 1.4, 0.9, 1.1, 1.3, 0.8, 1.2, 1.5, 1.1, 0.7, 1.3, 1.2, 1.1, 1.0, 1.2, 1.1, 0.8];
pub const BIG_CORNERS: [[f32; 3]; 4] = [[-80.0, 0.0, 80.0], [80.0, 0.0, 80.0], [-80.0, 0.0, -80.0], [80.0, 0.0, -80.0]];
/// Ribbon colour words A (older end) and B (newer end) per ship mark, little-endian RGBA bytes (0x1bdf30).
pub const RIBBON_A: [u32; 3] = [0x3880_a0c0, 0x3880_a0c0, 0x3880_c080];
pub const RIBBON_B: [u32; 3] = [0x0020_58b0, 0x0020_58b0, 0x0030_8030];
/// Ribbon half-width at the middle (`W[mk]`), and the per-step parameter.
pub const RIBBON_W: f32 = 0.8;
/// `fx_tex` key the big quad is uploaded under in the warp table (the ribbon texture is slot 0).
pub const WARP_BIG_TEX: usize = 0x100;

/// One frame of the arrival warp's draw pass (SCN-4.2), everything but the ship and the gloss.
#[derive(Clone, Debug, Default)]
pub struct WarpFrame {
	/// Star fan: (shell, scaled rotation rows, translation in sky units).
	pub fan: Vec<(usize, [[f32; 3]; 3], V3)>,
	/// Projection scalar of the fan (the camera key's, at least 0.63).
	pub fan_tan: f32,
	/// World quads in draw order: the big quad (type 4), then the ribbon strips (tex 0, additive).
	pub quads: Vec<crate::particles::WorldQuad>,
	/// Banner colour word `(a << 24) | 0x808080` (type 4, tick > 60).
	pub banner: Option<u32>,
	/// Black fade quad alpha, 0..=128 (0 = none).
	pub fade: u8,
}

/// `a + (b - a) * t` per byte, truncated (the ribbon colour lerp).
fn lerp_bytes(a: u32, b: u32, t: f32) -> [f32; 4] {
	std::array::from_fn(|k| {
		let (x, y) = (((a >> (8 * k)) & 0xff) as f32, ((b >> (8 * k)) & 0xff) as f32);
		(x + (y - x) * t).trunc() / 128.0
	})
}

fn v_sub(a: V3, b: V3) -> V3 {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn v_cross(a: V3, b: V3) -> V3 {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// `src * (w / |src|)`, zero for a zero vector (`FUN_00220f40`).
fn v_norm_to(src: V3, w: f32) -> V3 {
	let l = (src[0] * src[0] + src[1] * src[1] + src[2] * src[2]).sqrt();
	if l == 0.0 {
		return [0.0; 3];
	}
	let k = w / l;
	[src[0] * k, src[1] * k, src[2] * k]
}

/// The ribbon strips of a two-ring history (the arrival warp's and the departure flight's): for each step `i` in
/// `0..count - 1` the older entry is `head - i - 1`, the newer `head - i`, the even newer `head - i + 1` (the pair
/// itself for `i == 0`); two rings, two strips each (side vectors, then chords), `mk` picks the colour words, `alpha`
/// is the A alpha byte and `tex` the fx texture of every quad.
pub fn ribbon_quads(ring: &[(V3, V3); 32], head: usize, count: usize, mk: usize, alpha: u8, tex: usize) -> Vec<crate::particles::WorldQuad> {
	let mut out = Vec::new();
	let at = |back: usize| ring[(head + 32 - back) & 31];
	let a_word = (RIBBON_A[mk.min(2)] & 0x00ff_ffff) | (alpha as u32) << 24;
	let b_word = RIBBON_B[mk.min(2)];
	for i in 0..count.saturating_sub(1) {
		let (old, new) = (at(i + 1), at(i));
		let next = if i == 0 { None } else { Some(at(i - 1)) };
		let d0 = v_sub(old.1, old.0);
		let d2 = v_sub(new.1, new.0);
		let nd0 = d0.map(|x| -x);
		let nd2 = d2.map(|x| -x);
		let (t0, t1) = ((i + 1) as f32 * 0.031_25, i as f32 * 0.031_25);
		let (w0, w1) = ((1.0 - t0 * t0) * RIBBON_W, (1.0 - t1 * t1) * RIBBON_W);
		let cols = [lerp_bytes(a_word, b_word, t0), lerp_bytes(a_word, b_word, t0), lerp_bytes(a_word, b_word, t1), lerp_bytes(a_word, b_word, t1)];
		for r in 0..2 {
			let pick = |e: (V3, V3)| if r == 0 { e.0 } else { e.1 };
			let (p0, p1) = (pick(old), pick(new));
			let e1 = v_sub(p1, p0);
			let e2 = next.map_or(e1, |n| v_sub(pick(n), p1));
			let sides = [
				[v_norm_to(v_cross(e1, d0), w0), v_norm_to(v_cross(d0, e1), w0), v_norm_to(v_cross(e2, d2), w1), v_norm_to(v_cross(d2, e2), w1)],
				[v_norm_to(d0, w0), v_norm_to(nd0, w0), v_norm_to(d2, w1), v_norm_to(nd2, w1)],
			];
			for s in sides {
				let base = [p0, p0, p1, p1];
				let v: Vec<V3> = (0..4).map(|k| [base[k][0] + s[k][0], base[k][1] + s[k][1], base[k][2] + s[k][2]]).collect();
				let uv = [[0.0, 0.5], [1.0, 0.5], [0.0, 0.5], [1.0, 0.5]];
				// Strip order v0 v1 v2 v3, submitted as the quad [v0, v1, v3, v2].
				let o = [0, 1, 3, 2];
				out.push(crate::particles::WorldQuad { pos: o.map(|k| v[k]), uv: o.map(|k| uv[k]), colour: o.map(|k| cols[k]), tex, additive: true });
			}
		}
	}
	out
}

impl Warp {
	/// The ribbons and the gloss are drawn unless the destination is level 0, or level 1 before Kerwan.
	pub fn ribbons_shown(&self) -> bool {
		!self.quiet()
	}

	/// Star fan: shell `k` of `shells`, `M_k = scale_k * euler_m([0, ey_k, wrap(base + ez_k)])` (rebuilt each shell,
	/// not cumulative) and the phase vector over 1024 as translation.
	pub fn fan(shells: usize, tick: &WarpTick) -> Vec<(usize, [[f32; 3]; 3], V3)> {
		(0..shells.min(6))
			.map(|k| {
				let (ey, ez, scale) = FAN_SHELLS[k];
				let m = crate::particles::euler_m([0.0, ey, wrap_angle(tick.fan_base, ez)]);
				(k, m.map(|r| r.map(|x| x * scale)), [tick.phase[0] / 1024.0, tick.phase[1] / 1024.0, tick.phase[2] / 1024.0])
			})
			.collect()
	}

	/// The big quad's four world corners: the table corners times `S[to]` (1.0 from destination 0x13) plus the drift.
	pub fn big_quad(&self) -> [V3; 4] {
		let s = BIG_SCALE.get(self.to).copied().unwrap_or(1.0);
		BIG_CORNERS.map(|c| [c[0] * s + self.drift[0], c[1] * s + self.drift[1], c[2] * s + self.drift[2]])
	}

	/// Banner colour word: type 4 after tick 60, alpha `min((tick - 60) * 2, 128)`.
	pub fn banner_word(&self) -> Option<u32> {
		(self.ty == 4 && self.tick > 60).then(|| ((((self.tick - 60) * 2).min(128)) << 24) | 0x0080_8080)
	}

	/// Fade quad alpha: `trunc(min(fade, 1) * 128)` while `fade > 0`.
	pub fn fade_alpha(&self) -> u8 {
		if self.fade > 0.0 { (self.fade.min(1.0) * 128.0) as u8 } else { 0 }
	}

	/// Ribbon strips of the history ring for ship mark `mk`; `alpha` is the A colour's alpha byte (0x38, or
	/// `end - tick` in the last 0x38 ticks of a type-4 segment). Warp texture 0.
	pub fn ribbons(&self, mk: usize, alpha: u8) -> Vec<crate::particles::WorldQuad> {
		if !self.ribbons_shown() {
			return Vec::new();
		}
		ribbon_quads(&self.hist, self.head, self.count, mk, alpha, 0)
	}

	/// The whole frame after a tick: fan, big quad, ribbons, banner and fade. `shells` = the warp sky's shell count,
	/// `key_tan` the camera key's projection scalar (already floored), `last` the tick's result, `mk` the ship mark.
	pub fn draw_frame(&self, last: &WarpTick, key_tan: f32, shells: usize, mk: usize) -> WarpFrame {
		let mut quads = Vec::new();
		if self.ty == 4 {
			let c = self.big_quad();
			let uv = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
			let o = [0, 1, 3, 2];
			quads.push(crate::particles::WorldQuad { pos: o.map(|k| c[k]), uv: o.map(|k| uv[k]), colour: [[1.0; 4]; 4], tex: WARP_BIG_TEX, additive: false });
		}
		let alpha = if self.ty == 4 { last.ribbon_alpha } else { SHRINK_WINDOW as u8 };
		quads.extend(self.ribbons(mk, alpha));
		WarpFrame { fan: Warp::fan(shells, last), fan_tan: key_tan.max(FAN_TAN_FLOOR), quads, banner: self.banner_word(), fade: self.fade_alpha() }
	}
}

/// The class a warp stream's slot-0 actor (class 0x215) is replaced by: the ship of mark `mk` (OV:47561-47563).
pub fn warp_ship_class(mk: usize) -> u32 {
	0x213 + mk as u32
}

/// Ship mark: `mk = 0; unlock[8] || to > 7 -> 1; unlock[14] || to > 13 -> 2` (OV:104228-104235).
pub fn ship_mark(unlock8: bool, unlock14: bool, to: usize) -> usize {
	let mut mk = 0;
	if unlock8 || to > 7 {
		mk = 1;
	}
	if unlock14 || to > 13 {
		mk = 2;
	}
	mk
}

// ---- SCN-4.3: the departure flight --------------------------------------------------------------------------------

/// Bank limit (20 degrees).
pub const BANK_LIMIT: f32 = 0.349_065_84;
/// Ribbon edge points (model units) per ship type: ring 0, ring 1 (0x1bdeb0).
pub const RIBBON_EDGE: [[V3; 2]; 3] = [[[-3.5, 1.5, 1.2], [-3.5, -1.5, 1.2]], [[-3.5, 1.8, 2.6], [-3.5, -1.8, 2.6]], [[-3.5, 1.2, 1.2], [-3.5, -1.2, 1.2]]];

/// A passenger seat in the world (SCN-4.3c): the ship's position plus the seat offset through the ship's rotation
/// (`euler_m` rows, the same sum the ship prop's draw uses).
pub fn seat_pos(ship_pos: V3, ship_rot: [f32; 4], seat: V3) -> V3 {
	let m = crate::particles::euler_m([ship_rot[0], ship_rot[1], ship_rot[2]]);
	std::array::from_fn(|k| ship_pos[k] + m[0][k] * seat[0] + m[1][k] * seat[1] + m[2][k] * seat[2])
}

/// The ship's blend frames while flying: sequence 1 frame 0 into sequence 2 frame `f1` (`FUN_0026c140(ship, 1, 0)`),
/// `n1` = frame count of sequence 1; retail picks frame `min(1, n1 - 1)` and falls back to 0 past the count.
pub fn ship_blend_frames(n1: usize) -> (usize, usize) {
	let f1 = 1.min(n1.saturating_sub(1));
	(0, if f1 >= n1 { 0 } else { f1 })
}

/// Passenger model offsets per ship type: Ratchet (0x1be180) and Clank (0x1be1b0).
pub const RATCHET_SEAT: [V3; 3] = [[1.6, 0.45, 0.9], [2.0, 0.5, 2.1], [0.7, 0.3, 0.7]];
pub const CLANK_SEAT: [V3; 3] = [[1.6, -0.45, 1.3], [2.0, -0.5, 2.6], [0.7, -0.3, 1.25]];

fn wrap(x: f32) -> f32 {
	wrap_angle(x, 0.0)
}

/// `a - b` wrapped into [-pi, pi) (FUN_00221b28 on a and -b).
fn wrapsub(a: f32, b: f32) -> f32 {
	wrap_angle(a, -b)
}

fn yaw(d: V3) -> f32 {
	fast_atan2(d[0], d[1])
}

fn horizontal(d: V3) -> f32 {
	(d[0] * d[0] + d[1] * d[1]).sqrt()
}

fn len3(d: V3) -> f32 {
	(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

fn sub3(a: V3, b: V3) -> V3 {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn lerp3(f: f32, a: V3, b: V3) -> V3 {
	[a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
}

/// The path bank angles (once per flight): `w[i]` = turn at point i, scaled so the largest interior turn is 20 degrees.
pub fn path_banks(p: &[V3]) -> Vec<f32> {
	let n = p.len();
	let mut w = vec![0.0f32; n];
	let mut m = 0.0f32;
	for i in 1..n - 1 {
		w[i] = wrapsub(yaw(sub3(p[i + 1], p[i])), yaw(sub3(p[i], p[i - 1])));
		if w[i].abs() > m {
			m = w[i].abs();
		}
	}
	w[n - 1] = wrapsub(yaw(sub3(p[0], p[n - 1])), yaw(sub3(p[n - 1], p[n - 2])));
	for x in &mut w {
		*x *= BANK_LIMIT / m;
	}
	w
}

/// Black fade-out of the previous screen before the first flight tick (`FUN_0021b338(10)`): alpha byte of frame
/// `i` (0..10), `0x80 - (k * 0x80) / (k + 1)` with k = 9 - i.
pub fn flight_fade_alpha(i: u32) -> u32 {
	let k = 9 - i;
	0x80 - (k * 0x80) / (k + 1)
}

/// Seconds the retail flight waits (black, no world tick) for the ship voice before its first tick: 10 fade frames
/// plus `bytes / 16 * 28 / 44100` (VAG sizes 139472 / 139472 / 144512, `0x138f40` entries 69 / 69 / 71 sectors).
pub fn flight_voice_seconds(ship: usize) -> f32 {
	let bytes = [139_472.0f32, 139_472.0, 144_512.0][ship.min(2)];
	bytes / 16.0 * 28.0 / 44100.0
}

/// A camera cuboid's pose for the glide: position, euler y and z.
#[derive(Clone, Copy, Debug)]
pub struct CamPose {
	pub pos: V3,
	pub y: f32,
	pub z: f32,
}

#[derive(Clone, Debug)]
pub struct Flight {
	n: usize,
	pts: Vec<V3>,
	w: Vec<f32>,
	cams: Option<(CamPose, CamPose)>,
	ship_type: usize,
	/// Ticks done; the next tick has this value as retail's `c`.
	pub c: u32,
	p0: V3,
	y0: f32,
	z0: f32,
	ey: f32,
	ez: f32,
	len: f32,
	u: f32,
	vel: f32,
	pub fade: f32,
	smooth: (f32, f32),
	/// Ship pose written each tick: position and the four rotation words (+0x40 bank, +0x44 pitch, +0x48 yaw, +0x4c roll).
	pub ship_pos: V3,
	pub ship_rot: [f32; 4],
	/// `ship_rot` as it was when the tick began: the ship basis the passenger seats use (SCN-4.3c / 4.3d, retail rebuilds
	/// the basis at the head of the tick, before the path sample).
	pub rot_prev: [f32; 4],
	/// Ship animation blend (+0x54).
	pub ship_blend: f32,
	/// The ribbon history of the flight (rings 0x13e0f0 / 0x13e2f0): the two seat-edge points per phase-2 step, and
	/// whether this tick registered the ribbon draw.
	pub ribbon: [(V3, V3); 32],
	pub ribbon_live: bool,
	/// The scripted camera pose.
	pub cam_pos: V3,
	pub cam_rot: V3,
	pub head: usize,
	pub count: usize,
	/// The flight is over (retail `uGpffff8950 = 1`) and the ship draw is skipped from now on.
	pub exit: bool,
}

impl Flight {
	/// The one-time setup of tick c == 0: `ship_pos`/`ship_rot` are the ship's pose at the start of substate 3 (rot as
	/// +0x40..+0x48), `path` is the level path (None = path id < 0), `cams` the two camera cuboids (None = camera untouched).
	pub fn new(ship_type: usize, ship_pos: V3, ship_rot: V3, path: Option<&[V3]>, cams: Option<(CamPose, CamPose)>) -> Flight {
		let mut f = Flight {
			n: 0,
			pts: Vec::new(),
			w: Vec::new(),
			cams,
			ship_type,
			c: 0,
			p0: ship_pos,
			y0: ship_rot[1],
			z0: ship_rot[2],
			ey: 0.0,
			ez: 0.0,
			len: 1.0,
			u: 0.0,
			vel: 0.0,
			fade: 1.0,
			smooth: (0.0, 0.0),
			ship_pos,
			ship_rot: [ship_rot[0], ship_rot[1], ship_rot[2], 0.0],
			rot_prev: [ship_rot[0], ship_rot[1], ship_rot[2], 0.0],
			ship_blend: 0.0,
			ribbon: [([0.0; 3], [0.0; 3]); 32],
			ribbon_live: false,
			cam_pos: [0.0; 3],
			cam_rot: [0.0; 3],
			head: 0,
			count: 0,
			exit: false,
		};
		if let Some(p) = path.filter(|p| p.len() >= 3) {
			f.n = p.len();
			f.pts = p.to_vec();
			f.w = path_banks(p);
			let d = sub3(p[1], p[0]);
			f.ey = fast_atan2(horizontal(d), p[0][2] - p[1][2]);
			f.ez = yaw(d);
			f.len = len3(d);
		}
		f
	}

	fn has_path(&self) -> bool {
		self.n != 0
	}

	/// The path sampled at `u` (FUN_002778d8 mode 1): position and (0, pitch, yaw, roll).
	pub fn sample(&self, u: f32) -> (V3, [f32; 4]) {
		let n = self.n;
		let i = u as usize;
		let f = u - i as f32;
		let (j, k) = ((i + 1) % n, (i + 2) % n);
		let (a, b, c) = (self.pts[i % n], self.pts[j], self.pts[k]);
		let pos = lerp3(f, a, b);
		let (d0, d1) = (sub3(b, a), sub3(c, b));
		let (yaw0, pit0) = (yaw(d0), fast_atan2(horizontal(d0), d0[2]));
		let (yaw1, pit1) = (yaw(d1), fast_atan2(horizontal(d1), d1[2]));
		let (roll0, roll1) = (self.w[i % n], self.w[j]);
		let rot = [0.0, -wrap(pit0 + wrapsub(pit1, pit0) * f), wrap(yaw0 + wrapsub(yaw1, yaw0) * f), -wrap(roll0 + wrapsub(roll1, roll0) * f)];
		(pos, rot)
	}

	/// One flight tick. `pad_held`: Triangle or Cross held (not an edge).
	pub fn tick(&mut self, pad_held: bool) {
		self.rot_prev = self.ship_rot;
		self.ribbon_live = false;
		let c = self.c;
		self.c += 1;
		// (c) the camera glides between the two cuboids.
		if let Some((a, b)) = self.cams {
			approach(1.0, PAL_DT2 * 0.666, PAL_DT2 * 0.666, PAL_DT * 0.5, &mut self.smooth.0, &mut self.smooth.1);
			let s = self.smooth.0;
			self.cam_pos = lerp3(s, a.pos, b.pos);
			self.cam_rot = [0.0, wrap(a.y + wrapsub(b.y, a.y) * s), wrap(a.z + wrapsub(b.z, a.z) * s)];
		}
		if !self.has_path() {
			return;
		}
		if c < 125 {
			// phase 1: lift off the pad toward the first path point.
			let s = if c >= 100 { 1.0 } else { (1.0 - crate::camera::fast_cos(c as f32 * std::f32::consts::PI / 100.0)) * 0.5 };
			self.ship_blend = s;
			self.ship_pos = lerp3(s, self.p0, self.pts[0]);
			self.ship_rot[0] = 0.0;
			self.ship_rot[1] = wrap(self.y0 + wrapsub(self.ey, self.y0) * s);
			self.ship_rot[2] = wrap(self.z0 + wrapsub(self.ez, self.z0) * s);
			if self.fade > 0.0 {
				self.fade -= 0.125;
				if self.fade < 0.0 {
					self.fade = 0.0;
				}
			}
		} else {
			// phase 2: along the path.
			self.vel = (self.vel + 0.8).min(100.0);
			if pad_held && self.fade < 0.0625 {
				self.fade = 0.0625;
			}
			if (self.n as f32 - 6.0) < self.u && self.fade < 0.0625 {
				self.fade = 0.0625;
			}
			self.u += (self.vel * PAL_DT) / self.len;
			if (self.n as f32 - 1.0) < self.u || self.fade >= 1.0 {
				self.exit = true;
			} else {
				let (pos, rot) = self.sample(self.u);
				self.ship_pos = pos;
				self.ship_rot = [rot[3], rot[1], rot[2], rot[3]];
				self.head = (self.head + 1) & 0x1f;
				self.count = (self.count + 1).min(0x20);
				let edge = RIBBON_EDGE[self.ship_type.min(2)];
				self.ribbon[self.head] = (seat_pos(pos, self.rot_prev, edge[0]), seat_pos(pos, self.rot_prev, edge[1]));
				self.ribbon_live = true;
				if self.fade > 0.0 {
					self.fade += 0.0625;
					if self.fade > 1.0 {
						self.fade = 1.0;
					}
				}
			}
		}
	}

	/// The ship type 0..=2 the flight was created with.
	pub fn ship_type(&self) -> usize {
		self.ship_type
	}


}

