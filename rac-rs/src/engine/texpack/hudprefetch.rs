//! [PC] Pop-in cure for the GPU capture's replacement images (stage 3 of lane v041_hud): without it the first frames of a scene show the
//! disc's HUD pictures while the pack's files decode (`hudrepl::State::Pending`). The level load knows which HUD pictures the scene
//! will draw (the level's HUD sprites and the menu panel fx records are named from the disc data, `hud::Sprite::name`,
//! `front_hd::fx_name`), so it hands their names here and a background thread asks the registry for them in small batches, which
//! starts the decoding at once; by the time the capture draws the first HUD prim the images are `Ready`.
//!
//! Bounded: a batch is asked for only while the decoded-but-unused images in the registry stay under `CAP_MB` (images leave that
//! count when a capture takes them, `Slot::release`, and at every level load, before this runs, the images nobody took are dropped,
//! `hudrepl::drop_unused`, so the count is this level's prefetch only), and the whole list is asked once per name and level (a name
//! dropped at an earlier level load is decoded again by `lookup`). The registry's own budget (`hudrepl::BUDGET_MB`) and the atlas size
//! (`RAC_TEXPACK_HUD_MB`) are unchanged. With no pack (or `RAC_TEXPACK_HUD=0`) `start` returns at its first line.

use super::hudrepl::{self, Slot, State};
use super::name::TexName;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering::Acquire, Ordering::SeqCst};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The level load a prefetch thread belongs to: a thread of an older load stops asking at its next batch (R4: it would otherwise
/// re-arm names the level load just dropped).
static GEN: AtomicU64 = AtomicU64::new(0);

/// Starts a new generation (older prefetch threads are superseded).
fn begin() -> u64 {
	GEN.fetch_add(1, SeqCst) + 1
}

/// Bytes waiting in the registry, or `usize::MAX` (the cap is reached: `run` stops) once generation `generation` was superseded.
fn used_unless_superseded(generation: u64) -> usize {
	if GEN.load(Acquire) != generation { usize::MAX } else { hudrepl::used_bytes() }
}

/// Most MB of decoded images prefetching may leave waiting for the atlas.
pub const CAP_MB: usize = 128;
/// Names asked for at a time (the registry has two loader threads).
const BATCH: usize = 8;
/// Longest wait for one batch (a broken file ends in `Broken`, a huge one is slow, neither blocks the rest for long).
const BATCH_WAIT: Duration = Duration::from_secs(20);

/// What one `run` did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Done {
	/// Distinct names asked for.
	pub asked: usize,
	/// ... that were `Ready` when the run looked at them again.
	pub ready: usize,
	/// ... that the pack has no usable file for.
	pub other: usize,
	/// The run stopped because the waiting images reached the cap; names after the stop were not asked for.
	pub capped: bool,
}

/// Asks `lookup` for each distinct name in batches and waits for the batch to leave `Pending` before the next, stopping at `cap` bytes
/// of waiting images (`used`). Blocking; `start` runs it on a thread.
pub fn run(names: &[TexName], lookup: impl Fn(&TexName) -> Arc<Slot>, used: impl Fn() -> usize, cap: usize) -> Done {
	let mut seen = HashSet::new();
	let uniq: Vec<&TexName> = names.iter().filter(|n| seen.insert(**n)).collect();
	let mut d = Done::default();
	for batch in uniq.chunks(BATCH) {
		if used() >= cap {
			d.capped = true;
			break;
		}
		let slots: Vec<Arc<Slot>> = batch.iter().map(|n| lookup(n)).collect();
		let t0 = Instant::now();
		while slots.iter().any(|s| s.state() == State::Pending) && t0.elapsed() < BATCH_WAIT {
			std::thread::sleep(Duration::from_millis(2));
		}
		d.asked += slots.len();
		for s in &slots {
			match s.state() {
				State::Ready | State::Released => d.ready += 1,
				_ => d.other += 1,
			}
		}
	}
	d
}

/// Level load: starts decoding the pack images of `names` in the background. Returns at once; no thread without a pack.
pub fn start(names: Vec<TexName>) {
	if !hudrepl::enabled() {
		return;
	}
	let generation = begin();
	if names.is_empty() {
		return;
	}
	let _ = std::thread::Builder::new().name("texpack-hud-prefetch".into()).spawn(move || {
		let t0 = Instant::now();
		let d = run(&names, hudrepl::lookup, move || used_unless_superseded(generation), CAP_MB << 20);
		eprintln!("texpack hud: prefetch {} names, {} ready, {} without a usable file{} in {:.0} ms", d.asked, d.ready, d.other, if !d.capped { "" } else if GEN.load(Acquire) != generation { ", stopped: the next level load took over" } else { ", stopped at the memory cap" }, t0.elapsed().as_secs_f64() * 1000.0);
	});
}

#[cfg(test)]
#[path = "hudprefetch_tests.rs"]
mod tests;
