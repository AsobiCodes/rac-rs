//! Gameplay camera: the third-person follow rig and the level's fixed/scripted camera volumes,
//! decoded from the overlay (decomp/notes/camera.md). Independent of the viewer.
//!
//! Coordinate convention: world space, Z up, yaw 0 = +X, yaw increasing counter-clockwise seen from
//! above (the game's convention: forward = (cos yaw, sin yaw, 0)). Distances are game units.
//!
//! Timing: the game steps its camera once per frame with no dt. On PAL that is 50 Hz and the spring
//! constants are NOT rescaled, so the rig is simulated in fixed 0.02 s steps (`TICK`) and the render
//! view is interpolated between the last two steps.

use crate::bytes::{f32_at, i32_at, span, u16_at, u32_at, R};
use crate::camgrid::CamGrid;
use crate::gameplay::{self, Cuboid};
use crate::level::Level;
use crate::gameplay::PAL_DT;
use crate::physics::{add, cross, dot, length, normalize, scale, sub, vadd1, vmul, Scene, World, V3};
use crate::game::kerwan::cam14::lin_spring;

/// One game frame on PAL.
pub const TICK: f32 = 0.02;
/// Default boom length (FUN_002e6708: +0x15c = 0x40947ae1).
pub const DISTANCE: f32 = 4.64;
/// Default pivot height above the smoothed target (+0x160).
pub const HEIGHT: f32 = 2.0;
/// Default look-at height above the smoothed target (+0xf0).
pub const LOOK_HEIGHT: f32 = 1.5;
/// The boom is never flatter than this when following (FUN_002e90c8), 15 degrees.
pub const MIN_ELEVATION: f32 = 0.261_799_4;
/// The retail horizontal projection scalar (tan of the half horizontal FOV, decomp/notes/camera.md
/// "REN-7b port spec"): follow, free-look and pause cameras always use 0.63 (0x3f2147ae).
pub const PROJ_SCALAR: f32 = 0.63;


/// Horizontal stick rate, radians per tick (gp+0xb170 table entry 1, 1.3 degrees; the default
/// camera-speed setting is not known) [?].
pub const ROTATE_RATE: f32 = 0.022_689_3;

/// SHR-CAMDRVa1 camera_preupdate 0x1e62d8: the smoothed-up spring gain (0x3c75c28f) and damping (0x3e4ccccd), the
/// opposed-up test (dot below 0xbf7ae148 = -0.98) and the nudge added to each target axis then (0x3e4ccccd).
const UP_K: f32 = f32::from_bits(0x3c75_c28f);
const UP_D: f32 = f32::from_bits(0x3e4c_cccd);
const UP_OPPOSED: f32 = f32::from_bits(0xbf7a_e148);
const UP_NUDGE: f32 = f32::from_bits(0x3e4c_cccd);
/// SHR-CAMDRVa1: the record slots the driver scans (0x1671d0, stride 0xa0).
const REC_SLOTS: usize = 48;
const SIZE_K: f32 = 0.003;
const LOOK_K: f32 = 0.005;
/// Pull-in recovery timer: fead0(2000) = 1667 PAL ticks.
const RECOVER_TICKS: f32 = 1667.0;
const M_ZOOM: (f32, f32) = (1.5, 14.0);
const M_PITCH: (f32, f32) = (-0.5, 1.3);
const M_START_PITCH: f32 = 0.35;

#[derive(Clone, Copy, Default)]
pub struct CamInput {
	/// Port extra, not in the game: the "Modern" camera option, a free orbit (`tick_modern`).
	/// Starts behind the hero, then only the player turns, tilts and zooms it; no fixed camera
	/// volumes; wall collision still pulls it in. Cutscenes are drawn outside the rig.
	pub modern: bool,
	/// Modern only: mouse yaw / pitch change in radians for this update (signs already applied).
	pub look: [f32; 2],
	/// Modern only: zoom change in world units (+ = farther).
	pub zoom: f32,
}







#[derive(Clone, Copy, Debug)]
pub struct View {
	pub pos: V3,
	pub target: V3,
	/// Retail horizontal projection scalar (REN-7b); the viewer derives the vertical tan as
	/// `proj_scalar * PROJ_Y_RATIO`.
	pub proj_scalar: f32,
}

/// A camera volume record (gameplay +0x08, 0x20 bytes: class, pos, pad, yaw @0x18, pvar index @0x1c).
/// Classes 23 and 17 are the two on Veldin; they are not mobys.
#[derive(Clone, Debug)]
pub struct FixedCam {
	/// The trigger cuboid (pvar +0x0c). Sphere (+0x08) and cylinder (+0x10) volumes are -1 on Veldin.
	pub cuboid: Option<usize>,
	/// Keep the auto-follow swing behind the hero while active (class 23: pvar +0x24, class 17: +0x34).
	pub auto_follow: bool,
	/// Class 23 only (pvar +0x34): only engage while the stick is idle.
	pub requires_still: bool,
	/// Class 23 only (pvar +0x2c): pitch of the view from the fixed position, radians (negative looks up).
	pub pitch: f32,
	/// Class 17: pvar +0x54 / +0x56 lock the vertical / horizontal stick.
	pub lock_vertical: bool,
	pub lock_horizontal: bool,
	/// Class 17 pvar +0x36 != 0: the volume is skipped while the camera faces 80 degrees or more away from its yaw (CAM-7.3c).
	pub heading_gate: bool,
	/// Class 17 pvar +0x50 in radians: the idle-stick pitch bias.
	pub pitch_bias: f32,
	/// Class 17 overrides (pvar +0x24 distance, +0x28 height, +0x30 look height); 0 = no override.
	pub dist: f32,
	pub height: f32,
	pub look: f32,
	/// Class 3 rail camera (CAM-7.3a, sub-mode 3 with pvar +0x24 < 0 only): path index (pvar +0x20), camera lead
	/// along the path (pvar +0x38, negative = behind the hero) and look lead (pvar +0x00). Selected by the hero
	/// camera mode, never by a volume.
	pub path: Option<usize>,
	pub lead: f32,
	pub look_lead: f32,
}

fn pvar(g: &[u8], index: usize) -> R<&[u8]> {
	let table = u32_at(g, 0x54)? as usize;
	let data = u32_at(g, 0x58)? as usize;
	let off = u32_at(g, table + index * 8)? as usize;
	let size = u32_at(g, table + index * 8 + 4)? as usize;
	span(g, data + off, size)
}

/// Reads the level's class 23 and class 17 camera records and their pvars, and the class 3 rail records of
/// sub-mode 3 with no path pair (CAM-7.3a; sub-modes 0/2 and the pvar +0x24 >= 0 branch have no Novalis record).
pub fn fixed_cameras(level: &Level) -> R<Vec<FixedCam>> {
	let g = &level.gameplay;
	let start = u32_at(g, 0x08)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	let mut out = Vec::new();
	for r in span(g, start + 16, count * 0x20)?.chunks_exact(0x20) {
		let class = u32_at(r, 0)?;
		if class != 23 && class != 17 && class != 3 {
			continue;
		}
		let pv = pvar(g, u32_at(r, 0x1c)? as usize)?;
		if class == 3 && (i32_at(pv, 0x30)? != 3 || i32_at(pv, 0x24)? >= 0) {
			continue;
		}
		let cuboid = i32_at(pv, 0x0c)?;
		let flag = |o: usize| -> R<bool> { Ok(u16_at(pv, o)? != 0) };
		let mut cam = FixedCam {
			cuboid: (cuboid >= 0).then_some(cuboid as usize),
			auto_follow: true,
			requires_still: false,
			pitch: 0.0,
			lock_vertical: false,
			lock_horizontal: false,
			heading_gate: false,
			pitch_bias: 0.0,
			dist: 0.0,
			height: 0.0,
			look: 0.0,
			path: None,
			lead: 0.0,
			look_lead: 0.0,
		};
		if class == 3 {
			cam.cuboid = None;
			cam.path = Some(i32_at(pv, 0x20)?.max(0) as usize);
			cam.lead = f32_at(pv, 0x38)?;
			cam.look_lead = f32_at(pv, 0x00)?;
		} else if class == 23 {
			cam.auto_follow = flag(0x24)?;
			cam.requires_still = flag(0x34)?;
			cam.pitch = f32_at(pv, 0x2c)?;
		} else {
			cam.auto_follow = flag(0x34)?;
			cam.lock_vertical = flag(0x54)?;
			cam.lock_horizontal = flag(0x56)?;
			cam.heading_gate = flag(0x36)?;
			cam.pitch_bias = f32_at(pv, 0x50)? * 0.017453292;
			cam.dist = f32_at(pv, 0x24)?;
			cam.height = f32_at(pv, 0x28)?;
			cam.look = f32_at(pv, 0x30)?;
		}
		out.push(cam);
	}
	Ok(out)
}

/// One camera record as the shared selector sees it (FUN_0020d520 / FUN_0020d310, decomp/notes/camera.md CAM-4a): class word,
/// pvar priority (+0x1c), switch metadata (+0x1d) and selection mode (+0x1f), the trigger cuboid (pvar +0x0c), the class-3 path
/// index (pvar +0x24, -1 for other classes), and the runtime flags the registration initialises (armed 0, request 0, rate -1.0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamRec {
	pub class: i32,
	pub prio: u8,
	pub meta: u8,
	pub mode: u8,
	pub cuboid: Option<usize>,
	pub path: i32,
	pub armed: u8,
	pub request: i16,
	pub rate: f32,
	/// KER-63a: the runtime record's position (+0x00..+0x0c) and euler rotation (+0x10..+0x1c), and the class-1 look
	/// height (pvar +0x18; raw word for other classes).
	pub pos: V3,
	pub rot: V3,
	pub look: f32,
	/// KER-64a: pvar +0x20 / +0x24, the class-14 boom length and height (raw words for other classes).
	pub boom_len: f32,
	pub boom_h: f32,
}

impl Default for CamRec {
	/// The registration state of a record with no class data: no cuboid, no path, armed 0, request 0, rate -1.0.
	fn default() -> CamRec {
		CamRec { class: 0, prio: 0, meta: 0, mode: 0, cuboid: None, path: -1, armed: 0, request: 0, rate: -1.0, pos: [0.0; 3], rot: [0.0; 3], look: 0.0, boom_len: 0.0, boom_h: 0.0 }
	}
}

/// KER-63a: the camera switch's follow-blend registers a class-1 accept resets (Kerwan 0x166f73 flag, 0x166f88 / 0x166f94
/// rates). Only the CAM-4b driver (ledger SHR-CAMDRV, not ported) reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CamBlendRegs {
	pub flag: u8,
	pub rate_a: f32,
	pub rate_b: f32,
	/// KER-64a: the switch blend countdown (Kerwan 0x166ff4); class-14 setup stores a raw 120.
	pub count: i32,
}

/// Every camera record of the level in index order (the same table walk as `fixed_cameras`, but all classes are kept).
pub fn camera_records(level: &Level) -> R<Vec<CamRec>> {
	let g = &level.gameplay;
	let start = u32_at(g, 0x08)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	span(g, start + 16, count * 0x20)?
		.chunks_exact(0x20)
		.map(|r| {
			let class = i32_at(r, 0)?;
			let pv = pvar(g, u32_at(r, 0x1c)? as usize)?;
			let cuboid = i32_at(pv, 0x0c)?;
			Ok(CamRec {
				class,
				prio: *pv.get(0x1c).ok_or("short pvar")?,
				meta: *pv.get(0x1d).ok_or("short pvar")?,
				mode: *pv.get(0x1f).ok_or("short pvar")?,
				cuboid: (cuboid >= 0).then_some(cuboid as usize),
				path: if class == 3 { i32_at(pv, 0x24)? } else { -1 },
				armed: 0,
				request: 0,
				rate: -1.0,
				pos: [f32_at(r, 4)?, f32_at(r, 8)?, f32_at(r, 12)?],
				rot: [f32_at(r, 0x10)?, f32_at(r, 0x14)?, f32_at(r, 0x18)?],
				look: f32_at(pv, 0x18)?,
				// Pvars of other classes can end at 0x20 (Novalis class 23 / 17); those records never read the boom.
				boom_len: f32_at(pv, 0x20).unwrap_or(0.0),
				boom_h: f32_at(pv, 0x24).unwrap_or(0.0),
			})
		})
		.collect()
}






fn wrap(a: f32) -> f32 {
	let tau = std::f32::consts::TAU;
	let a = a % tau;
	if a > std::f32::consts::PI {
		a - tau
	} else if a < -std::f32::consts::PI {
		a + tau
	} else {
		a
	}
}


/// Boom as (length, elevation, azimuth) for a vector.
fn polar(v: V3) -> (f32, f32, f32) {
	let l = length(v).max(1e-6);
	((l), (v[2] / l).clamp(-1.0, 1.0).asin(), v[1].atan2(v[0]))
}

fn from_polar(len: f32, elev: f32, az: f32) -> V3 {
	[len * elev.cos() * az.cos(), len * elev.cos() * az.sin(), len * elev.sin()]
}

pub struct CameraRig {
	/// CAM-6: the sticky surface skip of the line mask (DAT_001672d0), zeroed on a rebuild.
	line_skip: u32,
	/// CAM-6: Novalis (level id 1) retail never raises the lava / mud ground flags.
	novalis: bool,
	/// Camera collision grid (gameplay +0x84, decomp/notes/camera.md); empty when not loaded.
	grid: CamGrid,
	started: bool,
	// Smoothed target (+0xa0) and its spring velocities.
	s: V3,
	sv: V3,
	// Look height spring.
	look_z: f32,
	look_zv: f32,
	// Sprung rig parameters: boom length, pivot height, look offset.
	dist: f32,
	dist_v: f32,
	height: f32,
	height_v: f32,
	look: f32,
	look_v: f32,
	// Raw boom (pivot to camera), smoothed boom and its spring velocities.
	dir: V3,
	len: f32,
	elev: f32,
	az: f32,
	len_v: f32,
	elev_v: f32,
	az_v: f32,
	pull_in: f32,
	/// Recovery timer (+0x204): held at 1667 while blocked, then counts down.
	pull_t: f32,
	/// The stick's elevation offset applied so far (moves toward stickY * 40 deg).
	tilt: f32,
	pos: V3,
	prev: (V3, V3),
	cur: (V3, V3),
	/// Index into `fixed` of the volume currently overriding the rig.
	active: Option<usize>,
	/// Azimuth the recenter swing is heading for.
	centering: Option<f32>,
	/// Modern orbit (port extra): yaw, pitch and zoom set by the player, boom length after walls,
	/// and whether the orbit has been placed since the last reset or mode change.
	m_yaw: f32,
	m_pitch: f32,
	m_dist: f32,
	m_len: f32,
	m_on: bool,
	/// A fixed cut (Novalis camera trigger, decomp/notes/camera.md): the view is held as given.
	over: Option<View>,
	/// Consecutive ticks the pivot -> boom line was blocked (pvar +0x21c).
	blocked_ticks: u32,
	/// Follow-rig sub-mode (+0x104; CAM-7.4): the hold countdown (+0x114) and its inverse (+0x10c), the held target z
	/// (+0x70), the entry snapshot z (+0x60), the re-seed flag (+0x117) and the last output z (+0x50).
	gsub: u8,
	gout: f32,
	center_left: u32,
	k_dist: f32,
	k_height: f32,
	k_look: f32,
	bprev: (V3, V3),
	bcur: (V3, V3),
	blend_out: bool,
	over_up: V3,
	bup: V3,
	/// KER-63a: the follow-blend registers a class-1 accept resets (read only by the CAM-4b driver, SHR-CAMDRV).
	pub blend_regs: CamBlendRegs,
	/// CAM-7.3a: the level's path polygons the class-3 rail camera follows (gameplay polygons, as the currents).
	rail_paths: Vec<gameplay::Polygon>,
	/// CAM-7.2b aim state (decomp/notes/camera.md "CAM-7.2b port spec"): look/height springs (+0x24/+0x28 with
	/// velocities +0x2c/+0x30), look blend counter and flag (+0x118/+0x116), pitch bias (+0x34/+0x38), boom retreat
	/// latch and length (+0x224/+0x228), the hero position of the previous tick (0x167270), the last forward
	/// (camera +0x00), and the orientation blend counter / old forward (rig +0x20, camera +0x40).
	look_s: f32,
	look_sv: f32,
	height_s: f32,
	height_sv: f32,
	aim_w: i32,
	aim_flag: bool,
	retreat: bool,
	retreat_len: f32,
	prev_hero: V3,
	fwd: V3,
	orient_blend: i32,
	orient_old: V3,
	/// SHR-CAMDRVa1 (decomp/notes/camera.md "SHR-CAMDRVa port spec"): the live camera record driver. `recs` the level's
	/// registered records (0x1671d0, slot i active while i < len), `rec_cur` / `rec_prev` the current and previous
	/// record (0x166e80 / 0x166e84), `rec_ticks` the frames since the last switch (0x167098), `cam1` / `cam14` the class
	/// 1 / 14 camera instances (one per rig: the class-1 spring velocity outlives a session), `planet` the level index
	/// the hand-off rate reads, `rec_view` the pose the driver last published for a class 1 / 14 record (a view the
	/// driver owns, unlike a Game hold, so `tick_step` keeps running the driver under it).
	recs: Vec<CamRec>,
	rec_cur: usize,
	rec_prev: Option<usize>,
	rec_ticks: u32,
	planet: i32,
	rec_view: Option<(V3, V3)>,
	/// SHR-CAMDRVa1 camera_preupdate 0x1e62d8: the smoothed world up (0x166eb0) and its spring velocity (0x166ee0), and
	/// the unsmoothed target up (0x166ec0; its reader is the class-4 predicate, SHR-CAMDRVa2).
	up_s: V3,
	up_v: V3,
	up_t: V3,
}




/// FUN_00210098: `v` rescaled to length `l` with the VU0 rsqrt (one truncation); a zero vector stays zero.
fn set_length(v: V3, l: f32) -> V3 {
	let q = crate::physics::vrsqrt(l, vadd1(vadd1(vmul(v[0], v[0]), vmul(v[1], v[1])), vmul(v[2], v[2])));
	[vmul(v[0], q), vmul(v[1], q), vmul(v[2], q)]
}

















/// CAM-7.2b aim/orient blend length, scale(90).
const AIM_BLEND: i32 = 75;

/// FUN_00221258 (Aridia FUN_002103b0): the retail ASIN approximation (gp 0x1606e0..0x1606ec, evaluated in this order).
pub(crate) fn retail_asin(x: f32) -> f32 {
	let (s, x) = if x < 0.0 { (-1.0, x.abs()) } else { (1.0, x) };
	let c = [f32::from_bits(0x3fc90da4), f32::from_bits(0xbe593484), f32::from_bits(0x3d981627), f32::from_bits(0xbc996e30)];
	let poly = x * x * x * c[3] + x * x * c[2] + x * c[1] + c[0] + 0.0;
	// EE sqrt.s takes |x|: a dot product rounded just above 1.0 stays finite (ARI-47d orientation, found 2026-10-07).
	s * (1.5707964 - poly * (1.0 - x).abs().sqrt())
}





impl CameraRig {
	/// The level's camera collision grid and its shape tables (ARI-22 / BRIDGE-59 reads the cylinders).
	pub fn grid(&self) -> &CamGrid {
		&self.grid
	}



	pub fn new(level: &Level) -> R<CameraRig> {
		let mut rig = CameraRig::with_cameras(fixed_cameras(level)?, gameplay::cuboids(level)?, camera_records(level)?);
		rig.grid = CamGrid::parse(&level.gameplay)?;
		rig.novalis = level.id == 1;
		rig.planet = level.id as i32;
		rig.rail_paths = gameplay::polygons(level)?;
		Ok(rig)
	}

	/// `recs` the level's camera records (`camera_records`), registered by `register_records`.
	pub fn with_cameras(fixed: Vec<FixedCam>, cuboids: Vec<Cuboid>, recs: Vec<CamRec>) -> CameraRig {
		let mut rig = CameraRig {
			line_skip: 0,
			novalis: false,
			grid: CamGrid::default(),
			started: false,
			s: [0.0; 3],
			sv: [0.0; 3],
			look_z: 0.0,
			look_zv: 0.0,
			dist: DISTANCE,
			dist_v: 0.0,
			height: HEIGHT,
			height_v: 0.0,
			look: LOOK_HEIGHT,
			look_v: 0.0,
			dir: [0.0; 3],
			len: DISTANCE,
			elev: MIN_ELEVATION,
			az: 0.0,
			len_v: 0.0,
			elev_v: 0.0,
			az_v: 0.0,
			pull_in: 0.0,
			pull_t: RECOVER_TICKS,
			pos: [0.0; 3],
			prev: ([0.0; 3], [0.0; 3]),
			cur: ([0.0; 3], [0.0; 3]),
			active: None,
			tilt: 0.0,
			centering: None,
			m_yaw: 0.0,
			m_pitch: M_START_PITCH,
			m_dist: DISTANCE + 1.0,
			m_len: DISTANCE + 1.0,
			m_on: false,
			over: None,
			blocked_ticks: 0,
			gsub: 0,
			gout: 0.0,
			center_left: 0,
			k_dist: SIZE_K,
			k_height: SIZE_K,
			k_look: LOOK_K,
			bprev: ([0.0; 3], [0.0; 3]),
			bcur: ([0.0; 3], [0.0; 3]),
			blend_out: false,
			over_up: [0.0, 0.0, 1.0],
			bup: [0.0, 0.0, 1.0],
			blend_regs: CamBlendRegs::default(),
			rail_paths: Vec::new(),
			look_s: LOOK_HEIGHT,
			look_sv: 0.0,
			height_s: HEIGHT,
			height_sv: 0.0,
			aim_w: 0,
			aim_flag: false,
			retreat: false,
			retreat_len: DISTANCE,
			prev_hero: [0.0; 3],
			fwd: [0.0; 3],
			orient_blend: 0,
			orient_old: [0.0; 3],
			recs: Vec::new(),
			rec_cur: 0,
			rec_prev: None,
			rec_ticks: 0,
			planet: 0,
			rec_view: None,
			up_s: [0.0, 0.0, 1.0],
			up_v: [0.0; 3],
			up_t: [0.0, 0.0, 1.0],
		};
		rig.register_records(recs);
		rig
	}

	/// SHR-CAMDRVa1 camera_records_init 0x1e6bc0 (level load): the records take slots 0.. (at most 48) with their
	/// runtime words cleared (+0x7d hand-off word, +0x7e request, +0x78 rate -1.0), the tick counter and previous record
	/// are cleared, the smoothed up is seeded from the hero moby's up (+0xe0: world Z on every level) with zero spring
	/// velocity, then camera_reset_default picks the start record.
	fn register_records(&mut self, mut recs: Vec<CamRec>) {
		recs.truncate(REC_SLOTS);
		for r in &mut recs {
			(r.armed, r.request, r.rate) = (0, 0, -1.0);
		}
		self.recs = recs;
		(self.rec_ticks, self.rec_prev, self.rec_cur, self.rec_view) = (0, None, 0, None);
		self.up_s = set_length([0.0, 0.0, 1.0], 1.0);
		(self.up_t, self.up_v) = (self.up_s, [0.0; 3]);
		self.reset_default(None);
	}

	/// SHR-CAMDRVa1 camera_reset_default 0x1e6ae8: the old current record's hand-off word and request are cleared and the
	/// level's first class-0 record becomes current (none: record 0, and `drive` does nothing), then camera_preupdate and
	/// the class-0 setup (the follow reseed behind `hero`). At registration there is no hero yet: the viewer's level-start
	/// `reset` is that setup. The blend mode 0x166f70 = 0 and the record +0x8e word belong to SHR-CAMDRVc.
	fn reset_default(&mut self, hero: Option<(V3, f32)>) {
		if let Some(r) = self.recs.get_mut(self.rec_cur) {
			(r.armed, r.request) = (0, 0);
		}
		self.rec_cur = self.recs.iter().position(|r| r.class == 0).unwrap_or(0);
		self.preupdate();
		if let Some((pos, yaw)) = hero {
			self.reset(pos, yaw);
		}
	}

	/// SHR-CAMDRVa1 camera_preupdate 0x1e62d8 (main step 2), the smoothed world up: the target is the reversed hero
	/// gravity (0x13f5e0, (0, 0, -1) on every level), kept unsmoothed in `up_t`; when the smoothed up points against it
	/// (dot < -0.98) each target axis is nudged by 0.2 so the spring has a direction to turn in; each axis then springs
	/// (FUN_001e4aa8, k 0.015, damping 0.2, no limit) and the result is rescaled to length 1. The separate hero-z spring
	/// and the hero-relative history of this routine have no reader outside the blend internals (SHR-CAMDRVc).
	fn preupdate(&mut self) {
		let mut d = set_length([0.0, 0.0, -1.0], -1.0);
		self.up_t = d;
		let s = self.up_s;
		let dt = gameplay::ee_add(gameplay::ee_add(gameplay::ee_mul(s[0], d[0]), gameplay::ee_mul(s[1], d[1])), gameplay::ee_mul(s[2], d[2]));
		if dt < UP_OPPOSED {
			for c in &mut d {
				*c = gameplay::ee_add(*c, UP_NUDGE);
			}
		}
		for i in 0..3 {
			lin_spring(&mut self.up_s[i], d[i], UP_K, UP_D, &mut self.up_v[i]);
		}
		self.up_s = set_length(self.up_s, 1.0);
	}





















	/// BRIDGE-56: the up of the view: the held view's (`over_up`) while held, the blend output's while a release blend
	/// runs, else Z.
	pub fn up(&self) -> V3 {
		if self.over.is_some() {
			self.over_up
		} else if self.blend_out {
			self.bup
		} else {
			[0.0, 0.0, 1.0]
		}
	}



	/// Puts the camera behind the hero with everything at rest (level start, respawn, cut).
	pub fn reset(&mut self, hero_pos: V3, hero_yaw: f32) {
		// Rig init (FUN_002e6708 / FUN_00311868) restores the size spring k values.
		(self.k_dist, self.k_height, self.k_look) = (SIZE_K, SIZE_K, LOOK_K);
		// CAM-7.2b: the orientation blends from the previous rig's forward over scale(90) ticks (OV:170795-170800).
		let d = sub(self.cur.1, self.cur.0);
		self.orient_old = if length(d) > 0.0 { normalize(d) } else { [0.0; 3] };
		self.orient_blend = AIM_BLEND;
		self.reset_behind_hero(hero_pos, hero_yaw, None);
	}

	/// The camera rebuild (decomp/notes/camera.md "25-blocked-tick rebuild"): rig parameters back to their defaults,
	/// springs and velocities zeroed, pull-in and blocked count cleared, recovery timer full, then the camera placed
	/// 4.64 behind the hero and 2.0 up. With a scene, the look point (hero + 1.5 up) -> camera line is tested and a
	/// hit pulls the camera back 0.5 from it [P: FUN_00212860 retry and the near-hit fallbacks of FUN_003111c0 not ported].
	fn reset_behind_hero(&mut self, hero_pos: V3, hero_yaw: f32, scene: Option<&Scene>) {
		self.started = true;
		self.blocked_ticks = 0;
		self.line_skip = 0;
		self.s = hero_pos;
		self.sv = [0.0; 3];
		self.look_z = hero_pos[2] + LOOK_HEIGHT;
		self.look_zv = 0.0;
		self.dist = DISTANCE;
		self.height = HEIGHT;
		self.look = LOOK_HEIGHT;
		self.dist_v = 0.0;
		self.height_v = 0.0;
		self.look_v = 0.0;
		self.az = wrap(hero_yaw + std::f32::consts::PI);
		self.len = DISTANCE;
		self.len_v = 0.0;
		self.elev_v = 0.0;
		self.az_v = 0.0;
		self.pull_in = 0.0;
		self.pull_t = RECOVER_TICKS;
		self.tilt = 0.0;
		self.active = None;
		self.centering = None;
		self.center_left = 0;
		self.gsub = 0;
		self.gout = hero_pos[2];
		let back = [-hero_yaw.cos(), -hero_yaw.sin(), 0.0];
		let mut pos = add(add(hero_pos, scale(back, DISTANCE)), [0.0, 0.0, HEIGHT]);
		if let Some(w) = scene {
			// FUN_003111c0(1): the look point -> camera line (0x94; 0x96 when the first hit is within 0.001 of the
			// look point horizontally), then the pull back / near-hit fallback, then the same-spot nudge.
			let look = add(hero_pos, [0.0, 0.0, LOOK_HEIGHT]);
			let flat = |a: V3, b: V3| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
			let mask = match w.line(look, pos, 0x94, None) {
				Some(f) if flat(f.p, look) < 0.001 => 0x96,
				_ => 0x94,
			};
			if let Some(h) = w.line(look, pos, mask, None) {
				let d = length(sub(h.p, look));
				if d == 0.0 {
					pos = hero_pos;
				} else if d >= 0.2 {
					pos = add(look, scale(sub(h.p, look), (d - 0.5) / d));
				} else {
					// Near hit: push a sphere probe out of the geometry (up to 3 times), re-extend 4.64 along the
					// push and line-test again.
					let base = add(hero_pos, [0.0, 0.0, 0.5]);
					let (mut probe, mut r, mut k) = (base, 0.5, 0);
					while k < 3 {
						match w.capsule(probe, 0.0, r, 4, None) {
							Some((o, _)) => {
								probe = o.to;
								k += 1;
							}
							None => {
								if k != 0 {
									break;
								}
								r += 0.25;
								k = 1;
							}
						}
					}
					let e = sub(probe, base);
					if length(e) == 0.0 {
						pos = hero_pos;
					} else {
						pos = add(base, scale(e, DISTANCE / length(e)));
						if let Some(h2) = w.line(base, pos, mask, None) {
							let d2 = length(sub(h2.p, probe));
							pos = if d2 == 0.0 { hero_pos } else { add(probe, scale(sub(h2.p, probe), (d2 - 0.5) / d2)) };
						}
					}
				}
			}
			if flat(pos, hero_pos) < 0.05 {
				let mut rng = crate::particles::Rng::shared();
				pos[0] += if rng.small(2) == 0 { -0.5 } else { 0.5 };
				pos[1] += if rng.small(2) == 0 { -0.5 } else { 0.5 };
				pos[2] += 0.5;
			}
		}
		self.pos = pos;
		let boom = sub(pos, add(hero_pos, [0.0, 0.0, HEIGHT]));
		let (_, elev, _) = polar(boom);
		self.dir = boom;
		self.elev = elev;
		self.cur = (self.pos, [hero_pos[0], hero_pos[1], self.look_z]);
		self.prev = self.cur;
		// CAM-7.2b aim/retreat state (FUN_00311868; FUN_0020ed80 sets 0x167270 = hero, OV:1839).
		(self.look_s, self.look_sv, self.height_s, self.height_sv) = (self.look, 0.0, self.height, 0.0);
		(self.aim_w, self.aim_flag, self.retreat, self.retreat_len) = (0, false, false, self.dist);
		self.prev_hero = hero_pos;
		let f = [hero_pos[0] - pos[0], hero_pos[1] - pos[1], 0.0];
		self.fwd = if length(f) > 0.0 { normalize(f) } else { f };
		// Modern: behind the hero again, zoom kept.
		self.m_yaw = self.az;
		self.m_pitch = M_START_PITCH;
		self.m_len = self.m_dist;
		self.m_on = false;
	}


	/// Per display frame input: first-use reset and the Modern / back-to-Classic look and zoom (PC camera modes).
	pub fn frame_input(&mut self, hero_pos: V3, hero_yaw: f32, input: &CamInput) {
		if self.over.is_some() {
			return;
		}
		if !self.started {
			self.reset(hero_pos, hero_yaw);
		}
		if input.modern {
			if !self.m_on {
				// Entering Modern (or after a reset): start behind the hero.
				self.m_on = true;
				self.m_yaw = wrap(hero_yaw + std::f32::consts::PI);
				self.m_pitch = M_START_PITCH;
				self.m_len = self.m_len.min(self.m_dist);
			}
			self.m_yaw = wrap(self.m_yaw + input.look[0]);
			self.m_pitch = (self.m_pitch + input.look[1]).clamp(M_PITCH.0, M_PITCH.1);
			self.m_dist = (self.m_dist + input.zoom).clamp(M_ZOOM.0, M_ZOOM.1);
			if input.look != [0.0; 2] {
				self.centering = None;
			}
		} else if self.m_on {
			// Back to Classic: the classic boom picks up where the orbit was.
			self.m_on = false;
			self.dir = from_polar(self.m_len, self.m_pitch.max(MIN_ELEVATION), self.m_yaw);
			self.az = self.m_yaw;
			self.elev = self.m_pitch.max(MIN_ELEVATION);
			self.len = self.m_len;
		}
	}





	/// The display view: previous -> newest camera tick lerped by `f` (0..1).
	pub fn view(&self, f: f32) -> View {
		if let Some(v) = self.over {
			return v;
		}
		let lerp = |a: V3, b: V3| add(a, scale(sub(b, a), f));
		let (pv, cu) = if self.blend_out { (self.bprev, self.bcur) } else { (self.prev, self.cur) };
		View { pos: lerp(pv.0, cu.0), target: lerp(pv.1, cu.1), proj_scalar: PROJ_SCALAR }
	}

	/// The newest camera tick without display interpolation (the `f = 1` case of `update`), for per-tick state such as
	/// the glint cull (FRAME-1).
	pub fn tick_view(&self) -> View {
		if let Some(v) = self.over {
			return v;
		}
		let cu = if self.blend_out { self.bcur } else { self.cur };
		View { pos: cu.0, target: cu.1, proj_scalar: PROJ_SCALAR }
	}


























}

/// One camera-shake record (camera.md "Shake consumer"): the amplitude and timer a writer sets, the
/// latched length and the offset the consumer derives each frame.
#[derive(Clone, Copy, Default, Debug)]
pub struct ShakeRecord {
	pub amp: f32,
	pub offset: f32,
	pub timer: i32,
}

impl ShakeRecord {
	/// A writer overwrites amplitude and timer unconditionally; the latched length is untouched.
	pub fn write(&mut self, amp: f32, timer: i32) {
		self.amp = amp;
		self.timer = timer;
	}

}

/// Native VU0 cosine of the shake curve (camera.md "NOV-5.3"): cos(x) = sin(x + pi/2) via the
/// odd sine polynomial, argument already wrapped to [-pi, pi) by the caller. Operation order
/// mirrors the microprogram accumulator (u, then c3..c7 terms, then c9).
pub fn fast_cos(x: f32) -> f32 {
	const H: f32 = f32::from_bits(0x3fc9_0fda);
	vu_sine(x + H)
}

/// FUN_00221240 = VU0 entry 0xc90 (the same microprogram past the `+ pi/2` step, vu0_rotation.txt): the odd sine
/// polynomial of the argument itself, wrapped to [-pi, pi) by the caller.
pub fn fast_sin(x: f32) -> f32 {
	vu_sine(x)
}

fn vu_sine(a: f32) -> f32 {
	const P: f32 = f32::from_bits(0x4049_0fda);
	const C3: f32 = f32::from_bits(0xbe2a_aaa4);
	const C5: f32 = f32::from_bits(0x3c08_873e);
	const C7: f32 = f32::from_bits(0xb94f_b21d);
	const C9: f32 = f32::from_bits(0x362e_9c14);
	let mut u = a.min(P - a);
	u = u.max(-P - a);
	let u2 = u * u;
	let u3 = u * u2;
	let u5 = u3 * u2;
	let u7 = u5 * u2;
	let u9 = u7 * u2;
	let mut r = u;
	r += C3 * u3;
	r += C5 * u5;
	r += C7 * u7;
	r + C9 * u9
}

/// The two shake records: A (every Novalis writer, shifts along basis row 2 [I: the up-like axis]) and
/// B (no Novalis writer, shifts along row 0 [I: forward]).
#[derive(Clone, Copy, Default, Debug)]
pub struct CamShake {
	pub a: ShakeRecord,
	pub b: ShakeRecord,
}

impl CamShake {

	/// The render-position offset for a camera looking along (yaw, pitch); orientation is untouched.
	pub fn offset(&self, yaw: f32, pitch: f32) -> V3 {
		let (sy, cy) = yaw.sin_cos();
		let (sp, cp) = pitch.sin_cos();
		let fwd = [cp * cy, cp * sy, sp];
		let up = [-sp * cy, -sp * sy, cp];
		// FUN_00220f40: each basis row is rescaled to the length `offset` (zero vector when the row sums to 0).
		let set_len = |len: f32, v: V3| {
			let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
			if n == 0.0 { [0.0; 3] } else { v.map(|c| c * len / n) }
		};
		add(set_len(self.a.offset, up), set_len(self.b.offset, fwd))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::physics::vsub1;
	use crate::{iso, toc};

	#[test]
	fn shake_offset_follows_the_record_rows() {
		let mut sh = CamShake::default();
		sh.a.offset = 0.1;
		assert_eq!(sh.offset(0.0, 0.0), [0.0, 0.0, 0.1]);
		sh.a.offset = 0.0;
		sh.b.offset = 0.1;
		let o = sh.offset(0.0, 0.0);
		assert!((o[0] - 0.1).abs() < 1e-7 && o[1] == 0.0 && o[2] == 0.0);
		let y = std::f32::consts::FRAC_PI_2;
		let o = sh.offset(y, 0.0);
		assert!(o[0].abs() < 1e-8 && (o[1] - 0.1).abs() < 1e-7);
		sh.b.offset = 0.0;
		assert_eq!(sh.offset(0.3, 0.2), [0.0; 3]);
	}

	// REN-7b tests 1 and 3: follow View scalar and the ship-travel minimum.
	// CAM-7.1 test 1: free-look engages on the 6th held grounded tick (scale(7) = 6), not the 5th.
	// CAM-7.2a tests 1 and 4: the ledge cue aims at 12 deg/tick (rock cue 8 deg); no wall angle, no cue.
	// CAM-7.2a tests 2, 3, 5 and 7.2b test 3: underwater swim height 0.5 at k 0.003, the aim reads look 0.25 that tick.
	// CAM-7.2a test 6: item-9 fire moves the xy target with k 0.04 instead of 0.015.
	fn aim_rig() -> CameraRig {
		let mut rig = CameraRig::with_cameras(Vec::new(), Vec::new(), Vec::new());
		rig.reset([0.0; 3], std::f32::consts::FRAC_PI_2);
		rig.s = [0.0; 3];
		rig.pos = [0.0, -4.6, 3.6];
		rig.dir = [0.0, -4.6, 1.6];
		rig.orient_blend = 0;
		rig
	}

	// CAM-7.2b test 1: the follow camera looks down 24.5 degrees at the hero.
	// CAM-7.2b test 2: the aim point is the raw hero without the flag and blends to S over 75 ticks with it.
	// CAM-7.2b test 4: the boom retreat latch and gains.
	// CAM-7.2b test 5: a ceiling 2.0 above and a floor 0.5 below hang the pivot 0.95 under the ceiling.
	// CAM-7.2b test 6: the orientation blends from the previous forward over 75 ticks after a reset.
	// NOV-7.2b tests 1, 2: progress accumulation and the ease.
	// NOV-7.2b tests 3, 4: the release step shows the held view, step 27 is 0.478 of the way, then the follow pose.
	// NOV-7.2b test 5: nlerp flips b when the dot is negative; view_quat/quat_view round-trip the forward.
	// NOV-5.1: class-6 clear zeroes timer and latched max and adds nothing; a null/other camera runs the curve.
	// NOV-5.3: native cosine vs std cos, and the whole shake curve.
	#[test]
	fn shake_fast_cos() {
		assert!((fast_cos(0.0) - 1.0).abs() < 1e-6);
		assert!((fast_cos(std::f32::consts::PI) + 1.0).abs() < 1e-5);
		for t in 0..=21 {
			let mut a = 2.0 * t as f32;
			let tau = 2.0 * std::f32::consts::PI;
			while a >= std::f32::consts::PI {
				a -= tau;
			}
			while a < -std::f32::consts::PI {
				a += tau;
			}
			assert!((fast_cos(a) - a.cos()).abs() < 1e-5, "t {t}");
		}
	}

	fn empty() -> World {
		World::new(std::iter::empty::<[V3; 3]>(), 4.0)
	}

	fn view_bits(v: View) -> [u32; 7] {
		[v.pos[0], v.pos[1], v.pos[2], v.target[0], v.target[1], v.target[2], v.proj_scalar].map(f32::to_bits)
	}

	// FRAME-1c test 1/2: the split (frame_input, one tick_step per 50 Hz tick, view) is `update`; 0.04 - 0.02 - 0.02 is 0.0
	// exactly in f32, so `update`'s factor is 0 and the views are bitwise equal; view(1.0) is the newest tick.
	// NOV-7.2 test 1: the class-5 mode-3 radius ease (hero at the origin, r 0 -> 10 along +X) and the 150th-update snap.
	// NOV-6.6 test 6: the rock cue's one-shot aim, stick override and size springs.
	// CAM-7.4 tests 1-5: the follow-rig sub-mode machine.
	// HERO-JUMPLAND test 6: a run jump (state 9, group 4) keeps sub-mode 2 through its landed ticks.
	// CAM-7.3c tests 1-4.
	// CAM-6 tests: sphere / end / line masks of FUN_0020ea40.
	// CAM-7.3a tests (1)-(5).
	fn wall_at(x: f32) -> World {
		World::new([[[x, -20.0, -20.0], [x, 20.0, -20.0], [x, 20.0, 20.0]], [[x, -20.0, -20.0], [x, 20.0, 20.0], [x, -20.0, 20.0]]], 4.0)
	}

	// CAM-1 test 2: a grid shape covering the blocked hit point keeps the count at 0.
	// CAM-1 test 3: the blocking solid's owner is reported every blocked tick.
	// CAM-1 test 4: the rebuild against a wall at x = -3 pulls the camera 0.5 short of the hit.
	#[test]
	fn rebuild_pulls_back_from_a_wall() {
		let w = wall_at(-3.0);
		let mut rig = CameraRig::with_cameras(Vec::new(), Vec::new(), Vec::new());
		rig.reset_behind_hero([0.0; 3], 0.0, Some(&Scene { world: &w, solids: &[] }));
		let want = [-2.502_878, 0.0, 1.769_706_6];
		for k in 0..3 {
			assert!((rig.pos[k] - want[k]).abs() < 1e-4, "{:?}", rig.pos);
		}
	}

	// CAM-1 test 5: a hit exactly at the look point puts the camera on the hero, then the same-spot nudge moves it.
	#[test]
	fn rebuild_with_a_zero_distance_hit_nudges() {
		let w = wall_at(0.0);
		let mut rig = CameraRig::with_cameras(Vec::new(), Vec::new(), Vec::new());
		rig.reset_behind_hero([0.0; 3], 0.0, Some(&Scene { world: &w, solids: &[] }));
		assert!((rig.pos[0].abs() - 0.5).abs() < 1e-6 && (rig.pos[1].abs() - 0.5).abs() < 1e-6 && (rig.pos[2] - 0.5).abs() < 1e-6, "{:?}", rig.pos);
	}

	fn rec(class: i32, prio: u8, meta: u8, mode: u8, cuboid: Option<usize>) -> CamRec {
		CamRec { class, prio, meta, mode, cuboid, ..CamRec::default() }
	}

	/// CAM-4a test 1: the census rows of both levels read from the gameplay camera table.
	#[test]
	fn camera_record_census() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let row = |r: &CamRec| (r.class, r.prio, r.meta, r.mode, r.cuboid);
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let nov: Vec<_> = camera_records(&l).unwrap().iter().map(row).collect();
		assert_eq!(nov, vec![(3, 6, 5, 7, None), (17, 4, 3, 3, Some(82)), (0, 5, 0, 0, None), (4, 6, 3, 3, None), (5, 6, 0, 2, None), (6, 6, 1, 7, None), (7, 6, 3, 7, None)]);
		let l = Level::load(&mut disc, &levels[0]).unwrap();
		let vel: Vec<_> = camera_records(&l).unwrap().iter().map(row).collect();
		assert_eq!(vel, vec![(23, 4, 3, 3, Some(12)), (17, 4, 1, 4, Some(13)), (0, 5, 0, 0, None), (4, 6, 3, 3, None), (5, 6, 0, 2, None), (6, 6, 1, 7, None), (7, 6, 3, 7, None)]);
	}

	/// SHR-CAMDRVa1 test 1: Kerwan registration: 34 records in slots 0..33 with cleared runtime words, the start camera
	/// is record 30 (the first class-0 record), no previous record, the smoothed up seeded to world Z.
	#[test]
	fn record_driver_registration_kerwan() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[3]).unwrap();
		let rig = CameraRig::new(&l).unwrap();
		let classes: Vec<i32> = rig.recs.iter().map(|r| r.class).collect();
		assert_eq!(rig.recs.len(), 34, "{classes:?}");
		assert_eq!((rig.rec_cur, rig.rec_prev, rig.rec_ticks, rig.planet), (30, None, 0, 3), "{classes:?}");
		assert_eq!((classes[30], classes[22], classes[21]), (0, 1, 18), "{classes:?}");
		assert!(rig.recs.iter().all(|r| r.armed == 0 && r.request == 0 && r.rate == -1.0));
		assert_eq!((rig.up_s, rig.up_t, rig.up_v), ([0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0; 3]));
	}

	/// SHR-CAMDRVa1 camera_preupdate: the seeded up stays world Z; an up opposed to the target (dot < -0.98) springs
	/// toward the nudged target (0.2, 0.2, 1.2) while `up_t` keeps (0, 0, 1).
	#[test]
	fn record_driver_preupdate_up_spring() {
		let mut rig = CameraRig::with_cameras(Vec::new(), Vec::new(), Vec::new());
		rig.preupdate();
		assert_eq!(rig.up_s, [0.0, 0.0, 1.0]);
		rig.up_s = [0.0, 0.0, -1.0];
		rig.preupdate();
		assert_eq!(rig.up_t, [0.0, 0.0, 1.0]);
		let v = (vmul(f32::from_bits(0x3c75_c28f), 0.2), vmul(f32::from_bits(0x3c75_c28f), vsub1(gameplay::ee_add(1.0, UP_NUDGE), -1.0)));
		assert_eq!(rig.up_v[0], v.0);
		assert_eq!(rig.up_v[2], v.1);
		assert!((rig.up_s[0] - 0.003_102_3).abs() < 1e-6 && rig.up_s[2] < -0.9999, "{:?}", rig.up_s);
	}

	// WPN-6.3d test 6 / BRIDGE-56: the first `remote` call from a follow view starts the 0.018 entry blend at the follow
	// pose; the 57th call outputs the remote pose exactly and ends it; the release blend starts from the rolled pose
	// (`bup` seeded before `release` resets `over_up`) and ends at Z; a remote entry during a release blend starts from `bcur`.
	// WPN-6.3d test 7: while the class-6 camera is current the shake consumer zeroes both records.
	// ARI-44 FUN_002fa040: the look cue LOOK_HEIGHT + bias * 0.35 with k 0.005 in follow mode only; the callback (and so
	// the post step) runs on every camera tick with a nonzero bias.
	// ARI-47d tests (1), (2), (8): setup values, the first tick's slews and the yaw spring.
	// ARI-47d test (3): an anchor below the hero gives the ratio 0 (distance 3.65, height 0, look -0.5).
	// ARI-47d tests (4), (5): the line clip and the sweep step.
	// ARI-47d tests (6), (7) and ARI-47f test (5): the state-0x25 ramp, the fall keeps class 7, the exit hands the
	// pose to the follow rig with the orientation counter 0 when grounded.
	// ARI-47f tests (1), (2), (4): counts, the hero polar coordinates and one length step.
	// ARI-47f test (3): a swing entry from the class-7 target pose blends invisibly over 42 ticks; a re-setup blends 32,
	// a rail entry 34.
	// NOV-12b / BRIDGE-81: the free-look update reports the look live only with no switch blend running.
	/// KER-69 (1): the marker's speed gate is 0.1 * PAL_DT in EE float32.
	#[test]
	fn cam_marker_speed_gate() {
		assert_eq!(vmul(PAL_DT, f32::from_bits(0x3dcccccd)), f32::from_bits(0x3b03126f));
	}

}
