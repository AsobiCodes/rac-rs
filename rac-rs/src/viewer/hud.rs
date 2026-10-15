use super::*;

/// The counter bar: end caps (frame 1, the right one flipped) around a stretched middle (frame 0).
pub(super) fn bar(out: &mut Vec<Quad>, art: &HudArt, left: f32, y: f32, inner: f32, k: f32, a: f32) {
	let (top, hgt) = (y - 16.0 * k, 32.0 * k);
	out.push(sprite(art.bar[1].0, [left - 32.0 * k, top, 32.0 * k, hgt], false, a));
	out.push(sprite(art.bar[0].0, [left, top, inner, hgt], false, a));
	out.push(sprite(art.bar[1].0, [left + inner, top, 32.0 * k, hgt], true, a));
}

/// One 50 Hz tick of the nanotech meter (init FUN_00239190, update FUN_00239210, slot tick): state is
/// (lifetime, f1, f2, displayed value). The lifetime counts down first; a changed value (or a displayed 1)
/// reloads it to scale(120) = 100; below scale(5) = 4 the orbs (f2) then the bar (f1) fade out, otherwise
/// the bar fades in to 8 and then the orbs.
pub(super) fn nano_step((life, f1, f2, shown): (i32, u8, u8, i32), health: i32) -> (i32, u8, u8, i32) {
	let value = health.clamp(0, 4);
	let (mut life, mut f1, mut f2, mut shown) = if shown < 0 { (crate::gameplay::pal_ticks(180) + 30, 0, 0, value) } else { (life, f1, f2, shown) };
	if life > 0 {
		life -= 1;
	}
	if shown != value || shown == 1 {
		life = crate::gameplay::pal_ticks(120);
		shown = value;
	}
	if life < crate::gameplay::pal_ticks(5) {
		if f2 > 0 {
			f2 -= 1;
		} else if f1 > 0 {
			f1 -= 1;
		}
	} else if f1 < 8 {
		f1 += 1;
	} else if f2 < 8 {
		f2 += 1;
	}
	(life, f1, f2, shown)
}

/// NOV-30f nano request latch, once per frame before the steps: the shop (FUN_0024aa78) empties slot 1 and drops the
/// latch; on foot a not-live slot is requested again only by damage (FUN_00226a70 -> FUN_0024a000), i.e. `health` below
/// the previous frame's `prev`. A fresh request starts from `(0, 0, 0, -1)`, so `nano_step` runs its init.
pub(super) fn nano_request(live: &mut bool, nano: &mut (i32, u8, u8, i32), in_shop: bool, health: i32, prev: i32) {
	if in_shop {
		*live = false;
		*nano = (0, 0, 0, -1);
	} else if !*live && health < prev {
		*live = true;
	}
}

/// One 50 Hz nanotech tick under the latch: `nano_step` (lifetime, the reload on a value change OV:41237-41241, the
/// fade); when the lifetime is still below scale(5) and both fade bytes were already 0 the slot is released instead
/// (FUN_0024dda0 OV:41243-41252: the update tick after both bytes reach 0). Nothing runs while not live or in the shop.
pub(super) fn nano_tick(live: &mut bool, nano: &mut (i32, u8, u8, i32), in_shop: bool, health: i32) {
	if !*live || in_shop {
		return;
	}
	let next = nano_step(*nano, health);
	if nano.3 >= 0 && nano.1 == 0 && nano.2 == 0 && next.0 < crate::gameplay::pal_ticks(5) {
		*live = false;
		*nano = (0, 0, 0, -1);
		return;
	}
	*nano = next;
}

/// The nanotech bar for fade byte `f1`: (half width in PS2 px, alpha) = (trunc(48 * f1 / 8), trunc(20 * f1 / 8) / 128).
pub(super) fn nano_bar(f1: u8) -> (i32, f32) {
	let f = f1 as f32 / 8.0;
	((48.0 * f) as i32, (20.0 * f).trunc() / 128.0)
}

/// NOV-14.2 RC range static (FUN_00301d68, decomp/notes/novalis_checklist.md): 17 x 15 tiles of 32 PS2 px at
/// (col * 32 - 16, row * 32 - 16), texel rect (u, v, u + 32, v + 32) of fx texture 0x1e, vertex colour 0x7f/0x7f/0xff
/// (GS 0x80 = 1.0) with alpha `a`. The 512 x 448 screen is stretched over the window [PC: widescreen].
pub(super) fn rc_static(out: &mut Vec<Quad>, (slot, tw, th): (usize, f32, f32), a: u8, tiles: &[[u8; 2]], w: f32, h: f32) {
	let (sx, sy) = (w / 512.0, h / 448.0);
	for (i, t) in tiles.iter().enumerate().take(17 * 15) {
		let (col, row) = ((i / 15) as f32, (i % 15) as f32);
		let (u, v) = (t[0] as f32, t[1] as f32);
		let uv = [u / tw, v / th, (u + 32.0) / tw, (v + 32.0) / th];
		out.push(Quad { tex: slot, rect: [(col * 32.0 - 16.0) * sx, (row * 32.0 - 16.0) * sy, 32.0 * sx, 32.0 * sy], uv, colour: [127.0 / 128.0, 127.0 / 128.0, 255.0 / 128.0, a as f32 / 128.0], angle: 0.0, corners: None });
	}
}

/// NOV-14.2b marker prompt layout (draw FUN_0024ea48): (bar extra half width, bar alpha, icon alpha) in GS units
/// (0x80 = 1.0) for the ramps (a, b).
pub(super) fn marker_layout(a: u8, b: u8) -> (i32, u32, u32) {
	let fa = (a as f32 / 8.0).clamp(0.0, 1.0);
	let fb = (b as f32 / 8.0).clamp(0.0, 1.0);
	((16.0 * fa) as i32, (80.0 * fa) as u32, (fb * 128.0) as u32)
}

/// NOV-14.2b "!" pulse: `max(0, sin((frames % S) / S * 6.28318 - 3.14159) * 4 - 3) * fb`, S = scale(45) = 37.
pub(super) fn marker_pulse(frames: u32, b: u8) -> f32 {
	let fb = (b as f32 / 8.0).clamp(0.0, 1.0);
	let s = crate::gameplay::pal_ticks(45) as u32;
	let x = (frames % s) as f32 / s as f32 * 6.28318 - 3.14159;
	(x.sin() * 4.0 - 3.0).max(0.0) * fb
}

/// The breath meter (HUD slot 4, hud.md "S1 breath HUD"): a backing rectangle, the aquarium sprite (frame 1 while the
/// low-air blink is on) and the bubbles of frame 2 that sit at or above the water line of the smoothed value. PS2
/// coordinates are right-anchored like the other slot widgets and PS2 x is scaled by `sx` like them; bubbles use the
/// draw's 1/16 pixel fixed point. The 256x64 tank frame is drawn rotated 90 degrees counter-clockwise (FUN_00250238,
/// hud.md "HUD-BREATH-TANK port spec").
pub(super) fn breath_meter(out: &mut Vec<Quad>, art: &HudArt, s: &game::breath_hud::BreathSnapshot, w: f32, k: f32) {
	use game::breath_hud::{radius, HIDE, SHOW};
	if art.breath.len() != 3 {
		return;
	}
	let sx = 448.0f32 * 4.0 / 3.0 / 512.0;
	let px = |x: f32| w - (512.0 - x) * sx * k;
	let curve = f32::from_bits(if s.lifetime != 0 { SHOW } else { HIDE }[s.slide.clamp(0, 23) as usize]);
	let off = (curve * (42.0 + 20.0) + 0.5) as i32;
	// Right-aligned, vertically centred anchor (492, 208) for a 42 x 192 box.
	let (x, y) = (450 + off, 112);
	// Backing: top-left (x*16 + 42*208/64, y*16 + 192*160/256), size (42*608/64, 192*3744/256) in 1/16 px.
	let (bx, by) = ((x * 16 + 42 * 208 / 64) as f32 / 16.0, (y * 16 + 192 * 160 / 256) as f32 / 16.0);
	let (bw, bh) = ((42 * 608 / 64) as f32 / 16.0, (192 * 3744 / 256) as f32 / 16.0);
	overlay::rect(out, [px(bx), by * k, bw * sx * k, bh * k], [0x2f as f32 / 255.0, 0x4c as f32 / 255.0, 0x68 as f32 / 255.0, 1.0]);
	let frame = &art.breath[usize::from(s.blink)];
	// Rotated tank: the pre-rotation rect is centred on the box centre (x+21, y+96) with the axes swapped; -pi/2 turns
	// it counter-clockwise (texture right end at the top). GS colour 0x7f7f7f.
	let (cx, cy) = (px(x as f32 + 21.0), (y as f32 + 96.0) * k);
	let c = 127.0 / 128.0;
	out.push(Quad {
		tex: frame.0,
		rect: [cx - 96.0 * k, cy - 21.0 * sx * k, 192.0 * k, 42.0 * sx * k],
		uv: [0.0, 0.0, 1.0, 1.0],
		colour: [c, c, c, 1.0],
		angle: -std::f32::consts::FRAC_PI_2,
		corners: None,
	});
	let loss = (game::breath_hud::MAX - s.display) as f32 / game::breath_hud::MAX as f32;
	let cut = 116.0 - (loss + loss) * 116.0;
	let xscale = (42.0f32 * 0.5 / 2.0) / 17.0;
	let yscale = -(192.0f32 * 0.9 / 2.0) / 116.0;
	for (i, b) in s.bubbles.iter().enumerate() {
		let r = radius(i);
		let (ox, oy) = (((x as f32 + 21.0) - r) * 16.0, ((y as f32 + 96.0) - r) * 16.0);
		if b[1] > cut + r {
			continue;
		}
		let left = ((b[0] * xscale) * 16.0 + ox).trunc();
		let top = ((b[1] * yscale) * 16.0 + oy).trunc();
		let size = r * 2.0;
		let mut q = sprite(art.breath[2].0, [px(left / 16.0), top / 16.0 * k, size * sx * k, size * k], false, 1.0);
		q.colour = [c, c, c, 1.0];
		out.push(q);
	}
}

/// WPN-6.7e4 Morph-o-Ray meter draw FUN_0024c050 (decomp/notes/weapons_framework.md "WPN-6.7e4"): HUD slot 4 as the
/// breath meter (same anchor, slide and bubble array), sheet 0x7533. No backing rectangle and no blink: tank frame 0 as
/// the breath meter's rotated quad (FUN_00250238, hud.md "HUD-BREATH-TANK"), then every bubble as frame 1 at the
/// radius-6 size and origin (only the i == 0 adjustment runs) with the cut + 6.
pub(super) fn morph_meter(out: &mut Vec<Quad>, art: &HudArt, s: &game::breath_hud::BreathSnapshot, w: f32, k: f32) {
	use game::breath_hud::{HIDE, SHOW};
	if art.morph.len() != 2 {
		return;
	}
	let sx = 448.0f32 * 4.0 / 3.0 / 512.0;
	let px = |x: f32| w - (512.0 - x) * sx * k;
	let curve = f32::from_bits(if s.lifetime != 0 { SHOW } else { HIDE }[s.slide.clamp(0, 23) as usize]);
	let off = (curve * (42.0 + 20.0) + 0.5) as i32;
	let (x, y) = (450 + off, 112);
	let (cx, cy) = (px(x as f32 + 21.0), (y as f32 + 96.0) * k);
	let c = 127.0 / 128.0;
	out.push(Quad {
		tex: art.morph[0].0,
		rect: [cx - 96.0 * k, cy - 21.0 * sx * k, 192.0 * k, 42.0 * sx * k],
		uv: [0.0, 0.0, 1.0, 1.0],
		colour: [c, c, c, 1.0],
		angle: -std::f32::consts::FRAC_PI_2,
		corners: None,
	});
	let (base, scale, cut) = morph_bubble_frame(x, y, s.display);
	for b in &s.bubbles {
		if b[1] > cut {
			continue;
		}
		let left = (b[0] * scale[0] + base[0]).trunc();
		let top = (b[1] * scale[1] + base[1]).trunc();
		let mut q = sprite(art.morph[1].0, [px(left / 16.0), top / 16.0 * k, 12.0 * sx * k, 12.0 * k], false, 1.0);
		q.colour = [c, c, c, 1.0];
		out.push(q);
	}
}

/// FUN_0024c050's bubble frame in 1/16 px: the base after the radius-6 adjustment, the per-unit scale and the cut
/// (`116 - 2 * loss * 116 + 6`), all f32 as retail.
fn morph_bubble_frame(x: i32, y: i32, display: i32) -> ([f32; 2], [f32; 2], f32) {
	let half_w = 42.0f32 * 0.5 * 0.5;
	let pad_w = (1.0f32 - 0.5) * 0.5 * 42.0;
	let half_h = 192.0f32 * 0.9 * 0.5;
	let pad_h = (1.0f32 - 0.9) * 0.5 * 192.0;
	let base = [(half_w + x as f32 + pad_w) * 16.0 - 96.0, (half_h + y as f32 + pad_h) * 16.0 - 96.0];
	let scale = [(half_w / 17.0) * 16.0, (-half_h / 116.0) * 16.0];
	let max = game::breath_hud::MAX;
	let loss = (max - display) as f32 / max as f32;
	(base, scale, 116.0 - (loss + loss) * 116.0 + 6.0)
}

/// NOV-14.2b marker prompt widget (decomp/notes/novalis_checklist.md "NOV-14.2b"): bar (right cap, middle, left cap)
/// around PS2 x 464, the spinning bolt at x 460, the pulsing red "!" in font 3 at x 448; y 406 = H - 0x2a (PAL,
/// FUN_0024ea48 with iRam0015ed80 != 0; corrected 2026-10-06: was 398). PS2 x is right-anchored [I: as the slot-2 bolt row].
pub(super) fn marker_widget(out: &mut Vec<Quad>, art: &HudArt, (a, b, frames): (u8, u8, u32), hidden: bool, w: f32, k: f32) {
	if hidden || a == 0 || art.bar.len() != 2 || art.bolt.len() != 30 {
		return;
	}
	let (wd, ba, ia) = marker_layout(a, b);
	let (wd, ba, ia) = (wd as f32, ba as f32 / 128.0, ia as f32 / 128.0);
	let px = |x: f32| w - (512.0 - x) * k;
	let (x0, y, s) = (464.0, MARKER_Y, 32.0 * k);
	out.push(sprite(art.bar[1].0, [px(x0 + 16.0), y * k, s, s], true, ba));
	out.push(sprite(art.bar[0].0, [px(x0 - wd), y * k, (wd + 16.0) * k, s], false, ba));
	out.push(sprite(art.bar[1].0, [px(x0 - wd - 32.0), y * k, s, s], false, ba));
	out.push(sprite(art.bolt[((frames % 60) >> 1) as usize].0, [px(460.0), y * k, s, s], false, ia));
	if let Some(font) = &art.font3 {
		let al = (128.0 * marker_pulse(frames, b)) as u32 as f32 / 128.0;
		art_text(out, font, "!", px(449.0), (y + 7.0) * k, k, [0.0, 0.0, 0.0, al]);
		art_text(out, font, "!", px(448.0), (y + 6.0) * k, k, [0xff as f32 / 128.0, 0.0, 0.0, al]);
	}
}

/// NOV-14.2b / ARI-15b: the PAL y of the slot-7 widgets (H - 0x2a = 406, `iRam0015ed80 != 0`).
const MARKER_Y: f32 = 406.0;

/// ARI-15b (decomp/notes/aridia_flow.md "ARI-15b port spec"; draw FUN_0023c918): Skid's counter widget `i` (0 = slot 5
/// sharks at the left, sheet 2000; 1 = slot 7 nests at the right, sheet 2001), drawn while f1 != 0: the bar 0x7580 with
/// both caps at alpha f2i(f2i(f1/8 * 128) * 0.7), the icon at alpha 1.0 [I], the count in font 1 right-aligned with its
/// shadow, both at alpha trunc(128 * f2/8). PS2 x: slot 5 left-anchored `x * k`, slot 7 right-anchored as `marker_widget`.
pub(super) fn count_widget(out: &mut Vec<Quad>, art: &HudArt, i: usize, (display, max, f1, f2): (i32, i32, u8, u8), w: f32, k: f32) {
	if f1 == 0 || art.bar.len() != 2 {
		return;
	}
	let (f1n, f2n) = (f1 as f32 / 8.0, f2 as f32 / 8.0);
	let ba = ((f1n * 128.0) as i32 as f32 * 0.7) as i32 as f32 / 128.0;
	let bw = if max > 9 { 70.0 } else { 48.0 };
	let wd = (bw * f1n) as i32 as f32;
	let (y, s) = (MARKER_Y, 32.0 * k);
	let ta = (128.0 * f2n) as i32 as f32 / 128.0;
	let text = format!("{display}");
	let left = i == 0;
	let px = |x: f32| if left { x * k } else { w - (512.0 - x) * k };
	let (icon, shadow_end, text_end) = if left {
		let x0 = 24.0;
		out.push(sprite(art.bar[1].0, [px(x0 - 28.0), y * k, s, s], false, ba));
		out.push(sprite(art.bar[0].0, [px(x0 + 4.0), y * k, wd * k, s], false, ba));
		out.push(sprite(art.bar[1].0, [px(x0 + 4.0 + wd), y * k, s, s], true, ba));
		(x0, x0 + 67.0, x0 + 66.0)
	} else {
		let x0 = 480.0;
		out.push(sprite(art.bar[1].0, [px(x0 + 4.0), y * k, s, s], true, ba));
		out.push(sprite(art.bar[0].0, [px(x0 + 4.0 - wd), y * k, wd * k, s], false, ba));
		out.push(sprite(art.bar[1].0, [px(x0 + 4.0 - wd - 32.0), y * k, s, s], false, ba));
		(x0 - 22.0, x0 - 25.0, x0 - 26.0)
	};
	if let Some((slot, tw, th)) = art.counters[i] {
		out.push(sprite(slot, [px(icon), y * k, tw * k, th * k], false, 1.0));
	}
	if let Some(font) = &art.font {
		let tw = art_width(font, &text, k);
		art_text(out, font, &text, px(shadow_end) - tw, (y + 9.0) * k, k, [0.0, 0.0, 0.0, ta]);
		art_text(out, font, &text, px(text_end) - tw, (y + 8.0) * k, k, [1.0, 1.0, 0xe0 as f32 / 128.0, ta]);
	}
}

/// Text in the game's font (16x16 cells in a 256x128 sheet); returns the width.
pub(super) fn art_text(out: &mut Vec<Quad>, font: &(usize, Vec<art::Glyph>), s: &str, x: f32, y: f32, k: f32, colour: [f32; 4]) -> f32 {
	let mut cx = x;
	for c in s.chars() {
		let Some(g) = font.1.get(c as usize).copied().filter(|g| g.adv != 0) else { continue };
		if c != ' ' {
			let (u, v) = (g.u as f32, g.v as f32);
			out.push(Quad { tex: font.0, rect: [cx, y + g.dy as f32 * k, 16.0 * k, 16.0 * k], uv: [u / 256.0, v / 128.0, (u + 16.0) / 256.0, (v + 16.0) / 128.0], colour, angle: 0.0, corners: None });
		}
		cx += g.adv as f32 * k;
	}
	cx - x
}

pub(super) fn art_width(font: &(usize, Vec<art::Glyph>), s: &str, k: f32) -> f32 {
	s.chars().filter_map(|c| font.1.get(c as usize)).map(|g| g.adv as f32 * k).sum()
}

/// The HUD: nanotech top left, bolts top right (the game's art when decoded), help and messages.
/// Sized from a 448-line PS2 screen.
/// The scene ids a level's script plays from its scene.wad (decomp/notes/veldin_flow.md,
/// novalis_flow.md: Novalis 0 the Plumber's offer .. 5 the arrival).
pub(super) fn scene_ids(level: usize) -> Option<std::ops::RangeInclusive<u32>> {
	match level {
		0 => Some(1..=4),
		1 => Some(0..=5),
		// ARI-53 (decomp/notes/aridia_flow.md "ARI-53 port spec"): Aridia scenes 0..4 (Skid 0 / 1, agent 2 / 3, pickup 4).
		2 => Some(0..=4),
		// KER-52 (decomp/notes/kerwan_flow.md "KER-52 port spec"): Kerwan scenes 0..10; ids 11..14 have no chunks.
		// Scene 9 (the ship-pad scene) only plays on another planet but is on the disc, so it preloads too.
		3 => Some(0..=10),
		_ => None,
	}
}

/// WPN-6.2g2 / BRIDGE-39 FUN_0020fb40: the queued entries with their centres in window pixels. Hero state 0x72
/// empties the queue; an entry without `at` sits at the screen centre, one whose point does not project is skipped.
pub(super) fn queue_sprites(queue: &[game::ScreenSprite], hero_state: u8, vp: &[[f32; 4]; 4], w: f32, h: f32) -> Vec<(game::ScreenSprite, [f32; 2])> {
	if hero_state == 0x72 {
		return Vec::new();
	}
	queue.iter().filter_map(|s| match s.at {
		None => Some((*s, [w / 2.0, h / 2.0])),
		Some(at) => project(vp, at).map(|[nx, ny]| (*s, [(nx + 1.0) * 0.5 * w, (1.0 - ny) * 0.5 * h])),
	}).collect()
}

/// WPN-6.2g2 / BRIDGE-39 FUN_0020fb40 mode 2 (FUN_0020f738 + FUN_0021c3f0, decomp/notes/weapons_framework.md): one
/// centred quad per queued sprite, side 40 * scale retail pixels, texels 1..63, colour bytes / 128. Screen v runs
/// bottom-to-top and a positive retail rotation turns counter-clockwise (Quad angles are clockwise).
fn screen_sprites(out: &mut Vec<Quad>, sprites: &[(game::ScreenSprite, [f32; 2])], k: f32, art: &HudArt) {
	for (sp, [cx, cy]) in sprites {
		let Some(Some((slot, tw, th))) = (sp.tex as usize).checked_sub(0x21).and_then(|i| art.screen_fx.get(i).copied()) else { continue };
		let s = 40.0 * sp.scale * k;
		let c = sp.abgr.to_le_bytes().map(|b| b as f32 / 128.0);
		out.push(Quad { tex: slot, rect: [cx - s / 2.0, cy - s / 2.0, s, s], uv: [1.0 / tw, 63.0 / th, 63.0 / tw, 1.0 / th], colour: c, angle: -sp.angle, corners: None });
	}
}

/// ARI-42c (decomp/notes/aridia_gadgets.md "ARI-42c port spec"): the Trespasser puzzle HUD strips as four-corner quads
/// (BRIDGE-74). Retail pixel offsets from the centre: x = w / 2 + dx * sx * k, y = (224 + dy) * k; a strip v0 v1 v2 v3
/// maps to corners [v0, v1, v3, v2] (beams, arcs, backdrop) or [v1, v0, v2, v3] (icons, whose u follows y). Colour bytes
/// / 128. [I] the overlay sampler does not switch clamp / repeat per strip.
fn pad_hud(out: &mut Vec<Quad>, prims: &[game::PadPrim], art: &HudArt, w: f32, k: f32) {
	let sx = 448.0f32 * 4.0 / 3.0 / 512.0;
	let px = |[dx, dy]: [f32; 2]| [w / 2.0 + dx * sx * k, (224.0 + dy) * k];
	for q in prims {
		let Some(Some((tex, tw, th))) = game::PAD_FX.iter().position(|&n| n == q.tex).map(|i| art.pad_fx[i]) else { continue };
		let (order, a, b) = if q.tex == 105 { ([1, 0, 2, 3], 1, 2) } else { ([0, 1, 3, 2], 0, 3) };
		let uv = [q.uv[a][0] / tw, q.uv[a][1] / th, q.uv[b][0] / tw, q.uv[b][1] / th];
		let colour = q.rgba.to_le_bytes().map(|c| c as f32 / 128.0);
		out.push(Quad { tex, rect: [0.0; 4], uv, colour, angle: 0.0, corners: Some(order.map(|v| px(q.xy[v]))) });
	}
}

pub(super) fn hud(out: &mut Vec<Quad>, w: f32, h: f32, play: Option<HudState>, messages: &[(String, f32)], flow: Option<&Flow>, art: &HudArt, subtitles: bool) {
	out.clear();
	let k = h / 448.0;
	if let (Some((caption, _)), Some(f)) = (flow.and_then(|f| f.cutscene.as_ref()), flow) {
		overlay::rect(out, [0.0, 0.0, w, 56.0 * k], [0.0, 0.0, 0.0, 1.0]);
		overlay::rect(out, [0.0, h - 56.0 * k, w, 56.0 * k], [0.0, 0.0, 0.0, 1.0]);
		let size = 14.0 * k;
		if f.decoded {
			// The scene's subtitle, word-wrapped in the bottom bar.
			// Subtitles option (default off) gates cutscene subtitles only (ingame_menu_leftovers.md P1-d).
			if let Some(text) = f.subtitle.as_ref().filter(|_| subtitles) {
				let size = 11.0 * k;
				let mut lines = vec![String::new()];
				for word in text.split_whitespace() {
					let line = lines.last_mut().unwrap();
					if !line.is_empty() && line.len() + 1 + word.len() > 60 {
						lines.push(word.to_string());
					} else {
						if !line.is_empty() {
							line.push(' ');
						}
						line.push_str(word);
					}
				}
				let top = h - 50.0 * k + (3usize.saturating_sub(lines.len())) as f32 * 7.0 * k;
				for (i, l) in lines.iter().take(3).enumerate() {
					overlay::text(out, l, (w - overlay::text_width(l, size)) / 2.0, top + i as f32 * 14.0 * k, size, [1.0, 1.0, 1.0, 1.0]);
				}
			}
			let hint = "SPACE: SKIP";
			overlay::text(out, hint, w - overlay::text_width(hint, size * 0.6) - 12.0 * k, 22.0 * k, size * 0.6, [0.6, 0.6, 0.6, 1.0]);
			return;
		}
		overlay::text(out, caption, (w - overlay::text_width(caption, size)) / 2.0, h - 40.0 * k, size, [1.0, 1.0, 1.0, 1.0]);
		let hint = "(NOT DECODED YET)  SPACE: SKIP";
		overlay::text(out, hint, (w - overlay::text_width(hint, size * 0.7)) / 2.0, h - 20.0 * k, size * 0.7, [0.7, 0.7, 0.7, 1.0]);
		return;
	}
	// The HelpDesk box (help.rs), drawn only with HelpDesk Text on.
	if let (Some(f), Some(font)) = (flow, &art.font) {
		f.help.draw(out, font, art.help_icon, w, h, f.help_text);
	}
	// An NPC's or the ship pad's text box (novalis_flow.md "Prompt and dialogue box"): sheet 0x7580
	// pieces, top y 42 PAL, height 0x20 (0x36 with two lines), colour 0x40f040, 8-frame fade.
	if let (Some(f), Some(font), true) = (flow.filter(|f| !f.help.busy() && f.prompt_box.2 > 0.0), &art.font, art.bar.len() == 2) {
		let fade = (f.prompt_box.2 * 8.0).floor() / 8.0;
		let lines: Vec<&str> = f.prompt_box.0.split('\u{1}').collect();
		let tw = lines.iter().map(|l| art_width(font, l, 1.0)).fold(0.0, f32::max);
		let (top, hgt) = (42.0 * k, if lines.len() > 1 { 54.0 } else { 32.0 } * k);
		let left = w / 2.0 - tw * k / 2.0;
		out.push(sprite(art.bar[1].0, [left - 32.0 * k, top, 32.0 * k, hgt], false, fade));
		out.push(sprite(art.bar[0].0, [left, top, tw * k, hgt], false, fade));
		out.push(sprite(art.bar[1].0, [left + tw * k, top, 32.0 * k, hgt], true, fade));
		for (i, line) in lines.iter().enumerate() {
			// One colour 0x40f040 (bytes / 128) for the whole string; codes 0x08..0x0f are skipped with
			// no highlight colour (FUN_0021cbf8 OV:12189/12195).
			let mut x = w / 2.0 - art_width(font, line, 1.0) * k / 2.0;
			let y = top + (8.0 + i as f32 * 27.0) * k;
			let c = [0.5, 0xf0 as f32 / 128.0, 0.5];
			for ch in line.chars() {
				match ch {
					'\u{c}' | '\u{8}' => {}
					_ => x += art_text(out, font, &ch.to_string(), x, y, k, [c[0], c[1], c[2], fade]),
				}
			}
		}
	}
	let gold = [1.0, 0.82, 0.25, 1.0];
	// PS2 x to overlay units: 512 PS2 pixels span the 4:3 picture (448 lines tall).
	let sx = 448.0 * 4.0 / 3.0 / 512.0;
	match play {
		Some(st) => {
			screen_sprites(out, &st.sprites, k, art);
			pad_hud(out, &st.pad, art, w, k);
			let (health, bolts) = (st.health, st.bolts);
			// Nanotech: four orbs on the bar, filled ones spinning. Placed top centre, 32 PS2 px apart,
			// row centre y 34, measured from a PAL retail capture (hud.md) [I]; the anchor is not decoded.
			// Fade bytes (f1 bar, f2 orbs) come from `nano_step`; the bar spans 6 * f1 PS2 px either side of the centre.
			let (f1, f2) = st.nano;
			let a = f2 as f32 / 8.0;
			if f1 > 0 && art.orb.len() > 30 && art.bar.len() == 2 {
				let (step, y) = (32.0 * sx * k, 34.0 * k);
				let x = w / 2.0 - 2.0 * step;
				let (hw, ba) = nano_bar(f1);
				let (hwp, cap) = (hw as f32 * sx * k, 32.0 * sx * k);
				let c = w / 2.0;
				let (top, hgt) = (y - 16.0 * k, 32.0 * k);
				out.push(sprite(art.bar[1].0, [c - hwp - cap, top, cap, hgt], false, ba));
				out.push(sprite(art.bar[0].0, [c - hwp, top, 2.0 * hwp, hgt], false, ba));
				out.push(sprite(art.bar[1].0, [c + hwp, top, cap, hgt], true, ba));
				for i in 0..4 {
					let f = if (i as i32) < health { ((st.clock * 30.0) as usize + i * 6) % 30 } else { 30 };
					// Retail draws the empty frame 30 under a filled orb (OV:47172-47174); quad sizes are the PS2 texture size,
					// x stretched by the 4:3 factor like the positions (FUN_0023b0c0).
					for fr in if f == 30 { vec![30] } else { vec![30, f] } {
						let (slot, sw, sh) = art.orb[fr];
						out.push(sprite(slot, [x + (i as f32 + 0.5) * step - sw * sx / 2.0 * k, y - sh / 2.0 * k, sw * sx * k, sh * k], false, a));
					}
				}
			} else if f1 > 0 {
				for i in 0..4 {
					let r = [24.0 * k + i as f32 * 22.0 * k, 20.0 * k, 16.0 * k, 16.0 * k];
					overlay::rect(out, [r[0] - 2.0 * k, r[1] - 2.0 * k, r[2] + 4.0 * k, r[3] + 4.0 * k], [0.0, 0.0, 0.0, 0.6 * a]);
					let on = (i as i32) < health;
					overlay::rect(out, r, if on { [0.25, 0.9, 1.0, a] } else { [0.15, 0.2, 0.25, 0.8 * a] });
				}
			}
			// Bolts: digits right-aligned before the spinning bolt, on the bar (FUN_002399d8).
			let a = st.alpha.0;
			let text = format!("{bolts}");
			match &art.font {
				Some(font) if a > 0.0 && art.bolt.len() == 30 && art.bar.len() == 2 => {
					// Positions measured from a retail capture (hud.md): bar ends at the slot-2 anchor
					// x 492 (23 units in from the right edge), bolt centred 46 in, row centre y 36 (PAL).
					let (x, y) = (w - 62.0 * k, 36.0 * k);
					let tw = art_width(font, &text, k);
					let left = x - tw + 20.0 * k;
					bar(out, art, left, y, (w - 55.0 * k - left).max(0.0), k, a);
					let (slot, _, _) = art.bolt[((st.clock * 30.0) as usize) % 30];
					out.push(sprite(slot, [x, y - 16.0 * k, 32.0 * sx * k, 32.0 * k], false, a));
					// 0x80e08060 and shadow 0x80000000, both scaled by the fade (FUN_002399d8), bytes / 128.
					art_text(out, font, &text, x - tw + k, y - 7.0 * k, k, [0.0, 0.0, 0.0, a]);
					art_text(out, font, &text, x - tw, y - 8.0 * k, k, [0.75, 1.0, 0xe0 as f32 / 128.0, a]);
				}
				_ if a > 0.0 => {
					let size = 20.0 * k;
					let x = w - 24.0 * k - overlay::text_width(&text, size);
					overlay::text(out, &text, x, 18.0 * k, size, [gold[0], gold[1], gold[2], a]);
					overlay::text(out, "BOLTS", x - overlay::text_width("BOLTS ", size * 0.6), 18.0 * k + size * 0.35, size * 0.6, [gold[0], gold[1], gold[2], a]);
				}
				_ => {}
			}
			// NOV-14.2b marker prompt (HUD slot 7). The level-mode-2 and hero-state-0x32 gates stay false (HERO-13).
			marker_widget(out, art, st.marker, false, w, k);
			// ARI-15b: Skid's counter widgets (HUD slots 5 / 7).
			for (i, c) in st.counters.iter().enumerate() {
				if let Some(c) = c {
					count_widget(out, art, i, *c, w, k);
				}
			}
			if let Some(b) = &st.breath {
				// Slot 4 holds one of the two (WPN-6.7e4).
				if b.morph {
					morph_meter(out, art, b, w, k);
				} else {
					breath_meter(out, art, b, w, k);
				}
			}
			// Ammo HUD (hud.md "Ammo HUD", draw FUN_0023a390): x 20, y 18 (PAL capture; y 10 is the
			// iRam0015ed80 variant, seen in a non-PAL capture), PS2 x scaled by `sx`. Bar 0x7580 at alpha 0.7*f1 from
			// x-8 to the text end +12 (span measured from the capture [I]); bw 70, or 95 when
			// max >= 100. Weapon icon frame 3 at (x, y), alpha f1. "%d/%d" right-aligned at x+25+bw,
			// y+8 after a black +1,+1 shadow, colour 0x00e08060, 0x00202080 at 0 ammo, alpha 0x80*f2.
			if let Some((id, n, max, f1, f2)) = st.ammo {
				let (x, y) = (20.0 * sx, 18.0);
				let bw = if max >= 100 { 95.0 } else { 70.0 };
				let tr = (20.0 + 25.0 + bw) * sx;
				if art.bar.len() == 2 {
					let (l, r) = (12.0, tr + 14.0);
					let (mid, ba) = ((r - l - 64.0).max(0.0) * f1, 0.7 * f1);
					out.push(sprite(art.bar[1].0, [l * k, y * k, 32.0 * k, 32.0 * k], false, ba));
					out.push(sprite(art.bar[0].0, [(l + 32.0) * k, y * k, mid * k, 32.0 * k], false, ba));
					out.push(sprite(art.bar[1].0, [(l + 32.0 + mid) * k, y * k, 32.0 * k, 32.0 * k], true, ba));
				}
				if let Some(&(slot, iw, ih)) = art.weapon_icons.get(&id) {
					out.push(sprite(slot, [x * k, y * k, iw * k, ih * k], false, f1));
				}
				if let (Some(font), true) = (&art.font, f2 > 0.0) {
					let text = format!("{n}/{max}");
					let c = |v: u8| v as f32 / 128.0;
					let colour = if n == 0 { [c(0x80), c(0x20), c(0x20), f2] } else { [c(0x60), c(0x80), c(0xe0), f2] };
					let tx = tr * k - art_width(font, &text, k);
					art_text(out, font, &text, tx + k, (y + 9.0) * k, k, [0.0, 0.0, 0.0, f2]);
					art_text(out, font, &text, tx, (y + 8.0) * k, k, colour);
				}
			}
			// Lock-on reticle: four corner brackets on the target (the game's reticle sprites are not located).
			if let Some([nx, ny]) = st.reticle {
				let (cx, cy) = ((nx + 1.0) * 0.5 * w, (1.0 - ny) * 0.5 * h);
				let (r, len, t) = (18.0 * k, 8.0 * k, 2.0 * k);
				let red = [1.0, 0.25, 0.15, 0.9];
				for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
					let (x, y) = (cx + sx * r, cy + sy * r);
					let hx = if sx < 0.0 { x } else { x - len };
					let vy = if sy < 0.0 { y } else { y - len };
					overlay::rect(out, [hx, y - t / 2.0, len, t], red);
					overlay::rect(out, [x - t / 2.0, vy, t, len], red);
				}
			}
			if let (Some((a, tiles)), Some(tex)) = (&st.rc_static, art.static_tex) {
				rc_static(out, tex, *a, tiles, w, h);
			}
		}
		None => overlay::text(out, "P: PLAY   RMB: LOOK   WASD/QE: FLY   PGUP/PGDN: LEVEL", 16.0 * k, 16.0 * k, 12.0 * k, [1.0, 1.0, 1.0, 0.8]),
	}
	for (i, (m, t)) in messages.iter().rev().take(3).enumerate() {
		let size = 16.0 * k;
		let x = (w - overlay::text_width(m, size)) / 2.0;
		overlay::text(out, m, x, h - (60.0 + 24.0 * i as f32) * k, size, [1.0, 1.0, 1.0, t.min(1.0)]);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// ARI-15b test 3: the counter widgets' layout at f1 = f2 = 7 (k = 1, a 512-wide screen).
	#[test]
	fn count_widget_layout() {
		let mut art = font_art();
		art.counters = [Some((40, 32.0, 32.0)), Some((41, 32.0, 32.0))];
		let mut out = Vec::new();
		count_widget(&mut out, &art, 0, (57, 100, 7, 7), 512.0, 1.0);
		let xs: Vec<f32> = out[..4].iter().map(|q| q.rect[0]).collect();
		assert_eq!(xs, [-4.0, 28.0, 89.0, 24.0]);
		assert_eq!(out[1].rect[2], 61.0);
		assert!((out[0].colour[3] - 78.0 / 128.0).abs() < 1e-6);
		assert_eq!(out[3].rect[1], 406.0);
		// "57": two 8-wide glyphs ending at 90 (text) / 91 (shadow), y 414 / 415, alpha 112.
		let text: Vec<_> = out[4..].iter().map(|q| (q.rect[0], q.rect[1], q.colour[3])).collect();
		assert_eq!(text, [(75.0, 415.0, 112.0 / 128.0), (83.0, 415.0, 112.0 / 128.0), (74.0, 414.0, 112.0 / 128.0), (82.0, 414.0, 112.0 / 128.0)]);
		out.clear();
		count_widget(&mut out, &art, 1, (7, 7, 7, 7), 512.0, 1.0);
		let xs: Vec<f32> = out[..4].iter().map(|q| q.rect[0]).collect();
		assert_eq!(xs, [484.0, 442.0, 410.0, 458.0]);
		assert_eq!(out[1].rect[2], 42.0);
		// "7" ends at 454.
		assert_eq!((out[4].rect[0] + 8.0, out[5].rect[0] + 8.0, out[5].rect[1]), (455.0, 454.0, 414.0));
		out.clear();
		count_widget(&mut out, &art, 0, (57, 100, 0, 0), 512.0, 1.0);
		assert!(out.is_empty());
	}

	/// NOV-14.2b: the marker prompt sits at PAL y 406 (FUN_0024ea48, H - 0x2a).
	#[test]
	fn marker_widget_pal_y() {
		let art = font_art();
		let mut out = Vec::new();
		marker_widget(&mut out, &art, (5, 5, 0), false, 512.0, 1.0);
		assert!(!out.is_empty() && out.iter().take(4).all(|q| q.rect[1] == 406.0));
	}

	/// ARI-53 test 3: the viewer preloads Aridia scenes 0..4.
	#[test]
	fn aridia_scene_ids() {
		assert_eq!(scene_ids(2), Some(0..=4));
	}

	/// KER-52: the viewer preloads Kerwan scenes 0..10 (the KER-28 smoke scenes 3, 4 and 10 among them).
	#[test]
	fn kerwan_scene_ids() {
		assert_eq!(scene_ids(3), Some(0..=10));
		assert!(scene_ids(3).is_some_and(|r| [3, 4, 10].iter().all(|i| r.contains(i)) && !r.contains(&11)));
	}

	fn font_art() -> HudArt {
		let mut art = HudArt::default();
		art.bar = vec![(1, 32.0, 32.0), (2, 32.0, 32.0)];
		art.bolt = (0..30).map(|i| (10 + i, 32.0, 32.0)).collect();
		art.font = Some((7, (0..128).map(|_| art::Glyph { u: 0, v: 0, dy: 0, adv: 8 }).collect()));
		art
	}

	// UI-TEXT-1 test (4): one prompt colour 0x40f040 / 128, codes 0x0c and 0x08 skipped.
	#[test]
	fn prompt_text_one_colour() {
		let art = font_art();
		let mut flow = Flow::new(&[], Vec::new(), &std::collections::HashMap::new(), 0);
		flow.prompt_box = ("a\u{c}b\u{8}c".to_string(), 1.0, 1.0);
		let mut out = Vec::new();
		hud(&mut out, 512.0, 448.0, None, &[], Some(&flow), &art, false);
		let glyphs: Vec<[f32; 4]> = out.iter().filter(|q| q.tex == 7).map(|q| q.colour).collect();
		assert_eq!(glyphs, vec![[0.5, 1.875, 0.5, 1.0]; 3]);
	}

	// UI-TEXT-1 test (5): bolt digits 0x80e08060 / 128 and the shadow 0x80000000, both at the fade a.
	#[test]
	fn bolt_digit_colours() {
		let art = font_art();
		let st = HudState { health: 1, bolts: 12, clock: 0.0, alpha: (0.5, 0.0), nano: (0, 0), ammo: None, reticle: None, rc_static: None, marker: (0, 0, 0), breath: None, counters: [None; 2], sprites: Vec::new(), pad: Vec::new() };
		let mut out = Vec::new();
		hud(&mut out, 512.0, 448.0, Some(st), &[], None, &art, false);
		let glyphs: Vec<[f32; 4]> = out.iter().filter(|q| q.tex == 7).map(|q| q.colour).collect();
		assert_eq!(glyphs, vec![[0.0, 0.0, 0.0, 0.5], [0.0, 0.0, 0.0, 0.5], [0.75, 1.0, 1.75, 0.5], [0.75, 1.0, 1.75, 0.5]]);
	}

	// NOV-14.2b tests (1)-(5).
	// NOV-14.2 test (6): the first tile of the 512 x 448 screen sits at (-16, -16), 32 x 32, 255 tiles.
	#[test]
	fn rc_static_layout() {
		let mut out = Vec::new();
		rc_static(&mut out, (7, 32.0, 32.0), 127, &[[3, 5]; 255], 512.0, 448.0);
		assert_eq!(out.len(), 255);
		assert_eq!(out[0].rect, [-16.0, -16.0, 32.0, 32.0]);
		assert_eq!(out[0].uv, [3.0 / 32.0, 5.0 / 32.0, 35.0 / 32.0, 37.0 / 32.0]);
		assert_eq!(out[1].rect, [-16.0, 16.0, 32.0, 32.0]);
		assert_eq!(out[15].rect, [16.0, -16.0, 32.0, 32.0]);
		assert_eq!(out[254].rect, [16.0 * 32.0 - 16.0, 14.0 * 32.0 - 16.0, 32.0, 32.0]);
	}

	// VEL-11.1 test (1)-(2): an orb sprite is 32 * sx wide and 32 tall; the step is unchanged; a filled orb has the empty frame under it.
	#[test]
	fn orb_quads_use_the_ps2_size_and_the_4_3_factor() {
		let mut art = HudArt::default();
		art.bar = vec![(1, 32.0, 32.0), (2, 32.0, 32.0)];
		art.orb = (0..32).map(|i| (100 + i, 32.0, 32.0)).collect();
		let st = HudState { health: 1, bolts: 0, clock: 0.0, alpha: (0.0, 0.0), nano: (8, 8), ammo: None, reticle: None, rc_static: None, marker: (0, 0, 0), breath: None, counters: [None; 2], sprites: Vec::new(), pad: Vec::new() };
		let mut out = Vec::new();
		hud(&mut out, 1920.0, 1080.0, Some(st), &[], None, &art, false);
		let (sx, k) = (448.0 * 4.0 / 3.0 / 512.0, 1080.0 / 448.0);
		let orbs: Vec<&Quad> = out.iter().filter(|q| (100..132).contains(&q.tex)).collect();
		assert_eq!(orbs.len(), 5, "1 filled (+ empty under) + 3 empty");
		assert!((orbs[0].rect[2] - 32.0 * sx * k).abs() < 1e-3 && (orbs[0].rect[3] - 32.0 * k).abs() < 1e-3);
		assert_eq!((orbs[0].tex, orbs[1].tex), (130usize, 100usize));
		assert!((orbs[2].rect[0] - orbs[0].rect[0] - 32.0 * sx * k).abs() < 1e-3);
	}
	// UI-7: nanotech bar geometry, fade-byte step and lifetime rules.
	#[test]
	fn nano_bar_orbs_and_lifetime() {
		assert_eq!(nano_bar(8), (48, 20.0 / 128.0));
		assert_eq!(nano_bar(4), (24, 0.078125));
		// Bar spans (256 -/+ hw) * sx with 32 * sx caps; first orb centre is 208 * sx.
		let mut art = HudArt::default();
		art.bar = vec![(1, 32.0, 32.0), (2, 32.0, 32.0)];
		art.orb = (0..32).map(|i| (100 + i, 32.0, 32.0)).collect();
		let st = HudState { health: 4, bolts: 0, clock: 0.0, alpha: (0.0, 0.0), nano: (8, 8), ammo: None, reticle: None, rc_static: None, marker: (0, 0, 0), breath: None, counters: [None; 2], sprites: Vec::new(), pad: Vec::new() };
		let mut out = Vec::new();
		hud(&mut out, 1920.0, 1080.0, Some(st), &[], None, &art, false);
		let (sx, k) = (448.0 * 4.0 / 3.0 / 512.0, 1080.0 / 448.0);
		let mid = out.iter().find(|q| q.tex == 1).unwrap();
		assert!((mid.rect[0] - (960.0 - 48.0 * sx * k)).abs() < 1e-2 && (mid.rect[2] - 96.0 * sx * k).abs() < 1e-2);
		assert!((out.iter().filter(|q| q.tex == 2).all(|q| (q.rect[2] - 32.0 * sx * k).abs() < 1e-2)));
		// Health 3 unchanged: hides after 97 lifetime ticks (100 - k < 4), then 16 fade ticks.
		let mut s = nano_step((0, 0, 0, -1), 3);
		let mut n = 0;
		while s.1 > 0 || s.2 > 0 || n < 20 {
			s = nano_step(s, 3);
			n += 1;
			assert!(n < 400);
		}
		// Health 1 never hides.
		let mut s = (0, 0, 0, -1);
		for _ in 0..1000 {
			s = nano_step(s, 1);
		}
		assert_eq!((s.1, s.2), (8, 8));
		// Init lifetime 180: still shown after 176 ticks, hidden when lifetime < 4 (after 177) and fully faded 16 ticks later.
		let mut s = nano_step((0, 0, 0, -1), 3);
		assert_eq!(s.0, 179);
		for _ in 0..175 {
			s = nano_step(s, 3);
		}
		assert_eq!(s.0, 4);
		assert!(s.1 == 8 && s.2 == 8);
		s = nano_step(s, 3);
		assert_eq!((s.0, s.2), (3, 7));
	}

	// HUD-BREATH-TANK: the tank is a rotated quad and PS2 x is scaled by sx (hud.md "HUD-BREATH-TANK port spec").
	#[test]
	fn breath_tank_rotated_and_scaled() {
		let mut art = HudArt::default();
		art.breath = vec![(1, 256.0, 64.0), (2, 256.0, 64.0), (3, 32.0, 32.0)];
		let snap = |blink: bool, bubbles: Vec<[f32; 4]>| game::breath_hud::BreathSnapshot { display: 10000, slide: 23, lifetime: 1, blink, bubbles, morph: false };
		let (w, k) = (1920.0f32, 1080.0f32 / 448.0);
		let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-2);
		let mut out = Vec::new();
		breath_meter(&mut out, &art, &snap(false, Vec::new()), w, k);
		assert_eq!(out.len(), 2);
		assert!(near(out[0].rect, [1769.5312, 288.08038, 70.13672, 423.08038]), "{:?}", out[0].rect);
		let t = out[1];
		assert_eq!((t.tex, t.angle, t.uv), (1, -std::f32::consts::FRAC_PI_2, [0.0, 0.0, 1.0, 1.0]));
		assert!(near(t.rect, [1573.2589, 442.3661, 462.85718, 118.12501]), "{:?}", t.rect);
		// Rotated corners: half extents swap under the quarter turn.
		let (cx, cy) = (t.rect[0] + t.rect[2] / 2.0, t.rect[1] + t.rect[3] / 2.0);
		let (hx, hy) = (t.rect[3] / 2.0, t.rect[2] / 2.0);
		assert!(near([cx - hx, cx + hx, cy - hy, cy + hy], [1745.625, 1863.75, 270.0, 732.857]));
		out.clear();
		breath_meter(&mut out, &art, &snap(true, Vec::new()), w, k);
		assert_eq!(out[1].tex, 2);
		out.clear();
		breath_meter(&mut out, &art, &snap(false, vec![[0.0; 4]]), w, k);
		assert_eq!(out.len(), 3);
		let sx = 448.0f32 * 4.0 / 3.0 / 512.0;
		assert_eq!((out[2].rect[2], out[2].rect[3]), (12.0 * sx * k, 12.0 * k));
		assert!((out[2].rect[2] - 33.75).abs() < 1e-2 && (out[2].rect[3] - 28.928574).abs() < 1e-2);
	}

	// WPN-6.7e4 (6): bubble placement in 1/16 px at slide 23 (x 450) and the cut line; the tank is the rotated quad.
	#[test]
	fn morph_meter_bubbles_and_cut() {
		let (base, scale, cut) = morph_bubble_frame(450, 112, 10000);
		let at = |b: [f32; 2]| ((b[0] * scale[0] + base[0]).trunc(), (b[1] * scale[1] + base[1]).trunc());
		assert_eq!(at([0.0, 0.0]), (7440.0, 3232.0));
		assert_eq!(at([-11.0, -110.0]), (7331.0, 4542.0));
		assert_eq!(at([11.0, 110.0]), (7548.0, 1921.0));
		assert_eq!([cut, morph_bubble_frame(450, 112, 7500).2, morph_bubble_frame(450, 112, 0).2], [122.0, 64.0, -110.0]);
		let mut art = HudArt::default();
		art.morph = vec![(1, 256.0, 64.0), (2, 32.0, 32.0)];
		let s = game::breath_hud::BreathSnapshot { display: 7500, slide: 23, lifetime: 1, blink: false, bubbles: vec![[0.0, 64.0, 0.0, 0.0], [0.0, 64.01, 0.0, 0.0]], morph: true };
		let (w, k) = (1920.0f32, 1080.0f32 / 448.0);
		let mut out = Vec::new();
		morph_meter(&mut out, &art, &s, w, k);
		assert_eq!(out.len(), 2);
		assert_eq!((out[0].tex, out[0].angle), (1, -std::f32::consts::FRAC_PI_2));
		assert!(out[0].rect.iter().zip([1573.2589, 442.3661, 462.85718, 118.12501]).all(|(a, b)| (a - b).abs() < 1e-2), "{:?}", out[0].rect);
		let sx = 448.0f32 * 4.0 / 3.0 / 512.0;
		assert_eq!((out[1].tex, out[1].rect[2], out[1].rect[3]), (2, 12.0 * sx * k, 12.0 * k));
	}

	#[test]
	fn breath_cut_line_values() {
		let cut = |d: i32| {
			let loss = (10000 - d) as f32 / 10000.0;
			116.0 - (loss + loss) * 116.0
		};
		assert_eq!(cut(5000), 0.0);
		assert_eq!(cut(3333), -38.674407958984375);
		assert_eq!(cut(3334), -38.65119934082031);
		assert_eq!(cut(10000), 116.0);
		assert_eq!(cut(0), -116.0);
	}

	// NOV-30f test 8: the nano request latch.
	#[test]
	fn nano_latch_shop_and_damage() {
		let (mut live, mut n) = (true, (0, 0, 0, -1));
		let frame = |live: &mut bool, n: &mut (i32, u8, u8, i32), shop: bool, h: i32, prev: i32| {
			nano_request(live, n, shop, h, prev);
			nano_tick(live, n, shop, h);
		};
		frame(&mut live, &mut n, false, 4, -1);
		assert_eq!(n, (179, 1, 0, 4));
		for _ in 0..50 {
			frame(&mut live, &mut n, true, 4, 4);
		}
		assert_eq!((live, n), (false, (0, 0, 0, -1)));
		for _ in 0..100 {
			frame(&mut live, &mut n, false, 4, 4);
		}
		assert_eq!((live, n), (false, (0, 0, 0, -1)));
		// Damage 4 -> 3: requested, fresh init.
		frame(&mut live, &mut n, false, 3, 4);
		assert_eq!((live, n), (true, (179, 1, 0, 3)));
		// Expires: released the tick after both fade bytes reach 0.
		let mut k = 0;
		while live {
			frame(&mut live, &mut n, false, 3, 3);
			k += 1;
			assert!(k < 400);
		}
		// 175 lifetime ticks, 16 fade ticks (from lifetime 3), then the release tick.
		assert_eq!(k, 175 + 16 + 1);
		// Heal 3 -> 4 on a not-live slot: nothing.
		frame(&mut live, &mut n, false, 4, 3);
		assert_eq!((live, n), (false, (0, 0, 0, -1)));
		frame(&mut live, &mut n, false, 3, 4);
		assert_eq!((live, n), (true, (179, 1, 0, 3)));
		// Live and hidden with a value change: the update reloads before the release check.
		let (mut live, mut n) = (true, (0, 0, 0, 3));
		nano_tick(&mut live, &mut n, false, 2);
		assert_eq!((live, n), (true, (100, 1, 0, 2)));
	}

	// WPN-6.2g2 tests (1)-(3): the BRIDGE-39 screen sprites.
	#[test]
	fn screen_sprite_quads() {
		let mut art = HudArt::default();
		art.screen_fx[5] = Some((9, 64.0, 64.0));
		let sp = game::ScreenSprite { scale: 1.0, angle: 0.0, abgr: 0xff0f_0fff, tex: 0x26, at: None };
		let id = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
		let q = queue_sprites(&[sp], 0, &id, 1280.0, 896.0);
		let mut out = Vec::new();
		screen_sprites(&mut out, &q, 896.0 / 448.0, &art);
		assert_eq!(out.len(), 1);
		assert_eq!((out[0].tex, out[0].rect, out[0].angle), (9, [600.0, 408.0, 80.0, 80.0], 0.0));
		assert_eq!(out[0].colour, [1.9921875, 0.1171875, 0.1171875, 1.9921875]);
		assert_eq!(out[0].uv, [1.0 / 64.0, 63.0 / 64.0, 63.0 / 64.0, 1.0 / 64.0]);
		// (2) scale 0.5 at a point projecting to NDC (0, 0): centre (640, 448), side 40.
		let at = game::ScreenSprite { scale: 0.5, at: Some([0.0, 0.0, 0.5]), ..sp };
		let q = queue_sprites(&[at], 0, &id, 1280.0, 896.0);
		out.clear();
		screen_sprites(&mut out, &q, 2.0, &art);
		assert_eq!(out[0].rect, [620.0, 428.0, 40.0, 40.0]);
		// (3) hero state 0x72: nothing.
		assert!(queue_sprites(&[sp, at], 0x72, &id, 1280.0, 896.0).is_empty());
		// A kind without a texture is skipped.
		out.clear();
		screen_sprites(&mut out, &[(game::ScreenSprite { tex: 0x27, ..sp }, [0.0; 2])], 2.0, &art);
		assert!(out.is_empty());
	}
}
