//! [PC] Fire light of the Thruster-Pack (lane rfire worker 8, user 2026-11-07): while the pack fires (glide, the high jumps, the
//! other firing hero states) its exhaust flames give a small warm light (engine viewer/firefx_carried.rs via `PoolFx::flame_lights`).
//! Retail draws those flames as type-0x15 pool sprites of 0.03..0.06, far under the size from which a flame lights (firefx.rs
//! `SMALL_MIN`), and the same effect type is also the Devastator's impact sparks (size 0.08), which must stay unlit: so the light is
//! keyed on the firing exhaust mobys themselves (class 167 in state 3, pack_fx.rs), never on the sprites. One light per pack, at the
//! mean of its firing nozzles (moved with the pack while strafing, `exhaust_shift`), a little below them where the flames burn.
//! Draw only; RAM is not written.

use super::pack_fx::{EXHAUST, FIRING, THRUSTER_PACK};
use crate::fxpool::FlameLight;

/// The light's flame area (m^2 of flame, the scale of firefx.rs `Emit` h x w; intensity = flitlist.rs flit_gain 1.5 x area):
/// about a 0.2 m flame, under the smallest pool flame that lights (firefx.rs `SMALL_MIN` 0.3: h 0.3 x w 0.33). With the light's
/// body (firefx_carried.rs `BODY` 0.8 m) Ratchet's back takes at most ~0.12 x full sun of warm light, the ground a metre under
/// the nozzles ~0.08.
const AREA: f32 = 0.05;

/// How far under the nozzles the light sits (m): the flames burn down and back from them.
const DROP: f32 = 0.12;

/// The pack's light from its exhaust mobys `(class, state, pack the exhaust belongs to, nozzle F)`: one light at the mean nozzle of
/// those that are class 167 in state 3 (firing) and belong to `pack`; None when none fires.
fn pick(exhausts: impl Iterator<Item = (u16, u8, u32, [f32; 3])>, pack: u32) -> Option<FlameLight> {
	let (mut sum, mut n) = ([0.0f32; 3], 0u32);
	for (class, state, owner, f) in exhausts {
		if class == EXHAUST && state == FIRING && owner == pack {
			(0..3).for_each(|k| sum[k] += f[k]);
			n += 1;
		}
	}
	(n > 0).then(|| {
		let c = sum.map(|v| v / n as f32);
		FlameLight { id: pack, pos: [c[0], c[1], c[2] - DROP], area: AREA }
	})
}

/// The RAM reads of the light (the machine; a byte slice in the tests).
trait Rd {
	fn r8(&self, a: u32) -> u8;
	fn r16(&self, a: u32) -> u16;
	fn r32(&self, a: u32) -> u32;
	fn rv3(&self, a: u32) -> [f32; 3];
}

impl Rd for ee_oracle::Machine {
	fn r8(&self, a: u32) -> u8 {
		self.rd8(a)
	}
	fn r16(&self, a: u32) -> u16 {
		self.rd16(a)
	}
	fn r32(&self, a: u32) -> u32 {
		self.rd32(a)
	}
	fn rv3(&self, a: u32) -> [f32; 3] {
		self.rdv3(a)
	}
}

/// The worn pack (`*0x1404d0`) when it is the Thruster-Pack (class 608).
const WORN_PACK: u32 = 0x1404d0;

/// This field's Thruster-Pack light (at most one) into `out` (cleared first): none without the Thruster-Pack (`*0x1404d0`, class
/// 608) or while it does not fire.
pub fn flame_lights(m: &ee_oracle::Machine, out: &mut Vec<FlameLight>) {
	let span = crate::retail::ram_globals::machine_span(m, m.rd8(crate::retail::ram_globals::LEVEL) as u32);
	lights_in(m, span, |a| super::exhaust_shift(m, a), out);
}

/// `flame_lights` over the moby slots of `span` (first, last; lane rfire worker 9, review L2: the sane span, the last slot too):
/// an exhaust's pack is its pvar +8, its state byte +0x20; `shift` = the strafing move of a nozzle (strafe.rs `turn_parts`).
fn lights_in<R: Rd + ?Sized>(m: &R, span: Option<(u32, u32)>, shift: impl Fn(u32) -> Option<[f32; 3]>, out: &mut Vec<FlameLight>) {
	out.clear();
	let pack = m.r32(WORN_PACK) & 0x01ff_ffff;
	if pack == 0 || m.r16(pack + 0xa6) != THRUSTER_PACK {
		return;
	}
	let walk = span.into_iter().flat_map(|(s, e)| (s..=e).step_by(0x100)).filter(|&a| m.r16(a + 0xa6) == EXHAUST).map(|a| {
		let pv = m.r32(a + 0x78) & 0x01ff_ffff;
		let owner = if pv == 0 { 0 } else { m.r32(pv + 8) & 0x01ff_ffff };
		// While strafing the pack is drawn turned (strafe.rs `turn_parts`): the nozzle moves with it.
		let mut f = m.rv3(a + 0x10);
		if let Some(d) = shift(a) {
			(0..3).for_each(|k| f[k] += d[k]);
		}
		(EXHAUST, m.r8(a + 0x20), owner, f)
	});
	out.extend(pick(walk, pack));
}

#[cfg(test)]
mod tests {
	use super::*;

	const PACK: u32 = 0x1a_0000;

	/// A fake RAM: little-endian bytes.
	impl Rd for [u8] {
		fn r8(&self, a: u32) -> u8 {
			self[a as usize]
		}
		fn r16(&self, a: u32) -> u16 {
			u16::from_le_bytes([self[a as usize], self[a as usize + 1]])
		}
		fn r32(&self, a: u32) -> u32 {
			u32::from_le_bytes(self[a as usize..a as usize + 4].try_into().unwrap())
		}
		fn rv3(&self, a: u32) -> [f32; 3] {
			[0, 4, 8].map(|k| f32::from_bits(self.r32(a + k)))
		}
	}

	/// Review L4: the RAM side: the worn pack (`*0x1404d0`, class at +0xa6 must be 608, not the Heli-Pack's 607), the exhausts'
	/// owner through pvar +8, state byte +0x20 == 3, the last slot of the span included.
	#[test]
	fn the_ram_side_reads_owner_class_and_state() {
		let mut ram = vec![0u8; 0x1c_0000];
		let w32 = |r: &mut Vec<u8>, a: u32, v: u32| r[a as usize..a as usize + 4].copy_from_slice(&v.to_le_bytes());
		let w16 = |r: &mut Vec<u8>, a: u32, v: u16| r[a as usize..a as usize + 2].copy_from_slice(&v.to_le_bytes());
		w32(&mut ram, WORN_PACK, PACK | 0x2000_0000);
		w16(&mut ram, PACK + 0xa6, THRUSTER_PACK);
		// Three slots from 0x1b_0000: an exhaust of the pack firing (the LAST slot), one idle, one of another pack firing.
		let (s, e) = (0x1b_0000u32, 0x1b_0200u32);
		let pvar = |i: u32| 0x1b_8000 + 0x40 * i;
		for (i, (state, owner)) in [(0u8, PACK), (FIRING, PACK + 0x100), (FIRING, PACK)].into_iter().enumerate() {
			let a = s + 0x100 * i as u32;
			w16(&mut ram, a + 0xa6, EXHAUST);
			ram[(a + 0x20) as usize] = state;
			w32(&mut ram, a + 0x78, pvar(i as u32));
			w32(&mut ram, pvar(i as u32) + 8, owner);
			for (k, v) in [4.0f32, 5.0, 6.0].into_iter().enumerate() {
				w32(&mut ram, a + 0x10 + 4 * k as u32, v.to_bits());
			}
		}
		let mut out = vec![FlameLight { id: 1, pos: [0.0; 3], area: 1.0 }];
		lights_in(&ram[..], Some((s, e)), |_| None, &mut out);
		assert_eq!(out, vec![FlameLight { id: PACK, pos: [4.0, 5.0, 6.0 - DROP], area: AREA }], "the last slot is read");
		lights_in(&ram[..], Some((s, e)), |_| Some([1.0, 0.0, 0.0]), &mut out);
		assert_eq!(out[0].pos, [5.0, 5.0, 6.0 - DROP], "the strafing shift moves the light");
		// No sane span: nothing (and the old list cleared).
		lights_in(&ram[..], None, |_| None, &mut out);
		assert!(out.is_empty());
		// The Heli-Pack (607) worn: no light.
		w16(&mut ram, PACK + 0xa6, THRUSTER_PACK - 1);
		lights_in(&ram[..], Some((s, e)), |_| None, &mut out);
		assert!(out.is_empty());
		// The state byte only 3 fires: the firing one idle now.
		w16(&mut ram, PACK + 0xa6, THRUSTER_PACK);
		ram[(e + 0x20) as usize] = 2;
		lights_in(&ram[..], Some((s, e)), |_| None, &mut out);
		assert!(out.is_empty());
	}

	/// Only firing exhausts (class 167 in state 3) of the worn pack light, merged into ONE light at their mean nozzle, under it.
	#[test]
	fn only_the_firing_exhausts_of_the_pack_light() {
		let l = [(EXHAUST, FIRING, PACK, [1.0, 2.0, 3.0]), (EXHAUST, FIRING, PACK, [3.0, 2.0, 3.0])];
		let got = pick(l.into_iter(), PACK).expect("firing");
		assert_eq!((got.id, got.pos, got.area), (PACK, [2.0, 2.0, 3.0 - DROP], AREA));
		// Not firing (any other state), another class (a type-0x15 spark's owner, the Heli-Pack's parts), another pack: no light.
		for x in [(EXHAUST, 0, PACK, [0.0; 3]), (EXHAUST, 2, PACK, [0.0; 3]), (EXHAUST, 4, PACK, [0.0; 3]), (166, FIRING, PACK, [0.0; 3]), (EXHAUST, FIRING, PACK + 0x100, [0.0; 3])] {
			assert_eq!(pick([x].into_iter(), PACK), None, "{x:?}");
		}
		// One firing among idle ones: the light sits at that one.
		let mixed = [(EXHAUST, 0, PACK, [9.0; 3]), (EXHAUST, FIRING, PACK, [1.0, 1.0, 1.0])];
		assert_eq!(pick(mixed.into_iter(), PACK).map(|l| l.pos), Some([1.0, 1.0, 1.0 - DROP]));
	}
}
