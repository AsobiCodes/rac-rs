//! [PC] Texture packs beyond the four world tables (lane texw): the level-load hooks that put the decoded pack image into the
//! texture of the fx table and particles (`apply_fx`), the chrome and glass of the metal packets (`hd_metal`), the sky (`hd_sky`),
//! the ship bundle (`hd_ship`) and the warp bundle (`hd_warp`). The names come from `extra.rs`; a texture whose exact PCSX2 name is
//! not in the active packs keeps its original. The HD image replaces the `Texture` itself (engine RGBA8, engine alpha scale), so
//! every consumer (GPU upload, mips, clamp / no-mip variants, particle tints) sees it as it saw the original, only larger; the UVs
//! are normalised, so nothing else changes. All of it runs once per level (or bundle) load on the load thread; with no textures mod
//! active every hook returns at its first line.

use super::extra::{self, ExtraSource, Kind, FX_KEY, PART_KEY, WARP_FX_KEY, WARP_MOBY_KEY};
use super::index::PackFile;
use super::name::TexName;
use super::{chain, jobs, repl, Packs};
use crate::level::{Level, Texture};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;

/// fx records that keep their original: 1..=3 are the 256x128 text fonts (the HUD paths own them), 0x13 is the glint whose quad size
/// (`app.rs glint_u`) assumes the original width.
fn fx_slot_key(k: usize) -> Option<u32> {
	match k {
		1..=3 | 0x13 => None,
		k if k < 0x400 => Some(FX_KEY | k as u32),
		k if (crate::particles::PART_TEX_BASE as usize..crate::particles::PART_TEX_BASE as usize + 0x100).contains(&k) => Some(PART_KEY | (k - crate::particles::PART_TEX_BASE as usize) as u32),
		_ => None,
	}
}

/// One pack image decoded to RGBA8 in the engine's alpha scale; None (and a log line) when the file cannot be used.
fn load_texture(f: &PackFile) -> Option<Texture> {
	let img = repl::load_file(f).map_err(|e| eprintln!("texpack: {e}")).ok()?;
	let m = chain::assemble(&[Some(img)], 1).and_then(|t| t.to_rgba8()).map_err(|e| eprintln!("texpack: {}: {e}", f.path.display())).ok()?.mips.into_iter().next()?;
	Some(Texture { w: m.w as usize, h: m.h as usize, rgba: m.data.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect() })
}

/// Decodes the files of `want` ((caller's id, file)) on the loader threads; the decoded ones, in the order of `want`.
pub(super) fn decode_all(want: &[(usize, PackFile)]) -> Vec<(usize, Texture)> {
	let (next, done) = (AtomicUsize::new(0), Mutex::new(Vec::new()));
	std::thread::scope(|s| {
		for _ in 0..jobs::loader_threads().min(want.len()).max(1) {
			s.spawn(|| loop {
				let i = next.fetch_add(1, Relaxed);
				let Some((id, f)) = want.get(i) else { break };
				if let Some(t) = load_texture(f) {
					done.lock().unwrap().push((i, *id, t));
				}
			});
		}
	});
	let mut v = done.into_inner().unwrap();
	v.sort_by_key(|x| x.0);
	v.into_iter().map(|(_, id, t)| (id, t)).collect()
}

/// The files the packs have for `items` ((caller's id, name)).
fn find(p: &Packs, items: impl IntoIterator<Item = (usize, TexName)>) -> Vec<(usize, PackFile)> {
	items.into_iter().filter_map(|(id, n)| p.index.get(&n).map(|f| (id, f.clone()))).collect()
}

thread_local! {
	/// The disc's original of every texture a load hook below replaced in a CPU texture table (the sky shells) by texture key, kept on
	/// the load thread for `SceneCtx::repl_begin` (the scene build shares the thread) when the split view could be used.
	static EXTRA_ORIG: std::cell::RefCell<std::collections::HashMap<u32, Texture>> = Default::default();
}

/// Split view: remembers the original `t` of the texture about to be replaced under scene key `key` (only when `splitview::keep_originals`).
fn note_orig(key: u32, t: &Texture) {
	if crate::retail::splitview::keep_originals() {
		EXTRA_ORIG.with(|m| m.borrow_mut().insert(key, Texture { w: t.w, h: t.h, rgba: t.rgba.clone() }));
	}
}

/// `SceneCtx::repl_begin`: the originals noted on this thread since the last scene build, by scene texture key.
pub fn take_extra_originals() -> std::collections::HashMap<u32, Texture> {
	EXTRA_ORIG.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

/// Split view: the disc's chrome and glass `hd_metal` replaced in the last level load (None unless kept); taken by `Gpu::load_metal`.
static METAL_ORIG: Mutex<Option<[Texture; 2]>> = Mutex::new(None);

/// The originals `hd_metal` kept (chrome, glass), once.
pub fn take_metal_originals() -> Option<[Texture; 2]> {
	METAL_ORIG.lock().unwrap().take()
}

/// The original (disc) fx / particle textures `apply_fx` replaced in the last level load, kept for the split view's Classic half (empty unless
/// `splitview::keep_originals`); taken by the level install (`take_fx_originals`).
static FX_ORIG: Mutex<Vec<(usize, Texture)>> = Mutex::new(Vec::new());

/// The originals `apply_fx` kept (key, texture), once.
pub fn take_fx_originals() -> Vec<(usize, Texture)> {
	std::mem::take(&mut *FX_ORIG.lock().unwrap())
}

/// The fx table and particle textures of a level (`viewer/app.rs`, `fx_textures`: key, texture): replaces those the packs have. `front`
/// = the title screen level (not replaced, like the world tables).
pub fn apply_fx(l: &Level, fx: &mut [(usize, Texture)], front: bool) {
	FX_ORIG.lock().unwrap().clear();
	let Some(p) = super::packs().filter(|_| !front) else { return };
	let t0 = std::time::Instant::now();
	let slot: std::collections::HashMap<u32, usize> = fx.iter().enumerate().filter_map(|(i, (k, _))| Some((fx_slot_key(*k)?, i))).collect();
	let srcs = extra::extra_sources(l);
	let want = find(&p, srcs.iter().filter(|s| matches!(s.kind, Kind::Fx | Kind::Particle)).filter_map(|s| Some((*slot.get(&s.key)?, s.name()))));
	let got = decode_all(&want);
	let (all, parts) = (got.len(), got.iter().filter(|(i, _)| fx[*i].0 >= crate::particles::PART_TEX_BASE as usize).count());
	let keep = crate::retail::splitview::keep_originals(); // [PC] split view: the Classic half shows the originals
	let mut orig = Vec::new();
	for (i, t) in got {
		if keep {
			orig.push((fx[i].0, Texture { w: fx[i].1.w, h: fx[i].1.h, rgba: fx[i].1.rgba.clone() }));
		}
		fx[i].1 = t;
	}
	*FX_ORIG.lock().unwrap() = orig;
	eprintln!("texpack: level {}: extra: {} fx + {parts} particle textures replaced in {:.0} ms", l.id, all - parts, t0.elapsed().as_secs_f64() * 1000.0);
}

/// The chrome and glass textures of the metal packets (`Level::metal_textures`).
pub fn hd_metal(l: &Level, mut t: [Texture; 2]) -> [Texture; 2] {
	let Some(p) = super::packs() else { return t };
	let mut srcs = Vec::new();
	extra::metal_sources(l, &mut srcs);
	let want = find(&p, srcs.iter().filter_map(|s| Some((if s.kind == Kind::Chrome { 0 } else { 1 }, s.name()))));
	let got = decode_all(&want);
	if !got.is_empty() && crate::retail::splitview::keep_originals() {
		*METAL_ORIG.lock().unwrap() = Some(t.each_ref().map(|o| Texture { w: o.w, h: o.h, rgba: o.rgba.clone() }));
	}
	for (i, h) in got {
		t[i] = h;
	}
	t
}

/// The sky shells' textures (not the title screen's sky).
pub fn hd_sky(l: &Level, mut sky: crate::sky::Sky) -> crate::sky::Sky {
	let Some(p) = super::packs().filter(|_| !l.title) else { return sky };
	let Ok(Some(block)) = crate::sky::block_of(l) else { return sky };
	let mut srcs: Vec<ExtraSource> = Vec::new();
	extra::sky_sources(block, &mut srcs);
	for (k, t) in decode_all(&find(&p, srcs.iter().map(|s| (s.key as usize, s.name())))) {
		if let Some(o) = sky.textures.get(&(k as u32)) {
			note_orig(k as u32, o);
		}
		sky.textures.insert(k as u32, t);
	}
	sky
}

/// The two textures of a ship bundle (`scene::ship_assets_of`).
pub fn hd_ship(bundle: &[u8], mut sa: crate::scene::ShipAssets) -> crate::scene::ShipAssets {
	let Some(p) = super::packs() else { return sa };
	for (k, t) in decode_all(&find(&p, extra::ship_sources(bundle).iter().map(|s| (s.key as usize, s.name())))) {
		sa.textures[k] = t;
	}
	sa
}

/// The warp bundle's moby textures, fx textures and star-fan sky (`scene::warp_assets`).
pub fn hd_warp(bundle: &[u8], mut a: crate::scene::WarpAssets) -> crate::scene::WarpAssets {
	let Some(p) = super::packs() else { return a };
	let srcs = extra::warp_sources(bundle);
	let got = decode_all(&find(&p, srcs.iter().enumerate().map(|(i, s)| (i, s.name()))));
	eprintln!("texpack: warp bundle: {} of {} textures replaced", got.len(), srcs.len());
	for (i, t) in got {
		let k = srcs[i].key;
		let n = (k & 0xff_ffff) as usize;
		match k & 0xff00_0000 {
			WARP_MOBY_KEY => {
				if let Some(s) = a.moby_textures.get_mut(n) {
					*s = Some(t);
				}
			}
			WARP_FX_KEY => {
				if let Some(s) = a.fx_textures.get_mut(n) {
					*s = t;
				}
			}
			_ => {
				a.sky.textures.insert(k, t);
			}
		}
	}
	a
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::particles::PART_TEX_BASE;

	#[test]
	fn fx_keys_skip_the_fonts_and_the_glint() {
		assert_eq!([1, 2, 3, 0x13, 0x400, 0x2fff, 0x3000].map(fx_slot_key), [None; 7]);
		assert_eq!(fx_slot_key(0), Some(FX_KEY));
		assert_eq!(fx_slot_key(22), Some(FX_KEY | 22));
		assert_eq!(fx_slot_key(PART_TEX_BASE as usize + 3), Some(PART_KEY | 3));
	}

	#[test]
	fn the_extra_key_spaces_do_not_meet() {
		let ranges = [(FX_KEY, 0x400u32), (PART_KEY, 0x100), (4 << 16, 0x100), (WARP_MOBY_KEY, 0x10000), (WARP_FX_KEY, 0x10000), (crate::level::CHROME_KEY, 1), (crate::level::GLASS_KEY, 1)];
		for (i, a) in ranges.iter().enumerate() {
			for b in &ranges[i + 1..] {
				assert!(a.0 + a.1 <= b.0 || b.0 + b.1 <= a.0, "{a:?} meets {b:?}");
			}
		}
	}
}
