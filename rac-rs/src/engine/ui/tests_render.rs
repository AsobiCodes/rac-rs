//! Stage S2 tests: fonts, text fit, atlas, glyph cache, shader (naga WGSL + HLSL), blur numbers, scale math, off state, draw list.
use super::atlas::Atlas;
use super::backdrop::{kernel, plan, Ease, TAPS};
use super::font::{FontMeasure, FontSet};
use super::glyph_cache::{GlyphCache, GlyphKey};
use super::gpu::UiGpu;
use super::instances::{self, K_IMAGE, K_RRECT, K_TEXT};
use super::kit::{Scale, TextMeasure};
use super::prim::{Prim, Rect, Rgba};
use super::{demo, fit, gpu_pipes, testkit};

fn make() -> Box<dyn TextMeasure + Send> {
	Box::new(FontMeasure)
}

#[test]
fn font_covers_every_char_of_every_text() {
	let literal = "\u{e9}\u{e8}\u{ea}\u{e0}\u{e7}\u{153}\u{ab}\u{bb}\u{e4}\u{f6}\u{fc}\u{df}\u{bf}\u{a1}\u{f1}\u{b7}\u{2026}\u{b0}\u{c9}\u{c4}".to_string();
	let mut texts = fit::all_texts(demo::menu());
	texts.extend(fit::all_texts(testkit::menu()));
	texts.extend(fit::drawn_texts(demo::menu(), &make));
	texts.push(literal);
	let missing = fit::missing_in(&texts);
	assert!(missing.is_empty(), "no glyph for: {missing:?}");
	assert!(fit::missing_chars(demo::menu()).is_empty());
}

#[test]
fn text_fits_column_at_design_size() {
	let bad = fit::overflows(demo::menu(), &make);
	assert!(bad.is_empty(), "{} texts wider than their box, first: {:?}", bad.len(), bad.first());
}

#[test]
fn demo_detail_panels_fit_above_their_meter_and_footer() {
	let bad = fit::detail_cuts(demo::menu(), &make);
	assert!(bad.is_empty(), "{} detail panels cut or past their floor, first: {:?}", bad.len(), bad.first());
}

#[test]
fn atlas_packing_no_overlap_and_grows() {
	let mut a = Atlas::new(64, 128);
	let mut got: Vec<(u32, u32, u32, u32)> = Vec::new();
	let mut n = 0u32;
	while let Some((x, y)) = a.alloc(5 + n % 9, 4 + n % 7) {
		got.push((x, y, 5 + n % 9, 4 + n % 7));
		n += 1;
	}
	assert!(got.len() > 20);
	let check = |got: &[(u32, u32, u32, u32)], size: u32| {
		for (i, r) in got.iter().enumerate() {
			assert!(r.0 + r.2 <= size && r.1 + r.3 <= size, "{r:?} outside {size}");
			for o in &got[i + 1..] {
				assert!(r.0 + r.2 <= o.0 || o.0 + o.2 <= r.0 || r.1 + r.3 <= o.1 || o.1 + o.3 <= r.1, "{r:?} overlaps {o:?}");
			}
		}
	};
	check(&got, 64);
	assert!(a.grow() && a.size() == 128 && !a.grow(), "grows once to the maximum");
	let (x, y) = a.alloc(10, 10).expect("room after growing");
	got.push((x, y, 10, 10));
	check(&got, 128);
	a.clear();
	assert_eq!((a.shelf_count(), a.alloc(8, 8)), (0, Some((0, 0))));
}

/// R1-5: a bitmap bigger than the biggest atlas is refused without clearing the atlas (the glyphs already stored stay).
#[test]
fn glyph_cache_refuses_a_bitmap_bigger_than_its_largest_atlas() {
	use super::shapes::Shape;
	let fonts = FontSet::get();
	let mut c = GlyphCache::with_limits(32, 64);
	let small = GlyphKey::shape(Shape::ChevronLeft, 8);
	assert!(c.ensure(fonts, small));
	assert!(!c.ensure(fonts, GlyphKey::shape(Shape::ChevronRight, 200)), "200 px cannot fit a 64 px atlas");
	assert!(c.get(&small).is_some(), "the atlas was not cleared");
	assert_eq!(GlyphKey::shape(Shape::ChevronLeft, 60_000).size_q as u32, super::glyph_cache::SHAPE_MAX_PX, "a shape bitmap is capped");
}

#[test]
fn glyph_cache_growth_keeps_slots_and_pixels() {
	let fonts = FontSet::get();
	let mut c = GlyphCache::with_limits(32, 256);
	let key = GlyphKey::glyph(0, 'A', 80, 0);
	assert!(c.ensure(fonts, key));
	let s = c.get(&key).unwrap();
	let sum = |c: &GlyphCache| -> u32 {
		let size = c.size();
		(0..s.h).flat_map(|j| (0..s.w).map(move |i| (i, j))).map(|(i, j)| c.pixels[((s.y + j) * size + s.x + i) as usize] as u32).sum()
	};
	let before = sum(&c);
	assert!(before > 0, "the glyph has ink");
	for ch in "BCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz".chars() {
		assert!(c.ensure(fonts, GlyphKey::glyph(0, ch, 80, 0)));
	}
	assert!(c.size() > 32 && c.epoch() == 0, "the atlas grew without a clear");
	let t = c.get(&key).unwrap();
	assert_eq!((t.x, t.y, t.w, t.h, t.dx, t.dy), (s.x, s.y, s.w, s.h, s.dx, s.dy));
	assert_eq!(sum(&c), before);
}

#[test]
fn wgsl_ui_validates_naga_wgsl_and_hlsl_out() {
	let m = naga::front::wgsl::parse_str(gpu_pipes::BUILT_IN).expect("ui shader");
	let names: Vec<_> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
	for n in ["vs_full", "fs_down", "fs_blur", "fs_comp", "vs_inst", "fs_inst"] {
		assert!(names.contains(&n), "{n}");
	}
	let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
	let info = v.validate(&m).expect("ui shader validates");
	assert!(gpu_pipes::BUILT_IN.contains(&format!("const K_RRECT: f32 = {K_RRECT:?};")) && gpu_pipes::BUILT_IN.contains(&format!("const K_TEXT: f32 = {K_TEXT:?};")) && gpu_pipes::BUILT_IN.contains(&format!("const K_IMAGE: f32 = {K_IMAGE:?};")));
	#[cfg(windows)]
	{
		let (options, pipeline) = (naga::back::hlsl::Options::default(), naga::back::hlsl::PipelineOptions::default());
		let mut out = String::new();
		naga::back::hlsl::Writer::new(&mut out, &options, &pipeline).write(&m, &info, None).expect("HLSL writes");
		assert!(out.contains("fs_comp") && out.contains("fs_inst"), "the UI entry points are in the HLSL");
	}
	let _ = info;
}

#[test]
fn blur_kernel_weights_sum_to_one() {
	for i in 1..=12 {
		let w = kernel(i as f32 * 0.25);
		let sum = w[0] + 2.0 * w[1..TAPS].iter().sum::<f32>();
		assert!((sum - 1.0).abs() < 1e-5, "sigma {}: {sum}", i as f32 * 0.25);
		assert!(w.windows(2).all(|p| p[0] >= p[1]), "weights fall off");
	}
	assert_eq!(kernel(0.0)[0], 1.0);
	for sigma in [2.0, 8.0, 16.0, 24.0, 40.0, 60.0] {
		let (k, s) = plan(sigma);
		assert!((1..=5).contains(&k) && s > 0.0 && s <= 2.41, "sigma {sigma}: k {k} s {s}");
	}
	assert!(plan(60.0).0 > plan(4.0).0, "wider blur halves more");
}

#[test]
fn ease_moves_smoothly_and_the_first_target_is_immediate() {
	let mut e = Ease::default();
	e.target([16.0, 0.42], 0.0, 0.3);
	assert_eq!(e.value(0.0, 0.3), [16.0, 0.42]);
	e.target([2.0, 0.05], 1.0, 0.3);
	assert_eq!(e.value(1.0, 0.3), [16.0, 0.42]);
	let mid = e.value(1.15, 0.3);
	assert!((mid[0] - 9.0).abs() < 1e-3 && (mid[1] - 0.235).abs() < 1e-3, "{mid:?}");
	let end = e.value(9.0, 0.3);
	assert!((end[0] - 2.0).abs() < 1e-4 && (end[1] - 0.05).abs() < 1e-4, "{end:?}");
	e.reset();
	e.target([7.0, 0.1], 20.0, 0.3);
	assert_eq!(e.value(20.0, 0.3), [7.0, 0.1], "after a reset the next target is taken at once");
}

#[test]
fn scale_math_to_window() {
	let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
	for (w, h, s, ox, oy) in [(1280, 720, 1.0, 0.0, 0.0), (1280, 800, 1.0, 0.0, 40.0), (1920, 1080, 1.5, 0.0, 0.0), (3440, 1440, 2.0, 440.0, 0.0)] {
		let sc = Scale::fit(w, h);
		assert!(near(sc.s, s) && near(sc.ox, ox) && near(sc.oy, oy), "{w}x{h}: {} {} {}", sc.s, sc.ox, sc.oy);
	}
}

#[test]
fn off_state_allocates_nothing() {
	let mut g = UiGpu::default();
	assert!(!g.is_allocated());
	g.set_scene(Vec::new());
	assert!(g.is_allocated(), "a scene waits for the next draw");
	g.release();
	assert!(!g.is_allocated(), "release frees everything");
	assert_eq!((g.held(), g.pipe_builds()), (Default::default(), 0));
}

#[test]
fn demo_parse_cases() {
	assert_eq!(demo::parse(""), None);
	assert_eq!(demo::parse("0"), None);
	assert_eq!(demo::parse("off"), None);
	assert_eq!(demo::parse("1"), Some(demo::DemoSpec::default()));
	let s = demo::parse("tab=1,lang=de,focus=3,peek,pad,letters").unwrap();
	assert_eq!((s.tab, s.lang, s.focus, s.peek, s.pad, s.glyphs), (1, 2, 3, true, true, 1));
	assert_eq!(demo::parse("lang=fr").unwrap().lang, 1);
}

#[test]
fn draw_list_of_the_demo_is_finite_and_clip_culls() {
	let fonts = FontSet::get();
	let sc = Scale::fit(1280, 720);
	let mut cache = GlyphCache::default();
	let ui = demo::new_ui(demo::DemoSpec::default(), make());
	let built = instances::build(&ui.frame().1, &sc, fonts, &mut cache);
	assert!(built.insts.len() > 50 && built.backdrop.is_some());
	for i in &built.insts {
		assert!(i.rect.iter().chain(&i.fill).chain(&i.border).chain(&i.uv).all(|v| v.is_finite()), "{i:?}");
	}
	let r = |x, y| Prim::RRect { rect: Rect::new(x, y, 20.0, 20.0), radius: 4.0, fill: Rgba::rgba(51, 102, 153, 1.0), border: None };
	let prims = [Prim::ClipPush { rect: Rect::new(0.0, 0.0, 100.0, 100.0), radius: 0.0 }, r(10.0, 10.0), r(500.0, 500.0), Prim::ClipPop, r(500.0, 500.0)];
	let built = instances::build(&prims, &sc, fonts, &mut cache);
	assert_eq!(built.insts.len(), 2, "the row outside the clip is culled, the one after the pop is drawn");
}

/// A picture becomes one K_IMAGE instance with its UV (corner radius capped by the half size); an unknown id draws nothing; the clip culls it.
#[test]
fn an_image_prim_is_one_picture_instance_and_unknown_ids_draw_nothing() {
	let (fonts, sc, mut cache) = (FontSet::get(), Scale::fit(1280, 720), GlyphCache::default());
	let uv = |id: u32| (id == 7).then_some([0.25, 0.5, 0.5, 0.75]);
	let white = Rgba::rgba(255, 255, 255, 1.0);
	let img = |id, x| Prim::Image { rect: Rect::new(x, 10.0, 40.0, 30.0), id, radius: 99.0, tint: white };
	let prims = [img(7, 10.0), img(8, 60.0), Prim::ClipPush { rect: Rect::new(0.0, 0.0, 100.0, 100.0), radius: 0.0 }, img(7, 500.0), Prim::ClipPop];
	let built = instances::build_with(&prims, &sc, fonts, &mut cache, &uv);
	assert_eq!(built.insts.len(), 1);
	let i = &built.insts[0];
	assert_eq!((i.q[2], i.uv), (K_IMAGE, [0.25, 0.5, 0.5, 0.75]));
	assert!(i.q[0] <= i.rect[3] * 0.5 + 1e-3, "the radius is capped by the half height");
	assert!(prims[0].dump().starts_with("image 10 10 40 30 id 7"), "{}", prims[0].dump());
}

/// The old cut: take one character off, place the whole string again, until it fits (or only the ellipsis is left).
fn cut_by_popping(fonts: &FontSet, role: super::prim::FontRole, px: f32, track: f32, text: &str, max_w: f32) -> String {
	let mut chars: Vec<char> = text.chars().collect();
	let mut cut = String::from("\u{2026}");
	while !chars.is_empty() {
		chars.pop();
		cut = chars.iter().collect::<String>() + "\u{2026}";
		if fonts.place(role, px, track, &cut).width <= max_w {
			break;
		}
	}
	cut
}

#[test]
fn the_ellipsis_cut_equals_cutting_one_character_at_a_time() {
	let fonts = FontSet::get();
	let texts = ["Eine sehr lange deutsche \u{dc}berschrift: Donnerstag", "WWWWWWWWWWWWWWWW", "iiiiiiiiiiiiiiiii", "r\u{e9}sum\u{e9} \u{e0} l'\u{e9}t\u{e9}", "AV AVA To Ty", "x", ""];
	let mut cuts = 0;
	for role in super::font::ROLES {
		for px in [9.0, 14.5, 22.0] {
			for track in [0.0, 0.02, 0.1] {
				for text in texts {
					let full = fonts.place(role, px, track, text).width;
					for k in 0..13 {
						let max_w = full * k as f32 / 13.0; // always narrower than the run: the cut path
						let cut = fonts.ellipsis_cut(role, px, track, text, max_w);
						assert_eq!(cut, cut_by_popping(fonts, role, px, track, text, max_w), "{role:?} px {px} track {track} {text:?} max {max_w}");
						assert!(cut.ends_with('\u{2026}') && cut.chars().count() <= text.chars().count().max(1));
						let fits = fonts.place(role, px, track, &cut).width <= max_w;
						assert!(fits || cut == "\u{2026}", "a cut that is not the bare ellipsis fits: {cut:?}");
						cuts += 1;
					}
				}
			}
		}
	}
	assert!(cuts > 1000);
	let long = "x".repeat(4000);
	let start = std::time::Instant::now();
	let cut = fonts.ellipsis_cut(super::prim::FontRole::Regular, 14.0, 0.0, &long, 120.0);
	assert!(cut.chars().count() < 40 && start.elapsed().as_millis() < 200, "one walk, not one placement per character: {cut:?} {:?}", start.elapsed());
}
