use super::badge::{badge, badge_with};
use super::icons::{build, decode, Layout, ICON_TEX};
use super::*;
use crate::overlay::FONT;

/// An 8-bit RGB PNG of `size` x `size` (a stored deflate block: the pixels are the test, not the compressor), pixel (x, y) = `f(x, y)`.
fn png(size: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
	let mut raw = Vec::new();
	for y in 0..size {
		raw.push(0);
		for x in 0..size {
			raw.extend(f(x, y));
		}
	}
	let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
	let mut chunk = |k: &[u8], d: &[u8]| {
		out.extend((d.len() as u32).to_be_bytes());
		out.extend(k);
		out.extend(d);
		out.extend([0u8; 4]);
	};
	let mut ih = Vec::new();
	ih.extend(size.to_be_bytes());
	ih.extend(size.to_be_bytes());
	ih.extend([8, 2, 0, 0, 0]);
	chunk(b"IHDR", &ih);
	let mut z = vec![0x78, 0x01, 1];
	z.extend((raw.len() as u16).to_le_bytes());
	z.extend((!(raw.len() as u16)).to_le_bytes());
	z.extend(&raw);
	z.extend([0u8; 4]);
	chunk(b"IDAT", &z);
	chunk(b"IEND", &[]);
	out
}

const S: u32 = 16;

fn leak<T>(v: T) -> &'static T {
	Box::leak(Box::new(v))
}

/// Three entries (the middle one hidden), icons of the colours red / green / blue, a yellow hidden icon, `size` pixels square.
fn set_of(size: u32) -> &'static IconSet {
	let colours: [[u8; 3]; 3] = [[200, 40, 40], [40, 200, 40], [40, 40, 200]];
	let pngs: Vec<&'static [u8]> = colours.iter().map(|c| leak(png(size, |_, _| *c)).as_slice()).collect();
	leak(IconSet { size, png: leak(pngs), hidden: leak(png(size, |_, _| [220, 220, 0])) })
}

/// The reason a build was refused (`Texture` has no Debug, so no `unwrap_err`).
fn refused(r: Result<crate::level::Texture, String>) -> String {
	r.err().expect("the build must be refused")
}

fn set() -> &'static IconSet {
	set_of(S)
}

static DEFS3: [Def; 3] = [
	Def { id: "a", name: "A", desc: "", grade: Grade::Bronze, hidden: false, max: 0, platinum: false },
	Def { id: "b", name: "B", desc: "", grade: Grade::Gold, hidden: true, max: 0, platinum: false },
	Def { id: "c", name: "C", desc: "", grade: Grade::Platinum, hidden: false, max: 0, platinum: false },
];

fn pack3(icons: Option<&'static IconSet>) -> AchPack {
	AchPack { game: "test", defs: &DEFS3, read: |_| None, icons }
}

#[test]
fn layout_holds_every_tile_once_inside_the_atlas() {
	for n in [0usize, 1, 3, 35] {
		let l = Layout::new(n, 128);
		assert_eq!(l.tiles(), 2 * n + 2);
		assert!(l.cols() * l.rows() >= l.tiles());
		let (w, h) = l.dims();
		let mut seen = std::collections::HashSet::new();
		let all = (0..n).flat_map(|i| [l.colour(i), l.grey(i)]).chain([l.hidden(), l.ring()]);
		for t in all {
			assert!(seen.insert(t), "tile {t} is used twice");
			assert!(t < l.tiles());
			let (x, y) = l.origin(t);
			assert!(x + 128 <= w && y + 128 <= h);
			let uv = l.uv(t);
			assert!(uv[0] > x as f32 / w as f32 && uv[2] < (x + 128) as f32 / w as f32, "u is inset from the tile edge");
			assert!(uv[1] > y as f32 / h as f32 && uv[3] < (y + 128) as f32 / h as f32, "v is inset from the tile edge");
		}
		assert_eq!(seen.len(), l.tiles());
	}
	// The R&C1 list: 35 trophies + hidden + 35 grey + ring = 72 tiles in 9 x 8.
	let l = Layout::new(35, 128);
	assert_eq!((l.cols(), l.rows(), l.dims()), (9, 8, (1152, 1024)));
}

#[test]
fn hidden_rule_picks_the_tile() {
	let l = Layout::new(3, S);
	assert_eq!(l.tile_for(1, true, true), l.colour(1), "unlocked hidden entry shows its own icon");
	assert_eq!(l.tile_for(1, false, true), l.hidden(), "locked hidden entry shows the hidden icon");
	assert_eq!(l.tile_for(0, false, false), l.grey(0), "locked entry is grey");
	assert_eq!(l.tile_for(0, true, false), l.colour(0));
}

#[test]
fn the_icon_tex_id_is_its_own() {
	use crate::viewer::{ITEM_PANEL_TEX, POPUP_PANEL_TEX, PORTRAIT_PANEL_TEX, RETAIL_FILTER_TEX, RETAIL_HD_TEX, RETAIL_OV_TEX};
	let others = [ITEM_PANEL_TEX, POPUP_PANEL_TEX, PORTRAIT_PANEL_TEX, RETAIL_FILTER_TEX, RETAIL_HD_TEX, RETAIL_OV_TEX, crate::splitview::SPLIT_TEX, FONT];
	assert!(others.iter().all(|&o| o != ICON_TEX));
	assert!(ICON_TEX > 0x1000_0000, "past every real HUD slot (`pass_hud` matches it before the slot clamp)");
}

#[test]
fn the_atlas_has_circles_grey_locked_icons_and_a_ring() {
	const BIG: u32 = 64;
	let t = build(set_of(BIG), 3).unwrap();
	let l = Layout::new(3, BIG);
	assert_eq!((t.w, t.h), l.dims());
	let at = |tile: usize, x: usize, y: usize| {
		let (ox, oy) = l.origin(tile);
		t.rgba[(oy + y) * t.w + ox + x]
	};
	let c = BIG as usize / 2;
	assert_eq!(at(l.colour(0), c, c), [200, 40, 40, 255], "icon colour kept, opaque in the middle");
	assert_eq!(at(l.colour(0), 0, 0)[3], 0, "corner is outside the circle");
	assert!(at(l.colour(2), c, 0)[3] > 240, "the edge midpoints stay inside the circle");
	let g = at(l.grey(1), c, c);
	assert!(g[0] == g[1] && g[1] == g[2] && g[3] == 255, "grey");
	assert!(g[0] < 200 && g[0] > 40, "grey is darker than the green's brightest channel and lighter than its darkest: {g:?}");
	assert_eq!(at(l.hidden(), c, c), [220, 220, 0, 255]);
	assert_eq!(at(l.ring(), c, c)[3], 0, "the ring is hollow");
	let r = c as f32 * 0.94;
	let (tl, br) = (at(l.ring(), (c as f32 - r * 0.7) as usize, (c as f32 - r * 0.7) as usize), at(l.ring(), (c as f32 + r * 0.7) as usize, (c as f32 + r * 0.7) as usize));
	assert!(tl[3] > 200 && br[3] > 200, "ring is opaque on its band: {tl:?} {br:?}");
	assert!(tl[0] > br[0], "lit from the top left");
	assert_eq!(at(l.ring(), 0, 0)[3], 0, "ring corner is clear");
}

#[test]
fn bad_sets_are_refused_with_a_reason() {
	let s = set();
	assert!(refused(build(s, 2)).contains("3 pictures for 2"));
	let wrong = leak(IconSet { size: 32, png: s.png, hidden: s.hidden });
	assert!(refused(build(wrong, 3)).contains("icon 0"));
	assert!(decode(b"not a png", S).is_err());
	let bad_hidden = leak(IconSet { size: S, png: s.png, hidden: b"junk" });
	assert!(refused(build(bad_hidden, 3)).contains("hidden icon"));
	let png16 = png(S, |_, _| [1, 2, 3]);
	assert_eq!(decode(&png16, S).unwrap().len(), (S * S) as usize);
	assert!(decode(&png16, 8).unwrap_err().contains("16x16"));
}

#[test]
fn a_badge_is_the_icon_inside_the_grade_ring() {
	let p = pack3(Some(set()));
	let l = Layout::new(3, S);
	let mut out = Vec::new();
	badge(&mut out, &p, 0, true, [100.0, 50.0, 64.0], 1.0);
	assert_eq!(out.len(), 2);
	assert!(out.iter().all(|q| q.tex == ICON_TEX));
	assert_eq!(out[0].uv, l.uv(l.colour(0)));
	assert_eq!(out[1].uv, l.uv(l.ring()));
	let g = Grade::Bronze.colour();
	assert_eq!(&out[1].colour[..3], &g, "ring = the grade colour");
	assert_eq!(out[1].rect, [100.0, 50.0, 64.0, 64.0]);
	let r = out[0].rect;
	assert!(r[0] > 100.0 && r[2] < 64.0 && (r[0] + r[2] / 2.0 - 132.0).abs() < 1e-3, "icon centred inside the ring");
}

#[test]
fn locked_badges_are_grey_or_hidden_and_dimmer() {
	let p = pack3(Some(set()));
	let l = Layout::new(3, S);
	let (mut grey, mut hid, mut open) = (Vec::new(), Vec::new(), Vec::new());
	badge(&mut grey, &p, 0, false, [0.0, 0.0, 50.0], 1.0);
	badge(&mut hid, &p, 1, false, [0.0, 0.0, 50.0], 1.0);
	badge(&mut open, &p, 1, true, [0.0, 0.0, 50.0], 1.0);
	assert_eq!(grey[0].uv, l.uv(l.grey(0)), "locked = grey tile");
	assert_eq!(hid[0].uv, l.uv(l.hidden()), "locked hidden = the hidden icon");
	assert_eq!(open[0].uv, l.uv(l.colour(1)), "unlocked hidden = its own icon");
	assert!(grey[0].colour[3] < open[0].colour[3] && grey[1].colour[3] < open[1].colour[3]);
}

#[test]
fn without_icons_the_cup_stays() {
	let p = pack3(None);
	let (mut out, mut dim) = (Vec::new(), Vec::new());
	badge(&mut out, &p, 2, true, [0.0, 0.0, 50.0], 1.0);
	badge(&mut dim, &p, 2, false, [0.0, 0.0, 50.0], 1.0);
	assert!(!out.is_empty() && out.iter().all(|q| q.tex == FONT), "cup quads use the built-in font slot");
	assert!(dim.iter().all(|q| q.colour[3] < 1.0));
	let mut none = Vec::new();
	badge(&mut none, &p, 9, true, [0.0, 0.0, 50.0], 1.0);
	assert!(none.is_empty(), "an index past the list draws nothing");
}

/// m7: a pack WITH icons whose atlas failed to build draws the cups (same as a pack without icons), never `ICON_TEX` quads that the GPU
/// would drop; while the atlas is fine, the same pack draws icon quads only.
#[test]
fn a_failed_atlas_falls_back_to_the_cup() {
	let p = pack3(Some(set()));
	assert!(!super::icons::atlas_failed(), "nothing in the unit tests builds the running game's atlas");
	for unlocked in [true, false] {
		let (mut ok, mut failed, mut plain) = (Vec::new(), Vec::new(), Vec::new());
		badge_with(&mut ok, &p, 0, unlocked, [0.0, 0.0, 50.0], 1.0, false);
		badge_with(&mut failed, &p, 0, unlocked, [0.0, 0.0, 50.0], 1.0, true);
		badge_with(&mut plain, &pack3(None), 0, unlocked, [0.0, 0.0, 50.0], 1.0, false);
		assert!(!ok.is_empty() && ok.iter().all(|q| q.tex == ICON_TEX), "atlas fine: icon quads (unlocked {unlocked})");
		assert!(!failed.is_empty() && failed.iter().all(|q| q.tex == FONT), "atlas failed: cup quads (unlocked {unlocked})");
		assert_eq!(failed.len(), plain.len(), "the same cup as a pack without icons");
		assert!(failed.iter().zip(&plain).all(|(a, b)| a.rect == b.rect && a.colour == b.colour));
	}
	let mut gone = Vec::new();
	badge_with(&mut gone, &p, 9, true, [0.0, 0.0, 50.0], 1.0, true);
	assert!(gone.is_empty(), "an index past the list still draws nothing");
}
