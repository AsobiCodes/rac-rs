//! [PC] Standing effect quads blended between fields (`still_kinds`: Pokitaru's pool, Kalebo's sea, waterfall sheets; Split view:
//! frame smoothing of retail's own quads, not an Enhanced effect, so both halves draw the same blended quads). Their
//! corners stand (nearly) still, but retail moves their texture (the pool's scrolling layers), colours and heave once a field,
//! so above 50 fps the whole surface jumped on every field over the smooth Enhanced waves (RAC_TEST_SUB=3, Pokitaru lagoon:
//! water band change 0.9 0.9 2.1 per frame, the field frame twice the others; Classic's own step 1.6 a field). Each standing
//! quad is paired with the quad of last field it is the `same` as (corners in any order, matched by position), and its
//! corners, colours and texture coordinates are drawn between the two. Texture coordinates of a repeating texture blend by
//! their change modulo one texture (retail wraps its scroll); a change of more than `UV_MAX` (after the wrap) is left as it
//! is (a fast scroll can not be told from one going the other way).
use super::{FxMap, STILL_EPS, V3, kind_of};
use crate::particles::{FX_CLAMP, WorldQuad};
use crate::retail::RetailFx;
use std::collections::HashSet;

/// One standing quad: (index in this field's list, index in last field's, last field's corner for each corner of this one).
pub(super) type StillPair = (u32, u32, [u8; 4]);

/// The largest texture coordinate change a field (after the wrap of a repeating texture) that is blended.
const UV_MAX: f32 = 0.25;

/// The fewest quads a standing kind has at the field before for `hold` to carry it.
const HOLD_MIN: usize = 16;

/// [PC] A standing mesh (a kind of `standing`, the newest pairing's `still_kinds`, with at least `HOLD_MIN` quads in `prev`) that
/// has no quad at all in this field's `cur` is carried over: last field's quads of that kind are appended to `cur`, for one field
/// (not again right after a hold, `held_last`). A whole surface missing for one field and back the next (the Kalebo III sea,
/// user clip 2026-10-24: gone for one field while the camera turned fast) is a field the effects layer did not deliver, not
/// the game removing it; drawn steady instead of blinking. Smoothed frames only (the copy in interp.rs). Returns the kinds carried.
pub(super) fn hold(prev: &[WorldQuad], cur: &mut Vec<WorldQuad>, standing: &HashSet<(usize, bool)>, held_last: &[(usize, bool)]) -> Vec<(usize, bool)> {
	if standing.is_empty() {
		return Vec::new();
	}
	let mut gone: Vec<(usize, bool)> = standing.iter().copied().filter(|k| !held_last.contains(k) && prev.iter().filter(|q| kind_of(q) == *k).count() >= HOLD_MIN).collect();
	if gone.is_empty() {
		return gone;
	}
	for q in cur.iter() {
		let k = kind_of(q);
		gone.retain(|&g| g != k);
	}
	if !gone.is_empty() {
		cur.extend(prev.iter().filter(|q| gone.contains(&kind_of(q))).cloned());
	}
	gone
}

/// The quads of `cur` of the standing kinds `still` with their partner in `prev` (`StillPair`). Once per field.
pub(super) fn still_pairs(prev: &[WorldQuad], cur: &[WorldQuad], still: &HashSet<(usize, bool)>) -> Vec<StillPair> {
	if still.is_empty() {
		return Vec::new();
	}
	let cell = |q: &WorldQuad| {
		let m: V3 = std::array::from_fn(|c| (q.pos[0][c] + q.pos[1][c] + q.pos[2][c] + q.pos[3][c]) * 0.25);
		((m[0] / 0.5).floor() as i32, (m[1] / 0.5).floor() as i32, (m[2] / 0.5).floor() as i32)
	};
	let mut grid: FxMap<((usize, bool), (i32, i32, i32)), Vec<u32>> = FxMap::default();
	for (j, q) in prev.iter().enumerate() {
		let k = kind_of(q);
		if still.contains(&k) {
			grid.entry((k, cell(q))).or_default().push(j as u32);
		}
	}
	let close = |a: V3, b: V3| (0..3).all(|c| (a[c] - b[c]).abs() < STILL_EPS);
	let corners = |a: &WorldQuad, b: &WorldQuad| -> Option<[u8; 4]> {
		let mut m = [0u8; 4];
		for (k, &p) in a.pos.iter().enumerate() {
			let d = |q: V3| (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>();
			let best = (0..4).min_by(|&x, &y| d(b.pos[x]).total_cmp(&d(b.pos[y])))?;
			if !close(p, b.pos[best]) {
				return None;
			}
			m[k] = best as u8;
		}
		b.pos.iter().all(|&q| a.pos.iter().any(|&p| close(p, q))).then_some(m)
	};
	let mut out = Vec::new();
	for (i, q) in cur.iter().enumerate() {
		let k = kind_of(q);
		if !still.contains(&k) {
			continue;
		}
		let (cx, cy, cz) = cell(q);
		// The quad's own cell first, then the 26 around it.
		let found = (0..27).map(|n| if n == 0 { (0, 0, 0) } else { let n = if n <= 13 { n - 1 } else { n }; (n / 9 - 1, n / 3 % 3 - 1, n % 3 - 1) }).find_map(|(dx, dy, dz)| {
			grid.get(&(k, (cx + dx, cy + dy, cz + dz)))?.iter().find_map(|&j| corners(q, &prev[j as usize]).map(|m| (j, m)))
		});
		if let Some((j, m)) = found {
			out.push((i as u32, j, m));
		}
	}
	out
}

/// `o` (`blend_fx` of last field `a` and this one) with its standing queue-A / queue-B quads drawn at `t` (`pairs`).
pub(super) fn blend(mut o: RetailFx, a: &RetailFx, t: f32, pairs: &[Vec<StillPair>; 2]) -> RetailFx {
	lerp_still(&a.qa, &mut o.qa, t, &pairs[0]);
	lerp_still(&a.qb, &mut o.qb, t, &pairs[1]);
	o
}

/// `cur` (a copy of this field's quads) drawn at `t` between last field's `prev` (t = 0) and them (t = 1): colours and texture
/// coordinates (the corners move by their welded vertex, `weld::apply`).
pub(super) fn lerp_still(prev: &[WorldQuad], cur: &mut [WorldQuad], t: f32, pairs: &[StillPair]) {
	let s = 1.0 - t;
	for &(i, j, m) in pairs {
		let (Some(a), Some(q)) = (prev.get(j as usize), cur.get_mut(i as usize)) else { continue };
		let wrap = q.tex & FX_CLAMP == 0;
		for k in 0..4 {
			let p = m[k] as usize;
			for c in 0..4 {
				q.colour[k][c] -= (q.colour[k][c] - a.colour[p][c]) * s;
			}
			let d: [f32; 2] = std::array::from_fn(|c| {
				let d = q.uv[k][c] - a.uv[p][c];
				if wrap { d - d.round() } else { d }
			});
			if d[0].abs() <= UV_MAX && d[1].abs() <= UV_MAX {
				q.uv[k][0] -= d[0] * s;
				q.uv[k][1] -= d[1] * s;
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn quad(x: f32, u: f32) -> WorldQuad {
		WorldQuad { pos: [[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x + 1.0, 1.0, 0.0], [x, 1.0, 0.0]], uv: [[u, 0.0], [u + 1.0, 0.0], [u + 1.0, 1.0], [u, 1.0]], colour: [[1.0; 4]; 4], tex: 3, additive: false }
	}

	/// A standing quad whose texture scrolled (and wrapped) is drawn half way at t = 0.5, its corners matched in another order.
	#[test]
	fn scroll_blends_through_the_wrap() {
		let prev = vec![quad(0.0, 0.9)];
		let mut cur = vec![quad(0.0, 0.1)];
		cur[0].pos.rotate_left(1);
		cur[0].uv.rotate_left(1);
		let still: HashSet<_> = [kind_of(&cur[0])].into_iter().collect();
		let pairs = still_pairs(&prev, &cur, &still);
		assert_eq!(pairs, vec![(0, 0, [1, 2, 3, 0])]);
		lerp_still(&prev, &mut cur, 0.5, &pairs);
		assert!((cur[0].uv[3][0] - 0.0).abs() < 1e-5, "{:?}", cur[0].uv);
		lerp_still(&prev, &mut cur, 1.0, &pairs);
	}

	/// A standing kind with no quad in this field is carried once; a kind still present, a small kind and a second field in a row are not.
	#[test]
	fn missing_standing_mesh_is_held_once() {
		let prev: Vec<WorldQuad> = (0..20).map(|i| quad(i as f32, 0.0)).collect();
		let k = kind_of(&prev[0]);
		let still: HashSet<_> = [k].into_iter().collect();
		let mut other = quad(0.0, 0.0);
		other.tex = 9;
		let mut cur = vec![other.clone()];
		assert_eq!(hold(&prev, &mut cur, &still, &[]), vec![k]);
		assert_eq!(cur.len(), 21);
		let mut again = vec![other.clone()];
		assert!(hold(&prev, &mut again, &still, &[k]).is_empty());
		let mut present = vec![quad(3.0, 0.5)];
		assert!(hold(&prev, &mut present, &still, &[]).is_empty());
		assert!(hold(&prev[..8], &mut vec![other], &still, &[]).is_empty());
	}

	/// Quads of a kind that is not standing, or too far from every quad of last field, are left alone.
	#[test]
	fn unmatched_quads_stay() {
		let prev = vec![quad(0.0, 0.0)];
		let cur = vec![quad(5.0, 0.2)];
		let still: HashSet<_> = [kind_of(&cur[0])].into_iter().collect();
		assert!(still_pairs(&prev, &cur, &still).is_empty());
		assert!(still_pairs(&prev, &cur, &HashSet::new()).is_empty());
	}
}
