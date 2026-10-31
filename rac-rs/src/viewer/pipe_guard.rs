//! [PC] Build-time safety net for the optional effects (fog, clouds, water, vegetation, shadows ...). A graphics driver or shader
//! compiler can refuse one shader or pipeline: DX12 without dxcompiler.dll compiles with the old FXC, which aborts on some valid
//! WGSL (the fog's `textureDimensions` in a loop, `viewer/volfog/floor.rs`). wgpu's default reaction to such an error is a panic,
//! so one effect took the whole game down.
//!
//! `guard` / `build` run an effect's build code inside wgpu error scopes (Validation + Internal + OutOfMemory). On an error they log
//! ONE line, set the effect's bit in `OFF` and (`guard`) return None; the effect's draw gate asks `off(effect)` (one atomic load,
//! only on the paths that were already gated) and skips the effect, so the game keeps running without it. An effect that is off is
//! never built again, so the refusal costs nothing per frame. What is guarded: the shader and pipeline refusals of the build code.
//! A resource that a later, unguarded step uses (a texture view, a bind group) is NOT covered by this: its out-of-memory would
//! still panic there. The one such resource that is small to cover, the sun shadow map, has its own fallback (`refuse`,
//! `sunshadow.rs`). A refusal is permanent for the run (a transient out-of-memory in the GPU tie rewrite means the CPU tie path
//! until the next start) and silent on any backend: `RAC_PIPE_STRICT=1` makes it panic with the same text, so tests and gate
//! runs on Vulkan catch coding bugs in an effect's build code. Only build time is guarded: no per-frame cost while all is well,
//! and the picture of a healthy driver does not change. Never used for the core level pipelines (`viewer/gpu/setup.rs`): they are the
//! picture itself and are swept for FXC offline (tools `fxc_check.py` in the lane notes, RAC_WGSL_DUMP in ENV.md).
//! Test a refusal by hand: RAC_WGSL_DIR=DIR with a broken `<label>.wgsl` in DIR (devtune.rs) makes that effect's shader fail.
//!
//! Queue writes inside a scope (v0.4.3 review): the viewer's `GatedQueue` lists its writes for the present thread (qdefer.rs), which
//! applies them later, outside every scope: an error such a write causes (an invalid texture after an out-of-memory) would reach wgpu's
//! default handler and panic. While a scope of this module is open on the thread (`InScope`, `in_scope`), `GatedQueue` therefore makes
//! its writes at once on the wgpu queue (after the gate wait and the pending list, in order), inside the scope. This holds for every
//! guarded build and for the hand-made scope of `HeroGpu::new` (gpuhero.rs) without a change at the call sites. The builds are lazy and
//! run once, so no wait is added per frame; with `RAC_WRITE_DEFER=0` the writes were immediate already.

use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};

/// The optional effects that have a build guard. The value is the bit in `OFF`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
	VolFog,
	VolClouds,
	Water,
	Underwater,
	Veg,
	SunShadow,
	ShadowVol,
	Post,
	Star,
	Starfield,
	HudSharp,
	RenderScale,
	Reshade,
	TieGpu,
	WaterBedMin,
}

impl Effect {
	const fn bit(self) -> u32 {
		1 << self as u32
	}

	/// The name in the run-log line.
	pub(crate) const fn name(self) -> &'static str {
		match self {
			Effect::VolFog => "volumetric fog",
			Effect::VolClouds => "volumetric clouds",
			Effect::Water => "water fx",
			Effect::Underwater => "underwater",
			Effect::Veg => "vegetation",
			Effect::SunShadow => "sun shadows",
			Effect::ShadowVol => "shadow volumes",
			Effect::Post => "post (lighting enhanced)",
			Effect::Star => "animated star",
			Effect::Starfield => "starfield",
			Effect::HudSharp => "sharp HUD",
			Effect::RenderScale => "render scale",
			Effect::Reshade => "reshade passes",
			Effect::TieGpu => "GPU tie rewrite (CPU path stays)",
			Effect::WaterBedMin => "water bed minimum (the full shore fade stays)",
		}
	}
}

static OFF: AtomicU32 = AtomicU32::new(0);

thread_local! {
	/// How many build scopes this thread has open (`InScope`); error scopes are per thread, so is this.
	static SCOPE_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Whether this thread is inside a build error scope: `GatedQueue` writes are then made at once instead of listed (see the module doc).
pub(crate) fn in_scope() -> bool {
	SCOPE_DEPTH.with(|d| d.get() > 0)
}

/// Marks the thread as inside a build error scope while it lives (also when the build panics). Take it BEFORE the scopes are pushed.
pub(crate) struct InScope(());

impl InScope {
	pub(crate) fn enter() -> InScope {
		SCOPE_DEPTH.with(|d| d.set(d.get() + 1));
		InScope(())
	}
}

impl Drop for InScope {
	fn drop(&mut self) {
		SCOPE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
	}
}

/// Whether the effect was refused by the driver and is off for this run.
pub(crate) fn off(e: Effect) -> bool {
	OFF.load(Ordering::Relaxed) & e.bit() != 0
}

/// What to do after a build: the first error text (if any) of the scopes decides.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
	Keep,
	/// Switch the effect off and log this text.
	SwitchOff(String),
}

/// The decision, kept free of wgpu so it can be tested: `errors` are what the scopes returned, in pop order.
pub(crate) fn decide(errors: &[Option<String>]) -> Verdict {
	match errors.iter().flatten().next() {
		Some(e) => Verdict::SwitchOff(e.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect()),
		None => Verdict::Keep,
	}
}

/// `RAC_PIPE_STRICT=1`: a refusal panics (same text) instead of switching the effect off.
fn strict() -> bool {
	static STRICT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*STRICT.get_or_init(|| std::env::var("RAC_PIPE_STRICT").is_ok_and(|v| v == "1"))
}

/// The driver refused part of the effect: log ONE line (first time) and switch it off for the run; with `RAC_PIPE_STRICT=1` panic.
pub(crate) fn refuse(e: Effect, why: &str) {
	let line = format!("gpu: {}: shader/pipeline refused by the driver ({why}) -> effect off", e.name());
	if strict() {
		panic!("{line}");
	}
	if mark(&OFF, e) {
		eprintln!("{line}");
	}
}

/// Sets the effect's bit in `bits`; true the first time (the caller logs once).
fn mark(bits: &AtomicU32, e: Effect) -> bool {
	bits.fetch_or(e.bit(), Ordering::Relaxed) & e.bit() == 0
}

/// Runs `f` (the effect's shader and pipeline build) inside error scopes: the value and whether the build was accepted. The scopes
/// are per thread and popped last-in-first-out, as wgpu requires. A refusal sets the effect off and logs one line.
fn run<T>(device: &wgpu::Device, e: Effect, f: impl FnOnce() -> T) -> (T, bool) {
	let _in_scope = InScope::enter();
	let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
	let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
	let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
	let value = f();
	let errors = [pollster::block_on(validation.pop()), pollster::block_on(internal.pop()), pollster::block_on(oom.pop())].map(|r| r.map(|x| x.to_string()));
	match decide(&errors) {
		Verdict::Keep => (value, true),
		Verdict::SwitchOff(why) => {
			refuse(e, &why);
			(value, false)
		}
	}
}

/// A lazy build: Some(value) = built; None = refused now or earlier (`f` is not run again once the effect is off, so a refusal is
/// never retried). The value of a refused build is dropped.
pub(crate) fn guard<T>(device: &wgpu::Device, e: Effect, f: impl FnOnce() -> T) -> Option<T> {
	if off(e) {
		return None;
	}
	let (v, ok) = run(device, e, f);
	ok.then_some(v)
}

/// A build at start-up whose struct has to exist either way: the value always, with the effect marked off when the driver refused
/// part of it; the draw gate then asks `off(e)` and never touches the refused pipelines.
pub(crate) fn build<T>(device: &wgpu::Device, e: Effect, f: impl FnOnce() -> T) -> T {
	run(device, e, f).0
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn no_error_keeps_the_effect() {
		assert_eq!(decide(&[None, None, None]), Verdict::Keep);
	}

	#[test]
	fn first_error_switches_off_with_one_short_line() {
		let text = "Validation Error

Caused by:
  In Device::create_render_pipeline, label = 'fs_apply'
    FXC D3DCompile error: internal error".to_string();
		assert_eq!(decide(&[None, Some(text), Some("later".into())]), Verdict::SwitchOff("Validation Error Caused by: In Device::create_render_pipeline, label = 'fs_apply' FXC D3DCompile error: internal error".into()));
		match decide(&[Some("y".repeat(900))]) {
			Verdict::SwitchOff(t) => assert_eq!(t.len(), 300),
			v => panic!("{v:?}"),
		}
	}

	#[test]
	fn a_refused_fog_or_cloud_shader_gives_level_zero() {
		use crate::retail::pcext::Graphics;
		let g = Graphics { lighting: 1, fog_q: 3, clouds_q: 2, ..Graphics::CLASSIC };
		assert_eq!(crate::viewer::volfog::frame::level_if(&g, false), 3);
		assert_eq!(crate::viewer::volfog::frame::level_if(&g, true), 0, "the retail fog must not be faded when the fog shader was refused");
	}

	/// `GatedQueue` asks `in_scope` to make its writes at once inside a build scope: nesting counts, other threads are not affected,
	/// and a build that panics leaves the scope.
	#[test]
	fn a_build_scope_is_seen_by_queue_writes_on_its_thread_only() {
		assert!(!in_scope());
		{
			let _outer = InScope::enter();
			assert!(in_scope());
			{
				let _inner = InScope::enter();
				assert!(in_scope());
			}
			assert!(in_scope(), "the inner scope ending must not close the outer one");
			std::thread::scope(|s| {
				s.spawn(|| assert!(!in_scope(), "scopes are per thread")).join().unwrap();
			});
		}
		assert!(!in_scope());
		let hook = std::panic::take_hook();
		std::panic::set_hook(Box::new(|_| {}));
		let r = std::panic::catch_unwind(|| {
			let _s = InScope::enter();
			panic!("the build failed");
		});
		std::panic::set_hook(hook);
		assert!(r.is_err() && !in_scope(), "a panicking build must leave the scope");
	}

	#[test]
	fn bits_are_distinct_and_marking_is_once() {
		let all = [Effect::VolFog, Effect::VolClouds, Effect::Water, Effect::Underwater, Effect::Veg, Effect::SunShadow, Effect::ShadowVol, Effect::Post, Effect::Star, Effect::Starfield, Effect::HudSharp, Effect::RenderScale, Effect::Reshade, Effect::TieGpu, Effect::WaterBedMin];
		let mut seen = 0u32;
		for e in all {
			assert_eq!(seen & e.bit(), 0, "{e:?} shares a bit");
			seen |= e.bit();
			assert!(!e.name().is_empty());
		}
		let bits = AtomicU32::new(0);
		assert!(mark(&bits, Effect::Water), "first refusal is logged");
		assert!(!mark(&bits, Effect::Water), "never logged twice");
		assert!(mark(&bits, Effect::Veg), "another effect is independent");
		assert_eq!(bits.load(Ordering::Relaxed), Effect::Water.bit() | Effect::Veg.bit());
	}
}
