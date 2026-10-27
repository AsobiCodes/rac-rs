use super::*;

/// Decodes the HUD sprites (bolt icon 0x754f, counter bar 0x7580, nanotech orb 0x7536) and the
/// default font; returns the textures in slot order. Missing pieces fall back to plain shapes.
pub(super) fn hud_art(l: &Level, disc: &mut iso::Disc) -> (Vec<Texture>, HudArt) {
	let (mut tex, mut a) = (Vec::new(), HudArt::default());
	match art::hud_sprites(l) {
		Ok(mut h) => {
			eprintln!("hud icons: {:x?}", h.icons.iter().map(|i| i.id).collect::<Vec<_>>());
			for (id, n, out) in [(0x753f, 3, &mut a.breath), (0x7533, 2, &mut a.morph), (0x754f, 30, &mut a.bolt), (0x7580, 2, &mut a.bar), (0x7536, 32, &mut a.orb), (0xea60 + 10, 1, &mut a.glove), (0x755d, 1, &mut a.save)] {
				let Some(i) = h.icons.iter().find(|i| i.id == id).copied() else { continue };
				for k in 0..n.min(i.count) {
					if let Some(sp) = h.frames.get_mut(i.first + k).and_then(|f| f.take()) {
						out.push((1 + tex.len(), sp.tex.w as f32, sp.tex.h as f32));
						tex.push(sp.tex);
					}
				}
			}
		}
		Err(e) => eprintln!("hud art: {e}"),
	}
	// The panel underlay 0xe99e frame 7 comes from the resident global HUD (128x128, black RGB, an alpha
	// vignette), so the title level has it too (decomp/notes/title_assets.md).
	let mut underlay = crate::hud::global_hud_sprites(disc).map_err(|e| eprintln!("global hud: {e}")).ok().and_then(|mut g| {
		let i = g.icons.iter().find(|i| i.id == 0xe99e).copied().filter(|i| 7 < i.count)?;
		let sp = g.frames.get_mut(i.first + 7).and_then(|f| f.take())?;
		tex.push(sp.tex);
		let t = &tex[tex.len() - 1];
		Some((tex.len(), t.w as f32, t.h as f32))
	});
	if let Ok(mut h) = art::hud_sprites(l) {
		let mut take = |id: u16, k: usize| {
			let i = h.icons.iter().find(|i| i.id == id).copied().filter(|i| k < i.count)?;
			let sp = h.frames.get_mut(i.first + k).and_then(|f| f.take())?;
			tex.push(sp.tex);
			let t = &tex[tex.len() - 1];
			Some((tex.len(), t.w as f32, t.h as f32))
		};
		a.vol = [take(0xe99e, 8), take(0xe99e, 9)];
		if underlay.is_none() {
			underlay = take(0xe99e, 7);
		}
		a.wheel_bg = take(0xe934, 0);
		a.wheel_sel = take(0xe99c, 0);
		a.marker = take(0xe99a, 5);
		a.counters = [take(2000, 0), take(2001, 0)];
		a.pics.arrow = take(0xe99e, 0);
		a.pics.tick = take(0xe99e, 1);
		// Galactic Map sprites 0xe99a frames 0xc..0xf (marker, ring, backdrop, stars).
		for k in 0..4 {
			a.pics.galaxy[k] = take(0xe99a, 0xc + k);
		}
	}
	// Global pictures straight from the disc ToC (help_ss 0x488, options_ss 0x4c0) and the map
	// bundle's globe layers (decomp/notes/ingame_pause_menu.md "Pause presentation").
	if let Ok(toc) = toc::read_toc(disc) {
		for i in 0..19 {
			a.pics.planets.push(frontend::planet_picture(disc, &toc, i).map_err(|e| eprintln!("planet picture {i}: {e}")).ok().map(|t| {
				tex.push(t);
				tex.len()
			}));
		}
		for (base, n, out) in [(frontend::HELP_SS, 5, &mut a.pics.help), (frontend::OPTIONS_SS, 7, &mut a.pics.options), (frontend::HELP_GADGETS, 14, &mut a.pics.gadget_help)] {
			for i in 0..n {
				out.push(frontend::ss_picture(disc, &toc, base, i).map_err(|e| eprintln!("ss picture {base:#x}/{i}: {e}")).ok().map(|t| {
					tex.push(t);
					tex.len()
				}));
			}
		}
	}
	// The menu panel slots (class 0x472, in the title WAD and every level; decomp/notes/title_assets.md).
	a.pics.panels = crate::anim::animation(l, 0x472).map_err(|e| eprintln!("menu panels: {e}")).ok().flatten().map(std::sync::Arc::new);
	// Every planet's globe: the Select map shows this level's, the ship's details page the selected one's.
	for p in 0..19 {
		let g = map::load_globe(disc, p).map_err(|e| eprintln!("globe {p}: {e}")).ok().flatten();
		a.pics.globes.push(g.map(|layers| {
			layers.map(|t| {
				tex.push(t);
				tex.len()
			})
		}));
	}
	if let Some(&Some(layers)) = a.pics.globes.get(l.id) {
		a.pics.globe = Some(layers);
		a.pics.globe_inset = map::GLOBE_INSET.get(l.id).copied().unwrap_or(0);
		a.pics.globe_planet = l.id;
	}
	if let Ok(mut h) = art::hud_sprites(l) {
		if let Some(i) = h.icons.iter().find(|i| i.id == 0xe935).copied() {
			if let Some(sp) = h.frames.get_mut(i.first).and_then(|f| f.take()) {
				a.led = Some((1 + tex.len(), sp.tex.w as f32, sp.tex.h as f32));
				tex.push(sp.tex);
			}
		}
	}
	let mut fx = |k: usize| {
		let t = frontend::fx_texture(l, k).map_err(|e| eprintln!("menu frame fx {k:#x}: {e}")).ok()?;
		let (w, h) = (t.w as f32, t.h as f32);
		tex.push(t);
		Some((tex.len(), w, h))
	};
	let parts = [underlay.or(Some((frontend::NO_UNDERLAY, 0.0, 0.0))), fx(0x1a), fx(0x1c), fx(0x19)];
	a.static_tex = fx(0x1e);
	a.screen_fx = std::array::from_fn(|i| fx(0x21 + i));
	a.pad_fx = crate::game::PAD_FX.map(|n| fx(n as usize));
	a.frame = parts.iter().all(Option::is_some).then(|| parts.map(Option::unwrap));
	match frontend::fx_texture(l, 4) {
		Ok(t) => {
			tex.push(t);
			a.help_icon = Some(tex.len());
		}
		Err(e) => eprintln!("help icon: {e}"),
	}
	if let Ok(mut h) = art::hud_sprites(l) {
		for id in 0..crate::inventory::GADGETS as u8 {
			let Some(i) = h.icons.iter().find(|i| i.id == 0xea60 + id as u16).copied() else { continue };
			if let Some(sp) = h.frames.get_mut(i.first).and_then(|f| f.take()) {
				a.items.insert(id, (1 + tex.len(), sp.tex.w as f32, sp.tex.h as f32));
				tex.push(sp.tex);
			}
			if let Some(sp) = h.frames.get_mut(i.first + 1).filter(|_| i.count > 1 && frontend::GADGET_GRID_IDS.contains(&id)).and_then(|f| f.take()) {
				a.pics.worn.insert(id, (1 + tex.len(), sp.tex.w as f32, sp.tex.h as f32));
				tex.push(sp.tex);
			}
			if let Some(sp) = h.frames.get_mut(i.first + 4).filter(|_| i.count > 4).and_then(|f| f.take()) {
				a.pics.gold.insert(id, (1 + tex.len(), sp.tex.w as f32, sp.tex.h as f32));
				tex.push(sp.tex);
			}
			if let Some(sp) = h.frames.get_mut(i.first + 2).filter(|_| i.count > 2).and_then(|f| f.take()) {
				a.ammo_items.insert(id, (1 + tex.len(), sp.tex.w as f32, sp.tex.h as f32));
				tex.push(sp.tex);
			}
			if let Some(sp) = h.frames.get_mut(i.first + 3).filter(|_| i.count > 3).and_then(|f| f.take()) {
				a.weapon_icons.insert(id, (1 + tex.len(), sp.tex.w as f32, sp.tex.h as f32));
				tex.push(sp.tex);
			}
		}
	}
	// The Items grid icons (frontend::ITEM_GRID): 0xea7e frame 0 is item 30's icon, taken above.
	if let Ok(mut h) = art::hud_sprites(l) {
		for (j, &(icon, frame, ..)) in frontend::ITEM_GRID.iter().enumerate() {
			let Some(i) = h.icons.iter().find(|i| i.id == icon).copied().filter(|i| (frame as usize) < i.count) else { continue };
			if let Some(sp) = h.frames.get_mut(i.first + frame as usize).and_then(|f| f.take()) {
				a.pics.items_grid[j] = Some((1 + tex.len(), sp.tex.w as f32, sp.tex.h as f32));
				tex.push(sp.tex);
			}
		}
	}
	a.pics.items_grid[0] = a.pics.items_grid[0].or(a.items.get(&0x1e).copied());
	// The glove's icon was taken above.
	if let Some(&g) = a.glove.first() {
		a.items.insert(crate::inventory::BOMB_GLOVE, g);
	}
	match art::font_glyph_tables(disc).and_then(|g| art::level_fonts(l, &g)) {
		Ok(mut fonts) if !fonts.is_empty() => {
			let mut f = fonts.remove(0);
			// The button glyphs style: the Xbox artwork goes into the sheet's ten button cells. The sheet as it was decoded
			// is kept, so the style can be changed back (the PlayStation style takes the retail's own symbols out of it).
			a.font_tex_orig = Some(f.tex.clone());
			let map = crate::retail::binds::pad_map();
			crate::retail::xglyphs::blit(a.font_tex_orig.as_ref().unwrap(), &mut f.tex, crate::retail::pcext::glyph_style_now(), &map);
			a.font = Some((1 + tex.len(), f.glyphs));
			a.font_tex = Some(f.tex.clone());
			tex.push(f.tex);
			// NOV-14.2b: font 3 (the last of the three level fonts).
			if let Some(f3) = fonts.pop() {
				a.font3 = Some((1 + tex.len(), f3.glyphs));
				tex.push(f3.tex);
			}
			// Font 2 (the shop panels' text).
			if let Some(f2) = fonts.pop() {
				a.font2 = Some((1 + tex.len(), f2.glyphs));
				tex.push(f2.tex);
			}
		}
		r => eprintln!("hud font: {:?}", r.err()),
	}
	// The retail's own prompts (the in-game tutorial text, the Quick Select ring, the paged menus) are drawn out
	// of the graphics memory, where the sheet is 8-bit indexed: the artwork goes into the sheet's ten button cells
	// there too (the capture recognises the sheet's upload by its content).
	if let Ok(g) = art::font_glyph_tables(disc) {
		if let Some(p) = crate::retail::xglyphs::patch_for(l, &g) {
			crate::retail::xglyphs::set_patch(p);
		}
	}
	(tex, a)
}
