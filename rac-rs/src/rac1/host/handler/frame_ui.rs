//! Display-frame step of the modern Options page (v0.5; `modern_menu/host.rs` owns the `Ui`): opens it when the worker's swallow
//! (`retail/modern_swallow.rs`) queued a request, feeds it the pad and the repeat clock, hands the engine's UI pass the draw list
//! for this frame (`Gpu::ui_gpu.set_scene`, drawn by `viewer/gpu/ui_pass.rs`), plays its ticks, and closes it when the retail menu
//! it was opened from is gone. Before `frame_input`; nothing else runs on a frame without the page (one flag read).
use super::*;
use crate::modern_menu::host;
use crate::retail::modern_swallow;
use rac_engine::ui::{self, gpu as ui_gpu};

/// [PC] While the page is up over the pause menu, the retail overlay (menu panels, HUD, screen filter, panel previews) is not drawn:
/// the blurred backdrop is the world alone. Over the title nothing is hidden (the title's picture is the overlay). The same holds for a
/// screen with a modern page of its own (`modern_menu::hide`: pause, vendor, ship). The mods' draw layer (OmniLoader shapes, laid out
/// on the overlay's HUD) goes with the overlay: it would sit over the modern page where retail's HUD was. `frame_render`.
pub(super) fn hide_retail(g: &mut crate::viewer::Gpu) {
	use crate::viewer::{ITEM_PANEL_TEX, POPUP_PANEL_TEX, PORTRAIT_PANEL_TEX, RETAIL_FILTER_TEX, RETAIL_HD_TEX, RETAIL_OV_TEX};
	let h = crate::modern_menu::hide::HideSet::now();
	let portrait = g.hud.iter().find(|q| q.tex == PORTRAIT_PANEL_TEX).copied(); // [PC] the vendor's live portrait moves into the modern shop's card (vendor_portrait.rs)
	if h.overlay {
		g.hud.retain(|q| match q.tex {
			RETAIL_OV_TEX | RETAIL_HD_TEX | RETAIL_FILTER_TEX => false,
			ITEM_PANEL_TEX | POPUP_PANEL_TEX | PORTRAIT_PANEL_TEX => !h.targets,
			_ => true,
		});
		g.hud_split = None;
	}
	g.mods_hidden = h.overlay; // [PC] ... and so do the mods' shapes laid out on it (Arsenal Mastery's level bar under the ammo box ran through the shop title)
	// The HUD list's pixel space is the render size (`config`; prepare.rs maps it to clip space), not the window: with a Render scale
	// below 100 % or an upscaler (RAC_UPSCALE) window pixels put these quads off screen. Uniform scale, so the design rects still fit.
	let win = (g.config.width, g.config.height);
	crate::modern_menu::vendor_host::place_portrait(&mut g.hud, win, portrait); // [PC] ... and the vendor's portrait into the modern shop's card
	crate::modern_menu::pause_host::place_preview(&mut g.hud, win); // [PC] the modern Weapons / Gadgets / Items pages crop Ratchet (or the gold bolt) out of the overlay
	crate::modern_menu::ship_host::place_art(g, win); // [PC] the modern Galactic Map lays its galaxy painting under the page (and frees it again)
	crate::modern_menu::ship_host::place_planet(&mut g.hud, win); // [PC] ... and crops the game's planet picture out of the overlay into the page's card
	crate::modern_menu::map_host::place_crops(&mut g.hud, win); // [PC] the modern planet map crops the game's globe, map and mission pictures into its holes
}

impl App {
	pub(super) fn frame_ui(&mut self) {
		crate::modern_menu::host::watchdog(); // [PC] a modern page up with nothing drawn for 1.5 s is given up (modern_menu/watchdog.rs)
		self.frame_pause(); // [PC] the modern pause menu: decides, builds and draws its page (frame_pause.rs)
		self.frame_vendor(); // [PC] the modern vendor shop, the same way (frame_vendor.rs)
		self.frame_ship(); // [PC] the modern Galactic Map of the ship, the same way (frame_ship.rs)
		self.frame_map(); // [PC] the modern planet map and mission list, last: while the Map tab opens the map it wins over the closing pause page (frame_map.rs)
		let refused = ui_gpu::refused();
		modern_swallow::set_gpu_refused(refused);
		if let Some(a) = self.audio.as_mut() {
			ui::sound::drain(a);
		}
		let Some(r) = self.retail.as_ref() else { return };
		if r.front_ram().is_none() {
			crate::modern_menu::title_host::set_up(false); // the front end is over: the modern title page lets go of the keys and the pad
		}
		let was_open = ui::is_open();
		let request = modern_swallow::take_open();
		if !was_open && request.is_none() {
			return;
		}
		let pad = crate::retail::prompts::pad_peek().map(|p| host::pad_sample(p.buttons, p.sticks));
		if !was_open {
			// A request made while the UI pipes were refused (or the pack has no modern menu) opens nothing: the classic page is the fallback.
			if let Some(o) = request.filter(|_| !refused) {
				host::open(o, pad);
			}
		}
		// The page lives while the retail menu it was opened from is up (the pause menu, or the title's Options list).
		let menu_up = if host::from_title() { r.front_ram().is_some() } else { r.front_ram().is_none() && r.paused() };
		let win = self.gpu.as_ref().map_or((1280, 720), |g| g.win_size());
		let scene = host::step(win, pad, menu_up && !refused);
		if let (Some(prims), Some(g)) = (scene, self.gpu.as_mut()) {
			g.ui_gpu.set_scene(prims);
		}
	}
}
