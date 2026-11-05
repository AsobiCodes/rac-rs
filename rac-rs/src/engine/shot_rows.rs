//! [PC] Weapon lights (v0.5, lane nwlit): the game pack's table of what fires light (`Pack::shot_lights`) and the pure tracker that
//! turns this frame's sources into a few smooth point lights. The GPU side (where the sources come from, where the lights go) is
//! `viewer/shotlight.rs`. Game-neutral: a game without rows (`Pack::NONE`, R&C2 today) gets no weapon lights.
//!
//! A source is one of: a moby class (a bolt or a rocket: one light at its drawn, smoothed place), a pool sprite effect type (a flame
//! stream: its sprites gathered into up to `ShotRow::max` lights along it) or a point the pack feeds per field (`ShotPoint`, e.g. both
//! ends of an arc, with a stable id). Each light fades in over `FADE_IN` s and out over `FADE_OUT` s where it was last seen, so a bolt
//! that hits a wall or a stream that stops never pops. At most `cap` lights a frame (the Particles quality's budget): they are chosen on
//! their steady brightness for the eye (no flicker), the ones chosen last frame keep their place unless another is clearly stronger
//! (`HOLD`), and a light that loses its place fades out (`Track::vis`) before the next one fades in, so the cut never pops either.

use std::sync::atomic::{AtomicBool, Ordering};

/// What a row lights from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShotSrc {
	/// Every drawn moby of this class.
	Moby(u32),
	/// The pool sprites of this effect type (slot byte +0, `fxpool::PoolFx::sprite_types`).
	Sprite(u8),
	/// The pack's own points with this tag (`ShotPoint::tag`).
	Point(u8),
}

/// One row of the pack's table: the source, the fallback colour (used where no effect particle of the shot is near enough to take
/// the colour from, `Tracker::sample`), the reach (m), the brightness, the metal highlight gain, the flicker depth (0 = steady), the
/// gathering radius of a sprite / point source (m), how many lights the row may take (every source kind), for a row whose effect other
/// things share the row it belongs to (`with`: it counts only next to a source of that row, `drop_lone`), how near the hero its
/// sprites / points must be (`hero_r`, m; 0 = anywhere: a row whose effect type other fires of the level use too) and how far a light
/// with an id may move in one frame and still be the same light (`follow`, m; 0 = `FOLLOW`: an arc's end that jumps to a new target).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotRow {
	pub src: ShotSrc,
	pub rgb: [f32; 3],
	pub reach: f32,
	pub gain: f32,
	pub spec: f32,
	pub flicker: f32,
	pub gather: f32,
	pub max: u8,
	pub with: Option<ShotSrc>,
	pub hero_r: f32,
	pub follow: f32,
}

/// A point the pack feeds for a field (`viewer::shotlight::sprites`, `PoolFx::shots`): tag, an id that stays the same while the thing
/// lives (a moby slot, a fixed role along an arc; the frame smoothing pairs the points of two fields by tag and id), world position
/// and, when the effect has one, its own colour (1.0 = 0x80 on a GS byte).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotPoint {
	pub tag: u8,
	pub id: u32,
	pub pos: [f32; 3],
	pub rgb: Option<[f32; 3]>,
}

/// Lights a frame at most (Particles High; `viewer::shotlight::shot_count`), fade times (s), how far a light may move in one frame and
/// still be the same light (m), how far from the eye a light still counts (m), the rank bonus of a light chosen last frame.
pub const MAX_SHOTS: usize = 6;
pub const FADE_IN: f32 = 0.05;
pub const FADE_OUT: f32 = 0.18;
const FOLLOW: f32 = 4.0;
const FAR: f32 = 80.0;
const HOLD: f32 = 1.25;

/// The render side wants the pack's points (`viewer::shotlight::frame` sets it each frame: the feature's gate); the pack's field walks
/// that feed them (R&C1 retail/fx_queue.rs) skip their work while it is false. Read on the game's worker thread (STATE.md).
static WANT: AtomicBool = AtomicBool::new(false);

pub fn wanted() -> bool {
	WANT.load(Ordering::Relaxed)
}

pub(crate) fn set_wanted(on: bool) {
	WANT.store(on, Ordering::Relaxed);
}

/// One source this frame: its row, an identity when it has one (moby slot + 1, `ShotPoint::id`; 0 = match by place), position, weight
/// (sprites: size x alpha summed; others 1) and its own colour when the effect gives one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cand {
	pub row: usize,
	pub id: u32,
	pub pos: [f32; 3],
	pub w: f32,
	pub rgb: Option<[f32; 3]>,
}

/// A light the tracker keeps across frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Track {
	pub row: usize,
	pub id: u32,
	pub pos: [f32; 3],
	/// 0..1 fade level of the source; `seen` = a source held it this frame.
	pub level: f32,
	pub seen: bool,
	/// 0..1 fade level of its place among the frame's lights (`pick`); `held` = chosen this frame.
	pub vis: f32,
	pub held: bool,
	pub rgb: [f32; 3],
	/// The colour of the shot's own effect particles (`sample`), when one was near.
	pub hue: Option<[f32; 3]>,
	/// Flicker phase seed (stable while the light lives).
	pub seed: f32,
}

/// One output light: position, reach, colour with brightness, fade and flicker folded in, metal highlight gain, the fade (0..1: how
/// much of the light is there, for the crossfade with the effect lights it replaces) and the steady rank for the eye (no flicker).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
	pub pos: [f32; 3],
	pub reach: f32,
	pub rgb: [f32; 3],
	pub spec: f32,
	pub lv: f32,
	pub rank: f32,
}

/// Drops the candidates of a row with `with` set that have no candidate of that row within their own `gather` (at least 1 m) (R&C1:
/// the fire bursts of the Pyrocitor's stream are an effect type burning wrecks use too; away from the jet they are not the weapon).
pub fn drop_lone(rows: &[ShotRow], cands: &mut Vec<Cand>) {
	let keep: Vec<bool> = cands
		.iter()
		.map(|c| match rows[c.row].with {
			None => true,
			Some(w) => {
				let r = rows[c.row].gather.max(1.0);
				cands.iter().any(|x| rows[x.row].src == w && d2(x.pos, c.pos) < r * r)
			}
		})
		.collect();
	let mut k = keep.into_iter();
	cands.retain(|_| k.next().unwrap_or(true));
}

fn d2(a: [f32; 3], b: [f32; 3]) -> f32 {
	(0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum()
}

fn smooth(x: f32) -> f32 {
	let x = x.clamp(0.0, 1.0);
	x * x * (3.0 - 2.0 * x)
}

/// Gathers the sprite / point candidates of one row into clusters within the row's `gather` radius (weighted centre, heaviest first,
/// the id of the heaviest), at most `max`; moby candidates pass unchanged, at most `max` of a row (the first ones: slot order).
pub fn cluster(rows: &[ShotRow], cands: &[Cand]) -> Vec<Cand> {
	let mut out: Vec<Cand> = Vec::new();
	let mut loose: Vec<&Cand> = Vec::new();
	for c in cands {
		match rows.get(c.row).map(|r| (r.src, r.max as usize)) {
			Some((ShotSrc::Moby(_), max)) if out.iter().filter(|o| o.row == c.row).count() < max => out.push(*c),
			Some((ShotSrc::Moby(_), _)) | None => {}
			Some(_) => loose.push(c),
		}
	}
	loose.sort_by(|a, b| b.w.total_cmp(&a.w));
	// Per cluster: row, id, weighted position sum, weight sum, colour sum (weighted), colour weight.
	let mut acc: Vec<(usize, u32, [f32; 3], f32, [f32; 3], f32)> = Vec::new();
	for c in loose {
		let r = &rows[c.row];
		let near = acc.iter().position(|a| a.0 == c.row && d2([a.2[0] / a.3, a.2[1] / a.3, a.2[2] / a.3], c.pos) < r.gather * r.gather);
		let i = match near {
			Some(i) => i,
			None if acc.iter().filter(|a| a.0 == c.row).count() < r.max as usize => {
				acc.push((c.row, c.id, [0.0; 3], 0.0, [0.0; 3], 0.0));
				acc.len() - 1
			}
			None => continue,
		};
		let a = &mut acc[i];
		for k in 0..3 {
			a.2[k] += c.pos[k] * c.w;
		}
		a.3 += c.w;
		if let Some(rgb) = c.rgb {
			for k in 0..3 {
				a.4[k] += rgb[k] * c.w;
			}
			a.5 += c.w;
		}
	}
	out.extend(acc.into_iter().filter(|a| a.3 > 0.0).map(|a| Cand {
		row: a.0,
		id: a.1,
		pos: [a.2[0] / a.3, a.2[1] / a.3, a.2[2] / a.3],
		w: a.3,
		rgb: (a.5 > 0.0).then(|| [a.4[0] / a.5, a.4[1] / a.5, a.4[2] / a.5]),
	}));
	out
}

/// The tracker: matches this frame's (clustered) candidates to last frame's lights, fades, and gives the frame's lights.
#[derive(Default)]
pub struct Tracker {
	pub tracks: Vec<Track>,
	seeds: f32,
}

impl Tracker {
	/// Moves the lights on by `dt` seconds of game time with this frame's candidates `cands` (from `cluster`) and chooses the at most
	/// `cap` lights of the frame for `eye` (`pick`).
	pub fn step(&mut self, rows: &[ShotRow], cands: &[Cand], eye: [f32; 3], cap: usize, dt: f32) {
		for t in self.tracks.iter_mut() {
			t.seen = false;
		}
		for c in cands {
			let Some(row) = rows.get(c.row) else { continue };
			let fol = if row.follow > 0.0 { row.follow } else { FOLLOW };
			let hit = match c.id {
				0 => self.tracks.iter().enumerate().filter(|(_, t)| !t.seen && t.row == c.row && t.id == 0 && d2(t.pos, c.pos) < FOLLOW * FOLLOW).min_by(|a, b| d2(a.1.pos, c.pos).total_cmp(&d2(b.1.pos, c.pos))).map(|(i, _)| i),
				id => self.tracks.iter().position(|t| !t.seen && t.row == c.row && t.id == id && d2(t.pos, c.pos) < fol * fol),
			};
			match hit {
				Some(i) => {
					let t = &mut self.tracks[i];
					let rgb = c.rgb.or(t.hue).unwrap_or(row.rgb);
					t.pos = c.pos;
					t.seen = true;
					// The colour eases towards the sampled one (a cluster's mean hue moves a little every frame).
					for k in 0..3 {
						t.rgb[k] += (rgb[k] - t.rgb[k]) * (dt * 12.0).min(1.0);
					}
				}
				None => {
					self.seeds += 1.618;
					let rgb = c.rgb.unwrap_or(row.rgb);
					self.tracks.push(Track { row: c.row, id: c.id, pos: c.pos, level: 0.0, seen: true, vis: 0.0, held: false, rgb, hue: None, seed: self.seeds % 97.0 });
				}
			}
		}
		for t in self.tracks.iter_mut() {
			t.level = match t.seen {
				true => (t.level + dt / FADE_IN).min(1.0),
				false => t.level - dt / FADE_OUT,
			};
		}
		self.tracks.retain(|t| t.level > 0.0 && t.row < rows.len());
		self.pick(rows, eye, cap, dt);
	}

	/// The steady rank of a light for `eye`: brightness x source fade over distance (no flicker, so two lights of nearly the same rank
	/// do not swap with their flicker phases); 0 beyond `FAR`.
	fn rank(rows: &[ShotRow], t: &Track, eye: [f32; 3]) -> f32 {
		let d = d2(t.pos, eye);
		match d < FAR * FAR {
			true => rows[t.row].gain * smooth(t.level) / (1.0 + d.sqrt() / 20.0),
			false => 0.0,
		}
	}

	/// Chooses the frame's lights: the `cap` strongest (a light chosen last frame counts `HOLD` x). A chosen light fades its place in
	/// (`vis`) once a place is free; one that lost its place fades it out and keeps the place until it is gone, so the at most `cap`
	/// lights on screen never change in one frame.
	fn pick(&mut self, rows: &[ShotRow], eye: [f32; 3], cap: usize, dt: f32) {
		let mut order: Vec<(f32, usize)> = self
			.tracks
			.iter()
			.enumerate()
			.map(|(i, t)| (Self::rank(rows, t, eye) * if t.held { HOLD } else { 1.0 }, i))
			.filter(|(r, _)| *r > 1e-4)
			.collect();
		order.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
		let mut want = vec![false; self.tracks.len()];
		for &(_, i) in order.iter().take(cap) {
			want[i] = true;
		}
		let mut busy = self.tracks.iter().filter(|t| t.vis > 0.0).count();
		for &(_, i) in &order {
			let t = &mut self.tracks[i];
			if want[i] && (t.vis > 0.0 || busy < cap) {
				busy += (t.vis <= 0.0) as usize;
				t.vis = (t.vis + dt / FADE_IN).min(1.0);
			}
		}
		for (t, w) in self.tracks.iter_mut().zip(want) {
			if !w {
				t.vis = (t.vis - dt / FADE_OUT).max(0.0);
			}
			t.held = w && t.vis > 0.0;
		}
	}

	/// The frame's lights at game clock `time` (s, for the flicker): those with a place (`pick`), the steadily strongest for `eye`
	/// first, at most `cap`.
	pub fn lights(&self, rows: &[ShotRow], eye: [f32; 3], time: f64, cap: usize) -> Vec<Shot> {
		let mut out: Vec<Shot> = self
			.tracks
			.iter()
			.filter(|t| t.vis > 0.0 && d2(t.pos, eye) < FAR * FAR)
			.map(|t| {
				let r = &rows[t.row];
				let ph = time + t.seed as f64;
				let f = 1.0 + r.flicker * (0.55 * (ph * 17.3).sin() + 0.3 * (ph * 31.1).sin() + 0.15 * (ph * 53.0).sin()) as f32;
				// Smoothstep of both fades: no visible step where a light starts, ends, takes or gives up its place.
				let lv = smooth(t.level) * smooth(t.vis);
				let b = r.gain * lv * f.max(0.0);
				let top = t.rgb[0].max(t.rgb[1]).max(t.rgb[2]).max(1e-4);
				let rank = r.gain * lv / (1.0 + d2(t.pos, eye).sqrt() / 20.0);
				Shot { pos: t.pos, reach: r.reach, rgb: [t.rgb[0] / top * b, t.rgb[1] / top * b, t.rgb[2] / top * b], spec: r.spec * lv, lv, rank }
			})
			.filter(|s| s.rank > 1e-4)
			.collect();
		out.sort_by(|a, b| b.rank.total_cmp(&a.rank));
		out.truncate(cap);
		out
	}

	/// Takes each light's hue from the nearest effect light within `r` m (`lights`: position, colour), so a shot glows in the colour
	/// of its own particles; a light with none near keeps the hue it had (else its row's colour). A `Point` row keeps its own colour
	/// (the pack's point colour, else the row's): its effect is not made of pool particles, so a nearby effect light is not its own.
	pub fn sample(&mut self, rows: &[ShotRow], lights: &[([f32; 3], [f32; 3])], r: f32) {
		for t in self.tracks.iter_mut() {
			if rows.get(t.row).is_none_or(|row| matches!(row.src, ShotSrc::Point(_))) {
				continue;
			}
			let near = lights.iter().filter(|l| d2(l.0, t.pos) < r * r).min_by(|a, b| d2(a.0, t.pos).total_cmp(&d2(b.0, t.pos)));
			if let Some((_, c)) = near {
				let top = c[0].max(c[1]).max(c[2]);
				if top > 1e-4 {
					t.hue = Some([c[0] / top, c[1] / top, c[2] / top]);
				}
			}
		}
	}

	pub fn clear(&mut self) {
		self.tracks.clear();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const fn row(src: ShotSrc, rgb: [f32; 3], reach: f32, gather: f32, max: u8) -> ShotRow {
		ShotRow { src, rgb, reach, gain: 1.0, spec: 1.0, flicker: 0.0, gather, max, with: None, hero_r: 0.0, follow: 0.0 }
	}

	const ROWS: [ShotRow; 3] = [
		row(ShotSrc::Moby(305), [1.0, 0.5, 0.1], 6.0, 0.0, 8),
		ShotRow { gain: 1.2, ..row(ShotSrc::Sprite(12), [1.0, 0.6, 0.2], 8.0, 2.5, 2) },
		row(ShotSrc::Point(0), [0.5, 0.5, 1.0], 5.0, 1.5, 3),
	];
	const EYE: [f32; 3] = [0.0; 3];

	fn moby(id: u32, x: f32) -> Cand {
		Cand { row: 0, id, pos: [x, 0.0, 0.0], w: 1.0, rgb: None }
	}

	fn step(t: &mut Tracker, rows: &[ShotRow], c: &[Cand], dt: f32) {
		t.step(rows, c, EYE, MAX_SHOTS, dt);
	}

	#[test]
	fn a_burst_row_counts_only_next_to_its_stream() {
		let mut rows = ROWS;
		rows[2].with = Some(ShotSrc::Sprite(12));
		let c = |row: usize, x: f32| Cand { row, id: 0, pos: [x, 0.0, 0.0], w: 1.0, rgb: None };
		let mut lone = vec![c(0, 0.0), c(2, 0.0)];
		drop_lone(&rows, &mut lone);
		assert_eq!(lone, vec![c(0, 0.0)]);
		let mut both = vec![c(2, 0.0), c(1, 1.0)];
		drop_lone(&rows, &mut both);
		assert_eq!(both, vec![c(2, 0.0), c(1, 1.0)]);
		let mut away = vec![c(2, 0.0), c(1, 6.0)];
		drop_lone(&rows, &mut away);
		assert_eq!(away, vec![c(1, 6.0)], "a burst far from the jet is not the weapon's");
	}

	#[test]
	fn a_light_fades_in_and_out_without_a_step() {
		let mut t = Tracker::default();
		step(&mut t, &ROWS, &[moby(1, 0.0)], 0.02);
		let a = t.lights(&ROWS, EYE, 0.0, MAX_SHOTS)[0].rgb[0];
		assert!(a > 0.0 && a < 0.5, "first frame is dim: {a}");
		for _ in 0..10 {
			step(&mut t, &ROWS, &[moby(1, 0.0)], 0.02);
		}
		assert!((t.lights(&ROWS, EYE, 0.0, MAX_SHOTS)[0].rgb[0] - 1.0).abs() < 1e-4, "full after FADE_IN");
		let mut last = 1.0;
		for _ in 0..20 {
			step(&mut t, &ROWS, &[], 0.02);
			let now = t.lights(&ROWS, EYE, 0.0, MAX_SHOTS).first().map_or(0.0, |s| s.rgb[0]);
			assert!(now <= last && last - now < 0.35, "fades out smoothly: {last} -> {now}");
			last = now;
		}
		assert!(t.tracks.is_empty(), "gone after FADE_OUT");
		let mut z = Tracker::default();
		step(&mut z, &ROWS, &[moby(1, 0.0)], 0.0);
		assert!(z.lights(&ROWS, EYE, 0.0, MAX_SHOTS).is_empty(), "a frame with no time (first frame, pause) shows nothing new");
	}

	#[test]
	fn a_moving_bolt_keeps_its_light_and_a_far_jump_is_a_new_one() {
		let mut t = Tracker::default();
		for k in 0..5 {
			step(&mut t, &ROWS, &[moby(3, k as f32 * 0.8)], 0.02);
		}
		assert_eq!(t.tracks.len(), 1);
		assert_eq!(t.tracks[0].pos[0], 3.2);
		step(&mut t, &ROWS, &[moby(3, 40.0)], 0.02);
		assert_eq!(t.tracks.len(), 2, "a slot reused far away starts a new light, the old one fades");
		let mut rows = ROWS;
		rows[2].follow = 50.0;
		let mut a = Tracker::default();
		step(&mut a, &rows, &[Cand { row: 2, ..moby(3, 0.0) }], 0.02);
		step(&mut a, &rows, &[Cand { row: 2, ..moby(3, 20.0) }], 0.02);
		assert_eq!(a.tracks.len(), 1, "a row with a long follow keeps an arc's end that jumped");
	}

	#[test]
	fn sprites_gather_into_at_most_max_lights_and_mobys_keep_their_row_max() {
		let cands: Vec<Cand> = (0..12).map(|k| Cand { row: 1, id: 0, pos: [k as f32 * 0.5, 0.0, 0.0], w: 1.0, rgb: Some([1.0, 0.5, 0.0]) }).collect();
		let c = cluster(&ROWS, &cands);
		assert_eq!(c.len(), 2, "row max 2");
		assert!(c.iter().all(|c| c.rgb == Some([1.0, 0.5, 0.0])));
		assert!(cluster(&ROWS, &[]).is_empty());
		let mut rows = ROWS;
		rows[0].max = 3;
		let m: Vec<Cand> = (0..5).map(|k| moby(k + 1, k as f32)).collect();
		assert_eq!(cluster(&rows, &m), m[..3].to_vec(), "moby rows: the first `max`");
		let p = [Cand { row: 2, ..moby(7, 0.0) }, Cand { row: 2, ..moby(9, 1.0) }, Cand { row: 2, ..moby(8, 6.0) }];
		let ids: Vec<u32> = cluster(&ROWS, &p).iter().map(|c| c.id).collect();
		assert_eq!(ids, vec![7, 8], "a point cluster keeps the id of its first point");
	}

	#[test]
	fn at_most_cap_nearest_first_and_none_far_away() {
		let mut t = Tracker::default();
		let cands: Vec<Cand> = (0..10).map(|k| moby(k + 1, k as f32 * 5.0)).collect();
		for _ in 0..5 {
			step(&mut t, &ROWS, &cands, 0.02);
		}
		let l = t.lights(&ROWS, EYE, 0.0, MAX_SHOTS);
		assert_eq!(l.len(), MAX_SHOTS);
		assert_eq!(l[0].pos[0], 0.0, "nearest first");
		assert!(t.lights(&ROWS, [1000.0, 0.0, 0.0], 0.0, MAX_SHOTS).is_empty(), "beyond FAR: none");
		let mut low = Tracker::default();
		for _ in 0..5 {
			low.step(&ROWS, &cands, EYE, 1, 0.02);
		}
		assert_eq!(low.lights(&ROWS, EYE, 0.0, 1).len(), 1, "the quality's cap");
	}

	/// M1 (review R27): eight flickering lights of nearly the same rank, six places: the chosen set holds for 60 frames, and a light
	/// that loses its place to a stronger new one fades out (no frame drops more than a fade step) before the new one comes in.
	#[test]
	fn the_chosen_lights_hold_and_a_cut_light_fades() {
		let mut rows = ROWS;
		rows[0].flicker = 0.3;
		let mut t = Tracker::default();
		let cands: Vec<Cand> = (0..8).map(|k| moby(k + 1, 10.0 + 0.01 * k as f32)).collect();
		for _ in 0..20 {
			step(&mut t, &rows, &cands, 1.0 / 60.0);
		}
		let set = |t: &Tracker| -> Vec<u32> {
			let mut v: Vec<u32> = t.tracks.iter().filter(|x| x.vis > 0.0).map(|x| x.id).collect();
			v.sort();
			v
		};
		let first = set(&t);
		assert_eq!(first.len(), MAX_SHOTS);
		for k in 0..60 {
			step(&mut t, &rows, &cands, 1.0 / 60.0);
			assert_eq!(set(&t), first, "frame {k}");
			assert!(t.lights(&rows, EYE, k as f64 / 60.0, MAX_SHOTS).len() <= MAX_SHOTS);
		}
		// A much nearer new light: the weakest chosen one fades out, then the new one fades in; never more than MAX_SHOTS.
		let mut more = cands.clone();
		more.push(moby(99, 1.0));
		let lit = |t: &Tracker, id: u32| t.tracks.iter().find(|x| x.id == id).map_or(0.0, |x| smooth(x.vis));
		let gone = *first.last().unwrap();
		let mut last = lit(&t, gone);
		for k in 0..40 {
			step(&mut t, &rows, &more, 1.0 / 60.0);
			assert!(t.tracks.iter().filter(|x| x.vis > 0.0).count() <= MAX_SHOTS, "frame {k}");
			let now = lit(&t, gone);
			assert!(last - now < 0.2, "the cut light fades: {last} -> {now}");
			last = now;
		}
		assert!(lit(&t, 99) > 0.99 && set(&t).contains(&99), "the new light is in");
	}

	#[test]
	fn the_sampled_colour_wins_over_the_row_colour() {
		let mut t = Tracker::default();
		step(&mut t, &ROWS, &[Cand { rgb: Some([0.2, 0.4, 1.0]), ..moby(1, 0.0) }], 0.02);
		assert_eq!(t.tracks[0].rgb, [0.2, 0.4, 1.0]);
	}

	#[test]
	fn a_sampled_hue_steers_the_colour_and_far_lights_are_ignored() {
		let mut t = Tracker::default();
		step(&mut t, &ROWS, &[moby(1, 0.0)], 0.02);
		t.sample(&ROWS, &[([10.0, 0.0, 0.0], [0.0, 1.0, 0.0])], 2.5);
		assert_eq!(t.tracks[0].hue, None, "too far");
		t.sample(&ROWS, &[([1.0, 0.0, 0.0], [0.0, 0.5, 0.25])], 2.5);
		assert_eq!(t.tracks[0].hue, Some([0.0, 1.0, 0.5]));
		let mut p = Tracker::default();
		step(&mut p, &ROWS, &[Cand { row: 2, ..moby(0, 50.0) }], 0.02);
		p.sample(&ROWS, &[([50.5, 0.0, 0.0], [1.0, 1.0, 1.0])], 2.5);
		assert_eq!((p.tracks[0].hue, p.tracks[0].rgb), (None, [0.5, 0.5, 1.0]), "a point row keeps its own colour");
		for _ in 0..30 {
			step(&mut t, &ROWS, &[moby(1, 0.0)], 0.02);
		}
		assert!((t.tracks[0].rgb[1] - 1.0).abs() < 1e-3 && t.tracks[0].rgb[0] < 1e-3, "eased to the hue: {:?}", t.tracks[0].rgb);
	}
}
