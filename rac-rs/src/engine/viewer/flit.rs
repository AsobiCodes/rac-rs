//! [PC] Fire + weapon lights on the world surfaces (lane flit, 2026-11-05; user: "the flames should brighten up the shadows"; lane
//! dlit, 2026-11-05: real point lights; lane dlit2, 2026-11-06: "i want the fire to be a true source of light bro").
//! The frame's surface light list (flitlist.rs: every fire as up to 32 tracked sphere lights at Global illumination High, 16 Medium,
//! 8 Low, plus the weapon lights) is packed into the level uniform (`Uniforms::flit`, `FlitU`, written by draw/passes.rs before the
//! level pass) and the forward shaders light the surface's texture colour with them (flit.wgsl `flit_add`: fire light v2, a sphere
//! emitter: E = I / max(d, rs)^2 x the sphere's horizon-clipped form factor, windowed smoothly to 0 at the reach), added to its sun +
//! ambient light before fog. Rows: 2k = (centre, reach), 2k + 1 = (rgb x intensity, body radius + 8 x shadow map (0 none, 1..4));
//! `SHAPE` = (on, debug, sheen, deep); `MAPS` .. `MAPS + 3` = the shadow maps' (origin, far) (sunshadow/omni.rs, flitshadow.wgsl);
//! `MAPQ` = the moment lookup's (mip bias, scale of the fire radius, widest penumbra cone in rad, light-bleeding reduction);
//! `MAPF` = the fade of maps 0..3; `MAPSZ` = the radius of the fires of maps 0..3 (flitshadow.wgsl's soft shadows); `MAPB` = the
//! receivers' (normal offset in mip-0 moment texels, depth bias in m, slope bias, moment bias) (sunshadow/omni.rs `light_shadows`); `GRID` = (origin x, y, 1 / cell, cells
//! per side; .w 0 = no light at all: the shader returns the colour untouched); `SHAPE2` = (wrap: extra facing round a light's
//! horizon, roll: the soft roll-off of the added light, per map its fire's class and mean weight: `fire_code`); `MAPZN` = the near plane (m) of maps 0..3 (fire light v2 stage 2: the
//! emitter's half extent, flitshadow.wgsl); then per world cell the bit mask of its lights (flitlist.rs `grid`).
//! Post (`fx_light`) adds nothing more for the fire / weapon rows on the level pass's pixels. Nothing in use (Classic, GI Off with
//! no shot, post off, no fire or shot): all zero, the picture as before. Knobs (RAC_TUNE): flit_on (0 = this pass off, post keeps
//! all), flit_sheen, flit_debug (1 = the light alone, 2 = the albedo, 3 = the shadow maps, 4 = the normal), flit_deep (m from the body to the deep colour), flit_wrap,
//! flit_roll;
//! flitlist.rs flit_gain, flit_lift, flit_rsk, flit_shot; sunshadow/omni.rs flsh_*.

use super::flitlist::{self, Light};
use bytemuck::{Pod, Zeroable};

/// Byte offset of `Uniforms::flit` in the level uniform buffer.
pub(super) const OFFSET: u64 = std::mem::offset_of!(super::Uniforms, flit) as u64;

/// Shadow maps at most (sunshadow/omni.rs `MAX_LIGHTS`).
pub(super) const MAPN: usize = 4;
/// The rows after the lights: the shape, the shadow maps, the maps' shape / fades / fire radii / bias, the grid, the second shape.
/// Twins of flit.wgsl / flitshadow.wgsl.
pub(super) const SHAPE: usize = 2 * flitlist::MAX;
pub(super) const MAPS: usize = SHAPE + 1;
pub(super) const MAPQ: usize = MAPS + MAPN;
pub(super) const MAPF: usize = MAPQ + 1;
pub(super) const MAPSZ: usize = MAPQ + 2;
pub(super) const MAPB: usize = MAPQ + 3;
pub(super) const GRID: usize = MAPQ + 4;
pub(super) const SHAPE2: usize = MAPQ + 5;
pub(super) const MAPZN: usize = MAPQ + 6;
pub(super) const ROWS: usize = MAPQ + 7;
const TAIL: usize = ROWS - 2 * flitlist::MAX;
const MASKS: usize = flitlist::CELLS * flitlist::CELLS / 4;

/// `Uniforms::flit`: the WGSL `flit: array<vec4<f32>, ROWS>, flit_grid: array<vec4<u32>, MASKS>` (the light rows split in two
/// arrays only for bytemuck's Pod sizes).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub(super) struct FlitU {
	lights: [[f32; 4]; 2 * flitlist::MAX],
	tail: [[f32; 4]; TAIL],
	grid: [[u32; 4]; MASKS],
}

impl FlitU {
	/// Row `i` of the WGSL `flit` array.
	pub(super) fn row(&self, i: usize) -> [f32; 4] {
		if i < SHAPE { self.lights[i] } else { self.tail[i - SHAPE] }
	}
	fn row_mut(&mut self, i: usize) -> &mut [f32; 4] {
		if i < SHAPE { &mut self.lights[i] } else { &mut self.tail[i - SHAPE] }
	}
}

/// [PC] lane dlit: the fires with a cube shadow map this frame (sunshadow/omni.rs `light_shadows`): `idx[s]` = the fire (index in
/// flitlist.rs `groups`) of map s; `maps[s]` = its (origin, far); `q` = (texels per face, faces per atlas side, a face's share of
/// the atlas side, widest filter radius in texels; since stage 5 the row MAPQ, see the module doc); per map its fade and (lane dlit2, the soft shadows) its fire's radius; `bias` =
/// the receivers' row `MAPB`. Default: none, all zero.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(super) struct Shadows {
	pub idx: [Option<usize>; MAPN],
	pub maps: [[f32; 4]; MAPN],
	pub q: [f32; 4],
	pub fade: [f32; MAPN],
	pub size: [f32; MAPN],
	pub bias: [f32; 4],
	/// Fire light v2 stage 2: per map its faces' near plane (m; flitlist.rs `Group::near`, the emitter's half extent).
	pub near: [f32; MAPN],
	/// Fire light v2 stage 6 (flitsplit.rs): per map its fire (index in the frame's fires) and its origin's share of that fire's
	/// power; `mean`: every light of a fire takes the share-weighted mean of its fire's maps (RAC_TUNE flsh_mean, `fire_code`).
	pub fire: [u8; MAPN],
	pub share: [f32; MAPN],
	pub mean: bool,
}

/// 1 while the level pass lights the surfaces with these rows (post.rs `fx_light`, p.q.w: then it adds nothing for them on the
/// level pass's pixels); RAC_TUNE flit_on=0 turns it off and post keeps its old screen term. Twin of row `SHAPE` .x.
pub(super) fn near_share() -> f32 {
	if crate::devtune::f32("flit_on", 1.0) > 0.0 { 1.0 } else { 0.0 }
}

/// The light shape: on (`near_share`), sheen (flit.wgsl `flit_add_obj`), debug view, deep (m past the body where the colour is deep),
/// wrap (extra facing past a light's horizon: the warm rim on a side turned away), roll (flit.wgsl `flit_mix`: the added light x
/// 1 / (1 + its peak / roll); 0 = none).
#[derive(Clone, Copy)]
pub(super) struct Shape {
	pub near: f32,
	pub sheen: f32,
	pub debug: f32,
	pub deep: f32,
	pub wrap: f32,
	pub roll: f32,
}

/// The rows of `list` (at most `flitlist::MAX`; a light of a fire with a map reads it), the shape, the maps of `shw`, the grid
/// round `eye`. No light: all zero.
pub(super) fn rows(list: &[Light], sh: Shape, shw: &Shadows, eye: [f32; 3]) -> FlitU {
	let mut out = FlitU::zeroed();
	if list.is_empty() {
		return out;
	}
	for (k, l) in list.iter().take(flitlist::MAX).enumerate() {
		// Lane rfire review: a fire light's reach is negative (flit.wgsl `flit_light` sums the fire lights apart, `flit_fire_s`; the
		// shader uses only its square otherwise).
		out.lights[2 * k] = [l.pos[0], l.pos[1], l.pos[2], if l.fire { -l.reach } else { l.reach }];
		out.lights[2 * k + 1] = [l.rgb[0], l.rgb[1], l.rgb[2], l.rs.clamp(0.0, 7.9) + 8.0 * slot_code(l, shw)];
	}
	*out.row_mut(SHAPE) = [sh.near.max(0.0), sh.debug, sh.sheen.max(0.0), sh.deep.max(0.0)];
	let (cls, wq) = fire_code(shw);
	*out.row_mut(SHAPE2) = [sh.wrap.clamp(0.0, 1.0), sh.roll.max(0.0), cls, wq];
	let n = shw.idx.iter().map_while(|i| *i).count();
	for s in 0..n {
		*out.row_mut(MAPS + s) = shw.maps[s];
	}
	if n > 0 {
		*out.row_mut(MAPQ) = shw.q;
		*out.row_mut(MAPF) = shw.fade;
		*out.row_mut(MAPSZ) = shw.size;
		*out.row_mut(MAPB) = shw.bias;
		*out.row_mut(MAPZN) = shw.near;
	}
	let (g, masks) = flitlist::grid(list, eye);
	*out.row_mut(GRID) = g;
	out.grid = masks;
	out
}

/// A light's shadow code (flit.wgsl `flit_light`): slot + 5 x second slot + 25 x round(40 x its blend share): slot s = map s - 1
/// (0 none) of its fire's nearest origin, the second = the next nearest's (fire light v2 stage 6, flitsplit.rs `Light::alt`, share
/// 0..0.5). A light whose nearest origin holds no map reads the second one's alone.
pub(super) fn slot_code(l: &Light, shw: &Shadows) -> f32 {
	let slot = |g: Option<u8>| g.and_then(|g| shw.idx.iter().position(|i| *i == Some(g as usize))).map_or(0, |s| s + 1);
	let (a, (b, w)) = (slot(l.group), l.alt.map_or((0, 0.0), |(g, w)| (slot(Some(g)), w)));
	let (a, b, w) = if a == 0 { (b, 0, 0.0) } else if b == 0 { (a, 0, 0.0) } else { (a, b, w) };
	let q = (w.clamp(0.0, 0.5) * 40.0).round() as usize;
	(a + 5 * b + 25 * if b > 0 { q } else { 0 }) as f32
}

/// Row `SHAPE2` .z / .w (flit.wgsl `flit_fire_vis`): per map m its fire's class (the first map of the same fire, 2 bits at 2m) and
/// its weight (6 bits at 6m: 1 + round(62 x share), 0 = no map; the shader weighs a map q - 1, so a share-0 map counts 0); both 0
/// unless `Shadows::mean` (exact in f32: < 2^24).
pub(super) fn fire_code(shw: &Shadows) -> (f32, f32) {
	if !shw.mean {
		return (0.0, 0.0);
	}
	let (mut cls, mut wq) = (0u32, 0u32);
	for m in 0..MAPN {
		if shw.idx[m].is_none() {
			continue;
		}
		let c = (0..=m).find(|&k| shw.idx[k].is_some() && shw.fire[k] == shw.fire[m]).unwrap_or(m) as u32;
		cls |= c << (2 * m);
		wq |= (1 + (shw.share[m].clamp(0.0, 1.0) * 62.0).round() as u32) << (6 * m);
	}
	(cls as f32, wq as f32)
}

/// Review S1 (fire light v2 stage 6): a fire light whose nearest origin and second origin both hold no map this frame (a new origin
/// waiting for a free track, an origin past the near line, a share-0 one) reads the nearest drawn map of its own fire instead (in the
/// mean mode any map of the fire gives the fire's mean): no lit patch beside the fire's shadowed lights. `origins` = the shadow
/// origins `Light::group` points into (flitsplit.rs; `Group::fire` = their fire). Lights of a fire without any map stay unshadowed.
pub(super) fn fallback(list: &mut [Light], origins: &[flitlist::Group], shw: &Shadows) {
	let mapped = |g: u8| shw.idx.iter().any(|i| *i == Some(usize::from(g)));
	for l in list.iter_mut().filter(|l| l.fire) {
		let Some(o) = l.group.and_then(|g| origins.get(usize::from(g))) else { continue };
		if l.group.is_some_and(mapped) || l.alt.is_some_and(|a| mapped(a.0)) {
			continue;
		}
		let d = |s: usize| (0..3).map(|k| (shw.maps[s][k] - l.pos[k]).powi(2)).sum::<f32>();
		let own = (0..MAPN).filter(|&s| shw.idx[s].is_some_and(|i| origins.get(i).is_some_and(|g| g.fire == o.fire))).min_by(|&a, &b| d(a).total_cmp(&d(b)));
		if let Some(s) = own {
			l.group = shw.idx[s].map(|i| i as u8);
			l.alt = None;
		}
	}
}

/// `rows` with the defaults / RAC_TUNE knobs (flit_on, flit_sheen, flit_debug, flit_deep, flit_wrap, flit_roll); zero when `on` is
/// false (post off, the Classic half).
/// `live`: the Fire row draws (FireFx::draws); Fire Off keeps test build 7's shoulder.
pub(super) fn frame_rows(list: &[Light], on: bool, shw: &Shadows, eye: [f32; 3], live: bool) -> FlitU {
	if !on {
		return FlitU::zeroed();
	}
	use crate::devtune::f32 as t;
	// Lane rfire: on >= 2 selects flit.wgsl's warm-white shoulder (`flit_shoulder_warm`) whenever a fire light is in the list (every GI
	// level; RAC_TUNE flit_warm=0 = test build 7's), plus the adaptation q (on - 2, RAC_TUNE flit_adapt, `ADAPT`); a frame with
	// weapon / effect lights only, or Fire Off (`live` false: the game's own flame billboards light), keeps test build 7's shoulder,
	// so its picture stays byte-identical.
	let warm = live && list.iter().any(|l| l.fire) && t("flit_warm", 1.0) > 0.0;
	let on = if warm { 2.0 + t("flit_adapt", ADAPT).clamp(0.0, 16.0) } else { 1.0 };
	let sh = Shape { near: near_share() * on, sheen: t("flit_sheen", SHEEN), debug: ab_debug(t("flit_debug", 0.0)), deep: t("flit_deep", DEEP), wrap: t("flit_wrap", WRAP), roll: t("flit_roll", ROLL) };
	rows(list, sh, shw, eye)
}

/// [dev] Lane fshade (in-process A/B, one process = one machine load): RAC_TUNE fsh_ab=N cycles every N frames through mode 0 = the
/// shipped path (2-tap filter + quad-shared lookup), 1 = today's (flit_debug -2: 4 taps, per pixel), 2 = 2 taps per pixel (flit_debug
/// -1); prints `fshade ab: mode M` at each switch. Off (0): flit_debug as set.
fn ab_debug(debug: f32) -> f32 {
	use std::sync::atomic::{AtomicU32, Ordering};
	static FRAME: AtomicU32 = AtomicU32::new(0);
	let n = crate::devtune::f32("fsh_ab", 0.0) as u32;
	if n == 0 {
		return debug;
	}
	let f = FRAME.fetch_add(1, Ordering::Relaxed);
	let mode = (f / n) % 3;
	if f % n == 0 {
		eprintln!("fshade ab: mode {mode}");
	}
	[0.0, -2.0, -1.0][mode as usize]
}

/// [PC] lane dlit2-9 (GPU cost): the frame's rows make the shaders read a fire shadow map (the early returns of flit.wgsl `flit_mix` /
/// `flit_light` and flitshadow.wgsl `flit_shadow`: a grid, the pass on, the atlas, a map with its far and fade above 0). False: the
/// pipelines without the lookup can draw the frame with the same picture (metalpipe.rs `set_fls`).
pub(super) fn reads_maps(r: &FlitU) -> bool {
	r.row(GRID)[3] > 0.0 && r.row(SHAPE)[0] > 0.0 && r.row(MAPQ)[2] > 0.0 && (0..MAPN).any(|k| r.row(MAPS + k)[3] > 0.0 && r.row(MAPF)[k] > 0.0)
}

/// [PC] lane fshade worker 7: `reads_maps` and at least one light row codes a map (`slot_code` > 0: flit.wgsl `flit_light` runs the
/// lookup only for `slot > 0`; the second slot only with a first). False with maps read: no lookup runs this frame, so metalpipe.rs
/// draws with the plain module's lookup-on sets (`Fls::Plain`, the pre-lane pipelines) instead of the fire module's.
pub(super) fn lit_maps(r: &FlitU) -> bool {
	reads_maps(r) && (0..flitlist::MAX).any(|k| (r.row(2 * k + 1)[3] / 8.0).floor() as u32 % 5 > 0)
}

/// Default flit_adapt (lane rfire worker 3, user on test build 7: "still kinda uneven"): flit.wgsl `flit_mix` scales the base (sun +
/// ambient) by 1 / (1 + q x the fire light's luminance) where fires light it and gives the luminance it took back in the fire-lit
/// colour's hue (never darker than test build 7, only warmer): the fire outshines the sun, so a fire's pool no longer stops at every
/// sun-shadow edge as a hue edge (deep orange in the shade, tan in the sun); 0 = the plain sum (test build 7). 6 picked from pictures
/// (pp\crop_td_b.jpg: 3 still showed the wreck's sun-shadow edge).
const ADAPT: f32 = 6.0;

/// Default flit_wrap: extra facing past a light's horizon. Fire light v2 (lane dlit3, FIRE_LIGHT_V2.md 2.3): 0, the sphere's own
/// horizon-clipped form factor (flit.wgsl `flit_form`) is the whole facing (was 0.3: half-Lambert, a side turned away still got 50 %).
/// 0.2 since test build 7 (user 2026-11-07, "why do we have shadows near a literal fucking inferno which should cast light"): the
/// ground a little above a fire's foot (the crash site's plateau round its burning dip) sat on the light's horizon and stayed dark.
const WRAP: f32 = 0.2;

/// Default flit_roll (linear light): the added light x 1 / (1 + its peak / roll). Fire light v2: 0 = none (was 1.0: lit and shaded
/// sides converged on the same value, the flat orange hero); the energy is right instead (flitlist.rs GAIN) and the shoulder takes
/// the highlights to white. 2.0 since test build 7 with GAIN 1.5: the near field keeps its gradient (1.0 with GAIN 3.5 went flat
/// orange again) while the stronger light reaches the ground metres out.
const ROLL: f32 = 2.0;

/// Default flit_sheen: F0 of the rough specular share (Schlick, rising to 0.4 at grazing) added to an object's albedo the fire lights
/// (flit.wgsl `flit_add_obj`).
const SHEEN: f32 = 0.08;

/// Default flit_deep: metres past a light's body over which its colour deepens (flit.wgsl: c -> c^2 / peak). Fire light v2: 0 = off
/// (clear air does not redden light over metres; the colour is the flame's blackbody, flitcolour.rs; was 6).
const DEEP: f32 = 0.0;

#[cfg(test)]
mod tests {
	use super::*;

	fn sh() -> Shape {
		Shape { near: 1.0, sheen: 0.04, debug: 0.0, deep: 6.0, wrap: 0.25, roll: 1.5 }
	}

	fn l(x: f32, group: Option<u8>) -> Light {
		Light { pos: [x, 2.0, 3.0], reach: 10.0, rgb: [1.0, 0.5, 0.1], rs: 1.5, group, fire: group.is_some(), area: 1.0, flick: 1.0, alt: None, small: false, carried: false }
	}

	#[test]
	fn the_lights_are_packed_in_order_with_their_maps() {
		let list = [l(1.0, None), l(2.0, Some(1)), l(3.0, Some(0)), l(4.0, Some(2))];
		let mut shw = Shadows { idx: [Some(1), Some(2), None, None], q: [408.0, 5.0, 408.0 / 2048.0, 24.0], bias: [1.5, 0.5, 0.02, 0.0], ..Default::default() };
		(shw.maps[0], shw.maps[1], shw.fade, shw.size) = ([2.0, 2.0, 3.0, 12.0], [4.0, 2.0, 3.0, 9.0], [0.5, 1.0, 0.0, 0.0], [2.0, 1.0, 0.0, 0.0]);
		let r = rows(&list, sh(), &shw, [0.0; 3]);
		assert_eq!(r.row(0), [1.0, 2.0, 3.0, 10.0]);
		assert_eq!(r.row(1), [1.0, 0.5, 0.1, 1.5]);
		assert_eq!(r.row(2), [2.0, 2.0, 3.0, -10.0], "a fire light's reach is negative (the fire bit)");
		assert_eq!(r.row(3), [1.0, 0.5, 0.1, 9.5], "fire 1 reads map 0 (slot 1)");
		assert_eq!(r.row(5)[3], 1.5, "fire 0 has no map");
		assert_eq!(r.row(7)[3], 17.5, "fire 2 reads map 1 (slot 2)");
		assert!((8..SHAPE).all(|i| r.row(i) == [0.0; 4]));
		assert_eq!(r.row(SHAPE), [1.0, 0.0, 0.04, 6.0]);
		assert_eq!(r.row(SHAPE2), [0.25, 1.5, 0.0, 0.0]);
		assert_eq!([r.row(MAPS), r.row(MAPS + 1), r.row(MAPS + 2)], [shw.maps[0], shw.maps[1], [0.0; 4]]);
		assert_eq!([r.row(MAPQ), r.row(MAPF), r.row(MAPSZ), r.row(MAPB)], [shw.q, shw.fade, shw.size, shw.bias]);
		assert_eq!(r.row(GRID)[3], flitlist::CELLS as f32);
		assert!(r.grid.iter().flatten().any(|m| *m == 0b1111));
	}

	#[test]
	fn a_light_between_two_origins_codes_both_maps() {
		let shw = Shadows { idx: [Some(1), Some(2), None, None], ..Default::default() };
		let both = Light { alt: Some((2, 0.5)), ..l(1.0, Some(1)) };
		assert_eq!(slot_code(&both, &shw), (1 + 5 * 2 + 25 * 20) as f32);
		assert_eq!(slot_code(&Light { alt: Some((1, 0.3)), ..l(1.0, Some(0)) }, &shw), 1.0, "nearest without a map: the second alone");
		assert_eq!(slot_code(&Light { alt: Some((3, 0.3)), ..l(1.0, Some(2)) }, &shw), 2.0, "second without a map: the nearest alone");
		let r = rows(&[both], sh(), &shw, [0.0; 3]);
		assert_eq!(r.row(1)[3], 1.5 + 8.0 * 511.0, "f32 keeps the radius exact at the largest code");
	}

	#[test]
	fn the_fire_mean_codes_each_maps_fire_and_share() {
		let mut shw = Shadows { idx: [Some(0), Some(1), Some(2), Some(3)], fire: [0, 0, 1, 0], share: [0.5, 0.25, 1.0, 0.25], ..Default::default() };
		assert_eq!(fire_code(&shw), (0.0, 0.0), "off: per-light maps");
		shw.mean = true;
		let (c, w) = fire_code(&shw);
		assert_eq!(c as u32, 2 << 4);
		assert_eq!(w as u32, 32 | (17 << 6) | (63 << 12) | (17 << 18));
		shw.share = [1.0; 4];
		assert_eq!(fire_code(&shw).1, 16777215.0, "the largest code is exact in f32");
	}

	#[test]
	fn no_lights_or_off_leave_the_uniform_zero() {
		assert_eq!(rows(&[], sh(), &Shadows::default(), [0.0; 3]), FlitU::zeroed());
		assert_eq!(frame_rows(&[l(1.0, None)], false, &Shadows::default(), [0.0; 3], true), FlitU::zeroed());
	}

	/// Review S1: a light whose origins hold no map reads the nearest drawn map of its own fire; another fire's map never.
	#[test]
	fn a_light_without_a_map_falls_back_to_its_fires_other_map() {
		let g = |x: f32, fire: u8| flitlist::Group { pos: [x, 2.0, 3.0], fire, ..Default::default() };
		let origins = [g(-3.0, 0), g(0.0, 0), g(3.0, 0), g(0.5, 1), g(9.0, 2)];
		let mut shw = Shadows { idx: [Some(0), Some(2), Some(4), None], ..Default::default() };
		for (s, x) in [-3.0f32, 3.0, 9.0].into_iter().enumerate() {
			shw.maps[s] = [x, 2.0, 3.0, 10.0];
		}
		let mut list = vec![l(1.0, Some(1)), Light { alt: Some((1, 0.4)), ..l(-2.0, Some(0)) }, l(0.4, Some(3)), l(0.0, None)];
		let before = list.clone();
		fallback(&mut list, &origins, &shw);
		assert_eq!((list[0].group, list[0].alt), (Some(2), None), "nearest drawn map of fire 0");
		assert_eq!(&list[1..], &before[1..], "own map kept; fire 1 has none (fire 2's map is not its); a shot light untouched");
		assert_eq!(slot_code(&list[0], &shw), 2.0);
		assert_eq!(slot_code(&before[0], &shw), 0.0, "without the fallback: no shadow (the lit patch)");
	}

	#[test]
	fn the_maps_are_read_only_with_a_live_map() {
		let list = [l(1.0, Some(0))];
		let mut shw = Shadows { idx: [Some(0), None, None, None], q: [408.0, 5.0, 408.0 / 2048.0, 24.0], ..Default::default() };
		(shw.maps[0], shw.fade[0]) = ([2.0, 2.0, 3.0, 12.0], 1.0);
		assert!(reads_maps(&rows(&list, sh(), &shw, [0.0; 3])));
		assert!(!reads_maps(&FlitU::zeroed()) && !reads_maps(&rows(&list, sh(), &Shadows::default(), [0.0; 3])));
		let faded = Shadows { fade: [0.0; MAPN], ..shw };
		assert!(!reads_maps(&rows(&list, sh(), &faded, [0.0; 3])), "a map faded out is not read");
		let no_atlas = Shadows { q: [0.0; 4], ..shw };
		assert!(!reads_maps(&rows(&list, sh(), &no_atlas, [0.0; 3])));
		assert!(lit_maps(&rows(&list, sh(), &shw, [0.0; 3])));
		let unlit = [l(1.0, None), l(2.0, Some(3))];
		assert!(reads_maps(&rows(&unlit, sh(), &shw, [0.0; 3])) && !lit_maps(&rows(&unlit, sh(), &shw, [0.0; 3])), "maps, but no light codes one");
		assert!(!lit_maps(&rows(&list, sh(), &faded, [0.0; 3])));
	}

	#[test]
	fn the_offset_is_the_tail_of_the_uniform_and_the_shader_twins_match() {
		assert_eq!(OFFSET as usize + std::mem::size_of::<FlitU>(), std::mem::size_of::<super::super::Uniforms>());
		assert_eq!(std::mem::size_of::<FlitU>(), (ROWS + MASKS) * 16);
		assert!(super::super::SHADER.contains(&format!("flit: array<vec4<f32>, {ROWS}>, flit_grid: array<vec4<u32>, {MASKS}>")));
		let w = include_str!("flit.wgsl");
		assert!(w.contains(&format!("const FLIT_SHAPE: u32 = {SHAPE}u;")) && w.contains(&format!("const FLIT_GRID: u32 = {GRID}u;")) && w.contains(&format!("const FLIT_SHAPE2: u32 = {SHAPE2}u;")));
		let s = include_str!("flitshadow.wgsl");
		assert!(s.contains(&format!("const FLS_MAPS: u32 = {MAPS}u;")) && s.contains(&format!("const FLS_MAPQ: u32 = {MAPQ}u;")) && s.contains(&format!("const FLS_SIZE: u32 = {MAPSZ}u;")));
		assert!(s.contains(&format!("const FLS_MAPF: u32 = {MAPF}u;")) && s.contains(&format!("const FLS_BIAS: u32 = {MAPB}u;")));
		assert!(s.contains(&format!("const FLS_MAPN: u32 = {MAPZN}u;")));
		assert_eq!(MAPN, super::super::sunshadow::omni::MAX_LIGHTS);
	}

	/// Twin of flit.wgsl `flit_form` (the sphere's horizon-clipped form factor).
	fn form(c: f64, s: f64) -> f64 {
		if c >= s {
			return c;
		}
		if c <= -s {
			return 0.0;
		}
		let s2 = s * s;
		let st = (1.0 - c * c).max(0.0).sqrt().max(1e-4);
		let x = (1.0 / s2 - 1.0).max(0.0).sqrt();
		let y = (-x * c / st).clamp(-1.0, 1.0);
		let q = st * (1.0 - y * y).max(0.0).sqrt();
		((c * y.acos() - x * q) * s2 + q.atan2(x)).max(0.0) / (std::f64::consts::PI * s2)
	}

	/// The irradiance of a uniform sphere seen under angular radius asin(s) by a receiver whose normal makes acos(c) with the
	/// direction to its centre, / (pi x radiance x s^2): numeric integral of max(n . w, 0) over the sphere's cone.
	fn integral(c: f64, s: f64) -> f64 {
		let (sig, th) = (s.asin(), c.acos());
		let (na, np) = (200, 400);
		let (da, dp) = (sig / na as f64, 2.0 * std::f64::consts::PI / np as f64);
		let mut e = 0.0;
		for i in 0..na {
			let a = (i as f64 + 0.5) * da;
			for j in 0..np {
				let p = j as f64 * dp;
				e += (th.sin() * a.sin() * p.cos() + th.cos() * a.cos()).max(0.0) * a.sin() * da * dp;
			}
		}
		e / (std::f64::consts::PI * s * s)
	}

	/// Fire light v2 (FIRE_LIGHT_V2.md 2.3): the shader's form factor is the sphere's true one at every size (far: a point's cosine;
	/// by the flames; inside: (1 + c) / 2) within 1 % of the peak, and the side turned away from the whole sphere gets nothing.
	#[test]
	fn the_sphere_form_factor_fits_the_integral() {
		for s in [0.05, 0.2, 0.4, 0.6, 0.8, 0.95, 1.0] {
			for k in 0..=40 {
				let c = -1.0 + 2.0 * k as f64 / 40.0;
				let (f, t) = (form(c, s), integral(c, s));
				assert!((f - t).abs() < 0.01, "s {s} c {c}: form {f} integral {t}");
			}
			assert_eq!(form(-s - 1e-3, s), 0.0);
		}
		assert!((form(0.0, 1.0) - 0.5).abs() < 1e-6, "inside the sphere a side at 90 degrees gets half");
		let w = include_str!("flit.wgsl");
		assert!(w.contains("return max((c * acos(y) - x * q) * s2 + atan2(q, x), 0.0) / (3.14159265 * s2);") && w.contains("face = flit_form("));
		assert!(!w.contains("(d2 + rs * rs)"), "v2: d clamped to the body, no softened inverse square");
	}

	/// Twin of flit.wgsl `flit_shoulder_warm` (per-channel knee + the lost luminance lifting towards white).
	fn shoulder(c: [f32; 3]) -> [f32; 3] {
		const KNEE: f32 = 0.7;
		const SPILL: f32 = 0.6;
		let k1 = |x: f32| if x > KNEE { KNEE + (1.0 - KNEE) * (1.0 - (-(x - KNEE) / (1.0 - KNEE)).exp()) } else { x };
		let s = c.map(k1);
		let y = [0.2126, 0.7152, 0.0722];
		let lost = (0..3).map(|i| (c[i] - s[i]) * y[i]).sum::<f32>().max(0.0);
		let w = 1.0 - (-SPILL * lost).exp();
		s.map(|v| v + (1.0 - v) * w)
	}

	/// Lane rfire (user on test build 7: "can we somehow prevent ratchet from going red"): over-exposed warm light walks orange ->
	/// yellow -> cream -> white, never pink: green always keeps its lead over blue, no channel passes 1, the red share falls as the
	/// light grows, and below the knee nothing changes.
	#[test]
	fn the_shoulder_goes_warm_white_never_pink() {
		let w = include_str!("flit.wgsl");
		assert!(w.contains("const FLIT_KNEE: f32 = 0.7;") && w.contains("const FLIT_SPILL: f32 = 0.6;"), "twin constants");
		// The twin's body (review: test the shader's formula, not only its constants).
		for line in [
			"return select(x, FLIT_KNEE + r * (1.0 - exp(-(x - FLIT_KNEE) / r)), x > FLIT_KNEE);",
			"let s = vec3<f32>(flit_knee1(c.r), flit_knee1(c.g), flit_knee1(c.b));",
			"let lost = max(dot(c - s, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0);",
			"return s + (vec3<f32>(1.0) - s) * (1.0 - exp(-FLIT_SPILL * lost));",
		] {
			assert!(w.contains(line), "flit_shoulder_warm twin: {line}");
		}
		assert_eq!(shoulder([0.6, 0.3, 0.1]), [0.6, 0.3, 0.1], "below the knee: untouched");
		// Warm fur (cream albedo) under a 1900 K light of growing strength (flitcolour.rs: [2.59, 0.63, 0]).
		let alb = [0.8f32, 0.6, 0.35];
		let mut last_red_share = f32::MAX;
		for k in 1..=40 {
			let i = 0.25 * k as f32;
			let c = [alb[0] * 2.59 * i, alb[1] * 0.63 * i, alb[2] * 0.02 * i];
			let s = shoulder(c);
			assert!(s.iter().all(|v| *v <= 1.0 + 1e-6), "{i}: {s:?}");
			assert!(s[1] - s[2] >= -1e-6, "{i}: green leads blue (not pink/magenta): {s:?}");
			// Pink = red far ahead while blue comes up as much as green: the gap red - green must not exceed green - blue by much
			// once blue rises (the old peak mix gave (1, 0.74, 0.67)).
			if s[2] > 0.3 {
				assert!(s[1] - s[2] > 0.5 * (s[0] - s[1]), "{i}: cream / yellow, not salmon: {s:?}");
			}
			let share = s[0] / (s[0] + s[1] + s[2]);
			assert!(share <= last_red_share + 1e-5, "{i}: brighter = less red");
			last_red_share = share;
		}
		let hot = shoulder([30.0, 7.0, 0.2]);
		assert!(hot.iter().all(|v| *v > 0.75), "very bright: near white {hot:?}");
	}

	#[test]
	fn the_level_shader_with_the_fire_light_validates() {
		assert!(super::super::SHADER.contains("fn flit_add") && super::super::SHADER.matches("flit_add(").count() + super::super::SHADER.matches("flit_add_obj(").count() >= 12);
		let src = super::super::volfog::soft::level_source(super::super::SHADER, false);
		let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("level: {}", e.emit_to_string(&src)));
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	}

	/// Twin of flit.wgsl test build 7's `flit_shoulder` (the peak rolls off, the colour mixes towards white of it).
	fn shoulder_tb7(c: [f32; 3]) -> [f32; 3] {
		let m = c[0].max(c[1]).max(c[2]);
		if m <= 0.7 {
			return c;
		}
		let mc = 0.7 + 0.3 * (1.0 - (-(m - 0.7) / 0.3).exp());
		let f = 1.0 - mc / m;
		c.map(|v| v * (mc / m) * (1.0 - f) + mc * f)
	}

	fn lum(c: [f32; 3]) -> f32 {
		0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
	}

	/// Twin of flit.wgsl `flit_warm` (lane rfire worker 3 + review): base0 / add / fadd linear, `lf` = the fire light's luminance.
	fn warm(base0: [f32; 3], add: [f32; 3], fadd: [f32; 3], lf: f32, q: f32) -> [f32; 3] {
		let ss = |e0: f32, e1: f32, x: f32| {
			let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
			t * t * (3.0 - 2.0 * t)
		};
		let (la, lb) = (lum(fadd), lum(base0));
		let k = (1.0 - 1.0 / (1.0 + q.max(0.0) * lf)) * (1.0 - ss(0.18, 0.3, lb));
		let mut b = base0;
		if k > 0.0 && la > 1e-5 {
			b = [0, 1, 2].map(|i| base0[i] * (1.0 - k) + fadd[i] * (k * lb / la));
			let pk = b[0].max(b[1]).max(b[2]);
			let cap = base0[0].max(base0[1]).max(base0[2]).max(0.7);
			if pk > cap {
				b = b.map(|v| v * cap / pk);
			}
		}
		let (sa, s0) = (shoulder([0, 1, 2].map(|i| b[i] + add[i])), shoulder(base0));
		[0, 1, 2].map(|i| (base0[i] + sa[i] - s0[i]).min(base0[i].max(1.0)))
	}

	/// Twin of flit.wgsl `flit_mix` past the debug views (linear in, linear out): base0 = sun + ambient on the albedo, `a` the albedo
	/// (+ rim), `s` all the light, `sf` the fire lights' part, `x` = FLIT_SHAPE .x.
	fn mix_px(base0: [f32; 3], a: [f32; 3], s: [f32; 3], sf: [f32; 3], roll: f32, x: f32) -> [f32; 3] {
		let as0 = [0, 1, 2].map(|i| a[i] * s[i]);
		let pk = as0[0].max(as0[1]).max(as0[2]);
		let rd = if roll > 0.0 { 1.0 + pk / roll } else { 1.0 };
		let add = as0.map(|v| v / rd);
		let (sa, s0) = (shoulder_tb7([0, 1, 2].map(|i| base0[i] + add[i])), shoulder_tb7(base0));
		let lit = [0, 1, 2].map(|i| base0[i] + sa[i] - s0[i]);
		let fs = (lum(sf) / lum(s).max(1e-6)).clamp(0.0, 1.0);
		if x > 1.5 && fs > 0.0 {
			let w = warm(base0, add, [0, 1, 2].map(|i| a[i] * sf[i] / rd), lum(sf), x - 2.0);
			return [0, 1, 2].map(|i| lit[i] * (1.0 - fs) + w[i] * fs);
		}
		lit
	}

	/// The fire light (1900 K, flitcolour.rs) of luminance `y`.
	fn fire(y: f32) -> [f32; 3] {
		let c = [2.59f32, 0.63, 0.02];
		c.map(|v| v * y / lum(c))
	}

	fn red_share(c: [f32; 3]) -> f32 {
		c[0] / (c[0] + c[1] + c[2])
	}

	#[test]
	fn the_warm_mix_twin_is_the_shader() {
		let w = include_str!("flit.wgsl");
		for line in [
			"let k = (1.0 - 1.0 / (1.0 + max(q, 0.0) * lf)) * (1.0 - smoothstep(FLIT_ADAPT_LO, FLIT_ADAPT_HI, lb));",
			"b = base0 * (1.0 - k) + fadd * (k * lb / la);",
			"let cap = max(max(base0.r, max(base0.g, base0.b)), FLIT_KNEE);",
			"let c = base0 + (flit_shoulder_warm(b + add) - flit_shoulder_warm(base0));",
			"return min(c, max(base0, vec3<f32>(1.0)));",
			"const FLIT_ADAPT_LO: f32 = 0.18;",
			"const FLIT_ADAPT_HI: f32 = 0.3;",
			"let fs = clamp(dot(flit_fire_s, FLIT_LUM) / max(ls, 1e-6), 0.0, 1.0);",
			"if (u.flit[FLIT_SHAPE].x > 1.5 && fs > 0.0) {",
			"return pow(max(mix(lit, warm, fs), vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));",
			"if (a.w < 0.0) { flit_fire_s += col * e; }",
		] {
			assert!(w.contains(line), "flit_mix twin: {line}");
		}
	}

	/// Lane rfire review (fix 1): on bright bases (sunlit fur, light sand, snow: luminance 0.3..0.8) the warm mix never pushes a
	/// channel past 1 and is never redder than without the adaptation (q 0); the dark sunlit sand still takes the fire's hue.
	#[test]
	fn the_fire_outshines_the_sun_without_reddening_bright_surfaces() {
		let q = ADAPT;
		let bases = [[0.5f32, 0.27, 0.08], [0.55, 0.4, 0.2], [0.7, 0.55, 0.3], [0.8, 0.8, 0.82], [0.9, 0.78, 0.6], [0.95, 0.95, 0.95]];
		for b in bases {
			assert!((0.3..=0.96).contains(&lum(b)), "{b:?} lum {}", lum(b));
			for alb in [[0.8f32, 0.55, 0.3], [0.9, 0.9, 0.9]] {
				for k in 1..=24 {
					let s = fire(0.25 * k as f32);
					let (adapted, plain) = (mix_px(b, alb, s, s, ROLL, 2.0 + q), mix_px(b, alb, s, s, ROLL, 2.0));
					assert!(adapted.iter().all(|v| *v <= 1.0 + 1e-6), "{b:?} {s:?}: {adapted:?}");
					assert!(red_share(adapted) <= red_share(plain) + 1e-5, "{b:?} {s:?}: {adapted:?} redder than {plain:?}");
				}
			}
		}
		// Sunlit sand at the crash site (~0.1 linear): the pool's hue crosses the sun-shadow edge (worker 3's fix kept), luminance
		// not below the plain sum's.
		let (sand, alb) = ([0.25f32, 0.094, 0.013], [0.45f32, 0.3, 0.15]);
		let s = fire(1.0);
		let (adapted, plain) = (mix_px(sand, alb, s, s, ROLL, 2.0 + q), mix_px(sand, alb, s, s, ROLL, 2.0));
		assert!(red_share(adapted) > red_share(plain) + 0.02, "{adapted:?} vs {plain:?}");
		assert!(lum(adapted) >= 0.95 * lum(plain) && adapted.iter().all(|v| *v <= 1.0), "{adapted:?} vs {plain:?}");
		// No fire light: untouched; k 0 (q 0) = the warm shoulder of the plain sum (continuous).
		assert_eq!(warm(sand, [0.0; 3], [0.0; 3], 0.0, q), sand);
	}

	/// Lane rfire review (fix 2): the warm mix goes by the FIRE lights' share per pixel: a pixel lit by a weapon light alone keeps
	/// test build 7's result exactly in a frame with fires, a pixel lit by both blends between the two, so nothing pops.
	#[test]
	fn weapon_light_keeps_test_build_7s_shoulder_per_pixel() {
		let (b, alb) = ([0.2f32, 0.12, 0.05], [0.7f32, 0.5, 0.3]);
		let shot = [0.4f32, 0.8, 1.6];
		let tb7 = mix_px(b, alb, shot, [0.0; 3], ROLL, 1.0);
		assert_eq!(mix_px(b, alb, shot, [0.0; 3], ROLL, 2.0 + ADAPT), tb7, "weapon-only pixel in a fire frame: test build 7");
		let f = fire(1.0);
		let both = [0, 1, 2].map(|i| f[i] + shot[i]);
		let (fire_only, mixed) = (mix_px(b, alb, f, f, ROLL, 2.0 + ADAPT), mix_px(b, alb, both, f, ROLL, 2.0 + ADAPT));
		let tb7_both = mix_px(b, alb, both, f, ROLL, 1.0);
		assert!(mixed.iter().all(|v| v.is_finite() && *v <= 1.0) && fire_only.iter().all(|v| *v <= 1.0));
		// A weak fire light beside a strong weapon light moves the pixel only a little from test build 7.
		let weak = fire(0.01);
		let near_shot = [0, 1, 2].map(|i| weak[i] + shot[i]);
		let d = (0..3).map(|i| (mix_px(b, alb, near_shot, weak, ROLL, 2.0 + ADAPT)[i] - mix_px(b, alb, near_shot, weak, ROLL, 1.0)[i]).abs()).fold(0.0f32, f32::max);
		assert!(d < 0.01, "no pop as a fire light enters: {d}");
		assert_ne!(mixed, tb7_both);
	}

	/// Row SHAPE .x (flit.wgsl `flit_mix`): weapon lights only = 1 (test build 7), Fire Off (`live` false) = 1, a fire light while the
	/// Fire row draws = 2 + flit_adapt, the pass off = all zero.
	#[test]
	fn frame_rows_pick_the_shoulder() {
		let x = |list: &[Light], on: bool, live: bool| frame_rows(list, on, &Shadows::default(), [0.0; 3], live).row(SHAPE)[0];
		let (shot, fire) = (l(1.0, None), l(2.0, Some(0)));
		assert_eq!(x(&[shot], true, true), 1.0, "weapon lights only");
		assert_eq!(x(&[shot, fire], true, false), 1.0, "Fire Off");
		assert_eq!(x(&[shot, fire], true, true), 2.0 + ADAPT, "fire + live");
		assert_eq!(frame_rows(&[shot, fire], false, &Shadows::default(), [0.0; 3], true), FlitU::zeroed(), "flit off");
	}
}
