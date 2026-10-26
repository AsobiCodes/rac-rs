//! Hit and attack code of the old hand-port `Game`: only an empty `impl Game` and an empty `attack` module (the collector-query
//! (radius, mask) table per owner) are left.
//! Leftover from the retired hand port; still compiled because retail mode uses part of `game::Game` (the prop pools). Do not extend.
use super::*;




impl Game {
























}

/// COL-9: the literal (radius, mask) of each collector query (`Scene::collect`, FUN_00214368) per owner. A
/// non-literal radius (`r`, a parameter) is the owner's own value and has no entry here; every owner port reads
/// its row from this table.
pub mod attack {

}

