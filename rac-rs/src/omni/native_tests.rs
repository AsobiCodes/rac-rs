//! Unit tests of the native-mod host (ABI 1.1): the pure helpers, the disc rule, the function table. Neither the DLL nor a machine is needed.

use super::binding::Binding;
use super::cabi::{Host, Value, HOST_SIZE_V1, HOST_SIZE_V1_1, HOST_SIZE_V1_2, HOST_SIZE_V1_3};
use super::funcs::{candidates, find_in, find_spec, parse_funcs, CODE_LO};
use super::json::parse;
use super::layout::{disc_string, pal_as_ntsc};
use super::load::{find_mods, is_native_manifest};
use super::native_host::{loaded_level_of, place_args, relocate_in, stack_alloc, Placement};
use ee_oracle::hs_reloc::Reloc;

const BINDING: &str = include_str!("rac1.json");

#[test]
fn the_abi_1_1_host_has_the_seven_native_entries_after_capture_mouse() {
	assert_eq!(std::mem::size_of::<Host>(), super::cabi::HOST_SIZE_V1_6);
	assert_eq!(HOST_SIZE_V1_1, 256);
	assert_eq!(HOST_SIZE_V1_2, 296);
	assert_eq!(HOST_SIZE_V1_3, 312);
	assert_eq!(std::mem::offset_of!(Host, draw_submit), HOST_SIZE_V1_1);
	assert_eq!(std::mem::offset_of!(Host, ram_ptr), HOST_SIZE_V1);
	assert_eq!(std::mem::offset_of!(Host, stack_pop), HOST_SIZE_V1_1 - 8);
}

#[test]
fn stack_room_is_16_aligned_below_sp_and_refused_when_there_is_none() {
	let (new, old) = stack_alloc(0x01ff_ff00, 100).unwrap();
	assert_eq!((new, old), (0x01ff_fe90, 0x01ff_ff00));
	assert_eq!(new % 16, 0);
	assert!(new + 100 <= old);
	assert_eq!(stack_alloc(0x01ff_ff00, 0), None);
	assert_eq!(stack_alloc(0x0010_0010, 0x100), None);
	assert_eq!(stack_alloc(0x0010_0010, 0x20_0000), None);
	assert_eq!(stack_alloc(0x0400_0000, 16), None);
}

#[test]
fn call_arguments_go_to_the_integer_and_float_registers_each_in_order() {
	let p = place_args(&[Value::int(-5), Value::float(1.5), Value::int(0xffff_fff0), Value::float(-2.0), Value::int(0x1_0000_0000)]).unwrap();
	assert_eq!(p, Placement { ints: vec![0xffff_ffff_ffff_fffb, 0xffff_ffff_ffff_fff0, 0x1_0000_0000], floats: vec![0x3fc0_0000, 0xc000_0000] });
	assert!(place_args(&[Value::int(1); 8]).is_some());
	assert!(place_args(&[Value::int(1); 9]).is_none());
	assert!(place_args(&[Value::float(1.0); 9]).is_none());
	assert_eq!(place_args(&[]).unwrap(), Placement { ints: vec![], floats: vec![] });
}

#[test]
fn the_loaded_level_is_the_level_word_or_minus_one() {
	assert_eq!((loaded_level_of(0), loaded_level_of(3), loaded_level_of(18)), (0, 3, 18));
	assert_eq!((loaded_level_of(-1), loaded_level_of(19), loaded_level_of(0x100000)), (-1, -1, -1));
}

#[test]
fn executable_globals_stay_and_overlay_data_goes_through_the_level_map() {
	assert_eq!(relocate_in(false, 3, 0x13_C940), Some(0x13_C940));
	assert_eq!(relocate_in(false, -1, 0x15_ED64), Some(0x15_ED64));
	// lock-strafe's three globals (reference NTSC-U level-0 numbers): level mode, camera yaw, camera pan.
	assert_eq!(relocate_in(false, 3, 0x15_F5C4), Some(0x15_F5A4));
	assert_eq!(relocate_in(false, 3, 0x16_6DD8), Some(0x16_6E58));
	assert_eq!(relocate_in(false, 3, 0x15_EF40), Some(0x15_EF40));
	assert_eq!(relocate_in(false, -1, 0x16_6DD8), None);
	assert!(relocate_in(true, 3, 0x16_6DD8).is_some());
}

#[test]
fn the_pal_disc_tells_native_mods_ntsc_and_script_mods_pal() {
	assert_eq!(disc_string(false, None, false), ("pal", None));
	let (d, note) = disc_string(false, None, true);
	assert_eq!(d, "ntsc");
	assert_eq!(note, Some("PAL v1.00 disc: native mods get the ntsc address layout (same data layout)"));
	assert_eq!(disc_string(true, None, false), ("ntsc", None));
	assert_eq!(disc_string(true, None, true).0, "ntsc");
	// The override wins, any case; anything else is ignored.
	assert_eq!(disc_string(false, Some("NTSC"), false).0, "ntsc");
	assert_eq!(disc_string(false, Some("pal"), true).0, "pal");
	assert_eq!(disc_string(true, Some("pal"), false).0, "pal");
	assert_eq!(disc_string(false, Some("bogus"), true).0, "ntsc");
}

#[test]
fn the_binding_for_a_pal_v1_disc_has_the_ntsc_addresses_in_its_pal_column() {
	let b = Binding::parse(&pal_as_ntsc(BINDING).unwrap()).unwrap();
	let orig = Binding::parse(BINDING).unwrap();
	assert_eq!((b.base("hero", false), b.base("hero", true)), (Some(0x13F350), Some(0x13F350)));
	let bolts = |b: &Binding| b.fields.iter().find(|f| f.name == "bolts").map(|f| (f.pal, f.ntsc));
	assert_eq!(bolts(&orig), Some((0x15EE98, 0x15ED98)));
	assert_eq!(bolts(&b), Some((0x15ED98, 0x15ED98)));
	// The hook symbols and everything else are untouched.
	assert_eq!(b.hooks.len(), orig.hooks.len());
	assert!(b.hooks.iter().zip(&orig.hooks).all(|(x, y)| x.id == y.id && x.pal == y.pal && x.ntsc == y.ntsc && x.signature == y.signature));
	assert_eq!(b.fields.len(), orig.fields.len());
}

#[test]
fn json_text_reads_back_equal() {
	let j = parse(r#"{"a":[1,2.5,true,null,"q\"\\\n\u0001x"],"b":{"c":"d"}}"#).unwrap();
	assert_eq!(parse(&j.to_text()).unwrap(), j);
	let b = parse(BINDING).unwrap();
	assert_eq!(parse(&b.to_text()).unwrap(), b);
}

#[test]
fn a_manifest_with_a_library_table_or_type_native_is_a_native_mod() {
	assert!(is_native_manifest(&parse(r#"{"id":"x","plugin":{"windows-x86_64":"lib/x.dll"}}"#).unwrap()));
	assert!(is_native_manifest(&parse(r#"{"id":"x","type":"native"}"#).unwrap()));
	assert!(!is_native_manifest(&parse(r#"{"id":"x","type":"script","scripts":["main.luau"]}"#).unwrap()));
}

#[test]
fn mod_folders_are_listed_with_their_ids_and_kind() {
	let dir = std::env::temp_dir().join(format!("omni_find_mods_{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	for (folder, manifest) in [("b-script", r#"{"id":"bee"}"#), ("a-native", r#"{"id":"ay","plugin":{"windows-x86_64":"x.dll"}}"#), ("c-broken", "{")] {
		std::fs::create_dir_all(dir.join(folder)).unwrap();
		std::fs::write(dir.join(folder).join("omni-mod.json"), manifest).unwrap();
	}
	let found = find_mods(&dir);
	let _ = std::fs::remove_dir_all(&dir);
	let got: Vec<(&str, &str, bool)> = found.iter().map(|m| (m.folder.as_str(), m.id.as_str(), m.native)).collect();
	assert_eq!(got, [("a-native", "ay", true), ("b-script", "bee", false), ("c-broken", "c-broken", false)]);
}

#[test]
fn a_folder_with_only_an_openrac_manifest_is_listed_and_omni_mod_json_wins() {
	let dir = std::env::temp_dir().join(format!("omni_find_compat_{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	// compat: OpenRAC's manifest file name
	for (folder, file, manifest) in [("old", "openrac-mod.json", r#"{"id":"old"}"#), ("both", "openrac-mod.json", r#"{"id":"compat"}"#), ("both", "omni-mod.json", r#"{"id":"new"}"#)] {
		std::fs::create_dir_all(dir.join(folder)).unwrap();
		std::fs::write(dir.join(folder).join(file), manifest).unwrap();
	}
	let found = find_mods(&dir);
	let _ = std::fs::remove_dir_all(&dir);
	let got: Vec<(&str, &str)> = found.iter().map(|m| (m.folder.as_str(), m.id.as_str())).collect();
	assert_eq!(got, [("both", "new"), ("old", "old")]);
}

#[test]
fn the_built_in_function_table_has_lock_strafes_six_functions_on_both_numberings() {
	let specs = parse_funcs(include_str!("rac1_funcs.txt"));
	// Lock-strafe's six, the seven overlay functions of the big-mod binding (stage B2), the seven of stage P2a and moby_attach_to_joint (P2a2).
	assert_eq!(specs.len(), 21);
	assert!(specs.iter().all(|s| s.reloc.len > 0 && s.reloc.mhash != 0));
	for (ntsc, pal, name) in [(0x1f9dc8, 0x1f9f90, "fast_cos"), (0x1f9de0, 0x1f9fa8, "fast_sin"), (0x2e8568, 0x2e9a18, "camera_slide"), (0x2e8590, 0x2e9a40, "camera_spring"), (0x2e8648, 0x2e9af8, "camera_pan_off"), (0x2e8918, 0x2e9dc8, "camera_look_along")] {
		assert_eq!(find_spec(ntsc, true).map(|s| s.name.as_str()), Some(name));
		assert_eq!(find_spec(pal, false).map(|s| s.name.as_str()), Some(name));
	}
	assert!(find_spec(0x2e9a18, true).is_none(), "a PAL number is not an NTSC number");
	assert!(find_spec(0x1234, true).is_none());
}

#[test]
fn a_function_is_found_by_its_code_wherever_the_level_put_it() {
	const JR_RA: u32 = 0x03e0_0008;
	// addiu sp,sp,-16 ; addiu a0,a0,<imm> ; jr ra ; addiu sp,sp,16 (the immediate of word 1 is masked)
	let func = |imm: u32| [0x27bd_fff0, 0x2484_0000 | imm, JR_RA, 0x27bd_0010];
	let mut r = Reloc::parse("reloc t entry 0x100 len 0x10\nmhash 0x0\nmask 0x4 0xffff\n").unwrap();
	let f0 = func(0x1234);
	r.mhash = r.masked_hash_by(0, |a| f0[(a / 4) as usize]);
	let mut words = vec![0, 0, JR_RA, 0];
	let first = CODE_LO + 4 * words.len() as u32;
	words.extend(func(0x1234));
	words.extend([0, 0, 0, 0]);
	assert_eq!(find_in(&words, CODE_LO, &r), Some(first));
	// Another level: the same code with another immediate, somewhere else, with padding after the previous function's return.
	let mut other = vec![0, JR_RA, 0, 0, 0, 0];
	let second = CODE_LO + 4 * other.len() as u32;
	other.extend(func(0x5678));
	assert_eq!(find_in(&other, CODE_LO, &r), Some(second));
	// A different function of the same length does not match; neither does a function cut off by the end.
	let different = [0x27bd_fff0, 0x2484_0000, JR_RA, 0x27bd_0020];
	assert_eq!(find_in(&different, CODE_LO, &r), None);
	assert_eq!(find_in(&func(1)[..3], CODE_LO, &r), None);
	// Not after a `jr ra`: found by the second pass.
	let mut bare = vec![7, 7, 7, 7];
	bare.extend(func(0x42));
	assert_eq!(find_in(&bare, CODE_LO, &r), Some(CODE_LO + 16));
}

#[test]
fn start_candidates_follow_a_return_with_up_to_three_padding_words() {
	const JR_RA: u32 = 0x03e0_0008;
	let words = [JR_RA, 0, 5, 6, JR_RA, 0, 0, 0, 0, 9, 9];
	let strict = candidates(&words, 0x100, true);
	assert!(strict.contains(&(0x100 + 8)), "right after the delay slot");
	assert!(strict.contains(&(0x100 + 4 * 9)), "after three padding words");
	assert!(!strict.contains(&(0x100 + 4 * 10)), "too far after the return");
	assert!(!strict.contains(&0x100), "the first word has nothing before it");
	assert_eq!(candidates(&words, 0x100, false).len(), words.len());
}
