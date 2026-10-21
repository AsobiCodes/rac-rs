//! Mine Glove (held class 190, mine class 74), decomp/notes/weapons_framework.md "lane 5: Mine Glove": the held updater
//! FUN_002d7068 with its ballistic helper, the mine creator and launch (WPN-6.4a), the class-74 updater FUN_002bf770
//! (WPN-6.4b1 frame and bob, 6.4b2 flight contact / plant / moving-solid ride, 6.4b3 arming, chase and push-outs, 6.4b4
//! landing preview), its impact and removal tail (WPN-6.4c; the effects visuals WPN-6.4c-fx are not mapped) and the mine
//! registry 0x1b0bb0 (WPN-6.4d). The water coupling of the updater is NOV-20.2 (decomp/notes/water.md).


use super::*;

/// The drawn lift: subtracted at the head of the update, added back with the bob (OV:120128, OV:120675).
const LIFT: f32 = 0.4;



/// The mine class.
pub const MINE: u32 = 74;
/// The pooled class-74 props.
const POOL: usize = 10;













pub(super) fn horizontal(a: V3, b: V3) -> f32 {
	length([a[0] - b[0], a[1] - b[1], 0.0])
}


