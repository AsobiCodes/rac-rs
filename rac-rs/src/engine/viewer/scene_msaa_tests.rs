//! Lane noaa: the scene sample count's pure parts (scene_msaa.rs). The count is injected (`with_count`): no test reads the options
//! file or RAC_SCENE_MSAA.
use super::*;

fn validate(src: &str, what: &str) {
	let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{what}: {}", e.emit_to_string(src)));
	let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
	v.validate(&m).unwrap_or_else(|e| panic!("{what} validates: {e:?}"));
}

#[test]
fn decide_follows_env_then_row_and_upscaler() {
	assert_eq!(decide(None, 0, true, true), 4);
	assert_eq!(decide(None, 1, false, true), 4, "the upscaler cannot run = 4x");
	assert_eq!(decide(None, 1, true, true), 1);
	assert_eq!(decide(Some("1"), 0, false, true), 1);
	assert_eq!(decide(Some("4"), 1, true, true), 4);
	assert_eq!(decide(Some("1"), 1, true, false), 4, "no surface copy = 4x");
	assert_eq!(index(None), DEFAULT);
	assert_eq!(index(Some(" Upscaler_Off ")), 1);
}

/// The injected count reaches `msaa`, `wgsl`, `copy_usage` and `ms_size`; the default is 4 (today's picture).
#[test]
fn injected_count_drives_the_helpers() {
	assert_eq!(msaa(), FULL);
	assert_eq!(copy_usage(), wgpu::TextureUsages::empty());
	assert_eq!(ms_size(640, 360), (640, 360));
	let src = "var d: texture_depth_multisampled_2d;";
	assert!(matches!(wgsl(Cow::Borrowed(src)), Cow::Borrowed(_)), "4x: the same text");
	with_count(1, || {
		assert_eq!(msaa(), 1);
		assert!(copy_usage().contains(wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST));
		assert_eq!(ms_size(640, 360), (1, 1));
		assert_eq!(wgsl(Cow::Borrowed(src)), "var d: texture_depth_2d;");
	});
	assert_eq!(msaa(), FULL);
}

/// The 1-sample variant of a shader reading the multisampled depth (sample 0, a sample loop, textureNumSamples) validates.
#[test]
fn single_sample_variant_validates() {
	let src = "@group(0) @binding(0) var d: texture_depth_multisampled_2d;\nconst SAMPLES: i32 = 4;\n@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {\n\tvar z = textureLoad(d, vec2<i32>(p.xy), 0);\n\tfor (var s = 1; s < SAMPLES; s++) { z = min(z, textureLoad(d, vec2<i32>(p.xy), s)); }\n\tfor (var s = 0; s < i32(textureNumSamples(d)); s++) { z = min(z, textureLoad(d, vec2<i32>(p.xy), s)); }\n\treturn vec4<f32>(z);\n}\n";
	validate(src, "4x");
	let one = to_single(src);
	assert!(!one.contains("multisampled") && one.contains("const SAMPLES: i32 = 1;"));
	validate(&one, "1x");
	assert!(bad_sample_reads(src, "sample loop test").is_empty());
	assert_eq!(bad_sample_reads("var m: texture_multisampled_2d<f32>;\nfn f() { for (var s = 0; s < 4; s++) { let c = textureLoad(m, vec2<i32>(0), s); } }", "x").len(), 1);
}

/// Every engine shader with a multisampled texture, as the renderer builds it.
fn ms_sources() -> Vec<(&'static str, String)> {
	let soft = |early| crate::viewer::volfog::soft::level_source(crate::viewer::SHADER, early);
	let level = |early| crate::viewer::texfilter::frag_w_fix(crate::viewer::metalfx::with_override(soft(early)));
	vec![
		("level", level(false)),
		("level early", level(true)),
		("level bindless", crate::viewer::bindless::shader_source(&level(false))),
		("level bindless early", crate::viewer::bindless::shader_source(&level(true))),
		("water", crate::viewer::waterfx::test_source()),
		("water shore map", crate::viewer::waterfx::SHORE_TEST_SOURCE.to_string()),
		("underwater", crate::viewer::underwater::SHADER.to_string() + &crate::viewer::waterfx::sky_wgsl(4, 5)),
		("post", crate::viewer::post::POST_SHADER.to_string()),
		("post grade", crate::viewer::post::graded_post_shader()),
		("metal contact", crate::viewer::metalenv::contact::CONTACT.to_string()),
		("fire", include_str!("firefx.wgsl").to_string()),
		("fire volumes", include_str!("firefx_vol.wgsl").to_string()),
		("firebook", crate::viewer::firebook::WGSL.to_string()),
		("fire step", crate::viewer::firesim::STEP_WGSL.to_string()),
		("fire draw", crate::viewer::firesim::DRAW_WGSL.to_string()),
		("ground fire", include_str!("groundfire.wgsl").to_string()),
		("upscale motion", include_str!("upscale/motion.wgsl").to_string()),
		("upscale slot", include_str!("upscale/slot_gpu.wgsl").to_string()),
		("reshade passes", include_str!("gpu/reshade/passes.wgsl").to_string()),
	]
}

/// The multisampled reads of `src` that take a sample other than 0 outside a `SAMPLES` / `textureNumSamples` loop: at 1 sample
/// they would read mips 1.. of a 1-mip texture (the rewrite in `to_single` only covers sample 0 and those two loop forms).
fn bad_sample_reads(src: &str, what: &str) -> Vec<String> {
	let names: Vec<&str> = src
		.lines()
		.filter(|l| l.contains("multisampled_2d"))
		.filter_map(|l| l.split("var ").nth(1)?.split(':').next().map(str::trim))
		.filter(|n| !n.is_empty())
		.collect();
	let mut bad = Vec::new();
	for n in names {
		let call = format!("textureLoad({n},");
		for (at, _) in src.match_indices(&call) {
			let last = last_arg(&src[at + call.len()..]);
			let before = &src[at.saturating_sub(400)..at];
			let looped = before.rfind("for (").is_some_and(|f| before[f..].contains("SAMPLES") || before[f..].contains("textureNumSamples("));
			if !matches!(last, "0" | "0i" | "0u") && !looped {
				bad.push(format!("{what}: textureLoad({n}, .., {last})"));
			}
		}
	}
	bad
}

/// The last argument of a call whose arguments start at `args` (after its first comma), at paren depth 0.
fn last_arg(args: &str) -> &str {
	let (mut depth, mut start) = (0i32, 0usize);
	for (i, c) in args.char_indices() {
		match c {
			'(' => depth += 1,
			')' if depth == 0 => return args[start..i].trim(),
			')' => depth -= 1,
			',' if depth == 0 => start = i + 1,
			_ => {}
		}
	}
	args[start..].trim()
}

/// Every engine shader with a multisampled texture validates as it is (4x) and as its 1-sample variant, and reads its
/// multisampled textures only in the forms `to_single` covers.
#[test]
fn engine_shaders_validate_at_one_sample() {
	let all = ms_sources();
	let mut bad = Vec::new();
	for (what, src) in &all {
		assert!(src.contains("multisampled_2d"), "{what} has a multisampled texture");
		validate(src, what);
		let one = with_count(1, || wgsl(Cow::Borrowed(src.as_str())).into_owned());
		assert!(!one.contains("multisampled"), "{what}");
		validate(&one, &format!("{what} (1 sample)"));
		bad.extend(bad_sample_reads(src, what));
	}
	assert!(bad.is_empty(), "{bad:#?}");
}

/// No engine shader with a multisampled texture is missing from `ms_sources` (a new one must be added there).
#[test]
fn every_multisampled_shader_is_listed() {
	let listed = ["firebook.wgsl", "firefx.wgsl", "firefx_vol.wgsl", "firesim.wgsl", "firesim_step.wgsl", "gpu/reshade/passes.wgsl", "groundfire.wgsl", "metalcontact.rs", "post.rs", "underwater.rs", "upscale/motion.wgsl", "upscale/slot_gpu.wgsl", "volfog/soft.rs", "waterfx/shader.rs", "waterfx/shore_map.rs", "waterfx/skymap.rs", "scene_msaa.rs", "scene_msaa_tests.rs"];
	let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/engine/viewer");
	let mut found = Vec::new();
	let mut stack = vec![root.clone()];
	while let Some(d) = stack.pop() {
		for e in std::fs::read_dir(&d).expect("viewer dir").flatten() {
			let p = e.path();
			if p.is_dir() {
				stack.push(p);
			} else if p.extension().is_some_and(|x| x == "rs" || x == "wgsl") && std::fs::read_to_string(&p).is_ok_and(|t| t.contains("multisampled_2d")) {
				found.push(p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"));
			}
		}
	}
	let missing: Vec<_> = found.iter().filter(|f| !listed.contains(&f.as_str())).collect();
	assert!(missing.is_empty(), "add these to ms_sources: {missing:?}");
}
