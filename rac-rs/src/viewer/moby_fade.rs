//! Moby draw verdicts: the dynamic-light id table, the alpha / baked-colour encodings, the draw-distance fades and
//! the metal alpha (MOBY-FADE, SCH-6b, NOV-35b), with their tests.

use super::*;

/// Registers the third light a moby at `origin` takes (MobyProc, `dynlight::moby_light`) and returns its 1-based id
/// for the vertex colour alpha, 0 when no light reaches it.
pub(super) fn dyn_id(table: &[Option<dynlight::SlotLight>; 8], list: &mut Vec<([f32; 3], [f32; 3])>, origin: [f32; 3]) -> f32 {
	if list.len() >= MAX_DYN {
		return 0.0;
	}
	match dynlight::moby_light(origin, table) {
		Some(l) => {
			list.push(l);
			list.len() as f32
		}
		None => 0.0,
	}
}


pub(super) fn moby_alpha_encoding(id: f32, alpha: u8) -> (f32, bool) {
	(id + (128.0 - alpha as f32) / 512.0, alpha < 0x80)
}

/// The tinted tail of a faded moby: baked -2 colour code with the draw alpha as a fraction below it (0x80 = exactly -2.0).
pub(super) fn moby_baked_encoding(alpha: u8) -> f32 {
	-2.0 - (128.0 - alpha as f32) / 512.0
}

/// Moby draw-distance fade (FUN_0026a338): `depth1024` is the camera forward distance of the moby's sphere centre in 1/1024 m,
/// the limit `draw` is clamped to 500 m. None = beyond the limit (not drawn), else the fade byte 0..=0x80.
/// Entries of the moby draw's deferred list per frame.
pub(super) const DEFERRED_CAP: usize = 48;

pub(super) fn moby_fade(draw: i16, depth1024: f32) -> Option<u8> {
	let y = ((draw as i32).min(500) << 10) as f32 - depth1024;
	if y < 0.0 {
		return None;
	}
	Some(((y as i32 as u32) >> 7).min(0x80) as u8)
}

/// Fade byte of a moby on the deferred-list path (flags & 0xc00, in-game frame): `(L << 10) - trunc(z - w)` with L the unclamped
/// draw distance and `w` the sphere radius, >> 7 and capped at 0x80 (SCH-6b; the cull itself already used `moby_fade`).
pub(super) fn moby_fade_alt(draw: i16, z: f32, w: f32) -> u8 {
	let y = ((draw as i32) << 10).wrapping_sub((z - w) as i32);
	(y >> 7).clamp(0, 0x80) as u8
}

/// SCH-6b: the draw-distance verdict of an effect moby or debris piece centred at `pos` (metres): None = culled,
/// else the fade byte (0x80 without a draw distance or a view). The centre is `pos * 1024` [I].
pub(super) fn effect_fade(view: Option<([f32; 3], [f32; 3])>, draw: Option<i16>, pos: [f32; 3]) -> Option<u8> {
	let (Some((e, f)), Some(d)) = (view, draw) else { return Some(0x80) };
	let z: f32 = (0..3).map(|k| f[k] * (pos[k] * 1024.0 - e[k] * 1024.0)).sum();
	moby_fade(d, z)
}

/// The moby header alpha: fade times the moby's +0x23 byte, >> 7.
pub(super) fn moby_draw_alpha(fade: u8, alpha: u8) -> u8 {
	((fade as u32 * alpha as u32) >> 7) as u8
}

/// NOV-35b metal alpha (FUN_0026a338 raw 0x26b498-0x26b4c4, moby_format.md 4.1): with t0 = trunc(z - w) (the sphere's view depth
/// and radius, 1/1024 m), d = (metal << 10) - t0; d <= 0 -> None (no metal part this frame), else min(d >> 7, 0x80). Not scaled by
/// the draw-distance fade or byte +0x23.
pub(super) fn metal_alpha(metal: u8, z: f32, w: f32) -> Option<u8> {
	let d = ((metal as i32) << 10).wrapping_sub((z - w) as i32);
	(d > 0).then(|| ((d as u32) >> 7).min(0x80) as u8)
}

/// NOV-35b: moby +0x73 of a class with metal packets: 0x18 from the spawn init (FUN_00263020), 0x14 for the bolt pickups 13-16
/// (bolt_update FUN_002bb088, reward_bolt_create FUN_002bc6e8), 0x20 for the vendor orb 1143 (FUN_003092c0).
pub(super) fn metal_byte(class_id: u32) -> u8 {
	match class_id {
		13..=16 => 0x14,
		1143 => 0x20,
		_ => 0x18,
	}
}

/// NOV-35b: `Vertex::light.w` of a metal vertex: its colour is built on the CPU (`light.rgb`), the shader adds no light.
pub(super) const METAL_LIGHT: f32 = -4.0;

/// Column-major placement of a moby instance: rotation Z * Y * X (radians), then translation.
/// A tinted prop's vertex `j` from the tint's first vertex on: constant unlit colour (baked code, draw alpha below -2.0), else unchanged.
#[cfg(test)]
pub(super) fn tint_override(colour: [f32; 4], j: usize, tint: Option<([f32; 3], usize)>, alpha: u8) -> [f32; 4] {
	match tint.filter(|&(_, s)| j >= s) {
		Some((c, _)) => [c[0], c[1], c[2], moby_baked_encoding(alpha)],
		None => colour,
	}
}

#[cfg(test)]
mod tint_tests {
	use super::*;

	#[test]
	fn tint_override_starts_at_colour_start() {
		let t = Some(([0.5, 0.5, 0.5], 23));
		assert_eq!(tint_override([1.0, 1.0, 1.0, 0.25], 22, t, 0x80), [1.0, 1.0, 1.0, 0.25]);
		assert_eq!(tint_override([1.0, 1.0, 1.0, 0.25], 23, t, 0x80), [0.5, 0.5, 0.5, -2.0]);
		assert_eq!(tint_override([1.0, 1.0, 1.0, 0.25], 99, None, 0x80), [1.0, 1.0, 1.0, 0.25]);
		// CLASS-tint-alpha test 3: a faded prop (alpha 64) carries its alpha in the tail code.
		assert_eq!(tint_override([1.0; 4], 23, t, 64)[3], -2.125);
	}

	// MOBY-FADE tests 1-6.
	#[test]
	fn moby_fade_distance_terms() {
		assert_eq!(moby_fade(64, 50.0 * 1024.0), Some(112));
		assert_eq!(moby_fade(64, 48.0 * 1024.0), Some(128));
		assert_eq!(moby_fade(64, 64.001 * 1024.0), None);
		assert_eq!(moby_fade(600, 490.0 * 1024.0), Some(80));
		assert_eq!(moby_fade(64, 63.95 * 1024.0), Some(0));
		assert_eq!((moby_draw_alpha(112, 0x80), moby_draw_alpha(64, 64)), (112, 32));
	}

	// SCH-6b: the deferred-list fade uses the unclamped distance and the radius (integers).
	#[test]
	fn moby_fade_alt_uses_radius_and_unclamped_distance() {
		assert_eq!(moby_fade_alt(64, 50.0 * 1024.0, 1024.0), 120);
		assert_eq!(moby_fade_alt(64, 0.0, 0.0), 0x80);
		// L = 600 is not clamped to 500: (600 << 10) - 490 * 1024 = 110 * 1024 -> capped.
		assert_eq!(moby_fade_alt(600, 490.0 * 1024.0, 0.0), 0x80);
		assert_eq!(moby_fade_alt(64, 63.0 * 1024.0, 0.0), 8);
	}

	// SCH-6b: effect mobys and debris pieces (draw 255) take the moby draw-distance verdict, centre pos * 1024.
	#[test]
	fn effect_fade_culls_past_255_metres() {
		let view = Some(([0.0; 3], [1.0, 0.0, 0.0]));
		assert_eq!(effect_fade(view, Some(255), [10.0, 0.0, 0.0]), Some(0x80));
		assert_eq!(effect_fade(view, Some(255), [256.0, 0.0, 0.0]), None);
		assert_eq!(effect_fade(view, Some(255), [254.5, 0.0, 0.0]), Some(4));
		assert_eq!(effect_fade(view, None, [900.0, 0.0, 0.0]), Some(0x80));
		assert_eq!(effect_fade(None, Some(255), [900.0, 0.0, 0.0]), Some(0x80));
	}

	// CLASS-tint-alpha tests 1 and 2.
	#[test]
	fn baked_alpha_encoding_round_trips() {
		for (a, e) in [(0x80u8, -2.0f32), (96, -2.0625), (64, -2.125), (0, -2.25)] {
			assert_eq!(moby_baked_encoding(a), e);
			assert_eq!(128.0 - ((e - (e + 0.5).floor()).abs() * 512.0).round(), a as f32);
		}
		let lit = 3.0625f32;
		assert_eq!(128.0 - ((lit - (lit + 0.5).floor()).abs() * 512.0).round(), 96.0);
	}
}
