//! Quick Select wheel by mouse direction (GitHub #13: "hold T and choose the weapon with the mouse, like Rift Apart").
//!
//! Retail already works like that with the pad: holding Triangle opens the 8-sector wheel (the player's weapon / gadget list, update
//! FUN_00237780 of every level overlay), the LEFT STICK picks the sector, and RELEASING Triangle closes the wheel and equips the
//! picked weapon. So nothing in the game has to change: while Triangle is held (the T key, the middle mouse button, a pad's
//! Triangle) the mouse's direction is turned into a left-stick push. Nothing is written to the game; the mouse look of the Modern
//! camera is held back while the wheel is up (`owns_motion`).
//!
//! The retail rule this file feeds (stick byte -> float, the 0.9 length, the sector formula, the narrow even sectors, the 3 pi / 16
//! hold, the empty-slot hop; all [C]) is written down in decomp/notes/glove_prereq_inventory_input.md, "Quick Select wheel: how the
//! stick picks a sector". In short: each stick byte is d = |b - 0x7f|, 0 up to 0x2f, else (d - 0x30) / 0x4c clamped to 1; the wheel
//! needs the LENGTH of the float pair to exceed 0.9 and takes the angle (sector 0 = up, clockwise, 8 sectors). The EVEN sectors (up /
//! right / down / left) are only 27 deg wide, the diagonals 63 deg, and a held pick stays while the stick is within 33.75 deg of its
//! centre. That is why the first version (a unit-circle push, 0.57 per axis on a diagonal = 0.80 long) only ever reached N / S / E / W.
//!
//! So this file never pushes "the mouse angle" itself: it decides the sector the mouse points at (45 deg wide, a small hysteresis so
//! the boundary does not flicker) and pushes the stick through that sector's CENTRE with the corner-of-the-square length
//! (`bytes_at`, the inverse of the byte -> float rule), which retail resolves to that sector from any previous pick.
//!
//! The push leans `LEAN` (5 deg) off the centre toward the side of the sector the pointer is on. Why: when the picked slot is EMPTY
//! (the entry's first word is 0) a FRESH pick tries ONE neighbour, the next sector clockwise when the stick leans clockwise of the
//! sector centre, else the previous one (a stick exactly on the centre rounds to the previous one), and takes it only if that slot is
//! filled; a HELD pick is kept even when empty and never hops. A push through the exact centre would send every hop the same way
//! whatever half of the empty slot the mouse points at; with the lean the hop follows the pointer like a real stick. 5 deg is inside
//! the even sectors' +-13.5 deg band and still 40 deg (> 33.75) from the adjacent centre, so the pushed sector always wins.
//!
//! The mouse moves a virtual pointer inside the wheel, and only WHILE retail's wheel is actually up (`open_in`: the fade byte of the
//! Quick Select HUD element, HUD element 3 = array + 0x1b0, byte +0x70, which the update FUN_00237780 counts up while Triangle is held
//! and back to 0 after the release; the element array sits at a per-level address, `ELEM_PAL` / `ELEM_USA`). Everywhere else Triangle
//! held (ship, hoverboard, a tap on the middle button) leaves the stick and the camera alone. The first `ARM` fields of a wheel
//! ignore the mouse (the jiggle of pressing the wheel button). Motion adds up scaled by the Mouse speed row (`mousespeed_scale`, the
//! knob for a mouse's DPI), up to `REACH` from the centre so reversing is quick (45 deg round the rim = 79 counts), and beyond `DEAD`
//! the pointed sector is pushed. Inside the dead zone the stick stays as the keyboard (WASD) or the controller gives it and retail
//! keeps its last pick. Off by default (retail 1:1): Graphics -> System row Quick Select mouse (`pcinput::wheel_mouse`), or
//! `RAC_WHEEL_MOUSE=1`. Test only: `RAC_WHEEL_MOUSE_SCRIPT` injects mouse motion (see `script_delta`).

use crate::fmath::Fm;
use std::sync::Mutex;

/// The pointer inside the wheel can go this far from its centre (scaled mouse counts): turning round takes `REACH + DEAD`, a step
/// to the next sector round the rim 79 counts.
const REACH: f32 = 100.0;
/// Inside this distance the mouse pushes nothing (and the pick stays).
const DEAD: f32 = 26.0;
/// Fields after the wheel opened in which the mouse is ignored.
const ARM: u32 = 6;
/// Quick Select sectors (the 8 ids at 0x141ea0).
const SECTORS: u32 = 8;
/// A pointer must be this far (in sectors) beyond a sector's edge before the pick moves on: 0.15 of 45 deg = 6.75 deg.
const HYST: f32 = 0.15;
/// The push leans this far (radians, 5 deg) off a sector's centre toward the pointer's side (file doc: the empty-slot hop).
const LEAN: f32 = 5.0 * std::f32::consts::PI / 180.0;

/// HUD element array (stride 0x90, slot 0 = ammo box) per level 0..18 on the PAL / USA disc (omni/tests.rs, found by the static slot anchors).
const ELEM_PAL: [u32; 19] = [0x17dc50, 0x17e050, 0x17e1d0, 0x17dcd0, 0x17dd50, 0x17e0d0, 0x17e350, 0x17dc50, 0x17e3d0, 0x17dd50, 0x17e050, 0x17e5d0, 0x17e050, 0x17ded0, 0x17e350, 0x17e1d0, 0x17e050, 0x17e4d0, 0x17e5d0];
const ELEM_USA: [u32; 19] = [0x17dc50, 0x17e0d0, 0x17e250, 0x17dd50, 0x17ddd0, 0x17e0d0, 0x17e3d0, 0x17dcd0, 0x17e450, 0x17ddd0, 0x17e0d0, 0x17e650, 0x17e0d0, 0x17df50, 0x17e350, 0x17e250, 0x17e0d0, 0x17e550, 0x17e650];

/// RAM address of the Quick Select element's fade byte on `level` (None outside a level).
fn fade_byte(usa: bool, level: u32) -> Option<u32> {
	let t = if usa { &ELEM_USA } else { &ELEM_PAL };
	t.get(level as usize).map(|b| b + 3 * 0x90 + 0x70)
}

/// Is retail's Quick Select wheel up on `level` (level word)? `rd8` reads RAM.
pub fn open_in(level: u32, rd8: impl Fn(u32) -> u8) -> bool {
	fade_byte(super::disc::usa(), level).is_some_and(|a| rd8(a) != 0)
}

/// The pointer and the sector it picked (pure, so the tests can drive it with mouse paths).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Pointer {
	v: [f32; 2],
	sector: Option<u32>,
}

impl Pointer {
	/// Adds a motion (screen counts, y down); the pointer stays within `REACH` of the centre.
	fn add(&mut self, dx: f32, dy: f32) {
		let (x, y) = (self.v[0] + dx, self.v[1] + dy);
		let len = (x * x + y * y).sqrt();
		self.v = if len > REACH { [x / len * REACH, y / len * REACH] } else { [x, y] };
	}

	/// Where the pointer points, in sectors from the top, clockwise (0.0 = up, 2.0 = right).
	fn at(&self) -> f32 {
		self.v[0].fatan2(-self.v[1]) / std::f32::consts::FRAC_PI_4
	}

	/// How far (sectors, -4..4) the pointer is clockwise of the centre of `sector`.
	fn off(&self, sector: u32) -> f32 {
		(self.at() - sector as f32 + SECTORS as f32 / 2.0).rem_euclid(SECTORS as f32) - SECTORS as f32 / 2.0
	}

	/// Which side of `sector`'s centre the pointer is on: 1.0 clockwise (toward the next sector), -1.0 counter-clockwise (exactly on
	/// the centre counts as clockwise).
	fn side(&self, sector: u32) -> f32 {
		if self.off(sector) < 0.0 { -1.0 } else { 1.0 }
	}

	/// The sector the pointer points at: None inside the dead zone (the last pick stays); else the nearest sector (0 = up, clockwise),
	/// the held one kept until the pointer is `HYST` beyond its edge.
	fn pick(&mut self) -> Option<u32> {
		if self.v[0].fhypot(self.v[1]) < DEAD {
			return None;
		}
		if let Some(c) = self.sector {
			if self.off(c).abs() <= 0.5 + HYST {
				return Some(c);
			}
		}
		self.sector = Some((self.at().round() as i32).rem_euclid(SECTORS as i32) as u32);
		self.sector
	}
}

/// The left stick bytes (x, y; PS2: up = 0x00, right = 0xff) for a full push at `sector`'s centre leaning `lean` (+1.0 clockwise /
/// -1.0 counter-clockwise / 0.0 none) `LEAN` off it (file doc: the empty-slot hop).
fn stick_bytes(sector: u32, lean: f32) -> [u8; 2] {
	bytes_at(sector as f32 * std::f32::consts::FRAC_PI_4 + lean * LEAN)
}

/// The stick bytes that make retail's pad update read a full push at angle `a` (radians from the top, clockwise): the larger axis at
/// 1.0 (byte 0x7f +- 124), the other axis in proportion, so the length is >= 1 (retail needs > 0.9) and the angle is `a`. The byte ->
/// float rule is retail's: d = |b - 0x7f|, 0 up to 0x2f, else (d - 0x30) / 0x4c.
fn bytes_at(a: f32) -> [u8; 2] {
	let (x, y) = (a.fsin(), -a.fcos());
	let m = x.abs().max(y.abs());
	[x / m, y / m].map(|c| {
		let d = if c.abs() < 0.5 / 76.0 { 0 } else { 0x30 + (c.abs() * 76.0).round() as i32 };
		if d == 0 { 0x80 } else { (0x7f + d * if c < 0.0 { -1 } else { 1 }) as u8 }
	})
}

#[derive(Default)]
struct Wheel {
	held: bool,
	age: u32,
	p: Pointer,
}

impl Wheel {
	/// Mouse motion (already scaled): counts only while the wheel is up and past the first `ARM` fields.
	fn motion(&mut self, dx: f32, dy: f32) {
		if self.held && self.age > ARM {
			self.p.add(dx, dy);
		}
	}
}

static W: Mutex<Wheel> = Mutex::new(Wheel { held: false, age: 0, p: Pointer { v: [0.0; 2], sector: None } });

/// Is the feature on: the Quick Select mouse row (`pcinput::wheel_mouse`) or `RAC_WHEEL_MOUSE=1` (read once)?
pub fn enabled() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	super::pcinput::wheel_mouse() || *ON.get_or_init(|| std::env::var("RAC_WHEEL_MOUSE").is_ok_and(|v| v == "1"))
}

/// Raw mouse motion (viewer/handler.rs `device_event`, after the mod and dev-camera owners, main thread).
pub fn motion(dx: f64, dy: f64) {
	if !enabled() {
		return;
	}
	let k = super::pcext::mousespeed_scale();
	if let Ok(mut w) = W.lock() {
		w.motion(dx as f32 * k, dy as f32 * k);
	}
}

/// The wheel is up and the mouse is picking: the camera must not turn with it (modcam.rs `ui_mouse`).
pub fn owns_motion() -> bool {
	enabled() && W.lock().is_ok_and(|w| w.held)
}

/// One `RAC_WHEEL_MOUSE_SCRIPT` entry: from field, to field (wheel age, inclusive), DX, DY (raw counts per field).
type ScriptEntry = (u32, u32, f32, f32);

/// Parses `FROM-TO:DX,DY;..` (entries that do not parse are skipped).
fn parse_script(v: &str) -> Vec<ScriptEntry> {
	let one = |e: &str| -> Option<ScriptEntry> {
		let (r, d) = e.split_once(':')?;
		let (a, b) = r.split_once('-')?;
		let (x, y) = d.split_once(',')?;
		Some((a.trim().parse().ok()?, b.trim().parse().ok()?, x.trim().parse().ok()?, y.trim().parse().ok()?))
	};
	v.split(';').filter_map(one).collect()
}

/// The summed motion of the entries that cover wheel field `age` (None when none does).
fn script_sum(s: &[ScriptEntry], age: u32) -> Option<(f32, f32)> {
	let mut sum = None::<(f32, f32)>;
	for e in s.iter().filter(|e| (e.0..=e.1).contains(&age)) {
		let t = sum.get_or_insert((0.0, 0.0));
		(t.0, t.1) = (t.0 + e.2, t.1 + e.3);
	}
	sum
}

/// TEST ONLY (`RAC_WHEEL_MOUSE_SCRIPT=FROM-TO:DX,DY;..`): the mouse motion injected in wheel field `age` (fields since the wheel
/// opened, decimal; DX, DY raw counts per field). It takes the place of real mouse motion in the scripted game runs.
fn script_delta(age: u32) -> Option<(f32, f32)> {
	static S: std::sync::OnceLock<Vec<ScriptEntry>> = std::sync::OnceLock::new();
	script_sum(S.get_or_init(|| std::env::var("RAC_WHEEL_MOUSE_SCRIPT").map(|v| parse_script(&v)).unwrap_or_default()), age)
}

/// One field (input.rs `tick_keys`): `up` = Triangle is held in play (no menu), `open` = retail's wheel is up (`open_in`). Sets the left
/// stick `s` (`[rx, ry, lx, ly]`) when the pointer is beyond the dead zone. Wheel closed: the pointer is back in the centre for the next time.
pub fn step(up: bool, open: bool, s: &mut [u8; 4]) {
	if !enabled() {
		return;
	}
	let Ok(mut w) = W.lock() else { return };
	field(&mut w, up, open, super::pcext::mousespeed_scale(), script_delta, s);
}

/// The per-field work of `step` on any wheel state (so the tests can run it): `k` = the Mouse speed scale, `script` = the injected
/// motion of a wheel field (`script_delta` in the game).
fn field(w: &mut Wheel, up: bool, open: bool, k: f32, script: impl Fn(u32) -> Option<(f32, f32)>, s: &mut [u8; 4]) {
	if !(up && open) {
		if w.held {
			eprintln!("wheel_mouse: wheel closed");
		}
		*w = Wheel::default();
		return;
	}
	if !w.held {
		eprintln!("wheel_mouse: wheel up, the mouse picks after {ARM} fields");
	}
	w.held = true;
	w.age += 1;
	if let Some((dx, dy)) = script(w.age) {
		w.motion(dx * k, dy * k);
	}
	let before = w.p.sector;
	if let Some(sec) = w.p.pick() {
		let [x, y] = stick_bytes(sec, w.p.side(sec));
		(s[2], s[3]) = (x, y);
		if before != Some(sec) && std::env::var_os("RAC_WHEEL_MOUSE_LOG").is_some() {
			eprintln!("wheel_mouse: age {} pointer {:.0},{:.0} -> sector {sec}", w.age, w.p.v[0], w.p.v[1]);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

	/// Retail's pad update (boot FUN_00216e20): stick byte -> float, d = |b - 0x7f|, 0 up to 0x2f, else (d - 0x30) / 0x4c clamped to 1.
	fn axis(b: u8) -> f32 {
		let d = (b as i32 - 0x7f).abs();
		if d <= 0x2f {
			return 0.0;
		}
		let v = ((d - 0x30) as f32 / 0x4c as f32).min(1.0);
		if b < 0x7f { -v } else { v }
	}

	/// Retail's selection (overlay FUN_00237780, the stick branch, [C] disassembly 0x2378dc-0x237c54; a pick = element +0x74; `full[s]`
	/// = the wheel entry of slot s has a non-zero first word): length > 0.9 else nothing changes; a held pick stays while the stick
	/// is within 0.5890486 (3 pi / 16) of its centre (no hop); else sector = trunc((angle + 2 pi + pi / 2 + pi / 8) * 8 / 2 pi) % 8
	/// and an EVEN sector gives way to its neighbour in the outer 0.2 (gp-0x7498) of its span on each side. The angle is atan2(y, x)
	/// of the two floats (up = -y, so sector 0 is the top). A fresh pick of an empty slot tries the one neighbour on the stick's side
	/// of the sector centre (dir = +1 when FUN_001ffd28(sum - (s * pi/4 + pi/8)) > 0, else -1) and takes it if it is filled.
	fn retail_select(b: [u8; 2], cur: Option<u32>, full: [bool; 8]) -> Option<u32> {
		let (x, y) = (axis(b[0]), axis(b[1]));
		if x.fhypot(y) <= 0.9 {
			return cur;
		}
		let ang = y.fatan2(x);
		if let Some(c) = cur {
			let centre = c as f32 * TAU / 8.0 - PI;
			let d = (ang - PI / 2.0 - centre).abs().rem_euclid(TAU);
			if d.min(TAU - d) <= 0.5890486 {
				return cur;
			}
		}
		let sum = ang + PI + PI + FRAC_PI_2 + PI / 8.0;
		let f = sum * (8.0 / TAU);
		let mut s = f.trunc() as i32 % 8;
		if s % 2 == 0 {
			let fr = f - f.floor();
			if 1.0 - 0.2 < fr {
				s += 1;
			} else if fr < 0.2 {
				s -= 1;
			}
			s = (s + 8) % 8;
		}
		let mut off = sum - (s as f32 * 0.7853982 + 0.3926991);
		while off >= PI {
			off -= PI + PI;
		}
		while off < -PI {
			off += PI + PI;
		}
		let dir = if off > 0.0 { 1 } else { -1 };
		if !full[s as usize] {
			let n = (s + dir).rem_euclid(8);
			if full[n as usize] {
				s = n;
			}
		}
		Some(s as u32)
	}

	/// `retail_select` with every slot filled.
	fn retail_pick(b: [u8; 2], cur: Option<u32>) -> Option<u32> {
		retail_select(b, cur, [true; 8])
	}

	/// The bytes the file pushes for sector `s` while the pointer `p` points there.
	fn push(p: &Pointer, s: u32) -> [u8; 2] {
		stick_bytes(s, p.side(s))
	}

	/// Mouse path -> per step (sector pushed, sector retail then holds); retail sees only the pushed bytes.
	fn drive(p: &mut Pointer, retail: &mut Option<u32>, path: &[(f32, f32)]) -> Vec<(Option<u32>, Option<u32>)> {
		path.iter()
			.map(|&(dx, dy)| {
				p.add(dx, dy);
				let pushed = p.pick();
				if let Some(s) = pushed {
					*retail = retail_pick(push(p, s), *retail);
				}
				(pushed, *retail)
			})
			.collect()
	}

	/// A pointer `len` counts out toward `sector`.
	fn toward(sector: u32, len: f32) -> (f32, f32) {
		let a = sector as f32 * FRAC_PI_4;
		(a.fsin() * len, -a.fcos() * len)
	}

	/// A pointer 60 counts out at `at` sectors from the top (clockwise, fractional).
	fn pointer_at(at: f32) -> Pointer {
		let a = at * FRAC_PI_4;
		Pointer { v: [a.fsin() * 60.0, -a.fcos() * 60.0], sector: None }
	}

	/// What retail ends on when the mouse points at `at` sectors from a fresh pick, the Quick Select slots being `full`.
	fn pointed(at: f32, full: [bool; 8]) -> Option<u32> {
		let mut p = pointer_at(at);
		let s = p.pick()?;
		retail_select(push(&p, s), None, full)
	}

	/// The v0.4.2 test 2 bug: a unit-circle push on a diagonal reads 0.8 long in retail (below its 0.9), the new bytes read 1.4.
	#[test]
	fn the_diagonal_push_reaches_retails_threshold() {
		let old = [0x80u8 + 90, 0x80 - 90];
		assert!(axis(old[0]).fhypot(axis(old[1])) < 0.9, "the old push never reached retail's 0.9");
		for s in 0..8 {
			for lean in [-1.0, 0.0, 1.0] {
				let b = stick_bytes(s, lean);
				assert!(axis(b[0]).fhypot(axis(b[1])) > 0.99, "sector {s} lean {lean}: {b:?}");
			}
		}
		assert_eq!(stick_bytes(0, 0.0), [0x80, 3]);
		assert_eq!(stick_bytes(2, 0.0), [0xfb, 0x80]);
		assert_eq!(stick_bytes(1, 0.0), [0xfb, 3]);
		assert_eq!(stick_bytes(3, 0.0), [0xfb, 0xfb]);
		assert_eq!(stick_bytes(5, 0.0), [3, 0xfb]);
	}

	/// The lean really is about 5 deg off the centre, on the side asked for, as retail's float pair sees it (the bytes are rounded).
	#[test]
	fn the_lean_is_five_degrees_toward_the_pointers_side() {
		for s in 0..8u32 {
			for lean in [-1.0f32, 1.0] {
				let b = stick_bytes(s, lean);
				let ang = axis(b[1]).fatan2(axis(b[0]));
				let mut off = (ang - (s as f32 * FRAC_PI_4 - FRAC_PI_2)).to_degrees();
				off = (off + 540.0).rem_euclid(360.0) - 180.0;
				assert!(off * lean > 0.0, "sector {s}: leans to the wrong side ({off} deg)");
				assert!(off.abs() > 3.5 && off.abs() < 6.5, "sector {s} lean {lean}: {off} deg");
			}
		}
	}

	/// Every sector is picked by retail from every previous pick (also from none), whichever side the pointer leans: the push beats
	/// the narrow even sectors, the 33.75 deg hold and the neighbour hand-over.
	#[test]
	fn retail_resolves_each_pushed_sector_from_any_start() {
		for to in 0..8 {
			for lean in [-1.0, 0.0, 1.0] {
				for from in std::iter::once(None).chain((0..8).map(Some)) {
					assert_eq!(retail_pick(stick_bytes(to, lean), from), Some(to), "from {from:?} to {to} lean {lean}");
				}
			}
		}
	}

	/// The model itself has retail's unequal bands: the old circle push is ignored, a fresh stick 20 deg off the top already gives
	/// the diagonal (the up sector is 27 deg wide), a held top stays.
	#[test]
	fn the_model_has_retails_unequal_bands() {
		assert_eq!(retail_pick([0x80 + 90, 0x80 - 90], None), None, "0.8 long: retail ignores it");
		assert_eq!(retail_pick([0xff, 0xff], None), Some(3), "down right corner");
		let b = bytes_at(20.0f32.to_radians());
		assert_eq!(retail_pick(b, None), Some(1));
		assert_eq!(retail_pick(b, Some(0)), Some(0), "but a held top stays");
	}

	/// Pointing at an EMPTY slot (the Glove + 4 weapons, slots 5 6 7 empty): retail tries the neighbour on the side of the sector
	/// centre the stick leans to, so the pointer's half of the slot decides.
	#[test]
	fn an_empty_slot_hands_over_to_the_neighbour_on_the_pointers_side() {
		let full = [true, true, true, true, true, false, false, false];
		assert_eq!(pointed(7.3, full), Some(0), "slot 7, clockwise half: the neighbour is slot 0, filled");
		assert_eq!(pointed(6.7, full), Some(7), "slot 7, counter-clockwise half: slot 6 is empty too, so nothing");
		assert_eq!(pointed(5.3, full), Some(5), "slot 5, clockwise half: slot 6 is empty, nothing");
		assert_eq!(pointed(4.7, full), Some(4), "slot 5, counter-clockwise half: slot 4 is filled");
		assert_eq!(pointed(2.3, full), Some(2), "a filled slot is taken as it is, on either half");
		assert_eq!(pointed(1.7, full), Some(2));
	}

	/// For every sector and both halves: the hop goes to the neighbour on the pointer's side, from a fresh pick and from any pick a
	/// push can beat; a HELD empty pick (the 5 deg lean is inside the 33.75 deg hold) is kept like retail does with a stick.
	#[test]
	fn the_hop_follows_the_pointers_side_for_every_sector() {
		for s in 0..8u32 {
			let mut full = [true; 8];
			full[s as usize] = false;
			for side in [-1.0f32, 1.0] {
				let want = (s as i32 + side as i32).rem_euclid(8) as u32;
				for from in std::iter::once(None).chain((0..8).filter(|&c| c != s).map(Some)) {
					assert_eq!(retail_select(stick_bytes(s, side), from, full), Some(want), "slot {s} side {side} from {from:?}");
				}
				assert_eq!(retail_select(stick_bytes(s, side), Some(s), full), Some(s), "slot {s} held");
			}
		}
	}

	/// All 8 directions from the centre: the sector under the pointer is the one retail ends on.
	#[test]
	fn eight_directions_from_the_centre() {
		for s in 0..8 {
			let (mut p, mut r) = (Pointer::default(), None);
			let out = drive(&mut p, &mut r, &[toward(s, 60.0)]);
			assert_eq!(out, vec![(Some(s), Some(s))], "direction {s}");
		}
	}

	/// A slow drag (half a degree per step) once round the rim: every sector comes up in order, retail agrees at every step, and
	/// the pick never goes back and forth.
	#[test]
	fn a_slow_circle_walks_the_sectors_in_order() {
		let (mut p, mut r) = (Pointer::default(), None);
		let mut seq = vec![];
		for i in 0..=720 {
			let a = (i as f32 * 0.5).to_radians();
			p.v = [a.fsin() * REACH, -a.fcos() * REACH];
			let o = drive(&mut p, &mut r, &[(0.0, 0.0)])[0];
			assert_eq!(o.0, o.1, "retail follows the push at {i} half degrees");
			if seq.last() != o.0.as_ref() {
				seq.push(o.0.unwrap());
			}
		}
		assert_eq!(seq, vec![0, 1, 2, 3, 4, 5, 6, 7, 0], "in order, no flicker back");
	}

	/// Jitter of 3 deg round a sector edge never changes the pick; a step to 35 deg (12.5 past the edge) does, at once.
	#[test]
	fn boundary_jitter_does_not_flicker() {
		let (mut p, mut r) = (Pointer::default(), None);
		p.add(0.0, -REACH);
		assert_eq!(p.pick(), Some(0));
		for i in 0..40 {
			let a = (22.5f32 + if i % 2 == 0 { 3.0 } else { -3.0 }).to_radians(); // 22.5 deg = the edge of sectors 0 and 1
			p.v = [a.fsin() * REACH, -a.fcos() * REACH];
			let s = p.pick();
			assert_eq!(s, Some(0), "jitter step {i}");
			r = retail_pick(push(&p, s.unwrap()), r);
			assert_eq!(r, Some(0), "retail stays too at jitter step {i}");
		}
		let a = 35.0f32.to_radians();
		p.v = [a.fsin() * REACH, -a.fcos() * REACH];
		assert_eq!(p.pick(), Some(1), "past the edge + hysteresis it moves on at once");
		assert_eq!(retail_pick(push(&p, 1), r), Some(1));
	}

	/// Fast flicks: a huge stroke is clamped to the rim, so the way back is one stroke of 130 counts, never a sweep through the centre.
	#[test]
	fn quick_flicks_and_reversals() {
		let (mut p, mut r) = (Pointer::default(), None);
		let o = drive(&mut p, &mut r, &[(300.0, 0.0), (-130.0, 0.0), (0.0, 400.0), (0.0, -400.0)]);
		assert_eq!(o[0], (Some(2), Some(2)), "right");
		assert_eq!(o[1], (Some(6), Some(6)), "one stroke back = left");
		assert_eq!(o[2].1, Some(4), "then down");
		assert_eq!(o[3].1, Some(0), "then up");
	}

	/// Inside the dead zone nothing is pushed and the last pick stays for retail.
	#[test]
	fn the_dead_zone_pushes_nothing() {
		let (mut p, mut r) = (Pointer::default(), None);
		assert_eq!(drive(&mut p, &mut r, &[(DEAD - 1.0, 0.0)])[0], (None, None));
		drive(&mut p, &mut r, &[toward(3, 70.0)]);
		assert_eq!(p.sector, Some(3));
		p.v = [3.0, 2.0];
		assert_eq!(p.pick(), None);
		assert_eq!(p.sector, Some(3));
	}

	/// The side of a sector's centre: clockwise, counter-clockwise, and across the top (sector 0, 7.9 sectors = a little left of up).
	#[test]
	fn the_side_is_the_pointers_half_of_the_sector() {
		assert_eq!(pointer_at(2.2).side(2), 1.0);
		assert_eq!(pointer_at(1.8).side(2), -1.0);
		assert_eq!(pointer_at(7.9).side(0), -1.0, "just left of up is counter-clockwise of sector 0");
		assert_eq!(pointer_at(0.1).side(0), 1.0);
		assert_eq!(pointer_at(0.1).side(7), 1.0, "held sector 7 with the pointer past the top: still clockwise");
	}

	/// `step`'s field work: the first `ARM` fields push nothing whatever the mouse does, then the left stick bytes (s[2], s[3]) carry
	/// the pointed sector's push, the right stick (s[0], s[1]) is never touched.
	#[test]
	fn the_field_arms_then_pushes_only_the_left_stick() {
		let mut w = Wheel::default();
		let mut s = [0x11u8, 0x22, 0x80, 0x80];
		let right = |_: u32| Some((200.0, 0.0)); // a hard flick to the right in every field
		for i in 1..=ARM {
			field(&mut w, true, true, 1.0, right, &mut s);
			assert_eq!(s, [0x11, 0x22, 0x80, 0x80], "field {i} is still arming");
			assert!(w.held && w.age == i && w.p.v == [0.0; 2]);
		}
		field(&mut w, true, true, 1.0, right, &mut s);
		assert_eq!(w.p.sector, Some(2));
		let [x, y] = push(&w.p, 2);
		assert_eq!(s, [0x11, 0x22, x, y], "lx / ly carry the push, rx / ry stay");
		assert_eq!(retail_pick([x, y], None), Some(2));
	}

	/// The Mouse speed scale multiplies the injected motion: 60 counts clear the dead zone at 1.0 but not at 0.4.
	#[test]
	fn the_field_scales_the_motion() {
		let go = |k: f32| {
			let mut w = Wheel { held: true, age: ARM, p: Pointer::default() };
			let mut s = [0x80u8; 4];
			field(&mut w, true, true, k, |_| Some((0.0, 60.0)), &mut s);
			(s, w.p)
		};
		let (s, p) = go(1.0);
		assert_eq!(p.sector, Some(4), "down");
		assert_eq!(s, [0x80, 0x80, push(&p, 4)[0], push(&p, 4)[1]]);
		let (s, p) = go(0.4);
		assert_eq!((s, p.sector), ([0x80; 4], None), "24 counts: inside the dead zone");
	}

	/// Inside the dead zone the pad's own left stick (WASD / controller) is left as it is; the previous pick stays.
	#[test]
	fn the_field_leaves_the_pad_stick_in_the_dead_zone() {
		let mut w = Wheel { held: true, age: ARM, p: Pointer { v: [0.0, 0.0], sector: Some(3) } };
		let mut s = [0x80, 0x80, 0xff, 0x00];
		field(&mut w, true, true, 1.0, |_| Some((DEAD - 1.0, 0.0)), &mut s);
		assert_eq!(s, [0x80, 0x80, 0xff, 0x00]);
		assert_eq!(w.p.sector, Some(3));
	}

	/// A wheel that closes (Triangle released, or retail's fade is back at 0) puts the pointer back in the centre and pushes nothing;
	/// the next wheel arms again.
	#[test]
	fn a_closed_wheel_resets_and_pushes_nothing() {
		for (up, open) in [(false, true), (true, false), (false, false)] {
			let mut w = Wheel::default();
			let mut s = [0x80u8; 4];
			let flick = |_: u32| Some((0.0, -200.0));
			for _ in 0..=ARM {
				field(&mut w, true, true, 1.0, flick, &mut s);
			}
			assert_eq!(w.p.sector, Some(0));
			s = [0x80, 0x80, 0x55, 0x66];
			field(&mut w, up, open, 1.0, flick, &mut s);
			assert_eq!(s, [0x80, 0x80, 0x55, 0x66], "up {up} open {open}: closed pushes nothing");
			assert!(!w.held && w.age == 0 && w.p == Pointer::default(), "up {up} open {open}: reset");
			field(&mut w, true, true, 1.0, flick, &mut s);
			assert_eq!((w.age, w.p.v), (1, [0.0; 2]), "the next wheel arms again");
		}
	}

	/// Real mouse motion counts only while a wheel is up and after the first `ARM` fields.
	#[test]
	fn raw_motion_is_ignored_while_arming_or_closed() {
		let mut w = Wheel::default();
		w.motion(50.0, 0.0);
		assert_eq!(w.p.v, [0.0; 2], "no wheel");
		(w.held, w.age) = (true, ARM);
		w.motion(50.0, 0.0);
		assert_eq!(w.p.v, [0.0; 2], "still arming");
		w.age = ARM + 1;
		w.motion(50.0, 0.0);
		assert_eq!(w.p.v, [50.0, 0.0]);
	}

	/// The injected test motion: entries parse (bad ones are skipped), overlapping entries add, other fields get nothing.
	#[test]
	fn the_script_parses_and_sums() {
		let s = parse_script("10-12:3,-4; 11-11:1,1;junk;5-x:1,1");
		assert_eq!(s, vec![(10, 12, 3.0, -4.0), (11, 11, 1.0, 1.0)]);
		assert_eq!(script_sum(&s, 9), None);
		assert_eq!(script_sum(&s, 10), Some((3.0, -4.0)));
		assert_eq!(script_sum(&s, 11), Some((4.0, -3.0)));
		assert_eq!(script_sum(&s, 13), None);
		assert_eq!(script_sum(&parse_script(""), 1), None);
	}

	/// The wheel is up only when the Quick Select element's fade byte is not 0; Veldin's element 3 fade byte is 0x17de70 (0x17dc50 + 0x220), USA Novalis' 0x17e0d0 + 0x220.
	#[test]
	fn the_wheel_state_is_the_element_fade_byte() {
		assert_eq!(fade_byte(false, 0), Some(0x17dc50 + 0x1b0 + 0x70));
		assert_eq!(fade_byte(true, 1), Some(0x17e0d0 + 0x1b0 + 0x70));
		assert_eq!(fade_byte(false, 99), None);
		assert!(!open_in(99, |_| 5), "no level, no wheel");
		let on = std::cell::Cell::new(0u8);
		let at = fade_byte(super::super::disc::usa(), 3).unwrap();
		assert!(!open_in(3, |a| if a == at { on.get() } else { 9 }));
		on.set(2);
		assert!(open_in(3, |a| if a == at { on.get() } else { 0 }));
	}
}
