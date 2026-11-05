//! Tests of engine/omni/input/tests.rs (engine split stage E2, 2026-10-26): they run with R&C1's part of the mod host (rac1/omni_bind.rs: binding, spec
//! files, RAM words), so they live in the R&C1 pack; moved from the engine file with paths adjusted, each test hands that part over first.
#![allow(unused_imports)]
use crate::omni::input::*;
use winit::keyboard::KeyCode as K;

#[test]
fn neutral_is_the_default_and_the_replay_input() {
	crate::omni_bind::for_tests();
	assert_eq!(InputSnap::default(), InputSnap::NEUTRAL);
	let n = crate::retail::FieldIn::default().omni;
	assert!(!n.key("KeyW") && !n.pressed("MouseLeft") && n.window()[0] == 0 && !n.inside);
}


#[test]
fn take_input_masks_the_pad_keys() {
	crate::omni_bind::for_tests();
	use crate::retail::pad_from_keys;
	let neutral = pad_from_keys(|_| false, false, false);
	let w = |k: K| k == K::KeyW || k == K::Space;
	assert_ne!(pad_from_keys(gated(false, w), false, false), neutral, "WASD drives the pad when no mod took the keyboard");
	assert_eq!(pad_from_keys(gated(true, w), false, false), neutral, "a mod holds the keyboard: the same keys press nothing");
	// The mouse buttons are not the keyboard: Circle (left button) still reaches the pad.
	assert_ne!(pad_from_keys(gated(true, w), true, false), neutral);
}
