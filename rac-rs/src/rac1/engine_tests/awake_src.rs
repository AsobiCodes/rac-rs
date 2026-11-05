//! engine/awake.rs: only real pad input wakes the display (dev s138 test; it reads the R&C1 host and retail sources).

/// Only real pad input resets the timers: the one call site sits behind `take_pad_pulse`, which only `poll_pad` (the XInput and
/// PlayStation-controller readers; nothing under RAC_TEST_WINDOW=hidden / RAC_RETAIL_SMOKE) sets. The scripted pad (RAC_RETAIL_PAD,
/// racpad replays: retail/tick.rs) and the scripted cursor device (RAC_CURSOR_DEV) never reach it.
#[test]
fn only_real_pad_input_calls_pad_input() {
	let vis = include_str!("../host/handler/cursor_vis.rs");
	let call = "crate::awake::pad_input()";
	assert_eq!(vis.matches(call).count(), 1);
	let at = vis.find(call).unwrap();
	let pulse = vis.find("if crate::retail::prompts::take_pad_pulse() {").unwrap();
	let scripted = vis.find("if let Some(w) = scripted(frame)").unwrap();
	assert!(pulse < at && scripted < pulse && vis[pulse..at].matches('}').count() == 0, "inside the take_pad_pulse branch");
	for (name, src) in [("tick.rs", include_str!("../retail/tick.rs")), ("input.rs", include_str!("../retail/input.rs")), ("rec.rs", include_str!("../retail/rec.rs"))] {
		assert!(!src.contains("pad_input"), "{name} must not call awake::pad_input");
	}
}
