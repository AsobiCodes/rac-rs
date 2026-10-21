//! Modern camera and RMB aim mode for retail mode (PC extra the user asked for, 2026-10-12): `--modern-cam` or
//! RAC_MODERN_CAM=1. Off, nothing is registered and nothing is written: retail stays bit-exact. F7 toggles it in game.
//!
//! How it works (design: scratch_play/handoff/modcam.md):
//! - A wrapping hand on the follow camera's class update (every level, table `LVS`; Eudora 0x2ece98, Rilgar 0x324c68) runs the retail rig, then
//!   overwrites the current camera record's rows (+0x00/+0x10/+0x20) and eye (+0x30) with an orbit pose (mouse look,
//!   keyboard right stick, wheel zoom). The camera main publishes that record into the camera globals, so the view,
//!   culling, sound listener and the hero's camera-relative steering (hero_stick_wish_yaw reads yaw 0x166ed8 and the
//!   rows 0x1670d0) all follow it [C modcam.md 1.1/1.3].
//! - The wall test calls retail's own segment test (seg_collide, mask 0xb4 = retail's camera line mask) and restores
//!   its side effects (result block, moby touch ring, moby +0xa4) and every register afterwards.
//! - Other camera classes (scripted, free look, swing, rail...), scenes, menus and the hero states listed in `GATE_STATES`
//!   keep the retail camera; the modern pose blends back to retail over `BLEND` fields.
//! - Aim mode: RMB held. No target pick and no framing: the mouse keeps turning the camera. Ratchet faces the camera's
//!   horizontal direction (idle: his yaw 0x13f3e8 / wish 0x13f4d0 turn at once; moving: his drawn facing, moby +0x48,
//!   while his path stays the stick's = strafe). Guns: retail's manual-aim override (0x140600 = 1, yaw 0x14060c = camera
//!   yaw, which no retail code writes) replaces the hero-forward yaw as the shot / search direction, so retail's OWN
//!   target search (flag 0x1000 actors within its angle window, LOS) and its green reticle stay. Wrench: wrapping hand on
//!   hero_attack_lock; retail's own pick stays, with no target the swing yaw 0x13fdac / flag 0x13fda8 = the camera yaw
//!   (as retail sets them from stick input).
//!
//! Input reaches the worker in `FieldIn::mc` (recorded and replayed with the pad).
use ee_oracle::cpu::Cpu;
use ee_oracle::rec::{register_hand, Hand};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use crate::retail::disc::va;

static ENABLED: AtomicBool = AtomicBool::new(false);
static IN: AtomicBool = AtomicBool::new(false);
static IN_ATK: AtomicBool = AtomicBool::new(false);
/// hero_attack_lock calls seen by the hand (tests).
static ATK_RUNS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// `--modern-cam`.
pub fn enable() {
	ENABLED.store(true, Ordering::SeqCst);
}

/// The switch: `--modern-cam` or RAC_MODERN_CAM=1 (read when the retail machine loads).
pub fn enabled() -> bool {
	// Off on the USA disc with RAC_USA_EXTRAS=0 (retail/disc.rs).
	if super::disc::extras_off() {
		return false;
	}
	ENABLED.load(Ordering::SeqCst) || std::env::var("RAC_MODERN_CAM").is_ok_and(|v| v != "0" && !v.is_empty()) || super::pcext::enabled()
}

/// The PC options' Camera row (pcext.rs): Modern = on, Classic = off. Same effect as F7, set from the remembered value.
pub fn set_active(on: bool) {
	if let Ok(mut st) = ST.lock() {
		st.active = on;
	}
	if let Ok(mut u) = UI.lock() {
		u.on = on;
	}
}

/// One field's modern-camera input (part of `FieldIn`, so recordings replay it).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
	/// Mouse counts since the last field (x right, y down).
	pub look: [i16; 2],
	/// RMB held: aim mode.
	pub aim: bool,
	/// Wheel steps (positive = zoom in).
	pub zoom: i8,
	/// F7 pressed: toggle the modern camera.
	pub toggle: bool,
}

impl Input {
	pub fn is_zero(&self) -> bool {
		*self == Input::default()
	}
}

// ---- main thread: what the window collected since the last field -------------------------------------------------

#[derive(Default)]
struct Ui {
	look: [f32; 2],
	aim: bool,
	zoom: f32,
	toggle: bool,
	/// The modern camera as the main thread last saw it (title / grab), mirrored from the worker.
	on: bool,
}

static UI: Mutex<Ui> = Mutex::new(Ui { look: [0.0; 2], aim: false, zoom: 0.0, toggle: false, on: true });
static FEED: Mutex<Option<Vec<(u32, u32, Input)>>> = Mutex::new(None);

/// Mouse motion while the cursor is captured.
pub fn ui_mouse(dx: f32, dy: f32) {
	if let Ok(mut u) = UI.lock() {
		u.look[0] += dx;
		u.look[1] += dy;
	}
}

/// RMB down / up.
pub fn ui_aim(down: bool) {
	if let Ok(mut u) = UI.lock() {
		u.aim = down;
	}
}

/// Gamepad R2 held (prompts.rs `poll_pad`): aim mode like RMB.
static PAD_AIM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// R2 down / up on the gamepad.
pub fn pad_aim(down: bool) {
	PAD_AIM.store(down, std::sync::atomic::Ordering::Relaxed);
}

/// Wheel lines (positive = away from the user = zoom in).
pub fn ui_zoom(lines: f32) {
	if let Ok(mut u) = UI.lock() {
		u.zoom += lines;
	}
}

/// F7.
pub fn ui_toggle() {
	if let Ok(mut u) = UI.lock() {
		u.toggle = true;
	}
}

/// Focus lost: let go of RMB and drop the pending motion.
pub fn ui_release() {
	if let Ok(mut u) = UI.lock() {
		u.look = [0.0; 2];
		u.aim = false;
		u.zoom = 0.0;
	}
}

/// Whether the modern camera is on (switch enabled and not toggled off): the viewer captures the mouse only then.
pub fn on() -> bool {
	enabled() && UI.lock().is_ok_and(|u| u.on)
}

/// RAC_MODCAM_FEED (tests, shots): `from-to:dx,dy,aim[,zoom];...` (decimal), the mouse counts per field and RMB for fields
/// `from..to`, ORed over the window's input.
fn feed() -> Vec<(u32, u32, Input)> {
	parse_feed(&std::env::var("RAC_MODCAM_FEED").unwrap_or_default())
}

fn parse_feed(v: &str) -> Vec<(u32, u32, Input)> {
	v.split(';')
		.filter_map(|p| {
			let (r, a) = p.split_once(':')?;
			let (f, t) = r.split_once('-')?;
			let n: Vec<i32> = a.split(',').filter_map(|x| x.trim().parse().ok()).collect();
			let inp = Input { look: [*n.first()? as i16, *n.get(1)? as i16], aim: n.get(2).is_some_and(|&l| l != 0), zoom: n.get(3).copied().unwrap_or(0) as i8, toggle: false };
			Some((f.trim().parse().ok()?, t.trim().parse().ok()?, inp))
		})
		.collect()
}

/// The input of field `n` (Retail::tick, live play): the window's input since the last field plus RAC_MODCAM_FEED.
pub fn take_ui(n: u32) -> Input {
	if !enabled() {
		return Input::default();
	}
	let mut out = Input::default();
	if let Ok(mut u) = UI.lock() {
		let x = u.look[0].round().clamp(-32000.0, 32000.0);
		let y = u.look[1].round().clamp(-32000.0, 32000.0);
		u.look[0] -= x;
		u.look[1] -= y;
		let z = u.zoom.trunc().clamp(-100.0, 100.0);
		u.zoom -= z;
		out = Input { look: [x as i16, y as i16], aim: u.aim || PAD_AIM.load(std::sync::atomic::Ordering::Relaxed), zoom: z as i8, toggle: std::mem::take(&mut u.toggle) };
	}
	if let Ok(mut f) = FEED.lock() {
		for &(a, b, i) in f.get_or_insert_with(feed).iter() {
			if (a..b).contains(&n) {
				out.look[0] = out.look[0].saturating_add(i.look[0]);
				out.look[1] = out.look[1].saturating_add(i.look[1]);
				out.aim |= i.aim;
				out.zoom = out.zoom.saturating_add(i.zoom);
			}
		}
	}
	out
}

// ---- per level retail addresses -------------------------------------------------------------------------------

struct Lv {
	level: u32,
	follow: u32,
	/// seg_collide (start*, end*, mask, ignore) -> v0 1 on a hit [C modcam.md 2].
	seg: u32,
	/// seg_collide's result block (contact point at +0x20) and the moby touch ring (64 x 0x40).
	res: u32,
	ring: u32,
	/// Lock registry: the null-ended actor list hero_lock_registry_pick walks (tests: finding enemies).
	reg: u32,
	attack: u32,
	/// objects::attached_value(moby) -> the health float (!= 0) [C 0x2009a0 / 0x23b228 disasm] (tests).
	health: u32,
	/// LEVEL_SHIFT of the camera globals from Novalis / Rilgar's addresses.
	shift: i32,
}

static LVS: [Lv; 19] = [
	Lv { level: 0, follow: 0x2e95d0, seg: 0x1efbf0, res: 0x173e40, ring: 0x178100, reg: 0x1aba00, attack: 0x214ab8, health: 0x25bf48, shift: -0x400 }, // 0_veldin_tutorial
	Lv { level: 1, follow: 0x314730, seg: 0x211770, res: 0x174240, ring: 0x178500, reg: 0x1abe00, attack: 0x234d38, health: 0x270d90, shift: 0x0 }, // 1_novalis
	Lv { level: 2, follow: 0x2f74d8, seg: 0x200a58, res: 0x1743c0, ring: 0x178680, reg: 0x1abf80, attack: 0x2236e0, health: 0x25e4f8, shift: 0x180 }, // 2_aridia
	Lv { level: 3, follow: 0x2e7ec0, seg: 0x1e94d8, res: 0x173ec0, ring: 0x178180, reg: 0x1aba80, attack: 0x20cbe0, health: 0x24a918, shift: -0x380 }, // 3_kerwan
	Lv { level: 4, follow: 0x2ece98, seg: 0x1e24d8, res: 0x173f40, ring: 0x178200, reg: 0x1abb00, attack: 0x208018, health: 0x24eef8, shift: -0x300 }, // 4_eudora
	Lv { level: 5, follow: 0x324c68, seg: 0x21e858, res: 0x174240, ring: 0x178500, reg: 0x1abe80, attack: 0x242568, health: 0x2861a8, shift: 0x0 }, // 5_rilgar
	Lv { level: 6, follow: 0x3115f0, seg: 0x207758, res: 0x174540, ring: 0x178800, reg: 0x1ac100, attack: 0x22b748, health: 0x26acd8, shift: 0x300 }, // 6_nebula_g34
	Lv { level: 7, follow: 0x324f00, seg: 0x217958, res: 0x173e40, ring: 0x178100, reg: 0x1aba00, attack: 0x23c8a0, health: 0x2848e8, shift: -0x400 }, // 7_umbris
	Lv { level: 8, follow: 0x311128, seg: 0x202580, res: 0x1745c0, ring: 0x178880, reg: 0x1ac180, attack: 0x226160, health: 0x2663c0, shift: 0x380 }, // 8_batalia
	Lv { level: 9, follow: 0x311080, seg: 0x20eed8, res: 0x173f40, ring: 0x178200, reg: 0x1abb00, attack: 0x234718, health: 0x27b980, shift: -0x300 }, // 9_gaspar
	Lv { level: 10, follow: 0x2f1988, seg: 0x1e31d8, res: 0x174240, ring: 0x178500, reg: 0x1abe00, attack: 0x2080a0, health: 0x24e9d8, shift: 0x0 }, // 10_orxon
	Lv { level: 11, follow: 0x324908, seg: 0x2213d8, res: 0x1747c0, ring: 0x178a80, reg: 0x1ac380, attack: 0x2447f0, health: 0x281d70, shift: 0x580 }, // 11_pokitaru
	Lv { level: 12, follow: 0x311758, seg: 0x2125e0, res: 0x174240, ring: 0x178500, reg: 0x1abe00, attack: 0x235f68, health: 0x274dc0, shift: 0x0 }, // 12_hoven
	Lv { level: 13, follow: 0x3139b8, seg: 0x1fb558, res: 0x1740c0, ring: 0x178380, reg: 0x1abc80, attack: 0x220520, health: 0x267f10, shift: -0x180 }, // 13_oltanis_orbit
	Lv { level: 14, follow: 0x3100d8, seg: 0x202de0, res: 0x174540, ring: 0x178800, reg: 0x1ac100, attack: 0x226400, health: 0x2642d0, shift: 0x300 }, // 14_oltanis
	Lv { level: 15, follow: 0x2f37f8, seg: 0x1e9258, res: 0x1743c0, ring: 0x178680, reg: 0x1abf80, attack: 0x20c7c8, health: 0x24b778, shift: 0x180 }, // 15_quartu
	Lv { level: 16, follow: 0x2ef6c0, seg: 0x1ee3d8, res: 0x1741c0, ring: 0x178480, reg: 0x1abe00, attack: 0x212218, health: 0x2577a8, shift: -0x80 }, // 16_kalebo_iii
	Lv { level: 17, follow: 0x2fbb58, seg: 0x1ecbd8, res: 0x1746c0, ring: 0x178980, reg: 0x1ac280, attack: 0x211828, health: 0x256778, shift: 0x480 }, // 17_veldin_orbit
	Lv { level: 18, follow: 0x302190, seg: 0x1f8858, res: 0x1747c0, ring: 0x178a80, reg: 0x1ac380, attack: 0x21c540, health: 0x25e210, shift: 0x580 }, // 18_veldin_finale
];
/// `LVS` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
static LVS_USA: [Lv; 19] = [
	Lv { level: 0, follow: 0x2e9c28, seg: 0x1efc70, res: 0x173e40, ring: 0x178100, reg: 0x1aba00, attack: 0x214ed8, health: 0x25c338, shift: -0x400 }, // 0_veldin_tutorial
	Lv { level: 1, follow: 0x314e00, seg: 0x211870, res: 0x1742c0, ring: 0x178580, reg: 0x1abe80, attack: 0x2351d0, health: 0x2711f8, shift: 0x0 }, // 1_novalis
	Lv { level: 2, follow: 0x2f7ba8, seg: 0x200b58, res: 0x174440, ring: 0x178700, reg: 0x1ac000, attack: 0x223b78, health: 0x25e960, shift: 0x180 }, // 2_aridia
	Lv { level: 3, follow: 0x2e8590, seg: 0x1e95d8, res: 0x173f40, ring: 0x178200, reg: 0x1abb00, attack: 0x20d078, health: 0x24ad80, shift: -0x380 }, // 3_kerwan
	Lv { level: 4, follow: 0x2ed570, seg: 0x1e25d8, res: 0x173fc0, ring: 0x178280, reg: 0x1abb80, attack: 0x2084b8, health: 0x24f368, shift: -0x300 }, // 4_eudora
	Lv { level: 5, follow: 0x3252a8, seg: 0x21e8d8, res: 0x174240, ring: 0x178500, reg: 0x1abe80, attack: 0x242980, health: 0x2865b0, shift: 0x0 }, // 5_rilgar
	Lv { level: 6, follow: 0x311cc8, seg: 0x207858, res: 0x1745c0, ring: 0x178880, reg: 0x1ac180, attack: 0x22bbe8, health: 0x26b148, shift: 0x300 }, // 6_nebula_g34
	Lv { level: 7, follow: 0x3255d8, seg: 0x217a58, res: 0x173ec0, ring: 0x178180, reg: 0x1aba80, attack: 0x23cd40, health: 0x284d58, shift: -0x400 }, // 7_umbris
	Lv { level: 8, follow: 0x3117f8, seg: 0x202680, res: 0x174640, ring: 0x178900, reg: 0x1ac200, attack: 0x2265f8, health: 0x266828, shift: 0x380 }, // 8_batalia
	Lv { level: 9, follow: 0x311758, seg: 0x20efd8, res: 0x173fc0, ring: 0x178280, reg: 0x1abb80, attack: 0x234bb8, health: 0x27bdf0, shift: -0x300 }, // 9_gaspar
	Lv { level: 10, follow: 0x2f2060, seg: 0x1e32d8, res: 0x1742c0, ring: 0x178580, reg: 0x1abe80, attack: 0x208540, health: 0x24ee48, shift: 0x0 }, // 10_orxon
	Lv { level: 11, follow: 0x324fd8, seg: 0x2214d8, res: 0x174840, ring: 0x178b00, reg: 0x1ac400, attack: 0x244c88, health: 0x2821d8, shift: 0x580 }, // 11_pokitaru
	Lv { level: 12, follow: 0x311e28, seg: 0x2126e0, res: 0x1742c0, ring: 0x178580, reg: 0x1abe80, attack: 0x236400, health: 0x275228, shift: 0x0 }, // 12_hoven
	Lv { level: 13, follow: 0x314090, seg: 0x1fb658, res: 0x174140, ring: 0x178400, reg: 0x1abd00, attack: 0x2209c0, health: 0x268380, shift: -0x180 }, // 13_oltanis_orbit
	Lv { level: 14, follow: 0x310728, seg: 0x202e60, res: 0x174540, ring: 0x178800, reg: 0x1ac100, attack: 0x226818, health: 0x2646b8, shift: 0x300 }, // 14_oltanis
	Lv { level: 15, follow: 0x2f3ec8, seg: 0x1e9358, res: 0x174440, ring: 0x178700, reg: 0x1ac000, attack: 0x20cc60, health: 0x24bbe0, shift: 0x180 }, // 15_quartu
	Lv { level: 16, follow: 0x2efdb0, seg: 0x1ee4d8, res: 0x174240, ring: 0x178500, reg: 0x1abe80, attack: 0x2126b0, health: 0x257c30, shift: -0x80 }, // 16_kalebo_iii
	Lv { level: 17, follow: 0x2fc230, seg: 0x1eccd8, res: 0x174740, ring: 0x178a00, reg: 0x1ac300, attack: 0x211cc8, health: 0x256be8, shift: 0x480 }, // 17_veldin_orbit
	Lv { level: 18, follow: 0x302860, seg: 0x1f8958, res: 0x174840, ring: 0x178b00, reg: 0x1ac400, attack: 0x21c9d8, health: 0x25e678, shift: 0x580 }, // 18_veldin_finale
];

impl Lv {
	fn g(&self, novalis: u32) -> u32 {
		va((novalis as i32 + self.shift) as u32)
	}
	fn cur(&self) -> u32 {
		self.g(0x167200)
	}
	fn blend(&self) -> u32 {
		self.g(0x1672f0)
	}
	fn gate(&self) -> u32 {
		self.g(0x16c46c)
	}
}

/// The row of the running level (every one of the 19 overlays), from the level byte or, failing that, from the pc.
fn lv_of(level: u32) -> Option<&'static Lv> {
	(*crate::retail::disc::pick(&LVS, &LVS_USA)).get(level as usize)
}

/// The row whose hand entry (`at`) is the running pc: a recompiled prologue calls the hand with `c.pc == entry`; the
/// follow entries and the attack entries are pairwise distinct across the 19 levels [C wg_modcam_plan.md section 1].
fn lv_here(c: &Cpu, at: fn(&Lv) -> u32) -> Option<&'static Lv> {
	let l = c.mem.ram[0x15ed84] as usize; // level byte (Machine::level)
	(*crate::retail::disc::pick(&LVS, &LVS_USA)).get(l).filter(|lv| at(lv) == c.pc).or_else(|| (*crate::retail::disc::pick(&LVS, &LVS_USA)).iter().find(|lv| at(lv) == c.pc))
}
/// Log only (RAC_AIM_LOG, the camera-speed proof): the follow camera's position, Novalis 0x1671c0 plus the level's shift.
pub fn cam_pos_diag(m: &ee_oracle::Machine) -> [f32; 3] {
	lv_of(m.rd8(0x15ed84) as u32).map_or([0.0; 3], |lv| m.rdv3(lv.g(0x1671c0)))
}
fn follow_any(c: &mut Cpu) -> bool {
	lv_here(c, |l| l.follow).is_some_and(|lv| follow(c, lv))
}
fn attack_any(c: &mut Cpu) -> bool {
	lv_here(c, |l| l.attack).is_some_and(|lv| attack(c, lv))
}

// Hashes: rec::hash over [entry, end) of each overlay.elf (= the l0..l6 crates' rec::Fn hashes where crates exist) [C].
static HANDS: [Hand; 38] = [
	Hand { entry: 0x2e95d0, end: 0x2e965c, hash: 0x43ff9c929dbdb677, est: 0, f: follow_any }, // 0 follow
	Hand { entry: 0x214ab8, end: 0x214bc8, hash: 0xbdd748fe4a77c4f9, est: 0, f: attack_any }, // 0 hero_attack_lock
	Hand { entry: 0x314730, end: 0x3147bc, hash: 0xed52881251a9384f, est: 0, f: follow_any }, // 1 follow
	Hand { entry: 0x234d38, end: 0x234e48, hash: 0xca232fd1b4fb56d3, est: 0, f: attack_any }, // 1 hero_attack_lock
	Hand { entry: 0x2f74d8, end: 0x2f7564, hash: 0x72dda68f20983d1f, est: 0, f: follow_any }, // 2 follow
	Hand { entry: 0x2236e0, end: 0x2237f0, hash: 0xdbcfa10e2a853ca9, est: 0, f: attack_any }, // 2 hero_attack_lock
	Hand { entry: 0x2e7ec0, end: 0x2e7f4c, hash: 0x47248440728c2b1f, est: 0, f: follow_any }, // 3 follow
	Hand { entry: 0x20cbe0, end: 0x20ccf0, hash: 0x217701fd3e7fae0d, est: 0, f: attack_any }, // 3 hero_attack_lock
	Hand { entry: 0x2ece98, end: 0x2ecf24, hash: 0xbe5e9ea5407b0c9f, est: 0, f: follow_any }, // 4 follow
	Hand { entry: 0x208018, end: 0x208128, hash: 0x884c406ccef2fb0f, est: 0, f: attack_any }, // 4 hero_attack_lock
	Hand { entry: 0x324c68, end: 0x324cf4, hash: 0x07298cd75a7c7f77, est: 0, f: follow_any }, // 5 follow
	Hand { entry: 0x242568, end: 0x242678, hash: 0x0acff4789ec643a1, est: 0, f: attack_any }, // 5 hero_attack_lock
	Hand { entry: 0x3115f0, end: 0x31167c, hash: 0x0a57d352b7575ed7, est: 0, f: follow_any }, // 6 follow
	Hand { entry: 0x22b748, end: 0x22b858, hash: 0xac763d68e74026d9, est: 0, f: attack_any }, // 6 hero_attack_lock
	Hand { entry: 0x324f00, end: 0x324f8c, hash: 0x9aeff827d06d10b3, est: 0, f: follow_any }, // 7 follow
	Hand { entry: 0x23c8a0, end: 0x23c9b0, hash: 0x9ca74a4983df5e8b, est: 0, f: attack_any }, // 7 hero_attack_lock
	Hand { entry: 0x311128, end: 0x3111b4, hash: 0xf9e9b50f50fd1b8b, est: 0, f: follow_any }, // 8 follow
	Hand { entry: 0x226160, end: 0x226270, hash: 0x7f410f4fc145b927, est: 0, f: attack_any }, // 8 hero_attack_lock
	Hand { entry: 0x311080, end: 0x31110c, hash: 0x973c85487eb9e397, est: 0, f: follow_any }, // 9 follow
	Hand { entry: 0x234718, end: 0x234828, hash: 0x236282bb0e9c8fb1, est: 0, f: attack_any }, // 9 hero_attack_lock
	Hand { entry: 0x2f1988, end: 0x2f1a14, hash: 0xe73baa414754feff, est: 0, f: follow_any }, // 10 follow
	Hand { entry: 0x2080a0, end: 0x2081b0, hash: 0xe226ef36307a348b, est: 0, f: attack_any }, // 10 hero_attack_lock
	Hand { entry: 0x324908, end: 0x324994, hash: 0xf6965c06c234ab77, est: 0, f: follow_any }, // 11 follow
	Hand { entry: 0x2447f0, end: 0x244900, hash: 0xd2dbda59afcc0601, est: 0, f: attack_any }, // 11 hero_attack_lock
	Hand { entry: 0x311758, end: 0x3117e4, hash: 0x2220803636caed1f, est: 0, f: follow_any }, // 12 follow
	Hand { entry: 0x235f68, end: 0x236078, hash: 0xd7349a1315700a5b, est: 0, f: attack_any }, // 12 hero_attack_lock
	Hand { entry: 0x3139b8, end: 0x313a44, hash: 0x595e0d50de29a815, est: 0, f: follow_any }, // 13 follow
	Hand { entry: 0x220520, end: 0x220630, hash: 0x8e074137bce30beb, est: 0, f: attack_any }, // 13 hero_attack_lock
	Hand { entry: 0x3100d8, end: 0x310164, hash: 0x4929eb709022758f, est: 0, f: follow_any }, // 14 follow
	Hand { entry: 0x226400, end: 0x226510, hash: 0x655a7962f8e919e1, est: 0, f: attack_any }, // 14 hero_attack_lock
	Hand { entry: 0x2f37f8, end: 0x2f3884, hash: 0x9364d0cbcdb8d447, est: 0, f: follow_any }, // 15 follow
	Hand { entry: 0x20c7c8, end: 0x20c8d8, hash: 0x4f5697c59741cfef, est: 0, f: attack_any }, // 15 hero_attack_lock
	Hand { entry: 0x2ef6c0, end: 0x2ef74c, hash: 0x9c5eca717286ce31, est: 0, f: follow_any }, // 16 follow
	Hand { entry: 0x212218, end: 0x212328, hash: 0x52c0b81186ade901, est: 0, f: attack_any }, // 16 hero_attack_lock
	Hand { entry: 0x2fbb58, end: 0x2fbbe4, hash: 0x39af68ae5c32550f, est: 0, f: follow_any }, // 17 follow
	Hand { entry: 0x211828, end: 0x211938, hash: 0x9d39489339dcc20d, est: 0, f: attack_any }, // 17 hero_attack_lock
	Hand { entry: 0x302190, end: 0x30221c, hash: 0x5fc858a93fe8ec0b, est: 0, f: follow_any }, // 18 follow
	Hand { entry: 0x21c540, end: 0x21c650, hash: 0x24022ab3eb60394b, est: 0, f: attack_any }, // 18 hero_attack_lock
];

/// Registers the hands (Retail::load, only when `enabled`).
pub fn register() {
	register_hand(crate::retail::disc::hands(&HANDS));
}

// ---- feel (the old port's tick_modern, camera.rs, the user's own feature) ------------------------------------

const PIVOT: f32 = 0.9;
const DIST: f32 = 5.64;
const ZOOM: (f32, f32) = (1.5, 14.0);
const PITCH: (f32, f32) = (-0.5, 1.3);
const START_PITCH: f32 = 0.35;
const TILT_STEP: f32 = 0.0305;
const ROTATE_RATE: f32 = 0.022_689_3;
const MOUSE: f32 = 0.003;
const FOLLOW: f32 = 0.35;
const RELEASE: f32 = 0.1;
const MIN_LEN: f32 = 0.2;
/// Below this boom length (m) the camera rises over the hero.
const RISE_MIN: f32 = 1.8;
/// The rise ends only once the boom has this much more room than RISE_MIN (m): no on/off bobbing along a wall.
const RISE_BAND: f32 = 0.5;
/// The rise spring (critically damped, rad per field squared factor): no velocity kick when it starts or ends.
const RISE_W: f32 = 0.25;
/// Fields to blend back to the retail camera (toggle off, a gated hero state).
const BLEND: u32 = 20;
/// RMB aim: the orbit pivot (the point the camera looks at) is lifted this far above Ratchet's chest, so the screen centre (the
/// reticle and every shot) lies above his head and he sits in the lower middle of the frame ("Raised, Ratchet lower").
const AIM_LIFT: f32 = 1.5;
/// Fields of the smooth in / out of that lift.
const AIM_BLEND: u32 = 12;
/// Room kept between the lifted pivot and a ceiling above Ratchet (m), and the most the ceiling limit moves in one field.
const AIM_CEIL: f32 = 0.4;
const AIM_CAP_STEP: f32 = 0.15;
/// Hero states that keep the retail camera (camera.md CAM-7, CAM-1): script freeze, grind, swing, slides, and the
/// death states (death_respawn.md section 2: 0x3d/0x47/0x7a-0x7d/0x7f/0x80/0x82, pit tumble 0x77).
const GATE_STATES: [u32; 16] = [0x72, 0x74, 0x2d, 0x29, 0x2a, 0x31, 0x3d, 0x47, 0x77, 0x7a, 0x7b, 0x7c, 0x7d, 0x7f, 0x80, 0x82];
/// Retail options (savegame.md A25/A26): 0 = Up/Down resp. Left/Right camera reversed; the modern camera's mouse and
/// right stick follow them.
const OPT_UD: u32 = 0x15eddc;
const OPT_LR: u32 = 0x15ede0;
/// A hero jump longer than this in one field (respawn, teleport) re-seeds the camera from retail's pose.
const JUMP: f32 = 8.0;
/// Hero camera mode byte 0x1415d4 values that keep the retail camera: slide/rail 3, look 4, swing 7.
const GATE_CAM: [u8; 3] = [3, 4, 7];

const HERO_POS: u32 = 0x13f3d0;
const HERO_YAW: u32 = 0x13f3e8;
const HERO_WISH: u32 = 0x13f4d0;
const HERO_STATE: u32 = 0x1413d4;
const HERO_CAM: u32 = 0x1415d4;
const HERO_MOBY: u32 = 0x1413d0;
/// Hero body form byte: 0 Ratchet, 1 Clank, 2 Giant Clank, 3 Hologuise [C hero_movement.md]. Giant Clank keeps the
/// retail camera (the 0.9 m pivot would sit inside his body).
const BODY_FORM: u32 = 0x1413f4;
const GAME_MODE: u32 = 0x15f5a4;
/// The follow camera's hidden marker moby (class 1007) slot, Novalis / Rilgar address (Eudora 0x166fd4 by `Lv::g`).
const MARKER_SLOT: u32 = 0x1672d4;
const ATK_TARGET: u32 = 0x13fda4;
const ATK_STICK: u32 = 0x13fda8;
const ATK_YAW: u32 = 0x13fdac;
const AIM_ON: u32 = 0x140600;
const AIM_YAW: u32 = 0x14060c;
/// Hero state group word (hero_movement.md section 2): 2 = fall states, 4 = jump states. And the airborne flag (0 = grounded).
const HERO_GROUP: u32 = 0x1413dc;
const HERO_AIRBORNE: u32 = 0x13f65e;

#[derive(Clone, Copy)]
struct St {
	inp: Input,
	rstick: [u8; 2],
	lstick: [u8; 2],
	active: bool,
	field: u32,
	/// The field the follow hand last ran in (a gap means retail ran other camera classes meanwhile: re-seed).
	ran: u32,
	live: bool,
	yaw: f32,
	pitch: f32,
	dist: f32,
	len: f32,
	/// Extra pitch while a wall behind the hero leaves the boom too short (the camera rises over him).
	rise: f32,
	/// The rise's held goal and its spring velocity.
	rise_goal: f32,
	rise_v: f32,
	pivot: [f32; 3],
	/// Last pose written (eye, yaw, pitch) and fields of blend-out left.
	last: ([f32; 3], f32, f32),
	/// The camera rows last written (row 0 forward, 1 left, 2 up): the blend-out start while the view is rolled.
	last_rows: [[f32; 3]; 3],
	/// The orbit frame the yaw / pitch are measured in (hero up and a carried reference axis; world frame on flat ground).
	fr: Frame,
	blend: u32,
	/// RMB aim mode in force (the camera hand ran live with RMB held), and the last value logged.
	aiming: bool,
	aim_logged: bool,
	aim_poked: bool,
	/// Aim lift: fields into the blend (0..=AIM_BLEND), the smoothed ceiling limit and the lift in force (m).
	aim_t: u32,
	lift_cap: f32,
	lift: f32,
	level: u32,
	/// Hero position at the last hand run (respawn / teleport detection).
	hero_last: [f32; 3],
	/// Trace of the last wall test (base reach, safe reach, rise goal) and the moby the boom segment hit (0 = terrain).
	tr: [f32; 3],
	tr_moby: u32,
	/// The dynamic layer's state (lane P; unused with RAC_MODCAM_DYN=0).
	dy: Dy,
	/// Whether the dynamic layer (Smooth) ran last field: a change (Options Static / Smooth, or the env switch) re-seeds `dy`.
	dyn_was: bool,
}

static ST: Mutex<St> = Mutex::new(St {
	inp: Input { look: [0; 2], aim: false, zoom: 0, toggle: false },
	rstick: [0x80; 2],
	lstick: [0x80; 2],
	active: true,
	field: 0,
	ran: u32::MAX,
	live: false,
	yaw: 0.0,
	pitch: START_PITCH,
	dist: DIST,
	len: DIST,
	rise: 0.0,
	rise_goal: 0.0,
	rise_v: 0.0,
	pivot: [0.0; 3],
	last: ([0.0; 3], 0.0, 0.0),
	last_rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
	fr: FLAT,
	blend: 0,
	aiming: false,
	aim_logged: false,
	aim_poked: false,
	aim_t: 0,
	lift_cap: AIM_LIFT,
	lift: 0.0,
	level: 0,
	hero_last: [0.0; 3],
	tr: [0.0; 3],
	tr_moby: 0,
	dy: Dy::ZERO,
	dyn_was: true,
});

/// The moby the last `seg` hit (0 = terrain or no hit), for traces.
static HIT_MOBY: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn rf(cpu: &mut Cpu, a: u32) -> f32 {
	f32::from_bits(cpu.mem.rd32(a))
}
fn wf(cpu: &mut Cpu, a: u32, v: f32) {
	cpu.mem.wr32(a, v.to_bits());
}
fn rv(cpu: &mut Cpu, a: u32) -> [f32; 3] {
	[rf(cpu, a), rf(cpu, a + 4), rf(cpu, a + 8)]
}
fn wv(cpu: &mut Cpu, a: u32, v: [f32; 3]) {
	for (i, x) in v.iter().enumerate() {
		wf(cpu, a + 4 * i as u32, *x);
	}
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn len3(a: [f32; 3]) -> f32 {
	(a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn wrap(a: f32) -> f32 {
	let t = std::f32::consts::TAU;
	let mut a = a % t;
	if a > std::f32::consts::PI {
		a -= t;
	} else if a < -std::f32::consts::PI {
		a += t;
	}
	a
}

/// Camera rows for a view yaw / pitch (pitch > 0 looks down), the convention `retail::read_cam` / basis_euler read
/// back: row0 forward, row1 left, row2 up (euler = [r1.z atan2 r2.z, asin(-r0.z), r0.y atan2 r0.x]).
pub fn rows(yaw: f32, pitch: f32) -> [[f32; 3]; 3] {
	let (sy, cy) = yaw.sin_cos();
	let (sp, cp) = pitch.sin_cos();
	[[cy * cp, sy * cp, -sp], [-sy, cy, 0.0], [cy * sp, sy * sp, cp]]
}

/// The eye for a pivot, view yaw / pitch and boom length.
#[cfg(test)]
pub fn eye(pivot: [f32; 3], yaw: f32, pitch: f32, len: f32) -> [f32; 3] {
	let f = rows(yaw, pitch)[0];
	[pivot[0] - f[0] * len, pivot[1] - f[1] * len, pivot[2] - f[2] * len]
}

/// The orbit frame: yaw is measured from `a` toward `b`, pitch toward `u` (hero up), `b = u x a`. On flat ground (`u` = world z,
/// `a` = world x) it is the world frame and every formula below is the plain `rows` / `eye` (bit-exact with the old camera).
/// On a magnetic wall retail's smoothed hero up (0x167230, camera.md) turns away from z: the mouse then orbits Ratchet's own up
/// and the camera rolls with it, as retail's follow camera does (the world turns around an upright hero) [C 0x167230, I frame].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
	pub a: [f32; 3],
	pub b: [f32; 3],
	pub u: [f32; 3],
}

pub const FLAT: Frame = Frame { a: [1.0, 0.0, 0.0], b: [0.0, 1.0, 0.0], u: [0.0, 0.0, 1.0] };

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit3(a: [f32; 3]) -> [f32; 3] {
	let l = len3(a).max(1e-12);
	[a[0] / l, a[1] / l, a[2] / l]
}

impl Frame {
	pub fn flat(&self) -> bool {
		*self == FLAT
	}

	/// The frame for hero up `u`, carrying the reference axis `a` of the last field (parallel transport: `a` projected onto the new
	/// tangent plane, so the camera turns by the least angle as the wall curves). Returns the frame and the yaw to add (the turn
	/// of the carried axis against world x when `u` came back to world z, folded into the yaw so the heading stays).
	fn carry(a: [f32; 3], u: [f32; 3]) -> (Frame, f32) {
		if u == [0.0, 0.0, 1.0] {
			return (FLAT, if a == FLAT.a { 0.0 } else { a[1].atan2(a[0]) });
		}
		let d = dot3(a, u);
		let mut p = [a[0] - u[0] * d, a[1] - u[1] * d, a[2] - u[2] * d];
		if len3(p) < 1e-3 {
			// a was along u: any axis across it (world x, else world y).
			let t = if u[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
			let d = dot3(t, u);
			p = [t[0] - u[0] * d, t[1] - u[1] * d, t[2] - u[2] * d];
		}
		let a = unit3(p);
		(Frame { a, b: cross3(u, a), u }, 0.0)
	}

	/// Camera rows for yaw / pitch in this frame (`rows` on flat ground).
	pub fn rows(&self, yaw: f32, pitch: f32) -> [[f32; 3]; 3] {
		if self.flat() {
			return rows(yaw, pitch);
		}
		let (sy, cy) = yaw.sin_cos();
		let (sp, cp) = pitch.sin_cos();
		let comb = |x: f32, y: f32, z: f32| [self.a[0] * x + self.b[0] * y + self.u[0] * z, self.a[1] * x + self.b[1] * y + self.u[1] * z, self.a[2] * x + self.b[2] * y + self.u[2] * z];
		[comb(cy * cp, sy * cp, -sp), comb(-sy, cy, 0.0), comb(cy * sp, sy * sp, cp)]
	}

	/// The eye for a pivot, yaw / pitch and boom length (`eye` on flat ground).
	pub fn eye(&self, pivot: [f32; 3], yaw: f32, pitch: f32, len: f32) -> [f32; 3] {
		let f = self.rows(yaw, pitch)[0];
		[pivot[0] - f[0] * len, pivot[1] - f[1] * len, pivot[2] - f[2] * len]
	}

	/// `p` moved `k` along hero up.
	fn lift(&self, p: [f32; 3], k: f32) -> [f32; 3] {
		if self.flat() {
			[p[0], p[1], p[2] + k]
		} else {
			[p[0] + self.u[0] * k, p[1] + self.u[1] * k, p[2] + self.u[2] * k]
		}
	}
}

/// Retail's smoothed hero up (0x167230 at Novalis, the camera's own up source: spring k 0.015 toward -0x13f5e0, camera.md): world z
/// on the ground and whenever the vector is not a unit vector (level load); snapped to exactly z within 1e-4 so flat ground runs
/// the unchanged formulas.
fn hero_up(c: &mut Cpu, lv: &Lv) -> [f32; 3] {
	let u = rv(c, lv.g(0x167230));
	let l = len3(u);
	if !l.is_finite() || (l - 1.0).abs() > 0.1 {
		return [0.0, 0.0, 1.0];
	}
	let u = [u[0] / l, u[1] / l, u[2] / l];
	if len3(sub(u, [0.0, 0.0, 1.0])) < 1e-4 {
		[0.0, 0.0, 1.0]
	} else {
		u
	}
}

/// Every register a nested retail call may change (the follow update's results stay as retail left them).
pub(super) struct Regs {
	r: [u128; 32],
	hi: u128,
	lo: u128,
	sa: u32,
	f: [u32; 32],
	facc: u32,
	fcr31: u32,
	vf: [[u32; 4]; 32],
	vacc: [u32; 4],
	vi: [u32; 32],
}

impl Regs {
	pub(super) fn save(c: &Cpu) -> Regs {
		Regs { r: c.r, hi: c.hi, lo: c.lo, sa: c.sa, f: c.f, facc: c.facc, fcr31: c.fcr31, vf: c.vf, vacc: c.vacc, vi: c.vi }
	}
	pub(super) fn restore(&self, c: &mut Cpu) {
		(c.r, c.hi, c.lo, c.sa, c.f, c.facc, c.fcr31, c.vf, c.vacc, c.vi) = (self.r, self.hi, self.lo, self.sa, self.f, self.facc, self.fcr31, self.vf, self.vacc, self.vi);
	}
}

fn ram(c: &Cpu, a: u32, n: u32) -> Vec<u8> {
	let a = (a & 0x01ff_ffff) as usize;
	c.mem.ram[a..a + n as usize].to_vec()
}
fn put(c: &mut Cpu, a: u32, b: &[u8]) {
	let a = (a & 0x01ff_ffff) as usize;
	c.mem.ram[a..a + b.len()].copy_from_slice(b);
}

/// Live moby addresses (retail.rs `table`, stride 0x100).
fn mobys(c: &mut Cpu, level: u32) -> Vec<u32> {
	let (s, e) = super::table(level);
	let (s, e) = (c.mem.rd32(s) & 0x01ff_ffff, c.mem.rd32(e) & 0x01ff_ffff);
	if s == 0 || e < s || e - s > 0x100 * 4096 {
		return Vec::new();
	}
	(s..e).step_by(0x100).collect()
}

/// Retail's segment test from `a` to `b` (camera mask 0xb4): the contact point on a hit. Its side effects on game
/// state are undone (result block, terrain cell cache, moby touch ring and the mobys' +0xa4 touch index).
fn seg(c: &mut Cpu, lv: &Lv, a: [f32; 3], b: [f32; 3]) -> Option<[f32; 3]> {
	seg_mask(c, lv, a, b, 0xb4)
}

/// `seg` with the line-test mask as a parameter (aim.rs casts the crosshair ray with it).
fn seg_mask(c: &mut Cpu, lv: &Lv, a: [f32; 3], b: [f32; 3], mask: u32) -> Option<[f32; 3]> {
	let res = ram(c, lv.res, 0x80);
	let ring = ram(c, lv.ring, 0x1000);
	let ms = mobys(c, lv.level);
	let touch: Vec<u8> = ms.iter().map(|&m| c.mem.rd8(m + 0xa4)).collect();
	let sp = c.g(29) as u32;
	let buf = (sp.wrapping_sub(0x200)) & !0xf;
	let stack = ram(c, buf.wrapping_sub(0x400), 0x600);
	wv(c, buf, a);
	wf(c, buf + 12, 1.0);
	wv(c, buf + 0x10, b);
	wf(c, buf + 0x1c, 1.0);
	c.s(29, buf.wrapping_sub(0x40) as i32 as i64 as u64);
	c.s(4, buf as i32 as i64 as u64);
	c.s(5, (buf + 0x10) as i32 as i64 as u64);
	c.s(6, mask as u64);
	// The hero's own moby is not a wall.
	let hero = c.mem.rd32(HERO_MOBY);
	c.s(7, hero as i32 as i64 as u64);
	// No collision-event record (t0 = 0, as retail's plain line tests pass): with a stale t0 the test copies 0x24 bytes from
	// it into the event ring when it meets a moby with flag 0x4000 (FUN_001e24d8 tail), a fault when t0 holds no pointer.
	c.s(8, 0);
	// Nor is the follow camera's hidden marker (class 1007, slot 0x166fd4 Eudora / 0x1672d4 Rilgar, created at the
	// camera position by cam_helper_sync FUN_001ddcd0 and kept there by its update): it rides on the eye, so a boom
	// test that sees it pulls in, the marker follows, the boom releases into it again: the camera pulsed (cam_pulse.md).
	// Retail's own end sphere passes it as the ignored moby (FUN_001e35c8 a3, overlay.elf.c:172530); here the hero takes
	// that argument, so its collision pointer +0x94 (seg_collide skips mobys whose +0x94 is 0) is cleared for the call.
	let mk = c.mem.rd32(lv.g(MARKER_SLOT)) & 0x01ff_ffff;
	let mk_col = (mk != 0).then(|| c.mem.rd32(mk + 0x94));
	if mk != 0 {
		c.mem.wr32(mk + 0x94, 0);
	}
	c.call(lv.seg);
	if let Some(v) = mk_col {
		c.mem.wr32(mk + 0x94, v);
	}
	let hit = c.g(2) as u32 & 1 != 0;
	let p = rv(c, lv.res + 0x20);
	HIT_MOBY.store(if hit { c.mem.rd32(lv.res + 0x18) } else { 0 }, Ordering::Relaxed);
	put(c, lv.res, &res);
	put(c, lv.ring, &ring);
	for (&m, &t) in ms.iter().zip(&touch) {
		c.mem.wr8(m + 0xa4, t);
	}
	put(c, buf.wrapping_sub(0x400), &stack);
	c.s(29, sp as i32 as i64 as u64);
	hit.then_some(p)
}

/// The first hand-back gate that holds (modcam.md 1.4), or None when the modern camera may drive.
fn why_retail(c: &mut Cpu, lv: &Lv, rec: u32) -> Option<String> {
	let class = c.mem.rd16(rec + 0x86) as i16;
	let blend = c.mem.rd32(lv.blend());
	let mode = c.mem.rd32(va(GAME_MODE));
	let cam = c.mem.rd8(HERO_CAM);
	let hs = c.mem.rd32(HERO_STATE);
	if mode != 0 {
		Some(format!("game mode {mode}"))
	} else if c.mem.rd32(lv.gate()) != 0 {
		Some("cinematic gate".into())
	} else if (1..=3).contains(&blend) {
		Some(format!("camera switch blend {blend}"))
	} else if class != 0 {
		Some(format!("camera class {class}"))
	} else if GATE_CAM.contains(&cam) {
		Some(format!("hero camera mode {cam}"))
	} else if GATE_STATES.contains(&hs) {
		Some(format!("hero state {hs:#x}"))
	} else if c.mem.rd8(BODY_FORM) == 2 {
		Some("giant clank".into())
	} else {
		None
	}
}

/// RAC_MODCAM_SENS (default 1.0), read once.
fn sens() -> f32 {
	static S: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_MODCAM_SENS").ok().and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(1.0))
}

/// Writes a pose into the camera record.
fn write_pose(c: &mut Cpu, rec: u32, e: [f32; 3], fr: &Frame, yaw: f32, pitch: f32) -> [[f32; 3]; 3] {
	let r = fr.rows(yaw, pitch);
	write_rows(c, rec, e, &r);
	r
}

fn write_rows(c: &mut Cpu, rec: u32, e: [f32; 3], r: &[[f32; 3]; 3]) {
	for (i, row) in r.iter().enumerate() {
		wv(c, rec + 0x10 * i as u32, *row);
	}
	wv(c, rec + 0x30, e);
}

/// Yaw / pitch of the record's rows (retail.rs read_cam convention).
fn rec_angles(c: &mut Cpu, rec: u32) -> (f32, f32) {
	let r0 = rv(c, rec);
	(r0[1].atan2(r0[0]), (-r0[2]).clamp(-1.0, 1.0).asin())
}

struct Guard;
impl Drop for Guard {
	fn drop(&mut self) {
		IN.store(false, Ordering::SeqCst);
	}
}

// ---- dynamic layer (lane P, 2026-10-20: "more dynamic, Elden Ring style") ------------------------------------------------------
// The pivot follows the hero through critically damped springs (horizontal fast, vertical slow, a jump does not bob it, a leash
// keeps the hero framed); after a pause without camera input and while the hero runs, the view eases behind the run direction and
// toward a default elevation (with a little look down at drops / falls and up on slopes); the boom breathes with run speed and open
// space and eases out after a wall; the strafe lock-on frames hero and target; the right stick ramps up. Mouse look stays direct.
// RAC_MODCAM_DYN=0 runs the plain Modern camera of before: every dynamic line of `step` sits behind `dynamic`, the old arithmetic
// is untouched. Constants: RAC_TUNE="mc_h_time=0.12,mc_rc_delay=2" (devtune, read once; names and defaults in `tune()`).

/// The PC options' Camera row (pcext.rs): Smooth (true, the default) = the dynamic camera, Static = the plain Modern camera. Live:
/// the next field follows. RAC_MODCAM_DYN, when set, wins (the exactness scripts run with RAC_MODCAM_DYN=0).
pub fn set_smooth(on: bool) {
	SMOOTH.store(on, Ordering::Relaxed);
}

static SMOOTH: AtomicBool = AtomicBool::new(true);

/// Whether the dynamic layer runs: RAC_MODCAM_DYN ("0" = the plain Modern camera, read once) wins when set, else the Camera row.
fn dyn_on() -> bool {
	static D: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
	dyn_resolve(*D.get_or_init(|| std::env::var("RAC_MODCAM_DYN").ok().map(|v| v != "0")), SMOOTH.load(Ordering::Relaxed))
}

/// The env switch (when set) wins over the Camera row.
fn dyn_resolve(env: Option<bool>, smooth: bool) -> bool {
	env.unwrap_or(smooth)
}

/// RAC_MODCAM_LOG=1: one stderr line per field with the camera state (proof logs), read once.
fn log_on() -> bool {
	static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*D.get_or_init(|| std::env::var("RAC_MODCAM_LOG").is_ok_and(|v| v != "0" && !v.is_empty()))
}

/// Seconds a field is: the disc's own field rate (PAL 0.02 s, USA 1001/60000 s), so every spring and rate below (all in seconds)
/// feels the same on both discs.
#[inline]
fn dt() -> f32 {
	super::disc::field_dt()
}

struct Tune {
	/// Pivot spring smoothing times (s): horizontal, vertical.
	h_time: f32,
	v_time: f32,
	/// Share of a jump's rise the pivot follows before landing (falls below the takeoff follow fully).
	air_follow: f32,
	/// Leash (m): the pivot never lags the hero's chest by more than this sideways, with the hero above it, or with it above him.
	leash_h: f32,
	leash_up: f32,
	leash_dn: f32,
	/// Smoothing of the hero velocity estimate (s).
	vel_time: f32,
	/// Recentre: idle seconds before it starts, ease-in time, rate (1/s) base and extra at 90 deg sideways, the highest turn rate
	/// (rad/s), the lowest run speed (m/s), the most it swings per episode (rad).
	rc_delay: f32,
	rc_ramp: f32,
	rc_base: f32,
	rc_side: f32,
	rc_max: f32,
	rc_speed: f32,
	rc_budget: f32,
	/// Default elevation (rad, + looks down), the dead zone around it that is left alone (rad), its ease rate (1/s) and the lowest
	/// speed it works at.
	pitch_def: f32,
	pitch_dz: f32,
	pitch_k: f32,
	pitch_speed: f32,
	/// Look-up on slopes (rad per unit slope, most), look-down at a drop ahead and while falling (rad), bias in / out times (s).
	bias_up_k: f32,
	bias_up_max: f32,
	bias_drop: f32,
	bias_fall: f32,
	bias_in: f32,
	bias_out: f32,
	/// Drop probe: base and per-speed reach ahead (m), the depth without ground that counts as a drop (m).
	drop_ahead: f32,
	drop_ahead_k: f32,
	drop_depth: f32,
	/// Boom length: extra share at run speed, factor in tight / open space, smoothing (s), probe length (m), ease-out time (s).
	dist_run: f32,
	dist_open_min: f32,
	dist_open_max: f32,
	dist_tau: f32,
	open_ray: f32,
	boom_out: f32,
	/// Right stick ramp: rise and fall rate (full deflection per second).
	ramp_up: f32,
	ramp_dn: f32,
	/// Lock-on framing: pivot share toward the target sideways / up, the target's rise above centre (m), the largest shift (m),
	/// extra boom for far targets (start m, per m, most m), ease in / out (s), hold after the lock drops (s).
	frame_side: f32,
	frame_up_k: f32,
	frame_rise: f32,
	frame_max: f32,
	frame_far_start: f32,
	frame_far_k: f32,
	frame_far_max: f32,
	frame_tau_in: f32,
	frame_tau_out: f32,
	frame_hold: f32,
	/// Safety layer (bit flags, 3 = all, 0 = off = the camera of dev s85, for A/B proofs): 1 = the lagged pivot is clipped by a wall
	/// test from the hero's chest, 2 = the boom never ends past a contact (no `MIN_LEN` floor); the margin kept off a surface (m).
	safe: f32,
	safe_margin: f32,
}

fn tune() -> &'static Tune {
	static T: std::sync::OnceLock<Tune> = std::sync::OnceLock::new();
	T.get_or_init(|| {
		let f = |n: &str, d: f32| crate::devtune::f32(&format!("mc_{n}"), d);
		Tune {
			h_time: f("h_time", 0.10),
			v_time: f("v_time", 0.28),
			air_follow: f("air_follow", 0.35),
			leash_h: f("leash_h", 1.6),
			leash_up: f("leash_up", 1.5),
			leash_dn: f("leash_dn", 1.0),
			vel_time: f("vel_time", 0.12),
			rc_delay: f("rc_delay", 1.2),
			rc_ramp: f("rc_ramp", 0.5),
			rc_base: f("rc_base", 0.35),
			rc_side: f("rc_side", 1.6),
			rc_max: f("rc_max", 0.9),
			rc_speed: f("rc_speed", 3.0),
			rc_budget: f("rc_budget", 1.3),
			pitch_def: f("pitch_def", 0.33),
			pitch_dz: f("pitch_dz", 0.10),
			pitch_k: f("pitch_k", 0.6),
			pitch_speed: f("pitch_speed", 1.5),
			bias_up_k: f("bias_up_k", 0.30),
			bias_up_max: f("bias_up_max", 0.12),
			bias_drop: f("bias_drop", 0.16),
			bias_fall: f("bias_fall", 0.22),
			bias_in: f("bias_in", 0.35),
			bias_out: f("bias_out", 0.5),
			drop_ahead: f("drop_ahead", 0.6),
			drop_ahead_k: f("drop_ahead_k", 0.28),
			drop_depth: f("drop_depth", 3.0),
			dist_run: f("dist_run", 0.10),
			dist_open_min: f("dist_open_min", 0.82),
			dist_open_max: f("dist_open_max", 1.10),
			dist_tau: f("dist_tau", 0.6),
			open_ray: f("open_ray", 7.0),
			boom_out: f("boom_out", 0.2),
			ramp_up: f("ramp_up", 4.0),
			ramp_dn: f("ramp_dn", 12.0),
			frame_side: f("frame_side", 0.45),
			frame_up_k: f("frame_up_k", 0.06),
			frame_rise: f("frame_rise", 0.30),
			frame_max: f("frame_max", 2.0),
			frame_far_start: f("frame_far_start", 8.0),
			frame_far_k: f("frame_far_k", 0.12),
			frame_far_max: f("frame_far_max", 2.0),
			frame_tau_in: f("frame_tau_in", 0.3),
			frame_tau_out: f("frame_tau_out", 0.45),
			frame_hold: f("frame_hold", 0.3),
			safe: f("safe", 3.0),
			safe_margin: f("safe_margin", 0.2),
		}
	})
}

/// The dynamic layer's per-camera state.
#[derive(Clone, Copy)]
struct Dy {
	/// The field `dyn_track` last ran in (a gap re-seeds the velocity) and the hero position then.
	field: u32,
	hero_prev: [f32; 3],
	/// Smoothed hero velocity (m/s, world) and the pivot spring's velocity.
	hv: [f32; 3],
	pv: [f32; 3],
	/// Fields without camera input.
	idle: u32,
	/// Fields in a row the hero was airborne in a jump / fall state; `is_air` = at least 2 (debounce).
	air: u32,
	is_air: bool,
	/// At the last grounded field: the hero goal's and the pivot's height along hero up (the takeoff reference).
	ref_h: f32,
	ref_p: f32,
	/// Horizontal speed and vertical speed along hero up (m/s, smoothed).
	speed: f32,
	vz: f32,
	/// Recentre turned so far this episode (rad).
	rc_used: f32,
	/// Pitch bias (rad, + looks down).
	bias: f32,
	/// A drop lies ahead (probe, every 3rd field) and its smoothed 0..1 weight.
	drop: bool,
	drop_w: f32,
	/// Free share (0..1) of the left, right and up probes, the boom length factor, the boom's ease-out velocity.
	open_s: [f32; 3],
	dist_f: f32,
	len_v: f32,
	/// Lock framing: fields left of the hold, the last lock-on point, the pivot shift and the extra boom length.
	lock_hold: u32,
	lock_pt: [f32; 3],
	frame_off: [f32; 3],
	frame_len: f32,
	/// The ramped right stick.
	rs: [f32; 2],
	/// The pivot the camera orbits this field: the spring's pivot clipped at walls and floors between it and the hero's chest.
	pivc: [f32; 3],
}

impl Dy {
	const ZERO: Dy = Dy {
		field: 0,
		hero_prev: [0.0; 3],
		hv: [0.0; 3],
		pv: [0.0; 3],
		idle: 0,
		air: 0,
		is_air: false,
		ref_h: 0.0,
		ref_p: 0.0,
		speed: 0.0,
		vz: 0.0,
		rc_used: 0.0,
		bias: 0.0,
		drop: false,
		drop_w: 0.0,
		open_s: [1.0; 3],
		dist_f: 1.0,
		len_v: 0.0,
		lock_hold: 0,
		lock_pt: [0.0; 3],
		frame_off: [0.0; 3],
		frame_len: 0.0,
		rs: [0.0; 2],
		pivc: [0.0; 3],
	};
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale3(a: [f32; 3], k: f32) -> [f32; 3] {
	[a[0] * k, a[1] * k, a[2] * k]
}
fn sstep(x: f32) -> f32 {
	let x = x.clamp(0.0, 1.0);
	x * x * (3.0 - 2.0 * x)
}
/// The share of the way to a goal one field covers with a first-order lag of `tau` seconds.
fn k_of(tau: f32) -> f32 {
	if tau <= 1e-4 {
		1.0
	} else {
		1.0 - (-dt() / tau).exp()
	}
}
/// One field of a critically damped spring on the offset `y` (velocity `v`) from its goal, smoothing time `ts` (w = 2 / ts, the
/// exact solution, so no overshoot and no step-size trouble): (y, v) afterwards.
fn crit(y: f32, v: f32, ts: f32) -> (f32, f32) {
	let w = 2.0 / ts.max(0.02);
	let e = (-w * dt()).exp();
	let t = v + w * y;
	((y + t * dt()) * e, (v - w * t * dt()) * e)
}
/// A stick axis `tgt` ramped from `r`: up at `ramp_up` full deflections a second, down (or through zero) at `ramp_dn`.
fn ramp1(r: f32, tgt: f32) -> f32 {
	let t = tune();
	let rate = if tgt.abs() > r.abs() && tgt * r >= 0.0 { t.ramp_up } else { t.ramp_dn };
	let m = rate * dt();
	r + (tgt - r).clamp(-m, m)
}

/// Every field, before the camera moves: the hero's velocity, jump / fall state, the idle time (`input` = the camera was used).
fn dyn_track(c: &mut Cpu, st: &mut St, fr: &Frame, hero: [f32; 3], hero_goal: [f32; 3], input: bool) {
	let t = tune();
	let field = st.field;
	let piv_h = dot3(st.pivot, fr.u);
	let d = &mut st.dy;
	let dp = sub(hero, d.hero_prev);
	let gap = field.wrapping_sub(d.field) != 1 || len3(dp) > JUMP;
	d.field = field;
	d.hero_prev = hero;
	if gap {
		d.hv = [0.0; 3];
	} else {
		let a = k_of(t.vel_time);
		for i in 0..3 {
			d.hv[i] += (dp[i] / dt() - d.hv[i]) * a;
		}
	}
	d.speed = dot3(d.hv, fr.a).hypot(dot3(d.hv, fr.b));
	d.vz = dot3(d.hv, fr.u);
	// A jump or fall state with no ground contact; two fields in a row (a bump on a slope is not a jump).
	let flying = matches!(c.mem.rd32(HERO_GROUP), 2 | 4) && c.mem.rd16(HERO_AIRBORNE) != 0;
	d.air = if flying { d.air.saturating_add(1) } else { 0 };
	d.is_air = d.air >= 2;
	if d.air == 0 {
		d.ref_h = dot3(hero_goal, fr.u);
		d.ref_p = piv_h;
	}
	d.idle = if input { 0 } else { d.idle.saturating_add(1) };
	if input || d.speed < t.rc_speed * 0.5 {
		d.rc_used = 0.0;
	}
}

/// The pivot follows the hero goal (chest) through critically damped springs: fast sideways, slower vertically; while a jump is in
/// the air the vertical goal follows only `air_follow` of the rise above the takeoff (falls below it follow fully); a leash keeps
/// the hero within `leash_*` of the pivot. Not used in RMB aim (the old exponential follow keeps that feel).
fn dyn_follow(st: &mut St, fr: &Frame, hero_goal: [f32; 3]) {
	let t = tune();
	let u = fr.u;
	let d = &mut st.dy;
	let hh = dot3(hero_goal, u);
	let mut goal = hero_goal;
	if d.is_air {
		let off = hh - d.ref_h;
		let gh = d.ref_p + if off > 0.0 { off * t.air_follow } else { off };
		goal = add3(goal, scale3(u, gh - hh));
	}
	let y = sub(st.pivot, goal);
	let (yv, vv) = (dot3(y, u), dot3(d.pv, u));
	let yh = sub(y, scale3(u, yv));
	let vh = sub(d.pv, scale3(u, vv));
	let (mut nyh, mut nvh) = ([0.0f32; 3], [0.0f32; 3]);
	for i in 0..3 {
		(nyh[i], nvh[i]) = crit(yh[i], vh[i], t.h_time);
	}
	let (nyv, nvv) = crit(yv, vv, t.v_time);
	let mut pv = add3(nvh, scale3(u, nvv));
	// Leash against the real hero (not the partial jump goal): never lose him.
	let e = add3(sub(goal, hero_goal), add3(nyh, scale3(u, nyv)));
	let ev = dot3(e, u);
	let mut eh = sub(e, scale3(u, ev));
	let mut ev2 = ev;
	let hvv = dot3(d.hv, u);
	let lh = len3(eh);
	if lh > t.leash_h {
		eh = scale3(eh, t.leash_h / lh);
		pv = add3(sub(d.hv, scale3(u, hvv)), scale3(u, dot3(pv, u)));
	}
	if ev > t.leash_dn || ev < -t.leash_up {
		ev2 = ev.clamp(-t.leash_up, t.leash_dn);
		pv = add3(sub(pv, scale3(u, dot3(pv, u))), scale3(u, hvv));
	}
	st.pivot = add3(hero_goal, add3(eh, scale3(u, ev2)));
	d.pv = pv;
}

/// Lazy recentre and default elevation (no camera input for `rc_delay` s, the hero running, no RMB aim): the yaw eases toward the
/// run direction, faster the more sideways he runs, not at all toward the camera, at most `rc_budget` per episode; the pitch eases
/// toward `pitch_def`.
fn dyn_auto(st: &mut St, fr: &Frame) {
	let t = tune();
	let idle_s = st.dy.idle as f32 * dt();
	let sp = st.dy.speed;
	if idle_s < t.rc_delay {
		return;
	}
	let ease = sstep((idle_s - t.rc_delay) / t.rc_ramp);
	if sp >= t.rc_speed && st.dy.rc_used < t.rc_budget {
		let (vx, vy) = (dot3(st.dy.hv, fr.a), dot3(st.dy.hv, fr.b));
		let ang = wrap(vy.atan2(vx) - st.yaw);
		// Running toward the camera (more than about 110 deg off the view direction): no swing.
		let gate = sstep((ang.cos() + 0.5) / 0.3);
		// The swing tapers off over the last 0.4 rad of its budget (no sudden stop).
		let taper = sstep((t.rc_budget - st.dy.rc_used) / 0.4);
		let rate = (t.rc_base + t.rc_side * ang.sin().abs()) * gate * ease * sstep((sp - t.rc_speed) / 2.0) * taper;
		let lim = t.rc_max * dt();
		let turn = (ang * (1.0 - (-rate * dt()).exp())).clamp(-lim, lim);
		st.yaw = wrap(st.yaw + turn);
		st.dy.rc_used += turn.abs();
	}
	if sp >= t.pitch_speed {
		// Only the part outside the dead zone around the default pulls (a pitch the player chose nearby stays).
		let err = t.pitch_def - st.pitch;
		st.pitch += (err - err.clamp(-t.pitch_dz, t.pitch_dz)) * k_of(1.0 / t.pitch_k) * ease;
	}
}

/// The look-up / look-down bias: a little up on uphill runs, down at a drop ahead and while falling. Eased, never touches the
/// player's own pitch; fades fast in RMB aim.
fn dyn_bias(st: &mut St) {
	let t = tune();
	let aiming = st.aiming;
	let d = &mut st.dy;
	let mut g = 0.0f32;
	if !aiming {
		if !d.is_air && d.speed > 1.5 && d.vz > 0.0 {
			g -= (d.vz / d.speed * t.bias_up_k).min(t.bias_up_max);
		}
		g += t.bias_drop * d.drop_w;
		if d.is_air && d.vz < -3.0 {
			g += t.bias_fall * ((-d.vz - 3.0) / 10.0).clamp(0.0, 1.0);
		}
	}
	let tau = if aiming { 0.12 } else if g.abs() > d.bias.abs() { t.bias_in } else { t.bias_out };
	d.bias += (g - d.bias) * k_of(tau);
}

/// Retail segment probes (cheap: one or two per field, none in RMB aim): the drop ahead of a running hero (every 3rd field, ground
/// missing within `drop_depth`) and, one direction per field (left, right, up), how open the space around the pivot is.
fn dyn_probe(c: &mut Cpu, lv: &Lv, st: &mut St, fr: &Frame, hero: [f32; 3]) {
	let t = tune();
	if st.aiming {
		st.dy.drop = false;
		st.dy.drop_w += (0.0 - st.dy.drop_w) * k_of(0.15);
		return;
	}
	if st.dy.speed > 1.5 && !st.dy.is_air {
		if st.field % 3 == 0 {
			let (vx, vy) = (dot3(st.dy.hv, fr.a), dot3(st.dy.hv, fr.b));
			let l = vx.hypot(vy).max(1e-3);
			let ahead = t.drop_ahead + t.drop_ahead_k * st.dy.speed;
			let dirw = add3(scale3(fr.a, vx / l), scale3(fr.b, vy / l));
			let s = add3(add3(hero, scale3(dirw, ahead)), scale3(fr.u, 0.7));
			let e = add3(s, scale3(fr.u, -(0.7 + t.drop_depth)));
			st.dy.drop = seg(c, lv, s, e).is_none();
		}
	} else {
		st.dy.drop = false;
	}
	st.dy.drop_w += ((st.dy.drop as u8 as f32) - st.dy.drop_w) * k_of(0.3);
	let o = fr.lift(st.dy.pivc, st.lift);
	let r = fr.rows(st.yaw, st.pitch);
	let i = (st.field % 3) as usize;
	let dir = [r[1], scale3(r[1], -1.0), fr.u][i];
	st.dy.open_s[i] = match seg(c, lv, o, add3(o, scale3(dir, t.open_ray))) {
		Some(h) => (len3(sub(h, o)) / t.open_ray).min(1.0),
		None => 1.0,
	};
}

/// The boom length factor: a bit longer at run speed and in open space, shorter in tight spaces; 1.0 (fast) in RMB aim.
fn dyn_dist(st: &mut St) {
	let t = tune();
	let s = st.dy.open_s;
	let open = 0.45 * s[0] + 0.45 * s[1] + 0.10 * s[2];
	let f_open = t.dist_open_min + (t.dist_open_max - t.dist_open_min) * open;
	let f_run = 1.0 + t.dist_run * (st.dy.speed / 6.0).clamp(0.0, 1.0);
	let (goal, tau) = if st.aiming { (1.0, 0.2) } else { (f_open * f_run, t.dist_tau) };
	st.dy.dist_f += (goal - st.dy.dist_f) * k_of(tau);
}

/// Lock-on framing (RMB aim with a strafe lock): the pivot shifts toward the target (hero and target stay framed, the target a
/// little above the centre) and the boom pulls back for far targets; the camera's rotation, and so the aim, is untouched. Eases
/// back when the lock ends.
fn dyn_frame(c: &mut Cpu, lv: &Lv, st: &mut St, fr: &Frame, hero_goal: [f32; 3]) {
	let t = tune();
	let lock = if st.aiming { super::aim::strafe_lock_for_cam(c, st.field) } else { None };
	match lock {
		Some((_, p)) => {
			st.dy.lock_hold = (t.frame_hold / dt()).round() as u32;
			st.dy.lock_pt = p;
		}
		None => st.dy.lock_hold = st.dy.lock_hold.saturating_sub(1),
	}
	let have = st.aiming && st.dy.lock_hold > 0;
	let (mut want, mut want_len) = ([0.0f32; 3], 0.0f32);
	if have {
		let p0 = fr.lift(st.dy.pivc, st.lift);
		let rel = sub(st.dy.lock_pt, p0);
		let r = fr.rows(st.yaw, st.pitch);
		let (right, up) = (scale3(r[1], -1.0), r[2]);
		let off_r = (dot3(rel, right) * t.frame_side).clamp(-t.frame_max, t.frame_max);
		let off_u = (dot3(rel, up) * t.frame_up_k - t.frame_rise).clamp(-0.6, 0.4);
		want = add3(scale3(right, off_r), scale3(up, off_u));
		want_len = ((len3(sub(st.dy.lock_pt, hero_goal)) - t.frame_far_start) * t.frame_far_k).clamp(0.0, t.frame_far_max);
		// Not through a wall: the shift stops 0.2 m short of the first contact.
		let m = len3(want);
		if m > 0.05 {
			if let Some(h) = seg(c, lv, p0, add3(p0, want)) {
				want = scale3(want, ((len3(sub(h, p0)) - 0.2).max(0.0) / m).min(1.0));
			}
		}
	}
	let k = k_of(if have { t.frame_tau_in } else { t.frame_tau_out });
	for i in 0..3 {
		st.dy.frame_off[i] += (want[i] - st.dy.frame_off[i]) * k;
	}
	st.dy.frame_len += (want_len - st.dy.frame_len) * k;
}

/// The segment test from `from` to `to`, extended `safe_margin` beyond `to`: on a contact the point stops `safe_margin` short of it
/// (never past `to`). The extension makes the clip continuous: it starts the moment `to` comes within the margin of a surface, so
/// nothing pops when it begins or ends.
fn clip_line(c: &mut Cpu, lv: &Lv, from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
	let d = sub(to, from);
	let m = len3(d);
	if m < 0.05 {
		return to;
	}
	let k = tune().safe_margin;
	let u = scale3(d, 1.0 / m);
	match seg(c, lv, from, add3(to, scale3(u, k))) {
		Some(h) => add3(from, scale3(u, (len3(sub(h, from)) - k).clamp(0.0, m))),
		None => to,
	}
}

/// Safety, step 1: the spring pivot lags the hero's chest (soft follow; a jump follows only 35 % of its rise), so on a climb it
/// ends up under the ledge he stands on or inside the step in front of him. A boom test from there starts inside the rock: it
/// collapses to `MIN_LEN` and the eye sits in the terrain (the whole screen dark green). The pivot is therefore tested from the
/// chest (retail's segment test) and stops `safe_margin` short of the first contact. The spring state itself is not changed: it
/// keeps its smooth catch-up and the pivot comes out of the clip with no pop.
fn dyn_clear(c: &mut Cpu, lv: &Lv, chest: [f32; 3], pivot: [f32; 3]) -> [f32; 3] {
	clip_line(c, lv, chest, pivot)
}

/// Proof log: where the retail segment test meets the line chest -> eye (both directions) and chest -> pivot, as the fraction of the
/// line (9 = free). Chest -> pivot below 1, or a boom collapsed to `MIN_LEN`, is a camera inside the terrain; chest -> eye below 1 alone
/// is only the hero hidden behind a corner (a lip he jumps over, a wall he leans on) and is not clipped.
fn dbg_chk(c: &mut Cpu, lv: &Lv, chest: [f32; 3], pivot: [f32; 3], eye: [f32; 3]) -> [f32; 3] {
	let fr = |c: &mut Cpu, a: [f32; 3], b: [f32; 3]| {
		let m = len3(sub(b, a));
		if m < 0.05 {
			return 9.0;
		}
		seg(c, lv, a, b).map_or(9.0, |h| len3(sub(h, a)) / m)
	};
	[fr(c, chest, eye), fr(c, eye, chest), fr(c, chest, pivot)]
}

/// Wrapping hand on the follow camera update: retail first, then the modern pose.
fn follow(c: &mut Cpu, lv: &'static Lv) -> bool {
	if IN.swap(true, Ordering::SeqCst) {
		return false;
	}
	let _g = Guard;
	let rec = c.g(4) as u32 & 0x01ff_ffff;
	c.call(lv.follow);
	let regs = Regs::save(c);
	if let Ok(mut st) = ST.lock() {
		if rec == c.mem.rd32(lv.cur()) & 0x01ff_ffff {
			step(c, lv, rec, &mut st);
		}
	}
	regs.restore(c);
	true
}

fn step(c: &mut Cpu, lv: &'static Lv, rec: u32, st: &mut St) {
	// A new level re-seeds. A gap with the follow class still current (pause, vendor, menus: the camera main did not
	// run) resumes the modern pose; another class in between already ended `live` (post_field).
	let fresh = st.level != lv.level;
	st.ran = st.field;
	st.level = lv.level;
	let hero = rv(c, HERO_POS);
	// Respawn / teleport: no blend from the old place; seed from retail's new pose (retail resets its camera there).
	let jumped = !fresh && len3(sub(hero, st.hero_last)) > JUMP;
	st.hero_last = hero;
	if jumped {
		st.blend = 0;
		if st.live {
			st.live = false;
			eprintln!("retail: modcam re-seeds after a hero jump (respawn / teleport, field {})", st.field);
		}
	}
	let why = if st.active { why_retail(c, lv, rec) } else { Some("toggled off (F7)".into()) };
	if let Some(why) = why {
		st.aiming = false;
		(st.aim_t, st.lift, st.lift_cap) = (0, 0.0, AIM_LIFT);
		if st.live {
			st.live = false;
			st.blend = BLEND;
			eprintln!("retail: modcam hands back to retail: {why} (field {})", st.field);
		}
		if st.blend > 0 && !fresh {
			// Blend from the last modern pose to retail's.
			st.blend -= 1;
			let t = 1.0 - st.blend as f32 / BLEND as f32;
			let t = t * t * (3.0 - 2.0 * t);
			let re = rv(c, rec + 0x30);
			let (e0, y0, p0) = st.last;
			let e = [e0[0] + (re[0] - e0[0]) * t, e0[1] + (re[1] - e0[1]) * t, e0[2] + (re[2] - e0[2]) * t];
			let rr = [rv(c, rec), rv(c, rec + 0x10), rv(c, rec + 0x20)];
			if st.fr.flat() && rr[1][2].abs() < 1e-4 {
				let (ry, rp) = rec_angles(c, rec);
				let y = y0 + wrap(ry - y0) * t;
				let p = p0 + (rp - p0) * t;
				st.last_rows = write_pose(c, rec, e, &FLAT, y, p);
				st.last = (e, y, p);
			} else {
				// A rolled view (magnetic wall) on either end: blend the rows themselves and keep them orthonormal.
				let l = st.last_rows;
				let mix = |i: usize| [l[i][0] + (rr[i][0] - l[i][0]) * t, l[i][1] + (rr[i][1] - l[i][1]) * t, l[i][2] + (rr[i][2] - l[i][2]) * t];
				let f = unit3(mix(0));
				let m2 = mix(2);
				let d = dot3(m2, f);
				let u = unit3([m2[0] - f[0] * d, m2[1] - f[1] * d, m2[2] - f[2] * d]);
				let rows3 = [f, cross3(u, f), u];
				write_rows(c, rec, e, &rows3);
				st.last_rows = rows3;
				st.last = (e, f[1].atan2(f[0]), (-f[2]).clamp(-1.0, 1.0).asin());
			}
		} else {
			st.blend = 0;
		}
		return;
	}
	// Orbit frame from retail's smoothed hero up (flat ground: the world frame, nothing below changes).
	let (fr, turn) = Frame::carry(if fresh || !st.live { FLAT.a } else { st.fr.a }, hero_up(c, lv));
	st.yaw = wrap(st.yaw + turn);
	st.fr = fr;
	let flat = fr.flat();
	let piv_goal = fr.lift(hero, PIVOT);
	if !st.live || fresh {
		// Seed from the pose on screen (retail's, or the blend-out in progress): no snap.
		let e = if st.blend > 0 && !fresh { st.last.0 } else { rv(c, rec + 0x30) };
		let d = sub(e, piv_goal);
		let l = len3(d).max(MIN_LEN);
		st.yaw = (-dot3(d, fr.b)).atan2(-dot3(d, fr.a));
		st.pitch = (dot3(d, fr.u) / l).clamp(-1.0, 1.0).asin().clamp(PITCH.0, PITCH.1);
		st.len = l.min(ZOOM.1);
		st.pivot = piv_goal;
		(st.rise, st.rise_v, st.rise_goal) = (0.0, 0.0, 0.0);
		if !st.live {
			eprintln!("retail: modcam drives the follow camera (field {})", st.field);
		}
		st.live = true;
		st.blend = 0;
		(st.aim_t, st.lift, st.lift_cap) = (0, 0.0, AIM_LIFT);
		st.dy = Dy::ZERO;
		st.dy.hero_prev = hero;
		st.dy.field = st.field.wrapping_sub(1);
	}
	let dynamic = dyn_on();
	if dynamic != st.dyn_was {
		// Static <-> Smooth flipped in the menu: the springs start afresh from the pose on screen (the pivot is shared, so the eye does
		// not jump); leaving Smooth keeps the look-up / look-down bias it had in the pitch.
		if !dynamic && st.dy.bias != 0.0 {
			st.pitch = (st.pitch + st.dy.bias).clamp(PITCH.0, PITCH.1);
		}
		st.dy = Dy::ZERO;
		st.dy.hero_prev = hero;
		st.dy.field = st.field.wrapping_sub(1);
		st.dyn_was = dynamic;
		eprintln!("retail: modcam feel {} (field {})", if dynamic { "Smooth" } else { "Static" }, st.field);
	}
	// Input: mouse (0.003 rad a count, RAC_MODCAM_SENS scales), the right stick (keys Q/E turn the view left / right,
	// Z/X tilt it up / down), wheel zoom.
	let sens = sens();
	let inp = st.inp;
	let rx = (st.rstick[0] as f32 - 128.0) / 127.0;
	let ry = (st.rstick[1] as f32 - 128.0) / 127.0;
	let dz = |v: f32| if v.abs() < 0.2 { 0.0 } else { v };
	let lr = if c.mem.rd8(OPT_LR) == 0 { -1.0 } else { 1.0 };
	let ud = if c.mem.rd8(OPT_UD) == 0 { -1.0 } else { 1.0 };
	// The Camera speed row (pcext.rs camopts section): percent / 50, exactly 1.0 until the player sets it.
	let k = super::pcext::camspeed_scale();
	// Dynamic: the mouse stays direct, the right stick ramps up (a gentle acceleration; keys Q/E Z/X ramp too).
	let (sx, sy) = if dynamic {
		st.dy.rs = [ramp1(st.dy.rs[0], dz(rx)), ramp1(st.dy.rs[1], dz(ry))];
		(st.dy.rs[0], st.dy.rs[1])
	} else {
		(dz(rx), dz(ry))
	};
	let dyaw = (-(inp.look[0] as f32) * MOUSE * sens * k + sx * ROTATE_RATE * k) * lr;
	let dpitch = (inp.look[1] as f32 * MOUSE * sens * k - sy * TILT_STEP * k) * ud;
	st.dist = (st.dist - inp.zoom as f32 * 0.5).clamp(ZOOM.0, ZOOM.1);
	// Aim mode (RMB): the mouse keeps turning the camera as it does without RMB; nothing is picked or framed.
	// Aim mode is a ground feature: on a magnetic wall retail stows the guns (0x1413fc) and the facing / aim yaw words are world-z.
	st.aiming = inp.aim && super::pcext::strafe_on() && flat;
	if st.aiming != st.aim_logged {
		st.aim_logged = st.aiming;
		eprintln!("retail: modcam aim {} (field {})", if st.aiming { "on" } else { "off" }, st.field);
	}
	let piv = piv_goal;
	let mut goal_len = st.dist;
	st.yaw = wrap(st.yaw + dyaw);
	st.pitch = (st.pitch + dpitch).clamp(PITCH.0, PITCH.1);
	if dynamic {
		let input = inp.look != [0, 0] || dz(rx) != 0.0 || dz(ry) != 0.0 || inp.zoom != 0 || inp.aim;
		dyn_track(c, st, &fr, hero, piv, input);
		if !st.aiming {
			dyn_auto(st, &fr);
			st.pitch = st.pitch.clamp(PITCH.0, PITCH.1);
		}
	}
	if st.aiming {
		face(c, st, st.yaw);
	}
	if dynamic && !st.aiming && st.aim_t == 0 {
		dyn_follow(st, &fr, piv);
	} else {
		let old = st.pivot;
		for i in 0..3 {
			st.pivot[i] += (piv[i] - st.pivot[i]) * FOLLOW;
		}
		if dynamic {
			// Aim keeps the old follow; the spring takes over from the velocity it left.
			st.dy.pv = [(st.pivot[0] - old[0]) / dt(), (st.pivot[1] - old[1]) / dt(), (st.pivot[2] - old[2]) / dt()];
		}
	}
	// Big hero jumps (respawn, teleport): follow at once.
	if len3(sub(piv, st.pivot)) > 8.0 {
		st.pivot = piv;
		st.dy.pv = [0.0; 3];
	}
	// The pivot the camera orbits: the spring's, clipped at what lies between it and the hero's chest (dynamic only).
	st.dy.pivc = if dynamic && (tune().safe as u32) & 1 != 0 { dyn_clear(c, lv, piv, st.pivot) } else { st.pivot };
	// Wall: pull in at once to the hit (minus 0.2), release outward RELEASE a field. With a wall close behind the hero
	// (boom under RISE_MIN) the camera rises over him (pitch up in 0.3 rad steps, the first that gives room) instead of
	// sitting in his head; the rise ends once the boom has RISE_BAND more room (hysteresis), and it moves on a critically
	// damped spring (no bobbing at the threshold, no jolt when it starts or ends).
	// Aim lift: the camera orbits a pivot AIM_LIFT above Ratchet's chest (blended in and out over AIM_BLEND fields, limited by a
	// ceiling over him: retail's segment test up from the chest), so the centre ray clears his head. Released: lift 0, the same pose.
	if !flat {
		(st.aim_t, st.lift, st.lift_cap) = (0, 0.0, AIM_LIFT);
	}
	st.aim_t = if st.aiming { (st.aim_t + 1).min(AIM_BLEND) } else { st.aim_t.saturating_sub(1) };
	if st.aim_t > 0 {
		let up = [st.dy.pivc[0], st.dy.pivc[1], st.dy.pivc[2] + AIM_LIFT + AIM_CEIL];
		let goal = match seg(c, lv, st.dy.pivc, up) {
			Some(h) => (h[2] - st.dy.pivc[2] - AIM_CEIL).clamp(0.0, AIM_LIFT),
			None => AIM_LIFT,
		};
		st.lift_cap += (goal - st.lift_cap).clamp(-AIM_CAP_STEP, AIM_CAP_STEP);
		let t = st.aim_t as f32 / AIM_BLEND as f32;
		st.lift = (AIM_LIFT * t * t * (3.0 - 2.0 * t)).min(st.lift_cap);
	} else {
		st.lift = 0.0;
	}
	let mut pivot_r = fr.lift(st.dy.pivc, st.lift);
	if dynamic {
		dyn_probe(c, lv, st, &fr, hero);
		dyn_bias(st);
		dyn_dist(st);
		dyn_frame(c, lv, st, &fr, piv);
		pivot_r = add3(pivot_r, st.dy.frame_off);
		goal_len = (st.dist * st.dy.dist_f + st.dy.frame_len).clamp(ZOOM.0, ZOOM.1);
	}
	let (pivot, yaw) = (pivot_r, st.yaw);
	let reach = |c: &mut Cpu, p: f32| match seg(c, lv, pivot, fr.eye(pivot, yaw, p, goal_len)) {
		// Dynamic: never past the contact itself (the floor of MIN_LEN would put the eye in the wall when the contact is closer).
		Some(h) if dynamic && (tune().safe as u32) & 2 != 0 => {
			let l = len3(sub(h, pivot));
			(l - 0.2).max(MIN_LEN.min(0.5 * l))
		}
		Some(h) => (len3(sub(h, pivot)) - 0.2).max(MIN_LEN),
		None => goal_len,
	};
	let base = reach(c, st.pitch);
	st.tr_moby = HIT_MOBY.load(Ordering::Relaxed);
	let (lo, hi) = (RISE_MIN.min(goal_len), (RISE_MIN + RISE_BAND).min(goal_len));
	let mut rise_goal = st.rise_goal;
	if base >= hi || st.pitch >= PITCH.1 {
		rise_goal = 0.0;
	} else if base < lo {
		rise_goal = 0.0;
		let mut best = base;
		for k in 1..=4 {
			let p = (st.pitch + 0.3 * k as f32).min(PITCH.1);
			let r = reach(c, p);
			if r > best + 0.3 {
				best = r;
				rise_goal = p - st.pitch;
			}
			if r >= RISE_MIN.min(goal_len) || p >= PITCH.1 {
				break;
			}
		}
	}
	st.rise_goal = rise_goal;
	st.rise_v += RISE_W * RISE_W * (rise_goal - st.rise) - 2.0 * RISE_W * st.rise_v;
	st.rise += st.rise_v;
	let pitch = if dynamic { (st.pitch + st.rise + st.dy.bias).clamp(PITCH.0, PITCH.1) } else { (st.pitch + st.rise).min(PITCH.1) };
	let safe = if (pitch - st.pitch).abs() < 1e-4 { base } else { reach(c, pitch) };
	st.len = if safe < st.len {
		st.dy.len_v = 0.0;
		safe
	} else if dynamic && !st.aiming {
		// Dynamic: after a wall the boom eases back out on a critically damped spring (about 0.5 s, no pop, no jolt).
		let (y, v) = crit(st.len - safe, st.dy.len_v, tune().boom_out);
		st.dy.len_v = v;
		(safe + y).min(safe)
	} else {
		(st.len + RELEASE).min(safe)
	};
	st.tr = [base, safe, rise_goal];
	let e = fr.eye(pivot_r, st.yaw, pitch, st.len);
	st.last_rows = write_pose(c, rec, e, &fr, st.yaw, pitch);
	st.last = (e, st.yaw, pitch);
	if log_on() {
		let chk = dbg_chk(c, lv, piv, pivot_r, e);
		let d = &st.dy;
		eprintln!(
			"modcam-log f={} dyn={} yaw={:.5} pitch={:.5} out={:.5} bias={:.4} len={:.5} goal={:.4} safe={:.4} piv={:.3},{:.3},{:.3} hero={:.3},{:.3},{:.3} sp={:.2} vz={:.2} idle={} air={} drop={:.2} open={:.2},{:.2},{:.2} lock={} off={:.2},{:.2},{:.2} flen={:.2} aim={} rc={:.2} chk={:.3},{:.3},{:.3} pc={:.3},{:.3},{:.3} eye={:.3},{:.3},{:.3} cl={:.3}",
			st.field, dynamic as u8, st.yaw, st.pitch, pitch, d.bias, st.len, goal_len, safe, st.pivot[0], st.pivot[1], st.pivot[2], hero[0], hero[1], hero[2], d.speed, d.vz, d.idle, d.is_air as u8, d.drop_w, d.open_s[0], d.open_s[1], d.open_s[2], d.lock_hold, d.frame_off[0], d.frame_off[1], d.frame_off[2], d.frame_len, st.aiming as u8, d.rc_used, chk[0], chk[1], chk[2], d.pivc[0], d.pivc[1], d.pivc[2], e[0], e[1], e[2], len3(sub(d.pivc, st.pivot))
		);
	}
}

/// Aiming: Ratchet faces `want` (the camera's horizontal direction). Standing (stick idle), his facing and wished yaw turn
/// to it at once (retail's own state); moving, RAC_MODCAM_STRAFE=0 leaves him running where the stick points, otherwise
/// his drawn facing turns to it (moby yaw +0x48; the next hero update rebuilds it from his own yaw, so his path stays the
/// stick's = strafe).
///
/// Stick released, the drawn facing stays on the aim too in the stop skid (state 3, after a strafe sideways or backwards
/// retail skids along the old path: drawn from the logic yaw he spun round to the path for the whole skid = the "slip") and in
/// the strafe flips (state 0xb, whose logic facing turns to the flip heading), in the run states' field the stick is released
/// (2 / 0x73, before the skid or stand starts) and in the first standing field (the hero
/// commit drew the old yaw before the facing above was set). Draw only: logic keeps retail's skid and flip.
fn face(c: &mut Cpu, st: &St, want: f32) {
	let st_hero = c.mem.rd32(HERO_STATE);
	let idle = st.lstick.iter().all(|&v| (v as i32 - 128).abs() < 40);
	let strafe = std::env::var("RAC_MODCAM_STRAFE").map_or(true, |v| v != "0");
	let m = c.mem.rd32(HERO_MOBY) & 0x01ff_ffff;
	if idle && matches!(st_hero, 0 | 0x1e) {
		wf(c, HERO_YAW, wrap(want));
		wf(c, HERO_WISH, wrap(want));
		if strafe && m != 0 {
			wf(c, m + 0x48, wrap(want));
		}
	} else if strafe && (!idle || matches!(st_hero, 2 | 0x73 | 3 | 0xb)) && m != 0 {
		wf(c, m + 0x48, wrap(want));
	}
}

/// Wrapping hand on hero_attack_lock: retail picks its wrench target as always. With RMB aiming and no target found, the
/// swing turns to the camera yaw: the same two words retail sets from stick input (0x13fda8 = 1, 0x13fdac = yaw).
fn attack(c: &mut Cpu, lv: &'static Lv) -> bool {
	if IN_ATK.swap(true, Ordering::SeqCst) {
		return false;
	}
	c.call(lv.attack);
	IN_ATK.store(false, Ordering::SeqCst);
	ATK_RUNS.fetch_add(1, Ordering::Relaxed);
	let Ok(st) = ST.lock() else { return true };
	if st.active && st.aiming && st.live && st.level == lv.level && c.mem.rd32(ATK_TARGET) & 0x01ff_ffff == 0 {
		c.mem.wr32(ATK_STICK, 1);
		wf(c, ATK_YAW, wrap(st.yaw));
	}
	true
}

/// The camera yaw Ratchet faces while RMB aim is in force (the modern camera drives and aims), else None. strafe.rs reads it
/// for the flips and the torso twist: modcam's own yaw state, not RAM 0x166ed8, so it is not a field behind.
pub fn aim_yaw() -> Option<f32> {
	let st = ST.lock().ok()?;
	(enabled() && st.active && st.aiming && st.live).then(|| wrap(st.yaw))
}

/// The camera while RMB aim is in force, for aim.rs (crosshair ray): the pose last written (eye, forward = row 0), the pivot
/// (the orbit pivot: Ratchet's chest, smoothed, plus the aim lift) and the field counter (the cache key of one field's ray).
#[derive(Clone, Copy, Debug)]
pub struct AimCam {
	pub eye: [f32; 3],
	pub fwd: [f32; 3],
	pub pivot: [f32; 3],
	pub field: u32,
}

/// None unless `aim_yaw()` is Some (the same state: modern camera driving, RMB held).
pub fn aim_cam() -> Option<AimCam> {
	let st = ST.lock().ok()?;
	if !(enabled() && st.active && st.aiming && st.live) {
		return None;
	}
	let (eye, yaw, pitch) = st.last;
	let o = st.dy.frame_off;
	Some(AimCam { eye, fwd: rows(yaw, pitch)[0], pivot: [st.dy.pivc[0] + o[0], st.dy.pivc[1] + o[1], st.dy.pivc[2] + st.lift + o[2]], field: st.field })
}

/// Retail's segment test (seg_collide of the running level) from `a` to `b` with `mask`: the contact point and the moby hit
/// (0 = terrain). Side effects on game state and every register are undone, so a hand may call it mid-function.
pub fn cast(c: &mut Cpu, a: [f32; 3], b: [f32; 3], mask: u32) -> Option<([f32; 3], u32)> {
	let lv = lv_of(c.mem.ram[0x15ed84] as u32)?;
	let regs = Regs::save(c);
	let hit = seg_mask(c, lv, a, b, mask);
	let moby = HIT_MOBY.load(Ordering::Relaxed);
	regs.restore(c);
	hit.map(|p| (p, moby))
}

/// The screen-sprite queue of a level (base of the slots, stride 0x30, and the count word; Novalis 0x169440 / 0x169500 by `Lv::g`).
pub fn sprite_queue(level: u32) -> Option<(u32, u32)> {
	let lv = lv_of(level)?;
	Some((lv.g(0x169440), lv.g(0x169500)))
}

/// Worker, before each field: the field's input, the F7 toggle, and the gun aim override while aiming.
pub fn pre_field(m: &mut ee_oracle::Machine, fin: &super::FieldIn) {
	if !enabled() {
		return;
	}
	let Ok(mut st) = ST.lock() else { return };
	st.field = st.field.wrapping_add(1);
	st.inp = fin.mc;
	st.rstick = [fin.sticks[0], fin.sticks[1]];
	st.lstick = [fin.sticks[2], fin.sticks[3]];
	if fin.mc.toggle {
		st.active = !st.active;
		eprintln!("retail: modern camera {}", if st.active { "ON" } else { "OFF" });
		if let Ok(mut u) = UI.lock() {
			u.on = st.active;
		}
	}
	let aiming = st.active && st.aiming && st.live && st.level == m.level() && lv_of(m.level()).is_some() && m.rd32(va(GAME_MODE)) == 0;
	if aiming {
		// Retail's manual-aim override: the gun updaters read 0x14060c instead of the hero-forward yaw as the shot and
		// target-search direction (their own search and green reticle run on it).
		m.cpu.mem.wr32(AIM_ON, 1);
		m.cpu.mem.wr32(AIM_YAW, wrap(st.yaw).to_bits());
		st.aim_poked = true;
	} else if st.aim_poked {
		m.cpu.mem.wr32(AIM_ON, 0);
		st.aim_poked = false;
	}
}

/// Worker, after each field: hand back when retail ran another camera class.
pub fn post_field(m: &ee_oracle::Machine) {
	if !enabled() {
		return;
	}
	if let (Ok(mut st), Some(lv)) = (ST.lock(), lv_of(m.level())) {
		// Retail ran another camera class this field (scripted, crank, swing, free look...): the follow update and so the
		// hand did not run; retail's own switch blend starts from the last modern pose left in the follow record.
		let rec = m.rd32(lv.cur()) & 0x01ff_ffff;
		let class = if rec != 0 { m.rd16(rec + 0x86) as i16 } else { -1 };
		if st.ran != st.field && class != 0 {
			// A blend-out still pending would otherwise resume later from a stale pose.
			st.blend = 0;
		}
		if st.live && st.ran != st.field && class != 0 {
			st.live = false;
			st.aiming = false;
			eprintln!("retail: modcam hands back to retail: camera class {class} (game mode {}, field {})", m.rd32(va(GAME_MODE)), st.field);
		}
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rows_orthonormal_and_read_back() {
		for &(y, p) in &[(0.3f32, 0.2f32), (-2.0, -0.4), (3.0, 1.2)] {
			let r = rows(y, p);
			let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
			for i in 0..3 {
				assert!((dot(r[i], r[i]) - 1.0).abs() < 1e-5);
				for j in 0..i {
					assert!(dot(r[i], r[j]).abs() < 1e-5);
				}
			}
			// read_cam / basis_euler's Euler.
			let e = [r[1][2].atan2(r[2][2]), (-r[0][2]).asin(), r[0][1].atan2(r[0][0])];
			assert!(e[0].abs() < 1e-5 && (e[1] - p).abs() < 1e-5 && wrap(e[2] - y).abs() < 1e-5);
			// The eye sits behind and (pitch > 0) above the pivot.
			let ey = eye([0.0; 3], y, p, 5.0);
			assert!((len3(ey) - 5.0).abs() < 1e-4 && (ey[2] > 0.0) == (p > 0.0));
		}
	}

	#[test]
	fn feed_parses() {
		let f = parse_feed("10-20:5,-3,1;30-31:0,0,0,2");
		assert_eq!(f.len(), 2);
		assert_eq!(f[0], (10, 20, Input { look: [5, -3], aim: true, zoom: 0, toggle: false }));
		assert_eq!(f[1].2.zoom, 2);
	}

	#[test]
	fn camera_feel_switch_resolution() {
		// The Camera row (Static / Smooth) decides unless RAC_MODCAM_DYN is set, which wins either way.
		assert!(dyn_resolve(None, true), "Smooth: the dynamic layer");
		assert!(!dyn_resolve(None, false), "Static: the plain Modern camera");
		assert!(!dyn_resolve(Some(false), true), "RAC_MODCAM_DYN=0 forces Static");
		assert!(dyn_resolve(Some(true), false), "RAC_MODCAM_DYN=1 forces Smooth");
	}

	#[test]
	fn dynamic_spring_and_ramp() {
		// The pivot spring is critically damped: it converges and never overshoots, whatever the start velocity sign.
		let (mut y, mut v) = (1.0f32, 0.0f32);
		for _ in 0..100 {
			(y, v) = crit(y, v, 0.28);
			assert!(y >= 0.0);
		}
		assert!(y < 1e-3 && v.abs() < 1e-2);
		// The right stick ramps to full deflection in 1 / ramp_up s and lets go faster.
		let mut r = 0.0f32;
		for _ in 0..13 {
			r = ramp1(r, 1.0);
		}
		assert!((r - 1.0).abs() < 1e-5);
		assert!(ramp1(1.0, 0.0) < 1.0 && 1.0 - ramp1(1.0, 0.0) > ramp1(0.0, 1.0));
		assert_eq!(sstep(2.0), 1.0);
		assert!((k_of(0.5) - (1.0 - (-0.04f32).exp())).abs() < 1e-6);
	}

	fn reset_st() {
		let mut st = ST.lock().unwrap();
		st.active = true;
		st.live = false;
		st.aiming = false;
		st.blend = 0;
		st.level = u32::MAX;
	}

	/// Per-phase steadiness of the modern camera: boom length range, the largest one-field boom change, boom direction
	/// reversals (pull-in / release cycles), eye jerk (largest second difference) and the moby hits of the wall test.
	#[derive(Default, Debug)]
	struct Steady {
		fields: u32,
		len_min: f32,
		len_max: f32,
		dlen_max: f32,
		reversals: u32,
		jerk_max: f32,
		rise_max: f32,
		rise_flips: u32,
		mobys: Vec<(u32, u16)>,
	}

	fn step_field(m: &mut ee_oracle::Machine, buttons: u16, sticks: [u8; 4], mc: Input) {
		let fin = crate::retail::FieldIn { buttons, sticks, travel: None, mc };
		m.set_pad(buttons, sticks);
		pre_field(m, &fin);
		m.frame().expect("frame");
		post_field(m);
		let _ = crate::retail::take_sound_of(m);
	}

}
