//! `Snap`, what the main thread needs of one field, and `snap`, which reads it from the interpreter's RAM; the scene
//! sequences it copies out and the native draw's delay against the GS overlay.

use super::*;

/// The effects layer of one field, still being built on the VU1 thread (`efx_vu::Shadow::submit`): `Retail::tick` waits for it
/// after the next field's pad is out and appends it to `fx.qa` / `fx.pools` (what `efx_draw::build` appended on the worker).
pub enum FxLate {
	/// The VU1 thread's answer (the inline path).
	Now(std::sync::mpsc::Receiver<efx_vu::FxOut>),
	/// The picture thread hands the receiver of the VU1 thread's answer over once it has walked the field (`snap_pipe`).
	Later(snap_pipe::Late),
}

impl FxLate {
	/// Waits for the field's effects layer.
	pub(super) fn get(self) -> Option<efx_vu::FxOut> {
		match self {
			FxLate::Now(r) => r.recv().ok(),
			FxLate::Later(l) => l.recv().ok().flatten().and_then(|r| r.recv().ok()),
		}
	}
}

/// What the main thread needs of one field: live mobys, the hero moby (pos, yaw) and the camera.
#[derive(Default)]
pub struct Snap {
	pub mobys: Vec<Moby>,
	/// The page menu on screen (mode 3), for the mouse (retail/mouse_menu.rs).
	pub menu: Option<mouse_menu::MenuSnap>,
	/// The vendor's shop as a one-item menu for the mouse while its screen is up (retail/mouse_vendor.rs).
	pub vend: Option<mouse_menu::MenuSnap>,
	/// The front end (frontbin: title, main menu, New / Load Game) runs instead of a level: nothing of a level is mirrored,
	/// `mode` is the front end's mode word 0x15f5e4 and the picture is the GS's own (title_boot.rs).
	/// A ship fight is on (retail/shipmouse.rs): the left mouse button fires with Cross there.
	pub ship_fight: bool,
	/// The hero's body form byte 0x1413f4: 0 Ratchet, 1 Clank, 2 Giant Clank, 3 Hologuise (`live_pad`, R2 for the pad).
	pub form: u8,
	/// Retail's own occlusion mask of this field (128 bytes at 0x174100 + the level shift; None while loading / outside a level).
	pub occ_mask: Option<[u8; 128]>,
	pub front: bool,
	/// Quick Select wheel is up (its HUD element's fade byte is not 0; `wheel_mouse::open_in`).
	pub wheel_up: bool,
	/// The front end is inside the level-load loop of FUN_00231b00 or its frontbin copy (`in_load_loop`): the main thread runs its
	/// fields unthrottled.
	pub turbo: bool,
	/// The front end's own state out of RAM, for the mirror's picture of it (`FrontRam`; Some in the front end only).
	pub front_ram: Option<FrontRam>,
	pub(super) hero: Option<([f32; 3], f32, Clock)>,
	/// Hero moby +0x34 bit 0 (FUN_00248228; set every hero tick while 0x1413f5, e.g. the shop, vendor.md NOV-30g) [C].
	pub(super) hero_hidden: bool,
	/// `frozen_view` of this field (the main thread keeps the last frame), and `scene_view`.
	pub frozen: bool,
	pub scene: bool,
	/// `frozen` or a field retail does not present (`NO_PRESENT`): the main thread keeps the last picture. Kept apart from
	/// `frozen` (= `paused()`), which also pauses the music and paces the window.
	pub hold: bool,
	/// Retail's no-present word `NO_PRESENT` is set this field (a frozen mode alone does not set it: the menu modes present
	/// their own picture every field, `menu_presents`).
	pub no_present: bool,
	/// The mode-6 substate `SHIP_STATE` (0 outside mode 6).
	pub(super) ship: u32,
	/// The hero moby's draw verdict +0x31 (a scene hides the placed hero and draws its own class-0 actor).
	pub(super) hero_shown: bool,
	/// Scene actors' sequences read from RAM this field: (class, slot +0x53, `scene_seq` block).
	pub(super) scene_seqs: Vec<(u32, u8, Vec<u8>)>,
	/// Mode 6 substate 4: retail's arrival-warp state (retail/warp.rs).
	pub(super) warp: Option<warp::WarpRam>,
	/// Mode 6 substates 0 / 8 (takeoff / landing stream): the ship gloss quads (canopy) the cutscene ship callback
	/// FUN_002a69d8 left in RAM this field (`warp::gloss`, the same tables the warp reads).
	pub(super) ship_gloss: Vec<crate::particles::WorldQuad>,
	pub(super) cam: Option<Cam>,
	/// The render camera's up row (basis row 2, roll included) read with `cam`; zero = not read.
	pub(super) cam_up: [f32; 3],
	/// Hero health 0x1415f8 and the bolt counter 0x15ed98, for the rac-rs HUD.
	pub(super) health: i32,
	pub(super) bolts: u32,
	/// Owned flags 0x13d4c0, ammo 0x13d428, equipped 0x140408 and last-chosen 0x141660 (inventory.rs).
	pub(super) have: Vec<bool>,
	pub(super) ammo: Vec<u16>,
	pub(super) equipped: u8,
	pub(super) last: u8,
	/// The pad as the game's pad update left it (pad struct 0x13c940, Eudora FUN_00259330): held +0x1a0, new presses +0x1a4
	/// (both cut to Start / Select while a menu locks the pad, +0x1cc = 2), and the uncut copies +0x1c0 / +0x1c4 Quick Select reads.
	pub(super) pad: [u32; 4],
	/// The driver's SPU2 register writes of the field (spumix.rs), played by the viewer.
	pub(super) sound: SndBatch,
	/// The retail GS overlay finished since the last field (oracle gs.rs): (width, rows, RGBA8 with rgb = the
	/// premultiplied colour and a = how much of rac-rs's scene shows through).
	pub(super) ov: Option<(u32, u32, Vec<[u8; 4]>, HudGuard)>,
	/// `ov` was drawn into the display buffer on a no-present field (gscap `Overlay::disp`): it covers what the
	/// display showed, a held movie picture too.
	pub(super) ov_disp: bool,
	/// The overlay of this field, not taken yet (in-game only): the worker hands the field over without waiting for the GS
	/// capture thread; `Retail::resolve_ov` resolves it at its first reader, after `tick` has sent the next field's pad (`ov` / `ov_disp`
	/// then hold it).
	pub(super) ov_late: Option<OvLate>,
	pub(super) fx_late: Option<FxLate>,
	/// Game mode 0x15f5a4 (0 gameplay, 3 pause menu; ingame_pause_menu.md): the menu saves the camera and points
	/// the camera globals at its own scene.
	pub(super) mode: u32,
	/// Retail level 0x15ed84, and the new level's prop pools once its first gameplay field ran (level travel).
	pub(super) level: u32,
	pub(super) pools: Option<Vec<(u32, u32)>>,
	/// Nanotech orbs (class 806, no model) the field queued for drawing; None while frozen (keep the last).
	pub orbs: Option<Vec<crate::particles::OrbDraw>>,
	/// The effect pool and the glow / marker draw-queue callbacks (`queue_fx`); None while frozen (keep the last).
	pub fx: Option<RetailFx>,
	/// The vendor shop's off-list mobys while the shop screen is up (`shop_snap`).
	pub shop: Option<ShopSnap>,
	/// The mode-4 dialog's keys (Quit Race? / Quit? ..., `menu_keys::dialog_on`) while one is up.
	pub dialog: Option<menu_keys::Screen>,
	/// Retail's held moby (`HELD_MOBY`) while it is in the table: (address, class +0xa6, clock +0x50..+0x5f).
	pub held: Option<(u32, u32, Clock)>,
	/// The held wrench, back objects, joint records and loose wrenches (`HeroParts`).
	pub parts: HeroParts,
	/// Hero moby +0x40 / +0x44: the Euler x / y its draw rows +0xc0 are built from (FUN_00265770, with yaw +0x48).
	pub(super) hero_tilt: [f32; 2],
	/// M3a shadows: the shadow-volume records of the drawn frame and the deferred draw path; None while frozen (keep the last).
	pub shadows: Option<RetailShadows>,
	/// [PC] Frame smoothing (viewer/interp.rs): `fx`, `shadows` and `orbs` blended toward the field before, drawn in their place
	/// between two fields. A new snap starts without (None), so each field draws its own lists until the viewer blends again.
	pub smooth: Option<Box<SmoothDraw>>,
	/// Screen words of the world draw FUN_001eae20 (debug, RAC_SCENE_LOG): skip 0x15f5b8, draw mask 0x15f3f4, black fade
	/// 0x15f3fc, white fade 0x15f400, fade gate 0x16a134.
	pub(super) screen: (u32, u32, f32, f32, u32),
	/// GS PMODE, DISPFB1, BGCOLOR (privileged registers, debug).
	pub(super) crtc: (u64, u64, u64),
	pub(super) ms: f32,
	pub(super) err: Option<String>,
	/// The field this snap is the state of, counted from 1 (`Retail::fields` when `tick` took it): the label of the
	/// RAC_SCENE_LOG lines and of RAC_SHOT_FIELD shots (gsfull frame `field` + 1).
	pub(super) field: u32,
}

/// The native draw's delay against the GS overlay (hybrid). Measured [C stage 9, warp fade 0164 f718-722, field-exact
/// shots]: the GS picture of field N (--gsfull, and the capture overlay that equals it, stage 7) shows the state of field
/// N-2, the native draw that of field N. So the natively drawn state (mobys, hero, camera, warp, scene, view flags) is the
/// snap `d` fields back, with this field's GS overlay, sound, pad and HUD values:
/// - d = 2 (retail-exact, native = overlay) while the player has no camera control or a fade matters: any mode but 0
///   (scenes, ship travel / warp, shop, menus) and a mode-0 fade (black 0x15f3fc or white 0x15f400) that is starting or
///   holding. The fade start is in RAM 2 fields before the overlay shows it, so the newest snap decides.
/// - d = 0 in plain gameplay (mode 0, no fade) and during a mode-0 fade-in (the fade word falling): no added mouse-camera
///   latency; the overlay carries the fade darkness itself [C mirror_m3b: FUN_001eae20 draws it through the GS overlay].
/// Growing holds the last picture (the held snaps are never shown twice). Shrinking drops only snaps nobody sees: a
/// held / frozen field, or one under a full black or white fade; 25 fields after gameplay wants 0 with no such field, one
/// snap per field is dropped anyway (`RAC_SCENE_LOG` prints `native delay`).
/// RAC_RETAIL_NATIVE_DELAY=d forces a fixed d (0 = the old undelayed draw).
pub(super) fn native_delay(s: Snap) -> Snap {
	static FORCED: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
	native_delay_with(s, *FORCED.get_or_init(|| std::env::var("RAC_RETAIL_NATIVE_DELAY").ok().and_then(|v| v.parse().ok())))
}

/// `native_delay` with the forced delay given (None = the rules above); the tests pass None so the environment cannot change them.
fn native_delay_with(s: Snap, forced: Option<usize>) -> Snap {
	NATIVE_Q.with(|q| {
		let mut nd = q.borrow_mut();
		if s.mode != nd.mode {
			eprintln!("retail: mode {} -> {} at field {}", nd.mode, s.mode, s.field);
			nd.mode = s.mode;
		}
		nd.form = s.form;
		if forced == Some(0) {
			return s;
		}
		let f = s.screen.2.max(s.screen.3);
		let want = match forced {
			Some(d) => d,
			None if s.mode != 0 => 2,
			// Mode 0: a fade starting or growing wants the overlay's timing, a fade-in (falling) gameplay's.
			None if f > nd.fade && f > 0.0 => 2,
			None if f < nd.fade || f <= 0.0 => if s.hold { nd.want } else { 0 },
			None => nd.want,
		};
		nd.fade = f;
		nd.want = want;
		nd.q.push_back(s);
		let unseen = |x: &Snap| x.hold || x.frozen || x.screen.2 >= 1.0 || x.screen.3 >= 1.0;
		if nd.q.len() > want + 1 {
			nd.late = if want == 0 && nd.q.iter().all(|x| x.mode == 0) { nd.late + 1 } else { 0 };
		} else {
			nd.late = 0;
		}
		// Shrink: drop the oldest queued snaps while the delay is above the wanted one and nobody would see them.
		while nd.q.len() > want + 1 && (unseen(&nd.q[0]) || nd.late > 25) {
			let x = nd.q.pop_front().unwrap();
			if std::env::var_os("RAC_SCENE_LOG").is_some() {
				eprintln!("retail: native delay drop field {} (unseen {}) -> delay {}", x.field, unseen(&x), nd.q.len() - 1);
			}
			if nd.late > 25 {
				break;
			}
		}
		let mut out = if nd.q.len() > want {
			nd.q.pop_front().unwrap()
		} else {
			// Grow: hold the last picture until `want` snaps are queued.
			let n = nd.q.back().unwrap();
			let (mode, level, ship, scene, frozen, hero_shown) = nd.last.unwrap_or((n.mode, n.level, n.ship, n.scene, n.frozen, n.hero_shown));
			Snap { hold: true, no_present: true, mode, level, ship, scene, frozen, hero_shown, ..Default::default() }
		};
		let s = if nd.q.is_empty() { None } else { nd.q.back_mut() };
		if let Some(s) = s {
			std::mem::swap(&mut out.sound, &mut s.sound);
			std::mem::swap(&mut out.ov, &mut s.ov);
			std::mem::swap(&mut out.ov_disp, &mut s.ov_disp);
			std::mem::swap(&mut out.ov_late, &mut s.ov_late);
			out.menu = s.menu.clone();
			out.front = s.front;
			out.front_ram = s.front_ram.clone();
			std::mem::swap(&mut out.pools, &mut s.pools);
			std::mem::swap(&mut out.err, &mut s.err);
			(out.pad, out.ms, out.field, out.health, out.bolts, out.equipped, out.last) = (s.pad, s.ms, s.field, s.health, s.bolts, s.equipped, s.last);
			out.have = s.have.clone();
			out.ammo = s.ammo.clone();
		}
		let d = nd.q.len();
		if d != nd.shown_d && std::env::var_os("RAC_SCENE_LOG").is_some() {
			eprintln!("retail: native delay field {} mode {} fade {f} want {want} -> delay {d}{}", out.field, out.mode, if out.hold { " (hold)" } else { "" });
		}
		nd.shown_d = d;
		nd.last = Some((out.mode, out.level, out.ship, out.scene, out.frozen, out.hero_shown));
		out
	})
}

/// The game mode and the hero's body form of the newest snapshot taken (before the delay): what the next field's pad is decided in
/// (`Retail::tick_inner`). It reads `native_delay`'s thread-local state, so it MUST run on the thread that calls `native_delay`
/// (the main thread, `Retail::tick_inner`; STATE.md NATIVE_Q): on another thread it would return (0, 0) forever, R2 would always
/// be the aim (STATE.md says so; the state is per thread so tests with several `Retail`s do not cross-talk).
pub(super) fn live_pad() -> (u32, u8) {
	NATIVE_Q.with(|q| {
		let nd = q.borrow();
		(nd.mode, nd.form)
	})
}

/// `native_delay`'s state (main thread): the queued newer snaps, the wanted delay, the newest fade word, the view flags
/// of the last shown snap (a growing hold keeps them), the delay last logged and the fields gameplay waited to shrink.
#[derive(Default)]
struct NativeDelay {
	pub(super) q: std::collections::VecDeque<Snap>,
	pub(super) want: usize,
	pub(super) fade: f32,
	pub(super) last: Option<(u32, u32, u32, bool, bool, bool)>,
	shown_d: usize,
	pub(super) late: u32,
	/// The newest snap's game mode (the `mode a -> b` log line).
	pub(super) mode: u32,
	/// The newest snap's hero body form.
	pub(super) form: u8,
}

/// The scene sequence moby `a` plays, copied out of RAM: scene_chunk_parse appends each actor's sequence to its class
/// (`class[+0x48 + 4 * slot]` = actor record + 0x10, slot = the class's sequence count, moby +0x52 = +0x53 = slot) and
/// relocates the frame words at header +0x1c to absolute addresses. Returned as one block in `anim::parse_sequence`'s
/// layout (header, then each frame with its offset written back), or None when +0x53 is not a scene sequence: the
/// record's class word (+0x00) must be the moby's class (or 0x215, the ship actor mode 6 swaps for the ship type), the
/// trigger count +0x12 0xff (every scene record) and every frame after the header.
pub(super) fn scene_seq(m: &ee_oracle::Machine, a: u32, class: u32) -> Option<Vec<u8>> {
	let hdr = m.rd32(a + 0x24) & 0x01ff_ffff;
	let p = m.rd32(hdr + 0x48 + 4 * m.rd8(a + 0x53) as u32) & 0x01ff_ffff;
	if !(0x100..0x01ff_0000).contains(&p) || m.rd8(p + 0x12) != 0xff {
		return None;
	}
	let rec = m.rd32(p - 0x10);
	if rec != class && rec != 0x215 {
		return None;
	}
	let n = m.rd8(p + 0x10) as u32;
	if n == 0 {
		return None;
	}
	let head = 0x1c + 4 * n;
	let mut out: Vec<u8> = (0..head).map(|i| m.rd8(p + i)).collect();
	for k in 0..n {
		let f = m.rd32(p + 0x1c + 4 * k) & 0x01ff_ffff;
		if f < p + head || f - p > 0x0010_0000 {
			return None;
		}
		let size = 16 + 16 * m.rd16(f + 6) as u32;
		let at = (0x1c + 4 * k) as usize;
		let off = (out.len() as u32).to_le_bytes();
		out[at..at + 4].copy_from_slice(&off);
		out.extend((0..size).map(|i| m.rd8(f + i)));
	}
	Some(out)
}

/// What `Retail::scene_seqs` last put into the viewer's class animations: (class, slot) -> the RAM block it parsed, and
/// the viewer's own sequence it replaced (rac-rs's hero class has more sequences than retail's 134, so a scene slot can
/// land on one of rac-rs's extras), put back when the scene ends. Main thread only.
#[derive(Default)]
pub(super) struct SceneSeqs {
	pub(super) done: HashMap<(u32, u8), Vec<u8>>,
	pub(super) orig: HashMap<(u32, u8), Option<crate::anim::Sequence>>,
	/// The hero / back-Clank / held-item hide flags from before the scene (`sync` restores them after it).
	pub(super) saved: Option<(bool, bool, bool)>,
}

thread_local! {
	pub(super) static SCENE_SEQS: std::cell::RefCell<SceneSeqs> = std::cell::RefCell::new(SceneSeqs::default());
	/// `native_delay`'s queue of newer snaps (main thread).
	static NATIVE_Q: std::cell::RefCell<NativeDelay> = std::cell::RefCell::new(Default::default());
}

/// The new level after travel is up: gameplay (mode 0), an in-engine scene (mode 2), or the landing stream (mode 6
/// substate 8) once the world draws again (draw mask 0x15f3f4 nonzero; it is 0 while the level loads behind the warp
/// [C: departure 0164 log 2026-10-12]). The pools are counted then, so arrival scenes draw live on the new level.
pub(super) fn arrived(s: &Snap, m: &ee_oracle::Machine) -> bool {
	matches!(s.mode, 0 | 2) || (s.mode == 6 && s.ship == 8 && m.rd32(0x15f3f4) != 0)
}

/// The plain RAM words of a snapshot: HUD numbers, inventory, the body form and the pad words.
fn read_words(m: &ee_oracle::Machine, mode: u32, level: u32) -> Snap {
	Snap {
		health: m.rd32(0x1415f8) as i32,
		bolts: m.rd32(0x15ed98),
		mode,
		level,
		have: (0..crate::inventory::GADGETS as u32).map(|i| m.rd8(0x13d4c0 + i) != 0).collect(),
		ammo: (0..crate::inventory::GADGETS as u32).map(|i| m.rd32(0x13d428 + 4 * i).min(0xffff) as u16).collect(),
		equipped: m.rd32(0x140408) as u8,
		form: m.rd8(0x1413f4),
		last: m.rd32(0x141660) as u8,
		pad: [m.rd32(0x13cae0), m.rd32(0x13cae4), m.rd32(0x13cb00), m.rd32(0x13cb04)],
		wheel_up: super::wheel_mouse::enabled() && super::wheel_mouse::open_in(level, |a| m.rd8(a)),
		..Snap::default()
	}
}

pub fn snap(m: &ee_oracle::Machine) -> Snap {
	let level = m.level();
	let mode = m.rd32(va(0x15f5a4));
	let mut out = read_words(m, mode, level);
	let ship = if mode == 6 { m.rd32(SHIP_STATE) } else { 0 };
	out.ship_fight = shipmouse::in_ship_fight(m);
	(out.frozen, out.scene, out.ship) = (frozen_view(mode, ship), scene_view(mode, ship), ship);
	out.no_present = m.rd32(va(NO_PRESENT)) != 0;
	out.hold = out.frozen || out.no_present;
	let c = crate::gsfull::crtc::Crtc::from_regs(&m.cpu.mem.gs);
	out.crtc = (c.pmode, c.dispfb[0], c.bgcolor);
	out.screen = (m.rd32(va(NO_PRESENT)), m.rd32(0x15f3f4), m.rdf(0x15f3fc), m.rdf(0x15f400), m.rd32(va(0x16a134)));
	out.warp = warp::read(m, mode, level);
	out.menu = mouse_menu::read(&|a| m.rd32(a), level, mode);
	prompts::set_menu(out.menu.is_some());
	menu_keys::publish(out.menu.as_ref().map(menu_keys::screen));
	// A mode-4 dialog is no page menu but its keys and prompts work the menu way (menu_keys::dialog).
	out.dialog = menu_keys::dialog_on(&|a| m.rd32(a), level, mode);
	if out.menu.is_none() && out.dialog.is_some() {
		prompts::set_menu(true);
		menu_keys::publish(out.dialog);
	}
	if mode == 6 && matches!(ship, 0 | 8) {
		if LEVEL_SHIFT.get(level as usize).is_some() {
			let cl = crate::game::transit::warp_ship_class((m.rd8(0x13e056) as usize).min(2));
			out.ship_gloss = queue_story::gloss_for(m, level, cl, 0x15);
			if std::env::var_os("RAC_SCENE_LOG").is_some() {
				let n = out.ship_gloss.len() as f32 * 4.0;
				let c = out.ship_gloss.iter().flat_map(|q| q.pos).fold([0.0f32; 3], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n, a[2] + p[2] / n]);
				eprintln!("retail: ship gloss level {level} ship {ship} class {cl} quads {} centroid {c:?} colour {:?}", out.ship_gloss.len(), out.ship_gloss.first().map(|q| q.colour[0]));
				let (lo, hi) = out.ship_gloss.iter().flat_map(|q| q.pos).fold(([f32::MAX; 3], [f32::MIN; 3]), |(l, h), p| ([l[0].min(p[0]), l[1].min(p[1]), l[2].min(p[2])], [h[0].max(p[0]), h[1].max(p[1]), h[2].max(p[2])]));
				eprintln!("retail: ship gloss bbox {lo:?} {hi:?} q0 {:?}", out.ship_gloss.first().map(|q| (q.pos, q.uv)));
			}
		}
	}
	// Mode 6: the ship thruster flames (queue A callback FUN_0027f7e8, retail/warp.rs `flames`) ride with the gloss quads.
	if mode == 6 {
		out.ship_gloss.extend(warp::flames(m, level));
	}
	// Cutscenes and mode 0: the queued ship's gloss (queue_story.rs).
	if mode != 6 && !out.hold {
		out.ship_gloss = queue_story::cutscene_gloss(m, level);
	}
	// Where retail draws no world (pause menu, movie, level load) the main thread keeps the last frame; the table may be
	// mid-load.
	if out.hold {
		return out;
	}
	out.cam = super::wide_side::field(read_cam(m, level));
	out.cam_up = read_cam_up(m, level);
	out.orbs = Some(orb_draws(m, level));
	out.fx = Some(queue_fx(m, level));
	out.shop = shop_snap(m, level);
	// The vendor's shop is no page menu but its keys, prompts and mouse work the menu way (menu_keys::VENDOR, mouse_vendor.rs).
	if out.menu.is_none() && out.shop.is_some() {
		out.vend = super::mouse_vendor::read(&|a| m.rd32(a), level);
		prompts::set_menu(true); menu_keys::publish(Some(menu_keys::VENDOR));
	}
	let hp = m.rd32(HELD_MOBY) & 0x01ff_ffff;
	let view = DrawView::read(m, level);
	out.occ_mask = view.as_ref().map(|v| v.mask);
	let (s, e) = table(level);
	let (s, e) = (m.rd32(s) & 0x01ff_ffff, m.rd32(e) & 0x01ff_ffff);
	let mut a = s;
	let mut walked = true;
	let mut hero_addr = 0;
	while a < e {
		let hdr = m.rd32(a + 0x24);
		let class = m.rd16(a + 0xa6) as u32;
		let slot = m.rd8(a + 0x20);
		// Retail's walk (0x248600, 0x248f98) skips slots with a negative +0x20 and stops at the first 0xff.
		walked &= slot != 0xff;
		if hdr != 0 && slot < 0xfd {
			if out.scene || mode == VENDOR_MODE {
				if let Some(b) = scene_seq(m, a, class) {
					out.scene_seqs.push((class, m.rd8(a + 0x53), b));
				}
			}
			if class == 0 {
				// A scene spawns its own class-0 Ratchet and hides the placed one (+0x34 bit 0): the drawn one wins.
				let shown = m.rd8(a + 0x31) != 0;
				// The hero's afterimage copies (FUN_00276fc0, the only writer of flags 0x80a: a copy of the hero class with +0x34 = 0x80a and
				// a translucent alpha +0x23, one per trail entry, 2 / 4 / 6 fields behind in Thruster-Pack flight, state 0x10) are class-0
				// mobys of their own and sit after the hero in the table: taking them for the hero drew Ratchet (pose, joints, position)
				// up to 6 fields behind the true place while Clank and the packs stayed there.
				let afterimage = m.rd16(a + 0x34) == 0x80a && a != m.rd32(0x1413d0) & 0x01ff_ffff;
				if !afterimage && !(out.hero_shown && !shown) {
					out.hero = Some((m.rdv3(a + 0x10), m.rdf(a + 0x48), Clock::read(m, a)));
					out.hero_hidden = m.rd16(a + 0x34) & 1 != 0;
					out.parts.hero_fx = joint_records(m, a);
					hero_addr = a;
					(out.parts.strafe, out.parts.pose_ops, out.parts.follow) = mod_poses(m, a, out.scene);
					out.parts.layers = layer_list(m, a);
					out.parts.from_frame = snapshot_frame(m, a);
					out.hero_tilt = [m.rdf(a + 0x40), m.rdf(a + 0x44)];
					out.hero_shown = shown;
				}
			} else {
				let base = f32::from_bits(m.rd32((hdr & 0x01ff_ffff) + 0x24));
				let shown = m.rd8(a + 0x31) != 0;
				let cut = view.as_ref().filter(|_| !shown && walked && (slot as i8) >= 0).map(|v| v.culls(m, a));
				let side_only = cut.and_then(|(culls, side)| culls.side_only().then_some(side));
				let (rot, mirror) = basis_pose(m, a, true); let flags = m.rd16(a + 0x34); // +0x34 read once (ovr, additive)
				out.mobys.push(Moby {
					addr: a,
					class,
					pos: m.rdv3(a + 0x10),
					rot,
					mirror,
					scale: if base != 0.0 { m.rdf(a + 0x2c) / base } else { 1.0 },
					shown,
					side_only,
					view_culled: cut.map_or(Culled::No, |(culls, _)| culls.why()),
					#[cfg(test)]
					culls: view.as_ref().map(|v| v.culls(m, a).0),
					#[cfg(test)]
					raw: (m.rd16(a + 0x34), m.rd16(a + 0x32) as i16, m.rd32(if matches!(level, 0 | 5 | 14 | 16) { va(0x15ffb0) } else { va(0x15ff70) }) as i32),
					rgb: [m.rd8(a + 0x3c), m.rd8(a + 0x3d), m.rd8(a + 0x3e)],
					light: m.rd32(a + 0x38),
					clock: Clock::read(m, a),
					fx: joint_records(m, a),
					alpha: m.rd8(a + 0x23),
					draw: m.rd16(a + 0x32) as i16,
					ovr: (flags & 0x10 != 0).then(|| m.rd32(a + 0x90)), additive: is_additive(flags),
				});
			}
		}
		a += 0x100;
	}
	// The gadget cinematic after a purchase runs in the vendor's mode 5 and plays its actors (Ratchet and the bought
	// gadget's own moby) through the same chunk player as a scene: it is a scene while a moby plays a scene sequence.
	if mode == VENDOR_MODE && !out.scene_seqs.is_empty() {
		out.scene = true;
	}
	// The shop's purchase prompt (`ShopSnap::prompt`): a slot-block moby outside the table, always drawn while up.
	if let Some(a) = out.shop.as_ref().and_then(|sh| sh.prompt).filter(|&a| m.rd32(a + 0x24) != 0) {
		let hdr = m.rd32(a + 0x24) & 0x01ff_ffff;
		let base = f32::from_bits(m.rd32(hdr + 0x24));
		let (rot, mirror) = basis_pose(m, a, true); let flags = m.rd16(a + 0x34);
		out.mobys.push(Moby {
			addr: a,
			class: m.rd16(a + 0xa6) as u32,
			pos: m.rdv3(a + 0x10),
			rot,
			mirror,
			scale: if base != 0.0 { m.rdf(a + 0x2c) / base } else { 1.0 },
			shown: true,
			side_only: None,
			view_culled: Culled::No,
			#[cfg(test)]
			culls: None,
			#[cfg(test)]
			raw: (m.rd16(a + 0x34), m.rd16(a + 0x32) as i16, 0),
			rgb: [m.rd8(a + 0x3c), m.rd8(a + 0x3d), m.rd8(a + 0x3e)],
			light: m.rd32(a + 0x38),
			clock: Clock::read(m, a),
			fx: joint_records(m, a),
			alpha: m.rd8(a + 0x23),
			draw: m.rd16(a + 0x32) as i16,
			ovr: (flags & 0x10 != 0).then(|| m.rd32(a + 0x90)), additive: is_additive(flags),
		});
	}
	// In a scene with actors the held item belongs to the hidden placed hero, not to the scene's Ratchet.
	if out.scene_seqs.is_empty() && hp >= s && hp < e && (hp - s) % 0x100 == 0 && m.rd32(hp + 0x24) != 0 && m.rd8(hp + 0x20) < 0xfd {
		out.held = Some((hp, m.rd16(hp + 0xa6) as u32, Clock::read(m, hp)));
	}
	if let Some((hp, crate::mobies::WRENCH_CLASS, clock)) = out.held {
		out.parts.wrench = Some(HeldItem { addr: hp, clock, attached: m.rd16(hp + 0x34) & 4 != 0 && m.rd8(0x1403fa) == 0 });
	}
	let back = |p: u32| Some(m.rd32(p) & 0x01ff_ffff).filter(|&b| b >= s && b < e && (b - s) % 0x100 == 0 && m.rd32(b + 0x24) != 0 && m.rd8(b + 0x20) < 0xfd);
	out.parts.d0 = back(0x1404d0).map(|b| Clock::read(m, b));
	if let Some(b) = back(0x1404d4) {
		out.parts.d4 = Some(Clock::read(m, b));
		out.parts.clank_fx = joint_records(m, b);
	}
	let in_hand = out.parts.wrench.filter(|w| w.attached).map(|w| w.addr);
	out.parts.loose = out
		.mobys
		.iter()
		.filter(|mb| mb.class == crate::mobies::WRENCH_CLASS && Some(mb.addr) != in_hand && (m.rd8(mb.addr + 0x20) as i8) >= 0)
		.map(|mb| loose_wrench(m, mb))
		.collect();
	// Modern camera: Clank, the back object and the bomb in the glove follow the drawn yaw (strafe.rs `turn_parts`, draw only).
	if hero_addr != 0 && modcam::enabled() {
		strafe::turn_parts(m, hero_addr, hp, &mut out.mobys, out.fx.as_mut());
	}
	out.shadows = Some(shadow_snap(m, level));
	// [PC] Water Enhanced wakes (viewer/waterfx.rs `Wakes`): where the drawn mobys stand this field.
	if let Some(fx) = out.fx.as_mut() {
		// The bounding sphere is moby +0x00..0x0c, x1024 (decomp/notes MOBY-FADE): its radius at +0x0c.
		fx.movers = out.mobys.iter().filter(|mb| mb.shown).map(|mb| (mb.addr, mb.pos, m.rdf(mb.addr + 0xc) / 1024.0)).collect();
	}
	out
}

/// One loose class-71 wrench moby (thrown, or an afterimage) as the hero marker draws it: pose, scale, alpha and its own light bytes.
fn loose_wrench(m: &ee_oracle::Machine, mb: &Moby) -> LooseWrench {
	let base = f32::from_bits(m.rd32((m.rd32(mb.addr + 0x24) & 0x01ff_ffff) + 0x24));
	LooseWrench {
		addr: mb.addr,
		pos: mb.pos,
		rows: [m.rdv3(mb.addr + 0xc0), m.rdv3(mb.addr + 0xd0), m.rdv3(mb.addr + 0xe0)],
		scale: if base != 0.0 { m.rdf(mb.addr + 0x2c) / base } else { 1.0 },
		alpha: m.rd8(mb.addr + 0x23),
		light: Some((mb.rgb, mb.light)),
		clock: mb.clock,
		shown: mb.shown,
		side_only: mb.side_only,
	}
}

/// The PC strafe pose and the render-only poses the mods asked for on the hero moby `a` (omni pose ops and follow poses; both empty without a
/// posable hero: class 0 with 111 joints, or in a scene).
fn mod_poses(m: &ee_oracle::Machine, a: u32, scene: bool) -> (strafe::Draw, Vec<crate::anim::JointFx>, Vec<crate::omni::FollowRec>) {
	let posable = m.rd16(a + 0xa6) == 0 && m.rd8((m.rd32(a + 0x24) & 0x01ff_ffff) + 8) == 111;
	(strafe::current(), crate::omni::pose_ops_for(a, posable, scene), crate::omni::follow_for(posable, scene))
}

#[cfg(test)]
mod live_mode_tests {
	use super::*;

	/// Leaving the pause: the pad of the next field sees the game's NEWEST mode (0), not the delayed shown snapshot's (3), so R2 is
	/// the aim from the first gameplay field and retail gets no crouch from it (`Retail::pad_gameplay`, `prompts::route_r2`).
	#[test]
	fn the_pad_sees_the_newest_mode_not_the_delayed_one() {
		let snap = |mode| Snap { mode, form: 1, ..Default::default() };
		assert_eq!(live_pad(), (0, 0));
		// In the pause menu (mode 3): `native_delay` queues 2 fields, the shown snapshot lags, the live mode is 3 at once.
		for _ in 0..4 {
			let shown = native_delay_with(snap(3), None);
			assert_eq!(live_pad(), (3, 1), "the newest field's mode and body form at once");
			assert!(shown.mode == 3 || shown.hold, "shown: the held picture or the pause menu");
		}
		// The game left the pause at this field: the shown snapshot may still be the pause menu's (2 fields queued), the live mode
		// is 0 already.
		let shown = native_delay_with(snap(0), None);
		assert_eq!(live_pad().0, 0);
		assert_eq!(shown.mode, 3, "the delayed picture still shows the pause menu: the old gameplay() would still have said no");
	}
}
