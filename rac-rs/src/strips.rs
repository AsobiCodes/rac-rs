//! NOV-14.5 (B) / NOV-15.1b: the Novalis strip-surface actors 678 / 1225 / 761 and the environment-map passes of class
//! 1848: overlay table loaders, the scroll step, the strip wave and the sphere-map UV (decomp/notes/novalis_environment.md
//! "NOV-15.1b port spec").

/// Overlay image base (the `elf_at` convention of water.rs): data virtual address minus this indexes the image.
const IMAGE_BASE: usize = 0x15ef38;
const STRIP_RECORDS: usize = 0x1fbb80;
const STRIP_STRIDE: usize = 0x60;
/// NOV-14.5: the record tables of classes 676 (8 strips), 678 (2) and 1225 (3), same 0x60 record layout.
pub const TABLE_676: (usize, usize) = (0x1e2ec0, 8);
pub const TABLE_678: (usize, usize) = (0x1e3280, 2);
pub const TABLE_1225: (usize, usize) = (0x202c80, 3);
const ENV_COUNTS: usize = 0x202da0;
const ENV_POS: usize = 0x208008;
const ENV_NORMAL: usize = 0x208038;
const ENV_COLOUR: usize = 0x208050;

/// One class-761 strip record (4 x 0x60 at 0x1fbb80): vertices `(x, y, w8, wc)`, uvs, the cull sphere, the two scroll
/// gains, the wave amplitude and phase step, the four scroll rates, the colour word and the two draw passes' texture
/// slot and GS FIX alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct StripRec {
	pub verts: Vec<[f32; 4]>,
	pub uv: Vec<[f32; 2]>,
	pub sphere: [f32; 4],
	pub gain: [f32; 2],
	pub amp: f32,
	pub step: u32,
	pub rate: [f32; 4],
	pub colour: u32,
	pub tex: [usize; 2],
	pub fix: [u8; 2],
}

/// One class-1848 environment pass: positions, normals and colour words of a vertex strip.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvPass {
	pub pos: Vec<[f32; 3]>,
	pub normal: Vec<[f32; 3]>,
	pub colour: Vec<u32>,
}

fn at(img: &[u8], va: usize, len: usize) -> Option<&[u8]> {
	img.get(va.checked_sub(IMAGE_BASE)?..)?.get(..len)
}

fn word(img: &[u8], va: usize) -> Option<u32> {
	Some(u32::from_le_bytes(at(img, va, 4)?.try_into().ok()?))
}

fn float(img: &[u8], va: usize) -> Option<f32> {
	Some(f32::from_bits(word(img, va)?))
}

fn floats<const N: usize>(img: &[u8], va: usize, n: usize) -> Option<Vec<[f32; N]>> {
	(0..n)
		.map(|j| {
			let mut v = [0.0; N];
			for (k, x) in v.iter_mut().enumerate() {
				*x = float(img, va + (j * N + k) * 4)?;
			}
			Some(v)
		})
		.collect()
}

/// The four class-761 strip records from a Novalis overlay image; None when the image does not hold them.
pub fn parse_strips(ov: &[u8]) -> Option<Vec<StripRec>> {
	parse_records(ov, STRIP_RECORDS, 4)
}

/// `count` strip records of 0x60 bytes at `base` (a table of 676 / 678 / 1225 / 761). The cull sphere is read when its
/// pointer resolves and stays zero otherwise (class 678 fills its own on its first update).
pub fn parse_records(ov: &[u8], base: usize, count: usize) -> Option<Vec<StripRec>> {
	(0..count)
		.map(|k| {
			let r = base + k * STRIP_STRIDE;
			let n = word(ov, r + 0xc)? as usize;
			if n == 0 || n > 4096 {
				return None;
			}
			let sphere: [f32; 4] = floats::<4>(ov, word(ov, r + 8)? as usize, 1).map_or([0.0; 4], |s| s[0]);
			let b = at(ov, r + 0x3c, 2)?;
			Some(StripRec {
				verts: floats::<4>(ov, word(ov, r)? as usize, n)?,
				uv: floats::<2>(ov, word(ov, r + 4)? as usize, n)?,
				sphere,
				gain: [float(ov, r + 0x10)?, float(ov, r + 0x14)?],
				amp: float(ov, r + 0x18)?,
				step: word(ov, r + 0x1c)?,
				rate: [float(ov, r + 0x20)?, float(ov, r + 0x24)?, float(ov, r + 0x28)?, float(ov, r + 0x2c)?],
				colour: word(ov, r + 0x30)?,
				tex: [word(ov, r + 0x34)? as usize, word(ov, r + 0x38)? as usize],
				fix: [b[0], b[1]],
			})
		})
		.collect()
}

/// The five class-1848 environment passes from a Novalis overlay image.
pub fn parse_env_passes(ov: &[u8]) -> Option<Vec<EnvPass>> {
	(0..5)
		.map(|k| {
			let n = word(ov, ENV_COUNTS + k * 4)? as usize;
			if n == 0 || n > 4096 {
				return None;
			}
			let colour_at = word(ov, ENV_COLOUR + k * 4)? as usize;
			Some(EnvPass {
				pos: floats::<3>(ov, word(ov, ENV_POS + k * 4)? as usize, n)?,
				normal: floats::<3>(ov, word(ov, ENV_NORMAL + k * 4)? as usize, n)?,
				colour: (0..n).map(|j| word(ov, colour_at + j * 4)).collect::<Option<_>>()?,
			})
		})
		.collect()
}

/// NOV-14.5 (B): the draw state of one strip-surface class (676, 678 or 1225): its records, the per-record scroll words
/// and wave phase, the pulse `I` (+0x5c, written to every record by the 1225 draw callback only), the 678 sphere flag
/// and whether the class's update registered the draw callback this tick.
#[derive(Clone, Debug, Default)]
pub struct StripGroup {
	pub recs: Vec<StripRec>,
	pub scroll: Vec<[f32; 4]>,
	pub phase: Vec<u32>,
	pub pulse: f32,
	pub init: bool,
	pub reg: bool,
}

impl StripGroup {
	pub fn new(recs: Vec<StripRec>) -> StripGroup {
		let n = recs.len();
		StripGroup { recs, scroll: vec![[0.0; 4]; n], phase: vec![0; n], ..Default::default() }
	}


}

#[cfg(test)]
fn wrap(v: &mut f32) {
	if 1.0 < *v {
		*v -= 1.0;
	} else if *v < -1.0 {
		*v += 1.0;
	}
}

/// FUN_002b9010 per strip per drawn tick: the two scroll pairs advance by rate x gain and wrap strictly past +-1, the
/// wave phase by the record step (24 bits).
#[cfg(test)]
pub fn strip_scroll_step(rec: &StripRec, scroll: &mut [f32; 4], phase: &mut u32) {
	scroll[0] += rec.rate[0] * rec.gain[0];
	scroll[1] += rec.rate[1] * rec.gain[0];
	wrap(&mut scroll[0]);
	wrap(&mut scroll[1]);
	scroll[2] += rec.rate[2] * rec.gain[1];
	scroll[3] += rec.rate[3] * rec.gain[1];
	wrap(&mut scroll[2]);
	wrap(&mut scroll[3]);
	*phase = phase.wrapping_add(rec.step) & 0xff_ffff;
}

/// The wave angle index of a vertex (FUN_002621b0): bits 16..24 of the x and y words' sum, times 64, plus the phase, x8, 12 bits.
pub fn wave_index(x: f32, y: f32, phase: u32) -> u32 {
	(((x.to_bits().wrapping_add(y.to_bits()) >> 16) & 0xff) * 64 + phase) << 3 & 0xfff
}

/// FUN_002621b0: the strip's drawn vertices for pulse `i`: positions `(x, y, w8 * I + wc * (1 - I))` and the two uv sets,
/// both the record uv plus its scroll pair, offset by the vertex wave (opposite signs).
pub fn strip_wave(rec: &StripRec, scroll: &[f32; 4], phase: u32, i: f32) -> (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[f32; 2]>) {
	let mut pos = Vec::with_capacity(rec.verts.len());
	let (mut a, mut b) = (Vec::new(), Vec::new());
	for (v, uv) in rec.verts.iter().zip(&rec.uv) {
		pos.push([v[0], v[1], v[2] * i + v[3] * (1.0 - i)]);
		let t = (wave_index(v[0], v[1], phase) as f32 / 4096.0) * 6.2831855 - 3.1415927;
		let wv = [rec.amp * crate::camera::fast_sin(t), rec.amp * crate::camera::fast_cos(t)];
		a.push([uv[0] + scroll[0] + wv[0], uv[1] + scroll[1] + wv[1]]);
		b.push([uv[0] + scroll[2] - wv[0], uv[1] + scroll[3] - wv[1]]);
	}
	(pos, a, b)
}


/// Class 1225 draw callback: the pulse `I` written to `+0x5c` of records 2, 1, 0 after the strips are drawn,
/// `0.5 + 0.5 * sin((float(tick & 0x3f) - 32) * 0.09817477)`.
pub fn pulse_1225(tick: u32) -> f32 {
	0.5 + 0.5 * (((tick & 0x3f) as f32 - 32.0) * 0.09817477f32).sin()
}




#[cfg(test)]
mod tests {
	use super::*;

	/// NOV-14.5 test 2.
	#[test]
	fn pulse_values() {
		assert_eq!(pulse_1225(0x30), 1.0);
		assert_eq!(pulse_1225(0x20), 0.5);
		assert!(pulse_1225(0x10).abs() < 1e-6);
	}

	/// NOV-14.5 test 3: the scene-tick scale table.
	#[test]
	fn scene_scale_table() {
		use crate::gameplay::pal_ticks as sc;
		let got = [0x1d8, 0x219, 0x352, 0x488, 0x44c, 0x94, 0x78, 0x76, 0x3c, 0x1e, 6].map(sc);
		assert_eq!(got, [393, 447, 708, 967, 917, 123, 100, 98, 50, 25, 5]);
	}

	fn rec(rate: [f32; 4], gain: [f32; 2]) -> StripRec {
		StripRec { verts: vec![[216.0, 127.0, 0.55, 0.60]], uv: vec![[0.0, 0.0]], sphere: [0.0; 4], gain, amp: 0.025, step: 1, rate, colour: 0x808080, tex: [44, 44], fix: [0x28, 0x28] }
	}

	/// NOV-15.1b tests 2-3: the pulsed z and the wave angle.
	#[test]
	fn pulsed_z_and_wave_angle() {
		let (p, a, b) = strip_wave(&rec([0.0; 4], [0.007, 0.004]), &[0.0; 4], 0, 0.5);
		assert_eq!(p[0][2], 0.57500005);
		assert_eq!(wave_index(216.0, 127.0, 0), 3072);
		assert_eq!(wave_index(216.0, 127.0, 1), 3080);
		for (phase, want) in [(0u32, 1.5707963f32), (1, 1.5830681), (100, 2.797981)] {
			let t = (wave_index(216.0, 127.0, phase) as f32 / 4096.0) * 6.2831855 - 3.1415927;
			assert!((t - want).abs() < 1e-6, "{phase}");
		}
		// Phase 0: sin = 1, cos = 0, opposite signs on the two uv sets.
		assert!((a[0][0] - 0.025).abs() < 1e-4 && (b[0][0] + 0.025).abs() < 1e-4);
	}

	/// NOV-15.1b test 4: one scroll tick from zero, and the strict wrap.
	#[test]
	fn scroll_tick_from_zero() {
		let r = rec([-0.1, 0.9, 0.1, 0.9], [0.007, 0.004]);
		let (mut s, mut ph) = ([0.0f32; 4], 0u32);
		strip_scroll_step(&r, &mut s, &mut ph);
		assert_eq!((s, ph), ([-0.1f32 * 0.007, 0.9f32 * 0.007, 0.1f32 * 0.004, 0.9f32 * 0.004], 1));
		let mut s = [1.0, -1.0, 0.0, 0.0];
		strip_scroll_step(&rec([0.0, 0.0, 0.0, 0.0], [0.5, 0.5]), &mut s, &mut ph);
		assert_eq!(&s[..2], &[1.0, -1.0], "1.0 and -1.0 are not past the wrap");
		let mut s = [1.5, -1.5, 0.0, 0.0];
		strip_scroll_step(&rec([0.0; 4], [0.5, 0.5]), &mut s, &mut ph);
		assert_eq!(&s[..2], &[0.5, -0.5]);
	}

	/// NOV-15.1b tests 1 and 6: the overlay tables of the Novalis level.
	#[test]
	fn overlay_tables_parse() {
		let mut disc = crate::iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = crate::toc::read_levels(&mut disc).unwrap();
		let l = crate::level::Level::load(&mut disc, &levels[1]).unwrap();
		let ov = l.sections.first().cloned().flatten().expect("overlay");
		let s = parse_strips(&ov).expect("strips");
		assert_eq!(s.iter().map(|r| r.verts.len()).collect::<Vec<_>>(), [48, 48, 48, 40]);
		assert_eq!(s[0].uv.len(), 48);
		assert_eq!((s[0].gain, s[0].amp, s[0].step, s[0].rate, s[0].tex, s[0].fix), ([0.007, 0.004], 0.025, 1, [-0.1, 0.9, 0.1, 0.9], [44, 44], [0x28, 0x28]));
		// The cull sphere pointer (0x161c70) does not land on the sphere floats of the extracted ELF in the level image
		// (the same floats sit 0x48 bytes later); the port draws all four strips and never reads it.
		assert_eq!(s[0].colour, 0x808080);
		// NOV-14.5 (B): the 676 / 678 / 1225 tables.
		let counts = |t: (usize, usize)| parse_records(&ov, t.0, t.1).expect("table").iter().map(|r| r.verts.len()).collect::<Vec<_>>();
		assert_eq!(counts(TABLE_676), [38, 30, 38, 30, 38, 30, 38, 30]);
		assert_eq!(counts(TABLE_678), [4, 4]);
		assert_eq!(counts(TABLE_1225), [94, 70, 82]);
		let r = parse_records(&ov, TABLE_1225.0, 1).unwrap();
		assert_eq!((r[0].tex, r[0].fix, r[0].gain), ([0x2b, 0x2b], [0x30, 0x30], [0.0014, 0.0008]));
		let e = parse_env_passes(&ov).expect("env");
		assert_eq!(e.iter().map(|p| p.pos.len()).collect::<Vec<_>>(), [148, 49, 148, 148, 89]);
		assert!(e.iter().all(|p| p.normal.len() == p.pos.len() && p.colour.len() == p.pos.len()));
	}
}
