//! [PC] Fire, the Pyrocitor's stream, part 2 (stage 1 of the v0.5 fire extras): the wiring from the frame's pool sprites to the planner
//! (firefx_jet.rs) and the quads and ember emitters of the joined jets.
//!
//! `JetState::plan` runs once a frame inside `FireFx::begin_sprites`, after the sprites were classified: it collects the stream segments
//! (type-12 pool sprites that the particle loop would draw), joins them into jets and rewrites the picks:
//! - a segment that is part of a jet stays `Jet` (the loop only keeps its retail quad to give back, `add_sprite`); one that is not
//!   (a lone group, too many streams) becomes `Burst`, today's approved fireball;
//! - a sub-burst (`JetBurst`) or ember (`JetSpark`) is kept only while it is near a jet or near where one stood in the last 1.2 s
//!   (the same effect types are also used by things that are not fire, which stay retail);
//! - a burn puff (`Burn`) is kept only when its colour is warm or green (type 4 is also grey smoke).
//! `push` then adds a few ember emitters along the jet and, only with Fire volumes off (`fire_vol=0`) or for a jet over the volume
//! cap, draws each node as a camera-facing puff stretched along the stream (kind 3 of firefx.wgsl, `jet_at`); with Fire volumes on
//! (the default) the jet itself is drawn as volume slabs (firefx_jet_vol.rs). Everything follows the nodes of the planner, which are
//! the same from frame to frame (no strobe).

use super::firefx::{dist, tune, Emit};
use super::firefx_jet::{self, Jet, Quality, Seg};
use super::*;
use crate::fire_option::{self, FireKind};

/// The camera of the frame: eye, forward and up (rolled) axes, and the pool's far clamp (`Gpu::part_far`).
#[derive(Clone, Copy, Debug)]
pub(super) struct View {
	pub eye: [f32; 3],
	pub fwd: [f32; 3],
	pub up: [f32; 3],
	pub far: f32,
}

/// How long (s) the place of a jet is remembered for its sub-bursts and sparks, and how many places are remembered.
const MEMORY: f64 = 1.2;
const MAX_MEM: usize = 96;

/// One remembered place of a jet: where, how far from it a sub-burst still belongs to it, when it was last seen.
#[derive(Clone, Copy, Debug)]
struct Mem {
	pos: [f32; 3],
	reach: f32,
	seen: f64,
}

/// What the jets keep from frame to frame: the places they stood at lately.
#[derive(Default)]
pub(super) struct JetState {
	mem: Vec<Mem>,
	/// The nozzle points of the last `plan`'s jets: each stream keeps the end it stood at unless the other is clearly better
	/// (`firefx_jet::nozzle_from`), so two ends of equal rank do not swap from frame to frame.
	roots: Vec<[f32; 3]>,
	/// The noise seed of each of those jets (`seeds[i]` goes with `roots[i]`, review R7-8).
	seeds: Vec<f32>,
	/// When a jet was last seen (the fire clock's age): burn puffs are fire only for `fire_burn_after` s after (`burning`).
	last_jet: Option<f64>,
	/// Which burn puffs are a burning enemy and which a R.Y.N.O. trail (firefx_burn_trail.rs).
	trail: super::firefx_burn_trail::BurnTrail,
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
	[a[0] * k, a[1] * k, a[2] * k]
}

fn len(a: [f32; 3]) -> f32 {
	dot(a, a).sqrt()
}

/// `Jet::d0` of a stream whose nozzle end stands at `root`: 0 within `fire_jet_attach` m (2.5) of the hero (the stream at the gun),
/// else how much farther it is (a piece cut off from the stream reads as its continuation, and fades its core out smoothly as it flies
/// off); 0 with no hero.
pub(super) fn start_down(root: [f32; 3], hero: Option<[f32; 3]>) -> f32 {
	hero.map_or(0.0, |h| (len(sub(root, h)) - tune("fire_jet_attach", 2.5)).max(0.0))
}

/// The stream segments of this frame: the `Jet` picks the particle loop would draw (inside the pool's near / far range, not flat, not
/// invisible), with the size and the fade the loop gives them; and the sprite index of each.
fn segments(sprites: &[crate::prims::Sprite], picks: &[Option<(FireKind, u16)>], v: &View) -> (Vec<Seg>, Vec<usize>) {
	let (mut segs, mut of) = (Vec::new(), Vec::new());
	for (si, (p, sp)) in picks.iter().zip(sprites).enumerate() {
		let (Some((FireKind::Jet, _)), Some(b9), false) = (p, sp.pool, sp.flat) else { continue };
		let vz = dot(sub(sp.pos, v.eye), v.fwd);
		let (near, far) = ((b9 & 15) as f32 / 4.0, crate::prims::pool_far(b9, v.far));
		if vz <= near || vz >= far {
			continue;
		}
		let a = sp.colour[3] * (vz - near).min((far - vz) / 16.0).clamp(0.0, 1.0);
		if a <= 0.004 {
			continue;
		}
		segs.push(Seg { pos: sp.pos, size: sp.size * vz / (vz + 0.5), colour: [sp.colour[0], sp.colour[1], sp.colour[2], a] });
		of.push(si);
		if segs.len() >= firefx_jet::MAX_SEGS {
			break;
		}
	}
	(segs, of)
}

impl JetState {
	/// Joins this frame's stream segments into jets and rewrites the picks (see the module doc). `now` = the fire clock's age (s).
	/// Returns the jets to draw.
	pub(super) fn plan(&mut self, sprites: &[crate::prims::Sprite], picks: &mut [Option<(FireKind, u16)>], v: &View, hero: Option<[f32; 3]>, q: Quality, now: f64) -> Vec<Jet> {
		let (segs, of) = segments(sprites, picks, v);
		let wid = tune("fire_jet_wid", 1.0);
		let mut member = vec![false; picks.len()];
		let mut jets = Vec::new();
		for g in firefx_jet::groups(&segs, tune("fire_jet_link", 1.4)) {
			if jets.len() >= firefx_jet::MAX_JETS {
				break;
			}
			if let Some(j) = firefx_jet::build_with(&segs, &g, hero, q, wid, &self.roots) {
				for &m in &j.members {
					member[of[m]] = true;
				}
				jets.push(j);
			}
		}
		let chosen = seeds(&jets, &self.roots, &self.seeds);
		for (j, seed) in jets.iter_mut().zip(chosen) {
			j.seed = seed;
			j.d0 = start_down(j.root, hero);
		}
		self.roots = jets.iter().map(|j| j.root).collect();
		self.seeds = jets.iter().map(|j| j.seed).collect();
		self.remember(&jets, now);
		if !jets.is_empty() {
			self.last_jet = Some(now);
		}
		let burning = self.burning(now);
		for (si, (p, sp)) in picks.iter_mut().zip(sprites).enumerate() {
			*p = match *p {
				Some((FireKind::Jet, slot)) => Some((if member[si] { FireKind::Jet } else { FireKind::Burst }, slot)),
				Some((FireKind::JetBurst | FireKind::JetSpark, _)) if !self.near(sp.pos, now) => None,
				Some((FireKind::Burn, _)) if !burning || !fire_option::is_hot([sp.colour[0], sp.colour[1], sp.colour[2]]) => None,
				other => other,
			};
		}
		// The burn picks left: a R.Y.N.O. salvo's trails go back to retail, the burning enemies stay flames (firefx_burn_trail.rs).
		self.trail.filter(picks, |si| sprites[si].pos);
		jets
	}

	/// Forgets the places the jets stood at and their nozzles (a swapped world, firefx_gametime.rs; fire off).
	pub(super) fn forget(&mut self) {
		self.mem.clear();
		self.roots.clear();
		self.seeds.clear();
		self.last_jet = None;
		self.trail.forget();
	}

	/// Whether an enemy can be burning: a jet was seen within `fire_burn_after` s (12). The burn puffs' type is also the R.Y.N.O.
	/// rockets' trail (seen 2026-11-03, `fire_pick_log` on Eudora: orange type-4 puffs along the trails, no Pyrocitor), which stays retail
	/// (in the window too: firefx_burn_trail.rs). Dev knob `fire_burn_after` below 0: always (a test of the trails without a Pyrocitor).
	fn burning(&self, now: f64) -> bool {
		let after = tune("fire_burn_after", 12.0);
		after < 0.0 || self.last_jet.is_some_and(|t| (0.0..=after as f64).contains(&(now - t)))
	}

	/// Notes the places of this frame's jets (a node refreshes the entry within 0.3 m of it) and forgets the ones older than `MEMORY`.
	fn remember(&mut self, jets: &[Jet], now: f64) {
		self.mem.retain(|m| now - m.seen <= MEMORY);
		for n in jets.iter().flat_map(|j| j.nodes.iter()).filter(|n| n.alpha >= 0.05) {
			let fresh = Mem { pos: n.pos, reach: (2.0 * n.half).max(1.5), seen: now };
			if let Some(m) = self.mem.iter_mut().find(|m| dist(m.pos, n.pos) < 0.3) {
				*m = fresh;
			} else if self.mem.len() < MAX_MEM {
				self.mem.push(fresh);
			} else if let Some(old) = self.mem.iter_mut().min_by(|a, b| a.seen.total_cmp(&b.seen)) {
				*old = fresh;
			}
		}
	}

	/// Whether `pos` is within reach of a jet that stands now or stood in the last `MEMORY` seconds.
	fn near(&self, pos: [f32; 3], now: f64) -> bool {
		self.mem.iter().any(|m| now - m.seen <= MEMORY && dist(m.pos, pos) <= m.reach)
	}
}

/// The puff's screen axes at `pos` for a stream running along `tangent` (nozzle toward tip): `ax` along the stream (the part of the
/// tangent across the view), `ay` across it, and `sn` = how side-on the stream is (1 side-on, 0 end-on). End-on the axes ease to the
/// camera's up so the puff is round and does not turn with noise in the tangent.
fn axes(pos: [f32; 3], tangent: [f32; 3], v: &View) -> ([f32; 3], [f32; 3], f32) {
	let view = normalise(sub(pos, v.eye));
	let t = normalise(tangent);
	let t2 = sub(t, scale(view, dot(t, view)));
	let sn = len(t2);
	let up = normalise(sub(v.up, scale(view, dot(v.up, view))));
	let w = ((sn - 0.02) / 0.13).clamp(0.0, 1.0);
	let w = w * w * (3.0 - 2.0 * w);
	let along = if sn > 1e-6 { scale(t2, 1.0 / sn) } else { up };
	let mixed: [f32; 3] = std::array::from_fn(|k| up[k] + (along[k] - up[k]) * w);
	let ax = if len(mixed) > 1e-3 { normalise(mixed) } else { up };
	(ax, normalise(cross(view, ax)), sn)
}

/// The jet's puffs, 4 vertices each, in draw order (the far end first, so the blends stack back to front): one per node that shows.
/// Corners `(across, along)` = (-1,-1) (1,-1) (1,1) (-1,1), uv = (x + 1) / 2; `light` = (seed, s, half width, kind 3), `normal` =
/// (half length, jet length, 0): the shader needs the metres to keep the noise in world scale.
pub(super) fn puffs(jet: &Jet, seed: f32, v: &View) -> Vec<[Vertex; 4]> {
	let n = jet.nodes.len();
	let stretch = tune("fire_jet_stretch", 1.0);
	let mut out = Vec::with_capacity(n);
	for (j, nd) in jet.nodes.iter().enumerate() {
		if nd.alpha <= 0.004 {
			continue;
		}
		let (a, b) = (jet.nodes[j.saturating_sub(1)].pos, jet.nodes[(j + 1).min(n - 1)].pos);
		let (ax, ay, sn) = axes(nd.pos, sub(b, a), v);
		let (w, l) = (nd.half, nd.half + 0.6 * jet.step * sn * stretch);
		let corner = |cx: f32, cy: f32| Vertex {
			pos: std::array::from_fn(|k| nd.pos[k] + ay[k] * w * cx + ax[k] * l * cy),
			uv: [(cx + 1.0) * 0.5, (cy + 1.0) * 0.5],
			normal: [l, jet.len, 0.0],
			colour: [nd.tint[0], nd.tint[1], nd.tint[2], nd.alpha],
			light: [seed, nd.s, w, 3.0],
		};
		out.push([corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)]);
	}
	if let (Some(first), Some(last)) = (jet.nodes.first(), jet.nodes.last()) {
		if dot(sub(last.pos, first.pos), v.fwd) > 0.0 {
			out.reverse(); // the tip is the far end
		}
	}
	out
}

/// The k-th fresh noise seed (a jet with no remembered nozzle takes the first one not in use; one jet alone takes `jet_seed(0)`).
fn jet_seed(k: usize) -> f32 {
	(0.31 + 0.17 * k as f32).fract()
}

/// The noise seeds of this frame's jets (review R7-8). The streams come in the pool order of their segments, which changes as slots are
/// freed and reused, so a seed by place in that order would swap two streams' patterns in one frame; instead a jet whose nozzle stands
/// within reach (max(2 m, 0.6 x its length), as `nozzle_from`) of a nozzle of the last frame (`roots`, with `prev` = their seeds) keeps
/// that seed, nearest pairs first and each remembered seed to one jet only (a split stream: the part at the nozzle keeps it). The others
/// take the first fresh `jet_seed(k)` no jet of this frame has.
fn seeds(jets: &[Jet], roots: &[[f32; 3]], prev: &[f32]) -> Vec<f32> {
	let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
	for (i, j) in jets.iter().enumerate() {
		let reach = 2.0f32.max(0.6 * j.len);
		for (k, &r) in roots.iter().enumerate().take(prev.len()) {
			let d = len(sub(j.root, r));
			if d <= reach {
				pairs.push((d, i, k));
			}
		}
	}
	pairs.sort_by(|x, y| x.0.total_cmp(&y.0));
	let (mut out, mut used) = (vec![None; jets.len()], vec![false; prev.len()]);
	for (_, i, k) in pairs {
		if out[i].is_none() && !used[k] {
			out[i] = Some(prev[k]);
			used[k] = true;
		}
	}
	let mut fresh = (0..).map(jet_seed).filter(|s| !out.contains(&Some(*s)));
	out.iter().map(|o| o.unwrap_or_else(|| fresh.next().unwrap_or(0.0))).collect()
}

/// The ember emitters of a jet: `q.emitters` of them at 25 / 50 / 75 % of the way along (3), where the jet is dense enough to show.
pub(super) fn emitters(jet: &Jet, seed: f32, q: Quality) -> impl Iterator<Item = Emit> + '_ {
	let last = jet.nodes.len().saturating_sub(1);
	(0..q.emitters).filter_map(move |k| {
		let nd = jet.nodes.get(((k + 1) as f32 / (q.emitters + 1) as f32 * last as f32).round() as usize)?;
		(nd.alpha > 0.1).then(|| Emit { base: nd.pos, h: 3.0 * nd.half, w: 2.5 * nd.half, seed: (seed + 0.23 * (k + 1) as f32).fract() })
	})
}

/// Adds the frame's jets: their puffs to `quads` (until it holds `quad_cap` vertices) and their ember emitters to `big` (until `emit_cap`).
pub(super) fn push(jets: &[Jet], v: &View, q: Quality, quads: &mut Vec<Vertex>, big: &mut Vec<Emit>, quad_cap: usize, emit_cap: usize) {
	for jet in jets {
		let seed = jet.seed;
		for quad in puffs(jet, seed, v) {
			if quads.len() + 4 > quad_cap {
				return;
			}
			quads.extend(quad);
		}
		for e in emitters(jet, seed, q) {
			if big.len() < emit_cap {
				big.push(e);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::prims::Sprite;

	const Q: Quality = Quality { step: 0.4, nodes: 28, emitters: 3 };
	const HERO: Option<[f32; 3]> = Some([-1.0, 0.0, 1.0]);

	/// Camera 10 m south of the stream (which runs along +x at y = 0), looking north.
	fn side() -> View {
		View { eye: [2.0, -10.0, 1.0], fwd: [0.0, 1.0, 0.0], up: [0.0, 0.0, 1.0], far: 200.0 }
	}

	/// Camera behind the nozzle looking along the stream.
	fn behind() -> View {
		View { eye: [-8.0, 0.0, 1.0], fwd: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0], far: 200.0 }
	}

	fn sprite(x: f32, y: f32) -> Sprite {
		Sprite { pos: [x, y, 1.0], size: 1.0, rot: 0.0, colour: [1.0, 0.6, 0.2, 0.8], tex: 0, additive: true, pool: Some(0xF0), flat: false }
	}

	/// A stream of `n` segments 0.1 m apart along +x (picks: all `Jet`).
	fn stream(n: usize) -> (Vec<Sprite>, Vec<Option<(FireKind, u16)>>) {
		((0..n).map(|i| sprite(i as f32 * 0.1, 0.0)).collect(), (0..n).map(|i| Some((FireKind::Jet, i as u16))).collect())
	}

	fn a_jet() -> Jet {
		let (sprites, picks) = stream(40);
		let (segs, _) = segments(&sprites, &picks, &side());
		firefx_jet::build(&segs, &(0..segs.len()).collect::<Vec<_>>(), HERO, Q, 1.0).expect("a 4 m stream is a jet")
	}

	#[test]
	fn only_visible_pool_stream_segments_are_looked_at() {
		let (mut sprites, mut picks) = stream(6);
		sprites[1].flat = true; // drawn flat by the loop
		sprites[2].colour[3] = 0.001; // invisible
		sprites[3].pool = None; // not a pool sprite
		sprites[4].pos[1] = 400.0; // beyond the far clamp
		picks[5] = Some((FireKind::Burst, 5)); // another kind
		let (segs, of) = segments(&sprites, &picks, &side());
		assert_eq!(of, vec![0]);
		// The size and fade are the loop's: size x vz / (vz + 0.5), alpha x min(vz - near, (far - vz) / 16).
		assert!((segs[0].size - 10.0 / 10.5).abs() < 1e-4 && (segs[0].colour[3] - 0.8).abs() < 1e-5);
		sprites[0].pos[1] = 195.0; // 205 m from the eye: past the far clamp (200 m)
		assert!(segments(&sprites, &picks, &side()).0.is_empty());
		sprites[0].pos[1] = 180.0; // 190 m: inside, in the far fade (200 - 190) / 16
		let a = segments(&sprites, &picks, &side()).0[0].colour[3];
		assert!(a > 0.0 && a < 0.8, "the far fade: {a}");
	}

	#[test]
	fn a_stream_becomes_one_jet_and_its_segments_are_members() {
		let (sprites, mut picks) = stream(40);
		let mut st = JetState::default();
		let jets = st.plan(&sprites, &mut picks, &side(), HERO, Q, 10.0);
		assert_eq!(jets.len(), 1);
		assert!(picks.iter().all(|p| matches!(p, Some((FireKind::Jet, _)))), "members stay Jet");
	}

	#[test]
	fn lone_segments_fall_back_to_the_fireball() {
		let sprites = vec![sprite(0.0, 0.0), sprite(30.0, 0.0), sprite(60.0, 0.0)];
		let mut picks: Vec<_> = (0..3).map(|i| Some((FireKind::Jet, 10 + i as u16))).collect();
		let jets = JetState::default().plan(&sprites, &mut picks, &side(), HERO, Q, 10.0);
		assert!(jets.is_empty());
		assert_eq!(picks, vec![Some((FireKind::Burst, 10)), Some((FireKind::Burst, 11)), Some((FireKind::Burst, 12))], "the slot (noise seed) is kept");
	}

	#[test]
	fn a_second_stream_beyond_the_cap_falls_back_too() {
		let n = firefx_jet::MAX_JETS + 1;
		let mut sprites = Vec::new();
		for j in 0..n {
			sprites.extend((0..6).map(|i| sprite(i as f32 * 0.4, 10.0 * j as f32)));
		}
		let mut picks: Vec<_> = (0..sprites.len()).map(|i| Some((FireKind::Jet, i as u16))).collect();
		let jets = JetState::default().plan(&sprites, &mut picks, &side(), HERO, Q, 0.0);
		assert_eq!(jets.len(), firefx_jet::MAX_JETS);
		assert!(picks[..6 * firefx_jet::MAX_JETS].iter().all(|p| matches!(p, Some((FireKind::Jet, _)))));
		assert!(picks[6 * firefx_jet::MAX_JETS..].iter().all(|p| matches!(p, Some((FireKind::Burst, _)))), "the fifth stream draws as fireballs");
	}

	/// Sub-bursts and sparks belong to a jet: kept near one, and for 1.2 s near where one stood; far from any they are retail.
	#[test]
	fn sub_bursts_and_sparks_are_kept_only_near_a_jet() {
		let (mut sprites, mut picks) = stream(40);
		sprites.extend([sprite(2.0, 0.8), sprite(2.0, 9.0), sprite(2.5, 1.0)]);
		picks.extend([Some((FireKind::JetBurst, 100)), Some((FireKind::JetBurst, 101)), Some((FireKind::JetSpark, 102))]);
		let extra = picks.clone();
		let mut st = JetState::default();
		st.plan(&sprites, &mut picks, &side(), HERO, Q, 10.0);
		assert_eq!(picks[40], Some((FireKind::JetBurst, 100)), "0.8 m from the stream");
		assert_eq!(picks[41], None, "9 m away: retail");
		assert_eq!(picks[42], Some((FireKind::JetSpark, 102)));
		// The trigger is released: no stream segments any more, the sub-bursts of the dying jet still belong to it for 1.2 s.
		let (only, mut tail): (Vec<_>, Vec<_>) = (sprites[40..].to_vec(), extra[40..].to_vec());
		st.plan(&only, &mut tail, &side(), HERO, Q, 10.5);
		assert_eq!(tail, vec![Some((FireKind::JetBurst, 100)), None, Some((FireKind::JetSpark, 102))]);
		let mut tail: Vec<_> = extra[40..].to_vec();
		st.plan(&only, &mut tail, &side(), HERO, Q, 11.3);
		assert!(tail.iter().all(Option::is_none), "1.3 s later the memory is gone: retail");
		// A fresh state has no memory at all.
		let mut tail: Vec<_> = extra[40..].to_vec();
		JetState::default().plan(&only, &mut tail, &side(), HERO, Q, 10.5);
		assert!(tail.iter().all(Option::is_none));
	}

	#[test]
	fn burn_puffs_are_flames_only_while_warm_or_green() {
		// Four burning enemies 4 m apart, a 3 x 3 blob of puffs each (a lone puff is too few to tell from a R.Y.N.O. trail's first
		// ticks: firefx_burn_trail.rs), one colour each.
		let colours = [[1.0, 0.6, 0.15, 0.7], [0.4, 1.0, 0.3, 0.7], [0.5, 0.5, 0.5, 0.7], [0.1, 0.05, 0.0, 0.7]]; // orange, the upgraded gun's green, grey smoke of the same type, too dark
		let sprites: Vec<_> = (0..36)
			.map(|i| Sprite { colour: colours[i / 9], ..sprite((i / 9) as f32 * 4.0 + (i % 3) as f32 * 0.3, (i / 3 % 3) as f32 * 0.3) })
			.collect();
		let mut picks: Vec<_> = (0..36).map(|i| Some((FireKind::Burn, i as u16))).collect();
		let mut st = JetState { last_jet: Some(0.0), ..Default::default() };
		st.plan(&sprites, &mut picks, &side(), HERO, Q, 5.0);
		let want: Vec<_> = (0..36).map(|i| (i < 18).then_some((FireKind::Burn, i as u16))).collect();
		assert_eq!(picks, want);
		// No Pyrocitor lately (the R.Y.N.O.'s trail puffs share the type): retail.
		let mut picks: Vec<_> = (0..36).map(|i| Some((FireKind::Burn, i as u16))).collect();
		JetState::default().plan(&sprites, &mut picks, &side(), HERO, Q, 5.0);
		assert!(picks.iter().all(Option::is_none));
		st.plan(&sprites, &mut picks, &side(), HERO, Q, 30.0);
		assert!(picks.iter().all(Option::is_none), "12 s after the last jet");
	}

	#[test]
	fn the_memory_is_bounded() {
		let mut st = JetState::default();
		for k in 0..40 {
			// A jet sweeping 400 m in 40 frames, 20 distinct places a frame.
			let jet = Jet { nodes: (0..20).map(|i| firefx_jet::Node { pos: [k as f32 * 10.0 + i as f32, 0.0, 0.0], half: 0.3, alpha: 1.0, tint: [1.0; 3], d: 0.0, s: 0.0 }).collect(), len: 5.0, step: 0.4, members: vec![], root: [0.0; 3], seed: 0.0, d0: 0.0 };
			st.remember(&[jet], k as f64 * 0.01);
		}
		assert!(st.mem.len() <= MAX_MEM);
	}

	#[test]
	fn the_puffs_follow_the_nodes_and_are_drawn_far_end_first() {
		let jet = a_jet();
		let q = puffs(&jet, 0.3, &side());
		assert_eq!(q.len(), jet.nodes.iter().filter(|n| n.alpha > 0.004).count());
		let quad = &q[0];
		assert!(quad.iter().all(|v| v.light[3] == 3.0 && v.light[0] == 0.3));
		assert_eq!([quad[0].uv, quad[1].uv, quad[2].uv, quad[3].uv], [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
		// Side-on the camera sees the whole stream length: the half length is the half width plus 0.6 of a node step, and the quad
		// is centred on its node (the corners average to the node). Nozzle and tip are equally far: the nozzle's puff is drawn first.
		let (n0, v) = (&jet.nodes[0], &q[0]);
		let centre: [f32; 3] = std::array::from_fn(|k| v.iter().map(|c| c.pos[k]).sum::<f32>() / 4.0);
		assert!(dist(centre, n0.pos) < 1e-4, "the first quad drawn is the nozzle's");
		assert!((v[0].normal[0] - (n0.half + 0.6 * jet.step)).abs() < 0.02, "half length {}", v[0].normal[0]);
		assert_eq!(v[0].normal[1], jet.len);
		assert_eq!(v[0].light[1], n0.s);
		assert_eq!(v[0].colour[3], n0.alpha);
	}

	/// The tip nearer the camera than the nozzle: drawn last (the order flips with the view, not with the frame).
	#[test]
	fn the_nearer_end_is_drawn_last() {
		let jet = a_jet();
		let near_tip = View { eye: [20.0, 0.0, 1.0], fwd: [-1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0], far: 200.0 };
		let q = puffs(&jet, 0.3, &near_tip);
		assert_eq!(q.last().unwrap()[0].light[1], jet.nodes.last().unwrap().s, "tip last");
		let q = puffs(&jet, 0.3, &behind());
		assert_eq!(q.last().unwrap()[0].light[1], 0.0, "nozzle last when the camera stands behind it");
	}

	/// Seen end-on (camera behind the gun) the puffs are round: no thin line.
	#[test]
	fn seen_end_on_the_puffs_are_round() {
		let jet = a_jet();
		for quad in puffs(&jet, 0.3, &behind()) {
			let (w, l) = (quad[0].light[2], quad[0].normal[0]);
			assert!(l - w < 0.05 * jet.step + 1e-3, "half length {l} vs half width {w}");
		}
	}

	/// No strobe: a camera that moves a centimetre moves no corner by more than a few centimetres, also near the end-on blend.
	#[test]
	fn a_small_camera_move_moves_the_corners_little() {
		let jet = a_jet();
		for (a, b) in [(side(), View { eye: [2.01, -10.0, 1.0], ..side() }), (behind(), View { eye: [-8.0, 0.01, 1.0], ..behind() }), (behind(), View { eye: [-8.0, 0.0, 1.01], ..behind() })] {
			let (qa, qb) = (puffs(&jet, 0.3, &a), puffs(&jet, 0.3, &b));
			assert_eq!(qa.len(), qb.len());
			for (x, y) in qa.iter().zip(&qb) {
				for k in 0..4 {
					assert!(dist(x[k].pos, y[k].pos) < 0.15, "{:?} vs {:?}", x[k].pos, y[k].pos);
				}
			}
		}
	}

	#[test]
	fn ember_emitters_follow_the_quality() {
		let jet = a_jet();
		assert_eq!(emitters(&jet, 0.3, Q).count(), 3);
		assert!(emitters(&jet, 0.3, Quality { emitters: 0, ..Q }).next().is_none(), "Low has none");
		let e: Vec<Emit> = emitters(&jet, 0.3, Q).collect();
		assert!(e[0].base[0] < e[1].base[0] && e[1].base[0] < e[2].base[0], "25 / 50 / 75 % of the way out");
		assert!(e[0].seed != e[1].seed && e[1].seed != e[2].seed && e.iter().all(|e| (0.0..1.0).contains(&e.seed)));
		assert!(e.iter().all(|e| e.h > 0.0 && e.w > 0.0));
	}

	/// R7-8: a stream keeps its noise seed when the pool order of the streams changes, and when it splits the part at its nozzle keeps
	/// it while the broken-off part takes a fresh seed no other jet has.
	#[test]
	fn a_stream_keeps_its_seed_through_order_changes_and_splits() {
		let at = |x: f32| Jet { root: [x, 0.0, 0.0], ..a_jet() };
		let first = seeds(&[at(0.0), at(20.0)], &[], &[]);
		assert_eq!(first, vec![jet_seed(0), jet_seed(1)], "no memory: fresh seeds in order (one jet alone = jet_seed(0), as before)");
		let swapped = seeds(&[at(20.3), at(0.2)], &[[0.0; 3], [20.0, 0.0, 0.0]], &first);
		assert_eq!(swapped, vec![first[1], first[0]], "the other pool order: each stream keeps its own seed");
		let split = seeds(&[at(9.0), at(0.1), at(20.1)], &[[20.3, 0.0, 0.0], [0.2, 0.0, 0.0]], &swapped);
		assert_eq!((split[1], split[2]), (first[0], first[1]), "the nozzle parts keep their seeds");
		assert!(split[0] != first[0] && split[0] != first[1], "the broken-off part gets a fresh seed: {split:?}");
		let both_near = seeds(&[at(0.5), at(0.3)], &[[0.0; 3]], &[0.77]);
		assert_eq!(both_near[1], 0.77, "one remembered seed goes to the nearest jet only");
		assert_eq!(both_near[0], jet_seed(0));
	}

	#[test]
	fn push_respects_both_caps() {
		let jet = a_jet();
		let (mut quads, mut big) = (Vec::new(), Vec::new());
		push(&[jet.clone()], &side(), Q, &mut quads, &mut big, 1000, 128);
		assert_eq!(quads.len() % 4, 0);
		assert_eq!((quads.len() / 4, big.len()), (puffs(&jet, jet_seed(0), &side()).len(), 3));
		let (mut quads, mut big) = (Vec::new(), vec![Emit { base: [0.0; 3], h: 1.0, w: 1.0, seed: 0.0 }]);
		let n = puffs(&jet, jet_seed(0), &side()).len();
		push(&[jet.clone(), jet], &side(), Q, &mut quads, &mut big, n * 4 + 9, 2);
		assert_eq!(quads.len(), n * 4 + 8, "stops at the vertex cap, whole quads only");
		assert_eq!(big.len(), 2, "one emitter fits");
	}
}
