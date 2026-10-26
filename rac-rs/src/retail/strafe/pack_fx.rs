//! [PC] The Thruster-Pack exhaust while strafing (part of strafe.rs `turn_parts`): retail spawns the pack's flame and smoke
//! particles from its exhaust mobys, placed at the nozzles in the logic hero frame (rows +0xc0, the way Ratchet runs), while
//! `turn_parts` draws the pack turned to the drawn yaw; unmoved, the flames burned beside the drawn
//! pack (user report 2026-10-24, Pokitaru). A particle an exhaust moby spawned this field is moved once by that field's turn about
//! the hero and keeps the offset for its life, so the trail stays where it was left in the world; the flame cones the exhaust
//! mobys draw are moved with the pack every field (`exhaust_shift`). Draw only; RAM is not written.

use std::sync::Mutex;

/// The Thruster-Pack (back object `*0x1404d0`, class at +0xa6; 607 is the Heli-Pack).
const THRUSTER_PACK: u16 = 608;

/// The pack's exhaust mobys (class 167, update Orxon FUN_002a5380, hidden). In state 3 (firing) its spawner FUN_002a4e38 puts,
/// every field, type-0x15 flames at its origin F (+0x10) and halfway back to its last smoke point, and type-0x17 smoke at
/// S = F + its rows (+0xc0) x (0, 0, -0.3) and up to 3 points back toward the last S; S is kept in pvar +0x10, the pack in
/// pvar +8 [C Orxon OV:128746-128932, 128959-129092].
const EXHAUST: u16 = 167;
const FIRING: u8 = 3;
const FLAME: u8 = 0x15;
const SMOKE: u8 = 0x17;

/// How far from those spawn segments a particle new this field may be (one step of its velocity, the puff's jitter).
const SPAWN_REACH: f32 = 0.2;

/// A flame with its flag (+0x3a, 1 from the spawner) spawns a child flame at its own position on odd ticks (its update Orxon
/// FUN_0025f3a0 OV:84668-84763 [C]); the child, one step of its velocity away, takes the offset of the nearest particle this
/// near.
const CHILD_REACH: f32 = 0.15;

/// A slot whose particle moved further than this in one field holds a new particle.
const SAME_REACH: f32 = 0.5;

/// Effect pool slots (`RetailFx::sprite_slots`, 0..0x800).
const SLOTS: usize = 0x800;

struct State {
	/// Per pool slot: the particle's raw position last field and its offset.
	seen: Vec<Option<([f32; 3], [f32; 3])>>,
	/// Each firing exhaust moby's smoke point S of the last field.
	smoke: Vec<(u32, [f32; 3])>,
}

static ST: Mutex<State> = Mutex::new(State { seen: Vec::new(), smoke: Vec::new() });

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
	dot(sub(a, b), sub(a, b)).sqrt()
}

/// Distance from `p` to the segment `a`..`b`.
fn seg_dist(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
	let ab = sub(b, a);
	let l = dot(ab, ab);
	let t = if l > 0.0 { (dot(sub(p, a), ab) / l).clamp(0.0, 1.0) } else { 0.0 };
	dist(p, [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t])
}

/// The spawn segments of this field's firing exhaust mobys of `pack` (last S to F, last S to S), with `smoke` updated.
fn spawn_segments(m: &ee_oracle::Machine, pack: u32, smoke: &mut Vec<(u32, [f32; 3])>) -> Vec<([f32; 3], [f32; 3])> {
	let (s, e) = crate::retail::ram_globals::table(m.rd8(crate::retail::ram_globals::LEVEL) as u32);
	let (mut a, e) = (m.rd32(s) & 0x01ff_ffff, m.rd32(e) & 0x01ff_ffff);
	let (mut segs, mut now) = (Vec::new(), Vec::new());
	while a < e && a != 0 {
		let pv = m.rd32(a + 0x78) & 0x01ff_ffff;
		if m.rd16(a + 0xa6) == EXHAUST && m.rd8(a + 0x20) == FIRING && pv != 0 && m.rd32(pv + 8) & 0x01ff_ffff == pack {
			let (f, sm) = (m.rdv3(a + 0x10), m.rdv3(pv + 0x10));
			let last = smoke.iter().find(|x| x.0 == a).map_or(sm, |x| x.1);
			segs.extend([(last, f), (last, sm)]);
			now.push((a, sm));
		}
		a += 0x100;
	}
	*smoke = now;
	segs
}

/// [PC] The move of an exhaust moby's flame (its draw-queue callback's world quads, efx_draw `BuildIn::shift`): the turn
/// `turn_parts` gives the pack, applied to the moby's origin, as a move (the flame faces the screen: turned, it would go edge-on);
/// None for any other owner, without the Thruster-Pack, or while not aiming.
pub fn exhaust_shift(m: &ee_oracle::Machine, moby: u32) -> Option<[f32; 3]> {
	let (a, pack) = (moby & 0x01ff_ffff, m.rd32(0x1404d0) & 0x01ff_ffff);
	if a == 0 || pack == 0 || m.rd16(a + 0xa6) != EXHAUST || m.rd16(pack + 0xa6) != THRUSTER_PACK {
		return None;
	}
	let pv = m.rd32(a + 0x78) & 0x01ff_ffff;
	if pv == 0 || m.rd32(pv + 8) & 0x01ff_ffff != pack {
		return None;
	}
	let (delta, hp) = super::part_turn(m)?;
	let (f, (sn, cs)) = (m.rdv3(a + 0x10), delta.sin_cos());
	let (x, y) = (f[0] - hp[0], f[1] - hp[1]);
	Some([cs * x - sn * y - x, sn * x + cs * y - y, 0.0])
}

/// Offsets the pool sprites of field `fx`: a flame or smoke particle an exhaust moby spawned this field (or a child flame of one) by
/// the turn `delta` (drawn - rows yaw) about the hero `hp`, one seen before by its offset; nothing without the Thruster-Pack, nor a
/// sprite of a part `turn_parts` turned (`turned`). `delta` 0 (not aiming) moves no new particle, the earlier ones keep theirs
/// until they die.
pub(super) fn carry(m: &ee_oracle::Machine, hp: [f32; 3], delta: f32, fx: &mut crate::retail::RetailFx, turned: &[u32]) {
	let pack = m.rd32(0x1404d0) & 0x01ff_ffff;
	let Ok(mut st) = ST.lock() else { return };
	if pack == 0 || m.rd16(pack + 0xa6) != THRUSTER_PACK {
		st.seen.clear();
		st.smoke.clear();
		return;
	}
	let segs = spawn_segments(m, pack, &mut st.smoke);
	let (sn, cs) = delta.sin_cos();
	let kind = |n: usize| fx.sprite_types.get(n).copied().unwrap_or(0);
	// Seen before, spawned by an exhaust moby, or None: new elsewhere (a child flame, or not the pack's).
	let offs: Vec<Option<[f32; 3]>> = fx
		.sprites
		.iter()
		.zip(&fx.sprite_slots)
		.enumerate()
		.map(|(n, (s, &slot))| {
			let p = s.pos;
			let old = st.seen.get(slot as usize % SLOTS).copied().flatten().filter(|e| dist(e.0, p) < SAME_REACH).map(|e| e.1);
			old.or_else(|| {
				let exhaust = matches!(kind(n), FLAME | SMOKE) && segs.iter().any(|&(a, b)| seg_dist(p, a, b) < SPAWN_REACH);
				let mine = !fx.sprite_owners.get(n).is_some_and(|o| turned.contains(o));
				(exhaust && mine).then(|| {
					let (x, y) = (p[0] - hp[0], p[1] - hp[1]);
					if delta.abs() < 1e-4 { [0.0; 3] } else { [cs * x - sn * y - x, sn * x + cs * y - y, 0.0] }
				})
			})
		})
		.collect();
	let parent = |p: [f32; 3]| {
		let near = fx.sprites.iter().zip(&offs).filter_map(|(q, o)| o.map(|o| (dist(q.pos, p), o))).filter(|x| x.0 < CHILD_REACH);
		near.min_by(|a, b| a.0.total_cmp(&b.0)).map_or([0.0; 3], |x| x.1)
	};
	let offs: Vec<[f32; 3]> = offs.iter().enumerate().map(|(n, o)| o.unwrap_or_else(|| if kind(n) == FLAME { parent(fx.sprites[n].pos) } else { [0.0; 3] })).collect();
	let mut next = vec![None; SLOTS];
	for ((s, &slot), off) in fx.sprites.iter_mut().zip(&fx.sprite_slots).zip(offs) {
		let p = s.pos;
		next[slot as usize % SLOTS] = Some((p, off));
		s.pos = [p[0] + off[0], p[1] + off[1], p[2]];
	}
	st.seen = next;
}
