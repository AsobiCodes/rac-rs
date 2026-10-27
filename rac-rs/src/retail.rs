//! Retail mode (`view LEVEL --play --retail STATE_DIR`): the game's own EE code runs the level in the ee-oracle
//! interpreter, resumed from a PCSX2 savestate (<dev>/oracle\p2s.py output). rac-rs feeds it the pad once per
//! 50 Hz tick and draws its mobys and camera; none of the rac-rs game logic runs.

use crate::game::Game;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use crate::retail::disc::va;

mod gscap;
pub mod hot;
mod rec;
#[path = "../../oracle/pcsx2rec/playfeed.rs"]
mod playfeed;
pub mod asset_feed;
pub mod queue_b;
pub mod efx_own;
pub mod efx_draw;
pub mod efx_vu;
pub mod snap_pipe;
pub mod queue_q1;
pub mod queue_q2;
pub mod queue_fxa;
pub mod queue_fxb;
pub mod queue_story;
pub mod snd;
pub mod fmv_bridge;
pub mod memcard;
pub mod modcam;
pub mod aim;
pub mod pcext;
pub mod binds;
pub mod pcctl;
  pub mod prompts;
  pub mod xglyphs;
  pub mod pspad;
pub mod lxpad;
pub mod vendor;
pub mod pcopts;
pub mod strafe;
pub mod shipmouse;
pub mod debugkeys;
pub mod devtools;
pub mod oldsim;
pub mod mouse_menu;
pub mod menu_keys;
pub mod title_boot;
pub mod disc;
pub mod disc_hands;
pub mod disc_map;

/// One field's input as the interpreter thread consumes it (what the recorder writes and the replayer reads).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FieldIn {
	pub buttons: u16,
	/// [rx, ry, lx, ly], the order `Machine::set_pad` takes.
	pub sticks: [u8; 4],
	/// Level-travel request poked before the field (RAC_RETAIL_TRAVEL, or a replayed `!TRAVEL`).
	pub travel: Option<u32>,
	/// Modern camera input (retail/modcam.rs); zero when the switch is off.
	pub mc: modcam::Input,
}

static STATE: OnceLock<String> = OnceLock::new();
static POOLS: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());
/// Retail's occlusion mask of the field the render camera was read from (REN-CULL-OCC); the viewer's tie / fragment / moby tests use it
/// instead of rebuilding one from the level grid and rac-rs's own camera.
pub static OCC_MASK: Mutex<Option<[u8; 128]>> = Mutex::new(None);
/// The savestate's level byte (0x15ed84) read by `enable`: the level `view` loads.
static START_LEVEL: OnceLock<u32> = OnceLock::new();
/// GS-full display mode requested (`--gsfull`; RAC_GSFULL=1 does the same): the game's own picture from the GS.
static GSFULL: AtomicBool = AtomicBool::new(false);
/// `--retail-cold` (the ISO path): no savestate, `Machine::cold` boots the retail game from the disc (logos, title, New Game, Veldin).
static COLD: OnceLock<String> = OnceLock::new();

const LEVEL: u32 = 0x15ed84;
/// Spare props per class beyond the savestate's count, for mobys the game spawns later.
pub const SPARE: u32 = 4;
/// Props per class for a class the level loads but neither the start state nor a placement holds (spawned mobys:
/// bombs, debris, Glove of Doom bots, gadget mobys); `Retail::sync` grows a pool past it on demand.
pub const SPAWN: u32 = 8;
/// The held moby pointer (boot BSS, every level): [C] aridia_gadgets.md:143 (FUN_002c0a58 reads `*0x1403e0` as the held
/// Trespasser), kerwan_enemies.md:573.
pub const HELD_MOBY: u32 = 0x1403e0;
/// Classes the viewer has a prop model for (non-empty faces), set by `load_level`: `sync` grows only their pools.
static MODELLED: Mutex<Option<HashMap<u32, u32>>> = Mutex::new(None);

/// Moby table start / end pointers: Veldin, Rilgar, Oltanis and Gemlik keep them 0x40 higher (each overlay's code
/// windows referencing them paired with Novalis's, as for `camera_globals`).
pub fn table(level: u32) -> (u32, u32) {
	if matches!(level, 0 | 5 | 14 | 16) { (va(0x15ff98), va(0x15ffa0)) } else { (va(0x15ff58), va(0x15ff60)) }
}

/// Each level's shift of the overlay globals from Novalis's addresses (`camera_globals`).
const LEVEL_SHIFT: [i32; 19] = [-0x400, 0, 0x180, -0x380, -0x300, 0, 0x300, -0x400, 0x380, -0x300, 0, 0x580, 0, -0x180, 0x300, 0x180, -0x80, 0x480, 0x580];

/// Render camera of the level's overlay: position, basis rows (row 0 forward, row 2 up) and the tan half-FOV scalar.
/// Every overlay keeps the three at Novalis's addresses plus one shift per level (each level's code windows that
/// reference them, masked of immediates, paired with Novalis's: about 1100 / 190 / 60 votes, no other candidate).
fn camera_globals(level: u32) -> Option<(u32, u32, u32)> {
	let d = *LEVEL_SHIFT.get(level as usize)?;
	let at = |a: u32| va(a.wrapping_add_signed(d));
	Some((at(0x1671c0), at(0x1673d0), at(0x16cef0)))
}

/// [PC] The landing hint (`pools_hint`) takes a moby table only up to this many slots and with class ids below `HINT_CLASS`. Real
/// tables (43 whole-game savestates) span 551-1789 slots and hold class ids up to 1973; on some trips the level byte already names the
/// new level while the table pointers still hold other data (Rilgar -> Blarg: a "table" with 26,000+ classes, counted every field of
/// a ~30 s landing, each new hint a whole prop slot order on the main thread: 15-19 fps).
const HINT_SLOTS: u32 = 4096;
const HINT_CLASS: u32 = 0x1000;

/// (class, props) of a level's live moby table: the count per class plus half of it (at least `SPARE`).
pub fn count_pools(level: u32, rd32: impl Fn(u32) -> u32, rd16: impl Fn(u32) -> u16) -> Vec<(u32, u32)> {
	let (s, e) = table(level);
	let (s, e) = (rd32(s) & 0x01ff_ffff, rd32(e) & 0x01ff_ffff);
	let mut n: HashMap<u32, u32> = HashMap::new();
	let mut a = s;
	while a < e && a < 0x0200_0000 - 0x100 {
		let class = rd16(a + 0xa6) as u32;
		if rd32(a + 0x24) != 0 && class != 0 {
			*n.entry(class).or_insert(0) += 1;
		}
		a += 0x100;
	}
	// [PC] A class with many live mobys at the start spawns more of them: half its count again (at least `SPARE`), so its
	// pool rarely grows after the load (`Retail::sync`).
	let mut pools: Vec<(u32, u32)> = n.into_iter().map(|(c, k)| (c, k + SPARE.max(k / 2))).collect();
	pools.sort();
	pools
}

/// Turns retail mode on for the next game start; the prop pools come from the state's moby table.
pub fn enable(dir: &str) -> Result<(), String> {
	let ram = std::fs::read(format!("{dir}/eeMemory.bin")).map_err(|e| format!("{dir}/eeMemory.bin: {e}"))?;
	// RAC_RETAIL_REC / RAC_RETAIL_REPLAY: opened here so a bad path stops the start instead of running unrecorded.
	rec::open_from_env()?;
	rec::replay_from_env()?;
	let at = |a: u32| ram.get(a as usize..a as usize + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
	let level = ram[LEVEL as usize] as u32;
	set_pools(count_pools(level, at, |a| at(a) as u16));
	// The disc of the run (ORACLE_ISO, else the oracle's default PAL image) and its video mode (retail/disc.rs).
	ee_oracle::ver::from_iso(&std::env::var("ORACLE_ISO").unwrap_or_else(|_| ee_oracle::DEFAULT_ISO.into()));
	let _ = STATE.set(dir.to_string());
	let _ = START_LEVEL.set(level);
	Ok(())
}

/// Turns retail mode on for the next game start without a savestate (`--retail-cold`): the machine is the cold boot of the retail
/// ELF from `iso` (ee_oracle `Machine::cold`); the first level is Veldin (0), loaded after the front end's New Game.
pub fn enable_cold(iso: &str) -> Result<(), String> {
	rec::open_from_env()?;
	rec::replay_from_env()?;
	// The oracle reads level data (overlay signatures, sound banks) from the same disc: ORACLE_ISO, else its default path.
	if std::env::var_os("ORACLE_ISO").is_none() {
		// SAFETY: called from main before any other thread exists.
		unsafe { std::env::set_var("ORACLE_ISO", iso) };
	}
	let _ = COLD.set(iso.to_string());
	// The disc (PAL or USA) and its video mode (retail/disc.rs).
	ee_oracle::ver::from_iso(iso);
	disc::announce();
	ee_oracle::ver::on_rebuild(disc::rescope);
	let _ = STATE.set("cold:".to_string());
	let _ = START_LEVEL.set(0);
	Ok(())
}

/// `rac-rs warm-gpu` (setup, launchers): opens the GPU capture once so the driver builds and keeps its kernels; a first
/// start of the game then has them at once instead of drawing on the CPU until they are built.
pub fn warm_gpu() -> Result<(), String> {
	if gscap::gpu_warm() { Ok(()) } else { Err("no GPU for the capture".into()) }
}

/// The window's GPU (viewer/gpu.rs, before the game loads): the GS capture uses a GPU of its own by default only when that one
/// is NVIDIA (gscap draw/gpu.rs `set_default`).
pub fn gpu_capture_default(nvidia: bool) {
	gscap::gpu_default(nvidia);
}

pub fn active() -> bool {
	STATE.get().is_some()
}

/// Turns the GS-full display on for the next game start (`--gsfull`).
pub fn enable_gsfull() {
	GSFULL.store(true, Ordering::SeqCst);
}

/// Whether retail mode shows the game's own GS picture (`--gsfull` or RAC_GSFULL=1; default off).
pub fn gsfull_on() -> bool {
	GSFULL.load(Ordering::SeqCst) || std::env::var("RAC_GSFULL").is_ok_and(|v| v != "0" && !v.is_empty())
}

/// The level of the savestate `enable` loaded (None outside retail mode).
pub fn start_level() -> Option<u32> {
	START_LEVEL.get().copied()
}

/// (class, count) of the retail prop pools `Game::with_story` appends in retail mode.
pub fn pools() -> Vec<(u32, u32)> {
	if let Some(p) = POOLS_OVERRIDE.with(|o| o.borrow().clone()) {
		return p;
	}
	POOLS.lock().map(|v| v.clone()).unwrap_or_default()
}

thread_local! {
	static POOLS_OVERRIDE: std::cell::RefCell<Option<Vec<(u32, u32)>>> = const { std::cell::RefCell::new(None) };
}

/// [PC] Runs `f` with `pools()` answering `p` on this thread (the level switch asks for the prop slot order of pools it has not
/// set yet).
pub fn with_pools<T>(p: Vec<(u32, u32)>, f: impl FnOnce() -> T) -> T {
	POOLS_OVERRIDE.with(|o| *o.borrow_mut() = Some(p));
	let r = f();
	POOLS_OVERRIDE.with(|o| *o.borrow_mut() = None);
	r
}

/// [PC] (level, pools) of the level retail is landing on, counted from its moby table before the arrival (worker, every field of
/// the landing once the table holds mobys); None after the arrival. Only a guess for the level prefetch: the switch uses the arrival's.
static POOLS_HINT: Mutex<Option<(u32, Vec<(u32, u32)>)>> = Mutex::new(None);

pub fn pools_hint(level: u32) -> Option<Vec<(u32, u32)>> {
	POOLS_HINT.lock().ok().and_then(|h| h.as_ref().filter(|h| h.0 == level).map(|h| h.1.clone()))
}

/// The pools of the level retail travelled to (`Retail::switch`), for the next `Game::with_story`.
pub fn set_pools(p: Vec<(u32, u32)>) {
	if let Ok(mut v) = POOLS.lock() {
		*v = p;
	}
}

/// Widens the pools of the level being loaded (viewer `load_level`, before the prop slot order and the game are built):
/// each placed class gets at least its placement count + `SPARE`, every other class the level loads `SPARE`. The game
/// rewrites a moby's class in place (+0xa6 / +0x22 / +0x24, e.g. the Eudora bridges 0x41c -> 0x1b0 when built and
/// back on a reload, FUN_002c6180) and spawns classes the start state did not hold; without a pool `sync` finds no free
/// prop and the moby is not drawn. Idempotent (a second call leaves the pools as they are).
pub fn widen_pools(placed: &[crate::gameplay::MobyInstance], loaded: impl IntoIterator<Item = (u32, u32)>) {
	if deferring() {
		let (k, loaded) = (placed_counts(placed), loaded.into_iter().collect::<Vec<(u32, u32)>>());
		DEFER_WIDEN.with(|d| d.borrow_mut().push((k.clone(), loaded.clone())));
		defer(Box::new(move || set_pools(widened_counts(pools(), k, loaded))));
		return;
	}
	set_pools(widened(pools(), placed, loaded));
}

/// [PC] A level prefetch (viewer `app.rs` load_level, run on a background thread while the game is still in the travel
/// sequence) must not touch the globals of the level being played: its writes to them are queued here (in call order) and
/// run by `run_deferred` on the main thread at the level switch, where the same calls would have run.
pub type Effect = Box<dyn FnOnce() + Send>;

/// One deferred `widen_pools`: the placement counts per class and the loaded classes.
pub type Widen = (HashMap<u32, u32>, Vec<(u32, u32)>);

thread_local! {
	static DEFER: std::cell::RefCell<Option<Vec<Effect>>> = const { std::cell::RefCell::new(None) };
	static DEFER_WIDEN: std::cell::RefCell<Vec<Widen>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The `widen_pools` calls this thread deferred, in call order (their effects are in `defer_take`'s queue).
pub fn defer_take_widen() -> Vec<Widen> {
	DEFER_WIDEN.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

/// The pools the deferred widenings leave when run on `pools` (what `set_pools` holds after the queued effects ran).
pub fn widen_all(mut pools: Vec<(u32, u32)>, widen: &[Widen]) -> Vec<(u32, u32)> {
	for (k, loaded) in widen {
		pools = widened_counts(pools, k.clone(), loaded.iter().copied());
	}
	pools
}

/// This thread queues its global writes (the prefetch thread, between `defer_begin` and `defer_take`).
pub fn deferring() -> bool {
	DEFER.with(|d| d.borrow().is_some())
}

pub fn defer_begin() {
	DEFER.with(|d| *d.borrow_mut() = Some(Vec::new()));
}

pub fn defer_take() -> Vec<Effect> {
	DEFER.with(|d| d.borrow_mut().take().unwrap_or_default())
}

pub fn defer(f: Effect) {
	DEFER.with(|d| d.borrow_mut().get_or_insert_with(Vec::new).push(f));
}

/// `widen_pools` as a pure function (the audit tool replays it): `pools` widened so each placed class holds at least its
/// placement count + `SPARE` and every other loaded class `SPAWN`.
pub fn widened(pools: Vec<(u32, u32)>, placed: &[crate::gameplay::MobyInstance], loaded: impl IntoIterator<Item = (u32, u32)>) -> Vec<(u32, u32)> {
	widened_counts(pools, placed_counts(placed), loaded)
}

fn placed_counts(placed: &[crate::gameplay::MobyInstance]) -> HashMap<u32, u32> {
	let mut k: HashMap<u32, u32> = HashMap::new();
	for m in placed.iter().filter(|m| m.class_id != 0) {
		*k.entry(m.class_id).or_insert(0) += 1;
	}
	k
}

fn widened_counts(pools: Vec<(u32, u32)>, k: HashMap<u32, u32>, loaded: impl IntoIterator<Item = (u32, u32)>) -> Vec<(u32, u32)> {
	let mut n: HashMap<u32, u32> = pools.into_iter().collect();
	for (c, k) in k {
		let e = n.entry(c).or_insert(0);
		*e = (*e).max(k + SPARE);
	}
	for (c, verts) in loaded.into_iter().filter(|&(c, _)| c != 0) {
		let e = n.entry(c).or_insert(0);
		*e = (*e).max(spawn_pool(verts));
	}
	let mut p: Vec<(u32, u32)> = n.into_iter().collect();
	p.sort();
	p
}

/// The classes the viewer built a prop model for (`load_level`, retail mode); `sync` grows only their pools.
pub fn set_modelled(c: HashMap<u32, u32>) {
	if let Ok(mut v) = MODELLED.lock() {
		*v = Some(c);
	}
}

fn modelled(c: u32) -> bool {
	MODELLED.lock().is_ok_and(|v| v.as_ref().is_some_and(|s| s.contains_key(&c)))
}

/// [PC] Vertices of the prop model of class `c` (`set_modelled`), 0 when unknown.
fn model_verts(c: u32) -> u32 {
	MODELLED.lock().ok().and_then(|v| v.as_ref().and_then(|s| s.get(&c).copied())).unwrap_or(0)
}

/// [PC] Pool size of a class the level loads but nothing holds (spawned mobys): `SPAWN`, more for a small model. Growing a
/// pool at run time rebuilds the level's whole prop index (about 14 ms on a laptop, a visible hitch), the spawn bursts seen are
/// 9-12 live of a 8-prop pool, and a spare prop of a 200-vertex model costs a few KB: a vertex budget decides, 8..=`SPAWN_MAX`.
pub fn spawn_pool(verts: u32) -> u32 {
	if no_room() {
		return SPAWN;
	}
	(SPAWN_VERTS / verts.max(1)).clamp(SPAWN, SPAWN_MAX)
}

/// RAC_POOL_ROOM=0: the old pool sizes (SPAWN props, doubling), to compare the growth hitch.
fn no_room() -> bool {
	static V: OnceLock<bool> = OnceLock::new();
	*V.get_or_init(|| std::env::var("RAC_POOL_ROOM").is_ok_and(|v| v == "0"))
}

/// Spare vertices a spawn-only class pool aims at, and the most props that gets (see `spawn_pool`).
const SPAWN_VERTS: u32 = 6000;
const SPAWN_MAX: u32 = 32;
/// Vertices a grown pool leaves as spare room at least (`grown`), and the most props that gets.
const GROW_VERTS: u32 = 24000;
const GROW_MAX: u32 = 96;

/// A pool that ran out in `sync`: `need` live mobys of the class this field. The new count leaves room for the next
/// burst: `need` + `SPAWN`, and at least twice the old count and twice `need` so a growing class rebuilds the slots rarely.
pub fn grown(class: u32, old: u32, need: u32) -> u32 {
	let room = if no_room() { 0 } else { (GROW_VERTS / model_verts(class).max(1)).clamp(SPAWN, GROW_MAX) };
	(need + SPAWN).max(old * 2).max(need * 2).max(need + room)
}

/// A camera pose: position, yaw, pitch (rac-rs convention, up positive) and the retail tan half-FOV scalar.
pub type Cam = ([f32; 3], f32, f32, f32);

pub mod warp;

/// One moby as the interpreter left it after a field.
pub struct Moby {
	pub addr: u32,
	pub class: u32,
	pub pos: [f32; 3],
	pub rot: [f32; 3],
	/// The draw basis is mirrored in model Y (+0x34 bit 0x8000 negates basis row 1): `rot` is the plain rotation (`basis_pose`).
	pub mirror: bool,
	/// Scale over the class header's base scale.
	pub scale: f32,
	pub shown: bool,
	/// Retail culled it only at the 4:3 side plane (`DrawView::culls`): rac-rs shows it when its wider plane keeps it.
	pub side_only: Option<SideOnly>,
	/// The 4:3 draw tests as rac-rs reproduces them (tests compare them with `+0x31`).
	#[cfg(test)]
	pub culls: Option<Culls>,
	/// (+0x34 flags, +0x32 draw distance, draw-distance cap read apart from `DrawView`), for the tests' own recheck of the rule.
	#[cfg(test)]
	pub raw: (u16, i16, i32),
	pub rgb: [u8; 3],
	pub light: u32,
	pub clock: Clock,
	/// Its +0x64 joint override records (`joint_records`, empty when none): the joint evaluator FUN_00268a80 applies every
	/// evaluated moby's list (hero_movement.md HERO-17b) [C]; e.g. Clank's (class 601, `*0x1404d4`) blink records.
	pub fx: Vec<crate::anim::JointFx>,
	/// Draw alpha byte +0x23 (moby header colour word = min(fade, 0x80) * +0x23 >> 7, moby_format.md CLASS-tint-alpha). [C]
	pub alpha: u8,
	/// Draw distance +0x32 (whole metres; the draw fade of the header alpha). [C]
	pub draw: i16,
	/// The tail-packet colour +0x90 while flag +0x34 & 0x10 is set (FUN_00263020 init = class +0x40; overrides by actors). [C]
	pub ovr: Option<u32>,
}

/// The inputs of the 4:3 side tests of a moby that retail's draw rejected for a side plane (x or y) and nothing else: view-space
/// x, y and depth of its bounding sphere, its radius (all 1/1024 m) and the vertical tangent rac-rs renders with (`tan_v`: the retail
/// horizontal scalar times `PROJ_Y_RATIO`, as `self.cam.tan_half`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SideOnly {
	vx: f32,
	vy: f32,
	vz: f32,
	w: f32,
	tan_v: f32,
}

impl SideOnly {
	/// Retail's horizontal reject (`tx * z - (|x| - sx * w)` with its sign bit set) with the plane of a window `aspect` wide:
	/// the same vertical tangent, so the horizontal one is `tan_v * aspect`. REN-MOBY-DRAWN keeps the near / far / vertical tests.
	pub fn inside(&self, aspect: f32) -> bool {
		self.inside_lag(aspect, ViewLag::NONE)
	}

	/// [PC] `inside` for a drawn view that trails the tick's: both side planes turned out by `lag.ang` radians and the sphere
	/// grown by `lag.grow` (1/1024 m). A smoothed frame's camera lies between the last two ticks (`Retail::camera`) while
	/// retail's verdict is the newest tick's, so without it a moby at the trailing screen edge vanishes while still on screen.
	pub fn inside_lag(&self, aspect: f32, lag: ViewLag) -> bool {
		let turn = |t: f32| if lag.ang > 0.0 { (t.atan() + lag.ang).min(1.45).tan() } else { t };
		let (tx, ty) = (turn(self.tan_v * aspect), turn(self.tan_v));
		let (sx, sy) = ((1.0 + tx * tx).sqrt(), (1.0 + ty * ty).sqrt());
		let w = self.w + lag.grow;
		!(tx * self.vz - (self.vx.abs() - sx * w)).is_sign_negative() && !(ty * self.vz - (self.vy.abs() - sy * w)).is_sign_negative()
	}
}

/// [PC] How far the drawn view can trail the newest tick's (`SideOnly::inside_lag`, `Retail::view_lag`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewLag {
	/// Radians the side planes turn out.
	pub ang: f32,
	/// Growth of the sphere radius, 1/1024 m.
	pub grow: f32,
}

impl ViewLag {
	pub const NONE: ViewLag = ViewLag { ang: 0.0, grow: 0.0 };
}

/// What the retail moby draw (`moby_draw_build` FUN_0026a338 and its overlay twins, REN-MOBY-DRAWN in decomp/notes) tests, one
/// flag per criterion, true = rejects. `+0x31` is 1 when none of them does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Culls {
	occ: bool,
	hidden: bool,
	dist: bool,
	near: bool,
	side_x: bool,
	side_y: bool,
}

impl Culls {
	#[cfg(test)]
	fn any(&self) -> bool {
		self.occ || self.hidden || self.dist || self.near || self.side_x || self.side_y
	}

	/// Rejected by a side plane (horizontal or vertical) and by nothing else.
	fn side_only(&self) -> bool {
		(self.side_x || self.side_y) && !(self.occ || self.hidden || self.dist || self.near)
	}
}

/// The camera side of the retail moby draw, from the overlay's globals: eye, the world-to-view rows at 0x167080, the
/// projection scalars 0x16cef0 / 0x16cef4 and the 128-byte occlusion mask at 0x174100 (Novalis addresses plus the level shift).
struct DrawView {
	eye: [f32; 3],
	rows: [[f32; 3]; 3],
	tx: f32,
	ty: f32,
	near: f32,
	/// Stored `sqrt(1 + t^2)` of each tangent (vf22 of the retail draw, 0x16d020 / 0x16d024 plus the level shift).
	sx: f32,
	sy: f32,
	/// The draw-distance cap in metres, `DAT_0015ff70` (`DAT_0015ffb0` in levels 0, 5, 14, 16): 500 at level setup, 0x90 during
	/// Visibomb flight, 0x40 with the debug toggle. Applied with a signed `pminw` to `+0x32`.
	cap: i32,
	mask: [u8; 128],
}

impl DrawView {
	fn read(m: &ee_oracle::Machine, level: u32) -> Option<DrawView> {
		let d = *LEVEL_SHIFT.get(level as usize)?;
		let at = |a: u32| va(a.wrapping_add_signed(d));
		Some(DrawView {
			eye: m.rdv3(at(0x1671c0)),
			rows: [m.rdv3(at(0x167080)), m.rdv3(at(0x167090)), m.rdv3(at(0x1670a0))],
			tx: m.rdf(at(0x16cef0)),
			ty: m.rdf(at(0x16cef4)),
			near: m.rdf(at(0x16cee0)),
			sx: m.rdf(at(0x16d020)),
			sy: m.rdf(at(0x16d024)),
			cap: m.rd32(if matches!(level, 0 | 5 | 14 | 16) { va(0x15ffb0) } else { va(0x15ff70) }) as i32,
			mask: std::array::from_fn(|i| m.rd8(at(0x174100) + i as u32)),
		})
	}

	/// The retail draw's tests of the moby at `a` (asm 0x26a6c4-0x26a7b8 in Novalis): occlusion `(+0x36 & mask[+0x37])`, hidden
	/// `+0x34 & 0x81`, draw distance `((min(+0x32, cap) << 10) - w) - (z - w)`, near `32 - (w + z)`, side `t * z - (|v| - s * w)`.
	/// Returns them with the sphere's view-space (x, z) and radius.
	fn culls(&self, m: &ee_oracle::Machine, a: u32) -> (Culls, SideOnly) {
		let (c, w) = (m.rdv3(a), m.rdf(a + 0xc));
		let d: [f32; 3] = std::array::from_fn(|k| c[k] - self.eye[k] * 1024.0);
		// vf25..vf27 are the columns of the world-to-view matrix (VMULAbc / VMADDAbc / VMADDbc by d.x, d.y, d.z).
		let dot = |k: usize| (d[0] * self.rows[0][k] + d[1] * self.rows[1][k]) + d[2] * self.rows[2][k];
		let (vx, vy, vz) = (dot(0), dot(1), dot(2));
		let (tx, ty) = (self.tx, self.ty);
		let (sx, sy) = (self.sx, self.sy);
		let limit = ((m.rd16(a + 0x32) as i16 as i32).min(self.cap) << 10) as f32;
		let culls = Culls {
			occ: m.rd8(a + 0x36) & self.mask[m.rd8(a + 0x37) as usize & 0x7f] == 0,
			hidden: m.rd16(a + 0x34) & 0x81 != 0,
			dist: ((limit - w) - (vz - w)).is_sign_negative(),
			near: !(self.near - (w + vz)).is_sign_negative(),
			side_x: (tx * vz - (vx.abs() - sx * w)).is_sign_negative(),
			side_y: (ty * vz - (vy.abs() - sy * w)).is_sign_negative(),
		};
		(culls, SideOnly { vx, vy, vz, w, tan_v: tx * crate::viewer::PROJ_Y_RATIO })
	}
}

/// Moby `+0x50..+0x5f`: cur, next, playing / requested sequence, progress, multiplier, rate (`PropAnim::mirror`).
#[derive(Clone, Copy, Default, Debug)]
pub struct Clock {
	pub cur: u8,
	pub next: u8,
	pub old_seq: u8,
	pub new_seq: u8,
	pub p: f32,
	pub m: f32,
	pub r: f32,
}

impl Clock {
	fn read(m: &ee_oracle::Machine, a: u32) -> Clock {
		Clock { cur: m.rd8(a + 0x50), next: m.rd8(a + 0x51), old_seq: m.rd8(a + 0x52), new_seq: m.rd8(a + 0x53), p: m.rdf(a + 0x54), m: m.rdf(a + 0x58), r: m.rdf(a + 0x5c) }
	}

	pub fn apply(&self, a: &mut crate::anim::PropAnim) {
		a.mirror(self.cur, self.next, self.old_seq, self.new_seq, self.p, self.m, self.r);
	}
}

/// Retail's held wrench (class 71 at `HELD_MOBY`; the crank 0x2997a0 and the held-object mirror 0x224fb8 write its clock):
/// its moby, its clock, and whether it sits in the hand (+0x34 bit 2 set and the slot-0 detached byte 0x1403fa clear; the
/// Comet-Strike throw, hero state 0x15, clears both while it flies) [C: hero_anims.md, dump w0006 f714-757].
#[derive(Clone, Copy)]
pub struct HeldItem {
	pub addr: u32,
	pub clock: Clock,
	pub attached: bool,
}

/// A class-71 moby retail draws away from the hand: the thrown wrench (the detached held moby) and its two afterimage
/// copies (+0x34 0x080a). World vertex = `pos + scale * (x rows[0] + y rows[1] + z rows[2])` from +0x10 and the rows
/// +0xc0..+0xe0, `scale` = +0x2c over the class base; `alpha` = the draw alpha byte +0x23 (moby_format.md: vertex alpha
/// `min(fade, 0x80) * +0x23 >> 7`).
#[derive(Clone, Copy)]
pub struct LooseWrench {
	pub addr: u32,
	pub pos: [f32; 3],
	pub rows: [[f32; 3]; 3],
	pub scale: f32,
	pub alpha: u8,
	pub clock: Clock,
	shown: bool,
	side_only: Option<SideOnly>,
}

/// The hero's attached parts as retail poses them in one field: the held wrench, the back objects d0 `*0x1404d0`
/// (class 607) and d4 `*0x1404d4` (Clank, 601), the +0x64 joint override records of the hero and of d4, and the loose
/// class-71 mobys.
#[derive(Clone, Default)]
pub struct HeroParts {
	pub wrench: Option<HeldItem>,
	pub d0: Option<Clock>,
	pub d4: Option<Clock>,
	pub hero_fx: Vec<crate::anim::JointFx>,
	/// Modern-controls torso twist / backpedal of this field (strafe.rs; default = none).
	pub strafe: strafe::Draw,
	/// The hero +0x60 weighted layers (gadget / weapon hold arms) in chain order (`layer_list`).
	pub layers: Vec<crate::anim::ListLayer>,
	pub clank_fx: Vec<crate::anim::JointFx>,
	pub loose: Vec<LooseWrench>,
	/// The hero's FROM frame while its clock fades from a snapshot (+0x52 == 0xff): the frame-format bytes at +0x68, the
	/// snapshot slot `0x18f0c0 + (+0x50) * 0x800` that hero SetAnim filled through FUN_00266660 (moby_bind_frames
	/// FUN_002632b0, NOV OV:54124-54152) [C]; empty otherwise.
	pub from_frame: Vec<u8>,
}

/// M3a: retail evaluates the hero's joints (the draw matrices and the held-item attach 0x13fe40) BEFORE the hero update
/// steps the +0x60 layer list (hold clock FUN_0022e128, layer pose rebuild FUN_00267308), so the field's picture uses the
/// layer list as the previous field left it [C measured, window eudora1_0002 + w0002.racpad, 331 fields: wrist joint 56
/// vs 0x13fe40 p90 0.033 with this field's list, 0.0001 with the previous field's; the previous field's weights alone 0.032,
/// t alone 0.020]. Swaps `s.parts.layers` for the list kept from the previous snap (empty after a level change).
fn lag_layers(s: &mut Snap, used: &mut (u32, Vec<crate::anim::ListLayer>)) {
	let now = std::mem::take(&mut s.parts.layers);
	let prev = std::mem::replace(used, (s.level, now));
	if prev.0 == s.level {
		s.parts.layers = prev.1;
	}
}

/// The joint override records of a moby's +0x64 chain (next +8, capped at 64) in chain order, the order retail applies
/// them [C: hero_anims.md "+0x64 record layout", dump w0002b]: +3 mode (0 compose, 1 weighted), +4 dest = scratchpad
/// joint matrix 0x70000000 + joint * 0x40 (the skeleton joint itself), +0xc weight (mode 1), +0x10 quaternion xyzw (the
/// port's `JointFx.rot` is its conjugate, headpose.rs `block_quat`), +0x20 scale, +0x30 offset. `reg` keeps chain order
/// under the descending sort.
fn joint_records(m: &ee_oracle::Machine, moby: u32) -> Vec<crate::anim::JointFx> {
	let mut v = Vec::new();
	let mut r = m.rd32(moby + 0x64) & 0x01ff_ffff;
	while r != 0 && v.len() < 64 {
		let q = m.rdv3(r + 0x10);
		v.push(crate::anim::JointFx {
			joint: ((m.rd32(r + 4) & 0xffff) / 0x40) as usize,
			rot: [-q[0], -q[1], -q[2], m.rdf(r + 0x1c)],
			scale: m.rdv3(r + 0x20),
			off: m.rdv3(r + 0x30),
			blend: (m.rd8(r + 3) != 0).then(|| m.rdf(r + 0xc)),
			reg: 0,
		});
		r = m.rd32(r + 8) & 0x01ff_ffff;
	}
	let n = v.len() as u32;
	for (i, f) in v.iter_mut().enumerate() {
		f.reg = n - i as u32;
	}
	v
}

/// M3a F5: the hero's +0x60 weighted layer objects (next +0x1c, at most 6: the pool of anim_layer_alloc FUN_002639a0) in
/// chain order, the order the joint evaluator applies them [C: hero_movement.md HERO-24 "Draw"; layout verified on the
/// r1250 RAM image: jtab 12 object +2 = 20, +0xc = the jtab-12 joint list 52..71]. A frame flagged +6 / +7 comes from
/// the hold set of the hold slot `*(0x140050 + 4i)` (i = 0 class 1, i = 1 class 2) whose object it is, or whose hold
/// model +0x18 it shares [I: a flagged non-slot object was not seen].
/// `HeroParts::from_frame`: header (16 bytes, body size u16 +6 in quadwords) plus body, capped at the 0x800-byte slot.
fn snapshot_frame(m: &ee_oracle::Machine, moby: u32) -> Vec<u8> {
	let f = m.rd32(moby + 0x68) & 0x01ff_ffff;
	if m.rd8(moby + 0x52) != 0xff || f == 0 {
		return Vec::new();
	}
	let n = (16 + m.rd16(f + 6) as u32 * 16).min(0x800);
	(0..n).map(|i| m.rd8(f + i)).collect()
}

fn layer_list(m: &ee_oracle::Machine, hero: u32) -> Vec<crate::anim::ListLayer> {
	let slots = [0u32, 1].map(|i| m.rd32(0x140050 + 4 * i) & 0x01ff_ffff);
	let mut v = Vec::new();
	let mut o = m.rd32(hero + 0x60) & 0x01ff_ffff;
	while o != 0 && v.len() < 6 {
		let set = slots.iter().position(|&s| s == o).or_else(|| {
			let model = m.rd32(o + 0x18);
			slots.iter().position(|&s| s != 0 && model != 0 && m.rd32(s + 0x18) == model)
		});
		let list = m.rd32(o + 0xc) & 0x01ff_ffff;
		let n = m.rd16(o + 2).min(128) as u32;
		v.push(crate::anim::ListLayer {
			pose: crate::anim::LayerPose {
				seq: m.rd8(o + 0x23) as usize,
				frame: m.rd8(o + 0x21) as usize,
				from_seq: m.rd8(o + 0x22) as usize,
				from_frame: m.rd8(o + 0x20) as usize,
				t: m.rdf(o + 0x24),
				weight: m.rdf(o + 8),
			},
			joints: (0..n).map(|k| m.rd8(list + k) as usize).collect(),
			from_hold: set.filter(|_| m.rd8(o + 6) != 0),
			to_hold: set.filter(|_| m.rd8(o + 7) != 0),
		});
		o = m.rd32(o + 0x1c) & 0x01ff_ffff;
	}
	v
}

/// rac-rs Euler angles (viewer `place`) of the draw basis rows the update leaves at moby `+0xc0..+0xef` (actor_transform_bsphere
/// FUN_00265770: Euler of `+0x40`, or the rows themselves under `+0x34` bit 0x100, as for held items on a hero joint).
fn basis_euler(m: &ee_oracle::Machine, a: u32) -> [f32; 3] {
	basis_pose(m, a, false).0
}

/// `basis_euler` plus the mirror of the draw basis (`unmirror`): +0x34 bit 0x8000 makes the standard transform negate basis row 1
/// (actor_transform_bsphere OV:56461-56467, collision_and_gameplay.md), a reflection in model Y that Euler angles cannot carry (the
/// class-331 right track of the Veldin return tank is the left track's model so drawn). Then the angles come from the rows with
/// row 1 negated back, i.e. the plain rotation, and the caller applies the mirror on top (`Prop::mirror`, viewer `place_mirrored`).
fn basis_pose(m: &ee_oracle::Machine, a: u32, unmirror: bool) -> ([f32; 3], bool) {
	let n = |v: [f32; 3]| {
		let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
		if l > 0.0 { v.map(|x| x / l) } else { v }
	};
	let (r0, mut r1, r2) = (n(m.rdv3(a + 0xc0)), n(m.rdv3(a + 0xd0)), n(m.rdv3(a + 0xe0)));
	if r0 == [0.0; 3] {
		return (m.rdv3(a + 0x40), false);
	}
	// Left-handed rows (r0 . r1 x r2 < 0) under the flag: the update really negated row 1 (not the 0x100 supplied-rows path).
	let det = r0[0] * (r1[1] * r2[2] - r1[2] * r2[1]) - r0[1] * (r1[0] * r2[2] - r1[2] * r2[0]) + r0[2] * (r1[0] * r2[1] - r1[1] * r2[0]);
	let mirror = unmirror && m.rd16(a + 0x34) & 0x8000 != 0 && det < -0.5;
	if mirror {
		r1 = r1.map(|x| -x);
	}
	([r1[2].atan2(r2[2]), (-r0[2]).clamp(-1.0, 1.0).asin(), r0[1].atan2(r0[0])], mirror)
}

/// The sound of one field: the SPU2 register writes of the exact 989snd driver port (spumix.rs).
pub type SndBatch = crate::spumix::Batch;

/// The sound events the oracle made in the field it just ran.
pub fn take_sound_of(m: &mut ee_oracle::Machine) -> SndBatch {
	crate::spumix::drain(m)
}

/// The deferred `Capture::take` of one field: waits for the capture thread to finish the field's jobs, then gives the overlay as
/// (width, rows, RGBA8, `disp`). The overlays are tagged by the capture thread's field-end count, so taking later (on the main
/// thread, after the worker already runs the next field) gives the same overlay as taking at the field's end.
pub struct OvLate(Box<dyn FnOnce() -> Option<(u32, u32, Vec<[u8; 4]>, bool)> + Send>);

/// The effects layer of one field, still being built on the VU1 thread (`efx_vu::Shadow::submit`): `Retail::tick` waits for it
/// after the next field's pad is out and appends it to `fx.qa` / `fx.pools` (what `efx_draw::build` appended on the worker).
pub enum FxLate {
	/// The VU1 thread's answer (the inline path).
	Now(std::sync::mpsc::Receiver<efx_vu::FxOut>),
	/// The picture thread hands the receiver of the VU1 thread's answer over once it has walked the field (`snap_pipe`).
	Later(snap_pipe::Late),
}

impl FxLate {
	/// Waits for the field's effects layer.
	fn get(self) -> Option<efx_vu::FxOut> {
		match self {
			FxLate::Now(r) => r.recv().ok(),
			FxLate::Later(l) => l.recv().ok().flatten().and_then(|r| r.recv().ok()),
		}
	}
}

/// What the main thread needs of one field: live mobys, the hero moby (pos, yaw) and the camera.
#[derive(Default)]
pub struct Snap {
	pub mobys: Vec<Moby>,
	/// The page menu on screen (mode 3), for the mouse (retail/mouse_menu.rs).
	pub menu: Option<mouse_menu::MenuSnap>,
	/// The front end (frontbin: title, main menu, New / Load Game) runs instead of a level: nothing of a level is mirrored,
	/// `mode` is the front end's mode word 0x15f5e4 and the picture is the GS's own (title_boot.rs).
	/// A ship fight is on (retail/shipmouse.rs): the left mouse button fires with Cross there.
	pub ship_fight: bool,
	/// Retail's own occlusion mask of this field (128 bytes at 0x174100 + the level shift; None while loading / outside a level).
	pub occ_mask: Option<[u8; 128]>,
	pub front: bool,
	/// The front end is inside the level-load loop of FUN_00231b00 or its frontbin copy (`in_load_loop`): the main thread runs its
	/// fields unthrottled.
	pub turbo: bool,
	/// The front end's own state out of RAM, for the mirror's picture of it (`FrontRam`; Some in the front end only).
	pub front_ram: Option<FrontRam>,
	hero: Option<([f32; 3], f32, Clock)>,
	/// Hero moby +0x34 bit 0 (FUN_00248228; set every hero tick while 0x1413f5, e.g. the shop, vendor.md NOV-30g) [C].
	hero_hidden: bool,
	/// `frozen_view` of this field (the main thread keeps the last frame), and `scene_view`.
	pub frozen: bool,
	pub scene: bool,
	/// `frozen` or a field retail does not present (`NO_PRESENT`): the main thread keeps the last picture. Kept apart from
	/// `frozen` (= `paused()`), which also pauses the music and paces the window.
	pub hold: bool,
	/// Retail's no-present word `NO_PRESENT` is set this field (a frozen mode alone does not set it: the menu modes present
	/// their own picture every field, `menu_presents`).
	pub no_present: bool,
	/// The mode-6 substate `SHIP_STATE` (0 outside mode 6).
	ship: u32,
	/// The hero moby's draw verdict +0x31 (a scene hides the placed hero and draws its own class-0 actor).
	hero_shown: bool,
	/// Scene actors' sequences read from RAM this field: (class, slot +0x53, `scene_seq` block).
	scene_seqs: Vec<(u32, u8, Vec<u8>)>,
	/// Mode 6 substate 4: retail's arrival-warp state (retail/warp.rs).
	warp: Option<warp::WarpRam>,
	/// Mode 6 substates 0 / 8 (takeoff / landing stream): the ship gloss quads (canopy) the cutscene ship callback
	/// FUN_002a69d8 left in RAM this field (`warp::gloss`, the same tables the warp reads).
	ship_gloss: Vec<crate::particles::WorldQuad>,
	cam: Option<Cam>,
	/// The render camera's up row (basis row 2, roll included) read with `cam`; zero = not read.
	cam_up: [f32; 3],
	/// Hero health 0x1415f8 and the bolt counter 0x15ed98, for the rac-rs HUD.
	health: i32,
	bolts: u32,
	/// Owned flags 0x13d4c0, ammo 0x13d428, equipped 0x140408 and last-chosen 0x141660 (inventory.rs).
	have: Vec<bool>,
	ammo: Vec<u16>,
	equipped: u8,
	last: u8,
	/// The pad as the game's pad update left it (pad struct 0x13c940, Eudora FUN_00259330): held +0x1a0, new presses +0x1a4
	/// (both cut to Start / Select while a menu locks the pad, +0x1cc = 2), and the uncut copies +0x1c0 / +0x1c4 Quick Select reads.
	pad: [u32; 4],
	/// The driver's SPU2 register writes of the field (spumix.rs), played by the viewer.
	sound: SndBatch,
	/// The retail GS overlay finished since the last field (oracle gs.rs): (width, rows, RGBA8 with rgb = the
	/// premultiplied colour and a = how much of rac-rs's scene shows through).
	ov: Option<(u32, u32, Vec<[u8; 4]>)>,
	/// `ov` was drawn into the display buffer on a no-present field (gscap `Overlay::disp`): it covers what the
	/// display showed, a held movie picture too.
	ov_disp: bool,
	/// The overlay of this field, not taken yet (in-game only): the worker hands the field over without waiting for the GS
	/// capture thread; `Retail::tick` resolves it after it has sent the next field's pad (`ov` / `ov_disp` then hold it).
	ov_late: Option<OvLate>,
	fx_late: Option<FxLate>,
	/// Game mode 0x15f5a4 (0 gameplay, 3 pause menu; ingame_pause_menu.md): the menu saves the camera and points
	/// the camera globals at its own scene.
	mode: u32,
	/// Retail level 0x15ed84, and the new level's prop pools once its first gameplay field ran (level travel).
	level: u32,
	pools: Option<Vec<(u32, u32)>>,
	/// Nanotech orbs (class 806, no model) the field queued for drawing; None while frozen (keep the last).
	pub orbs: Option<Vec<crate::particles::OrbDraw>>,
	/// The effect pool and the glow / marker draw-queue callbacks (`queue_fx`); None while frozen (keep the last).
	pub fx: Option<RetailFx>,
	/// The vendor shop's off-list mobys while the shop screen is up (`shop_snap`).
	pub shop: Option<ShopSnap>,
	/// The mode-4 dialog's keys (Quit Race? / Quit? ..., `menu_keys::dialog_on`) while one is up.
	pub dialog: Option<menu_keys::Screen>,
	/// Retail's held moby (`HELD_MOBY`) while it is in the table: (address, class +0xa6, clock +0x50..+0x5f).
	pub held: Option<(u32, u32, Clock)>,
	/// The held wrench, back objects, joint records and loose wrenches (`HeroParts`).
	pub parts: HeroParts,
	/// Hero moby +0x40 / +0x44: the Euler x / y its draw rows +0xc0 are built from (FUN_00265770, with yaw +0x48).
	hero_tilt: [f32; 2],
	/// M3a shadows: the shadow-volume records of the drawn frame and the deferred draw path; None while frozen (keep the last).
	pub shadows: Option<RetailShadows>,
	/// [PC] Frame smoothing (viewer/interp.rs): `fx`, `shadows` and `orbs` blended toward the field before, drawn in their place
	/// between two fields. A new snap starts without (None), so each field draws its own lists until the viewer blends again.
	pub smooth: Option<Box<SmoothDraw>>,
	/// Screen words of the world draw FUN_001eae20 (debug, RAC_SCENE_LOG): skip 0x15f5b8, draw mask 0x15f3f4, black fade
	/// 0x15f3fc, white fade 0x15f400, fade gate 0x16a134.
	screen: (u32, u32, f32, f32, u32),
	/// GS PMODE, DISPFB1, BGCOLOR (privileged registers, debug).
	crtc: (u64, u64, u64),
	ms: f32,
	err: Option<String>,
	/// The field this snap is the state of, counted from 1 (`Retail::fields` when `tick` took it): the label of the
	/// RAC_SCENE_LOG lines and of RAC_SHOT_FIELD shots (gsfull frame `field` + 1).
	field: u32,
}

/// Game modes where the main thread keeps the last frame: the moby table, hero and camera are not mirrored. The main
/// loop's mode switch (Eudora OV:54316) draws the world (FUN_001eae20) in modes 0 (gameplay), 2 (in-engine scene:
/// FUN_002897b0 / draw FUN_001eb6f8), 5 (vendor shop) and 6 (ship: draw FUN_00282520 by substate, `frozen_view`). Mode 1
/// (movie: the FMV is drawn over the frame), 3 (pause and galactic-map menus: FUN_00269f00, drawn by the GS capture
/// overlay), 4 (FUN_001f5b48 menu, world only while 0x173154 == 0 [C], not seen in Eudora play) and 7 (FUN_001eb7e0: a
/// sprite screen, no world) stay frozen.
pub fn frozen_mode(mode: u32) -> bool {
	!matches!(mode, 0 | 2 | 5 | 6)
}

/// The native draw's delay against the GS overlay (hybrid). Measured [C stage 9, warp fade 0164 f718-722, field-exact
/// shots]: the GS picture of field N (--gsfull, and the capture overlay that equals it, stage 7) shows the state of field
/// N-2, the native draw that of field N. So the natively drawn state (mobys, hero, camera, warp, scene, view flags) is the
/// snap `d` fields back, with this field's GS overlay, sound, pad and HUD values:
/// - d = 2 (retail-exact, native = overlay) while the player has no camera control or a fade matters: any mode but 0
///   (scenes, ship travel / warp, shop, menus) and a mode-0 fade (black 0x15f3fc or white 0x15f400) that is starting or
///   holding. The fade start is in RAM 2 fields before the overlay shows it, so the newest snap decides.
/// - d = 0 in plain gameplay (mode 0, no fade) and during a mode-0 fade-in (the fade word falling): no added mouse-camera
///   latency; the overlay carries the fade darkness itself [C mirror_m3b: FUN_001eae20 draws it through the GS overlay].
/// Growing holds the last picture (the held snaps are never shown twice). Shrinking drops only snaps nobody sees: a
/// held / frozen field, or one under a full black or white fade; 25 fields after gameplay wants 0 with no such field, one
/// snap per field is dropped anyway (`RAC_SCENE_LOG` prints `native delay`).
/// RAC_RETAIL_NATIVE_DELAY=d forces a fixed d (0 = the old undelayed draw).
fn native_delay(s: Snap) -> Snap {
	static FORCED: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
	let forced = *FORCED.get_or_init(|| std::env::var("RAC_RETAIL_NATIVE_DELAY").ok().and_then(|v| v.parse().ok()));
	NATIVE_Q.with(|q| {
		let mut nd = q.borrow_mut();
		if s.mode != nd.mode {
			eprintln!("retail: mode {} -> {} at field {}", nd.mode, s.mode, s.field);
			nd.mode = s.mode;
		}
		if forced == Some(0) {
			return s;
		}
		let f = s.screen.2.max(s.screen.3);
		let want = match forced {
			Some(d) => d,
			None if s.mode != 0 => 2,
			// Mode 0: a fade starting or growing wants the overlay's timing, a fade-in (falling) gameplay's.
			None if f > nd.fade && f > 0.0 => 2,
			None if f < nd.fade || f <= 0.0 => if s.hold { nd.want } else { 0 },
			None => nd.want,
		};
		nd.fade = f;
		nd.want = want;
		nd.q.push_back(s);
		let unseen = |x: &Snap| x.hold || x.frozen || x.screen.2 >= 1.0 || x.screen.3 >= 1.0;
		if nd.q.len() > want + 1 {
			nd.late = if want == 0 && nd.q.iter().all(|x| x.mode == 0) { nd.late + 1 } else { 0 };
		} else {
			nd.late = 0;
		}
		// Shrink: drop the oldest queued snaps while the delay is above the wanted one and nobody would see them.
		while nd.q.len() > want + 1 && (unseen(&nd.q[0]) || nd.late > 25) {
			let x = nd.q.pop_front().unwrap();
			if std::env::var_os("RAC_SCENE_LOG").is_some() {
				eprintln!("retail: native delay drop field {} (unseen {}) -> delay {}", x.field, unseen(&x), nd.q.len() - 1);
			}
			if nd.late > 25 {
				break;
			}
		}
		let mut out = if nd.q.len() > want {
			nd.q.pop_front().unwrap()
		} else {
			// Grow: hold the last picture until `want` snaps are queued.
			let n = nd.q.back().unwrap();
			let (mode, level, ship, scene, frozen, hero_shown) = nd.last.unwrap_or((n.mode, n.level, n.ship, n.scene, n.frozen, n.hero_shown));
			Snap { hold: true, no_present: true, mode, level, ship, scene, frozen, hero_shown, ..Default::default() }
		};
		let s = if nd.q.is_empty() { None } else { nd.q.back_mut() };
		if let Some(s) = s {
			std::mem::swap(&mut out.sound, &mut s.sound);
			std::mem::swap(&mut out.ov, &mut s.ov);
			std::mem::swap(&mut out.ov_disp, &mut s.ov_disp);
			std::mem::swap(&mut out.ov_late, &mut s.ov_late);
			out.menu = s.menu.clone();
			out.front = s.front;
			out.front_ram = s.front_ram.clone();
			std::mem::swap(&mut out.pools, &mut s.pools);
			std::mem::swap(&mut out.err, &mut s.err);
			(out.pad, out.ms, out.field, out.health, out.bolts, out.equipped, out.last) = (s.pad, s.ms, s.field, s.health, s.bolts, s.equipped, s.last);
			out.have = s.have.clone();
			out.ammo = s.ammo.clone();
		}
		let d = nd.q.len();
		if d != nd.shown_d && std::env::var_os("RAC_SCENE_LOG").is_some() {
			eprintln!("retail: native delay field {} mode {} fade {f} want {want} -> delay {d}{}", out.field, out.mode, if out.hold { " (hold)" } else { "" });
		}
		nd.shown_d = d;
		nd.last = Some((out.mode, out.level, out.ship, out.scene, out.frozen, out.hero_shown));
		out
	})
}

/// `native_delay`'s state (main thread): the queued newer snaps, the wanted delay, the newest fade word, the view flags
/// of the last shown snap (a growing hold keeps them), the delay last logged and the fields gameplay waited to shrink.
#[derive(Default)]
struct NativeDelay {
	q: std::collections::VecDeque<Snap>,
	want: usize,
	fade: f32,
	last: Option<(u32, u32, u32, bool, bool, bool)>,
	shown_d: usize,
	late: u32,
	/// The newest snap's game mode (the `mode a -> b` log line).
	mode: u32,
}

/// Mode 6 (ship) substate word 0x13e050 (boot RAM): FUN_00282520 draws the world in 0 (takeoff stream) and 8 (landing
/// stream) as a scene, 3 (flight away) and 7; 4 is the arrival warp (its own star / ribbon pass, not the level) and
/// 1, 2, 5, 6 draw nothing (level_transition.md).
const SHIP_STATE: u32 = 0x13e050;

/// The main loop's no-present word 0x15f5b8 (Eudora OV:54400 [C]): while it is nonzero the loop skips the whole frame
/// submit (FUN_001e5f60, the vsync / flip func_0x00122298) and only syncs (FUN_002911f8(1), FUN_00291018), and the
/// mode draws FUN_001eb698 / FUN_001eb6f8 return at once, so the screen keeps the last presented picture. Seen in
/// play_0040_0041 [C, RAC_SCENE_LOG]: f1849-1870 (gameplay -> scene cut, then the 0x15f3fc fade-in 1.0 / 0.66 / 0.32),
/// f2626-2639 (scene end) and f2647-2663 (after the movie: the movie's last picture stays up).
const NO_PRESENT: u32 = 0x15f5b8;

/// `frozen_mode` with the mode-6 substate: true when retail draws no level world this field.
pub fn frozen_view(mode: u32, ship: u32) -> bool {
	frozen_mode(mode) || (mode == 6 && !matches!(ship, 0 | 3 | 7 | 8))
}

/// Game mode of the vendor (`0x15f5a4`): the shop and the gadget cinematic that follows a purchase (`FUN_002ae128`).
const VENDOR_MODE: u32 = 5;

/// A scene plays its actors (scene_chunk_parse, Veldin FUN_00244e58; cutscenes.md "Scene actors"): mode 2, or mode 6 in
/// the takeoff / landing streams (substates 0 and 8, which run the same chunk player).
fn scene_view(mode: u32, ship: u32) -> bool {
	mode == 2 || (mode == 6 && matches!(ship, 0 | 8))
}

/// The scene sequence moby `a` plays, copied out of RAM: scene_chunk_parse appends each actor's sequence to its class
/// (`class[+0x48 + 4 * slot]` = actor record + 0x10, slot = the class's sequence count, moby +0x52 = +0x53 = slot) and
/// relocates the frame words at header +0x1c to absolute addresses. Returned as one block in `anim::parse_sequence`'s
/// layout (header, then each frame with its offset written back), or None when +0x53 is not a scene sequence: the
/// record's class word (+0x00) must be the moby's class (or 0x215, the ship actor mode 6 swaps for the ship type), the
/// trigger count +0x12 0xff (every scene record) and every frame after the header.
fn scene_seq(m: &ee_oracle::Machine, a: u32, class: u32) -> Option<Vec<u8>> {
	let hdr = m.rd32(a + 0x24) & 0x01ff_ffff;
	let p = m.rd32(hdr + 0x48 + 4 * m.rd8(a + 0x53) as u32) & 0x01ff_ffff;
	if !(0x100..0x01ff_0000).contains(&p) || m.rd8(p + 0x12) != 0xff {
		return None;
	}
	let rec = m.rd32(p - 0x10);
	if rec != class && rec != 0x215 {
		return None;
	}
	let n = m.rd8(p + 0x10) as u32;
	if n == 0 {
		return None;
	}
	let head = 0x1c + 4 * n;
	let mut out: Vec<u8> = (0..head).map(|i| m.rd8(p + i)).collect();
	for k in 0..n {
		let f = m.rd32(p + 0x1c + 4 * k) & 0x01ff_ffff;
		if f < p + head || f - p > 0x0010_0000 {
			return None;
		}
		let size = 16 + 16 * m.rd16(f + 6) as u32;
		let at = (0x1c + 4 * k) as usize;
		let off = (out.len() as u32).to_le_bytes();
		out[at..at + 4].copy_from_slice(&off);
		out.extend((0..size).map(|i| m.rd8(f + i)));
	}
	Some(out)
}

/// What `Retail::scene_seqs` last put into the viewer's class animations: (class, slot) -> the RAM block it parsed, and
/// the viewer's own sequence it replaced (rac-rs's hero class has more sequences than retail's 134, so a scene slot can
/// land on one of rac-rs's extras), put back when the scene ends. Main thread only.
#[derive(Default)]
struct SceneSeqs {
	done: HashMap<(u32, u8), Vec<u8>>,
	orig: HashMap<(u32, u8), Option<crate::anim::Sequence>>,
	/// The hero / back-Clank / held-item hide flags from before the scene (`sync` restores them after it).
	saved: Option<(bool, bool, bool)>,
}

thread_local! {
	static SCENE_SEQS: std::cell::RefCell<SceneSeqs> = std::cell::RefCell::new(SceneSeqs::default());
	/// `native_delay`'s queue of newer snaps (main thread).
	static NATIVE_Q: std::cell::RefCell<NativeDelay> = std::cell::RefCell::new(Default::default());
}

/// The GS capture (oracle gs.rs) on a thread of its own, as `Machine::capture_gs_async` runs it, plus a count of the
/// jobs it has not run yet. The capture needs about 38 ms per field while a pause menu is up (the game restores the
/// grabbed frame into the main buffer every field) but the interpreter needs 1 ms, so left alone the queue grows by
/// 12 fields per second of menu time and the picture shows the menu seconds late: the menu looked dead (2026-10-08).
/// `pace` makes the interpreter thread wait for the capture instead.
struct Capture {
	/// Finished overlays, each tagged with the field whose jobs published it (1-based, the worker's `field`).
	slot: Arc<Mutex<std::collections::VecDeque<(u64, gscap::Overlay)>>>,
	/// Field-end markers the capture has run (= the last field whose jobs are all drawn).
	ends: Arc<AtomicU64>,
	/// Jobs the interpreter has queued / the capture has run.
	queued: Arc<AtomicU64>,
	ran: Arc<AtomicU64>,
	/// The capture thread ended (a panic): nothing to wait for.
	dead: Arc<AtomicBool>,
	lag: usize,
	/// The sender the machine's `gsq` holds while the capture is fed (the field-end markers go through it even while the
	/// front end's GS-full pipe takes the jobs, title_boot.rs).
	tx: Option<std::sync::mpsc::Sender<ee_oracle::gs::Job>>,
}

/// The fewest fields a key tap is held for (`Retail::press_key`): 60 ms, a quick tap on a pad [I]. The game's pad update
/// (Eudora FUN_00259330) sees a press in one field (new presses = held & !previous held), so one would do for menus.
const TAP_FIELDS: u8 = 3;

/// The click latch of the left (0, Circle) and middle (1, Triangle) mouse buttons: fields each still counts as held.
#[derive(Default)]
struct MouseTaps([u8; 2]);

impl MouseTaps {
	fn press(&mut self, b: usize) {
		if let Some(n) = self.0.get_mut(b) {
			*n = TAP_FIELDS;
		}
	}

	/// One field sampled.
	fn tick(&mut self) {
		self.0 = self.0.map(|n| n.saturating_sub(1));
	}
}

/// How many fields the capture may trail the interpreter by (RAC_RETAIL_GS_LAG=N; 0 = never wait, the old behaviour).
const GS_LAG: usize = 2;
/// The longest the interpreter waits for the capture in one field, ms (a longer wait gives up and resyncs).
const GS_WAIT_MS: u128 = 500;
/// Snap `n` carries the overlay published by field `n - OV_DELAY`'s jobs (RAC_RETAIL_OV_DELAY=N). Before, each snap took
/// whatever overlay the capture thread had finished last: one field early or late by thread timing, and an overlay
/// replaced before a snap took it never showed (the one-field HUD fade-out frames were lost).
const OV_DELAY: u64 = 0;
/// The front end's load loop runs unpaced (the turbo) once this many of its fields in a row were drawn as usual (the fade-in).
const TURBO_STABLE: u32 = 8;
/// In the turbo two fields are drawn every this many ms of wall time (the first draws the loop's picture into the main buffer,
/// the second copies it to the display buffer), the others not: the loading starfield moves on a few times a second.
const TURBO_SHOW_MS: u128 = 100;
/// Every this many fields the worker has the game thread unwind its recompiled frames at the vblank wait
/// (`Cpu::rec_unwind`, the same path as a table rebuild; the game goes on interpreting from the wait, RAM unchanged).
/// Compiled code nests a Rust frame per retail call and some retail paths never return through them: on Novalis the
/// 'ee' thread grew ~9.5 KB a field and overflowed its 256 MB stack at field ~28000 (2026-10-12). Before the O2 probe
/// fix the frequent table rebuilds unwound those frames as a side effect; now nothing else does.
/// [PC] (speed) 10, was 250: fights leak ~17-25 KB of frames a field and the unwinder walks the whole stack (~0.43 ms a MB
/// on Windows), so every 250th field took a 1.6-5.5 ms extra step (a hitch every 5 s); every 10th field it is ~0.15 ms.
const UNWIND_FIELDS: u32 = 10;

/// The GS-full picture last handed to the window: its field counted from 1 (`Frame::field` + 1), for RAC_SHOT_FIELD.
static GSF_SHOWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// [PC] DEV: RAC_SHOT_SEQ shots (viewer/handler.rs) let the game run one more field past RAC_SHOT_FIELD per saved frame.
pub static SHOT_ALLOW: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// RAC_RETAIL_SMOKE=N (tools/smoke.ps1, the speed check before an exe goes out): every 100 fields one `smoke:` line
/// on stderr with the EE thread's `Machine::frame` time per field and the wall-clock fields/s of the worker; after N
/// fields the RAM hash (movetest's trail hash, `rec::hash_ram`) and the process exits. Off: one branch per field.
struct Smoke {
	end: u32,
	start: std::time::Instant,
	lap: std::time::Instant,
	ns: u64,
	max: u64,
	all: u64,
}

impl Smoke {
	fn from_env() -> Smoke {
		let end = std::env::var("RAC_RETAIL_SMOKE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
		let now = std::time::Instant::now();
		Smoke { end, start: now, lap: now, ns: 0, max: 0, all: 0 }
	}

	/// After field `n` (1-based: fields run so far) whose `Machine::frame` took `ns`.
	fn field(&mut self, n: u32, ns: u64, ram: &[u8]) {
		if self.end == 0 {
			return;
		}
		(self.ns, self.max, self.all) = (self.ns + ns, self.max.max(ns), self.all + ns);
		if n % 100 == 0 {
			let wall = self.lap.elapsed().as_secs_f64();
			eprintln!("smoke: field {n} ee {:.2} ms/field (max {:.2}) | worker {:.1} fields/s", self.ns as f64 / 1e8, self.max as f64 / 1e6, 100.0 / wall.max(1e-9));
			(self.ns, self.max, self.lap) = (0, 0, std::time::Instant::now());
		}
		if n >= self.end {
			let wall = self.start.elapsed().as_secs_f64();
			eprintln!("smoke: done {n} fields | ee {:.2} ms/field | worker {:.1} fields/s | ram {:016x}", self.all as f64 / 1e6 / n as f64, n as f64 / wall.max(1e-9), rec::hash_ram(ram));
			if let Ok(p) = std::env::var("RAC_RETAIL_SMOKE_RAM") {
				// Dev only: the RAM at the end of the smoke run, for diffing two runs.
				let _ = std::fs::write(p, ram);
			}
			std::process::exit(0);
		}
	}
}

/// The size of the picture `ov_px` hands over: the sharp HUD's (lane G: the GPU capture drew the overlay again at the window's
/// resolution) or the 512-wide overlay's. Asked before `ov_px`, which takes the pixels.
fn ov_dims(o: &gscap::Overlay) -> (u32, u32) {
	o.hd.as_ref().map_or((o.w, o.rows), |h| (h.w, h.rows))
}

/// The sharp HUD for the window the viewer draws in (`window_h` pixels high), every frame from the render thread: on while the
/// Graphics row HUD is Sharp (`pcext::hud_sharp_now`, pc_options.txt `hud=`), off with RAC_HUD=original or RAC_PC_EXT=0. The GPU
/// capture then draws the overlay a second time at the window's resolution (gscap/draw/gpu.rs, gpu.wgsl k_hd); the 512 picture
/// stays what the game's retail picture is.
pub fn set_hud_target(window_h: u32) {
	static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	let off = *OFF.get_or_init(|| !pcext::enabled() || std::env::var("RAC_HUD").is_ok_and(|v| v.eq_ignore_ascii_case("original")));
	gscap::set_hd_target(window_h, !off && pcext::hud_sharp_now());
}

/// The sharp title / main menu (viewer/hudsharp.rs) for a window `window_h` pixels high: the front end's art quads take the edge-aware
/// sampler. The same switches as `set_hud_target` (Graphics row HUD Sharp; off with RAC_HUD=original or RAC_PC_EXT=0), and the window
/// at least `HD_MIN_PX_PER_ROW` pixels per row of the 448-row menu screen (896 px; below it the nearest picture is as before).
pub fn ui_sharp(window_h: u32) -> bool {
	static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	let off = *OFF.get_or_init(|| !pcext::enabled() || std::env::var("RAC_HUD").is_ok_and(|v| v.eq_ignore_ascii_case("original")));
	!off && pcext::hud_sharp_now() && window_h as f32 >= 448.0 * gscap::HD_MIN_PX_PER_ROW
}

/// An overlay's pixels as RGBA8.
fn ov_px(o: &mut gscap::Overlay) -> Vec<[u8; 4]> {
	if let Some(h) = o.hd.take() {
		return h.rgba;
	}
	if !o.rgba.is_empty() {
		// The CPU capture converted the picture itself (gscap `Gs::publish`).
		return std::mem::take(&mut o.rgba);
	}
	o.px[..(o.w * o.rows) as usize].iter().map(|p| p.map(|c| (c * 255.0 + 0.5) as u8)).collect()
}

/// RAC_PERF_LOG=1 (PC debug aid): the worker thread's field split into sections, one `perf:` line per 100 fields with the
/// average and the worst of each section (ms) and the fields whose busy time passed 20 ms (a 50 Hz field).
/// Sections: wait (for the main thread's pad), pre (pad hooks), ee (Machine::frame), mid (front-end bookkeeping),
/// pace (waits for the GS capture), snap (efx prep + snap + VU1 thread), post (efx build, lag), take (waits for the overlay), tail.
struct WPerf {
	on: bool,
	last: std::time::Instant,
	cur: [u64; 9],
	sum: [u64; 9],
	max: [u64; 9],
	over: u32,
	n: u32,
	worst: (u64, u32),
	/// Not part of the sections: efx prep, snap() alone, the VU1 thread's join wait (ns, summed over the 100 fields).
	ex: [u64; 3],
	lap100: std::time::Instant,
	/// Instructions the EE ran in this field (slow-field print).
	icd: u64,
	/// RAC_PC_SAMPLE=1: the EE pc sampled every 200 us (racy read, diagnostics only); printed for a slow field.
	samples: Option<std::sync::Arc<std::sync::Mutex<Vec<u32>>>>,
}

const WPERF_NAMES: [&str; 9] = ["wait", "pre", "ee", "mid", "pace", "snap", "post", "take", "tail"];

impl WPerf {
	fn from_env() -> WPerf {
		let now = std::time::Instant::now();
		WPerf { on: std::env::var("RAC_PERF_LOG").is_ok_and(|v| v == "1"), last: now, cur: [0; 9], sum: [0; 9], max: [0; 9], over: 0, n: 0, worst: (0, 0), ex: [0; 3], lap100: now, icd: 0, samples: None }
	}

	fn sample_pc(&mut self, pc_addr: usize) {
		if !self.on || !std::env::var_os("RAC_PC_SAMPLE").is_some() {
			return;
		}
		let v = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
		self.samples = Some(v.clone());
		std::thread::spawn(move || loop {
			std::thread::sleep(std::time::Duration::from_micros(200));
			let pc = unsafe { std::ptr::read_volatile(pc_addr as *const u32) };
			v.lock().unwrap().push(pc);
		});
	}

	fn extra(&mut self, i: usize, ns: u64) {
		self.ex[i] += ns;
	}

	fn lap(&mut self, i: usize) {
		if self.on {
			let now = std::time::Instant::now();
			self.cur[i] += now.duration_since(self.last).as_nanos() as u64;
			self.last = now;
		}
	}

	fn field(&mut self, field: u32) {
		if !self.on {
			return;
		}
		let busy: u64 = self.cur[1..].iter().sum();
		for i in 0..9 {
			self.sum[i] += self.cur[i];
			self.max[i] = self.max[i].max(self.cur[i]);
		}
		if busy > 20_000_000 {
			self.over += 1;
		}
		if busy > self.worst.0 {
			self.worst = (busy, field);
		}
		if self.cur[2] > 30_000_000 {
			eprintln!("perf: slow field {field} ({} M insns): {}", self.icd / 1_000_000, WPERF_NAMES.iter().zip(self.cur).map(|(n, v)| format!("{n} {:.1}", v as f64 / 1e6)).collect::<Vec<_>>().join(" "));
		}
		if let Some(sm) = &self.samples {
			let mut v = sm.lock().unwrap();
			if self.cur[2] > 30_000_000 || std::env::var_os("RAC_PERF_ALL").is_some() {
				eprintln!("perf: insns last field {} M", self.icd / 1_000_000);
				let mut h: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
				for &pc in v.iter() {
					*h.entry(pc & !0xff).or_default() += 1;
				}
				let mut hv: Vec<_> = h.into_iter().collect();
				hv.sort_by_key(|x| std::cmp::Reverse(x.1));
				eprintln!("perf: pc samples ({}) by 256-byte block: {}", v.len(), hv.iter().take(12).map(|(a, n)| format!("{a:#x}:{n}")).collect::<Vec<_>>().join(" "));
				let mut h2: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
				for &pc in v.iter() {
					*h2.entry(pc).or_default() += 1;
				}
				let mut hv2: Vec<_> = h2.into_iter().collect();
				hv2.sort_by_key(|x| std::cmp::Reverse(x.1));
				eprintln!("perf: top pcs: {}", hv2.iter().take(12).map(|(a, n)| format!("{a:#x}:{n}")).collect::<Vec<_>>().join(" "));
			}
			v.clear();
		}
		self.cur = [0; 9];
		self.n += 1;
		if self.n == 100 {
			let wall = self.lap100.elapsed().as_secs_f64();
			let avg = WPERF_NAMES.iter().zip(self.sum.iter().zip(self.max)).map(|(n, (s, m))| format!("{n} {:.1}/{:.1}", *s as f64 / 1e8, m as f64 / 1e6)).collect::<Vec<_>>().join(" ");
			eprintln!("perf: field {field} {:.1} fields/s | avg/max ms: {avg} | prep {:.1} snap() {:.1} vujoin {:.1} | busy>20ms: {} worst {:.1} ms at {}", 100.0 / wall.max(1e-9), self.ex[0] as f64 / 1e8, self.ex[1] as f64 / 1e8, self.ex[2] as f64 / 1e8, self.over, self.worst.0 as f64 / 1e6, self.worst.1);
			(self.sum, self.max, self.over, self.n, self.worst, self.ex, self.lap100) = ([0; 9], [0; 9], 0, 0, (0, 0), [0; 3], std::time::Instant::now());
		}
	}
}

/// The front end's GS-full pipe (title_boot.rs): the jobs of the machine go to it from now on (the mirror's HUD capture only
/// keeps getting the field-end markers). false: the pipe could not start (GS.bin and the VU1 files missing in `dir`).
fn start_late_gs(m: &mut ee_oracle::Machine, dir: &str, gtx: &mut Option<std::sync::mpsc::SyncSender<crate::gsfull::pipe::FieldIn>>, jobs_rx: &mut Option<std::sync::mpsc::Receiver<ee_oracle::gs::Job>>) -> bool {
	let Some((tx, shared)) = crate::gsfull::pipe::start(dir) else {
		eprintln!("retail: front end: the GS-full pipe cannot start (no GS.bin / VU1 files in {dir}); no picture of the front end");
		return false;
	};
	let (jtx, jrx) = std::sync::mpsc::channel::<ee_oracle::gs::Job>();
	m.cpu.mem.gsq = Some(jtx);
	(*gtx, *jobs_rx) = (Some(tx), Some(jrx));
	title_boot::set_late(Some(shared));
	eprintln!("retail: front end: GS-full pipe started for the game's own picture");
	true
}

/// The front end's state the mirror draws its picture from (title scene tick, logo and Press Start fades, save slots; all
/// read from frontbin's RAM, `front_snap`). Addresses: frontend_flow.md section 2, frontbin.elf.c FUN_001eafa8 / FUN_001eb090.
#[derive(Clone, Debug, Default)]
pub struct FrontRam {
	/// Title timeline tick DAT_0018cad4 (50 Hz, wraps at DAT_0018cae0).
	pub tick: u32,
	/// Logo alpha DAT_0015ef50 and Press Start alpha DAT_0015ef54 (PS2 alpha, 0x80 = opaque).
	pub logo: u32,
	pub press: u32,
	/// The black fade float DAT_0015f43c (0 none, 1 = fully black; FUN_001f4e50 draws a black quad of alpha 128 * it).
	pub fade: f32,
	/// Language word DAT_0015ed88.
	pub lang: u32,
	/// The five slot summaries at 0x13d2b0 (stride 0x1c): level (-1 empty), bolts, playtime frames, date bytes (+0x15..0x17).
	pub slots: Vec<Option<(u32, u32, u32, [u8; 3])>>,
	/// Cursor words (+0x40) of the slot-list items of the Load Game page (0x1d17b0) and the New Game page (0x1d4ce8) [I: same layout as the other list items].
	pub slot_cur: [u32; 2],
	/// The modal message box (mode 4) words.
	pub dialog: FrontDialog,
}

/// The front end's modal message box (mode 4; FUN_001fb5e8 opens it, FUN_001fc8f0 runs it, FUN_001fb780 draws it): the words it
/// draws from. `kind` DAT_00193280 (3 = memory card messages), `state` the card state DAT_0015eeb0, `flags` DAT_0015eeb4, `card`
/// DAT_0015f5c8, fade-in counter DAT_00193284, box / hint timers DAT_001932a0 / DAT_001932a4, 60 Hz step DAT_0019329c, screen height DAT_0013e504.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrontDialog {
	/// The front end's mode word DAT_0015f5e4: 4 = the box is up.
	pub mode: u32,
	pub kind: u32,
	pub state: u32,
	pub flags: u32,
	pub card: u32,
	pub fade_in: u32,
	pub box_t: u32,
	pub hint_t: u32,
	pub step: u32,
	pub height: u32,
}

/// The snapshot of a field in the front end (frontbin): no level to mirror, the front end's mode word and menu.
/// Return addresses behind the three `jal FUN_00122298` (the vblank wait) of the level-load loops of FUN_00231b00 (boot ELF; the
/// loops at 0x2321b0 / 0x2321f8 / 0x232238, decomp/notes/loading.md): while one of them is on the game thread's stack, the front
/// end only waits for the level data to arrive, it reads no pad and shows no change that depends on the wall clock [C].
const LOAD_LOOP_RA: [u32; 3] = [0x2321b8, 0x232208, 0x232248];
/// `LOAD_LOOP_RA` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const LOAD_LOOP_RA_USA: [u32; 3] = [0x2326a8, 0x2326f8, 0x232738];
/// `LOAD_LOOP_RA` of frontbin's copy of the loops (FUN_002314b8, the same code 0x648 lower): the front end after a Quit Game runs
/// from frontbin, the boot ELF's copy only carries the cold-boot title [C].
const LOAD_LOOP_RA_FRONTBIN: [u32; 3] = [0x231b70, 0x231bc0, 0x231c00];
/// `LOAD_LOOP_RA_FRONTBIN` on the USA disc (disc_map `FRONT`: one aligned run there, USA = PAL + 0x4f0).
const LOAD_LOOP_RA_FRONTBIN_USA: [u32; 3] = [0x232060, 0x2320b0, 0x2320f0];

/// The front end's game thread is parked inside one of the load loops (see `LOAD_LOOP_RA`): its stack above `sp` (and `ra`) holds a
/// return address into FUN_00231b00's wait calls (`frontbin`: into FUN_002314b8's, the front end after a Quit Game).
fn in_load_loop(m: &ee_oracle::Machine, frontbin: bool) -> bool {
	if m.rd32(title_boot::front_mode()) != 6 {
		return false;
	}
	let ras = if frontbin { crate::retail::disc::pick(&LOAD_LOOP_RA_FRONTBIN, &LOAD_LOOP_RA_FRONTBIN_USA) } else { crate::retail::disc::pick(&LOAD_LOOP_RA, &LOAD_LOOP_RA_USA) };
	let ra = m.cpu.g(31) as u32;
	if ras.contains(&ra) {
		return true;
	}
	let sp = m.cpu.g(29) as u32;
	(0..0x100u32).any(|k| {
		let a = sp.wrapping_add(k * 8);
		a < 0x200_0000 - 8 && m.rd32(a + 4) == 0 && ras.contains(&m.rd32(a))
	})
}

fn front_snap(m: &ee_oracle::Machine) -> Snap {
	let mode = m.rd32(title_boot::front_mode());
	let slots = (0..5u32)
		.map(|i| {
			let a = 0x13d2b0 + i * 0x1c;
			let level = m.rd32(a);
			(level < 19).then(|| (level, m.rd32(a + 4), m.rd32(a + 0xc), [m.rd8(a + 0x15), m.rd8(a + 0x16), m.rd8(a + 0x17)]))
		})
		.collect();
	let front_ram = FrontRam { tick: m.rd32(va(0x18cad4)), logo: m.rd32(0x15ef50), press: m.rd32(0x15ef54), fade: f32::from_bits(m.rd32(0x15f43c)), lang: m.rd32(0x15ed88), slots, slot_cur: [m.rd32(va(0x1d17b0) + 0x40), m.rd32(va(0x1d4ce8) + 0x40)], dialog: FrontDialog { mode, kind: m.rd32(va(0x193280)), state: m.rd32(0x15eeb0), flags: m.rd32(0x15eeb4), card: m.rd32(va(0x15f5c8)), fade_in: m.rd32(va(0x193284)), box_t: m.rd32(va(0x1932a0)), hint_t: m.rd32(va(0x1932a4)), step: m.rd32(va(0x19329c)), height: m.rd32(0x13e504) } };
	let menu = title_boot::front_menu(m, mode);
	prompts::set_menu(menu.is_some());
	menu_keys::publish(menu.as_ref().map(menu_keys::screen));
	// The front end's message box (mode 4: memory card, 60 Hz) is no page menu, but its keys and prompts work the menu way
	// (menu_keys::front_dialog: Enter / Space / left click = Yes / Continue, Esc = No), like the in-level mode-4 dialogs.
	let dialog = if menu.is_none() && mode == 4 { crate::frontend::dialog_keys(&front_ram.dialog) } else { None };
	if dialog.is_some() {
		prompts::set_menu(true);
		menu_keys::publish(dialog);
	}
	Snap { front: true, hold: true, mode, level: u32::MAX, menu, dialog, front_ram: Some(front_ram), pad: [m.rd32(0x13cae0), m.rd32(0x13cae4), 0, 0], ..Snap::default() }
}

fn capture_gs(m: &mut ee_oracle::Machine, dir: &str) -> Option<Capture> {
	capture_gs_vram(m, ee_oracle::gs::Gs::load_vram(&format!("{dir}/GS.bin"))?, dir)
}

/// Pages of the page-menu engine (level overlay) whose render-to-texture panels hold actors drawn through VU1 (path 1),
/// which the capture draws only in P1 mode (gscap `P1`): the post-game page (id 0x22) with the green robot (class 1957), and
/// the pause pages with 3D previews (decomp/notes/ingame_pause_menu.md "Item previews"): Weapons 3 (Ratchet with the weapon,
/// the weapon, its ammo), Gadgets 4 (Ratchet), Quick Select 5 (the selected weapon), Items 6 (the gold bolt), Weapons help
/// 0x18 (weapon, ammo) and Gadgets help 0x19. Checked against --gsfull; the other pages draw no model (their pictures are
/// plain textures the capture already shows).
fn rtt_page(s: &Snap) -> bool {
	s.menu.as_ref().is_some_and(|m| !m.front && matches!(m.id, 3 | 4 | 5 | 6 | 0x18 | 0x19 | 0x22))
}

/// The capture thread on a given 4 MB VRAM (a cold boot has no GS.bin: empty VRAM, attached from the first field).
fn capture_gs_vram(m: &mut ee_oracle::Machine, vram: Vec<u8>, vu_dir: &str) -> Option<Capture> {
	let mut c = Capture { tx: None, slot: Arc::new(Mutex::new(Default::default())), ends: Arc::new(AtomicU64::new(0)), queued: Arc::new(AtomicU64::new(0)), ran: Arc::new(AtomicU64::new(0)), dead: Arc::new(AtomicBool::new(false)), lag: std::env::var("RAC_RETAIL_GS_LAG").ok().and_then(|v| v.parse().ok()).unwrap_or(GS_LAG) };
	// The machine's sender feeds a relay that counts the jobs, then the capture thread.
	let (tx, rx) = std::sync::mpsc::channel::<ee_oracle::gs::Job>();
	let (tx2, rx2) = std::sync::mpsc::channel::<ee_oracle::gs::Job>();
	let (queued, ran, dead2, out, ends) = (c.queued.clone(), c.ran.clone(), c.dead.clone(), c.slot.clone(), c.ends.clone());
	let vu_dir = vu_dir.to_string();
	std::thread::Builder::new()
		.name("gs-relay".into())
		.spawn(move || {
			while let Ok(j) = rx.recv() {
				queued.fetch_add(1, Ordering::SeqCst);
				if tx2.send(j).is_err() {
					break;
				}
			}
		})
		.ok()?;
	std::thread::Builder::new()
		.name("gs".into())
		.spawn(move || {
			struct Flag(Arc<AtomicBool>);
			impl Drop for Flag {
				fn drop(&mut self) {
					self.0.store(true, Ordering::SeqCst);
				}
			}
			let _flag = Flag(dead2);
			hot::thread();
			// rac-rs's multi-threaded copy of the oracle capture (retail/gscap.rs): same pixels, menus 4x faster.
			let mut g = gscap::Gs::new(vram);
			g.set_vu1_dir(&vu_dir);
			// RAC_GSCAP_OVHASH=1 (debug): one line per published overlay with an FNV hash of its pixels (GPU against CPU path, old against new kernels).
			let ovhash = std::env::var_os("RAC_GSCAP_OVHASH").is_some();
			// RAC_GSCAP_OVHASH8=1 (debug): one line per published overlay with an FNV hash of the RGBA8 picture the main thread gets.
			let ovhash8 = std::env::var_os("RAC_GSCAP_OVHASH8").is_some();
			let mut ovn = 0u64;
			// [PC] (speed) Lazy readback (gscap `set_lazy`): a published overlay is collected while the next job records, and `ends`
			// (END jobs whose overlays are all out) waits for it. RAC_GS_ASYNC=0 switches it off.
			let lazy = std::env::var("RAC_GS_ASYNC").map(|v| v != "0").unwrap_or(true);
			g.set_lazy(lazy);
			let mut ends_seen = 0u64;
			let mut put = |tag: u64, mut o: gscap::Overlay| {
				if ovhash {
					let h = o.px.iter().flatten().fold(0xcbf2_9ce4_8422_2325u64, |h, v| (h ^ v.to_bits() as u64).wrapping_mul(0x0100_0000_01b3));
					eprintln!("ovhash {ovn} {h:016x} rows {} frozen {}", o.rows, o.frozen);
				}
				if ovhash8 {
					// The sharp HUD (lane G) keeps its own picture: the hash is of the 512 picture then, and `hd` stays for the viewer.
					let hd = o.hd.take();
					let px8 = ov_px(&mut o);
					let h = px8.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| (h ^ u32::from_le_bytes(*p) as u64).wrapping_mul(0x0100_0000_01b3));
					eprintln!("ovhash8 {ovn} {h:016x} rows {} frozen {} disp {}", o.rows, o.frozen, o.disp);
					o.rgba = px8;
					o.hd = hd;
				}
				if ovhash || ovhash8 {
					ovn += 1;
				}
				if let Ok(mut s) = out.lock() {
					s.push_back((tag, o));
				}
			};
			loop {
				let j = match rx2.try_recv() {
					Ok(j) => j,
					Err(std::sync::mpsc::TryRecvError::Empty) => {
						// Nothing queued: the picture in flight is collected now (the main thread may be waiting for it).
						g.gpu_finish(true);
						while let Some((tag, o)) = g.ready.pop_front() {
							put(tag, o);
						}
						ends.store(g.pending_tag().map_or(ends_seen, |t| (t - 1).min(ends_seen)), Ordering::SeqCst);
						match rx2.recv() {
							Ok(j) => j,
							Err(_) => break,
						}
					}
					Err(_) => break,
				};
				let end = matches!(&j, ee_oracle::gs::Job::Gif(q, u64::MAX) if q.is_empty());
				let tag0 = ends_seen + 1;
				g.run_oracle(j);
				if end {
					ends_seen += 1;
				}
				g.gpu_finish(false);
				while let Some((tag, o)) = g.ready.pop_front() {
					put(tag, o);
				}
				if let Some(o) = g.done.take() {
					put(tag0, o);
				}
				ends.store(g.pending_tag().map_or(ends_seen, |t| (t - 1).min(ends_seen)), Ordering::SeqCst);
				ran.fetch_add(1, Ordering::SeqCst);
			}
			// The stream ended: the last picture still being read back goes out first.
			g.gpu_finish(true);
			while let Some((tag, o)) = g.ready.pop_front() {
				put(tag, o);
			}
			ends.store(ends_seen, Ordering::SeqCst);
		})
		.ok()?;
	c.tx = Some(tx.clone());
	m.cpu.mem.gsq = Some(tx);
	Some(c)
}

impl Capture {
	/// After a field: waits until the capture has run every job of the field `GS_LAG` fields back.
	fn pace(&self, marks: &mut std::collections::VecDeque<u64>) {
		if self.lag == 0 {
			return;
		}
		marks.push_back(self.queued.load(Ordering::SeqCst));
		if marks.len() <= self.lag {
			return;
		}
		let target = marks.pop_front().unwrap_or(0);
		let t = std::time::Instant::now();
		while self.ran.load(Ordering::SeqCst) < target && !self.dead.load(Ordering::SeqCst) {
			if t.elapsed().as_millis() > GS_WAIT_MS {
				eprintln!("retail: the GS capture did not finish within {GS_WAIT_MS} ms; not waiting for it");
				marks.clear();
				return;
			}
			std::thread::sleep(std::time::Duration::from_micros(300));
		}
	}

	/// The overlay for snap `field`: waits until field `field - delay` is drawn, then hands out the newest overlay
	/// published up to that field (older ones of the same window are what the display showed before it).
	fn take(&self, field: u64, delay: u64) -> Option<gscap::Overlay> {
		self.handle().take(field, delay)
	}

	/// The parts `take` needs, to run it on another thread.
	fn handle(&self) -> CapTake {
		CapTake { ends: self.ends.clone(), dead: self.dead.clone(), slot: self.slot.clone() }
	}
}

/// `Capture::take`'s shared state (see `OvLate`).
struct CapTake {
	ends: Arc<AtomicU64>,
	dead: Arc<AtomicBool>,
	slot: Arc<Mutex<std::collections::VecDeque<(u64, gscap::Overlay)>>>,
}

impl CapTake {
	fn take(&self, field: u64, delay: u64) -> Option<gscap::Overlay> {
		let want = field.saturating_sub(delay);
		let t = std::time::Instant::now();
		while self.ends.load(Ordering::SeqCst) < want && !self.dead.load(Ordering::SeqCst) && t.elapsed().as_millis() <= GS_WAIT_MS {
			std::thread::sleep(std::time::Duration::from_micros(100));
		}
		let mut q = self.slot.lock().ok()?;
		let mut last = None;
		while q.front().is_some_and(|(f, _)| *f <= want) {
			last = q.pop_front().map(|(_, o)| o);
		}
		last
	}
}

/// The level overlay's first draw-callback queue (FUN_001ebc48 on Eudora; Novalis draw_queue_add 0x21aee0): count
/// 0x15f424 (max 0x40, reset at the tick head, still filled at field end), the fn[] and arg[] arrays, and the
/// level's nanotech orb draw (Eudora FUN_002db2c8, queued by the orb update FUN_002da4a8 when its frustum test
/// passes). Read from each overlay's queue function and its one orb-update call site (decomp/overlays/*/overlay.elf.c).
fn orb_queue(level: u32) -> Option<(u32, u32, u32)> {
	const Q: [(u32, u32, u32); 19] = [
		(0x16d880, 0x16d980, 0x2d91b8), // 0 veldin
		(0x16dc80, 0x16dd80, 0x301530), // 1 novalis
		(0x16de00, 0x16df00, 0x2e3bb0), // 2 aridia
		(0x16d900, 0x16da00, 0x2d2450), // 3 kerwan
		(0x16d980, 0x16da80, 0x2db2c8), // 4 eudora
		(0x16dc80, 0x16dd80, 0x30b1b0), // 5 rilgar
		(0x16df80, 0x16e080, 0x2e79e8), // 6 nebula g34
		(0x16d880, 0x16d980, 0x303aa8), // 7 umbris
		(0x16e000, 0x16e100, 0x2facc0), // 8 batalia
		(0x16d980, 0x16da80, 0x2fa3e8), // 9 gaspar
		(0x16dc80, 0x16dd80, 0x2cb528), // 10 orxon
		(0x16e200, 0x16e300, 0x301888), // 11 pokitaru
		(0x16dc80, 0x16dd80, 0x2f93a0), // 12 hoven
		(0x16db00, 0x16dc00, 0x2fbf78), // 13 oltanis orbit
		(0x16df80, 0x16e080, 0x2f5520), // 14 oltanis
		(0x16de00, 0x16df00, 0x2dc978), // 15 quartu
		(0x16dc00, 0x16dd00, 0x2d7cd0), // 16 kalebo iii
		(0x16e100, 0x16e200, 0x2dae18), // 17 veldin orbit
		(0x16e200, 0x16e300, 0x2e32d0), // 18 veldin finale
	];
	/// `Q` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
	const Q_USA: [(u32, u32, u32); 19] = [
		(0x16d880, 0x16d980, 0x2d9810), // 0 veldin
		(0x16dd00, 0x16de00, 0x301c00), // 1 novalis
		(0x16de80, 0x16df80, 0x2e4280), // 2 aridia
		(0x16d980, 0x16da80, 0x2d2b20), // 3 kerwan
		(0x16da00, 0x16db00, 0x2db9a0), // 4 eudora
		(0x16dc80, 0x16dd80, 0x30b820), // 5 rilgar
		(0x16e000, 0x16e100, 0x2e80c0), // 6 nebula g34
		(0x16d900, 0x16da00, 0x304180), // 7 umbris
		(0x16e080, 0x16e180, 0x2fb390), // 8 batalia
		(0x16da00, 0x16db00, 0x2faac0), // 9 gaspar
		(0x16dd00, 0x16de00, 0x2cbc00), // 10 orxon
		(0x16e280, 0x16e380, 0x301f58), // 11 pokitaru
		(0x16dd00, 0x16de00, 0x2f9a70), // 12 hoven
		(0x16db80, 0x16dc80, 0x2fc650), // 13 oltanis orbit
		(0x16df80, 0x16e080, 0x2f5b70), // 14 oltanis
		(0x16de80, 0x16df80, 0x2dd048), // 15 quartu
		(0x16dc80, 0x16dd80, 0x2d83c0), // 16 kalebo iii
		(0x16e180, 0x16e280, 0x2db4f0), // 17 veldin orbit
		(0x16e280, 0x16e380, 0x2e39a0), // 18 veldin finale
	];
	(*crate::retail::disc::pick(&Q, &Q_USA)).get(level as usize).copied()
}

/// The ground blob queue (Eudora moby_shadow FUN_0024cba8 fills it, shadow_queue_draw FUN_001ebec8 draws it; count
/// 0x15f434, max 0x20, reset with the draw queues by FUN_001eac38): 0x20-byte records at the queue-A fn array + 0x800
/// (the table follows `DAT_0015f434 < 0x20` in each overlay; checked on all 19 levels) [C]. Record: +0 x, +4 y,
/// +8 ground z + 0.025, +0xc radius * height fade, +0x10 ground normal. Casters: crates, planted mines, sand mice, ...
fn blob_queue(m: &ee_oracle::Machine, level: u32) -> Vec<([f32; 3], f32, [f32; 3])> {
	let Some((fns, _, _)) = orb_queue(level) else { return Vec::new() };
	let t = fns + 0x800;
	(0..m.rd32(0x15f434).min(0x20)).map(|i| (m.rdv3(t + 0x20 * i), m.rdf(t + 0x20 * i + 0xc), m.rdv3(t + 0x20 * i + 0x10))).collect()
}

/// The nanotech orbs retail draws this field: the queue entries whose fn is the level's orb draw. Each takes what the
/// draw (Eudora FUN_002db2c8) reads: orb position +0x10, bob pvar[0] (pvar = +0x78), the parent crate (pvar +0xc)
/// rows +0xc0 / +0xd0 / +0xe0, the glass while orb +0x20 == 1 (FUN_002d9be0).
fn orb_draws(m: &ee_oracle::Machine, level: u32) -> Vec<crate::particles::OrbDraw> {
	let Some((fns, args, orb_fn)) = orb_queue(level) else { return Vec::new() };
	if std::env::var_os("RAC_QUEUE_LOG").is_some() {
		let q = |f: u32, n: u32| (0..m.rd32(n).min(0x40)).map(|i| (m.rd32(f + 4 * i), m.rd32(f + 0x100 + 4 * i))).map(|(f, a)| format!("{f:x}:{a:x}")).collect::<Vec<_>>().join(" ");
		eprintln!("retail: queue A [{}] B [{}]", q(fns, 0x15f424), q(fns + 0x200, 0x15f428));
	}
	(0..m.rd32(0x15f424).min(0x40))
		.filter(|&i| m.rd32(fns + 4 * i) == orb_fn)
		.filter_map(|i| {
			let a = m.rd32(args + 4 * i) & 0x01ff_ffff;
			let pv = m.rd32(a + 0x78) & 0x01ff_ffff;
			if a == 0 || pv == 0 {
				return None;
			}
			let cr = m.rd32(pv + 0xc) & 0x01ff_ffff;
			let parent = if cr != 0 { [m.rdv3(cr + 0xc0), m.rdv3(cr + 0xd0), m.rdv3(cr + 0xe0)] } else { [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] };
			let state = m.rd8(a + 0x20);
			Some(crate::particles::OrbDraw { pos: m.rdv3(a + 0x10), bob: m.rdf(pv), parent, glass: state == 1, })
		})
		.collect()
}

/// The level's effect pool (decomp/notes/particles.md section 1; alloc Eudora FUN_00259a38,
/// Veldin FUN_00267248): the gp-relative word holding the pool base (0x800 slots x 0x40), with the high-water slot count
/// at +8, and the occupancy bitmap (bit per slot). Read from each overlay's alloc (gp 0x166c00; levels 0, 5, 14, 16 keep
/// the gp block 0x40 higher, as `table`).
fn effect_pool(level: u32) -> Option<(u32, u32)> {
	const BITMAP: [u32; 19] = [
		0x1b1880, 0x1b1c80, 0x1b1e00, 0x1b1900, 0x1b1980, 0x1b1d00, 0x1b1f80, 0x1b1880, 0x1b2000, 0x1b1980, 0x1b1c80, 0x1b2200, 0x1b1c80, 0x1b1b00, 0x1b1f80, 0x1b1e00, 0x1b1c80, 0x1b2100,
		0x1b2200,
	];
	/// `BITMAP` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
	const BITMAP_USA: [u32; 19] = [
		0x1b1880, 0x1b1d00, 0x1b1e80, 0x1b1980, 0x1b1a00, 0x1b1d00, 0x1b2000, 0x1b1900, 0x1b2080, 0x1b1a00, 0x1b1d00, 0x1b2280, 0x1b1d00, 0x1b1b80, 0x1b1f80, 0x1b1e80, 0x1b1d00, 0x1b2180,
		0x1b2280,
	];
	let bm = *(*crate::retail::disc::pick(&BITMAP, &BITMAP_USA)).get(level as usize)?;
	Some((if matches!(level, 0 | 5 | 14 | 16) { va(0x16012c) } else { va(0x1600ec) }, bm))
}

/// What the field draws besides mobys and orbs in normal mode: the whole effect pool (draw FUN_00267628 / Eudora
/// FUN_00259e18) and the draw-queue callbacks that are glow quads or landing markers.
#[derive(Clone, Debug, Default)]
pub struct RetailFx {
	/// Pool slots in mode 0 (point billboard) and 1 (flat ground quad), as pool sprites.
	pub sprites: Vec<crate::particles::Sprite>,
	/// The pool slot index of each entry of `sprites` (the draw sorts every mode by depth, then falling slot index).
	pub sprite_slots: Vec<u16>,
	/// Pool slots in mode 2 (untextured GS line, +0x10 colour +4 to +0x20 colour +0xc).
	pub lines: Vec<crate::particles::Line>,
	/// The pool slot index of each entry of `lines`.
	pub line_slots: Vec<u16>,
	/// Pool slots in mode 3 (two-point ribbon), drawn by `pool_ribbon_quad`.
	pub ribbons: Vec<PoolRibbon>,
	/// Glow quads (FUN_00255a98 calls of the hero glow callback Eudora FUN_001fade8): (pos, size, bias, rgba).
	pub glows: Vec<([f32; 3], f32, f32, u32)>,
	/// Landing markers (bomb / Glove of Doom reticle draws, Eudora FUN_0029eec8 / FUN_002bc450): pvar +0x10 and +0x20.
	pub reticles: Vec<crate::game::Reticle>,
	/// Frame counter 0x15f5ac (the markers turn once per scale_ticks(180) = 150 frames).
	pub frames: u32,
	/// Ported queue-A callbacks (Taunter rings, tube fade) and queue-B callbacks (`queue_b::fill`), in queue order.
	pub qa: Vec<crate::particles::WorldQuad>,
	pub qb: Vec<crate::particles::WorldQuad>,
	/// The queued Swingshot rope (`queue_b::swing_rope`) and the `qa` length at its queue slot: the viewer builds it with
	/// the posed held attach point 0 (`queue_b::rope_quads`) and draws it between `qa[..n]` and `qa[n..]`.
	pub rope: Option<(usize, queue_b::SwingRope)>,
	/// Fogged alpha quads of the queue-B water callbacks (Drek's Fleet hangar water, `queue_fxa::fleet_water`), drawn with the water.
	pub qw: Vec<crate::particles::WorldQuad>,
	/// The screen filter rects (`queue_b::screen_filter`, the Visibomb remote view), drawn last over the overlay.
	pub screen: Vec<queue_b::ScreenRect>,
	/// The fog block and pool far clamp this field draws with: the previous field's end-of-frame swap (`SwapWords`).
	pub swap: Option<SwapWords>,
	/// The shrub draw-distance cap (m), `DAT_00160424` (`DAT_00160464` in levels 0, 5, 14, 16): 500.0 at level setup,
	/// 144.0 in the Visibomb render snapshot (Eudora FUN_002cac68 twin, decomp/notes/weapons_framework.md:1529) [C].
	pub shrub_cap: Option<f32>,
	/// The RC range static (`queue_b::rc_static`, queue B at the Visibomb range limit), drawn under the overlay.
	pub rc: Option<queue_b::RcStatic>,
	/// Ship ground shadows (draw-queue callback Eudora FUN_0027ee90 = Novalis FUN_002a1a60, NOV-14.5d).
	pub ship_shadows: Vec<ShipShadow>,
	/// Ground blob shadows as retail queued them this field (count 0x15f434, max 0x20, records `blob_queue`): centre (z =
	/// ground + 0.025), half-size, ground normal; drawn by `shadows::blob_square` (Eudora FUN_001ebec8).
	pub blobs: Vec<([f32; 3], f32, [f32; 3])>,
	/// The hero's position (0x13f3d0, as retail/strafe.rs `HERO_POS`): the water pass's contact rings (viewer/waterfx.rs `Rings`).
	pub hero: Option<[f32; 3]>,
	/// [PC] The drawn mobys (address, position, bounding sphere radius) for the water pass's wakes (viewer/waterfx.rs
	/// `Wakes`); set by `snap`.
	pub movers: Vec<(u32, [f32; 3], f32)>,
	/// The 8 dynamic light slots (`dyn_lights`), index = slot.
	pub lights: [Option<crate::particles::PointLight>; 8],
	/// Level-light slots 12..15 as retail RAM holds them (`reserved_lights`); None when the table is not found.
	pub slots: Option<[[[f32; 4]; 4]; 4]>,
	/// The water patches retail's patch draw (Novalis FUN_002b8af8 and its five twins) shows this field: the light vector and the patches.
	pub water: Option<([f32; 3], Vec<crate::water::RamPatch>)>,
	/// Gadgetron vendor glows (class 11 in states 1 and 2, draw callback Novalis FUN_002ba2f0 on queue B).
	pub vendors: Vec<VendorGlow>,
	/// Robot lightning arcs (class 1471 helper, draw callback Veldin FUN_002e2498 on queue A), world strands.
	pub arcs: Vec<ArcDraw>,
	/// Class UV scrolls retail holds in RAM (`asset_feed::class_uv`): (class id, (du, dv) in texture units).
	pub class_uv: Vec<(u32, [f32; 2])>,
	/// The glint pool (pickup twinkles, `glint_pool_draw` twins) as retail RAM holds it (`oldsim::glints`).
	pub glints: [crate::game::Glint; 16],
	/// Aridia's sky sprite records (`oldsim::sky_sprites`), empty on other levels.
	pub sky_sprites: Vec<crate::sky::SkySprite>,
	/// The oceans of Rilgar, Umbris and Gaspar as RAM holds them (`oldsim::oceans`); Batalia and Oltanis come from `ocean` (queue_q1), Aridia and Kerwan from `ocean.rs`.
	pub oceans: Vec<oldsim::RamOcean>,
	/// FIX-blend world strips of the Novalis strip-surface callbacks (`queue_q1::fill`).
	pub strips: Vec<crate::particles::WorldStrip>,
	/// The Hoven water pools of queue C (`queue_q1::fill`); retail flushes queue C before the shrub pass.
	pub pools: Vec<crate::particles::WorldStrip>,
	/// The level's ocean built from RAM while its queue-A wrapper is queued (`queue_q1::ocean`; Batalia, Oltanis).
	pub ocean: Option<std::sync::Arc<crate::ocean::Ocean>>,
}

/// One live lightning arc: its origin and end, both strands in world space, the ribbon U scroll as the draw used it and
/// the two phases (the glow size stand-ins).
#[derive(Clone, Debug)]
pub struct ArcDraw {
	pub origin: [f32; 3],
	pub end: [f32; 3],
	pub strand_a: Vec<[f32; 3]>,
	pub strand_b: Vec<[f32; 3]>,
	pub uv: f32,
	pub phase_a: f32,
	pub phase_b: f32,
}

/// One vendor glow draw: the vendor position, the ramp factor (pvar +0x90), the two scroll globals, the four blue node
/// glow positions and their colour word (moby +0x90 & 0xffff0000).
#[derive(Clone, Debug)]
pub struct VendorGlow {
	pub pos: [f32; 3],
	pub ramp: f32,
	pub scroll: (f32, f32),
	pub nodes: [[f32; 3]; 4],
	pub word: u32,
}

/// The vendor glow draws of this field (queue B entries of the level's FUN_002ba2f0 twin): node k sits at (1.1 cos a, 1.1 sin a,
/// 0.59) with a = k * pi / 2 - pi in the vendor frame (basis rows +0xc0 / +0xd0 / +0xe0, then + pos +0x10) [C: callback tail].
fn vendor_glows(m: &ee_oracle::Machine, level: u32) -> Vec<VendorGlow> {
	let Some(scroll) = queue_b::vendor_scrolls(m, level) else { return Vec::new() };
	queue_b::vendor_mobys(m, level)
		.into_iter()
		.filter_map(|v| {
			let pv = m.rd32(v + 0x78) & 0x01ff_ffff;
			if pv == 0 {
				return None;
			}
			let pos = m.rdv3(v + 0x10);
			let rows = [0, 1, 2].map(|r| m.rdv3(v + 0xc0 + 0x10 * r));
			let nodes = std::array::from_fn(|k| {
				let a = k as f32 * std::f32::consts::PI * 0.5 - std::f32::consts::PI;
				let l = [a.cos() * 1.1, a.sin() * 1.1, 0.59];
				std::array::from_fn(|c| pos[c] + l[0] * rows[0][c] + l[1] * rows[1][c] + l[2] * rows[2][c])
			});
			Some(VendorGlow { pos, ramp: m.rdf(pv + 0x90), scroll, nodes, word: m.rd32(v + 0x90) & 0xffff_0000 })
		})
		.collect()
}

/// The live robot arcs (FUN_002e2498 draws the slots with an owner and state > 1): strands from the polylines in RAM, which are
/// in strand space (along, sideways, up) around the slot's origin (matrix translation) and end, mapped by the tick's frame
/// (`particles::arc_frame`); UV scroll and phases from the slot tables. The 19 points FUN_002e1bf8 writes are used [C].
fn arc_draws(m: &ee_oracle::Machine, level: u32) -> Vec<ArcDraw> {
	let Some(f) = queue_b::arc_live(m, level) else { return Vec::new() };
	let Some(t) = queue_b::arc_tables(m, f) else { return Vec::new() };
	let mut out = Vec::new();
	for i in 0..3u32 {
		let (state, owner) = (m.rd32(t.state + 4 * i), m.rd32(t.owner + 4 * i));
		if owner == 0 || !(2..=4).contains(&state) {
			continue;
		}
		let origin = m.rdv3(t.poly - 0xf0 + 0x40 * i);
		let end = m.rdv3(t.poly - 0x60 + 0x10 * i);
		let frame = crate::particles::arc_frame(origin, end);
		let strand = |base: u32| (0..19u32).map(|k| crate::particles::arc_world(origin, &frame, m.rdv3(base + 0x10 * k))).collect::<Vec<_>>();
		let (strand_a, strand_b) = (strand(t.poly + 0x280 * i), strand(t.poly + 0x140 + 0x280 * i));
		// Phases: A / B at the 0x161bc8 / 0x161bd8 block, 0x161bc8 - 0x161bb8 = 0x10 above the state words.
		out.push(ArcDraw { origin, end, strand_a, strand_b, uv: m.rdf(t.uv + 4 * i), phase_a: m.rdf(t.state + 0x10 + 4 * i), phase_b: m.rdf(t.state + 0x20 + 4 * i) });
	}
	out
}

/// Debug probe `RAC_DBG_CLASS=N`: log the retail moby fields (sync) and the draw inputs (set_props) of class N (read once).
pub fn dbg_class() -> Option<u32> {
	static C: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
	*C.get_or_init(|| std::env::var("RAC_DBG_CLASS").ok().and_then(|v| v.parse().ok()))
}

/// One ship ground shadow: moby +0x10, rows of +0xc0, ship type 0x13e056, alpha after the flight fade, and whether the
/// height is the ground ray (game mode 0x15f5a4 == 6) or pos.z + 0.1.
#[derive(Clone, Copy, Debug)]
pub struct ShipShadow {
	pub pos: [f32; 3],
	pub m: [[f32; 3]; 3],
	pub ship: usize,
	pub alpha: i32,
	pub ground: bool,
}

/// The level's Mine Glove landing-marker draw (Eudora FUN_0029b780, queued by the class-74 mine update FUN_0029bdd8): the
/// bomb marker's body (`q.py diff` with FUN_0029eec8: only the spin vector lives in a global, 0x161330) on pvar +0x10 /
/// +0x20. Binary twins by twins.py on 8 levels; 0 = no identical twin (not mirrored there).
const MINE_RETICLE_FN: [u32; 19] = [0x2a95f8, 0, 0, 0, 0x29b780, 0, 0, 0x2d2688, 0, 0x2c85a0, 0x29ae30, 0, 0, 0x2be018, 0, 0x29f0f8, 0, 0, 0x2abde8];
/// `MINE_RETICLE_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const MINE_RETICLE_FN_USA: [u32; 19] = [0x2a9c50, 0, 0, 0, 0x29be58, 0, 0, 0x2d2d60, 0, 0x2c8c78, 0x29b508, 0, 0, 0x2be6f0, 0, 0x29f7c8, 0, 0, 0x2ac4b8];

/// The level's class-203 landing-marker draw (Eudora FUN_002b5ba8, queued by FUN_002b64e0): the bomb marker's body
/// (`q.py diff:0x29eec8@eudora:0x2b5ba8@eudora`: only the pvar offsets differ) on pvar +0x20 (centre) / +0x30 (normal).
/// [C] Binary twins by twins.py on all 19 levels.
const MARKER203_FN: [u32; 19] = [
	0x2c2b68, 0x2d8758, 0x2c2500, 0x2b4558, 0x2b5ba8, 0x2ee7d8, 0x2cfcf0, 0x2ebbf8, 0x2cbaf8, 0x2e38a8, 0x2b43a8, 0x2e88d8, 0x2daed0, 0x2dc908, 0x2d1790,
	0x2be108, 0x2bd3e0, 0x2c0b28, 0x2c5358,
];
/// `MARKER203_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const MARKER203_FN_USA: [u32; 19] = [
	0x2c31c0, 0x2d8e28, 0x2c2bd0, 0x2b4c28, 0x2b6280, 0x2eee48, 0x2d03c8, 0x2ec2d0, 0x2cc1c8, 0x2e3f80, 0x2b4a80, 0x2e8fa8, 0x2db5a0, 0x2dcfe0, 0x2d1de0,
	0x2be7d8, 0x2bdad0, 0x2c1200, 0x2c5a28,
];

/// The level's ship ground shadow callback (Eudora FUN_0027ee90; twins by twins.py, level order as `orb_queue`).
const SHIP_SHADOW_FN: [u32; 19] = [
	0x28e018, 0x2a1a60, 0x28d710, 0x27a8e8, 0x27ee90, 0x2b6750, 0x29b090, 0x2b4c40, 0x296328, 0x2aa8e8, 0x27ea98, 0x2b0e98, 0x2a4828, 0x2983a8, 0x294398,
	0x27bee8, 0x286e58, 0x2861f8, 0x28db48,
];
/// `SHIP_SHADOW_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const SHIP_SHADOW_FN_USA: [u32; 19] = [
	0x28e670, 0x2a2130, 0x28dde0, 0x27afb8, 0x27f568, 0x2b6dc0, 0x29b768, 0x2b5318, 0x2969f8, 0x2aafc0, 0x27f170, 0x2b1568, 0x2a4ef8, 0x298a80, 0x2949e8,
	0x27c5b8, 0x287548, 0x2868d0, 0x28e218,
];

/// The frustum sphere fade (Eudora FUN_001f2970 -> FUN_001f2878; Novalis FUN_00221e18 -> FUN_00221d20), from the
/// camera registers the frame set up at 0x16d0c0 (Novalis; LEVEL_SHIFT per level, Eudora 0x16cdc0): rows vf25..27,
/// vf24 = eye * 1024 (w 1024), vf20 (f, f*k, ..), vf21, vf22 (sec f, sec f*k, ..), vf23. `buf` = centre and radius in
/// world units (moby[0..3] * 2^-10), `lim` the fade length (32.0). None = culled (-1); else the alpha
/// clamp(ftoi(lim * 1024 - (Vz - R)) >> 7, 0, 0x80). [C] decoded from the raw COP2 words (0x1f2878..0x1f29a8 Eudora).
fn sphere_fade(m: &ee_oracle::Machine, level: u32, buf: [f32; 4], lim: f32) -> Option<i32> {
	let b = va(0x16d0c0u32.wrapping_add_signed(*LEVEL_SHIFT.get(level as usize)?));
	let q = |o: u32| [0, 4, 8, 12].map(|k| m.rdf(b + o + k));
	let (r0, r1, r2, e, v20, v21, v22) = (q(0), q(0x10), q(0x20), q(0x30), q(0x40), q(0x50), q(0x60));
	let p = buf.map(|c| c * e[3]);
	let d = [p[0] - e[0], p[1] - e[1], p[2] - e[2]];
	let v: [f32; 3] = std::array::from_fn(|i| r0[i] * d[0] + r1[i] * d[1] + r2[i] * d[2]);
	let r = p[3];
	let x3 = 0.0 - (r + v[2]);
	let y3 = lim * e[3] - (-r + v[2]);
	if y3.is_sign_negative() || !x3.is_sign_negative() {
		return None;
	}
	let f8 = [v22[0] * r, v22[1] * r];
	let f5 = [v20[0] * v[2], v20[1] * v[2]];
	let f6 = [v[0].abs() - f8[0], v[1].abs() - f8[1]];
	let f7 = [f5[0] - f6[0], f5[1] - f6[1]];
	let _ = v21;
	if f7[1].is_sign_negative() || f7[0].is_sign_negative() {
		return None;
	}
	// vf4 / vf8 only choose the return 0 or 1 (both drawn).
	Some(((y3 as i32 as u32) >> 7).min(0x80) as i32)
}

/// Eudora FUN_0027ee90 (moby `a`): sphere moby[0..3] * 2^-10 through `sphere_fade(32.0)`; game mode 6 with substate
/// 0x13e050 == 3 and tick i16 0x13e054 > scale_ticks(0x96) fades by (tick - 125) * -4 and skips below 1; corners of
/// type i16 0x13e056 (table Eudora 0x1bd9d0 = shadows::SHIP_CORNERS) through the rows of +0xc0 plus +0x10. [C]
fn ship_shadow(m: &ee_oracle::Machine, level: u32, a: u32) -> Option<ShipShadow> {
	let buf = [0, 4, 8, 12].map(|o| m.rdf(a + o) * (1.0 / 1024.0));
	let mut alpha = sphere_fade(m, level, buf, 32.0)?;
	let ground = m.rd32(va(0x15f5a4)) == 6;
	if ground && m.rd32(0x13e050) == 3 {
		let (t, s) = (m.rd16(0x13e054) as i16 as i32, crate::gameplay::pal_ticks(0x96));
		if s < t {
			alpha += (t - s) * -4;
			if alpha < 1 {
				return None;
			}
		}
	}
	Some(ShipShadow { pos: m.rdv3(a + 0x10), m: [m.rdv3(a + 0xc0), m.rdv3(a + 0xd0), m.rdv3(a + 0xe0)], ship: m.rd16(0x13e056) as i16 as usize, alpha, ground })
}

/// The words the frame's last step writes for the next frame's draw (Novalis FUN_00218c70 OV:9936-9971 = Eudora
/// overlay.elf.c 9970-9995, called last in the frame function) [C]: camera water flag 0x167414 clear -> the live fog words
/// 0x15f444..0x15f454 (zone fog FUN_002101b8, the Visibomb snapshot) and clamp 0x1f4000, set -> the underwater words
/// 0x161184.. and 0x40000. The fog block is Novalis 0x16d058 near, +4 far, +0x10 near intensity, +0x14 far intensity,
/// +0x18/+0x1c/+0x20 colour bytes, at the camera level shift (every overlay's copy checked, 2026-10-12); the clamp is
/// 0x1600fc (0x16013c in levels 0, 5, 14, 16). The draw of a frame precedes the swap, so it uses the previous frame's
/// words (decomp/notes/water.md NOV-28.2) [C].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwapWords {
	pub fog_rgb: [u8; 3],
	/// Near and far in metres (raw / 1024, as `gameplay::Lighting::fog`), then the two GS fog intensities.
	pub fog: [f32; 4],
	/// The pool particle far clamp in metres (raw / 4096, as `water::PART_FAR`).
	pub part_far: f32,
	/// [PC] The camera water flag itself (0x167414 at the level shift: the camera is below a water face, or the hero swims
	/// underwater): what picks the underwater words above. The PC underwater look reads it (viewer/underwater.rs).
	pub under: bool,
	/// [PC] The hero's water surface height (boot word 0x13f640, the water face under the hero) while the hero swims (group
	/// word 0x1413dc = 0x11 underwater or 0x12 at the surface): the surface the underwater look measures its depth from.
	pub surf: Option<f32>,
}

/// [PC] Dev aid: RAC_UNDER_LOG=1 prints the camera water flag and its proof words every field.
fn under_log() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var_os("RAC_UNDER_LOG").is_some())
}

impl SwapWords {
	fn read(m: &ee_oracle::Machine, level: u32) -> Option<SwapWords> {
		let b = va(0x16d058u32.wrapping_add_signed(*LEVEL_SHIFT.get(level as usize)?));
		let small = if matches!(level, 0 | 5 | 14 | 16) { 0x40 } else { 0 };
		let part_far = m.rd32(va(0x1600fc + small)) as i32 as f32 / 4096.0;
		if under_log() {
			let flag = m.rd32(va(0x167414u32.wrapping_add_signed(*LEVEL_SHIFT.get(level as usize)?)));
			eprintln!("UNDER_LOG level {level} flag {flag:#x} part_far {part_far} fog_far {} grp {:#x} f640 {}", m.rdf(b + 4) / 1024.0, m.rd32(0x1413dc), m.rdf(0x13f640));
		}
		Some(SwapWords {
			fog_rgb: [m.rd32(b + 0x18) as u8, m.rd32(b + 0x1c) as u8, m.rd32(b + 0x20) as u8],
			fog: [m.rdf(b) / 1024.0, m.rdf(b + 4) / 1024.0, m.rdf(b + 0x10), m.rdf(b + 0x14)],
			part_far,
			under: m.rd32(va(0x167414u32.wrapping_add_signed(*LEVEL_SHIFT.get(level as usize)?))) & 0xff != 0 && (part_far - 64.0).abs() < 0.5,
			surf: matches!(m.rd32(0x1413dc), 0x11 | 0x12).then(|| m.rdf(0x13f640)).filter(|z| z.is_finite()),
		})
	}
}

/// The field's draw takes the previous field's swap words; `lag` keeps this field's for the next.
fn lag_swap(fx: &mut RetailFx, lag: &mut Option<SwapWords>) {
	let now = fx.swap;
	if lag.is_some() {
		fx.swap = *lag;
	}
	*lag = now;
}

/// A mode-3 pool slot: head +0x10, tail +0x20, half-width +0x1c, tail scale +0x2c, colours +4 (head) / +0xc (tail),
/// particle texture +2, additive unless ALPHA +3 == 0x44, fade/cull byte +9, and its pool slot index.
#[derive(Clone, Copy, Debug)]
pub struct PoolRibbon {
	pub head: [f32; 3],
	pub tail: [f32; 3],
	pub half: f32,
	pub scale: f32,
	pub c_head: u32,
	pub c_tail: u32,
	pub tex: u8,
	pub additive: bool,
	pub b9: u8,
	pub slot: u16,
}

/// The level's hero glow draw callback (Eudora FUN_001fade8; twins by twins.py, level order as `orb_queue`).
const GLOW_FN: [u32; 19] = [
	0x207888, 0x228f08, 0x218060, 0x200998, 0x1fade8, 0x235df0, 0x21f668, 0x22f670, 0x219b80, 0x2274e8, 0x1fae70, 0x238960, 0x229ff0, 0x2132f0, 0x21a778,
	0x200688, 0x205868, 0x204760, 0x20fd00,
];
/// `GLOW_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const GLOW_FN_USA: [u32; 19] = [
	0x207d40, 0x229440, 0x218598, 0x200ed0, 0x1fb320, 0x2362a8, 0x21fba0, 0x22fba8, 0x21a0b8, 0x227a20, 0x1fb3a8, 0x238e98, 0x22a528, 0x213828, 0x21ac30,
	0x200bc0, 0x205da0, 0x204c98, 0x210238,
];
/// The level's two landing-marker draws (bomb and Glove of Doom; one body, Eudora FUN_0029eec8 / FUN_002bc450).
const RETICLE_FN: [[u32; 2]; 19] = [
	[0x2ac598, 0x2c77a0],
	[0x2c1cf0, 0x2dd390],
	[0x2abf30, 0x2c7138],
	[0x29df88, 0x2b9190],
	[0x29eec8, 0x2bc450],
	[0x2d7950, 0x2f3410],
	[0x2b9288, 0x2d4928],
	[0x2d5628, 0x2f0830],
	[0x2b4ad8, 0x2d0730],
	[0x2cb540, 0x2e84e0],
	[0x29ddd8, 0x2b8fe0],
	[0x2d2308, 0x2ed510],
	[0x2c42b0, 0x2dfb08],
	[0x2c4ba0, 0x2e2748],
	[0x2bb1c0, 0x2d7bb0],
	[0x2a3ad8, 0x2c33d8],
	[0x2a6e10, 0x2c2018],
	[0x2a93a8, 0x2c5e58],
	[0x2aed88, 0x2c9f90],
];
/// `RETICLE_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const RETICLE_FN_USA: [[u32; 2]; 19] = [
	[0x2acbf0, 0x2c7df8],
	[0x2c23c0, 0x2dda60],
	[0x2ac600, 0x2c7808],
	[0x29e658, 0x2b9860],
	[0x29f5a0, 0x2bcb28],
	[0x2d7fc0, 0x2f3a80],
	[0x2b9960, 0x2d5000],
	[0x2d5d00, 0x2f0f08],
	[0x2b51a8, 0x2d0e00],
	[0x2cbc18, 0x2e8bb8],
	[0x29e4b0, 0x2b96b8],
	[0x2d29d8, 0x2edbe0],
	[0x2c4980, 0x2e01d8],
	[0x2c5278, 0x2e2e20],
	[0x2bb810, 0x2d8200],
	[0x2a41a8, 0x2c3aa8],
	[0x2a7500, 0x2c2708],
	[0x2a9a80, 0x2c6530],
	[0x2af458, 0x2ca660],
];

/// Clank's bulb glow in the list `hero_glows` makes (the slot-3 item light: size bits 0x3d6978d5, colour 0x280000c0).
pub fn is_bulb_glow(g: &([f32; 3], f32, f32, u32)) -> bool {
	g.1.to_bits() == 0x3d69_78d5 && g.3 == 0x2800_00c0
}

/// The hero glow callback (Eudora FUN_001fade8, OV:21972-22057) in gameplay modes (not 2 / 6), by the back-slot kind
/// byte 0x1413f4: 3 = three glows at 0x1410a0.. (size 0.2, rgba 0x141634); 0 = the slot-3 item light
/// (FUN_00200540(3) = 0x1404f8 while 0x1404f4 == 2; kind 2 or 3 and 0x141628 == 0: size 0x3d6978d5, z + 0.015,
/// 0x280000c0), then the shared glow list 0x141120 (count i16 0x141200; per entry size 0x1411c0, bias 0x1411e0,
/// rgba 0x1411a0); 2 = one glow at z - 0.07 (size 0.25, 0x300000c0); 1 = z - 0.02 (0.01 when 0x15edb3 != 0), size the
/// eased word 0x14097c, rgba 0x141650. Bias 0x3da3d70a (0.08) except the list's own.
fn hero_glows(m: &ee_oracle::Machine, out: &mut Vec<([f32; 3], f32, f32, u32)>) {
	let bias = f32::from_bits(0x3da3_d70a);
	let at = |dz: f32| {
		let mut p = m.rdv3(0x1410d0);
		p[2] += dz;
		p
	};
	match m.rd8(0x1413f4) {
		3 => {
			let w = m.rd32(0x141634);
			for i in 0..3 {
				out.push((m.rdv3(0x1410a0 + 0x10 * i), f32::from_bits(0x3e4c_cccd), bias, w));
			}
		}
		0 => {
			let kind = if m.rd32(0x1404f4) == 2 { m.rd32(0x1404f8) } else { u32::MAX };
			if (kind == 3 || kind == 2) && m.rd16(0x141628) == 0 {
				out.push((at(0.015), f32::from_bits(0x3d69_78d5), bias, 0x2800_00c0));
			}
			for i in 0..(m.rd16(0x141200) as i16).max(0) as u32 {
				out.push((m.rdv3(0x141120 + 0x10 * i), m.rdf(0x1411c0 + 4 * i), m.rdf(0x1411e0 + 4 * i), m.rd32(0x1411a0 + 4 * i)));
			}
		}
		2 => out.push((at(-0.07), 0.25, bias, 0x3000_00c0)),
		1 => out.push((at(if m.rd8(0x15edb3) == 0 { -0.02 } else { -0.01 }), m.rdf(0x14097c), bias, m.rd32(0x141650))),
		_ => {}
	}
}

/// A pool colour word (R low byte, alpha on top) as 0..1 channels (0x80 = 1.0).
fn pool_rgba(w: u32) -> [f32; 4] {
	[(w & 0xff) as f32 / 128.0, (w >> 8 & 0xff) as f32 / 128.0, (w >> 16 & 0xff) as f32 / 128.0, (w >> 24) as f32 / 128.0]
}

/// The effect pool and draw-queue callbacks retail draws this field (`RetailFx`). Pool: every occupied slot up to the
/// high-water index gp-0x6b0c (inclusive: the alloc keeps the highest slot index, the draw walks index + 1 slots) with
/// the hidden flag (+1 bit 7) clear, by mode +1 & 3 (decomp/notes/particles.md "Draw size").
/// Dynamic light allocator FUN_002522c0 (Novalis; twins.py: binary twins on all 19 levels, Eudora FUN_00230c90) [C].
const LIGHT_ALLOC_FN: [u32; 19] = [
	0x23e320, 0x2522c0, 0x23fdf8, 0x22c580, 0x230c90, 0x2672d0, 0x24cf60, 0x2658f8, 0x247e98, 0x25d390, 0x230d18, 0x263658, 0x256498, 0x249520,
	0x245eb8, 0x22d8a8, 0x239960, 0x238998, 0x240590,
];
/// `LIGHT_ALLOC_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const LIGHT_ALLOC_FN_USA: [u32; 19] = [
	0x23e738, 0x252750, 0x240288, 0x22ca10, 0x231128, 0x267700, 0x24d3f8, 0x265d90, 0x248328, 0x25d828, 0x2311b0, 0x263ae8, 0x256928, 0x2499b8,
	0x2462c8, 0x22dd38, 0x239e10, 0x238e30, 0x240a20,
];

/// The 8 dynamic point-light slots (decomp/notes/lighting_fog.md 5b) as MobyProc reads them. Table 8 x 0x20: rgb
/// floats +0, intensity +0xc, position +0x10, radius +0x1c (Eudora 0x1803c0, Rilgar 0x180740, Novalis 0x1806c0: not
/// the camera-globals shift); its address comes from the allocator's own `lui v0 / addiu v0` at +0x78 / +0x80 (+0x30 / +0x38 load the state table, +0x200) (same
/// layout in the Eudora and Rilgar twins [C], the others [I]; a mismatch gives no lights). Eudora's moby pass
/// FUN_00248288 copies the slots whose position x/y doubleword is nonzero to SPR 0x2400 (free FUN_002523c0 zeroes a
/// record) and lights every moby from them (overlay.elf.c 67779-67806) [C]; `dynlight::moby_light` is the per-moby sum.
pub fn dyn_lights(m: &ee_oracle::Machine, level: u32) -> [Option<crate::particles::PointLight>; 8] {
	let mut out = [None; 8];
	let Some(&f) = (*crate::retail::disc::pick(&LIGHT_ALLOC_FN, &LIGHT_ALLOC_FN_USA)).get(level as usize) else { return out };
	let (hi, lo) = (m.rd32(f + 0x78), m.rd32(f + 0x80));
	if hi >> 16 != 0x3c02 || lo >> 16 != 0x2442 {
		return out;
	}
	let t = (hi << 16).wrapping_add(lo as i16 as i32 as u32);
	for (k, o) in out.iter_mut().enumerate() {
		let a = t + 0x20 * k as u32;
		if m.rd32(a + 0x10) | m.rd32(a + 0x14) != 0 {
			*o = Some(crate::particles::PointLight { pos: m.rdv3(a + 0x10), radius: m.rdf(a + 0x1c), rgb: [m.rdf(a), m.rdf(a + 4), m.rdf(a + 8)], intensity: m.rdf(a + 0xc) });
		}
	}
	out
}

/// Level-light slots 12..15 (decomp/notes/lighting_fog.md "Reserved level-light slots 12-14"), as the uniform rows
/// (colour A + wrap, to-light A = -dir A, colour B + wrap, to-light B). The loader copies at most 12 level lights; code
/// fills slot 12 (hero-attached / gold models), 13 (camera-yaw light, every frame) and 14 (menu and shop models: the
/// vendor's purchase prompt class 1137 has light word 0x0e0e) [C]. Records 0x40 each at the 16-slot table that ends
/// where the dynamic slots (`dyn_lights`) begin: Eudora 0x17ffc0 (slot 13 (0.8, 0.8, 0.8, -0.3) at 0x180300 and slot 14
/// (0.9, 0.9, 0.6, -0.25) at 0x180340 in s12 / fg1 4_0159 / s07 RAM), Novalis 0x1802c0 = 0x1806c0 - 0x400 [C]; the
/// other levels use the same layout [I].
pub fn reserved_lights(m: &ee_oracle::Machine, level: u32) -> Option<[[[f32; 4]; 4]; 4]> {
	let f = *(*crate::retail::disc::pick(&LIGHT_ALLOC_FN, &LIGHT_ALLOC_FN_USA)).get(level as usize)?;
	let (hi, lo) = (m.rd32(f + 0x78), m.rd32(f + 0x80));
	if hi >> 16 != 0x3c02 || lo >> 16 != 0x2442 {
		return None;
	}
	let t = (hi << 16).wrapping_add(lo as i16 as i32 as u32).wrapping_sub(0x400);
	Some(std::array::from_fn(|k| {
		let a = t + 0x40 * (12 + k as u32);
		let v = |o: u32, neg: f32| [m.rdf(a + o) * neg, m.rdf(a + o + 4) * neg, m.rdf(a + o + 8) * neg, if neg > 0.0 { m.rdf(a + o + 0xc) } else { 0.0 }];
		[v(0, 1.0), v(0x10, -1.0), v(0x20, 1.0), v(0x30, -1.0)]
	}))
}

/// Levels that run the Novalis patch-draw FUN_002b8af8 (binary twins, 230 lines each): (level, draw function, light-constants
/// delta from Novalis 0x1cac00, shift of the patch globals 0x161250 / 0x161258 / 0x1611a0). Novalis 0x2b8af8, Hoven 0x2bb5f8,
/// Oltanis Orbit 0x2af410, Pokitaru 0x2c7c68, Rilgar 0x2cd520, Umbris 0x2cba28 [C: read in fg1 states 1_0018, 12_0554, 13_0622, 11_0421, 5_0216, 7_0284].
const WATER_DRAW: [(u32, u32, i32, u32); 6] = [(1, 0x2b8af8, 0, 0), (12, 0x2bb5f8, 0, 0), (13, 0x2af410, -0x180, 0), (11, 0x2c7c68, 0x580, 0), (5, 0x2cd520, 0x80, 0x40), (7, 0x2cba28, -0x400, 0)];
/// `WATER_DRAW` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const WATER_DRAW_USA: [(u32, u32, i32, u32); 6] = [(1, 0x2b91c8, 0, 0), (12, 0x2bbcc8, 0, 0), (13, 0x2afae8, -0x180, 0), (11, 0x2c8338, 0x580, 0), (5, 0x2cdb90, 0x80, 0x40), (7, 0x2cc100, -0x400, 0)];

/// Retail's water patches (class 751 state) as the draw reads them [C]. Patch table pointer 0x161250, count 0x161258, output buffer
/// index 0x1611a0 (+ shift); record stride 0x1190: +0 pos, +0x14 / +0x18 fx textures (sphere / water pass), +0x1c / +0x1d ALPHA FIX,
/// +0x1e visible-subblock mask, +0x20 / +0x24 UV scroll, +0x30 / +0x34 wobble angles, +0x40 amplitude, +0x44 pin flags, three
/// 0x5c0-byte height buffers from +0x50 (the draw adds index * 0x5c0). The light vector sits at 0x1cac30 + the level delta.
fn water_patches(m: &ee_oracle::Machine, level: u32) -> Option<([f32; 3], Vec<crate::water::RamPatch>)> {
	let &(_, f, d, s) = (*crate::retail::disc::pick(&WATER_DRAW, &WATER_DRAW_USA)).iter().find(|e| e.0 == level)?;
	if [m.rd32(f), m.rd32(f + 4), m.rd32(f + 8)] != [0x27bd_fee0, 0x7fb2_0090, 0x7fbf_0100] {
		return None;
	}
	let table = m.rd32(va(0x161250 + s)) & 0x01ff_ffff;
	let count = m.rd32(va(0x161258 + s));
	let idx = m.rd32(va(0x1611a0 + s));
	if table < 0x10_0000 || count == 0 || count > 64 || idx > 2 {
		return None;
	}
	let light = m.rdv3(va(0x1cac30u32.wrapping_add_signed(d)));
	let mut out = Vec::new();
	for k in 0..count {
		let b = table + k * 0x1190;
		let mask = m.rd16(b + 0x1e);
		let mut patch = crate::water::Patch {
			pos: m.rdv3(b),
			nb: [0xff; 8],
			tex_sphere: (m.rd32(b + 0x14) as i32).max(0) as usize,
			tex_water: (m.rd32(b + 0x18) as i32).max(0) as usize,
			fix_sphere: m.rd8(b + 0x1c),
			fix_water: m.rd8(b + 0x1d),
			mask,
			scroll: [m.rdf(b + 0x20), m.rdf(b + 0x24)],
			speed: [0.0; 2],
			ang: [m.rdf(b + 0x30), m.rdf(b + 0x34)],
			ang_rate: [0.0; 2],
			amp: m.rdf(b + 0x40),
			pin: m.rd8(b + 0x44),
		};
		let mut grid = Vec::new();
		if mask != 0 {
			let g = b + 0x50 + idx * 0x5c0;
			let buf: Vec<f32> = (0..368).map(|i| m.rdf(g + 4 * i)).collect();
			grid = crate::water::grid_from_buffer(&buf);
		} else {
			patch.mask = 0;
		}
		out.push(crate::water::RamPatch { patch, grid });
	}
	Some((light, out))
}

fn queue_fx(m: &ee_oracle::Machine, level: u32) -> RetailFx {
	let mut out = RetailFx { frames: m.rd32(va(oldsim::FRAME_COUNTER)), ..RetailFx::default() };
	out.lights = dyn_lights(m, level);
	out.slots = reserved_lights(m, level);
	out.water = water_patches(m, level);
	out.vendors = vendor_glows(m, level);
	out.arcs = arc_draws(m, level);
	out.class_uv = asset_feed::class_uv(m, level);
	out.glints = oldsim::glints(m, level);
	out.sky_sprites = oldsim::sky_sprites(m, level);
	out.oceans = oldsim::oceans(m, level, out.frames);
	if let Some((g, bm)) = effect_pool(level) {
		let base = m.rd32(g) & 0x01ff_ffff;
		let hw = m.rd32(g + 8);
		if base != 0 && (hw as i32) >= 0 {
			for i in 0..=hw.min(0x7ff) {
				let s = base + 0x40 * i;
				let fl = m.rd8(s + 1);
				if m.rd8(bm + (i >> 3)) >> (i & 7) & 1 == 0 || fl & 0x80 != 0 {
					continue;
				}
				match fl & 3 {
					0 | 1 => {
						out.sprites.push(crate::particles::retail_pool_sprite(m.rdv3(s + 0x10), m.rdf(s + 0xc), m.rd8(s + 8), m.rd32(s + 4), m.rd8(s + 2), m.rd8(s + 3), m.rd8(s + 9), fl));
						out.sprite_slots.push(i as u16);
					}
					2 => {
						let b9 = m.rd8(s + 9);
						out.line_slots.push(i as u16);
						out.lines.push(crate::particles::Line {
							a: m.rdv3(s + 0x10),
							b: m.rdv3(s + 0x20),
							col_a: pool_rgba(m.rd32(s + 4)),
							col_b: pool_rgba(m.rd32(s + 0xc)),
							near: (b9 & 15) as f32 / 4.0,
							far: crate::water::pool_far(b9, f32::MAX),
						});
					}
					_ => out.ribbons.push(PoolRibbon {
						head: m.rdv3(s + 0x10),
						tail: m.rdv3(s + 0x20),
						half: m.rdf(s + 0x1c),
						scale: m.rdf(s + 0x2c),
						c_head: m.rd32(s + 4),
						c_tail: m.rd32(s + 0xc),
						tex: m.rd8(s + 2),
						additive: m.rd8(s + 3) != 0x44,
						b9: m.rd8(s + 9),
						slot: i as u16,
					}),
				}
			}
		}
	}
	out.blobs = blob_queue(m, level);
	out.hero = Some(m.rdv3(0x13f3d0));
	if let Some((fns, args, _)) = orb_queue(level) {
		let li = level as usize;
		let mut glow_done = false;
		for i in 0..m.rd32(0x15f424).min(0x40) {
			let f = m.rd32(fns + 4 * i);
			if f == (*crate::retail::disc::pick(&GLOW_FN, &GLOW_FN_USA))[li] && !glow_done {
				glow_done = true;
				hero_glows(m, &mut out.glows);
			} else if (*crate::retail::disc::pick(&RETICLE_FN, &RETICLE_FN_USA))[li].contains(&f) || (f != 0 && (*crate::retail::disc::pick(&MINE_RETICLE_FN, &MINE_RETICLE_FN_USA))[li] == f) {
				let pv = m.rd32((m.rd32(args + 4 * i) & 0x01ff_ffff) + 0x78) & 0x01ff_ffff;
				if pv != 0 {
					out.reticles.push(crate::game::Reticle { pos: m.rdv3(pv + 0x10), normal: m.rdv3(pv + 0x20) });
				}
			} else if (*crate::retail::disc::pick(&MARKER203_FN, &MARKER203_FN_USA))[li] == f {
				let pv = m.rd32((m.rd32(args + 4 * i) & 0x01ff_ffff) + 0x78) & 0x01ff_ffff;
				if pv != 0 {
					out.reticles.push(crate::game::Reticle { pos: m.rdv3(pv + 0x20), normal: m.rdv3(pv + 0x30) });
				}
			} else if (*crate::retail::disc::pick(&SHIP_SHADOW_FN, &SHIP_SHADOW_FN_USA))[li] == f {
				out.ship_shadows.extend(ship_shadow(m, level, m.rd32(args + 4 * i) & 0x01ff_ffff));
			}
		}
	}
	queue_b::fill(m, level, &mut out.qa, &mut out.qb, &mut out.rope);
	queue_fxa::fleet_water(m, level, &mut out.qw);
	queue_q1::fill(m, level, &mut out.glows, &mut out.strips, &mut out.pools);
	out.ocean = queue_q1::ocean(m, level);
	out.screen = queue_b::screen_filter(m, level);
	out.swap = SwapWords::read(m, level);
	out.rc = queue_b::rc_static(m, level);
	out.shrub_cap = Some(m.rdf(if matches!(level, 0 | 5 | 14 | 16) { va(0x160464) } else { va(0x160424) }));
	out
}

/// One moby of the vendor's 0x100-stride slot block (or its portrait actor): not in the moby table, so `snap` never sees it.
pub struct ShopMoby {
	pub class: u32,
	pub pos: [f32; 3],
	pub rot: [f32; 3],
	/// Scale over the class header's base scale (as `Moby::scale`).
	pub scale: f32,
	pub clock: Clock,
}

/// What the vendor screen draws with VU1 besides the world (vendor.md REN-9 / NOV-30a / NOV-30d / BRIDGE-37) [C].
pub struct ShopSnap {
	/// The world item over the pad: B (slot +0x200, ammo row world model) or C (slot +0x300, the hologram item).
	pub world: Option<ShopMoby>,
	/// Panel 1 (item target): A (slot +0x100) and the slot-0 class-13 bolt icon.
	pub panel: Option<ShopMoby>,
	pub icon: Option<ShopMoby>,
	/// Panel 2 (portrait target): the class-12 vendor actor.
	pub portrait: Option<ShopMoby>,
	/// The hologram beam over the pad (`shop_beam`), empty while the shop's alt flag is set.
	pub beam: Vec<crate::particles::WorldQuad>,
	/// The purchase prompt moby (slot +0x400, class 1137) while the prompt is up (base + 0x5c != 0): FUN_00290ad0
	/// OV:118067-118069 draws it with FUN_002904f0, which renders the popup target, draws the moby into the frame
	/// (FUN_002427d0(slot, 1), VU1, so absent from the GS capture) and blits the target over it (GS, in the overlay).
	/// `snap` adds it to the moby list, so `sync` mirrors it as an ordinary prop.
	pub prompt: Option<u32>,
	/// The popup target's class-13 bolt (slot +0x500): FUN_0028f2f8 OV:117284-117298 draws it on an ammo row that is not
	/// full (max u16 price +0xe > owned 0x13d428 + item * 4) when the bolts 0x15ed98 reach the unit price (u16 +8, +0xa
	/// with the alt flag); price table base - 0x6410 + item * 0x18 on every level (twins' lui/addiu, round 8) [C].
	pub popup_bolt: Option<ShopMoby>,
}

/// Per-level globals of the shop screen. Eudora [C]: FUN_00290ad0 (= Novalis FUN_002b3950) OV:117937-118082 keeps the
/// screen state in 0x1ca548 (1 = up), the slot-0 pointer in 0x1ca560 (slot block base + 0x100) and the portrait actor in
/// 0x1ca564; the world draw (OV:118051-118060) runs when 0x161f38 == 0 && 0x1ca580 == 0: row = 0x1ca598, the row
/// records (stride 20) at 0x1ca610 (item) / 0x1ca614 (kind): kind 1 draws B when the item's 0x1c90a0 + item * 0x40 word
/// is set, any other kind draws C. Panel 1 FUN_0028e9b8 draws slot +0x100 and slot 0 (OV:117000-117001); panel 2
/// FUN_0028e608 draws the portrait when 0x161108 == 1 (OV:116852-116853). Other levels [C]: the twins of FUN_00290ad0 /
/// FUN_0028e608 (twins.py --all) are the same code; the block bases (Eudora 0x1ca540 in a0, ammo 0x1c9070, 0x161f38, 0x161108) are
/// read from each twin's lui/addiu pairs and the fields keep Eudora's offsets from them (scratch_m3b round 7).
struct ShopAddrs {
	state: u32,
	slots: u32,
	portrait: u32,
	gate: u32,
	alt: u32,
	row: u32,
	rows: u32,
	ammo: u32,
	portrait_on: u32,
}

fn shop_addrs(level: u32) -> Option<ShopAddrs> {
	match level {
		0 => Some(ShopAddrs { state: va(0x1ca448), slots: va(0x1ca460), portrait: va(0x1ca464), gate: va(0x161e38), alt: va(0x1ca480), row: va(0x1ca498), rows: va(0x1ca510), ammo: va(0x1c8fa0), portrait_on: va(0x161148) }), // veldin
		1 => Some(ShopAddrs { state: va(0x1ca848), slots: va(0x1ca860), portrait: va(0x1ca864), gate: va(0x162238), alt: va(0x1ca880), row: va(0x1ca898), rows: va(0x1ca910), ammo: va(0x1c93a0), portrait_on: va(0x161108) }), // novalis
		2 => Some(ShopAddrs { state: va(0x1ca9c8), slots: va(0x1ca9e0), portrait: va(0x1ca9e4), gate: va(0x1623b8), alt: va(0x1caa00), row: va(0x1caa18), rows: va(0x1caa90), ammo: va(0x1c9520), portrait_on: va(0x161108) }), // aridia
		3 => Some(ShopAddrs { state: va(0x1ca4c8), slots: va(0x1ca4e0), portrait: va(0x1ca4e4), gate: va(0x161eb8), alt: va(0x1ca500), row: va(0x1ca518), rows: va(0x1ca590), ammo: va(0x1c9020), portrait_on: va(0x161108) }), // kerwan
		4 => Some(ShopAddrs { state: va(0x1ca548), slots: va(0x1ca560), portrait: va(0x1ca564), gate: va(0x161f38), alt: va(0x1ca580), row: va(0x1ca598), rows: va(0x1ca610), ammo: va(0x1c90a0), portrait_on: va(0x161108) }), // eudora
		5 => Some(ShopAddrs { state: va(0x1ca8c8), slots: va(0x1ca8e0), portrait: va(0x1ca8e4), gate: va(0x162238), alt: va(0x1ca900), row: va(0x1ca918), rows: va(0x1ca990), ammo: va(0x1c9420), portrait_on: va(0x161148) }), // rilgar
		6 => Some(ShopAddrs { state: va(0x1cab48), slots: va(0x1cab60), portrait: va(0x1cab64), gate: va(0x162538), alt: va(0x1cab80), row: va(0x1cab98), rows: va(0x1cac10), ammo: va(0x1c96a0), portrait_on: va(0x161108) }), // nebula
		7 => Some(ShopAddrs { state: va(0x1ca448), slots: va(0x1ca460), portrait: va(0x1ca464), gate: va(0x161e38), alt: va(0x1ca480), row: va(0x1ca498), rows: va(0x1ca510), ammo: va(0x1c8fa0), portrait_on: va(0x161108) }), // umbris
		8 => Some(ShopAddrs { state: va(0x1cabc8), slots: va(0x1cabe0), portrait: va(0x1cabe4), gate: va(0x1625b8), alt: va(0x1cac00), row: va(0x1cac18), rows: va(0x1cac90), ammo: va(0x1c9720), portrait_on: va(0x161108) }), // batalia
		9 => Some(ShopAddrs { state: va(0x1ca548), slots: va(0x1ca560), portrait: va(0x1ca564), gate: va(0x161f38), alt: va(0x1ca580), row: va(0x1ca598), rows: va(0x1ca610), ammo: va(0x1c90a0), portrait_on: va(0x161108) }), // gaspar
		10 => Some(ShopAddrs { state: va(0x1ca848), slots: va(0x1ca860), portrait: va(0x1ca864), gate: va(0x162238), alt: va(0x1ca880), row: va(0x1ca898), rows: va(0x1ca910), ammo: va(0x1c93a0), portrait_on: va(0x161108) }), // orxon
		11 => Some(ShopAddrs { state: va(0x1cadc8), slots: va(0x1cade0), portrait: va(0x1cade4), gate: va(0x1627b8), alt: va(0x1cae00), row: va(0x1cae18), rows: va(0x1cae90), ammo: va(0x1c9920), portrait_on: va(0x161108) }), // pokitaru
		12 => Some(ShopAddrs { state: va(0x1ca848), slots: va(0x1ca860), portrait: va(0x1ca864), gate: va(0x162238), alt: va(0x1ca880), row: va(0x1ca898), rows: va(0x1ca910), ammo: va(0x1c93a0), portrait_on: va(0x161108) }), // hoven
		13 => Some(ShopAddrs { state: va(0x1ca6c8), slots: va(0x1ca6e0), portrait: va(0x1ca6e4), gate: va(0x1620b8), alt: va(0x1ca700), row: va(0x1ca718), rows: va(0x1ca790), ammo: va(0x1c9220), portrait_on: va(0x161108) }), // oltanis_orbit
		14 => Some(ShopAddrs { state: va(0x1cab48), slots: va(0x1cab60), portrait: va(0x1cab64), gate: va(0x162538), alt: va(0x1cab80), row: va(0x1cab98), rows: va(0x1cac10), ammo: va(0x1c96a0), portrait_on: va(0x161148) }), // oltanis
		15 => Some(ShopAddrs { state: va(0x1ca9c8), slots: va(0x1ca9e0), portrait: va(0x1ca9e4), gate: va(0x1623b8), alt: va(0x1caa00), row: va(0x1caa18), rows: va(0x1caa90), ammo: va(0x1c9520), portrait_on: va(0x161108) }), // quartu
		16 => Some(ShopAddrs { state: va(0x1ca848), slots: va(0x1ca860), portrait: va(0x1ca864), gate: va(0x1621b8), alt: va(0x1ca880), row: va(0x1ca898), rows: va(0x1ca910), ammo: va(0x1c93a0), portrait_on: va(0x161148) }), // kalebo
		17 => Some(ShopAddrs { state: va(0x1cacc8), slots: va(0x1cace0), portrait: va(0x1cace4), gate: va(0x1626b8), alt: va(0x1cad00), row: va(0x1cad18), rows: va(0x1cad90), ammo: va(0x1c9820), portrait_on: va(0x161108) }), // veldin_orbit
		18 => Some(ShopAddrs { state: va(0x1cadc8), slots: va(0x1cade0), portrait: va(0x1cade4), gate: va(0x1627b8), alt: va(0x1cae00), row: va(0x1cae18), rows: va(0x1cae90), ammo: va(0x1c9920), portrait_on: va(0x161108) }), // veldin_finale
		_ => None,
	}
}

fn shop_moby(m: &ee_oracle::Machine, a: u32) -> Option<ShopMoby> {
	let a = a & 0x01ff_ffff;
	let hdr = m.rd32(a + 0x24) & 0x01ff_ffff;
	if a == 0 || hdr == 0 {
		return None;
	}
	let base = f32::from_bits(m.rd32(hdr + 0x24));
	Some(ShopMoby {
		class: m.rd16(a + 0xa6) as u32,
		pos: m.rdv3(a + 0x10),
		rot: basis_euler(m, a),
		scale: if base != 0.0 { m.rdf(a + 0x2c) / base } else { 1.0 },
		clock: Clock::read(m, a),
	})
}

/// The shop screen's VU1 mobys of this field (`ShopAddrs`), None while the screen is not up.
fn shop_snap(m: &ee_oracle::Machine, level: u32) -> Option<ShopSnap> {
	let s = shop_addrs(level)?;
	if m.rd32(s.state) != 1 {
		return None;
	}
	let slot0 = m.rd32(s.slots) & 0x01ff_ffff;
	if slot0 == 0 {
		return None;
	}
	let world = (m.rd32(s.gate) == 0 && m.rd32(s.alt) == 0).then(|| {
		let r = s.rows + m.rd32(s.row).wrapping_mul(20);
		if m.rd32(r + 4) == 1 {
			(m.rd32(s.ammo + m.rd32(r).wrapping_mul(0x40)) != 0).then_some(slot0 + 0x200)
		} else {
			Some(slot0 + 0x300)
		}
	});
	Some(ShopSnap {
		world: world.flatten().and_then(|a| shop_moby(m, a)),
		panel: shop_moby(m, slot0 + 0x100),
		icon: shop_moby(m, slot0),
		portrait: (m.rd32(s.portrait_on) == 1).then(|| shop_moby(m, m.rd32(s.portrait))).flatten(),
		beam: if m.rd32(s.alt) == 0 { shop_beam(m, s.state - 8, s.portrait_on - 4) } else { Vec::new() },
		prompt: (m.rd32(s.state + 0x54) != 0).then_some(slot0 + 0x400),
		popup_bolt: (m.rd32(s.state + 0x54) != 0 && popup_bolt_drawn(m, &s)).then(|| shop_moby(m, slot0 + 0x500)).flatten(),
	})
}

/// Whether the popup content FUN_0028f2f8 draws the slot +0x500 bolt this field (`ShopSnap::popup_bolt`) [C].
fn popup_bolt_drawn(m: &ee_oracle::Machine, s: &ShopAddrs) -> bool {
	let r = s.rows + m.rd32(s.row).wrapping_mul(20);
	if m.rd32(r + 4) != 1 {
		return false;
	}
	let item = m.rd32(r);
	let price = (s.state - 8 - 0x6410).wrapping_add(item.wrapping_mul(0x18));
	let owned = m.rd32(0x13d428u32.wrapping_add(item.wrapping_mul(4))) as i32;
	let unit = m.rd16(price + if m.rd32(s.alt) == 0 { 8 } else { 0xa }) as i32;
	(m.rd16(price + 0xe) as i32) > owned && m.rd32(0x15ed98) as i32 >= unit
}

/// The shop's hologram beam over the pad: Eudora FUN_00290778 (OV:117766-117936), called by the shop draw FUN_00290ad0
/// (OV:118062-118064) while the alt flag (base + 0x40) is 0 [C]. Four strip quads (FUN_001ef7c0) whose vertex indices
/// are 4 i32 each at base - 0x40; vertex i: vendor-local position base - 0x140 + i * 0x10, placed by the vendor moby
/// (pointer base + 0x1c) as pos + x * M0 + y * M1 + z * M2 (rows +0xc0 / +0xd0 / +0xe0, FUN_001f1c68); UV at
/// base - 0xb0 + i * 8 with V + the scroll at gp-0x5afc (+0.01 per field, wraps at 1, added before the draw, so the
/// value after the field is the drawn one); rgba word base - 0x68 + i * 4; fx texture 0x18, ALPHA 0x44 (alpha-over).
/// Base = the shop state - 8 and the scroll = portrait_on - 4 on every level: each twin's lui/addiu pairs give
/// base - 0x140 / - 0xb0 / - 0x68 / - 0x40 and gp-0x5afc (gp-0x5abc on levels 0, 5, 14, 16) (scratch_m3b round 8).
fn shop_beam(m: &ee_oracle::Machine, base: u32, scroll: u32) -> Vec<crate::particles::WorldQuad> {
	let v = m.rd32(base + 0x1c) & 0x01ff_ffff;
	if v == 0 {
		return Vec::new();
	}
	let pos = m.rdv3(v + 0x10);
	let rows = [0, 1, 2].map(|r| m.rdv3(v + 0xc0 + 0x10 * r));
	let sc = m.rdf(scroll);
	let vert = |i: u32| {
		let l = m.rdv3(base - 0x140 + i * 0x10);
		let p: [f32; 3] = std::array::from_fn(|a| pos[a] + l[0] * rows[0][a] + l[1] * rows[1][a] + l[2] * rows[2][a]);
		(p, [m.rdf(base - 0xb0 + i * 8), m.rdf(base - 0xb0 + i * 8 + 4) + sc], pool_rgba(m.rd32(base - 0x68 + i * 4)))
	};
	(0..4u32)
		.filter_map(|q| {
			let idx = [0, 1, 2, 3].map(|k| m.rd32(base - 0x40 + q * 0x10 + k * 4));
			if idx.iter().any(|&i| i > 8) {
				return None;
			}
			// Strip order (v0, v1, v2, v3) -> fan order (v0, v1, v3, v2).
			let [a, b, c, d] = idx.map(vert);
			Some(crate::particles::WorldQuad { pos: [a.0, b.0, d.0, c.0], uv: [a.1, b.1, d.1, c.1], colour: [a.2, b.2, d.2, c.2], tex: 0x18, additive: false })
		})
		.collect()
}





/// The draw-queue arrays (count 0x15f424, fn[], arg[]) of `level`, for the audit.
pub fn draw_queue(level: u32) -> Option<(u32, u32)> {
	orb_queue(level).map(|q| (q.0, q.1))
}

/// A mode-3 pool slot's quad (FUN_00259e18 mode 3, particles.md "Draw size"): corners head +- off, tail +- off * scale,
/// off = the unit cross(midpoint - eye, tail - head) * half-width; U across, V along (head 0, tail 1). Fan order.
/// `fade` scales both alphas: the packet build multiplies bytes +7 / +0xf by min(key - near, (far - key) / 16, 1) of the
/// midpoint depth key, as the sprite modes (overlay.elf.c 82318-82380: pminw of (far - key) >> 4, key - near * 0x400 and
/// 0x1000, pmultuw, >> 12) [C].
pub fn pool_ribbon_quad(r: &PoolRibbon, eye: [f32; 3], fade: f32) -> crate::particles::WorldQuad {
	let (a, b) = (r.head, r.tail);
	let d: [f32; 3] = std::array::from_fn(|i| b[i] - a[i]);
	let v: [f32; 3] = std::array::from_fn(|i| (a[i] + b[i]) * 0.5 - eye[i]);
	let x = [v[1] * d[2] - v[2] * d[1], v[2] * d[0] - v[0] * d[2], v[0] * d[1] - v[1] * d[0]];
	let l = (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
	let off: [f32; 3] = if l > 0.0 { std::array::from_fn(|i| x[i] / l * r.half) } else { [0.0; 3] };
	let p = |o: [f32; 3], s: f32| -> [f32; 3] { std::array::from_fn(|i| o[i] + off[i] * s) };
	let (mut c0, mut c1) = (pool_rgba(r.c_head), pool_rgba(r.c_tail));
	c0[3] *= fade;
	c1[3] *= fade;
	crate::particles::WorldQuad {
		pos: [p(a, -1.0), p(a, 1.0), p(b, r.scale), p(b, -r.scale)],
		uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
		colour: [c0, c0, c1, c1],
		tex: (crate::particles::PART_TEX_BASE + r.tex as u32) as usize,
		additive: r.additive,
	}
}

/// The new level after travel is up: gameplay (mode 0), an in-engine scene (mode 2), or the landing stream (mode 6
/// substate 8) once the world draws again (draw mask 0x15f3f4 nonzero; it is 0 while the level loads behind the warp
/// [C: departure 0164 log 2026-10-12]). The pools are counted then, so arrival scenes draw live on the new level.
fn arrived(s: &Snap, m: &ee_oracle::Machine) -> bool {
	matches!(s.mode, 0 | 2) || (s.mode == 6 && s.ship == 8 && m.rd32(0x15f3f4) != 0)
}

pub fn snap(m: &ee_oracle::Machine) -> Snap {
	let level = m.level();
	let mode = m.rd32(va(0x15f5a4));
	let mut out = Snap {
		health: m.rd32(0x1415f8) as i32,
		bolts: m.rd32(0x15ed98),
		mode,
		level,
		have: (0..crate::inventory::GADGETS as u32).map(|i| m.rd8(0x13d4c0 + i) != 0).collect(),
		ammo: (0..crate::inventory::GADGETS as u32).map(|i| m.rd32(0x13d428 + 4 * i).min(0xffff) as u16).collect(),
		equipped: m.rd32(0x140408) as u8,
		last: m.rd32(0x141660) as u8,
		pad: [m.rd32(0x13cae0), m.rd32(0x13cae4), m.rd32(0x13cb00), m.rd32(0x13cb04)],
		..Snap::default()
	};
	let ship = if mode == 6 { m.rd32(SHIP_STATE) } else { 0 };
	out.ship_fight = shipmouse::in_ship_fight(m);
	(out.frozen, out.scene, out.ship) = (frozen_view(mode, ship), scene_view(mode, ship), ship);
	out.no_present = m.rd32(va(NO_PRESENT)) != 0;
	out.hold = out.frozen || out.no_present;
	let c = crate::gsfull::crtc::Crtc::from_regs(&m.cpu.mem.gs);
	out.crtc = (c.pmode, c.dispfb[0], c.bgcolor);
	out.screen = (m.rd32(va(NO_PRESENT)), m.rd32(0x15f3f4), m.rdf(0x15f3fc), m.rdf(0x15f400), m.rd32(va(0x16a134)));
	out.warp = warp::read(m, mode, level);
	out.menu = mouse_menu::read(&|a| m.rd32(a), level, mode);
	prompts::set_menu(out.menu.is_some());
	menu_keys::publish(out.menu.as_ref().map(menu_keys::screen));
	// A mode-4 dialog is no page menu but its keys and prompts work the menu way (menu_keys::dialog).
	out.dialog = menu_keys::dialog_on(&|a| m.rd32(a), level, mode);
	if out.menu.is_none() && out.dialog.is_some() {
		prompts::set_menu(true);
		menu_keys::publish(out.dialog);
	}
	if mode == 6 && matches!(ship, 0 | 8) {
		if LEVEL_SHIFT.get(level as usize).is_some() {
			let cl = crate::game::transit::warp_ship_class((m.rd8(0x13e056) as usize).min(2));
			out.ship_gloss = queue_story::gloss_for(m, level, cl, 0x15);
			if std::env::var_os("RAC_SCENE_LOG").is_some() {
				let n = out.ship_gloss.len() as f32 * 4.0;
				let c = out.ship_gloss.iter().flat_map(|q| q.pos).fold([0.0f32; 3], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n, a[2] + p[2] / n]);
				eprintln!("retail: ship gloss level {level} ship {ship} class {cl} quads {} centroid {c:?} colour {:?}", out.ship_gloss.len(), out.ship_gloss.first().map(|q| q.colour[0]));
				let (lo, hi) = out.ship_gloss.iter().flat_map(|q| q.pos).fold(([f32::MAX; 3], [f32::MIN; 3]), |(l, h), p| ([l[0].min(p[0]), l[1].min(p[1]), l[2].min(p[2])], [h[0].max(p[0]), h[1].max(p[1]), h[2].max(p[2])]));
				eprintln!("retail: ship gloss bbox {lo:?} {hi:?} q0 {:?}", out.ship_gloss.first().map(|q| (q.pos, q.uv)));
			}
		}
	}
	// Mode 6: the ship thruster flames (queue A callback FUN_0027f7e8, retail/warp.rs `flames`) ride with the gloss quads.
	if mode == 6 {
		out.ship_gloss.extend(warp::flames(m, level));
	}
	// Cutscenes and mode 0: the queued ship's gloss (queue_story.rs).
	if mode != 6 && !out.hold {
		out.ship_gloss = queue_story::cutscene_gloss(m, level);
	}
	// Where retail draws no world (pause menu, movie, level load) the main thread keeps the last frame; the table may be
	// mid-load.
	if out.hold {
		return out;
	}
	out.cam = read_cam(m, level);
	out.cam_up = read_cam_up(m, level);
	out.orbs = Some(orb_draws(m, level));
	out.fx = Some(queue_fx(m, level));
	out.shop = shop_snap(m, level);
	// The vendor's shop is no page menu but its keys and prompts work the menu way (menu_keys::VENDOR).
	if out.menu.is_none() && out.shop.is_some() {
		prompts::set_menu(true);
		menu_keys::publish(Some(menu_keys::VENDOR));
	}
	let hp = m.rd32(HELD_MOBY) & 0x01ff_ffff;
	let view = DrawView::read(m, level);
	out.occ_mask = view.as_ref().map(|v| v.mask);
	let (s, e) = table(level);
	let (s, e) = (m.rd32(s) & 0x01ff_ffff, m.rd32(e) & 0x01ff_ffff);
	let mut a = s;
	let mut walked = true;
	let mut hero_addr = 0;
	while a < e {
		let hdr = m.rd32(a + 0x24);
		let class = m.rd16(a + 0xa6) as u32;
		let slot = m.rd8(a + 0x20);
		// Retail's walk (0x248600, 0x248f98) skips slots with a negative +0x20 and stops at the first 0xff.
		walked &= slot != 0xff;
		if hdr != 0 && slot < 0xfd {
			if out.scene || mode == VENDOR_MODE {
				if let Some(b) = scene_seq(m, a, class) {
					out.scene_seqs.push((class, m.rd8(a + 0x53), b));
				}
			}
			if class == 0 {
				// A scene spawns its own class-0 Ratchet and hides the placed one (+0x34 bit 0): the drawn one wins.
				let shown = m.rd8(a + 0x31) != 0;
				// The hero's afterimage copies (FUN_00276fc0, the only writer of flags 0x80a: a copy of the hero class with +0x34 = 0x80a and
				// a translucent alpha +0x23, one per trail entry, 2 / 4 / 6 fields behind in Thruster-Pack flight, state 0x10) are class-0
				// mobys of their own and sit after the hero in the table: taking them for the hero drew Ratchet (pose, joints, position)
				// up to 6 fields behind the true place while Clank and the packs stayed there.
				let afterimage = m.rd16(a + 0x34) == 0x80a && a != m.rd32(0x1413d0) & 0x01ff_ffff;
				if !afterimage && !(out.hero_shown && !shown) {
					out.hero = Some((m.rdv3(a + 0x10), m.rdf(a + 0x48), Clock::read(m, a)));
					out.hero_hidden = m.rd16(a + 0x34) & 1 != 0;
					out.parts.hero_fx = joint_records(m, a);
					out.parts.strafe = strafe::current();
					hero_addr = a;
					out.parts.layers = layer_list(m, a);
					out.parts.from_frame = snapshot_frame(m, a);
					out.hero_tilt = [m.rdf(a + 0x40), m.rdf(a + 0x44)];
					out.hero_shown = shown;
				}
			} else {
				let base = f32::from_bits(m.rd32((hdr & 0x01ff_ffff) + 0x24));
				let shown = m.rd8(a + 0x31) != 0;
				let side_only = view.as_ref().filter(|_| !shown && walked && (slot as i8) >= 0).and_then(|v| {
					let (culls, side) = v.culls(m, a);
					culls.side_only().then_some(side)
				});
				let (rot, mirror) = basis_pose(m, a, true);
				out.mobys.push(Moby {
					addr: a,
					class,
					pos: m.rdv3(a + 0x10),
					rot,
					mirror,
					scale: if base != 0.0 { m.rdf(a + 0x2c) / base } else { 1.0 },
					shown,
					side_only,
					#[cfg(test)]
					culls: view.as_ref().map(|v| v.culls(m, a).0),
					#[cfg(test)]
					raw: (m.rd16(a + 0x34), m.rd16(a + 0x32) as i16, m.rd32(if matches!(level, 0 | 5 | 14 | 16) { va(0x15ffb0) } else { va(0x15ff70) }) as i32),
					rgb: [m.rd8(a + 0x3c), m.rd8(a + 0x3d), m.rd8(a + 0x3e)],
					light: m.rd32(a + 0x38),
					clock: Clock::read(m, a),
					fx: joint_records(m, a),
					alpha: m.rd8(a + 0x23),
					draw: m.rd16(a + 0x32) as i16,
					ovr: (m.rd16(a + 0x34) & 0x10 != 0).then(|| m.rd32(a + 0x90)),
				});
			}
		}
		a += 0x100;
	}
	// The gadget cinematic after a purchase runs in the vendor's mode 5 and plays its actors (Ratchet and the bought
	// gadget's own moby) through the same chunk player as a scene: it is a scene while a moby plays a scene sequence.
	if mode == VENDOR_MODE && !out.scene_seqs.is_empty() {
		out.scene = true;
	}
	// The shop's purchase prompt (`ShopSnap::prompt`): a slot-block moby outside the table, always drawn while up.
	if let Some(a) = out.shop.as_ref().and_then(|sh| sh.prompt).filter(|&a| m.rd32(a + 0x24) != 0) {
		let hdr = m.rd32(a + 0x24) & 0x01ff_ffff;
		let base = f32::from_bits(m.rd32(hdr + 0x24));
		let (rot, mirror) = basis_pose(m, a, true);
		out.mobys.push(Moby {
			addr: a,
			class: m.rd16(a + 0xa6) as u32,
			pos: m.rdv3(a + 0x10),
			rot,
			mirror,
			scale: if base != 0.0 { m.rdf(a + 0x2c) / base } else { 1.0 },
			shown: true,
			side_only: None,
			#[cfg(test)]
			culls: None,
			#[cfg(test)]
			raw: (m.rd16(a + 0x34), m.rd16(a + 0x32) as i16, 0),
			rgb: [m.rd8(a + 0x3c), m.rd8(a + 0x3d), m.rd8(a + 0x3e)],
			light: m.rd32(a + 0x38),
			clock: Clock::read(m, a),
			fx: joint_records(m, a),
			alpha: m.rd8(a + 0x23),
			draw: m.rd16(a + 0x32) as i16,
			ovr: (m.rd16(a + 0x34) & 0x10 != 0).then(|| m.rd32(a + 0x90)),
		});
	}
	// In a scene with actors the held item belongs to the hidden placed hero, not to the scene's Ratchet.
	if out.scene_seqs.is_empty() && hp >= s && hp < e && (hp - s) % 0x100 == 0 && m.rd32(hp + 0x24) != 0 && m.rd8(hp + 0x20) < 0xfd {
		out.held = Some((hp, m.rd16(hp + 0xa6) as u32, Clock::read(m, hp)));
	}
	if let Some((hp, crate::mobies::WRENCH_CLASS, clock)) = out.held {
		out.parts.wrench = Some(HeldItem { addr: hp, clock, attached: m.rd16(hp + 0x34) & 4 != 0 && m.rd8(0x1403fa) == 0 });
	}
	let back = |p: u32| Some(m.rd32(p) & 0x01ff_ffff).filter(|&b| b >= s && b < e && (b - s) % 0x100 == 0 && m.rd32(b + 0x24) != 0 && m.rd8(b + 0x20) < 0xfd);
	out.parts.d0 = back(0x1404d0).map(|b| Clock::read(m, b));
	if let Some(b) = back(0x1404d4) {
		out.parts.d4 = Some(Clock::read(m, b));
		out.parts.clank_fx = joint_records(m, b);
	}
	let in_hand = out.parts.wrench.filter(|w| w.attached).map(|w| w.addr);
	out.parts.loose = out
		.mobys
		.iter()
		.filter(|mb| mb.class == crate::mobies::WRENCH_CLASS && Some(mb.addr) != in_hand && (m.rd8(mb.addr + 0x20) as i8) >= 0)
		.map(|mb| {
			let base = f32::from_bits(m.rd32((m.rd32(mb.addr + 0x24) & 0x01ff_ffff) + 0x24));
			LooseWrench {
				addr: mb.addr,
				pos: mb.pos,
				rows: [m.rdv3(mb.addr + 0xc0), m.rdv3(mb.addr + 0xd0), m.rdv3(mb.addr + 0xe0)],
				scale: if base != 0.0 { m.rdf(mb.addr + 0x2c) / base } else { 1.0 },
				alpha: m.rd8(mb.addr + 0x23),
				clock: mb.clock,
				shown: mb.shown,
				side_only: mb.side_only,
			}
		})
		.collect();
	// Modern camera: Clank, the back object and the bomb in the glove follow the drawn yaw (strafe.rs `turn_parts`, draw only).
	if hero_addr != 0 && modcam::enabled() {
		let mut glows = out.fx.as_mut().map(|fx| std::mem::take(&mut fx.glows)).unwrap_or_default();
		strafe::turn_parts(m, hero_addr, hp, &mut out.mobys, &mut glows);
		if let Some(fx) = out.fx.as_mut() {
			fx.glows = glows;
		}
	}
	out.shadows = Some(shadow_snap(m, level));
	// [PC] Water Enhanced wakes (viewer/waterfx.rs `Wakes`): where the drawn mobys stand this field.
	if let Some(fx) = out.fx.as_mut() {
		// The bounding sphere is moby +0x00..0x0c, x1024 (decomp/notes MOBY-FADE): its radius at +0x0c.
		fx.movers = out.mobys.iter().filter(|mb| mb.shown).map(|mb| (mb.addr, mb.pos, m.rdf(mb.addr + 0xc) / 1024.0)).collect();
	}
	out
}

/// M3a shadows: retail's moby shadow volumes of the last drawn frame (decomp/notes/shadows.md "Moby shadow volumes").
#[derive(Clone, Debug, Default)]
pub struct RetailShadows {
	/// (caster moby, its record) in caster-list order.
	pub casters: Vec<(u32, crate::shadows::RecCaster)>,
	/// Mobys on retail's deferred list (the casters and the drawn +0x34 & 0x800 mobys), drawn after the volumes and the darken.
	pub deferred: Vec<u32>,
	/// The hero moby is on the deferred list (a caster, or +0x34 & 0x800).
	pub hero_deferred: bool,
}

/// [PC] Frame smoothing (viewer/interp.rs): the draw lists of a display frame between two fields (`Snap::smooth`); None = the
/// field's own.
#[derive(Default)]
pub struct SmoothDraw {
	pub fx: Option<RetailFx>,
	pub shadows: Option<RetailShadows>,
	pub orbs: Option<Vec<crate::particles::OrbDraw>>,
}

impl RetailShadows {
	fn push_deferred(&mut self, moby: u32, hero: bool) {
		if hero {
			self.hero_deferred = true;
		} else if !self.deferred.contains(&moby) {
			self.deferred.push(moby);
		}
	}
}

/// The shadow caster list of each level (8 B: moby, fade u16, straddle u16; the records follow at + 0x180): the
/// immediate the record build loads (Eudora FUN_002787e0 0x1ac700, Blarg FUN_00294170 0x1acd00) [C: all 19 overlays].
/// Rilgar and Kalebo III sit 0x80 past the camera-globals shift (`LEVEL_SHIFT`), the rest follow it.
const SHADOW_LIST: [u32; 19] = [
	0x1ac600, 0x1aca00, 0x1acb80, 0x1ac680, 0x1ac700, 0x1aca80, 0x1acd00, 0x1ac600, 0x1acd80, 0x1ac700, 0x1aca00, 0x1acf80, 0x1aca00, 0x1ac880,
	0x1acd00, 0x1acb80, 0x1aca00, 0x1ace80, 0x1acf80,
];
/// `SHADOW_LIST` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const SHADOW_LIST_USA: [u32; 19] = [
	0x1ac600, 0x1aca80, 0x1acc00, 0x1ac700, 0x1ac780, 0x1aca80, 0x1acd80, 0x1ac680, 0x1ace00, 0x1ac780, 0x1aca80, 0x1ad000, 0x1aca80, 0x1ac900,
	0x1acd00, 0x1acc00, 0x1aca80, 0x1acf00, 0x1ad000,
];
/// The shadow pass gate the driver's caller tests (Eudora FUN_00242850 `*0x16a118`, Blarg FUN_0025e7d8 `*0x16a718`) [C: all
/// 19 overlays; = 0x16a118 + LEVEL_SHIFT[level] - LEVEL_SHIFT[4]].
const SHADOW_GATE: [u32; 19] = [
	0x16a018, 0x16a418, 0x16a598, 0x16a098, 0x16a118, 0x16a418, 0x16a718, 0x16a018, 0x16a798, 0x16a118, 0x16a418, 0x16a998, 0x16a418, 0x16a298,
	0x16a718, 0x16a598, 0x16a398, 0x16a898, 0x16a998,
];
/// `SHADOW_GATE` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const SHADOW_GATE_USA: [u32; 19] = [
	0x16a018, 0x16a498, 0x16a618, 0x16a118, 0x16a198, 0x16a418, 0x16a798, 0x16a098, 0x16a818, 0x16a198, 0x16a498, 0x16aa18, 0x16a498, 0x16a318,
	0x16a718, 0x16a618, 0x16a418, 0x16a918, 0x16aa18,
];
/// The caster count and the shadow DMA slot the driver tests: 0x15ff7c / 0x15ff80, 0x40 higher on levels 0 5 14 16 (Rilgar
/// driver: 0x15ffbc / 0x15ffc0) [C].
fn shadow_count(level: u32) -> (u32, u32) {
	if matches!(level, 0 | 5 | 14 | 16) { (va(0x15ffbc), va(0x15ffc0)) } else { (va(0x15ff7c), va(0x15ff80)) }
}

/// M3a shadows: the Eudora shadow driver 0x242500 (Novalis FUN_002645b0) runs from 0x242850 when `*0x16a118 != 0` and the
/// shadow DMA slot `*0x15ff80 != 0`; with caster count `*0x15ff7c` > 0 it builds one record per caster-list entry
/// (0x1ac700, 8 B: moby, fade u16, straddle u16) at 0x1ac880 (shadow_records_build 0x2787e0) and draws them
/// (0x2785e8) [C: Eudora disassembly; the Novalis addresses shifted by -0x300 as LEVEL_SHIFT]. The records stay in RAM
/// after the frame, so the last drawn frame's world-space primitives are read as they are. Every level runs the same
/// driver twin with its own addresses (`SHADOW_LIST`, `SHADOW_GATE`, `shadow_count`) [C: all 19 overlays, blarg_fx.md].
fn shadow_snap(m: &ee_oracle::Machine, level: u32) -> RetailShadows {
	let mut out = RetailShadows::default();
	let (Some(&list), Some(&gate)) = ((*crate::retail::disc::pick(&SHADOW_LIST, &SHADOW_LIST_USA)).get(level as usize), (*crate::retail::disc::pick(&SHADOW_GATE, &SHADOW_GATE_USA)).get(level as usize)) else {
		return out;
	};
	let (count, slot) = shadow_count(level);
	let (s, e) = table(level);
	let (mut a, e) = (m.rd32(s) & 0x01ff_ffff, m.rd32(e) & 0x01ff_ffff);
	while a < e {
		// 0x800 mobys always take the deferred list when drawn; a 0x400 moby only as a caster (below), else it is drawn at
		// once (Novalis 0x26b268-0x26b408, collision_and_gameplay.md SCH-6b) [C].
		if m.rd32(a + 0x24) != 0 && m.rd8(a + 0x20) < 0xfd && m.rd16(a + 0x34) & 0x800 != 0 {
			out.push_deferred(a, m.rd16(a + 0xa6) == 0);
		}
		a += 0x100;
	}
	if m.rd32(gate) == 0 || m.rd32(slot) == 0 {
		return out;
	}
	let mut r = list + 0x180;
	'list: for i in 0..m.rd32(count).min(48) {
		let moby = m.rd32(list + 8 * i) & 0x01ff_ffff;
		let n = m.rd32(r);
		if n > 64 {
			break;
		}
		let mut rc = crate::shadows::RecCaster { straddle: m.rd32(r + 4) != 0, band: [m.rdf(r + 8), m.rdf(r + 12)], light: m.rdv3(r + 16), prims: Vec::new() };
		r += 0x20;
		for _ in 0..n {
			// [I] guard: the record area's end (Eudora 0x1ae800, Blarg 0x1aee00 = list + 0x2100 [C]).
			if r >= list + 0x2100 {
				break 'list;
			}
			match m.rd32(r) {
				0 => {
					rc.prims.push(crate::shadows::RecPrim::Sphere { c: m.rdv3(r + 0x10), r: m.rdf(r + 0x1c), n: m.rd32(r + 4) });
					r += 0x20;
				}
				1 => {
					rc.prims.push(crate::shadows::RecPrim::Capsule {
						p0: m.rdv3(r + 0x10),
						r0: m.rdf(r + 0x1c),
						p1: m.rdv3(r + 0x20),
						r1: m.rdf(r + 0x2c),
						n0: m.rd32(r + 4),
						n1: m.rd32(r + 8),
					});
					r += 0x30;
				}
				_ => break 'list,
			}
		}
		out.push_deferred(moby, m.rd16(moby + 0xa6) == 0);
		out.casters.push((moby, rc));
	}
	out
}

/// Effects layer claimed set (retail/efx_draw.rs): the queue-A callbacks `snap` itself turns into mirror geometry.
fn claims(level: u32, f: u32) -> bool {
	let li = level as usize;
	li < 19 && f != 0 && ((*crate::retail::disc::pick(&GLOW_FN, &GLOW_FN_USA))[li] == f || (*crate::retail::disc::pick(&RETICLE_FN, &RETICLE_FN_USA))[li].contains(&f) || (*crate::retail::disc::pick(&MINE_RETICLE_FN, &MINE_RETICLE_FN_USA))[li] == f || (*crate::retail::disc::pick(&MARKER203_FN, &MARKER203_FN_USA))[li] == f || (*crate::retail::disc::pick(&SHIP_SHADOW_FN, &SHIP_SHADOW_FN_USA))[li] == f)
}

fn read_cam(m: &ee_oracle::Machine, level: u32) -> Option<Cam> {
	let (eye, basis, tan) = camera_globals(level)?;
	let f = m.rdv3(basis);
	Some((m.rdv3(eye), f[1].atan2(f[0]), f[2].clamp(-1.0, 1.0).asin(), m.rdf(tan)))
}

/// The render camera's up vector: row 2 of the published camera basis (0x1673d0 + 0x20 at Novalis, `camera_globals`) [C].
/// Retail's view matrix keeps the roll there (the follow camera aligns it to the smoothed hero up 0x167230, camera.md), so on
/// a magnetic wall the world turns around an upright Ratchet. (0, 0, 1) when the row is not a unit vector (level load).
fn read_cam_up(m: &ee_oracle::Machine, level: u32) -> [f32; 3] {
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

/// The interpreter on its own thread: each tick hands it the pad and takes the field it finished during the last
/// frame, so the game logic runs while the main thread renders (one field of input latency).
pub struct Retail {
	tx: std::sync::mpsc::Sender<FieldIn>,
	/// The interpreter thread; joined on drop while recording so a later load's `!LOAD` follows its last field.
	worker: Option<std::thread::JoinHandle<()>>,
	/// RAC_RETAIL_REPLAY: the pad file that replaces the keyboard.
	replay: Option<Arc<rec::Replay>>,
	replay_ended: bool,
	rx: std::sync::mpsc::Receiver<Snap>,
	/// Fields handed to the thread whose snapshot is not taken yet (1 = lockstep, more while `ahead` lets it run ahead).
	inflight: u32,
	/// Snapshots taken (`Snap::field`).
	taken: u32,
	/// [PC] How many fields the thread may hold (`depth`): 1, or up to 3 in gameplay and scenes while the window's frames
	/// take longer than a field, so the thread keeps working while a slow frame draws instead of waiting for the next tick.
	/// The pad of each field is still the one of its own tick (`fields` indexes it), only one or two fields later.
	ahead: u32,
	/// [PC] The window's frame interval in ms, averaged (`set_frame_ms`).
	frame_avg: f32,
	/// [PC] Milliseconds the last tick waited for the thread's field (the viewer's per-frame wait budget).
	pub waited: f32,
	/// [PC] A field the thread already finished, taken off `rx` by `field_ready` and used by the next tick.
	early: Option<Snap>,
	/// A movie the game thread waits for (fmv_bridge), until the viewer takes it (`take_movie`).
	movie_req: Option<fmv_bridge::Request>,
	/// Buttons held when a movie ended: kept from the game until released (the skip press must not reach it).
	movie_mask: u16,
	/// The movie playing now: what ends it early (retail's rule, fmv_bridge `Skip`) and the pad read at its last poll
	/// (`movie_pressed`); `pad_last` is the pad of the newest `tick_keys`.
	movie_rule: fmv_bridge::Skip,
	movie_prev: u16,
	pad_last: u16,
	snap: Snap,
	/// Class -> its retail props not holding a moby.
	free: HashMap<u32, Vec<usize>>,
	/// [PC] The order `bind` hands the game's props out in (None = index order): after an in-place pool growth the pool
	/// props as a rebuilt game holds them, each class's new props after its old ones (`App::append_pools`).
	order: Option<Vec<usize>>,
	/// Moby address -> the prop drawing it.
	held: HashMap<u32, usize>,
	/// Prop -> the +0x64 joint override records of the moby it draws (only props with records).
	prop_fx: HashMap<usize, Vec<crate::anim::JointFx>>,
	bound: bool,
	/// The level rac-rs has loaded for the props (`switch` follows retail's level travel).
	level: u32,
	/// RAC_RETAIL_TRAVEL's (level, field).
	travel: Option<(u32, u32)>,
	cam: Cam,
	cam_prev: Cam,
	cam_up: [f32; 3],
	cam_up_prev: [f32; 3],
	failed: bool,
	/// The error that stopped the thread, until `take_stop` hands it out.
	stop: Option<String>,
	/// Fields handed to the thread, for `script`.
	fields: u32,
	/// RAC_RETAIL_PAD (shots and tests): `from-to:buttons:lx,ly;...` in hex, held for fields `from..to` over the keys.
	script: Vec<(u32, u32, u16, [u8; 2], Option<[u8; 2]>)>,
	/// RAC_RETAIL_KEYTAP (tests: a hidden window has no keyboard): see `KeyTap`.
	keytap: Vec<KeyTap>,
	/// Movies the game asked for so far (`take_movie`), for `KeyTap` `movieN`.
	movies: u32,
	/// Keys (and the left / middle mouse buttons, `mouse_taps`: Circle / Triangle) pressed since the keyboard was last sampled, with the fields
	/// each still counts as held (`press_key`, `tick_keys`).
	taps: HashMap<winit::keyboard::KeyCode, u8>,
	/// The keys and controller buttons that ended a Controls-page capture (bound, or Esc): the game sees a neutral pad until they
	/// are released, else the bound key would act as the button it now is (Confirm) and the page would start a capture again.
	cap_hold: (Vec<winit::keyboard::KeyCode>, u32),
	mouse_taps: MouseTaps,
	/// Buttons the game's pad update read since `take_pad_seen`.
	pad_seen: u32,
	/// Interpreter time of the last field (ms), for the frame budget.
	pub ms: f32,
	/// Running average of `ms` over about 8 fields (the viewer's catch-up cap: a sustained worker-bound game runs one field per frame).
	pub ms_avg: f32,
	/// The retail HUD and menus come from the GS capture (rac-rs draws none of its own).
	pub gs: bool,
	/// GS-full display (`gsfull_on`): the game's whole picture from its own GS output, drawn on a thread of its own
	/// (gsfull/pipe.rs); `take_gsfull_frame` hands out the newest.
	pub gsfull: Option<Arc<crate::gsfull::pipe::GsFull>>,
	/// Width over height of the window, set by the viewer before `sync` (the moby side test of `SideOnly`).
	pub aspect: f32,
	/// The mouse in the page menus (retail/mouse_menu.rs).
	mm: mouse_menu::State,
	/// Keys held across a menu opening or closing (menu_keys.rs).
	mk: menu_keys::Layer,
	/// Pools `sync` found too small: the grown pool list for the viewer to rebuild the prop slots with (`take_grow`).
	grow: Option<Vec<(u32, u32)>>,
	/// `--title`: the boot to the game's own title screen (retail/title_boot.rs); None without the flag.
	tb: Option<title_boot::Boot>,
}

/// oracle/stubs.txt for the cold boot: the first one from the exe's folder upwards (an install, or a test build copied elsewhere,
/// keeps working after it was moved or its build folder is gone), else the one of the source tree the exe was built from. The
/// oracle skips a missing file silently, and a cold boot without it hangs on the title screen (seen 2026-10-15), so that is logged.
fn stubs_path() -> String {
	let exe = std::env::current_exe().ok();
	let near = exe.as_deref().and_then(|e| e.parent()).and_then(|d| d.ancestors().map(|a| a.join("oracle").join("stubs.txt")).find(|p| p.is_file()));
	if let Some(p) = near {
		return p.to_string_lossy().into_owned();
	}
	let built = concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/stubs.txt");
	if !std::path::Path::new(built).is_file() {
		eprintln!("retail: oracle/stubs.txt found neither above the exe nor at {built}: the cold boot will hang on the title screen");
	}
	built.into()
}

impl Retail {
	pub fn load() -> Option<Retail> {
		let dir = STATE.get()?;
		crate::loadlog::say(|| "retail: load starts".into());
		// The window's main loop runs here; the worker and the GS capture mark their own threads.
		hot::process();
		hot::thread();
		gscap::gpu_prestart(hot::idle);
		let cold = COLD.get();
		pcext::init(cold.is_some());
		prompts::init();
		// The picture thread's mirror (snap_pipe.rs) copies only the RAM pages written since its last sync: the live RAM is write-watched.
		if snap_pipe::enabled() {
			ee_oracle::mem::watch::watch_next();
		}
		let mut m = match cold {
			Some(iso) => {
				let stubs = std::env::var("ORACLE_STUBS").unwrap_or_else(|_| stubs_path());
				match ee_oracle::Machine::cold(iso, &stubs) {
					Ok(m) => m,
					Err(e) => {
						eprintln!("retail: cold boot failed: {e}");
						return None;
					}
				}
			}
			None => ee_oracle::Machine::load(dir),
		};
		crate::loadlog::say(|| "retail: machine ready".into());
		// The player's own memory card (retail/memcard.rs): a save made in an earlier session, before the first card call.
		memcard::restore(&mut m);
		m.record_sound();
		// The oracle driver's state before its first field: running voices of an adopted savestate, core mix registers.
		*crate::spumix::ATTACH.lock().unwrap() = Some(crate::spumix::attach_state(&mut m));
		let level = m.level();
		// The recompiled retail functions (ee_oracle rec/); ORACLE_REC=0 interprets everything.
		ee_game::register();
		crate::loadlog::say(|| "retail: recompiled code registered".into());
		// Movies play natively (retail/fmv_bridge.rs): the retail player function is replaced by a hook.
		fmv_bridge::reset();
		fmv_bridge::register();
		if cold.is_some() {
			title_boot::hook_boot_movies(&mut m);
		}
		// --wide (GS-full only): the wrapping hands of the field-of-view function (gsfull/wide.rs).
		if crate::gsfull::wide::enabled() && !disc::extras_off() {
			if gsfull_on() {
				crate::gsfull::wide::register();
				eprintln!("retail: 16:9 view (wide field of view, HUD squeezed to the corners)");
			} else {
				eprintln!("retail: --wide needs --gsfull (the mirror renderer has its own widescreen); ignored");
			}
		}
		// --modern-cam / RAC_MODERN_CAM=1 (retail/modcam.rs): off registers nothing (retail bit-exact).
		if modcam::enabled() {
			modcam::register();
			strafe::register();
			aim::register();
			eprintln!("retail: modern camera on (F7 toggles, RMB aims)");
		}
		// PC options rows (retail/pcext.rs): registered after modcam's hands; RAC_PC_EXT=0 registers nothing.
		if pcext::enabled() {
			pcext::register();
		}
		if !disc::extras_off() {
			vendor::register();
		}
		// A loaded savestate's level and draw queues are known now: its first field gets the effects layer too.
		if cold.is_none() {
			efx_own::prime(&mut m.cpu);
		}
		let rec = m.enable_rec();
		eprintln!("retail: {dir} level {level}, {rec} recompiled functions");
		rec::begin(dir, level, rec);
		let replay = rec::replay();
		if let Some(r) = &replay {
			let norm = |a: &str| a.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
			if norm(&r.state) != norm(dir) {
				eprintln!("retail: replay: the file starts from {} but this run loaded {dir}; it will not replay exactly", r.state);
			}
		}
		// A cold boot starts in the boot ELF (logos): no level yet, the front end's (blank) snapshot until the front end runs.
		let mut first = if cold.is_some() { front_snap(&m) } else { snap(&m) };
		let mut swap_lag = None;
		if let Some(fx) = first.fx.as_mut() {
			lag_swap(fx, &mut swap_lag);
		}
		// The hero layer list retail's next joint evaluation uses (see `lag_layers`).
		let mut layers_used = (first.level, first.parts.layers.clone());
		// RAC_RETAIL_HUD=0 keeps rac-rs's own HUD.
		// RAC_GSFULL=1 / --gsfull: the whole GS output is drawn instead (no mirror overlay capture).
		// A cold boot has no savestate dir for the pipe's start VRAM and VU1 memory: RAC_GSFULL_STATE names one (reference shots),
		// else the pipe starts blank ("": the boot uploads its own VU1 programs and draws everything it shows).
		let gs_dir = if cold.is_some() { std::env::var("RAC_GSFULL_STATE").unwrap_or_default() } else { dir.clone() };
		let gsf = if gsfull_on() { crate::gsfull::pipe::start(&gs_dir) } else { None };
		// EFFECTS LAYER, VU1 owners (retail/efx_vu.rs): the VIF1 + VU1 model the field's jobs pass through.
		let mut vush = efx_vu::Shadow::new();
		// The picture half (capture + chain walk) lives on its own thread (snap_pipe.rs) or, RAC_SNAP_PIPE=0, inline on the worker.
		let mut pipe = if snap_pipe::enabled() { snap_pipe::Pipe::new(efx_vu::Picker::new(&gs_dir), snap_pipe::check().then(|| efx_vu::Picker::new(&gs_dir))) } else { None };
		let mut picker = if pipe.is_none() { Some(efx_vu::Picker::new(&gs_dir)) } else { None };
		// Without the ghost pipe: capture + walk run on a helper thread beside `snap()` when that is faster (snap_pipe.rs `Par`).
		let mut par = if pipe.is_none() { snap_pipe::Par::new(snap_pipe::check().then(|| efx_vu::Picker::new(&gs_dir))) } else { None };
		let jobs_rx = gsf.as_ref().map(|_| {
			let (jtx, jrx) = std::sync::mpsc::channel::<ee_oracle::gs::Job>();
			m.cpu.mem.gsq = Some(jtx);
			jrx
		});
		if gsf.is_some() {
			eprintln!("retail: GS-full display: the game's own picture, {} raster threads", crate::gsfull::pipe::threads());
		} else if gsfull_on() {
			eprintln!("retail: GS-full display unavailable (needs GS.bin, vu1Memory.bin and vu1MicroMem.bin in {dir})");
		}
		let cap = if gsf.is_some() || std::env::var("RAC_RETAIL_HUD").is_ok_and(|v| v == "0") {
			None
		} else if cold.is_some() {
			capture_gs_vram(&mut m, vec![0u8; 4 << 20], &gs_dir)
		} else {
			capture_gs(&mut m, dir)
		};
		if cap.is_none() && gsf.is_none() {
			eprintln!("retail: no GS capture (rac-rs HUD)");
		}
		crate::loadlog::say(|| "retail: GS capture ready".into());
		let gs = cap.is_some() || gsf.is_some();
		let (gtx, gshared) = match gsf {
			Some((tx, sh)) => (Some(tx), Some(sh)),
			None => (None, None),
		};
		// RAC_RETAIL_TRAVEL=LEVEL:FIELD (shots and tests): at that field, request travel the way the level-end helper
		// does (Eudora 0x27f6d0: target 0x15f5a0, 0x15f5b8 = 1, 0x15f550 = 1).
		let travel: Option<(u32, u32)> = std::env::var("RAC_RETAIL_TRAVEL").ok().and_then(|v| {
			let (l, f) = v.split_once(':')?;
			Some((l.parse().ok()?, f.parse().ok()?))
		});
		// A replayed file's last travel request stands in for RAC_RETAIL_TRAVEL (shots wait for that level).
		let travel = match &replay {
			Some(r) => r.fields.iter().enumerate().rev().find_map(|(i, f)| Some((f.travel?, i as u32))),
			None => travel,
		};
		let (tx, prx) = std::sync::mpsc::channel::<FieldIn>();
		// RAC_RETAIL_PLAY=@play (acceptance test): the recorded pad report + RAM pokes of the savestate dir's pad.raw / pad.pokes.
		let play = playfeed::open(std::env::var("RAC_RETAIL_PLAY").ok().as_deref(), false, dir, 0, "RAC_RETAIL_PLAYTRAIL", playfeed::code_hash(include_str!("../../oracle/pcsx2rec/playfeed.rs")));
		let (ptx, rx) = std::sync::mpsc::channel();
		let gsh = gshared.clone();
		let tb = if title_boot::enabled() { Some(title_boot::Boot::new()) } else if cold.is_some() && std::env::var_os("RAC_TITLE_PILOT").is_some() { Some(title_boot::Boot::pilot_only()) } else { None };
		let hold_card = title_boot::enabled();
		// Cold boot: the logos run in the boot ELF before the front end is in RAM; the worker sends the blank front snapshot until then.
		let mut booting = cold.is_some();
		// ... and the boot ELF's own front end (title, New Game) is "the front end" until the first level overlay is in RAM
		// (its movie player is not frontbin's: `title_boot::front_end` never matches there).
		let mut cold_front = cold.is_some();
		let worker = std::thread::Builder::new()
			.name("retail".into())
			// Recompiled retail code nests a Rust frame per retail call.
			.stack_size(256 << 20)
			.spawn(move || {
				hot::thread();
				// Level travel: the level whose pools were counted, and whether a non-gameplay field (the travel
				// sequence and loader, game mode 6) ran since the level byte changed.
				let (mut pooled, mut transit) = (level, false);
				let (mut field, mut noted) = (0u32, (level, m.rd32(va(0x15f5a4))));
				let ov_log = std::env::var("RAC_RETAIL_OVLOG").is_ok();
				let ov_sync = ov_log || std::env::var_os("RAC_RETAIL_OVDUMP").is_some() || std::env::var_os("RAC_OV_SYNC").is_some();
				let title_trace = std::env::var_os("RAC_TITLE_TRACE").is_some();
				let mut last_mode = u32::MAX;
				let mut marks = std::collections::VecDeque::new();
				let mut movies = fmv_bridge::counts().0;
				let ov_delay = std::env::var("RAC_RETAIL_OV_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(OV_DELAY);
				let mut smoke = Smoke::from_env();
				let mut wperf = WPerf::from_env();
				let mut shipmouse_state = shipmouse::State::default();
				wperf.sample_pc(&m.cpu.pc as *const u32 as usize);
				let mut card = memcard::Saver::new(&m);
				if hold_card {
					card.hold(&m);
				}
				// The front end (frontbin, title_boot.rs): its picture comes from a GS-full pipe started when Quit Game is
				// confirmed (unless --gsfull runs one all along); `late_gs` says that one is ours.
				let (mut gtx, mut jobs_rx) = (gtx, jobs_rx);
				let (mut front, mut late_gs) = (false, false);
				// The capture draws the display buffer in full (gscap FULL_ON): the front end's mode 6 (New Game intro: caption
				// screens, fades around the movies), whose picture is not the title scene the mirror draws.
				let mut full_on = false;
				// P1 mode of the capture (gscap `P1`): a menu page with a VU1-drawn render-to-texture actor is up.
				let mut p1_on = false;
				// RAC_FRONT_GS=1: the front end shows the GS's own picture (debug / -GsFull references); else the mirror draws it.
				let gs_front = title_boot::gs_picture();
				let (mut front_n, mut front_ns) = (0u32, 0u64);
				let mut last_turbo = false;
				// The turbo starts once the load loop has run `TURBO_STABLE` fields in a row (`loop_n`); in it the capture draws a pair of
				// fields every `TURBO_SHOW_MS` (`show_t`, `show_n`: fields of the pair still to draw) and skips the prims of the others
				// (`draw_on`: the capture's DRAW_ON / DRAW_OFF state in full mode, which FULL_ON resets to drawing).
				let (mut loop_n, mut show_n, mut draw_on, mut show_t) = (0u32, 0u32, true, std::time::Instant::now());
				// RAC_LOAD_LOG (loadlog.rs): per 50 fields the average of the EE time, the rest of the worker's field and the time it
				// waited for the next field to be handed to it; and the mode changes of the front end / the level.
				let (mut ll_last, mut ll_idle, mut ll_ee, mut ll_post, mut ll_n) = (std::time::Instant::now(), 0u64, 0u64, 0u64, 0u32);
				let (mut ll_fmode, mut ll_lmode, mut ll_seen_front, mut ll_ctl, mut ll_ctl_field) = (u32::MAX, u32::MAX, false, false, 0u32);
				// RAC_LOAD_STOP=N (tests): exit N fields after the first gameplay field, with the RAM hash.
				let ll_stop: Option<u32> = std::env::var("RAC_LOAD_STOP").ok().and_then(|v| v.parse().ok());
				while let Ok(fin) = prx.recv() {
					fmv_bridge::hold_field();
					let t = std::time::Instant::now();
					ll_idle += t.duration_since(ll_last).as_nanos() as u64;
					wperf.lap(0);
					if let Some(to) = fin.travel.filter(|_| !disc::extras_off()) {
						m.cpu.mem.wr32(va(0x15f5a0), to);
						m.cpu.mem.wr32(va(0x15f5b8), 1);
						m.cpu.mem.wr32(va(0x15f550), 1);
					}
					// Ship fights: the mouse counts of the field become a virtual stick on top of the keyboard / pad sticks (shipmouse.rs);
					// the recording keeps the raw sticks + counts, so a replay re-derives the same bytes.
					let ship = !front && !booting && !disc::extras_off() && shipmouse::in_ship_fight(&m);
					// Turret sections (Batalia, Gaspar, Hoven): the same counts steer the turret through its stick (shipmouse.rs).
					let turret = if front || booting || disc::extras_off() { None } else { shipmouse::turret(&m, shipmouse::y_inverted(&m)) };
					let pad_sticks = shipmouse_state.step(ship, ship && shipmouse::y_inverted(&m), fin.mc.look, fin.sticks);
					let pad_sticks = shipmouse_state.step_turret(turret, fin.mc.look, pad_sticks);
					m.set_pad(fin.buttons, pad_sticks);
					// Button prompts: the device of this field (prompts.rs).
					prompts::begin_field(field, &mut m.cpu.mem);
					if !front && !booting && !disc::extras_off() {
						let mut fin2 = pcext::pre_field(&mut m, &fin);
						// The mouse steers the ship or the turret, not the modern camera, in a ship fight / turret section.
						if ship || turret.is_some() {
							fin2.mc.look = [0, 0];
						}
						modcam::pre_field(&mut m, &fin2);
						strafe::pre_field(&mut m, &fin2);
						aim::pre_field(&mut m, &fin2);
					} else if front {
						// The title's Options -> Graphics rows (pcext.rs `front_field`).
						pcext::front_field(&mut m);
					}
					// Dev debug keys (retail/debugkeys.rs): inert unless RAC_DEBUG_KEYS=1; their writes happen here, between two fields.
					debugkeys::pre_field(&mut m, field, !front && !booting);
					if let Some(p) = &play {
						p.apply(&mut m.cpu, field);
					}
					rec::field(field, &fin);
					if rec::trail_due(field) {
						rec::trail(field, &m.cpu.mem.ram[..]);
					}
					field += 1;
					if field % UNWIND_FIELDS == 0 {
						// The game thread is parked at the vblank wait: the flag unwinds its compiled frames there.
						m.cpu.rec_unwind = true;
					}
					wperf.lap(1);
					let t_frame = std::time::Instant::now();
					let teed = !front && !booting && efx_draw::enabled();
					if teed {
						vush.begin(&mut m.cpu.mem.gsq);
					}
					let ic0 = m.cpu.icount;
					let r = m.frame();
					disc::rescope(&m.cpu.mem.ram);
					wperf.icd = m.cpu.icount.wrapping_sub(ic0);
					vush.end(&mut m.cpu.mem.gsq);
					let frame_ns = t_frame.elapsed().as_nanos() as u64;
					// RAC_TEST_SLOW_EE=F (tests): the worker idles (F - 1) times its field after it, as a CPU F times slower would be busy.
					if let Some(f) = test_slow_ee() {
						std::thread::sleep(std::time::Duration::from_nanos((frame_ns as f64 * (f - 1.0)) as u64));
					}
					wperf.lap(2);
					// The title boot over without a front end (it failed): the card is the game's again.
					if card.is_held() && !title_boot::holding() {
						card.release(&mut m, false);
					}
					// The front end replaced the level (or the level replaced it): see title_boot.rs.
					let in_front = if cold_front {
						if title_boot::level_loaded(&m) {
							cold_front = false;
							false
						} else {
							true
						}
					} else {
						title_boot::front_end(&m)
					};
					if in_front && booting {
						booting = false;
						eprintln!("retail: cold boot: the front end is in RAM at field {field}");
					}
					if gs_front && !front && !in_front && gtx.is_none() && title_boot::want_late_gs(&m) {
						late_gs = start_late_gs(&mut m, dir, &mut gtx, &mut jobs_rx);
					}
					if in_front && !front {
						if gtx.is_none() && gs_front {
							late_gs = start_late_gs(&mut m, dir, &mut gtx, &mut jobs_rx);
						}
						// The mirror draws the front end (viewer front_mirror) but the capture keeps getting its GS jobs: they carry the
						// texture uploads the game makes once per boot (cold boot: the HUD / weapon-icon atlas at VRAM block 4096 is
						// uploaded at field 25, before any level) and that no later frame repeats until the pause menu. Its overlays
						// of these fields are dropped below (the front end is the mirror's picture) and not even drawn (DRAW_OFF).
						if let Some(q) = cap.as_ref().and_then(|c| c.tx.as_ref()) {
							let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), gscap::DRAW_OFF));
						}
						card.release(&mut m, true);
						if !cold_front {
							title_boot::hook_front_movies(&mut m);
						}
						// The title's Options -> Graphics row (pcext.rs `hook_front`).
						pcext::hook_front(&mut m);
						(pooled, transit, front_n, front_ns) = (u32::MAX, true, 0, 0);
						eprintln!("retail: front end running at field {field} (late GS pipe {late_gs})");
					} else if front && !in_front {
						if late_gs {
							// Back to the level: the mirror's HUD capture takes the jobs again, the front end's pipe ends.
							m.cpu.mem.gsq = cap.as_ref().and_then(|c| c.tx.clone());
							(gtx, jobs_rx, late_gs) = (None, None, false);
							title_boot::set_late(None);
						} else if gtx.is_none() {
							m.cpu.mem.gsq = cap.as_ref().and_then(|c| c.tx.clone());
						}
						// The capture draws again (it only kept the GS state and the VRAM transfers in the front end).
						if let Some(q) = cap.as_ref().and_then(|c| c.tx.as_ref()) {
							if std::mem::replace(&mut full_on, false) {
								let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), gscap::FULL_OFF));
							}
							let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), gscap::DRAW_ON));
						}
						eprintln!("retail: front end left at field {field} after {front_n} fields");
					}
					if cold.is_some() && !disc::extras_off() {
						pcext::watch_language(&m, field, in_front);
					}
					front = in_front;
					// Mode 6 of the front end: the display buffer's picture comes from the capture (gscap `full`), see the overlay below.
					let want_full = in_front && !booting && m.rd32(title_boot::front_mode()) == 6;
					if want_full != full_on {
						if let Some(q) = cap.as_ref().and_then(|c| c.tx.as_ref()) {
							let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), if want_full { gscap::FULL_ON } else { gscap::FULL_OFF }));
							full_on = want_full;
							(loop_n, show_n, draw_on) = (0, 0, true);
						}
					}
					if front && title_trace {
						let mode = m.rd32(title_boot::front_mode());
						if mode != last_mode {
							eprintln!("title: field {field} front mode {last_mode:#x} -> {mode:#x} crtc {:x?}", crate::gsfull::crtc::Crtc::from_regs(&m.cpu.mem.gs));
							last_mode = mode;
						}
					}
					if front {
						(front_n, front_ns) = (front_n + 1, front_ns + frame_ns);
						if front_n % 100 == 0 {
							eprintln!("retail: front end field {front_n}: ee {:.2} ms/field (last 100)", front_ns as f64 / 1e8);
							front_ns = 0;
						}
					}
					card.after_field(&m);
					// The level-load loop of the front end (`in_load_loop`): no pacing while it runs, the main thread hands the next field
					// at once (`Retail::turbo`), and the GS picture work only for a pair of fields every `TURBO_SHOW_MS`. The loop only
					// waits for the level data (it reads no pad); its picture, a slowly moving starfield, just moves on faster.
					let was_load = loop_n > 0;
					loop_n = if in_front && !booting && full_on && in_load_loop(&m, !cold_front) { loop_n.saturating_add(1) } else { 0 };
					// While the loop runs the capture draws its main-to-display copies as a clean picture (gscap `LOAD_ON`, `Gs::load`):
					// full mode keeps no main buffer, the copy would show stale VRAM. FULL_ON / FULL_OFF clear it on both sides.
					if (loop_n > 0) != was_load {
						if let Some(q) = cap.as_ref().and_then(|c| c.tx.as_ref()) {
							let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), if loop_n > 0 { gscap::LOAD_ON } else { gscap::LOAD_OFF }));
						}
					}
					let turbo = loop_n > TURBO_STABLE;
					if turbo != last_turbo {
						crate::loadlog::say(|| format!("field {field}: load loop turbo {}", if turbo { "ON" } else { "off" }));
						(show_n, show_t, last_turbo) = (0, std::time::Instant::now(), turbo);
					}
					// The marker takes effect from the next field's prims: the pair's second field copies the main buffer its first drew.
					let draw = !turbo
						|| if show_n > 0 {
							show_n -= 1;
							true
						} else if show_t.elapsed().as_millis() >= TURBO_SHOW_MS {
							(show_n, show_t) = (1, std::time::Instant::now());
							true
						} else {
							false
						};
					if full_on && draw != draw_on {
						if let Some(q) = cap.as_ref().and_then(|c| c.tx.as_ref()) {
							let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), if draw { gscap::DRAW_ON } else { gscap::DRAW_OFF }));
							draw_on = draw;
						}
					}
					if field % 100 == 0 && std::env::var_os("RAC_TITLE_TRACE").is_some() {
						let rw = |a: usize| i32::from_le_bytes(m.cpu.mem.ram[a..a + 4].try_into().unwrap());
						eprintln!("title: ram field {field}: level {} bolts {} slot summaries {:?} autosave {}", rw(0x15ed84), rw(0x15ed98), (0..6).map(|i| rw(0x13d2b0 + i * 0x1c)).collect::<Vec<_>>(), rw(0x13d384));
						eprintln!("title: ram field {field}: front mode {:#x} req level {} req flag {:#x} loader {:#x} pc {:08x} 15f5a4 {:#x}", rw(va(0x15f5e4)), rw(va(0x15f5a0)), rw(va(0x15f5b8)), rw(0x15ee48) & 0xffff, m.cpu.pc, rw(va(0x15f5a4)));
					}
					if !front && !booting && !disc::extras_off() {
						modcam::post_field(&m);
						strafe::post_field(&m);
					}
					smoke.field(field, frame_ns, &m.cpu.mem.ram[..]);
					let own = efx_own::field_end(&mut m.cpu);
					if std::env::var_os("RAC_EFX_LOG").is_some() {
						eprintln!("efxown field {field}: {:x?}", own);
						for o in &own {
							let p = efx_own::walk(&m.cpu, o);
							let mut ids = [0u32; 8];
							p.blocks.iter().for_each(|b| ids[b.3 as usize] += 1);
							let (vc, whole) = efx_own::vif_census(&p.words);
							eprintln!("efxpkt field {field} q{} #{} {:x}: ok {} words {} blocks {} ids {:?} vif mscal/f/cnt/direct/unpack/mpg {:?} whole {}", o.queue, o.idx, o.fnp, p.ok, p.words.len(), p.blocks.len(), ids, vc, whole);
							if vc[3] > 0 && vc[0] + vc[1] + vc[2] == 0 {
								eprintln!("efxgif field {field} {:x}: {:x?}", o.fnp, efx_own::gif_census(&p.words));
							}
						}
					}
					if let (Some(tx), Some(rx)) = (&gtx, &jobs_rx) {
						// One field's graphics jobs and CRTC registers to the GS-full threads; blocks while they are `QUEUE` fields behind.
						let inp = crate::gsfull::pipe::FieldIn { jobs: rx.try_iter().collect(), crtc: crate::gsfull::crtc::Crtc::from_regs(&m.cpu.mem.gs) };
						let t_send = std::time::Instant::now();
						let _ = tx.send(inp);
						if let Some(sh) = &gsh {
							sh.ee.work(frame_ns);
							sh.ee.wait(t_send.elapsed().as_nanos() as u64);
						}
					}
					wperf.lap(3);
					if let Some(c) = &cap {
						// The field's end for the capture: a picture drawn straight into the display buffer this field
						// (no-present fields, gscap `field_end`) shows now.
						if let Some(q) = &c.tx {
							// A movie went up this field (fmv_bridge: the game thread entered the player hook): its picture
							// replaced the display buffer, so the movie-end fade draws over the movie (gscap `movie_shown`).
							let shown = fmv_bridge::counts().0 + fmv_bridge::waiting() as u32;
							if std::mem::replace(&mut movies, shown) != shown {
								let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), gscap::MOVIE_SHOWN));
							}
							let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), u64::MAX));
						}
						if !turbo {
							c.pace(&mut marks);
						}
					}
					wperf.lap(4);
					let t_l4 = std::time::Instant::now();
					// EFFECTS LAYER, VU1 owners: the chain walk reads RAM here; the VIF1 / VU1 feed runs on its own thread while
					// the snapshot is built (retail/efx_vu.rs `prep` / `run`).
					let lv = m.cpu.mem.ram[LEVEL as usize] as u32;
					let efx_on = !front && !booting && efx_draw::enabled();
					let t_prep = std::time::Instant::now();
					let (mut bi, mut work, mut hand) = (None, None, None);
					let mut par_on = false;
					if efx_on {
						if let Some(p) = pipe.as_mut() {
							// The ghost machine gets the RAM the walk reads now; capture + walk run on the picture thread.
							hand = p.hand(&m, &own, lv, field, vush.take_pending());
						} else if let Some(pk) = picker.as_mut() {
							if par.as_mut().is_some_and(|p| p.want()) {
								// Capture + walk run on the helper while snap() below builds the snapshot; both only read the machine.
								par_on = par.as_mut().is_some_and(|p| p.start(&m, &own, lv, pk, vush.take_pending()));
								if !par_on {
									par = None;
								}
							} else {
								let b = efx_draw::capture(&m, lv, &own);
								work = Some(pk.prep(&m, &own, &b, vush.take_pending()));
								bi = Some(b);
							}
						}
					}
					wperf.extra(0, t_prep.elapsed().as_nanos() as u64);
					let t_snap = std::time::Instant::now();
					let mut s = if front || booting { front_snap(&m) } else { snap(&m) };
					let want_p1 = !front && !booting && rtt_page(&s);
					if want_p1 != p1_on {
						if let Some(q) = cap.as_ref().and_then(|c| c.tx.as_ref()) {
							let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), if want_p1 { gscap::P1_ON } else { gscap::P1_OFF }));
							p1_on = want_p1;
						}
					}
					wperf.extra(1, t_snap.elapsed().as_nanos() as u64);
					// EFFECTS LAYER: the VU1 feed and the owned DIRECT-GIF callbacks this field drew (retail/efx_draw.rs, after the
					// ported ones) are built on the VU1 thread while the EE runs the next field; `Retail::tick` appends them.
					let t_join = std::time::Instant::now();
					if par_on {
						if let Some((b, w)) = par.as_mut().and_then(|p| p.join(&m, field)) {
							bi = Some(b);
							work = Some(w);
						}
					}
					let t_sub = std::time::Instant::now();
					if let Some(h) = hand {
						s.fx_late = Some(FxLate::Later(h.finish(s.fx.is_some())));
					} else if let (Some(w), Some(b), Some(pk)) = (work, bi, picker.as_mut()) {
						s.fx_late = pk.submit(w, b, field, s.fx.is_some()).map(FxLate::Now);
					}
					if efx_on {
						if let Some(p) = par.as_mut() {
							p.done(t_prep.elapsed().as_nanos() as u64);
						}
					}
					wperf.extra(2, t_join.elapsed().as_nanos() as u64);
					wperf.lap(5);
					// After the section timer: the print itself takes ms when stderr is a file.
					if wperf.on && t_l4.elapsed().as_micros() > 2000 {
						let hs = |i: usize| snap_pipe::HS[i].load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6;
						eprintln!("perf: snap spike field {field}: total {:.2} ms = before {:.2} dispatch {:.2} snap() {:.2} join {:.2} submit {:.2} | par {par_on} | helper wake {:.2} capture {:.2} walk {:.2} | fresh buffers {} KB", t_l4.elapsed().as_secs_f64() * 1e3, t_prep.duration_since(t_l4).as_secs_f64() * 1e3, t_snap.duration_since(t_prep).as_secs_f64() * 1e3, t_join.duration_since(t_snap).as_secs_f64() * 1e3, t_sub.duration_since(t_join).as_secs_f64() * 1e3, t_sub.elapsed().as_secs_f64() * 1e3, hs(0), hs(1), hs(2), efx_vu::FRESH.swap(0, std::sync::atomic::Ordering::Relaxed) / 1024);
					}
					s.turbo = turbo;
					if !front && !booting {
						lag_layers(&mut s, &mut layers_used);
						if let Some(fx) = s.fx.as_mut() {
							lag_swap(fx, &mut swap_lag);
						}
					}
					if !front && !booting && (s.level, s.mode) != noted {
						noted = (s.level, s.mode);
						rec::note(field, s.level, s.mode);
					}
					if !front && !booting && s.level != pooled {
						if !arrived(&s, &m) {
							transit = true;
							// [PC] The pools as the moby table holds them now (it fills during the last fields of the landing): the level
							// prefetch builds the prop slots for them ahead of the arrival (`pools_hint`). Read only. Only a table that looks
							// real (`HINT_SLOTS`, `HINT_CLASS`).
							let (ts, te) = table(s.level);
							let (lo, hi) = (m.rd32(ts) & 0x01ff_ffff, m.rd32(te) & 0x01ff_ffff);
							let p = if lo <= hi && hi - lo <= HINT_SLOTS * 0x100 { count_pools(s.level, |a| m.rd32(a), |a| m.rd16(a)) } else { Vec::new() };
							if p.last().is_some_and(|c| c.0 < HINT_CLASS) {
								if let Ok(mut h) = POOLS_HINT.lock() {
									*h = Some((s.level, p));
								}
							}
						} else if transit {
							s.pools = Some(count_pools(s.level, |a| m.rd32(a), |a| m.rd16(a)));
							if let Ok(mut h) = POOLS_HINT.lock() {
								*h = None;
							}
							fmv_bridge::check_level(&m, s.level);
							(pooled, transit) = (s.level, false);
						}
					}
					s.sound = take_sound_of(&mut m);
					wperf.lap(6);
					// In-game the overlay is taken later, on the main thread (`OvLate`): the worker goes on to the next field while the
					// capture thread finishes this one. The front end and the debug dumps take it here.
					let late = !front && !booting && !ov_sync;
					if let (true, Some(c)) = (late, cap.as_ref()) {
						let (h, f) = (c.handle(), field as u64);
						s.ov_late = Some(OvLate(Box::new(move || h.take(f, ov_delay).map(|mut o| {
							let (w, rows) = ov_dims(&o);
							(w, rows, ov_px(&mut o), o.disp)
						}))));
					}
					let taken = if late { None } else { cap.as_ref().and_then(|c| c.take(field as u64, ov_delay)) };
					wperf.lap(7);
					if let Some(mut o) = taken.filter(|_| (!front && !booting) || full_on) {
						let (ow, orows) = ov_dims(&o);
						let px = ov_px(&mut o);
						// RAC_RETAIL_OVLOG=1: one line per overlay (the field it arrived in, FNV of its pixels): does the menu change?
						let h = if ov_log {
							px.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| (h ^ u32::from_le_bytes(*p) as u64).wrapping_mul(0x0100_0000_01b3))
						} else {
							0
						};
						if ov_log {
							eprintln!("retail: overlay field {field} frozen {} hash {h:016x}", o.frozen);
						}
						if let Ok(path) = std::env::var("RAC_RETAIL_OVDUMP") {
							let mut f = format!("P6 {} {} 255
", ow, orows).into_bytes();
							for p in &px {
								let p: &[u8; 4] = p;
								f.extend(p[..3].iter().zip([0.25f32, 0.25, 0.25]).map(|(c, _)| (*c as f32 + p[3] as f32 * 0.3) as u8));
							}
							let _ = std::fs::write(&path, f);
						}
						s.ov = Some((ow, orows, px));
						s.ov_disp = o.disp;
					}
					s.ms = t.elapsed().as_secs_f32() * 1000.0;
					if crate::loadlog::on() {
						let el = t.elapsed().as_nanos() as u64;
						(ll_ee, ll_post, ll_n) = (ll_ee + frame_ns, ll_post + el.saturating_sub(frame_ns), ll_n + 1);
						let (fm, lm) = (m.rd32(title_boot::front_mode()), m.rd32(va(0x15f5a4)));
						if front && fm != ll_fmode {
							crate::loadlog::say(|| format!("field {field}: front mode {ll_fmode:#x} -> {fm:#x}"));
							ll_fmode = fm;
						}
						if !front && !booting && lm != ll_lmode {
							crate::loadlog::say(|| format!("field {field}: level {} mode {ll_lmode:#x} -> {lm:#x} loader {:#x}", m.rd32(0x15ed84), m.rd32(0x15ee48) & 0xffff));
							ll_lmode = lm;
						}
						ll_seen_front |= front;
						if ll_seen_front && !front && !ll_ctl && lm == 0 {
							ll_ctl = true;
							ll_ctl_field = field;
							crate::loadlog::say(|| format!("field {field}: FIRST GAMEPLAY FIELD (level {} mode 0) ram {:016x}", m.rd32(0x15ed84), rec::hash_ram(&m.cpu.mem.ram[..])));
						}
						if let (true, Some(n)) = (ll_ctl, ll_stop) {
							if field >= ll_ctl_field + n {
								crate::loadlog::say(|| format!("field {field}: STOP ram {:016x}", rec::hash_ram(&m.cpu.mem.ram[..])));
								std::process::exit(0);
							}
						}
						if ll_n == 50 {
							let f = |v: u64| v as f64 / 1e6 / 50.0;
							crate::loadlog::say(|| format!("worker field {field} front {front} fmode {fm:#x} lmode {lm:#x} level {}: per field ee {:.1} ms, rest {:.1} ms, waited {:.1} ms (loader {:#x})", m.rd32(0x15ed84), f(ll_ee), f(ll_post), f(ll_idle), m.rd32(0x15ee48) & 0xffff));
							(ll_ee, ll_post, ll_idle, ll_n) = (0, 0, 0, 0);
						}
					}
					wperf.lap(8);
					wperf.field(field);
					// RAC_PERF_LOG=1 only: after the section timers (its eprintln takes up to 3.5 ms when stderr is a file or pipe, which
					// showed up as a 100-field `snap` max spike when it ran inside the snap section).
					snap_pipe::report(field);
					ll_last = std::time::Instant::now();
					s.err = r.err();
					let stop = s.err.is_some();
					if ptx.send(s).is_err() || stop {
						break;
					}
				}
				card.flush(&m);
			})
			.ok()?;
		crate::loadlog::say(|| "retail: load done, game thread started".into());
		let cam = first.cam.unwrap_or(([0.0; 3], 0.0, 0.0, 0.63));
		Some(Retail { tx, worker: Some(worker), replay, replay_ended: false, rx, inflight: 0, taken: 0, ahead: 1, frame_avg: 0.0, waited: 0.0, early: None, movie_req: None, movie_mask: 0, movie_rule: fmv_bridge::Skip::Start, movie_prev: 0, pad_last: 0, snap: first, free: HashMap::new(), order: None, held: HashMap::new(), prop_fx: HashMap::new(), bound: false, level, travel, cam, cam_prev: cam, cam_up: [0.0, 0.0, 1.0], cam_up_prev: [0.0, 0.0, 1.0], failed: false, stop: None, fields: 0, script: pad_script(), keytap: keytap_script(), movies: 0, taps: HashMap::new(), cap_hold: (Vec::new(), 0), mouse_taps: MouseTaps::default(), pad_seen: 0, ms: 0.0, ms_avg: 0.0, gs, gsfull: gshared, aspect: 4.0 / 3.0, mm: mouse_menu::State::default(), mk: menu_keys::Layer::default(), grow: None, tb })
	}

	/// One PAL field: takes the field the thread finished, then hands it this tick's pad.
	/// The sound requests of the field `tick` just took (each field's list is handed out once).
	pub fn take_sound(&mut self) -> SndBatch {
		let mut b = std::mem::take(&mut self.snap.sound);
		if b.attach.is_none() {
			b.attach = crate::spumix::ATTACH.lock().ok().and_then(|mut a| a.take());
		}
		b
	}

	/// A key went down (not an auto-repeat). The viewer samples the keyboard once per frame, and a frame can run several
	/// fields or take ~200 ms (the pause menu's GS capture paces the interpreter at ~35 ms a field), so a tap pressed and
	/// released between two samples never reached the game: the menu ignored most presses (2026-10-11). A tap now counts
	/// as held for at least `TAP_FIELDS` fields.
	pub fn press_key(&mut self, k: winit::keyboard::KeyCode) {
		self.taps.insert(k, TAP_FIELDS);
	}

	/// The left (0, Circle: fire) or middle (1, Triangle: Quick Select) mouse button went down; see `press_key`.
	pub fn press_mouse(&mut self, b: usize) {
		self.mouse_taps.press(b);
	}

	/// One field from the keyboard: the keys held now (`held`, `circle` / `triangle` for the left / middle mouse buttons)
	/// plus the taps still owed fields (`press_key`); then `tick`.
	pub fn tick_keys(&mut self, held: impl Fn(winit::keyboard::KeyCode) -> bool, circle: bool, triangle: bool) {
		let (m, parked) = (self.snap.mode, fmv_bridge::waiting());
		for t in self.keytap.iter_mut().filter(|t| !t.fired) {
			let on = match t.when.as_str() {
				"abs" => true,
				"movie" => parked,
				w if w.starts_with("movie") => parked && w[5..].parse() == Ok(self.movies),
				"scene" => m == 2 && !parked,
				"ship" => m == 6 && !parked,
				_ => false,
			};
			t.seen = if on { t.seen + 1 } else { 0 };
			if on && t.seen >= t.n {
				t.fired = true;
				eprintln!("retail: keytap {:?} at {}+{} (field {})", t.key, t.when, t.n, self.fields);
				if t.key == winit::keyboard::KeyCode::F24 {
					// "RMB": a right click (mouse_menu::State::back).
					self.mm.back();
					continue;
				}
				self.taps.insert(t.key, TAP_FIELDS);
			}
		}
		let taps = &self.taps;
		// A menu is open: the left button clicks it (Circle = fire is off), see `mouse_menu`, and the keyboard works it the modern
		// way with the buttons of the screen on show (`menu_keys`: arrows / WASD, Enter / Space, Esc / Backspace, Q / E, Tab).
		let menu = self.snap.menu.as_ref().filter(|_| !self.tb.as_ref().is_some_and(|t| t.active()));
		// The screen whose keys apply: a page menu's, else a mode-4 dialog's, else the vendor shop's (no page menus).
		let keys_screen = menu.map(menu_keys::screen).or(self.snap.dialog).or_else(|| self.snap.shop.as_ref().map(|_| menu_keys::VENDOR));
		let down = |k| held(k) || taps.contains_key(&k);
		self.mk.begin(keys_screen.is_some(), &down);
		let mk = &self.mk;
		let live = |k| down(k) && mk.live(k);
		let middle = triangle || self.mouse_taps.0[1] > 0;
		let (mut b, mut s) = match keys_screen {
			Some(sc) => {
				let (b, s) = menu_keys::menu_pad(&sc, &live);
				// A dialog or the vendor (no page menu, whose clicks `mouse_menu` takes): a left click is the screen's confirm, as
				// Enter. Only a click made there (the tap latch, not a button held since play, where it fired).
				let click = menu.is_none() && self.mouse_taps.0[0] > 0;
				let extra = (if middle { mouse_menu::TRIANGLE } else { 0 }) | (if click { sc.confirm } else { 0 });
				(b | extra, s)
			}
			// A ship fight (shipmouse.rs): the left mouse button fires with Cross, the ship's fire button; elsewhere it is Circle.
			None => {
				let (as_circle, extra) = shipmouse::left_button(self.snap.ship_fight, circle || self.mouse_taps.0[0] > 0);
				let (b, s) = pad_from_keys(live, as_circle, middle);
				(b | extra, s)
			}
		};
		menu_keys::log(self.fields, menu, keys_screen.filter(|_| menu.is_none()), b);
		// A gamepad adds its buttons and sticks and, when it moved, becomes the device of the button prompts (prompts.rs).
		prompts::poll_pad(&mut b, &mut s);
		let (mode, front) = (self.snap.mode, self.snap.front);
		// Esc / Space skip a scene or ship flight through retail's own skip test (`skip_feed`); the gamepad's Start does the
		// same, like Space on the keyboard.
		let skip_key = [winit::keyboard::KeyCode::Escape, winit::keyboard::KeyCode::Space].iter().any(|&k| held(k) || taps.contains_key(&k))
			|| (prompts::pad_buttons() & START) != 0;
		if !front {
			b |= skip_feed(mode, self.snap.ship, skip_key);
		}
		if let Some(tb) = self.tb.as_mut().filter(|t| t.active()) {
			// The boot to the title (title_boot.rs): the game is steered by pulses, the player's keys are ignored.
			b = tb.step(self.snap.menu.as_ref(), mode, front);
			s = [0x80; 4];
		} else {
			b |= self.mm.step(self.snap.menu.as_ref(), b);
			if let Some(tb) = self.tb.as_mut() {
				b |= tb.pilot(self.snap.menu.as_ref(), mode, front, &mut self.mm);
			}
		}
		// The Controls page waits for a key or button (binds.rs): the keys that went down this field (a tap counts TAP_FIELDS at
		// its first poll, so the Confirm that started the capture never binds) and the controller's new buttons go to it, and
		// the game sees a neutral pad meanwhile.
		if binds::capturing() {
			let fresh: Vec<winit::keyboard::KeyCode> = taps.iter().filter(|(_, n)| **n == TAP_FIELDS).map(|(k, _)| *k).collect();
			binds::capture_poll(&fresh, prompts::phys_fresh());
			self.cap_hold = (fresh, prompts::phys_now());
			(b, s) = (0, [0x80; 4]);
		} else if !self.cap_hold.0.is_empty() || self.cap_hold.1 != 0 {
			// The key or button that ended the capture is still down: it is not a press on the menu (it may be Confirm or Back now).
			self.cap_hold.0.retain(|k| down(*k));
			self.cap_hold.1 &= prompts::phys_now();
			if !self.cap_hold.0.is_empty() || self.cap_hold.1 != 0 {
				(b, s) = (0, [0x80; 4]);
			}
		}
		// [PC] DEV ONLY (RAC_DEV_TOOLS=1, devtools.rs): the free camera hands the game a neutral pad; the stick clicks that turned
		// it off stay away from the game until released. Off: no change.
		if devtools::free_on() {
			(b, s) = (0, [0x80; 4]);
		} else {
			b &= !devtools::pad_mask();
		}
		self.taps.retain(|_, n| {
			*n -= 1;
			*n > 0
		});
		self.mouse_taps.tick();
		if crate::loadlog::on() && b != self.pad_last {
			crate::loadlog::say(|| format!("pad field {} {b:#x}", self.fields));
		}
		self.pad_last = b;
		self.tick(b, s);
	}

	/// A page menu is on screen (pause menu, Quick Select, map): the mouse works it and the left button does not fire.
	pub fn mouse_menu_active(&self) -> bool {
		self.snap.menu.is_some()
	}

	/// The cursor is at (fx, fy), fractions of the window's client area: the overlay pixel under it.
	pub fn mouse_cursor(&mut self, fx: f32, fy: f32) {
		let full = self.gsfull_view().map(|f| if f.wide { crate::gsfull::wide::ASPECT } else { 4.0 / 3.0 });
		let (x, y) = mouse_menu::to_ps2(fx, fy, self.aspect, full);
		self.mm.cursor(x, y);
	}

	/// Left click / right click / wheel notches (positive = up) over a menu.
	pub fn mouse_click(&mut self) {
		self.mm.click();
	}

	pub fn mouse_back(&mut self) {
		self.mm.back();
	}

	pub fn mouse_wheel(&mut self, y: f32) {
		self.mm.wheel(y);
	}

	/// Gameplay on screen (game mode 0): the modern camera may capture the mouse.
	pub fn gameplay(&self) -> bool {
		self.snap.mode == 0 && !self.snap.front && !self.failed
	}

	/// The vendor screen is up (game mode 5): the world behind it stands still.
	pub fn vendor(&self) -> bool {
		self.snap.mode == 5 && !self.snap.front
	}



	pub fn tick(&mut self, buttons: u16, sticks: [u8; 4]) {
		self.tick_inner(buttons, sticks);
		// The next field's pad is out (the worker runs it now): only then wait for the capture thread's overlay of the field just taken.
		if let Some(l) = self.snap.ov_late.take() {
			let w = crate::runlog::waiting("the GS capture thread's overlay of the last field");
			let got = (l.0)();
			drop(w);
			if let Some((w, rows, px, disp)) = got {
				self.snap.ov = Some((w, rows, px));
				self.snap.ov_disp = disp;
			}
		}
		// The effects layer of the field just taken (built on the VU1 thread meanwhile): the quads follow the ported ones.
		if let Some(l) = self.snap.fx_late.take() {
			let t_w = std::time::Instant::now();
			let w = crate::runlog::waiting("the effects (VU1) thread's layer of the last field");
			let got = l.get();
			drop(w);
			efx_vu::prof_wait(t_w.elapsed().as_nanos() as u64);
			if let (Some((qa, pools)), Some(fx)) = (got, self.snap.fx.as_mut()) {
				fx.qa.extend(qa);
				fx.pools.extend(pools);
			}
		}
	}

	fn tick_inner(&mut self, buttons: u16, sticks: [u8; 4]) {
		if self.failed {
			return;
		}
		self.waited = 0.0;
		if self.inflight > 0 {
			let t_wait = std::time::Instant::now();
			let w = crate::runlog::waiting("the game thread's next field (retail)");
			// The game thread can park in the movie hook (fmv_bridge) in the middle of a field: wait for the field in
			// 2 ms steps and hand a movie request to the viewer instead of blocking; while the movie plays, return at once.
			let got = loop {
				if let Some(s) = self.early.take() {
					break Ok(s);
				}
				match self.rx.try_recv() {
					Ok(s) => break Ok(s),
					Err(std::sync::mpsc::TryRecvError::Disconnected) => break Err(()),
					Err(std::sync::mpsc::TryRecvError::Empty) => {}
				}
				if let Some(q) = fmv_bridge::take_request() {
					self.movie_req = Some(q);
					return;
				}
				if fmv_bridge::waiting() {
					return;
				}
				match self.rx.recv_timeout(std::time::Duration::from_millis(2)) {
					Ok(s) => break Ok(s),
					Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
					Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break Err(()),
				}
			};
			drop(w);
			self.waited = t_wait.elapsed().as_secs_f32() * 1000.0;
			match got {
				Ok(mut s) => {
					// Several ticks can run per frame: the travel pools wait for `switch`, the overlay for `take_overlay`.
					if s.pools.is_none() {
						s.pools = self.snap.pools.take();
					}
					if s.ov.is_none() {
						s.ov = self.snap.ov.take();
					}
					if let Some(e) = &s.err {
						eprintln!("retail: {e}");
						self.failed = true;
						self.stop = Some(e.clone());
					}
					self.ms = s.ms;
					self.ms_avg += (s.ms - self.ms_avg) * 0.125;
					self.pad_seen |= s.pad.iter().fold(0, |a, p| a | p);
					self.taken += 1;
					s.field = self.taken;
					// "Keep the last" (orbs / fx / shadows are None while frozen) is decided on the SHOWN snap: `native_delay` queues
					// 2 fields in modes != 0, so handing the old shown snap's pieces to the newest queued snap (as before) left the
					// shown one without them for 2 of every 3 fields (pause menu: the sea / water flickered every third field).
					let mut old = std::mem::take(&mut self.snap);
					self.snap = native_delay(s);
					if self.snap.orbs.is_none() {
						self.snap.orbs = old.orbs.take();
					}
					if self.snap.fx.is_none() {
						self.snap.fx = old.fx.take();
					}
					if self.snap.shadows.is_none() {
						self.snap.shadows = old.shadows.take();
					}
					// The front end replaced the level: whatever level comes next is a new one for rac-rs (`switch`), also the same number.
					if self.snap.front && self.level != u32::MAX {
						self.level = u32::MAX;
						self.unbind();
					}
				}
				Err(_) => self.failed = true,
			}
			self.inflight -= 1;
		}
		// The game thread is in the movie hook (its field already ended through the vblank callback): hold the next
		// field until the movie ends, so no `Machine::frame` waits on its 20 s watchdog meanwhile.
		if let Some(q) = fmv_bridge::take_request() {
			self.movie_req = Some(q);
		}
		if fmv_bridge::waiting() {
			return;
		}
		self.cam_prev = self.cam;
		self.cam_up_prev = self.cam_up;
		// Scenes and the ship streams write the render camera too (scene_camera_eval FUN_00289608: eye 0x166ec0, rows
		// 0x1670d0, tan 0x16cbf0 = Eudora's `camera_globals`); outside gameplay only while rac-rs shows retail's level.
		OCC_MASK.lock().map(|mut g| *g = None).ok();
		if let Some(c) = self.snap.cam.filter(|_| !self.snap.hold && (self.snap.mode == 0 || self.snap.level == self.level)) {
			OCC_MASK.lock().map(|mut g| *g = self.snap.occ_mask).ok();
			self.cam = c;
			self.cam_up = self.snap.cam_up;
		}
		// [PC] Run-ahead (`ahead`): hand the thread fields until it holds `depth` of them (one in lockstep).
		let want = self.depth();
		while self.inflight < want && !self.failed {
			// RAC_RETAIL_HOLD=N (shots): run no field past N, so a normal and a --gsfull shot show the same retail field.
			if std::env::var("RAC_RETAIL_HOLD").ok().and_then(|v| v.parse::<u64>().ok()).is_some_and(|n| self.fields as u64 >= n) {
				return;
			}
			// Debug shots (RAC_SHOT_FIELD = N): the game stops after field N, so the shot shows exactly that field.
			if std::env::var("RAC_SHOT_FIELD").ok().and_then(|v| v.parse::<u32>().ok()).is_some_and(|f| self.fields >= f + SHOT_ALLOW.load(std::sync::atomic::Ordering::Relaxed)) {
				return;
			}
			// [PC] DEV ONLY: the dev tools' planet warp (devtools.rs, RAC_DEV_TOOLS=1) is the same travel request, in gameplay only.
			devtools::feed_field(self.fields);
			let warp_ok = self.gameplay() && !self.snap.hold && self.snap.level == self.level;
			// The field's final input: a replayed file, or the keys with the RAC_RETAIL_PAD script ORed in. The worker
			// records exactly this (rec.rs), so a replay is field-exact whatever the window's frame timing was.
			let fin = if let Some(r) = &self.replay {
				r.fields.get(self.fields as usize).copied().unwrap_or_else(|| {
					if !std::mem::replace(&mut self.replay_ended, true) {
						eprintln!("retail: replay ended at field {} (neutral pad from here)", self.fields);
					}
					FieldIn { buttons: 0, sticks: [0x80; 4], travel: None, mc: modcam::Input::default() }
				})
			} else {
				let (mut buttons, mut sticks) = (buttons, sticks);
				// The keys that skipped a movie stay away from the game until released.
				self.movie_mask &= buttons;
				buttons &= !self.movie_mask;
				for &(a, b, bt, st, rs) in &self.script {
					if (a..b).contains(&self.fields) {
						buttons |= bt;
						(sticks[2], sticks[3]) = (st[0], st[1]);
						if let Some(r) = rs {
							(sticks[0], sticks[1]) = (r[0], r[1]);
						}
					}
				}
				let travel = self.travel.filter(|&(_, f)| f == self.fields).map(|(l, _)| l).or_else(|| devtools::take_warp(warp_ok, self.level));
				FieldIn { buttons, sticks, travel, mc: modcam::take_ui(self.fields) }
			};
			// [PC] DEV ONLY: under a replay (tests) the warp joins the replayed field; with the tools off a replay is untouched.
			let fin = if self.replay.is_some() && fin.travel.is_none() && devtools::enabled() { FieldIn { travel: devtools::take_warp(warp_ok, self.level), ..fin } } else { fin };
			self.fields += 1;
			if self.tx.send(fin).is_err() {
				break;
			}
			self.inflight += 1;
		}
	}

	/// [PC] The next tick takes its field without waiting: the thread already finished it (kept in `early`), or the tick
	/// waits for nothing (no field handed out, a movie, a stopped game).
	pub fn field_ready(&mut self) -> bool {
		if self.failed || self.inflight == 0 || self.early.is_some() || fmv_bridge::waiting() {
			return true;
		}
		match self.rx.try_recv() {
			Ok(s) => {
				self.early = Some(s);
				true
			}
			Err(std::sync::mpsc::TryRecvError::Disconnected) => true,
			Err(std::sync::mpsc::TryRecvError::Empty) => false,
		}
	}

	/// [PC] The run-ahead depth for this tick (`ahead`). 1 outside gameplay and scenes (the front end, menus, movies, level
	/// loads and travel, the title boot) and with RAC_RUN_AHEAD=0; else the fields that end while one frame draws, up to 3:
	/// 1 while frames come at least once a field (a fast PC is in lockstep as before), raised at once when frames slow
	/// down, lowered when they are fast again.
	fn depth(&mut self) -> u32 {
		static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
		let off = *OFF.get_or_init(|| std::env::var("RAC_RUN_AHEAD").is_ok_and(|v| v == "0"));
		let s = &self.snap;
		let free = !off && (s.mode == 0 || s.mode == 2) && !s.front && !s.frozen && !s.turbo && s.level == self.level
			&& self.movie_req.is_none() && !fmv_bridge::waiting() && !self.tb.as_ref().is_some_and(|t| t.active());
		if !free {
			self.ahead = 1;
			return 1;
		}
		let f = disc::field_dt() * 1000.0;
		let up = ((self.frame_avg / f).ceil() as u32).clamp(1, 3);
		if up > self.ahead {
			self.ahead = up;
		} else if self.ahead > 1 && self.frame_avg < (self.ahead - 1) as f32 * f * 0.8 {
			self.ahead -= 1;
		}
		self.ahead
	}

	/// [PC] The window's frame interval of this redraw (ms), for the run-ahead depth.
	pub fn set_frame_ms(&mut self, ms: f32) {
		self.frame_avg += (ms.min(100.0) - self.frame_avg) * 0.1;
	}

	/// A movie the game asked for (once): the viewer plays it and calls `end_movie` when it ended or was skipped.
	/// Fields handed to the game so far (logs).
	pub fn field_count(&self) -> u32 {
		self.fields
	}

	pub fn take_movie(&mut self) -> Option<fmv_bridge::Request> {
		let q = self.movie_req.take()?;
		self.movies += 1;
		// The title boot runs on black: a movie in the way (the Veldin intro) is skipped at once.
		if self.tb.as_ref().is_some_and(|t| t.active()) {
			eprintln!("title: movie lba {} skipped during the boot", q.lba);
			self.end_movie();
			return None;
		}
		// The skip rule of this movie; a button down now is not a new press (retail reads newly pressed buttons).
		(self.movie_rule, self.movie_prev) = (q.skip, self.pad_last);
		Some(q)
	}

	/// The front end is in its level-load loop and no movie is parked: the viewer may run fields without waiting for the 50 Hz step.
	pub fn turbo(&self) -> bool {
		self.snap.turbo && !self.failed && !fmv_bridge::waiting()
	}

	/// The front end runs (frontbin) and the mirror draws it from this state; None outside it or with RAC_FRONT_GS.
	pub fn front_ram(&self) -> Option<&FrontRam> {
		if title_boot::gs_picture() {
			return None;
		}
		self.snap.front_ram.as_ref().filter(|_| self.snap.front)
	}

	/// The front end's menu snapshot (page, items with PS2-pixel rects, cursor).
	pub fn front_menu(&self) -> Option<&mouse_menu::MenuSnap> {
		self.snap.menu.as_ref().filter(|_| self.snap.front)
	}

	/// The title boot is running: the viewer draws plain black and mutes the sound.
	pub fn title_cover(&self) -> bool {
		self.tb.as_ref().is_some_and(|t| t.active())
	}

	/// The GS-full pipe whose picture is on screen: the one `--gsfull` runs all along, else the front end's (title_boot.rs).
	pub fn gsfull_view(&self) -> Option<Arc<crate::gsfull::pipe::GsFull>> {
		self.gsfull.clone().or_else(title_boot::late)
	}

	/// Retail's skip test for the movie playing (`take_movie` stored its rule), for every movie the game plays, in the front
	/// end and in a level alike: true when the keys now give a press the rule accepts that was not down at the previous call
	/// (`movie_poll`). Start is Esc / Enter; the combo's L1 + R1 + L2 + R2 are C, Left Ctrl, V, B (`pad_from_keys`).
	pub fn movie_pressed(&mut self, held: impl Fn(winit::keyboard::KeyCode) -> bool) -> bool {
		let taps = &self.taps;
		let (hit, now) = movie_poll(self.movie_rule, self.movie_prev, |k| held(k) || taps.contains_key(&k), prompts::pad_buttons());
		self.movie_prev = now;
		hit
	}

	/// The movie ended or was skipped: the game thread returns from the player (v0 = 0, as retail's does). The buttons
	/// held now reach the game only after a release.
	pub fn end_movie(&mut self) {
		self.movie_mask = 0xffff;
		fmv_bridge::finish();
	}

	/// A ship fight is on in the latest field (`shipmouse::in_ship_fight`).
	pub fn ship_fight(&self) -> bool {
		self.snap.ship_fight
	}

	/// The error that stopped the interpreter, once (the viewer shows it in the title bar).
	pub fn take_stop(&mut self) -> Option<String> {
		self.stop.take()
	}

	/// The render camera's up vector between the last two ticks (roll; `camera`'s companion).
	pub fn camera_up(&self, alpha: f32) -> [f32; 3] {
		let (a, b) = (self.cam_up_prev, self.cam_up);
		let v = [a[0] + (b[0] - a[0]) * alpha, a[1] + (b[1] - a[1]) * alpha, a[2] + (b[2] - a[2]) * alpha];
		let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
		if l > 1e-4 { [v[0] / l, v[1] / l, v[2] / l] } else { [0.0, 0.0, 1.0] }
	}

	/// [PC] How far a smoothed frame's camera (`camera(alpha)`, between the last two ticks) can trail the newest tick's view that
	/// retail's moby verdict uses: the turn and zoom between the ticks plus 0.03 rad, the camera's move plus 0.25 m (the mobys'
	/// own smoothed motion); capped at 0.35 rad and 4 m so a camera cut does not draw every side-culled moby.
	pub fn view_lag(&self) -> ViewLag {
		let (a, b) = (self.cam_prev, self.cam);
		let mut dy = (b.1 - a.1).abs() % std::f32::consts::TAU;
		if dy > std::f32::consts::PI {
			dy = std::f32::consts::TAU - dy;
		}
		let zoom = (a.3.atan() - b.3.atan()).max(0.0);
		let d = [b.0[0] - a.0[0], b.0[1] - a.0[1], b.0[2] - a.0[2]];
		let mv = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
		ViewLag { ang: (dy + (b.2 - a.2).abs() + zoom + 0.03).min(0.35), grow: (mv + 0.25).min(4.0) * 1024.0 }
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

	/// Retail finished travelling to another level: (level, its prop pools) once, after which the viewer loads that
	/// level and starts a game with this `Retail` again (the props bind anew).
	/// Retail arrived on another level and `switch` has not run yet (the viewer still holds the old level's assets).
	pub fn switch_pending(&self) -> bool {
		self.snap.level != self.level && self.snap.pools.is_some()
	}

	/// [PC] The level retail has begun travelling to (its level word changed) while `switch` has not run: the viewer starts that
	/// level's host-side parse now (`App::prefetch_level`), during the landing stream, instead of at the switch.
	pub fn pending_pools(&self) -> Option<Vec<(u32, u32)>> {
		if self.snap.level == self.level { None } else { self.snap.pools.clone() }
	}

	pub fn travel_target(&self) -> Option<usize> {
		(self.snap.level != self.level).then_some(self.snap.level as usize)
	}

	pub fn switch(&mut self) -> Option<(usize, Vec<(u32, u32)>)> {
		if self.snap.level == self.level {
			return None;
		}
		let pools = self.snap.pools.take()?;
		self.level = self.snap.level;
		self.free.clear();
		self.held.clear();
		self.bound = false;
		self.order = None;
		self.cam_prev = self.cam;
		self.cam_up_prev = self.cam_up;
		Some((self.level as usize, pools))
	}


	/// [PC] The grown pools of `take_grow`, the bound props left as they are: the viewer appends the new props to the
	/// running game (`App::append_pools`) and hands them out anew with `rebind`, or calls `unbind` and rebuilds.
	pub fn take_grow_kept(&mut self) -> Option<Vec<(u32, u32)>> {
		self.grow.take()
	}

	/// Every prop free again: the next `sync` binds the game's props anew (a rebuilt game, index order).
	pub fn unbind(&mut self) {
		self.free.clear();
		self.held.clear();
		self.bound = false;
		self.order = None;
	}

	/// [PC] The game's prop order `bind` follows (`order`), None = index order.
	pub fn bind_order(&self) -> Option<&[usize]> {
		self.order.as_deref()
	}

	/// [PC] An in-place pool growth (`App::append_pools`): every prop free again, handed out in `order` from the next
	/// `sync` on (as a rebuilt game binds its pools).
	pub fn rebind(&mut self, order: Vec<usize>) {
		self.unbind();
		self.order = Some(order);
	}

	/// Debug shots: RAC_SHOT_FIELD = N holds the capture until N retail fields were sent (same field in normal and gsfull).
	pub fn shot_field_wait(&self) -> bool {
		// Field N = the snap of the Nth field is shown (and in --gsfull the frame of that field is rasterised).
		std::env::var("RAC_SHOT_FIELD").ok().and_then(|v| v.parse::<u32>().ok()).is_some_and(|f| { let f = f + SHOT_ALLOW.load(std::sync::atomic::Ordering::Relaxed); self.snap.field < f || self.gsfull.is_some() && GSF_SHOWN.load(std::sync::atomic::Ordering::Relaxed) < f as u64 })
	}

	/// RAC_RETAIL_TRAVEL is set and rac-rs has not reached its level yet (shots wait for it).
	pub fn travelling(&self) -> bool {
		self.travel.is_some_and(|(l, _)| l != self.level)
	}

	fn bind(&mut self, g: &Game) {
		let order: Vec<usize> = match &self.order {
			Some(o) if o.len() == g.props.len() => o.clone(),
			_ => (0..g.props.len()).collect(),
		};
		for i in order {
			let p = &g.props[i];
			if p.is_retail() {
				self.free.entry(p.class_id).or_default().push(i);
			}
		}
		for v in self.free.values_mut() {
			v.reverse();
		}
		self.bound = true;
	}

	/// The newest GS-full picture (512x448 RGBA8) that finished since the last call, if any.
	pub fn take_gsfull_frame(&mut self) -> Option<crate::gsfull::pipe::Frame> {
		let sh = self.gsfull_view()?;
		let f = sh.take()?;
		GSF_SHOWN.store(f.field + 1, std::sync::atomic::Ordering::Relaxed);
		Some(f)
	}

	/// The retail overlay of the field `tick` just took, if one finished (each is handed out once).
	/// This field's overlay was drawn into the display buffer (no-present field) and is not one uniform colour: it
	/// replaces a held movie picture.
	pub fn overlay_covers(&self) -> bool {
		self.snap.ov_disp && self.movie_tint().is_none() && self.snap.ov.is_some()
	}

	/// This field's display-buffer overlay is one colour over the whole frame (the movie-end fade: black sprite strips
	/// over the movie's last picture, GS alpha (Cs-Cd)*As+Cd [C stage 6]): the held movie picture's tint, as
	/// `fmv::present::Presenter::tint` (rgb added, a = how much of the picture shows).
	pub fn movie_tint(&self) -> Option<[f32; 4]> {
		let (_, _, px) = self.snap.ov.as_ref().filter(|_| self.snap.ov_disp)?;
		let p = *px.first()?;
		px.iter().all(|q| *q == p).then(|| p.map(|c| c as f32 / 255.0))
	}

	pub fn take_overlay(&mut self) -> Option<(u32, u32, Vec<[u8; 4]>)> {
		self.snap.ov.take()
	}

	/// Retail draws no world this field (pause menu, movie, level load: `frozen_view`).
	pub fn paused(&self) -> bool {
		self.snap.frozen
	}

	/// A menu mode (3: pause, galactic map, the post-game page) presents its own picture this field: `NO_PRESENT` is clear, the
	/// GS capture's overlay is the whole picture, so a movie's held last picture (viewer MOVIE_HOLD) must give way to it.
	pub fn menu_presents(&self) -> bool {
		self.snap.mode == 3 && !self.snap.no_present
	}

	/// The front end drawn by the GS itself (title_boot::gs_picture): a movie's held last picture gives way to the GS picture at
	/// once, as the mirror's own scene does (front_mirror.rs `fm_movie_hold`), except under the New Game intro (mode 6) fades.
	/// The front snapshot always holds (no present word there), so without this the held picture stayed up for good.
	pub fn gs_front_drops_movie(&self) -> bool {
		title_boot::gs_picture() && self.snap.front && !self.snap.front_ram.as_ref().is_some_and(|f| f.dialog.mode == 6)
	}

	/// Retail presents no new picture this field (`Snap::hold`): the viewer keeps the last one (a movie's last picture too).
	pub fn holds(&self) -> bool {
		self.snap.hold
	}

	/// Puts the scene actors' sequences of the last field (`scene_seq`) into the viewer's class animations at the slots
	/// retail appended them to (class 0 = the hero's), as scene_chunk_parse does to the class tables; after the scene the
	/// viewer's own sequences come back (retail's clocks only reach a scene slot during the scene).
	pub fn scene_seqs(&mut self, anims: &mut HashMap<u32, (crate::mobies::MobyClass, crate::anim::Animated)>, mut hero: Option<&mut (crate::mobies::MobyClass, crate::anim::Animated)>) {
		if std::env::var_os("RAC_SCENE_LOG").is_some() && self.snap.mode != 0 && self.fields % 10 == 0 {
			eprintln!("retail: view field {} level {} mode {} ship {} frozen {} scene {}", self.snap.field, self.snap.level, self.snap.mode, self.snap.ship, self.snap.frozen, self.snap.scene);
			if self.snap.mode == 6 {
				let mut cls: Vec<(u32, bool)> = self.snap.mobys.iter().map(|m| (m.class, m.shown)).collect();
				cls.sort_unstable();
				cls.dedup();
				eprintln!("retail: view field {} classes (class, shown) {cls:?}", self.snap.field);
			}
		}
		if std::env::var_os("RAC_SCENE_LOG").is_some() && (self.snap.mode != 0 || self.snap.screen.2 > 0.0) {
			eprintln!("retail: screen field {} mode {} skip/mask/fade/white/gate {:?} pmode/dispfb/bg {:x?}", self.snap.field, self.snap.mode, self.snap.screen, self.snap.crtc);
		}
		if std::env::var_os("RAC_SCENE_LOG").is_some() && self.snap.scene && self.fields % 50 == 0 {
			let h = self.snap.hero.map(|(p, y, c)| (p, y, c.cur, c.next, c.old_seq, c.new_seq, c.p));
			eprintln!("retail: scene field {} seqs {:?} hero {h:?} shown {}", self.snap.field, self.snap.scene_seqs.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>(), self.snap.hero_shown);
		}
		if self.snap.level != self.level || self.snap.hold {
			return;
		}
		SCENE_SEQS.with(|c| {
			let mut c = c.borrow_mut();
			if !self.snap.scene {
				for ((class, slot), q) in std::mem::take(&mut c.orig) {
					let a = if class == 0 { hero.as_deref_mut() } else { anims.get_mut(&class) };
					if let (Some((_, a)), Some(q)) = (a, q) {
						if let Some(s) = a.sequences.get_mut(slot as usize) {
							*s = q;
						}
					}
				}
				c.done.clear();
				return;
			}
			for (class, slot, b) in &self.snap.scene_seqs {
				let a = if *class == 0 { hero.as_deref_mut() } else { anims.get_mut(class) };
				let Some((_, a)) = a else { continue };
				let i = *slot as usize;
				if std::env::var_os("RAC_SCENE_LOG").is_some() && !c.done.contains_key(&(*class, *slot)) {
					eprintln!("retail: scene seq class {class} slot {slot} len {} bytes {}", a.sequences.len(), b.len());
				}
				let have = a.sequences.get(i).is_some_and(|s| !s.frames.is_empty());
				if have && c.done.get(&(*class, *slot)) == Some(b) {
					continue;
				}
				match crate::anim::parse_sequence_frames(b, b, a.skeleton.parents.len()) {
					Ok(q) => {
						if a.sequences.len() <= i {
							a.sequences.resize_with(i + 1, || crate::anim::Sequence { frames: Vec::new(), speed: 0.0, sphere: [0.0; 4], triggers: Vec::new(), loop_sound: 0xff });
						}
						let old = std::mem::replace(&mut a.sequences[i], q);
						c.orig.entry((*class, *slot)).or_insert(Some(old));
						c.done.insert((*class, *slot), b.clone());
					}
					Err(e) => eprintln!("retail: scene sequence of class {class} slot {slot}: {e}"),
				}
			}
		});
	}

	/// The hero moby's animation clock of the last field.
	pub fn hero_clock(&self) -> Option<Clock> {
		self.snap.hero.map(|h| h.2)
	}

	/// The +0x64 joint override records of the moby prop `i` draws (empty when none or not a retail prop).
	pub fn prop_fx(&self, i: usize) -> &[crate::anim::JointFx] {
		self.prop_fx.get(&i).map_or(&[], |v| v.as_slice())
	}

	/// The hero's held wrench, back objects, joint records and the loose wrenches retail shows (with the 16:9 side
	/// widening of `sync`) of the last gameplay field; None while frozen or while the level rac-rs shows is not retail's
	/// (the caller keeps the last).
	pub fn hero_parts(&self) -> Option<HeroParts> {
		if self.snap.frozen || self.snap.level != self.level || self.snap.hero.is_none() {
			return None;
		}
		let wide = !cfg!(test) || std::env::var_os("RAC_WIDE_OFF").is_none();
		let mut p = self.snap.parts.clone();
		let lag = self.view_lag();
		p.loose.retain(|l| l.shown || (wide && l.side_only.is_some_and(|s| s.inside_lag(self.aspect, lag))));
		Some(p)
	}

	/// M3a shadows: retail's shadow records of the last drawn frame, the props on the deferred draw path (drawn after the
	/// volumes) and whether the hero is on it; None while the level rac-rs shows is not retail's.
	pub fn shadows(&self) -> Option<(Vec<crate::shadows::RecCaster>, Vec<usize>, bool)> {
		if self.snap.level != self.level {
			return None;
		}
		let s = self.snap.smooth.as_ref().and_then(|d| d.shadows.as_ref()).or(self.snap.shadows.as_ref())?;
		let late = s.deferred.iter().filter_map(|a| self.held.get(a).copied()).collect();
		Some((s.casters.iter().map(|c| c.1.clone()).collect(), late, s.hero_deferred))
	}

	/// Mirrors the last field's moby table into the game's props and the hero moby into the player.
	/// The nanotech orbs retail queued for drawing in the last gameplay field (`orb_draws`); empty while the level
	/// rac-rs shows is not retail's (travel).
	pub fn orb_draws(&self) -> &[crate::particles::OrbDraw] {
		match self.snap.smooth.as_ref().and_then(|d| d.orbs.as_ref()).or(self.snap.orbs.as_ref()) {
			Some(o) if self.snap.level == self.level => o,
			_ => &[],
		}
	}

	/// The effect pool and draw-queue extras of the last gameplay field (`queue_fx`); None while the level rac-rs shows
	/// is not retail's.
	/// The vendor shop mobys of the last field (`shop_snap`), None outside the shop screen.
	pub fn shop(&self) -> Option<&ShopSnap> {
		self.snap.shop.as_ref().filter(|_| self.snap.level == self.level)
	}

	/// [PC] PREVIEW (viewer/vegfx.rs touch): the mobys of the last field, empty while the level rac-rs shows is not retail's.
	pub fn live_mobys(&self) -> &[Moby] {
		if self.snap.level == self.level { &self.snap.mobys } else { &[] }
	}

	/// [PC] The shown field's own effect lists as the game ticked them, before Smoothing blends them with the field before (`fx`).
	pub fn fx_tick(&self) -> Option<&RetailFx> {
		self.snap.fx.as_ref().filter(|_| self.snap.level == self.level)
	}

	pub fn fx(&self) -> Option<&RetailFx> {
		match self.snap.smooth.as_ref().and_then(|d| d.fx.as_ref()).or(self.snap.fx.as_ref()) {
			Some(f) if self.snap.level == self.level => Some(f),
			_ => None,
		}
	}

	/// [PC] Frame smoothing (viewer/interp.rs): the shown field's own lists (fx, shadow records, orbs) while rac-rs shows retail's
	/// level, and (level, game mode, scene view, frozen or held) of the shown field.
	pub fn smooth_src(&self) -> (Option<&RetailFx>, Option<&RetailShadows>, Option<&[crate::particles::OrbDraw]>, (u32, u32, bool, bool)) {
		let mine = self.snap.level == self.level;
		(self.snap.fx.as_ref().filter(|_| mine), self.snap.shadows.as_ref().filter(|_| mine), self.snap.orbs.as_deref().filter(|_| mine), (self.snap.level, self.snap.mode, self.snap.scene, self.snap.frozen || self.snap.hold || self.snap.front))
	}

	/// [PC] Frame smoothing: the blended lists this display frame draws (None = the field's own).
	pub fn set_smooth(&mut self, d: Option<SmoothDraw>) {
		self.snap.smooth = d.map(Box::new);
	}

	pub fn sync(&mut self, g: &mut Game) {
		if !self.bound {
			self.bind(g);
		}
		// The pause menu (mode 3) borrows mobys for its own scene while gameplay is frozen: keep the last frame.
		if self.snap.hold || self.snap.level != self.level {
			return;
		}
		// A scene with actors draws its own: its Ratchet (when retail draws it), Clank as an actor of its own, no held
		// item. A mode-2 field without actors (a scripted camera over gameplay) keeps the placed hero as it is.
		SCENE_SEQS.with(|c| {
			let mut c = c.borrow_mut();
			if self.snap.scene && !self.snap.scene_seqs.is_empty() {
				c.saved.get_or_insert((g.hero_hidden, g.player.clank_hidden, g.held_hidden));
				(g.hero_hidden, g.player.clank_hidden, g.held_hidden) = (!self.snap.hero_shown, true, true);
			} else if let Some(f) = c.saved.take() {
				(g.hero_hidden, g.player.clank_hidden, g.held_hidden) = f;
			}
		});
		g.health = self.snap.health;
		g.bolts = self.snap.bolts;
		if self.snap.have.len() == crate::inventory::GADGETS {
			g.inv.have.copy_from_slice(&self.snap.have);
			g.inv.ammo.copy_from_slice(&self.snap.ammo);
			(g.inv.equipped, g.inv.last) = (self.snap.equipped, self.snap.last);
		}
		// rac-rs's own story popups (planet unlocks at game start) are not retail's.
		g.popups.clear();
		// NOV-30g: retail hides the hero (and his parts) in the shop; the marker follows `hero_hidden`.
		// [merge M7] A scene with actors keeps its own choice (SCENE_SEQS above).
		if !(self.snap.scene && !self.snap.scene_seqs.is_empty()) {
			g.hero_hidden = self.snap.hero_hidden;
		}
		if let Some((pos, yaw, _)) = self.snap.hero {
			g.player.pos = pos;
			g.player.yaw = yaw;
			g.player.body_tilt = self.snap.hero_tilt;
		}
		// The held moby drives the viewer's held rigs (game/pyro.rs `mirror_held`); a moby a rig took is not a prop. The Bomb
		// Glove (192) has a viewer rig of its own (viewer.rs GLOVE_CLASS) and a prop pool for the purchase scene: its held moby
		// stays out of the props too.
		let held_addr = match self.snap.held {
			Some((a, c, clock)) => (g.mirror_held(Some(c), |p| clock.apply(p)) || c == 192).then_some(a),
			None => {
				g.mirror_held(None, |_| ());
				None
			}
		};
		let mut seen = HashSet::new();
		self.prop_fx.clear();
		// Live mobys per class that found no free prop: (live count, pool) for `grow`.
		let mut short: HashMap<u32, u32> = HashMap::new();
		// Tests only: RAC_WIDE_OFF=1 draws retail's 4:3 verdict as it was (the before pictures of scratch_widecull).
		let wide = !cfg!(test) || std::env::var_os("RAC_WIDE_OFF").is_none();
		let lag = self.view_lag();
		for mb in self.snap.mobys.iter().filter(|mb| Some(mb.addr) != held_addr) {
			let held = self.held.get(&mb.addr).copied().filter(|&i| g.props[i].class_id == mb.class);
			if held.is_none() {
				if let Some(i) = self.held.remove(&mb.addr) {
					release(g, &mut self.free, i);
				}
			}
			if let Some(i) = held.or_else(|| self.free.get_mut(&mb.class).and_then(|v| v.pop())) {
				self.held.insert(mb.addr, i);
				seen.insert(mb.addr);
				let p = &mut g.props[i];
				p.pos = mb.pos;
				p.rot = mb.rot;
				p.mirror = mb.mirror;
				p.scale = mb.scale;
				// Retail's verdict stays as it is in RAM (gameplay reads it); rac-rs's wide window also shows what retail culled only
			// at its 4:3 side plane.
			p.shown = mb.shown || (wide && mb.side_only.is_some_and(|s| s.inside_lag(self.aspect, lag)));
				p.light = Some((mb.rgb, mb.light));
				p.set_retail_draw(mb.alpha, mb.ovr, mb.draw);
				if dbg_class() == Some(mb.class as u32) {
					eprintln!("DBGCLASS f{} {} {:#x} slot {i} rgb {:?} light {:#x} alpha {:#x} draw {} ovr {:?} shown {} scale {}", self.fields, mb.class, mb.addr, mb.rgb, mb.light, mb.alpha, mb.draw, mb.ovr, mb.shown, mb.scale);
				}
				mb.clock.apply(&mut p.anim);
				if !mb.fx.is_empty() {
					self.prop_fx.insert(i, mb.fx.clone());
				}
			} else if modelled(mb.class) {
				short.insert(mb.class, 0);
			}
		}
		if !short.is_empty() && self.grow.is_none() {
			for mb in &self.snap.mobys {
				if let Some(n) = short.get_mut(&mb.class) {
					*n += 1;
				}
			}
			let mut p: HashMap<u32, u32> = pools().into_iter().collect();
			for (&c, &need) in &short {
				let e = p.entry(c).or_insert(0);
				crate::loadlog::later(format!("retail: pool of class {c} full ({} props, {need} live): rebuilding the prop slots", *e));
				*e = grown(c, *e, need);
			}
			let mut p: Vec<(u32, u32)> = p.into_iter().collect();
			p.sort();
			self.grow = Some(p);
		}
		let gone: Vec<(u32, usize)> = self.held.iter().filter(|(a, _)| !seen.contains(a)).map(|(&a, &i)| (a, i)).collect();
		for (a, i) in gone {
			self.held.remove(&a);
			release(g, &mut self.free, i);
		}
	}
}

impl Drop for Retail {
	fn drop(&mut self) {
		// A game thread waiting in the movie hook must come out (it skips every later movie).
		fmv_bridge::abort();
		// While recording, wait for the thread's last field so a following load's `!LOAD` line comes after it.
		if rec::recording() {
			drop(std::mem::replace(&mut self.tx, std::sync::mpsc::channel().0));
			if let Some(w) = self.worker.take() {
				let _ = w.join();
			}
		}
	}
}

/// `from-to:buttons:lx,ly[:rx,ry]` (hex, `;` between entries): the optional fourth part is the right stick (the retail camera).
fn pad_script() -> Vec<(u32, u32, u16, [u8; 2], Option<[u8; 2]>)> {
	let v = std::env::var("RAC_RETAIL_PAD").unwrap_or_default();
	let hex = |t: &str| u32::from_str_radix(t.trim(), 16).ok();
	v.split(';')
		.filter_map(|e| {
			let mut f = e.split(':');
			let (r, b, st) = (f.next()?, f.next()?, f.next()?);
			let (a, z) = r.split_once('-')?;
			let (x, y) = st.split_once(',')?;
			let rs = f.next().and_then(|t| t.split_once(',')).and_then(|(x, y)| Some([hex(x)? as u8, hex(y)? as u8]));
			Some((a.trim().parse().ok()?, z.trim().parse().ok()?, hex(b)? as u16, [hex(x)? as u8, hex(y)? as u8], rs))
		})
		.collect()
}

/// One scripted key tap of RAC_RETAIL_KEYTAP: `KEY@WHEN+N;...`, KEY a winit name (Space, Escape, Enter; for menus Backspace, Tab,
/// Up, Down, Left, Right, Q, E, W, S, T), WHEN `abs` (always),
/// `movie` (a movie is playing; `movieN`: the Nth movie of the run), `scene` (game mode 2) or `ship` (mode 6); the key is tapped (`press_key`) once WHEN has held
/// for N consecutive polls of `tick_keys` (one per 20 ms field slot, also while a movie parks the game).
struct KeyTap {
	key: winit::keyboard::KeyCode,
	when: String,
	n: u32,
	seen: u32,
	fired: bool,
}

fn keytap_script() -> Vec<KeyTap> {
	use winit::keyboard::KeyCode as K;
	let v = std::env::var("RAC_RETAIL_KEYTAP").unwrap_or_default();
	v.split(';')
		.filter_map(|e| {
			let (k, w) = e.trim().split_once('@')?;
			let (when, n) = w.split_once('+')?;
			let key = match k {
				"Space" => K::Space,
				"Escape" => K::Escape,
				"Enter" => K::Enter,
				// Menu keys (menu_keys.rs) for scripted menu runs.
				"Backspace" => K::Backspace,
				"Tab" => K::Tab,
				"Up" => K::ArrowUp,
				"Down" => K::ArrowDown,
				"Left" => K::ArrowLeft,
				"Right" => K::ArrowRight,
				"Q" => K::KeyQ,
				"E" => K::KeyE,
				"W" => K::KeyW,
				"S" => K::KeyS,
				"T" => K::KeyT,
				// A right mouse click (tests of the menus' mouse back).
				"RMB" => K::F24,
				// Any key the Controls page can bind, by its options-file name (KeyK, Digit5, ...): remap tests.
				_ => binds::key_from_name(k)?,
			};
			Some(KeyTap { key, when: when.to_string(), n: n.parse().ok()?, seen: 0, fired: false })
		})
		.collect()
}

fn release(g: &mut Game, free: &mut HashMap<u32, Vec<usize>>, i: usize) {
	g.props[i].shown = false;
	free.entry(g.props[i].class_id).or_default().push(i);
}

/// The PS2 Start bit (the retail pad bits): the skip press on a gamepad, like Space on the keyboard.
const START: u16 = 0x800;

/// One poll of the skip test of a playing movie: (the rule ends it now, the pad to remember for the next poll).
/// `pad` is the gamepad's own buttons (`prompts::pad_buttons`): its Start is the skip press, like Space on the keyboard.
fn movie_poll(rule: fmv_bridge::Skip, prev: u16, held: impl Fn(winit::keyboard::KeyCode) -> bool, pad: u16) -> (bool, u16) {
	let skip_key = held(winit::keyboard::KeyCode::Escape) || held(winit::keyboard::KeyCode::Space);
	// The gamepad's Start is the skip press on a gamepad, like Space on the keyboard.
	let pad_start = pad & START != 0;
	let mut now = pad_from_keys(held, false, false).0 | pad;
	// Esc and Space are the player's skip keys (user request 2026-10-13), and so is the gamepad's Start: they feed the pad the
	// player's own skip test wants (`Skip::feed`), so retail's rule decides and a button held from before the movie still
	// counts for nothing.
	if skip_key || pad_start {
		now |= rule.feed();
	}
	// The logo movies (the Insomniac intro at the start of a cold boot) have no skip in retail (`Skip::Never`): the port lets
	// the skip press end them, so the intro is not in the way.
	let hit = rule.pressed(prev, now)
		|| (rule == fmv_bridge::Skip::Never && (skip_key || pad_start) && (now & !prev) != 0);
	(hit, now)
}

/// The pad bits Esc / Space add while a scene or a ship flight plays, so retail's own skip test sees its press: the
/// in-engine scene controller (game mode 2; Veldin FUN_00298938, twin per level, Novalis FUN_002ac3b0) skips on a new
/// Start, with L1 + R1 + L2 + R2 held as well in a normal run past Veldin (the same rule as the movie player); the ship
/// flight (mode 6, substates 0 takeoff and 8 landing; Novalis FUN_002a39b0) skips on a new Triangle or Cross.
pub fn skip_feed(mode: u32, ship: u32, skip_key: bool) -> u16 {
	if !skip_key {
		return 0;
	}
	match mode {
		2 => 0x80f,
		6 if matches!(ship, 0 | 8) => 0x40,
		_ => 0,
	}
}

/// rac-rs's keyboard and mouse as a DualShock report: (buttons pressed, [rx, ry, lx, ly]). By default WASD = left stick, Q/E Z/X =
/// right stick, Space = Cross, F = Square, G or the left mouse button (`circle`) = Circle (fire the weapon), T or the middle
/// mouse button (`triangle`) = Triangle (Quick Select), Ctrl = R1, C = L1, V = L2, B = R2, Esc/Enter = Start, M = Select, Left
/// Shift = L3, R = R3, arrows = d-pad; Pause -> Options -> Controls remaps them (`binds::kb`, the defaults with the extension off).
pub fn pad_from_keys(held: impl Fn(winit::keyboard::KeyCode) -> bool, circle: bool, triangle: bool) -> (u16, [u8; 4]) {
	binds::kb().pad(held, circle, triangle)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The skip rule on the keyboard, as the gameplay path and the front-end mirror both poll it (`movie_poll`): the logos
	/// never skip, the attract movie ends on any new key, the New Game intro and the story movies on a new Enter (Start) or
	/// on Esc / Space (the player's skip keys, which feed Start), and past Veldin Start alone does nothing: retail wants
	/// Start with C, Left Ctrl, V and B (L1 R1 L2 R2) held, which Esc / Space supply together.
	#[test]
	fn movie_skip_rule_keyboard() {
		use fmv_bridge::Skip;
		use winit::keyboard::KeyCode as K;
		let poll = |rule: Skip, prev: u16, keys: &[K]| movie_poll(rule, prev, |k| keys.contains(&k), 0);
		// Boot logos (mode -1): the player's skip keys end them (the port lets them skip).
		assert!(poll(Skip::Never, 0, &[K::Escape]).0, "Esc skips the logos");
		assert!(poll(Skip::Never, 0, &[K::Space]).0, "Space skips the logos");
		assert!(!poll(Skip::Never, 0, &[K::KeyF]).0, "other keys do not skip the logos");
		assert!(!poll(Skip::Never, 0x800, &[K::Escape]).0, "Start held from before the movie");
		// Attract movie (mode 2): any new button, but not one already down at the last poll.
		assert!(poll(Skip::Any, 0, &[K::Space]).0);
		assert!(poll(Skip::Any, 0, &[K::KeyF]).0);
		assert!(!poll(Skip::Any, 0x40, &[K::Space]).0, "held since before");
		// Everything else (level word 0: New Game intro; Start rule): a new Esc / Enter / Numpad Enter / Space.
		for start in [K::Escape, K::Enter, K::NumpadEnter] {
			assert_eq!(poll(Skip::Start, 0, &[start]), (true, 0x800), "{start:?} = Start");
		}
		assert!(poll(Skip::Start, 0, &[K::Space]).0, "Space is a skip key");
		assert!(!poll(Skip::Start, 0, &[K::KeyF]).0, "other keys are no skip");
		assert!(!poll(Skip::Start, 0x800, &[K::Escape]).0, "Start held from before the movie");
		assert!(!poll(Skip::Start, 0x840, &[K::Space]).0, "Space held from before the movie");
		// A press polled twice skips once: the second poll sees it already down.
		let (hit, prev) = poll(Skip::Start, 0, &[K::Escape]);
		assert!(hit && !poll(Skip::Start, prev, &[K::Escape]).0);
		// Past Veldin in a normal run (combo rule): Start alone (Enter) does nothing, with all four shoulder keys it skips.
		let four = [K::KeyC, K::ControlLeft, K::KeyV, K::KeyB];
		assert_eq!(pad_from_keys(|k| four.contains(&k), false, false).0, 0xf, "L1 R1 L2 R2");
		assert!(!poll(Skip::Combo, 0, &[K::Enter]).0);
		for missing in 0..4 {
			let mut keys: Vec<K> = four.iter().enumerate().filter(|(i, _)| *i != missing).map(|(_, k)| *k).collect();
			keys.push(K::Enter);
			assert!(!poll(Skip::Combo, 0, &keys).0, "only three shoulder keys");
		}
		let mut keys = four.to_vec();
		keys.push(K::Enter);
		assert!(poll(Skip::Combo, 0, &keys).0);
		assert!(!poll(Skip::Combo, 0xf | 0x800, &keys).0, "Start held from before");
		// The shoulder keys pressed after Start do not skip, the Start press that completes the combo does.
		let (hit, prev) = poll(Skip::Combo, 0, &[K::Enter]);
		assert!(!hit && !poll(Skip::Combo, prev, &keys).0, "Start was already down when the shoulders came");
		// Esc / Space feed the whole combo, once per press.
		for k in [K::Escape, K::Space] {
			let (hit, prev) = poll(Skip::Combo, 0, &[k]);
			assert!(hit && !poll(Skip::Combo, prev, &[k]).0, "{k:?} skips a combo movie once");
			assert!(!poll(Skip::Combo, 0x80f, &[k]).0, "{k:?} held from before");
		}
	}

	/// The gamepad's Start is the skip press on a gamepad, like Space on the keyboard: it skips the logos, the New Game intro
	/// and the story movies, and it feeds the whole combo. A Start held from before the movie counts for nothing.
	#[test]
	fn movie_skip_rule_gamepad() {
		use fmv_bridge::Skip;
		let poll = |rule: Skip, prev: u16, pad: u16| movie_poll(rule, prev, |_| false, pad);
		// The logos (mode -1): the gamepad's Start ends them, like Space does.
		assert!(poll(Skip::Never, 0, START).0, "Start skips the logos");
		assert!(!poll(Skip::Never, 0, 0).0, "no button, no skip");
		assert!(!poll(Skip::Never, START, START).0, "Start held from before the movie");
		// The New Game intro (Start rule): the gamepad's Start skips it.
		assert!(poll(Skip::Start, 0, START).0);
		assert!(!poll(Skip::Start, 0, 0x40).0, "Cross alone is no skip");
		assert!(!poll(Skip::Start, START, START).0, "Start held from before the movie");
		// The story movies (combo rule): the gamepad's Start feeds the whole combo, like Esc / Space.
		assert!(poll(Skip::Combo, 0, START).0);
		assert!(!poll(Skip::Combo, 0x80f, START).0, "Start held from before the movie");
		// A press polled twice skips once: the second poll sees it already down.
		let (hit, prev) = poll(Skip::Start, 0, START);
		assert!(hit && !poll(Skip::Start, prev, START).0);
	}

	/// Esc / Space during a scene (mode 2) press Start with the four shoulder buttons (retail's scene skip test), during a
	/// ship takeoff or landing (mode 6, substate 0 / 8) Cross (its skip test is a new Triangle or Cross); nothing elsewhere.
	#[test]
	fn skip_keys_feed_scene_and_flight() {
		assert_eq!(skip_feed(2, 0, true), 0x80f);
		assert_eq!(skip_feed(2, 0, false), 0);
		assert_eq!(skip_feed(6, 0, true), 0x40);
		assert_eq!(skip_feed(6, 8, true), 0x40);
		assert_eq!(skip_feed(6, 3, true), 0, "flight away: no skip");
		for mode in [0, 1, 3, 4, 5, 7] {
			assert_eq!(skip_feed(mode, 0, true), 0, "mode {mode}");
		}
	}

	/// Mouse in retail play: left = Circle (0x20), middle = Triangle (0x10), neither Square (0x80); the keyboard keeps F =
	/// Square, G = Circle, T = Triangle; a click latched by `press_mouse` outlives the release for `TAP_FIELDS` fields.
	#[test]
	fn mouse_buttons_are_circle_and_triangle() {
		use winit::keyboard::KeyCode as K;
		let pad = |k: Option<K>, c: bool, t: bool| pad_from_keys(|x| Some(x) == k, c, t).0;
		assert_eq!(pad(None, true, false), 0x20, "LMB = Circle");
		assert_eq!(pad(None, false, true), 0x10, "MMB = Triangle");
		assert_eq!(pad(None, true, true), 0x30);
		assert_eq!(pad(None, false, false), 0);
		assert_eq!(pad(Some(K::KeyF), false, false), 0x80, "F = Square");
		assert_eq!(pad(Some(K::KeyG), false, false), 0x20, "G = Circle");
		assert_eq!(pad(Some(K::KeyT), false, false), 0x10, "T = Triangle");
		assert_eq!(pad(Some(K::KeyG), true, false), 0x20);
		assert_eq!(pad(Some(K::Space), true, true), 0x70, "Space = Cross");
		// A click down and up between two samples: held for the next fields (tick_keys reads the latch, then ticks).
		let mut m = MouseTaps::default();
		m.press(0);
		m.press(1);
		for f in 0..TAP_FIELDS {
			assert!(m.0[0] > 0 && m.0[1] > 0, "tap lost at field {f}");
			let (b, _) = pad_from_keys(|_| false, m.0[0] > 0, m.0[1] > 0);
			assert_eq!(b, 0x30);
			m.tick();
		}
		assert_eq!(m.0, [0, 0]);
		m.press(2);
		assert_eq!(m.0, [0, 0], "no third button");
	}

	/// Stage I2 bisect (manual): the acceptance oracle runner's loop on RAC_TEST_STATE with the Retail::load extras
	/// switched on one by one (RAC_BIS = letters: f fmv_bridge::register, r record_sound, p set_pad
	/// neutral before the feed, c memcard Saver).
	#[test]
	#[ignore]
	fn retail_bisect() {
		let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/play/eudora1_0022").into());
		let n: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
		let bis = std::env::var("RAC_BIS").unwrap_or_default();
		let mut m = ee_oracle::Machine::load(&dir);
		if bis.contains('r') {
			m.record_sound();
		}
		ee_game::register();
		if bis.contains('f') {
			fmv_bridge::reset();
			fmv_bridge::register();
		}
		m.enable_rec();
		let feed = playfeed::open(Some(playfeed::MARK), false, &dir, 0, "ACC_ORACLE_NO_TRAIL", playfeed::code_hash(include_str!("../../oracle/pcsx2rec/playfeed.rs"))).expect("feed");
		let mut card = memcard::Saver::new(&m);
		for f in 0..n {
			if bis.contains('p') {
				m.set_pad(0, [0x80; 4]);
			}
			feed.apply(&mut m.cpu, f);
			if let Err(e) = m.frame() {
				panic!("bisect {bis:?}: stop at field {f}: {e}");
			}
			if bis.contains('c') {
				card.after_field(&m);
			}
			let _ = take_sound_of(&mut m);
		}
		eprintln!("bisect {bis:?}: {n} fields ok");
	}

	/// Stage I2 (manual): from RAC_TEST_STATE (default fg1/4_0159) request travel to RAC_TEST_TRAVEL (default 5, Rilgar)
	/// the way the level-end helper does, service any movie (ended at once), wait for the new level's gameplay, run
	/// RAC_TEST_FIELDS (600) more fields and report the memory-card writes (RAC_PLAYER_DIR must be a scratch dir).
	#[test]
	#[ignore]
	fn retail_travel_save() {
		let pdir = std::env::var("RAC_PLAYER_DIR").expect("set RAC_PLAYER_DIR to a scratch dir");
		assert!(pdir.contains("scratch"), "RAC_PLAYER_DIR must be a scratch dir");
		let to: u32 = std::env::var("RAC_TEST_TRAVEL").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
		unsafe {
			std::env::set_var("RAC_RETAIL_HUD", "0");
			std::env::set_var("RAC_RETAIL_TRAVEL", format!("{to}:10"));
		}
		let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/fg1/4_0159").into());
		let after: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
		enable(&dir).unwrap();
		let mut r = Retail::load().unwrap();
		fmv_bridge::set_serviced(true);
		let (mut arrived, mut modes, mut movies) = (None, Vec::<(u32, u32, u32)>::new(), Vec::new());
		while !r.failed && r.fields < 6000 && arrived.is_none_or(|a| r.fields < a + after) {
			r.tick(0, [0x80; 4]);
			if let Some(q) = r.take_movie() {
				movies.push((r.fields, q));
				r.end_movie();
			}
			if modes.last().map(|x| (x.1, x.2)) != Some((r.snap.level, r.snap.mode)) {
				modes.push((r.fields, r.snap.level, r.snap.mode));
			}
			if let Some((l, pools)) = r.switch() {
				eprintln!("travel_save: field {} switch to level {l}, {} pools, hero {:?}", r.fields, pools.len(), r.snap.hero.map(|h| h.0));
				arrived = Some(r.fields);
			}
		}
		let card = std::path::Path::new(&pdir).join("card.bin");
		eprintln!("travel_save: fields {} (field, level, mode) changes {modes:?}; movies {movies:?}; stop {:?}; card {} {:?}", r.fields, r.stop, card.display(), std::fs::metadata(&card).map(|m| m.len()).ok());
		assert!(r.stop.is_none() && !r.failed, "oracle stopped: {:?}", r.stop);
		let a = arrived.expect("no travel");
		assert_eq!(r.snap.level, to);
		assert!(r.fields >= a + after);
	}

	/// Stage I2 (manual): a fresh `Retail::load` restores RAC_PLAYER_DIR/card.bin; prints the card-0 tree of the base
	/// image and of the restored card (files that differ are the player's saves) and runs RAC_TEST_FIELDS fields.
	#[test]
	#[ignore]
	fn retail_card_restore() {
		let pdir = std::env::var("RAC_PLAYER_DIR").expect("set RAC_PLAYER_DIR to a scratch dir");
		assert!(pdir.contains("scratch"), "RAC_PLAYER_DIR must be a scratch dir");
		unsafe { std::env::set_var("RAC_RETAIL_HUD", "0") };
		let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/fg1/4_0159").into());
		let bytes = std::fs::read(std::path::Path::new(&pdir).join("card.bin")).expect("no card.bin");
		fn walk(e: &ee_oracle::mc::Ent, path: &str, out: &mut Vec<(String, u32, [u8; 8], u64)>) {
			for k in &e.kids {
				let p = format!("{path}/{}", k.name);
				let h = k.data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100_0000_01b3));
				out.push((p.clone(), k.len, k.modified, h));
				walk(k, &p, out);
			}
		}
		let base = ee_oracle::Machine::load(&dir);
		let mut fresh = ee_oracle::Machine::load(&dir);
		assert!(memcard::restore_bytes(&mut fresh, &bytes));
		let (mut b, mut f) = (Vec::new(), Vec::new());
		// The state's card is read only at the first card call: compare with the base image itself then.
		let img = ee_oracle::mc::image_path(0).and_then(|f| std::fs::read(f).ok()).and_then(|i| ee_oracle::mc::Card::parse(&i).ok());
		if let Some(c) = base.cpu.mc.cards[0].as_ref().or(img.as_ref()) {
			walk(&c.root, "", &mut b);
		}
		if let Some(c) = &fresh.cpu.mc.cards[0] {
			walk(&c.root, "", &mut f);
		}
		for e in &f {
			let tag = match b.iter().find(|x| x.0 == e.0) {
				None => "NEW",
				Some(x) if x.3 != e.3 || x.2 != e.2 => "CHANGED",
				Some(_) => "same",
			};
			eprintln!("card_restore: {tag:7} {} len {} modified {:02x?} data {:016x}", e.0, e.1, e.2, e.3);
		}
		eprintln!("card_restore: base loaded {} files {}, restored loaded {} files {}", base.cpu.mc.loaded, b.len(), fresh.cpu.mc.loaded, f.len());
		drop((base, fresh));
		enable(&dir).unwrap();
		let mut r = Retail::load().unwrap();
		let n: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
		while r.fields < n && !r.failed {
			r.tick(0, [0x80; 4]);
		}
		eprintln!("card_restore: Retail ran {} fields after the restore, stop {:?}, level {} mode {}", r.fields, r.stop, r.snap.level, r.snap.mode);
		assert!(r.stop.is_none() && !r.failed);
	}

	/// Stage I2 (manual): from fg1/4_0159, keyboard keys through `tick_keys` (the viewer's path, no input injection):
	/// RAC_TEST_IDLE (960) + 30 idle fields, then W (stick forward) for 90, D for 60, Space (jump) taps; the hero must move.
	#[test]
	#[ignore]
	fn retail_keyboard_moves() {
		use winit::keyboard::KeyCode as K;
		unsafe {
			std::env::set_var("RAC_RETAIL_HUD", "0");
			std::env::set_var("RAC_PLAYER_CARD", "0");
		}
		let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/fg1/4_0159").into());
		enable(&dir).unwrap();
		let mut r = Retail::load().unwrap();
		// In this save the hero stands locked (hero state 114) until field ~897 in the PCSX2 recording too (fullgame1
		// ends.tsv: hstate 114 -> 0 at vb 253442 with only stick input before it); keys start after RAC_TEST_IDLE fields.
		let idle: u32 = std::env::var("RAC_TEST_IDLE").ok().and_then(|v| v.parse().ok()).unwrap_or(960);
		let mut track = Vec::new();
		for k in 0..240u32 + idle {
			let key = match k.saturating_sub(idle) {
				_ if k < idle => None,
				30..120 => Some(K::KeyW),
				120..180 => Some(K::KeyD),
				190..194 => Some(K::Space),
				_ => None,
			};
			r.tick_keys(|c| Some(c) == key, false, false);
			if k >= idle && (k - idle) % 30 == 29 {
				track.push((k + 1, r.snap.mode, r.snap.pad[0], r.snap.hero.map(|h| h.0)));
			}
		}
		for t in &track {
			eprintln!("keyboard: field {} mode {} game pad {:05x} hero {:?}", t.0, t.1, t.2, t.3);
		}
		assert!(r.stop.is_none() && !r.failed);
		let p = |i: usize| track[i].3.unwrap();
		let d = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
		eprintln!("keyboard: idle drift {:.2}, W moved {:.2}, D moved {:.2}", d(p(0), p(1)), d(p(0), p(3)), d(p(3), p(5)));
		assert!(d(p(0), p(3)) > 1.0, "W did not move the hero");
		assert!(d(p(3), p(5)) > 1.0, "D did not move the hero");
	}

	/// Manual (one game run): fg1/1_0018 (Novalis). Start opens the pause menu; the mouse hovers every root button through
	/// the layer (the game's own selection must follow), clicks into each page, hovers inside it, and right-clicks back.
	/// Everything goes through `mouse_*` + `tick_keys` (the viewer's path); the pages visited are printed for the handoff.
	#[test]
	#[ignore]
	fn retail_mouse_menus() {
		use super::mouse_menu::{Kind, MenuSnap, Target};
		use winit::keyboard::KeyCode as K;
		unsafe {
			std::env::set_var("RAC_RETAIL_HUD", "0");
			std::env::set_var("RAC_PLAYER_CARD", "0");
		}
		let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/fg1/1_0018").into());
		enable(&dir).unwrap();
		let mut r = Retail::load().unwrap();
		r.aspect = 16.0 / 9.0;
		let idle: u32 = std::env::var("RAC_TEST_IDLE").ok().and_then(|v| v.parse().ok()).unwrap_or(200);
		for _ in 0..idle {
			r.tick_keys(|_| false, false, false);
		}
		eprintln!("mouse: level {} mode {} after {idle} idle fields", r.snap.level, r.snap.mode);
		fn pump(r: &mut Retail, n: u32) {
			for _ in 0..n {
				r.tick_keys(|_| false, false, false);
			}
		}
		fn until(r: &mut Retail, max: u32, f: impl Fn(&MenuSnap) -> bool) -> bool {
			for _ in 0..max {
				r.tick_keys(|_| false, false, false);
				if r.snap.menu.as_ref().is_some_and(|m| !m.busy() && f(m)) {
					return true;
				}
			}
			false
		}
		// Window fractions of an overlay pixel (the inverse of `to_ps2` at 16:9).
		let k = r.aspect / (4.0 / 3.0);
		let frac = move |x: f32, y: f32| ((x / 512.0 - 0.5) / k + 0.5, y / 448.0);
		let centre = |it: &mouse_menu::Item, sub: u32| -> (f32, f32) {
			let [x, y, w, h] = it.rect.map(|v| v as f32);
			match &it.kind {
				Kind::Label { rows, .. } => (x + w * 0.5, y + rows.get(sub as usize).map_or(h * 0.5, |r| r.0)),
				Kind::Value { n, .. } => (x + w * 0.5, y + (sub as f32 + 1.0) * h / (*n as f32 + 1.0)),
				Kind::Grid { rows, cols, .. } => (x + ((sub % cols) as f32 + 0.5) * w / *cols as f32, y + ((sub / cols) as f32 + 0.5) * h / *rows as f32),
				Kind::Ring { .. } => mouse_menu::ring_slot(it.rect, sub),
				_ => (x + w * 0.5, y + h * 0.5),
			}
		};
		// Start (Enter) opens the pause menu.
		for _ in 0..4 {
			r.tick_keys(|k| k == K::Enter, false, false);
		}
		assert!(until(&mut r, 600, |m| m.id == 2), "pause menu did not open: mode {} menu {:?}", r.snap.mode, r.snap.menu);
		if let Some((w, rows, _)) = &r.snap.ov {
			eprintln!("mouse: overlay {w}x{rows}");
		}
		pump(&mut r, 30);
		let root = r.snap.menu.clone().unwrap();
		eprintln!("mouse: root page {:?}", root);
		assert!(root.items.len() >= 5, "root items: {}", root.items.len());
		let mut report = Vec::new();
		// Hover every root button: the game's selection must reach it.
		let mut items: Vec<_> = root.items.iter().filter(|i| matches!(i.kind, Kind::Label { .. })).cloned().collect();
		items.sort_by_key(|i| i.rect[1]);
		for it in &items {
			let (x, y) = centre(it, 0);
			let (fx, fy) = frac(x, y);
			r.mouse_cursor(fx, fy);
			let from = r.snap.menu.as_ref().map_or(0, |m| m.cur);
			let mut n = 0;
			let ok = loop {
				n += 1;
				r.tick_keys(|_| false, false, false);
				if r.snap.menu.as_ref().is_some_and(|m| !m.busy() && m.cur == it.addr) {
					break true;
				}
				if n >= 150 {
					break false;
				}
			};
			// Fields until the game's selection (as the snap reads it, 2 fields behind) is the hovered item.
			eprintln!("mouse: hover {:#x} at ({x:.0},{y:.0}) from {from:#x} -> cur {:#x} ok {ok} in {n} fields", it.addr, r.snap.menu.as_ref().map_or(0, |m| m.cur));
			assert!(ok, "hover {:#x} did not select it", it.addr);
		}
		// A long jump: from the last root button straight to the first (several rows in one hover), and back.
		for it in [items.first().unwrap(), items.last().unwrap()] {
			let (x, y) = centre(it, 0);
			let (fx, fy) = frac(x, y);
			r.mouse_cursor(fx, fy);
			let mut n = 0;
			while n < 200 && !r.snap.menu.as_ref().is_some_and(|m| !m.busy() && m.cur == it.addr) {
				r.tick_keys(|_| false, false, false);
				n += 1;
			}
			// The selection must stay there (no overshoot past the target).
			pump(&mut r, 10);
			let cur = r.snap.menu.as_ref().map_or(0, |m| m.cur);
			eprintln!("mouse: jump to {:#x}: {n} fields, {} rows, stays {}", it.addr, items.len() - 1, cur == it.addr);
			assert!(n < 200 && cur == it.addr, "jump to {:#x}: {n} fields, now {cur:#x}", it.addr);
		}
		// Click into every page, look around, right-click back.
		for it in &items {
			let (x, y) = centre(it, 0);
			let (fx, fy) = frac(x, y);
			r.mouse_cursor(fx, fy);
			r.mouse_click();
			let mut ok = false;
			for _ in 0..250 {
				r.tick_keys(|_| false, false, false);
				if r.snap.menu.as_ref().is_some_and(|m| m.id != 2) {
					ok = true;
					break;
				}
			}
			let pid = r.snap.menu.as_ref().map_or(0, |m| m.id);
			if !ok {
				report.push(format!("click {:#x}: no new page (still {pid:#x})", it.addr));
				continue;
			}
			pump(&mut r, 60);
			if r.snap.menu.as_ref().is_none_or(|m| m.busy()) {
				report.push(format!("click {:#x}: page {pid:#x} never went idle: {:?}", it.addr, r.snap.menu));
			}
			let pg = r.snap.menu.clone().unwrap();
			eprintln!("mouse: clicked {:#x} -> page {:#x}: {:?}", it.addr, pg.id, pg);
			// Hover each selectable target of the page (rows / cells / slots) and see whether the selection follows.
			let mut seen = 0;
			for sub_it in pg.items.iter().filter(|i| !matches!(i.kind, Kind::Hint(_) | Kind::Other)).cloned().collect::<Vec<_>>() {
				let subs: Vec<u32> = match &sub_it.kind {
					Kind::Label { rows, .. } => (0..rows.len() as u32).filter(|&i| rows[i as usize].1).collect(),
					Kind::Value { n, .. } => (0..*n).collect(),
					Kind::Grid { rows, cols, .. } => vec![0, rows * cols - 1],
					Kind::Ring { .. } => vec![3, 6],
					_ => vec![],
				};
				for sub in subs.into_iter().take(4) {
					let (x, y) = centre(&sub_it, sub);
					let (fx, fy) = frac(x, y);
					let t = r.snap.menu.as_ref().and_then(|m| m.hit(x, y));
					r.mouse_cursor(fx, fy);
					let want = Target::Sub { item: sub_it.addr, sub };
					let ok = until(&mut r, 150, |m| m.selected(want));
					seen += 1;
					if !ok || t != Some(want) {
						report.push(format!("page {:#x}: hover {:?} hit {:?} followed {ok}", pg.id, want, t));
					}
				}
			}
			eprintln!("mouse: page {:#x}: {seen} targets hovered", pg.id);
			// Right click = back to the root.
			r.mouse_back();
			let back = until(&mut r, 250, |m| m.id == 2);
			if !back {
				report.push(format!("page {:#x}: right click did not return to the root (now {:#x})", pg.id, r.snap.menu.as_ref().map_or(0, |m| m.id)));
				// try the keyboard so the next page can be tried
				for _ in 0..3 {
					r.tick_keys(|k| k == K::Enter, false, false);
				}
				until(&mut r, 250, |m| m.id == 2);
			}
			pump(&mut r, 20);
		}
		for l in &report {
			eprintln!("mouse REPORT: {l}");
		}
		assert!(r.stop.is_none() && !r.failed);
		assert!(report.is_empty(), "{} mouse problems (see REPORT lines)", report.len());
	}

	/// A field draws with the previous field's swap words; the first field with its own.
	#[test]
	fn swap_words_lag_one_field() {
		let w = |far: f32| SwapWords { fog_rgb: [0x72, 0xbe, 0xc8], fog: [50.0, far, 249.9, 63.75], part_far: 500.0, under: false, surf: None };
		let fx = |far: f32| RetailFx { swap: Some(w(far)), ..RetailFx::default() };
		let mut lag = None;
		let (mut a, mut b, mut c) = (fx(250.0), fx(128.0), fx(250.0));
		lag_swap(&mut a, &mut lag);
		lag_swap(&mut b, &mut lag);
		lag_swap(&mut c, &mut lag);
		assert_eq!((a.swap, b.swap, c.swap, lag), (Some(w(250.0)), Some(w(250.0)), Some(w(128.0)), Some(w(250.0))));
	}

	/// Manual check (needs the Eudora savestate): stick forward for 100 fields moves the hero.
	#[test]
	#[ignore]
	fn retail_stick_moves_hero() {
		enable(concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07")).unwrap();
		let mut r = Retail::load().unwrap();
		let p0 = r.snap.hero.unwrap().0;
		let (mut ms, mut worst) = (0.0, 0f32);
		for k in 0..600 {
			r.tick(if k == 100 { 0x40 } else { 0 }, [0x80, 0x80, 0x80, if k < 100 { 0 } else { 0x80 }]);
			ms += r.ms;
			worst = worst.max(r.ms);
		}
		r.tick(0, [0x80; 4]);
		let p1 = r.snap.hero.unwrap().0;
		eprintln!("hero {p0:?} -> {p1:?}, {:.1} ms/field (worst {worst:.1}), cam {:?}", ms / 600.0, r.cam);
		assert!(!r.failed);
		assert!((p1[0] - p0[0]).abs() + (p1[1] - p0[1]).abs() > 1.0);
	}

	/// Manual: Eudora -> Rilgar by the game's own travel sequence and loader; the new level's pools and camera come up.
	#[test]
	#[ignore]
	fn retail_travel() {
		enable(concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07")).unwrap();
		// SAFETY: the only test that sets it, before the interpreter thread starts.
		unsafe { std::env::set_var("RAC_RETAIL_TRAVEL", "5:10") };
		let mut r = Retail::load().unwrap();
		let mut got = None;
		for k in 0..2400 {
			r.tick(0, [0x80; 4]);
			if let Some(s) = r.switch() {
				eprintln!("field {k}: level {} pools {} classes, hero {:?}, cam {:?}", s.0, s.1.len(), r.snap.hero.map(|h| h.0), r.snap.cam);
				got = Some(s);
				break;
			}
		}
		assert!(!r.failed);
		let (l, pools) = got.expect("no travel");
		assert_eq!(l, 5);
		assert!(!pools.is_empty() && r.snap.hero.is_some() && r.snap.cam.is_some());
	}

	/// Manual (--wide across a level change): GS-full + wide from the Eudora state, travel to Rilgar, 200 more fields there; the
	/// wrapping hand must be used on both levels, VU1 errors stay 0, and every picture is 512x448 or 512x512.
	#[test]
	#[ignore]
	fn retail_wide_travel_probe() {
		enable(concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07")).unwrap();
		enable_gsfull();
		crate::gsfull::wide::enable();
		// SAFETY: the only test that sets it, before the interpreter thread starts.
		unsafe { std::env::set_var("RAC_RETAIL_TRAVEL", "5:10") };
		let mut r = Retail::load().unwrap();
		let gs = r.gsfull.clone().expect("GS-full threads");
		let (mut switched, mut after, mut shown, mut sizes) = (None, 0u32, 0u32, std::collections::BTreeSet::new());
		for k in 0..4000u32 {
			r.tick(0, [0x80; 4]);
			if let Some(f) = r.take_gsfull_frame() {
				shown += 1;
				sizes.insert((f.w, f.h));
			}
			if switched.is_none() {
				if let Some(s) = r.switch() {
					eprintln!("field {k}: switched to level {}, wide calls so far {:?}", s.0, crate::gsfull::wide::by_level());
					switched = Some(k);
				}
			} else {
				after += 1;
				if after >= 200 {
					break;
				}
			}
		}
		let by = crate::gsfull::wide::by_level();
		eprintln!("wide calls per level (eudora, rilgar) {by:?}; pictures {shown}, sizes {sizes:?}; {}", gs.report());
		assert!(!r.failed && switched.is_some());
		assert!(by[0] > 0 && by[1] > 0, "the hand must run on both levels");
	}

	/// Manual (needs the Eudora savestate): the 4:3 tests reproduce retail's `+0x31` on every moby of every field, and while the
	/// camera turns the mobys retail culled only at its 4:3 side plane that rac-rs's 16:9 window shows are all outside the 4:3
	/// band, inside the wide one, unhidden and within the draw distance.
	#[test]
	#[ignore]
	fn retail_wide_cull() {
		enable(concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07")).unwrap();
		let mut r = Retail::load().unwrap();
		let (w, h) = (1282.0f32, 720.0f32);
		let aspect = w / h;
		let (mut fields, mut checked, mut wrong) = (0, 0, Vec::new());
		let mut added: std::collections::BTreeMap<u32, (u32, u32, u32, u32)> = Default::default();
		let mut min_margin = f32::MAX;
		let (mut bad, mut centre_in_43, mut centre_out_wide) = (0, 0, 0);
		let mut shown_fields = 0;
			let mut caps: std::collections::BTreeSet<i32> = Default::default();
		let mut sample = Vec::new();
		let mut crates: Vec<String> = Vec::new();
		let script = |k: u32| -> u8 {
			match k {
				0..=149 => 0x80,
				150..=399 => 0xff,
				400..=649 => 0x00,
				650..=899 => 0xff,
				_ => 0x80,
			}
		};
		for k in 0..1000u32 {
			r.tick(0, [script(k), 0x80, 0x80, 0x80]);
			if k < 3 || r.snap.mobys.is_empty() {
				continue;
			}
			fields += 1;
			let mut any = false;
			for mb in &r.snap.mobys {
				let Some(c) = mb.culls else { continue };
				checked += 1;
				if mb.shown == c.any() {
					wrong.push((k, mb.addr, mb.class, mb.shown, c));
				}
				let Some(s) = mb.side_only else { continue };
				if !s.inside(aspect) {
					continue;
				}
				any = true;
				let tx43 = s.tan_v * 4.0 / 3.0;
				let txw = s.tan_v * aspect;
				let x43 = w / 2.0 + s.vx / (s.vz * tx43) * 480.0;
				let xw = w / 2.0 + s.vx / (s.vz * txw) * (w / 2.0);
				let e = added.entry(mb.addr).or_insert((0, k, 0, mb.class));
				e.0 += 1;
				e.2 = k;
				// The independent recheck: not hidden, inside the draw distance, outside the 4:3 band, nothing the rule skipped.
				let (flags, draw, cap) = mb.raw;
					caps.insert(cap);
				let dist_ok = (draw as i32).min(cap) << 10;
				if flags & 0x81 != 0 || s.vz - s.w > dist_ok as f32 || mb.shown || c.any() && !c.side_only() {
					bad += 1;
				}
				// Pixels the centre lies outside the 4:3 band (negative = inside it, only possible for a small far sphere: rac-rs's
				// 4:3 window is ty * 4/3 = 0.635 wide against retail's 0.63).
				min_margin = min_margin.min((x43 - w / 2.0).abs() - 480.0);
				if s.vx.abs() <= tx43 * s.vz {
					centre_in_43 += 1;
				}
				if xw < 0.0 || xw > w {
					centre_out_wide += 1;
				}
				if (500..=511).contains(&mb.class) && k % 10 == 0 && k < 340 {
					crates.push(format!("field {k} {:06x} class {} x {xw:.0} z {:.1} m", mb.addr, mb.class, s.vz / 1024.0));
				}
				if sample.len() < 12 && (sample.is_empty() || k % 25 == 0) {
					sample.push(format!("field {k} {:06x} class {} x43 {x43:.0} x16:9 {xw:.0} (4:3 band 161..1121) z {:.1} m r {:.2} m", mb.addr, mb.class, s.vz / 1024.0, s.w / 1024.0));
				}
			}
			if any {
				shown_fields += 1;
			}
		}
		eprintln!("fields {fields}, mobys checked {checked}, +0x31 mismatches {}", wrong.len());
		for x in wrong.iter().take(10) {
			eprintln!("  mismatch field {} {:06x} class {} shown {} culls {:?}", x.0, x.1, x.2, x.3, x.4);
		}
		eprintln!("fields where the rule adds a moby: {shown_fields}; distinct mobys {}; rule violations {bad}; centre inside 4:3 band {centre_in_43}; centre off the 16:9 window {centre_out_wide}", added.len());
		eprintln!("draw-distance caps seen with a moby added: {caps:?}");
		eprintln!("least pixels a centre lies outside the 4:3 band: {min_margin:.1}");
		for (a, (n, first, last, class)) in &added {
			eprintln!("  moby {a:06x} class {class}: added in {n} fields ({first}..{last})");
		}
		for l in &crates {
			eprintln!("  crate {l}");
		}
		for l in &sample {
			eprintln!("  {l}");
		}
		assert!(!r.failed);
		assert!(wrong.is_empty(), "the 4:3 tests disagree with +0x31");
		assert_eq!(bad, 0);
		assert!(!added.is_empty(), "the camera turn showed no wide-only moby");
	}

	/// Manual (input track): RAC_TEST_STATE (default s13, the pause menu) with RAC_TEST_PAD `from-to:buttons;...` (hex
	/// buttons, fields decimal; default d-pad Down at 10-14 and 25-29) for RAC_TEST_FIELDS fields through `Retail::tick`:
	/// one line per field with the game mode and the overlay hash (does the drawn menu follow the presses?).
	#[test]
	#[ignore]
	fn retail_pad_probe() {
		let state = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s13").into());
		let pad = std::env::var("RAC_TEST_PAD").unwrap_or_else(|_| "10-15:4000;25-30:4000".into());
		let fields: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
		// `from-to:buttons[:lx,ly[,rx,ry]]` (sticks hex, 80 centred).
		let script: Vec<(u32, u32, u16, Option<[u8; 4]>)> = pad
			.split(';')
			.filter_map(|e| {
				let mut f = e.split(':');
				let (r, b) = (f.next()?, f.next()?);
				let (a, z) = r.split_once('-')?;
				let st = f.next().and_then(|s| {
					let v: Vec<u8> = s.split(',').map(|x| u8::from_str_radix(x, 16).ok()).collect::<Option<_>>()?;
					Some([v[0], v[1], v.get(2).copied().unwrap_or(0x80), v.get(3).copied().unwrap_or(0x80)])
				});
				Some((a.parse().ok()?, z.parse().ok()?, u16::from_str_radix(b, 16).ok()?, st))
			})
			.collect();
		enable(&state).unwrap();
		let mut r = Retail::load().unwrap();
		let mut last = 0u64;
		for k in 0..fields {
			let b = script.iter().filter(|s| (s.0..s.1).contains(&k)).fold(0u16, |b, s| b | s.2);
			let st = script.iter().filter(|s| (s.0..s.1).contains(&k)).find_map(|s| s.3).unwrap_or([0x80; 4]);
			r.tick(b, [st[2], st[3], st[0], st[1]]);
			let ov = r.take_overlay().map(|(_, _, px)| px.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| (h ^ u32::from_le_bytes(*p) as u64).wrapping_mul(0x0100_0000_01b3)));
			let note = match ov {
				Some(h) if h != last => {
					last = h;
					format!("overlay {h:016x} CHANGED")
				}
				Some(h) => format!("overlay {h:016x}"),
				None => "no overlay".into(),
			};
			eprintln!("field {k} pad {b:04x} game held {:05x} new {:05x} raw {:05x} {:05x} mode {} equipped {} ammo {} last {} {:.1} ms cam yaw {:.1} pitch {:.1} hero {:?} {note}", r.snap.pad[0], r.snap.pad[1], r.snap.pad[2], r.snap.pad[3], r.snap.mode, r.snap.equipped, r.snap.ammo.get(r.snap.equipped as usize).copied().unwrap_or(0), r.snap.last, r.ms, r.cam.1.to_degrees(), r.cam.2.to_degrees(), r.snap.hero.map(|h| h.0));
		}
		assert!(!r.failed);
	}

	/// Manual (GS-full proof, run with RAC_GSFULL_LOG=1 [RAC_GSFULL_DUMP=DIR]): RAC_TEST_STATE (default s07) for RAC_TEST_FIELDS
	/// idle fields (default 20) through `Retail::tick` with the GS-full display on. The GS-full thread logs one line per field
	/// (FNV hash of the presented 512x448 RGBA picture, ms); this test waits for it to draw them all and prints the EE thread's
	/// and the GS-full thread's mean ms per field. <dev>/scratch_play_gsint_src (gsint) compares those pictures with the
	/// prototype's.
	#[test]
	#[ignore]
	fn retail_gsfull_probe() {
		let state = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07").into());
		let fields: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
		enable(&state).unwrap();
		enable_gsfull();
		let mut r = Retail::load().unwrap();
		let gs = r.gsfull.clone().expect("GS-full thread (needs GS.bin, vu1Memory.bin, vu1MicroMem.bin)");
		let (t0, mut ee, mut shown, mut first) = (std::time::Instant::now(), 0f32, 0u32, None);
		for _ in 0..fields + 1 {
			r.tick(0, [0x80; 4]);
			ee += r.ms;
			if let Some(f) = r.take_gsfull_frame() {
				shown += 1;
				first.get_or_insert((f.field, crate::gsfull::pipe::hash(&f.rgba)));
			}
		}
		let t_ee = t0.elapsed();
		let t = std::time::Instant::now();
		while gs.fields() < fields as u64 && !gs.dead.load(Ordering::SeqCst) && t.elapsed().as_secs() < 120 {
			std::thread::sleep(std::time::Duration::from_millis(20));
		}
		eprintln!("retail_gsfull: {fields} fields, EE thread {:.2} ms/field (ticks took {:.2} ms/field wall), GS-full: {}, {} pictures taken by the main thread, first {first:x?}", ee / (fields + 1) as f32, t_ee.as_secs_f64() * 1000.0 / (fields + 1) as f64, gs.report(), shown);
		assert!(!r.failed && !gs.dead.load(Ordering::SeqCst));
		assert!(gs.fields() >= fields as u64, "GS-full thread drew only {} fields", gs.fields());
	}

	/// Manual (--wide evidence): the same state and pad script run plain and with the wide-FOV hands (gsfull/wide.rs); every
	/// 50 fields the hero, the moby table and the RAM are compared. RAC_TEST_STATE (default s07), RAC_TEST_FIELDS (default 300).
	#[test]
	#[ignore]
	fn retail_wide_gameplay_probe() {
		let state = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07").into());
		let n: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
		ee_game::register();
		crate::gsfull::wide::register();
		let mut runs: Vec<Vec<(u32, Snap, Vec<u8>)>> = vec![];
		for wide in [false, true] {
			crate::gsfull::wide::set_active(wide);
			let mut m = ee_oracle::Machine::load(&state);
			m.enable_rec();
			let mut cps = vec![];
			for k in 0..n {
				// Idle for 20 fields, then the left stick forward, with a jump (Cross) every 90 fields.
				let (b, st) = if k < 20 { (0, [0x80u8; 4]) } else { (if k % 90 < 6 { 0x40 } else { 0 }, [0x80, 0x80, 0x80, 0x00]) };
				m.set_pad(b, st);
				m.frame().unwrap();
				if (k + 1) % 50 == 0 || k + 1 == n {
					cps.push((k + 1, snap(&m), m.cpu.mem.ram.to_vec()));
				}
			}
			runs.push(cps);
		}
		crate::gsfull::wide::set_active(true);
		let (calls, rewide, seen) = crate::gsfull::wide::stats();
		eprintln!("wide hand: {calls} calls, {rewide} with an already-wide input, game inputs seen {:x?}", seen);
		for (a, b) in runs[0].iter().zip(&runs[1]) {
			let (sa, sb) = (&a.1, &b.1);
			let ha = sa.hero.as_ref().map(|h| h.0);
			let hb = sb.hero.as_ref().map(|h| h.0);
			let bm: std::collections::HashMap<u32, &Moby> = sb.mobys.iter().map(|m| (m.addr, m)).collect();
			let (mut moved, mut shown, mut gone) = (0, 0, 0);
			for m in &sa.mobys {
				match bm.get(&m.addr) {
					Some(o) => {
						moved += (m.pos != o.pos || m.class != o.class) as u32;
						shown += (m.shown != o.shown) as u32;
					}
					None => gone += 1,
				}
			}
			let words = a.2.chunks_exact(4).zip(b.2.chunks_exact(4)).filter(|(x, y)| x != y).count();
			eprintln!(
				"field {}: hero {:?} vs {:?} same={}; health {}/{} bolts {}/{}; mobys {}/{}: moved {moved}, shown flag differs {shown}, only in plain {gone}; RAM words differing {words}",
				a.0, ha, hb, ha == hb, sa.health, sb.health, sa.bolts, sb.bolts, sa.mobys.len(), sb.mobys.len()
			);
		}
		// The mobys that differ at the last checkpoint: class, positions, the game's shown flag (+0x31) in both runs.
		{
			let (sa, sb) = (&runs[0].last().unwrap().1, &runs[1].last().unwrap().1);
			let bm: std::collections::HashMap<u32, &Moby> = sb.mobys.iter().map(|m| (m.addr, m)).collect();
			let mut by_class: std::collections::BTreeMap<u32, (u32, u32, u32)> = Default::default();
			let mut rows = vec![];
			for m in &sa.mobys {
				let e = by_class.entry(m.class).or_default();
				e.0 += 1;
				if let Some(o) = bm.get(&m.addr) {
					let mv = m.pos != o.pos || m.class != o.class;
					e.1 += mv as u32;
					e.2 += (m.shown != o.shown) as u32;
					if mv && rows.len() < 14 {
						let d = ((m.pos[0] - o.pos[0]).powi(2) + (m.pos[1] - o.pos[1]).powi(2) + (m.pos[2] - o.pos[2]).powi(2)).sqrt();
						rows.push(format!("class {} @{:x} plain {:?} wide {:?} dist {d:.2} shown {}/{}", m.class, m.addr, m.pos, o.pos, m.shown, o.shown));
					}
				}
			}
			eprintln!("per class (count, moved, shown differs), classes with a difference: {:?}", by_class.iter().filter(|(_, v)| v.1 + v.2 > 0).collect::<Vec<_>>());
			for r in rows {
				eprintln!("  {r}");
			}
		}
		// Where the RAM differs at the last checkpoint (1 MB buckets).
		let (x, y) = (&runs[0].last().unwrap().2, &runs[1].last().unwrap().2);
		let mut bucket = std::collections::BTreeMap::new();
		for (i, (p, q)) in x.chunks_exact(4).zip(y.chunks_exact(4)).enumerate() {
			if p != q {
				*bucket.entry(i * 4 / 0x1000).or_insert(0u32) += 1;
			}
		}
		eprintln!("differing 4 KB pages (page address: words): {:x?}", bucket.iter().map(|(k, v)| (k * 0x1000, *v)).collect::<Vec<_>>());
	}

	/// Manual (EE speed): `Machine::frame` ms per field of RAC_TEST_STATE (default s07) over RAC_TEST_FIELDS fields (default 40) with
	/// the pieces of `Retail::load` switched on one by one by RAC_TEST_EE=snd,sound,gsq (comma list).
	#[test]
	#[ignore]
	fn retail_ee_probe() {
		let state = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07").into());
		let fields: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
		let opts = std::env::var("RAC_TEST_EE").unwrap_or_default();
		let has = |k: &str| opts.split(',').any(|o| o == k);
		if has("regfirst") {
			ee_game::register();
		}
		let mut m = ee_oracle::Machine::load(&state);
		if has("sound") {
			m.record_sound();
		}
		if !has("regfirst") {
			ee_game::register();
		}
		let nrec = m.enable_rec();
		eprintln!("retail_ee: {nrec} recompiled functions");
		let (tx, rx) = std::sync::mpsc::channel::<ee_oracle::gs::Job>();
		if has("gsq") {
			m.cpu.mem.gsq = Some(tx);
		}
		let worker = has("thread");
		let mut run = move || {
			let mut ms = Vec::new();
			for _ in 0..fields {
				m.set_pad(0, [0x80; 4]);
				let t = std::time::Instant::now();
				m.frame().unwrap();
				ms.push(t.elapsed().as_secs_f64() * 1000.0);
				let _ = rx.try_iter().count();
			}
			ms
		};
		let ms = if worker { std::thread::Builder::new().stack_size(256 << 20).spawn(run).unwrap().join().unwrap() } else { run() };
		let skip = 3.min(ms.len());
		eprintln!("retail_ee [{opts}]: {:.2} ms/field (first {skip} skipped), min {:.2}", ms[skip..].iter().sum::<f64>() / (ms.len() - skip) as f64, ms[skip..].iter().cloned().fold(f64::MAX, f64::min));
	}

	/// The movie hook dispatches from the oracle (stage I1). eudora1 window 0041's recorded pad (RAC_RETAIL_PLAY=@play)
	/// enters an in-game movie after field 1133 (the oracle used to spin there, pc 0x25939c, game mode 1): through the
	/// real Retail path the retail player call reaches fmv_bridge's hook, `tick` hands the request out instead of
	/// blocking, the game thread stays parked (no spin, no field) until `end_movie`, then the game leaves movie mode.
	/// Run: cargo test --release retail_movie_hook -- --ignored --nocapture
	#[test]
	#[ignore]
	fn retail_movie_hook() {
		unsafe {
			std::env::set_var("RAC_RETAIL_HUD", "0");
			std::env::set_var("RAC_RETAIL_PLAY", "@play");
			std::env::set_var("RAC_PLAYER_CARD", "0");
			std::env::set_var("RAC_PLAYER_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/target/player_test"));
		}
		let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/play/eudora1_0041").into());
		let max: u32 = std::env::var("RAC_TEST_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(1450);
		enable(&dir).unwrap();
		let mut r = Retail::load().unwrap();
		fmv_bridge::set_serviced(true);
		let t0 = std::time::Instant::now();
		let mut got = None;
		while r.fields < max && !r.failed {
			r.tick(0, [0x80; 4]);
			if let Some(q) = r.take_movie() {
				got = Some(q);
				break;
			}
		}
		let q = got.unwrap_or_else(|| panic!("no movie request in {} fields (stop {:?}, mode {})", r.fields, r.stop, r.snap.mode));
		let at = r.fields;
		eprintln!("movie_hook: request {q:?} after field {at} ({:.1} s), last finished field mode {} level {}", t0.elapsed().as_secs_f32(), r.snap.mode, r.snap.level);
		assert!(fmv_bridge::waiting());
		// Parked, not spinning: no field completes while the movie "plays" (1 s), and tick returns at once.
		let mut tick_ms = 0.0f64;
		for _ in 0..50 {
			let t1 = std::time::Instant::now();
			r.tick(0, [0x80; 4]);
			tick_ms = tick_ms.max(t1.elapsed().as_secs_f64() * 1000.0);
			std::thread::sleep(std::time::Duration::from_millis(20));
		}
		assert_eq!(r.fields, at, "a field ran while the game thread should wait in the hook");
		assert!(r.take_movie().is_none() && fmv_bridge::waiting() && !r.failed);
		eprintln!("movie_hook: parked 1 s: fields still {at}, longest tick {tick_ms:.3} ms");
		r.end_movie();
		let mut modes = Vec::new();
		let mut more = 0;
		// After the movie the game may run an in-engine scene (mode 2); wait for gameplay (mode 0), at most 3000 fields.
		for _ in 0..3000 {
			r.tick(0, [0x80; 4]);
			if r.take_movie().is_some() {
				more += 1;
				r.end_movie();
			}
			if modes.last().map(|x: &(u32, u32)| x.1) != Some(r.snap.mode) {
				modes.push((r.fields, r.snap.mode));
			}
			if r.failed || (r.snap.mode == 0 && r.fields > at + 20) {
				break;
			}
		}
		let (played, skipped) = fmv_bridge::counts();
		eprintln!("movie_hook: after end_movie: modes (field, mode) {modes:?}; requests played {played} skipped {skipped} more {more}; stop {:?}; fields {}", r.stop, r.fields);
		assert!(r.stop.is_none() && !r.failed, "oracle stopped: {:?}", r.stop);
		assert_eq!((played, skipped, more), (1, 0, 0));
		assert_eq!(r.snap.mode, 0, "game mode after the movie");
	}

	/// Manual: the mobys near the camera after 150 idle fields.
	#[test]
	#[ignore]
	fn retail_near_camera() {
		enable(concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07")).unwrap();
		let mut r = Retail::load().unwrap();
		for _ in 0..151 {
			r.tick(0, [0x80; 4]);
		}
		let c = r.cam.0;
		for mb in &r.snap.mobys {
			let d = ((mb.pos[0] - c[0]).powi(2) + (mb.pos[1] - c[1]).powi(2) + (mb.pos[2] - c[2]).powi(2)).sqrt();
			if d < 12.0 {
				eprintln!("{:06x} class {} d {d:.1} pos {:?} scale {} shown {}", mb.addr, mb.class, mb.pos, mb.scale, mb.shown);
			}
		}
	}
}

/// RAC_TEST_SLOW_EE=F (F > 1, tests only): imitates a CPU F times slower for the game thread (see the worker loop).
pub fn test_slow_ee() -> Option<f64> {
	static V: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
	*V.get_or_init(|| std::env::var("RAC_TEST_SLOW_EE").ok().and_then(|v| v.parse().ok()).filter(|&f: &f64| f > 1.0))
}

/// RAC_TEST_SLOW_RENDER=MS (tests only): every redraw takes MS longer, as a slow GPU would (viewer/handler.rs).
pub fn test_slow_render() -> Option<std::time::Duration> {
	static V: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
	V.get_or_init(|| std::env::var("RAC_TEST_SLOW_RENDER").ok().and_then(|v| v.parse().ok()).filter(|&m: &f64| m > 0.0)).map(|m| std::time::Duration::from_secs_f64(m / 1000.0))
}
