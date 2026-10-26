//! Follow poses in the picture (ABI 1.6 `pose_follow`, omni/pose_follow.rs): a prop (the item) is drawn where another moby (the source, the gun in the
//! right hand) sits relative to one hero joint (`from`), but relative to another hero joint (`to`, the left hand), optionally mirrored:
//!
//!     item_draw = W(to) * D * (W(from)^-1 * W(source))        (D = identity without the mirror flag)
//!
//! W are the world matrices of THIS displayed frame: the hero joints as the hero pose draws them (after pose overrides, strafe twist; model space
//! kept per game tick and blended between two ticks like `Smooth::hero` blends the hero pose) in the hero's drawn placement (`smooth::hero_frame`
//! of the final `at` / `tilt`), the source as drawn (the held gun: the hero joint it hangs on; a prop: its drawn matrix). D = diag(1, -1, 1) in
//! the joint's local axes = negating row 1 of `rel` (indices 1, 5, 9, 13 of a column-major 4x4): the convention of pose mirror ops (anim_pose.rs
//! `mirrored`: a mirror-posed left arm is `R * W_right * D`, R = the reflection across the body plane, model Y), so the mirror image of the source gun is
//! exactly `R * W_src = W_left * D * rel`. With the flag the result has determinant -1 (left-handed), which is what the item prop's own mirror flag
//! (moby +0x34 bit 0x8000) makes the draw path expect. Picture only: nothing is written to RAM; the matrix goes into `smooth::PROP_XF`.
//! Pulse squash of the hero's limbs is not followed.
//!
//! `RAC_FOLLOW_LOG=1` (ENV.md): every 120 displayed frames one line with the distance between the item's origin and the `to` joint (min / mean / max)
//! next to the same distance of the source and the `from` joint (the two are equal by construction) and the share of frames that had a solution.
//! `RAC_FOLLOW_ROOT=1`: the held gun is taken at the hero joint times the held anim's root joint (the game's held actor matrix) instead of the joint.

use super::smooth::{blend_place, hero_frame, prop_xf, PROP_XF};
use super::App;
use crate::omni::FollowRec;
use std::cell::RefCell;

type M = [f32; 16];

const IDENTITY: M = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
const LOG_EVERY: u32 = 120;

/// `a * b`, column-major 4x4 (general: the mirrored result is not a rigid placement).
pub(super) fn mul(a: &M, b: &M) -> M {
	let mut o = [0.0; 16];
	for c in 0..4 {
		for r in 0..4 {
			o[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
		}
	}
	o
}

/// A row-major 3x4 (translation at 3, 7, 11) as a column-major 4x4.
pub(super) fn from_rows(m: &[f32; 12]) -> M {
	[m[0], m[4], m[8], 0.0, m[1], m[5], m[9], 0.0, m[2], m[6], m[10], 0.0, m[3], m[7], m[11], 1.0]
}

/// Inverse of a rigid (rotation + translation) column-major placement.
pub(super) fn rigid_inverse(m: &M) -> M {
	let mut o = IDENTITY;
	for c in 0..3 {
		for r in 0..3 {
			o[c * 4 + r] = m[r * 4 + c];
		}
	}
	for r in 0..3 {
		o[12 + r] = -(o[r] * m[12] + o[4 + r] * m[13] + o[8 + r] * m[14]);
	}
	o
}

/// `D * m` with D = diag(1, -1, 1, 1): row 1 negated.
pub(super) fn negate_row1(mut m: M) -> M {
	for i in [1, 5, 9, 13] {
		m[i] = -m[i];
	}
	m
}

/// `item_draw = W(to) * D * (W(from)^-1 * W(source))` (D only with `mirror`).
pub(super) fn follow_matrix(w_to: &M, w_from: &M, w_src: &M, mirror: bool) -> M {
	let rel = mul(&rigid_inverse(w_from), w_src);
	mul(w_to, &if mirror { negate_row1(rel) } else { rel })
}

/// Determinant of the 3x3 part.
pub(super) fn det3(m: &M) -> f32 {
	m[0] * (m[5] * m[10] - m[9] * m[6]) - m[4] * (m[1] * m[10] - m[9] * m[2]) + m[8] * (m[1] * m[6] - m[5] * m[2])
}

/// The model-space matrices one game tick's hero pose gave: the joints the records name, and the held gun (hero joint, and joint times the root).
#[derive(Clone, Default)]
pub(super) struct Snap {
	joints: Vec<(u16, M)>,
	held: Option<(M, M)>,
}

#[derive(Default)]
struct State {
	last: Snap,
	before: Option<Snap>,
	log: Log,
}

#[derive(Default)]
struct Log {
	frames: u32,
	solved: u32,
	gap: (f32, f32, f32),
	reference: f32,
}

thread_local! {
	/// The snapshot `capture` made this frame (a frame that reuses the cached hero bundle makes none), the history, and the log.
	static PENDING: RefCell<Option<Snap>> = const { RefCell::new(None) };
	static STATE: RefCell<State> = RefCell::new(State::default());
}

fn log_on() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_FOLLOW_LOG").is_ok_and(|v| v == "1"))
}

fn root_on() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_FOLLOW_ROOT").is_ok_and(|v| v == "1"))
}

/// Called from the hero skinning block (frame_hero.rs) with the final joint matrices: remembers the model-space matrix of every joint the records
/// name (`world(j)` = the joint's unit-column row-major 3x4). Nothing is kept without records.
pub(super) fn capture(recs: &[FollowRec], joints: usize, world: impl Fn(usize) -> [f32; 12]) {
	let mut wanted: Vec<u16> = recs.iter().flat_map(|r| [r.from, r.to]).filter(|&j| (j as usize) < joints).collect();
	wanted.sort_unstable();
	wanted.dedup();
	let snap = (!wanted.is_empty()).then(|| Snap { joints: wanted.into_iter().map(|j| (j, from_rows(&world(j as usize)))).collect(), held: None });
	PENDING.with(|p| *p.borrow_mut() = snap);
}

/// Called from the held-rig block: the hero joint the held gun hangs on (`hj`, model space) and the held anim's root joint matrix.
pub(super) fn capture_held(hj: &[f32; 12], root: &[f32; 12]) {
	PENDING.with(|p| {
		if let Some(s) = p.borrow_mut().as_mut() {
			let hjm = from_rows(&super::maths::norm_cols(*hj));
			let mut rr = from_rows(root);
			rr[12..15].fill(0.0);
			s.held = Some((hjm, mul(&hjm, &rr)));
		}
	});
}

impl State {
	/// New game-tick snapshot arrives: roll the history exactly like `Smooth::hero` (`before` only when the hero pose blends).
	fn roll(&mut self, now: Option<Snap>, ticks: u32, has_before: bool) {
		let Some(now) = now else { return };
		if ticks > 0 {
			let old = std::mem::replace(&mut self.last, now);
			self.before = has_before.then_some(old);
		} else {
			self.last = now;
		}
	}

	fn joint(&self, j: u16, t: Option<f32>) -> Option<M> {
		let find = |s: &Snap| s.joints.iter().find(|x| x.0 == j).map(|x| x.1);
		let now = find(&self.last)?;
		match (t, self.before.as_ref().and_then(find)) {
			(Some(t), Some(b)) => Some(blend_place(&b, &now, t)),
			_ => Some(now),
		}
	}

	fn held(&self, t: Option<f32>) -> Option<M> {
		let pick = |s: &Snap| s.held.map(|h| if root_on() { h.1 } else { h.0 });
		let now = pick(&self.last)?;
		match (t, self.before.as_ref().and_then(pick)) {
			(Some(t), Some(b)) => Some(blend_place(&b, &now, t)),
			_ => Some(now),
		}
	}
}

fn dist(a: &M, b: &M) -> f32 {
	(0..3).map(|k| (a[12 + k] - b[12 + k]).powi(2)).sum::<f32>().sqrt()
}

impl Log {
	fn add(&mut self, gap: f32, reference: f32) {
		self.solved += 1;
		self.gap = if self.solved == 1 { (gap, gap, gap) } else { (self.gap.0.min(gap), self.gap.1 + gap, self.gap.2.max(gap)) };
		self.reference += reference;
	}

	fn frame(&mut self) {
		self.frames += 1;
		if self.frames >= LOG_EVERY {
			let n = self.solved.max(1) as f32;
			eprintln!(
				"follow: {} frames, {} solved, item-to-joint gap min {:.4} mean {:.4} max {:.4} m, source-to-joint reference mean {:.4} m",
				self.frames,
				self.solved,
				self.gap.0,
				self.gap.1 / n,
				self.gap.2,
				self.reference / n
			);
			*self = Log::default();
		}
	}
}

impl App {
	/// After `smooth.props` (frame_props.rs): puts the matrices of the follow records into `PROP_XF`.
	pub(super) fn apply_follow(&mut self, at: Option<([f32; 3], f32)>, tilt: [f32; 2]) {
		let recs: Vec<FollowRec> = self.retail.as_ref().and(self.retail_parts.as_ref()).map_or(Vec::new(), |q| q.follow.clone());
		let pending = PENDING.with(|p| p.borrow_mut().take());
		if recs.is_empty() {
			STATE.with(|s| *s.borrow_mut() = State::default());
			return;
		}
		let (Some(at), Some(game), Some(retail)) = (at, self.game.as_ref(), self.retail.as_ref()) else { return };
		let blend = (self.smooth.active && self.smooth.hero_before().is_some()).then_some(self.smooth.alpha);
		let hero = hero_frame(at.0, at.1, tilt);
		let held_addr = retail.held_addr();
		let prop_matrix = |pi: usize| {
			let p = &game.props[pi];
			prop_xf(pi).map_or_else(|| super::maths::place_mirrored(p.pos, p.rot, p.mirror), |x| x.0)
		};
		STATE.with(|st| {
			let mut st = st.borrow_mut();
			st.roll(pending, self.smooth.ticks, self.smooth.hero_before().is_some());
			let mut solved = Vec::new();
			for r in &recs {
				let Some(ipi) = retail.prop_of_addr(r.item).filter(|&i| i < game.props.len()) else { continue };
				let (Some(from), Some(to)) = (st.joint(r.from, blend), st.joint(r.to, blend)) else { continue };
				let src = if Some(r.source) == held_addr {
					st.held(blend).map(|m| mul(&hero, &m))
				} else {
					retail.prop_of_addr(r.source).filter(|&i| i < game.props.len()).map(prop_matrix)
				};
				let Some(src) = src else { continue };
				let (w_from, w_to) = (mul(&hero, &from), mul(&hero, &to));
				let mut x = follow_matrix(&w_to, &w_from, &src, r.mirror);
				if !r.mirror && game.props[ipi].mirror {
					x = super::smooth::mirror_column(x, true);
				}
				let scale = prop_xf(ipi).map_or(game.props[ipi].scale, |p| p.1);
				solved.push((ipi, x, scale, dist(&x, &w_to), dist(&src, &w_from)));
			}
			PROP_XF.with(|t| {
				let mut t = t.borrow_mut();
				for s in &solved {
					if t.len() <= s.0 {
						t.resize(s.0 + 1, None);
					}
					t[s.0] = Some((s.1, s.2));
				}
			});
			if log_on() {
				for s in &solved {
					st.log.add(s.3, s.4);
				}
				st.log.frame();
			}
		});
	}
}

#[cfg(test)]
#[path = "follow_tests.rs"]
mod tests;
