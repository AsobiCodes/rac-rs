//! `mods/active.txt` as the RAC-RS Launcher writes it, read and written by the game's own Mods page (retail/pcmods.rs) in EXACTLY the
//! launcher's format (contract: scratch_launcher\HANDOFF.md "SESSION 10"; the game side of the contract, `mods::parse_active`, is
//! unchanged: it ignores everything after a `#`).
//!
//! - Line 1 is always `HEADER` (recognised and rewritten, never kept as a foreign line).
//! - The two master ticks live in the file: a tick that is OFF is a comment line right after the header, `# Load texture packs: off`
//!   and / or `# Load mods: off`; ON writes no line. "Load texture packs" governs mods of kind `textures`, "Load mods" every other kind
//!   (the kind comes from the mod's mod.json; an id with no installed mod belongs to no tick and is never held back).
//! - Tick OFF: every id of that kind is written as `# <id>` at the same place of the priority order, tick ON writes the plain ids back,
//!   so the per-mod switches and the order survive. A `# <id>` line counts as a held mod only while that kind's tick is off and the id
//!   is an installed mod of that kind; a hand-commented line is turned into `## <id>` when a tick is switched off.
//! - Any other `#` line is kept: it stays directly above the id line that followed it and moves with that id; lines after the last
//!   id, or whose id is gone, go to the end. Blank lines are dropped. A BOM and CRLF are read; the output is LF without a BOM.
use std::collections::HashSet;

/// The first line of the file.
pub const HEADER: &str = "# Enabled mods, one id per line, highest priority first. Written by the RAC-RS Launcher.";

/// The two master ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Master {
	/// Texture packs (kind `textures`).
	Textures,
	/// Every other kind: script and native mods.
	Mods,
}

impl Master {
	pub const ALL: [Master; 2] = [Master::Textures, Master::Mods];

	/// The tick that governs a mod of this kind.
	pub fn of_kind(kind: &str) -> Master {
		if kind == "textures" {
			Master::Textures
		} else {
			Master::Mods
		}
	}

	/// The tick's name in the marker line.
	pub fn label(self) -> &'static str {
		match self {
			Master::Textures => "Load texture packs",
			Master::Mods => "Load mods",
		}
	}
}

/// Which ticks are on (stored as "off" flags: `Default` = nothing touched = both on).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Masters {
	textures_off: bool,
	mods_off: bool,
}

impl Masters {
	pub fn is_on(&self, m: Master) -> bool {
		!match m {
			Master::Textures => self.textures_off,
			Master::Mods => self.mods_off,
		}
	}

	pub fn set(&mut self, m: Master, on: bool) {
		match m {
			Master::Textures => self.textures_off = !on,
			Master::Mods => self.mods_off = !on,
		}
	}
}

/// Foreign `#` lines: (the id line they sit above, None = at the end, the line).
pub type Kept = Vec<(Option<String>, String)>;

/// What a file says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
	/// Every mod that is switched on per mod, highest priority first (also those a tick holds back).
	pub ids: Vec<String>,
	pub masters: Masters,
	pub kept: Kept,
}

/// `# Load texture packs: off` -> (Textures, false). Case and spaces around the colon do not matter; `on` is accepted.
fn marker(l: &str) -> Option<(Master, bool)> {
	let (label, value) = l.strip_prefix('#')?.trim().split_once(':')?;
	let m = Master::ALL.into_iter().find(|m| m.label().eq_ignore_ascii_case(label.trim()))?;
	match value.trim().to_ascii_lowercase().as_str() {
		"off" => Some((m, false)),
		"on" => Some((m, true)),
		_ => None,
	}
}

/// `# alpha` -> Some("alpha"): one word after the `#`, no marker.
fn single_word(l: &str) -> Option<&str> {
	let body = l.strip_prefix('#')?.trim();
	(!body.is_empty() && !body.contains(char::is_whitespace) && !body.starts_with('#') && !body.contains(':')).then_some(body)
}

/// Reads the file. `master_of(id)` = the tick of an installed mod (None = not installed or unreadable).
pub fn parse(text: &str, master_of: &dyn Fn(&str) -> Option<Master>) -> Parsed {
	let lines: Vec<&str> = text.lines().map(|l| l.trim_start_matches('\u{feff}').trim()).filter(|l| !l.is_empty()).collect();
	let mut p = Parsed::default();
	for l in &lines {
		if let Some((m, on)) = marker(l) {
			p.masters.set(m, on);
		}
	}
	let mut pending: Vec<String> = Vec::new();
	for l in lines {
		let id = if l == HEADER || marker(l).is_some() {
			continue;
		} else if l.starts_with('#') {
			match single_word(l).and_then(|w| master_of(w).filter(|m| !p.masters.is_on(*m)).map(|_| w)) {
				Some(w) => w,
				None => {
					pending.push(l.to_string());
					continue;
				}
			}
		} else {
			l
		};
		for c in pending.drain(..) {
			p.kept.push((Some(id.to_string()), c));
		}
		if !p.ids.iter().any(|x| x == id) {
			p.ids.push(id.to_string());
		}
	}
	p.kept.extend(pending.into_iter().map(|c| (None, c)));
	p
}

/// The file text: the header, the marker lines of the ticks that are off, then the ids (`# id` for those a tick holds back).
pub fn render(ids: &[String], masters: Masters, kept: &[(Option<String>, String)], master_of: &dyn Fn(&str) -> Option<Master>) -> String {
	let mut s = format!("{HEADER}\n");
	for m in Master::ALL {
		if !masters.is_on(m) {
			s += &format!("# {}: off\n", m.label());
		}
	}
	let held = |id: &str| master_of(id).is_some_and(|m| !masters.is_on(m));
	// A typed comment that looks like a held mod would become a switched-on mod at the next tick cycle: `##` keeps it a comment.
	let safe = |c: &str| match single_word(c).and_then(|w| master_of(w).filter(|m| !masters.is_on(*m))) {
		Some(_) => format!("#{c}"),
		None => c.to_string(),
	};
	let known: HashSet<&str> = ids.iter().map(|i| i.as_str()).collect();
	for id in ids {
		for (anchor, line) in kept {
			if anchor.as_deref() == Some(id.as_str()) {
				s += &safe(line);
				s += "\n";
			}
		}
		if held(id) {
			s += "# ";
		}
		s += id;
		s += "\n";
	}
	for (anchor, line) in kept {
		if anchor.as_deref().map(|a| !known.contains(a)).unwrap_or(true) {
			s += &safe(line);
			s += "\n";
		}
	}
	s
}
