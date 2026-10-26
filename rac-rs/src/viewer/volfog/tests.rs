use super::*;

/// The fog shader compiles (naga) with its three entry pairs and validates.
#[test]
fn volfog_shader_parses() {
	let m = naga::front::wgsl::parse_str(&VolFog::source()).expect("volfog shader");
	let names: Vec<_> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
	for n in ["vs_full", "fs_march", "fs_resolve", "fs_apply"] {
		assert!(names.contains(&n), "{n}");
	}
	let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
	v.validate(&m).expect("volfog shader validates");
}

/// The Rust uniform is the WGSL block: 16-byte multiples, 16 members of the sizes the block lists.
#[test]
fn uniform_size_matches_block() {
	assert_eq!(std::mem::size_of::<Uniform>(), 4 * 16 * 2 + 3 * 64 + 17 * 16);
	assert_eq!(std::mem::size_of::<Uniform>() % 16, 0);
}

#[test]
fn quality_levels() {
	assert_eq!(quality(0), None);
	assert_eq!(quality(1).map(|q| (q.div, q.shadows)), Some((2, false)));
	assert!(quality(3).unwrap().steps > quality(2).unwrap().steps);
	assert_eq!(quality(3).unwrap().div, 1);
}
