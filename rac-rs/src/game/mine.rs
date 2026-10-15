//! Mine Glove (held class 190, mine class 74), decomp/notes/weapons_framework.md "lane 5: Mine Glove": the held updater
//! FUN_002d7068 with its ballistic helper, the mine creator and launch (WPN-6.4a), the class-74 updater FUN_002bf770
//! (WPN-6.4b1 frame and bob, 6.4b2 flight contact / plant / moving-solid ride, 6.4b3 arming, chase and push-outs, 6.4b4
//! landing preview), its impact and removal tail (WPN-6.4c; the effects visuals WPN-6.4c-fx are not mapped) and the mine
//! registry 0x1b0bb0 (WPN-6.4d). The water coupling of the updater is NOV-20.2 (decomp/notes/water.md).

#![allow(dead_code)]

use super::*;
use crate::camera::{fast_cos, fast_sin};
use crate::gameplay::{pal_ticks, PAL_DT, PAL_DT2};
use crate::particles::euler_m;
use crate::particles::polar;
use crate::particles::Blast;
use crate::particles::Rng;
use crate::particles::Spawn;
use crate::inventory::MINE_GLOVE;
use crate::physics::{Hit, Scene};
use crate::shadows::fast_atan2;
use super::bolt::set_len;
use super::mouse::wrap_angle;

/// The drawn lift: subtracted at the head of the update, added back with the bob (OV:120128, OV:120675).
const LIFT: f32 = 0.4;



/// The mine class.
pub const MINE: u32 = 74;
/// The pooled class-74 props.
const POOL: usize = 10;













pub(super) fn horizontal(a: V3, b: V3) -> f32 {
	length([a[0] - b[0], a[1] - b[1], 0.0])
}


