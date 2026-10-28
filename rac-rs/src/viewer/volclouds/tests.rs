use super::*;

fn validate(src: &str, what: &str, entries: &[&str]) -> naga::Module {
	let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{what}: {}", e.emit_to_string(src)));
	let names: Vec<_> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
	for n in entries {
		assert!(names.contains(n), "{what}: {n}");
	}
	let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
	v.validate(&m).unwrap_or_else(|e| panic!("{what} validates: {e:?}"));
	m
}

/// The cloud shader compiles (naga) with its entry points and validates.
#[test]
fn volclouds_shader_parses() {
	validate(&VolClouds::source(), "volclouds", &["vs_full", "fs_cloud", "fs_resolve", "fs_apply_clouds"]);
}

/// The sky shaders (reduction, backdrop after the level shader's head) and the noise generator validate.
#[test]
fn sky_and_noise_shaders_parse() {
	validate(sky::REDUCE, "sky model", &["vs_full", "fs_reduce", "fs_latlong"]);
	let head = &crate::viewer::SHADER[..crate::viewer::SHADER.find("struct VOut").expect("struct VOut")];
	let src = format!("{head}{}{}", sky::LOOKUP.replace("SKYTEX", "tex").replace("SKYSAMP", "samp"), sky::BACKDROP);
	validate(&src, "sky backdrop", &["vs_backdrop", "fs_backdrop"]);
	validate(noise3d::Noise::wgsl(), "cloud noise", &["cs_shape", "cs_detail", "cs_weather"]);
}

/// The Rust uniform is the WGSL block `C`: same size (a mismatch is a wgpu validation panic at the first cloud draw).
#[test]
fn uniform_size_matches_block() {
	assert_eq!(std::mem::size_of::<Uniform>(), 2 * 64 + 18 * 16);
	let m = naga::front::wgsl::parse_str(&VolClouds::source()).expect("volclouds shader");
	let span = m.types.iter().find(|(_, t)| t.name.as_deref() == Some("C")).map(|(_, t)| t.inner.size(m.to_ctx())).expect("struct C");
	assert_eq!(span as usize, std::mem::size_of::<Uniform>(), "Rust Uniform vs WGSL struct C");
}

#[test]
fn quality_levels() {
	assert_eq!(params::quality(0), None);
	let (l, m, h) = (params::quality(1).unwrap(), params::quality(2).unwrap(), params::quality(3).unwrap());
	assert!(l.steps < m.steps && m.steps < h.steps && l.light <= m.light && m.light <= h.light && l.div > m.div && m.div >= h.div);
	assert!([l, m, h].iter().all(|q| q.div % 2 == 0), "the grid divides the half-size depth grid evenly");
}

#[test]
fn closed_levels_keep_the_retail_sky() {
	for l in [2, 6, 13, 15, 17] {
		assert!(params::look(l).is_none(), "level {l}");
	}
	for l in [0, 1, 3, 4, 5, 7, 8, 9, 10, 11, 12, 14, 16, 18] {
		let k = params::look(l).unwrap_or_else(|| panic!("level {l}"));
		assert!(k.top > k.base && k.base > 0.0 && [k.cu, k.sc, k.ac, k.ci, k.cc, k.tower].iter().all(|c| (0.0..=1.0).contains(c)) && k.mid > k.top && k.high > k.mid, "level {l}");
	}
}

#[test]
fn the_sun_never_sets_below_the_floor() {
	for v in [[0.3, 0.2, 0.9], [0.6, 0.0, -0.4], [0.0, 0.0, -1.0], [-0.5, 0.5, 0.0]] {
		let s = params::sun_dir(None, v);
		let len = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
		assert!((len - 1.0).abs() < 1e-4 && s[2] >= 0.06 - 1e-4, "{v:?} -> {s:?}");
	}
	assert_eq!(params::sun_dir(Some([0.0, 0.0, 2.0]), [1.0, 0.0, 0.0]), [0.0, 0.0, 1.0]);
}

/// Each capture face looks along its forward axis: the forward direction lands in the middle of the face, in front.
#[test]
fn capture_faces_look_forward() {
	let fw = [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0f32]];
	for (k, f) in fw.iter().enumerate() {
		let m = skymodel::face_matrix(k);
		let clip: Vec<f32> = (0..4).map(|r| (0..3).map(|c| m[c][r] * f[c]).sum()).collect();
		assert!(clip[0].abs() < 1e-6 && clip[1].abs() < 1e-6 && clip[3] > 0.5, "face {k}: {clip:?}");
	}
}
