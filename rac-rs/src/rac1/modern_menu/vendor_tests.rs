//! Tests of the modern vendor page (`vendor.rs`, `vendor_text.rs`): rows, prices and chips from the game's facts, which buttons exist
//! when (buy, refill, the prompt's four), when the page replaces retail's picture (`wanted`), and that every word fits its box and is
//! drawable in all five languages. The host's mouse is in `vendor_host/tests.rs`; the facts reader in `shop_view.rs`.
use super::hide::{hidden_in, Screen};
use super::icons;
use super::vendor::*;
use super::vendor_text::{self as t, commas};
use crate::retail::shop_view::tests::{Ram, STOCK};
use crate::retail::shop_view::{ShopRow, ShopView};
use rac_engine::ui::font::{FontMeasure, FontSet};
use rac_engine::ui::input::{Device, GlyphSet};
use rac_engine::ui::kit::text::run_width;
use rac_engine::ui::prim::{Prim, Rect};

fn names() -> Vec<String> {
	vec!["Bomb Glove".into(), "Pyrocitor Ammo".into(), "Blaster Ammo".into()]
}

/// The STOCK shop (a 7,500-bolt weapon, ammo 22 / 40 at 5, ammo 150 / 240 at 1) with `bolts`, `sel` and `stage`.
fn view(bolts: u32, sel: u32, stage: u32) -> ShopView {
	Ram::shop(&STOCK, &[(10, 22), (16, 150)], bolts, sel, stage).view(true).expect("a shop")
}

fn page(v: &ShopView, lang: usize, device: Device) -> Built {
	build(&Facts { view: v, names: &names(), focus: None, buyall: false, scroll: 0, lang, device, glyphs: GlyphSet::PlayStation }, &FontMeasure).expect("the page builds")
}

fn texts(b: &Built) -> Vec<String> {
	b.prims.iter().filter_map(|p| if let Prim::Text(r) = p { Some(r.text.clone()) } else { None }).collect()
}

fn has(b: &Built, s: &str) -> bool {
	texts(b).iter().any(|t| t == s)
}

fn centre(r: rac_engine::ui::prim::Rect) -> (f32, f32) {
	(r.cx(), r.cy())
}

#[test]
fn rows_carry_their_names_prices_and_chips() {
	let b = page(&view(4200, 0, 0), 0, Device::Pad);
	assert_eq!(b.rows.len(), 3);
	assert_eq!((b.sel, b.count, b.prompt), (0, 3, false));
	for s in ["Bomb Glove", "Pyrocitor Ammo", "Blaster Ammo", "7,500 Bolts", "Ammo 22 / 40 \u{b7} 5 Bolts each", "Ammo 150 / 240 \u{b7} 1 Bolt each", "4,200", "GADGETRON VENDOR"] {
		assert!(has(&b, s), "text {s:?} on the page: {:?}", texts(&b));
	}
	// A weapon the bolts do not reach and ammo that fits: "Need bolts" once, "Owned" twice.
	let count = |s: &str| texts(&b).iter().filter(|t| *t == s).count();
	assert_eq!((count("Need bolts"), count("Owned"), count("New"), count("Full")), (1, 2, 0, 0));
	let rich = page(&view(9000, 0, 0), 0, Device::Pad);
	assert!(has(&rich, "New") && !has(&rich, "Need bolts"), "a weapon the bolts reach is New");
	let full = page(&Ram::shop(&STOCK, &[(10, 40), (16, 240)], 9000, 1, 0).view(true).unwrap(), 0, Device::Pad);
	assert_eq!(texts(&full).iter().filter(|t| *t == "Full").count(), 2, "ammo at the most is Full");
	assert_eq!(commas(1234567), "1,234,567");
}

#[test]
fn the_list_shows_seven_rows_under_buy_all_and_scrolls_with_the_selection() {
	let rows = (0..12).map(|i| ShopRow { item: i, ammo: false, price: 100, ..Default::default() }).collect();
	let mut v = ShopView { phase: 1, bolts: 5000, rows, sel: 0, ..Default::default() };
	let names: Vec<String> = (0..12).map(|i| format!("Item {i}")).collect();
	let make = |v: &ShopView| build(&Facts { view: v, names: &names, focus: None, buyall: false, scroll: 0, lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation }, &FontMeasure).unwrap();
	let ids = |b: &Built| b.rows.iter().map(|r| r.0).collect::<Vec<_>>();
	assert_eq!(ids(&make(&v)), (0..7).collect::<Vec<_>>());
	v.sel = 11;
	assert_eq!(ids(&make(&v)), (5..12).collect::<Vec<_>>(), "the last row is in view");
	assert!(make(&v).rows.last().unwrap().1.bottom() <= LIST.bottom() - 8.0, "the seventh row fits in the panel");
	v.sel = 9;
	let b = make(&v);
	assert!(ids(&b).contains(&9) && b.sel == 9);
	let (x, y) = centre(b.rows.iter().find(|r| r.0 == 9).unwrap().1);
	assert_eq!(b.hit(x, y), Hit::Row(9), "a click gives the row's real index");
	let f = Facts { view: &v, names: &names, focus: Some(2), buyall: false, scroll: 0, lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation };
	assert_eq!(build(&f, &FontMeasure).unwrap().sel, 2, "the player's own focus wins for its moment");
}

#[test]
fn a_hover_after_the_wheel_keeps_the_list_where_it_is() {
	// User 2026-11-04: the wheel walked the list down, then a mouse move sent it back to the top.
	let rows = (0..12).map(|i| ShopRow { item: i, ammo: false, price: 100, ..Default::default() }).collect();
	let v = ShopView { phase: 1, bolts: 5000, rows, sel: 9, ..Default::default() };
	let names: Vec<String> = (0..12).map(|i| format!("Item {i}")).collect();
	let make = |focus: Option<usize>, scroll: usize| {
		build(&Facts { view: &v, names: &names, focus, buyall: false, scroll, lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation }, &FontMeasure).unwrap()
	};
	// The wheel walked down to row 9 from the top: the window follows it (rows 3..10).
	let b = make(None, 0);
	assert_eq!((b.first, b.rows.first().map(|r| r.0)), (3, Some(3)));
	// The pointer rests on the window's second row (row 4): the hover chooses row 4, and the window stays, frame after frame.
	let (x, y) = centre(b.rows[1].1);
	let mut first = b.first;
	for _ in 0..5 {
		let Hit::Row(i) = make(Some(4), first).hit(x, y) else { panic!("the pointer is on a row") };
		assert_eq!(i, 4, "the row under a still pointer stays the same row");
		first = make(Some(i), first).first;
		assert_eq!(first, 3, "a row inside the window does not scroll the list");
	}
	// Leaving the window scrolls as little as shows the selection.
	assert_eq!(make(Some(11), 3).first, 5);
	assert_eq!(make(Some(1), 3).first, 1);
}

#[test]
fn the_first_row_moves_only_to_show_the_selection() {
	assert_eq!(first_row(0, 0, 12), 0);
	assert_eq!(first_row(0, 6, 12), 0);
	assert_eq!(first_row(0, 7, 12), 1);
	assert_eq!(first_row(3, 4, 12), 3);
	assert_eq!(first_row(3, 2, 12), 2);
	assert_eq!(first_row(9, 11, 12), 5, "never past the last full window");
	assert_eq!(first_row(4, 2, 5), 0, "a short list never scrolls");
}

#[test]
fn the_buy_button_exists_only_when_the_purchase_would_go_through() {
	assert!(page(&view(4200, 0, 0), 0, Device::Pad).buy.is_none(), "7,500 for a weapon with 4,200 bolts");
	assert!(page(&view(7500, 0, 0), 0, Device::Pad).buy.is_some(), "exactly the price");
	assert!(page(&view(4200, 1, 0), 0, Device::Pad).buy.is_some(), "some ammo fits");
	assert!(page(&view(4, 1, 0), 0, Device::Pad).buy.is_none(), "not one round of 5 bolts");
	let full = Ram::shop(&STOCK, &[(10, 40)], 9000, 1, 0).view(true).unwrap();
	let b = page(&full, 0, Device::Pad);
	assert!(b.buy.is_none() && has(&b, "Ammo is full"), "full ammo cannot be bought");
	let b = page(&view(4200, 0, 0), 0, Device::Pad);
	assert!(has(&b, "Not enough bolts"), "the disabled button says why");
	let (x, y) = centre(page(&view(4200, 1, 0), 0, Device::Pad).buy.unwrap());
	assert_eq!(page(&view(4200, 1, 0), 0, Device::Pad).hit(x, y), Hit::Buy);
}

#[test]
fn buy_all_ammo_is_always_the_first_row_and_follows_the_games_hook() {
	let b = page(&view(4200, 0, 0), 0, Device::Pad);
	let r = b.refill.expect("both ammo rows can take rounds");
	assert!(has(&b, "Buy all ammo") && has(&b, "Ammo +108 \u{b7} 180 Bolts"), "18 x 5 + 90 x 1 = 180: {:?}", texts(&b));
	assert_eq!(b.hit(r.cx(), r.cy()), Hit::Refill);
	assert!(b.rows.iter().all(|s| s.1.y > r.bottom()) && r.y < LIST.y + 20.0, "at the top of the list, above every stock row");
	assert!(!texts(&b).iter().any(|s| s.contains("Refill")), "no refill hint along the bottom any more");
	let poor = page(&view(100, 0, 0), 0, Device::Pad);
	assert!(poor.refill.is_some() && has(&poor, "Ammo +28 \u{b7} 100 Bolts"), "what the bolts reach, row by row");
	let broke = page(&view(0, 1, 0), 0, Device::Pad);
	assert!(broke.refill.is_some() && has(&broke, "Buy all ammo"), "no bolts: the row stays choosable (its card says why)");
	assert_eq!(texts(&broke).iter().filter(|s| *s == "Not enough bolts").count(), 2, "the row and the detail card say why");
	let none = Ram::shop(&STOCK, &[(10, 40), (16, 240)], 9000, 0, 0).view(true).unwrap();
	let full = page(&none, 0, Device::Pad);
	assert!(full.refill.is_some() && has(&full, "Buy all ammo") && has(&full, "Ammo is full"), "nothing to fill, still choosable");
	assert_eq!(full.hit(full.refill.unwrap().cx(), full.refill.unwrap().cy()), Hit::Refill);
	let off = Ram::shop(&STOCK, &[(10, 22)], 4200, 0, 0).view(false).unwrap();
	assert!(page(&off, 0, Device::Pad).refill.is_none(), "Square buys nothing (the option off in the Classic style)");
	let prompt = page(&view(4200, 0, 2), 0, Device::Pad);
	assert!(prompt.refill.is_none() && has(&prompt, "Buy all ammo"), "no refill under the prompt");
	let mut leaving = view(4200, 0, 0);
	leaving.leaving = true;
	assert!(page(&leaving, 0, Device::Pad).refill.is_none(), "the exit is pending");
}

fn focused_page(v: &ShopView, names: &[String], lang: usize) -> Built {
	build(&Facts { view: v, names, focus: None, buyall: true, scroll: 0, lang, device: Device::Keyboard, glyphs: GlyphSet::PlayStation }, &FontMeasure).expect("the page builds")
}

/// The accent highlight of a selected row: (rect) of every such rrect.
fn highlights(b: &Built) -> Vec<Rect> {
	b.prims.iter().filter_map(|p| if let Prim::RRect { rect, border: Some(_), radius, .. } = p { (*radius == 10.0 && rect.h == 60.0).then_some(*rect) } else { None }).collect()
}

#[test]
fn buy_all_ammo_takes_the_focus_like_a_row_with_its_own_card() {
	let v = view(4200, 0, 0);
	let b = focused_page(&v, &names(), 0);
	assert!(b.buyall && b.refill.is_some());
	assert_eq!(highlights(&b), [b.refill.unwrap()], "the Buy all row is the one highlighted, no stock row");
	assert_eq!(highlights(&page(&v, 0, Device::Pad)), [b.rows[0].1], "without the focus: retail's row");
	assert!(has(&b, "Refills: Pyrocitor Ammo, Blaster Ammo") && has(&b, "Ammo +108") && has(&b, "180"), "{:?}", texts(&b));
	assert!(texts(&b).iter().filter(|s| *s == "Buy all ammo").count() == 3, "the row, the card's title and its button");
	assert!(b.buy.is_some_and(|r| b.hit(r.cx(), r.cy()) == Hit::Buy), "the card's button buys");
	assert!(!has(&b, "Bomb Glove") || texts(&b).iter().filter(|s| *s == "Bomb Glove").count() == 1, "the weapon's card is gone");
	assert!(b.portrait.is_some(), "the vendor's portrait stays");
	// Only what the bolts reach: 4 bolts fill the 1-bolt row alone.
	let four = Ram::shop(&STOCK, &[(10, 22), (16, 150)], 4, 0, 0).view(true).unwrap();
	assert!(has(&focused_page(&four, &names(), 0), "Refills: Blaster Ammo"));
	// Full: greyed, no button that buys; broke: why.
	let full = Ram::shop(&STOCK, &[(10, 40), (16, 240)], 9000, 0, 0).view(true).unwrap();
	let f = focused_page(&full, &names(), 0);
	assert!(f.buyall && f.buy.is_none() && texts(&f).iter().filter(|s| *s == "Ammo is full").count() == 3 && has(&f, "\u{2014}"), "{:?}", texts(&f));
	let broke = focused_page(&view(0, 0, 0), &names(), 0);
	assert!(broke.buy.is_none() && texts(&broke).iter().filter(|s| *s == "Not enough bolts").count() == 3, "{:?}", texts(&broke));
	// The focus shows only where the row can be chosen; the list scrolls to the top.
	assert!(!focused_page(&view(4200, 1, 2), &names(), 0).buyall, "the prompt names retail's own item");
	let rows = (0..12).map(|i| ShopRow { item: i, ammo: i % 2 == 1, price: 1, owned: 0, max: 10 }).collect();
	let long = ShopView { phase: 1, bolts: 5000, rows, sel: 11, refill_on: true, ..Default::default() };
	let many: Vec<String> = (0..12).map(|i| format!("Item {i}")).collect();
	let b = focused_page(&long, &many, 0);
	assert_eq!(b.rows.first().map(|r| r.0), Some(0), "scrolled to the top");
	let lines: Vec<String> = texts(&b).into_iter().filter(|s| s.starts_with("Refills") || s.starts_with("Item 1")).collect();
	assert!(lines.iter().any(|s| s.starts_with("Refills: Item 1, Item 3")), "{lines:?}");
}

#[test]
fn the_prompt_answers_only_its_own_buttons_and_only_at_stage_two() {
	let b = page(&view(4200, 1, 2), 0, Device::Pad);
	assert!(b.prompt && has(&b, "Buy Pyrocitor Ammo ammo") && has(&b, "How many") && has(&b, "Total"));
	let (ok, no, less, more) = (b.confirm.unwrap(), b.cancel.unwrap(), b.less.unwrap(), b.more.unwrap());
	assert_eq!([b.hit(ok.cx(), ok.cy()), b.hit(no.cx(), no.cy()), b.hit(less.cx(), less.cy()), b.hit(more.cx(), more.cy())], [Hit::Confirm, Hit::Cancel, Hit::Less, Hit::More]);
	// Everything else of the screen is dead while the prompt is up.
	for r in b.rows.iter().map(|r| r.1).chain(b.buy).chain([DETAIL, LIST, Rect::new(LIST.x + 12.0, LIST.y + 12.0, LIST.w - 24.0, 60.0)]) {
		assert_eq!(b.hit(r.cx(), r.cy()), Hit::None, "{r:?}");
	}
	let weapon = page(&view(9000, 0, 2), 0, Device::Pad);
	assert!(weapon.confirm.is_some() && weapon.cancel.is_some() && weapon.less.is_none() && weapon.more.is_none(), "a weapon has no quantity");
	for stage in [3, 4] {
		let closing = page(&view(4200, 1, stage), 0, Device::Pad);
		assert!(closing.prompt, "stage {stage} still shows the prompt");
		assert!([closing.confirm, closing.cancel, closing.less, closing.more].iter().all(Option::is_none), "stage {stage} is the game's alone");
	}
	for stage in [0, 1] {
		let browse = page(&view(4200, 1, stage), 0, Device::Pad);
		assert!(!browse.prompt && [browse.confirm, browse.cancel, browse.less, browse.more].iter().all(Option::is_none));
	}
}

fn gate(v: &ShopView) -> Gate<'_> {
	Gate { modern: true, ui_open: false, refused: false, vendor: true, view: Some(v) }
}

#[test]
fn the_page_replaces_retails_picture_from_the_opening_fade_to_the_closing_in_the_modern_style() {
	let v = view(4200, 0, 0);
	assert!(wanted(&gate(&v)));
	let mut g = gate(&v);
	g.modern = false;
	assert!(!wanted(&g), "Classic is today's picture");
	let mut g = gate(&v);
	g.ui_open = true;
	assert!(!wanted(&g), "the Options page is up");
	let mut g = gate(&v);
	g.refused = true;
	assert!(!wanted(&g), "no GPU for the UI pipes");
	let mut g = gate(&v);
	g.vendor = false;
	assert!(!wanted(&g), "not the vendor screen");
	let mut g = gate(&v);
	g.view = None;
	assert!(!wanted(&g), "no readable shop");
	let mut p = v.clone();
	p.phase = 0;
	assert!(wanted(&gate(&p)), "the opening fade: the page at once");
	p.phase = 2;
	assert!(!wanted(&gate(&p)) && holds(&gate(&p)), "the closing: no page, the retail shop still hidden");
	assert!(holds(&Gate { view: None, ..gate(&v) }), "flying in before the list reads: hidden");
	assert!(!holds(&Gate { vendor: false, ..gate(&v) }) && !holds(&Gate { modern: false, ..gate(&v) }), "outside the shop, or Classic");
	let mut empty = v.clone();
	empty.rows.clear();
	assert!(!wanted(&gate(&empty)));
	assert!(wanted(&gate(&view(4200, 1, 2))), "the prompt is part of the list phase");
	assert!(build(&Facts { view: &empty, names: &[], focus: None, buyall: false, scroll: 0, lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation }, &FontMeasure).is_none());
}

#[test]
fn classic_hides_nothing() {
	for mask in 0..=0b1111u8 {
		assert!(!hidden_in(Screen::Vendor, mask, false, None), "mask {mask:#b}");
	}
	assert!(hidden_in(Screen::Vendor, 0b100, true, None), "Modern with the switch on hides the retail shop overlay");
}

/// The smallest `max_w / width` over the page's boxed texts (1 = everything fits; the renderer squeezes a wider text to its box).
fn tightest(b: &Built) -> (f32, String) {
	let m = FontMeasure;
	let mut worst = (1.0f32, String::new());
	for p in &b.prims {
		if let Prim::Text(r) = p {
			let w = run_width(&m, r.role, r.px, &r.text, r.track);
			if r.max_w > 0.0 && w > r.max_w + 0.5 && r.max_w / w < worst.0 {
				worst = (r.max_w / w, format!("{:?} is {w:.1} px in a box of {}", r.text, r.max_w));
			}
		}
	}
	worst
}

#[test]
fn every_word_fits_its_box_in_all_five_languages() {
	let m = FontMeasure;
	let long = ["Pyrocitor Ammunition", "Glove of Doom Ammo", "Lance-grenades de l'Enfer", "Handschuh des Verderbens", "Lanzagranadas de combate"];
	for lang in 0..5 {
		let short: Vec<String> = ["Blaster", "Bomb Glove", "Walloper"].iter().map(|s| s.to_string()).collect();
		let n: Vec<String> = (0..3).map(|i| long[(lang + i) % 5].to_string()).collect();
		for (bolts, sel, stage) in [(4200, 0, 0), (4200, 1, 0), (9_999_999, 2, 0), (0, 1, 0), (4200, 0, 2), (4200, 1, 2), (4200, 2, 2), (4200, 1, 3)] {
			// Also with the focus on Buy all ammo (its own card).
			for (device, buyall) in [(Device::Pad, false), (Device::Keyboard, false), (Device::Pad, true), (Device::Keyboard, true)] {
				let v = view(bolts, sel, stage);
				let make = |names: &[String]| build(&Facts { view: &v, names, focus: None, buyall, scroll: 0, lang, device, glyphs: GlyphSet::PlayStation }, &m).unwrap();
				let (ratio, what) = tightest(&make(&short));
				assert!(ratio >= 1.0, "{what} (lang {lang}, stage {stage}, {device:?})");
				let (ratio, what) = tightest(&make(&n));
				assert!(ratio >= 0.6, "a long item name is squeezed too far: {what} (lang {lang}, stage {stage}, {device:?})");
				let b = make(&short);
				for p in &b.prims {
					if let Prim::Glyph { rect, .. } = p {
						assert!(rect.x >= 0.0 && rect.right() <= 1280.0, "glyph {rect:?}");
					}
				}
				// The hint bar (y = 688) starts inside the screen.
				let left = b.prims.iter().filter_map(|p| match p {
					Prim::Text(r) if r.y == 688.0 => Some(r.x),
					Prim::Glyph { rect, .. } if rect.cy() == 688.0 => Some(rect.x),
					_ => None,
				});
				assert!(left.fold(f32::MAX, f32::min) >= 48.0, "hint bar (lang {lang}, stage {stage}, {device:?})");
			}
		}
	}
}

/// A shop's most ammo rows (7), all hungry, with long names: the Buy all ammo card's "Refills:" lines fit their box unsqueezed and
/// name or count every row in all five languages (R23 m2).
#[test]
fn the_refills_lines_fit_seven_hungry_rows_in_all_five_languages() {
	let m = FontMeasure;
	let long = ["Pyrocitor Ammunition", "Glove of Doom Ammo", "Lance-grenades de l'Enfer", "Handschuh des Verderbens", "Lanzagranadas de combate", "Visibomben-Munition", "Munizioni Lanciadecoy"];
	let rows = (0..7).map(|i| ShopRow { item: i, ammo: true, price: 1, owned: 0, max: 10 }).collect();
	let v = ShopView { phase: 1, bolts: 5000, rows, sel: 0, refill_on: true, ..Default::default() };
	for lang in 0..5 {
		let names: Vec<String> = (0..7).map(|i| long[(lang + i) % 7].to_string()).collect();
		let b = focused_page(&v, &names, lang);
		let head = t::fill(&t::REFILLS, lang, "");
		let runs: Vec<_> = b.prims.iter().filter_map(|p| if let Prim::Text(r) = p { Some(r) } else { None }).collect();
		let i = runs.iter().position(|r| r.text.starts_with(&head)).expect("a Refills line");
		let lines: Vec<_> = runs[i..].iter().take(2).filter(|r| r.y == runs[i].y || r.y == runs[i].y + 22.0).collect();
		for r in &lines {
			let w = run_width(&m, r.role, r.px, &r.text, r.track);
			assert!(w <= r.max_w + 0.5, "{:?} is {w:.1} px in a box of {} (lang {lang})", r.text, r.max_w);
		}
		let joined = lines.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join(" ");
		let named = names.iter().filter(|s| joined.contains(s.as_str())).count();
		let more: usize = joined.split('+').nth(1).map_or(0, |s| s.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap());
		assert_eq!(named + more, 7, "every row named or counted: {joined:?} (lang {lang})");
	}
}

#[test]
fn every_character_has_a_glyph_in_every_font() {
	let set = FontSet::get();
	assert!(set.covers('\u{b7}') && set.covers('\u{2014}'), "the middle dot and the dash the page uses");
	for tab in t::all() {
		for (lang, s) in tab.iter().enumerate() {
			for c in s.chars() {
				assert!(set.covers(c), "{c:?} ({:#x}) of {s:?} (lang {lang})", c as u32);
			}
		}
	}
	for lang in 0..5 {
		let v = view(4200, 1, 2);
		for p in &page(&v, lang, Device::Keyboard).prims {
			if let Prim::Text(r) = p {
				assert!(r.text.chars().all(|c| set.covers(c)), "{:?} (lang {lang})", r.text);
			}
		}
	}
}

#[test]
fn every_stock_row_shows_the_games_own_picture_a_weapon_its_menu_icon_and_ammo_its_shop_row_icon() {
	let pics = [(icons::item(11, icons::FRAME_MENU), 41, 38), (icons::item(10, icons::FRAME_AMMO_ROW), 30, 30), (icons::item(16, icons::FRAME_AMMO_ROW), 30, 30)];
	let ids = |b: &Built| -> Vec<u32> { b.prims.iter().filter_map(|p| if let Prim::Image { id, .. } = p { Some(*id) } else { None }).collect() };
	let v = view(9000, 0, 0);
	let got = icons::with_pictures(&pics, || ids(&page(&v, 0, Device::Pad)));
	assert_eq!(got, pics.map(|p| p.0), "one picture per row, in row order, none for the Buy all row");
	assert!(icons::with_pictures(&[], || ids(&page(&v, 0, Device::Pad))).is_empty(), "no registered picture: the generic ring");
}

#[test]
fn exactly_one_bolt_is_singular() {
	let rows = vec![ShopRow { item: 16, ammo: true, price: 1, owned: 0, max: 5 }];
	let v = ShopView { phase: 1, bolts: 1, rows, sel: 0, ..Default::default() };
	let b = page(&v, 0, Device::Pad);
	assert!(has(&b, "Ammo 0 / 5 \u{b7} 1 Bolt each"), "{:?}", texts(&b));
	let none = page(&view(0, 0, 0), 0, Device::Pad);
	assert!(has(&none, "7,500 Bolts") && !has(&none, "7,500 Bolt"));
}
