//! The badge of one entry: its icon (a circle) inside a ring in the grade colour (bronze / silver / gold / platinum), drawn by the
//! pop-up and the list. A locked entry shows its grey icon under a dimmed ring, a locked hidden entry the hidden icon. A game without
//! icons (`AchPack::icons` None) keeps the procedural cup of `toast::cup`.
use super::icons::{Layout, ICON_TEX};
use super::toast::cup;
use super::AchPack;
use crate::overlay::Quad;

/// Share of the badge the icon circle takes: a little under the ring's inner edge, so no gap shows between them.
const ICON_SHARE: f32 = 0.9;
/// A locked entry: opacity of its icon and of its ring (the grey icon is also darkened in the atlas).
const LOCKED_ICON: f32 = 0.85;
const LOCKED_RING: f32 = 0.5;
/// The cup's opacity when it stands in for a locked entry's badge.
const LOCKED_CUP: f32 = 0.45;

/// Pushes the badge of entry `i` into `out`: `at` = [x, y, size] of its square box in window pixels, `a` the overall opacity (pop-up fade).
pub fn badge(out: &mut Vec<Quad>, p: &AchPack, i: usize, unlocked: bool, at: [f32; 3], a: f32) {
	badge_with(out, p, i, unlocked, at, a, super::icons::atlas_failed());
}

/// `badge` with the atlas state given: a failed atlas (`icons::atlas_failed`) draws the cup like a game without icons (m7: the GPU would
/// drop the `ICON_TEX` quads and the entry would show no badge at all).
pub(super) fn badge_with(out: &mut Vec<Quad>, p: &AchPack, i: usize, unlocked: bool, at: [f32; 3], a: f32, atlas_failed: bool) {
	let [x, y, s] = at;
	let icons = p.icons.filter(|_| !atlas_failed);
	let (Some(set), Some(d)) = (icons, p.defs.get(i)) else {
		if let Some(d) = p.defs.get(i) {
			cup(out, x, y, s, d.grade, if unlocked { a } else { a * LOCKED_CUP });
		}
		return;
	};
	let l = Layout::new(p.defs.len(), set.size);
	let (ia, ra) = if unlocked { (a, a) } else { (a * LOCKED_ICON, a * LOCKED_RING) };
	let (inner, c) = (s * ICON_SHARE, d.grade.colour());
	let off = (s - inner) * 0.5;
	let quad = |rect: [f32; 4], tile: usize, colour: [f32; 4]| Quad { tex: ICON_TEX, rect, uv: l.uv(tile), colour, angle: 0.0, corners: None };
	out.push(quad([x + off, y + off, inner, inner], l.tile_for(i, unlocked, d.hidden), [1.0, 1.0, 1.0, ia]));
	out.push(quad([x, y, s, s], l.ring(), [c[0], c[1], c[2], ra]));
}
