//! [PC] DEV: RAC_TEXNAMES=<file.tsv> collects the PCSX2 name of every texture the game draws through the two GS paths (the GS capture =
//! HUD, menus, fonts, pause / vendor pages; the GS-full pipe = title, front end, and the whole picture when run with --gsfull), for
//! the texture-pack coverage measurement (lane tex2, stage A). Zero cost when the variable is unset: the hooks test `on()` first.
//!
//! Two phases per draw so the caller can keep its borrows: `probe` (cache hit = counted, nothing more) and, on a miss only, the caller
//! flushes its pending uploads and hands the 4 MB local memory to `record`, which names the texture (`gsname::name_for_draw`).
//! The cache key is the draw's texture registers plus a generation counter the caller bumps whenever local memory may have changed
//! under a cached texture, so a changed texture is named again.
//!
//! The TSV (rewritten at most once a second and at `finish`): stem, psm, w, h, region, draws, first_field, mxl, draw_w, draw_h,
//! slot_variants (distinct names seen at the same TBP / TBW / PSM / TW / TH; more than 3 = a dynamic texture), tbp, origin
//! (cap / full / both), raw, sig (8 x 8 mean RGB grid, hex), mipstems (names of mip levels 1..MXL at the MIPTBP bases, `;`-joined, `-` = none).
//! Names and signatures only, never pictures or game bytes.

use super::gsname;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Which GS path saw the draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
	Cap,
	Full,
}

/// The registers of one textured draw that decide its name, plus what the TSV logs next to the name.
#[derive(Clone, Copy, Debug)]
pub struct Draw {
	pub tex0: u64,
	pub clamp: u64,
	pub texa: u64,
	pub texclut: u64,
	pub tex1: u64,
	/// MIPTBP1 / MIPTBP2 of the draw's context (0 on the capture path: its textures have no mips).
	pub mip1: u64,
	pub mip2: u64,
	/// Pixel extent of the draw's primitive (0 when unknown).
	pub w: u32,
	pub h: u32,
	pub field: u32,
}

/// A cache miss: the caller flushes, then passes this and the local memory to `record`.
pub struct Probe {
	origin: Origin,
	epoch: u32,
	d: Draw,
}

type Key = (u64, u64, u64, u64, u64);
type Slot = (u32, u32, u32, u32, u32);

/// TEX0 bits that decide a name: TBP0, TBW, PSM, TW, TH and the CLUT fields; not TCC, TFX, CLD.
const TEX0_MASK: u64 = ((1u64 << 34) - 1) | (((1u64 << 24) - 1) << 37);

struct Rec {
	psm: u32,
	w: u32,
	h: u32,
	region: (u32, u32),
	draws: u64,
	first_field: u32,
	mxl: u32,
	dw: u32,
	dh: u32,
	tbp: u32,
	origin: u8,
	raw: bool,
	slot: Slot,
	sig: Option<Vec<u8>>,
	/// Names of mip levels 1..=MXL at the draw's MIPTBP bases (first sight), `;`-joined.
	mips: String,
}

struct State {
	path: PathBuf,
	/// Draw key -> (generation, stem); an empty stem = no texture (format the hash cannot name).
	cache: HashMap<Key, (u32, String)>,
	recs: HashMap<String, Rec>,
	slots: HashMap<Slot, Vec<u64>>,
	dirty: bool,
	last_write: Instant,
	misses: u64,
	hits: u64,
	raw_n: u32,
}

static ON: OnceLock<Option<PathBuf>> = OnceLock::new();
static STATE: OnceLock<Mutex<State>> = OnceLock::new();

/// The output file when RAC_TEXNAMES is set (read once).
pub fn on() -> Option<&'static PathBuf> {
	ON.get_or_init(|| std::env::var_os("RAC_TEXNAMES").filter(|v| !v.is_empty()).map(PathBuf::from)).as_ref()
}

fn state() -> &'static Mutex<State> {
	STATE.get_or_init(|| Mutex::new(State { path: on().cloned().unwrap_or_default(), cache: HashMap::new(), recs: HashMap::new(), slots: HashMap::new(), dirty: false, last_write: Instant::now(), misses: 0, hits: 0, raw_n: 0 }))
}

fn key_of(d: &Draw) -> Key {
	let mips = if (d.tex1 >> 2) & 7 > 0 { d.mip1 ^ d.mip2.rotate_left(29) } else { 0 };
	(d.tex0 & TEX0_MASK, d.clamp & ((1u64 << 44) - 1), d.texa, d.texclut, mips)
}

fn bump(r: &mut Rec, d: &Draw, origin: Origin) {
	r.draws += 1;
	r.dw = r.dw.max(d.w);
	r.dh = r.dh.max(d.h);
	r.origin |= if origin == Origin::Cap { 1 } else { 2 };
}

/// Counts the draw when its texture is cached for this generation (None); else the caller must `record`.
pub fn probe(origin: Origin, epoch: u32, d: Draw) -> Option<Probe> {
	on()?;
	let mut st = state().lock().ok()?;
	if let Some((g, stem)) = st.cache.get(&key_of(&d)).cloned() {
		if g == epoch {
			st.hits += 1;
			if let Some(r) = st.recs.get_mut(&stem) {
				bump(r, &d, origin);
			}
			return None;
		}
	}
	Some(Probe { origin, epoch, d })
}

/// Names the texture from the local memory `vram` (4 MB, uploads flushed) and logs it.
pub fn record(p: Probe, vram: &[u8]) {
	let Ok(mut st) = state().lock() else { return };
	let d = p.d;
	st.misses += 1;
	let Some(n) = gsname::name_for_draw(vram, d.tex0, d.clamp, d.texa, d.texclut, true) else {
		st.cache.insert(key_of(&d), (p.epoch, String::new()));
		return;
	};
	let stem = n.name.stem(n.palette);
	if let Some(dir) = std::env::var_os("RAC_TEXNAMES_RAW") {
		// DEV diagnostic (never keep the output): the raw block stream of the first distinct T8 names with a dynamic slot, for the table comparison.
		if !st.recs.contains_key(&stem) && n.psm == 0x13 && st.raw_n < 600 {
			let t = gsname::Tex::from_tex0(d.tex0);
			if let Some(s) = gsname::raw_stream(vram, &t, &gsname::Region::create(&t, d.clamp)) {
				st.raw_n += 1;
				use std::io::Write;
				if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir) {
					let _ = writeln!(f, "{stem}	{}", s.len());
					let _ = f.write_all(&s);
					let _ = writeln!(f);
				}
			}
		}
	}
	let t = gsname::Tex::from_tex0(d.tex0);
	let slot: Slot = (t.tbp, t.tbw, t.psm, t.tw, t.th);
	let v = st.slots.entry(slot).or_default();
	if !v.contains(&n.name.tex0) && v.len() < 64 {
		v.push(n.name.tex0);
	}
	st.cache.insert(key_of(&d), (p.epoch, stem.clone()));
	let r = st.recs.entry(stem).or_insert_with(|| Rec {
		psm: n.psm,
		w: n.w,
		h: n.h,
		region: (n.name.region_w, n.name.region_h),
		draws: 0,
		first_field: d.field,
		mxl: ((d.tex1 >> 2) & 7) as u32,
		dw: 0,
		dh: 0,
		tbp: t.tbp,
		origin: 0,
		raw: n.raw,
		slot,
		sig: n.sig.clone(),
		mips: gsname::mip_stems(vram, d.tex0, d.texa, d.texclut, ((d.tex1 >> 2) & 7) as u32, d.mip1, d.mip2).join(";"),
	});
	bump(r, &d, p.origin);
	st.dirty = true;
	write_if_due(&mut st, false);
}

fn write_if_due(st: &mut State, force: bool) {
	if !st.dirty || (!force && st.last_write.elapsed() < Duration::from_secs(1)) {
		return;
	}
	st.last_write = Instant::now();
	st.dirty = false;
	let mut stems: Vec<&String> = st.recs.keys().collect();
	stems.sort();
	let mut out = String::from("stem\tpsm\tw\th\tregion\tdraws\tfirst_field\tmxl\tdraw_w\tdraw_h\tslot_variants\ttbp\torigin\traw\tsig\tmipstems\n");
	for s in stems {
		let r = &st.recs[s];
		let region = if r.region == (0, 0) { "-".to_string() } else { format!("{}x{}", r.region.0, r.region.1) };
		let origin = match r.origin {
			1 => "cap",
			2 => "full",
			_ => "both",
		};
		let variants = st.slots.get(&r.slot).map_or(1, |v| v.len());
		let _ = write!(out, "{s}\t{:#x}\t{}\t{}\t{region}\t{}\t{}\t{}\t{}\t{}\t{variants}\t{}\t{origin}\t{}\t", r.psm, r.w, r.h, r.draws, r.first_field, r.mxl, r.dw, r.dh, r.tbp, r.raw as u8);
		if let Some(sig) = &r.sig {
			for b in sig {
				let _ = write!(out, "{b:02x}");
			}
		}
		out.push('\t');
		out.push_str(if r.mips.is_empty() { "-" } else { &r.mips });
		out.push('\n');
	}
	if let Err(e) = std::fs::write(&st.path, out) {
		eprintln!("texnames: {}: {e}", st.path.display());
	}
}

/// Final write (the shot ends the process right after); logs the counters.
pub fn finish() {
	if on().is_none() {
		return;
	}
	if let Ok(mut st) = state().lock() {
		write_if_due(&mut st, true);
		eprintln!("texnames: {} names, {} draws cached, {} named ({})", st.recs.len(), st.hits, st.misses, st.path.display());
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn key_ignores_tcc_tfx_cld() {
		let a = Draw { tex0: 0x1234 | 1 << 34 | 2 << 35 | 1 << 61, clamp: 0, texa: 0, texclut: 0, tex1: 0, mip1: 0, mip2: 0, w: 0, h: 0, field: 0 };
		let b = Draw { tex0: 0x1234, ..a };
		assert_eq!(key_of(&a), key_of(&b));
		let c = Draw { tex0: 0x1234 | 1 << 37, ..a };
		assert_ne!(key_of(&c), key_of(&b));
	}
}
