//! The Options values the pause menu edits (decomp/notes/ingame_menu_leftovers.md P1-d), kept in a
//! small text file `saves/settings.txt` next to the executable (the game keeps them in RAM globals).

use std::path::PathBuf;

/// Volume range of 0x15edf0 / 0x15edec.
pub const VOLUME_MAX: u16 = 0x400;
/// Sound page: Left/Right held moves the volume by 3 per frame.
pub const VOLUME_STEP: u16 = 3;
/// Camera rotation speed table 0x161d70 (Slow, Medium, Fast), radians per tick.
pub const ROTATE_RATES: [f32; 3] = [0.017_453, 0.022_689, 0.027_925];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
	/// Sound effects volume 0x15edf0 (boot 0x400) and music volume 0x15edec (boot 0x2cc), 0..0x400.
	pub sfx: u16,
	pub music: u16,
	/// 0x15ede8 (boot 1 = stereo).
	pub stereo: bool,
	/// Camera 'Left/Right movement' 0x15ede0 and 'Up/Down movement' 0x15eddc: true = Normal (boot).
	pub cam_lr: bool,
	pub cam_ud: bool,
	/// Rotation speed 0x15ede4: 0 Slow, 1 Medium (boot), 2 Fast.
	pub cam_speed: u8,
	/// Subtitles 0x15ee40 (boot 0): cutscene subtitles only.
	pub subtitles: bool,
	/// HelpDesk Voice 0x15ee1c and Text 0x15ee1d (boot 1 each).
	pub help_voice: bool,
	pub help_text: bool,
	/// Front-end language (menu index 0..4) and video mode, kept here too.
	pub lang: u8,
	pub hz60: bool,
	/// Port extra, not in the game: Options > Camera "Camera  Classic / Modern" (camera::CamInput::modern).
	pub cam_modern: bool,
	/// PC-FPS-1 port extra: Options > Camera "Smooth motion" draws between the last two 50 Hz ticks at the display
	/// rate (viewer/smooth.rs); off = the exact 50 Hz stepping. Never read by the simulation.
	pub smooth: bool,
	/// A7 cheat toggles and A37 cheat unlocks (12 bytes each): saved with the slot, never in settings.txt.
	/// No cheat is wired in rac-rs yet, so they only travel between the save image and the next save.
	pub cheats: [u8; 12],
	pub cheat_unlocks: [u8; 12],
}

impl Default for Settings {
	fn default() -> Self {
		Settings { sfx: 0x400, music: 0x2cc, stereo: true, cam_lr: true, cam_ud: true, cam_speed: 1, subtitles: false, help_voice: true, help_text: true, lang: 0, hz60: false, cam_modern: false, smooth: true, cheats: [0; 12], cheat_unlocks: [0; 12] }
	}
}

/// One frame of a held Left (-1) or Right (+1) on a volume row.
pub fn step_volume(v: u16, dir: i32) -> u16 {
	(v as i32 + dir * VOLUME_STEP as i32).clamp(0, VOLUME_MAX as i32) as u16
}

impl Settings {
	/// The options a new game starts with: FUN_0025f1b0 restores the default image but keeps the live
	/// music and SFX volume (savegame.md); the port's own extras (language, video mode, modern camera) stay.
	pub fn new_game(&self) -> Settings {
		Settings { sfx: self.sfx, music: self.music, lang: self.lang, hz60: self.hz60, cam_modern: self.cam_modern, smooth: self.smooth, ..Settings::default() }
	}

	/// Sound effects gain 0..1.
	pub fn sfx_gain(&self) -> f32 {
		self.sfx as f32 / VOLUME_MAX as f32
	}

	/// Music gain relative to the boot default (1.0 at 0x2cc), so the default mix is unchanged.
	pub fn music_gain(&self) -> f32 {
		self.music as f32 / 0x2cc as f32
	}

	pub fn rotate_rate(&self) -> f32 {
		ROTATE_RATES[(self.cam_speed as usize).min(2)]
	}

	pub fn to_text(&self) -> String {
		let b = |v: bool| v as u8;
		format!(
			"sfx={}\nmusic={}\nstereo={}\ncam_lr={}\ncam_ud={}\ncam_speed={}\nsubtitles={}\nhelp_voice={}\nhelp_text={}\nlang={}\nhz60={}\ncam_modern={}\nsmooth={}\n",
			self.sfx, self.music, b(self.stereo), b(self.cam_lr), b(self.cam_ud), self.cam_speed, b(self.subtitles), b(self.help_voice), b(self.help_text), self.lang, b(self.hz60), b(self.cam_modern), b(self.smooth)
		)
	}

	/// Unknown keys and bad values keep their defaults.
	pub fn parse(s: &str) -> Settings {
		let mut o = Settings::default();
		for line in s.lines() {
			let Some((k, v)) = line.split_once('=') else { continue };
			let Ok(n) = v.trim().parse::<u32>() else { continue };
			match k.trim() {
				"sfx" => o.sfx = n.min(VOLUME_MAX as u32) as u16,
				"music" => o.music = n.min(VOLUME_MAX as u32) as u16,
				"stereo" => o.stereo = n != 0,
				"cam_lr" => o.cam_lr = n != 0,
				"cam_ud" => o.cam_ud = n != 0,
				"cam_speed" => o.cam_speed = n.min(2) as u8,
				"subtitles" => o.subtitles = n != 0,
				"help_voice" => o.help_voice = n != 0,
				"help_text" => o.help_text = n != 0,
				"lang" => o.lang = n.min(4) as u8,
				"hz60" => o.hz60 = n != 0,
				"cam_modern" => o.cam_modern = n != 0,
				"smooth" => o.smooth = n != 0,
				_ => {}
			}
		}
		o
	}

	pub fn load() -> Settings {
		std::fs::read_to_string(path()).map(|s| Settings::parse(&s)).unwrap_or_default()
	}

	pub fn store(&self) {
		let p = path();
		if let Some(d) = p.parent() {
			let _ = std::fs::create_dir_all(d);
		}
		if let Err(e) = std::fs::write(&p, self.to_text()) {
			eprintln!("settings: {e}");
		}
	}
}

fn path() -> PathBuf {
	crate::save::slot_path(0).with_file_name("settings.txt")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn volume_steps_and_round_trip() {
		assert_eq!(step_volume(0x400, 1), 0x400);
		assert_eq!(step_volume(1, -1), 0);
		assert_eq!(step_volume(0x2cc, 1), 0x2cf);
		// A full sweep at 3 per frame takes 342 frames.
		let mut v = 0;
		let mut n = 0;
		while v < VOLUME_MAX {
			v = step_volume(v, 1);
			n += 1;
		}
		assert_eq!(n, 342);
		let s = Settings { sfx: 17, music: 900, stereo: false, cam_lr: false, cam_ud: true, cam_speed: 2, subtitles: true, help_voice: false, help_text: true, lang: 3, hz60: true, cam_modern: true, smooth: false, cheats: [0; 12], cheat_unlocks: [0; 12] };
		assert_eq!(Settings::parse(&s.to_text()), s);
		assert_eq!(Settings::parse("junk\nsfx=x\nmusic=5000"), Settings { music: 0x400, ..Settings::default() });
		assert!((Settings::default().music_gain() - 1.0).abs() < 1e-6);
		let n = Settings { sfx: 9, music: 8, stereo: false, cam_lr: false, cam_speed: 2, subtitles: true, help_voice: false, lang: 2, cheats: [1; 12], ..Settings::default() }.new_game();
		assert_eq!((n.sfx, n.music, n.lang), (9, 8, 2));
		assert_eq!(Settings { sfx: 9, music: 8, lang: 2, ..Settings::default() }, n);
	}
}
