//! Fur: the shader validates, the welded normals, the per-part uniform block, the level switches, the texture key.

use super::*;
use crate::furspec::{shells, texhash, FurMask, FurPart, FurRow, FurSpec, SHELLS};

fn vert(x: f32, y: f32, z: f32) -> Vertex {
	Vertex { pos: [x, y, z], uv: [0.0; 2], normal: [0.0, 0.0, 1.0], colour: [1.0; 4], light: [0.0; 4] }
}

const MASK: FurMask = FurMask { hue: [18.0, 75.0], sat: 0.4, val: 0.45, pale: [45.0, 80.0, 0.2, 0.6], erode: 0, add: &[], cut: &[], zones: &[] };

fn spec() -> FurSpec {
	const PART: FurPart = FurPart { name: "t", len: [0.01, 0.04], comb: [0.0, 0.01], density: 50.0, thick: [0.8, 1.0], halo: [1.5, 2.5], tip: [1.1, 1.0, 0.9], root: 0.4, sheen: [0.5, 0.7] };
	static PARTS: [FurPart; 2] = [PART, PART];
	static ROWS: [FurRow; 2] = [FurRow { hash: 7, part: 1, mask: MASK }, FurRow { hash: 9, part: 0, mask: MASK }];
	FurSpec { rows: &ROWS, parts: &PARTS }
}

/// The level shader with the fur entry points parses, validates with every capability, and has both entry points.
#[test]
fn the_fur_shader_validates() {
	for early in [false, true] {
		let src = pipes::source_with(early);
		let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("fur: {}", e.emit_to_string(&src)));
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("the fur shader validates");
		for name in ["vs_fur", "fs_fur"] {
			assert!(m.entry_points.iter().any(|e| e.name == name), "entry point {name}");
		}
	}
}

#[test]
fn equal_positions_share_the_first_vertex_of_a_seam() {
	let model = [vert(0.0, 0.0, 0.0), vert(1.0, 0.0, 0.0), vert(0.0, 0.0, 0.0), vert(1.0, 0.0, 0.00001), vert(0.0, 1.0, 0.0)];
	assert_eq!(pick::weld_table(&model), vec![0, 1, 0, 1, 4]);
}

#[test]
fn the_part_block_has_one_slot_per_part_and_the_style_picks_the_values() {
	let s = spec();
	let soft = params::block(&s, 1, false, true);
	let fluffy = params::block(&s, 3, true, false);
	assert_eq!(soft.len(), 2 * params::SLOT as usize);
	let f = |b: &[u8], slot: usize, i: usize| -> f32 { bytemuck::pod_read_unaligned(&b[slot * params::SLOT as usize + i * 4..][..4]) };
	assert_eq!((f(&soft, 1, 0), f(&soft, 1, 1), f(&soft, 1, 3)), (0.01, 50.0, SHELLS[0][0] as f32));
	assert_eq!((f(&fluffy, 0, 0), f(&fluffy, 0, 2), f(&fluffy, 0, 3)), (0.04, 0.01, SHELLS[2][1] as f32));
	assert_eq!((f(&soft, 0, 8), f(&fluffy, 0, 8), f(&soft, 0, 12), f(&fluffy, 0, 12)), (0.8, 1.0, 1.5, 2.5), "opacity, halo");
	assert_eq!((f(&soft, 0, 11), f(&fluffy, 0, 11)), (1.0, 0.0), "weld flag");
	assert_eq!((f(&soft, 0, 10), f(&fluffy, 0, 10)), (0.0, 1.0), "style flag");
}

/// The afterimage copies are the last faces of every texture group: each draw stops before them (hidden or not, whatever their colour
/// alpha), a draw made only of copies gets none, and the class tail's own faces (below `ghost_from`) keep their shells.
#[test]
fn the_fur_draw_stops_before_the_afterimage_tail() {
	// Hero vertices 0..10 (tail tinted 6..10), copies 10..20 and 20..30; groups in key order 3, 5, 9 like the batch's BTreeMap.
	let faces = [(5, [0, 1, 2]), (3, [6, 7, 8]), (5, [7, 8, 9]), (5, [10, 11, 12]), (3, [13, 14, 15]), (5, [20, 21, 22]), (9, [23, 24, 25]), (3, [26, 27, 28])];
	assert_eq!(pick::ghost_cut(&faces, 10), vec![3, 6, 0], "key 3: 1 hero face; key 5: 2 hero faces then 2 copies; key 9: copies only");
	assert_eq!(pick::ghost_cut(&faces, 30), vec![9, 12, 3], "no tail: every face is drawn");
	assert_eq!(pick::ghost_cut(&[], 10), Vec::<u32>::new());
	assert_eq!([pick::fur_count(&[3, 6, 0], 1, 9), pick::fur_count(&[3, 6, 0], 2, 3), pick::fur_count(&[3, 6, 0], 0, 2)], [6, 0, 2], "never past the draw");
	assert_eq!(pick::fur_count(&[], 4, 21), 21, "no table: all of it");
}

/// The shader grows fur on every vertex it is given (no colour-alpha collapse that also took the tinted tail's fur) and never
/// normalises a zero normal.
#[test]
fn the_fur_shader_has_no_alpha_collapse_and_guards_the_normal() {
	let src = include_str!("fur.wgsl");
	assert!(!src.contains("o.pos = vec4"), "no vertex is thrown off the screen by its colour alpha");
	assert!(!src.contains("normalize(i.n)"), "the fragment normal is guarded");
	assert!(src.contains("nlen > 1.0e-4"));
}

/// R5-1: a texel the mask, the skin's cut-out, the strand length or the tip zeroes is dropped before the two noise samples, and the
/// noise samples after that discard take explicit gradients (the validation test checks the uniformity rule).
#[test]
fn the_fur_shader_drops_bare_texels_before_the_noise() {
	let src = include_str!("fur.wgsl");
	let fs = &src[src.find("@fragment fn fs_fur").expect("fs_fur")..];
	let early = fs.find("if (fm.r <= 0.2 || base.a < 0.25 || sf <= 0.03 || hr >= 1.15) { discard; }").expect("the early discard");
	assert!(early < fs.find("fur_cov(").expect("the noise"), "the early discard comes first");
	assert!(fs.find("dpdx(g2)").expect("the gradients") < early, "the gradients are taken before it");
	assert!(fs.contains("smoothstep(0.2, 0.8, fm.r) * step(0.25, base.a) * smoothstep(0.03, 0.12, sf) * (1.0 - smoothstep(1.02, 1.15, hr))"), "the factors the early test mirrors");
	assert!(!src.contains("textureSample(fur_noise"), "no implicit-derivative noise sample");
}

#[test]
fn shells_grow_with_the_level_and_off_has_none() {
	assert_eq!((shells(0, false), shells(0, true), shells(9, true)), (0, 0, 0));
	for fluffy in [false, true] {
		assert!(shells(1, fluffy) < shells(2, fluffy) && shells(2, fluffy) < shells(3, fluffy));
	}
	assert!(shells(3, true) > shells(3, false));
}

/// The noise has a full mip chain down to 1x1, strands thin out towards the tip, and it is neither empty nor solid.
#[test]
fn the_noise_tapers_and_has_every_mip() {
	let lv = noise::levels();
	assert_eq!(lv.len(), noise::SIZE.trailing_zeros() as usize + 1);
	for (m, l) in lv.iter().enumerate() {
		assert_eq!(l.len(), (noise::SIZE >> m) * (noise::SIZE >> m) * 4);
	}
	let mean = |c: usize| lv[0].iter().skip(c).step_by(4).map(|&v| v as f32 / 255.0).sum::<f32>() / (noise::SIZE * noise::SIZE) as f32;
	let m = [mean(0), mean(1), mean(2), mean(3)];
	assert!(m[0] > 0.2 && m[0] < 0.8, "root coverage {m:?}");
	assert!(m[0] > m[1] && m[1] > m[2] && m[2] > m[3] && m[3] > 0.0, "taper {m:?}");
	assert!((lv.last().unwrap()[0] as f32 / 255.0 - m[0]).abs() < 0.02, "1x1 mip = the mean");
}

#[test]
fn the_texture_key_depends_on_size_and_texels() {
	let a = [[1u8, 2, 3, 4]; 4];
	let mut b = a;
	b[3][0] = 9;
	assert_ne!(texhash(2, 2, &a), texhash(2, 2, &b));
	assert_ne!(texhash(2, 2, &a), texhash(4, 1, &a));
	assert_eq!(texhash(2, 2, &a), texhash(2, 2, &a));
}

#[test]
fn the_level_follows_graphics_and_a_pack_without_fur_has_none() {
	let g = crate::gfx::Graphics { fur_q: 2, fur_long: true, ..crate::gfx::Graphics::CLASSIC };
	if std::env::var("RAC_FUR").is_err() && std::env::var("RAC_FUR_STYLE").is_err() {
		assert_eq!(level(g), (2, true));
		assert_eq!(level(crate::gfx::Graphics { fur_q: 9, ..g }).0, 3);
	}
	assert!((crate::pack::NONE.fur)().is_none());
}
