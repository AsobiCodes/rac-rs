//! Keyboard, mouse and pad into the game: key taps, the mouse in the menus, the scripted pad and key taps of tests,
//! the skip rule of a playing movie and the keyboard-as-DualShock mapping.

use super::*;
use rac_engine::input::kbm::{self, Ctx, Input, Mouse, Wheel};

/// The fewest fields a key tap is held for (`Retail::press_key`): 60 ms, a quick tap on a pad [I]. The game's pad update
/// (Eudora FUN_00259330) sees a press in one field (new presses = held & !previous held), so one would do for menus.
pub(super) const TAP_FIELDS: u8 = 3;

impl MouseTaps {
	/// The input of each `press_mouse` index: left, middle (Circle / Triangle as always), then right, the side buttons and the wheel notches.
	pub(super) const INPUTS: [Input; 7] = [Input::Mouse(Mouse::Left), Input::Mouse(Mouse::Middle), Input::Mouse(Mouse::Right), Input::Mouse(Mouse::Back), Input::Mouse(Mouse::Forward), Input::Wheel(Wheel::Up), Input::Wheel(Wheel::Down)];

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

/// The `press_mouse` index of a wheel notch (`MouseTaps::INPUTS`: 5 = up, 6 = down; it lands in `extra_taps[3]` / `[4]`).
fn wheel_index(up: bool) -> usize {
	if up { 5 } else { 6 }
}

/// The keys whose first poll this is and that the Controls page may read as Tab / Delete: none in the field a capture was running
/// (the answer can be Delete or Tab) and none still held from a capture's answer.
fn page_notes(taps: &HashMap<winit::keyboard::KeyCode, u8>, was_capturing: bool, hold: &[winit::keyboard::KeyCode]) -> Vec<winit::keyboard::KeyCode> {
	if was_capturing {
		return Vec::new();
	}
	taps.iter().filter(|(k, n)| **n == TAP_FIELDS && !hold.contains(k)).map(|(k, _)| *k).collect()
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;

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
		match b {
			0 | 1 => self.mouse_taps.press(b),
			2..=6 => self.extra_taps[b - 2] = TAP_FIELDS,
			_ => {}
		}
	}

	/// A mouse button other than left / middle went down or up (the held state; the tap latch is `press_mouse`).
	pub fn set_mouse_held(&mut self, m: Mouse, down: bool) {
		let bit = match m {
			Mouse::Right => 1,
			Mouse::Back => 2,
			Mouse::Forward => 4,
			_ => return,
		};
		self.mouse_held = if down { self.mouse_held | bit } else { self.mouse_held & !bit };
	}

	/// A wheel notch: a tap of the wheel input (it presses what it is bound to for `TAP_FIELDS` fields).
	pub fn press_wheel(&mut self, up: bool) {
		self.press_mouse(wheel_index(up));
	}

	/// RAC_RETAIL_KEYTAP: the scripted taps due this field become key taps (`taps`), a right click, or a modern screen's keys.
	fn fire_keytaps(&mut self) {
		let (m, parked) = (self.snap.mode, fmv_bridge::waiting());
		let (front, title) = (self.front_ram().is_some(), crate::modern_menu::title_host::up());
		for t in self.keytap.iter_mut().filter(|t| !t.fired) {
			let on = match t.when.as_str() {
				"abs" => true,
				"movie" => parked,
				w if w.starts_with("movie") => parked && w[5..].parse() == Ok(self.movies),
				"scene" => m == 2 && !parked,
				"ship" => m == 6 && !parked,
				"front" => front && !parked,
				"title" => title && !parked,
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
				// A modern screen is up (modern_menu/host.rs): the tap is its window key, press and release, as a real key reaches it
				// through `ui_events.rs` (the retail key layer under it gets a neutral pad), so scripted runs can drive the modern pages.
				if modern_screen_up() && crate::modern_menu::host::key(t.key, true, false) {
					crate::modern_menu::host::key(t.key, false, false);
					continue;
				}
				self.taps.insert(t.key, TAP_FIELDS);
			}
		}
	}

	/// Play: the held inputs through the live bindings (kbm). A ship fight turns the Ship layer on: there the left button is Cross
	/// (`kbm_actions` ship_fire) and no longer Circle (shipmouse.rs, v0.5.0). `live`: key down now; `middle`: the middle button.
	fn play_pad(&self, live: &impl Fn(winit::keyboard::KeyCode) -> bool, circle: bool, middle: bool) -> (u16, [u8; 4]) {
		let (mt, et, mh) = (self.mouse_taps.0, self.extra_taps, self.mouse_held);
		let lmb = circle || mt[0] > 0;
		let down_in = |i: Input| match i {
			Input::Key(k) => live(k),
			Input::Mouse(Mouse::Left) => lmb,
			Input::Mouse(Mouse::Middle) => middle,
			Input::Mouse(Mouse::Right) => mh & 1 != 0 || et[0] > 0,
			Input::Mouse(Mouse::Back) => mh & 2 != 0 || et[1] > 0,
			Input::Mouse(Mouse::Forward) => mh & 4 != 0 || et[2] > 0,
			Input::Wheel(Wheel::Up) => et[3] > 0,
			Input::Wheel(Wheel::Down) => et[4] > 0,
		};
		let ctx = Ctx { contexts: if self.snap.ship_fight { 1 << crate::kbm_actions::SHIP } else { 0 }, menu: None };
		let o = kbm_glue::eval(&down_in, &ctx);
		(o.buttons, o.sticks)
	}

	/// One poll of the Controls capture: the keys and mouse buttons that went down this field (the tap latch's first poll) and the
	/// controller's new buttons are the answer (`kbm::capture_poll`); `cap_hold` remembers what is still down.
	fn capture_step(taps: &HashMap<winit::keyboard::KeyCode, u8>, mouse: &MouseTaps, extra: &[u8; 5], cap_hold: &mut (Vec<winit::keyboard::KeyCode>, u32)) {
		let fresh: Vec<winit::keyboard::KeyCode> = taps.iter().filter(|(_, n)| **n == TAP_FIELDS).map(|(k, _)| *k).collect();
		let mut fresh_in: Vec<Input> = fresh.iter().map(|&k| Input::Key(k)).collect();
		fresh_in.extend(MouseTaps::INPUTS.iter().enumerate().filter(|(i, _)| (if *i < 2 { mouse.0[*i] } else { extra[*i - 2] }) == TAP_FIELDS).map(|(_, m)| *m));
		kbm::capture_poll(&fresh_in, prompts::phys_fresh());
		*cap_hold = (fresh, prompts::phys_now());
	}

	/// The controller reads the STANDARD layout in this context (a menu screen, a dialog, the shop, the front end, a scene, a ship flight's
	/// takeoff / landing: retail's own skip tests), the player's remap elsewhere. It uses the NEWEST mode too (`snapshot::live_pad`, not
	/// only the shown snapshot that lags up to 2 fields).
	fn pad_menu_context(&self, keys_screen: bool) -> bool {
		let menu_mode = |m: u32| matches!(m, 2..=5) || (m == 6 && matches!(self.snap.ship, 0 | 8));
		// The newest state decides: the shown snapshot lags up to 2 fields, and a press in the fields right after a menu closed must act in
		// the player's layout at once (TB2-N5). Mode 6 reads the shown ship substate (the newest snapshot carries none).
		keys_screen || self.snap.front || menu_mode(snapshot::live_pad().0)
	}

	/// One field from the keyboard: the keys held now (`held`, `circle` / `triangle` for the left / middle mouse buttons)
	/// plus the taps still owed fields (`press_key`); then `tick`.
	pub fn tick_keys(&mut self, held: impl Fn(winit::keyboard::KeyCode) -> bool, circle: bool, triangle: bool) {
		// A mod took the keyboard (omni/input.rs `take_input`): the keys, held or tapped, press nothing; the gamepad and the mouse still do.
		let kb_taken = crate::omni::input::keyboard_taken();
		let held = crate::omni::input::gated(kb_taken, held);
		self.fire_keytaps();
		// RAC_RETAIL_MOUSE: the scripted pointer of a menu run (mouse_script.rs).
		mouse_script::poll(self);
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
				let click = menu.is_none() && self.snap.vend.is_none() && self.mouse_taps.0[0] > 0;
				let extra = (if middle { mouse_menu::TRIANGLE } else { 0 }) | (if click { sc.confirm } else { 0 });
				(b | extra, s)
			}
			None => self.play_pad(&live, circle, middle),
		};
		menu_keys::log(self.fields, menu, keys_screen.filter(|_| menu.is_none()), b);
		// A gamepad adds its buttons and sticks and, when it moved, becomes the device of the button prompts (prompts.rs).
		self.kb_rs = [s[0], s[1]].map(|v| (v as i16 - 0x80) as i8);
		// The controller follows the player's remap in gameplay and reads the standard layout in the Menu context (design V051_PAD_DESIGN:
		// a menu screen, dialog or shop as the keyboard's `keys_screen`, and the front end, so a moved Start cannot lock the title).
		let kb_b = b;
		prompts::poll_pad(&mut b, &mut s, self.pad_menu_context(keys_screen.is_some()));
		// Quick Select wheel (RAC_WHEEL_MOUSE=1): Triangle held while retail's wheel is up = the mouse direction is the left stick (wheel_mouse.rs).
		wheel_mouse::step(menu.is_none() && keys_screen.is_none() && b & mouse_menu::TRIANGLE != 0, self.snap.wheel_up, &mut s);
		let (mode, front) = (self.snap.mode, self.snap.front);
		// Esc / Space skip a scene or ship flight through retail's own skip test (`skip_feed`); so does a plain Start (keyboard or
		// controller: `b` holds both) while the Start skips cutscenes row is On (pcskip.rs; GitHub #34 / #23).
		let skip_key = [winit::keyboard::KeyCode::Escape, winit::keyboard::KeyCode::Space].iter().any(|&k| held(k) || taps.contains_key(&k)) || pcskip::start_pressed(kb_b | prompts::std_buttons());
		if !front {
			b |= skip_feed(mode, self.snap.ship, skip_key);
		}
		let mut instant = None;
		if let Some(tb) = self.tb.as_mut().filter(|t| t.active()) {
			// The boot to the title (title_boot.rs): the game is steered by pulses, the player's keys are ignored.
			b = tb.step(self.snap.menu.as_ref(), mode, front);
			s = [0x80; 4];
		} else {
			// The left stick counts as a menu button too (WASD on the Galactic Map, a controller's stick): `b` has no stick bits.
			// A modern page that owns the pad (pause, title, Galactic Map) gets it through its own requests: its buttons stay away from
			// the mouse state, whose "keys are down" step would cancel the page's request in flight (`kbd_bits`).
			let kbd = mouse_menu::kbd_bits(crate::modern_menu::host::pad_owned(), b, s);
			b |= self.mm.step(self.snap.menu.as_ref().or(self.snap.vend.as_ref()), kbd);
			if let Some(tb) = self.tb.as_mut() {
				b |= tb.pilot(self.snap.menu.as_ref(), mode, front, &mut self.mm);
			}
			// An instant selection (or its end) for the worker's next field (mouse_instant.rs), sent below once it is known whether the
			// game gets a neutral pad this field.
			instant = self.mm.take_cmd();
		}
		// The Controls page waits for a key or button (binds.rs): the keys that went down this field (a tap counts TAP_FIELDS at
		// its first poll, so the Confirm that started the capture never binds) and the controller's new buttons go to it, and
		// the game sees a neutral pad meanwhile.
		// The game sees a neutral pad in the three states below; the worker's instant command must not move the menu either then.
		let mut neutral = false;
		let was_capturing = binds::capturing();
		if was_capturing {
			Self::capture_step(taps, &self.mouse_taps, &self.extra_taps, &mut self.cap_hold);
			(b, s) = (0, [0x80; 4]);
			neutral = true;
		} else if !self.cap_hold.0.is_empty() || self.cap_hold.1 != 0 {
			// The key or button that ended the capture is still down: it is not a press on the menu (it may be Confirm or Back now).
			self.cap_hold.0.retain(|k| down(*k));
			self.cap_hold.1 &= prompts::phys_now();
			if !self.cap_hold.0.is_empty() || self.cap_hold.1 != 0 {
				(b, s) = (0, [0x80; 4]);
				neutral = true;
			}
		}
		// [PC] DEV ONLY (RAC_DEV_TOOLS=1, devtools.rs): the free camera hands the game a neutral pad; the stick clicks that turned
		// it off stay away from the game until released. Off: no change.
		if devtools::free_on() {
			(b, s) = (0, [0x80; 4]);
			neutral = true;
		} else {
			b &= !devtools::pad_mask();
		}
		// [PC] The Achievements row of the pause Options list (pcext/achrow.rs): the worker decided on live RAM and took the Cross out of
		// the pad; the list opens here (the pad below is neutral from the next field on, `ui_pad`).
		if pcext::achrow::take_open() {
			crate::achievements::open();
			// The modern page shows the list from the next frame on (`frame_pause`); hosted already in this one, so the HUD list does
			// not draw under the page for a frame and the pad below stays the page host's (review R22 m2).
			if crate::modern_menu::style::modern() && crate::modern_menu::pause_host::up() {
				crate::achievements::set_hosted(true);
			}
		}
		// [PC] Achievements list open (engine/achievements/screen.rs `ui_pad`): the pad scrolls / closes it, the game sees a neutral pad.
		if crate::achievements::ui_pad(b, s) {
			(b, s) = (0, [0x80; 4]);
			neutral = true;
		}
		// [PC] The modern Options page is up (modern_menu/host.rs): the game sees a neutral pad, and the button that closed the page
		// stays away from it until released (Circle / Esc must not also back out of the retail Options list behind it).
		if crate::modern_menu::host::pad_filter(b) {
			(b, s) = (0, [0x80; 4]);
			neutral = true;
		}
		// [PC] The title's fourth entry "Quit to Desktop", Classic style (title_quit.rs): Down from Options highlights it, Cross / Enter asks,
		// Circle / Enter = Yes, Triangle / Esc = No; the game sees a neutral pad while it is up. Runs every field (off the root page it only
		// clears); the Modern page keeps the same state itself (title_host.rs), so it is skipped then.
		if !crate::modern_menu::title_host::up() {
			let (yes, no) = (down(winit::keyboard::KeyCode::Enter) || down(winit::keyboard::KeyCode::NumpadEnter), down(winit::keyboard::KeyCode::Escape));
			neutral |= title_quit::classic_field(front && self.snap.dialog.is_none(), menu, &mut b, &mut s, yes, no);
		}
		crate::modern_menu::title_host::filter(&mut b, &mut s, front); // [PC] the modern title page is up: neutral pad (title_host.rs)
		// [PC] The modern shop page is up: this field's RAC_RETAIL_PAD (tests) goes in with the keys, like a real pad (`prompts::poll_pad`
		// above), so it walks the page too; `tick` ORs it in after this otherwise (R23 M1).
		let page = crate::modern_menu::vendor_host::up();
		if page {
			self.script_into(&mut b, &mut s);
		}
		self.script_in = page;
		crate::modern_menu::vendor_nav::field(&mut b, &mut s, self.snap.vend.as_ref()); // [PC] the modern shop: Up / Down / the stick walk its list, Buy all ammo is a row (vendor_nav.rs); before the queued pulses, which pass unchanged
		screen_presses(neutral, menu, self.snap.vend.as_ref(), mode == 0 && !front, &mut b, &mut instant); // [PC] the modern screens' queued presses
		if let Some(c) = instant {
			mouse_instant::send(mouse_instant::when_neutral(c, neutral));
		}
		// The Controls page reads Tab / Delete (slot 2, clear) once their first poll is over, so a Tab that opens a capture never binds itself.
		// Not in the field a capture was answered (Delete / Tab can be the answer) nor for a key still held from the answer; not while a mod
		// has the keyboard.
		for k in page_notes(taps, was_capturing, &self.cap_hold.0) {
			pcctl_kbm::note_key(k);
		}
		self.taps.retain(|_, n| {
			*n -= 1;
			*n > 0
		});
		self.mouse_taps.tick();
		self.extra_taps = self.extra_taps.map(|n| n.saturating_sub(1));
		if crate::loadlog::on() && b != self.pad_last {
			crate::loadlog::say(|| format!("pad field {} {b:#x}", self.fields));
		}
		self.pad_last = b;
		self.tick(b, s);
	}

	/// The RAC_RETAIL_PAD script of the next field ORed into the buttons `b`, its sticks over `s` (left `[2..4]`, right `[0..2]`).
	pub(super) fn script_into(&self, b: &mut u16, s: &mut [u8; 4]) {
		for &(from, to, bt, st, rs) in &self.script {
			if (from..to).contains(&self.fields) {
				*b |= bt;
				(s[2], s[3]) = (st[0], st[1]);
				if let Some(r) = rs {
					(s[0], s[1]) = (r[0], r[1]);
				}
			}
		}
	}

	/// A page menu is on screen (pause menu, Quick Select, map): the mouse works it and the left button does not fire.
	pub fn mouse_menu_active(&self) -> bool {
		self.snap.menu.is_some() || self.snap.vend.is_some()
	}

	/// The cursor is at (fx, fy), fractions of the window's client area: the overlay pixel under it.
	pub fn mouse_cursor(&mut self, fx: f32, fy: f32) {
		let (x, y) = self.ps2_point(fx, fy);
		if !crate::modern_menu::title_host::up() && title_quit::mouse_move(self.snap.menu.as_ref(), x, y) {
			return; // [PC] over the title's fourth entry (title_quit.rs): it takes the highlight, the game's hover does not steer
		}
		self.mm.cursor(x, y);
	}

	/// The cursor moved while it is hidden (a pad / keyboard player's desk bump, `cursor_vis.rs`): the place is kept for a click,
	/// nothing aims at it.
	pub fn mouse_track(&mut self, fx: f32, fy: f32) {
		let (x, y) = self.ps2_point(fx, fy);
		if !crate::modern_menu::title_host::up() {
			title_quit::mouse_track(x, y); // [PC] the place a click on the title's fourth entry answers at
		}
		self.mm.track(x, y);
	}

	/// The classic PAL Help Log and Moves screens use a square-pixel overlay.
	pub(crate) fn classic_help_aspect(&self) -> Option<f32> {
		(rac_engine::localization::hd_cards() && self.gs && self.paused() && self.front_ram().is_none() && !super::disc::usa()
			&& self.level_menu().is_some_and(|m| matches!(m.menu.id, 0x15 | 0x17)))
			.then_some(512.0 / 448.0)
	}

	/// The cursor (fractions of the client area) in the overlay's pixels (`mouse_menu::SCREEN`).
	fn ps2_point(&self, fx: f32, fy: f32) -> (f32, f32) {
		let full = self.gsfull_view().map(|f| if f.wide { crate::gsfull::wide::ASPECT } else { 4.0 / 3.0 });
		mouse_menu::to_ps2(fx, fy, self.aspect, full.or_else(|| self.classic_help_aspect()))
	}

	/// The cursor over the game's 4:3 picture, 0..1 across and down (outside it below 0 / above 1): the mods' `omni.pointer`.
	pub fn picture_point(&self, fx: f32, fy: f32) -> [f32; 2] {
		let (x, y) = self.ps2_point(fx, fy);
		[x / mouse_menu::SCREEN.0, y / mouse_menu::SCREEN.1]
	}

	/// Left click / right click / wheel notches (positive = up) over a menu.
	pub fn mouse_click(&mut self) {
		if !crate::modern_menu::title_host::up() && title_quit::mouse_click(self.snap.menu.as_ref()) {
			return; // [PC] the title's fourth entry or its question took the click (title_quit.rs)
		}
		self.mm.click();
	}

	pub fn mouse_back(&mut self) {
		if !crate::modern_menu::title_host::up() && title_quit::mouse_back() {
			return; // [PC] a right click closes the title's Quit question (title_quit.rs)
		}
		self.mm.back();
	}

	/// The left button went up (ends a Sound Options bar drag).
	pub fn mouse_release(&mut self) {
		self.mm.button_up();
	}

	/// The left button is held on a page menu (clicked there, not released yet): the Aspect ratio box's bars pass the cursor clamped
	/// into the box then (`host/handler/box_mouse.rs`), so a bar drag reaches the bar's end.
	pub fn mouse_held(&self) -> bool {
		self.mouse_menu_active() && self.mm.held()
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
/// `movie` (a movie is playing; `movieN`: the Nth movie of the run), `scene` (game mode 2), `ship` (mode 6), `front` (the front end
/// runs, no movie: Press Start and the title menus) or `title` (the modern title page is up, no movie); the key is tapped (`press_key`) once WHEN has held
/// for N consecutive polls of `tick_keys` (one per 20 ms field slot, also while a movie parks the game).
pub(super) struct KeyTap {
	pub(super) key: winit::keyboard::KeyCode,
	when: String,
	pub(super) n: u32,
	pub(super) seen: u32,
	fired: bool,
}

/// [PC] The modern screens' queued presses of this field (`tick_keys`, after the neutral-pad cases): the queued commands
/// (menu_model/cmd.rs; the shop's pulses need its settled menu `vend`) and the Map tab's second press (map_open.rs; `gameplay` = mode 0,
/// not the front end). Nothing queued or armed = `b` unchanged. In a `neutral` field nothing is pressed: a pulse queued before a neutral
/// stretch must not fire on the page when it ends (R2-3).
fn screen_presses(neutral: bool, menu: Option<&mouse_menu::MenuSnap>, vend: Option<&mouse_menu::MenuSnap>, gameplay: bool, b: &mut u16, instant: &mut Option<mouse_instant::Cmd>) {
	if neutral {
		menu_model::cmd::neutral_field();
		return;
	}
	menu_model::cmd::feed(menu.or(vend), b, instant);
	*b |= map_open::field(menu.filter(|m| !m.front).map(|m| m.id), gameplay);
}

/// A modern screen takes the window's keys now (the same test as `host/handler/ui_events.rs`).
fn modern_screen_up() -> bool {
	use crate::modern_menu::{map_host, pause_host, ship_host, title_host, vendor_host};
	rac_engine::ui::is_open() || title_host::up() || pause_host::up() || vendor_host::up() || ship_host::up() || map_host::up()
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

/// `prev` for the FIRST poll of a boot logo: every button down then counts as held from before (Space is remembered as Cross in the pad
/// state, so it would otherwise look like a new Start once the rule turns it into one).
pub(super) const FIRST_POLL: u16 = u16::MAX;

/// One poll of the skip test of a playing movie: (the rule ends it now, the pad to remember for the next poll). `pad`: the controller's
/// buttons now (PS2 bits, `prompts::pad_buttons`); `start_skips`: the Start skips cutscenes row is On (pcskip.rs).
pub(super) fn movie_poll(rule: fmv_bridge::Skip, prev: u16, held: impl Fn(winit::keyboard::KeyCode) -> bool, pad: u16, start_skips: bool) -> (bool, u16) {
	let skip_key = held(winit::keyboard::KeyCode::Escape) || held(winit::keyboard::KeyCode::Space);
	let mut now = pad_from_keys(held, false, false).0 | pad;
	// Esc and Space are the player's skip keys (user request 2026-10-13): they feed the pad the player's own skip test wants
	// (`Skip::feed`), so retail's rule decides and a button held from before the movie still counts for nothing. With the Start
	// skips cutscenes row On a plain Start (Enter, the controller's Start) feeds it too (the logos and the attract movie: no change,
	// `feed` is 0 there).
	if skip_key || (start_skips && now & pcskip::START != 0) {
		now |= rule.feed();
	}
	// The boot logo movies have no skip in retail (`Skip::Never`): the port lets the player's skip press end them, so the intro is
	// not in the way (the Skip intro row cuts them before they play, when the player asked for that). A press held from before
	// the logo counts for nothing, as everywhere. (Ported from DeSynkro's rac-rs-linux, commit 97e5c493.)
	if rule == fmv_bridge::Skip::Never && (skip_key || now & pcskip::START != 0) {
		now |= pcskip::START;
	}
	let hit = rule.pressed(prev, now) || (rule == fmv_bridge::Skip::Never && now & !prev & pcskip::START != 0);
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
	use rac_engine::input::kbm::{Ctx, Input, Mouse};
	let down = |i: Input| match i {
		Input::Key(k) => held(k),
		Input::Mouse(Mouse::Left) => circle,
		Input::Mouse(Mouse::Middle) => triangle,
		_ => false,
	};
	let o = kbm_glue::eval(&down, &Ctx::default());
	(o.buttons, o.sticks)
}
