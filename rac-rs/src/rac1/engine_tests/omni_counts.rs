//! engine/omni/counts.rs tests (dev s138): the line names the function specs, which come from R&C1's mod-host part.

use crate::omni::counts::*;
use std::sync::atomic::Ordering;

#[test]
fn counts_per_key_and_line_lists_them() {
	crate::omni_bind::for_tests();
	let before = (get(Counter::RaycastHit), get(Counter::RaycastMiss));
	bump(Counter::RaycastHit);
	bump(Counter::RaycastHit);
	bump(Counter::RaycastMiss);
	assert_eq!((get(Counter::RaycastHit) - before.0, get(Counter::RaycastMiss) - before.1), (2, 1));
	let l = line().unwrap();
	assert!(l.contains("raycast hit=") && l.contains("raycast miss="), "{l}");
	// Every counter has its name.
	assert_eq!(NAMES[Counter::RaycastRefused as usize], "raycast refused");
	assert_eq!(NAMES[Counter::CallGameRefused as usize], "call_game refused");
	assert_eq!((NAMES[Counter::Relocate as usize], NAMES[Counter::RelocateUnmapped as usize]), ("relocate", "relocate (unmapped)"));
}

#[test]
fn a_function_call_is_counted_in_its_spec_and_named_in_the_line() {
	crate::omni_bind::for_tests();
	let spec = &crate::omni::funcs::all()[0];
	spec.calls.fetch_add(1, Ordering::Relaxed);
	let l = line().unwrap();
	assert!(l.contains(&format!("call_game {}=", spec.name)), "{l}");
}
