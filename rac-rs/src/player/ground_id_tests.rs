//! Notes for the hero ground-face-id and free-look tests (NOV-12b): imports and comments only.
//! Test-only (`cfg(test)`) leftover from the retired hand port; the test bodies are gone. Do not extend.
use super::*;
use crate::gameplay::PAL_DT;

// NOV-12b test (2): any state change clears the free-look byte; in the look state with it set the yaw springs to the
// camera yaw, otherwise it is left alone.
// NOV-12b: entering look state 1 reloads the look countdown to pal_ticks(60) = 50; state 0x1e leaves it; a respawn
// keeps the countdown and the session count.