//! Input recorder and replayer for retail mode: `RAC_RETAIL_REC=file` writes every field's pad exactly as the retail
//! worker hands it to the game, `RAC_RETAIL_REPLAY=file` drives retail mode from such a file instead of the keyboard.
//!
//! File format "racpad 1" (UTF-8, LF, one record per line, tab separated):
//! - line 1: `# racpad 1 state=DIR key=value ...` (spaces inside a value are written as `%20`);
//! - `FIELD<TAB>BUTTONS<TAB>LX<TAB>LY<TAB>RX<TAB>RY`: the pad of one field. FIELD counts fields since the savestate
//!   load, from 0, one line per field, as fndiff / movetest count them (set_pad, then one `Machine::frame`). BUTTONS
//!   is the 16-bit active-high mask (4 hex digits; Select 0x100, Start 0x800, d-pad 0x1000 up 0x2000 right 0x4000 down
//!   0x8000 left, L2 1, R2 2, L1 4, R1 8, Triangle 0x10, Circle 0x20, Cross 0x40, Square 0x80). Sticks are the report
//!   bytes in decimal, 0..255, 128 = centred;
//!   Modern-camera runs add columns after RY, only when nonzero: 5 more (mouse x, mouse y, right mouse button, wheel, F7 toggle;
//!   11 columns in all) and, since 2026-10-23, 2 more (13 columns: the camera keys' share of the right stick, Q/E and Z/X, offset
//!   from 0x80; modcam_turn.rs `camera_sticks` needs it to tell the keys from the controller's stick). Readers take 6, 11 or 13;
//!   an older file without the key columns replays Q/E/Z/X as the physical stick (the view then turns the other way);
//! - `FIELD<TAB>!EVENT<TAB>ARGS`: anything that is not a pad read but changes the run. It applies before that field's
//!   pad is set. `!TRAVEL<TAB>LEVEL` pokes the level-travel request (0x15f5a0 = level, 0x15f5b8 = 1, 0x15f550 = 1);
//!   `0<TAB>!LOAD<TAB>DIR` starts a new segment: the machine is reloaded from savestate DIR and FIELD restarts at 0
//!   (the first segment's load is the header's `state=`); `!INFO<TAB>text` is a note readers ignore (level or game mode
//!   changed);
//! - `#` lines and empty lines are ignored.

use super::FieldIn;
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

struct Recorder {
	pad: std::fs::File,
	/// `RAC_RETAIL_REC_TRAIL=N`: every N fields a `trail FIELD HASH` line (movetest's RAM hash) in `<file>.trail`.
	trail: Option<(std::fs::File, u32)>,
	loads: u32,
	failed: bool,
}

static REC: Mutex<Option<Recorder>> = Mutex::new(None);

fn put(f: &mut std::fs::File, failed: &mut bool, s: &str) {
	if let Err(e) = f.write_all(s.as_bytes()) {
		if !*failed {
			eprintln!("retail: recording failed: {e}");
		}
		*failed = true;
	}
}

fn esc(s: &str) -> String {
	s.replace(' ', "%20")
}

/// Opens the recorder when RAC_RETAIL_REC is set (a failure is an error: a playthrough must not run unrecorded).
pub fn open_from_env() -> Result<(), String> {
	let Ok(path) = std::env::var("RAC_RETAIL_REC") else { return Ok(()) };
	if std::env::var("RAC_RETAIL_REPLAY").is_ok_and(|r| r == path) {
		return Err("RAC_RETAIL_REC and RAC_RETAIL_REPLAY name the same file".into());
	}
	let pad = std::fs::File::create(&path).map_err(|e| format!("RAC_RETAIL_REC {path}: {e}"))?;
	let trail = match std::env::var("RAC_RETAIL_REC_TRAIL").ok().and_then(|v| v.parse::<u32>().ok()).filter(|&n| n > 0) {
		Some(n) => Some((std::fs::File::create(format!("{path}.trail")).map_err(|e| format!("RAC_RETAIL_REC {path}.trail: {e}"))?, n)),
		None => None,
	};
	eprintln!("retail: recording the pad to {path}");
	if let Ok(mut r) = REC.lock() {
		*r = Some(Recorder { pad, trail, loads: 0, failed: false });
	}
	Ok(())
}

pub fn recording() -> bool {
	REC.lock().is_ok_and(|r| r.is_some())
}

/// A savestate was loaded (`Retail::load`): the header for the first one, a `!LOAD` segment start for later ones.
pub fn begin(state: &str, level: u32, rec: usize) {
	let Ok(mut g) = REC.lock() else { return };
	let Some(r) = g.as_mut() else { return };
	let var = |k: &str| std::env::var(k).map_or("-".to_string(), |v| esc(&v));
	let s = if r.loads == 0 {
		format!(
			"# racpad 1 state={} level={level} rec={rec} oracle_rec={} oracle_hand={} hand_range={} hand_skip={} iso={}\n# FIELD\tBUTTONS_HEX\tLX\tLY\tRX\tRY | FIELD\t!EVENT\tARGS\n",
			esc(state),
			var("ORACLE_REC"),
			var("ORACLE_HAND"),
			var("ORACLE_HAND_RANGE"),
			var("ORACLE_HAND_SKIP"),
			var("ORACLE_ISO")
		)
	} else {
		format!("0\t!LOAD\t{}\n", esc(state))
	};
	r.loads += 1;
	put(&mut r.pad, &mut r.failed, &s);
	if let Some((f, n)) = r.trail.as_mut() {
		let s = format!("# trail every {n} fields: RAM 0x80000..0x1f00000 after the field's events and pad, before its frame\ninfo load {state} level {level} rec {rec}\n");
		let _ = f.write_all(s.as_bytes());
	}
}

/// The field about to run: its events, then its pad line (one write, so a crash loses at most the unsent field).
pub fn field(n: u32, f: &FieldIn) {
	let Ok(mut g) = REC.lock() else { return };
	let Some(r) = g.as_mut() else { return };
	let mut s = String::new();
	if let Some(to) = f.travel {
		s += &format!("{n}\t!TRAVEL\t{to}\n");
	}
	s += &format!("{n}\t{:04x}\t{}\t{}\t{}\t{}", f.buttons, f.sticks[2], f.sticks[3], f.sticks[0], f.sticks[1]);
	// Modern camera input (retail/modcam.rs): five extra columns only when nonzero (mouse x, y, RMB, wheel, F7).
	if !f.mc.is_zero() {
		s += &format!("\t{}\t{}\t{}\t{}\t{}", f.mc.look[0], f.mc.look[1], f.mc.aim as u8, f.mc.zoom, f.mc.toggle as u8);
		// The camera keys' share of the right stick (modcam_turn.rs): two more columns only when a key is held.
		if f.mc.keys != [0; 2] {
			s += &format!("\t{}\t{}", f.mc.keys[0], f.mc.keys[1]);
		}
	}
	s.push('\n');
	put(&mut r.pad, &mut r.failed, &s);
}

/// The level or game mode changed after the previous field (`n` = the next field): an `!INFO` note.
pub fn note(n: u32, level: u32, mode: u32) {
	let Ok(mut g) = REC.lock() else { return };
	let Some(r) = g.as_mut() else { return };
	put(&mut r.pad, &mut r.failed, &format!("{n}\t!INFO\tlevel={level} mode={mode}\n"));
}

/// Whether `trail` wants the RAM of field `n`.
pub fn trail_due(n: u32) -> bool {
	REC.lock().is_ok_and(|r| r.as_ref().and_then(|r| r.trail.as_ref()).is_some_and(|(_, k)| n % k == 0))
}

/// movetest's trail hash: RAM 0x80000..0x1f00000 as little-endian u64 words (no kernel, no main stack).
pub fn hash_ram(ram: &[u8]) -> u64 {
	let r = &ram[0x80000..0x1f00000.min(ram.len())];
	r.chunks_exact(8).fold(0xcbf2_9ce4_8422_2325u64, |h, w| (h ^ u64::from_le_bytes(w.try_into().unwrap())).wrapping_mul(0x0100_0000_01b3).rotate_left(23))
}

pub fn trail(n: u32, ram: &[u8]) {
	let h = hash_ram(ram);
	let Ok(mut g) = REC.lock() else { return };
	if let Some((f, _)) = g.as_mut().and_then(|r| r.trail.as_mut()) {
		let _ = f.write_all(format!("trail {n} {h:016x}\n").as_bytes());
	}
}

/// A parsed pad file: the savestate it starts from and the first segment's fields.
pub struct Replay {
	pub state: String,
	pub fields: Vec<FieldIn>,
}

static REPLAY: OnceLock<Arc<Replay>> = OnceLock::new();

/// Reads RAC_RETAIL_REPLAY when set.
pub fn replay_from_env() -> Result<(), String> {
	let Ok(path) = std::env::var("RAC_RETAIL_REPLAY") else { return Ok(()) };
	let text = std::fs::read_to_string(&path).map_err(|e| format!("RAC_RETAIL_REPLAY {path}: {e}"))?;
	let r = parse(&text).map_err(|e| format!("RAC_RETAIL_REPLAY {path}: {e}"))?;
	eprintln!("retail: replaying {} fields of {path} (state {})", r.fields.len(), r.state);
	let _ = REPLAY.set(Arc::new(r));
	Ok(())
}

pub fn replay() -> Option<Arc<Replay>> {
	REPLAY.get().cloned()
}

pub fn parse(text: &str) -> Result<Replay, String> {
	let mut lines = text.lines();
	let head = lines.next().unwrap_or("");
	let mut it = head.split_whitespace();
	if (it.next(), it.next(), it.next()) != (Some("#"), Some("racpad"), Some("1")) {
		return Err(format!("not a racpad 1 file: {head:?}"));
	}
	let state = it.find_map(|t| t.strip_prefix("state=")).map(|s| s.replace("%20", " ")).ok_or("header has no state=")?;
	let mut fields: Vec<FieldIn> = Vec::new();
	let mut travel: Option<u32> = None;
	for (i, line) in lines.enumerate() {
		let at = |m: &str| format!("line {}: {m}: {line:?}", i + 2);
		if line.is_empty() || line.starts_with('#') {
			continue;
		}
		let f: Vec<&str> = line.split('\t').collect();
		let n: usize = f[0].parse().map_err(|_| at("bad field number"))?;
		if let Some(e) = f.get(1).filter(|e| e.starts_with('!')) {
			match *e {
				"!TRAVEL" => travel = Some(f.get(2).and_then(|x| x.parse().ok()).ok_or_else(|| at("bad travel level"))?),
				// A second segment would restart the machine mid-file; rac-rs reloads only when the app does.
				"!LOAD" => return Err(at("a !LOAD segment (replay it with racreplay)")),
				"!INFO" => {}
				_ => return Err(at("unknown event")),
			}
			continue;
		}
		if (f.len() != 6 && f.len() != 11 && f.len() != 13) || n != fields.len() {
			return Err(at(&format!("want field {} with 6 (or 11, 13) columns", fields.len())));
		}
		let b = u16::from_str_radix(f[1].trim_start_matches("0x"), 16).map_err(|_| at("bad buttons"))?;
		let s = |k: usize| f[k].parse::<u8>().map_err(|_| at("bad stick"));
		let num = |k: usize| f.get(k).and_then(|x| x.parse::<i32>().ok()).unwrap_or(0);
		let mc = super::modcam::Input { look: [num(6) as i16, num(7) as i16], aim: num(8) != 0, zoom: num(9) as i8, toggle: num(10) != 0, keys: [num(11) as i8, num(12) as i8] };
		fields.push(FieldIn { buttons: b, sticks: [s(4)?, s(5)?, s(2)?, s(3)?], travel: travel.take(), mc, omni: Default::default() });
	}
	if travel.is_some() {
		return Err("a !TRAVEL after the last field".into());
	}
	Ok(Replay { state, fields })
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_roundtrip() {
		let t = "# racpad 1 state=x/a%20b/s07 level=4\n0\t0000\t128\t128\t128\t128\n1\t!INFO\tlevel=4 mode=3\n1\t!TRAVEL\t5\n1\t0800\t10\t20\t30\t40\n# c\n\n2\t0040\t0\t255\t128\t128\n";
		let r = parse(t).unwrap();
		assert_eq!(r.state, "x/a b/s07");
		assert_eq!(r.fields.len(), 3);
		assert_eq!(r.fields[1], FieldIn { buttons: 0x800, sticks: [30, 40, 10, 20], travel: Some(5), mc: Default::default(), omni: Default::default() });
		assert_eq!(r.fields[2].sticks, [128, 128, 0, 255]);
		assert!(parse("0\t0000\t1\t1\t1\t1\n").is_err());
		assert!(parse("# racpad 1 state=x\n1\t0000\t1\t1\t1\t1\n").is_err());
		assert!(parse("# racpad 1 state=x\n0\t0000\t1\t1\t1\t999\n").is_err());
	}
}
