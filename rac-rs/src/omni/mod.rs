//! OmniLoader host: runs omni-format Luau script mods (`<player dir>/mods/<id>/omni-mod.json`) in this game through the
//! OmniLoader DLL (`omniloader.dll` next to the exe; MIT/Apache, a separate project, driven only through its C ABI v1).
//!
//! With no mod folder, with RAC_OMNI=0 or without the DLL nothing happens: no DLL is loaded, no hook is registered, no call is
//! made, so game state and timing are those of a build without this module.
//!
//! - `load.rs`: finds the mods, the DLL and the binding, starts the loader, writes the run-log lines (`omni: N mods loaded (...)`).
//! - `host.rs`: the `OmniHost` callbacks (log, memory, fields, bases) on the machine's RAM.
//! - `hooks.rs` + `bridge.rs`: hook points = relocatable hands at the retail functions found by their code (specs in `rac1_reloc.txt`):
//!   one generic bridge reads the arguments by the binding's signature, runs `omni_dispatch`, and `call_original` runs the retail code.
//! - `input.rs` + `input_host.rs`: keyboard, mouse, window and pointer for the mods (`omni.key`, `.pressed`, `.mouse`, `.window`, `.pointer`,
//!   `.take_input`, `.capture_mouse`): a per-field snapshot that rides in `FieldIn`, so a replay hands mods a neutral one.
//! - `events.rs`: ABI 1.2 events (`level_loaded`, `level_unloading`, `mode`, `pause_page`), read from RAM once per field, sent only on change.
//!   `original.rs`: `call_original` between fields (outside a hook dispatch). `omni_storage_dir` is set in `load.rs` before `omni_load`.
//! - `native_host.rs` + `funcs.rs` + `layout.rs`: native C/C++ mods (ABI 1.1): game RAM pointer, `call_game` (game functions found by code, `rac1_funcs.txt`),
//!   relocate, loaded_level, stack_push/pop; the disc string and data layout the mods are told.
//! - `data_map.rs` + `rac1_data.txt`: the per-level map of the overlay's global data (`relocate`); `counts.rs`: the native-call counters of the run log.
//! - `clash.rs`: the clash switch: a mod that does the job of one of our PC features (modern camera, strafe aim, lock-on, keyboard table) replaces it; `off(ID)` is a gate term.
//! - `draw.rs` + `draw_map.rs` + `draw_quads.rs` + `draw_mask.rs` + `draw_art.rs` + `draw_w2s.rs` (ABI 1.2): the mods' draw list (text in the game font, rects, bars, HUD icons) copied into a slot for the renderer's last 2D layer
//!   (`viewer/gpu/draw/pass_omni.rs`), the game's 512 x 448 space <-> window mapping, and `world_to_screen` from the game's camera.
//! - `pvars.rs` (host `pvars`: `omni.pvars(moby)`, the class variables named by the binding's `pvars` + `structs` sections) and `window.rs` (the overlay
//!   window: binding fields at `0x40000000 + reference address` are mapped to the level now loaded by `read_mem` / `write_mem`).
//! - `mobys.rs` (ABI 1.3 `moby_query` / `moby_valid`: `omni.mobys{..}`): the walk of the moby pool and the target registry, driven only by the binding's `mobys` section.
//! - `pose_follow.rs` (ABI 1.6 `pose_follow`: `omni.pose.follow`): picture-only "draw this moby where that one is relative to a hero joint, mirrored" records (host store, auto-clear, callback); they reach the renderer as `HeroParts.follow` and viewer/follow.rs places the item prop.
//! - `pose.rs` + `pose_host.rs` + `pose_read.rs` (ABI 1.4 `pose_set` / `pose_info` / `pose_read`: `omni.pose`): render-only pose overrides of the hero (store, auto-clear rules, callbacks, game-pose reads); the ops reach the renderer as `HeroParts.pose_ops` (retail/snapshot.rs).
//! - `lock.rs`: one re-entrant lock around every call into the DLL.
//! - `logq.rs`: the log queue: `log_cb` only appends a line, the thread `omnilog` writes the pending lines to stderr in one call (about 1 ms per line before);
//!   `flush_log` / `drain_log_raw` write what is pending on the way out (panic hook, exits, crash filter).
//! - `cabi.rs`: the C ABI structs and the DLL's functions. `binding.rs` + `json.rs`: the game binding (rac1.json, mod interface 1.3).
//!
//! Threads: `init` runs on the main thread (inside `Retail::load`, before the recompiled table is built); `tick`, `pad` and the hooks
//! on the retail worker thread between two fields; the hooks inside a field on the oracle game thread "ee" (bridge.rs `on_game_thread`:
//! it is the thread that catches the oracle's `RecRebuilt` unwind). Never concurrent: the worker waits in `Machine::frame` while "ee" runs;
//! `lock.rs` makes that true by construction (a hook may re-enter the DLL: `original` runs retail code that reaches another hook).

mod binding;
mod bridge;
mod cabi;
pub mod hostabi;
mod card;
pub mod clash;
mod codeguard;
mod counts;
mod data_map;
pub mod draw;
pub mod draw_art;
mod draw_demo;
pub mod draw_map;
pub mod draw_mask;
pub mod draw_quads;
mod draw_w2s;
mod events;
mod frame_wait;
mod return_stop;
mod funcs;
mod hooks;
mod host;
pub mod input;
mod input_host;
pub(crate) mod json;
mod layer;
mod layout;
pub(crate) mod live_settings;
mod load;
#[cfg(target_os = "linux")]
pub(crate) use load::loader_present;
mod lock;
mod logq;
mod mobys;
pub(crate) mod modset;
mod native_host;
mod newgame;
mod original;
mod prescan;
pub(crate) mod plugins;
mod pose;
pub(crate) mod pose_capture;
mod pose_follow;
mod pose_host;
mod pose_read;
mod pvars;
mod raycast;
mod trace;
mod window;
#[cfg(test)]
mod mobys_tests;
#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod tests;

pub use input::InputSnap;

/// The hero's render-only pose overrides for `HeroParts.pose_ops` (pose.rs `snapshot_for`); empty with no ops.
pub use pose::snapshot_for as pose_ops_for;

/// The follow poses for `HeroParts.follow` (pose_follow.rs `snapshot_for`); empty with none.
pub use pose_follow::snapshot_for as follow_for;
pub use pose_follow::FollowRec;

/// A recompiled-function table was (re)built: the cache of the built-in hands under the hook entries is void (layer.rs).
pub fn layer_rebuilt() {
	layer::rebuilt();
}

/// The layer cache's generation (tests: it moves with table builds only).
#[cfg(test)]
pub fn layer_epoch() -> u64 {
	layer::epoch()
}

/// Writes the mods' log lines still queued (logq.rs) now, on the calling thread: the panic hook and every normal way out of the process.
pub fn flush_log() {
	logq::flush();
}

/// The queued log lines to `write` from the Windows crash filter (runlog.rs); never waits for a lock.
pub fn drain_log_raw(write: fn(&str)) {
	logq::drain_raw(write);
}

/// `retail/worker/field_front.rs`, once per field: whether the title screen / front end is running (events.rs `mode`).
pub fn note_front(front: bool) {
	events::note_front(front);
}

use ee_oracle::cpu::Cpu;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// A started loader.
pub struct Omni {
	pub(crate) loader: cabi::Loader,
	pub(crate) state: &'static host::HostState,
	pub(crate) ticks: AtomicU64,
	/// A native (C/C++) mod folder was present: such mods hold the first machine's RAM pointer (native_host.rs).
	pub(crate) native: bool,
	/// Mods that loaded (status.json state "loaded"); events (ABI 1.2) are sent only when this is not 0.
	pub(crate) mods: usize,
	/// What the mods were told about the level, the mode and the pause page (events.rs).
	pub(crate) events: events::Shared,
}

/// Set once per process: a later `Retail::load` (Load Game, travel) keeps the loaded mods.
static OMNI: OnceLock<Option<Omni>> = OnceLock::new();

pub(crate) fn get() -> Option<&'static Omni> {
	OMNI.get().and_then(Option::as_ref)
}

/// `Retail::load`, after the machine exists and the PC hands are registered, before the recompiled table is built (hooks.rs registers hands).
/// The machine is in scope while the mods' main scripts run, so their load-time memory reads see its RAM.
pub fn init(c: &mut Cpu) {
	let o = OMNI.get_or_init(|| load::start(crate::retail::disc::usa(), c));
	// A later load made another machine: native mods still point into the first one's RAM (native_host.rs).
	native_host::machine_check(c, o.as_ref().is_some_and(|o| o.native));
}

/// After each field the EE ran (the game's logic tick: 50 a second PAL, 60 NTSC), on the worker thread.
pub fn tick(c: &mut Cpu) {
	let Some(o) = get() else { return };
	if native_host::stale() {
		return;
	}
	let n = o.ticks.fetch_add(1, Ordering::Relaxed) + 1;
	live_settings::apply(o, c);
	{
		let s = events::snapshot(&c.mem.ram, false);
		pose::tick(&c.mem.ram, events::level_of(&s), s.exit, s.mode);
		pose_follow::tick(&c.mem.ram, o.state, events::level_of(&s), s.exit, s.mode);
	}
	pose_host::debug_tick(c, o.state);
	trace::game_pad(n, c, o.state.ntsc);
	events::send(o, c, n);
	{
		let _held = lock::enter();
		let sp = native_host::sp_before(c);
		host::scope(c, || o.loader.tick());
		native_host::sp_check(c, sp, "on_frame");
	}
	draw_demo::tick(c);
	if n == 1 || n % 1000 == 0 {
		// Queued, not written here: this runs on the retail worker thread, between two fields (logq.rs).
		logq::line(&format!("omni: tick {n}"));
		if let Some(l) = hooks::counts_line() {
			logq::line(&format!("omni: hook calls at tick {n}: {l}"));
		}
		if let Some(l) = counts::line() {
			logq::line(&format!("omni: native calls at tick {n}: {l}"));
		}
	}
}

/// Before the EE runs a field, after the PC features (modern camera, aim, strafe, replay, ...) had their say: the 18-byte PS2 pad
/// report the game's pad read (`PAD_READ`, once per field, right after this) hands out goes through the mods' `on_pad` callbacks.
/// It is written back only when a mod changed a byte, so a mod that does nothing leaves the machine as it was.
pub fn pad(c: &mut Cpu) {
	let Some(o) = get() else { return };
	let Some(orig) = c.pad else { return };
	if native_host::stale() {
		return;
	}
	let mut report = orig;
	{
		let _held = lock::enter();
		let sp = native_host::sp_before(c);
		host::scope(c, || o.loader.pad(0, &mut report));
		native_host::sp_check(c, sp, "on_pad");
	}
	trace::report(o.ticks.load(Ordering::Relaxed) + 1, &orig, &report);
	if report != orig {
		c.pad = Some(report);
	}
}
