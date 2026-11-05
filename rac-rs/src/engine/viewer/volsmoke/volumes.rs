//! The CPU half of the volumetric smoke: the raw volumes the pool's smoke sprites become, the cull and importance order, the merge
//! into at most the quality's volume cap (a world grid of doubling cells with hysteresis, so a crowd of hundreds of puffs is a few
//! big billows and the picture does not pop when the count crosses the cap), and the GPU records (`Vol`).

use bytemuck::{Pod, Zeroable};
use std::collections::BTreeMap;

/// The most volumes the shader reads (the storage buffer's size).
pub const MAX_VOLS: usize = 64;

/// The merge grid ladder in metres; 0 = no merge. The cells nest (each is twice the one before), so the cluster count never rises
/// with a larger cell.
pub const CELLS: [f32; 6] = [0.0, 1.0, 2.0, 4.0, 8.0, 16.0];

/// One smoke puff as the sprite pool describes it, in world units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Raw {
	pub pos: [f32; 3],
	pub radius: f32,
	/// Thickness through the centre (the row's density times the sprite's fade): 1 = a puff that is about half see-through.
	pub thick: f32,
	pub albedo: [f32; 3],
	/// Share of the volume drawn (0..1): the trail band's (`plan::trail_share`) times the range fade (`range_weight`). The shader
	/// scales the density by it AFTER the clumping (review R18 M1: a thickness scaled down by it clumped back toward full strength,
	/// so a puff in a band overshot instead of cross-fading with its sprite).
	pub share: f32,
	/// 0 fresh .. 1 faded out (the sprite's alpha is low): more eroded.
	pub age: f32,
	pub rise: f32,
	pub billow: f32,
	/// Vertical half-extent over `radius` (1 = round, up to `MAX_TALL`): a column of smoke is a tall ellipsoid, not a ball.
	pub tall: f32,
}

/// The tallest a volume gets (vertical half-extent over the horizontal radius).
pub const MAX_TALL: f32 = 4.0;

/// The half-extent of a volume's bounding sphere (the larger of its radius and its vertical half-extent).
pub fn bound(r: &Raw) -> f32 {
	r.radius * r.tall.max(1.0)
}

/// The GPU record of one volume (`Vol` in the WGSL): 80 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vol {
	/// Centre, radius.
	pub c: [f32; 4],
	/// Albedo, extinction per world unit at the core.
	pub col: [f32; 4],
	/// Fire glow colour, share drawn (`Raw::share`).
	pub glow: [f32; 4],
	/// Rise speed (read on the CPU: flow.rs), billow, age, tall (vertical half-extent over the radius, >= 1).
	pub mo: [f32; 4],
	/// The fire light's centre and reach (`glow`): the shader fades the glow per sample with the distance from it.
	pub lp: [f32; 4],
}

/// An effect light as `partfx::FxLights` gives it: position, reach, colour (brightness folded in).
pub type Light = ([f32; 3], f32, [f32; 3]);

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
	(0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum()
}

/// The thickness drawn (thickness x share).
pub fn drawn(r: &Raw) -> f32 {
	r.thick * r.share
}

/// How much a volume matters on screen: drawn thickness times its (side-on) area, falling with the squared distance.
pub fn importance(r: &Raw, cam: [f32; 3]) -> f32 {
	drawn(r).max(0.05) * r.radius * bound(r) / (dist2(r.pos, cam) + 1.0)
}

/// Whether a puff is in front of / around the camera and inside `range`.
fn visible(r: &Raw, cam: [f32; 3], fwd: [f32; 3], range: f32) -> bool {
	let d = [r.pos[0] - cam[0], r.pos[1] - cam[1], r.pos[2] - cam[2]];
	let along = d[0] * fwd[0] + d[1] * fwd[1] + d[2] * fwd[2];
	along > -bound(r) && dist2(r.pos, cam).sqrt() - bound(r) <= range && drawn(r) > 0.0 && r.radius > 0.0
}

/// Keeps the puffs in front of / around the camera and inside `range`.
#[cfg(test)]
pub fn cull(raw: &[Raw], cam: [f32; 3], fwd: [f32; 3], range: f32) -> Vec<Raw> {
	raw.iter().filter(|r| visible(r, cam, fwd, range)).copied().collect()
}

fn cell_of(p: [f32; 3], cell: f32) -> (i32, i32, i32) {
	((p[0] / cell).floor() as i32, (p[1] / cell).floor() as i32, (p[2] / cell).floor() as i32)
}

/// How many clusters a grid of `cell` metres makes.
pub fn cluster_count(raw: &[Raw], cell: f32) -> usize {
	if cell <= 0.0 {
		return raw.len();
	}
	let mut s: Vec<(i32, i32, i32)> = raw.iter().map(|r| cell_of(r.pos, cell)).collect();
	s.sort_unstable();
	s.dedup();
	s.len()
}

/// The smallest ladder step whose cluster count fits `cap` (the last step when none does: `reduce` then keeps the most important).
pub fn want_step(raw: &[Raw], cap: usize) -> usize {
	(0..CELLS.len()).find(|&s| cluster_count(raw, CELLS[s]) <= cap).unwrap_or(CELLS.len() - 1)
}

/// The merge step in force: up at once, down one step after `hold` frames that all asked for less.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ladder {
	pub step: usize,
	below: u32,
}

impl Ladder {
	pub fn update(&mut self, want: usize, hold: u32) -> usize {
		if want >= self.step {
			self.step = want;
			self.below = 0;
		} else {
			self.below += 1;
			if self.below >= hold {
				self.step -= 1;
				self.below = 0;
			}
		}
		self.step
	}
}

/// Merges one cluster into one volume: the centroid weighted by optical mass (thickness x volume), a horizontal radius that covers all
/// members (at most 3x the biggest) and a vertical half-extent that covers them too (a stack of puffs = a tall column, at most
/// `MAX_TALL` x the radius), the mass kept (thickness = mass / volume, at most 4x the thickest member). The share is the drawn mass
/// over the mass (thickness x share = the drawn mass / volume); centroid, albedo, age, rise and billow are weighted by the drawn mass.
/// The envelope (radius, height) takes each member's reach from the centroid by its share below a knee, a quarter of the cluster's
/// largest share (review R29 M1, as the shader's shape): a puff joining at share ~0 does not grow the volume at once, and members
/// that fade together (all at one share) keep the whole envelope (bit for bit as without the knee).
fn merge_group(g: &[Raw]) -> Raw {
	let mass: Vec<f32> = g.iter().map(|r| r.thick * r.radius.powi(3) * r.tall.max(1.0)).collect();
	let msum: f32 = mass.iter().sum();
	let dsum: f32 = g.iter().zip(&mass).map(|(r, m)| m * r.share).sum();
	if g.len() == 1 || msum <= 0.0 || dsum <= 0.0 {
		return g[0];
	}
	let w = |k: usize| mass[k] * g[k].share / dsum;
	let mut c = [0.0f32; 3];
	let (mut albedo, mut age, mut rise, mut billow) = ([0.0f32; 3], 0.0f32, 0.0f32, 0.0f32);
	for (k, r) in g.iter().enumerate() {
		for a in 0..3 {
			c[a] += r.pos[a] * w(k);
			albedo[a] += r.albedo[a] * w(k);
		}
		(age, rise, billow) = (age + r.age * w(k), rise + r.rise * w(k), billow + r.billow * w(k));
	}
	// dsum > 0, so some share is > 0.
	let smax = g.iter().map(|r| r.share).fold(0.0, f32::max);
	let knee = |r: &Raw| (4.0 * r.share / smax).min(1.0);
	let rmax = g.iter().map(|r| knee(r) * r.radius).fold(0.0, f32::max);
	let cover = g.iter().map(|r| knee(r) * (((r.pos[0] - c[0]).powi(2) + (r.pos[1] - c[1]).powi(2)).sqrt() + r.radius)).fold(0.0, f32::max);
	let radius = cover.min(3.0 * rmax).max(rmax);
	let up = g.iter().map(|r| knee(r) * ((r.pos[2] - c[2]).abs() + bound(r))).fold(0.0, f32::max);
	let tall = (up / radius).clamp(1.0, MAX_TALL);
	let tmax = g.iter().map(|r| r.thick).fold(0.0, f32::max);
	let thick = (msum / (radius.powi(3) * tall)).min(4.0 * tmax);
	Raw { pos: c, radius, thick, albedo, share: (dsum / msum).min(1.0), age, rise, billow, tall }
}

/// One output volume and the input puffs (indices into the input list) it stands for.
pub struct Cluster {
	pub vol: Raw,
	pub members: Vec<usize>,
}

/// The puffs `ix` of `raw` merged on a grid of `cell` metres (0 = untouched), each cluster with its members (in input order).
fn clusters(raw: &[Raw], ix: &[usize], cell: f32) -> Vec<Cluster> {
	if cell <= 0.0 {
		return ix.iter().map(|&i| Cluster { vol: raw[i], members: vec![i] }).collect();
	}
	let mut groups: BTreeMap<(i32, i32, i32), Vec<usize>> = BTreeMap::new();
	for &i in ix {
		groups.entry(cell_of(raw[i].pos, cell)).or_default().push(i);
	}
	groups
		.into_values()
		.map(|members| {
			let g: Vec<Raw> = members.iter().map(|&i| raw[i]).collect();
			Cluster { vol: merge_group(&g), members }
		})
		.collect()
}

/// The puffs merged on a grid of `cell` metres (0 = untouched).
#[cfg(test)]
pub fn merge(raw: &[Raw], cell: f32) -> Vec<Raw> {
	clusters(raw, &(0..raw.len()).collect::<Vec<_>>(), cell).into_iter().map(|c| c.vol).collect()
}

/// What the reduction remembers between frames: the ladder step and last frame's drawn volumes (each with whether it stood for a merged
/// cluster), so a merged volume eases to its new place instead of jumping (A8 / A23).
#[derive(Default)]
pub struct Track {
	pub ladder: Ladder,
	last: Vec<(Raw, bool)>,
}

impl Track {
	/// The volumes stopped drawing: the next frame starts fresh (the ladder keeps its step).
	pub fn forget(&mut self) {
		self.last.clear();
	}
}

/// Share of the way a merged volume moves to its new centre, radius and thickness per frame.
const EASE: f32 = 0.5;

/// The last tenth of the range over which a volume fades out (its near edge at the range = gone). Over that band its puffs' sprites
/// fade in as it fades out (`Reduced::band`, review R7-2), so the smoke hands over to retail's sprites without a gap.
const FADE_BAND: f32 = 0.1;

/// Each volume (most important first) finds the nearest unclaimed volume of last frame within the larger of the two radii; when either
/// was a merged cluster it eases from that volume's centre, radius, thickness and share. (The noise is world anchored, shader.rs: a
/// volume has no noise of its own to carry over any more.)
pub fn follow(out: &mut [Cluster], last: &[(Raw, bool)]) {
	let mut claimed = vec![false; last.len()];
	for c in out.iter_mut() {
		let v = &mut c.vol;
		let near = last.iter().enumerate().filter(|(k, (p, _))| !claimed[*k] && dist2(p.pos, v.pos) <= bound(p).max(bound(v)).powi(2));
		let Some((k, &(p, merged))) = near.min_by(|a, b| dist2(a.1 .0.pos, v.pos).total_cmp(&dist2(b.1 .0.pos, v.pos))) else { continue };
		claimed[k] = true;
		if merged || c.members.len() > 1 {
			let mix = |a: f32, b: f32| a + (b - a) * EASE;
			v.pos = std::array::from_fn(|i| mix(p.pos[i], v.pos[i]));
			(v.radius, v.thick, v.tall, v.share) = (mix(p.radius, v.radius), mix(p.thick, v.thick), mix(p.tall, v.tall), mix(p.share, v.share));
		}
	}
}

/// A volume's share at the range: 1 inside, falling to 0 over the last tenth of `range` (measured at its near edge).
pub fn range_weight(v: &Raw, cam: [f32; 3], range: f32) -> f32 {
	let band = (range * FADE_BAND).max(1e-3);
	((range - (dist2(v.pos, cam).sqrt() - bound(v))) / band).clamp(0.0, 1.0)
}

/// The frame's reduction.
pub struct Reduced {
	/// The volumes to draw, most important first.
	pub vols: Vec<Raw>,
	/// The input puffs no volume shows (a cluster the cap cut, or out of view / beyond the range): their retail sprites / quads must
	/// be drawn after all.
	pub cut: Vec<usize>,
	/// The input puffs of the volumes fading out at the range, with the volume's share (0..1): their sprites come back at the rest.
	pub band: Vec<(usize, f32)>,
}

/// The frame's volumes: culled, merged to fit `cap` (hysteresis in `track.ladder`), the most important `cap` kept, most important first,
/// merged shapes followed over from last frame (`follow`), faded out at the range (share x `range_weight`, `Reduced::band`). `view` = (eye, forward, range).
pub fn reduce(raw: &[Raw], view: ([f32; 3], [f32; 3], f32), cap: usize, hold: u32, track: &mut Track) -> Reduced {
	let (cam, fwd, range) = view;
	let cap = cap.clamp(1, MAX_VOLS);
	let seen: Vec<usize> = (0..raw.len()).filter(|&i| visible(&raw[i], cam, fwd, range)).collect();
	let kept: Vec<Raw> = seen.iter().map(|&i| raw[i]).collect();
	let step = track.ladder.update(want_step(&kept, cap), hold);
	let mut out = clusters(raw, &seen, CELLS[step]);
	out.sort_by(|a, b| importance(&b.vol, cam).total_cmp(&importance(&a.vol, cam)).then(a.members[0].cmp(&b.members[0])));
	let mut cut: Vec<usize> = out.split_off(cap.min(out.len())).into_iter().flat_map(|c| c.members).collect();
	// (R7-2: a held puff no volume shows must not vanish either.)
	let mut shown = vec![false; raw.len()];
	seen.iter().for_each(|&i| shown[i] = true);
	cut.extend((0..raw.len()).filter(|&i| !shown[i]));
	follow(&mut out, &track.last);
	track.last = out.iter().map(|c| (c.vol, c.members.len() > 1)).collect();
	let mut band = Vec::new();
	let vols = out
		.into_iter()
		.map(|c| {
			let w = range_weight(&c.vol, cam, range);
			if w < 1.0 {
				band.extend(c.members.iter().map(|&m| (m, w)));
			}
			Raw { share: c.vol.share * w, ..c.vol }
		})
		.collect();
	Reduced { vols, cut, band }
}

/// The warm light fire throws on a volume at `pos` with `radius`, for the shader to fade per sample: the summed colour of the lights
/// whose reach touches the volume (capped) and their brightness-weighted centre and reach (reach 0 = no fire near). Light on smoke
/// falls off with the distance from the fire, not from the volume's centre, so a big merged volume glows only on its side near the
/// fire (no evenly orange ball or ellipse in the sky, whatever its size).
pub fn glow(pos: [f32; 3], radius: f32, lights: &[Light]) -> ([f32; 3], [f32; 4]) {
	let (mut g, mut c, mut reach, mut wsum) = ([0.0f32; 3], [0.0f32; 3], 0.0f32, 0.0f32);
	for (lp, r, rgb) in lights {
		if *r <= 0.0 || dist2(pos, *lp).sqrt() >= r + radius {
			continue;
		}
		let w = (0.3 * rgb[0] + 0.59 * rgb[1] + 0.11 * rgb[2]).max(1e-4);
		for a in 0..3 {
			g[a] += rgb[a];
			c[a] += lp[a] * w;
		}
		(reach, wsum) = (reach + r * w, wsum + w);
	}
	if wsum <= 0.0 {
		return ([0.0; 3], [0.0; 4]);
	}
	// Capped as a whole, so the hue stays (a cap per channel turned several orange fire lights' sum yellow-green: R and G both at the
	// cap; user 2026-11-04, the fire lights of every fire with Global illumination on).
	let k = 2.0 / g[0].max(g[1]).max(g[2]).max(2.0);
	(g.map(|v| v * k), [c[0] / wsum, c[1] / wsum, c[2] / wsum, reach / wsum])
}

/// The shader's glow at point `p` (mirror of the WGSL in `fs_smoke`, for the tests): the colour times (1 - distance / reach) squared.
#[cfg(test)]
pub fn glow_at(g: ([f32; 3], [f32; 4]), p: [f32; 3]) -> [f32; 3] {
	let k = if g.1[3] > 0.0 { (1.0 - dist2(p, [g.1[0], g.1[1], g.1[2]]).sqrt() / g.1[3]).clamp(0.0, 1.0) } else { 0.0 };
	g.0.map(|v| v * k * k)
}

/// The GPU records of the volumes.
pub fn to_vols(raw: &[Raw], lights: &[Light]) -> Vec<Vol> {
	raw.iter()
		.map(|r| {
			let (g, lp) = glow(r.pos, bound(r), lights);
			Vol {
				c: [r.pos[0], r.pos[1], r.pos[2], r.radius],
				col: [r.albedo[0], r.albedo[1], r.albedo[2], r.thick * 1.5 / r.radius],
				glow: [g[0], g[1], g[2], r.share.clamp(0.0, 1.0)],
				mo: [r.rise, r.billow, r.age, r.tall.clamp(1.0, MAX_TALL)],
				lp,
			}
		})
		.collect()
}
