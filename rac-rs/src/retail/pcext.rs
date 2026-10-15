//! PC options in the game's own pause menu (Pause -> Options -> Camera Options, retail page id 0x1d): two extra rows in the
//! retail look, remembered between sessions (`pcopts.rs`, `<player dir>/pc_options.txt`).
//!
//! - `Camera: Classic / Modern`: Modern = the mouse-driven modern camera (modcam.rs), Classic = retail's camera and controls.
//!   A new player starts with Modern.
//! - `Strafe aiming: On / Off`: the right mouse button's strafe mode (modcam's aim mode, strafe.rs). It only matters while the
//!   camera is Modern; with Classic strafe aiming is off whatever the row says. A new player starts with On.
//!
//! How (design: scratch_race/OPTIONS_DESIGN.md): the retail multi-choice item (update 0x27e898 / draw 0x27ea40 on every level)
//! reads its rows from a table of 6-word records `{label id, byte*, choice0..3}` ended by a zero word, so a row is data:
//! - `patch_rows` (worker, each field): on the running level, when the retail table sits where the signature says, a copy
//!   with five records (the three retail ones + Camera + Strafe) is built in reserved RAM and the item's table pointer (+0x34) is
//!   repointed to it. Overlay data is initialised by every level load, so the poll patches again after a reload.
//! - The rows' values are bytes in reserved RAM (`BASE`), outside the save struct, so the memory card stays 1:1. The
//!   retail update toggles them itself; `pre_field` reads them back every field, applies them (modcam's camera, the strafe gate)
//!   and writes the file when one changed. F7 flips the camera byte.
//! - Three texts are new (ids 0x7f01..0x7f03: Classic, Modern, the strafe label). A wrapping hand on the retail text lookup
//!   (`FUN_00203620` twin, 16 instructions, per level) answers them from a pool in reserved RAM in the game's current language
//!   and declines every other id, so retail's own lookup runs for those. Labels and On/Off reuse retail's text ids
//!   (0x4f23 Camera, 0x4f5a On, 0x4f5b Off), so only three strings have wording of their own.
//! - Language: Options -> Language (title) already exists; the host watches the game's language word and remembers it
//!   (`watch_language`), and a cold boot starts in that language (`init`). The retail card is untouched.
//!
//! `RAC_PC_EXT=0` registers nothing and writes nothing (today's bit-exact path); the oracle's gates never link this file.
use super::pcopts::{self, Lang, PcOptions};
use super::{modcam, FieldIn};
use ee_oracle::cpu::Cpu;
use ee_oracle::mem::Mem;
use ee_oracle::rec::{register_hand, Hand};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Reserved RAM: all zero in every savestate scanned, below the ELF image (0x100000) and outside the rec/hand range.
pub const BASE: u32 = 0xB0000;
/// "PCX1": the region has been set up (fresh RAM after a start, a savestate load or a cold boot reads 0 here).
const MAGIC: u32 = 0x3158_4350;
/// Camera style: 0 Classic, 1 Modern (the byte the retail row toggles).
pub const CAM: u32 = BASE + 4;
/// Strafe aiming: 0 Off, 1 On.
pub const STRAFE: u32 = BASE + 5;
/// Buy all ammo at a vendor: 0 Off, 1 On.
pub const BUYALL: u32 = BASE + 6;
/// The copy of the row table (5 records + the zero record).
const TAB: u32 = BASE + 0x40;
/// String pool: one 0x40-byte slot per new text.
const POOL: u32 = BASE + 0x100;
/// The game's language word.
const LANG_WORD: u32 = 0x15ed88;

const ID_CAMERA: u32 = 0x4f23;
const ID_ON: u32 = 0x4f5a;
const ID_OFF: u32 = 0x4f5b;
pub const ID_CLASSIC: u32 = 0x7f01;
pub const ID_MODERN: u32 = 0x7f02;
pub const ID_STRAFE: u32 = 0x7f03;
pub const ID_BUYALL: u32 = 0x7f04;
/// The Buy plate text with the new line: retail's "[Cross] Buy", a newline, "[Square] label" (vendor.rs draws it on the plate).
pub const ID_BUYALL_PROMPT: u32 = 0x7f05;
const NEW_IDS: [u32; 5] = [ID_CLASSIC, ID_MODERN, ID_STRAFE, ID_BUYALL, ID_BUYALL_PROMPT];

/// New texts per language (En, Fr, De, Es, It) in the game's font bytes: a-acute 0x84, e-acute 0x8c, a-umlaut 0x87.
/// Short like the retail labels (the German ones are abbreviated to fit). [I] wording, native review wanted.
static TEXTS: [[&[u8]; 5]; 5] = [
	[b"Classic", b"Classique", b"Klassisch", b"Cl\x84sica", b"Classica"],
	[b"Modern", b"Moderne", b"Modern", b"Moderna", b"Moderna"],
	[b"Strafe aiming", b"Vis\x8ce lat\x8crale", b"Strafe-Zielen", b"Apuntado lateral", b"Mira laterale"],
	[b"Refill all ammo", b"Tout remplir", b"Alles f\xa7llen", b"Llenar todo", b"Ricarica tutto"],
	// The Buy plate: retail's own "[Cross] Buy" (id 0x5234), a newline (byte 1), the Square glyph (0x13) and a one-word label that fits
	// the plate on one line (vendor.rs test `plate_text_is_two_lines`; the plate holds about 7 letters after a glyph).
	[b"\x10 Buy\x01\x13 Refill", b"\x10 Acheter\x01\x13 Remplir", b"\x10 Kaufen\x01\x13 F\xa7llen", b"\x10 Comprar\x01\x13 Llenar", b"\x10 Compra\x01\x13 Ricarica"],
];

/// The retail Camera Options row table per level (index = level byte 0x15ed84): `{0x4fa2, 0x15ede0, 0x4fa5, 0x4fa4, 0, 0}` is its
/// first record; the item (+0x34 = table pointer) follows at +0x60, so the pointer to patch is at +0x94. Computed from
/// extracted/levels/N_*/overlay.elf (test `tables_match_the_overlays`).
pub static TABLES: [u32; 19] = [
	0x1b5b40, 0x1b5f40, 0x1b60c0, 0x1b5bc0, 0x1b5c40, 0x1b5fc0, 0x1b6240, 0x1b5b40, 0x1b62c0, 0x1b5c40, 0x1b5f40, 0x1b64c0, 0x1b5f40, 0x1b5dc0, 0x1b6240, 0x1b60c0, 0x1b5f40, 0x1b63c0, 0x1b64c0,
];
const PTR_OFF: u32 = 0x94;
const SIG: [u32; 2] = [0x4fa2, 0x15ede0];

/// The text lookup (`FUN_00203620` twin) of every level: entry, end = entry + 0x40, FNV hash of the code (jal targets differ per
/// level). est 13 = the retail function's own length.
static HANDS: [Hand; 21] = [
	Hand { entry: 0x203620, end: 0x203660, hash: 0x3629dc5b6b49d307, est: 13, f: text_hook }, // 0 veldin_tutorial
	Hand { entry: 0x2254b0, end: 0x2254f0, hash: 0x7e7b1f7e1623414b, est: 13, f: text_hook }, // 1 novalis
	Hand { entry: 0x214608, end: 0x214648, hash: 0x85b7ba437d360729, est: 13, f: text_hook }, // 2 aridia
	Hand { entry: 0x1fcf38, end: 0x1fcf78, hash: 0xeb24b940634c9005, est: 13, f: text_hook }, // 3 kerwan
	Hand { entry: 0x1f6b80, end: 0x1f6bc0, hash: 0x6da79bf77928f66f, est: 13, f: text_hook }, // 4 eudora
	Hand { entry: 0x232308, end: 0x232348, hash: 0x3f708f5518982169, est: 13, f: text_hook }, // 5 rilgar
	Hand { entry: 0x21b640, end: 0x21b680, hash: 0xa3eb0366abb23fbf, est: 13, f: text_hook }, // 6 nebula_g34
	Hand { entry: 0x22b408, end: 0x22b448, hash: 0x15c56ecb291b9929, est: 13, f: text_hook }, // 7 umbris
	Hand { entry: 0x216120, end: 0x216160, hash: 0xda3f93931e3c60c7, est: 13, f: text_hook }, // 8 batalia
	Hand { entry: 0x223280, end: 0x2232c0, hash: 0x61e854250e8f17af, est: 13, f: text_hook }, // 9 gaspar
	Hand { entry: 0x1f6c08, end: 0x1f6c48, hash: 0xbcc287a94ed20b29, est: 13, f: text_hook }, // 10 orxon
	Hand { entry: 0x234f08, end: 0x234f48, hash: 0x77db3b94ebeea8e9, est: 13, f: text_hook }, // 11 pokitaru
	Hand { entry: 0x2264b0, end: 0x2264f0, hash: 0x41a7c7eb53740d4b, est: 13, f: text_hook }, // 12 hoven
	Hand { entry: 0x20f088, end: 0x20f0c8, hash: 0x7a1c9242706d3909, est: 13, f: text_hook }, // 13 oltanis_orbit
	Hand { entry: 0x216c30, end: 0x216c70, hash: 0x8b9c875f10f60eeb, est: 13, f: text_hook }, // 14 oltanis
	Hand { entry: 0x1fcd18, end: 0x1fcd58, hash: 0x9ad2134b19b2bbed, est: 13, f: text_hook }, // 15 quartu
	Hand { entry: 0x201e08, end: 0x201e48, hash: 0x6f83b2093c7fd729, est: 13, f: text_hook }, // 16 kalebo_iii
	Hand { entry: 0x200648, end: 0x200688, hash: 0xe641001c37a5fe99, est: 13, f: text_hook }, // 17 veldin_orbit
	Hand { entry: 0x20c388, end: 0x20c3c8, hash: 0xee17fd3646c79949, est: 13, f: text_hook }, // 18 veldin_finale
	// The same function in the boot ELF and in the front end (frontbin: title, New Game, memory card), for the keyboard prompts.
	Hand { entry: 0x1fd7d8, end: 0x1fd818, hash: 0xa54b5b2195d3c76f, est: 13, f: text_hook }, // boot ELF
	Hand { entry: 0x1fd518, end: 0x1fd558, hash: 0x3c354b39385afa9f, est: 13, f: text_hook }, // frontbin
];

// ---- switch and gates -----------------------------------------------------------------------------------------------

/// The extension is on unless `RAC_PC_EXT=0` (read once). Off: nothing is registered, patched or written.
pub fn enabled() -> bool {
	static E: OnceLock<bool> = OnceLock::new();
	*E.get_or_init(|| !std::env::var("RAC_PC_EXT").is_ok_and(|v| v == "0"))
}

/// The camera row as the worker last applied it (true = Modern). The defaults hold until `init` / the first field.
static CAM_MODERN: AtomicBool = AtomicBool::new(true);
/// The strafe row as last applied (true = On).
static STRAFE_ON: AtomicBool = AtomicBool::new(true);
/// The buy-all-ammo row as last applied (true = On).
static BUYALL_ON: AtomicBool = AtomicBool::new(true);

/// The vendor's buy-all-ammo option (vendor.rs): the menu row; false with the extension off (no row, no text, retail exact).
pub fn buyall_on() -> bool {
	enabled() && BUYALL_ON.load(Ordering::Relaxed)
}

/// The one gate of the right-mouse-button strafe mode (modcam's aim mode and strafe.rs): the menu's verdict. With the extension
/// on it is true only while Camera is Modern and Strafe aiming is On. With `RAC_PC_EXT=0` there is no menu, the old switches
/// (`--modern-cam` / RAC_MODERN_CAM, RAC_MOVE_CLASSIC) decide as before and this answers true.
pub fn strafe_on() -> bool {
	gate(enabled(), CAM_MODERN.load(Ordering::Relaxed), STRAFE_ON.load(Ordering::Relaxed))
}

fn gate(ext: bool, camera_modern: bool, strafe: bool) -> bool {
	!ext || (camera_modern && strafe)
}

/// Registers the text hands (Retail::load, after modcam's, so a hand of the swarm on the same entry is replaced).
pub fn register() {
	if enabled() {
		register_hand(&HANDS);
		register_hand(&super::prompts::GLYPH_HANDS);
	}
}

// ---- state ----------------------------------------------------------------------------------------------------------

struct St {
	opts: PcOptions,
	path: Option<PathBuf>,
	/// The row bytes last seen in RAM (None: the region was not set up yet).
	last: Option<(u8, u8, u8)>,
	/// The language word last seen by the watcher.
	lang_seen: Option<u32>,
	/// A failed write was reported.
	warned: bool,
}

static ST: Mutex<St> = Mutex::new(St { opts: PcOptions::new(), path: None, last: None, lang_seen: None, warned: false });

impl St {
	fn save(&mut self) {
		let Some(path) = &self.path else { return };
		if let Err(e) = self.opts.store(path) {
			if !self.warned {
				self.warned = true;
				eprintln!("retail: PC options: cannot write {}: {e}", path.display());
			}
		}
	}
}

/// Retail::load, before the machine exists: reads the options file, applies the camera style to modcam and, on a cold boot,
/// starts the console in the remembered language (the dev override COLD_LANG wins).
pub fn init(cold: bool) {
	if !enabled() {
		return;
	}
	let path = pcopts::file();
	let opts = PcOptions::load(&path);
	eprintln!(
		"retail: PC options {}: camera {}, strafe aiming {}, language {}",
		path.display(),
		if opts.camera_modern() { "Modern" } else { "Classic" },
		if opts.strafe_on() { "On" } else { "Off" },
		opts.lang.map_or("(not chosen)", |l| l.code())
	);
	if cold && std::env::var_os("COLD_LANG").is_none() {
		if let Some(l) = opts.lang {
			ee_oracle::cold::set_language(l.osd());
			eprintln!("retail: PC options: cold boot in the remembered language ({})", l.code());
		}
	}
	CAM_MODERN.store(opts.camera_modern(), Ordering::Relaxed);
	STRAFE_ON.store(opts.strafe_on(), Ordering::Relaxed);
	BUYALL_ON.store(opts.buyall_on(), Ordering::Relaxed);
	modcam::set_active(opts.camera_modern());
	if let Ok(mut s) = ST.lock() {
		*s = St { opts, path: Some(path), last: None, lang_seen: None, warned: false };
	}
}

// ---- worker ---------------------------------------------------------------------------------------------------------

/// Worker, before each field of a level (not in the front end): sets up the reserved region on fresh RAM, applies F7 (flips the
/// camera byte), reads the row bytes back and applies / remembers a change, and patches the Camera Options table of the running
/// level. Returns the field's input with the F7 press taken out (it is consumed here).
pub fn pre_field(m: &mut ee_oracle::Machine, fin: &FieldIn) -> FieldIn {
	let mut out = *fin;
	if !enabled() {
		return out;
	}
	let level = m.level();
	if let Ok(mut s) = ST.lock() {
		if sync(&mut m.cpu.mem, level, fin.mc.toggle, &mut s) {
			out.mc.toggle = false;
		}
	}
	out
}

/// `pre_field` on the memory: true when the F7 press was consumed.
fn sync(mem: &mut Mem, level: u32, toggle: bool, s: &mut St) -> bool {
	if mem.rd32(BASE) != MAGIC {
		// Fresh RAM (start, savestate load, cold boot): the options file's values go in.
		mem.wr32(BASE, MAGIC);
		let (c, t, b) = (s.opts.camera_modern() as u8, s.opts.strafe_on() as u8, s.opts.buyall_on() as u8);
		mem.wr8(CAM, c);
		mem.wr8(STRAFE, t);
		mem.wr8(BUYALL, b);
		s.last = Some((c, t, b));
		apply(s, c, t, b, false);
	}
	if toggle {
		let c = mem.rd8(CAM);
		mem.wr8(CAM, (c == 0) as u8);
		eprintln!("retail: F7: camera {}", if c == 0 { "Modern" } else { "Classic" });
	}
	let (c, t, b) = (mem.rd8(CAM), mem.rd8(STRAFE), mem.rd8(BUYALL));
	if s.last != Some((c, t, b)) {
		s.last = Some((c, t, b));
		apply(s, c, t, b, true);
	}
	patch_rows(mem, level);
	toggle
}

/// A row byte changed (or the region was set up): the camera hands and the strafe gate follow; a change is remembered.
fn apply(s: &mut St, cam: u8, strafe: u8, buyall: u8, changed: bool) {
	let (cam, strafe, buyall) = (cam == 1, strafe == 1, buyall == 1);
	BUYALL_ON.store(buyall, Ordering::Relaxed);
	CAM_MODERN.store(cam, Ordering::Relaxed);
	STRAFE_ON.store(strafe, Ordering::Relaxed);
	modcam::set_active(cam);
	if changed {
		eprintln!("retail: PC options: camera {}, strafe aiming {}", if cam { "Modern" } else { "Classic" }, if strafe { "On" } else { "Off" });
		s.opts.camera = Some(cam);
		s.opts.strafe = Some(strafe);
		s.opts.buyall = Some(buyall);
		s.save();
	}
}

/// The Camera Options table of `level`, when the retail one is there: builds the five-record copy and repoints the item to it.
/// Idempotent: a second look finds the copy's address, not the table's. Guards: the level has a row, the item points at the
/// table and the table starts with the retail signature, so another overlay (loading, other level) is never touched.
fn patch_rows(mem: &mut Mem, level: u32) -> bool {
	let Some(&t) = TABLES.get(level as usize) else { return false };
	if mem.rd32(t + PTR_OFF) != t || mem.rd32(t) != SIG[0] || mem.rd32(t + 4) != SIG[1] {
		return false;
	}
	// The three retail records, then Camera and Strafe, then the zero record.
	let mut w: Vec<u32> = (0..18).map(|i| mem.rd32(t + 4 * i)).collect();
	w.extend([ID_CAMERA, CAM, ID_CLASSIC, ID_MODERN, 0, 0]);
	w.extend([ID_STRAFE, STRAFE, ID_OFF, ID_ON, 0, 0]);
	w.extend([ID_BUYALL, BUYALL, ID_OFF, ID_ON, 0, 0]);
	w.extend([0; 6]);
	for (i, v) in w.iter().enumerate() {
		mem.wr32(TAB + 4 * i as u32, *v);
	}
	mem.wr32(t + PTR_OFF, TAB);
	true
}

/// The text lookup hand: the three new ids get their string for the game's language; every other id goes to `prompts::lookup`
/// (keyboard prompts; it declines with a gamepad, so retail runs).
fn text_hook(c: &mut Cpu) -> bool {
	let id = c.g(4) as u32;
	// Not one of ours: the keyboard's button prompts (prompts.rs) or retail's own lookup.
	let Some(k) = NEW_IDS.iter().position(|&n| n == id) else { return super::prompts::lookup(c) };
	let lang = Lang::from_word(c.mem.rd32(LANG_WORD)).map_or(0, |l| l as usize);
	let at = POOL + 0x40 * k as u32;
	let s = text_for(k, lang);
	// Keyboard: key names for the glyphs (slot is 0x40 bytes with the terminator).
	let s = super::prompts::for_device(&s).filter(|t| t.len() < 0x40).unwrap_or(s);
	for (i, b) in s.iter().enumerate() {
		c.mem.wr8(at + i as u32, *b);
	}
	c.mem.wr8(at + s.len() as u32, 0);
	c.s(2, at as u64);
	true
}

fn text_for(k: usize, lang: usize) -> Vec<u8> {
	TEXTS[k][lang].to_vec()
}

/// Cold boots only: remembers Options -> Language. Called each field while the front end runs: the game sets its language word
/// from the console language during the first fields, so the baseline is taken from field 4; a later change is the
/// player's choice.
pub fn watch_language(m: &ee_oracle::Machine, field: u32, in_front: bool) {
	if !enabled() || field < 4 || !in_front {
		return;
	}
	let w = m.rd32(LANG_WORD);
	let Ok(mut s) = ST.lock() else { return };
	match s.lang_seen {
		None => s.lang_seen = Some(w),
		Some(p) if p != w => {
			s.lang_seen = Some(w);
			if let Some(l) = Lang::from_word(w) {
				eprintln!("retail: PC options: language {} remembered (field {field})", l.code());
				s.opts.lang = Some(l);
				s.save();
			}
		}
		_ => {}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn state_dir(name: &str) -> Option<String> {
		[concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/")].iter().map(|b| format!("{b}{name}")).find(|p| std::path::Path::new(p).join("eeMemory.bin").exists())
	}

	fn scratch(name: &str) -> PathBuf {
		let d = std::env::temp_dir().join(format!("pcext_test_{}_{name}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		d
	}

	#[test]
	fn strafe_gate_truth_table() {
		// Extension on: only Modern + On.
		assert!(gate(true, true, true));
		assert!(!gate(true, true, false));
		assert!(!gate(true, false, true), "Classic camera: strafe is off whatever the row says");
		assert!(!gate(true, false, false));
		// Extension off: the old switches decide, the menu does not restrict.
		assert!(gate(false, false, false));
	}

	#[test]
	fn texts_fit_the_font_and_are_nonempty() {
		for row in TEXTS {
			for s in row {
				assert!(!s.is_empty() && s.len() < 0x3f);
				// Printable ASCII, the game's accent bytes (0x80..0xb0, see TEXTS) or the plate's newline (1) and button glyphs (0x10, 0x13).
				assert!(s.iter().all(|&b| (0x20..0x7f).contains(&b) || (0x80..=0xb0).contains(&b) || [1, 0x10, 0x13].contains(&b)));
			}
		}
	}

	/// The hashes and table addresses against the extracted overlays: the text lookup of every level and the Camera Options row table.
	#[test]
	fn tables_match_the_overlays() {
		let dir = [concat!(env!("CARGO_MANIFEST_DIR"), "/../extracted/levels")].into_iter().find(|d| std::path::Path::new(d).is_dir());
		let Some(dir) = dir else { return };
		let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().join("overlay.elf").exists()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
		names.sort_by_key(|n| n.split('_').next().and_then(|x| x.parse::<u32>().ok()).unwrap_or(99));
		assert_eq!(names.len(), 19);
		for (lv, name) in names.iter().enumerate() {
			let elf = std::fs::read(format!("{dir}/{name}/overlay.elf")).unwrap();
			let word = |a: u32| -> Option<u32> {
				let u = |o: usize| u32::from_le_bytes(elf[o..o + 4].try_into().unwrap()) as usize;
				let (phoff, phn) = (u(0x1c), u16::from_le_bytes([elf[0x2c], elf[0x2d]]) as usize);
				(0..phn).find_map(|i| {
					let p = phoff + 32 * i;
					let (ty, off, va, fs) = (u(p), u(p + 4), u(p + 8) as u32, u(p + 16));
					(ty == 1 && a >= va && (a - va) as usize + 4 <= fs).then(|| u(off + (a - va) as usize) as u32)
				})
			};
			let h = &HANDS[lv];
			let mut hash = 0xcbf2_9ce4_8422_2325u64;
			for a in (h.entry..h.end).step_by(4) {
				hash = (hash ^ word(a).unwrap() as u64).wrapping_mul(0x100_0000_01b3);
			}
			assert_eq!(hash, h.hash, "text lookup hash, level {lv} {name}");
			assert_eq!(h.end - h.entry, 0x40);
			// The row table: signature, and the item (table + 0x60) points back at it.
			let t = TABLES[lv];
			assert_eq!((word(t), word(t + 4), word(t + 8), word(t + 12), word(t + 16), word(t + 20)), (Some(0x4fa2), Some(0x15ede0), Some(0x4fa5), Some(0x4fa4), Some(0), Some(0)), "table signature, level {lv}");
			assert_eq!(word(t + PTR_OFF), Some(t), "item pointer, level {lv}");
		}
	}

	/// No swarm hand sits on one of the text lookups (the PC hand would replace it; its decline path runs the retail code).
	#[test]
	fn no_hand_of_the_swarm_on_a_text_lookup() {
		ee_game::register();
		let n: Vec<_> = ee_oracle::rec::hands().into_iter().filter(|h| HANDS.iter().any(|p| p.entry == h.entry && !std::ptr::eq(*h, p))).map(|h| h.entry).collect();
		eprintln!("hands on a text lookup entry besides ours: {n:x?}");
		assert!(n.is_empty(), "a hand of ee-hand sits on a text lookup: {n:x?}");
	}

	/// On a loaded Eudora state: the lookup answers the new ids in all five languages, declines the retail ones, the row table is
	/// patched once, and a byte change is applied and remembered.
	#[test]
	fn eudora_text_rows_and_persistence() {
		let Some(dir) = state_dir("fg1/4_0159") else { return };
		let mut m = ee_oracle::Machine::load(&dir);
		ee_game::register();
		register_hand(&HANDS);
		m.enable_rec();
		let level = m.level();
		assert_eq!(level, 4);
		// Text lookup: the new ids in every language, via the real call path.
		let pool = |m: &mut ee_oracle::Machine, id: u32, word: u32| -> String {
			m.cpu.mem.wr32(LANG_WORD, word);
			m.cpu.s(4, id as u64);
			m.cpu.call(0x1f6b80);
			let p = m.cpu.g(2) as u32;
			let mut out = Vec::new();
			let mut a = p;
			while m.cpu.mem.rd8(a) != 0 {
				out.push(m.cpu.mem.rd8(a));
				a += 1;
			}
			assert!((POOL..POOL + 0x140).contains(&p) || id < 0x7f00, "pool pointer for {id:#x}: {p:#x}");
			out.iter().map(|&b| b as char).collect()
		};
		for (word, expect) in [(0u32, "Modern"), (2, "Moderne"), (3, "Modern"), (4, "Moderna"), (5, "Moderna")] {
			assert_eq!(pool(&mut m, ID_MODERN, word), expect, "language word {word}");
		}
		assert_eq!(pool(&mut m, ID_STRAFE, 3), "Strafe-Zielen");
		assert_eq!(pool(&mut m, ID_CLASSIC, 0), "Classic");
		// A retail id declines: the retail lookup runs and finds the German 'Kamera' (the state is German).
		m.cpu.mem.wr32(LANG_WORD, 3);
		assert_eq!(pool(&mut m, ID_CAMERA, 3), "Kamera");
		assert_eq!(pool(&mut m, ID_ON, 3), "ein");
		assert_eq!(pool(&mut m, ID_OFF, 3), "aus");
		// Rows: the retail table is patched once, then idempotently.
		let t = TABLES[4];
		assert_eq!(m.cpu.mem.rd32(t + PTR_OFF), t);
		let dirp = scratch("eudora");
		let mut s = St { opts: PcOptions::default(), path: Some(dirp.join("pc_options.txt")), last: None, lang_seen: None, warned: false };
		assert!(!sync(&mut m.cpu.mem, 4, false, &mut s));
		assert_eq!(m.cpu.mem.rd32(t + PTR_OFF), TAB);
		assert_eq!((m.cpu.mem.rd8(CAM), m.cpu.mem.rd8(STRAFE), m.cpu.mem.rd8(BUYALL)), (1, 1, 1), "new player: Modern, strafe On, buy all On");
		let rec = |mem: &mut Mem, i: u32| -> Vec<u32> { (0..6).map(|k| mem.rd32(TAB + 24 * i + 4 * k)).collect() };
		assert_eq!(rec(&mut m.cpu.mem, 0), [0x4fa2, 0x15ede0, 0x4fa5, 0x4fa4, 0, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 3), [ID_CAMERA, CAM, ID_CLASSIC, ID_MODERN, 0, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 4), [ID_STRAFE, STRAFE, ID_OFF, ID_ON, 0, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 5), [ID_BUYALL, BUYALL, ID_OFF, ID_ON, 0, 0]);
		assert_eq!(rec(&mut m.cpu.mem, 6), [0; 6]);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		assert_eq!(m.cpu.mem.rd32(t + PTR_OFF), TAB);
		assert!(!dirp.join("pc_options.txt").exists(), "nothing is written until the player changes something");
		// The player flips Camera to Classic in the menu (the retail update writes the byte): applied, remembered, strafe follows.
		m.cpu.mem.wr8(CAM, 0);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		assert!(!CAM_MODERN.load(Ordering::Relaxed));
		assert_eq!(PcOptions::load(&dirp.join("pc_options.txt")).camera, Some(false));
		// F7 flips it back and is consumed.
		assert!(sync(&mut m.cpu.mem, 4, true, &mut s));
		assert_eq!(m.cpu.mem.rd8(CAM), 1);
		assert!(CAM_MODERN.load(Ordering::Relaxed));
		// Strafe Off in the menu.
		m.cpu.mem.wr8(STRAFE, 0);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		assert!(!STRAFE_ON.load(Ordering::Relaxed));
		assert_eq!(PcOptions::load(&dirp.join("pc_options.txt")).strafe, Some(false));
		// A level reload re-initialises the overlay data: the next look patches again, the bytes stay.
		m.cpu.mem.wr32(t + PTR_OFF, t);
		sync(&mut m.cpu.mem, 4, false, &mut s);
		assert_eq!(m.cpu.mem.rd32(t + PTR_OFF), TAB);
		assert_eq!(m.cpu.mem.rd8(STRAFE), 0);
		// Another level's guard: never touches a wrong overlay.
		assert!(!patch_rows(&mut m.cpu.mem, 5));
		assert!(!patch_rows(&mut m.cpu.mem, 99));
		let _ = std::fs::remove_dir_all(&dirp);
		// Leave the process-wide state as a fresh run has it.
		CAM_MODERN.store(true, Ordering::Relaxed);
		STRAFE_ON.store(true, Ordering::Relaxed);
		modcam::set_active(true);
	}
}
