//! What the viewer shows this frame: movie hand-off, the GS overlay and GS-full picture, the front end's state and the
//! flags that say whether retail presents a picture.

use super::*;

/// The GS-full picture last handed to the window: its field counted from 1 (`Frame::field` + 1), for RAC_SHOT_FIELD.
pub(super) static GSF_SHOWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl Retail {
	/// Gameplay on screen (game mode 0): the modern camera may capture the mouse.
	pub fn gameplay(&self) -> bool {
		self.snap.mode == 0 && !self.snap.front && !self.failed
	}

	/// The vendor screen is up (game mode 5): the world behind it stands still.
	pub fn vendor(&self) -> bool {
		self.snap.mode == 5 && !self.snap.front
	}

	pub fn take_movie(&mut self) -> Option<fmv_bridge::Request> {
		let q = self.movie_req.take()?;
		self.movies += 1;
		// The title boot runs on black: a movie in the way (the Veldin intro) is skipped at once.
		if self.tb.as_ref().is_some_and(|t| t.active()) {
			eprintln!("title: movie lba {} skipped during the boot", q.lba);
			self.end_movie();
			return None;
		}
		// The skip rule of this movie; a button down now is not a new press (retail reads newly pressed buttons).
		(self.movie_rule, self.movie_prev) = (q.skip, self.pad_last);
		Some(q)
	}

	/// The front end runs (frontbin) and the mirror draws it from this state; None outside it or with RAC_FRONT_GS.
	pub fn front_ram(&self) -> Option<&FrontRam> {
		if title_boot::gs_picture() {
			return None;
		}
		self.snap.front_ram.as_ref().filter(|_| self.snap.front)
	}

	/// The front end's menu snapshot (page, items with PS2-pixel rects, cursor).
	pub fn front_menu(&self) -> Option<&mouse_menu::MenuSnap> {
		self.snap.menu.as_ref().filter(|_| self.snap.front)
	}

	/// The title boot is running: the viewer draws plain black and mutes the sound.
	pub fn title_cover(&self) -> bool {
		self.tb.as_ref().is_some_and(|t| t.active())
	}

	/// The GS-full pipe whose picture is on screen: the one `--gsfull` runs all along, else the front end's (title_boot.rs).
	pub fn gsfull_view(&self) -> Option<Arc<crate::gsfull::pipe::GsFull>> {
		self.gsfull.clone().or_else(title_boot::late)
	}

	/// Retail's skip test for the movie playing (`take_movie` stored its rule), for every movie the game plays, in the front
	/// end and in a level alike: true when the keys now give a press the rule accepts that was not down at the previous call
	/// (`movie_poll`). Start is Esc / Enter; the combo's L1 + R1 + L2 + R2 are C, Left Ctrl, V, B (`pad_from_keys`).
	pub fn movie_pressed(&mut self, held: impl Fn(winit::keyboard::KeyCode) -> bool) -> bool {
		let taps = &self.taps;
		let (hit, now) = movie_poll(self.movie_rule, self.movie_prev, |k| held(k) || taps.contains_key(&k), prompts::pad_buttons());
		self.movie_prev = now;
		hit
	}

	/// The movie now playing is a boot logo movie (retail's rule `Skip::Never`: nothing skips it; the Skip intro row ends it).
	pub fn movie_is_logo(&self) -> bool {
		self.movie_rule == fmv_bridge::Skip::Never
	}

	/// The movie ended or was skipped: the game thread returns from the player (v0 = 0, as retail's does). The buttons
	/// held now reach the game only after a release.
	pub fn end_movie(&mut self) {
		self.movie_mask = 0xffff;
		fmv_bridge::finish();
	}

	/// A ship fight is on in the latest field (`shipmouse::in_ship_fight`).
	pub fn ship_fight(&self) -> bool {
		self.snap.ship_fight
	}

	/// The newest GS-full picture (512x448 RGBA8) that finished since the last call, if any.
	pub fn take_gsfull_frame(&mut self) -> Option<crate::gsfull::pipe::Frame> {
		let sh = self.gsfull_view()?;
		let f = sh.take()?;
		GSF_SHOWN.store(f.field + 1, std::sync::atomic::Ordering::Relaxed);
		Some(f)
	}

	/// The retail overlay of the field `tick` just took, if one finished (each is handed out once).
	/// This field's overlay was drawn into the display buffer (no-present field) and is not one uniform colour: it
	/// replaces a held movie picture.
	pub fn overlay_covers(&self) -> bool {
		self.snap.ov_disp && self.movie_tint().is_none() && self.snap.ov.is_some()
	}

	/// This field's display-buffer overlay is one colour over the whole frame (the movie-end fade: black sprite strips
	/// over the movie's last picture, GS alpha (Cs-Cd)*As+Cd [C stage 6]): the held movie picture's tint, as
	/// `fmv::present::Presenter::tint` (rgb added, a = how much of the picture shows).
	pub fn movie_tint(&self) -> Option<[f32; 4]> {
		let (_, _, px) = self.snap.ov.as_ref().filter(|_| self.snap.ov_disp)?;
		let p = *px.first()?;
		px.iter().all(|q| *q == p).then(|| p.map(|c| c as f32 / 255.0))
	}

	pub fn take_overlay(&mut self) -> Option<(u32, u32, Vec<[u8; 4]>)> {
		self.snap.ov.take()
	}

	/// Retail draws no world this field (pause menu, movie, level load: `frozen_view`).
	pub fn paused(&self) -> bool {
		self.snap.frozen
	}

	/// A menu mode (3: pause, galactic map, the post-game page) presents its own picture this field: `NO_PRESENT` is clear, the
	/// GS capture's overlay is the whole picture, so a movie's held last picture (viewer MOVIE_HOLD) must give way to it.
	pub fn menu_presents(&self) -> bool {
		self.snap.mode == 3 && !self.snap.no_present
	}

	/// The front end drawn by the GS itself (title_boot::gs_picture): a movie's held last picture gives way to the GS picture at
	/// once, as the mirror's own scene does (front_mirror.rs `fm_movie_hold`), except under the New Game intro (mode 6) fades.
	/// The front snapshot always holds (no present word there), so without this the held picture stayed up for good.
	pub fn gs_front_drops_movie(&self) -> bool {
		title_boot::gs_picture() && self.snap.front && !self.snap.front_ram.as_ref().is_some_and(|f| f.dialog.mode == 6)
	}

	/// Retail presents no new picture this field (`Snap::hold`): the viewer keeps the last one (a movie's last picture too).
	pub fn holds(&self) -> bool {
		self.snap.hold
	}
}
