//! [PC] Lane rfire worker 8 (user 2026-11-07: a carried flame source should light its carrier and the ground a little): the light-only
//! flames the game pack reports (`PoolFx::flame_lights`: flames it draws too small to light by their sprites), each ONE small surface
//! light (flitlist.rs `Cluster`, `small` + `carried`: ranked after every big fire, in no fire, never a shadow map, no sway, no dance,
//! no smoke row; worker 9, review H1/M3). They do not go through the fires' tracker (flitlist.rs `Tracker` eases a light's place over
//! 0.2 s: a light on a gliding carrier would trail a metre behind it): the place is the pack's (blended per display frame by the frame
//! smoothing), only the strength fades in over `FADE_IN` s and out over `FADE_OUT` s by id (no pop when the flames start or stop).
//! Only while the Fire row draws (firefx.rs `begin_sprites`): Fire Off, Lighting Classic, GI Off keep test build 8's lights exactly.
//! Knob RAC_TUNE fire_carried (area scale, 0 = none).

use super::flitlist::Cluster;
use crate::fxpool::FlameLight;

/// Seconds the light takes to fade in / out (game time).
const FADE_IN: f32 = 0.08;
const FADE_OUT: f32 = 0.12;
/// The light's body radius (m, RAC_TUNE fire_carried_rs): the flit pass lights a surface by I / max(d, rs)^2, and a carried light
/// sits a hand's width from its carrier: at the flames' own 0.3 m (flitlist.rs `RS_LO`) the carrier went bright orange, at 0.5 m
/// it still read cream (lane pictures pp\w8_rs.jpg, 2026-11-07), so the near light is held to I / BODY^2; the ground a metre and
/// more away (d > BODY) is lit as by any small flame.
const BODY: f32 = 0.8;
/// Most light-only flames kept (fading ones too).
const MAX: usize = 4;

#[derive(Clone, Copy, Debug)]
struct Track {
	l: FlameLight,
	fade: f32,
	seen: bool,
}

/// The light-only flames followed by id (firefx.rs `FireFx::carried`).
#[derive(Default)]
pub(super) struct Carried {
	tracks: Vec<Track>,
	/// This frame's flames (`note`).
	now: Vec<FlameLight>,
	last: Option<f64>,
}

impl Carried {
	/// This frame's flames (`flames`, empty while the Fire row does not draw: then everything is dropped at once).
	pub(super) fn note(&mut self, flames: &[FlameLight], on: bool) {
		self.now.clear();
		if !on {
			self.tracks.clear();
			self.last = None;
			return;
		}
		self.now.extend(flames.iter().copied().take(MAX));
	}

	/// Drops everything (the fire lights are off).
	pub(super) fn forget(&mut self) {
		self.tracks.clear();
		self.now.clear();
		self.last = None;
	}

	/// One frame at game time `t`: the noted flames take their track (place and area as given) and fade in, the others fade out
	/// where they last stood; appends them to `out` as small carried clusters (power = area x fade x fire_carried). The game standing
	/// changes nothing.
	pub(super) fn step(&mut self, t: f64, out: &mut Vec<Cluster>) {
		let dt = self.last.map_or(0.0, |l| (t - l).clamp(0.0, 0.1) as f32);
		self.last = Some(t);
		self.tracks.iter_mut().for_each(|k| k.seen = false);
		for l in &self.now {
			match self.tracks.iter().position(|k| k.l.id == l.id) {
				Some(i) => {
					let k = &mut self.tracks[i];
					k.l = *l;
					k.fade = (k.fade + dt / FADE_IN).min(1.0);
					k.seen = true;
				}
				None if self.tracks.len() < MAX => self.tracks.push(Track { l: *l, fade: (dt / FADE_IN).min(1.0), seen: true }),
				None => {}
			}
		}
		for k in self.tracks.iter_mut().filter(|k| !k.seen) {
			k.fade = (k.fade - dt / FADE_OUT).max(0.0);
		}
		self.tracks.retain(|k| k.seen || k.fade > 0.0);
		let gain = crate::devtune::f32("fire_carried", 1.0).max(0.0);
		let rs = crate::devtune::f32("fire_carried_rs", BODY).clamp(0.1, 2.0);
		if super::firefx::tune("fire_pick_log", 0.0) > 0.5 && !self.tracks.is_empty() {
			eprintln!("fire_carried: t {t:.3} {:?}", self.tracks.iter().map(|k| (k.l.id, k.l.pos, k.fade)).collect::<Vec<_>>());
		}
		out.extend(self.tracks.iter().filter(|k| k.fade > 0.0 && gain > 0.0).map(|k| Cluster { pos: k.l.pos, power: k.l.area * k.fade * gain, rs, small: true, carried: true }));
	}
}

#[cfg(test)]
mod tests {
	use super::super::flitlist::{self, Light};
	use super::*;

	fn fl(id: u32, x: f32) -> FlameLight {
		FlameLight { id, pos: [x, 0.0, 1.0], area: 0.08 }
	}

	fn step(c: &mut Carried, t: f64) -> Vec<Cluster> {
		let mut out = Vec::new();
		c.step(t, &mut out);
		out
	}

	/// The frame's light list as passes.rs makes it: `fire_lights` (sway up to 3 m, GI High), `groups`, `dance`.
	fn frame(cs: &[Cluster], t: f64) -> (Vec<Light>, Vec<flitlist::Group>) {
		let mut list = flitlist::fire_lights(cs, t, 8, 3.0, [0.0; 3], true);
		let g = flitlist::groups(&mut list);
		flitlist::dance(&mut list, &g, t, true);
		(list, g)
	}

	/// A flame fades in, follows its place exactly (no lag), fades out where it stood; nothing while the Fire row is off. Review M3:
	/// through `fire_lights` / `groups` / `dance` the FINAL light stands exactly at the flame (no sway, no dance).
	#[test]
	fn carried_flames_fade_in_follow_and_fade_out() {
		let mut c = Carried::default();
		c.note(&[fl(7, 0.0)], true);
		let first = step(&mut c, 1.0);
		assert!(first.is_empty() || first[0].power == 0.0, "starts dark: {first:?}");
		let mut t = 1.0;
		let mut last = 0.0;
		for k in 1..=10 {
			t += 1.0 / 60.0;
			c.note(&[fl(7, k as f32)], true);
			let cs = step(&mut c, t);
			assert_eq!(cs.len(), 1);
			assert_eq!(cs[0].pos, [k as f32, 0.0, 1.0], "the place is the carrier's own");
			assert!(cs[0].small && cs[0].carried && cs[0].power >= last && cs[0].power <= 0.08 + 1e-6);
			last = cs[0].power;
			let (list, g) = frame(&cs, t);
			assert!(g.is_empty() && list.len() == 1 && list[0].group.is_none(), "in no fire: {list:?} {g:?}");
			assert_eq!(list[0].pos, [k as f32, 0.0, 1.0], "no sway, no dance");
		}
		assert!((last - 0.08).abs() < 1e-6, "full after FADE_IN");
		// Stopped: fades out over a few frames at the last place, then gone.
		let mut n = 0;
		loop {
			t += 1.0 / 60.0;
			c.note(&[], true);
			let cs = step(&mut c, t);
			if cs.is_empty() {
				break;
			}
			assert!(cs[0].power < last && cs[0].pos == [10.0, 0.0, 1.0]);
			assert_eq!(frame(&cs, t).0[0].pos, [10.0, 0.0, 1.0]);
			last = cs[0].power;
			n += 1;
		}
		assert!((3..=10).contains(&n), "fades out over a few frames: {n}");
		// Fire row off: dropped at once; forget (respawn, savestate, warp): dropped at once.
		c.note(&[fl(7, 0.0)], true);
		step(&mut c, t + 0.5);
		c.note(&[fl(7, 0.0)], false);
		assert!(step(&mut c, t + 0.6).is_empty());
		c.note(&[fl(7, 0.0)], true);
		step(&mut c, t + 0.7);
		c.forget();
		assert!(step(&mut c, t + 0.8).is_empty());
	}

	/// The carried light is small: with the light cap full, the big fires keep every slot; under the cap it comes after them.
	#[test]
	fn the_big_fires_keep_the_slots_first() {
		let mut c = Carried::default();
		c.note(&[fl(1, 0.0)], true);
		step(&mut c, 0.0);
		c.note(&[fl(1, 0.0)], true);
		let mut cs = vec![Cluster { pos: [30.0, 0.0, 1.0], power: 4.0, rs: 1.0, small: false, carried: false }];
		c.step(0.5, &mut cs);
		assert_eq!(cs.len(), 2);
		let one = flitlist::fire_lights(&cs, 0.5, 1, 0.1, [0.0; 3], true);
		assert!(one.len() == 1 && !one[0].small, "the big fire keeps the only slot");
		let two = flitlist::fire_lights(&cs, 0.5, 8, 0.1, [0.0; 3], true);
		assert!(two.len() == 2 && !two[0].small && two[1].small, "the carried light comes after it");
		// Nothing reported this frame: no light at all.
		let mut none = Carried::default();
		none.note(&[], true);
		assert!(step(&mut none, 1.0).is_empty());
	}

	/// Review H1: a carried light 2 m from a big fire changes nothing of that fire (centre, near plane, size, power, its smoke row,
	/// its shadow origins with tongues right at the carried light) and is in no fire itself (no map read, no dance).
	#[test]
	fn a_carried_light_beside_a_big_fire_leaves_the_fire_alone() {
		let fire = Cluster { pos: [10.0, 4.0, 1.0], power: 6.0, rs: 1.5, small: false, carried: false };
		let pack = Cluster { pos: [12.0, 4.0, 1.5], power: 0.05, rs: BODY, small: true, carried: true };
		let t = 2.0;
		let (alone, ga) = frame(&[fire], t);
		let (with, gw) = frame(&[fire, pack], t);
		assert_eq!(ga, gw, "the fire's group is the no-carried run's");
		assert_eq!(with.len(), 2);
		assert_eq!(with[0], alone[0], "the fire's light is the no-carried run's");
		assert!(with[1].carried && with[1].group.is_none() && with[1].pos == pack.pos, "{:?}", with[1]);
		let fx = super::super::partfx::FxLights::default();
		let (sa, sw) = (flitlist::smoke(&ga, &fx, 4), flitlist::smoke(&gw, &fx, 4));
		assert_eq!((sa.n, sa.l), (sw.n, sw.l), "no smoke row of its own");
		// Shadow origins (flitsplit.rs, 4 maps): tongues of the fire and two right at the carried light.
		let tongues = vec![([9.0, 4.0, 1.5], 1.0, 0.5), ([11.0, 4.0, 1.5], 1.0, 0.5), ([12.0, 4.0, 1.6], 0.2, 0.2), ([12.2, 4.0, 1.6], 0.2, 0.2)];
		let origins = |mut list: Vec<Light>, g: &[flitlist::Group]| {
			let mut sp = super::super::flitsplit::Split::default();
			sp.tongues = tongues.clone();
			let o = sp.step(&mut list, g, [0.0; 3], 4, t).to_vec();
			(o, list)
		};
		let ((oa, la), (ow, lw)) = (origins(alone, &ga), origins(with, &gw));
		assert_eq!(oa, ow, "the fire's shadow origins are the no-carried run's");
		assert_eq!(la[0], lw[0]);
		assert!(lw[1].group.is_none() && lw[1].alt.is_none(), "the carried light reads no map: {:?}", lw[1]);
	}
}
