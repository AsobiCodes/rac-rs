//! EFFECTS LAYER (stage 3): every owned draw-queue callback whose packets are PATH2 DIRECT GIF only (the EE built the
//! GS packets itself, no VU1 program) is drawn in the mirror from its own GS data, with no per-effect code.
//!
//! Source of each part [C]:
//! - Ownership and the packet words: `efx_own` (record-only probes; `walk` re-reads the callback's DMA range from RAM).
//! - GS state the packet sets itself: PRIM (incl. GIF tag PRE), RGBAQ, ST, UV, TEX0, CLAMP, ALPHA (both contexts),
//!   PRMODECONT / PRMODE. Everything the mirror needs comes from these.
//! - Screen -> view: the projection block the callbacks project with, read at the queue runner's entry together with
//!   the camera (`efx_own::view_words`): pixel x = cx + sx * vx / vz, y = cy - sy * vy / vz, GS z = za + zb / vz
//!   (vz in 1/1024 m; camera.md projection table). cx / cy are the projection's own screen offset (vector 0x16cfe0 =
//!   2048, 2048 on every level), the same numbers XYOFFSET centres the frame on, so XYOFFSET is not needed. Depth comes
//!   from Z (Q = near / vz only when Z is outside the projection's range).
//! - Texture: TEX0 -> fx slot = the record of the slot table 0x16d580 (+ level shift) whose cached TEX0 (+0, written by
//!   fx_tex_upload Rilgar FUN_00227e80 / Novalis FUN_0021ad98) equals the packet's TEX0; rac-rs's fx texture of that
//!   index (frontend::fx_texture, PSMT8 + CSM1 palette) is the same picture. CLAMP 5 -> `FX_CLAMP` sampler.
//! - Blend: ALPHA 0x48 (Cs*As + Cd, additive) and 0x44 (Cs*As + Cd*(1-As)) map to the mirror's two particle pipelines;
//!   both depth-test against the mirror's scene without writing depth (TEST / ZBUF are not in the owners' packets: the
//!   GS keeps the world pass's ZTST GEQUAL with ZMSK for blended effects [I, stage 4: read them from the gsfull replay]).
//! Not drawn (counted in `Stats.skip`): callbacks an older port claims (`claimed`), VU1 owners, prims without a slot
//! texture, other blends, lines / points, untextured prims.
use super::efx_own::{self, Own};
use crate::particles::{WorldQuad, WorldStrip, FX_CLAMP};
use ee_oracle::Machine;
use crate::retail::disc::va;

/// fx slot table (16-byte records, +0 = the slot's TEX0 once uploaded) at Novalis's address; per level + LEVEL_SHIFT.
const SLOT_TABLE: u32 = 0x16d580;
/// Slots searched (Rilgar's level index holds fewer fx textures than this).
const SLOTS: u32 = 0x100;

/// The main draw environment packet [C]: GIF tag at 0x13cf10, then 10 A+D qwords (FRAME_1/2, ZBUF_1/2, XYOFFSET_1/2,
/// SCISSOR_1/2, FBA_1/2) that the display setup FUN_001f33e8 (boot ELF, every level) writes. Every VU1 program's GS
/// header sets these same values (Rilgar / Pokitaru owners: FRAME 0x80080, ZBUF 0x10000f0, XYOFFSET 0x7000 / 0x7200,
/// SCISSOR 0..0x1ff x 0..0x1bf), i.e. it draws into the world's frame.
const DRAW_ENV: u32 = 0x13cf20;

/// Skip reasons: 0 claimed, 1 VU1 owner without its program's GS packets (efx_vu found no match in the field's jobs), 2 record not exact / walk failed, 3 no slot texture, 4 other blend or
/// ABE off, 5 line / point, 6 untextured, 7 unproject failed, 8 owner sets frame / depth / offset / scissor state
/// different from the main draw environment (`DRAW_ENV`: a render-to-texture or screen pass, not a world effect), 9 depth /
/// alpha-test state the mirror's particle pipelines cannot draw exactly (`depth_ok`).
#[derive(Debug, Default, Clone)]
pub struct Stats {
	pub owners: u32,
	pub tris: u32,
	/// VU1 owners drawn from their program's GS packets (stage 5, efx_vu).
	pub vu: u32,
	pub skip: [u32; 10],
	pub fns: Vec<u32>,
	/// Target-register writes (owner fn, GS register, value) of the owners skipped for reason 8 (distinct).
	pub tgt: Vec<(u32, u8, u64)>,
	/// Distinct (owner fn, TEST, ZBUF, TEX1) of the drawn-or-depth-skipped prims (None = not set by the owner's stream).
	pub zs: Vec<(u32, Option<u64>, Option<u64>, Option<u64>)>,
	/// Drawn prims whose retail TEST / ZBUF writes depth for some pixels (`depth_ok`).
	pub zw: u32,
	/// Prims drawn through the FIX-blend strip pass (ALPHA 0x64 / 0x68).
	pub fix_tris: u32,
	/// Distinct (owner fn, what, word) of the prims the layer cannot draw as the GS does: "blend" = ALPHA word of a skipped prim (reason 4,
	/// ABE off = 0), "mag" = TEX1 with MMAG nearest of a drawn prim (GS_AUDIT.md run 9).
	pub odd: Vec<(u32, &'static str, u64)>,
}

/// A prim's TEST / ZBUF against the mirror's particle pipelines [C: GS manual TEST / ZBUF]: (drawable, writes Z).
/// Drawable = depth test GEQUAL / GREATER (ZTE 1, ZTST 2 / 3: never through walls) and every pixel's colour written
/// (no destination alpha test; alpha test off, ALWAYS, or a failing pixel still writes the frame: AFAIL FB_ONLY /
/// RGB_ONLY). Writes Z = some pixel writes depth (ZMSK 0 and the alpha test can pass): the mirror draws every
/// particle without a depth write (`Stats.zw` counts these prims). `zbuf` None = the main draw environment's ZBUF
/// (`DRAW_ENV`, ZMSK 0), the value every owner that writes ZBUF writes. `test` None (not in the owner's stream: the GS
/// keeps the previous pass's TEST) is the stage-4 rule and stays drawn; the sweep logs which owners rely on it.
fn depth_ok(test: Option<u64>, zbuf: Option<u64>) -> (bool, bool) {
	let Some(t) = test else { return (true, false) };
	let (ate, atst, afail, date, zte, ztst) = (t & 1 != 0, (t >> 1) & 7, (t >> 12) & 3, (t >> 14) & 1 != 0, (t >> 16) & 1 != 0, (t >> 17) & 3);
	let zmsk = zbuf.unwrap_or(0x100_00f0) >> 32 & 1 != 0;
	let colour_all = !ate || atst == 1 || afail == 1 || afail == 3;
	let z_never = zmsk || (ate && atst == 0 && (afail == 1 || afail == 3));
	(!date && zte && (ztst == 2 || ztst == 3) && colour_all, !z_never)
}

/// The retail view of one queue run (see `efx_own::view_words`).
#[derive(Clone, Copy)]
struct View {
	eye: [f32; 3],
	f: [f32; 3],
	r: [f32; 3],
	u: [f32; 3],
	near: f32,
	sx: f32,
	sy: f32,
	cx: f32,
	cy: f32,
	za: f32,
	zb: f32,
}

impl View {
	fn new(w: &[u32; 20]) -> Option<View> {
		let f = |i: usize| f32::from_bits(w[i]);
		let v3 = |i: usize| [f(i), f(i + 1), f(i + 2)];
		let left = v3(6);
		let near = f(12);
		let v = View { eye: v3(0), f: v3(3), r: left.map(|x| -x), u: v3(9), near, sx: f(13) * near, sy: f(14) * near, cx: f(17), cy: f(18), za: f(19) + near * f(15), zb: near * f(16) };
		(v.sx > 0.0 && v.sy > 0.0 && v.zb > 0.0 && near > 0.0).then_some(v)
	}

	/// GS screen vertex (pixels in primitive space, GS z, Q) -> world point (m).
	fn world(&self, x: f32, y: f32, z: f32, q: f32) -> Option<[f32; 3]> {
		let dz = z - self.za;
		let vz = if dz > 0.0 && z <= 16_776_656.0 { self.zb / dz } else if q > 0.0 { self.near / q } else { return None };
		let d = vz / 1024.0;
		let (a, b) = ((x - self.cx) / self.sx * d, -(y - self.cy) / self.sy * d);
		Some([0, 1, 2].map(|k| self.eye[k] + self.f[k] * d + self.r[k] * a + self.u[k] * b))
	}
}

#[derive(Clone, Copy, Default)]
struct V {
	x: f32,
	y: f32,
	z: f32,
	rgba: [u8; 4],
	s: f32,
	t: f32,
	q: f32,
	u: f32,
	v: f32,
	/// Fog F (XYZF2 / XYZF3 bits 56-63; 0xff = no fog for XYZ2 / XYZ3).
	f: u8,
}

/// One drawn GS primitive: triangle (3 vertices) or sprite (2), with the context registers at its kick.
struct Prim {
	sprite: bool,
	v: [V; 3],
	attr: u64,
	tex0: u64,
	clamp: u64,
	alpha: u64,
	/// TEST / ZBUF / TEX1 of the prim's context as the owner's own stream set them (None = not set by the stream).
	test: Option<u64>,
	zbuf: Option<u64>,
	tex1: Option<u64>,
	/// FOGCOL (reg 0x3d, 24 bits) when the owner's stream wrote it (None = the GS keeps the earlier pass's value: the level fog colour).
	fogcol: Option<u32>,
}

/// The GS register state a DIRECT stream sets, and the vertex queue.
struct Gs {
	prim: u64,
	prmode: u64,
	prmodecont: bool,
	rgba: [u8; 4],
	q: f32,
	st: [f32; 2],
	uv: [f32; 2],
	tex0: [u64; 2],
	clamp: [u64; 2],
	alpha: [u64; 2],
	test: [Option<u64>; 2],
	zbuf: [Option<u64>; 2],
	tex1: [Option<u64>; 2],
	fogcol: Option<u32>,
	vq: [V; 3],
	n: usize,
	lines: u32,
	/// The stream's writes to XYOFFSET, SCISSOR, FRAME or ZBUF (either context): (register, value), distinct.
	target: Vec<(u8, u64)>,
	/// The triangles the stream queued with ADC set (not drawn), kept with `keep_adc` (`widescreen_refill`).
	adc: Vec<Prim>,
	keep_adc: bool,
}

impl Gs {
	fn new() -> Gs {
		Gs { prim: 0, prmode: 0, prmodecont: true, rgba: [0x80; 4], q: 1.0, st: [0.0; 2], uv: [0.0; 2], tex0: [0; 2], clamp: [0; 2], alpha: [0; 2], test: [None; 2], zbuf: [None; 2], tex1: [None; 2], fogcol: None, vq: [V::default(); 3], n: 0, lines: 0, target: Vec::new(), adc: Vec::new(), keep_adc: false }
	}

	fn set_prim(&mut self, v: u64) {
		self.prim = v & 0x7ff;
		self.n = 0;
	}

	/// PRIM attributes (bits 3-10): PRIM itself, or PRMODE when PRMODECONT = 0.
	fn attr(&self) -> u64 {
		if self.prmodecont { self.prim } else { (self.prim & 7) | (self.prmode & 0x7f8) }
	}

	fn ad(&mut self, a: u8, d: u64, out: &mut Vec<Prim>) {
		match a {
			0x00 => self.set_prim(d),
			0x01 => {
				self.rgba = [d as u8, (d >> 8) as u8, (d >> 16) as u8, (d >> 24) as u8];
				self.q = f32::from_bits((d >> 32) as u32);
			}
			0x02 => self.st = [f32::from_bits(d as u32), f32::from_bits((d >> 32) as u32)],
			0x03 => self.uv = [(d & 0x3fff) as f32 / 16.0, ((d >> 16) & 0x3fff) as f32 / 16.0],
			0x04 | 0x0c => self.vertex((d & 0xffff) as u32, ((d >> 16) & 0xffff) as u32, ((d >> 32) & 0xff_ffff) as u32, (d >> 56) as u8, a == 0x04, out),
			0x05 | 0x0d => self.vertex((d & 0xffff) as u32, ((d >> 16) & 0xffff) as u32, (d >> 32) as u32, 0xff, a == 0x05, out),
			0x06 | 0x07 => self.tex0[(a - 6) as usize] = d,
			0x08 | 0x09 => self.clamp[(a - 8) as usize] = d,
			0x42 | 0x43 => self.alpha[(a - 0x42) as usize] = d,
			0x47 | 0x48 => self.test[(a - 0x47) as usize] = Some(d),
			0x14 | 0x15 => self.tex1[(a - 0x14) as usize] = Some(d),
			0x3d => self.fogcol = Some(d as u32 & 0xff_ffff),
			0x1a => self.prmodecont = d & 1 != 0,
			0x1b => self.prmode = d,
			0x18 | 0x19 | 0x40 | 0x41 | 0x4c | 0x4d | 0x4e | 0x4f => {
				if a & 0xfe == 0x4e {
					self.zbuf[(a - 0x4e) as usize] = Some(d);
				}
				if !self.target.contains(&(a, d)) {
					self.target.push((a, d));
				}
			}
			_ => {}
		}
	}

	fn vertex(&mut self, x: u32, y: u32, z: u32, f: u8, kick: bool, out: &mut Vec<Prim>) {
		let v = V { x: x as f32 / 16.0, y: y as f32 / 16.0, z: z as f32, rgba: self.rgba, s: self.st[0], t: self.st[1], q: self.q, u: self.uv[0], v: self.uv[1], f };
		let ty = self.prim & 7;
		let need = match ty {
			0 => 1,
			1 | 2 | 6 => 2,
			_ => 3,
		};
		self.vq[self.n] = v;
		self.n += 1;
		if self.n < need {
			return;
		}
		let attr = self.attr();
		let ctx = (attr & 0x200 != 0) as usize;
		if kick {
			match ty {
				3..=6 => out.push(Prim { sprite: ty == 6, v: self.vq, attr, tex0: self.tex0[ctx], clamp: self.clamp[ctx], alpha: self.alpha[ctx], test: self.test[ctx], zbuf: self.zbuf[ctx], tex1: self.tex1[ctx], fogcol: self.fogcol }),
				_ => self.lines += 1,
			}
		} else if (3..=5).contains(&ty) && self.keep_adc {
			self.adc.push(Prim { sprite: false, v: self.vq, attr, tex0: self.tex0[ctx], clamp: self.clamp[ctx], alpha: self.alpha[ctx], test: self.test[ctx], zbuf: self.zbuf[ctx], tex1: self.tex1[ctx], fogcol: self.fogcol });
		}
		match ty {
			2 => {
				self.vq[0] = self.vq[1];
				self.n = 1;
			}
			4 => {
				self.vq[0] = self.vq[1];
				self.vq[1] = self.vq[2];
				self.n = 2;
			}
			5 => {
				self.vq[1] = self.vq[2];
				self.n = 2;
			}
			_ => self.n = 0,
		}
	}

	/// One PACKED register write (GIF register descriptor `r`, 128-bit data).
	fn packed(&mut self, r: u8, d: u128, out: &mut Vec<Prim>) {
		let lo = d as u64;
		match r {
			0 => self.set_prim(lo & 0x7ff),
			1 => self.rgba = [d as u8, (d >> 32) as u8, (d >> 64) as u8, (d >> 96) as u8],
			2 => {
				self.st = [f32::from_bits(d as u32), f32::from_bits((d >> 32) as u32)];
				self.q = f32::from_bits((d >> 64) as u32);
			}
			3 => self.uv = [(d & 0x3fff) as f32 / 16.0, ((d >> 32) & 0x3fff) as f32 / 16.0],
			4 | 0xc => {
				let kick = r == 4 && (d >> 111) & 1 == 0;
				self.vertex((d & 0xffff) as u32, ((d >> 32) & 0xffff) as u32, ((d >> 68) & 0xff_ffff) as u32, ((d >> 100) & 0xff) as u8, kick, out);
			}
			5 | 0xd => {
				let kick = r == 5 && (d >> 111) & 1 == 0;
				self.vertex((d & 0xffff) as u32, ((d >> 32) & 0xffff) as u32, (d >> 64) as u32, 0xff, kick, out);
			}
			6..=9 => self.ad(r, lo, out),
			0xe => self.ad(((d >> 64) & 0xff) as u8, lo, out),
			_ => {}
		}
	}

	/// A run of GIF packets (qwords: tag + data, as DIRECT carries them or a VU1 program XGKICKs them).
	pub(crate) fn gif(&mut self, q: &[u128], out: &mut Vec<Prim>) {
		let mut i = 0usize;
		while i < q.len() {
			let t = q[i];
			i += 1;
			let nloop = (t & 0x7fff) as usize;
			let flg = ((t >> 58) & 3) as usize;
			let nreg = match ((t >> 60) & 15) as usize { 0 => 16, n => n };
			if flg != 2 && (t >> 46) & 1 != 0 {
				self.set_prim(((t >> 47) & 0x7ff) as u64);
			}
			let regs: Vec<u8> = (0..nreg).map(|k| ((t >> (64 + 4 * k)) & 15) as u8).collect();
			match flg {
				0 => {
					'l: for _ in 0..nloop {
						for &r in &regs {
							let Some(&d) = q.get(i) else {
								break 'l;
							};
							i += 1;
							self.packed(r, d, out);
						}
					}
				}
				1 => {
					let n = nloop * nreg;
					for k in 0..n {
						let Some(&d) = q.get(i + k / 2) else {
							break;
						};
						let r = regs[k % nreg];
						if r < 0xe {
							self.ad(r, (d >> (64 * (k % 2))) as u64, out);
						}
					}
					i += n.div_ceil(2);
				}
				_ => i += nloop,
			}
		}
	}

	/// One VIF stream (the callback's packet words): every DIRECT / DIRECTHL payload's GIF packets.
	fn run(&mut self, words: &[u32], out: &mut Vec<Prim>) {
		let mut i = 0usize;
		while i < words.len() {
			let w = words[i];
			let cmd = (w >> 24) & 0x7f;
			let imm = (w & 0xffff) as usize;
			i += 1;
			match cmd {
				0x50 | 0x51 => {
					let end = (i + if imm == 0 { 0x10000 } else { imm } * 4).min(words.len());
					let qs: Vec<u128> = words[i..end].chunks_exact(4).map(|c| (0..4).fold(0u128, |a, k| a | (c[k] as u128) << (32 * k))).collect();
					self.gif(&qs, out);
					i = end;
				}
				0x20 => i += 1,
				0x30 | 0x31 => i += 4,
				0x4a => i += (if (w >> 16) & 0xff == 0 { 256 } else { (w >> 16) & 0xff }) as usize * 2,
				0x60..=0x7f => {
					let (vn, vl) = ((cmd >> 2) & 3, cmd & 3);
					let num = match (w >> 16) & 0xff { 0 => 256, n => n };
					i += (((32 >> vl) * (vn + 1) * num) as usize).div_ceil(32);
				}
				_ => {}
			}
		}
	}
}

/// The water patch drivers (level, queue callback) [C: decomp/notes/draw_producers.md, 10-line callbacks whose only call is
/// the level's patch draw]: crate::water draws the patches from the level data, so the layer leaves them out (their VU1
/// output was 18 qwords without a prim in every sweep state, efx6l-*.err, but a visible patch would be drawn twice).
const WATER_DRIVERS: [(u32, u32); 6] = [(1, 0x2fc9f0), (5, 0x30d008), (7, 0x30c648), (11, 0x30e038), (12, 0x2bf078), (13, 0x309d88)];
/// `WATER_DRIVERS` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const WATER_DRIVERS_USA: [(u32, u32); 6] = [(1, 0x2fd0c0), (5, 0x30d678), (7, 0x30cd20), (11, 0x30e708), (12, 0x2bf748), (13, 0x30a460)];

/// Whether an older one-off port already draws callback `f` on this level (the claimed set): every module that turns
/// queue callbacks into mirror geometry answers for its own tables.
pub fn claimed(m: &Machine, level: u32, f: u32) -> bool {
	(*crate::retail::disc::pick(&WATER_DRIVERS, &WATER_DRIVERS_USA)).contains(&(level, f)) || super::queue_b::claims(m, level, f) || super::queue_q1::claims(level, f) || super::queue_story::claims(level, f) || super::queue_fxa::claims(level, f) || super::claims(level, f)
}

/// The fx slot whose cached TEX0 is `tex0` (exact 64-bit match).
fn slot(slots: &[u64], tex0: u64, cache: &mut Vec<(u64, Option<usize>)>) -> Option<usize> {
	if let Some(&(_, s)) = cache.iter().find(|c| c.0 == tex0) {
		return s;
	}
	let s = (0..SLOTS as usize).find(|&n| tex0 != 0 && slots.get(n) == Some(&tex0));
	cache.push((tex0, s));
	s
}

/// GS fog (PRIM FGE = 1) of an effect prim, carried in the high bits of `WorldQuad.tex` / `WorldStrip.tex` (the texture index
/// and FX_CLAMP / FX_NOMIP use bits 0..21): bit 22 `FOG_ON` (F per vertex follows), bit 23 `FOG_OFF` (FIX strips with FGE 0: the shared
/// strip shader would otherwise fog them by distance), bits 24..47 the fog bytes F of vertices 0..2 (quads only: a strip carries F in its
/// vertex colour alpha, which its shader ignores), bits 48..63 the FOGCOL (reg 0x3d) as an index into an intern table (0xffff = the
/// GS kept the level's fog colour). `viewer/gpu.rs` turns it into the vertex `light` = [F, FOGCOL word, 0, -5]; the shaders
/// (`fog_class` l.w = -5 gives the linear-interpolated F, fs_part / fs_part_fix apply `c = (F * C + (255 - F) * FOGCOL) >> 8`).
pub const FOG_ON: usize = 1 << 22;
pub const FOG_OFF: usize = 1 << 23;
/// Every bit of `tex` that is not the texture index or a texture-variant flag.
pub const FOG_BITS: usize = !((1usize << 22) - 1);
const FOG_COL_NONE: usize = 0xffff;

static FOGCOLS: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

/// The `tex` word of an effect prim drawn with GS fog: `f` = the fog bytes of vertices 0..2 (a quad's fourth corner repeats the
/// third), `fogcol` = the prim's FOGCOL if its owner wrote one.
fn fog_tex(tex: usize, f: [u8; 3], fogcol: Option<u32>) -> usize {
	let ci = fogcol.map_or(FOG_COL_NONE, |c| {
		let mut t = FOGCOLS.lock().unwrap();
		let i = t.iter().position(|&x| x == c).unwrap_or_else(|| {
			t.push(c);
			t.len() - 1
		});
		assert!(i < FOG_COL_NONE, "FOGCOL table full");
		i
	});
	tex | FOG_ON | (f[0] as usize) << 24 | (f[1] as usize) << 32 | (f[2] as usize) << 40 | ci << 48
}

/// `tex` of an FGE 0 FIX strip: not fogged at all.
fn fog_off(tex: usize) -> usize {
	tex | FOG_OFF
}

/// The texture key (index + variant flags) of a `tex` word.
pub fn plain_tex(tex: usize) -> usize {
	tex & !FOG_BITS
}

/// The vertex `light` of corner `k` of a quad (or of a strip vertex whose F is `strip_f`) drawn with `tex`: [F, FOGCOL word
/// (r | g << 8 | b << 16, -1 = the level fog colour), 0, -5] for GS fog, [0, 0, 0, -6] for an FGE 0 FIX strip (no fog), zeros otherwise.
pub fn fog_light(tex: usize, k: usize, strip_f: f32) -> [f32; 4] {
	if tex & FOG_OFF != 0 {
		return [0.0, 0.0, 0.0, -6.0];
	}
	if tex & FOG_ON == 0 {
		return [0.0; 4];
	}
	let ci = tex >> 48;
	let col = if ci == FOG_COL_NONE { -1.0 } else { FOGCOLS.lock().unwrap().get(ci).map_or(-1.0, |&c| c as f32) };
	let f = if strip_f >= 0.0 { strip_f } else { ((tex >> (24 + 8 * k.min(2))) & 0xff) as f32 };
	[f, col, 0.0, -5.0]
}

fn colour(c: [u8; 4]) -> [f32; 4] {
	c.map(|x| x as f32 / 128.0)
}

/// RAC_EFX_DUMP=<hex fn>: print that owner's GS prims (first 48 per field) to stderr (debug only).
fn dump_fn() -> Option<u32> {
	static F: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
	*F.get_or_init(|| std::env::var("RAC_EFX_DUMP").ok().and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok()))
}

/// [PC] Widescreen: Pokitaru's waterfall sheets (fx texture 42, level 11) whole where the wider view shows more than retail's 4:3
/// screen. Their owner 0x31e260 queues the triangles its VU1 clip test rejects with ADC set (not drawn); those beside the 4:3 screen
/// are in the widescreen margins, where the sheet showed dark triangular holes that came and went as the camera turned
/// [C: ADC census of the fall owners (lane F1 log) at fullgame1 0447, 2026-10-19: owner 0x31e260 kicked only triangles inside the 4:3 screen in a turned
/// view; owner 0x31d480 kicks most of its off-screen ones]. Candidates: a skipped triangle in front of the eye (Q > 0 at every
/// corner: the others are behind it), inside the guard band (|x| <= 1024, |y| <= 896 pixels from the centre) with a corner off the
/// 4:3 screen (|x| > 256 or |y| > 224); the ones inside the 4:3 screen stay out as retail leaves them. A rejected batch can carry
/// stale vertices (near the eye, in strip patterns: huge false triangles at the user's falls recording, field 390), so a candidate
/// goes out flagged `FX_REFILL` and only the waterfall pass draws it, when its corners are a sheet triangle seen kicked before
/// (viewer/waterfx.rs `SheetMem::refill`); every other path skips the flag.
pub const FX_REFILL: usize = 1 << 19;
const REFILL_LEVEL: u32 = 11;
const REFILL_TEX: usize = 42;

fn widescreen_refill(pr: &Prim, view: &View) -> bool {
	if pr.sprite {
		return false;
	}
	let d = pr.v.map(|v| (v.x - view.cx, v.y - view.cy, v.q));
	d.iter().all(|&(x, y, q)| q > 0.0 && x.abs() <= 1024.0 && y.abs() <= 896.0) && d.iter().any(|&(x, y, _)| x.abs() > 256.0 || y.abs() > 224.0)
}

/// RAC_EFX_DRAW=0 leaves the layer out (A/B pictures); default on.
pub fn enabled() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| !std::env::var("RAC_EFX_DRAW").is_ok_and(|v| v == "0"))
}

/// Everything `build_in` reads from the machine at a field's end, copied (a few KB) so the build can run after the
/// worker has gone on to the next field: per owner whether an older port claims it and its packet walk (None when claimed),
/// the draw environment's ten registers and the fx slot table's TEX0 words (`slot`).
#[derive(Debug)]
pub struct BuildIn {
	own: Vec<Own>,
	pub pkts: Vec<Option<efx_own::Pkt>>,
	env: Vec<(u8, u64)>,
	slots: Vec<u64>,
	level: u32,
}

/// `BuildIn` of the field the machine has just ended (reads RAM).
pub fn capture(m: &Machine, level: u32, own: &[Own]) -> BuildIn {
	let t_cap = std::time::Instant::now();
	let r = capture_inner(m, level, own);
	super::efx_vu::sp_add(0, t_cap.elapsed().as_nanos() as u64);
	r
}

fn capture_inner(m: &Machine, level: u32, own: &[Own]) -> BuildIn {
	let claimed: Vec<bool> = own.iter().map(|o| claimed(m, level, o.fnp)).collect();
	let pkts = own.iter().zip(&claimed).map(|(o, &c)| (!c).then(|| efx_own::walk(&m.cpu, o))).collect();
	let env = (0..10).map(|k| DRAW_ENV + 16 * k).map(|a| (m.rd32(a + 8) as u8, m.rd32(a) as u64 | (m.rd32(a + 4) as u64) << 32)).collect();
	let tbl = va(SLOT_TABLE.wrapping_add_signed(*super::LEVEL_SHIFT.get(level as usize).unwrap_or(&0)));
	let slots = (0..SLOTS).map(|n| tbl + 16 * n).map(|a| m.rd32(a) as u64 | (m.rd32(a + 4) as u64) << 32).collect();
	BuildIn { own: own.to_vec(), pkts, env, slots, level }
}

/// `build` from a captured `BuildIn` (no machine access: any thread).
pub fn build_in(b: &BuildIn, vu: &super::efx_vu::VuOut, out: &mut Vec<WorldQuad>, pools: &mut Vec<WorldStrip>) -> Stats {
	let own = &b.own[..];
	let mut st = Stats::default();
	let mut cache = Vec::new();
	let mut run: Option<Run> = None;
	for (k, o) in own.iter().enumerate() {
		flush(&mut run, pools);
		let Some(p) = &b.pkts[k] else {
			st.skip[0] += 1;
			continue;
		};
		let view = View::new(&o.view);
		let (Some(view), true) = (view, p.ok) else {
			st.skip[2] += 1;
			continue;
		};
		let (vc, whole) = efx_own::vif_census(&p.words);
		let vu1 = vc[0] + vc[1] + vc[2] != 0;
		// VU1 owner: the GS packets its program sent (efx_vu); without them (not found in the field's jobs) skipped.
		let vq = if vu1 { vu.iter().find(|x| x.0 == k).map(|x| &x.1) } else { None };
		if (vu1 && vq.is_none()) || !whole {
			st.skip[1] += 1;
			continue;
		}
		if !vu1 && vc[3] == 0 {
			continue;
		}
		if vu1 {
			st.vu += 1;
		}
		st.owners += 1;
		if !st.fns.contains(&o.fnp) {
			st.fns.push(o.fnp);
		}
		let mut g = Gs::new();
		g.keep_adc = b.level == REFILL_LEVEL;
		let mut refill = Vec::new();
		let mut prims = Vec::new();
		match vq {
			Some(q) => g.gif(q, &mut prims),
			None => g.run(&p.words, &mut prims),
		}
		st.skip[5] += g.lines;
		g.target.retain(|t| !b.env.contains(t));
		if !g.target.is_empty() {
			st.skip[8] += 1;
			for &(r, d) in &g.target {
				if !st.tgt.contains(&(o.fnp, r, d)) {
					st.tgt.push((o.fnp, r, d));
				}
			}
			continue;
		}
		if g.keep_adc {
			for pr in std::mem::take(&mut g.adc) {
				if widescreen_refill(&pr, &view) && efx_own::slot_of(o, pr.tex0).unwrap_or_else(|| slot(&b.slots, pr.tex0, &mut cache)).is_some_and(|t| plain_tex(t) == REFILL_TEX) {
					refill.push(pr);
				}
			}
		}
		let kicked = prims.len();
		prims.extend(refill.drain(..));
		let dump = dump_fn() == Some(o.fnp);
		if dump {
			eprintln!("efxdump fn {:x} c0 {:x} c1 {:x} prims {} words {}", o.fnp, o.c0, o.c1, prims.len(), p.words.len());
			if let (Some(q), Some(path)) = (vq, std::env::var_os("RAC_EFX_DUMPFILE")) {
				let b: Vec<u8> = q.iter().flat_map(|x| x.to_le_bytes()).collect();
				let _ = std::fs::write(path, b);
			}
			if let Some(q) = vq {
				// The first GIF tags of the program's output (debug).
				let (mut i, mut n) = (0usize, 0);
				while i < q.len() && n < 12 {
					let t = q[i];
					let (nloop, flg, nreg) = ((t & 0x7fff) as usize, ((t >> 58) & 3) as usize, match ((t >> 60) & 15) as usize { 0 => 16, x => x });
					eprintln!("efxdump  tag @{i} {t:032x} nloop {nloop} flg {flg} nreg {nreg} data {:032x?}", &q[(i + 1).min(q.len())..(i + 7).min(q.len())]);
					i += 1 + if flg == 0 { nloop * nreg } else if flg == 1 { (nloop * nreg).div_ceil(2) } else { nloop };
					n += 1;
				}
			}
		}
		for (pi, pr) in prims.into_iter().enumerate() {
			let attr = pr.attr;
			let z = (o.fnp, pr.test, pr.zbuf, pr.tex1);
			if !st.zs.contains(&z) {
				st.zs.push(z);
			}
			let (dok, zw) = depth_ok(pr.test, pr.zbuf);
			if !dok {
				if crate::viewer::gs_state::enabled() {
					crate::viewer::gs_state::note("efx", "prim.skipped_by_test", pr.test.unwrap_or(0) as u32 & 0x3ffff);
				}
				st.skip[9] += 1;
				continue;
			}
			st.zw += zw as u32;
			if dump && pi < 48 {
				let vs: Vec<String> = pr.v.iter().map(|v| format!("({:.1} {:.1} z{:x} f{:x} q{:.4} c{:02x?} st{:.3},{:.3} uv{:.1},{:.1})", v.x, v.y, v.z as u32, v.f, v.q, v.rgba, v.s, v.t, v.u, v.v)).collect();
				eprintln!("efxdump  #{pi} spr {} attr {:x} alpha {:x} tex0 {:x} clamp {:x} slot {:?} {}", pr.sprite, attr, pr.alpha, pr.tex0, pr.clamp, efx_own::slot_of(o, pr.tex0).unwrap_or_else(|| slot(&b.slots, pr.tex0, &mut cache)), vs.join(" "));
			}
			if attr & 0x10 == 0 {
				st.skip[6] += 1;
				continue;
			}
			if crate::viewer::gs_state::enabled() {
				use crate::viewer::gs_state as gs;
				let regs = gs::GsRegs { prim: Some(attr), tex0: Some(pr.tex0), tex1: pr.tex1.or(Some(0x20)), clamp: Some(if pr.clamp & 0xf == 5 { 0 } else { pr.clamp }), test: pr.test, alpha: Some(if pr.alpha == 0 { 0x44 } else { pr.alpha }), zbuf: pr.zbuf, fogcol: pr.fogcol.map(u64::from), ..Default::default() };
				// A TEX1 / ALPHA the owner stream never writes is inherited from the main pass (census: TEX1 MMAG linear, ALPHA 0x44), not a state of its own.
				gs::guard("efx", &gs::GsState::decode(&regs), &gs::efx_support());
				// The vertex fog byte matters only with PRIM FGE (`fog_tex`); a prim with FGE 0 draws unfogged as the GS does.
			}
			if pr.tex1.is_some_and(|t| t >> 5 & 1 == 0) && attr & 0x10 != 0 {
				let o = (o.fnp, "mag", pr.tex1.unwrap_or(0));
				if !st.odd.contains(&o) {
					st.odd.push(o);
				}
			}
			// ALPHA low byte: 0x48 Cs*As + Cd, 0x44 Cs*As + Cd*(1-As); FIX blends (C = 2): 0x64 (Cs-Cd)*FIX + Cd, 0x68 Cs*FIX + Cd.
			let (additive, fix) = match (attr & 0x40 != 0, pr.alpha & 0xff) {
				(true, 0x48) => (true, None),
				(true, 0x44) => (false, None),
				(true, 0x64) => (false, Some(((((pr.alpha >> 32) & 0xff) as u8).min(0x80), false))),
				(true, 0x68) => (true, Some(((((pr.alpha >> 32) & 0xff) as u8).min(0x80), true))),
				_ => {
					st.skip[4] += 1;
					let o = (o.fnp, "blend", if attr & 0x40 != 0 { pr.alpha } else { 0 });
					if !st.odd.contains(&o) {
						st.odd.push(o);
					}
					continue;
				}
			};
			let Some(tex) = efx_own::slot_of(o, pr.tex0).unwrap_or_else(|| slot(&b.slots, pr.tex0, &mut cache)) else {
				st.skip[3] += 1;
				continue;
			};
			let tex = if pr.clamp & 0xf == 5 { tex | FX_CLAMP } else { tex };
			let fst = attr & 0x100 != 0;
			let (tw, th) = ((1u32 << ((pr.tex0 >> 26) & 15)) as f32, (1u32 << ((pr.tex0 >> 30) & 15)) as f32);
			let uv = |v: &V| if fst { [v.u / tw, v.v / th] } else if v.q != 0.0 { [v.s / v.q, v.t / v.q] } else { [v.s, v.t] };
			let flat = attr & 8 == 0;
			// PRIM FGE: the GS fogs the texture-shaded colour per vertex F (`fog_tex`).
			let fogged = attr & 0x20 != 0;
			if pi >= kicked && (fix.is_some() || pr.sprite) {
				continue;
			}
			if let Some((fix, add)) = fix {
				// FIX blends go to the strip pass (texture and vertex alpha ignored): consecutive prims of one state share a strip.
				let tri: Option<[([f32; 3], [f32; 2], [f32; 4]); 3]> = if pr.sprite {
					None
				} else {
					let pos = pr.v.map(|v| view.world(v.x, v.y, v.z, v.q));
					match pos {
						[Some(p0), Some(p1), Some(p2)] => {
							let mut c = if flat { [colour(pr.v[2].rgba); 3] } else { pr.v.map(|v| colour(v.rgba)) };
							if fogged {
								// The strip shader ignores vertex alpha: it carries the fog byte F.
								for k in 0..3 {
									c[k][3] = pr.v[k].f as f32;
								}
							}
							Some([(p0, uv(&pr.v[0]), c[0]), (p1, uv(&pr.v[1]), c[1]), (p2, uv(&pr.v[2]), c[2])])
						}
						_ => {
							st.skip[7] += 1;
							continue;
						}
					}
				};
				let tris: Vec<[([f32; 3], [f32; 2], [f32; 4]); 3]> = match tri {
					Some(t) => vec![t],
					None => {
						let (a, b) = (pr.v[0], pr.v[1]);
						let mut c = colour(b.rgba);
						if fogged {
							c[3] = b.f as f32;
						}
						let (ua, ub) = (uv(&a), uv(&b));
						let corner = |x: f32, y: f32| view.world(x, y, b.z, b.q);
						let [Some(p0), Some(p1), Some(p2), Some(p3)] = [corner(a.x, a.y), corner(b.x, a.y), corner(b.x, b.y), corner(a.x, b.y)] else {
							st.skip[7] += 1;
							continue;
						};
						let (v0, v1, v2, v3) = ((p0, ua, c), (p1, [ub[0], ua[1]], c), (p2, ub, c), (p3, [ua[0], ub[1]], c));
						vec![[v0, v1, v3], [v1, v3, v2]]
					}
				};
				let key = (if fogged { fog_tex(tex, [0; 3], pr.fogcol) } else { fog_off(tex) }, fix, add);
				if run.as_ref().is_some_and(|r| r.key != key) {
					flush(&mut run, pools);
				}
				let r = run.get_or_insert_with(|| Run { key, pos: Vec::new(), uv: Vec::new(), colour: Vec::new() });
				for t in tris {
					// Triangles join into one strip through degenerate triangles (repeat the last and the first vertex).
					if let Some(&last) = r.pos.last() {
						r.pos.extend([last, t[0].0]);
						let (lu, lc) = (*r.uv.last().unwrap(), *r.colour.last().unwrap());
						r.uv.extend([lu, t[0].1]);
						r.colour.extend([lc, t[0].2]);
					}
					for (p, u, c) in t {
						r.pos.push(p);
						r.uv.push(u);
						r.colour.push(c);
					}
					st.tris += 1;
				}
				st.fix_tris += 1;
				continue;
			}
			if pr.sprite {
				let (a, b) = (pr.v[0], pr.v[1]);
				let c = colour(b.rgba);
				let corner = |x: f32, y: f32| view.world(x, y, b.z, b.q);
				let (ua, ub) = (uv(&a), uv(&b));
				let pos = [corner(a.x, a.y), corner(b.x, a.y), corner(b.x, b.y), corner(a.x, b.y)];
				let [Some(p0), Some(p1), Some(p2), Some(p3)] = pos else {
					st.skip[7] += 1;
					continue;
				};
				// A sprite takes the second vertex's F for the whole rectangle (gs_raster `sprite`).
				let tex = if fogged { fog_tex(tex, [b.f; 3], pr.fogcol) } else { tex };
				out.push(WorldQuad { pos: [p0, p1, p2, p3], uv: [ua, [ub[0], ua[1]], ub, [ua[0], ub[1]]], colour: [c; 4], tex, additive });
				st.tris += 2;
			} else {
				let pos = pr.v.map(|v| view.world(v.x, v.y, v.z, v.q));
				let [Some(p0), Some(p1), Some(p2)] = pos else {
					st.skip[7] += 1;
					continue;
				};
				let c = if flat { [colour(pr.v[2].rgba); 3] } else { pr.v.map(|v| colour(v.rgba)) };
				let t = pr.v.map(|v| uv(&v));
				let tex = if fogged { fog_tex(tex, [pr.v[0].f, pr.v[1].f, pr.v[2].f], pr.fogcol) } else { tex };
				let tex = if pi >= kicked { tex | FX_REFILL } else { tex };
				out.push(WorldQuad { pos: [p0, p1, p2, p2], uv: [t[0], t[1], t[2], t[2]], colour: [c[0], c[1], c[2], c[2]], tex, additive });
				st.tris += 1;
			}
		}
		flush(&mut run, pools);
	}
	st
}

/// A strip being built from consecutive FIX-blend prims: state key (fx texture incl. clamp flag, FIX, additive 0x68).
struct Run {
	key: (usize, u8, bool),
	pos: Vec<[f32; 3]>,
	uv: Vec<[f32; 2]>,
	colour: Vec<[f32; 4]>,
}

fn flush(run: &mut Option<Run>, pools: &mut Vec<WorldStrip>) {
	if let Some(r) = run.take() {
		pools.push(WorldStrip { pos: r.pos, uv: r.uv, colour: r.colour, tex: r.key.0, fix: r.key.1, add: r.key.2 });
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The fog word round trip: F bytes of vertices 0..2, the interned FOGCOL and the texture index survive `fog_tex`; a strip's
	/// own F wins over the packed one; `fog_off` and `plain_tex` behave as documented.
	#[test]
	fn fog_word_round_trip() {
		let tex = 0x1234 | crate::particles::FX_CLAMP;
		let w = fog_tex(tex, [10, 200, 255], Some(0x202020));
		assert_eq!(plain_tex(w), tex);
		let (l0, l1, l2, l3) = (fog_light(w, 0, -1.0), fog_light(w, 1, -1.0), fog_light(w, 2, -1.0), fog_light(w, 3, -1.0));
		assert_eq!([l0[0], l1[0], l2[0], l3[0]], [10.0, 200.0, 255.0, 255.0]);
		assert_eq!((l0[1], l0[3]), (0x202020 as f32, -5.0));
		assert_eq!(fog_light(w, 0, 77.0)[0], 77.0);
		let level = fog_tex(tex, [1, 2, 3], None);
		assert_eq!(fog_light(level, 0, -1.0)[1], -1.0);
		assert_eq!(fog_light(fog_off(tex), 0, -1.0), [0.0, 0.0, 0.0, -6.0]);
		assert_eq!(fog_light(tex, 0, -1.0), [0.0; 4]);
		assert_eq!(plain_tex(fog_off(tex)), tex);
	}

	/// ZTST GEQUAL passes, ALWAYS does not; ATE NEVER with AFAIL KEEP rejects the prim.
	#[test]
	fn depth_filter() {
		assert_eq!(depth_ok(Some(0x5360b), None).0, true);
		assert_eq!(depth_ok(Some(0x30000), None).0, false);
		assert_eq!(depth_ok(None, None), (true, false));
	}
}
