//! Button prompts that follow the device in use, and the gamepad as an input device.
//!
//! Retail [C, decomp/notes/button_prompts.md]: a text string is bytes of the font; codes 0x10..=0x19 are 24-wide coloured button
//! cells (0x10 Cross, 0x11 Circle, 0x12 Triangle, 0x13 Square, 0x14 L1, 0x15 R1, 0x16 L2, 0x17 R2, 0x18 right stick, 0x19 left
//! stick). Every string the game shows comes from its text lookup (`FUN_002254b0` twin, 19 levels + boot ELF + front end,
//! `pcext::HANDS`): id -> pointer into the loaded language bank. The draw (`FUN_0021cbf8`), the width measure (`FUN_0021cb40`)
//! and the word wrap (`FUN_0021d678`) all take that pointer, so a lookup that answers a copy with the glyph bytes spelled out as
//! key names makes retail's own layout measure, centre, wrap and draw the substituted text.
//!
//! Device (decided 2026-10-13): the game starts on the KEYBOARD. It becomes the gamepad only on real pad input (a button
//! pressed, a stick or trigger moved out of its deadzone) and the keyboard again on any key or mouse button press or clear mouse
//! movement; a connected but idle pad changes nothing. The host (window thread) keeps the latest verdict; the worker copies it
//! once per field (`begin_field`), so a field is consistent. Gamepad mode runs retail's lookup untouched; keyboard mode runs the
//! same instructions on the hand (stack write, search, registers) and answers a rewritten copy from a pool in reserved RAM
//! (`POOL`) only for strings that hold a glyph, written only when the slot differs; nothing else in RAM changes.
//!
//! Key names come from the keyboard bindings of `retail::pad_from_keys` (the `BINDINGS` table below is checked against it by
//! the unit tests): the English keycap names, in all five languages (a key name is a label on a physical key, not a word of the
//! sentence, and the pad's glyphs were language-free as well).
//! Exception: a short legend line that starts with Cross ("Select file", "Seleccionar") gets "Spc" (or "Sp") instead of "Space" when
//! "Space" plus the label would be wider than retail's legend box (see `substitute`).
//!
//! The gamepad (XInput): A = Cross, B = Circle, X = Square, Y = Triangle, LB = L1, RB = R1, LT = L2, RT = R2, Back = Select,
//! Start = Start, d-pad, both sticks analog as PCSX2 gives them to the game by default (`stick`), L3 / R3. A PlayStation controller (pspad.rs: DualShock 4,
//! DualSense, read over HID) arrives as an XInput report with its buttons in their PS2 places and goes the same way. The pads
//! are merged with the keyboard (OR of buttons, the larger deflection of each stick axis) and work without any switch; only
//! the keyboard prompts follow `RAC_PC_EXT`.
//!
//! Dev: `RAC_PROMPT_DEV` = `from-to:kb|pad;...` (fields, `to` may be empty = open end) forces the device per field in scripted
//! runs; `RAC_PROMPT_CANARY=1` fills the pool range with 0xA5 and logs any field that finds a byte of it changed by anything but
//! this file (the proof that retail never writes there). `RAC_PROMPT_LOG=from-to` prints every text lookup of those fields
//! ("prompts: lookup field F id X kb|pad"); both are off unless the variable is set.
use crate::fmath::Fm;
use ee_oracle::cpu::Cpu;
use ee_oracle::rec::Hand;
use ee_oracle::mem::Mem;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use winit::keyboard::KeyCode;

const KB: u8 = 0;
const PAD: u8 = 1;
/// The device the host saw last (window thread) and the one the worker uses for the current field.
static HOST: AtomicU8 = AtomicU8::new(KB);
static FIELD: AtomicU8 = AtomicU8::new(KB);
/// A retail page menu is open (set by the worker after each field): Circle is then the right mouse button (back).
static MENU: AtomicBool = AtomicBool::new(false);

/// Reserved RAM for the substituted strings (see decomp/notes/button_prompts.md for the proof that retail never writes it):
/// `SLOTS` slots of `SLOT` bytes.
pub const POOL: u32 = super::pcext::BASE + 0x2000;
const SLOT: u32 = 0x400;
const SLOTS: usize = 32;

/// Button glyph codes of the font.
pub const CROSS: u8 = 0x10;
pub const CIRCLE: u8 = 0x11;
pub const TRIANGLE: u8 = 0x12;
pub const SQUARE: u8 = 0x13;
pub const L1: u8 = 0x14;
pub const R1: u8 = 0x15;
pub const L2: u8 = 0x16;
pub const R2: u8 = 0x17;
pub const RSTICK: u8 = 0x18;
pub const LSTICK: u8 = 0x19;

// ---- key names (the keyboard bindings) --------------------------------------------------------------------------------

/// One glyph and its binding in `retail::pad_from_keys`: the PS2 pad bit, the keys that set it, the name shown.
pub struct Binding {
	pub code: u8,
	/// DualShock bit(s) of `pad_from_keys` (the sticks: the axis indices instead, see `Binding::axes`).
	pub bit: u16,
	pub keys: &'static [KeyCode],
	pub name: &'static str,
}

/// The glyphs a text can hold. Circle also has the left mouse button (fire) and, in a menu, the right one (back); a menu line
/// names it G when it is no confirm / back / Tab button of the screen (the left button clicks the menu there, `menu_keys::key_for`); the right stick
/// is the mouse with the Modern camera and Q/E/Z/X otherwise (both always work, see `key_name`); Triangle also has the middle
/// mouse button.
pub const BINDINGS: [Binding; 10] = [
	Binding { code: CROSS, bit: 0x40, keys: &[KeyCode::Space], name: "Space" },
	Binding { code: CIRCLE, bit: 0x20, keys: &[KeyCode::KeyG], name: "LMB" },
	Binding { code: TRIANGLE, bit: 0x10, keys: &[KeyCode::KeyT], name: "T" },
	Binding { code: SQUARE, bit: 0x80, keys: &[KeyCode::KeyF], name: "F" },
	Binding { code: L1, bit: 4, keys: &[KeyCode::KeyC], name: "C" },
	Binding { code: R1, bit: 8, keys: &[KeyCode::ControlLeft], name: "Ctrl" },
	Binding { code: L2, bit: 1, keys: &[KeyCode::KeyV], name: "V" },
	Binding { code: R2, bit: 2, keys: &[KeyCode::KeyB], name: "B" },
	Binding { code: RSTICK, bit: 0, keys: &[KeyCode::KeyQ, KeyCode::KeyE, KeyCode::KeyZ, KeyCode::KeyX], name: "Q/E/Z/X" },
	Binding { code: LSTICK, bit: 0, keys: &[KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD], name: "WASD" },
];

/// What the player sees of the world when a name is chosen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ctx {
	/// Camera row Modern (the mouse is the right stick).
	pub modern_camera: bool,
	/// The retail page menu on screen and what its keys press (`menu_keys::screen`); None outside menus.
	pub menu: Option<super::menu_keys::Screen>,
}

/// The binding function (`binds::CROSS` ..) of a button glyph code (Cross .. R2); None for the sticks and other bytes.
fn bound_function(code: u8) -> Option<usize> {
	use super::binds as b;
	Some(match code {
		CROSS => b::CROSS,
		CIRCLE => b::CIRCLE,
		TRIANGLE => b::TRIANGLE,
		SQUARE => b::SQUARE,
		L1 => b::L1,
		R1 => b::R1,
		L2 => b::L2,
		R2 => b::R2,
		_ => return None,
	})
}

/// The key name of a glyph code for the keyboard; None for a byte that is not a button glyph. A remapped function (Pause ->
/// Options -> Controls, `binds`) shows the key it was given; one that still has its default key keeps the name of `BINDINGS`.
pub fn key_name(code: u8, ctx: Ctx) -> Option<Cow<'static, [u8]>> {
	use super::binds as b;
	let own = |n: Option<String>| n.map(|n| Cow::Owned(n.into_bytes()));
	let stick = |first: usize| match own(b::stick_name(first)) {
		Some(n) => Some(n),
		None => BINDINGS.iter().find(|k| k.code == if first == b::LSTICK { LSTICK } else { RSTICK }).map(|k| Cow::Borrowed(k.name.as_bytes())),
	};
	match code {
		RSTICK if ctx.modern_camera => Some(Cow::Borrowed(&b"Mouse"[..])),
		RSTICK => stick(b::RSTICK),
		LSTICK => stick(b::LSTICK),
		c => own(bound_function(c).and_then(b::prompt_name)).or_else(|| BINDINGS.iter().find(|k| k.code == c).map(|k| Cow::Borrowed(k.name.as_bytes()))),
	}
}

/// Legend boxes of the menus have a fixed size (art of the page: a panel of the menu atlas, text drawn left-aligned and clipped at
/// its right edge once it is wider than the box: decomp/notes/button_prompts.md), sized for the retail line "glyph + word" with
/// the glyph 18 px wide. A short legend line (at most `LEGEND_MAX_BYTES` bytes, starting with Cross) therefore gets the longest of
/// "Space", "Spc", "Sp" that still fits `LEGEND_FIT_PX` (measured with the title font's advance table `FONT_ADV`, the table
/// retail measures with). Cold boot shots (2026-10-13): en Load legend "Spc Select file" (128 px) and fr "Spc Choix fich." (129)
/// fit; "Spc Seleccionar" (139, Spanish) loses its last letter, so Spanish gets "Sp" (130). Sentences (long strings, or Cross
/// inside the text) are wrapped by retail and always say "Space".
const LEGEND_FIT_PX: u32 = 129;
const LEGEND_MAX_BYTES: usize = 24;
const LEGEND_NAMES: [&[u8]; 3] = [b"Space", b"Spc", b"Sp"];

/// Advance (px) of each font byte in the title font (table 0x1defd0 of the front-end bin, byte +3 of each 4-byte entry).
const FONT_ADV: [u8; 256] = [
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	18, 18, 18, 18, 25, 25, 25, 25, 16, 16, 0, 0, 0, 0, 0, 0,
	5, 7, 9, 18, 11, 18, 0, 4, 9, 9, 0, 9, 6, 9, 6, 7,
	13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 6, 6, 11, 0, 11, 11,
	18, 14, 13, 13, 14, 12, 12, 14, 14, 4, 9, 13, 11, 16, 13, 16,
	14, 16, 14, 14, 13, 14, 14, 16, 13, 13, 13, 0, 0, 0, 18, 14,
	0, 10, 11, 9, 10, 11, 8, 10, 10, 4, 6, 9, 4, 15, 10, 11,
	11, 10, 7, 9, 8, 10, 10, 15, 10, 11, 9, 0, 0, 0, 0, 0,
	14, 14, 14, 14, 10, 10, 10, 10, 12, 12, 12, 12, 11, 11, 11, 11,
	5, 5, 5, 5, 6, 6, 6, 6, 16, 16, 16, 16, 11, 11, 11, 11,
	14, 14, 14, 14, 10, 10, 10, 10, 13, 10, 13, 9, 12, 16, 7, 11,
	15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	1, 1, 2, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0,
	254, 254, 254, 254, 254, 254, 254, 254, 2, 2, 3, 2, 1, 1, 1, 1,
	2, 2, 3, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	0, 0, 0, 0, 0, 0, 0, 0, 18, 18, 18, 18, 25, 25, 25, 25,
];

fn text_px(s: &[u8]) -> u32 {
	s.iter().map(|&b| FONT_ADV[b as usize] as u32).sum()
}

/// Menu texts whose glyphs name the MENU keys (the legend / hint lines of the page menus, decomp/notes/button_prompts.md
/// "Modern menu controls"): '\x12 Exit', '\x10 Toggle', '\x10 Equip', '\x10 View Missions', '\x11 Play Infobot', the map legend
/// 0x4efa-0x4efc, the Help / Options legend 0x4f05, Load / Save 'Select file' 0x4f27, '\x14/\x15 to view other planets', '\x18 zooms',
/// '\x19 tracks', the Yes / No / Quit / Continue answers. Any other short line that starts with a glyph counts as a legend too.
pub const MENU_TEXTS: [u16; 18] = [0x4ee0, 0x4edf, 0x4eec, 0x4eee, 0x4eef, 0x4efa, 0x4efb, 0x4efc, 0x4f05, 0x4f27, 0x4f55, 0x4f56, 0x4f57, 0x5248, 0x5249, 0x524a, 0x524b, 0x524f];

/// The menu key of glyph `code` on screen `sc` (`short`: the abbreviation that fits a tight legend box); None = no menu key
/// (the gameplay name stands).
pub fn menu_name(code: u8, sc: &super::menu_keys::Screen, short: bool) -> Option<&'static [u8]> {
	match code {
		RSTICK if sc.sticks => return Some(b"Z/X"),
		LSTICK if sc.sticks => return Some(b"WASD"),
		_ => {}
	}
	let bit = BINDINGS.iter().find(|b| b.code == code && b.bit != 0)?.bit;
	super::menu_keys::key_for(sc, bit, short).map(str::as_bytes)
}

/// The keyboard key of glyph `code` on screen `sc` as the modern pages print it: the menu key, else the play key; "" for a byte
/// that is no button glyph.
pub fn menu_key(code: u8, sc: &super::menu_keys::Screen) -> String {
	let ctx = Ctx { modern_camera: super::modcam::on(), menu: Some(*sc) };
	let name = menu_name(code, sc, false).map(Cow::Borrowed).or_else(|| key_name(code, ctx));
	name.map(|n| String::from_utf8_lossy(&n).into_owned()).unwrap_or_default()
}

/// `s` with every button glyph replaced by its key name; None when it holds none.
pub fn substitute(s: &[u8], ctx: Ctx) -> Option<Vec<u8>> {
	substitute_text(s, ctx, 0)
}

/// `substitute` for the text `id`: inside a page menu its legend / hint lines (`MENU_TEXTS`, or a short line that starts with
/// a glyph) name the MENU keys of the screen (Enter, Esc, Q / E, Tab); sentences (help bodies describe play) keep the play keys.
pub fn substitute_text(s: &[u8], ctx: Ctx, id: u32) -> Option<Vec<u8>> {
	if !s.iter().any(|&b| (CROSS..=LSTICK).contains(&b)) {
		return None;
	}
	let short_line = (CROSS..=LSTICK).contains(&s[0]) && s.len() <= LEGEND_MAX_BYTES;
	if let Some(sc) = ctx.menu.filter(|_| short_line || MENU_TEXTS.contains(&(id as u16))) {
		let rest = &s[1..1 + s.iter().skip(1).take_while(|&&b| b >= 0x20).count()];
		let mut o = Vec::with_capacity(s.len() + 16);
		for (i, &b) in s.iter().enumerate() {
			let name = if i == 0 && short_line {
				// The first glyph of a legend line: the long name when it fits the box, else the short one; a confirm line that
				// does not fit with "Ent" either names the other confirm key, Space, as "Sp" (de "Sp Umschalten" 130 px; "Ent
				// Umschalten" 135 loses its last letter). The vendor's plates (font 2, 89 px) always take the short one; the
				// mode-4 dialogs centre their lines with no box, so they keep the long name.
				let fits = |n: &[u8]| super::menu_keys::is_dialog(&sc) || text_px(n) + text_px(rest) <= LEGEND_FIT_PX;
				let vendor = sc == super::menu_keys::VENDOR;
				menu_name(b, &sc, false)
					.filter(|n| !vendor && fits(n))
					.or_else(|| menu_name(b, &sc, true).filter(|n| vendor || fits(n)))
					.or_else(|| BINDINGS.iter().any(|k| k.code == b && k.bit == sc.confirm && k.bit != 0).then_some(&b"Sp"[..]))
					.or_else(|| menu_name(b, &sc, true))
			} else {
				menu_name(b, &sc, false)
			};
			match name.map(Cow::Borrowed).or_else(|| key_name(b, ctx)) {
				Some(k) => o.extend_from_slice(&k),
				None => o.push(b),
			}
		}
		return Some(o);
	}
	let mut legend: Option<&'static [u8]> = None;
	// A remapped Cross names its key as it is (no "Spc").
	if s[0] == CROSS && s.len() <= LEGEND_MAX_BYTES && super::binds::prompt_name(super::binds::CROSS).is_none() {
		let rest = &s[1..1 + s.iter().skip(1).take_while(|&&b| b >= 0x20).count()];
		let px = text_px(rest);
		legend = LEGEND_NAMES.iter().copied().find(|n| text_px(n) + px <= LEGEND_FIT_PX).or(LEGEND_NAMES.last().copied());
	}
	let mut o = Vec::with_capacity(s.len() + 16);
	for (i, &b) in s.iter().enumerate() {
		if i == 0 {
			if let Some(n) = legend {
				o.extend_from_slice(n);
				continue;
			}
		}
		match key_name(b, ctx) {
			Some(k) => o.extend_from_slice(&k),
			None => o.push(b),
		}
	}
	Some(o)
}

#[cfg(test)]
thread_local! {
	/// Tests: the menu screen `ctx_now` reports on this thread (the oracle runs a test's hands on its own thread).
	static TEST_MENU: std::cell::Cell<Option<super::menu_keys::Screen>> = const { std::cell::Cell::new(None) };
}

/// Tests: `ctx_now` on this thread names the keys of screen `sc` (None: the published screen as at run time).
#[cfg(test)]
pub fn test_menu(sc: Option<super::menu_keys::Screen>) {
	TEST_MENU.with(|t| t.set(sc));
}

fn ctx_now() -> Ctx {
	#[cfg(test)]
	if let Some(sc) = TEST_MENU.with(|t| t.get()) {
		return Ctx { modern_camera: super::modcam::on(), menu: Some(sc) };
	}
	Ctx { modern_camera: super::modcam::on(), menu: super::menu_keys::current().filter(|_| MENU.load(Ordering::Relaxed)) }
}

/// A bank text the PC draws itself (the title front end's mirror, frontend.rs; each char is one byte of the bank): its glyphs
/// spelled out as key names while the keyboard is the device (the menu keys while a page menu is open), unchanged otherwise.
pub fn for_text(s: &str) -> String {
	if !keyboard_now() {
		return s.to_string();
	}
	let b: Vec<u8> = s.chars().map(|c| c as u32 as u8).collect();
	match substitute_text(&b, ctx_now(), 0) {
		Some(o) => o.iter().map(|&c| c as char).collect(),
		None => s.to_string(),
	}
}

/// A whole string a PC extra builds itself (pcext's new text ids, e.g. the vendor's "[Square] Refill" plate line): key names for its
/// glyphs while the keyboard is the device, None while the gamepad is or when it holds no glyph.
pub fn for_device(s: &[u8]) -> Option<Vec<u8>> {
	if !keyboard_now() {
		return None;
	}
	substitute(s, ctx_now())
}

/// Glyph strings that never pass the text lookup (the Quick Select ring's "L1" / "R1", literal one-byte strings 0x14 / 0x15 of the
/// overlay, Eudora 0x160318 / 0x160320, drawn by the ring FUN_002713b0 through the font line wrapper FUN_001edce0 (x, y, colour,
/// a3 = string, t0 = -1): decomp/notes/button_prompts.md "Modern menu controls"). On every twin of the wrapper (`GLYPH_HANDS`):
/// with the keyboard the device and a button glyph in the string, a3 is pointed at a pool copy with the glyphs spelled out
/// (`substitute_text`: in a menu its keys, so the ring says "Q" / "E"); the hand then declines and retail's own wrapper draws
/// it. With a gamepad, or no glyph: nothing is touched.
pub fn glyph_draw_hook(c: &mut Cpu) -> bool {
	if keyboard_now() {
		spell_glyphs(c);
	}
	// Decline: retail's own wrapper draws (the Eudora hands that used to take the draw are gone).
	false
}

/// Keyboard: a3 -> a pool copy of the string with its glyphs spelled out (nothing when it holds no glyph).
fn spell_glyphs(c: &mut Cpu) {
	spell_arg(c, 7);
}

/// Argument register `reg` (4 = a0 .. 7 = a3) holds a string pointer: points it at a pool copy with the glyphs spelled out
/// (nothing changes when the string holds no glyph or is not in RAM). Used by the font line wrapper and the box text hook
/// (`prompts_box`).
pub(super) fn spell_arg(c: &mut Cpu, reg: usize) {
	let p = c.g(reg) as u32;
	if !(0x10_0000..0x0200_0000).contains(&p) {
		return;
	}
	let mut s = Vec::new();
	for i in 0..SLOT - 1 {
		match c.mem.rd8(p + i) {
			0 => break,
			b => s.push(b),
		}
	}
	// A string the read window cut, or whose spelled-out copy does not fit a pool slot (a mod's or translation's long text): left as it is,
	// like the lookup path (`.filter(|n| n.len() < SLOT - 1)`).
	if s.len() >= SLOT as usize - 1 {
		return;
	}
	let ctx = ctx_now();
	let Some(n) = substitute_text(&s, ctx, 0).filter(|n| n.len() < SLOT as usize - 1) else { return };
	let at = place(&mut c.mem, (p, fnv(&s), ctx.modern_camera, ctx.menu.map_or(0, super::menu_keys::pack) ^ fnv(&n)), &n);
	if at != 0 {
		c.s(reg, at as u64);
	}
}

/// The font line wrapper on every level (FUN_001edce0 in Eudora), the boot ELF and the front end: 31 instructions, hash = FNV-1a
/// over [entry, end) (twins.py / fninfo.py, 2026-10-13). Registered by `pcext::register`; it only spells the glyphs
/// and declines, so retail's wrapper still does the draw.
#[rustfmt::skip]
pub static GLYPH_HANDS: [Hand; 21] = [
	Hand { entry: 0x1fb2f8, end: 0x1fb374, hash: 0x3dfa40969489f6d5, est: 31, f: glyph_draw_hook }, // 0 veldin_tutorial
	Hand { entry: 0x21ce78, end: 0x21cef4, hash: 0x24f0bb0a4e57d995, est: 31, f: glyph_draw_hook }, // 1 novalis
	Hand { entry: 0x20c2b0, end: 0x20c32c, hash: 0x65fb785fb4f90009, est: 31, f: glyph_draw_hook }, // 2 aridia
	Hand { entry: 0x1f4be0, end: 0x1f4c5c, hash: 0xd3e7931263791dfd, est: 31, f: glyph_draw_hook }, // 3 kerwan
	Hand { entry: 0x1edce0, end: 0x1edd5c, hash: 0xb2b2af1fab8ae0bd, est: 31, f: glyph_draw_hook }, // 4 eudora
	Hand { entry: 0x229fb0, end: 0x22a02c, hash: 0x56eacbc69f95b349, est: 31, f: glyph_draw_hook }, // 5 rilgar
	Hand { entry: 0x212f60, end: 0x212fdc, hash: 0xb9007169e94b60bd, est: 31, f: glyph_draw_hook }, // 6 nebula_g34
	Hand { entry: 0x2230b0, end: 0x22312c, hash: 0xe6530dc167173d49, est: 31, f: glyph_draw_hook }, // 7 umbris
	Hand { entry: 0x20dd88, end: 0x20de04, hash: 0xa886d6c205981815, est: 31, f: glyph_draw_hook }, // 8 batalia
	Hand { entry: 0x21a630, end: 0x21a6ac, hash: 0x857430c9dab05409, est: 31, f: glyph_draw_hook }, // 9 gaspar
	Hand { entry: 0x1ee8e0, end: 0x1ee95c, hash: 0xe2b2ae744a2b4b7d, est: 31, f: glyph_draw_hook }, // 10 orxon
	Hand { entry: 0x22cbe0, end: 0x22cc5c, hash: 0x5f7939b2cc0261bd, est: 31, f: glyph_draw_hook }, // 11 pokitaru
	Hand { entry: 0x21de38, end: 0x21deb4, hash: 0x1f2f688eaacb8fd1, est: 31, f: glyph_draw_hook }, // 12 hoven
	Hand { entry: 0x206d60, end: 0x206ddc, hash: 0x09e619a708db343d, est: 31, f: glyph_draw_hook }, // 13 oltanis_orbit
	Hand { entry: 0x20e588, end: 0x20e604, hash: 0x48b3ae8af2c6e9cd, est: 31, f: glyph_draw_hook }, // 14 oltanis
	Hand { entry: 0x1f49b0, end: 0x1f4a2c, hash: 0x04065b007df7da49, est: 31, f: glyph_draw_hook }, // 15 quartu
	Hand { entry: 0x1f9ae0, end: 0x1f9b5c, hash: 0xa73d51d04b3d717d, est: 31, f: glyph_draw_hook }, // 16 kalebo_iii
	Hand { entry: 0x1f82e0, end: 0x1f835c, hash: 0x1a1b702773e94afd, est: 31, f: glyph_draw_hook }, // 17 veldin_orbit
	Hand { entry: 0x204060, end: 0x2040dc, hash: 0xc31d3304d58412bd, est: 31, f: glyph_draw_hook }, // 18 veldin_finale
// Boot ELF and frontbin twins (title, New Game, memory card); table differs (lui/addiu t2), same 31-insn shape.
	Hand { entry: 0x1f6438, end: 0x1f64b4, hash: 0xb08b0a46f9e2e11b, est: 31, f: glyph_draw_hook }, // boot ELF
	Hand { entry: 0x1f6178, end: 0x1f61f4, hash: 0xb7a8f00bac1f759b, est: 31, f: glyph_draw_hook }, // frontbin
];

/// True while the keyboard is the device of the current field.
pub fn keyboard_now() -> bool {
	FIELD.load(Ordering::Relaxed) == KB && super::pcext::enabled()
}

/// True while the keyboard (or the mouse) is the last device the window thread saw: a modern page that opens now starts with keycaps
/// in its hint bar, a pad player (real pad input seen last) with the pad glyphs.
pub fn host_is_keyboard() -> bool {
	HOST.load(Ordering::Relaxed) == KB
}

// ---- device ---------------------------------------------------------------------------------------------------------

/// Real pad input since the last `take_pad_pulse` (the viewer's cursor visibility, viewer/handler/cursor_vis.rs).
static PAD_PULSE: AtomicBool = AtomicBool::new(false);

/// True once per pad input seen by `poll_pad` since the last call (window thread).
pub fn take_pad_pulse() -> bool {
	PAD_PULSE.swap(false, Ordering::Relaxed)
}

/// A key or mouse button went down, or the mouse moved clearly (window thread).
pub fn note_keyboard() {
	if scripted(FIELD_NO.load(Ordering::Relaxed)).is_some() {
		return; // RAC_PROMPT_DEV decides the device of that field (pictures), scripted taps must not flip it
	}
	HOST.store(KB, Ordering::Relaxed);
}

/// A pad button went down (the modern menu hosts, `begin(pad)`): the pad is the device now, as `poll_pad` sets it for real pad input.
pub fn note_pad() {
	if scripted(FIELD_NO.load(Ordering::Relaxed)).is_some() {
		return;
	}
	HOST.store(PAD, Ordering::Relaxed);
}

/// Mouse movement counts as use of the keyboard side once it adds up to `MOVE_PX` pixels (counts) within `MOVE_GAP`.
const MOVE_PX: f32 = 40.0;
const MOVE_GAP_MS: u64 = 400;

/// Running sum of mouse movement; a pause of `MOVE_GAP_MS` starts it again.
#[derive(Default)]
pub struct MoveSum {
	last_ms: u64,
	sum: f32,
}

impl MoveSum {
	/// `dist` pixels moved at `now_ms`: true once the movement within the last `MOVE_GAP_MS` gaps reaches `MOVE_PX`.
	pub fn add(&mut self, now_ms: u64, dist: f32) -> bool {
		if now_ms.saturating_sub(self.last_ms) > MOVE_GAP_MS {
			self.sum = 0.0;
		}
		self.last_ms = now_ms;
		self.sum += dist;
		if self.sum >= MOVE_PX {
			self.sum = 0.0;
			return true;
		}
		false
	}
}

fn mouse_moved(dist: f32) {
	static M: Mutex<Option<(Instant, MoveSum)>> = Mutex::new(None);
	let Ok(mut g) = M.lock() else { return };
	let (t0, sum) = g.get_or_insert_with(|| (Instant::now(), MoveSum::default()));
	if sum.add(t0.elapsed().as_millis() as u64 + 1000, dist) {
		note_keyboard();
	}
}

/// Raw mouse motion (winit `DeviceEvent::MouseMotion`, the grabbed Modern camera).
pub fn note_mouse_motion(dx: f64, dy: f64) {
	mouse_moved((dx * dx + dy * dy).sqrt() as f32);
}

/// Window events: any key or mouse button press, the wheel and clear cursor movement are the keyboard (and mouse).
pub fn note_window_event(e: &winit::event::WindowEvent) {
	use winit::event::{ElementState, WindowEvent};
	match e {
		WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => note_keyboard(),
		WindowEvent::MouseInput { state: ElementState::Pressed, .. } => note_keyboard(),
		WindowEvent::MouseWheel { .. } => note_keyboard(),
		WindowEvent::CursorMoved { position, .. } => {
			static LAST: Mutex<Option<(Instant, f64, f64)>> = Mutex::new(None);
			let Ok(mut l) = LAST.lock() else { return };
			let now = Instant::now();
			// The first position after a pause only sets the reference (a cursor entering the window is no movement).
			if let Some((t, x, y)) = *l {
				if now.duration_since(t).as_millis() as u64 <= MOVE_GAP_MS {
					let d = ((position.x - x).powi(2) + (position.y - y).powi(2)).sqrt() as f32;
					*l = Some((now, position.x, position.y));
					mouse_moved(d);
					return;
				}
			}
			*l = Some((now, position.x, position.y));
		}
		_ => {}
	}
}

/// Agent test runs never read a real controller (`rac_engine::input::pad::real_pads_off`).
pub(crate) use rac_engine::input::pad::real_pads_off;

/// Start of the run (Retail::load): says which pad is there (the device stays the keyboard until the pad is used).
pub fn init() {
	// The game's hooks into the engine's pad reader (engine/input/pad.rs): the Stick dead zone row on the left stick, and on Linux the
	// evdev reader (lxpad) in place of pspad.
	rac_engine::input::pad::set_left_stick(super::pcinput::left_stick);
	#[cfg(target_os = "linux")]
	rac_engine::input::pad::set_hid_reader(super::lxpad::state, super::lxpad::connected);
	if super::pcext::enabled() {
		super::prompts_box::register();
	}
	if super::pcext::enabled() && !real_pads_off() && rac_engine::input::pad::connected() {
		eprintln!("retail: prompts: gamepad found, keyboard prompts until it is used");
	}
}

/// The scripted device of a field (`RAC_PROMPT_DEV`), if any.
fn scripted(field: u32) -> Option<u8> {
	static S: OnceLock<Vec<(u32, u32, u8)>> = OnceLock::new();
	let s = S.get_or_init(|| {
		let v = std::env::var("RAC_PROMPT_DEV").unwrap_or_default();
		v.split(';')
			.filter_map(|e| {
				let (r, d) = e.split_once(':')?;
				let (a, z) = r.split_once('-')?;
				let dev = match d.trim() {
					"kb" => KB,
					"pad" => PAD,
					_ => return None,
				};
				Some((a.trim().parse().ok()?, z.trim().parse().unwrap_or(u32::MAX), dev))
			})
			.collect()
	});
	s.iter().rev().find(|(a, z, _)| (*a..*z).contains(&field)).map(|x| x.2)
}

/// Dev (`RAC_PROMPT_LOG=from-to`): logs every text lookup of those fields (to find out when retail asks again).
fn lookup_log(field: u32) -> bool {
	static R: OnceLock<Option<(u32, u32)>> = OnceLock::new();
	let r = R.get_or_init(|| {
		let v = std::env::var("RAC_PROMPT_LOG").ok()?;
		let (a, z) = v.split_once('-')?;
		Some((a.trim().parse().ok()?, z.trim().parse().ok()?))
	});
	r.is_some_and(|(a, z)| (a..=z).contains(&field))
}

/// The field the worker is running (for the dev log).
static FIELD_NO: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The field the worker is running (the map page's titles, `map_seen`).
pub fn field_no() -> u32 {
	FIELD_NO.load(Ordering::Relaxed)
}

/// Worker, before each field: the device this field's texts use.
pub fn begin_field(field: u32, mem: &mut Mem) {
	if !super::pcext::enabled() {
		return;
	}
	FIELD_NO.store(field, Ordering::Relaxed);
	let d = match scripted(field) {
		Some(d) => {
			HOST.store(d, Ordering::Relaxed); // the modern menus read the host device live (`host_is_keyboard`)
			d
		}
		None => HOST.load(Ordering::Relaxed),
	};
	if FIELD.swap(d, Ordering::Relaxed) != d {
		eprintln!("retail: prompts: {} prompts from field {field}", if d == PAD { "gamepad" } else { "keyboard" });
	}
	canary(mem, field);
}

/// Worker, after each field: whether a retail page menu is open.
pub fn set_menu(open: bool) {
	MENU.store(open, Ordering::Relaxed);
}

/// Dev proof that retail never writes the pool range (`RAC_PROMPT_CANARY=1`): the range is filled with 0xA5 once, and every
/// later field is checked; what differs and is not a slot this file wrote is logged (first 8 addresses, then a count).
fn canary(mem: &mut Mem, field: u32) {
	static ON: OnceLock<bool> = OnceLock::new();
	if !*ON.get_or_init(|| std::env::var("RAC_PROMPT_CANARY").is_ok_and(|v| v == "1")) {
		return;
	}
	const LO: u32 = super::pcext::BASE + 0x1000;
	const HI: u32 = super::pcext::BASE + 0x10000;
	static ARMED: AtomicBool = AtomicBool::new(false);
	static BAD: Mutex<u32> = Mutex::new(0);
	if !ARMED.swap(true, Ordering::Relaxed) {
		for a in (LO..HI).filter(|a| !super::pcctl::RANGE.contains(a) && !super::pcctl::DBG_RANGE.contains(a) && !super::pcmods::RANGE.contains(a) && !super::pcdisp::RANGE.contains(a)) {
			mem.wr8(a, 0xa5);
		}
		eprintln!("retail: canary armed {LO:#x}..{HI:#x} at field {field}");
		return;
	}
	let (own_lo, own_hi) = (POOL, POOL + SLOT * SLOTS as u32);
	// The Camera Options rows' text slots (pcext.rs camopts section) are ours too.
	let co = super::pcext::BASE + 0x1000..super::pcext::CO_POOL_END;
	let mut bad = BAD.lock().unwrap();
	for a in LO..HI {
		// The Controls page (pcctl.rs) lives in the range too and writes its own bytes, and so does its Debug section's text pool; the Mods page (pcmods.rs) likewise.
		if (own_lo..own_hi).contains(&a) || co.contains(&a) || super::pcctl::RANGE.contains(&a) || super::pcctl::DBG_RANGE.contains(&a) || super::pcmods::RANGE.contains(&a) || super::pcdisp::RANGE.contains(&a) || mem.rd8(a) == 0xa5 {
			continue;
		}
		*bad += 1;
		if *bad <= 8 {
			eprintln!("retail: canary: byte {a:#x} = {:#04x} at field {field} (pc of the last field's code unknown)", mem.rd8(a));
		}
		mem.wr8(a, 0xa5);
	}
	if field % 200 == 0 {
		eprintln!("retail: canary: {} changed bytes so far, field {field}", *bad);
	}
}

// ---- the text lookup ------------------------------------------------------------------------------------------------

struct Pool {
	/// (retail string pointer, hash of its bytes, camera, menu) -> slot.
	map: HashMap<(u32, u64, bool, u64), usize>,
	owner: [Option<(u32, u64, bool, u64)>; SLOTS],
	next: usize,
}

static POOLS: Mutex<Option<Pool>> = Mutex::new(None);

fn fnv(s: &[u8]) -> u64 {
	s.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// Writes `s` (+ NUL) to a pool slot for the key (a hit keeps the slot, so the pointer stays the same from field to field) and
/// returns its address. A slot whose bytes already equal `s` is not written.
fn place(mem: &mut Mem, key: (u32, u64, bool, u64), s: &[u8]) -> u32 {
	let Ok(mut g) = POOLS.lock() else { return 0 };
	let p = g.get_or_insert_with(|| Pool { map: HashMap::new(), owner: [None; SLOTS], next: 0 });
	let slot = match p.map.get(&key) {
		Some(&k) => k,
		None => {
			let k = p.next;
			p.next = (k + 1) % SLOTS;
			if let Some(old) = p.owner[k].take() {
				p.map.remove(&old);
			}
			p.owner[k] = Some(key);
			p.map.insert(key, k);
			k
		}
	};
	let at = POOL + SLOT * slot as u32;
	// RAM may be fresh (savestate load): compare, rewrite when different.
	let same = (0..=s.len()).all(|i| mem.rd8(at + i as u32) == s.get(i).copied().unwrap_or(0));
	if !same {
		for (i, b) in s.iter().enumerate() {
			mem.wr8(at + i as u32, *b);
		}
		mem.wr8(at + s.len() as u32, 0);
	}
	at
}

/// The text lookup hand (called from `pcext::text_hook` for every id that is not one of its own): with the keyboard in use, runs
/// retail's function instruction for instruction (stack frame, search, result registers) and answers the string with its glyphs
/// spelled out. Returns false (retail's own function runs, RAM and registers exactly as without this file) with a gamepad and
/// when the code is not the known 16-instruction shape.
pub fn lookup(c: &mut Cpu) -> bool {
	let id = c.g(4) as u32;
	super::map_seen::note(id, FIELD_NO.load(Ordering::Relaxed));
	let log = lookup_log(FIELD_NO.load(Ordering::Relaxed));
	if log {
		eprintln!("prompts: lookup field {} id {:#x} {}", FIELD_NO.load(Ordering::Relaxed), c.g(4) as u32, if keyboard_now() { "kb" } else { "pad" });
	}
	if !keyboard_now() {
		return false;
	}
	let pc = c.pc;
	let w = |c: &mut Cpu, o: u32| c.mem.rd32(pc + o);
	// addiu sp,-0x10 / sq ra / jal SEARCH / nop / move v1,v0 / bgezl v1 / lw v0,OFF(gp) / lui v0,HI / b / addiu v0,v0,LO / sll /
	// addu / lw v0,(v1) / lq ra / jr ra / addiu sp
	let (w0, w1, w2, w4, w5, w6, w7, w9, wd, we) = (w(c, 0), w(c, 4), w(c, 8), w(c, 0x10), w(c, 0x14), w(c, 0x18), w(c, 0x1c), w(c, 0x24), w(c, 0x30), w(c, 0x34));
	if w0 != 0x27bd_fff0 || w1 != 0x7fbf_0000 || w2 >> 26 != 3 || w4 != 0x0040_182d || w5 >> 16 != 0x0463 || w6 >> 16 != 0x8f82 || w7 >> 16 != 0x3c02 || w9 >> 16 != 0x2442 || wd != 0x8c62_0000 || we != 0x7bbf_0000 {
		return false;
	}
	let (w_lw, w_lui, w_addiu) = (w6, w7, w9);
	let search = ((w2 & 0x03ff_ffff) << 2) | (pc & 0xf000_0000);
	let (gp_off, hi, lo) = (w_lw as u16 as i16 as i32, w_lui & 0xffff, w_addiu as u16 as i16 as i32);
	// addiu sp,sp,-0x10 ; sq ra,0(sp)
	let (ra, sp0) = (c.r[31], c.g(29));
	let sp = (sp0 as u32).wrapping_sub(0x10);
	c.s(29, sp as i32 as i64 as u64);
	c.mem.wr128(sp, ra);
	c.call(search);
	// move v1,v0 ; bgezl v1
	let v0 = c.g(2);
	c.s(3, v0);
	let mut ptr = 0;
	if (v0 as i64) < 0 {
		// Not found: the "does not exist" literal (lui v0 / addiu v0 in the branch delay slot).
		c.s(2, ((hi << 16) as i32).wrapping_add(lo) as i64 as u64);
	} else {
		// lw v0,OFF(gp) ; sll v1,v1,4 ; addu v1,v1,v0 ; lw v0,(v1)
		let base = c.mem.rd32((c.g(28) as u32).wrapping_add(gp_off as u32));
		c.s(2, base as i32 as i64 as u64);
		let v1 = ((v0 as u32) << 4).wrapping_add(base);
		c.s(3, v1 as i32 as i64 as u64);
		ptr = c.mem.rd32(v1);
		c.s(2, ptr as i32 as i64 as u64);
	}
	// lq ra,0(sp) ; jr ra ; addiu sp,sp,0x10
	c.r[31] = ra;
	c.s(29, sp0);
	if ptr >= 0x100000 && ptr < 0x200_0000 {
		let mut s = Vec::with_capacity(128);
		while s.len() < 0x300 {
			let b = c.mem.rd8(ptr + s.len() as u32);
			if b == 0 {
				break;
			}
			s.push(b);
		}
		let ctx = ctx_now();
		if let Some(n) = substitute_text(&s, ctx, id).filter(|n| n.len() < SLOT as usize - 1) {
			let at = place(&mut c.mem, (ptr, fnv(&s), ctx.modern_camera, ctx.menu.map_or(0, super::menu_keys::pack) ^ fnv(&n)), &n);
			c.s(2, at as i32 as i64 as u64);
		}
	}
	true
}

// ---- the gamepad ----------------------------------------------------------------------------------------------------

pub use rac_engine::input::pad::{map_xinput, merge, pad_activity, PadState, XRaw};
use rac_engine::input::pad::read as pad_read;

fn pad_state() -> Option<PadState> {
	pad_read().map(|(p, _)| p)
}

/// The physical buttons of `pad_read` at the last two `poll_pad` calls (one per field).
static PHYS_NOW: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static PHYS_PREV: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The controller buttons (`binds::PHYS` masks) that went down between the last two `poll_pad` calls: what the Controls page
/// binds. A button held since before (the one that confirmed the row) is not fresh.
pub fn phys_fresh() -> u32 {
	PHYS_NOW.load(Ordering::Relaxed) & !PHYS_PREV.load(Ordering::Relaxed)
}

/// The controller buttons down at the last `poll_pad` call.
pub fn phys_now() -> u32 {
	PHYS_NOW.load(Ordering::Relaxed)
}

/// [PC] The connected gamepad's state now, without the prompt-device bookkeeping. Read once per display frame on the main thread by
/// the four modern hosts (frame_vendor.rs, frame_pause.rs, frame_ship.rs, front_modern.rs: their pad sample) and by the dev free
/// camera (devtools.rs), on top of `poll_pad` per field (R2b-4: the doc said dev only).
pub fn pad_peek() -> Option<PadState> {
	pad_state()
}

/// The connected pads' buttons now as PS2 bits (0 without a pad), without `poll_pad`'s prompt-device bookkeeping: the skip test of a
/// playing movie reads them (`Retail::movie_pressed`; a movie parks the game thread, so `tick_keys` is the only other reader).
pub fn pad_buttons() -> u16 {
	pad_state().map_or(0, |p| p.buttons)
}

/// R2 is the aim and is kept from the game: in gameplay only (`gameplay`: game mode 0), with the modern camera on, or with Camera
/// Classic while Strafe aiming is On (`aim_button`). Elsewhere it is retail's R2: the pause menu's button-code scanner (L1 + R2
/// held, exactly: Novalis FUN_002988a0 `held & 0xf == 6`, decomp/notes/hero_movement.md HERO-19b) needs it. In gameplay retail's
/// R2 only crouches and swims fast like its R1 (mask 10, hero_crouch_ledge.md), so nothing is lost there.
fn r2_is_aim(gameplay: bool, aim_button: bool) -> bool {
	gameplay && aim_button
}

/// The aim takes R2 away from the game only for Ratchet in his own body (`ratchet`: body form 0): Clank, Giant Clank and the
/// Hologuise have no aim (strafe.rs and aim.rs need form 0), so retail gets their R2 as it is (whether any of their moves reads it
/// is not decoded; passing it on is retail-exact either way).
fn r2_stripped(aim: bool, ratchet: bool) -> bool {
	aim && ratchet
}

/// Where the pad's R2 goes this field: (modern camera's aim mode down, retail's pad has R2). `pad_r2`: the pad's R2 is held;
/// `gameplay` / `ratchet`: the newest field's mode and body form; `aim_button`: Camera Classic + Strafe aiming On, or the modern camera.
/// Form 0 in gameplay: the aim takes R2. Everywhere else R2 is retail's, and nothing aims (no modcam aim for Clank / Hologuise).
fn r2_route(pad_r2: bool, gameplay: bool, ratchet: bool, aim_button: bool) -> (bool, bool) {
	let strip = r2_stripped(r2_is_aim(gameplay, aim_button), ratchet);
	(strip && pad_r2, pad_r2 && !strip)
}

/// Bit 0: the pad's R2 is held (this field's `poll_pad`); bit 1: the keyboard's R2 key (B) is held in the same field (never stripped).
static R2_HELD: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Takes the pad's R2 out of `buttons` when the aim has it (the call of `Retail::tick_inner`, after the newest snapshot is taken, so
/// the mode / body form are those of the field just finished: no lag in lockstep; with run-ahead (depth > 1) a mode change reaches
/// the pad up to depth - 1 fields late, `Retail::pad_gameplay`) and sets the modern camera's aim mode.
/// `buttons` holds the pad's R2 from `poll_pad`; the keyboard's R2 key stays.
pub fn route_r2(buttons: &mut u16, gameplay: bool, ratchet: bool) {
	let down = apply_r2(buttons, R2_HELD.load(Ordering::Relaxed), gameplay, ratchet, super::modcam::classic::aim_button());
	super::modcam::pad_aim(down);
}

/// `route_r2` without the globals: `held` as `R2_HELD`; returns whether the aim mode is down.
fn apply_r2(buttons: &mut u16, held: u8, gameplay: bool, ratchet: bool, aim_button: bool) -> bool {
	let (aim_down, to_game) = r2_route(held & 1 != 0, gameplay, ratchet, aim_button);
	if held & 3 == 1 && !to_game {
		*buttons &= !2;
	}
	aim_down
}

/// Gamepad input for this field (Retail::tick_keys, window thread): merges the pad's buttons and sticks into the keyboard's and
/// makes the gamepad the device when `pad_activity` says so. Nothing without a connected pad. The pad's R2 is merged like the other
/// buttons and noted (`R2_HELD`); whether the aim keeps it from the game is decided later, where the newest snapshot is known
/// (`route_r2`).
pub fn poll_pad(buttons: &mut u16, sticks: &mut [u8; 4]) {
	let pr = pad_read();
	PHYS_PREV.store(PHYS_NOW.load(Ordering::Relaxed), Ordering::Relaxed);
	PHYS_NOW.store(pr.map_or(0, |r| r.1), Ordering::Relaxed);
	let Some((p, _)) = pr else {
		R2_HELD.store(0, Ordering::Relaxed);
		return;
	};
	R2_HELD.store((p.buttons & 2 != 0) as u8 | (((*buttons & 2 != 0) as u8) << 1), Ordering::Relaxed);
	static PREV: Mutex<PadState> = Mutex::new(PadState::IDLE);
	if let Ok(mut prev) = PREV.lock() {
		if pad_activity(&prev, &p) {
			HOST.store(PAD, Ordering::Relaxed);
			PAD_PULSE.store(true, Ordering::Relaxed);
		}
		*prev = p;
	}
	(*buttons, *sticks) = merge((*buttons, *sticks), &p);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn r2_is_the_aim_only_in_gameplay() {
		assert!(r2_is_aim(true, true));
		assert!(!r2_is_aim(false, true), "the pause menu's L1 + R2 code scanner needs R2");
		assert!(!r2_is_aim(true, false) && !r2_is_aim(false, false));
	}

	#[test]
	fn r2_goes_to_retail_for_clank_and_the_hologuise() {
		assert!(r2_stripped(true, true), "Ratchet: R2 is the aim, not the game's crouch");
		assert!(!r2_stripped(true, false), "Clank / Giant Clank / Hologuise: no aim exists, retail keeps R2");
		assert!(!r2_stripped(false, true) && !r2_stripped(false, false));
	}

	#[test]
	fn the_pad_r2_has_one_meaning_per_mode_and_body() {
		// (pad_r2, gameplay, ratchet, aim_button) -> (aim down, retail has R2)
		assert_eq!(r2_route(true, true, true, true), (true, false), "Ratchet playing: aim, not the crouch");
		assert_eq!(r2_route(true, true, false, true), (false, true), "Clank / Giant Clank / Hologuise: retail's R2, no modcam aim");
		assert_eq!(r2_route(true, false, true, true), (false, true), "the pause menu: the code scanner gets R2, nothing aims");
		assert_eq!(r2_route(true, true, true, false), (false, true), "no aim button (Classic, Strafe aiming Off): retail's R2");
		assert_eq!(r2_route(false, true, true, true), (false, false), "not held: nothing");
		assert_eq!(r2_route(false, false, false, false), (false, false));
	}

	/// The field's buttons through `apply_r2`: R2 is retail's in the pause (mode 3), none of it in gameplay as Ratchet, and the
	/// keyboard's B key is never taken. Leaving the pause with R2 held gives retail no crouch (gameplay at once), entering it keeps R2.
	#[test]
	fn apply_r2_takes_only_the_pads_r2_and_only_when_the_aim_has_it() {
		let go = |held: u8, b: u16, gameplay: bool, ratchet: bool, aim_button: bool| {
			let mut b = b;
			let down = apply_r2(&mut b, held, gameplay, ratchet, aim_button);
			(b, down)
		};
		assert_eq!(go(1, 0x12, true, true, true), (0x10, true), "gameplay as Ratchet: R2 is the aim's");
		assert_eq!(go(1, 0x12, false, true, true), (0x12, false), "the pause menu: retail gets R2 (L1 + R2 scanner)");
		assert_eq!(go(1, 0x12, true, false, true), (0x12, false), "Clank: retail gets R2, modcam's aim stays up");
		assert_eq!(go(1, 0x12, true, true, false), (0x12, false), "no aim button: retail's R2");
		assert_eq!(go(3, 0x02, true, true, true), (0x02, true), "the keyboard's B key (R2) is not the pad's and stays");
		assert_eq!(go(0, 0x02, true, true, true), (0x02, false), "B alone: untouched, no pad aim");
		assert_eq!(go(0, 0x00, true, true, true), (0x00, false));
	}

	const KBD: Ctx = Ctx { modern_camera: true, menu: None };

	/// The glyph hook displaces no other hand: no registered hand shares an entry with a wrapper twin (it declines, retail draws).
	#[test]
	fn glyph_hook_displaces_no_hand() {
		ee_game::register();
		ee_oracle::rec::register_hand(crate::retail::disc::hands(&GLYPH_HANDS));
		let ours = glyph_draw_hook as fn(&mut Cpu) -> bool as usize;
		for g in &GLYPH_HANDS {
			for h in ee_oracle::rec::hands() {
				assert!(h.entry != g.entry || h.f as usize == ours, "another hand sits on the wrapper twin {:x}", g.entry);
			}
		}
	}

	#[test]
	fn glyphs_become_key_names() {
		assert_eq!(substitute(b"\x10 Select", KBD).unwrap(), b"Space Select");
		assert_eq!(substitute(b"press \x13 three times", KBD).unwrap(), b"press F three times");
		assert_eq!(substitute(b"\x14/\x15 to view", KBD).unwrap(), b"C/Ctrl to view");
		assert_eq!(substitute(b"\x18 zooms", KBD).unwrap(), b"Mouse zooms");
		assert_eq!(substitute(b"\x18 zooms", Ctx { modern_camera: false, menu: None }).unwrap(), b"Q/E/Z/X zooms");
		assert_eq!(substitute(b"\x11 fires", KBD).unwrap(), b"LMB fires");
		assert_eq!(substitute(b"\x11 Back", KBD).unwrap(), b"LMB Back");
		// Legend lines too long for the box with "Space" use "Spc"; the others and sentences keep "Space".
		assert_eq!(substitute(b"\x10 Select file", KBD).unwrap(), b"Spc Select file");
		assert_eq!(substitute(b"\x10 Selecci\x6fn", KBD).unwrap(), b"Spc Seleccion");
		assert_eq!(substitute(b"\x10 W\x84hlen", KBD).unwrap(), b"Space W\x84hlen");
		assert_eq!(substitute(b"\x10 S\x82lect.", KBD).unwrap(), b"Space S\x82lect.");
		assert_eq!(substitute(b"press \x10 to jump over the wall", KBD).unwrap(), b"press Space to jump over the wall");
		assert_eq!(substitute(b"\x10 to jump over the wall and keep going", KBD).unwrap(), b"Space to jump over the wall and keep going");
		// Highlight / paragraph codes and plain text are not glyphs.
		assert!(substitute(b"The \x0cBlaster\x08\x01\x01 is", KBD).is_none());
	}

	/// Menu legends name the keys of the screen's table (menu_keys.rs): the Quit Game answers, the default page legends,
	/// the map; help sentences keep the play keys; every result fits the legend box.
	#[test]
	fn menu_lines_name_the_menu_keys() {
		use super::super::menu_keys::{screen, Screen, DEFAULT, MAP, QUIT_GAME};
		let page = |id| super::super::mouse_menu::MenuSnap { id, ..Default::default() };
		let ctx = |s: Screen| Ctx { modern_camera: true, menu: Some(s) };
		let quit = ctx(screen(&page(QUIT_GAME)));
		assert_eq!(substitute_text(b"\x11 Yes", quit, 0x524f).unwrap(), b"Enter Yes");
		assert_eq!(substitute_text(b"\x12 No", quit, 0x524b).unwrap(), b"Esc No");
		let opt = ctx(DEFAULT);
		assert_eq!(substitute_text(b"\x10 Select", opt, 0x4f05).unwrap(), b"Enter Select");
		assert_eq!(substitute_text(b"\x10 Equip", opt, 0x4eec).unwrap(), b"Enter Equip");
		assert_eq!(substitute_text(b"\x12 Exit", opt, 0x4ee0).unwrap(), b"Esc Exit");
		// Circle with no role on the screen: G (in a menu the left mouse button clicks, it fires only in play).
		assert_eq!(substitute_text(b"\x11 Back", opt, 0).unwrap(), b"G Back");
		// The front end's message box (menu_keys::front_dialog): its Yes / No hints name Enter / Esc, centred long names.
		let card = ctx(super::super::menu_keys::front_dialog(0x524f, 0x524b, 0).unwrap());
		assert_eq!(substitute_text(b"\x11 Yes", card, 0x524f).unwrap(), b"Enter Yes");
		assert_eq!(substitute_text(b"\x12 No", card, 0x524b).unwrap(), b"Esc No");
		let cont = ctx(super::super::menu_keys::front_dialog(0, 0, 0x524a).unwrap());
		assert_eq!(substitute_text(b"\x10 Continue", cont, 0x524a).unwrap(), b"Enter Continue");
		assert_eq!(substitute_text(b"\x10 Select file", opt, 0x4f27).unwrap(), b"Ent Select file");
		for l in [&b"\x10 Seleccionar"[..], b"\x10 Choix fich.", b"\x10 Seleccion", b"\x10 Seleziona", b"\x10 W\x84hlen"] {
			let o = substitute_text(l, opt, 0x4f27).unwrap();
			assert!(text_px(&o) <= 130, "{:?}", String::from_utf8_lossy(&o));
		}
		// 130 px is the widest legend line seen whole ("Sp Seleccionar", cold boot shots); "Ent Umschalten" (135) clips.
		assert_eq!(substitute_text(b"\x10 Umschalten", opt, 0x4edf).unwrap(), b"Sp Umschalten");
		assert_eq!(substitute_text(b"\x10 Toggle", opt, 0x4edf).unwrap(), b"Enter Toggle");
		// Mode-4 dialogs (menu_keys::dialog): centred lines, long names.
		use super::super::menu_keys::dialog;
		assert_eq!(substitute_text(b"\x10 Quit", ctx(dialog(0)), 0x5248).unwrap(), b"Enter Quit");
		assert_eq!(substitute_text(b"\x11 Continue", ctx(dialog(0)), 0x5249).unwrap(), b"Esc Continue");
		assert_eq!(substitute_text(b"\x12 Exit", ctx(dialog(1)), 0x4ee0).unwrap(), b"Enter Exit");
		assert_eq!(substitute_text(b"\x10 Continue", ctx(dialog(1)), 0x524a).unwrap(), b"Esc Continue");
		assert_eq!(substitute_text(b"\x10 Continue", ctx(dialog(2)), 0x524a).unwrap(), b"Enter Continue");
		let map = ctx(screen(&page(MAP)));
		assert_eq!(substitute_text(b"\x14/\x15 to view other planets", map, 0x4f55).unwrap(), b"Q/E to view other planets");
		assert_eq!(substitute_text(b"\x18 zooms", map, 0x4f56).unwrap(), b"Z/X zooms");
		assert_eq!(substitute_text(b"\x19 tracks", map, 0x4f57).unwrap(), b"WASD tracks");
		assert_eq!(substitute_text(b"\x11 Play Infobot", map, 0x4eef).unwrap(), b"Tab Play Infobot");
		// A help sentence inside the menu describes play: the play keys.
		assert_eq!(substitute_text(b"press \x13 three times in a row to strike", opt, 0x4f0d).unwrap(), b"press F three times in a row to strike");
		// Outside a menu nothing changes.
		assert_eq!(substitute_text(b"\x12 Exit", KBD, 0x4ee0).unwrap(), b"T Exit");
	}

	/// A remapped function names its new key in the prompts; the others keep their names; the sticks name their four keys; the
	/// menu names (fixed keys) do not change.
	#[test]
	fn remapped_keys_name_the_new_key() {
		use super::super::binds;
		binds::init(Some("cross:KeyK,l1:Digit5,forward:KeyI,strafe_left:KeyJ,back:KeyK2,strafe_right:KeyL"), None);
		assert_eq!(substitute(b" Select", KBD).unwrap(), b"K Select", "no Spc for a remapped Cross");
		assert_eq!(substitute(b"press  now", KBD).unwrap(), b"press 5 now");
		assert_eq!(substitute(b"press  now", KBD).unwrap(), b"press F now");
		assert_eq!(substitute(b" moves", KBD).unwrap(), b"I/J/S/L moves", "forward, left, back (not bound: default S), right");
		assert_eq!(substitute(b" zooms", KBD).unwrap(), b"Mouse zooms");
		assert_eq!(substitute(b" zooms", Ctx { modern_camera: false, menu: None }).unwrap(), b"Q/E/Z/X zooms");
		binds::init(None, None);
		assert_eq!(substitute(b" Select", KBD).unwrap(), b"Space Select");
		assert_eq!(substitute(b" moves", KBD).unwrap(), b"WASD moves");
	}

	#[test]
	fn every_glyph_has_a_name_and_nothing_else_does() {
		for c in 0u8..=0xff {
			assert_eq!(key_name(c, KBD).is_some(), (CROSS..=LSTICK).contains(&c), "{c:#x}");
		}
	}

	/// The names are the keyboard's real bindings: the key of each entry sets the entry's pad bit in `pad_from_keys`; the sticks
	/// move the right axes; the mouse buttons are the `circle` / `triangle` arguments.
	#[test]
	fn names_follow_pad_from_keys() {
		use crate::retail::pad_from_keys;
		for b in BINDINGS.iter().filter(|b| b.bit != 0) {
			for &k in b.keys {
				assert_eq!(pad_from_keys(|x| x == k, false, false).0 & b.bit, b.bit, "{} / {k:?}", b.name);
			}
		}
		// Left stick: A D move lx (2), W S move ly (3); right stick: Q E move rx (0), Z X move ry (1).
		for (k, axis) in [(KeyCode::KeyA, 2), (KeyCode::KeyD, 2), (KeyCode::KeyW, 3), (KeyCode::KeyS, 3), (KeyCode::KeyQ, 0), (KeyCode::KeyE, 0), (KeyCode::KeyZ, 1), (KeyCode::KeyX, 1)] {
			assert_ne!(pad_from_keys(|x| x == k, false, false).1[axis], 0x80, "{k:?}");
			let name = if axis >= 2 { "WASD" } else { "Q/E/Z/X" };
			assert!(BINDINGS.iter().any(|b| b.name == name && b.keys.contains(&k)), "{k:?}");
		}
		// Mouse: left = Circle, middle = Triangle; both also stay on their keys (G, T).
		assert_eq!(pad_from_keys(|_| false, true, false).0, 0x20);
		assert_eq!(pad_from_keys(|_| false, false, true).0, 0x10);
		// Every glyph code is covered exactly once.
		for c in CROSS..=LSTICK {
			assert_eq!(BINDINGS.iter().filter(|b| b.code == c).count(), 1, "{c:#x}");
		}
	}

	#[test]
	fn mouse_movement_must_be_clear() {
		let mut m = MoveSum::default();
		assert!(!m.add(1000, 3.0), "a twitch");
		assert!(!m.add(1100, 3.0));
		assert!(!m.add(2000, 30.0), "a pause restarts the sum");
		assert!(m.add(2100, 15.0), "40 px within a short time");
		assert!(!m.add(2150, 1.0), "reset after it fired");
	}
}
