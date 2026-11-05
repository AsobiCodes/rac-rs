//! BRIDGE-40 HERO-GHOST: the translucent copies of the hero's base pose in the hero marker (dev s138 `viewer/hero_pose.rs`; it
//! reads the R&C1 player, so it lives in the R&C1 host).

use super::*;

/// BRIDGE-40 HERO-GHOST (WPN-6.6a, trail 0x1409c0): up to three translucent copies of the hero's current base pose at the lagged trail
/// positions and yaws, appended to the marker model; hidden when absent. No layers or joint effects [C]: the retail copy's +0x60 / +0x64
/// lists stay 0 (WPN-6.6b4, decomp/notes/weapons_framework.md). Untinted [C]: its flags 0x80a clear the CLASS-tint bit 0x10, so the hero
/// glow tint covers the hero range only. Lit with the hero's current light, where retail freezes the spawn tick's light word [I].
/// `base_pose` skins the hero's base pose (called only when a copy exists); `hero_n` = vertices of the hero class.
pub(super) fn append_hero_ghosts(ps: &mut Vec<[f32; 3]>, ns: &mut Vec<[f32; 3]>, ghost_ranges: &mut [Option<(usize, usize, u8)>; 5], p: &crate::player::Player, hero_n: usize, base_pose: impl FnOnce() -> Posed) {
	let trail = p.hero_ghosts();
	let ghost_pose = trail.iter().any(Option::is_some).then(|| {
		let (mut gp, mut gn) = base_pose();
		gp.resize(hero_n, [0.0; 3]);
		gn.resize(hero_n, [0.0, 0.0, 1.0]);
		(gp, gn)
	});
	for (gi, g) in trail.into_iter().enumerate() {
		let base = ps.len();
		let (Some((gpos, gyaw, alpha)), Some((gp, gn))) = (g, ghost_pose.as_ref()) else {
			hide(ps, ns, hero_n);
			continue;
		};
		let (c, sn) = (p.yaw.cos(), p.yaw.sin());
		let (gc, gs) = (gyaw.cos(), gyaw.sin());
		let local = |v: [f32; 3], t: f32| {
			let r = [v[0] * gc - v[1] * gs, v[0] * gs + v[1] * gc, v[2]];
			let d = [r[0] + t * (gpos[0] - p.pos[0]), r[1] + t * (gpos[1] - p.pos[1]), r[2] + t * (gpos[2] - p.pos[2])];
			[d[0] * c + d[1] * sn, -d[0] * sn + d[1] * c, d[2]]
		};
		ps.extend(gp.iter().map(|&v| local(v, 1.0)));
		ns.extend(gn.iter().map(|&v| local(v, 0.0)));
		ghost_ranges[2 + gi] = Some((base, ps.len(), alpha));
	}
}
