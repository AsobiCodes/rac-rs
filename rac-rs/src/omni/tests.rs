//! Unit tests of the OmniLoader host that need neither the DLL nor a machine.

use super::binding::{symbol_address, Binding, Sig, Ty};
use super::cabi::{host_size_for, Config, Host, OmniDrawCmd, OmniMobyQuery, OmniPoseOp, Value, HOST_SIZE_V1, HOST_SIZE_V1_1, HOST_SIZE_V1_2, HOST_SIZE_V1_3, HOST_SIZE_V1_4, HOST_SIZE_V1_6, VALUE_SIZE};
use super::json::{parse, Json};
use super::load::{plugins_json, status_lines};

const BINDING: &str = include_str!("rac1.json");

#[test]
fn abi_struct_sizes_match_the_header() {
	assert_eq!(std::mem::size_of::<Host>(), HOST_SIZE_V1_6);
	assert_eq!((HOST_SIZE_V1_2, HOST_SIZE_V1_3, HOST_SIZE_V1_4, HOST_SIZE_V1_6), (296, 312, 352, 360));
	assert_eq!(std::mem::offset_of!(Host, pose_follow), HOST_SIZE_V1_4);
	assert_eq!(std::mem::size_of::<OmniMobyQuery>(), 104);
	assert_eq!(std::mem::offset_of!(Host, moby_query), HOST_SIZE_V1_2);
	assert_eq!(std::mem::offset_of!(Host, pose_set), HOST_SIZE_V1_3);
	assert_eq!(std::mem::offset_of!(Host, pose_info), HOST_SIZE_V1_3 + 8);
	assert_eq!(std::mem::offset_of!(Host, pose_read), HOST_SIZE_V1_3 + 16);
	assert_eq!(std::mem::offset_of!(Host, pose_snapshot), HOST_SIZE_V1_3 + 24);
	assert_eq!(std::mem::offset_of!(Host, pose_debug), HOST_SIZE_V1_3 + 32);
	assert_eq!(std::mem::size_of::<OmniPoseOp>(), 48);
	assert_eq!(std::mem::offset_of!(OmniPoseOp, q), 16);
	assert_eq!(std::mem::offset_of!(OmniPoseOp, weight), 32);
	assert_eq!(std::mem::offset_of!(OmniPoseOp, reserved), 44);
	assert_eq!(std::mem::size_of::<Value>(), VALUE_SIZE);
	assert_eq!(host_size_for(5), HOST_SIZE_V1_4, "ABI 1.5 changes no struct size");
	assert_eq!(std::mem::size_of::<Config>(), 24);
	assert_eq!(std::mem::size_of::<OmniDrawCmd>(), 64);
}

#[test]
fn the_host_size_follows_the_dlls_minor_abi_step() {
	assert_eq!(host_size_for(0), HOST_SIZE_V1);
	assert_eq!(host_size_for(1), HOST_SIZE_V1_1);
	assert_eq!(host_size_for(1), 256);
	assert_eq!(host_size_for(2), HOST_SIZE_V1_2);
	assert_eq!(host_size_for(2), 296);
	assert_eq!(host_size_for(3), HOST_SIZE_V1_3);
	assert_eq!(host_size_for(3), 312);
	assert_eq!(host_size_for(4), HOST_SIZE_V1_4);
	assert_eq!(host_size_for(4), 352);
	assert_eq!(host_size_for(5), 352);
	assert_eq!(host_size_for(6), HOST_SIZE_V1_6);
	assert_eq!(host_size_for(9), 360);
}

#[test]
fn json_reads_objects_arrays_and_escapes() {
	let j = parse(r#"{"a":[1,2.5,true,null,"x\n\u0041"],"b":{"c":"d"}}"#).unwrap();
	assert_eq!(j.get("b").and_then(|b| b.get("c")).and_then(Json::str), Some("d"));
	let a = j.get("a").unwrap().arr();
	assert_eq!((a[0].clone(), a[1].clone(), a[2].bool()), (Json::Num(1.0), Json::Num(2.5), true));
	assert_eq!(a[4].str(), Some("x\nA"));
	assert!(parse("{\"a\":}").is_err());
	assert!(parse("[1,2] x").is_err());
}

#[test]
fn signatures_parse() {
	let s = Sig::parse("void(i32,f32,f32,f32)").unwrap();
	assert_eq!((s.ret, s.args), (Ty::Void, vec![Ty::I32, Ty::F32, Ty::F32, Ty::F32]));
	assert_eq!(Sig::parse("u32()").unwrap().args.len(), 0);
	assert!(Sig::parse("void(i32").is_err());
	assert!(Sig::parse("void(quux)").is_err());
}

#[test]
fn symbols_give_addresses() {
	assert_eq!(symbol_address("func_L00_002056D8"), Some(0x2056D8));
	assert_eq!(symbol_address("FUN_L00_00211e30"), Some(0x211e30));
	assert_eq!(symbol_address("func_002181F0"), Some(0x2181F0));
	assert_eq!(symbol_address("nothing"), None);
}

#[test]
fn the_built_in_binding_has_210_hooks_and_the_hero_base() {
	let b = Binding::parse(BINDING).unwrap();
	assert_eq!(b.hooks.len(), 210);
	assert!(b.hooks.len() <= super::hooks::MAX_HOOKS);
	let take = b.hook("rac1.weapon.take_ammo").unwrap();
	assert_eq!((take.entry(false), take.entry(true)), (Some(0x234638), Some(0x233db8)));
	assert!(Sig::parse(&take.signature).is_ok());
	assert_eq!((b.base("hero", false), b.base("hero", true)), (Some(0x13F450), Some(0x13F350)));
	assert!(b.has_field("hero.position.x", false) && b.has_field("bolts", true));
	assert!(!b.has_field("hero.nothing", false));
	assert!(b.hooks.iter().all(|h| Sig::parse(&h.signature).is_ok()));
	// The 17 engine hooks name their retail entry on both discs; the 135 moby update hooks have only a v2 pal symbol (the host finds them by code);
	// the 27 big-mod hooks (stage B2: damage, wrench, weapons, ammo, equip, vendor, pause page) and the 8 of stage B3
	// (damage.resolve, flame.create, 6 projectile creators `.fire`) follow them.
	assert!(b.hooks.iter().take(17).all(|h| h.entry(false).is_some() && h.entry(true).is_some()));
	assert!(b.hooks.iter().skip(17).take(135).all(|h| h.id.starts_with("rac1.moby.") && h.id.ends_with(".update")));
	assert_eq!(b.hooks.iter().skip(152).count(), 58);
	assert!(b.hooks.iter().skip(152).all(|h| h.id.starts_with("rac1.omni.") || h.id == "rac1.gold_bolt.count_spare"));
	let mut ids: Vec<&str> = b.hooks.iter().map(|h| h.id.as_str()).collect();
	ids.sort_unstable();
	ids.dedup();
	assert_eq!(ids.len(), b.hooks.len(), "hook ids are unique");
}

#[test]
fn plugins_json_lists_the_ids_in_order() {
	let t = plugins_json(&["a".to_string(), "b\"c".to_string()]);
	let j = parse(&t).unwrap();
	let ids: Vec<&str> = j.get("mods").unwrap().arr().iter().filter_map(|m| m.get("id").and_then(Json::str)).collect();
	assert_eq!(ids, ["a", "b\"c"]);
	assert!(j.get("mods").unwrap().arr().iter().all(|m| m.get("enabled").is_some_and(Json::bool)));
}

#[test]
fn status_lines_name_loaded_and_refused_mods() {
	let st = r#"{"mods":[{"id":"x","state":"loaded"},{"id":"y","state":"refused","reason":"needs hook point"},{"id":"z","state":"failed"}]}"#;
	let l = status_lines(st);
	assert_eq!(l[0], "omni: 1 mods loaded (x)");
	assert_eq!(l[1], "omni: mod y refused: needs hook point");
	assert_eq!(l[2], "omni: mod z failed: no reason given");
}

#[test]
fn loaded_count_counts_only_loaded_mods() {
	let st = r#"{"mods":[{"id":"x","state":"loaded"},{"id":"y","state":"refused"},{"id":"z","state":"loaded"}]}"#;
	assert_eq!(super::load::loaded_count(st), 2);
	assert_eq!(super::load::loaded_count("{}"), 0);
	assert_eq!(super::load::loaded_count("not json"), 0);
}

#[test]
fn the_built_in_hook_specs_parse_and_match_binding_hooks() {
	let specs = super::hooks::parse_specs(include_str!("rac1_reloc.txt"));
	let b = Binding::parse(BINDING).unwrap();
	// Every hook point of the binding has a spec (stage 2c: the last 6 were derived per level, see tools/make_spec.py).
	assert_eq!(specs.len(), b.hooks.len());
	for id in specs.keys() {
		let h = b.hook(id).unwrap_or_else(|| panic!("{id} is not in the binding"));
		assert!(super::hooks::sig_supported(&Sig::parse(&h.signature).unwrap()), "{id}");
	}
	assert!(b.hooks.iter().all(|h| specs.contains_key(&h.id)));
}

#[test]
fn ammo_pickup_is_the_moby_update_and_the_collect_helper_has_its_own_id() {
	// Stage AP1: the pickup moby update (504 bytes) jumps into an INLINED copy of the collect helper (480 bytes), so a hook on the helper alone
	// never saw a crate's ammo. `tools/hook_reach_check.py` finds such twins; this pins the two bindings.
	let specs = super::hooks::parse_specs(include_str!("rac1_reloc.txt"));
	let b = Binding::parse(BINDING).unwrap();
	assert_eq!(specs["rac1.omni.ammo.pickup"].len, 0x1f8);
	assert_eq!(specs["rac1.omni.ammo.collect"].len, 0x1dc);
	assert_eq!(Sig::parse(&b.hook("rac1.omni.ammo.pickup").unwrap().signature).unwrap().ret, Ty::Void);
	assert_eq!(Sig::parse(&b.hook("rac1.omni.ammo.collect").unwrap().signature).unwrap().ret, Ty::U32);
}

#[test]
fn register_values_round_trip_through_the_signature_types() {
	use super::bridge::{canon, int_of};
	assert_eq!(int_of(Ty::I32, canon(Ty::I32, -5)), -5);
	assert_eq!(canon(Ty::U32, 0xffff_fff0), 0xffff_ffff_ffff_fff0);
	assert_eq!(int_of(Ty::U32, 0xffff_ffff_ffff_fff0), 0xffff_fff0);
	assert_eq!(canon(Ty::U8, 0x1ff), 0xff);
}

#[test]
fn trace_range_parses_one_tick_or_a_span() {
	use super::trace::parse_range;
	assert_eq!(parse_range("290-312"), Some((290, 312)));
	assert_eq!(parse_range(" 300 "), Some((300, 300)));
	assert_eq!(parse_range("a-b"), None);
	assert_eq!(parse_range(""), None);
}

#[test]
fn the_functions_section_parses_and_every_function_has_a_spec() {
	let j = parse(BINDING).unwrap();
	let fns = j.get("functions").expect("the binding has a functions section").arr();
	assert!(fns.len() >= 7);
	let specs = super::funcs::parse_funcs(include_str!("rac1_funcs.txt"));
	let mut names = Vec::new();
	for f in fns {
		let name = f.get("name").and_then(Json::str).expect("a function name");
		assert!(Sig::parse(f.get("signature").and_then(Json::str).unwrap()).is_ok(), "{name}");
		let num = |disc: &str| symbol_address(f.get("symbols").and_then(|s| s.get(disc)).and_then(Json::str).unwrap_or_else(|| panic!("{name}: no {disc} symbol")));
		let (pal, ntsc) = (num("pal").unwrap(), num("ntsc").unwrap());
		let s = specs.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("{name} has no spec in rac1_funcs.txt"));
		assert_eq!((s.ntsc, s.pal), (ntsc, pal), "{name}: numbers differ from the spec header");
		names.push(name);
	}
	names.sort_unstable();
	names.dedup();
	assert_eq!(names.len(), fns.len(), "function names are unique");
	// No two game functions share a number on either disc.
	for (i, a) in specs.iter().enumerate() {
		assert!(specs.iter().skip(i + 1).all(|b| a.ntsc != b.ntsc && a.pal != b.pal), "{}: number shared", a.name);
	}
}

#[test]
fn field_names_are_unique_and_the_weapon_fields_exist() {
	let b = Binding::parse(BINDING).unwrap();
	let mut names: Vec<&str> = b.fields.iter().map(|f| f.name.as_str()).collect();
	names.sort_unstable();
	names.dedup();
	assert_eq!(names.len(), b.fields.len());
	for f in ["omni.weapon.equipped", "omni.weapon.quick_select", "omni.weapon.tesla_claw.ammo", "omni.weapon.pyrocitor.gold", "omni.weapon.wrench.have"] {
		assert!(b.has_field(f, true), "{f}");
	}
	let ammo = b.fields.iter().find(|f| f.name == "omni.weapon.bomb_glove.ammo").unwrap();
	assert_eq!(ammo.ntsc, 0x13D428 + 4 * 10);
}

#[test]
fn stage_b3_hooks_have_their_signatures_and_are_the_last_eight() {
	let b = Binding::parse(BINDING).unwrap();
	let last8: Vec<&str> = b.hooks.iter().skip(179).take(8).map(|h| h.id.as_str()).collect();
	assert_eq!(last8, [
		"rac1.omni.damage.resolve", "rac1.omni.weapon.flame.create", "rac1.omni.weapon.bomb_glove.fire", "rac1.omni.weapon.mine_glove.fire",
		"rac1.omni.weapon.glove_of_doom.fire", "rac1.omni.weapon.devastator.fire", "rac1.omni.weapon.visibomb.fire", "rac1.omni.weapon.blaster.fire"
	]);
	let resolve = Sig::parse(&b.hook("rac1.omni.damage.resolve").unwrap().signature).unwrap();
	assert_eq!((resolve.args.len(), resolve.ret), (8, Ty::U32), "eight parameters, a0..a3 and t0..t3");
	let blaster = Sig::parse(&b.hook("rac1.omni.weapon.blaster.fire").unwrap().signature).unwrap();
	assert_eq!(blaster.args.iter().filter(|t| t.is_float()).count(), 4);
	let flame = Sig::parse(&b.hook("rac1.omni.weapon.flame.create").unwrap().signature).unwrap();
	assert_eq!((flame.args[0], flame.args.len()), (Ty::F32, 4));
}

#[test]
fn max_ammo_fields_sit_in_the_overlay_window_and_the_retail_caps_are_the_notes() {
	use super::window::{translate, OVERLAY_WINDOW};
	let b = Binding::parse(BINDING).unwrap();
	// (weapon, item id, retail cap read from fg1/0_0001 on Veldin, omni_weapons.md): the field of each, mapped on Veldin (level 0)
	let weapons = [("bomb_glove", 10), ("devastator", 11), ("visibomb", 13), ("blaster", 15), ("pyrocitor", 16), ("mine_glove", 17), ("tesla_claw", 19), ("glove_of_doom", 20), ("ryno", 23), ("drone_device", 24), ("decoy", 25)];
	for (w, id) in weapons {
		let f = b.fields.iter().find(|f| f.name == format!("omni.weapon.{w}.max_ammo")).unwrap_or_else(|| panic!("{w}"));
		assert_eq!((f.ntsc, f.pal), (OVERLAY_WINDOW + 0x1C_40B0 + id * 0x18 + 0xE, f.ntsc), "{w}");
		assert_eq!(translate(false, 0, f.ntsc, 2), Some(0x1C_4030 + id * 0x18 + 0xE), "{w} on Veldin");
		assert!(b.has_field(&f.name, true));
	}
	assert!(!b.fields.iter().any(|f| f.name == "omni.weapon.walloper.max_ammo"), "weapons without ammo have no max_ammo");
}

#[test]
fn ammo_box_fields_map_to_the_hud_element_array_of_every_level_on_both_discs() {
	use super::window::translate;
	let b = Binding::parse(BINDING).unwrap();
	let field = |n: &str| b.fields.iter().find(|f| f.name == n).unwrap_or_else(|| panic!("{n}")).clone();
	let (max, def_max, id) = (field("omni.hud.ammo_box.max"), field("omni.hud.ammo_box.def_max"), field("omni.hud.ammo_box.id"));
	// the start of the 4-slot HUD element array (slot 0 = the ammo box) per level 0..18, found by its static slot anchors in each disc's level data
	let pal = [0x17dc50, 0x17e050, 0x17e1d0, 0x17dcd0, 0x17dd50, 0x17e0d0, 0x17e350, 0x17dc50, 0x17e3d0, 0x17dd50, 0x17e050, 0x17e5d0, 0x17e050, 0x17ded0, 0x17e350, 0x17e1d0, 0x17e050, 0x17e4d0, 0x17e5d0];
	let usa = [0x17dc50, 0x17e0d0, 0x17e250, 0x17dd50, 0x17ddd0, 0x17e0d0, 0x17e3d0, 0x17dcd0, 0x17e450, 0x17ddd0, 0x17e0d0, 0x17e650, 0x17e0d0, 0x17df50, 0x17e350, 0x17e250, 0x17e0d0, 0x17e550, 0x17e650];
	for (is_usa, bases) in [(false, pal), (true, usa)] {
		for (level, base) in bases.iter().enumerate() {
			let at = |f: &super::binding::FieldDef, off: u32| translate(is_usa, level as i32, if is_usa { f.ntsc } else { f.pal }, 4).map(|a| (a, base + off));
			for (f, off) in [(&max, 0x08), (&def_max, 0x28), (&id, 0x20)] {
				let (got, want) = at(f, off).unwrap_or_else(|| panic!("{} unmapped (usa {is_usa}, level {level})", f.name));
				assert_eq!(got, want, "{} (usa {is_usa}, level {level})", f.name);
			}
		}
	}
	assert!(b.has_field("omni.hud.ammo_box.max", true) && b.has_field("omni.hud.ammo_box.def_max", true) && b.has_field("omni.hud.ammo_box.id", true));
	// outside a level (title screen) the array is not mapped
	assert_eq!(translate(false, 99, max.pal, 4), None);
}

#[test]
fn structs_and_pvars_sections_are_read_and_old_parsers_ignore_them() {
	let b = Binding::parse(BINDING).unwrap();
	let names: Vec<&str> = b.structs.iter().map(|s| s.0.as_str()).collect();
	assert_eq!(
		names,
		["moby", "enemy", "hit", "hit_desc", "pvar0", "component", "blaster_shot", "ryno_rocket", "devastator_rocket", "bomb", "mine", "doom_agent", "visibomb", "gold_case", "matrix4", "decoy", "drone",
			"held_taunter", "held_morph", "tesla_claw", "flame", "vec4", "hud_health"]
	);
	assert_eq!((b.struct_offset("hit", "attacker"), b.struct_offset("hit", "strength")), (Some(0x20), Some(0x2c)));
	assert_eq!(b.struct_offset("hit_desc", "damage"), Some(0x1c));
	// B3b: the moby position (proven on the Kerwan fight probe) and the Weapons page highlight field in the overlay window
	assert_eq!((b.struct_offset("moby", "x"), b.struct_offset("moby", "y"), b.struct_offset("moby", "z")), (Some(0x10), Some(0x14), Some(0x18)));
	let hl = b.fields.iter().find(|f| f.name == "omni.menu.weapons_page.highlight_class").expect("highlight_class");
	assert_eq!((hl.pal, hl.ntsc), (super::window::OVERLAY_WINDOW + 0x1B_9E10, super::window::OVERLAY_WINDOW + 0x1B_9E10));
	assert_eq!(super::window::translate(false, 0, super::window::OVERLAY_WINDOW + 0x1B_9E10, 4), Some(0x1B_9E10));
	assert_eq!(super::window::translate(false, 5, super::window::OVERLAY_WINDOW + 0x1B_9E10, 4), Some(0x1B_A290));
	// a binding without the two sections parses (unknown keys are ignored in both directions)
	let plain = Binding::parse(r#"{"game":"rac1","hooks":[{"id":"a.b"}],"x_new_section":[1]}"#).unwrap();
	assert!(plain.structs.is_empty() && plain.pvars.is_empty() && plain.hooks.len() == 1);
	// every struct field has a distinct name and the moby offsets stay inside its 0x100 bytes
	for (n, fields) in &b.structs {
		let mut f: Vec<&str> = fields.iter().map(|x| x.name.as_str()).collect();
		f.sort_unstable();
		f.dedup();
		assert_eq!(f.len(), fields.len(), "{n}");
	}
	assert!(b.struct_fields("moby").unwrap().iter().all(|f| f.offset < 0x100));
}

#[test]
fn call_original_flags_a_missing_result_in_the_reserved_bit() {
	// ABI 1.5: Some(7) -> plain 7 (reserved 0); None -> zero value with bit 0 of `reserved` set.
	let some = super::hooks::ret_value(Some(Value::int(7)));
	assert_eq!((some.is_float, some.reserved, some.i), (0, 0, 7));
	let f = super::hooks::ret_value(Some(Value::float(1.5)));
	assert_eq!((f.is_float, f.reserved, f.f), (1, 0, 1.5));
	let none = super::hooks::ret_value(None);
	assert_eq!((none.is_float, none.reserved, none.i), (0, super::cabi::OMNI_VALUE_NO_RESULT, 0));
	assert_eq!(super::cabi::OMNI_VALUE_NO_RESULT, 1);
}

#[test]
fn json_refuses_nesting_past_the_cap_and_numbers_that_are_not_finite() {
	// A hostile manifest of thousands of `[` is an error, not a stack overflow.
	assert!(parse(&"[".repeat(10_000)).is_err());
	assert!(parse(&format!("{}{}", "{\"a\":".repeat(10_000), "1")).is_err());
	// Real nesting is far below the cap.
	assert!(parse(&format!("{}{}", "[".repeat(100), "]".repeat(100))).is_ok());
	// A number that overflows f64 would be written as `inf` (not JSON) into settings.json: refused when read.
	assert!(parse(r#"{"max":1e999}"#).is_err());
	assert!(parse(r#"{"min":-1e999}"#).is_err());
	let j = parse(r#"{"max":1e300}"#).unwrap();
	assert_eq!(parse(&j.to_text()).unwrap(), j);
}

#[test]
fn only_the_abi_the_host_speaks_is_accepted() {
	use super::cabi::{abi_check, ABI_VERSION};
	assert!(abi_check(ABI_VERSION).is_ok());
	assert!(abi_check(ABI_VERSION + 1).unwrap_err().contains("ABI 2"), "a newer major has other struct layouts");
	assert!(abi_check(0).is_err());
}

#[test]
fn a_failed_load_switches_every_installed_hook_back_to_the_game() {
	use std::sync::atomic::Ordering;
	super::hooks::slot(249).active.store(true, Ordering::SeqCst);
	super::hooks::deactivate_all();
	assert!(!super::hooks::slot(249).active.load(Ordering::SeqCst));
}
