//! Guards of the PAL -> USA address layer (disc.rs, disc_map.rs, disc_proven.rs). The game's extras read retail RAM at PAL
//! addresses; on the USA disc every one of them has to go through `disc::va` / `va_level` / `g` / `pick` / `hands`
//! (the `disc::` functions only: not a local closure of the same name, not a method) or be one of the few raw addresses
//! listed in `ALLOWED` with the reason it is the same on both discs. `g` maps only `G_LO..=G_HI`, and only a literal (or a const) that is the whole argument (test b4).
//!
//! WHAT THE GUARD DOES NOT CATCH (the USA smoke trace, `RAC_VA_TRACE` + tools/addr/gen_disc_map.py --check-trace, is the net
//! for these; they are listed so nobody trusts the green tests for more than they prove):
//!  - a decimal literal, an address built by arithmetic or read from a variable, and a mapped address that falls in a hole
//!    of the USA map (disc_trace.rs);
//!  - literals >= 0x400000 (heap / tail) and KSEG forms (0x2xxxxxxx, 0x8xxxxxxx) of an overlay address: only 0x15ef00..0x3fffff
//!    is scanned;
//!  - the allow-list pins COUNTS, not values: replacing one allowed literal of an `Other` group by another address of the
//!    same count passes, and so does changing the value of an `Other` const (Core rows are re-proven, so they are safe);
//!  - the `Via` proof is not transitive and does not follow the argument: a helper "maps" when its body holds any mapping
//!    call or a call of another listed helper (two helpers that only call each other would pass); only the first
//!    `fn` / `let` of that name per file is looked at;
//!  - import holes: `use super::{disc, other::g}` makes a foreign bare `g(..)` look imported; a parameter or match binding
//!    named `g` / `va` shadows the import unseen (only `let` / `fn` are seen); `use a::X as Y` is not followed (a renamed
//!    ViaVa / Via / Pick / Hands item is not checked); `METHOD_MAPPERS` goes by file and name (modcam `Lv::g` is a mapper,
//!    also the CPU register getter `c.g(29)` of that file, which is never called with addresses today); one
//!    `let at = |..| va(..)` makes every `at(` call of its file count as mapped; `Via` helper names count in any file;
//!  - `f2` repeats the generator's own criterion (masked code words equal) on a second ISO reader, so it is a regression
//!    check, not independent evidence; its mask is looser than the Python one for COP1 / COP2 / MMI and branch offsets, the
//!    39 rows of the front and boot scopes are not compared, a row mis-aligned the same way by vermap, the generator and f2
//!    (a twin function) would pass all three, and without both ISOs it prints SKIPPED and passes unless RAC_REQUIRE_ISOS=1;
//!  - dead data: the USA mapping in shiprefl.rs and strips.rs feeds structures nobody reads (noted here only);
//!    14 `Via` / `Other` rows stay for it until the census deletes it.
//!
//! Each guard has a seeded bad case in its own test (the scanner flags a raw address, the checker rejects a wrong USA address,
//! a hole in a segment table fails, a method that never calls `va` is found), so none of them can pass while checking nothing.
//!  a. `scanner_flags_a_seeded_raw_address`: the raw-literal scanner finds what it should (and nothing in comments, strings,
//!     test modules, wrapped calls and twinned tables).
//!  a2. `scanner_follows_imports_and_twins`: grouped / glob / `super` imports, twinned items, `Via` helpers, hook tables.
//!  a3. `core_proof_needs_every_scope`: a core address maps to itself in all 21 scopes.
//!  a4. `a_lone_usa_item_is_scanned`: an `X_USA` item without its PAL twin is a raw address.
//!  a5. `a_method_mapper_without_va_is_found`: seeded failing case of b3.
//!  a6. `a_path_g_call_in_a_method_mapper_file_is_checked`: in modcam.rs only `.g(..)` is a general mapper.
//!  b. `raw_pal_addresses_stay_in_the_allow_list`: the scan of the whole source tree equals `ALLOWED` exactly; a new raw
//!     PAL address anywhere fails, and so does a stale entry (the list only shrinks).
//!  b2. `twinned_tables_are_only_used_through_pick`: every use of a twinned table and of its `_USA` twin is in `disc::pick`.
//!  b3. `method_mappers_call_va`: the methods of `METHOD_MAPPERS` call `va` in their body.
//!  b4. `g_maps_only_its_window`: every `g(..)` argument is in `G_LO..=G_HI` (seeded cases first).
//!  c. `every_disc_pick_has_its_usa_twin`: `disc::pick(&A, &B)` always pairs A with A_USA.
//!  d. `proven_addresses_map_to_their_exact_segment`: every (scope, PAL, USA) of disc_proven.rs maps to exactly that USA
//!     address in a run / gap / ext segment of its scope.
//!  e. `a_hole_in_the_segments_is_none_and_fails_the_tests`: an address outside every segment is `K_NONE` and panics under
//!     test; the real tables are sorted and disjoint.
//!  f. `both_discs_confirm_the_proven_list`: with both ISOs on the path of the project folder the code / data words at the
//!     PAL and the USA address of every C / D proven row are equal (prints how many rows it compared; without the ISOs it prints
//!     "SKIPPED: ISO missing" and compares nothing: run it with --nocapture to see the line; RAC_REQUIRE_ISOS=1 makes a
//!     missing ISO a failure).
//! The trace tests of disc_trace.rs (`off_path_takes_no_lock`, `capped_slot_miss_writes_nothing`,
//! `hot_functions_have_no_lock_or_scan`) guard the run-time cost of the layer.
use super::disc::{map_kind, segs_of, traced, G_HI, G_LO};
use super::disc_map::{Seg, BOOTOV, FRONT, LEVELS};
use super::disc_proven::PROVEN;
use super::disc_trace::{K_CORE, K_NONE};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---- text helpers --------------------------------------------------------------------------------------------------

pub(crate) fn is_id(c: char) -> bool {
	c.is_alphanumeric() || c == '_'
}

fn starts(c: &[char], i: usize, s: &str) -> bool {
	s.chars().enumerate().all(|(k, ch)| c.get(i + k) == Some(&ch))
}

/// The text with comments, strings and char literals blanked (newlines kept), so only code is left.
pub(crate) fn blank_text(src: &str) -> Vec<char> {
	let c: Vec<char> = src.chars().collect();
	let mut out = c.clone();
	let mut i = 0;
	let blank = |out: &mut Vec<char>, a: usize, b: usize| {
		for x in out.iter_mut().take(b.min(c.len())).skip(a) {
			if *x != '\n' {
				*x = ' ';
			}
		}
	};
	while i < c.len() {
		let next = c.get(i + 1).copied();
		let prefix_ok = i == 0 || !is_id(c[i - 1]) || (matches!(c[i - 1], 'b' | 'c') && (i < 2 || !is_id(c[i - 2])));
		if c[i] == '/' && next == Some('/') {
			let e = c[i..].iter().position(|&x| x == '\n').map_or(c.len(), |p| i + p);
			blank(&mut out, i, e);
			i = e;
		} else if c[i] == '/' && next == Some('*') {
			let (mut depth, mut j) = (1, i + 2);
			while j < c.len() && depth > 0 {
				if starts(&c, j, "/*") {
					depth += 1;
					j += 2;
				} else if starts(&c, j, "*/") {
					depth -= 1;
					j += 2;
				} else {
					j += 1;
				}
			}
			blank(&mut out, i, j);
			i = j;
		} else if c[i] == 'r' && prefix_ok && matches!(next, Some('"' | '#')) {
			let mut j = i + 1;
			let mut hashes = 0;
			while c.get(j) == Some(&'#') {
				hashes += 1;
				j += 1;
			}
			if c.get(j) != Some(&'"') {
				i = j;
				continue;
			}
			j += 1;
			while j < c.len() && !(c[j] == '"' && (0..hashes).all(|k| c.get(j + 1 + k) == Some(&'#'))) {
				j += 1;
			}
			let e = (j + 1 + hashes).min(c.len());
			blank(&mut out, i, e);
			i = e;
		} else if c[i] == '"' {
			let mut j = i + 1;
			while j < c.len() && c[j] != '"' {
				j += if c[j] == '\\' { 2 } else { 1 };
			}
			blank(&mut out, i, j + 1);
			i = j + 1;
		} else if c[i] == '\'' && next == Some('\\') {
			let mut j = i + 3;
			while j < c.len() && c[j] != '\'' && c[j] != '\n' {
				j += 1;
			}
			blank(&mut out, i, j + 1);
			i = j + 1;
		} else if c[i] == '\'' && c.get(i + 2) == Some(&'\'') {
			blank(&mut out, i, i + 3);
			i += 3;
		} else {
			i += 1;
		}
	}
	out
}

/// Index after the bracket that closes the one at `open`.
fn close_of(c: &[char], open: usize) -> usize {
	let mut depth = 0i32;
	let mut j = open;
	while j < c.len() {
		match c[j] {
			'(' | '[' | '{' => depth += 1,
			')' | ']' | '}' => {
				depth -= 1;
				if depth == 0 {
					return j + 1;
				}
			}
			_ => {}
		}
		j += 1;
	}
	c.len()
}

fn skip_ws(c: &[char], mut i: usize) -> usize {
	while i < c.len() && c[i].is_whitespace() {
		i += 1;
	}
	i
}

fn ident_at(c: &[char], i: usize) -> String {
	c[i..].iter().take_while(|&&x| is_id(x)).collect()
}

/// Blanks every `#[cfg(test)] mod x { .. }` block (the tests may use raw addresses).
fn blank_test_mods(c: &mut [char]) {
	let mut i = 0;
	while i < c.len() {
		if starts(c, i, "#[cfg(test)]") {
			let mut j = skip_ws(c, i + 12);
			for kw in ["pub(crate)", "pub(super)", "pub"] {
				if starts(c, j, kw) {
					j = skip_ws(c, j + kw.len());
					break;
				}
			}
			if starts(c, j, "mod ") {
				if let Some(b) = c[j..].iter().position(|&x| x == '{' || x == ';') {
					if c[j + b] == '{' {
						let e = close_of(c, j + b);
						for x in c.iter_mut().take(e).skip(i) {
							if *x != '\n' {
								*x = ' ';
							}
						}
						i = e;
						continue;
					}
				}
			}
		}
		i += 1;
	}
}

// ---- the scanner ---------------------------------------------------------------------------------------------------

/// The `disc::` functions whose call maps its argument. A bare call counts only where the file imports the function from
/// `disc` and defines no local item of that name; a path call only as `disc::name(`; a method call never (`METHOD_MAPPERS`).
/// `g` is not here: it maps only `G_LO..=G_HI` (see `G_MAPPER`).
const MAPPERS: [&str; 2] = ["va", "va_level"];
/// `disc::g` maps only the boot globals window `G_LO..=G_HI`; any other literal inside `g(..)` stays a raw PAL address
/// (`Scanned.gmap`, test `g_maps_only_its_window`). The method `modcam.rs` `Lv::g` is a general mapper (it calls `va`).
const G_MAPPER: &str = "g";
/// Methods of the tree whose body calls `va`, so their argument is mapped (proved by `method_mappers_call_va`).
const METHOD_MAPPERS: [(&str, &str); 1] = [("retail/modcam.rs", "g")];

/// R8-4: the only `_USA` items that have no PAL twin in their own file: the PAL tables are in fmv_bridge.rs.
const LONE_USA: [(&str, &str); 2] = [("retail/disc.rs", "PLAYERS_USA"), ("retail/disc.rs", "HOOKS_USA")];

type Span = (usize, usize);

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Hit {
	pub owner: String,
	pub lit: u32,
	pub line: usize,
}

struct Item {
	name: String,
	start: usize,
	end: usize,
	/// The declared type names `Hand` (a table of hooks that has to go through `disc::hands`).
	hand: bool,
}

/// One source file read for the guards: code only, the argument spans of the `disc::` calls, the const / static items and
/// the `use` statements (span and text).
struct Scanned {
	code: Vec<char>,
	map: Vec<Span>,
	/// The arguments of `disc::g(..)` calls: they map only `G_LO..=G_HI`.
	gmap: Vec<Span>,
	pick: Vec<Span>,
	hands: Vec<Span>,
	items: Vec<Item>,
	uses: Vec<(Span, String)>,
	/// The `_USA` items of this file that have no PAL twin in it (`LONE_USA`).
	lone_usa: Vec<&'static str>,
}

fn has_word(text: &str, w: &str) -> bool {
	text.split(|c: char| !is_id(c)).any(|x| x == w)
}

/// Calls `name(` / `name::<..>(` of `names` for which `ok(index of the name, name)` holds: the span of their arguments.
fn call_spans(c: &[char], names: &[&str], ok: &dyn Fn(usize, &str) -> bool) -> Vec<Span> {
	let mut out = Vec::new();
	let mut i = 0;
	while i < c.len() {
		if is_id(c[i]) && (i == 0 || !is_id(c[i - 1])) {
			let name = ident_at(c, i);
			let end = i + name.chars().count();
			if names.contains(&name.as_str()) && ok(i, &name) {
				let mut j = skip_ws(c, end);
				if starts(c, j, "::<") {
					j = skip_ws(c, c[j..].iter().position(|&x| x == '>').map_or(j, |p| j + p + 1));
				}
				if c.get(j) == Some(&'(') {
					out.push((j + 1, close_of(c, j) - 1));
				}
			}
			i = end;
		} else {
			i += 1;
		}
	}
	out
}

/// The `use` statements of the code: span and text.
fn find_uses(c: &[char]) -> Vec<(Span, String)> {
	let mut out = Vec::new();
	let mut i = 0;
	while i < c.len() {
		if is_id(c[i]) && (i == 0 || !is_id(c[i - 1])) {
			let name = ident_at(c, i);
			if name == "use" {
				let e = c[i..].iter().position(|&x| x == ';').map_or(c.len(), |p| i + p);
				out.push(((i, e), c[i..e].iter().collect()));
				i = e;
			} else {
				i += name.chars().count();
			}
		} else {
			i += 1;
		}
	}
	out
}

/// The code has `let name` / `let mut name` / `fn name`: a bare call of `name` may be that item, not the `disc::` function.
fn defines_locally(c: &[char], name: &str) -> bool {
	let mut i = 0;
	while i < c.len() {
		if is_id(c[i]) && (i == 0 || !is_id(c[i - 1])) {
			let w = ident_at(c, i);
			let end = i + w.chars().count();
			if w == "let" || w == "fn" {
				let mut j = skip_ws(c, end);
				if ident_at(c, j) == "mut" {
					j = skip_ws(c, j + 3);
				}
				if ident_at(c, j) == name {
					return true;
				}
			}
			i = end;
		} else {
			i += 1;
		}
	}
	false
}

/// Whether the call of `name` whose identifier starts at `i` is the `disc::` function of that name: `disc::name(`, or a bare
/// `name(` in a file that imports it (or `disc::*`) and has no local `let name` / `fn name`; never `x.name(` unless
/// `METHOD_MAPPERS` lists it for this file.
fn is_disc_fn(c: &[char], uses: &[(Span, String)], inherited: &dyn Fn(&str) -> bool, rel: &str, i: usize, name: &str) -> bool {
	let mut p = i;
	while p > 0 && c[p - 1].is_whitespace() {
		p -= 1;
	}
	if p > 0 && c[p - 1] == '.' {
		return METHOD_MAPPERS.contains(&(rel, name));
	}
	if p >= 2 && c[p - 1] == ':' && c[p - 2] == ':' {
		let mut q = p - 2;
		while q > 0 && c[q - 1].is_whitespace() {
			q -= 1;
		}
		let mut s = q;
		while s > 0 && is_id(c[s - 1]) {
			s -= 1;
		}
		return c[s..q].iter().collect::<String>() == "disc";
	}
	(disc_import(uses, name) || inherited(name)) && !defines_locally(c, name)
}

/// A `use` statement of `uses` imports `name` from `disc` (or globs `disc`).
fn disc_import(uses: &[(Span, String)], name: &str) -> bool {
	uses.iter().any(|(_, t)| has_word(t, "disc") && (has_word(t, name) || t.contains("disc::*")))
}

/// The parent module file of `rel` among the scanned files (retail/foo.rs -> retail.rs or retail/mod.rs).
fn parent_of(rel: &str, all: &BTreeMap<String, Vec<(Span, String)>>) -> Option<String> {
	let dir = rel.rsplit_once('/')?.0;
	[format!("{dir}.rs"), format!("{dir}/mod.rs")].into_iter().find(|c| c != rel && all.contains_key(c))
}

/// File `rel` has `name` from `disc` without importing it itself: through `use super::*` / `use super::{name}` of a parent
/// module that imports it (the retail submodules get `va` this way), or `use crate::retail::name`.
fn inherits(rel: &str, name: &str, all: &BTreeMap<String, Vec<(Span, String)>>) -> bool {
	let Some(u) = all.get(rel) else { return false };
	if disc_import(u, name) {
		return true;
	}
	let via = |w: &str| u.iter().any(|(_, t)| has_word(t, w) && (has_word(t, name) || t.contains(&format!("{w}::*"))));
	(via("super") && parent_of(rel, all).is_some_and(|p| inherits(&p, name, all))) || (rel != "retail.rs" && via("retail") && inherits("retail.rs", name, all))
}

/// The const / static items (UPPER_CASE name followed by `:`) of the code.
fn find_items(code: &[char]) -> Vec<Item> {
	let mut items = Vec::new();
	let mut i = 0;
	while i < code.len() {
		if is_id(code[i]) && (i == 0 || !is_id(code[i - 1])) {
			let name = ident_at(code, i);
			let end = i + name.chars().count();
			if name == "const" || name == "static" {
				let mut j = skip_ws(code, end);
				if starts(code, j, "mut ") {
					j = skip_ws(code, j + 4);
				}
				let item = ident_at(code, j);
				let after = skip_ws(code, j + item.chars().count());
				if !item.is_empty() && item.chars().all(|x| x.is_ascii_uppercase() || x.is_ascii_digit() || x == '_') && code.get(after) == Some(&':') {
					let mut depth = 0i32;
					let mut e = after;
					let mut eq = None;
					while e < code.len() && !(code[e] == ';' && depth == 0) {
						match code[e] {
							'(' | '[' | '{' => depth += 1,
							')' | ']' | '}' => depth -= 1,
							'=' if depth == 0 && eq.is_none() => eq = Some(e),
							_ => {}
						}
						if depth < 0 {
							break;
						}
						e += 1;
					}
					let ty: String = code[after..eq.unwrap_or(e)].iter().collect();
					items.push(Item { name: item, start: i, end: e, hand: has_word(&ty, "Hand") });
				}
			}
			i = end;
		} else {
			i += 1;
		}
	}
	items
}

fn scan_prepare(src: &str, rel: &str) -> Scanned {
	scan_prepare_in(src, rel, &|_| false)
}

/// `inherited(name)`: the file gets the `disc::` function `name` through its parent module (see `inherits`).
fn scan_prepare_in(src: &str, rel: &str, inherited: &dyn Fn(&str) -> bool) -> Scanned {
	let mut code = blank_text(src);
	blank_test_mods(&mut code);
	let uses = find_uses(&code);
	let items = find_items(&code);
	let real = |i: usize, n: &str| is_disc_fn(&code, &uses, inherited, rel, i, n);
	let mut map = call_spans(&code, &MAPPERS, &real);
	// R8-3: in a `METHOD_MAPPERS` file only the method call `.g(..)` is a general mapper; `disc::g(..)` and a bare imported
	// `g(..)` are window calls like everywhere else.
	let is_method = |i: usize| code[..i].iter().rev().find(|x| !x.is_whitespace()) == Some(&'.');
	let method_file = METHOD_MAPPERS.contains(&(rel, G_MAPPER));
	if method_file {
		map.extend(call_spans(&code, &[G_MAPPER], &|i, n| is_method(i) && real(i, n)));
	}
	let gmap = call_spans(&code, &[G_MAPPER], &|i, n| !(method_file && is_method(i)) && real(i, n));
	let (pick, hands) = (call_spans(&code, &["pick"], &real), call_spans(&code, &["hands"], &real));
	// `at` maps only where the file says so: a `let at = |..| .. va(..)` closure on one line.
	let at_maps = (0..code.len()).filter(|&p| starts(&code, p, "let at = |") && (p == 0 || !is_id(code[p - 1]))).any(|p| {
		let line_end = code[p..].iter().position(|&x| x == '\n').map_or(code.len(), |q| p + q);
		map.iter().any(|&(a, _)| (p..line_end).contains(&a))
	});
	if at_maps {
		map.extend(call_spans(&code, &["at"], &|_, _| true));
	}
	Scanned { code, map, gmap, pick, hands, items, uses, lone_usa: LONE_USA.iter().filter(|&&(f, _)| f == rel).map(|&(_, n)| n).collect() }
}

fn line_of(c: &[char], at: usize) -> usize {
	1 + c[..at].iter().filter(|&&x| x == '\n').count()
}

/// Value and end index of the hex literal (`0x` / `0X`, digits and `_`, optional type suffix) at `i`.
fn hex_at(c: &[char], i: usize) -> Option<(u64, usize)> {
	if !(c[i] == '0' && matches!(c.get(i + 1), Some('x' | 'X')) && (i == 0 || !is_id(c[i - 1]))) {
		return None;
	}
	let digits: String = c[i + 2..].iter().take_while(|&&x| x.is_ascii_hexdigit() || x == '_').collect();
	let end = i + 2 + digits.chars().count();
	let suffix: String = c[end..].iter().take_while(|&&x| is_id(x)).collect();
	let int_suffix = suffix.strip_prefix(['u', 'i']).is_some_and(|n| matches!(n, "8" | "16" | "32" | "64" | "128" | "size"));
	if !(suffix.is_empty() || int_suffix) {
		return None;
	}
	let plain: String = digits.chars().filter(|&x| x != '_').collect();
	Some((u64::from_str_radix(&plain, 16).ok()?, end))
}

/// R8-2: the argument `a..b` (inside the parentheses of a call) is exactly one hex literal, spaces aside: its start index
/// and value. `0x15f5e4 + 8` is not (the literal only starts the argument).
fn arg_literal(c: &[char], a: usize, b: usize) -> Option<(usize, u32)> {
	let start = skip_ws(c, a);
	let mut end = b;
	while end > start && c[end - 1].is_whitespace() {
		end -= 1;
	}
	if start >= end {
		return None;
	}
	let (v, lit_end) = hex_at(c, start)?;
	(lit_end == end).then_some((start, v as u32))
}

/// The raw PAL overlay addresses (hex literals `0x` / `0X` of any number of digits and leading zeros whose value is in
/// 0x15ef00..0x3fffff, also the start of a range) in `src` that are not an argument of a mapping call, not in a
/// `#[cfg(test)]` module and not in an item that has a `_USA` twin. A `Hand { .. }` / `Player { .. }` line hides nothing;
/// the generated hook tables are items typed `Hand` that `hand_routed(name)` says go only through `disc::hands`. `owner` is
/// the const / static they sit in, else "expr". Decimal literals are not scanned (192 hits when tried, all floats,
/// seeds and sizes: they would be false positives).
pub(super) fn scan_source(src: &str) -> Vec<Hit> {
	scan_prepared(&scan_prepare(src, ""), &|_| false)
}

fn scan_prepared(s: &Scanned, hand_routed: &dyn Fn(&str) -> bool) -> Vec<Hit> {
	let c = &s.code;
	let has = |n: &str| s.items.iter().any(|o| o.name == n);
	// F6: an `X_USA` item is skipped only where its PAL twin `X` is in the same file (or is listed in `LONE_USA`); any other lone `_USA` is scanned.
	let twinned = |name: &str| name.strip_suffix("_USA").is_some_and(|b| s.lone_usa.contains(&name) || has(b)) || has(&format!("{name}_USA"));
	let mut out = Vec::new();
	let mut i = 0;
	while i + 2 < c.len() {
		let Some((v, end)) = hex_at(c, i) else {
			i += 1;
			continue;
		};
		if (0x15_ef00..0x40_0000).contains(&v) {
			let item = s.items.iter().find(|o| (o.start..o.end).contains(&i));
			// R8-2: a literal is mapped by `g` only as the whole argument of `g(..)`, and only inside the window.
			let in_g = (G_LO..=G_HI).contains(&(v as u32)) && s.gmap.iter().any(|&(a, b)| arg_literal(c, a, b).is_some_and(|(at, _)| at == i));
			let skip = in_g || s.map.iter().any(|&(a, b)| (a..b).contains(&i)) || item.is_some_and(|o| twinned(&o.name) || (o.hand && hand_routed(&o.name)));
			if !skip {
				out.push(Hit { owner: item.map_or("expr".to_string(), |o| o.name.clone()), lit: v as u32, line: line_of(c, i) });
			}
		}
		i = end.max(i + 2);
	}
	out
}

/// `disc::pick(&A, &B)` calls of `src` whose B is not `A_USA`.
fn bad_picks(src: &str) -> (usize, Vec<String>) {
	let c = blank_text(src);
	let (mut n, mut bad) = (0, Vec::new());
	let mut i = 0;
	while i < c.len() {
		if starts(&c, i, "disc::pick(") && (i == 0 || !is_id(c[i - 1])) {
			n += 1;
			let open = i + 10;
			let args: String = c[open + 1..close_of(&c, open) - 1].iter().collect();
			let parts: Vec<&str> = args.split(',').map(str::trim).collect();
			let name = |s: &str| s.trim_start_matches(['&', '*', ' ']).rsplit("::").next().unwrap_or("").to_string();
			if parts.len() != 2 || name(parts[1]) != format!("{}_USA", name(parts[0])) {
				bad.push(args.clone());
			}
		}
		i += 1;
	}
	(n, bad)
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
	let mut entries: Vec<_> = std::fs::read_dir(dir).expect("src folder").flatten().map(|e| e.path()).collect();
	entries.sort();
	for p in entries {
		if p.is_dir() {
			rs_files(&p, out);
		} else if p.extension().is_some_and(|e| e == "rs") {
			out.push(p);
		}
	}
}

/// (path relative to src with `/`, source) of every hand-written .rs file the scan covers: not the generated tables, not
/// this file, not test files, not disc.rs (the layer itself: `g`'s range edge).
fn source_files() -> Vec<(String, String)> {
	let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
	let mut paths = Vec::new();
	rs_files(&src, &mut paths);
	paths
		.into_iter()
		.filter_map(|p| {
			let rel = p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
			let name = rel.rsplit('/').next().unwrap().to_string();
			let skip = matches!(name.as_str(), "disc_map.rs" | "disc_hands.rs" | "disc_proven.rs" | "disc_guard_tests.rs" | "disc.rs" | "tests.rs") || name.ends_with("_tests.rs");
			(!skip).then(|| (rel, std::fs::read_to_string(&p).unwrap_or_default()))
		})
		.collect()
}

// ---- the allow-list ------------------------------------------------------------------------------------------------

#[derive(Debug)]
enum Why {
	/// Every literal is a core small-data address that is the same on both discs (disc_proven.rs holds it as scope 20, p -> p,
	/// and the map gives p in a run / gap / ext segment of all 21 scopes).
	Core,
	/// A const used only inside the arguments of a mapping call, everywhere in the tree (checked by `unwrapped_uses`).
	ViaVa,
	/// A const used only inside the arguments of calls of these local helpers (or of a mapping call), everywhere in the tree;
	/// each helper maps its address argument itself (checked by `unwrapped_uses` and `helpers_map`).
	Via(&'static [&'static str]),
	/// Anything else, with the reason it works on the USA disc. Not checked beyond its length: LEFT where no helper is named.
	Other(&'static str),
}
use Why::{Core, Other, Via, ViaVa};

/// (file, owner, number of literals, why). Owner = the const / static, or "expr" for a literal in a function body.
/// A new raw PAL address anywhere fails the guard until it goes through `va` or is added here with a true reason.
const ALLOWED: &[(&str, &str, usize, Why)] = &[
	("game/morph.rs", "expr", 1, Other("0x20207f is a colour word handed to ribbon(), not an address")),
	("game/shiprefl.rs", "IMAGE_BASE", 1, Core),
	("game/shiprefl.rs", "NORMALS", 1, Via(&["at"])),
	("game/shiprefl.rs", "POSITIONS", 1, Via(&["at"])),
	("game/shiprefl.rs", "QUADS", 1, Via(&["at"])),
	("gsfull/gs_mem.rs", "expr", 10, Other("0x3fffff is the 4 MB GS local-memory mask (22 bit), not an EE address")),
	("gsfull/gs_raster.rs", "expr", 2, Other("0x3fffff is the 4 MB GS local-memory mask (22 bit), not an EE address")),
	("game/transit.rs", "RIBBON_B", 3, Other("RGBA colour words of the ship ribbon written with leading zeros (0x0020_58b0), not addresses")),
	("hud.rs", "FONT_TABLES", 3, Other("file offsets into the boot ELF; the USA boot executable is read with a +0x80 shift and its 3 font tables are byte-equal to the PAL ones (tools/addr/font.py)")),
	("ocean.rs", "ARIDIA", 10, Other("descriptor addresses are read with va_level(level.id, ..) in Desc::read / elf_at (ocean.rs lines 171-179); the image_base fields are core addresses")),
	("ocean.rs", "GAP", 1, Other("compared with a PAL address only on the PAL path; elf_at returns through ov_bytes before it on the USA disc")),
	("ocean.rs", "KERWAN", 4, Other("descriptor addresses are read with va_level(level.id, ..) in Desc::read / elf_at (ocean.rs lines 171-179); the image_base fields are core addresses")),
	("omni/draw_w2s.rs", "MODE_WORD", 1, ViaVa),
	("omni/events.rs", "EXIT_WORD", 1, ViaVa),
	("omni/events.rs", "MODE_WORD", 1, ViaVa),
	("omni/native_host.rs", "EXE_DATA_END", 1, Other("edge inside the native-mod mapper relocate_in itself: below it the executable addresses are equal on both discs")),
	("omni/raycast.rs", "RECORD_VELDIN", 1, Other("only the argument of native_host::relocate_in(usa, level, ..), the native-mod PAL -> USA mapper")),
	("particles.rs", "expr", 1, Other("tex_def offset of Effect::Smoke2c, a variant nothing outside particles.rs ever constructs (hand-port leftover): no USA RAM reaches it")),
	("retail/aim.rs", "GAME_MODE", 1, ViaVa),
	("retail/debugkeys.rs", "SCAN_HI", 1, Other("debug-key RAM search window, a scan over RAM with no address meaning; covers both discs images")),
	("retail/debugkeys.rs", "SCAN_LO", 1, Other("debug-key RAM search window, a scan over RAM with no address meaning; covers both discs images")),
	("retail/efx_draw.rs", "SLOT_TABLE", 1, ViaVa),
	("retail/efx_own.rs", "expr", 1, Core),
	("retail/free_water.rs", "COL", 1, ViaVa),
	("retail/free_water.rs", "COUNTS", 1, ViaVa),
	("retail/free_water.rs", "FIX_ADD", 1, ViaVa),
	("retail/free_water.rs", "OWNER", 1, ViaVa),
	("retail/free_water.rs", "POS", 1, ViaVa),
	("retail/free_water.rs", "SCROLL", 1, ViaVa),
	("retail/free_water.rs", "SIG", 5, Other("its elements are only read as m.rd32(va(a)) in sig_ok (the loop variable a is the argument of va)")),
	("retail/free_water.rs", "UV", 1, ViaVa),
	("retail/fmv_bridge.rs", "PLAYERS", 42, Other("the PAL table of the movie players: on the USA disc players() (fmv_bridge.rs line 78) returns disc::PLAYERS_USA and the USA hooks registered are disc::HOOKS_USA, not HOOKS (lines 193-195)")),
	("retail/front_end.rs", "expr", 3, Core),
	("retail/fx_queue.rs", "expr", 5, Core),
	("retail/gscap.rs", "expr", 6, Other("0x3fffff is the 4 MB GS local-memory mask (22 bit), not an EE address")),
	("retail/gscap/draw.rs", "N", 2, Other("0x3fffff is the 4 MB GS local-memory mask (22 bit), not an EE address")),
	("retail/gscap/draw.rs", "expr", 1, Other("0x3fffff is the 4 MB GS local-memory mask (22 bit), not an EE address")),
	("retail/modcam.rs", "GAME_MODE", 1, ViaVa),
	("retail/modcam.rs", "MARKER_SLOT", 1, ViaVa),
	("retail/mouse_menu.rs", "FRONT_PAGE_PTR", 1, ViaVa),
	("retail/oldsim.rs", "FRAME_COUNTER", 1, ViaVa),
	("retail/oldsim.rs", "expr", 3, Core),
	("retail/pcctl.rs", "MENU_MODE", 1, ViaVa),
	("retail/pcext/camopts.rs", "MENU_MODE", 1, ViaVa),
	("retail/pcext/camopts.rs", "RATE_SCAN", 2, Other("content search for the three retail turn-rate words; the table lies at PAL 0x161d70..0x1626e8 on every level, far inside the window on both discs; 0x15ef00 is the first overlay address of both discs")),
	("retail/pcmods.rs", "expr", 3, Other("cfg(test) fn test_open: builds the pause page in a fake Mem at 0x300000, never runs in the game")),
	("retail/queue_b.rs", "BARRIER2_FN", 1, ViaVa),
	("retail/queue_b.rs", "BARRIER_AGE", 1, ViaVa),
	("retail/queue_b.rs", "BARRIER_C1", 1, ViaVa),
	("retail/queue_b.rs", "BARRIER_C2", 1, ViaVa),
	("retail/queue_b.rs", "BARRIER_EULER", 1, ViaVa),
	("retail/queue_b.rs", "BARRIER_FN", 1, ViaVa),
	("retail/queue_b.rs", "BARRIER_OFF", 1, ViaVa),
	("retail/queue_b.rs", "BARRIER_RIB", 1, ViaVa),
	("retail/queue_b.rs", "BARRIER_SCALE", 1, ViaVa),
	("retail/queue_b.rs", "BARRIER_STEP", 1, ViaVa),
	("retail/queue_b.rs", "COUNT_A", 1, Core),
	("retail/queue_b.rs", "COUNT_B", 1, Core),
	("retail/queue_b.rs", "DRAW_FLAGS", 1, Core),
	("retail/queue_b.rs", "PASS_2D", 1, ViaVa),
	("retail/queue_b.rs", "expr", 4, Other("0x166c00 is gp (reginfo ri_gp_value, the same on the USA disc); 0x15ef00..0x300000 is a range test of a word read from RAM (is it an overlay pointer), the arc tables lie below 0x1f0000")),
	("retail/queue_fxa.rs", "BOLT_FN", 1, ViaVa),
	("retail/queue_fxa.rs", "GP", 1, Other("gp is 0x166c00 on both discs (ri_gp_value of both boot ELFs); the offsets added are the running USA function own lwc1 words")),
	("retail/queue_fxa.rs", "expr", 3, Other("0x170000 only splits the camera vector (0x167540) from the corner table (0x1d1c80), both read from the running USA code; far from the split; 0x15ef00 starts a range test of a word read from RAM (is it an overlay pointer)")),
	("retail/queue_fxb.rs", "GP", 1, Other("gp is 0x166c00 on both discs (ri_gp_value of both boot ELFs); every use adds a fixed offset and maps it with va(GP - ..) or sits in the PAL / _USA Cfg tables (tools/addr/twins.py)")),
	("retail/queue_q1.rs", "STRIP_DRAW", 1, ViaVa),
	("retail/queue_q1.rs", "expr", 5, Other("0x15f4xx are core counters (same on both discs), 0x166c00 is gp (same on the USA disc, reginfo ri_gp_value)")),
	("retail/queue_q2.rs", "expr", 5, Other("0x166c00 is gp (same on the USA disc); 0x170000 only splits the camera vector from the corner table, both read from the running USA code, far from the split; 0x15ef00 starts a range test of a word read from RAM (is it an overlay pointer)")),
	("retail/queue_story.rs", "expr", 2, Core),
	("retail/ram_globals.rs", "NO_PRESENT", 1, ViaVa),
	("retail/shipmouse.rs", "GAME_MODE", 1, ViaVa),
	("retail/snapshot.rs", "expr", 4, Core),
	("retail/strafe.rs", "GAME_MODE", 1, ViaVa),
	("retail/title_boot.rs", "FRONT_MODE", 1, ViaVa),
	("retail/title_boot.rs", "PAGE_QUIT", 1, ViaVa),
	("retail/title_boot.rs", "REQ_FLAG", 1, ViaVa),
	("retail/title_boot.rs", "REQ_LEVEL", 1, ViaVa),
	("retail/warp.rs", "expr", 8, Other("0x1f0000..0x280000 is a content-hash window of RAM (no address meaning), 0x166c00 / 0x166c40 are gp values, 0x15f424 / 0x15f3fc core counters, 0x308000 / 0x2058b0 colours")),
	("shipgloss.rs", "COLOURS", 1, Via(&["u32_at", "vecs"])),
	("shipgloss.rs", "COUNTS", 1, Via(&["u32_at", "vecs"])),
	("shipgloss.rs", "IMAGE_BASE", 1, Core),
	("shipgloss.rs", "QUADS", 1, Via(&["u32_at", "vecs"])),
	("shipgloss.rs", "TABLE_A", 1, Via(&["u32_at", "vecs"])),
	("shipgloss.rs", "TABLE_B", 1, Via(&["u32_at", "vecs"])),
	("shipgloss.rs", "TABLE_D", 1, Via(&["u32_at", "vecs"])),
	("spurev.rs", "expr", 2, Other("0x3fffff is the 4 MB GS local-memory mask (22 bit), not an EE address")),
	("strips.rs", "ENV_COLOUR", 1, Via(&["word", "at"])),
	("strips.rs", "ENV_COUNTS", 1, Via(&["word", "at"])),
	("strips.rs", "ENV_NORMAL", 1, Via(&["word", "at"])),
	("strips.rs", "ENV_POS", 1, Via(&["word", "at"])),
	("strips.rs", "IMAGE_BASE", 1, Core),
	("strips.rs", "STRIP_RECORDS", 1, Via(&["parse_records", "word", "at"])),
	("strips.rs", "TABLE_1225", 1, Other("handed to novalis.rs (parse_strips callers: line 254 destructures the (base, count) pair) which passes base to strips::parse_records, and parse_records reads every field only through word() / at(), which map with va_level(1, ..) + ov_bytes (Novalis); the consts are NOT read through word/at themselves, so the guard cannot follow them across the file (LEFT as Other)")),
	("strips.rs", "TABLE_676", 1, Other("handed to novalis.rs (parse_strips callers: line 254 destructures the (base, count) pair) which passes base to strips::parse_records, and parse_records reads every field only through word() / at(), which map with va_level(1, ..) + ov_bytes (Novalis); the consts are NOT read through word/at themselves, so the guard cannot follow them across the file (LEFT as Other)")),
	("strips.rs", "TABLE_678", 1, Other("handed to novalis.rs (parse_strips callers: line 254 destructures the (base, count) pair) which passes base to strips::parse_records, and parse_records reads every field only through word() / at(), which map with va_level(1, ..) + ov_bytes (Novalis); the consts are NOT read through word/at themselves, so the guard cannot follow them across the file (LEFT as Other)")),
	("texpack/dds.rs", "expr", 1, Other("0x200000 is a bit of the DDS header flags word, not an address")),
	("viewer/front_mirror.rs", "P_LANGUAGE", 1, Other("compared with the unva(page) of the running USA menu page, which maps it back to PAL (proven in the front and bootov scopes)")),
	("viewer/front_mirror.rs", "P_LOAD", 1, Other("compared with the unva(page) of the running USA menu page, which maps it back to PAL (proven in the front and bootov scopes)")),
	("viewer/front_mirror.rs", "P_NEW", 1, Other("compared with the unva(page) of the running USA menu page, which maps it back to PAL (proven in the front and bootov scopes)")),
	("viewer/front_mirror.rs", "P_OPTIONS", 2, Other("compared with the unva(page) of the running USA menu page, which maps it back to PAL (proven in the front and bootov scopes)")),
	("viewer/front_mirror.rs", "P_OVERWRITE", 1, Other("compared with the unva(page) of the running USA menu page, which maps it back to PAL (proven in the front and bootov scopes)")),
	("viewer/front_mirror.rs", "P_ROOT", 1, Other("compared with the unva(page) of the running USA menu page, which maps it back to PAL (proven in the front and bootov scopes)")),
	("viewer/front_mirror.rs", "P_SOUND", 1, Other("compared with the unva(page) of the running USA menu page, which maps it back to PAL (proven in the front and bootov scopes)")),
	("viewer/front_mirror.rs", "P_VIDEO", 1, Other("compared with the unva(page) of the running USA menu page, which maps it back to PAL (proven in the front and bootov scopes)")),
	("water.rs", "IMAGE_BASE", 1, Core),
	("water.rs", "NOVALIS", 2, Other("descriptor addresses are read through water.rs at(), which on the USA disc maps with va_level(1, ..) + ov_bytes (line 124); passed only to Water::new")),
];

/// Which argument spans make a use of an item fine.
#[derive(Clone, Copy)]
enum Wrap {
	/// The arguments of `disc::va` / `va_level` / `g` (and `at` where the file defines it as one).
	Map,
	/// The arguments of `disc::pick`.
	Pick,
	/// The arguments of `disc::hands`.
	Hands,
	/// `Map` plus the arguments of calls of these local helpers (which map their address argument themselves).
	Via(&'static [&'static str]),
}

fn spans_of(s: &Scanned, w: Wrap) -> Vec<Span> {
	match w {
		Wrap::Map => s.map.iter().chain(&s.gmap).copied().collect(),
		Wrap::Pick => s.pick.clone(),
		Wrap::Hands => s.hands.clone(),
		Wrap::Via(names) => s.map.iter().chain(&s.gmap).copied().chain(call_spans(&s.code, names, &|_, _| true)).collect(),
	}
}

/// The module name a file defines: its stem, or its folder for `mod.rs`.
fn module_stem(rel: &str) -> String {
	let mut it = rel.trim_end_matches(".rs").rsplit('/');
	let last = it.next().unwrap_or("");
	if last == "mod" { it.next().unwrap_or("").to_string() } else { last.to_string() }
}

/// Whether file `rel` can write the item `name` of `home` without a path: it is `home`, or a `use` statement names the item
/// (grouped, `self`-less) or globs the module `home`, or it is a child module of `home` with a `use super::..`.
fn names_bare(s: &Scanned, rel: &str, home: &str, name: &str) -> bool {
	if rel == home {
		return true;
	}
	let stem = module_stem(home);
	let child = rel.starts_with(&format!("{}/", home.trim_end_matches(".rs")));
	s.uses.iter().any(|(_, t)| {
		let glob = t.contains("::*");
		(has_word(t, &stem) && (has_word(t, name) || glob)) || (child && has_word(t, "super") && (has_word(t, name) || glob))
	})
}

/// The use at `i` is written `<stem>::NAME` (or `super::NAME` in a child module of the home file).
fn path_to(c: &[char], i: usize, stem: &str, child: bool) -> bool {
	if !(i >= 2 && c[i - 1] == ':' && c[i - 2] == ':') {
		return false;
	}
	let mut s = i - 2;
	while s > 0 && is_id(c[s - 1]) {
		s -= 1;
	}
	let seg: String = c[s..i - 2].iter().collect();
	seg == stem || (child && seg == "super")
}

/// The places (file, line) where item `name` of `home` is used outside the argument spans of `w`: in its own file by the bare
/// name, in other files through a path (`module::NAME`) or after a `use` that names it or globs its module.
fn unwrapped_uses(files: &[(String, Scanned)], home: &str, name: &str, w: Wrap) -> Vec<(String, usize)> {
	let mut out = Vec::new();
	let n = name.chars().count();
	for (rel, s) in files {
		let c = &s.code;
		let shadowed = rel != home && s.items.iter().any(|o| o.name == name);
		let bare = !shadowed && names_bare(s, rel, home, name);
		let stem = module_stem(home);
		let child = rel.starts_with(&format!("{}/", home.trim_end_matches(".rs")));
		let mut spans: Option<Vec<Span>> = None;
		for i in 0..c.len().saturating_sub(n - 1) {
			let hit = starts(c, i, name) && !c.get(i + n).is_some_and(|&x| is_id(x)) && (i == 0 || !is_id(c[i - 1]));
			if !hit || !(bare || (!shadowed && path_to(c, i, &stem, child))) {
				continue;
			}
			let own_def = s.items.iter().any(|o| o.name == name && (o.start..o.end).contains(&i));
			if own_def || s.uses.iter().any(|((a, b), _)| (*a..*b).contains(&i)) {
				continue;
			}
			if !spans.get_or_insert_with(|| spans_of(s, w)).iter().any(|&(a, b)| (a..b).contains(&i)) {
				out.push((rel.clone(), line_of(c, i)));
			}
		}
	}
	out
}

/// The byte range (char indexes) of the definition of the local `fn name` / `let name = ..;` in the code.
fn def_extent(c: &[char], name: &str) -> Option<Span> {
	let mut i = 0;
	while i < c.len() {
		if is_id(c[i]) && (i == 0 || !is_id(c[i - 1])) {
			let w = ident_at(c, i);
			let end = i + w.chars().count();
			if w == "fn" || w == "let" {
				let mut j = skip_ws(c, end);
				if ident_at(c, j) == "mut" {
					j = skip_ws(c, j + 3);
				}
				if ident_at(c, j) == name {
					if w == "fn" {
						let open = j + c[j..].iter().position(|&x| x == '{')?;
						return Some((j, close_of(c, open)));
					}
					let mut depth = 0i32;
					let mut e = j;
					while e < c.len() && !(c[e] == ';' && depth == 0) {
						match c[e] {
							'(' | '[' | '{' => depth += 1,
							')' | ']' | '}' => depth -= 1,
							_ => {}
						}
						e += 1;
					}
					return Some((j, e));
				}
			}
			i = end;
		} else {
			i += 1;
		}
	}
	None
}

/// Each helper of `helpers` that file `s` defines maps its address argument itself: its body holds a mapping call, or a call
/// of another of the helpers. The error names the helper that does not.
fn helpers_map(s: &Scanned, helpers: &[&str]) -> Result<(), String> {
	for h in helpers {
		let Some((a, b)) = def_extent(&s.code, h) else { return Err(format!("`{h}` is not defined in the file")) };
		let others: Vec<&str> = helpers.iter().copied().filter(|x| x != h).collect();
		let maps = s.map.iter().any(|&(x, _)| (a..b).contains(&x)) || call_spans(&s.code, &others, &|_, _| true).iter().any(|&(x, _)| (a..b).contains(&x));
		if !maps {
			return Err(format!("`{h}` has no mapping call and calls no other helper"));
		}
	}
	Ok(())
}

/// The `use` statements of a source file (no scan of the rest).
fn file_uses(src: &str) -> Vec<(Span, String)> {
	let mut code = blank_text(src);
	blank_test_mods(&mut code);
	find_uses(&code)
}

fn scan_tree() -> (Vec<(String, Scanned)>, BTreeMap<(String, String), Vec<Hit>>) {
	let sources = source_files();
	let all_uses: BTreeMap<String, Vec<(Span, String)>> = sources.iter().map(|(rel, src)| (rel.clone(), file_uses(src))).collect();
	let files: Vec<(String, Scanned)> = sources.iter().map(|(rel, src)| (rel.clone(), scan_prepare_in(src, rel, &|n| inherits(rel, n, &all_uses)))).collect();
	let mut groups: BTreeMap<(String, String), Vec<Hit>> = BTreeMap::new();
	for (rel, s) in &files {
		let routed = |name: &str| unwrapped_uses(&files, rel, name, Wrap::Hands).is_empty();
		for h in scan_prepared(s, &routed) {
			groups.entry((rel.clone(), h.owner.clone())).or_default().push(h);
		}
	}
	(files, groups)
}

/// Whether `lit` maps to itself, in a run / gap / ext segment, in every one of `tables`.
fn same_in_all(lit: u32, tables: &[&[Seg]]) -> bool {
	tables.iter().all(|t| {
		let (m, kind) = map_kind(t, lit);
		m == lit && kind <= 2
	})
}

/// A core small-data address: a proven row (20, p, p) and the same address in all 21 scopes (every level, front, boot).
fn core_proven(lit: u32) -> bool {
	let tables: Vec<&[Seg]> = (0..=20u8).map(segs_of).collect();
	PROVEN.iter().any(|&(sc, p, u, _)| sc == 20 && p == lit && u == lit) && same_in_all(lit, &tables)
}

// ---- the tests -----------------------------------------------------------------------------------------------------

/// a. The scanner on seeded sources: it flags what it must and passes what it must.
#[test]
fn scanner_flags_a_seeded_raw_address() {
	let hits = |s: &str| scan_source(s).into_iter().map(|h| (h.owner, h.lit)).collect::<Vec<_>>();
	let expr = |v: u32| ("expr".to_string(), v);
	// Seeded bad cases: a raw address in a const and in an expression.
	assert_eq!(hits("const SEEDED_BAD: u32 = 0x1c4030;"), vec![("SEEDED_BAD".to_string(), 0x1c4030)]);
	assert_eq!(hits("fn f(m: &M) { m.rd32(0x1c4030 + 4); }"), vec![expr(0x1c4030)]);
	assert_eq!(hits("fn f() { let g = if a { 0x166c40u32 } else { 0x166c00 }; }").len(), 2);
	// B1.3: other literal shapes (7 / 8 digits, capital X, underscores, usize suffix, range starts) are the same address.
	for lit in ["0x01c4030", "0x001c4030", "0X1C4030", "0x1c_4030", "0x1c4030usize", "0x1c4030_u32"] {
		assert_eq!(hits(&format!("fn f(m: &M) {{ m.rd32({lit}); }}")), vec![expr(0x1c4030)], "{lit} is a raw address");
	}
	assert_eq!(hits("fn f() { for a in 0x1c4030..0x1c4040 {} }"), vec![expr(0x1c4030), expr(0x1c4040)], "a range start is a raw address");
	// B1.1: `unva` and `ov_bytes` take an address of the disc that runs, so a PAL literal in them is a raw address.
	assert_eq!(hits("fn f() { unva(0x1d4548); ov_bytes(i, 0x1c4030, 4); }"), vec![expr(0x1d4548), expr(0x1c4030)]);
	// B1.2: only the real `disc::` functions map. A local closure / method / foreign path of the same name does not.
	let import = "use crate::retail::disc::{usa, g};\n";
	assert_eq!(hits("fn f() { let g = |x: u32| x; g(0x1c4030); }").len(), 1, "no import: `g` is not disc::g");
	assert_eq!(hits(&format!("{import}fn f() {{ let g = |x: u32| x; g(0x1c4030); }}")).len(), 1, "a local `let g` shadows the import");
	assert!(hits(&format!("{import}fn f() {{ g(0x15f5a4); }}")).is_empty(), "disc::g imported and not shadowed");
	assert!(hits("fn f() { crate::retail::disc::g(0x15f5a4); disc::va(0x1c4030); }").is_empty());
	assert_eq!(hits("fn f() { other::va(0x1c4030); Self::g(0x1c4030); }").len(), 2, "a path call maps only as disc::");
	assert_eq!(hits("fn f() { l.g(0x167200); x.va(0x1c4030); }").len(), 2, "a method call never maps");
	let modcam = scan_prepared(&scan_prepare("fn f() { l.g(0x167200); l.va(0x1c4030); }", "retail/modcam.rs"), &|_| false);
	assert_eq!(modcam.len(), 1, "only the listed method (modcam Lv::g) maps");
	// Seeded good cases: mapped, twinned, text, test modules, out of range.
	assert!(hits("fn f(m: &M) { m.rd32(disc::va(0x1c4030)); m.rd32(disc::g(0x15f5a4)); }").is_empty());
	assert!(hits("fn f() { crate::retail::disc::va_level(1, 0x1c4030); disc::va::<u32>(0x1c4030); }").is_empty());
	assert!(hits("const A: u32 = 0x1c4030;\nconst A_USA: u32 = 0x1c40b0;").is_empty());
	assert!(hits("// 0x1c4030\n/* 0x1c4030 */ fn f() { let s = \"0x1c4030\"; let r = r#\"0x1c4030\"#; }").is_empty());
	assert!(hits("#[cfg(test)]\nmod tests { fn f() { m.rd32(0x1c4030); } }").is_empty());
	assert_eq!(hits("fn f() { a(0x15eeff); b(0x400000); c(0x15ef00..0x300000); d(0x1c40); e(0x1c4030g); }").len(), 2, "the range start and end are raw addresses");
	// B1.5: a `Hand { .. }` / `Player { .. }` line hides nothing; a table typed `Hand` passes only when it goes through disc::hands.
	assert_eq!(hits("fn f() { let h = Hand { entry: 0x2f66f8, end: 0x2f6700 }; let p = Player { entry: 0x2edd28 }; }").len(), 3, "entry and end of the Hand line, entry of the Player line");
	let table = scan_prepare("static H: [Hand; 1] = [Hand { entry: 0x2f66f8, end: 0x2f6700 }];", "");
	assert_eq!(scan_prepared(&table, &|_| false).len(), 2, "a hook table nothing routes through disc::hands: entry and end are raw addresses");
	assert!(scan_prepared(&table, &|_| true).is_empty(), "a hook table that only goes through disc::hands is fine");
	// `at` maps only where the file says so.
	assert!(hits("use crate::retail::disc::va;\nfn f() { let at = |a: u32| va(a); m.rd32(at(0x1c4030)); }").is_empty());
	assert_eq!(hits("fn f() { m.rd32(at(0x1c4030)); }").len(), 1);
	assert_eq!(hits("fn f() { let at = |a: u32| a; m.rd32(at(0x1c4030)); }").len(), 1);
	// A wrapper's arguments end at its closing bracket.
	assert_eq!(hits("fn f() { m.rd32(disc::va(0x1c4030) + 0x1c4040); }"), vec![expr(0x1c4040)]);
}

fn two_files(a: &str, b: &str) -> Vec<(String, Scanned)> {
	vec![("a.rs".to_string(), scan_prepare(a, "a.rs")), ("b.rs".to_string(), scan_prepare(b, "b.rs"))]
}

/// a2. B1.4 / B1.6 on seeded sources: uses of an item are found across grouped and glob imports, and a twinned table is
/// only fine inside `disc::pick`.
#[test]
fn scanner_follows_imports_and_twins() {
	let a = "pub const X: u32 = 0x1c4030;\npub const Y: u32 = 0x1c4040;";
	let lines = |files: &[(String, Scanned)], w: Wrap| unwrapped_uses(files, "a.rs", "X", w).into_iter().map(|u| u.0).collect::<Vec<_>>();
	let grouped = two_files(a, "use super::a::{X, Y};\nfn f(m: &M) { m.rd32(X); }");
	assert_eq!(lines(&grouped, Wrap::Map), vec!["b.rs"], "a grouped import then a bare use");
	let glob = two_files(a, "use crate::a::*;\nfn f(m: &M) { m.rd32(X + 4); }");
	assert_eq!(lines(&glob, Wrap::Map), vec!["b.rs"], "a glob import then a bare use");
	let path = two_files(a, "fn f(m: &M) { m.rd32(a::X); }");
	assert_eq!(lines(&path, Wrap::Map), vec!["b.rs"], "a path use");
	let other = two_files(a, "use super::c::{X};\nfn f(m: &M) { m.rd32(X); }");
	assert!(lines(&other, Wrap::Map).is_empty(), "an X imported from another module is not this X");
	let fine = two_files(a, "use super::a::{X};\nuse crate::retail::disc::va;\nfn f(m: &M) { m.rd32(va(X)); }");
	assert!(lines(&fine, Wrap::Map).is_empty(), "inside va the use is fine");
	let helper = two_files(a, "use super::a::{X};\nfn f(m: &M) { m.rd32(word(X)); m.rd32(X); }");
	assert_eq!(lines(&helper, Wrap::Via(&["word"])).len(), 1, "inside the helper's arguments is fine, outside is not");
	let child = vec![("a.rs".to_string(), scan_prepare(a, "a.rs")), ("a/c.rs".to_string(), scan_prepare("use super::*;\nfn f() { m.rd32(X); }", "a/c.rs"))];
	assert_eq!(lines(&child, Wrap::Map), vec!["a/c.rs"], "a child module with `use super::*`");
	// Twins: every use of A and A_USA has to be inside disc::pick.
	let twin = "const A: [u32; 1] = [1];\nconst A_USA: [u32; 1] = [2];\n";
	let ok = vec![("t.rs".to_string(), scan_prepare(&format!("{twin}fn f() {{ let x = crate::retail::disc::pick(&A, &A_USA); }}"), "t.rs"))];
	assert!(unwrapped_uses(&ok, "t.rs", "A", Wrap::Pick).is_empty() && unwrapped_uses(&ok, "t.rs", "A_USA", Wrap::Pick).is_empty());
	let raw = vec![("t.rs".to_string(), scan_prepare(&format!("{twin}fn f() {{ let x = A[0]; let y = B_USA; }}"), "t.rs"))];
	assert_eq!(unwrapped_uses(&raw, "t.rs", "A", Wrap::Pick).len(), 1, "a raw use of a twinned table");
	// Hook tables: only inside disc::hands.
	let h = vec![("h.rs".to_string(), scan_prepare("static HANDS: [Hand; 0] = [];\nfn f() { register(HANDS); register(disc::hands(&HANDS)); }", "h.rs"))];
	assert_eq!(unwrapped_uses(&h, "h.rs", "HANDS", Wrap::Hands).len(), 1);
	// The helper proof: a helper that maps its argument, and one that does not.
	let s = scan_prepare("use crate::retail::disc::va_level;\nfn at(a: usize) -> usize { va_level(1, a) }\nfn word(a: usize) -> usize { at(a) }\nfn bad(a: usize) -> usize { a }", "x.rs");
	assert!(helpers_map(&s, &["at", "word"]).is_ok());
	assert!(helpers_map(&s, &["at", "bad"]).is_err(), "a helper that does not map is rejected");
	assert!(helpers_map(&s, &["missing"]).is_err());
}

/// a3. B3: the core proof needs the same address in every scope; seeded with one scope that shifts it.
#[test]
fn core_proof_needs_every_scope() {
	let seg = |lo, hi, d| Seg { lo, hi, d, d_hi: d, kind: 0 };
	let same = [seg(0x160000, 0x170000, 0)];
	let shifted = [seg(0x160000, 0x170000, 0x20)];
	let hole: [Seg; 0] = [];
	assert!(same_in_all(0x161000, &[&same, &same]));
	assert!(!same_in_all(0x161000, &[&same, &shifted]), "one shifted scope fails the proof");
	assert!(!same_in_all(0x161000, &[&same, &hole]), "an address outside a scope fails the proof");
}

/// b. Every raw PAL address in the source tree is one of `ALLOWED`, with the right count and a reason that holds.
#[test]
fn raw_pal_addresses_stay_in_the_allow_list() {
	let (files, groups) = scan_tree();
	let mut problems = Vec::new();
	let mut seen = std::collections::BTreeSet::new();
	for ((file, owner), hits) in &groups {
		let lits: Vec<String> = hits.iter().map(|h| format!("{:#x}@{}", h.lit, h.line)).collect();
		match ALLOWED.iter().find(|a| a.0 == file && a.1 == owner) {
			None => problems.push(format!("NEW  (\"{file}\", \"{owner}\", {}, Other(\"TODO\")),  // {}", hits.len(), lits.join(" "))),
			Some((_, _, n, why)) => {
				seen.insert((file.clone(), owner.clone()));
				if *n != hits.len() {
					problems.push(format!("COUNT (\"{file}\", \"{owner}\") is {} not {n}: {}", hits.len(), lits.join(" ")));
				}
				match why {
					Core => {
						for h in hits.iter().filter(|h| !core_proven(h.lit)) {
							problems.push(format!("CORE (\"{file}\", \"{owner}\") {:#x} is not a proven core address (disc_proven.rs scope 20 p -> p and the same run / gap / ext address in all 21 scopes)", h.lit));
						}
					}
					ViaVa => {
						let uses = unwrapped_uses(&files, file, owner, Wrap::Map);
						if owner == "expr" || !uses.is_empty() {
							problems.push(format!("VIAVA (\"{file}\", \"{owner}\") is used outside a mapping call at {uses:?}"));
						}
					}
					Via(helpers) => {
						let proof = files.iter().find(|(r, _)| r == file).map_or(Err("file not scanned".to_string()), |(_, s)| helpers_map(s, helpers));
						if let Err(e) = proof {
							problems.push(format!("VIA (\"{file}\", \"{owner}\") helper does not map: {e}"));
						}
						let uses = unwrapped_uses(&files, file, owner, Wrap::Via(helpers));
						if owner == "expr" || !uses.is_empty() {
							problems.push(format!("VIA (\"{file}\", \"{owner}\") is used outside {helpers:?} and the mapping calls at {uses:?}"));
						}
					}
					Other(r) => assert!(r.len() > 20, "the reason of {file} {owner} is too short to be one"),
				}
			}
		}
	}
	for a in ALLOWED {
		if !seen.contains(&(a.0.to_string(), a.1.to_string())) {
			problems.push(format!("STALE (\"{}\", \"{}\") is in ALLOWED but the scan no longer finds it: remove it", a.0, a.1));
		}
	}
	assert!(problems.is_empty(), "raw PAL addresses outside va / g / pick:\n{}", problems.join("\n"));
}

/// b2. B1.4: every use of a twinned table and of its `_USA` twin, in any file that can name it, is inside `disc::pick`.
#[test]
fn twinned_tables_are_only_used_through_pick() {
	let (files, _) = scan_tree();
	let (mut pairs, mut leaks) = (0, Vec::new());
	for (rel, s) in &files {
		for o in s.items.iter().filter(|o| !o.name.ends_with("_USA") && s.items.iter().any(|t| t.name == format!("{}_USA", o.name))) {
			pairs += 1;
			for name in [o.name.clone(), format!("{}_USA", o.name)] {
				for (f, l) in unwrapped_uses(&files, rel, &name, Wrap::Pick) {
					leaks.push(format!("{f}:{l}: {name} (defined in {rel}) is used outside disc::pick"));
				}
			}
		}
	}
	assert!(leaks.is_empty(), "twinned tables used without pick:\n{}", leaks.join("\n"));
	assert!(pairs >= 20, "the scan found only {pairs} twin pairs: it checks nothing");
}

/// b3. The methods `METHOD_MAPPERS` lists really call `va` in their body.
#[test]
fn method_mappers_call_va() {
	let (files, _) = scan_tree();
	for (rel, name) in METHOD_MAPPERS {
		let s = &files.iter().find(|(r, _)| r == rel).unwrap_or_else(|| panic!("{rel} not scanned")).1;
		let (a, b) = def_extent(&s.code, name).unwrap_or_else(|| panic!("{rel}: no fn {name}"));
		assert!(s.map.iter().any(|&(x, _)| (a..b).contains(&x)), "{rel}: fn {name} does not call va / va_level / g, so its callers pass an unmapped address");
	}
}

/// R8-3: in a `METHOD_MAPPERS` file only the method call `.g(..)` is a general mapper; `disc::g(..)` is checked like anywhere else.
#[test]
fn a_path_g_call_in_a_method_mapper_file_is_checked() {
	let hits = |rel: &str, s: &str| scan_prepared(&scan_prepare(s, rel), &|_| false).into_iter().map(|h| h.lit).collect::<Vec<_>>();
	let body = "use super::disc::va;
fn g(&self, a: u32) -> u32 { va(a) }
";
	assert!(hits("retail/modcam.rs", &format!("{body}fn h(&self) {{ self.g(0x1c4030); }}")).is_empty(), "the method `g` of modcam maps");
	assert_eq!(hits("retail/modcam.rs", &format!("{body}fn h() {{ crate::retail::disc::g(0x1c4030); }}")), vec![0x1c4030], "disc::g(..) maps only its window");
	assert!(hits("retail/modcam.rs", &format!("{body}fn h() {{ crate::retail::disc::g(0x15f5e4); }}")).is_empty(), "disc::g(..) in its window is fine");
	assert_eq!(hits("retail/other.rs", "fn h(&self) { self.g(0x1c4030); }"), vec![0x1c4030], "a method `g` elsewhere maps nothing");
	let s = scan_prepare(&format!("{body}fn h(&self) {{ self.g(1); disc::g(0x15f5e4); }}"), "retail/modcam.rs");
	assert_eq!((s.map.len(), s.gmap.len()), (2, 1), "one `va` plus the method call map generally, the path call is a window call");
}

/// F6: a lone `_USA` item (no PAL twin in its file) is a raw address like any other; a real pair is skipped.
#[test]
fn a_lone_usa_item_is_scanned() {
	let hits = |s: &str| scan_source(s).into_iter().map(|h| (h.owner, h.lit)).collect::<Vec<_>>();
	assert_eq!(hits("const NEW_TABLE_USA: u32 = 0x1c40b0;"), vec![("NEW_TABLE_USA".to_string(), 0x1c40b0)], "a lone _USA table is flagged");
	assert!(hits("const T: u32 = 0x1c4030; const T_USA: u32 = 0x1c4050;").is_empty(), "a pair is skipped");
	let lone = scan_prepared(&scan_prepare("const PLAYERS_USA: u32 = 0x1c40b0; const HOOKS_USA: u32 = 0x1c40b4;", "retail/disc.rs"), &|_| false);
	assert!(lone.is_empty(), "disc.rs holds PLAYERS_USA / HOOKS_USA, whose PAL tables are in fmv_bridge.rs");
	// R8-4: only those two names, not every `_USA` item of disc.rs.
	let lone = scan_prepared(&scan_prepare("const SOME_TABLE_USA: u32 = 0x1c40b0;", "retail/disc.rs"), &|_| false);
	assert_eq!(lone.iter().map(|h| h.lit).collect::<Vec<_>>(), vec![0x1c40b0], "a new lone _USA item of disc.rs is scanned");
	let other = scan_prepared(&scan_prepare("const PLAYERS_USA: u32 = 0x1c40b0;", "retail/x.rs"), &|_| false);
	assert_eq!(other.len(), 1, "PLAYERS_USA is exempt in disc.rs only");
}

/// F13: the b3 check on a seeded method: one that calls `va` passes, one that does not is found.
#[test]
fn a_method_mapper_without_va_is_found() {
	let calls_va = |src: &str| {
		let s = scan_prepare(&format!("use super::disc::va;\n{src}"), "retail/modcam.rs");
		let (a, b) = def_extent(&s.code, "g").expect("fn g");
		s.map.iter().any(|&(x, _)| (a..b).contains(&x))
	};
	assert!(calls_va("fn g(&self, a: u32) -> u32 { va(a) }"));
	assert!(!calls_va("fn g(&self, a: u32) -> u32 { a }"), "a `g` that only returns its argument maps nothing");
	assert!(!calls_va("fn g(&self, a: u32) -> u32 { 5 } fn h() { va(1) }"), "a va call outside fn g does not count");
}

/// The value of the const `name` of a scanned file: the hex literal after its `=`, only if nothing but `;` follows it
/// (`0x15f5d0 + 0x40` has no single value).
fn const_value(s: &Scanned, name: &str) -> Option<u32> {
	let it = s.items.iter().find(|o| o.name == name)?;
	let eq = s.code[it.start..it.end].iter().position(|&x| x == '=')? + it.start;
	let at = skip_ws(&s.code, eq + 1);
	let (v, end) = hex_at(&s.code, at)?;
	(s.code.get(skip_ws(&s.code, end)) == Some(&';')).then_some(v as u32)
}

/// The value of the argument `a..b` of a `g(..)` call: a hex literal that is the WHOLE argument, or a const of the same
/// file whose whole value is one literal.
fn g_arg_value(s: &Scanned, a: usize, b: usize) -> Option<u32> {
	if let Some((_, v)) = arg_literal(&s.code, a, b) {
		return Some(v);
	}
	let arg: String = s.code[a..b].iter().collect();
	let arg = arg.trim();
	const_value(s, arg.rsplit("::").next().unwrap_or(arg))
}

/// b4. F2: `disc::g` maps only `G_LO..=G_HI`, so a `g(..)` argument has to be a literal or a const of the same file whose
/// value is in that window; any other address in `g(..)` would reach the USA disc unmapped. Seeded cases first.
#[test]
fn g_maps_only_its_window() {
	let hits = |s: &str| scan_source(s).into_iter().map(|h| h.lit).collect::<Vec<_>>();
	let import = "use crate::retail::disc::g;\n";
	assert!(hits(&format!("{import}fn f() {{ g(0x15f5a4); g(0x15f5e7); g(0x15f5a0); }}")).is_empty(), "the window itself is mapped");
	assert_eq!(hits(&format!("{import}fn f() {{ g(0x1c4030); g(0x15f5e8); g(0x15f59f); }}")), vec![0x1c4030, 0x15f5e8, 0x15f59f], "outside the window `g` maps nothing");
	assert_eq!(hits("fn f() { disc::g(0x1c4030); disc::va(0x1c4030); }"), vec![0x1c4030], "only va maps an address outside the window");
	let src = format!("{import}const IN: u32 = 0x15f5a4; const OUT: u32 = 0x1c4030; fn f() {{ g(IN); g(OUT); }}");
	let s = scan_prepare(&src, "x.rs");
	assert_eq!(s.gmap.len(), 2);
	assert_eq!((const_value(&s, "IN"), const_value(&s, "OUT")), (Some(0x15f5a4), Some(0x1c4030)));
	// R8-2: an argument that only STARTS with an in-window literal (or a const that does) is not mapped by `g`.
	assert_eq!(hits(&format!("{import}fn f() {{ g(0x15f5e4 + 8); }}")), vec![0x15f5e4], "scan: g(0x15f5e4 + 8) is a raw address");
	let s = scan_prepare(&format!("{import}fn f() {{ g(0x15f5e4 + 8); g( 0x15f5e4 ); }}"), "x.rs");
	assert_eq!(s.gmap.iter().map(|&(a, b)| g_arg_value(&s, a, b)).collect::<Vec<_>>(), vec![None, Some(0x15f5e4)], "b4: only a whole literal counts");
	let s = scan_prepare(&format!("{import}const X: u32 = 0x15f5d0 + 0x40; const Y: u32 = 0x15f5d0; fn f() {{ g(X); g(Y); }}"), "x.rs");
	assert_eq!(s.gmap.iter().map(|&(a, b)| g_arg_value(&s, a, b)).collect::<Vec<_>>(), vec![None, Some(0x15f5d0)], "b4: a const counts only as its whole value");
	let (files, _) = scan_tree();
	let (mut calls, mut bad) = (0, Vec::new());
	for (rel, s) in &files {
		for &(a, b) in &s.gmap {
			calls += 1;
			let v = g_arg_value(s, a, b);
			if !v.is_some_and(|v| (G_LO..=G_HI).contains(&v)) {
				let arg: String = s.code[a..b].iter().collect();
				bad.push(format!("{rel}:{}: g({}) is not a literal or a same-file const in {G_LO:#x}..={G_HI:#x} ({v:x?})", line_of(&s.code, a), arg.trim()));
			}
		}
	}
	assert!(bad.is_empty(), "disc::g used outside its window (use disc::va):\n{}", bad.join("\n"));
	assert!(calls >= 3, "the scan found only {calls} g(..) calls: it checks nothing");
}

/// c. `disc::pick(&A, &B)` takes A's USA twin as B everywhere; the checker sees a seeded wrong pair.
#[test]
fn every_disc_pick_has_its_usa_twin() {
	let (n, bad) = bad_picks("let a = disc::pick(&SEEDED, &OTHER_USA); let b = crate::retail::disc::pick(&X, &X_USA);");
	assert_eq!((n, bad.len()), (2, 1), "seeded wrong twin: {bad:?}");
	let (mut total, mut wrong) = (0, Vec::new());
	for (rel, src) in source_files() {
		let (n, bad) = bad_picks(&src);
		total += n;
		wrong.extend(bad.into_iter().map(|b| format!("{rel}: pick({b})")));
	}
	assert!(wrong.is_empty(), "pick without its _USA twin:\n{}", wrong.join("\n"));
	assert!(total >= 20, "the scan found only {total} picks: it checks nothing");
}

/// The error of a proven triple that does not map to exactly its USA address in a run / gap / ext segment.
fn check_triple(segs: &[Seg], pal: u32, usa: u32) -> Result<(), String> {
	let (m, kind) = map_kind(segs, pal);
	if kind > 2 {
		return Err(format!("{pal:#x} falls in kind {kind} (3 tail, 4 ambig, {K_CORE} core, {K_NONE} none)"));
	}
	if m != usa {
		return Err(format!("{pal:#x} maps to {m:#x}, proven {usa:#x}"));
	}
	Ok(())
}

/// d. The proven list against the real segment tables; the checker on seeded wrong rows.
#[test]
fn proven_addresses_map_to_their_exact_segment() {
	let seg = |lo, hi, d, kind| Seg { lo, hi, d, d_hi: d, kind };
	let good = [seg(0x200000, 0x200100, 0x80, 0)];
	assert_eq!(check_triple(&good, 0x200010, 0x200090), Ok(()));
	assert!(check_triple(&good, 0x200010, 0x200094).is_err(), "usa + 4 must be rejected");
	assert!(check_triple(&good, 0x200110, 0x200190).is_err(), "a hole address must be rejected");
	assert!(check_triple(&[seg(0x200000, 0x200100, 0x80, 3)], 0x200010, 0x200090).is_err(), "a tail segment is a guess");
	let mut per_scope = [0usize; 21];
	let mut errors = Vec::new();
	for &(sc, pal, usa, _) in PROVEN {
		per_scope[sc as usize] += 1;
		if let Err(e) = check_triple(segs_of(sc), pal, usa) {
			errors.push(format!("scope {sc}: {e}"));
		}
	}
	assert!(errors.is_empty(), "{} proven rows do not map exactly:\n{}", errors.len(), errors.iter().take(30).cloned().collect::<Vec<_>>().join("\n"));
	assert!(PROVEN.len() > 2000 && per_scope.iter().all(|&n| n > 10), "the proven list is too small to guard anything: {per_scope:?}");
}

/// e. A hole is K_NONE and panics under test; the tables have no overlap.
#[test]
fn a_hole_in_the_segments_is_none_and_fails_the_tests() {
	let seg = |lo, hi| Seg { lo, hi, d: 0x80, d_hi: 0x80, kind: 0 };
	let segs = [seg(0x200000, 0x200100), seg(0x200200, 0x200300)];
	let here = std::panic::Location::caller();
	assert_eq!(map_kind(&segs, 0x200150), (0x200150, K_NONE));
	assert_eq!(map_kind(&segs, 0x200250), (0x2002d0, 0));
	assert_eq!(map_kind(&segs, 0x100000), (0x100000, K_CORE));
	assert_eq!(traced(0, &segs, 0x200050, here), 0x2000d0, "a covered address does not panic");
	let hole = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| traced(0, &segs, 0x200150, here)));
	assert!(hole.is_err(), "an address in a hole must panic under test");
	let tables: Vec<(String, &[Seg])> = std::iter::once(("BOOTOV".to_string(), BOOTOV)).chain(std::iter::once(("FRONT".to_string(), FRONT))).chain(LEVELS.iter().enumerate().map(|(i, l)| (format!("L{i}"), *l))).collect();
	for (name, t) in tables {
		for w in t.windows(2) {
			assert!(w[0].lo < w[0].hi && w[0].hi <= w[1].lo, "{name}: segments {:#x}..{:#x} and {:#x}..{:#x} overlap or are unsorted", w[0].lo, w[0].hi, w[1].lo, w[1].hi);
		}
		assert!(t.iter().all(|s| s.kind <= 4), "{name}: unknown segment kind");
	}
}

// ---- f: the discs ----------------------------------------------------------------------------------------------------

const PAL_ISO: &str = "Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso";
/// The USA disc lies next to the PAL one (the project root) or in a folder of its own name.
const USA_ISOS: [&str; 2] = ["Ratchet & Clank (USA) (En,Fr,De,Es,It).iso", "Ratchet & Clank (USA) (En,Fr,De,Es,It)/Ratchet & Clank (USA) (En,Fr,De,Es,It).iso"];

fn find_iso(rels: &[&str]) -> Option<PathBuf> {
	Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().flat_map(|a| rels.iter().map(move |r| a.join(r))).find(|p| p.is_file())
}

/// Equal under relocation: code compares opcode and registers (J targets and every immediate are relocated), data compares
/// words and treats two pointer-like words (into the overlay range) as equal.
fn window_equal(code: bool, pal: &[u32], usa: &[u32]) -> bool {
	let ptr = |w: u32| (0x15_ef00..0x40_0000).contains(&(w & 0x01ff_ffff)) && w >> 25 == 0;
	pal.len() == usa.len()
		&& pal.iter().zip(usa).all(|(&p, &u)| {
			if code {
				match p >> 26 {
					0 => p == u,
					2 | 3 => p >> 26 == u >> 26,
					_ => p >> 16 == u >> 16,
				}
			} else {
				p == u || (ptr(p) && ptr(u))
			}
		})
}

fn words(b: &[u8]) -> Vec<u32> {
	b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
}

/// The overlay images (`sections[0]`) of the 19 levels of the ISO at `path`.
fn overlays(path: &Path) -> Vec<Option<Vec<u8>>> {
	let mut disc = crate::iso::Disc::open(path).expect("open the ISO");
	let levels = crate::toc::read_levels(&mut disc).expect("read the level table");
	(0..19).map(|n| crate::level::Level::load(&mut disc, &levels[n]).ok().and_then(|l| l.sections.first().cloned().flatten())).collect()
}

/// The words at `addr` of the overlay image: 12 words, else fewer until the window fits one record.
fn window(img: &[u8], addr: u32) -> Option<Vec<u32>> {
	[12usize, 8, 4, 2, 1].iter().find_map(|&n| super::disc::ov_bytes(img, addr as usize, 4 * n).map(words))
}

/// f1. `window_equal` on seeded windows.
#[test]
fn window_equal_ignores_relocations_only() {
	let lui = |imm: u32| 0x3c04_0000 | imm; // lui a0, imm
	assert!(window_equal(true, &[lui(0x20), 0x2484_89e0], &[lui(0x21), 0x2484_8a60]), "lui and addiu immediates are relocated");
	assert!(window_equal(true, &[0x0c07_8000], &[0x0c07_9000]), "jal targets are relocated");
	assert!(!window_equal(true, &[lui(0x20), 0x2484_89e0], &[lui(0x20), 0x2c84_89e0]), "a changed opcode is a different function");
	assert!(!window_equal(true, &[0x0080_2021], &[0x0080_2025]), "an R-type word compares whole");
	assert!(window_equal(false, &[0x1f3b00, 7], &[0x1f3c00, 7]), "pointers into the overlay are relocated");
	assert!(!window_equal(false, &[0x3f80_0000, 7], &[0x3f80_0001, 7]), "a changed float is a different table");
	assert!(!window_equal(false, &[1, 2, 3], &[1, 2]), "different lengths");
}

/// f2. With both ISOs on disk: every code / data row of the proven list has equal words at its PAL and its USA address.
#[test]
fn both_discs_confirm_the_proven_list() {
	let (Some(pal), Some(usa)) = (find_iso(&[PAL_ISO]), find_iso(&USA_ISOS)) else {
		eprintln!("both_discs_confirm_the_proven_list: SKIPPED: ISO missing (the PAL and the USA ISO are not both on the path of the project folder); compared 0 rows");
		assert!(std::env::var_os("RAC_REQUIRE_ISOS").is_none(), "RAC_REQUIRE_ISOS is set but an ISO is missing");
		return;
	};
	let (po, uo) = (overlays(&pal), overlays(&usa));
	let (mut compared, mut errors) = (0, Vec::new());
	for &(sc, p, u, class) in PROVEN.iter().filter(|r| r.0 < 19 && matches!(r.3, b'C' | b'D')) {
		let (Some(Some(pi)), Some(Some(ui))) = (po.get(sc as usize), uo.get(sc as usize)) else { continue };
		let (Some(pw), Some(uw)) = (window(pi, p), window(ui, u)) else { continue };
		let n = pw.len().min(uw.len());
		compared += 1;
		if !window_equal(class == b'C', &pw[..n], &uw[..n]) {
			errors.push(format!("scope {sc} {p:#x} -> {u:#x} ({}): PAL {:x?} USA {:x?}", class as char, &pw[..n.min(4)], &uw[..n.min(4)]));
		}
	}
	let total = PROVEN.iter().filter(|r| r.0 < 19 && matches!(r.3, b'C' | b'D')).count();
	eprintln!("both_discs_confirm_the_proven_list: compared {compared} of {total} C / D rows of the proven list on the two discs, {} differ", errors.len());
	assert!(errors.is_empty(), "{} of {compared} proven rows differ on the discs:\n{}", errors.len(), errors.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
	assert!(compared > 800, "only {compared} rows could be compared: the check is hollow");
}
