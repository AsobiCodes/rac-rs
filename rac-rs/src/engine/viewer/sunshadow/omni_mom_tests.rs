use super::*;
use super::super::omni::SCALE;

/// The transform round-trips (the inverse matrix undoes the forward one) and its outputs stay in a half float's comfortable range.
#[test]
fn the_moment_transform_round_trips() {
	for i in 0..=100 {
		let z = i as f32 / 100.0;
		let o = optimized(z);
		let b = unoptimized(o);
		let want = [z, z * z, z * z * z, z * z * z * z];
		for c in 0..4 {
			assert!((b[c] - want[c]).abs() < 2e-4, "z {z}: moment {c} {} vs {}", b[c], want[c]);
		}
		assert!(o.iter().all(|v| (-0.01..=1.01).contains(v)), "z {z}: {o:?}");
	}
	// Linear: the mean of two stored texels is the stored texel of the mean moments (box-filtered mips stay valid).
	let (a, c) = (optimized(0.2), optimized(0.9));
	let mean = unoptimized(std::array::from_fn(|k| 0.5 * (a[k] + c[k])));
	assert!((mean[0] - 0.55).abs() < 2e-4 && (mean[1] - 0.5 * (0.04 + 0.81)).abs() < 2e-4);
}

/// Every cube texel direction belongs to its own face (the GPU's face pick and (s, t) give it back) and lies inside the drawn,
/// slightly wider face tile (so `cs_mom` reads real depth, not the clamp at the tile edge).
#[test]
fn cube_texels_map_to_their_face_and_inside_the_drawn_tile() {
	for f in 0..FACES {
		for j in 0..MOM {
			for i in 0..MOM {
				let s = (i as f32 + 0.5) * 2.0 / MOM as f32 - 1.0;
				let t = (j as f32 + 0.5) * 2.0 / MOM as f32 - 1.0;
				let d = cube_dir(f, s, t);
				let a = d.map(f32::abs);
				let ax = if a[0] >= a[1] && a[0] >= a[2] { 0 } else if a[1] >= a[2] { 1 } else { 2 };
				let pick = 2 * ax + usize::from(d[ax] < 0.0);
				assert_eq!(pick, f, "face {f} texel ({i}, {j}) -> {d:?}");
				// The GPU's (sc, tc) per face (Vulkan / D3D cube convention).
				let (sc, tc) = match f {
					0 => (-d[2], -d[1]),
					1 => (d[2], -d[1]),
					2 => (d[0], d[2]),
					3 => (d[0], -d[2]),
					4 => (d[0], -d[1]),
					_ => (-d[0], -d[1]),
				};
				assert!((sc - s).abs() < 1e-6 && (tc - t).abs() < 1e-6);
				let n = drawn_ndc(f, d);
				assert!(n[0].abs() < 1.0 && n[1].abs() < 1.0, "face {f}: ndc {n:?}");
			}
		}
	}
}

/// The array holds whole cubes, the mips end at 8 texels, the sub-directions cover the depth tile, and the dispatch covers each mip.
#[test]
fn the_moment_layout_holds() {
	assert_eq!(LAYERS % 6, 0);
	assert_eq!(LAYERS as usize, MAX_LIGHTS * FACES);
	assert_eq!((0..MIPS).map(mip_size).collect::<Vec<_>>(), [128, 64, 32, 16, 8]);
	assert_eq!((sub(256), sub(408), sub(512), sub(64)), (2, 4, 4, 2));
	for tile in [256u32, 408, 512] {
		// a moment texel spans tile / (MOM x SCALE) depth texels; the sub-directions are at least that many.
		assert!(sub(tile) as f32 >= tile as f32 / (MOM as f32 * SCALE) - 0.5, "tile {tile}");
	}
	for m in 0..MIPS {
		assert!(mip_size(m).div_ceil(WG) * WG >= mip_size(m));
	}
	assert_eq!(std::mem::size_of::<MomU>(), 16 * (1 + MAX_LIGHTS));
}

/// The shader carries the same constants (twins).
#[test]
fn the_shader_twins_match() {
	assert!(WGSL.contains(&format!("const MOM_N: u32 = {MOM}u;")));
	assert!(WGSL.contains(&format!("const MOM_SCALE: f32 = {SCALE};")));
	assert!(WGSL.contains("const MOM_OFS: f32 = 0.035955884801;") && OFS == 0.035955884801);
	assert!(WGSL.contains("vec4<f32>(-2.07224649, 13.7948857237, 0.105877704, 9.7924062118)"));
	assert!(WGSL.contains("vec4<f32>(39.3703274134, -35.364903257, -6.6543490496, -23.9728048165)"));
	assert!(WGSL.contains("texture_storage_2d_array<rgba16float, write>") && FORMAT == wgpu::TextureFormat::Rgba16Float);
	let fls = include_str!("../flitshadow.wgsl");
	assert!(fls.contains(&format!("@group(0) @binding({BINDING}) var fls_mom: texture_cube_array<f32>;")));
	assert!(fls.contains("vec4<f32>(0.2227744146, 0.1549679261, 0.1451988946, 0.163127443)"));
}

/// Naga accepts the module (both entry points).
#[test]
fn the_shader_parses() {
	let m = naga::front::wgsl::parse_str(WGSL).expect("omni_mom.wgsl parses");
	naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("omni_mom.wgsl validates");
}
