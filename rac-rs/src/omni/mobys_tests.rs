//! Tests of the moby walk (mobys.rs) on the Kerwan savestate fg1/3_0086; the expected numbers come from an independent python walk of the same
//! RAM image (stage P2a). The tests skip with a message when the savestate is not on this machine.

use super::binding::{Binding, MobysDef};
use super::cabi::OmniMobyQuery;
use super::mobys::*;
use super::window;

const BINDING: &str = include_str!("rac1.json");
const BASE: u32 = 0x01EA_BA00;
const TAIL_SLOT: u32 = 0x01EE_E600;

fn load() -> Option<Vec<u8>> {
	let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../oracle/states/fg1/3_0086/eeMemory.bin");
	// a lane copy (<dev>/scratch_x/rac-rs) finds the dev savestates two levels up
	let alt = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/states/fg1/3_0086/eeMemory.bin");
	match std::fs::read(&p).or_else(|_| std::fs::read(&alt)) {
		Ok(v) if v.len() >= 0x200_0000 => Some(v),
		_ => {
			eprintln!("skipped: savestate oracle/states/fg1/3_0086 not found");
			None
		}
	}
}

fn def() -> MobysDef {
	Binding::parse(BINDING).unwrap().mobys.expect("mobys section")
}

fn level(ram: &[u8]) -> i32 {
	i32::from_le_bytes(ram[0x15_ED84..0x15_ED88].try_into().unwrap())
}

fn q(flags: u32, classes: &[i32]) -> OmniMobyQuery {
	let mut q = OmniMobyQuery { flags, n_classes: classes.len() as u32, max: 256, ..Default::default() };
	q.classes[..classes.len()].copy_from_slice(classes);
	q
}

fn run(d: &MobysDef, ram: &[u8], q: &OmniMobyQuery) -> Vec<u32> {
	let lv = level(ram);
	let tr = move |a: u32, l: u32| window::translate(false, lv, a, l);
	super::mobys::query(d, ram, &tr, q).unwrap()
}

#[test]
fn pool_and_counts() {
	let Some(ram) = load() else { return };
	let d = def();
	assert_eq!(level(&ram), 3);
	let lv = level(&ram);
	let tr = move |a: u32, l: u32| window::translate(false, lv, a, l);
	let p = pool(&d, &ram, &tr).unwrap();
	assert_eq!((p.start, p.pool_start, p.end, p.tail), (0x01EA_BA00, 0x01ED_EC00, 0x01EE_EB00, TAIL_SLOT));
	assert_eq!((p.end - p.start) / d.stride, 1073);
	assert_eq!(run(&d, &ram, &q(0, &[])).len(), 256.min(783));
	// 783 live: the ABI cap is 256, so count with class filters
	assert_eq!(run(&d, &ram, &q(0, &[500])).len(), 141);
	assert_eq!(run(&d, &ram, &q(0, &[13])).len(), 102);
	assert_eq!(run(&d, &ram, &q(0, &[500, 13])).len(), 243);
}

#[test]
fn registry_and_enemy() {
	let Some(ram) = load() else { return };
	let d = def();
	assert_eq!(run(&d, &ram, &q(Q_REGISTRY, &[])).len(), 256.min(306));
	assert_eq!(run(&d, &ram, &q(Q_REGISTRY, &[500])).len(), 50);
	assert_eq!(run(&d, &ram, &q(Q_REGISTRY | Q_ENEMY, &[])).len(), 1);
	assert_eq!(run(&d, &ram, &q(Q_ENEMY, &[])).len(), 12);
}

#[test]
fn near_the_hero() {
	let Some(ram) = load() else { return };
	let d = def();
	// slot 0 is the hero moby: class 0, flags 0x4416, position (286.452, 110.650, 66.0)
	let at = |o: u32| f32::from_le_bytes(ram[(BASE + o) as usize..(BASE + o) as usize + 4].try_into().unwrap());
	assert_eq!(u16::from_le_bytes([ram[(BASE + 0xa6) as usize], ram[(BASE + 0xa7) as usize]]), 0);
	assert_eq!(u16::from_le_bytes([ram[(BASE + 0x34) as usize], ram[(BASE + 0x35) as usize]]), 0x4416);
	assert!((at(0x10) - 286.452).abs() < 0.01 && (at(0x14) - 110.650).abs() < 0.01 && (at(0x18) - 66.0).abs() < 0.01);
	let mut nq = q(Q_NEAR, &[]);
	nq.near = [at(0x10), at(0x14), at(0x18)];
	nq.radius = 5.0;
	let v = run(&d, &ram, &nq);
	assert_eq!(v.len(), 8);
	assert_eq!(&v[..4], &[0x01EA_BA00, 0x01EE_7200, 0x01EE_4300, 0x01EE_2100]);
	// max 0 = 32, max 300 = capped to 256
	nq.radius = 0.0;
	nq.max = 0;
	assert_eq!(run(&d, &ram, &nq).len(), 32);
	nq.max = 300;
	assert_eq!(run(&d, &ram, &nq).len(), 256);
}

#[test]
fn tail_marker_and_valid() {
	let Some(ram) = load() else { return };
	let d = def();
	let lv = level(&ram);
	let tr = move |a: u32, l: u32| window::translate(false, lv, a, l);
	let base = run(&d, &ram, &q(0, &[500])).len();
	// a state-0 slot of class 500 behind the tail marker does not count
	let mut r2 = ram.clone();
	let behind = (TAIL_SLOT + 0x100) as usize;
	r2[behind + 0x20] = 0;
	r2[behind + 0xa6..behind + 0xa8].copy_from_slice(&500i16.to_le_bytes());
	assert_eq!(run(&d, &r2, &q(0, &[500])).len(), base);
	// the marker slot itself set to state 0: the walk goes on behind it
	r2[(TAIL_SLOT + 0x20) as usize] = 0;
	r2[(TAIL_SLOT + 0xa6) as usize..(TAIL_SLOT + 0xa8) as usize].copy_from_slice(&500i16.to_le_bytes());
	assert!(run(&d, &r2, &q(0, &[500])).len() > base);
	// moby_valid
	assert_eq!(run_valid(&d, &ram, &tr, BASE), 1);
	assert_eq!(run_valid(&d, &ram, &tr, BASE + 2), 0);
	assert_eq!(run_valid(&d, &ram, &tr, TAIL_SLOT + 0x100), 0);
	assert_eq!(run_valid(&d, &ram, &tr, 0x0010_0000), 0);
	let mut r3 = ram.clone();
	r3[(BASE + 0x100 + 0x20) as usize] = 0xfd;
	assert_eq!(run_valid(&d, &r3, &tr, BASE + 0x100), 0);
	// no level: -1 / 0
	let tr99 = |a: u32, l: u32| window::translate(false, 99, a, l);
	let mut out = [0u32; 4];
	assert_eq!(run_query(&d, &ram, &tr99, &q(0, &[]), &mut out), -1);
	assert_eq!(run_valid(&d, &ram, &tr99, BASE), 0);
}

// ---- binding data of stages P2a / P2a2 (no savestate needed)

use super::binding::{Sig, Ty};
use super::json::{parse, Json};

#[test]
fn readonly_struct_fields_parse_and_an_old_binding_without_the_key_loads() {
	let b = Binding::parse(BINDING).unwrap();
	let ro = |st: &str, f: &str| b.struct_field(st, f).unwrap_or_else(|| panic!("{st}.{f}")).readonly;
	for f in ["class", "state", "pvar", "flags", "class_ptr", "next", "index"] {
		assert!(ro("moby", f), "moby.{f} is readonly");
	}
	for f in ["x", "y", "z", "rot_x", "update", "scale", "flags_w", "visible", "row0_x", "light0", "clock0"] {
		assert!(!ro("moby", f), "moby.{f} is writable");
	}
	assert!(!ro("pvar0", "component") && ro("gold_case", "index"));
	let old = Binding::parse(r#"{"game":"rac1","structs":{"s":{"fields":[{"name":"a","offset":"0x4","type":"u32"}]}}}"#).unwrap();
	assert!(!old.struct_field("s", "a").unwrap().readonly);
}

#[test]
fn p2a_hooks_functions_and_fields_exist_with_their_signatures() {
	let b = Binding::parse(BINDING).unwrap();
	let sig = |id: &str| Sig::parse(&b.hook(id).unwrap_or_else(|| panic!("{id}")).signature).unwrap();
	let blast = sig("rac1.omni.blast.apply");
	assert_eq!((blast.args.iter().filter(|t| t.is_float()).count(), blast.args.iter().filter(|t| **t == Ty::U32).count()), (3, 8));
	let explode = sig("rac1.omni.fx.explode");
	assert_eq!((explode.args.iter().filter(|t| t.is_float()).count(), explode.args.len()), (6, 9));
	assert_eq!(sig("rac1.omni.moby.create").args.len(), 1);
	assert_eq!(sig("rac1.gold_bolt.count_spare").ret, Ty::I32);
	// P2a2: HeroItemsAttach has no arguments and no result
	let at = sig("rac1.omni.hero.items_attach");
	assert_eq!((at.args.len(), at.ret), (0, Ty::Void));
	let j = parse(BINDING).unwrap();
	let fns: Vec<&str> = j.get("functions").unwrap().arr().iter().filter_map(|f| f.get("name").and_then(Json::str)).collect();
	assert_eq!(fns.len(), 15);
	for n in ["create_moby", "delete_moby", "target_component", "blaster_aim_search", "spark_spawn", "glow_flare_spawn", "moby_cue", "moby_attach_to_joint"] {
		assert!(fns.contains(&n), "{n}");
	}
	assert!(!fns.contains(&"rocket_retarget"), "left out by the contract");
	let attach = j.get("functions").unwrap().arr().iter().find(|f| f.get("name").and_then(Json::str) == Some("moby_attach_to_joint")).unwrap();
	assert_eq!(attach.get("signature").and_then(Json::str), Some("void(u32,u32,u32)"));
}

#[test]
fn gold_prices_and_challenge_sit_where_the_notes_say() {
	use super::window::{translate, OVERLAY_WINDOW};
	let b = Binding::parse(BINDING).unwrap();
	let f = b.fields.iter().find(|f| f.name == "omni.weapon.bomb_glove.gold_price").expect("gold_price");
	assert_eq!((f.ntsc, f.pal), (OVERLAY_WINDOW + 0x1C_40C4 + 10 * 0x18, OVERLAY_WINDOW + 0x1C_40C4 + 10 * 0x18));
	// Novalis (level 1): price rows start at 0x1c4430
	assert_eq!(translate(false, 1, f.ntsc, 2), Some(0x1C_4430 + 10 * 0x18 + 0x14));
	assert_eq!(translate(false, 0, f.ntsc, 2), Some(0x1C_4030 + 10 * 0x18 + 0x14));
	let n = b.fields.iter().filter(|f| f.name.ends_with(".gold_price")).count();
	assert_eq!(n, 10);
	assert_eq!(b.fields.iter().find(|f| f.name == "omni.challenge").map(|f| f.ntsc), Some(0x15_EE20));
}

#[test]
fn mobys_section_and_pvar_classes_are_read() {
	let b = Binding::parse(BINDING).unwrap();
	let d = b.mobys.as_ref().expect("mobys section");
	assert_eq!((d.stride, d.state, d.dead_from, d.tail, d.class, d.pos, d.flags), (0x100, 0x20, 0x80, 0xff, 0xa6, 0x10, 0x34));
	assert_eq!((d.registered_flag, d.has_component_flag, d.class_ptr, d.class_type, d.enemy_type, d.pvar, d.max_slots), (0x1000, 0x20, 0x24, 0x46, 5, 0x78, 4096));
	assert_eq!((d.cells, d.registry), (0x4000_0000 + 0x15_FFD8, 0x4000_0000 + 0x1A_BA00));
	assert_eq!((d.start_word, d.pool_start_word, d.end_word), (0, 4, 8));
	let plain = Binding::parse(r#"{"game":"rac1","hooks":[]}"#).unwrap();
	assert!(plain.mobys.is_none(), "a binding without the section: moby queries not supported");
	assert_eq!(b.pvars.len(), 94);
	for c in [305, 457, 153, 121, 74, 186, 172, 304, 1456, 1465, 175, 177, 185, 203, 1900, 479] {
		assert!(b.pvars_of(c).is_some(), "pvars of class {c}");
	}
}

#[test]
fn dual_blaster_entries_flags_w_alias_and_hero_fields() {
	let b = Binding::parse(BINDING).unwrap();
	let off = |st: &str, f: &str| b.struct_offset(st, f).unwrap_or_else(|| panic!("{st}.{f}"));
	assert_eq!(off("moby", "flags"), off("moby", "flags_w"));
	assert_eq!(b.struct_field("moby", "flags_w").unwrap().ty, Ty::U16);
	assert_eq!((off("moby", "visible"), off("moby", "light0"), off("moby", "light1")), (0x31, 0x38, 0x3c));
	assert_eq!((off("moby", "row0_x"), off("moby", "row0_w"), off("moby", "row1_x"), off("moby", "row2_z")), (0xc0, 0xcc, 0xd0, 0xe8));
	assert_eq!((off("moby", "clock0"), off("moby", "clock3")), (0x50, 0x5c));
	assert_eq!((off("matrix4", "m00"), off("matrix4", "m30"), off("matrix4", "m31"), off("matrix4", "m33")), (0, 0x30, 0x34, 0x3c));
	assert_eq!(b.struct_fields("matrix4").unwrap().len(), 16);
	let at = |n: &str| b.fields.iter().find(|f| f.name == n).unwrap_or_else(|| panic!("{n}"));
	assert_eq!(at("omni.hero.held_moby").ntsc, 0x1403E0);
	assert_eq!(at("omni.hero.moby").ntsc, 0x1413D0);
	assert_eq!((at("omni.hero.right_hand.x").ntsc, at("omni.hero.right_hand.z").ntsc), (0x13FE40, 0x13FE48));
	assert_eq!(at("omni.hero.held_moby").pal, at("omni.hero.held_moby").ntsc + 0x100);
	assert!(b.fields.iter().filter(|f| f.name.starts_with("omni.hero.")).all(|f| f.ntsc < 0x20_0000), "plain boot RAM addresses");
}

/// The hero fields of the binding on a Blaster scene (Rilgar fg1/7_0284, Blaster equipped): held_moby is class 168, flags 6, and sits exactly on
/// the right hand attach matrix (omni.hero.right_hand); hero.moby is a class-0 moby.
#[test]
fn hero_fields_on_a_blaster_savestate() {
	let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../oracle/states/fg1/7_0284/eeMemory.bin");
	let alt = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/states/fg1/7_0284/eeMemory.bin");
	let Ok(ram) = std::fs::read(&p).or_else(|_| std::fs::read(&alt)) else {
		eprintln!("skipped: savestate oracle/states/fg1/7_0284 not found");
		return;
	};
	let b = Binding::parse(BINDING).unwrap();
	let u32_at = |a: u32| u32::from_le_bytes(ram[a as usize..a as usize + 4].try_into().unwrap());
	let f32_at = |a: u32| f32::from_bits(u32_at(a));
	let fa = |n: &str| b.fields.iter().find(|f| f.name == n).unwrap().ntsc;
	assert_eq!(u32_at(fa("omni.weapon.equipped")), 15, "Blaster equipped");
	let held = u32_at(fa("omni.hero.held_moby"));
	let hero = u32_at(fa("omni.hero.moby"));
	let class = |m: u32| i16::from_le_bytes([ram[(m + 0xa6) as usize], ram[(m + 0xa7) as usize]]);
	assert_eq!((class(held), class(hero)), (168, 0));
	let off = |f: &str| b.struct_offset("moby", f).unwrap();
	assert_eq!(u16::from_le_bytes([ram[(held + off("flags_w")) as usize], ram[(held + off("flags_w")) as usize + 1]]), 6);
	for (c, o) in [("x", "x"), ("y", "y"), ("z", "z")] {
		assert_eq!(f32_at(held + off(o)), f32_at(fa(&format!("omni.hero.right_hand.{c}"))), "held weapon sits on the right hand matrix ({c})");
	}
	assert_eq!(index_of(&b, "omni.hero.right_hand.x") + 1, index_of(&b, "omni.hero.right_hand.y"));
}

fn index_of(b: &Binding, name: &str) -> usize {
	b.fields.iter().position(|f| f.name == name).unwrap()
}
