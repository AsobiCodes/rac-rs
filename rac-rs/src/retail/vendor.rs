//! Gadgetron vendor "buy all ammo" (PC extra, Options -> Camera Options -> Buy all ammo, default On).
//!
//! One press of Square (keyboard F; the shop's other keys work the menu way, menu_keys::VENDOR) while the shop browses buys the ammo of every ammo row, in list order, each to the most the
//! retail quantity prompt would allow: `min(max - owned, bolts / unit price)`. Per row it does what the retail purchase machine does
//! for a confirmed ammo purchase (stage 4 of Eudora FUN_0028c298, see decomp/notes/vendor.md "Buy all ammo hook"): AddAmmo(item,
//! qty) -> overflow, bought[item] += qty - overflow, portrait event 3 (idempotent: it only clears the idle timer), bolts -= unit *
//! (qty - overflow), row rebuild. Once per press: the THANK YOU ticker + vendor cue 7 when anything was bought; else "not enough
//! bolts" (0x4f60) + cue 0 when some row had room but bolts < unit; else (all full) cue 0 alone, which is what a retail single buy
//! does in each case. The press is seen at the purchase function (called every frame of the shop), which then runs retail's own
//! code unchanged (stage 0 returns). Off (or RAC_PC_EXT=0): no hook acts.
//!
//! The Buy plate (shop_buy_panel_draw, Eudora FUN_0028eee0) is drawn by `panel_any` when the option is on and the shop is in browse: the
//! same steps as retail (clear the 512x128 target with FUN_001f4670, a box with the plate's fields, FUN_001eea40 twice: measure, draw
//! centred) with the text pcext ID_BUYALL_PROMPT = retail's "[Cross] Buy" word, a newline (byte 1), glyph 0x13 (Square) and a one-word
//! label that fits the plate on one line (tests below). Every other stage, and the option off: retail's panel runs. All data is
//! block-relative and identical on the 19 levels; the addresses per level come from decomp/scripts/twins (scratch_vendor/mkvendor.py).
//! RAC_VENDOR_OFF=1 (tests): the hooks are not registered at all, retail's own shop runs.
use super::pcext;
use ee_oracle::cpu::Cpu;
use ee_oracle::rec::{register_hand, Hand};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const PAD_NOW: u32 = 0x13cb04;
const BOLTS: u32 = 0x15ed98;
const AMMO: u32 = 0x13d428;
const BOUGHT: u32 = 0x13dd70;
const SQUARE: u32 = 0x80;
const ROW: u32 = 0x14;
const ID_THANKS: u32 = 0x4f5f;
const ID_NO_BOLTS: u32 = 0x4f60;
const COLOUR: u64 = 0xffff_ffff_80f0_f0f0;

/// One level's addresses.
struct VLevel {
	stage: u32,
	sel: u32,
	rows: u32,
	alt: u32,
	count: u32,
	vendor: u32,
	phase: u32,
	tick: u32,
	exitp: u32,
	p0: u32,
	add: u32,
	port: u32,
	reb: u32,
	text: u32,
	ticker: u32,
	snd: u32,
	entry: u32,
	panel: u32,
	boxt: u32,
	clear: u32,
}

#[rustfmt::skip]
static LV: [VLevel; 19] = [
VLevel { stage: 0x1ca49c, sel: 0x1ca498, rows: 0x1ca510, alt: 0x1ca480, count: 0x1ca650, vendor: 0x1ca45c, phase: 0x1ca440, tick: 0x1ca444, exitp: 0x161e18, p0: 0x1c4030, add: 0x233a20, port: 0x29ae80, reb: 0x299da8, text: 0x203620, ticker: 0x29aa18, snd: 0x28d590, entry: 0x29b420, panel: 0x29e068, boxt: 0x1fc058, clear: 0x201110 }, // 0 veldin
	VLevel { stage: 0x1ca89c, sel: 0x1ca898, rows: 0x1ca910, alt: 0x1ca880, count: 0x1caa50, vendor: 0x1ca85c, phase: 0x1ca840, tick: 0x1ca844, exitp: 0x162218, p0: 0x1c4430, add: 0x249040, port: 0x2aeb78, reb: 0x2ad820, text: 0x2254b0, ticker: 0x2ae710, snd: 0x2a0f48, entry: 0x2af118, panel: 0x2b1d60, boxt: 0x21dbd8, clear: 0x222fa0 }, // 1 novalis
	VLevel { stage: 0x1caa1c, sel: 0x1caa18, rows: 0x1caa90, alt: 0x1caa00, count: 0x1cabd0, vendor: 0x1ca9dc, phase: 0x1ca9c0, tick: 0x1ca9c4, exitp: 0x162398, p0: 0x1c45b0, add: 0x236ea0, port: 0x29a610, reb: 0x299538, text: 0x214608, ticker: 0x29a1a8, snd: 0x28cbf8, entry: 0x29abb0, panel: 0x29d7f8, boxt: 0x20d010, clear: 0x2120f8 }, // 2 aridia
	VLevel { stage: 0x1ca51c, sel: 0x1ca518, rows: 0x1ca590, alt: 0x1ca500, count: 0x1ca6d0, vendor: 0x1ca4dc, phase: 0x1ca4c0, tick: 0x1ca4c4, exitp: 0x161e98, p0: 0x1c40b0, add: 0x223178, port: 0x287750, reb: 0x286678, text: 0x1fcf38, ticker: 0x2872e8, snd: 0x279d60, entry: 0x287cf0, panel: 0x28a938, boxt: 0x1f5940, clear: 0x1faa28 }, // 3 kerwan
	VLevel { stage: 0x1ca59c, sel: 0x1ca598, rows: 0x1ca610, alt: 0x1ca580, count: 0x1ca750, vendor: 0x1ca55c, phase: 0x1ca540, tick: 0x1ca544, exitp: 0x161f18, p0: 0x1c4130, add: 0x226f80, port: 0x28bcf8, reb: 0x28ac20, text: 0x1f6b80, ticker: 0x28b890, snd: 0x27e470, entry: 0x28c298, panel: 0x28eee0, boxt: 0x1eea40, clear: 0x1f4670 }, // 4 eudora
	VLevel { stage: 0x1ca91c, sel: 0x1ca918, rows: 0x1ca990, alt: 0x1ca900, count: 0x1caad0, vendor: 0x1ca8dc, phase: 0x1ca8c0, tick: 0x1ca8c4, exitp: 0x162218, p0: 0x1c44b0, add: 0x25cda0, port: 0x2c35b8, reb: 0x2c24e0, text: 0x232308, ticker: 0x2c3150, snd: 0x2b5c38, entry: 0x2c3b58, panel: 0x2c67a0, boxt: 0x22ad10, clear: 0x22fdf8 }, // 5 rilgar
	VLevel { stage: 0x1cab9c, sel: 0x1cab98, rows: 0x1cac10, alt: 0x1cab80, count: 0x1cad50, vendor: 0x1cab5c, phase: 0x1cab40, tick: 0x1cab44, exitp: 0x162518, p0: 0x1c4730, add: 0x243760, port: 0x2a7ef8, reb: 0x2a6e20, text: 0x21b640, ticker: 0x2a7a90, snd: 0x29a600, entry: 0x2a8498, panel: 0x2ab0e0, boxt: 0x213cc0, clear: 0x219130 }, // 6 nebula
	VLevel { stage: 0x1ca49c, sel: 0x1ca498, rows: 0x1ca510, alt: 0x1ca480, count: 0x1ca650, vendor: 0x1ca45c, phase: 0x1ca440, tick: 0x1ca444, exitp: 0x161e18, p0: 0x1c4030, add: 0x25b808, port: 0x2c1aa8, reb: 0x2c09d0, text: 0x22b408, ticker: 0x2c1640, snd: 0x2b41d0, entry: 0x2c2048, panel: 0x2c4c90, boxt: 0x223e10, clear: 0x228ef8 }, // 7 umbris
	VLevel { stage: 0x1cac1c, sel: 0x1cac18, rows: 0x1cac90, alt: 0x1cac00, count: 0x1cadd0, vendor: 0x1cabdc, phase: 0x1cabc0, tick: 0x1cabc4, exitp: 0x162598, p0: 0x1c47b0, add: 0x23ec18, port: 0x2a3228, reb: 0x2a2150, text: 0x216120, ticker: 0x2a2dc0, snd: 0x295810, entry: 0x2a37c8, panel: 0x2a6410, boxt: 0x20eae8, clear: 0x213c10 }, // 8 batalia
	VLevel { stage: 0x1ca59c, sel: 0x1ca598, rows: 0x1ca610, alt: 0x1ca580, count: 0x1ca750, vendor: 0x1ca55c, phase: 0x1ca540, tick: 0x1ca544, exitp: 0x161f18, p0: 0x1c4130, add: 0x253680, port: 0x2b7750, reb: 0x2b6678, text: 0x223280, ticker: 0x2b72e8, snd: 0x2a9dd0, entry: 0x2b7cf0, panel: 0x2ba938, boxt: 0x21b390, clear: 0x220d70 }, // 9 gaspar
	VLevel { stage: 0x1ca89c, sel: 0x1ca898, rows: 0x1ca910, alt: 0x1ca880, count: 0x1caa50, vendor: 0x1ca85c, phase: 0x1ca840, tick: 0x1ca844, exitp: 0x162218, p0: 0x1c4430, add: 0x227008, port: 0x28b900, reb: 0x28a828, text: 0x1f6c08, ticker: 0x28b498, snd: 0x27dfa0, entry: 0x28bea0, panel: 0x28eae8, boxt: 0x1ef640, clear: 0x1f46f8 }, // 10 orxon
	VLevel { stage: 0x1cae1c, sel: 0x1cae18, rows: 0x1cae90, alt: 0x1cae00, count: 0x1cafd0, vendor: 0x1caddc, phase: 0x1cadc0, tick: 0x1cadc4, exitp: 0x162798, p0: 0x1c49b0, add: 0x25a3d8, port: 0x2bdd00, reb: 0x2bcc28, text: 0x234f08, ticker: 0x2bd898, snd: 0x2b03f0, entry: 0x2be2a0, panel: 0x2c0ee8, boxt: 0x22d940, clear: 0x2329f8 }, // 11 pokitaru
	VLevel { stage: 0x1ca89c, sel: 0x1ca898, rows: 0x1ca910, alt: 0x1ca880, count: 0x1caa50, vendor: 0x1ca85c, phase: 0x1ca840, tick: 0x1ca844, exitp: 0x162218, p0: 0x1c4430, add: 0x24d218, port: 0x2b1690, reb: 0x2b05b8, text: 0x2264b0, ticker: 0x2b1228, snd: 0x2a3d10, entry: 0x2b1c30, panel: 0x2b4878, boxt: 0x21eb98, clear: 0x223fa0 }, // 12 hoven
	VLevel { stage: 0x1ca71c, sel: 0x1ca718, rows: 0x1ca790, alt: 0x1ca700, count: 0x1ca8d0, vendor: 0x1ca6dc, phase: 0x1ca6c0, tick: 0x1ca6c4, exitp: 0x162098, p0: 0x1c42b0, add: 0x23f488, port: 0x2a5490, reb: 0x2a4138, text: 0x20f088, ticker: 0x2a5028, snd: 0x2978c8, entry: 0x2a5a30, panel: 0x2a8678, boxt: 0x207ac0, clear: 0x20cb78 }, // 13 oltanis_orbit
	VLevel { stage: 0x1cab9c, sel: 0x1cab98, rows: 0x1cac10, alt: 0x1cab80, count: 0x1cad50, vendor: 0x1cab5c, phase: 0x1cab40, tick: 0x1cab44, exitp: 0x162518, p0: 0x1c4730, add: 0x23cd28, port: 0x2a1230, reb: 0x2a0158, text: 0x216c30, ticker: 0x2a0dc8, snd: 0x2937f0, entry: 0x2a17d0, panel: 0x2a4418, boxt: 0x20f2e8, clear: 0x214720 }, // 14 oltanis
	VLevel { stage: 0x1caa1c, sel: 0x1caa18, rows: 0x1caa90, alt: 0x1caa00, count: 0x1cabd0, vendor: 0x1ca9dc, phase: 0x1ca9c0, tick: 0x1ca9c4, exitp: 0x162398, p0: 0x1c45b0, add: 0x2245a0, port: 0x288d50, reb: 0x287c78, text: 0x1fcd18, ticker: 0x2888e8, snd: 0x27b3f0, entry: 0x2892f0, panel: 0x28bf38, boxt: 0x1f5710, clear: 0x1fa808 }, // 15 quartu
	VLevel { stage: 0x1ca89c, sel: 0x1ca898, rows: 0x1ca910, alt: 0x1ca880, count: 0x1caa50, vendor: 0x1ca85c, phase: 0x1ca840, tick: 0x1ca844, exitp: 0x162198, p0: 0x1c4430, add: 0x22f430, port: 0x293d58, reb: 0x292c80, text: 0x201e08, ticker: 0x2938f0, snd: 0x286340, entry: 0x2942f8, panel: 0x296f40, boxt: 0x1fa840, clear: 0x1ff8f8 }, // 16 kalebo
	VLevel { stage: 0x1cad1c, sel: 0x1cad18, rows: 0x1cad90, alt: 0x1cad00, count: 0x1caed0, vendor: 0x1cacdc, phase: 0x1cacc0, tick: 0x1cacc4, exitp: 0x162698, p0: 0x1c48b0, add: 0x22ec88, port: 0x293060, reb: 0x291f88, text: 0x200648, ticker: 0x292bf8, snd: 0x285770, entry: 0x293600, panel: 0x296248, boxt: 0x1f9040, clear: 0x1fe138 }, // 17 veldin_orbit
	VLevel { stage: 0x1cae1c, sel: 0x1cae18, rows: 0x1cae90, alt: 0x1cae00, count: 0x1cafd0, vendor: 0x1caddc, phase: 0x1cadc0, tick: 0x1cadc4, exitp: 0x162798, p0: 0x1c49b0, add: 0x236f88, port: 0x29a9b0, reb: 0x2998d8, text: 0x20c388, ticker: 0x29a548, snd: 0x28cfc0, entry: 0x29af50, panel: 0x29db98, boxt: 0x204dc0, clear: 0x209e78 }, // 18 veldin_finale
];
/// `LV` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
#[rustfmt::skip]
static LV_USA: [VLevel; 19] = [
VLevel { stage: 0x1ca51c, sel: 0x1ca518, rows: 0x1ca590, alt: 0x1ca500, count: 0x1ca6d0, vendor: 0x1ca4dc, phase: 0x1ca4c0, tick: 0x1ca4c4, exitp: 0x161e18, p0: 0x1c40b0, add: 0x233e40, port: 0x29b4d8, reb: 0x29a400, text: 0x203ad8, ticker: 0x29b070, snd: 0x28dbe8, entry: 0x29ba78, panel: 0x29e6c0, boxt: 0x1fc4a8, clear: 0x201560 }, // 0 veldin
	VLevel { stage: 0x1ca99c, sel: 0x1ca998, rows: 0x1caa10, alt: 0x1ca980, count: 0x1cab50, vendor: 0x1ca95c, phase: 0x1ca940, tick: 0x1ca944, exitp: 0x162298, p0: 0x1c4530, add: 0x2494d8, port: 0x2af248, reb: 0x2adef0, text: 0x2259e8, ticker: 0x2aede0, snd: 0x2a1618, entry: 0x2af7e8, panel: 0x2b2430, boxt: 0x21e0a8, clear: 0x223470 }, // 1 novalis
	VLevel { stage: 0x1cab1c, sel: 0x1cab18, rows: 0x1cab90, alt: 0x1cab00, count: 0x1cacd0, vendor: 0x1caadc, phase: 0x1caac0, tick: 0x1caac4, exitp: 0x162418, p0: 0x1c46b0, add: 0x237338, port: 0x29ace0, reb: 0x299c08, text: 0x214b40, ticker: 0x29a878, snd: 0x28d2c8, entry: 0x29b280, panel: 0x29dec8, boxt: 0x20d4e0, clear: 0x2125c8 }, // 2 aridia
	VLevel { stage: 0x1ca61c, sel: 0x1ca618, rows: 0x1ca690, alt: 0x1ca600, count: 0x1ca7d0, vendor: 0x1ca5dc, phase: 0x1ca5c0, tick: 0x1ca5c4, exitp: 0x161f18, p0: 0x1c41b0, add: 0x223610, port: 0x287e20, reb: 0x286d48, text: 0x1fd470, ticker: 0x2879b8, snd: 0x27a430, entry: 0x2883c0, panel: 0x28b008, boxt: 0x1f5e10, clear: 0x1faef8 }, // 3 kerwan
	VLevel { stage: 0x1ca69c, sel: 0x1ca698, rows: 0x1ca710, alt: 0x1ca680, count: 0x1ca850, vendor: 0x1ca65c, phase: 0x1ca640, tick: 0x1ca644, exitp: 0x161f98, p0: 0x1c4230, add: 0x227420, port: 0x28c3d0, reb: 0x28b2f8, text: 0x1f70b8, ticker: 0x28bf68, snd: 0x27eb48, entry: 0x28c970, panel: 0x28f5b8, boxt: 0x1eef10, clear: 0x1f4b40 }, // 4 eudora
	VLevel { stage: 0x1ca99c, sel: 0x1ca998, rows: 0x1caa10, alt: 0x1ca980, count: 0x1cab50, vendor: 0x1ca95c, phase: 0x1ca940, tick: 0x1ca944, exitp: 0x162218, p0: 0x1c4530, add: 0x25d1b8, port: 0x2c3c28, reb: 0x2c2b50, text: 0x2327c0, ticker: 0x2c37c0, snd: 0x2b62a8, entry: 0x2c41c8, panel: 0x2c6e10, boxt: 0x22b160, clear: 0x230248 }, // 5 rilgar
	VLevel { stage: 0x1cac9c, sel: 0x1cac98, rows: 0x1cad10, alt: 0x1cac80, count: 0x1cae50, vendor: 0x1cac5c, phase: 0x1cac40, tick: 0x1cac44, exitp: 0x162598, p0: 0x1c4830, add: 0x243c00, port: 0x2a85d0, reb: 0x2a74f8, text: 0x21bb78, ticker: 0x2a8168, snd: 0x29acd8, entry: 0x2a8b70, panel: 0x2ab7b8, boxt: 0x214190, clear: 0x219600 }, // 6 nebula
	VLevel { stage: 0x1ca59c, sel: 0x1ca598, rows: 0x1ca610, alt: 0x1ca580, count: 0x1ca750, vendor: 0x1ca55c, phase: 0x1ca540, tick: 0x1ca544, exitp: 0x161e98, p0: 0x1c4130, add: 0x25bca8, port: 0x2c2180, reb: 0x2c10a8, text: 0x22b940, ticker: 0x2c1d18, snd: 0x2b48a8, entry: 0x2c2720, panel: 0x2c5368, boxt: 0x2242e0, clear: 0x2293c8 }, // 7 umbris
	VLevel { stage: 0x1cad1c, sel: 0x1cad18, rows: 0x1cad90, alt: 0x1cad00, count: 0x1caed0, vendor: 0x1cacdc, phase: 0x1cacc0, tick: 0x1cacc4, exitp: 0x162618, p0: 0x1c48b0, add: 0x23f0b0, port: 0x2a38f8, reb: 0x2a2820, text: 0x216658, ticker: 0x2a3490, snd: 0x295ee0, entry: 0x2a3e98, panel: 0x2a6ae0, boxt: 0x20efb8, clear: 0x2140e0 }, // 8 batalia
	VLevel { stage: 0x1ca69c, sel: 0x1ca698, rows: 0x1ca710, alt: 0x1ca680, count: 0x1ca850, vendor: 0x1ca65c, phase: 0x1ca640, tick: 0x1ca644, exitp: 0x161f98, p0: 0x1c4230, add: 0x253b20, port: 0x2b7e28, reb: 0x2b6d50, text: 0x2237b8, ticker: 0x2b79c0, snd: 0x2aa4a8, entry: 0x2b83c8, panel: 0x2bb010, boxt: 0x21b860, clear: 0x221240 }, // 9 gaspar
	VLevel { stage: 0x1ca99c, sel: 0x1ca998, rows: 0x1caa10, alt: 0x1ca980, count: 0x1cab50, vendor: 0x1ca95c, phase: 0x1ca940, tick: 0x1ca944, exitp: 0x162298, p0: 0x1c4530, add: 0x2274a8, port: 0x28bfd8, reb: 0x28af00, text: 0x1f7140, ticker: 0x28bb70, snd: 0x27e678, entry: 0x28c578, panel: 0x28f1c0, boxt: 0x1efb10, clear: 0x1f4bc8 }, // 10 orxon
	VLevel { stage: 0x1caf1c, sel: 0x1caf18, rows: 0x1caf90, alt: 0x1caf00, count: 0x1cb0d0, vendor: 0x1caedc, phase: 0x1caec0, tick: 0x1caec4, exitp: 0x162818, p0: 0x1c4ab0, add: 0x25a870, port: 0x2be3d0, reb: 0x2bd2f8, text: 0x235440, ticker: 0x2bdf68, snd: 0x2b0ac0, entry: 0x2be970, panel: 0x2c15b8, boxt: 0x22de10, clear: 0x232ec8 }, // 11 pokitaru
	VLevel { stage: 0x1ca99c, sel: 0x1ca998, rows: 0x1caa10, alt: 0x1ca980, count: 0x1cab50, vendor: 0x1ca95c, phase: 0x1ca940, tick: 0x1ca944, exitp: 0x162298, p0: 0x1c4530, add: 0x24d6b0, port: 0x2b1d60, reb: 0x2b0c88, text: 0x2269e8, ticker: 0x2b18f8, snd: 0x2a43e0, entry: 0x2b2300, panel: 0x2b4f48, boxt: 0x21f068, clear: 0x224470 }, // 12 hoven
	VLevel { stage: 0x1ca81c, sel: 0x1ca818, rows: 0x1ca890, alt: 0x1ca800, count: 0x1ca9d0, vendor: 0x1ca7dc, phase: 0x1ca7c0, tick: 0x1ca7c4, exitp: 0x162118, p0: 0x1c43b0, add: 0x23f928, port: 0x2a5b68, reb: 0x2a4810, text: 0x20f5c0, ticker: 0x2a5700, snd: 0x297fa0, entry: 0x2a6108, panel: 0x2a8d50, boxt: 0x207f90, clear: 0x20d048 }, // 13 oltanis_orbit
	VLevel { stage: 0x1cac1c, sel: 0x1cac18, rows: 0x1cac90, alt: 0x1cac00, count: 0x1cadd0, vendor: 0x1cabdc, phase: 0x1cabc0, tick: 0x1cabc4, exitp: 0x162518, p0: 0x1c47b0, add: 0x23d140, port: 0x2a1880, reb: 0x2a07a8, text: 0x2170e8, ticker: 0x2a1418, snd: 0x293e40, entry: 0x2a1e20, panel: 0x2a4a68, boxt: 0x20f738, clear: 0x214b70 }, // 14 oltanis
	VLevel { stage: 0x1cab1c, sel: 0x1cab18, rows: 0x1cab90, alt: 0x1cab00, count: 0x1cacd0, vendor: 0x1caadc, phase: 0x1caac0, tick: 0x1caac4, exitp: 0x162418, p0: 0x1c46b0, add: 0x224a38, port: 0x289420, reb: 0x288348, text: 0x1fd250, ticker: 0x288fb8, snd: 0x27bac0, entry: 0x2899c0, panel: 0x28c608, boxt: 0x1f5be0, clear: 0x1facd8 }, // 15 quartu
	VLevel { stage: 0x1ca99c, sel: 0x1ca998, rows: 0x1caa10, alt: 0x1ca980, count: 0x1cab50, vendor: 0x1ca95c, phase: 0x1ca940, tick: 0x1ca944, exitp: 0x162218, p0: 0x1c4530, add: 0x22f8c8, port: 0x294448, reb: 0x293370, text: 0x202340, ticker: 0x293fe0, snd: 0x286a30, entry: 0x2949e8, panel: 0x297630, boxt: 0x1fad10, clear: 0x1ffdc8 }, // 16 kalebo
	VLevel { stage: 0x1cae1c, sel: 0x1cae18, rows: 0x1cae90, alt: 0x1cae00, count: 0x1cafd0, vendor: 0x1caddc, phase: 0x1cadc0, tick: 0x1cadc4, exitp: 0x162718, p0: 0x1c49b0, add: 0x22f128, port: 0x293738, reb: 0x292660, text: 0x200b80, ticker: 0x2932d0, snd: 0x285e48, entry: 0x293cd8, panel: 0x296920, boxt: 0x1f9510, clear: 0x1fe608 }, // 17 veldin_orbit
	VLevel { stage: 0x1caf1c, sel: 0x1caf18, rows: 0x1caf90, alt: 0x1caf00, count: 0x1cb0d0, vendor: 0x1caedc, phase: 0x1caec0, tick: 0x1caec4, exitp: 0x162818, p0: 0x1c4ab0, add: 0x237420, port: 0x29b080, reb: 0x299fa8, text: 0x20c8c0, ticker: 0x29ac18, snd: 0x28d690, entry: 0x29b620, panel: 0x29e268, boxt: 0x205290, clear: 0x20a348 }, // 18 veldin_finale
];

#[rustfmt::skip]
static HANDS: [Hand; 38] = [
	Hand { entry: 0x29b420, end: 0x29bff0, hash: 0x5d652cf21c35d259, est: 0, f: vendor_any }, // 0 veldin
	Hand { entry: 0x2af118, end: 0x2afce8, hash: 0x8f72f2c61fcb86bf, est: 0, f: vendor_any }, // 1 novalis
	Hand { entry: 0x29abb0, end: 0x29b780, hash: 0xaf382dae97cf4c38, est: 0, f: vendor_any }, // 2 aridia
	Hand { entry: 0x287cf0, end: 0x2888c0, hash: 0xbb97fdfec3377a56, est: 0, f: vendor_any }, // 3 kerwan
	Hand { entry: 0x28c298, end: 0x28ce68, hash: 0x7ae01c6fb48a8497, est: 0, f: vendor_any }, // 4 eudora
	Hand { entry: 0x2c3b58, end: 0x2c4728, hash: 0x6713e38fc64e0494, est: 0, f: vendor_any }, // 5 rilgar
	Hand { entry: 0x2a8498, end: 0x2a9068, hash: 0xa25e08cd794bc70e, est: 0, f: vendor_any }, // 6 nebula
	Hand { entry: 0x2c2048, end: 0x2c2c18, hash: 0x607dc78cd65c5a34, est: 0, f: vendor_any }, // 7 umbris
	Hand { entry: 0x2a37c8, end: 0x2a4398, hash: 0x6e756ea0012926b6, est: 0, f: vendor_any }, // 8 batalia
	Hand { entry: 0x2b7cf0, end: 0x2b88c0, hash: 0x3d0862204ea80e95, est: 0, f: vendor_any }, // 9 gaspar
	Hand { entry: 0x28bea0, end: 0x28ca70, hash: 0x9a2c5e387447170d, est: 0, f: vendor_any }, // 10 orxon
	Hand { entry: 0x2be2a0, end: 0x2bee70, hash: 0xddd885bb87774932, est: 0, f: vendor_any }, // 11 pokitaru
	Hand { entry: 0x2b1c30, end: 0x2b2800, hash: 0x0beaccc7af202967, est: 0, f: vendor_any }, // 12 hoven
	Hand { entry: 0x2a5a30, end: 0x2a6600, hash: 0x9dd1a4428139d8c4, est: 0, f: vendor_any }, // 13 oltanis_orbit
	Hand { entry: 0x2a17d0, end: 0x2a23a0, hash: 0xcea6ee7999563110, est: 0, f: vendor_any }, // 14 oltanis
	Hand { entry: 0x2892f0, end: 0x289ec0, hash: 0x619a10a1b502f7c8, est: 0, f: vendor_any }, // 15 quartu
	Hand { entry: 0x2942f8, end: 0x294ec8, hash: 0x2aa49f9f01647b71, est: 0, f: vendor_any }, // 16 kalebo
	Hand { entry: 0x293600, end: 0x2941d0, hash: 0xcf69f57a4007fa15, est: 0, f: vendor_any }, // 17 veldin_orbit
	Hand { entry: 0x29af50, end: 0x29bb20, hash: 0x4266de7473b54af2, est: 0, f: vendor_any }, // 18 veldin_finale
	Hand { entry: 0x29e068, end: 0x29e2c0, hash: 0x1ee811931e15f55c, est: 0, f: panel_any }, // 0 veldin
	Hand { entry: 0x2b1d60, end: 0x2b1fb8, hash: 0x59f5a881917c8548, est: 0, f: panel_any }, // 1 novalis
	Hand { entry: 0x29d7f8, end: 0x29da50, hash: 0x112c42d79929b93e, est: 0, f: panel_any }, // 2 aridia
	Hand { entry: 0x28a938, end: 0x28ab90, hash: 0x038bef7b9812d4da, est: 0, f: panel_any }, // 3 kerwan
	Hand { entry: 0x28eee0, end: 0x28f138, hash: 0xd2277184ec7a49f4, est: 0, f: panel_any }, // 4 eudora
	Hand { entry: 0x2c67a0, end: 0x2c69f8, hash: 0xe86765feed5f57fe, est: 0, f: panel_any }, // 5 rilgar
	Hand { entry: 0x2ab0e0, end: 0x2ab338, hash: 0x4b542ac452fe8c24, est: 0, f: panel_any }, // 6 nebula
	Hand { entry: 0x2c4c90, end: 0x2c4ee8, hash: 0xc6d3fbfeb1da8fbe, est: 0, f: panel_any }, // 7 umbris
	Hand { entry: 0x2a6410, end: 0x2a6668, hash: 0xf4ecb42f04e84c64, est: 0, f: panel_any }, // 8 batalia
	Hand { entry: 0x2ba938, end: 0x2bab90, hash: 0x3a6a2d4d0250ab1c, est: 0, f: panel_any }, // 9 gaspar
	Hand { entry: 0x28eae8, end: 0x28ed40, hash: 0xbc9488fefcabd3be, est: 0, f: panel_any }, // 10 orxon
	Hand { entry: 0x2c0ee8, end: 0x2c1140, hash: 0x65ba6d8e175348fe, est: 0, f: panel_any }, // 11 pokitaru
	Hand { entry: 0x2b4878, end: 0x2b4ad0, hash: 0x23775d795f670fc8, est: 0, f: panel_any }, // 12 hoven
	Hand { entry: 0x2a8678, end: 0x2a88d0, hash: 0x6664f1b1432e325e, est: 0, f: panel_any }, // 13 oltanis_orbit
	Hand { entry: 0x2a4418, end: 0x2a4670, hash: 0xfb61a5a646bf7d70, est: 0, f: panel_any }, // 14 oltanis
	Hand { entry: 0x28bf38, end: 0x28c190, hash: 0x012e247c8d296cc2, est: 0, f: panel_any }, // 15 quartu
	Hand { entry: 0x296f40, end: 0x297198, hash: 0x9419f3426a452f3e, est: 0, f: panel_any }, // 16 kalebo
	Hand { entry: 0x296248, end: 0x2964a0, hash: 0x544008046b6e7ace, est: 0, f: panel_any }, // 17 veldin_orbit
	Hand { entry: 0x29db98, end: 0x29ddf0, hash: 0xee3015a510bd5f9e, est: 0, f: panel_any }, // 18 veldin_finale
];

/// Registers the hooks (Retail::load, after pcext's text hand). RAC_VENDOR_OFF=1 (tests): none, retail's own shop.
pub fn register() {
	if pcext::enabled() && std::env::var_os("RAC_VENDOR_OFF").is_none() {
		register_hand(crate::retail::disc::hands(&HANDS));
	}
}

/// Re-entrancy guard (the retail functions a hook calls run the hands again, which must fall through to retail).
struct Guard(&'static AtomicBool);
impl Guard {
	fn take(f: &'static AtomicBool) -> Option<Guard> {
		(!f.swap(true, Ordering::SeqCst)).then_some(Guard(f))
	}
}
impl Drop for Guard {
	fn drop(&mut self) {
		self.0.store(false, Ordering::SeqCst);
	}
}
static G_BUY: AtomicBool = AtomicBool::new(false);
static G_PANEL: AtomicBool = AtomicBool::new(false);
/// The pad word of the last purchase-function call (u32::MAX: none yet): Square is a new press when it was clear there.
static PREV: AtomicU32 = AtomicU32::new(u32::MAX);

struct Regs {
	r: [u128; 32],
	hi: u128,
	lo: u128,
	sa: u32,
	f: [u32; 32],
	facc: u32,
	fcr31: u32,
	vf: [[u32; 4]; 32],
	vacc: [u32; 4],
	vi: [u32; 32],
}
impl Regs {
	fn save(c: &Cpu) -> Regs {
		Regs { r: c.r, hi: c.hi, lo: c.lo, sa: c.sa, f: c.f, facc: c.facc, fcr31: c.fcr31, vf: c.vf, vacc: c.vacc, vi: c.vi }
	}
	fn restore(&self, c: &mut Cpu) {
		(c.r, c.hi, c.lo, c.sa, c.f, c.facc, c.fcr31, c.vf, c.vacc, c.vi) = (self.r, self.hi, self.lo, self.sa, self.f, self.facc, self.fcr31, self.vf, self.vacc, self.vi);
	}
}

/// The purchase state machine (called once per frame by the browse driver, only while the shop is up): on a new Square press in
/// browse (stage 0, not leaving) with the option on, buys all the ammo; always declines so retail's own code runs after it.
fn vendor_any(c: &mut Cpu) -> bool {
	let Some(v) = (*crate::retail::disc::pick(&LV, &LV_USA)).iter().find(|v| v.entry == c.pc) else { return false };
	let now = c.mem.rd32(PAD_NOW);
	let prev = PREV.swap(now, Ordering::Relaxed);
	if !pcext::buyall_on() || now & SQUARE == 0 || prev & SQUARE != 0 {
		return false;
	}
	if c.mem.rd32(v.phase) != 1 || c.mem.rd32(v.stage) != 0 || c.mem.rd32(v.exitp) != 0 || c.mem.rd32(v.tick) <= 2 {
		return false;
	}
	let Some(_g) = Guard::take(&G_BUY) else { return false };
	buy_all(c, v);
	false
}

fn buy_all(c: &mut Cpu, v: &VLevel) {
	let regs = Regs::save(c);
	let alt = c.mem.rd32(v.alt) != 0;
	let n = c.mem.rd32(v.count).min(40);
	let mut items: Vec<u32> = Vec::new();
	for i in 0..n {
		if c.mem.rd32(v.rows + i * ROW + 4) == 1 {
			items.push(c.mem.rd32(v.rows + i * ROW));
		}
	}
	let (mut bought, mut broke) = (false, false);
	for item in items {
		let t = v.p0 + item * 0x18;
		let unit = c.mem.rd16(t + if alt { 0xa } else { 8 }) as i32;
		let max = c.mem.rd16(t + 0xe) as i32;
		let owned = c.mem.rd32(AMMO + 4 * item) as i32;
		if unit == 0 || owned >= max {
			continue;
		}
		let qty = (max - owned).min(c.mem.rd32(BOLTS) as i32 / unit);
		if qty <= 0 {
			broke = true;
			continue;
		}
		c.s(4, item as u64);
		c.s(5, qty as u64);
		c.call(v.add);
		let got = qty - c.g(2) as i32;
		let b = BOUGHT + 4 * item;
		let nb = (c.mem.rd32(b) as i32 + got) as u32;
		c.mem.wr32(b, nb);
		c.s(4, 3);
		c.call(v.port);
		let nbolts = (c.mem.rd32(BOLTS) as i32 - unit * got) as u32;
		c.mem.wr32(BOLTS, nbolts);
		c.s(4, alt as u64);
		c.call(v.reb);
		bought = true;
	}
	let msg = if bought {
		Some(ID_THANKS)
	} else if broke {
		Some(ID_NO_BOLTS)
	} else {
		None
	};
	if let Some(id) = msg {
		c.s(4, id as u64);
		c.call(v.text);
		let t = c.g(2);
		c.s(4, t);
		c.call(v.ticker);
	}
	c.s(4, if bought { 7 } else { 0 });
	c.s(5, 0);
	let vp = c.mem.rd32(v.vendor) as i32 as i64 as u64;
	c.s(6, vp);
	c.call(v.snd);
	regs.restore(c);
}

/// The Buy plate's content (`shop_buy_panel_draw(_, width, height)`): with the option on, in browse, the plate is drawn here: the
/// same steps as retail (clear the 512x128 target with `FUN_001f4670`, a box like the plate's own, `FUN_001eea40` twice: measure,
/// then draw centred by the measured height) but with the two-line text pcext `ID_BUYALL_PROMPT` ("[Cross] Buy", newline,
/// "[Square] Buy all ammo"), so the lines sit together in retail's own line pitch. Every other stage and the option off: retail's runs.
fn panel_any(c: &mut Cpu) -> bool {
	let Some(v) = (*crate::retail::disc::pick(&LV, &LV_USA)).iter().find(|v| v.panel == c.pc) else { return false };
	if !pcext::buyall_on() || c.mem.rd32(v.stage) != 0 {
		return false;
	}
	let Some(_g) = Guard::take(&G_PANEL) else { return false };
	let (w, h) = (c.g(5) as u32 & 0xffff, c.g(6) as u32 & 0xffff);
	let regs = Regs::save(c);
	draw_plate(c, v, w, h);
	regs.restore(c);
	true
}

/// Box fields: +2 height, +6 width, +8 half width, +0xa y, +0xe measured height (written by the measure pass), +0x10 = 0x10,
/// +0x12 flags (5 measure pass, then ^4 = 1 draw pass).
fn draw_plate(c: &mut Cpu, v: &VLevel, w: u32, h: u32) {
	for (i, a) in [0u64, 0, 0x200, 0x80, 0x200, 0x80, 0].into_iter().enumerate() {
		c.s(4 + i, a);
	}
	c.call(v.clear);
	let sp = c.g(29) as u32;
	let nsp = sp - 0x80;
	let bx = nsp + 0x40;
	for i in 0..0x18 {
		c.mem.wr8(bx + i, 0);
	}
	c.mem.wr16(bx + 2, h as u16);
	c.mem.wr16(bx + 6, w as u16);
	c.mem.wr16(bx + 8, (w >> 1) as u16);
	c.mem.wr16(bx + 0xa, ((h >> 1) as i32 - 7) as u16);
	c.mem.wr16(bx + 0x10, 0x10);
	c.mem.wr16(bx + 0x12, 5);
	c.s(4, pcext::ID_BUYALL_PROMPT as u64);
	c.call(v.text);
	let text = c.g(2);
	c.s(29, nsp as u64);
	for pass in 0..2 {
		c.s(4, bx as u64);
		c.s(5, COLOUR);
		c.s(6, text);
		c.s(7, u64::MAX);
		c.call(v.boxt);
		if pass == 0 {
			let m = c.mem.rd16(bx + 0xe) as i16 as i32;
			c.mem.wr16(bx + 0x12, 1);
			c.mem.wr16(bx + 0xa, ((h as i32 - m) >> 1) as u16);
		}
	}
	c.s(29, sp as u64);
}


#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::AtomicU32;

	/// (width << 16 | height) the retail plate function was called with, caught by a pass-through hand.
	static WH: AtomicU32 = AtomicU32::new(0);
	fn spy(c: &mut Cpu) -> bool {
		WH.store(((c.g(5) as u32 & 0xffff) << 16) | (c.g(6) as u32 & 0xffff), Ordering::Relaxed);
		false
	}

	/// The plate text of every language is exactly two lines in retail's own box measure (no third line from a wrapped label), on the
	/// open Eudora shop (savestate s12: shop in browse, stage 0), with the width retail passes to its plate function.
	#[test]
	fn plate_text_is_two_lines() {
		let dir = [concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s12")].into_iter().find(|d| std::path::Path::new(d).join("eeMemory.bin").exists());
		let Some(dir) = dir else { return };
		let mut m = ee_oracle::Machine::load(dir);
		ee_game::register();
		pcext::register();
		static SPY: [Hand; 1] = [Hand { entry: 0x28eee0, end: 0x28f138, hash: 0xd2277184ec7a49f4, est: 0, f: spy }];
		register_hand(crate::retail::disc::hands(&SPY));
		m.enable_rec();
		for _ in 0..4 {
			m.frame().expect("shop frame");
		}
		let (w, h) = (WH.load(Ordering::Relaxed) >> 16, WH.load(Ordering::Relaxed) & 0xffff);
		assert!(w > 0 && h > 0, "the retail plate function ran with a size");
		eprintln!("plate w={w} h={h}");
		let v = LV.iter().find(|v| v.entry == 0x28c298).unwrap();
		let (bx, tx) = (0xb0900u32, 0xb0a00u32);
		let c = &mut m.cpu;
		// The retail measure pass over a text: the box's measured height (+0xe).
		let mut measure = |c: &mut Cpu, text: u64| -> i32 {
			for i in 0..0x18 {
				c.mem.wr8(bx + i, 0);
			}
			c.mem.wr16(bx + 2, h as u16);
			c.mem.wr16(bx + 6, w as u16);
			c.mem.wr16(bx + 8, (w >> 1) as u16);
			c.mem.wr16(bx + 0xa, ((h >> 1) as i32 - 7) as u16);
			c.mem.wr16(bx + 0x10, 0x10);
			c.mem.wr16(bx + 0x12, 5);
			c.s(4, bx as u64);
			c.s(5, COLOUR);
			c.s(6, text);
			c.s(7, u64::MAX);
			c.call(v.boxt);
			c.mem.rd16(bx + 0xe) as i16 as i32
		};
		let mut put = |c: &mut Cpu, s: &[u8]| -> u64 {
			for (i, b) in s.iter().enumerate() {
				c.mem.wr8(tx + i as u32, *b);
			}
			c.mem.wr8(tx + s.len() as u32, 0);
			tx as u64
		};
		let one = { let t = put(c, b"X"); measure(c, t) };
		let two = { let t = put(c, b"X\x01Y"); measure(c, t) };
		eprintln!("one line {one}, two lines {two}");
		assert!(one > 0 && two == 2 * one, "the measure counts the lines");
		let mut bad = Vec::new();
		// The shop works the menu way (menu_keys::VENDOR): "Ent" buys, F buys all (a one-letter key: in the 89 px plate "Tab Remplir",
		// "Tab Ricarica" wrap, measured 2026-10-13), so the plate names those keys.
		crate::retail::prompts::test_menu(Some(crate::retail::menu_keys::VENDOR));
		for word in [0u32, 2, 3, 4, 5] {
			c.mem.wr32(0x15ed88, word);
			c.s(4, pcext::ID_BUYALL_PROMPT as u64);
			c.call(v.text);
			let p = c.g(2);
			let mut s = Vec::new();
			let mut a = p as u32;
			while c.mem.rd8(a) != 0 {
				s.push(c.mem.rd8(a));
				a += 1;
			}
			let got = measure(c, p);
			// Retail's own "[Cross] Buy" (id 0x5234, all_text.bin) in this language, measured with the same box: the prompt must be
			// that exact line (first line of the prompt) plus exactly one line. (The in-RAM table is the loaded UI language, so the
			// retail words are listed here; in Spanish the 89 px plate already wraps "Comprar", so Spanish is 2 + 1 lines.)
			let retail_raw: &[u8] = [&b" Buy"[..], b" Acheter", b" Kaufen", b" Comprar", b" Compra"][[0u32, 2, 3, 4, 5].iter().position(|&w| w == word).unwrap()];
			// Keyboard device: the plate gets key names for its glyphs (prompts::for_device), so the first line is the substituted Buy line.
			let retail_buy = crate::retail::prompts::for_device(retail_raw).unwrap_or_else(|| retail_raw.to_vec());
			assert!(s.starts_with(&retail_buy) && s[retail_buy.len()] == 1, "the prompt starts with retail's own Buy line");
			let t = put(c, &retail_buy);
			let buy = measure(c, t);
			eprintln!("language word {word}: {:?} measures {got}, retail Buy {buy}", String::from_utf8_lossy(&s));
			if got != buy + one {
				bad.push(word);
			}
		}
		crate::retail::prompts::test_menu(None);
		assert!(bad.is_empty(), "languages whose plate text is not retail's line plus one line (the label wraps): {bad:?}");
	}
	/// The open Eudora shop (savestate s12) with the shop UI ticked a few frames.
	fn open_shop() -> Option<ee_oracle::Machine> {
		let dir = [concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s12")].into_iter().find(|d| std::path::Path::new(d).join("eeMemory.bin").exists())?;
		let mut m = ee_oracle::Machine::load(dir);
		ee_game::register();
		pcext::register();
		m.enable_rec();
		for _ in 0..4 {
			m.frame().expect("shop frame");
		}
		Some(m)
	}

	/// (ammo, bought, bolts) of the ammo items 10 and 16 plus the bolts.
	fn snap(c: &mut Cpu) -> ([u32; 2], [u32; 2], u32) {
		let mut g = |a: u32| [c.mem.rd32(a + 40), c.mem.rd32(a + 64)];
		let (x, y) = (g(AMMO), g(BOUGHT));
		(x, y, c.mem.rd32(BOLTS))
	}

	/// Retail's own purchase flow, per ammo row in list order, exactly what a player does by hand with the pad: select the row, Cross
	/// (stage 0 -> quantity prompt, stage 2), quantity = the prompt's cap `min(max - owned, bolts / unit)` (stage 2 of FUN_0028c298,
	/// Eudora 0x28c6c8..0x28c778; holding Right counts up to it), Cross again (stage 3 animation, stage 4 confirm: AddAmmo, bought +=,
	/// portrait event, bolts -=, row rebuild, THANK YOU), all by running the retail frames.
	fn by_hand(m: &mut ee_oracle::Machine, v: &VLevel, items: &[u32]) {
		let press = |m: &mut ee_oracle::Machine, n: usize| {
			m.set_pad(0x40, [128; 4]);
			for _ in 0..n {
				m.frame().expect("frame");
			}
			m.set_pad(0, [128; 4]);
			for _ in 0..4 {
				m.frame().expect("frame");
			}
		};
		for &item in items {
			let c = &mut m.cpu;
			let t = v.p0 + item * 0x18;
			let unit = c.mem.rd16(t + 8) as i32;
			let max = c.mem.rd16(t + 0xe) as i32;
			let owned = c.mem.rd32(AMMO + 4 * item) as i32;
			let qty = (max - owned).min(c.mem.rd32(BOLTS) as i32 / unit);
			if owned >= max || qty <= 0 {
				continue; // retail: no prompt (full row, or not even one unit affordable): nothing changes
			}
			let n = c.mem.rd32(v.count);
			let idx = (0..n).find(|&i| c.mem.rd32(v.rows + i * ROW) == item && c.mem.rd32(v.rows + i * ROW + 4) == 1).expect("ammo row");
			c.mem.wr32(v.sel, idx);
			press(m, 2);
			assert_eq!(m.cpu.mem.rd32(v.stage), 2, "quantity prompt for item {item}");
			m.cpu.mem.wr32(0x161f34, qty as u32);
			press(m, 2);
			for _ in 0..80 {
				if m.cpu.mem.rd32(v.stage) == 0 {
					break;
				}
				m.frame().expect("frame");
			}
			assert_eq!(m.cpu.mem.rd32(v.stage), 0, "retail finished the purchase for item {item}");
		}
	}

	/// Square = buying every owned weapon's ammo by hand (retail's own purchase code per row) for plenty of bolts, too few bolts
	/// (list order: the first rows take them) and all ammo at max.
	#[test]
	fn buy_all_equals_buying_by_hand() {
		let (Some(mut a), Some(mut b)) = (open_shop(), open_shop()) else { return };
		let v = LV.iter().find(|v| v.entry == 0x28c298).unwrap();
		// (bolts, ammo of item 10, ammo of item 16, expected ammo, expected bolts)
		let cases: [(u32, u32, u32, [u32; 2], u32); 4] = [
			(20000, 30, 210, [40, 240], 19920),
			(53, 30, 210, [40, 213], 0),
			(20000, 40, 240, [40, 240], 20000),
			(20000, 0, 0, [40, 240], 20000 - 40 * 5 - 240),
		];
		for (bolts, a10, a16, want_ammo, want_bolts) in cases {
			for m in [&mut a, &mut b] {
				m.cpu.mem.wr32(BOLTS, bolts);
				m.cpu.mem.wr32(AMMO + 40, a10);
				m.cpu.mem.wr32(AMMO + 64, a16);
			}
			let before = snap(&mut a.cpu);
			buy_all(&mut a.cpu, v);
			by_hand(&mut b, v, &[10, 16]);
			let (x, y) = (snap(&mut a.cpu), snap(&mut b.cpu));
			eprintln!("bolts {bolts} ammo {a10}/{a16}: before {before:?} buy_all {x:?} by hand {y:?}");
			assert_eq!(x, y, "buy_all equals the by-hand purchases");
			assert_eq!((x.0, x.2), (want_ammo, want_bolts), "expected numbers");
		}
	}
}
