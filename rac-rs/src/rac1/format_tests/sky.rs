//! Disc tests of formats/sky.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::sky::*;
	use crate::{iso, toc};
	use crate::bytes::i16_at;
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::level::Level;
	use crate::level::Texture;
	use crate::level::SKY_OFFSET;
	use crate::mesh::Mesh;
	use std::collections::HashMap;

	/// KER-72a: the Kerwan sky has three shells in the retail shell-table order (header +0x20, the table the retail
	/// sky_shell_draw indexes): shell 0 the flat gouraud dome, shells 1 and 2 the textured rings that turn.
	#[test]
	fn kerwan_sky_shell_order() {
		let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = iso::Disc::open(path).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let kerwan = sky(&Level::load(&mut disc, levels.iter().find(|l| l.id == 3).unwrap()).unwrap()).unwrap().unwrap();
		assert_eq!(kerwan.untextured, vec![true, false, false]);
		assert_eq!(kerwan.meshes.iter().map(|m| m.faces.len()).collect::<Vec<_>>(), vec![768, 64, 64]);
		assert!(kerwan.meshes[1].faces.iter().all(|f| f.0 == 4 << 16));
		assert!(kerwan.meshes[2].faces.iter().all(|f| f.0 == 4 << 16 | 1));
	}

	/// ARI-4 spec test 8: the disc headers.
	#[test]
	fn aridia_sky_sprite_header() {
		let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = iso::Disc::open(path).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let get = |disc: &mut iso::Disc, id| sky(&Level::load(disc, levels.iter().find(|l| l.id == id).unwrap()).unwrap()).unwrap().unwrap();
		let aridia = get(&mut disc, 2);
		assert_eq!((aridia.sprite_count, aridia.sprite_max), (0, 256));
		assert!(aridia.textures.contains_key(&(4 << 16 | 2)) && aridia.textures.contains_key(&(4 << 16 | 3)));
		assert_eq!(get(&mut disc, 1).sprite_max, 0);
	}

	#[test]
	fn all_levels() {
		let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = iso::Disc::open(path).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let mut failed = 0;
		for info in &levels {
			let result = Level::load(&mut disc, info).and_then(|l| sky(&l));
			match result {
				Ok(Some(s)) => {
					let tris: usize = s.meshes.iter().map(|m| m.faces.len()).sum();
					println!(
						"level {}: shells {} (untextured {:?}), triangles {}, textures {}, colour {:?}",
						info.id, s.meshes.len(), s.untextured, tris, s.textures.len(), s.colour
					);
				}
				Ok(None) => println!("level {}: no sky", info.id),
				Err(e) => {
					failed += 1;
					println!("level {}: error {e}", info.id);
				}
			}
		}
		assert_eq!(failed, 0);
	}
}
