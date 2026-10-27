//! `active.txt` of a profile: the per-mod switches (ids, highest priority first), the two master ticks of the Mods page, and the lines
//! the launcher did not write.
//!
//! Contract with the game (does not change): `mods\active.txt` lists the enabled ids, one per line; text after a `#` is ignored.
//! A master tick that is OFF writes the ids of its kind as `# <id>` lines (the game skips them); ON writes them back as plain `<id>`
//! lines, at the same place. The tick itself is remembered in a comment line (`# Load texture packs: off`), written only while it is off,
//! so a profile that never touched a tick has a file as before. Lines this code does not know are kept: each one stays above the id
//! line that followed it (or at the end).
use std::collections::HashSet;

/// The first line the launcher writes (also how an old file's header is recognised, so it is not kept as a foreign line).
pub const HEADER: &str = "# Enabled mods, one id per line, highest priority first. Written by the RAC-RS Launcher.";

/// The two master ticks of the Mods page.
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

    /// The text of the tick on the page and in the marker line.
    pub fn label(self) -> &'static str {
        match self {
            Master::Textures => "Load texture packs",
            Master::Mods => "Load mods",
        }
    }
}

/// Which master ticks are on. Stored as "off" flags so that `Default` (nothing touched) is "everything on".
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

    pub fn any_off(&self) -> bool {
        self.textures_off || self.mods_off
    }
}

/// Foreign `#` lines: (the id line they sit above, None = at the end, the line).
pub type Kept = Vec<(Option<String>, String)>;

/// What a file says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    /// Every mod that is switched on per mod, highest priority first (also those a master tick holds back).
    pub ids: Vec<String>,
    pub masters: Masters,
    pub kept: Kept,
}

/// `# Load texture packs: off` -> (Textures, false).
fn marker(l: &str) -> Option<(Master, bool)> {
    let body = l.strip_prefix('#')?.trim();
    let (label, value) = body.split_once(':')?;
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

/// Reads the file. `master_of(id)` = the tick of an installed mod (None = not installed or unreadable). A `# <id>` line is a mod that a
/// tick holds back only while that tick is off; with the tick on it is a comment someone typed, and stays one.
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

/// The file text: the header, the tick lines of ticks that are off, then the ids (`# id` for those a tick holds back).
pub fn render(ids: &[String], masters: Masters, kept: &[(Option<String>, String)], master_of: &dyn Fn(&str) -> Option<Master>) -> String {
    let mut s = format!("{HEADER}\n");
    for m in Master::ALL {
        if !masters.is_on(m) {
            s += &format!("# {}: off\n", m.label());
        }
    }
    let held = |id: &str| master_of(id).map(|m| !masters.is_on(m)).unwrap_or(false);
    // a typed comment that looks like a held mod would turn into a switched-on mod the next time the tick is switched on: `##` keeps it a comment
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

#[cfg(test)]
mod tests {
    use super::*;

    /// a, b = texture packs; s = a script mod; ghost is not installed.
    fn kinds(id: &str) -> Option<Master> {
        match id {
            "a" | "b" => Some(Master::Textures),
            "s" => Some(Master::Mods),
            _ => None,
        }
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_file_without_ticks_is_what_the_launcher_always_wrote() {
        let text = format!("{HEADER}\nb\na\nghost\n");
        let p = parse(&text, &kinds);
        assert_eq!((p.ids.clone(), p.masters, p.kept.len()), (ids(&["b", "a", "ghost"]), Masters::default(), 0));
        assert_eq!(render(&p.ids, p.masters, &p.kept, &kinds), text, "byte for byte");
        assert_eq!(super::super::active_text(&p.ids), text);
    }

    #[test]
    fn tick_off_comments_the_ids_of_its_kind_and_on_restores_them_in_place() {
        let on = format!("{HEADER}\ns\nb\na\n");
        let mut p = parse(&on, &kinds);
        p.masters.set(Master::Textures, false);
        let off = render(&p.ids, p.masters, &p.kept, &kinds);
        assert_eq!(off, format!("{HEADER}\n# Load texture packs: off\ns\n# b\n# a\n"), "the script mod is untouched");
        // the game's rule: text after # is ignored
        let game_sees: Vec<&str> = off.lines().filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty()).collect();
        assert_eq!(game_sees, vec!["s"]);
        // the launcher reads its own file: switches remembered, tick off
        let q = parse(&off, &kinds);
        assert_eq!(q.ids, ids(&["s", "b", "a"]), "per-mod switches and order survive");
        assert!(!q.masters.is_on(Master::Textures) && q.masters.is_on(Master::Mods));
        let mut q = q;
        q.masters.set(Master::Textures, true);
        assert_eq!(render(&q.ids, q.masters, &q.kept, &kinds), on, "ON restores the file exactly");
    }

    #[test]
    fn both_ticks_off_and_a_tick_with_nothing_to_hold_is_still_remembered() {
        let mut p = parse(&format!("{HEADER}\ns\na\n"), &kinds);
        p.masters.set(Master::Textures, false);
        p.masters.set(Master::Mods, false);
        let off = render(&p.ids, p.masters, &p.kept, &kinds);
        assert_eq!(off, format!("{HEADER}\n# Load texture packs: off\n# Load mods: off\n# s\n# a\n"));
        assert_eq!(parse(&off, &kinds).ids, ids(&["s", "a"]));
        // no mod of the kind: the tick line alone keeps the state
        let q = parse(&format!("{HEADER}\n# Load mods: off\na\n"), &kinds);
        assert!(!q.masters.is_on(Master::Mods) && q.masters.is_on(Master::Textures));
        assert_eq!(q.ids, ids(&["a"]));
        assert!(render(&q.ids, q.masters, &q.kept, &kinds).contains("# Load mods: off\n"));
    }

    #[test]
    fn ids_that_are_not_installed_are_never_held_back() {
        let mut p = parse(&format!("{HEADER}\nghost\na\n"), &kinds);
        p.masters.set(Master::Textures, false);
        let off = render(&p.ids, p.masters, &p.kept, &kinds);
        assert!(off.contains("\nghost\n# a\n"), "{off}");
    }

    #[test]
    fn unknown_lines_are_kept_above_the_id_they_sat_above() {
        let text = "# my notes\n# keep this\nb\n# the loud one\na\n# at the end\n";
        let p = parse(text, &kinds);
        assert_eq!(p.ids, ids(&["b", "a"]));
        let out = render(&p.ids, p.masters, &p.kept, &kinds);
        assert_eq!(out, format!("{HEADER}\n# my notes\n# keep this\nb\n# the loud one\na\n# at the end\n"));
        // a mod that moves takes its note along; a note of a removed mod is not lost
        let moved = ids(&["a", "b"]);
        assert!(render(&moved, p.masters, &p.kept, &kinds).contains("# the loud one\na\n# my notes\n# keep this\nb\n"));
        let gone = ids(&["a"]);
        let out = render(&gone, p.masters, &p.kept, &kinds);
        assert!(out.contains("# my notes\n") && out.contains("# keep this\n") && out.ends_with("# at the end\n"), "{out}");
        // and while a tick is off
        let mut m = p.masters;
        m.set(Master::Textures, false);
        let off = render(&p.ids, m, &p.kept, &kinds);
        assert!(off.contains("# my notes\n# keep this\n# b\n# the loud one\n# a\n# at the end\n"), "{off}");
        assert_eq!(parse(&off, &kinds).kept, p.kept, "the same lines come back");
    }

    #[test]
    fn a_typed_comment_that_looks_like_a_held_mod_is_not_switched_on_by_a_tick_cycle() {
        // the player commented `a` out by hand while the tick was on: the line is a comment, not a held mod
        let p = parse("b\n# a\n", &kinds);
        assert_eq!(p.ids, ids(&["b"]));
        let mut m = p.masters;
        m.set(Master::Textures, false);
        let off = render(&p.ids, m, &p.kept, &kinds);
        let q = parse(&off, &kinds);
        assert_eq!(q.ids, ids(&["b"]), "a is still off: {off}");
        let mut m = q.masters;
        m.set(Master::Textures, true);
        let back = render(&q.ids, m, &q.kept, &kinds);
        assert!(!back.lines().any(|l| l == "a"), "{back}");
        assert!(back.lines().any(|l| l.contains('a') && l.starts_with('#')), "the line is still there: {back}");
    }

    #[test]
    fn bom_crlf_blank_lines_duplicates_and_other_header_text_are_tolerated() {
        let p = parse("\u{feff}# whatever the old header was\r\n\r\nb\r\n  a  \r\nb\r\n", &kinds);
        assert_eq!(p.ids, ids(&["b", "a"]));
        assert_eq!(p.kept, vec![(Some("b".to_string()), "# whatever the old header was".to_string())]);
        assert!(!render(&p.ids, p.masters, &p.kept, &kinds).contains('\r'));
        assert_eq!(parse("# Load Texture Packs : OFF\na\n", &kinds).masters.is_on(Master::Textures), false, "case and spaces do not matter");
        assert_eq!(parse("# Load texture packs: maybe\na\n", &kinds).kept.len(), 1, "a marker with a value we do not know is a foreign line");
    }
}
