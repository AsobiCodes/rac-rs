//! The classic Help > Controls picture under a controller remap (v0.5.1, review R2-S4; data `HELP_CTL_DATA.md` option A).
//!
//! Retail page 0x16 shows two 256x256 8-bit images (item +0x48 / +0x4c, 32-byte header, palette at +0x20, indices at +0x420) with the
//! labels baked in. While the controller is remapped, the labels of the buttons whose gameplay actions differ from the defaults are
//! repainted IN RAM: the old words are filled with the half's background index and the new words are drawn into the label's free box
//! (`help_ctl_data.rs`, text and fitting `help_ctl_text.rs`) in white quantised to the half's own palette; actions on buttons the art
//! has no label for (d-pad, L3, R3) go into small extra lines. The images are known by a hash of palette + indices (same on the
//! PAL and USA disc); unknown art is left alone (logged once). Without a remap nothing is touched and the pristine bytes are put
//! back if they were patched. Leader lines stay.

use super::help_ctl_data::{Lab, HALVES, LABS, SLOTS};
use super::help_ctl_text::{self as tx, Line};
use super::mouse_menu;
use super::prompt_pad;
use crate::kbm_actions as ka;
use ee_oracle::mem::Mem;
use rac_engine::input::kbm::{self, Binds, Layer, Table};
use std::sync::Mutex;

/// Page id of the Controls picture page.
const PAGE_ID: u32 = 0x16;
/// Bytes hashed per half (palette + indices) and where they start in the file.
const HASH_AT: u32 = 0x20;
const HASH_LEN: usize = 0x10400;
/// Offset of the indices.
const PIX: u32 = 0x420;
/// Short names of the buttons that get extra lines (PHYS index, name).
const EXTRA: [(u8, &str); 6] = [(10, "D-Up"), (11, "D-Down"), (12, "D-Left"), (13, "D-Right"), (8, "L3"), (9, "R3")];
/// The group labels' button names.
const GROUP: [(u8, &str); 4] = [(4, "L1"), (14, "L2"), (5, "R1"), (15, "R2")];
/// Labels whose words start at the old label's left edge (the others are centred on it).
const LEFT: [&str; 2] = ["quick", "fire"];

/// What was patched: the two halves' buffers, the hashes after the patch, the pristine index bytes, the remap and language it was made for.
struct Patched {
	at: [u32; 2],
	after: [u64; 2],
	pristine: [Vec<u8>; 2],
	key: u64,
}

#[derive(Default)]
struct St {
	patched: Option<Patched>,
	/// Each message is logged once, on its own flag.
	logged_unknown: bool,
	logged_font: bool,
}

static ST: Mutex<St> = Mutex::new(St { patched: None, logged_unknown: false, logged_font: false });

pub fn fnv(b: &[u8]) -> u64 {
	let mut h = 0xcbf2_9ce4_8422_2325u64;
	for &x in b {
		h = (h ^ x as u64).wrapping_mul(0x100_0000_01b3);
	}
	h
}

/// The gameplay actions (table indices) on each physical button 0..16, controller slots 1 and 2.
pub fn by_button(t: &Table, b: &Binds, defaults: bool) -> [Vec<usize>; 16] {
	let d = Binds::defaults(t);
	let src = if defaults { &d } else { b };
	let mut out: [Vec<usize>; 16] = Default::default();
	for (i, a) in t.actions.iter().enumerate() {
		if !a.pad_ok || a.layer != Layer::Game {
			continue;
		}
		for p in src.1[i].iter().flatten() {
			if let Some(v) = out.get_mut(*p as usize) {
				if !v.contains(&i) {
					v.push(i);
				}
			}
		}
	}
	out
}

/// The words of one button's actions in the label wording: units to fit, and whether they may be joined by commas.
fn words(t: &Table, acts: &[usize], lang: usize, alone_fire: bool) -> Vec<String> {
	let mut out: Vec<String> = Vec::new();
	for &i in acts {
		let id = t.actions[i].id;
		let mut w = tx::main_words(id, lang);
		if id == "fire" && alone_fire && acts.len() == 1 {
			w.push(tx::gadget(lang));
		}
		for s in w {
			if !out.contains(&s) {
				out.push(s);
			}
		}
	}
	out
}

/// The lines to draw for label `lab` under `cur` / `def` (None = unchanged, keep the art): units, `join` flag.
fn label_units(t: &Table, lab: &Lab, cur: &[Vec<usize>; 16], def: &[Vec<usize>; 16], lang: usize) -> Option<(Vec<String>, bool, usize)> {
	let btns: Vec<u8> = lab.btn.iter().copied().filter(|&b| b < 16).collect();
	if btns.iter().all(|&b| norm(&cur[b as usize]) == norm(&def[b as usize])) {
		return None;
	}
	let name = |b: u8| GROUP.iter().find(|g| g.0 == b).map_or("", |g| g.1);
	if btns.len() == 1 {
		let acts = &cur[btns[0] as usize];
		let n = acts.len();
		return Some((words(t, acts, lang, true), true, n));
	}
	let (a, b) = (&cur[btns[0] as usize], &cur[btns[1] as usize]);
	if norm(a) == norm(b) {
		let mut u = words(t, a, lang, false);
		let tag = format!("({}/{})", name(btns[0]), name(btns[1]));
		if u.is_empty() {
			return Some((Vec::new(), false, 0));
		}
		u = vec![u.join(", ")];
		u.push(tag);
		return Some((u, false, a.len()));
	}
	let one = |b: u8, acts: &Vec<usize>| {
		let w = words(t, acts, lang, false).join(", ");
		format!("{}: {}", name(b), if w.is_empty() { "-".to_string() } else { w })
	};
	Some((vec![one(btns[0], a), one(btns[1], b)], false, a.len() + b.len()))
}

fn norm(v: &[usize]) -> Vec<usize> {
	let mut v = v.to_vec();
	v.sort_unstable();
	v
}

/// One drawing step: clear this rect, or draw this line.
pub enum Op {
	Clear([u16; 4]),
	Text(Line),
}

/// The steps that repaint the picture slot `slot` for the bindings `b` in language `lang` (empty when the picture needs no change).
pub fn plan(t: &Table, b: &Binds, slot: usize, lang: usize) -> Vec<Op> {
	plan_counted(t, b, slot, lang).0
}

/// Columns of the 512-wide picture the game's Help panel shows (it crops the picture at both sides; measured in game: text outside was cut).
pub const VIS_X: (u16, u16) = (28, 476);

/// `r` cut to the visible columns.
pub fn visible(r: [u16; 4]) -> [u16; 4] {
	let (x0, x1) = (r[0].max(VIS_X.0), (r[0] + r[2]).min(VIS_X.1));
	[x0, r[1], x1.saturating_sub(x0), r[3]]
}

/// `plan` and the number of entries that had to be cut short (the tests require 0).
pub fn plan_counted(t: &Table, b: &Binds, slot: usize, lang: usize) -> (Vec<Op>, usize) {
	let (cur, def) = (by_button(t, b, false), by_button(t, b, true));
	let (mut ops, mut lost) = (Vec::new(), 0);
	let mut queue: Vec<Vec<String>> = Vec::new();
	let ps = super::glyphs::style() == 0;
	for lab in LABS[slot].iter() {
		let Some((units, join, count)) = label_units(t, lab, &cur, &def, lang) else { continue };
		for r in lab.clear {
			ops.push(Op::Clear(*r));
		}
		if units.is_empty() {
			continue;
		}
		let left = LEFT.contains(&super::help_ctl_data::KEYS[lab.key as usize]);
		let first = lab.btn.iter().filter(|&&b| b < 16).find_map(|&b| cur[b as usize].first().copied());
		let first_name = first.map(|i| tx::short(t.actions[i].id, lang)).unwrap_or_default();
		let fallback = format!("{first_name} x{count}");
		match tx::layout(&units, join, visible(lab.free), lab.clear[0], left) {
			Some(lines) => ops.extend(lines.into_iter().map(Op::Text)),
			// Too narrow for the words (Map / Menu), or too many of them: "x N" form in the label's box, else an extra line that names the button.
			None => match tx::layout(std::slice::from_ref(&fallback), false, visible(lab.free), lab.clear[0], left) {
				Some(lines) if lines.len() == 1 => ops.extend(lines.into_iter().map(Op::Text)),
				_ => {
					let names: Vec<String> = lab.btn.iter().filter(|&&b| b < 16).map(|&b| rac_engine::input::binds::pad_shown(b, ps).to_string()).collect();
					let acts: Vec<usize> = lab.btn.iter().filter(|&&b| b < 16).flat_map(|&b| cur[b as usize].clone()).collect();
					queue.push(entry_forms(&names.join("/"), &short_words(t, &acts, lang), &first_name, count));
				}
			},
		}
	}
	// Buttons with no label in the art.
	for (p, name) in EXTRA {
		let acts = &cur[p as usize];
		if !acts.is_empty() && norm(acts) != norm(&def[p as usize]) {
			let first = tx::short(t.actions[acts[0]].id, lang);
			queue.push(entry_forms(name, &short_words(t, acts, lang), &first, acts.len()));
		}
	}
	let slots: Vec<[u16; 4]> = SLOTS[slot].iter().map(|r| visible(*r)).collect();
	let (lines, cut) = super::help_ctl_extras::place(queue, &slots);
	ops.extend(lines.into_iter().map(Op::Text));
	lost += cut;
	(ops, lost)
}

/// The wordings of an extra line, longest first: all names, the first name + " xN", its first word + " xN", the button + " xN".
fn entry_forms(prefix: &str, words: &str, first: &str, n: usize) -> Vec<String> {
	let word = first.split(' ').next().unwrap_or(first);
	vec![format!("{prefix}: {words}"), format!("{prefix}: {first} x{n}"), format!("{prefix}: {word} x{n}"), format!("{prefix} x{n}")]
}

/// The actions' short names joined by commas (the extra lines).
pub fn short_words(t: &Table, acts: &[usize], lang: usize) -> String {
	acts.iter().map(|&i| tx::short(t.actions[i].id, lang)).collect::<Vec<_>>().join(", ")
}

/// Nearest-neutral palette index for each coverage level 0..=16 (0 = the background).
struct Pal {
	bg: u8,
	lut: [u8; 17],
}

fn palette(pal: &[u8], bg_idx: u8) -> Pal {
	let col = |i: usize| {
		let s = (i & 0xe7) | ((i & 8) << 1) | ((i & 0x10) >> 1);
		[pal[s * 4] as i32, pal[s * 4 + 1] as i32, pal[s * 4 + 2] as i32]
	};
	let bg = col(bg_idx as usize);
	let neutral: Vec<usize> = (0..256).filter(|&i| i == bg_idx as usize || (col(i)[0] - col(i)[1]).abs() <= 10 && (col(i)[1] - col(i)[2]).abs() <= 14).collect();
	let mut lut = [bg_idx; 17];
	for (k, slot) in lut.iter_mut().enumerate().skip(1) {
		let target: Vec<i32> = (0..3).map(|c| bg[c] + (253 - bg[c]) * k as i32 / 16).collect();
		*slot = neutral.iter().copied().min_by_key(|&i| (0..3).map(|c| (col(i)[c] - target[c]).pow(2)).sum::<i32>()).unwrap_or(bg_idx as usize) as u8;
	}
	Pal { bg: bg_idx, lut }
}

/// The picture's two halves as editable index planes (256x256 each) plus their palettes; applies `ops`.
pub fn paint(planes: [&mut [u8]; 2], pals: [&[u8]; 2], ops: &[Op]) {
	let [l, r] = planes;
	let ps = [palette(pals[0], l[0]), palette(pals[1], r[0])];
	let mut planes = [l, r];
	let mut put = |x: i32, y: i32, f: &dyn Fn(&Pal) -> Option<u8>| {
		if (0..512).contains(&x) && (0..256).contains(&y) {
			let h = (x >> 8) as usize;
			if let Some(v) = f(&ps[h]) {
				planes[h][(y * 256 + (x & 255)) as usize] = v;
			}
		}
	};
	for op in ops {
		match op {
			Op::Clear(c) => {
				for y in c[1]..c[1] + c[3] {
					for x in c[0]..c[0] + c[2] {
						put(x as i32, y as i32, &|p| Some(p.bg));
					}
				}
			}
			Op::Text(line) => {
				let Some(f) = super::help_ctl_font::get() else { continue };
				let (w, cov) = f.line(line.role, &line.text);
				for (i, &c) in cov.iter().enumerate() {
					let lv = (c as u32 * 16 + 127) / 255;
					if lv > 0 {
						put(line.x + (i as u32 % w) as i32, line.y + (i as u32 / w) as i32 - super::help_ctl_font::TOP as i32, &|p| Some(p.lut[lv as usize]));
					}
				}
			}
		}
	}
}

fn key_of(b: &Binds, lang: usize) -> u64 {
	let mut s = Vec::new();
	for slots in b.1.iter() {
		for p in slots {
			s.push(p.map_or(255, |p| p));
		}
	}
	s.push(lang as u8);
	fnv(&s)
}

/// The page's image buffers (both) and whether retail has the images marked loaded (item +0x44 >= 4: it draws them from now on).
fn find_buffers(mem: &mut Mem, level: u32) -> Option<([u32; 2], bool)> {
	if mem.rd32(super::disc::va(super::pcctl::MENU_MODE)) != mouse_menu::MODE {
		return None;
	}
	let ram = &mem.ram;
	let rd = |a: u32| -> u32 {
		let i = (a & 0x01ff_ffff) as usize;
		ram.get(i..i + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
	};
	let snap = mouse_menu::read(&rd, level, mouse_menu::MODE)?;
	if snap.id != PAGE_ID || snap.front {
		return None;
	}
	snap.items.iter().find_map(|it| {
		let (a, b) = (rd(it.addr + 0x48), rd(it.addr + 0x4c));
		let ok = |p: u32| p != 0 && p & !0x01ff_ffff == 0 && rd(p) == u32::from_le_bytes(*b"2FIP");
		(ok(a) && ok(b)).then_some(([a, b], rd(it.addr + 0x44) >= 4))
	})
}

fn half<'a>(mem: &'a Mem, at: u32) -> &'a [u8] {
	let i = (at & 0x01ff_ffff) as usize;
	// A pointer within the half's size of the end of RAM gives an empty slice, never a panic (no hash match, so nothing is patched).
	mem.ram.get(i + HASH_AT as usize..i + HASH_AT as usize + HASH_LEN).unwrap_or(&[])
}

/// Worker, each field after the other page syncs (`pcext::sync`, in a level): keeps the picture right for the controller layout.
pub fn sync(mem: &mut Mem, level: u32) {
	let remapped = prompt_pad::remapped();
	let Ok(mut st) = ST.lock() else { return };
	if !remapped && st.patched.is_none() {
		return;
	}
	let Some((at, loaded)) = find_buffers(mem, level) else {
		// The page went away: put the retail pixels back if the buffers still hold exactly our patch (they may be shown again unloaded).
		if let Some(p) = st.patched.take() {
			if (0..2).all(|k| p.at[k] as usize + HASH_AT as usize + HASH_LEN <= mem.ram.len() && fnv(half(mem, p.at[k])) == p.after[k]) {
				for k in 0..2 {
					let base = (p.at[k] & 0x01ff_ffff) as usize + PIX as usize;
					mem.ram[base..base + 0x10000].copy_from_slice(&p.pristine[k]);
				}
			}
		}
		return;
	};
	apply(mem, &mut st, at, remapped, loaded);
}

/// Font 1 (the menu font) from the level's sheet and retail's glyph table (set by the host from the disc); None until both are known.
fn game_font() -> Option<()> {
	let patch = super::glyphs::patch()?;
	// The host sets the sheet (`glyphs::set_patch_for`) and then the glyph table (`hudart::load_fonts`) on each level load: the cache key
	// holds both, so a font made from the new sheet and the previous level's table is never kept.
	let table = super::help_ctl_font::table();
	let key = (std::sync::Arc::as_ptr(&patch) as usize) ^ (fnv(&table.iter().flat_map(|&(u, v, dy, adv)| [u, v, dy as u8, adv]).collect::<Vec<u8>>()) as usize);
	super::help_ctl_font::install(key, || {
		let mut alpha = [0u8; 256];
		for (a, c) in alpha.iter_mut().zip(patch.palettes.first()?.iter()) {
			*a = c[3];
		}
		super::help_ctl_font::Font::new(patch.sheets.first()?, &alpha, &table)
	})
	.map(|_| ())
}

fn apply(mem: &mut Mem, st: &mut St, at: [u32; 2], remapped: bool, loaded: bool) {
	ka::install();
	let lang = super::menu_model::text::lang_index(mem.rd32(super::menu_model::addrs::LANG_WORD));
	let (t, b) = (kbm::table(), kbm::binds());
	let key = key_of(&b, lang);
	let h = [fnv(half(mem, at[0])), fnv(half(mem, at[1]))];
	if let Some(p) = st.patched.as_ref() {
		if p.at == at && p.after == h {
			if remapped && p.key == key {
				return;
			}
			// Remap changed or ended while the page is up: back to the retail pixels first.
			let p = st.patched.take().unwrap();
			for k in 0..2 {
				let base = (at[k] & 0x01ff_ffff) as usize + PIX as usize;
				mem.ram[base..base + 0x10000].copy_from_slice(&p.pristine[k]);
			}
		} else {
			st.patched = None; // reloaded: the buffers hold something else now
		}
	}
	if !remapped {
		return;
	}
	let known = |x: u64, half: u8| HALVES.iter().find(|e| e.0 == x && e.2 == half).map(|e| e.1);
	let (Some(s0), Some(s1)) = (known(fnv_of(mem, at[0]), 0), known(fnv_of(mem, at[1]), 1)) else {
		// Not marked loaded yet: the buffers may still be filling (they are patched the moment they hold the whole art, before retail's first draw).
		if loaded && !st.logged_unknown {
			st.logged_unknown = true;
			eprintln!("retail: help controls picture: unknown art (hash {:016x} {:016x}), left as it is", h[0], h[1]);
		}
		return;
	};
	if s0 != s1 {
		return;
	}
	if game_font().is_none() {
		if !st.logged_font {
			st.logged_font = true;
			eprintln!("retail: help controls picture: the game font is not available, picture left as it is");
		}
		return;
	}
	let ops = plan(t, &b, s0 as usize, lang);
	let pristine = [0, 1].map(|k| {
		let base = (at[k] & 0x01ff_ffff) as usize + PIX as usize;
		mem.ram[base..base + 0x10000].to_vec()
	});
	let pals = [0, 1].map(|k| {
		let base = (at[k] & 0x01ff_ffff) as usize + HASH_AT as usize;
		mem.ram[base..base + 1024].to_vec()
	});
	let mut planes = pristine.clone();
	{
		let [a, c] = &mut planes;
		paint([a, c], [&pals[0], &pals[1]], &ops);
	}
	for k in 0..2 {
		let base = (at[k] & 0x01ff_ffff) as usize + PIX as usize;
		mem.ram[base..base + 0x10000].copy_from_slice(&planes[k]);
	}
	let after = [fnv(half(mem, at[0])), fnv(half(mem, at[1]))];
	st.patched = Some(Patched { at, after, pristine, key });
}

fn fnv_of(mem: &Mem, at: u32) -> u64 {
	fnv(half(mem, at))
}

/// Test hook: forget what was patched.
#[cfg(test)]
pub(super) fn reset() {
	*ST.lock().unwrap() = St::default();
}
