//! [PC] perf (no retail counterpart): the props batch's pose entries (`gpuskin.rs` `PoseEntry`, one copy per distinct pose,
//! `props_set.rs` `build_records`) written only where they change. Each distinct pose comes with its maker's token
//! (`handler/pose_born.rs`: equal tokens = bit-equal poses, 0 = unknown). A pose with last build's token, place and length is
//! already in `props_pose` and in the GPU buffer, so it is neither copied nor uploaded; the others are copied and their
//! ranges (`dirty`) uploaded. Invariant: after each `PropsGpu::upload` the GPU buffer holds `props_pose` (a buffer grown there
//! is written whole), and only `build_records` writes `props_pose` (a layout rebuild empties it: then nothing is reused).
//!
//! [PC] perf (lane nprops): the direct mode (default; RAC_PROPS_SET_OLD=1 = the mirror above) keeps no CPU copy of the entries:
//! `place_direct` only decides (same token rule) and lists each distinct pose as a `PoseJob`; `PropsGpu::upload_direct` converts the
//! sources straight into the upload's staging memory. `valid` stands in for the mirror's length (the entries the GPU buffer holds).

use std::borrow::Cow;
use std::ops::Range;
use super::super::gpuskin::{PoseEntry, PropInst};

/// RAC_PROPS_SET_OLD=1: `set_props` keeps the CPU mirror of the pose entries (`Scene::props_pose`) and uploads from it, as before
/// lane nprops (same-exe A/B and fault isolation). Default: the direct mode of the module note.
pub(in crate::viewer) fn old_path() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_PROPS_SET_OLD").is_ok_and(|v| v == "1"))
}

/// One distinct pose of a build: its token, first entry, entry count and bounding radius.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Placed {
	token: u64,
	base: usize,
	n: usize,
	rad: f32,
}

/// The last build's distinct poses (in order) and this build's.
#[derive(Default)]
pub(in crate::viewer) struct PoseReuse {
	prev: Vec<Placed>,
	cur: Vec<Placed>,
	next: usize,
	/// The entry ranges this build wrote (ascending, adjacent ones joined).
	pub(in crate::viewer) dirty: Vec<Range<usize>>,
	/// The instance record slots this build changed (ascending, adjacent ones joined): `put_inst`.
	pub(in crate::viewer) inst_dirty: Vec<Range<usize>>,
	/// Direct mode: how many entries the GPU buffer holds as of the last upload (the mirror's length in the old mode).
	valid: usize,
	/// `build_records`' lookup of the distinct poses (source arrays and length -> first entry, radius), kept for its capacity.
	pub(in crate::viewer) bases: crate::viewer::interp::FxMap<(usize, usize, usize), (u32, f32)>,
	/// RAC_PROPS_SET_CHECK=1 (direct mode): the old mirror path run on the same inputs, to compare with the GPU's bytes.
	pub(in crate::viewer) shadow: Option<Box<super::super::gpuskin::check::Shadow>>,
}

/// Where `build_records` puts the pose entries: the CPU mirror (old path) or a job list (direct mode).
pub(in crate::viewer) enum PoseSink<'p, 'a> {
	Mirror(&'p mut Vec<PoseEntry>),
	Jobs(&'p mut Vec<PoseJob<'a>>),
}

/// [PC] nprops: one distinct pose of a direct-mode build: its first entry, entry count, source arrays, and whether the GPU
/// buffer does not hold it yet (token, place or length changed). Jobs come in ascending, gapless `base` order.
pub(in crate::viewer) struct PoseJob<'a> {
	pub(in crate::viewer) base: usize,
	pub(in crate::viewer) n: usize,
	pub(in crate::viewer) ps: &'a [[f32; 3]],
	pub(in crate::viewer) ns: &'a [[f32; 3]],
	pub(in crate::viewer) write: bool,
}

/// The entry of a pose source vertex: position, then normal.
#[inline]
pub(in crate::viewer) fn entry(p: &[f32; 3], q: &[f32; 3]) -> PoseEntry {
	[p[0], p[1], p[2], q[0], q[1], q[2]]
}

impl PoseReuse {
	pub(in crate::viewer) fn begin(&mut self) {
		self.cur.clear();
		self.dirty.clear();
		self.inst_dirty.clear();
		self.next = 0;
	}

	/// Places the next distinct pose (`n` entries from `ps` / `ns`, token `token`) after the ones before it: the entries of
	/// `pose` are reused when last build had the same token at the same place with the same length, else written. Gives the
	/// first entry and the pose's bounding radius (`rad`, computed only for a written pose).
	pub(in crate::viewer) fn place(&mut self, pose: &mut Vec<PoseEntry>, token: u64, ps: &[[f32; 3]], ns: &[[f32; 3]], n: usize, rad: impl FnOnce() -> f32) -> (u32, f32) {
		let base = self.next;
		let fits = base + n <= pose.len();
		let rad = match self.prev.get(self.cur.len()) {
			Some(p) if token != 0 && fits && p.token == token && p.base == base && p.n == n => p.rad,
			_ => {
				let entries = ps[..n].iter().zip(&ns[..n]).map(|(p, q)| [p[0], p[1], p[2], q[0], q[1], q[2]]);
				if fits {
					for (d, e) in pose[base..base + n].iter_mut().zip(entries) {
						*d = e;
					}
				} else {
					pose.truncate(base);
					pose.extend(entries);
				}
				if n > 0 {
					match self.dirty.last_mut() {
						Some(r) if r.end == base => r.end = base + n,
						_ => self.dirty.push(base..base + n),
					}
				}
				rad()
			}
		};
		self.cur.push(Placed { token, base, n, rad });
		self.next = base + n;
		(base as u32, rad)
	}

	/// The next build writes every pose (the GPU buffer is new or its layout was rebuilt): direct mode's `valid` back to empty.
	pub(in crate::viewer) fn forget(&mut self) {
		self.valid = 0;
		if let Some(sh) = self.shadow.as_mut() {
			sh.forget();
		}
	}

	/// `place` for the direct mode (see the module note): the same decision, no entries copied; the pose joins `jobs`. A pose
	/// with n = 0 has nothing to write and no job.
	pub(in crate::viewer) fn place_direct<'a>(&mut self, jobs: &mut Vec<PoseJob<'a>>, token: u64, ps: &'a [[f32; 3]], ns: &'a [[f32; 3]], n: usize, rad: impl FnOnce() -> f32) -> (u32, f32) {
		let base = self.next;
		let fits = base + n <= self.valid;
		if let Some(sh) = self.shadow.as_mut() {
			sh.place(token, ps, ns, n);
		}
		let (write, rad) = match self.prev.get(self.cur.len()) {
			Some(p) if token != 0 && fits && p.token == token && p.base == base && p.n == n => (false, p.rad),
			_ => {
				if !fits {
					self.valid = base + n;
				}
				(true, rad())
			}
		};
		if n > 0 {
			jobs.push(PoseJob { base, n, ps, ns, write });
		}
		self.cur.push(Placed { token, base, n, rad });
		self.next = base + n;
		(base as u32, rad)
	}

	/// Ends a direct-mode build: the GPU buffer will hold exactly the placed entries.
	pub(in crate::viewer) fn finish_direct(&mut self) -> usize {
		self.valid = self.next;
		if let Some(sh) = self.shadow.as_mut() {
			sh.finish();
		}
		std::mem::swap(&mut self.prev, &mut self.cur);
		self.next
	}

	/// Ends the build: `pose` keeps exactly the placed entries.
	pub(in crate::viewer) fn finish(&mut self, pose: &mut Vec<PoseEntry>) {
		pose.truncate(self.next);
		std::mem::swap(&mut self.prev, &mut self.cur);
	}

	/// `begin` for the sink's mode (the check shadow starts with the direct mode's first build).
	pub(in crate::viewer) fn begin_in(&mut self, sink: &PoseSink) {
		self.begin();
		if let PoseSink::Jobs(_) = sink {
			if self.shadow.is_none() && super::super::gpuskin::check::on() {
				self.shadow = Some(Default::default());
			}
			if let Some(sh) = self.shadow.as_mut() {
				sh.begin();
			}
		}
	}

	/// `place` / `place_direct` by the sink's mode.
	pub(in crate::viewer) fn place_in<'a>(&mut self, sink: &mut PoseSink<'_, 'a>, token: u64, ps: &'a [[f32; 3]], ns: &'a [[f32; 3]], n: usize, rad: impl FnOnce() -> f32) -> (u32, f32) {
		match sink {
			PoseSink::Mirror(pose) => self.place(pose, token, ps, ns, n, rad),
			PoseSink::Jobs(jobs) => self.place_direct(jobs, token, ps, ns, n, rad),
		}
	}

	/// `finish` / `finish_direct` by the sink's mode: the number of entries placed.
	pub(in crate::viewer) fn finish_in(&mut self, sink: PoseSink) -> usize {
		match sink {
			PoseSink::Mirror(pose) => {
				let total = self.next;
				self.finish(pose);
				total
			}
			PoseSink::Jobs(_) => self.finish_direct(),
		}
	}
}

/// Slot `si` of `inst` (last upload's records; slots are put in order) becomes `rec`. A record bit-equal to the one already there
/// is left alone and not listed in `dirty`; a new slot is appended. Same invariant as the poses: after each `PropsGpu::upload` the
/// GPU buffer holds `inst` (a layout rebuild empties `inst`: then every slot is written).
pub(in crate::viewer) fn put_inst(dirty: &mut Vec<Range<usize>>, inst: &mut Vec<PropInst>, si: usize, rec: PropInst) {
	if si < inst.len() {
		if bytemuck::bytes_of(&inst[si]) == bytemuck::bytes_of(&rec) {
			return;
		}
		inst[si] = rec;
	} else {
		debug_assert_eq!(si, inst.len());
		inst.push(rec);
	}
	match dirty.last_mut() {
		Some(r) if r.end == si => r.end = si + 1,
		_ => dirty.push(si..si + 1),
	}
}

/// The pose entries slot record `s` reads, as a record rebased to entry 0 plus those entries: the mirror's own slice from
/// `k[2]` (old mode, `jobs` None) or converted from the direct-mode job that starts at `k[2]` (the clipper, the prompt batch).
pub(in crate::viewer) fn slot_pose<'a>(mirror: &'a [PoseEntry], jobs: Option<&[PoseJob]>, s: &PropInst) -> (PropInst, Cow<'a, [PoseEntry]>) {
	let Some(jobs) = jobs else { return (*s, Cow::Borrowed(mirror)) };
	let at = jobs.partition_point(|j| j.base < s.k[2] as usize);
	let own = jobs.get(at).filter(|j| j.base == s.k[2] as usize);
	let n = (s.k[3] as usize).min(own.map_or(0, |j| j.n));
	let entries = own.map_or_else(Vec::new, |j| j.ps[..n].iter().zip(&j.ns[..n]).map(|(p, q)| entry(p, q)).collect());
	(PropInst { k: [s.k[0], s.k[1], 0, s.k[3]], ..*s }, Cow::Owned(entries))
}

/// [PC] nprops: the writes of a direct-mode upload, as ranges of `jobs` (each: from its first to its last job; the unwritten ones
/// inside are converted again): written jobs whose entries lie at most `gap` apart share a run. The same runs `spans(dirty, gap)`
/// gives for the old path's `dirty` ranges (those are the written jobs' entries, adjacent ones joined).
pub(in crate::viewer) fn pose_runs(jobs: &[PoseJob], gap: usize) -> Vec<Range<usize>> {
	let mut out: Vec<Range<usize>> = Vec::with_capacity(8);
	let mut end = 0;
	for (i, j) in jobs.iter().enumerate().filter(|(_, j)| j.write) {
		match out.last_mut() {
			Some(o) if j.base <= end + gap => o.end = i + 1,
			_ => out.push(i..i + 1),
		}
		end = j.base + j.n;
	}
	out
}

/// Entry ranges `dirty` (ascending, disjoint) joined across gaps of at most `gap` entries: fewer, larger GPU writes (a gap's
/// entries are equal on both sides, so writing them again changes nothing).
pub(in crate::viewer) fn spans(dirty: &[Range<usize>], gap: usize) -> Vec<Range<usize>> {
	let mut out: Vec<Range<usize>> = Vec::with_capacity(dirty.len());
	for r in dirty {
		match out.last_mut() {
			Some(o) if r.start <= o.end + gap => o.end = o.end.max(r.end),
			_ => out.push(r.clone()),
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	fn pose_of(v: f32, n: usize) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
		((0..n).map(|i| [v, i as f32, 0.0]).collect(), (0..n).map(|i| [0.0, 0.0, v + i as f32]).collect())
	}

	/// What `build_records` wrote before: every pose copied in order.
	fn full(poses: &[&(Vec<[f32; 3]>, Vec<[f32; 3]>)]) -> Vec<PoseEntry> {
		poses.iter().flat_map(|(ps, ns)| ps.iter().zip(ns).map(|(p, q)| [p[0], p[1], p[2], q[0], q[1], q[2]])).collect()
	}

	fn build(r: &mut PoseReuse, pose: &mut Vec<PoseEntry>, list: &[(u64, &(Vec<[f32; 3]>, Vec<[f32; 3]>))]) -> Vec<(u32, f32)> {
		r.begin();
		let out = list.iter().map(|(t, (ps, ns))| r.place(pose, *t, ps, ns, ps.len(), || ps.len() as f32)).collect();
		r.finish(pose);
		out
	}

	#[test]
	fn reuses_equal_tokens_in_place_and_writes_the_rest() {
		let (a, b, c, b2) = (pose_of(1.0, 3), pose_of(2.0, 4), pose_of(3.0, 2), pose_of(9.0, 4));
		let (mut r, mut pose) = (PoseReuse::default(), Vec::new());
		build(&mut r, &mut pose, &[(1, &a), (2, &b), (3, &c)]);
		assert_eq!(pose, full(&[&a, &b, &c]));
		assert_eq!(r.dirty, vec![0..9]);
		// Same tokens: nothing written.
		let out = build(&mut r, &mut pose, &[(1, &a), (2, &b), (3, &c)]);
		assert!(r.dirty.is_empty());
		assert_eq!(out, vec![(0, 3.0), (3, 4.0), (7, 2.0)]);
		assert_eq!(pose, full(&[&a, &b, &c]));
		// The middle pose is new: only its range.
		build(&mut r, &mut pose, &[(1, &a), (4, &b2), (3, &c)]);
		assert_eq!(r.dirty, vec![3..7]);
		assert_eq!(pose, full(&[&a, &b2, &c]));
		// Token 0 is always written.
		build(&mut r, &mut pose, &[(0, &a), (4, &b2), (3, &c)]);
		assert_eq!(r.dirty, vec![0..3]);
	}

	#[test]
	fn a_moved_or_resized_layout_is_written_from_there_on() {
		let (a, b, c, a5) = (pose_of(1.0, 3), pose_of(2.0, 4), pose_of(3.0, 2), pose_of(5.0, 5));
		let (mut r, mut pose) = (PoseReuse::default(), Vec::new());
		build(&mut r, &mut pose, &[(1, &a), (2, &b), (3, &c)]);
		// A longer first pose shifts the others: all written, the vector grows.
		build(&mut r, &mut pose, &[(5, &a5), (2, &b), (3, &c)]);
		assert_eq!(pose, full(&[&a5, &b, &c]));
		assert_eq!(r.dirty, vec![0..11]);
		// A pose dropped: the tail shrinks.
		build(&mut r, &mut pose, &[(5, &a5), (2, &b)]);
		assert_eq!(pose, full(&[&a5, &b]));
		assert!(r.dirty.is_empty());
		// An emptied vector (layout rebuild) reuses nothing although the tokens match.
		pose.clear();
		build(&mut r, &mut pose, &[(5, &a5), (2, &b)]);
		assert_eq!(pose, full(&[&a5, &b]));
		assert_eq!(r.dirty, vec![0..9]);
		// Fewer entries than last time at the same token: written.
		let short = (a5.0[..2].to_vec(), a5.1[..2].to_vec());
		build(&mut r, &mut pose, &[(5, &short), (2, &b)]);
		assert_eq!(pose, full(&[&short, &b]));
	}

	#[test]
	fn put_inst_lists_only_changed_slots() {
		let rec = |k: u32| PropInst { k: [k, 0, 0, 0], ..PropInst::hidden() };
		let (mut inst, mut dirty) = (Vec::new(), Vec::new());
		for (i, k) in [1, 2, 3, 4].into_iter().enumerate() {
			put_inst(&mut dirty, &mut inst, i, rec(k));
		}
		assert_eq!(dirty, vec![0..4]);
		dirty.clear();
		for (i, k) in [1, 9, 8, 4, 5].into_iter().enumerate() {
			put_inst(&mut dirty, &mut inst, i, rec(k));
		}
		assert_eq!(dirty, vec![1..3, 4..5]);
		assert_eq!(inst, [1, 9, 8, 4, 5].map(rec));
		dirty.clear();
		for (i, k) in [1, 9, 8, 4, 5].into_iter().enumerate() {
			put_inst(&mut dirty, &mut inst, i, rec(k));
		}
		assert!(dirty.is_empty());
	}

	#[test]
	fn spans_join_small_gaps() {
		assert_eq!(spans(&[0..2, 3..5, 20..21], 1), vec![0..5, 20..21]);
		assert_eq!(spans(&[0..2, 3..5, 20..21], 0), vec![0..2, 3..5, 20..21]);
		assert_eq!(spans(&[], 8), Vec::<Range<usize>>::new());
		assert_eq!(spans(&[4..6, 8..9], 2), vec![4..9]);
	}

	/// A tiny deterministic generator for the frame sequences below.
	struct Lcg(u64);

	impl Lcg {
		fn next(&mut self, n: u64) -> u64 {
			self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
			(self.0 >> 33) % n
		}
	}

	/// The pose a token stands for (equal tokens = bit-equal poses, as in handler/pose_born.rs); token 0 = fresh data per call.
	fn pose_for(tok: u64, salt: u64) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
		let n = if tok == 0 { (salt % 6) as usize } else if tok % 11 == 0 { 0 } else { 1 + (tok % 7) as usize };
		let v = if tok == 0 { 1000.0 + salt as f32 } else { tok as f32 };
		((0..n).map(|i| [v, i as f32, 0.5]).collect(), (0..n).map(|i| [0.25, v + i as f32, -1.0]).collect())
	}

	/// The old path (mirror + `spans` of `dirty`) and the direct mode (jobs + `pose_runs`) on the same random frames, with a
	/// simulated GPU buffer each (new ones on growth): same decisions (bases, radii) and the same buffer bytes after every build,
	/// across token reuse, token 0, empty poses, layout rebuilds, growth and shrinking.
	#[test]
	fn direct_mode_matches_the_mirror_path() {
		// The upload's gap (gpuskin.rs POSE_GAP), far wider than the test poses (1..7 entries): reused poses between written ones are
		// filled from their sources often.
		const GAP: usize = 64;
		let mut rng = Lcg(7);
		let (mut old, mut direct) = (PoseReuse::default(), PoseReuse::default());
		let (mut mirror, mut cap) = (Vec::<PoseEntry>::new(), 8usize);
		let (mut gpu_old, mut gpu_new) = (vec![[f32::NAN; 6]; cap], vec![[f32::NAN; 6]; cap]);
		let mut toks: Vec<u64> = Vec::new();
		let (mut reused, mut gap_filled) = (0, 0);
		// Each path's radius maker gives a new value per call, so a reuse (the kept radius) and a write (a fresh one) differ.
		let (rad_old, rad_new) = (std::cell::Cell::new(0.0f32), std::cell::Cell::new(0.0f32));
		let next = |c: &std::cell::Cell<f32>| {
			c.set(c.get() + 1.0);
			c.get()
		};
		for frame in 0..3000u64 {
			if rng.next(25) == 0 {
				// A layout rebuild: nothing the GPU holds counts any more.
				mirror.clear();
				toks.clear();
				direct.forget();
			}
			for i in 0..(1 + rng.next(12)) as usize {
				match toks.get(i) {
					Some(_) if rng.next(10) < 7 => {}
					_ => {
						let t = match rng.next(6) {
							0 => 0,
							_ => 1 + rng.next(30),
						};
						if i < toks.len() { toks[i] = t } else { toks.push(t) }
					}
				}
			}
			toks.truncate(1 + rng.next(14) as usize);
			let poses: Vec<_> = toks.iter().enumerate().map(|(i, &t)| (t, pose_for(t, frame * 100 + i as u64))).collect();
			let full = rng.next(40) == 0;
			let tok = |t: u64| if full { 0 } else { t };
			old.begin();
			let out_old: Vec<_> = poses.iter().map(|(t, (ps, ns))| old.place(&mut mirror, tok(*t), ps, ns, ps.len(), || next(&rad_old))).collect();
			old.finish(&mut mirror);
			let mut jobs = Vec::new();
			direct.begin();
			let out_new: Vec<_> = poses.iter().map(|(t, (ps, ns))| direct.place_direct(&mut jobs, tok(*t), ps, ns, ps.len(), || next(&rad_new))).collect();
			let total = direct.finish_direct();
			assert_eq!(out_old, out_new, "frame {frame}");
			assert_eq!(total, mirror.len(), "frame {frame}");
			// Old upload.
			let grown = total > cap;
			if grown {
				cap = total.next_power_of_two();
				gpu_old = vec![[f32::NAN; 6]; cap];
				gpu_new = vec![[f32::NAN; 6]; cap];
			}
			if grown || full {
				gpu_old[..total].copy_from_slice(&mirror);
			} else {
				for r in spans(&old.dirty, GAP) {
					gpu_old[r.clone()].copy_from_slice(&mirror[r]);
				}
			}
			// Direct upload: runs of whole jobs converted from the sources.
			let mut convert = |js: &[PoseJob]| {
				for j in js {
					for (k, (p, q)) in j.ps[..j.n].iter().zip(&j.ns[..j.n]).enumerate() {
						gpu_new[j.base + k] = entry(p, q);
					}
				}
			};
			if grown || full {
				convert(&jobs);
			} else {
				for r in pose_runs(&jobs, GAP) {
					convert(&jobs[r]);
				}
			}
			// The runs are the old spans, entry for entry.
			reused += jobs.iter().filter(|j| !j.write).count();
			gap_filled += pose_runs(&jobs, GAP).iter().map(|r| jobs[r.clone()].iter().filter(|j| !j.write).count()).sum::<usize>();
			if !(grown || full) {
				let runs: Vec<Range<usize>> = pose_runs(&jobs, GAP).into_iter().map(|r| jobs[r.start].base..jobs[r.end - 1].base + jobs[r.end - 1].n).collect();
				assert_eq!(runs, spans(&old.dirty, GAP), "frame {frame}");
			}
			let bytes = |g: &[PoseEntry]| g[..total].iter().flat_map(|e| bytemuck::bytes_of(e).to_vec()).collect::<Vec<u8>>();
			assert_eq!(bytes(&gpu_old), bytes(&gpu_new), "frame {frame}");
			assert_eq!(bytes(&gpu_new), poses.iter().flat_map(|(_, (ps, ns))| ps.iter().zip(ns).flat_map(|(p, q)| bytemuck::bytes_of(&entry(p, q)).to_vec())).collect::<Vec<u8>>(), "frame {frame}: the buffer holds the sources");
		}
		// The sequence did exercise reuse and runs that span unwritten poses.
		assert!(reused > 200 && gap_filled > 20, "reused {reused} gap_filled {gap_filled}");
	}

	#[test]
	fn slot_pose_gives_the_slots_entries_rebased() {
		let (a, b, c) = (pose_of(1.0, 3), pose_of(2.0, 4), pose_of(3.0, 2));
		let mut r = PoseReuse::default();
		let mut jobs = Vec::new();
		r.begin();
		let bases: Vec<u32> = [&a, &b, &c].iter().map(|(ps, ns)| r.place_direct(&mut jobs, 0, ps, ns, ps.len(), || 0.0).0).collect();
		let mirror = full(&[&a, &b, &c]);
		for (si, (base, n)) in [(0, 3u32), (1, 4), (2, 2), (1, 2), (2, 0)].into_iter().enumerate() {
			let s = PropInst { k: [crate::viewer::gpuskin::KIND_PROP, u32::MAX, bases[base], n], ..PropInst::hidden() };
			let (rec, entries) = slot_pose(&mirror, Some(&jobs), &s);
			assert_eq!(rec.k, [s.k[0], s.k[1], 0, n], "slot {si}");
			assert_eq!(&entries[..], &mirror[bases[base] as usize..(bases[base] + n) as usize], "slot {si}");
			let (rec, entries) = slot_pose(&mirror, None, &s);
			assert_eq!((rec, entries.len()), (s, mirror.len()));
		}
		// A record that reads nothing (hidden, base 0, no entries).
		assert!(slot_pose(&mirror, Some(&jobs), &PropInst::hidden()).1.is_empty());
	}
}
