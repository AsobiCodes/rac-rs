//! PC perf (speed lane A, B1): the hero's posed bundle is computed once per game tick, not per display frame.
//! `HeroCache` holds the result of the hero skinning block in handler.rs (hero + glove + held items + back objects + wrench + ghosts)
//! with the side values derived with it. A display frame that ran no tick and has an equal `HeroKey` reuses it; the per-frame
//! blend (`smooth.hero`) stays per frame. RAC_HERO_CHECK=1 computes the fresh result every frame as well, compares it with the
//! cached one that would have been reused (byte for byte, float bits) and counts mismatches (stderr `hero_check:` lines).
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) type HeroKey = [u64; 14];

pub(super) struct HeroCache {
	pub key: HeroKey,
	/// [PC] perf (lane R2): pose generation number, see `next_gen`.
	pub pose_gen: u64,
	pub skinned: Option<Posed>,
	pub ghost_ranges: [Option<(usize, usize, u8)>; 5],
	pub bomb_range: Option<(usize, usize)>,
	pub light_at: Option<[f32; 3]>,
	pub held_rows: Option<[[f32; 3]; 3]>,
	pub muzzle_model: Option<[f32; 3]>,
	pub held_muzzle_model: Option<[f32; 3]>,
	pub held_joint0_model: Option<[[f32; 3]; 3]>,
	pub held_xform_model: Option<([[f32; 3]; 3], [f32; 3])>,
}

static GEN: AtomicU64 = AtomicU64::new(1);

/// A new pose generation number: every freshly computed hero pose gets one, and a copy that is not modified keeps it, so equal numbers
/// mean equal pose bytes (the GPU hero skips its byte compare, `HeroGpu::set_poses`). Never 0 (0 = unknown).
pub(super) fn next_gen() -> u64 {
	GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

static FRAMES: AtomicU64 = AtomicU64::new(0);
static COMPARED: AtomicU64 = AtomicU64::new(0);
static MISMATCH: AtomicU64 = AtomicU64::new(0);

pub(super) fn hero_check_on() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_HERO_CHECK").is_ok_and(|v| v == "1"))
}

/// Everything the posed bundle is a function of that can change without a tick: identity of the rig, the level, the field and
/// play-tick counters (a tick changes them), the hero's placement and the equipment flags. The state itself only changes in ticks.
/// `anim` = address of the hero rig's class (identity), `field` = the retail field counter (u64::MAX without retail),
/// `counts` = layer / loose wrench / held rig / hero ghost counts, `flags` = parts present, wrench rig present.
#[allow(clippy::too_many_arguments)]
pub(super) fn hero_cache_key(anim: usize, level: usize, play_ticks: u32, field: Option<u32>, counts: [usize; 3], flags: u64, game: &Game) -> HeroKey {
	let p = &game.player;
	let b = |v: f32| v.to_bits() as u64;
	[
		anim as u64,
		level as u64,
		play_ticks as u64,
		field.map_or(u64::MAX, |f| f as u64),
		flags,
		b(p.pos[0]) | b(p.pos[1]) << 32,
		b(p.pos[2]) | b(p.yaw) << 32,
		p.pulse_scale.map_or(u64::MAX, b),
		game.inv.equipped as u64,
		game.held_hidden as u64 | (p.clank_hidden as u64) << 1 | (game.bomb_in_hand() as u64) << 2,
		game.frames as u64,
		counts[0] as u64,
		counts[1] as u64,
		counts[2] as u64 | (p.hero_ghosts().iter().filter(|g| g.is_some()).count() as u64) << 32,
	]
}

fn same3(a: &[[f32; 3]], b: &[[f32; 3]]) -> bool {
	a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (0..3).all(|k| x[k].to_bits() == y[k].to_bits()))
}
fn bits3(v: &Option<[f32; 3]>) -> Option<[u32; 3]> {
	v.map(|v| v.map(f32::to_bits))
}
fn bits33(v: &Option<[[f32; 3]; 3]>) -> Option<[[u32; 3]; 3]> {
	v.map(|v| v.map(|r| r.map(f32::to_bits)))
}

pub(super) fn hero_check_compare(old: &HeroCache, new: &HeroCache) {
	let posed_same = match (&old.skinned, &new.skinned) {
		(Some(a), Some(b)) => same3(&a.0, &b.0) && same3(&a.1, &b.1),
		(None, None) => true,
		_ => false,
	};
	let same = posed_same
		&& old.ghost_ranges == new.ghost_ranges
		&& old.bomb_range == new.bomb_range
		&& bits3(&old.light_at) == bits3(&new.light_at)
		&& bits33(&old.held_rows) == bits33(&new.held_rows)
		&& bits3(&old.muzzle_model) == bits3(&new.muzzle_model)
		&& bits3(&old.held_muzzle_model) == bits3(&new.held_muzzle_model)
		&& bits33(&old.held_joint0_model) == bits33(&new.held_joint0_model)
		&& old.held_xform_model.map(|(r, m)| (r.map(|x| x.map(f32::to_bits)), m.map(f32::to_bits))) == new.held_xform_model.map(|(r, m)| (r.map(|x| x.map(f32::to_bits)), m.map(f32::to_bits)));
	let n = COMPARED.fetch_add(1, Ordering::Relaxed) + 1;
	if !same {
		let m = MISMATCH.fetch_add(1, Ordering::Relaxed) + 1;
		if m <= 20 {
			eprintln!("hero_check: MISMATCH #{m} (posed {posed_same})");
		}
	}
	if n % 500 == 0 {
		eprintln!("hero_check: {} reuse candidates compared, {} mismatches, {} frames", n, MISMATCH.load(Ordering::Relaxed), FRAMES.load(Ordering::Relaxed));
	}
}

pub(super) fn hero_check_frame() {
	FRAMES.fetch_add(1, Ordering::Relaxed);
}
