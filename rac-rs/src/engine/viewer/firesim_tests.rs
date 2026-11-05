use super::super::firefx_vol::{ball, flame};
use super::*;

fn module(src: &str, what: &str) -> (naga::Module, naga::valid::ModuleInfo) {
	let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{what}: {}", e.emit_to_string(src)));
	let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{what} validate: {e:?}"));
	(m, info)
}

fn names(m: &naga::Module) -> Vec<String> {
	m.entry_points.iter().map(|e| e.name.clone()).collect()
}

/// A jet slab as firefx_jet_vol.rs writes it: p0 r0 in a, p1 r1 in d, alphas in c.w / b.z, start s0 in e.w, d0 in b.w - KIND_JET.
fn jet(p0: [f32; 3], p1: [f32; 3], r: f32, s0: f32, d0: f32) -> Vol {
	Vol { a: [p0[0], p0[1], p0[2], r], b: [0.5, 0.3, 1.0, KIND_JET + d0], c: [1.0, 0.8, 0.6, 1.0], d: [p1[0], p1[1], p1[2], r], e: [0.0, 0.0, 0.0, s0], f: [0.0, 0.0, 0.0, 0.7], ..bytemuck::Zeroable::zeroed() }
}

#[test]
fn both_shaders_validate_with_their_entry_points() {
	let (s, _) = module(STEP_WGSL, "fire sim step");
	assert!(names(&s).iter().any(|n| n == "cs_step"));
	let (d, _) = module(DRAW_WGSL, "fire sim draw");
	let n = names(&d);
	for e in ["vs_part", "fs_part", "fs_heat", "vs_comp", "fs_comp"] {
		assert!(n.iter().any(|x| x == e), "{e}");
	}
	assert_eq!(std::mem::size_of::<SimEmit>(), 128, "eight vec4 rows");
	assert_eq!(std::mem::size_of::<super::super::firesim_heat::HeatU>(), 48, "three vec4 rows");
	assert!(!DRAW_WGSL.contains("EDGE0") && !DRAW_WGSL.contains("FLICK"), "the per-sprite flame shading is gone");
	assert_eq!(std::mem::size_of::<SimU>(), 144, "nine vec4 rows");
	let line = format!("const SMOKE_X: f32 = {SMOKE_X:?};");
	assert!(STEP_WGSL.contains(&line) && DRAW_WGSL.contains(&line), "the shaders mirror SMOKE_X");
}

#[cfg(windows)]
#[test]
fn both_shaders_write_hlsl() {
	for (src, what, entry) in [(STEP_WGSL, "step", "cs_step"), (DRAW_WGSL, "draw", "fs_part"), (DRAW_WGSL, "heat", "fs_heat"), (DRAW_WGSL, "comp", "fs_comp")] {
		let (m, info) = module(src, what);
		let (options, pipeline) = (naga::back::hlsl::Options::default(), naga::back::hlsl::PipelineOptions::default());
		let mut out = String::new();
		naga::back::hlsl::Writer::new(&mut out, &options, &pipeline).write(&m, &info, None).expect("HLSL writes");
		assert!(out.contains(entry), "{what}");
	}
}

#[test]
fn the_pool_follows_the_particles_quality() {
	assert_eq!((pool_of(1), pool_of(2), pool_of(3)), (4096, 8192, POOL_MAX));
	assert!(density(1) < density(2) && density(2) < density(3));
}

#[test]
fn a_flame_a_ball_and_a_jet_slab_map_their_rows() {
	let f = want_of(&flame([1.0, 2.0, 3.0], 2.0, 0.5, 0.1, 1.0, [1.0, 0.5, 0.25]), [1.0, 2.0, 3.0], 1.0).expect("flame");
	assert_eq!(f.e.n[1], EMIT_FLAME);
	assert_eq!((f.e.a, f.e.b[0], f.e.c), ([1.0, 2.0, 3.0, 2.0], 0.5, [1.0, 0.5, 0.25, 1.0]));
	assert!(f.rate > 0.0 && f.live > f.rate * 0.1);
	let b = want_of(&ball([0.0; 3], 1.0, 0.2, 0.5, [1.0; 3]), [0.0; 3], 1.0).expect("ball");
	assert_eq!((b.e.n[1], b.e.a[3], b.e.b[0]), (EMIT_BURST, 1.0, 0.5));
	let j = want_of(&jet([0.0; 3], [2.0, 0.0, 0.0], 0.3, 0.5, 0.75), [0.0; 3], 1.0).expect("jet");
	assert_eq!(j.e.n[1], EMIT_JET);
	assert_eq!((j.e.b, j.e.d, j.e.e[1]), ([2.0, 0.0, 0.0, 0.3], [0.5, 0.7, 0.5, 1.0], 0.75), "p1 r1, (s0, f.w, b.x, alpha 1), d0");
	assert_eq!(j.e.c[3], 1.0);
}

#[test]
fn nothing_visible_emits_nothing() {
	assert!(want_of(&flame([0.0; 3], 2.0, 0.5, 0.1, 0.0, [1.0; 3]), [0.0; 3], 1.0).is_none());
	assert!(want_of(&ball([0.0; 3], 1.0, 0.2, 0.0, [1.0; 3]), [0.0; 3], 1.0).is_none());
	let mut j = jet([0.0; 3], [1.0, 0.0, 0.0], 0.3, 0.0, 0.0);
	(j.c[3], j.b[2]) = (0.0, 0.0);
	assert!(want_of(&j, [0.0; 3], 1.0).is_none());
}

#[test]
fn a_far_fire_gets_fewer_bigger_particles() {
	let v = flame([0.0; 3], 2.0, 0.5, 0.1, 1.0, [1.0; 3]);
	let (near, far) = (want_of(&v, [3.0, 0.0, 0.0], 1.0).unwrap(), want_of(&v, [60.0, 0.0, 0.0], 1.0).unwrap());
	assert!(far.rate < near.rate * 0.5 && far.live < near.live * 0.5);
	assert!((near.e.e[0] - 1.0).abs() < 1e-6 && far.e.e[0] > 1.0);
	let low = want_of(&v, [3.0, 0.0, 0.0], 0.45).unwrap();
	assert!(low.rate < near.rate && low.e.e[0] > 1.0, "a lower density: fewer, bigger");
}

fn wants(n: usize, rate: f32, live: f32) -> Vec<Want> {
	(0..n).map(|i| Want { e: SimEmit { a: [i as f32, 0.0, 0.0, 1.0], e: [1.0, 0.0, 1.0, 0.0], ..Default::default() }, rate, live, weigh: live }).collect()
}

#[test]
fn the_plan_runs_its_ends_up_to_the_total() {
	let mut rows = Vec::new();
	let w = wants(20, 300.0, 100.0);
	let total = plan(&w, 1.0 / 60.0, 8192, 7, &mut rows);
	assert!(!rows.is_empty());
	assert!(rows.windows(2).all(|p| p[0].n[0] < p[1].n[0]), "ends ascending");
	assert_eq!(rows.last().unwrap().n[0], total);
	assert!(total <= 8192 / 2);
	let burst = plan(&wants(64, 1e6, 1.0), 0.05, 4096, 1, &mut rows);
	assert_eq!(burst, 2048, "one frame never spawns more than half the pool");
	assert_eq!(rows.last().unwrap().n[0], burst);
}

#[test]
fn the_plan_keeps_its_rate_on_average_and_scales_over_the_budget() {
	let mut rows = Vec::new();
	let dt = 1.0 / 60.0;
	let mean = |w: &[Want], pool: u32, rows: &mut Vec<SimEmit>| (0..1000).map(|f| plan(w, dt, pool, f, rows) as f64).sum::<f64>() / 1000.0;
	let w = wants(5, 30.0, 50.0);
	let m = mean(&w, 8192, &mut rows);
	assert!((m - 5.0 * 30.0 * dt as f64).abs() < 0.15, "rounding keeps the rate: {m}");
	plan(&w, dt, 8192, 3, &mut rows);
	assert!(rows.iter().all(|r| (r.e[0] - 1.0).abs() < 1e-6), "under budget: sizes kept");
	// Twice the room alive: every rate halves (live * s = 0.8 pool), the particles grow by sqrt 2.
	let room = 0.8 * 4096.0;
	let big = wants(4, 600.0, room * 2.0 / 4.0);
	let m = mean(&big, 4096, &mut rows);
	assert!((m - 4.0 * 600.0 * 0.5 * dt as f64).abs() < 0.3, "over budget: rates scale {m}");
	plan(&big, dt, 4096, 5, &mut rows);
	assert!(rows.iter().all(|r| (r.e[0] - 2f32.sqrt()).abs() < 1e-4));
}

#[test]
fn the_step_follows_the_draw_clock() {
	assert_eq!(step_of(None, 1.0), None, "first frame");
	assert_eq!(step_of(Some(2.0), 1.0), None, "clock back");
	assert_eq!(step_of(Some(1.0), 2.0), None, "jump");
	assert_eq!(step_of(Some(1.0), 1.0), Some(0.0), "repeat frame");
	assert_eq!(step_of(Some(1.0), 1.2), Some(MAX_DT), "a hitch is clamped");
	assert!((step_of(Some(1.0), 1.0 + 1.0 / 60.0).unwrap() - 1.0 / 60.0).abs() < 1e-6);
}

/// Seen 2026-11-03 (`fire_pick_log`, Blaster on Batalia, Devastator on Gemlik): the flame rows' types also draw the Blaster's beam
/// (type 72, white, 0.16), its impact embers (15, red, 0.06) and the Devastator's muzzle smoke (44, grey, 0.56): retail.
#[test]
fn only_flame_looking_sprites_are_flames() {
	use super::super::firefx::{blast_like, flame_like};
	assert!(!flame_like([1.99, 1.99, 1.99, 1.0], 0.16), "Blaster beam");
	assert!(!flame_like([1.09, 0.10, 0.0, 0.27], 0.06), "impact ember");
	assert!(!flame_like([0.62, 0.62, 0.62, 0.31], 0.56), "muzzle smoke");
	assert!(flame_like([1.95, 0.0, 0.0, 1.0], 0.34) && flame_like([1.0, 0.6, 0.2, 1.0], 1.0), "burn puff, a flame");
	assert!(blast_like([1.0, 0.6, 0.2, 1.0]) && blast_like([0.46, 0.06, 0.01, 0.99]), "warm explosion frames");
	assert!(!blast_like([0.05, 0.43, 0.03, 0.99]) && !blast_like([0.2, 0.3, 1.0, 1.0]) && !blast_like([0.9, 0.9, 0.9, 1.0]), "green, blue, white glows");
}

#[test]
fn each_emitter_finds_its_place_last_frame() {
	let row = |x: f32, kind: u32, s0: f32| SimEmit { a: [x, 0.0, 0.0, 1.0], b: [x + 1.0, 0.0, 0.0, 0.2], d: [s0, 0.0, 0.0, 0.0], n: [0, kind, 0, 0], ..Default::default() };
	let prev = [row(0.0, EMIT_JET, 0.0), row(1.0, EMIT_JET, 1.0), row(0.1, EMIT_FLAME, 0.0)];
	let mut rows = vec![row(0.2, EMIT_JET, 0.0), row(1.1, EMIT_JET, 1.0), row(5.0, EMIT_FLAME, 0.0), row(0.15, EMIT_FLAME, 0.0)];
	link_prev(&mut rows, &prev);
	assert_eq!(rows[0].pa, [0.0, 0.0, 0.0, 1.0], "the nozzle slab follows the nozzle slab");
	assert_eq!(rows[0].pb, [1.0, 0.0, 0.0, 0.0]);
	assert_eq!(rows[1].pa, [1.0, 0.0, 0.0, 1.0], "the next slab its own, not the nearer nozzle");
	assert_eq!(rows[2].pa[3], 0.0, "nothing within reach: spawned where it is");
	assert_eq!(rows[3].pa, [0.1, 0.0, 0.0, 1.0], "a flame matches a flame only");
	let step = STEP_WGSL;
	assert!(step.contains("pa: vec4<f32>, pb: vec4<f32>"), "the step's emitter mirrors SimEmit");
}

#[test]
fn the_heat_buffer_follows_the_fire_level() {
	use super::super::firesim_heat::{heat_size, tier};
	assert_eq!(tier(1), (4, 0));
	assert_eq!(tier(2), (2, 1));
	assert_eq!(tier(3), (2, 2));
	assert_eq!(heat_size(1920, 1080, 2), (960, 540));
	assert_eq!(heat_size(1281, 721, 4), (321, 181));
	assert_eq!(heat_size(0, 0, 4), (1, 1));
}

#[test]
fn a_wide_flame_breaks_into_tongues_with_the_same_total() {
	use super::super::firesim_tongues::{push_split, sub_count};
	assert_eq!((sub_count(0.3), sub_count(0.65), sub_count(1.6), sub_count(9.0)), (1, 2, 5, 8));
	let w = want_of(&flame([10.0, 20.0, 3.0], 4.0, 2.0, 0.1, 1.0, [1.0, 0.5, 0.25]), [10.0, 30.0, 3.0], 1.0).expect("flame");
	let mut out = Vec::new();
	push_split(w, &mut out);
	assert_eq!(out.len(), sub_count(2.0));
	let (live, rate): (f32, f32) = (out.iter().map(|s| s.live).sum(), out.iter().map(|s| s.rate).sum());
	assert!((live - w.live).abs() < 1e-3 * w.live && (rate - w.rate).abs() < 1e-3 * w.rate, "same total heat");
	for s in &out {
		let d = ((s.e.a[0] - 10.0).powi(2) + (s.e.a[1] - 20.0).powi(2)).sqrt();
		assert!(d <= 0.72 * 2.0 + 1e-4 && s.e.b[0] < 2.0 && s.e.e[3] > 0.0 && s.e.e[3] <= 1.0);
	}
	let mut again = Vec::new();
	push_split(w, &mut again);
	assert_eq!(out, again, "stable over frames");
	let small = want_of(&flame([0.0; 3], 1.0, 0.4, 0.1, 1.0, [1.0; 3]), [0.0, 5.0, 0.0], 1.0).expect("small");
	let mut one = Vec::new();
	push_split(small, &mut one);
	assert_eq!((one.len(), one[0].e.e[3]), (1, 0.0), "a small flame stays one emitter without a phase");
}

#[test]
fn a_level_tongue_stays_inside_its_retail_quad_as_separate_licks() {
	use super::super::firefx_vol::level_tongue;
	use super::super::firesim_tongues::push_split;
	// A tongue of a 3.2 m wide, 6 m tall retail quad (r = half the bottom edge x fire_w 1.25 = 2.0).
	let v = level_tongue(flame([10.0, 20.0, 3.0], 6.0, 2.0, 0.1, 1.0, [1.0, 0.5, 0.25]));
	let w = want_of(&v, [10.0, 30.0, 3.0], 1.0).expect("level tongue");
	assert_eq!(w.e.n[2], 1, "marked for the lick shape");
	assert!((w.e.b[1] - LEVEL_X * 1.6).abs() < 1e-4 && (w.e.b[2] - 6.0).abs() < 1e-4, "reach = the quad's half edge x 1.2, top = its side");
	// The same flame unmarked (a burn spot, a sprite's flame) keeps its old row.
	let plain = want_of(&flame([10.0, 20.0, 3.0], 6.0, 2.0, 0.1, 1.0, [1.0, 0.5, 0.25]), [10.0, 30.0, 3.0], 1.0).expect("flame");
	assert_eq!((plain.e.n[2], plain.e.b[1], plain.e.b[2]), (0, 0.0, 0.0));
	let mut out = Vec::new();
	push_split(w, &mut out);
	assert!(out.len() > 1);
	for s in &out {
		let d = ((s.e.a[0] - 10.0).powi(2) + (s.e.a[1] - 20.0).powi(2)).sqrt();
		assert_eq!(s.e.n[2], 1);
		assert!(s.e.a[3] <= 6.0 + 1e-4 && (s.e.b[2] - s.e.a[3]).abs() < 1e-5, "no lick taller than the quad");
		assert!(d + s.e.b[1] <= LEVEL_X * 1.6 + 1e-4 || s.e.b[1] <= 0.5 * s.e.b[0] + 1e-4, "each lick's reach inside the quad x 1.2");
	}
	// The shaders know the lick (flag 8, the cylinder leash).
	assert!(STEP_WGSL.contains("fn level_lick") && STEP_WGSL.contains("fn leash_lick") && DRAW_WGSL.contains("(flags & 8u) != 0u"));
}

#[test]
fn the_split_stays_in_budget_and_never_drops_a_fire() {
	use super::super::firesim_tongues::{split_all, sub_count};
	// A level full of wide fires (each would split 8 ways) and, last, the gun's stream: every one keeps a row, the total fits.
	let mut src: Vec<Want> = (0..380).map(|i| want_of(&flame([i as f32 * 3.0, 0.0, 0.0], 4.0, 3.0 + (i % 7) as f32 * 0.1, 0.1, 1.0, [1.0, 0.5, 0.25]), [0.0, -5.0, 0.0], 1.0).expect("flame")).collect();
	src.push(want_of(&jet([0.0; 3], [3.0, 0.0, 0.0], 0.3, 0.0, 0.0), [0.0, -2.0, 0.0], 1.0).expect("jet"));
	src.push(want_of(&jet([3.0, 0.0, 0.0], [6.0, 0.0, 0.0], 0.4, 3.0, 0.0), [0.0, -2.0, 0.0], 1.0).expect("jet"));
	assert!(src.iter().filter(|w| w.e.n[1] == EMIT_FLAME).map(|w| sub_count(w.e.b[0])).sum::<usize>() > MAX_EMIT);
	let mut out = Vec::new();
	let full = split_all(&src, MAX_EMIT, &mut out);
	assert!(full > MAX_EMIT && out.len() <= MAX_EMIT && out.len() > src.len(), "{} rows of {full}", out.len());
	assert_eq!(out.iter().filter(|w| w.e.n[1] == EMIT_JET).count(), 2, "the stream is still there");
	let (a, b): (f32, f32) = (src.iter().map(|w| w.live).sum(), out.iter().map(|w| w.live).sum());
	assert!((a - b).abs() < 1e-3 * a, "every fire keeps its whole count");
	// Each parent keeps its sub-rows together, in order, with its whole count (at least one row each).
	let mut j = 0;
	for (i, w) in src.iter().enumerate() {
		let (start, mut acc) = (j, 0.0f32);
		while acc < w.live * (1.0 - 1e-4) && j < out.len() {
			acc += out[j].live;
			j += 1;
		}
		assert!(j > start && (acc - w.live).abs() < 1e-3 * w.live, "fire {i} kept");
	}
	assert_eq!(j, out.len());
	let mut rows = Vec::new();
	plan(&out, 0.05, 16384, 1, &mut rows);
	assert!(rows.len() <= MAX_EMIT);
	// Few fires: all split in full; a cap smaller than the emitters: nothing splits, nothing is lost here.
	let few = &src[..10];
	assert_eq!(split_all(few, MAX_EMIT, &mut out), out.len());
	assert_eq!(out.len(), 80);
	split_all(&src, 100, &mut out);
	assert_eq!(out.len(), src.len());

}

#[test]
fn a_slab_that_turned_more_than_30_degrees_is_not_swept() {
	let slab = |dx: f32, dy: f32| SimEmit { a: [0.0, 0.0, 1.0, 0.2], b: [dx, dy, 1.0, 0.2], n: [0, EMIT_JET, 0, 0], ..Default::default() };
	let prev = [slab(2.0, 0.0)];
	let mut rows = [slab(2.0, 0.6)];
	link_prev(&mut rows, &prev);
	assert_eq!(rows[0].pa[3], 1.0, "a small turn (17 degrees) is swept");
	let mut rows = [slab(-2.0, 0.2)];
	link_prev(&mut rows, &prev);
	assert_eq!(rows[0].pa[3], 0.0, "an about-turn starts afresh: no sheet across the turn");
	let mut rows = [slab(0.0, 2.0)];
	link_prev(&mut rows, &prev);
	assert_eq!(rows[0].pa[3], 0.0, "a quarter turn too");
}

#[test]
fn a_burst_carries_its_retail_footprint() {
	let mut v = ball([1.0, 2.0, 3.0], 1.2, 0.5, 1.0, [1.0, 0.5, 0.1]);
	v.d[0] = super::super::firefx_vol::FOOT_X * 0.5 * 2.0;
	let w = want_of(&v, [1.0, 2.0, 4.0], density(3)).unwrap();
	assert_eq!(w.e.n[1], EMIT_BURST);
	assert!((w.e.b[1] - 1.2).abs() < 1e-6, "1.2 x the 2 m sprite's half side: {}", w.e.b[1]);
}

#[test]
fn level_licks_draw_as_tongues_with_embers() {
	// Lane nfire agent 18 (user: the burning domes were "one big round fluffy blob"): a level lick (flag 8) is an upright tongue cut by
	// noise in the heat pass and tagged for its own composite ramp; a few of its particles are embers (flag 16, never smoke, no leash).
	let (draw, step) = (DRAW_WGSL, STEP_WGSL);
	assert!(draw.contains("fn lick_vs") && draw.contains("fn lick_heat") && draw.contains("fn lick_ramp") && draw.contains("fn lick_at"));
	assert!(draw.contains("const LICK_TAG: f32 = 64.0;"), "the lick tag lies above every packed tint (1..32)");
	assert!(draw.contains("(flags & 16u) != 0u") && draw.contains("fn ember_vs") && draw.contains("i.kind == 16u"));
	assert!(step.contains("fn ember_of") && step.contains("16.0), vec4<f32>(e.c.xyz, e.c.w), vec4<f32>(pos, 0.0))"), "an ember is flag 16 with no leash");
	assert!(step.contains("if (p.c.w < 15.5 && rnd(s) < smoke_share(k))"), "an ember never turns into smoke");
}