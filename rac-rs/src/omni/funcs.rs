//! Game functions by number, for native mods (`call_game`): `function("FUN_001f9dc8")` hands the host the hex digits of the name, which are the
//! number of the function on the disc the mod was told it runs on (`disc` of the host: "ntsc" numbers on this PAL v1.00 disc, see layout.rs).
//! The function sits at a different address in every level's overlay, so it is found by its code, like a hook point (hooks.rs): a spec is a
//! masked hash of the code words (no retail bytes), from `omni/rac1_funcs.txt` next to the exe over the copy built into the exe.
//!
//! File format, one block per function: `fn 0x<ntsc number> 0x<pal number> <name>`, then its `reloc` / `mhash` / `mask` / `jabs` lines (the output of
//! `tools/make_spec.py`). The scan looks for a start of code after a `jr ra` (plus the delay slot and a little padding) in 0x100000..0x400000
//! whose masked hash equals the spec; the answer is cached per number and checked again at every use (a level change moves the overlay).

use ee_oracle::hs_reloc::Reloc;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The specs built into the exe.
const BUILTIN: &str = include_str!("rac1_funcs.txt");

/// Where retail code can live (the executable and the overlay).
pub const CODE_LO: u32 = 0x10_0000;
pub const CODE_HI: u32 = 0x40_0000;

/// One function the host can find.
pub struct FuncSpec {
	pub ntsc: u32,
	pub pal: u32,
	pub name: String,
	pub reloc: Reloc,
}

fn hex(s: &str) -> Option<u32> {
	u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()
}

/// The specs in a file text; a block that does not parse is said once and left out.
pub fn parse_funcs(text: &str) -> Vec<FuncSpec> {
	let mut out = Vec::new();
	let mut head: Option<(u32, u32, String)> = None;
	let mut body = String::new();
	let mut flush = |head: &mut Option<(u32, u32, String)>, body: &mut String| {
		if let Some((ntsc, pal, name)) = head.take() {
			match Reloc::parse(body) {
				Some(reloc) => out.push(FuncSpec { ntsc, pal, name, reloc }),
				None => eprintln!("omni: the spec of game function {name} does not parse, it is not available to native mods"),
			}
		}
		body.clear();
	};
	for l in text.lines() {
		let w: Vec<&str> = l.split_whitespace().collect();
		if w.first() == Some(&"fn") && w.len() >= 4 {
			flush(&mut head, &mut body);
			head = match (hex(w[1]), hex(w[2])) {
				(Some(n), Some(p)) => Some((n, p, w[3].to_string())),
				_ => None,
			};
		} else if head.is_some() && !l.trim_start().starts_with('#') {
			body.push_str(l);
			body.push('\n');
		}
	}
	flush(&mut head, &mut body);
	out
}

/// Built-in specs, then `<exe dir>/omni/rac1_funcs.txt` (a launcher update can add functions; its entries win per number).
fn specs() -> &'static Vec<FuncSpec> {
	static SPECS: OnceLock<Vec<FuncSpec>> = OnceLock::new();
	SPECS.get_or_init(|| {
		let mut v = parse_funcs(BUILTIN);
		if let Some(t) = super::load::exe_dir().map(|d| d.join("omni").join("rac1_funcs.txt")).and_then(|p| std::fs::read_to_string(p).ok()) {
			let ext = parse_funcs(&t);
			eprintln!("omni: {} game function spec(s) read from omni/rac1_funcs.txt", ext.len());
			v.retain(|b| !ext.iter().any(|e| e.ntsc == b.ntsc && e.pal == b.pal));
			v.extend(ext);
		}
		v
	})
}

/// Every spec the host knows (the prescan resolves them all when a level loads).
pub(super) fn all() -> &'static [FuncSpec] {
	specs()
}

/// Records the answer of a level-load prescan for `spec` (both of its numbers): found, or not in the code at `level_word`.
pub(super) fn seed(spec: &FuncSpec, entry: Option<u32>, level_word: i32) {
	let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
	let cache = g.get_or_insert_with(|| Cache { found: HashMap::new(), failed: HashMap::new() });
	for n in [spec.ntsc, spec.pal] {
		match entry {
			Some(e) => {
				cache.found.insert(n, e);
				cache.failed.remove(&n);
			}
			None => {
				cache.found.remove(&n);
				cache.failed.insert(n, level_word);
			}
		}
	}
}

/// The spec of function `number` on the disc `ntsc` says (true: the NTSC numbers, false: the PAL v2.00 ones).
pub fn find_spec(number: u32, ntsc: bool) -> Option<&'static FuncSpec> {
	specs().iter().find(|s| number == if ntsc { s.ntsc } else { s.pal })
}

const JR_RA: u32 = 0x03e0_0008;

/// The addresses in `words` (the code words from `base`) where a function may start: after a `jr ra` and its delay slot, with up to three
/// padding words between (`strict`), else every word.
pub fn candidates(words: &[u32], base: u32, strict: bool) -> Vec<u32> {
	(0..words.len())
		.filter(|&i| !strict || (2..=5).any(|k| i >= k && words[i - k] == JR_RA))
		.map(|i| base + 4 * i as u32)
		.collect()
}

/// The first address in `words` (code from `base`) whose masked hash is the spec's, or None.
pub fn find_in(words: &[u32], base: u32, r: &Reloc) -> Option<u32> {
	let end = base + 4 * words.len() as u32;
	let at = |a: u32| words.get(((a - base) / 4) as usize).copied().unwrap_or(0);
	for strict in [true, false] {
		for a in candidates(words, base, strict) {
			if a + r.len <= end && r.masked_hash_by(a, at) == r.mhash {
				return Some(a);
			}
		}
	}
	None
}

pub(super) fn code_words(c: &ee_oracle::cpu::Cpu) -> Vec<u32> {
	c.mem.ram.get(CODE_LO as usize..CODE_HI as usize).map_or_else(Vec::new, |b| b.chunks_exact(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect())
}

fn still_there(c: &ee_oracle::cpu::Cpu, r: &Reloc, entry: u32) -> bool {
	r.matches(&c.mem, entry)
}

struct Cache {
	found: HashMap<u32, u32>,
	/// number -> the level word it failed at (no new scan until the level changes).
	failed: HashMap<u32, i32>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// The runtime entry of function `number` in the code that is loaded now.
pub fn resolve(c: &ee_oracle::cpu::Cpu, number: u32, ntsc: bool, level_word: i32) -> Result<u32, String> {
	let spec = find_spec(number, ntsc).ok_or_else(|| format!("game function number 0x{number:x} is not known to this port"))?;
	let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
	let cache = g.get_or_insert_with(|| Cache { found: HashMap::new(), failed: HashMap::new() });
	if let Some(&e) = cache.found.get(&number) {
		if still_there(c, &spec.reloc, e) {
			return Ok(e);
		}
		cache.found.remove(&number);
	}
	if cache.failed.get(&number) == Some(&level_word) {
		return Err(format!("game function {} (0x{number:x}) is not in the code loaded now", spec.name));
	}
	match find_in(&code_words(c), CODE_LO, &spec.reloc) {
		Some(e) => {
			cache.found.insert(number, e);
			Ok(e)
		}
		None => {
			cache.failed.insert(number, level_word);
			Err(format!("game function {} (0x{number:x}) is not in the code loaded now", spec.name))
		}
	}
}
