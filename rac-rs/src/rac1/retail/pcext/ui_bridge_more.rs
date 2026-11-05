//! The modern screen's rows that belong to a module's own setter (`ui_bridge.rs` hands them over): Renderer, Skip intro, Start skips
//! cutscenes, Quick Select mouse, Stick dead zone, Button glyphs, Wrench throw, Upscaler and Upscale quality (no RAM
//! byte: the engine's `upscale_choice`, options file only), Underwater look (no RAM byte: the engine's `under_choice`), Aspect ratio
//! (no RAM byte: the engine's `aspect_box`), the
//! texture filter the presets send (no row of its own), and the retail Left / Right and Up / Down bytes. Each setter writes
//! the classic page's row byte too when that page exists on this RAM (else its `sync` would read the differing byte as a press and
//! undo the change) and returns whether the options file changed. Quick Select pauses the game (qs_pause.rs) and Auto-equip gadgets
//! (auto_gadget.rs) have no RAM byte and no classic row: options file only.
use super::*;
use crate::retail::{glyphs, pcinput, pcintro, pcrender, pcskip, wrench_throw};
use rac_engine::viewer::{aspect_box, under_choice, upscale_choice};

/// The first two records of the Camera Options page's row copy at `TAB`, as retail has them: `{label, byte*, Reversed text,
/// Normal text, 0, 0}` for Left / Right (k = 0) and Up / Down (k = 1). The byte is 0 = Reversed, 1 = Normal; boot default 1.
/// [C] decomp/notes/ingame_menu_leftovers.md (P1-d, "Camera page"); the addresses are the same on both discs (`SIG` is checked on the
/// PAL and the USA table alike, `modcam.rs` reads them as constants).
pub(super) const INVERT: [[u32; 6]; 2] = [[0x4fa2, 0x15ede0, 0x4fa5, 0x4fa4, 0, 0], [0x4fa3, 0x15eddc, 0x4fa5, 0x4fa4, 0, 0]];

/// The retail Left / Right (k = 0) and Up / Down (k = 1) byte of the Options -> Camera Options page, once the copy at `TAB` holds the
/// whole retail record (all six words, so a wrong or half-patched copy never makes this write anywhere else). None until then.
///
/// Why a direct byte write is the same as the game's own toggle [C]: the retail row is a multi-choice item (FUN_0027e898), and its
/// Cross does `*byte = (*byte + 1) % n` with n = the number of non-zero choice texts of the record (2 here): one byte, the values
/// 0 and 1, nothing else (no sound, no side effect; the camera reads the byte every tick). The page cannot be open on the retail
/// side while the modern screen is, so nothing races the write.
fn invert_addr(mem: &mut Mem, k: u32) -> Option<u32> {
	let want = INVERT[k as usize];
	let rec = TAB + 24 * k;
	want.iter().zip(0u32..).all(|(&w, i)| mem.rd32(rec + 4 * i) == w).then_some(want[1])
}

/// Worker, from `ui_bridge::other`: row `id` set to `v`. Some(true) = the options file changed (save), Some(false) = applied or
/// nothing to do, None = not one of these rows.
pub(super) fn set(mem: &mut Mem, s: &mut St, id: &str, v: i32) -> Option<bool> {
	let b = v.clamp(0, 255) as u8;
	let on = b != 0;
	Some(match id {
		"renderer" => pcrender::set_from_ui(mem, &mut s.opts, on),
		"intro" => pcintro::set_from_ui(mem, &mut s.opts, on),
		"skipcut" => pcskip::set_from_ui(mem, &mut s.opts, on),
		"wheel" => pcinput::set_wheel_from_ui(mem, &mut s.opts, on),
		"stickdz" => pcinput::set_stick_from_ui(mem, &mut s.opts, b),
		"glyphs" => glyphs::set_from_ui(mem, &mut s.opts, b),
		// No row: every modern preset sends it (`UiMenu::preset_also`), as the classic `presets::apply_preset` writes it.
		"texfilter" => {
			texfilter::set(on);
			s.opts.texfilter.replace(on) != Some(on)
		}
		"wrenchthrow" => wrench_throw::set_from_ui(&mut s.opts, on),
		"upscaler" => upscale_choice::set_kind(&mut s.opts, v),
		"upscale_q" => upscale_choice::set_quality(&mut s.opts, v),
		"underwater" => under_choice::set_from_ui(&mut s.opts, on),
		"aspect" => aspect_box::set_from_ui(&mut s.opts, v),
		"qspause" => crate::retail::qs_pause::set_from_ui(&mut s.opts, on),
		"autogadget" => crate::retail::auto_gadget::set_from_ui(&mut s.opts, on),
		"leftright" | "updown" => {
			if let Some(a) = invert_addr(mem, (id == "updown") as u32) {
				mem.wr8(a, 1 - on as u8);
			}
			false
		}
		_ => return None,
	})
}

/// Worker, from `ui_bridge::publish`: the current values of these rows (the Left / Right and Up / Down ones read 0 = Normal until
/// the page copy exists).
pub(super) fn publish(mem: &mut Mem, v: &mut Vec<(&'static str, i32)>) {
	let inv = |mem: &mut Mem, k: u32| invert_addr(mem, k).map_or(0, |a| (mem.rd8(a) == 0) as i32);
	let (lr, ud) = (inv(mem, 0), inv(mem, 1));
	v.extend([
		("renderer", pcrender::wanted_dx12() as i32),
		("intro", pcintro::skip() as i32),
		("skipcut", pcskip::skip() as i32),
		("wheel", pcinput::wheel_mouse() as i32),
		("stickdz", pcinput::stick_byte() as i32),
		("glyphs", glyphs::byte() as i32),
		("wrenchthrow", wrench_throw::moving() as i32),
		("upscaler", upscale_choice::kind() as i32),
		("upscale_q", upscale_choice::quality() as i32),
		("underwater", under_choice::on() as i32),
		("aspect", aspect_box::current() as i32),
		("qspause", crate::retail::qs_pause::on() as i32),
		("autogadget", crate::retail::auto_gadget::on() as i32),
		("leftright", lr),
		("updown", ud),
	]);
}
