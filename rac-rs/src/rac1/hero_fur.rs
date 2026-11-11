//! [PC] Ratchet's fur table (the game's half of the fur effect: engine/viewer/furfx.rs, the types in engine/furspec.rs, the pack hook
//! `Pack::fur`). USA hash verified identical to PAL (test `hero_fur_discs`).
//! Facts, found with RAC_FUR_DUMP on Kerwan, Veldin and Umbris (the same hash on all three, so the key is stable):
//! the hero marker draws his body with ONE 256x256 texture (hash below) that also holds the gloves, boots, harness, eye and the green
//! cloth; every other marker texture is Clank, a weapon or the box texture. So the texture is the key and a colour mask picks the
//! fur texels in it (user 2026-10-31: "make every visible part of ratchet's body fluffy"): every saturated orange / yellow / brown
//! (body, stripes and markings, the dark brows: saturation >= 0.5, where the leather of the gloves, boots and belt stays below
//! 0.42) plus the pale yellow tufts; no green cloth, no black or grey hardware, no white, no pink pads or nose. The brows' darkest
//! texels look like leather, so a rectangle adds them; rectangles take out the eye, two fur-coloured plates (the chest emblem
//! and the back harness's hexagon), the lips and the nose. Mask over the texture: scratch_perf_fur/runs/pics2/mask_final.png. Texel rectangles are [x0, y0, x1, y1).

use crate::furspec::{FurMask, FurPart, FurRow, FurSpec, FurZone};

/// Content hash (`furspec::texhash`) of Ratchet's body texture.
pub const BODY_TEXTURE: u64 = 0x71a0_47c5_3db9_925f;

/// The brows (top centre of the texture).
static ADD: [[u16; 4]; 1] = [[80, 4, 114, 24]];
/// The eye, the chest emblem plate (with the green triangle), a band along the mouth line (the lips' shells would cover the teeth), a
/// margin around the nose and the khaki hexagon plate of the back harness (texels x 218..242, y 146..166: it has a fur colour, so
/// without the rectangle it grew fur, user 2026-11-06). NOT a cut any more: x 28..50 y 109..129, once called the buckle: it is the
/// skin of the upper chest above the strap (UV map in a Kerwan front shot, scratch_perf_fur3/pp/cv_front.png), cutting it left the
/// chest bare. The real buckle above the trousers is grey hardware and the colour rule drops it.
static CUT: [[u16; 4]; 5] = [[156, 107, 186, 138], [0, 9, 15, 43], [76, 81, 116, 90], [101, 67, 116, 79], [216, 144, 244, 168]];

/// The face (user 2026-10-31: "the face looks off"; the look of Rift Apart's Ratchet). The texture holds the left half of the face in
/// its top left corner, mirrored onto the right half (face centre line at x 116): brow x 82..112 y 11..24, eye socket x 84..108
/// y 24..56, nose x 104..116 y 68..76, muzzle x 68..116 y 56..80, mouth line y 84..86, chin y 86..100, cheeks x 24..76 y 36..104
/// (UV map read in a shot whose fur shader wrote the uv: scratch_perf_fur/runs/pics4/uvgrid_front.png). Long tufts on the cheeks,
/// near-smooth fur on the muzzle, the nose bridge and around the mouth that keeps the skin's shading, none on the eye rims (the
/// shells would grow over the eyes), short dense brows that keep their shape. Preview: runs/tools2/zones.py.
static ZONES: [FurZone; 5] = [
	FurZone { at: [20, 32, 78, 110], feather: 12.0, len: 1.45, halo: 1.4, tone: 0.85 },
	FurZone { at: [60, 50, 126, 106], feather: 9.0, len: 0.28, halo: 0.3, tone: 0.0 },
	FurZone { at: [100, 22, 132, 72], feather: 6.0, len: 0.3, halo: 0.3, tone: 0.2 },
	FurZone { at: [80, 20, 112, 60], feather: 5.0, len: 0.0, halo: 0.0, tone: 0.0 },
	FurZone { at: [78, 6, 116, 27], feather: 4.0, len: 0.4, halo: 0.3, tone: 0.5 },
];

static ROWS: [FurRow; 1] = [FurRow { hash: BODY_TEXTURE, part: 0, mask: FurMask { hue: [20.0, 75.0], sat: 0.50, val: 0.06, pale: [45.0, 80.0, 0.20, 0.60], erode: 1, add: &ADD, cut: &CUT, zones: &ZONES } }];

/// One look for the whole body: Soft (short, the PS2 Ratchet with a soft outline), Fluffy (Rift Apart plush).
static PARTS: [FurPart; 1] = [FurPart {
	name: "body",
	len: [0.010, 0.022],
	comb: [0.004, 0.010],
	density: 7.0,
	thick: [1.08, 1.2],
	halo: [1.8, 2.6],
	tip: [1.14, 1.08, 0.98],
	root: 0.05,
	sheen: [0.40, 0.55],
	occ: 0.5,
	spec: 0.6,
	spec_exp: 48.0,
	lift: 0.4,
	sharp: 0.45,
	strand: 0.3,
}];

static SPEC: FurSpec = FurSpec { rows: &ROWS, parts: &PARTS };

pub fn spec() -> Option<&'static FurSpec> {
	Some(&SPEC)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rows_point_at_parts_and_hashes_are_unique() {
		let s = spec().unwrap();
		for r in s.rows {
			assert!((r.part as usize) < s.parts.len());
		}
		for (i, a) in s.rows.iter().enumerate() {
			assert!(s.rows[i + 1..].iter().all(|b| b.hash != a.hash));
		}
	}

	#[test]
	fn fluffy_is_longer_than_soft_and_looks_are_sane() {
		for p in spec().unwrap().parts {
			assert!(p.len[1] > p.len[0] && p.len[0] > 0.0 && p.len[1] < 0.1, "{}", p.name);
			assert!(p.comb.iter().all(|&c| (0.0..=0.05).contains(&c)) && p.density > 0.0);
			assert!(p.thick.iter().chain(&p.halo).all(|&v| v > 0.0 && v <= 4.0));
			assert!((0.0..1.0).contains(&p.root));
		}
	}

	/// The mask keeps every fur colour (stripes and dark brows too) and drops leather, cloth, hardware, pads and see-through texels.
	#[test]
	fn mask_picks_fur_colours_only() {
		let m = FurMask { erode: 0, add: &[], cut: &[], ..ROWS[0].mask };
		let px = |t: [u8; 4]| m.texels(1, 1, &[t])[0];
		assert_eq!(px([196, 144, 38, 255]), 255, "body orange");
		assert_eq!(px([220, 228, 96, 255]), 255, "pale yellow tuft");
		assert_eq!(px([126, 79, 22, 255]), 255, "stripe");
		assert_eq!(px([47, 28, 8, 255]), 255, "dark brow");
		assert_eq!(px([46, 35, 29, 255]), 0, "glove leather");
		assert_eq!(px([68, 42, 34, 255]), 0, "boot");
		assert_eq!(px([24, 57, 32, 255]), 0, "green cloth");
		assert_eq!(px([32, 32, 31, 255]), 0, "black hardware");
		assert_eq!(px([139, 131, 115, 255]), 0, "grey");
		assert_eq!(px([217, 221, 223, 255]), 0, "white");
		assert_eq!(px([198, 173, 134, 255]), 0, "pad");
		assert_eq!(px([51, 15, 15, 255]), 0, "nose");
		assert_eq!(px([196, 144, 38, 0]), 0, "transparent");
	}

	/// The face zones: no fur on the eye rims, short fur that keeps the skin's shading on the muzzle, long tufts on the cheeks, short
	/// brows; the rest of the body keeps the part's look exactly (scale byte 128 = 1.0, tone 255).
	#[test]
	fn face_zones_paint_the_map() {
		let m = FurMask { erode: 0, add: &[], cut: &[], ..ROWS[0].mask };
		let t = vec![[196u8, 144, 38, 255]; 256 * 256];
		let map = m.map(256, 256, &t);
		let at = |x: usize, y: usize| map[y * 256 + x];
		let one = crate::furspec::ZONE_ONE;
		assert_eq!(at(96, 40)[1], 0, "eye rim");
		assert!((at(92, 78)[1] as f32) < 0.35 * one && at(92, 78)[3] < 10, "muzzle");
		assert!((at(48, 70)[1] as f32) > 1.3 * one, "cheek");
		assert!((at(96, 14)[1] as f32) < 0.5 * one, "brow");
		assert_eq!(at(200, 200), [255, 128, 128, 255], "body");
		assert_eq!(at(10, 4), [255, 128, 128, 255], "outside every zone");
	}

	/// The upper chest (x 28..50, y 109..129, skin) keeps its fur; the chest emblem plate and the back harness's hexagon plate do not.
	#[test]
	fn chest_skin_is_fur_and_the_plates_are_not() {
		let m = ROWS[0].mask;
		let in_cut = |x: u16, y: u16| m.cut.iter().any(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3]);
		for (x, y) in [(30, 112), (40, 120), (48, 127), (35, 110)] {
			assert!(!in_cut(x, y), "chest skin {x},{y}");
		}
		for (x, y) in [(5, 20), (10, 40), (232, 150), (240, 160), (225, 150)] {
			assert!(in_cut(x, y), "plate {x},{y}");
		}
	}

	/// Erosion takes one texel off the edge of an island; `add` rectangles become fur, `cut` rectangles none.
	#[test]
	fn mask_erodes_island_edges_and_applies_the_rectangles() {
		let fur = [214, 150, 24, 255];
		let leather = [46, 35, 29, 255];
		let mut t = vec![leather; 25];
		for y in 1..4 {
			for x in 1..4 {
				t[y * 5 + x] = fur;
			}
		}
		let m = FurMask { add: &[], cut: &[], ..ROWS[0].mask }.texels(5, 5, &t);
		assert_eq!(m.iter().filter(|&&b| b != 0).count(), 1);
		assert_eq!(m[2 * 5 + 2], 255);
		let m = FurMask { erode: 0, add: &[[0, 0, 5, 1]], cut: &[[2, 2, 3, 3]], ..ROWS[0].mask }.texels(5, 5, &t);
		assert_eq!(m.iter().filter(|&&b| b != 0).count(), 5 + 8);
		assert_eq!(m[2 * 5 + 2], 0);
	}
}
