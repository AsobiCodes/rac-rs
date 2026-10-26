//! Keeps ENV.md (the catalog of the engine's environment switches) in step with the source: every "RAC_..." string literal in
//! src needs a table row there, and every RAC_ row there must still exist in src.
use std::collections::BTreeSet;
use std::path::Path;

fn walk(dir: &Path, out: &mut BTreeSet<String>) {
	let Ok(rd) = std::fs::read_dir(dir) else { return };
	for e in rd.flatten() {
		let p = e.path();
		if p.is_dir() {
			walk(&p, out);
		} else if p.extension().is_some_and(|x| x == "rs") && p.file_name().is_some_and(|n| n != "env_doc_tests.rs") {
			let Ok(text) = std::fs::read(&p) else { continue };
			let key = b"\"RAC_";
			let mut i = 0;
			while i + key.len() <= text.len() {
				if &text[i..i + key.len()] == key {
					let mut j = i + 1;
					while j < text.len() && (text[j].is_ascii_uppercase() || text[j].is_ascii_digit() || text[j] == b'_') {
						j += 1;
					}
					out.insert(String::from_utf8_lossy(&text[i + 1..j]).into_owned());
					i = j;
				} else {
					i += 1;
				}
			}
		}
	}
}

fn in_source() -> BTreeSet<String> {
	let mut s = BTreeSet::new();
	walk(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")), &mut s);
	s
}

/// The RAC_ names of ENV.md table rows (`| \`NAME\` | ...`).
fn in_doc() -> BTreeSet<String> {
	let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/ENV.md")).expect("ENV.md missing next to Cargo.toml");
	doc.lines()
		.filter_map(|l| l.strip_prefix("| `"))
		.filter_map(|l| l.split('`').next())
		.filter(|n| n.starts_with("RAC_"))
		.map(str::to_owned)
		.collect()
}

#[test]
fn every_switch_in_src_has_a_row_in_env_md() {
	let (src, doc) = (in_source(), in_doc());
	let missing: Vec<_> = src.difference(&doc).cloned().collect();
	assert!(!src.is_empty(), "no RAC_ literals found: the scan is broken");
	assert!(missing.is_empty(), "env switches in src without a row in ENV.md, add a row to ENV.md for: {missing:?}");
}

#[test]
fn every_row_in_env_md_still_exists_in_src() {
	let (src, doc) = (in_source(), in_doc());
	let stale: Vec<_> = doc.difference(&src).cloned().collect();
	assert!(stale.is_empty(), "ENV.md rows whose switch is no longer in src, remove these rows from ENV.md: {stale:?}");
}
