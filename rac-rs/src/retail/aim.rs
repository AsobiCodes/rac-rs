//! Crosshair and aimed shots for the strafe mode (PC extra the user asked for, 2026-10-13, model: Ratchet: Gladiator, third-person
//! aiming): while RMB is held with the modern camera and the strafe movement on (`strafe::enabled()` and `modcam::aim_yaw()`
//! is Some, the same state that turns the torso and starts the flips), a retail reticle sits at the screen centre and every
//! direct-fire shot of the hero leaves the muzzle toward the point under it. RMB released, the movement set to classic, the
//! modern camera off, menus, cutscenes, death, vehicles: `active()` is false, nothing is written and every weapon runs
//! retail's own code (auto-lock and all). Per-weapon decisions: scratch_aim/AIM.md.
//!
//! - Crosshair (only for the items that shoot straight, `aims_straight`): `pre_field` appends one entry to retail's screen-sprite queue (FUN_001e07c8's slot layout, Eudora 0x169140 stride
//!   0x30, count 0x169200; `modcam::sprite_queue`) with retail's lock-on reticle (texture id 0x26, colour 0xff0fff0f, scale 1) and
//!   no position, which retail's own consumer (FUN_001e08a8) draws at the screen centre.
//! - Aim point: one ray per field from the point of the camera ray level with Ratchet's chest (so nothing between the camera and
//!   him blocks it) along the camera's forward axis (the screen centre), cast with retail's segment test (`modcam::cast`); the
//!   contact point (terrain or moby) is the aim point, else the point `RANGE` out (the ray is cast in pieces of `CHUNK`).
//! - Strafe lock-on (PC extra with the wide lock-on, `strafe_lock`): the Blaster's target search, which retail's code would answer
//!   with its own cone about the hero's heading, looks for the nearest actor in a cone about the crosshair line instead; the shot
//!   flies to it and the crosshair icon moves onto it, red (the same reticle sprite in retail's other colour). No target: the green
//!   crosshair and the shot to the point under it, as before. RAC_LOCKON_CLASSIC=1 / RAC_PC_EXT=0: none of it.
//! - Hooks (hash-gated wrappers per level, rows made by scratch_aim/mkaim.py from the twin tables): each overrides only the
//!   aim or target input of one retail function and then runs the retail function; with `active()` false it returns at once and
//!   the retail code runs untouched.
use super::{modcam, strafe};
use ee_oracle::cpu::Cpu;
use ee_oracle::rec::{register_hand, Hand};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use crate::retail::disc::va;

const HERO_YAW: u32 = 0x13f3e8;
/// The hero's heading vector (cos yaw, sin yaw, 0, 0): the direction the Pyrocitor flame leaves along.
const HERO_HEADING: u32 = 0x13f9c0;
const HERO_MOBY: u32 = 0x1413d0;
const BODY_FORM: u32 = 0x1413f4;
const GAME_MODE: u32 = 0x15f5a4;
/// Bomb Glove lock-on target word (hero state 0x23's entry turns the hero toward it).
const LOCK_TARGET: u32 = 0x13fda0;
/// Retail's lock-on reticle: texture id 0x26 (circle with three tabs), colour 0xff0fff0f (green), request FUN_001e07c8.
const RETICLE_TEX: u32 = 0x26;
const RETICLE_COLOUR: u32 = 0xff0f_ff0f;
/// The strafe lock-on icon (a target locked): the same sprite in retail's other reticle colour, 0xff0f0fff (red; the Blaster's
/// first-person reticle word, OV:125878).
const RETICLE_LOCKED: u32 = 0xff0f_0fff;
/// The item in hand (inventory id, glove_prereq_inventory_input.md: equipped 0x140408).
const EQUIPPED: u32 = 0x140408;

/// Items that fire straight along the aim and so get the crosshair: the weapons retail holds out in its aimed fire stance
/// (hero_weapon_stance FUN_0022e8d0, called by the held updaters of 9 Suck Cannon, 11 Devastator, 16 Pyrocitor, 19 Tesla
/// Claw, 21 Morph-o-ray and 23 R.Y.N.O. and by the fire dispatcher's case 15 Blaster), plus 13 Visibomb Gun (the missile
/// leaves straight ahead before it is steered). No crosshair for the fire dispatcher's thrown group (10 Bomb Glove, 17 Mine
/// Glove, 20 Glove of Doom, 24 Drone Device, 25 Decoy Glove: hero throw state 0x23, lobbed or dropped), the melee items (8
/// OmniWrench, 18 Walloper), 14 Taunter (no shot) and the gadgets. Only the reticle depends on this; the hooks below do not.
fn aims_straight(item: u32) -> bool {
	matches!(item, 9 | 11 | 13 | 15 | 16 | 19 | 21 | 23)
}
/// Retail's seg_collide finds nothing on a segment longer than about 40 units (measured: 40 hits the ground, 120 hits nothing), so the
/// crosshair ray is cast in consecutive pieces of CHUNK units (nearest contact of the first piece that hits) up to RANGE units from
/// the chest; no contact = the point RANGE out. RAC_AIM_FAR overrides the range (tests).
const CHUNK: f32 = 38.0;
const RANGE: f32 = 190.0;
/// Radius floor (game units) of the sphere an actor presents to the crosshair ray (RAC_AIM_R overrides, tests): retail's actors carry
/// a lock-on point (target record +0x10 above the moby origin, 0.5 without a record) and often a radius of 0.
const ACTOR_R: f32 = 1.0;
/// Actors nearer than this along the ray are the hero's own (held weapon, attachments).
const ACTOR_MIN_T: f32 = 2.5;
/// The line-test masks of the crosshair ray (RAC_AIM_MASK overrides, tests): 0xb4 = the camera's terrain test, 0 = mobys.
const MASKS: [u32; 2] = [0xb4, 0];
const PTR: u32 = 0x01ff_ffff;

/// A pointer register as an address the memory accepts: scratchpad (0x7000_0000..) as is, everything else cut to RAM.
fn ptr(v: u64) -> u32 {
	let a = v as u32;
	if (0x7000_0000..0x7000_4000).contains(&a) {
		a
	} else {
		a & PTR
	}
}

/// The switch: RMB aim in force with the strafe movement on (modern camera driving, on foot as Ratchet). Everything below asks
/// this and nothing else.
pub fn active() -> bool {
	strafe::enabled() && modcam::aim_yaw().is_some()
}

/// The line-test masks of the crosshair ray; the nearest contact of all of them is the aim point (RAC_AIM_MASK=hex,hex,..).
fn masks() -> &'static [u32] {
	static M: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
	M.get_or_init(|| {
		let v: Vec<u32> = std::env::var("RAC_AIM_MASK")
			.ok()
			.map(|v| v.split(',').filter_map(|x| u32::from_str_radix(x.trim().trim_start_matches("0x"), 16).ok()).collect())
			.unwrap_or_default();
		if v.is_empty() {
			MASKS.to_vec()
		} else {
			v
		}
	})
}

/// Registers the hooks (Retail::load); nothing when both the strafe movement and the wide lock-on are off.
pub fn register() {
	// RAC_AIM_OFF=1 (tests): no hooks at all, the baseline of the released-button comparison.
	if (strafe::enabled() || lockon_wide()) && std::env::var_os("RAC_AIM_OFF").is_none() {
		register_hand(crate::retail::disc::hands(&HANDS));
	}
}

/// Wide Blaster lock-on (PC extra, the Gladiator-like reach the user asked for): on unless RAC_LOCKON_CLASSIC=1 or the PC
/// extensions are off (RAC_PC_EXT=0, USA with RAC_USA_EXTRAS=0). Off, the hook returns false and retail's own search runs.
pub fn lockon_wide() -> bool {
	static W: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*W.get_or_init(|| super::pcext::enabled() && !std::env::var("RAC_LOCKON_CLASSIC").is_ok_and(|v| v != "0" && !v.is_empty()))
}

// ---- the aim point ----------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Aim {
	pos: [f32; 3],
	/// The moby under the crosshair (0 = terrain or nothing).
	moby: u32,
	hit: bool,
}

struct S {
	field: u32,
	aim: Option<Aim>,
	/// Fields run since the load (log lines) and the shot being followed in the log (moby, fields left).
	tick: u32,
	track: u32,
	track_left: u32,
	/// The moby the aim picked when the followed shot was made (log only), and the last lock-on word logged.
	victim: u32,
	lock: u32,
	/// Strafe lock-on: the reticle slot `pre_field` queued (field, address; 0 = none) and the target the Blaster search found
	/// (moby, field).
	slot: (u32, u32),
	slock: Option<(u32, u32)>,
	log: Option<std::fs::File>,
	log_tried: bool,
}

static ST: Mutex<S> = Mutex::new(S { field: u32::MAX, aim: None, tick: 0, track: 0, track_left: 0, victim: 0, lock: u32::MAX, slot: (0, 0), slock: None, log: None, log_tried: false });

/// Whether the log (RAC_AIM_LOG=path, tests) is on; opens it on the first call.
fn log_on(s: &mut S) -> bool {
	if !s.log_tried {
		s.log_tried = true;
		if let Ok(p) = std::env::var("RAC_AIM_LOG") {
			s.log = std::fs::File::create(p).ok();
		}
	}
	s.log.is_some()
}

fn log(s: &mut S, line: &str) {
	if log_on(s) {
		if let Some(f) = s.log.as_mut() {
			let _ = writeln!(f, "{line}");
		}
	}
}

fn rf(c: &mut Cpu, a: u32) -> f32 {
	f32::from_bits(c.mem.rd32(a))
}
fn rv(c: &mut Cpu, a: u32) -> [f32; 3] {
	[rf(c, a), rf(c, a + 4), rf(c, a + 8)]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn at(o: [f32; 3], d: [f32; 3], t: f32) -> [f32; 3] {
	[o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t]
}
/// A tuning value from the environment (tests), else the default.
fn envf(name: &str, d: f32) -> f32 {
	std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

/// The point under the crosshair this field (cached per field: several hooks ask in one field).
fn aim_point(c: &mut Cpu) -> Option<Aim> {
	let cam = modcam::aim_cam()?;
	if let Ok(s) = ST.lock() {
		if s.field == cam.field {
			return s.aim;
		}
	}
	let t0 = dot([cam.pivot[0] - cam.eye[0], cam.pivot[1] - cam.eye[1], cam.pivot[2] - cam.eye[2]], cam.fwd).max(0.0);
	let a = at(cam.eye, cam.fwd, t0);
	let range = envf("RAC_AIM_FAR", RANGE);
	let snap = Snap::take(c);
	let mut best: Option<(f32, [f32; 3], u32)> = None;
	let mut s0 = 0.0f32;
	while s0 < range && best.is_none() {
		let s1 = (s0 + CHUNK).min(range);
		let (p0, p1) = (at(a, cam.fwd, s0), at(a, cam.fwd, s1));
		for &m in masks() {
			if let Some((p, moby)) = modcam::cast(c, p0, p1, m) {
				let d = (p[0] - p0[0]).powi(2) + (p[1] - p0[1]).powi(2) + (p[2] - p0[2]).powi(2);
				if best.is_none_or(|x| d < x.0) {
					best = Some((d, p, moby));
				}
			}
		}
		s0 = s1;
	}
	let hit_t = best.map(|(_, p, _)| ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2) + (p[2] - a[2]).powi(2)).sqrt());
	// An actor (enemy, crate: flag 0x1000, the registry retail's lock-on scans) the ray passes through nearer than the contact
	// is the target: the shot goes to its lock-on point, so it is struck.
	let lg = ST.lock().map_or(false, |mut s| log_on(&mut s));
	let (actor, diag) = actor_hit(c, a, cam.fwd, hit_t.unwrap_or(range).min(range), lg);
	snap.restore(c);
	let aim = match (actor, best) {
		(Some((_, centre, moby)), _) => Aim { pos: centre, moby, hit: true },
		(None, Some((_, p, moby))) => Aim { pos: p, moby, hit: true },
		(None, None) => Aim { pos: at(a, cam.fwd, range), moby: 0, hit: false },
	};
	let dist = ((aim.pos[0] - a[0]).powi(2) + (aim.pos[1] - a[1]).powi(2) + (aim.pos[2] - a[2]).powi(2)).sqrt();
	let probe = format!(" dist {dist:.1}{diag}");
	let class = if aim.moby != 0 { c.mem.rd16((aim.moby & PTR) + 0xa6) } else { 0 };
	if let Ok(mut s) = ST.lock() {
		s.field = cam.field;
		s.aim = Some(aim);
		let line = format!(
			"aim field {} eye {:.2} {:.2} {:.2} fwd {:.3} {:.3} {:.3} t0 {t0:.2} hit {} pos {:.2} {:.2} {:.2} moby {:#x} class {class}{probe}",
			cam.field, cam.eye[0], cam.eye[1], cam.eye[2], cam.fwd[0], cam.fwd[1], cam.fwd[2], aim.hit, aim.pos[0], aim.pos[1], aim.pos[2], aim.moby
		);
		log(&mut s, &line);
	}
	Some(aim)
}

/// What the crosshair casts may leave changed besides the registers and the line-test records `modcam::cast` puts back: the
/// scratchpad (the line test's work area), VU0 memory, the stack below the caller and the moby table (a test that meets a moby
/// bumps its word +0x9c). A hook casts in the middle of retail's own work, so all of it is put back as it was.
struct Snap {
	spr: Vec<u8>,
	vu0d: Vec<u8>,
	vu0m: Vec<u8>,
	lo: usize,
	stack: Vec<u8>,
	mb: usize,
	mobys: Vec<u8>,
}
const STACK_SNAP: usize = 0x4000;
impl Snap {
	fn take(c: &mut Cpu) -> Snap {
		let sp = (c.g(29) as u32 & PTR) as usize;
		let lo = sp.saturating_sub(STACK_SNAP);
		let (ts, te) = super::table(c.mem.ram[0x15ed84] as u32);
		let (ms, me) = ((c.mem.rd32(ts) & PTR) as usize, (c.mem.rd32(te) & PTR) as usize);
		let (mb, mobys) = if ms != 0 && me > ms && me - ms <= 0x100 * 4096 { (ms, c.mem.ram[ms..me].to_vec()) } else { (0, Vec::new()) };
		Snap { spr: c.mem.spr.clone(), vu0d: c.mem.vu0d.clone(), vu0m: c.mem.vu0m.clone(), lo, stack: c.mem.ram[lo..sp].to_vec(), mb, mobys }
	}
	fn restore(self, c: &mut Cpu) {
		if c.mem.vu0m != self.vu0m {
			c.mem.vu0m.copy_from_slice(&self.vu0m);
			c.mem.vu0m_changed();
		}
		c.mem.spr.copy_from_slice(&self.spr);
		c.mem.vu0d.copy_from_slice(&self.vu0d);
		c.mem.ram[self.lo..self.lo + self.stack.len()].copy_from_slice(&self.stack);
		c.mem.ram[self.mb..self.mb + self.mobys.len()].copy_from_slice(&self.mobys);
	}
}

/// The nearest actor of the active list (retail's registry scan of blaster_aim_search: flag 0x1000, not marked dead by a negative
/// byte at +0x20) whose lock-on sphere the ray `o + t * d` passes through within `max_t`: (t, lock-on point, moby). The lock-on
/// point is the moby origin plus the target record's height (record = first word of the pointer at +0x78 when flag 0x20 is
/// set; +0x10 the height, byte +10 the radius in eighths), 0.5 and radius 0 without a record.
fn actor_hit(c: &mut Cpu, o: [f32; 3], d: [f32; 3], max_t: f32, lg: bool) -> (Option<(f32, [f32; 3], u32)>, String) {
	let (ts, _) = super::table(c.mem.ram[0x15ed84] as u32);
	let hero = c.mem.rd32(HERO_MOBY) & PTR;
	let rmin = envf("RAC_AIM_R", ACTOR_R);
	let mut m = c.mem.rd32(ts + 0xc) & PTR;
	let mut best: Option<(f32, [f32; 3], u32)> = None;
	let mut diag = String::new();
	let mut n = 0;
	while m != 0 && n < 4096 {
		n += 1;
		let fl = c.mem.rd16(m + 0x34);
		if m != hero && fl & 0x1000 != 0 {
			let p = rv(c, m + 0x10);
			let (mut h, mut r) = (0.5f32, 0.0f32);
			if fl & 0x20 != 0 {
				let pr = c.mem.rd32(m + 0x78) & PTR;
				let rec = c.mem.rd32(pr) & PTR;
				if rec != 0 {
					h = rf(c, rec + 0x10);
					r = c.mem.rd8(rec + 10) as f32 * 0.125;
				}
			}
			let r = r.max(rmin);
			let ctr = [p[0], p[1], p[2] + h];
			let v = [ctr[0] - o[0], ctr[1] - o[1], ctr[2] - o[2]];
			let t = dot(v, d);
			let perp2 = dot(v, v) - t * t;
			let live = c.mem.rd8(m + 0x20) < 0x80;
			if lg && perp2 <= 9.0 && t > 0.0 && t < max_t + 3.0 {
				diag += &format!(" [actor {m:#x} class {} t {t:.1} off {:.2} b20 {:#x} h {h:.2} r {r:.2}]", c.mem.rd16(m + 0xa6), perp2.max(0.0).sqrt(), c.mem.rd8(m + 0x20));
			}
			if live && t >= ACTOR_MIN_T && t <= max_t + r && perp2 <= r * r && best.is_none_or(|b| t < b.0) {
				best = Some((t, ctr, m));
			}
		}
		m = c.mem.rd32(m + 0x28) & PTR;
	}
	(best, diag)
}

/// Yaw and elevation (radians, positive up) from `from` toward `to`; the camera's forward direction when `to` is not ahead of
/// the muzzle (a wall right in front of Ratchet).
fn dir_to(from: [f32; 3], to: [f32; 3]) -> (f32, f32) {
	let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
	let fwd = modcam::aim_cam().map_or([1.0, 0.0, 0.0], |c| c.fwd);
	let d = if dot(d, fwd) < 0.3 { fwd } else { d };
	let xy = (d[0] * d[0] + d[1] * d[1]).sqrt();
	(d[1].atan2(d[0]), d[2].atan2(xy))
}

/// The turn from the hero's logic pose to the drawn one: while aiming and moving, the hero's logic pose (rows +0xc0 = his path
/// yaw, strafe) is drawn turned to the aim yaw about his origin, and the held weapon with it (strafe::turn_parts). `(sin, cos,
/// hero origin)` of aim yaw - rows yaw; None standing (logic yaw = aim yaw) or without aim.
fn drawn_turn(c: &mut Cpu) -> Option<(f32, f32, [f32; 3])> {
	let (want, h) = (modcam::aim_yaw()?, c.mem.rd32(HERO_MOBY) & PTR);
	if h == 0 {
		return None;
	}
	let (r0, hp) = (rv(c, h + 0xc0), rv(c, h + 0x10));
	if r0[0] == 0.0 && r0[1] == 0.0 {
		return None;
	}
	let (sn, cs) = (want - r0[1].atan2(r0[0])).sin_cos();
	(!(sn.abs() < 1e-6 && cs > 0.0)).then_some((sn, cs, hp))
}

/// Where `p` (a point retail posed on the hero this tick, e.g. a muzzle joint) is drawn (`drawn_turn`); `p` unchanged standing.
fn drawn_point(c: &mut Cpu, p: [f32; 3]) -> [f32; 3] {
	let Some((sn, cs, hp)) = drawn_turn(c) else { return p };
	let (x, y) = (p[0] - hp[0], p[1] - hp[1]);
	[hp[0] + cs * x - sn * y, hp[1] + sn * x + cs * y, p[2]]
}

/// Log only: `p` relative to the hero in the drawn frame (forward along the aim yaw, left).
fn off_drawn(c: &mut Cpu, p: [f32; 3]) -> (f32, f32) {
	let (h, ay) = (c.mem.rd32(HERO_MOBY) & PTR, modcam::aim_yaw().unwrap_or(0.0));
	let hp = rv(c, h + 0x10);
	let o = [p[0] - hp[0], p[1] - hp[1]];
	(ay.cos() * o[0] + ay.sin() * o[1], -ay.sin() * o[0] + ay.cos() * o[1])
}

/// `drawn_point` of the vector at `a`, written back (the caller's muzzle buffer: retail then spawns the shot and its sparks there).
fn drawn_at(c: &mut Cpu, a: u32) -> [f32; 3] {
	let p = rv(c, a);
	let q = drawn_point(c, p);
	if q != p {
		for i in 0..3 {
			c.mem.wr32(a + 4 * i as u32, q[i].to_bits());
		}
	}
	q
}

fn trace(name: &str, line: String) {
	if let Ok(mut s) = ST.lock() {
		let l = format!("hook {name} tick {} field {} {line}", s.tick, s.field);
		log(&mut s, &l);
	}
}

/// Log only (RAC_AIM_LOG): the shot or bomb a hook just created, field by field, and the live actors near the hero every 30 fields
/// (class, position, bearing and elevation from the camera, to aim at them in tests).
fn debug_log(m: &mut ee_oracle::Machine) {
	let Ok(mut s) = ST.lock() else { return };
	s.tick = s.tick.wrapping_add(1);
	if !log_on(&mut s) {
		return;
	}
	let lw = m.rd32(LOCK_TARGET);
	if lw != s.lock {
		s.lock = lw;
		let (tick, on) = (s.tick, active());
		log(&mut s, &format!("lock tick {tick} word {lw:#x} active {on}"));
	}
	if s.victim != 0 && s.track_left > 0 {
		let (v, tick) = (s.victim, s.tick);
		let (cls, fl, b20, p) = (m.rd16(v + 0xa6), m.rd16(v + 0x34), m.rd8(v + 0x20), m.rdv3(v + 0x10));
		log(&mut s, &format!("victim tick {tick} moby {v:#x} class {cls} flags {fl:#x} b20 {b20:#x} pos {:.2} {:.2} {:.2}", p[0], p[1], p[2]));
	}
	{
		// Diag (log only): the hero's logic yaw, drawn yaw (moby +0x48), state, stick and position each field.
		let h = m.rd32(HERO_MOBY) & PTR;
		if h != 0 {
			let (tick, p) = (s.tick, m.rdv3(h + 0x10));
			let (y, w, d48) = (f32::from_bits(m.rd32(HERO_YAW)), f32::from_bits(m.rd32(0x13f4d0)), f32::from_bits(m.rd32(h + 0x48)));
			let (st, sx, sy) = (m.rd32(0x1413d4), f32::from_bits(m.rd32(0x141070)), f32::from_bits(m.rd32(0x141074)));
			let c0 = [f32::from_bits(m.rd32(h + 0xc0)), f32::from_bits(m.rd32(h + 0xc4))];
			// The equipped weapon id (0x140408), the held moby (0x1403e0) and its class (log only).
			let (eq, hm) = (m.rd32(0x140408), m.rd32(0x1403e0) & PTR);
			let hcls = if hm != 0 { m.rd16(hm + 0xa6) } else { 0 };
			let cp = modcam::cam_pos_diag(m);
			log(&mut s, &format!("hero tick {tick} st {st:#x} yaw {y:.3} wish {w:.3} m48 {d48:.3} c0 {:.3} {:.3} stick {sx:.2} {sy:.2} pos {:.2} {:.2} {:.2} eq {eq} held {hm:#x} class {hcls} suck {} {} cam {:.3} {:.3} {:.3}", c0[0], c0[1], p[0], p[1], p[2], m.rd32(0x1413c8), m.rd32(0x1413cc), cp[0], cp[1], cp[2]));
			if hcls == 177 {
				// Tesla Claw: the arc the updater left in its variables (pvar = *(held+0x78): start +0, end +0x10, target +0x20,
				// secondary target +0x38, free length +0x24), as a direction from the start to the end.
				let pv = m.rd32(hm + 0x78) & PTR;
				let (a, b) = (m.rdv3(pv), m.rdv3(pv + 0x10));
				let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
				let (ayaw, aelev) = (d[1].atan2(d[0]), d[2].atan2((d[0] * d[0] + d[1] * d[1]).sqrt()));
				let aim = modcam::aim_yaw().unwrap_or(f32::NAN);
				log(&mut s, &format!("tesla arc tick {tick} start {:.2} {:.2} {:.2} end {:.2} {:.2} {:.2} yaw {ayaw:.3} elev {aelev:.3} aim_yaw {aim:.3} hero_yaw {y:.3} target {:#x} t2 {:#x} len {:.2}", a[0], a[1], a[2], b[0], b[1], b[2], m.rd32(pv + 0x20), m.rd32(pv + 0x38), f32::from_bits(m.rd32(pv + 0x24))));
			}
		}
	}
	if s.track != 0 && s.track_left > 0 {
		s.track_left -= 1;
		let (t, tick) = (s.track, s.tick);
		let (cls, p) = (m.rd16(t + 0xa6), m.rdv3(t + 0x10));
		log(&mut s, &format!("track tick {tick} moby {t:#x} class {cls} pos {:.2} {:.2} {:.2}", p[0], p[1], p[2]));
	}
	if s.tick >= 90 {
		// Bomb Glove: the bomb is a class 121 moby (waiting in the glove, then thrown).
		let (ts, _) = super::table(m.level());
		let (mut a, mut n, tick) = (m.rd32(ts + 0xc) & PTR, 0, s.tick);
		while a != 0 && n < 4096 {
			n += 1;
			let cl = m.rd16(a + 0xa6);
			if cl == 121 || cl == 74 || cl == 457 {
				let p = m.rdv3(a + 0x10);
				let nm = match cl { 74 => "mine", 457 => "rocket", _ => "bomb" };
				log(&mut s, &format!("{nm} tick {tick} moby {a:#x} state {} pos {:.2} {:.2} {:.2}", m.rd8(a + 0x20), p[0], p[1], p[2]));
			}
			a = m.rd32(a + 0x28) & PTR;
		}
	}
	if s.tick % 30 != 0 {
		return;
	}
	let (ts, te) = super::table(m.level());
	let (b, e) = (m.rd32(ts) & PTR, m.rd32(te) & PTR);
	let hero = m.rd32(HERO_MOBY) & PTR;
	let hp = m.rdv3(hero + 0x10);
	let cam = modcam::aim_cam();
	let tick = s.tick;
	let mut n = 0;
	for a in (b..e.min(b + 0x100 * 4096)).step_by(0x100) {
		let (fl, cl) = (m.rd16(a + 0x34), m.rd16(a + 0xa6));
		if a == hero || cl == 0 || fl & 0x1000 == 0 || cl == 500 {
			continue;
		}
		let p = m.rdv3(a + 0x10);
		let d = ((p[0] - hp[0]).powi(2) + (p[1] - hp[1]).powi(2) + (p[2] - hp[2]).powi(2)).sqrt();
		if d > 90.0 || n >= 12 {
			continue;
		}
		n += 1;
		let rel = cam.map_or(String::new(), |c| {
			let v = [p[0] - c.eye[0], p[1] - c.eye[1], p[2] + 1.0 - c.eye[2]];
			let (cy, ey) = (v[1].atan2(v[0]), v[2].atan2((v[0] * v[0] + v[1] * v[1]).sqrt()));
			let (fy, fe) = (c.fwd[1].atan2(c.fwd[0]), c.fwd[2].atan2((c.fwd[0] * c.fwd[0] + c.fwd[1] * c.fwd[1]).sqrt()));
			format!(" cam yaw {fy:.3} elev {fe:.3} enemy yaw {cy:.3} elev {ey:.3}")
		});
		log(&mut s, &format!("actor tick {tick} {a:#x} class {cl} flags {fl:#x} pos {:.1} {:.1} {:.1} dist {d:.1}{rel}", p[0], p[1], p[2]));
	}
}

/// Log only: remember the moby a creator hook returned (v0) so `debug_log` follows it.
fn follow(c: &mut Cpu, name: &str) {
	let v0 = ptr(c.g(2));
	if let Ok(mut s) = ST.lock() {
		s.track = v0;
		s.track_left = 80;
		s.victim = s.aim.map_or(0, |a| a.moby);
		let l = format!("hook {name} returned {v0:#x}");
		log(&mut s, &l);
	}
}

// ---- crosshair -----------------------------------------------------------------------------------------------------------

/// Worker, before each field (after modcam's): the reticle request. Same slot words as retail's FUN_001e07c8 (scale +0x10,
/// colour +0x14, texture +0x18, rotation +0x1c, owner moby +0x20, has-position +0x24 = 0: screen centre, +0x26 = 4, +0x28 =
/// 90.0, +0x2c = 4); the consumer skips an entry whose owner moby is gone, so the hero is the owner.
pub fn pre_field(m: &mut ee_oracle::Machine, _fin: &super::FieldIn) {
	debug_log(m);
	if !active() {
		return;
	}
	let (mode, form) = (m.rd32(va(GAME_MODE)), m.cpu.mem.rd8(BODY_FORM));
	let Some((base, cnt)) = modcam::sprite_queue(m.level()) else { return };
	let n = m.rd32(cnt);
	let hero = m.rd32(HERO_MOBY);
	trace("reticle", format!("mode {mode} form {form} queue {base:#x} count {n} hero {hero:#x} owner20 {:#x}", if hero != 0 { m.rd8(hero + 0x20) } else { 0 }));
	if mode != 0 || form != 0 || n >= 3 || hero == 0 || !aims_straight(m.rd32(EQUIPPED)) {
		return;
	}
	let s = base + n * 0x30;
	let mem = &mut m.cpu.mem;
	for o in (0..0x30).step_by(4) {
		mem.wr32(s + o, 0);
	}
	mem.wr32(s + 0x10, 1.0f32.to_bits());
	mem.wr32(s + 0x14, RETICLE_COLOUR);
	mem.wr32(s + 0x18, RETICLE_TEX);
	mem.wr32(s + 0x20, hero);
	// What FUN_001e07c8 stores for texture 0x26 with mode -1: +0x26 = 0, size pi/2, mode 2.
	mem.wr32(s + 0x28, 0x3fc9_0fdb);
	mem.wr32(s + 0x2c, 2);
	mem.wr32(cnt, n + 1);
	// The Blaster search (later in this field) moves this icon onto a locked target: remember the slot.
	if let (Some(cam), Ok(mut st)) = (modcam::aim_cam(), ST.lock()) {
		st.slot = (cam.field, s);
	}
}

// ---- hooks ---------------------------------------------------------------------------------------------------------------

/// Re-entrancy guard of one hook (the retail function it calls runs the hand again, which must fall through to retail).
struct Guard(&'static AtomicBool);
impl Guard {
	fn take(f: &'static AtomicBool) -> Option<Guard> {
		(!f.swap(true, Ordering::SeqCst)).then(|| Guard(f))
	}
}
impl Drop for Guard {
	fn drop(&mut self) {
		self.0.store(false, Ordering::SeqCst);
	}
}
static G_SPAWN: AtomicBool = AtomicBool::new(false);
static G_BOMBV: AtomicBool = AtomicBool::new(false);
static G_BOMBH: AtomicBool = AtomicBool::new(false);
static G_DEVA: AtomicBool = AtomicBool::new(false);
static G_MINEV: AtomicBool = AtomicBool::new(false);
static G_RYNO: AtomicBool = AtomicBool::new(false);
static G_MORPH: AtomicBool = AtomicBool::new(false);
static G_PYRO: AtomicBool = AtomicBool::new(false);
static G_TESLA: AtomicBool = AtomicBool::new(false);
static G_SUCK: AtomicBool = AtomicBool::new(false);
static G_LOCK: AtomicBool = AtomicBool::new(false);

/// Blaster, target search `blaster_aim_search(held, muzzle, &yaw, &pitch)` (Eudora 0x2a6a50, called every tick from the held
/// updater): with the crosshair on it returns 0 to retail (so retail neither snaps the shot to an enemy nor draws its green lock-on
/// icon); with the wide lock-on on, `strafe_lock` looks for the nearest target around the crosshair direction meanwhile, and the
/// shot hook and the icon use it. Otherwise, with the wide lock-on on, `lockon_search` answers with the wider numbers and the
/// nearest pick (RAC_LOCKON_AB=1 instead runs it with retail's constants next to retail's own function and logs differences, the
/// exactness test); else retail runs untouched.
fn blaster_search_any(c: &mut Cpu) -> bool {
	if active() {
		// The Lock-on row (pcext.rs camopts section): Off, or Auto with the keyboard, leaves the green crosshair centred and the
		// shot flies to the aim point.
		if lockon_wide() && super::pcext::lockon_on() {
			strafe_lock(c);
		} else if let Ok(mut s) = ST.lock() {
			s.slock = None;
		}
		c.s(2, 0);
		return true;
	}
	let ab = std::env::var_os("RAC_LOCKON_AB").is_some();
	if !lockon_wide() && !ab {
		return false;
	}
	let Some(_g) = Guard::take(&G_LOCK) else { return false };
	let entry = c.pc;
	let (mu_p, yp, pp) = (ptr(c.g(5)), ptr(c.g(6)), ptr(c.g(7)));
	let (y0, p0) = (rf(c, yp), rf(c, pp));
	let mu = rv(c, mu_p);
	if ab {
		// Test: retail's function and the port with retail's numbers on the real input and on 72 yaw turns of it (retail only writes
		// the two floats, so the extra calls change nothing); one log line per field with the counts.
		let args: Vec<u64> = (4..8).map(|r| c.g(r)).collect();
		let (mut same, mut diff, mut hits) = (0, 0, 0);
		let mut first = String::new();
		for k in 0..72 {
			let mut yin = if k == 0 { y0 } else { y0 + k as f32 * std::f32::consts::TAU / 72.0 };
			if yin > std::f32::consts::PI {
				yin -= std::f32::consts::TAU;
			}
			c.mem.wr32(yp, yin.to_bits());
			c.mem.wr32(pp, p0.to_bits());
			let (m, y, p) = lockon_search(c, mu, yin, p0, LockOn::RETAIL);
			for (i, a) in args.iter().enumerate() {
				c.s(4 + i, *a);
			}
			c.call(entry);
			let (rm, ry, rp) = (c.g(2) as u32 & PTR, rf(c, yp), rf(c, pp));
			if rm != 0 {
				hits += 1;
			}
			if rm == m && (ry - y).abs() < 2e-3 && (rp - p).abs() < 2e-3 {
				same += 1;
			} else {
				diff += 1;
				if first.is_empty() {
					first = format!(" first DIFF in {yin:.4} {p0:.4} retail {rm:#x} {ry:.4} {rp:.4} port {m:#x} {y:.4} {p:.4}");
				}
			}
		}
		c.mem.wr32(yp, y0.to_bits());
		c.mem.wr32(pp, p0.to_bits());
		for (i, a) in args.iter().enumerate() {
			c.s(4 + i, *a);
		}
		c.call(entry);
		trace("lockon_ab", format!("same {same} diff {diff} retail_hits {hits}{first}"));
		return true;
	}
	let (m, y, p) = lockon_search(c, mu, y0, p0, LockOn::wide());
	if m != 0 {
		c.mem.wr32(yp, y.to_bits());
		c.mem.wr32(pp, p.to_bits());
	}
	c.s(2, m as i32 as i64 as u64);
	if m != 0 {
		if ST.lock().map_or(false, |mut s| log_on(&mut s)) {
			// Log only: distance and bearing off the shot of the pick, and the pick of retail's own numbers next to it.
			let tp = rv(c, m + 0x10);
			let dist = ((tp[0] - mu[0]).powi(2) + (tp[1] - mu[1]).powi(2) + (tp[2] - mu[2]).powi(2)).sqrt();
			let off = ((tp[1] - mu[1]).atan2(tp[0] - mu[0]) - y0 + 3.0 * std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
			let (rm, _, _) = lockon_search(c, mu, y0, p0, LockOn::RETAIL);
			trace("lockon", format!("target {m:#x} class {} dist {dist:.1} off {:.1} deg retail_pick {rm:#x} yaw {y:.3} pitch {p:.3}", c.mem.rd16(m + 0xa6), off.to_degrees()));
		}
	}
	true
}

/// The numbers of the Blaster's lock-on search: reach, cone half angle about the shot direction, the yaw difference inside which
/// the shot yaw snaps to the target, and how the pick is made.
#[derive(Clone, Copy)]
struct LockOn {
	range: f32,
	cone: f32,
	snap: f32,
	/// PC pick (`RAC_LOCKON_NEAREST=0` turns it off): of the candidates in range, inside the cone of the INPUT ray and not blocked,
	/// the nearest one to the muzzle. A blocked candidate is skipped (retail's search ends at the first one), and a pick does not
	/// turn the ray the remaining candidates are measured against (retail's follows the pick's bearing and elevation, so a
	/// nearer actor found after a farther one is tested against the farther one's ray).
	nearest: bool,
}
impl LockOn {
	/// Retail's inline constants (20.0, 0.15707964 = 9 degrees, 0.17453292 = 10 degrees) and retail's loop.
	const RETAIL: LockOn = LockOn { range: 20.0, cone: 0.157_079_64, snap: 0.174_532_92, nearest: false };
	/// Wide: twice the reach and the cone, snap 20 degrees (RAC_LOCKON_RANGE / _CONE / _SNAP in radians, tests), nearest pick.
	fn wide() -> LockOn {
		LockOn {
			range: envf("RAC_LOCKON_RANGE", 40.0),
			cone: envf("RAC_LOCKON_CONE", 0.3491),
			snap: envf("RAC_LOCKON_SNAP", 0.3491),
			nearest: std::env::var("RAC_LOCKON_NEAREST").map_or(true, |v| v != "0"),
		}
	}
}

/// A segment test that works for long segments (retail's finds nothing on one longer than about 40 units): true = blocked.
fn blocked(c: &mut Cpu, a: [f32; 3], b: [f32; 3]) -> bool {
	let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
	let len = dot(d, d).sqrt();
	let n = (len / 30.0).ceil().max(1.0) as u32;
	(0..n).any(|i| {
		let (t0, t1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
		modcam::cast(c, at(a, d, t0), at(a, d, t1), 6).is_some()
	})
}

/// The record of a lock-on actor (target record pointer, 0 = none; the point retail aims at is the moby position plus the record's
/// height, 0.5 without one): `(record, height)`.
fn lock_height(c: &mut Cpu, m: u32) -> (u32, f32) {
	let rec = if c.mem.rd16(m + 0x34) & 0x20 != 0 {
		let pr = c.mem.rd32(m + 0x78) & PTR;
		c.mem.rd32(pr) & PTR
	} else {
		0
	};
	(rec, if rec != 0 { rf(c, rec + 0x10) } else { 0.5 })
}

/// The port of `blaster_aim_search` (Novalis 0x2c9c40, decomp overlay.elf.c:125579): the nearest registered actor (flag 0x1000, state
/// byte >= 0) inside `k.range` whose centre lies within `k.cone` of the shot ray from the muzzle `mu` (yaw, floor-tilted pitch; the
/// cone shrinks by the angular radius of a target with a component), clear of terrain from the camera and from the muzzle. Returns
/// (actor, yaw, pitch): pitch is the negative elevation of the pick, yaw snaps to the bearing when within `k.snap` of the input; the
/// input values when nothing is found. Retail's loop (`!k.nearest`): a blocked candidate ends the search with the best so far, and
/// the ray follows each pick. `k.nearest`: see `LockOn`.
fn lockon_search(c: &mut Cpu, mu: [f32; 3], yaw: f32, pitch: f32, k: LockOn) -> (u32, f32, f32) {
	let level = c.mem.ram[0x15ed84] as u32;
	let (ts, _) = super::table(level);
	let Some((cam_at, _, _)) = super::camera_globals(level) else { return (0, yaw, pitch) };
	let (cam, floor) = (rv(c, cam_at), rf(c, 0x13f634));
	let (yaw0, pitch0) = (yaw, pitch);
	let (mut yaw, mut pitch, mut best, mut best_d) = (yaw, pitch, 0u32, k.range);
	// (gap, bearing, elevation) of the nearest pick (`k.nearest`); the log lists every candidate inside the cone.
	let mut pick = (0.0f32, 0.0f32, 0.0f32);
	let lg = k.nearest && ST.lock().map_or(false, |mut s| log_on(&mut s));
	let mut cands = String::new();
	let mut m = c.mem.rd32(ts + 0xc) & PTR;
	let mut n = 0;
	while m != 0 && n < 4096 {
		n += 1;
		let next = c.mem.rd32(m + 0x28) & PTR;
		let (fl, st) = (c.mem.rd16(m + 0x34), c.mem.rd8(m + 0x20) as i8);
		if st < 0 || fl & 0x1000 == 0 {
			m = next;
			continue;
		}
		let (rec, lift) = lock_height(c, m);
		let p = rv(c, m + 0x10);
		let t = [p[0], p[1], p[2] + lift];
		let (dx, dy, dz) = (t[0] - mu[0], t[1] - mu[1], t[2] - mu[2]);
		let bearing = dy.atan2(dx);
		let elev = dz.atan2((dx * dx + dy * dy).sqrt());
		let (ry, rp) = if k.nearest { (yaw0, pitch0) } else { (yaw, pitch) };
		let mut gap = (ry - bearing).abs() % std::f32::consts::TAU;
		if gap > std::f32::consts::PI {
			gap = std::f32::consts::TAU - gap;
		}
		let d = (dx * dx + dy * dy + dz * dz).sqrt();
		if d <= best_d {
			let pa = rp + floor * 0.5;
			let ray = [mu[0] + d * ry.cos() * pa.cos(), mu[1] + d * ry.sin() * pa.cos(), mu[2] + d * pa.sin()];
			let chord = ((t[0] - ray[0]).powi(2) + (t[1] - ray[1]).powi(2) + (t[2] - ray[2]).powi(2)).sqrt();
			let mut ang = if d > 1e-4 { 2.0 * (chord / (2.0 * d)).clamp(-1.0, 1.0).asin() } else { 0.0 };
			if ang > k.cone && rec != 0 {
				let r = c.mem.rd8(rec + 10) as f32 * 0.125;
				if r < d {
					ang = (ang - (r / d).clamp(-1.0, 1.0).asin()).max(0.0);
				}
			}
			if ang < k.cone {
				let hidden = blocked(c, cam, t) || blocked(c, mu, t);
				if lg {
					cands.push_str(&format!(" [{m:#x} class {} dist {d:.1} ang {:.1}{}]", c.mem.rd16(m + 0xa6), ang.to_degrees(), if hidden { " blocked" } else { "" }));
				}
				if hidden {
					if k.nearest {
						m = next;
						continue;
					}
					return (best, yaw, pitch);
				}
				best_d = d;
				best = m;
				if k.nearest {
					pick = (gap, bearing, elev);
				} else {
					pitch = -elev;
					if gap < k.snap {
						yaw = bearing;
					}
				}
			}
		}
		m = next;
	}
	if k.nearest && best != 0 {
		pitch = -pick.2;
		if pick.0 < k.snap {
			yaw = pick.1;
		}
	}
	if lg && !cands.is_empty() {
		trace("lockon_cands", format!("range {:.0} cone {:.1} deg candidates{cands} -> pick {best:#x} dist {best_d:.1}", k.range, k.cone.to_degrees()));
	}
	(best, yaw, pitch)
}

/// Strafe lock-on (PC extra, with the wide lock-on): the Blaster search of this field while the crosshair is on. The ray of the
/// search is the camera's forward direction (the crosshair line) from the drawn muzzle; the pick is `lockon_search`'s nearest
/// target (range 40, cone 20 degrees, not behind terrain). The target is kept for the shot (`blaster_spawn_any` aims at it and
/// hands it to the shot creator, as retail's own lock-on does) and the icon moves onto it, red (`lock_icon`). Nothing found: the
/// crosshair stays as it was, green at the screen centre, and the shot flies to the point under it.
fn strafe_lock(c: &mut Cpu) {
	let Some(cam) = modcam::aim_cam() else { return };
	let mp = rv(c, ptr(c.g(5)));
	let mu = drawn_point(c, mp);
	let f = cam.fwd;
	let (yaw, elev) = (f[1].atan2(f[0]), f[2].atan2((f[0] * f[0] + f[1] * f[1]).sqrt()));
	// The search adds half the hero's floor tilt to the pitch it is given: take it off, so the ray is the camera's.
	let floor = rf(c, 0x13f634);
	let (m, _, _) = lockon_search(c, mu, yaw, elev - floor * 0.5, LockOn::wide());
	if let Ok(mut s) = ST.lock() {
		s.slock = (m != 0).then_some((m, cam.field));
	}
	if m != 0 {
		let t = lock_point(c, m);
		let dist = ((t[0] - mu[0]).powi(2) + (t[1] - mu[1]).powi(2) + (t[2] - mu[2]).powi(2)).sqrt();
		let shown = lock_icon(c, m, &cam);
		trace("strafe_lock", format!("target {m:#x} class {} dist {dist:.1} icon {shown}", c.mem.rd16(m + 0xa6)));
	}
}

/// The point retail aims at on a lock-on actor: its position plus the height of its target record (0.5 without one).
pub fn lock_point(c: &mut Cpu, m: u32) -> [f32; 3] {
	let (_, lift) = lock_height(c, m);
	let p = rv(c, m + 0x10);
	[p[0], p[1], p[2] + lift]
}

/// The strafe lock this field (`strafe_lock` ran in it) if its actor is still a live target: (actor, lock-on point).
pub fn strafe_locked(c: &mut Cpu) -> Option<(u32, [f32; 3])> {
	let cam = modcam::aim_cam()?;
	let (m, f) = ST.lock().ok()?.slock?;
	if f != cam.field || c.mem.rd8(m + 0x20) as i8 == -1 || c.mem.rd16(m + 0x34) & 0x1000 == 0 {
		return None;
	}
	Some((m, lock_point(c, m)))
}

/// The strafe lock for the Modern camera's framing (modcam.rs `dyn_frame`): the actor and its lock-on point, when `strafe_lock` found
/// it in field `field` or the field before (the follow camera runs before or after the gun update within a field) and it is still a
/// live target. Does not touch modcam's state (its caller holds modcam's lock).
pub fn strafe_lock_for_cam(c: &mut Cpu, field: u32) -> Option<(u32, [f32; 3])> {
	let (m, f) = ST.lock().ok()?.slock?;
	if f > field || field - f > 1 || c.mem.rd8(m + 0x20) as i8 == -1 || c.mem.rd16(m + 0x34) & 0x1000 == 0 {
		return None;
	}
	Some((m, lock_point(c, m)))
}

/// The lock-on icon on the target: the slot `pre_field` queued this field (the crosshair at the screen centre) becomes retail's
/// target reticle of the same sprite, red: position = the lock-on point, scale abs(0.9 - 0.65 d / 50) with d the distance from the
/// camera (retail's formula, OV:125928), has-position 1 so retail's consumer projects it. False = the slot is not ours any more.
fn lock_icon(c: &mut Cpu, m: u32, cam: &modcam::AimCam) -> bool {
	let (f, slot) = ST.lock().map_or((0, 0), |s| s.slot);
	if slot == 0 || f != cam.field || c.mem.rd32(slot + 0x14) != RETICLE_COLOUR || c.mem.rd32(slot + 0x18) != RETICLE_TEX || c.mem.rd32(slot + 0x24) != 0 {
		return false;
	}
	let t = lock_point(c, m);
	let d = ((t[0] - cam.eye[0]).powi(2) + (t[1] - cam.eye[1]).powi(2) + (t[2] - cam.eye[2]).powi(2)).sqrt();
	for (i, v) in t.iter().enumerate() {
		c.mem.wr32(slot + 4 * i as u32, v.to_bits());
	}
	c.mem.wr32(slot + 12, 1.0f32.to_bits());
	c.mem.wr32(slot + 0x10, (0.9 - d / 50.0 * 0.65).abs().to_bits());
	c.mem.wr32(slot + 0x14, RETICLE_LOCKED);
	c.mem.wr32(slot + 0x24, 1);
	true
}

/// Blaster shot creator `blaster_ray_spawn(yaw, pitch, yaw, pitch, hero, &muzzle, target)` (Eudora 0x2c04b0; floats f12..f15 = yaw, -elevation, yaw,
/// -elevation; ints a0 = hero, a1 = &muzzle, a2 = target): yaw and pitch from the muzzle toward the aim point, no target; with a strafe
/// lock (`strafe_lock` found one this field) toward its lock-on point, and the target is handed over (the shot then ends in the target,
/// as in retail's own locked shot).
fn blaster_spawn_any(c: &mut Cpu) -> bool {
	if !active() {
		return false;
	}
	let Some(_g) = Guard::take(&G_SPAWN) else { return false };
	let entry = c.pc;
	let mp = ptr(c.g(5));
	let Some(aim) = aim_point(c) else { return false };
	let mu = drawn_at(c, mp);
	let lock = strafe_locked(c);
	let (yaw, elev) = dir_to(mu, lock.map_or(aim.pos, |l| l.1));
	let h = c.mem.rd32(HERO_MOBY) & PTR;
	let hp = rv(c, h + 0x10);
	let (hy, d48) = (rf(c, HERO_YAW), modcam::aim_yaw().unwrap_or(0.0));
	let o = [mu[0] - hp[0], mu[1] - hp[1]];
	let (fx, fy) = (hy.cos() * o[0] + hy.sin() * o[1], -hy.sin() * o[0] + hy.cos() * o[1]);
	let (gx, gy) = (d48.cos() * o[0] + d48.sin() * o[1], -d48.sin() * o[0] + d48.cos() * o[1]);
	trace("blaster_spawn", format!("muzzle {:.2} {:.2} {:.2} yaw {yaw:.3} elev {elev:.3} aim {:.2} {:.2} {:.2} hero {:.2} {:.2} {:.2} hyaw {hy:.3} m48 {d48:.3} off_logic {fx:.2} {fy:.2} off_drawn {gx:.2} {gy:.2} dz {:.2} lock {:#x}", mu[0], mu[1], mu[2], aim.pos[0], aim.pos[1], aim.pos[2], hp[0], hp[1], hp[2], mu[2] - hp[2], lock.map_or(0, |l| l.0)));
	c.f[12] = yaw.to_bits();
	c.f[13] = (-elev).to_bits();
	c.f[14] = yaw.to_bits();
	c.f[15] = (-elev).to_bits();
	c.s(6, lock.map_or(0, |l| l.0 as i32 as i64 as u64));
	c.call(entry);
	follow(c, "blaster_spawn");
	true
}

/// Bomb Glove launch velocity `bomb_velocity(g, _, range, &from, &target, &out)` (Eudora 0x2b4e70, called every tick by the held
/// updater for the landing marker and the throw): the target point is the aim point (retail clamps it to its range and solves the
/// lob to land on it), and the hero yaw word it falls back to is the bearing to it for the call.
fn bomb_velocity_any(c: &mut Cpu) -> bool {
	if !active() {
		return false;
	}
	let Some(_g) = Guard::take(&G_BOMBV) else { return false };
	let entry = c.pc;
	let from_p = ptr(c.g(4));
	let Some(aim) = aim_point(c) else { return false };
	// The throw leaves from `from` (the caller's point beside the hero, on his logic heading; the launch FUN_0029f2b0 reads the
	// same buffer): turned to the drawn Ratchet, so the lob is solved from, and the bomb leaves, the drawn glove side.
	let from = drawn_at(c, from_p);
	let bearing = (aim.pos[1] - from[1]).atan2(aim.pos[0] - from[0]);
	let sp = c.g(29) as u32;
	let buf = sp.wrapping_sub(0x200) & !0xf;
	let saved: Vec<u32> = (0..4).map(|i| c.mem.rd32(buf + 4 * i)).collect();
	let w = c.mem.rd32(ptr(c.g(5)) + 12);
	for i in 0..3 {
		c.mem.wr32(buf + 4 * i as u32, aim.pos[i].to_bits());
	}
	c.mem.wr32(buf + 12, w);
	let yaw = c.mem.rd32(HERO_YAW);
	c.mem.wr32(HERO_YAW, bearing.to_bits());
	let od = off_drawn(c, from);
	trace("bomb_velocity", format!("from {:.2} {:.2} {:.2} aim {:.2} {:.2} {:.2} bearing {bearing:.3} off_drawn {:.2} {:.2}", from[0], from[1], from[2], aim.pos[0], aim.pos[1], aim.pos[2], od.0, od.1));
	c.s(5, buf as u64);
	c.call(entry);
	c.mem.wr32(HERO_YAW, yaw);
	for (i, v) in saved.iter().enumerate() {
		c.mem.wr32(buf + 4 * i as u32, *v);
	}
	true
}

/// Bomb Glove held updater (class 192, Eudora 0x2b50b0): retail runs, then the lock-on target word it set from its cone scan is
/// cleared, so the throw state does not turn the hero toward an enemy (the crosshair aims).
fn bomb_held_any(c: &mut Cpu) -> bool {
	if !active() {
		return false;
	}
	let Some(_g) = Guard::take(&G_BOMBH) else { return false };
	let entry = c.pc;
	c.call(entry);
	c.mem.wr32(LOCK_TARGET, 0);
	true
}

/// Devastator rocket creator `(yaw, pitch, _, yaw, pitch, held, &muzzle, target)` (Eudora 0x2a1f48): the rocket leaves toward the
/// aim point and homes on the enemy under the crosshair (a moby with the actor flag 0x1000), or on nothing.
fn devastator_spawn_any(c: &mut Cpu) -> bool {
	if !active() {
		return false;
	}
	let Some(_g) = Guard::take(&G_DEVA) else { return false };
	let entry = c.pc;
	let mp = ptr(c.g(5));
	let Some(aim) = aim_point(c) else { return false };
	let mu = drawn_at(c, mp);
	let (yaw, elev) = dir_to(mu, aim.pos);
	let target = if aim.moby != 0 && c.mem.rd16(ptr(aim.moby as u64) + 0x34) & 0x1000 != 0 { aim.moby } else { 0 };
	let hm = c.mem.rd32(HERO_MOBY) & PTR;
	let (hp, ay) = (rv(c, hm + 0x10), modcam::aim_yaw().unwrap_or(0.0));
	let o = [mu[0] - hp[0], mu[1] - hp[1]];
	trace("devastator_spawn", format!("muzzle {:.2} {:.2} {:.2} yaw {yaw:.3} elev {elev:.3} target {target:#x} aim {:.2} {:.2} {:.2} off_drawn {:.2} {:.2}", mu[0], mu[1], mu[2], aim.pos[0], aim.pos[1], aim.pos[2], ay.cos() * o[0] + ay.sin() * o[1], -ay.sin() * o[0] + ay.cos() * o[1]));
	c.f[12] = yaw.to_bits();
	c.f[13] = (-elev).to_bits();
	c.f[15] = yaw.to_bits();
	c.f[16] = (-elev).to_bits();
	c.s(6, target as i32 as i64 as u64);
	c.call(entry);
	follow(c, "devastator_spawn");
	true
}

/// RYNO rocket creator `(target height, yaw, -elevation, hero, &muzzle, target)` (Eudora 0x2c8f68, one call per rocket of the salvo): the
/// rocket leaves toward the aim point and homes on the enemy under the crosshair (a moby with the actor flag 0x1000) or on nothing.
/// The height is that of the lock-on point above the actor's origin (the rocket homes on origin + height).
fn ryno_spawn_any(c: &mut Cpu) -> bool {
	if !active() {
		return false;
	}
	let Some(_g) = Guard::take(&G_RYNO) else { return false };
	let entry = c.pc;
	let mp = ptr(c.g(5));
	let Some(aim) = aim_point(c) else { return false };
	let mu = drawn_at(c, mp);
	let (yaw, elev) = dir_to(mu, aim.pos);
	let mut target = 0u32;
	if aim.moby != 0 {
		let m = ptr(aim.moby as u64);
		if c.mem.rd16(m + 0x34) & 0x1000 != 0 {
			target = aim.moby;
			c.f[12] = (aim.pos[2] - rf(c, m + 0x18)).to_bits();
		}
	}
	trace("ryno_spawn", format!("muzzle {:.2} {:.2} {:.2} yaw {yaw:.3} elev {elev:.3} target {target:#x}", mu[0], mu[1], mu[2]));
	c.f[13] = yaw.to_bits();
	c.f[14] = (-elev).to_bits();
	c.s(6, target as i32 as i64 as u64);
	c.call(entry);
	follow(c, "ryno_spawn");
	true
}

/// RYNO lock-on search `(yaw range, pitch range, range, &from, &dir, history)` (Eudora 0x2c83e8, every tick of the held updater): returns
/// 0 (no target) so retail neither marks an enemy with its lock-on reticle nor homes on one it chose; the crosshair picks (creator hook).
fn ryno_search_any(c: &mut Cpu) -> bool {
	if !active() {
		return false;
	}
	c.s(2, 0);
	true
}

/// Morph-o-Ray beam target search `(held, &from, &yaw, &elevation)` (Eudora 0x2aed98, every tick of the held updater): retail computes yaw
/// and elevation from the hero's heading; here both are the muzzle -> aim point direction, written through the two pointers before
/// retail runs (the held updater stores them again every tick and draws the beam from them).
fn morph_search_any(c: &mut Cpu) -> bool {
	if !active() {
		return false;
	}
	let Some(_g) = Guard::take(&G_MORPH) else { return false };
	let entry = c.pc;
	let from_p = ptr(c.g(5));
	let (yp, ep) = (ptr(c.g(6)), ptr(c.g(7)));
	let Some(aim) = aim_point(c) else { return false };
	let mu = drawn_at(c, from_p);
	let (yaw, elev) = dir_to(mu, aim.pos);
	c.mem.wr32(yp, yaw.to_bits());
	c.mem.wr32(ep, elev.to_bits());
	trace("morph_search", format!("from {:.2} {:.2} {:.2} yaw {yaw:.3} elev {elev:.3}", mu[0], mu[1], mu[2]));
	c.call(entry);
	follow(c, "morph_search");
	true
}

/// Pyrocitor held updater (class 176, Eudora 0x2a9b98): the flame leaves the nozzle (joint 0 of the held moby, posed on its
/// +0xc0 rows and +0x10, i.e. on the hero's logic pose) along the hero's heading vector (x, y, 0 at 0x13f9c0). For the call the
/// held pose and the hero frame copy 0x13f990 (the spark offsets) are turned to the drawn Ratchet (`drawn_turn`), so the flame
/// and its pilot moby leave the drawn nozzle, and the heading is the unit vector from that nozzle to the aim point; all restored.
fn pyro_held_any(c: &mut Cpu) -> bool {
	if !active() {
		return false;
	}
	let Some(_g) = Guard::take(&G_PYRO) else { return false };
	let entry = c.pc;
	let held = ptr(c.g(4));
	let Some(aim) = aim_point(c) else { return false };
	// (address, value before, value written) of every turned word.
	let mut put: Vec<(u32, u32, u32)> = Vec::new();
	if let Some((sn, cs, hp)) = drawn_turn(c) {
		let p = rv(c, held + 0x10);
		let (x, y) = (p[0] - hp[0], p[1] - hp[1]);
		let mut w = vec![(held + 0x10, hp[0] + cs * x - sn * y), (held + 0x14, hp[1] + sn * x + cs * y)];
		for row in [held + 0xc0, held + 0xd0, held + 0xe0, 0x13f990, 0x13f9a0, 0x13f9b0] {
			let r = rv(c, row);
			w.push((row, cs * r[0] - sn * r[1]));
			w.push((row + 4, sn * r[0] + cs * r[1]));
		}
		for (a, v) in w {
			put.push((a, c.mem.rd32(a), v.to_bits()));
			c.mem.wr32(a, v.to_bits());
		}
	}
	let mu = joint0(c, entry, held).unwrap_or_else(|| rv(c, held + 0x10));
	let (yaw, elev) = dir_to(mu, aim.pos);
	let v = [elev.cos() * yaw.cos(), elev.cos() * yaw.sin(), elev.sin()];
	let saved: Vec<u32> = (0..3).map(|i| c.mem.rd32(HERO_HEADING + 4 * i)).collect();
	for (i, x) in v.iter().enumerate() {
		c.mem.wr32(HERO_HEADING + 4 * i as u32, x.to_bits());
	}
	if let Ok(mut s) = ST.lock() {
		s.victim = aim.moby;
		s.track_left = 3;
	}
	c.call(entry);
	for (i, x) in saved.iter().enumerate() {
		c.mem.wr32(HERO_HEADING + 4 * i as u32, *x);
	}
	// Put back what retail did not rewrite in the call.
	for &(a, old, new) in put.iter().rev() {
		if c.mem.rd32(a) == new {
			c.mem.wr32(a, old);
		}
	}
	// Log only: the pilot moby (*held+0x78) retail placed at the nozzle it used.
	let pv = c.mem.rd32(held + 0x78) & PTR;
	let pilot = c.mem.rd32(pv) & PTR;
	let pp = if pilot != 0 { rv(c, pilot + 0x10) } else { [0.0; 3] };
	let (od, op) = (off_drawn(c, mu), off_drawn(c, pp));
	trace("pyro_held", format!("nozzle {:.2} {:.2} {:.2} yaw {yaw:.3} elev {elev:.3} state {} turned {} off_drawn {:.2} {:.2} pilot {pilot:#x} off_drawn {:.2} {:.2} dz {:.2}", mu[0], mu[1], mu[2], c.mem.rd8(held + 0x20), !put.is_empty(), od.0, od.1, op.0, op.1, pp[2] - mu[2]));
	true
}

/// Tesla Claw (class 177, Eudora 0x2aab88) and Suck Cannon (class 849, Eudora 0x2dc448) held updaters: retail aims both along the hero's
/// logic frame (yaw 0x13f3e8 for the Tesla target cone and the 2.09 rad drop test, 0x13f9d8 for the Suck close-target cone, rows
/// 0x13f990.. for the free arc end, the Suck launch and suction), which while strafing is the movement path. For the call the held
/// pose and the frame rows are turned to the drawn Ratchet (`drawn_turn`), row 0x13f990 (forward) points from the held weapon to the
/// aim point, and the yaw words are the aim yaw; every word written is put back unless retail rewrote it.
fn weapon_held_any(c: &mut Cpu, g: &'static AtomicBool, name: &str, yaw_words: &[u32]) -> bool {
	if !active() {
		return false;
	}
	let Some(_g) = Guard::take(g) else { return false };
	let entry = c.pc;
	let held = ptr(c.g(4));
	let (Some(aim), Some(ay)) = (aim_point(c), modcam::aim_yaw()) else { return false };
	// (address, value before, value written) of every changed word.
	let mut put: Vec<(u32, u32, u32)> = Vec::new();
	let mut w: Vec<(u32, u32)> = Vec::new();
	let mut mu = rv(c, held + 0x10);
	if let Some((sn, cs, hp)) = drawn_turn(c) {
		let (x, y) = (mu[0] - hp[0], mu[1] - hp[1]);
		mu = [hp[0] + cs * x - sn * y, hp[1] + sn * x + cs * y, mu[2]];
		w.push((held + 0x10, mu[0].to_bits()));
		w.push((held + 0x14, mu[1].to_bits()));
		for row in [held + 0xc0, held + 0xd0, held + 0xe0, 0x13f990, 0x13f9a0, 0x13f9b0] {
			let r = rv(c, row);
			w.push((row, (cs * r[0] - sn * r[1]).to_bits()));
			w.push((row + 4, (sn * r[0] + cs * r[1]).to_bits()));
		}
	}
	let (yaw, elev) = dir_to(mu, aim.pos);
	for (i, x) in [elev.cos() * yaw.cos(), elev.cos() * yaw.sin(), elev.sin()].iter().enumerate() {
		w.retain(|&(a, _)| a != 0x13f990 + 4 * i as u32);
		w.push((0x13f990 + 4 * i as u32, x.to_bits()));
	}
	for &a in yaw_words {
		w.push((a, ay.to_bits()));
	}
	for (a, v) in w {
		put.push((a, c.mem.rd32(a), v));
		c.mem.wr32(a, v);
	}
	if let Ok(mut s) = ST.lock() {
		s.victim = aim.moby;
		s.track_left = 3;
	}
	c.call(entry);
	for &(a, old, new) in put.iter().rev() {
		if c.mem.rd32(a) == new {
			c.mem.wr32(a, old);
		}
	}
	trace(name, format!("held {:.2} {:.2} {:.2} yaw {yaw:.3} elev {elev:.3} aim_yaw {ay:.3} state {} turned {}", mu[0], mu[1], mu[2], c.mem.rd8(held + 0x20), put.len() > 3));
	true
}

fn tesla_held_any(c: &mut Cpu) -> bool {
	weapon_held_any(c, &G_TESLA, "tesla_held", &[HERO_YAW])
}

fn suck_held_any(c: &mut Cpu) -> bool {
	weapon_held_any(c, &G_SUCK, "suck_held", &[HERO_YAW, 0x13f9d8])
}

/// Joint 0 of the held moby in world space, by retail's own call (the `jal` at +0x4c of the Pyrocitor held updater: moby joint
/// world position FUN_00242090 on Eudora). Registers, stack, scratchpad, VU0 and the moby table are put back.
fn joint0(c: &mut Cpu, entry: u32, held: u32) -> Option<[f32; 3]> {
	let w = c.mem.rd32(entry + 0x4c);
	if w >> 26 != 3 {
		return None;
	}
	let f = ((entry + 0x50) & 0xf000_0000) | ((w & 0x03ff_ffff) << 2);
	let regs = modcam::Regs::save(c);
	let snap = Snap::take(c);
	let buf = (c.g(29) as u32).wrapping_sub(0x200) & !0xf;
	c.s(29, buf.wrapping_sub(0x40) as i32 as i64 as u64);
	c.s(4, held as i32 as i64 as u64);
	c.s(5, 0);
	c.s(6, buf as i32 as i64 as u64);
	c.call(f);
	let p = rv(c, buf);
	snap.restore(c);
	regs.restore(c);
	Some(p)
}

/// Mine Glove launch velocity `mine_velocity(g, minimum, range, &from, &target, &out)` (Eudora 0x2b4310, called every tick by the held
/// updater with the point 60 steps ahead of the hero and writing the held mine's launch velocity): the target point is the aim point
/// (retail clamps it to its range and solves the lob to land on it).
fn mine_velocity_any(c: &mut Cpu) -> bool {
	if !active() {
		return false;
	}
	let Some(_g) = Guard::take(&G_MINEV) else { return false };
	let entry = c.pc;
	let from_p = ptr(c.g(4));
	let Some(aim) = aim_point(c) else { return false };
	// As the Bomb Glove: `from` (also the launch point mine_launch FUN_0029bb38 reads) turned to the drawn Ratchet.
	let from = drawn_at(c, from_p);
	let out_p = ptr(c.g(6));
	let sp = c.g(29) as u32;
	let buf = sp.wrapping_sub(0x200) & !0xf;
	let saved: Vec<u32> = (0..4).map(|i| c.mem.rd32(buf + 4 * i)).collect();
	let w = c.mem.rd32(ptr(c.g(5)) + 12);
	for i in 0..3 {
		c.mem.wr32(buf + 4 * i as u32, aim.pos[i].to_bits());
	}
	c.mem.wr32(buf + 12, w);
	c.s(5, buf as u64);
	c.call(entry);
	for (i, v) in saved.iter().enumerate() {
		c.mem.wr32(buf + 4 * i as u32, *v);
	}
	let v = rv(c, out_p);
	let bearing = (aim.pos[1] - from[1]).atan2(aim.pos[0] - from[0]);
	let od = off_drawn(c, from);
	trace("mine_velocity", format!("from {:.2} {:.2} {:.2} aim {:.2} {:.2} {:.2} bearing {bearing:.3} out {:.3} {:.3} {:.3} out_bearing {:.3} off_drawn {:.2} {:.2}", from[0], from[1], from[2], aim.pos[0], aim.pos[1], aim.pos[2], v[0], v[1], v[2], v[1].atan2(v[0]), od.0, od.1));
	true
}

// Rows: scratch_aim/mkaim.py (Eudora entry -> its 18 twins, size from decomp/names, FNV-1a of the overlay.elf words).
static HANDS: [Hand; 228] = [
// blaster_search (eudora 0x2a6a50): 19 levels
	Hand { entry: 0x2b4050, end: 0x2b4350, hash: 0x4a3ed2aedf0335e9, est: 0, f: blaster_search_any }, // 0 veldin
	Hand { entry: 0x2c9c40, end: 0x2c9f40, hash: 0x6a0427d0080da2f3, est: 0, f: blaster_search_any }, // 1 novalis
	Hand { entry: 0x2b39e8, end: 0x2b3ce8, hash: 0x338c9031e40a4b7d, est: 0, f: blaster_search_any }, // 2 aridia
	Hand { entry: 0x2a5a40, end: 0x2a5d40, hash: 0x954e5dd02d91cee7, est: 0, f: blaster_search_any }, // 3 kerwan
	Hand { entry: 0x2a6a50, end: 0x2a6d50, hash: 0xe82ecf2cdb074309, est: 0, f: blaster_search_any }, // 4 eudora
	Hand { entry: 0x2dfcc0, end: 0x2dffc0, hash: 0xac3c7c6b28bf592b, est: 0, f: blaster_search_any }, // 5 rilgar
	Hand { entry: 0x2c11d8, end: 0x2c14d8, hash: 0xa692933debc9d351, est: 0, f: blaster_search_any }, // 6 nebula
	Hand { entry: 0x2dd0e0, end: 0x2dd3e0, hash: 0x26ed09b262c9196f, est: 0, f: blaster_search_any }, // 7 umbris
	Hand { entry: 0x2bcfe0, end: 0x2bd2e0, hash: 0xc69997c89fe75e41, est: 0, f: blaster_search_any }, // 8 batalia
	Hand { entry: 0x2d3490, end: 0x2d3790, hash: 0xc949c4da6d7b22a7, est: 0, f: blaster_search_any }, // 9 gaspar
	Hand { entry: 0x2a5890, end: 0x2a5b90, hash: 0x5b9c2f9f9451f1d5, est: 0, f: blaster_search_any }, // 10 orxon
	Hand { entry: 0x2d9dc0, end: 0x2da0c0, hash: 0xf76a368be19994c9, est: 0, f: blaster_search_any }, // 11 pokitaru
	Hand { entry: 0x2cbd70, end: 0x2cc070, hash: 0xa3434bdc8ead6135, est: 0, f: blaster_search_any }, // 12 hoven
	Hand { entry: 0x2ccf88, end: 0x2cd288, hash: 0x4310d3a2ad31057f, est: 0, f: blaster_search_any }, // 13 oltanis_orbit
	Hand { entry: 0x2c2c78, end: 0x2c2f78, hash: 0x0f4127c10c248e15, est: 0, f: blaster_search_any }, // 14 oltanis
	Hand { entry: 0x2ad2c8, end: 0x2ad5c8, hash: 0x52fedb51678f25f5, est: 0, f: blaster_search_any }, // 15 quartu
	Hand { entry: 0x2ae8c8, end: 0x2aebc8, hash: 0xefbdc6d534c8cfc5, est: 0, f: blaster_search_any }, // 16 kalebo
	Hand { entry: 0x2b1d10, end: 0x2b2010, hash: 0x6d946abe20ba6465, est: 0, f: blaster_search_any }, // 17 veldin_orbit
	Hand { entry: 0x2b6840, end: 0x2b6b40, hash: 0xd191c78809707f23, est: 0, f: blaster_search_any }, // 18 veldin_finale
// blaster_spawn (eudora 0x2c04b0): 19 levels
	Hand { entry: 0x2ca918, end: 0x2cadc0, hash: 0x90842f2845b679e4, est: 0, f: blaster_spawn_any }, // 0 veldin
	Hand { entry: 0x2e15f8, end: 0x2e1aa0, hash: 0x6a94ff32e4da4ff2, est: 0, f: blaster_spawn_any }, // 1 novalis
	Hand { entry: 0x2ca2b0, end: 0x2ca758, hash: 0xe53cc19b0782afa0, est: 0, f: blaster_spawn_any }, // 2 aridia
	Hand { entry: 0x2bc560, end: 0x2bca08, hash: 0x730ec3d11d842fac, est: 0, f: blaster_spawn_any }, // 3 kerwan
	Hand { entry: 0x2c04b0, end: 0x2c0958, hash: 0x1034e74fbcbf7240, est: 0, f: blaster_spawn_any }, // 4 eudora
	Hand { entry: 0x2f6a18, end: 0x2f6ec0, hash: 0x4ffad76ca102fb8c, est: 0, f: blaster_spawn_any }, // 5 rilgar
	Hand { entry: 0x2d8640, end: 0x2d8ae8, hash: 0x5f9760eaf01aeb98, est: 0, f: blaster_spawn_any }, // 6 nebula
	Hand { entry: 0x2f3e38, end: 0x2f42e0, hash: 0xe16d12cfed78bd5c, est: 0, f: blaster_spawn_any }, // 7 umbris
	Hand { entry: 0x2d89f8, end: 0x2d8ea0, hash: 0x1b4b61c30b02f83e, est: 0, f: blaster_spawn_any }, // 8 batalia
	Hand { entry: 0x2edd58, end: 0x2ee200, hash: 0xd0b8289b31a40e6e, est: 0, f: blaster_spawn_any }, // 9 gaspar
	Hand { entry: 0x2bd188, end: 0x2bd630, hash: 0x465a1d0526b65112, est: 0, f: blaster_spawn_any }, // 10 orxon
	Hand { entry: 0x2f0e50, end: 0x2f12f8, hash: 0xba359b793bede0e6, est: 0, f: blaster_spawn_any }, // 11 pokitaru
	Hand { entry: 0x2e88a0, end: 0x2e8d48, hash: 0x1cac0e25b819153a, est: 0, f: blaster_spawn_any }, // 12 hoven
	Hand { entry: 0x2e6d68, end: 0x2e7210, hash: 0xd3dff6e794c38060, est: 0, f: blaster_spawn_any }, // 13 oltanis_orbit
	Hand { entry: 0x2dcbb0, end: 0x2dd058, hash: 0x189d54386a40f0cc, est: 0, f: blaster_spawn_any }, // 14 oltanis
	Hand { entry: 0x2c8b78, end: 0x2c9020, hash: 0xf68c3cf929d128c2, est: 0, f: blaster_spawn_any }, // 15 quartu
	Hand { entry: 0x2c5ca0, end: 0x2c6148, hash: 0xf3980b29798bbd68, est: 0, f: blaster_spawn_any }, // 16 kalebo
	Hand { entry: 0x2c9c40, end: 0x2ca0e8, hash: 0x587d04a6b3a04822, est: 0, f: blaster_spawn_any }, // 17 veldin_orbit
	Hand { entry: 0x2cd108, end: 0x2cd5b0, hash: 0x3c44e207b22b3c50, est: 0, f: blaster_spawn_any }, // 18 veldin_finale
// bomb_velocity (eudora 0x2b4e70): 19 levels
	Hand { entry: 0x2c1e30, end: 0x2c2070, hash: 0xc52b619fa3cd1ac0, est: 0, f: bomb_velocity_any }, // 0 veldin
	Hand { entry: 0x2d7a20, end: 0x2d7c60, hash: 0xed4169768a4517be, est: 0, f: bomb_velocity_any }, // 1 novalis
	Hand { entry: 0x2c17c8, end: 0x2c1a08, hash: 0xa8dfaa4760da63d6, est: 0, f: bomb_velocity_any }, // 2 aridia
	Hand { entry: 0x2b3820, end: 0x2b3a60, hash: 0x9a3aeecbebfede3a, est: 0, f: bomb_velocity_any }, // 3 kerwan
	Hand { entry: 0x2b4e70, end: 0x2b50b0, hash: 0x9848f689bab44326, est: 0, f: bomb_velocity_any }, // 4 eudora
	Hand { entry: 0x2edaa0, end: 0x2edce0, hash: 0x4327cad82ef5183a, est: 0, f: bomb_velocity_any }, // 5 rilgar
	Hand { entry: 0x2cefb8, end: 0x2cf1f8, hash: 0x42954a2536318c04, est: 0, f: bomb_velocity_any }, // 6 nebula
	Hand { entry: 0x2eaec0, end: 0x2eb100, hash: 0x1067351791d3106a, est: 0, f: bomb_velocity_any }, // 7 umbris
	Hand { entry: 0x2cadc0, end: 0x2cb000, hash: 0x0a7283654dc173c6, est: 0, f: bomb_velocity_any }, // 8 batalia
	Hand { entry: 0x2e1270, end: 0x2e14b0, hash: 0x3ecd6f386592537e, est: 0, f: bomb_velocity_any }, // 9 gaspar
	Hand { entry: 0x2b3670, end: 0x2b38b0, hash: 0xb47f5d5af505b58e, est: 0, f: bomb_velocity_any }, // 10 orxon
	Hand { entry: 0x2e7ba0, end: 0x2e7de0, hash: 0xe21a4e1272bc3458, est: 0, f: bomb_velocity_any }, // 11 pokitaru
	Hand { entry: 0x2da198, end: 0x2da3d8, hash: 0x46785ca8200fc70a, est: 0, f: bomb_velocity_any }, // 12 hoven
	Hand { entry: 0x2dbbd0, end: 0x2dbe10, hash: 0x87b2030d9bb67770, est: 0, f: bomb_velocity_any }, // 13 oltanis_orbit
	Hand { entry: 0x2d0a58, end: 0x2d0c98, hash: 0x236e94c326bdb5de, est: 0, f: bomb_velocity_any }, // 14 oltanis
	Hand { entry: 0x2bb0a8, end: 0x2bb2e8, hash: 0xa053108ecb010ada, est: 0, f: bomb_velocity_any }, // 15 quartu
	Hand { entry: 0x2bc6a8, end: 0x2bc8e8, hash: 0x7e78eb7e293c3c7a, est: 0, f: bomb_velocity_any }, // 16 kalebo
	Hand { entry: 0x2bfaf0, end: 0x2bfd30, hash: 0xa7a185113d686f66, est: 0, f: bomb_velocity_any }, // 17 veldin_orbit
	Hand { entry: 0x2c4620, end: 0x2c4860, hash: 0x3c6e6c3418d47800, est: 0, f: bomb_velocity_any }, // 18 veldin_finale
// bomb_held (eudora 0x2b50b0): 19 levels
	Hand { entry: 0x2c2070, end: 0x2c2b68, hash: 0xd556b31da6988758, est: 0, f: bomb_held_any }, // 0 veldin
	Hand { entry: 0x2d7c60, end: 0x2d8758, hash: 0x12edf3f2bfc0add0, est: 0, f: bomb_held_any }, // 1 novalis
	Hand { entry: 0x2c1a08, end: 0x2c2500, hash: 0x06823c7ecbefdb29, est: 0, f: bomb_held_any }, // 2 aridia
	Hand { entry: 0x2b3a60, end: 0x2b4558, hash: 0x65ee0c3eddda0891, est: 0, f: bomb_held_any }, // 3 kerwan
	Hand { entry: 0x2b50b0, end: 0x2b5ba8, hash: 0x8fd6654fece5a395, est: 0, f: bomb_held_any }, // 4 eudora
	Hand { entry: 0x2edce0, end: 0x2ee7d8, hash: 0xe175a41cc57870bd, est: 0, f: bomb_held_any }, // 5 rilgar
	Hand { entry: 0x2cf1f8, end: 0x2cfcf0, hash: 0xa65753a391b46e79, est: 0, f: bomb_held_any }, // 6 nebula
	Hand { entry: 0x2eb100, end: 0x2ebbf8, hash: 0xdf40a9584aa14732, est: 0, f: bomb_held_any }, // 7 umbris
	Hand { entry: 0x2cb000, end: 0x2cbaf8, hash: 0x9e6bea64eb2cafd1, est: 0, f: bomb_held_any }, // 8 batalia
	Hand { entry: 0x2e14b0, end: 0x2e1fa8, hash: 0x8f634b34d75ad8ca, est: 0, f: bomb_held_any }, // 9 gaspar
	Hand { entry: 0x2b38b0, end: 0x2b43a8, hash: 0x852b8ae8bd352b59, est: 0, f: bomb_held_any }, // 10 orxon
	Hand { entry: 0x2e7de0, end: 0x2e88d8, hash: 0x4921523ef1c43ab3, est: 0, f: bomb_held_any }, // 11 pokitaru
	Hand { entry: 0x2da3d8, end: 0x2daed0, hash: 0x25ef14239e861fb0, est: 0, f: bomb_held_any }, // 12 hoven
	Hand { entry: 0x2dbe10, end: 0x2dc908, hash: 0x8a8c563a7ef86114, est: 0, f: bomb_held_any }, // 13 oltanis_orbit
	Hand { entry: 0x2d0c98, end: 0x2d1790, hash: 0xa3da2eb4b2078c07, est: 0, f: bomb_held_any }, // 14 oltanis
	Hand { entry: 0x2bb2e8, end: 0x2bbde0, hash: 0x0f51baf333477fcd, est: 0, f: bomb_held_any }, // 15 quartu
	Hand { entry: 0x2bc8e8, end: 0x2bd3e0, hash: 0x7a601191a1d62d4d, est: 0, f: bomb_held_any }, // 16 kalebo
	Hand { entry: 0x2bfd30, end: 0x2c0828, hash: 0xc1e711db6d798a6b, est: 0, f: bomb_held_any }, // 17 veldin_orbit
	Hand { entry: 0x2c4860, end: 0x2c5358, hash: 0xd7f22db51b21fc70, est: 0, f: bomb_held_any }, // 18 veldin_finale
// devastator_spawn (eudora 0x2a1f48): 19 levels
	Hand { entry: 0x2af618, end: 0x2af8f8, hash: 0x837e1159b1ad30fc, est: 0, f: devastator_spawn_any }, // 0 veldin
	Hand { entry: 0x2c4d70, end: 0x2c5050, hash: 0xa6da906784a9a5ba, est: 0, f: devastator_spawn_any }, // 1 novalis
	Hand { entry: 0x2aefb0, end: 0x2af290, hash: 0xfa6f06ffe724e8de, est: 0, f: devastator_spawn_any }, // 2 aridia
	Hand { entry: 0x2a1008, end: 0x2a12e8, hash: 0x741f59cb0d68ed58, est: 0, f: devastator_spawn_any }, // 3 kerwan
	Hand { entry: 0x2a1f48, end: 0x2a2228, hash: 0xd2317b77bcfc7ff6, est: 0, f: devastator_spawn_any }, // 4 eudora
	Hand { entry: 0x2db288, end: 0x2db568, hash: 0xa34d58cdbf8d5c30, est: 0, f: devastator_spawn_any }, // 5 rilgar
	Hand { entry: 0x2bc308, end: 0x2bc5e8, hash: 0x258509d8c205d47e, est: 0, f: devastator_spawn_any }, // 6 nebula
	Hand { entry: 0x2d86a8, end: 0x2d8988, hash: 0xec6e45796102374c, est: 0, f: devastator_spawn_any }, // 7 umbris
	Hand { entry: 0x2b8110, end: 0x2b83f0, hash: 0x211e3d8ac7aa329c, est: 0, f: devastator_spawn_any }, // 8 batalia
	Hand { entry: 0x2ce5c0, end: 0x2ce8a0, hash: 0x2e46530fd225126a, est: 0, f: devastator_spawn_any }, // 9 gaspar
	Hand { entry: 0x2a0e58, end: 0x2a1138, hash: 0x0220814c4c2b4fec, est: 0, f: devastator_spawn_any }, // 10 orxon
	Hand { entry: 0x2d5388, end: 0x2d5668, hash: 0xd1684b8033c6b0e4, est: 0, f: devastator_spawn_any }, // 11 pokitaru
	Hand { entry: 0x2c7330, end: 0x2c7610, hash: 0x42db94eac5b4182c, est: 0, f: devastator_spawn_any }, // 12 hoven
	Hand { entry: 0x2c8550, end: 0x2c8830, hash: 0x845ab98942f90684, est: 0, f: devastator_spawn_any }, // 13 oltanis_orbit
	Hand { entry: 0x2be240, end: 0x2be520, hash: 0x81637f427d0ad644, est: 0, f: devastator_spawn_any }, // 14 oltanis
	Hand { entry: 0x2a7d30, end: 0x2a8010, hash: 0x7cc8a871dd96097e, est: 0, f: devastator_spawn_any }, // 15 quartu
	Hand { entry: 0x2a9e90, end: 0x2aa170, hash: 0x745a0b16d716103c, est: 0, f: devastator_spawn_any }, // 16 kalebo
	Hand { entry: 0x2ace40, end: 0x2ad120, hash: 0x01b2a34356c890fc, est: 0, f: devastator_spawn_any }, // 17 veldin_orbit
	Hand { entry: 0x2b1e08, end: 0x2b20e8, hash: 0xa190bcbdf9327314, est: 0, f: devastator_spawn_any }, // 18 veldin_finale
// ryno_spawn (eudora 0x2c8f68): 19 levels
	Hand { entry: 0x2cc888, end: 0x2ccb98, hash: 0x38d6fa41bf2d7ea7, est: 0, f: ryno_spawn_any }, // 0 veldin
	Hand { entry: 0x2e5068, end: 0x2e5378, hash: 0xbd75a914e29d59d9, est: 0, f: ryno_spawn_any }, // 1 novalis
	Hand { entry: 0x2cdd50, end: 0x2ce060, hash: 0xe164228350be505f, est: 0, f: ryno_spawn_any }, // 2 aridia
	Hand { entry: 0x2bf438, end: 0x2bf748, hash: 0xd9f3a01c36d6fabb, est: 0, f: ryno_spawn_any }, // 3 kerwan
	Hand { entry: 0x2c8f68, end: 0x2c9278, hash: 0x6fbc6cac1155c2a1, est: 0, f: ryno_spawn_any }, // 4 eudora
	Hand { entry: 0x2f9050, end: 0x2f9360, hash: 0x9c9979f767cceec3, est: 0, f: ryno_spawn_any }, // 5 rilgar
	Hand { entry: 0x2da780, end: 0x2daa90, hash: 0xb43892f09ab68027, est: 0, f: ryno_spawn_any }, // 6 nebula
	Hand { entry: 0x2f6b80, end: 0x2f6e90, hash: 0x81c7556204d10b07, est: 0, f: ryno_spawn_any }, // 7 umbris
	Hand { entry: 0x2e6a00, end: 0x2e6d10, hash: 0xd5defc0aba2e967d, est: 0, f: ryno_spawn_any }, // 8 batalia
	Hand { entry: 0x2f0968, end: 0x2f0c78, hash: 0x0cd6bedd223a47b1, est: 0, f: ryno_spawn_any }, // 9 gaspar
	Hand { entry: 0x2bfa20, end: 0x2bfd30, hash: 0x5bb70438ea7c607d, est: 0, f: ryno_spawn_any }, // 10 orxon
	Hand { entry: 0x2f3c98, end: 0x2f3fa8, hash: 0xbbc93ac7a3877ec5, est: 0, f: ryno_spawn_any }, // 11 pokitaru
	Hand { entry: 0x2ede38, end: 0x2ee148, hash: 0x0e6675b5532d8749, est: 0, f: ryno_spawn_any }, // 12 hoven
	Hand { entry: 0x2ee5a8, end: 0x2ee8b8, hash: 0xda1f9ee322c1f7c1, est: 0, f: ryno_spawn_any }, // 13 oltanis_orbit
	Hand { entry: 0x2e2070, end: 0x2e2380, hash: 0x005e721633e4331d, est: 0, f: ryno_spawn_any }, // 14 oltanis
	Hand { entry: 0x2cc708, end: 0x2cca18, hash: 0x0346576109ab206b, est: 0, f: ryno_spawn_any }, // 15 quartu
	Hand { entry: 0x2c7fb0, end: 0x2c82c0, hash: 0xf051bf422614efbb, est: 0, f: ryno_spawn_any }, // 16 kalebo
	Hand { entry: 0x2cd0b0, end: 0x2cd3c0, hash: 0xf8fbaa8941d17139, est: 0, f: ryno_spawn_any }, // 17 veldin_orbit
	Hand { entry: 0x2cf350, end: 0x2cf660, hash: 0x6bde0246f70b89c5, est: 0, f: ryno_spawn_any }, // 18 veldin_finale
// ryno_search (eudora 0x2c83e8): 19 levels
	Hand { entry: 0x2cbd08, end: 0x2cbfb0, hash: 0x4a12bb0debbe1250, est: 0, f: ryno_search_any }, // 0 veldin
	Hand { entry: 0x2e44e8, end: 0x2e4790, hash: 0x77871220ec7c226e, est: 0, f: ryno_search_any }, // 1 novalis
	Hand { entry: 0x2cd1d0, end: 0x2cd478, hash: 0xb19fdb99ece2d0c4, est: 0, f: ryno_search_any }, // 2 aridia
	Hand { entry: 0x2bd950, end: 0x2bdbf8, hash: 0xe445f9e2a794f710, est: 0, f: ryno_search_any }, // 3 kerwan
	Hand { entry: 0x2c83e8, end: 0x2c8690, hash: 0x87ed1df922d3b9a4, est: 0, f: ryno_search_any }, // 4 eudora
	Hand { entry: 0x2f84d0, end: 0x2f8778, hash: 0x5cfeb7eab51783b8, est: 0, f: ryno_search_any }, // 5 rilgar
	Hand { entry: 0x2d9c00, end: 0x2d9ea8, hash: 0xed28badc487026c2, est: 0, f: ryno_search_any }, // 6 nebula
	Hand { entry: 0x2f6000, end: 0x2f62a8, hash: 0xf68701332a22c968, est: 0, f: ryno_search_any }, // 7 umbris
	Hand { entry: 0x2e4f18, end: 0x2e51c0, hash: 0x6c9201c1985eaa12, est: 0, f: ryno_search_any }, // 8 batalia
	Hand { entry: 0x2efde8, end: 0x2f0090, hash: 0x7f673ec51f422b08, est: 0, f: ryno_search_any }, // 9 gaspar
	Hand { entry: 0x2beea0, end: 0x2bf148, hash: 0x464d99a4cf04dbbc, est: 0, f: ryno_search_any }, // 10 orxon
	Hand { entry: 0x2f3118, end: 0x2f33c0, hash: 0xbe66bdd3d3269402, est: 0, f: ryno_search_any }, // 11 pokitaru
	Hand { entry: 0x2ed2b8, end: 0x2ed560, hash: 0x7f55eaf997ef77ae, est: 0, f: ryno_search_any }, // 12 hoven
	Hand { entry: 0x2eda28, end: 0x2edcd0, hash: 0x3efb7a7498a14dca, est: 0, f: ryno_search_any }, // 13 oltanis_orbit
	Hand { entry: 0x2e0588, end: 0x2e0830, hash: 0xf9b12258bb94eb82, est: 0, f: ryno_search_any }, // 14 oltanis
	Hand { entry: 0x2cbb88, end: 0x2cbe30, hash: 0xd3f371c0eaf1c330, est: 0, f: ryno_search_any }, // 15 quartu
	Hand { entry: 0x2c7430, end: 0x2c76d8, hash: 0xb7e19c076bc963d8, est: 0, f: ryno_search_any }, // 16 kalebo
	Hand { entry: 0x2cc530, end: 0x2cc7d8, hash: 0x7531bd351f3a2cf4, est: 0, f: ryno_search_any }, // 17 veldin_orbit
	Hand { entry: 0x2ce7d0, end: 0x2cea78, hash: 0xf66a10c90855fc8a, est: 0, f: ryno_search_any }, // 18 veldin_finale
// morph_search (eudora 0x2aed98): 19 levels
	Hand { entry: 0x2bbd58, end: 0x2bc190, hash: 0xbdc44e14290044a8, est: 0, f: morph_search_any }, // 0 veldin
	Hand { entry: 0x2d1948, end: 0x2d1d80, hash: 0x1d5a02fa86afdf22, est: 0, f: morph_search_any }, // 1 novalis
	Hand { entry: 0x2bb6f0, end: 0x2bbb28, hash: 0x15b13da9a03d834c, est: 0, f: morph_search_any }, // 2 aridia
	Hand { entry: 0x2ad748, end: 0x2adb80, hash: 0xc285f6e541e2099e, est: 0, f: morph_search_any }, // 3 kerwan
	Hand { entry: 0x2aed98, end: 0x2af1d0, hash: 0x78d13f063ce0d0c8, est: 0, f: morph_search_any }, // 4 eudora
	Hand { entry: 0x2e79c8, end: 0x2e7e00, hash: 0x44d8757e1c9afb06, est: 0, f: morph_search_any }, // 5 rilgar
	Hand { entry: 0x2c8ee0, end: 0x2c9318, hash: 0x1e5f6f117491c60c, est: 0, f: morph_search_any }, // 6 nebula
	Hand { entry: 0x2e4de8, end: 0x2e5220, hash: 0x210dd4bb7b4e070a, est: 0, f: morph_search_any }, // 7 umbris
	Hand { entry: 0x2c4ce8, end: 0x2c5120, hash: 0xfcee19f612b647f4, est: 0, f: morph_search_any }, // 8 batalia
	Hand { entry: 0x2db198, end: 0x2db5d0, hash: 0x2b00ef005a21eac4, est: 0, f: morph_search_any }, // 9 gaspar
	Hand { entry: 0x2ad598, end: 0x2ad9d0, hash: 0xbc593c92cd4f9ff4, est: 0, f: morph_search_any }, // 10 orxon
	Hand { entry: 0x2e1ac8, end: 0x2e1f00, hash: 0x55b60b1746e75940, est: 0, f: morph_search_any }, // 11 pokitaru
	Hand { entry: 0x2d40c0, end: 0x2d44f8, hash: 0xaaf632fdb869a424, est: 0, f: morph_search_any }, // 12 hoven
	Hand { entry: 0x2d5af8, end: 0x2d5f30, hash: 0x45a43a7249d56532, est: 0, f: morph_search_any }, // 13 oltanis_orbit
	Hand { entry: 0x2ca980, end: 0x2cadb8, hash: 0xee946f2909239904, est: 0, f: morph_search_any }, // 14 oltanis
	Hand { entry: 0x2b4fd0, end: 0x2b5408, hash: 0x2ea4c5d7da011b90, est: 0, f: morph_search_any }, // 15 quartu
	Hand { entry: 0x2b65d0, end: 0x2b6a08, hash: 0xd9852ba93bf259fc, est: 0, f: morph_search_any }, // 16 kalebo
	Hand { entry: 0x2b9a18, end: 0x2b9e50, hash: 0x3d95bddb39c15df4, est: 0, f: morph_search_any }, // 17 veldin_orbit
	Hand { entry: 0x2be548, end: 0x2be980, hash: 0x2801c84a0b262cee, est: 0, f: morph_search_any }, // 18 veldin_finale
// pyro_held (eudora 0x2a9b98): 19 levels
	Hand { entry: 0x2b7198, end: 0x2b7bd8, hash: 0x8d1c7d20f048b284, est: 0, f: pyro_held_any }, // 0 veldin
	Hand { entry: 0x2ccd88, end: 0x2cd7c8, hash: 0x72ddc46986c9554a, est: 0, f: pyro_held_any }, // 1 novalis
	Hand { entry: 0x2b6b30, end: 0x2b7570, hash: 0xd3543822e1f7a54e, est: 0, f: pyro_held_any }, // 2 aridia
	Hand { entry: 0x2a8b88, end: 0x2a95c8, hash: 0xd58d1d2b1192c0b2, est: 0, f: pyro_held_any }, // 3 kerwan
	Hand { entry: 0x2a9b98, end: 0x2aa5d8, hash: 0xbcf43003ef8323f4, est: 0, f: pyro_held_any }, // 4 eudora
	Hand { entry: 0x2e2e08, end: 0x2e3848, hash: 0xe816d54075f94e3a, est: 0, f: pyro_held_any }, // 5 rilgar
	Hand { entry: 0x2c4320, end: 0x2c4d60, hash: 0x16277ed652318982, est: 0, f: pyro_held_any }, // 6 nebula
	Hand { entry: 0x2e0228, end: 0x2e0c68, hash: 0x4b35478d7a41f35e, est: 0, f: pyro_held_any }, // 7 umbris
	Hand { entry: 0x2c0128, end: 0x2c0b68, hash: 0xae73b34f22ada1ce, est: 0, f: pyro_held_any }, // 8 batalia
	Hand { entry: 0x2d65d8, end: 0x2d7018, hash: 0xca3b06b3528b305a, est: 0, f: pyro_held_any }, // 9 gaspar
	Hand { entry: 0x2a89d8, end: 0x2a9418, hash: 0xaddc9c56d25e7c72, est: 0, f: pyro_held_any }, // 10 orxon
	Hand { entry: 0x2dcf08, end: 0x2dd948, hash: 0x0b7873b8972c34e8, est: 0, f: pyro_held_any }, // 11 pokitaru
	Hand { entry: 0x2ceeb8, end: 0x2cf8f8, hash: 0x5128319fa744287e, est: 0, f: pyro_held_any }, // 12 hoven
	Hand { entry: 0x2d0f38, end: 0x2d1978, hash: 0x3e82a86ad34f572e, est: 0, f: pyro_held_any }, // 13 oltanis_orbit
	Hand { entry: 0x2c5dc0, end: 0x2c6800, hash: 0x20665c8b61b81e64, est: 0, f: pyro_held_any }, // 14 oltanis
	Hand { entry: 0x2b0410, end: 0x2b0e50, hash: 0x02369b673235156c, est: 0, f: pyro_held_any }, // 15 quartu
	Hand { entry: 0x2b1a10, end: 0x2b2450, hash: 0xbed3a0fb519049d0, est: 0, f: pyro_held_any }, // 16 kalebo
	Hand { entry: 0x2b4e58, end: 0x2b5898, hash: 0x354395cfce3c3784, est: 0, f: pyro_held_any }, // 17 veldin_orbit
	Hand { entry: 0x2b9988, end: 0x2ba3c8, hash: 0x81a924f7919babfa, est: 0, f: pyro_held_any }, // 18 veldin_finale
// mine_velocity (eudora 0x2b4310): 19 levels
	Hand { entry: 0x2c12d0, end: 0x2c1478, hash: 0x7f4a4656240d57e8, est: 0, f: mine_velocity_any }, // 0 veldin
	Hand { entry: 0x2d6ec0, end: 0x2d7068, hash: 0x1e7af627e34ca4b4, est: 0, f: mine_velocity_any }, // 1 novalis
	Hand { entry: 0x2c0c68, end: 0x2c0e10, hash: 0x8955ab00c454e046, est: 0, f: mine_velocity_any }, // 2 aridia
	Hand { entry: 0x2b2cc0, end: 0x2b2e68, hash: 0xdadf1eb0bd40e06a, est: 0, f: mine_velocity_any }, // 3 kerwan
	Hand { entry: 0x2b4310, end: 0x2b44b8, hash: 0x0f6d2e981c0823c6, est: 0, f: mine_velocity_any }, // 4 eudora
	Hand { entry: 0x2ecf40, end: 0x2ed0e8, hash: 0x2106247b57429986, est: 0, f: mine_velocity_any }, // 5 rilgar
	Hand { entry: 0x2ce458, end: 0x2ce600, hash: 0x60cae3b8d38acf0a, est: 0, f: mine_velocity_any }, // 6 nebula
	Hand { entry: 0x2ea360, end: 0x2ea508, hash: 0xba288db3c5bee8c6, est: 0, f: mine_velocity_any }, // 7 umbris
	Hand { entry: 0x2ca260, end: 0x2ca408, hash: 0xd2d8f16a5fa617a8, est: 0, f: mine_velocity_any }, // 8 batalia
	Hand { entry: 0x2e0710, end: 0x2e08b8, hash: 0xc5739020557b3c20, est: 0, f: mine_velocity_any }, // 9 gaspar
	Hand { entry: 0x2b2b10, end: 0x2b2cb8, hash: 0xc087e002b0e58ec6, est: 0, f: mine_velocity_any }, // 10 orxon
	Hand { entry: 0x2e7040, end: 0x2e71e8, hash: 0x0e522747be402286, est: 0, f: mine_velocity_any }, // 11 pokitaru
	Hand { entry: 0x2d9638, end: 0x2d97e0, hash: 0xa17c164f2446f0b4, est: 0, f: mine_velocity_any }, // 12 hoven
	Hand { entry: 0x2db070, end: 0x2db218, hash: 0xa892513b0e1613a6, est: 0, f: mine_velocity_any }, // 13 oltanis_orbit
	Hand { entry: 0x2cfef8, end: 0x2d00a0, hash: 0x30e78e5cc9ed7814, est: 0, f: mine_velocity_any }, // 14 oltanis
	Hand { entry: 0x2ba548, end: 0x2ba6f0, hash: 0xf67742d5a041d432, est: 0, f: mine_velocity_any }, // 15 quartu
	Hand { entry: 0x2bbb48, end: 0x2bbcf0, hash: 0x7623f0879ca01646, est: 0, f: mine_velocity_any }, // 16 kalebo
	Hand { entry: 0x2bef90, end: 0x2bf138, hash: 0x94ea5e4b47643536, est: 0, f: mine_velocity_any }, // 17 veldin_orbit
	Hand { entry: 0x2c3ac0, end: 0x2c3c68, hash: 0x96e82e784515f066, est: 0, f: mine_velocity_any }, // 18 veldin_finale
// tesla_held (eudora 0x2aab88): 19 levels
	Hand { entry: 0x2b8188, end: 0x2b8cb8, hash: 0xaaf0bc5e1d1b3c8b, est: 0, f: tesla_held_any }, // 0 veldin
	Hand { entry: 0x2cdd78, end: 0x2ce8a8, hash: 0x394b763367b9c5ac, est: 0, f: tesla_held_any }, // 1 novalis
	Hand { entry: 0x2b7b20, end: 0x2b8650, hash: 0x2b46e9ce6f218f46, est: 0, f: tesla_held_any }, // 2 aridia
	Hand { entry: 0x2a9b78, end: 0x2aa6a8, hash: 0x8c7886a254062979, est: 0, f: tesla_held_any }, // 3 kerwan
	Hand { entry: 0x2aab88, end: 0x2ab6b8, hash: 0x81326046019d73eb, est: 0, f: tesla_held_any }, // 4 eudora
	Hand { entry: 0x2e3df8, end: 0x2e4928, hash: 0x1e794e63bbea3ec4, est: 0, f: tesla_held_any }, // 5 rilgar
	Hand { entry: 0x2c5310, end: 0x2c5e40, hash: 0x75aa14e27c72d36a, est: 0, f: tesla_held_any }, // 6 nebula
	Hand { entry: 0x2e1218, end: 0x2e1d48, hash: 0x87d779d110e4e889, est: 0, f: tesla_held_any }, // 7 umbris
	Hand { entry: 0x2c1118, end: 0x2c1c48, hash: 0x91cf0e1e13429656, est: 0, f: tesla_held_any }, // 8 batalia
	Hand { entry: 0x2d75c8, end: 0x2d80f8, hash: 0x897f04c9b32c2191, est: 0, f: tesla_held_any }, // 9 gaspar
	Hand { entry: 0x2a99c8, end: 0x2aa4f8, hash: 0xa462768586066023, est: 0, f: tesla_held_any }, // 10 orxon
	Hand { entry: 0x2ddef8, end: 0x2dea28, hash: 0x00dcdadd9b39dd06, est: 0, f: tesla_held_any }, // 11 pokitaru
	Hand { entry: 0x2cfea8, end: 0x2d09d8, hash: 0x4902cf34ce634dae, est: 0, f: tesla_held_any }, // 12 hoven
	Hand { entry: 0x2d1f28, end: 0x2d2a58, hash: 0xa59205761b7eaee6, est: 0, f: tesla_held_any }, // 13 oltanis_orbit
	Hand { entry: 0x2c6db0, end: 0x2c78e0, hash: 0xc98655d73a8af823, est: 0, f: tesla_held_any }, // 14 oltanis
	Hand { entry: 0x2b1400, end: 0x2b1f30, hash: 0xd1527ffa305c7bab, est: 0, f: tesla_held_any }, // 15 quartu
	Hand { entry: 0x2b2a00, end: 0x2b3530, hash: 0xec3e1563b5e7c549, est: 0, f: tesla_held_any }, // 16 kalebo
	Hand { entry: 0x2b5e48, end: 0x2b6978, hash: 0x656a57d533c1f5db, est: 0, f: tesla_held_any }, // 17 veldin_orbit
	Hand { entry: 0x2ba978, end: 0x2bb4a8, hash: 0x8126cea5d919ed53, est: 0, f: tesla_held_any }, // 18 veldin_finale
// suck_held (eudora 0x2dc448): 19 levels
	Hand { entry: 0x2da138, end: 0x2db1f0, hash: 0x5e09b5da8a1f7ea0, est: 0, f: suck_held_any }, // 0 veldin
	Hand { entry: 0x302930, end: 0x3039e8, hash: 0x7081ae6ee747f508, est: 0, f: suck_held_any }, // 1 novalis
	Hand { entry: 0x2e4d30, end: 0x2e5de8, hash: 0xb73d8e8634d8ec8e, est: 0, f: suck_held_any }, // 2 aridia
	Hand { entry: 0x2d53f8, end: 0x2d64b0, hash: 0x109565bcf46e5fbc, est: 0, f: suck_held_any }, // 3 kerwan
	Hand { entry: 0x2dc448, end: 0x2dd500, hash: 0x8fa06fcf4fca6c9f, est: 0, f: suck_held_any }, // 4 eudora
	Hand { entry: 0x30fbf8, end: 0x310cb0, hash: 0xd474f4c3047dd0b0, est: 0, f: suck_held_any }, // 5 rilgar
	Hand { entry: 0x2ead20, end: 0x2ebdd8, hash: 0x371bb8aa343c939e, est: 0, f: suck_held_any }, // 6 nebula
	Hand { entry: 0x305c20, end: 0x306cd8, hash: 0xd971fbf90d0d046f, est: 0, f: suck_held_any }, // 7 umbris
	Hand { entry: 0x2fbe40, end: 0x2fcef8, hash: 0xc7b945c932c07234, est: 0, f: suck_held_any }, // 8 batalia
	Hand { entry: 0x2fb568, end: 0x2fc620, hash: 0x33bd469670c5ea76, est: 0, f: suck_held_any }, // 9 gaspar
	Hand { entry: 0x2ced58, end: 0x2cfe10, hash: 0x79f4deec0e11635b, est: 0, f: suck_held_any }, // 10 orxon
	Hand { entry: 0x303358, end: 0x304410, hash: 0xf7c82d20bde3eb56, est: 0, f: suck_held_any }, // 11 pokitaru
	Hand { entry: 0x2fa520, end: 0x2fb5d8, hash: 0x8532cdc20fc69d6a, est: 0, f: suck_held_any }, // 12 hoven
	Hand { entry: 0x2fd0f8, end: 0x2fe1b0, hash: 0x306997d55a5fd69b, est: 0, f: suck_held_any }, // 13 oltanis_orbit
	Hand { entry: 0x2f66a0, end: 0x2f7758, hash: 0xe2db3062c7269476, est: 0, f: suck_held_any }, // 14 oltanis
	Hand { entry: 0x2df068, end: 0x2e0120, hash: 0x2496ea4d716d42f8, est: 0, f: suck_held_any }, // 15 quartu
	Hand { entry: 0x2d8aa0, end: 0x2d9b58, hash: 0xb688ce52c891890c, est: 0, f: suck_held_any }, // 16 kalebo
	Hand { entry: 0x2dc900, end: 0x2dd9b8, hash: 0x6e56c355639e0fc1, est: 0, f: suck_held_any }, // 17 veldin_orbit
	Hand { entry: 0x2e4450, end: 0x2e5508, hash: 0x95f4a7163477434d, est: 0, f: suck_held_any }, // 18 veldin_finale
];
