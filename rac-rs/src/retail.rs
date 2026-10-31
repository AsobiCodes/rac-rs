//! Retail mode (`view ISO LEVEL --retail DIR`, or `--retail-cold`): the game's own EE code runs the level in the ee-oracle
//! interpreter, resumed from a PCSX2 savestate folder (oracle/p2s.py output) or cold-booted from the disc. rac-rs feeds it the pad once per
//! 50 Hz tick and draws its mobys and camera; none of the rac-rs game logic runs.
//!
//! This file is the front door: `FieldIn`, `Cam`, `OvLate`, `MouseTaps`, the `Retail` struct and the module list.
//! Glue children (each starts with `use super::*;` and is glob re-exported here, so every old `retail::x` path still works):
//! - `session`: `enable` / `enable_cold` / `--gsfull`: turning retail mode on, asking whether it is on
//! - `ram_globals`: boot-RAM addresses (level byte, held moby, moby table) and the game-mode predicates
//! - `camera`: the render camera in RAM, the occlusion mask, `Retail::camera` / `view_lag`
//! - `prop_pools`: the prop pools for `Game::with_story`: counting, widening, deferred writes, growth
//! - `moby_state`: `Moby`, side-test / cull inputs, animation clock, hero parts and their RAM readers
//! - `snapshot`: `Snap` (one field for the main thread) and `snap` (reads it from RAM), scene sequences, native delay
//! - `capture`: the GS capture thread (`Capture`, `capture_gs_vram`) and the overlay pictures it hands over
//! - `front_end`: the front end (frontbin) as the mirror sees it: `FrontRam`, `front_snap`, the late GS-full pipe
//! - `fx_queue`: the effects layer read from RAM each field (`RetailFx`, `queue_fx`): pool, orbs, arcs, glows, ribbons
//! - `lights_water`: dynamic point lights, reserved level lights, the water patch table
//! - `shadow_snap`: shadow volumes and the ship ground shadow read from RAM, `SmoothDraw`
//! - `shop_snap`: the vendor shop screen's VU1 mobys and hologram beam (`ShopSnap`)
//! - `worker`: `Retail::load`: the interpreter worker thread; `Drop for Retail`
//! - `tick`: the main thread's field loop: `tick`, `field_ready`, the field's sound, the lookahead depth
//! - `input`: keyboard / mouse / pad into the game, scripted test input, the movie skip rule
//! - `present`: what the viewer shows: movie hand-off, overlay, GS-full picture, front-end state flags
//! - `accessors`: the viewer's read access to the latest snapshot (hero parts, shadows, orbs, fx, smoothing)
//! - `props_sync`: binding the game's props to retail's mobys: level switch, rebind order, `sync`
//! - `debug_aids`: dev / test aids: `Smoke`, `WPerf`, the class probe, shot allowance, slow-machine imitations
//! - `test_suite`: the tests of retail mode (savestate probes are `#[ignore]`)
//!
//! Self-contained children (their own items, reached as `retail::name::x`):
//! - effects layer: `efx_own` (ownership of the draw-queue callbacks' packets), `efx_draw` (PATH2 direct-GIF callbacks drawn from their GS data),
//!   `efx_vu` (callbacks that go through a VU1 microprogram), `snap_pipe` (the effects layer's picture work on its own thread), `asset_feed` (class texture
//!   scroll), `queue_b` (draw queue B and its world-quad callbacks), `queue_q1` / `queue_q2` / `queue_fxa` / `queue_fxb` (more queue callbacks
//!   the mirror did not draw: early planets, later planets, queue-B ones, ribbon trails), `queue_story` (story-moment callbacks)
//! - `oldsim`: effects drawn from the old simulation's state, read from retail RAM instead
//! - `warp`: the arrival warp (game mode 6, substate 4) read from RAM
//! - `gscap`: rac-rs copy of the oracle's GS capture; big prims drawn on several threads, pixel-exact with the oracle
//! - `snd`: retail sound: the exact 989snd driver plays the game's sounds, rac-rs mixes the SPU2 output
//! - `fmv_bridge`: native movies for retail mode (the oracle has no IPU / MPEG emulation)
//! - `memcard`: the persistent player memory card
//! - `rec`: input recorder and replayer (`RAC_RETAIL_REC`, `RAC_RETAIL_REPLAY`)
//! - `playfeed`: feeds a recorded PCSX2 playthrough window into an oracle run (source: oracle/pcsx2rec/playfeed.rs, included by path)
//! - `title_boot`: boot to the game's own title screen, and the front end (frontbin)
//! - `disc`, `disc_hands`, `disc_map`: which retail disc runs (PAL or USA) and what differs; the hand and address maps are generated
//! - `hot`: [PC] the busy threads at above-normal priority, out of Windows power throttling
//! - PC options and controls: `pcext` (options in the pause menu), `pcopts` (the options file), `pcctl` (Controls page), `binds` (button
//!   remap), `prompts` (button prompts, gamepad), `pspad` (DualShock / DualSense over HID; `lxpad` on Linux: gamepads over evdev), `mouse_menu` and `menu_keys` (mouse and keys
//!   in the menus), `vendor` ("buy all ammo")
//! - PC extras: `modcam` (modern camera, RMB aim), `aim` (crosshair and aimed shots), `strafe` ("Controls: Modern" movement),
//!   `shipmouse` (mouse steering in ship fights)
//! - dev aids: `debugkeys` (playtest keys), `devtools` (developer tools); both inert unless switched on

use crate::game::Game;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use crate::retail::disc::va;

mod gscap;
pub mod hot;
mod rec;
#[path = "../../oracle/pcsx2rec/playfeed.rs"]
mod playfeed;
pub mod asset_feed;
pub mod queue_b;
pub mod efx_own;
pub mod efx_draw;
pub mod efx_vu;
pub mod free_water;
pub mod snap_pipe;
pub mod queue_q1;
pub mod queue_q2;
pub mod queue_fxa;
pub mod queue_fxb;
pub mod queue_story;
pub mod snd;
pub mod fmv_bridge;
pub mod memcard;
pub mod modcam;
pub mod modcam_turn;
pub mod aim;
pub mod pcext;
pub mod console_clock;
pub mod binds;
pub mod pcctl;
pub mod pcmods;
pub mod prompts;
pub mod prompts_box;
pub mod wheel_mouse;
pub mod pspad;
#[cfg(any(test, target_os = "linux"))]
pub mod lxpad;
/// The gamepad reader of this platform for PlayStation / evdev controllers: pspad (HID) on Windows, lxpad (evdev through gilrs) on
/// Linux. Both give `state()` and `connected()`, an XInput report each.
#[cfg(target_os = "linux")]
pub(crate) use lxpad as hidpad;
#[cfg(not(target_os = "linux"))]
pub(crate) use pspad as hidpad;
pub mod vendor;
pub mod pcopts;
pub mod pcdisp;
pub mod glyphs;
pub mod pcinput;
pub mod pcintro;
pub mod pcrender;
pub mod pcskip;
pub mod strafe;
pub mod strafe_feel;
pub mod shipmouse;
pub mod debugkeys;
pub mod devtools;
pub mod splitview;
pub mod oldsim;
pub mod mouse_menu;
pub mod mouse_instant;
pub mod mouse_slider;
mod mouse_answers;
pub mod mouse_vendor;
mod mouse_script;
pub mod menu_keys;
pub mod title_boot;
pub mod titlequit;
pub mod disc;
pub mod disc_hands;
pub mod disc_map;
pub mod disc_trace;
#[cfg(test)]
mod disc_proven;
#[cfg(test)]
pub(crate) mod disc_guard_tests;

mod session;
mod ram_globals;
pub(crate) mod camera;
mod camera_arc;
mod prop_pools;
mod moby_state;
mod snapshot;
mod capture;
mod front_end;
mod fx_queue;
mod lights_water;
mod shadow_snap;
mod shop_snap;
mod wide_side;
mod worker;
pub(crate) mod tick;
mod input;
mod present;
mod accessors;
mod props_sync;
mod debug_aids;
#[cfg(test)]
mod test_suite;
#[cfg(test)]
mod mouse_lists_test;
#[cfg(test)]
mod mouse_latency_test;
pub use self::session::*;
pub use self::ram_globals::*;
pub use self::camera::*;
pub use self::camera_arc::{boom_turn, orbits, MAX_TURN as ARC_MAX_TURN};
pub use self::prop_pools::*;
pub use self::moby_state::*;
pub use self::snapshot::*;
pub use self::capture::*;
pub use self::front_end::*;
pub use self::fx_queue::*;
pub use self::lights_water::*;
pub use self::shadow_snap::*;
pub use self::shop_snap::*;
pub use self::tick::*;
pub use self::input::*;
use self::present::*;
use self::props_sync::*;
pub use self::debug_aids::*;

/// One field's input as the interpreter thread consumes it (what the recorder writes and the replayer reads).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FieldIn {
	pub buttons: u16,
	/// [rx, ry, lx, ly], the order `Machine::set_pad` takes.
	pub sticks: [u8; 4],
	/// Level-travel request poked before the field (RAC_RETAIL_TRAVEL, or a replayed `!TRAVEL`).
	pub travel: Option<u32>,
	/// Modern camera input (retail/modcam.rs); zero when the switch is off.
	pub mc: modcam::Input,
	/// Keyboard / mouse / window / pointer for the OmniLoader mods (omni/input.rs); neutral in a replay and with no mods.
	pub omni: crate::omni::InputSnap,
}

/// A camera pose: position, yaw, pitch (rac-rs convention, up positive) and the retail tan half-FOV scalar.
pub type Cam = ([f32; 3], f32, f32, f32);

pub mod warp;

/// The deferred `Capture::take` of one field: waits for the capture thread to finish the field's jobs, then gives the overlay as
/// (width, rows, RGBA8, `disp`, `HudGuard`). The overlays are tagged by the capture thread's field-end count, so taking later (on the main
/// thread, after the worker already runs the next field) gives the same overlay as taking at the field's end.
pub struct OvLate(Box<dyn FnOnce() -> Option<(u32, u32, Vec<[u8; 4]>, bool, HudGuard)> + Send>);

/// The click latch of the left (0, Circle) and middle (1, Triangle) mouse buttons: fields each still counts as held.
#[derive(Default)]
struct MouseTaps([u8; 2]);

/// The interpreter on its own thread: each tick hands it the pad and takes the field it finished during the last
/// frame, so the game logic runs while the main thread renders (one field of input latency).
pub struct Retail {
	tx: std::sync::mpsc::Sender<FieldIn>,
	/// The interpreter thread; joined on drop while recording so a later load's `!LOAD` follows its last field.
	worker: Option<std::thread::JoinHandle<()>>,
	/// RAC_RETAIL_REPLAY: the pad file that replaces the keyboard.
	replay: Option<Arc<rec::Replay>>,
	replay_ended: bool,
	rx: std::sync::mpsc::Receiver<Snap>,
	/// Fields handed to the thread whose snapshot is not taken yet (1 = lockstep, more while `ahead` lets it run ahead).
	inflight: u32,
	/// Snapshots taken (`Snap::field`).
	taken: u32,
	/// [PC] How many fields the thread may hold (`depth`): 1, or up to 3 in gameplay and scenes while the window's frames
	/// take longer than a field, so the thread keeps working while a slow frame draws instead of waiting for the next tick.
	/// The pad of each field is still the one of its own tick (`fields` indexes it), only one or two fields later.
	ahead: u32,
	/// [PC] The window's frame interval in ms, averaged (`set_frame_ms`).
	frame_avg: f32,
	/// [PC] Milliseconds the last tick waited for the thread's field (the viewer's per-frame wait budget).
	pub waited: f32,
	/// [PC] A field the thread already finished, taken off `rx` by `field_ready` and used by the next tick.
	early: Option<Snap>,
	/// A movie the game thread waits for (fmv_bridge), until the viewer takes it (`take_movie`).
	movie_req: Option<fmv_bridge::Request>,
	/// Buttons held when a movie ended: kept from the game until released (the skip press must not reach it).
	movie_mask: u16,
	/// The movie playing now: what ends it early (retail's rule, fmv_bridge `Skip`) and the pad read at its last poll
	/// (`movie_pressed`); `pad_last` is the pad of the newest `tick_keys`.
	movie_rule: fmv_bridge::Skip,
	movie_prev: u16,
	pad_last: u16,
	/// The camera keys' share of this field's right stick (offset from 0x80; `tick_keys` sets it, `tick` hands it to the modern camera's input).
	kb_rs: [i8; 2],
	snap: Snap,
	/// Class -> its retail props not holding a moby.
	free: HashMap<u32, Vec<usize>>,
	/// [PC] The order `bind` hands the game's props out in (None = index order): after an in-place pool growth the pool
	/// props as a rebuilt game holds them, each class's new props after its old ones (`App::append_pools`).
	order: Option<Vec<usize>>,
	/// Moby address -> the prop drawing it.
	held: HashMap<u32, usize>,
	/// Prop -> the +0x64 joint override records of the moby it draws (only props with records).
	prop_fx: HashMap<usize, Vec<crate::anim::JointFx>>,
	bound: bool,
	/// The level rac-rs has loaded for the props (`switch` follows retail's level travel).
	level: u32,
	/// RAC_RETAIL_TRAVEL's (level, field).
	travel: Option<(u32, u32)>,
	cam: Cam,
	cam_prev: Cam,
	cam_up: [f32; 3],
	cam_up_prev: [f32; 3],
	failed: bool,
	/// The error that stopped the thread, until `take_stop` hands it out.
	stop: Option<String>,
	/// Fields handed to the thread, for `script`.
	fields: u32,
	/// RAC_RETAIL_PAD (shots and tests): `from-to:buttons:lx,ly;...` in hex, held for fields `from..to` over the keys.
	script: Vec<(u32, u32, u16, [u8; 2], Option<[u8; 2]>)>,
	/// RAC_RETAIL_KEYTAP (tests: a hidden window has no keyboard): see `KeyTap`.
	keytap: Vec<KeyTap>,
	/// Movies the game asked for so far (`take_movie`), for `KeyTap` `movieN`.
	movies: u32,
	/// Keys (and the left / middle mouse buttons, `mouse_taps`: Circle / Triangle) pressed since the keyboard was last sampled, with the fields
	/// each still counts as held (`press_key`, `tick_keys`).
	taps: HashMap<winit::keyboard::KeyCode, u8>,
	/// The keys and controller buttons that ended a Controls-page capture (bound, or Esc): the game sees a neutral pad until they
	/// are released, else the bound key would act as the button it now is (Confirm) and the page would start a capture again.
	cap_hold: (Vec<winit::keyboard::KeyCode>, u32),
	mouse_taps: MouseTaps,
	/// Buttons the game's pad update read since `take_pad_seen`.
	pad_seen: u32,
	/// Interpreter time of the last field (ms), for the frame budget.
	pub ms: f32,
	/// Running average of `ms` over about 8 fields (the viewer's catch-up cap: a sustained worker-bound game runs one field per frame).
	pub ms_avg: f32,
	/// The retail HUD and menus come from the GS capture (rac-rs draws none of its own).
	pub gs: bool,
	/// GS-full display (`gsfull_on`): the game's whole picture from its own GS output, drawn on a thread of its own
	/// (gsfull/pipe.rs); `take_gsfull_frame` hands out the newest.
	pub gsfull: Option<Arc<crate::gsfull::pipe::GsFull>>,
	/// Width over height of the window, set by the viewer before `sync` (the moby side test of `SideOnly`).
	pub aspect: f32,
	/// The mouse in the page menus (retail/mouse_menu.rs).
	mm: mouse_menu::State,
	/// Keys held across a menu opening or closing (menu_keys.rs).
	mk: menu_keys::Layer,
	/// Pools `sync` found too small: the grown pool list for the viewer to rebuild the prop slots with (`take_grow`).
	grow: Option<Vec<(u32, u32)>>,
	/// `--title`: the boot to the game's own title screen (retail/title_boot.rs); None without the flag.
	tb: Option<title_boot::Boot>,
}
