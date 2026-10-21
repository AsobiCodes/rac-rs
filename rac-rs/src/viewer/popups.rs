use super::*;

/// The Quick Select fade counter (byte +0x70) runs 0..8 frames at 50 Hz while Triangle is held.
pub(super) const WHEEL_FADE: f32 = 8.0 / 50.0;

/// The pickup popup (ingame_menu_leftovers.md P1-b): white text at y 100 on the counter bar, fading in
/// and out; the shadow sits 1 px right and down.
pub(super) fn popup(out: &mut Vec<Quad>, w: f32, h: f32, art: &HudArt, font: &(usize, Vec<art::Glyph>), s: &str, a: f32) {
	let k = h / 448.0;
	let ox = (w - 512.0 * k) / 2.0;
	let lines = help::wrap(font, s, 424.0);
	let tw = lines.iter().map(|l| help::width(font, l)).fold(0.0, f32::max);
	let left = 256.0 - tw / 2.0;
	let y = 100.0;
	if art.bar.len() == 2 {
		// Bar from textLeft-32 at y-8, 32 high, alpha at most 0x50.
		let hgt = 32.0f32.max(lines.len() as f32 * 16.0 + 16.0);
		bar(out, art, ox + left * k, (y - 8.0 + hgt / 2.0) * k, tw * k, k, a.min(0x50 as f32 / 128.0));
	}
	let c = 0xf0 as f32 / 128.0;
	for (i, l) in lines.iter().enumerate() {
		let x = ox + (256.0 - help::width(font, l) / 2.0) * k;
		let ly = (y + i as f32 * 16.0) * k;
		help::text_line(out, font, l, x + k, ly + k, k, [0.0, 0.0, 0.0, a]);
		help::text_line(out, font, l, x, ly, k, [c, c, c, a]);
	}
}

/// The Quick Select wheel (FUN_00237e80, decomp/notes/ingame_other_screens.md s2): a 210x200 panel of
/// backdrop 0xe934 drawn as four mirrored quadrants, entry i at angle i*2pi/8 - pi/2 (0 = up, clockwise)
/// radius 74 (x stretched 1.05), the selected icon over highlight 0xe99c with a 30-frame pulse, the
/// others at half alpha, and the selected item's ammo and name. The panel's screen anchor is not
/// resolved: placed as in the retail PCSX2 capture, centre 146 square units from the left edge (kept
/// edge-anchored on wide screens), with a dark square behind every slot [I].
pub(super) fn wheel(out: &mut Vec<Quad>, w: f32, h: f32, art: &HudArt, font: &(usize, Vec<art::Glyph>), inv: &crate::inventory::Inventory, names: &[String], sel: usize, a: f32, clock: f32) {
	let k = h / 448.0;
	let ox = (w - 512.0 * k) / 2.0;
	let a = (a * 128.0).floor() / 128.0;
	let (cx, cy) = (146.0 - ox / k, 224.0);
	let px = |x: f32, y: f32, rw: f32, rh: f32| [ox + x * k, y * k, rw * k, rh * k];
	match art.wheel_bg {
		Some((slot, _, _)) => {
			for (qx, qy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
				let uv = [qx, qy, 1.0 - qx, 1.0 - qy];
				out.push(Quad { tex: slot, rect: px(cx - 105.0 + qx * 105.0, cy - 100.0 + qy * 100.0, 105.0, 100.0), uv, colour: [1.0, 1.0, 1.0, a], angle: 0.0, corners: None });
			}
		}
		None => overlay::rect(out, px(cx - 105.0, cy - 100.0, 210.0, 200.0), [0.0, 0.0, 0.0, 0.5 * a]),
	}
	let frame = clock * 50.0;
	for i in 0..8 {
		let ang = i as f32 * std::f32::consts::TAU / 8.0 - std::f32::consts::FRAC_PI_2;
		let (ex, ey) = (cx + 74.0 * ang.cos() * 1.05, cy + 74.0 * ang.sin());
		let id = inv.quick[i];
		overlay::rect(out, px(ex - 20.0, ey - 20.0, 40.0, 40.0), [40.0 / 255.0, 52.0 / 255.0, 100.0 / 255.0, 0.9 * a]);
		overlay::rect(out, px(ex - 18.0, ey - 18.0, 36.0, 36.0), [16.0 / 255.0, 22.0 / 255.0, 44.0 / 255.0, 0.95 * a]);
		if i == sel {
			if let Some((slot, _, _)) = art.wheel_sel {
				let pulse = (a * 128.0 * (0.875 + 0.125 * ((frame % 30.0) / 30.0 * std::f32::consts::TAU - std::f32::consts::PI).sin())).floor() / 128.0;
				out.push(sprite(slot, px(ex - 19.0, ey - 19.0, 38.0, 38.0), false, pulse));
			}
		}
		let ea = if i == sel { a } else { a * 0.5 };
		if let Some(&(slot, sw, sh)) = art.items.get(&id).filter(|_| id != 0) {
			out.push(sprite(slot, px(ex - sw / 2.0, ey - sh / 2.0, sw, sh), false, ea));
		}
	}
	// Selected entry: ammo "%d/%d" at y+12 (red when empty, else green) and the name split at '-' or ' '
	// in 0xff8080 with a black shadow; their x and the name's y are not resolved [I].
	let id = inv.quick[sel];
	if id == 0 {
		return;
	}
	let shadowed = |out: &mut Vec<Quad>, s: &str, y: f32, c: [f32; 3]| {
		let tw = help::width(font, s);
		help::text_line(out, font, s, ox + (cx - tw / 2.0 + 1.0) * k, (y + 1.0) * k, k, [0.0, 0.0, 0.0, a]);
		help::text_line(out, font, s, ox + (cx - tw / 2.0) * k, y * k, k, [c[0], c[1], c[2], a]);
	};
	if let Some(max) = crate::inventory::info(id).map(|i| i.max).filter(|&m| m > 0) {
		let n = inv.ammo(id);
		let c = if n == 0 { [0xff as f32 / 128.0, 0x40 as f32 / 128.0, 0x40 as f32 / 128.0] } else { [0x20 as f32 / 128.0, 0xe0 as f32 / 128.0, 0x40 as f32 / 128.0] };
		shadowed(out, &format!("{n}/{max}"), cy + 12.0, c);
	}
	if let Some(n) = names.get(sel) {
		let parts: Vec<&str> = n.splitn(2, |c| c == '-' || c == ' ').filter(|p| !p.is_empty()).collect();
		for (j, p) in parts.iter().enumerate() {
			shadowed(out, p, cy - 16.0 * (parts.len() - j) as f32 - 2.0, [1.0, 1.0, 0xff as f32 / 128.0]);
		}
	}
}

/// The save icon 0x755d, bottom right, spinning while a save runs [P position and spin].
pub(super) fn save_icon(out: &mut Vec<Quad>, w: f32, h: f32, art: &HudArt, clock: f32) {
	let k = h / 448.0;
	let ox = (w - 512.0 * k) / 2.0;
	match art.save.first() {
		Some(&(slot, sw, sh)) => {
			// A horizontal squash stands in for the spin.
			let sx = (clock * 4.0).cos().abs().max(0.1);
			let (cx, cy) = (512.0 - 48.0, 448.0 - 56.0);
			out.push(sprite(slot, [ox + (cx - sw * sx / 2.0) * k, (cy - sh / 2.0) * k, sw * sx * k, sh * k], false, 1.0));
		}
		None => overlay::rect(out, [ox + 448.0 * k, 376.0 * k, 32.0 * k, 32.0 * k], [0.9, 0.9, 0.2, 0.8]),
	}
}

/// Select-map zoom limits; the default shows the whole map in the panel [I].
pub(super) const MAP_ZOOM: (f32, f32) = (0.65, 4.0);

/// The Galactic Map texture fitted into the screen with a blinking hero marker.
pub(super) fn map_view(out: &mut Vec<Quad>, w: f32, h: f32, mv: &MapView, marker: Option<(usize, f32, f32)>, clock: f32) {
	let k = h / 448.0;
	let ox = (w - 512.0 * k) / 2.0;
	// The square map fitted into the map panel (slot 2 of page 0x1b3518) [I].
	let p = frontend::MAP_PANEL;
	let side = p[2].min(p[3]) - 8.0;
	let (x, y) = (p[0] + (p[2] - side) / 2.0, p[1] + (p[3] - side) / 2.0);
	// The zoomed window of the map about `pan`, cropped to the panel.
	let half = 256.0 * MAP_ZOOM.0 / mv.zoom;
	let (u0, v0) = (mv.pan[0] - half, mv.pan[1] - half);
	let uv = [u0 / 512.0, v0 / 512.0, (u0 + 2.0 * half) / 512.0, (v0 + 2.0 * half) / 512.0];
	out.push(Quad { tex: mv.slot, rect: [ox + x * k, y * k, side * k, side * k], uv, colour: [1.0, 1.0, 1.0, 1.0], angle: 0.0, corners: None });
	let scale = side / (2.0 * half);
	let (px, py) = (x + (mv.hero[0] - u0) * scale, y + (mv.hero[1] - v0) * scale);
	let inside = (0.0..=side).contains(&(px - x)) && (0.0..=side).contains(&(py - y));
	match marker {
		_ if !inside => {}
		// Size (((zoom*4+10)*0.75)/13)*256 sixteenths of a pixel (11.6 px at 0.65, 24 px at 4.0).
		// Rotated by heading: world +x is screen right and +y screen up, the sprite is taken to point
		// up at angle 0 [I].
		Some((t, _, _)) => {
			let s = ((mv.zoom * 4.0 + 10.0) * 0.75 / 13.0) * 256.0 / 16.0;
			out.push(Quad { tex: t, rect: [ox + (px - s / 2.0) * k, (py - s / 2.0) * k, s * k, s * k], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0, 1.0, 1.0, 1.0], angle: std::f32::consts::FRAC_PI_2 - mv.yaw, corners: None });
		}
		None if (clock * 3.0).fract() < 0.6 => overlay::rect(out, [ox + (px - 3.0) * k, (py - 3.0) * k, 6.0 * k, 6.0 * k], [1.0, 0.2, 0.2, 1.0]),
		None => {}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

}
