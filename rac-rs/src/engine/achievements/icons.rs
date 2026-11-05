//! Achievement icons (user 2026-11-02: 36 icons, one per trophy plus the hidden one). A game's pack hands over one small PNG per list
//! entry (`AchPack::icons`, game-neutral: the engine never names a game); this file turns them into ONE RGBA atlas that the GPU
//! uploads once (`Gpu::ach_icons`, a linear sampler with mips) and says where every picture sits (`Layout`, plain arithmetic: the
//! pop-up and the list need no decoded pixels to place their quads).
//!
//! Atlas tiles, all `size` x `size`: the colour icons (entry i = tile i, a circle: the corners are transparent), the icon for a locked
//! hidden entry, the grey icons (a locked entry's look), and one white bevelled ring that is tinted with the grade colour when drawn.
//! The PNGs are decoded with the engine's own decoder (`texpack::png`, no new dependency): 8-bit RGB files, so the alpha of the
//! decoded picture is opaque and the circle is cut here.
use crate::level::Texture;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// `Quad.tex` selecting the icon atlas (drawn by `Gpu::pass_hud`, like `splitview::SPLIT_TEX`).
pub const ICON_TEX: usize = usize::MAX - 9;

/// A locked entry's grey icon keeps this share of the colour icon's brightness.
const GREY_LEVEL: f32 = 0.6;
/// The ring tile: the ring reaches from this share of the tile radius out to the tile edge.
const RING_INNER: f32 = 0.88;

/// A game's icons: one square 8-bit RGB PNG per entry of `AchPack::defs` (same index) and one for a locked hidden entry.
pub struct IconSet {
	/// Edge of every PNG in pixels.
	pub size: u32,
	pub png: &'static [&'static [u8]],
	pub hidden: &'static [u8],
}

/// Where the tiles sit in the atlas for `n` entries (see the file header for the order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
	pub n: usize,
	pub size: u32,
}

impl Layout {
	pub fn new(n: usize, size: u32) -> Layout {
		Layout { n, size }
	}
	pub fn tiles(&self) -> usize {
		2 * self.n + 2
	}
	/// Tiles per row: the smallest square-ish grid.
	pub fn cols(&self) -> usize {
		(1..).find(|c| c * c >= self.tiles()).unwrap_or(1)
	}
	pub fn rows(&self) -> usize {
		self.tiles().div_ceil(self.cols())
	}
	/// Atlas size in pixels.
	pub fn dims(&self) -> (usize, usize) {
		(self.cols() * self.size as usize, self.rows() * self.size as usize)
	}
	pub fn colour(&self, i: usize) -> usize {
		i
	}
	pub fn hidden(&self) -> usize {
		self.n
	}
	pub fn grey(&self, i: usize) -> usize {
		self.n + 1 + i
	}
	pub fn ring(&self) -> usize {
		2 * self.n + 1
	}
	/// The tile entry `i` shows: its colour icon once unlocked, the hidden icon while a hidden one is locked, else the grey icon.
	pub fn tile_for(&self, i: usize, unlocked: bool, hidden: bool) -> usize {
		match (unlocked, hidden) {
			(true, _) => self.colour(i),
			(false, true) => self.hidden(),
			(false, false) => self.grey(i),
		}
	}
	/// Top-left pixel of a tile.
	pub fn origin(&self, tile: usize) -> (usize, usize) {
		((tile % self.cols()) * self.size as usize, (tile / self.cols()) * self.size as usize)
	}
	/// UV rectangle of a tile, inset by half a texel so the linear filter never reads the neighbour tile.
	pub fn uv(&self, tile: usize) -> [f32; 4] {
		let (w, h) = self.dims();
		let (x, y) = self.origin(tile);
		let s = self.size as f32;
		[(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32, (x as f32 + s - 0.5) / w as f32, (y as f32 + s - 0.5) / h as f32]
	}
}

/// Decodes one icon file to `size` x `size` RGBA (an error names what is wrong: bad file, wrong size).
pub fn decode(png: &[u8], size: u32) -> Result<Vec<[u8; 4]>, String> {
	let im = crate::texpack::png::load(png)?;
	if im.w != size || im.h != size {
		return Err(format!("icon is {}x{}, the set says {size}x{size}", im.w, im.h));
	}
	let px = im.levels.first().filter(|l| l.len() == (size * size * 4) as usize).ok_or("icon: decoded size mismatch")?;
	Ok(px.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect())
}

/// Per pixel: (distance from the tile centre in pixels, coverage 0..1 of the disc of radius `size / 2` with a one-pixel soft edge).
fn disc(x: usize, y: usize, size: usize) -> (f32, f32) {
	let c = size as f32 * 0.5;
	let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
	(d, (c - d + 0.5).clamp(0.0, 1.0))
}

/// The icon inside the circle (corners transparent); `grey` = the locked look.
fn circle(src: &[[u8; 4]], size: usize, grey: bool) -> Vec<[u8; 4]> {
	let mut out = Vec::with_capacity(src.len());
	for (i, p) in src.iter().enumerate() {
		let (_, cover) = disc(i % size, i / size, size);
		let a = (p[3] as f32 * cover).round() as u8;
		out.push(if grey {
			let l = ((p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114) * GREY_LEVEL).round().min(255.0) as u8;
			[l, l, l, a]
		} else {
			[p[0], p[1], p[2], a]
		});
	}
	out
}

/// The white ring: lit from the top left (brightest there, darkest bottom right), soft on both edges. The caller tints it.
fn ring(size: usize) -> Vec<[u8; 4]> {
	let r = size as f32 * 0.5;
	let c = r;
	let mut out = Vec::with_capacity(size * size);
	for y in 0..size {
		for x in 0..size {
			let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
			let d = (dx * dx + dy * dy).sqrt().max(1e-3);
			let outer = (r - d + 0.5).clamp(0.0, 1.0);
			let inner = (d - RING_INNER * r + 0.5).clamp(0.0, 1.0);
			let lit = (-(dx + dy) / (d * std::f32::consts::SQRT_2)).clamp(-1.0, 1.0);
			let v = ((0.78 + 0.22 * lit) * 255.0).round() as u8;
			out.push([v, v, v, (outer.min(inner) * 255.0).round() as u8]);
		}
	}
	out
}

/// Builds the atlas of `set` for `n` entries. Errors: a count that does not match `n`, a file that does not decode or has the wrong size.
pub fn build(set: &IconSet, n: usize) -> Result<Texture, String> {
	if set.png.len() != n {
		return Err(format!("icon set has {} pictures for {n} entries", set.png.len()));
	}
	let l = Layout::new(n, set.size);
	let (w, h) = l.dims();
	let s = set.size as usize;
	let mut rgba = vec![[0u8; 4]; w * h];
	let mut put = |tile: usize, px: &[[u8; 4]]| {
		let (ox, oy) = l.origin(tile);
		for y in 0..s {
			rgba[(oy + y) * w + ox..(oy + y) * w + ox + s].copy_from_slice(&px[y * s..(y + 1) * s]);
		}
	};
	for (i, png) in set.png.iter().enumerate() {
		let src = decode(png, set.size).map_err(|e| format!("icon {i}: {e}"))?;
		put(l.colour(i), &circle(&src, s, false));
		put(l.grey(i), &circle(&src, s, true));
	}
	let hidden = decode(set.hidden, set.size).map_err(|e| format!("hidden icon: {e}"))?;
	put(l.hidden(), &circle(&hidden, s, false));
	put(l.ring(), &ring(s));
	Ok(Texture { w, h, rgba })
}

/// Set once the running game's atlas could not be built (a bad icon file, or the build thread died): `badge` then draws the cups
/// instead of quads that the GPU has no atlas for (`pass_hud` drops an `ICON_TEX` quad without a bind group, so an entry would show no badge).
static ATLAS_FAILED: AtomicBool = AtomicBool::new(false);

/// True after a failed atlas build (see `ATLAS_FAILED`); false while it is still being built or when the game has no icons.
pub fn atlas_failed() -> bool {
	ATLAS_FAILED.load(Ordering::Relaxed)
}

/// The running game's atlas built now (None: the game has no icons, or a file is bad: the pop-up and the list keep the cups).
fn build_running() -> Option<Texture> {
	let p = crate::pack::get().achievements?;
	match build(p.icons?, p.defs.len()) {
		Ok(t) => Some(t),
		Err(e) => {
			eprintln!("achievements: icons not used: {e}");
			ATLAS_FAILED.store(true, Ordering::Relaxed);
			None
		}
	}
}

/// The helper thread `prewarm` started (decoding takes some milliseconds: not on the frame that shows the first pop-up).
static PREWARM: Mutex<Option<std::thread::JoinHandle<Option<Texture>>>> = Mutex::new(None);

/// Starts building the atlas on a helper thread (`achievements::init`, the first field of real play); `atlas` takes the result.
pub fn prewarm() {
	let mut h = PREWARM.lock().unwrap_or_else(|e| e.into_inner());
	if h.is_none() && crate::pack::get().achievements.is_some_and(|p| p.icons.is_some()) {
		*h = std::thread::Builder::new().name("ach-icons".into()).spawn(build_running).ok();
	}
}

/// The running game's atlas, once (the GPU uploads it and the pixels are dropped): the prewarmed one, else built here.
pub fn atlas() -> Option<Texture> {
	let started = PREWARM.lock().unwrap_or_else(|e| e.into_inner()).take();
	match started {
		Some(h) => h.join().unwrap_or_else(|_| {
			eprintln!("achievements: the icon thread died, icons not used");
			ATLAS_FAILED.store(true, Ordering::Relaxed);
			None
		}),
		None => build_running(),
	}
}
