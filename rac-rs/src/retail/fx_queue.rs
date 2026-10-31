//! The effects layer read from RAM each field (`RetailFx`, `queue_fx`): the effect pool, the draw-queue callbacks (orbs,
//! blobs, arcs, glows, reticles, ribbons), the vendor glows and the swap words.

use crate::fmath::Fm;
use super::*;

/// The level overlay's first draw-callback queue (FUN_001ebc48 on Eudora; Novalis draw_queue_add 0x21aee0): count
/// 0x15f424 (max 0x40, reset at the tick head, still filled at field end), the fn[] and arg[] arrays, and the
/// level's nanotech orb draw (Eudora FUN_002db2c8, queued by the orb update FUN_002da4a8 when its frustum test
/// passes). Read from each overlay's queue function and its one orb-update call site (decomp/overlays/*/overlay.elf.c).
pub(super) fn orb_queue(level: u32) -> Option<(u32, u32, u32)> {
	const Q: [(u32, u32, u32); 19] = [
		(0x16d880, 0x16d980, 0x2d91b8), // 0 veldin
		(0x16dc80, 0x16dd80, 0x301530), // 1 novalis
		(0x16de00, 0x16df00, 0x2e3bb0), // 2 aridia
		(0x16d900, 0x16da00, 0x2d2450), // 3 kerwan
		(0x16d980, 0x16da80, 0x2db2c8), // 4 eudora
		(0x16dc80, 0x16dd80, 0x30b1b0), // 5 rilgar
		(0x16df80, 0x16e080, 0x2e79e8), // 6 nebula g34
		(0x16d880, 0x16d980, 0x303aa8), // 7 umbris
		(0x16e000, 0x16e100, 0x2facc0), // 8 batalia
		(0x16d980, 0x16da80, 0x2fa3e8), // 9 gaspar
		(0x16dc80, 0x16dd80, 0x2cb528), // 10 orxon
		(0x16e200, 0x16e300, 0x301888), // 11 pokitaru
		(0x16dc80, 0x16dd80, 0x2f93a0), // 12 hoven
		(0x16db00, 0x16dc00, 0x2fbf78), // 13 oltanis orbit
		(0x16df80, 0x16e080, 0x2f5520), // 14 oltanis
		(0x16de00, 0x16df00, 0x2dc978), // 15 quartu
		(0x16dc00, 0x16dd00, 0x2d7cd0), // 16 kalebo iii
		(0x16e100, 0x16e200, 0x2dae18), // 17 veldin orbit
		(0x16e200, 0x16e300, 0x2e32d0), // 18 veldin finale
	];
	/// `Q` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
	const Q_USA: [(u32, u32, u32); 19] = [
		(0x16d880, 0x16d980, 0x2d9810), // 0 veldin
		(0x16dd00, 0x16de00, 0x301c00), // 1 novalis
		(0x16de80, 0x16df80, 0x2e4280), // 2 aridia
		(0x16d980, 0x16da80, 0x2d2b20), // 3 kerwan
		(0x16da00, 0x16db00, 0x2db9a0), // 4 eudora
		(0x16dc80, 0x16dd80, 0x30b820), // 5 rilgar
		(0x16e000, 0x16e100, 0x2e80c0), // 6 nebula g34
		(0x16d900, 0x16da00, 0x304180), // 7 umbris
		(0x16e080, 0x16e180, 0x2fb390), // 8 batalia
		(0x16da00, 0x16db00, 0x2faac0), // 9 gaspar
		(0x16dd00, 0x16de00, 0x2cbc00), // 10 orxon
		(0x16e280, 0x16e380, 0x301f58), // 11 pokitaru
		(0x16dd00, 0x16de00, 0x2f9a70), // 12 hoven
		(0x16db80, 0x16dc80, 0x2fc650), // 13 oltanis orbit
		(0x16df80, 0x16e080, 0x2f5b70), // 14 oltanis
		(0x16de80, 0x16df80, 0x2dd048), // 15 quartu
		(0x16dc80, 0x16dd80, 0x2d83c0), // 16 kalebo iii
		(0x16e180, 0x16e280, 0x2db4f0), // 17 veldin orbit
		(0x16e280, 0x16e380, 0x2e39a0), // 18 veldin finale
	];
	(*crate::retail::disc::pick(&Q, &Q_USA)).get(level as usize).copied()
}

/// The ground blob queue (Eudora moby_shadow FUN_0024cba8 fills it, shadow_queue_draw FUN_001ebec8 draws it; count
/// 0x15f434, max 0x20, reset with the draw queues by FUN_001eac38): 0x20-byte records at the queue-A fn array + 0x800
/// (the table follows `DAT_0015f434 < 0x20` in each overlay; checked on all 19 levels) [C]. Record: +0 x, +4 y,
/// +8 ground z + 0.025, +0xc radius * height fade, +0x10 ground normal. Casters: crates, planted mines, sand mice, ...
fn blob_queue(m: &ee_oracle::Machine, level: u32) -> Vec<([f32; 3], f32, [f32; 3])> {
	let Some((fns, _, _)) = orb_queue(level) else { return Vec::new() };
	let t = fns + 0x800;
	(0..m.rd32(0x15f434).min(0x20)).map(|i| (m.rdv3(t + 0x20 * i), m.rdf(t + 0x20 * i + 0xc), m.rdv3(t + 0x20 * i + 0x10))).collect()
}

/// The nanotech orbs retail draws this field: the queue entries whose fn is the level's orb draw. Each takes what the
/// draw (Eudora FUN_002db2c8) reads: orb position +0x10, bob pvar[0] (pvar = +0x78), the parent crate (pvar +0xc)
/// rows +0xc0 / +0xd0 / +0xe0, the glass while orb +0x20 == 1 (FUN_002d9be0).
pub(super) fn orb_draws(m: &ee_oracle::Machine, level: u32) -> Vec<crate::particles::OrbDraw> {
	let Some((fns, args, orb_fn)) = orb_queue(level) else { return Vec::new() };
	if std::env::var_os("RAC_QUEUE_LOG").is_some() {
		let q = |f: u32, n: u32| (0..m.rd32(n).min(0x40)).map(|i| (m.rd32(f + 4 * i), m.rd32(f + 0x100 + 4 * i))).map(|(f, a)| format!("{f:x}:{a:x}")).collect::<Vec<_>>().join(" ");
		eprintln!("retail: queue A [{}] B [{}]", q(fns, 0x15f424), q(fns + 0x200, 0x15f428));
	}
	(0..m.rd32(0x15f424).min(0x40))
		.filter(|&i| m.rd32(fns + 4 * i) == orb_fn)
		.filter_map(|i| {
			let a = m.rd32(args + 4 * i) & 0x01ff_ffff;
			let pv = m.rd32(a + 0x78) & 0x01ff_ffff;
			if a == 0 || pv == 0 {
				return None;
			}
			let cr = m.rd32(pv + 0xc) & 0x01ff_ffff;
			let parent = if cr != 0 { [m.rdv3(cr + 0xc0), m.rdv3(cr + 0xd0), m.rdv3(cr + 0xe0)] } else { [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] };
			let state = m.rd8(a + 0x20);
			Some(crate::particles::OrbDraw { pos: m.rdv3(a + 0x10), bob: m.rdf(pv), parent, glass: state == 1, })
		})
		.collect()
}

/// The level's effect pool (decomp/notes/particles.md section 1; alloc Eudora FUN_00259a38,
/// Veldin FUN_00267248): the gp-relative word holding the pool base (0x800 slots x 0x40), with the high-water slot count
/// at +8, and the occupancy bitmap (bit per slot). Read from each overlay's alloc (gp 0x166c00; levels 0, 5, 14, 16 keep
/// the gp block 0x40 higher, as `table`).
fn effect_pool(level: u32) -> Option<(u32, u32)> {
	const BITMAP: [u32; 19] = [
		0x1b1880, 0x1b1c80, 0x1b1e00, 0x1b1900, 0x1b1980, 0x1b1d00, 0x1b1f80, 0x1b1880, 0x1b2000, 0x1b1980, 0x1b1c80, 0x1b2200, 0x1b1c80, 0x1b1b00, 0x1b1f80, 0x1b1e00, 0x1b1c80, 0x1b2100,
		0x1b2200,
	];
	/// `BITMAP` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
	const BITMAP_USA: [u32; 19] = [
		0x1b1880, 0x1b1d00, 0x1b1e80, 0x1b1980, 0x1b1a00, 0x1b1d00, 0x1b2000, 0x1b1900, 0x1b2080, 0x1b1a00, 0x1b1d00, 0x1b2280, 0x1b1d00, 0x1b1b80, 0x1b1f80, 0x1b1e80, 0x1b1d00, 0x1b2180,
		0x1b2280,
	];
	let bm = *(*crate::retail::disc::pick(&BITMAP, &BITMAP_USA)).get(level as usize)?;
	Some((if matches!(level, 0 | 5 | 14 | 16) { va(0x16012c) } else { va(0x1600ec) }, bm))
}

/// What the field draws besides mobys and orbs in normal mode: the whole effect pool (draw FUN_00267628 / Eudora
/// FUN_00259e18) and the draw-queue callbacks that are glow quads or landing markers.
#[derive(Clone, Debug, Default)]
pub struct RetailFx {
	/// Pool slots in mode 0 (point billboard) and 1 (flat ground quad), as pool sprites.
	pub sprites: Vec<crate::particles::Sprite>,
	/// The pool slot index of each entry of `sprites` (the draw sorts every mode by depth, then falling slot index).
	pub sprite_slots: Vec<u16>,
	/// The moby each entry of `sprites` rides on: pool slot +0x20, the owner of a glow such as the Glove of Doom capsule's
	/// (type 0x20, weapons_framework.md WPN-6.5e2), masked as a pointer; compared only with hero-part addresses (retail/strafe.rs).
	pub sprite_owners: Vec<u32>,
	/// The effect type (pool slot byte 0, particles.md section 1) of each entry of `sprites`.
	pub sprite_types: Vec<u8>,
	/// Pool slots in mode 2 (untextured GS line, +0x10 colour +4 to +0x20 colour +0xc).
	pub lines: Vec<crate::particles::Line>,
	/// The pool slot index of each entry of `lines`.
	pub line_slots: Vec<u16>,
	/// Pool slots in mode 3 (two-point ribbon), drawn by `pool_ribbon_quad`.
	pub ribbons: Vec<PoolRibbon>,
	/// Glow quads (FUN_00255a98 calls of the hero glow callback Eudora FUN_001fade8): (pos, size, bias, rgba).
	pub glows: Vec<([f32; 3], f32, f32, u32)>,
	/// How many of `glows` the hero glow callback made (`hero_glows`, first in the list); the rest are world glows (`queue_q1::fill`).
	pub hero_glows: usize,
	/// [PC] Modern camera: the indices of `glows` of the held Pyrocitor (queued by its updater, or near its grip: the pilot light; retail/strafe.rs
	/// `turn_parts`), carried with the drawn gun (viewer/smooth.rs `hero_glow`).
	pub gun_glows: Vec<usize>,
	/// [PC] Modern camera: the mobys retail placed on the hero from its own pose this field (retail/strafe.rs `turn_parts`: Clank,
	/// the pack, the bomb in the Bomb Glove, the Glove of Doom capsule...), carried onto the drawn pose (viewer/smooth.rs `attach`).
	pub hero_parts: Vec<u32>,
	/// Landing markers (bomb / Glove of Doom reticle draws, Eudora FUN_0029eec8 / FUN_002bc450): pvar +0x10 and +0x20.
	pub reticles: Vec<crate::game::Reticle>,
	/// Frame counter 0x15f5ac (the markers turn once per scale_ticks(180) = 150 frames).
	pub frames: u32,
	/// Ported queue-A callbacks (Taunter rings, tube fade) and queue-B callbacks (`queue_b::fill`), in queue order.
	pub qa: Vec<crate::particles::WorldQuad>,
	pub qb: Vec<crate::particles::WorldQuad>,
	/// The queued Swingshot rope (`queue_b::swing_rope`) and the `qa` length at its queue slot: the viewer builds it with
	/// the posed held attach point 0 (`queue_b::rope_quads`) and draws it between `qa[..n]` and `qa[n..]`.
	pub rope: Option<(usize, queue_b::SwingRope)>,
	/// Fogged alpha quads of the queue-B water callbacks (Drek's Fleet hangar water, `queue_fxa::fleet_water`), drawn with the water.
	pub qw: Vec<crate::particles::WorldQuad>,
	/// The screen filter rects (`queue_b::screen_filter`, the Visibomb remote view), drawn last over the overlay.
	pub screen: Vec<queue_b::ScreenRect>,
	/// The fog block and pool far clamp this field draws with: the previous field's end-of-frame swap (`SwapWords`).
	pub swap: Option<SwapWords>,
	/// The shrub draw-distance cap (m), `DAT_00160424` (`DAT_00160464` in levels 0, 5, 14, 16): 500.0 at level setup,
	/// 144.0 in the Visibomb render snapshot (Eudora FUN_002cac68 twin, decomp/notes/weapons_framework.md:1529) [C].
	pub shrub_cap: Option<f32>,
	/// The RC range static (`queue_b::rc_static`, queue B at the Visibomb range limit), drawn under the overlay.
	pub rc: Option<queue_b::RcStatic>,
	/// Ship ground shadows (draw-queue callback Eudora FUN_0027ee90 = Novalis FUN_002a1a60, NOV-14.5d).
	pub ship_shadows: Vec<ShipShadow>,
	/// Ground blob shadows as retail queued them this field (count 0x15f434, max 0x20, records `blob_queue`): centre (z =
	/// ground + 0.025), half-size, ground normal; drawn by `shadows::blob_square` (Eudora FUN_001ebec8).
	pub blobs: Vec<([f32; 3], f32, [f32; 3])>,
	/// The hero's position (0x13f3d0, as retail/strafe.rs `HERO_POS`): the water pass's contact rings (viewer/waterfx.rs `Rings`).
	pub hero: Option<[f32; 3]>,
	/// [PC] The drawn mobys (address, position, bounding sphere radius) for the water pass's wakes (viewer/waterfx.rs
	/// `Wakes`); set by `snap`.
	pub movers: Vec<(u32, [f32; 3], f32)>,
	/// The 8 dynamic light slots (`dyn_lights`), index = slot.
	pub lights: [Option<crate::particles::PointLight>; 8],
	/// Level-light slots 12..15 as retail RAM holds them (`reserved_lights`); None when the table is not found.
	pub slots: Option<[[[f32; 4]; 4]; 4]>,
	/// The water patches retail's patch draw (Novalis FUN_002b8af8 and its five twins) shows this field: the light vector and the patches.
	pub water: Option<([f32; 3], Vec<crate::water::RamPatch>)>,
	/// Gadgetron vendor glows (class 11 in states 1 and 2, draw callback Novalis FUN_002ba2f0 on queue B).
	pub vendors: Vec<VendorGlow>,
	/// Robot lightning arcs (class 1471 helper, draw callback Veldin FUN_002e2498 on queue A), world strands.
	pub arcs: Vec<ArcDraw>,
	/// Class UV scrolls retail holds in RAM (`asset_feed::class_uv`): (class id, (du, dv) in texture units).
	pub class_uv: Vec<(u32, [f32; 2])>,
	/// The glint pool (pickup twinkles, `glint_pool_draw` twins) as retail RAM holds it (`oldsim::glints`).
	pub glints: [crate::game::Glint; 16],
	/// Aridia's sky sprite records (`oldsim::sky_sprites`), empty on other levels.
	pub sky_sprites: Vec<crate::sky::SkySprite>,
	/// The oceans of Rilgar, Umbris and Gaspar as RAM holds them (`oldsim::oceans`); Batalia and Oltanis come from `ocean` (queue_q1), Aridia and Kerwan from `ocean.rs`.
	pub oceans: Vec<oldsim::RamOcean>,
	/// FIX-blend world strips of the Novalis strip-surface callbacks (`queue_q1::fill`).
	pub strips: Vec<crate::particles::WorldStrip>,
	/// The Hoven water pools of queue C (`queue_q1::fill`); retail flushes queue C before the shrub pass.
	pub pools: Vec<crate::particles::WorldStrip>,
	/// The level's ocean built from RAM while its queue-A wrapper is queued (`queue_q1::ocean`; Batalia, Oltanis).
	pub ocean: Option<std::sync::Arc<crate::ocean::Ocean>>,
}

/// One live lightning arc: its origin and end, both strands in world space, the ribbon U scroll as the draw used it and
/// the two phases (the glow size stand-ins).
#[derive(Clone, Debug)]
pub struct ArcDraw {
	pub origin: [f32; 3],
	pub end: [f32; 3],
	pub strand_a: Vec<[f32; 3]>,
	pub strand_b: Vec<[f32; 3]>,
	pub uv: f32,
	pub phase_a: f32,
	pub phase_b: f32,
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

/// The vendor glow draws of this field (queue B entries of the level's FUN_002ba2f0 twin): node k sits at (1.1 cos a, 1.1 sin a,
/// 0.59) with a = k * pi / 2 - pi in the vendor frame (basis rows +0xc0 / +0xd0 / +0xe0, then + pos +0x10) [C: callback tail].
fn vendor_glows(m: &ee_oracle::Machine, level: u32) -> Vec<VendorGlow> {
	let Some(scroll) = queue_b::vendor_scrolls(m, level) else { return Vec::new() };
	queue_b::vendor_mobys(m, level)
		.into_iter()
		.filter_map(|v| {
			let pv = m.rd32(v + 0x78) & 0x01ff_ffff;
			if pv == 0 {
				return None;
			}
			let pos = m.rdv3(v + 0x10);
			let rows = [0, 1, 2].map(|r| m.rdv3(v + 0xc0 + 0x10 * r));
			let nodes = std::array::from_fn(|k| {
				let a = k as f32 * std::f32::consts::PI * 0.5 - std::f32::consts::PI;
				let l = [a.fcos() * 1.1, a.fsin() * 1.1, 0.59];
				std::array::from_fn(|c| pos[c] + l[0] * rows[0][c] + l[1] * rows[1][c] + l[2] * rows[2][c])
			});
			Some(VendorGlow { pos, ramp: m.rdf(pv + 0x90), scroll, nodes, word: m.rd32(v + 0x90) & 0xffff_0000 })
		})
		.collect()
}

/// The live robot arcs (FUN_002e2498 draws the slots with an owner and state > 1): strands from the polylines in RAM, which are
/// in strand space (along, sideways, up) around the slot's origin (matrix translation) and end, mapped by the tick's frame
/// (`particles::arc_frame`); UV scroll and phases from the slot tables. The 19 points FUN_002e1bf8 writes are used [C].
fn arc_draws(m: &ee_oracle::Machine, level: u32) -> Vec<ArcDraw> {
	let Some(f) = queue_b::arc_live(m, level) else { return Vec::new() };
	let Some(t) = queue_b::arc_tables(m, f) else { return Vec::new() };
	let mut out = Vec::new();
	for i in 0..3u32 {
		let (state, owner) = (m.rd32(t.state + 4 * i), m.rd32(t.owner + 4 * i));
		if owner == 0 || !(2..=4).contains(&state) {
			continue;
		}
		let origin = m.rdv3(t.poly - 0xf0 + 0x40 * i);
		let end = m.rdv3(t.poly - 0x60 + 0x10 * i);
		let frame = crate::particles::arc_frame(origin, end);
		let strand = |base: u32| (0..19u32).map(|k| crate::particles::arc_world(origin, &frame, m.rdv3(base + 0x10 * k))).collect::<Vec<_>>();
		let (strand_a, strand_b) = (strand(t.poly + 0x280 * i), strand(t.poly + 0x140 + 0x280 * i));
		// Phases: A / B at the 0x161bc8 / 0x161bd8 block, 0x161bc8 - 0x161bb8 = 0x10 above the state words.
		out.push(ArcDraw { origin, end, strand_a, strand_b, uv: m.rdf(t.uv + 4 * i), phase_a: m.rdf(t.state + 0x10 + 4 * i), phase_b: m.rdf(t.state + 0x20 + 4 * i) });
	}
	out
}

/// The level's Mine Glove landing-marker draw (Eudora FUN_0029b780, queued by the class-74 mine update FUN_0029bdd8): the
/// bomb marker's body (`q.py diff` with FUN_0029eec8: only the spin vector lives in a global, 0x161330) on pvar +0x10 /
/// +0x20. Binary twins by twins.py on 8 levels; 0 = no identical twin (not mirrored there).
const MINE_RETICLE_FN: [u32; 19] = [0x2a95f8, 0, 0, 0, 0x29b780, 0, 0, 0x2d2688, 0, 0x2c85a0, 0x29ae30, 0, 0, 0x2be018, 0, 0x29f0f8, 0, 0, 0x2abde8];

/// `MINE_RETICLE_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const MINE_RETICLE_FN_USA: [u32; 19] = [0x2a9c50, 0, 0, 0, 0x29be58, 0, 0, 0x2d2d60, 0, 0x2c8c78, 0x29b508, 0, 0, 0x2be6f0, 0, 0x29f7c8, 0, 0, 0x2ac4b8];

/// The level's class-203 landing-marker draw (Eudora FUN_002b5ba8, queued by FUN_002b64e0): the bomb marker's body
/// (`q.py diff:0x29eec8@eudora:0x2b5ba8@eudora`: only the pvar offsets differ) on pvar +0x20 (centre) / +0x30 (normal).
/// [C] Binary twins by twins.py on all 19 levels.
const MARKER203_FN: [u32; 19] = [
	0x2c2b68, 0x2d8758, 0x2c2500, 0x2b4558, 0x2b5ba8, 0x2ee7d8, 0x2cfcf0, 0x2ebbf8, 0x2cbaf8, 0x2e38a8, 0x2b43a8, 0x2e88d8, 0x2daed0, 0x2dc908, 0x2d1790,
	0x2be108, 0x2bd3e0, 0x2c0b28, 0x2c5358,
];

/// `MARKER203_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const MARKER203_FN_USA: [u32; 19] = [
	0x2c31c0, 0x2d8e28, 0x2c2bd0, 0x2b4c28, 0x2b6280, 0x2eee48, 0x2d03c8, 0x2ec2d0, 0x2cc1c8, 0x2e3f80, 0x2b4a80, 0x2e8fa8, 0x2db5a0, 0x2dcfe0, 0x2d1de0,
	0x2be7d8, 0x2bdad0, 0x2c1200, 0x2c5a28,
];

/// The level's ship ground shadow callback (Eudora FUN_0027ee90; twins by twins.py, level order as `orb_queue`).
const SHIP_SHADOW_FN: [u32; 19] = [
	0x28e018, 0x2a1a60, 0x28d710, 0x27a8e8, 0x27ee90, 0x2b6750, 0x29b090, 0x2b4c40, 0x296328, 0x2aa8e8, 0x27ea98, 0x2b0e98, 0x2a4828, 0x2983a8, 0x294398,
	0x27bee8, 0x286e58, 0x2861f8, 0x28db48,
];

/// `SHIP_SHADOW_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const SHIP_SHADOW_FN_USA: [u32; 19] = [
	0x28e670, 0x2a2130, 0x28dde0, 0x27afb8, 0x27f568, 0x2b6dc0, 0x29b768, 0x2b5318, 0x2969f8, 0x2aafc0, 0x27f170, 0x2b1568, 0x2a4ef8, 0x298a80, 0x2949e8,
	0x27c5b8, 0x287548, 0x2868d0, 0x28e218,
];

/// The words the frame's last step writes for the next frame's draw (Novalis FUN_00218c70 OV:9936-9971 = Eudora
/// overlay.elf.c 9970-9995, called last in the frame function) [C]: camera water flag 0x167414 clear -> the live fog words
/// 0x15f444..0x15f454 (zone fog FUN_002101b8, the Visibomb snapshot) and clamp 0x1f4000, set -> the underwater words
/// 0x161184.. and 0x40000. The fog block is Novalis 0x16d058 near, +4 far, +0x10 near intensity, +0x14 far intensity,
/// +0x18/+0x1c/+0x20 colour bytes, at the camera level shift (every overlay's copy checked, 2026-10-12); the clamp is
/// 0x1600fc (0x16013c in levels 0, 5, 14, 16). The draw of a frame precedes the swap, so it uses the previous frame's
/// words (decomp/notes/water.md NOV-28.2) [C].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwapWords {
	pub fog_rgb: [u8; 3],
	/// Near and far in metres (raw / 1024, as `gameplay::Lighting::fog`), then the two GS fog intensities.
	pub fog: [f32; 4],
	/// The pool particle far clamp in metres (raw / 4096, as `water::PART_FAR`).
	pub part_far: f32,
	/// [PC] The camera water flag itself (0x167414 at the level shift: the camera is below a water face, or the hero swims
	/// underwater): what picks the underwater words above. The PC underwater look reads it (viewer/underwater.rs).
	pub under: bool,
	/// [PC] The hero's water surface height (boot word 0x13f640, the water face under the hero) while the hero swims (group
	/// word 0x1413dc = 0x11 underwater or 0x12 at the surface): the surface the underwater look measures its depth from.
	pub surf: Option<f32>,
}

/// [PC] Dev aid: RAC_UNDER_LOG=1 prints the camera water flag and its proof words every field.
fn under_log() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var_os("RAC_UNDER_LOG").is_some())
}

impl SwapWords {
	pub(super) fn read(m: &ee_oracle::Machine, level: u32) -> Option<SwapWords> {
		let b = va(0x16d058u32.wrapping_add_signed(*LEVEL_SHIFT.get(level as usize)?));
		let small = if matches!(level, 0 | 5 | 14 | 16) { 0x40 } else { 0 };
		let part_far = m.rd32(va(0x1600fc + small)) as i32 as f32 / 4096.0;
		if under_log() {
			let flag = m.rd32(va(0x167414u32.wrapping_add_signed(*LEVEL_SHIFT.get(level as usize)?)));
			eprintln!("UNDER_LOG level {level} flag {flag:#x} part_far {part_far} fog_far {} grp {:#x} f640 {} state {:#x} hero_z {}", m.rdf(b + 4) / 1024.0, m.rd32(0x1413dc), m.rdf(0x13f640), m.rd32(0x1413d4), m.rdf(0x13f3d8));
		}
		Some(SwapWords {
			fog_rgb: [m.rd32(b + 0x18) as u8, m.rd32(b + 0x1c) as u8, m.rd32(b + 0x20) as u8],
			fog: [m.rdf(b) / 1024.0, m.rdf(b + 4) / 1024.0, m.rdf(b + 0x10), m.rdf(b + 0x14)],
			part_far,
			under: m.rd32(va(0x167414u32.wrapping_add_signed(*LEVEL_SHIFT.get(level as usize)?))) & 0xff != 0 && (part_far - 64.0).abs() < 0.5,
			surf: matches!(m.rd32(0x1413dc), 0x11 | 0x12).then(|| m.rdf(0x13f640)).filter(|z| z.is_finite()),
		})
	}
}

/// The field's draw takes the previous field's swap words; `lag` keeps this field's for the next.
pub(super) fn lag_swap(fx: &mut RetailFx, lag: &mut Option<SwapWords>) {
	let now = fx.swap;
	if lag.is_some() {
		fx.swap = *lag;
	}
	*lag = now;
}

/// A mode-3 pool slot: head +0x10, tail +0x20, half-width +0x1c, tail scale +0x2c, colours +4 (head) / +0xc (tail),
/// particle texture +2, additive unless ALPHA +3 == 0x44, fade/cull byte +9, and its pool slot index.
#[derive(Clone, Copy, Debug)]
pub struct PoolRibbon {
	pub head: [f32; 3],
	pub tail: [f32; 3],
	pub half: f32,
	pub scale: f32,
	pub c_head: u32,
	pub c_tail: u32,
	pub tex: u8,
	pub additive: bool,
	pub b9: u8,
	pub slot: u16,
}

/// The level's hero glow draw callback (Eudora FUN_001fade8; twins by twins.py, level order as `orb_queue`).
const GLOW_FN: [u32; 19] = [
	0x207888, 0x228f08, 0x218060, 0x200998, 0x1fade8, 0x235df0, 0x21f668, 0x22f670, 0x219b80, 0x2274e8, 0x1fae70, 0x238960, 0x229ff0, 0x2132f0, 0x21a778,
	0x200688, 0x205868, 0x204760, 0x20fd00,
];

/// `GLOW_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const GLOW_FN_USA: [u32; 19] = [
	0x207d40, 0x229440, 0x218598, 0x200ed0, 0x1fb320, 0x2362a8, 0x21fba0, 0x22fba8, 0x21a0b8, 0x227a20, 0x1fb3a8, 0x238e98, 0x22a528, 0x213828, 0x21ac30,
	0x200bc0, 0x205da0, 0x204c98, 0x210238,
];

/// The level's two landing-marker draws (bomb and Glove of Doom; one body, Eudora FUN_0029eec8 / FUN_002bc450).
const RETICLE_FN: [[u32; 2]; 19] = [
	[0x2ac598, 0x2c77a0],
	[0x2c1cf0, 0x2dd390],
	[0x2abf30, 0x2c7138],
	[0x29df88, 0x2b9190],
	[0x29eec8, 0x2bc450],
	[0x2d7950, 0x2f3410],
	[0x2b9288, 0x2d4928],
	[0x2d5628, 0x2f0830],
	[0x2b4ad8, 0x2d0730],
	[0x2cb540, 0x2e84e0],
	[0x29ddd8, 0x2b8fe0],
	[0x2d2308, 0x2ed510],
	[0x2c42b0, 0x2dfb08],
	[0x2c4ba0, 0x2e2748],
	[0x2bb1c0, 0x2d7bb0],
	[0x2a3ad8, 0x2c33d8],
	[0x2a6e10, 0x2c2018],
	[0x2a93a8, 0x2c5e58],
	[0x2aed88, 0x2c9f90],
];

/// `RETICLE_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const RETICLE_FN_USA: [[u32; 2]; 19] = [
	[0x2acbf0, 0x2c7df8],
	[0x2c23c0, 0x2dda60],
	[0x2ac600, 0x2c7808],
	[0x29e658, 0x2b9860],
	[0x29f5a0, 0x2bcb28],
	[0x2d7fc0, 0x2f3a80],
	[0x2b9960, 0x2d5000],
	[0x2d5d00, 0x2f0f08],
	[0x2b51a8, 0x2d0e00],
	[0x2cbc18, 0x2e8bb8],
	[0x29e4b0, 0x2b96b8],
	[0x2d29d8, 0x2edbe0],
	[0x2c4980, 0x2e01d8],
	[0x2c5278, 0x2e2e20],
	[0x2bb810, 0x2d8200],
	[0x2a41a8, 0x2c3aa8],
	[0x2a7500, 0x2c2708],
	[0x2a9a80, 0x2c6530],
	[0x2af458, 0x2ca660],
];

/// Clank's bulb glow in the list `hero_glows` makes (the slot-3 item light: size bits 0x3d6978d5, colour 0x280000c0).
pub fn is_bulb_glow(g: &([f32; 3], f32, f32, u32)) -> bool {
	g.1.to_bits() == 0x3d69_78d5 && g.3 == 0x2800_00c0
}

/// The hero glow callback (Eudora FUN_001fade8, OV:21972-22057) in gameplay modes (not 2 / 6), by the back-slot kind
/// byte 0x1413f4: 3 = three glows at 0x1410a0.. (size 0.2, rgba 0x141634); 0 = the slot-3 item light
/// (FUN_00200540(3) = 0x1404f8 while 0x1404f4 == 2; kind 2 or 3 and 0x141628 == 0: size 0x3d6978d5, z + 0.015,
/// 0x280000c0), then the shared glow list 0x141120 (count i16 0x141200; per entry size 0x1411c0, bias 0x1411e0,
/// rgba 0x1411a0); 2 = one glow at z - 0.07 (size 0.25, 0x300000c0); 1 = z - 0.02 (0.01 when 0x15edb3 != 0), size the
/// eased word 0x14097c, rgba 0x141650. Bias 0x3da3d70a (0.08) except the list's own.
pub(super) fn hero_glows(m: &ee_oracle::Machine, out: &mut Vec<([f32; 3], f32, f32, u32)>) {
	let bias = f32::from_bits(0x3da3_d70a);
	let at = |dz: f32| {
		let mut p = m.rdv3(0x1410d0);
		p[2] += dz;
		p
	};
	match m.rd8(0x1413f4) {
		3 => {
			let w = m.rd32(0x141634);
			for i in 0..3 {
				out.push((m.rdv3(0x1410a0 + 0x10 * i), f32::from_bits(0x3e4c_cccd), bias, w));
			}
		}
		0 => {
			let kind = if m.rd32(0x1404f4) == 2 { m.rd32(0x1404f8) } else { u32::MAX };
			if (kind == 3 || kind == 2) && m.rd16(0x141628) == 0 {
				out.push((at(0.015), f32::from_bits(0x3d69_78d5), bias, 0x2800_00c0));
			}
			for i in 0..(m.rd16(0x141200) as i16).max(0) as u32 {
				out.push((m.rdv3(0x141120 + 0x10 * i), m.rdf(0x1411c0 + 4 * i), m.rdf(0x1411e0 + 4 * i), m.rd32(0x1411a0 + 4 * i)));
			}
		}
		2 => out.push((at(-0.07), 0.25, bias, 0x3000_00c0)),
		1 => out.push((at(if m.rd8(0x15edb3) == 0 { -0.02 } else { -0.01 }), m.rdf(0x14097c), bias, m.rd32(0x141650))),
		_ => {}
	}
}

/// A pool colour word (R low byte, alpha on top) as 0..1 channels (0x80 = 1.0).
pub(super) fn pool_rgba(w: u32) -> [f32; 4] {
	[(w & 0xff) as f32 / 128.0, (w >> 8 & 0xff) as f32 / 128.0, (w >> 16 & 0xff) as f32 / 128.0, (w >> 24) as f32 / 128.0]
}

pub(super) fn queue_fx(m: &ee_oracle::Machine, level: u32) -> RetailFx {
	let mut out = RetailFx { frames: m.rd32(va(oldsim::FRAME_COUNTER)), ..RetailFx::default() };
	out.lights = dyn_lights(m, level);
	out.slots = reserved_lights(m, level);
	out.water = water_patches(m, level);
	out.vendors = vendor_glows(m, level);
	out.arcs = arc_draws(m, level);
	out.class_uv = asset_feed::class_uv(m, level);
	out.glints = oldsim::glints(m, level);
	out.sky_sprites = oldsim::sky_sprites(m, level);
	out.oceans = oldsim::oceans(m, level, out.frames);
	if let Some((g, bm)) = effect_pool(level) {
		let base = m.rd32(g) & 0x01ff_ffff;
		let hw = m.rd32(g + 8);
		if base != 0 && (hw as i32) >= 0 {
			for i in 0..=hw.min(0x7ff) {
				let s = base + 0x40 * i;
				let fl = m.rd8(s + 1);
				if m.rd8(bm + (i >> 3)) >> (i & 7) & 1 == 0 || fl & 0x80 != 0 {
					continue;
				}
				match fl & 3 {
					0 | 1 => {
						out.sprites.push(crate::particles::retail_pool_sprite(m.rdv3(s + 0x10), m.rdf(s + 0xc), m.rd8(s + 8), m.rd32(s + 4), m.rd8(s + 2), m.rd8(s + 3), m.rd8(s + 9), fl));
						out.sprite_slots.push(i as u16);
						out.sprite_owners.push(m.rd32(s + 0x20) & 0x01ff_ffff);
						out.sprite_types.push(m.rd8(s));
					}
					2 => {
						let b9 = m.rd8(s + 9);
						out.line_slots.push(i as u16);
						out.lines.push(crate::particles::Line {
							a: m.rdv3(s + 0x10),
							b: m.rdv3(s + 0x20),
							col_a: pool_rgba(m.rd32(s + 4)),
							col_b: pool_rgba(m.rd32(s + 0xc)),
							near: (b9 & 15) as f32 / 4.0,
							far: crate::water::pool_far(b9, f32::MAX),
						});
					}
					_ => out.ribbons.push(PoolRibbon {
						head: m.rdv3(s + 0x10),
						tail: m.rdv3(s + 0x20),
						half: m.rdf(s + 0x1c),
						scale: m.rdf(s + 0x2c),
						c_head: m.rd32(s + 4),
						c_tail: m.rd32(s + 0xc),
						tex: m.rd8(s + 2),
						additive: m.rd8(s + 3) != 0x44,
						b9: m.rd8(s + 9),
						slot: i as u16,
					}),
				}
			}
		}
	}
	out.blobs = blob_queue(m, level);
	out.hero = Some(m.rdv3(0x13f3d0));
	if let Some((fns, args, _)) = orb_queue(level) {
		let li = level as usize;
		let mut glow_done = false;
		for i in 0..m.rd32(0x15f424).min(0x40) {
			let f = m.rd32(fns + 4 * i);
			if f == (*crate::retail::disc::pick(&GLOW_FN, &GLOW_FN_USA))[li] && !glow_done {
				glow_done = true;
				hero_glows(m, &mut out.glows);
				out.hero_glows = out.glows.len();
			} else if (*crate::retail::disc::pick(&RETICLE_FN, &RETICLE_FN_USA))[li].contains(&f) || (f != 0 && (*crate::retail::disc::pick(&MINE_RETICLE_FN, &MINE_RETICLE_FN_USA))[li] == f) {
				let pv = m.rd32((m.rd32(args + 4 * i) & 0x01ff_ffff) + 0x78) & 0x01ff_ffff;
				if pv != 0 {
					out.reticles.push(crate::game::Reticle { pos: m.rdv3(pv + 0x10), normal: m.rdv3(pv + 0x20) });
				}
			} else if (*crate::retail::disc::pick(&MARKER203_FN, &MARKER203_FN_USA))[li] == f {
				let pv = m.rd32((m.rd32(args + 4 * i) & 0x01ff_ffff) + 0x78) & 0x01ff_ffff;
				if pv != 0 {
					out.reticles.push(crate::game::Reticle { pos: m.rdv3(pv + 0x20), normal: m.rdv3(pv + 0x30) });
				}
			} else if (*crate::retail::disc::pick(&SHIP_SHADOW_FN, &SHIP_SHADOW_FN_USA))[li] == f {
				out.ship_shadows.extend(ship_shadow(m, level, m.rd32(args + 4 * i) & 0x01ff_ffff));
			}
		}
	}
	queue_b::fill(m, level, &mut out.qa, &mut out.qb, &mut out.rope);
	queue_fxa::fleet_water(m, level, &mut out.qw);
	queue_q1::fill(m, level, &mut out.glows, &mut out.strips, &mut out.pools);
	out.ocean = queue_q1::ocean(m, level);
	out.screen = queue_b::screen_filter(m, level);
	out.swap = SwapWords::read(m, level);
	out.rc = queue_b::rc_static(m, level);
	out.shrub_cap = Some(m.rdf(if matches!(level, 0 | 5 | 14 | 16) { va(0x160464) } else { va(0x160424) }));
	out
}





/// The draw-queue arrays (count 0x15f424, fn[], arg[]) of `level`, for the audit.
pub fn draw_queue(level: u32) -> Option<(u32, u32)> {
	orb_queue(level).map(|q| (q.0, q.1))
}

/// A mode-3 pool slot's quad (FUN_00259e18 mode 3, particles.md "Draw size"): corners head +- off, tail +- off * scale,
/// off = the unit cross(midpoint - eye, tail - head) * half-width; U across, V along (head 0, tail 1). Fan order.
/// `fade` scales both alphas: the packet build multiplies bytes +7 / +0xf by min(key - near, (far - key) / 16, 1) of the
/// midpoint depth key, as the sprite modes (overlay.elf.c 82318-82380: pminw of (far - key) >> 4, key - near * 0x400 and
/// 0x1000, pmultuw, >> 12) [C].
pub fn pool_ribbon_quad(r: &PoolRibbon, eye: [f32; 3], fade: f32) -> crate::particles::WorldQuad {
	let (a, b) = (r.head, r.tail);
	let d: [f32; 3] = std::array::from_fn(|i| b[i] - a[i]);
	let v: [f32; 3] = std::array::from_fn(|i| (a[i] + b[i]) * 0.5 - eye[i]);
	let x = [v[1] * d[2] - v[2] * d[1], v[2] * d[0] - v[0] * d[2], v[0] * d[1] - v[1] * d[0]];
	let l = (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
	let off: [f32; 3] = if l > 0.0 { std::array::from_fn(|i| x[i] / l * r.half) } else { [0.0; 3] };
	let p = |o: [f32; 3], s: f32| -> [f32; 3] { std::array::from_fn(|i| o[i] + off[i] * s) };
	let (mut c0, mut c1) = (pool_rgba(r.c_head), pool_rgba(r.c_tail));
	c0[3] *= fade;
	c1[3] *= fade;
	crate::particles::WorldQuad {
		pos: [p(a, -1.0), p(a, 1.0), p(b, r.scale), p(b, -r.scale)],
		uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
		colour: [c0, c0, c1, c1],
		tex: (crate::particles::PART_TEX_BASE + r.tex as u32) as usize,
		additive: r.additive,
	}
}

/// Effects layer claimed set (retail/efx_draw.rs): the queue-A callbacks `snap` itself turns into mirror geometry.
pub(super) fn claims(level: u32, f: u32) -> bool {
	let li = level as usize;
	li < 19 && f != 0 && ((*crate::retail::disc::pick(&GLOW_FN, &GLOW_FN_USA))[li] == f || (*crate::retail::disc::pick(&RETICLE_FN, &RETICLE_FN_USA))[li].contains(&f) || (*crate::retail::disc::pick(&MINE_RETICLE_FN, &MINE_RETICLE_FN_USA))[li] == f || (*crate::retail::disc::pick(&MARKER203_FN, &MARKER203_FN_USA))[li] == f || (*crate::retail::disc::pick(&SHIP_SHADOW_FN, &SHIP_SHADOW_FN_USA))[li] == f)
}
