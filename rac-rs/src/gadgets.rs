//! Gadget (hero weapon / item) textures.
//!
//! The gadget table (core index 0x80) has no texture data. Like the level loader, `mobies::gadget_classes`
//! takes each gadget's 16-byte texture remap from the moby class table entry with the same class ID, so a
//! gadget face carries `texture_key(Moby, remap[slot])`: an ordinary texture of the level's moby group.

use crate::bytes::R;
use crate::level::{texture_key, Level, Texture, TextureGroup};
use crate::mobies;
use std::collections::{HashMap, HashSet};

/// Every texture used by any gadget class of the level, by the same keys the gadget mesh faces carry
/// (`texture_key(Moby, global texture index)`), ready to merge into the level's texture map.
pub fn gadget_textures(level: &Level) -> R<HashMap<u32, Texture>> {
	let classes = mobies::gadget_classes(level)?;
	let used: HashSet<u32> = classes.values().flat_map(|c| c.mesh.faces.iter().map(|f| f.0 & mobies::KEY_MASK)).collect();
	let mut out = HashMap::new();
	for (i, t) in level.textures(TextureGroup::Moby)?.into_iter().enumerate() {
		let key = texture_key(TextureGroup::Moby, i as u32);
		if let (true, Some(t)) = (used.contains(&key), t) {
			out.insert(key, t);
		}
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{iso, toc};

	#[test]
	fn wrench_textures() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let wrench = &mobies::gadget_classes(&level).unwrap()[&mobies::WRENCH_CLASS];
		let tex = gadget_textures(&level).unwrap();
		let keys: std::collections::BTreeSet<u32> = wrench.mesh.faces.iter().map(|f| f.0 & mobies::KEY_MASK).collect();
		println!("wrench keys {keys:x?}");
		assert!(!keys.is_empty());
		for k in &keys {
			let t = tex.get(k).unwrap_or_else(|| panic!("no texture for key {k:#x}"));
			assert!(t.w >= 8 && t.h >= 8 && t.w <= 512 && t.h <= 512, "size {}x{}", t.w, t.h);
			assert_eq!(t.rgba.len(), t.w * t.h);
			assert!(t.rgba.iter().any(|p| p[3] > 0));
			println!("key {k:#x}: {}x{}", t.w, t.h);
		}
		// PNG for a look (scratchpad only).
		let t = &tex[keys.iter().next().unwrap()];
		let rgb: Vec<u8> = t.rgba.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
		let png = crate::render::png(&crate::render::Image { w: t.w, h: t.h, rgb });
		let out = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test_out");
		std::fs::create_dir_all(out).unwrap();
		std::fs::write(format!("{out}/wrench_tex.png"), png).unwrap();
	}
}
