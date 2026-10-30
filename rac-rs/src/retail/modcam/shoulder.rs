//! Strafe aiming "Shoulder" (v0.4.2, lane aim; Options -> Camera Options -> Strafe aiming: Raised / Shoulder / Off, key
//! `aimcam=shoulder`). The over-the-shoulder aim camera of the Modern camera: while the strafe aim is held the camera eases to a
//! point behind and to the RIGHT of Ratchet, closer than the normal orbit, so he stands left of the centre in the lower third of
//! the frame and the reticle (the screen centre) lies just beside and above his head ("Ratchet & Clank: Rift Apart" framing).
//!
//! How it fits `modcam.rs` `step`: the camera still orbits one pivot (the point at the screen centre). Raised lifts that pivot
//! (`AIM_LIFT`); Shoulder lifts it less (`LIFT`, `lift`) and moves it sideways by `shift` along the camera's right, with a shorter
//! boom. Everything blends in and out with the lift's own `aim_t` (`BLEND_S` seconds, smoothstep), so no pose jumps when the aim
//! starts or ends; switching Raised <-> Shoulder while the aim is held eases through `Sh::mix`. Mouse / right stick turn the yaw and
//! pitch exactly as in Raised (the orbit angles are untouched). Every rate below is in seconds (`dt()`), so PAL and USA agree.
//!
//! Aim: the shifted pivot is on the camera's centre ray, `modcam::aim_cam()` reports it (and the real eye), so the reticle ray,
//! the lock-on search and the shot direction (aim.rs) all follow the shoulder view; `AimCam::shoulder` tells aim.rs the muzzle is
//! off the centre line (strafe_lock then searches toward the point under the reticle, not parallel to the view).
//!
//! Walls: a probe from the unshifted pivot toward the right looks `LOOK` beyond where the shift would end, and four LEAD probes
//! (rays angled `LEAD` metres forward and back at the same reach) see a pillar, doorframe or corner edge coming before it reaches
//! the side line. A lead probe counts walls only (`lead_d`: an enemy ahead on the reticle line must not slide the camera), and the
//! nearest lead contact is held `HOLD_S` (`hold_lead`) so a thin post that each probe sees for a field keeps the pull going. The
//! probes use `seg_clean`: they leave the mobys' query stamps as they found them (`stamps.rs`). The wanted shift is the free room
//! (the nearest of the contacts, projected on the right) minus `MARGIN`,
//! entered through a smooth knee (`soft_min`) so it starts to retreat before the wall is there, and the shift eases to it on a fast
//! spring (`TAU_IN`; growing back `TAU_OUT`). A wall (terrain or a moby's triangle mesh: door, gate, bridge) that the side line
//! itself meets closer than the shifted pivot ends there at once (never a pivot behind a wall: `ease_side`); one that is merely
//! within `HARD` of it pulls the shift in at most `CLAMP_RATE`. A moby primitive (enemy) only moves the wanted shift. The boom from
//! the shifted pivot to the eye runs the normal wall pull-in and rise logic of `step`; the ceiling over Ratchet limits the lift as
//! in Raised (`lift`).
//!
//! Camera Classic: the strafe aim borrows the modern camera while held (`classic.rs`), so Shoulder applies there too, exactly as
//! Raised's lift does; the retail camera outside the aim is untouched. `RAC_SHOULDER=side,dist,lift` overrides the numbers (a dev
//! tuning knob, read in dev builds only: bad entries are ignored, values clamped to `LIMITS`; ENV.md).
use super::*;
use std::sync::OnceLock;

/// Sideways offset of the screen-centre pivot from Ratchet (m, to the camera's right): the larger, the further left of centre he stands.
pub const SIDE: f32 = 1.3;
/// Boom length (m) from the pivot to the eye while aiming (the normal orbit is `DIST` 5.64 m, the wheel zooms 1.5..14).
pub const DIST: f32 = 3.7;
/// Height (m) of the pivot above the orbit pivot (Ratchet's chest, `PIVOT`): Raised lifts 1.5, Shoulder keeps the reticle near his head.
pub const LIFT: f32 = 0.85;
/// Room the wanted shift keeps between the shifted pivot and a wall at his side (m).
const MARGIN: f32 = 0.3;
/// The shifted pivot keeps this far from a wall the side line meets (m): closer than that the shift is pulled in at `CLAMP_RATE`.
const HARD: f32 = 0.15;
/// How fast (m/s) the shift may be pulled in to keep `HARD` from a wall (0.12 m a PAL field); only a wall the shifted pivot is
/// already past ends the shift at once.
const CLAMP_RATE: f32 = 6.0;
/// Forward offsets (m) of the lead probes' end points at the full reach: each is cast both forward and back. A wall edge sweeping
/// in from ahead (walking past it) or from the side (the mouse turning) is met by a lead probe some fields before the side line.
const LEAD: [f32; 2] = [1.0, 2.0];
/// How far beyond the shift the side probe looks, and the width of the knee the wanted shift retreats through (m).
const LOOK: f32 = 0.6;
/// Smoothing time (s) of the shift toward a smaller wanted value (a wall coming) and toward a larger one (the wall let go).
const TAU_IN: f32 = 0.1;
const TAU_OUT: f32 = 0.25;
/// Seconds a lead contact is held after it was last the nearest (`hold_lead`): a post a hand wide is on each lead ray for a field or two.
const HOLD_S: f32 = 0.2;
/// Seconds of the blend in and out (Raised keeps its 12 fields, `AIM_BLEND`).
const BLEND_S: f32 = 0.24;
/// Boom release (m/s) while the aim blends out and the boom has not reached its goal (the normal `RELEASE` is 0.1 m a PAL field).
const RELEASE_OUT: f32 = 15.0;
/// The length of a PAL field (s): the unit the Raised constants were tuned in.
const PAL_DT: f32 = 0.02;
/// Sane ranges of the `RAC_SHOULDER` entries: side (m, negative = camera at his left), boom (m), lift (m).
const LIMITS: [(f32, f32); 3] = [(-2.5, 2.5), (1.5, 8.0), (0.0, 2.0)];

static ON: AtomicBool = AtomicBool::new(false);

/// The Strafe aiming row is Shoulder (`pcext::apply`, `pcinput::init`).
pub fn set(on: bool) {
	ON.store(on, Ordering::Relaxed);
}

pub fn on() -> bool {
	ON.load(Ordering::Relaxed)
}

/// `(side, dist, lift)`: the constants, or the entries of `v` ("side,dist,lift"; a missing, bad or non-finite entry keeps its default,
/// the rest is clamped to `LIMITS`).
fn parse_tune(v: &str) -> [f32; 3] {
	let mut t = [SIDE, DIST, LIFT];
	for ((slot, x), (lo, hi)) in t.iter_mut().zip(v.split(',')).zip(LIMITS) {
		if let Some(f) = x.trim().parse::<f32>().ok().filter(|f| f.is_finite()) {
			*slot = f.clamp(lo, hi);
		}
	}
	t
}

/// The numbers in force: the constants, or in a dev build (not the public `windows_gui` one) the `RAC_SHOULDER` entries.
fn tune() -> [f32; 3] {
	static T: OnceLock<[f32; 3]> = OnceLock::new();
	*T.get_or_init(|| if cfg!(feature = "windows_gui") { [SIDE, DIST, LIFT] } else { parse_tune(&std::env::var("RAC_SHOULDER").unwrap_or_default()) })
}

static HIT_MESH: AtomicBool = AtomicBool::new(false);

/// `seg_mask` notes what its line test hit before it puts the result block back: the moby (`HIT_MOBY`, 0 = terrain or no hit) and
/// whether the record word is a triangle (> 0: terrain, a moby's collision mesh) and not a primitive (< 0: enemy capsules, spheres).
pub(super) fn note_hit(c: &mut Cpu, lv: &Lv, hit: bool) {
	HIT_MOBY.store(if hit { c.mem.rd32(lv.res + 0x18) } else { 0 }, Ordering::Relaxed);
	HIT_MESH.store(hit && (c.mem.rd32(lv.res + 0x1c) as i32) > 0, Ordering::Relaxed);
}

/// What Shoulder holds between fields (in `St`): `mix` (0 = Raised ... 1 = Shoulder, eases when the row changes), the sideways
/// shift in force after the wall test (infinity = not set: the next aim takes the wanted value at once), whether the boom is still
/// growing back fast after a Shoulder aim, the held lead contact (`Hold`), and the pivot shift applied this field (zero outside the aim).
#[derive(Clone, Copy)]
pub(super) struct Sh {
	mix: f32,
	side: f32,
	out: bool,
	hold: Hold,
	pub(super) shift: [f32; 3],
}

impl Sh {
	pub(super) const ZERO: Sh = Sh { mix: 0.0, side: f32::INFINITY, out: false, hold: Hold::NONE, shift: [0.0; 3] };
}

/// The aim ends (hand-back to retail, a seed): no lift, no shift; the next aim starts afresh in the row's mode.
pub(super) fn reset(st: &mut St) {
	(st.aim_t, st.lift, st.lift_cap) = (0, 0.0, AIM_LIFT);
	st.sh = Sh { mix: if on() { 1.0 } else { 0.0 }, ..Sh::ZERO };
}

/// Fields of the blend: Raised keeps `AIM_BLEND`; Shoulder (and the ease between the two) takes `BLEND_S` seconds at this field rate.
fn blend_fields(dt: f32) -> u32 {
	((BLEND_S / dt.max(1e-3)).round() as u32).max(1)
}

/// The two per-field constants of the aim lift: the blend length in fields and the ceiling cap's step (m a field). At `mix` 0 with
/// the row not on Shoulder (Raised / Off) they are `AIM_BLEND` and `AIM_CAP_STEP` exactly, whatever the field rate.
fn raised_params(mix: f32, on: bool, dt: f32) -> (u32, f32) {
	let blend = if mix > 0.0 || on { blend_fields(dt) } else { AIM_BLEND };
	(blend, if mix > 0.0 { AIM_CAP_STEP * dt / PAL_DT } else { AIM_CAP_STEP })
}

fn blend(mix: f32) -> u32 {
	raised_params(mix, on(), dt()).0
}

/// The pivot lift at full aim: Raised `AIM_LIFT` exactly at `mix` 0, `LIFT` at 1.
fn max_lift(mix: f32, lift: f32) -> f32 {
	if mix > 0.0 {
		AIM_LIFT + (lift - AIM_LIFT) * mix
	} else {
		AIM_LIFT
	}
}

/// `aim_t` after one field: up to the blend length while the aim is held, down to 0 after.
fn next_t(aim_t: u32, aiming: bool, blend: u32) -> u32 {
	if aiming {
		(aim_t + 1).min(blend)
	} else {
		aim_t.saturating_sub(1)
	}
}

/// The lift in force (the old inline formula of `step`, unchanged): the smoothstep of `aim_t` up to `max`, limited by the ceiling.
fn lift_value(max: f32, aim_t: u32, blend: u32, cap: f32) -> f32 {
	let t = (aim_t as f32 / blend as f32).min(1.0);
	(max * t * t * (3.0 - 2.0 * t)).min(cap)
}

/// The smoothstep share of the blend `aim_t` of `blend` fields has reached.
fn ease_k(aim_t: u32, blend: u32) -> f32 {
	let t = (aim_t as f32 / blend as f32).min(1.0);
	t * t * (3.0 - 2.0 * t)
}

/// The aim lift in and out (moved from `step`, same maths for Raised): `aim_t` runs 0..=blend while the aim is held, the lift
/// follows it smoothly up to `max_lift` and is limited by the ceiling over Ratchet (`lift_cap`, which moves at most
/// `AIM_CAP_STEP` a PAL field).
pub(super) fn lift(c: &mut Cpu, lv: &Lv, st: &mut St, flat: bool) {
	if !flat {
		reset(st);
	}
	let target = if on() { 1.0 } else { 0.0 };
	let step = 1.0 / blend(st.sh.mix) as f32;
	st.sh.mix += (target - st.sh.mix).clamp(-step, step);
	let ((b, cap_step), max) = (raised_params(st.sh.mix, on(), dt()), max_lift(st.sh.mix, tune()[2]));
	st.aim_t = next_t(st.aim_t, st.aiming, b);
	if st.aim_t > 0 {
		let up = [st.dy.pivc[0], st.dy.pivc[1], st.dy.pivc[2] + max + AIM_CEIL];
		let goal = match seg(c, lv, st.dy.pivc, up) {
			Some(h) => (h[2] - st.dy.pivc[2] - AIM_CEIL).clamp(0.0, max),
			None => max,
		};
		st.lift_cap += (goal - st.lift_cap).clamp(-cap_step, cap_step);
		st.lift = lift_value(max, st.aim_t, b, st.lift_cap);
	} else {
		st.lift = 0.0;
	}
}

/// Smooth minimum of `a` and `b` over a knee of width `w`: equals the smaller one when they differ by `w` or more, dips at most
/// `w / 4` below it where they meet, slope at most 1 in either argument.
fn soft_min(a: f32, b: f32, w: f32) -> f32 {
	let h = (w - (a - b).abs()).max(0.0) / w;
	a.min(b) - h * h * w * 0.25
}

/// What the side line (pivot -> shifted pivot and `LOOK` beyond) first meets: `d` its distance along the shift direction, `hard` a
/// wall (terrain or a moby's triangle mesh: door, gate, bridge) and not an enemy / crate primitive that moves.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Contact {
	d: f32,
	hard: bool,
}

/// The spring and clamp rates of one field.
#[derive(Clone, Copy)]
struct Rates {
	/// Share of the gap a shrinking / growing shift closes in a field.
	k_in: f32,
	k_out: f32,
	/// Most (m) a field the shift is pulled in to keep `HARD` from a wall.
	clamp: f32,
}

fn rates() -> Rates {
	Rates { k_in: k_of(TAU_IN), k_out: k_of(TAU_OUT), clamp: CLAMP_RATE * dt() }
}

/// One field of the sideways shift: `(shift, snapped)`. `cur` is the shift in force (infinity = not set), `side` the full shift,
/// `main` the first contact of the side line, `lead` the distances (along the shift direction) of the lead probes' contacts
/// (non-finite = none). The wanted shift is the free room to the NEAREST of them less `MARGIN`, through the knee; `cur` eases to it
/// with `k_in` (shrinking) or `k_out` (growing). A wall on the side line itself keeps `HARD` from the shifted pivot: closer than
/// that `cur` is pulled in at most `clamp` a field; only when the shifted pivot is already PAST the wall (`d < cur`) does `cur`
/// end at `d - HARD` at once (`snapped`).
fn ease_side(cur: f32, side: f32, main: Option<Contact>, lead: &[f32], r: Rates) -> (f32, bool) {
	let main = main.filter(|h| h.d.is_finite());
	let free = |d: f32| soft_min(d - MARGIN, side, LOOK).clamp(0.0, side);
	let near = main.iter().map(|h| h.d).chain(lead.iter().copied().filter(|d| d.is_finite()));
	let want = near.fold(side, |w, d| w.min(free(d)));
	let cur = if cur.is_finite() { cur.min(side) } else { want };
	let cur = cur + (want - cur) * if want < cur { r.k_in } else { r.k_out };
	let Some(wall) = main.filter(|h| h.hard) else { return (cur, false) };
	let room = (wall.d - HARD).max(0.0);
	if wall.d < cur {
		(room, true)
	} else {
		(if cur > room { (cur - r.clamp).max(room) } else { cur }, false)
	}
}

/// The boom goal at blend share `w`: the normal goal `g`, but its orbit part (`g` less the Smooth lock-on framing pull-back
/// `frame_len`) capped at `dist`; the framing boom stays.
fn goal(g: f32, frame_len: f32, dist: f32, w: f32) -> f32 {
	let base = (g - frame_len).max(0.0);
	g + (base.min(dist) - base) * w
}

/// The shoulder pivot and boom goal for this field, given the pivot `step` would orbit (after the lift) and the normal boom goal
/// `goal_len`. Raised / Off (or no aim in progress): both unchanged and the shift reset.
/// No shoulder this field (Raised / Off, or the aim is not in progress): `apply` hands the pivot and the boom goal back untouched.
fn inert(mix: f32, aim_t: u32) -> bool {
	mix <= 0.0 || aim_t == 0
}

/// `Sh::out` while inert: only a Shoulder blend-out (mix > 0) that has not yet got the boom back to its goal / last safe length.
fn idle_out(mix: f32, out: bool, len: f32, goal_len: f32, safe: f32) -> bool {
	mix > 0.0 && out && len + 1e-3 < goal_len.min(safe)
}

/// A lead probe's distance: only a wall (`hard`) counts. An enemy or crate primitive on a lead ray (it crosses the reticle line
/// about 1.2 m ahead) must not slide the camera off the target he is aiming at; the side line still sees it softly.
fn lead_d(contact: Option<Contact>) -> f32 {
	contact.filter(|h| h.hard).map_or(f32::INFINITY, |h| h.d)
}

/// The nearest lead contact of the last `HOLD_S` seconds (`d`, infinity = none) and the fields left before a farther one may replace it.
#[derive(Clone, Copy)]
struct Hold {
	d: f32,
	left: u32,
}

impl Hold {
	const NONE: Hold = Hold { d: f32::INFINITY, left: 0 };
}

/// Fields of the lead hold at this field rate (10 PAL, 12 USA).
fn hold_fields(dt: f32) -> u32 {
	((HOLD_S / dt.max(1e-3)).round() as u32).max(1)
}

/// The lead distance the wanted shift follows: the nearest of the lead probes now, held `fields` after it was last the nearest, so
/// a thin post that each probe sees for only a field or two keeps pulling the shift in through the gaps between the probes
/// (a closer or equal contact takes over at once; a farther one only after the hold ran out; no contact at all lets go after it).
fn hold_lead(h: &mut Hold, lead: &[f32], fields: u32) -> f32 {
	let now = lead.iter().copied().filter(|d| d.is_finite()).fold(f32::INFINITY, f32::min);
	if now <= h.d || h.left == 0 {
		*h = Hold { d: now, left: fields };
	} else {
		h.left -= 1;
	}
	h.d
}

/// The side line's first contact (None: free to the end) and the lead probes' contacts projected on `right` (infinity = none).
/// The probes leave the mobys' query stamps as they found them (`seg_clean`), so retail's next collision query is not blind to
/// the enemies and crates they touched.
fn probes(c: &mut Cpu, lv: &Lv, pivot: [f32; 3], right: [f32; 3], fwd: [f32; 3], reach: f32) -> (Option<Contact>, [f32; 2 * LEAD.len()]) {
	let mut cast = |off: f32| {
		let h = seg_clean(c, lv, pivot, add3(add3(pivot, scale3(right, reach)), scale3(fwd, off)))?;
		let hard = HIT_MOBY.load(Ordering::Relaxed) == 0 || HIT_MESH.load(Ordering::Relaxed);
		Some(Contact { d: dot3(sub(h, pivot), right), hard })
	};
	let main = cast(0.0);
	let mut lead = [f32::INFINITY; 2 * LEAD.len()];
	for (slot, off) in lead.iter_mut().zip(LEAD.iter().flat_map(|&l| [l, -l])) {
		*slot = lead_d(cast(off));
	}
	(main, lead)
}

pub(super) fn apply(c: &mut Cpu, lv: &Lv, st: &mut St, fr: &Frame, pivot: [f32; 3], goal_len: f32, dynamic: bool) -> ([f32; 3], f32) {
	let mix = st.sh.mix;
	if inert(mix, st.aim_t) {
		let out = idle_out(mix, st.sh.out, st.len, goal_len, st.tr[1]);
		st.sh = Sh { mix, out, ..Sh::ZERO };
		return (pivot, goal_len);
	}
	let [side, dist, _] = tune();
	// A negative side (RAC_SHOULDER only) puts the camera at Ratchet's left; the wall test follows the same direction.
	let rows = fr.rows(st.yaw, 0.0);
	let right = scale3(rows[1], -side.signum());
	let side = side.abs();
	let (main, lead) = probes(c, lv, pivot, right, rows[0], side + MARGIN + LOOK);
	let held = hold_lead(&mut st.sh.hold, &lead, hold_fields(dt()));
	let (shift, snapped) = ease_side(st.sh.side, side, main, &[held], rates());
	if log_on() {
		eprintln!("shoulder-log f={} side={shift:.4} main={main:?} lead={lead:.3?} snapped={} held={held:.3}", st.field, snapped as u8);
	}
	st.sh.side = shift;
	let w = ease_k(st.aim_t, blend(mix)) * mix;
	st.sh.shift = scale3(right, st.sh.side * w);
	st.sh.out = true;
	let frame_len = if dynamic { st.dy.frame_len } else { 0.0 };
	(add3(pivot, st.sh.shift), goal(goal_len, frame_len, dist, w))
}

/// How far the boom may grow back in a field (m): `RELEASE`, faster after a Shoulder aim until the boom has reached its goal.
pub(super) fn release(st: &St) -> f32 {
	release_rate(st.sh.out, st.aiming, dt())
}

fn release_rate(out: bool, aiming: bool, dt: f32) -> f32 {
	if out && !aiming {
		(RELEASE_OUT * dt).max(RELEASE)
	} else {
		RELEASE
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::fmath::Fm;

	/// The PAL field's rates (k_of(TAU_IN), k_of(TAU_OUT), CLAMP_RATE * 0.02).
	const PAL: Rates = Rates { k_in: 0.181, k_out: 0.077, clamp: 0.12 };
	const USA_DT: f32 = 1001.0 / 60000.0;
	const REACH: f32 = SIDE + MARGIN + LOOK;
	const NO_LEAD: [f32; 2 * LEAD.len()] = [f32::INFINITY; 2 * LEAD.len()];

	fn wall(d: f32, hard: bool) -> Option<Contact> {
		Some(Contact { d, hard })
	}

	/// `ease_side` without the snapped flag.
	fn ease(cur: f32, main: Option<Contact>, lead: &[f32]) -> f32 {
		ease_side(cur, SIDE, main, lead, PAL).0
	}

	#[test]
	fn the_numbers_frame_ratchet_left_of_centre() {
		// Ratchet's sideways angle from the view axis is atan(SIDE / DIST) (no std trig here: the tan is checked): 11..25 degrees,
		// i.e. well left of centre but in frame.
		let tan = SIDE / DIST;
		assert!((0.19..0.47).contains(&tan), "{tan}");
		assert!(LIFT < AIM_LIFT && DIST < super::super::DIST && MARGIN < SIDE && HARD < MARGIN);
	}

	#[test]
	fn the_tuning_knob_rejects_bad_values_and_clamps() {
		assert_eq!(parse_tune(""), [SIDE, DIST, LIFT]);
		assert_eq!(parse_tune("1.0,3.0,0.5"), [1.0, 3.0, 0.5]);
		assert_eq!(parse_tune("nan,inf,-inf"), [SIDE, DIST, LIFT]);
		assert_eq!(parse_tune("abc, ,0.7"), [SIDE, DIST, 0.7]);
		assert_eq!(parse_tune("99,99,99"), [2.5, 8.0, 2.0]);
		assert_eq!(parse_tune("-99,-99,-99"), [-2.5, 1.5, 0.0]);
		assert_eq!(parse_tune("-1.0"), [-1.0, DIST, LIFT]);
	}

	#[test]
	fn raised_lift_is_the_old_formula() {
		// Mix 0 (Raised / Off): the lift maximum, the blend length and the smoothstep are what `step` had inline.
		assert_eq!(max_lift(0.0, LIFT).to_bits(), AIM_LIFT.to_bits());
		for aim_t in 0..=AIM_BLEND {
			let t = aim_t as f32 / AIM_BLEND as f32;
			let old = (AIM_LIFT * t * t * (3.0 - 2.0 * t)).min(AIM_LIFT);
			assert_eq!(lift_value(AIM_LIFT, aim_t, AIM_BLEND, AIM_LIFT).to_bits(), old.to_bits(), "aim_t {aim_t}");
		}
		let mut t = 0;
		for _ in 0..40 {
			t = next_t(t, true, AIM_BLEND);
		}
		assert_eq!(t, AIM_BLEND);
		assert_eq!(next_t(t, false, AIM_BLEND), AIM_BLEND - 1);
		assert_eq!(next_t(0, false, AIM_BLEND), 0);
		// Shoulder at mix 1 lifts `LIFT`; halfway blends.
		assert_eq!(max_lift(1.0, LIFT), LIFT);
		assert!((max_lift(0.5, LIFT) - 0.5 * (AIM_LIFT + LIFT)).abs() < 1e-6);
	}

	#[test]
	fn the_blend_takes_the_same_time_on_both_discs() {
		assert_eq!(blend_fields(0.02), 12);
		let n = blend_fields(USA_DT);
		assert!((n as f32 * USA_DT - BLEND_S).abs() < USA_DT, "{n} USA fields");
		assert!(blend_fields(0.0) >= 1);
	}

	#[test]
	fn raised_and_off_keep_the_old_per_field_constants_on_both_discs() {
		// The branch `lift` takes at mix 0 with the row not on Shoulder: the retail-era constants, bit for bit, PAL and USA.
		for dt in [0.02, USA_DT] {
			let (b, cap) = raised_params(0.0, false, dt);
			assert_eq!((b, cap.to_bits()), (AIM_BLEND, AIM_CAP_STEP.to_bits()), "dt {dt}");
		}
		// The row on Shoulder but mix still 0 (the first field): the Shoulder blend, the old cap step.
		let (b, cap) = raised_params(0.0, true, USA_DT);
		assert_eq!((b, cap.to_bits()), (14, AIM_CAP_STEP.to_bits()));
		// Mix above 0 (Shoulder, or easing out to Raised): the blend in seconds and the cap step scaled to the field.
		let (b, cap) = raised_params(0.4, false, USA_DT);
		assert_eq!(b, 14);
		assert!((cap - AIM_CAP_STEP * USA_DT / PAL_DT).abs() < 1e-7);
		assert_eq!(raised_params(1.0, true, 0.02).0, AIM_BLEND);
	}

	#[test]
	fn raised_and_off_leave_the_camera_alone() {
		// `apply` returns the pivot and the boom goal untouched while mix is 0, whatever aim_t is; it stays out of the way of `out`.
		for aim_t in 0..=AIM_BLEND {
			assert!(inert(0.0, aim_t), "aim_t {aim_t}");
			assert!(!idle_out(0.0, true, 0.0, 5.0, 5.0));
		}
		assert!(inert(0.7, 0) && !inert(0.7, 1) && !inert(1.0, AIM_BLEND));
		// A Shoulder session that ended keeps the fast release until the boom is back; Raised never does.
		assert!(idle_out(0.3, true, 2.0, 5.0, 5.0) && !idle_out(0.3, true, 5.0, 5.0, 5.0) && !idle_out(0.3, false, 2.0, 5.0, 5.0));
		// `release` of a Raised aim (Sh::ZERO.out false) is the old fixed step on both discs, held aim or not.
		for dt in [0.02, USA_DT] {
			for aiming in [false, true] {
				assert_eq!(release_rate(Sh::ZERO.out, aiming, dt).to_bits(), RELEASE.to_bits());
			}
		}
		assert_eq!(RELEASE.to_bits(), 0.1f32.to_bits());
	}

	#[test]
	fn the_boom_caps_to_the_shoulder_distance_and_keeps_the_framing_pull_back() {
		assert!((goal(5.64, 0.0, DIST, 1.0) - DIST).abs() < 1e-6);
		// Smooth, far lock-on: the orbit part (5.64) is capped, the 1.46 m framing boom stays.
		assert!((goal(5.64 + 1.46, 1.46, DIST, 1.0) - (DIST + 1.46)).abs() < 1e-5);
		// A boom already closer than the shoulder distance is not pushed out.
		assert!((goal(2.5, 0.0, DIST, 1.0) - 2.5).abs() < 1e-6);
		// Halfway through the blend: halfway.
		assert!((goal(5.64, 0.0, DIST, 0.5) - 0.5 * (5.64 + DIST)).abs() < 1e-5);
	}

	#[test]
	fn a_fresh_state_takes_the_wanted_value_at_once() {
		assert_eq!(ease(f32::INFINITY, None, &NO_LEAD), SIDE);
		let want = ease(f32::INFINITY, wall(1.0, false), &NO_LEAD);
		assert!((want - soft_min(1.0 - MARGIN, SIDE, LOOK)).abs() < 1e-6 && want < SIDE);
		assert!(Sh::ZERO.side.is_infinite() && Sh::ZERO.shift == [0.0; 3] && Sh::ZERO.mix == 0.0 && !Sh::ZERO.out);
	}

	#[test]
	fn a_wall_beyond_the_shift_only_eases_the_shift_in() {
		// The side probe finds a wall past the shifted pivot (SIDE + HARD): the shift moves by a share of the gap, never by it all.
		for d in [1.5f32, 1.6, 1.8, 2.0, 2.2, 3.0] {
			let next = ease(SIDE, wall(d, true), &NO_LEAD);
			assert!(SIDE - next <= 0.12, "wall at {d}: {SIDE} -> {next}");
			assert!(next <= SIDE && next >= 0.0);
		}
		// Frame after frame the shift settles on the wanted value: free room minus the margin.
		let mut s = SIDE;
		for _ in 0..200 {
			s = ease(s, wall(1.0, true), &NO_LEAD);
		}
		assert!((s - soft_min(1.0 - MARGIN, SIDE, LOOK)).abs() < 1e-3 && (0.45..=0.75).contains(&s), "{s}");
		// The wall lets go: it grows back gradually.
		let next = ease(s, None, &NO_LEAD);
		assert!(next > s && next - s <= (SIDE - s) * PAL.k_out + 1e-6);
	}

	#[test]
	fn a_wall_the_shifted_pivot_is_past_ends_the_shift_at_once_but_an_enemy_does_not() {
		// A terrain wall 0.5 m away with the shift at 1.3: the shifted pivot would be behind it, the shift stops HARD short of it.
		let (next, snapped) = ease_side(SIDE, SIDE, wall(0.5, true), &NO_LEAD, PAL);
		assert!(snapped && (next - (0.5 - HARD)).abs() < 1e-6, "{next}");
		// A wall at the pivot itself: no shift, never negative.
		assert_eq!(ease_side(SIDE, SIDE, wall(0.0, true), &NO_LEAD, PAL), (0.0, true));
		// A moby's triangle mesh (door, gate) is a wall like terrain: `hard` is set by the probe from the record word.
		assert_eq!(ease_side(SIDE, SIDE, wall(0.5, true), &NO_LEAD, PAL), (0.5 - HARD, true));
		// An enemy / crate primitive at the same place only pulls the wanted value (eased).
		let (moby, snapped) = ease_side(SIDE, SIDE, wall(0.5, false), &NO_LEAD, PAL);
		assert!(!snapped && SIDE - moby <= (SIDE - 0.2) * PAL.k_in + 1e-6 && moby > next);
		// Non-finite contacts are ignored.
		assert_eq!(ease_side(SIDE, SIDE, wall(f32::NAN, true), &[f32::NAN; 4], PAL), (SIDE, false));
	}

	#[test]
	fn a_wall_within_the_margin_is_not_snapped_to_but_pulled_to_at_the_clamp_rate() {
		// The wall is beyond the shifted pivot (d > cur) but closer than HARD to it: the shift moves at most `clamp` a field.
		let (next, snapped) = ease_side(SIDE, SIDE, wall(1.4, true), &NO_LEAD, PAL);
		assert!(!snapped && next <= SIDE && SIDE - next <= PAL.clamp + 1e-6 && next >= 1.4 - HARD - 1e-6, "{next}");
		// A lead contact alone never clamps (it is not on the side line): it only lowers the wanted shift, eased.
		let (next, snapped) = ease_side(SIDE, SIDE, None, &[0.2, f32::INFINITY, f32::INFINITY, f32::INFINITY], PAL);
		assert!(!snapped && (SIDE - next - (SIDE - 0.0) * PAL.k_in).abs() < 1e-6, "{next}");
	}

	#[test]
	fn the_shift_follows_an_approaching_wall_without_jumps() {
		// A wall on the side line coming closer by `speed` m a field (strafing into it): the shift, spring and hard clamp together,
		// moves at most 0.15 m a field and the shifted pivot is never past the wall.
		for speed in [0.01f32, 0.03, 0.06, 0.1] {
			let (mut cur, mut d) = (f32::INFINITY, 4.0f32);
			while d > 0.3 {
				d -= speed;
				let (next, snapped) = ease_side(cur, SIDE, wall(d, true), &NO_LEAD, PAL);
				assert!(!snapped, "speed {speed}, wall at {d}: snapped to {next} from {cur}");
				assert!(next <= d + 1e-6, "past the wall: {next} at {d}");
				if cur.is_finite() {
					assert!((cur - next).abs() <= 0.15, "speed {speed}, wall at {d}: {cur} -> {next}");
				}
				cur = next;
			}
		}
		assert_eq!(ease(SIDE, wall(0.1, true), &NO_LEAD), 0.0);
	}

	/// Segment from the pivot (0, 0) to `e` against the box `[x0, x1, y0, y1]` (x to the right, y ahead): the hit fraction.
	fn ray_box(e: [f32; 2], b: [f32; 4]) -> Option<f32> {
		let (mut t0, mut t1) = (0.0f32, 1.0f32);
		for (d, lo, hi) in [(e[0], b[0], b[1]), (e[1], b[2], b[3])] {
			if d.abs() < 1e-9 {
				if lo > 0.0 || hi < 0.0 {
					return None;
				}
				continue;
			}
			let (a, z) = (lo / d, hi / d);
			(t0, t1) = (t0.max(a.min(z)), t1.min(a.max(z)));
			if t0 > t1 {
				return None;
			}
		}
		Some(t0)
	}

	/// What `probes` finds for the box `b` (a pillar, in world axes, pivot at the origin) with the camera turned by `rot`.
	fn cast_box(rot: f32, b: [f32; 4]) -> (Option<Contact>, [f32; 2 * LEAD.len()]) {
		let (s, c) = rot.fsin_cos();
		let proj = |off: f32| {
			let e = [REACH * c - off * s, REACH * s + off * c];
			ray_box(e, b).map(|t| t * (e[0] * c + e[1] * s))
		};
		let main = proj(0.0).map(|d| Contact { d, hard: true });
		let mut lead = NO_LEAD;
		for (slot, off) in lead.iter_mut().zip(LEAD.iter().flat_map(|&l| [l, -l])) {
			*slot = proj(off).unwrap_or(f32::INFINITY);
		}
		(main, lead)
	}

	/// Which probes `sweep` feeds `ease_side`: the side line alone (first fix round), plus the raw lead probes (round 2b), plus the
	/// lead probes held `HOLD_S` (what `apply` does).
	#[derive(Clone, Copy, PartialEq)]
	enum Lead {
		Off,
		Raw,
		Held,
	}

	/// Runs a pillar through the probes (`pose(f)` gives the camera turn and the box at field `f`): the largest one-field change of
	/// the shift and the fields where it snapped.
	fn sweep(pose: impl Fn(i32) -> (f32, [f32; 4]), leads: Lead) -> (f32, Vec<i32>) {
		let (mut cur, mut worst, mut snaps, mut hold) = (f32::INFINITY, 0.0f32, Vec::new(), Hold::NONE);
		for f in -60..120 {
			let (rot, b) = pose(f);
			let (main, lead) = cast_box(rot, b);
			let held = [hold_lead(&mut hold, &lead, 10)];
			let lead: &[f32] = match leads {
				Lead::Off => &NO_LEAD,
				Lead::Raw => &lead,
				Lead::Held => &held,
			};
			let (next, snapped) = ease_side(cur, SIDE, main, lead, PAL);
			if cur.is_finite() {
				worst = worst.max((next - cur).abs());
			}
			if snapped {
				snaps.push(f);
			}
			cur = next;
		}
		(worst, snaps)
	}

	#[test]
	fn walking_past_a_pillar_edge_never_snaps_the_shift() {
		// A 1 m pillar `xp` to the right whose front face comes toward Ratchet at `v` m a field (running is about 0.12-0.16).
		for xp in [0.8f32, 1.0, 1.2, 1.4] {
			for v in [0.08f32, 0.12, 0.16] {
				let (worst, snaps) = sweep(|f| (0.0, [xp, xp + 1.0, 3.0 - v * f as f32, 4.0 - v * f as f32]), Lead::Held);
				assert!(worst <= 0.15 && snaps.is_empty(), "pillar at {xp}, {v} m a field: worst {worst}, snaps {snaps:?}");
			}
			// The side line alone (no lead probes) snaps the shift to the wall: the pop this fix removes.
			let (alone, snaps) = sweep(|f| (0.0, [xp, xp + 1.0, 3.0 - 0.12 * f as f32, 4.0 - 0.12 * f as f32]), Lead::Off);
			if xp <= 1.0 {
				assert!(alone > 0.3 && !snaps.is_empty(), "pillar at {xp}: worst {alone}");
			}
		}
	}

	#[test]
	fn turning_the_camera_past_a_pillar_edge_never_snaps_the_shift() {
		// A fixed 1 m pillar ahead and to the right; the camera yaws toward it by `turn` rad a field (0.15 = 8.6 degrees, a hard flick).
		for xp in [0.8f32, 1.0, 1.2, 1.4] {
			for turn in [0.03f32, 0.06, 0.1, 0.15] {
				let (worst, snaps) = sweep(|f| (turn * f as f32, [xp, xp + 1.0, 0.5, 1.5]), Lead::Held);
				assert!(worst <= 0.15 && snaps.is_empty(), "pillar at {xp}, turn {turn}: worst {worst}, snaps {snaps:?}");
			}
		}
		// Very close (0.5-0.6 m) and fast: the lead probes still cut the pop to a bounded step; a snap, if any, is the penetration case.
		for xp in [0.5f32, 0.6] {
			let (worst, snaps) = sweep(|f| (0.0, [xp, xp + 1.0, 3.0 - 0.16 * f as f32, 4.0 - 0.16 * f as f32]), Lead::Held);
			assert!(worst <= 0.25 && snaps.len() <= 2, "pillar at {xp}: worst {worst}, snaps {snaps:?}");
		}
	}

	#[test]
	fn a_thin_post_walked_past_pulls_the_shift_in_without_a_pop() {
		// A post 0.1 m wide and deep, `xp` m to the right, walked past at `v` m a field: each lead probe sees it for a field or two, and
		// in the gaps between the probes the shift used to grow back and then snap to the post. The hold keeps the pull going.
		let post = |xp: f32, v: f32| move |f: i32| (0.0, [xp, xp + 0.1, 3.0 - v * f as f32, 3.1 - v * f as f32]);
		for xp in [0.8f32, 0.9, 1.0, 1.2] {
			for v in [0.08f32, 0.12, 0.16] {
				// The floor of the pull is the spring's first step toward a wall that appears 0.8 m away: (SIDE + MARGIN - 0.8) * k_in = 0.15.
				let (worst, snaps) = sweep(post(xp, v), Lead::Held);
				assert!(worst <= if xp < 0.9 { 0.16 } else { 0.15 } && snaps.is_empty(), "post at {xp}, {v} m a field: worst {worst}, snaps {snaps:?}");
				if xp <= 0.9 {
					let (raw, snaps) = sweep(post(xp, v), Lead::Raw);
					assert!(raw > 0.25 && !snaps.is_empty(), "without the hold, post at {xp}, {v} m a field: worst {raw}, snaps {snaps:?}");
				}
			}
		}
		// Closer than that a fast walker (0.16 m a field = 8 m/s) outruns the spring and meets the penetration clamp once; a slow one does not.
		for v in [0.08f32, 0.12] {
			let (worst, snaps) = sweep(post(0.7, v), Lead::Held);
			assert!(worst <= 0.16 && snaps.is_empty(), "post at 0.7, {v} m a field: worst {worst}, snaps {snaps:?}");
		}
	}

	#[test]
	fn the_lead_hold_keeps_the_nearest_contact_for_a_fifth_of_a_second() {
		let (inf, mut h) = (f32::INFINITY, Hold::NONE);
		assert_eq!(hold_lead(&mut h, &[inf; 4], 10), inf);
		// A contact takes over at once; no contact (or NaN) is ignored for the 10 fields, then the hold lets go.
		assert_eq!(hold_lead(&mut h, &[inf, 0.8, f32::NAN, 1.1], 10), 0.8);
		for _ in 0..10 {
			assert_eq!(hold_lead(&mut h, &[inf, f32::NAN], 10), 0.8);
		}
		assert_eq!(hold_lead(&mut h, &[inf; 4], 10), inf);
		// A closer contact replaces a held one at once and restarts the hold; a farther one waits for it to run out.
		assert_eq!(hold_lead(&mut h, &[0.8], 10), 0.8);
		assert_eq!(hold_lead(&mut h, &[0.6], 10), 0.6);
		for _ in 0..10 {
			assert_eq!(hold_lead(&mut h, &[0.9], 10), 0.6);
		}
		assert_eq!(hold_lead(&mut h, &[0.9], 10), 0.9);
		assert_eq!((hold_fields(0.02), hold_fields(USA_DT), hold_fields(0.0) >= 1), (10, 12, true));
	}

	#[test]
	fn a_lead_probe_counts_walls_only() {
		let contact = |hard| Some(Contact { d: 1.2, hard });
		assert_eq!([lead_d(contact(true)), lead_d(contact(false)), lead_d(None)], [1.2, f32::INFINITY, f32::INFINITY]);
	}

	#[test]
	fn the_boom_releases_at_the_old_rate_unless_a_shoulder_aim_is_blending_out() {
		assert_eq!(release_rate(false, false, 0.02).to_bits(), RELEASE.to_bits());
		assert_eq!(release_rate(true, true, 0.02).to_bits(), RELEASE.to_bits(), "held aim: the normal release");
		assert!((release_rate(true, false, 0.02) - 0.3).abs() < 1e-6);
		// The same speed in m/s on USA (59.94 fields/s): a smaller step a field.
		assert!(release_rate(true, false, USA_DT) < release_rate(true, false, 0.02));
	}
}
