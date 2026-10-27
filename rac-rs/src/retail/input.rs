//! Keyboard, mouse and pad into the game: key taps, the mouse in the menus, the scripted pad and key taps of tests,
//! the skip rule of a playing movie and the keyboard-as-DualShock mapping.

use super::*;

/// The fewest fields a key tap is held for (`Retail::press_key`): 60 ms, a quick tap on a pad [I]. The game's pad update
/// (Eudora FUN_00259330) sees a press in one field (new presses = held & !previous held), so one would do for menus.
pub(super) const TAP_FIELDS: u8 = 3;

impl MouseTaps {
	pub(super) fn press(&mut self, b: usize) {
		if let Some(n) = self.0.get_mut(b) {
			*n = TAP_FIELDS;
		}
	}

	/// One field sampled.
	pub(super) fn tick(&mut self) {
		self.0 = self.0.map(|n| n.saturating_sub(1));
	}
}

impl Retail {
	/// A key went down (not an auto-repeat). The viewer samples the keyboard once per frame, and a frame can run several
	/// fields or take ~200 ms (the pause menu's GS capture paces the interpreter at ~35 ms a field), so a tap pressed and
	/// released between two samples never reached the game: the menu ignored most presses (2026-10-11). A tap now counts
	/// as held for at least `TAP_FIELDS` fields.
	pub fn press_key(&mut self, k: winit::keyboard::KeyCode) {
		self.taps.insert(k, TAP_FIELDS);
	}

	/// The left (0, Circle: fire) or middle (1, Triangle: Quick Select) mouse button went down; see `press_key`.
	pub fn press_mouse(&mut self, b: usize) {
		self.mouse_taps.press(b);
	}

	/// One field from the keyboard: the keys held now (`held`, `circle` / `triangle` for the left / middle mouse buttons)
	/// plus the taps still owed fields (`press_key`); then `tick`.
	pub fn tick_keys(&mut self, held: impl Fn(winit::keyboard::KeyCode) -> bool, circle: bool, triangle: bool) {
		// A mod took the keyboard (omni/input.rs `take_input`): the keys, held or tapped, press nothing; the gamepad and the mouse still do.
		let kb_taken = crate::omni::input::keyboard_taken();
		let held = crate::omni::input::gated(kb_taken, held);
		let (m, parked) = (self.snap.mode, fmv_bridge::waiting());
		for t in self.keytap.iter_mut().filter(|t| !t.fired) {
			let on = match t.when.as_str() {
				"abs" => true,
				"movie" => parked,
				w if w.starts_with("movie") => parked && w[5..].parse() == Ok(self.movies),
				"scene" => m == 2 && !parked,
				"ship" => m == 6 && !parked,
				_ => false,
			};
			t.seen = if on { t.seen + 1 } else { 0 };
			if on && t.seen >= t.n {
				t.fired = true;
				eprintln!("retail: keytap {:?} at {}+{} (field {})", t.key, t.when, t.n, self.fields);
				if t.key == winit::keyboard::KeyCode::F24 {
					// "RMB": a right click (mouse_menu::State::back).
					self.mm.back();
					continue;
				}
				self.taps.insert(t.key, TAP_FIELDS);
			}
		}
		let no_taps = HashMap::new();
		let taps = if kb_taken { &no_taps } else { &self.taps };
		// A menu is open: the left button clicks it (Circle = fire is off), see `mouse_menu`, and the keyboard works it the modern
		// way with the buttons of the screen on show (`menu_keys`: arrows / WASD, Enter / Space, Esc / Backspace, Q / E, Tab).
		let menu = self.snap.menu.as_ref().filter(|_| !self.tb.as_ref().is_some_and(|t| t.active()));
		// The screen whose keys apply: a page menu's, else a mode-4 dialog's, else the vendor shop's (no page menus).
		let keys_screen = menu.map(menu_keys::screen).or(self.snap.dialog).or_else(|| self.snap.shop.as_ref().map(|_| menu_keys::VENDOR));
		let down = |k| held(k) || taps.contains_key(&k);
		self.mk.begin(keys_screen.is_some(), &down);
		let mk = &self.mk;
		let live = |k| down(k) && mk.live(k);
		let middle = triangle || self.mouse_taps.0[1] > 0;
		let (mut b, mut s) = match keys_screen {
			Some(sc) => {
				let (b, s) = menu_keys::menu_pad(&sc, &live);
				// A dialog or the vendor (no page menu, whose clicks `mouse_menu` takes): a left click is the screen's confirm, as
				// Enter. Only a click made there (the tap latch, not a button held since play, where it fired).
				let click = menu.is_none() && self.mouse_taps.0[0] > 0;
				let extra = (if middle { mouse_menu::TRIANGLE } else { 0 }) | (if click { sc.confirm } else { 0 });
				(b | extra, s)
			}
			// A ship fight (shipmouse.rs): the left mouse button fires with Cross, the ship's fire button; elsewhere it is Circle.
			None => {
				let (as_circle, extra) = shipmouse::left_button(self.snap.ship_fight, circle || self.mouse_taps.0[0] > 0);
				let (b, s) = pad_from_keys(live, as_circle, middle);
				(b | extra, s)
			}
		};
		menu_keys::log(self.fields, menu, keys_screen.filter(|_| menu.is_none()), b);
		// A gamepad adds its buttons and sticks and, when it moved, becomes the device of the button prompts (prompts.rs).
		self.kb_rs = [s[0], s[1]].map(|v| (v as i16 - 0x80) as i8);
		prompts::poll_pad(&mut b, &mut s);
		let (mode, front) = (self.snap.mode, self.snap.front);
		// Esc / Space skip a scene or ship flight through retail's own skip test (`skip_feed`); the gamepad's Start does the
		// same, like Space on the keyboard.
		let skip_key = [winit::keyboard::KeyCode::Escape, winit::keyboard::KeyCode::Space].iter().any(|&k| held(k) || taps.contains_key(&k))
			|| (prompts::pad_buttons() & START) != 0;
		if !front {
			b |= skip_feed(mode, self.snap.ship, skip_key);
		}
		if let Some(tb) = self.tb.as_mut().filter(|t| t.active()) {
			// The boot to the title (title_boot.rs): the game is steered by pulses, the player's keys are ignored.
			b = tb.step(self.snap.menu.as_ref(), mode, front);
			s = [0x80; 4];
		} else {
			b |= self.mm.step(self.snap.menu.as_ref(), b);
			if let Some(tb) = self.tb.as_mut() {
				b |= tb.pilot(self.snap.menu.as_ref(), mode, front, &mut self.mm);
			}
		}
		// The Controls page waits for a key or button (binds.rs): the keys that went down this field (a tap counts TAP_FIELDS at
		// its first poll, so the Confirm that started the capture never binds) and the controller's new buttons go to it, and
		// the game sees a neutral pad meanwhile.
		if binds::capturing() {
			let fresh: Vec<winit::keyboard::KeyCode> = taps.iter().filter(|(_, n)| **n == TAP_FIELDS).map(|(k, _)| *k).collect();
			binds::capture_poll(&fresh, prompts::phys_fresh());
			self.cap_hold = (fresh, prompts::phys_now());
			(b, s) = (0, [0x80; 4]);
		} else if !self.cap_hold.0.is_empty() || self.cap_hold.1 != 0 {
			// The key or button that ended the capture is still down: it is not a press on the menu (it may be Confirm or Back now).
			self.cap_hold.0.retain(|k| down(*k));
			self.cap_hold.1 &= prompts::phys_now();
			if !self.cap_hold.0.is_empty() || self.cap_hold.1 != 0 {
				(b, s) = (0, [0x80; 4]);
			}
		}
		// [PC] DEV ONLY (RAC_DEV_TOOLS=1, devtools.rs): the free camera hands the game a neutral pad; the stick clicks that turned
		// it off stay away from the game until released. Off: no change.
		if devtools::free_on() {
			(b, s) = (0, [0x80; 4]);
		} else {
			b &= !devtools::pad_mask();
		}
		self.taps.retain(|_, n| {
			*n -= 1;
			*n > 0
		});
		self.mouse_taps.tick();
		if crate::loadlog::on() && b != self.pad_last {
			crate::loadlog::say(|| format!("pad field {} {b:#x}", self.fields));
		}
		self.pad_last = b;
		self.tick(b, s);
	}

	/// A page menu is on screen (pause menu, Quick Select, map): the mouse works it and the left button does not fire.
	pub fn mouse_menu_active(&self) -> bool {
		self.snap.menu.is_some()
	}

	/// The cursor is at (fx, fy), fractions of the window's client area: the overlay pixel under it.
	pub fn mouse_cursor(&mut self, fx: f32, fy: f32) {
		let (x, y) = self.ps2_point(fx, fy);
		self.mm.cursor(x, y);
	}

	/// The cursor (fractions of the client area) in the overlay's pixels (`mouse_menu::SCREEN`).
	fn ps2_point(&self, fx: f32, fy: f32) -> (f32, f32) {
		let full = self.gsfull_view().map(|f| if f.wide { crate::gsfull::wide::ASPECT } else { 4.0 / 3.0 });
		mouse_menu::to_ps2(fx, fy, self.aspect, full)
	}

	/// The cursor over the game's 4:3 picture, 0..1 across and down (outside it below 0 / above 1): the mods' `omni.pointer`.
	pub fn picture_point(&self, fx: f32, fy: f32) -> [f32; 2] {
		let (x, y) = self.ps2_point(fx, fy);
		[x / mouse_menu::SCREEN.0, y / mouse_menu::SCREEN.1]
	}

	/// Left click / right click / wheel notches (positive = up) over a menu.
	pub fn mouse_click(&mut self) {
		self.mm.click();
	}

	pub fn mouse_back(&mut self) {
		self.mm.back();
	}

	pub fn mouse_wheel(&mut self, y: f32) {
		self.mm.wheel(y);
	}
}

/// `from-to:buttons:lx,ly[:rx,ry]` (hex, `;` between entries): the optional fourth part is the right stick (the retail camera).
pub(super) fn pad_script() -> Vec<(u32, u32, u16, [u8; 2], Option<[u8; 2]>)> {
	let v = std::env::var("RAC_RETAIL_PAD").unwrap_or_default();
	let hex = |t: &str| u32::from_str_radix(t.trim(), 16).ok();
	v.split(';')
		.filter_map(|e| {
			let mut f = e.split(':');
			let (r, b, st) = (f.next()?, f.next()?, f.next()?);
			let (a, z) = r.split_once('-')?;
			let (x, y) = st.split_once(',')?;
			let rs = f.next().and_then(|t| t.split_once(',')).and_then(|(x, y)| Some([hex(x)? as u8, hex(y)? as u8]));
			Some((a.trim().parse().ok()?, z.trim().parse().ok()?, hex(b)? as u16, [hex(x)? as u8, hex(y)? as u8], rs))
		})
		.collect()
}

/// One scripted key tap of RAC_RETAIL_KEYTAP: `KEY@WHEN+N;...`, KEY a winit name (Space, Escape, Enter; for menus Backspace, Tab,
/// Up, Down, Left, Right, Q, E, W, S, T), WHEN `abs` (always),
/// `movie` (a movie is playing; `movieN`: the Nth movie of the run), `scene` (game mode 2) or `ship` (mode 6); the key is tapped (`press_key`) once WHEN has held
/// for N consecutive polls of `tick_keys` (one per 20 ms field slot, also while a movie parks the game).
pub(super) struct KeyTap {
	pub(super) key: winit::keyboard::KeyCode,
	when: String,
	pub(super) n: u32,
	pub(super) seen: u32,
	fired: bool,
}

pub(super) fn keytap_script() -> Vec<KeyTap> {
	use winit::keyboard::KeyCode as K;
	let v = std::env::var("RAC_RETAIL_KEYTAP").unwrap_or_default();
	v.split(';')
		.filter_map(|e| {
			let (k, w) = e.trim().split_once('@')?;
			let (when, n) = w.split_once('+')?;
			let key = match k {
				"Space" => K::Space,
				"Escape" => K::Escape,
				"Enter" => K::Enter,
				// Menu keys (menu_keys.rs) for scripted menu runs.
				"Backspace" => K::Backspace,
				"Tab" => K::Tab,
				"Up" => K::ArrowUp,
				"Down" => K::ArrowDown,
				"Left" => K::ArrowLeft,
				"Right" => K::ArrowRight,
				"Q" => K::KeyQ,
				"E" => K::KeyE,
				"W" => K::KeyW,
				"S" => K::KeyS,
				"T" => K::KeyT,
				// A right mouse click (tests of the menus' mouse back).
				"RMB" => K::F24,
				// Any key the Controls page can bind, by its options-file name (KeyK, Digit5, ...): remap tests.
				_ => binds::key_from_name(k)?,
			};
			Some(KeyTap { key, when: when.to_string(), n: n.parse().ok()?, seen: 0, fired: false })
		})
		.collect()
}

/// The PS2 Start bit (the retail pad bits): the skip press on a gamepad, like Space on the keyboard.
const START: u16 = 0x800;

/// One poll of the skip test of a playing movie: (the rule ends it now, the pad to remember for the next poll). `pad` is the
/// gamepad's own buttons (`prompts::pad_buttons`): its Start is the skip press, like Space on the keyboard.
pub(super) fn movie_poll(rule: fmv_bridge::Skip, prev: u16, held: impl Fn(winit::keyboard::KeyCode) -> bool, pad: u16) -> (bool, u16) {
	let skip_key = held(winit::keyboard::KeyCode::Escape) || held(winit::keyboard::KeyCode::Space);
	// The gamepad's Start is the skip press on a gamepad, like Space on the keyboard.
	let pad_start = pad & START != 0;
	let mut now = pad_from_keys(held, false, false).0 | pad;
	// Esc and Space are the player's skip keys (user request 2026-10-13), and so is the gamepad's Start: they feed the pad the
	// player's own skip test wants (`Skip::feed`), so retail's rule decides and a button held from before the movie still
	// counts for nothing.
	if skip_key || pad_start {
		now |= rule.feed();
	}
	// The boot logo movies have no skip in retail (`Skip::Never`): the port lets the player's skip press end them, so the
	// intro is not in the way (the Skip intro row cuts them before they play, when the player asked for that).
	if rule == fmv_bridge::Skip::Never && (skip_key || pad_start) {
		now |= START;
	}
	let hit = rule.pressed(prev, now) || (rule == fmv_bridge::Skip::Never && (now & !prev & START) != 0);
	(hit, now)
}

/// The pad bits Esc / Space add while a scene or a ship flight plays, so retail's own skip test sees its press: the
/// in-engine scene controller (game mode 2; Veldin FUN_00298938, twin per level, Novalis FUN_002ac3b0) skips on a new
/// Start, with L1 + R1 + L2 + R2 held as well in a normal run past Veldin (the same rule as the movie player); the ship
/// flight (mode 6, substates 0 takeoff and 8 landing; Novalis FUN_002a39b0) skips on a new Triangle or Cross.
pub fn skip_feed(mode: u32, ship: u32, skip_key: bool) -> u16 {
	if !skip_key {
		return 0;
	}
	match mode {
		2 => 0x80f,
		6 if matches!(ship, 0 | 8) => 0x40,
		_ => 0,
	}
}

/// rac-rs's keyboard and mouse as a DualShock report: (buttons pressed, [rx, ry, lx, ly]). By default WASD = left stick, Q/E Z/X =
/// right stick, Space = Cross, F = Square, G or the left mouse button (`circle`) = Circle (fire the weapon), T or the middle
/// mouse button (`triangle`) = Triangle (Quick Select), Ctrl = R1, C = L1, V = L2, B = R2, Esc/Enter = Start, M = Select, Left
/// Shift = L3, R = R3, arrows = d-pad; Pause -> Options -> Controls remaps them (`binds::kb`, the defaults with the extension off).
pub fn pad_from_keys(held: impl Fn(winit::keyboard::KeyCode) -> bool, circle: bool, triangle: bool) -> (u16, [u8; 4]) {
	binds::kb().pad(held, circle, triangle)
}
