//! KER-80a: the class 822 ride (decomp/notes/kerwan_environment.md "KER-80a port spec"): the shared head and states
//! 0-3 of its updater FUN_002927c8. States 4-6 are KER-80b; the placement helper, the per-tick follower and the springs
//! are KER-81a / KER-81c / KER-81b.


/// KER-80a: the ride class (moby 4 drives the class 845 flyer 925; moby 5 is a dormant twin).
pub(in crate::game) const RIDE822: u32 = 822;
/// KER-81a: the ride's follower class (mobies 14-18 chained through pvar +0xc0).
pub(in crate::game) const FOLLOWER: u32 = 1210;








