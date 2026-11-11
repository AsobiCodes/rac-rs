//! [PC] lane dlit2 (2026-11-06; user: "i want the fire to be a true source of light bro"): the surface light list. Every fire of the
//! frame (the level's tongues and the burning things, firefx.rs `lit` / `big`) is clustered per Global illumination level (`budget`:
//! Low 8 lights from flames within 6 m, Medium 16 within 4 m, High 32 within 2.5 m: a field of flames becomes several lights where
//! the flames are, not one at its centre), tracked from frame to frame (`Tracker`: a light follows its flames, its place / power /
//! body ease in over `TAU` s; a new light fades in and a lost one out over `FADE` s: no popping) and made a sphere light (`Light`:
//! fire light v2, lane dlit3: a sphere emitter round the flame volume (centre = the tongues' mid-points, radius = their spread and
//! size), intensity = the flames' area (h x w summed) x flit_gain x the slow flicker, blackbody colour (flitcolour.rs), reach where
//! its light falls to `EDGE` of full sun). The weapon lights (`FxLights` rows with the shot bit) join them
//! (`with_shots`). Lights within `GROUP` m are one fire (`groups`): the light shadow maps (sunshadow/omni.rs) go to whole fires and
//! every light of a mapped fire reads its map. `grid`: per `CELL` m world cell round the eye the bit mask of the lights whose reach
//! touches it (flit.wgsl loops over its cell's lights only). flit.rs packs it all into `Uniforms::flit`.
//! `smoke`: the fires as the volumetric smoke's lights (one per fire, test build 4's brightness and reach, volsmoke.rs).
//! Knobs (RAC_TUNE): flit_gain (intensity per m^2 of flame), flit_lift (the light's height in the flames, share of their height),
//! flit_rsk (body radius scale), flit_temp (flame temperature, K), flit_shot (weapon lights), flit_log=1 (the list every 30th frame).

use super::firefx::Emit;
use super::partfx::FxLights;
use crate::devtune;

/// Most lights in the list (flit.rs rows 0..63; one bit each in the grid's u32 masks).
pub(super) const MAX: usize = 32;
/// The grid: `CELLS` x `CELLS` cells of `CELL` m round the eye (flit.wgsl `flit_cell`; the border cells take what lies beyond).
pub(super) const CELLS: usize = 16;
pub(super) const CELL: f32 = 8.0;
/// Fire lights within this distance (m) of a fire's centre are that fire (one shadow map, sunshadow/omni.rs).
pub(super) const GROUP: f32 = 6.0;
/// A fire's shadow-map near plane = this share of its emitter's horizontal half extent, held to [NEAR_LO, NEAR_HI] m (`Group::near`;
/// NEAR_LO = sunshadow/omni.rs NEAR).
const NEAR_SHARE: f32 = 0.8;
const NEAR_LO: f32 = 0.25;
const NEAR_HI: f32 = 4.0;
/// Seconds a light takes to follow its flames (place, power, body; exponential).
const TAU: f32 = 0.2;
/// Seconds a light takes to fade in / out.
const FADE: f32 = 0.3;
/// Default flit_lift: a tongue's light comes from this share of its height over its foot. Fire light v2 (lane dlit3): 0.5 = the
/// tongue's mid-point, so a cluster's centre is the centre of its flame volume (sphere emitter, FIRE_LIGHT_V2.md 2.1; was 0.7, which
/// only lifted the shadow origin out of the hull). 0.75 since test build 7: the light comes from the brighter, taller part of the
/// flames, so ground beside a fire in a dip is not on the light's horizon (user 2026-11-07, the dark crash-site dip).
pub(super) const LIFT: f32 = 0.75;
/// The body radius (m): sqrt(BALL x spread^2 + mean half size^2) of the flames round the centre (the sphere that holds the flame
/// volume: a tongue's half size = half its height or width; the spread is the tongues' RMS distance from the centre, and a uniform
/// ball's RMS radius is sqrt(3 / 5) of its radius, hence BALL = 5 / 3), held to this range. Stage 1 follow-up (lane dlit3): was
/// sqrt(spread^2 + (max(w, h / 2) / 2)^2), half the flame's real size, which made a big wreck fire a ~1 m ball of 20x the flame
/// luminance and put a hard horizon on every terrain crease.
const BALL: f32 = 5.0 / 3.0;
pub(super) const RS_LO: f32 = 0.3;
const RS_HI: f32 = 4.0;
/// The reach (m): where the light falls to `EDGE` (share of full sun), held to this range.
const REACH_LO: f32 = 3.0;
pub(super) const REACH_HI: f32 = 40.0;
const EDGE: f32 = 0.02;
/// Default flit_gain: luminous intensity (full sun at 1 m) per m^2 of flame (h x w). Fire light v2 (lane dlit3, FIRE_LIGHT_V2.md
/// 2.2): a flame of luminance L ~ 2.5 x a sunlit white surface, silhouette ~ 0.6 x the tongues' summed area: I = L x 0.6 A / pi
/// ~ 0.5 A, so a big wreck fire (~80 m^2) gives ~1x full sun 6 m off, 0.3 at 12 m (was 3.5 with the wrap and roll-off).
/// 1.5 since test build 7 (user 2026-11-07, test build 6 at the Veldin crash site: "why do we have shadows near a literal fucking
/// inferno which should cast light"): the dip round the wreck fires stayed sun-shadow dark red 4-10 m out; with flit.rs ROLL 2.0
/// the flames' foot keeps its gradient (picture runs klight ng_0005 f60 camera 14 m, fg1 0_0001 f120, Novalis, Pyrocitor).
const GAIN: f32 = 1.5;
/// Default flit_rsk: the body radius x this (1 = the flame volume itself; was 1.5, an inflated body that lit every side).
const RSK: f32 = 1.0;
/// Default flit_temp: the flames' temperature (K) at full strength; the colour is that blackbody's (flitcolour.rs), cooler as the
/// flame dims. 1900 K (test build 7's deep warm orange; lane rfire worker 2 tried 3000 K: it hid the uneven pool by nearly hiding
/// the light, rejected). Bright warm light reads cream / white through flit.wgsl `flit_shoulder_warm`, not by a paler colour.
const TEMP: f32 = 1900.0;
/// The weapon lights keep test build 4's strength: intensity = screen gain x `SHOT_GAIN` x r0^2, r0 = `SHOT_SIZE` x reach in
/// [0.3, 0.6] m (flit.wgsl of lane dlit: gain 10, size share 0.04).
const SHOT_GAIN: f32 = 10.0;
const SHOT_SIZE: f32 = 0.04;
/// Tracks kept at most (the fading ones over the budget too).
const TRACKS: usize = 2 * MAX;

/// The fire lights and their cluster radius (m) at Global illumination level `gi` (0 Off, 1 Low, 2 Medium, 3 High).
pub(super) const fn budget(gi: u8) -> (usize, f32) {
	match gi {
		0 => (0, 0.0),
		1 => (8, 6.0),
		2 => (16, 4.0),
		_ => (MAX, 2.5),
	}
}

/// A surface light: centre, reach (m), colour x intensity (linear, 1 = full sun at 1 m from a point), body radius (m), its fire
/// (`groups`; None: a weapon light, no shadow map).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Light {
	pub pos: [f32; 3],
	pub reach: f32,
	pub rgb: [f32; 3],
	pub rs: f32,
	pub group: Option<u8>,
	pub fire: bool,
	/// A fire light's flames (m^2, h x w summed, x its fade) and its flicker now (the smoke's light, `smoke`); 0 for weapon lights.
	pub area: f32,
	pub flick: f32,
	/// Fire light v2 stage 6 (flitsplit.rs): the second-nearest shadow origin of its fire (`group` = the nearest) and the share of
	/// its map in this light's shadow (0..0.5, blended round the bisector); None: `group`'s map alone.
	pub alt: Option<(u8, f32)>,
	/// Lane rfire review: a light of small flames only (firefx.rs `small`): ranked after the big fires for the light slots, and a fire
	/// of small lights only never takes a shadow map (sunshadow/omni.rs `candidates`).
	pub small: bool,
	/// Lane rfire worker 9: a carried flame source's light (firefx_carried.rs: it moves with whatever carries it): no sway, in no
	/// fire (`groups`: group None, so no shadow map, no dance, no smoke row, no shadow origins of a fire it passes).
	pub carried: bool,
}

impl Light {
	fn peak(&self) -> f32 {
		self.rgb[0].max(self.rgb[1]).max(self.rgb[2])
	}
}

/// Flames that light as one: centre, power (h x w summed, x the fade once tracked), body radius.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Cluster {
	pub pos: [f32; 3],
	pub power: f32,
	pub rs: f32,
	/// Made of small flames only (`clusters_tagged`).
	pub small: bool,
	/// A carried flame source (firefx_carried.rs; `Light::carried`).
	pub carried: bool,
}

/// One fire (lights within `GROUP` m): power-weighted centre, the reach of its farthest-reaching light from there, power, radius
/// (the lights' power-weighted spread round the centre + their mean body radius: the soft shadows' light size, flitshadow.wgsl).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Group {
	pub pos: [f32; 3],
	pub reach: f32,
	/// Its lights' flames (m^2, steady: no flicker, `groups`): the weight of its centre and of its shadow map's rank (omni.rs).
	pub power: f32,
	pub size: f32,
	/// The emitter's half extent x `NEAR_SHARE` (m, horizontal: max over its lights of |centre - light| + body radius), held to
	/// [NEAR_LO, NEAR_HI]: its shadow map's near plane (sunshadow/omni.rs `face`), so the hull inside the flames never shadows its
	/// own fire (FIRE_LIGHT_V2.md 3.2).
	pub near: f32,
	/// Its lights' flames (m^2) and their area-weighted flicker (`smoke`).
	pub area: f32,
	pub flick: f32,
	/// DEV RAC_TUNE flsh_ref (flitref.rs `jitter`): this frame's emitter point, its shadow map's origin; None (always without the knob).
	pub jit: Option<[f32; 3]>,
	/// Fire light v2 stage 6 (flitsplit.rs): a shadow origin's fire (index in `groups`) and its share of that fire's power (a fire
	/// of `groups` itself: its own index and 1; sunshadow/omni.rs `step` hands a new map its fire's fade by it).
	pub fire: u8,
	pub share: f32,
	/// Lane rfire review: every light of the fire is `Light::small`: no shadow map for it (sunshadow/omni.rs `candidates`).
	pub small: bool,
}

fn d2(a: [f32; 3], b: [f32; 3]) -> f32 {
	(0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum()
}

/// The flames grouped greedily, biggest first: each joins the first cluster whose centre lies within `r` m, else starts one; sorted
/// by power, biggest first.
pub(super) fn clusters<'a>(fires: impl Iterator<Item = &'a Emit>, r: f32) -> Vec<Cluster> {
	clusters_tagged(fires.map(|e| (e, false)), r)
}

/// `clusters` of flames tagged small (firefx.rs `small`, true) or not: a cluster with no big flame is `Cluster::small`.
pub(super) fn clusters_tagged<'a>(fires: impl Iterator<Item = (&'a Emit, bool)>, r: f32) -> Vec<Cluster> {
	let mut es: Vec<(Emit, bool)> = fires.map(|(e, s)| (*e, s)).filter(|(e, _)| e.h > 0.0 && e.w > 0.0).collect();
	es.sort_by(|a, b| (b.0.h * b.0.w).total_cmp(&(a.0.h * a.0.w)));
	let lift = devtune::f32("flit_lift", LIFT).clamp(0.0, 1.0);
	// Per cluster: sum of p x wgt, of |p|^2 x wgt, of the half size x wgt, of wgt; whether a big flame is in it.
	let mut acc: Vec<([f32; 3], f32, f32, f32)> = Vec::new();
	let mut big: Vec<bool> = Vec::new();
	for (e, small) in &es {
		let p = [e.base[0], e.base[1], e.base[2] + lift * e.h];
		let wgt = e.h * e.w;
		let half = 0.5 * e.w.max(e.h);
		let at = |a: &([f32; 3], f32, f32, f32)| d2([a.0[0] / a.3, a.0[1] / a.3, a.0[2] / a.3], p) < r * r;
		let i = match acc.iter().position(at) {
			Some(i) => i,
			None => {
				acc.push(([0.0; 3], 0.0, 0.0, 0.0));
				big.push(false);
				acc.len() - 1
			}
		};
		big[i] |= !small;
		let a = &mut acc[i];
		(0..3).for_each(|k| a.0[k] += p[k] * wgt);
		a.1 += (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]) * wgt;
		a.2 += half * wgt;
		a.3 += wgt;
	}
	let mut out: Vec<Cluster> = acc
		.iter()
		.zip(&big)
		.map(|(a, &b)| {
			let c = [a.0[0] / a.3, a.0[1] / a.3, a.0[2] / a.3];
			let spread = (a.1 / a.3 - (c[0] * c[0] + c[1] * c[1] + c[2] * c[2])).max(0.0).sqrt();
			let half = a.2 / a.3;
			Cluster { pos: c, power: a.3, rs: (BALL * spread * spread + half * half).sqrt().clamp(RS_LO, RS_HI), small: !b, carried: false }
		})
		.collect();
	out.sort_by(|a, b| b.power.total_cmp(&a.power));
	out
}

#[derive(Clone, Copy, Debug)]
struct Track {
	c: Cluster,
	fade: f32,
	seen: bool,
}

/// The fire lights followed from frame to frame (firefx.rs `FireFx::surf`).
#[derive(Default)]
pub(super) struct Tracker {
	tracks: Vec<Track>,
	last: Option<f64>,
}

impl Tracker {
	/// One frame at game time `t` (s): each cluster (biggest first) moves the nearest unclaimed track within max(1.5 m, `r`) towards
	/// it (`TAU`) and fades it in, or starts a track (fading in; at full on the first frame); the tracks no cluster claims fade out
	/// where they stand. Returns the tracks with their power x fade. The game standing (same `t`) changes nothing. With all `TRACKS`
	/// in use a new cluster takes the weakest unclaimed track (power x fade) if it is stronger (lane rfire review: fading small flames
	/// must not keep a new big fire unlit for `FADE` s).
	pub(super) fn step(&mut self, cs: &[Cluster], r: f32, t: f64) -> Vec<Cluster> {
		let first = self.last.is_none();
		let dt = self.last.map_or(0.0, |l| (t - l).clamp(0.0, 0.1) as f32);
		self.last = Some(t);
		let ease = if first { 1.0 } else { 1.0 - (-dt / TAU).exp() };
		let k = dt / FADE;
		let m2 = r.max(1.5) * r.max(1.5);
		self.tracks.iter_mut().for_each(|t| t.seen = false);
		for c in cs {
			let near = self.tracks.iter().enumerate().filter(|(_, t)| !t.seen).map(|(i, t)| (i, d2(t.c.pos, c.pos))).filter(|x| x.1 < m2).min_by(|a, b| a.1.total_cmp(&b.1));
			match near {
				Some((i, _)) => {
					let t = &mut self.tracks[i];
					(0..3).for_each(|j| t.c.pos[j] += (c.pos[j] - t.c.pos[j]) * ease);
					t.c.power += (c.power - t.c.power) * ease;
					t.c.rs += (c.rs - t.c.rs) * ease;
					t.c.small = c.small;
					t.fade = (t.fade + k).min(1.0);
					t.seen = true;
				}
				None if self.tracks.len() < TRACKS => self.tracks.push(Track { c: *c, fade: if first { 1.0 } else { k.min(1.0) }, seen: true }),
				None => {
					let weak = self.tracks.iter().enumerate().filter(|(_, t)| !t.seen).map(|(i, t)| (i, t.c.power * t.fade)).min_by(|a, b| a.1.total_cmp(&b.1));
					if let Some((i, _)) = weak.filter(|w| w.1 < c.power) {
						self.tracks[i] = Track { c: *c, fade: k.min(1.0), seen: true };
					}
				}
			}
		}
		for t in self.tracks.iter_mut().filter(|t| !t.seen) {
			t.fade = (t.fade - k).max(0.0);
		}
		self.tracks.retain(|t| t.seen || t.fade > 0.0);
		self.tracks.iter().map(|t| Cluster { power: t.c.power * t.fade, ..t.c }).collect()
	}
}

/// The fire lights of the tracked clusters `cs` at game time `t`: intensity flit_gain x power x the slow flicker (firefx_light.rs,
/// fire_flicker), the colour a blackbody at flit_temp x flicker^(1/4) (flitcolour.rs), the centre swaying at most `sway_max` m;
/// the big fires first (lane rfire review: small flames never take a slot from a big fire), each part ranked by intensity, those
/// far from the eye (past their reach + 20 m) weighed down; at most `n`. `live`: the Fire row draws (lane rfire's licks); false
/// (Fire Off, the game's own flame billboards) = test build 7's lights. The flames' dance comes later, per fire (`dance`).
pub(super) fn fire_lights(cs: &[Cluster], t: f64, n: usize, sway_max: f32, eye: [f32; 3], live: bool) -> Vec<Light> {
	let (gain, depth, sway) = (devtune::f32("flit_gain", GAIN), devtune::f32("fire_flicker", 0.35), devtune::f32("fire_sway", 1.0));
	let rsk = devtune::f32("flit_rsk", RSK).clamp(0.1, 4.0);
	let t0 = devtune::f32("flit_temp", TEMP).clamp(1000.0, 4000.0);
	let mut out: Vec<(f32, Light)> = cs
		.iter()
		.filter(|c| c.power > 0.0)
		.map(|c| {
			let ph = (0.11 * c.pos[0] + 0.17 * c.pos[1] + 0.07 * c.pos[2]) as f64;
			let i0 = gain.max(0.0) * c.power;
			let flick = super::firefx_light::flicker(t, ph, depth, live).max(0.2);
			let i = i0 * flick;
			let reach = (i0 / EDGE).sqrt().clamp(REACH_LO, REACH_HI);
			// Lane rfire worker 9: a carried light stands where its carrier is (no sway).
			let amp = if c.carried { 0.0 } else { (super::firefx_light::SWAY * reach).min(sway_max) * sway };
			let off = super::firefx_light::sway_off(t, ph);
			let rs = (c.rs * rsk).clamp(RS_LO, 2.0 * RS_HI);
			let pos = [c.pos[0] + amp * off[0], c.pos[1] + amp * off[1], c.pos[2] + amp * off[2]];
			let far = (d2(pos, eye).sqrt() - reach).max(0.0) / 20.0;
			let col = super::flitcolour::rgb(super::flitcolour::temperature(t0, flick));
			(i / (1.0 + far * far), Light { pos, reach, rgb: col.map(|c| c * i), rs, group: None, fire: true, area: c.power, flick, alt: None, small: c.small, carried: c.carried })
		})
		.collect();
	out.sort_by(|a, b| a.1.small.cmp(&b.1.small).then(b.0.total_cmp(&a.0)));
	out.into_iter().take(n.min(MAX)).map(|x| x.1).collect()
}

/// The weapon lights of `fx` (rows with the shot bit, colour .w > 0) first, then `fires`; at most `MAX`.
pub(super) fn with_shots(fires: Vec<Light>, fx: &FxLights) -> Vec<Light> {
	let k = devtune::f32("flit_shot", 1.0).max(0.0);
	let mut out: Vec<Light> = (0..fx.n.min(fx.l.len()))
		.filter(|&i| fx.shot & (1u8 << i) != 0 && fx.l[i][0][3] > 0.0 && fx.l[i][1][3] > 0.0)
		.map(|i| {
			let [p, c] = fx.l[i];
			let r0 = (SHOT_SIZE * p[3]).clamp(0.3, 0.6);
			let s = c[3] * SHOT_GAIN * r0 * r0 * k;
			Light { pos: [p[0], p[1], p[2]], reach: p[3], rgb: [c[0] * s, c[1] * s, c[2] * s], rs: r0, ..Default::default() }
		})
		.collect();
	out.extend(fires);
	out.truncate(MAX);
	out
}

/// The fires of `list`: each fire light (biggest first) joins the first fire whose centre lies within `GROUP` m (power-weighted,
/// power = its flames' area: steady, tb5 review S1: the flickering peak reordered and reweighted the lights every frame, so a fire
/// wider than `GROUP` split differently and its centre jumped metres, restarting its shadow map), else starts one; sets the lights'
/// `group`. A fire's reach is that of its farthest-reaching light from the fire's centre, at most `REACH_HI`.
pub(super) fn groups(list: &mut [Light]) -> Vec<Group> {
	let mut acc: Vec<([f32; 3], f32)> = Vec::new();
	// Lane rfire worker 9: a carried light joins no fire (review H1: within GROUP m of a big fire it moved that fire's centre, near
	// plane and shadow origins with the carrier, and read the fire's map from the fire's place).
	let mut order: Vec<usize> = (0..list.len()).filter(|&i| list[i].fire && !list[i].carried).collect();
	order.sort_by(|&a, &b| list[b].area.total_cmp(&list[a].area));
	for &i in &order {
		let (p, w) = (list[i].pos, list[i].area.max(1e-6));
		let g = match acc.iter().position(|a| d2([a.0[0] / a.1, a.0[1] / a.1, a.0[2] / a.1], p) < GROUP * GROUP) {
			Some(g) => g,
			None => {
				acc.push(([0.0; 3], 0.0));
				acc.len() - 1
			}
		};
		(0..3).for_each(|k| acc[g].0[k] += p[k] * w);
		acc[g].1 += w;
		list[i].group = Some(g as u8);
	}
	let mut out: Vec<Group> = acc.iter().enumerate().map(|(i, a)| Group { pos: [a.0[0] / a.1, a.0[1] / a.1, a.0[2] / a.1], power: a.1, fire: i as u8, share: 1.0, ..Default::default() }).collect();
	for l in list.iter() {
		if let Some(g) = l.group {
			let gr = &mut out[g as usize];
			gr.reach = gr.reach.max(d2(gr.pos, l.pos).sqrt() + l.reach).min(REACH_HI);
			let w = l.area.max(1e-6) / gr.power;
			gr.size += w * (d2(gr.pos, l.pos) + l.rs * l.rs);
			let dh = ((gr.pos[0] - l.pos[0]).powi(2) + (gr.pos[1] - l.pos[1]).powi(2)).sqrt();
			gr.near = gr.near.max(dh + l.rs);
			gr.area += l.area;
			gr.flick += l.area * l.flick;
		}
	}
	for (i, g) in out.iter_mut().enumerate() {
		g.small = list.iter().filter(|l| l.group == Some(i as u8)).all(|l| l.small);
		g.size = g.size.sqrt().clamp(RS_LO, GROUP);
		g.flick = if g.area > 0.0 { g.flick / g.area } else { 1.0 };
		g.near = (NEAR_SHARE * g.near).clamp(NEAR_LO, NEAR_HI);
	}
	out
}

/// Lane rfire (user 2026-11-07: "the shadows should flicker i think because fire flickers"): fire `g`'s dance now (firefx_light.rs
/// `dance` from the fire's centre and radius: ONE phase per fire): its lights (`dance`) and its shadow origins (firefx.rs
/// `shadow_origins`) move by the same offset, so a light never leaves its shadow origin (the shadowed sway cap holds).
pub(super) fn fire_dance(groups: &[Group], g: usize, t: f64) -> [f32; 3] {
	groups.get(g).map_or([0.0; 3], |gr| super::firefx_light::dance(gr.pos, gr.size, t))
}

/// The fire lights of `list` dance with their fire (`fire_dance`); `live` false (Fire Off, the Classic half) = still (test build 7).
pub(super) fn dance(list: &mut [Light], groups: &[Group], t: f64, live: bool) {
	if !live {
		return;
	}
	for l in list.iter_mut().filter(|l| l.fire) {
		if let Some(g) = l.group {
			let d = fire_dance(groups, usize::from(g), t);
			(0..3).for_each(|k| l.pos[k] += d[k]);
		}
	}
}

/// The fires `groups` (strongest first, at most `n`: test build 4's firefx_light.rs `count`) as the volumetric smoke's lights,
/// test build 4's brightness and reach of a fire of that many m^2 (firefx_light.rs `lights`: fire_light, fire_lreach), then the other
/// rows of `fx` (pool particles, weapon lights: not `fx.fire`), `FX_LIGHTS` in all. The smoke keeps its approved look; its light now
/// sits where the tracked surface lights are (no popping, every fire of the list).
pub(super) fn smoke(groups: &[Group], fx: &FxLights, n: usize) -> FxLights {
	let (gain, cap) = (devtune::f32("fire_light", 1.4), devtune::f32("fire_lreach", 32.0));
	let mut gs: Vec<&Group> = groups.iter().filter(|g| g.area > 0.0).collect();
	gs.sort_by(|a, b| b.area.total_cmp(&a.area));
	let mut out = FxLights::default();
	let rows = gs.iter().take(n).map(|g| {
		let b = (0.35 * g.area.sqrt()).min(1.3) * gain * g.flick;
		let reach = (6.0 + 2.2 * g.area.sqrt()).min(cap.max(4.0));
		[[g.pos[0], g.pos[1], g.pos[2], reach], [b, b * 0.5, b * 0.12, 0.0]]
	});
	let rest = (0..fx.n.min(fx.l.len())).filter(|&i| fx.fire & (1u8 << i) == 0).map(|i| (fx.l[i], fx.shot & (1u8 << i) != 0));
	for (l, shot) in rows.map(|l| (l, false)).chain(rest).take(fx.l.len()) {
		out.l[out.n] = l;
		out.shot |= u8::from(shot) << out.n;
		out.n += 1;
	}
	out
}

/// The grid row (origin x, y of cell 0, 1 / `CELL`, `CELLS`; .w 0 = no light at all) and per cell (row-major, 4 per row) the bit mask
/// of the lights of `list` whose reach box touches it, the border cells also for what lies beyond them.
pub(super) fn grid(list: &[Light], eye: [f32; 3]) -> ([f32; 4], [[u32; 4]; CELLS * CELLS / 4]) {
	let mut cells = [[0u32; 4]; CELLS * CELLS / 4];
	let half = (CELLS / 2) as f32;
	let o = [((eye[0] / CELL).floor() - half) * CELL, ((eye[1] / CELL).floor() - half) * CELL];
	let at = |v: f32, o: f32| (((v - o) / CELL).floor().clamp(0.0, (CELLS - 1) as f32)) as usize;
	for (k, l) in list.iter().take(MAX).enumerate() {
		let (x0, x1) = (at(l.pos[0] - l.reach, o[0]), at(l.pos[0] + l.reach, o[0]));
		let (y0, y1) = (at(l.pos[1] - l.reach, o[1]), at(l.pos[1] + l.reach, o[1]));
		for y in y0..=y1 {
			for x in x0..=x1 {
				let c = y * CELLS + x;
				cells[c / 4][c % 4] |= 1u32 << k;
			}
		}
	}
	let on = if list.is_empty() { 0.0 } else { CELLS as f32 };
	([o[0], o[1], 1.0 / CELL, on], cells)
}

/// RAC_TUNE flit_log=1: the frame's surface lights on stderr every 30th frame.
pub(super) fn log(list: &[Light], groups: &[Group]) {
	static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
	if devtune::f32("flit_log", 0.0) > 0.5 && N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 30 == 0 {
		let ls: Vec<String> = list.iter().map(|l| format!("({:.1},{:.1},{:.1} R{:.0} I{:.2} rs{:.1} g{:?})", l.pos[0], l.pos[1], l.pos[2], l.reach, l.peak(), l.rs, l.group)).collect();
		eprintln!("flit lights {}: {} | fires {groups:?}", list.len(), ls.join(" "));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn e(x: f32, y: f32) -> Emit {
		Emit { base: [x, y, 0.0], h: 4.0, w: 2.0, seed: 0.5 }
	}

	#[test]
	fn the_budget_grows_with_the_level() {
		assert_eq!((budget(0).0, budget(1).0, budget(2).0, budget(3).0), (0, 8, 16, MAX));
		assert!(budget(1).1 > budget(2).1 && budget(2).1 > budget(3).1);
	}

	/// A field of flames is several lights where the flames stand, with the whole field's power; one far flame is its own light.
	#[test]
	fn a_field_of_flames_is_several_lights_with_its_power() {
		let fires: Vec<Emit> = (0..12).map(|i| e(i as f32, 0.0)).chain([e(100.0, 0.0)]).collect();
		let cs = clusters(fires.iter(), 2.5);
		assert!(cs.len() >= 4, "{cs:?}");
		assert!((cs.iter().map(|c| c.power).sum::<f32>() - 13.0 * 8.0).abs() < 1e-3);
		assert!(cs.iter().all(|c| c.rs >= RS_LO && c.rs <= RS_HI && (c.pos[2] - LIFT * 4.0).abs() < 1e-4));
		assert!(clusters(fires.iter(), 6.0).len() < cs.len());
	}

	/// New lights fade in, lost ones fade out where they stood, a moving fire is followed and eased, a standing game changes nothing.
	#[test]
	fn tracks_fade_in_follow_and_fade_out() {
		let mut t = Tracker::default();
		let c = Cluster { pos: [0.0; 3], power: 8.0, rs: 1.0, small: false, carried: false };
		assert_eq!(t.step(&[c], 2.5, 1.0)[0].power, 8.0, "full on the first frame (no fade-in at load)");
		let d = Cluster { pos: [50.0, 0.0, 0.0], ..c };
		let o = t.step(&[c, d], 2.5, 1.05);
		assert_eq!(o.len(), 2);
		assert!(o[1].power > 0.0 && o[1].power < 8.0, "the new one fades in: {o:?}");
		for k in 2..20 {
			t.step(&[c, d], 2.5, 1.0 + 0.05 * k as f64);
		}
		let moved = Cluster { pos: [1.0, 0.0, 0.0], ..c };
		let o = t.step(&[moved, d], 2.5, 2.0);
		assert!(o[0].pos[0] > 0.0 && o[0].pos[0] < 1.0, "eased: {:?}", o[0].pos);
		let o = t.step(&[moved], 2.5, 2.05);
		assert!(o.len() == 2 && o[1].power > 0.0 && o[1].power < 8.0 && o[1].pos == d.pos, "fading out in place");
		assert_eq!(t.step(&[moved], 2.5, 2.05), o, "the game stands: nothing changes");
		for k in 2..10 {
			t.step(&[moved], 2.5, 2.0 + 0.05 * k as f64);
		}
		assert_eq!(t.step(&[moved], 2.5, 2.6).len(), 1, "gone once faded");
	}

	/// Fire Off (`live` false): no dance (test build 7's centres); the Fire row's lights dance with their fire, every light of one
	/// fire by the same offset (one phase per fire, also its shadow origins'), the weapon lights never.
	#[test]
	fn fire_off_lights_do_not_dance() {
		let c = [Cluster { pos: [5.0, 2.0, 1.0], power: 4.0, rs: 1.0, small: false, carried: false }, Cluster { pos: [7.0, 2.0, 1.0], power: 3.0, rs: 1.0, small: false, carried: false }];
		let mut list = fire_lights(&c, 3.3, 2, 0.0, [0.0; 3], true);
		assert!(list.iter().all(|l| c.iter().any(|c| c.pos == l.pos)), "no dance inside fire_lights");
		list.push(Light { pos: [5.5, 2.0, 1.0], reach: 5.0, ..Default::default() });
		let g = groups(&mut list);
		let before = list.clone();
		let mut off = list.clone();
		dance(&mut off, &g, 3.3, false);
		assert_eq!(off, before, "Fire Off: still");
		dance(&mut list, &g, 3.3, true);
		let d = |i: usize| [0, 1, 2].map(|k| list[i].pos[k] - before[i].pos[k]);
		assert_ne!(d(0), [0.0; 3]);
		assert!((0..3).all(|k| (d(0)[k] - d(1)[k]).abs() < 1e-6), "one fire, one offset: {:?} {:?}", d(0), d(1));
		let f = fire_dance(&g, 0, 3.3);
		assert!((0..3).all(|k| (d(0)[k] - f[k]).abs() < 1e-6), "the shadow origins take the same offset");
		assert_eq!(list[2].pos, before[2].pos, "a weapon light does not dance");
	}

	/// Lane rfire review: small flames rank after every big fire for the light slots, a fire of small lights only is `small` (no
	/// shadow map), a cluster with one big flame is big.
	#[test]
	fn small_flames_come_after_the_big_fires() {
		let mut cs: Vec<Cluster> = (0..8).map(|i| Cluster { pos: [3.0 * i as f32, 0.0, 0.0], power: 6.0, rs: 0.5, small: true, carried: false }).collect();
		cs.push(Cluster { pos: [60.0, 0.0, 0.0], power: 1.0, rs: 0.5, small: false, carried: false });
		let l = fire_lights(&cs, 1.0, 4, 0.0, [0.0; 3], true);
		assert!(!l[0].small && l[0].pos[0] > 50.0 && l[1..].iter().all(|l| l.small), "{l:?}");
		let mut list = l.clone();
		let g = groups(&mut list);
		let gi = |i: usize| usize::from(list[i].group.unwrap());
		assert!(!g[gi(0)].small && g[gi(1)].small);
		let e = |x: f32| Emit { base: [x, 0.0, 0.0], h: 1.0, w: 0.5, seed: 0.0 };
		let (a, b, c) = (e(0.0), e(0.5), e(30.0));
		let k = clusters_tagged([(&a, false), (&b, true), (&c, true)].into_iter(), 2.5);
		assert_eq!(k.iter().map(|c| c.small).collect::<Vec<_>>(), vec![false, true], "{k:?}");
	}

	/// Lane rfire review: with every track in use (fading small flames) a new big fire takes the weakest one at once.
	#[test]
	fn a_new_big_fire_evicts_the_weakest_fading_track() {
		let mut t = Tracker::default();
		let smalls: Vec<Cluster> = (0..TRACKS).map(|i| Cluster { pos: [10.0 * i as f32, 0.0, 0.0], power: 0.5 + 0.01 * i as f32, rs: 0.3, small: true, carried: false }).collect();
		t.step(&smalls, 2.5, 1.0);
		let big = Cluster { pos: [0.0, 500.0, 0.0], power: 40.0, rs: 2.0, small: false, carried: false };
		let o = t.step(&[big], 2.5, 1.05);
		assert_eq!(o.len(), TRACKS);
		assert!(o.iter().any(|c| c.pos == big.pos && !c.small && c.power > 0.0), "the big fire has a light at once");
		assert!(!o.iter().any(|c| c.pos == smalls[0].pos), "the weakest small track went");
		let weaker = Cluster { power: 0.1, pos: [0.0, -500.0, 0.0], ..big };
		let o = t.step(&[big, weaker], 2.5, 1.1);
		assert!(!o.iter().any(|c| c.pos == weaker.pos), "a cluster weaker than every track waits");
	}

	#[test]
	fn fire_lights_are_ranked_capped_and_warm() {
		let cs: Vec<Cluster> = (0..40).map(|i| Cluster { pos: [3.0 * i as f32, 0.0, 0.0], power: 1.0 + i as f32, rs: 0.5, small: false, carried: false }).collect();
		let l = fire_lights(&cs, 1.0, MAX, 1.0, [0.0; 3], true);
		assert_eq!(l.len(), MAX);
		assert!(l.iter().all(|l| l.fire && l.rgb[0] > l.rgb[1] && l.rgb[1] > l.rgb[2] && l.reach >= REACH_LO && l.reach <= REACH_HI));
		assert_eq!(fire_lights(&cs, 1.0, 4, 1.0, [0.0; 3], true).len(), 4);
		// A big fire reaches far, a small one not.
		let big = fire_lights(&[Cluster { pos: [0.0; 3], power: 80.0, rs: 3.0, small: false, carried: false }], 1.0, 1, 0.0, [0.0; 3], true)[0];
		let small = fire_lights(&[Cluster { pos: [0.0; 3], power: 0.2, rs: 0.3, small: false, carried: false }], 1.0, 1, 0.0, [0.0; 3], true)[0];
		assert!(big.reach >= 30.0 && small.reach < 10.0, "{big:?} {small:?}");
		// Fire light v2: intensity GAIN per m^2 (FIRE_LIGHT_V2.md 2.2; 1.5 since test build 7), luminance in the blackbody colour
		// (flitcolour.rs), reach where it falls to EDGE: a 20 m^2 cluster (I 30) reaches ~39 m, a 1 m^2 tongue ~9 m.
		let one = |p: f32| fire_lights(&[Cluster { pos: [0.0; 3], power: p, rs: 1.0, small: false, carried: false }], 1.0, 1, 0.0, [0.0; 3], true)[0];
		let (l20, l1) = (one(20.0), one(1.0));
		assert!(20.0 * GAIN / EDGE < REACH_HI * REACH_HI, "the 20 m^2 cluster stays under the reach cap");
		assert!((l20.reach - (20.0 * GAIN / EDGE).sqrt()).abs() < 1e-3 && (l1.reach - (GAIN / EDGE).sqrt()).abs() < 1e-3, "{l20:?} {l1:?}");
		let y = super::super::flitcolour::luma(l20.rgb) / l20.flick;
		assert!((y - 20.0 * GAIN).abs() < 0.015 * 20.0 * GAIN, "luminance = intensity: {y}");
		assert!(l20.rgb[2] < 0.05 * l20.rgb[0], "a ~1900 K flame: no blue to speak of");
	}

	/// The emitter sphere holds the flame volume: sqrt(5 / 3 x spread^2 + half size^2), centred LIFT up the tongues (0.75 since test
	/// build 7).
	#[test]
	fn the_emitter_is_the_flame_volume() {
		let one = clusters([e(0.0, 0.0)].iter(), 2.5);
		assert!((one[0].rs - 2.0).abs() < 1e-5 && (one[0].pos[2] - LIFT * 4.0).abs() < 1e-5, "{one:?}: half size max(w, h) / 2 = 2, centre LIFT x h");
		let two = clusters([e(-1.0, 0.0), e(1.0, 0.0)].iter(), 2.5);
		assert!((two[0].rs - (5.0f32 / 3.0 + 4.0).sqrt()).abs() < 1e-4, "{two:?}");
	}

	/// A fire's shadow-map near plane: 0.8 x its emitter's horizontal half extent, held to [0.25, 4] m.
	#[test]
	fn the_near_plane_holds_the_emitter() {
		let at = |x: f32, z: f32, rs: f32| Light { pos: [x, 0.0, z], reach: 5.0, rgb: [1.0; 3], rs, group: None, fire: true, area: 1.0, flick: 1.0, alt: None, small: false, carried: false };
		let g = groups(&mut [at(-1.0, 0.0, 1.0), at(1.0, 3.0, 1.0)]);
		assert!((g[0].near - 0.8 * 2.0).abs() < 1e-4, "{g:?}: horizontal only");
		assert!((groups(&mut [at(0.0, 0.0, 0.1)])[0].near - NEAR_LO).abs() < 1e-6);
		assert!((groups(&mut [at(0.0, 0.0, 3.9), at(5.0, 0.0, 3.9)])[0].near - NEAR_HI).abs() < 1e-6);
	}

	#[test]
	fn shots_go_first_and_keep_their_strength() {
		let mut fx = FxLights::default();
		fx.l[0] = [[1.0, 2.0, 3.0, 20.0], [1.0, 0.5, 0.1, 2.2]]; // a fire row of the effect lights: replaced by the list
		fx.l[1] = [[7.0, 8.0, 9.0, 5.0], [0.2, 0.4, 1.0, 2.0]]; // a weapon light
		fx.n = 2;
		fx.shot = 0b10;
		let fires = vec![Light { fire: true, reach: 9.0, ..Default::default() }];
		let l = with_shots(fires, &fx);
		assert_eq!(l.len(), 2);
		assert!(!l[0].fire && l[0].pos == [7.0, 8.0, 9.0] && l[0].reach == 5.0 && (l[0].rs - 0.3).abs() < 1e-6);
		assert!((l[0].rgb[2] - 2.0 * 10.0 * 0.09).abs() < 1e-4, "{:?}", l[0].rgb);
		assert!(l[1].fire);
	}

	#[test]
	fn near_lights_are_one_fire_and_the_grid_masks_their_cells() {
		let at = |x: f32, i: f32| Light { pos: [x, 0.0, 0.0], reach: 5.0, rgb: [i, i * 0.5, i * 0.1], rs: 0.5, group: None, fire: true, area: i, flick: 1.0, alt: None, small: false, carried: false };
		let mut list = vec![Light { fire: false, ..at(0.0, 9.0) }, at(0.0, 2.0), at(3.0, 1.0), at(40.0, 1.0)];
		let g = groups(&mut list);
		assert_eq!(g.len(), 2);
		assert_eq!((list[0].group, list[1].group, list[2].group, list[3].group), (None, Some(0), Some(0), Some(1)));
		assert!((g[0].pos[0] - 1.0).abs() < 1e-4 && g[0].reach >= 7.0 && g[0].reach <= REACH_HI);
		assert!(g[0].size > 1.0 && g[0].size < 3.0 && (g[1].size - 0.5).abs() < 1e-4, "{g:?}");
		let (row, cells) = grid(&list, [0.0; 3]);
		assert_eq!(row, [-64.0, -64.0, 1.0 / CELL, CELLS as f32]);
		let cell = |x: f32, y: f32| {
			let c = (((y - row[1]) / CELL).floor().clamp(0.0, 15.0) as usize) * CELLS + ((x - row[0]) / CELL).floor().clamp(0.0, 15.0) as usize;
			cells[c / 4][c % 4]
		};
		assert_eq!(cell(1.0, 1.0), 0b0111);
		assert_eq!(cell(40.0, 0.0), 0b1000);
		assert_eq!(cell(-30.0, 30.0), 0);
		// Past the grid: the border cell holds what lies beyond.
		let far = [Light { pos: [500.0, 0.0, 0.0], reach: 5.0, ..list[3] }];
		assert_eq!(grid(&far, [0.0; 3]).1.iter().flatten().filter(|m| **m != 0).count(), 2, "column 15, rows 7 and 8");
		assert_eq!(grid(&[], [0.0; 3]).0[3], 0.0, "no light: the shader returns the colour untouched");
	}

	/// The smoke takes one light per fire with test build 4's brightness / reach of its m^2, strongest first, at most `n`; the old fire
	/// rows of the effect lights go, the pool / weapon rows stay (shot bit kept).
	#[test]
	fn the_smoke_takes_the_fires_with_the_old_brightness() {
		let at = |x: f32, a: f32| Light { pos: [x, 0.0, 0.0], reach: 9.0, rgb: [a, a * 0.5, a * 0.1], rs: 0.5, group: None, fire: true, area: a, flick: 1.0, alt: None, small: false, carried: false };
		let mut list = vec![at(0.0, 16.0), at(2.0, 9.0), at(50.0, 4.0), at(90.0, 1.0)];
		let g = groups(&mut list);
		let mut fx = FxLights::default();
		fx.l[0] = [[0.0, 0.0, 0.0, 20.0], [1.8, 0.9, 0.2, 2.2]]; // test build 4's fire row
		fx.l[1] = [[5.0, 5.0, 5.0, 3.0], [0.3, 0.3, 0.3, 0.0]]; // a pool particle light
		fx.l[2] = [[7.0, 7.0, 7.0, 5.0], [0.2, 0.4, 1.0, 2.0]]; // a weapon light
		(fx.n, fx.fire, fx.shot) = (3, 0b001, 0b100);
		let s = smoke(&g, &fx, 2);
		assert_eq!(s.n, 4);
		let b = (0.35f32 * 25.0f32.sqrt()).min(1.3) * 1.4;
		assert!((s.l[0][1][0] - b).abs() < 1e-5 && (s.l[0][0][3] - (6.0 + 2.2 * 5.0)).abs() < 1e-4 && s.l[0][0][0] > 0.0 && s.l[0][0][0] < 2.0, "{:?}", s.l[0]);
		assert!((s.l[1][1][0] - 0.35 * 2.0 * 1.4).abs() < 1e-5, "the 4 m^2 fire second; the 1 m^2 one past n");
		assert_eq!((s.l[2], s.l[3], s.shot, s.fire), (fx.l[1], fx.l[2], 0b1000, 0));
	}
}
