use super::volumes::*;
use super::*;
use crate::smoke_rows::{SmokeRow, ANY_TEX};

fn puff(x: f32, y: f32, z: f32, r: f32, thick: f32) -> Raw {
	Raw { pos: [x, y, z], radius: r, thick, albedo: [0.3; 3], share: 1.0, age: 0.1, rise: 1.0, billow: 0.4, tall: 1.0 }
}

fn quad() -> [Vertex; 4] {
	[Vertex { pos: [0.0; 3], uv: [0.0; 2], normal: [0.0; 3], colour: [1.0; 4], light: [0.0; 4] }; 4]
}

fn sprite(pos: [f32; 3]) -> crate::prims::Sprite {
	crate::prims::Sprite { pos, size: 2.0, rot: 0.0, colour: [0.4, 0.4, 0.4, 0.8], tex: 0, additive: false, pool: Some(0), flat: false }
}

const GREY: [f32; 4] = [0.4, 0.4, 0.4, 0.8];

static TABLE: [SmokeRow; 2] = [
	SmokeRow { kind: 4, tex: ANY_TEX, grey_only: true, radius: 0.5, density: 1.0, rise: 1.0, billow: 0.3 },
	SmokeRow { kind: 6, tex: ANY_TEX, grey_only: false, radius: 0.5, density: 1.0, rise: 1.0, billow: 0.3 },
];

/// The assembled WGSL compiles (naga) with its entry points and validates.
#[test]
fn smoke_shader_parses() {
	let m = naga::front::wgsl::parse_str(&shader::built_in()).expect("volsmoke shader");
	let names: Vec<_> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
	for n in ["vs_full", "fs_smoke", "fs_resolve", "fs_apply"] {
		assert!(names.contains(&n), "{n}");
	}
	let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
	v.validate(&m).expect("volsmoke shader validates");
}

/// The Rust records are the WGSL structs: the fog's uniform is untouched, the volume 80 bytes, the extra block seven vec4.
#[test]
fn uniform_sizes_match_the_blocks() {
	assert_eq!(std::mem::size_of::<volfog::Uniform>(), 4 * 16 * 2 + 3 * 64 + 17 * 16);
	assert_eq!(std::mem::size_of::<Vol>(), 80);
	assert_eq!(std::mem::size_of::<Extra>(), 7 * 16);
	assert_eq!(MAX_VOLS * std::mem::size_of::<Vol>(), 5120);
}

#[test]
fn quality_levels() {
	assert_eq!(quality(0), None);
	let (l, m, h) = (quality(1).unwrap(), quality(2).unwrap(), quality(3).unwrap());
	assert!(h.steps > m.steps && m.steps > l.steps);
	assert!(h.max > m.max && m.max > l.max && h.max <= MAX_VOLS);
	assert!(h.taps > m.taps && m.taps > l.taps);
	assert!(!l.sun_map && m.sun_map && h.sun_map);
	assert!(l.div >= m.div);
	assert_eq!(quality(9), quality(3));
}

/// More puffs than the cap merge into at most `cap` volumes that keep the optical mass; few puffs stay as they are.
#[test]
fn merge_fits_the_cap_and_keeps_mass() {
	let raw: Vec<Raw> = (0..300).map(|i| puff((i % 20) as f32 * 0.7, (i / 20) as f32 * 0.7, 3.0 + (i % 7) as f32 * 0.3, 1.0, 0.8)).collect();
	let mass = |v: &[Raw]| v.iter().map(|r| r.thick * r.radius.powi(3)).sum::<f32>();
	let step = want_step(&raw, 24);
	assert!(step > 0 && cluster_count(&raw, CELLS[step]) <= 24);
	let merged = merge(&raw, CELLS[step]);
	assert!(merged.len() <= 24);
	// The mass is conserved except where a cluster hit the thickness cap (4x the thickest member) or the radius cap.
	assert!(mass(&merged) <= mass(&raw) * 1.001 && mass(&merged) > mass(&raw) * 0.4, "{} vs {}", mass(&merged), mass(&raw));
	let few: Vec<Raw> = raw[..5].iter().map(|r| Raw { pos: [r.pos[0] * 40.0, 0.0, 0.0], ..*r }).collect();
	assert_eq!(want_step(&few, 24), 0);
	let cam = [0.0, -10.0, 3.0];
	let out = reduce(&raw, (cam, [0.0, 1.0, 0.0], 150.0), 24, 90, &mut Track::default()).vols;
	assert!(out.len() <= 24 && !out.is_empty());
	assert!(out.windows(2).all(|w| importance(&w[0], cam) >= importance(&w[1], cam)));
}

/// The merge step rises at once and falls one step after `hold` quiet frames.
#[test]
fn ladder_has_hysteresis() {
	let mut l = Ladder::default();
	assert_eq!(l.update(3, 90), 3);
	for _ in 0..89 {
		assert_eq!(l.update(0, 90), 3);
	}
	assert_eq!(l.update(0, 90), 2);
	assert_eq!(l.update(2, 90), 2);
	for _ in 0..89 {
		l.update(1, 90);
	}
	assert_eq!(l.update(1, 90), 1);
	assert_eq!(l.update(4, 90), 4);
}

#[test]
fn cull_drops_behind_and_far() {
	let raw = [puff(0.0, 10.0, 0.0, 1.0, 1.0), puff(0.0, -10.0, 0.0, 1.0, 1.0), puff(0.0, 500.0, 0.0, 1.0, 1.0), puff(0.0, 10.0, 0.0, 1.0, 0.0)];
	let kept = cull(&raw, [0.0; 3], [0.0, 1.0, 0.0], 150.0);
	assert_eq!(kept.len(), 1);
	assert_eq!(kept[0].pos[1], 10.0);
}

/// Fire lights the smoke near it: warm glow falls with the distance from the fire per sample, is capped, and is zero away from any light.
#[test]
fn glow_follows_the_fire_lights() {
	let lights: [Light; 1] = [([0.0, 0.0, 0.0], 6.0, [1.0, 0.5, 0.1])];
	let g = glow([0.0, 0.0, 1.0], 1.0, &lights);
	assert_eq!(g.1, [0.0, 0.0, 0.0, 6.0]);
	let (near, far) = (glow_at(g, [0.0, 0.0, 1.0]), glow_at(g, [0.0, 0.0, 5.0]));
	assert!(near[0] > far[0] && far[0] > 0.0 && near[0] > near[1] && near[1] > near[2]);
	assert_eq!(glow_at(g, [0.0, 0.0, 7.0]), [0.0; 3]);
	assert_eq!(glow([0.0, 0.0, 50.0], 1.0, &lights), ([0.0; 3], [0.0; 4]));
	assert_eq!(glow([0.0; 3], 1.0, &[]), ([0.0; 3], [0.0; 4]));
	let strong: [Light; 1] = [([0.0; 3], 6.0, [9.0, 9.0, 9.0])];
	assert_eq!(glow([0.0; 3], 1.0, &strong).0, [2.0; 3]);
	// Several warm lights over the cap keep their hue (a cap per channel made them yellow-green: R = G = 2).
	let warm: [Light; 4] = [([0.0; 3], 6.0, [1.8, 0.9, 0.2]); 4];
	let g = glow([0.0; 3], 1.0, &warm).0;
	assert!((g[0] - 2.0).abs() < 1e-5 && (g[1] - 1.0).abs() < 1e-5 && (g[2] - 2.0 * 0.2 / 1.8).abs() < 1e-5, "{g:?}");
	// A volume far bigger than the light's reach glows only near the fire: its far side and top stay unlit (no evenly orange ball).
	let huge = glow([0.0, 0.0, 30.0], 40.0, &lights);
	assert!(glow_at(huge, [0.0, 0.0, 30.0]) == [0.0; 3] && glow_at(huge, [0.0, 0.0, 1.0])[0] > 0.5);
	// Two lights: the centre lies between them, nearer the brighter one.
	let two: [Light; 2] = [([0.0; 3], 4.0, [2.0, 1.0, 0.2]), ([4.0, 0.0, 0.0], 4.0, [0.5, 0.25, 0.05])];
	let c = glow([2.0, 0.0, 0.0], 1.0, &two).1;
	assert!(c[0] > 0.0 && c[0] < 2.0 && c[3] == 4.0, "{c:?}");
	let v = to_vols(&[puff(0.0, 0.0, 1.0, 1.0, 1.0)], &lights);
	assert!(v[0].glow[0] > 0.0 && v[0].lp[3] == 6.0 && (v[0].col[3] - 1.5).abs() < 1e-6 && v[0].c[3] == 1.0);
}

/// Smoke Off (no frame, no pool types, no rows): the plan is empty and takes nothing, so every sprite is drawn as before.
#[test]
fn off_path_touches_nothing() {
	let mut plan = Plan::new(&[sprite([0.0; 3])], None, [0.0; 3]);
	assert!(plan.raw.is_empty() && plan.held.is_empty());
	assert!(!plan.take(0, &sprite([0.0; 3]), 2.0, GREY, (1, false, 0), quad()));
	assert!(plan.held.is_empty());
	let off = Plan::off();
	assert!(off.raw.is_empty() && off.held.is_empty());
	let mut s = VolSmoke::new(wgpu::TextureFormat::Rgba8Unorm);
	s.finish(off);
	assert!(!s.ready());
}

/// A smoke-type sprite becomes a volume and its quad is held; a saturated (orange fire ball) sprite of a grey-only row and a
/// sprite of another type stay drawn; a puff beyond the range stays drawn.
#[test]
fn take_holds_smoke_and_leaves_fire() {
	let mut plan = Plan::with_rows(vec![0, 0, u8::MAX, 1, 1], &TABLE, [0.0; 3]);
	let s = sprite([2.0, 8.0, 3.0]);
	assert!(plan.take(0, &s, 2.0, [0.9, 0.9, 0.9, 0.8], (1, false, 2), quad()));
	assert!(!plan.take(1, &s, 2.0, [1.9, 0.96, 0.21, 0.8], (2, true, 3), quad()), "the orange ball stays a sprite");
	assert!(!plan.take(2, &s, 2.0, [0.5, 0.5, 0.5, 0.8], (3, false, 4), quad()), "not a smoke type");
	assert!(plan.take(3, &s, 2.0, [1.9, 0.96, 0.21, 0.8], (4, false, 5), quad()), "a non grey-only row takes any colour");
	assert!(!plan.take(4, &sprite([0.0, 400.0, 0.0]), 2.0, GREY, (5, false, 6), quad()), "beyond the range");
	assert_eq!((plan.raw.len(), plan.held.len()), (2, 2));
	assert!((plan.raw[0].radius - 1.0).abs() < 1e-6 && plan.raw[0].albedo[0] < 0.9 * 0.75 + 1e-4 && plan.raw[0].share == 1.0);
	// A faded sprite (alpha < 0.02) is hidden without a volume.
	assert!(plan.take(3, &s, 2.0, [0.4, 0.4, 0.4, 0.01], (6, false, 6), quad()));
	assert_eq!((plan.raw.len(), plan.held.len()), (2, 3));
}

/// A camera at the origin looking along +y.
fn eye() -> Camera {
	Camera { pos: [0.0; 3], yaw: std::f32::consts::FRAC_PI_2, pitch: 0.0, tan_half: 0.75, up: [0.0, 0.0, 1.0] }
}

/// A quad whose vertices carry `x` (tells quads apart).
fn quad_x(x: f32) -> [Vertex; 4] {
	let mut q = quad();
	q.iter_mut().for_each(|v| v.pos[0] = x);
	q
}

type Verts = std::collections::BTreeMap<plan::Key, Vec<Vertex>>;

/// When the volumes cannot draw the held quads go back into the particle pass; when they draw they are dropped.
#[test]
fn settle_gives_quads_back_or_drops_them() {
	let held_plan = || {
		let mut plan = Plan::with_rows(vec![1], &TABLE, [0.0; 3]);
		assert!(plan.take(0, &sprite([0.0, 8.0, 0.0]), 2.0, GREY, (1, false, 2), quad()));
		plan
	};
	let mut s = VolSmoke::new(wgpu::TextureFormat::Rgba8Unorm);
	s.finish(held_plan());
	assert!(s.raw.len() == 1 && s.held.len() == 1);
	let mut verts = Verts::new();
	s.settle_cap(true, &eye(), 24, &mut verts);
	assert!(verts.is_empty() && s.vols.len() == 1, "the volume draws, the sprite stays out");
	s.settle_cap(false, &eye(), 24, &mut verts);
	assert_eq!(verts.get(&(1, false, 2)).map(|v| v.len()), Some(4));
	assert!(s.vols.is_empty(), "no volumes this pass");
}

/// Split view: the Classic pass settles `false` and the Enhanced pass of the same frame settles `true` on the same `finish`: the
/// volumes and the held quads survive the first pass, and a quad given back is taken out again (in either order, repeated).
#[test]
fn the_split_views_two_passes_each_get_what_they_need() {
	let mut plan = Plan::with_rows(vec![1, 1], &TABLE, [0.0; 3]);
	assert!(plan.take(0, &sprite([0.0, 8.0, 0.0]), 2.0, GREY, (1, false, 2), quad_x(1.0)));
	assert!(plan.take(1, &sprite([0.0, 9.0, 0.0]), 2.0, GREY, (1, false, 2), quad_x(2.0)));
	let mut s = VolSmoke::new(wgpu::TextureFormat::Rgba8Unorm);
	s.finish(plan);
	let mut verts = Verts::new();
	let quads = |v: &Verts| v.get(&(1, false, 2)).map_or(0, |q| q.len());
	for _ in 0..2 {
		s.settle_cap(false, &eye(), 24, &mut verts);
		assert_eq!(quads(&verts), 8, "the Classic half draws both sprites, however often it settles");
		s.settle_cap(true, &eye(), 24, &mut verts);
		assert!(verts.is_empty(), "the Enhanced half draws volumes instead of the sprites");
		assert_eq!((s.vols.len(), s.raw.len(), s.held.len()), (2, 2, 2), "the Classic pass did not drain the Enhanced half's volumes");
	}
	s.settle_cap(false, &eye(), 24, &mut verts);
	s.settle_cap(false, &eye(), 24, &mut verts);
	assert_eq!(quads(&verts), 8, "twice in a row gives the sprites back once");
	s.finish(Plan::off());
	assert!(s.held.is_empty() && s.given.is_empty());
}

/// A22: 30 far-apart puffs under a cap of 24: the 24 most important draw as volumes, the 6 cut ones keep their retail sprite.
#[test]
fn a_volume_cut_by_the_cap_keeps_its_retail_sprite() {
	let spots: Vec<[f32; 3]> = (0..30).map(|i| [-50.0 + (i % 6) as f32 * 20.0, 20.0 + (i / 6) as f32 * 20.0, 3.0]).collect();
	let mut plan = Plan::with_rows(vec![1; 30], &TABLE, [0.0; 3]);
	for (i, p) in spots.iter().enumerate() {
		assert!(plan.take(i, &sprite(*p), 2.0, GREY, (1, false, 2), quad_x(i as f32)));
	}
	let mut s = VolSmoke::new(wgpu::TextureFormat::Rgba8Unorm);
	s.finish(plan);
	let mut verts = Verts::new();
	s.settle_cap(true, &eye(), 24, &mut verts);
	assert_eq!(s.vols.len(), 24);
	let given: Vec<f32> = verts.values().flatten().map(|v| v.pos[0]).collect();
	assert_eq!(given.len(), 6 * 4, "the six cut puffs are drawn as sprites");
	for (i, p) in spots.iter().enumerate() {
		let drawn = s.vols.iter().any(|v| v.pos == *p);
		assert_ne!(drawn, given.contains(&(i as f32)), "puff {i} is a volume or a sprite, never neither and never both");
	}
	let again = s.vols.clone();
	s.settle_cap(true, &eye(), 24, &mut verts);
	assert_eq!((verts.values().map(Vec::len).sum::<usize>(), s.vols.len()), (24, again.len()), "a second pass does not give the sprites back twice");
}

/// A22 for the level fires' quads: a quad whose volume the cap cut is un-taken (the quad pass draws it again).
#[test]
fn a_quad_volume_cut_by_the_cap_is_drawn_as_a_quad_again() {
	let mut s = VolSmoke::new(wgpu::TextureFormat::Bgra8Unorm);
	s.set_quads(columns::Gathered { raw: vec![puff(0.0, 10.0, 0.0, 1.0, 1.0), puff(0.0, 50.0, 0.0, 1.0, 1.0), puff(0.0, 100.0, 0.0, 1.0, 1.0)], src: vec![0, 2, 3], took: vec![true, false, true, true], ..Default::default() });
	let mut verts = Verts::new();
	s.settle_cap(true, &eye(), 2, &mut verts);
	assert_eq!((0..4).map(|i| s.took(i)).collect::<Vec<_>>(), vec![true, false, true, false], "only the farthest quad is given back");
	assert_eq!(s.vols.len(), 2);
}

/// A22 / review MED-1: a plume's cut puff gives back EVERY quad of its plume, not only the quad its puff maps to.
#[test]
fn a_cut_plume_gives_back_all_its_quads() {
	let mut s = VolSmoke::new(wgpu::TextureFormat::Bgra8Unorm);
	s.set_quads(columns::Gathered { raw: vec![puff(0.0, 10.0, 0.0, 1.0, 1.0), puff(0.0, 100.0, 0.0, 1.0, 1.0)], src: vec![0, 2], grp: vec![0, 1], groups: vec![vec![0, 1], vec![2, 3, 4]], took: vec![true; 5] });
	let mut verts = Verts::new();
	s.settle_cap(true, &eye(), 1, &mut verts);
	assert_eq!((0..5).map(|i| s.took(i)).collect::<Vec<_>>(), vec![true, true, false, false, false], "the farthest plume's three quads all come back");
}

/// DX12 parity: the smoke shader must also go through naga's HLSL writer (wgpu's dx12 feature builds naga's `hlsl-out` on Windows;
/// the same check as metalenv::tests::level_shader_writes_hlsl). It does not run FXC; the picture pass checks that on a DX12 run.
#[cfg(windows)]
#[test]
fn smoke_shader_writes_hlsl() {
	let src = shader::built_in();
	let m = naga::front::wgsl::parse_str(&src).expect("wgsl parse");
	let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	let (options, pipeline) = (naga::back::hlsl::Options::default(), naga::back::hlsl::PipelineOptions::default());
	let mut out = String::new();
	naga::back::hlsl::Writer::new(&mut out, &options, &pipeline).write(&m, &info, None).expect("HLSL writes");
	assert!(out.contains("fs_smoke") && out.contains("fs_apply"), "the smoke entry points are in the HLSL");
}

/// The quads of a level fire: a frame with only smoke quads is ready, takes them, and gives them back when the volumes cannot draw.
#[test]
fn smoke_quads_make_the_pass_ready_and_settle_gives_them_back() {
	let mut s = VolSmoke::new(wgpu::TextureFormat::Bgra8Unorm);
	assert!(!s.took(0), "nothing taken before a frame");
	s.set_quads(columns::Gathered { raw: vec![puff(0.0, 10.0, 1.0, 1.0, 1.0)], src: vec![1], took: vec![false, true], ..Default::default() });
	assert!(s.took(1) && !s.took(0) && !s.took(7));
	// (ready() also reads the global option: only the quad bookkeeping is asserted here.)
	let mut verts = Verts::new();
	s.settle_cap(true, &eye(), 24, &mut verts);
	assert!(s.took(1), "the volumes draw: the quads stay out of the quad pass");
	s.settle_cap(false, &eye(), 24, &mut verts);
	assert!(!s.took(1) && s.qraw.is_empty(), "no volumes this frame: the quads draw as retail");
}

/// The quads' volumes join the pool's puffs in the frame's volume list (borrowed as they are when there are no quad volumes).
#[test]
fn quad_volumes_join_the_pool_puffs() {
	let pool = [puff(0.0, 0.0, 1.0, 1.0, 1.0)];
	let quads = [puff(2.0, 0.0, 1.0, 1.0, 1.0), puff(4.0, 0.0, 1.0, 1.0, 1.0)];
	assert!(matches!(joined(&pool, &[]), std::borrow::Cow::Borrowed(_)));
	let all = joined(&pool, &quads);
	assert_eq!(all.len(), 3);
	assert_eq!((all[0].pos[0], all[2].pos[0]), (0.0, 4.0));
	assert_eq!(volumes::cull(&all, [0.0; 3], [1.0, 0.0, 0.0], 150.0).len(), 3);
}

/// A8 / A23: when a second puff joins a lone one, the cluster's centre eases toward the new centroid instead of jumping; its share
/// is the drawn mass over the mass (a band puff joining a full one: between them, drawn mass kept).
#[test]
fn a_cluster_eases_when_a_puff_joins() {
	let view = ([0.0, -10.0, 3.0], [0.0, 1.0, 0.0], 150.0);
	let a = puff(2.0, 5.0, 3.0, 1.0, 1.0);
	let b = Raw { share: 0.5, ..puff(3.0, 5.0, 3.0, 1.2, 1.0) };
	let mut track = Track::default();
	let one = reduce(&[a], view, 1, 90, &mut track).vols;
	assert_eq!(one[0].pos[0], 2.0);
	let two = reduce(&[a, b], view, 1, 90, &mut track).vols;
	assert_eq!(two.len(), 1);
	let centroid = 2.0 + 0.5 * 1.728 / 1.864;
	assert!(two[0].pos[0] > 2.1 && two[0].pos[0] < (2.0 + centroid) * 0.5 + 0.01, "eased half way to the centroid {centroid}: {}", two[0].pos[0]);
	let fresh = reduce(&[a, b], view, 1, 90, &mut Track::default()).vols[0];
	assert!((fresh.share - 1.864 / 2.728).abs() < 1e-4, "share = drawn mass / mass: {}", fresh.share);
}

/// Review R29 M1: a puff joining a cluster at share ~0 does not grow its envelope at once (the reach takes it by its share below a
/// quarter of the cluster's largest); at a full share the same puff does, and members that all share one share keep the envelope.
#[test]
fn a_puff_joining_at_share_zero_does_not_grow_the_cluster() {
	let (a, b) = (puff(2.0, 3.0, 2.0, 1.0, 1.0), puff(3.0, 3.5, 2.5, 1.2, 1.0));
	let c = puff(6.5, 1.0, 6.0, 1.5, 0.8);
	let base = merge(&[a, b], 8.0)[0];
	let with = |share: f32| merge(&[a, b, Raw { share, ..c }], 8.0)[0];
	let near = |x: f32, y: f32| (x - y).abs() < 1.0e-3 * y.abs().max(1.0);
	let joined = with(1.0e-5);
	assert!(near(joined.radius, base.radius) && near(joined.tall, base.tall), "{joined:?} vs {base:?}");
	assert!((0..3).all(|k| near(joined.pos[k], base.pos[k])) && near(drawn(&joined), drawn(&base)), "{joined:?} vs {base:?}");
	let whole = with(1.0);
	assert!(whole.radius > base.radius + 0.5 && whole.radius * whole.tall > base.radius * base.tall + 0.5, "the test bites: {whole:?}");
	// One share for all: the knee is 1 for every member, the envelope is the unshared one (the centroid's weights round alike).
	let half = |r: Raw| Raw { share: 0.3, ..r };
	let (h, f) = (merge(&[half(a), half(b), half(c)], 8.0)[0], merge(&[a, b, c], 8.0)[0]);
	assert!(near(h.radius, f.radius) && near(h.tall, f.tall) && (0..3).all(|k| near(h.pos[k], f.pos[k])), "{h:?} vs {f:?}");
}

/// `follow`: a merged volume moves half way per frame (its share too); a lone one that stays lone is exact.
#[test]
fn follow_eases_merged_volumes_only() {
	let last = vec![(puff(0.0, 10.0, 0.0, 2.0, 1.0), true), (puff(30.0, 10.0, 0.0, 1.0, 1.0), false)];
	let mut out = vec![
		Cluster { vol: Raw { share: 0.5, ..puff(1.0, 10.0, 0.0, 2.0, 1.0) }, members: vec![0, 1] },
		Cluster { vol: puff(30.5, 10.0, 0.0, 1.0, 1.0), members: vec![2] },
		Cluster { vol: puff(-40.0, 10.0, 0.0, 1.0, 1.0), members: vec![3] },
	];
	follow(&mut out, &last);
	assert!((out[0].vol.pos[0] - 0.5).abs() < 1e-6 && (out[0].vol.share - 0.75).abs() < 1e-6);
	assert!((out[1].vol.pos[0] - 30.5).abs() < 1e-6, "a lone puff stays exact");
	assert_eq!(out[2].vol.pos[0], -40.0, "nothing of last frame is near: as it is");
}

/// A18: the volume's share (drawn thickness) falls to zero over the last tenth of the range, its thickness unchanged (the shader
/// fades after the clumping: review R18 M1); the march reaches past the range by the largest radius.
#[test]
fn a_volume_fades_out_at_the_range() {
	let view = ([0.0; 3], [0.0, 1.0, 0.0], 150.0);
	let raw = [puff(0.0, 100.0, 0.0, 1.0, 1.0), puff(0.0, 140.0, 0.0, 1.0, 1.0), puff(0.0, 150.9, 0.0, 1.0, 1.0)];
	let v = reduce(&raw, view, 24, 90, &mut Track::default()).vols;
	assert!(v.iter().all(|r| r.thick == 1.0));
	let thick = |y: f32| v.iter().find(|r| r.pos[1] == y).map(volumes::drawn);
	assert_eq!(thick(100.0), Some(1.0));
	assert!((thick(140.0).unwrap() - (150.0 - 139.0) / 15.0).abs() < 1e-5, "{:?}", thick(140.0));
	assert!(thick(150.9).unwrap() < 0.01, "at the range edge the volume is gone");
}

/// A15: a full hit list keeps the 16 nearest hits (a nearer hit replaces the farthest).
#[test]
fn the_hit_list_keeps_the_nearest_hits() {
	let src = shader::built_in();
	assert!(src.contains("if (t0 < hit_a[worst]) { slot = worst; }"));
}

/// A13: an additive sprite never becomes a volume (it would turn into grey fog); the same sprite with alpha blending does.
#[test]
fn additive_sprites_are_never_volumes() {
	let mut plan = Plan::with_rows(vec![0], &TABLE, [0.0; 3]);
	assert!(!plan.take(0, &sprite([0.0, 8.0, 0.0]), 2.0, GREY, (1, true, 2), quad()));
	assert!(plan.raw.is_empty() && plan.held.is_empty(), "the additive sprite is drawn as it is");
	assert!(plan.take(0, &sprite([0.0, 8.0, 0.0]), 2.0, GREY, (1, false, 2), quad()));
	assert_eq!((plan.raw.len(), plan.held.len()), (1, 1));
}

/// R7-2: smoke at the edge of the range hands over to retail without a gap. In the band the volume fades out (thickness x w) while its
/// sprite comes back at alpha x (1 - w); a held puff no volume shows (behind the eye) gets its sprite back whole; a near one stays a
/// volume. A level quad whose volume is in the band is un-taken (the quad pass draws it under the fading volume).
#[test]
fn smoke_hands_over_to_the_sprite_at_the_range() {
	let mut plan = Plan::with_rows(vec![1; 3], &TABLE, [0.0; 3]);
	assert!(plan.take(0, &sprite([0.0, 140.0, 0.0]), 2.0, GREY, (1, false, 2), quad_x(1.0)));
	assert!(plan.take(1, &sprite([0.0, -20.0, 0.0]), 2.0, GREY, (1, false, 2), quad_x(2.0)));
	assert!(plan.take(2, &sprite([0.0, 30.0, 0.0]), 2.0, GREY, (1, false, 2), quad_x(3.0)));
	let w = volumes::range_weight(&plan.raw[0], [0.0; 3], 150.0);
	assert!(w > 0.5 && w < 0.9, "the first puff is in the band ({w})");
	let thick = plan.raw[0].thick;
	let mut s = VolSmoke::new(wgpu::TextureFormat::Rgba8Unorm);
	s.finish(plan);
	s.set_quads(columns::Gathered { raw: vec![puff(0.0, 145.0, 0.0, 1.0, 1.0), puff(0.0, 20.0, 0.0, 1.0, 1.0)], src: vec![0, 1], took: vec![true, true], ..Default::default() });
	let mut verts = Verts::new();
	s.settle_cap(true, &eye(), 24, &mut verts);
	let back = &verts[&(1, false, 2)];
	let alpha = |x: f32| back.iter().find(|v| v.pos[0] == x).map(|v| v.colour[3]);
	assert_eq!(back.len(), 8, "the band's sprite and the one behind the eye come back, the near one stays a volume");
	assert!((alpha(1.0).unwrap() - (1.0 - w)).abs() < 1e-6, "cross-fade: the sprite at alpha x (1 - w)");
	assert_eq!((alpha(2.0), alpha(3.0)), (Some(1.0), None), "behind the eye: whole; near: a volume only");
	let far = s.vols.iter().find(|v| v.pos[1] == 140.0).expect("the band's volume still draws");
	assert!(far.thick == thick && (far.share - w).abs() < 1e-6, "and fades by w (its share)");
	assert_eq!((s.took(0), s.took(1)), (false, true), "the band's quad draws again, the near quad stays a volume");
	// The second pass of a split frame takes the faded quads out again (bit for bit) before it decides.
	s.settle_cap(true, &eye(), 24, &mut verts);
	assert_eq!(verts.values().map(Vec::len).sum::<usize>(), 8);
}

/// The trail band (plan.rs `trail_share`): below smoke_min_size the retail sprite alone, from `TRAIL_BAND` times it the volume alone,
/// smooth and rising between; min 0 = no band (every puff a volume).
#[test]
fn trail_share_hands_small_puffs_to_the_sprite() {
	use plan::{trail_share, MIN_SIZE, TRAIL_BAND};
	assert_eq!(trail_share(0.5 * MIN_SIZE, MIN_SIZE), 0.0);
	assert_eq!(trail_share(MIN_SIZE, MIN_SIZE), 0.0);
	assert_eq!(trail_share(MIN_SIZE * TRAIL_BAND, MIN_SIZE), 1.0);
	assert_eq!(trail_share(9.0 * MIN_SIZE, MIN_SIZE), 1.0);
	assert_eq!(trail_share(0.1, 0.0), 1.0, "no band");
	assert!((trail_share(1.0 + 0.5 * (TRAIL_BAND - 1.0), 1.0) - 0.5).abs() < 1.0e-5, "half way = half the volume");
	let mut last = 0.0;
	for k in 1..40 {
		let s = trail_share(1.0 + (TRAIL_BAND - 1.0) * k as f32 / 40.0, 1.0);
		assert!(s > last && s < 1.0, "rises smoothly through the band");
		last = s;
	}
}

/// A puff below the band stays the retail sprite (not taken, no volume). A puff in the band is held with its sprite share: its volume
/// keeps the puff's thickness and age (a faded sprite: alpha 0.4 = half thickness, age 0.5) and draws at the rest as its share (the
/// shader applies it after the clumping: review R18 M1), and while the volume draws that share of the sprite comes back at alpha x keep.
#[test]
fn a_band_puff_keeps_part_of_its_sprite() {
	let mid = 1.0 + 0.5 * (plan::TRAIL_BAND - 1.0);
	let faded = [0.4, 0.4, 0.4, 0.4];
	let mut plan = Plan::with_rows(vec![1, 1], &TABLE, [0.0; 3]);
	assert!(!plan.take(0, &sprite([0.0, 8.0, 0.0]), 0.6, faded, (1, false, 2), quad_x(1.0)), "a small trail puff stays a sprite");
	assert!(plan.raw.is_empty() && plan.held.is_empty());
	assert!(plan.take(1, &sprite([0.0, 8.0, 0.0]), mid, faded, (1, false, 2), quad_x(2.0)));
	let keep = plan.held[0].keep;
	assert!((keep - 0.5).abs() < 1.0e-5);
	let mut whole = Plan::with_rows(vec![1], &TABLE, [0.0; 3]);
	assert!(whole.take(0, &sprite([0.0, 8.0, 0.0]), 2.0, faded, (1, false, 2), quad()));
	assert_eq!(whole.held[0].keep, 0.0, "a big puff is the volume alone");
	assert!((whole.raw[0].thick - 0.5).abs() < 1.0e-5 && (whole.raw[0].age - 0.5).abs() < 1.0e-5, "alpha 0.4: half thick, half aged");
	let (p, w) = (&plan.raw[0], &whole.raw[0]);
	assert!((p.thick - w.thick).abs() < 1.0e-6 && (p.age - w.age).abs() < 1.0e-6, "the band neither thins nor ages the puff");
	assert!((p.share - 0.5).abs() < 1.0e-5 && w.share == 1.0, "the volume draws the other half");
	let rec = volumes::to_vols(&plan.raw, &[]);
	assert!((rec[0].glow[3] - 0.5).abs() < 1.0e-5 && (rec[0].col[3] - p.thick * 1.5 / p.radius).abs() < 1.0e-6, "the share rides in glow.w, the extinction is the puff's");
	let mut s = VolSmoke::new(wgpu::TextureFormat::Rgba8Unorm);
	s.finish(plan);
	let mut verts = Verts::new();
	s.settle_cap(true, &eye(), 24, &mut verts);
	assert_eq!(s.vols.len(), 1, "the volume draws");
	let back = verts.get(&(1, false, 2)).expect("the sprite's share comes back");
	assert_eq!(back.len(), 4);
	assert!(back.iter().all(|v| v.pos[0] == 2.0 && (v.colour[3] - keep).abs() < 1.0e-5));
}

/// Review R18 m1 / m6: the trail band, the range band and the cap together. Per held band puff (keep 0.5): drawn as a volume ->
/// sprite at keep; its volume fading out at the range with w -> sprite at keep + (1 - keep)(1 - w) (sprite + volume share add up
/// to one); its volume cut by the cap -> the whole sprite; a big puff fading at the range -> 1 - w.
#[test]
fn trail_band_range_band_and_cut_add_up() {
	let mid = 1.0 + 0.5 * (plan::TRAIL_BAND - 1.0);
	// Far apart (no merge): a near band puff, a band puff and a big puff in the range band, three near big puffs, and a barely
	// taken far puff (the least important: the cap of 6 cuts it).
	let spots: [([f32; 3], f32); 7] = [
		([0.0, 20.0, 0.0], mid),
		([0.0, 140.0, 0.0], mid),
		([40.0, 140.0, 0.0], 2.0),
		([-30.0, 10.0, 0.0], 2.0),
		([30.0, 10.0, 0.0], 2.0),
		([0.0, 8.0, 0.0], 2.0),
		([60.0, 100.0, 0.0], 1.1),
	];
	let mut plan = Plan::with_rows(vec![1; 7], &TABLE, [0.0; 3]);
	for (i, (p, size)) in spots.iter().enumerate() {
		assert!(plan.take(i, &sprite(*p), *size, [0.4, 0.4, 0.4, 1.0], (1, false, 2), quad_x(i as f32)));
	}
	let keep6 = plan.held[6].keep;
	assert!(keep6 > 0.9 && keep6 < 1.0);
	let w_band = volumes::range_weight(&plan.raw[1], [0.0; 3], 150.0);
	let w_big = volumes::range_weight(&plan.raw[2], [0.0; 3], 150.0);
	assert!(w_band > 0.5 && w_band < 0.9 && w_big > 0.1 && w_big < 0.5, "{w_band} {w_big}");
	let mut s = VolSmoke::new(wgpu::TextureFormat::Rgba8Unorm);
	s.finish(plan);
	let mut verts = Verts::new();
	s.settle_cap(true, &eye(), 7, &mut verts);
	let back = &verts[&(1, false, 2)];
	let alpha = |x: f32| back.iter().find(|v| v.pos[0] == x).map(|v| v.colour[3]);
	assert!((alpha(0.0).unwrap() - 0.5).abs() < 1e-5, "near band puff: its keep");
	assert!((alpha(1.0).unwrap() - (0.5 + 0.5 * (1.0 - w_band))).abs() < 1e-5, "band puff in the range band: keep + (1 - keep)(1 - w)");
	assert!((alpha(2.0).unwrap() - (1.0 - w_big)).abs() < 1e-5, "big puff in the range band: 1 - w");
	assert_eq!((alpha(3.0), alpha(4.0), alpha(5.0)), (None, None, None), "near big puffs: volumes only");
	assert!((alpha(6.0).unwrap() - keep6).abs() < 1e-5, "the far band puff draws: its keep");
	let vol = |x: f32, y: f32| s.vols.iter().find(|v| v.pos[0] == x && v.pos[1] == y).map(|v| v.share);
	assert!((vol(0.0, 140.0).unwrap() - 0.5 * w_band).abs() < 1e-5, "its volume at share x w: sprite + volume = 1");
	// The same frame under a cap of 6: the least important (the far band puff) is cut and gets its whole sprite back.
	let mut verts = Verts::new();
	s.settle_cap(true, &eye(), 6, &mut verts);
	let back = &verts[&(1, false, 2)];
	assert_eq!(back.iter().find(|v| v.pos[0] == 6.0).map(|v| v.colour[3]), Some(1.0), "cut: the whole sprite");
	assert!(!s.vols.iter().any(|v| v.pos == spots[6].0) && s.vols.len() == 6);
	assert!((back.iter().find(|v| v.pos[0] == 1.0).unwrap().colour[3] - (0.5 + 0.5 * (1.0 - w_band))).abs() < 1e-5);
}
