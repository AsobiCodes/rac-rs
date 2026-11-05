use super::*;

fn module(src: &str) -> (naga::Module, naga::valid::ModuleInfo) {
	let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("firebook: {}", e.emit_to_string(src)));
	let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("firebook validate: {e:?}"));
	(m, info)
}

#[test]
fn the_shader_validates_and_writes_hlsl() {
	let (m, info) = module(WGSL);
	let names: Vec<&str> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
	assert!(names.contains(&"vs_book") && names.contains(&"fs_book") && names.contains(&"fs_dark") && names.contains(&"fs_light"));
	let (options, pipeline) = (naga::back::hlsl::Options::default(), naga::back::hlsl::PipelineOptions::default());
	let mut out = String::new();
	naga::back::hlsl::Writer::new(&mut out, &options, &pipeline).write(&m, &info, None).expect("HLSL writes");
	assert!(out.contains("fs_book"));
}

/// Frame `f`'s two-byte texel (R temperature, G opacity) at (`x`, `y`) in the atlas (16 x 16 frames of 128 x 256).
fn texel(f: usize, x: usize, y: usize) -> [u8; 2] {
	let (c, r) = (f % 16, f / 16);
	let o = (((r * 256 + y) * AW as usize) + c * 128 + x) * 2;
	[atlas()[o], atlas()[o + 1]]
}

#[test]
fn the_atlas_is_16_by_16_frames_of_128_by_256_two_bytes_and_unpacks() {
	let t = std::time::Instant::now();
	let raw = crate::texpack::inflate::zlib(ATLAS_Z, (AW * AH * 2) as usize).expect("the atlas unpacks");
	eprintln!("firebook atlas: {} bytes deflated -> {} bytes in {} ms", ATLAS_Z.len(), raw.len(), t.elapsed().as_millis());
	assert_eq!(raw.len(), (AW * AH * 2) as usize);
	assert_eq!(atlas().len(), (AW * AH * 2) as usize);
	// The unpack really worked (a bad stream gives all zero): flame in the middle of most frames.
	assert!((0..FRAMES as usize).all(|f| (0..256).any(|y| texel(f, 64, y)[1] > 200)));
	assert_eq!((AW / 128) * (AH / 256), FRAMES);
	// The shader's constants are the atlas's, and the loop is 4 s at the playback rate (as the old 64 frames at 16 fps).
	assert!(WGSL.contains(&format!("const COLS: f32 = {}.0;", AW / 128)) && WGSL.contains(&format!("const FRAMES: f32 = {}.0;", FRAMES)));
	assert_eq!(FRAMES as f32 / FPS, 4.0);
	let m = mips();
	assert_eq!(m.len(), MIPS as usize);
	for (l, d) in m.iter().enumerate() {
		assert_eq!(d.len(), ((AW >> l) * (AH >> l) * 2) as usize);
	}
	assert_eq!(m[0], atlas());
	// A cell of the last level is still whole texels (no level mixes two frames).
	assert!(128 >> (MIPS - 1) >= 2 && 256 >> (MIPS - 1) >= 2);
}

#[test]
fn sprites_stand_along_the_edge_by_the_count_and_height_rule() {
	let (side, edge) = (6.0, 4.0);
	let h = height_of(side);
	assert!((h - side * H_X).abs() < 1e-5);
	let n = count(edge, h, 6);
	// One sprite per flame body less the overlap: 4 m / (12 x 0.5 x 0.6 x 0.3 = 1.08 m) -> 4.
	assert_eq!(n, 4);
	assert_eq!(count(0.2, h, 6), 1);
	assert_eq!(count(50.0, h, 6), 6);
	let mut v = Vec::new();
	place([10.0, 20.0, 5.0], [1.0, 0.0, 0.0], edge, side, 0.37, 0.8, [1.0, 0.5, 0.1], 3, &mut v);
	assert!(!v.is_empty() && v.len() <= (n * 3).div_ceil(2));
	for s in &v {
		// On the bottom edge (a little jitter), on the foot's height, +-H_VAR of the height (a gap tongue at SMALL of it), the width from
		// the frame's aspect.
		let d = ((s.a[0] - 10.0).powi(2) + (s.a[1] - 20.0).powi(2)).sqrt();
		assert!(d <= 0.5 * edge * W_X + 1e-3 && s.a[2] == 5.0);
		assert!(s.a[3] >= SMALL * (1.0 - H_VAR) * h - 1e-4 && s.a[3] <= (1.0 + H_VAR) * h + 1e-4);
		assert!((s.b[0] - s.a[3] * ASPECT).abs() < 1e-4);
		assert!(s.b[1] >= 0.0 && s.b[1] < FRAMES as f32 && s.b[2].abs() == 1.0 && s.b[3] == 0.8);
	}
	// The frame offsets and mirrors differ between sprites (no two in step).
	assert!(v.windows(2).any(|w| w[0].b[1] != w[1].b[1]));
	// The same tongue gives the same sprites every frame.
	let mut again = Vec::new();
	place([10.0, 20.0, 5.0], [1.0, 0.0, 0.0], edge, side, 0.37, 0.8, [1.0, 0.5, 0.1], 3, &mut again);
	assert_eq!(bytemuck::cast_slice::<Spr, u8>(&v), bytemuck::cast_slice::<Spr, u8>(&again));
}

#[test]
fn about_a_quarter_of_the_sprites_are_gaps_and_a_lone_sprite_stays() {
	let (mut out, mut small) = (0, 0);
	for k in 0..10_000u32 {
		match gap_of(hash(0.61, k), GAP, false) {
			None => out += 1,
			Some(f) if f == SMALL => small += 1,
			Some(f) => assert_eq!(f, 1.0),
		}
	}
	// GAP / 2 left out, GAP / 2 small (hash spread +-1.5 %).
	assert!((out as f32 / 1e4 - 0.5 * GAP).abs() < 0.015 && (small as f32 / 1e4 - 0.5 * GAP).abs() < 0.015, "{out} {small}");
	assert!((0..1000u32).all(|k| gap_of(hash(0.3, k), GAP, true) == Some(1.0)));
	// Over many tongues of a long edge some sprites are left out: dark gaps between the tongues, no unbroken wall.
	let (mut placed, mut wanted) = (0, 0);
	for t in 0..200 {
		let mut v = Vec::new();
		place([0.0; 3], [1.0, 0.0, 0.0], 8.0, 6.0, t as f32 * 0.013, 1.0, [1.0, 0.5, 0.1], 3, &mut v);
		placed += v.len();
		wanted += (count(8.0 * W_X, height_of(6.0), max_per(3)) * 3).div_ceil(2);
	}
	assert!(placed < wanted && placed as f32 > 0.8 * wanted as f32, "{placed} of {wanted}");
}
#[test]
fn lower_fire_levels_draw_fewer_wider_sprites_and_off_none() {
	let mut hi = Vec::new();
	let mut lo = Vec::new();
	let mut off = Vec::new();
	place([0.0; 3], [0.0, 1.0, 0.0], 5.0, 6.0, 0.5, 1.0, [1.0, 0.5, 0.1], 3, &mut hi);
	place([0.0; 3], [0.0, 1.0, 0.0], 5.0, 6.0, 0.5, 1.0, [1.0, 0.5, 0.1], 1, &mut lo);
	place([0.0; 3], [0.0, 1.0, 0.0], 5.0, 6.0, 0.5, 1.0, [1.0, 0.5, 0.1], 0, &mut off);
	// (a tongue's disc holds 1.5 x the count, gaps left out: at most that)
	let h = height_of(6.0);
	let (n_lo, n_hi) = (count(5.0 * W_X, h, max_per(1)), count(5.0 * W_X, h, max_per(3)));
	assert!(n_lo <= n_hi && lo.len() <= (n_lo * 3).div_ceil(2) && hi.len() <= (n_hi * 3).div_ceil(2) && lo.len() < hi.len() && off.is_empty());
	assert!(lo.iter().all(|s| s.b[0] >= s.a[3] * ASPECT - 1e-5));
	let mut faded = Vec::new();
	place([0.0; 3], [0.0, 1.0, 0.0], 5.0, 6.0, 0.5, 0.0, [1.0, 0.5, 0.1], 3, &mut faded);
	assert!(faded.is_empty());
}

#[test]
fn sprites_sort_far_to_near_and_keep_the_nearest() {
	let mut v: Vec<Spr> = (0..MAX_SPR + 10).map(|i| Spr { a: [i as f32, 0.0, 0.0, 1.0], ..Default::default() }).collect();
	sort(&mut v, [0.0; 3]);
	assert_eq!(v.len(), MAX_SPR);
	assert!(v.windows(2).all(|w| w[0].a[0] >= w[1].a[0]));
	assert_eq!(v.last().expect("one").a[0], 0.0);
}

#[test]
fn a_flipbook_tongue_keeps_only_its_embers_in_the_sim() {
	use super::super::{firefx_vol, firesim};
	let v = firefx_vol::flame_own([0.0, 0.0, 0.0], 6.0, 2.0, 0.3, 1.0, [1.0, 0.5, 0.1], [0.0, 0.0]);
	let licks = firesim::want_of(&firefx_vol::level_tongue(v), [0.0, 10.0, 2.0], 1.0).expect("licks");
	let embers = firesim::want_of(&firefx_vol::ember_tongue(v), [0.0, 10.0, 2.0], 1.0).expect("embers");
	assert_eq!((licks.e.n[2], embers.e.n[2]), (1, 2));
	assert_eq!(embers.e.b[2], 6.0);
	assert!(embers.live < licks.live * 0.05 && embers.live > 0.0);
}

#[test]
fn the_sprites_do_not_depend_on_the_view() {
	// Nothing about the camera goes in: turning the view round the tongue gives the very same sprites.
	let (mut a, mut b) = (Vec::new(), Vec::new());
	place([3.0, 4.0, 1.0], [1.0, 0.0, 0.0], 6.0, 4.0, 0.21, 1.0, [1.0, 0.5, 0.1], 3, &mut a);
	place([3.0, 4.0, 1.0], [0.0, 1.0, 0.0], 6.0, 4.0, 0.21, 1.0, [1.0, 0.5, 0.1], 3, &mut b);
	assert!(!a.is_empty() && bytemuck::cast_slice::<Spr, u8>(&a) == bytemuck::cast_slice::<Spr, u8>(&b));
}

#[test]
fn the_frames_clamp_zone_is_faded_out_so_no_flat_bars_stick_out() {
	// Some flames of the atlas have opacity at the texel `frame_at` repeats over the outer 1/16 of the frame (columns 8 and 119):
	// without `edge_window` that was a hard-edged horizontal bar beside the fire (user view, nfire24).
	let mut touching = 0;
	for f in 0..FRAMES as usize {
		for y in 0..256usize {
			for x in [8usize, 119] {
				if texel(f, x, y)[1] > 40 {
					touching += 1;
				}
			}
		}
	}
	assert!(touching > 0, "if no frame touches its side the window is not needed");
	// The window is zero over the whole clamp zone (uv.x < 1/16, > 15/16, uv.y < 0.032), not a fade inside it.
	assert!(WGSL.contains("edge_window(i.uv)") && WGSL.contains("smoothstep(0.0625, 0.125, uv.x)") && WGSL.contains("smoothstep(0.9375, 0.875, uv.x)") && WGSL.contains("smoothstep(0.032, 0.064, uv.y)"));
	assert!(WGSL.contains("vec2<f32>(0.0625, 0.032), vec2<f32>(0.9375, 0.968)"), "the window must match frame_at's clamp");
}

#[test]
fn the_loop_is_seamless_and_every_step_is_one_solver_step() {
	// Mean |change of opacity| between consecutive frames: the wrap (255 -> 0) is no bigger than the steps beside it, and the steps are even
	// (one solver step each: no crossfade of far apart frames, which read as a dissolve = low frame rate).
	let step = |a: usize, b: usize| (0..256).flat_map(|y| (0..128).map(move |x| (y, x))).map(|(y, x)| (texel(a, x, y)[1] as f32 - texel(b, x, y)[1] as f32).abs()).sum::<f32>() / (256.0 * 128.0);
	let n = FRAMES as usize;
	let steps: Vec<f32> = (0..n).step_by(5).map(|i| step(i, (i + 1) % n)).collect();
	let mean = steps.iter().sum::<f32>() / steps.len() as f32;
	let wrap = step(n - 1, 0);
	assert!(wrap < 1.3 * mean && wrap > 0.7 * mean, "wrap {wrap} vs mean step {mean}");
	assert!(steps.iter().all(|&s| s < 1.5 * mean && s > 0.5 * mean), "{steps:?}");
	// A frame four steps on differs clearly more than one step on (the old atlas's spacing was these four steps).
	assert!(step(100, 104) > 1.4 * step(100, 101));
}

#[test]
fn the_background_mip_chain_is_the_one_made_inline() {
	prewarm();
	prewarm(); // idempotent
	let bg = take_mips();
	let inline = mips();
	assert_eq!(bg.len(), inline.len());
	assert!(bg.iter().zip(&inline).all(|(a, b)| a == b), "same bytes");
}
