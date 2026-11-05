//! Tests of the shared overlay crop (`crop.rs`): the rect math the pause grid and the galactic map both rest on.
use super::*;

#[test]
fn contain_keeps_the_aspect_and_centres() {
	let card = Rect::new(100.0, 50.0, 300.0, 100.0);
	let wide = contain(card, 3.0, 1.0);
	assert_eq!((wide.x, wide.y, wide.w, wide.h), (100.0, 50.0, 300.0, 100.0), "the same aspect fills the card");
	let tall = contain(card, 1.0, 1.0);
	assert_eq!((tall.x, tall.y, tall.w, tall.h), (200.0, 50.0, 100.0, 100.0), "pillar boxes");
	let flat = contain(card, 6.0, 1.0);
	assert_eq!((flat.x, flat.y, flat.w, flat.h), (100.0, 75.0, 300.0, 50.0), "letter boxes");
	assert_eq!(contain(card, 0.0, 5.0), card, "a degenerate aspect leaves the card");
	assert_eq!(contain(card, 5.0, -1.0), card);
}

#[test]
fn inset_cuts_every_side_and_never_inverts() {
	assert_eq!(inset([10.0, 20.0, 100.0, 50.0], 3.0), [13.0, 23.0, 94.0, 44.0]);
	assert_eq!(inset([10.0, 20.0, 100.0, 50.0], -4.0), [10.0, 20.0, 100.0, 50.0], "a negative inset is none");
	assert_eq!(inset([10.0, 20.0, 4.0, 4.0], 9.0), [19.0, 29.0, 1.0, 1.0], "a crop keeps one pixel");
}

#[test]
fn uv_spans_the_whole_picture_and_scales_with_the_region() {
	assert_eq!(uv_of([0.0, 0.0, OV_W, OV_H]), [0.0, 0.0, 1.0, 1.0]);
	assert_eq!(uv_of([256.0, 224.0, 128.0, 112.0]), [0.5, 0.5, 0.75, 0.75]);
}

#[test]
fn a_crop_shows_seven_sixths_wider_than_its_pixels() {
	// 512 x 448 on a 4:3 screen: 4/3 / (512/448) = 7/6.
	let r = display_rect(Rect::new(0.0, 0.0, 1600.0, 1600.0), [0.0, 0.0, 512.0, 448.0]);
	assert!((r.w / r.h - 4.0 / 3.0).abs() < 1e-5, "the whole picture is 4:3");
	// A crop as wide as it is tall shows 7:6.
	let r = display_rect(Rect::new(0.0, 0.0, 700.0, 700.0), [10.0, 10.0, 60.0, 60.0]);
	assert!((r.w / r.h - 7.0 / 6.0).abs() < 1e-5);
}

#[test]
fn the_quad_maps_the_design_hole_into_the_window() {
	// A 1280 x 720 window is the design space itself; a wider window centres it (`Scale::fit`).
	let hole = Rect::new(100.0, 200.0, 300.0, 120.0);
	let uv = uv_of([183.0, 304.0, 307.0, 120.0]);
	let q = quad((1280, 720), hole, uv);
	assert_eq!((q.tex, q.rect, q.uv), (crate::viewer::RETAIL_OV_TEX, [100.0, 200.0, 300.0, 120.0], uv));
	let wide = quad((2560, 720), hole, uv);
	assert_eq!((wide.rect[1], wide.rect[2], wide.rect[3]), (200.0, 300.0, 120.0));
	assert!((wide.rect[0] - (100.0 + 640.0)).abs() < 1e-3, "pillar-boxed by 640 px a side");
	let mut hud = Vec::new();
	place(&mut hud, (1280, 720), hole, uv);
	assert_eq!(hud.len(), 1);
	assert_eq!(hud[0].rect, q.rect);
}
