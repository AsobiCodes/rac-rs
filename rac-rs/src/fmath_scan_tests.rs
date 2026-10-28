//! Keeps std's C-runtime float maths out of the code that writes game memory (`fmath.rs` says why): no `.sin(`, `f32::atan2`
//! and the like in src/retail/**, src/retail.rs and src/omni/**; those go through `crate::fmath::Fm` (`.fsin(`, `.fatan2(` ...).
//! The source is read with comments, strings and char literals blanked (`disc_guard_tests::blank_text`), so text never counts and
//! never hides a call. `sqrt`, `abs`, `floor`, `powi`, `rem_euclid`, `to_radians` and `to_degrees` give the same bits on every
//! target (exact IEEE operations, or the same integer-power loop for `powi`) and are allowed.
use crate::retail::disc_guard_tests::{blank_text, is_id};
use std::path::Path;

const NAMES: [&str; 26] = [
	"sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sin_cos", "sinh", "cosh", "tanh", "asinh", "acosh", "atanh", "exp",
	"exp2", "exp_m1", "ln", "ln_1p", "log", "log2", "log10", "powf", "hypot", "cbrt", "mul_add",
];

/// The fewest .rs files the scan must read (178 on 2026-10-28): a moved or renamed folder fails instead of passing empty.
const MIN_FILES: usize = 150;

/// The std calls in a source text, as (line, form): method form `.sin(` (spaces allowed before the bracket) and path form
/// `f32::sin` / `f64::sin` (called or passed as a value, e.g. `.map(f32::sin)`).
fn std_calls(src: &str) -> Vec<(usize, String)> {
	let c = blank_text(src);
	let ident_at = |i: usize| c[i..].iter().take_while(|&&x| is_id(x)).collect::<String>();
	let mut hits = Vec::new();
	let mut line = 1;
	for i in 0..c.len() {
		if c[i] == '\n' {
			line += 1;
			continue;
		}
		if c[i] == '.' {
			let name = ident_at(i + 1);
			let after = c[i + 1 + name.len()..].iter().find(|x| !x.is_whitespace());
			if NAMES.contains(&name.as_str()) && after == Some(&'(') {
				hits.push((line, format!(".{name}(")));
			}
		} else if (i == 0 || !is_id(c[i - 1])) && (c[i..].starts_with(&['f', '3', '2', ':', ':']) || c[i..].starts_with(&['f', '6', '4', ':', ':'])) {
			let name = ident_at(i + 5);
			if NAMES.contains(&name.as_str()) {
				hits.push((line, format!("{}::{name}", c[i..i + 3].iter().collect::<String>())));
			}
		}
	}
	hits
}

/// Every .rs file under `p` (or `p` itself when it is one); a path that cannot be read fails the test.
fn walk(p: &Path, files: &mut usize, out: &mut Vec<String>) {
	if p.is_dir() {
		for e in std::fs::read_dir(p).unwrap_or_else(|e| panic!("{}: {e}", p.display())) {
			walk(&e.expect("directory entry").path(), files, out);
		}
	} else if p.extension().is_some_and(|x| x == "rs") {
		let text = std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
		*files += 1;
		out.extend(std_calls(&text).into_iter().map(|(l, hit)| format!("{}:{l}: {hit}", p.display())));
	} else if !p.exists() {
		panic!("{} is missing (moved? update the scan list)", p.display());
	}
}

#[test]
fn the_game_memory_code_uses_the_same_maths_on_every_os() {
	let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
	let (mut files, mut found) = (0, Vec::new());
	for p in ["retail", "retail.rs", "omni"] {
		walk(&src.join(p), &mut files, &mut found);
	}
	assert!(files >= MIN_FILES, "only {files} files scanned");
	assert!(found.is_empty(), "std float maths in code that writes game memory (use crate::fmath::Fm):\n{}", found.join("\n"));
}

#[test]
fn the_scan_sees_every_call_form_and_no_text() {
	let forms = |s: &str| std_calls(s).into_iter().map(|(_, h)| h).collect::<Vec<_>>();
	assert_eq!(forms("let a = y.atan2(x);"), [".atan2("]);
	assert_eq!(forms("let s = f32::sin(t);"), ["f32::sin"]);
	assert_eq!(forms("let v: Vec<f64> = a.iter().copied().map(f64::exp).collect();"), ["f64::exp"]);
	assert_eq!(forms("let c = t.cos ();"), [".cos("]);
	assert_eq!(forms("let p = \"http://x\"; let s = t.sin();"), [".sin("], "a // inside a string hides nothing");
	assert_eq!(forms("let a = 1;\nlet b = x.hypot(y);"), [".hypot("]);
	assert_eq!(std_calls("let a = 1;\nlet b = x.hypot(y);")[0].0, 2);
	assert!(forms("let a = y.fatan2(x).fsin(); // was y.atan2(x)").is_empty());
	assert!(forms("/* x.sin() */ let t = \"a.cos(b)\"; let c = 'x';").is_empty());
	assert!(forms("let r = x.sqrt().powi(2).to_radians().floor();").is_empty());
	assert!(forms("let e = self.exp; let l = log.len(); let x = my_f32::sin(1);").is_empty());
}
