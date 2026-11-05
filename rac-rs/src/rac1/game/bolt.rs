//! The bolt pickup state machine (FUN_002bb088, rest step FUN_002bc098, attract FUN_002bc4c0) over `BoltSt`:
//! decomp/notes/novalis_checklist.md NOV-14.7 / 14.7b. The reward creator FUN_002bc6e8 init (scale, light copy, spin draws and quaternion, NOV-14.7e) is in `Game::drop_bolts`; its packing/adaptive part (NOV-9) is not.

use super::pickups::bolt_wrap;
use super::*;










pub(super) fn set_len(v: V3, l: f32) -> V3 {
	let n = length(v);
	if n == 0.0 { [0.0; 3] } else { scale(v, l / n) }
}


/// FUN_00221b28 / FUN_00221b70: one-step wrap of `a + b` / `a - b` into [-pi, pi).
pub(super) fn wrap_add(a: f32, b: f32) -> f32 {
	bolt_wrap(a + b)
}

pub(super) fn wrap_sub(a: f32, b: f32) -> f32 {
	bolt_wrap(a - b)
}



