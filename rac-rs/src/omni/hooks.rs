//! Hook points (stage 2b): the host entries `supports_hook`, `install_hook`, `uninstall_hook`, `call_original`.
//!
//! A hook point is a RELOCATABLE hand (oracle `hs_reloc`): the retail function the mods hook sits at a different address in every level
//! (and on the USA disc), so it is found by its code: a spec (a masked FNV hash of the code words, no retail bytes) per hook id, from
//! `omni/rac1_reloc.txt` next to the exe (a launcher update can add hook points) over the copy built into the exe (`rac1_reloc.txt`).
//! `Table::build` installs `bridge::hook_n::<N>` at the entry of every recompiled function that matches, in every level it builds.
//!
//! `install_hook` runs inside `omni_load`, before the recompiled table is built (`omni::init` is called before `begin_rec`), so
//! registering the hands there is enough. A hook id without a spec is "not supported on this port" (said once in the run log).
//! The hands only exist for hook ids a loaded mod takes: with no mods nothing is registered.

use super::binding::{Sig, Ty};
use super::bridge::{self, hook_n};
use super::cabi::{Host, Value, OMNI_VALUE_NO_RESULT};
use super::host::{self, state, text};
use ee_oracle::cpu::Cpu;
use ee_oracle::hs_reloc::{Reloc, RelocHand};
use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Hook slots = the binding's hook indices (the built-in binding has 210; the table below must have this many entries, and a launcher binding with more hooks than this cannot install the rest).
pub const MAX_HOOKS: usize = 256;

/// The specs built into the exe: `hook <id>` then the `reloc`/`mhash`/`mask`/`jabs` lines of the spec.
const BUILTIN_SPECS: &str = include_str!("rac1_reloc.txt");

/// One hook point.
pub struct Slot {
	pub id: OnceLock<String>,
	pub reloc: OnceLock<Reloc>,
	pub sig: OnceLock<Sig>,
	/// False after `uninstall_hook`: the hand falls through to retail.
	pub active: AtomicBool,
	/// Re-entry guard of the hand (bridge.rs).
	pub guard: AtomicBool,
	/// Dispatches so far.
	pub calls: AtomicU64,
}

impl Slot {
	const fn new() -> Slot {
		Slot { id: OnceLock::new(), reloc: OnceLock::new(), sig: OnceLock::new(), active: AtomicBool::new(false), guard: AtomicBool::new(false), calls: AtomicU64::new(0) }
	}
}

static SLOTS: [Slot; MAX_HOOKS] = [const { Slot::new() }; MAX_HOOKS];

pub fn slot(n: usize) -> &'static Slot {
	&SLOTS[n]
}

fn reloc_n<const N: usize>() -> &'static Reloc {
	SLOTS[N].reloc.get().expect("hook slot registered without a spec")
}

macro_rules! tables {
	($($n:literal)*) => {
		static RELOC_FNS: [fn() -> &'static Reloc; MAX_HOOKS] = [$(reloc_n::<$n>),*];
		static HOOK_FNS: [fn(&mut Cpu) -> bool; MAX_HOOKS] = [$(hook_n::<$n>),*];
	};
}
tables!(
	0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31
	32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63
	64 65 66 67 68 69 70 71 72 73 74 75 76 77 78 79 80 81 82 83 84 85 86 87 88 89 90 91 92 93 94 95
	96 97 98 99 100 101 102 103 104 105 106 107 108 109 110 111 112 113 114 115 116 117 118 119 120 121 122 123 124 125 126 127
	128 129 130 131 132 133 134 135 136 137 138 139 140 141 142 143 144 145 146 147 148 149 150 151 152 153 154 155 156 157 158 159
	160 161 162 163 164 165 166 167 168 169 170 171 172 173 174 175 176 177 178 179 180 181 182 183 184 185 186 187 188 189 190 191
	192 193 194 195 196 197 198 199 200 201 202 203 204 205 206 207 208 209 210 211 212 213 214 215 216 217 218 219 220 221 222 223
	224 225 226 227 228 229 230 231 232 233 234 235 236 237 238 239 240 241 242 243 244 245 246 247 248 249 250 251 252 253 254 255
);

/// The spec file text: `hook <id>` lines, each followed by that hook's spec lines; other lines (`#`, empty) are ignored.
pub fn parse_specs(text: &str) -> HashMap<String, Reloc> {
	let mut out = HashMap::new();
	let mut id: Option<String> = None;
	let mut body = String::new();
	let mut flush = |id: &mut Option<String>, body: &mut String| {
		if let Some(i) = id.take() {
			match Reloc::parse(body) {
				Some(r) => {
					out.insert(i, r);
				}
				None => eprintln!("omni: the spec of hook {i} does not parse, hook not supported"),
			}
		}
		body.clear();
	};
	for l in text.lines() {
		if let Some(rest) = l.strip_prefix("hook ") {
			flush(&mut id, &mut body);
			id = Some(rest.trim().to_string());
		} else if id.is_some() && !l.trim_start().starts_with('#') {
			body.push_str(l);
			body.push('\n');
		}
	}
	flush(&mut id, &mut body);
	out
}

/// Built-in specs, then `<exe dir>/omni/rac1_reloc.txt` over them (its entries win).
fn specs() -> &'static HashMap<String, Reloc> {
	static SPECS: OnceLock<HashMap<String, Reloc>> = OnceLock::new();
	SPECS.get_or_init(|| {
		let mut m = parse_specs(BUILTIN_SPECS);
		if let Some(t) = super::load::exe_dir().map(|d| d.join("omni").join("rac1_reloc.txt")).and_then(|p| std::fs::read_to_string(p).ok()) {
			let ext = parse_specs(&t);
			eprintln!("omni: {} hook spec(s) read from omni/rac1_reloc.txt", ext.len());
			m.extend(ext);
		}
		m
	})
}

/// The bridge reads integers from r4..r11 and floats from f12..f19; no f64 (the EE has no double registers).
pub fn sig_supported(sig: &Sig) -> bool {
	let floats = sig.args.iter().filter(|t| t.is_float()).count();
	sig.args.iter().chain(std::iter::once(&sig.ret)).all(|t| *t != Ty::F64) && floats <= 8 && sig.args.len() - floats <= 8
}

/// A line at most once per (hook, text); keeps the run log short when a hook fails on every call.
pub fn warn_once(n: usize, msg: &str) {
	static SEEN: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
	let mut s = SEEN.lock().unwrap_or_else(|e| e.into_inner());
	if !s.iter().any(|(i, m)| *i == n && m == msg) {
		s.push((n, msg.to_string()));
		super::logq::line(&format!("omni: hook {}: {msg}", slot(n).id.get().map_or("?", String::as_str)));
	}
}

unsafe extern "C" fn supports_cb(user: *mut c_void, id: *const c_char) -> i32 {
	let id = text(id);
	let ok = specs().contains_key(&id) && state(user).binding.hook(&id).and_then(|h| Sig::parse(&h.signature).ok()).is_some_and(|s| sig_supported(&s));
	if !ok {
		static SAID: Mutex<Vec<String>> = Mutex::new(Vec::new());
		let mut said = SAID.lock().unwrap_or_else(|e| e.into_inner());
		if !said.contains(&id) {
			eprintln!("omni: hook point {id} is not supported on this port (no relocation spec for it)");
			said.push(id);
		}
	}
	i32::from(ok)
}

unsafe extern "C" fn install_cb(user: *mut c_void, index: u32, id: *const c_char) -> i32 {
	let id = text(id);
	let n = index as usize;
	if n >= MAX_HOOKS {
		eprintln!("omni: hook point {id} has index {n}, the host has {MAX_HOOKS} hook slots: not installed");
		return -3;
	}
	let st = state(user);
	let (Some(def), Some(reloc)) = (st.binding.hooks.get(n).filter(|h| h.id == id), specs().get(&id)) else { return -1 };
	let Ok(sig) = Sig::parse(&def.signature) else { return -2 };
	if !sig_supported(&sig) {
		return -3;
	}
	let s = slot(n);
	if s.reloc.set(reloc.clone()).is_err() {
		// Installed before (a second omni_load): switch it on again.
		s.active.store(true, Ordering::SeqCst);
		return 0;
	}
	let _ = (s.id.set(id.clone()), s.sig.set(sig));
	s.active.store(true, Ordering::SeqCst);
	let hand: &'static RelocHand = Box::leak(Box::new(RelocHand { name: id.clone().leak(), reloc: RELOC_FNS[n], est: ee_oracle::hs::EXACT, f: HOOK_FNS[n] }));
	ee_oracle::rec::register_reloc_hand(hand);
	allow_reloc_hands();
	eprintln!("omni: hook point {id} installed (relocatable hand, {} bytes of code matched by hash)", reloc.len);
	0
}

/// `Table::build` installs REGISTERED relocatable hands (mod hooks, PC hooks) regardless of RAC_HS (only the built-in speed hands
/// follow it), so RAC_HS=0 no longer needs overriding; ORACLE_HAND=0 still switches every hand off.
pub(super) fn allow_reloc_hands() {
	if std::env::var("ORACLE_HAND").is_ok_and(|v| v == "0") {
		eprintln!("omni: ORACLE_HAND=0 switches every hand off, mod hooks included; the hooks will not run");
	}
}

unsafe extern "C" fn uninstall_cb(_user: *mut c_void, index: u32, _id: *const c_char) {
	if let Some(s) = SLOTS.get(index as usize) {
		s.active.store(false, Ordering::SeqCst);
	}
}

/// Every hook point back to the game's own code: `omni_load` failed after it had installed some (load.rs).
pub fn deactivate_all() {
	for s in &SLOTS {
		s.active.store(false, Ordering::SeqCst);
	}
}

unsafe extern "C" fn original_cb(_user: *mut c_void, index: u32, args: *const Value, nargs: u32, ret: *mut Value) {
	let n = index as usize;
	// SAFETY: the loader passes `nargs` values.
	let given: &[Value] = if args.is_null() { &[] } else { unsafe { std::slice::from_raw_parts(args, nargs as usize) } };
	// A panic of the retail code must not leave this `extern "C"` fn (that aborts the process): on the game thread it is parked and unwinds
	// it when the dispatch it happened in is over; between two fields (worker thread) it is swallowed (bridge.rs `park_panic`).
	let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match (host::cpu(), SLOTS.get(n).and_then(|s| s.sig.get())) {
		// Inside the dispatch of hook `n` (the innermost one): the retail function with the dispatch's entry. Any other time (between two
		// fields, or inside the dispatch of ANOTHER hook): found by its spec (original.rs).
		(Some(c), Some(sig)) if bridge::dispatching(n) => bridge::call_retail(c, n, sig, given),
		(Some(c), Some(sig)) => Some(super::original::between_value(c, n, sig, given)),
		_ => None,
	}));
	let result = ran.unwrap_or_else(|p| {
		if let Some(c) = host::cpu() {
			bridge::park_panic(c, p);
		}
		None
	});
	if !ret.is_null() {
		// SAFETY: `ret` is one OmniValue the loader gave.
		unsafe { *ret = ret_value(result) };
	}
}

/// What `call_original` hands back (ABI 1.5): the value, or, when the original produced none (the call did not complete, e.g. it
/// waits for a frame and was let go), a zero value with `reserved` bit 0 (`OMNI_VALUE_NO_RESULT`) set so the loader can tell
/// "no result" from a real 0. An ABI 1.4 loader never reads the bit.
pub(super) fn ret_value(result: Option<Value>) -> Value {
	match result {
		Some(v) => v,
		None => Value { reserved: OMNI_VALUE_NO_RESULT, ..Value::default() },
	}
}

/// Fills the hook entries of the host (supports_hook, install_hook, uninstall_hook, call_original).
pub fn fill(h: &mut Host) {
	h.supports_hook = Some(supports_cb);
	h.install_hook = Some(install_cb);
	h.uninstall_hook = Some(uninstall_cb);
	h.call_original = Some(original_cb);
}

/// After `omni_load`: a summary line of the hook points mods took.
pub fn installed(o: &super::Omni) {
	let ids: Vec<String> = o.loader.hooked().iter().map(|&i| o.loader.hook_id(i)).collect();
	if !ids.is_empty() {
		eprintln!("omni: {} hook point(s) taken by mods: {}", ids.len(), ids.join(", "));
	}
}

/// `id=calls` for every hook point that was installed, for the periodic run-log line.
pub fn counts_line() -> Option<String> {
	let v: Vec<String> = SLOTS.iter().filter_map(|s| Some(format!("{}={}", s.id.get()?, s.calls.load(Ordering::Relaxed)))).collect();
	(!v.is_empty()).then(|| v.join(" "))
}
