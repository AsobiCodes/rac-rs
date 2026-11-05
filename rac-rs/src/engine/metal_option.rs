//! Shared, live player setting for the optional metal look.
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
static ENABLED: AtomicBool = AtomicBool::new(false);
pub fn set(on: bool) { ENABLED.store(on, Ordering::Relaxed); }
pub fn enabled() -> bool { ENABLED.load(Ordering::Relaxed) }

/// Metal reflections strength in percent (0..=100, steps of 5; 100 = the full look, 75 the default). The metal look's reflection gain
/// (`metalenv.rs` `knobs`) is scaled by it; highlights from lights stay as they are.
pub const REFLECT_DEFAULT: u8 = 75;
static REFLECT: AtomicU8 = AtomicU8::new(REFLECT_DEFAULT);
pub fn set_reflect(pct: u8) { REFLECT.store(pct.min(100), Ordering::Relaxed); }
pub fn reflect_pct() -> u8 { REFLECT.load(Ordering::Relaxed) }
