//! A level's texture-pack replacements in two halves (engine split stage E6, the R&C2 pack): `prepare` names the level's textures and
//! picks the pack files on any thread (it reads only the level and the textures it is given), `Prepared::start` starts the loaders
//! and leaves the replacements for the scene build of the calling thread (`repl::set_thread_repl`). R&C1 runs both on its level job
//! thread (`level_loaded`); R&C2 decodes the level on a worker and builds the scene on the render thread (`adopt`).

use super::repl::{self, Job};
use super::{jobs, names, packs, refresh, source};
use crate::level::{Level, Texture};
use std::collections::HashMap;

/// The replacement jobs of one level, ready to start.
pub struct Prepared {
	level: u32,
	jobs: Vec<Job>,
}

impl Prepared {
	pub(super) fn jobs(&self) -> &[Job] {
		&self.jobs
	}

	/// Starts the loader threads and leaves the registry for the next scene build on this thread; nothing when no file matched.
	pub fn start(self) {
		if !self.jobs.is_empty() {
			let r = repl::LevelRepl::new(self.level, self.jobs);
			r.start(jobs::loader_threads());
			repl::set_thread_repl(Some(r));
		}
	}

	/// Number of world textures that have a replacement.
	pub fn len(&self) -> usize {
		self.jobs.len()
	}

	pub fn is_empty(&self) -> bool {
		self.jobs.is_empty()
	}
}

/// The render thread's half for a game that prepared its level elsewhere: the scene build that runs next on this thread gets `p`'s
/// replacements, or none (what the previous level left is dropped).
pub fn adopt(p: Option<Prepared>) {
	repl::set_thread_repl(None);
	if let Some(p) = p {
		p.start();
	}
}

/// With a textures mod active: the PCSX2 names of the level's textures (`source::level_sources`), the pack files they find (the
/// PAL <-> USA twin names too when `twins`: R&C1's table) and the jobs for the textures in `textures` (`exclude` keys keep their
/// original). None with no mod active (nothing else runs then) or when the level's tables cannot be read.
pub fn prepare(l: &Level, textures: &HashMap<u32, Texture>, exclude: &[u32], twins: bool) -> Option<(Prepared, names::LevelHits)> {
	refresh();
	let p = packs()?;
	let t = std::time::Instant::now();
	let (srcs, skipped) = match source::level_sources(l) {
		Ok(s) => s,
		Err(e) => {
			eprintln!("texpack: level {}: cannot read the texture tables: {e}", l.id);
			return None;
		}
	};
	let hits = if twins { names::resolve(&srcs, p.mip_names, &p.index) } else { names::resolve_with(&srcs, p.mip_names, &p.index, &super::xmap::XMap::default()) };
	eprintln!(
		"texpack: level {}: {} textures ({} unreadable), {} names, {} found in the active packs ({} through the PAL/USA name map; {} textures with a replacement), {:.0} ms",
		l.id, hits.textures, skipped, hits.names, hits.hit_names, hits.mapped_names, hits.textures_hit, t.elapsed().as_secs_f64() * 1000.0
	);
	let jobs = jobs::make_jobs(&srcs, &hits, textures, exclude);
	Some((Prepared { level: l.id as u32, jobs }, hits))
}
