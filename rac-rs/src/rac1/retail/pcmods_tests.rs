use super::*;
use crate::texpack::active::HEADER as FILE_HEADER;
use std::path::{Path, PathBuf};

fn dir(name: &str) -> PathBuf {
	let d = std::env::temp_dir().join(format!("rac_pcmods_{}_{name}", std::process::id()));
	let _ = std::fs::remove_dir_all(&d);
	std::fs::create_dir_all(&d).unwrap();
	d
}

fn add_mod(root: &Path, id: &str, name: &str, kind: &str) {
	std::fs::create_dir_all(root.join(id)).unwrap();
	std::fs::write(root.join(id).join("mod.json"), format!("{{\"format\":1,\"id\":\"{id}\",\"name\":\"{name}\",\"kind\":\"{kind}\",\"game\":\"rac1\"}}")).unwrap();
}

/// A script mod as the format ships it: `<folder>/omni-mod.json` with the manifest id `id` and these settings.
const AM_SETTINGS: &str = r#"[
	{"key":"perks","name":"Extra perks","type":"bool","default":true},
	{"key":"mode","name":"Ammo mode","type":"choice","default":"normal","choices":[{"value":"normal","name":"Normal"},{"value":"more","name":"More"},{"value":"lots","name":"Lots"}]},
	{"key":"start","name":"Start level","type":"int","default":1,"min":1,"max":5,"group":"advanced"}
]"#;

fn add_script(root: &Path, folder: &str, id: &str, name: &str, settings: &str) {
	std::fs::create_dir_all(root.join(folder)).unwrap();
	std::fs::write(root.join(folder).join("omni-mod.json"), format!("{{\"id\":\"{id}\",\"name\":\"{name}\",\"settings\":{settings}}}")).unwrap();
}

fn file(root: &Path) -> String {
	std::fs::read_to_string(root.join("active.txt")).unwrap_or_default()
}

fn rows() -> Vec<Row> {
	state(|st| st.rows.clone())
}

fn byte(mem: &mut Mem, i: u32) -> u8 {
	mem.rd8(MBASE + i)
}

fn pause_items_n(n: usize) -> Vec<Row> {
	pause_items(n, 0)
}

fn open_title(mem: &mut Mem, root: &Path) {
	build_title_with(mem, [0x30_0000, 0x30_0200, 0x30_0300], 0, Store::load(root));
}

fn stored(root: &Path, id: &str, key: &str) -> Option<Json> {
	plugins::value_of(&plugins::read_settings(root), id, key).cloned()
}

#[test]
fn region_and_ids_fit() {
	assert!(RANGE.start >= BASE + 0xBC00 && RANGE.end <= BASE + 0x10000, "above the Debug pool, inside the window");
	assert!(RANGE.end <= pcctl::RANGE.start || RANGE.start >= pcctl::RANGE.end);
	assert!(RANGE.start >= pcctl::DBG_RANGE.end);
	assert!(MBASE + TROWS as u32 <= MAGIC_AT && MAGIC_AT + 4 <= MPAGE && MPAGE + 0x100 <= MHDR && MHDR + 0x58 <= MVAL && MVAL + 0x40 <= MTAB);
	assert!(MTAB + 24 * TABLE_RECS as u32 <= POOL);
	assert!(POOL + SLOT * (ID_END - ID_FIRST) <= TPAGE, "the text pool ends before the title page");
	// The title page, its items, its row table (5-word rows + the zero row) and the Options list records (5 + the zero record).
	assert!(TPAGE + 0x100 <= THDR && THDR + 0x58 <= TVAL && TVAL + 0x40 <= TTAB);
	assert!(TTAB + 20 * (TROWS as u32 + 1) <= TRECS, "{:#x} {:#x}", TTAB + 20 * (TROWS as u32 + 1), TRECS);
	assert!(TRECS + 12 * 6 <= RANGE.end);
	assert!(ID_ROW + TROWS as u32 <= ID_END && ID_STEP < ID_ROW && ID_FIRST == ID_MODS);
	assert!(ID_VAL < ID_END && ID_VAL == ID_ROW + TROWS as u32);
	// No id of another page: pcctl 0x7f40..0x7f80, camopts 0x7f80..0x7f84, Debug 0x7fa0..0x7fb0.
	assert!(ID_FIRST >= 0x7fb0 && ID_END <= 0x8000);
	assert_eq!(TABLE_RECS, 8);
}

#[test]
fn every_text_is_short_font_bytes_in_every_language() {
	let root = dir("texts");
	add_mod(&root, "weird", "Caf\u{e9} \u{201c}Night\u{201d} %s_pack with a very long name indeed", "textures");
	let mut mem = Mem::new();
	for title in [false, true] {
		if title {
			open_title(&mut mem, &root);
		} else {
			test_open(&mut mem, &root);
		}
		for lang in 0..5 {
			for id in ID_FIRST..ID_END {
				let s = state(|st| text(id, lang, st));
				assert!(s.len() < SLOT as usize - 1, "{id:#x} {lang}");
				assert!(s.iter().all(|&b| (0x20..0x7f).contains(&b) || (0x80..=0xb0).contains(&b)), "{id:#x} {lang}: {s:?}");
				assert!(!s.contains(&b'%'), "0x25 draws the Cross glyph: {s:?}");
			}
			for id in [ID_MODS, ID_DESC, ID_HEADER, ID_PACKS, ID_TICK, ID_NOTE_L] {
				assert!(!state(|st| text(id, lang, st)).is_empty(), "{id:#x} {lang}");
			}
			// The note row is one line of at most 32 characters (the box shows about 40 beside an empty value; see the shot).
			let note = state(|st| text(ID_NOTE_L, lang, st));
			assert!(note.len() <= 32, "{}", String::from_utf8_lossy(&note));
			assert!(state(|st| text(ID_NOTE_V, lang, st)).is_empty(), "the note row has no value");
			assert!(!String::from_utf8_lossy(&note).to_lowercase().contains("launcher"), "no word about the launcher or other mods");
		}
	}
	let l = state(|st| text(ID_ROW + 1, 0, st));
	assert!(!l.is_empty() && l.len() <= modset::MAX_LABEL, "{}", String::from_utf8_lossy(&l));
	assert!(!l.contains(&b'%') && !l.contains(&b'_'));
	assert_ne!(state(|st| text(ID_NOTE_L, 0, st)), NOTE[0].to_vec(), "the title's note names the start, not the planet");
}

#[test]
fn sections_hold_at_most_seven_rows() {
	for (packs, secs) in [(0, 1), (5, 1), (6, 2), (20, 5), (60, 13), (200, 13)] {
		assert_eq!(sections(&pause_items_n(packs), ROWS), secs, "{packs} packs");
		for sec in 0..secs {
			let r = layout(&pause_items_n(packs), sec, ROWS);
			assert!(r.len() <= ROWS, "{packs} packs, section {sec}: {r:?}");
			assert_eq!(r.last(), Some(&Row::Note));
			assert_eq!(r.first() == Some(&Row::Step), secs > 1);
		}
	}
	assert_eq!(layout(&pause_items_n(0), 0, ROWS), [Row::Note], "no packs, no settings: just the note");
	assert_eq!(layout(&pause_items_n(6), 1, ROWS), [Row::Step, Row::Pack(4), Row::Pack(5), Row::Note], "7 items: the second section holds the last two");
	let all: Vec<Row> = (0..sections(&pause_items_n(20), ROWS)).flat_map(|s| layout(&pause_items_n(20), s, ROWS)).filter(|r| matches!(r, Row::Packs | Row::Pack(_))).collect();
	assert_eq!(all.len(), 21, "every item on exactly one section");
	// The pager takes any item list: settings rows after the packs page like the others.
	let more = pause_items(3, 3); // Packs, 3 packs, 3 settings = 7 items
	assert_eq!((sections(&more, ROWS), layout(&more, 1, ROWS)), (2, vec![Row::Step, Row::Set(1), Row::Set(2), Row::Note]));
	assert_eq!(pause_items(0, 2), [Row::Set(0), Row::Set(1)], "no packs: no packs tick");
}

#[test]
fn the_page_lists_the_packs_and_a_flip_writes_the_launchers_file() {
	let root = dir("flip");
	add_mod(&root, "beta", "Beta", "script");
	add_mod(&root, "alpha", "Alpha Pack", "textures");
	add_mod(&root, "gamma", "Gamma", "textures");
	let mut mem = Mem::new();
	test_open(&mut mem, &root);
	assert_eq!(mem.rd32(MAGIC_AT), MAGIC);
	assert_eq!(rows(), [Row::Packs, Row::Pack(0), Row::Pack(1), Row::Note], "the packs by name: Alpha, Gamma; the script mod is not a row in the game menu");
	let rec = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..6).map(|k| mem.rd32(MTAB + 24 * i + 4 * k)).collect() };
	assert_eq!(rec(&mut mem, 0), [ID_PACKS, MBASE, ID_OFF, ID_ON, 0, 0]);
	assert_eq!(rec(&mut mem, 1), [ID_ROW + 1, MBASE + 1, ID_OFF, ID_ON, 0, 0]);
	assert_eq!(rec(&mut mem, 3), [ID_NOTE_L, MBASE + 3, ID_NOTE_V, ID_NOTE_V, 0, 0]);
	assert_eq!(rec(&mut mem, 4), [0; 6], "the zero record ends the table");
	assert_eq!([0, 1, 2].map(|i| byte(&mut mem, i)), [1, 0, 0], "the tick on, no pack on yet");
	// Nothing is written without a flip.
	sync_with(&mut mem, true, false);
	assert_eq!(file(&root), "");
	// Alpha on: the launcher's file, the new pack first.
	mem.wr8(MBASE + 1, 1);
	sync_with(&mut mem, true, false);
	assert_eq!(file(&root), format!("{FILE_HEADER}\nalpha\n"));
	mem.wr8(MBASE + 2, 1);
	sync_with(&mut mem, true, false);
	assert_eq!(file(&root), format!("{FILE_HEADER}\ngamma\nalpha\n"));
	// The master tick "Texture packs" off: a comment line and the held ids, like the launcher.
	mem.wr8(MBASE, 0);
	sync_with(&mut mem, true, false);
	assert_eq!(file(&root), format!("{FILE_HEADER}\n# Load texture packs: off\n# gamma\n# alpha\n"));
	assert_eq!(byte(&mut mem, 1), 1, "the pack's own switch stays on");
	// What the game itself reads from the file: the plain ids only.
	assert!(crate::texpack::mods::parse_active(&file(&root)).is_empty());
	mem.wr8(MBASE, 1);
	mem.wr8(MBASE + 2, 0);
	sync_with(&mut mem, true, false);
	assert_eq!(file(&root), format!("{FILE_HEADER}\nalpha\n"));
	assert!(!root.join("active.txt.tmp").exists());
}

#[test]
fn a_pack_flip_leaves_script_and_native_entries_and_the_mods_tick_untouched() {
	let root = dir("foreign");
	add_mod(&root, "alpha", "Alpha", "textures");
	add_mod(&root, "gamma", "Gamma", "textures");
	add_mod(&root, "beta", "Beta", "script");
	add_mod(&root, "delta", "Delta", "native");
	// The launcher's file: "Load mods" off, one pack on, a script mod and a native mod held back by that tick.
	let before = format!("{FILE_HEADER}\n# Load mods: off\nalpha\n# beta\n# delta\n");
	std::fs::write(root.join("active.txt"), &before).unwrap();
	let mut mem = Mem::new();
	test_open(&mut mem, &root);
	assert_eq!(rows(), [Row::Packs, Row::Pack(0), Row::Pack(1), Row::Note], "only the packs are rows");
	assert_eq!([0, 1, 2].map(|i| byte(&mut mem, i)), [1, 1, 0]);
	sync_with(&mut mem, true, false);
	assert_eq!(file(&root), before, "no flip, no change");
	mem.wr8(MBASE + 2, 1); // Gamma on
	sync_with(&mut mem, true, false);
	let on = file(&root);
	assert_eq!(on, format!("{FILE_HEADER}\n# Load mods: off\ngamma\nalpha\n# beta\n# delta\n"));
	for line in ["# Load mods: off", "# beta", "# delta"] {
		assert!(on.lines().any(|l| l == line), "{line} stays");
	}
	mem.wr8(MBASE + 2, 0); // Gamma off again: the file is the original, byte for byte
	sync_with(&mut mem, true, false);
	assert_eq!(file(&root), before);
	mem.wr8(MBASE, 0); // the packs tick off and on
	sync_with(&mut mem, true, false);
	assert!(file(&root).contains("# Load texture packs: off\n") && file(&root).contains("# Load mods: off\n") && file(&root).contains("# beta\n# delta\n"));
	mem.wr8(MBASE, 1);
	sync_with(&mut mem, true, false);
	assert_eq!(file(&root), before);
	// The same with the script mod switched on by the launcher (its tick on): the line stays as it is.
	let live = format!("{FILE_HEADER}\nbeta\nalpha\ndelta\n");
	std::fs::write(root.join("active.txt"), &live).unwrap();
	test_open(&mut mem, &root);
	mem.wr8(MBASE + 1, 0); // Alpha off
	sync_with(&mut mem, true, false);
	assert_eq!(file(&root), format!("{FILE_HEADER}\nbeta\ndelta\n"), "only the pack's line left");
}

#[test]
fn the_options_row_wants_any_installed_mod_that_is_on() {
	let root = dir("visible");
	test_root(Some(&root));
	assert!(load_store().is_none(), "no mod installed");
	add_mod(&root, "alpha", "Alpha", "textures");
	add_mod(&root, "beta", "Beta", "script");
	assert!(load_store().is_none(), "installed, all off");
	std::fs::write(root.join("active.txt"), format!("{FILE_HEADER}\n# Load mods: off\n# beta\n")).unwrap();
	assert!(load_store().is_some(), "a script mod held back by its tick counts as on: the ticks are ignored");
	std::fs::write(root.join("active.txt"), format!("{FILE_HEADER}\nghost\n")).unwrap();
	assert!(load_store().is_none(), "an id that is not installed does not count");
	std::fs::write(root.join("active.txt"), format!("{FILE_HEADER}\nalpha\n")).unwrap();
	assert!(load_store().is_some(), "a pack on");
	test_root(None);
}

#[test]
fn a_flip_counts_only_on_the_open_page_and_the_note_row_is_inert() {
	let root = dir("stale");
	add_mod(&root, "alpha", "Alpha", "textures");
	let mut mem = Mem::new();
	test_open(&mut mem, &root);
	mem.wr8(MBASE + 1, 1); // stale bytes from a savestate while the page is closed
	sync_with(&mut mem, false, false);
	assert_eq!((file(&root), byte(&mut mem, 1)), (String::new(), 0), "not written, put back from the store");
	let note = rows().len() as u32 - 1;
	mem.wr8(MBASE + note, 1); // Cross on the note row
	sync_with(&mut mem, true, false);
	assert_eq!((byte(&mut mem, note), file(&root)), (0, String::new()), "the note row only shows text");
}

#[test]
fn the_stepper_pages_through_many_packs_and_wraps() {
	let root = dir("pages");
	for i in 0..11 {
		add_mod(&root, &format!("m{i:02}"), &format!("Pack {i:02}"), "textures");
	}
	add_mod(&root, "s1", "A script", "script"); // not a row, not a page
	let mut mem = Mem::new();
	test_open(&mut mem, &root);
	let n = state(|st| st.sections());
	assert_eq!(n, 3, "12 items in sections of 5");
	let r = rows();
	assert_eq!((r[0], r.len()), (Row::Step, 7));
	assert_eq!(state(|st| table(&st.rows, &st.vids, false))[..6], [pcctl::ID_PAGE, MBASE, ID_STEP, ID_STEP, 0, 0]);
	assert_eq!(state(|st| text(ID_STEP, 0, st)), b"< 1/3 >");
	let right = mouse_menu::RIGHT;
	let on = |_: &mut Mem| true;
	field_with(&mut mem, 0, &on); // baseline
	field_with(&mut mem, right, &on);
	assert_eq!((state(|st| st.pager.sec), state(|st| text(ID_STEP, 0, st))), (1, b"< 2/3 >".to_vec()));
	assert_eq!(rows()[1], Row::Pack(4));
	field_with(&mut mem, 0, &on);
	field_with(&mut mem, mouse_menu::LEFT, &on);
	field_with(&mut mem, 0, &on);
	field_with(&mut mem, mouse_menu::LEFT, &on);
	assert_eq!(state(|st| st.pager.sec), 2, "wraps from the first section to the last");
	// A flip in the last section writes the pack of that row.
	mem.wr8(MBASE + 1, 1);
	sync_with(&mut mem, true, false);
	assert_eq!(rows()[1], Row::Pack(9));
	assert_eq!(file(&root), format!("{FILE_HEADER}\nm09\n"));
	// Not on the selector: Left / Right do nothing.
	field_with(&mut mem, 0, &|_| false);
	field_with(&mut mem, right, &|_| false);
	assert_eq!(state(|st| st.pager.sec), 2);
}

#[test]
fn a_failed_write_puts_the_row_back() {
	let root = dir("fail");
	add_mod(&root, "alpha", "Alpha", "textures");
	let mut mem = Mem::new();
	test_open(&mut mem, &root);
	std::fs::create_dir_all(root.join("active.txt")).unwrap(); // a folder where the file belongs: the rename fails
	mem.wr8(MBASE + 1, 1);
	sync_with(&mut mem, true, false);
	assert_eq!(byte(&mut mem, 1), 0, "the row reads Off again");
	assert!(state(|st| st.warned));
}

#[test]
fn sync_and_field_are_inert_without_the_page() {
	let mut mem = Mem::new();
	mem.wr8(MBASE + 2, 1);
	sync(&mut mem, 4);
	field(&mut mem, 4, mouse_menu::RIGHT);
	front_field(&mut mem);
	assert_eq!(mem.rd8(MBASE + 2), 1);
	assert!(!title_open());
}

// ---- the title page and the settings rows ---------------------------------------------------------------------------------

#[test]
fn no_mod_installed_means_no_title_page_and_not_a_byte_of_ram() {
	let root = dir("empty");
	test_root(Some(&root));
	let mut mem = Mem::new();
	assert!(build_title(&mut mem, [0x30_0000, 0x30_0200, 0x30_0300], 0).is_none());
	assert_eq!(mem.rd32(MAGIC_AT), 0);
	assert!(RANGE.step_by(4).all(|a| mem.rd32(a) == 0), "the region stays untouched");
	// A folder that is no mod (no mod.json, no manifest) does not make a page either.
	std::fs::create_dir_all(root.join("junk")).unwrap();
	assert!(build_title(&mut mem, [0x30_0000, 0x30_0200, 0x30_0300], 0).is_none());
	// One pack makes one.
	add_mod(&root, "alpha", "Alpha", "textures");
	assert_eq!(build_title(&mut mem, [0x30_0000, 0x30_0200, 0x30_0300], 0), Some([ID_MODS | 3 << 16, TPAGE, 0]));
	test_root(None);
}

#[test]
fn the_title_page_switches_a_mod_by_its_manifest_id_and_shows_its_settings_when_on() {
	let root = dir("title");
	// The folder is what a GitHub zip unpacks to; the id of the lines in active.txt and settings.json is the manifest's.
	add_script(&root, "am-main", "am", "Arsenal Mastery", AM_SETTINGS);
	add_mod(&root, "alpha", "Alpha Pack", "textures");
	std::fs::write(root.join("settings.json"), r#"{"format":1,"mods":{"zz":{"k":1}}}"#).unwrap();
	let mut mem = Mem::new();
	open_title(&mut mem, &root);
	assert_eq!(mem.rd32(MAGIC_AT), MAGIC);
	assert_eq!(state(|st| st.store.as_ref().unwrap().mods.iter().map(|m| (m.id.clone(), m.folder.clone())).collect::<Vec<_>>()), [("alpha".into(), "alpha".into()), ("am".into(), "am-main".into())]);
	assert_eq!(rows(), [Row::Tick, Row::Mod(0), Row::Packs, Row::Pack(0), Row::Note], "mod off: no settings rows");
	// 5-word rows {label, byte, v0, v1, 0}, the zero row ends the table.
	let rec = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..5).map(|k| mem.rd32(TTAB + 20 * i + 4 * k)).collect() };
	assert_eq!(rec(&mut mem, 0), [ID_TICK, MBASE, ID_OFF, ID_ON, 0]);
	assert_eq!(rec(&mut mem, 1), [ID_ROW + 1, MBASE + 1, ID_OFF, ID_ON, 0]);
	assert_eq!(rec(&mut mem, 4), [ID_NOTE_L, MBASE + 4, ID_NOTE_V, ID_NOTE_V, 0]);
	assert_eq!(rec(&mut mem, 5), [0; 5]);
	assert_eq!(state(|st| text(ID_ROW + 1, 0, st)), b"Arsenal Mastery");
	assert_eq!(state(|st| text(ID_NOTE_L, 0, st)), NOTE_T[0]);
	assert_eq!([0, 1, 2, 3].map(|i| byte(&mut mem, i)), [1, 0, 1, 0], "the mods tick on, the mod off, the packs tick on, no pack on");
	// A flip while the page is not the open one is put back.
	mem.wr8(MBASE + 1, 1);
	sync_with(&mut mem, false, true);
	assert_eq!((file(&root), byte(&mut mem, 1)), (String::new(), 0));
	// The pause sync does not touch the title page.
	mem.wr8(MBASE + 1, 1);
	sync_with(&mut mem, true, false);
	assert_eq!(file(&root), "");
	// The mod on: the line is the manifest id and its basic settings rows appear (start is advanced: no row).
	sync_with(&mut mem, true, true);
	assert_eq!(file(&root), format!("{FILE_HEADER}\nam\n"));
	assert_eq!(rows(), [Row::Tick, Row::Mod(0), Row::Set(0), Row::Set(1), Row::Packs, Row::Pack(0), Row::Note]);
	assert_eq!([0, 1, 2, 3].map(|i| byte(&mut mem, i)), [1, 1, 1, 0], "perks default On, mode Normal");
	assert_eq!(state(|st| text(ID_ROW + 2, 0, st)), b"Extra perks");
	assert_eq!(state(|st| text(ID_ROW + 3, 0, st)), b"Ammo mode");
	// A bool: Off.
	mem.wr8(MBASE + 2, 0);
	sync_with(&mut mem, true, true);
	assert_eq!(stored(&root, "am", "perks"), Some(Json::Bool(false)));
	assert_eq!(stored(&root, "zz", "k"), Some(Json::Num(1.0)), "the other mods' keys stay");
	assert_eq!(stored(&root, "am-main", "perks"), None, "keyed by the manifest id, not the folder");
	// A three-value row: the title's value item only flips the byte, so each flip steps the value on (Normal -> More -> Lots -> Normal).
	for (want, text) in [(1u8, "More"), (2, "Lots"), (0, "Normal")] {
		let b = byte(&mut mem, 3);
		mem.wr8(MBASE + 3, if b == 0 { 1 } else { 0 });
		sync_with(&mut mem, true, true);
		assert_eq!(byte(&mut mem, 3), want);
		assert_eq!(stored(&root, "am", "mode"), Some(Json::Str(["normal", "more", "lots"][want as usize].into())), "{text}");
	}
	// The mod off again: its settings rows go, the values stay in settings.json.
	mem.wr8(MBASE + 1, 0);
	sync_with(&mut mem, true, true);
	assert_eq!(file(&root), format!("{FILE_HEADER}\n"));
	assert_eq!(rows(), [Row::Tick, Row::Mod(0), Row::Packs, Row::Pack(0), Row::Note]);
	assert_eq!(stored(&root, "am", "perks"), Some(Json::Bool(false)));
	// The master tick off.
	mem.wr8(MBASE, 0);
	sync_with(&mut mem, true, true);
	assert!(file(&root).contains("# Load mods: off"), "{}", file(&root));
	assert!(!root.join("settings.json.tmp").exists());
}

#[test]
fn stored_values_show_on_the_title_and_the_mods_tick_state_is_read() {
	let root = dir("stored");
	add_script(&root, "am", "am", "Arsenal Mastery", AM_SETTINGS);
	std::fs::write(root.join("active.txt"), format!("{FILE_HEADER}\nam\n")).unwrap();
	std::fs::write(root.join("settings.json"), r#"{"format":1,"mods":{"am":{"perks":false,"mode":"lots"}}}"#).unwrap();
	let mut mem = Mem::new();
	open_title(&mut mem, &root);
	assert_eq!(rows(), [Row::Tick, Row::Mod(0), Row::Set(0), Row::Set(1), Row::Note]);
	assert_eq!([0, 1, 2, 3].map(|i| byte(&mut mem, i)), [1, 1, 0, 2]);
	// The value texts of the three-value row are in the pool, the bool's are Off / On.
	let (vids, vt) = state(|st| (st.vids.clone(), st.vtext.clone()));
	assert_eq!((vids[2][0], vids[2][1]), (ID_OFF, ID_ON));
	let names: Vec<&str> = vids[3][..3].iter().map(|id| vt.iter().find(|(i, _)| i == id).unwrap().1.as_str()).collect();
	assert_eq!(names, ["Normal", "More", "Lots"]);
	assert!(vids.iter().flatten().all(|id| *id < ID_END));
	assert_eq!(title_rows(), None, "not the open page");
	// The record the Options list gets.
	let mut m2 = Mem::new();
	test_root(Some(&root));
	assert_eq!(build_title(&mut m2, [0x30_0000, 0x30_0200, 0x30_0300], 0), Some([ID_MODS | 3 << 16, TPAGE, 0]));
	test_root(None);
}

#[test]
fn the_pause_page_has_settings_rows_but_never_a_mod_on_off_row() {
	let root = dir("pause");
	add_script(&root, "am-main", "am", "Arsenal Mastery", AM_SETTINGS);
	add_mod(&root, "alpha", "Alpha Pack", "textures");
	std::fs::write(root.join("active.txt"), format!("{FILE_HEADER}\nam\nalpha\n")).unwrap();
	let mut mem = Mem::new();
	let sets: Vec<SetRow> = modset::manifest(&root.join("am-main")).map(|(id, _, l)| modset::rows(&id, &l, &|_| None)).unwrap();
	assert_eq!(sets.len(), 2);
	build_with(&mut mem, [0x30_0000, 0x30_0200, 0x30_0300], None, Store::load(&root), sets);
	assert_eq!(rows(), [Row::Packs, Row::Pack(0), Row::Set(0), Row::Set(1), Row::Note]);
	assert!(rows().iter().all(|r| !matches!(r, Row::Mod(_) | Row::Tick)), "no mod On / Off in the game menu");
	assert_eq!(state(|st| text(ID_ROW + 2, 0, st)), b"Extra perks");
	assert_eq!([0, 1, 2, 3].map(|i| byte(&mut mem, i)), [1, 1, 1, 0]);
	// 4-word value ids: the choice row has its three texts, the bool Off / On.
	let rec = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..6).map(|k| mem.rd32(MTAB + 24 * i + 4 * k)).collect() };
	let r2 = rec(&mut mem, 2);
	assert_eq!((r2[2], r2[3]), (ID_OFF, ID_ON));
	let r3 = rec(&mut mem, 3);
	assert!(r3[2] >= ID_VAL && r3[3] >= ID_VAL && r3[4] >= ID_VAL && r3[5] == 0, "{r3:?}");
	// The pause menu's own item cycles the value: the byte holds the new index; settings.json gets it, active.txt stays.
	let before = file(&root);
	mem.wr8(MBASE + 3, 2);
	sync_with(&mut mem, true, false);
	assert_eq!(stored(&root, "am", "mode"), Some(Json::Str("lots".into())));
	mem.wr8(MBASE + 2, 0);
	sync_with(&mut mem, true, false);
	assert_eq!(stored(&root, "am", "perks"), Some(Json::Bool(false)));
	assert_eq!(file(&root), before, "a settings flip never touches active.txt");
	// Out of range from a stale byte: clamped to the last value.
	mem.wr8(MBASE + 3, 9);
	sync_with(&mut mem, true, false);
	assert_eq!(byte(&mut mem, 3), 2);
	// Not the open page: put back.
	mem.wr8(MBASE + 2, 1);
	sync_with(&mut mem, false, false);
	assert_eq!(byte(&mut mem, 2), 0);
	assert_eq!(stored(&root, "am", "perks"), Some(Json::Bool(false)));
}

#[test]
fn the_title_page_pages_what_the_panel_holds_and_drops_nothing() {
	let root = dir("many");
	let many = (0..9).map(|i| format!(r#"{{"key":"k{i}","name":"Row {i}","type":"choice","default":"a","choices":[{{"value":"a","name":"A{i}"}},{{"value":"b","name":"B{i}"}},{{"value":"c","name":"C{i}"}},{{"value":"d","name":"D{i}"}}]}}"#)).collect::<Vec<_>>().join(",");
	add_script(&root, "big", "big", "Big mod", &format!("[{many}]"));
	for i in 0..8 {
		add_mod(&root, &format!("p{i}"), &format!("Pack {i}"), "textures");
	}
	std::fs::write(root.join("active.txt"), format!("{FILE_HEADER}
big
")).unwrap();
	let mut mem = Mem::new();
	open_title(&mut mem, &root);
	let n = state(|st| st.sections());
	// Tick, Mod, 9 settings, Packs tick, 8 packs = 20 items: at most 11 per section and 8 settings rows.
	assert!(n >= 2, "{n} sections");
	let mut seen = Vec::new();
	for sec in 0..n {
		state(|st| go(&mut mem, st, sec));
		let r = rows();
		assert!(r.len() <= TROWS, "{r:?}");
		assert_eq!((r.first(), r.last()), (Some(&Row::Step), Some(&Row::Note)), "section {sec}");
		assert!(r.iter().filter(|x| matches!(x, Row::Set(_))).count() <= MAX_SETS);
		assert!(state(|st| st.vids.iter().flatten().all(|id| *id < ID_END)));
		let ids: Vec<u32> = state(|st| st.vids.iter().flat_map(|v| v.iter().copied()).collect());
		assert!(ids.iter().all(|id| *id < ID_END - 1 || *id == 0), "the text pool did not run out");
		for i in 0..=r.len() as u32 {
			assert_eq!(mem.rd32(TTAB + 20 * i) == 0, i as usize == r.len(), "section {sec} row {i}");
		}
		seen.extend(r.into_iter().filter(|x| !matches!(x, Row::Step | Row::Note)));
	}
	let want = title_items(1, &(0..9).map(|_| 0).collect::<Vec<_>>(), 8);
	assert_eq!(seen, want, "every item on exactly one section, in order");
	assert_eq!(seen.len(), 20);
}

#[test]
fn the_title_stepper_flips_through_the_sections_and_wraps() {
	let root = dir("tpages");
	let many = (0..9).map(|i| format!(r#"{{"key":"k{i}","name":"Row {i}","type":"bool","default":true}}"#)).collect::<Vec<_>>().join(",");
	add_script(&root, "big", "big", "Big mod", &format!("[{many}]"));
	std::fs::write(root.join("active.txt"), format!("{FILE_HEADER}
big
")).unwrap();
	let mut mem = Mem::new();
	open_title(&mut mem, &root);
	let n = state(|st| st.sections());
	assert_eq!(n, 2, "11 items, 9 of them settings: two sections");
	assert_eq!(rows()[0], Row::Step);
	assert_eq!(state(|st| table(&st.rows, &st.vids, true))[..5], [ID_PAGE_T, MBASE, ID_STEP, ID_STEP, 0]);
	assert_eq!(state(|st| text(ID_PAGE_T, 0, st)), b"Page");
	assert_eq!(state(|st| text(ID_STEP, 0, st)), b"< 1/2 >");
	assert_eq!(texts(0).iter().find(|(id, _)| *id == ID_PAGE_T).map(|(_, t)| t.as_str()), Some("Page"), "the mirror gets the label");
	// Cross on the stepper (cursor on row 0): the second section. A flip with the cursor elsewhere does nothing.
	title_step(&mut mem, true); // baseline
	mem.wr8(MBASE, 1);
	title_step(&mut mem, false);
	assert_eq!(state(|st| st.pager.sec), 0);
	title_step(&mut mem, true);
	assert_eq!(state(|st| (st.pager.sec, text(ID_STEP, 0, st))), (0, b"< 1/2 >".to_vec()), "the flip was seen while the cursor was away");
	mem.wr8(MBASE, 0);
	title_step(&mut mem, true);
	assert_eq!(state(|st| st.pager.sec), 1);
	assert_eq!(rows()[0], Row::Step);
	assert!(rows().contains(&Row::Set(8)), "the ninth setting is on the second section");
	mem.wr8(MBASE, 1);
	title_step(&mut mem, true);
	assert_eq!(state(|st| st.pager.sec), 0, "wraps");
	// A settings flip on a later section writes settings.json.
	mem.wr8(MBASE, 0);
	title_step(&mut mem, true);
	let k = rows().iter().position(|r| *r == Row::Set(8)).unwrap() as u32;
	mem.wr8(MBASE + k, 0);
	sync_with(&mut mem, true, true);
	assert_eq!(stored(&root, "big", "k8"), Some(Json::Bool(false)));
}

#[test]
fn a_title_page_that_fits_has_no_stepper() {
	let root = dir("tfits");
	add_script(&root, "am-main", "am", "Arsenal Mastery", AM_SETTINGS);
	std::fs::write(root.join("active.txt"), format!("{FILE_HEADER}
am
")).unwrap();
	let mut mem = Mem::new();
	open_title(&mut mem, &root);
	assert_eq!(state(|st| st.sections()), 1);
	assert_eq!(rows().first(), Some(&Row::Tick));
	assert_eq!(rows().last(), Some(&Row::Note));
}

#[test]
fn the_pause_note_is_blank_without_packs_and_names_the_planet_with_them() {
	let root = dir("pnote");
	add_script(&root, "am-main", "am", "Arsenal Mastery", AM_SETTINGS);
	std::fs::write(root.join("active.txt"), format!("{FILE_HEADER}\nam\n")).unwrap();
	let mut mem = Mem::new();
	test_open(&mut mem, &root);
	assert_eq!(state(|st| text(ID_NOTE_L, 0, st)), Vec::<u8>::new(), "settings apply at once: nothing to announce");
	add_mod(&root, "alpha", "Alpha Pack", "textures");
	test_open(&mut mem, &root);
	assert_eq!(state(|st| text(ID_NOTE_L, 0, st)), NOTE[0].to_vec());
}

#[test]
fn a_hand_copied_pcsx2_pack_that_is_off_still_gives_the_title_page_and_not_the_pause_row() {
	let root = dir("by_hand");
	test_root(Some(&root));
	let p = root.join("SCES-50916").join("replacements");
	std::fs::create_dir_all(&p).unwrap();
	std::fs::write(p.join("1-2-00005dd3.dds"), b"x").unwrap();
	let mut mem = Mem::new();
	assert_eq!(build_title(&mut mem, [0x30_0000, 0x30_0200, 0x30_0300], 0), Some([ID_MODS | 3 << 16, TPAGE, 0]), "installed, off: the title row");
	assert!(load_store().is_none(), "the pause row waits for a pack that is on");
	std::fs::write(root.join("active.txt"), format!("{FILE_HEADER}\nSCES-50916\n")).unwrap();
	assert!(load_store().is_some());
	test_root(None);
}

/// Without a mod loader (Linux) the script / native mods are left out because they cannot run; texture packs stay. With a loader both are
/// there, and the mods folder keeps everything either way.
#[test]
fn the_script_mods_wait_for_the_loader() {
	let d = dir("loader");
	add_script(&d, "am", "arsenal_mastery", "Arsenal Mastery", AM_SETTINGS);
	add_mod(&d, "pack", "A Pack", "textures");
	test_root(Some(&d));
	let s = read_store().expect("the store");
	assert_eq!((scripts(&s).len(), packs(&s).len()), (1, 1), "read_store as such offers both");
	let s = without_loader(read_store().expect("the store"), false);
	assert!(scripts(&s).is_empty(), "no script mods without a loader");
	assert_eq!(packs(&s).len(), 1, "the texture pack stays");
	let s = without_loader(read_store().expect("the store"), true);
	assert_eq!((scripts(&s).len(), packs(&s).len()), (1, 1), "with a loader both are offered");
	test_root(None);
}
