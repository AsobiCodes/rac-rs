//! [PC] Fire: the tongues of the level's fire quads (the `Ground` rows, firefx.rs `gather`; review A2 / A16 / A24).
//! Each retail flame quad (a slot of a class 760 emitter, rac1/fire.rs: a camera-facing billboard standing on its slot's point, with
//! its own size for its whole life and its own fade in / hold / fade out) becomes ONE tongue standing on that point, as tall as the
//! billboard and with the billboard's own fade (lane nfire, agent 6: the old merge of the quads of one fire into one tongue took the
//! anchor, the size and the fade from whichever slot sorted first or was biggest, so every re-roll of a slot, several a second per
//! building, re-formed the whole fire: "the fire still flickers af"). Only quads that stand on the same point with the same size are one
//! tongue (`Tongue::members`; the quad pass leaves exactly those out, `FireFx::skips`). A quad that is flat, narrow, beyond the fire
//! distance or over the tongue cap is not a member and draws as the retail billboard; over the cap the NEAREST quads become tongues,
//! whatever order the game submitted them in. The noise seed and scale follow a tongue from frame to frame (firefx_ground_track.rs).

use super::firefx::{dist, tune, Emit};
use super::firefx_grid::{dist2, Grid};
use crate::prims::WorldQuad;

/// One drawn tongue: the emitter (also the embers' and the lights'), the tint and fade of its quad, the effect quads it replaces and
/// its noise scale and scroll rate (firefx_vol.rs `noise_of`, kept for the tongue's life by firefx_ground_track.rs; 0 = from its size).
#[derive(Clone, Debug)]
pub(super) struct Tongue {
	pub emit: Emit,
	pub tint: [f32; 3],
	pub fade: f32,
	pub members: Vec<usize>,
	pub noise: [f32; 2],
}

/// One candidate: index of the effect quad, its emitter, tint, fade (`ground_of`).
pub(super) type Item = (usize, Emit, [f32; 3], f32);

pub(super) fn hash3(p: [f32; 3]) -> f32 {
	let h = (p[0] as i32 as u32).wrapping_mul(73856093) ^ (p[1] as i32 as u32).wrapping_mul(19349663) ^ (p[2] as i32 as u32).wrapping_mul(83492791);
	let h = (h ^ (h >> 13)).wrapping_mul(1274126177);
	((h ^ (h >> 16)) & 0xffff) as f32 / 65536.0
}

/// A quad leaning further from upright than this (its vertical extent over its side, cos 60 degrees) is lying down, not a flame.
const UPRIGHT: f32 = 0.5;

/// Quads whose feet are this close (m) and whose heights differ by less than `SAME_H` (fraction) are one flame drawn twice.
const SAME_FOOT: f32 = 0.15;
pub(super) const SAME_H: f32 = 0.1;

/// The foot of a fire on the grid the seed is hashed on (half a metre across, a metre up).
pub(super) fn foot_key(base: [f32; 3]) -> [i32; 3] {
	[(base[0] * 2.0).round() as i32, (base[1] * 2.0).round() as i32, base[2].round() as i32]
}

/// The seed of a tongue standing at `foot` with no tongue to follow (firefx_ground_track.rs).
pub(super) fn seed_at(foot: [f32; 3]) -> f32 {
	let k = foot_key(foot);
	hash3([k[0] as f32, k[1] as f32, k[2] as f32])
}

/// A triangle passed as a quad (its last corner repeated): the effects layer's mirror of a fire the game also draws as billboard quads
/// (rac1: the class 760 draw rebuilt from its GS packets, retail/efx_draw.rs, beside rac1/fire.rs's quads of the same emitters). It has no
/// bottom edge: `ground_of` took a bottom and a top corner for its foot (halfway up the flame, flipping between corners as the camera
/// turned: ~95 % of the tongues that popped in and out while the camera moved, lane nfire 2026-11-03), so it makes no tongue.
pub(super) fn is_tri(q: &WorldQuad) -> bool {
	q.pos[2] == q.pos[3]
}

/// Hides (`took`) the ground-fire triangles `tris` (`is_tri`) up to `reach` m from `eye` while whole quads made the tongues there: the
/// tongues stand for the whole fire, the mirror's second copy of it would draw as flat retail sheets through them. A frame with no whole
/// ground quad keeps its triangles (a fire drawn only as triangles stays retail).
pub(super) fn hide_tris(quads: &[WorldQuad], tris: &[usize], whole: bool, eye: [f32; 3], reach: f32, took: &mut [bool]) {
	if !whole {
		return;
	}
	for &i in tris {
		let low = quads[i].pos.iter().copied().min_by(|a, b| a[2].total_cmp(&b[2])).unwrap_or(quads[i].pos[0]);
		if dist2(low, eye) <= reach * reach {
			took[i] = true;
		}
	}
}

/// The tongue a retail fire quad stands for: (emitter, tint, fade). The two lowest corners are the bottom edge, the two highest the top
/// edge; None for a quad that is not upright (too flat) or too narrow. The tongue stands on the bottom edge's midpoint (the slot's point:
/// retail turns the billboard about it to face the camera, rac1/fire.rs `quads`), as tall as the quad's side and as wide as its bottom
/// edge: all three stay put while the billboard turns to face a moving camera. Its fade is the quad's bottom alpha (the slot's fade).
pub(super) fn ground_of(q: &WorldQuad) -> Option<(Emit, [f32; 3], f32)> {
	let mut ix = [0usize, 1, 2, 3];
	ix.sort_by(|&a, &b| q.pos[a][2].total_cmp(&q.pos[b][2]));
	let mid = |a: usize, b: usize| -> [f32; 3] { std::array::from_fn(|k| (q.pos[a][k] + q.pos[b][k]) * 0.5) };
	let (low, top) = (mid(ix[0], ix[1]), mid(ix[2], ix[3]));
	let (side, w) = (dist(low, top), dist(q.pos[ix[0]], q.pos[ix[1]]));
	if top[2] - low[2] < 0.3 || top[2] - low[2] < UPRIGHT * side || w < 0.1 {
		return None;
	}
	let fade = ((q.colour[ix[0]][3] + q.colour[ix[1]][3]) * 0.5).clamp(0.0, 1.0);
	let c = q.colour[ix[0]];
	Some((Emit { base: low, h: side * tune("fire_h", 1.0), w: w * tune("fire_w", 1.25), seed: seed_at(low) }, [c[0], c[1], c[2]], fade))
}

/// One tongue per quad. Candidates beyond `far` of `eye` are dropped (retail billboards). A quad standing on the same point as an
/// earlier one with the same height (`SAME_FOOT`, `SAME_H`) is a member of that tongue, which keeps the brighter fade. At most `cap`
/// tongues, the nearest first.
pub(super) fn tongues_of(items: impl Iterator<Item = Item>, eye: [f32; 3], far: f32, cap: usize) -> Vec<Tongue> {
	let mut v: Vec<Item> = items.filter(|it| dist2(it.1.base, eye) <= far * far).collect();
	// A canonical order, whatever order the game submitted the quads in: by foot, the taller quad first.
	v.sort_by(|a, b| foot_key(a.1.base).cmp(&foot_key(b.1.base)).then(b.1.h.total_cmp(&a.1.h)).then(a.0.cmp(&b.0)));
	let mut out: Vec<Tongue> = Vec::new();
	// The earliest tongue a quad is a twin of, among the tongues in nearby grid cells (review R25 m5: not all pairs).
	let mut grid = Grid::new(SAME_FOOT);
	for (i, e, tint, fade) in v {
		let same = |c: &Tongue| dist2(c.emit.base, e.base) < SAME_FOOT * SAME_FOOT && (c.emit.h - e.h).abs() <= SAME_H * c.emit.h.max(e.h);
		let twin = grid.near(e.base).filter(|&k| same(&out[k])).min();
		match twin {
			Some(k) => {
				let c = &mut out[k];
				if fade > c.fade {
					(c.tint, c.fade) = (tint, fade);
				}
				c.members.push(i);
			}
			None => {
				grid.insert(e.base, out.len());
				out.push(Tongue { emit: e, tint, fade, members: vec![i], noise: [0.0; 2] });
			}
		}
	}
	out.sort_by(|a, b| dist2(a.emit.base, eye).total_cmp(&dist2(b.emit.base, eye)).then(foot_key(a.emit.base).cmp(&foot_key(b.emit.base))).then(b.emit.h.total_cmp(&a.emit.h)));
	out.truncate(cap);
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	pub(super) fn quad(base_z: f32, top_z: f32, x0: f32, x1: f32) -> WorldQuad {
		// Retail loop order [left-top, left-bottom, right-bottom, right-top].
		let c = |a: f32| [1.0, 0.5, 0.25, a];
		WorldQuad { pos: [[x0, 5.0, top_z], [x0, 5.0, base_z], [x1, 5.0, base_z], [x1, 5.0, top_z]], uv: [[0.0; 2]; 4], colour: [c(0.0), c(0.99), c(0.99), c(0.0)], tex: 46, additive: true }
	}

	fn item(i: usize, q: &WorldQuad) -> Item {
		let (e, t, f) = ground_of(q).expect("upright");
		(i, e, t, f)
	}

	/// The quad turned by `ang` about the x axis through (y 5, z `pz`).
	fn tilted(mut q: WorldQuad, ang: f32, pz: f32) -> WorldQuad {
		let (ca, sa) = (ang.cos(), ang.sin());
		for p in q.pos.iter_mut() {
			let (dy, dz) = (p[1] - 5.0, p[2] - pz);
			(p[1], p[2]) = (5.0 + dy * ca - dz * sa, pz + dy * sa + dz * ca);
		}
		q
	}

	#[test]
	fn the_mirrors_triangles_make_no_tongue_and_hide_where_the_tongues_reach() {
		// One flame quad as the effects layer's two triangles [top-left, bottom-left, top-right] and [bottom-left, top-right, bottom-right].
		let q = quad(10.0, 14.0, 0.0, 2.0);
		let (tl, bl, br, tr) = (q.pos[0], q.pos[1], q.pos[2], q.pos[3]);
		let tri = |a, b, c| WorldQuad { pos: [a, b, c, c], ..q };
		let (t0, t1) = (tri(tl, bl, tr), tri(bl, tr, br));
		assert!(is_tri(&t0) && is_tri(&t1) && !is_tri(&q));
		// Its "foot" would be a bottom and a top corner's midpoint: halfway up the flame (the pops).
		assert_eq!(ground_of(&t0).map(|e| e.0.base[2]), Some(12.0));
		let quads = [q, t0, t1];
		let mut took = [false; 3];
		hide_tris(&quads, &[1, 2], true, [1.0, 0.0, 10.0], 10.0, &mut took);
		assert_eq!(took, [false, true, true], "within reach: hidden (the whole quad's tongue stands for the fire)");
		let mut took = [false; 3];
		hide_tris(&quads, &[1, 2], true, [1.0, 0.0, 10.0], 4.0, &mut took);
		assert_eq!(took, [false; 3], "beyond the farthest tongue: retail");
		hide_tris(&quads, &[1, 2], false, [1.0, 0.0, 10.0], 10.0, &mut took);
		assert_eq!(took, [false; 3], "no whole quad this frame: the triangles stay retail");
	}

	#[test]
	fn ground_fire_becomes_an_upright_tongue_on_its_bottom_edge() {
		let (e, tint, fade) = ground_of(&quad(10.0, 14.0, 0.0, 2.0)).expect("tongue");
		assert_eq!(e.base, [1.0, 5.0, 10.0]);
		assert!((e.h - 4.0).abs() < 1e-4 && (e.w - 2.0 * 1.25).abs() < 1e-4, "as tall as the billboard (lane nfire: 1.5 x made tall streaky walls)");
		assert_eq!(tint, [1.0, 0.5, 0.25]);
		assert!((fade - 0.99).abs() < 1e-5, "the foot's alpha, not the transparent top's");
		// A flat quad and a sliver are not flames.
		assert!(ground_of(&quad(10.0, 10.1, 0.0, 2.0)).is_none());
		assert!(ground_of(&quad(10.0, 14.0, 0.0, 0.05)).is_none());
		// Retail turns the billboard about its slot's point (the bottom edge's midpoint) to face the camera: the same tongue.
		let (te, _, _) = ground_of(&tilted(quad(10.0, 14.0, 0.0, 2.0), 0.5, 10.0)).expect("tilted tongue");
		assert!((0..3).all(|k| (te.base[k] - e.base[k]).abs() < 1e-4) && (te.h - e.h).abs() < 1e-4 && (te.w - e.w).abs() < 1e-4);
		assert_eq!(te.seed, e.seed);
		// Lying down (tilted 70 degrees) it is not a flame.
		assert!(ground_of(&tilted(quad(10.0, 14.0, 0.0, 2.0), 1.22, 10.0)).is_none());
		// Quad order does not matter and the seed is stable under a small jitter.
		let mut q = quad(10.0, 14.0, 0.0, 2.0);
		q.pos.swap(0, 2);
		assert_eq!(ground_of(&q).unwrap().0.base, e.base);
		assert_eq!(ground_of(&quad(10.02, 14.0, 0.01, 2.01)).unwrap().0.seed, e.seed);
	}

	#[test]
	fn a_far_quad_is_not_a_tongue() {
		let q = quad(10.0, 14.0, 0.0, 2.0);
		assert!(tongues_of([item(0, &q)].into_iter(), [1.0, 5.0, 10.0], 120.0, 8).len() == 1);
		assert!(tongues_of([item(0, &q)].into_iter(), [500.0, 5.0, 10.0], 120.0, 8).is_empty(), "beyond the fire distance: the retail billboard stays");
	}

	#[test]
	fn each_slot_of_a_fire_is_its_own_tongue_with_its_own_size_and_fade() {
		// Two slots of one emitter: a big one holding, a small one fading in close by. Merged, the new slot re-formed the whole fire.
		let (big, mut small) = (quad(10.0, 14.0, 0.0, 2.0), quad(10.0, 12.0, 0.3, 1.3));
		small.colour[1][3] = 0.1;
		small.colour[2][3] = 0.1;
		let t = tongues_of([item(3, &small), item(7, &big)].into_iter(), [0.0; 3], 120.0, 8);
		assert_eq!(t.len(), 2);
		let (tb, ts) = if t[0].members == vec![7] { (&t[0], &t[1]) } else { (&t[1], &t[0]) };
		assert_eq!((tb.members.clone(), ts.members.clone()), (vec![7], vec![3]));
		assert!((tb.emit.h - 4.0).abs() < 1e-4 && (ts.emit.h - 2.0).abs() < 1e-4);
		assert!((tb.fade - 0.99).abs() < 1e-5 && (ts.fade - 0.1).abs() < 1e-5, "each keeps its own fade");
		// The same flame drawn twice (same point, same height) is one tongue that takes both quads, with the brighter fade.
		let twin = quad(10.0, 14.2, 0.05, 2.05);
		let t = tongues_of([item(1, &big), item(2, &twin)].into_iter(), [0.0; 3], 120.0, 8);
		assert_eq!(t.len(), 1);
		let mut m = t[0].members.clone();
		m.sort();
		assert_eq!(m, vec![1, 2]);
	}

	#[test]
	fn the_submission_order_does_not_change_the_tongues() {
		let qs = [quad(10.0, 14.0, 0.0, 2.0), quad(10.2, 13.0, 0.3, 1.9), quad(10.0, 12.0, -0.4, 1.0), quad(10.0, 14.1, 0.02, 2.02)];
		let run = |order: [usize; 4]| -> Vec<(Emit, [f32; 3], f32)> {
			tongues_of(order.iter().map(|&k| item(k, &qs[k])), [0.0; 3], 120.0, 8).iter().map(|t| (t.emit, t.tint, t.fade)).collect()
		};
		let a = run([0, 1, 2, 3]);
		assert_eq!(a.len(), 3);
		for order in [[3, 2, 1, 0], [1, 0, 3, 2], [1, 2, 0, 3]] {
			assert_eq!(run(order), a, "order {order:?}");
		}
	}

	#[test]
	fn over_the_cap_the_farthest_fires_stay_retail() {
		let qs: Vec<WorldQuad> = (0..6).map(|k| quad(10.0, 14.0, 20.0 * k as f32, 20.0 * k as f32 + 2.0)).collect();
		// Submitted farthest first: the cap must still keep the nearest three.
		let items: Vec<Item> = (0..6).rev().map(|k| item(k, &qs[k])).collect();
		let t = tongues_of(items.into_iter(), [0.0, 5.0, 10.0], 1000.0, 3);
		let mut taken: Vec<usize> = t.iter().flat_map(|t| t.members.clone()).collect();
		taken.sort();
		assert_eq!(taken, vec![0, 1, 2]);
	}
}
