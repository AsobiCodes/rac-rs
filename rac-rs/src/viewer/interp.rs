//! [PC] FRAME SMOOTHING (port extra, user request 2026-10-17: "unlocked frame rate like the Sly Cooper port"; Pause -> Options ->
//! Graphics -> Frame smoothing, pc_options `smoothing=on|off`). The retail game makes one new state per field (50 Hz PAL, 59.94 Hz
//! USA) and keeps doing exactly that: nothing here is read by the game, the sound or the RNG, only what a display frame draws.
//!
//! Each display frame draws the state between the field before the shown one and the shown one, at `alpha` = time since that
//! field / field time (the fixed-step scheme smooth.rs already runs for the camera, the props, the hero and their skinned poses:
//! `Smooth::frame` takes this module's verdict). This module adds the rest of what retail RAM gives per field and decides when
//! not to blend at all:
//! - The field's draw lists (`RetailFx`: effect pool sprites / lines / ribbons by pool slot, glows, landing markers, blob shadows,
//!   dynamic lights, glints, vendor glows, ship shadows, queue-A / queue-B quads; the shadow-volume records by caster moby; the
//!   nanotech orbs) are blended item by item into a copy that `Retail::fx` / `shadows` / `orb_draws` hand out until the next field
//!   (retail.rs `SmoothDraw`). Items pair by their own key (pool slot, light slot, glint slot, caster moby) or, in keyless lists,
//!   by order among the items of the same kind; an item without a partner, of another kind or `SNAP_DIST` away is drawn as the
//!   field has it.
//! - Cuts (one field drawn unblended): the camera jumped or turned far beyond its motion of the field before, or zoomed abruptly
//!   (`cam_cut`); the level, game mode or scene view changed; a frozen or held field (menus, pause, the front end); the first field
//!   after a load.
//! Not blended: the HUD and every 2D overlay (retail's GS picture), lightning arc strands (new random strands each field), water
//! and ocean surfaces from RAM, sky sprites, the swing rope (built from the blended hero).
//! Off: `frame` answers false and clears the copy, so every frame draws the newest field exactly as without this module.
//! Latency: the newest field starts to show at once and is fully shown one field later (+10 ms on average at 50 Hz).
use super::smooth::{lerp3, lerp_angle, near};
use crate::particles::{OrbDraw, PointLight, WorldQuad};
use crate::retail::{Cam, Retail, RetailFx, RetailShadows, SmoothDraw};
use crate::shadows::RecPrim;
use std::cell::RefCell;
use std::collections::HashMap;

type V3 = [f32; 3];

/// One field's draw lists and view, as the field left them.
#[derive(Default)]
struct Field {
	fx: Option<RetailFx>,
	shadows: Option<RetailShadows>,
	orbs: Option<Vec<OrbDraw>>,
	cam: Option<Cam>,
	/// (level, game mode, scene view, frozen / held / front end).
	key: (u32, u32, bool, bool),
}

#[derive(Default)]
struct State {
	prev: Option<Field>,
	cur: Option<Field>,
	/// The camera's move of the field before the newest: (eye distance, view turn in radians, |log| zoom change).
	last_move: (f32, f32, f32),
	/// The newest field is a cut: drawn as it is until the next field.
	cut: bool,
	/// F9 (test launchers with RAC_SMOOTH_KEY=1): this session's override of the menu row.
	over: Option<bool>,
	/// Fields seen, cuts (RAC_SMOOTH_LOG).
	fields: u64,
	cuts: u64,
	/// Blended display frames since the last log line (RAC_SMOOTH_LOG): count, summed and largest picture delay over the
	/// newest field in seconds ((1 - alpha) * field time), summed blend time in seconds.
	stat: (u64, f64, f64, f64),
	/// The queue-A and queue-B quads that blend between `prev` and `cur` (`quad_pairs`), once per field.
	quads: [Vec<(u32, u32)>; 2],
}

/// [PC] FxHash-style hasher for the per-field pairing maps (`pair_ordered`, `still_kinds`): with std's SipHash, Pokitaru's pool
/// and waterfall quads cost ~2.3 ms of the main thread a field. Only lookups use it, never an iteration order.
#[derive(Default, Clone, Copy)]
struct FxHasher(u64);

impl FxHasher {
	#[inline]
	fn add(&mut self, x: u64) {
		self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x517c_c1b7_2722_0a95);
	}
}

impl std::hash::Hasher for FxHasher {
	fn finish(&self) -> u64 {
		self.0
	}
	fn write(&mut self, b: &[u8]) {
		for &x in b {
			self.add(x as u64);
		}
	}
	fn write_u8(&mut self, x: u8) {
		self.add(x as u64);
	}
	fn write_u16(&mut self, x: u16) {
		self.add(x as u64);
	}
	fn write_u32(&mut self, x: u32) {
		self.add(x as u64);
	}
	fn write_i32(&mut self, x: i32) {
		self.add(x as u32 as u64);
	}
	fn write_u64(&mut self, x: u64) {
		self.add(x);
	}
	fn write_usize(&mut self, x: usize) {
		self.add(x as u64);
	}
}

type FxMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<FxHasher>>;

thread_local! {
	static ST: RefCell<State> = RefCell::new(State::default());
}

/// The F9 test key (only with RAC_SMOOTH_KEY=1, set by the test launcher): flips smoothing for this session.
pub(super) fn key(k: winit::keyboard::KeyCode, menu: bool) {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	if k != winit::keyboard::KeyCode::F9 || !*ON.get_or_init(|| std::env::var("RAC_SMOOTH_KEY").is_ok_and(|v| v == "1")) {
		return;
	}
	ST.with(|s| {
		let mut s = s.borrow_mut();
		let now = !s.over.unwrap_or(menu);
		s.over = Some(now);
		eprintln!("smooth: F9: frame smoothing {}", if now { "On" } else { "Off" });
	});
}

/// The player's choice: the menu row (`menu`) unless F9 flipped it this session.
pub(super) fn wanted(menu: bool) -> bool {
	ST.with(|s| s.borrow().over.unwrap_or(menu))
}

/// Unit view direction of a retail camera (yaw about z, pitch up).
fn fwd(c: &Cam) -> V3 {
	let (sy, cy) = c.1.sin_cos();
	let (sp, cp) = c.2.sin_cos();
	[cy * cp, sy * cp, sp]
}

/// A camera cut between two fields: a move or turn far beyond the one of the field before (`last` = its eye distance, turn,
/// zoom change), or an abrupt zoom. Gameplay cameras (running, the hoverboard, the ship, a quick mouse turn) move steadily and
/// stay well inside; a scene cut or a respawn jumps.
fn cam_cut(a: &Cam, b: &Cam, last: (f32, f32, f32)) -> ((f32, f32, f32), bool) {
	let d = (0..3).map(|i| (b.0[i] - a.0[i]) * (b.0[i] - a.0[i])).sum::<f32>().sqrt();
	let (fa, fb) = (fwd(a), fwd(b));
	let turn = (fa[0] * fb[0] + fa[1] * fb[1] + fa[2] * fb[2]).clamp(-1.0, 1.0).acos();
	let zoom = if a.3 > 1e-6 && b.3 > 1e-6 { (b.3 / a.3).ln().abs() } else { 0.0 };
	let jump = d > 1.5 && d > 3.0 * last.0 + 0.75;
	let swing = turn > 0.6 && turn > 3.0 * last.1 + 0.2;
	let zoomed = zoom > 0.22 && zoom > 3.0 * last.2 + 0.05;
	((d, turn, zoom), jump || swing || zoomed)
}

/// Retail branch of each tick, after `Retail::sync`: keeps the field's lists and decides whether it is a cut. `on` = the player's
/// choice (`wanted`): while it is Off nothing is kept (a copy of the lists and the quad pairing cost ~2.3 ms a field on Pokitaru,
/// for nothing), and the first field after it is switched On is a cut.
pub(super) fn field(r: &Retail, on: bool) {
	if !on {
		ST.with(|s| {
			let mut s = s.borrow_mut();
			s.fields += 1;
			s.prev = None;
			s.cur = None;
			s.cut = true;
			s.quads = Default::default();
		});
		return;
	}
	let (fx, sh, orbs, key) = r.smooth_src();
	let f = Field { fx: fx.cloned(), shadows: sh.cloned(), orbs: orbs.map(<[OrbDraw]>::to_vec), cam: Some(r.camera(1.0)), key };
	ST.with(|s| {
		let mut s = s.borrow_mut();
		s.fields += 1;
		let prev = s.cur.take();
		let (mv, cut) = match prev.as_ref() {
			Some(p) if p.key == f.key && !f.key.3 => match (&p.cam, &f.cam) {
				(Some(a), Some(b)) => cam_cut(a, b, s.last_move),
				_ => ((0.0, 0.0, 0.0), true),
			},
			_ => ((0.0, 0.0, 0.0), true),
		};
		s.last_move = mv;
		s.cut = cut;
		if cut {
			s.cuts += 1;
			if std::env::var_os("RAC_SMOOTH_LOG").is_some() {
				eprintln!("smooth: cut {} of {} fields at field {} (move {:.2} m, turn {:.2} rad, zoom {:.2}, key {:?})", s.cuts, s.fields, r.field_count(), mv.0, mv.1, mv.2, f.key);
			}
		}
		s.quads = match (prev.as_ref().and_then(|p| p.fx.as_ref()), f.fx.as_ref()) {
			(Some(a), Some(b)) if !cut => [quad_pairs(&a.qa, &b.qa), quad_pairs(&a.qb, &b.qb)],
			_ => Default::default(),
		};
		s.prev = prev;
		s.cur = Some(f);
		if s.fields % 500 == 0 && s.stat.0 > 0 && std::env::var_os("RAC_SMOOTH_LOG").is_some() {
			let (n, sum, max, cost) = s.stat;
			eprintln!(
				"smooth: {n} blended frames in 500 fields: picture delay mean {:.1} ms, max {:.1} ms; blend {:.3} ms/frame",
				sum / n as f64 * 1e3,
				max * 1e3,
				cost / n as f64 * 1e3
			);
			s.stat = (0, 0.0, 0.0, 0.0);
		}
	});
}

/// After the ticks of a display frame: whether this frame draws between fields (`want` = the player's choice, already false
/// while paused or capturing), and if so the blended lists go into `r`. Outside retail mode `want` stands as it is.
pub(super) fn frame(r: Option<&mut Retail>, want: bool, alpha: f32) -> bool {
	let Some(r) = r else { return want };
	let t0 = std::time::Instant::now();
	let blend = ST.with(|s| {
		let s = s.borrow();
		match (want && !s.cut && !r.paused(), &s.prev, &s.cur) {
			(true, Some(p), Some(c)) => Some(SmoothDraw {
				fx: match (&p.fx, &c.fx) {
					(Some(a), Some(b)) => Some(blend_fx(a, b, alpha, &s.quads)),
					_ => None,
				},
				shadows: match (&p.shadows, &c.shadows) {
					(Some(a), Some(b)) => Some(blend_shadows(a, b, alpha)),
					_ => None,
				},
				orbs: match (&p.orbs, &c.orbs) {
					(Some(a), Some(b)) => Some(blend_orbs(a, b, alpha)),
					_ => None,
				},
			}),
			_ => None,
		}
	});
	let on = blend.is_some();
	r.set_smooth(blend);
	if on {
		let late = (1.0 - alpha as f64) * crate::retail::disc::field_dt() as f64;
		let cost = t0.elapsed().as_secs_f64();
		ST.with(|s| {
			let st = &mut s.borrow_mut().stat;
			*st = (st.0 + 1, st.1 + late, st.2.max(late), st.3 + cost);
		});
	}
	on
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
	a + (b - a) * t
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
	std::array::from_fn(|i| lerp(a[i], b[i], t))
}

/// Unit-length blend of two directions (normals, light directions); the newer one when they cancel out.
fn lerp_dir(a: V3, b: V3, t: f32) -> V3 {
	let v = lerp3(a, b, t);
	let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
	let lb = (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt();
	if l > 1e-6 { v.map(|c| c / l * lb) } else { b }
}

/// Partners in a keyless list: item `i` of `cur` pairs with the k-th item of `prev` of the same kind when it is the k-th of its
/// kind in `cur` and lies within `SNAP_DIST`. Retail fills these lists in a fixed order (moby update order, queue order), so the
/// k-th of a kind is the same object unless one of its kind spawned or died before it this field (then `near` mostly refuses).
fn pair_ordered<K: std::hash::Hash + Eq + Copy>(prev: &[(K, V3)], cur: &[(K, V3)]) -> Vec<Option<usize>> {
	let mut by_kind: FxMap<K, Vec<usize>> = FxMap::default();
	for (j, (k, _)) in prev.iter().enumerate() {
		by_kind.entry(*k).or_default().push(j);
	}
	let mut seen: FxMap<K, usize> = FxMap::default();
	cur.iter()
		.map(|(k, p)| {
			let n = seen.entry(*k).or_insert(0);
			let j = by_kind.get(k).and_then(|v| v.get(*n)).copied();
			*n += 1;
			j.filter(|&j| near(prev[j].1, *p))
		})
		.collect()
}

/// [PC] How far (world units) a quad's corners may lie from last field's to count as the same standing quad (`still_kinds`): the
/// effects layer rebuilds positions from GS depth, which wobble by up to about 0.1 a field.
const STILL_EPS: f32 = 0.2;

/// [PC] The quad kinds (texture, blend) that stand still: at least half their quads this field lie where one of last field's did
/// (`STILL_EPS`). Pokitaru's pool surface and waterfall sheets are such meshes; which of their triangles retail draws changes as
/// the camera turns (its VU1 clip), so the k-th of a kind is often another triangle than last field's k-th, and blending the two
/// pulled neighbours together: holes in the water for a frame (user clip 2026-10-19). Standing quads are drawn as they are.
/// The same triangle may come with its corners in another order (where its strip starts changes with the clip), so corners
/// match in any order, and the grid cell is the quad's centre.
/// A quad hits when a quad of its kind in last field's grid cell or one of the 26 around it is the `same`; a kind of fewer than 4
/// quads never counts, and a kind is decided (and its other quads skip the search) once half its quads hit or no longer can.
fn still_kinds(prev: &[WorldQuad], cur: &[WorldQuad]) -> std::collections::HashSet<(usize, bool)> {
	let mid = |q: &WorldQuad| std::array::from_fn::<f32, 3, _>(|c| (q.pos[0][c] + q.pos[1][c] + q.pos[2][c] + q.pos[3][c]) * 0.25);
	let cell = |p: [f32; 3]| ((p[0] / 0.5).floor() as i32, (p[1] / 0.5).floor() as i32, (p[2] / 0.5).floor() as i32);
	// Per kind of this field: (hits, quads searched, quads).
	let mut count: FxMap<(usize, bool), (u32, u32, u32)> = FxMap::default();
	for q in cur {
		count.entry(kind_of(q)).or_default().2 += 1;
	}
	// Last field's quads of the kinds that can count, chained per (kind, cell): `head` holds a cell's last quad, `next` the one before.
	const END: u32 = u32::MAX;
	let mut head: FxMap<((usize, bool), (i32, i32, i32)), u32> = FxMap::default();
	let mut next = vec![END; prev.len()];
	for (j, q) in prev.iter().enumerate() {
		let kind = kind_of(q);
		if count.get(&kind).is_some_and(|c| c.2 >= 4) {
			let e = head.entry((kind, cell(mid(q)))).or_insert(END);
			next[j] = *e;
			*e = j as u32;
		}
	}
	let close = |a: V3, b: V3| (0..3).all(|c| (a[c] - b[c]).abs() < STILL_EPS);
	let within = |a: &WorldQuad, b: &WorldQuad| a.pos.iter().all(|&p| b.pos.iter().any(|&q| close(p, q)));
	let same = |a: &WorldQuad, b: &WorldQuad| within(a, b) && within(b, a);
	// The quad's own cell first (where a standing quad nearly always is), then the 26 around it.
	const AROUND: [(i32, i32, i32); 27] = {
		let mut o = [(0, 0, 0); 27];
		let mut n = 1;
		let mut i = 0;
		while i < 27 {
			let d = ((i / 9) as i32 - 1, (i / 3 % 3) as i32 - 1, (i % 3) as i32 - 1);
			if d.0 != 0 || d.1 != 0 || d.2 != 0 {
				o[n] = d;
				n += 1;
			}
			i += 1;
		}
		o
	};
	for q in cur {
		let kind = kind_of(q);
		let Some(e) = count.get_mut(&kind) else { continue };
		let (hit, searched, n) = *e;
		if n < 4 || 2 * hit >= n || 2 * (hit + n - searched) < n {
			continue;
		}
		let (cx, cy, cz) = cell(mid(q));
		let found = AROUND.iter().any(|&(dx, dy, dz)| {
			let mut j = head.get(&(kind, (cx + dx, cy + dy, cz + dz))).copied().unwrap_or(END);
			while j != END {
				if same(&prev[j as usize], q) {
					return true;
				}
				j = next[j as usize];
			}
			false
		});
		e.0 += found as u32;
		e.1 += 1;
	}
	count.into_iter().filter(|&(_, (hit, _, n))| n >= 4 && 2 * hit >= n).map(|(k, _)| k).collect()
}

/// [PC] A quad's kind for pairing (texture, blend): the effects layer packs each corner's fog F and the fog colour into the high
/// bits of `tex` (retail/efx_draw.rs `FOG_BITS`), which change as the camera moves, so they are left out. With them in, the same
/// waterfall sheet was hundreds of kinds that changed from field to field and its triangles paired with others of the same fog:
/// torn sheets on smoothed frames while the camera turned (user clip 2026-10-19).
fn kind_of(q: &WorldQuad) -> (usize, bool) {
	(crate::retail::efx_draw::plain_tex(q.tex), q.additive)
}

/// The quads of `cur` that blend with one of `prev` as (index in `cur`, index in `prev`): the k-th quad of a kind pairs with the
/// k-th of that kind (`pair_ordered`), not of a standing kind (`still_kinds`, left as they are), every corner `near`. Once per
/// field; each display frame only lerps them (`lerp_quads`).
fn quad_pairs(prev: &[WorldQuad], cur: &[WorldQuad]) -> Vec<(u32, u32)> {
	let still = still_kinds(prev, cur);
	let key = |q: &WorldQuad| (kind_of(q), q.pos[0]);
	let pairs = pair_ordered(&prev.iter().map(key).collect::<Vec<_>>(), &cur.iter().map(key).collect::<Vec<_>>());
	cur.iter()
		.zip(pairs)
		.enumerate()
		.filter_map(|(i, (q, j))| {
			let j = j?;
			(!still.contains(&kind_of(q)) && (0..4).all(|k| near(prev[j].pos[k], q.pos[k]))).then_some((i as u32, j as u32))
		})
		.collect()
}

/// `pairs`: `quad_pairs(prev, cur)`.
fn lerp_quads(prev: &[WorldQuad], cur: &mut [WorldQuad], t: f32, pairs: &[(u32, u32)]) {
	for &(i, j) in pairs {
		let (a, q) = (&prev[j as usize], &mut cur[i as usize]);
		for k in 0..4 {
			q.pos[k] = lerp3(a.pos[k], q.pos[k], t);
			q.colour[k] = lerp4(a.colour[k], q.colour[k], t);
		}
	}
}

/// The field's effect lists between field `a` (t = 0) and field `b` (t = 1); `quads`: `quad_pairs` of queue A and queue B.
fn blend_fx(a: &RetailFx, b: &RetailFx, t: f32, quads: &[Vec<(u32, u32)>; 2]) -> RetailFx {
	let mut o = b.clone();
	// Effect pool sprites and lines: the same pool slot with the same texture / look.
	if a.sprite_slots.len() == a.sprites.len() && o.sprite_slots.len() == o.sprites.len() {
		let at: HashMap<u16, usize> = a.sprite_slots.iter().enumerate().map(|(i, &s)| (s, i)).collect();
		for (s, slot) in o.sprites.iter_mut().zip(&o.sprite_slots) {
			let Some(p) = at.get(slot).map(|&j| &a.sprites[j]) else { continue };
			if p.tex == s.tex && p.additive == s.additive && p.flat == s.flat && p.pool == s.pool && near(p.pos, s.pos) {
				s.pos = lerp3(p.pos, s.pos, t);
				s.size = lerp(p.size, s.size, t);
				s.rot = lerp_angle(p.rot, s.rot, t);
				s.colour = lerp4(p.colour, s.colour, t);
			}
		}
	}
	if a.line_slots.len() == a.lines.len() && o.line_slots.len() == o.lines.len() {
		let at: HashMap<u16, usize> = a.line_slots.iter().enumerate().map(|(i, &s)| (s, i)).collect();
		for (l, slot) in o.lines.iter_mut().zip(&o.line_slots) {
			let Some(p) = at.get(slot).map(|&j| &a.lines[j]) else { continue };
			if near(p.a, l.a) && near(p.b, l.b) {
				l.a = lerp3(p.a, l.a, t);
				l.b = lerp3(p.b, l.b, t);
				l.col_a = lerp4(p.col_a, l.col_a, t);
				l.col_b = lerp4(p.col_b, l.col_b, t);
			}
		}
	}
	{
		let at: HashMap<u16, usize> = a.ribbons.iter().enumerate().map(|(i, r)| (r.slot, i)).collect();
		for r in &mut o.ribbons {
			let Some(p) = at.get(&r.slot).map(|&j| &a.ribbons[j]) else { continue };
			if p.tex == r.tex && near(p.head, r.head) && near(p.tail, r.tail) {
				r.head = lerp3(p.head, r.head, t);
				r.tail = lerp3(p.tail, r.tail, t);
				r.half = lerp(p.half, r.half, t);
				r.scale = lerp(p.scale, r.scale, t);
			}
		}
	}
	// Keyless lists: glows (kind = colour word), landing markers, blob shadows, vendor glows, ship shadows.
	{
		let k = |g: &([f32; 3], f32, f32, u32)| (g.3, g.0);
		let pairs = pair_ordered(&a.glows.iter().map(k).collect::<Vec<_>>(), &o.glows.iter().map(k).collect::<Vec<_>>());
		for (g, j) in o.glows.iter_mut().zip(pairs) {
			if let Some(p) = j.map(|j| a.glows[j]) {
				g.0 = lerp3(p.0, g.0, t);
				g.1 = lerp(p.1, g.1, t);
			}
		}
	}
	{
		let k = |r: &crate::game::Reticle| (0u8, r.pos);
		let pairs = pair_ordered(&a.reticles.iter().map(k).collect::<Vec<_>>(), &o.reticles.iter().map(k).collect::<Vec<_>>());
		for (r, j) in o.reticles.iter_mut().zip(pairs) {
			if let Some(p) = j.map(|j| &a.reticles[j]) {
				r.pos = lerp3(p.pos, r.pos, t);
				r.normal = lerp_dir(p.normal, r.normal, t);
			}
		}
	}
	{
		let k = |b: &([f32; 3], f32, [f32; 3])| (0u8, b.0);
		let pairs = pair_ordered(&a.blobs.iter().map(k).collect::<Vec<_>>(), &o.blobs.iter().map(k).collect::<Vec<_>>());
		for (b, j) in o.blobs.iter_mut().zip(pairs) {
			if let Some(p) = j.map(|j| a.blobs[j]) {
				*b = (lerp3(p.0, b.0, t), lerp(p.1, b.1, t), lerp_dir(p.2, b.2, t));
			}
		}
	}
	{
		let k = |v: &crate::retail::VendorGlow| (0u8, v.pos);
		let pairs = pair_ordered(&a.vendors.iter().map(k).collect::<Vec<_>>(), &o.vendors.iter().map(k).collect::<Vec<_>>());
		for (v, j) in o.vendors.iter_mut().zip(pairs) {
			if let Some(p) = j.map(|j| &a.vendors[j]) {
				v.pos = lerp3(p.pos, v.pos, t);
				v.ramp = lerp(p.ramp, v.ramp, t);
				for n in 0..4 {
					v.nodes[n] = lerp3(p.nodes[n], v.nodes[n], t);
				}
			}
		}
	}
	{
		let k = |s: &crate::retail::ShipShadow| (s.ship, s.pos);
		let pairs = pair_ordered(&a.ship_shadows.iter().map(k).collect::<Vec<_>>(), &o.ship_shadows.iter().map(k).collect::<Vec<_>>());
		for (s, j) in o.ship_shadows.iter_mut().zip(pairs) {
			if let Some(p) = j.map(|j| &a.ship_shadows[j]) {
				s.pos = lerp3(p.pos, s.pos, t);
				for r in 0..3 {
					s.m[r] = lerp3(p.m[r], s.m[r], t);
				}
			}
		}
	}
	// Keyed by slot: dynamic lights, glints.
	for (l, p) in o.lights.iter_mut().zip(&a.lights) {
		if let (Some(l), Some(p)) = (l.as_mut(), p) {
			if near(p.pos, l.pos) {
				*l = PointLight { pos: lerp3(p.pos, l.pos, t), radius: lerp(p.radius, l.radius, t), rgb: lerp3(p.rgb, l.rgb, t), intensity: lerp(p.intensity, l.intensity, t) };
			}
		}
	}
	for (g, p) in o.glints.iter_mut().zip(&a.glints) {
		if g.life > 0 && p.life > 0 && g.owner == p.owner && near(p.pos, g.pos) {
			g.pos = lerp3(p.pos, g.pos, t);
			g.angle = lerp_angle(p.angle, g.angle, t);
			g.size = lerp(p.size, g.size, t);
		}
	}
	// Queue-A / queue-B quads (Taunter rings, tube fade, the effects layer's quads, ...): k-th quad of a texture and blend.
	lerp_quads(&a.qa, &mut o.qa, t, &quads[0]);
	lerp_quads(&a.qb, &mut o.qb, t, &quads[1]);
	if let (Some(p), Some(h)) = (a.hero, o.hero.as_mut()) {
		if near(p, *h) {
			*h = lerp3(p, *h, t);
		}
	}
	o
}

/// Shadow-volume records: the same caster moby with primitives of the same shape.
fn blend_shadows(a: &RetailShadows, b: &RetailShadows, t: f32) -> RetailShadows {
	let mut o = b.clone();
	let at: HashMap<u32, usize> = a.casters.iter().enumerate().map(|(i, c)| (c.0, i)).collect();
	for (moby, c) in &mut o.casters {
		let Some(p) = at.get(moby).map(|&j| &a.casters[j].1) else { continue };
		if p.prims.len() != c.prims.len() {
			continue;
		}
		let blended: Option<Vec<RecPrim>> = p
			.prims
			.iter()
			.zip(&c.prims)
			.map(|(pa, pb)| match (*pa, *pb) {
				(RecPrim::Sphere { c: ca, r: ra, n: na }, RecPrim::Sphere { c: cb, r: rb, n: nb }) if na == nb && near(ca, cb) => Some(RecPrim::Sphere { c: lerp3(ca, cb, t), r: lerp(ra, rb, t), n: nb }),
				(RecPrim::Capsule { p0: a0, r0: ar0, p1: a1, r1: ar1, n0: an0, n1: an1 }, RecPrim::Capsule { p0: b0, r0: br0, p1: b1, r1: br1, n0, n1 }) if (an0, an1) == (n0, n1) && near(a0, b0) && near(a1, b1) => {
					Some(RecPrim::Capsule { p0: lerp3(a0, b0, t), r0: lerp(ar0, br0, t), p1: lerp3(a1, b1, t), r1: lerp(ar1, br1, t), n0, n1 })
				}
				_ => None,
			})
			.collect();
		if let Some(prims) = blended {
			c.prims = prims;
			c.light = lerp_dir(p.light, c.light, t);
			c.band = [lerp(p.band[0], c.band[0], t), lerp(p.band[1], c.band[1], t)];
		}
	}
	o
}

fn blend_orbs(a: &[OrbDraw], b: &[OrbDraw], t: f32) -> Vec<OrbDraw> {
	let k = |o: &OrbDraw| (o.glass, o.pos);
	let pairs = pair_ordered(&a.iter().map(k).collect::<Vec<_>>(), &b.iter().map(k).collect::<Vec<_>>());
	b.iter()
		.zip(pairs)
		.map(|(o, j)| match j.map(|j| &a[j]) {
			Some(p) => OrbDraw { pos: lerp3(p.pos, o.pos, t), bob: lerp(p.bob, o.bob, t), parent: std::array::from_fn(|r| lerp3(p.parent[r], o.parent[r], t)), glass: o.glass },
			None => o.clone(),
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn steady_motion_is_no_cut() {
		let cam = |x: f32, yaw: f32| -> Cam { ([x, 0.0, 0.0], yaw, 0.0, 0.7) };
		// Hoverboard-like steady 0.8 m per field.
		let (mv, cut) = cam_cut(&cam(0.0, 0.0), &cam(0.8, 0.0), (0.8, 0.0, 0.0));
		assert!(!cut, "{mv:?}");
		// A mouse turn of 20 degrees in one field after a still one.
		assert!(!cam_cut(&cam(0.0, 0.0), &cam(0.0, 0.35), (0.0, 0.0, 0.0)).1);
		// Running speeds up from rest (0.25 m after 0).
		assert!(!cam_cut(&cam(0.0, 0.0), &cam(0.25, 0.0), (0.0, 0.0, 0.0)).1);
	}

	#[test]
	fn scene_cuts_and_teleports_are_cuts() {
		let cam = |x: f32, yaw: f32, tan: f32| -> Cam { ([x, 0.0, 0.0], yaw, 0.0, tan) };
		assert!(cam_cut(&cam(0.0, 0.0, 0.7), &cam(6.0, 0.0, 0.7), (0.1, 0.0, 0.0)).1, "eye jump");
		assert!(cam_cut(&cam(0.0, 0.0, 0.7), &cam(0.5, 1.6, 0.7), (0.1, 0.01, 0.0)).1, "turned 90 degrees");
		assert!(cam_cut(&cam(0.0, 0.0, 0.7), &cam(0.0, 0.0, 0.4), (0.0, 0.0, 0.0)).1, "zoom cut");
		// A slow scene zoom is not.
		assert!(!cam_cut(&cam(0.0, 0.0, 0.7), &cam(0.0, 0.0, 0.69), (0.0, 0.0, 0.015)).1);
	}

	#[test]
	fn ordered_pairs_follow_kind_and_distance() {
		let p = [(1u8, [0.0, 0.0, 0.0]), (2, [5.0, 0.0, 0.0]), (1, [10.0, 0.0, 0.0])];
		// A kind-2 item spawned in front: the kind-1 items still pair with theirs.
		let c = [(2u8, [50.0, 0.0, 0.0]), (1, [0.5, 0.0, 0.0]), (2, [5.2, 0.0, 0.0]), (1, [10.5, 0.0, 0.0])];
		// Kind 2: the first one in cur pairs with prev's only one, but it is 45 m away: no partner.
		assert_eq!(pair_ordered(&p, &c), vec![None, Some(0), None, Some(2)]);
	}

	#[test]
	fn sprite_blend_by_slot() {
		let s = |x: f32, tex: u32| crate::particles::Sprite { pos: [x, 0.0, 0.0], size: 1.0, rot: 0.0, colour: [1.0; 4], tex, additive: true, pool: Some(0), flat: false };
		let a = RetailFx { sprites: vec![s(0.0, 3), s(1.0, 3)], sprite_slots: vec![7, 9], ..Default::default() };
		let b = RetailFx { sprites: vec![s(2.0, 3), s(3.0, 4)], sprite_slots: vec![7, 9], ..Default::default() };
		let o = blend_fx(&a, &b, 0.25, &Default::default());
		assert_eq!(o.sprites[0].pos, [0.5, 0.0, 0.0]);
		// Slot 9 shows another effect now: drawn as the field has it.
		assert_eq!(o.sprites[1].pos, [3.0, 0.0, 0.0]);
		// t = 1 is the newest field exactly.
		let o = blend_fx(&a, &b, 1.0, &Default::default());
		assert_eq!(o.sprites[0].pos, b.sprites[0].pos);
	}

	#[test]
	fn standing_sheet_is_not_blended() {
		use crate::retail::efx_draw::FOG_ON;
		// A row of sheet triangles (p3 == p2) with fog F in the high bits of `tex`, as the effects layer gives them.
		let tri = |x: f32, f: usize| WorldQuad {
			pos: [[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x, 0.0, 1.0], [x, 0.0, 1.0]],
			uv: [[0.0; 2]; 4],
			colour: [[1.0; 4]; 4],
			tex: 42 | FOG_ON | f << 24,
			additive: false,
		};
		let prev: Vec<WorldQuad> = (0..6).map(|i| tri(i as f32 * 2.0, 0x40 + i)).collect();
		// Next field: the camera turned, so the fog changed, the first triangle is clipped and each comes rotated.
		let mut cur: Vec<WorldQuad> = (1..6).map(|i| tri(i as f32 * 2.0, 0x50 + i)).collect();
		for q in &mut cur {
			q.pos = [q.pos[1], q.pos[2], q.pos[0], q.pos[0]];
		}
		let still = still_kinds(&prev, &cur);
		assert!(still.contains(&(42, false)), "{still:?}");
		let mut out = cur.clone();
		lerp_quads(&prev, &mut out, 0.5, &quad_pairs(&prev, &cur));
		assert!(out.iter().zip(&cur).all(|(a, b)| a.pos == b.pos));
	}

	/// `still_kinds` (chained grid, decided kinds skip the search) answers what the plain rule does: a kind of at least 4 quads
	/// where at least half find the `same` quad of their kind within one 0.5 grid cell of their centre's.
	#[test]
	fn still_kinds_matches_plain_rule() {
		let mid = |q: &WorldQuad| std::array::from_fn::<f32, 3, _>(|c| (q.pos[0][c] + q.pos[1][c] + q.pos[2][c] + q.pos[3][c]) * 0.25);
		let cell = |p: [f32; 3]| ((p[0] / 0.5).floor() as i32, (p[1] / 0.5).floor() as i32, (p[2] / 0.5).floor() as i32);
		let close = |a: V3, b: V3| (0..3).all(|c| (a[c] - b[c]).abs() < STILL_EPS);
		let within = |a: &WorldQuad, b: &WorldQuad| a.pos.iter().all(|&p| b.pos.iter().any(|&q| close(p, q)));
		let plain = |prev: &[WorldQuad], cur: &[WorldQuad]| -> std::collections::HashSet<(usize, bool)> {
			let mut count: HashMap<(usize, bool), (u32, u32)> = HashMap::new();
			for q in cur {
				let (c, k) = (cell(mid(q)), kind_of(q));
				let hit = prev.iter().any(|p| {
					let d = cell(mid(p));
					kind_of(p) == k && (d.0 - c.0).abs() <= 1 && (d.1 - c.1).abs() <= 1 && (d.2 - c.2).abs() <= 1 && within(p, q) && within(q, p)
				});
				let e = count.entry(k).or_default();
				e.0 += hit as u32;
				e.1 += 1;
			}
			count.into_iter().filter(|&(_, (h, n))| n >= 4 && 2 * h >= n).map(|(k, _)| k).collect()
		};
		// Small deterministic noise; quads of 6 kinds, some standing (moved < STILL_EPS), some moving, some repeated corners.
		let mut seed = 0x2545_f491u32;
		let mut rnd = move || {
			seed ^= seed << 13;
			seed ^= seed >> 17;
			seed ^= seed << 5;
			(seed % 10_000) as f32 / 10_000.0
		};
		for round in 0..40 {
			let mut prev = Vec::new();
			let mut cur = Vec::new();
			for i in 0..(30 + round * 7) {
				let kind = i % 6;
				let o = [rnd() * 6.0, rnd() * 6.0, rnd() * 2.0];
				let mut pos = [o, [o[0] + 0.4, o[1], o[2]], [o[0], o[1] + 0.4, o[2]], [o[0], o[1] + 0.4, o[2]]];
				if i % 5 == 0 {
					pos[1] = pos[0];
				}
				let q = WorldQuad { pos, uv: [[0.0; 2]; 4], colour: [[1.0; 4]; 4], tex: kind / 2, additive: kind % 2 == 1 };
				prev.push(q);
				// Kinds 0-1 stand (jitter, corners rotated), 2-3 move a little or a lot, 4-5 come and go.
				let mv = match kind {
					0 | 1 => 0.15,
					2 | 3 => 0.3 + rnd(),
					_ => 0.2,
				};
				if kind >= 4 && rnd() < 0.4 {
					continue;
				}
				let mut c = q;
				for k in 0..4 {
					for a in 0..3 {
						c.pos[k][a] += (rnd() - 0.5) * 2.0 * mv;
					}
				}
				c.pos = [c.pos[2], c.pos[3], c.pos[0], c.pos[1]];
				cur.push(c);
			}
			assert_eq!(still_kinds(&prev, &cur), plain(&prev, &cur), "round {round}");
		}
	}
}
