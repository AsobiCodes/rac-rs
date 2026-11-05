//! NOV-9.1/9.2 break fragments (decomp/notes/novalis_environment.md "NOV-9.1/9.2 port spec"): a broken tall shroom
//! (1813) throws one class-1815 piece, a broken mushroom pot (754) four class-1817 pieces (creator FUN_00278670);
//! they tumble, bounce off the world and explode when their life runs out (updater FUN_0030c648). Both parents leave
//! the static class-1816 remains (creator FUN_002789b8, updater 0x30cb30 `jr ra`).


pub(super) const SHROOM_FRAGMENT: u32 = 1815;
pub(super) const REMAINS: u32 = 1816;
pub(super) const POT_FRAGMENT: u32 = 1817;
/// The classes the viewer hands its bounding spheres for (`Game::class_spheres`): the parts below in order. Registry: one
/// entry per line; add new classes in the owning level module.
pub const FRAGMENT_CLASSES: &[u32] = &super::levels::cat::<{ super::levels::total_len(FRAGMENT_PARTS) }>(FRAGMENT_PARTS);
pub(crate) const FRAGMENT_PARTS: &[&[u32]] = &[NOVALIS_FRAGMENTS, super::aridia::FRAGMENT_CLASSES, super::kerwan::FRAGMENT_CLASSES];
/// Novalis's part (spawn order): the shroom / pot pieces and NOV-22.6 the Birdbot pieces.
const NOVALIS_FRAGMENTS: &[u32] = &[
	SHROOM_FRAGMENT,
	REMAINS,
	POT_FRAGMENT,
	BIRD_FRAGMENTS[0],
	BIRD_FRAGMENTS[1],
	BIRD_FRAGMENTS[2],
	BIRD_FRAGMENTS[3],
];
/// NOV-22.6: the four pieces a Birdbot throws when it crashes (state 14 or the water), in spawn order.
pub const BIRD_FRAGMENTS: [u32; 4] = [1736, 1737, 1738, 1770];
/// ARI-12b: the Aridia shark nest's death pieces (two of each, FUN_00265148 x4 in nest state 4).
pub const NEST_FRAGMENTS: [u32; 2] = [1764, 1765];











