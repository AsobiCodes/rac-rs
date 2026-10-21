//! The Options values the pause menu edits (decomp/notes/ingame_menu_leftovers.md P1-d), kept in a
//! small text file `saves/settings.txt` next to the executable (the game keeps them in RAM globals).

use std::path::PathBuf;

/// Volume range of 0x15edf0 / 0x15edec.
pub const VOLUME_MAX: u16 = 0x400;
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

impl Settings {

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

}

fn path() -> PathBuf {
	crate::save::slot_path(0).with_file_name("settings.txt")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_reads_the_settings_file_format() {
		let s = Settings { sfx: 17, music: 900, stereo: false, cam_lr: false, cam_ud: true, cam_speed: 2, subtitles: true, help_voice: false, help_text: true, lang: 3, hz60: true, cam_modern: true, smooth: false, cheats: [0; 12], cheat_unlocks: [0; 12] };
		let text = "sfx=17\nmusic=900\nstereo=0\ncam_lr=0\ncam_ud=1\ncam_speed=2\nsubtitles=1\nhelp_voice=0\nhelp_text=1\nlang=3\nhz60=1\ncam_modern=1\nsmooth=0\n";
		assert_eq!(Settings::parse(text), s);
		assert_eq!(Settings::parse("junk\nsfx=x\nmusic=5000"), Settings { music: 0x400, ..Settings::default() });
		assert!((Settings::default().music_gain() - 1.0).abs() < 1e-6);
	}
}
