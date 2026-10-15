//! Mirror of the arrival warp (game mode 6, substate 4; decomp/notes/level_transition.md SCN-4.1 / SCN-4.2): the
//! warp state retail keeps in RAM, read once per field so the viewer draws retail's warp (star fan, big quad, ribbons,
//! ship stream actors, fade, banner) with the same tick, stream type and history instead of the frozen pad.
//!
//! RAM [C: Novalis FUN_002a2ce0 OV:102529-102700; addresses checked against departure 0164, 2026-10-12]:
//! - boot: type `0x13e088`, segment `0x13e08c`, history head `0x13e080`, count `0x13e084`, history rings P1
//!   `0x13e0f0 + 16 i` and P2 `0x13e2f0 + 16 i`, mark `0x13e056` (byte), from `0x15ed88`, to `0x15ed84`, unlock bytes
//!   `0x13dd40..`, fade `0x15f3fc` (gp-0x7804), drift `0x160530` (gp-0x66d0, 4 floats).
//! - overlay (Novalis addresses, `LEVEL_SHIFT` of the overlay in memory): tick `0x16cc94`, chunk tick `0x16cc98`, end
//!   `0x16cca0`; the warp camera is the render camera (`camera_globals`).
//! - The level byte already names the target during the warp, but the overlay in memory is still the departed level's
//!   (the tick counter sits at Eudora's -0x300 while the byte says 6) [C: log 2026-10-12]. `OV_LEVEL` keeps the level of
//!   the last field outside substates 4-6.
//! - The target's overlay is read in during the no-present fields after the warp (Aridia 0164a f1321-1326 [C log]).
//!   Its warp state starts empty (tick 0, no actors), and the load fields that follow (f1327-1344, draw mask 0x15f3f4
//!   = 0) show the last warp picture with only the banner redrawn (gsfull f1320 vs f1330 / f1344: the pixels differ
//!   only in the banner rows) [C shots 2026-10-12]. `code_sig` notices the load (overlay code words are fixed while one
//!   overlay is in memory) and `read` then keeps handing out the last warp field.

use super::{read_cam, Cam, LEVEL_SHIFT, SHIP_STATE};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

type V3 = [f32; 3];

/// The level whose overlay is in memory (worker thread; see the module doc).
static OV_LEVEL: AtomicU32 = AtomicU32::new(u32::MAX);
/// `code_sig` of the departed overlay, taken at the first field of substates 4-6 (0 = not taken).
static OV_SIG: AtomicU64 = AtomicU64::new(0);
/// The last warp field read from the departed overlay; handed out again once the target's overlay is in memory.
static LAST: std::sync::Mutex<Option<WarpRam>> = std::sync::Mutex::new(None);
static LOADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A fingerprint of the overlay code in memory: words spread over the overlay's text (every level's code covers
/// 0x1f0000..0x280000; Eudora's main loop sits at 0x1e95d8.., its scene code at 0x2897b0 [C]).
fn code_sig(m: &ee_oracle::Machine) -> u64 {
	(0x1f_0000u32..0x28_0000).step_by(0x8000).fold(0xcbf2_9ce4_8422_2325u64, |h, a| (h ^ m.rd32(a + 0x40) as u64).wrapping_mul(0x0100_0000_01b3)) | 1
}

/// One field of retail's warp state.
#[derive(Clone, Debug)]
pub struct WarpRam {
	pub ty: usize,
	pub seg: u32,
	pub tick: u32,
	pub cc98: u32,
	pub end: u32,
	pub fade: f32,
	pub drift: [f32; 4],
	pub head: usize,
	pub count: usize,
	pub hist: [(V3, V3); 32],
	pub from: usize,
	pub to: usize,
	/// unlock[3] (Kerwan): the quiet-warp test of level 1.
	pub kerwan: bool,
	pub mk: usize,
	/// The stream actor mobys (`0x16cdd8`, count `0x16cca4`): (class +0xa6, position +0x10, scale +0x2c, light: ambient
	/// bytes +0x3c..+0x3e and light word +0x38, the pair the moby pass lights with, port_bridges.md BRIDGE-57).
	pub actors: Vec<(u32, V3, f32, ([u8; 3], u32))>,
	/// The render camera retail drew the warp with (the stream key, projection floor applied).
	pub cam: Option<Cam>,
	/// The directional light table the moby pass reads (Novalis `0x1802c0`, Eudora `0x17ffc0`, Veldin `0x17fec0`;
	/// 12 x 0x40 records {colA+wrap, dirA, colB+wrap, dirB}; lighting_fog.md section 2) [C OV:55520 / Eudora OV:61007].
	pub lights: Vec<[[f32; 4]; 4]>,
	/// The slot-0 ship gloss quads retail drew this field (SCN-4.2 Order 6; `gloss` below).
	pub gloss: Vec<crate::particles::WorldQuad>,
	/// Debug (RAC_SCENE_LOG): each stream actor's draw basis rows `+0xc0/+0xd0/+0xe0` and the row `+0xf0`.
	pub basis: Vec<[f32; 12]>,
	/// Debug (RAC_SCENE_LOG): the loading-indicator test words of the warp draw FUN_002808c0 (Eudora OV:107684): the
	/// spinner draws while `0x13d364 > 2 || 0x13d36c >= 0` [C].
	pub load: (i32, i32),
	/// Debug (RAC_SCENE_LOG): the render camera's three basis rows (`camera_globals`), roll included.
	pub cam_rows: [[f32; 3]; 3],
	/// Each stream actor's animation clock (+0x50..) and its scene sequence from RAM (`scene_seq`: the chunk parse
	/// appends the stream record's sequence to the class like a mode-2 scene does).
	pub seqs: Vec<Option<(super::Clock, Vec<u8>)>>,
}

/// The ship gloss (NOV-14.5b, retail callback FUN_002a69d8, Eudora twin OV:109496) as retail left it this field: the
/// callback runs in the warp pass under the ribbons' gate, so its world points `vtx` and UVs `cur` in RAM are what retail
/// drew. Tables (gp-relative, the same in every overlay): colour 0x160560, quad count 0x160580, quad index table
/// pointer 0x1605b0 (four i16 vertex indices per 16 B). State (Novalis addresses + LEVEL_SHIFT; Eudora 0x1c06e0 /
/// 0x1c0d40 [C OV:109496-109560]): vtx 0x1c09e0 (16 B each), cur 0x1c1040 (8 B each). Texture `tex`: fx 1 in mode 6
/// substate 4, else fx 0x15 [C Eudora OV:109466-109477, FUN_001ebb00(tex)]; GS alpha 0x44. Corner order as `ShipGloss::step` (1, 0, 2, 3).
pub(super) fn gloss(m: &ee_oracle::Machine, class: u32, at: impl Fn(u32) -> u32, tex: usize) -> Vec<crate::particles::WorldQuad> {
	let Some(idx) = class.checked_sub(0x212).filter(|&i| i < 4) else { return Vec::new() };
	let nq = m.rd32(0x160580 + 4 * idx).min(128);
	let d = m.rd32(0x1605b0 + 4 * idx) & 0x01ff_ffff;
	let w = m.rd32(0x160560 + 4 * idx);
	let colour: [f32; 4] = std::array::from_fn(|k| ((w >> (8 * k)) & 0xff) as f32 / 128.0);
	if d == 0 {
		return Vec::new();
	}
	(0..nq)
		.map(|q| {
			let v: [u32; 4] = std::array::from_fn(|k| (m.rd16(d + 16 * q + 4 * k as u32) as u32).min(101));
			let p = v.map(|i| m.rdv3(at(0x1c09e0) + 16 * i));
			let t = v.map(|i| [m.rdf(at(0x1c1040) + 8 * i), m.rdf(at(0x1c1040) + 8 * i + 4)]);
			crate::particles::WorldQuad { pos: [p[1], p[0], p[2], p[3]], uv: [t[1], t[0], t[2], t[3]], colour: [colour; 4], tex, additive: false }
		})
		.collect()
}

/// The ship thruster flame draw callback per level (Eudora FUN_0027f7e8 = Novalis 0x2a23e8; `twins.py --all`, binary
/// twins) in where.py LEVELS order [C].
const FLAME_DRAW: [u32; 19] = [
	0x28e970, 0x2a23e8, 0x28e068, 0x27b240, 0x27f7e8, 0x2b70a8, 0x29b9e8, 0x2b5598, 0x296c80, 0x2ab240, 0x27f3f0, 0x2b17f0, 0x2a5180, 0x298d00, 0x294d20,
	0x27c840, 0x2877b0, 0x286b50, 0x28e4a0,
];

/// The address a `lui` at `f + hi` forms with the addiu at `f + lo` on the same register (None when the code differs).
fn lui_addiu(m: &ee_oracle::Machine, f: u32, hi: u32, lo: u32) -> Option<u32> {
	let (w, x) = (m.rd32(f + hi), m.rd32(f + lo));
	(w >> 26 == 0x0f && x >> 26 == 0x09 && (x >> 21 & 31) == (w >> 16 & 31)).then(|| ((w & 0xffff) << 16).wrapping_add(x as i16 as i32 as u32))
}

/// The ship thruster flames retail drew this field: queue A entries (FUN_001ebc48, `orb_queue` arrays) whose fn is the
/// level's FUN_0027f7e8, queued by ship_stream_update (Eudora FUN_00280db0, OV:108189-108278) for the ship actors
/// 0x213..0x215 with intensity moby +0xbc and flicker range +0xb2 [C]: takeoff (substate 0) counter 0x16c994 in
/// (scale 420, scale 504): +0xbc = (counter - scale 420.0) * k * 1.2; landing (substate 8) counter in [scale 240,
/// scale 300): +0xbc = (scale 300.0 - counter) * k * 1.5; both +0xb2 = 0; then the +0xbc 0x32 / +0xb2 10 arm (counter <
/// scale 240 in substate 8), live in arrival landings (RAM 0164k f1430: +0xbc 50, +0xb2 10; level_transition.md SCN-4.3b).
/// The draw [C asm 0x27f7e8-0x27fa54]: ship type t = 0x13e056 (i16); engine table e = code +0x14/+0x4c (t 1: +0x50/+0x58,
/// t 2: +0x64/+0x68), count gp-0x6700 + 4 t; base = moby +0x00 (world bsphere centre, 1/1024 units) * 2^-10; per
/// engine i: a = +0xbc (+ rand_below(+0xb2) when +0xb2 != 0), size = a * e[i].w / 40, corner k = (c[k] * size +
/// e[i].xyz) * rows of the ship moby (0x13e030, +0xc0) + base with c = code +0x184/+0x18c (4 vec4); colour
/// a << 24 | 0x2058b0 (class 0x215: | 0x308000), UV pairs code +0xa0/+0xa8, fx 5 (FUN_001ebb00(5)), ALPHA_1 0x48
/// (additive), submit_strip4 FUN_001ef7c0 (strip v0..v3 = fan 0 1 3 2).
/// Flicker (+0xb2 != 0, the landing arm counter < scale 240: +0xbc 0x32, +0xb2 10): r = rand_below(+0xb2) =
/// ((rand() >> 16) & 0x7fff) % +0xb2 (FUN_0024a418, signed div) once per engine before its quad, a = +0xbc + r, colour
/// alpha byte (a << 24). These are the last rand() calls of the field (call trace 0164k, 73 landing fields, interpreter
/// ORACLE_CALL 0x1160d8 / Kerwan rand_below 0x246258: the 8 calls from 0x27b368 always end the field) [C], so they are
/// recovered from the end-of-field rand state by stepping the LCG back (`rand_back`).
pub(super) fn flames(m: &ee_oracle::Machine, level: u32) -> Vec<crate::particles::WorldQuad> {
	let (Some((fns, args, _)), Some(&f)) = (super::orb_queue(level), FLAME_DRAW.get(level as usize)) else { return Vec::new() };
	if m.rd32(f) != 0x27bd_fea0 {
		return Vec::new();
	}
	let ty = m.rd16(0x13e056) as i16;
	let (hi, lo) = match ty {
		1 => (0x50, 0x58),
		2 => (0x64, 0x68),
		_ => (0x14, 0x4c),
	};
	let (Some(eng), Some(uvs), Some(cor)) = (lui_addiu(m, f, hi, lo), lui_addiu(m, f, 0xa0, 0xa8), lui_addiu(m, f, 0x184, 0x18c)) else { return Vec::new() };
	let gp = if matches!(level, 0 | 5 | 14 | 16) { 0x166c40u32 } else { 0x166c00 };
	let n = (m.rd32(gp.wrapping_sub(0x6700).wrapping_add(4 * ty as u32)) as i32).clamp(0, 8) as u32;
	let ship = m.rd32(0x13e030) & 0x01ff_ffff;
	if ship == 0 {
		return Vec::new();
	}
	let rows = [m.rdv3(ship + 0xc0), m.rdv3(ship + 0xd0), m.rdv3(ship + 0xe0)];
	let uv: [[f32; 2]; 4] = std::array::from_fn(|k| [m.rdf(uvs + 8 * k as u32), m.rdf(uvs + 8 * k as u32 + 4)]);
	let mobys: Vec<u32> = (0..m.rd32(0x15f424).min(0x40)).filter(|&i| m.rd32(fns + 4 * i) == f).map(|i| m.rd32(args + 4 * i) & 0x01ff_ffff).filter(|&a| a != 0).collect();
	let calls = mobys.iter().filter(|&&a| m.rd16(a + 0xb2) != 0).count() * n as usize;
	let mut draws = rand_back(m.rd32((m.rd32(0x12f76c) & 0x01ff_ffff) + 0x58), calls).into_iter();
	let mut out = Vec::new();
	for a in mobys {
		let base = m.rd8(a + 0xbc) as u32;
		let range = m.rd16(a + 0xb2) as i16 as i32;
		let b = m.rdv3(a).map(|x| x * (1.0 / 1024.0));
		let rgb = if m.rd16(a + 0xa6) == 0x215 { 0x30_8000 } else { 0x20_58b0 };
		for e in 0..n {
			let s = if range != 0 { base.wrapping_add(draws.next().map_or(0, |r| (r as i32 % range) as u32)) } else { base };
			let w = s << 24 | rgb;
			let col = [(w & 0xff) as f32 / 128.0, (w >> 8 & 0xff) as f32 / 128.0, (w >> 16 & 0xff) as f32 / 128.0, (w >> 24) as f32 / 128.0];
			let ep = m.rdv3(eng + 16 * e);
			let size = s as i32 as f32 * (m.rdf(eng + 16 * e + 0xc) / 40.0);
			let p: [[f32; 3]; 4] = std::array::from_fn(|k| {
				let c = m.rdv3(cor + 16 * k as u32);
				let v: [f32; 3] = std::array::from_fn(|j| c[j] * size + ep[j]);
				std::array::from_fn(|j| v[0] * rows[0][j] + v[1] * rows[1][j] + v[2] * rows[2][j] + b[j])
			});
			out.push(crate::particles::WorldQuad { pos: [p[0], p[1], p[3], p[2]], uv: [uv[0], uv[1], uv[3], uv[2]], colour: [col; 4], tex: 5, additive: true });
		}
		if std::env::var_os("RAC_SCENE_LOG").is_some() {
			static LAST: AtomicU32 = AtomicU32::new(0);
			let now = m.rd32((m.rd32(0x12f76c) & 0x01ff_ffff) + 0x58);
			let mut x = LAST.swap(now, Ordering::Relaxed);
			let steps = (0..=4096).find(|_| x == now || { x = x.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039); false });
			// Stream counter: Novalis DAT_0016cc94, Kerwan DAT_0016c914 (OV:103215), Eudora 0x16c994 [C Ghidra].
			eprintln!("retail: ship flame rand steps since last field {steps:?}");
			eprintln!("retail: ship flame sub {} counter {} moby {a:#x} class {} bc {base} b2 {} engines {n} base {b:?} q0 {:?}", m.rd32(0x13e050), m.rd32(match level { 1 => 0x16cc94, 3 => 0x16c914, _ => 0x16c994 }) as i32, m.rd16(a + 0xa6), m.rd16(a + 0xb2), out.last().map(|q| (q.pos, q.uv)));
		}
	}
	out
}

/// The last `n` rand() results (in call order) that ended at state `s`: rand() (boot FUN_001160d8) steps
/// s = s * 0x41c64e6d + 0x3039 and rand_below takes (s >> 16) & 0x7fff [C].
fn rand_back(mut s: u32, n: usize) -> Vec<u32> {
	// Inverse of 0x41c64e6d mod 2^32 (Newton: x = x * (2 - a x), 5 rounds from x = a).
	let a = 0x41c6_4e6du32;
	let mut inv = a;
	for _ in 0..5 {
		inv = inv.wrapping_mul(2u32.wrapping_sub(a.wrapping_mul(inv)));
	}
	let mut out = vec![0u32; n];
	for k in (0..n).rev() {
		out[k] = s >> 16 & 0x7fff;
		s = s.wrapping_sub(0x3039).wrapping_mul(inv);
	}
	out
}

/// The warp state of this field: Some only in mode 6, substate 4. Call once per field (it tracks `OV_LEVEL`).
pub fn read(m: &ee_oracle::Machine, mode: u32, level: u32) -> Option<WarpRam> {
	let sub = if mode == 6 { m.rd32(SHIP_STATE) } else { 0 };
	if !(mode == 6 && (4..=6).contains(&sub)) {
		OV_LEVEL.store(level, Ordering::Relaxed);
		OV_SIG.store(0, Ordering::Relaxed);
		LOADED.store(false, Ordering::Relaxed);
	} else {
		let sig = code_sig(m);
		match OV_SIG.load(Ordering::Relaxed) {
			0 => OV_SIG.store(sig, Ordering::Relaxed),
			// The target's overlay was read in (the level byte already names it).
			old if old != sig && !LOADED.load(Ordering::Relaxed) => {
				if std::env::var("RAC_SCENE_LOG").is_ok() {
					eprintln!("retail: warp overlay now level {level} (code signature changed): the last warp field stays up");
				}
				LOADED.store(true, Ordering::Relaxed);
			}
			_ => {}
		}
	}
	if mode != 6 || sub != 4 {
		return None;
	}
	if LOADED.load(Ordering::Relaxed) {
		return LAST.lock().ok()?.clone();
	}
	let w = read_departed(m, level);
	if let Ok(mut l) = LAST.lock() {
		*l = w.clone();
	}
	w
}

/// `read` while the departed overlay is still in memory.
fn read_departed(m: &ee_oracle::Machine, level: u32) -> Option<WarpRam> {
	let ov = match OV_LEVEL.load(Ordering::Relaxed) {
		u32::MAX => level,
		l => l,
	};
	let d = *LEVEL_SHIFT.get(ov as usize)?;
	let at = |a: u32| a.wrapping_add_signed(d);
	let ty = m.rd32(0x13e088) as usize;
	if ty > 4 {
		return None;
	}
	let mut hist = [([0.0; 3], [0.0; 3]); 32];
	for (i, h) in hist.iter_mut().enumerate() {
		*h = (m.rdv3(0x13e0f0 + 16 * i as u32), m.rdv3(0x13e2f0 + 16 * i as u32));
	}
	let dr = m.rdv3(0x160530);
	let (from, to, kerwan) = (m.rd32(0x15ed88) as usize, m.rd32(0x15ed84) as usize, m.rd8(0x13dd43) != 0);
	let actors: Vec<_> = (0..m.rd32(at(0x16cca4)).min(8)).filter_map(|i| {
		let p = m.rd32(at(0x16cdd8) + 4 * i) & 0x01ff_ffff;
		(p != 0).then(|| (m.rd16(p + 0xa6) as u32, m.rdv3(p + 0x10), m.rdf(p + 0x2c), ([m.rd8(p + 0x3c), m.rd8(p + 0x3d), m.rd8(p + 0x3e)], m.rd32(p + 0x38))))
	}).collect();
	// The gloss shares the ribbons' gate (SCN-4.2 Branches): no gloss on the quiet warps.
	let quiet = to == 0 || (to == 1 && !kerwan);
	let gloss = match actors.first() {
		Some(a) if !quiet => gloss(m, a.0, at, 1),
		_ => Vec::new(),
	};
	Some(WarpRam {
		ty,
		seg: m.rd32(0x13e08c),
		tick: m.rd32(at(0x16cc94)),
		cc98: m.rd32(at(0x16cc98)),
		end: m.rd32(at(0x16cca0)),
		fade: m.rdf(0x15f3fc),
		drift: [dr[0], dr[1], dr[2], m.rdf(0x16053c)],
		head: (m.rd32(0x13e080) & 31) as usize,
		count: m.rd32(0x13e084).min(32) as usize,
		hist,
		from,
		to,
		kerwan,
		mk: (m.rd8(0x13e056) as usize).min(2),
		cam: read_cam(m, ov),
		lights: (0..12u32).map(|k| std::array::from_fn(|q| {
			let r = at(0x1802c0) + k * 0x40 + 16 * q as u32;
			[m.rdf(r), m.rdf(r + 4), m.rdf(r + 8), m.rdf(r + 12)]
		})).collect(),
		basis: (0..m.rd32(at(0x16cca4)).min(8)).filter_map(|i| {
			let p = m.rd32(at(0x16cdd8) + 4 * i) & 0x01ff_ffff;
			(p != 0).then(|| { let r = [0xc0u32, 0xd0, 0xe0, 0xf0].map(|o| m.rdv3(p + o)); std::array::from_fn(|k| r[k / 3][k % 3]) })
		}).collect(),
		seqs: (0..m.rd32(at(0x16cca4)).min(8)).map(|i| {
			let p = m.rd32(at(0x16cdd8) + 4 * i) & 0x01ff_ffff;
			if p == 0 {
				return None;
			}
			super::scene_seq(m, p, m.rd16(p + 0xa6) as u32).map(|b| (super::Clock::read(m, p), b))
		}).collect(),
		load: (m.rd32(0x13d364) as i32, m.rd32(0x13d36c) as i32),
		cam_rows: super::camera_globals(ov).map_or([[0.0; 3]; 3], |(_, b, _)| [m.rdv3(b), m.rdv3(b + 0x10), m.rdv3(b + 0x20)]),
		actors,
		gloss,
	})
}

impl super::Retail {
	/// Mode 6 substates 0 / 8: retail's ship gloss (canopy) quads of the last field.
	pub fn ship_gloss(&self) -> &[crate::particles::WorldQuad] {
		&self.snap.ship_gloss
	}

	/// Retail's warp state of the last field (mode 6 substate 4), for the viewer's warp mirror.
	pub fn warp_ram(&self) -> Option<&WarpRam> {
		self.snap.warp.as_ref()
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn flame_rand_back_inverts_the_lcg() {
		let mut s = 0x1234_5678u32;
		let mut fwd = Vec::new();
		for _ in 0..9 {
			s = s.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
			fwd.push(s >> 16 & 0x7fff);
		}
		assert_eq!(super::rand_back(s, 9), fwd);
	}
}
