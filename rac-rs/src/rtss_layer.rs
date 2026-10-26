//! RivaTuner Statistics Server (RTSS) Vulkan layer: off for this game unless asked (GitHub #12 / the v0.3.1 start-up freeze).
//!
//! RTSS registers an implicit Vulkan layer (`VK_LAYER_RTSS`, `RTSSVkLayer64.dll`) that the Vulkan loader puts into every Vulkan
//! program, also when RTSS is closed. With NVIDIA driver 621.01 that layer deadlocked the start: a white "Not Responding" window. The
//! cause was the game's own two Vulkan instances made at once on two threads (the window's on the main thread, the GS capture's own on
//! `gs-gpu-open`), both waiting inside the layer; dev s97 (the capture shares the window's device) removed the second one
//! (scratch_perf_rtss2/HANDOFF.md; corrected 2026-10-23: was "not something the game's own Vulkan calls can avoid"). The layer stays off
//! anyway, as a second guard. Its manifest has an off switch, the environment variable `DISABLE_RTSS_LAYER=1`; the game sets it in its
//! own process before the first Vulkan call (every loader version honours it, `VK_LOADER_LAYERS_DISABLE` needs loader 1.3.234+).
//! RTSS's on-screen counter still shows on Vulkan while RTSS runs: its injected `RTSSHooks64.dll` draws it without the layer (user
//! check 2026-10-23 on dev s100; corrected: was "the counter does not show in this game"). `RAC_RTSS_LAYER=1` keeps the layer. A
//! player's own `DISABLE_RTSS_LAYER` is left as it is.
use std::sync::atomic::{AtomicU8, Ordering};

/// What `apply` did: 0 not run yet, 1 switched the layer off, 2 the layer is kept (`RAC_RTSS_LAYER=1`), 3 already off (the variable was set).
static STATE: AtomicU8 = AtomicU8::new(0);

/// The decision for the two variables' values (None = not set).
fn decide(disable: Option<&str>, keep: Option<&str>) -> u8 {
	if keep == Some("1") {
		2
	} else if disable.is_some() {
		3
	} else {
		1
	}
}

/// Runs first thing in `main`, before any thread starts and before the first Vulkan call (`set_var` is only sound then).
pub fn apply() {
	let state = decide(std::env::var("DISABLE_RTSS_LAYER").ok().as_deref(), std::env::var("RAC_RTSS_LAYER").ok().as_deref());
	if state == 1 {
		// SAFETY: single-threaded, first thing in `main` (after the load-log clock, which starts no thread).
		unsafe { std::env::set_var("DISABLE_RTSS_LAYER", "1") };
	}
	STATE.store(state, Ordering::Relaxed);
}

/// The run-log line (the log starts after `apply`, so `Gpu::new` prints it next to the other `gpu:` lines).
pub fn note() {
	match STATE.load(Ordering::Relaxed) {
		1 => eprintln!("gpu: RivaTuner's Vulkan layer is off for this game (RAC_RTSS_LAYER=1 keeps it)"),
		2 => eprintln!("gpu: RivaTuner's Vulkan layer is kept (RAC_RTSS_LAYER=1)"),
		3 => eprintln!("gpu: RivaTuner's Vulkan layer is off (DISABLE_RTSS_LAYER is set)"),
		_ => {}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_layer_goes_off_unless_kept_or_already_off() {
		assert_eq!(decide(None, None), 1, "default: switched off");
		assert_eq!(decide(None, Some("1")), 2, "RAC_RTSS_LAYER=1 keeps the layer");
		assert_eq!(decide(Some("1"), Some("1")), 2, "the opt-out wins over a set variable");
		assert_eq!(decide(Some("1"), None), 3, "already off: nothing to do");
		assert_eq!(decide(None, Some("0")), 1, "only 1 keeps it");
	}
}
