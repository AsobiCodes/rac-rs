//! [PC] Strafe aiming "Steady" (GitHub #73 + #27, lane nstr, 2026-11-03): the Ratchet & Clank 2 strafe. Modern Options -> Camera ->
//! Aiming -> Strafe aiming: Raised / Shoulder / Steady / Off; options file `strafe=on` + `aimcam=steady`. Default Raised: with `on()`
//! false every hook below gives back the value Raised / Shoulder / Off had before, so nothing changes.
//!
//! While the strafe button (RMB / pad R2) is held, Ratchet faces the camera heading and strafe-walks (modcam.rs `face`, strafe.rs,
//! strafe_feel.rs with R&C2's numbers: unchanged), and the camera keeps its normal-movement look: no aim lift (`shoulder::lift` runs its
//! blend only for `cam_aim`, so a lift in force fades out as when the aim ends), no shoulder shift (the Shoulder flag is off: Steady is
//! byte Raised + this flag), and with Camera Smooth the look bias, the probes, the boom length and its wall spring, the pivot follow and
//! the lock framing stay those of normal movement (`cam_aim` false for them). Only the auto-turn behind the run (`dyn_auto`) stays off
//! as in every strafe aim, so the camera does not swing round. Camera Classic borrows the modern camera while the button is held
//! (classic.rs), as for Raised: the press seeds it from retail's pose on screen and the camera stays where it was.
//!
//! Aim: the screen centre is on Ratchet, so there is no green centre reticle (aim.rs `pre_field` queues it at scale 0; a strafe lock
//! still turns it into the red icon on the target). The direct-fire weapons shoot along the camera heading, level from his chest (`level`
//! turns the view `aim_cam` gives aim.rs; the line starts at the chest goal, not the lagging orbit pivot), with the Lock-on help; Lock-on
//! Auto counts as on also with keyboard and mouse (`auto_keyboard`; R&C2 aims for you). The weapons whose retail code picks its own
//! target (thrown, homing) aim as retail does with Ratchet facing the camera heading (aim/facing.rs; table in the lane's HANDOFF.md).
//!
//! RAM: the classic Camera Options row record and the byte `pcext::STRAFE` keep their three values (Steady = byte 0, Raised), so a run
//! with the default writes exactly the RAM it wrote before; the classic page shows Raised while Steady is on. Off (either page) keeps the
//! choice before it, as Off keeps Shoulder's key, so the launcher's strafe on / off or a later Off -> on gives Steady back; Shoulder or
//! Raised picked on the classic page are plain (`keeps_flag`).
use super::*;
use crate::retail::pcopts::PcOptions;
use std::sync::atomic::AtomicU8;

/// The options file's value of `aimcam` for Steady.
const VALUE: &str = "steady";
const KEY: &str = "aimcam";
/// The modern row's values (Raised 0 and Shoulder 1 are the byte's own values).
pub const UI_STEADY: u8 = 2;
pub const UI_OFF: u8 = 3;
/// `MARK` empty.
const NO_MARK: u8 = 0xff;

static ON: AtomicBool = AtomicBool::new(false);
/// The strafe byte the modern row (or the options load) wrote last, until the change reaches `pcinput::set_strafe_row` (`keeps_flag`).
static MARK: AtomicU8 = AtomicU8::new(NO_MARK);

/// Steady is chosen. Byte Off still wins: every caller is behind `st.aiming` / aim.rs `active()`, which need the strafe row on.
pub fn on() -> bool {
	ON.load(Ordering::Relaxed)
}

pub fn set(on: bool) {
	ON.store(on, Ordering::Relaxed);
}

/// The options load (`pcinput::init`): the flag from the file, and the file's row byte counts as chosen there (a RAM image that already
/// holds that byte keeps the file's Steady).
pub fn load(opts: &PcOptions, row: u8) {
	set(from_opts(opts));
	MARK.store(row, Ordering::Relaxed);
}

/// The file says Steady (`aimcam=steady`; `strafe=off` with it is Off, the byte decides).
pub fn from_opts(opts: &PcOptions) -> bool {
	opts.extra_get(KEY).is_some_and(|v| v.trim().eq_ignore_ascii_case(VALUE))
}

/// The modern row picked `v` (`pcext/ui_bridge.rs`): `(byte, file changed)`. Raised / Shoulder / Steady set the flag and the file's
/// `aimcam=steady` (`set_from_ui`); Off leaves both, so it remembers Steady as it remembers Shoulder. The byte is marked as this row's.
pub fn row_set(opts: &mut PcOptions, v: u8) -> (u8, bool) {
	let (byte, steady) = from_ui(v);
	let save = byte != super::super::pcinput::STRAFE_OFF && set_from_ui(opts, steady);
	MARK.store(byte, Ordering::Relaxed);
	(byte, save)
}

/// The strafe row changed to `row` (`pcinput::set_strafe_row`): whether Raised keeps the Steady flag, i.e. the byte is the one the modern
/// row (or the options load) wrote. A change made on the classic Camera Options page is not marked: Raised picked there is plain Raised.
/// Every change takes the mark.
pub fn keeps_flag(row: u8) -> bool {
	MARK.swap(NO_MARK, Ordering::Relaxed) == row
}

/// Sets Steady on or off with the file's `aimcam` following: true = the file changed. Off only removes an `aimcam=steady` (Shoulder's
/// key is `pcinput::set_strafe_row`'s).
pub fn set_from_ui(opts: &mut PcOptions, steady: bool) -> bool {
	set(steady);
	key_change(opts, steady)
}

/// `set_from_ui`'s file part (the flag untouched).
fn key_change(opts: &mut PcOptions, steady: bool) -> bool {
	if from_opts(opts) == steady {
		return false;
	}
	opts.extra_set(KEY, steady.then(|| VALUE.to_string()));
	eprintln!("retail: PC options: strafe aiming Steady {}", if steady { "on" } else { "off" });
	true
}

/// The modern row's value from the byte and the flag: Raised 0, Shoulder 1, Steady 2, Off 3.
pub fn ui_value(byte: u8, steady: bool) -> u8 {
	match byte {
		0 if steady => UI_STEADY,
		0 | 1 => byte,
		_ => UI_OFF,
	}
}

/// The byte and the flag for a modern row value (out of range = Off).
pub fn from_ui(v: u8) -> (u8, bool) {
	match v {
		0 | 1 => (v, false),
		UI_STEADY => (super::super::pcinput::STRAFE_RAISED, true),
		_ => (super::super::pcinput::STRAFE_OFF, false),
	}
}

/// "Aiming" for the camera's look (the lift, Smooth's bias / probes / boom / follow / lock framing): the strafe aim is held and the row is
/// not Steady. Raised / Shoulder: exactly `st.aiming`.
pub(super) fn cam_aim(st: &St) -> bool {
	st.aiming && !on()
}

/// The "keyboard" input `camopts::lockon_on` gives Lock-on Auto (on with a pad): with Steady never, so Auto is on also with keyboard
/// and mouse.
pub fn auto_keyboard(keyboard: bool) -> bool {
	keyboard && !on()
}

/// `aimcam` for byte Raised (`pcinput::set_strafe_row`): Steady keeps its key, plain Raised has none.
pub fn raised_key(steady: bool) -> Option<String> {
	steady.then(|| VALUE.to_string())
}

/// The view aim.rs aims along (`aim_cam`): Raised / Shoulder unchanged; Steady = the camera heading made level from `chest` (the orbit
/// pivot's goal this field, Ratchet's chest: the orbit pivot itself lags it by up to ~1.6 m in Smooth), with the eye put on that line
/// behind it at the same distance.
pub(super) fn level(cam: AimCam, chest: [f32; 3]) -> AimCam {
	if on() { level_view(AimCam { pivot: chest, ..cam }) } else { cam }
}

fn level_view(cam: AimCam) -> AimCam {
	let h = cam.fwd[0].fhypot(cam.fwd[1]);
	if h < 1e-4 {
		return cam;
	}
	let fwd = [cam.fwd[0] / h, cam.fwd[1] / h, 0.0];
	let back = len3(sub(cam.pivot, cam.eye));
	AimCam { eye: [cam.pivot[0] - fwd[0] * back, cam.pivot[1] - fwd[1] * back, cam.pivot[2]], fwd, ..cam }
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::pcinput::{self, set_strafe_row, strafe_row};

	#[test]
	fn the_row_maps_both_ways() {
		for v in 0..=3u8 {
			let (b, s) = from_ui(v);
			assert_eq!(ui_value(b, s), v, "row value {v}");
		}
		assert_eq!(from_ui(9), (2, false), "out of range = Off");
		assert_eq!((ui_value(0, false), ui_value(1, false), ui_value(2, false), ui_value(2, true), ui_value(1, true)), (0, 1, 3, 3, 1), "without Steady the byte is the row");
	}

	#[test]
	fn the_file_key_follows() {
		let mut o = PcOptions::parse("");
		assert!(!from_opts(&o));
		assert!(key_change(&mut o, true) && from_opts(&o));
		assert!(!key_change(&mut o, true), "no change, no save");
		assert!(key_change(&mut o, false) && !from_opts(&o) && o.extra_get(KEY).is_none());
		let mut o = PcOptions::parse("aimcam=shoulder\n");
		assert!(!key_change(&mut o, false) && o.extra_get(KEY).is_some(), "Off leaves Shoulder's key alone");
		assert!(key_change(&mut o, true) && from_opts(&o), "Shoulder -> Steady");
		assert_eq!((raised_key(false), raised_key(true)), (None, Some(VALUE.to_string())));
	}

	/// The strafe byte in RAM, as `pcext::sync` sees it: the modern row (`row_set`, then `apply` -> `set_strafe_row` when the byte
	/// changed), the classic page (the byte, then `set_strafe_row`) and the options load (`pcinput::init`, `setup` writes the file's row).
	struct Sim {
		o: PcOptions,
		byte: u8,
	}
	impl Sim {
		fn load(text: &str) -> Sim {
			let o = PcOptions::parse(text);
			pcinput::init(&o);
			let byte = strafe_row(&o);
			Sim { o, byte }
		}
		fn modern(&mut self, v: u8) {
			let (b, _) = row_set(&mut self.o, v);
			if b != self.byte {
				self.classic(b);
			}
		}
		fn classic(&mut self, b: u8) {
			self.byte = b;
			set_strafe_row(&mut self.o, b);
		}
		/// The modern row's value, and the file as the launcher and the next start read it (without the `version=1` line).
		fn state(&self) -> (u8, u8, String) {
			let text = self.o.to_text();
			(ui_value(self.byte, on()), ui_value(strafe_row(&self.o), from_opts(&self.o)), text.trim_start_matches("version=1\n").into())
		}
	}

	#[test]
	fn off_remembers_and_classic_picks_are_plain() {
		let _g = crate::retail::pcopts::globals_guard();
		pcinput::init(&PcOptions::parse(""));
		let (r, sh, st, off) = (0u8, 1u8, UI_STEADY, UI_OFF);
		// Modern: Off keeps Steady (flag and key), and so does the launcher's strafe on / off; Off -> Steady / Raised / Shoulder.
		let mut s = Sim::load("");
		s.modern(st);
		assert_eq!(s.state(), (st, st, "aimcam=steady\n".into()), "Raised -> Steady: the byte stays, only the key");
		s.modern(off);
		assert_eq!(s.state(), (off, off, "strafe=off\naimcam=steady\n".into()), "Off from Steady keeps Steady");
		let mut l = Sim::load("strafe=on\naimcam=steady\n");
		assert_eq!(l.state().0, st, "the launcher's strafe=on gives Steady back");
		l.modern(off);
		l.modern(st);
		assert_eq!(l.state(), (st, st, "strafe=on\naimcam=steady\n".into()));
		s.modern(r);
		assert_eq!(s.state(), (r, r, "strafe=on\n".into()), "Off -> Raised is plain Raised");
		s.modern(sh);
		s.modern(off);
		assert_eq!(s.state(), (off, off, "strafe=off\naimcam=shoulder\n".into()), "Off from Shoulder keeps Shoulder");
		s.modern(sh);
		s.modern(st);
		assert_eq!(s.state(), (st, st, "strafe=on\naimcam=steady\n".into()), "Shoulder -> Steady");
		s.modern(r);
		s.modern(off);
		s.modern(r);
		assert_eq!(s.state(), (r, r, "strafe=on\n".into()), "Raised -> Off -> Raised");
		// Classic page: Off keeps Steady; Raised picked there is plain Raised; Shoulder drops Steady.
		let mut c = Sim::load("strafe=on\naimcam=steady\n");
		c.classic(2);
		assert_eq!(c.state(), (off, off, "strafe=off\naimcam=steady\n".into()), "classic Off keeps Steady");
		c.classic(0);
		assert_eq!(c.state(), (r, r, "strafe=on\n".into()), "classic Raised is plain Raised");
		let mut c = Sim::load("strafe=off\naimcam=steady\n");
		assert_eq!(c.state().0, off);
		c.classic(0);
		assert_eq!(c.state(), (r, r, "strafe=on\n".into()), "classic Off -> Raised after a file Off with Steady");
		let mut c = Sim::load("strafe=on\naimcam=steady\n");
		c.classic(1);
		c.classic(0);
		assert_eq!(c.state(), (r, r, "strafe=on\n".into()), "classic Shoulder -> Raised");
		// Modern Steady with a byte change (Off -> Steady) reaches set_strafe_row and keeps Steady; a stale mark from a modern Raised
		// <-> Steady (byte unchanged) is taken by the next classic change.
		let mut m = Sim::load("strafe=off\n");
		m.modern(st);
		assert_eq!(m.state(), (st, st, "strafe=on\naimcam=steady\n".into()), "Off -> Steady");
		let mut m = Sim::load("");
		m.modern(st);
		m.classic(2);
		m.classic(0);
		assert_eq!(m.state(), (r, r, "strafe=on\n".into()), "modern Steady, classic Off -> Raised");
		// Options load: a RAM image that already holds the file's byte (setup leaves it, the first sync applies it) keeps Steady.
		let mut f = Sim::load("strafe=on\naimcam=steady\n");
		f.classic(0);
		assert_eq!(f.state(), (st, st, "strafe=on\naimcam=steady\n".into()), "file Steady, RAM image byte Raised");
		pcinput::init(&PcOptions::parse(""));
		assert!(!on());
	}

	#[test]
	fn steady_aims_level_from_the_chest() {
		let cam = AimCam { eye: [-3.0, 0.0, 3.0], fwd: [0.8, 0.0, -0.6], pivot: [1.0, 0.0, 0.0], field: 7, shoulder: false };
		let l = level_view(cam);
		assert_eq!(l.fwd, [1.0, 0.0, 0.0]);
		assert_eq!((l.eye, l.pivot, l.field), ([-4.0, 0.0, 0.0], cam.pivot, 7), "eye on the level line, 5 m behind the chest");
		let up = AimCam { fwd: [0.0, 0.0, 1.0], ..cam };
		assert_eq!(level_view(up).fwd, up.fwd, "straight up: unchanged");
		let _g = crate::retail::pcopts::globals_guard();
		pcinput::init(&PcOptions::parse(""));
		assert_eq!(level(cam, [9.0; 3]).pivot, cam.pivot, "Raised / Shoulder (the flag is off outside the guarded tests): unchanged");
		let s = level_view(AimCam { pivot: [1.0, 0.0, 0.5], ..cam });
		assert_eq!((s.pivot, s.fwd, s.eye[2]), ([1.0, 0.0, 0.5], [1.0, 0.0, 0.0], 0.5), "Steady: the line starts at the chest goal");
		assert!(auto_keyboard(true) && !auto_keyboard(false), "without Steady, Auto reads the real input");
	}
}
