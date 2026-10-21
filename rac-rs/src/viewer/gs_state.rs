//! One decode of the retail GS draw state for every native draw path (run 6 of lane scratch_cliff2, "general fix").
//!
//! Each native path (tfrag, tie, shrub, moby, effects, sky, ocean, HUD) builds a [`GsRegs`] from the register words retail
//! writes for that draw (level data, the draw packet constants, or the live GIF words in RAM), and takes its sampler, alpha
//! test, blend, fog and Z write from [`GsState::decode`]. A path states what it implements with a [`Support`] mask;
//! [`GsState::unhandled`] lists every decoded value outside that mask, and [`guard`] logs each (path, state) pair once with a
//! count when `RAC_GSSTATE_LOG=1`, so a register a path ignores is visible instead of silently drawn wrong (the tie CLAMP and
//! tie alpha-blend bugs of runs 4-5 were exactly that: two register words no path read).
//!
//! Register layouts are the GS manual's (TEX0_1 0x06, CLAMP_1 0x08, TEX1_1 0x14, ALPHA_1 0x42, TEST_1 0x47, PABE 0x49,
//! FBA_1 0x4a, ZBUF_1 0x4e, COLCLAMP 0x46, FOGCOL 0x3d, TEXA 0x3b, PRIM 0x00).

use std::collections::HashMap;
use std::sync::Mutex;

/// The raw register words of one draw. `None` = the path does not know the word (it is not part of the decode then).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GsRegs {
	pub prim: Option<u64>,
	pub tex0: Option<u64>,
	pub tex1: Option<u64>,
	pub clamp: Option<u64>,
	pub test: Option<u64>,
	pub alpha: Option<u64>,
	pub fba: Option<u64>,
	pub pabe: Option<u64>,
	pub zbuf: Option<u64>,
	pub colclamp: Option<u64>,
	pub fogcol: Option<u64>,
	pub texa: Option<u64>,
}

/// GS alpha / depth compare (TEST ATST, ZTST map onto the same order for the first four).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cmp {
	Never,
	Always,
	Less,
	LEqual,
	Equal,
	GEqual,
	Greater,
	NotEqual,
}

/// TEST AFAIL: what a pixel that fails the alpha test still writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AFail {
	Keep,
	FbOnly,
	ZbOnly,
	RgbOnly,
}

/// TEST: alpha test (`ate`), destination alpha test and depth test.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Test {
	pub ate: bool,
	pub atst: Cmp,
	/// AREF, 0..=0xff on the GS alpha scale (0x80 = 1.0).
	pub aref: u8,
	pub afail: AFail,
	pub date: bool,
	pub datm: bool,
	pub zte: bool,
	/// ZTST: Never, Always, GEqual, Greater (the GS keeps larger Z nearer; a native path with smaller-is-nearer depth maps
	/// GEqual -> LessEqual and Greater -> Less).
	pub ztst: Cmp,
}

/// One blend input (ALPHA A, B, D) and the factor (C).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlendIn {
	Cs,
	Cd,
	Zero,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlendC {
	As,
	Ad,
	Fix,
}

/// ALPHA: ((A - B) * C >> 7) + D, `fix` on the 0x80 = 1.0 scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Blend {
	pub a: BlendIn,
	pub b: BlendIn,
	pub c: BlendC,
	pub d: BlendIn,
	pub fix: u8,
}

/// TEX1 + CLAMP: the sampler a draw needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sampler {
	/// CLAMP WMS / WMT: 0 REPEAT, 1 CLAMP, 2 REGION_CLAMP, 3 REGION_REPEAT.
	pub wms: u8,
	pub wmt: u8,
	/// CLAMP MINU, MAXU, MINV, MAXV (region modes only).
	pub region: [u16; 4],
	/// TEX1 MMAG: true = LINEAR.
	pub mag_linear: bool,
	/// TEX1 MMIN 0..5: NEAREST, LINEAR, NEAREST_MIPMAP_NEAREST, NEAREST_MIPMAP_LINEAR, LINEAR_MIPMAP_NEAREST,
	/// LINEAR_MIPMAP_LINEAR.
	pub mmin: u8,
	pub mxl: u8,
	/// TEX1 LCM (true = fixed K), L, K (signed, 1/16 mip).
	pub lcm: bool,
	pub l: u8,
	pub k: i16,
}

/// The decoded draw state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GsState {
	pub textured: bool,
	pub gouraud: bool,
	/// PRIM FGE: fog on.
	pub fog: bool,
	/// PRIM ABE: blending on (`blend` is then used).
	pub abe: bool,
	pub aa1: bool,
	/// TEX0 TCC (texel alpha used), TFX 0 MODULATE, 1 DECAL, 2 HIGHLIGHT, 3 HIGHLIGHT2, PSM, CPSM.
	pub tcc: bool,
	pub tfx: u8,
	pub psm: u8,
	pub sampler: Sampler,
	pub test: Test,
	pub blend: Blend,
	/// FBA: the written alpha gets bit 7 set. PABE: blend only where the source alpha has bit 7 set.
	pub fba: bool,
	pub pabe: bool,
	/// ZBUF ZMSK: no Z write.
	pub zmsk: bool,
	/// COLCLAMP: true = clamp to 0..255 (false = wrap the low 8 bits).
	pub colclamp: bool,
	pub fogcol: [u8; 3],
	/// TEXA TA0, AEM, TA1 (16/24-bit texel alpha expansion).
	pub texa: (u8, bool, u8),
}

fn cmp(v: u64) -> Cmp {
	[Cmp::Never, Cmp::Always, Cmp::Less, Cmp::LEqual, Cmp::Equal, Cmp::GEqual, Cmp::Greater, Cmp::NotEqual][(v & 7) as usize]
}

fn blend_in(v: u64) -> BlendIn {
	[BlendIn::Cs, BlendIn::Cd, BlendIn::Zero, BlendIn::Zero][(v & 3) as usize]
}

impl GsState {
	/// Decodes `r`; a missing word decodes as the GS power-on value of that register (0), except COLCLAMP (retail sets 1).
	pub fn decode(r: &GsRegs) -> Self {
		let prim = r.prim.unwrap_or(0);
		let tex0 = r.tex0.unwrap_or(0);
		let tex1 = r.tex1.unwrap_or(0);
		let clamp = r.clamp.unwrap_or(0);
		let test = r.test.unwrap_or(0);
		let alpha = r.alpha.unwrap_or(0);
		let fogcol = r.fogcol.unwrap_or(0);
		let texa = r.texa.unwrap_or(0);
		let k = ((tex1 >> 32) & 0xfff) as i16;
		GsState {
			textured: prim >> 4 & 1 != 0,
			gouraud: prim >> 3 & 1 != 0,
			fog: prim >> 5 & 1 != 0,
			abe: prim >> 6 & 1 != 0,
			aa1: prim >> 7 & 1 != 0,
			tcc: tex0 >> 34 & 1 != 0,
			tfx: (tex0 >> 35 & 3) as u8,
			psm: (tex0 >> 20 & 0x3f) as u8,
			sampler: Sampler {
				wms: (clamp & 3) as u8,
				wmt: (clamp >> 2 & 3) as u8,
				region: [(clamp >> 4 & 0x3ff) as u16, (clamp >> 14 & 0x3ff) as u16, (clamp >> 24 & 0x3ff) as u16, (clamp >> 34 & 0x3ff) as u16],
				mag_linear: tex1 >> 5 & 1 != 0,
				mmin: (tex1 >> 6 & 7) as u8,
				mxl: (tex1 >> 2 & 7) as u8,
				lcm: tex1 & 1 != 0,
				l: (tex1 >> 19 & 3) as u8,
				k: if k >= 0x800 { k - 0x1000 } else { k },
			},
			test: Test {
				ate: test & 1 != 0,
				atst: cmp(test >> 1),
				aref: (test >> 4 & 0xff) as u8,
				afail: [AFail::Keep, AFail::FbOnly, AFail::ZbOnly, AFail::RgbOnly][(test >> 12 & 3) as usize],
				date: test >> 14 & 1 != 0,
				datm: test >> 15 & 1 != 0,
				zte: test >> 16 & 1 != 0,
				ztst: [Cmp::Never, Cmp::Always, Cmp::GEqual, Cmp::Greater][(test >> 17 & 3) as usize],
			},
			blend: Blend {
				a: blend_in(alpha),
				b: blend_in(alpha >> 2),
				c: [BlendC::As, BlendC::Ad, BlendC::Fix, BlendC::Fix][(alpha >> 4 & 3) as usize],
				d: blend_in(alpha >> 6),
				fix: (alpha >> 32 & 0xff) as u8,
			},
			fba: r.fba.unwrap_or(0) & 1 != 0,
			pabe: r.pabe.unwrap_or(0) & 1 != 0,
			zmsk: r.zbuf.unwrap_or(0) >> 32 & 1 != 0,
			colclamp: r.colclamp.unwrap_or(1) & 1 != 0,
			fogcol: [fogcol as u8, (fogcol >> 8) as u8, (fogcol >> 16) as u8],
			texa: (texa as u8, texa >> 15 & 1 != 0, (texa >> 32) as u8),
		}
	}

	/// The alpha test as the native passes split it: `Some((cmp, aref, afail))` when ATE is on and the test is not ALWAYS.
	pub fn alpha_test(&self) -> Option<(Cmp, u8, AFail)> {
		(self.test.ate && self.test.atst != Cmp::Always).then_some((self.test.atst, self.test.aref, self.test.afail))
	}

	/// True when a texel that passes writes Z: ZTE on, ZMSK off (AFAIL ZB_ONLY / KEEP decide for the failing texels).
	#[cfg(test)]
	pub fn writes_z(&self) -> bool {
		self.test.zte && !self.zmsk
	}

	/// The wgpu blend for ABE + ALPHA, or None when the draw is opaque. `Err` = a formula no fixed-function wgpu blend
	/// state expresses (a path must then blend in the shader or log it as unhandled).
	#[cfg(test)]
	pub fn wgpu_blend(&self) -> Result<Option<wgpu::BlendState>, Blend> {
		if !self.abe {
			return Ok(None);
		}
		blend_state(self.blend).map(Some).ok_or(self.blend)
	}

	/// Lists the decoded values a path does not handle (`s` = what it implements), as (field, value) pairs.
	pub fn unhandled(&self, s: &Support) -> Vec<(&'static str, u32)> {
		let mut out = Vec::new();
		let sm = &self.sampler;
		if self.textured {
			if sm.wms >= 2 && !s.region_clamp || sm.wmt >= 2 && !s.region_clamp {
				out.push(("clamp.region", (sm.wms | sm.wmt << 2) as u32));
			}
			if (sm.wms == 1 || sm.wmt == 1) && !s.clamp {
				out.push(("clamp.clamp", (sm.wms | sm.wmt << 2) as u32));
			}
			if sm.mxl > 0 && sm.mmin >= 2 && !s.mip {
				out.push(("tex1.mip", (sm.mxl as u32) << 8 | sm.mmin as u32));
			}
			if let Some(m) = s.mip_model.filter(|&m| sm.mmin != m || sm.lcm || sm.l != 0) {
				out.push(("tex1.mip_model", (sm.mmin as u32) << 8 | (sm.l as u32) << 1 | sm.lcm as u32 | (m as u32) << 16));
			}
			if !sm.mag_linear && !s.nearest {
				out.push(("tex1.mmag_nearest", 0));
			}
			if self.tfx != 0 && !s.tfx.contains(&self.tfx) {
				out.push(("tex0.tfx", self.tfx as u32));
			}
			if !self.tcc && !s.tcc_off {
				out.push(("tex0.tcc_off", 0));
			}
		}
		if let Some((c, aref, afail)) = self.alpha_test() {
			if !s.alpha_test.iter().any(|&(sc, sr, sf)| sc == c && (sr == aref || sr == 0xff) && sf == afail) {
				out.push(("test.alpha", (c as u32) << 16 | (aref as u32) << 4 | afail as u32));
			}
		}
		if self.test.date && !s.date {
			out.push(("test.date", self.test.datm as u32));
		}
		if self.abe {
			match blend_state(self.blend) {
				None => out.push(("alpha.formula", blend_word(self.blend))),
				Some(_) if !s.blend.contains(&(blend_word(self.blend) & 0xff)) && !s.any_blend => out.push(("alpha.blend", blend_word(self.blend))),
				_ => {}
			}
			if self.pabe && !s.pabe {
				out.push(("pabe", 1));
			}
		}
		if self.fog && !s.fog {
			out.push(("prim.fge", 1));
		}
		if self.aa1 && !s.aa1 {
			out.push(("prim.aa1", 1));
		}
		if self.fba && !s.fba {
			out.push(("fba", 1));
		}
		if !self.colclamp && !s.colclamp_off {
			out.push(("colclamp.off", 0));
		}
		out
	}
}

/// ALPHA as the low register word (A | B << 2 | C << 4 | D << 6 | FIX << 32 folded to u32: FIX in bits 8..16).
pub fn blend_word(b: Blend) -> u32 {
	let i = |x: BlendIn| match x {
		BlendIn::Cs => 0,
		BlendIn::Cd => 1,
		BlendIn::Zero => 2,
	};
	let c = match b.c {
		BlendC::As => 0,
		BlendC::Ad => 1,
		BlendC::Fix => 2,
	};
	i(b.a) | i(b.b) << 2 | c << 4 | i(b.d) << 6 | (if b.c == BlendC::Fix { (b.fix as u32) << 8 } else { 0 })
}

/// ((A - B) * C) + D as a wgpu blend: the Cs and Cd coefficients must each be 0, 1, C or 1 - C (one of them may be -C
/// with the other non-negative: subtract). FIX goes through the blend constant (`BlendFactor::Constant`, set per draw with
/// `set_blend_constant(fix / 128)`); Ad through the destination alpha (the native targets keep alpha = written alpha).
pub fn blend_state(b: Blend) -> Option<wgpu::BlendState> {
	use wgpu::{BlendComponent, BlendFactor as F, BlendOperation as Op};
	// Coefficient of Cs and Cd in units: (constant part, C part).
	let coef = |x: BlendIn| -> (i32, i32) {
		((b.d == x) as i32, (b.a == x) as i32 - (b.b == x) as i32)
	};
	let (cs, cd) = (coef(BlendIn::Cs), coef(BlendIn::Cd));
	let (c, one_minus_c) = match b.c {
		BlendC::As => (F::SrcAlpha, F::OneMinusSrcAlpha),
		BlendC::Ad => (F::DstAlpha, F::OneMinusDstAlpha),
		BlendC::Fix => (F::Constant, F::OneMinusConstant),
	};
	let factor = |k: (i32, i32)| -> Option<(F, bool)> {
		Some(match k {
			(0, 0) => (F::Zero, false),
			(1, 0) => (F::One, false),
			(0, 1) => (c, false),
			(1, -1) => (one_minus_c, false),
			(0, -1) => (c, true),
			(-1, 0) => (F::One, true),
			_ => return None,
		})
	};
	let (fs, ns) = factor(cs)?;
	let (fd, nd) = factor(cd)?;
	let operation = match (ns, nd) {
		(false, false) => Op::Add,
		(false, true) => Op::Subtract,
		(true, false) => Op::ReverseSubtract,
		(true, true) => return None,
	};
	let color = BlendComponent { src_factor: fs, dst_factor: fd, operation };
	// The written alpha is the source alpha (the GS blends RGB only).
	Some(wgpu::BlendState { color, alpha: BlendComponent { src_factor: F::One, dst_factor: F::Zero, operation: Op::Add } })
}

/// The wgpu depth compare for a GS ZTST (a native path with smaller-is-nearer depth): GEQUAL passes nearer-or-equal
/// (`LessEqual`, so a coplanar later draw wins as on the GS), GREATER strictly nearer (`Less`).
pub fn depth_compare(z: Cmp) -> wgpu::CompareFunction {
	match z {
		Cmp::Never => wgpu::CompareFunction::Never,
		Cmp::Always => wgpu::CompareFunction::Always,
		Cmp::GEqual => wgpu::CompareFunction::LessEqual,
		_ => wgpu::CompareFunction::Less,
	}
}

/// ZTST GEQUAL, the test of every retail world and effect draw (fx_census_late.md: only HUD, sky and zmsk draws use ALWAYS).
pub fn ztest() -> wgpu::CompareFunction {
	depth_compare(Cmp::GEqual)
}

/// What one native path implements. `blend` lists the ALPHA words by their low byte (A | B << 2 | C << 4 | D << 6; a FIX value
/// in bits 8.. is the path's constant, never compared). `alpha_test` lists the (compare, AREF, AFAIL) triples it splits into passes (AREF 0xff
/// = any AREF); `blend` the ALPHA words (`blend_word` & 0xff) it draws, or `any_blend` when it takes the blend from `blend_state`.
#[derive(Clone, Debug, Default)]
pub struct Support {
	pub clamp: bool,
	pub region_clamp: bool,
	pub mip: bool,
	/// The MMIN the path's mip selection implements (with LCM 0, L 0): anything else is `tex1.mip_model`.
	pub mip_model: Option<u8>,
	pub nearest: bool,
	pub tfx: Vec<u8>,
	pub tcc_off: bool,
	pub alpha_test: Vec<(Cmp, u8, AFail)>,
	pub date: bool,
	pub blend: Vec<u32>,
	pub any_blend: bool,
	pub pabe: bool,
	pub fog: bool,
	pub aa1: bool,
	pub fba: bool,
	pub colclamp_off: bool,
}

/// True with `RAC_GSSTATE_LOG=1`. Callers test this before building a `GsRegs`, so the guard costs one load when off.
pub fn enabled() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_GSSTATE_LOG").is_ok_and(|v| v == "1"))
}

/// PRIM of the world paths' triangles: TME, IIP, FGE, ABE, type TRIANGLE_STRIP (retail draws terrain, ties, shrubs and mobys
/// fogged, textured, Gouraud and blended).
pub const PRIM_WORLD: u64 = 0x7c;
/// TEST_1 of the world passes (fx_census_late.md, GSPIX): tfrag and moby 0x5360b (ATE GEQUAL AREF 0x60 AFAIL RGB_ONLY, ZTE
/// ZTST GEQUAL), TieProc 0x5340b (AREF 0x40), shrub 0x5320b (AREF 0x20). ALPHA_1 0x44 = (Cs - Cd) * As + Cd.
pub const TEST_TFRAG: u64 = 0x5360b;
pub const TEST_TIE: u64 = 0x5340b;
pub const TEST_SHRUB: u64 = 0x5320b;
pub const ALPHA_LERP: u64 = 0x44;
/// TEX0 of a MODULATE texture with texel alpha (TCC 1, TFX 0): the world materials are patched at load, only the flags matter.
pub const TEX0_MODULATE: u64 = 1 << 34;

/// What a world path (tfrag, tie, shrub, moby) implements: CLAMP both axes, TEX1 K and MMIN / MXL, linear mag, MODULATE with
/// texel alpha, its own GEQUAL alpha test split, the 0x44 lerp, fog.
pub fn world_support(aref: u8) -> Support {
	Support { clamp: true, mip: true, mip_model: Some(4), nearest: false, tfx: vec![0], alpha_test: vec![(Cmp::GEqual, aref, AFail::RgbOnly)], blend: vec![0x44], fog: true, colclamp_off: false, ..Default::default() }
}

/// Guards one world material (tfrag / tie / shrub / moby record): `tex1` and `clamp` come from the level data, the rest are the
/// retail constants above.
pub fn guard_world(path: &'static str, test: u64, tex1: u64, clamp: u64) {
	let regs = GsRegs { prim: Some(PRIM_WORLD), tex0: Some(TEX0_MODULATE), tex1: Some(tex1), clamp: Some(clamp), test: Some(test), alpha: Some(ALPHA_LERP), ..Default::default() };
	let mut s = world_support((test >> 4) as u8);
	if path == "moby" {
		// Mobies draw the base level with the packet's CLAMP wrap (mobies.rs `packet_strips`); no mip selection.
		(s.mip, s.mip_model) = (false, None);
	}
	guard(path, &GsState::decode(&regs), &s);
}

/// Guards a native path whose retail GS state is a census constant (fx_census_late.md, qd/GSPIX census), not a live register
/// stream: sky shells, oceans, FIX strips / pools, shadow volumes, queue-hand quads. Each call names the path; the state is what
/// retail sets for it and `Support` is what the path implements, so a finding means the path draws that state differently.
pub fn guard_path(path: &'static str) {
	if !enabled() {
		return;
	}
	// (PRIM, ALPHA, TEST, CLAMP, TEX1, support)
	let (prim, alpha, test, clamp, tex1, s) = match path {
		// Sky shells: TME IIP ABE strip, FGE 0, ALPHA 0x44 / 0x48, CLAMP both axes (5), ZTST ALWAYS.
		"sky" => (0x5c, 0x44, 0x30000, 5, 0x20, Support { clamp: true, blend: vec![0x44, 0x48], ..Default::default() }),
		// Ocean patches: fogged FIX blend (0x64), TEST 0x5360a (ATE off, ZTST GEQUAL), FOGCOL per descriptor.
		"ocean" => (0x7c, 0x64 | 0x20 << 32, 0x5360a, 0, 0x20, Support { blend: vec![0x64, 0x68], fog: true, ..Default::default() }),
		// FIX strips and pools (ocean RAM strips, efx pools, queue / strip records): ALPHA 0x64 / 0x68, level fog.
		"strips" => (0x7c, 0x64 | 0x80 << 32, 0x5360a, 0, 0x20, Support { blend: vec![0x64, 0x68], fog: true, ..Default::default() }),
		// Shadow volumes: ALPHA 0x44, TEST 0x51001 (ATE NEVER, AFAIL FB_ONLY: the stencil trick), no texture.
		"shadowvol" => (0x44, 0x44 | 0x80 << 32, 0x51001, 0, 0x20, Support { blend: vec![0x44], alpha_test: vec![(Cmp::Never, 0xff, AFail::FbOnly)], ..Default::default() }),
		// Queue-hand quads (queue_b / q1 / q2 / fxa / fxb): ALPHA 0x48 additive or 0x44 lerp, CLAMP 5, TEX1 0x260, no fog.
		"queue" => (0x5c, 0x48, 0x30000, 5, 0x260, Support { clamp: true, blend: vec![0x48, 0x44], ..Default::default() }),
		_ => return,
	};
	let regs = GsRegs { prim: Some(prim), tex0: Some(TEX0_MODULATE), tex1: Some(tex1), clamp: Some(clamp), test: Some(test), alpha: Some(alpha), ..Default::default() };
	guard(path, &GsState::decode(&regs), &s);
}

/// What the effect layer (retail/efx_draw.rs) implements of a prim's live GS state: ABE blends 0x48 / 0x44 / 0x64 / 0x68, the
/// depth filter of `depth_ok` (ZTST GEQUAL / GREATER only, no destination alpha test), CLAMP 5 (both axes), the level sampler,
/// PRIM FGE with the vertex fog byte and FOGCOL (`efx_draw::fog_tex`).
pub fn efx_support() -> Support {
	// The effect layer writes no Z, so an alpha test whose failing pixels still write the frame (AFAIL FB_ONLY / RGB_ONLY, any
	// compare and AREF) rejects nothing it draws (`efx_draw::depth_ok` skips the prims whose AFAIL keeps the old colour).
	let cmps = [Cmp::Never, Cmp::Always, Cmp::Less, Cmp::LEqual, Cmp::Equal, Cmp::GEqual, Cmp::Greater, Cmp::NotEqual];
	let alpha_test = cmps.iter().flat_map(|&c| [AFail::FbOnly, AFail::RgbOnly].map(|f| (c, 0xff, f))).collect();
	Support { blend: vec![0x48, 0x44, 0x64, 0x68], alpha_test, fog: true, ..Default::default() }
}

static LOG: Mutex<Option<HashMap<(&'static str, &'static str, u32), u64>>> = Mutex::new(None);

/// The guard: with `RAC_GSSTATE_LOG=1`, every (path, field, value) a path does not handle is printed the first time and
/// at counts 10, 100, 1000, ... (stderr, `GSSTATE path field=0x.. count n`). Cheap when the variable is unset.
pub fn guard(path: &'static str, st: &GsState, s: &Support) {
	if !enabled() {
		return;
	}
	for (field, v) in st.unhandled(s) {
		note(path, field, v);
	}
}

/// One logged finding (see `guard`): also for a path's own check of a register word it cannot express as a `GsRegs`.
pub fn note(path: &'static str, field: &'static str, v: u32) {
	if !enabled() {
		return;
	}
	let mut g = LOG.lock().unwrap();
	let n = g.get_or_insert_with(HashMap::new).entry((path, field, v)).or_insert(0);
	*n += 1;
	if log_tick(*n) {
		eprintln!("GSSTATE {path} {field}=0x{v:x} count {n}");
	}
}

/// 1, 10, 100, ...: the counts `guard` prints at.
fn log_tick(mut n: u64) -> bool {
	while n >= 10 && n % 10 == 0 {
		n /= 10;
	}
	n == 1
}

#[cfg(test)]
mod tests {
	use super::*;

	/// TieProc's TEST_1 0x5340b and ALPHA_1 0x44 (decomp/notes/fx_census_late.md "Tie CLAMP and texel alpha").
	#[test]
	fn decodes_the_tie_draw_state() {
		let st = GsState::decode(&GsRegs { prim: Some(0x5c), test: Some(0x5340b), alpha: Some(0x2000000044), clamp: Some(0x5d000004), tex1: Some(0xff92_0000_012c), ..Default::default() });
		assert_eq!(st.alpha_test(), Some((Cmp::GEqual, 0x40, AFail::RgbOnly)));
		assert!(st.test.zte && st.test.ztst == Cmp::GEqual && st.writes_z());
		assert_eq!((st.sampler.wms, st.sampler.wmt), (0, 1));
		assert_eq!(st.sampler.k, -0x6e);
		assert_eq!(blend_word(st.blend), 0x44);
		assert!(st.textured && st.gouraud && st.abe && !st.fog);
		let b = st.wgpu_blend().unwrap().unwrap();
		assert_eq!((b.color.src_factor, b.color.dst_factor, b.color.operation), (wgpu::BlendFactor::SrcAlpha, wgpu::BlendFactor::OneMinusSrcAlpha, wgpu::BlendOperation::Add));
	}

	/// The blend formulas retail uses map onto fixed-function wgpu blends: 0x44 lerp, 0x48 additive, 0x64 / 0x68 FIX, 0x42
	/// Cd - Cs * As (subtract); 0x04 (Cs - Cd) * As + Cs is not expressible.
	#[test]
	fn blend_formulas() {
		use wgpu::{BlendFactor as F, BlendOperation as Op};
		let dec = |w: u64| GsState::decode(&GsRegs { prim: Some(0x40), alpha: Some(w), ..Default::default() }).wgpu_blend();
		let fs = |w| dec(w).unwrap().map(|b| (b.color.src_factor, b.color.dst_factor, b.color.operation));
		assert_eq!(fs(0x48), Some((F::SrcAlpha, F::One, Op::Add)));
		assert_eq!(fs(0x8000000064), Some((F::Constant, F::OneMinusConstant, Op::Add)));
		assert_eq!(fs(0x68), Some((F::Constant, F::One, Op::Add)));
		// (Cd - Cs) * As + Cs.
		assert_eq!(fs(0x01), Some((F::OneMinusSrcAlpha, F::SrcAlpha, Op::Add)));
		// (0 - Cs) * As + Cd = Cd - Cs * As.
		assert_eq!(fs(0x42), Some((F::SrcAlpha, F::One, Op::ReverseSubtract)));
		assert!(dec(0x04).is_err(), "(Cs - Cd) * As + Cs is not a fixed-function blend");
		assert_eq!(GsState::decode(&GsRegs { alpha: Some(0x44), ..Default::default() }).wgpu_blend(), Ok(None), "ABE off");
	}

	/// `unhandled` names exactly the decoded values outside a path's `Support`.
	#[test]
	fn unhandled_lists_ignored_state() {
		let st = GsState::decode(&GsRegs { prim: Some(0x7c), test: Some(0x5340b), alpha: Some(0x44), clamp: Some(0x5), tex1: Some(0x12c), ..Default::default() });
		let none = Support { nearest: true, tcc_off: true, ..Default::default() };
		let names: Vec<_> = st.unhandled(&none).into_iter().map(|(f, _)| f).collect();
		assert_eq!(names, vec!["clamp.clamp", "tex1.mip", "test.alpha", "alpha.blend", "prim.fge"]);
		let full = Support { clamp: true, mip: true, nearest: true, tcc_off: true, alpha_test: vec![(Cmp::GEqual, 0x40, AFail::RgbOnly)], blend: vec![0x44], fog: true, ..Default::default() };
		assert!(st.unhandled(&full).is_empty());
		assert!(log_tick(1) && log_tick(10) && log_tick(1000) && !log_tick(20) && !log_tick(0));
	}

	/// The effect layer accepts every colour-writing alpha test (no Z write) and flags the ones it would have to cut pixels for.
	#[test]
	fn efx_alpha_test_support() {
		let efx = |test: u64| GsState::decode(&GsRegs { prim: Some(0x4c), test: Some(test), alpha: Some(0x44), ..Default::default() }).unhandled(&efx_support());
		// GEQUAL 0x70 RGB_ONLY (0x50703) and NEVER FB_ONLY (0x3f1 form): colour always written.
		assert!(efx(0x50703).is_empty() && efx(0x1 | 0x1000).is_empty());
		// GEQUAL 0x70 AFAIL KEEP: pixels below 0x70 are rejected, which the layer cannot express.
		assert_eq!(efx(0x50701).iter().map(|x| x.0).collect::<Vec<_>>(), vec!["test.alpha"]);
	}

	/// ZTST maps with equality included (GEQUAL), GREATER strictly.
	#[test]
	fn ztst_maps_with_equality() {
		use wgpu::CompareFunction as C;
		assert_eq!((depth_compare(Cmp::GEqual), depth_compare(Cmp::Greater), depth_compare(Cmp::Always), depth_compare(Cmp::Never)), (C::LessEqual, C::Less, C::Always, C::Never));
		assert_eq!(ztest(), C::LessEqual);
		let st = GsState::decode(&GsRegs { test: Some(0x5360b), ..Default::default() });
		assert_eq!(depth_compare(st.test.ztst), C::LessEqual);
	}
}
