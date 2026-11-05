//! The effect primitives a game pack hands the renderer each field (engine split stage E2 batch 4b, 2026-10-26: moved out of
//! rac1/particles.rs, particles/tracer.rs, retail/efx_draw.rs, water.rs, ocean.rs, retail/fx_queue.rs and retail/shadow_snap.rs;
//! the old paths re-export): billboards (`Sprite`), ribbons, lines, world quads and strips, effect / debris / orb mobys, the
//! draw records of vendor glows and ship shadows, and the bits of a prim's `tex` word (texture variants `FX_CLAMP` /
//! `FX_NOMIP` / `FX_REFILL`, the GS fog `FOG_ON` / `FOG_OFF` with `fog_light` the vertex light the shaders read).

/// Added to an fx_textures index to say "this is not an effect type".
pub const FX_TEX: u32 = 0x1000;

/// What the renderer draws: one camera-facing billboard.
#[derive(Clone, Copy, Debug)]
pub struct Sprite {
	pub pos: [f32; 3],
	/// Full side of the textured square in world units. Pool sprites are scaled by vz/(vz+0.5) at draw.
	pub size: f32,
	/// Radians. Pool sprites: the game's rotation angle (byte * 2pi/256); the renderer turns it into the
	/// texture orientation (prog2: 135 deg + angle clockwise on screen).
	pub rot: f32,
	/// 0..1 channels (the game's 0x80 = 1.0).
	pub colour: [f32; 4],
	/// Effect type id (TEX_*, or FX_TEX + fx index); the renderer resolves the particle texture.
	pub tex: u32,
	/// GS ALPHA from effect byte +3: 0x48 = additive ((Cs-0)*As+Cd, the default), 0x44 = source-alpha-over.
	pub additive: bool,
	/// Pool sprites drawn by FUN_00267628: byte +9 (near/far fade). None = drawn in world units as given.
	pub pool: Option<u8>,
	/// Mode 1 (+1 = 1): quad in the world XY plane, decomp/notes/particles.md Mode-1 flat pool sprite.
	pub flat: bool,
}

/// A camera-facing ribbon along `points` (lightning arcs, FUN_002e2630).
#[derive(Clone, Debug)]
pub struct Ribbon {
	pub points: Vec<[f32; 3]>,
	/// Half-width at the first and last point [P scale].
	pub width0: f32,
	pub width1: f32,
	pub colour: [f32; 4],
	/// Colour at the tip (reddish on the robot's arc).
	pub tip_colour: [f32; 4],
	pub tex: u32,
	/// U scroll offset (-0.2 per tick, wrapped at -8).
	pub uv_scroll: f32,
	pub additive: bool,
	/// Lightning strand (FUN_002e2630): per-point Hermite taper t = j/20, UVs U scroll..scroll+1 per segment
	/// and V 0..1 across, first segment not drawn.
	pub arc: bool,
}

/// A world-space quad with its own fx texture (blob shadows, fire quads): corners in fan order.
#[derive(Clone, Copy, Debug)]
pub struct WorldQuad {
	pub pos: [[f32; 3]; 4],
	pub uv: [[f32; 2]; 4],
	pub colour: [[f32; 4]; 4],
	/// fx texture index (level fx table); `| FX_CLAMP` samples it with GS CLAMP_1 = 5 instead of REPEAT.
	pub tex: usize,
	/// GS ALPHA_1 0x48 (additive) vs 0x44 (source-alpha-over).
	pub additive: bool,
}

/// `WorldQuad.tex` flag: the GS context's CLAMP_1 is 5 (WMS = WMT = CLAMP) for this quad, so the viewer binds the
/// texture with a clamp-to-edge sampler (fx table key `tex | FX_CLAMP`). Without it the quad samples with REPEAT
/// (CLAMP_1 = 0, the other fx contexts) [C: the context words that FUN_001ef7c0 sends as CLAMP_1 / TEX0_1].
pub const FX_CLAMP: usize = 1 << 20;

/// `WorldStrip.tex` flag: sample the fx texture with REPEAT, linear and no mip levels (GS CLAMP_1 0, TEX1_1 0xff9000000260:
/// MXL 0, MMAG / MMIN linear), fx table key `tex | FX_NOMIP` (the Kalebo / Fleet glass of `retail::queue_q1`).
pub const FX_NOMIP: usize = 1 << 21;

/// BRIDGE-23: a world-space textured triangle strip drawn with the GS FIX blend `out = (Cs - Cd) * fix / 128 + Cd`
/// (ALPHA A=Cs B=Cd C=FIX D=Cd); texture and vertex alpha are ignored, depth-tested, fogged. Triangles (i, i+1, i+2).
#[derive(Clone, Debug, PartialEq)]
pub struct WorldStrip {
	pub pos: Vec<[f32; 3]>,
	pub uv: Vec<[f32; 2]>,
	pub colour: Vec<[f32; 4]>,
	/// fx texture index (level fx table), as `WorldQuad.tex`.
	pub tex: usize,
	/// GS ALPHA FIX value (0..=128).
	pub fix: u8,
	/// ALPHA A=Cs B=0 C=FIX D=Cd (0x68): out = Cs * fix / 128 + Cd (additive, constant weight) instead of the FIX lerp; no Z write.
	pub add: bool,
}

/// A game-owned, non-placed model instance drawn through the debris batch (NOV-10.5a-draw teleporter rings): `m` columns
/// x, y, z as `Debris.m` (`euler_m` convention).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldInst {
	pub class: u32,
	pub pos: [f32; 3],
	pub m: [[f32; 3]; 3],
	pub scale: f32,
}

/// A debris moby (spawned by FUN_002af258 for crates, FUN_00263be8 for robots), stepped by
/// `Particles::step_debris`.
#[derive(Clone, Copy, Debug)]
pub struct Debris {
	pub class: u32,
	/// Moby origin.
	pub pos: [f32; 3],
	/// Orientation, local -> world, as columns x, y, z.
	pub m: [[f32; 3]; 3],
	/// Scale relative to the class's normal size.
	pub scale: f32,
	/// Moby +0x32 draw distance (SCH-6b): crate and robot pieces 255.
	pub draw: i16,
}

/// Rotation matrix of euler angles (columns), the same convention as the viewer's `place`.
pub fn euler_m(rot: [f32; 3]) -> [[f32; 3]; 3] {
	let (sx, cx) = rot[0].sin_cos();
	let (sy, cy) = rot[1].sin_cos();
	let (sz, cz) = rot[2].sin_cos();
	[[cz * cy, sz * cy, -sy], [cz * sy * sx - sz * cx, sz * sy * sx + cz * cx, cy * sx], [cz * sy * cx + sz * sx, sz * sy * cx - cz * sx, cy * cx]]
}

/// What the renderer draws for the blast's moby effects (classes 112 and 122).
#[derive(Clone, Copy, Debug)]
pub struct EffectMoby {
	pub class: u32,
	pub pos: [f32; 3],
	/// Euler angles about x, y, z.
	pub rot: [f32; 3],
	pub scale: f32,
	/// 0..0x80.
	pub alpha: f32,
	/// Moby colour modulation, 1.0 = 0x80.
	pub tint: [f32; 3],
	/// Moby +0x32 draw distance (SCH-6b): shards, fireballs and glows 255; None = no draw-distance cull (water).
	pub draw: Option<i16>,
}

/// PAR-2a: one nanotech orb to draw (FUN_002d91b8): the actor position (no bob), the bob added to the 0.5 centre
/// lift, the parent crate matrix rows (images of the x, y, z axes), the glass while the crate is intact (state 1).
#[derive(Clone, Copy, Debug)]
pub struct OrbDraw {
	pub pos: [f32; 3],
	pub bob: f32,
	pub parent: [[f32; 3]; 3],
	pub glass: bool,
}

/// A one-pixel Gouraud line (pool draw mode 2): colour `col_a` at `a`, `col_b` at `b` (alpha over 128), drawn alpha-over
/// with no texture and no fog while the midpoint view depth lies strictly inside (`near`, `far`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
	pub a: [f32; 3],
	pub b: [f32; 3],
	pub col_a: [f32; 4],
	pub col_b: [f32; 4],
	pub near: f32,
	pub far: f32,
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
pub fn fog_tex(tex: usize, f: [u8; 3], fogcol: Option<u32>) -> usize {
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
pub fn fog_off(tex: usize) -> usize {
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

/// NOV-28.2: pool-particle far cap while the camera is dry (int 0x1f4000, 12-bit fixed, FUN_00218c70).
pub const PART_FAR: f32 = 500.0;

/// NOV-28.2: a pool particle's far distance in m: byte +9 high nibble * 32 m, capped by the live clamp
/// (`pminw((b9 & 0xf0) << 13, clamp)`, FUN_0027c190).
pub fn pool_far(b9: u8, clamp: f32) -> f32 {
	((b9 >> 4) as f32 * 32.0).min(clamp)
}

/// NOV-28.3: a GS colour for the gamma-space Unorm target: the raw byte / 255, as the GS blends and fogs it.
pub fn gs_rgb(c: [u8; 3]) -> [f32; 3] {
	[c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]
}

/// The GPU blend constant of a FIX-blended ocean: FIX / 128.
pub fn blend_constant(fix: u8) -> f64 {
	fix as f64 / 128.0
}

/// One vendor glow draw: the vendor position, the ramp factor (pvar +0x90), the two scroll globals, the four blue node
/// glow positions and their colour word (moby +0x90 & 0xffff0000).
#[derive(Clone, Debug)]
pub struct VendorGlow {
	pub pos: [f32; 3],
	pub ramp: f32,
	pub scroll: (f32, f32),
	pub nodes: [[f32; 3]; 4],
	pub word: u32,
}

/// One ship ground shadow: moby +0x10, rows of +0xc0, ship type 0x13e056, alpha after the flight fade, and whether the
/// height is the ground ray (game mode 0x15f5a4 == 6) or pos.z + 0.1.
#[derive(Clone, Copy, Debug)]
pub struct ShipShadow {
	pub pos: [f32; 3],
	pub m: [[f32; 3]; 3],
	pub ship: usize,
	pub alpha: i32,
	pub ground: bool,
}

/// One element of the Gaspar second lava pass as RAM holds it: a triangle strip at lava height, ST already halved.
#[derive(Clone, Debug)]
pub struct RamPatch {
	pub pos: Vec<[f32; 3]>,
	pub uv: Vec<[f32; 2]>,
}

/// One RAM ocean of a field as the renderer draws it (viewer/gpu/sky_water.rs `set_ocean_ram`; R&C1: the oceans of Rilgar,
/// Umbris and Gaspar, retail/oldsim.rs `RamOcean::draw`): the patch strips after the camera distance cull, the fog (near and
/// far scale / offset, the fog colour word, Visibomb's level fog), the alpha fix, the 64x64 texture and the second lava pass
/// (`patches`, drawn in `patch_colour`).
pub struct RamOceanDraw<'a> {
	pub strips: Vec<WorldStrip>,
	pub fog: [f32; 4],
	pub fogcol: u32,
	pub level_fog: bool,
	pub fix: u8,
	pub rgba: &'a [[u8; 4]],
	pub patches: &'a [RamPatch],
	pub patch_colour: [f32; 4],
}
