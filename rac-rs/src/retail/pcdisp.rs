//! The Display section of the pause Graphics page (Pause -> Options -> Graphics -> Display, v0.4.1, GitHub #33 and player requests),
//! retail side: four rows that change how the finished picture is shown, never what the game computes.
//!
//! - HUD position: `4:3` (default, the HUD stays where retail has it, in the centred 4:3 picture) or `Corners` (the two top HUD boxes,
//!   nanotech and bolts / ammo, go to the window's corners on wide screens, gameplay only; viewer/handler/hud_place.rs).
//! - HUD scale: 75 .. 150 in steps of 5 (default 100; the top boxes are drawn that much larger or smaller; hud_place.rs).
//! - FPS limit: `Display` (default: the monitor's refresh rate, at least 50, as before), 30, 60, 120, 144 or `Off` (viewer/pace.rs
//!   reads `fps_setting` at every frame, so a change applies at once; `RAC_FPS_CAP` still wins).
//! - Render scale: 50, 75, 100 (default), 125, 150 or 200 percent of the window size (the picture is drawn at that size and scaled to
//!   the window). This file only keeps the choice (`render_scale_pct`); the renderer that uses it is the render-scale lane's.
//!
//! The rows are built like the Mouse speed row (pcext/mousespeed.rs): HUD position is a plain two-choice row (byte 0 / 1); the other
//! three are "steppers": a record with two EQUAL choice ids (a dynamic text "< 60 >") whose byte retail toggles on Cross, which the
//! host reads as one step on; Left / Right (`pcctl::field`'s auto-repeated `dir` pair, also the mouse wheel) step back / on while the
//! page's cursor is on that row. Steps wrap. The values live in atomics (the options file's values until the player changes a row),
//! `sync` (worker, every field) writes a change to the options file as keys `hudpos=corners`, `hudscale=N`, `fps=N|off`,
//! `renderscale=N` (pcopts.rs `extra`; a default value removes its key). The row bytes are at `CBASE + 0x17 ..= 0x1a` (pcctl's
//! block, default 0 = today's RAM), the texts are the ids 0x7fb0..0x7fbf in a pool of their own (`POOL`, BASE + 0xE000, 16 slots of
//! 0x40; the canary of prompts.rs skips it). `%` is not a font glyph (0x25 draws the Cross button), so values carry no unit.
//!
//! The Display section is the pause page's and the title's Graphics page (titlegfx.rs) alike.
use super::pcctl::CBASE;
use super::pcopts::PcOptions;
use ee_oracle::cpu::Cpu;
use ee_oracle::mem::Mem;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};

/// The HUD position row's byte (0 = 4:3, 1 = Corners) and the three steppers' toggle bytes.
pub(super) const POS_B: u32 = CBASE + 0x17;
const SCALE_B: u32 = CBASE + 0x18;
const FPS_B: u32 = CBASE + 0x19;
const RS_B: u32 = CBASE + 0x1a;
/// Rows of the Display section in the pause page's table (row 0 is the section stepper).
const SCALE_ROW: u32 = 2;
const FPS_ROW: u32 = 3;
const RS_ROW: u32 = 4;

/// The text pool: ids `ID0..ID_END`, one 0x40-byte slot each, BASE + 0xE000 .. + 0xE400.
pub const POOL: u32 = super::pcext::BASE + 0xE000;
pub const RANGE: std::ops::Range<u32> = POOL..POOL + 0x400;
const SLOT: u32 = 0x40;
pub const ID0: u32 = 0x7fb0;
pub const ID_END: u32 = 0x7fc0;
/// Labels of the four rows, then the two HUD position values and the three stepper values.
pub(super) const ID_HUDPOS: u32 = 0x7fb0;
pub(super) const ID_HUDSCALE: u32 = 0x7fb1;
pub(super) const ID_FPS: u32 = 0x7fb2;
pub(super) const ID_RSCALE: u32 = 0x7fb3;
pub(super) const ID_P43: u32 = 0x7fb4;
pub(super) const ID_CORNERS: u32 = 0x7fb5;
pub(super) const ID_VSCALE: u32 = 0x7fb6;
pub(super) const ID_VFPS: u32 = 0x7fb7;
pub(super) const ID_VRS: u32 = 0x7fb8;

/// The HUD scale range in percent (default 100).
pub const HUD_MIN: u8 = 75;
pub const HUD_MAX: u8 = 150;
pub const HUD_STEP: u8 = 5;
/// FPS limit values: 0 = the display's refresh rate (today's default), `u32::MAX` = Off (uncapped).
pub const FPS_LIST: [u32; 6] = [0, 30, 60, 120, 144, u32::MAX];
/// Render scale values in percent.
pub const RS_LIST: [u8; 6] = [50, 75, 100, 125, 150, 200];

static POS: AtomicU8 = AtomicU8::new(0);
static SCALE: AtomicU8 = AtomicU8::new(100);
static FPS: AtomicU32 = AtomicU32::new(0);
static RSCALE: AtomicU8 = AtomicU8::new(100);
/// A stepper changed since `sync` looked (the options file needs the new value).
static DIRTY: AtomicU8 = AtomicU8::new(0);
/// The steppers' toggle bytes as last seen (0xff: not known yet, the next look is the baseline). Per thread: the worker thread
/// builds the page and reads the bytes (and each test has its own).
thread_local! { static SEEN: std::cell::Cell<[u8; 3]> = const { std::cell::Cell::new([0xff; 3]) }; }
/// The monitor's refresh rate in Hz as the pacer last saw it (0 = unknown) and the window's size in pixels (0 = unknown).
static REFRESH: AtomicU32 = AtomicU32::new(0);
static WIN_W: AtomicU32 = AtomicU32::new(0);
static WIN_H: AtomicU32 = AtomicU32::new(0);

// ---- what the rest of the game reads ---------------------------------------------------------------------------------------

/// HUD position: true = Corners.
pub fn hud_corners() -> bool {
	POS.load(Ordering::Relaxed) != 0
}

/// HUD scale in percent (75..=150, 100 = today).
pub fn hud_scale_pct() -> u32 {
	SCALE.load(Ordering::Relaxed) as u32
}

/// FPS limit: 0 = the display's refresh rate (today's default), `u32::MAX` = Off, else the limit in Hz.
pub fn fps_setting() -> u32 {
	FPS.load(Ordering::Relaxed)
}

/// Render scale in percent (50, 75, 100, 125, 150, 200; 100 = today's picture).
pub fn render_scale_pct() -> u32 {
	RSCALE.load(Ordering::Relaxed) as u32
}

/// The pacer saw the monitor's refresh rate (shown in the FPS limit row).
pub fn note_refresh(hz: u32) {
	REFRESH.store(hz, Ordering::Relaxed);
}

/// The window's size in pixels (shown in the Render scale row: "< 50 - 960x540 >").
pub fn note_window_size(w: u32, h: u32) {
	WIN_W.store(w, Ordering::Relaxed);
	WIN_H.store(h, Ordering::Relaxed);
}

// ---- values -------------------------------------------------------------------------------------------------------------------

/// A HUD scale from the file: the nearest step of 5 inside 75..=150.
pub fn snap_hud(v: u32) -> u8 {
	let v = v.clamp(HUD_MIN as u32, HUD_MAX as u32);
	((v + HUD_STEP as u32 / 2) / HUD_STEP as u32 * HUD_STEP as u32).clamp(HUD_MIN as u32, HUD_MAX as u32) as u8
}

/// The FPS limit of a file value: `display`, `off` or a number (the nearest listed limit); anything else = the display's rate.
fn parse_fps(s: &str) -> u32 {
	match s.trim().to_ascii_lowercase().as_str() {
		"off" => u32::MAX,
		t => t.parse::<u32>().ok().filter(|&n| n > 0).map_or(0, |n| FPS_LIST[1..5].iter().copied().min_by_key(|&l| l.abs_diff(n)).unwrap_or(0)),
	}
}

fn fps_text(v: u32) -> String {
	match v {
		0 => "display".into(),
		u32::MAX => "off".into(),
		n => n.to_string(),
	}
}

/// A render scale from the file: the nearest listed value, 100 when it is not a number.
fn parse_rscale(s: &str) -> u8 {
	s.trim().trim_end_matches('%').parse::<u32>().map_or(100, |n| RS_LIST.iter().copied().min_by_key(|&l| (l as u32).abs_diff(n)).unwrap_or(100))
}

fn index_of<T: PartialEq + Copy>(list: &[T], v: T) -> usize {
	list.iter().position(|&x| x == v).unwrap_or(0)
}

/// One step on a wrapping list of `n` values.
fn step_index(i: usize, n: usize, back: bool) -> usize {
	if back {
		(i + n - 1) % n
	} else {
		(i + 1) % n
	}
}

/// The value after one step: HUD scale (`k` 0), FPS limit (1) or Render scale (2); `back` = Left.
fn stepped(k: usize, back: bool) -> u32 {
	match k {
		0 => {
			let n = ((HUD_MAX - HUD_MIN) / HUD_STEP) as usize + 1;
			let i = (SCALE.load(Ordering::Relaxed).saturating_sub(HUD_MIN) / HUD_STEP) as usize;
			(HUD_MIN as usize + step_index(i.min(n - 1), n, back) * HUD_STEP as usize) as u32
		}
		1 => FPS_LIST[step_index(index_of(&FPS_LIST, FPS.load(Ordering::Relaxed)), FPS_LIST.len(), back)],
		_ => RS_LIST[step_index(index_of(&RS_LIST, RSCALE.load(Ordering::Relaxed)), RS_LIST.len(), back)] as u32,
	}
}

// ---- options file and page --------------------------------------------------------------------------------------------------

/// `pcext::init`: the file's values are the rows' start values.
pub fn init(opts: &PcOptions) {
	POS.store(opts.extra_get("hudpos").is_some_and(|v| v.trim().eq_ignore_ascii_case("corners")) as u8, Ordering::Relaxed);
	SCALE.store(opts.extra_get("hudscale").and_then(|v| v.trim().parse().ok()).map_or(100, snap_hud), Ordering::Relaxed);
	FPS.store(opts.extra_get("fps").map_or(0, parse_fps), Ordering::Relaxed);
	RSCALE.store(opts.extra_get("renderscale").map_or(100, parse_rscale), Ordering::Relaxed);
	DIRTY.store(0, Ordering::Relaxed);
	reset_seen();
}

fn reset_seen() {
	SEEN.with(|s| s.set([0xff; 3]));
}

/// `pcctl::build_gfx`, on RAM that never held the page: the position row's byte takes the start value, the steppers' bytes start at 0.
pub(super) fn build(mem: &mut Mem) {
	mem.wr8(POS_B, POS.load(Ordering::Relaxed));
	for b in [SCALE_B, FPS_B, RS_B] {
		mem.wr8(b, 0);
	}
	reset_seen();
}

/// The cursor's row on the Graphics page, when it is open and idle.
fn cursor_row(mem: &mut Mem, level: u32) -> Option<u32> {
	let row = std::cell::Cell::new(None);
	super::pcctl::open_page(mem, level, super::pcext::GPAGE, |rd, snap| {
		let ok = snap.id == 0x1d && snap.state == 0x1d && snap.cur == super::pcext::GVAL;
		if ok {
			row.set(Some(rd(super::pcext::GVAL + 0x38)));
		}
		ok
	});
	row.get()
}

/// Worker, each field while the Graphics page shows the Display section (`pcctl::field`): the steppers' steps. `dir` is Left / Right
/// this field (auto-repeated).
pub(super) fn field(mem: &mut Mem, level: u32, dir: [bool; 2]) {
	let mut cursor = None;
	for (k, (addr, row)) in [(SCALE_B, SCALE_ROW), (FPS_B, FPS_ROW), (RS_B, RS_ROW)].into_iter().enumerate() {
		let sel = mem.rd8(addr);
		let seen = SEEN.with(|s| {
			let mut v = s.get();
			let old = std::mem::replace(&mut v[k], sel);
			s.set(v);
			old
		});
		let cross = seen != 0xff && seen != sel;
		let held = dir[0] != dir[1];
		if !cross && !(held && *cursor.get_or_insert_with(|| cursor_row(mem, level)) == Some(row)) {
			continue;
		}
		let next = stepped(k, !cross && dir[0]);
		match k {
			0 => SCALE.store(next as u8, Ordering::Relaxed),
			1 => FPS.store(next, Ordering::Relaxed),
			_ => RSCALE.store(next as u8, Ordering::Relaxed),
		}
		DIRTY.store(1, Ordering::Relaxed);
		eprintln!("retail: PC options: {} {}", ["HUD scale", "FPS limit", "Render scale"][k], if k == 1 { fps_text(next) } else { next.to_string() });
	}
}

/// Worker, every field after the page patch: the rows' changes go into the options file (true = the caller saves).
pub fn sync(mem: &mut Mem, opts: &mut PcOptions) -> bool {
	if !super::pcctl::gfx_built(mem) {
		return false;
	}
	let pos = (mem.rd8(POS_B) != 0) as u8;
	let moved = POS.swap(pos, Ordering::Relaxed) != pos;
	if !(moved | (DIRTY.swap(0, Ordering::Relaxed) != 0)) {
		return false;
	}
	if moved {
		eprintln!("retail: PC options: HUD position {}", if pos != 0 { "Corners" } else { "4:3" });
	}
	let (scale, fps, rs) = (SCALE.load(Ordering::Relaxed), FPS.load(Ordering::Relaxed), RSCALE.load(Ordering::Relaxed));
	opts.extra_set("hudpos", (pos != 0).then(|| "corners".to_string()));
	opts.extra_set("hudscale", (scale != 100).then(|| scale.to_string()));
	opts.extra_set("fps", (fps != 0).then(|| fps_text(fps)));
	opts.extra_set("renderscale", (rs != 100).then(|| rs.to_string()));
	true
}

// ---- texts ------------------------------------------------------------------------------------------------------------------

type T5 = [&'static [u8]; 5];

#[rustfmt::skip]
static LABELS: [T5; 4] = [
	[b"HUD position", b"Position HUD", b"HUD-Position", b"Posici\x9cn del HUD", b"Posizione HUD"],
	[b"HUD scale", b"Taille HUD", b"HUD-Skalierung", b"Escala del HUD", b"Scala HUD"],
	[b"FPS limit", b"Limite FPS", b"FPS-Limit", b"L\x94mite de FPS", b"Limite FPS"],
	[b"Render scale", b"Taille du rendu", b"Render-Skalierung", b"Escala de render", b"Scala di render"],
];
static CORNERS: T5 = [b"Corners", b"Coins", b"Ecken", b"Esquinas", b"Angoli"];
/// "< Display 144Hz >": the word for "the display's own rate" per language (Fr: e-acute 0x8c), and "Off".
static DISPLAY: T5 = [b"Display", b"\x8ccran", b"Anzeige", b"Pantalla", b"Schermo"];
static OFF: T5 = [b"Off", b"Sans limite", b"Aus", b"Sin l\x94mite", b"Nessuno"];
/// The section's name on the Graphics page's stepper.
pub(super) static SECTION: T5 = [b"Display", b"Interface", b"Anzeige", b"Visualizaci\x9cn", b"Visualizzazione"];

/// The text of `id` (`ID0..ID_END`) in language `l` (0 En .. 4 It); empty for the free ids.
pub fn text(id: u32, l: usize) -> Vec<u8> {
	let l = l.min(4);
	let stepper = |s: Vec<u8>| [b"< ".as_slice(), &s, b" >"].concat();
	let word = |t: &T5, hz: u32| -> Vec<u8> { if hz == 0 { t[l].to_vec() } else { [t[l], format!(" {hz}Hz").as_bytes()].concat() } };
	match id {
		ID_HUDPOS..=ID_RSCALE => LABELS[(id - ID_HUDPOS) as usize][l].to_vec(),
		ID_P43 => b"4:3".to_vec(),
		ID_CORNERS => CORNERS[l].to_vec(),
		ID_VSCALE => stepper(SCALE.load(Ordering::Relaxed).to_string().into_bytes()),
		ID_VFPS => match FPS.load(Ordering::Relaxed) {
			0 => stepper(word(&DISPLAY, REFRESH.load(Ordering::Relaxed))),
			u32::MAX => stepper(OFF[l].to_vec()),
			n => stepper(n.to_string().into_bytes()),
		},
		ID_VRS => {
			let (pct, w, h) = (RSCALE.load(Ordering::Relaxed) as u32, WIN_W.load(Ordering::Relaxed), WIN_H.load(Ordering::Relaxed));
			let size = if w > 0 && h > 0 { format!(" - {}x{}", (w * pct + 50) / 100, (h * pct + 50) / 100) } else { String::new() };
			stepper(format!("{pct}{size}").into_bytes())
		}
		_ => Vec::new(),
	}
}

/// The text lookup hand for the ids `ID0..ID_END` (`pcctl::text_hook` hands them over): the string goes into the id's pool slot and
/// its address into v0.
pub fn text_hook(c: &mut Cpu, id: u32, lang: usize) -> bool {
	let s = text(id, lang);
	let at = POOL + SLOT * (id - ID0);
	let n = s.len().min(SLOT as usize - 1);
	for (i, b) in s[..n].iter().enumerate() {
		c.mem.wr8(at + i as u32, *b);
	}
	c.mem.wr8(at + n as u32, 0);
	c.s(2, at as u64);
	true
}

#[cfg(test)]
#[path = "pcdisp_tests.rs"]
mod tests;
