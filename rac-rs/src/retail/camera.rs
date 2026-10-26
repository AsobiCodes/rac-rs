//! The render camera: where the level overlay keeps it in RAM (`camera_globals`, `read_cam`), the occlusion mask
//! published with it, and the camera the viewer asks for between two ticks (`Retail::camera`, `view_lag`).

use super::*;

/// Retail's occlusion mask of the field the render camera was read from (REN-CULL-OCC); the viewer's tie / fragment / moby tests use it
/// instead of rebuilding one from the level grid and rac-rs's own camera.
pub static OCC_MASK: Mutex<Option<[u8; 128]>> = Mutex::new(None);

/// Each level's shift of the overlay globals from Novalis's addresses (`camera_globals`).
pub(super) const LEVEL_SHIFT: [i32; 19] = [-0x400, 0, 0x180, -0x380, -0x300, 0, 0x300, -0x400, 0x380, -0x300, 0, 0x580, 0, -0x180, 0x300, 0x180, -0x80, 0x480, 0x580];

/// Render camera of the level's overlay: position, basis rows (row 0 forward, row 2 up) and the tan half-FOV scalar.
/// Every overlay keeps the three at Novalis's addresses plus one shift per level (each level's code windows that
/// reference them, masked of immediates, paired with Novalis's: about 1100 / 190 / 60 votes, no other candidate).
pub(super) fn camera_globals(level: u32) -> Option<(u32, u32, u32)> {
	let d = *LEVEL_SHIFT.get(level as usize)?;
	let at = |a: u32| va(a.wrapping_add_signed(d));
	Some((at(0x1671c0), at(0x1673d0), at(0x16cef0)))
}

pub(super) fn read_cam(m: &ee_oracle::Machine, level: u32) -> Option<Cam> {
	let (eye, basis, tan) = camera_globals(level)?;
	let f = m.rdv3(basis);
	Some((m.rdv3(eye), f[1].atan2(f[0]), f[2].clamp(-1.0, 1.0).asin(), m.rdf(tan)))
}

/// The render camera as the OmniLoader host (`omni/draw_w2s.rs`) needs it, read from a RAM image: eye, forward (basis row 0), up (row 2, or
/// (0, 0, 1) when it is not a unit vector) and the tan half-FOV scalar. `None` when the level has no camera globals or the values are not finite.
pub(crate) fn view_in_ram(ram: &[u8], level: u32) -> Option<([f32; 3], [f32; 3], [f32; 3], f32)> {
	let (eye, basis, tan) = camera_globals(level)?;
	let f32_at = |a: u32| {
		let i = (a & 0x01ff_ffff) as usize;
		ram.get(i..i + 4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
	};
	let v3 = |a: u32| Some([f32_at(a)?, f32_at(a + 4)?, f32_at(a + 8)?]);
	let (e, f, u, t) = (v3(eye)?, v3(basis)?, v3(basis + 0x20)?, f32_at(tan)?);
	let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
	if !(e.iter().chain(&f).chain(&u).all(|x| x.is_finite()) && t.is_finite() && t > 0.0 && (len(f) - 1.0).abs() < 0.05) {
		return None;
	}
	let up = if (len(u) - 1.0).abs() < 0.05 { u } else { [0.0, 0.0, 1.0] };
	Some((e, f, up, t))
}

/// The render camera's up vector: row 2 of the published camera basis (0x1673d0 + 0x20 at Novalis, `camera_globals`) [C].
/// Retail's view matrix keeps the roll there (the follow camera aligns it to the smoothed hero up 0x167230, camera.md), so on
/// a magnetic wall the world turns around an upright Ratchet. (0, 0, 1) when the row is not a unit vector (level load).
pub(super) fn read_cam_up(m: &ee_oracle::Machine, level: u32) -> [f32; 3] {
	let Some((_, basis, _)) = camera_globals(level) else { return [0.0, 0.0, 1.0] };
	let (f, u) = (m.rdv3(basis), m.rdv3(basis + 0x20));
	let l = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
	// The row must be unit length and not parallel to forward (roll about the forward axis is all it adds).
	let d = (f[0] * u[0] + f[1] * u[1] + f[2] * u[2]).abs();
	if !(l.is_finite() && (l - 1.0).abs() < 0.05 && d < 0.5) {
		return [0.0, 0.0, 1.0];
	}
	let u = [u[0] / l, u[1] / l, u[2] / l];
	// A pitched camera without roll has row 2 = world z made perpendicular to forward: that is the same view as plain (0, 0, 1),
	// so return exactly (0, 0, 1) (flat ground renders bit-identical to before; only a real roll changes the picture).
	let fl = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
	if fl > 0.5 {
		let fz = f[2] / fl;
		let z0 = [-fz * f[0] / fl, -fz * f[1] / fl, 1.0 - fz * f[2] / fl];
		let n = (z0[0] * z0[0] + z0[1] * z0[1] + z0[2] * z0[2]).sqrt();
		if n > 1e-3 && (0..3).all(|i| (u[i] - z0[i] / n).abs() < 1e-3) {
			return [0.0, 0.0, 1.0];
		}
	}
	u
}

/// [PC] Caps of `Retail::view_lag`. A fast mouse flick turns the follow camera by 0.4-1.2 rad in one field and swings the eye
/// 2-5 m around the hero, and frame smoothing blends such a field (viewer/interp.rs `cam_cut`), so a smoothed frame trails the
/// newest view by up to that turn. The old caps (0.35 rad, 4 m, from when such a field was a cut drawn with the newest camera)
/// left mobys retail culled at its side plane missing at the trailing screen edge on those frames (lane yflk, 2026-10-24).
const LAG_ANG_MAX: f32 = 1.25;
const LAG_MOVE_MAX: f32 = 6.0;

impl Retail {
	/// The render camera's up vector between the last two ticks (roll; `camera`'s companion).
	pub fn camera_up(&self, alpha: f32) -> [f32; 3] {
		let (a, b) = (self.cam_up_prev, self.cam_up);
		let v = [a[0] + (b[0] - a[0]) * alpha, a[1] + (b[1] - a[1]) * alpha, a[2] + (b[2] - a[2]) * alpha];
		let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
		if l > 1e-4 { [v[0] / l, v[1] / l, v[2] / l] } else { [0.0, 0.0, 1.0] }
	}

	/// [PC] How far a smoothed frame's camera (`camera(alpha)`, between the last two ticks) can trail the newest tick's view that
	/// retail's moby verdict uses: the turn and zoom between the ticks plus 0.03 rad, the camera's move plus 0.25 m (the mobys'
	/// own smoothed motion); capped at `LAG_ANG_MAX` and `LAG_MOVE_MAX` so a camera cut does not draw every side-culled moby.
	pub fn view_lag(&self) -> ViewLag {
		let (a, b) = (self.cam_prev, self.cam);
		let mut dy = (b.1 - a.1).abs() % std::f32::consts::TAU;
		if dy > std::f32::consts::PI {
			dy = std::f32::consts::TAU - dy;
		}
		let zoom = (a.3.atan() - b.3.atan()).max(0.0);
		let d = [b.0[0] - a.0[0], b.0[1] - a.0[1], b.0[2] - a.0[2]];
		let mv = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
		ViewLag { ang: (dy + (b.2 - a.2).abs() + zoom + 0.03).min(LAG_ANG_MAX), grow: (mv + 0.25).min(LAG_MOVE_MAX) * 1024.0 }
	}

	/// The camera between the last two ticks (`alpha` 0 = previous, 1 = newest).
	pub fn camera(&self, alpha: f32) -> Cam {
		let (a, b) = (self.cam_prev, self.cam);
		let l = |x: f32, y: f32| x + (y - x) * alpha;
		let mut dy = b.1 - a.1;
		if dy > std::f32::consts::PI {
			dy -= std::f32::consts::TAU;
		} else if dy < -std::f32::consts::PI {
			dy += std::f32::consts::TAU;
		}
		([l(a.0[0], b.0[0]), l(a.0[1], b.0[1]), l(a.0[2], b.0[2])], a.1 + dy * alpha, l(a.2, b.2), l(a.3, b.3))
	}
}
