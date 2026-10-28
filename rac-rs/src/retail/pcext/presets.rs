//! The Graphics page's Preset row (Pause -> Options -> Graphics -> Picture): Low, Medium, High and Custom.
//!
//! The values are ONE data file, `rac-rs/assets/gfx_presets.txt` (`<preset>.<key>=<value>` lines, built in with `include_str!`; the
//! RAC-RS Launcher shows the same three presets from the same table). A preset sets the picture options (lighting, draw distance, frame
//! smoothing, HUD, ambient occlusion, dynamic shadows, water, particles, sun rays, vegetation, global illumination, waves, fog, clouds); it never
//! touches the camera, controls, language, display mode or renderer. Every preset sets Culling On (v0.4.1: Off gives no gameplay benefit and
//! only costs speed); Culling Off with the other rows at a preset reads as Custom. High is the game's default picture.
//!
//! Choosing a preset (the player cycles the row) writes its values into the Graphics rows' bytes and the HUD row; the sync code then
//! sees them as any other change (renderer verdict, `pc_options.txt`). The row always shows the preset that matches the current
//! values, High first, then Medium, then Low, else Custom, so touching any other row turns it to Custom. Custom is a choice of the row
//! only while the state is Custom (`pcctl::preset_show`).
use super::*;
use crate::retail::pcctl;
use std::sync::OnceLock;

/// The table: the file's text.
const TABLE: &str = include_str!("../../../assets/gfx_presets.txt");

/// The names in row order (the row byte is the index; 3 = Custom).
const NAMES: [&str; 4] = ["Low", "Medium", "High", "Custom"];
const PRESETS: [&str; 3] = ["low", "medium", "high"];
/// The keys of a preset, in the file's order.
#[cfg(test)]
const KEYS: [&str; 15] = ["lighting", "draw", "smoothing", "hud", "ao", "shadows", "water", "particles", "sunrays", "vegetation", "gi", "waves", "fog", "clouds", "texfilter"];

/// The values of one preset as levels (`pcopts` numbering: 0 Off / Classic .. 3 High; draw 0 Low .. 2 High).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Vals {
	lighting: u8,
	draw: u8,
	smoothing: bool,
	hud_sharp: bool,
	ao: u8,
	shadows: u8,
	water: u8,
	particles: u8,
	sun: u8,
	veg: u8,
	gi: u8,
	waves: bool,
	fog: u8,
	clouds: u8,
	texfilter: bool,
	/// Culling On. Not in the table file: every preset has it On.
	culling: bool,
}

fn pos(list: &[&str], v: &str) -> Option<u8> {
	list.iter().position(|x| x.eq_ignore_ascii_case(v)).map(|p| p as u8)
}

fn flag(v: &str, yes: &str, no: &str) -> Option<bool> {
	pos(&[no, yes], v).map(|p| p == 1)
}

/// One level word: `off` and `classic` are 0.
fn level(v: &str) -> Option<u8> {
	pos(&pcopts::LEVELS, v).or_else(|| pos(&pcopts::LEVELS_CLASSIC, v))
}

/// The three presets of a table text (Low, Medium, High); every key of every preset must be there.
fn parse(text: &str) -> Result<[Vals; 3], String> {
	let mut vals = Vec::new();
	for name in PRESETS {
		let get = |key: &str| -> Result<&str, String> {
			let want = format!("{name}.{key}");
			text.lines().filter_map(|l| l.trim().split_once('=')).find(|(k, _)| k.trim() == want).map(|(_, v)| v.trim()).ok_or(format!("{want} is missing"))
		};
		let bad = |key: &str| format!("{name}.{key} has no valid value");
		let lv = |key: &str| get(key).and_then(|v| level(v).ok_or(bad(key)));
		vals.push(Vals {
			lighting: get("lighting").and_then(|v| pos(&pcopts::LIGHTING, v).ok_or(bad("lighting")))?,
			draw: get("draw").and_then(|v| pos(&pcopts::DRAW, v).ok_or(bad("draw")))?,
			smoothing: get("smoothing").and_then(|v| flag(v, "on", "off").ok_or(bad("smoothing")))?,
			hud_sharp: get("hud").and_then(|v| flag(v, "sharp", "original").ok_or(bad("hud")))?,
			ao: lv("ao")?,
			shadows: lv("shadows")?,
			water: lv("water")?,
			particles: lv("particles")?,
			sun: lv("sunrays")?,
			veg: get("vegetation").and_then(|v| pos(&["off", "moderate", "lush"], v).ok_or(bad("vegetation")))?,
			gi: lv("gi")?,
			waves: get("waves").and_then(|v| flag(v, "on", "off").ok_or(bad("waves")))?,
			fog: lv("fog")?,
			clouds: lv("clouds")?,
			texfilter: get("texfilter").and_then(|v| flag(v, "smooth", "classic").ok_or(bad("texfilter")))?,
				culling: true,
		});
	}
	Ok([vals[0], vals[1], vals[2]])
}

/// The built-in table, parsed once (a broken file cannot get past the unit test; the fallback is the default picture three times).
fn table() -> &'static [Vals; 3] {
	static T: OnceLock<[Vals; 3]> = OnceLock::new();
	T.get_or_init(|| {
		parse(TABLE).unwrap_or_else(|e| {
			eprintln!("retail: PC options: assets/gfx_presets.txt: {e}");
			let d = vals_of(swap_rows(PcOptions::new().graphics_rows()), PcOptions::new().hud_sharp());
			[d; 3]
		})
	})
}

/// The values the Graphics rows' bytes (`GROWS` order) and the HUD row stand for.
fn vals_of(g: [u8; 15], hud_sharp: bool) -> Vals {
	let l = swap_rows(g);
	Vals { lighting: l[0].min(1), draw: pcopts::draw_level(g[12]), smoothing: l[7] == 1, hud_sharp, ao: l[1], shadows: l[2], water: l[4], particles: l[5], sun: l[6], veg: l[8].min(2), gi: l[10], waves: l[11] == 0, fog: l[13], clouds: l[14], texfilter: true, culling: g[9] == 0 }
}

/// `vals_of`'s inverse on the bytes the preset sets (culling On); display mode stays as it is.
fn apply_to(g: &mut [u8; 15], v: Vals) {
	let mut l = swap_rows(*g);
	(l[0], l[1], l[2], l[4], l[5], l[6], l[7], l[8], l[10], l[11], l[13], l[14]) = (v.lighting, v.ao, v.shadows, v.water, v.particles, v.sun, v.smoothing as u8, v.veg, v.gi, !v.waves as u8, v.fog, v.clouds);
	*g = swap_rows(l);
	g[12] = pcopts::draw_byte(v.draw);
	g[9] = !v.culling as u8;
}

/// The Preset row's byte for these values: High first, then Medium, then Low, else 3 (Custom).
fn detect(g: [u8; 15], hud_sharp: bool, texfilter: bool) -> u8 {
	let v = Vals { texfilter, ..vals_of(g, hud_sharp) };
	[2usize, 1, 0].into_iter().find(|&p| table()[p] == v).map_or(3, |p| p as u8)
}

/// Worker, every field in `pcext::sync` before the rows are stepped: when the player chose a preset, its values go into the rows and
/// the HUD row, and the renderer and the options file follow like for any change.
pub(super) fn choose(mem: &mut Mem, s: &mut St) {
	let Some(p) = pcctl::preset_chosen(mem) else { return };
	let mut g = GROWS.map(|a| mem.rd8(a));
	let v = table()[p as usize];
	apply_to(&mut g, v);
	for (a, b) in GROWS.iter().zip(g) {
		if mem.rd8(*a) != b {
			mem.wr8(*a, b);
		}
	}
	s.opts.texfilter = Some(v.texfilter);
	s.glast = Some(g);
	apply_graphics(s, g, true);
	pcctl::set_hud(mem, v.hud_sharp);
	eprintln!("retail: PC options: preset {}", NAMES[p as usize]);
}

/// Worker, the end of `pcext::sync`: the Preset row shows the preset that matches the rows now.
pub(super) fn show(mem: &mut Mem, s: &St) {
	let (Some(g), Some(sharp)) = (s.glast, pcctl::hud_sharp(mem)) else { return };
	pcctl::preset_show(mem, &ROWS, GTAB, detect(g, sharp, s.opts.texfilter.unwrap_or(true)));
}

#[cfg(test)]
mod tests {
	use super::*;

	fn defaults() -> ([u8; 15], bool) {
		(swap_rows(PcOptions::new().graphics_rows()), PcOptions::new().hud_sharp())
	}

	/// The file has the 45 lines of the launcher's table, preset by preset in the launcher's key order, and nothing else but comments.
	#[test]
	fn the_table_file_is_complete_and_in_the_launchers_order() {
		let lines: Vec<&str> = TABLE.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
		assert_eq!(lines.len(), 45);
		for (i, l) in lines.iter().enumerate() {
			let (k, v) = l.split_once('=').expect("key=value");
			assert_eq!(k, format!("{}.{}", PRESETS[i / 15], KEYS[i % 15]), "line {i}");
			assert!(!v.is_empty() && v == v.trim());
		}
		let t = parse(TABLE).expect("parses");
		assert!(t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
		assert!(parse("low.ao=low").is_err(), "a missing key is an error");
		assert!(parse(&TABLE.replace("high.waves=on", "high.waves=maybe")).is_err(), "a bad value is an error");
	}

	/// The same values as the launcher's table (`ui/presets.rs`: a first guess, no measurements).
	#[test]
	fn the_values_are_the_launchers() {
		let [low, medium, high] = *table();
		assert_eq!(low, Vals { lighting: 0, draw: 0, smoothing: false, hud_sharp: false, ao: 1, shadows: 1, water: 1, particles: 1, sun: 1, veg: 0, gi: 0, waves: false, fog: 0, clouds: 0, texfilter: true, culling: true });
		assert_eq!(medium, Vals { lighting: 0, draw: 0, smoothing: true, hud_sharp: true, ao: 2, shadows: 2, water: 2, particles: 2, sun: 2, veg: 1, gi: 0, waves: true, fog: 1, clouds: 1, texfilter: true, culling: true });
		assert_eq!(high, Vals { lighting: 0, draw: 1, smoothing: true, hud_sharp: true, ao: 3, shadows: 3, water: 3, particles: 3, sun: 3, veg: 1, gi: 0, waves: true, fog: 3, clouds: 3, texfilter: true, culling: true });
	}

	/// The `<preset>.<key>=<value>` lines the launcher's `TABLE` (ui/presets.rs: `("key", ["low", "medium", "high"]),` rows) stands for.
	fn launcher_lines(src: &str) -> Vec<String> {
		let rows: Vec<Vec<&str>> = src
			.lines()
			.skip_while(|l| !l.contains("pub const TABLE"))
			.map(str::trim)
			.take_while(|l| *l != "];")
			.filter(|l| l.starts_with("(\""))
			.map(|l| l.split('"').skip(1).step_by(2).collect())
			.collect();
		PRESETS.iter().enumerate().flat_map(|(p, name)| rows.iter().map(move |r| format!("{name}.{}={}", r[0], r[1 + p]))).collect()
	}

	#[test]
	fn the_launcher_table_reader_reads_a_table() {
		let sample = "use x;\npub const TABLE: &[(&str, [&str; 3])] = &[\n    //  key  Low  Medium  High\n    (\"draw\", [\"low\", \"low\", \"medium\"]),\n    (\"waves\", [\"off\", \"on\", \"on\"]),\n];\nfn other() { (\"no\", [\"a\", \"b\", \"c\"]) }\n";
		assert_eq!(launcher_lines(sample), vec!["low.draw=low", "low.waves=off", "medium.draw=low", "medium.waves=on", "high.draw=medium", "high.waves=on"]);
	}

	/// The game's table file equals the launcher's `TABLE` value for value, in the same order. Needs the launcher sources
	/// (`scratch_launcher` next to the lane or the dev tree); without them the check is skipped.
	#[test]
	fn the_table_file_equals_the_launchers_table_value_for_value() {
		let m = env!("CARGO_MANIFEST_DIR");
		let near = ["../scratch_launcher", "../../scratch_launcher"].map(|r| std::path::Path::new(m).join(r).join("launcher/src/ui/presets.rs"));
		let Some(path) = near.into_iter().find(|p| p.is_file()) else {
			eprintln!("skipped: the launcher sources (scratch_launcher/launcher/src/ui/presets.rs) are not here");
			return;
		};
		let theirs = launcher_lines(&std::fs::read_to_string(&path).unwrap());
		let ours: Vec<String> = TABLE.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string).collect();
		assert_eq!(theirs.len(), 45, "the launcher table at {} was read", path.display());
		assert_eq!(ours, theirs, "assets/gfx_presets.txt and the launcher's TABLE differ");
	}

	/// High is today's default picture (a new player's options), so the Preset row of a new player reads High.
	#[test]
	fn high_is_the_default_picture() {
		let (g, sharp) = defaults();
		assert_eq!(vals_of(g, sharp), table()[2]);
		assert_eq!(detect(g, sharp, true), 2);
	}

	#[test]
	fn applying_a_preset_reads_back_as_that_preset_sets_culling_on_and_leaves_display() {
		for start in [[0u8; 15], [1, 3, 2, 1, 3, 1, 2, 0, 2, 1, 3, 1, 2, 2, 2], defaults().0] {
			for (p, v) in table().iter().enumerate() {
				let mut g = start;
				apply_to(&mut g, *v);
				assert_eq!(detect(g, v.hud_sharp, v.texfilter), p as u8, "preset {p} from {start:?}");
				assert_eq!(vals_of(g, v.hud_sharp), *v);
				assert_eq!(g[3], start[3], "display mode stays");
				assert_eq!(g[9], 0, "every preset sets Culling On, also from Off");
			}
		}
	}

	/// One value changed away from a preset is Custom (unless the change lands on another preset); display mode alone is no change.
	#[test]
	fn any_other_value_is_custom() {
		let (g, sharp) = defaults();
		assert_eq!(detect(g, !sharp, true), 3, "HUD Original with the High picture");
		for row in [0usize, 1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14] {
			for to in 0..4u8 {
				let mut c = g;
				c[row] = to;
				let v = vals_of(c, sharp);
				let want = [2usize, 1, 0].into_iter().find(|&p| table()[p] == v).map_or(3, |p| p as u8);
				assert_eq!(detect(c, sharp, true), want, "row {row} byte {to}");
			}
		}
		let mut c = g;
		c[1] = 2;
		assert_eq!(detect(c, sharp, true), 3, "ambient occlusion Medium with the rest High");
		let mut off = g;
		off[9] = 1;
		assert_eq!(detect(off, sharp, true), 3, "High with Culling Off is Custom, so the row never claims a preset it would not apply");
		let mut w = g;
		w[3] = 1;
		assert_eq!(detect(w, sharp, true), 2, "fullscreen alone does not make it Custom");
	}

	/// The host side: the player cycles the row, the rows and the HUD take the preset, the row shows what matches.
	#[test]
	fn choosing_sets_the_rows_and_a_changed_row_shows_custom() {
		let _g = crate::retail::pcopts::globals_guard();
		let (g, sharp) = defaults();
		let mut mem = Mem::new();
		for (a, b) in GROWS.iter().zip(g) {
			mem.wr8(*a, b);
		}
		let mut s = St { opts: PcOptions::new(), path: None, last: None, glast: Some(g), lang_seen: None, warned: false };
		pcctl::build_gfx(&mut mem, GTAB, &ROWS, sharp, false);
		choose(&mut mem, &mut s);
		show(&mut mem, &s);
		assert_eq!(mem.rd8(pcctl::PRESETB), 2, "a new player reads High");
		mem.wr8(pcctl::PRESETB, 0); // the player cycles to Low
		choose(&mut mem, &mut s);
		let got = GROWS.map(|a| mem.rd8(a));
		assert_eq!(vals_of(got, false), table()[0]);
		assert_eq!(s.glast, Some(got), "the stepping code sees no change");
		assert_eq!(pcctl::hud_sharp(&mut mem), Some(false), "Low wants the Original HUD");
		show(&mut mem, &s);
		assert_eq!(mem.rd8(pcctl::PRESETB), 0);
		// a row changes: Custom, with its 4th choice in the table
		mem.wr8(GROWS[1], 2);
		s.glast = Some(GROWS.map(|a| mem.rd8(a)));
		show(&mut mem, &s);
		assert_eq!(mem.rd8(pcctl::PRESETB), 3);
		assert_eq!(mem.rd32(GTAB + 24 + 20), pcctl::ID_CUSTOM);
	}
}
