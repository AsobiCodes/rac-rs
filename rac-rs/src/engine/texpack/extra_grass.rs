//! [PC] Texture packs (lane texw): the vegetation planner (`viewer/vegfx.rs`) takes the colour of a grass blade from the ground
//! texture under it, on the CPU, from the original `Texture`. With a pack that redraws the ground, the blades should follow the
//! picture that is drawn, so for every grass-coloured world texture the pack replaces, the HD image is averaged back down over the
//! footprint of each original texel (a box average) and put into the CPU copy the planner reads (`textures` and level 0 of the
//! texture's mip chain). Same size as the original, so the planner is untouched; a pack that leaves the ground alone, or a
//! texture that is not grass-coloured, keeps the identical `Texture`. `level_loaded` records the pack hits (`remember`), the level
//! build calls `apply` right before the planner runs.

use super::index::PackFile;
use super::repl::{Fp, Job};
use crate::level::Texture;
use std::collections::HashMap;
use std::sync::Mutex;

/// Per level (the two newest): world texture key, fingerprint of the original it was named for, pack file. Paths only, no pixels.
static HITS: Mutex<Vec<(u32, Vec<(u32, Fp, PackFile)>)>> = Mutex::new(Vec::new());

/// The ground texture chains of a level: terrain, shrub, tie (the order of `terrain_chains` in `viewer/app.rs`).
pub type Chains = (Vec<Option<Vec<Texture>>>, Vec<Option<Vec<Texture>>>, Vec<Option<Vec<Texture>>>);

/// Keeps the files of the replacement jobs of `level` (called by `level_loaded`).
pub fn remember(level: u32, jobs: &[Job]) {
	let rows: Vec<_> = jobs.iter().filter(|j| matches!(j.key >> 16, 0 | 2 | 3)).filter_map(|j| Some((j.key, j.fp, j.files[0].clone()?))).collect();
	let mut h = HITS.lock().unwrap();
	h.retain(|(l, _)| *l != level);
	h.push((level, rows));
	if h.len() > 2 {
		h.remove(0);
	}
}

/// Share of the texels the planner's `green` test takes for grass (same numbers as `vegfx::green`).
fn grass_share(t: &Texture) -> f32 {
	let n = t.rgba.iter().filter(|p| {
		let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
		g >= 28.0 && g >= r * 0.92 && g >= b * 1.15 && g - b >= 12.0
	});
	n.count() as f32 / t.rgba.len().max(1) as f32
}

/// `hd` (RGBA8) averaged down to `w` x `h`: each target texel is the mean of the source texels it covers (at least one).
pub fn box_to(hd: &Texture, w: usize, h: usize) -> Texture {
	let mut rgba = Vec::with_capacity(w * h);
	for y in 0..h {
		let (y0, y1) = (y * hd.h / h, ((y + 1) * hd.h / h).max(y * hd.h / h + 1).min(hd.h));
		for x in 0..w {
			let (x0, x1) = (x * hd.w / w, ((x + 1) * hd.w / w).max(x * hd.w / w + 1).min(hd.w));
			let mut acc = [0u32; 4];
			for sy in y0..y1 {
				for p in &hd.rgba[sy * hd.w + x0..sy * hd.w + x1] {
					for c in 0..4 {
						acc[c] += p[c] as u32;
					}
				}
			}
			let n = ((y1 - y0) * (x1 - x0)) as u32;
			rgba.push(acc.map(|v| ((v + n / 2) / n) as u8));
		}
	}
	Texture { w, h, rgba }
}

fn copy_of(t: &Texture) -> Texture {
	Texture { w: t.w, h: t.h, rgba: t.rgba.clone() }
}

/// Right before `plan_veg` (`viewer/app.rs`): puts the HD-averaged colour into the planner's copies of the grass-coloured world
/// textures the packs replace. Nothing happens without a textures mod or without hits for this level.
pub fn apply(level: usize, textures: &mut HashMap<u32, Texture>, chains: &mut Chains) {
	if super::packs().is_none() {
		return;
	}
	let rows = {
		let mut h = HITS.lock().unwrap();
		h.iter().position(|(l, _)| *l as usize == level).map(|i| h.remove(i).1)
	};
	let Some(rows) = rows else { return };
	let want: Vec<(usize, PackFile)> = rows.iter().enumerate().filter(|(_, (k, fp, _))| textures.get(k).is_some_and(|t| fp.is(t) && grass_share(t) >= 0.1)).map(|(i, (_, _, f))| (i, f.clone())).collect();
	let t0 = std::time::Instant::now();
	let got = super::extra_hd::decode_all(&want);
	for (i, hd) in &got {
		let key = rows[*i].0;
		let Some(orig) = textures.get(&key) else { continue };
		let avg = box_to(hd, orig.w, orig.h);
		let list = match key >> 16 {
			0 => &mut chains.0,
			2 => &mut chains.2,
			_ => &mut chains.1,
		};
		if let Some(c0) = list.get_mut((key & 0xffff) as usize).and_then(|c| c.as_mut()).and_then(|c| c.first_mut()).filter(|c| c.w == avg.w && c.h == avg.h) {
			*c0 = copy_of(&avg);
		}
		textures.insert(key, avg);
	}
	eprintln!("texpack: level {level}: grass colours from {} HD ground textures ({} ms)", got.len(), t0.elapsed().as_millis());
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tex(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Texture {
		Texture { w, h, rgba: (0..w * h).map(|i| f(i % w, i / w)).collect() }
	}

	#[test]
	fn box_average_over_the_original_footprint() {
		// 4x4 HD of an exactly doubled 2x2: the average is the 2x2 itself.
		let small = tex(2, 2, |x, y| [(x * 100) as u8, (y * 100) as u8, 7, 255]);
		let hd = tex(4, 4, |x, y| [((x / 2) * 100) as u8, ((y / 2) * 100) as u8, 7, 255]);
		assert_eq!(box_to(&hd, 2, 2).rgba, small.rgba);
		// A 2x2 block of 0 / 100 / 100 / 200 averages to 100.
		let mix = Texture { w: 2, h: 2, rgba: vec![[0, 0, 0, 0], [100, 0, 0, 0], [100, 0, 0, 0], [200, 0, 0, 0]] };
		assert_eq!(box_to(&mix, 1, 1).rgba, vec![[100, 0, 0, 0]]);
		// Same size: unchanged; an HD smaller than the original still fills every texel.
		assert_eq!(box_to(&small, 2, 2).rgba, small.rgba);
		assert_eq!(box_to(&small, 4, 4).rgba.len(), 16);
	}

	#[test]
	fn only_grass_coloured_textures_count() {
		assert!(grass_share(&tex(2, 2, |_, _| [40, 120, 40, 255])) > 0.9);
		assert!(grass_share(&tex(2, 2, |_, _| [120, 100, 90, 255])) < 0.01);
	}

	#[test]
	fn remember_keeps_the_two_newest_levels_and_only_world_groups() {
		let fp = Fp { w: 1, h: 1, hash: 0 };
		let file = || Some(PackFile { path: "a.dds".into(), ext: super::super::name::Ext::Dds, mod_idx: 0 });
		let job = |key| Job { key, fp, n: 1, files: [file(), None, None, None] };
		for lvl in [91, 92, 93] {
			remember(lvl, &[job(5), job(1 << 16 | 5), job(2 << 16 | 5)]);
		}
		let h = HITS.lock().unwrap();
		let levels: Vec<u32> = h.iter().map(|x| x.0).filter(|l| (91..=93).contains(l)).collect();
		assert!(!levels.contains(&91) && levels.contains(&93));
		assert_eq!(h.iter().find(|x| x.0 == 93).unwrap().1.len(), 2, "the moby key is not a ground texture");
	}
}
