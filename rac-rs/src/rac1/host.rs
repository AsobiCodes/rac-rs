//! The R&C1 game host (engine split stage E2, 2026-10-26; plan ENGINE_SPLIT.md): the `App` that runs R&C1 in the
//! engine's window: the winit event handler and its per-frame steps (handler/), the level life cycle (app.rs), the front end,
//! HUD, travel and hero helpers. It drives the engine's renderer (`engine::viewer`: Gpu, effects, smoothing, pacing) and the
//! retail glue (`rac1::retail`). Moved from engine/viewer/ at the same folder depth; the children reach the renderer through
//! `super::` as before, by the imports below.

#![allow(unused_imports)]
use crate::level::{Level, Texture};
use crate::mesh::{Lit, LitKind, Mesh};
use crate::physics::World;
use crate::flow::Flow;
use crate::game::{self, Game, Prop};
use crate::interp;
use crate::overlay::{self, Quad};
use crate::player::{self, Input, Orbit};
use crate::hud as art;
use crate::help::{self, HelpLog};
use crate::settings::Settings;
use crate::{ambience, anim, audio, camera, dynlight, fmv, frontend, map, music, collision, gameplay, iso, mobies, particles, save, scene, shrubs, sky, sound, terrain, ties, toc};
use bytemuck::{Pod, Zeroable};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};
// The renderer's items (Gpu, Scene, Camera, the WGSL SHADER, the submodules gpu, smooth, interp, ...) for the children's `super::`.
use crate::viewer::*;

mod travel;
mod hud;
mod hudart;
mod popups;
mod app;
mod handler;
mod front_mirror;
mod front_modern;
mod front_quit_draw;
mod modern_lang;
mod heroanim;
mod herocache;
mod follow;
mod startup;
mod app_state;
mod sfx;
mod ui_icons;
use hud::*;
use hudart::*;
use popups::*;
use self::app_state::*;
use self::sfx::*;
pub use self::startup::run;

/// Checkpoint snapshot of the non-position environment (FUN_0029a540 -> FUN_0029a6f8; savegame.md SAV-1c).
#[derive(Clone, Copy)]
struct CkptEnv {
	hero_env: (usize, usize, f32, [f32; 3]),
	reverb: ambience::Reverb,
	track: i32,
}

struct App {
	/// Frame-profile bench (viewer/bench.rs): an invisible window and no audio output.
	headless: bool,
	iso_path: String,
	levels: Vec<toc::Level>,
	level: usize,
	gpu: Option<Gpu>,
	cam: Camera,
	keys: HashSet<KeyCode>,
	looking: bool,
	last: Instant,
	/// [PC] Frame cap of the uncapped present modes (viewer/pace.rs).
	pace: pace::Pace,
	/// (png path, frames to wait) for a one-frame capture.
	shot: Option<(String, u32)>,
	/// Camera override (x, y, z, yaw deg, pitch deg) applied after the level loads.
	start: Option<[f32; 5]>,
	/// Collision for play mode: terrain and ties (shrubs are walk-through).
	world: Option<World>,
	/// Play mode.
	game: Option<Game>,
	/// What a new game needs: placed mobies, crate class boxes, kill plane.
	instances: Vec<gameplay::MobyInstance>,
	/// Enemy territories (gameplay header +0x70).
	polygons: Vec<gameplay::Polygon>,
	/// KER-60: the level's grind rails (gameplay header +0x74).
	grind_rails: Vec<gameplay::GrindRail>,
	/// NOV-3.4: the level's raw moby group lists (`gameplay::group_lists`), handed to each new `Game`.
	groups: Vec<Option<Vec<u16>>>,
	cuboids: Vec<gameplay::Cuboid>,
	/// KER-5: the cuboid records' forward basis rows (`gameplay::cuboid_basis`).
	cuboid_basis: Vec<[[f32; 3]; 3]>,
	bounds: HashMap<u32, ([f32; 3], [f32; 3])>,
	death_height: f32,
	/// The level header's ship transform (gameplay::ship).
	ship: Option<([f32; 3], f32)>,
	title: String,
	orbit: Orbit,
	/// The game's follow camera and fixed cameras (camera.rs); the orbit is the fallback.
	rig: Option<camera::CameraRig>,
	/// Mouse x motion while right-dragging, consumed by the rig each frame.
	mouse_dx: f32,
	/// Modern camera (port extra): mouse y motion and wheel zoom, consumed by the rig each frame.
	mouse_dy: f32,
	zoom_acc: f32,
	/// Cursor grabbed and hidden for the Modern camera; window focus.
	grabbed: bool,
	focused: bool,
	/// The Display mode the window was last switched to (handler.rs `display_change`; true = borderless fullscreen).
	fullscreen: bool,
	/// Hero position last frame, to reset the rig on respawn.
	rig_hero: [f32; 3],
	/// Enter play mode right after the first level loads (--play).
	autoplay: bool,
	/// Ratchet's start (feet, yaw) from the gameplay data.
	spawn: Option<([f32; 3], f32)>,
	attack_mouse: bool,
	/// Port extra: the pointer in window pixels, and the left button held on a menu (volume drag).
	cursor: Option<(f32, f32)>,
	menu_drag: bool,
	/// The level script as loaded, copied fresh into `flow` for each play.
	flow_start: Option<Flow>,
	flow: Option<Flow>,
	/// Save-anywhere: where the next `toggle_play` puts the hero (set by a load, save.rs `Where`).
	resume: Option<save::Where>,
	/// Per level left this session: the cutscene controller's progress, help slots seen and saved
	/// bits, so a revisit does not replay the intro (the original's per-level first-visit state).
	visited: HashMap<usize, (Option<i32>, Vec<i16>, std::collections::HashSet<(u32, u32)>)>,
	/// Skip input last frame, so holding it skips one cutscene, not the chain.
	skip_held: bool,
	/// The FMV streaming while a movie scene plays (fmv/player.rs).
	movie: Option<fmv::player::Player>,
	/// Retail front end, mode 6 (New Game intro; viewer/front_mirror.rs): the capture's display-buffer picture (caption screens,
	/// fades) is up, and a movie that ended keeps its last picture under the fade until the next picture covers it.
	fm_ov: bool,
	fm_movie_hold: bool,
	/// The ship's departure sequence after the Galactic Map (viewer/travel.rs).
	travel: Option<travel::Travel>,
	/// The ship takeoff / landing stream playing (travel.rs).
	ship_play: Option<travel::ShipPlay>,
	/// The departure flight's level data: path points and the two camera poses (SCN-4.3a); None without a ship pad.
	flight_data: Option<gameplay::FlightData>,
	/// A level change the script asked for, done after this frame.
	pending_level: Option<usize>,
	/// Live transient debris mobys (decomp/notes/particles.md, FUN_00263be8).
	debris: Vec<particles::Debris>,
	/// Bounding spheres of the debris classes (`MobyClass::bsphere`), for the robot pieces' spin and blast.
	debris_bspheres: HashMap<u32, [f32; 4]>,
	/// WPN-6.6b1: class header +0x46 types of kind 5 / 9 (`MobyClass::kind`), copied to `Game::class_kinds`.
	class_kinds: HashMap<u32, u16>,
	/// BRIDGE-53: class header +0x24 scales (`MobyClass::scale * 1024`) of every loaded class, copied to `Game::class_scales`.
	class_scales: HashMap<u32, f32>,
	/// Class collision blocks for the hero push-out (Game::set_colliders).
	colliders: HashMap<u32, mobies::Collision>,
	/// Ratchet's class and animations; his and the held gadgets' clocks (heroanim.rs).
	hero_anim: Option<(mobies::MobyClass, anim::Animated)>,
	hero_rig: heroanim::HeroRig,
	/// RNG-4c: the hero moby's synced position and yaw (retail +0x10 / +0xc0, written by FUN_002299e8 after the hero's motion);
	/// the water effects of the next tick read it. `None` after a teleport, respawn or load.
	hero_moby: Option<([f32; 3], f32)>,
	/// Pause hero preview items: id -> (hero joint, item mesh in its joint frame).
	hero_items: HashMap<u8, HeroItem>,
	/// Seq index of menu clip 0 in `hero_anim` (None when the clips failed to load).
	menu_clip_base: Option<usize>,
	/// Pause hero extra actors by item id: (class, sequence, scale), drawn after the item in the preview mesh.
	hero_actors: HashMap<u8, Vec<(u32, usize, f32)>>,
	/// Their classes, with the aux clips installed in the sequence slots.
	menu_actor_classes: HashMap<u32, (mobies::MobyClass, anim::Animated)>,
	/// The pause hero preview's item, the clock when it was selected and the pose it fades in from.
	hero_preview_clip: Option<MenuHero>,
	/// The panel item model's class and the clock when it was spawned (respawned when the class changes).
	panel_model: Option<(u32, f32)>,
	/// Worn-item candidates of the hero preview (UI-3b) and the back item's clock.
	hero_worn: WornSet,
	worn_back: Option<anim::BackAnim>,
	/// Animated prop classes (enemies), looping their first sequence for now.
	prop_anims: HashMap<u32, (mobies::MobyClass, anim::Animated)>,
	/// Scene actors other than Ratchet, one model slot each after the game props: (scene id, actor
	/// index, class id); their classes are in `prop_anims`.
	actor_slots: Vec<(u32, usize, u32)>,
	clock: f32,
	/// Recent game messages with seconds left on screen.
	messages: Vec<(String, f32)>,
	/// Shown bolt count, rolling up toward the real one like the game's counter.
	shown_bolts: f32,
	/// Unsimulated time for the fixed-step game loop.
	accum: f32,
	/// PC-FPS-1: render interpolation between the last two game ticks (smooth.rs).
	smooth: smooth::Smooth,
	/// [PC] perf: the joint-record props' kept skinnings and pose tokens (handler/fx_skin.rs; dev s138 kept them in `Smooth`).
	fx_skins: handler::fx_skin::FxSkins,
	/// PC perf: (game frame counter, props address, props count) when the last frame's prop poses were built; a frame with
	/// no game tick and the same stamp moves those poses over instead of skinning them again (same pose keys, same anim state).
	pose_stamp: Option<(u32, usize, usize)>,
	/// PC perf (lane A B1): the hero's posed bundle of the last tick (viewer/herocache.rs).
	hero_cache: Option<herocache::HeroCache>,
	/// The wrench in Ratchet's hand: its vertices and normals in his wrist joint's frame.
	wrench: Option<Posed>,
	/// Wrench joints 0 and 1 in the held pose, in the wrist frame (the hit sweep's endpoints, FUN_00259918).
	wrench_axis: Option<[[f32; 3]; 2]>,
	/// The wrench's own model and animation (swing sequences 3-5 and 10 extend it, decomp/notes/wrench.md section 7).
	wrench_rig: Option<Held>,
	/// Retail mode (M3a): the hero parts of the last gameplay field (held wrench, back objects, +0x64 joint records,
	/// loose class-71 mobys), kept while frozen; and the clocks of the loose wrenches by moby address.
	retail_parts: Option<crate::retail::HeroParts>,
	loose_anims: HashMap<u32, anim::PropAnim>,
	/// The level's decoded cutscenes and their English speech, by scene id.
	scenes: HashMap<u32, (scene::Scene, Option<audio::Clip>)>,
	/// The scene whose speech is playing, and the voice.
	scene_voice: Option<(u32, Option<u64>)>,
	/// NOV-13.5: the playing fanfare voice of the gadget cinematic.
	gadget_voice: Option<u64>,
	/// BRIDGE-37 (d): the shop portrait's voice slot and the id waiting for the voice stream to be free.
	portrait_voice: Option<u64>,
	portrait_pending: Option<u16>,
	/// NOV-34a: the playing ship boarding / landing voice (40000-table cue).
	ship_voice: Option<u64>,
	/// NOV-13.5b: this tick's gadget cinematic frame (camera, fade, actors).
	gadget_frame: Option<game::gadget_scene::SceneFrame>,
	art: HudArt,
	/// Seconds left on the bolt and nanotech counters (they show on a change, then fade).
	hud_t: (f32, f32),
	/// Ammo HUD: the weapon shown, bar/icon fade f1 and text fade f2 (0..1 = bytes +0x70/+0x71 over 8).
	ammo_hud: (Option<u8>, f32, f32),
	hud_seen: (u32, i32),
	/// Nanotech meter tick state (lifetime, f1, f2, displayed; displayed -1 = not initialised) and the whole-tick accumulator.
	nano_hud: (i32, u8, u8, i32),
	/// NOV-30f: nanotech slot 1 holds a live request (cleared by the shop and on expiry, set by damage).
	nano_live: bool,
	nano_acc: f32,
	audio: Option<audio::Audio>,
	sfx: Sfx,
	/// The global bank's menu sounds for the pause menu, loaded the first time it opens in play.
	ui_sfx: Vec<Option<audio::Clip>>,
	/// HelpDesk voices by voice id (None = none on the disc), and the playing one: (help id, voice).
	help_clips: HashMap<usize, Option<audio::Clip>>,
	help_voice: Option<(u32, u64)>,
	/// The level's positional ambient emitters and, per emitter, its voice and retrigger countdown.
	emitters: Vec<ambience::Emitter>,
	emitter_state: Vec<(Option<u64>, f32)>,
	/// SCH-6b: the class table's loaded index per class (the effect mobys' scheduler rank).
	class_ranks: std::collections::HashMap<u32, u32>,
	/// [PC] The host-side parse of the level retail is travelling to, started early (`prefetch_level`): (level, thread).
	prefetch: Option<(usize, Box<dyn std::any::Any + Send>)>,
	/// [PC] The channels to that thread: what the slot order needs from it, and the order back (it builds the scene meanwhile).
	prefetch_link: Option<(usize, app::PrefetchLink)>,
	/// The level switch waits for the prefetch's scene build (no new picture meanwhile).
	switch_hold: bool,
	/// [PC] Frames left in which the world draws black after a retail level switch, until retail presents its first picture of the
	/// new level (its fade-in overlay is not up on the switch frame: the new level flashed in full brightness).
	arrival_black: u32,
	/// The level a prefetch was started for last (not started twice for one travel).
	prefetch_tried: Option<usize>,
	class_rows: std::collections::HashMap<u32, u32>,
	/// BRIDGE-24: box-emitter plays decided in the tick (emitter index, point, gain), started by the frame's voice code.
	ambient_plays: Vec<(usize, [f32; 3], f32)>,
	/// BRIDGE-24: volume boxes whose countdown fired in the tick.
	volume_start: Vec<bool>,
	/// Doppler / underwater listener state (sound.rs) and its 60 Hz tick accumulator.
	listener: sound::ListenerVelocity,
	/// NOV-28: the camera zone fog (sticky, reset to the level fog at load) and the fog swap computed last frame.
	fog_zone: ([u8; 3], [f32; 4]),
	fog_next: Option<([u8; 3], [f32; 4])>,
	/// NOV-28.2: next frame's pool-particle far cap, chosen with `fog_next`.
	part_far_next: f32,
	/// Voices of the level's volume boxes (type 1) and underwater loops (type 5), by zone.
	volume_voices: Vec<Option<u64>>,
	underwater_voices: Vec<Option<u64>>,
	/// Per toad/robot prop index: animation sound cue clock and loop-sound voice (sound.md "Animation sound cues").
	enemy_sfx: HashMap<usize, EnemySfx>,
	live_shots: Vec<LiveShot>,
	/// BRIDGE-50: the voice of the last `Event::HeldSound` (0: none).
	held_click: u64,
	/// NOV-6.3: looping class-sound voices by owner prop index (`game::Event::SoundLoop`).
	loops: HashMap<usize, u64>,
	/// AUD-11i: loop owners whose voice is wanted (their cue), driven each frame by `loop_act`.
	loop_want: HashMap<usize, sound::Cue>,
	/// BRIDGE-49: the bend value the game last wrote per loop owner (`game::Event::LoopBend`).
	loop_bend: HashMap<usize, i32>,
	/// The hero's death sequence trigger clock (sequence, position) for its class 0 cues.
	/// The level's music in slot order (intro, loop, ...), and the playing (voice, slot, seconds).
	music: Vec<audio::Clip>,
	/// Music controller (music.rs) and its main-stream and stinger voices as (voice, slot); the
	/// zones, spawn areas and the reverb request they feed (ambience.md).
	music_player: Option<music::Player>,
	music_voice: Option<(u64, usize)>,
	sting_voice: Option<(u64, usize)>,
	music_deaths: u32,
	zones: ambience::Zones,
	areas: Vec<ambience::Area>,
	reverb: ambience::Reverb,
	/// SAV-1c: checkpoint environment captured after the last commit (None until the first), and the serial seen.
	ckpt_env: Option<CkptEnv>,
	ckpt_seen: u32,
	/// The pause menu has frozen every voice (music.md "Pause audio").
	audio_paused: bool,
	/// Last frame's state, to turn changes into sounds.
	prev_swing: Option<usize>,
	prev_shown: Vec<bool>,
	/// Effect pool (particles.rs) and the level's class-760 smoke emitters with their volumes.
	particles: particles::Particles,
	smoke: Vec<(particles::SmokePvars, particles::Cuboid)>,
	/// Class 760 fire quads (fire.rs).
	fires: Vec<crate::fire::Fire>,
	/// Ripple water (Novalis only so far).
	water: Option<crate::water::Water>,
	/// NOV-14.5b: the cutscene ship gloss (Novalis only) and the quads of the tick it last ran.
	ship_gloss: Option<crate::shipgloss::ShipGloss>,
	/// ARI-49a / KER-30: the Aridia (class 854) or Kerwan (class 994) ocean; None on other levels.
	ocean: Option<crate::ocean::Ocean>,
	/// The level overlay image: Veldin's ship reflection tables (REN-2), Novalis' strip tables (NOV-15.1b / NOV-23.1).
	level_overlay: Option<Vec<u8>>,
	gloss_quads: Vec<particles::WorldQuad>,
	/// This frame's blast moby draws, and the scene-5 ship crash state (last tick handled, previous joint 5).
	effect_mobys: Vec<particles::EffectMoby>,
	sprites: Vec<particles::Sprite>,
	ribbons: Vec<particles::Ribbon>,
	/// KER-92: this frame's one-pixel pool lines (type-0x31 tracers).
	lines: Vec<particles::Line>,
	/// Load the front end (title scene and menu) instead of `level` on the next `load_level`.
	want_front: bool,
	front: Option<FrontEnd>,
	/// The save slot being played (None = not saving) and its image, kept current on level changes.
	save: Option<(Option<usize>, save::Save)>,
	/// Play timer A3 (1/60 s units, PAL step in save::tick_play), its 50 Hz accumulator and the
	/// post-menu skipped frame flag.
	a3_ticks: u32,
	a3_acc: f32,
	a3_skip: bool,
	a3_menu: bool,
	was_dead: bool,
	/// Circle on the left mouse button in retail play (middle in the clean-room game); the `--give` item; fixed steps since play started.
	fire_mouse: bool,
	/// Triangle (Quick Select) on the middle mouse button in retail play.
	quick_mouse: bool,
	give: Option<(u8, Option<u16>)>,
	play_ticks: u32,
	/// RAC_ATTACK_TICKS: play ticks the wrench is pressed for 3 ticks (scripted --shot scenes).
	attack_script: Vec<u32>,
	/// The two camera shake records (camera.md "Shake consumer").
	shake: camera::CamShake,
	/// The held weapons: per gadget class, its model (bind pose and animation) and the hero joint it
	/// follows (wrench 71 at the wrist 56, glove 192 at joint 54), plus the bomb in the glove.
	glove: Option<Held>,
	bomb: Option<(Vec<[f32; 3]>, Vec<[f32; 3]>)>,
	/// The held Pyrocitor (game/pyro.rs), the muzzle it put in the world last frame and its firing loop voice.
	pyro_rig: Option<PyroRig>,
	/// The held Suck Cannon (class 849, WPN-5.1b2), its suction loop voice and the held matrix it posed last frame.
	suck_rig: Option<PyroRig>,
	/// BRIDGE-38 (WPN-6.0): the generic held-weapon rigs by class (game/held.rs `HELD_RIG_CLASSES`) and the muzzle posed last frame.
	held_rigs: HashMap<u32, PyroRig>,
	/// BRIDGE-76 (ARI-47c): the Swingshot hook model (class 209), drawn at the game's hook with the hero mesh's held rigs.
	hook_rig: Option<Held>,
	held_muzzle: Option<[f32; 3]>,
	/// BRIDGE-38 (c) extension (WPN-6.2a): the posed held weapon's world matrix rows and position, posed last frame.
	held_xform: Option<([[f32; 3]; 3], [f32; 3])>,
	/// BRIDGE-38 (d) (WPN-6.4a): the posed held weapon's joint-0 rotation rows in the world, posed last frame.
	held_joint0: Option<[[f32; 3]; 3]>,
	/// BRIDGE-21: the hero's back objects d0 (class 607) and d4 (class 601) and the hero joint of attach point 5 they sit on.
	back_rig: Option<Held>,
	clank_rig: Option<Held>,
	back_joint: Option<usize>,
	/// BRIDGE-21c: the Clank light (class 1204) mesh and this frame's glow position in the world.
	light_rig: Option<mobies::MobyClass>,
	clank_light_glow: Option<[f32; 3]>,
	suck_voice: Option<u64>,
	suck_matrix: Option<[[f32; 3]; 3]>,
	pyro_muzzle: Option<[f32; 3]>,
	/// NOV-14.7b-draw: UV extent of the glint sprite, 32 texels over the width of fx texture 0x13.
	glint_u: f32,
	/// ARI-4: the Aridia sky sprite records (None until the first tick after a load creates them).
	sky_sprites: Option<Vec<sky::SkySprite>>,
	/// FRAME-1: the rig's newest tick camera plus shake (pos, yaw, pitch, tan_half) for the per-tick glint cull.
	glint_cam: Option<([f32; 3], f32, f32, f32)>,
	/// BRIDGE-56: the up of `glint_cam` (the rig's `up()`; Z when `glint_cam` is None).
	glint_up: [f32; 3],
	/// FRAME-1e: this tick's scene / ship stream / gadget camera (pos, yaw, pitch, tan_half) while one owns the view.
	over_cam: Option<([f32; 3], f32, f32, f32)>,
	/// FRAME-1c: a sim tick of this frame stepped the rig while it owned the view (glint cull and draw verdict ran per tick).
	tick_cam_live: bool,
	pyro_voice: Option<u64>,
	/// RNG-4b: voices granted at their request point in the tick, and the pitch draw of each running voice.
	voices: VoiceBook,
	pyro_frames: game::pyro::Frames,
	/// The Options values (settings.rs) and whether they still need writing to disk.
	settings: Settings,
	settings_dirty: bool,
	/// The pause menu or Galactic Map over the frozen game (decomp/notes/ingame_pause_menu.md), with
	/// the all_text strings and font it draws with.
	pause: Option<frontend::Menu>,
	/// SCN-6 (decomp/notes/camera.md "SCN-6 port spec"): the projection saved at pause open (retail 0x1ba1d8) and
	/// restored on every pause close; the paused backdrop uses the default 0.63 projection.
	pause_saved_tan_half: Option<f32>,
	ui_text: Option<frontend::Text>,
	/// The text index (0..=4) `ui_text` was last loaded with; None = never (host/modern_lang.rs reloads it when the game's language differs).
	ui_text_lang: Option<usize>,
	ui_art: Option<frontend::Art>,
	help_log: HelpLog,
	/// Autosave enabled ([0x13d384]): set by New/Load Game and manual save, cleared by Quit and IO errors.
	autosave_on: bool,
	/// Seconds the HUD save icon still shows, and whether help 20011 has been shown this session.
	save_icon: f32,
	save_help: bool,
	/// A cutscene was playing last frame (its end is an autosave point).
	was_scene: bool,
	/// The pickup popup: text id, hold timer (50 Hz ticks) and fade alpha 0..=128 (decomp/notes/ingame_menu_leftovers.md P1-b).
	popup: Option<(u32, i32, i32)>,
	/// Triangle (T) held seconds, and the Quick Select wheel's sector while it is open.
	tri_held: Option<f32>,
	wheel: Option<usize>,
	/// The wheel's fade 0..1 (up while open, down after release) and the sector it shows.
	wheel_fade: (f32, usize),
	/// The level's Galactic Map (decomp/notes/ingame_menu_leftovers.md P1-c).
	map: Option<MapView>,
	/// Retail mode (retail.rs): the interpreter that runs the level instead of the rac-rs game step.
	retail: Option<crate::retail::Retail>,
	/// The running `Retail` across a retail level travel (`Retail::switch`), for the next `toggle_play`.
	retail_carry: Option<crate::retail::Retail>,
	/// Retail mode: the level's prop meshes (`load_level`), kept so a pool that outgrew its slots rebuilds only the prop
	/// slots (`regrow_props`), not the level.
	retail_prop_meshes: HashMap<u32, crate::mesh::Mesh>,
}

/// Class 760: the crash-site smoke emitter.
const SMOKE: u32 = 760;
