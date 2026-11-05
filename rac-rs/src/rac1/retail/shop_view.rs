//! The Gadgetron vendor shop's facts for the modern shop page (v0.5 screens stage E): the list rows with their prices, the ammo
//! owned and the most the game lets you hold, the bolts, the shop's phase and stage, the quantity on the prompt, and what the
//! "Buy all ammo" hook would spend. Read from the game's RAM through `rd` (a 32-bit word read) in the worker's snapshot
//! (`shop_snap`); pure over `rd`, so the tests run on a fake RAM. The game decides everything (prices, purchases, ammo, the refill
//! hook of `vendor.rs`): this file only reads, and the modern page never writes bolts or ammo. Words per level: `vendor::Words`;
//! the globals (bolts, owned ammo, the alternate-price byte) are the boot ELF's (`disc::va`, so USA is read too).
use super::disc::va;
use super::vendor;

/// Rows read: the cap of every shop reader (`mouse_vendor::MAX_CYCLIC`). Retail's list is cyclic from 8 rows; no level has more than 7
/// without the pause cheat codes (decomp/notes/vendor.md), and the modern page scrolls a longer one.
const MAX_ROWS: u32 = super::mouse_vendor::MAX_CYCLIC;
/// The prompt's quantity word sits 0x1c above the leaving flag (`mouse_vendor::publish`). Checked on all 19 levels of both
/// discs (2026-11-02, scratch_perf_nrv2/qtyoff.py over the ISOs' overlays): the purchase function `entry` stores 1 there at
/// entry + 0x54 (stage 1) and reads it 7 times (price = unit * qty), so one offset serves PAL and USA without a `VLevel` word.
const QTY_OFF: u32 = 0x1c;
/// The Persuader's owned byte (`0x13d4c0 + 35`) selects the alternate weapon price (decomp/notes/vendor.md NOV-13).
const ALT_PRICE_ITEM: u32 = 35;

/// One row of the shop's list (retail's 0x14-byte records: item, kind, 0, 0, price).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ShopRow {
	pub item: u32,
	/// An ammo row (kind 1); else a weapon or gadget row.
	pub ammo: bool,
	/// Weapon: the price (the alternate one while the Persuader is owned). Ammo: the price of one round.
	pub price: u32,
	/// Ammo rows: rounds held and the most the game lets you hold.
	pub owned: u32,
	pub max: u32,
}

impl ShopRow {
	/// An ammo row that holds the most already.
	pub fn full(&self) -> bool {
		self.ammo && self.owned >= self.max
	}

	/// The most rounds the quantity prompt allows: `min(max - owned, bolts / unit)` (retail's clamp; 0 on a weapon row).
	pub fn most(&self, bolts: u32) -> u32 {
		if !self.ammo || self.price == 0 {
			return 0;
		}
		self.max.saturating_sub(self.owned).min(bolts / self.price)
	}

	/// Would a purchase go through now (a weapon: the bolts reach the price; ammo: at least one round fits)?
	pub fn can_buy(&self, bolts: u32) -> bool {
		if self.ammo { self.most(bolts) > 0 } else { bolts >= self.price }
	}
}

/// What the refill hook buys: every ammo row in list order, each to the most the prompt would allow with the bolts left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Refill {
	pub cost: u32,
	pub rounds: u32,
}

/// The shop as the modern page needs it (one field's facts).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ShopView {
	/// 0 opening fade, 1 browse, 2 closing.
	pub phase: u32,
	/// 0 browse, 1 start, 2 quantity prompt, 3 closing prompt, 4 resolve.
	pub stage: u32,
	/// The highlighted row (retail's `sel`), inside `rows`.
	pub sel: usize,
	/// The exit is pending (Triangle was pressed).
	pub leaving: bool,
	pub bolts: u32,
	/// The quantity on the ammo prompt.
	pub qty: u32,
	/// The remote ammo-only (PDA) shop.
	pub ammo_only: bool,
	pub rows: Vec<ShopRow>,
	/// The page lists Buy all ammo: the option "Buy all ammo" is on or the menus are Modern (`vendor::buyall_listed`).
	pub refill_on: bool,
}

impl ShopView {
	/// The purchase prompt (or its closing / resolving steps) is the screen.
	pub fn prompt(&self) -> bool {
		matches!(self.stage, 2..=4)
	}

	pub fn row(&self) -> Option<&ShopRow> {
		self.rows.get(self.sel)
	}

	/// The Square refill, as `vendor::buy_all` runs it: the cost and rounds over every ammo row.
	pub fn refill(&self) -> Refill {
		self.refill_rows().iter().fold(Refill::default(), |a, &(i, q)| Refill { cost: a.cost + q * self.rows[i].price, rounds: a.rounds + q })
	}

	/// The rows the Square refill fills: (row index, rounds), in list order, each to the most the prompt would allow with the bolts
	/// the rows before it left (as `vendor::buy_all` spends them); a row that takes nothing is left out.
	pub fn refill_rows(&self) -> Vec<(usize, u32)> {
		let mut left = self.bolts;
		let mut out = Vec::new();
		for (i, r) in self.rows.iter().enumerate().filter(|(_, r)| r.ammo) {
			let q = r.most(left);
			left -= q * r.price;
			if q > 0 {
				out.push((i, q));
			}
		}
		out
	}

	/// Square would do something now: the option is on, the shop browses, and some ammo row can take rounds.
	pub fn can_refill(&self) -> bool {
		self.refill_on && self.phase == 1 && self.stage == 0 && !self.leaving && self.refill().rounds > 0
	}
}

fn byte(rd: &dyn Fn(u32) -> u32, a: u32) -> u32 {
	(rd(a & !3) >> ((a & 3) * 8)) & 0xff
}

fn half(rd: &dyn Fn(u32) -> u32, a: u32) -> u32 {
	(rd(a & !3) >> ((a & 2) * 8)) & 0xffff
}

/// The shop of `level` as the game's RAM holds it now (`refill_on` = the page lists Buy all ammo, `vendor::buyall_listed`). None when the list is empty or absurd.
pub fn read(rd: &dyn Fn(u32) -> u32, level: u32, refill_on: bool) -> Option<ShopView> {
	let w = vendor::words(level)?;
	let count = rd(w.count);
	if !(1..=MAX_ROWS).contains(&count) {
		return None;
	}
	let ammo_only = rd(w.alt) != 0;
	let alternate = byte(rd, va(0x13d4c0) + ALT_PRICE_ITEM) != 0;
	let owned: u32 = va(0x13d428);
	let rows: Vec<ShopRow> = (0..count)
		.map(|i| {
			let r = w.rows + i * 0x14;
			let item = rd(r);
			let rec = w.p0.wrapping_add(item.wrapping_mul(0x18));
			if rd(r + 4) == 1 {
				ShopRow { item, ammo: true, price: half(rd, rec + if ammo_only { 0xa } else { 8 }), owned: rd(owned.wrapping_add(item.wrapping_mul(4))), max: half(rd, rec + 0xe) }
			} else {
				ShopRow { item, ammo: false, price: rd(rec + if alternate { 4 } else { 0 }), owned: 0, max: 0 }
			}
		})
		.collect();
	Some(ShopView {
		phase: rd(w.phase),
		stage: rd(w.stage),
		sel: (rd(w.sel) as usize).min(rows.len() - 1),
		leaving: rd(w.exitp) != 0,
		bolts: rd(va(0x15ed98)),
		qty: rd(w.exitp + QTY_OFF),
		ammo_only,
		rows,
		refill_on,
	})
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;
	use std::collections::HashMap;

	pub const LEVEL: u32 = 1;

	/// A fake RAM for a shop on `LEVEL`: `rows` = (item, ammo, price word at +0, alt price word at +4, unit +8, unit +0xa, max +0xe), the
	/// ammo owned per item, bolts, sel, stage, phase.
	pub struct Ram(pub HashMap<u32, u32>);

	pub type RowSpec = (u32, bool, u32, u32, u32, u32, u32);

	fn put_halves(m: &mut HashMap<u32, u32>, a: u32, lo: u32, hi: u32) {
		m.insert(a, lo | hi << 16);
	}

	impl Ram {
		pub fn shop(rows: &[RowSpec], owned: &[(u32, u32)], bolts: u32, sel: u32, stage: u32) -> Ram {
			let w = vendor::words(LEVEL).expect("level 1 has a shop");
			let mut m = HashMap::new();
			for (i, &(item, ammo, price, alt, unit, unit_alt, max)) in rows.iter().enumerate() {
				let r = w.rows + i as u32 * 0x14;
				m.insert(r, item);
				m.insert(r + 4, ammo as u32);
				let rec = w.p0 + item * 0x18;
				m.insert(rec, price);
				m.insert(rec + 4, alt);
				put_halves(&mut m, rec + 8, unit, unit_alt);
				put_halves(&mut m, rec + 0xc, 0, max);
			}
			for &(item, n) in owned {
				m.insert(va(0x13d428) + item * 4, n);
			}
			m.insert(w.count, rows.len() as u32);
			m.insert(va(0x15ed98), bolts);
			m.insert(w.sel, sel);
			m.insert(w.stage, stage);
			m.insert(w.phase, 1);
			Ram(m)
		}

		pub fn set(&mut self, a: u32, v: u32) {
			self.0.insert(a, v);
		}

		pub fn view(&self, refill_on: bool) -> Option<ShopView> {
			read(&|a| self.0.get(&a).copied().unwrap_or(0), LEVEL, refill_on)
		}
	}

	/// Item 11 (a weapon at 7,500, 3,750 with the Persuader), item 10 (ammo: 5 a round, 25 in ammo-only mode, max 40) and item 16
	/// (ammo: 1 a round, 5, max 240).
	pub const STOCK: [RowSpec; 3] = [(11, false, 7500, 3750, 0, 0, 0), (10, true, 0, 0, 5, 25, 40), (16, true, 2500, 1250, 1, 5, 240)];

	#[test]
	fn rows_carry_their_prices_ammo_and_the_stage() {
		let v = Ram::shop(&STOCK, &[(10, 22), (16, 150)], 4200, 1, 0).view(true).expect("a shop");
		assert_eq!((v.phase, v.stage, v.sel, v.bolts, v.leaving, v.ammo_only), (1, 0, 1, 4200, false, false));
		assert_eq!(v.rows[0], ShopRow { item: 11, ammo: false, price: 7500, owned: 0, max: 0 });
		assert_eq!(v.rows[1], ShopRow { item: 10, ammo: true, price: 5, owned: 22, max: 40 });
		assert_eq!(v.rows[2], ShopRow { item: 16, ammo: true, price: 1, owned: 150, max: 240 });
		assert_eq!(v.row().map(|r| r.item), Some(10));
	}

	#[test]
	fn the_persuader_and_the_ammo_only_flag_pick_the_other_price() {
		let mut ram = Ram::shop(&STOCK, &[], 100, 0, 0);
		let w = vendor::words(LEVEL).unwrap();
		let plain = ram.view(false).unwrap();
		assert_eq!(plain.rows[0].price, 7500);
		let alt_word = (va(0x13d4c0) + ALT_PRICE_ITEM) & !3;
		ram.set(alt_word, 1 << (((va(0x13d4c0) + ALT_PRICE_ITEM) & 3) * 8));
		assert_eq!(ram.view(false).unwrap().rows[0].price, 3750, "the Persuader: the +4 price");
		ram.set(w.alt, 1);
		let v = ram.view(false).unwrap();
		assert!(v.ammo_only);
		assert_eq!((v.rows[1].price, v.rows[2].price), (25, 5), "ammo-only mode: the +0xa unit price");
	}

	#[test]
	fn affordability_and_the_most_the_prompt_allows() {
		let weapon = ShopRow { item: 11, price: 7500, ..Default::default() };
		assert!(weapon.can_buy(7500) && !weapon.can_buy(7499));
		assert_eq!(weapon.most(9999), 0);
		let ammo = ShopRow { item: 10, ammo: true, price: 5, owned: 22, max: 40 };
		assert_eq!((ammo.most(1000), ammo.most(30), ammo.most(4)), (18, 6, 0), "min(max - owned, bolts / unit)");
		assert!(ammo.can_buy(5) && !ammo.can_buy(4));
		let full = ShopRow { owned: 40, ..ammo.clone() };
		assert!(full.full() && !full.can_buy(1000) && !weapon.full());
	}

	#[test]
	fn the_refill_spends_row_by_row_like_the_hook() {
		let v = Ram::shop(&STOCK, &[(10, 22), (16, 150)], 4200, 0, 0).view(true).unwrap();
		assert_eq!(v.refill(), Refill { cost: 18 * 5 + 90, rounds: 18 + 90 }, "both ammo rows to the max");
		let poor = Ram::shop(&STOCK, &[(10, 22), (16, 150)], 100, 0, 0).view(true).unwrap();
		assert_eq!(poor.refill(), Refill { cost: 100, rounds: 28 }, "the first row takes 18 rounds (90), the second what is left (10)");
		let none = Ram::shop(&STOCK, &[(10, 40), (16, 240)], 4200, 0, 0).view(true).unwrap();
		assert_eq!(none.refill(), Refill::default());
		assert!(!none.can_refill() && v.can_refill());
		assert_eq!((v.refill_rows(), poor.refill_rows(), none.refill_rows()), (vec![(1, 18), (2, 90)], vec![(1, 18), (2, 10)], vec![]), "the rows it fills");
		let four = Ram::shop(&STOCK, &[(10, 22), (16, 150)], 4, 0, 0).view(true).unwrap();
		assert_eq!(four.refill_rows(), [(2, 4)], "not one round of 5 bolts: the 1-bolt row alone");
	}

	#[test]
	fn the_refill_button_follows_the_option_the_stage_and_the_exit() {
		let ram = Ram::shop(&STOCK, &[(10, 22)], 4200, 0, 0);
		assert!(ram.view(true).unwrap().can_refill());
		assert!(!ram.view(false).unwrap().can_refill(), "option off");
		let prompt = Ram::shop(&STOCK, &[(10, 22)], 4200, 0, 2).view(true).unwrap();
		assert!(!prompt.can_refill() && prompt.prompt(), "no refill under the prompt");
		let mut leaving = Ram::shop(&STOCK, &[(10, 22)], 4200, 0, 0);
		leaving.set(vendor::words(LEVEL).unwrap().exitp, 1);
		assert!(leaving.view(true).unwrap().leaving && !leaving.view(true).unwrap().can_refill());
	}

	#[test]
	fn an_empty_or_absurd_list_and_a_wild_sel_are_safe() {
		let w = vendor::words(LEVEL).unwrap();
		let mut ram = Ram::shop(&STOCK, &[], 0, 99, 0);
		assert_eq!(ram.view(true).unwrap().sel, 2, "sel is clamped into the list");
		ram.set(w.count, 0);
		assert!(ram.view(true).is_none());
		ram.set(w.count, 400);
		assert!(ram.view(true).is_none());
		assert!(read(&|_| 0, 99, true).is_none(), "a level without a shop");
	}
}
