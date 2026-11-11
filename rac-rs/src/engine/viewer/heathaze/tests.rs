use super::*;

/// A 1920 x 1080 view from (0, 0, 2) looking along +Y, Z up.
fn view() -> View {
	View { eye: [0.0, 0.0, 2.0], f: [0.0, 1.0, 0.0], r: [1.0, 0.0, 0.0], u: [0.0, 0.0, 1.0], tx: 16.0 / 9.0 * 0.6, ty: 0.6, w: 1920, h: 1080 }
}

#[test]
fn only_fire_medium_and_high_draw() {
	assert_eq!(quality(0), None);
	assert_eq!(quality(1), None);
	assert_eq!(quality(2).map(|q| q.cols), Some(8));
	assert_eq!(quality(3).map(|q| q.cols), Some(MAX_COLS));
	assert!(quality(2).unwrap().amp < quality(3).unwrap().amp);
}

#[test]
fn no_source_no_pass() {
	assert_eq!(plan(&view(), &[], &[], quality(3).unwrap()), None);
}

#[test]
fn a_fire_ahead_gives_a_rect_round_it() {
	let p = plan(&view(), &[([0.0, 10.0, 0.5], 0.5, 0.5)], &[], quality(3).unwrap()).expect("plan");
	assert_eq!(p.cols.len(), 1);
	let [x0, y0, x1, y1] = p.rect;
	// Centred horizontally on the screen, wholly inside it, small.
	assert!(x0 > 700 && x1 < 1220 && x0 < 960 && x1 > 960, "{:?}", p.rect);
	assert!(y0 < y1 && y1 <= 1080 && (x1 - x0) < 600);
}

#[test]
fn fires_behind_far_or_off_screen_are_dropped() {
	let q = quality(3).unwrap();
	assert_eq!(plan(&view(), &[([0.0, -10.0, 0.5], 0.5, 0.5)], &[], q), None, "behind the eye");
	assert_eq!(plan(&view(), &[([0.0, FADE_FAR + 20.0, 0.5], 0.5, 0.5)], &[], q), None, "past the fade");
	assert_eq!(plan(&view(), &[([40.0, 5.0, 0.5], 0.5, 0.5)], &[], q), None, "off to the side");
}

#[test]
fn medium_keeps_the_nearest_eight() {
	let cores: Vec<_> = (0..12).map(|k| ([0.0, 5.0 + 2.0 * k as f32, 0.5], 0.4, 0.4)).collect();
	let p = plan(&view(), &cores, &[], quality(2).unwrap()).expect("plan");
	assert_eq!(p.cols.len(), 8);
	assert!(p.cols.iter().all(|c| c[0][1] <= 5.0 + 2.0 * 7.0));
}

/// Review S2: the screen test comes before the nearest-N cut, so fires behind the eye never take the visible fires' slots.
#[test]
fn medium_keeps_the_nearest_eight_on_screen() {
	let mut cores: Vec<_> = (0..10).map(|k| ([0.0, -3.0 - k as f32, 0.5], 0.4, 0.4)).collect();
	cores.extend((0..4).map(|k| ([0.0, 20.0 + k as f32, 0.5], 0.4, 0.4)));
	let p = plan(&view(), &cores, &[], quality(2).unwrap()).expect("plan");
	assert_eq!(p.cols.len(), 4, "the four fires ahead, none of the ten nearer ones behind");
	assert!(p.cols.iter().all(|c| c[0][1] >= 20.0));
	let lava = [[-2.0, 30.0, 0.0, 2.0, 34.0, 0.0], [-2.0, 8.0, 0.0, 2.0, 12.0, 0.0], [-2.0, 20.0, 0.0, 2.0, 24.0, 0.0], [-2.0, 14.0, 0.0, 2.0, 18.0, 0.0], [-2.0, 4.0, 0.0, 2.0, 6.0, 0.0]];
	let p = plan(&view(), &[], &lava, quality(3).unwrap()).expect("plan");
	assert_eq!(p.lava.iter().map(|l| l[0][1]).collect::<Vec<_>>(), [4.0, 8.0, 14.0, 20.0], "the nearest lava, not table order");
}

/// Review S3: a box beside and behind the eye, wholly outside the view, gives no rect (it used to cover the screen); one that
/// reaches behind the eye but into the view is clipped at the near plane, not the whole screen.
#[test]
fn boxes_reaching_behind_the_eye_are_clipped() {
	assert_eq!(screen_rect(&view(), [6.0, -3.0, 0.0], [8.0, 1.0, 3.0]), None, "right of the view, partly behind");
	let r = screen_rect(&view(), [0.5, -1.0, 1.5], [1.5, 6.0, 2.5]).expect("rect");
	assert!(r[0] > 960.0 && r[2] == 1920.0, "only the right of the screen: {r:?}");
}

/// Review B1: the wobble's speeds are whole cycles per fire-clock period, so the pattern does not jump at the wrap.
#[test]
fn the_wobble_is_seamless_at_the_clock_wrap() {
	assert!(SHADER.contains(&format!("const PERIOD: f32 = {:.1};", super::super::firefx_clock::PERIOD)));
	for k in ["W_RISE", "W_SWAY", "W_FAST", "W_SIDE", "W_SLOW"] {
		let line = SHADER.lines().find(|l| l.starts_with(&format!("const {k}: f32 = "))).expect(k);
		let n: f32 = line.split('=').nth(1).unwrap().trim().split(' ').next().unwrap().parse().unwrap();
		assert_eq!(n.fract(), 0.0, "{k}: whole cycles");
		assert!(SHADER.contains(&format!("t * {k}")), "{k} used");
	}
	assert!(!SHADER.contains("t * 5.5") && !SHADER.contains("t * 9.0"));
}

#[test]
fn a_box_reaching_the_eye_covers_the_screen() {
	let r = screen_rect(&view(), [-5.0, -5.0, 0.0], [5.0, 5.0, 3.0]).expect("rect");
	assert_eq!(r, [0.0, 0.0, 1920.0, 1080.0]);
}

#[test]
fn lava_is_cut_to_the_fade_range_and_far_lava_dropped() {
	let eye = [0.0, 0.0, 5.0];
	let l = lava_layer([-500.0, -500.0, -1.0, 500.0, 500.0, -1.0], eye).expect("near lava");
	assert_eq!(l[0], [-FADE_FAR, -FADE_FAR, FADE_FAR, FADE_FAR]);
	assert_eq!(l[1][0], -1.0);
	assert_eq!(lava_layer([200.0, 200.0, 0.0, 300.0, 300.0, 0.0], eye), None);
	assert_eq!(lava_layer([-5.0, -5.0, -80.0, 5.0, 5.0, -80.0], eye), None, "far below");
}

#[test]
fn lava_under_the_view_covers_the_screen() {
	let p = plan(&view(), &[], &[[-100.0, -100.0, 0.5, 100.0, 100.0, 0.5]], quality(3).unwrap()).expect("plan");
	assert_eq!(p.lava.len(), 1);
	assert_eq!(p.rect, [0, 0, 1920, 1080]);
}

#[test]
fn columns_grow_with_the_fire() {
	let small = column([0.0; 3], 0.2, 0.2);
	let big = column([0.0; 3], 1.0, 1.0);
	assert!(big[0][3] > small[0][3] && big[1][0] > small[1][0] && big[1][1] >= small[1][1]);
	assert!(small[1][1] >= 0.4 && big[1][1] <= 1.0);
	let huge = column([0.0; 3], 6.0, 6.0);
	assert!(huge[0][3] <= COL_MAX_R && huge[1][0] <= COL_MAX_H, "a jet-sized core stays a modest column");
}

#[test]
fn uniform_matches_the_shader_layout() {
	assert_eq!(std::mem::size_of::<HazeU>(), 16 * (1 + 4 + 1 + 1 + 2 * MAX_COLS + 2 * MAX_LAVA));
	assert!(SHADER.contains("array<vec4<f32>, 32>") && SHADER.contains("array<vec4<f32>, 8>"));
}

#[test]
fn the_shader_validates() {
	let m = naga::front::wgsl::parse_str(SHADER).expect("wgsl parse");
	naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
}
