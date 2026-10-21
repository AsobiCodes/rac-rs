//! "Controls: Modern" movement for retail mode (PC extra, R&C3-style strafe built from R&C1's own data): on top of the modern
//! camera's RMB aim mode (`modcam.rs`, which already makes Ratchet face the aim and lets him run and shoot at once).
//! Off (`RAC_MOVE_CLASSIC=1`, or the modern camera off / not aiming) nothing is registered or written: retail stays bit-exact.
//!
//! - Flips (logic): while RMB aims and Ratchet stands or runs, Cross with the stick pushed sideways or backwards (relative to
//!   the aim, i.e. the camera heading) enters R&C1's own crouch high jump, state 0xb (side flip seq 0x1c / 0x1d, back flip 0x1f),
//!   without the crouch (R1/R2) and stick-edge presses retail asks for. A wrapping hand on retail's jump chooser (hero state
//!   transition's `FUN_0021bcb8(window, mode)`, per level, table `ROWS`) checks the Cross press with retail's own input-ring
//!   test, then calls retail's SetState(0xb, 1) with the logic facing 0x13f3e8 set to the aim yaw, so retail's own stick
//!   classification (HeroClassifyStick, stick against facing) picks the variant relative to the aim. Anything else runs retail.
//!   The flip is 360 degrees: the nearest flip type is picked (keyboard diagonals: side flips) and the flip travels along the
//!   stick, without the carry of the run before it bending it toward the old path (`FLIP_CARRY`).
//! - Torso twist and backpedal (draw only): after each field `post_field` measures the hero's ground path (retail's move
//!   vector 0x13f430) against the drawn yaw (moby +0x48, which modcam sets to the camera yaw while aiming) and publishes
//!   `Draw`: the legs' twist toward the path (at most 90 degrees: past that the legs face the path + 180 and the run clip
//!   plays reversed), with hysteresis and smoothing. The renderer (viewer/handler.rs) applies it to the posed joint matrices
//!   (`twist_mats`) and reverses the run clock (`reverse_clock`). Game logic is never touched by this part.
use super::modcam;
use ee_oracle::cpu::Cpu;
use ee_oracle::rec::{register_hand, Hand};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use crate::retail::disc::va;

/// RAC_MOVE_CLASSIC=1 keeps the classic (retail) movement. A later options row ("Controls: Classic / Modern") replaces the
/// env read here; everything else asks `enabled()`.
fn classic() -> bool {
	static C: OnceLock<bool> = OnceLock::new();
	*C.get_or_init(|| std::env::var("RAC_MOVE_CLASSIC").is_ok_and(|v| v != "0" && !v.is_empty()))
}

/// The switch: the modern camera is enabled (`--modern-cam`, or the PC options) and the movement is not set to classic, and the
/// PC options' Strafe aiming row is On with Camera Modern (pcext.rs; always true with `RAC_PC_EXT=0`). Read at run time.
pub fn enabled() -> bool {
	modcam::enabled() && !classic() && super::pcext::strafe_on()
}

const HERO_POS: u32 = 0x13f3d0;
const HERO_YAW: u32 = 0x13f3e8;
const HERO_MOVE: u32 = 0x13f430;
const HERO_WISH: u32 = 0x13f4d0;
const HERO_STATE: u32 = 0x1413d4;
const HERO_MOBY: u32 = 0x1413d0;
const BODY_FORM: u32 = 0x1413f4;
const GAME_MODE: u32 = 0x15f5a4;
const STICK_X: u32 = 0x141070;
const STICK_Y: u32 = 0x141074;
/// Pad button Cross in retail's input ring (hero state transition: `FUN_00258ee0(0x40, window, 0)`).
const CROSS: u32 = 0x40;

/// Hero states a strafe flip may start from: idle 0, run / walk 2 and 0x73 (hero_movement.md state ids).
const FLIP_STATES: [u32; 3] = [0, 2, 0x73];
/// Hero states the legs twist in: the same ground states. Firing keeps state 2 (the weapon arms are a layer on top, measured), the
/// wrench swing (state 0x13) lunges along the aim, so it needs no twist and the legs ease back to 0 there.
const TWIST_STATES: [u32; 3] = [0, 2, 0x73];
/// The run / walk cycles of the hero (sequences 3 and 4) are the clips played reversed for a backpedal.
const RUN_SEQS: [u8; 2] = [3, 4];

// ---- per level retail addresses (checked against extracted/levels/N_*/overlay.elf, see STRAFE.md) -----------------------

struct Row {
	/// The jump chooser `FUN_0021bcb8(window, mode)` of the hero state transition (Eudora 0x21bcb8, 39 instructions, a binary
	/// twin on all 19 levels): Cross pressed within `window` fields -> crouch high jump (state 0xb) with R1/R2 held and a
	/// stick edge, else the plain jump (state 7) when `mode` is 0, else 0.
	entry: u32,
	/// Its first call: the input ring test `FUN_00258ee0(mask, window, out)` (Eudora 0x258ee0), a pure read.
	press: u32,
	/// Its last call: SetState(state, flag) (Eudora 0x215538).
	set_state: u32,
}

static ROWS: [Row; 19] = [
	Row { entry: 0x228758, press: 0x2666f0, set_state: 0x221fd8 }, // 0 veldin
	Row { entry: 0x2414d0, press: 0x27b258, set_state: 0x23cb00 }, // 1 novalis
	Row { entry: 0x22f528, press: 0x267cf0, set_state: 0x22b290 }, // 2 aridia
	Row { entry: 0x21b208, press: 0x254830, set_state: 0x2161b0 }, // 3 kerwan
	Row { entry: 0x21bcb8, press: 0x258ee0, set_state: 0x215538 }, // 4 eudora
	Row { entry: 0x2526d0, press: 0x290638, set_state: 0x24cad0 }, // 5 rilgar
	Row { entry: 0x239fd8, press: 0x275458, set_state: 0x235200 }, // 6 nebula
	Row { entry: 0x250540, press: 0x28e968, set_state: 0x249dc0 }, // 7 umbris
	Row { entry: 0x235c78, press: 0x2705e8, set_state: 0x2306a0 }, // 8 batalia
	Row { entry: 0x2483b8, press: 0x285030, set_state: 0x241c38 }, // 9 gaspar
	Row { entry: 0x21bd40, press: 0x259450, set_state: 0x2155c0 }, // 10 orxon
	Row { entry: 0x2525b8, press: 0x28ba48, set_state: 0x24d6b8 }, // 11 pokitaru
	Row { entry: 0x244d60, press: 0x27ed50, set_state: 0x23fc38 }, // 12 hoven
	Row { entry: 0x2341c0, press: 0x272078, set_state: 0x22da40 }, // 13 oltanis_orbit
	Row { entry: 0x2341f0, press: 0x26e060, set_state: 0x22fb40 }, // 14 oltanis
	Row { entry: 0x21bbe0, press: 0x255c60, set_state: 0x2167a0 }, // 15 quartu
	Row { entry: 0x223db8, press: 0x261a98, set_state: 0x21df00 }, // 16 kalebo
	Row { entry: 0x224410, press: 0x260000, set_state: 0x21e090 }, // 17 veldin_orbit
	Row { entry: 0x22d498, press: 0x2685e8, set_state: 0x227938 }, // 18 veldin_finale
];
/// `ROWS` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
static ROWS_USA: [Row; 19] = [
	Row { entry: 0x228b78, press: 0x266d60, set_state: 0x2223f8 }, // 0 veldin
	Row { entry: 0x241968, press: 0x27b940, set_state: 0x23cf98 }, // 1 novalis
	Row { entry: 0x22f9c0, press: 0x2683d8, set_state: 0x22b728 }, // 2 aridia
	Row { entry: 0x21b6a0, press: 0x254f18, set_state: 0x216648 }, // 3 kerwan
	Row { entry: 0x21c158, press: 0x2595d0, set_state: 0x2159d8 }, // 4 eudora
	Row { entry: 0x252ae8, press: 0x290cc0, set_state: 0x24cee8 }, // 5 rilgar
	Row { entry: 0x23a478, press: 0x275b48, set_state: 0x2356a0 }, // 6 nebula
	Row { entry: 0x2509e0, press: 0x28f058, set_state: 0x24a260 }, // 7 umbris
	Row { entry: 0x236110, press: 0x270cd0, set_state: 0x230b38 }, // 8 batalia
	Row { entry: 0x248858, press: 0x285720, set_state: 0x2420d8 }, // 9 gaspar
	Row { entry: 0x21c1e0, press: 0x259b40, set_state: 0x215a60 }, // 10 orxon
	Row { entry: 0x252a50, press: 0x28c130, set_state: 0x24db50 }, // 11 pokitaru
	Row { entry: 0x2451f8, press: 0x27f438, set_state: 0x2400d0 }, // 12 hoven
	Row { entry: 0x234660, press: 0x272768, set_state: 0x22dee0 }, // 13 oltanis_orbit
	Row { entry: 0x234608, press: 0x26e6c8, set_state: 0x22ff58 }, // 14 oltanis
	Row { entry: 0x21c078, press: 0x256348, set_state: 0x216c38 }, // 15 quartu
	Row { entry: 0x224250, press: 0x2621a0, set_state: 0x21e398 }, // 16 kalebo
	Row { entry: 0x2248b0, press: 0x2606f0, set_state: 0x21e530 }, // 17 veldin_orbit
	Row { entry: 0x22d930, press: 0x268cd0, set_state: 0x227dd0 }, // 18 veldin_finale
];

// Hashes: rec::hash (FNV-1a over the words) of [entry, end) of each overlay.elf; every function is 39 instructions
// (Eudora's was the ee-hand `combo_input_fire` row 0x87731a4978079f20; the Eudora hands are gone, 2026-10-19). Its fall-back
// runs the retail code.
static HANDS: [Hand; 19] = [
	Hand { entry: 0x228758, end: 0x2287f4, hash: 0x9b39924c29812120, est: 0, f: jump_any }, // 0
	Hand { entry: 0x2414d0, end: 0x24156c, hash: 0x1b12cb9128bfc53e, est: 0, f: jump_any }, // 1
	Hand { entry: 0x22f528, end: 0x22f5c4, hash: 0x82ced79aeb654e90, est: 0, f: jump_any }, // 2
	Hand { entry: 0x21b208, end: 0x21b2a4, hash: 0x07e19d74a020f120, est: 0, f: jump_any }, // 3
	Hand { entry: 0x21bcb8, end: 0x21bd54, hash: 0x87731a4978079f20, est: 0, f: jump_any }, // 4
	Hand { entry: 0x2526d0, end: 0x25276c, hash: 0xccbbbc86cc2bdfe4, est: 0, f: jump_any }, // 5
	Hand { entry: 0x239fd8, end: 0x23a074, hash: 0x9af85e871a83c0e4, est: 0, f: jump_any }, // 6
	Hand { entry: 0x250540, end: 0x2505dc, hash: 0x3e91eed3ba6b26a4, est: 0, f: jump_any }, // 7
	Hand { entry: 0x235c78, end: 0x235d14, hash: 0x210fe86c2d3a8114, est: 0, f: jump_any }, // 8
	Hand { entry: 0x2483b8, end: 0x248454, hash: 0x8857520abb4bdb10, est: 0, f: jump_any }, // 9
	Hand { entry: 0x21bd40, end: 0x21bddc, hash: 0x0dffee5fa2382ec8, est: 0, f: jump_any }, // 10
	Hand { entry: 0x2525b8, end: 0x252654, hash: 0x3677d4b9e0d27f94, est: 0, f: jump_any }, // 11
	Hand { entry: 0x244d60, end: 0x244dfc, hash: 0x87bb1876d473aca0, est: 0, f: jump_any }, // 12
	Hand { entry: 0x2341c0, end: 0x23425c, hash: 0x7dbc53bcc4c5b084, est: 0, f: jump_any }, // 13
	Hand { entry: 0x2341f0, end: 0x23428c, hash: 0xa71b0d38ecfdcf40, est: 0, f: jump_any }, // 14
	Hand { entry: 0x21bbe0, end: 0x21bc7c, hash: 0x8983c8ab952ef510, est: 0, f: jump_any }, // 15
	Hand { entry: 0x223db8, end: 0x223e54, hash: 0x8f376bb32d3229a4, est: 0, f: jump_any }, // 16
	Hand { entry: 0x224410, end: 0x2244ac, hash: 0xe676d807f6b152b8, est: 0, f: jump_any }, // 17
	Hand { entry: 0x22d498, end: 0x22d534, hash: 0xbc26e91f97c240c4, est: 0, f: jump_any }, // 18
];

/// Registers the hands (Retail::load); nothing when the movement is classic or the modern camera is off. The hands exist even
/// while the PC options say Strafe Off or Camera Classic: they check `enabled()` per call, so the rows can switch them on later.
pub fn register() {
	if modcam::enabled() && !classic() {
		register_hand(crate::retail::disc::hands(&HANDS));
	}
}

// ---- state --------------------------------------------------------------------------------------------------------------

/// What the renderer needs of one field (part of `HeroParts`, so it travels with the field it was measured in).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Draw {
	/// Legs twist in radians about the vertical (positive = toward +yaw), |twist| <= 90 degrees.
	pub twist: f32,
	/// Play the run clip reversed (backpedal).
	pub rev: bool,
	/// How far retail's leg rotation overrides are faded out (0 = as retail, 1 = none; `leg_fx`): eased in and out with `rev`.
	pub leg_off: f32,
}

struct S {
	field: u32,
	pos: [f32; 2],
	have_pos: bool,
	twist: f32,
	rev: bool,
	/// Smoothed path direction relative to the drawn yaw (radians) and whether it is known (moving, or held since).
	dir: f32,
	have_dir: bool,
	/// Last move vectors of the path average (ring of `AVG`), how many are valid, the write index, and the fields in a row the
	/// run direction wanted to change (`LATCH`).
	hist: [[f32; 2]; AVG],
	hn: usize,
	hi: usize,
	cand: u8,
	draw: Draw,
	flips: u32,
	/// Entry speed (u per field) of the strafe flip in the air (0 = none, or a standing flip) and whether it is a back flip
	/// (`pre_field`).
	flip: f32,
	back: bool,
	log: Option<std::fs::File>,
	log_tried: bool,
}

static ST: Mutex<S> = Mutex::new(S { field: 0, pos: [0.0; 2], have_pos: false, twist: 0.0, rev: false, dir: 0.0, have_dir: false, hist: [[0.0; 2]; AVG], hn: 0, hi: 0, cand: 0, draw: Draw { twist: 0.0, rev: false, leg_off: 0.0 }, flips: 0, flip: 0.0, back: false, log: None, log_tried: false });
static IN: AtomicBool = AtomicBool::new(false);

/// The twist / reverse state of the last field (`retail::snap` copies it into the hero parts).
pub fn current() -> Draw {
	ST.lock().map_or(Draw::default(), |s| s.draw)
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

fn log(s: &mut S, line: &str) {
	if !s.log_tried {
		s.log_tried = true;
		if let Ok(p) = std::env::var("RAC_STRAFE_LOG") {
			s.log = std::fs::File::create(p).ok();
		}
	}
	if let Some(f) = s.log.as_mut() {
		let _ = writeln!(f, "{line}");
	}
}

// ---- flips (logic) ---------------------------------------------------------------------------------------------------

struct Guard;
impl Drop for Guard {
	fn drop(&mut self) {
		IN.store(false, Ordering::SeqCst);
	}
}

fn jump_any(c: &mut Cpu) -> bool {
	match (*crate::retail::disc::pick(&ROWS, &ROWS_USA)).iter().find(|r| r.entry == c.pc) {
		Some(r) => jump(c, r),
		None => false,
	}
}

/// Class of the stick against the aim: 0 forward, 1 side (+90 or -90), 2 back: the nearest of retail's flip directions
/// (HeroClassifyStick's 45 degree windows). The keyboard diagonals (45 / 135 degrees up to rounding) are side flips on both
/// sides: forward diagonals flip sideways instead of jumping, back diagonals flip sideways instead of back.
fn stick_class(d: f32) -> u8 {
	let a = d.abs().to_degrees();
	if a < 44.0 {
		0
	} else if a <= 136.0 {
		1
	} else {
		2
	}
}

/// State 0xb's flip heading (SetState: the wish yaw 0x13f4d0 at entry; the horizontal velocity runs along it) and its carry
/// heading (SetState: the facing of 8 ticks ago from the yaw ring 0x141478; each tick the facing turns to it and the entry speed
/// 0x13f7a4 is carried along it, times sin of its angle to the flip heading), hero_air_states.md.
const FLIP_HEAD: u32 = 0x13f788;
const FLIP_CARRY: u32 = 0x13f78c;
/// State 0xb's entry speed (u per field; 0 below 1.5 u/s), its speed ramp along the flip heading (approaches 4.2 u/s at 8 u/s2,
/// and 0 once descending late in the flip), its variant (3 = back flip) and the hero's run speed, which the flip physics sets
/// to the entry speed every tick (0 for a back flip) and the run after the landing starts from (Eudora OV:29718-29757).
const FLIP_ENTRY: u32 = 0x13f7a4;
const FLIP_RAMP: u32 = 0x13f7a8;
const FLIP_VARIANT: u32 = 0x13f7a0;
const RUN_SPEED: u32 = 0x13f4e4;
/// Jump landing count (0 in the air, 1 on the touchdown field, then counting while the landing lasts), hero_air_states.md.
const FLIP_LANDED: u32 = 0x13f768;

/// Wrapping hand on the jump chooser: with RMB aiming and the stick sideways / back of the aim, a Cross press (retail's own
/// input-ring test with the caller's window) starts retail's crouch high jump (state 0xb); everything else is the retail
/// function. The hero state transition has just run hero_stick_wish_yaw(1.0) when it calls the chooser, so the wish yaw
/// 0x13f4d0 is the stick (plus the camera yaw) of this field.
fn jump(c: &mut Cpu, r: &'static Row) -> bool {
	if IN.swap(true, Ordering::SeqCst) {
		return false;
	}
	let _g = Guard;
	if enabled() {
		if let Some(aim) = modcam::aim_yaw() {
			let state = c.mem.rd32(HERO_STATE);
			if FLIP_STATES.contains(&state) && c.mem.rd8(BODY_FORM) == 0 && c.mem.rd32(va(GAME_MODE)) == 0 {
				let window = c.g(4);
				let (x, y) = (f32::from_bits(c.mem.rd32(STICK_X)), f32::from_bits(c.mem.rd32(STICK_Y)));
				let wish = f32::from_bits(c.mem.rd32(HERO_WISH));
				let d = wrap(wish - aim);
				let mag = (x * x + y * y).sqrt();
				let class = stick_class(d);
				if class != 0 && mag > 0.5 {
					// Retail's own test for a Cross press in the last `window` fields (a pure read of the input ring).
					c.s(4, CROSS as u64);
					c.s(5, window);
					c.s(6, 0);
					c.call(r.press);
					let pressed = c.g(2) as u32 != 0;
					if let Ok(mut s) = ST.lock() {
						let line = format!(
							"jump_request field {} state {state:#x} window {window} stick {x:.2} {y:.2} mag {mag:.2} wish {wish:.3} aim {aim:.3} d {:.1} class {class} pressed {pressed}",
							s.field,
							d.to_degrees()
						);
						log(&mut s, &line);
						if pressed {
							s.flips += 1;
						}
					}
					if !pressed {
						// Retail's answer when no Cross press is in the window.
						c.s(2, 0);
						return true;
					}
					// Retail's variant (HeroClassifyStick: wish yaw against the logic facing) gets the facing that puts the stick
					// exactly on the chosen flip direction (side +-90, back 180), then the facing is the aim again.
					let side = if class == 1 { std::f32::consts::FRAC_PI_2.copysign(d) } else { std::f32::consts::PI };
					c.mem.wr32(HERO_YAW, wrap(wish - side).to_bits());
					c.s(4, 0xb);
					c.s(5, 1);
					c.call(r.set_state);
					c.mem.wr32(HERO_YAW, wrap(aim).to_bits());
					// 360 degree flips: retail's flip runs along the stick (FLIP_HEAD), but the run before it is carried along the
					// facing of 8 ticks ago (strafing: the old path), which bends a flip across the run toward the old path. The
					// carry heading on the flip heading makes the carry 0 (sin 0) and turns the logic facing to the flip: the flip
					// travels along the stick like retail's flip in the run direction (speed ramp and height unchanged).
					// Momentum: retail's flip starts its ramp from 0 (a run drops from 5.7 to 0.16 u/s at the press and builds back to
					// 4.2 over 27 fields, then eases toward 0 while descending: 2.6 at the landing, scratch_strafe2 b1_fh_90), and a
					// back flip zeroes the run speed, so the run after its landing starts from 0 (38 fields to full speed, b1_fh_180).
					// The ramp starts at the entry speed instead and `pre_field` holds it there through the flip, and a back flip lands
					// into the run at the entry speed like a side flip does. A standing flip has entry speed 0: unchanged.
					if c.mem.rd32(HERO_STATE) == 0xb {
						let head = c.mem.rd32(FLIP_HEAD);
						c.mem.wr32(FLIP_CARRY, head);
						let entry = c.mem.rd32(FLIP_ENTRY);
						c.mem.wr32(FLIP_RAMP, entry);
						if let Ok(mut s) = ST.lock() {
							s.flip = f32::from_bits(entry);
							s.back = c.mem.rd32(FLIP_VARIANT) == 3;
						}
					}
					c.s(2, 1);
					return true;
				}
			}
		}
	}
	c.call(r.entry);
	true
}

/// Worker, before each field: the field counter of the log lines, and the momentum of a strafe flip out of a run: in the air
/// its speed ramp is held at the entry speed, and a back flip that touches down with the stick held lands into the run at the
/// entry speed (the run speed retail's flip physics zeroed for the back flip every tick), so it flows on like the side flips.
pub fn pre_field(m: &mut ee_oracle::Machine, _fin: &super::FieldIn) {
	if !enabled() {
		return;
	}
	if let Ok(mut s) = ST.lock() {
		s.field = s.field.wrapping_add(1);
		if s.flip > 0.0 {
			let state = m.rd32(HERO_STATE);
			let (x, y) = (m.rdf(STICK_X), m.rdf(STICK_Y));
			let held = (x * x + y * y).sqrt() > 0.5;
			let speed = s.flip;
			if state == 0xb {
				// In the air: the flip keeps the run's speed (the ramp eases one retail step, 0.16 u/s, toward 4.2 or 0 per field).
				if m.rdf(FLIP_RAMP) < speed {
					m.cpu.mem.wr32(FLIP_RAMP, speed.to_bits());
				}
				// Touched down with the stick held: the next field goes from the flip into the run (state 2) and runs the run's
				// physics with the run speed retail's back flip zeroed every tick: one field at speed 0, then the run from 0. The
				// side flips keep the entry speed there (measured: b2_fh_90 field 152 5.56 u/s, b3_fh_180 field 149 0).
				if s.back && held && m.rd32(FLIP_LANDED) != 0 && m.rdf(RUN_SPEED) < speed {
					m.cpu.mem.wr32(RUN_SPEED, speed.to_bits());
					let line = format!("back_flip_touchdown field {} run speed {speed:.4}", s.field);
					log(&mut s, &line);
				}
			} else {
				if s.back && (state == 2 || state == 0x73) && m.rdf(RUN_SPEED) < speed {
					m.cpu.mem.wr32(RUN_SPEED, speed.to_bits());
					let line = format!("back_flip_landing field {} state {state:#x} run speed {speed:.4}", s.field);
					log(&mut s, &line);
				}
				(s.flip, s.back) = (0.0, false);
			}
		}
	}
}

// ---- torso twist and backpedal (draw only) --------------------------------------------------------------------------

/// Legs may twist this far from the aim (radians); past it the legs face the path + 180 and the run clip plays reversed.
const LIMIT: f32 = std::f32::consts::FRAC_PI_2;
/// The run reverses once the smoothed path direction is this far from the aim (112 degrees) for `LATCH` fields, and goes
/// forward again below `REV_OFF` (98 degrees): pure sideways (90) is always forward (legs toward the path).
const REV_ON: f32 = 1.955;
const REV_OFF: f32 = 1.710;
/// Smoothing of the path direction per field, the twist's rate limit (radians per field, 31.5 degrees) and the ease back to 0.
const DIR_SMOOTH: f32 = 0.35;
const RATE: f32 = 0.55;
const SMOOTH: f32 = 0.3;
/// A path change larger than this (a reversal: A <-> D, W <-> S) takes the new direction at once instead of sweeping through
/// the side.
const SNAP: f32 = 2.6;
/// Below this ground path per field (game units; run speed 5.7 u/s = 0.114) the hero counts as standing in state 0; in the
/// walk / run states (2, 0x73) any step above `MIN_WALK` counts as moving: a slow (analog) backwards walk moves 0.018 u per
/// field and the first field of a start 0.003, and below 0.03 the clip used to play forward (moonwalk, scratch_strafe2 bkw).
const MIN_STEP: f32 = 0.03;
const MIN_WALK: f32 = 0.002;
/// Below this path speed the legs follow the stick (wish yaw 0x13f4d0) instead of the path: starting off, retail turns the
/// logic facing from the old facing to the stick at about 0.2 rad per field while it accelerates (measured: 16 fields for a
/// back start), so the path sweeps from forward through the side to the back, which twisted the legs sideways and then
/// flipped the clip (scratch_strafe2 bk1 fields 43-58). The stick is where the hero is about to go.
const SETTLE: f32 = 0.04;
/// The path is the mean of the last `AVG` move vectors (a wall contact makes retail's vector alternate with a period of 3 fields),
/// and the run direction changes only after `LATCH` fields in a row asked for it (the twist holds still meanwhile).
const AVG: usize = 6;
const LATCH: u8 = 3;
/// Per field step of `Draw::leg_off` (4 fields from retail's leg overrides to none and back).
const LEG_FADE: f32 = 0.25;
/// Air states (fall 6, jump 7, 9, 0xe): retail's move vector reads 0 there, so the twist, the run direction and the path are
/// held through the jump instead of measured.
const AIR_STATES: [u32; 4] = [6, 7, 9, 0xe];
/// Run states in which a stall (wall, turn-around: move vector about 0 for a few fields) holds the twist.
const STALL_STATES: [u32; 2] = [2, 0x73];

/// Worker, after each field (after modcam's): measures the ground path against the drawn yaw and updates the twist.
/// Track (moving on the ground): the path direction `dir` relative to the aim is smoothed, the run direction latches with
/// hysteresis away from sideways, and the twist moves toward its target at most `RATE` per field. Hold (air, stall): nothing
/// changes. Reset (anything else: standing, skid, flips, attacks, no aim): the twist eases to 0, the run goes forward.
pub fn post_field(m: &ee_oracle::Machine) {
	if !enabled() {
		// The Strafe aiming row went Off (or Camera Classic) while the legs were twisted: draw straight again.
		if let Ok(mut s) = ST.lock() {
			if s.draw != Draw::default() || s.twist != 0.0 {
				(s.twist, s.rev, s.draw, s.have_pos, s.have_dir, s.hn, s.hi, s.cand) = (0.0, false, Draw::default(), false, false, 0, 0, 0);
			}
		}
		return;
	}
	let Ok(mut s) = ST.lock() else { return };
	let pos = [m.rdf(HERO_POS), m.rdf(HERO_POS + 4)];
	let step = if s.have_pos { [pos[0] - s.pos[0], pos[1] - s.pos[1]] } else { [0.0; 2] };
	(s.pos, s.have_pos) = (pos, true);
	// The ground path: retail's move vector of this field (0x13f430, units per field), averaged over the last fields.
	let raw = [m.rdf(HERO_MOVE), m.rdf(HERO_MOVE + 4)];
	let state = m.rd32(HERO_STATE);
	let aim = modcam::aim_yaw();
	let moby = m.rd32(HERO_MOBY) & 0x01ff_ffff;
	let drawn = if moby != 0 { m.rdf(moby + 0x48) } else { 0.0 };
	let inst = raw[0].hypot(raw[1]);
	let ok = aim.is_some() && m.rd8(BODY_FORM) == 0 && m.rd32(va(GAME_MODE)) == 0;
	let min = if state == 0 { MIN_STEP } else { MIN_WALK };
	let moving = ok && TWIST_STATES.contains(&state) && inst > min && inst < 2.0;
	let mut mv = [0.0f32; 2];
	if moving {
		let i = s.hi;
		s.hist[i] = raw;
		s.hi = (i + 1) % AVG;
		s.hn = (s.hn + 1).min(AVG);
		for h in &s.hist[..s.hn] {
			mv = [mv[0] + h[0] / s.hn as f32, mv[1] + h[1] / s.hn as f32];
		}
	}
	let speed = mv[0].hypot(mv[1]);
	let track = moving && speed > min;
	// Hold: in the air, a stall in a run state, or the path mean passing through 0 at a reversal, once a direction is known.
	let hold = !track && s.have_dir && ok && (AIR_STATES.contains(&state) || STALL_STATES.contains(&state) || moving);
	let mut d_log = f32::NAN;
	let mode;
	if track {
		let wish_rel = wrap(m.rdf(HERO_WISH) - drawn);
		let d = if speed < SETTLE { wish_rel } else { wrap(mv[1].atan2(mv[0]) - drawn) };
		d_log = d;
		let e = wrap(d - s.dir);
		let fresh = !s.have_dir;
		s.dir = if fresh || e.abs() > SNAP { d } else { wrap(s.dir + e * DIR_SMOOTH) };
		s.have_dir = true;
		let ad = s.dir.abs();
		// Retail turns around in an arc (the path sweeps through the back at A <-> D, through the side at W <-> S): the run
		// reverses while the path is behind, and goes forward again as soon as the stick (wish yaw 0x13f4d0) no longer
		// points behind and the path has left the back sector, which keeps the leg swing at the way out small.
		let wish = wish_rel.abs();
		let want = if s.rev { ad < REV_OFF || (wish < REV_OFF && ad < REV_ON) } else { ad > REV_ON };
		s.cand = if want { s.cand.saturating_add(1) } else { 0 };
		// A start from standing straight toward the back (path and stick behind) backpedals at once: no forward swing first.
		if fresh && ad > REV_ON && wish > REV_ON {
			s.rev = true;
			s.cand = 0;
		} else if s.cand >= LATCH {
			s.rev = !s.rev;
			s.cand = 0;
		}
		// Reversed: the legs face the path + 180 (so |twist| <= 90 at all times). While a switch is pending the twist holds.
		if s.cand == 0 {
			let target = (if s.rev { wrap(s.dir - std::f32::consts::PI * s.dir.signum()) } else { s.dir }).clamp(-LIMIT, LIMIT);
			s.twist += (target - s.twist).clamp(-RATE, RATE);
		}
		mode = if s.cand > 0 { 'P' } else { 'T' };
	} else if hold {
		mode = 'H';
	} else {
		s.rev = false;
		s.cand = 0;
		s.have_dir = false;
		s.hn = 0;
		s.hi = 0;
		s.twist -= s.twist * SMOOTH;
		if s.twist.abs() < 0.002 {
			s.twist = 0.0;
		}
		mode = 'R';
	}
	let rev = s.rev && (track || hold);
	let leg_off = (s.draw.leg_off + if rev { LEG_FADE } else { -LEG_FADE }).clamp(0.0, 1.0);
	s.draw = Draw { twist: s.twist, rev, leg_off };
	if s.log.is_some() || !s.log_tried {
		let (seq, cur, nxt, old, ph) = if moby != 0 { (m.rd8(moby + 0x53), m.rd8(moby + 0x50), m.rd8(moby + 0x51), m.rd8(moby + 0x52), m.rdf(moby + 0x54)) } else { (0, 0, 0, 0, 0.0) };
		// Leg direction against the path (0 = the legs point along the path, forward or reversed).
		let err = if track { wrap(mv[1].atan2(mv[0]) - drawn - s.twist - if s.rev { std::f32::consts::PI } else { 0.0 }).to_degrees() } else { f32::NAN };
		// The hero +0x60 layer list (retail.rs `layer_list`): seq, frame, from seq / frame, t, weight, joints (count, min..max).
		let mut lay = String::new();
		let mut o = if moby != 0 { m.rd32(moby + 0x60) & 0x01ff_ffff } else { 0 };
		for _ in 0..6 {
			if o == 0 {
				break;
			}
			let (list, n) = (m.rd32(o + 0xc) & 0x01ff_ffff, m.rd16(o + 2).min(128) as u32);
			let js: Vec<u8> = (0..n).map(|k| m.rd8(list + k)).collect();
			lay += &format!(" [s{} f{} fs{} ff{} t{:.2} w{:.2} j{}:{}..{}]", m.rd8(o + 0x23), m.rd8(o + 0x21), m.rd8(o + 0x22), m.rd8(o + 0x20), m.rdf(o + 0x24), m.rdf(o + 8), n, js.iter().min().unwrap_or(&0), js.iter().max().unwrap_or(&0));
			o = m.rd32(o + 0x1c) & 0x01ff_ffff;
		}
		// The hero +0x64 joint override records (retail.rs `joint_records`): joint, mode, rotation xyz.
		let mut r = if moby != 0 { m.rd32(moby + 0x64) & 0x01ff_ffff } else { 0 };
		lay += " fx";
		for _ in 0..16 {
			if r == 0 {
				break;
			}
			lay += &format!(" [j{} b{} {:.3} {:.3} {:.3}]", (m.rd32(r + 4) & 0xffff) / 0x40, m.rd8(r + 3), m.rdf(r + 0x10), m.rdf(r + 0x14), m.rdf(r + 0x18));
			r = m.rd32(r + 8) & 0x01ff_ffff;
		}
		let line = format!(
			"field {} state {state:#x} eq {} layers{lay} seq {seq} cur {cur} next {nxt} pos {:.3} {:.3} {:.3} speed {speed:.3} step {:.3} {:.3} vel {:.3} {:.3} logic {:.3} drawn {drawn:.3} wish {:.3} aim {} d {:.1} twist {:.1} rev {} flips {} dir {:.1} mode {mode} err {err:.1} old {old} p {ph:.3} inst {inst:.4} ramp {:.3} carry {:.3} fh {:.3} fc {:.3} land {} tick {}",
			s.field, m.rd32(0x140408), pos[0], pos[1], m.rdf(HERO_POS + 8), step[0], step[1], mv[0], mv[1], m.rdf(HERO_YAW), m.rdf(HERO_WISH), aim.map_or("-".to_string(), |a| format!("{a:.3}")), d_log.to_degrees(), s.twist.to_degrees(), s.draw.rev, s.flips, s.dir.to_degrees(), m.rdf(0x13f7a8), m.rdf(0x13f7a4), m.rdf(FLIP_HEAD), m.rdf(FLIP_CARRY), m.rd32(0x13f768), m.rd32(0x13f4e8)
		);
		log(&mut s, &line);
	}
}

/// Class 1204: a shown attachment (collision_and_gameplay.md creator table "1204 attachment") that rides 0.24 behind the hero at
/// head height, constant in the logic hero frame (measured, runs/logf2.tsv `parts` lines); turned with the parts within 1.5 u.
const ATTACH_1204: u32 = 1204;

/// Head items (hero item table rows 5-7: classes 433, 1289 O2 mask / helmet, 1290 Pilot's Helmet; flags +0x806): plain mobys that
/// retail poses on the hero's head from the logic-yaw joint matrices like the parts above, so they are turned with them while
/// aiming (before, only the body was drawn toward the aim and the helmet stayed on retail's logic facing: desynced strafing).
const HEAD_ITEMS: [u32; 3] = [433, 1289, 1290];

/// Hero-attached props drawn from RAM (modcam): retail places Clank (`*0x1404d4`, class 601), the back object (`*0x1404d0`,
/// class 607), the bomb waiting in the Bomb Glove (class 121, +0x20 == 0), the class 1204 attachment and a held moby on a hero
/// joint (+0x34 bit 0x100)
/// at the joints of ITS hero matrix (rows +0xc0, the logic yaw), while modcam turns only the drawn yaw +0x48 of the hero. They
/// are turned about the hero by (drawn - rows yaw) so they ride with the drawn Ratchet. Draw only; RAM is not written.
pub fn turn_parts(m: &ee_oracle::Machine, hero: u32, held: u32, mobys: &mut [super::Moby], glows: &mut [([f32; 3], f32, f32, u32)]) {
	if !modcam::enabled() || modcam::aim_yaw().is_none() {
		return;
	}
	let drawn = m.rdf(hero + 0x48);
	let r0 = m.rdv3(hero + 0xc0);
	if r0[0] == 0.0 && r0[1] == 0.0 {
		return;
	}
	let rows = r0[1].atan2(r0[0]);
	let delta = wrap(drawn - rows);
	let field = ST.lock().map_or(0, |s| s.field);
	let hp = m.rdv3(hero + 0x10);
	if field % 10 == 0 && ST.lock().is_ok_and(|s| s.log.is_some()) {
		let mut l = format!("parts field {field} drawn {drawn:.3} rows {rows:.3} logic {:.3} delta {:.1} near", m.rdf(HERO_YAW), delta.to_degrees());
		for mb in mobys.iter() {
			let (dx, dy, dz) = (mb.pos[0] - hp[0], mb.pos[1] - hp[1], mb.pos[2] - hp[2]);
			if dx * dx + dy * dy + dz * dz < 6.25 {
				l += &format!(" [{} {:#x} s{} f{:#x} v{} {:.2} {:.2} {:.2}]", mb.class, mb.addr, m.rd8(mb.addr + 0x20), m.rd16(mb.addr + 0x34), mb.shown as u8, dx, dy, dz);
			}
		}
		if let Ok(mut s) = ST.lock() {
			log(&mut s, &l);
		}
	}
	if delta.abs() < 1e-4 {
		return;
	}
	let (d0, d4) = (m.rd32(0x1404d0) & 0x01ff_ffff, m.rd32(0x1404d4) & 0x01ff_ffff);
	let (sn, cs) = delta.sin_cos();
	for mb in mobys.iter_mut() {
		let part = (mb.addr == d0 && d0 != 0)
			|| (mb.addr == d4 && d4 != 0)
			|| (mb.class == crate::game::BOMB && m.rd8(mb.addr + 0x20) == 0)
			|| (mb.class == ATTACH_1204 && (mb.pos[0] - hp[0]).hypot(mb.pos[1] - hp[1]) < 1.5)
			|| (HEAD_ITEMS.contains(&mb.class) && (mb.pos[0] - hp[0]).hypot(mb.pos[1] - hp[1]) < 1.5)
			|| (mb.addr == held && held != 0 && m.rd16(held + 0x34) & 0x100 != 0);
		if !part {
			continue;
		}
		let (x, y) = (mb.pos[0] - hp[0], mb.pos[1] - hp[1]);
		mb.pos[0] = hp[0] + cs * x - sn * y;
		mb.pos[1] = hp[1] + sn * x + cs * y;
		mb.rot[2] = wrap(mb.rot[2] + delta);
	}
	// Clank's bulb glow (retail's slot-3 item light, queue_fx `hero_glows`) is placed from the logic-yaw joints like the bulb mesh
	// (class 1204) above: turned with it, so it stays on the bulb.
	for g in glows.iter_mut().filter(|g| super::is_bulb_glow(g)) {
		let (x, y) = (g.0[0] - hp[0], g.0[1] - hp[1]);
		g.0[0] = hp[0] + cs * x - sn * y;
		g.0[1] = hp[1] + sn * x + cs * y;
	}
}

// ---- renderer side ----------------------------------------------------------------------------------------------------

/// Joints of the hero skeleton the twist moves, with the share of the twist each takes (decomp skeleton dump of class 0,
/// 111 joints, joint 1 = pelvis, 2..5 spine, 6 chest = the first joint whose children are arms / neck, 92..103 legs,
/// 104..110 tail): the pelvis, legs and tail take it all, the spine hands it back (2..4: 0.75, 0.5, 0.25), joint 5 (Clank's
/// attach point), the chest, arms, neck and head none: the upper body keeps the aim, Clank stays on the chest.
fn twist_share(joint: usize) -> f32 {
	match joint {
		1 | 92..=110 => 1.0,
		2 => 0.75,
		3 => 0.5,
		4 => 0.25,
		_ => 0.0,
	}
}

/// Twists the posed hero's lower body by `theta` about the vertical axis through the posed pelvis. `mats` are the skin
/// matrices (3x4 row major, bind -> posed model space, z up) `Animated::pose_held` returns; the joint matrix of the pelvis
/// gives the pivot.
pub fn twist_mats(mats: &mut [[f32; 12]], anim: &crate::anim::Animated, theta: f32) {
	if theta.abs() < 1e-4 || mats.len() < 111 {
		return;
	}
	let jw = anim.joint_world(mats, 1);
	let p = [jw[3], jw[7]];
	for (j, m) in mats.iter_mut().enumerate() {
		let w = twist_share(j);
		if w == 0.0 {
			continue;
		}
		let (s, c) = (theta * w).sin_cos();
		// Rotation of the rows x, y of the linear part and of the translation about the pivot.
		for k in 0..4 {
			let (x, y) = if k == 3 { (m[3] - p[0], m[7] - p[1]) } else { (m[k], m[4 + k]) };
			let (nx, ny) = (c * x - s * y, s * x + c * y);
			if k == 3 {
				m[3] = nx + p[0];
				m[7] = ny + p[1];
			} else {
				m[k] = nx;
				m[4 + k] = ny;
			}
		}
	}
}

/// Hero leg joints (`twist_share`: 92..97 one leg, 98..103 the other).
const LEG_JOINTS: std::ops::RangeInclusive<usize> = 92..=103;

/// Retail's leg rotation overrides (hero +0x64 records on the leg joints: the feet-on-slope pass FUN_0022c088 that bends hips,
/// knees and feet to the ground under each foot, and the run lean) are computed for Ratchet facing his path with the clip
/// playing forward. A backpedal draws the legs turned 180 degrees with the clip reversed, so those bends land on the wrong leg
/// and the wrong way: on a slope one leg stayed lifted while backpedaling (seen on Veldin, and on the Novalis start slope,
/// runs/g1.slog fields 48-108: hips -0.087 / -0.195). While `leg_off` > 0 the rotation of every mode-0 leg record is eased
/// toward none (scale and offset records stay: the boots hide the feet through them); a backpedal plays the plain clip.
pub fn leg_fx(fx: &mut [crate::anim::JointFx], d: Draw) {
	if d.leg_off <= 0.0 {
		return;
	}
	let w = 1.0 - d.leg_off.min(1.0);
	for f in fx.iter_mut().filter(|f| f.blend.is_none() && LEG_JOINTS.contains(&f.joint)) {
		// Toward the identity of the record's own sign (q and -q are the same turn; the multiply keeps the sign).
		let one = if f.rot[3] < 0.0 { -1.0 } else { 1.0 };
		let q = [f.rot[0] * w, f.rot[1] * w, f.rot[2] * w, f.rot[3] * w + (1.0 - w) * one];
		let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
		f.rot = if n > 1e-6 { q.map(|v| v / n) } else { [0.0, 0.0, 0.0, one] };
	}
}

/// Renderer side of the drawn hero clock: (the last drawn clock was a reversed run / walk clip, the clock was fading, the fade
/// running now started from a reversed pose).
static CLK: Mutex<(bool, bool, bool)> = Mutex::new((false, false, false));

/// The hero clock as drawn, once per field after retail's clock was mirrored (viewer/handler.rs): with `rev` (backpedal) the
/// run / walk cycle plays backwards, including the target frame of a fade into it (`reverse_target`). Returns true while a fade
/// runs that started from a reversed pose: retail's FROM frame (hero +0x68) is then its FORWARD pose of that field, so the
/// renderer fades from its own snapshot of the drawn (reversed) pose instead. Before, every fade out of a backpedal (walk <->
/// run, stop skid) started from the forward frame and every fade into one aimed at the forward frame: a leg pop at each
/// change (scratch_strafe2 bk1: fields 58, 65, 162).
pub fn draw_clock(c: &mut crate::anim::AnimClock, frames: usize, rev: bool) -> bool {
	let Ok(mut k) = CLK.lock() else { return false };
	let (last_rev, was_fading, from_rev) = *k;
	let fading = c.old_seq == 0xff;
	let from_rev = fading && if was_fading { from_rev } else { last_rev };
	let changed = rev && if fading { reverse_target(c, frames) } else { reverse_clock(c, frames) };
	*k = (changed, fading, from_rev);
	from_rev
}

/// A fade (snapshot -> frame `next` of a run / walk cycle) into a reversed cycle: the target frame `next` becomes `N - next`,
/// the frame `reverse_clock` shows when the fade ends (retail then sets cur = next, p = 0).
fn reverse_target(c: &mut crate::anim::AnimClock, frames: usize) -> bool {
	if frames < 2 || !RUN_SEQS.contains(&c.new_seq) || c.next < 0 || c.next as usize >= frames {
		return false;
	}
	let n = frames as i32;
	c.next = (n - c.next) % n;
	true
}

/// Plays the hero's run / walk cycle backwards: the clock phase `cur + p` of an N-frame cycle becomes `N - (cur + p)`.
/// Only a settled clock (no snapshot fade) of the run sequences is touched. Returns whether it changed.
pub fn reverse_clock(c: &mut crate::anim::AnimClock, frames: usize) -> bool {
	if frames < 2 || c.old_seq == 0xff || !RUN_SEQS.contains(&c.new_seq) || c.cur < 0 || c.cur as usize >= frames {
		return false;
	}
	let n = frames as i32;
	c.cur = n - 1 - c.cur;
	c.next = (c.cur + 1) % n;
	c.p = (1.0 - c.p).clamp(0.0, 1.0);
	true
}
