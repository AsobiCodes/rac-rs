//! Retail's shadow volumes and the ship ground shadow read from RAM (`RetailShadows`, `shadow_snap`, `ship_shadow`),
//! and the smoothed draw lists they feed (`SmoothDraw`).

use super::*;

/// One ship ground shadow: moby +0x10, rows of +0xc0, ship type 0x13e056, alpha after the flight fade, and whether the
/// height is the ground ray (game mode 0x15f5a4 == 6) or pos.z + 0.1.
#[derive(Clone, Copy, Debug)]
pub struct ShipShadow {
	pub pos: [f32; 3],
	pub m: [[f32; 3]; 3],
	pub ship: usize,
	pub alpha: i32,
	pub ground: bool,
}

/// The frustum sphere fade (Eudora FUN_001f2970 -> FUN_001f2878; Novalis FUN_00221e18 -> FUN_00221d20), from the
/// camera registers the frame set up at 0x16d0c0 (Novalis; LEVEL_SHIFT per level, Eudora 0x16cdc0): rows vf25..27,
/// vf24 = eye * 1024 (w 1024), vf20 (f, f*k, ..), vf21, vf22 (sec f, sec f*k, ..), vf23. `buf` = centre and radius in
/// world units (moby[0..3] * 2^-10), `lim` the fade length (32.0). None = culled (-1); else the alpha
/// clamp(ftoi(lim * 1024 - (Vz - R)) >> 7, 0, 0x80). [C] decoded from the raw COP2 words (0x1f2878..0x1f29a8 Eudora).
fn sphere_fade(m: &ee_oracle::Machine, level: u32, buf: [f32; 4], lim: f32) -> Option<i32> {
	let b = va(0x16d0c0u32.wrapping_add_signed(*LEVEL_SHIFT.get(level as usize)?));
	let q = |o: u32| [0, 4, 8, 12].map(|k| m.rdf(b + o + k));
	let (r0, r1, r2, e, v20, v21, v22) = (q(0), q(0x10), q(0x20), q(0x30), q(0x40), q(0x50), q(0x60));
	let p = buf.map(|c| c * e[3]);
	let d = [p[0] - e[0], p[1] - e[1], p[2] - e[2]];
	let v: [f32; 3] = std::array::from_fn(|i| r0[i] * d[0] + r1[i] * d[1] + r2[i] * d[2]);
	let r = p[3];
	let x3 = 0.0 - (r + v[2]);
	let y3 = lim * e[3] - (-r + v[2]);
	if y3.is_sign_negative() || !x3.is_sign_negative() {
		return None;
	}
	let f8 = [v22[0] * r, v22[1] * r];
	let f5 = [v20[0] * v[2], v20[1] * v[2]];
	let f6 = [v[0].abs() - f8[0], v[1].abs() - f8[1]];
	let f7 = [f5[0] - f6[0], f5[1] - f6[1]];
	let _ = v21;
	// [PC] A side reject the wider window still shows is kept (`wide_side::keep`; never in a 4:3 window).
	if (f7[1].is_sign_negative() || f7[0].is_sign_negative()) && !super::wide_side::keep(v, r, v20[0]) {
		return None;
	}
	// vf4 / vf8 only choose the return 0 or 1 (both drawn).
	Some(((y3 as i32 as u32) >> 7).min(0x80) as i32)
}

/// Eudora FUN_0027ee90 (moby `a`): sphere moby[0..3] * 2^-10 through `sphere_fade(32.0)`; game mode 6 with substate
/// 0x13e050 == 3 and tick i16 0x13e054 > scale_ticks(0x96) fades by (tick - 125) * -4 and skips below 1; corners of
/// type i16 0x13e056 (table Eudora 0x1bd9d0 = shadows::SHIP_CORNERS) through the rows of +0xc0 plus +0x10. [C]
pub(super) fn ship_shadow(m: &ee_oracle::Machine, level: u32, a: u32) -> Option<ShipShadow> {
	let buf = [0, 4, 8, 12].map(|o| m.rdf(a + o) * (1.0 / 1024.0));
	let mut alpha = sphere_fade(m, level, buf, 32.0)?;
	let ground = m.rd32(va(0x15f5a4)) == 6;
	if ground && m.rd32(0x13e050) == 3 {
		let (t, s) = (m.rd16(0x13e054) as i16 as i32, crate::gameplay::pal_ticks(0x96));
		if s < t {
			alpha += (t - s) * -4;
			if alpha < 1 {
				return None;
			}
		}
	}
	Some(ShipShadow { pos: m.rdv3(a + 0x10), m: [m.rdv3(a + 0xc0), m.rdv3(a + 0xd0), m.rdv3(a + 0xe0)], ship: m.rd16(0x13e056) as i16 as usize, alpha, ground })
}

/// M3a shadows: retail's moby shadow volumes of the last drawn frame (decomp/notes/shadows.md "Moby shadow volumes").
#[derive(Clone, Debug, Default)]
pub struct RetailShadows {
	/// (caster moby, its record) in caster-list order.
	pub casters: Vec<(u32, crate::shadows::RecCaster)>,
	/// Mobys on retail's deferred list (the casters and the drawn +0x34 & 0x800 mobys), drawn after the volumes and the darken.
	pub deferred: Vec<u32>,
	/// The hero moby is on the deferred list (a caster, or +0x34 & 0x800).
	pub hero_deferred: bool,
}

/// [PC] Frame smoothing (viewer/interp.rs): the draw lists of a display frame between two fields (`Snap::smooth`); None = the
/// field's own.
#[derive(Default)]
pub struct SmoothDraw {
	pub fx: Option<RetailFx>,
	pub shadows: Option<RetailShadows>,
	pub orbs: Option<Vec<crate::particles::OrbDraw>>,
	/// The hero's position at the field before and at the newest one, in game view (the camera follows him): `Retail::camera` blends
	/// the eye on the arc round him (`camera_arc`). None = the straight blend.
	pub cam_hero: Option<([f32; 3], [f32; 3])>,
}

impl RetailShadows {
	fn push_deferred(&mut self, moby: u32, hero: bool) {
		if hero {
			self.hero_deferred = true;
		} else if !self.deferred.contains(&moby) {
			self.deferred.push(moby);
		}
	}
}

/// The shadow caster list of each level (8 B: moby, fade u16, straddle u16; the records follow at + 0x180): the
/// immediate the record build loads (Eudora FUN_002787e0 0x1ac700, Blarg FUN_00294170 0x1acd00) [C: all 19 overlays].
/// Rilgar and Kalebo III sit 0x80 past the camera-globals shift (`LEVEL_SHIFT`), the rest follow it.
const SHADOW_LIST: [u32; 19] = [
	0x1ac600, 0x1aca00, 0x1acb80, 0x1ac680, 0x1ac700, 0x1aca80, 0x1acd00, 0x1ac600, 0x1acd80, 0x1ac700, 0x1aca00, 0x1acf80, 0x1aca00, 0x1ac880,
	0x1acd00, 0x1acb80, 0x1aca00, 0x1ace80, 0x1acf80,
];

/// `SHADOW_LIST` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const SHADOW_LIST_USA: [u32; 19] = [
	0x1ac600, 0x1aca80, 0x1acc00, 0x1ac700, 0x1ac780, 0x1aca80, 0x1acd80, 0x1ac680, 0x1ace00, 0x1ac780, 0x1aca80, 0x1ad000, 0x1aca80, 0x1ac900,
	0x1acd00, 0x1acc00, 0x1aca80, 0x1acf00, 0x1ad000,
];

/// The shadow pass gate the driver's caller tests (Eudora FUN_00242850 `*0x16a118`, Blarg FUN_0025e7d8 `*0x16a718`) [C: all
/// 19 overlays; = 0x16a118 + LEVEL_SHIFT[level] - LEVEL_SHIFT[4]].
const SHADOW_GATE: [u32; 19] = [
	0x16a018, 0x16a418, 0x16a598, 0x16a098, 0x16a118, 0x16a418, 0x16a718, 0x16a018, 0x16a798, 0x16a118, 0x16a418, 0x16a998, 0x16a418, 0x16a298,
	0x16a718, 0x16a598, 0x16a398, 0x16a898, 0x16a998,
];

/// `SHADOW_GATE` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const SHADOW_GATE_USA: [u32; 19] = [
	0x16a018, 0x16a498, 0x16a618, 0x16a118, 0x16a198, 0x16a418, 0x16a798, 0x16a098, 0x16a818, 0x16a198, 0x16a498, 0x16aa18, 0x16a498, 0x16a318,
	0x16a718, 0x16a618, 0x16a418, 0x16a918, 0x16aa18,
];

/// The caster count and the shadow DMA slot the driver tests: 0x15ff7c / 0x15ff80, 0x40 higher on levels 0 5 14 16 (Rilgar
/// driver: 0x15ffbc / 0x15ffc0) [C].
fn shadow_count(level: u32) -> (u32, u32) {
	if matches!(level, 0 | 5 | 14 | 16) { (va(0x15ffbc), va(0x15ffc0)) } else { (va(0x15ff7c), va(0x15ff80)) }
}

/// M3a shadows: the Eudora shadow driver 0x242500 (Novalis FUN_002645b0) runs from 0x242850 when `*0x16a118 != 0` and the
/// shadow DMA slot `*0x15ff80 != 0`; with caster count `*0x15ff7c` > 0 it builds one record per caster-list entry
/// (0x1ac700, 8 B: moby, fade u16, straddle u16) at 0x1ac880 (shadow_records_build 0x2787e0) and draws them
/// (0x2785e8) [C: Eudora disassembly; the Novalis addresses shifted by -0x300 as LEVEL_SHIFT]. The records stay in RAM
/// after the frame, so the last drawn frame's world-space primitives are read as they are. Every level runs the same
/// driver twin with its own addresses (`SHADOW_LIST`, `SHADOW_GATE`, `shadow_count`) [C: all 19 overlays, blarg_fx.md].
pub(super) fn shadow_snap(m: &ee_oracle::Machine, level: u32) -> RetailShadows {
	let mut out = RetailShadows::default();
	let (Some(&list), Some(&gate)) = ((*crate::retail::disc::pick(&SHADOW_LIST, &SHADOW_LIST_USA)).get(level as usize), (*crate::retail::disc::pick(&SHADOW_GATE, &SHADOW_GATE_USA)).get(level as usize)) else {
		return out;
	};
	let (count, slot) = shadow_count(level);
	let (s, e) = table(level);
	let (mut a, e) = (m.rd32(s) & 0x01ff_ffff, m.rd32(e) & 0x01ff_ffff);
	while a < e {
		// 0x800 mobys always take the deferred list when drawn; a 0x400 moby only as a caster (below), else it is drawn at
		// once (Novalis 0x26b268-0x26b408, collision_and_gameplay.md SCH-6b) [C].
		if m.rd32(a + 0x24) != 0 && m.rd8(a + 0x20) < 0xfd && m.rd16(a + 0x34) & 0x800 != 0 {
			out.push_deferred(a, m.rd16(a + 0xa6) == 0);
		}
		a += 0x100;
	}
	if m.rd32(gate) == 0 || m.rd32(slot) == 0 {
		return out;
	}
	let mut r = list + 0x180;
	'list: for i in 0..m.rd32(count).min(48) {
		let moby = m.rd32(list + 8 * i) & 0x01ff_ffff;
		let n = m.rd32(r);
		if n > 64 {
			break;
		}
		let mut rc = crate::shadows::RecCaster { straddle: m.rd32(r + 4) != 0, band: [m.rdf(r + 8), m.rdf(r + 12)], light: m.rdv3(r + 16), prims: Vec::new() };
		r += 0x20;
		for _ in 0..n {
			// [I] guard: the record area's end (Eudora 0x1ae800, Blarg 0x1aee00 = list + 0x2100 [C]).
			if r >= list + 0x2100 {
				break 'list;
			}
			match m.rd32(r) {
				0 => {
					rc.prims.push(crate::shadows::RecPrim::Sphere { c: m.rdv3(r + 0x10), r: m.rdf(r + 0x1c), n: m.rd32(r + 4) });
					r += 0x20;
				}
				1 => {
					rc.prims.push(crate::shadows::RecPrim::Capsule {
						p0: m.rdv3(r + 0x10),
						r0: m.rdf(r + 0x1c),
						p1: m.rdv3(r + 0x20),
						r1: m.rdf(r + 0x2c),
						n0: m.rd32(r + 4),
						n1: m.rd32(r + 8),
					});
					r += 0x30;
				}
				_ => break 'list,
			}
		}
		out.push_deferred(moby, m.rd16(moby + 0xa6) == 0);
		out.casters.push((moby, rc));
	}
	out
}
