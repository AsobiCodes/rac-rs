//! [PC] PCSX2 texture packs (lane tex, DESIGN.md in scratch_texpack): the game loads replacement textures made for PCSX2 unchanged.
//! Stage S1: names and index only; nothing is drawn differently yet. With no textures mod active nothing here runs after the
//! single `mods/active.txt` check, and the game is exactly as without this module.
//!
//! - `hash`: XXH3_64 TEX0 hash (GS block stream), CLUT hash, `bits` field.
//! - `name`: `TexName` and the PCSX2 file-name parser (six formats, bit 14 masked, `-mipN` files refused).
//! - `gsname`: the PCSX2 texture name of any draw (GS local memory + TEX0/CLAMP/TEXA/TEXCLUT), for HUD/menu/font/effect classes; `discover`: the
//!   RAC_TEXNAMES collector (stage A measurement, zero cost when unset).
//! - `layout`: where a hand-copied pack's images are (documented `textures` folder, or PCSX2 `replacements` shapes).
//! - `json`, `mods`: the mods contract (`mods/active.txt`, `mods/<id>/mod.json`, images in `mods/<id>/textures`).
//! - `active`: `mods/active.txt` in the launcher's exact format (master ticks, held ids, foreign comment lines, BOM / CRLF in, LF out);
//!   `store`: the installed mods plus that file as one value, what the in-game Mods page (retail/pcmods.rs) lists and saves.
//! - `index`: `PackIndex`, name -> file over the enabled mods' folders.
//! - `bcenc`: RGBA8 -> BC3 for a chain that would not fit the memory budget (PNG packs).
//! - `inflate`, `png`, `dds`, `bc`, `bc7dec`: dependency-free decoders (zlib, PNG, DDS headers, BC1/2/3, BC7); `bc7alpha`: the BC7 file alpha rule; `chain`: mip chain assembly; `repl`: the per-level
//!   replacement registry (`LevelRepl`, async decode).
//! - `hudrepl`: replacement images for the GPU capture's HUD / menu / text textures, looked up by their run-time name (async decode,
//!   memory budget); nothing runs without a pack.
//! - `front_hd`: the same for the pictures the mirror draws itself (title logo, Press Start, the menu panel frame): named from the disc data, replaced
//!   in the decoded `Texture`; `hud::Sprite::name` names the HUD sprites.
//! - `hudprefetch`: starts the decoding of a level's HUD pictures at level load (no pop-in on the first frames).
//! - `source`: the raw PSMT8 tables of a level; `names`: their PCSX2 names and the lookup in the index; `xmap`: the PAL <-> USA twin names
//!   (`assets/texmap.tsv`) tried for a name the pack lacks; `levelrule`: which files a texture's replacement is built from.
//! - `extra`, `extra_hd`, `extra_grass`: the textures outside the four world tables (fx / particles / chrome / glass / sky / ship / warp
//!   bundle): their names, the level-load hooks that swap the decoded image into the texture, and the grass colour from the HD ground.

pub mod active;
pub mod bc;
pub mod bc7alpha;
pub mod bc7dec;
pub mod bcenc;
pub mod chain;
pub mod dds;
pub mod discover;
pub mod extra;
pub mod extra_grass;
pub mod extra_hd;
pub mod front_hd;
pub mod gsname;
pub mod guard;
pub mod hash;
pub mod hudprefetch;
pub mod hudrepl;
pub mod inflate;
pub mod index;
pub mod jobs;
pub mod json;
pub mod layout;
pub mod levelrule;
pub mod mods;
pub mod name;
pub mod names;
pub mod png;
pub mod repl;
pub mod source;
pub mod store;
pub mod xmap;

use crate::level::Level;
use index::PackIndex;
use mods::{Active, MipNames};
use std::sync::{Arc, Mutex};

/// The enabled textures mods and their index, built once at the first level load.
pub struct Packs {
	pub active: Active,
	pub index: PackIndex,
	/// Mip naming to try: the one every mod agrees on, else both.
	pub mip_names: MipNames,
}

/// The packs and the text of `active.txt` they were built from. The game's Mods page (retail/pcmods.rs) rewrites that file; the next
/// level load (`refresh`) sees the changed text and builds the packs again, so a texture pack switched on or off in the menu applies
/// at the next planet.
static PACKS: Slot = Mutex::new(None);

/// Scans `root` (the mods folder) and indexes the enabled textures mods; None when there are none.
pub fn load_packs(root: &std::path::Path) -> Option<Packs> {
	let active = mods::load_active(root);
	for n in &active.notes {
		crate::loadlog::say(|| format!("texpack: {n}"));
	}
	if active.mods.is_empty() {
		return None;
	}
	let mut index = PackIndex::new();
	for (i, m) in active.mods.iter().enumerate() {
		let st = index.scan_dir(&m.textures, i);
		eprintln!("texpack: mod {} ({}): {} replacement names ({} duplicates, {} mip dumps skipped, {} other files)", m.id, m.version, st.added, st.duplicates, st.mip_dumps, st.other);
	}
	let mip_names = active.mods.iter().map(|m| m.mip_names).reduce(|a, b| if a == b { a } else { MipNames::Both }).unwrap_or(MipNames::Both);
	Some(Packs { active, index, mip_names })
}

type Slot = Mutex<Option<(String, Option<Arc<Packs>>)>>;

fn active_text_at(root: Option<&std::path::Path>) -> String {
	root.and_then(|r| std::fs::read(r.join("active.txt")).ok()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

fn build_at(root: Option<&std::path::Path>) -> (String, Option<Arc<Packs>>) {
	(active_text_at(root), root.and_then(load_packs).map(Arc::new))
}

fn build() -> (String, Option<Arc<Packs>>) {
	build_at(mods::mods_root().as_deref())
}

/// The active packs (read at the first look, then at a level load after `active.txt` changed; None when no textures mod is enabled
/// or there is no mods folder).
pub fn packs() -> Option<Arc<Packs>> {
	if let Some((_, p)) = &*PACKS.lock().unwrap_or_else(|e| e.into_inner()) {
		return p.clone();
	}
	let b = build();
	PACKS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(b).1.clone()
}

/// When `active.txt` under `root` is not the text the packs in `slot` were built from, builds them again (the scan runs outside the
/// lock); an empty slot is left to `packs()`. True when it rebuilt.
fn refresh_slot(slot: &Slot, root: Option<&std::path::Path>) -> bool {
	let text = active_text_at(root);
	let same = slot.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_none_or(|(t, _)| *t == text);
	if !same {
		eprintln!("texpack: active.txt changed since the packs were read: reloading them");
		let b = build_at(root);
		*slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(b);
	}
	!same
}

/// Level load: the packs are built again when the Mods page (or the launcher) changed `active.txt` since they were read.
fn refresh() {
	refresh_slot(&PACKS, mods::mods_root().as_deref());
}

/// Hook at level load (`viewer/app.rs load_level_from`): with a textures mod active, computes the PCSX2 names of the level's
/// textures, logs how many the active packs have and starts the replacement loaders (`jobs`). The replacements travel to the scene
/// build of the same thread (`repl::set_thread_repl`). `exclude` = texture keys that must keep their original (a liquid drawn as
/// terrain), `front` = the title screen level (not replaced). Returns the hits; None with no mod (nothing else runs then).
pub fn level_loaded(l: &Level, textures: &std::collections::HashMap<u32, crate::level::Texture>, exclude: &[u32], front: bool) -> Option<names::LevelHits> {
	repl::set_thread_repl(None);
	if front {
		return None;
	}
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
	let hits = names::resolve(&srcs, p.mip_names, &p.index);
	eprintln!(
		"texpack: level {}: {} textures ({} unreadable), {} names, {} found in the active packs ({} through the PAL/USA name map; {} textures with a replacement), {:.0} ms",
		l.id, hits.textures, skipped, hits.names, hits.hit_names, hits.mapped_names, hits.textures_hit, t.elapsed().as_secs_f64() * 1000.0
	);
	let jobs = jobs::make_jobs(&srcs, &hits, textures, exclude);
	extra_grass::remember(l.id as u32, &jobs);
	if !jobs.is_empty() {
		let r = repl::LevelRepl::new(l.id as u32, jobs);
		r.start(jobs::loader_threads());
		repl::set_thread_repl(Some(r));
	}
	Some(hits)
}

/// The BC texture formats are requested from the GPU whenever the adapter has them: the pack folder is not scanned while the device is
/// made, and a textures mod switched on later (Mods page) still gets BC textures. Without a pack nothing uses the feature.
///
/// This is also where the loaders learn whether they may hand BC chains to the GPU as they are (`repl::set_bc_native`): `Gpu::new` makes
/// the device with exactly the features returned here (no fallback descriptor), so the device has BC iff this returns it. Do not move
/// that into a call after the device is made (the `gpu_features_is_called_for_real` test reads `gpu/setup.rs`).
pub fn gpu_features(adapter: &wgpu::Adapter) -> wgpu::Features {
	bc_features(adapter.features())
}

/// `gpu_features` for the feature set `have` of an adapter.
fn bc_features(have: wgpu::Features) -> wgpu::Features {
	let f = have & wgpu::Features::TEXTURE_COMPRESSION_BC;
	repl::set_bc_native(!f.is_empty());
	f
}

#[cfg(test)]
mod bc_flag_tests {
	use super::*;

	/// The flag the loaders read follows the adapter's features (the unit test of `repl` used to set it by hand, which hid a call that
	/// sat inside a comment).
	#[test]
	fn the_loaders_flag_follows_the_adapter_features() {
		assert!(bc_features(wgpu::Features::TEXTURE_COMPRESSION_BC | wgpu::Features::DEPTH32FLOAT_STENCIL8) == wgpu::Features::TEXTURE_COMPRESSION_BC);
		assert!(repl::bc_native());
		assert!(bc_features(wgpu::Features::DEPTH32FLOAT_STENCIL8).is_empty());
		assert!(!repl::bc_native());
		bc_features(wgpu::Features::TEXTURE_COMPRESSION_BC);
	}

	/// `Gpu::new` must really call `gpu_features` outside of any `//` comment (no GPU needed: the source is read).
	#[test]
	fn gpu_features_is_called_for_real() {
		let src = include_str!("../viewer/gpu/setup.rs");
		let live = src.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
		assert!(live.contains("crate::texpack::gpu_features(&adapter)"), "Gpu::new does not call texpack::gpu_features outside a comment");	}
}

#[cfg(test)]
mod active_tests;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod dump_tests;
