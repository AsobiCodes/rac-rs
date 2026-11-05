//! Tests of the vendor portrait's geometry (pure: the hole, the card around it, where the quads go).
use super::*;
use crate::viewer::{PORTRAIT_PANEL_TEX, RETAIL_OV_TEX};

fn quad(rect: [f32; 4]) -> Quad {
	Quad { tex: PORTRAIT_PANEL_TEX, rect, uv: [0.0, 0.0, 0.5, 0.25], colour: [1.0; 4], angle: 0.0, corners: None }
}

#[test]
fn the_hole_sits_inside_the_detail_card_between_the_name_and_the_price_line() {
	let h = hole(DETAIL);
	assert!(h.x >= DETAIL.x + SIDE - 0.01 && h.right() <= DETAIL.right() - SIDE + 0.01, "{h:?}");
	assert!(h.y >= DETAIL.y + BAND_TOP - 0.01 && h.bottom() <= DETAIL.bottom() - BAND_BOTTOM + 0.01, "{h:?}");
	assert!((h.w / h.h - ASPECT).abs() < 0.01, "the hole keeps the retail panel's aspect: {h:?}");
	assert!(h.w > 300.0 && h.h > 240.0, "big enough to see the guy: {h:?}");
}

#[test]
fn the_card_leaves_the_hole_open_and_frames_it() {
	let h = hole(DETAIL);
	let prims = card(h);
	// No fill covers the hole's centre: every filled RRect of the card is outside it (the clip strips) or a border-only outline.
	let (cx, cy) = (h.cx(), h.cy());
	for p in &prims {
		if let Prim::RRect { rect, fill, .. } = p {
			if fill.a > 0 {
				assert!(!rect.contains(cx, cy), "a fill covers the hole: {rect:?}");
			}
		}
	}
	assert!(prims.iter().any(|p| matches!(p, Prim::RRect { rect, border: Some(_), .. } if *rect == h)), "the hole has its hairline");
}

#[test]
fn a_quad_of_another_aspect_is_letterboxed_inside_the_hole() {
	let h = hole(DETAIL);
	let win = (1280, 720);
	// A square retail quad: fitted at its own aspect, centred in the (wider) hole.
	let (back, inner) = rects(win, h, Some([0.0, 0.0, 100.0, 100.0]));
	let inner = inner.expect("a quad was given");
	assert!((inner[2] - inner[3]).abs() < 0.01, "square stays square: {inner:?}");
	assert!(inner[3] <= back[3] + 0.01 && inner[2] <= back[2] + 0.01);
	assert!(((inner[0] + inner[2] / 2.0) - (back[0] + back[2] / 2.0)).abs() < 0.01, "centred");
	// The real aspect fills the hole.
	let (back, inner) = rects(win, h, Some([5.0, 6.0, 295.0, 295.0 / ASPECT]));
	let inner = inner.unwrap();
	assert!((inner[2] - back[2]).abs() < 1.0 || (inner[3] - back[3]).abs() < 1.0, "fits one side: {inner:?} in {back:?}");
	// No quad (or a degenerate one): only the backdrop.
	assert!(rects(win, h, None).1.is_none());
	assert!(rects(win, h, Some([0.0, 0.0, 0.0, 10.0])).1.is_none());
}

#[test]
fn rects_follow_the_window_size() {
	let h = hole(DETAIL);
	let (a, _) = rects((1280, 720), h, None);
	let (b, _) = rects((2560, 1440), h, None);
	for i in 0..4 {
		assert!((b[i] - 2.0 * a[i]).abs() < 0.01, "{a:?} vs {b:?}");
	}
}

#[test]
fn place_pushes_a_black_backdrop_then_the_retail_quad_keeping_its_texture_and_uv() {
	let h = hole(DETAIL);
	let mut hud = Vec::new();
	let q = quad([700.0, 120.0, 295.0, 230.0]);
	place(&mut hud, (1280, 720), h, Some(q));
	assert_eq!(hud.len(), 2);
	assert_eq!(hud[0].colour, [0.0, 0.0, 0.0, 1.0]);
	assert_ne!(hud[0].tex, PORTRAIT_PANEL_TEX);
	assert_eq!((hud[1].tex, hud[1].uv), (PORTRAIT_PANEL_TEX, q.uv));
	assert_ne!(hud[1].tex, RETAIL_OV_TEX);
	let (back, inner) = (hud[0].rect, hud[1].rect);
	assert!(inner[0] >= back[0] - 0.01 && inner[1] >= back[1] - 0.01 && inner[0] + inner[2] <= back[0] + back[2] + 0.01 && inner[1] + inner[3] <= back[1] + back[3] + 0.01);
	// The game drew no portrait this frame: the backdrop alone.
	let mut hud = Vec::new();
	place(&mut hud, (1280, 720), h, None);
	assert_eq!(hud.len(), 1);
}
