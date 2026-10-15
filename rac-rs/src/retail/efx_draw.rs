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
	vq: [V; 3],
	n: usize,
	lines: u32,
	/// The stream's writes to XYOFFSET, SCISSOR, FRAME or ZBUF (either context): (register, value), distinct.
	target: Vec<(u8, u64)>,
}

impl Gs {
	fn new() -> Gs {
		Gs { prim: 0, prmode: 0, prmodecont: true, rgba: [0x80; 4], q: 1.0, st: [0.0; 2], uv: [0.0; 2], tex0: [0; 2], clamp: [0; 2], alpha: [0; 2], test: [None; 2], zbuf: [None; 2], tex1: [None; 2], vq: [V::default(); 3], n: 0, lines: 0, target: Vec::new() }
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
				3..=6 => out.push(Prim { sprite: ty == 6, v: self.vq, attr, tex0: self.tex0[ctx], clamp: self.clamp[ctx], alpha: self.alpha[ctx], test: self.test[ctx], zbuf: self.zbuf[ctx], tex1: self.tex1[ctx] }),
				_ => self.lines += 1,
			}
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

/// Whether an older one-off port already draws callback `f` on this level (the claimed set): every module that turns
/// queue callbacks into mirror geometry answers for its own tables.
pub fn claimed(m: &Machine, level: u32, f: u32) -> bool {
	WATER_DRIVERS.contains(&(level, f)) || super::queue_b::claims(m, level, f) || super::queue_q1::claims(level, f) || super::queue_story::claims(level, f) || super::queue_fxa::claims(level, f) || super::claims(level, f)
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

fn colour(c: [u8; 4]) -> [f32; 4] {
	c.map(|x| x as f32 / 128.0)
}

/// RAC_EFX_DUMP=<hex fn>: print that owner's GS prims (first 48 per field) to stderr (debug only).
fn dump_fn() -> Option<u32> {
	static F: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
	*F.get_or_init(|| std::env::var("RAC_EFX_DUMP").ok().and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok()))
}

/// RAC_EFX_DRAW=0 leaves the layer out (A/B pictures); default on.
pub fn enabled() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| !std::env::var("RAC_EFX_DRAW").is_ok_and(|v| v == "0"))
}

/// Everything `build_in` reads from the machine at a field's end, copied (a few KB) so the build can run after the
/// worker has gone on to the next field: per owner whether an older port claims it and its packet walk (None when claimed),
/// the draw environment's ten registers and the fx slot table's TEX0 words (`slot`).
pub struct BuildIn {
	level: u32,
	own: Vec<Own>,
	pub claimed: Vec<bool>,
	pub pkts: Vec<Option<efx_own::Pkt>>,
	env: Vec<(u8, u64)>,
	slots: Vec<u64>,
}

/// `BuildIn` of the field the machine has just ended (reads RAM).
pub fn capture(m: &Machine, level: u32, own: &[Own]) -> BuildIn {
	let claimed: Vec<bool> = own.iter().map(|o| claimed(m, level, o.fnp)).collect();
	let pkts = own.iter().zip(&claimed).map(|(o, &c)| (!c).then(|| efx_own::walk(&m.cpu, o))).collect();
	let env = (0..10).map(|k| DRAW_ENV + 16 * k).map(|a| (m.rd32(a + 8) as u8, m.rd32(a) as u64 | (m.rd32(a + 4) as u64) << 32)).collect();
	let tbl = SLOT_TABLE.wrapping_add_signed(*super::LEVEL_SHIFT.get(level as usize).unwrap_or(&0));
	let slots = (0..SLOTS).map(|n| tbl + 16 * n).map(|a| m.rd32(a) as u64 | (m.rd32(a + 4) as u64) << 32).collect();
	BuildIn { level, own: own.to_vec(), claimed, pkts, env, slots }
}

/// The field's owners (`efx_own::field_end`) -> world quads for the mirror (`Fx.qa`), in queue order.
pub fn build(m: &Machine, level: u32, own: &[Own], vu: &super::efx_vu::VuOut, out: &mut Vec<WorldQuad>, pools: &mut Vec<WorldStrip>) -> Stats {
	build_in(&capture(m, level, own), vu, out, pools)
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
			// ALPHA low byte: 0x48 Cs*As + Cd, 0x44 Cs*As + Cd*(1-As); FIX blends (C = 2): 0x64 (Cs-Cd)*FIX + Cd, 0x68 Cs*FIX + Cd.
			let (additive, fix) = match (attr & 0x40 != 0, pr.alpha & 0xff) {
				(true, 0x48) => (true, None),
				(true, 0x44) => (false, None),
				(true, 0x64) => (false, Some(((((pr.alpha >> 32) & 0xff) as u8).min(0x80), false))),
				(true, 0x68) => (true, Some(((((pr.alpha >> 32) & 0xff) as u8).min(0x80), true))),
				_ => {
					st.skip[4] += 1;
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
			if let Some((fix, add)) = fix {
				// FIX blends go to the strip pass (texture and vertex alpha ignored): consecutive prims of one state share a strip.
				let tri: Option<[([f32; 3], [f32; 2], [f32; 4]); 3]> = if pr.sprite {
					None
				} else {
					let pos = pr.v.map(|v| view.world(v.x, v.y, v.z, v.q));
					match pos {
						[Some(p0), Some(p1), Some(p2)] => {
							let c = if flat { [colour(pr.v[2].rgba); 3] } else { pr.v.map(|v| colour(v.rgba)) };
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
						let c = colour(b.rgba);
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
				let key = (tex, fix, add);
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
