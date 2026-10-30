//! Unit tests of the renderer helpers (kept together; they reach every sibling through `use super::*`).
#![allow(unused_imports)]

use super::*;

#[cfg(test)]
mod line_tests {
	use super::*;

	/// KER-92 test (5): near 0.5 / far 224 strict on the midpoint depth key; the pool clamp lowers far; both ends clip.
	#[test]
	fn tracer_line_depth_key_and_clip() {
		let cam = Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: 0.5, up: [0.0, 0.0, 1.0] };
		let f = [1.0, 0.0, 0.0];
		let m = view_proj(&cam, 16.0 / 9.0, RETAIL_NEAR, RETAIL_FAR);
		let line = |x: f32| particles::Line { a: [x + 1.0, 0.0, 0.0], b: [x - 1.0, 0.0, 0.0], col_a: [1.0; 4], col_b: [1.0; 4], near: 0.5, far: 224.0 };
		let shown = |l: &particles::Line, clamp: f32| line_shown(&m, cam.pos, f, l, clamp);
		assert!(shown(&line(10.0), 500.0));
		assert!(!shown(&line(0.4), 500.0));
		assert!(!shown(&line(230.0), 500.0));
		assert!(!shown(&line(224.0), 500.0));
		assert!(shown(&line(200.0), 500.0));
		assert!(!shown(&line(70.0), 64.0));
		// One end outside the frustum (far to the side): skipped.
		let side = particles::Line { a: [10.0, 0.0, 0.0], b: [10.0, 50.0, 0.0], ..line(10.0) };
		assert!(!shown(&side, 500.0));
	}
}

#[cfg(test)]
mod instance_tests {
	use super::*;

	/// NOV-22.12 tests 1-2: flat corners for raw sizes 420000 and 210000.
	#[test]
	fn flat_pool_sprite_corners() {
		let scale = std::f32::consts::SQRT_2 / 210_000.0;
		let near = |a: [[f32; 3]; 4], b: [[f32; 3]; 4], e: f32| a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| (x - y).abs() <= e);
		let c = flat_corners([0.0; 3], 420_000.0 * scale, 0.0);
		let o = 0.99999994;
		assert!(near(c, [[o, 0.0, 0.0], [0.0, o, 0.0], [-o, 0.0, 0.0], [0.0, -o, 0.0]], 1e-6), "{c:?}");
		let c = flat_corners([10.0, 20.0, 5.0], 210_000.0 * scale, 32.0 * std::f32::consts::TAU / 256.0);
		let want = [[10.353554, 20.353554, 5.0], [9.646447, 20.353554, 5.0], [9.646446, 19.646446, 5.0], [10.353554, 19.646446, 5.0]];
		assert!(near(c, want, 2e-6), "{c:?}");
	}

	fn mesh(n: usize, faces: &[(u32, [u32; 3])]) -> Mesh {
		let mut m = Mesh::new(String::new());
		m.positions = vec![[0.0; 3]; n];
		m.uvs = vec![[0.0; 2]; n];
		m.faces = faces.to_vec();
		m
	}

	/// BRIDGE-27 test 3 / BRIDGE-29: one draw per (mesh, texture), ascending key within a mesh, per-mesh ranges.
	#[test]
	fn instance_layout_keeps_per_mesh_ranges() {
		let a = mesh(4, &[(5, [0, 1, 2]), (3, [1, 2, 3]), (5, [2, 3, 0])]);
		let b = mesh(3, &[(5, [0, 1, 2])]);
		let (idx, draws, ranges) = instance_layout(&[&a, &b]);
		assert_eq!(draws, vec![(3, 0, 3), (5, 3, 6), (5, 9, 3)]);
		assert_eq!(ranges, vec![(0..2, 0..4), (2..3, 4..7)]);
		assert_eq!(&idx[9..], &[4, 5, 6], "the second mesh's indices are offset by the first's vertex count");
	}

	/// An empty mesh (a LOD with no packets) has an empty draw range and advances nothing.
	#[test]
	fn instance_layout_handles_empty_meshes() {
		let a = mesh(0, &[]);
		let b = mesh(3, &[(1, [0, 1, 2])]);
		let (_, draws, ranges) = instance_layout(&[&a, &b]);
		assert_eq!(draws.len(), 1);
		assert_eq!(ranges, vec![(0..0, 0..0), (0..1, 0..3)]);
	}
	/// The shrub fragment alpha rule as the shader computes it: As = (texel alpha x A) >> 7 with the texel in PS2 units
	/// (`floor(c.a * 128 + 0.5)`), A decoded from the carrier. Returns As and which Z split of `lim` it falls on.
	fn shrub_alpha(texel_ps2: u32, carrier: f32) -> (u32, bool) {
		let a = ((-2.0 - carrier) * 256.0).round() as u32;
		(texel_ps2 * a / 128, true)
	}

	/// BRIDGE-27 test (1): the alpha carrier round-trips.
	#[test]
	fn shrub_carrier_values() {
		for (a, w) in [(74u8, -2.2890625f32), (127, -2.4960938), (0x80, -2.5), (0, -2.0)] {
			assert_eq!(shrub_carrier(a), w);
			assert_eq!(((-2.0 - w) * 256.0).round() as u8, a);
		}
	}

	/// Test (2): fragment alpha and Z split (the shader's As rule, entries ge12 / lt12 and ge96 / lt96).
	#[test]
	fn shrub_fragment_alpha_split() {
		let (as_, _) = shrub_alpha(0x80, shrub_carrier(74));
		assert_eq!(as_, 74);
		assert!(as_ >= 12);
		assert_eq!(shrub_alpha(0x80, shrub_carrier(5)).0, 5);
		assert!(shrub_alpha(0x80, shrub_carrier(5)).0 < 12);
		assert_eq!(shrub_alpha(0x14, shrub_carrier(74)).0, 11);
		assert_eq!(shrub_alpha(96, shrub_carrier(0x80)).0, 96);
		assert!(shrub_alpha(95, shrub_carrier(0x80)).0 < 96);
		// The shader text carries the same constants.
		assert!(super::super::SHADER.contains("floor(ta * a / 128.0)"));
		// (the entries test `ra`, the alpha of `shrub_alpha`, before the colour is computed: lane scene speed cut)
		assert!(super::super::SHADER.contains("ra * 128.0 < 12.0") && super::super::SHADER.contains("ra * 128.0 < 96.0"));
	}

	#[test]
	fn present_mode_prefers_uncapped() {
		use wgpu::PresentMode::{Fifo, FifoRelaxed, Immediate, Mailbox};
		assert_eq!(present_mode(&[Fifo, Immediate, Mailbox], false, None), Mailbox);
		assert_eq!(present_mode(&[Fifo, FifoRelaxed, Immediate], false, None), Immediate);
		assert_eq!(present_mode(&[Fifo], false, None), Fifo);
		assert_eq!(present_mode(&[Fifo, Immediate, Mailbox], true, None), Fifo);
		assert_eq!(present_mode(&[Fifo, Immediate, Mailbox], false, Some("immediate")), Immediate);
		assert_eq!(present_mode(&[Fifo, Immediate], false, Some("mailbox")), Immediate);
		assert_eq!(present_mode(&[Fifo, Immediate, Mailbox], true, Some("immediate")), Fifo);
	}

	/// Test (4): which buffer a relit vertex address lands in.
	#[test]
	fn relight_addresses_split_at_the_shrub_boundary() {
		assert_eq!(relight_space(1012, 800, 1000), (VertexSpace::Shrub, 12));
		assert_eq!(12 * std::mem::size_of::<Vertex>(), 768);
		assert_eq!(relight_space(1000, 800, 1000), (VertexSpace::Shrub, 0));
		assert_eq!(relight_space(40, 800, 1000), (VertexSpace::World, 40));
		assert_eq!(relight_space(900, 800, 1000), (VertexSpace::Tie, 100));
	}

	/// Test (6): a shrub rewrites its vertex colours only when its fade state changes.
	#[test]
	fn shrub_fade_cache_rewrites_on_change_only() {
		assert_eq!(shrub_rewrite(0x80, 74), Some(-2.2890625));
		assert_eq!(shrub_rewrite(74, 74), None);
		assert_eq!(shrub_rewrite(74, 0x80), Some(-2.5));
	}

	/// Test (7): the draw order follows the class load slot, stable within a slot.
	#[test]
	fn shrub_order_sorts_by_class_slot() {
		let inst = |slot| shrubs::ShrubInstance { mesh: Mesh::new(String::new()), cull: shrubs::ShrubCull { centre: [0.0; 3], radius: 0.0, dist: 0.0, bb: None }, bb: None, class_slot: slot, class_id: 0 };
		assert_eq!(shrub_order(&[inst(2), inst(0), inst(1)]), vec![1, 2, 0]);
		assert_eq!(shrub_order(&[inst(1), inst(1), inst(0), inst(1)]), vec![2, 0, 1, 3]);
	}
	/// The shader's level choice (`shrub_bb_texel`): lod = log2(32 vz) + K, level 0 when lod <= 0, else the nearest level capped at `mxl`.
	fn mip_level(vz: f32, k: f32, mxl: u32) -> u32 {
		let lod = (32.0 * vz).log2() + k;
		if lod > 0.0 {
			((lod + 0.5).floor() as u32).min(mxl)
		} else {
			0
		}
	}

	/// REN-CULL-SHRUB-BB-MIP tests (1)-(5) (also the tfrag pass of REN-TEX-RESIDENCY: same Q = 1 / (32 vz)).
	#[test]
	fn mip_level_switches_at_fixed_depths() {
		assert!(super::super::SHADER.contains("log2(32.0 / i.pos.w) + i.kmx.x") && super::super::SHADER.contains("floor(lod + 0.5), i.kmx.y"));
		let k = -10.625;
		assert_eq!(mip_level(69.7, k, 3), 0);
		assert_eq!(mip_level(69.8, k, 3), 1);
		assert_eq!(mip_level(139.5, k, 3), 1);
		assert_eq!(mip_level(139.7, k, 3), 2);
		assert_eq!(mip_level(279.3, k, 3), 3);
		assert_eq!(mip_level(279.3, k, 2), 2);
		assert_eq!(mip_level(100.0, k, 3), 1);
		assert_eq!(mip_level(100.0, -11.625, 3), 0);
		assert_eq!(mip_level(200.0, -11.625, 3), 1);
		assert_eq!(mip_level(400.0, -12.25, 3), 1);
		assert_eq!(mip_level(450.0, -12.25, 3), 2);
		assert_eq!(mip_level(50.0, -10.625, 3), 0);
		assert_eq!(mip_level(50.0, -12.25, 3), 0);
		// Within the 500 m cap K = -11.625 never reaches level 3 (558.34 m).
		assert_eq!(mip_level(500.0, -11.625, 3), 2);
	}

	/// REN-TEX-RESIDENCY: the tfrag draw's first-instance word round-trips K and the top level through the vertex shader decode.
	#[test]
	fn tfrag_instance_word() {
		let w = terrain_inst(-7.8125, 3);
		assert_eq!(w, 0x3f83);
		let mut k = (w & 0xfff) as i32;
		if k >= 2048 {
			k -= 4096;
		}
		assert_eq!((k as f32 / 16.0, (w >> 12) & 3), (-7.8125, 3));
		assert_eq!(terrain_inst(-8.5625, 2), 0x2f77);
		assert_eq!(terrain_inst(3.0, 0), 48);
		assert!(super::super::SHADER.contains("f32((inst >> 12u) & 3u)"));
	}

	/// A terrain mesh copy keys each face with its material index, leaving the texture index in the low byte.
	#[test]
	fn terrain_face_keys_carry_the_material() {
		let mut m = Mesh::new(String::new());
		m.faces = vec![(13, [0, 1, 2]), (13, [1, 2, 3]), (7, [2, 3, 4])];
		let k = terrain_face_keys(&m, &[0, 5, 2]);
		assert_eq!(k.faces.iter().map(|f| f.0).collect::<Vec<_>>(), vec![13, 13 | 5 << 8, 7 | 2 << 8]);
	}
}

#[cfg(test)]
mod verdict_tests {
	use super::*;

}

#[cfg(test)]
mod grow_tests {
	use super::*;

	fn rnd(s: &mut u64) -> u64 {
		*s ^= *s << 13;
		*s ^= *s >> 7;
		*s ^= *s << 17;
		*s
	}

	/// Lane A7: a pool growth as `grow_plan`'s insert (old indices copied around the new faces, runs moved) gives exactly the
	/// layout a whole rebuild (`prop_layout`) gives for the new order, over random classes (1-4 texture keys each, cull bits,
	/// keys new to the scene), pools, other props, actors and several growths in a row, as `App::append_pools` orders them.
	#[test]
	fn grow_plan_is_a_rebuild() {
		let mut seed = 0x9e37_79b9_7f4a_7c15u64;
		let mut tex_slots: HashMap<u32, usize> = HashMap::new();
		for k in 0..12u32 {
			for cull in [0, 1 << 24, 2 << 24] {
				tex_slots.insert((k | cull) & crate::mobies::SLOT_MASK, k as usize);
			}
		}
		let (mut grown, mut new_keys) = (0, 0);
		for _ in 0..400 {
			let mut cf: HashMap<u32, ClassFaces> = HashMap::new();
			for c in 1..=10u32 {
				let ng = 1 + rnd(&mut seed) % 4;
				let mut keys: Vec<u32> = (0..ng).map(|_| (rnd(&mut seed) % 12) as u32 | [0, 1 << 24, 2 << 24][(rnd(&mut seed) % 3) as usize]).collect();
				keys.sort_unstable();
				keys.dedup();
				let groups = keys.iter().map(|&k| (k, (0..3 * (1 + rnd(&mut seed) % 3) as u32).map(|i| (i * 7 + k) % 5).collect())).collect();
				cf.insert(c, ClassFaces { groups, clipped: Vec::new() });
			}
			// Classes 9 and 10 only arrive by growth (keys the scene may not have yet).
			let pick = |s: &mut u64, hi: u64| 1 + (rnd(s) % hi) as u32;
			let mut pools: Vec<u32> = (0..rnd(&mut seed) % 12).map(|_| pick(&mut seed, 8)).collect();
			pools.sort_unstable();
			let rest: Vec<u32> = (0..rnd(&mut seed) % 4).map(|_| pick(&mut seed, 8)).collect();
			let actors: Vec<u32> = (0..rnd(&mut seed) % 3).map(|_| pick(&mut seed, 8)).collect();
			let mut is_pool: Vec<bool> = pools.iter().map(|_| true).chain(rest.iter().map(|_| false)).collect();
			let mut classes: Vec<u32> = pools.iter().chain(&rest).chain(&actors).copied().collect();
			let mut ranges: Vec<std::ops::Range<usize>> = (0..classes.len()).map(|i| i * 5..i * 5 + 5).collect();
			let mut verts = classes.len() * 5;
			let mut game_order: Vec<usize> = (0..pools.len() + rest.len()).collect();
			let mut order: Vec<usize> = (0..classes.len()).collect();
			let (mut draws, mut runs, mut indices) = prop_layout(&cf, &tex_slots, &classes, &ranges, &order);
			for _ in 0..3 {
				let at = is_pool.len();
				let mut add: Vec<u32> = (0..1 + rnd(&mut seed) % 4).map(|_| pick(&mut seed, 10)).collect();
				add.sort_unstable();
				let m = add.len();
				// App::append_pools: the pools by class (each class's new props after its old ones), then the other props, then the actors.
				let mut pool: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
				let mut other = Vec::new();
				for &i in &game_order {
					if is_pool[i] { pool.entry(classes[i]).or_default().push(i) } else { other.push(i) }
				}
				for (k, &c) in add.iter().enumerate() {
					pool.entry(c).or_default().push(at + k);
				}
				let mut new_game: Vec<usize> = pool.into_values().flatten().collect();
				new_game.extend(other);
				let n_actors = classes.len() - at;
				let new_order: Vec<usize> = new_game.iter().copied().chain(at + m..at + m + n_actors).collect();
				let new_ranges: Vec<std::ops::Range<usize>> = (0..m).map(|k| verts + k * 5..verts + k * 5 + 5).collect();
				let mut all_classes = classes.clone();
				all_classes.splice(at..at, add.iter().copied());
				let mut all_ranges = ranges.clone();
				all_ranges.splice(at..at, new_ranges.iter().cloned());
				let (nd, nr, ni) = prop_layout(&cf, &tex_slots, &all_classes, &all_ranges, &new_order);
				let old = GrowOld { cf: &cf, tex_slots: &tex_slots, classes: &classes, order: &order, draws: &draws, runs: &runs };
				let mut plan = grow_plan(&old, at, &add, &new_ranges, &new_order).expect("an insert");
				new_keys += (plan.draws.len() > draws.len()) as u32;
				assert_eq!(plan.apply(&indices), ni);
				assert!(same_draws(&plan.draws, &nd));
				assert_eq!(plan.moved_runs(runs.clone(), at), nr);
				grown += 1;
				(draws, runs, indices) = (nd, nr, ni);
				is_pool.splice(at..at, vec![true; m]);
				(classes, ranges, order, game_order) = (all_classes, all_ranges, new_order, new_game);
				verts += m * 5;
			}
		}
		assert!(grown == 1200 && new_keys > 50, "{grown} growths, {new_keys} with a new texture key");
	}

	/// An order that is not the old one with the new slots put in gives no plan (the caller rebuilds).
	#[test]
	fn grow_plan_refuses_a_reorder() {
		let mut cf: HashMap<u32, ClassFaces> = HashMap::new();
		cf.insert(1, ClassFaces { groups: vec![(0, vec![0, 1, 2])], clipped: Vec::new() });
		let tex_slots: HashMap<u32, usize> = [(0, 0)].into_iter().collect();
		let classes = [1, 1];
		let ranges = [0..3, 3..6];
		let (draws, runs, _) = prop_layout(&cf, &tex_slots, &classes, &ranges, &[0, 1]);
		let old = GrowOld { cf: &cf, tex_slots: &tex_slots, classes: &classes, order: &[0, 1], draws: &draws, runs: &runs };
		assert!(grow_plan(&old, 2, &[1], &[6..9], &[1, 0, 2]).is_none());
		assert!(grow_plan(&old, 2, &[1], &[6..9], &[0, 2, 1]).is_some());
	}
}
