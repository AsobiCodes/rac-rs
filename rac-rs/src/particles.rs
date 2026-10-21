//! Veldin effect pool (decomp/notes/particles.md): the game's shared 2048-slot sprite pool
//! (FUN_00267248 alloc, FUN_00267398 free, FUN_00267598 update), the effect types Veldin uses
//! (0x17 soft puff, 0x0d fluff, 0x0b spark, 0x45 glint) and the spawn recipes that feed it.
//! Plain data, no GPU: `sprites` and `ribbons` hand the renderer billboards to draw.
//! Anything the notes leave open is a placeholder marked [P]; guesses from the notes are [I].
//! Positions are world units (Ratchet is 1.42 tall). Per-tick values keep the notes' units and
//! the pool runs at 50 Hz (`TICK_HZ`); velocities handed to the caller are per second.

use crate::dynlight::SlotLight;
use std::f32::consts::TAU;
mod blastfx;
mod fume;
pub use fume::Fume;
pub use blastfx::{BlastEmber, BlastSpark, Flame, Puff10, Smoke, SmokeLoft, WaterDebris};
mod spawn;
mod fx;
pub use fx::{arc_frame, arc_world};
mod blast;
mod orbs;
mod orb_tables;
mod wade;
mod waterfall;
mod lights;
mod blasterfx;
pub use blasterfx::{Beam48, Fade27, Flash5, Glow20};
mod tracer;
pub use tracer::Line;
pub use tracer::Tracer31;

/// 0x800 slots shared by every effect type.
pub const CAPACITY: usize = 2048;
/// Pool size float (+0x0c) to the full side of the drawn square, in world units, before the renderer's
/// vz/(vz+0.5) term (decomp/notes/particles.md "Draw size"): the corner sits size/210000 from the centre.
const SIZE_SCALE: f32 = std::f32::consts::SQRT_2 / 210_000.0;
/// Byte +9 (near/far fade) the spawners write: near nibble 4 = 1 m, far nibble x 32 m (particles.md "Draw size").
const B9_PUFF: u8 = 0x74; // 0x17 (FUN_0026c990), 0x45 (FUN_002742d8)
const B9_FLUFF: u8 = 0x44; // 0x0d (FUN_0026b120)
const B9_FAR: u8 = 0xa4; // 0x0b, 0x15, and the crash types 4, 8, 0xb, 0xf
const B9_SPARKLE: u8 = 0x20; // 0x35 (FUN_002719f0: ftoi(0.4) = 0 near)
const B9_RAY: u8 = 0x24; // 0x4e (FUN_0028a620: ftoi(4.0) + 0x20)
const B9_GOLD: u8 = 0x21; // 0x3b / 0x3c (FUN_00287d28 / FUN_00287ea0: ftoi(1.0) + 0x20)
/// Alpha and colour channels run 0..0x80 (0x80 = full).
const FULL: f32 = 128.0;

/// `Sprite::tex` / `Ribbon::tex` hold an EFFECT TYPE id (the pool's type byte), not a texture. The
/// renderer maps it to a particle texture number through the level's def table (`*DAT_001b2080[type]`,
/// first byte of the def = byte +2 of the effect; `Level::particle_def_textures`). Values from
/// `FX_TEX` up are instead fx_textures table indices (arc strands). 0x47 seen in draw code is the GS
/// TEST_1 register, not a texture or an effect type (decomp/notes/particles.md).
pub const TEX_PUFF: u32 = 0x17;
/// Type 0x12, the Suck Cannon victim's trail puff.
pub const TEX_SUCK: u32 = 0x12;
/// Type 0x1c foot-dust puff texture (particles.md CAM-5a).
pub const TEX_DUST: u32 = 0x1c;
pub const TEX_FLUFF: u32 = 0x0d;
pub const TEX_SPARK: u32 = 0x0b;
/// Effect types of the other pool entries drawn: 0x15 ember (FUN_0026c540), 0x13 streak (FUN_0026c078),
/// 0x35 bolt-pickup sparkle (FUN_002719f0).
pub const TEX_EMBER: u32 = 0x15;
pub const TEX_STREAK: u32 = 0x13;
pub const TEX_SPARKLE: u32 = 0x35;
/// Type 0x4e Morph-o-Ray homing ray (WPN-6.7e1).
pub const TEX_RAY4E: u32 = 0x4e;
/// Type 0x4a sand-mouse dust (NOV-14.3b), byte +9 0xa4 (`(int)4.0 - 0x60`).
pub const TEX_MOUSE_DUST: u32 = 0x4a;
const B9_MOUSE_DUST: u8 = 0xa4;
/// Glow quad texture (FUN_00263228 binds FUN_001f9218(0xb) = particle texture NUMBER 11, not effect type
/// 0x0b): `FX_TEX + viewer PART_TEX (0x2000) + 11` (decomp/notes/veldin_ambient.md).
pub const TEX_GLOW: u32 = FX_TEX + 11;
/// Added to an fx_textures index to say "this is not an effect type".
pub const FX_TEX: u32 = 0x1000;
/// Robot arc layers: class 1471 pvar textures 41 (white inner layer) and 40 (blue outer layer, twice as wide); the end glints use
/// particle texture 11 = TEX_GLOW (decomp/notes/veldin_enemies.md, lightning section). 0x47 is a GS register.
pub const TEX_ARC_A: u32 = FX_TEX + 41;
pub const TEX_ARC_B: u32 = FX_TEX + 40;

/// FUN_00263228: additive camera-facing glow quad, half-extent `half`, pulled `bias` towards the camera.
/// `rgba` is the game's word (R low, 0x80 = 1.0 per channel, alpha on top); the Clank eye glow and the red
/// orb glow share it.
pub fn glow(out: &mut Vec<Sprite>, camera: [f32; 3], pos: [f32; 3], half: f32, bias: f32, rgba: u32) {
	let to = sub(camera, pos);
	let l = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
	let at = if l > 1e-4 { add(pos, scale(to, bias / l)) } else { pos };
	let c = rgb_of(rgba);
	let colour = [c[0] / FULL, c[1] / FULL, c[2] / FULL, (rgba >> 24) as f32 / FULL];
	out.push(Sprite { pos: at, size: half * 2.0, rot: 0.0, colour, tex: TEX_GLOW, additive: true , pool: None, flat: false });
}

/// The game's rand() (newlib FUN_001160d8, decomp/notes/title_assets.md): s = s * 1103515245 + 12345, returns
/// s & 0x7fffffff; the game seeds it 12345 once at boot and every subsystem draws from that one sequence. `Rng` is
/// a handle on that shared state, one per thread (the game thread's is the game's; each test starts from the
/// seed). Draws match the game's order only where every consumer of the tick is ported [I].
#[derive(Clone, Copy, Debug, Default)]
pub struct Rng;

/// The game's rand() seed.
pub const RAND_SEED: u32 = 12345;
/// The state both levels' initialisation sets (setter FUN_001160c8); only the frontend resets to `RAND_SEED`.
pub const LEVEL_SEED: u32 = 1234;

thread_local! {
	static RAND: std::cell::Cell<u32> = const { std::cell::Cell::new(RAND_SEED) };
}

impl Rng {
	/// The shared generator.
	pub fn shared() -> Rng {
		Rng
	}
	/// FUN_001160c8: sets the state word. Level entry calls it with `LEVEL_SEED` (decomp/notes/particles.md RNG).
	pub fn reseed(&mut self, seed: u32) {
		RAND.with(|s| s.set(seed));
	}
	/// rand(): 0..0x7fffffff.
	pub fn next(&mut self) -> u32 {
		RAND.with(|s| {
			let v = s.get().wrapping_mul(1_103_515_245).wrapping_add(12345);
			s.set(v);
			v & 0x7fff_ffff
		})
	}
	/// The state word without a draw (PC debug aid: the viewer bench's determinism hash).
	pub fn state(&self) -> u32 {
		RAND.with(|s| s.get())
	}
	/// High-word draw h = (r >> 16) & 0x7fff, one rand().
	fn high(&mut self) -> u32 {
		(self.next() >> 16) & 0x7fff
	}
	/// FUN_00257760(n) / FUN_00212d58: h % n; the draw is consumed even for n == 0.
	pub fn small(&mut self, n: u32) -> u32 {
		let h = self.high();
		if n == 0 { 0 } else { h % n }
	}
	/// FUN_002577f8(a, b): a + float(h) * (b - a) * (1/32768), one draw.
	pub fn float(&mut self, a: f32, b: f32) -> f32 {
		let h = self.high() as f32;
		a + h * (b - a) * (1.0 / 32768.0)
	}
	/// FUN_002577a0(a, b): a + h % (b - a + 1), one draw even for an empty range.
	pub fn int(&mut self, a: i32, b: i32) -> i32 {
		let h = self.high();
		let span = b.wrapping_sub(a).wrapping_add(1);
		if span == 0 { a } else { a.wrapping_add((h as i32).wrapping_rem(span)) }
	}
	/// FUN_00257848(lo, hi): one draw; magnitude lo + float(q) * (hi - lo) / 4096 from the high word's low 12 bits,
	/// negated when high-word bit 0 is set (magnitude and sign are correlated).
	pub fn signed(&mut self, lo: f32, hi: f32) -> f32 {
		let r = self.next() >> 16;
		let a = lo + (r & 4095) as f32 * (hi - lo) / 4096.0;
		if r & 1 != 0 { -a } else { a }
	}
	/// FUN_00257858(0, m): symmetric in (-m, m), one draw.
	pub fn sym(&mut self, m: f32) -> f32 {
		self.signed(0.0, m)
	}
	/// FUN_0026c628: float(q - 2048) * pi / 2048 for q = high-word low 12 bits, one draw.
	pub fn angle(&mut self) -> f32 {
		let q = ((self.next() >> 16) & 4095) as i32;
		(q - 2048) as f32 * std::f32::consts::PI / 2048.0
	}
	/// FUN_0026c678(lo, hi): exactly three draws, angle, angle, float(lo, hi); a point at that radius.
	pub fn unit_vec_len(&mut self, lo: f32, hi: f32) -> [f32; 3] {
		let a = self.angle();
		let b = self.angle();
		let r = self.float(lo, hi);
		[a.cos() * r * b.cos(), a.sin() * r * b.cos(), b.sin() * r]
	}
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn scale(a: [f32; 3], s: f32) -> [f32; 3] { [a[0] * s, a[1] * s, a[2] * s] }
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn len(a: [f32; 3]) -> f32 { dot(a, a).sqrt() }
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: [f32; 3]) -> [f32; 3] {
	let l = len(a);
	if l < 1e-6 { [0.0, 0.0, 1.0] } else { scale(a, 1.0 / l) }
}
/// Packed RGBA (R in the low byte, A in the top) to 0..0x80 channel floats [I].
fn rgb_of(c: u32) -> [f32; 3] {
	[(c & 0xff) as f32, ((c >> 8) & 0xff) as f32, ((c >> 16) & 0xff) as f32]
}

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




/// Runtime moby pool: the spawnable slots of the gameplay moby block (header word 2, 256 on the PAL levels checked:
/// Veldin, Novalis, Aridia), scanned lowest first by the actor allocator (Novalis FUN_00262f28, Veldin FUN_0024e448).
/// Debris, glow spheres, class-122 shards and light flashes share it with no per-class limit; when no slot is
/// eligible the allocator returns null. decomp/notes/effects_lighting_mapping.md section 1.
pub const MOBY_LIMIT: usize = 256;
/// Every class a break can spawn as debris: crate pieces (and their class+1 fragments) and robot parts.
pub const DEBRIS_CLASSES: [u32; 20] = [
	0x95, 0x96, 0x15c, 0x15d, 0x162, 0x163, 0x15f, 0x160, 0x165, 0x166, 0x168, 0x169, 0x16b, 0x16c, 0x16e, 0x16f, 0x171, 0x172, 0x7ab, 0x78d,
];
/// Further robot parts (kept apart so the array above stays a plain list).
pub const DEBRIS_CLASSES_EXTRA: [u32; 2] = [0x78e, 0x78f];

/// Occupancy of the shared 0x800-slot effect pool (bitmap DAT_001b1880, high-water iGpffff9534). One per thread
/// because the Pyrocitor flames (game/pyro.rs) take slots from the same pool as `Particles`. `epoch` changes when a
/// level resets the pool, so a stale holder's release is ignored.
struct Occupancy {
	used: Vec<bool>,
	/// Highest used index + 1.
	high: usize,
	epoch: u32,
}

thread_local! {
	static OCC: std::cell::RefCell<Occupancy> = std::cell::RefCell::new(Occupancy { used: vec![false; CAPACITY], high: 0, epoch: 0 });
}


/// A claimed slot of the shared effect pool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoolSlot {
	pub idx: usize,
	epoch: u32,
}


/// FUN_00267398: frees a slot and lowers the high-water mark past trailing free slots.
pub fn pool_release(s: PoolSlot) {
	OCC.with(|o| {
		let mut o = o.borrow_mut();
		if s.epoch != o.epoch || !o.used[s.idx] {
			return;
		}
		o.used[s.idx] = false;
		while o.high > 0 && !o.used[o.high - 1] {
			o.high -= 1;
		}
	})
}


/// Empties the pool (a level load). Earlier holders' slots are abandoned.
pub fn pool_reset() {
	OCC.with(|o| {
		let mut o = o.borrow_mut();
		o.used.iter_mut().for_each(|u| *u = false);
		o.high = 0;
		o.epoch = o.epoch.wrapping_add(1);
	})
}



/// Type 0x17 soft puff (spawn FUN_0026c990, update FUN_0026cb40).
#[derive(Clone, Debug)]
pub struct Puff {
	pub pos: [f32; 3],
	pub size: f32,
	pub rot: f32,
	pub rgb: [f32; 3],
	/// 0..0x80.
	pub alpha: f32,
	pub additive: bool,
	/// Byte +9, the near/far fade nibbles (default 0x74; NOV-14.5 scene-4 puffs 0x44).
	pub b9: u8,
}


/// Type 0x12 trail puff of a flying Suck Cannon victim (creator FUN_00280d48, update FUN_00280ed0; weapons_framework.md
/// WPN-5.1p). `ph` is +0x28, which the creator never writes [I: 0].
#[derive(Clone, Debug)]
pub struct SuckPuff {
	pub pos: [f32; 3],
	pub size: f32,
	pub rot: u8,
	pub alpha: u8,
}


/// Type 0x1c hero foot-dust puff (ctor FUN_00282808, update FUN_00282ac8; particles.md CAM-5a). `alpha` is a 0..~0.12
/// float that rises with `rate`, bounces at 0.12 and frees at 0.0244 on the way down; `a` is the drawn alpha byte.
#[derive(Clone, Debug)]
pub struct FootPuff {
	pub pos: [f32; 3],
	pub size: f32,
	pub rgb: [f32; 3],
	pub a: u8,
	pub rot: u8,
}


/// Type 0x0d crate-dust fluff (spawn FUN_0026b120, update FUN_0026b2a0).
#[derive(Clone, Debug)]
pub struct Fluff {
	pub pos: [f32; 3],
	pub size: f32,
	pub rot: f32,
	pub rgb: [f32; 3],
	pub alpha: f32,
}



/// PAR-3: type-0x13 slot flags 3 (stretched quad): half-width +0x1c = 0.09 at the head, x +0x2c = 0.5 at the tail
/// (FUN_0026c078).
pub const STREAK_HALF_W: f32 = 0.09;
/// Type id of the joint ribbon (texture byte filled from the level's def table).
pub const TEX_RIBBON37: u32 = 0x37;

/// Type 0x13 streak (spawn FUN_0026c078, update FUN_0026c180): a line from `pos` back to
/// `pos - 2*vel`, moving at `vel` per tick with no gravity or drag; alpha starts at 0x78 and drops
/// 6 per tick (gone after 20 ticks). Red 0x40..0x5e, green and blue 0x40, additive.
#[derive(Clone, Debug)]
pub struct Streak {
	pub pos: [f32; 3],
	pub vel: [f32; 3],
	pub rgb: [f32; 3],
	pub alpha: i32,
}



/// Type 0x37 joint ribbon of the held item class 479 (Novalis ctor FUN_002871b8 / Veldin FUN_00271f68, update
/// FUN_00272068): a two-point ribbon from the joint's world position to a tail trailing behind its motion. No life
/// counter: it lives while its owner does (PAR-4).
#[derive(Clone, Debug)]
pub struct Ribbon37 {
	pub width: f32,
	pub taper: f32,
	pub head: [f32; 3],
	pub tail: [f32; 3],
	/// Colours of the head (+4) and the tail (+0xc).
	pub c1: u32,
	pub c2: u32,
}


/// Type 0x15 ember (spawn FUN_0026c540, update FUN_0026c640): size shrinks linearly to 0 over its
/// life, colour runs start -> end [I direction], vel.z -= 0.00144 per tick, vel *= 0.96. With
/// `trail` set, every other tick skips the drag and drops a trail ember instead (life 12..17).
#[derive(Clone, Debug)]
pub struct Ember {
	pub pos: [f32; 3],
	pub size0: f32,
	pub life: i32,
	pub life0: i32,
	pub c0: [f32; 4],
	pub c1: [f32; 4],
	pub rot: f32,
}

impl Ember {
	fn t(&self) -> f32 {
		self.life as f32 / self.life0.max(1) as f32
	}

}

/// A flat fading quad: crate flashes (FUN_002ac2b8) and type 0x45 glints (FUN_002742d8).
#[derive(Clone, Debug)]
pub struct Quad {
	pub pos: [f32; 3],
	/// Full side in world units (pool glints: already through SIZE_SCALE).
	pub size: f32,
	pub life: i32,
	pub life0: i32,
	pub rgb: [f32; 3],
	pub peak: f32,
	pub rot: f32,
	pub tex: u32,
	pub additive: bool,
	/// Byte +9 for pool sprites (type 0x45 glints); None for the moby-class flash.
	pub pool: Option<u8>,
}


/// PAR-5 type 0x45 glint (spawn FUN_002742d8, update FUN_00274400): moves by `vel` (link mode 0; the link modes
/// 1-4 need robot joint arrays no caller reaches, decomp/notes/particles.md PAR-5), turns a random 0..64 bytes a
/// tick and fades linearly over its life.
#[derive(Clone, Debug)]
pub struct Glint {
	pub pos: [f32; 3],
	/// +0x0c in the pool's size units.
	pub size: f32,
	/// +8 rotation byte.
	pub rot: u8,
	pub rgba: u32,
}


/// Type 0x35 bolt-pickup sparkle (spawn FUN_002719f0, update FUN_00271b80). Size runs from `end`
/// to the mean of `start` and `end` over the first 5 ticks, then down to `start`; alpha 0x7f -> 0x3f
/// -> 0. Moves at a constant velocity, rotates `spin` 1/256 turns per tick, additive (+3 = 0x48).
/// decomp/notes/particles.md "Bolt pickup sparkle".
#[derive(Clone, Debug)]
pub struct Sparkle {
	pub pos: [f32; 3],
	pub rgb: [f32; 3],
	/// Rotation byte (+8, 256 per turn) and its per-tick step (+0x2a, signed).
	pub rot: u8,
	pub size: f32,
	pub alpha: f32,
}


/// WPN-6.7e1: the Morph-o-Ray homing ray, type 0x4e (ctor FUN_0028a620, updater FUN_0028a7b0;
/// decomp/notes/weapons_framework.md "WPN-6.7e1 port spec"). Size and alpha ramp as the type-0x35 sparkle; with a
/// target it bends toward the target prop's position.
#[derive(Clone, Debug)]
pub struct Ray4e {
	pub pos: [f32; 3],
	pub size: f32,
	pub rgb: [f32; 3],
	pub alpha: f32,
	pub rot: u8,
}


#[derive(Clone, Debug)]
enum Effect {
	/// WPN-6.7e1: the Morph-o-Ray homing ray, type 0x4e.
	Ray4e(Ray4e),
	Fume(Fume),
	Sparkle(Sparkle),
	Puff(Puff),
	SuckPuff(SuckPuff),
	FootDust(FootPuff),
	Fluff(Fluff),
	Quad(Quad),
	Streak(Streak),
	Ribbon37(Ribbon37),
	Ember(Ember),
	BlastEmber(BlastEmber),
	/// WPN-6.4e2 type 0x40 water debris.
	WaterDebris(WaterDebris),
	BlastSpark(BlastSpark),
	Flame(Flame),
	Smoke(Smoke),
	Puff10(Puff10),
	SmokeLoft(SmokeLoft),
	Bubble(wade::Bubble),
	Drop(wade::Drop),
	Dust(wade::Dust),
	Ripple(wade::Ripple),
	Wake(wade::Wake),
	WaterfallMist(waterfall::Mist),
	WaterfallFlow(waterfall::Flow),
	/// NOV-8.4: gold bolt glow layer, pool type 0x3b (update FUN_00287e48).
	GoldGlow(GoldSprite),
	/// NOV-8.4: gold bolt spark, pool type 0x3c (update FUN_00288058).
	GoldSpark(GoldSprite),
	/// PAR-5 / BRIDGE-1: type 0x1a follower trail (ctor FUN_00282418, update FUN_00282558).
	Trail(Trail),
	/// WPN-6.1a: type 0x1b fading spark (ctor FUN_00282698, update FUN_00282778).
	Fade27(Fade27),
	/// WPN-6.1b: type 0x48 beam sprite (ctor FUN_00289d08, update FUN_00289e90).
	Beam48(Beam48),
	/// WPN-6.5e2: type 0x20 Glove of Doom capsule glow (ctor FUN_002836a0, update FUN_002837c0).
	Glow20(Glow20),
	/// WPN-6.5e3: type 5 flash (ctor FUN_0027e068, update FUN_0027e168).
	Flash5(Flash5),
	/// PAR-5: type 0x45 glint (spawn FUN_002742d8, update FUN_00274400).
	Glint(Glint),
	/// PAR-2b: type 0x3e nanotech orb sparkle (ctor FUN_00273098, update FUN_00273190).
	OrbSpark(OrbSpark),
	/// NOV-14.3b: sand-mouse dust, pool type 0x4a (spawn FUN_0028a0c0, update FUN_0028a220).
	MouseDust(MouseDust),
	/// WPN-6.2a / BRIDGE-1: type 0x2c smoke (ctor FUN_00285d68; its update and draw are not mapped yet).
	Smoke2c(Smoke2c),
	/// NOV-22.8: Amoeboid slime decal, pool type 0x34 (ctor FUN_00286a70, update FUN_00286ba0).
	Slime(Slime),
	/// KER-92: type 0x31 heli shot tracer (ctor FUN_0025fa48, update FUN_0025fb28), drawn as a line (`lines`).
	Tracer31(Tracer31),
}




/// Type 0x4a dust puff on an anchor (the mouse): the offset `off` is in the anchor's frame.
#[derive(Clone, Debug)]
pub struct MouseDust {
	pos: [f32; 3],
	size: f32,
	rot: u8,
	rgba: [u8; 4],
}


/// WPN-6.2a / 6.2d: the type 0x2c smoke record as the ctor FUN_00285d68 fills it: size +0xc, +0x34 = `grow` * PAL 1.2,
/// +0x38 = (`keep` - 1) * 1.2 + 1, +0x2c `drift`, +0x1c `fade`, velocity +0x20, spin +0x30 (int * 1.2), life +0x3c,
/// alpha +0x3e, rgb +0x31..+0x33, rotation +8 (one draw), blend byte +3 (0x48, the rocket trail rewrites it to 0x44)
/// and the texture def byte +2 (`tex_def`: the def table address the byte is read from, 0x1b2530 by default).
#[derive(Clone, Debug)]
pub struct Smoke2c {
	pub pos: [f32; 3],
	pub size: f32,
	pub rot: u8,
	pub blend: u8,
	pub tex_def: u32,
	/// +4: the colour word, alpha byte rewritten each update.
	pub rgba: u32,
	/// ARI-27b: byte +9, the pool-sprite near/far fade byte (ctor 0x44; the Aridia 1208 rocket writes 0x84).
	pub b9: u8,
}


/// Type 0x1a follower trail: sits on its owner prop (joint -1) and fades over the last fifth of its life.
#[derive(Clone, Debug)]
pub struct Trail {
	pub size: f32,
	/// Colour word (+4); its alpha byte is rewritten by the ramp.
	pub rgba: u32,
	pub rot: u8,
	pub pos: [f32; 3],
}


/// NOV-22.8 type 0x34 slime decal (ctor FUN_00286a70, update FUN_00286ba0): laid at a point, never moves; grows
/// from `s0` to `s1` while its colour fades from `start` to `end`.
#[derive(Clone, Debug)]
pub struct Slime {
	/// +0x10: the spawn point, z lifted 0.05.
	pub pos: [f32; 3],
	/// +0xc: raw size (units * 210000).
	pub size: f32,
	/// +4: the live colour word (R low byte, alpha on top, 0x80 = 1.0).
	pub rgba: u32,
	/// +8 rotation byte.
	pub rot: u8,
}

impl Slime {

	/// The pool draw: byte +3 0x44 (source-alpha-over), +9 0x24, texture of def 0x34, +1 = 1: a mode-1 flat sprite
	/// (NOV-22.12).
	fn sprite(&self) -> Sprite {
		let c = word(self.rgba);
		Sprite { pos: self.pos, size: self.size * SIZE_SCALE, rot: self.rot as f32 * TAU / 256.0, colour: [c[0] / FULL, c[1] / FULL, c[2] / FULL, (c[3] / FULL).clamp(0.0, 1.0)], tex: TEX_SLIME, additive: false, pool: Some(B9_SLIME), flat: true }
	}
}



/// The gold bolt sprites (types 0x3b / 0x3c): world position, per-tick velocity, size * 210000, rot byte, colour
/// bytes (R, G, B, A; 0x80 = 1.0) and the life counter u16 +0x0a.
#[derive(Clone, Debug)]
pub struct GoldSprite {
	pos: [f32; 3],
	size: f32,
	rot: u8,
	rgba: [u8; 4],
	/// Blend byte +3: 0x44 alpha-over (gold glow), 0x48 additive (gold spark, PAR-2c orb glow/spark).
	blend: u8,
}





/// PAR-BRIDGE: an effect a game-side producer created at its call point inside the actor/hero pass: the slot is
/// claimed (FUN_00267248) and the creator's own draws are made there, so claim and RNG order follow retail; the
/// viewer installs it before the pool tick of the same game tick (`Particles::install`).
#[derive(Clone, Debug)]
pub struct Spawn {
}

impl Spawn {







}





/// Class 760 pvars (96 bytes, little-endian), only the fields the puff emitter uses.
#[derive(Clone, Copy, Debug)]
pub struct SmokePvars {
	/// +0x04: index of the emitter's cuboid volume (-1 = delete the moby).
	pub volume: i32,
}

impl SmokePvars {
	pub fn parse(b: &[u8]) -> Option<SmokePvars> {
		if b.len() < 0x44 {
			return None;
		}
		let i32_at = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
		let f32_at = |o: usize| f32::from_bits(i32_at(o) as u32);
		let i16_at = |o: usize| i16::from_le_bytes([b[o], b[o + 1]]);
		Some(SmokePvars {
			volume: i32_at(0x04),
		})
	}
}

/// An emitter's gameplay cuboid: centre plus three half-extent vectors (the volume matrix rows).
#[derive(Clone, Copy, Debug)]
pub struct Cuboid {
	pub centre: [f32; 3],
	pub axes: [[f32; 3]; 3],
}




/// Robot lightning (class 1471, FUN_002e1798): 3 arc slots.
pub const ARC_SLOTS: usize = 3;
/// Half-width at the start of the strand: 0.6 (gp 0.6 in FUN_002e2630, 0.3 for class 28) [C].
const ARC_WIDTH: f32 = 0.6;

#[derive(Clone, Debug)]
struct Arc {
	origin: [f32; 3],
	end: [f32; 3],
	phase_a: f32,
	phase_b: f32,
	strand_a: Vec<[f32; 3]>,
	strand_b: Vec<[f32; 3]>,
	/// This arc's ribbon U scroll: -0.2 * 1.2 per tick while live, wraps +8 (effects_lighting_mapping.md §5).
	uv: f32,
}





/// The effect pool plus the stateful emitters.
pub struct Particles {
	/// Effect contents by pool index; occupancy lives in the shared `OCC` (the flames hold slots too).
	slots: Vec<Option<Effect>>,
	/// Slots of the claims tracked by count only (water rings), most recent last.
	anon_slots: Vec<usize>,
	uv809: f32,
	/// Class 809 state (NOV-14.4 (D)): 0 none placed, 1 init tick pending, 2 running.
	uv809_state: u8,
	arcs: [Option<Arc>; ARC_SLOTS],
	/// Particle texture numbers of effect def 8's 10 flipbook frames (`Level::particle_def_frames`).
	pub flame_frames: [u8; 10],
	/// Def 0x42's two dust frames (`Level::particle_def_frames(0x42, 2)`).
	pub dust_frames: [u8; 2],
	/// The three bytes of effect def 0x10 (`Level::particle_def_frames(0x10, 3)`): texture by mode (mode 3 reads the first).
	pub puff10_frames: [u8; 3],
	/// WPN-6.4e2: the type-0x40 def's frame 1 (water debris texture byte +2).
	pub debris_frame: u8,
	shards: Vec<Shard>,
	fireballs: Vec<Fireball>,
	/// The 8 dynamic light slots (table 0x1802c0): a class-0x27f flash or a moby that holds one owns each.
	lslots: [Option<LightSlot>; LIGHT_SLOTS],
	/// Allocation counter behind `LightSlot::serial`.
	serial: u32,
	/// Frame load fraction DAT_0015f5b4 (timer count / frame ticks); light allocation is refused above 0.8
	/// [PC: no PS2 frame budget, so 0.0 unless a caller sets it].
	pub alloc_load: f32,
	orb_draws: Vec<OrbDraw>,
	/// Type 0x3e's two def frames (`Level::particle_def_frames(0x3e, 2)`): modes 0/2, mode 1.
	pub orb_frames: [u8; 2],
	wanders: Vec<Wander>,
}

impl Particles {
	pub fn new() -> Particles {
		pool_reset();
		crate::game::sched::reset_slots(MOBY_LIMIT + 1);
		Self::detached()
	}

	/// BRIDGE-1: the same fresh instance as `new` without resetting the shared pool occupancy (the placeholder
	/// `Game` holds while it is not lent the viewer's instance).
	pub fn detached() -> Particles {
		Particles {
			slots: vec![None; CAPACITY],
			anon_slots: Vec::new(),
			uv809: 0.0,
			uv809_state: 0,
			arcs: [None, None, None],
			flame_frames: [0; 10],
			dust_frames: [0; 2],
			puff10_frames: [0; 3],
			debris_frame: 0,
			shards: Vec::new(),
			fireballs: Vec::new(),
			lslots: [None; LIGHT_SLOTS],
			serial: 0,
			alloc_load: 0.0,
			orb_draws: Vec::new(),
			orb_frames: [0; 2],
			wanders: Vec::new(),
		}
	}










	/// WPN-6.4e test hook: live effects whose variant is `name` (by its Debug name).
	#[cfg(test)]
	pub(crate) fn effect_count(&self, name: &str) -> usize {
		let head = format!("{name}(");
		self.slots.iter().flatten().filter(|e| format!("{e:?}").starts_with(&head)).count()
	}

	/// WPN-6.4e test hook: the live effects of variant `name` as Debug strings.
	#[cfg(test)]
	pub(crate) fn effects_named(&self, name: &str) -> Vec<String> {
		let head = format!("{name}(");
		self.slots.iter().flatten().map(|e| format!("{e:?}")).filter(|d| d.starts_with(&head)).collect()
	}

	/// NOV-22.8 test hook: the slime decal in slot `i`.
	#[cfg(test)]
	pub(crate) fn slime_at(&self, i: usize) -> Option<&Slime> {
		match self.slots.get(i) {
			Some(Some(Effect::Slime(e))) => Some(e),
			_ => None,
		}
	}






	/// A class 809 moby is placed: its first update is the init branch, the scroll starts the tick after.
	pub fn place_scroll(&mut self) {
		self.uv809_state = 1;
	}

	/// Shared fire-texture UV scroll (class 809, FUN_002a4680).
	pub fn fire_scroll(&self) -> f32 {
		self.uv809
	}











	/// NOV-9.10: the moby pool capacity from the level load (`D - 1`; the allocator keeps one more sentinel slot), the
	/// counter restarting at the new limit. Resets the shared allocator (BRIDGE-15).
	pub fn set_moby_limit(&mut self, limit: usize) {
		crate::game::sched::reset_slots(limit + 1);
		self.anon_slots.clear();
	}






	/// The class-809 shared UV scroll of one game tick: -0.0025 * 1.2, wraps +8 at <= -8 (the fire emitters' texture scroll).
	pub fn tick_scroll(&mut self) {
		match self.uv809_state {
			1 => self.uv809_state = 2,
			2 => {
				self.uv809 -= 0.0025 * 1.2;
				if self.uv809 <= -8.0 {
					self.uv809 += 8.0;
				}
			}
			_ => {}
		}
	}


	// ---- spawners --------------------------------------------------------------------------

	// ---- output ----------------------------------------------------------------------------


	/// `sprites`, plus per sprite its pool slot key (`slot << 8 | n`-th sprite of the slot) for the viewer's render
	/// interpolation (PC-FPS-1); the sprites are the same.
	pub fn sprites_keyed(&self, out: &mut Vec<Sprite>, mut keys: Option<&mut Vec<u32>>) {
		for (si, e) in self.slots.iter().enumerate() {
			let Some(e) = e else { continue };
			let n0 = out.len();
			match e {
				Effect::Sparkle(k) => out.push(Sprite {
					pos: k.pos,
					size: k.size * SIZE_SCALE,
					rot: k.rot as f32 * TAU / 256.0,
					colour: [k.rgb[0] / FULL, k.rgb[1] / FULL, k.rgb[2] / FULL, (k.alpha / FULL).clamp(0.0, 1.0)],
					tex: TEX_SPARKLE,
					additive: true,
					pool: Some(B9_SPARKLE),
					flat: false,
				}),
				Effect::Ray4e(k) => out.push(Sprite {
					pos: k.pos,
					size: k.size * SIZE_SCALE,
					rot: k.rot as f32 * TAU / 256.0,
					colour: [k.rgb[0] / FULL, k.rgb[1] / FULL, k.rgb[2] / FULL, (k.alpha / FULL).clamp(0.0, 1.0)],
					tex: TEX_RAY4E,
					additive: true,
					pool: Some(B9_RAY),
					flat: false,
				}),
				Effect::Puff(p) => out.push(Sprite {
					pos: p.pos,
					size: p.size * SIZE_SCALE,
					rot: p.rot,
					colour: [p.rgb[0] / FULL, p.rgb[1] / FULL, p.rgb[2] / FULL, (p.alpha / FULL).clamp(0.0, 1.0)],
					tex: TEX_PUFF,
					additive: p.additive,
					pool: Some(p.b9),
					flat: false,
				}),
				Effect::SuckPuff(p) => out.push(Sprite {
					pos: p.pos,
					size: p.size * SIZE_SCALE,
					rot: p.rot as f32 * TAU / 256.0,
					colour: [0x40 as f32 / FULL, 0x60 as f32 / FULL, 0x80 as f32 / FULL, (p.alpha as f32 / FULL).clamp(0.0, 1.0)],
					tex: TEX_SUCK,
					additive: true,
					pool: Some(0x44),
					flat: false,
				}),
				Effect::FootDust(p) => out.push(Sprite {
					pos: p.pos,
					size: p.size * SIZE_SCALE,
					rot: p.rot as f32 * TAU / 256.0,
					colour: [p.rgb[0] / FULL, p.rgb[1] / FULL, p.rgb[2] / FULL, (p.a as f32 / FULL).clamp(0.0, 1.0)],
					tex: TEX_DUST,
					additive: true,
					pool: Some(0x44),
					flat: false,
				}),
				Effect::Fluff(p) => out.push(Sprite {
					pos: p.pos,
					size: p.size * SIZE_SCALE,
					rot: p.rot,
					colour: [p.rgb[0] / FULL, p.rgb[1] / FULL, p.rgb[2] / FULL, (p.alpha / FULL).clamp(0.0, 1.0)],
					tex: TEX_FLUFF,
					additive: true, // +3 = 0x48 (FUN_0026b120)
					pool: Some(B9_FLUFF),
					flat: false,
				}),
				Effect::Ember(e) => {
					let t = e.t().clamp(0.0, 1.0);
					let c: Vec<f32> = (0..4).map(|i| (e.c1[i] + (e.c0[i] - e.c1[i]) * t) / FULL).collect();
					out.push(Sprite { pos: e.pos, size: e.size0 * t * SIZE_SCALE, rot: e.rot, colour: [c[0].min(2.0), c[1].min(2.0), c[2].min(2.0), c[3].clamp(0.0, 1.0)], tex: TEX_EMBER, additive: true, pool: Some(B9_FAR), flat: false });
				}
				Effect::WaterfallMist(_) | Effect::WaterfallFlow(_) => Self::waterfall_sprites(e, out),
				Effect::Streak(_) | Effect::Ribbon37(_) | Effect::Tracer31(_) => {}
				// WPN-6.2g1 type 0x2c: generic mode-0 billboard, +9 = 0x44; +3 0x48 additive, 0x44 alpha-over; the texture is
				// the def byte of `tex_def` (0x1b2530 -> 0x2c, 0x1b24dc -> 0x17).
				Effect::Smoke2c(s) => {
					let c = word(s.rgba);
					out.push(Sprite { pos: s.pos, size: s.size * SIZE_SCALE, rot: s.rot as f32 * TAU / 256.0, colour: [c[0] / FULL, c[1] / FULL, c[2] / FULL, (c[3] / FULL).clamp(0.0, 1.0)], tex: (s.tex_def - 0x1b_2480) / 4, additive: s.blend == 0x48, pool: Some(s.b9), flat: false });
				}
				// NOV-22.8 type 0x34, mode-1 flat (NOV-22.12).
				Effect::Slime(e) => out.push(e.sprite()),
				Effect::Fume(f) => out.push(f.sprite()),
				Effect::Bubble(_) | Effect::Drop(_) | Effect::Ripple(_) | Effect::Wake(_) => Self::wade_sprites(e, out),
				// NOV-8.4: a negative size draws |size| point-reflected (rot + 128).
				Effect::GoldGlow(g) | Effect::GoldSpark(g) => out.push(Sprite {
					pos: g.pos,
					size: g.size.abs() * SIZE_SCALE,
					rot: (if g.size < 0.0 { g.rot.wrapping_add(128) } else { g.rot }) as f32 * TAU / 256.0,
					colour: [g.rgba[0] as f32 / FULL, g.rgba[1] as f32 / FULL, g.rgba[2] as f32 / FULL, g.rgba[3] as f32 / FULL],
					tex: TEX_SPARKLE,
					additive: g.blend == 0x48,
					pool: Some(B9_GOLD),
					flat: false,
				}),
				// FUN_002890c8 draw: +3 = 0x48 additive, +9 = 0x22, texture = the def-0x42 frame picked at spawn.
				Effect::Dust(d) => out.push(Sprite {
					pos: d.pos,
					size: d.size * SIZE_SCALE,
					rot: d.rot as f32 * TAU / 256.0,
					colour: [128.0 / FULL, 128.0 / FULL, 128.0 / FULL, (d.alpha as f32 / FULL).clamp(0.0, 1.0)],
					tex: FX_TEX + PART_TEX_BASE + self.dust_frames[d.frame] as u32,
					additive: true,
					pool: Some(0x22),
					// NOV-22.12: type 0x42 is a mode-1 flat sprite (ctor +1 = 1).
					flat: true,
				}),
				Effect::BlastEmber(_) | Effect::BlastSpark(_) | Effect::Flame(_) | Effect::Smoke(_) | Effect::Puff10(_) | Effect::SmokeLoft(_) | Effect::WaterDebris(_) => self.blast_sprites(e, out),
				// PAR-5 type 0x1a: +3 = 0x48 additive, +9 = 0x74.
				Effect::Trail(t) => {
					let c = word(t.rgba);
					out.push(Sprite { pos: t.pos, size: t.size * SIZE_SCALE, rot: t.rot as f32 * TAU / 256.0, colour: [c[0] / FULL, c[1] / FULL, c[2] / FULL, (c[3] / FULL).clamp(0.0, 1.0)], tex: TEX_TRAIL, additive: true, pool: Some(0x74), flat: false });
				}
				// WPN-6.1a type 0x1b: +3 = 0x48 additive, +9 = 0x44.
				Effect::Fade27(f) => {
					let c = word(f.rgba);
					out.push(Sprite { pos: f.pos, size: f.size * SIZE_SCALE, rot: f.rot as f32 * TAU / 256.0, colour: [c[0] / FULL, c[1] / FULL, c[2] / FULL, (c[3] / FULL).clamp(0.0, 1.0)], tex: TEX_FADE27, additive: true, pool: Some(0x44), flat: false });
				}
				// WPN-6.5e2 type 0x20: +3 = 0x48 additive, +9 = 0x44.
				Effect::Glow20(g) => {
					let c = word(g.rgba);
					out.push(Sprite { pos: g.pos, size: g.size * SIZE_SCALE, rot: g.rot_byte as f32 * TAU / 256.0, colour: [c[0] / FULL, c[1] / FULL, c[2] / FULL, (c[3] / FULL).clamp(0.0, 1.0)], tex: TEX_GLOW20, additive: true, pool: Some(0x44), flat: false });
				}
				// WPN-6.5e3 type 5: +3 = 0x48 additive, +9 = 0xa4, rotation 0.
				Effect::Flash5(e) => {
					let c = word(e.rgba);
					out.push(Sprite { pos: e.pos, size: e.size * SIZE_SCALE, rot: 0.0, colour: [c[0] / FULL, c[1] / FULL, c[2] / FULL, (c[3] / FULL).clamp(0.0, 1.0)], tex: TEX_FLASH5, additive: true, pool: Some(B9_FAR), flat: false });
				}
				// WPN-6.1b type 0x48: +3 = 0x48 additive, +9 = 0x44.
				Effect::Beam48(b) => {
					let c = word(b.rgba);
					out.push(Sprite { pos: b.pos, size: b.size * SIZE_SCALE, rot: b.rot as f32 * TAU / 256.0, colour: [c[0] / FULL, c[1] / FULL, c[2] / FULL, (c[3] / FULL).clamp(0.0, 1.0)], tex: TEX_BEAM48, additive: true, pool: Some(0x44), flat: false });
				}
				// PAR-2b type 0x3e: +3 = 0x48 additive, +9 = 0x44, colour 0x7f7f4040 with the live alpha; texture the
				// def's frame for modes 0/2, the second for mode 1.
				Effect::OrbSpark(o) => out.push(Sprite {
					pos: o.pos,
					size: o.size * SIZE_SCALE,
					rot: o.rot as f32 * TAU / 256.0,
					colour: [64.0 / FULL, 64.0 / FULL, 127.0 / FULL, o.alpha as f32 / FULL],
					tex: FX_TEX + PART_TEX_BASE + self.orb_frames[(o.mode == 1) as usize] as u32,
					additive: true,
					pool: Some(0x44),
					flat: false,
				}),
				Effect::Glint(g) => {
					let c = word(g.rgba);
					out.push(Sprite { pos: g.pos, size: g.size * SIZE_SCALE, rot: g.rot as f32 * TAU / 256.0, colour: [c[0] / FULL, c[1] / FULL, c[2] / FULL, (c[3] / FULL).clamp(0.0, 1.0)], tex: TEX_GLOW, additive: true, pool: Some(B9_PUFF), flat: false });
				}
				// NOV-14.3b type 0x4a: +3 = 0x48 additive, +9 = 0xa4.
				Effect::MouseDust(d) => {
					let c = d.rgba.map(|b| b as f32);
					out.push(Sprite { pos: d.pos, size: d.size * SIZE_SCALE, rot: d.rot as f32 * TAU / 256.0, colour: [c[0] / FULL, c[1] / FULL, c[2] / FULL, (c[3] / FULL).clamp(0.0, 1.0)], tex: TEX_MOUSE_DUST, additive: true, pool: Some(B9_MOUSE_DUST), flat: false });
				}
				Effect::Quad(q) => {
					let f = q.life as f32 / q.life0.max(1) as f32;
					out.push(Sprite {
						pos: q.pos,
						size: q.size,
						rot: q.rot,
						colour: [q.rgb[0] / FULL, q.rgb[1] / FULL, q.rgb[2] / FULL, (q.peak * f / FULL).clamp(0.0, 1.0)],
						tex: q.tex,
						additive: q.additive,
						pool: q.pool,
						flat: false,
					});
				}
			}
			if let Some(k) = keys.as_deref_mut() {
				k.extend((n0..out.len()).map(|n| (si as u32) << 8 | (n - n0) as u32));
			}
		}
	}

	/// Arc end glows (FUN_002e2d30, particle texture 11): per end a white alpha-over blob (half-extent
	/// rand(0.3,0.325) at the origin, rand(0.2,0.225) at the end) and an additive 3x halo coloured 0x607f4040.
	pub fn arc_glows(&self, out: &mut Vec<Sprite>) {
		for a in self.arcs.iter().flatten() {
			if a.strand_a.is_empty() {
				continue;
			}
			arc_end_glows(a.origin, a.end, a.phase_a, a.phase_b, out);
		}
	}

	/// Append the live lightning arcs: per arc a wide dim halo strand and a bright core strand.
	pub fn ribbons(&self, out: &mut Vec<Ribbon>) {
		for e in self.slots.iter().flatten() {
			if let Effect::Ribbon37(r) = e {
				let rgba = |w: u32| {
					let c = rgb_of(w);
					[c[0] / FULL, c[1] / FULL, c[2] / FULL, (w >> 24) as f32 / FULL]
				};
				out.push(Ribbon { points: vec![r.head, r.tail], width0: r.width, width1: r.taper, colour: rgba(r.c1), tip_colour: rgba(r.c2), tex: TEX_RIBBON37, uv_scroll: 0.0, additive: true, arc: false });
			}
		}
		// Type 0x13 streaks: head to a tail two ticks behind; widths 0.09 / 0.5 are pos.w / tail.w [I] [P scale].
		for e in self.slots.iter().flatten() {
			if let Effect::Streak(k) = e {
				let c = [k.rgb[0] / FULL, k.rgb[1] / FULL, k.rgb[2] / FULL, k.alpha as f32 / FULL];
				out.push(Ribbon { points: vec![k.pos, sub(k.pos, scale(k.vel, 2.0))], width0: STREAK_HALF_W, width1: STREAK_HALF_W * 0.5, colour: c, tip_colour: c, tex: TEX_STREAK, uv_scroll: 0.0, additive: true, arc: false });
			}
		}
		for a in self.arcs.iter().flatten() {
			if a.strand_a.is_empty() {
				continue;
			}
			arc_ribbons(&a.strand_a, &a.strand_b, a.uv, out);
		}
	}
}

/// Arc end glows (FUN_002e2d30, particle texture 11): per end a white alpha-over blob (half-extent rand(0.3,0.325) at the
/// origin, rand(0.2,0.225) at the end) and an additive 3x halo coloured 0x607f4040; `phase_a` / `phase_b` stand in for the
/// random draws (the port's phases).
pub fn arc_end_glows(origin: [f32; 3], end: [f32; 3], phase_a: f32, phase_b: f32, out: &mut Vec<Sprite>) {
	let r = (phase_b.sin() * 0.5 + 0.5, (phase_a * 3.0).sin() * 0.5 + 0.5);
	for (pos, lo, rr) in [(origin, 0.3, r.0), (end, 0.2, r.1)] {
		let half = lo + 0.025 * rr;
		out.push(Sprite { pos, size: half * 2.0, rot: 0.0, colour: [2.0, 2.0, 2.0, 1.0], tex: TEX_GLOW, additive: false, pool: None, flat: false });
		out.push(Sprite { pos, size: half * 6.0, rot: 0.0, colour: [0.5, 0.5, 1.0, 0.75], tex: TEX_GLOW, additive: true, pool: None, flat: false });
	}
}

/// The ribbons of one lightning arc (FUN_002e2630, per polyline): a white layer (texture 41, alpha 0x80 / 0x30, half-width
/// 0.6 -> 0.05) and a blue layer (0x7f4040 = R .5 G .5 B 1.0 in ABGR, texture 40, alpha 0x40 / 0x10, twice as wide).
pub fn arc_ribbons(strand_a: &[[f32; 3]], strand_b: &[[f32; 3]], uv: f32, out: &mut Vec<Ribbon>) {
	let blue = |alpha: f32| [0x40 as f32 / FULL, 0x40 as f32 / FULL, 127.0 / FULL, alpha / FULL];
	let white = |alpha: f32| [127.0 / FULL, 127.0 / FULL, 127.0 / FULL, alpha / FULL];
	for (pts, a_white, a_blue) in [(strand_a, 128.0, 64.0), (strand_b, 48.0, 16.0)] {
		let (c, b) = (white(a_white), blue(a_blue));
		out.push(Ribbon { points: pts.to_vec(), width0: ARC_WIDTH, width1: 0.05, colour: c, tip_colour: c, tex: TEX_ARC_A, uv_scroll: uv, additive: true, arc: true });
		out.push(Ribbon { points: pts.to_vec(), width0: ARC_WIDTH * 2.0, width1: 0.1, colour: b, tip_colour: b, tex: TEX_ARC_B, uv_scroll: uv, additive: true, arc: true });
	}
}

// ---- Novalis explosion effects (decomp/notes/particles.md "Novalis explosion FUN_00272ea8") ----------

/// Particle texture numbers are uploaded under `PART_TEX_BASE + n` in the viewer's fx table; an effect
/// whose frame is chosen per tick (the flame flipbook) names its texture that way
/// (`FX_TEX + PART_TEX_BASE + n`). Must equal the viewer's `PART_TEX`.
pub const PART_TEX_BASE: u32 = 0x2000;
/// Novalis effect types (the pool's type byte, FUN_0027bdb0): 4 smoke, 8 flame, 0xb spark, 0xf ember.
pub const TEX_SMOKE4: u32 = 0x04;
/// NOV-9.3: the type-0x16 lofting smoke sprite (`particle_def_textures()[0x16]`, *DAT_001b24d8).
pub const TEX_SMOKE_LOFT: u32 = 0x16;
/// PAR-5: the type-0x1a trail texture (texture index = type, as for 4/8/0xf).
pub const TEX_TRAIL: u32 = 0x1a;
/// WPN-6.1a: the type-0x1b spark texture (*DAT_001b24ec).
pub const TEX_FADE27: u32 = 0x1b;
/// WPN-6.5e2: the type-0x20 capsule glow texture (*DAT_001b24a0, index = type).
pub const TEX_GLOW20: u32 = 0x20;
/// WPN-6.5e3: the type-5 flash texture (*DAT_001b2494, index = type).
pub const TEX_FLASH5: u32 = 0x05;
/// WPN-6.1b: the type-0x48 beam texture (*DAT_001b25a0).
pub const TEX_BEAM48: u32 = 0x48;
/// NOV-22.8: the type-0x34 slime decal texture (def 0x34, *DAT_001b2550) and its byte +9 (trunc(4.0) + 0x20).
pub const TEX_SLIME: u32 = 0x34;
const B9_SLIME: u8 = 0x24;

/// FUN_00220960: authored 60 Hz ticks to 50 Hz ticks.
use crate::gameplay::pal_ticks as sc;




/// A retail effect-pool slot as a pool sprite (draw FUN_00267628; decomp/notes/particles.md "Common slot header"): +2
/// texture, +3 ALPHA (0x44 alpha-over, else 0x48 additive), +4 colour word, +8 rotation byte, +9 near/far fade, +0xc
/// size, +0x10 position; flags & 3 == 1 = flat ground quad. Retail mode mirrors slots from RAM with it (retail.rs).
pub fn retail_pool_sprite(pos: [f32; 3], size: f32, rot: u8, rgba: u32, tex: u8, alpha_op: u8, b9: u8, flags: u8) -> Sprite {
	let c = word(rgba);
	Sprite {
		pos,
		size: size * SIZE_SCALE,
		rot: rot as f32 * TAU / 256.0,
		colour: [c[0] / FULL, c[1] / FULL, c[2] / FULL, (c[3] / FULL).clamp(0.0, 1.0)],
		tex: FX_TEX + PART_TEX_BASE + tex as u32,
		additive: alpha_op != 0x44,
		pool: Some(b9),
		flat: flags & 3 == 1,
	}
}

/// A pool colour word (R low byte, alpha on top) as 0..255 channels.
fn word(c: u32) -> [f32; 4] {
	[(c & 0xff) as f32, ((c >> 8) & 0xff) as f32, ((c >> 16) & 0xff) as f32, (c >> 24) as f32]
}


/// Colour channels to a sprite colour (0x80 = 1.0; red may exceed it).
fn tint(c: [f32; 4]) -> [f32; 4] {
	[(c[0] / FULL).min(2.0), (c[1] / FULL).min(2.0), (c[2] / FULL).min(2.0), (c[3] / FULL).clamp(0.0, 1.0)]
}


/// FUN_002776e8: (cos az cos el, sin az cos el, sin el) * m.
pub(crate) fn polar(m: f32, az: f32, el: f32) -> [f32; 3] {
	[az.cos() * m * el.cos(), az.sin() * m * el.cos(), el.sin() * m]
}


/// Class 122 shard moby (spawn FUN_002c4550, update FUN_002c46b8).
#[derive(Clone, Debug)]
struct Shard {
	pos: [f32; 3],
	rot: [f32; 2],
	scale: f32,
}

/// Class 112 fireball moby (spawn FUN_002c1a10, update FUN_002c1bd8): a sphere that grows to `size` over its life
/// while it spins, fading over the second half.
#[derive(Clone, Debug)]
struct Fireball {
	class: u32,
	pos: [f32; 3],
	rot: [f32; 3],
	scale: f32,
	alpha: f32,
	rgb: [f32; 3],
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

pub const FIREBALL_CLASS: u32 = 112;
pub const SHARD_CLASS: u32 = 122;
/// Veldin bomb glow sphere (FUN_002ac2b8's class 1192, the class-112 mesh and update but not pre-grown).
pub const GLOW_CLASS: u32 = 1192;

/// FUN_00272ea8's arguments that vary between its callers.
#[derive(Clone, Copy, Debug)]
pub struct Blast {
}






// ---- nanotech orb (class 806) ----------------------------------------------------------------------

/// PAR-2a: one nanotech orb to draw (FUN_002d91b8): the actor position (no bob), the bob added to the 0.5 centre
/// lift, the parent crate matrix rows (images of the x, y, z axes), the glass while the crate is intact (state 1).
#[derive(Clone, Copy, Debug)]
pub struct OrbDraw {
	pub pos: [f32; 3],
	pub bob: f32,
	pub parent: [[f32; 3]; 3],
	pub glass: bool,
}



/// Type 0x3e sparkle: mode 0 fades in place, mode 1 is a persistent pointer (never fades), mode 2 fades on its
/// parent at `off`. Alpha 127 falls by `step` = trunc(127 / ticks) per update; freed below 1.
#[derive(Clone, Debug)]
pub struct OrbSpark {
	pos: [f32; 3],
	size: f32,
	rot: u8,
	alpha: i32,
	mode: u8,
}



/// FUN_001ff0b0: unit vector, the zero vector for zero length.
fn nz(a: [f32; 3]) -> [f32; 3] {
	let l = len(a);
	if l == 0.0 { [0.0; 3] } else { scale(a, 1.0 / l) }
}


// ---- dynamic point lights ------------------------------------------------------------------------

/// Runtime light slots (table 0x1802c0, 8 x 0x20). decomp/notes/lighting_fog.md section 5b.
pub const LIGHT_SLOTS: usize = 8;




/// Owner ids of the wandering lights' slots (one per placement).
const WANDER_OWNER: u32 = 0x1504_0000;

/// Wandering light, class 1504 (FUN_0030af48, decomp/notes/lighting_fog.md "Users of the dynamic slots"). Pvars:
/// up to 5 colours as float vec4 (/256) at +0x00, hold min/max ticks (i32) at +0x50/+0x54, the colour count at +0x58,
/// active range at +0x5c and light radius at +0x60.
#[derive(Clone, Debug)]
pub struct Wander {
}

impl Wander {
	/// From a placement; None when it has one colour or fewer (init deletes the moby).
	pub fn new(pos: [f32; 3], pvars: &[u8]) -> Option<Wander> {
		let f = |o: usize| pvars.get(o..o + 4).map(|b| [b[0], b[1], b[2], b[3]]);
		let (fl, int) = (|o: usize| f(o).map_or(0.0, f32::from_le_bytes), |o: usize| f(o).map_or(0, i32::from_le_bytes));
		let n = int(0x58).clamp(0, 5) as usize;
		if n <= 1 {
			return None;
		}
		let colours: Vec<[f32; 3]> = (0..n).map(|k| [fl(k * 16) / 256.0, fl(k * 16 + 4) / 256.0, fl(k * 16 + 8) / 256.0]).collect();
		Some(Wander {          })
	}

}

pub const WANDER_CLASS: u32 = 1504;
/// Fire-texture scroll mover (class 809, NOV-14.4 (D)).
pub const SCROLL_CLASS: u32 = 809;



/// Who owns a light slot.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Owner {
	Flash(u32),
	/// A moby that keeps and rewrites its own slot (`hold_light`).
	Held(u32),
}

/// A used entry of the light table (0x1802c0).
#[derive(Clone, Copy, Debug)]
struct LightSlot {
	owner: Owner,
	light: PointLight,
	serial: u32,
}

/// What a light slot holds for the shaders: rgb (1.0 = +128 on a byte), intensity I, position, radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointLight {
	pub pos: [f32; 3],
	pub radius: f32,
	pub rgb: [f32; 3],
	pub intensity: f32,
}

/// FUN_002522c0 / FUN_00252168 refuse while DAT_0015f5b4 exceeds this.
const ALLOC_LOAD_LIMIT: f32 = 0.8;

#[cfg(test)]
mod tests;

/// The glint pool draw (FUN_00220518, decomp/notes/novalis_checklist.md NOV-14.7b-draw): an entry whose centre is outside
/// the view volume is freed, the others emit one camera-facing additive quad of fx texture 0x13 (UV 0..`u`, `u` = 32 texels
/// over the texture width). The retail screen corners are carried to a plane through the glint facing the camera [I].
/// Pure read: the freeing of off-view entries runs once per tick in `Game::glint_cull` (FRAME-1).
pub fn glint_quads(glints:&[crate::game::Glint;16],eye:[f32;3],right:[f32;3],up:[f32;3],fwd:[f32;3],u:f32,out:&mut Vec<WorldQuad>) {
 use crate::camera::{fast_cos,fast_sin};
 for g in glints {if g.life<=0 || glint_off_view(g.pos,eye,right,up,fwd) {continue;}
  let d=sub(g.pos,eye);let vz=dot(d,fwd);
  let dist=(d[0]*d[0]+d[1]*d[1]+d[2]*d[2]).sqrt();
  let r=glint_radius(g.size,g.fade,dist);
  let l8=(r*fast_sin(g.angle)) as i32 as f32/16.0;let l9=(r*fast_cos(g.angle)) as i32 as f32/16.0;
  let (_,vz2)=glint_depth(vz);
  let centre=add(eye,scale(d,vz2/vz));
  // Screen offsets (x right, y down) in pixels to the camera plane: 256/0.63 and 224/(0.63*0.756) px per unit.
  let corner=|dx:f32,dy:f32|add(centre,sub(scale(right,dx*vz2/406.35),scale(up,dy*vz2/470.3)));
  let c=[1.0,1.0,1.0,g.fade as f32/128.0];
  out.push(WorldQuad{pos:[corner(l8,l9),corner(l9,-l8),corner(-l8,-l9),corner(-l9,l8)],uv:[[0.0,0.0],[u,0.0],[u,u],[0.0,u]],colour:[c;4],tex:0x13,additive:true});
 }
}
/// FUN_00220518 view-volume test (vclip via FUN_00220bf0): true when the glint centre is outside near/far or the guard band.
pub(crate) fn glint_off_view(pos:[f32;3],eye:[f32;3],right:[f32;3],up:[f32;3],fwd:[f32;3])->bool {
 let d=sub(pos,eye);let vz=dot(d,fwd);
 let x=dot(d,right);let y=dot(d,up);
 vz<0.03125 || vz>728.1632 || x.abs()>4.0*0.63*vz || y.abs()>4.0*(0.63*0.756)*vz
}
pub(crate) fn glint_radius(size:f32,fade:i16,dist:f32)->f32 {size*(fade+16) as f32*0.015625*(24.0-dist.clamp(2.0,18.0))*16.0}
pub(crate) fn glint_depth(vz:f32)->(i32,f32) {
 let zp=-8388800.0+536860160.0/(vz*1024.0);
 let z=(zp*0.9997+8388576.0) as i32;
 (z,536860160.0/(z as f32+224.0)/1024.0)
}
#[cfg(test)]
mod glint_draw_tests {
 use super::*;
 #[test]fn radius_depth_and_truncated_corners(){
  let r=glint_radius(1.4,60,5.0);assert!((r-505.4).abs()<0.0001);assert_eq!(r as i32,505);
  let a=0.5235988f32;assert_eq!((r*a.sin()) as i32,252);assert_eq!((r*a.cos()) as i32,437);
  assert!((glint_radius(1.4,30,1.0)-354.2).abs()<0.0001);assert!((glint_radius(1.4,30,30.0)-96.6).abs()<0.0001);
  let(z,near)=glint_depth(5.0);assert_eq!(z,107116);assert!(near<5.0);
 }
 /// Test 1 and 6: a glint straight ahead at 5 m, size 1.4, fade 60: corners at +-31.56 px, alpha fade/128, fade 0 still draws.
 #[test]fn emits_the_screen_quad(){
  let mut g=[crate::game::Glint::default();16];
  g[0]=crate::game::Glint{life:5,pos:[0.0,0.0,5.0],fade:60,size:1.4,..Default::default()};
  g[1]=crate::game::Glint{life:5,pos:[0.0,0.0,5.0],fade:0,size:1.4,..Default::default()};
  let mut out=Vec::new();
  glint_quads(&g,[0.0;3],[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0],0.5,&mut out);
  assert_eq!(out.len(),2);let q=&out[0];
  let (_,vz2)=glint_depth(5.0);let k=vz2/406.35;let m=31.5625*k;
  // fan order v0, v1, v3, v2: v0 (0,+31.56 down), v1 (+31.56, 0), v3 (0,-31.56), v2 (-31.56, 0)
  assert!(q.pos[0][0].abs()<1e-4 && (q.pos[0][1]+31.5625*vz2/470.3).abs()<1e-4);
  assert!((q.pos[1][0]-m).abs()<1e-4 && q.pos[1][1].abs()<1e-4);
  assert!((q.pos[3][0]+m).abs()<1e-4);
  assert_eq!(q.uv,[[0.0,0.0],[0.5,0.0],[0.5,0.5],[0.0,0.5]]);
  assert_eq!((q.tex,q.additive,q.colour[0][3]),(0x13,true,60.0/128.0));
  assert_eq!(out[1].colour[0][3],0.0);
 }
}
