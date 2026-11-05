//! Tests of `active.rs`: the launcher's own seven tests (scratch_launcher\launcher\src\mods\active.rs) ported one to one, plus the launcher's
//! file shapes as fixtures (`fixtures/launcher_*.txt`): each must read and write back byte for byte, a BOM + CRLF copy must come out as
//! the LF file, and the game's own reader (`mods::parse_active`) must see only the plain ids.
use super::active::*;
use super::mods::parse_active;

/// a, b = texture packs; s = a script mod; ghost is not installed.
fn kinds(id: &str) -> Option<Master> {
	match id {
		"a" | "b" => Some(Master::Textures),
		"s" => Some(Master::Mods),
		_ => None,
	}
}

fn ids(v: &[&str]) -> Vec<String> {
	v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_file_without_ticks_is_what_the_launcher_always_wrote() {
	let text = format!("{HEADER}\nb\na\nghost\n");
	let p = parse(&text, &kinds);
	assert_eq!((p.ids.clone(), p.masters, p.kept.len()), (ids(&["b", "a", "ghost"]), Masters::default(), 0));
	assert_eq!(render(&p.ids, p.masters, &p.kept, &kinds), text, "byte for byte");
}

#[test]
fn tick_off_comments_the_ids_of_its_kind_and_on_restores_them_in_place() {
	let on = format!("{HEADER}\ns\nb\na\n");
	let mut p = parse(&on, &kinds);
	p.masters.set(Master::Textures, false);
	let off = render(&p.ids, p.masters, &p.kept, &kinds);
	assert_eq!(off, format!("{HEADER}\n# Load texture packs: off\ns\n# b\n# a\n"), "the script mod is untouched");
	// the game's rule: text after # is ignored
	let game_sees: Vec<&str> = off.lines().filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty()).collect();
	assert_eq!(game_sees, vec!["s"]);
	// the file is read back with the switches remembered and the tick off
	let q = parse(&off, &kinds);
	assert_eq!(q.ids, ids(&["s", "b", "a"]), "per-mod switches and order survive");
	assert!(!q.masters.is_on(Master::Textures) && q.masters.is_on(Master::Mods));
	let mut q = q;
	q.masters.set(Master::Textures, true);
	assert_eq!(render(&q.ids, q.masters, &q.kept, &kinds), on, "ON restores the file exactly");
}

#[test]
fn both_ticks_off_and_a_tick_with_nothing_to_hold_is_still_remembered() {
	let mut p = parse(&format!("{HEADER}\ns\na\n"), &kinds);
	p.masters.set(Master::Textures, false);
	p.masters.set(Master::Mods, false);
	let off = render(&p.ids, p.masters, &p.kept, &kinds);
	assert_eq!(off, format!("{HEADER}\n# Load texture packs: off\n# Load mods: off\n# s\n# a\n"));
	assert_eq!(parse(&off, &kinds).ids, ids(&["s", "a"]));
	// no mod of the kind: the tick line alone keeps the state
	let q = parse(&format!("{HEADER}\n# Load mods: off\na\n"), &kinds);
	assert!(!q.masters.is_on(Master::Mods) && q.masters.is_on(Master::Textures));
	assert_eq!(q.ids, ids(&["a"]));
	assert!(render(&q.ids, q.masters, &q.kept, &kinds).contains("# Load mods: off\n"));
}

#[test]
fn ids_that_are_not_installed_are_never_held_back() {
	let mut p = parse(&format!("{HEADER}\nghost\na\n"), &kinds);
	p.masters.set(Master::Textures, false);
	let off = render(&p.ids, p.masters, &p.kept, &kinds);
	assert!(off.contains("\nghost\n# a\n"), "{off}");
}

#[test]
fn unknown_lines_are_kept_above_the_id_they_sat_above() {
	let text = "# my notes\n# keep this\nb\n# the loud one\na\n# at the end\n";
	let p = parse(text, &kinds);
	assert_eq!(p.ids, ids(&["b", "a"]));
	let out = render(&p.ids, p.masters, &p.kept, &kinds);
	assert_eq!(out, format!("{HEADER}\n# my notes\n# keep this\nb\n# the loud one\na\n# at the end\n"));
	// a mod that moves takes its note along; a note of a removed mod is not lost
	let moved = ids(&["a", "b"]);
	assert!(render(&moved, p.masters, &p.kept, &kinds).contains("# the loud one\na\n# my notes\n# keep this\nb\n"));
	let gone = ids(&["a"]);
	let out = render(&gone, p.masters, &p.kept, &kinds);
	assert!(out.contains("# my notes\n") && out.contains("# keep this\n") && out.ends_with("# at the end\n"), "{out}");
	// and while a tick is off
	let mut m = p.masters;
	m.set(Master::Textures, false);
	let off = render(&p.ids, m, &p.kept, &kinds);
	assert!(off.contains("# my notes\n# keep this\n# b\n# the loud one\n# a\n# at the end\n"), "{off}");
	assert_eq!(parse(&off, &kinds).kept, p.kept, "the same lines come back");
}

#[test]
fn a_typed_comment_that_looks_like_a_held_mod_is_not_switched_on_by_a_tick_cycle() {
	// the player commented `a` out by hand while the tick was on: the line is a comment, not a held mod
	let p = parse("b\n# a\n", &kinds);
	assert_eq!(p.ids, ids(&["b"]));
	let mut m = p.masters;
	m.set(Master::Textures, false);
	let off = render(&p.ids, m, &p.kept, &kinds);
	let q = parse(&off, &kinds);
	assert_eq!(q.ids, ids(&["b"]), "a is still off: {off}");
	let mut m = q.masters;
	m.set(Master::Textures, true);
	let back = render(&q.ids, m, &q.kept, &kinds);
	assert!(!back.lines().any(|l| l == "a"), "{back}");
	assert!(back.lines().any(|l| l.contains('a') && l.starts_with('#')), "the line is still there: {back}");
}

#[test]
fn bom_crlf_blank_lines_duplicates_and_other_header_text_are_tolerated() {
	let p = parse("\u{feff}# whatever the old header was\r\n\r\nb\r\n  a  \r\nb\r\n", &kinds);
	assert_eq!(p.ids, ids(&["b", "a"]));
	assert_eq!(p.kept, vec![(Some("b".to_string()), "# whatever the old header was".to_string())]);
	assert!(!render(&p.ids, p.masters, &p.kept, &kinds).contains('\r'));
	assert!(!parse("# Load Texture Packs : OFF\na\n", &kinds).masters.is_on(Master::Textures), "case and spaces do not matter");
	assert_eq!(parse("# Load texture packs: maybe\na\n", &kinds).kept.len(), 1, "a marker with a value we do not know is a foreign line");
}

/// (fixture text, the ids the game itself switches on). The fixtures are the launcher's four file shapes.
const FIXTURES: [(&str, &[&str]); 4] = [
	(include_str!("fixtures/launcher_plain.txt"), &["b", "a", "ghost"]),
	(include_str!("fixtures/launcher_textures_off.txt"), &["s"]),
	(include_str!("fixtures/launcher_both_off.txt"), &[]),
	(include_str!("fixtures/launcher_notes.txt"), &["b", "a"]),
];

#[test]
fn the_fixtures_are_lf_files_that_start_with_the_header() {
	for (i, (text, _)) in FIXTURES.iter().enumerate() {
		assert!(!text.contains('\r') && !text.starts_with('\u{feff}') && text.ends_with('\n'), "fixture {i} must be LF, no BOM, a final newline");
		assert_eq!(text.lines().next(), Some(HEADER), "fixture {i}");
	}
}

#[test]
fn every_launcher_file_reads_and_writes_back_byte_for_byte() {
	for (i, (text, _)) in FIXTURES.iter().enumerate() {
		let p = parse(text, &kinds);
		assert_eq!(&render(&p.ids, p.masters, &p.kept, &kinds), text, "fixture {i}");
	}
}

#[test]
fn a_bom_crlf_copy_comes_out_as_the_lf_file() {
	for (i, (text, _)) in FIXTURES.iter().enumerate() {
		let windows = format!("\u{feff}{}", text.replace('\n', "\r\n"));
		let p = parse(&windows, &kinds);
		let out = render(&p.ids, p.masters, &p.kept, &kinds);
		assert_eq!(&out, text, "fixture {i}");
		assert!(!out.contains('\r') && !out.starts_with('\u{feff}'), "fixture {i}");
	}
}

#[test]
fn the_games_reader_sees_only_the_plain_ids_of_what_is_written() {
	for (i, (text, want)) in FIXTURES.iter().enumerate() {
		let p = parse(text, &kinds);
		let out = render(&p.ids, p.masters, &p.kept, &kinds);
		assert_eq!(parse_active(&out), ids(want), "fixture {i}");
	}
}

#[test]
fn switching_a_tick_and_back_keeps_every_fixture_byte_for_byte() {
	for (i, (text, _)) in FIXTURES.iter().enumerate() {
		let mut p = parse(text, &kinds);
		let before = p.masters;
		for m in Master::ALL {
			p.masters.set(m, false);
		}
		let off = render(&p.ids, p.masters, &p.kept, &kinds);
		let mut q = parse(&off, &kinds);
		q.masters = before;
		assert_eq!(&render(&q.ids, q.masters, &q.kept, &kinds), text, "fixture {i}: all ticks off and back on is the same file");
	}
}
