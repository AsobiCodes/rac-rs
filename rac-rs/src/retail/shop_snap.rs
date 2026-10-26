//! The vendor shop screen's VU1 mobys and hologram beam, read from RAM (`ShopSnap`, `shop_snap`).

use super::*;

/// One moby of the vendor's 0x100-stride slot block (or its portrait actor): not in the moby table, so `snap` never sees it.
pub struct ShopMoby {
	pub class: u32,
	pub pos: [f32; 3],
	pub rot: [f32; 3],
	/// Scale over the class header's base scale (as `Moby::scale`).
	pub scale: f32,
	pub clock: Clock,
}

/// What the vendor screen draws with VU1 besides the world (vendor.md REN-9 / NOV-30a / NOV-30d / BRIDGE-37) [C].
pub struct ShopSnap {
	/// The world item over the pad: B (slot +0x200, ammo row world model) or C (slot +0x300, the hologram item).
	pub world: Option<ShopMoby>,
	/// Panel 1 (item target): A (slot +0x100) and the slot-0 class-13 bolt icon.
	pub panel: Option<ShopMoby>,
	pub icon: Option<ShopMoby>,
	/// Panel 2 (portrait target): the class-12 vendor actor.
	pub portrait: Option<ShopMoby>,
	/// The hologram beam over the pad (`shop_beam`), empty while the shop's alt flag is set.
	pub beam: Vec<crate::particles::WorldQuad>,
	/// The purchase prompt moby (slot +0x400, class 1137) while the prompt is up (base + 0x5c != 0): FUN_00290ad0
	/// OV:118067-118069 draws it with FUN_002904f0, which renders the popup target, draws the moby into the frame
	/// (FUN_002427d0(slot, 1), VU1, so absent from the GS capture) and blits the target over it (GS, in the overlay).
	/// `snap` adds it to the moby list, so `sync` mirrors it as an ordinary prop.
	pub prompt: Option<u32>,
	/// The popup target's class-13 bolt (slot +0x500): FUN_0028f2f8 OV:117284-117298 draws it on an ammo row that is not
	/// full (max u16 price +0xe > owned 0x13d428 + item * 4) when the bolts 0x15ed98 reach the unit price (u16 +8, +0xa
	/// with the alt flag); price table base - 0x6410 + item * 0x18 on every level (twins' lui/addiu, round 8) [C].
	pub popup_bolt: Option<ShopMoby>,
}

/// Per-level globals of the shop screen. Eudora [C]: FUN_00290ad0 (= Novalis FUN_002b3950) OV:117937-118082 keeps the
/// screen state in 0x1ca548 (1 = up), the slot-0 pointer in 0x1ca560 (slot block base + 0x100) and the portrait actor in
/// 0x1ca564; the world draw (OV:118051-118060) runs when 0x161f38 == 0 && 0x1ca580 == 0: row = 0x1ca598, the row
/// records (stride 20) at 0x1ca610 (item) / 0x1ca614 (kind): kind 1 draws B when the item's 0x1c90a0 + item * 0x40 word
/// is set, any other kind draws C. Panel 1 FUN_0028e9b8 draws slot +0x100 and slot 0 (OV:117000-117001); panel 2
/// FUN_0028e608 draws the portrait when 0x161108 == 1 (OV:116852-116853). Other levels [C]: the twins of FUN_00290ad0 /
/// FUN_0028e608 (twins.py --all) are the same code; the block bases (Eudora 0x1ca540 in a0, ammo 0x1c9070, 0x161f38, 0x161108) are
/// read from each twin's lui/addiu pairs and the fields keep Eudora's offsets from them (scratch_m3b round 7).
struct ShopAddrs {
	pub(super) state: u32,
	pub(super) slots: u32,
	portrait: u32,
	pub(super) gate: u32,
	pub(super) alt: u32,
	pub(super) row: u32,
	pub(super) rows: u32,
	pub(super) ammo: u32,
	portrait_on: u32,
}

fn shop_addrs(level: u32) -> Option<ShopAddrs> {
	match level {
		0 => Some(ShopAddrs { state: va(0x1ca448), slots: va(0x1ca460), portrait: va(0x1ca464), gate: va(0x161e38), alt: va(0x1ca480), row: va(0x1ca498), rows: va(0x1ca510), ammo: va(0x1c8fa0), portrait_on: va(0x161148) }), // veldin
		1 => Some(ShopAddrs { state: va(0x1ca848), slots: va(0x1ca860), portrait: va(0x1ca864), gate: va(0x162238), alt: va(0x1ca880), row: va(0x1ca898), rows: va(0x1ca910), ammo: va(0x1c93a0), portrait_on: va(0x161108) }), // novalis
		2 => Some(ShopAddrs { state: va(0x1ca9c8), slots: va(0x1ca9e0), portrait: va(0x1ca9e4), gate: va(0x1623b8), alt: va(0x1caa00), row: va(0x1caa18), rows: va(0x1caa90), ammo: va(0x1c9520), portrait_on: va(0x161108) }), // aridia
		3 => Some(ShopAddrs { state: va(0x1ca4c8), slots: va(0x1ca4e0), portrait: va(0x1ca4e4), gate: va(0x161eb8), alt: va(0x1ca500), row: va(0x1ca518), rows: va(0x1ca590), ammo: va(0x1c9020), portrait_on: va(0x161108) }), // kerwan
		4 => Some(ShopAddrs { state: va(0x1ca548), slots: va(0x1ca560), portrait: va(0x1ca564), gate: va(0x161f38), alt: va(0x1ca580), row: va(0x1ca598), rows: va(0x1ca610), ammo: va(0x1c90a0), portrait_on: va(0x161108) }), // eudora
		5 => Some(ShopAddrs { state: va(0x1ca8c8), slots: va(0x1ca8e0), portrait: va(0x1ca8e4), gate: va(0x162238), alt: va(0x1ca900), row: va(0x1ca918), rows: va(0x1ca990), ammo: va(0x1c9420), portrait_on: va(0x161148) }), // rilgar
		6 => Some(ShopAddrs { state: va(0x1cab48), slots: va(0x1cab60), portrait: va(0x1cab64), gate: va(0x162538), alt: va(0x1cab80), row: va(0x1cab98), rows: va(0x1cac10), ammo: va(0x1c96a0), portrait_on: va(0x161108) }), // nebula
		7 => Some(ShopAddrs { state: va(0x1ca448), slots: va(0x1ca460), portrait: va(0x1ca464), gate: va(0x161e38), alt: va(0x1ca480), row: va(0x1ca498), rows: va(0x1ca510), ammo: va(0x1c8fa0), portrait_on: va(0x161108) }), // umbris
		8 => Some(ShopAddrs { state: va(0x1cabc8), slots: va(0x1cabe0), portrait: va(0x1cabe4), gate: va(0x1625b8), alt: va(0x1cac00), row: va(0x1cac18), rows: va(0x1cac90), ammo: va(0x1c9720), portrait_on: va(0x161108) }), // batalia
		9 => Some(ShopAddrs { state: va(0x1ca548), slots: va(0x1ca560), portrait: va(0x1ca564), gate: va(0x161f38), alt: va(0x1ca580), row: va(0x1ca598), rows: va(0x1ca610), ammo: va(0x1c90a0), portrait_on: va(0x161108) }), // gaspar
		10 => Some(ShopAddrs { state: va(0x1ca848), slots: va(0x1ca860), portrait: va(0x1ca864), gate: va(0x162238), alt: va(0x1ca880), row: va(0x1ca898), rows: va(0x1ca910), ammo: va(0x1c93a0), portrait_on: va(0x161108) }), // orxon
		11 => Some(ShopAddrs { state: va(0x1cadc8), slots: va(0x1cade0), portrait: va(0x1cade4), gate: va(0x1627b8), alt: va(0x1cae00), row: va(0x1cae18), rows: va(0x1cae90), ammo: va(0x1c9920), portrait_on: va(0x161108) }), // pokitaru
		12 => Some(ShopAddrs { state: va(0x1ca848), slots: va(0x1ca860), portrait: va(0x1ca864), gate: va(0x162238), alt: va(0x1ca880), row: va(0x1ca898), rows: va(0x1ca910), ammo: va(0x1c93a0), portrait_on: va(0x161108) }), // hoven
		13 => Some(ShopAddrs { state: va(0x1ca6c8), slots: va(0x1ca6e0), portrait: va(0x1ca6e4), gate: va(0x1620b8), alt: va(0x1ca700), row: va(0x1ca718), rows: va(0x1ca790), ammo: va(0x1c9220), portrait_on: va(0x161108) }), // oltanis_orbit
		14 => Some(ShopAddrs { state: va(0x1cab48), slots: va(0x1cab60), portrait: va(0x1cab64), gate: va(0x162538), alt: va(0x1cab80), row: va(0x1cab98), rows: va(0x1cac10), ammo: va(0x1c96a0), portrait_on: va(0x161148) }), // oltanis
		15 => Some(ShopAddrs { state: va(0x1ca9c8), slots: va(0x1ca9e0), portrait: va(0x1ca9e4), gate: va(0x1623b8), alt: va(0x1caa00), row: va(0x1caa18), rows: va(0x1caa90), ammo: va(0x1c9520), portrait_on: va(0x161108) }), // quartu
		16 => Some(ShopAddrs { state: va(0x1ca848), slots: va(0x1ca860), portrait: va(0x1ca864), gate: va(0x1621b8), alt: va(0x1ca880), row: va(0x1ca898), rows: va(0x1ca910), ammo: va(0x1c93a0), portrait_on: va(0x161148) }), // kalebo
		17 => Some(ShopAddrs { state: va(0x1cacc8), slots: va(0x1cace0), portrait: va(0x1cace4), gate: va(0x1626b8), alt: va(0x1cad00), row: va(0x1cad18), rows: va(0x1cad90), ammo: va(0x1c9820), portrait_on: va(0x161108) }), // veldin_orbit
		18 => Some(ShopAddrs { state: va(0x1cadc8), slots: va(0x1cade0), portrait: va(0x1cade4), gate: va(0x1627b8), alt: va(0x1cae00), row: va(0x1cae18), rows: va(0x1cae90), ammo: va(0x1c9920), portrait_on: va(0x161108) }), // veldin_finale
		_ => None,
	}
}

fn shop_moby(m: &ee_oracle::Machine, a: u32) -> Option<ShopMoby> {
	let a = a & 0x01ff_ffff;
	let hdr = m.rd32(a + 0x24) & 0x01ff_ffff;
	if a == 0 || hdr == 0 {
		return None;
	}
	let base = f32::from_bits(m.rd32(hdr + 0x24));
	Some(ShopMoby {
		class: m.rd16(a + 0xa6) as u32,
		pos: m.rdv3(a + 0x10),
		rot: basis_euler(m, a),
		scale: if base != 0.0 { m.rdf(a + 0x2c) / base } else { 1.0 },
		clock: Clock::read(m, a),
	})
}

/// The shop screen's VU1 mobys of this field (`ShopAddrs`), None while the screen is not up.
pub(super) fn shop_snap(m: &ee_oracle::Machine, level: u32) -> Option<ShopSnap> {
	let s = shop_addrs(level)?;
	if m.rd32(s.state) != 1 {
		return None;
	}
	let slot0 = m.rd32(s.slots) & 0x01ff_ffff;
	if slot0 == 0 {
		return None;
	}
	let world = (m.rd32(s.gate) == 0 && m.rd32(s.alt) == 0).then(|| {
		let r = s.rows + m.rd32(s.row).wrapping_mul(20);
		if m.rd32(r + 4) == 1 {
			(m.rd32(s.ammo + m.rd32(r).wrapping_mul(0x40)) != 0).then_some(slot0 + 0x200)
		} else {
			Some(slot0 + 0x300)
		}
	});
	Some(ShopSnap {
		world: world.flatten().and_then(|a| shop_moby(m, a)),
		panel: shop_moby(m, slot0 + 0x100),
		icon: shop_moby(m, slot0),
		portrait: (m.rd32(s.portrait_on) == 1).then(|| shop_moby(m, m.rd32(s.portrait))).flatten(),
		beam: if m.rd32(s.alt) == 0 { shop_beam(m, s.state - 8, s.portrait_on - 4) } else { Vec::new() },
		prompt: (m.rd32(s.state + 0x54) != 0).then_some(slot0 + 0x400),
		popup_bolt: (m.rd32(s.state + 0x54) != 0 && popup_bolt_drawn(m, &s)).then(|| shop_moby(m, slot0 + 0x500)).flatten(),
	})
}

/// Whether the popup content FUN_0028f2f8 draws the slot +0x500 bolt this field (`ShopSnap::popup_bolt`) [C].
fn popup_bolt_drawn(m: &ee_oracle::Machine, s: &ShopAddrs) -> bool {
	let r = s.rows + m.rd32(s.row).wrapping_mul(20);
	if m.rd32(r + 4) != 1 {
		return false;
	}
	let item = m.rd32(r);
	let price = (s.state - 8 - 0x6410).wrapping_add(item.wrapping_mul(0x18));
	let owned = m.rd32(0x13d428u32.wrapping_add(item.wrapping_mul(4))) as i32;
	let unit = m.rd16(price + if m.rd32(s.alt) == 0 { 8 } else { 0xa }) as i32;
	(m.rd16(price + 0xe) as i32) > owned && m.rd32(0x15ed98) as i32 >= unit
}

/// The shop's hologram beam over the pad: Eudora FUN_00290778 (OV:117766-117936), called by the shop draw FUN_00290ad0
/// (OV:118062-118064) while the alt flag (base + 0x40) is 0 [C]. Four strip quads (FUN_001ef7c0) whose vertex indices
/// are 4 i32 each at base - 0x40; vertex i: vendor-local position base - 0x140 + i * 0x10, placed by the vendor moby
/// (pointer base + 0x1c) as pos + x * M0 + y * M1 + z * M2 (rows +0xc0 / +0xd0 / +0xe0, FUN_001f1c68); UV at
/// base - 0xb0 + i * 8 with V + the scroll at gp-0x5afc (+0.01 per field, wraps at 1, added before the draw, so the
/// value after the field is the drawn one); rgba word base - 0x68 + i * 4; fx texture 0x18, ALPHA 0x44 (alpha-over).
/// Base = the shop state - 8 and the scroll = portrait_on - 4 on every level: each twin's lui/addiu pairs give
/// base - 0x140 / - 0xb0 / - 0x68 / - 0x40 and gp-0x5afc (gp-0x5abc on levels 0, 5, 14, 16) (scratch_m3b round 8).
fn shop_beam(m: &ee_oracle::Machine, base: u32, scroll: u32) -> Vec<crate::particles::WorldQuad> {
	let v = m.rd32(base + 0x1c) & 0x01ff_ffff;
	if v == 0 {
		return Vec::new();
	}
	let pos = m.rdv3(v + 0x10);
	let rows = [0, 1, 2].map(|r| m.rdv3(v + 0xc0 + 0x10 * r));
	let sc = m.rdf(scroll);
	let vert = |i: u32| {
		let l = m.rdv3(base - 0x140 + i * 0x10);
		let p: [f32; 3] = std::array::from_fn(|a| pos[a] + l[0] * rows[0][a] + l[1] * rows[1][a] + l[2] * rows[2][a]);
		(p, [m.rdf(base - 0xb0 + i * 8), m.rdf(base - 0xb0 + i * 8 + 4) + sc], pool_rgba(m.rd32(base - 0x68 + i * 4)))
	};
	(0..4u32)
		.filter_map(|q| {
			let idx = [0, 1, 2, 3].map(|k| m.rd32(base - 0x40 + q * 0x10 + k * 4));
			if idx.iter().any(|&i| i > 8) {
				return None;
			}
			// Strip order (v0, v1, v2, v3) -> fan order (v0, v1, v3, v2).
			let [a, b, c, d] = idx.map(vert);
			Some(crate::particles::WorldQuad { pos: [a.0, b.0, d.0, c.0], uv: [a.1, b.1, d.1, c.1], colour: [a.2, b.2, d.2, c.2], tex: 0x18, additive: false })
		})
		.collect()
}
