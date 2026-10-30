//! Button glyphs option (retail/glyphs.rs) on the viewer's own copy of the three font sheets: the title and front-end menus, the front
//! dialogs and the mods' text (omni/draw_art.rs) draw `HudArt::font*` textures decoded from the level, not the GS capture's sheet.
//! `push` puts the cells of the style now wanted into a font texture as it is created and keeps the decoded original; `refresh`
//! (every front-end frame, `front_mirror_frame`, and every level frame, `frame_render`) redoes the three textures when the style or a pad binding changed, so the option and
//! a rebind show on the title page the moment they are set. PlayStation without rebinds = the decoded sheet, untouched.
use crate::level::Texture;
use crate::retail::binds::NP;
use crate::retail::glyphs as gl;
use crate::viewer::Gpu;
use std::sync::Mutex;

/// The decoded sheets of the level on screen, by HUD texture slot.
static ORIG: Mutex<Vec<(usize, Texture)>> = Mutex::new(Vec::new());
/// What the textures in the slots show now.
static SHOWN: Mutex<Option<(u8, [u8; NP])>> = Mutex::new(None);

/// A new level's HUD art starts: forget the old level's sheets.
pub(super) fn begin() {
	ORIG.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// The texture of `slot` as `t` is created: remembered, and drawn with the cells wanted now. Returns the texture to upload.
pub(super) fn dress(slot: usize, t: Texture) -> Texture {
	let want = gl::wanted();
	let out = restyled(&t, &want);
	ORIG.lock().unwrap_or_else(|e| e.into_inner()).push((slot, t));
	*SHOWN.lock().unwrap_or_else(|e| e.into_inner()) = Some(want);
	out
}

/// `orig` with the cells of `want` (a copy of it for retail's own).
fn restyled(orig: &Texture, want: &(u8, [u8; NP])) -> Texture {
	let mut live = Texture { w: orig.w, h: orig.h, rgba: orig.rgba.clone() };
	if !gl::is_retail(want) {
		gl::blit(orig, &mut live, want.0, &want.1);
	}
	live
}

/// The style or the bindings changed since the textures were made: upload them again.
pub(in crate::viewer) fn refresh(gpu: &mut Gpu) {
	let want = gl::wanted();
	let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
	if shown.is_none_or(|s| s == want) {
		return;
	}
	*shown = Some(want);
	for (slot, orig) in ORIG.lock().unwrap_or_else(|e| e.into_inner()).iter() {
		gpu.hud_texture(Some(*slot), &restyled(orig, &want));
	}
}
