//! Replay of a real play session's waterfall frames through `sheet_frame` (lane nfalls, 2026-10-30): the quads that `sheet_falls`
//! logged with RAC_FALL_LOG on the laptop (frame smoothing on, Pokitaru's landing pad, 44 frames with sheets, up to 2806 quads), and
//! the shaped vertices it logged after them. The log is geometry derived from the disc and not part of the tree: the tests that read
//! it are `#[ignore]`d (a plain `cargo test` lists them as ignored, never as passed) and take its path from RAC_FALL_REPLAY only
//! (`cargo test --release --lib fall_sheets -- --ignored --nocapture` with RAC_FALL_REPLAY set); run without it they fail loudly.
//! The always-on tests of the same old-vs-fast comparison, on synthetic sheets, are in `fast/stress_tests.rs`.
//! - `old_path_reproduces_the_log`: the old path gives the logged positions and light bit for bit (the log is this code's output).
//! - `fast_path_is_the_old_path`: the fast path (`fast.rs`) gives the same vertices, `falls` and SheetMem state, frame by frame,
//!   on the log, on the log with positions wobbled the way frame smoothing blends them, and with refill candidates.
//! - `bench_old_vs_fast` (ignored; `cargo test --release bench_old_vs_fast -- --ignored --nocapture`): time per frame.

use super::*;
use std::time::Instant;

pub(super) type Quad = ([[f32; 3]; 4], [[f32; 2]; 4], [[f32; 4]; 4]);

struct Frame {
	quads: Vec<Quad>,
	shape: Vec<([f32; 3], [f32; 4])>,
}

fn nums(line: &str) -> Vec<f32> {
	line.split(|c: char| matches!(c, '[' | ']' | ',' | ' ')).filter(|t| !t.is_empty()).map(|t| t.parse::<f32>().expect("number")).collect()
}

/// The logged frames of the file RAC_FALL_REPLAY names. Panics without it or with an unreadable file: an ignored test that was
/// asked for must not pass without replaying anything.
fn load() -> Vec<Frame> {
	let path = std::env::var("RAC_FALL_REPLAY").expect("skipped: RAC_FALL_REPLAY not set (the path of a RAC_FALL_LOG file)");
	let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("RAC_FALL_REPLAY={path} is not readable: {e}"));
	parse(&text).expect("RAC_FALL_REPLAY: not a RAC_FALL_LOG file")
}

/// The frames of a RAC_FALL_LOG text (complete ones only: the log may end in the middle of a frame).
fn parse(text: &str) -> Option<Vec<Frame>> {
	let mut out: Vec<Frame> = Vec::new();
	let mut want = 0usize;
	for line in text.lines() {
		if let Some(rest) = line.strip_prefix("FRAME ") {
			want = rest.split(' ').nth(2)?.parse().ok()?;
			out.push(Frame { quads: Vec::new(), shape: Vec::new() });
		} else if let Some(rest) = line.strip_prefix("SHAPE ") {
			let n = nums(rest);
			out.last_mut()?.shape.push(([n[0], n[1], n[2]], [n[3], n[4], n[5], n[6]]));
		} else if want > 0 {
			let n = nums(line);
			if n.len() != 36 {
				break; // the log ends in the middle of a line
			}
			let f = out.last_mut()?;
			let (mut pos, mut uv, mut col) = ([[0.0; 3]; 4], [[0.0; 2]; 4], [[0.0; 4]; 4]);
			for k in 0..4 {
				pos[k] = [n[k * 3], n[k * 3 + 1], n[k * 3 + 2]];
				uv[k] = [n[12 + k * 2], n[13 + k * 2]];
				col[k] = [n[20 + k * 4], n[21 + k * 4], n[22 + k * 4], n[23 + k * 4]];
			}
			f.quads.push((pos, uv, col));
		}
	}
	out.retain(|f| f.shape.len() == f.quads.len() * 4);
	eprintln!("fall replay: {} frames, {} with sheets", out.len(), out.iter().filter(|f| !f.quads.is_empty()).count());
	Some(out)
}

/// The state `WaterFx` keeps for the sheets.
#[derive(Default)]
pub(super) struct Run {
	pub(super) mem: SheetMem,
	pub(super) verts: Vec<Vertex>,
	pub(super) falls: Vec<[f32; 3]>,
}

impl Run {
	/// One `sheet_falls` call: `quads` are the sheet quads, `refill` the widescreen refill candidates.
	pub(super) fn step(&mut self, quads: &[Quad], refill: &[Quad], fast: bool) {
		self.falls.clear();
		self.verts.clear();
		let all = quads.iter().map(|&(p, u, c)| (p, u, c, false)).chain(refill.iter().map(|&(p, u, c)| (p, u, c, true)));
		push_quads(&mut self.mem.cand, &mut self.verts, all);
		sheet_frame(&mut self.mem, &mut self.verts, &mut self.falls, 0, fast);
	}
}

impl Run {
	/// `step` timed in parts: pushing the quads as vertices, `observe`, refill, feet (ns).
	fn timed_step(&mut self, quads: &[Quad], fast: bool) -> [u64; 4] {
		self.falls.clear();
		self.verts.clear();
		let t = Instant::now();
		push_quads(&mut self.mem.cand, &mut self.verts, quads.iter().map(|&(p, u, c)| (p, u, c, false)));
		let t_push = t.elapsed().as_nanos() as u64;
		let cand = std::mem::take(&mut self.mem.cand);
		let t = Instant::now();
		let drawn = if fast {
			self.mem.observe_fast(&mut self.verts[..]);
			Vec::new()
		} else {
			self.mem.observe(&mut self.verts[..])
		};
		let t_obs = t.elapsed().as_nanos() as u64;
		let t = Instant::now();
		if !cand.is_empty() {
			let more = self.mem.refill(&drawn, &cand);
			self.verts.extend(more);
		}
		let t_refill = t.elapsed().as_nanos() as u64;
		self.mem.cand = cand;
		let t = Instant::now();
		for f in &self.mem.feet {
			if !self.falls.iter().any(|g| (g[0] - f[0]).abs() < CHURN_CELL && (g[1] - f[1]).abs() < CHURN_CELL && (g[2] - f[2]).abs() < 1.0) {
				self.falls.push(*f);
			}
		}
		keep_lowest(&mut self.falls);
		[t_push, t_obs, t_refill, t.elapsed().as_nanos() as u64]
	}
}

pub(super) fn vbits(v: &[Vertex]) -> Vec<[u32; 7]> {
	v.iter().map(|v| [v.pos[0].to_bits(), v.pos[1].to_bits(), v.pos[2].to_bits(), v.light[0].to_bits(), v.light[1].to_bits(), v.light[2].to_bits(), v.light[3].to_bits()]).collect()
}

pub(super) fn fbits(f: &[[f32; 3]]) -> Vec<[u32; 3]> {
	f.iter().map(|p| p.map(f32::to_bits)).collect()
}

/// Everything `SheetMem` keeps, as bits (the known set sorted).
pub(super) fn mem_bits(m: &SheetMem) -> (Vec<[u32; 3]>, Vec<[u32; 4]>, Vec<[u32; 2]>, Vec<[u32; 3]>, Vec<[u32; 3]>, Vec<u32>, Vec<[u32; 4]>, Vec<[u32; 3]>) {
	let mut known: Vec<[u32; 3]> = m.known.iter().copied().collect();
	known.sort_unstable();
	(
		fbits(&m.verts),
		m.light.iter().map(|l| l.map(f32::to_bits)).collect(),
		m.steep.iter().map(|s| [s.0.to_bits(), s.1.to_bits()]).collect(),
		fbits(&m.feet_seen),
		fbits(&m.feet),
		m.last_ids.clone(),
		m.faces.clone(),
		known,
	)
}

struct Rnd(u32);

impl Rnd {
	fn next(&mut self) -> f32 {
		self.0 ^= self.0 << 13;
		self.0 ^= self.0 >> 17;
		self.0 ^= self.0 << 5;
		(self.0 % 100_000) as f32 / 100_000.0 - 0.5
	}
}

/// The quads with each corner moved by up to `amp` per axis (frame smoothing blends positions: new bits every frame).
fn wobble(quads: &[Quad], amp: f32, seed: u32) -> Vec<Quad> {
	let mut r = Rnd(seed | 1);
	quads
		.iter()
		.map(|&(mut p, u, c)| {
			for v in p.iter_mut() {
				for a in v.iter_mut() {
					*a += r.next() * 2.0 * amp;
				}
			}
			(p, u, c)
		})
		.collect()
}

#[ignore = "needs RAC_FALL_REPLAY=<RAC_FALL_LOG file>"]
#[test]
fn old_path_reproduces_the_log() {
	let frames = load();
	assert!(frames.iter().any(|f| f.quads.len() > 2000), "the log has the big frames");
	let mut run = Run::default();
	for (n, f) in frames.iter().enumerate() {
		run.step(&f.quads, &[], false);
		let want: Vec<[u32; 7]> = f.shape.iter().map(|(p, l)| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits(), l[0].to_bits(), l[1].to_bits(), l[2].to_bits(), l[3].to_bits()]).collect();
		assert_eq!(vbits(&run.verts), want, "frame {n} ({} quads)", f.quads.len());
	}
}

/// Runs the frames old and fast side by side, `jitter` = 0 for the log as it is, else the wobble amplitude; `cut` = every how many
/// quads one is not drawn but offered as a refill candidate (0 = none).
fn same_old_and_fast(frames: &[Frame], jitter: f32, cut: usize) -> usize {
	let (mut old, mut new) = (Run::default(), Run::default());
	let mut refilled = 0;
	for (n, f) in frames.iter().enumerate() {
		let quads = if jitter > 0.0 { wobble(&f.quads, jitter, 7 + n as u32) } else { f.quads.clone() };
		let (drawn, refill): (Vec<Quad>, Vec<Quad>) = if cut == 0 {
			(quads, Vec::new())
		} else {
			let mut d = Vec::new();
			let mut r = Vec::new();
			for (i, q) in quads.into_iter().enumerate() {
				if i % cut == 2 { r.push(q) } else { d.push(q) }
			}
			(d, r)
		};
		old.step(&drawn, &refill, false);
		new.step(&drawn, &refill, true);
		refilled += old.verts.len() - drawn.len() * 4;
		assert_eq!(vbits(&old.verts), vbits(&new.verts), "frame {n}: vertices");
		assert_eq!(fbits(&old.falls), fbits(&new.falls), "frame {n}: falls");
		assert!(mem_bits(&old.mem) == mem_bits(&new.mem), "frame {n}: sheet memory");
	}
	refilled
}

#[ignore = "needs RAC_FALL_REPLAY=<RAC_FALL_LOG file>"]
#[test]
fn fast_path_is_the_old_path() {
	let frames = load();
	assert_eq!(same_old_and_fast(&frames, 0.0, 0), 0);
	assert_eq!(same_old_and_fast(&frames, 0.03, 0), 0);
	assert_eq!(same_old_and_fast(&frames, 0.12, 0), 0);
	// With refill candidates (every 7th / 5th quad is offered as one instead of drawn): some come back as refill vertices.
	let a = same_old_and_fast(&frames, 0.0, 7);
	let b = same_old_and_fast(&frames, 0.04, 5);
	assert!(a > 100 && b > 100, "refill vertices: {a}, {b}");
}

#[ignore]
#[test]
fn bench_old_vs_fast() {
	let frames = load();
	let big: Vec<&Frame> = frames.iter().filter(|f| f.quads.len() >= 2700).collect();
	for fast in [false, true, false, true] {
		let mut run = Run::default();
		// The log as it is: the frames in order.
		let mut t_log = Vec::new();
		for f in &frames {
			let t = Instant::now();
			run.step(&f.quads, &[], fast);
			t_log.push((f.quads.len(), t.elapsed().as_secs_f64() * 1e3));
		}
		// Steady state with smoothing's new bits every frame: the big frames again and again, wobbled.
		let mut t_wob = Vec::new();
		let mut parts = [0u64; 4];
		for k in 0..120usize {
			let f = big[k % big.len()];
			let quads = wobble(&f.quads, 0.03, 1000 + k as u32);
			let t = Instant::now();
			run.step(&quads, &[], fast);
			t_wob.push(t.elapsed().as_secs_f64() * 1e3);
			let quads = wobble(&f.quads, 0.03, 5000 + k as u32);
			for (a, b) in parts.iter_mut().zip(run.timed_step(&quads, fast)) {
				*a += b;
			}
		}
		eprintln!("fast={fast} steady wobbled parts us/frame (push quads, observe, refill, feet): {:?}", parts.map(|p| p / 120 / 1000));
		let med = |mut v: Vec<f64>| {
			v.sort_by(|a, b| a.total_cmp(b));
			v[v.len() / 2]
		};
		let tail: Vec<f64> = t_log.iter().filter(|t| t.0 >= 2700).map(|t| t.1).collect();
		eprintln!(
			"fast={fast}: log big frames median {:.3} ms (n={}); first big frame {:.3}; wobbled steady median {:.3} ms (min {:.3}, max {:.3})",
			med(tail.clone()),
			tail.len(),
			tail[0],
			med(t_wob.clone()),
			t_wob.iter().cloned().fold(f64::MAX, f64::min),
			t_wob.iter().cloned().fold(0.0, f64::max)
		);
	}
}

#[ignore]
#[test]
fn stats_of_the_log() {
	let frames = load();
	let mut run = Run::default();
	for (n, f) in frames.iter().enumerate() {
		if f.quads.is_empty() {
			continue;
		}
		let last = run.mem.last_ids.clone();
		let before = (run.mem.verts.len(), run.mem.faces.len());
		let verts_before = run.mem.verts.clone();
		let t = Instant::now();
		run.step(&f.quads, &[], false);
		let dt = t.elapsed().as_secs_f64() * 1e3;
		let (mut hit, mut valid) = (0, 0);
		for (k, v) in f.quads.iter().flat_map(|q| q.0).enumerate() {
			if let Some(h) = last.get(k) {
				valid += 1;
				if let Some(q) = verts_before.get(*h as usize) {
					if (q[0] - v[0]).hypot(q[1] - v[1]).hypot(q[2] - v[2]) < SHEET_SNAP * 0.5 - 1e-3 {
						hit += 1;
					}
				}
			}
		}
		eprintln!(
			"stat frame {n} quads {} corners {} hint valid {valid} hit {hit}; new verts {} new faces {}; feet_seen {} feet {} falls {}; {dt:.3} ms",
			f.quads.len(),
			f.quads.len() * 4,
			run.mem.verts.len() - before.0,
			run.mem.faces.len() - before.1,
			run.mem.feet_seen.len(),
			run.mem.feet.len(),
			run.falls.len()
		);
	}
}

#[ignore]
#[test]
fn bench_shape() {
	let frames = load();
	let mut run = Run::default();
	for f in &frames {
		run.step(&f.quads, &[], false);
	}
	let m = &run.mem;
	let steep: Vec<f32> = m.steep.iter().map(|&(s, n)| if n > 0.0 { s / n } else { 0.0 }).collect();
	eprintln!("stat shape: {} verts, {} faces, {} steep >= 0.5", m.verts.len(), m.faces.len(), steep.iter().filter(|&&s| s >= 0.5).count());
	for par in [usize::MAX, 0, usize::MAX, 0] {
		let mut light = vec![[0.0f32; 4]; m.verts.len()];
		let t = Instant::now();
		for _ in 0..20 {
			shape_faces(&m.verts, &m.faces, &steep, &mut light, par);
		}
		eprintln!("stat shape par_min {par}: {:.3} ms per call", t.elapsed().as_secs_f64() * 1e3 / 20.0);
	}
}

#[ignore]
#[test]
fn stats_nearest_neighbour() {
	let frames = load();
	let mut run = Run::default();
	for f in &frames {
		run.step(&f.quads, &[], false);
	}
	let v = &run.mem.verts;
	let mut nn: Vec<f32> = (0..v.len())
		.map(|i| (0..v.len()).filter(|&j| j != i).map(|j| (v[i][0] - v[j][0]).hypot(v[i][1] - v[j][1]).hypot(v[i][2] - v[j][2])).fold(f32::MAX, f32::min))
		.collect();
	nn.sort_by(|a, b| a.total_cmp(b));
	let q = |f: f32| nn[((nn.len() - 1) as f32 * f) as usize];
	eprintln!("stat nn: {} verts; nn min {:.3} p5 {:.3} p25 {:.3} median {:.3} p75 {:.3} max {:.3}", nn.len(), q(0.0), q(0.05), q(0.25), q(0.5), q(0.75), q(1.0));
	// How far is each corner of the big frames from its vertex (the answer of `id`), and in units of the vertex's nn / 2.
	let mut ratios = Vec::new();
	let mut dists = Vec::new();
	for f in frames.iter().filter(|f| f.quads.len() > 2700) {
		for q in &f.quads {
			for p in q.0 {
				if let Some(i) = run.mem.search(p) {
					let w = v[i as usize];
					let d = (w[0] - p[0]).hypot(w[1] - p[1]).hypot(w[2] - p[2]);
					dists.push(d);
					let n = (0..v.len()).filter(|&j| j != i as usize).map(|j| (w[0] - v[j][0]).hypot(w[1] - v[j][1]).hypot(w[2] - v[j][2])).fold(f32::MAX, f32::min);
					ratios.push(d / (n.min(SHEET_SNAP * 2.0) * 0.5));
				}
			}
		}
	}
	dists.sort_by(|a, b| a.total_cmp(b));
	ratios.sort_by(|a, b| a.total_cmp(b));
	let r = |v: &Vec<f32>, f: f32| v[((v.len() - 1) as f32 * f) as usize];
	eprintln!("stat corners {}: dist to vertex p50 {:.3} p90 {:.3} p99 {:.3} max {:.3}; frac dist < 0.089: {:.3}; d/(min(nn,2*snap)/2) p50 {:.2} p90 {:.2} p99 {:.2}; frac < 0.99: {:.3}",
		dists.len(), r(&dists, 0.5), r(&dists, 0.9), r(&dists, 0.99), r(&dists, 1.0), dists.iter().filter(|&&d| d < 0.089).count() as f32 / dists.len() as f32,
		r(&ratios, 0.5), r(&ratios, 0.9), r(&ratios, 0.99), ratios.iter().filter(|&&d| d < 0.99).count() as f32 / ratios.len() as f32);
}
