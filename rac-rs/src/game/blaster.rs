//! Blaster (item 15): the held class 168 updater (WPN-6.1a) with its third-person lock-on search (WPN-6.1b-aim), the
//! class 305 shot creator (WPN-6.1b), the shot flight and finish (WPN-6.1c) and the two hit writers (WPN-6.1d)
//! (decomp/notes/weapons_framework.md "WPN-6 port specs", lanes 1 and 2).
//!
//! Port carriers [I]: the held rig is not a prop, so the hit records name no attacker prop and the segment queries
//! ignore nothing; the target component (height, radius byte) has no Rust carrier and takes its authored Novalis
//! values (height 0.5, radius byte 0); the latch counter 0x13f50c lives here (`BlasterHeld.latch_t`).

use super::*;
use crate::camera::{fast_cos, fast_sin};
use crate::gameplay::{pal_ticks, PAL_DT, PAL_DT2};
use crate::particles::PointLight;
use crate::particles::PoolSlot;
use crate::particles::Rng;
use crate::particles::Spawn;
use crate::physics::cross;
use crate::shadows::fast_atan2;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// The held class.
pub const HELD: u32 = 168;

/// The shot class.
pub const SHOT: u32 = 305;












