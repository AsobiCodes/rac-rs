//! Pause-menu hero preview: preview keys, aux / actor / clip / view tables, the worn-item batch types, the
//! preview clock (`MenuHero`) and the pause panel camera size.

use super::*;

/// The joint that holds the wrench (right wrist), and the wrench's in-hand sequence.
/// Preview key of the posed hero (+ item id for the hero with that item attached): the pause menu's big panel
/// (decomp/notes/ingame_pause_menu.md "Item previews").
pub(super) const HERO_PREVIEW: u32 = 0x10000;
/// Gold bolt class 1134, shown on the Galactic details page (frontend.rs `gold_preview`).
pub(super) const GOLD_BOLT_PREVIEW: u32 = 1134;
/// Vertical tan-half-FOV a pause panel of w x h (512x448 units) shows (decomp/notes/ingame_pause_menu.md "Menu
/// camera"): the frozen pause matrix maps 224 px to 0.63 * 0.756 (PAL); FUN_00276c50 renders into a pow2 target
/// (u = 7 up to 128, else the next power; v likewise; v shrinks while u + v > 17) centred on the view, then blits
/// the fit-aspect crop (flag 4) into the panel.
/// Pause aux clip table 0x1b9ad8 by aux index: (class, sequence slot) the clip is installed in (FUN_00284028).
pub(super) const MENU_AUX: [(u32, usize); 17] = [
	(168, 5), (657, 1), (74, 3), (497, 1), (497, 2), (497, 3), (203, 3), (186, 8), (186, 9), (186, 10), (483, 3), (634, 8), (188, 3),
	(433, 3), (163, 4), (619, 3), (270, 4),
];

/// Hero record 0x1b93e8 (0x30 per item id; decomp/notes/ingame_pause_menu.md "Hero preview records") for the items
/// with extra actors: (item, aux start +0x00, aux count +0x04, actors (class, sequence) from +0x18..+0x2c). Items 8
/// and 0x1e also list class 0 with sequence 0, left out [I].
pub(super) const MENU_ACTORS: [(u8, usize, usize, &[(u32, usize)]); 6] = [
	(0x0a, 1, 1, &[(657, 1)]),
	(0x11, 2, 1, &[(74, 3)]),
	(0x14, 7, 3, &[(186, 8), (186, 9), (186, 10)]),
	(0x15, 16, 1, &[(270, 4)]),
	(0x19, 6, 1, &[(203, 3)]),
	(0x1f, 10, 2, &[(634, 8)]),
];

/// Panel-3 item view table 0x1c4488 by item id (decomp/notes/ingame_pause_menu.md "Panel 3 item model"):
/// (distance when flags bit 0, distance otherwise, y, z, rot x, rot y, a, b). The Weapons / Quick Select /
/// Gadgets panel item 0x1b4f28 has flags 0.
pub(super) const PANEL3_VIEW: [[f32; 8]; 0x24] = {
	const H: f32 = -1.5708;
	[
		[6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
		[3.0, 5.0, 0.0, 0.0, 0.0, 0.2, 0.2, 0.0],
		[3.0, 5.0, 0.0, 0.0, 0.0, 0.2, 0.2, 0.0],
		[3.0, 5.0, 0.0, 0.0, 0.0, 0.2, 0.2, 0.0],
		[3.0, 5.0, 0.0, 0.0, 0.0, 0.2, 0.2, 0.0],
		[2.0, 3.0, 0.0, -0.2, 0.0, 0.0, 0.0, 0.0],
		[2.0, 3.0, 0.0, -0.2, 0.0, 0.0, 0.0, 0.0],
		[2.0, 3.0, 0.0, -0.2, 0.0, 0.0, 0.0, 0.0],
		[6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
		[4.0, 4.0, 0.0, 0.0, 0.0, H, 0.0, 0.5],
		[2.5, 2.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2],
		[5.0, 5.0, 0.0, -0.2, 0.0, H, 0.0, 0.4],
		[2.0, 3.0, 0.0, -0.1, 0.0, 0.0, 0.0, 0.1],
		[4.0, 4.0, 0.0, -0.05, 0.0, H, 0.0, 0.45],
		[6.0, 6.0, 0.0, -0.15, 0.0, H, 0.0, 0.3],
		[5.0, 5.0, 0.0, -0.18, 0.0, H, 0.0, 0.3],
		[6.0, 6.0, 0.0, -0.1, 0.0, H, 0.0, 0.5],
		[2.5, 2.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2],
		[3.0, 3.0, 0.0, -0.05, 0.0, 0.0, 0.0, 0.3],
		[5.0, 5.0, 0.0, -0.1, 0.0, H, 0.0, 0.3],
		[2.5, 2.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2],
		[4.0, 4.0, 0.0, -0.1, 0.0, H, 0.0, 0.3],
		[4.5, 6.0, 0.0, 0.0, 0.0, H, 0.0, 0.75],
		[7.0, 7.0, 0.0, -0.3, 0.0, H, 0.0, 0.3],
		[2.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
		[2.5, 2.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2],
		[3.0, 6.0, 0.0, -0.52, -1.5708, 0.0, 0.0, 0.0],
		[2.0, 5.0, 0.0, 0.45, 1.5708, 0.0, 0.2, 0.0],
		[2.0, 3.0, 0.0, 0.2, 0.0, 0.5, 0.0, 0.0],
		[2.0, 3.0, 0.0, 0.2, 0.0, 0.5, 0.0, 0.0],
		[6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
		[2.5, 5.0, 0.0, -0.25, 0.0, H, 0.0, 0.18],
		[1.2, 2.4, 0.0, -0.1, 0.0, H, -0.05, 0.15],
		[6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
		[6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
		[6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
	]
};

pub(super) fn menu_tan_half_y(w: f32, h: f32) -> f32 {
	let (w, h) = (w.round().max(1.0) as i32, h.round().max(1.0) as i32);
	let lg = |x: i32| if x <= 128 { 7 } else { (8..31).find(|&n| 1 << n >= x).unwrap_or(30) };
	let (u, mut v) = (lg(w), lg(h));
	while u + v > 17 {
		v -= 1;
	}
	let rh = 1 << v;
	let src_h = if w < h { rh } else { rh - 2 * (rh / 2 - (h * rh) / (2 * w)) };
	src_h as f32 / 2.0 / 224.0 * 0.63 * 0.756
}

/// Pause hero clip table 0x1b93f0 (0x30-byte rows, indexed by item id; decomp/notes/ingame_pause_menu.md "Hero
/// preview clips"): (clip, loops, item seq, clip2). The clip plays `loops` times then clip2 loops; loops 0 loops
/// the clip. Rows 0 and >= 0x24 change nothing.
pub(super) const MENU_CLIP_ROWS: [(u8, u8, u8, u8); 0x24] = [
	(0, 0, 0, 0), (7, 1, 1, 7), (7, 1, 1, 7), (7, 1, 1, 7), (7, 1, 1, 7), (0x19, 1, 1, 7), (0x1a, 1, 1, 7), (0x15, 1, 1, 7),
	(7, 1, 1, 7), (0x16, 1, 1, 6), (4, 1, 1, 7), (0, 1, 1, 6), (8, 1, 1, 7), (0xf, 1, 4, 7), (0xe, 1, 1, 7), (3, 1, 5, 7),
	(0xa, 1, 1, 6), (5, 0, 1, 7), (2, 0, 1, 7), (1, 1, 1, 7), (0xd, 1, 1, 7), (0x17, 1, 1, 7), (0x12, 1, 1, 6), (9, 1, 1, 6),
	(0x11, 1, 1, 7), (0x10, 1, 1, 7), (0x18, 1, 3, 7), (0x1b, 1, 1, 6), (0xb, 1, 1, 7), (0xc, 1, 1, 7), (6, 1, 1, 6), (0x13, 1, 3, 7),
	(0x14, 1, 1, 7), (6, 1, 1, 6), (6, 1, 1, 6), (6, 1, 1, 6),
];

/// A pause hero preview item: the hero joint it rides, its bind mesh in the joint frame, its class and the view
/// row's mode (0 = pose-copy from the hero, else own itemseq pose with normalised attach columns).
pub(super) struct HeroItem {
	pub(super) joint: usize,
	pub(super) mesh: Mesh,
	pub(super) class: u32,
	pub(super) mode: u8,
}

/// Preview key of the worn-item batch (boots, back item, extra items), drawn over the hero panel.
pub(super) const HERO_WORN: u32 = 0x20000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum WornKind {
	/// Boot moby at hero attach point 2 / 3 (feet slot).
	BootA,
	BootB,
	/// The back item (attach point 5).
	Back,
	/// An extra item by id (35 / 33 / 34).
	Extra(u8),
}

/// One candidate moby of the worn batch: its vertices are `range` of the batch mesh (collapsed to a point while the
/// part is not worn).
pub(super) struct WornPart {
	pub(super) kind: WornKind,
	pub(super) class: u32,
	pub(super) joint: usize,
	pub(super) range: std::ops::Range<usize>,
	pub(super) mesh: Mesh,
}

/// The worn batch of a level: every candidate part in batch order, and the hero joints of attach points 0x16 / 0x17
/// whose scale is zeroed while boots are worn.
#[derive(Default)]
pub(super) struct WornSet {
	pub(super) parts: Vec<WornPart>,
	pub(super) total: usize,
	pub(super) scale_joints: [Option<usize>; 2],
}

/// Hero joint lists the two boot mobies copy their pose from (dat 0x17a650 / 0x17a668).
pub(super) const BOOT_COPY: [[usize; 5]; 2] = [[99, 100, 101, 102, 103], [93, 94, 95, 96, 97]];

/// Hero record 0x1b93e8 (+0x00, +0x04): the aux clip range (start, count) in `MENU_AUX` per item id.
pub(super) const ITEM_AUX: [(u8, u8); 37] = [
	(0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (1, 1), (0, 0), (0, 0), (14, 1), (0, 0), (0, 1), (0, 0), (2, 1), (0, 0), (0, 0), (7, 3), (16, 1), (0, 0), (0, 0), (0, 0), (6, 1), (12, 1), (0, 0), (0, 0), (0, 0), (0, 0), (10, 2), (15, 1), (0, 0), (0, 0), (0, 0), (0, 0),
];

/// The pause hero preview's clock: a new item's clip fades in over 10 ticks from the pose at the switch
/// (FUN_002575d8(hero, clip, 0, 10, 5)); the clip plays `loops` times (0 = forever) then clip2 fades in from its end.
/// Stepped at 50 Hz from the time the item was selected.
pub(super) struct MenuHero {
	pub(super) id: u8,
	pub(super) pa: anim::PropAnim,
	/// The item's own clock (hand or head moby, BRIDGE-19): its requested itemseq, then sequence 1 once it ends.
	pub(super) item: Option<anim::PropAnim>,
	pub(super) start: f32,
	pub(super) ticks: u32,
	pub(super) plays: u32,
	pub(super) second: bool,
}

/// (clip sequence, loops, clip2 sequence) of item `id`'s row; `base` = seq index of menu clip 0.
pub(super) fn menu_clip(base: usize, id: u8) -> Option<(usize, u8, usize)> {
	MENU_CLIP_ROWS.get(id as usize).filter(|_| id != 0).map(|&(clip, loops, _, clip2)| (base + clip as usize, loops, base + clip2 as usize))
}

pub(super) fn gpu_has_preview(scene: &Option<Scene>, key: u32) -> bool {
	scene.as_ref().is_some_and(|s| s.previews.contains_key(&key))
}
