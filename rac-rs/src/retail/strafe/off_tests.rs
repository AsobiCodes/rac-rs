//! `feel_off` (strafe.rs): a strafe flip in the air when the strafe goes off is forgotten.

use super::*;

#[test]
fn a_flip_in_the_air_is_forgotten_when_the_strafe_goes_off() {
	let mut mem = ee_oracle::mem::Mem::new();
	let mut guard = ST.lock().unwrap_or_else(|e| e.into_inner());
	super::super::strafe_feel::flip_start(&mut mem, &mut guard.feel);
	assert!(guard.feel.flipping(), "the flip is on");
	drop(guard);
	feel_off();
	assert!(!ST.lock().unwrap_or_else(|e| e.into_inner()).feel.flipping(), "the flip is gone: no old flip speed on a later run");
}
