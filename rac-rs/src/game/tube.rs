//! The Suck Cannon's tube (decomp/notes/weapons_framework.md WPN-5.1a): ten rings of twenty vertices that bend along a
//! spring-driven path from the muzzle, the alpha ramp that follows the ray, the puffs that travel up the tube and the
//! quads drawn through it. Per-tick units throughout (retail game ticks at 50 Hz).

use super::suckcomp::qmul;
use super::Game;
use crate::gameplay::pal_ticks;
use crate::particles::PoolSlot;
use crate::particles::Rng;
use crate::particles::Spawn;
use crate::particles::WorldQuad;
use crate::physics::{add, cross, dot, length, scale, sub, Scene, World, V3};














