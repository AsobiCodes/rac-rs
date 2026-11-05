//! The engine split's boundary (stage E1, 2026-10-26; plan ENGINE_SPLIT.md). The game-neutral `engine` must not reach
//! into the `rac1` pack, the shared `formats` into neither, and neither may hold an R&C1 RAM address. Counted per file of
//! src/engine and src/formats: references to a top-level module of a forbidden group (`crate::retail`, a bare `retail::x`, a `super::` chain up to
//! the crate root, `use crate::{..}`) and hex literals 0x100000..0x4fffff that are not a multiple of 0x10000 nor an all-ones
//! mask like 0x3f_ffff (comments, strings and `#[cfg(test)] mod` blocks skipped). The crossings of 2026-10-26 are capped in BOUNDARY_CAPS.txt (next to Cargo.toml). A cap only goes down,
//! as in SIZE_CAPS.txt: after removing crossings run `RAC_BOUNDARY_CAPS_WRITE=1 cargo test --lib boundary` (it lowers caps and
//! drops the zeros); raising or adding a cap fails against the committed file (git HEAD). Stage E2 works the list to zero.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const GROUPS: [&str; 3] = ["engine", "formats", "rac1"];

/// The groups a file of `group` must not name.
fn forbidden(group: &str) -> &'static [&'static str] {
	match group {
		"engine" => &["rac1"],
		"formats" => &["engine", "rac1"],
		_ => &[],
	}
}

fn src() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn caps_path() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("BOUNDARY_CAPS.txt")
}

/// Top-level module -> its group, read from the group crate roots (`pub mod X;` lines of src/<group>/lib.rs; stage E3).
fn group_of() -> BTreeMap<String, String> {
	let mut m = BTreeMap::new();
	for g in GROUPS {
		let t = std::fs::read_to_string(src().join(g).join("lib.rs")).unwrap_or_default();
		for l in t.lines() {
			if let Some(name) = l.trim().strip_prefix("pub mod ").and_then(|r| r.strip_suffix(';')) {
				m.insert(name.to_string(), g.to_string());
			}
		}
	}
	m
}

#[derive(Debug, PartialEq)]
enum Tok {
	Id(String),
	Path,
	Hex(u64),
	Ch(char),
}

/// Identifiers, `::`, hex literals and single characters of the code (comments and literals already blanked).
fn tokens(code: &[char]) -> Vec<Tok> {
	let (mut t, mut i) = (Vec::new(), 0);
	while i < code.len() {
		let c = code[i];
		if c.is_whitespace() {
			i += 1;
		} else if c == ':' && code.get(i + 1) == Some(&':') {
			t.push(Tok::Path);
			i += 2;
		} else if c == '0' && matches!(code.get(i + 1), Some('x' | 'X')) {
			let mut j = i + 2;
			let mut v: u64 = 0;
			while j < code.len() && (code[j].is_ascii_hexdigit() || code[j] == '_') {
				if let Some(d) = code[j].to_digit(16) {
					v = v.saturating_mul(16).saturating_add(d as u64);
				}
				j += 1;
			}
			while j < code.len() && (code[j].is_alphanumeric() || code[j] == '_') {
				j += 1; // suffix: u32, usize, ...
			}
			t.push(Tok::Hex(v));
			i = j;
		} else if c.is_alphanumeric() || c == '_' {
			let mut j = i;
			while j < code.len() && (code[j].is_alphanumeric() || code[j] == '_') {
				j += 1;
			}
			t.push(Tok::Id(code[i..j].iter().collect()));
			i = j;
		} else {
			t.push(Tok::Ch(c));
			i += 1;
		}
	}
	t
}

/// The module names a file declares (`mod x;`, `mod x {`, with or without `pub` / `#[path]`).
fn declared_mods(text: &str) -> Vec<String> {
	let t = tokens(&super::size_limits_tests::code_only(text));
	t.windows(2)
		.filter_map(|w| match w {
			[Tok::Id(m), Tok::Id(n)] if m == "mod" => Some(n.clone()),
			_ => None,
		})
		.collect()
}

/// The top-level modules a file names (once per mention) and its address-like literals. `depth` = how many modules deep the
/// file sits below the crate root as the code sees it (the group folder does not count: `src/engine/viewer/gpu.rs` = 2). A bare
/// path `retail::x` counts too (the group front doors import `crate::*`, so it reaches the root module), unless `local` declares
/// a module of that name on the file's own path (engine/fmv's `player`, omni's `host`).
fn crossings(text: &str, depth: usize, local: &[String]) -> (Vec<String>, usize) {
	let t = tokens(&super::size_limits_tests::code_only(text));
	let test = test_blocks(&t);
	let id = |k: usize| match t.get(k) {
		Some(Tok::Id(s)) => Some(s.as_str()),
		_ => None,
	};
	let (mut names, mut addrs) = (Vec::new(), 0);
	let mut k = 0;
	while k < t.len() {
		if let Tok::Hex(v) = t[k] {
			if !test[k] && (0x100000..0x500000).contains(&v) && v & 0xffff != 0 && v & (v + 1) != 0 {
				addrs += 1;
			}
			k += 1;
			continue;
		}
		if let Some(n) = id(k).filter(|n| !matches!(*n, "crate" | "super" | "self") && !local.iter().any(|l| l == n)) {
			if t.get(k + 1) == Some(&Tok::Path) && (k == 0 || !matches!(t[k - 1], Tok::Path | Tok::Ch('.'))) {
				names.push(n.to_string());
				k += 1;
				continue;
			}
		}
		// `crate::` or `super::` x depth: the next name (or each first name of a `{..}` group) is a crate-root item.
		let start = k;
		let root = if id(k) == Some("crate") && t.get(k + 1) == Some(&Tok::Path) {
			k += 2;
			true
		} else {
			let mut supers = 0;
			while id(k) == Some("super") && t.get(k + 1) == Some(&Tok::Path) {
				supers += 1;
				k += 2;
			}
			supers > 0 && supers == depth
		};
		if !root {
			k = if k == start { k + 1 } else { k };
			continue;
		}
		if let Some(n) = id(k) {
			names.push(n.to_string());
			k += 1;
		} else if t.get(k) == Some(&Tok::Ch('{')) {
			let (mut level, mut first) = (0, true);
			while k < t.len() {
				match &t[k] {
					Tok::Ch('{') => {
						level += 1;
						first = level == 1;
					}
					Tok::Ch('}') => {
						level -= 1;
						if level == 0 {
							break;
						}
					}
					Tok::Ch(',') if level == 1 => first = true,
					Tok::Id(s) if first && level == 1 => {
						names.push(s.clone());
						first = false;
					}
					_ => first = false,
				}
				k += 1;
			}
		}
	}
	(names, addrs)
}

/// Which tokens sit in a `#[cfg(test)] mod name { .. }` block: their hex literals are test data (sample entries, SPU2 RAM
/// addresses, WAV words), not the R&C1 RAM layout, and do not count as addresses. Paths there still count.
fn test_blocks(t: &[Tok]) -> Vec<bool> {
	let head = [Tok::Ch('#'), Tok::Ch('['), Tok::Id("cfg".into()), Tok::Ch('('), Tok::Id("test".into()), Tok::Ch(')'), Tok::Ch(']'), Tok::Id("mod".into())];
	let (mut out, mut k) = (vec![false; t.len()], 0);
	while k + head.len() + 2 <= t.len() {
		if t[k..k + head.len()] != head || !matches!(t[k + head.len()], Tok::Id(_)) || t[k + head.len() + 1] != Tok::Ch('{') {
			k += 1;
			continue;
		}
		let mut level = 0;
		k += head.len() + 1;
		while k < t.len() {
			out[k] = true;
			match t[k] {
				Tok::Ch('{') => level += 1,
				Tok::Ch('}') => level -= 1,
				_ => {}
			}
			k += 1;
			if level == 0 {
				break;
			}
		}
	}
	out
}

/// (path from rac-rs, kind) -> count, for every file of the checked groups. Kind = a forbidden group's name, or "addr".
fn measure() -> BTreeMap<(String, String), usize> {
	let groups = group_of();
	let mut out = BTreeMap::new();
	for g in GROUPS.iter().filter(|g| !forbidden(g).is_empty()) {
		let base = src().join(g);
		let mut todo = vec![base.clone()];
		while let Some(d) = todo.pop() {
			let Ok(rd) = std::fs::read_dir(&d) else { continue };
			for p in rd.flatten().map(|e| e.path()) {
				if p.is_dir() {
					todo.push(p);
					continue;
				}
				if p.extension().is_none_or(|x| x != "rs") {
					continue;
				}
				let text = String::from_utf8_lossy(&std::fs::read(&p).unwrap()).into_owned();
				if text.lines().take(5).any(|l| l.to_ascii_lowercase().contains("generated by")) {
					continue;
				}
				let rel = p.strip_prefix(&base).unwrap();
				let mut depth = rel.components().count();
				if rel.file_name().is_some_and(|f| f == "mod.rs") {
					depth -= 1;
				}
				// modules declared on the file's own path (its parents' front doors and the file itself) shadow a root name
				let mut local = declared_mods(&text);
				let mut up = base.clone();
				for c in rel.parent().into_iter().flat_map(|r| r.components()) {
					up = up.join(c);
					for door in [up.with_extension("rs"), up.join("mod.rs")] {
						if let Ok(b) = std::fs::read(&door) {
							local.extend(declared_mods(&String::from_utf8_lossy(&b)));
						}
					}
				}
				let (names, addrs) = crossings(&text, depth, &local);
				let key = p.strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap().to_string_lossy().replace('\\', "/");
				for n in names {
					if let Some(to) = groups.get(&n).filter(|to| forbidden(g).contains(&to.as_str())) {
						*out.entry((key.clone(), to.clone())).or_insert(0) += 1;
					}
				}
				if addrs > 0 {
					out.insert((key, "addr".to_string()), addrs);
				}
			}
		}
	}
	out
}

fn parse_caps(text: &str) -> BTreeMap<(String, String), usize> {
	let mut m = BTreeMap::new();
	for l in text.lines() {
		match l.split_whitespace().collect::<Vec<_>>().as_slice() {
			[] => {}
			[first, ..] if first.starts_with('#') => {}
			[p, kind, n] => {
				let n = n.parse().unwrap_or_else(|_| panic!("BOUNDARY_CAPS.txt: bad number in {l:?}"));
				m.insert((p.to_string(), kind.to_string()), n);
			}
			_ => panic!("BOUNDARY_CAPS.txt: cannot read {l:?}"),
		}
	}
	m
}

fn read_caps() -> Option<BTreeMap<(String, String), usize>> {
	Some(parse_caps(&std::fs::read_to_string(caps_path()).ok()?))
}

/// BOUNDARY_CAPS.txt as committed (git HEAD of the rac-rs repo); None outside a git checkout or before it was first committed.
fn committed_caps() -> Option<BTreeMap<(String, String), usize>> {
	let out = std::process::Command::new("git").args(["-C", env!("CARGO_MANIFEST_DIR"), "show", "HEAD:BOUNDARY_CAPS.txt"]).output().ok()?;
	out.status.success().then(|| parse_caps(&String::from_utf8_lossy(&out.stdout)))
}

const HEADER: &str = "# Engine split boundary (ENGINE_SPLIT.md): the crossings each file of src/engine and src/formats still has.
# Kept by rac-rs/src/boundary_tests.rs. Kinds: rac1 = names a module of the R&C1 pack; engine = a formats file names an engine
# module; addr = an R&C1 RAM address literal (0x100000..0x4fffff, not a multiple of 0x10000 nor an all-ones mask, outside
# `#[cfg(test)] mod` blocks). A cap only goes down: after removing crossings run
#  RAC_BOUNDARY_CAPS_WRITE=1 cargo test --lib boundary  (lowers caps, drops the zeros). Never raise or add one by hand.
";

fn write_caps(m: &BTreeMap<(String, String), usize>) {
	let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
	for ((_, kind), n) in m {
		*totals.entry(kind.as_str()).or_insert(0) += n;
	}
	let mut t = String::from(HEADER);
	t += &format!("# Totals: {}\n", totals.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", "));
	for ((p, kind), n) in m {
		t += &format!("{p} {kind} {n}\n");
	}
	let tmp = caps_path().with_extension("txt.tmp");
	std::fs::write(&tmp, t).expect("write BOUNDARY_CAPS.txt.tmp");
	std::fs::rename(&tmp, caps_path()).expect("replace BOUNDARY_CAPS.txt");
}

#[test]
fn no_new_boundary_crossing() {
	let now = measure();
	let caps = read_caps().unwrap_or_default();
	let bad: Vec<String> = now
		.iter()
		.filter_map(|((p, kind), &n)| {
			let cap = caps.get(&(p.clone(), kind.clone())).copied().unwrap_or(0);
			(n > cap).then(|| format!("{p}: {n} x {kind}, allowed {cap}"))
		})
		.collect();
	assert!(
		bad.is_empty(),
		"new crossing of the engine split boundary (engine must not use rac1; formats neither engine nor rac1; no R&C1 RAM addresses \
		 there): put the game fact on the rac1 side and ask through the Game interface (ENGINE_SPLIT.md):\n{}",
		bad.join("\n")
	);
}

#[test]
fn boundary_caps_are_tight() {
	let now = measure();
	let write = std::env::var("RAC_BOUNDARY_CAPS_WRITE").is_ok_and(|v| v == "1");
	let Some(caps) = read_caps() else {
		assert!(write, "BOUNDARY_CAPS.txt is missing next to Cargo.toml");
		write_caps(&now);
		return;
	};
	let mut loose = Vec::new();
	let mut keep = BTreeMap::new();
	for (k, &cap) in &caps {
		let n = now.get(k).copied().unwrap_or(0).min(cap);
		if n != cap {
			loose.push(format!("{} {}: cap {cap}, now {n}", k.0, k.1));
		}
		if n > 0 {
			keep.insert(k.clone(), n);
		}
	}
	if write {
		write_caps(&keep);
		return;
	}
	assert!(
		loose.is_empty(),
		"BOUNDARY_CAPS.txt allows more crossings than the code has now; lower it with  RAC_BOUNDARY_CAPS_WRITE=1 cargo test --lib boundary:\n{}",
		loose.join("\n")
	);
}

/// Against the committed BOUNDARY_CAPS.txt: no cap went up, no line was added (a moved file may take over the cap of a dropped
/// line of the same kind that is not lower).
#[test]
fn no_boundary_cap_was_raised_or_added() {
	let (Some(now), Some(was)) = (read_caps(), committed_caps()) else { return };
	let mut dropped: Vec<(&str, usize)> = was.iter().filter(|(k, _)| !now.contains_key(*k)).map(|((_, kind), &n)| (kind.as_str(), n)).collect();
	let mut bad = Vec::new();
	for ((p, kind), &n) in &now {
		match was.get(&(p.clone(), kind.clone())) {
			Some(&w) if n > w => bad.push(format!("{p} {kind}: cap raised from {w} to {n}")),
			Some(_) => {}
			None => match dropped.iter().position(|&(k, w)| k == kind && w >= n) {
				Some(i) => {
					dropped.remove(i);
				}
				None => bad.push(format!("{p} {kind}: cap {n} added")),
			},
		}
	}
	assert!(bad.is_empty(), "BOUNDARY_CAPS.txt may only go down (compared with git HEAD):\n{}", bad.join("\n"));
}

#[test]
fn the_boundary_scan_sees_the_groups() {
	let g = group_of();
	assert!(g.len() >= 60, "group front doors list only {} modules", g.len());
	for (m, want) in [("viewer", "engine"), ("ties", "formats"), ("retail", "rac1")] {
		assert_eq!(g.get(m).map(String::as_str), Some(want), "{m}");
	}
}

#[test]
fn crossings_count_root_paths_and_address_literals() {
	let text = "use crate::retail::x;\nuse crate::{toc, viewer::gpu};\n// crate::game 0x15ed84\nfn f() {\n\tsuper::map::a();\n\
		\tsuper::super::save::b();\n\tlet a = 0x13d4c0u32;\n\tlet b = 0x100000;\n\tlet c = 0x5000_00;\n\tlet m = 0x3f_ffff;\n\tlet s = \"crate::game 0x15ed84\";\n\
		\tpub(super) fn g() {}\n}\n";
	let (names, addrs) = crossings(text, 1, &[]);
	assert_eq!(names, ["retail", "toc", "viewer", "map"]);
	assert_eq!(addrs, 1);
	let (names, _) = crossings(text, 2, &[]);
	assert_eq!(names, ["retail", "toc", "viewer", "save"]);
	// hex literals in a `#[cfg(test)] mod` block are test data; a path there still counts
	let tests = "#[cfg(test)]\nmod tests {\n\tfn f() { let a = 0x13d4c0; { crate::retail::x(); } }\n}\nfn g() { let b = 0x2c04b0; }\n";
	let (names, addrs) = crossings(tests, 1, &[]);
	assert_eq!((names, addrs), (vec!["retail".to_string()], 1));
	// bare paths count (not after `::`), unless a module of that name is declared on the file's path
	let bare = "mod player;\nfn f() {\n\tretail::pcext::graphics();\n\tplayer::open();\n\tstd::mem::swap(a, b);\n\tx.y::<u8>();\n}\n";
	let (names, _) = crossings(bare, 1, &declared_mods(bare));
	assert_eq!(names, ["retail", "std"]);
	let (names, _) = crossings(bare, 1, &[]);
	assert_eq!(names, ["retail", "player", "std"]);
}
