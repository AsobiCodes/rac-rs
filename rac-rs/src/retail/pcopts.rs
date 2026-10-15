//! The PC options file (`<player dir>/pc_options.txt`, next to card.bin): what the player chose in the game's own menus and
//! that the retail memory card does not hold. Plain `key=value` lines, written by the game, never typed by the player:
//!
//! ```text
//! version=1
//! lang=de          en fr de es it   (Options -> Language at the title)
//! camera=modern    modern classic   (Pause -> Options -> Camera Options -> Camera)
//! strafe=on        on off           (Pause -> Options -> Camera Options -> Strafe aiming)
//! buyall=on        on off           (Pause -> Options -> Camera Options -> Buy all ammo: Square at a Gadgetron vendor)
//! ```
//!
//! A missing file, a missing key or a value that cannot be read gives that key's default (camera modern, strafe on, language
//! not chosen yet = the console language of the cold boot). Keys this build does not know are kept when the file is rewritten.
//! The file is written to a temporary name and renamed, so a crash never leaves half a file.
use std::path::{Path, PathBuf};

/// The five languages of the PAL disc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
	En,
	Fr,
	De,
	Es,
	It,
}

impl Lang {
	pub const ALL: [Lang; 5] = [Lang::En, Lang::Fr, Lang::De, Lang::Es, Lang::It];

	pub fn code(self) -> &'static str {
		["en", "fr", "de", "es", "it"][self as usize]
	}

	pub fn from_code(s: &str) -> Option<Lang> {
		Lang::ALL.into_iter().find(|l| l.code() == s.trim().to_ascii_lowercase())
	}

	/// The PS2 console's OSD language number a cold boot reports (`ee_oracle::cold::set_language`): English 1, French 2, Spanish 3,
	/// German 4, Italian 5.
	pub fn osd(self) -> u32 {
		[1, 2, 4, 3, 5][self as usize]
	}

	/// The game's own language word (0x15ed88): English 0, French 2, German 3, Spanish 4, Italian 5 (the value Options -> Language
	/// stores; the OSD numbers map 1 -> 0, 2 -> 2, 3 -> 4, 4 -> 3, 5 -> 5).
	pub fn word(self) -> u32 {
		[0, 2, 3, 4, 5][self as usize]
	}

	pub fn from_word(w: u32) -> Option<Lang> {
		Lang::ALL.into_iter().find(|l| l.word() == w)
	}
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PcOptions {
	/// None: not chosen yet.
	pub lang: Option<Lang>,
	/// Some(true) = Modern, Some(false) = Classic; None: never changed (the default applies).
	pub camera: Option<bool>,
	/// Some(true) = On; None: never changed.
	pub strafe: Option<bool>,
	/// Some(true) = On (Square buys all ammo at a vendor); None: never changed.
	pub buyall: Option<bool>,
	/// Lines this build does not know, kept verbatim on rewrite.
	extra: Vec<(String, String)>,
}

fn flag(v: &str, yes: &str, no: &str) -> Option<bool> {
	let v = v.trim().to_ascii_lowercase();
	match v.as_str() {
		x if x == yes || x == "true" || x == "1" || x == "yes" => Some(true),
		x if x == no || x == "false" || x == "0" || x == "no" => Some(false),
		_ => None,
	}
}

impl PcOptions {
	pub const fn new() -> PcOptions {
		PcOptions { lang: None, camera: None, strafe: None, buyall: None, extra: Vec::new() }
	}

	/// Camera style with the default: Modern.
	pub fn camera_modern(&self) -> bool {
		self.camera.unwrap_or(true)
	}

	/// Buy all ammo with the default: On.
	pub fn buyall_on(&self) -> bool {
		self.buyall.unwrap_or(true)
	}

	/// Strafe aiming with the default: On.
	pub fn strafe_on(&self) -> bool {
		self.strafe.unwrap_or(true)
	}

	/// Reads the text of a file: tolerant, never fails.
	pub fn parse(text: &str) -> PcOptions {
		let mut o = PcOptions::default();
		for line in text.lines() {
			let line = line.trim_start_matches('\u{feff}').trim();
			let Some((k, v)) = line.split_once('=') else { continue };
			let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
			match k.as_str() {
				"version" => {}
				"lang" => o.lang = Lang::from_code(v),
				"camera" => o.camera = flag(v, "modern", "classic"),
				"strafe" => o.strafe = flag(v, "on", "off"),
				"buyall" => o.buyall = flag(v, "on", "off"),
				_ if !k.is_empty() && !k.starts_with('#') => {
					o.extra.retain(|(e, _)| *e != k);
					o.extra.push((k, v.to_string()));
				}
				_ => {}
			}
		}
		o
	}

	/// The file text: only the keys the player has set, plus the unknown ones.
	pub fn to_text(&self) -> String {
		let mut s = String::from("version=1\n");
		if let Some(l) = self.lang {
			s += &format!("lang={}\n", l.code());
		}
		if let Some(c) = self.camera {
			s += &format!("camera={}\n", if c { "modern" } else { "classic" });
		}
		if let Some(t) = self.strafe {
			s += &format!("strafe={}\n", if t { "on" } else { "off" });
		}
		if let Some(t) = self.buyall {
			s += &format!("buyall={}\n", if t { "on" } else { "off" });
		}
		for (k, v) in &self.extra {
			s += &format!("{k}={v}\n");
		}
		s
	}

	/// The file's options; the defaults when it is missing or unreadable.
	pub fn load(path: &Path) -> PcOptions {
		std::fs::read(path).map(|b| PcOptions::parse(&String::from_utf8_lossy(&b))).unwrap_or_default()
	}

	/// Writes the file atomically (temporary file + rename).
	pub fn store(&self, path: &Path) -> std::io::Result<()> {
		if let Some(dir) = path.parent() {
			std::fs::create_dir_all(dir)?;
		}
		let mut tmp = path.as_os_str().to_owned();
		tmp.push(".tmp");
		let tmp = PathBuf::from(tmp);
		std::fs::write(&tmp, self.to_text())?;
		std::fs::rename(&tmp, path)
	}
}

/// `<player dir>/pc_options.txt`.
pub fn file() -> PathBuf {
	super::memcard::player_dir().join("pc_options.txt")
}

#[cfg(test)]
mod tests {
	use super::*;

	fn scratch(name: &str) -> PathBuf {
		let d = std::env::temp_dir().join(format!("pcopts_test_{}_{name}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		d
	}

	#[test]
	fn missing_file_gives_the_defaults() {
		let d = scratch("missing");
		let o = PcOptions::load(&d.join("pc_options.txt"));
		assert_eq!(o.lang, None);
		assert!(o.camera_modern(), "a new player starts with the modern camera");
		assert!(o.strafe_on(), "a new player starts with strafe aiming on");
		assert!(o.buyall_on(), "a new player starts with buy all ammo on");
		assert_eq!(o.to_text(), "version=1\n", "nothing is written until the player chooses something");
	}

	#[test]
	fn round_trip_and_atomic_write() {
		let d = scratch("round");
		let p = d.join("pc_options.txt");
		let o = PcOptions { lang: Some(Lang::De), camera: Some(false), strafe: Some(false), buyall: Some(false), ..Default::default() };
		o.store(&p).unwrap();
		assert_eq!(std::fs::read_to_string(&p).unwrap(), "version=1\nlang=de\ncamera=classic\nstrafe=off\nbuyall=off\n");
		assert_eq!(PcOptions::load(&p), o);
		let names: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
		assert_eq!(names, ["pc_options.txt"], "no temporary file is left behind");
		// A second write replaces the first.
		let o2 = PcOptions { camera: Some(true), ..o.clone() };
		o2.store(&p).unwrap();
		assert_eq!(PcOptions::load(&p).camera, Some(true));
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn garbage_lines_and_bad_values_keep_the_other_keys() {
		let o = PcOptions::parse("\u{feff}version=1\nthis is not a setting\n=\nlang = FR \ncamera=sideways\nstrafe=OFF\n\n");
		assert_eq!(o.lang, Some(Lang::Fr), "case and spaces do not matter");
		assert_eq!(o.camera, None, "a bad value falls back to the default for that key only");
		assert!(o.camera_modern());
		assert_eq!(o.strafe, Some(false));
		assert_eq!(PcOptions::parse("buyall=OFF\nstrafe=maybe\n").buyall, Some(false));
		let o = PcOptions::parse("lang=klingon\ncamera=classic\n");
		assert_eq!(o.lang, None);
		assert_eq!(o.camera, Some(false));
	}

	#[test]
	fn unknown_keys_survive_a_rewrite() {
		let o = PcOptions::parse("version=1\nfuture_thing=7\ncamera=classic\nfuture_thing=8\n");
		assert_eq!(o.to_text(), "version=1\ncamera=classic\nfuture_thing=8\n");
	}

	#[test]
	fn language_numbers_both_ways() {
		// OSD number (what the cold kernel reports) and the game's language word (what Options -> Language stores).
		let want = [(Lang::En, "en", 1, 0), (Lang::Fr, "fr", 2, 2), (Lang::De, "de", 4, 3), (Lang::Es, "es", 3, 4), (Lang::It, "it", 5, 5)];
		for (l, code, osd, word) in want {
			assert_eq!((l.code(), l.osd(), l.word()), (code, osd, word));
			assert_eq!(Lang::from_code(code), Some(l));
			assert_eq!(Lang::from_word(word), Some(l));
			// The cold kernel's own parser agrees with the OSD number.
			assert_eq!(ee_oracle::cold::parse_language(code), Some(osd));
		}
		assert_eq!(Lang::from_word(1), None, "word 1 is not on the disc");
		assert_eq!(Lang::from_word(6), None);
	}
}
