//! The rows of a script mod's settings in the game's Mods pages (retail/pcmods.rs): from a manifest's `"settings"` declarations (the format:
//! bool, int, float, choice, string, keybind) or from the loader's own list (`omni_settings_json`, same objects plus `"value"`).
//!
//! A menu row cycles through at most four values, so only the BASIC settings get a row:
//! - bool: Off / On; choice with 2 to 4 choices: the choices' names;
//! - int / float: every value of the min..max grid when there are at most four, else four stops (min, default, a point between the
//!   default and max, max), the row showing the stop nearest to the stored value (the launcher sets any value);
//! - not shown (set them in the launcher): `"group": "advanced"`, string, keybind, a choice with more than four choices.
use super::json::{self, Json};
use std::path::Path;

/// Characters of a row's label / value text as the menu fonts hold them (a longer name is cut).
pub const MAX_LABEL: usize = 28;
pub const MAX_VALUE: usize = 11;

/// A character as the menu fonts draw it: letters, digits and a few signs, else '?'.
pub fn font_char(c: char) -> char {
	if c.is_ascii_alphanumeric() || " -.,'()+&!:/".contains(c) { c } else { '?' }
}

/// `s` as menu font text of at most `max` characters; `short` also cuts it at the first " (" (a choice's explanation).
pub fn font_text(s: &str, max: usize, short: bool) -> String {
	let s = if short { s.split(" (").next().unwrap_or(s) } else { s };
	s.chars().map(font_char).take(max).collect::<String>().trim_end().to_string()
}

/// One row: setting `key` of mod `mod_id` with its values (JSON value, text) and the one shown now.
#[derive(Clone, Debug, PartialEq)]
pub struct SetRow {
	pub mod_id: String,
	pub key: String,
	pub name: String,
	pub values: Vec<(Json, String)>,
	pub cur: usize,
}

fn num_text(v: f64) -> String {
	let v = (v * 1e6).round() / 1e6;
	if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") }
}

/// The values of a numeric setting (see the module doc).
fn stops(s: &Json, int: bool) -> Vec<f64> {
	let num = |k: &str| s.get(k).and_then(Json::as_num);
	let (Some(min), Some(max)) = (num("min"), num("max")) else { return Vec::new() };
	let step = num("step").filter(|v| *v > 0.0).unwrap_or(if int { 1.0 } else { (max - min) / 10.0 });
	if max <= min || step <= 0.0 {
		return Vec::new();
	}
	let grid = |v: f64| (min + ((v - min) / step).round() * step).clamp(min, max);
	let n = ((max - min) / step + 1e-9).floor() as usize + 1;
	let default = num("default").unwrap_or(min).clamp(min, max);
	let mut out: Vec<f64> = if n <= 4 { (0..n).map(|i| min + i as f64 * step).collect() } else { vec![min, grid(default), grid((default + max) / 2.0), max] };
	out.iter_mut().for_each(|v| *v = (*v * 1e6).round() / 1e6);
	out.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
	out
}

/// The row of one setting object, with `value` (the stored or current value, None = the default).
pub fn row(mod_id: &str, s: &Json, value: Option<&Json>) -> Option<SetRow> {
	let key = s.get("key").and_then(Json::str)?;
	if s.get("group").and_then(Json::str) == Some("advanced") {
		return None;
	}
	let name = font_text(s.get("name").and_then(Json::str).unwrap_or(key), MAX_LABEL, false);
	let default = s.get("default");
	let value = value.or(default);
	let (values, cur): (Vec<(Json, String)>, usize) = match s.get("type").and_then(Json::str)? {
		"bool" => (vec![(Json::Bool(false), "Off".into()), (Json::Bool(true), "On".into())], value.is_some_and(Json::bool) as usize),
		"choice" => {
			let list = s.get("choices")?.arr();
			let vals: Vec<(Json, String)> = list
				.iter()
				.filter_map(|c| {
					let v = c.get("value").and_then(Json::str)?;
					Some((Json::Str(v.to_string()), font_text(c.get("name").and_then(Json::str).unwrap_or(v), MAX_VALUE, true)))
				})
				.collect();
			if !(2..=4).contains(&vals.len()) {
				return None;
			}
			let cur = value.and_then(|v| vals.iter().position(|(j, _)| j == v)).unwrap_or(0);
			(vals, cur)
		}
		t @ ("int" | "float") => {
			let st = stops(s, t == "int");
			if st.len() < 2 {
				return None;
			}
			let x = value.and_then(Json::as_num).unwrap_or(st[0]);
			let cur = (0..st.len()).min_by(|&a, &b| (st[a] - x).abs().total_cmp(&(st[b] - x).abs())).unwrap_or(0);
			(st.iter().map(|&v| (Json::Num(v), num_text(v))).collect(), cur)
		}
		_ => return None,
	};
	Some(SetRow { mod_id: mod_id.to_string(), key: key.to_string(), name, values, cur })
}

/// The rows of a settings list (manifest or loader), `stored(key)` = the value in settings.json.
pub fn rows(mod_id: &str, settings: &[Json], stored: &dyn Fn(&str) -> Option<Json>) -> Vec<SetRow> {
	settings.iter().filter_map(|s| row(mod_id, s, s.get("key").and_then(Json::str).and_then(stored).as_ref())).collect()
}

/// The manifest of a mod folder: `omni-mod.json`, else (compat) OpenRAC's `openrac-mod.json`, as the loader DLL reads them.
pub(crate) fn manifest_file(dir: &Path) -> Option<std::path::PathBuf> {
	["omni-mod.json", "openrac-mod.json"].into_iter().map(|f| dir.join(f)).find(|p| p.is_file()) // compat: second name
}

/// A mod's (id, name, settings declarations) from `<dir>/omni-mod.json` (the manifest; the id is the folder name when it has none).
pub fn manifest(dir: &Path) -> Option<(String, String, Vec<Json>)> {
	let j = json::parse(&std::fs::read_to_string(manifest_file(dir)?).ok()?).ok()?;
	let id = j.get("id").and_then(Json::str).map(str::to_string).or_else(|| dir.file_name().map(|f| f.to_string_lossy().into_owned()))?;
	let name = j.get("name").and_then(Json::str).unwrap_or(&id).to_string();
	Some((id, name, j.get("settings").map_or(Vec::new(), |s| s.arr().to_vec())))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn setting(text: &str) -> Json {
		json::parse(text).unwrap()
	}

	#[test]
	fn bool_choice_and_number_rows() {
		let b = row("m", &setting(r#"{"key":"perks","name":"Extra projectiles and perks","type":"bool","default":true}"#), None).unwrap();
		assert_eq!((b.values.len(), b.cur, b.name.as_str()), (2, 1, "Extra projectiles and perks"));
		assert_eq!(row("m", &setting(r#"{"key":"k","type":"bool","default":true}"#), Some(&Json::Bool(false))).unwrap().cur, 0);
		let c = setting(r#"{"key":"cap","name":"Level cap","type":"choice","default":"uya","choices":[{"value":"uya","name":"Standard (V5, V8 in New Game+)"},{"value":"endless","name":"Endless (V99)"}]}"#);
		let r = row("m", &c, Some(&Json::Str("endless".into()))).unwrap();
		assert_eq!((r.cur, r.values[0].1.as_str(), r.values[1].1.as_str()), (1, "Standard", "Endless"));
		assert_eq!(row("m", &c, None).unwrap().cur, 0);
		assert_eq!(row("m", &c, Some(&Json::Str("bogus".into()))).unwrap().cur, 0, "an unknown stored value shows the first choice");
	}

	#[test]
	fn numbers_use_the_grid_or_four_stops_and_show_the_nearest() {
		let f = setting(r#"{"key":"xp_rate","name":"XP rate","type":"float","default":1.0,"min":0.5,"max":3.0,"step":0.1}"#);
		let r = row("m", &f, None).unwrap();
		let texts: Vec<&str> = r.values.iter().map(|v| v.1.as_str()).collect();
		assert_eq!(texts, ["0.5", "1", "2", "3"], "{:?}", r.values);
		assert_eq!(r.cur, 1);
		assert_eq!(row("m", &f, Some(&Json::Num(2.4))).unwrap().cur, 2, "2.4 is nearest to 2");
		let i = setting(r#"{"key":"n","type":"int","default":2,"min":1,"max":3}"#);
		assert_eq!(row("m", &i, None).unwrap().values.len(), 3, "a short grid lists every value");
		assert!(row("m", &setting(r#"{"key":"n","type":"int","default":2}"#), None).is_none(), "no bounds: no row");
	}

	#[test]
	fn advanced_string_keybind_and_long_choices_get_no_row() {
		assert!(row("m", &setting(r#"{"key":"a","type":"bool","default":false,"group":"advanced"}"#), None).is_none());
		assert!(row("m", &setting(r#"{"key":"a","type":"string","default":""}"#), None).is_none());
		assert!(row("m", &setting(r#"{"key":"a","type":"keybind","default":"x"}"#), None).is_none());
		let long = r#"{"key":"a","type":"choice","default":"a","choices":[{"value":"a","name":"A"},{"value":"b","name":"B"},{"value":"c","name":"C"},{"value":"d","name":"D"},{"value":"e","name":"E"}]}"#;
		assert!(row("m", &setting(long), None).is_none());
	}

	#[test]
	fn the_arsenal_mastery_manifest_gives_nine_basic_rows() {
		// dev: <root>/scratch_omni/...; lane copy: two levels up; release tree: bundled/mods (the mod the game ships)
		let m = env!("CARGO_MANIFEST_DIR");
		let found = ["../scratch_omni/omniloader/example-mods/arsenal-mastery", "../../scratch_omni/omniloader/example-mods/arsenal-mastery", "../bundled/mods/arsenal-mastery"]
			.iter()
			.find_map(|r| manifest(&Path::new(m).join(r)));
		let Some((_, name, list)) = found else { return };
		assert_eq!(name, "Arsenal Mastery");
		let r = rows("arsenal-mastery", &list, &|_| None);
		let keys: Vec<&str> = r.iter().map(|r| r.key.as_str()).collect();
		assert_eq!(keys, ["xp_rate", "level_cap", "rebalance", "nanotech", "level_power", "gold_at_v5", "perks", "ammo_mode", "xp_display"], "start_level is advanced");
	}

	#[test]
	fn texts_are_font_safe_and_short() {
		assert_eq!(font_text("Caf\u{e9} \u{201c}x\u{201d} %s_pack", 28, false), "Caf? ?x? ?s?pack");
		assert_eq!(font_text("Standard (explained at length)", 11, true), "Standard");
	}
}
