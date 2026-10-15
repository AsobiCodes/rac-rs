//! Retail dynamic-light assignment and per-object evaluation (decomp/notes/lighting_fog.md section 5b and 7).
//!
//! The eight light slots are matched against the static objects (tie instances, tfrag fragments, shrub instances)
//! by bounding-sphere overlap; each object keeps up to four slot numbers as nibbles of a u16 (0xf = empty). Only
//! objects that hold a light (or were just freed of one) and are in view are relit, from their stored static colour.
//! Everything here is pure data so it tests without a GPU.

use crate::mesh::{Lit, LitKind, Mesh};
use crate::particles::{PointLight, LIGHT_SLOTS};

/// A light must move more than this to be re-assigned, and its sphere grows by it (FUN_00252418 / FUN_00252598).
pub const REACH: f32 = 8.0;
/// Shorts one light's object list holds (FUN_00252598 stops scanning when it is full).
pub const LIST_CAP: usize = 512;
/// Objects relit per kind per frame (the SPR relight lists, 0x3600..0x39fe).
pub const RELIGHT_CAP: usize = 511;

fn kind_index(k: LitKind) -> usize {
	match k {
		LitKind::Tie => 0,
		LitKind::Tfrag => 1,
		LitKind::Shrub => 2,
		LitKind::Moby => unreachable!("mobies are not assigned"),
	}
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A view frustum as six inward-facing planes (a, b, c, d: a x + b y + c z + d >= 0 inside).
#[derive(Clone, Copy, Debug)]
pub struct View {
	planes: [[f32; 4]; 6],
}

impl View {
	/// From a column-major view-projection with clip z in 0..1 (Gribb-Hartmann).
	pub fn from_matrix(m: &[[f32; 4]; 4]) -> View {
		// Row r of the matrix is (m[0][r], m[1][r], m[2][r], m[3][r]).
		let row = |r: usize| [m[0][r], m[1][r], m[2][r], m[3][r]];
		let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
		let add = |a: [f32; 4], b: [f32; 4], s: f32| -> [f32; 4] { std::array::from_fn(|i| a[i] + s * b[i]) };
		let mut planes = [add(r3, r0, 1.0), add(r3, r0, -1.0), add(r3, r1, 1.0), add(r3, r1, -1.0), r2, add(r3, r2, -1.0)];
		for p in &mut planes {
			let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-9);
			*p = p.map(|c| c / l);
		}
		View { planes }
	}

	/// FUN_00221d20: -1 wholly outside, 0 straddling a plane, 1 wholly inside.
	pub fn sphere(&self, c: [f32; 3], r: f32) -> i32 {
		let mut inside = true;
		for p in &self.planes {
			let d = p[0] * c[0] + p[1] * c[1] + p[2] * c[2] + p[3];
			if d < -r {
				return -1;
			}
			inside &= d >= r;
		}
		inside as i32
	}
}

/// One slot's light as the allocator last saw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotLight {
	pub light: PointLight,
	/// Bumped whenever the slot is (re)allocated, so a free and a re-allocation between two frames still counts as a
	/// new light.
	pub serial: u32,
}

#[derive(Clone, Default)]
struct Slot {
	used: bool,
	serial: u32,
	/// 1 = fresh, 2 = assigned (FUN_00252418).
	state: u8,
	last: [f32; 3],
	/// (kind index, object index) the slot's nibble was written to.
	list: Vec<(u8, u32)>,
}

/// Per-object assignment state.
#[derive(Clone, Copy, Debug)]
struct Entry {
	sphere: [f32; 4],
	nibbles: u16,
	dirty: bool,
}

/// The assignment tables: three object lists and the eight slot records.
pub struct Tracker {
	objs: [Vec<Entry>; 3],
	slots: [Slot; LIGHT_SLOTS],
}

impl Tracker {
	/// Objects as (bounding sphere, world units) per kind, in the game's array order (tie, tfrag, shrub).
	pub fn new(spheres: [Vec<[f32; 4]>; 3]) -> Tracker {
		let make = |v: &Vec<[f32; 4]>| v.iter().map(|&sphere| Entry { sphere, nibbles: 0xffff, dirty: false }).collect();
		Tracker { objs: [make(&spheres[0]), make(&spheres[1]), make(&spheres[2])], slots: Default::default() }
	}

	/// Slot numbers an object holds, lowest nibble first.
	pub fn held(&self, kind: LitKind, i: usize) -> Vec<usize> {
		let n = self.objs[kind_index(kind)][i].nibbles;
		(0..4).map(|k| (n >> (4 * k)) & 0xf).filter(|&s| s != 0xf).map(|s| s as usize).collect()
	}


	/// FUN_00252598: records `slot` in every overlapping object's lowest empty nibble, ties then tfrags then shrubs,
	/// until the 512-entry list is full. An object with all four nibbles taken is skipped.
	fn assign(&mut self, slot: usize, pos: [f32; 3], radius: f32) {
		let reach = radius + REACH;
		let mut list = Vec::new();
		'scan: for (k, objs) in self.objs.iter_mut().enumerate() {
			for (i, o) in objs.iter_mut().enumerate() {
				if list.len() >= LIST_CAP {
					break 'scan;
				}
				let d = sub(o.sphere[..3].try_into().unwrap(), pos);
				let r = reach + o.sphere[3];
				if dot(d, d) >= r * r {
					continue;
				}
				// The object is recorded only when a nibble was free.
				if let Some(n) = (0..4).find(|n| (o.nibbles >> (4 * n)) & 0xf == 0xf) {
					o.nibbles = (o.nibbles & !(0xf << (4 * n))) | (slot as u16) << (4 * n);
					list.push((k as u8, i as u32));
				}
			}
		}
		self.slots[slot].list = list;
	}

	/// FUN_00252978's list walk: removes the slot's nibble from each object (higher nibbles shift down, 0xf fills the
	/// top); an object left holding nothing is marked dirty so it is relit without the light.
	fn unassign(&mut self, slot: usize) {
		for (k, i) in std::mem::take(&mut self.slots[slot].list) {
			let o = &mut self.objs[k as usize][i as usize];
			let mut out = 0xffffu16;
			let mut at = 0;
			for n in 0..4 {
				let v = (o.nibbles >> (4 * n)) & 0xf;
				if v != slot as u16 && v != 0xf {
					out = (out & !(0xf << (4 * at))) | v << (4 * at);
					at += 1;
				}
			}
			o.nibbles = out;
			if out == 0xffff {
				o.dirty = true;
			}
		}
	}

	/// FUN_00252418, once per frame: follows the slot table. A light that moved more than 8.0 from where it was last
	/// assigned is (re)assigned (a fresh slot starts at the origin, so a light within 8.0 of it is not assigned
	/// until it moves).
	pub fn step(&mut self, table: &[Option<SlotLight>; LIGHT_SLOTS]) {
		for (s, entry) in table.iter().enumerate() {
			let stale = match (&entry, self.slots[s].used) {
				(None, true) => true,
				(Some(e), true) => e.serial != self.slots[s].serial,
				_ => false,
			};
			if stale {
				self.unassign(s);
				self.slots[s] = Slot::default();
			}
			let Some(e) = entry else { continue };
			if !self.slots[s].used {
				self.slots[s] = Slot { used: true, serial: e.serial, state: 1, last: [0.0; 3], list: Vec::new() };
			}
			let d = sub(e.light.pos, self.slots[s].last);
			if dot(d, d) <= REACH * REACH {
				continue;
			}
			self.slots[s].last = e.light.pos;
			match self.slots[s].state {
				1 => {
					self.assign(s, e.light.pos, e.light.radius);
					self.slots[s].state = 2;
				}
				2 => {
					// FUN_00252948: free, then assign again.
					self.unassign(s);
					self.assign(s, e.light.pos, e.light.radius);
				}
				_ => {}
			}
		}
	}

	/// The visibility pass: objects in view that are dirty or hold a light, at most 511 per kind, in array order.
	/// Taking an object clears its dirty flag (the Light* routines do).
	pub fn relight_list(&mut self, kind: LitKind, visible: impl Fn(usize) -> bool) -> Vec<usize> {
		let mut out = Vec::new();
		for (i, o) in self.objs[kind_index(kind)].iter_mut().enumerate() {
			if out.len() >= RELIGHT_CAP {
				break;
			}
			if visible(i) && (o.dirty || o.nibbles != 0xffff) {
				o.dirty = false;
				out.push(i);
			}
		}
		out
	}
}

/// The merged third light of a tie or shrub instance (LightTies / LightShrubs): colour, wrap and the travel direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Third {
	pub col: [f32; 3],
	pub wrap: f32,
	pub dir: [f32; 3],
}

/// Merges the lights an instance holds at its bounding-sphere centre `p`: each light in range adds rgb f and
/// intensity f (f = 1 - d/r) and a unit direction (p - L)/d; the direction is renormalised for two or more lights.
pub fn merge(p: [f32; 3], held: &[usize], table: &[Option<SlotLight>; LIGHT_SLOTS]) -> Option<Third> {
	let mut t = Third { col: [0.0; 3], wrap: 0.0, dir: [0.0; 3] };
	let mut n = 0;
	for &s in held {
		let Some(l) = table.get(s).and_then(|l| l.as_ref()).map(|l| l.light) else { continue };
		let v = sub(p, l.pos);
		let d = dot(v, v).sqrt();
		if d >= l.radius || d < 1e-6 {
			continue;
		}
		let f = 1.0 - d / l.radius;
		for j in 0..3 {
			t.col[j] += l.rgb[j] * f;
			t.dir[j] += v[j] / d;
		}
		t.wrap += l.intensity * f;
		n += 1;
	}
	if n == 0 {
		return None;
	}
	if n >= 2 {
		let l = dot(t.dir, t.dir).sqrt().max(1e-9);
		t.dir = t.dir.map(|c| c / l);
	}
	Some(t)
}

impl Third {
	/// What the third light adds to a vertex of normal `n`, in PS2 bytes (prog1: 128 col max(q, q wrap), q = n . -dir).
	pub fn bytes(&self, n: [f32; 3]) -> [f32; 3] {
		let q = -dot(n, self.dir);
		let q = q.max(q * self.wrap);
		self.col.map(|c| 128.0 * c * q)
	}
}

/// What the held lights add to a tfrag vertex at `p` with normal `n` (LightTfrags), in PS2 bytes: per light
/// q = n . (L - p)/d (1 - d/r), q' = max(q, q I), rgb q'.
pub fn tfrag_bytes(p: [f32; 3], n: [f32; 3], held: &[usize], table: &[Option<SlotLight>; LIGHT_SLOTS]) -> [f32; 3] {
	let mut s = [0.0; 3];
	for &k in held {
		let Some(l) = table.get(k).and_then(|l| l.as_ref()).map(|l| l.light) else { continue };
		let v = sub(l.pos, p);
		let d = dot(v, v).sqrt();
		if d >= l.radius || d < 1e-6 {
			continue;
		}
		let q = dot(n, v) / d * (1.0 - d / l.radius);
		let q = q.max(q * l.intensity);
		for j in 0..3 {
			s[j] += 128.0 * l.rgb[j] * q;
		}
	}
	s
}

/// The light a moby takes at its origin (MobyProc): each light with d < r adds rgb w (w = 1 - I d) and (p - L)/d w to
/// the direction; one light keeps its own direction, several are normalised. None when nothing reaches.
pub fn moby_light(p: [f32; 3], table: &[Option<SlotLight>; LIGHT_SLOTS]) -> Option<([f32; 3], [f32; 3])> {
	let (mut col, mut dir, mut n) = ([0.0f32; 3], [0.0f32; 3], 0);
	for l in table.iter().flatten().map(|l| l.light) {
		let v = sub(p, l.pos);
		let d = dot(v, v).sqrt();
		if d >= l.radius || d < 1e-6 {
			continue;
		}
		let w = 1.0 - l.intensity * d;
		for j in 0..3 {
			col[j] += l.rgb[j] * w;
			dir[j] += v[j] / d * w;
		}
		n += 1;
	}
	if n == 0 {
		return None;
	}
	let l = dot(dir, dir).sqrt();
	if l < 1e-9 {
		return None;
	}
	Some((col, dir.map(|c| c / l)))
}

/// One lit object: its vertices in the shared world vertex buffer and what shading it needs.
struct Object {
	first: usize,
	lit: Lit,
	/// Vertex positions (tfrags light per vertex; ties and shrubs light at the sphere centre).
	positions: Vec<[f32; 3]>,
}

/// The lit world: every mesh that carries `Lit` data, the assignment `Tracker`, and the relight pass.
pub struct LitWorld {
	tracker: Tracker,
	objects: Vec<Object>,
	/// Object indices per kind, in array order.
	by_kind: [Vec<usize>; 3],
}

/// A relit object: its first vertex in the world buffer and the new colours (PS2 bytes, alpha untouched): one per vertex,
/// except a tie, whose 64 colour slots are returned and recoloured per vertex by the viewer (BRIDGE-32). `kind` and `at`
/// are the object's kind and index within it (a tie's `at` is its instance index).
pub struct Relit {
	pub first: usize,
	pub colours: Vec<[f32; 4]>,
	pub kind: LitKind,
	pub at: usize,
}

impl LitWorld {


	/// `chain` with the terrain fragments first as per-position objects (REN-CULL-TFRAG): each `tfrags` mesh has one vertex per
	/// position record and occupies no address in the vertex space, so its `Relit.colours` are per position and `first` is unused.
	pub fn chain_tfrags<'a, 'b>(tfrags: &[&Mesh], world: impl IntoIterator<Item = &'a Mesh>, ties: &[[&Mesh; 3]], tail: impl IntoIterator<Item = &'b Mesh>) -> LitWorld {
		let mut objects = Vec::new();
		let mut by_kind: [Vec<usize>; 3] = Default::default();
		let mut spheres: [Vec<[f32; 4]>; 3] = Default::default();
		let mut first = 0;
		let mut push = |m: &Mesh, first: usize| {
			if let Some(l) = m.lit.as_ref().filter(|l| l.kind != LitKind::Moby) {
				by_kind[kind_index(l.kind)].push(objects.len());
				spheres[kind_index(l.kind)].push(l.sphere);
				let positions = if l.kind == LitKind::Tfrag { m.positions.clone() } else { Vec::new() };
				objects.push(Object { first, lit: l.clone(), positions });
			}
		};
		for m in tfrags {
			push(m, 0);
		}
		for m in world {
			push(m, first);
			first += m.positions.len();
		}
		for t in ties {
			push(t[0], first);
			first += t.iter().map(|m| m.positions.len()).sum::<usize>();
		}
		for m in tail {
			push(m, first);
			first += m.positions.len();
		}
		LitWorld { tracker: Tracker::new(spheres), objects, by_kind }
	}


	/// Vertex colours of object `at` of `kind` under the table (PS2 bytes, rgb capped, alpha from the static colour).
	pub fn shade(&self, kind: LitKind, at: usize, table: &[Option<SlotLight>; LIGHT_SLOTS]) -> Vec<[f32; 4]> {
		let o = &self.objects[self.by_kind[kind_index(kind)][at]];
		let (l, held) = (&o.lit, self.tracker.held(kind, at));
		let third = match kind {
			LitKind::Tfrag => None,
			_ => merge([l.sphere[0], l.sphere[1], l.sphere[2]], &held, table),
		};
		(0..l.raw.len())
			.map(|i| {
				let add = match kind {
					LitKind::Tfrag => tfrag_bytes(o.positions[i], l.normals[i], &held, table),
					_ => third.map_or([0.0; 3], |t| t.bytes(l.normals[i])),
				};
				let r = l.raw[i];
				[(r[0] + add[0]).clamp(0.0, l.cap), (r[1] + add[1]).clamp(0.0, l.cap), (r[2] + add[2]).clamp(0.0, l.cap), r[3]]
			})
			.collect()
	}

	/// One frame: follow the light table, then relight the visible objects that hold a light or lost one. `view` None
	/// treats everything as visible.
	pub fn frame(&mut self, table: &[Option<SlotLight>; LIGHT_SLOTS], view: Option<&View>) -> Vec<Relit> {
		self.tracker.step(table);
		let mut out = Vec::new();
		for kind in [LitKind::Tie, LitKind::Shrub, LitKind::Tfrag] {
			let ids = self.by_kind[kind_index(kind)].clone();
			let objects = &self.objects;
			let visible = |at: usize| {
				let s = objects[ids[at]].lit.sphere;
				view.map_or(true, |v| v.sphere([s[0], s[1], s[2]], s[3]) != -1)
			};
			for at in self.tracker.relight_list(kind, visible) {
				let o = &self.objects[ids[at]];
				out.push(Relit { first: o.first, colours: self.shade(kind, at, table), kind, at });
			}
		}
		out
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn light(pos: [f32; 3], radius: f32, serial: u32) -> SlotLight {
		SlotLight { light: PointLight { pos, radius, rgb: [1.0, 0.5, 0.0], intensity: 0.0 }, serial }
	}

	fn table(l: &[(usize, SlotLight)]) -> [Option<SlotLight>; LIGHT_SLOTS] {
		let mut t = [None; LIGHT_SLOTS];
		for &(k, s) in l {
			t[k] = Some(s);
		}
		t
	}

	fn world() -> Tracker {
		// Two ties, one tfrag, one shrub in a row along x, 20 apart, radius 2.
		let s = |x: f32| [x, 0.0, 0.0, 2.0];
		Tracker::new([vec![s(100.0), s(120.0)], vec![s(140.0)], vec![s(160.0)]])
	}

	#[test]
	fn overlap_uses_radius_plus_reach() {
		let mut t = world();
		// Light radius 5 + 8 reach + sphere 2 = 15: the tie at 100 is hit from 86, not from 84.
		t.step(&table(&[(0, light([86.0, 0.0, 0.0], 5.0, 1))]));
		assert_eq!(t.held(LitKind::Tie, 0), vec![0]);
		assert!(t.held(LitKind::Tie, 1).is_empty());
		let mut t = world();
		t.step(&table(&[(0, light([84.0, 0.0, 0.0], 5.0, 1))]));
		assert!(t.held(LitKind::Tie, 0).is_empty());
	}

	#[test]
	fn fresh_light_near_the_origin_waits_until_it_moves() {
		let mut t = Tracker::new([vec![[0.0, 0.0, 0.0, 2.0]], vec![], vec![]]);
		t.step(&table(&[(3, light([5.0, 0.0, 0.0], 5.0, 1))]));
		assert!(t.held(LitKind::Tie, 0).is_empty(), "within 8.0 of the origin: never assigned");
		t.step(&table(&[(3, light([9.0, 0.0, 0.0], 5.0, 1))]));
		assert_eq!(t.held(LitKind::Tie, 0), vec![3]);
	}

	#[test]
	fn nibbles_fill_lowest_first_and_cap_at_four() {
		let mut t = Tracker::new([vec![[100.0, 0.0, 0.0, 2.0]], vec![], vec![]]);
		let all: Vec<_> = (0..5).map(|k| (k, light([100.0, 0.0, 0.0], 5.0, 1))).collect();
		t.step(&table(&all));
		// Slots 0..3 got the four nibbles in order, slot 4 found none.
		assert_eq!(t.held(LitKind::Tie, 0), vec![0, 1, 2, 3]);
		assert_eq!(t.objs[0][0].nibbles, 0x3210);
	}

	#[test]
	fn a_moved_light_is_reassigned_only_past_eight_units() {
		let mut t = Tracker::new([vec![[100.0, 0.0, 0.0, 2.0], [140.0, 0.0, 0.0, 2.0]], vec![], vec![]]);
		t.step(&table(&[(0, light([100.0, 0.0, 0.0], 5.0, 1))]));
		assert_eq!(t.held(LitKind::Tie, 0), vec![0]);
		// 6 units: no work, the sphere (radius 15 around the first assignment) stands.
		t.step(&table(&[(0, light([106.0, 0.0, 0.0], 5.0, 1))]));
		assert_eq!(t.held(LitKind::Tie, 0), vec![0]);
		assert!(t.held(LitKind::Tie, 1).is_empty());
		t.step(&table(&[(0, light([128.0, 0.0, 0.0], 5.0, 1))]));
		assert!(t.held(LitKind::Tie, 0).is_empty());
		assert_eq!(t.held(LitKind::Tie, 1), vec![0]);
	}

	#[test]
	fn list_is_capped_at_512_in_tie_tfrag_shrub_order() {
		let spheres: Vec<[f32; 4]> = (0..600).map(|_| [100.0, 0.0, 0.0, 2.0]).collect();
		let mut t = Tracker::new([spheres.clone(), spheres.clone(), vec![]]);
		t.step(&table(&[(0, light([100.0, 0.0, 0.0], 5.0, 1))]));
		assert_eq!(t.slots[0].list.len(), LIST_CAP);
		assert!((0..LIST_CAP).all(|i| t.held(LitKind::Tie, i) == vec![0]));
		assert!((LIST_CAP..600).all(|i| t.held(LitKind::Tie, i).is_empty()));
		assert_eq!((0..600).filter(|&i| !t.held(LitKind::Tfrag, i).is_empty()).count(), 0, "full before the tfrags");
	}

	#[test]
	fn relight_needs_view_and_a_light_or_dirt() {
		let mut t = world();
		assert!(t.relight_list(LitKind::Tie, |_| true).is_empty(), "lit once at load, nothing to do");
		t.step(&table(&[(0, light([110.0, 0.0, 0.0], 5.0, 1))]));
		assert_eq!(t.relight_list(LitKind::Tie, |_| false), Vec::<usize>::new());
		assert_eq!(t.relight_list(LitKind::Tie, |_| true), vec![0, 1], "every frame while it holds a light");
		assert_eq!(t.relight_list(LitKind::Tie, |_| true), vec![0, 1]);
		t.step(&table(&[]));
		// Freed while out of view: stays dirty until it is next seen, then relit once.
		assert!(t.relight_list(LitKind::Tie, |_| false).is_empty());
		assert_eq!(t.relight_list(LitKind::Tie, |_| true), vec![0, 1]);
		assert!(t.relight_list(LitKind::Tie, |_| true).is_empty());
	}

	#[test]
	fn relight_list_caps_at_511() {
		let mut t = Tracker::new([(0..600).map(|_| [100.0, 0.0, 0.0, 2.0]).collect(), vec![], vec![]]);
		t.step(&table(&[(0, light([100.0, 0.0, 0.0], 5.0, 1))]));
		assert_eq!(t.relight_list(LitKind::Tie, |_| true).len(), RELIGHT_CAP);
	}

	#[test]
	fn frustum_classifies_spheres() {
		// A symmetric projection looking down -z in clip space terms: identity-like box [-1,1]^2 x [0,1].
		let m = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
		let v = View::from_matrix(&m);
		assert_eq!(v.sphere([0.0, 0.0, 0.5], 0.1), 1);
		assert_eq!(v.sphere([1.0, 0.0, 0.5], 0.3), 0);
		assert_eq!(v.sphere([3.0, 0.0, 0.5], 0.3), -1);
		assert_eq!(v.sphere([0.0, 0.0, 5.0], 0.3), -1);
	}

	#[test]
	fn tie_merge_and_tfrag_terms() {
		let mut a = light([0.0, 0.0, 10.0], 10.0, 1);
		a.light.intensity = 0.5;
		let t = table(&[(0, a), (1, light([0.0, 0.0, 10.0], 20.0, 1))]);
		// P straight below both lights: f = 0.5 and 0.75, directions (0, 0, -1) each.
		let m = merge([0.0, 0.0, 5.0], &[0, 1], &t).unwrap();
		assert!((m.col[0] - 1.25).abs() < 1e-5, "rgb f summed: 0.5 + 0.75");
		assert!((m.dir[2] + 1.0).abs() < 1e-5, "renormalised travel direction (down)");
		assert!((m.wrap - 0.25).abs() < 1e-5);
		// An upward normal is lit by a light above it; a downward one only through wrap (none here).
		assert!(m.bytes([0.0, 0.0, 1.0])[0] > 0.0);
		assert_eq!(merge([0.0, 0.0, 50.0], &[0, 1], &t), None, "out of range");
		let up = tfrag_bytes([0.0, 0.0, 5.0], [0.0, 0.0, 1.0], &[1], &t);
		// d = 5, f = 0.75, q = 0.75, red 128 * 0.75.
		assert!((up[0] - 96.0).abs() < 1e-3 && (up[1] - 48.0).abs() < 1e-3);
		assert_eq!(tfrag_bytes([0.0, 0.0, 5.0], [0.0, 0.0, -1.0], &[1], &t), [0.0; 3]);
	}

	#[test]
	fn moby_third_light_at_the_origin() {
		let t = table(&[(0, light([0.0, 0.0, 10.0], 20.0, 1))]);
		let (col, dir) = moby_light([0.0, 0.0, 0.0], &t).unwrap();
		assert_eq!(col, [1.0, 0.5, 0.0]);
		assert!((dir[2] + 1.0).abs() < 1e-6);
		assert!(moby_light([0.0, 0.0, 100.0], &t).is_none());
	}
	fn lit_mesh(kind: LitKind, c: [f32; 3]) -> Mesh {
		let mut m = Mesh::new("m".into());
		m.positions = vec![c, [c[0] + 1.0, c[1], c[2]]];
		m.lit = Some(Lit { kind, sphere: [c[0], c[1], c[2], 2.0], origin: c, cap: 243.0, normals: vec![[0.0, 0.0, 1.0]; 2], raw: vec![[100.0, 100.0, 100.0, 128.0]; 2] });
		m
	}

}
