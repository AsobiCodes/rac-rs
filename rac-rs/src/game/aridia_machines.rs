//! Aridia machinery and the rocket launcher (decomp/notes/aridia_environment.md): ARI-26 bulldozer 656 (FUN_002dc340),
//! grab crane 675 (FUN_002dcca0) and crane 732 (FUN_002dead8); ARI-27a rocket launcher 733 (FUN_002ded08, prelude
//! FUN_002df060) and its rocket 1208 (creator FUN_002ebb58, update FUN_002eb850); ARI-27b fixed-level boom cues and the
//! smoke byte +9. The actor variants live in `aridia::Actor`; `Game::update_aridia` dispatches here.

use super::aridia::CRANE;
use super::aridia::LAUNCHER;
use super::{sched, Event, Game, Kind, Prop};
use crate::gameplay::pal_ticks;
use crate::gameplay::PAL_DT;
use crate::gameplay::PAL_DT2;
use crate::particles::{Blast, Rng};
use crate::physics::{length, sub, vmul, V3};
use crate::shadows::fast_atan2;







