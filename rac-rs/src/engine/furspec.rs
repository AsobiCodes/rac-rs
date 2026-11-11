//! [PC] Fur: what a game says about the model that wears it (`pack::Pack::fur`; the effect itself is viewer/furfx.rs). The game
//! names the fur textures of its hero model by content hash (`texhash`) and gives each fur part its look; the engine never names a
//! model, a level or a texture. Without a game (`NONE`) there is no fur.

/// One fur texture of the hero model: the content hash of its decoded texels (`texhash`), the look (`FurSpec::parts` index) and which
/// texels of it are fur (a texture can hold fur, clothes and hardware side by side).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FurRow {
	pub hash: u64,
	/// Index into `FurSpec::parts`.
	pub part: u8,
	pub mask: FurMask,
}

/// The texels of a fur texture that carry fur: opaque, and either a fur colour (hue in degrees inside `hue`, saturation >= `sat`,
/// value >= `val`: the body colour, its stripes, the dark brows) or a pale fur colour (`pale` = hue low, hue high, saturation, value:
/// the light tufts). `add` texel rectangles [x0, y0, x1, y1) are fur whatever their colour (dark fur that looks like leather), then
/// `erode` texels come off the edge of every fur island (no strand grows from a clothes seam), then `cut` rectangles are no fur
/// (eyes, plates in a fur colour). `zones` paint where the fur differs from the part's look (`FurZone`; a face: short on the muzzle,
/// none on the eye rims, long tufts on the cheeks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FurMask {
	pub hue: [f32; 2],
	pub sat: f32,
	pub val: f32,
	pub pale: [f32; 4],
	pub erode: u8,
	pub add: &'static [[u16; 4]],
	pub cut: &'static [[u16; 4]],
	pub zones: &'static [FurZone],
}

/// A painted region of a fur texture: an ellipse inside the texel rectangle `at` [x0, y0, x1, y1), its edge blended over `feather`
/// texels, that scales the part's strand length (`len`, 0 = no fur) and outline halo (`halo`) and sets the `tone`: 1 = the part's
/// look (lighter tips, light wrap, sheen), 0 = the skin texel's own colour and shading kept (a face keeps its shape). Later zones
/// paint over earlier ones. Outside every zone: 1, 1, 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FurZone {
	pub at: [u16; 4],
	pub feather: f32,
	pub len: f32,
	pub halo: f32,
	pub tone: f32,
}

/// Byte of a `FurZone` length / halo scale of 1.0 in the map (exact, so the rest of the body keeps the part's look bit for bit);
/// the largest scale is 255 / ZONE_ONE.
pub const ZONE_ONE: f32 = 128.0;

impl FurMask {
	/// The fur map the shader samples by uv, one RGBA8 texel per texel: r = fur (`texels`), g = length scale and b = halo scale
	/// (byte / ZONE_ONE), a = tone (byte / 255).
	pub fn map(&self, w: usize, h: usize, rgba: &[[u8; 4]]) -> Vec<[u8; 4]> {
		let m = self.texels(w, h, rgba);
		let byte = |v: f32, one: f32| (v * one).round().clamp(0.0, 255.0) as u8;
		(0..w * h)
			.map(|i| {
				let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
				let mut v = [1.0f32; 3];
				for z in self.zones {
					let wt = z.weight(x, y);
					v = [v[0] + (z.len - v[0]) * wt, v[1] + (z.halo - v[1]) * wt, v[2] + (z.tone - v[2]) * wt];
				}
				[m[i], byte(v[0], ZONE_ONE), byte(v[1], ZONE_ONE), byte(v[2], 255.0)]
			})
			.collect()
	}

	/// One byte per texel: 255 = fur, 0 = not.
	pub fn texels(&self, w: usize, h: usize, rgba: &[[u8; 4]]) -> Vec<u8> {
		let mut m: Vec<u8> = rgba.iter().map(|&t| if self.is_fur(t) { 255 } else { 0 }).collect();
		let rect = |m: &mut Vec<u8>, r: &[u16; 4], v: u8| {
			for y in (r[1] as usize).min(h)..(r[3] as usize).min(h) {
				for x in (r[0] as usize).min(w)..(r[2] as usize).min(w) {
					if rgba[y * w + x][3] >= 128 {
						m[y * w + x] = v;
					}
				}
			}
		};
		self.add.iter().for_each(|r| rect(&mut m, r, 255));
		for _ in 0..self.erode {
			let src = m.clone();
			let at = |x: isize, y: isize| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && src[y as usize * w + x as usize] != 0;
			for y in 0..h as isize {
				for x in 0..w as isize {
					if !(at(x + 1, y) && at(x - 1, y) && at(x, y + 1) && at(x, y - 1)) {
						m[y as usize * w + x as usize] = 0;
					}
				}
			}
		}
		self.cut.iter().for_each(|r| rect(&mut m, r, 0));
		m
	}

	fn is_fur(&self, t: [u8; 4]) -> bool {
		let [r, g, b, a] = t.map(|c| c as f32 / 255.0);
		let v = r.max(g).max(b);
		let d = v - r.min(g).min(b);
		if a < 0.5 || v <= 0.0 || d <= 0.0 {
			return false;
		}
		let s = d / v;
		let h = if v == r { ((g - b) / d).rem_euclid(6.0) } else if v == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 } * 60.0;
		let p = self.pale;
		(v >= self.val && s >= self.sat && h >= self.hue[0] && h <= self.hue[1]) || (v >= p[3] && s >= p[2] && h >= p[0] && h <= p[1])
	}
}

impl FurZone {
	/// How much the zone paints texel centre (x, y): 1 inside the ellipse less `feather`, 0 outside it, smooth between.
	pub fn weight(&self, x: f32, y: f32) -> f32 {
		let [x0, y0, x1, y1] = self.at.map(|c| c as f32);
		let (rx, ry) = ((x1 - x0) * 0.5, (y1 - y0) * 0.5);
		if rx <= 0.0 || ry <= 0.0 {
			return 0.0;
		}
		let (dx, dy) = ((x - (x0 + rx)) / rx, (y - (y0 + ry)) / ry);
		// Distance outside the ellipse's edge in texels (negative inside), along the smaller radius.
		let d = ((dx * dx + dy * dy).sqrt() - 1.0) * rx.min(ry);
		let f = self.feather.max(0.5);
		let t = ((d + f) / f).clamp(0.0, 1.0);
		1.0 - t * t * (3.0 - 2.0 * t)
	}
}

/// The look of one fur part. Two styles the player picks: Soft (short, close to the PS2 look) and Fluffy (the full plush look);
/// every `[f32; 2]` is Soft, Fluffy. Lengths are world metres at the top shell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FurPart {
	pub name: &'static str,
	/// Strand length (m).
	pub len: [f32; 2],
	/// Comb: how far the tips lean down along the surface at the top shell (m).
	pub comb: [f32; 2],
	/// Fur noise tiles per uv unit of the part's texture (one tile holds ~6800 strands, viewer/furfx/noise.rs).
	pub density: f32,
	/// Opacity of the fur layer (1 = the strand coverage as is), independent of the shell count.
	pub thick: [f32; 2],
	/// Extra opacity where the surface turns away from the view (the soft fuzzy outline).
	pub halo: [f32; 2],
	/// Tip colour multiplier of the texel colour (1.0 = unchanged) and the root shade (0 = none, 1 = black).
	pub tip: [f32; 3],
	pub root: f32,
	/// Warm sheen at grazing angles.
	pub sheen: [f32; 2],
	/// Self-shadow at the root (0 = none, 1 = black), fading to none at the tip: the depth of the coat.
	pub occ: f32,
	/// Strand highlight (Kajiya-Kay along the strand) strength and its exponent.
	pub spec: f32,
	pub spec_exp: f32,
	/// Tip lift towards the colour's own lighter shade (dark stripes get lighter tips).
	pub lift: f32,
	/// Strand coverage at which a strand is fully opaque (0 = off): crisp strands.
	pub sharp: f32,
	/// Strand edge shade (0 = none): a strand's core is bright, its edges darker.
	pub strand: f32,
}

/// The game's fur table for its hero model.
#[derive(Clone, Copy, Debug)]
pub struct FurSpec {
	pub rows: &'static [FurRow],
	pub parts: &'static [FurPart],
}

/// Shells per quality level (Graphics `fur_q` 1, 2, 3): Soft, Fluffy. Same look at every level (the shader spreads the opacity over
/// the shells); more shells = a smoother, denser outline. High costs most.
pub const SHELLS: [[u32; 2]; 3] = [[8, 10], [12, 16], [20, 32]];

/// The shell count of a quality level (`q` 1..=3; 0 = no fur) and style (`long` = Fluffy).
pub fn shells(q: u8, long: bool) -> u32 {
	match q {
		1..=3 => SHELLS[q as usize - 1][long as usize],
		_ => 0,
	}
}

/// FNV-1a over the width, the height and the RGBA bytes of a decoded texture: the key of `FurRow::hash`.
pub fn texhash(w: usize, h: usize, rgba: &[[u8; 4]]) -> u64 {
	let mut x = 0xcbf2_9ce4_8422_2325u64;
	let head = [w as u8, (w >> 8) as u8, h as u8, (h >> 8) as u8];
	for b in head.into_iter().chain(rgba.iter().flatten().copied()) {
		x = (x ^ b as u64).wrapping_mul(0x0100_0000_01b3);
	}
	x
}
