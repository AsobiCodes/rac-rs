//! The PC options file (`<player dir>/pc_options.txt`, next to card.bin): what the player chose in the game's own menus and
//! that the retail memory card does not hold. Plain `key=value` lines, written by the game, never typed by the player:
//!
//! ```text
//! version=1
//! lang=de          en fr de es it   (Options -> Language at the title)
//! camera=modern    modern classic   (Pause -> Options -> Camera Options -> Camera)
//! camfeel=smooth   smooth static    (Pause -> Options -> Camera Options -> Camera, the Modern camera's feel: Smooth = the dynamic camera of
//!                   modcam.rs, Static = the plain Modern camera; absent = smooth, only `static` is written; the row is Classic / Static /
//!                   Smooth, `camera=` keeps meaning Classic or Modern)
//! strafe=on        on off           (Pause -> Options -> Camera Options -> Strafe aiming)
//! buyall=on        on off           (Pause -> Options -> Camera Options -> Buy all ammo: Square at a Gadgetron vendor)
//! lockon=auto      auto on off      (Pause -> Options -> Camera Options -> Lock-on: the strafe aim's red lock circle; auto = on with a
//!                   pad, off with keyboard and mouse)
//! camspeed=50      10..200 in steps of 10   (Pause -> Options -> Camera Options -> Camera speed, in percent: 50 = retail's Fast; the
//!                   key is only written once the player changes the row, absent = retail's own Slow / Medium / Fast row untouched)
//! keys=cross:KeyK,start:Escape+Enter   (Pause -> Options -> Controls: keyboard keys that differ from the defaults, function:key names; binds.rs)
//! pad=cross:b,circle:a                  (Pause -> Options -> Controls: controller buttons that differ from the defaults; binds.rs)
//! lighting=enhanced   classic enhanced   (Pause -> Options -> Graphics -> Lighting; viewer/post.rs. Older files: original reads
//!                   as classic, modern / experimental as enhanced)
//! ao=high           off low medium high   (Pause -> Options -> Graphics -> Ambient occlusion: with Lighting Enhanced; older files: on = high)
//! shadows=high      off low medium high   (Pause -> Options -> Graphics -> Dynamic shadows: with Lighting Enhanced; older files: on = high)
//! display=fullscreen   windowed fullscreen   (Pause -> Options -> Graphics -> Display mode: borderless on the window's monitor)
//! water=high        classic low medium high   (Pause -> Options -> Graphics -> Water: with Lighting Enhanced; viewer/waterfx.rs; older files: enhanced = high)
//! particles=high    classic low medium high   (Pause -> Options -> Graphics -> Particles: with Lighting Enhanced; older files: enhanced = high)
//! sunrays=high      off low medium high   (Pause -> Options -> Graphics -> Sun rays: with Lighting Enhanced; older files: on = high)
//! smoothing=on      on off           (Pause -> Options -> Graphics -> Frame smoothing; viewer/interp.rs)
//! vegetation=moderate   off moderate lush   (Pause -> Options -> Graphics -> Vegetation: with Lighting Enhanced; viewer/vegfx.rs)
//! culling=on        on off           (Pause -> Options -> Graphics -> Culling: Off draws what the renderer would leave out; viewer/gpu.rs)
//! gi=off            off low medium high   (Pause -> Options -> Graphics -> Global illumination: with Lighting Enhanced; viewer/post.rs)
//! waves=on          on off           (Pause -> Options -> Graphics -> Waves: the sea's geometric waves at Water Medium / High; viewer/waterfx.rs)
//! draw=medium       low medium high   (Pause -> Options -> Graphics -> Draw distance, with either Lighting: low = retail's distances and
//!                   PS2 far LODs, medium = 4x farther up to 720 m, high = up to 2 km; viewer/fardraw.rs)
//! hud=sharp         sharp original   (the HUD and menus drawn again at the window's resolution, no new art; original = the 512-wide picture;
//!                   gscap/draw/gpu.rs, gpu.wgsl k_hd; Pause -> Options -> Graphics -> Picture -> HUD, applied at once; only on windows at least
//!                   896 px tall (gscap.rs HD_MIN_PX_PER_ROW); RAC_HUD=original forces Original)
//! glyphs=xbox        playstation xbox switch   (Pause -> Options -> Graphics -> Screen -> Button glyphs: how a gamepad's buttons are named in
//!                   the prompts, PlayStation being the retail's own glyphs, Xbox the letters A B X Y, and Switch the Xbox's with the four
//!                   face buttons the other way round; the keyboard names the buttons by key whatever this says)
//! debug=off        off on           (Pause -> Options -> Controls -> Debug -> Debug mode: On switches the test keys of retail/devtools.rs (F1 planet
//!                   list / warp, F2 free camera) and retail/debugkeys.rs (F5 immortality, F6 infinite ammo, F8 +10000 bolts, F3 draw distance A/B)
//!                   on; only `on` is written, the env switches RAC_DEV_TOOLS / RAC_DEBUG_KEYS still force them on)
//! ```
//!
//! A missing file, a missing key or a value that cannot be read gives that key's default (camera modern, strafe on, language
//! not chosen yet = the console language of the cold boot, lighting classic = the retail picture, ambient occlusion,
//! dynamic shadows, water, particles and sun rays high (the full Enhanced picture) and vegetation moderate for when the player picks
//! Lighting Enhanced, global illumination off, display windowed, draw distance medium, button glyphs Xbox). The six quality rows hold a level 0..3 (off / classic, low, medium, high). Keys this build does not know are kept when the file is rewritten.
//! The file is written to a temporary name and renamed, so a crash never leaves half a file.
use std::path::{Path, PathBuf};

/// The five languages of the PAL disc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
	En,
	Fr,
	De,
	Es,
	It,
}

impl Lang {
	pub const ALL: [Lang; 5] = [Lang::En, Lang::Fr, Lang::De, Lang::Es, Lang::It];

	pub fn code(self) -> &'static str {
		["en", "fr", "de", "es", "it"][self as usize]
	}

	pub fn from_code(s: &str) -> Option<Lang> {
		Lang::ALL.into_iter().find(|l| l.code() == s.trim().to_ascii_lowercase())
	}

	/// The PS2 console's OSD language number a cold boot reports (`ee_oracle::cold::set_language`): English 1, French 2, Spanish 3,
	/// German 4, Italian 5.
	pub fn osd(self) -> u32 {
		[1, 2, 4, 3, 5][self as usize]
	}

	/// The game's own language word (0x15ed88): English 0, French 2, German 3, Spanish 4, Italian 5 (the value Options -> Language
	/// stores; the OSD numbers map 1 -> 0, 2 -> 2, 3 -> 4, 4 -> 3, 5 -> 5).
	pub fn word(self) -> u32 {
		[0, 2, 3, 4, 5][self as usize]
	}

	pub fn from_word(w: u32) -> Option<Lang> {
		Lang::ALL.into_iter().find(|l| l.word() == w)
	}
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PcOptions {
	/// None: not chosen yet.
	pub lang: Option<Lang>,
	/// Some(true) = Modern, Some(false) = Classic; None: never changed (the default applies).
	pub camera: Option<bool>,
	/// Camera Options -> Camera, the Modern camera's feel: Some(true) = Smooth, Some(false) = Static; None: never changed (Smooth).
	pub camfeel: Option<bool>,
	/// Some(true) = On; None: never changed.
	pub strafe: Option<bool>,
	/// Some(true) = On (Square buys all ammo at a vendor); None: never changed.
	pub buyall: Option<bool>,
	/// Camera Options -> Lock-on: Some(0 Auto, 1 On, 2 Off); None: never changed (Auto).
	pub lockon: Option<u8>,
	/// Camera Options -> Camera speed in percent: Some(10..=200, steps of 10; 50 = retail's Fast); None: never changed (retail's own
	/// Slow / Medium / Fast row decides).
	pub camspeed: Option<u8>,
	/// Graphics -> Lighting: 0 Classic (the retail picture), 1 Enhanced; None: never changed (Classic).
	pub lighting: Option<u8>,
	/// Graphics -> Ambient occlusion: Some(0 Off, 1 Low, 2 Medium, 3 High); None: never changed (High).
	pub ao: Option<u8>,
	/// Graphics -> Dynamic shadows: Some(0 Off, 1 Low, 2 Medium, 3 High); None: never changed (High).
	pub shadows: Option<u8>,
	/// Graphics -> Display mode: Some(true) = Fullscreen; None: never changed (Windowed).
	pub fullscreen: Option<bool>,
	/// Graphics -> Water: Some(0 Classic, 1 Low, 2 Medium, 3 High); None: never changed (High).
	pub water: Option<u8>,
	/// Graphics -> Particles: Some(0 Classic, 1 Low, 2 Medium, 3 High); None: never changed (High).
	pub particles: Option<u8>,
	/// Graphics -> Sun rays: Some(0 Off, 1 Low, 2 Medium, 3 High); None: never changed (High).
	pub sun_rays: Option<u8>,
	/// Graphics -> Frame smoothing: Some(true) = On; None: never changed (On).
	pub smoothing: Option<bool>,
	/// Graphics -> Vegetation: Some(0) Off, 1 Moderate, 2 Lush; None: never changed (Moderate).
	pub vegetation: Option<u8>,
	/// Graphics -> Culling: Some(true) = On; None: never changed (On).
	pub culling: Option<bool>,
	/// Graphics -> Global illumination: Some(0 Off, 1 Low, 2 Medium, 3 High); None: never changed (Off).
	pub gi: Option<u8>,
	/// Graphics -> Waves: Some(true) = On; None: never changed (On).
	pub waves: Option<bool>,
	/// Graphics -> Draw distance: Some(0 Low = retail, 1 Medium, 2 High); None: never changed (Medium).
	pub draw: Option<u8>,
	/// HUD: Some(true) = Sharp (the HUD drawn again at the window's resolution, gscap/draw/gpu.rs), Some(false) = Original; None: Sharp.
	pub hud: Option<bool>,
	/// Graphics -> Button glyphs: Some(0 PlayStation, 1 Xbox, 2 Switch); None: never changed (Xbox).
	pub glyphs: Option<u8>,
	/// Debug mode (Controls -> Debug): Some(true) = On; None: Off. Only `debug=on` is written.
	pub debug: Option<bool>,
	/// Lines this build does not know, kept verbatim on rewrite.
	extra: Vec<(String, String)>,
}

/// The Lighting values as the file writes them (index = the Graphics page's value byte).
pub const LIGHTING: [&str; 2] = ["classic", "enhanced"];

/// A stored Lighting value: the names above, plus the first test build's (original = classic, modern / experimental = enhanced).
fn lighting_value(v: &str) -> Option<u8> {
	match v.trim().to_ascii_lowercase().as_str() {
		"classic" | "original" => Some(0),
		"enhanced" | "modern" | "experimental" => Some(1),
		_ => None,
	}
}

fn vegetation_value(v: &str) -> Option<u8> {
	match v.trim().to_ascii_lowercase().as_str() {
		"off" => Some(0),
		"moderate" => Some(1),
		"lush" => Some(2),
		_ => None,
	}
}

/// The button glyphs styles as the file writes them (index = the Graphics page's value byte).
pub const GLYPHS: [&str; 3] = ["playstation", "xbox", "switch"];

/// A stored button glyphs style: the names above.
fn glyphs_value(v: &str) -> Option<u8> {
	GLYPHS.iter().position(|g| *g == v.trim().to_ascii_lowercase()).map(|i| i as u8)
}

/// The quality level names of the five effect rows (index = the row byte; 0 is Off, or Classic for Water and Particles).
pub const LEVELS: [&str; 4] = ["off", "low", "medium", "high"];
pub const LEVELS_CLASSIC: [&str; 4] = ["classic", "low", "medium", "high"];
/// The Draw distance levels as the file writes them (index = the level: 0 Low = retail, 1 Medium, 2 High).
pub const DRAW: [&str; 3] = ["low", "medium", "high"];

/// The Graphics page's Draw distance byte of a level and back: the byte order is Medium, High, Low, so a new player's zero byte
/// is Medium (the default) and the menu still cycles Low -> Medium -> High -> Low.
pub const fn draw_byte(level: u8) -> u8 {
	let l = if level > 2 { 2 } else { level };
	(l + 2) % 3
}
pub const fn draw_level(byte: u8) -> u8 {
	let b = if byte > 2 { 2 } else { byte };
	(b + 1) % 3
}

fn draw_value(v: &str) -> Option<u8> {
	DRAW.iter().position(|d| v.trim().eq_ignore_ascii_case(d)).map(|l| l as u8)
}

/// A stored quality level: 0 off / classic, 1 low, 2 medium, 3 high. The older files' on / enhanced (and true, 1, yes) mean the
/// full picture, high; their off / classic mean 0.
fn level_value(v: &str) -> Option<u8> {
	match v.trim().to_ascii_lowercase().as_str() {
		"off" | "classic" | "false" | "0" | "no" => Some(0),
		"low" => Some(1),
		"medium" => Some(2),
		"high" | "on" | "enhanced" | "true" | "1" | "yes" => Some(3),
		_ => None,
	}
}

/// The Lock-on values as the file writes them (index = the row byte: 0 Auto, 1 On, 2 Off).
pub const LOCKON: [&str; 3] = ["auto", "on", "off"];

fn lockon_value(v: &str) -> Option<u8> {
	LOCKON.iter().position(|d| v.trim().eq_ignore_ascii_case(d)).map(|l| l as u8)
}

/// The camera speed range of the row, in percent (50 = retail's Fast).
pub const CAMSPEED_MIN: u8 = 10;
pub const CAMSPEED_MAX: u8 = 200;
pub const CAMSPEED_STEP: u8 = 10;

/// A camera speed percent snapped to the row's grid: nearest multiple of 10 inside 10..=200.
pub fn snap_camspeed(p: u8) -> u8 {
	((p as u16 + 5) / 10 * 10).clamp(CAMSPEED_MIN as u16, CAMSPEED_MAX as u16) as u8
}

/// A stored camera speed: an integer percent (a trailing % is accepted), snapped to the grid; unreadable values give None.
fn camspeed_value(v: &str) -> Option<u8> {
	let n: u32 = v.trim().trim_end_matches('%').trim().parse().ok()?;
	Some(snap_camspeed(n.min(255) as u8))
}

fn flag(v: &str, yes: &str, no: &str) -> Option<bool> {
	let v = v.trim().to_ascii_lowercase();
	match v.as_str() {
		x if x == yes || x == "true" || x == "1" || x == "yes" => Some(true),
		x if x == no || x == "false" || x == "0" || x == "no" => Some(false),
		_ => None,
	}
}

impl PcOptions {
	pub const fn new() -> PcOptions {
		PcOptions { lang: None, camera: None, camfeel: None, strafe: None, buyall: None, lockon: None, camspeed: None, lighting: None, ao: None, shadows: None, fullscreen: None, water: None, particles: None, sun_rays: None, smoothing: None, vegetation: None, culling: None, gi: None, waves: None, draw: None, hud: None, glyphs: None, debug: None, extra: Vec::new() }
	}

	/// Lock-on with the default: Auto (0).
	pub fn lockon_mode(&self) -> u8 {
		self.lockon.unwrap_or(0).min(2)
	}

	/// The button glyphs style with the default: Xbox (1).
	pub fn glyph_style(&self) -> u8 {
		self.glyphs.unwrap_or(1).min(2)
	}

	/// Debug mode with the default: Off.
	pub fn debug_on(&self) -> bool {
		self.debug.unwrap_or(false)
	}

	/// The HUD row with the default: Sharp.
	pub fn hud_sharp(&self) -> bool {
		self.hud.unwrap_or(true)
	}

	/// Lighting with the default: Classic (0).
	pub fn lighting_or_default(&self) -> u8 {
		self.lighting.unwrap_or(0).min(1)
	}

	/// Ambient occlusion with the default: High (3).
	pub fn ao_level(&self) -> u8 {
		self.ao.unwrap_or(3).min(3)
	}

	/// Dynamic shadows with the default: High (3).
	pub fn shadows_level(&self) -> u8 {
		self.shadows.unwrap_or(3).min(3)
	}

	/// The Graphics page's row bytes (retail/pcext.rs `GROWS` order: lighting, ambient occlusion, dynamic shadows, display mode,
	/// water, particles, sun rays, frame smoothing, vegetation, culling: 0 On, 1 Off, global illumination, waves: 0 On, 1 Off, draw
	/// distance: `draw_byte`) with the defaults.
	pub fn graphics_rows(&self) -> [u8; 13] {
		[
			self.lighting_or_default(),
			self.ao_level(),
			self.shadows_level(),
			self.fullscreen_on() as u8,
			self.water.unwrap_or(3).min(3),
			self.particles.unwrap_or(3).min(3),
			self.sun_rays.unwrap_or(3).min(3),
			self.smoothing.unwrap_or(true) as u8,
			self.vegetation.unwrap_or(1),
			!self.culling.unwrap_or(true) as u8,
			self.gi.unwrap_or(0).min(3),
			!self.waves.unwrap_or(true) as u8,
			draw_byte(self.draw.unwrap_or(1)),
		]
	}

	/// The player chose `g` (`graphics_rows` order): every Graphics key is written.
	pub fn set_graphics_rows(&mut self, g: [u8; 13]) {
		self.lighting = Some(g[0].min(1));
		self.ao = Some(g[1].min(3));
		self.shadows = Some(g[2].min(3));
		self.fullscreen = Some(g[3] == 1);
		self.water = Some(g[4].min(3));
		self.particles = Some(g[5].min(3));
		self.sun_rays = Some(g[6].min(3));
		self.smoothing = Some(g[7] == 1);
		self.vegetation = Some(g[8].min(2));
		self.culling = Some(g[9] != 1);
		self.gi = Some(g[10].min(3));
		self.waves = Some(g[11] != 1);
		self.draw = Some(draw_level(g[12]));
	}

	/// The value of a key this file type does not model itself (`keys`, `pad`: binds.rs).
	pub fn extra_get(&self, key: &str) -> Option<&str> {
		self.extra.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
	}

	/// Sets (Some) or removes (None) such a key; it is written with the other unknown keys.
	pub fn extra_set(&mut self, key: &str, value: Option<String>) {
		self.extra.retain(|(k, _)| k != key);
		if let Some(v) = value {
			self.extra.push((key.to_string(), v));
		}
	}

	/// Display mode with the default: Windowed (false).
	pub fn fullscreen_on(&self) -> bool {
		self.fullscreen.unwrap_or(false)
	}

	/// Camera style with the default: Modern.
	pub fn camera_modern(&self) -> bool {
		self.camera.unwrap_or(true)
	}

	/// The Modern camera's feel with the default: Smooth (true), Static (false).
	pub fn camfeel_smooth(&self) -> bool {
		self.camfeel.unwrap_or(true)
	}

	/// The Camera row's byte: 0 Classic, 1 Static, 2 Smooth.
	pub fn camera_byte(&self) -> u8 {
		match (self.camera_modern(), self.camfeel_smooth()) {
			(false, _) => 0,
			(true, false) => 1,
			(true, true) => 2,
		}
	}

	/// The player chose the Camera row's byte `b` (0 Classic, 1 Static, 2 Smooth): Classic keeps the remembered feel (F7 flips between
	/// Classic and it); the feel is only remembered as Static (Smooth is the default and leaves no key).
	pub fn set_camera_byte(&mut self, b: u8) {
		self.camera = Some(b != 0);
		if b != 0 {
			self.camfeel = (b < 2).then_some(false);
		}
	}

	/// Buy all ammo with the default: On.
	pub fn buyall_on(&self) -> bool {
		self.buyall.unwrap_or(true)
	}

	/// Strafe aiming with the default: On.
	pub fn strafe_on(&self) -> bool {
		self.strafe.unwrap_or(true)
	}

	/// Reads the text of a file: tolerant, never fails.
	pub fn parse(text: &str) -> PcOptions {
		let mut o = PcOptions::default();
		for line in text.lines() {
			let line = line.trim_start_matches('\u{feff}').trim();
			let Some((k, v)) = line.split_once('=') else { continue };
			let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
			match k.as_str() {
				"version" => {}
				"lang" => o.lang = Lang::from_code(v),
				"camera" => o.camera = flag(v, "modern", "classic"),
				"camfeel" => o.camfeel = flag(v, "smooth", "static"),
				"strafe" => o.strafe = flag(v, "on", "off"),
				"buyall" => o.buyall = flag(v, "on", "off"),
				"lockon" => o.lockon = lockon_value(v),
				"camspeed" => o.camspeed = camspeed_value(v),
				"lighting" => o.lighting = lighting_value(v),
				"ao" => o.ao = level_value(v),
				"shadows" => o.shadows = level_value(v),
				"display" => o.fullscreen = flag(v, "fullscreen", "windowed"),
				"water" => o.water = level_value(v),
				"particles" => o.particles = level_value(v),
				"sunrays" => o.sun_rays = level_value(v),
				"smoothing" => o.smoothing = flag(v, "on", "off"),
				"vegetation" => o.vegetation = vegetation_value(v),
				"culling" => o.culling = flag(v, "on", "off"),
				"gi" => o.gi = level_value(v),
				"waves" => o.waves = flag(v, "on", "off"),
				"draw" => o.draw = draw_value(v),
				"hud" => o.hud = flag(v, "sharp", "original"),
				"glyphs" => o.glyphs = glyphs_value(v),
				"debug" => o.debug = flag(v, "on", "off"),
				_ if !k.is_empty() && !k.starts_with('#') => {
					o.extra.retain(|(e, _)| *e != k);
					o.extra.push((k, v.to_string()));
				}
				_ => {}
			}
		}
		o
	}

	/// The file text: only the keys the player has set, plus the unknown ones.
	pub fn to_text(&self) -> String {
		let mut s = String::from("version=1\n");
		if let Some(l) = self.lang {
			s += &format!("lang={}\n", l.code());
		}
		if let Some(c) = self.camera {
			s += &format!("camera={}\n", if c { "modern" } else { "classic" });
		}
		// Smooth is the default: only Static is written.
		if self.camfeel == Some(false) {
			s += "camfeel=static\n";
		}
		if let Some(t) = self.strafe {
			s += &format!("strafe={}\n", if t { "on" } else { "off" });
		}
		if let Some(t) = self.buyall {
			s += &format!("buyall={}\n", if t { "on" } else { "off" });
		}
		if let Some(l) = self.lockon {
			s += &format!("lockon={}\n", LOCKON[(l as usize).min(LOCKON.len() - 1)]);
		}
		if let Some(p) = self.camspeed {
			s += &format!("camspeed={}\n", snap_camspeed(p));
		}
		if let Some(l) = self.lighting {
			s += &format!("lighting={}\n", LIGHTING[(l as usize).min(LIGHTING.len() - 1)]);
		}
		if let Some(l) = self.ao {
			s += &format!("ao={}\n", LEVELS[l.min(3) as usize]);
		}
		if let Some(l) = self.shadows {
			s += &format!("shadows={}\n", LEVELS[l.min(3) as usize]);
		}
		if let Some(f) = self.fullscreen {
			s += &format!("display={}\n", if f { "fullscreen" } else { "windowed" });
		}
		if let Some(l) = self.water {
			s += &format!("water={}\n", LEVELS_CLASSIC[l.min(3) as usize]);
		}
		if let Some(l) = self.particles {
			s += &format!("particles={}\n", LEVELS_CLASSIC[l.min(3) as usize]);
		}
		if let Some(l) = self.sun_rays {
			s += &format!("sunrays={}\n", LEVELS[l.min(3) as usize]);
		}
		if let Some(t) = self.smoothing {
			s += &format!("smoothing={}\n", if t { "on" } else { "off" });
		}
		if let Some(t) = self.vegetation {
			s += &format!("vegetation={}\n", ["off", "moderate", "lush"][t.min(2) as usize]);
		}
		if let Some(t) = self.culling {
			s += &format!("culling={}\n", if t { "on" } else { "off" });
		}
		if let Some(l) = self.gi {
			s += &format!("gi={}\n", LEVELS[l.min(3) as usize]);
		}
		if let Some(t) = self.waves {
			s += &format!("waves={}\n", if t { "on" } else { "off" });
		}
		if let Some(l) = self.draw {
			s += &format!("draw={}\n", DRAW[l.min(2) as usize]);
		}
		if let Some(t) = self.hud {
			s += &format!("hud={}\n", if t { "sharp" } else { "original" });
		}
		if let Some(g) = self.glyphs {
			s += &format!("glyphs={}\n", GLYPHS[g.min(2) as usize]);
		}
		if self.debug == Some(true) {
			s += "debug=on\n";
		}
		for (k, v) in &self.extra {
			s += &format!("{k}={v}\n");
		}
		s
	}

	/// The file's options; the defaults when it is missing or unreadable.
	pub fn load(path: &Path) -> PcOptions {
		std::fs::read(path).map(|b| PcOptions::parse(&String::from_utf8_lossy(&b))).unwrap_or_default()
	}

	/// Writes the file atomically (temporary file + rename).
	pub fn store(&self, path: &Path) -> std::io::Result<()> {
		if let Some(dir) = path.parent() {
			std::fs::create_dir_all(dir)?;
		}
		let mut tmp = path.as_os_str().to_owned();
		tmp.push(".tmp");
		let tmp = PathBuf::from(tmp);
		std::fs::write(&tmp, self.to_text())?;
		std::fs::rename(&tmp, path)
	}
}

/// `<player dir>/pc_options.txt`.
pub fn file() -> PathBuf {
	super::memcard::player_dir().join("pc_options.txt")
}

#[cfg(test)]
mod tests {
	use super::*;

	fn scratch(name: &str) -> PathBuf {
		let d = std::env::temp_dir().join(format!("pcopts_test_{}_{name}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		d
	}

	#[test]
	fn missing_file_gives_the_defaults() {
		let d = scratch("missing");
		let o = PcOptions::load(&d.join("pc_options.txt"));
		assert_eq!(o.lang, None);
		assert!(o.camera_modern(), "a new player starts with the modern camera");
		assert!(o.strafe_on(), "a new player starts with strafe aiming on");
		assert!(o.buyall_on(), "a new player starts with buy all ammo on");
		assert_eq!(o.lighting_or_default(), 0, "a new player sees the retail picture (Lighting Classic)");
		assert_eq!((o.ao_level(), o.shadows_level()), (3, 3), "ambient occlusion and dynamic shadows wait at High for Enhanced");
		assert_eq!(o.glyph_style(), 1, "a new player gets the Xbox letters");
		assert_eq!(o.to_text(), "version=1\n", "nothing is written until the player chooses something");
	}

	/// The button glyphs style: the file's three names, and the default when the key is missing or unreadable.
	#[test]
	fn the_glyphs_style_is_read_and_written() {
		for (i, name) in GLYPHS.iter().enumerate() {
			let o = PcOptions::parse(&format!("version=1\nglyphs={name}\n"));
			assert_eq!(o.glyphs, Some(i as u8), "glyphs={name}");
			assert_eq!(o.glyph_style(), i as u8);
		}
		let o = PcOptions::parse("version=1\nglyphs=nonsense\n");
		assert_eq!(o.glyphs, None, "an unreadable value is not kept");
		assert_eq!(o.glyph_style(), 1, "and the default stands");
		let mut o = PcOptions::new();
		o.glyphs = Some(2);
		assert!(o.to_text().contains("glyphs=switch\n"), "the style is written back");
	}

	#[test]
	fn round_trip_and_atomic_write() {
		let d = scratch("round");
		let p = d.join("pc_options.txt");
		let o = PcOptions { lang: Some(Lang::De), camera: Some(false), strafe: Some(false), buyall: Some(false), ..Default::default() };
		o.store(&p).unwrap();
		assert_eq!(std::fs::read_to_string(&p).unwrap(), "version=1\nlang=de\ncamera=classic\nstrafe=off\nbuyall=off\n");
		assert_eq!(PcOptions::load(&p), o);
		let names: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
		assert_eq!(names, ["pc_options.txt"], "no temporary file is left behind");
		// A second write replaces the first.
		let o2 = PcOptions { camera: Some(true), ..o.clone() };
		o2.store(&p).unwrap();
		assert_eq!(PcOptions::load(&p).camera, Some(true));
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn garbage_lines_and_bad_values_keep_the_other_keys() {
		let o = PcOptions::parse("\u{feff}version=1\nthis is not a setting\n=\nlang = FR \ncamera=sideways\nstrafe=OFF\n\n");
		assert_eq!(o.lang, Some(Lang::Fr), "case and spaces do not matter");
		assert_eq!(o.camera, None, "a bad value falls back to the default for that key only");
		assert!(o.camera_modern());
		assert_eq!(o.strafe, Some(false));
		assert_eq!(PcOptions::parse("buyall=OFF\nstrafe=maybe\n").buyall, Some(false));
		let o = PcOptions::parse("lang=klingon\ncamera=classic\n");
		assert_eq!(o.lang, None);
		assert_eq!(o.camera, Some(false));
	}

	#[test]
	fn camera_feel_key_round_trips() {
		// Defaults: Smooth; old files (camera= only) read as Smooth.
		assert!(PcOptions::new().camfeel_smooth());
		assert_eq!(PcOptions::new().camera_byte(), 2, "a new player: Modern + Smooth = byte 2");
		let old = PcOptions::parse("version=1\ncamera=modern\nstrafe=on\n");
		assert_eq!((old.camfeel, old.camfeel_smooth(), old.camera_byte()), (None, true, 2), "old file: smooth");
		assert_eq!(PcOptions::parse("camera=classic\n").camera_byte(), 0);
		assert_eq!(PcOptions::parse("camera=modern\ncamfeel=static\n").camera_byte(), 1);
		assert_eq!(PcOptions::parse("camera=modern\ncamfeel=SMOOTH\n").camera_byte(), 2);
		assert_eq!(PcOptions::parse("camfeel=sideways\n").camfeel, None, "unreadable: default");
		// Static is the only value written; Smooth is the default and leaves no key.
		let st = PcOptions { camera: Some(true), camfeel: Some(false), ..Default::default() };
		assert_eq!(st.to_text(), "version=1\ncamera=modern\ncamfeel=static\n");
		assert_eq!(PcOptions::parse(&st.to_text()), st, "round trip");
		let sm = PcOptions { camera: Some(true), camfeel: Some(true), ..Default::default() };
		assert_eq!(sm.to_text(), "version=1\ncamera=modern\n", "smooth = default, not written");
		// set_camera_byte: all three values; Classic keeps the remembered feel.
		let mut o = PcOptions::new();
		o.set_camera_byte(1);
		assert_eq!((o.camera, o.camfeel, o.camera_byte()), (Some(true), Some(false), 1));
		o.set_camera_byte(0);
		assert_eq!((o.camera, o.camfeel, o.camera_byte()), (Some(false), Some(false), 0), "Classic keeps the feel");
		o.set_camera_byte(2);
		assert_eq!((o.camera, o.camfeel, o.camera_byte()), (Some(true), None, 2));
		assert_eq!(o.to_text(), "version=1\ncamera=modern\n");
		// Written between camera and strafe, the other keys undisturbed.
		let o = PcOptions { camera: Some(true), camfeel: Some(false), strafe: Some(false), ..Default::default() };
		assert_eq!(o.to_text(), "version=1\ncamera=modern\ncamfeel=static\nstrafe=off\n");
	}

	#[test]
	fn debug_key_round_trips() {
		// Default Off; a file without the key is Off; only `on` is written.
		assert!(!PcOptions::new().debug_on());
		assert!(!PcOptions::parse("version=1\ncamera=modern\n").debug_on());
		let on = PcOptions::parse("debug=on\n");
		assert_eq!((on.debug, on.debug_on()), (Some(true), true));
		assert_eq!(on.to_text(), "version=1\ndebug=on\n");
		for v in ["ON", "true", "1", "yes"] {
			assert!(PcOptions::parse(&format!("debug={v}\n")).debug_on(), "debug={v}");
		}
		let off = PcOptions::parse("debug=off\n");
		assert_eq!((off.debug, off.debug_on()), (Some(false), false));
		assert_eq!(off.to_text(), "version=1\n", "Off is the default: no key");
		assert_eq!(PcOptions::parse("debug=maybe\n").debug, None, "unreadable: Off");
		// Switching off again drops the key; the other keys and unknown lines stay.
		let mut o = PcOptions::parse("camera=modern\ndebug=on\nstrafe=off\nzzz=1\n");
		assert_eq!(o.to_text(), "version=1\ncamera=modern\nstrafe=off\ndebug=on\nzzz=1\n");
		o.debug = Some(false);
		assert_eq!(o.to_text(), "version=1\ncamera=modern\nstrafe=off\nzzz=1\n");
		let back = PcOptions::parse(&o.to_text());
		assert!(!back.debug_on());
		let mut o2 = PcOptions::new();
		o2.debug = Some(true);
		assert!(PcOptions::parse(&o2.to_text()).debug_on(), "round trip");
	}

	#[test]
	fn camera_option_keys_round_trip() {
		for (name, mode) in [("auto", 0u8), ("On", 1), ("OFF", 2)] {
			let o = PcOptions::parse(&format!("lockon={name}\n"));
			assert_eq!((o.lockon, o.lockon_mode()), (Some(mode), mode), "lockon={name}");
			assert_eq!(o.to_text(), format!("version=1\nlockon={}\n", name.to_ascii_lowercase()));
		}
		assert_eq!(PcOptions::new().lockon_mode(), 0, "default Auto");
		assert_eq!(PcOptions::parse("lockon=maybe\n").lockon, None, "unreadable value: Auto");
		// Camera speed: percent, 10..200 in steps of 10; absent = None (retail's own row untouched).
		for (text, want) in [("100", 100u8), ("50", 50), ("10", 10), ("200", 200), ("5", 10), ("0", 10), ("999", 200), ("255", 200), ("44", 40), ("45", 50), ("75%", 80), (" 120 ", 120)] {
			let o = PcOptions::parse(&format!("camspeed={text}\n"));
			assert_eq!(o.camspeed, Some(want), "camspeed={text}");
			assert_eq!(o.to_text(), format!("version=1\ncamspeed={want}\n"));
			assert_eq!(PcOptions::parse(&o.to_text()).camspeed, Some(want), "round trip");
		}
		assert_eq!(PcOptions::parse("camspeed=fast\n").camspeed, None);
		assert_eq!(PcOptions::new().camspeed, None);
		assert_eq!(PcOptions::new().to_text(), "version=1\n", "neither key is written until the player changes the row");
		// Written after buyall, before the Graphics keys.
		let o = PcOptions { buyall: Some(true), lockon: Some(2), camspeed: Some(70), lighting: Some(1), ..Default::default() };
		assert_eq!(o.to_text(), "version=1\nbuyall=on\nlockon=off\ncamspeed=70\nlighting=enhanced\n");
	}

	#[test]
	fn graphics_keys_round_trip() {
		let o = PcOptions::parse("version=1\nlighting=Enhanced\nao=off\nshadows=ON\n");
		assert_eq!((o.lighting, o.ao, o.shadows), (Some(1), Some(0), Some(3)), "an old file's on = High");
		assert_eq!(o.to_text(), "version=1\nlighting=enhanced\nao=off\nshadows=high\n", "rewritten with the new names");
		// The first test build's names still read: original = classic, modern / experimental = enhanced; rewritten new.
		assert_eq!(PcOptions::parse("lighting=original\n").to_text(), "version=1\nlighting=classic\n");
		assert_eq!(PcOptions::parse("lighting=Modern\n").to_text(), "version=1\nlighting=enhanced\n");
		let o = PcOptions::parse("display=Fullscreen\n");
		assert!(o.fullscreen_on());
		assert_eq!(o.to_text(), "version=1\ndisplay=fullscreen\n");
		assert_eq!(PcOptions::parse("display=windowed\n").fullscreen, Some(false));
		assert!(!PcOptions::parse("display=huge\n").fullscreen_on(), "unreadable value: Windowed");
		let o = PcOptions::parse("water=classic\nparticles=Enhanced\nsunrays=off\n");
		assert_eq!(o.graphics_rows(), [0, 3, 3, 0, 0, 3, 0, 1, 1, 0, 0, 0, 0], "old enhanced = High");
		assert_eq!(o.to_text(), "version=1\nwater=classic\nparticles=high\nsunrays=off\n");
		assert_eq!(PcOptions::default().graphics_rows(), [0, 3, 3, 0, 3, 3, 3, 1, 1, 0, 0, 0, 0], "defaults: Classic lighting, the rest ready for Enhanced at High, GI Off, waves On, draw distance Medium");
		let o = PcOptions::parse("vegetation=Lush\n");
		assert_eq!((o.vegetation, o.graphics_rows()[8]), (Some(2), 2));
		assert_eq!(o.to_text(), "version=1\nvegetation=lush\n");
		assert_eq!(PcOptions::parse("vegetation=off\n").vegetation, Some(0));
		assert_eq!(PcOptions::parse("vegetation=jungle\n").graphics_rows()[8], 1, "unreadable value: Moderate");
		let o = PcOptions::parse("culling=Off\n");
		assert_eq!((o.culling, o.graphics_rows()[9]), (Some(false), 1));
		assert_eq!(o.to_text(), "version=1\nculling=off\n");
		assert_eq!(PcOptions::new().graphics_rows()[9], 0, "default On");
		let o = PcOptions::parse("gi=Medium\n");
		assert_eq!((o.gi, o.graphics_rows()[10]), (Some(2), 2));
		assert_eq!(o.to_text(), "version=1\ngi=medium\n");
		assert_eq!(PcOptions::parse("gi=on\n").gi, Some(3));
		assert_eq!(PcOptions::new().graphics_rows()[10], 0, "default Off");
		let o = PcOptions::parse("waves=Off\n");
		assert_eq!((o.waves, o.graphics_rows()[11]), (Some(false), 1));
		assert_eq!(o.to_text(), "version=1\nwaves=off\n");
		assert_eq!(PcOptions::new().graphics_rows()[11], 0, "default On");
		// Draw distance: the file names the level (low, medium, high), the page's byte is Medium 0, High 1, Low 2.
		for (name, level, byte) in [("low", 0u8, 2u8), ("Medium", 1, 0), ("HIGH", 2, 1)] {
			let o = PcOptions::parse(&format!("draw={name}\n"));
			assert_eq!((o.draw, o.graphics_rows()[12]), (Some(level), byte), "draw={name}");
			assert_eq!(o.to_text(), format!("version=1\ndraw={}\n", name.to_ascii_lowercase()));
			let mut r = PcOptions::default();
			r.set_graphics_rows(o.graphics_rows());
			assert_eq!(PcOptions::parse(&r.to_text()).draw, Some(level), "draw={name} round trip");
		}
		assert_eq!(PcOptions::new().graphics_rows()[12], 0, "default Medium = byte 0");
		assert_eq!(PcOptions::parse("draw=far\n").draw, None, "unreadable value: Medium");
		assert_eq!([0u8, 1, 2].map(draw_level), [1, 2, 0]);
		assert_eq!([0u8, 1, 2].map(draw_byte), [2, 0, 1]);
		assert_eq!(PcOptions::parse("lighting=experimental\n").lighting, Some(1));
		let bad = PcOptions::parse("lighting=rtx\nao=maybe\n");
		assert_eq!((bad.lighting_or_default(), bad.ao_level()), (0, 3), "a bad value falls back to the default");
	}

	#[test]
	fn quality_levels_read_old_and_new_files() {
		// Every older value maps to the level that was the whole picture: on / enhanced = High, off / classic = 0.
		let old = PcOptions::parse("ao=on\nshadows=off\nwater=enhanced\nparticles=classic\nsunrays=on\n");
		assert_eq!((old.ao, old.shadows, old.water, old.particles, old.sun_rays), (Some(3), Some(0), Some(3), Some(0), Some(3)));
		// The new names, any case; rows keep their own zero name when written.
		let new = PcOptions::parse("ao=Low\nshadows=MEDIUM\nwater=low\nparticles=Medium\nsunrays=high\n");
		assert_eq!((new.ao, new.shadows, new.water, new.particles, new.sun_rays), (Some(1), Some(2), Some(1), Some(2), Some(3)));
		assert_eq!(new.graphics_rows(), [0, 1, 2, 0, 1, 2, 3, 1, 1, 0, 0, 0, 0]);
		assert_eq!(new.to_text(), "version=1\nao=low\nshadows=medium\nwater=low\nparticles=medium\nsunrays=high\n");
		let zero = PcOptions::parse("ao=off\nshadows=off\nwater=classic\nparticles=classic\nsunrays=off\n");
		assert_eq!(zero.to_text(), "version=1\nao=off\nshadows=off\nwater=classic\nparticles=classic\nsunrays=off\n");
		// An unreadable name is ignored (default High); a row set from the page round-trips through the file text.
		assert_eq!(PcOptions::parse("ao=ultra\n").ao_level(), 3);
		let mut o = PcOptions::default();
		o.set_graphics_rows([1, 2, 1, 1, 3, 0, 2, 0, 2, 1, 2, 1, 2]);
		assert_eq!(PcOptions::parse(&o.to_text()).graphics_rows(), [1, 2, 1, 1, 3, 0, 2, 0, 2, 1, 2, 1, 2]);
		assert_eq!(PcOptions::parse(&o.to_text()), o);
	}

	#[test]
	fn unknown_keys_survive_a_rewrite() {
		let o = PcOptions::parse("version=1\nfuture_thing=7\ncamera=classic\nfuture_thing=8\n");
		assert_eq!(o.to_text(), "version=1\ncamera=classic\nfuture_thing=8\n");
	}

	#[test]
	fn language_numbers_both_ways() {
		// OSD number (what the cold kernel reports) and the game's language word (what Options -> Language stores).
		let want = [(Lang::En, "en", 1, 0), (Lang::Fr, "fr", 2, 2), (Lang::De, "de", 4, 3), (Lang::Es, "es", 3, 4), (Lang::It, "it", 5, 5)];
		for (l, code, osd, word) in want {
			assert_eq!((l.code(), l.osd(), l.word()), (code, osd, word));
			assert_eq!(Lang::from_code(code), Some(l));
			assert_eq!(Lang::from_word(word), Some(l));
			// The cold kernel's own parser agrees with the OSD number.
			assert_eq!(ee_oracle::cold::parse_language(code), Some(osd));
		}
		assert_eq!(Lang::from_word(1), None, "word 1 is not on the disc");
		assert_eq!(Lang::from_word(6), None);
	}
}
