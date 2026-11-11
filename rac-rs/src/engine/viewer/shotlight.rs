//! [PC] Weapon lights (v0.5, lane nwlit; Graphics -> Lighting Enhanced with Particles Enhanced and Global illumination not Off):
//! the game pack's shots (`Pack::shot_lights`, engine/shot_rows.rs) are real moving point lights. Per display frame the sources come in
//! already smoothed (`props`: the drawn mobys of a shot class at their interpolated place; `sprites`: the pool sprites of a flame type
//! and the pack's own points, `PoolFx::shots`), `frame` tracks them (shot_rows.rs `Tracker`: fade in / out, flicker, at most the
//! Global illumination level's count, `shot_count`) and puts them into the free dynamic-light slots of the level uniform (`u.points`: the
//! moving highlights on metal, metalfx.wgsl `metal_lights`, and the scenery mobys' own light), and `merge` ranks them with the frame's
//! effect lights (colour .w = the screen-light gain, post.rs `fx_light`: the screen-space light on the world, the hero and the enemies,
//! with an albedo estimate so dark places light up too; the smoke and the smoke volumes take the colour without that gain, as bright
//! as any effect light). The fades and the flicker run on the GAME clock (firefx_gametime.rs), so a pause freezes them. Off (Classic,
//! Particles Classic, GI Off, a game without rows): nothing is written, the frame is the one before this feature, and the pack's field
//! walks for the points stand (`shot_rows::wanted`). Render thread only (`STATE`).
//! Split view: gated by the caller (prepare.rs `post_on` = `splitview::fx`), so the Classic half gets no weapon lights.

use std::cell::RefCell;

use super::firefx_gametime::GameClock;
use super::partfx::{FxLights, FX_LIGHTS};
use crate::shot_rows::{cluster, Cand, Shot, ShotRow, ShotSrc, Tracker, MAX_SHOTS};

/// How far from a shot an effect light gives it its hue (m), and how near one is the shot's own (the smooth shot light crossfades
/// with the effect light gathered from its particles: `merged`).
const HUE_R: f32 = 2.5;
const OWN_R: f32 = 2.0;
/// The longest frame step of the fades (s): a hitch does not pop a light to full.
const MAX_DT: f32 = 0.05;
/// Brightness of the screen-space light (`merge`, RAC_TUNE shot_gain) and of the metal highlight / scenery-moby light in the
/// point slots (`frame`, RAC_TUNE shot_spec): tuned on pictures 2026-11-03 (1.0 / 0.5 and 1.5 read as too faint next to the flames;
/// 2.2 lights the ground around each shot without washing it out; the highlight core already clips at 0.8).
const GAIN: f32 = 2.2;
const SPEC: f32 = 0.8;

#[derive(Default)]
struct State {
	mobys: Vec<Cand>,
	sprites: Vec<Cand>,
	tracker: Tracker,
	lights: Vec<Shot>,
	/// The game clock (fed by `sprites` with the pool's frame counter, read by `frame`) and its time at the last `frame`.
	game: GameClock,
	drawn: Option<f64>,
	clock: f64,
	level: usize,
	on: bool,
}

thread_local! {
	/// Render thread: this frame's sources, the tracked lights and the clock of the fades and the flicker (STATE.md).
	static STATE: RefCell<State> = RefCell::new(State::default());
}

fn rows() -> &'static [ShotRow] {
	crate::pack::get().shot_lights
}

fn row_of(rows: &[ShotRow], src: ShotSrc) -> Option<usize> {
	rows.iter().position(|r| r.src == src)
}

/// The weapon lights a frame may have at Global illumination level `q` (1 Low, 2 Medium, 3 High; user 2026-11-04: "the same goes for
/// weapon lights", plugged into the GI row like the fire's lights, firefx_light.rs), on top of `partfx::light_count`'s budget.
pub(super) const fn shot_count(q: u8) -> usize {
	match q {
		1 => 1,
		2 => 3,
		_ => MAX_SHOTS,
	}
}

/// `Gpu::set_props`: the mobys of a shot class the game draws (`draw_shown`) within the draw distance (`alphas` Some, not `past` it),
/// whether the occlusion test or the Culling option hides them (a rocket round a corner still lights the floor in view), at their
/// smoothed place (`smooth::prop_xf`). Nothing while the feature is off.
pub(super) fn props(props: &[crate::prop::Prop], alphas: &[Option<u8>], past: &[bool]) {
	let rows = rows();
	if rows.is_empty() {
		return;
	}
	STATE.with(|s| {
		let mut s = s.borrow_mut();
		s.mobys.clear();
		if !s.on {
			return;
		}
		for (pi, p) in props.iter().enumerate() {
			if !p.draw_shown() || alphas.get(pi).copied().flatten().is_none() || past.get(pi) == Some(&true) {
				continue;
			}
			let Some(row) = row_of(rows, ShotSrc::Moby(p.class_id)) else { continue };
			let pos = super::smooth::prop_xf(pi).map_or(p.pos, |(m, _)| [m[12], m[13], m[14]]);
			s.mobys.push(Cand { row, id: p.index.map_or(0, |i| i + 1), pos, w: 1.0, rgb: None });
		}
	});
}

/// `Gpu::set_particles`: moves the game clock on (the pool's frame counter; a swapped world clears the lights), then the pool sprites
/// of a flame type (the k-th pool sprite has effect type `fx.sprite_types[k]`; weight size x alpha) and the pack's points of this
/// frame (`fx.shots`), each within its row's `hero_r` of the hero. Nothing while the feature is off.
pub(super) fn sprites(sprites: &[crate::prims::Sprite], fx: Option<&crate::fxpool::PoolFx<'_>>) {
	let rows = rows();
	if rows.is_empty() {
		return;
	}
	STATE.with(|s| {
		let mut s = s.borrow_mut();
		s.sprites.clear();
		if !s.on {
			s.game.rest();
			return;
		}
		if s.game.update(super::firefx_clock::age(), fx.map(|f| f.frames), crate::field::field_dt() as f64) {
			s.tracker.clear();
		}
		let Some(fx) = fx else { return };
		let near_hero = |p: [f32; 3], r: f32| r <= 0.0 || fx.hero.is_some_and(|h| (0..3).map(|i| (p[i] - h[i]) * (p[i] - h[i])).sum::<f32>() < r * r);
		if !fx.sprite_types.is_empty() && rows.iter().any(|r| matches!(r.src, ShotSrc::Sprite(_))) {
			for (k, sp) in sprites.iter().filter(|sp| sp.pool.is_some()).enumerate() {
				let Some(&ty) = fx.sprite_types.get(k) else { break };
				let Some(row) = row_of(rows, ShotSrc::Sprite(ty)) else { continue };
				let w = sp.size.max(0.0) * sp.colour[3].max(0.0);
				if w > 0.0 && near_hero(sp.pos, rows[row].hero_r) {
					s.sprites.push(Cand { row, id: 0, pos: sp.pos, w, rgb: None });
				}
			}
		}
		for p in fx.shots {
			if let Some(row) = row_of(rows, ShotSrc::Point(p.tag)).filter(|&r| near_hero(p.pos, rows[r].hero_r)) {
				s.sprites.push(Cand { row, id: p.id, pos: p.pos, w: 1.0, rgb: p.rgb });
			}
		}
		crate::shot_rows::drop_lone(rows, &mut s.sprites);
	});
}

/// Writes `lights` into the free slots (a.w <= 0) of the level uniform's 8 point lights: (position, reach), (colour x metal gain, 1 /
/// reach), the falloff `metal_lights` and `points_moby` read.
fn fill_points(points: &mut [[f32; 4]; 16], lights: &[Shot], gain: f32) {
	let mut k = 0;
	for l in lights {
		while k < 8 && points[2 * k][3] > 0.0 {
			k += 1;
		}
		if k == 8 {
			return;
		}
		let g = l.spec * gain;
		points[2 * k] = [l.pos[0], l.pos[1], l.pos[2], l.reach];
		points[2 * k + 1] = [l.rgb[0] * g, l.rgb[1] * g, l.rgb[2] * g, 1.0 / l.reach];
		k += 1;
	}
}

/// `Gpu::frame_uniforms`, before the level uniform is written: moves the lights on to this display frame and puts them into the free
/// slots of `points` (`u.points`). `on` = the gate (Lighting + Particles Enhanced, GI not Off); `level` = the loaded level; `q` = the
/// Global illumination level (`shot_count`).
pub(super) fn frame(points: &mut [[f32; 4]; 16], eye: [f32; 3], on: bool, level: usize, q: u8) {
	frame_in(rows(), points, eye, on, level, q);
}

fn frame_in(rows: &[ShotRow], points: &mut [[f32; 4]; 16], eye: [f32; 3], on: bool, level: usize, q: u8) {
	STATE.with(|s| {
		let mut s = s.borrow_mut();
		s.on = on && !rows.is_empty();
		crate::shot_rows::set_wanted(s.on);
		if !s.on {
			// Off: the lights are forgotten (a later On starts afresh); the sources stop at the next `props` / `sprites`.
			s.tracker.clear();
			s.lights.clear();
			s.drawn = None;
			return;
		}
		if level != s.level {
			s.tracker.clear();
			s.level = level;
		}
		// The frame step: the game's draw clock (firefx_gametime.rs; RAC_TEST_CLOCK under shot runs): standing in a pause; none on the
		// first On frame, so nothing starts at full.
		let now = s.game.shown(super::firefx_clock::age());
		let dt = s.drawn.replace(now).map_or(0.0, |d| (now - d).clamp(0.0, MAX_DT as f64)) as f32;
		s.clock = now;
		let cap = shot_count(q);
		let all: Vec<Cand> = s.mobys.iter().chain(s.sprites.iter()).copied().collect();
		let cands = cluster(rows, &all);
		s.tracker.step(rows, &cands, eye, cap, dt);
		s.lights = s.tracker.lights(rows, eye, s.clock, cap);
		fill_points(points, &s.lights, crate::devtune::f32("shot_spec", SPEC));
		if log_on() && !cands.is_empty() {
			log(&s, &cands, eye);
		}
	});
}

/// RAC_SHOT_LOG (debug): each On frame with sources prints its sources and lights (`SHOTLOG` lines in the run log).
fn log_on() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var_os("RAC_SHOT_LOG").is_some())
}

fn log(s: &State, cands: &[Cand], eye: [f32; 3]) {
	let v = |p: [f32; 3]| format!("{:.1} {:.1} {:.1}", p[0], p[1], p[2]);
	let c: Vec<String> = cands.iter().map(|c| format!("r{} id{} ({}) w{:.2}", c.row, c.id, v(c.pos), c.w)).collect();
	let l: Vec<String> = s.lights.iter().map(|l| format!("({}) rgb({}) reach {:.1} lv {:.2}", v(l.pos), v(l.rgb), l.reach, l.lv)).collect();
	eprintln!("SHOTLOG t{:.3} eye ({}) cands [{}] lights [{}]", s.clock, v(eye), c.join("; "), l.join("; "));
}

/// `Gpu::pass_fx_lights`: the frame's effect lights and the weapon lights ranked together (`merged`), at most the Particles quality's
/// budget (`partfx::light_count(q)`, never fewer than the effect lights alone) plus the weapon lights (their count is the GI level's,
/// `shot_count`), `FX_LIGHTS` in all; the effect lights gathered from a shot's own particles
/// (within `OWN_R`) give it their hue (`Tracker::sample`) and crossfade with it.
pub(super) fn merge(base: FxLights, on: bool, eye: [f32; 3], q: u8) -> FxLights {
	STATE.with(|s| {
		let mut s = s.borrow_mut();
		if !on || !s.on || s.tracker.tracks.is_empty() {
			return base;
		}
		let ls: Vec<([f32; 3], [f32; 3])> = base.l[..base.n].iter().map(|l| ([l[0][0], l[0][1], l[0][2]], [l[1][0], l[1][1], l[1][2]])).collect();
		s.tracker.sample(rows(), &ls, HUE_R);
		merged(&base, &s.lights, crate::devtune::f32("shot_gain", GAIN), eye, super::partfx::light_count(q).max(base.n) + s.lights.len())
	})
}

fn d2(a: [f32; 3], b: &[f32; 4]) -> f32 {
	(0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum()
}

/// One list of at most `cap` lights, the strongest for `eye` first (brightness over distance, as `Tracker::lights` ranks the shots):
/// the shots (colour without the screen gain, .w = `gain`: post.rs multiplies, the smoke does not) and the effect lights, each of
/// those dimmed by 1 - the fade of the shots within `OWN_R` (a rocket's light at its impact hands over to the explosion's light
/// without a dip; a shot at full replaces its own effect light).
fn merged(base: &FxLights, lights: &[Shot], gain: f32, eye: [f32; 3], cap: usize) -> FxLights {
	let g = gain.max(1e-3);
	let far = |p: &[f32; 4]| 1.0 + d2(eye, p).sqrt() / 20.0;
	let mut all: Vec<(f32, [[f32; 4]; 2], bool, bool)> = lights.iter().map(|l| (l.rank * g, [[l.pos[0], l.pos[1], l.pos[2], l.reach], [l.rgb[0], l.rgb[1], l.rgb[2], g]], true, false)).collect();
	for (i, b) in base.l[..base.n].iter().enumerate() {
		let own = lights.iter().filter(|l| d2(l.pos, &b[0]) < OWN_R * OWN_R).fold(0.0f32, |m, l| m.max(l.lv));
		let k = 1.0 - own;
		if k <= 1e-3 {
			continue;
		}
		let c = [b[1][0] * k, b[1][1] * k, b[1][2] * k, b[1][3]];
		all.push((c[0].max(c[1]).max(c[2]) / far(&b[0]), [b[0], c], false, base.fire & (1u8 << i) != 0));
	}
	all.sort_by(|a, b| b.0.total_cmp(&a.0));
	let mut out = FxLights::default();
	for (_, l, shot, fire) in all.into_iter().take(cap.min(FX_LIGHTS)) {
		out.l[out.n] = l;
		out.shot |= u8::from(shot) << out.n;
		out.fire |= u8::from(fire) << out.n; // [PC] lane dlit2 (flitlist.rs `smoke`)
		out.n += 1;
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	fn shot(x: f32) -> Shot {
		Shot { pos: [x, 0.0, 0.0], reach: 5.0, rgb: [1.0, 0.5, 0.0], spec: 1.0, lv: 1.0, rank: 1.0 / (1.0 + x.abs() / 20.0) }
	}

	#[test]
	fn the_lights_take_only_free_slots() {
		let mut p = [[0.0f32; 4]; 16];
		p[0] = [1.0, 2.0, 3.0, 4.0];
		p[4] = [5.0, 5.0, 5.0, 2.0];
		let before = p;
		fill_points(&mut p, &[], 1.0);
		assert_eq!(p, before, "no lights: nothing written");
		fill_points(&mut p, &[shot(10.0), shot(20.0)], 0.5);
		assert_eq!(p[0], before[0]);
		assert_eq!(p[4], before[4]);
		assert_eq!(p[2], [10.0, 0.0, 0.0, 5.0]);
		assert_eq!(p[3], [0.5, 0.25, 0.0, 0.2]);
		assert_eq!(p[6][0], 20.0, "slot 2 is taken: the second light goes to slot 3");
		let mut full = [[0.0f32; 4]; 16];
		for k in 0..8 {
			full[2 * k][3] = 1.0;
		}
		let f0 = full;
		fill_points(&mut full, &[shot(1.0)], 1.0);
		assert_eq!(full, f0, "all 8 retail slots in use: none taken");
	}

	#[test]
	fn merge_ranks_the_shots_with_the_effect_lights_and_crossfades_their_own() {
		let mut base = FxLights::default();
		base.l[0] = [[10.5, 0.0, 0.0, 3.0], [1.0, 0.4, 0.0, 0.0]];
		base.l[1] = [[40.0, 0.0, 0.0, 3.0], [0.0, 0.0, 1.0, 0.0]];
		base.n = 2;
		let m = merged(&base, &[shot(10.0)], 2.0, [0.0; 3], 8);
		assert_eq!(m.n, 2);
		assert_eq!(m.l[0][1], [1.0, 0.5, 0.0, 2.0], "the shot light first, its colour without the gain, the gain in .w");
		assert_eq!(m.l[1], base.l[1], "the far effect light stays, the shot's own one (a shot at full) is gone");
		let half = merged(&base, &[Shot { lv: 0.25, ..shot(10.0) }], 2.0, [0.0; 3], 8);
		assert_eq!(half.n, 3);
		let back = |l: &[[f32; 4]; 2]| l[0] == base.l[0][0] && (l[1][0] - 0.75).abs() < 1e-6 && (l[1][1] - 0.3).abs() < 1e-6 && l[1][3] == 0.0;
		assert!(half.l[..half.n].iter().any(back), "a fading shot: its own effect light comes back by 1 - fade");
		let none = merged(&base, &[], 2.0, [0.0; 3], 8);
		assert_eq!((none.n, none.l[0], none.l[1]), (2, base.l[0], base.l[1]), "no shots: the effect lights as they were");
		let many: Vec<Shot> = (0..12).map(|k| shot(100.0 + 10.0 * k as f32)).collect();
		assert_eq!(merged(&base, &many, 2.0, [0.0; 3], 8).n, FX_LIGHTS);
		let low = merged(&base, &[shot(60.0)], 2.0, [0.0; 3], 2);
		assert_eq!(low.n, 2, "the budget");
		assert_eq!(low.l[0], base.l[0], "a near effect light outranks a far shot");
		assert_eq!(merged(&base, &[shot(10.0)], 2.0, [0.0; 3], 1).n, 1);
		assert_eq!((shot_count(1), shot_count(2), shot_count(3)), (1, 3, MAX_SHOTS));
	}

	/// m9 (review R27): the gate with rows present writes nothing and keeps the effect lights.
	#[test]
	fn off_writes_nothing_and_keeps_the_effect_lights() {
		let rows = [ShotRow { src: ShotSrc::Point(0), rgb: [1.0; 3], reach: 5.0, gain: 1.0, spec: 1.0, flicker: 0.0, gather: 1.0, max: 3, with: None, hero_r: 0.0, follow: 0.0 }];
		STATE.with(|s| {
			let mut s = s.borrow_mut();
			s.on = true;
			s.sprites = vec![Cand { row: 0, id: 1, pos: [1.0, 0.0, 0.0], w: 1.0, rgb: None }];
		});
		let mut p = [[0.0f32; 4]; 16];
		frame_in(&rows, &mut p, [0.0; 3], false, 3, 3);
		assert_eq!(p, [[0.0f32; 4]; 16]);
		assert!(!crate::shot_rows::wanted(), "the field walks stand");
		let mut base = FxLights::default();
		base.l[0] = [[1.0, 0.0, 0.0, 3.0], [1.0, 1.0, 1.0, 0.0]];
		base.n = 1;
		let m = merge(base, true, [0.0; 3], 3);
		assert_eq!((m.n, m.l[0]), (1, base.l[0]));
		// The first On frame has no time step: the source is there, nothing is lit at full.
		frame_in(&rows, &mut p, [0.0; 3], true, 3, 3);
		assert_eq!(p, [[0.0f32; 4]; 16], "first On frame: nothing at full");
		assert!(crate::shot_rows::wanted());
		frame_in(&rows, &mut p, [0.0; 3], false, 3, 3);
	}
}
