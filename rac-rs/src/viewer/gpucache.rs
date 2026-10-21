//! PC perf (speed lane A, B2 + B3): "same inputs as last frame" keys for `Gpu::set_marker` and `Gpu::cull_ties`, and their
//! off-by-default check modes (RAC_MARKER_CHECK=1 / RAC_CULL_CHECK=1: do the full work every frame as before, and compare the
//! result with what the skipped path would have kept, byte for byte; stderr `marker_check:` / `cull_check:` lines count mismatches).
use super::clip::ClipView;
use std::sync::atomic::{AtomicU64, Ordering};

fn env_on(name: &str) -> bool {
	std::env::var(name).is_ok_and(|v| v == "1")
}
pub(super) fn marker_check_on() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| env_on("RAC_MARKER_CHECK"))
}
pub(super) fn cull_check_on() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| env_on("RAC_CULL_CHECK"))
}

pub(super) fn bits3(v: [f32; 3]) -> [u32; 3] {
	v.map(f32::to_bits)
}

/// Everything `set_marker` reads besides the skinned pose (compared separately against `Scene::marker_skin`) and the model.
#[derive(Clone, PartialEq)]
pub(super) struct MarkerKey {
	pub skin: Option<usize>,
	pub feet: [u32; 3],
	pub yaw: u32,
	pub tilt: [u32; 2],
	pub light: [u32; 4],
	pub id: u32,
	pub tint: Option<([u32; 3], usize)>,
	pub hero_len: usize,
	pub clank: Option<(usize, usize, [u32; 3])>,
	pub ghosts: [Option<(usize, usize, u8)>; 5],
	pub bomb: Option<(usize, usize, [u32; 3])>,
	pub shift: Option<(usize, usize, u32)>,
}

impl MarkerKey {
	/// Check mode only: which parts differ (bit per field, in declaration order).
	pub fn diff(&self, o: &MarkerKey) -> u32 {
		[self.skin != o.skin, self.feet != o.feet, self.yaw != o.yaw, self.tilt != o.tilt, self.light != o.light, self.id != o.id, self.tint != o.tint, self.hero_len != o.hero_len, self.clank != o.clank, self.ghosts != o.ghosts, self.bomb != o.bomb, self.shift != o.shift]
			.iter()
			.enumerate()
			.fold(0, |m, (i, &d)| m | (d as u32) << i)
	}
}

/// What `cull_ties` reads that can change between frames: the clip view, the occlusion mask, the draw distance mode.
#[derive(Clone, PartialEq)]
pub(super) struct TiePassKey {
	pub view: Option<[u32; 14]>,
	pub mask: [u8; 128],
	pub far: Option<super::fardraw::Level>,
}

pub(super) fn view_bits(v: &ClipView) -> [u32; 14] {
	let b = |a: [f32; 3]| a.map(f32::to_bits);
	let (e, s, u, f) = (b(v.e), b(v.s), b(v.u), b(v.f));
	[e[0], e[1], e[2], s[0], s[1], s[2], u[0], u[1], u[2], f[0], f[1], f[2], v.tx.to_bits(), v.ty.to_bits()]
}

static MARKER_FRAMES: AtomicU64 = AtomicU64::new(0);
static MARKER_SAME: AtomicU64 = AtomicU64::new(0);
static MARKER_BAD: AtomicU64 = AtomicU64::new(0);
static MARKER_WHY: [AtomicU64; 13] = [const { AtomicU64::new(0) }; 13];
static CULL_FRAMES: AtomicU64 = AtomicU64::new(0);
static CULL_SAME: AtomicU64 = AtomicU64::new(0);
static CULL_BAD: AtomicU64 = AtomicU64::new(0);
static CULL_WRITES_RAW: AtomicU64 = AtomicU64::new(0);
static CULL_WRITES: AtomicU64 = AtomicU64::new(0);

/// One `set_marker` call in check mode; `same_inputs` = the skip would have applied, `same_out` = the fresh bytes equal the kept ones.
pub(super) fn marker_note(same_inputs: bool, same_out: bool, why: u32) {
	let n = MARKER_FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
	for (i, w) in MARKER_WHY.iter().enumerate() {
		if why >> i & 1 != 0 {
			w.fetch_add(1, Ordering::Relaxed);
		}
	}
	if same_inputs {
		MARKER_SAME.fetch_add(1, Ordering::Relaxed);
		if !same_out {
			let m = MARKER_BAD.fetch_add(1, Ordering::Relaxed) + 1;
			if m <= 20 {
				eprintln!("marker_check: MISMATCH #{m}");
			}
		}
	}
	if n % 500 == 0 {
		let why: Vec<u64> = MARKER_WHY.iter().map(|w| w.load(Ordering::Relaxed)).collect();
		eprintln!("marker_check: {} calls, {} skippable, {} mismatches; differs (skin feet yaw tilt light id tint hero_len clank ghosts bomb shift | pose bytes): {:?}", n, MARKER_SAME.load(Ordering::Relaxed), MARKER_BAD.load(Ordering::Relaxed), why);
	}
}

/// One `cull_ties` call in check mode; `writes_raw` / `writes` = range writes before and after coalescing.
pub(super) fn cull_note(same_inputs: bool, same_out: bool, writes_raw: usize, writes: usize) {
	let n = CULL_FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
	CULL_WRITES_RAW.fetch_add(writes_raw as u64, Ordering::Relaxed);
	CULL_WRITES.fetch_add(writes as u64, Ordering::Relaxed);
	if same_inputs {
		CULL_SAME.fetch_add(1, Ordering::Relaxed);
		if !same_out {
			let m = CULL_BAD.fetch_add(1, Ordering::Relaxed) + 1;
			if m <= 20 {
				eprintln!("cull_check: MISMATCH #{m}");
			}
		}
	}
	if n % 500 == 0 {
		eprintln!("cull_check: {} passes, {} skippable, {} mismatches, range writes {} -> {} coalesced", n, CULL_SAME.load(Ordering::Relaxed), CULL_BAD.load(Ordering::Relaxed), CULL_WRITES_RAW.load(Ordering::Relaxed), CULL_WRITES.load(Ordering::Relaxed));
	}
}
