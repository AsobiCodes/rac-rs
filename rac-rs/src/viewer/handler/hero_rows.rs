//! Hero model rows (body tilt) and the hero joint override records, with their tests.

use super::*;

/// BRIDGE-75 (decomp/notes/port_bridges.md): the hero model rows R = euler_m([roll, pitch, yaw]) of the game tick; a
/// model vector v goes to the world as v[0] R[0] + v[1] R[1] + v[2] R[2]. Untilted it is the yaw-only turn.
pub(super) fn hero_rows(p: &crate::player::Player) -> [[f32; 3]; 3] {
	if p.body_tilt == [0.0; 2] {
		let (s, c) = p.yaw.sin_cos();
		[[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]
	} else {
		particles::euler_m([p.body_tilt[0], p.body_tilt[1], p.yaw])
	}
}

/// M3a: the hero's joint override records and weighted layers for the draw: retail's (+0x64 records, `HeroParts`) in
/// retail mode, else the port's player state.
pub(super) fn hero_layers(parts: Option<&crate::retail::HeroParts>, p: &crate::player::Player, map: &[Option<usize>]) -> (Vec<anim::JointFx>, Option<anim::LayerPose>, Option<anim::LayerPose>, [Option<anim::LayerPose>; 2]) {
	match parts {
		Some(q) => {
			// Modern controls: a backpedal fades out retail's leg bends, made for the legs facing the path (retail/strafe.rs).
			let mut fx = q.hero_fx.clone();
			crate::retail::strafe::leg_fx(&mut fx, q.strafe);
			fx.extend_from_slice(&q.pose_ops);
			(fx, None, None, [None, None])
		}
		None => (p.hero_joint_fx(0, map), p.throw_layer.map(|l| l.pose()), p.weapon_layer1.map(|l| l.pose()), p.hold_poses()),
	}
}

/// Modern controls: the `PoseFix` (retail/strafe.rs) of the drawn skin matrices `drawn` against the hero pose retail ticked: its
/// own clock (`clock`, before `draw_clock` reversed the rig's) and FROM frame, its leg bends; None while the strafe draw is off.
pub(super) fn strafe_fix(rig: &anim::PropAnim, a: &anim::Animated, q: &crate::retail::HeroParts, clock: Option<crate::retail::Clock>, drawn: &[[f32; 12]]) -> Option<crate::retail::strafe::PoseFix> {
	crate::retail::strafe::pose_fix(q.strafe, a, drawn, || {
		let mut l = rig.clone();
		if let Some(c) = clock {
			let k = &mut l.clock;
			(k.cur, k.next, k.old_seq, k.new_seq, k.p, k.m, k.r) = (c.cur as i32, c.next as i32, c.old_seq, c.new_seq, c.p, c.m, c.r);
		}
		let joints = a.skeleton.parents.len();
		if !q.from_frame.is_empty() && joints > 0 {
			l.from_ram = anim::parse_frame(&q.from_frame, 0, joints).ok();
		}
		let mut fx = q.hero_fx.clone();
		fx.extend_from_slice(&q.pose_ops);
		l.pose_held(a, &fx, None, None, &[None, None])
	})
}

/// Hero model -> world offset through `hero_rows`.
pub(super) fn hero_to_world(r: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
	[v[0] * r[0][0] + v[1] * r[1][0] + v[2] * r[2][0], v[0] * r[0][1] + v[1] * r[1][1] + v[2] * r[2][1], v[0] * r[0][2] + v[1] * r[1][2] + v[2] * r[2][2]]
}

/// World offset -> hero model (the transpose of `hero_to_world`).
pub(super) fn hero_to_model(r: &[[f32; 3]; 3], d: [f32; 3]) -> [f32; 3] {
	[d[0] * r[0][0] + d[1] * r[0][1] + d[2] * r[0][2], d[0] * r[1][0] + d[1] * r[1][1] + d[2] * r[1][2], d[0] * r[2][0] + d[1] * r[2][1] + d[2] * r[2][2]]
}

/// [PC] Where the held Pyrocitor is drawn (hero model space, 3x4 row-major like `hand`, its hero joint): at retail's held basis
/// (`held_basis_model`) while retail holds one, else at `hand`. Before, it was always drawn at the hand joint and its flame and
/// pilot light (placed by retail on its own basis) floated off it while Ratchet turned. While aiming (Modern camera) it stays at
/// `hand`: retail turns the held basis after the hero's logic facing over a few ticks, and that facing jumps a quarter or half
/// turn at each A/D switch of the strafe, so the gun swung round the steady drawn hand (user report 2026-10-24); the pilot
/// light then rides from retail's basis onto the hand with it (`Smooth::put_gun_fix`).
pub(super) fn pyro_basis(p: &crate::player::Player, retail: Option<&crate::retail::Retail>, smooth: &mut crate::viewer::smooth::Smooth, hand: [f32; 12]) -> [f32; 12] {
	let held = retail.and_then(|r| r.held_addr().and_then(|h| r.live_mobys().iter().find(|m| m.addr == h && m.class == crate::game::pyro::PYRO_CLASS)));
	let basis = held.map(|m| held_basis_model(p, m, smooth.pose_fix_now(), &hand));
	if crate::retail::modcam::aim_yaw().is_none() {
		return basis.unwrap_or(hand);
	}
	smooth.put_gun_fix(basis.and_then(|b| crate::retail::strafe::mul_inv34(&hand, &b)));
	hand
}

/// [PC] Retail turns a held weapon about the grip on its own: the Pyrocitor's basis trails Ratchet's hand while he turns (about
/// 0.25 rad at a brisk mouse turn, ngplus/0047), and its flame, pilot light and stream leave from that basis. The held moby's draw
/// basis (`held`, as the snapshot turned it to the drawn yaw) in hero model space as a 3x4 row-major joint matrix, carried onto the
/// drawn pose by the strafe pose fix; the origin stays `hand`'s (the drawn hand joint, where it sits anyway). Draw only.
fn held_basis_model(p: &crate::player::Player, held: &crate::retail::Moby, fix: Option<&crate::retail::strafe::PoseFix>, hand: &[f32; 12]) -> [f32; 12] {
	let hr = hero_rows(p);
	let b = place(held.pos, held.rot);
	let mut rel = [0.0f32; 16];
	for k in 0..3 {
		rel[k * 4..k * 4 + 3].copy_from_slice(&hero_to_model(&hr, [b[k * 4], b[k * 4 + 1], b[k * 4 + 2]]));
	}
	rel[12..15].copy_from_slice(&hero_to_model(&hr, [held.pos[0] - p.pos[0], held.pos[1] - p.pos[1], held.pos[2] - p.pos[2]]));
	rel[15] = 1.0;
	let rel = fix.map_or(rel, |f| f.place(rel));
	std::array::from_fn(|n| if n % 4 == 3 { hand[n] } else { rel[(n % 4) * 4 + n / 4] })
}

#[cfg(test)]
mod tilt_tests {
	use super::*;

	// BRIDGE-75: tilt [0, 0] keeps the yaw-only rows; tilt [0, 0.3] at yaw 0 maps model (0, 0, 1) to (sin 0.3, 0, cos 0.3).
	#[test]
	fn hero_rows_tilt() {
		let mut p = crate::player::Player::new([0.0; 3], 0.7);
		let r = hero_rows(&p);
		let (s, c) = 0.7f32.sin_cos();
		assert_eq!(hero_to_world(&r, [1.0, 0.0, 0.0]), [c, s, 0.0]);
		p.yaw = 0.0;
		p.body_tilt = [0.0, 0.3];
		let r = hero_rows(&p);
		let v = hero_to_world(&r, [0.0, 0.0, 1.0]);
		assert!((v[0] - 0.3f32.sin()).abs() < 1e-6 && v[1].abs() < 1e-6 && (v[2] - 0.3f32.cos()).abs() < 1e-6, "{v:?}");
		let m = hero_to_model(&r, v);
		assert!(m[0].abs() < 1e-6 && m[1].abs() < 1e-6 && (m[2] - 1.0).abs() < 1e-6, "{m:?}");
	}
}
