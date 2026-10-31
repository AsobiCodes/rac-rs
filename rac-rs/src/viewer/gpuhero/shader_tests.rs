//! The hero build shader (`SHADER`) parses and validates (naga) and its parameter struct `P` is as large as `HeroParams`, so a field
//! added on one side only fails here and not as a driver error at start.

use super::*;

#[test]
fn the_hero_build_shader_validates_and_matches_the_parameter_block() {
	let m = naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("hero build: {}", e.emit_to_string(SHADER)));
	naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("the hero build shader validates");
	let span = m
		.types
		.iter()
		.find_map(|(_, t)| match (&t.name, &t.inner) {
			(Some(n), naga::TypeInner::Struct { span, .. }) if n == "P" => Some(*span),
			_ => None,
		})
		.expect("struct P in the shader");
	assert_eq!(span as usize, std::mem::size_of::<HeroParams>(), "P and HeroParams differ in size");
}

/// A frame of each self-check kind has its own number; the loose-wrench kind is the fourth bit.
#[test]
fn the_loose_wrench_frame_is_its_own_self_check_kind() {
	assert_eq!(kind(false, false, false, false), 0);
	assert_eq!(kind(true, false, false, false), 1);
	assert_eq!(kind(false, true, true, false), 6);
	assert_eq!(kind(false, false, false, true), 8);
	assert_eq!(kind(true, true, true, true), KINDS - 1);
	assert_eq!(kind_name(8), "loose");
	assert_eq!(kind_name(9), "blend+loose");
	assert_eq!(kind_name(0), "plain");
}
