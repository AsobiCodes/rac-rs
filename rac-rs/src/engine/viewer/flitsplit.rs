//! [PC] fire light v2 stage 6 (lane dlit3, 2026-11-06, FIRE_LIGHT_V2.md 3): tongue-cluster shadow ORIGINS. Stage 5 drew one moment
//! cube per fire (flitlist.rs `groups`) from its emitter centroid; for a big wreck fire the centroid sits behind / inside the hull, sees
//! ground the real flames cannot and misses the foreground the near-side flames light (stage 3 reference: rel 0.64 at the crash site).
//! Here a big fire is split into up to `K_MAX` sub-emitters: its lights (flitlist.rs clusters of tongues) are partitioned by k-means
//! round persistent centres (`Split`: matched to the fire from frame to frame, eased over `TAU` s, a new centre is seeded ON its parent
//! and slides out along the parent's major axis, so a split never pops), each centre is a shadow origin (a `Group` for
//! sunshadow/omni.rs: its own emitter radius from its lights, its share of the fire's power, its own moment cube) and every light
//! reads the map of its nearest centre, blended with the second nearest's over `BAND` m round their bisector (`Light::alt`: no hard
//! switch when a light crosses it). Lights keep their own form factor, colour and flicker (flit.wgsl): the fire is the sum of its
//! sub-emitters. A merged-away centre (fewer origins wanted) stays where it was and hands its lights and its share of the fire's
//! mean over to the others over `GONE_S` (review S3: no pop); its map fades out drawn. Origins per fire (`want`, `alloc`): one per `SPLIT_W` m of the fire's span, at most `K_MAX`, out of the map budget
//! (omni.rs `lights`: High 4, Medium 1): every near fire keeps one, the extra maps go to the strongest near fires first; a small or far
//! fire keeps one origin. Low / Off / Classic / no map: nothing changes (the fires as `groups` made them).
//! Knobs (RAC_TUNE): flsh_k (most origins a fire, 1 = stage 5's one map a fire), flsh_split (m of span per origin), flsh_band (m),
//! flsh_snap (default 1: each origin on its cluster's tongue point nearest the centroid; 0 = the centroid itself; worker 9 measured
//! it best at td, ng60, rtn and crash120).

use super::flitlist::{Group, Light};
use crate::devtune;

/// Most origins (shadow maps) one fire may take.
pub(super) const K_MAX: usize = 4;
/// A fire wants one origin per this many metres of span (its lights' farthest distance from its centre, x 2).
const SPLIT_W: f32 = 3.0;
/// Metres round the bisector of a light's two nearest centres over which it blends their maps.
const BAND: f32 = 1.5;
/// Seconds a centre takes to follow its lights (exponential).
const TAU: f32 = 0.2;
/// A fire of the last frame is this fire when its centre lies within this distance (m; half flitlist.rs GROUP).
const MATCH: f32 = 3.0;
/// A new centre starts this far (m) from its parent along the parent's major axis (then slides out).
const SEED: f32 = 0.05;
/// A fire farther from the eye than this takes no extra origin (omni.rs NEAR_EYE).
const NEAR_EYE: f32 = 30.0;
/// A centre's emitter radius = sqrt(BALL x its tongues' spread^2 + their mean half size^2) (flitlist.rs `BALL`).
const BALL: f32 = 5.0 / 3.0;

/// Seconds a merged-away centre takes to hand its lights and share over to the surviving ones (review S3; omni.rs FADE_S, so its map
/// fades out drawn over the same time).
const GONE_S: f32 = 0.35;

/// The centres of the fires of the last frame, this frame's tongues (`tongues`, filled by firefx.rs before `step`) and origins.
#[derive(Default)]
pub(super) struct Split {
	fires: Vec<Fire>,
	last: Option<f64>,
	/// This frame's flame tongues (firefx.rs `shadow_origins` fills it, only when `splits`).
	pub(super) tongues: Vec<Tongue>,
	/// `step`'s answer and its scratch, kept between frames (review N6: no allocation per fire per frame once warm).
	out: Vec<Group>,
	sc: Scratch,
}

#[derive(Default)]
struct Scratch {
	pts: Vec<Vec<([f32; 3], f32)>>,
	half: Vec<Vec<f32>>,
	fl: Vec<usize>,
	prev: Vec<Option<usize>>,
	wish: Vec<usize>,
	score: Vec<f32>,
	near: Vec<bool>,
	ks: Vec<usize>,
	order: Vec<usize>,
	next: Vec<Fire>,
	base: Vec<usize>,
	acc: Vec<(f32, f32, f32)>,
	fire_w: Vec<f32>,
	target: Vec<[f32; 3]>,
	lacc: Vec<([f32; 3], f32)>,
	power: Vec<f32>,
}

#[derive(Clone, Debug, Default)]
struct Fire {
	pos: [f32; 3],
	c: Vec<[f32; 3]>,
	/// The emitter radius of each centre of `c` last frame (a merged-away one keeps it while it fades).
	size: Vec<f32>,
	/// Centres merged away (review S3): they stand where they were while their weight `Gone::w` falls 1 -> 0 over GONE_S.
	gone: Vec<Gone>,
	seen: bool,
}

#[derive(Clone, Copy, Debug)]
struct Gone {
	pos: [f32; 3],
	size: f32,
	w: f32,
}

fn d2(a: [f32; 3], b: [f32; 3]) -> f32 {
	(0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum()
}

/// The origins a fire of span `span` m (lights' farthest distance from its centre) with `lights` lights wants, held at the current
/// `cur` while the wish lies within 0.75 of it (no flip-flop at a boundary); 1..=min(k_max, lights).
pub(super) fn want(span: f32, lights: usize, cur: usize, split_w: f32, k_max: usize) -> usize {
	let x = 1.0 + 2.0 * span / split_w.max(0.5);
	let k = if cur == 0 || (x - cur as f32).abs() > 0.75 { x.round() as usize } else { cur };
	k.clamp(1, k_max.clamp(1, K_MAX).min(lights.max(1)))
}

/// The origins per fire out of `n` maps: every `near` fire one (the strongest first while maps last), then the extra ones to the
/// strongest near fires up to their `wish`; the others one (they only fade out a map omni.rs still holds).
#[cfg(test)]
pub(super) fn alloc(wish: &[usize], score: &[f32], near: &[bool], n: usize) -> Vec<usize> {
	let (mut k, mut order) = (Vec::new(), Vec::new());
	alloc_into(wish, score, near, n, &mut k, &mut order);
	k
}

/// `alloc` into `k` (`order`: scratch).
fn alloc_into(wish: &[usize], score: &[f32], near: &[bool], n: usize, k: &mut Vec<usize>, order: &mut Vec<usize>) {
	k.clear();
	k.resize(wish.len(), 1);
	order.clear();
	order.extend((0..wish.len()).filter(|&i| near[i]));
	order.sort_by(|&a, &b| score[b].total_cmp(&score[a]).then(a.cmp(&b)));
	let mut left = n.saturating_sub(order.len());
	for &i in order.iter() {
		let extra = wish[i].saturating_sub(1).min(left);
		k[i] += extra;
		left -= extra;
	}
}

/// The nearest centre of `c` to `p` and the second nearest (= the nearest when there is one), with their distances.
fn nearest2(c: &[[f32; 3]], p: [f32; 3]) -> (usize, usize, f32, f32) {
	let (mut a, mut b, mut da, mut db) = (0usize, 0usize, f32::MAX, f32::MAX);
	for (i, &ci) in c.iter().enumerate() {
		let d = d2(ci, p);
		if d < da {
			(b, db) = (a, da);
			(a, da) = (i, d);
		} else if d < db {
			(b, db) = (i, d);
		}
	}
	if c.len() < 2 {
		(b, db) = (a, da);
	}
	(a, b, da.sqrt(), db.sqrt())
}

/// Lloyd's k-means, `rounds` rounds from `c` over the weighted points `ls` (pos, weight) into `out` (`acc`: scratch): a centre that
/// takes no point stays.
fn lloyd(c: &[[f32; 3]], ls: &[([f32; 3], f32)], rounds: usize, out: &mut Vec<[f32; 3]>, acc: &mut Vec<([f32; 3], f32)>) {
	out.clear();
	out.extend_from_slice(c);
	for _ in 0..rounds {
		acc.clear();
		acc.resize(out.len(), ([0.0f32; 3], 0.0f32));
		for &(p, w) in ls {
			let j = nearest2(out, p).0;
			(0..3).for_each(|k| acc[j].0[k] += p[k] * w);
			acc[j].1 += w;
		}
		for (ci, a) in out.iter_mut().zip(acc.iter()) {
			if a.1 > 0.0 {
				*ci = a.0.map(|v| v / a.1);
			}
		}
	}
}

/// The power-weighted major axis of the points of `ls` nearest centre `j` of `c` (unit; x when they are one point).
fn axis(c: &[[f32; 3]], j: usize, ls: &[([f32; 3], f32)]) -> [f32; 3] {
	let mut m = [[0.0f32; 3]; 3];
	for &(p, w) in ls.iter().filter(|(p, _)| nearest2(c, *p).0 == j) {
		let v = [p[0] - c[j][0], p[1] - c[j][1], p[2] - c[j][2]];
		(0..3).for_each(|r| (0..3).for_each(|s| m[r][s] += w * v[r] * v[s]));
	}
	let mut a = [1.0f32, 0.3, 0.1];
	for _ in 0..12 {
		let b = [0, 1, 2].map(|r| m[r][0] * a[0] + m[r][1] * a[1] + m[r][2] * a[2]);
		let l = (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt();
		if l < 1e-9 {
			return [1.0, 0.0, 0.0];
		}
		a = b.map(|v| v / l);
	}
	a
}

/// The power of each centre of `c` over `ls` into `p`.
fn powers(c: &[[f32; 3]], ls: &[([f32; 3], f32)], p: &mut Vec<f32>) {
	p.clear();
	p.resize(c.len(), 0.0);
	ls.iter().for_each(|&(q, w)| p[nearest2(c, q).0] += w);
}

/// Fire `f` with `k` centres: the weakest go to `f.gone` (review S3: they keep their map while their lights move over; no pop), new
/// ones are seeded `SEED` m off the strongest centre along its major axis, the parent moved as far the other way. `settle` (a new
/// fire, nothing to slide from): k-means after each split.
fn resize(f: &mut Fire, k: usize, ls: &[([f32; 3], f32)], settle: bool, p: &mut Vec<f32>) {
	while f.c.len() > k {
		powers(&f.c, ls, p);
		let j = (0..f.c.len()).min_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap_or(0);
		let size = if j < f.size.len() { f.size.remove(j) } else { super::flitlist::RS_LO };
		f.gone.push(Gone { pos: f.c.remove(j), size, w: 1.0 });
	}
	while f.c.len() < k && !f.c.is_empty() {
		powers(&f.c, ls, p);
		let j = (0..f.c.len()).max_by(|&a, &b| p[a].total_cmp(&p[b]).then(b.cmp(&a))).unwrap_or(0);
		let ax = axis(&f.c, j, ls);
		let o = f.c[j];
		f.c[j] = [0, 1, 2].map(|i| o[i] - SEED * ax[i]);
		f.c.push([0, 1, 2].map(|i| o[i] + SEED * ax[i]));
		if settle {
			let mut t = Vec::new();
			lloyd(&f.c, ls, 4, &mut t, &mut Vec::new());
			f.c = t;
		}
	}
}

/// A light at `p` of a fire with the centres `c` and the merged-away `gone`: the map it reads (index in `c`, then in `gone`) and the
/// second one with its share (<= 0.5). The nearest centre blends with the next one over `band` m round their bisector; the nearest
/// merged-away centre takes its weight x the same bisector blend against the nearest centre out of them; the two largest shares stay.
fn pick(c: &[[f32; 3]], gone: &[Gone], p: [f32; 3], band: f32) -> (usize, Option<(usize, f32)>) {
	let (a, b, da, db) = nearest2(c, p);
	let w2 = if a == b || band <= 0.0 { 0.0 } else { 0.5 * (1.0 - smooth((db - da) / band)) };
	let g = gone.iter().enumerate().map(|(i, x)| (i, d2(x.pos, p).sqrt())).min_by(|x, y| x.1.total_cmp(&y.1));
	let gw = g.map_or(0.0, |(i, dg)| {
		let x = if band > 0.0 { (da - dg) / band } else if dg < da { 1.0 } else { -1.0 };
		gone[i].w * (0.5 + 0.5 * x.signum() * smooth(x.abs()))
	});
	let mut s = [(a, (1.0 - w2) * (1.0 - gw)), (b, w2 * (1.0 - gw)), (c.len() + g.map_or(0, |x| x.0), gw)];
	s.sort_by(|x, y| y.1.total_cmp(&x.1));
	let w = s[1].1 / (s[0].1 + s[1].1).max(1e-9);
	(s[0].0, (w > 0.01 && s[1].0 != s[0].0).then_some((s[1].0, w)))
}

impl Split {
	/// Whether fires split this frame with `n` maps: not below 2 maps, with flsh_k=1 or the shadow reference (flsh_ref).
	pub(super) fn splits(n: usize) -> bool {
		n >= 2 && devtune::f32("flsh_k", K_MAX as f32) >= 2.0 && devtune::f32("flsh_ref", 0.0) <= 0.5
	}

	/// One frame at game time `t` (s): the shadow origins of `groups` (the fires of `list`, flitlist.rs `groups`) with `n` maps this
	/// frame (0: none drawn), from `self.tongues`: returns the sub-emitters (as `Group`s, omni.rs maps them; the centres of each fire,
	/// then its merged-away ones) and points each fire light of `list` at its nearest one (`Light::group`) and the second with its
	/// blend weight (`Light::alt`). Not `splits`: the fires unchanged (one origin each) and the centres dropped.
	#[cfg(test)]
	pub(super) fn step(&mut self, list: &mut [Light], groups: &[Group], eye: [f32; 3], n: usize, t: f64) -> &[Group] {
		self.step_mut(list, groups, eye, n, t)
	}

	/// `step`, the origins mutable (firefx.rs `shadow_origins` moves them with the flames' dance; the centres it keeps are not moved).
	pub(super) fn step_mut(&mut self, list: &mut [Light], groups: &[Group], eye: [f32; 3], n: usize, t: f64) -> &mut [Group] {
		self.out.clear();
		if !Self::splits(n) {
			self.fires.clear();
			self.last = None;
			self.out.extend(groups.iter().enumerate().map(|(i, g)| Group { fire: i as u8, share: 1.0, ..*g }));
			return &mut self.out;
		}
		let k_max = devtune::f32("flsh_k", K_MAX as f32).clamp(1.0, K_MAX as f32) as usize;
		let dt = self.last.map_or(0.0, |l| (t - l).clamp(0.0, 0.1) as f32);
		self.last = Some(t);
		let (split_w, band) = (devtune::f32("flsh_split", SPLIT_W), devtune::f32("flsh_band", BAND).max(0.0));
		let snap = devtune::f32("flsh_snap", 1.0) > 0.5;
		let mut sc = std::mem::take(&mut self.sc);
		points(list, groups.len(), &self.tongues, &mut sc);
		self.plan(groups, &mut sc, eye, n, (split_w, k_max));
		self.centres(groups, &mut sc, dt, snap);
		self.assign(list, groups, &mut sc, band);
		std::mem::swap(&mut self.fires, &mut sc.next);
		self.sc = sc;
		&mut self.out
	}

	/// Per fire: its last-frame fire (`sc.prev`), its wish, its score and near flag, then the origins it gets (`sc.ks`).
	fn plan(&mut self, groups: &[Group], sc: &mut Scratch, eye: [f32; 3], n: usize, (split_w, k_max): (f32, usize)) {
		self.fires.iter_mut().for_each(|f| f.seen = false);
		sc.prev.clear();
		for g in groups {
			let m = (0..self.fires.len()).filter(|&i| !self.fires[i].seen && d2(self.fires[i].pos, g.pos) < MATCH * MATCH).min_by(|&a, &b| d2(self.fires[a].pos, g.pos).total_cmp(&d2(self.fires[b].pos, g.pos)));
			if let Some(i) = m {
				self.fires[i].seen = true;
			}
			sc.prev.push(m);
		}
		sc.wish.clear();
		for (g, gr) in groups.iter().enumerate() {
			let span = sc.pts[g].iter().map(|p| d2(p.0, gr.pos).sqrt()).fold(0.0f32, f32::max);
			sc.wish.push(want(span, sc.pts[g].len(), sc.prev[g].map_or(0, |i| self.fires[i].c.len()), split_w, k_max));
		}
		sc.score.clear();
		sc.score.extend(groups.iter().map(|g| g.power / (1.0 + d2(g.pos, eye) / 64.0)));
		sc.near.clear();
		// Lane rfire worker 9 (review M1): a fire of small flames only takes no map (sunshadow/omni.rs `candidates`), so it is not near
		// here either: it no longer takes an extra origin's map from a big fire.
		sc.near.extend(groups.iter().map(|g| d2(g.pos, eye).sqrt() < NEAR_EYE.min(g.reach + 10.0) && !g.small));
		alloc_into(&sc.wish, &sc.score, &sc.near, n, &mut sc.ks, &mut sc.order);
	}

	/// Per fire: its centres resized to `sc.ks`, eased (TAU) towards their k-means targets (snapped onto a tongue with flsh_snap),
	/// its merged-away centres' weights stepped down by `dt` (out at 0); into `sc.next`.
	fn centres(&mut self, groups: &[Group], sc: &mut Scratch, dt: f32, snap: bool) {
		let ease = 1.0 - (-dt / TAU).exp();
		sc.next.clear();
		for (g, gr) in groups.iter().enumerate() {
			let (mut f, e) = match sc.prev[g] {
				Some(i) => (std::mem::take(&mut self.fires[i]), ease),
				None => (Fire { c: vec![gr.pos], ..Default::default() }, 1.0),
			};
			if f.c.is_empty() {
				f.c.push(gr.pos);
			}
			f.gone.iter_mut().for_each(|x| x.w -= dt / GONE_S);
			f.gone.retain(|x| x.w > 0.0);
			resize(&mut f, sc.ks[g], &sc.pts[g], sc.prev[g].is_none(), &mut sc.power);
			lloyd(&f.c, &sc.pts[g], 4, &mut sc.target, &mut sc.lacc);
			// RAC_TUNE flsh_snap (default 1): each origin on the tongue point nearest its centroid (never in the empty space between flames).
			if snap {
				for (j, tj) in sc.target.iter_mut().enumerate() {
					let mine = sc.pts[g].iter().filter(|p| nearest2(&f.c, p.0).0 == j).map(|p| p.0).min_by(|a, b| d2(*a, *tj).total_cmp(&d2(*b, *tj)));
					*tj = mine.unwrap_or(*tj);
				}
			}
			for (ci, ti) in f.c.iter_mut().zip(sc.target.iter()) {
				(0..3).for_each(|k| ci[k] += (ti[k] - ci[k]) * e);
			}
			f.pos = gr.pos;
			f.seen = true;
			sc.next.push(f);
		}
	}

	/// The origins (`self.out`: per fire its centres, then its merged-away ones) with their tongues' power share and radius (BALL as
	/// flitlist.rs `clusters`; a tongue nearer a merged-away centre than to any centre gives it its weight's part, the rest to its
	/// nearest centre: the shares move over as smoothly as the lights), and each fire light's maps (`pick`).
	fn assign(&mut self, list: &mut [Light], groups: &[Group], sc: &mut Scratch, band: f32) {
		sc.base.clear();
		for (g, gr) in groups.iter().enumerate() {
			sc.base.push(self.out.len());
			let f = &sc.next[g];
			let mk = |p: [f32; 3]| Group { pos: p, reach: (gr.reach + d2(p, gr.pos).sqrt()).min(2.0 * super::flitlist::REACH_HI), near: gr.near, flick: gr.flick, jit: None, fire: g as u8, small: gr.small, ..Default::default() };
			self.out.extend(f.c.iter().map(|&p| mk(p)));
			self.out.extend(f.gone.iter().map(|x| Group { size: x.size, ..mk(x.pos) }));
		}
		sc.acc.clear();
		sc.acc.resize(self.out.len(), (0.0, 0.0, 0.0));
		for g in 0..groups.len() {
			let f = &sc.next[g];
			for (&(p, w), &h) in sc.pts[g].iter().zip(&sc.half[g]) {
				let (j, _, dj, _) = nearest2(&f.c, p);
				let gone = f.gone.iter().enumerate().map(|(i, x)| (i, d2(x.pos, p).sqrt())).filter(|x| x.1 < dj).min_by(|x, y| x.1.total_cmp(&y.1));
				let (gi, gw) = gone.map_or((0, 0.0), |(i, _)| (i, f.gone[i].w));
				for (ix, at, wt) in [(j, f.c[j], w * (1.0 - gw)), (f.c.len() + gi, f.gone.get(gi).map_or(p, |x| x.pos), w * gw)] {
					if wt > 0.0 {
						let s = &mut sc.acc[sc.base[g] + ix];
						s.0 += wt;
						s.1 += wt * d2(p, at);
						s.2 += wt * h;
					}
				}
			}
		}
		sc.fire_w.clear();
		sc.fire_w.extend(sc.pts[..groups.len()].iter().map(|v| v.iter().map(|p| p.1).sum::<f32>()));
		for l in list.iter_mut().filter(|l| l.fire) {
			let Some(g) = l.group.map(usize::from).filter(|&g| g < groups.len()) else { continue };
			let (a, alt) = pick(&sc.next[g].c, &sc.next[g].gone, l.pos, band);
			l.group = Some((sc.base[g] + a) as u8);
			l.alt = alt.map(|(b, w)| ((sc.base[g] + b) as u8, w));
		}
		for (k, (o, &(p, r, h))) in self.out.iter_mut().zip(sc.acc.iter()).enumerate() {
			let g = o.fire as usize;
			o.share = p / sc.fire_w[g].max(1e-6);
			if k - sc.base[g] >= sc.next[g].c.len() {
				// A merged-away centre: its part of the fire's mean, no rank of its own (omni.rs: its map fades out drawn).
				continue;
			}
			o.power = groups[g].power * o.share;
			o.area = o.power;
			let (r, h) = if p > 0.0 { (r / p, h / p) } else { (0.0, 0.0) };
			o.size = (BALL * r + h * h).sqrt().clamp(super::flitlist::RS_LO, super::flitlist::GROUP);
		}
		for (g, f) in sc.next.iter_mut().enumerate() {
			f.size.clear();
			f.size.extend(self.out[sc.base[g]..sc.base[g] + f.c.len()].iter().map(|o| o.size));
		}
	}
}

/// A flame tongue for `Split::step`: (its light point (foot + flit_lift x height), weight h x w, half size max(w, h) / 2).
pub(super) type Tongue = ([f32; 3], f32, f32);

/// The tongues of `tongues` per fire (`n` fires) into `sc.pts` / `sc.half`: each goes to the fire of its nearest fire light of `list`
/// within flitlist.rs GROUP m (none: no fire); a fire without a tongue takes its lights (area, body radius).
fn points(list: &[Light], n: usize, tongues: &[Tongue], sc: &mut Scratch) {
	if sc.pts.len() < n {
		sc.pts.resize_with(n, Vec::new);
		sc.half.resize_with(n, Vec::new);
	}
	sc.pts.iter_mut().for_each(Vec::clear);
	sc.half.iter_mut().for_each(Vec::clear);
	sc.fl.clear();
	sc.fl.extend((0..list.len()).filter(|&i| list[i].fire && list[i].group.is_some_and(|g| usize::from(g) < n)));
	let g2 = super::flitlist::GROUP * super::flitlist::GROUP;
	for &(p, w, h) in tongues.iter().filter(|t| t.1 > 0.0) {
		let near = sc.fl.iter().map(|&i| (i, d2(list[i].pos, p))).filter(|x| x.1 < g2).min_by(|a, b| a.1.total_cmp(&b.1));
		if let Some((i, _)) = near {
			let g = usize::from(list[i].group.unwrap_or(0));
			sc.pts[g].push((p, w));
			sc.half[g].push(h);
		}
	}
	for g in 0..n {
		if sc.pts[g].is_empty() {
			for &i in sc.fl.iter().filter(|&&i| list[i].group == Some(g as u8)) {
				sc.pts[g].push((list[i].pos, list[i].area.max(1e-6)));
				sc.half[g].push(list[i].rs);
			}
		}
	}
}

fn smooth(x: f32) -> f32 {
	let x = x.clamp(0.0, 1.0);
	x * x * (3.0 - 2.0 * x)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fl(x: f32, y: f32, g: u8) -> Light {
		Light { pos: [x, y, 1.0], reach: 12.0, rgb: [1.0, 0.5, 0.1], rs: 1.0, group: Some(g), fire: true, area: 4.0, flick: 1.0, alt: None, small: false, carried: false }
	}

	fn wide() -> Vec<Light> {
		(0..8).map(|i| fl(-4.0 + i as f32 * 8.0 / 7.0, 0.3 * (i % 2) as f32, 0)).collect()
	}

	#[test]
	fn the_budget_splits_the_nearest_big_fire_first() {
		assert_eq!(want(0.5, 5, 0, 3.0, 4), 1);
		assert_eq!(want(4.5, 8, 0, 3.0, 4), 4);
		assert_eq!(want(4.5, 2, 0, 3.0, 4), 2);
		// Hysteresis: 1 + 2 x 2 / 3 = 2.33 keeps 3 and 2; from nothing it rounds to 2.
		assert_eq!((want(2.0, 8, 3, 3.0, 4), want(2.0, 8, 0, 3.0, 4)), (3, 2));
		assert_eq!(alloc(&[4, 1, 3], &[1.0, 5.0, 0.5], &[true, true, true], 4), vec![2, 1, 1]);
		assert_eq!(alloc(&[4, 3], &[1.0, 5.0], &[true, true], 4), vec![1, 3], "the strongest first");
		assert_eq!(alloc(&[4, 4], &[9.0, 5.0], &[true, false], 4), vec![4, 1]);
		assert_eq!(alloc(&[4, 4, 4, 4, 4], &[1.0; 5], &[true; 5], 4), vec![1; 5]);
		assert_eq!(alloc(&[4], &[1.0], &[true], 1), vec![1]);
	}

	#[test]
	fn the_sub_emitters_share_the_fire_and_cover_its_lights() {
		let mut s = Split::default();
		let mut list = wide();
		let g = super::super::flitlist::groups(&mut list);
		let subs = s.step(&mut list, &g, [0.0, -5.0, 1.0], 4, 0.0).to_vec();
		assert_eq!(subs.len(), 4);
		let sum: f32 = subs.iter().map(|s| s.power).sum();
		assert!((sum - g[0].power).abs() < 1e-3, "{sum} vs {}", g[0].power);
		let shares: f32 = subs.iter().map(|s| s.share).sum();
		assert!((shares - 1.0).abs() < 1e-4 && subs.iter().all(|s| s.fire == 0));
		// Every light reads one of the fire's centres, the nearest one; the centres sit apart along the fire.
		for l in &list {
			let j = l.group.unwrap() as usize;
			assert!(subs.iter().all(|s| d2(s.pos, l.pos) >= d2(subs[j].pos, l.pos) - 1e-4));
		}
		let mut xs: Vec<f32> = subs.iter().map(|s| s.pos[0]).collect();
		xs.sort_by(f32::total_cmp);
		assert!(xs[0] < -2.0 && xs[3] > 2.0, "{xs:?}");
		assert!(subs.iter().all(|s| s.size >= super::super::flitlist::RS_LO && s.size < g[0].size));
	}

	/// Review M1: a fire of small flames only near the eye takes no map (omni.rs `candidates`), so it takes no origin from the big
	/// fire either: the big fire keeps all 4 with a small pool flame by the camera.
	#[test]
	fn a_small_fire_near_the_eye_takes_no_origin() {
		let mut list = wide();
		let mut pool = fl(0.0, -14.0, 0);
		(pool.small, pool.area, pool.reach) = (true, 0.1, 3.0);
		list.push(pool);
		let g = super::super::flitlist::groups(&mut list);
		assert!(g.len() == 2 && !g[0].small && g[1].small, "{g:?}");
		let subs = Split::default().step(&mut list, &g, [0.0, -15.0, 1.0], 4, 0.0).to_vec();
		assert_eq!(subs.iter().filter(|s| s.fire == 0).count(), 4, "the big fire keeps 4 origins: {subs:?}");
		assert_eq!(subs.iter().filter(|s| s.fire == 1).count(), 1);
	}

	#[test]
	fn the_centres_hold_still_and_grow_without_a_jump() {
		let mut s = Split::default();
		let mut list = wide();
		let g = super::super::flitlist::groups(&mut list);
		let a = s.step(&mut list.clone(), &g, [0.0; 3], 4, 0.0).to_vec();
		// Same lights, next frames: the same centres (stable); a frame of the same game time moves nothing.
		let b = s.step(&mut list.clone(), &g, [0.0; 3], 4, 1.0 / 60.0).to_vec();
		let c = s.step(&mut list.clone(), &g, [0.0; 3], 4, 1.0 / 60.0).to_vec();
		for i in 0..4 {
			assert!(d2(a[i].pos, b[i].pos) < 1e-6 && d2(b[i].pos, c[i].pos) < 1e-9);
		}
		// One map -> four: the new centres start on their parent and slide out over frames (no jump).
		let g1 = vec![g[0]];
		let mut s3 = Split::default();
		let _ = s3.step(&mut list.clone(), &g1, [0.0; 3], 4, 0.0);
		s3.fires[0].c = vec![g1[0].pos];
		let mut prev = vec![g1[0].pos];
		for f in 1..40 {
			let out = s3.step(&mut list.clone(), &g1, [0.0; 3], 4, f as f64 / 60.0).to_vec();
			assert_eq!(out.len(), 4);
			let now: Vec<[f32; 3]> = out.iter().map(|o| o.pos).collect();
			// The new ones from their parent (prev[0] after the split), the others from where they were.
			for (i, p) in now.iter().enumerate() {
				let from = *prev.get(i).unwrap_or(&prev[0]);
				assert!(d2(*p, from).sqrt() < 0.5, "frame {f} centre {i}: {p:?} from {from:?}");
			}
			prev = now;
		}
		let mut xs: Vec<f32> = prev.iter().map(|p| p[0]).collect();
		xs.sort_by(f32::total_cmp);
		assert!(xs[0] < -2.0 && xs[3] > 2.0, "{xs:?}");
	}

	#[test]
	fn the_origins_sit_on_the_tongues_and_split_their_power() {
		// Two lights, but a row of tongues 10 m long: four origins along the tongues; a tongue 20 m off belongs to no fire.
		let mut list = vec![fl(-1.5, 0.0, 0), fl(1.5, 0.0, 0)];
		let g = super::super::flitlist::groups(&mut list);
		let mut ts: Vec<Tongue> = (0..12).map(|i| ([-5.0 + i as f32 * 10.0 / 11.0, 0.0, 1.5], 1.0 + (i % 3) as f32, 0.6)).collect();
		ts.push(([30.0, 0.0, 1.0], 50.0, 1.0));
		let mut s = Split { tongues: ts, ..Default::default() };
		let subs = s.step(&mut list, &g, [0.0; 3], 4, 0.0).to_vec();
		assert_eq!(subs.len(), 4);
		assert!((subs.iter().map(|s| s.share).sum::<f32>() - 1.0).abs() < 1e-4);
		assert!((subs.iter().map(|s| s.power).sum::<f32>() - g[0].power).abs() < 1e-3);
		let mut xs: Vec<f32> = subs.iter().map(|s| s.pos[0]).collect();
		xs.sort_by(f32::total_cmp);
		assert!(xs[0] < -3.0 && xs[3] > 3.0 && xs[3] < 6.0, "{xs:?}");
		assert!(subs.iter().all(|s| (s.pos[2] - 1.5).abs() < 1e-4 && s.size < 2.0));
	}

	#[test]
	fn a_light_on_the_bisector_blends_both_maps() {
		let mut s = Split::default();
		let mut list = vec![fl(-2.0, 0.0, 0), fl(-1.6, 0.0, 0), fl(1.6, 0.0, 0), fl(2.0, 0.0, 0), fl(0.0, 0.0, 0)];
		list[4].area = 0.01;
		let g = super::super::flitlist::groups(&mut list);
		let subs = s.step(&mut list, &g, [0.0; 3], 4, 0.0).to_vec();
		assert_eq!(subs.len(), 2);
		let (a, w) = list[4].alt.expect("blend");
		assert!(a != list[4].group.unwrap() && (w - 0.5).abs() < 0.05, "{:?}", list[4].alt);
		assert!(list[0].alt.is_none());
	}

	/// Review S3: 4 -> 3 origins: the merged-away centre keeps its map while its lights move over to the others (no light's shadow
	/// origin jumps, the fire's share-weighted centre neither), then it goes after GONE_S.
	#[test]
	fn a_merge_moves_the_lights_over_without_a_jump() {
		let mut s = Split::default();
		let list = wide();
		let mut g0 = list.clone();
		let g = super::super::flitlist::groups(&mut g0);
		let at = |out: &[Group], l: &Light| {
			let (a, w) = (out[l.group.unwrap() as usize].pos, l.alt.map_or(0.0, |x| x.1));
			let b = l.alt.map_or(a, |x| out[x.0 as usize].pos);
			[0, 1, 2].map(|k| a[k] * (1.0 - w) + b[k] * w)
		};
		let mean = |out: &[Group]| [0, 1, 2].map(|k| out.iter().map(|o| o.share * o.pos[k]).sum::<f32>() / out.iter().map(|o| o.share).sum::<f32>());
		let (mut prev, mut pm): (Vec<[f32; 3]>, [f32; 3]) = (Vec::new(), [0.0; 3]);
		let mut lens = Vec::new();
		for f in 0..120 {
			let mut l = g0.clone();
			let out = s.step(&mut l, &g, [0.0; 3], if f < 60 { 4 } else { 3 }, f as f64 / 60.0).to_vec();
			lens.push(out.len());
			let now: Vec<[f32; 3]> = l.iter().map(|x| at(&out, x)).collect();
			let m = mean(&out);
			if f > 30 {
				for (i, (p, q)) in now.iter().zip(&prev).enumerate() {
					assert!(d2(*p, *q).sqrt() < 0.3, "frame {f} light {i}: {q:?} -> {p:?}");
				}
				assert!(d2(m, pm).sqrt() < 0.1, "frame {f}: the fire's mean {pm:?} -> {m:?}");
				assert!((out.iter().map(|o| o.share).sum::<f32>() - 1.0).abs() < 1e-3);
			}
			(prev, pm) = (now, m);
		}
		assert_eq!((lens[59], lens[60], lens[75]), (4, 4, 4), "the merged-away map stays while it fades");
		assert_eq!(lens[60 + 23], 3, "and goes after GONE_S");
		assert!(list.len() == 8);
	}

	#[test]
	fn medium_and_the_reference_keep_one_origin_a_fire() {
		let mut s = Split::default();
		let mut list = wide();
		let g = super::super::flitlist::groups(&mut list);
		let before = list.clone();
		let one: Vec<Group> = g.iter().enumerate().map(|(i, g)| Group { fire: i as u8, share: 1.0, ..*g }).collect();
		assert_eq!(s.step(&mut list, &g, [0.0; 3], 1, 0.0), &one[..]);
		assert_eq!(list, before);
	}
}
