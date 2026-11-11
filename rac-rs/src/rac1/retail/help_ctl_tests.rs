//! Tests of the Help > Controls picture patch: no-op without a remap, every label slot fits every action in every language, the
//! unchanged-label rule, and (when the dev disc copy and the output folder exist) pictures of the real art as PPM files.
use super::help_ctl::{self as hc, Op};
use super::help_ctl_data::{HALVES, LABS, SLOTS};
use super::help_ctl_text as tx;
use crate::kbm_actions as ka;
use rac_engine::input::kbm::{self, Binds, Layer};

const DISC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");

/// Installs the game's menu font from the dev disc copy; false (the test returns) when the disc is not there.
fn fixture() -> bool {
	use crate::{iso, level::Level, toc};
	if !std::path::Path::new(DISC).exists() {
		return false;
	}
	super::help_ctl_font::install(1, || {
		let mut disc = iso::Disc::open(std::path::Path::new(DISC)).ok()?;
		let levels = toc::read_levels(&mut disc).ok()?;
		let glyphs = toc::font_glyph_tables(&mut disc).ok()?;
		let l = Level::load(&mut disc, &levels[0]).ok()?;
		let (sheet, pal) = crate::hud::level_fonts_raw(&l).ok()?.into_iter().next()?;
		let mut alpha = [0u8; 256];
		for (a, c) in alpha.iter_mut().zip(&pal) {
			*a = c[3];
		}
		let table: Vec<(u8, u8, i8, u8)> = glyphs[0].iter().map(|g| (g.u as u8, g.v as u8, g.dy as i8, g.adv as u8)).collect();
		super::help_ctl_font::Font::new(&sheet, &alpha, &table)
	})
	.is_some()
}

/// The language index that belongs to picture slot `s` (slots 0, 2..5 = En, Fr, De, Es, It; slot 1 = the US English art).
fn lang_of(s: usize) -> usize {
	if s == 0 || s == 1 { 0 } else { s - 1 }
}

fn with_only(a: usize, p: u8) -> Binds {
	ka::install();
	let t = kbm::table();
	let mut b = Binds::defaults(t);
	for slots in b.1.iter_mut() {
		for s in slots.iter_mut() {
			if matches!(s, Some(x) if *x == p) {
				*s = None;
			}
		}
	}
	b.1[a][0] = Some(p);
	b
}

/// The `n` widest visible extra lines of slot `s` (a label's fallback line is placed there first).
fn widest_visible(s: usize, n: usize) -> Vec<[u16; 4]> {
	let mut v: Vec<[u16; 4]> = SLOTS[s].iter().map(|r| hc::visible(*r)).collect();
	v.sort_by_key(|r| std::cmp::Reverse(r[2]));
	v.truncate(n);
	v
}


#[test]
fn defaults_repaint_nothing() {
	ka::install();
	let t = kbm::table();
	let b = Binds::defaults(t);
	for s in 0..6 {
		assert!(hc::plan(t, &b, s, lang_of(s)).is_empty(), "slot {s}");
	}
}

#[test]
fn halves_table_has_twelve_distinct_hashes() {
	let mut h: Vec<u64> = HALVES.iter().map(|e| e.0).collect();
	h.sort_unstable();
	h.dedup();
	assert_eq!(h.len(), 12);
}

#[test]
fn every_action_fits_every_label_in_every_language_alone_and_paired() {
	if !fixture() {
		return;
	}
	ka::install();
	let t = kbm::table();
	let acts: Vec<usize> = t.actions.iter().enumerate().filter(|(_, a)| a.pad_ok && a.layer == Layer::Game).map(|(i, _)| i).collect();
	for s in 0..6 {
		let lang = lang_of(s);
		for lab in LABS[s].iter() {
			let orig = lab.clear[0];
			for &a in &acts {
				for &o in &acts {
					let units: Vec<String> = [a, o].iter().flat_map(|&i| tx::main_words(t.actions[i].id, lang)).collect();
					let one = &units[..tx::main_words(t.actions[a].id, lang).len()];
					let fits = tx::layout(one, true, hc::visible(lab.free), orig, false).is_some();
					// Too narrow (Map / Menu): the fallback is an extra line "Button: short name", which must fit every extra line.
					let fall = String::from("Select x1");
					assert!(fits || widest_visible(s, 4).iter().all(|sl| tx::layout(std::slice::from_ref(&fall), false, *sl, [sl[0], sl[1], 1, sl[3]], true).is_some()), "slot {s} label {} action {} {one:?} box {:?}", lab.key, t.actions[a].id, lab.free);
					let two = tx::layout(&units, true, hc::visible(lab.free), orig, false).is_some();
					let count = String::from("Select x2");
					assert!(two || widest_visible(s, 4).iter().all(|sl| tx::layout(std::slice::from_ref(&count), false, *sl, [sl[0], sl[1], 1, sl[3]], true).is_some()), "slot {s} pair {units:?}");
				}
			}
		}
		assert!(SLOTS[s].len() >= 7, "slot {s} has {} extra lines", SLOTS[s].len());
	}
}

#[test]
fn only_changed_labels_are_repainted_and_every_moved_action_appears() {
	if !fixture() {
		return;
	}
	ka::install();
	let t = kbm::table();
	let jump = t.index("jump").unwrap();
	let mut b = Binds::defaults(t);
	b.1[jump][0] = Some(1); // Jump on B (Circle): Circle shows Jump + Fire, Cross shows nothing
	let ops = hc::plan(t, &b, 0, 0);
	let texts: Vec<String> = ops.iter().filter_map(|o| if let Op::Text(l) = o { Some(l.text.clone()) } else { None }).collect();
	assert!(texts.iter().any(|x| x.contains("Jump")), "{texts:?}");
	assert!(!texts.iter().any(|x| x.contains("wrench") || x.contains("Menu") || x.contains("Map")), "unchanged labels stay: {texts:?}");
	// d-pad up gets the wrench: extra line.
	let up = t.index("up").unwrap();
	b.1[up][1] = Some(10);
	let w = t.index("wrench").unwrap();
	b.1[w][1] = Some(10);
	let texts: Vec<String> = hc::plan(t, &b, 0, 0).iter().filter_map(|o| if let Op::Text(l) = o { Some(l.text.clone()) } else { None }).collect();
	assert!(texts.iter().any(|x| x.starts_with("D-Up:") && x.contains("OmniWrench")), "{texts:?}");
	let _ = with_only(w, 3);
}

#[test]
fn picture_files_of_the_real_art() {
	if !fixture() {
		return;
	}
	let iso = std::env::var("RAC_HELPCTL_ISO").unwrap_or_default(); // the PAL disc; unset = skipped (no machine paths in the release tree)
	let out = std::env::var("RAC_HELPCTL_OUT").unwrap_or_default(); // an existing folder for the pictures; unset = skipped
	if !std::path::Path::new(&iso).exists() || !std::path::Path::new(&out).exists() {
		return; // only on the dev machine: the picture files are for a human to look at
	}
	use std::io::{Read, Seek, SeekFrom};
	ka::install();
	let t = kbm::table();
	let mut f = std::fs::File::open(iso).unwrap();
	let mut b = Binds::defaults(t);
	let id = |n: &str| t.index(n).unwrap();
	b.1[id("jump")][0] = Some(1);
	b.1[id("fire")][0] = Some(0);
	b.1[id("up")][0] = None;
	b.1[id("wrench")][1] = Some(10);
	b.1[id("quick_select")][1] = Some(13);
	b.1[id("look2")][0] = Some(8);
	b.1[id("crouch2")][0] = Some(9);
	b.1[id("map")][0] = Some(14);
	b.1[id("pause")][0] = Some(7);
	b.1[id("l3")][0] = Some(12);
	for s in 0..6usize {
		let mut halves = Vec::new();
		for base in [3736u64, 3934] {
			let mut buf = vec![0u8; 33 * 2048];
			f.seek(SeekFrom::Start((base + 33 * s as u64) * 2048)).unwrap();
			f.read_exact(&mut buf).unwrap();
			halves.push(buf);
		}
		for (k, h) in halves.iter().enumerate() {
			assert_eq!(hc::fnv(&h[0x20..0x20 + 0x10400]), HALVES.iter().find(|e| e.1 as usize == s && e.2 as usize == k).unwrap().0);
		}
		for (name, bb) in [("heavy", b), ("def", Binds::defaults(t))] {
			let ops = hc::plan(t, &bb, s, lang_of(s));
			let mut planes: Vec<Vec<u8>> = halves.iter().map(|h| h[0x420..0x420 + 0x10000].to_vec()).collect();
			let pals: Vec<Vec<u8>> = halves.iter().map(|h| h[0x20..0x420].to_vec()).collect();
			let (a, c) = planes.split_at_mut(1);
			hc::paint([&mut a[0], &mut c[0]], [&pals[0], &pals[1]], &ops);
			let mut ppm = b"P6\n512 256\n255\n".to_vec();
			for y in 0..256usize {
				for x in 0..512usize {
					let (h, xx) = (x >> 8, x & 255);
					let i = planes[h][y * 256 + xx] as usize;
					let sl = (i & 0xe7) | ((i & 8) << 1) | ((i & 0x10) >> 1);
					ppm.extend_from_slice(&pals[h][sl * 4..sl * 4 + 3]);
				}
			}
			std::fs::write(format!("{out}/helpctl_s{s}_{name}.ppm"), ppm).unwrap();
		}
	}
}

#[test]
fn wrap_breaks_at_hyphens_and_spaces() {
	use super::help_ctl_font::Face;
	if !fixture() {
		return;
	}
	let l = tx::wrap_for_test(Face::Packed, "Erste-Person-Zielsicht", 60.0).unwrap();
	assert!(l.len() >= 2 && l.concat() == "Erste-Person-Zielsicht", "{l:?}");
	assert_eq!(tx::wrap_for_test(Face::Normal, "Jump", 100.0).unwrap(), vec!["Jump"]);
	assert!(tx::wrap_for_test(Face::Normal, "Schnellauswahl", 20.0).is_none());
}

/// The true worst case: every bindable controller button carries a non-default set of gameplay actions (many shifts of a
/// round-robin spread over both controller slots): nothing is cut short in any of the six picture slots, and every extra button's
/// name is on the picture.
#[test]
fn worst_case_every_button_remapped_drops_nothing() {
	if !fixture() {
		return;
	}
	ka::install();
	let t = kbm::table();
	let acts: Vec<usize> = t.actions.iter().enumerate().filter(|(_, a)| a.pad_ok && a.layer == Layer::Game).map(|(i, _)| i).collect();
	for shift in 0..16usize {
		let mut b = Binds::defaults(t);
		for slots in b.1.iter_mut() {
			*slots = Default::default();
		}
		for (n, &a) in acts.iter().enumerate() {
			b.1[a][0] = Some(((n + shift) % 16) as u8);
			b.1[a][1] = Some(((n * 3 + shift * 5 + 7) % 16) as u8);
		}
		let by = hc::by_button(t, &b, false);
		for s in 0..6 {
			let lang = lang_of(s);
			let (ops, lost) = hc::plan_counted(t, &b, s, lang);
			assert_eq!(lost, 0, "slot {s} shift {shift}");
			// The game's panel crops the picture: every drawn line lies inside the visible columns.
			for o in &ops {
				if let Op::Text(l) = o {
					let w = tx::text_width(l.role, &l.text).ceil() as i32;
					assert!(l.x >= hc::VIS_X.0 as i32 - 2 && l.x + w <= hc::VIS_X.1 as i32 + 2, "slot {s} shift {shift}: {:?} at x {} w {w} leaves the visible columns", l.text, l.x);
				}
			}
			let texts: Vec<String> = ops.iter().filter_map(|o| if let Op::Text(l) = o { Some(l.text.clone()) } else { None }).collect();
			for name_btn in [(10u8, "D-Up"), (11, "D-Down"), (12, "D-Left"), (13, "D-Right"), (8, "L3"), (9, "R3")] {
				if !by[name_btn.0 as usize].is_empty() {
					assert!(texts.iter().any(|x| x.starts_with(&format!("{}:", name_btn.1)) || x.starts_with(&format!("{} x", name_btn.1)) || x.contains(&format!("; {}", name_btn.1))), "slot {s} shift {shift}: {} missing in {texts:?}", name_btn.1);
				}
			}
		}
	}
}

/// Pictures of the font proof: unchanged labels of the real art (top) against the same words drawn by our code (bottom), 6x, as PPM.
#[test]
fn font_proof_picture() {
	if !fixture() {
		return;
	}
	let out = std::env::var("RAC_HELPCTL_OUT").unwrap_or_default(); // an existing folder for the pictures; unset = skipped
	if !std::path::Path::new(DISC).exists() || !std::path::Path::new(&out).exists() {
		return;
	}
	use std::io::{Read, Seek, SeekFrom};
	use super::help_ctl_font::Face;
	// (slot, half-picture x range is implicit) label rects (x, y, w, h) and their words, from HELP_CTL_DATA.md.
	let cases: [(usize, &[([u16; 4], &str)]); 2] = [
		(0, &[([368, 170, 40, 13], "Jump"), ([294, 29, 110, 13], "Swing wrench"), ([344, 84, 99, 12], "Quick select"), ([128, 214, 76, 13], "Walk/Run"), ([239, 215, 97, 11], "Look around")]),
		(3, &[([343, 172, 73, 14], "Springen"), ([263, 20, 188, 13], "Omni-Knochen-Angriff"), ([335, 83, 127, 11], "Schnellauswahl"), ([66, 217, 121, 14], "Gehen/Rennen"), ([217, 219, 120, 10], "Herumschauen")]),
	];
	let mut f = std::fs::File::open(DISC).unwrap();
	let mut rows: Vec<Vec<[u8; 3]>> = Vec::new();
	for (s, labels) in cases {
		let mut halves = Vec::new();
		for base in [3736u64, 3934] {
			let mut buf = vec![0u8; 33 * 2048];
			f.seek(SeekFrom::Start((base + 33 * s as u64) * 2048)).unwrap();
			f.read_exact(&mut buf).unwrap();
			halves.push(buf);
		}
		let pals: Vec<Vec<u8>> = halves.iter().map(|h| h[0x20..0x420].to_vec()).collect();
		let orig: Vec<Vec<u8>> = halves.iter().map(|h| h[0x420..0x420 + 0x10000].to_vec()).collect();
		let rgb = |planes: &[Vec<u8>], x: usize, y: usize| {
			let (h, xx) = (x >> 8, x & 255);
			let i = planes[h][y * 256 + xx] as usize;
			let sl = (i & 0xe7) | ((i & 8) << 1) | ((i & 0x10) >> 1);
			[pals[h][sl * 4], pals[h][sl * 4 + 1], pals[h][sl * 4 + 2]]
		};
		for (r, text) in labels {
			let mut mine = orig.clone();
			let ops = vec![Op::Clear(*r), Op::Text(tx::place_one(Face::Normal, text, [r[0].saturating_sub(40), r[1], r[2] + 80, r[3]], [r[0], r[1], r[2], r[3]]).remove(0))];
			let (a, c) = mine.split_at_mut(1);
			hc::paint([&mut a[0], &mut c[0]], [&pals[0], &pals[1]], &ops);
			for pl in [&orig, &mine] {
				for yy in 0..r[3] as usize + 8 {
					rows.push((0..r[2] as usize + 80).map(|xx| rgb(pl, (r[0] as usize + xx).saturating_sub(40).min(511), (r[1] as usize + yy).saturating_sub(3).min(255))).collect());
				}
			}
			rows.push(vec![[255, 0, 0]; r[2] as usize + 80]);
		}
	}
	let w = rows.iter().map(|r| r.len()).max().unwrap();
	let k = 5usize;
	let mut ppm = format!("P6
{} {}
255
", w * k, rows.len() * k).into_bytes();
	for r in &rows {
		for _ in 0..k {
			for x in 0..w * k {
				ppm.extend_from_slice(&r.get(x / k).copied().unwrap_or([0, 0, 0]));
			}
		}
	}
	std::fs::write(format!("{out}/font_proof.ppm"), ppm).unwrap();
}

/// Every character of every word the patch can draw has a real glyph cell of the game font (nothing is skipped silently), and the accented
/// letters carry retail's accent cell (inked rows above the base letter's x-height).
#[test]
fn every_drawable_character_has_a_glyph_cell() {
	if !fixture() {
		return;
	}
	use super::help_ctl_font as f;
	let font = f::get().unwrap();
	let mut ids: Vec<&str> = vec!["jump", "wrench", "fire", "quick_select", "look", "look2", "crouch", "crouch2", "pause", "map", "r3"];
	ka::install();
	ids.extend(ka::ACTIONS.iter().map(|a| a.id));
	let mut bad = Vec::new();
	let mut check = |what: &str, s: &str| {
		for c in s.chars() {
			if !font.knows(c) {
				bad.push(format!("{what}: {c:?} in {s:?}"));
			}
		}
	};
	for lang in 0..5 {
		check("gadget", &tx::gadget(lang));
		for id in &ids {
			for w in tx::main_words(id, lang) {
				check("main", &w);
				check("x2", &format!("{w} x2"));
			}
			let sh = tx::short(id, lang);
			check("short", &sh);
			check("entry", &format!("Select: {sh} x12, {sh}; ({sh}/{sh})"));
		}
	}
	for b in 0..16u8 {
		for ps in [false, true] {
			check("button", rac_engine::input::binds::pad_shown(b, ps));
		}
	}
	assert!(bad.is_empty(), "characters without a glyph cell: {bad:#?}");
	// Accents: each accented letter inks rows above its base letter's top.
	let top_row = |t: &str| {
		let (w, c) = font.line(f::Face::Normal, t);
		(0..(f::LINE_H + f::TOP) as usize).find(|&y| (0..w as usize).any(|x| c[y * w as usize + x] > 60))
	};
	for (acc, base) in [("ä", "a"), ("ö", "o"), ("ü", "u"), ("é", "e"), ("è", "e"), ("ê", "e"), ("à", "a"), ("ì", "i"), ("ò", "o"), ("ù", "u"), ("á", "a"), ("í", "i"), ("ó", "o"), ("ú", "u")] {
		assert!(top_row(acc) < top_row(base), "{acc} shows no accent");
	}
	for same in ["ñ", "ç", "ß", "¿", "¡"] {
		assert!(top_row(same).is_some(), "{same} draws nothing");
	}
	for cap in ["Ä", "Ö", "Ü", "É", "È", "À", "Ñ"] {
		assert!(top_row(cap).is_some_and(|y| y >= 0));
	}
}
